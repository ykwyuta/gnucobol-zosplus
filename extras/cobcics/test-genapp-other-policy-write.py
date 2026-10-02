#!/usr/bin/env python3
"""Add and delete life, house, and commercial policies via TN3270."""

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

SPECS = (
    ("SSP2", "SSMAPP2", "ENDOWMENT", "FUNDNAME", "CodexFund",
     "Life", [(8, 51, "CodexFund"), (9, 51, "10"),
              (10, 51, "50000"), (11, 51, "Test Life"),
              (12, 51, "Y"), (13, 51, "N"), (14, 51, "N")]),
    ("SSP3", "SSMAPP3", "HOUSE", "PROPERTYTYPE", "CodexHouse",
     "House", [(8, 51, "CodexHouse"), (9, 51, "3"),
               (10, 51, "300000"), (11, 51, "Test House"),
               (12, 51, "12"), (13, 51, "AB12CD")]),
    ("SSP4", "SSMAPP4", "COMMERCIAL", "ADDRESS", "CodexAddress",
     "Commercial", [(8, 51, "CodexAddress"), (9, 51, "AB12CD"),
                     (10, 51, "51.00000"), (10, 65, "-1.00000"),
                     (11, 51, "Test Customer"), (12, 51, "OFFICES"),
                     (13, 51, "1"), (13, 57, "1000"),
                     (14, 51, "2"), (14, 57, "2000"),
                     (15, 51, "3"), (15, 57, "3000"),
                     (16, 51, "4"), (16, 57, "4000"),
                     (17, 51, "1"), (18, 51, "NONE")]),
)


def keys(connection):
    result = ibm_db.exec_immediate(connection,
        "SELECT K FROM GENAPPTEST.KSDSPOLY")
    values = set()
    while (row := ibm_db.fetch_tuple(result)) is not False:
        values.add(bytes(row[0]))
    return values


def policy_row(connection, table, column, value):
    result = ibm_db.exec_immediate(connection,
        "SELECT P.POLICYNUMBER,P.CUSTOMERNUMBER FROM GENAPPTEST.POLICY P "
        f"JOIN GENAPPTEST.{table} T ON P.POLICYNUMBER=T.POLICYNUMBER "
        f"WHERE T.{column} LIKE '{value}%'")
    row = ibm_db.fetch_tuple(result)
    return row if row is not False else None


def main() -> None:
    database = ibm_db.connect(connection_string(), "", "")
    try:
        for transid, map_name, table, column, value, title, details in SPECS:
            before = keys(database)
            with tempfile.TemporaryDirectory() as storage:
                log = Path(storage) / f"{transid}.log"
                host, port = screens.start_host(transid, map_name, storage, log)
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                        connection.settimeout(20)
                        screens.terminal.negotiate(connection)
                        try:
                            response = screens.enter_fields(connection,
                                [(5, 51, "2"), (6, 51, "2026-10-02"),
                                 (7, 51, "2027-10-01"), *details,
                                 (22, 25, "2")])
                        except Exception as error:
                            raise AssertionError(
                                f"{transid} add: {log.read_text()}") from error
                        assert f"New {title} Policy Inserted".encode("cp037") in response, (
                            transid, response.decode("cp037", "replace")[-380:],
                            log.read_text())
                    row = policy_row(database, table, column, value)
                    assert row is not None and int(row[1]) == 2, (transid, row)
                    number = int(row[0])
                    assert len(keys(database) - before) == 1, transid
                    print(f"{transid}: {title.lower()} add {number} in POLICY,"
                          f" {table}, KSDSPOLY OK")
                    with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                        connection.settimeout(20)
                        screens.terminal.negotiate(connection)
                        response = screens.enter_fields(connection,
                            [(4, 51, str(number)), (5, 51, "2"),
                             (22, 25, "3")])
                        assert f"{title} Policy Deleted".encode("cp037") in response, (
                            transid, response.decode("cp037", "replace")[-380:],
                            log.read_text())
                    assert policy_row(database, table, column, value) is None
                    assert keys(database) == before
                    print(f"{transid}: {title.lower()} delete from all 3 resources OK")
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
