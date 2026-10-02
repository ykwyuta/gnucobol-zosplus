#!/usr/bin/env python3
"""Invoke LGASTAT1 through both COMMAREA and event containers."""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]


def request(transid: bytes, payload: bytes,
            containers: tuple[tuple[bytes, bytes, bytes], ...] | None = None) -> bytes:
    result = (b"CCI4\0" if containers is not None else b"CCI3\0")
    result += transid + b"COBCICS " + len(payload).to_bytes(2, "big")
    result += payload + b"\0\0"
    if containers is not None:
        result += len(containers).to_bytes(2, "big")
        for channel, name, value in containers:
            assert len(channel) <= 16 and len(name) <= 16
            result += channel.ljust(16, b" ") + name.ljust(16, b" ")
            result += len(value).to_bytes(2, "big") + value
    return result


def queue_path(storage: str, name: bytes) -> Path:
    value = 14695981039346656037
    for byte in name:
        value = ((value ^ byte) * 1099511628211) & ((1 << 64) - 1)
    return Path(storage) / f"cobcics-tsq-{value:016x}.data"


def main() -> None:
    worker = ROOT / "build/genapp/GENAPP"
    cases = (
        ("COMMAREA", b"01ACUS00" + b" " * 10, None),
        ("containers", b"", ((b"DFHEP.EVENT", b"DFHEP.DATA.00001", b"01ACUS"),
                              (b"DFHEP.EVENT", b"DFHEP.DATA.00002", b"00"))),
    )
    for label, payload, containers in cases:
        with tempfile.TemporaryDirectory() as storage:
            environment = dict(os.environ, COBCICS_STORAGE_DIR=storage)
            for frame in (request(b"LGSE", b"2"),
                          request(b"LGAS", payload, containers)):
                result = subprocess.run([str(worker)], input=frame,
                    capture_output=True, env=environment, timeout=20, check=False)
                assert result.returncode == 0, result.stderr.decode("utf-8", "replace")
                assert result.stdout.startswith(b"CCO6"), result.stdout[:16]
            queue = queue_path(storage, b"GENASTRT")
            assert queue.exists() and int.from_bytes(queue.read_bytes()[:4], "big") == 1
            print(f"GenApp LGASTAT1 event {label} wrote TS queue OK")


if __name__ == "__main__":
    main()
