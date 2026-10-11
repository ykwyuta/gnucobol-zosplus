/* SPDX-License-Identifier: MIT
 * COBOL's compact MQ copybooks are translated to IBM MQ's C MQI.
 * IBM headers and runtime are supplied separately by IBM and are not vendored.
 */
#define MQCONN MQ_NATIVE_DECL_CONN
#define MQCONNX MQ_NATIVE_DECL_CONNX
#define MQDISC MQ_NATIVE_DECL_DISC
#define MQOPEN MQ_NATIVE_DECL_OPEN
#define MQCLOSE MQ_NATIVE_DECL_CLOSE
#define MQPUT MQ_NATIVE_DECL_PUT
#define MQPUT1 MQ_NATIVE_DECL_PUT1
#define MQGET MQ_NATIVE_DECL_GET
#include <cmqc.h>
#include <cmqxc.h>
#undef MQCONN
#undef MQCONNX
#undef MQDISC
#undef MQOPEN
#undef MQCLOSE
#undef MQPUT
#undef MQPUT1
#undef MQGET

#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#pragma pack(push, 1)
typedef struct {
    char id[4];
    int32_t version, object_type;
    char object_name[48], object_qmgr_name[48];
} cob_mqod;
typedef struct {
    char id[4];
    int32_t version, msg_type, expiry, persistence, coded_char_set_id;
    char format[8], msg_id[24], correl_id[24];
    char reply_to_q[48], reply_to_qmgr[48];
} cob_mqmd;
typedef struct {
    char id[4];
    int32_t version, options, wait_interval;
} cob_mqgmo;
typedef struct {
    char id[4];
    int32_t version, options;
} cob_mqpmo;
#pragma pack(pop)

static void *client_library;
static PMQ_CONNX_CALL native_connx;
static PMQ_DISC_CALL native_disc;
static PMQ_OPEN_CALL native_open;
static PMQ_CLOSE_CALL native_close;
static PMQ_PUT_CALL native_put;
static PMQ_PUT1_CALL native_put1;
static PMQ_GET_CALL native_get;
static MQHCONN implicit_hconn = MQHC_UNUSABLE_HCONN;

/* The z/OS-compatible COBOL BINARY fields in our copybooks are big endian. */
static int32_t read_i(const int32_t *field)
{
    const unsigned char *p = (const unsigned char *)field;
    return (int32_t)((uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 |
                     (uint32_t)p[2] << 8 | (uint32_t)p[3]);
}

static void write_i(int32_t *field, int32_t value)
{
    unsigned char *p = (unsigned char *)field;
    uint32_t n = (uint32_t)value;
    p[0] = (unsigned char)(n >> 24);
    p[1] = (unsigned char)(n >> 16);
    p[2] = (unsigned char)(n >> 8);
    p[3] = (unsigned char)n;
}

static void result(int32_t *cc, int32_t *reason, MQLONG native_cc,
                   MQLONG native_reason)
{
    write_i(cc, native_cc);
    write_i(reason, native_reason);
}

static int has_value(const char *value, size_t length)
{
    size_t i;
    for (i = 0; i < length; ++i)
        if (value[i] != 0 && value[i] != ' ') return 1;
    return 0;
}

static void copy_field(char *to, size_t to_size, const char *from, size_t from_size)
{
    size_t n = from_size < to_size ? from_size : to_size;
    memset(to, ' ', to_size);
    memcpy(to, from, n);
}

static void make_od(MQOD *to, const cob_mqod *from)
{
    MQOD defaults = {MQOD_DEFAULT};
    *to = defaults;
    to->ObjectType = read_i(&from->object_type);
    if (!to->ObjectType) to->ObjectType = MQOT_Q;
    copy_field(to->ObjectName, sizeof(to->ObjectName),
               from->object_name, sizeof(from->object_name));
    copy_field(to->ObjectQMgrName, sizeof(to->ObjectQMgrName),
               from->object_qmgr_name, sizeof(from->object_qmgr_name));
}

static void make_md(MQMD *to, const cob_mqmd *from)
{
    MQMD defaults = {MQMD_DEFAULT};
    *to = defaults;
    to->MsgType = read_i(&from->msg_type);
    to->Expiry = read_i(&from->expiry);
    to->Persistence = read_i(&from->persistence);
    to->CodedCharSetId = read_i(&from->coded_char_set_id);
    if (has_value(from->format, sizeof(from->format)))
        memcpy(to->Format, from->format, sizeof(from->format));
    if (has_value(from->msg_id, sizeof(from->msg_id)))
        memcpy(to->MsgId, from->msg_id, sizeof(from->msg_id));
    if (has_value(from->correl_id, sizeof(from->correl_id)))
        memcpy(to->CorrelId, from->correl_id, sizeof(from->correl_id));
    copy_field(to->ReplyToQ, sizeof(to->ReplyToQ),
               from->reply_to_q, sizeof(from->reply_to_q));
    copy_field(to->ReplyToQMgr, sizeof(to->ReplyToQMgr),
               from->reply_to_qmgr, sizeof(from->reply_to_qmgr));
}

static void save_md(cob_mqmd *to, const MQMD *from)
{
    write_i(&to->msg_type, from->MsgType);
    write_i(&to->expiry, from->Expiry);
    write_i(&to->persistence, from->Persistence);
    write_i(&to->coded_char_set_id, from->CodedCharSetId);
    memcpy(to->format, from->Format, sizeof(to->format));
    memcpy(to->msg_id, from->MsgId, sizeof(to->msg_id));
    memcpy(to->correl_id, from->CorrelId, sizeof(to->correl_id));
    memcpy(to->reply_to_q, from->ReplyToQ, sizeof(to->reply_to_q));
    memcpy(to->reply_to_qmgr, from->ReplyToQMgr, sizeof(to->reply_to_qmgr));
}

static int load_native(void)
{
    const char *path = getenv("MQ_CLIENT_LIB");
    if (client_library) return 1;
    client_library = dlopen(path && *path ? path : "libmqic_r.so",
                            RTLD_NOW | RTLD_LOCAL);
    if (!client_library) {
        fprintf(stderr, "cobmq: IBM MQ client library: %s\n", dlerror());
        return 0;
    }
#define LOAD(name, variable) do { \
    variable = (PMQ_##name##_CALL)dlsym(client_library, "MQ" #name); \
    if (!variable) { \
        fprintf(stderr, "cobmq: missing MQ%s in client library\n", #name); \
        dlclose(client_library); \
        client_library = NULL; \
        return 0; \
    } \
} while (0)
    LOAD(CONNX, native_connx);
    LOAD(DISC, native_disc);
    LOAD(OPEN, native_open);
    LOAD(CLOSE, native_close);
    LOAD(PUT, native_put);
    LOAD(PUT1, native_put1);
    LOAD(GET, native_get);
#undef LOAD
    return 1;
}

static int read_password(char *out, size_t capacity)
{
    const char *env = getenv("MQ_PASSWORD");
    const char *path = getenv("MQ_PASSWORD_FILE");
    FILE *file;
    size_t n;
    if (env && *env) {
        if (strlen(env) >= capacity) return -1;
        strcpy(out, env);
        return 1;
    }
    if (!path || !*path) return 0;
    file = fopen(path, "rb");
    if (!file) return -1;
    n = fread(out, 1, capacity - 1, file);
    fclose(file);
    while (n && (out[n - 1] == '\n' || out[n - 1] == '\r')) --n;
    out[n] = 0;
    return n ? 1 : -1;
}

static void connect_native(MQHCONN *hconn, MQLONG *cc, MQLONG *reason)
{
    MQCD cd = {MQCD_CLIENT_CONN_DEFAULT};
    MQCNO cno = {MQCNO_DEFAULT};
    MQCSP csp = {MQCSP_DEFAULT};
    const char *channel = getenv("MQ_CHANNEL");
    const char *conn = getenv("MQ_CONN_NAME");
    const char *qmgr = getenv("MQ_QMGR_NAME");
    const char *user = getenv("MQ_USER");
    char password[256] = {0};
    int has_password;

    *hconn = MQHC_UNUSABLE_HCONN;
    if (!load_native()) {
        *cc = MQCC_FAILED;
        *reason = MQRC_CONNECTION_BROKEN;
        return;
    }
    if (!channel || !*channel) channel = "DEV.APP.SVRCONN";
    if (!conn || !*conn) conn = "localhost(1415)";
    if (!qmgr || !*qmgr) qmgr = "QM1";
    if (!user || !*user) user = "app";
    if (strlen(channel) > sizeof(cd.ChannelName) ||
        strlen(conn) > sizeof(cd.ConnectionName) ||
        strlen(qmgr) > MQ_Q_MGR_NAME_LENGTH) {
        *cc = MQCC_FAILED;
        *reason = MQRC_OPTIONS_ERROR;
        return;
    }
    has_password = read_password(password, sizeof(password));
    if (has_password < 0) {
        *cc = MQCC_FAILED;
        *reason = MQRC_NOT_AUTHORIZED;
        return;
    }
    copy_field(cd.ChannelName, sizeof(cd.ChannelName), channel, strlen(channel));
    copy_field(cd.ConnectionName, sizeof(cd.ConnectionName), conn, strlen(conn));
    cd.TransportType = MQXPT_TCP;
    cno.Version = MQCNO_VERSION_5;
    cno.Options = MQCNO_CLIENT_BINDING;
    cno.ClientConnPtr = &cd;
    if (has_password) {
        csp.AuthenticationType = MQCSP_AUTH_USER_ID_AND_PWD;
        csp.CSPUserIdPtr = (void *)user;
        csp.CSPUserIdLength = (MQLONG)strlen(user);
        csp.CSPPasswordPtr = password;
        csp.CSPPasswordLength = (MQLONG)strlen(password);
        cno.SecurityParmsPtr = &csp;
    }
    native_connx((char *)qmgr, &cno, hconn, cc, reason);
    memset(password, 0, sizeof(password));
}

static int connection(int32_t *field, MQHCONN *hconn,
                      MQLONG *cc, MQLONG *reason)
{
    MQHCONN value = read_i(field);
    if (!load_native()) {
        *cc = MQCC_FAILED;
        *reason = MQRC_CONNECTION_BROKEN;
        return 0;
    }
    if (value != 0) {
        *hconn = value;
        return 1;
    }
    if (implicit_hconn == MQHC_UNUSABLE_HCONN)
        connect_native(&implicit_hconn, cc, reason);
    if (implicit_hconn == MQHC_UNUSABLE_HCONN) return 0;
    *hconn = implicit_hconn;
    return 1;
}

int MQCONN(char *qmgr, int32_t *hconn, int32_t *cc, int32_t *reason)
{
    MQHCONN handle;
    MQLONG native_cc, native_reason;
    (void)qmgr; /* A literal need not be padded; MQ_QMGR_NAME selects QM1. */
    connect_native(&handle, &native_cc, &native_reason);
    write_i(hconn, native_cc == MQCC_OK ? handle : 0);
    result(cc, reason, native_cc, native_reason);
    return 0;
}

int MQDISC(int32_t *hconn, int32_t *cc, int32_t *reason)
{
    MQHCONN handle = read_i(hconn);
    MQLONG native_cc = MQCC_FAILED, native_reason = MQRC_HCONN_ERROR;
    if (load_native() && handle != 0) {
        native_disc(&handle, &native_cc, &native_reason);
        if (native_cc == MQCC_OK) write_i(hconn, 0);
    }
    result(cc, reason, native_cc, native_reason);
    return 0;
}

int MQOPEN(int32_t *hconn, cob_mqod *od, int32_t *options,
           int32_t *hobj, int32_t *cc, int32_t *reason)
{
    MQOD native_od;
    MQHCONN handle;
    MQHOBJ object = MQHO_UNUSABLE_HOBJ;
    MQLONG native_cc = MQCC_FAILED, native_reason = MQRC_HCONN_ERROR;
    MQLONG flags = read_i(options);
    if (connection(hconn, &handle, &native_cc, &native_reason)) {
        make_od(&native_od, od);
        if (!has_value(native_od.ObjectName, sizeof(native_od.ObjectName))) {
            native_reason = MQRC_UNKNOWN_OBJECT_NAME;
        } else {
            /* Context belongs to the CICS unit of work, unavailable here. */
            flags &= ~(MQOO_SAVE_ALL_CONTEXT | MQOO_PASS_ALL_CONTEXT);
            if (!(flags & (MQOO_INPUT_SHARED | MQOO_INPUT_EXCLUSIVE |
                           MQOO_OUTPUT | MQOO_BROWSE)))
                flags |= MQOO_INPUT_SHARED | MQOO_OUTPUT;
            native_open(handle, &native_od, flags, &object,
                        &native_cc, &native_reason);
        }
    }
    write_i(hobj, native_cc == MQCC_OK ? object : 0);
    result(cc, reason, native_cc, native_reason);
    return 0;
}

int MQCLOSE(int32_t *hconn, int32_t *hobj, int32_t *options,
            int32_t *cc, int32_t *reason)
{
    MQHCONN handle;
    MQHOBJ object = read_i(hobj);
    MQLONG native_cc = MQCC_FAILED, native_reason = MQRC_HCONN_ERROR;
    if (connection(hconn, &handle, &native_cc, &native_reason)) {
        native_close(handle, &object, read_i(options),
                     &native_cc, &native_reason);
        if (native_cc == MQCC_OK) write_i(hobj, 0);
    }
    result(cc, reason, native_cc, native_reason);
    return 0;
}

static MQPMO make_pmo(const cob_mqpmo *from)
{
    MQPMO defaults = {MQPMO_DEFAULT};
    MQLONG options = read_i(&from->options);
    defaults.Options = (options & ~MQPMO_SYNCPOINT) | MQPMO_NO_SYNCPOINT;
    return defaults;
}

int MQPUT(int32_t *hconn, int32_t *hobj, cob_mqmd *md, cob_mqpmo *pmo,
          int32_t *buffer_length, char *buffer, int32_t *cc, int32_t *reason)
{
    MQHCONN handle;
    MQMD native_md;
    MQPMO native_pmo;
    MQLONG native_cc = MQCC_FAILED, native_reason = MQRC_HCONN_ERROR;
    MQLONG length = read_i(buffer_length);
    if (length < 0) {
        native_reason = MQRC_OPTIONS_ERROR;
    } else if (connection(hconn, &handle, &native_cc, &native_reason)) {
        make_md(&native_md, md);
        native_pmo = make_pmo(pmo);
        native_put(handle, read_i(hobj), &native_md, &native_pmo,
                   length, buffer, &native_cc, &native_reason);
        if (native_cc == MQCC_OK) save_md(md, &native_md);
    }
    result(cc, reason, native_cc, native_reason);
    return 0;
}

int MQPUT1(int32_t *hconn, cob_mqod *od, cob_mqmd *md, cob_mqpmo *pmo,
           int32_t *buffer_length, char *buffer, int32_t *cc, int32_t *reason)
{
    MQHCONN handle;
    MQOD native_od;
    MQMD native_md;
    MQPMO native_pmo;
    MQLONG native_cc = MQCC_FAILED, native_reason = MQRC_HCONN_ERROR;
    MQLONG length = read_i(buffer_length);
    if (length < 0) {
        native_reason = MQRC_OPTIONS_ERROR;
    } else if (connection(hconn, &handle, &native_cc, &native_reason)) {
        make_od(&native_od, od);
        make_md(&native_md, md);
        native_pmo = make_pmo(pmo);
        native_put1(handle, &native_od, &native_md, &native_pmo,
                    length, buffer, &native_cc, &native_reason);
        if (native_cc == MQCC_OK) save_md(md, &native_md);
    }
    result(cc, reason, native_cc, native_reason);
    return 0;
}

int MQGET(int32_t *hconn, int32_t *hobj, cob_mqmd *md, cob_mqgmo *gmo,
          int32_t *buffer_length, char *buffer, int32_t *data_length,
          int32_t *cc, int32_t *reason)
{
    MQHCONN handle;
    MQMD native_md;
    MQGMO native_gmo = {MQGMO_DEFAULT};
    MQLONG native_length = 0;
    MQLONG native_cc = MQCC_FAILED, native_reason = MQRC_HCONN_ERROR;
    MQLONG length = read_i(buffer_length);
    write_i(data_length, 0);
    if (length < 0) {
        native_reason = MQRC_OPTIONS_ERROR;
    } else if (connection(hconn, &handle, &native_cc, &native_reason)) {
        make_md(&native_md, md);
        native_gmo.Version = MQGMO_VERSION_2;
        native_gmo.Options = (read_i(&gmo->options) & ~MQGMO_SYNCPOINT) |
                             MQGMO_NO_SYNCPOINT;
        native_gmo.WaitInterval = read_i(&gmo->wait_interval);
        native_gmo.MatchOptions = 0;
        if (has_value(md->msg_id, sizeof(md->msg_id)))
            native_gmo.MatchOptions |= MQMO_MATCH_MSG_ID;
        if (has_value(md->correl_id, sizeof(md->correl_id)))
            native_gmo.MatchOptions |= MQMO_MATCH_CORREL_ID;
        native_get(handle, read_i(hobj), &native_md, &native_gmo, length,
                   buffer, &native_length, &native_cc, &native_reason);
        write_i(data_length, native_length);
        if (native_cc == MQCC_OK) save_md(md, &native_md);
    }
    result(cc, reason, native_cc, native_reason);
    return 0;
}
