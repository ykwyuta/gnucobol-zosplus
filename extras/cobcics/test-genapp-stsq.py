#!/usr/bin/env python3
"""Run the LGST logging transaction and inspect its TD/TS output."""

import os
import pathlib
import subprocess
import sys
import tempfile


def request(message: bytes) -> bytes:
    return (b"CCI3\0LGSTCOBCICS \0\0\0\x01DATA    "
            + len(message).to_bytes(2, "big") + message)


def records(path: pathlib.Path, *, ts: bool = False) -> list[bytes]:
    raw = path.read_bytes()
    offset = 4 if ts else 0
    expected_count = int.from_bytes(raw[:4], "big") if ts else None
    items = []
    while offset < len(raw):
        size = int.from_bytes(raw[offset:offset + 4], "big")
        offset += 4
        items.append(raw[offset:offset + size])
        offset += size
    assert offset == len(raw)
    if ts:
        assert len(items) == expected_count
    return items


def main() -> None:
    worker = pathlib.Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory() as storage:
        env = dict(os.environ, COBCICS_STORAGE_DIR=storage)
        for message in (b"first error", b"Q=TEST second error"):
            result = subprocess.run([str(worker)], input=request(message),
                                    capture_output=True, env=env, timeout=30,
                                    check=False)
            if result.returncode:
                raise AssertionError(result.stderr.decode("utf-8", "replace"))
            assert result.stdout.startswith(b"CCO7"), result.stdout[:16]
        directory = pathlib.Path(storage)
        tdq = list(directory.glob("cobcics-tdq-*.data"))
        assert len(tdq) == 1, tdq
        assert records(tdq[0]) == [b"L001 first error", b"L001 second error"]
        tsq = list(directory.glob("cobcics-tsq-*.data"))
        assert len(tsq) == 2, tsq
        assert sorted(records(path, ts=True)[0] for path in tsq) == [
            b"L001 first error", b"L001 second error"]
    print("GenApp LGST TD/TS logging and queue routing OK")


if __name__ == "__main__":
    main()
