/* Shared local TS/TD queue storage for GenApp CICS workers.
   Queue files are keyed by the exact eight-byte CICS queue name. */
#ifndef _WIN32
#define _POSIX_C_SOURCE 200809L
#endif
#include <stdint.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#include <windows.h>
#include <process.h>
#else
#include <fcntl.h>
#include <sys/file.h>
#include <unistd.h>
#endif

#define MAX_QUEUE_ITEMS 1024
#define MAX_QUEUE_RECORD 32763
#define MAX_QUEUE_CURSORS 32

struct queue_item {
    uint16_t length;
    unsigned char *data;
};

struct queue_state {
    uint32_t count;
    struct queue_item items[MAX_QUEUE_ITEMS];
};

struct queue_cursor {
    unsigned char name[8];
    uint32_t item;
};

static struct queue_cursor cursors[MAX_QUEUE_CURSORS];
static size_t cursor_count;

static uint32_t fullword(const unsigned char *value) {
    return (uint32_t)value[0] << 24 | (uint32_t)value[1] << 16 |
           (uint32_t)value[2] << 8 | value[3];
}

static void put_fullword(unsigned char *value, uint32_t number) {
    value[0] = (unsigned char)(number >> 24);
    value[1] = (unsigned char)(number >> 16);
    value[2] = (unsigned char)(number >> 8);
    value[3] = (unsigned char)number;
}

static void status(unsigned char *resp, unsigned char *resp2,
                   uint32_t code, uint32_t detail) {
    put_fullword(resp, code);
    put_fullword(resp2, detail);
}

static uint64_t queue_hash(const unsigned char *name) {
    uint64_t hash = 14695981039346656037ULL;
    for (size_t index = 0; index < 8; ++index)
        hash = (hash ^ name[index]) * 1099511628211ULL;
    return hash;
}

static int queue_path(const unsigned char *name, const char *type,
                      const char *suffix, char *path, size_t size) {
    const char *directory = getenv("COBCICS_STORAGE_DIR");
    if (!directory || !*directory) {
#ifdef _WIN32
        directory = getenv("TEMP");
        if (!directory || !*directory)
            directory = ".";
#else
        directory = "/tmp";
#endif
    }
    int written = snprintf(path, size, "%s/cobcics-%s-%016llx.%s", directory,
                           type, (unsigned long long)queue_hash(name), suffix);
    return written > 0 && (size_t)written < size;
}

struct queue_lock {
#ifdef _WIN32
    HANDLE handle;
    OVERLAPPED overlap;
#else
    int fd;
#endif
};

static int take_lock(const unsigned char *name, const char *type,
                     struct queue_lock *lock) {
    char path[512];
    if (!queue_path(name, type, "lock", path, sizeof path))
        return 0;
#ifdef _WIN32
    lock->handle = CreateFileA(path, GENERIC_READ | GENERIC_WRITE,
                               FILE_SHARE_READ | FILE_SHARE_WRITE, NULL,
                               OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (lock->handle == INVALID_HANDLE_VALUE)
        return 0;
    memset(&lock->overlap, 0, sizeof lock->overlap);
    if (!LockFileEx(lock->handle, LOCKFILE_EXCLUSIVE_LOCK, 0, 1, 0,
                    &lock->overlap)) {
        CloseHandle(lock->handle);
        return 0;
    }
#else
    lock->fd = open(path, O_CREAT | O_RDWR, 0600);
    if (lock->fd < 0)
        return 0;
    if (flock(lock->fd, LOCK_EX) != 0) {
        close(lock->fd);
        return 0;
    }
#endif
    return 1;
}

static void drop_lock(struct queue_lock *lock) {
#ifdef _WIN32
    UnlockFileEx(lock->handle, 0, 1, 0, &lock->overlap);
    CloseHandle(lock->handle);
#else
    flock(lock->fd, LOCK_UN);
    close(lock->fd);
#endif
}

static int read_u32(FILE *file, uint32_t *value) {
    unsigned char bytes[4];
    if (fread(bytes, 1, 4, file) != 4)
        return 0;
    *value = fullword(bytes);
    return 1;
}

static int write_u32(FILE *file, uint32_t value) {
    unsigned char bytes[4];
    put_fullword(bytes, value);
    return fwrite(bytes, 1, 4, file) == 4;
}

static void free_state(struct queue_state *state) {
    if (!state)
        return;
    for (uint32_t index = 0; index < state->count; ++index)
        free(state->items[index].data);
    free(state);
}

/* 0: loaded, 1: no queue, -1: damaged or inaccessible. */
static int load_state(const char *path, struct queue_state *state) {
    FILE *file = fopen(path, "rb");
    if (!file)
        return errno == ENOENT ? 1 : -1;
    uint32_t count;
    int ok = read_u32(file, &count) && count <= MAX_QUEUE_ITEMS;
    for (uint32_t index = 0; ok && index < count; ++index) {
        uint32_t length;
        ok = read_u32(file, &length) && length > 0 &&
             length <= MAX_QUEUE_RECORD;
        if (!ok)
            break;
        state->items[index].data = malloc(length);
        ok = state->items[index].data != NULL &&
             fread(state->items[index].data, 1, length, file) == length;
        if (ok) {
            state->items[index].length = (uint16_t)length;
            state->count++;
        } else {
            free(state->items[index].data);
            state->items[index].data = NULL;
        }
    }
    if (ok)
        ok = fgetc(file) == EOF;
    fclose(file);
    return ok ? 0 : -1;
}

static int save_state(const char *path, const struct queue_state *state) {
    char temporary[560];
#ifdef _WIN32
    int pid = _getpid();
#else
    int pid = (int)getpid();
#endif
    int written = snprintf(temporary, sizeof temporary, "%s.%d.tmp", path, pid);
    if (written <= 0 || (size_t)written >= sizeof temporary)
        return 0;
    FILE *file = fopen(temporary, "wb");
    if (!file)
        return 0;
    int ok = write_u32(file, state->count);
    for (uint32_t index = 0; ok && index < state->count; ++index) {
        const struct queue_item *item = &state->items[index];
        ok = write_u32(file, item->length) &&
             fwrite(item->data, 1, item->length, file) == item->length;
    }
    ok = fclose(file) == 0 && ok;
    if (ok) {
#ifdef _WIN32
        ok = MoveFileExA(temporary, path, MOVEFILE_REPLACE_EXISTING) != 0;
#else
        ok = rename(temporary, path) == 0;
#endif
    }
    if (!ok)
        remove(temporary);
    return ok;
}

static struct queue_cursor *cursor_for(const unsigned char *name) {
    for (size_t index = 0; index < cursor_count; ++index)
        if (memcmp(cursors[index].name, name, 8) == 0)
            return &cursors[index];
    if (cursor_count == MAX_QUEUE_CURSORS)
        return NULL;
    struct queue_cursor *cursor = &cursors[cursor_count++];
    memcpy(cursor->name, name, 8);
    cursor->item = 0;
    return cursor;
}

void COBCICS_TSQ_READ(const unsigned char *name, unsigned char *destination,
                      unsigned char *length, const unsigned char *capacity,
                      const unsigned char *item, const unsigned char *next,
                      unsigned char *resp, unsigned char *resp2) {
    uint32_t available = fullword(length);
    uint32_t size = fullword(capacity);
    if (available > size || size > MAX_QUEUE_RECORD || *next > 1) {
        status(resp, resp2, 16, 1);
        return;
    }
    struct queue_lock lock;
    char path[512];
    if (!queue_path(name, "tsq", "data", path, sizeof path) ||
        !take_lock(name, "tsq", &lock)) {
        status(resp, resp2, 16, 2);
        return;
    }
    struct queue_state *state = calloc(1, sizeof *state);
    int loaded = state ? load_state(path, state) : -1;
    struct queue_cursor *cursor = cursor_for(name);
    uint32_t index = *next ? (cursor ? cursor->item + 1 : 0) : fullword(item);
    if (!*next && index == 0)
        index = 1;
    if (loaded == 1)
        status(resp, resp2, 44, 0); /* QIDERR */
    else if (loaded < 0 || !cursor)
        status(resp, resp2, 16, 3);
    else if (index == 0 || index > state->count)
        status(resp, resp2, 26, 0); /* ITEMERR */
    else {
        struct queue_item *record = &state->items[index - 1];
        put_fullword(length, record->length);
        if (record->length > available)
            status(resp, resp2, 22, 0); /* LENGERR */
        else {
            memcpy(destination, record->data, record->length);
            cursor->item = index;
            status(resp, resp2, 0, 0);
        }
    }
    free_state(state);
    drop_lock(&lock);
}

void COBCICS_TSQ_WRITE(const unsigned char *name, const unsigned char *source,
                       const unsigned char *length,
                       const unsigned char *capacity,
                       const unsigned char *item, const unsigned char *rewrite,
                       unsigned char *resp, unsigned char *resp2) {
    uint32_t size = fullword(length);
    /* CICS FROM can point at an elementary item followed by other items in
       its containing group.  LENGTH, rather than the item's PIC size,
       describes the contiguous source bytes to write. */
    (void)capacity;
    if (size == 0 || size > MAX_QUEUE_RECORD || *rewrite > 1) {
        status(resp, resp2, 22, 1);
        return;
    }
    struct queue_lock lock;
    char path[512];
    if (!queue_path(name, "tsq", "data", path, sizeof path) ||
        !take_lock(name, "tsq", &lock)) {
        status(resp, resp2, 16, 2);
        return;
    }
    struct queue_state *state = calloc(1, sizeof *state);
    int loaded = state ? load_state(path, state) : -1;
    uint32_t index = fullword(item);
    if (loaded == 1 && *rewrite)
        status(resp, resp2, 44, 0);
    else if (loaded < 0)
        status(resp, resp2, 16, 3);
    else if (*rewrite && (index == 0 || index > state->count))
        status(resp, resp2, 26, 0);
    else if (!*rewrite && state->count >= MAX_QUEUE_ITEMS)
        status(resp, resp2, 18, 0); /* NOSPACE */
    else {
        unsigned char *data = malloc(size);
        if (!data)
            status(resp, resp2, 18, 1);
        else {
            memcpy(data, source, size);
            if (*rewrite) {
                struct queue_item *record = &state->items[index - 1];
                free(record->data);
                record->data = data;
                record->length = (uint16_t)size;
            } else {
                struct queue_item *record = &state->items[state->count++];
                record->data = data;
                record->length = (uint16_t)size;
            }
            status(resp, resp2, save_state(path, state) ? 0 : 16, 0);
        }
    }
    free_state(state);
    drop_lock(&lock);
}

void COBCICS_TSQ_DELETE(const unsigned char *name,
                        unsigned char *resp, unsigned char *resp2) {
    struct queue_lock lock;
    char path[512];
    if (!queue_path(name, "tsq", "data", path, sizeof path) ||
        !take_lock(name, "tsq", &lock)) {
        status(resp, resp2, 16, 2);
        return;
    }
    FILE *file = fopen(path, "rb");
    if (!file)
        status(resp, resp2, 44, 0);
    else {
        fclose(file);
        status(resp, resp2, remove(path) == 0 ? 0 : 16, 0);
    }
    struct queue_cursor *cursor = cursor_for(name);
    if (cursor)
        cursor->item = 0;
    drop_lock(&lock);
}

void COBCICS_TDQ_WRITE(const unsigned char *name, const unsigned char *source,
                       const unsigned char *length,
                       const unsigned char *capacity,
                       unsigned char *resp, unsigned char *resp2) {
    uint32_t size = fullword(length);
    (void)capacity;
    if (size == 0 || size > MAX_QUEUE_RECORD) {
        status(resp, resp2, 22, 1);
        return;
    }
    struct queue_lock lock;
    char path[512];
    if (!queue_path(name, "tdq", "data", path, sizeof path) ||
        !take_lock(name, "tdq", &lock)) {
        status(resp, resp2, 16, 2);
        return;
    }
    FILE *file = fopen(path, "ab");
    int ok = file && write_u32(file, size) &&
             fwrite(source, 1, size, file) == size;
    if (file)
        ok = fclose(file) == 0 && ok;
    status(resp, resp2, ok ? 0 : 16, 0);
    drop_lock(&lock);
}

/* Counter values are kept separately from queues.  GET returns the current
   number and atomically advances it for the next caller. */
static void counter_key(const unsigned char *name, const unsigned char *pool,
                        unsigned char key[8]) {
    uint64_t hash = 14695981039346656037ULL;
    for (size_t index = 0; index < 8; ++index)
        hash = (hash ^ pool[index]) * 1099511628211ULL;
    for (size_t index = 0; index < 16; ++index)
        hash = (hash ^ name[index]) * 1099511628211ULL;
    for (size_t index = 0; index < 8; ++index)
        key[index] = (unsigned char)(hash >> (index * 8));
}

static void counter_command(const unsigned char *name,
                            const unsigned char *pool, unsigned char *value,
                            unsigned char *resp, unsigned char *resp2,
                            char command) {
    unsigned char key[8];
    char path[512];
    struct queue_lock lock;
    counter_key(name, pool, key);
    if (!queue_path(key, "counter", "data", path, sizeof path) ||
        !take_lock(key, "counter", &lock)) {
        status(resp, resp2, 16, 2);
        return;
    }
    FILE *file = fopen(path, "rb");
    uint32_t current = 0;
    int exists = file != NULL;
    int valid = !file || (read_u32(file, &current) && fgetc(file) == EOF);
    if (file)
        fclose(file);
    if (!valid) {
        status(resp, resp2, 16, 3);
    } else if (command == 'D') {
        if (!exists)
            status(resp, resp2, 16, 201);
        else
            status(resp, resp2, remove(path) == 0 ? 0 : 16, 0);
    } else if (command == 'F') {
        if (exists)
            status(resp, resp2, 16, 202);
        else {
            file = fopen(path, "wb");
            int ok = file && write_u32(file, fullword(value));
            if (file)
                ok = fclose(file) == 0 && ok;
            status(resp, resp2, ok ? 0 : 16, 0);
        }
    } else if (!exists) {
        status(resp, resp2, 16, 201);
    } else if (command == 'Q') {
        put_fullword(value, current);
        status(resp, resp2, 0, 0);
    } else if (current == 0x7fffffffU) {
        status(resp, resp2, 72, 101); /* SUPPRESSED: counter at limit */
    } else {
        file = fopen(path, "wb");
        int ok = file && write_u32(file, current + 1);
        if (file)
            ok = fclose(file) == 0 && ok;
        if (ok)
            put_fullword(value, current);
        status(resp, resp2, ok ? 0 : 16, 0);
    }
    drop_lock(&lock);
}

void COBCICS_COUNTER_DEFINE(const unsigned char *name,
                            const unsigned char *pool, unsigned char *value,
                            unsigned char *resp, unsigned char *resp2) {
    counter_command(name, pool, value, resp, resp2, 'F');
}

void COBCICS_COUNTER_DELETE(const unsigned char *name,
                            const unsigned char *pool, unsigned char *value,
                            unsigned char *resp, unsigned char *resp2) {
    counter_command(name, pool, value, resp, resp2, 'D');
}

void COBCICS_COUNTER_GET(const unsigned char *name,
                         const unsigned char *pool, unsigned char *value,
                         unsigned char *resp, unsigned char *resp2) {
    counter_command(name, pool, value, resp, resp2, 'G');
}

void COBCICS_COUNTER_QUERY(const unsigned char *name,
                           const unsigned char *pool, unsigned char *value,
                           unsigned char *resp, unsigned char *resp2) {
    counter_command(name, pool, value, resp, resp2, 'Q');
}
