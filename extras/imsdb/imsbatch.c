/* Batch launcher for COBOL programs with an IMS I/O PCB and one DB PCB,
 * or a DB PCB only when COB_IMS_LINKAGE=DB.
 * COB_LIBRARY_PATH must contain a module exporting COB_IMS_ENTRY.
 */
#include <libcob.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int pcb_character(int ch, int ebcdic)
{
    if (!ebcdic) return ch;
    if (ch == ' ') return 0x40;
    if (ch >= '0' && ch <= '9') return 0xf0 + ch - '0';
    if (ch >= 'A' && ch <= 'I') return 0xc1 + ch - 'A';
    if (ch >= 'J' && ch <= 'R') return 0xd1 + ch - 'J';
    if (ch >= 'S' && ch <= 'Z') return 0xe2 + ch - 'S';
    return -1;
}

int main(int argc, char **argv)
{
    const char *entry = getenv("COB_IMS_ENTRY");
    const char *dbd = getenv("COB_IMS_DBD_NAME");
    const char *encoding = getenv("COB_IMS_ENCODING");
    const char *linkage = getenv("COB_IMS_LINKAGE");
    if (!entry || !*entry || !dbd || !*dbd || strlen(dbd) > 8) {
        fputs("imsbatch: COB_IMS_ENTRY and COB_IMS_DBD_NAME (1..8) are required\n", stderr);
        return 8;
    }
    int ebcdic = encoding && strcmp(encoding, "IBM-037") == 0;
    unsigned char io_pcb[32] = {0};
    unsigned char db_pcb[64] = {0};
    for (size_t index = 0; index < 8; ++index) {
        int ch = index < strlen(dbd) ? dbd[index] : ' ';
        int byte = pcb_character(ch, ebcdic);
        if (byte < 0) {
            fputs("imsbatch: DBD name must use uppercase letters and digits\n", stderr);
            return 8;
        }
        db_pcb[index] = (unsigned char)byte;
    }
    const char *sysin = getenv("DD_SYSIN");
    if (sysin && *sysin && !freopen(sysin, "rb", stdin)) {
        fputs("imsbatch: cannot open DD_SYSIN\n", stderr);
        return 8;
    }
    cob_init(argc, argv);
    void *arguments[] = {io_pcb, db_pcb};
    void *db_argument[] = {db_pcb};
    int db_only = linkage && strcmp(linkage, "DB") == 0;
    if (linkage && !db_only && strcmp(linkage, "IO,DB") != 0) {
        fputs("imsbatch: COB_IMS_LINKAGE must be DB or IO,DB\n", stderr);
        return 8;
    }
    int result = db_only ? cob_call(entry, 1, db_argument)
                         : cob_call(entry, 2, arguments);
    /* A COBOL GOBACK returns to this C main without setting libcob's
       STOP RUN exit code. Finish the IMS UOW using the actual call result. */
    int (*end_task)(int) = (int (*)(int))cob_resolve("IMSDB_END_TASK");
    if (!end_task) {
        fputs("imsbatch: IMSDB_END_TASK is unavailable\n", stderr);
        return 12;
    }
    if (end_task(result == 0) != 0 && result == 0) result = 12;
    return result;
}
