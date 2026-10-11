/* Exercise the CICS task coordinator's optional IMS syncpoint hook. */
#define COBCICS_IMSDB
#include "bridge.c"

static int commits;
static int rollbacks;
static int fail_commit;

int IMSDB_END_TASK(int commit) {
    if (commit) {
        ++commits;
        return fail_commit;
    }
    ++rollbacks;
    return 0;
}

int main(void) {
    if (commit_uow() || commits != 1) return 1;
    if (rollback_uow() || rollbacks != 1) return 1;
    fail_commit = 1;
    if (commit_uow() != 1 || commits != 2) return 1;
    puts("CICS IMS UOW hook OK");
    return 0;
}
