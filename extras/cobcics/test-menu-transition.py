#!/usr/bin/env python3
"""Exercise Bank of Z menu options and PF3 across BMS maps."""

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
    parser.add_argument("menu_bms")
    parser.add_argument("account_bms")
    parser.add_argument("menu_worker")
    parser.add_argument("account_worker")
    parser.add_argument("transfer_bms", nargs="?")
    parser.add_argument("transfer_worker", nargs="?")
    args = parser.parse_args()
    if bool(args.transfer_bms) != bool(args.transfer_worker):
        parser.error("transfer_bms and transfer_worker must be given together")
    routes = []
    if args.transfer_bms:
        routes = ["--route", "OTFN=" + args.transfer_worker,
                  "--map-route", "OTFN=" + args.transfer_bms]
    process = subprocess.Popen(
        [args.server, "--bms", args.menu_bms, "--transid", "OMEN",
         "--applid", "BANKAPP", "--route", "OMEN=" + args.menu_worker,
         "--route", "ODAC=" + args.account_worker,
         "--map-route", "ODAC=" + args.account_bms,
         *routes, "--listen", "127.0.0.1:0"],
        stderr=subprocess.PIPE, text=True,
    )
    try:
        line = process.stderr.readline()
        match = re.search(r"listening at 127\.0\.0\.1:(\d+)", line)
        if not match:
            raise AssertionError(f"server did not start: {line}")
        with socket.create_connection(("127.0.0.1", int(match.group(1))), timeout=5) as connection:
            first = terminal.negotiate(connection)
            assert "CICS Banking Sample Application - Main Menu".encode("cp037") in first
            destinations = [("2", "Provide an ACCOUNT number", "BNK1DA ", "BNK1DAC")]
            if args.transfer_bms:
                destinations.append(("7", "Transfer funds", "BNK1TF ", "BNK1TFN"))
            action = bytes([0xc5, 0x50])
            for choice, title, identifier, program in destinations:
                connection.sendall(bytes([0x7d]) + action + bytes([0x11]) + action
                                   + choice.encode("cp037") + bytes([255, 239]))
                screen = terminal.read_record(connection)
                assert title.encode("cp037") in screen, screen
                assert identifier.encode("cp037") in screen, screen
                connection.sendall(bytes([0xf3, 0x40, 0x40, 255, 239]))
                menu_again = terminal.read_record(connection)
                assert "CICS Banking Sample Application - Main Menu".encode("cp037") in menu_again
                assert "BNK1MA ".encode("cp037") in menu_again
                print(f"BNKMENU option {choice} -> {program} -> PF3 -> BNKMENU OK")
    finally:
        process.terminate()
        process.wait(timeout=5)


if __name__ == "__main__":
    main()
