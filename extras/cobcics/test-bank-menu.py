#!/usr/bin/env python3
"""Exercise the original Bank of Z BNKMENU through cobcicsd."""

import argparse
import importlib.util
from pathlib import Path
import re
import socket
import subprocess


spec = importlib.util.spec_from_file_location(
    "cobcics_e2e", Path(__file__).with_name("test-e2e.py"))
terminal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(terminal)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server")
    parser.add_argument("bms")
    parser.add_argument("worker")
    args = parser.parse_args()
    process = subprocess.Popen(
        [args.server, "--bms", args.bms, "--transid", "OMEN",
         "--applid", "BANKAPP", "--route", "OMEN=" + args.worker,
         "--listen", "127.0.0.1:0"],
        stderr=subprocess.PIPE, text=True,
    )
    try:
        line = process.stderr.readline()
        match = re.search(r"listening at 127\.0\.0\.1:(\d+)", line)
        if not match:
            raise AssertionError(f"server did not start: {line}")
        with socket.create_connection(("127.0.0.1", int(match.group(1))), timeout=5) as connection:
            first = terminal.negotiate(connection)
            assert first[:2] == bytes([0xF5, 0xC2])
            assert "CICS Banking Sample Application".encode("cp037") in first, first.decode("cp037")
            address = bytes([0xC5, 0xD0])
            connection.sendall(bytes([0x7D]) + address + bytes([0x11]) + address
                               + "X".encode("cp037") + bytes([255, 239]))
            invalid = terminal.read_record(connection)
            assert "You must enter a valid value (1-7 or A).".encode("cp037") in invalid
            connection.sendall(bytes([0xF3, 0x40, 0x40, 255, 239]))
            ended = terminal.read_record(connection)
            assert "Session Ended".encode("cp037") in ended
            assert connection.recv(1) == b""
        print("Original BNKMENU/TN3270 integration OK")
    finally:
        process.terminate()
        process.wait(timeout=5)


if __name__ == "__main__":
    main()
