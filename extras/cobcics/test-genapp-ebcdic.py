#!/usr/bin/env python3
"""Exercise the IBM-037 GenApp screen against the normal TN3270 host."""

import importlib.util
import os
from pathlib import Path
import re
import socket
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "genapp_screens", Path(__file__).with_name("test-genapp-screens.py"))
screens = importlib.util.module_from_spec(spec)
spec.loader.exec_module(screens)


def main() -> None:
    with tempfile.TemporaryDirectory() as storage:
        log = Path(storage) / "ebcdic-host.log"
        command = [str(ROOT / "build/genapp/cobcicsd"),
                   "--bms", str(ROOT / "build/genapp/ssmap-normalized.bms"),
                   "--map", "SSMAPC1", "--transid", "SSC1",
                   "--route", f"SSC1={ROOT / 'build/genapp-ebcdic/lgtestc1'}",
                   "--listen", "127.0.0.1:0"]
        with log.open("w") as output:
            host = subprocess.Popen(command, stderr=output,
                env=dict(os.environ, COBCICS_STORAGE_DIR=storage))
        try:
            for _ in range(100):
                match = re.search(r"listening at 127\.0\.0\.1:(\d+)",
                                  log.read_text())
                if match:
                    break
                if host.poll() is not None:
                    raise AssertionError(log.read_text())
                time.sleep(0.05)
            else:
                raise AssertionError("TN3270 host did not start: " + log.read_text())
            try:
                with socket.create_connection(("127.0.0.1", int(match[1])), timeout=10) as terminal:
                    terminal.settimeout(20)
                    screen = screens.terminal.negotiate(terminal)
                    assert "SSC1".encode("cp037") in screen, log.read_text()
                    response = screens.enter_fields(terminal,
                        [(4, 51, "1"), (22, 25, "1")])
                    assert "Andrew".encode("cp037") in response, log.read_text()
            except Exception as error:
                raise AssertionError(log.read_text()) from error
            print("GenApp IBM-037 SSC1 screen and Db2 customer inquiry OK")
        finally:
            host.terminate()
            host.wait(timeout=5)


if __name__ == "__main__":
    main()
