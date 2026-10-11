/* CardDemo's ACCTDAT CICS READ from a local FB data set or sample text. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../cobcics/local_vsam.h"

static uint32_t
number(const unsigned char *p)
{
    return (uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 |
           (uint32_t)p[2] << 8 | p[3];
}

static void
put_number(unsigned char *p, uint32_t value)
{
    p[0] = (unsigned char)(value >> 24);
    p[1] = (unsigned char)(value >> 16);
    p[2] = (unsigned char)(value >> 8);
    p[3] = (unsigned char)value;
}

static void
response(unsigned char *resp, unsigned char *resp2,
         uint32_t code, uint32_t detail)
{
    put_number(resp, code);
    put_number(resp2, detail);
}

void
COBCICS_FILE_READ(const char *name, unsigned char *destination,
                  unsigned char *length, const unsigned char *capacity,
                  unsigned char *ridfld, const unsigned char *key_length,
                  const unsigned char *key_capacity, const unsigned char *mode,
                  unsigned char *resp, unsigned char *resp2)
{
    const char *vsam = getenv("COBCICS_VSAM_ACCTDAT");
    const char *path = getenv("CARDDEMO_ACCTDATA_FILE");
    FILE *file;
    char line[320];
    if (memcmp(name, "ACCTDAT ", 8) != 0 || number(key_length) != 11 ||
        number(key_capacity) < 11 || number(length) < 300 ||
        number(capacity) < 300 || *mode != 0) {
        response(resp, resp2, 16, 1); /* INVREQ */
        return;
    }
    if (vsam && *vsam) {
        int result = cobcics_vsam_read(vsam, 300, 11, 0, ridfld, 11, 0, destination);
        if (!result) put_number(length, 300);
        response(resp, resp2, (uint32_t)result, 0);
        return;
    }
    if (!path || !*path) {
        response(resp, resp2, 16, 1);
        return;
    }
    file = fopen(path, "rb");
    if (!file) {
        response(resp, resp2, 17, 1); /* IOERR */
        return;
    }
    while (fgets(line, sizeof(line), file)) {
        if (strlen(line) < 300) continue;
        if (memcmp(line, ridfld, 11) == 0) {
            memcpy(destination, line, 300);
            put_number(length, 300);
            response(resp, resp2, 0, 0);
            fclose(file);
            return;
        }
    }
    fclose(file);
    response(resp, resp2, 13, 0); /* NOTFND */
}
