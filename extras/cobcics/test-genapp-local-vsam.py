#!/usr/bin/env python3
"""Exercise the COBOL-facing CICS FILE bridge with jclsh FB output."""

import ctypes
import os
from pathlib import Path
import shutil
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[2]


def word(value):
    return ctypes.create_string_buffer(value.to_bytes(4, "big"), 4)


def code(value):
    return int.from_bytes(value.raw, "big")


def area(value):
    return ctypes.create_string_buffer(value, len(value))


def main():
    source = ROOT / "build/jclsh/genapp-vsam"
    with tempfile.TemporaryDirectory() as directory:
        for name, path in (("KSDSCUST", "ksdscust.dat"),
                           ("KSDSPOLY", "ksdspoly.dat")):
            target = Path(directory) / path
            original = source / path
            shutil.copyfile(original, target)
            shutil.copyfile(str(original) + ".jclsh-meta", str(target) + ".jclsh-meta")
            os.environ[f"COBCICS_VSAM_{name}"] = str(target)
        library = ctypes.CDLL(str(Path(sys.argv[1]).resolve()))
        for operation, count in (("READ", 10), ("WRITE", 9),
                                 ("REWRITE", 6), ("DELETE", 6)):
            getattr(library, f"COBCICS_FILE_{operation}").argtypes = [ctypes.c_void_p] * count
        response, response2 = word(0), word(0)
        customer_file = area(b"KSDSCUST")
        policy_file = area(b"KSDSPOLY")
        original = (source / "ksdscust.dat").read_bytes()

        def read(name, key, size, key_size, mode=0):
            destination = ctypes.create_string_buffer(size)
            ridfld = area(key)
            length = word(size)
            library.COBCICS_FILE_READ(name, destination, length, word(size), ridfld,
                word(key_size), word(len(key)), area(bytes([mode])), response, response2)
            return code(response), destination.raw

        assert read(customer_file, b"CUST000001", 225, 10) == (0, original)
        policy = (source / "ksdspoly.dat").read_bytes()
        assert read(policy_file, policy[:21], 64, 21) == (0, policy)
        new_record = b"CUST000002" + original[10:]
        library.COBCICS_FILE_WRITE(customer_file, area(new_record), word(225), word(225),
            area(new_record[:10]), word(10), word(10), response, response2)
        assert code(response) == 0
        assert read(customer_file, b"CUST000002", 225, 10) == (0, new_record)
        assert read(customer_file, b"CUST000002", 225, 10, 4)[0] == 0
        changed = new_record[:10] + b"Z" + new_record[11:]
        library.COBCICS_FILE_REWRITE(customer_file, area(changed), word(225),
            word(225), response, response2)
        assert code(response) == 0
        assert read(customer_file, b"CUST000002", 225, 10) == (0, changed)
        library.COBCICS_FILE_DELETE(customer_file, area(b"CUST000002"),
            word(10), word(10), response, response2)
        assert code(response) == 0
        assert read(customer_file, b"CUST000002", 225, 10)[0] == 13
        assert (Path(directory) / "ksdscust.dat").stat().st_size == 225
        generic_path = Path(directory) / "other.dat"
        generic_original = b"XXABCDtail!!"
        generic_path.write_bytes(generic_original)
        Path(str(generic_path) + ".jclsh-meta").write_text("FB\t12\t4\t2\n")
        os.environ["COBCICS_VSAM_OTHER"] = str(generic_path)
        generic_file = area(b"OTHER   ")
        assert read(generic_file, b"ABCD", 12, 4) == (0, generic_original)
        generic_new = b"ZZEFGHhello!"
        library.COBCICS_FILE_WRITE(generic_file, area(generic_new), word(12), word(12),
            area(b"EFGH"), word(4), word(4), response, response2)
        assert code(response) == 0
        assert read(generic_file, b"EFGH", 12, 4, 4) == (0, generic_new)
        generic_changed = b"ZZEFGHchange"
        library.COBCICS_FILE_REWRITE(generic_file, area(generic_changed), word(12),
            word(12), response, response2)
        assert code(response) == 0
        assert read(generic_file, b"EFGH", 12, 4) == (0, generic_changed)
        library.COBCICS_FILE_DELETE(generic_file, area(b"EFGH"),
            word(4), word(4), response, response2)
        assert code(response) == 0
        assert read(generic_file, b"EFGH", 12, 4)[0] == 13
        for operation in ("begin", "prepare", "finish"):
            getattr(library, f"cobcics_vsam_{operation}").restype = ctypes.c_int
        library.cobcics_vsam_finish.argtypes = [ctypes.c_int]
        pending = b"CUST000003" + original[10:]
        customer_path = Path(directory) / "ksdscust.dat"
        assert library.cobcics_vsam_begin() == 0
        library.COBCICS_FILE_WRITE(customer_file, area(pending), word(225), word(225),
            area(pending[:10]), word(10), word(10), response, response2)
        assert code(response) == 0
        assert read(customer_file, pending[:10], 225, 10) == (0, pending)
        assert customer_path.read_bytes() == original
        assert library.cobcics_vsam_finish(0) == 0
        assert read(customer_file, pending[:10], 225, 10)[0] == 13
        assert library.cobcics_vsam_begin() == 0
        library.COBCICS_FILE_WRITE(customer_file, area(pending), word(225), word(225),
            area(pending[:10]), word(10), word(10), response, response2)
        assert code(response) == 0
        assert library.cobcics_vsam_prepare() == 0
        assert customer_path.read_bytes() == original
        assert library.cobcics_vsam_finish(1) == 0
        assert read(customer_file, pending[:10], 225, 10) == (0, pending)
        assert library.cobcics_vsam_begin() == 0
        assert read(customer_file, pending[:10], 225, 10, 4)[0] == 0
        changed = pending[:10] + b"Q" + pending[11:]
        library.COBCICS_FILE_REWRITE(customer_file, area(changed), word(225),
            word(225), response, response2)
        assert code(response) == 0
        assert read(customer_file, pending[:10], 225, 10) == (0, changed)
        assert library.cobcics_vsam_finish(0) == 0
        assert read(customer_file, pending[:10], 225, 10) == (0, pending)
    print("GenApp CICS bridge with jclsh FB VSAM passed")


if __name__ == "__main__":
    main()
