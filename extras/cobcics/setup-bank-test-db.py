#!/usr/bin/env python3
"""Create isolated Db2 Bank of Z tables and deterministic screen-test rows."""

import os
from pathlib import Path

import ibm_db


def query_one(connection, statement):
    result = ibm_db.exec_immediate(connection, statement)
    row = ibm_db.fetch_tuple(result)
    return row[0] if row else None


def main():
    required = ("DB2_HOST", "DB2_PORT", "DB2_DATABASE", "DB2_USER", "DB2_PASSWORD")
    missing = [name for name in required if not os.environ.get(name)]
    if missing:
        raise RuntimeError("missing Db2 settings: " + ", ".join(missing))
    dsn = (f"DATABASE={os.environ['DB2_DATABASE']};"
           f"HOSTNAME={os.environ['DB2_HOST']};PORT={os.environ['DB2_PORT']};"
           f"PROTOCOL=TCPIP;UID={os.environ['DB2_USER']};"
           f"PWD={os.environ['DB2_PASSWORD']};")
    connection = ibm_db.connect(dsn, "", "")
    ibm_db.autocommit(connection, ibm_db.SQL_AUTOCOMMIT_OFF)
    try:
        if not query_one(connection, "SELECT COUNT(*) FROM SYSCAT.SCHEMATA WHERE SCHEMANAME='BANKZTEST'"):
            ibm_db.exec_immediate(connection, "CREATE SCHEMA BANKZTEST")
        source = Path(__file__).with_name("bank-test-db.sql").read_text()
        statements = [part.strip() for part in source.split(";") if part.strip()]
        for statement in statements:
            statement = "\n".join(line for line in statement.splitlines()
                                  if not line.lstrip().startswith("--")).strip()
            if not statement:
                continue
            if statement.startswith("CREATE TABLE"):
                table = statement.split()[2].split(".")[1]
                exists = query_one(connection, "SELECT COUNT(*) FROM SYSCAT.TABLES "
                                   f"WHERE TABSCHEMA='BANKZTEST' AND TABNAME='{table}'")
                if exists:
                    continue
            elif statement.startswith("CREATE") and "INDEX" in statement:
                words = statement.split()
                index = words[words.index("INDEX") + 1].split(".")[1]
                exists = query_one(connection, "SELECT COUNT(*) FROM SYSCAT.INDEXES "
                                   f"WHERE INDSCHEMA='BANKZTEST' AND INDNAME='{index}'")
                if exists:
                    continue
            ibm_db.exec_immediate(connection, statement)
        if not query_one(connection, "SELECT COUNT(*) FROM BANKZTEST.CUSTOMER "
                         "WHERE CUSTOMER_SORTCODE='987654' AND CUSTOMER_NUMBER='0000000001'"):
            ibm_db.exec_immediate(connection, """INSERT INTO BANKZTEST.CUSTOMER VALUES
                ('CUST','987654','0000000001','Mr','Test','Customer',19800102,
                 '0123456789','1 Test Street','','London','SW1 1AA',
                 'United Kingdom','ACTIVE',20240101,700,20270101)""")
        for number, balance in (("00000001", "1000.00"),
                                ("00000002", "500.00")):
            if not query_one(connection, "SELECT COUNT(*) FROM BANKZTEST.ACCOUNT "
                             f"WHERE ACCOUNT_SORTCODE='987654' AND ACCOUNT_NUMBER='{number}'"):
                ibm_db.exec_immediate(connection, f"""INSERT INTO BANKZTEST.ACCOUNT VALUES
                    ('ACCT','0000000001','987654','{number}','CURRENT',1.50,
                     DATE('2024-01-01'),100,DATE('2024-07-01'),DATE('2024-08-01'),
                     {balance},{balance})""")
        for name, value in (("BANKZCUST987654", 1),
                            ("987654-ACCOUNT-LAST", 2),
                            ("987654-ACCOUNT-COUNT", 2)):
            if not query_one(connection, "SELECT COUNT(*) FROM BANKZTEST.CONTROL "
                             f"WHERE CONTROL_NAME='{name}'"):
                ibm_db.exec_immediate(connection, "INSERT INTO BANKZTEST.CONTROL VALUES "
                                      f"('{name}',{value},'')")
        ibm_db.commit(connection)
        print("BANKZTEST tables and deterministic rows ready")
    except Exception:
        ibm_db.rollback(connection)
        raise
    finally:
        ibm_db.close(connection)


if __name__ == "__main__":
    main()
