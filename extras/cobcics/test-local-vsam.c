#include "local_vsam.h"
#include <stdio.h>
#include <string.h>
#ifndef _WIN32
#include <sys/wait.h>
#include <unistd.h>
#endif

static int expect(int actual, int wanted, const char *operation) {
    if (actual == wanted) return 0;
    fprintf(stderr, "%s: RESP=%d, expected %d\n", operation, actual, wanted);
    return 1;
}

int main(int argc, char **argv) {
    unsigned char record[225], found[225], next[225];
    size_t length = 0, key_length = 0, key_offset = 0;
    int failed = 0;
#ifndef _WIN32
    if (argc == 3 && strcmp(argv[1], "--probe") == 0) {
        memset(record, ' ', sizeof record);
        memcpy(record, "CUST000099", 10);
        return cobcics_vsam_write(argv[2], 225, 10, 0, record) == 17 ? 0 : 1;
    }
#endif
    if (argc != 2) return 2;
    failed |= expect(cobcics_vsam_layout(argv[1], &length, &key_length, &key_offset), 0, "layout");
    if (length != 225 || key_length != 10 || key_offset != 0) {
        fprintf(stderr, "wrong GenApp KSDS layout\n");
        return 1;
    }
    failed |= expect(cobcics_vsam_read(argv[1], 225, 10, 0,
                    (const unsigned char *)"CUST000001", 10, 0, found), 0, "read original");
    if (memcmp(found, "CUST000001", 10)) failed = 1;
    memset(record, ' ', sizeof record);
    memcpy(record, "CUST000002", 10);
    memcpy(record + 10, "PAYLOAD", 7);
    failed |= expect(cobcics_vsam_write(argv[1], 225, 10, 0, record), 0, "write");
    failed |= expect(cobcics_vsam_write(argv[1], 225, 10, 0, record), 14, "duplicate");
    failed |= expect(cobcics_vsam_read(argv[1], 225, 10, 0,
                    (const unsigned char *)"CUST000002", 10, 0, found), 0, "read written");
    if (memcmp(found, record, sizeof record)) failed = 1;
    failed |= expect(cobcics_vsam_read(argv[1], 225, 10, 0,
                    (const unsigned char *)"CUST000001", 10, 1, next), 0, "read GTEQ");
    if (memcmp(next, "CUST000001", 10)) failed = 1;
    memcpy(record + 10, "UPDATED", 7);
    failed |= expect(cobcics_vsam_rewrite(argv[1], 225, 10, 0,
                    (const unsigned char *)"CUST000002", record), 0, "rewrite");
    failed |= expect(cobcics_vsam_read(argv[1], 225, 10, 0,
                    (const unsigned char *)"CUST000002", 10, 0, found), 0, "read rewritten");
    if (memcmp(found, record, sizeof record)) failed = 1;
    failed |= expect(cobcics_vsam_delete(argv[1], 225, 10, 0,
                    (const unsigned char *)"CUST000002"), 0, "delete");
    failed |= expect(cobcics_vsam_read(argv[1], 225, 10, 0,
                    (const unsigned char *)"CUST000002", 10, 0, found), 13, "read deleted");
    failed |= expect(cobcics_vsam_read(argv[1], 224, 10, 0,
                    (const unsigned char *)"CUST000001", 10, 0, found), 17, "layout mismatch");
    failed |= expect(cobcics_vsam_begin(), 0, "begin rollback UOW");
    failed |= expect(cobcics_vsam_write(argv[1], 225, 10, 0, record), 0, "staged write");
#ifndef _WIN32
    pid_t child = fork();
    if (child == 0) {
        execl(argv[0], argv[0], "--probe", argv[1], (char *)NULL);
        _exit(2);
    }
    int status = 0;
    if (child < 0 || waitpid(child, &status, 0) != child ||
        !WIFEXITED(status) || WEXITSTATUS(status) != 0) failed = 1;
#endif
    failed |= expect(cobcics_vsam_read(argv[1], 225, 10, 0,
                    (const unsigned char *)"CUST000002", 10, 0, found), 0, "read own write");
    FILE *file = fopen(argv[1], "rb");
    if (!file || fseek(file, 0, SEEK_END) || ftell(file) != 225) failed = 1;
    if (file) fclose(file);
    failed |= expect(cobcics_vsam_finish(0), 0, "rollback UOW");
    failed |= expect(cobcics_vsam_read(argv[1], 225, 10, 0,
                    (const unsigned char *)"CUST000002", 10, 0, found), 13, "rolled back write");
    failed |= expect(cobcics_vsam_begin(), 0, "begin commit UOW");
    failed |= expect(cobcics_vsam_write(argv[1], 225, 10, 0, record), 0, "commit staged write");
    failed |= expect(cobcics_vsam_prepare(), 0, "prepare UOW");
    file = fopen(argv[1], "rb");
    if (!file || fseek(file, 0, SEEK_END) || ftell(file) != 225) failed = 1;
    if (file) fclose(file);
    failed |= expect(cobcics_vsam_finish(1), 0, "commit UOW");
    failed |= expect(cobcics_vsam_read(argv[1], 225, 10, 0,
                    (const unsigned char *)"CUST000002", 10, 0, found), 0, "committed write");
    failed |= expect(cobcics_vsam_begin(), 0, "begin delete UOW");
    failed |= expect(cobcics_vsam_delete(argv[1], 225, 10, 0,
                    (const unsigned char *)"CUST000002"), 0, "staged delete");
    failed |= expect(cobcics_vsam_finish(0), 0, "rollback delete");
    failed |= expect(cobcics_vsam_read(argv[1], 225, 10, 0,
                    (const unsigned char *)"CUST000002", 10, 0, found), 0, "delete rolled back");
    if (!failed) puts("local VSAM FB operations passed");
    return failed;
}
