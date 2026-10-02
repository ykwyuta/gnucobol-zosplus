/* Deferred CICS START requests.  The Rust host claims due .task files. */
#ifndef _WIN32
#define _POSIX_C_SOURCE 200809L
#endif
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#ifdef _WIN32
#include <windows.h>
#include <process.h>
#else
#include <unistd.h>
#endif

#define MAX_PENDING 8

struct pending_start {
    char transid[4];
    uint32_t minutes;
};

static struct pending_start pending[MAX_PENDING];
static unsigned pending_count;
static unsigned serial;

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

void COBCICS_START_AFTER(const char *transid, const unsigned char *minutes,
                         unsigned char *resp, unsigned char *resp2) {
    uint32_t value = fullword(minutes);
    if (value > 525600 || pending_count == MAX_PENDING) {
        status(resp, resp2, 16, 1);
        return;
    }
    int seen_space = 0;
    for (unsigned index = 0; index < 4; ++index) {
        unsigned char ch = (unsigned char)transid[index];
        if (ch == ' ') {
            if (index == 0) {
                status(resp, resp2, 16, 2);
                return;
            }
            seen_space = 1;
        } else if (seen_space || !((ch >= 'A' && ch <= 'Z') ||
                                   (ch >= '0' && ch <= '9'))) {
            status(resp, resp2, 16, 2);
            return;
        }
    }
    struct pending_start *start = &pending[pending_count++];
    memcpy(start->transid, transid, 4);
    start->minutes = value;
    status(resp, resp2, 0, 0);
}

void COBCICS_START_CANCEL(void) {
    pending_count = 0;
}

static time_t current_time(void) {
    const char *override = getenv("COBCICS_NOW");
    if (override && *override) {
        char *end = NULL;
        errno = 0;
        long long parsed = strtoll(override, &end, 10);
        if (!errno && end && !*end && parsed >= 0)
            return (time_t)parsed;
    }
    return time(NULL);
}

int COBCICS_START_FLUSH(void) {
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
#ifdef _WIN32
    unsigned pid = (unsigned)_getpid();
#else
    unsigned pid = (unsigned)getpid();
#endif
    time_t now = current_time();
    for (unsigned index = 0; index < pending_count; ++index) {
        const struct pending_start *start = &pending[index];
        long long due = (long long)now + (long long)start->minutes * 60;
        char path[512], temporary[520];
        int size = snprintf(path, sizeof path,
            "%s/cobcics-start-%lld-%u-%u.task", directory, due, pid, serial++);
        if (size < 1 || (size_t)size >= sizeof path)
            return 1;
        size = snprintf(temporary, sizeof temporary, "%s.tmp", path);
        if (size < 1 || (size_t)size >= sizeof temporary)
            return 1;
        FILE *file = fopen(temporary, "wb");
        if (!file)
            return 1;
        int ok = fwrite(start->transid, 1, 4, file) == 4;
        ok = fclose(file) == 0 && ok;
        if (ok) {
#ifdef _WIN32
            ok = MoveFileExA(temporary, path, MOVEFILE_REPLACE_EXISTING) != 0;
#else
            ok = rename(temporary, path) == 0;
#endif
        }
        if (!ok) {
            remove(temporary);
            return 1;
        }
    }
    pending_count = 0;
    return 0;
}
