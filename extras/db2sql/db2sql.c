/* Optional Db2 CLI bridge for the first embedded SQL milestone.
 * Build as a shared library and load with COB_PRE_LOAD.
 */
#include <ctype.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

#include <sqlcli.h>
#include <sqlcli1.h>
#include "libcob.h"

#define DB2_MAX_PARAMS 99
#define DB2_MAX_CURSORS 32
#define DB2_SQL_SIZE 8192

struct cursor_slot {
    char name[65];
    SQLHSTMT statement;
    char *inputs[DB2_MAX_PARAMS];
    SQLLEN indicators[DB2_MAX_PARAMS];
    int input_count;
};

static SQLHENV db2_env = SQL_NULL_HENV;
static SQLHDBC db2_connection = SQL_NULL_HDBC;
static struct cursor_slot cursors[DB2_MAX_CURSORS];
static int code_parameter = 3;
static int state_parameter = 4;

static int
success(SQLRETURN value)
{
    return value == SQL_SUCCESS || value == SQL_SUCCESS_WITH_INFO;
}

static int
supported_field(const cob_field *field)
{
    unsigned char type = COB_FIELD_TYPE(field);
    if (type == COB_TYPE_ALPHANUMERIC) return 1;
    return type == COB_TYPE_NUMERIC_DISPLAY ||
           type == COB_TYPE_NUMERIC_BINARY ||
           type == COB_TYPE_NUMERIC_PACKED ||
           type == COB_TYPE_NUMERIC_COMP5;
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
        if (*cursor == '.' && !dot) {
            dot = 1;
        } else if (isdigit(*cursor)) {
            if (dot) ++after;
            else ++before;
        } else {
            return 0;
        }
        ++cursor;
    }
    return before + after > 0 && scale >= 0 && precision >= scale &&
           before <= (size_t)(precision - scale) &&
           after <= (size_t)scale;
}

static int
valid_indicator(int parameter)
{
    cob_field *field = cob_get_param_field(parameter, "DB2SQL");
    return COB_FIELD_IS_NUMERIC(field) &&
           COB_FIELD_HAVE_SIGN(field) &&
           cob_get_field_scale(field) == 0;
}

static void
status_at(int code_param, int state_param, int code, const char *state)
{
    cob_put_s64_param(code_param, code);
    cob_put_param_str(state_param, state);
}

static void
status(int code, const char *state)
{
    status_at(code_parameter, state_parameter, code, state);
}

static int
decode_meta(const char *meta, int first, int *inputs, int *outputs,
            int *value_params, int *indicator_params)
{
    int index, parameter = first;
    if (sscanf(meta, "%2d,%2d", inputs, outputs) != 2 ||
        *inputs < 0 || *inputs > DB2_MAX_PARAMS ||
        *outputs < 0 || *outputs > DB2_MAX_PARAMS) {
        status(-99997, "HY009");
        return -1;
    }
    for (index = 0; index < *inputs + *outputs; ++index) {
        char flag = meta[5 + index];
        if (flag != '0' && flag != '1' && flag != ' ' && flag != 0) {
            status(-99997, "HY009");
            return -1;
        }
        value_params[index] = parameter++;
        indicator_params[index] = flag == '1' ? parameter++ : 0;
    }
    if (cob_get_num_params() != parameter - 1) {
        status(-99997, "HY009");
        return -1;
    }
    return 0;
}

static void
diagnostic(SQLSMALLINT kind, SQLHANDLE handle)
{
    SQLCHAR state[6] = "HY000";
    SQLCHAR message[512] = "Db2 CLI operation failed";
    SQLINTEGER native = -99999;
    SQLSMALLINT size = 0;
    if (handle != SQL_NULL_HANDLE &&
        success(SQLGetDiagRec(kind, handle, 1, state, &native, message,
                              sizeof(message), &size))) {
        fprintf(stderr, "Db2 SQLSTATE %.5s SQLCODE %d: %s\n",
                state, (int)native, message);
    }
    status(native != 0 ? (int)native : -99999, (const char *)state);
}

static const char *
required_env(const char *key)
{
    const char *value = getenv(key);
    return value && *value ? value : NULL;
}

static int
connect_db2(void)
{
    const char *dsn = required_env("DB2_DSN");
    const char *host = required_env("DB2_HOST");
    const char *database = required_env("DB2_DATABASE");
    const char *user = required_env("DB2_USER");
    const char *password = required_env("DB2_PASSWORD");
    const char *port = required_env("DB2_PORT");
    SQLRETURN result;
    char connection_string[2048];
    int size;
    const char *stage = "SQLAllocHandle(ENV)";

    if (db2_connection != SQL_NULL_HDBC) {
        return 0;
    }
    if (!user || !password || (!dsn && (!host || !database))) {
        fprintf(stderr, "Db2: DB2_USER, DB2_PASSWORD and DB2_DSN or "
                        "DB2_HOST/DB2_DATABASE are required\n");
        status(-99998, "08001");
        return -1;
    }
    if (!port) port = "50000";
    if (dsn) {
        size = snprintf(connection_string, sizeof(connection_string),
                        "DSN=%s;UID=%s;PWD=%s", dsn, user, password);
    } else {
        size = snprintf(connection_string, sizeof(connection_string),
                        "DATABASE=%s;HOSTNAME=%s;PORT=%s;PROTOCOL=TCPIP;"
                        "UID=%s;PWD=%s", database, host, port, user, password);
    }
    if (size < 0 || (size_t)size >= sizeof(connection_string)) {
        status(-99998, "08001");
        return -1;
    }
    result = SQLAllocHandle(SQL_HANDLE_ENV, SQL_NULL_HANDLE, &db2_env);
    if (!success(result)) {
        fprintf(stderr, "Db2 %s failed (CLI return %d)\n", stage, (int)result);
        status(-99999, "08001");
        goto failure;
    }
    stage = "SQLAllocHandle(DBC)";
    result = SQLAllocHandle(SQL_HANDLE_DBC, db2_env, &db2_connection);
    if (!success(result)) {
        fprintf(stderr, "Db2 %s failed (CLI return %d)\n", stage, (int)result);
        diagnostic(SQL_HANDLE_ENV, db2_env);
        goto failure;
    }
    stage = "SQLDriverConnect";
    result = SQLDriverConnect(db2_connection, NULL,
                              (SQLCHAR *)connection_string, SQL_NTS,
                              NULL, 0, NULL, SQL_DRIVER_NOPROMPT);
    memset(connection_string, 0, sizeof(connection_string));
    if (!success(result)) {
        fprintf(stderr, "Db2 %s failed (CLI return %d)\n", stage, (int)result);
        diagnostic(SQL_HANDLE_DBC, db2_connection);
        goto failure;
    }
    stage = "SQLSetConnectAttr";
    result = SQLSetConnectAttr(db2_connection, SQL_ATTR_AUTOCOMMIT,
                               (SQLPOINTER)SQL_AUTOCOMMIT_OFF, 0);
    if (!success(result)) {
        fprintf(stderr, "Db2 %s failed (CLI return %d)\n", stage, (int)result);
        diagnostic(SQL_HANDLE_DBC, db2_connection);
        goto failure;
    }
    const char *schema = required_env("DB2_SCHEMA");
    if (schema) {
        size_t length = strlen(schema);
        if (length == 0 || length > 128 ||
            !((schema[0] >= 'A' && schema[0] <= 'Z') ||
              (schema[0] >= 'a' && schema[0] <= 'z'))) {
            status(-99998, "08001");
            goto failure;
        }
        for (size_t index = 1; index < length; ++index) {
            char ch = schema[index];
            if (!((ch >= 'A' && ch <= 'Z') ||
                  (ch >= 'a' && ch <= 'z') ||
                  (ch >= '0' && ch <= '9') || ch == '_')) {
                status(-99998, "08001");
                goto failure;
            }
        }
        SQLHSTMT setting = SQL_NULL_HSTMT;
        stage = "SQLAllocHandle(STMT)";
        result = SQLAllocHandle(SQL_HANDLE_STMT, db2_connection, &setting);
        if (!success(result)) {
            diagnostic(SQL_HANDLE_DBC, db2_connection);
            goto failure;
        }
        char command[160];
        snprintf(command, sizeof command, "SET CURRENT SCHEMA %s", schema);
        stage = "SQLExecDirect(SET CURRENT SCHEMA)";
        result = SQLExecDirect(setting, (SQLCHAR *)command, SQL_NTS);
        if (!success(result))
            diagnostic(SQL_HANDLE_STMT, setting);
        SQLFreeHandle(SQL_HANDLE_STMT, setting);
        if (!success(result))
            goto failure;
    }
    if (getenv("DB2_SQL_TRACE"))
        fprintf(stderr, "Db2 connected: connection=%lu\n",
                (unsigned long)db2_connection);
    return 0;

failure:
    if (result != SQL_ERROR && !success(result))
        fprintf(stderr, "Db2 %s failed (CLI return %d)\n", stage, (int)result);
    if (db2_connection != SQL_NULL_HDBC) {
        SQLFreeHandle(SQL_HANDLE_DBC, db2_connection);
        db2_connection = SQL_NULL_HDBC;
    }
    if (db2_env != SQL_NULL_HENV) {
        SQLFreeHandle(SQL_HANDLE_ENV, db2_env);
        db2_env = SQL_NULL_HENV;
    }
    return -1;
}

static int
bind_inputs(SQLHSTMT statement, int count, const int *value_params,
            const int *indicator_params, char **buffers, SQLLEN *indicators)
{
    int index;
    for (index = 0; index < count; ++index) {
        int parameter = value_params[index];
        cob_field *field = cob_get_param_field(parameter, "DB2SQL");
        size_t capacity = (size_t)cob_get_field_size(field) * 3 + 64;
        int numeric = COB_FIELD_IS_NUMERIC(field);
        SQLRETURN result;
        if (!supported_field(field)) {
            status(-99990, "07006");
            return -1;
        }
        if (capacity > 65536) {
            status(-99996, "22001");
            return -1;
        }
        buffers[index] = calloc(capacity, 1);
        if (!buffers[index]) {
            status(-99995, "HY001");
            return -1;
        }
        if (indicator_params[index]) {
            if (!valid_indicator(indicator_params[index])) {
                status(-99990, "07006");
                return -1;
            }
            cob_s64_t value = cob_get_s64_param(indicator_params[index]);
            if (value != -1 && value != 0) {
                status(-99987, "22002");
                return -1;
            }
            indicators[index] = value == -1 ? SQL_NULL_DATA : SQL_NTS;
        } else {
            indicators[index] = SQL_NTS;
        }
        if (indicators[index] != SQL_NULL_DATA)
            cob_get_param_str(parameter, buffers[index], capacity);
        result = SQLBindParameter(statement, (SQLUSMALLINT)(index + 1),
                                  SQL_PARAM_INPUT, SQL_C_CHAR,
                                  numeric ? SQL_DECIMAL : SQL_VARCHAR,
                                  numeric ? (SQLULEN)cob_get_field_digits(field)
                                          : (SQLULEN)cob_get_field_size(field),
                                  numeric ? (SQLSMALLINT)cob_get_field_scale(field) : 0,
                                  buffers[index], (SQLLEN)capacity,
                                  &indicators[index]);
        if (!success(result)) {
            diagnostic(SQL_HANDLE_STMT, statement);
            return -1;
        }
    }
    return 0;
}

static int
read_outputs(SQLHSTMT statement, int count, const int *value_params,
             const int *indicator_params)
{
    int index;
    for (index = 0; index < count; ++index) {
        char value[8192];
        SQLLEN length = 0;
        SQLRETURN result = SQLGetData(statement, (SQLUSMALLINT)(index + 1),
                                      SQL_C_CHAR, value, sizeof(value), &length);
        cob_field *field;
        if (indicator_params[index] &&
            !valid_indicator(indicator_params[index])) {
            status(-99990, "07006");
            return -1;
        }
        if (length == SQL_NULL_DATA) {
            if (!indicator_params[index]) {
                status(-305, "22002");
                return -1;
            }
            cob_put_s64_param(indicator_params[index], -1);
            continue;
        }
        if (!success(result) ||
            length == SQL_NO_TOTAL || length >= (SQLLEN)sizeof(value)) {
            if (success(result)) status(-99992, "22001");
            else diagnostic(SQL_HANDLE_STMT, statement);
            return -1;
        }
        if (indicator_params[index])
            cob_put_s64_param(indicator_params[index], 0);
        field = cob_get_param_field(value_params[index], "DB2SQL");
        if (!supported_field(field)) {
            status(-99990, "07006");
            return -1;
        }
        if (COB_FIELD_IS_NUMERIC(field) && !number_fits(field, value)) {
            status(-99989, "22003");
            return -1;
        }
        int put_result = cob_put_param_str(value_params[index], value);
        if (put_result != 0) {
            if (put_result == ERANGE) status(-99992, "22001");
            else status(-99991, "22018");
            return -1;
        }
    }
    return 0;
}

static struct cursor_slot *
find_cursor(const char *name)
{
    int index;
    for (index = 0; index < DB2_MAX_CURSORS; ++index)
        if (cursors[index].statement != SQL_NULL_HSTMT &&
            strcmp(cursors[index].name, name) == 0)
            return &cursors[index];
    return NULL;
}

static void
release_cursor(struct cursor_slot *slot)
{
    int index;
    if (slot->statement != SQL_NULL_HSTMT)
        SQLFreeHandle(SQL_HANDLE_STMT, slot->statement);
    for (index = 0; index < slot->input_count; ++index)
        free(slot->inputs[index]);
    memset(slot, 0, sizeof(*slot));
}

static void
release_all_cursors(void)
{
    int index;
    for (index = 0; index < DB2_MAX_CURSORS; ++index)
        release_cursor(&cursors[index]);
}

/* CALL "DB2SQL_EXEC" USING sql-text "ii,oo" SQLCODE SQLSTATE
 *                           input-fields... output-fields... */
int
DB2SQL_EXEC(void)
{
    char sql[DB2_SQL_SIZE + 1];
    char meta[204];
    int value_params[DB2_MAX_PARAMS * 2];
    int indicator_params[DB2_MAX_PARAMS * 2];
    char *inputs[DB2_MAX_PARAMS] = {0};
    SQLLEN indicators[DB2_MAX_PARAMS] = {0};
    SQLHSTMT statement = SQL_NULL_HSTMT;
    SQLRETURN result;
    SQLSMALLINT columns = 0;
    int input_count, output_count, index;
    int ret = 1;
    size_t sql_length;
    const char *stage = "SQLPrepare";

    code_parameter = 3;
    state_parameter = 4;
    status(0, "00000");
    if (getenv("DB2_SQL_TRACE"))
        fprintf(stderr, "Db2 entry: state=%p connection=%lu\n",
                (void *)&db2_connection, (unsigned long)db2_connection);
    if (cob_get_num_params() < 4 ||
        !cob_get_param_str(1, sql, sizeof(sql)) ||
        !cob_get_param_str(2, meta, sizeof(meta)) ||
        decode_meta(meta, 5, &input_count, &output_count,
                    value_params, indicator_params) != 0) {
        status(-99997, "HY009");
        return 1;
    }
    sql_length = strlen(sql);
    while (sql_length && sql[sql_length - 1] == ' ') sql[--sql_length] = 0;
    if (getenv("DB2_SQL_TRACE"))
        fprintf(stderr, "Db2 SQL: %s, inputs=%d outputs=%d\n",
                sql, input_count, output_count);
    if (connect_db2() != 0) return 1;
    result = SQLAllocHandle(SQL_HANDLE_STMT, db2_connection, &statement);
    if (!success(result)) {
        fprintf(stderr, "Db2 SQLAllocHandle(STMT) failed (CLI return %d)\n",
                (int)result);
        diagnostic(SQL_HANDLE_DBC, db2_connection);
        return 1;
    }
    result = SQLPrepare(statement, (SQLCHAR *)sql, SQL_NTS);
    if (!success(result)) goto sql_error;

    if (bind_inputs(statement, input_count, value_params, indicator_params,
                    inputs, indicators) != 0)
        goto cleanup;
    stage = "SQLExecute";
    result = SQLExecute(statement);
    if (result == SQL_NO_DATA &&
        (!strncasecmp(sql, "UPDATE ", 7) ||
         !strncasecmp(sql, "DELETE ", 7))) {
        status(100, "02000");
        ret = 0;
        goto cleanup;
    }
    if (!success(result)) goto sql_error;
    if (!strncasecmp(sql, "UPDATE ", 7) ||
        !strncasecmp(sql, "DELETE ", 7)) {
        SQLLEN changed = 0;
        result = SQLRowCount(statement, &changed);
        if (!success(result)) goto sql_error;
        if (changed == 0) {
            status(100, "02000");
            ret = 0;
            goto cleanup;
        }
    }
    stage = "SQLNumResultCols";
    result = SQLNumResultCols(statement, &columns);
    if (!success(result)) goto sql_error;
    if (columns != output_count) {
        status(-99994, "07002");
        goto cleanup;
    }
    if (output_count) {
        stage = "SQLFetch";
        result = SQLFetch(statement);
        if (result == SQL_NO_DATA) {
            status(100, "02000");
            ret = 0;
            goto cleanup;
        }
        if (!success(result)) goto sql_error;
        if (read_outputs(statement, output_count,
                         value_params + input_count,
                         indicator_params + input_count) != 0)
            goto cleanup;
        result = SQLFetch(statement);
        if (result != SQL_NO_DATA) {
            if (success(result)) status(-811, "21000");
            else diagnostic(SQL_HANDLE_STMT, statement);
            goto cleanup;
        }
    }
    status(0, "00000");
    ret = 0;
    goto cleanup;

sql_error:
    fprintf(stderr, "Db2 %s failed (CLI return %d)\n", stage, (int)result);
    diagnostic(SQL_HANDLE_STMT, statement);
cleanup:
    for (index = 0; index < input_count; ++index) free(inputs[index]);
    SQLFreeHandle(SQL_HANDLE_STMT, statement);
    if (getenv("DB2_SQL_TRACE"))
        fprintf(stderr, "Db2 exit: connection=%lu code=%d\n",
                (unsigned long)db2_connection, ret);
    return ret;
}

int
DB2SQL_OPEN(void)
{
    char name[65], sql[DB2_SQL_SIZE + 1], meta[204];
    int value_params[DB2_MAX_PARAMS * 2];
    int indicator_params[DB2_MAX_PARAMS * 2];
    int input_count, output_count, index;
    SQLSMALLINT columns = 0;
    SQLRETURN result;
    struct cursor_slot *slot = NULL;
    code_parameter = 4;
    state_parameter = 5;
    status(0, "00000");
    if (cob_get_num_params() < 5 ||
        !cob_get_param_str(1, name, sizeof(name)) ||
        !cob_get_param_str(2, sql, sizeof(sql)) ||
        !cob_get_param_str(3, meta, sizeof(meta)) ||
        decode_meta(meta, 6, &input_count, &output_count,
                    value_params, indicator_params) != 0 ||
        output_count != 0) {
        status(-99997, "HY009");
        return 1;
    }
    name[strcspn(name, " ")] = 0;
    for (index = (int)strlen(sql) - 1; index >= 0 && sql[index] == ' '; --index)
        sql[index] = 0;
    if (!*name || find_cursor(name)) {
        status(-502, "24502");
        return 1;
    }
    for (index = 0; index < DB2_MAX_CURSORS; ++index)
        if (cursors[index].statement == SQL_NULL_HSTMT) {
            slot = &cursors[index];
            break;
        }
    if (!slot) {
        status(-99988, "HY001");
        return 1;
    }
    if (connect_db2() != 0) return 1;
    snprintf(slot->name, sizeof(slot->name), "%s", name);
    slot->input_count = input_count;
    result = SQLAllocHandle(SQL_HANDLE_STMT, db2_connection, &slot->statement);
    if (!success(result)) {
        diagnostic(SQL_HANDLE_DBC, db2_connection);
        goto failure;
    }
    result = SQLPrepare(slot->statement, (SQLCHAR *)sql, SQL_NTS);
    if (!success(result)) goto sql_error;
    if (bind_inputs(slot->statement, input_count,
                    value_params, indicator_params,
                    slot->inputs, slot->indicators) != 0)
        goto failure;
    result = SQLExecute(slot->statement);
    if (!success(result)) goto sql_error;
    result = SQLNumResultCols(slot->statement, &columns);
    if (!success(result)) goto sql_error;
    if (columns == 0) {
        status(-99994, "07002");
        goto failure;
    }
    status(0, "00000");
    return 0;
sql_error:
    diagnostic(SQL_HANDLE_STMT, slot->statement);
failure:
    release_cursor(slot);
    return 1;
}

int
DB2SQL_FETCH(void)
{
    char name[65], meta[204];
    int value_params[DB2_MAX_PARAMS * 2];
    int indicator_params[DB2_MAX_PARAMS * 2];
    int input_count, output_count;
    SQLSMALLINT columns = 0;
    SQLRETURN result;
    struct cursor_slot *slot;
    code_parameter = 3;
    state_parameter = 4;
    status(0, "00000");
    if (cob_get_num_params() < 4 ||
        !cob_get_param_str(1, name, sizeof(name)) ||
        !cob_get_param_str(2, meta, sizeof(meta)) ||
        decode_meta(meta, 5, &input_count, &output_count,
                    value_params, indicator_params) != 0 ||
        input_count != 0 || output_count < 1) {
        status(-99997, "HY009");
        return 1;
    }
    name[strcspn(name, " ")] = 0;
    slot = find_cursor(name);
    if (!slot) {
        status(-501, "24501");
        return 1;
    }
    result = SQLNumResultCols(slot->statement, &columns);
    if (!success(result)) goto sql_error;
    if (columns != output_count) {
        status(-99994, "07002");
        return 1;
    }
    result = SQLFetch(slot->statement);
    if (result == SQL_NO_DATA) {
        status(100, "02000");
        return 0;
    }
    if (!success(result)) goto sql_error;
    if (read_outputs(slot->statement, output_count,
                     value_params, indicator_params) != 0) return 1;
    status(0, "00000");
    return 0;
sql_error:
    diagnostic(SQL_HANDLE_STMT, slot->statement);
    return 1;
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
    release_cursor(slot);
    return 0;
}

static int
end_transaction(SQLSMALLINT operation)
{
    SQLRETURN result;
    code_parameter = 1;
    state_parameter = 2;
    status_at(1, 2, 0, "00000");
    if (connect_db2() != 0) return 1;
    result = SQLEndTran(SQL_HANDLE_DBC, db2_connection, operation);
    if (!success(result)) {
        diagnostic(SQL_HANDLE_DBC, db2_connection);
        return 1;
    }
    release_all_cursors();
    return 0;
}

int
DB2SQL_COMMIT(void)
{
    return end_transaction(SQL_COMMIT);
}

int
DB2SQL_ROLLBACK(void)
{
    return end_transaction(SQL_ROLLBACK);
}

int
DB2SQL_CLOSE(void)
{
    release_all_cursors();
    if (db2_connection != SQL_NULL_HDBC) {
        SQLDisconnect(db2_connection);
        SQLFreeHandle(SQL_HANDLE_DBC, db2_connection);
        db2_connection = SQL_NULL_HDBC;
    }
    if (db2_env != SQL_NULL_HENV) {
        SQLFreeHandle(SQL_HANDLE_ENV, db2_env);
        db2_env = SQL_NULL_HENV;
    }
    return 0;
}

/* CICS workers commit once at normal task end and back out on failure.
   A task with no SQL must not open a Db2 connection just to finish. */
int
DB2SQL_END_TASK(int commit)
{
    int result = 0;
    if (db2_connection != SQL_NULL_HDBC) {
        SQLRETURN transaction = SQLEndTran(
            SQL_HANDLE_DBC, db2_connection,
            commit ? SQL_COMMIT : SQL_ROLLBACK);
        if (!success(transaction)) {
            diagnostic(SQL_HANDLE_DBC, db2_connection);
            result = 1;
        }
    }
    DB2SQL_CLOSE();
    return result;
}
