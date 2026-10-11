/* PostgreSQL BYTEA-backed CICS KSDS. Uses the embedded SQL task connection. */
#include "pg_vsam.h"
#include <libpq-fe.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern PGconn *PGSQL_CONNECTION(void);

static int error_code(PGresult *result, const char *operation) {
    const char *state = result ? PQresultErrorField(result, PG_DIAG_SQLSTATE) : NULL;
    fprintf(stderr, "CICS VSAM PostgreSQL %s: %s", operation,
            result ? PQresultErrorMessage(result) : "connection unavailable\n");
    return state && !strcmp(state, "23505") ? 14 : 17;
}

static int command(PGconn *connection, const char *sql) {
    PGresult *result = PQexec(connection, sql);
    int okay = result && PQresultStatus(result) == PGRES_COMMAND_OK;
    if (!okay) error_code(result, sql);
    PQclear(result);
    return okay;
}

static PGresult *execute(PGconn *connection, const char *sql, int count,
                         const unsigned char *first, size_t first_size,
                         const unsigned char *second, size_t second_size) {
    Oid types[2] = {17, 17}; /* BYTEA */
    const char *values[2] = {(const char *)first, (const char *)second};
    int lengths[2] = {(int)first_size, (int)second_size};
    int formats[2] = {1, 1};
    return PQexecParams(connection, sql, count, types, values, lengths, formats, 1);
}

static int changed(PGresult *result) {
    const char *count = PQcmdTuples(result);
    return count && !strcmp(count, "1") ? 0 : 13;
}

int cobcics_pg_read(const char *table, size_t record_size, size_t key_size,
                    size_t key_offset, const unsigned char *key, size_t compare_size,
                    int gteq, int update, unsigned char *found_key,
                    unsigned char *record) {
    PGconn *connection = PGSQL_CONNECTION();
    if (!connection) return 17;
    char sql[256];
    int length = snprintf(sql, sizeof sql, "SELECT K,V FROM %s WHERE K %s $1%s%s",
        table, gteq ? ">=" : "=", gteq ? " ORDER BY K LIMIT 1" : "",
        update ? " FOR UPDATE" : "");
    if (length < 0 || (size_t)length >= sizeof sql) return 17;
    PGresult *result = execute(connection, sql, 1, key, compare_size, NULL, 0);
    if (!result || PQresultStatus(result) != PGRES_TUPLES_OK) {
        int code = error_code(result, "READ");
        PQclear(result);
        return code;
    }
    if (!PQntuples(result)) { PQclear(result); return 13; }
    if (PQntuples(result) != 1 || PQnfields(result) != 2 ||
        PQgetisnull(result, 0, 0) || PQgetisnull(result, 0, 1) ||
        PQgetlength(result, 0, 0) != (int)key_size ||
        PQgetlength(result, 0, 1) != (int)record_size) {
        PQclear(result);
        return 17;
    }
    memcpy(found_key, PQgetvalue(result, 0, 0), key_size);
    memcpy(record, PQgetvalue(result, 0, 1), record_size);
    PQclear(result);
    return memcmp(record + key_offset, found_key, key_size) ? 17 : 0;
}

static int mutate(const char *table, const char *operation,
                  const unsigned char *first, size_t first_size,
                  const unsigned char *second, size_t second_size) {
    PGconn *connection = PGSQL_CONNECTION();
    if (!connection || !command(connection, "SAVEPOINT cobcics_vsam_op")) return 17;
    char sql[256];
    if (!strcmp(operation, "WRITE"))
        snprintf(sql, sizeof sql, "INSERT INTO %s (K,V) VALUES ($1,$2)", table);
    else if (!strcmp(operation, "REWRITE"))
        snprintf(sql, sizeof sql, "UPDATE %s SET V=$1 WHERE K=$2", table);
    else
        snprintf(sql, sizeof sql, "DELETE FROM %s WHERE K=$1", table);
    int count = !strcmp(operation, "DELETE") ? 1 : 2;
    PGresult *result = execute(connection, sql, count, first, first_size, second, second_size);
    int code;
    if (!result || PQresultStatus(result) != PGRES_COMMAND_OK) {
        code = error_code(result, operation);
        PQclear(result);
        if (!command(connection, "ROLLBACK TO SAVEPOINT cobcics_vsam_op")) return 17;
    } else {
        code = !strcmp(operation, "WRITE") ? 0 : changed(result);
        PQclear(result);
    }
    return command(connection, "RELEASE SAVEPOINT cobcics_vsam_op") ? code : 17;
}

int cobcics_pg_write(const char *table, size_t record_size, size_t key_size,
                     size_t key_offset, const unsigned char *record) {
    return mutate(table, "WRITE", record + key_offset, key_size, record, record_size);
}

int cobcics_pg_rewrite(const char *table, size_t record_size, size_t key_size,
                       const unsigned char *key, const unsigned char *record) {
    return mutate(table, "REWRITE", record, record_size, key, key_size);
}

int cobcics_pg_delete(const char *table, size_t key_size,
                      const unsigned char *key) {
    return mutate(table, "DELETE", key, key_size, NULL, 0);
}
