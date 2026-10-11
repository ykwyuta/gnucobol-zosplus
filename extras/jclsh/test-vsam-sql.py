#!/usr/bin/env python3
"""Run unmodified IDCAMS DEFINE/REPRO/DELETE against a SQL table."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[2]


def main():
    backend, executable = sys.argv[1:]
    assert backend in ("db2", "postgres")
    base = ROOT / "build/jclsh"
    base.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="sql-vsam-", dir=base) as directory:
        folder = Path(directory)
        relative = folder.relative_to(ROOT).as_posix()
        table = (f"DB2INST1.VSAMJ{os.getpid()}" if backend == "db2"
                 else f"public.vsamj_{os.getpid()}")
        profile = folder / "profile.conf"
        profile_text = (
            "image=unused-for-rust-utilities\n"
            "env_file=extras/db2sql/.runtime.env\n"
            f"db2_schema=DB2INST1\n"
            "dsn_system=NONE\ndsn_plan=NONE\ndsn_library=NONE\n"
            f"dataset_root={relative}\n"
            f"dataset.TEST.VSAM.INPUT={relative}/input.dat\n"
            f"dataset.TEST.VSAM.BADINPUT={relative}/bad-input.dat\n"
            f"dataset.TEST.VSAM.KSDS={relative}/ksds.dat\n"
            f"dataset.TEST.VSAM.AIX={relative}/aix.dat\n"
            f"dataset.TEST.VSAM.OUTPUT={relative}/output.dat\n"
            f"vsam_backend={backend}\n"
            f"vsam_table.TEST.VSAM.KSDS={table}\n"
            "ld_library_path=unused\n")
        profile.write_text(profile_text)
        first_key = b"\xc1\xc2\x42\x43"
        second_key = b"\xc1\xc2\x42\x44"
        first = b"\x00\xff" + first_key + b"\x0e\x42\x43\x0f\x00\n"
        second = b"\x80\x01" + second_key + b"\x00\x1a\xfe\x7f\n\x00"
        (folder / "input.dat").write_bytes(second + first)
        (folder / "input.dat.jclsh-meta").write_text("FB\t12\t4\t2\n")
        (folder / "bad-input.dat").write_bytes(first + first[:6] + b"\x0e\x42\x44\x0f\x00\n")
        (folder / "bad-input.dat.jclsh-meta").write_text("FB\t12\t4\t2\n")
        command = [str(Path(executable).resolve()), "run",
                   "extras/jclsh/tests/vsam-sql.jcl", "--profile", str(profile)]
        missing = folder / "missing-table.conf"
        missing.write_text(profile_text.replace(
            f"vsam_table.TEST.VSAM.KSDS={table}\n", ""))
        rejected = subprocess.run(command[:-1] + [str(missing)], cwd=ROOT,
                                  capture_output=True, text=True)
        assert rejected.returncode != 0 and not (folder / "ksds.dat").exists()
        duplicate_command = command.copy()
        duplicate_command[2] = "extras/jclsh/tests/vsam-sql-duplicate.jcl"
        duplicate = subprocess.run(duplicate_command, cwd=ROOT,
                                   capture_output=True, text=True)
        assert duplicate.returncode != 0, duplicate.stdout + duplicate.stderr
        assert (folder / "output.dat").is_file(), duplicate.stdout + duplicate.stderr
        assert (folder / "output.dat").read_bytes() == b"", duplicate.stdout
        assert not (folder / "ksds.dat").exists()
        (folder / "output.dat").unlink()
        (folder / "output.dat.jclsh-meta").unlink()
        disposition_command = command.copy()
        disposition_command[2] = "extras/jclsh/tests/vsam-sql-disp-delete.jcl"
        disposition = subprocess.run(disposition_command, cwd=ROOT,
                                     capture_output=True, text=True)
        assert disposition.returncode == 0, disposition.stdout + disposition.stderr
        assert not (folder / "ksds.dat").exists()
        result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
        assert result.returncode == 0, result.stdout + result.stderr
        actual = (folder / "output.dat").read_bytes()
        assert actual == first + second, (result.stdout, actual.hex(), (first + second).hex())
        assert (folder / "output.dat.jclsh-meta").read_text().strip() == "FB\t12\t4\t2"
        assert "#JCLSH-AIX\tTEST.VSAM.KSDS\t2\t6\tUNIQUE" in (folder / "aix.dat").read_text()
        assert not (folder / "ksds.dat").exists()
    print(f"jclsh IDCAMS SQL VSAM {backend} define/repro/export/delete OK")


if __name__ == "__main__":
    main()
