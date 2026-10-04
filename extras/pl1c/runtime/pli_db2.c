#include "pli_db2.h"

#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <sqlcli.h>
#include <sqlcli1.h>

#define MAX_CURSORS 32
#define MAX_PARAMS 99

typedef struct {
    char name[65];
    SQLHSTMT statement;
    char *input_text[MAX_PARAMS];
    SQLLEN input_length[MAX_PARAMS];
} cursor_slot;

static SQLHENV environment = SQL_NULL_HENV;
static SQLHDBC connection = SQL_NULL_HDBC;
static cursor_slot cursors[MAX_CURSORS];

static int success(SQLRETURN result)
{
    return result == SQL_SUCCESS || result == SQL_SUCCESS_WITH_INFO;
}

static void status(pli_value *code, pli_value *state, int number, const char *sqlstate)
{
    char digits[32];
    snprintf(digits, sizeof digits, "%d", number);
    pli_assign(code, pli_number(digits));
    pli_assign(state, pli_literal(sqlstate));
}

static void diagnostic(pli_value *code, pli_value *state, SQLSMALLINT kind, SQLHANDLE handle)
{
    SQLCHAR sqlstate[6] = "HY000", message[512] = "Db2 CLI error";
    SQLINTEGER native = -99999;
    SQLSMALLINT length = 0;
    if (handle != SQL_NULL_HANDLE)
        SQLGetDiagRec(kind, handle, 1, sqlstate, &native, message,
                      sizeof message, &length);
    fprintf(stderr, "PL/I Db2 SQLSTATE %.5s SQLCODE %d: %s\n",
            sqlstate, (int)native, message);
    status(code, state, native ? (int)native : -99999, (const char *)sqlstate);
}

static const char *required_env(const char *name)
{
    const char *value = getenv(name);
    return value && *value ? value : NULL;
}

static int connect_db2(pli_value *code, pli_value *state)
{
    const char *dsn = required_env("DB2_DSN");
    const char *host = required_env("DB2_HOST");
    const char *port = required_env("DB2_PORT");
    const char *database = required_env("DB2_DATABASE");
    const char *user = required_env("DB2_USER");
    const char *password = required_env("DB2_PASSWORD");
    const char *schema = required_env("DB2_SCHEMA");
    char connection_string[2048];
    SQLRETURN result;
    int size;
    if (connection != SQL_NULL_HDBC) return 1;
    if (!user || !password || (!dsn && (!host || !database))) {
        status(code, state, -99998, "08001");
        return 0;
    }
    if (!port) port = "50000";
    if (dsn)
        size = snprintf(connection_string, sizeof connection_string,
                        "DSN=%s;UID=%s;PWD=%s", dsn, user, password);
    else
        size = snprintf(connection_string, sizeof connection_string,
                        "DATABASE=%s;HOSTNAME=%s;PORT=%s;PROTOCOL=TCPIP;UID=%s;PWD=%s",
                        database, host, port, user, password);
    if (size < 0 || (size_t)size >= sizeof connection_string) {
        status(code, state, -99998, "08001");
        return 0;
    }
    result = SQLAllocHandle(SQL_HANDLE_ENV, SQL_NULL_HANDLE, &environment);
    if (!success(result)) {
        status(code, state, -99999, "08001");
        return 0;
    }
    result = SQLAllocHandle(SQL_HANDLE_DBC, environment, &connection);
    if (!success(result)) {
        diagnostic(code, state, SQL_HANDLE_ENV, environment);
        goto failure;
    }
    result = SQLDriverConnect(connection, NULL, (SQLCHAR *)connection_string,
                              SQL_NTS, NULL, 0, NULL, SQL_DRIVER_NOPROMPT);
    memset(connection_string, 0, sizeof connection_string);
    if (!success(result)) {
        diagnostic(code, state, SQL_HANDLE_DBC, connection);
        goto failure;
    }
    result = SQLSetConnectAttr(connection, SQL_ATTR_AUTOCOMMIT,
                               (SQLPOINTER)SQL_AUTOCOMMIT_OFF, 0);
    if (!success(result)) {
        diagnostic(code, state, SQL_HANDLE_DBC, connection);
        goto failure;
    }
    if (schema) {
        SQLHSTMT statement = SQL_NULL_HSTMT;
        char command[160];
        size_t i, n = strlen(schema);
        if (!n || n > 128 || !isalpha((unsigned char)schema[0])) {
            status(code, state, -99998, "08001");
            goto failure;
        }
        for (i = 1; i < n; ++i)
            if (!isalnum((unsigned char)schema[i]) && schema[i] != '_') {
                status(code, state, -99998, "08001");
                goto failure;
            }
        snprintf(command, sizeof command, "SET CURRENT SCHEMA %s", schema);
        result = SQLAllocHandle(SQL_HANDLE_STMT, connection, &statement);
        if (!success(result)) {
            diagnostic(code, state, SQL_HANDLE_DBC, connection);
            goto failure;
        }
        result = SQLExecDirect(statement, (SQLCHAR *)command, SQL_NTS);
        if (!success(result)) diagnostic(code, state, SQL_HANDLE_STMT, statement);
        SQLFreeHandle(SQL_HANDLE_STMT, statement);
        if (!success(result)) goto failure;
    }
    status(code, state, 0, "00000");
    return 1;
failure:
    if (connection != SQL_NULL_HDBC) {
        SQLFreeHandle(SQL_HANDLE_DBC, connection);
        connection = SQL_NULL_HDBC;
    }
    if (environment != SQL_NULL_HENV) {
        SQLFreeHandle(SQL_HANDLE_ENV, environment);
        environment = SQL_NULL_HENV;
    }
    return 0;
}

static void release_cursor(cursor_slot *slot)
{
    size_t i;
    if (slot->statement != SQL_NULL_HSTMT)
        SQLFreeHandle(SQL_HANDLE_STMT, slot->statement);
    for (i = 0; i < MAX_PARAMS; ++i) free(slot->input_text[i]);
    memset(slot, 0, sizeof *slot);
}

static cursor_slot *find_cursor(const char *name)
{
    size_t i;
    for (i = 0; i < MAX_CURSORS; ++i)
        if (cursors[i].statement != SQL_NULL_HSTMT &&
            strcmp(cursors[i].name, name) == 0)
            return &cursors[i];
    return NULL;
}

static int bind_inputs(pli_value *code, pli_value *state, SQLHSTMT statement,
                       size_t count, const pli_sql_param *inputs,
                       cursor_slot *slot)
{
    size_t i;
    if (count > MAX_PARAMS) {
        status(code, state, -99997, "HY009");
        return 0;
    }
    for (i = 0; i < count; ++i) {
        pli_value value = *inputs[i].value;
        SQLSMALLINT sql_type = value.kind == PLI_CHAR ? SQL_VARCHAR : SQL_DECIMAL;
        SQLULEN precision = value.kind == PLI_CHAR ? (SQLULEN)value.length :
                            (SQLULEN)value.precision;
        SQLRETURN result;
        if (inputs[i].indicator && inputs[i].indicator->number < 0) {
            slot->input_length[i] = SQL_NULL_DATA;
            slot->input_text[i] = calloc(1, 1);
        } else {
            if (value.kind != PLI_CHAR) value = pli_char_builtin(value);
            slot->input_text[i] = calloc(value.length + 1, 1);
            if (slot->input_text[i])
                memcpy(slot->input_text[i], value.chars, value.length);
            slot->input_length[i] = (SQLLEN)value.length;
        }
        if (!slot->input_text[i]) {
            status(code, state, -99999, "HY001");
            return 0;
        }
        result = SQLBindParameter(statement, (SQLUSMALLINT)(i + 1),
                                  SQL_PARAM_INPUT, SQL_C_CHAR, sql_type,
                                  precision, (SQLSMALLINT)inputs[i].value->scale,
                                  slot->input_text[i],
                                  (SQLLEN)(strlen(slot->input_text[i]) + 1),
                                  &slot->input_length[i]);
        if (!success(result)) {
            diagnostic(code, state, SQL_HANDLE_STMT, statement);
            return 0;
        }
    }
    return 1;
}

static int read_outputs(pli_value *code, pli_value *state, SQLHSTMT statement,
                        size_t count, const pli_sql_param *outputs)
{
    size_t i;
    if (count > MAX_PARAMS) {
        status(code, state, -99997, "HY009");
        return 0;
    }
    for (i = 0; i < count; ++i) {
        char buffer[8192];
        SQLLEN length = 0;
        SQLRETURN result = SQLGetData(statement, (SQLUSMALLINT)(i + 1),
                                      SQL_C_CHAR, buffer, sizeof buffer, &length);
        if (length == SQL_NULL_DATA) {
            if (!outputs[i].indicator) {
                status(code, state, -305, "22002");
                return 0;
            }
            pli_assign(outputs[i].indicator, pli_number("-1"));
            continue;
        }
        if (!success(result) || length < 0 || length >= (SQLLEN)sizeof buffer) {
            if (success(result)) status(code, state, -99992, "22001");
            else diagnostic(code, state, SQL_HANDLE_STMT, statement);
            return 0;
        }
        if (outputs[i].indicator)
            pli_assign(outputs[i].indicator, pli_number("0"));
        if (outputs[i].value->kind == PLI_CHAR) {
            pli_value source = pli_storage_char((size_t)length);
            memcpy(source.chars, buffer, (size_t)length);
            pli_assign(outputs[i].value, source);
        } else {
            pli_assign(outputs[i].value, pli_number(buffer));
        }
    }
    return 1;
}

void pli_db2_open(pli_value *code, pli_value *state, const char *name,
                  const char *sql, size_t input_count, const pli_sql_param *inputs)
{
    cursor_slot *slot = NULL;
    SQLRETURN result;
    size_t i;
    if (!connect_db2(code, state)) return;
    if (strlen(name) > 64 || find_cursor(name)) {
        status(code, state, -99995, "34000");
        return;
    }
    for (i = 0; i < MAX_CURSORS; ++i)
        if (cursors[i].statement == SQL_NULL_HSTMT) {
            slot = &cursors[i];
            break;
        }
    if (!slot) {
        status(code, state, -99995, "34000");
        return;
    }
    result = SQLAllocHandle(SQL_HANDLE_STMT, connection, &slot->statement);
    if (!success(result)) {
        diagnostic(code, state, SQL_HANDLE_DBC, connection);
        return;
    }
    strcpy(slot->name, name);
    result = SQLPrepare(slot->statement, (SQLCHAR *)sql, SQL_NTS);
    if (!success(result)) goto error;
    if (!bind_inputs(code, state, slot->statement, input_count, inputs, slot)) {
        release_cursor(slot);
        return;
    }
    result = SQLExecute(slot->statement);
    if (!success(result)) goto error;
    status(code, state, 0, "00000");
    return;
error:
    diagnostic(code, state, SQL_HANDLE_STMT, slot->statement);
    release_cursor(slot);
}

void pli_db2_fetch(pli_value *code, pli_value *state, const char *name,
                   size_t output_count, const pli_sql_param *outputs)
{
    cursor_slot *slot = find_cursor(name);
    SQLRETURN result;
    if (!slot) {
        status(code, state, -99995, "34000");
        return;
    }
    result = SQLFetch(slot->statement);
    if (result == SQL_NO_DATA) {
        status(code, state, 100, "02000");
        return;
    }
    if (!success(result)) {
        diagnostic(code, state, SQL_HANDLE_STMT, slot->statement);
        return;
    }
    if (read_outputs(code, state, slot->statement, output_count, outputs))
        status(code, state, 0, "00000");
}

void pli_db2_close(pli_value *code, pli_value *state, const char *name)
{
    cursor_slot *slot = find_cursor(name);
    if (!slot) {
        status(code, state, -99995, "34000");
        return;
    }
    release_cursor(slot);
    status(code, state, 0, "00000");
}

void pli_db2_select(pli_value *code, pli_value *state, const char *sql,
                    size_t input_count, const pli_sql_param *inputs,
                    size_t output_count, const pli_sql_param *outputs)
{
    cursor_slot temporary = {0};
    SQLRETURN result;
    if (!connect_db2(code, state)) return;
    result = SQLAllocHandle(SQL_HANDLE_STMT, connection, &temporary.statement);
    if (!success(result)) {
        diagnostic(code, state, SQL_HANDLE_DBC, connection);
        return;
    }
    result = SQLPrepare(temporary.statement, (SQLCHAR *)sql, SQL_NTS);
    if (!success(result)) goto error;
    if (!bind_inputs(code, state, temporary.statement, input_count, inputs, &temporary))
        goto cleanup;
    result = SQLExecute(temporary.statement);
    if (!success(result)) goto error;
    result = SQLFetch(temporary.statement);
    if (result == SQL_NO_DATA) {
        status(code, state, 100, "02000");
        goto cleanup;
    }
    if (!success(result)) goto error;
    if (!read_outputs(code, state, temporary.statement, output_count, outputs))
        goto cleanup;
    result = SQLFetch(temporary.statement);
    if (result != SQL_NO_DATA) {
        if (success(result)) status(code, state, -811, "21000");
        else goto error;
    } else {
        status(code, state, 0, "00000");
    }
    goto cleanup;
error:
    diagnostic(code, state, SQL_HANDLE_STMT, temporary.statement);
cleanup:
    release_cursor(&temporary);
}

void pli_db2_commit(pli_value *code, pli_value *state)
{
    SQLRETURN result;
    size_t i;
    if (!connect_db2(code, state)) return;
    for (i = 0; i < MAX_CURSORS; ++i) release_cursor(&cursors[i]);
    result = SQLEndTran(SQL_HANDLE_DBC, connection, SQL_COMMIT);
    if (!success(result)) diagnostic(code, state, SQL_HANDLE_DBC, connection);
    else status(code, state, 0, "00000");
}
