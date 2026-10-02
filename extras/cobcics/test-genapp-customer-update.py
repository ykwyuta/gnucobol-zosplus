#!/usr/bin/env python3
"""Verify SSC1's inquiry/edit/submit path across Db2 and KSDS."""

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
        with tempfile.TemporaryDirectory() as storage:
            log = Path(storage) / "SSC1.log"
            host, port = screens.start_host("SSC1", "SSMAPC1", storage, log)
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                    connection.settimeout(20)
                    screens.terminal.negotiate(connection)
                    first = screens.enter_fields(connection,
                        [(4, 51, "1"), (22, 25, "4")])
                    assert "Andrew".encode("cp037") in first, log.read_text()
                    try:
                        second = screens.enter_fields(connection, [
                            (4, 51, "1"), (5, 51, "Andy"),
                            (6, 51, "Pandy"), (7, 51, "1950-07-11"),
                            (9, 51, "34"), (10, 51, "PI101OO"),
                            (11, 51, "01962 811234"),
                            (12, 51, "07799 123456"),
                            (13, 51, "A.Pandy@beebhouse.com"),
                            (22, 25, "4")])
                    except Exception as error:
                        raise AssertionError(log.read_text()) from error
                    assert "Customer details updated".encode("cp037") in second, (
                        second.decode("cp037", "replace")[-450:], log.read_text())
                row = ibm_db.fetch_tuple(ibm_db.exec_immediate(database,
                    "SELECT FIRSTNAME FROM GENAPPTEST.CUSTOMER "
                    "WHERE CUSTOMERNUMBER=1"))
                assert row[0].strip() == "Andy", row
                record = ibm_db.fetch_tuple(ibm_db.exec_immediate(database,
                    "SELECT V FROM GENAPPTEST.KSDSCUST "
                    "WHERE K=BX'30303030303030303031'"))[0]
                assert b"Andy" in bytes(record)
                print("SSC1 customer inquiry/edit/submit updated CUSTOMER and KSDSCUST OK")
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
