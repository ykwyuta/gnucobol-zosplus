#!/usr/bin/env python3
"""Verify local SYNCONRETURN commits only at task end, including ABEND backout."""

import argparse
import os
import subprocess
import uuid

import ibm_db


def request(aid, identifier):
    return (b"CCI3" + bytes([aid]) + b"OMENBANKAPP " + b"\0\x08"
            + identifier.encode("ascii") + b"\0\0")


def rows(connection, identifiers):
    values = ",".join(f"'{identifier}'" for identifier in identifiers)
    statement = ibm_db.exec_immediate(
        connection,
        f"SELECT ID FROM DB2SQL_DEMO WHERE ID IN ({values}) ORDER BY ID",
    )
    result = []
    while row := ibm_db.fetch_tuple(statement):
        result.append(row[0].strip())
    return result


def clean(connection, identifiers):
    values = ",".join(f"'{identifier}'" for identifier in identifiers)
    ibm_db.exec_immediate(
        connection, f"DELETE FROM DB2SQL_DEMO WHERE ID IN ({values})"
    )
    ibm_db.commit(connection)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("worker")
    args = parser.parse_args()
    dsn = ";".join(
        f"{key}={os.environ[environment]}"
        for key, environment in (
            ("DATABASE", "DB2_DATABASE"),
            ("HOSTNAME", "DB2_HOST"),
            ("PORT", "DB2_PORT"),
            ("UID", "DB2_USER"),
            ("PWD", "DB2_PASSWORD"),
        )
    ) + ";PROTOCOL=TCPIP;"
    connection = ibm_db.connect(dsn, "", "")
    identifiers = tuple("S" + uuid.uuid4().hex[:7].upper() for _ in range(4))
    try:
        assert len(set(identifiers)) == 4
        assert rows(connection, identifiers) == []
        committed = subprocess.run(
            [args.worker], input=request(0, identifiers[0]),
            capture_output=True, check=False
        )
        assert committed.returncode == 0, committed.stderr.decode(errors="replace")
        assert committed.stdout.startswith(b"CCO5"), committed.stdout
        assert rows(connection, identifiers) == [identifiers[0]]

        backed_out = subprocess.run(
            [args.worker], input=request(0xF3, identifiers[1]),
            capture_output=True, check=False
        )
        assert backed_out.returncode == 12, backed_out.stderr.decode(errors="replace")
        assert rows(connection, identifiers) == [identifiers[0]]

        rolled_back = subprocess.run(
            [args.worker], input=request(0x7D, identifiers[2]),
            capture_output=True, check=False
        )
        assert rolled_back.returncode == 0, rolled_back.stderr.decode(errors="replace")
        assert rows(connection, identifiers) == [identifiers[0]]

        committed_before_abend = subprocess.run(
            [args.worker], input=request(0xF1, identifiers[3]),
            capture_output=True, check=False
        )
        assert committed_before_abend.returncode == 12, committed_before_abend.stderr.decode(errors="replace")
        assert rows(connection, identifiers) == sorted([identifiers[0], identifiers[3]])
        print("local SYNCONRETURN, SYNCPOINT and ABEND Db2 transaction boundaries OK")
    finally:
        clean(connection, identifiers)
        ibm_db.close(connection)


if __name__ == "__main__":
    main()
