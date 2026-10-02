#!/usr/bin/env python3
"""Verify the two-step life and house policy updates through TN3270."""

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

CASES = (
    ("SSP2", "SSMAPP2", 4, 8, "ENDOWMENT", "FUNDNAME", "CodexFund",
     "Life", "SELECT FUNDNAME,TERM,SUMASSURED,LIFEASSURED,WITHPROFITS,"
     "MANAGEDFUND,EQUITIES FROM GENAPPTEST.ENDOWMENT WHERE POLICYNUMBER=4"),
    ("SSP3", "SSMAPP3", 6, 4, "HOUSE", "PROPERTYTYPE", "CodexHouse",
     "House", "SELECT PROPERTYTYPE,BEDROOMS,VALUE,HOUSENAME,HOUSENUMBER,"
     "POSTCODE FROM GENAPPTEST.HOUSE WHERE POLICYNUMBER=6"),
)


def key_values(database):
    result = ibm_db.exec_immediate(database, "SELECT K,V FROM GENAPPTEST.KSDSPOLY")
    values = {}
    while (row := ibm_db.fetch_tuple(result)) is not False:
        values[bytes(row[0])] = bytes(row[1])
    return values


def main() -> None:
    database = ibm_db.connect(connection_string(), "", "")
    try:
        for transid, map_name, number, customer, table, column, changed, title, query in CASES:
            old = ibm_db.fetch_tuple(ibm_db.exec_immediate(database, query))
            dates = ibm_db.fetch_tuple(ibm_db.exec_immediate(database,
                "SELECT ISSUEDATE,EXPIRYDATE FROM GENAPPTEST.POLICY "
                f"WHERE POLICYNUMBER={number}"))
            before = key_values(database)
            with tempfile.TemporaryDirectory() as storage:
                log = Path(storage) / f"{transid}.log"
                host, port = screens.start_host(transid, map_name, storage, log)
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                        connection.settimeout(20)
                        screens.terminal.negotiate(connection)
                        first = screens.enter_fields(connection,
                            [(4, 51, str(number)), (5, 51, str(customer)),
                             (22, 25, "4")])
                        assert str(old[0]).strip().encode("cp037") in first, log.read_text()
                        fields = [(4, 51, str(number)), (5, 51, str(customer)),
                                  (6, 51, str(dates[0])), (7, 51, str(dates[1]))]
                        for row, value in enumerate((changed, *old[1:]), start=8):
                            fields.append((row, 51, str(value).strip()))
                        fields.append((22, 25, "4"))
                        try:
                            second = screens.enter_fields(connection, fields)
                        except Exception as error:
                            raise AssertionError(log.read_text()) from error
                        assert f"{title} Policy Updated".encode("cp037") in second, (
                            second.decode("cp037", "replace")[-450:], log.read_text())
                    result = ibm_db.fetch_tuple(ibm_db.exec_immediate(database,
                        f"SELECT {column} FROM GENAPPTEST.{table} "
                        f"WHERE POLICYNUMBER={number}"))
                    assert result[0].strip() == changed, (transid, result)
                    after = key_values(database)
                    assert set(after) == set(before) and after != before, transid
                    print(f"{transid}: {title.lower()} update committed to {table} and KSDSPOLY OK")
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
