/* GenApp CICS KSDS operations backed by the embedded-SQL Db2 connection.
   Record and key columns are binary, so no CCSID conversion changes bytes. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "local_vsam.h"
#include <libcob.h>
#include <sqlcli.h>
#include <sqlcli1.h>

extern SQLHDBC DB2SQL_CONNECTION(void);

struct file_def {
    char name[8];
    const char *table;
    uint32_t key_size;
    uint32_t record_size;
    uint32_t key_offset;
};

static const struct file_def files[] = {
    {{'K','S','D','S','C','U','S','T'}, "KSDSCUST", 10, 225, 0},
    {{'K','S','D','S','P','O','L','Y'}, "KSDSPOLY", 21, 64, 0},
};

static _Thread_local struct {
    char name[8];
    unsigned char *key;
    size_t key_size;
    int active;
} held_update;

static int hold_update(const struct file_def *file, const unsigned char *key) {
    unsigned char *buffer = realloc(held_update.key, file->key_size);
    if (!buffer) return 0;
    held_update.key = buffer;
    held_update.key_size = file->key_size;
    memcpy(held_update.name, file->name, 8);
    memcpy(held_update.key, key, file->key_size);
    held_update.active = 1;
    return 1;
}

static uint32_t number(const unsigned char *value) {
    return (uint32_t)value[0] << 24 | (uint32_t)value[1] << 16 |
           (uint32_t)value[2] << 8 | value[3];
}

static void put_number(unsigned char *value, uint32_t number_value) {
    value[0] = (unsigned char)(number_value >> 24);
    value[1] = (unsigned char)(number_value >> 16);
    value[2] = (unsigned char)(number_value >> 8);
    value[3] = (unsigned char)number_value;
}

static void response(unsigned char *resp, unsigned char *resp2,
                     uint32_t code, uint32_t detail) {
    put_number(resp, code);
    put_number(resp2, detail);
}

static int success(SQLRETURN result) {
    return result == SQL_SUCCESS || result == SQL_SUCCESS_WITH_INFO;
}

static const char *path_for_name(const char *name) {
    char variable[32];
    size_t length = 8;
    while (length && name[length - 1] == ' ') --length;
    if (!length || length > 8) return NULL;
    for (size_t index = 0; index < length; ++index)
        if (!((name[index] >= 'A' && name[index] <= 'Z') ||
              (name[index] >= '0' && name[index] <= '9'))) return NULL;
    snprintf(variable, sizeof variable, "COBCICS_VSAM_%.*s", (int)length, name);
    const char *path = getenv(variable);
    return path && *path ? path : NULL;
}

static const struct file_def *file_for(const char *name) {
    char host_name[9];
    memcpy(host_name, name, 8);
    host_name[8] = '\0';
    if (cob_is_initialized() && (unsigned char)name[0] >= 0x80 &&
        !cob_get_param_str(1, host_name, sizeof host_name)) return NULL;
    for (size_t index = 0; index < sizeof files / sizeof files[0]; ++index)
        if (memcmp(host_name, files[index].name, 8) == 0)
            return &files[index];
    const char *path = path_for_name(host_name);
    if (!path) return NULL;
    size_t record_size, key_size, key_offset;
    if (cobcics_vsam_layout(path, &record_size, &key_size, &key_offset) ||
        !key_size || record_size > UINT32_MAX || key_size > UINT32_MAX ||
        key_offset > UINT32_MAX)
        return NULL;
    static _Thread_local struct file_def local_file;
    memcpy(local_file.name, host_name, 8);
    local_file.table = NULL;
    local_file.record_size = (uint32_t)record_size;
    local_file.key_size = (uint32_t)key_size;
    local_file.key_offset = (uint32_t)key_offset;
    return &local_file;
}

static const char *local_path(const struct file_def *file) {
    return file ? path_for_name(file->name) : NULL;
}

static const cob_charset *current_data_charset(void) {
    if (!cob_is_initialized())
        return &cob_charset_native;
    cob_global *global = cob_get_global_ptr();
    cob_module *module = global ? global->cob_current_module : NULL;
    return module && module->charset ? module->charset : &cob_charset_native;
}

static void copy_to_host(unsigned char *target, const unsigned char *source,
                         size_t length) {
    const unsigned char *table = current_data_charset()->to_host;
    for (size_t index = 0; index < length; ++index)
        target[index] = table ? table[source[index]] : source[index];
}

static void copy_from_host(unsigned char *target, const unsigned char *source,
                           size_t length) {
    const unsigned char *table = current_data_charset()->from_host;
    for (size_t index = 0; index < length; ++index)
        target[index] = table ? table[source[index]] : source[index];
}

static void sql_error(SQLHSTMT statement, const char *operation,
                      unsigned char *resp, unsigned char *resp2) {
    SQLCHAR state[6] = {0};
    SQLCHAR message[512] = {0};
    SQLINTEGER native = 0;
    SQLSMALLINT length = 0;
    if (SQLGetDiagRec(SQL_HANDLE_STMT, statement, 1, state, &native,
                      message, sizeof message, &length) == SQL_SUCCESS) {
        fprintf(stderr, "GenApp %s Db2 %s (%d): %s\n", operation,
                state, (int)native, message);
        response(resp, resp2, memcmp(state, "23505", 5) == 0 ? 14 : 17,
                 (uint32_t)(native < 0 ? -native : native));
    } else {
        fprintf(stderr, "GenApp %s Db2 error without diagnostic\n", operation);
        response(resp, resp2, 17, 0);
    }
}

static int prepare(SQLHDBC connection, const char *sql, SQLHSTMT *statement,
                   unsigned char *resp, unsigned char *resp2) {
    if (!connection || !success(SQLAllocHandle(SQL_HANDLE_STMT, connection,
                                                statement))) {
        response(resp, resp2, 17, 1);
        return 0;
    }
    if (!success(SQLPrepare(*statement, (SQLCHAR *)sql, SQL_NTS))) {
        sql_error(*statement, "prepare", resp, resp2);
        SQLFreeHandle(SQL_HANDLE_STMT, *statement);
        return 0;
    }
    return 1;
}

static int bind_bytes(SQLHSTMT statement, SQLUSMALLINT position,
                      const unsigned char *value, SQLLEN *length,
                      SQLSMALLINT kind, SQLULEN maximum) {
    return success(SQLBindParameter(statement, position, SQL_PARAM_INPUT,
        SQL_C_BINARY, kind, maximum, 0, (SQLPOINTER)value, *length, length));
}

/* READ mode: bit 0 GTEQ, bit 1 GENERIC, bit 2 UPDATE. */
void COBCICS_FILE_READ(const char *name, unsigned char *destination,
                       unsigned char *length, const unsigned char *capacity,
                       unsigned char *ridfld, const unsigned char *key_length,
                       const unsigned char *key_capacity,
                       const unsigned char *mode,
                       unsigned char *resp, unsigned char *resp2) {
    const struct file_def *file = file_for(name);
    uint32_t key_size = number(key_length);
    uint32_t available = number(length);
    uint32_t storage = number(capacity);
    if (!file || !key_size || key_size > file->key_size ||
        key_size > number(key_capacity) || *mode > 7 ||
        ((*mode & 2) && !(*mode & 1)) || ((*mode & 4) && (*mode & 1))) {
        response(resp, resp2, 16, 1);
        return;
    }
    if (!(*mode & 1) && key_size != file->key_size) {
        response(resp, resp2, 16, 2);
        return;
    }
    if (available > storage)
        available = storage;
    const char *path = local_path(file);
    if (path) {
        unsigned char *host_key = malloc(key_size);
        unsigned char *host_record = malloc(file->record_size);
        if (!host_key || !host_record) {
            free(host_key); free(host_record);
            response(resp, resp2, 17, 5); return;
        }
        copy_to_host(host_key, ridfld, key_size);
        int result = (*mode & 4 ? cobcics_vsam_read_update : cobcics_vsam_read)(
            path, file->record_size, file->key_size, file->key_offset,
            host_key, key_size, *mode & 1, host_record);
        free(host_key);
        if (result) { free(host_record); response(resp, resp2, result, 0); return; }
        put_number(length, file->record_size);
        if (available < file->record_size) {
            free(host_record); response(resp, resp2, 22, 1); return;
        }
        copy_from_host(destination, host_record, file->record_size);
        if (number(key_capacity) >= file->key_size)
            copy_from_host(ridfld, host_record + file->key_offset, file->key_size);
        if ((*mode & 4) && !hold_update(file, host_record + file->key_offset)) {
            free(host_record); response(resp, resp2, 17, 5); return;
        }
        free(host_record);
        response(resp, resp2, 0, 0);
        return;
    }
    SQLHDBC connection = DB2SQL_CONNECTION();
    if (!connection) {
        response(resp, resp2, 17, 2);
        return;
    }
    char sql[160];
    snprintf(sql, sizeof sql, "SELECT K, V FROM %s WHERE K %s ? %s",
             file->table, (*mode & 1) ? ">=" : "=",
             (*mode & 1) ? "ORDER BY K FETCH FIRST 1 ROW ONLY" : "");
    SQLHSTMT statement = SQL_NULL_HSTMT;
    if (!prepare(connection, sql, &statement, resp, resp2))
        return;
    unsigned char host_key[21];
    copy_to_host(host_key, ridfld, key_size);
    SQLLEN key_bytes = key_size;
    SQLRETURN result;
    if (!bind_bytes(statement, 1, host_key, &key_bytes, SQL_VARBINARY,
                    file->key_size) || !success(SQLExecute(statement))) {
        sql_error(statement, "READ", resp, resp2);
        goto finish;
    }
    result = SQLFetch(statement);
    if (result == SQL_NO_DATA) {
        response(resp, resp2, 13, 0);
        goto finish;
    }
    if (!success(result)) {
        sql_error(statement, "READ fetch", resp, resp2);
        goto finish;
    }
    unsigned char found_key[21];
    unsigned char found_record[225];
    SQLLEN found_key_length = 0, found_record_length = 0;
    result = SQLGetData(statement, 1, SQL_C_BINARY, found_key,
                        sizeof found_key, &found_key_length);
    if (!success(result) || found_key_length != (SQLLEN)file->key_size) {
        response(resp, resp2, 17, 3);
        goto finish;
    }
    result = SQLGetData(statement, 2, SQL_C_BINARY, found_record,
                        sizeof found_record, &found_record_length);
    if (!success(result) || found_record_length != (SQLLEN)file->record_size) {
        response(resp, resp2, 17, 4);
        goto finish;
    }
    put_number(length, file->record_size);
    if (available < file->record_size) {
        response(resp, resp2, 22, 1);
        goto finish;
    }
    copy_from_host(destination, found_record, file->record_size);
    if (number(key_capacity) >= file->key_size)
        copy_from_host(ridfld, found_key, file->key_size);
    if (*mode & 4) {
        SQLFreeHandle(SQL_HANDLE_STMT, statement);
        statement = SQL_NULL_HSTMT;
        snprintf(sql, sizeof sql, "UPDATE %s SET V=V WHERE K=?", file->table);
        if (!prepare(connection, sql, &statement, resp, resp2))
            return;
        key_bytes = file->key_size;
        if (!bind_bytes(statement, 1, found_key, &key_bytes, SQL_VARBINARY,
                        file->key_size) || !success(SQLExecute(statement))) {
            sql_error(statement, "READ UPDATE lock", resp, resp2);
            goto finish;
        }
        if (!hold_update(file, found_key)) {
            response(resp, resp2, 17, 5);
            goto finish;
        }
    }
    response(resp, resp2, 0, 0);
finish:
    SQLFreeHandle(SQL_HANDLE_STMT, statement);
}

void COBCICS_FILE_WRITE(const char *name, const unsigned char *source,
                        const unsigned char *length,
                        const unsigned char *capacity,
                        const unsigned char *ridfld,
                        const unsigned char *key_length,
                        const unsigned char *key_capacity,
                        unsigned char *resp, unsigned char *resp2) {
    const struct file_def *file = file_for(name);
    (void)capacity; /* FROM may begin at an elementary item of a group. */
    if (!file || number(length) != file->record_size ||
        number(key_length) != file->key_size ||
        number(key_capacity) < file->key_size ||
        memcmp(source + file->key_offset, ridfld, file->key_size) != 0) {
        response(resp, resp2, 16, 1);
        return;
    }
    const char *path = local_path(file);
    if (path) {
        unsigned char *host_record = malloc(file->record_size);
        if (!host_record) { response(resp, resp2, 17, 5); return; }
        copy_to_host(host_record, source, file->record_size);
        int result = cobcics_vsam_write(path, file->record_size,
                                       file->key_size, file->key_offset, host_record);
        free(host_record);
        response(resp, resp2, result, 0);
        return;
    }
    SQLHDBC connection = DB2SQL_CONNECTION();
    if (!connection) {
        response(resp, resp2, 17, 2);
        return;
    }
    char sql[120];
    snprintf(sql, sizeof sql, "INSERT INTO %s (K,V) VALUES (?,?)", file->table);
    SQLHSTMT statement = SQL_NULL_HSTMT;
    if (!prepare(connection, sql, &statement, resp, resp2))
        return;
    unsigned char host_key[21], host_record[225];
    copy_to_host(host_key, ridfld, file->key_size);
    copy_to_host(host_record, source, file->record_size);
    SQLLEN key_bytes = file->key_size, record_bytes = file->record_size;
    if (!bind_bytes(statement, 1, host_key, &key_bytes, SQL_VARBINARY,
                    file->key_size) ||
        !bind_bytes(statement, 2, host_record, &record_bytes, SQL_BLOB,
                    file->record_size) || !success(SQLExecute(statement)))
        sql_error(statement, "WRITE", resp, resp2);
    else
        response(resp, resp2, 0, 0);
    SQLFreeHandle(SQL_HANDLE_STMT, statement);
}

void COBCICS_FILE_REWRITE(const char *name, const unsigned char *source,
                          const unsigned char *length,
                          const unsigned char *capacity,
                          unsigned char *resp, unsigned char *resp2) {
    const struct file_def *file = file_for(name);
    (void)capacity;
    if (!file || !held_update.active ||
        memcmp(held_update.name, file->name, 8) ||
        held_update.key_size != file->key_size ||
        number(length) != file->record_size) {
        response(resp, resp2, 16, 1);
        return;
    }
    const char *path = local_path(file);
    if (path) {
        unsigned char *host_record = malloc(file->record_size);
        if (!host_record) { response(resp, resp2, 17, 5); return; }
        copy_to_host(host_record, source, file->record_size);
        if (memcmp(host_record + file->key_offset, held_update.key, file->key_size)) {
            free(host_record); response(resp, resp2, 16, 1); return;
        }
        int result = cobcics_vsam_rewrite(path, file->record_size,
                     file->key_size, file->key_offset, held_update.key, host_record);
        free(host_record);
        if (!result) held_update.active = 0;
        response(resp, resp2, result, 0);
        return;
    }
    unsigned char host_record[225];
    copy_to_host(host_record, source, file->record_size);
    if (memcmp(host_record, held_update.key, file->key_size)) {
        response(resp, resp2, 16, 1); return;
    }
    SQLHDBC connection = DB2SQL_CONNECTION();
    if (!connection) {
        response(resp, resp2, 17, 2);
        return;
    }
    char sql[120];
    snprintf(sql, sizeof sql, "UPDATE %s SET V=? WHERE K=?", file->table);
    SQLHSTMT statement = SQL_NULL_HSTMT;
    if (!prepare(connection, sql, &statement, resp, resp2))
        return;
    SQLLEN record_bytes = file->record_size, key_bytes = file->key_size;
    if (!bind_bytes(statement, 1, host_record, &record_bytes, SQL_BLOB,
                    file->record_size) ||
        !bind_bytes(statement, 2, held_update.key, &key_bytes,
                    SQL_VARBINARY, file->key_size) ||
        !success(SQLExecute(statement)))
        sql_error(statement, "REWRITE", resp, resp2);
    else {
        held_update.active = 0;
        response(resp, resp2, 0, 0);
    }
    SQLFreeHandle(SQL_HANDLE_STMT, statement);
}

void COBCICS_FILE_DELETE(const char *name, const unsigned char *ridfld,
                         const unsigned char *key_length,
                         const unsigned char *key_capacity,
                         unsigned char *resp, unsigned char *resp2) {
    const struct file_def *file = file_for(name);
    if (!file || number(key_length) != file->key_size ||
        number(key_capacity) < file->key_size) {
        response(resp, resp2, 16, 1);
        return;
    }
    const char *path = local_path(file);
    if (path) {
        unsigned char *host_key = malloc(file->key_size);
        if (!host_key) { response(resp, resp2, 17, 5); return; }
        copy_to_host(host_key, ridfld, file->key_size);
        int result = cobcics_vsam_delete(path, file->record_size,
                                        file->key_size, file->key_offset, host_key);
        free(host_key);
        response(resp, resp2, result, 0);
        return;
    }
    SQLHDBC connection = DB2SQL_CONNECTION();
    if (!connection) {
        response(resp, resp2, 17, 2);
        return;
    }
    char sql[120];
    snprintf(sql, sizeof sql, "DELETE FROM %s WHERE K=?", file->table);
    SQLHSTMT statement = SQL_NULL_HSTMT;
    if (!prepare(connection, sql, &statement, resp, resp2))
        return;
    unsigned char host_key[21];
    copy_to_host(host_key, ridfld, file->key_size);
    SQLLEN key_bytes = file->key_size;
    if (!bind_bytes(statement, 1, host_key, &key_bytes, SQL_VARBINARY,
                    file->key_size) || !success(SQLExecute(statement))) {
        sql_error(statement, "DELETE", resp, resp2);
    } else {
        SQLLEN changed = 0;
        if (!success(SQLRowCount(statement, &changed)))
            sql_error(statement, "DELETE count", resp, resp2);
        else
            response(resp, resp2, changed ? 0 : 13, 0);
    }
    SQLFreeHandle(SQL_HANDLE_STMT, statement);
}
