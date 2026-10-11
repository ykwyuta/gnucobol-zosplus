#!/usr/bin/env python3
"""Exercise CICS KSDS operations against the isolated Db2 fixture."""

import ctypes
import os
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[2]
DATA = ROOT / "reference/cics-genapp/base/data"


def word(number: int) -> ctypes.Array:
    return ctypes.create_string_buffer(number.to_bytes(4, "big"), 4)


def code(value: ctypes.Array) -> int:
    return int.from_bytes(value.raw, "big")


def area(value: bytes) -> ctypes.Array:
    return ctypes.create_string_buffer(value, len(value))


def main() -> None:
    os.environ["COBCICS_VSAM_BACKEND"] = "db2"
    library = ctypes.CDLL(str(Path(sys.argv[1]).resolve()))
    for operation, arguments in (("READ", 10), ("WRITE", 9),
                                 ("REWRITE", 6), ("DELETE", 6)):
        getattr(library, f"COBCICS_FILE_{operation}").argtypes = [
            ctypes.c_void_p] * arguments
    library.DB2SQL_END_TASK.argtypes = [ctypes.c_int]
    library.DB2SQL_END_TASK.restype = ctypes.c_int
    customer = (DATA / "ksdscust.txt").read_bytes().splitlines()[0]
    policy = (DATA / "ksdspoly.txt").read_bytes().splitlines()[0]
    cust_file, poly_file = area(b"KSDSCUST"), area(b"KSDSPOLY")
    resp, resp2 = word(0), word(0)

    def read(file: ctypes.Array, key: bytes, record_size: int,
             key_size: int, mode: int = 0):
        destination = ctypes.create_string_buffer(record_size)
        ridfld = area(key)
        length = word(record_size)
        library.COBCICS_FILE_READ(file, destination, length,
            word(record_size), ridfld, word(key_size), word(len(key)),
            area(bytes([mode])), resp, resp2)
        return code(resp), destination.raw, code(length)

    assert read(cust_file, b"0000000001", 225, 10) == (0, customer, 225)
    assert read(cust_file, b"0000000000", 225, 10, 1) == (0, customer, 225)
    assert read(poly_file, b"C0000000001", 64, 11, 3) == (0, policy, 64)
    assert read(poly_file, policy[:21], 64, 21) == (0, policy, 64)

    new_customer = b"9999999999" + customer[10:]
    library.COBCICS_FILE_WRITE(cust_file, area(new_customer), word(225),
        word(10), area(new_customer[:10]), word(10), word(10), resp, resp2)
    assert code(resp) == 0, (code(resp), code(resp2))
    library.COBCICS_FILE_WRITE(cust_file, area(new_customer), word(225),
        word(10), area(new_customer[:10]), word(10), word(10), resp, resp2)
    assert code(resp) == 14, (code(resp), code(resp2))
    assert library.DB2SQL_END_TASK(0) == 0
    assert read(cust_file, new_customer[:10], 225, 10)[0] == 13

    assert read(cust_file, customer[:10], 225, 10, 4)[0] == 0
    changed = bytearray(customer)
    changed[10] = ord("Z")
    library.COBCICS_FILE_REWRITE(cust_file, area(bytes(changed)), word(225),
        word(10), resp, resp2)
    assert code(resp) == 0, (code(resp), code(resp2))
    assert read(cust_file, customer[:10], 225, 10)[1] == bytes(changed)
    assert library.DB2SQL_END_TASK(0) == 0
    assert read(cust_file, customer[:10], 225, 10)[1] == customer

    library.COBCICS_FILE_DELETE(poly_file, area(policy[:21]), word(21),
        word(21), resp, resp2)
    assert code(resp) == 0, (code(resp), code(resp2))
    assert read(poly_file, policy[:21], 64, 21)[0] == 13
    assert library.DB2SQL_END_TASK(0) == 0
    assert read(poly_file, policy[:21], 64, 21)[1] == policy
    assert library.DB2SQL_END_TASK(1) == 0
    print("GenApp Db2 KSDS read/range/write/rewrite/delete/rollback OK")


if __name__ == "__main__":
    main()
