#!/usr/bin/env python3
"""Run LGSE through its worker ABI and inspect isolated queue state."""

import os
import pathlib
import subprocess
import sys
import tempfile


def main() -> None:
    worker = pathlib.Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory() as storage:
        env = dict(os.environ, COBCICS_STORAGE_DIR=storage)
        for number in (11, 42, 77):
            payload = b"" if number == 11 else str(number).encode("ascii")
            request = b"CCI3\0LGSECOBCICS \0\0" + (
                b"\0\0" if not payload else
                b"\0\x01DATA    " + len(payload).to_bytes(2, "big") + payload
            )
            result = subprocess.run([str(worker)], input=request,
                                    capture_output=True, env=env, timeout=30,
                                    check=False)
            if result.returncode:
                raise AssertionError(result.stderr.decode("utf-8", "replace"))
            response = result.stdout
            assert response.startswith(b"CCO7"), response[:16]
            expected = f"HIGH CUSTOMER={number:010d}".encode("ascii")
            assert response[-24:] == expected, response[-24:]

            counters = list(pathlib.Path(storage).glob("cobcics-counter-*.data"))
            values = [int.from_bytes(path.read_bytes(), "big") for path in counters]
            assert values.count(number) == 1, values
            assert all(value in (0, number) for value in values), values

            queues = list(pathlib.Path(storage).glob("cobcics-tsq-*.data"))
            assert len(queues) == 1, queues
            raw = queues[0].read_bytes()
            count = int.from_bytes(raw[:4], "big")
            assert count == 3, count
            offset = 4
            items = []
            for _ in range(count):
                size = int.from_bytes(raw[offset:offset + 4], "big")
                offset += 4
                items.append(raw[offset:offset + size])
                offset += size
            assert offset == len(raw) and [len(item) for item in items] == [20, 23, 24]
            assert items[-1] == expected, items[-1]
    print("GenApp LGSE default/input/reinitialization OK")


if __name__ == "__main__":
    main()
