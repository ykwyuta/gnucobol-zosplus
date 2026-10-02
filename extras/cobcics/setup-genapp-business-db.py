#!/usr/bin/env python3
"""Load GenApp's original Db2 JCL tables and rows into GENAPPTEST."""

import re
from pathlib import Path

import ibm_db

from setup_genapp_connection import connection_string


ROOT = Path(__file__).resolve().parents[2]
SCHEMA = "GENAPPTEST"
JCL = ROOT / "reference/cics-genapp/base/cntl/db2cre.jcl"
TABLES = ("CUSTOMER", "CUSTOMER_SECURE", "POLICY", "ENDOWMENT",
          "HOUSE", "MOTOR", "COMMERCIAL", "CLAIM")


def statements() -> tuple[list[str], list[str]]:
    text = JCL.read_text(encoding="ascii")
    creates = re.findall(r"(?ims)^CREATE TABLE\s+<DB2DBID>\..*?;", text)
    inserts = re.findall(r"(?ims)^\s*Insert Into\s+<DB2DBID>\..*?;", text)
    assert len(creates) == len(TABLES) and len(inserts) == 40
    converted = []
    for sql in creates:
        sql = sql.replace("<DB2DBID>", SCHEMA)
        sql = re.sub(r"\)\s+CCSID EBCDIC\s+IN\s+GENAPPTEST\.GENATS\d+\s*;\s*$",
                     ");", sql, flags=re.I)
        sql = sql.replace("VARCHAR(32606)", "CLOB(32606)")
        if "CREATE TABLE GENAPPTEST.Commercial" in sql:
            # The supplied rows use 'ACTIVE', though the supplied DDL says SMALLINT.
            sql = re.sub(r"(Status\s+)SmallINT", r"\1VARCHAR(16)", sql,
                         flags=re.I)
        assert "CCSID" not in sql and sql.rstrip().endswith(");"), sql
        converted.append(sql.rstrip().removesuffix(";"))
    return converted, [sql.replace("<DB2DBID>", SCHEMA).strip().removesuffix(";")
                       for sql in inserts]


def query_int(connection, sql: str) -> int:
    return ibm_db.fetch_tuple(ibm_db.exec_immediate(connection, sql))[0]


def main() -> None:
    creates, inserts = statements()
    connection = ibm_db.connect(connection_string(), "", "")
    ibm_db.autocommit(connection, ibm_db.SQL_AUTOCOMMIT_OFF)
    try:
        if not query_int(connection, "SELECT COUNT(*) FROM SYSCAT.SCHEMATA "
                         "WHERE SCHEMANAME='GENAPPTEST'"):
            ibm_db.exec_immediate(connection, f"CREATE SCHEMA {SCHEMA}")
        for name, sql in zip(TABLES, creates):
            if not query_int(connection, "SELECT COUNT(*) FROM SYSCAT.TABLES "
                             f"WHERE TABSCHEMA='{SCHEMA}' AND TABNAME='{name}'"):
                ibm_db.exec_immediate(connection, sql)
        for name in reversed(TABLES):
            ibm_db.exec_immediate(connection, f"DELETE FROM {SCHEMA}.{name}")
        for sql in inserts:
            ibm_db.exec_immediate(connection, sql)
        expected = {"CUSTOMER": 10, "CUSTOMER_SECURE": 10, "POLICY": 10,
                    "ENDOWMENT": 2, "HOUSE": 3, "MOTOR": 3,
                    "COMMERCIAL": 2, "CLAIM": 0}
        actual = {name: query_int(connection,
                  f"SELECT COUNT(*) FROM {SCHEMA}.{name}") for name in TABLES}
        assert actual == expected, actual
        ibm_db.commit(connection)
        print(f"GENAPPTEST business JCL rows ready: {actual}")
    except Exception:
        ibm_db.rollback(connection)
        raise
    finally:
        ibm_db.close(connection)


if __name__ == "__main__":
    main()
