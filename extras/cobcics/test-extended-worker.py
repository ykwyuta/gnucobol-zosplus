#!/usr/bin/env python3
"""Check a compiled COBOL worker's CCO7 BMS color/highlight response."""

import os
import subprocess
import sys


def take_u16(data, offset):
    return int.from_bytes(data[offset:offset + 2], "big"), offset + 2


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: test-extended-worker.py WORKER")
    request = b"CCI3\0OMENBANKAPP \0\0\0\0"
    result = subprocess.run([sys.argv[1]], input=request, capture_output=True,
                            env=os.environ.copy(), check=True)
    data = result.stdout
    assert data[:4] == b"CCO7", (data[:20], result.stderr)
    count, offset = take_u16(data, 4)
    fields = {}
    for _ in range(count):
        name = data[offset:offset + 8].decode("ascii").strip()
        length, offset = take_u16(data, offset + 8)
        fields[name] = data[offset:offset + length].decode("utf-8")
        offset += length
    count, offset = take_u16(data, offset)
    offset += count * 9  # Basic BMS attribute overrides.
    count, offset = take_u16(data, offset)
    attributes = {}
    for _ in range(count):
        name = data[offset:offset + 8].decode("ascii").strip()
        kind, value = data[offset + 8:offset + 10]
        attributes[(name, chr(kind))] = value
        offset += 10
    assert fields["MESSAGE"].rstrip() == "COLOR TEST", fields
    assert attributes == {("MESSAGE", "C"): 0xF2,
                          ("MESSAGE", "H"): 0xF4}, attributes
    assert data[offset] == 0, data[offset:]  # SEND MAP, not DATAONLY.
    print("COBOL CCO7: MESSAGE=COLOR TEST, red and underline OK")


if __name__ == "__main__":
    main()
