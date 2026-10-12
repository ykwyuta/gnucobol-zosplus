#!/usr/bin/env python3
"""Run CardDemo's original COPAUS2C INSERT and duplicate-key UPDATE paths."""

import os
import pathlib
import subprocess
import sys
import uuid

import ibm_db


CARD_ROOT = pathlib.Path(
    "reference/aws-mainframe-modernization-carddemo/app/app-authorization-ims-db2-mq"
)


def connection():
    dsn = ";".join(
        f"{key}={os.environ[name]}"
        for key, name in (
            ("DATABASE", "DB2_DATABASE"),
            ("HOSTNAME", "DB2_HOST"),
            ("PORT", "DB2_PORT"),
            ("UID", "DB2_USER"),
            ("PWD", "DB2_PASSWORD"),
        )
    ) + ";PROTOCOL=TCPIP;"
    return ibm_db.connect(dsn, "", "")


def scalar(db, sql):
    return ibm_db.fetch_tuple(ibm_db.exec_immediate(db, sql))[0]


def request(aid, card, encoding):
    return (
        b"CCI3" + bytes([aid]) + b"CPVDCARDDEMO" + b"\0\x10"
        + card.encode(encoding) + b"\0\0"
    )


def run_worker(worker, aid, card):
    encoding = "cp037" if worker.endswith("-ebcdic") else "ascii"
    result = subprocess.run(
        [worker], input=request(aid, card, encoding), capture_output=True, check=False
    )
    assert result.returncode == 0, result.stderr.decode(errors="replace")
    assert result.stdout.startswith(b"CCO7"), result.stdout[:32]
    return result.stderr.decode(errors="replace")


def main():
    workers = sys.argv[1:]
    if not workers:
        raise SystemExit("expected at least one CardDemo fraud worker")
    db = connection()
    try:
        if scalar(db, "SELECT COUNT(*) FROM SYSCAT.SCHEMATA WHERE SCHEMANAME='CARDDEMO'") == 0:
            ibm_db.exec_immediate(db, "CREATE SCHEMA CARDDEMO")
        if scalar(db, "SELECT COUNT(*) FROM SYSCAT.TABLES WHERE TABSCHEMA='CARDDEMO' AND TABNAME='AUTHFRDS'") == 0:
            ddl = (CARD_ROOT / "ddl/AUTHFRDS.ddl").read_text(encoding="utf-8")
            ibm_db.exec_immediate(db, ddl.strip().rstrip(";"))
        for worker in workers:
            card = "T" + uuid.uuid4().hex[:15].upper()
            try:
                trace = run_worker(worker, 0, card)
                row = ibm_db.fetch_tuple(ibm_db.exec_immediate(
                    db, f"SELECT AUTH_FRAUD FROM CARDDEMO.AUTHFRDS WHERE CARD_NUM='{card}'"
                ))
                if not row or row[0].strip() != "F":
                    statement = ibm_db.exec_immediate(
                        db, "SELECT CARD_NUM, HEX(CARD_NUM), AUTH_FRAUD "
                            "FROM CARDDEMO.AUTHFRDS"
                    )
                    rows = []
                    while item := ibm_db.fetch_tuple(statement):
                        rows.append(item)
                    raise AssertionError((row, rows, trace))
                run_worker(worker, 0xF3, card)
                row = ibm_db.fetch_tuple(ibm_db.exec_immediate(
                    db, f"SELECT AUTH_FRAUD FROM CARDDEMO.AUTHFRDS WHERE CARD_NUM='{card}'"
                ))
                assert row and row[0].strip() == "R", row
                assert scalar(db, f"SELECT COUNT(*) FROM CARDDEMO.AUTHFRDS WHERE CARD_NUM='{card}'") == 1
                print(f"CardDemo original COPAUS2C Db2 INSERT and UPDATE OK: {worker}")
            finally:
                ibm_db.exec_immediate(db, f"DELETE FROM CARDDEMO.AUTHFRDS WHERE CARD_NUM='{card}'")
    finally:
        ibm_db.close(db)


if __name__ == "__main__":
    main()
