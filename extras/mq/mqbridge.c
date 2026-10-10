/* COBOL MQI subset backed by IBM MQ's messaging REST API.
 * The companion copybooks define the ABI; IBM MQ files are not redistributed.
 */
#include <curl/curl.h>
#include <ctype.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifndef _WIN32
#include <strings.h>
#endif

#ifdef _WIN32
#define strcasecmp _stricmp
#define strncasecmp _strnicmp
#endif

#define MQ_OK 0
#define MQ_FAILED 2
#define MQRC_NO_MSG_AVAILABLE 2033
#define MQRC_NOT_AUTHORIZED 2035
#define MQRC_UNKNOWN_OBJECT_NAME 2085
#define MQRC_CONNECTION_BROKEN 2009
#define MQRC_TRUNCATED_MSG_FAILED 2080
#define MQRC_HOBJ_ERROR 2019
#define MQRC_OPTIONS_ERROR 2046

#pragma pack(push, 1)
typedef struct {
    char id[4];
    int32_t version;
    int32_t object_type;
    char object_name[48];
    char object_qmgr_name[48];
} mqod;
typedef struct {
    char id[4];
    int32_t version;
    int32_t msg_type;
    int32_t expiry;
    int32_t persistence;
    int32_t coded_char_set_id;
    char format[8];
    char msg_id[24];
    char correl_id[24];
    char reply_to_q[48];
    char reply_to_qmgr[48];
} mqmd;
typedef struct {
    char id[4];
    int32_t version;
    int32_t options;
    int32_t wait_interval;
} mqgmo;
typedef struct {
    char id[4];
    int32_t version;
    int32_t options;
} mqpmo;
#pragma pack(pop)

typedef struct {
    char *data;
    size_t size;
    char msg_id[24];
    char correl_id[24];
    char reply_to_q[48];
} mq_response;

static char handles[64][49];

/* IBM-style COBOL BINARY fields are stored most-significant byte first. */
static int32_t
read_i(const int32_t *field)
{
    const unsigned char *p = (const unsigned char *)field;
    return (int32_t)((uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 |
                     (uint32_t)p[2] << 8 | (uint32_t)p[3]);
}

static void
write_i(int32_t *field, int32_t value)
{
    unsigned char *p = (unsigned char *)field;
    uint32_t n = (uint32_t)value;
    p[0] = (unsigned char)(n >> 24);
    p[1] = (unsigned char)(n >> 16);
    p[2] = (unsigned char)(n >> 8);
    p[3] = (unsigned char)n;
}

static void
result(int32_t *cc, int32_t *reason, int32_t code)
{
    write_i(cc, code == 0 ? MQ_OK : MQ_FAILED);
    write_i(reason, code);
}

static void
trim_field(char *out, size_t out_size, const char *field, size_t field_size)
{
    size_t n = field_size;
    while (n && (field[n - 1] == ' ' || field[n - 1] == 0)) --n;
    if (n >= out_size) n = out_size - 1;
    memcpy(out, field, n);
    out[n] = 0;
}

static int
has_id(const char *bytes, size_t size)
{
    size_t i;
    for (i = 0; i < size; ++i)
        if (bytes[i] != 0 && bytes[i] != ' ') return 1;
    return 0;
}

static void
hex_id(char *out, const char *bytes)
{
    static const char digits[] = "0123456789abcdef";
    size_t i;
    for (i = 0; i < 24; ++i) {
        unsigned char c = (unsigned char)bytes[i];
        out[i * 2] = digits[c >> 4];
        out[i * 2 + 1] = digits[c & 15];
    }
    out[48] = 0;
}

static void
decode_id(char *out, const char *value)
{
    size_t i;
    if (!strncasecmp(value, "ID:", 3)) value += 3;
    if (strlen(value) < 48) return;
    for (i = 0; i < 24; ++i) {
        unsigned int byte;
        if (sscanf(value + i * 2, "%2x", &byte) != 1) return;
        out[i] = (char)byte;
    }
}

static size_t
body_callback(char *ptr, size_t size, size_t nmemb, void *context)
{
    mq_response *response = (mq_response *)context;
    size_t count = size * nmemb;
    char *next = (char *)realloc(response->data, response->size + count + 1);
    if (!next) return 0;
    response->data = next;
    memcpy(response->data + response->size, ptr, count);
    response->size += count;
    response->data[response->size] = 0;
    return count;
}

static size_t
header_callback(char *ptr, size_t size, size_t nmemb, void *context)
{
    mq_response *response = (mq_response *)context;
    size_t count = size * nmemb;
    char value[160];
    const char *start = NULL;
    size_t n;
    if (count > 21 && !strncasecmp(ptr, "ibm-mq-md-messageId:", 20)) {
        start = ptr + 20;
    } else if (count > 23 && !strncasecmp(ptr, "ibm-mq-md-correlationId:", 24)) {
        start = ptr + 24;
    } else if (count > 18 && !strncasecmp(ptr, "ibm-mq-md-replyTo:", 18)) {
        start = ptr + 18;
    }
    if (!start) return count;
    while (start < ptr + count && isspace((unsigned char)*start)) ++start;
    n = (size_t)(ptr + count - start);
    while (n && isspace((unsigned char)start[n - 1])) --n;
    if (n >= sizeof(value)) n = sizeof(value) - 1;
    memcpy(value, start, n);
    value[n] = 0;
    if (!strncasecmp(ptr, "ibm-mq-md-messageId:", 20))
        decode_id(response->msg_id, value);
    else if (!strncasecmp(ptr, "ibm-mq-md-correlationId:", 24))
        decode_id(response->correl_id, value);
    else {
        char *at = strchr(value, '@');
        if (at) *at = 0;
        memset(response->reply_to_q, ' ', sizeof(response->reply_to_q));
        n = strlen(value);
        if (n > sizeof(response->reply_to_q)) n = sizeof(response->reply_to_q);
        memcpy(response->reply_to_q, value, n);
    }
    return count;
}

static int
read_password(char *out, size_t size)
{
    const char *env = getenv("MQ_PASSWORD");
    const char *path = getenv("MQ_PASSWORD_FILE");
    FILE *file;
    size_t n;
    if (env && *env) {
        if (strlen(env) >= size) return 0;
        strcpy(out, env);
        return 1;
    }
    if (!path || !*path) return 0;
    file = fopen(path, "rb");
    if (!file) return 0;
    n = fread(out, 1, size - 1, file);
    fclose(file);
    while (n && (out[n - 1] == '\n' || out[n - 1] == '\r')) --n;
    out[n] = 0;
    return n > 0;
}

static int32_t
request(const char *method, const char *queue, mqmd *md,
        const char *body, int32_t body_size, int32_t wait_ms,
        mq_response *response)
{
    CURL *curl = NULL;
    struct curl_slist *headers = NULL;
    CURLcode curl_code;
    const char *base = getenv("MQ_REST_URL");
    const char *qmgr = getenv("MQ_QMGR_NAME");
    const char *user = getenv("MQ_USER");
    char password[256], url[1024], header[180], id[49];
    char *escaped_qmgr = NULL, *escaped_queue = NULL;
    long http_code = 0;
    int32_t reason = MQRC_CONNECTION_BROKEN;
    size_t length;
    memset(response, 0, sizeof(*response));
    if (!base || !*base) base = "https://localhost:9444";
    if (!qmgr || !*qmgr) qmgr = "QM1";
    if (!user || !*user) user = "app";
    if (!read_password(password, sizeof(password))) return MQRC_NOT_AUTHORIZED;
    curl = curl_easy_init();
    if (!curl) return reason;
    escaped_qmgr = curl_easy_escape(curl, qmgr, 0);
    escaped_queue = curl_easy_escape(curl, queue, 0);
    if (!escaped_qmgr || !escaped_queue) goto done;
    length = strlen(base);
    while (length && base[length - 1] == '/') --length;
    if (snprintf(url, sizeof(url), "%.*s/ibmmq/rest/v2/messaging/qmgr/%s/queue/%s/message",
                 (int)length, base, escaped_qmgr, escaped_queue) >= (int)sizeof(url))
        goto done;
    if (!strcmp(method, "DELETE")) {
        size_t used = strlen(url);
        if (wait_ms > 0) {
            snprintf(url + used, sizeof(url) - used, "?wait=%d", wait_ms);
            used = strlen(url);
        }
        if (md && has_id(md->correl_id, 24)) {
            hex_id(id, md->correl_id);
            snprintf(url + used, sizeof(url) - used, "%ccorrelationId=%s",
                     strchr(url, '?') ? '&' : '?', id);
            used = strlen(url);
        }
        if (md && has_id(md->msg_id, 24)) {
            hex_id(id, md->msg_id);
            snprintf(url + used, sizeof(url) - used, "%cmessageId=%s",
                     strchr(url, '?') ? '&' : '?', id);
        }
    }
    headers = curl_slist_append(headers, "ibm-mq-rest-csrf-token: cobmq");
    headers = curl_slist_append(headers, "Accept: text/plain");
    if (!strcmp(method, "POST")) {
        headers = curl_slist_append(headers, "Content-Type: text/plain; charset=utf-8");
        if (md && has_id(md->correl_id, 24)) {
            hex_id(id, md->correl_id);
            snprintf(header, sizeof(header), "ibm-mq-md-correlationId: %s", id);
            headers = curl_slist_append(headers, header);
        }
        if (md && has_id(md->reply_to_q, 48)) {
            char reply[49];
            trim_field(reply, sizeof(reply), md->reply_to_q, 48);
            snprintf(header, sizeof(header), "ibm-mq-md-replyTo: %s", reply);
            headers = curl_slist_append(headers, header);
        }
    }
    curl_easy_setopt(curl, CURLOPT_URL, url);
    curl_easy_setopt(curl, CURLOPT_USERNAME, user);
    curl_easy_setopt(curl, CURLOPT_PASSWORD, password);
    curl_easy_setopt(curl, CURLOPT_HTTPAUTH, CURLAUTH_BASIC);
    curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers);
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, body_callback);
    curl_easy_setopt(curl, CURLOPT_WRITEDATA, response);
    curl_easy_setopt(curl, CURLOPT_HEADERFUNCTION, header_callback);
    curl_easy_setopt(curl, CURLOPT_HEADERDATA, response);
    curl_easy_setopt(curl, CURLOPT_TIMEOUT_MS, wait_ms > 0 ? (long)wait_ms + 10000L : 10000L);
    if (getenv("MQ_TLS_INSECURE") && !strcmp(getenv("MQ_TLS_INSECURE"), "1")) {
        curl_easy_setopt(curl, CURLOPT_SSL_VERIFYPEER, 0L);
        curl_easy_setopt(curl, CURLOPT_SSL_VERIFYHOST, 0L);
    } else if (getenv("MQ_CA_BUNDLE")) {
        curl_easy_setopt(curl, CURLOPT_CAINFO, getenv("MQ_CA_BUNDLE"));
    }
    if (!strcmp(method, "POST")) {
        curl_easy_setopt(curl, CURLOPT_POST, 1L);
        curl_easy_setopt(curl, CURLOPT_POSTFIELDS, body);
        curl_easy_setopt(curl, CURLOPT_POSTFIELDSIZE_LARGE, (curl_off_t)body_size);
    } else {
        curl_easy_setopt(curl, CURLOPT_CUSTOMREQUEST, "DELETE");
    }
    curl_code = curl_easy_perform(curl);
    if (curl_code != CURLE_OK) goto done;
    curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &http_code);
    if (http_code == 204 && !strcmp(method, "DELETE"))
        reason = MQRC_NO_MSG_AVAILABLE;
    else if (http_code >= 200 && http_code < 300) reason = 0;
    else if (http_code == 401 || http_code == 403) reason = MQRC_NOT_AUTHORIZED;
    else if (http_code == 404)
        reason = !strcmp(method, "DELETE") ? MQRC_NO_MSG_AVAILABLE : MQRC_UNKNOWN_OBJECT_NAME;
    else if (http_code == 400) reason = MQRC_OPTIONS_ERROR;
done:
    if (escaped_qmgr) curl_free(escaped_qmgr);
    if (escaped_queue) curl_free(escaped_queue);
    curl_slist_free_all(headers);
    curl_easy_cleanup(curl);
    memset(password, 0, sizeof(password));
    return reason;
}

int
MQCONN(char *qmgr, int32_t *hconn, int32_t *cc, int32_t *reason)
{
    (void)qmgr;
    write_i(hconn, 1);
    result(cc, reason, 0);
    return 0;
}

int
MQDISC(int32_t *hconn, int32_t *cc, int32_t *reason)
{
    write_i(hconn, 0);
    result(cc, reason, 0);
    return 0;
}

int
MQOPEN(int32_t *hconn, mqod *od, int32_t *options, int32_t *hobj,
       int32_t *cc, int32_t *reason)
{
    char queue[49];
    int i;
    (void)hconn;
    (void)options;
    trim_field(queue, sizeof(queue), od->object_name, 48);
    if (!*queue) {
        result(cc, reason, MQRC_UNKNOWN_OBJECT_NAME);
        return 0;
    }
    for (i = 1; i < 64; ++i) {
        if (!handles[i][0]) {
            strcpy(handles[i], queue);
            write_i(hobj, i);
            result(cc, reason, 0);
            return 0;
        }
    }
    result(cc, reason, MQRC_HOBJ_ERROR);
    return 0;
}

int
MQCLOSE(int32_t *hconn, int32_t *hobj, int32_t *options,
        int32_t *cc, int32_t *reason)
{
    (void)hconn;
    (void)options;
    int32_t object = read_i(hobj);
    if (object <= 0 || object >= 64 || !handles[object][0])
        result(cc, reason, MQRC_HOBJ_ERROR);
    else {
        handles[object][0] = 0;
        write_i(hobj, 0);
        result(cc, reason, 0);
    }
    return 0;
}

int
MQPUT(int32_t *hconn, int32_t *hobj, mqmd *md, mqpmo *pmo,
      int32_t *buffer_length, char *buffer, int32_t *cc, int32_t *reason)
{
    mq_response response;
    int32_t code;
    (void)hconn;
    (void)pmo;
    int32_t object = read_i(hobj);
    int32_t length = read_i(buffer_length);
    if (object <= 0 || object >= 64 || !handles[object][0])
        code = MQRC_HOBJ_ERROR;
    else if (length < 0)
        code = MQRC_OPTIONS_ERROR;
    else {
        code = request("POST", handles[object], md, buffer,
                       length, 0, &response);
        if (code == 0 && has_id(response.msg_id, 24))
            memcpy(md->msg_id, response.msg_id, 24);
        free(response.data);
    }
    result(cc, reason, code);
    return 0;
}

int
MQPUT1(int32_t *hconn, mqod *od, mqmd *md, mqpmo *pmo,
       int32_t *buffer_length, char *buffer, int32_t *cc, int32_t *reason)
{
    mq_response response;
    char queue[49];
    int32_t code;
    (void)hconn;
    (void)pmo;
    trim_field(queue, sizeof(queue), od->object_name, 48);
    if (!*queue) code = MQRC_UNKNOWN_OBJECT_NAME;
    else if (read_i(buffer_length) < 0) code = MQRC_OPTIONS_ERROR;
    else {
        code = request("POST", queue, md, buffer,
                       read_i(buffer_length), 0, &response);
        if (code == 0 && has_id(response.msg_id, 24))
            memcpy(md->msg_id, response.msg_id, 24);
        free(response.data);
    }
    result(cc, reason, code);
    return 0;
}

int
MQGET(int32_t *hconn, int32_t *hobj, mqmd *md, mqgmo *gmo,
      int32_t *buffer_length, char *buffer, int32_t *data_length,
      int32_t *cc, int32_t *reason)
{
    mq_response response;
    int32_t code;
    (void)hconn;
    int32_t object = read_i(hobj);
    int32_t length = read_i(buffer_length);
    write_i(data_length, 0);
    if (object <= 0 || object >= 64 || !handles[object][0])
        code = MQRC_HOBJ_ERROR;
    else if (length < 0)
        code = MQRC_OPTIONS_ERROR;
    else {
        int32_t wait = read_i(&gmo->wait_interval);
        code = request("DELETE", handles[object], NULL, NULL, 0,
                       wait > 0 ? wait : 0,
                       &response);
        if (code == 0) {
            write_i(data_length, (int32_t)response.size);
            if (response.size > (size_t)length) {
                code = MQRC_TRUNCATED_MSG_FAILED;
            } else {
                memcpy(buffer, response.data, response.size);
                memcpy(md->msg_id, response.msg_id, 24);
                memcpy(md->correl_id, response.correl_id, 24);
                if (has_id(response.reply_to_q, 48))
                    memcpy(md->reply_to_q, response.reply_to_q, 48);
            }
        }
        free(response.data);
    }
    result(cc, reason, code);
    return 0;
}
