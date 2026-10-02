#!/usr/bin/env python3
"""Probe malformed numeric input on all five GenApp terminal screens."""

import importlib.util
from pathlib import Path
import socket
import tempfile

import ibm_db

from setup_genapp_connection import connection_string


spec = importlib.util.spec_from_file_location(
    "genapp_screens", Path(__file__).with_name("test-genapp-screens.py"))
screens = importlib.util.module_from_spec(spec)
spec.loader.exec_module(screens)


def counts(database):
    values = []
    for table in ("CUSTOMER", "POLICY", "KSDSCUST", "KSDSPOLY"):
        row = ibm_db.fetch_tuple(ibm_db.exec_immediate(database,
            f"SELECT COUNT(*) FROM GENAPPTEST.{table}"))
        values.append(row[0])
    return tuple(values)


def main() -> None:
    database = ibm_db.connect(connection_string(), "", "")
    before = counts(database)
    try:
        with tempfile.TemporaryDirectory() as storage:
            for transid, map_name, _ in screens.ROUTES:
                log = Path(storage) / f"{transid}.log"
                host, port = screens.start_host(transid, map_name, storage, log)
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                        connection.settimeout(15)
                        screens.terminal.negotiate(connection)
                        fields = [(4, 51, "ABC"), (22, 25, "1")]
                        if transid != "SSC1":
                            fields.insert(1, (5, 51, "2"))
                        response = screens.enter_fields(connection, fields)
                        assert response and host.poll() is None, (
                            transid, log.read_text())
                        assert "No data was returned.".encode("cp037") in response, (
                            transid, response.decode("cp037", "replace")[-230:])
                        print(f"{transid}: malformed numeric inquiry returned no data")
                finally:
                    host.terminate()
                    host.wait(timeout=5)
        assert counts(database) == before, (before, counts(database))
    finally:
        ibm_db.close(database)


if __name__ == "__main__":
    main()
