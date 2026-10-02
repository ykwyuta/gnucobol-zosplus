#!/usr/bin/env python3
"""Generate portable IBM Japanese SBCS/DBCS maps from GNU iconv.

Run this in the glibc-based gnucobol-db2-build image. The binary format is
CJ01, 256 big-endian Unicode scalars (FFFFFFFF means invalid), a big-endian
DBCS count, then sorted big-endian (two-byte EBCDIC, Unicode scalar) pairs.
"""

from __future__ import annotations

import ctypes
from pathlib import Path
import struct


libc = ctypes.CDLL(None, use_errno=True)
libc.iconv_open.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
libc.iconv_open.restype = ctypes.c_void_p
libc.iconv.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p),
                       ctypes.POINTER(ctypes.c_size_t),
                       ctypes.POINTER(ctypes.c_void_p),
                       ctypes.POINTER(ctypes.c_size_t)]
libc.iconv.restype = ctypes.c_size_t
libc.iconv_close.argtypes = [ctypes.c_void_p]
libc.iconv_close.restype = ctypes.c_int
INVALID = ctypes.c_size_t(-1).value
OUTPUT = Path(__file__).resolve().parent / "data"


def decode(descriptor: int, source: bytes) -> int | None:
    libc.iconv(descriptor, None, None, None, None)
    input_buffer = ctypes.create_string_buffer(source)
    input_pointer = ctypes.c_void_p(ctypes.addressof(input_buffer))
    input_left = ctypes.c_size_t(len(source))
    output_buffer = ctypes.create_string_buffer(16)
    output_pointer = ctypes.c_void_p(ctypes.addressof(output_buffer))
    output_left = ctypes.c_size_t(len(output_buffer))
    result = libc.iconv(descriptor, ctypes.byref(input_pointer),
                        ctypes.byref(input_left), ctypes.byref(output_pointer),
                        ctypes.byref(output_left))
    if result == INVALID or input_left.value:
        return None
    text = output_buffer.raw[:len(output_buffer) - output_left.value]
    try:
        decoded = text.decode("utf-8")
    except UnicodeDecodeError:
        return None
    return ord(decoded) if len(decoded) == 1 else None


def main() -> None:
    OUTPUT.mkdir(exist_ok=True)
    for number in (930, 939, 1390, 1399):
        descriptor = libc.iconv_open(b"UTF-8", f"IBM{number}".encode("ascii"))
        if descriptor == ctypes.c_void_p(-1).value:
            raise RuntimeError(f"GNU iconv does not support IBM{number}")
        try:
            sbcs = [decode(descriptor, bytes([byte]))
                    if byte not in (0x0e, 0x0f) else None
                    for byte in range(256)]
            dbcs = []
            for lead in range(0x40, 0xff):
                for trail in range(0x40, 0xff):
                    if (lead == 0x40) != (trail == 0x40):
                        continue
                    scalar = decode(descriptor, bytes((0x0e, lead, trail, 0x0f)))
                    if scalar is not None:
                        dbcs.append((lead << 8 | trail, scalar))
        finally:
            libc.iconv_close(descriptor)
        payload = bytearray(b"CJ01")
        for scalar in sbcs:
            payload.extend(struct.pack(">I", scalar if scalar is not None else 0xffffffff))
        payload.extend(struct.pack(">I", len(dbcs)))
        for code, scalar in dbcs:
            payload.extend(struct.pack(">HI", code, scalar))
        target = OUTPUT / f"ibm{number}.bin"
        target.write_bytes(payload)
        print(f"{target.name}: {sum(x is not None for x in sbcs)} SBCS, "
              f"{len(dbcs)} DBCS, {len(payload)} bytes")


if __name__ == "__main__":
    main()
