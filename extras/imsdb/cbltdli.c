#define _POSIX_C_SOURCE 200809L
/* GnuCOBOL CALL entry point for the length-bearing Rust IMS DB ABI.
 * The COBOL caller supplies a DB PCB mask initialized with its DBD name.
 */
#include <libcob.h>
#include "imsdb.h"
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define IMS_MAX_SSA 16

static pthread_mutex_t ims_mutex = PTHREAD_MUTEX_INITIALIZER;
static ImsHandle *ims_handle;
static int ims_encoding = -1;
static int ims_cleanup_registered;
static int ims_auto_commit;
static int ims_scheduled;
static FILE *ims_input;

struct ims_message {
    struct ims_message *next;
    size_t length;
    unsigned char data[];
};
static struct ims_message *ims_output_head;
static struct ims_message **ims_output_tail = &ims_output_head;

static void discard_messages(void)
{
    while (ims_output_head) {
        struct ims_message *next = ims_output_head->next;
        free(ims_output_head);
        ims_output_head = next;
    }
    ims_output_tail = &ims_output_head;
}

static int commit_messages(char *error, size_t capacity)
{
    if (!ims_output_head) return 0;
    const char *path = getenv("DD_IMSOUTPUT");
    if (!path || !*path) {
        snprintf(error, capacity, "DD_IMSOUTPUT is required");
        return -1;
    }
    FILE *file = fopen(path, "r+b");
    int created = 0;
    if (!file) {
        file = fopen(path, "w+b");
        created = 1;
    }
    if (!file) {
        snprintf(error, capacity, "cannot open DD_IMSOUTPUT");
        return -1;
    }
    if (fseeko(file, 0, SEEK_END) != 0) {
        fclose(file);
        snprintf(error, capacity, "cannot seek DD_IMSOUTPUT");
        return -1;
    }
    off_t original_length = ftello(file);
    if (original_length < 0) {
        fclose(file);
        snprintf(error, capacity, "cannot stat DD_IMSOUTPUT");
        return -1;
    }
    int failed = 0;
    for (struct ims_message *message = ims_output_head; message; message = message->next) {
        if (fwrite(message->data, 1, message->length, file) != message->length) {
            failed = 1;
            break;
        }
    }
    if (!failed && (fflush(file) != 0 || fsync(fileno(file)) != 0)) failed = 1;
    if (failed) {
        if (ftruncate(fileno(file), original_length) != 0 ||
            fsync(fileno(file)) != 0) {
            snprintf(error, capacity, "DD_IMSOUTPUT write and restore failed");
            fclose(file);
            return -1;
        }
        fclose(file);
        if (created) remove(path);
        snprintf(error, capacity, "DD_IMSOUTPUT write failed");
        return -1;
    }
    if (fclose(file) != 0) {
        snprintf(error, capacity, "DD_IMSOUTPUT close failed");
        return -1;
    }
    discard_messages();
    return 0;
}

static int io_pcb_call(const char operation[5], unsigned char *pcb,
                       unsigned char *area, size_t capacity,
                       const cob_charset *charset)
{
    unsigned char space = charset->from_host ? charset->from_host[(unsigned char)' '] : ' ';
    if (memcmp(operation, "GU  ", 4) == 0) {
        if (!ims_input) {
            const char *path = getenv("DD_IMSINPUT");
            if (!path || !*path || !(ims_input = fopen(path, "rb"))) {
                fputs("CBLTDLI: DD_IMSINPUT is required and must be readable\n", stderr);
                return -1;
            }
        }
        unsigned char length_bytes[2];
        size_t got = fread(length_bytes, 1, 2, ims_input);
        if (got == 0 && feof(ims_input)) {
            pcb[10] = charset->from_host ? charset->from_host[(unsigned char)'Q'] : 'Q';
            pcb[11] = charset->from_host ? charset->from_host[(unsigned char)'C'] : 'C';
            return 0;
        }
        if (got != 2) {
            fputs("CBLTDLI: truncated IMS input length\n", stderr);
            return -1;
        }
        size_t length = ((size_t)length_bytes[0] << 8) | length_bytes[1];
        if (length < 4 || length > capacity || length > 32767) {
            fputs("CBLTDLI: invalid IMS input message length\n", stderr);
            return -1;
        }
        area[0] = length_bytes[0];
        area[1] = length_bytes[1];
        if (fread(area + 2, 1, length - 2, ims_input) != length - 2) {
            fputs("CBLTDLI: truncated IMS input message\n", stderr);
            return -1;
        }
        pcb[10] = space;
        pcb[11] = space;
        return 0;
    }
    if (memcmp(operation, "ISRT", 4) == 0) {
        if (capacity < 4) return -1;
        size_t length = ((size_t)area[0] << 8) | area[1];
        if (length < 4 || length > capacity || length > 32767) {
            fputs("CBLTDLI: invalid IMS output message length\n", stderr);
            return -1;
        }
        struct ims_message *message = malloc(sizeof(*message) + length);
        if (!message) return -1;
        message->next = NULL;
        message->length = length;
        memcpy(message->data, area, length);
        *ims_output_tail = message;
        ims_output_tail = &message->next;
        pcb[10] = space;
        pcb[11] = space;
        return 0;
    }
    fputs("CBLTDLI: unsupported I/O PCB function\n", stderr);
    return -1;
}

static void ims_cleanup(void)
{
    pthread_mutex_lock(&ims_mutex);
    int normal_exit = ims_auto_commit && cob_last_exit_code() == 0;
    if (ims_handle) {
        if (normal_exit) {
            char error[512] = {0};
            if (imsdb_commit(ims_handle, error, sizeof error) != 0) {
                fprintf(stderr, "CBLTDLI: batch commit failed: %s\n", error);
                _Exit(12);
            }
        } else {
            imsdb_rollback(ims_handle);
        }
        imsdb_close(ims_handle);
        ims_handle = NULL;
    }
    if (normal_exit) {
        char error[512] = {0};
        if (commit_messages(error, sizeof error) != 0) {
            fprintf(stderr, "CBLTDLI: message commit failed: %s\n", error);
            _Exit(12);
        }
    }
    if (ims_input) { fclose(ims_input); ims_input = NULL; }
    discard_messages();
    pthread_mutex_unlock(&ims_mutex);
}

static int caller_encoding(const cob_charset **charset_out)
{
    cob_global *global = cob_get_global_ptr();
    cob_module *module = global ? global->cob_current_module : NULL;
    const cob_charset *charset = module && module->charset
        ? module->charset : &cob_charset_native;
    *charset_out = charset;
    if (!(charset->flags & COB_CS_EBCDIC)) return 0;
    /* The current Rust adapter uses GnuCOBOL's default 1047 translation table.
       IBM-037 has the same DL/I control characters used by these calls. */
    if (charset->ccsid == 37) return 1;
    return -1;
}

static int open_if_needed(int encoding)
{
    if (ims_handle) return ims_encoding == encoding ? 0 : -1;
    const char *wal = getenv("COB_IMS_WAL");
    const char *dbd = getenv("COB_IMS_DBD_DIR");
    const char *psb = getenv("COB_IMS_PSB");
    if (!wal || !dbd || !psb) {
        fputs("CBLTDLI: COB_IMS_WAL, COB_IMS_DBD_DIR and COB_IMS_PSB are required\n", stderr);
        return -1;
    }
    char error[512] = {0};
    uint16_t endian_probe = 1;
    uint32_t flags = (uint32_t)encoding;
    if (*(const uint8_t *)&endian_probe == 1) flags |= 2;
    ims_handle = imsdb_open(wal, dbd, psb, flags, error, sizeof error);
    if (!ims_handle) {
        fprintf(stderr, "CBLTDLI: %s\n", error);
        return -1;
    }
    ims_encoding = encoding;
    const char *auto_commit = getenv("COB_IMS_AUTOCOMMIT");
    ims_auto_commit = auto_commit && auto_commit[0] == '1' && auto_commit[1] == '\0';
    if (!ims_cleanup_registered) {
        atexit(ims_cleanup);
        ims_cleanup_registered = 1;
    }
    return 0;
}

int CBLTDLI(const unsigned char *function, unsigned char *pcb, ...)
{
    (void)function;
    (void)pcb;
    int argc = cob_get_num_params();
    if (argc < 2 || argc > 3 + IMS_MAX_SSA) {
        fprintf(stderr, "CBLTDLI: invalid argument count %d\n", argc);
        return -1;
    }
    const cob_charset *charset;
    int encoding = caller_encoding(&charset);
    if (encoding < 0) {
        fputs("CBLTDLI: this COBOL EBCDIC code page is not supported\n", stderr);
        return -1;
    }
    unsigned char *function_data = cob_get_param_data(1);
    unsigned char *pcb_data = cob_get_param_data(2);
    if (cob_get_param_size(1) != 4 || cob_get_param_size(2) < 12 ||
        !function_data || !pcb_data) {
        fputs("CBLTDLI: invalid function or PCB area\n", stderr);
        return -1;
    }
    char operation[5] = {0};
    for (int index = 0; index < 4; ++index) {
        unsigned char byte = function_data[index];
        operation[index] = (char)(charset->to_host ? charset->to_host[byte] : byte);
    }
    if (memcmp(operation, "CHKP", 4) == 0 || memcmp(operation, "ROLB", 4) == 0) {
        if ((operation[0] == 'C' && (argc != 3 || cob_get_param_size(3) < 8)) ||
            (operation[0] == 'R' && argc != 2)) {
            fputs("CBLTDLI: invalid batch CHKP or ROLB argument list\n", stderr);
            return -1;
        }
        pthread_mutex_lock(&ims_mutex);
        if (open_if_needed(encoding) != 0) {
            pthread_mutex_unlock(&ims_mutex);
            return -1;
        }
        char error[512] = {0};
        int rc;
        if (operation[0] == 'C') {
            unsigned char *id = cob_get_param_data(3);
            if (!id) {
                pthread_mutex_unlock(&ims_mutex);
                return -1;
            }
            for (int index = 0; index < 8; ++index) {
                unsigned char byte = charset->to_host ? charset->to_host[id[index]] : id[index];
                if (byte < 0x20 || byte > 0x7e) {
                    fputs("CBLTDLI: checkpoint ID must contain eight printable characters\n", stderr);
                    pthread_mutex_unlock(&ims_mutex);
                    return -1;
                }
            }
            rc = imsdb_commit(ims_handle, error, sizeof error);
            if (rc == 0) rc = commit_messages(error, sizeof error);
        } else {
            imsdb_rollback(ims_handle);
            discard_messages();
            rc = 0;
        }
        if (rc == 0) {
            unsigned char space = charset->from_host ? charset->from_host[(unsigned char)' '] : ' ';
            pcb_data[10] = space;
            pcb_data[11] = space;
        } else {
            fprintf(stderr, "CBLTDLI: %s: %s\n", operation, error);
        }
        pthread_mutex_unlock(&ims_mutex);
        return rc;
    }
    int gsam_control = memcmp(operation, "OPEN", 4) == 0 ||
                       memcmp(operation, "CLSE", 4) == 0;
    if (cob_get_param_size(2) == 32 && !gsam_control) {
        if (argc != 3 || cob_get_param_size(3) < 4 || !cob_get_param_data(3)) {
            fputs("CBLTDLI: invalid I/O PCB call arguments\n", stderr);
            return -1;
        }
        pthread_mutex_lock(&ims_mutex);
        const char *auto_commit = getenv("COB_IMS_AUTOCOMMIT");
        ims_auto_commit = auto_commit && auto_commit[0] == '1' && auto_commit[1] == '\0';
        if (!ims_cleanup_registered) {
            atexit(ims_cleanup);
            ims_cleanup_registered = 1;
        }
        int rc = io_pcb_call(operation, pcb_data, cob_get_param_data(3),
                             (size_t)cob_get_param_size(3), charset);
        pthread_mutex_unlock(&ims_mutex);
        return rc;
    }
    if ((gsam_control ? argc != 2 : argc < 3) ||
        cob_get_param_size(2) < 36 ||
        (!gsam_control && cob_get_param_size(3) <= 0)) {
        fputs("CBLTDLI: invalid DB PCB or I/O area length\n", stderr);
        return -1;
    }
    ImsSpan ssas[IMS_MAX_SSA];
    for (int index = 3; index < argc; ++index) {
        int length = cob_get_param_size(index + 1);
        void *data = cob_get_param_data(index + 1);
        if (length < 8 || length > 4096 || !data) {
            fputs("CBLTDLI: invalid SSA length or pointer\n", stderr);
            return -1;
        }
        ssas[index - 3].data = data;
        ssas[index - 3].len = (size_t)length;
    }
    unsigned char dbd_name[8];
    unsigned char dummy_io = 0;
    unsigned char *io_data = gsam_control ? &dummy_io : cob_get_param_data(3);
    if (!pcb_data || !io_data || !function_data) return -1;
    for (int index = 0; index < 8; ++index) {
        unsigned char byte = pcb_data[index];
        dbd_name[index] = charset->to_host ? charset->to_host[byte] : byte;
    }

    pthread_mutex_lock(&ims_mutex);
    if (open_if_needed(encoding) != 0) {
        pthread_mutex_unlock(&ims_mutex);
        return -1;
    }
    int pcb_index = imsdb_pcb_index(ims_handle, dbd_name, sizeof dbd_name);
    if (pcb_index < 0) {
        fputs("CBLTDLI: PCB DBD name is absent or ambiguous in PSB\n", stderr);
        pthread_mutex_unlock(&ims_mutex);
        return -1;
    }
    char error[512] = {0};
    int rc = imsdb_call(ims_handle, (uint32_t)pcb_index,
                        function_data, 4,
                        pcb_data, (size_t)cob_get_param_size(2),
                        io_data, gsam_control ? 0 : (size_t)cob_get_param_size(3),
                        ssas, gsam_control ? 0 : (size_t)(argc - 3),
                        error, sizeof error);
    if (rc != 0) fprintf(stderr, "CBLTDLI: %s\n", error);
    pthread_mutex_unlock(&ims_mutex);
    return rc;
}

/* Explicit batch boundary until DFSRRC00 integration owns program lifecycle. */
int IMSDBCOM(void)
{
    pthread_mutex_lock(&ims_mutex);
    char error[512] = {0};
    int rc = ims_handle ? imsdb_commit(ims_handle, error, sizeof error) : -1;
    if (rc == 0) rc = commit_messages(error, sizeof error);
    if (rc != 0) fprintf(stderr, "IMSDBCOM: %s\n", ims_handle ? error : "not open");
    pthread_mutex_unlock(&ims_mutex);
    return rc;
}

/* Optional CICS task coordinator hook. An unopened PSB has no IMS work. */
int IMSDB_END_TASK(int commit)
{
    pthread_mutex_lock(&ims_mutex);
    char error[512] = {0};
    int rc = 0;
    if (commit) {
        if (ims_handle) rc = imsdb_commit(ims_handle, error, sizeof error);
        if (rc == 0) rc = commit_messages(error, sizeof error);
    } else {
        if (ims_handle) imsdb_rollback(ims_handle);
        discard_messages();
    }
    if (rc != 0) fprintf(stderr, "IMSDB_END_TASK: %s\n", error);
    pthread_mutex_unlock(&ims_mutex);
    return rc;
}

/* Build a qualified SSA without converting a packed or binary field to text.
 * The translator supplies the 19-byte segment/field/operator prefix. */
int IMSDSA(unsigned char *ssa, const unsigned char *value)
{
    if (cob_get_num_params() != 2 || cob_get_param_size(1) < 20 ||
        cob_get_param_size(2) <= 0 || !ssa || !value) return -1;
    size_t capacity = (size_t)cob_get_param_size(1);
    size_t length = (size_t)cob_get_param_size(2);
    if (length > capacity - 20) return -1;
    const cob_charset *charset;
    if (caller_encoding(&charset) < 0) return -1;
    unsigned char open = charset->from_host ? charset->from_host['('] : '(';
    unsigned char close = charset->from_host ? charset->from_host[')'] : ')';
    if (ssa[8] != open) return -1;
    memcpy(ssa + 19, value, length);
    ssa[19 + length] = close;
    return 0;
}

/* Service boundary used by the EXEC DLI translator. The third parameter is
 * DIBSTAT; every service returns an IMS-style two-character status there. */
int IMSDEXEC(const unsigned char *function, const unsigned char *argument,
             unsigned char *status)
{
    (void)function;
    (void)argument;
    (void)status;
    if (cob_get_num_params() != 3 || cob_get_param_size(1) != 4 ||
        cob_get_param_size(2) < 8 || cob_get_param_size(3) < 2) return -1;
    const cob_charset *charset;
    int encoding = caller_encoding(&charset);
    if (encoding < 0) return -1;
    unsigned char *op_data = cob_get_param_data(1);
    unsigned char *arg_data = cob_get_param_data(2);
    unsigned char *out = cob_get_param_data(3);
    if (!op_data || !arg_data || !out) return -1;
    char op[5] = {0};
    char arg[9] = {0};
    for (int i = 0; i < 4; ++i)
        op[i] = (char)(charset->to_host ? charset->to_host[op_data[i]] : op_data[i]);
    for (int i = 0; i < 8; ++i)
        arg[i] = (char)(charset->to_host ? charset->to_host[arg_data[i]] : arg_data[i]);
    int rc = 0;
    const char *code = "  ";
    pthread_mutex_lock(&ims_mutex);
    if (memcmp(op, "SCHD", 4) == 0) {
        const char *psb = getenv("COB_IMS_PSB");
        const char *base = psb ? strrchr(psb, '/') : NULL;
        base = base ? base + 1 : psb;
        size_t name_length = base ? strcspn(base, ".") : 0;
        if (!base || name_length > 8 || strncmp(arg, base, name_length) != 0 ||
            (arg[name_length] != ' ' && arg[name_length] != '\0')) {
            code = "TE";
        } else if (ims_scheduled) {
            code = "TC";
        } else if (open_if_needed(encoding) != 0) {
            code = "TE";
        } else {
            ims_scheduled = 1;
        }
    } else if (memcmp(op, "TERM", 4) == 0) {
        if (!ims_scheduled) {
            code = "TR";
        } else {
            char error[512] = {0};
            if (imsdb_commit(ims_handle, error, sizeof error) != 0 ||
                commit_messages(error, sizeof error) != 0) {
                fprintf(stderr, "IMSDEXEC TERM: %s\n", error);
                code = "TR";
            } else {
                imsdb_close(ims_handle);
                ims_handle = NULL;
                ims_encoding = -1;
                ims_scheduled = 0;
            }
        }
    } else if (memcmp(op, "CHKP", 4) == 0) {
        if (open_if_needed(encoding) != 0) {
            code = "AJ";
        } else {
            for (int i = 0; i < 8; ++i) {
                if ((unsigned char)arg[i] < 0x20 || (unsigned char)arg[i] > 0x7e) {
                    code = "AJ";
                    break;
                }
            }
            if (strcmp(code, "  ") == 0) {
                char error[512] = {0};
                if (imsdb_commit(ims_handle, error, sizeof error) != 0 ||
                    commit_messages(error, sizeof error) != 0) {
                    fprintf(stderr, "IMSDEXEC CHKP: %s\n", error);
                    code = "AJ";
                }
            }
        }
    } else {
        code = "AM";
    }
    for (int i = 0; i < 2; ++i)
        out[i] = charset->from_host ? charset->from_host[(unsigned char)code[i]] : code[i];
    pthread_mutex_unlock(&ims_mutex);
    return rc;
}
