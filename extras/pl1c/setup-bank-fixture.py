#!/usr/bin/env python3
"""Create a dedicated Db2 fixture for the unmodified BNKSTMT PL/I program."""

import os
from pathlib import Path

import ibm_db


SCHEMA = "PL1TEST"


def query_one(connection, sql):
    row = ibm_db.fetch_tuple(ibm_db.exec_immediate(connection, sql))
    return row[0] if row else None


def main():
    required = ("DB2_HOST", "DB2_PORT", "DB2_DATABASE", "DB2_USER", "DB2_PASSWORD")
    missing = [name for name in required if not os.environ.get(name)]
    if missing:
        raise RuntimeError("missing Db2 settings: " + ", ".join(missing))
    dsn = (
        f"DATABASE={os.environ['DB2_DATABASE']};"
        f"HOSTNAME={os.environ['DB2_HOST']};PORT={os.environ['DB2_PORT']};"
        f"PROTOCOL=TCPIP;UID={os.environ['DB2_USER']};"
        f"PWD={os.environ['DB2_PASSWORD']};"
    )
    connection = ibm_db.connect(dsn, "", "")
    ibm_db.autocommit(connection, ibm_db.SQL_AUTOCOMMIT_OFF)
    try:
        if not query_one(
            connection,
            "SELECT COUNT(*) FROM SYSCAT.SCHEMATA WHERE SCHEMANAME='PL1TEST'",
        ):
            ibm_db.exec_immediate(connection, f"CREATE SCHEMA {SCHEMA}")

        source = (
            Path(__file__).resolve().parent.parent / "cobcics" / "bank-test-db.sql"
        ).read_text()
        for part in source.replace("BANKZTEST.", f"{SCHEMA}.").split(";"):
            sql = "\n".join(
                line for line in part.splitlines() if not line.lstrip().startswith("--")
            ).strip()
            if not sql:
                continue
            words = sql.split()
            if words[:2] == ["CREATE", "TABLE"]:
                table = words[2].split(".")[1]
                if query_one(
                    connection,
                    "SELECT COUNT(*) FROM SYSCAT.TABLES "
                    f"WHERE TABSCHEMA='{SCHEMA}' AND TABNAME='{table}'",
                ):
                    continue
            elif words[:2] == ["CREATE", "INDEX"] or words[:3] == [
                "CREATE", "UNIQUE", "INDEX"
            ]:
                index = words[words.index("INDEX") + 1].split(".")[1]
                if query_one(
                    connection,
                    "SELECT COUNT(*) FROM SYSCAT.INDEXES "
                    f"WHERE INDSCHEMA='{SCHEMA}' AND INDNAME='{index}'",
                ):
                    continue
            ibm_db.exec_immediate(connection, sql)

        rows = (
            (
                "CUSTOMER",
                "CUSTOMER_SORTCODE='123456' AND CUSTOMER_NUMBER='0000000001'",
                "INSERT INTO PL1TEST.CUSTOMER VALUES "
                "('CUST','123456','0000000001','Ms','Aiko','Tanaka',19900102,"
                "'0123456789','1 Main Street','','Tokyo','100-0001',"
                "'Japan','ACTIVE',20240101,750,20270101)",
            ),
            (
                "ACCOUNT",
                "ACCOUNT_SORTCODE='123456' AND ACCOUNT_NUMBER='00000001'",
                "INSERT INTO PL1TEST.ACCOUNT VALUES "
                "('ACCT','0000000001','123456','00000001','CURRENT',1.50,"
                "DATE('2024-01-01'),100,DATE('2026-05-31'),DATE('2026-06-30'),"
                "1100.00,1100.00)",
            ),
            (
                "ACCOUNT",
                "ACCOUNT_SORTCODE='123456' AND ACCOUNT_NUMBER='00000002'",
                "INSERT INTO PL1TEST.ACCOUNT VALUES "
                "('ACCT','0000000001','123456','00000002','SAVINGS',2.00,"
                "DATE('2024-02-01'),0,DATE('2026-05-31'),DATE('2026-06-30'),"
                "500.00,500.00)",
            ),
            (
                "PROCTRAN",
                "PROCTRAN_SORTCODE='123456' AND PROCTRAN_NUMBER='00000001' "
                "AND PROCTRAN_REF='PL1CR0000001'",
                "INSERT INTO PL1TEST.PROCTRAN VALUES "
                "('TRAN','123456','00000001',DATE('2026-06-05'),'091530',"
                "'PL1CR0000001','CR','Salary',150.00)",
            ),
            (
                "PROCTRAN",
                "PROCTRAN_SORTCODE='123456' AND PROCTRAN_NUMBER='00000001' "
                "AND PROCTRAN_REF='PL1DB0000002'",
                "INSERT INTO PL1TEST.PROCTRAN VALUES "
                "('TRAN','123456','00000001',DATE('2026-06-10'),'143000',"
                "'PL1DB0000002','DB','Groceries',-50.00)",
            ),
            (
                "PROCTRAN",
                "PROCTRAN_SORTCODE='123456' AND PROCTRAN_NUMBER='00000001' "
                "AND PROCTRAN_REF='PL1OLD000003'",
                "INSERT INTO PL1TEST.PROCTRAN VALUES "
                "('TRAN','123456','00000001',DATE('2026-05-31'),'120000',"
                "'PL1OLD000003','CR','Prior month',25.00)",
            ),
        )
        for table, predicate, insert in rows:
            if not query_one(
                connection, f"SELECT COUNT(*) FROM {SCHEMA}.{table} WHERE {predicate}"
            ):
                ibm_db.exec_immediate(connection, insert)
        ibm_db.commit(connection)
        print("PL1TEST ACCOUNT, CUSTOMER and PROCTRAN fixture ready")
    except Exception:
        ibm_db.rollback(connection)
        raise
    finally:
        ibm_db.close(connection)


if __name__ == "__main__":
    main()
