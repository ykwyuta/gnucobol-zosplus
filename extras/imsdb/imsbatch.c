/* Batch launcher for the PSB's ordered TP, DB and GSAM PCBs, preceded by
 * the implicit I/O PCB unless COB_IMS_LINKAGE=DB.
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
    if (!entry || !*entry || (dbd && strlen(dbd) > 8)) {
        fputs("imsbatch: COB_IMS_ENTRY is required; COB_IMS_DBD_NAME must fit eight bytes\n", stderr);
        return 8;
    }
    int ebcdic = encoding && strcmp(encoding, "IBM-037") == 0;
    unsigned char io_pcb[32] = {0};
    unsigned char pcbs[16][64] = {{0}};
    const char *sysin = getenv("DD_SYSIN");
    if (sysin && *sysin && !freopen(sysin, "rb", stdin)) {
        fputs("imsbatch: cannot open DD_SYSIN\n", stderr);
        return 8;
    }
    cob_init(argc, argv);
    int (*bind_io)(unsigned char *) = (int (*)(unsigned char *))cob_resolve("IMSTM_BIND_IO");
    if (bind_io) bind_io(io_pcb);
    void *arguments[17] = {0};
    int db_only = linkage && strcmp(linkage, "DB") == 0;
    if (linkage && !db_only && strcmp(linkage, "IO,DB") != 0) {
        fputs("imsbatch: COB_IMS_LINKAGE must be DB or IO,DB\n", stderr);
        return 8;
    }
    int argument_count = 0;
    if (!db_only) arguments[argument_count++] = io_pcb;
    int (*layout)(int, unsigned char *, unsigned char[8], unsigned char *) =
        (int (*)(int, unsigned char *, unsigned char[8], unsigned char *))cob_resolve("IMSPSBLAYOUT");
    int pcb_count = layout ? layout(-1, NULL, NULL, NULL) : -1;
    if (pcb_count < 0 && getenv("COB_IMS_PSB") && *getenv("COB_IMS_PSB")) {
        fputs("imsbatch: cannot read PSB layout\n", stderr);
        return 8;
    }
    if (pcb_count < 0) {
        if (!dbd || !*dbd) {
            fputs("imsbatch: COB_IMS_PSB or COB_IMS_DBD_NAME is required\n", stderr);
            return 8;
        }
        pcb_count = 1;
    }
    if (pcb_count > 16) {
        fputs("imsbatch: PSB has more than 16 PCBs\n", stderr);
        return 8;
    }
    int (*bind_alt)(unsigned char *, int) =
        (int (*)(unsigned char *, int))cob_resolve("IMSTM_BIND_ALT");
    for (int pcb = 0; pcb < pcb_count; ++pcb) {
        unsigned char kind = 'D', name[8], modifiable = 0;
        if (layout && layout(pcb, &kind, name, &modifiable) == 0) {
            /* Use exactly the PCB order from the original PSB. */
        } else {
            if (!dbd || !*dbd || pcb != 0) return 8;
            memset(name, ' ', 8);
            memcpy(name, dbd, strlen(dbd));
        }
        if (kind != 'D' && kind != 'G' && kind != 'T') {
            fputs("imsbatch: unsupported PSB PCB type\n", stderr);
            return 8;
        }
        for (int index = 0; index < 8; ++index) {
            int byte = pcb_character(name[index], ebcdic);
            if (byte < 0) {
                fputs("imsbatch: PCB name must use uppercase letters and digits\n", stderr);
                return 8;
            }
            pcbs[pcb][index] = (unsigned char)byte;
        }
        if (kind == 'T' && bind_alt) bind_alt(pcbs[pcb], modifiable);
        arguments[argument_count++] = pcbs[pcb];
    }
    int result = cob_call(entry, argument_count, arguments);
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
