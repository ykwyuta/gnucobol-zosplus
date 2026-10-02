#!/usr/bin/env python3
"""Check that CICS READ UPDATE holds a Db2 row lock across a task."""

import ctypes
import multiprocessing
from pathlib import Path

import ibm_db

from setup_genapp_connection import connection_string


ROOT = Path(__file__).resolve().parents[2]
KEY = b"0000000001"


def word(value: int):
    return ctypes.create_string_buffer(value.to_bytes(4, "big"), 4)


def lock_task(library_path: str, pipe) -> None:
    try:
        library = ctypes.CDLL(library_path)
        library.COBCICS_FILE_READ.argtypes = [ctypes.c_void_p] * 10
        library.DB2SQL_END_TASK.argtypes = [ctypes.c_int]
        library.DB2SQL_END_TASK.restype = ctypes.c_int
        record = ctypes.create_string_buffer(225)
        key = ctypes.create_string_buffer(KEY, len(KEY))
        response, response2 = word(0), word(0)
        library.COBCICS_FILE_READ(
            ctypes.create_string_buffer(b"KSDSCUST", 8), record,
            word(225), word(225), key, word(10), word(10),
            ctypes.create_string_buffer(b"\x04", 1), response, response2)
        pipe.send(int.from_bytes(response.raw, "big"))
        if pipe.recv() != "release":
            raise AssertionError("unexpected lock release request")
        pipe.send(library.DB2SQL_END_TASK(0))
    except BaseException as error:
        pipe.send(repr(error))
        raise
    finally:
        pipe.close()


def main() -> None:
    library = str((ROOT / "build/genapp/libgenapp_file.so").resolve())
    parent, child = multiprocessing.get_context("spawn").Pipe()
    process = multiprocessing.get_context("spawn").Process(
        target=lock_task, args=(library, child))
    process.start()
    child.close()
    try:
        assert parent.poll(20), "READ UPDATE did not finish"
        assert parent.recv() == 0, "READ UPDATE did not acquire the row"
        connection = ibm_db.connect(connection_string(), "", "")
        try:
            ibm_db.autocommit(connection, ibm_db.SQL_AUTOCOMMIT_OFF)
            ibm_db.exec_immediate(connection, "SET CURRENT LOCK TIMEOUT 1")
            try:
                ibm_db.exec_immediate(connection,
                    "UPDATE GENAPPTEST.KSDSCUST SET V=V "
                    "WHERE K=BX'30303030303030303031'")
            except Exception as error:
                assert "lock" in str(error).lower() or "911" in str(error), error
            else:
                raise AssertionError("another Db2 connection changed a held row")
            ibm_db.rollback(connection)
            parent.send("release")
            assert parent.poll(20), "READ UPDATE did not release the lock"
            assert parent.recv() == 0, "rollback failed"
            ibm_db.exec_immediate(connection,
                "UPDATE GENAPPTEST.KSDSCUST SET V=V "
                "WHERE K=BX'30303030303030303031'")
            ibm_db.rollback(connection)
        finally:
            ibm_db.close(connection)
        print("GenApp READ UPDATE locked KSDS row until task rollback OK")
    finally:
        if process.is_alive():
            parent.send("release")
        process.join(timeout=20)
        if process.is_alive():
            process.terminate()
            process.join(timeout=5)
        parent.close()
        assert process.exitcode == 0, process.exitcode


if __name__ == "__main__":
    main()
