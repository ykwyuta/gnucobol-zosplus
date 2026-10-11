/* Exercise the same bridge coordinator with a failing/succeeding Db2 stub. */
#define COBCICS_DB2SQL
#define COBCICS_LOCAL_VSAM
#include "bridge.c"
#include "local_vsam.h"
#include <stdio.h>
#include <string.h>

static int fail_commit;
static int commit_calls;
static int rollback_calls;

int DB2SQL_END_TASK(int commit) {
    if (commit) {
        ++commit_calls;
        return fail_commit;
    }
    ++rollback_calls;
    return 0;
}

static long disk_size(const char *path) {
    FILE *file = fopen(path, "rb");
    if (!file) return -1;
    if (fseek(file, 0, SEEK_END)) { fclose(file); return -1; }
    long size = ftell(file);
    fclose(file);
    return size;
}

int main(int argc, char **argv) {
    unsigned char record[225], found[225];
    if (argc != 2) return 2;
    memset(record, ' ', sizeof record);
    memcpy(record, "CUST000009", 10);
    if (disk_size(argv[1]) != 225 || cobcics_vsam_begin()) return 1;
    if (cobcics_vsam_write(argv[1], 225, 10, 0, record)) return 1;
    fail_commit = 1;
    if (commit_uow() != 1 || commit_calls != 1 || disk_size(argv[1]) != 225) return 1;
    if (cobcics_vsam_read(argv[1], 225, 10, 0,
            (const unsigned char *)"CUST000009", 10, 0, found) != 13) return 1;
    if (cobcics_vsam_begin() || cobcics_vsam_write(argv[1], 225, 10, 0, record)) return 1;
    fail_commit = 0;
    if (commit_uow() || commit_calls != 2 || disk_size(argv[1]) != 450) return 1;
    if (cobcics_vsam_begin() || cobcics_vsam_delete(argv[1], 225, 10, 0,
            (const unsigned char *)"CUST000009")) return 1;
    if (rollback_uow() || rollback_calls != 1 || disk_size(argv[1]) != 450) return 1;
    puts("CICS local VSAM and Db2 UOW coordinator passed");
    return 0;
}
