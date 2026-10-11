#ifndef _WIN32
#define _POSIX_C_SOURCE 200809L
#endif
#include "local_vsam.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <io.h>
#include <windows.h>
#else
#include <fcntl.h>
#include <sys/file.h>
#include <unistd.h>
#endif

typedef struct {
    int borrowed;
#ifdef _WIN32
    HANDLE file;
    OVERLAPPED overlap;
#else
    int file;
#endif
} vsam_lock;

typedef struct uow_entry {
    char *path;
    unsigned char *bytes;
    size_t length;
    int dirty;
    int prepared;
    vsam_lock lock;
    struct uow_entry *next;
} uow_entry;

static _Thread_local int uow_active;
static _Thread_local uow_entry *uow_files;

static uow_entry *find_uow(const char *path) {
    for (uow_entry *entry = uow_files; entry; entry = entry->next)
        if (strcmp(entry->path, path) == 0) return entry;
    return NULL;
}

static int raw_lock_dataset(const char *path, vsam_lock *lock) {
    char name[4096];
    lock->borrowed = 0;
    if (snprintf(name, sizeof name, "%s.jclsh-lock", path) >= (int)sizeof name)
        return 17;
#ifdef _WIN32
    memset(&lock->overlap, 0, sizeof lock->overlap);
    lock->file = CreateFileA(name, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
                             NULL, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (lock->file == INVALID_HANDLE_VALUE) return 17;
    if (!LockFileEx(lock->file, LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                    0, 1, 0, &lock->overlap)) {
        CloseHandle(lock->file);
        return 17;
    }
#else
    lock->file = open(name, O_CREAT | O_RDWR, 0600);
    if (lock->file < 0) return 17;
    if (flock(lock->file, LOCK_EX | LOCK_NB)) { close(lock->file); return 17; }
#endif
    return 0;
}

static void raw_unlock_dataset(vsam_lock *lock) {
#ifdef _WIN32
    UnlockFileEx(lock->file, 0, 1, 0, &lock->overlap);
    CloseHandle(lock->file);
#else
    flock(lock->file, LOCK_UN);
    close(lock->file);
#endif
}

static int lock_dataset(const char *path, vsam_lock *lock) {
    if (!uow_active) return raw_lock_dataset(path, lock);
    if (find_uow(path)) { lock->borrowed = 1; return 0; }
    uow_entry *entry = calloc(1, sizeof *entry);
    if (!entry) return 17;
    entry->path = malloc(strlen(path) + 1);
    if (!entry->path) { free(entry); return 17; }
    strcpy(entry->path, path);
    if (raw_lock_dataset(path, &entry->lock)) {
        free(entry->path); free(entry); return 17;
    }
    entry->next = uow_files;
    uow_files = entry;
    lock->borrowed = 1;
    return 0;
}

static void unlock_dataset(vsam_lock *lock) {
    if (!lock->borrowed) raw_unlock_dataset(lock);
}

/* Return CICS RESP values: NORMAL=0, NOTFND=13, DUPREC=14, IOERR=17. */
int cobcics_vsam_layout(const char *path, size_t *record_size,
                        size_t *key_size, size_t *key_offset) {
    char name[4096], format[3];
    FILE *file;
    if (snprintf(name, sizeof name, "%s.jclsh-meta", path) >= (int)sizeof name)
        return 17;
    file = fopen(name, "rb");
    if (!file) return 17;
    int count = fscanf(file, "%2s\t%zu\t%zu\t%zu", format,
                       record_size, key_size, key_offset);
    fclose(file);
    if (count != 4 || strcmp(format, "FB") || !*record_size ||
        *key_offset > *record_size || *key_size > *record_size - *key_offset) return 17;
    return 0;
}

static int load(const char *path, size_t record_size,
                unsigned char **bytes, size_t *length) {
    uow_entry *entry = find_uow(path);
    if (uow_active && entry && entry->dirty) {
        if (entry->length % record_size) return 17;
        *bytes = malloc(entry->length ? entry->length : 1);
        if (!*bytes) return 17;
        memcpy(*bytes, entry->bytes, entry->length);
        *length = entry->length;
        return 0;
    }
    FILE *file = fopen(path, "rb");
    long size;
    if (!file) return 17;
    if (fseek(file, 0, SEEK_END) || (size = ftell(file)) < 0 ||
        (unsigned long)size > 64UL * 1024 * 1024 ||
        fseek(file, 0, SEEK_SET) || (size_t)size % record_size) {
        fclose(file);
        return 17;
    }
    *bytes = malloc((size_t)size ? (size_t)size : 1);
    if (!*bytes) { fclose(file); return 17; }
    *length = (size_t)size;
    int read_ok = fread(*bytes, 1, *length, file) == *length;
    int close_ok = fclose(file) == 0;
    if (!read_ok || !close_ok) {
        free(*bytes);
        *bytes = NULL;
        return 17;
    }
    return 0;
}

static int save(const char *path, const unsigned char *bytes, size_t length) {
    if (uow_active) {
        uow_entry *entry = find_uow(path);
        if (!entry) return 17;
        unsigned char *copy = malloc(length ? length : 1);
        if (!copy) return 17;
        memcpy(copy, bytes, length);
        free(entry->bytes);
        entry->bytes = copy;
        entry->length = length;
        entry->dirty = 1;
        return 0;
    }
    char temporary[4096];
    FILE *file;
    if (snprintf(temporary, sizeof temporary, "%s.jclsh-tmp", path) >= (int)sizeof temporary)
        return 17;
    file = fopen(temporary, "wb");
    if (!file) return 17;
    int write_ok = fwrite(bytes, 1, length, file) == length;
    int flush_ok = fflush(file) == 0;
    int close_ok = fclose(file) == 0;
    if (!write_ok || !flush_ok || !close_ok) {
        remove(temporary);
        return 17;
    }
#ifdef _WIN32
    if (!MoveFileExA(temporary, path, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
#else
    if (rename(temporary, path)) {
#endif
        remove(temporary);
        return 17;
    }
    return 0;
}

static int uow_temp_path(const char *path, char *name, size_t capacity) {
    return snprintf(name, capacity, "%s.jclsh-uow-tmp", path) >= (int)capacity ? 17 : 0;
}

int cobcics_vsam_begin(void) {
    if (uow_active || uow_files) return 17;
    uow_active = 1;
    return 0;
}

int cobcics_vsam_prepare(void) {
    if (!uow_active) return 17;
    for (uow_entry *entry = uow_files; entry; entry = entry->next) {
        if (!entry->dirty) continue;
        char temporary[4096];
        if (uow_temp_path(entry->path, temporary, sizeof temporary)) return 17;
        FILE *file = fopen(temporary, "wb");
        if (!file) return 17;
        int okay = fwrite(entry->bytes, 1, entry->length, file) == entry->length;
        if (fflush(file)) okay = 0;
#ifdef _WIN32
        if (_commit(_fileno(file))) okay = 0;
#else
        if (fsync(fileno(file))) okay = 0;
#endif
        if (fclose(file)) okay = 0;
        if (!okay) { remove(temporary); return 17; }
        entry->prepared = 1;
    }
    return 0;
}

int cobcics_vsam_finish(int commit) {
    int result = 0;
    uow_entry *entry = uow_files;
    while (entry) {
        uow_entry *next = entry->next;
        char temporary[4096];
        if (uow_temp_path(entry->path, temporary, sizeof temporary)) result = 17;
        else if (commit && entry->dirty && entry->prepared) {
#ifdef _WIN32
            if (!MoveFileExA(temporary, entry->path,
                             MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) result = 17;
#else
            if (rename(temporary, entry->path)) result = 17;
#endif
        } else if (commit && entry->dirty) result = 17;
        else if (!commit && entry->prepared) remove(temporary);
        raw_unlock_dataset(&entry->lock);
        free(entry->path);
        free(entry->bytes);
        free(entry);
        entry = next;
    }
    uow_files = NULL;
    uow_active = 0;
    return result;
}

static int check_layout(const char *path, size_t record_size,
                        size_t key_size, size_t key_offset) {
    size_t actual_record, actual_key, actual_offset;
    if (cobcics_vsam_layout(path, &actual_record, &actual_key, &actual_offset)) return 17;
    return actual_record == record_size && actual_key == key_size &&
        actual_offset == key_offset ? 0 : 17;
}

int cobcics_vsam_read(const char *path, size_t record_size, size_t key_size,
                      size_t key_offset, const unsigned char *key,
                      size_t compare_size, int gteq, unsigned char *record) {
    unsigned char *bytes = NULL;
    size_t length, chosen = (size_t)-1;
    if (check_layout(path, record_size, key_size, key_offset)) return 17;
    if (!compare_size || compare_size > key_size) return 16;
    if (load(path, record_size, &bytes, &length)) return 17;
    for (size_t offset = 0; offset < length; offset += record_size) {
        const unsigned char *current = bytes + offset + key_offset;
        int comparison = memcmp(current, key, compare_size);
        if ((!gteq && comparison == 0) || (gteq && comparison >= 0)) {
            if (chosen == (size_t)-1 ||
                memcmp(current, bytes + chosen + key_offset, key_size) < 0)
                chosen = offset;
        }
    }
    if (chosen != (size_t)-1) memcpy(record, bytes + chosen, record_size);
    free(bytes);
    return chosen == (size_t)-1 ? 13 : 0;
}

int cobcics_vsam_read_update(const char *path, size_t record_size, size_t key_size,
                             size_t key_offset, const unsigned char *key,
                             size_t compare_size, int gteq, unsigned char *record) {
    vsam_lock lock;
    if (lock_dataset(path, &lock)) return 17;
    int result = cobcics_vsam_read(path, record_size, key_size, key_offset,
                                   key, compare_size, gteq, record);
    unlock_dataset(&lock);
    return result;
}

static int write_unlocked(const char *path, size_t record_size, size_t key_size,
                       size_t key_offset, const unsigned char *record) {
    unsigned char *bytes = NULL, *grown;
    size_t length;
    if (check_layout(path, record_size, key_size, key_offset)) return 17;
    if (load(path, record_size, &bytes, &length)) return 17;
    for (size_t offset = 0; offset < length; offset += record_size) {
        if (!memcmp(bytes + offset + key_offset, record + key_offset, key_size)) {
            free(bytes);
            return 14;
        }
    }
    grown = realloc(bytes, length + record_size);
    if (!grown) { free(bytes); return 17; }
    memcpy(grown + length, record, record_size);
    int result = save(path, grown, length + record_size);
    free(grown);
    return result;
}

static int rewrite_unlocked(const char *path, size_t record_size, size_t key_size,
                         size_t key_offset, const unsigned char *key,
                         const unsigned char *record) {
    unsigned char *bytes = NULL;
    size_t length;
    if (check_layout(path, record_size, key_size, key_offset)) return 17;
    if (memcmp(key, record + key_offset, key_size)) return 16;
    if (load(path, record_size, &bytes, &length)) return 17;
    for (size_t offset = 0; offset < length; offset += record_size) {
        if (!memcmp(bytes + offset + key_offset, key, key_size)) {
            memcpy(bytes + offset, record, record_size);
            int result = save(path, bytes, length);
            free(bytes);
            return result;
        }
    }
    free(bytes);
    return 13;
}

static int delete_unlocked(const char *path, size_t record_size, size_t key_size,
                        size_t key_offset, const unsigned char *key) {
    unsigned char *bytes = NULL;
    size_t length;
    if (check_layout(path, record_size, key_size, key_offset)) return 17;
    if (load(path, record_size, &bytes, &length)) return 17;
    for (size_t offset = 0; offset < length; offset += record_size) {
        if (!memcmp(bytes + offset + key_offset, key, key_size)) {
            memmove(bytes + offset, bytes + offset + record_size,
                    length - offset - record_size);
            int result = save(path, bytes, length - record_size);
            free(bytes);
            return result;
        }
    }
    free(bytes);
    return 13;
}

int cobcics_vsam_write(const char *path, size_t record_size, size_t key_size,
                       size_t key_offset, const unsigned char *record) {
    vsam_lock lock;
    if (lock_dataset(path, &lock)) return 17;
    int result = write_unlocked(path, record_size, key_size, key_offset, record);
    unlock_dataset(&lock);
    return result;
}

int cobcics_vsam_rewrite(const char *path, size_t record_size, size_t key_size,
                         size_t key_offset, const unsigned char *key,
                         const unsigned char *record) {
    vsam_lock lock;
    if (lock_dataset(path, &lock)) return 17;
    int result = rewrite_unlocked(path, record_size, key_size, key_offset, key, record);
    unlock_dataset(&lock);
    return result;
}

int cobcics_vsam_delete(const char *path, size_t record_size, size_t key_size,
                        size_t key_offset, const unsigned char *key) {
    vsam_lock lock;
    if (lock_dataset(path, &lock)) return 17;
    int result = delete_unlocked(path, record_size, key_size, key_offset, key);
    unlock_dataset(&lock);
    return result;
}
