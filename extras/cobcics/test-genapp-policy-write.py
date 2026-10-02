#!/usr/bin/env python3
"""Exercise motor policy add from SSP1 and verify Db2 plus KSDS."""

import importlib.util
import os
from pathlib import Path
import socket
import subprocess
import tempfile

import ibm_db

from setup_genapp_connection import connection_string


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "genapp_screens", Path(__file__).with_name("test-genapp-screens.py"))
screens = importlib.util.module_from_spec(spec)
spec.loader.exec_module(screens)


def keys(connection):
    result = ibm_db.exec_immediate(connection,
        "SELECT K FROM GENAPPTEST.KSDSPOLY")
    values = set()
    while (row := ibm_db.fetch_tuple(result)) is not False:
        values.add(bytes(row[0]))
    return values


def main() -> None:
    database = ibm_db.connect(connection_string(), "", "")
    before = keys(database)
    try:
        with tempfile.TemporaryDirectory() as storage:
            log = Path(storage) / "SSP1.log"
            host, port = screens.start_host("SSP1", "SSMAPP1", storage, log)
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                    connection.settimeout(20)
                    screens.terminal.negotiate(connection)
                    try:
                        response = screens.enter_fields(connection, [
                            (5, 51, "2"), (6, 51, "2026-10-02"),
                            (7, 51, "2027-10-01"), (8, 51, "CodexMotor"),
                            (9, 51, "TestModel"), (10, 51, "9000"),
                            (11, 51, "AB12CDE"), (12, 51, "BLUE"),
                            (13, 51, "1600"), (14, 51, "2020-01-01"),
                            (15, 51, "0"), (16, 51, "500"),
                            (22, 25, "2")])
                    except Exception as error:
                        raise AssertionError(log.read_text()) from error
                    assert "New Motor Policy Inserted".encode("cp037") in response, (
                        response.decode("cp037", "replace")[-450:], log.read_text())
                result = ibm_db.exec_immediate(database,
                    "SELECT P.POLICYNUMBER,P.CUSTOMERNUMBER,M.MAKE "
                    "FROM GENAPPTEST.POLICY P JOIN GENAPPTEST.MOTOR M "
                    "ON P.POLICYNUMBER=M.POLICYNUMBER "
                    "WHERE M.MAKE LIKE 'CodexMotor%'")
                row = ibm_db.fetch_tuple(result)
                assert row is not False and int(row[1]) == 2, row
                added = keys(database) - before
                assert len(added) == 1, added
                print(f"SSP1 motor policy add committed to POLICY, MOTOR,"
                      f" KSDSPOLY: {row[0]}")
                with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                    connection.settimeout(20)
                    screens.terminal.negotiate(connection)
                    response = screens.enter_fields(connection, [
                        (4, 51, str(row[0])), (5, 51, "2"), (22, 25, "3")])
                    assert "Motor Policy Deleted".encode("cp037") in response, (
                        response.decode("cp037", "replace")[-450:], log.read_text())
                remaining = ibm_db.fetch_tuple(ibm_db.exec_immediate(database,
                    "SELECT COUNT(*) FROM GENAPPTEST.POLICY "
                    f"WHERE POLICYNUMBER={int(row[0])}"))[0]
                assert remaining == 0 and keys(database) == before
                print("SSP1 motor policy delete removed POLICY, MOTOR and KSDSPOLY OK")
            finally:
                host.terminate()
                host.wait(timeout=5)
    finally:
        ibm_db.close(database)
        for loader in ("setup-genapp-business-db.py", "setup-genapp-file-test-db.py"):
            result = subprocess.run(["python3", str(Path(__file__).with_name(loader))],
                env=os.environ, capture_output=True, text=True, timeout=60,
                check=False)
            if result.returncode:
                raise AssertionError(f"fixture restore {loader}: {result.stderr}")


if __name__ == "__main__":
    main()
