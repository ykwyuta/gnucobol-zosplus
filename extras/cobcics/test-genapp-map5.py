#!/usr/bin/env python3
"""Render and round-trip the otherwise unused SSMAPP5 claim map."""

import importlib.util
from pathlib import Path
import socket
import tempfile


spec = importlib.util.spec_from_file_location(
    "genapp_screens", Path(__file__).with_name("test-genapp-screens.py"))
screens = importlib.util.module_from_spec(spec)
spec.loader.exec_module(screens)


def main() -> None:
    with tempfile.TemporaryDirectory() as storage:
        log = Path(storage) / "SSP5.log"
        host, port = screens.start_host("SSP5", "SSMAPP5", storage, log)
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                connection.settimeout(15)
                initial = screens.terminal.negotiate(connection)
                assert "SSP5".encode("cp037") in initial
                assert "Claim Menu".encode("cp037") in initial
                response = screens.enter_fields(connection,
                    [(4, 51, "12"), (5, 51, "6"), (6, 51, "4"),
                     (10, 51, "Test cause"), (22, 25, "1")])
                assert "Claim input echoed".encode("cp037") in response, (
                    response.decode("cp037", "replace")[-400:], log.read_text())
                for expected in ("0000000012", "0000000006", "0000000004",
                                 "Test cause"):
                    assert expected.encode("cp037") in response, expected
            print("SSMAPP5 claim map displayed and input echoed over TN3270 OK")
        finally:
            host.terminate()
            host.wait(timeout=5)


if __name__ == "__main__":
    main()
