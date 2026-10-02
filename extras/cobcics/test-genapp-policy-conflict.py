#!/usr/bin/env python3
"""Two SSP1 terminals must reject an edit based on an old timestamp."""

import importlib.util
import os
from pathlib import Path
import socket
import subprocess
import tempfile

import ibm_db

from setup_genapp_connection import connection_string


spec = importlib.util.spec_from_file_location(
    "genapp_screens", Path(__file__).with_name("test-genapp-screens.py"))
screens = importlib.util.module_from_spec(spec)
spec.loader.exec_module(screens)


def policy_rows(database):
    row = ibm_db.fetch_tuple(ibm_db.exec_immediate(database,
        "SELECT M.MAKE,P.LASTCHANGED FROM GENAPPTEST.MOTOR M "
        "JOIN GENAPPTEST.POLICY P ON M.POLICYNUMBER=P.POLICYNUMBER "
        "WHERE P.POLICYNUMBER=1 AND P.CUSTOMERNUMBER=2"))
    assert row is not False
    key = b"M00000000020000000001".hex()
    record = ibm_db.fetch_tuple(ibm_db.exec_immediate(database,
        "SELECT V FROM GENAPPTEST.KSDSPOLY "
        f"WHERE K=BX'{key}'"))
    assert record is not False
    return row, bytes(record[0])


def submit(connection, issue, expiry, make):
    return screens.enter_fields(connection, [
        (4, 51, "1"), (5, 51, "2"),
        (6, 51, str(issue)), (7, 51, str(expiry)),
        (8, 51, make), (9, 51, "KA"),
        (10, 51, "8500"), (11, 51, "LL60LOO"),
        (12, 51, "ORANGE"), (13, 51, "1000"),
        (14, 51, "1999-07-19"), (15, 51, "0"),
        (16, 51, "450"), (22, 25, "4")])


def main() -> None:
    database = ibm_db.connect(connection_string(), "", "")
    try:
        issue, expiry = ibm_db.fetch_tuple(ibm_db.exec_immediate(database,
            "SELECT ISSUEDATE,EXPIRYDATE FROM GENAPPTEST.POLICY "
            "WHERE POLICYNUMBER=1 AND CUSTOMERNUMBER=2"))
        before, before_record = policy_rows(database)
        assert before[0].strip() == "FORD", before
        with tempfile.TemporaryDirectory() as storage:
            log = Path(storage) / "SSP1-conflict.log"
            host, port = screens.start_host("SSP1", "SSMAPP1", storage, log)
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=10) as first, \
                     socket.create_connection(("127.0.0.1", port), timeout=10) as second:
                    for connection in (first, second):
                        connection.settimeout(20)
                        screens.terminal.negotiate(connection)
                        screen = screens.enter_fields(connection,
                            [(4, 51, "1"), (5, 51, "2"), (22, 25, "4")])
                        assert "FORD".encode("cp037") in screen, log.read_text()

                    success = submit(first, issue, expiry, "FORDX")
                    assert "Motor Policy Updated".encode("cp037") in success, (
                        success.decode("cp037", "replace")[-450:], log.read_text())
                    updated, updated_record = policy_rows(database)
                    assert updated[0].strip() == "FORDX", updated
                    assert updated[1] != before[1], (before, updated, log.read_text())
                    assert updated_record != before_record

                    conflict = submit(second, issue, expiry, "FORDY")
                    assert "Error Updating Motor Policy".encode("cp037") in conflict, (
                        conflict.decode("cp037", "replace")[-450:], log.read_text())
                    final, final_record = policy_rows(database)
                    assert final[0].strip() == "FORDX", final
                    assert final[1] == updated[1], (updated, final)
                    assert final_record == updated_record
                    print("SSP1 two-terminal stale edit rejected; Db2 and KSDS unchanged OK")
            finally:
                host.terminate()
                host.wait(timeout=5)
    finally:
        ibm_db.close(database)
        for loader in ("setup-genapp-business-db.py", "setup-genapp-file-test-db.py"):
            result = subprocess.run(["python3", str(Path(__file__).with_name(loader))],
                env=os.environ, capture_output=True, text=True, timeout=60)
            if result.returncode:
                raise AssertionError(f"fixture restore {loader}: {result.stderr}")


if __name__ == "__main__":
    main()
