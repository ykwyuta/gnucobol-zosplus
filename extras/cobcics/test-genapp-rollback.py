#!/usr/bin/env python3
"""A duplicate KSDS write must roll back GenApp's earlier Db2 insert."""

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


def count(connection, table: str, where: str) -> int:
    result = ibm_db.exec_immediate(connection,
        f"SELECT COUNT(*) FROM GENAPPTEST.{table} WHERE {where}")
    return ibm_db.fetch_tuple(result)[0]


def main() -> None:
    key = b"0000000011"
    key_hex = key.hex()
    record_hex = (key + b" " * 215).hex()
    database = ibm_db.connect(connection_string(), "", "")
    ibm_db.exec_immediate(database, "DELETE FROM GENAPPTEST.KSDSCUST "
                          f"WHERE K=BX'{key_hex}'")
    assert count(database, "CUSTOMER", "CUSTOMERNUMBER=11") == 0
    ibm_db.exec_immediate(database, "INSERT INTO GENAPPTEST.KSDSCUST(K,V) "
                          f"VALUES(BX'{key_hex}',BX'{record_hex}')")
    try:
        with tempfile.TemporaryDirectory() as storage:
            environment = dict(os.environ, COBCICS_STORAGE_DIR=storage)
            setup = subprocess.run([str(ROOT / "build/genapp/GENAPP")],
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
                        screens.enter_fields(connection, [
                            (5, 51, "Rollback"), (6, 51, "Fixture"),
                            (7, 51, "1980-01-02"), (22, 25, "2")])
                    except (AssertionError, ConnectionError):
                        pass  # CICS ABEND closes the worker session.
                details = log.read_text()
                assert "LGV0" in details, details
                assert count(database, "CUSTOMER", "CUSTOMERNUMBER=11") == 0
                assert count(database, "CUSTOMER_SECURE", "CUSTOMERNUMBER=11") == 0
                assert count(database, "KSDSCUST", f"K=BX'{key_hex}'") == 1
                print("SSC1 duplicate KSDS write rolled back CUSTOMER insert OK")
            finally:
                host.terminate()
                host.wait(timeout=5)
    finally:
        ibm_db.exec_immediate(database, "DELETE FROM GENAPPTEST.KSDSCUST "
                              f"WHERE K=BX'{key_hex}'")
        ibm_db.close(database)


if __name__ == "__main__":
    main()
