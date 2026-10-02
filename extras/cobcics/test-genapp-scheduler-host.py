#!/usr/bin/env python3
"""Verify the Rust host executes a due GenApp SSST and reschedules it."""

import os
from pathlib import Path
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parents[2]
def main() -> None:
    with tempfile.TemporaryDirectory() as storage:
        environment = dict(os.environ, COBCICS_STORAGE_DIR=storage,
                           COBCICS_NOW="1000")
        setup = subprocess.run([str(ROOT / "build/genapp/GENAPP")],
            input=b"CCI3\0LGSECOBCICS \0\0\0\0", capture_output=True,
            env=environment, timeout=30, check=False)
        assert setup.returncode == 0, setup.stderr
        due = Path(storage) / "cobcics-start-1060-1-1.task"
        due.write_bytes(b"SSST")
        bms = ROOT / "build/genapp/ssmap-normalized.bms"
        command = [str(ROOT / "build/genapp/cobcicsd"),
                   "--bms", str(bms), "--map", "SSMAPC1",
                   "--transid", "SSST", "--applid", "GENAAPP",
                   "--route", f"SSST={ROOT / 'build/genapp/GENAPP'}",
                   "--listen", "127.0.0.1:0"]
        log = Path(storage) / "host.log"
        environment["COBCICS_NOW"] = "1060"
        with log.open("w") as output:
            host = subprocess.Popen(command, stderr=output, env=environment)
        try:
            for _ in range(100):
                future = list(Path(storage).glob("cobcics-start-1120-*.task"))
                if future and not due.exists():
                    assert future[0].read_bytes() == b"SSST"
                    assert not list(Path(storage).glob("*.failed"))
                    print("Rust host executed due SSST and scheduled next minute OK")
                    break
                if host.poll() is not None:
                    raise AssertionError(log.read_text())
                time.sleep(0.05)
            else:
                raise AssertionError(log.read_text())
        finally:
            host.terminate()
            host.wait(timeout=5)


if __name__ == "__main__":
    main()
