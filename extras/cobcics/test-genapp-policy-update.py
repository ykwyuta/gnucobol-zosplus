#!/usr/bin/env python3
"""Verify SSP1's two-step motor policy update."""

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


def main() -> None:
    database = ibm_db.connect(connection_string(), "", "")
    try:
        result = ibm_db.exec_immediate(database,
            "SELECT ISSUEDATE,EXPIRYDATE FROM GENAPPTEST.POLICY "
            "WHERE POLICYNUMBER=1 AND CUSTOMERNUMBER=2")
        issue, expiry = ibm_db.fetch_tuple(result)
        result = ibm_db.exec_immediate(database,
            "SELECT K,V FROM GENAPPTEST.KSDSPOLY")
        before = {}
        while (row := ibm_db.fetch_tuple(result)) is not False:
            before[bytes(row[0])] = bytes(row[1])
        with tempfile.TemporaryDirectory() as storage:
            log = Path(storage) / "SSP1.log"
            host, port = screens.start_host("SSP1", "SSMAPP1", storage, log)
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                    connection.settimeout(20)
                    screens.terminal.negotiate(connection)
                    first = screens.enter_fields(connection,
                        [(4, 51, "1"), (5, 51, "2"), (22, 25, "4")])
                    assert "FORD".encode("cp037") in first, log.read_text()
                    try:
                        second = screens.enter_fields(connection, [
                            (4, 51, "1"), (5, 51, "2"),
                            (6, 51, str(issue)), (7, 51, str(expiry)),
                            (8, 51, "FORDX"), (9, 51, "KA"),
                            (10, 51, "8500"), (11, 51, "LL60LOO"),
                            (12, 51, "ORANGE"), (13, 51, "1000"),
                            (14, 51, "1999-07-19"), (15, 51, "0"),
                            (16, 51, "450"), (22, 25, "4")])
                    except Exception as error:
                        raise AssertionError(log.read_text()) from error
                    assert "Motor Policy Updated".encode("cp037") in second, (
                        second.decode("cp037", "replace")[-450:], log.read_text(),
                        [(path.name, path.read_bytes()[:800]) for path in
                         Path(storage).glob("cobcics-tdq-*.data")])
                row = ibm_db.fetch_tuple(ibm_db.exec_immediate(database,
                    "SELECT MAKE FROM GENAPPTEST.MOTOR WHERE POLICYNUMBER=1"))
                assert row[0].strip() == "FORDX", (
                    row, ibm_db.fetch_tuple(ibm_db.exec_immediate(database,
                        "SELECT POLICYNUMBER,MAKE FROM GENAPPTEST.MOTOR "
                        "WHERE MAKE LIKE 'FORDX%'")),
                    second.decode("cp037", "replace")[-450:], log.read_text())
                result = ibm_db.exec_immediate(database,
                    "SELECT K,V FROM GENAPPTEST.KSDSPOLY")
                after = {}
                while (row := ibm_db.fetch_tuple(result)) is not False:
                    after[bytes(row[0])] = bytes(row[1])
                assert set(after) == set(before)
                assert after != before
                print("SSP1 motor inquiry/edit/submit updated MOTOR and KSDSPOLY OK")
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
