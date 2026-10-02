#!/usr/bin/env python3
"""Load GenApp's KSDS sample records into isolated Db2 binary tables."""

import os
from pathlib import Path

import ibm_db


ROOT = Path(__file__).resolve().parents[2]
SCHEMA = "GENAPPTEST"


def connection_string() -> str:
    required = ("DB2_HOST", "DB2_PORT", "DB2_DATABASE", "DB2_USER", "DB2_PASSWORD")
    missing = [name for name in required if not os.environ.get(name)]
    if missing:
        raise RuntimeError("missing Db2 settings: " + ", ".join(missing))
    return (f"DATABASE={os.environ['DB2_DATABASE']};"
            f"HOSTNAME={os.environ['DB2_HOST']};PORT={os.environ['DB2_PORT']};"
            f"PROTOCOL=TCPIP;UID={os.environ['DB2_USER']};"
            f"PWD={os.environ['DB2_PASSWORD']};")


def count(connection, sql: str) -> int:
    result = ibm_db.exec_immediate(connection, sql)
    return ibm_db.fetch_tuple(result)[0]


def main() -> None:
    connection = ibm_db.connect(connection_string(), "", "")
    ibm_db.autocommit(connection, ibm_db.SQL_AUTOCOMMIT_OFF)
    try:
        if not count(connection, "SELECT COUNT(*) FROM SYSCAT.SCHEMATA "
                     "WHERE SCHEMANAME='GENAPPTEST'"):
            ibm_db.exec_immediate(connection, f"CREATE SCHEMA {SCHEMA}")
        for name, key_size, record_size in (("KSDSCUST", 10, 225),
                                            ("KSDSPOLY", 21, 64)):
            if not count(connection, "SELECT COUNT(*) FROM SYSCAT.TABLES "
                         f"WHERE TABSCHEMA='{SCHEMA}' AND TABNAME='{name}'"):
                ibm_db.exec_immediate(connection,
                    f"CREATE TABLE {SCHEMA}.{name} ("
                    f"K BINARY({key_size}) NOT NULL PRIMARY KEY, "
                    f"V BLOB({record_size}) NOT NULL)")
            ibm_db.exec_immediate(connection, f"DELETE FROM {SCHEMA}.{name}")
            records = (ROOT / "reference/cics-genapp/base/data" /
                       f"{name.lower()}.txt").read_bytes().splitlines()
            assert len(records) == 10 and all(len(row) == record_size for row in records)
            for row in records:
                key = row[:key_size]
                ibm_db.exec_immediate(connection,
                    f"INSERT INTO {SCHEMA}.{name} (K,V) VALUES "
                    f"(BX'{key.hex()}', BX'{row.hex()}')")
            assert count(connection, f"SELECT COUNT(*) FROM {SCHEMA}.{name}") == 10
        ibm_db.commit(connection)
        print("GENAPPTEST KSDSCUST/KSDSPOLY sample records ready (10 each)")
    except Exception:
        ibm_db.rollback(connection)
        raise
    finally:
        ibm_db.close(connection)


if __name__ == "__main__":
    main()
