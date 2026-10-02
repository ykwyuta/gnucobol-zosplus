#!/usr/bin/env python3
"""Exercise a compiled COBOL worker through the Rust TN3270 server."""

import argparse
import datetime
import re
import socket
import subprocess


def read_exact(connection, length):
    result = bytearray()
    while len(result) < length:
        chunk = connection.recv(length - len(result))
        if not chunk:
            raise AssertionError("terminal connection closed early")
        result.extend(chunk)
    return bytes(result)


def read_record(connection):
    result = bytearray()
    while True:
        byte = read_exact(connection, 1)[0]
        if byte != 0xFF:
            result.append(byte)
            continue
        command = read_exact(connection, 1)[0]
        if command == 0xFF:
            result.append(0xFF)
        elif command == 0xEF:
            return bytes(result)
        else:
            raise AssertionError(f"unexpected Telnet command: {command:02x}")


def negotiate(connection):
    connection.settimeout(5)
    assert read_exact(connection, 15) == bytes([
        255, 253, 24, 255, 251, 0, 255, 253, 0,
        255, 251, 25, 255, 253, 25,
    ])
    connection.sendall(bytes([
        255, 251, 24, 255, 253, 0, 255, 251, 0,
        255, 253, 25, 255, 251, 25,
    ]))
    assert read_exact(connection, 6) == bytes([255, 250, 24, 1, 255, 240])
    connection.sendall(bytes([255, 250, 24, 0]) + b"IBM-3279-2" + bytes([255, 240]))
    return read_record(connection)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("server")
    parser.add_argument("bms")
    parser.add_argument("worker")
    parser.add_argument("immediate_worker")
    args = parser.parse_args()
    process = subprocess.Popen(
        [args.server, "--bms", args.bms, "--transid", "OMEN",
         "--applid", "BANKAPP",
         "--route", "OMEN=" + args.worker,
         "--route", "ODCS=" + args.immediate_worker,
         "--listen", "127.0.0.1:0"],
        stderr=subprocess.PIPE, text=True,
    )
    try:
        line = process.stderr.readline()
        match = re.search(r"listening at 127\.0\.0\.1:(\d+)", line)
        if not match:
            raise AssertionError(f"server did not start: {line}")
        with socket.create_connection(("127.0.0.1", int(match.group(1))), timeout=5) as connection:
            first = negotiate(connection)
            assert first[:2] == bytes([0xF5, 0xC2])
            assert "COBOL MENU READY".encode("cp037") in first
            assert "BANKAPP".encode("cp037") in first
            assert "MENUCICS".encode("cp037") in first
            # BNK1MAI ACTION data starts at row 5, column 17 (0-based address 336).
            address = bytes([0xC5, 0xD0])
            connection.sendall(bytes([0x7D]) + address + bytes([0x11]) + address
                               + "1".encode("cp037") + bytes([255, 239]))
            second = read_record(connection)
            assert second[:2] == bytes([0xF1, 0xC2])
            assert "COBOL MENU ACTION 1 Y Z".encode("cp037") in second
            connection.sendall(bytes([0x7D, 0x40, 0x40, 255, 239]))
            blank = read_record(connection)
            assert blank[:2] == bytes([0xF1, 0xC6])
            assert "COBOL MAPFAIL 36".encode("cp037") in blank
            connection.sendall(bytes([0x6C, 255, 239]))
            pa1 = read_record(connection)
            assert pa1[:2] == bytes([0xF1, 0xC6])
            assert "COBOL MAPFAIL 36".encode("cp037") in pa1
            connection.sendall(bytes([0xF3, 0x40, 0x40, 255, 239]))
            third = read_record(connection)
            assert "COBOL AID F3".encode("cp037") in third
            assert connection.recv(1) == b""
        with socket.create_connection(("127.0.0.1", int(match.group(1))), timeout=5) as connection:
            first = negotiate(connection)
            assert "COBOL MENU READY".encode("cp037") in first
            connection.sendall(bytes([0x6D, 255, 239]))
            cleared = read_record(connection)
            assert cleared == bytes([0xF5, 0xC2])
            assert connection.recv(1) == b""
        with socket.create_connection(("127.0.0.1", int(match.group(1))), timeout=5) as connection:
            first = negotiate(connection)
            assert "COBOL MENU READY".encode("cp037") in first
            address = bytes([0xC5, 0xD0])
            connection.sendall(bytes([0x7D]) + address + bytes([0x11]) + address
                               + "2".encode("cp037") + bytes([255, 239]))
            immediate = read_record(connection)
            assert immediate[:2] == bytes([0xF5, 0xC2])
            assert "COBOL IMMEDIATE ODCS".encode("cp037") in immediate
            connection.sendall(bytes([0xF3, 0x40, 0x40, 255, 239]))
            resumed = read_record(connection)
            assert "COBOL AID F3".encode("cp037") in resumed
            assert connection.recv(1) == b""
        with socket.create_connection(("127.0.0.1", int(match.group(1))), timeout=5) as connection:
            assert "COBOL MENU READY".encode("cp037") in negotiate(connection)
            address = bytes([0xC5, 0xD0])
            connection.sendall(bytes([0x7D]) + address + bytes([0x11]) + address
                               + "3".encode("cp037") + bytes([255, 239]))
            text = read_record(connection)
            assert text[:4] == bytes([0xF5, 0xC2, 0x1D, 0xF0])
            assert "Session Ended".encode("cp037") in text
            assert connection.recv(1) == b""
        for action in ("4", "6"):
            with socket.create_connection(("127.0.0.1", int(match.group(1))), timeout=5) as connection:
                assert "COBOL MENU READY".encode("cp037") in negotiate(connection)
                address = bytes([0xC5, 0xD0])
                connection.sendall(bytes([0x7D]) + address + bytes([0x11]) + address
                                   + action.encode("cp037") + bytes([255, 239]))
                clock = read_record(connection).decode("cp037")
                formatted = re.search(r"COBOL CLOCK (\d{2}/\d{2}/\d{4}) (\d{6})", clock)
                assert formatted, clock
                observed = datetime.datetime.strptime(
                    " ".join(formatted.groups()), "%d/%m/%Y %H%M%S")
                if action == "4":
                    assert abs((observed - datetime.datetime.now()).total_seconds()) < 10
                if action == "6":
                    assert formatted.groups() == ("29/02/2000", "123456")
                assert connection.recv(1) == b""
        with socket.create_connection(("127.0.0.1", int(match.group(1))), timeout=5) as connection:
            assert "COBOL MENU READY".encode("cp037") in negotiate(connection)
            address = bytes([0xC5, 0xD0])
            connection.sendall(bytes([0x7D]) + address + bytes([0x11]) + address
                               + "5".encode("cp037") + bytes([255, 239]))
            assert connection.recv(1) == b""
        request = (b"CCI3\x7dOMENBANKAPP \x00\x01Y\x00\x01"
                   b"ACTION  \x00\x015")
        abend = subprocess.run([args.worker], input=request,
                               capture_output=True, timeout=5)
        assert abend.returncode == 12
        assert abend.stdout == b""
        assert b"cobcics abend: HBNK" in abend.stderr
        print("COBOL/TN3270 integration OK")
    finally:
        process.terminate()
        process.wait(timeout=5)


if __name__ == "__main__":
    main()
