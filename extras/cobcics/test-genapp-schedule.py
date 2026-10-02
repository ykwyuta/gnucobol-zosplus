#!/usr/bin/env python3
"""Exercise GenApp's repeating SSST transaction and committed START file."""

import os
from pathlib import Path
import subprocess
import sys
import tempfile


def run(worker: Path, transid: bytes, environment: dict[str, str]) -> bytes:
    request = b"CCI3\0" + transid + b"COBCICS \0\0\0\0"
    result = subprocess.run([str(worker)], input=request, capture_output=True,
                            env=environment, timeout=30, check=False)
    if result.returncode:
        raise AssertionError(result.stderr.decode("utf-8", "replace"))
    assert result.stdout.startswith(b"CCO6"), result.stdout[:16]
    return result.stdout


def main() -> None:
    worker = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory() as storage:
        environment = dict(os.environ, COBCICS_STORAGE_DIR=storage,
                           COBCICS_NOW="1000")
        run(worker, b"LGSE", environment)
        run(worker, b"SSST", environment)
        directory = Path(storage)
        tasks = list(directory.glob("cobcics-start-*.task"))
        assert len(tasks) == 1, tasks
        assert tasks[0].name.startswith("cobcics-start-1060-"), tasks[0]
        assert tasks[0].read_bytes() == b"SSST"
        assert list(directory.glob("cobcics-tsq-*.data"))
        environment["COBCICS_NOW"] = "1060"
        run(worker, b"SSST", environment)
        tasks = list(directory.glob("cobcics-start-*.task"))
        assert sorted(path.name.split("-")[2] for path in tasks) == [
            "1060", "1120"]
    print("GenApp SSST counters/TS queues and delayed START OK")


if __name__ == "__main__":
    main()
