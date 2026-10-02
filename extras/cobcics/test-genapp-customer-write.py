#!/usr/bin/env python3
"""Exercise GenApp SSC1 add across Db2 tables and the KSDS record."""

import os
from pathlib import Path
import importlib.util
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


def query(connection, sql):
    row = ibm_db.fetch_tuple(ibm_db.exec_immediate(connection, sql))
    return row if row is not False else None


def cleanup_fixture(connection) -> None:
    result = ibm_db.exec_immediate(connection,
        "SELECT CUSTOMERNUMBER FROM GENAPPTEST.CUSTOMER "
        "WHERE FIRSTNAME LIKE 'CodexTest%'")
    while (row := ibm_db.fetch_tuple(result)) is not False:
        number = int(row[0])
        key = f"{number:010d}".encode("ascii").hex()
        ibm_db.exec_immediate(connection, "DELETE FROM GENAPPTEST.KSDSCUST "
                              f"WHERE K=BX'{key}'")
        ibm_db.exec_immediate(connection, "DELETE FROM GENAPPTEST.CUSTOMER "
                              f"WHERE CUSTOMERNUMBER={number}")


def main() -> None:
    database = ibm_db.connect(connection_string(), "", "")
    cleanup_fixture(database)
    with tempfile.TemporaryDirectory() as storage:
        environment = dict(os.environ, COBCICS_STORAGE_DIR=storage)
        setup = subprocess.run([str(ROOT / os.environ.get(
            "GENAPP_BUILD_DIR", "build/genapp") / "GENAPP")],
            input=b"CCI3\0LGSECOBCICS \0\0\0\0", capture_output=True,
            env=environment, timeout=30, check=False)
        assert setup.returncode == 0, setup.stderr
        log = Path(storage) / "SSC1.log"
        host, port = screens.start_host("SSC1", "SSMAPC1", storage, log)
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                connection.settimeout(20)
                screens.terminal.negotiate(connection)
                try:
                    response = screens.enter_fields(connection, [
                        (5, 51, "CodexTest"), (6, 51, "Fixture"),
                        (7, 51, "1980-01-02"), (8, 51, "Test House"),
                        (9, 51, "12"), (10, 51, "AB12CD"),
                        (11, 51, "0123456789"), (12, 51, "0987654321"),
                        (13, 51, "test@example.com"), (22, 25, "2")])
                except Exception as error:
                    raise AssertionError(log.read_text()) from error
                assert "New Customer Inserted".encode("cp037") in response, (
                    response.decode("cp037", "replace")[-450:], log.read_text())
            row = query(database, "SELECT CUSTOMERNUMBER,FIRSTNAME,LASTNAME "
                        "FROM GENAPPTEST.CUSTOMER WHERE FIRSTNAME LIKE 'CodexTest%'")
            assert row is not None and row[2].strip() == "Fixture", (
                row, query(database, "SELECT CUSTOMERNUMBER,FIRSTNAME,LASTNAME "
                           "FROM GENAPPTEST.CUSTOMER WHERE CUSTOMERNUMBER>10"),
                response.decode("cp037", "replace")[-450:], log.read_text())
            number = int(row[0])
            assert query(database, "SELECT COUNT(*) FROM GENAPPTEST.CUSTOMER_SECURE "
                         f"WHERE CUSTOMERNUMBER={number}")[0] == 1
            key = f"{number:010d}".encode("ascii").hex()
            assert query(database, "SELECT COUNT(*) FROM GENAPPTEST.KSDSCUST "
                         f"WHERE K=BX'{key}'")[0] == 1
            print(f"SSC1 customer add committed to CUSTOMER, CUSTOMER_SECURE"
                  f" and KSDSCUST: {number}")
        finally:
            host.terminate()
            host.wait(timeout=5)
            cleanup_fixture(database)
            ibm_db.close(database)


if __name__ == "__main__":
    main()
