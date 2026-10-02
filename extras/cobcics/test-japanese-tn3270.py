#!/usr/bin/env python3
"""Round-trip Japanese IBM-1399 through TN3270, Rust, and COBOL."""

import importlib.util
import os
from pathlib import Path
import re
import socket
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
BUILD = ROOT / "build/cobcics-japanese"
spec = importlib.util.spec_from_file_location(
    "terminal", Path(__file__).with_name("test-e2e.py"))
terminal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(terminal)
ADDRESS_CODE = bytes.fromhex(
    "40 c1 c2 c3 c4 c5 c6 c7 c8 c9 4a 4b 4c 4d 4e 4f "
    "50 d1 d2 d3 d4 d5 d6 d7 d8 d9 5a 5b 5c 5d 5e 5f "
    "60 61 62 63 64 65 66 67 68 69 6a 6b 6c 6d 6e 6f "
    "f0 f1 f2 f3 f4 f5 f6 f7 f8 f9 7a 7b 7c 7d 7e 7f")


def ebcdic(text: str) -> bytes:
    return subprocess.run(["iconv", "-f", "UTF-8", "-t", "IBM1399"],
                          input=text.encode(), capture_output=True,
                          check=True).stdout


def address(row: int, column: int) -> bytes:
    offset = (row - 1) * 80 + column - 1
    return bytes((ADDRESS_CODE[offset >> 6], ADDRESS_CODE[offset & 63]))


def main() -> None:
    with tempfile.TemporaryDirectory() as storage:
        log = Path(storage) / "cobcicsd.log"
        env = dict(os.environ, COBCICS_STORAGE_DIR=storage)
        command = [str(BUILD / "cobcicsd"), "--bms",
                   str(ROOT / "extras/cobcics/japanese-demo.bms"),
                   "--charset", "IBM-1399", "--transid", "JP01",
                   "--route", f"JP01={BUILD / 'JPDEMO'}",
                   "--listen", "127.0.0.1:0"]
        with log.open("w") as output:
            host = subprocess.Popen(command, stderr=output, env=env)
        try:
            for _ in range(100):
                match = re.search(r"listening at 127\.0\.0\.1:(\d+)",
                                  log.read_text())
                if match:
                    break
                if host.poll() is not None:
                    raise AssertionError(log.read_text())
                import time
                time.sleep(0.05)
            else:
                raise AssertionError(log.read_text())
            with socket.create_connection(("127.0.0.1", int(match[1])), timeout=10) as connection:
                connection.settimeout(10)
                initial = terminal.negotiate(connection)
                assert ebcdic("日本語を入力") in initial, log.read_text()
                action = address(4, 3)
                record = b"\x7d\x40\x40\x11" + action + ebcdic("日本語")
                connection.sendall(record + b"\xff\xef")
                echoed = terminal.read_record(connection)
                assert ebcdic("日本語") in echoed, log.read_text()
                assert host.poll() is None, log.read_text()
            print("IBM-1399 Japanese TN3270 -> Rust -> COBOL -> Rust -> TN3270 OK")
        finally:
            host.terminate()
            host.wait(timeout=5)


if __name__ == "__main__":
    main()
