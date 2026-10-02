/* GenApp CICS KSDS operations backed by the embedded-SQL Db2 connection.
   Record and key columns are binary, so no CCSID conversion changes bytes. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <libcob.h>
#include <sqlcli.h>
#include <sqlcli1.h>

extern SQLHDBC DB2SQL_CONNECTION(void);

struct file_def {
    char name[8];
    const char *table;
    uint32_t key_size;
    uint32_t record_size;
};

static const struct file_def files[] = {
    {{'K','S','D','S','C','U','S','T'}, "KSDSCUST", 10, 225},
    {{'K','S','D','S','P','O','L','Y'}, "KSDSPOLY", 21, 64},
};

static struct {
    const struct file_def *file;
    unsigned char key[21];
} held_update;

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

static const struct file_def *file_for(const char *name) {
    for (size_t index = 0; index < sizeof files / sizeof files[0]; ++index)
        if (memcmp(name, files[index].name, 8) == 0)
            return &files[index];
    if (cob_is_initialized() && (unsigned char)name[0] >= 0x80) {
        char host_name[9];
        if (cob_get_param_str(1, host_name, sizeof host_name))
            for (size_t index = 0; index < sizeof files / sizeof files[0]; ++index)
                if (memcmp(host_name, files[index].name, 8) == 0)
                    return &files[index];
    }
    return NULL;
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
        held_update.file = file;
        memcpy(held_update.key, found_key, file->key_size);
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
        memcmp(source, ridfld, file->key_size) != 0) {
        response(resp, resp2, 16, 1);
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
    unsigned char host_record[225];
    if (file)
        copy_to_host(host_record, source, file->record_size);
    if (!file || held_update.file != file ||
        number(length) != file->record_size ||
        memcmp(host_record, held_update.key, file->key_size) != 0) {
        response(resp, resp2, 16, 1);
        return;
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
        held_update.file = NULL;
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
