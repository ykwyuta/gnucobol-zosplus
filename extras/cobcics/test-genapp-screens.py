#!/usr/bin/env python3
"""Run the five original GenApp screen transactions through TN3270."""

import importlib.util
import os
from pathlib import Path
import re
import socket
import subprocess
import tempfile

import ibm_db
from setup_genapp_connection import connection_string


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "terminal", Path(__file__).with_name("test-e2e.py"))
terminal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(terminal)

ROUTES = (
    ("SSC1", "SSMAPC1", "Customer Menu"),
    ("SSP1", "SSMAPP1", "Motor Policy Menu"),
    ("SSP2", "SSMAPP2", "Policy"),
    ("SSP3", "SSMAPP3", "Policy"),
    ("SSP4", "SSMAPP4", "Policy"),
)

ADDRESS_CODE = bytes.fromhex(
    "40 c1 c2 c3 c4 c5 c6 c7 c8 c9 4a 4b 4c 4d 4e 4f "
    "50 d1 d2 d3 d4 d5 d6 d7 d8 d9 5a 5b 5c 5d 5e 5f "
    "60 61 62 63 64 65 66 67 68 69 6a 6b 6c 6d 6e 6f "
    "f0 f1 f2 f3 f4 f5 f6 f7 f8 f9 7a 7b 7c 7d 7e 7f")


def address(row: int, column: int) -> bytes:
    offset = (row - 1) * 80 + column - 1
    return bytes([ADDRESS_CODE[offset >> 6], ADDRESS_CODE[offset & 63]])


def enter_fields(connection: socket.socket,
                 fields: list[tuple[int, int, str]]) -> bytes:
    record = bytearray([0x7d, 0x40, 0x40])
    for row, column, value in fields:
        record += b"\x11" + address(row, column) + value.encode("cp037")
    connection.sendall(record + b"\xff\xef")
    return terminal.read_record(connection)


def policy_fixture(table: str, column: str) -> tuple[int, int, str]:
    connection = ibm_db.connect(connection_string(), "", "")
    try:
        result = ibm_db.exec_immediate(connection,
            f"SELECT P.POLICYNUMBER,P.CUSTOMERNUMBER,T.{column} "
            f"FROM GENAPPTEST.POLICY P JOIN GENAPPTEST.{table} T "
            "ON P.POLICYNUMBER=T.POLICYNUMBER "
            "ORDER BY P.POLICYNUMBER FETCH FIRST 1 ROW ONLY")
        row = ibm_db.fetch_tuple(result)
        assert row, table
        return int(row[0]), int(row[1]), str(row[2]).strip()
    finally:
        ibm_db.close(connection)


def start_host(transid: str, map_name: str, storage: str,
               log: Path) -> tuple[subprocess.Popen, int]:
    build_dir = ROOT / os.environ.get("GENAPP_BUILD_DIR", "build/genapp")
    executable = build_dir / (
        "map5-demo" if transid == "SSP5" else "lgtest" + transid[-2:].lower())
    bms = ROOT / "build/genapp/ssmap-normalized.bms"
    command = [str(ROOT / "build/genapp/cobcicsd"),
               "--bms", str(bms), "--map", map_name,
               "--transid", transid, "--applid", "GENAAPP",
               "--route", f"{transid}={executable}",
               "--listen", "127.0.0.1:0"]
    environment = dict(os.environ, COBCICS_STORAGE_DIR=storage)
    with log.open("w") as output:
        process = subprocess.Popen(command, stderr=output, env=environment)
    import time
    for _ in range(100):
        content = log.read_text()
        match = re.search(r"listening at 127\.0\.0\.1:(\d+)", content)
        if match:
            return process, int(match.group(1))
        if process.poll() is not None:
            raise AssertionError(content)
        time.sleep(0.05)
    process.terminate()
    raise AssertionError(f"host did not start: {log.read_text()}")


def main() -> None:
    with tempfile.TemporaryDirectory() as storage:
        for transid, map_name, heading in ROUTES:
            log = Path(storage) / f"{transid}.log"
            host, port = start_host(transid, map_name, storage, log)
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                    connection.settimeout(15)
                    try:
                        screen = terminal.negotiate(connection)
                    except Exception as error:
                        raise AssertionError(
                            f"{transid}: {error}; host: {log.read_text()}") from error
                    assert transid.encode("cp037") in screen, (transid, screen)
                    assert heading.encode("cp037") in screen, (transid, screen)
                    print(f"{transid}: initial {map_name} screen OK")
                    if transid == "SSC1":
                        inquiry = enter_fields(connection, [
                            (4, 51, "1"), (22, 25, "1")])
                        assert "Andrew".encode("cp037") in inquiry
                        assert "Pandy".encode("cp037") in inquiry
                        print("SSC1: customer 1 Db2 inquiry OK")
                    elif transid == "SSP1":
                        inquiry = enter_fields(connection, [
                            (4, 51, "1"), (5, 51, "2"), (22, 25, "1")])
                        assert "FORD".encode("cp037") in inquiry
                        print("SSP1: motor policy 1/customer 2 Db2 inquiry OK")
                    elif transid in ("SSP2", "SSP3", "SSP4"):
                        table, column = {
                            "SSP2": ("ENDOWMENT", "LIFEASSURED"),
                            "SSP3": ("HOUSE", "PROPERTYTYPE"),
                            "SSP4": ("COMMERCIAL", "ADDRESS"),
                        }[transid]
                        policy, customer, expected = policy_fixture(table, column)
                        inquiry = enter_fields(connection, [
                            (4, 51, str(policy)), (5, 51, str(customer)),
                            (22, 25, "1")])
                        assert expected.encode("cp037") in inquiry, (
                            transid, expected,
                            inquiry.decode("cp037", "replace")[-450:])
                        print(f"{transid}: {table.lower()} policy {policy}"
                              f"/customer {customer} Db2 inquiry OK")
                    invalid = enter_fields(connection, [(22, 25, "9")])
                    assert "valid option".encode("cp037") in invalid, (
                        transid, invalid.decode("cp037", "replace")[-240:])
                    connection.sendall(b"\xf3\x40\x40\xff\xef")
                    ended = terminal.read_record(connection)
                    assert "Transaction ended".encode("cp037") in ended, (
                        transid, ended.decode("cp037", "replace")[-240:])
                    print(f"{transid}: invalid option and PF3 OK")
                with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                    connection.settimeout(15)
                    terminal.negotiate(connection)
                    missing_fields = ([(4, 51, "9999999999"), (22, 25, "1")]
                                      if transid == "SSC1" else
                                      [(4, 51, "9999999999"), (5, 51, "1"),
                                       (22, 25, "1")])
                    missing = enter_fields(connection, missing_fields)
                    assert "No data was returned.".encode("cp037") in missing, (
                        transid, missing.decode("cp037", "replace")[-240:])
                    print(f"{transid}: missing customer/policy OK")
            finally:
                host.terminate()
                host.wait(timeout=5)
            if "terminal session:" in log.read_text():
                raise AssertionError(log.read_text())
    print("GenApp five TN3270 initial screens OK")


if __name__ == "__main__":
    main()
