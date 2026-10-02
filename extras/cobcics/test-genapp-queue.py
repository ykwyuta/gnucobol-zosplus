#!/usr/bin/env python3
"""Exercise the GenApp TS/TD queue ABI against an isolated directory."""

import ctypes
import os
import pathlib
import subprocess
import sys
import tempfile


def word(number: int) -> ctypes.Array:
    return ctypes.create_string_buffer(number.to_bytes(4, "big"), 4)


def code(value: ctypes.Array) -> int:
    return int.from_bytes(value.raw, "big")


def main() -> None:
    library = ctypes.CDLL(str(pathlib.Path(sys.argv[1]).resolve()))
    for function, count in (
        ("COBCICS_TSQ_READ", 8),
        ("COBCICS_TSQ_WRITE", 8),
        ("COBCICS_TSQ_DELETE", 3),
        ("COBCICS_TDQ_WRITE", 6),
        ("COBCICS_COUNTER_DEFINE", 5),
        ("COBCICS_COUNTER_DELETE", 5),
        ("COBCICS_COUNTER_GET", 5),
        ("COBCICS_COUNTER_QUERY", 5),
    ):
        getattr(library, function).argtypes = [ctypes.c_void_p] * count
    counter = ctypes.create_string_buffer(b"GENACUSTNUM     ", 16)
    pool = ctypes.create_string_buffer(b"GENA    ", 8)
    if len(sys.argv) == 3 and sys.argv[2] == "get-counter":
        value, resp, resp2 = word(0), word(0), word(0)
        library.COBCICS_COUNTER_GET(counter, pool, value, resp, resp2)
        assert code(resp) == 0, (code(resp), code(resp2))
        print(code(value))
        return
    name = ctypes.create_string_buffer(b"GENACNTL", 8)
    with tempfile.TemporaryDirectory() as storage:
        os.environ["COBCICS_STORAGE_DIR"] = storage
        destination = ctypes.create_string_buffer(32)
        length, capacity, item = word(32), word(32), word(1)
        resp, resp2 = word(0), word(0)
        next_item, append, rewrite = (ctypes.create_string_buffer(bytes([mode]), 1)
                                      for mode in (1, 0, 1))
        library.COBCICS_TSQ_READ(name, destination, length, capacity,
                                 item, append, resp, resp2)
        assert code(resp) == 44, code(resp)  # QIDERR

        first = ctypes.create_string_buffer(b"FIRST", 5)
        length = word(5)
        library.COBCICS_TSQ_WRITE(name, first, length, capacity,
                                  item, append, resp, resp2)
        assert code(resp) == 0
        second = ctypes.create_string_buffer(b"SECOND", 6)
        length = word(6)
        library.COBCICS_TSQ_WRITE(name, second, length, capacity,
                                  item, append, resp, resp2)
        assert code(resp) == 0

        length = word(32)
        library.COBCICS_TSQ_READ(name, destination, length, capacity,
                                 item, append, resp, resp2)
        assert code(resp) == 0 and destination.raw[:5] == b"FIRST"
        assert code(length) == 5
        length = word(32)
        library.COBCICS_TSQ_READ(name, destination, length, capacity,
                                 item, next_item, resp, resp2)
        assert code(resp) == 0 and destination.raw[:6] == b"SECOND"
        length = word(32)
        library.COBCICS_TSQ_READ(name, destination, length, capacity,
                                 item, next_item, resp, resp2)
        assert code(resp) == 26, code(resp)  # ITEMERR

        replacement = ctypes.create_string_buffer(b"CHANGED", 7)
        length = word(7)
        library.COBCICS_TSQ_WRITE(name, replacement, length, capacity,
                                  item, rewrite, resp, resp2)
        assert code(resp) == 0
        length = word(32)
        library.COBCICS_TSQ_READ(name, destination, length, capacity,
                                 item, append, resp, resp2)
        assert code(resp) == 0 and destination.raw[:7] == b"CHANGED"

        library.COBCICS_TSQ_DELETE(name, resp, resp2)
        assert code(resp) == 0
        library.COBCICS_TSQ_READ(name, destination, length, capacity,
                                 item, append, resp, resp2)
        assert code(resp) == 44

        # A CICS FROM field can be shorter than LENGTH when following fields
        # in the same COBOL group are included in the data area.
        group_name = ctypes.create_string_buffer(b"GENACNT2", 8)
        group = ctypes.create_string_buffer(b"HEADERDATA", 10)
        library.COBCICS_TSQ_WRITE(group_name, group, word(10), word(6),
                                  item, append, resp, resp2)
        assert code(resp) == 0, code(resp)
        library.COBCICS_TSQ_READ(group_name, destination, word(32), capacity,
                                 item, append, resp, resp2)
        assert code(resp) == 0 and destination.raw[:10] == b"HEADERDATA"

        tdq_name = ctypes.create_string_buffer(b"CSMT    ", 8)
        library.COBCICS_TDQ_WRITE(tdq_name, first, word(5), capacity,
                                  resp, resp2)
        assert code(resp) == 0
        log = list(pathlib.Path(storage).glob("cobcics-tdq-*.data"))
        assert len(log) == 1 and log[0].read_bytes() == b"\0\0\0\x05FIRST"

        value = word(11)
        library.COBCICS_COUNTER_DEFINE(counter, pool, value, resp, resp2)
        assert code(resp) == 0
        library.COBCICS_COUNTER_GET(counter, pool, value, resp, resp2)
        assert code(resp) == 0 and code(value) == 11
        library.COBCICS_COUNTER_QUERY(counter, pool, value, resp, resp2)
        assert code(resp) == 0 and code(value) == 12
        library.COBCICS_COUNTER_DEFINE(counter, pool, value, resp, resp2)
        assert code(resp) == 16 and code(resp2) == 202
        library.COBCICS_COUNTER_DELETE(counter, pool, value, resp, resp2)
        assert code(resp) == 0
        library.COBCICS_COUNTER_QUERY(counter, pool, value, resp, resp2)
        assert code(resp) == 16
        library.COBCICS_COUNTER_DEFINE(counter, pool, word(0), resp, resp2)
        assert code(resp) == 0
        processes = [subprocess.Popen(
            [sys.executable, __file__, sys.argv[1], "get-counter"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
        ) for _ in range(16)]
        numbers = []
        for process in processes:
            output, error = process.communicate(timeout=15)
            assert process.returncode == 0, error
            numbers.append(int(output.strip()))
        assert sorted(numbers) == list(range(16)), numbers
    print("GenApp TS/TD queue and counter ABI OK")


if __name__ == "__main__":
    main()
