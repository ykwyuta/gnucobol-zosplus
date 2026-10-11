/* Integration test: CICS UOW helpers, Db2 CLI, and local FB file. */
#define COBCICS_DB2SQL
#define COBCICS_LOCAL_VSAM
#include "bridge.c"
#include "local_vsam.h"
#include <sqlcli.h>
#include <sqlcli1.h>
#include <stdio.h>
#include <string.h>

extern SQLHDBC DB2SQL_CONNECTION(void);
static int suppress_error;

static int sql(const char *command, int *count) {
    SQLHDBC connection = DB2SQL_CONNECTION();
    SQLHSTMT statement = SQL_NULL_HSTMT;
    if (!connection || SQLAllocHandle(SQL_HANDLE_STMT, connection, &statement) != SQL_SUCCESS)
        return 1;
    SQLRETURN result = SQLExecDirect(statement, (SQLCHAR *)command, SQL_NTS);
    if ((result == SQL_SUCCESS || result == SQL_SUCCESS_WITH_INFO) && count) {
        SQLINTEGER value = -1;
        result = SQLFetch(statement);
        if (result == SQL_SUCCESS || result == SQL_SUCCESS_WITH_INFO)
            result = SQLGetData(statement, 1, SQL_C_SLONG, &value, sizeof value, NULL);
        if (result == SQL_SUCCESS || result == SQL_SUCCESS_WITH_INFO) *count = value;
    }
    if (result != SQL_SUCCESS && result != SQL_SUCCESS_WITH_INFO && !suppress_error) {
        SQLCHAR state[6] = {0}, message[256] = {0};
        SQLINTEGER native = 0;
        SQLSMALLINT length = 0;
        SQLGetDiagRec(SQL_HANDLE_STMT, statement, 1, state, &native,
                      message, sizeof message, &length);
        fprintf(stderr, "SQL %s: %s (%s)\n", command, message, state);
    }
    SQLFreeHandle(SQL_HANDLE_STMT, statement);
    return result != SQL_SUCCESS && result != SQL_SUCCESS_WITH_INFO;
}

static long disk_size(const char *path) {
    FILE *file = fopen(path, "rb");
    if (!file) return -1;
    if (fseek(file, 0, SEEK_END)) { fclose(file); return -1; }
    long size = ftell(file);
    fclose(file);
    return size;
}

int main(int argc, char **argv) {
    unsigned char record[225];
    int count = -1, failed = 0;
    if (argc != 2 || disk_size(argv[1]) != 225) return 2;
    cob_init(argc, argv);
    memset(record, ' ', sizeof record);
    memcpy(record, "CUST000011", 10);
    /* This dedicated table is removed at the end; a prior interrupted test
       may have left it behind. */
    suppress_error = 1;
    sql("DROP TABLE UOWVSAM_TEST", NULL);
    suppress_error = 0;
    DB2SQL_END_TASK(1);
    if (sql("CREATE TABLE UOWVSAM_TEST (ID INTEGER NOT NULL PRIMARY KEY)", NULL) ||
        DB2SQL_END_TASK(1)) return 1;
    if (cobcics_vsam_begin() || sql("INSERT INTO UOWVSAM_TEST VALUES (1)", NULL) ||
        cobcics_vsam_write(argv[1], 225, 10, 0, record) || rollback_uow()) failed = 1;
    if (sql("SELECT COUNT(*) FROM UOWVSAM_TEST", &count) || count != 0 ||
        disk_size(argv[1]) != 225 || DB2SQL_END_TASK(0)) failed = 1;
    if (cobcics_vsam_begin() || sql("INSERT INTO UOWVSAM_TEST VALUES (2)", NULL) ||
        cobcics_vsam_write(argv[1], 225, 10, 0, record) || commit_uow()) failed = 1;
    count = -1;
    if (sql("SELECT COUNT(*) FROM UOWVSAM_TEST", &count) || count != 1 ||
        disk_size(argv[1]) != 450 || DB2SQL_END_TASK(0)) failed = 1;
    if (sql("DROP TABLE UOWVSAM_TEST", NULL) || DB2SQL_END_TASK(1)) failed = 1;
    if (!failed) puts("CICS local VSAM and real Db2 commit/rollback passed");
    return failed;
}
