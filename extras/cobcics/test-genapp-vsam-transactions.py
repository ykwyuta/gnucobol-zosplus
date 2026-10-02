#!/usr/bin/env python3
"""Exercise original LGCF/LGPF COBOL against the Db2 KSDS fixture."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile


def request(transid: bytes, payload: bytes = b"") -> bytes:
    header = b"CCI3\0" + transid + b"COBCICS \0\0"
    if not payload:
        return header + b"\0\0"
    return header + b"\0\x01DATA    " + len(payload).to_bytes(2, "big") + payload


def run(worker: Path, environment: dict[str, str],
        transid: bytes, payload: bytes = b"") -> bytes:
    result = subprocess.run([str(worker)], input=request(transid, payload),
                            capture_output=True, env=environment, timeout=30,
                            check=False)
    if result.returncode:
        raise AssertionError(result.stderr.decode("utf-8", "replace"))
    assert result.stdout.startswith(b"CCO6"), result.stdout[:16]
    return result.stdout


def main() -> None:
    worker = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory() as storage:
        environment = dict(os.environ, COBCICS_STORAGE_DIR=storage)
        assert run(worker, environment, b"LGSE", b"2").endswith(
            b"HIGH CUSTOMER=0000000002")
        assert run(worker, environment, b"LGCF").endswith(
            b"HIGH CUSTOMER=0000000001")
        assert b"Policy Key=C00000000010000000010" in run(
            worker, environment, b"LGPF", b"C0000000001")
        assert b"Policy Bad=" in run(
            worker, environment, b"LGPF", b"M9999999999")
    print("GenApp LGCF/LGPF Db2 KSDS found/not-found transactions OK")


if __name__ == "__main__":
    main()
