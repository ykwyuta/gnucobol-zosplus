#!/usr/bin/env python3
"""Db2 CLI worker for jclsh IDCAMS SQL-backed KSDS operations."""
import os
from pathlib import Path
import sys

import ibm_db


def connect():
    names = ("DB2_DATABASE", "DB2_HOST", "DB2_PORT", "DB2_USER", "DB2_PASSWORD")
    for name in names:
        if not os.environ.get(name):
            raise ValueError(f"missing {name}")
    connection = ibm_db.connect(
        f"DATABASE={os.environ['DB2_DATABASE']};HOSTNAME={os.environ['DB2_HOST']};"
        f"PORT={os.environ['DB2_PORT']};PROTOCOL=TCPIP;UID={os.environ['DB2_USER']};"
        f"PWD={os.environ['DB2_PASSWORD']};", "", "")
    ibm_db.autocommit(connection, ibm_db.SQL_AUTOCOMMIT_OFF)
    return connection


def main():
    verb, table, *arguments = sys.argv[1:]
    if not table or len(table) >= 64 or any(
        not part or not (part[0].isalpha() or part[0] == "_") or
        any(not (character.isascii() and (character.isalnum() or character == "_"))
            for character in part)
        for part in table.split(".")
    ) or len(table.split(".")) > 2:
        raise ValueError("invalid table name")
    connection = connect()
    try:
        if verb == "create":
            lrecl, keylen, offset = map(int, arguments)
            if lrecl <= 0 or keylen <= 0 or offset < 0 or offset + keylen > lrecl:
                raise ValueError("invalid keyed FB layout")
            ibm_db.exec_immediate(connection,
                f"CREATE TABLE {table} (K BINARY({keylen}) NOT NULL PRIMARY KEY, "
                f"V BLOB({lrecl}) NOT NULL)")
        elif verb == "drop":
            ibm_db.exec_immediate(connection, f"DROP TABLE {table}")
        elif verb == "replace":
            lrecl, keylen, offset = map(int, arguments[:3])
            data = Path(arguments[3]).read_bytes()
            if lrecl <= 0 or keylen <= 0 or offset < 0 or offset + keylen > lrecl \
                    or len(data) % lrecl:
                raise ValueError("invalid keyed FB input")
            ibm_db.exec_immediate(connection, f"DELETE FROM {table}")
            statement = ibm_db.prepare(connection,
                f"INSERT INTO {table} (K,V) VALUES (?,?)")
            for start in range(0, len(data), lrecl):
                record = data[start:start + lrecl]
                ibm_db.execute(statement, (record[offset:offset + keylen], record))
        elif verb == "export":
            statement = ibm_db.exec_immediate(connection, f"SELECT K,V FROM {table} ORDER BY K")
            while True:
                row = ibm_db.fetch_tuple(statement)
                if row is False:
                    break
                key, record = row
                if isinstance(key, memoryview):
                    key = key.tobytes()
                if isinstance(record, memoryview):
                    record = record.tobytes()
                print(f"{key.hex()}|{record.hex()}")
        else:
            raise ValueError("invalid VSAM SQL operation")
        ibm_db.commit(connection)
    except Exception:
        ibm_db.rollback(connection)
        raise
    finally:
        ibm_db.close(connection)


if __name__ == "__main__":
    main()
