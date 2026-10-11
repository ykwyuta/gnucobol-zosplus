#!/usr/bin/env python3
"""Exercise an arbitrary CICS FILE layout against a Db2 table."""

import ctypes
import os
from pathlib import Path
import sys

import ibm_db


def word(value):
    return ctypes.create_string_buffer(value.to_bytes(4, "big"), 4)


def code(value):
    return int.from_bytes(value.raw, "big")


def area(value):
    return ctypes.create_string_buffer(value, len(value))


def main():
    table = f"GENAPPTEST.VSAMT{os.getpid()}"
    conninfo = (f"DATABASE={os.environ['DB2_DATABASE']};"
                f"HOSTNAME={os.environ['DB2_HOST']};PORT={os.environ['DB2_PORT']};"
                f"PROTOCOL=TCPIP;UID={os.environ['DB2_USER']};"
                f"PWD={os.environ['DB2_PASSWORD']};")
    setup = ibm_db.connect(conninfo, "", "")
    ibm_db.autocommit(setup, ibm_db.SQL_AUTOCOMMIT_OFF)
    ibm_db.exec_immediate(setup,
        f"CREATE TABLE {table} (K BINARY(4) NOT NULL PRIMARY KEY, V BLOB(12) NOT NULL)")
    ibm_db.commit(setup)
    os.environ["COBCICS_VSAM_BACKEND"] = "db2"
    os.environ["COBCICS_VSAM_TABLE_DBTST"] = table
    os.environ["COBCICS_VSAM_LAYOUT_DBTST"] = "12,4,2"
    os.environ["COBCICS_VSAM_DBTST"] = "/nonexistent/local-file"
    library = ctypes.CDLL(str(Path(sys.argv[1]).resolve()))
    for operation, count in (("READ", 10), ("WRITE", 9),
                             ("REWRITE", 6), ("DELETE", 6)):
        getattr(library, f"COBCICS_FILE_{operation}").argtypes = [ctypes.c_void_p] * count
    library.DB2SQL_END_TASK.argtypes = [ctypes.c_int]
    library.DB2SQL_END_TASK.restype = ctypes.c_int
    filename = area(b"DBTST   ")
    response, response2 = word(0), word(0)
    key = b"\xc1\xc2\x42\x43"
    record = b"\x00\xff" + key + b"\x0e\x42\x43\x0f\x00\n"
    changed = record[:6] + b"\x0e\x42\x44\x0f\x00\n"

    def read(mode=0):
        destination = ctypes.create_string_buffer(12)
        library.COBCICS_FILE_READ(filename, destination, word(12), word(12),
            area(key), word(4), word(4), area(bytes([mode])), response, response2)
        return code(response), destination.raw

    def write():
        library.COBCICS_FILE_WRITE(filename, area(record), word(12), word(12),
            area(key), word(4), word(4), response, response2)
        return code(response)

    try:
        assert write() == 0, (code(response), code(response2))
        assert read() == (0, record)
        assert library.DB2SQL_END_TASK(0) == 0
        assert read()[0] == 13
        assert write() == 0, (code(response), code(response2))
        assert library.DB2SQL_END_TASK(1) == 0
        assert read(4) == (0, record)
        library.COBCICS_FILE_REWRITE(filename, area(changed), word(12), word(12),
            response, response2)
        assert code(response) == 0, (code(response), code(response2))
        assert read() == (0, changed)
        assert library.DB2SQL_END_TASK(0) == 0
        assert read() == (0, record)
        library.COBCICS_FILE_DELETE(filename, area(key), word(4), word(4),
            response, response2)
        assert code(response) == 0, (code(response), code(response2))
        assert library.DB2SQL_END_TASK(1) == 0
        assert read()[0] == 13
        os.environ["COBCICS_VSAM_BACKEND"] = "postgres"
        assert read()[0] == 17
    finally:
        os.environ["COBCICS_VSAM_BACKEND"] = "db2"
        library.DB2SQL_END_TASK(0)
        ibm_db.exec_immediate(setup, f"DROP TABLE {table}")
        ibm_db.commit(setup)
        ibm_db.close(setup)
    print("Db2 CICS VSAM generic binary/offset/update/delete/commit/rollback OK")


if __name__ == "__main__":
    main()
