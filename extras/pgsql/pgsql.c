/* PostgreSQL implementation of the embedded-SQL COBOL call ABI.
 * The shared SQL translator emits DB2SQL_* entry points for both backends;
 * load this library instead of libdb2sql.so for --postgres-sql programs.
 */
#include <ctype.h>
#include <errno.h>
#include <libpq-fe.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

#include "libcob.h"

#define MAX_PARAMS 99
#define MAX_CURSORS 32
#define SQL_SIZE 8192

struct cursor_slot {
    char name[65];
    PGresult *result;
    int next_row;
};

static PGconn *connection;
static struct cursor_slot cursors[MAX_CURSORS];
static int code_parameter = 3;
static int state_parameter = 4;

static void
status(int code, const char *state)
{
    cob_put_s64_param(code_parameter, code);
    cob_put_param_str(state_parameter, state);
}

static int
supported_field(const cob_field *field)
{
    unsigned char type = COB_FIELD_TYPE(field);
    return type == COB_TYPE_ALPHANUMERIC || type == COB_TYPE_GROUP ||
           type == COB_TYPE_NUMERIC_DISPLAY || type == COB_TYPE_NUMERIC_BINARY ||
           type == COB_TYPE_NUMERIC_PACKED || type == COB_TYPE_NUMERIC_COMP5;
}

static int
valid_indicator(int parameter)
{
    cob_field *field = cob_get_param_field(parameter, "DB2SQL");
    return COB_FIELD_IS_NUMERIC(field) && COB_FIELD_HAVE_SIGN(field) &&
           cob_get_field_scale(field) == 0;
}

static int
number_fits(const cob_field *field, const char *value)
{
    const unsigned char *cursor = (const unsigned char *)value;
    size_t before = 0, after = 0;
    int dot = 0;
    int scale = cob_get_field_scale(field);
    int precision = cob_get_field_digits(field);
    if (*cursor == '-' || *cursor == '+') {
        if (*cursor == '-' && !COB_FIELD_HAVE_SIGN(field)) return 0;
        ++cursor;
    }
    if (!*cursor) return 0;
    while (*cursor) {
        if (*cursor == '.' && !dot) dot = 1;
        else if (isdigit(*cursor)) {
            if (dot) ++after;
            else ++before;
        } else return 0;
        ++cursor;
    }
    return before + after > 0 && scale >= 0 && precision >= scale &&
           before <= (size_t)(precision - scale) && after <= (size_t)scale;
}

static int
decode_meta(const char *meta, int first, int *inputs, int *outputs,
            int *values, int *indicators)
{
    int parameter = first;
    if (sscanf(meta, "%2d,%2d", inputs, outputs) != 2 ||
        *inputs < 0 || *inputs > MAX_PARAMS ||
        *outputs < 0 || *outputs > MAX_PARAMS ||
        *inputs + *outputs > MAX_PARAMS * 2) {
        status(-99997, "HY009");
        return -1;
    }
    for (int index = 0; index < *inputs + *outputs; ++index) {
        char flag = meta[5 + index];
        if (flag != '0' && flag != '1' && flag != ' ' && flag != 0) {
            status(-99997, "HY009");
            return -1;
        }
        values[index] = parameter++;
        indicators[index] = flag == '1' ? parameter++ : 0;
    }
    if (cob_get_num_params() != parameter - 1) {
        status(-99997, "HY009");
        return -1;
    }
    return 0;
}

static int
sqlcode_for_state(const char *state)
{
    if (!strcmp(state, "23505")) return -803;
    if (!strcmp(state, "23503")) return -530;
    if (!strcmp(state, "23502")) return -407;
    if (!strcmp(state, "22003")) return -302;
    return -99999;
}

static void
diagnostic(PGresult *result)
{
    const char *state = result ? PQresultErrorField(result, PG_DIAG_SQLSTATE) : NULL;
    if (!state || strlen(state) != 5) state = "HY000";
    fprintf(stderr, "PostgreSQL SQLSTATE %s: %s", state,
            result ? PQresultErrorMessage(result) : PQerrorMessage(connection));
    status(sqlcode_for_state(state), state);
}

static int
ensure_connection(void)
{
    char password[256] = {0};
    const char *password_file = getenv("PG_PASSWORD_FILE");
    const char *conninfo = getenv("PG_CONNINFO");
    if (connection && PQstatus(connection) == CONNECTION_OK) return 0;
    if (connection) {
        PQfinish(connection);
        connection = NULL;
    }
    if (password_file && *password_file) {
        FILE *file = fopen(password_file, "rb");
        size_t length;
        if (!file) {
            fprintf(stderr, "PostgreSQL password file cannot be opened\n");
            status(-99998, "08001");
            return -1;
        }
        length = fread(password, 1, sizeof(password) - 1, file);
        fclose(file);
        while (length && (password[length - 1] == '\r' ||
                          password[length - 1] == '\n')) --length;
        password[length] = 0;
        if (!length || length == sizeof(password) - 1) {
            fprintf(stderr, "PostgreSQL password file is empty or too long\n");
            status(-99998, "08001");
            memset(password, 0, sizeof(password));
            return -1;
        }
        if (conninfo && *conninfo) {
            const char *keys[] = {"dbname", "password", NULL};
            const char *values[] = {conninfo, password, NULL};
            connection = PQconnectdbParams(keys, values, 1);
        } else {
            const char *keys[] = {"password", NULL};
            const char *values[] = {password, NULL};
            connection = PQconnectdbParams(keys, values, 0);
        }
        memset(password, 0, sizeof(password));
    } else connection = PQconnectdb(conninfo ? conninfo : "");
    if (!connection || PQstatus(connection) != CONNECTION_OK) {
        fprintf(stderr, "PostgreSQL connection failed: %s",
                connection ? PQerrorMessage(connection) : "out of memory\n");
        status(-99998, "08001");
        return -1;
    }
    PGresult *begin = PQexec(connection, "BEGIN");
    if (!begin || PQresultStatus(begin) != PGRES_COMMAND_OK) {
        diagnostic(begin);
        PQclear(begin);
        PQfinish(connection);
        connection = NULL;
        return -1;
    }
    PQclear(begin);
    return 0;
}

/* CICS VSAM shares the embedded-SQL connection and task transaction. */
PGconn *
PGSQL_CONNECTION(void)
{
    return ensure_connection() == 0 ? connection : NULL;
}

/* The existing SQL translator emits question-mark host placeholders. Only
 * unquoted markers become libpq's numbered $1, $2, ... placeholders. */
static int
postgres_sql(const char *source, char *target, size_t capacity, int count)
{
    size_t used = 0;
    int parameter = 0;
    char quote = 0;
    for (size_t index = 0; source[index]; ++index) {
        char ch = source[index];
        if (used + 5 >= capacity) return -1;
        if (quote) {
            target[used++] = ch;
            if (ch == quote) {
                if (source[index + 1] == quote)
                    target[used++] = source[++index];
                else quote = 0;
            }
        } else if (ch == '\'' || ch == '"') {
            quote = ch;
            target[used++] = ch;
        } else if (ch == '?') {
            int size = snprintf(target + used, capacity - used, "$%d", ++parameter);
            if (size < 0 || (size_t)size >= capacity - used) return -1;
            used += (size_t)size;
        } else target[used++] = ch;
    }
    target[used] = 0;
    return quote || parameter != count ? -1 : 0;
}

static void
free_inputs(char **buffers, int count)
{
    for (int index = 0; index < count; ++index) free(buffers[index]);
}

static int
bind_inputs(int count, const int *values, const int *indicators,
            char **buffers, const char **parameters)
{
    for (int index = 0; index < count; ++index) {
        int parameter = values[index];
        cob_field *field = cob_get_param_field(parameter, "DB2SQL");
        size_t capacity = (size_t)cob_get_field_size(field) * 3 + 64;
        if (!supported_field(field)) {
            status(-99990, "07006");
            return -1;
        }
        if (capacity > 65536) {
            status(-99996, "22001");
            return -1;
        }
        if (indicators[index]) {
            if (!valid_indicator(indicators[index])) {
                status(-99990, "07006");
                return -1;
            }
            cob_s64_t indicator = cob_get_s64_param(indicators[index]);
            if (indicator == -1) {
                parameters[index] = NULL;
                continue;
            }
            if (indicator != 0) {
                status(-99987, "22002");
                return -1;
            }
        }
        buffers[index] = calloc(capacity, 1);
        if (!buffers[index]) {
            status(-99995, "HY001");
            return -1;
        }
        if (COB_FIELD_TYPE(field) == COB_TYPE_GROUP) {
            const unsigned char *data = cob_get_param_data(parameter);
            size_t size = (size_t)cob_get_field_size(field);
            size_t length;
            if (size < 3 || (length = ((size_t)data[0] << 8) | data[1]) > size - 2 ||
                length + 1 > capacity) {
                status(-99996, "22001");
                return -1;
            }
            memcpy(buffers[index], data + 2, length);
        } else {
            const unsigned char *raw = cob_get_param_data(parameter);
            size_t size = (size_t)cob_get_field_size(field);
            const unsigned char *low = COB_FIELD_IS_NUMERIC(field) ? NULL :
                                       memchr(raw, 0, size);
            if (low) {
                cob_field prefix = *field;
                prefix.size = (size_t)(low - raw);
                if (prefix.size)
                    cob_get_field_str(&prefix, buffers[index], capacity);
            } else cob_get_param_str(parameter, buffers[index], capacity);
        }
        parameters[index] = buffers[index];
    }
    return 0;
}

static int
read_outputs(PGresult *result, int row, int count,
             const int *values, const int *indicators)
{
    for (int index = 0; index < count; ++index) {
        cob_field *field = cob_get_param_field(values[index], "DB2SQL");
        if (!supported_field(field) ||
            (indicators[index] && !valid_indicator(indicators[index]))) {
            status(-99990, "07006");
            return -1;
        }
        if (PQgetisnull(result, row, index)) {
            if (!indicators[index]) {
                status(-305, "22002");
                return -1;
            }
            cob_put_s64_param(indicators[index], -1);
            continue;
        }
        if (indicators[index]) cob_put_s64_param(indicators[index], 0);
        const char *value = PQgetvalue(result, row, index);
        size_t length = (size_t)PQgetlength(result, row, index);
        if (COB_FIELD_TYPE(field) == COB_TYPE_GROUP) {
            unsigned char *data = cob_get_param_data(values[index]);
            size_t size = (size_t)cob_get_field_size(field);
            if (size < 3 || length > size - 2 || length > 32767) {
                status(-99992, "22001");
                return -1;
            }
            data[0] = (unsigned char)(length >> 8);
            data[1] = (unsigned char)length;
            memset(data + 2, ' ', size - 2);
            memcpy(data + 2, value, length);
        } else {
            if (COB_FIELD_IS_NUMERIC(field) && !number_fits(field, value)) {
                status(-99989, "22003");
                return -1;
            }
            int rc = cob_put_param_str(values[index], value);
            if (rc) {
                status(rc == ERANGE ? -99992 : -99991,
                       rc == ERANGE ? "22001" : "22018");
                return -1;
            }
        }
    }
    return 0;
}

static void
release_cursors(void)
{
    for (int index = 0; index < MAX_CURSORS; ++index) {
        PQclear(cursors[index].result);
        memset(&cursors[index], 0, sizeof(cursors[index]));
    }
}

static struct cursor_slot *
find_cursor(const char *name)
{
    for (int index = 0; index < MAX_CURSORS; ++index)
        if (cursors[index].result && !strcmp(cursors[index].name, name))
            return &cursors[index];
    return NULL;
}

int
DB2SQL_EXEC(void)
{
    char sql[SQL_SIZE + 1], converted[SQL_SIZE + 400], meta[204];
    int values[MAX_PARAMS * 2], indicators[MAX_PARAMS * 2];
    char *buffers[MAX_PARAMS] = {0};
    const char *parameters[MAX_PARAMS] = {0};
    int inputs, outputs, rc = 1;
    PGresult *result = NULL;
    code_parameter = 3;
    state_parameter = 4;
    status(0, "00000");
    if (cob_get_num_params() < 4 ||
        !cob_get_param_str(1, sql, sizeof(sql)) ||
        !cob_get_param_str(2, meta, sizeof(meta)) ||
        decode_meta(meta, 5, &inputs, &outputs, values, indicators) ||
        postgres_sql(sql, converted, sizeof(converted), inputs)) {
        status(-99997, "HY009");
        return 1;
    }
    if (ensure_connection()) return 1;
    if (bind_inputs(inputs, values, indicators, buffers, parameters)) goto done;
    result = PQexecParams(connection, converted, inputs, NULL, parameters,
                          NULL, NULL, 0);
    if (!result || (PQresultStatus(result) != PGRES_COMMAND_OK &&
                    PQresultStatus(result) != PGRES_TUPLES_OK)) {
        diagnostic(result);
        goto done;
    }
    if (PQnfields(result) != outputs) {
        status(-99994, "07002");
        goto done;
    }
    if (outputs) {
        int rows = PQntuples(result);
        if (!rows) status(100, "02000");
        else if (rows > 1) {
            status(-811, "21000");
            goto done;
        } else if (read_outputs(result, 0, outputs,
                                values + inputs, indicators + inputs)) goto done;
    } else if ((!strncasecmp(converted, "UPDATE ", 7) ||
                !strncasecmp(converted, "DELETE ", 7)) &&
               !strcmp(PQcmdTuples(result), "0")) status(100, "02000");
    rc = 0;
done:
    PQclear(result);
    free_inputs(buffers, inputs);
    return rc;
}

int
DB2SQL_OPEN(void)
{
    char name[65], sql[SQL_SIZE + 1], converted[SQL_SIZE + 400], meta[204];
    int values[MAX_PARAMS * 2], indicators[MAX_PARAMS * 2];
    char *buffers[MAX_PARAMS] = {0};
    const char *parameters[MAX_PARAMS] = {0};
    int inputs, outputs, rc = 1;
    struct cursor_slot *slot = NULL;
    PGresult *result = NULL;
    code_parameter = 4;
    state_parameter = 5;
    status(0, "00000");
    if (cob_get_num_params() < 5 ||
        !cob_get_param_str(1, name, sizeof(name)) ||
        !cob_get_param_str(2, sql, sizeof(sql)) ||
        !cob_get_param_str(3, meta, sizeof(meta)) ||
        decode_meta(meta, 6, &inputs, &outputs, values, indicators) || outputs ||
        postgres_sql(sql, converted, sizeof(converted), inputs)) {
        status(-99997, "HY009");
        return 1;
    }
    name[strcspn(name, " ")] = 0;
    if (!*name || find_cursor(name)) {
        status(-502, "24502");
        return 1;
    }
    for (int index = 0; index < MAX_CURSORS; ++index)
        if (!cursors[index].result) {
            slot = &cursors[index];
            break;
        }
    if (!slot) {
        status(-99988, "HY001");
        return 1;
    }
    if (ensure_connection()) return 1;
    if (bind_inputs(inputs, values, indicators, buffers, parameters)) goto done;
    result = PQexecParams(connection, converted, inputs, NULL, parameters,
                          NULL, NULL, 0);
    if (!result || PQresultStatus(result) != PGRES_TUPLES_OK) {
        diagnostic(result);
        goto done;
    }
    if (PQnfields(result) < 1) {
        status(-99994, "07002");
        goto done;
    }
    snprintf(slot->name, sizeof(slot->name), "%s", name);
    slot->result = result;
    slot->next_row = 0;
    result = NULL;
    rc = 0;
done:
    PQclear(result);
    free_inputs(buffers, inputs);
    return rc;
}

int
DB2SQL_FETCH(void)
{
    char name[65], meta[204];
    int values[MAX_PARAMS * 2], indicators[MAX_PARAMS * 2];
    int inputs, outputs;
    struct cursor_slot *slot;
    code_parameter = 3;
    state_parameter = 4;
    status(0, "00000");
    if (cob_get_num_params() < 4 ||
        !cob_get_param_str(1, name, sizeof(name)) ||
        !cob_get_param_str(2, meta, sizeof(meta)) ||
        decode_meta(meta, 5, &inputs, &outputs, values, indicators) ||
        inputs || outputs < 1) {
        status(-99997, "HY009");
        return 1;
    }
    name[strcspn(name, " ")] = 0;
    slot = find_cursor(name);
    if (!slot) {
        status(-501, "24501");
        return 1;
    }
    if (PQnfields(slot->result) != outputs) {
        status(-99994, "07002");
        return 1;
    }
    if (slot->next_row >= PQntuples(slot->result)) {
        status(100, "02000");
        return 0;
    }
    if (read_outputs(slot->result, slot->next_row, outputs,
                     values, indicators)) return 1;
    ++slot->next_row;
    return 0;
}

int
DB2SQL_CLOSE_CURSOR(void)
{
    char name[65];
    struct cursor_slot *slot;
    code_parameter = 2;
    state_parameter = 3;
    status(0, "00000");
    if (cob_get_num_params() != 3 ||
        !cob_get_param_str(1, name, sizeof(name))) {
        status(-99997, "HY009");
        return 1;
    }
    name[strcspn(name, " ")] = 0;
    slot = find_cursor(name);
    if (!slot) {
        status(-501, "24501");
        return 1;
    }
    PQclear(slot->result);
    memset(slot, 0, sizeof(*slot));
    return 0;
}

static int
transaction(const char *command)
{
    code_parameter = 1;
    state_parameter = 2;
    status(0, "00000");
    if (ensure_connection()) return 1;
    PGresult *result = PQexec(connection, command);
    if (!result || PQresultStatus(result) != PGRES_COMMAND_OK) {
        diagnostic(result);
        PQclear(result);
        return 1;
    }
    PQclear(result);
    release_cursors();
    result = PQexec(connection, "BEGIN");
    if (!result || PQresultStatus(result) != PGRES_COMMAND_OK) {
        diagnostic(result);
        PQclear(result);
        return 1;
    }
    PQclear(result);
    return 0;
}

int DB2SQL_COMMIT(void) { return transaction("COMMIT"); }
int DB2SQL_ROLLBACK(void) { return transaction("ROLLBACK"); }

int
DB2SQL_CLOSE(void)
{
    release_cursors();
    if (connection) PQfinish(connection);
    connection = NULL;
    return 0;
}

int
DB2SQL_END_TASK(int commit)
{
    int rc = 0;
    if (connection) {
        PGresult *result = PQexec(connection, commit ? "COMMIT" : "ROLLBACK");
        if (!result || PQresultStatus(result) != PGRES_COMMAND_OK) {
            diagnostic(result);
            rc = 1;
        }
        PQclear(result);
    }
    DB2SQL_CLOSE();
    return rc;
}
