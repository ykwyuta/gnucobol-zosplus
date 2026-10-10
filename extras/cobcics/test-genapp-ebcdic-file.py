#!/usr/bin/env python3
"""Run IBM-037 LGCF/LGPF with the ASCII worker protocol."""

import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "vsam", Path(__file__).with_name("test-genapp-vsam-transactions.py"))
vsam = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vsam)


def call(worker, transid, payload, environment):
    result = subprocess.run([str(worker)],
        input=vsam.request(transid, payload.encode("ascii")),
        capture_output=True, env=environment, timeout=30)
    assert result.returncode == 0, result.stderr.decode("utf-8", "replace")
    assert result.stdout.startswith(b"CCO7"), result.stdout[:20]
    return result.stdout


def main() -> None:
    worker = ROOT / "build/genapp-ebcdic/GENAPP"
    with tempfile.TemporaryDirectory() as storage:
        environment = dict(os.environ, COBCICS_STORAGE_DIR=storage)
        setup = call(worker, b"LGSE", "2", environment)
        assert b"HIGH CUSTOMER=0000000002" in setup, setup[-80:]
        customer = call(worker, b"LGCF", "", environment)
        assert b"HIGH CUSTOMER=0000000001" in customer, customer[-80:]
        policy = call(worker, b"LGPF", "C0000000001", environment)
        assert b"Policy Key=C00000000010000000010" in policy, policy[-120:]
        print("GenApp IBM-037 LGCF/LGPF FILE queries OK")


if __name__ == "__main__":
    main()
