#!/usr/bin/env python3
"""Exercise HANDLE ABEND, ASSIGN ABCODE, CANCEL and ABNDFILE logging."""

import argparse
import os
from pathlib import Path
import subprocess
import tempfile


def request(aid):
    return b"CCI3" + bytes([aid]) + b"OMENBANKAPP " + b"\0\0\0\0"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("worker")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as directory:
        log = Path(directory) / "abend-records.bin"
        environment = dict(os.environ, COBCICS_ABEND_LOG=str(log))
        handled = subprocess.run([args.worker], input=request(0x7D),
                                 capture_output=True, env=environment)
        assert handled.returncode == 0, handled.stderr.decode(errors="replace")
        assert handled.stdout.startswith(b"CCO5"), handled.stdout
        data = log.read_bytes()
        size = int.from_bytes(data[:2], "big")
        assert size == len(data) - 2
        assert data[2 + 42:2 + 46] == b"TST4", data[2 + 42:2 + 46]

        cancelled = subprocess.run([args.worker], input=request(0xF3),
                                   capture_output=True, env=environment)
        assert cancelled.returncode == 12, cancelled.stderr.decode(errors="replace")
        assert b"TST5" in cancelled.stderr
        assert log.read_bytes() == data
        print("HANDLE ABEND, ASSIGN ABCODE, CANCEL and ABNDFILE WRITE OK")


if __name__ == "__main__":
    main()
