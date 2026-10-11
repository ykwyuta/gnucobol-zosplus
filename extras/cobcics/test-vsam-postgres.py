#!/usr/bin/env python3
"""Exercise the CICS FILE ABI against a real PostgreSQL BYTEA table."""

import ctypes
import os
from pathlib import Path
import sys


def word(value):
    return ctypes.create_string_buffer(value.to_bytes(4, "big"), 4)


def code(value):
    return int.from_bytes(value.raw, "big")


def area(value):
    return ctypes.create_string_buffer(value, len(value))


def main():
    table = f"vsam_pg_{os.getpid()}"
    os.environ["COBCICS_VSAM_BACKEND"] = "postgres"
    os.environ["COBCICS_VSAM_TABLE_PGTEST"] = table
    os.environ["COBCICS_VSAM_LAYOUT_PGTEST"] = "12,4,2"
    os.environ["COBCICS_VSAM_PGTEST"] = "/nonexistent/local-file"
    library = ctypes.CDLL(str(Path(sys.argv[1]).resolve()))
    for operation, count in (("READ", 10), ("WRITE", 9),
                             ("REWRITE", 6), ("DELETE", 6)):
        getattr(library, f"COBCICS_FILE_{operation}").argtypes = [ctypes.c_void_p] * count
    library.PGSQL_CONNECTION.restype = ctypes.c_void_p
    library.DB2SQL_END_TASK.argtypes = [ctypes.c_int]
    library.DB2SQL_END_TASK.restype = ctypes.c_int
    pq = ctypes.CDLL("libpq.so.5")
    pq.PQexec.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    pq.PQexec.restype = ctypes.c_void_p
    pq.PQresultStatus.argtypes = [ctypes.c_void_p]
    pq.PQresultStatus.restype = ctypes.c_int
    pq.PQresultErrorMessage.argtypes = [ctypes.c_void_p]
    pq.PQresultErrorMessage.restype = ctypes.c_char_p
    pq.PQclear.argtypes = [ctypes.c_void_p]

    def sql(statement):
        connection = library.PGSQL_CONNECTION()
        assert connection, "PostgreSQL connection failed"
        result = pq.PQexec(connection, statement.encode())
        assert result
        try:
            assert pq.PQresultStatus(result) == 1, pq.PQresultErrorMessage(result)
        finally:
            pq.PQclear(result)

    filename = area(b"PGTEST  ")
    response, response2 = word(0), word(0)

    def read(key, mode=0, key_size=4):
        destination = ctypes.create_string_buffer(12)
        ridfld = area(key.ljust(4, b"\x00"))
        length = word(12)
        library.COBCICS_FILE_READ(filename, destination, length, word(12),
            ridfld, word(key_size), word(4), area(bytes([mode])), response, response2)
        return code(response), destination.raw, ridfld.raw

    def write(record):
        library.COBCICS_FILE_WRITE(filename, area(record), word(12), word(12),
            area(record[2:6]), word(4), word(4), response, response2)
        return code(response)

    def rewrite(record):
        library.COBCICS_FILE_REWRITE(filename, area(record), word(12), word(12),
            response, response2)
        return code(response)

    def delete(key):
        library.COBCICS_FILE_DELETE(filename, area(key), word(4), word(4),
            response, response2)
        return code(response)

    first_key = b"\xc1\xc2\x42\x43"  # opaque EBCDIC/DBCS-like bytes
    second_key = b"\xc1\xc2\x42\x44"
    first = b"\x00\xff" + first_key + b"\x0e\x42\x43\x0f\x00\n"
    second = b"\x80\x01" + second_key + b"\x00\x1a\xfe\x7f\n\x00"
    assert len(first) == len(second) == 12
    created = False
    try:
        sql(f"CREATE TABLE {table} (K BYTEA PRIMARY KEY, V BYTEA NOT NULL)")
        assert library.DB2SQL_END_TASK(1) == 0
        created = True
        assert write(first) == 0
        assert write(first) == 14  # savepoint lets the transaction continue
        assert read(first_key)[:2] == (0, first)
        assert library.DB2SQL_END_TASK(0) == 0
        assert read(first_key)[0] == 13

        assert write(first) == 0
        assert library.DB2SQL_END_TASK(1) == 0
        assert read(first_key)[:2] == (0, first)
        assert write(second) == 0
        assert read(b"\xc1\xc2\x42\x42", mode=1)[:2] == (0, first)
        assert read(first_key[:2], mode=3, key_size=2)[:2] == (0, first)
        assert read(second_key)[:2] == (0, second)
        assert library.DB2SQL_END_TASK(0) == 0
        assert read(second_key)[0] == 13

        assert read(first_key, mode=4)[:2] == (0, first)
        changed = first[:6] + b"\x0e\x42\x44\x0f\x00\n"
        assert rewrite(changed) == 0
        assert read(first_key)[:2] == (0, changed)
        assert library.DB2SQL_END_TASK(0) == 0
        assert read(first_key)[:2] == (0, first)

        assert read(first_key, mode=4)[0] == 0
        assert rewrite(changed) == 0
        assert library.DB2SQL_END_TASK(1) == 0
        assert read(first_key)[:2] == (0, changed)
        assert delete(first_key) == 0
        assert delete(first_key) == 13
        assert read(first_key)[0] == 13
        assert library.DB2SQL_END_TASK(0) == 0
        assert read(first_key)[:2] == (0, changed)

        os.environ["COBCICS_VSAM_BACKEND"] = "db2"
        assert read(first_key)[0] == 17  # wrong SQL driver cannot become a fallback
        os.environ["COBCICS_VSAM_BACKEND"] = "postgres"
    finally:
        os.environ["COBCICS_VSAM_BACKEND"] = "postgres"
        library.DB2SQL_END_TASK(0)
        if created:
            sql(f"DROP TABLE {table}")
            assert library.DB2SQL_END_TASK(1) == 0
    print("PostgreSQL CICS VSAM binary/range/update/delete/commit/rollback OK")


if __name__ == "__main__":
    main()
