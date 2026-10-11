#!/usr/bin/env python3
"""Check CardDemo ACCTDAT CICS READ against a jclsh FB data set."""

import ctypes
import os
from pathlib import Path
import sys
import tempfile


def word(value):
    return ctypes.create_string_buffer(value.to_bytes(4, "big"), 4)


def main():
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "acctdat.dat"
        record = b"00000000001" + b" " * 289
        path.write_bytes(record)
        Path(str(path) + ".jclsh-meta").write_text("FB\t300\t11\t0\n")
        os.environ["COBCICS_VSAM_ACCTDAT"] = str(path)
        library = ctypes.CDLL(str(Path(sys.argv[1]).resolve()))
        library.COBCICS_FILE_READ.argtypes = [ctypes.c_void_p] * 10
        destination = ctypes.create_string_buffer(300)
        length, response, response2 = word(300), word(0), word(0)
        library.COBCICS_FILE_READ(ctypes.create_string_buffer(b"ACCTDAT "),
            destination, length, word(300), ctypes.create_string_buffer(b"00000000001"),
            word(11), word(11), ctypes.create_string_buffer(b"\0"), response, response2)
        assert int.from_bytes(response.raw, "big") == 0
        assert destination.raw == record
    print("CardDemo CICS ACCTDAT FB read passed")


if __name__ == "__main__":
    main()
