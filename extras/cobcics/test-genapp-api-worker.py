#!/usr/bin/env python3
"""Call the original customer and policy inquiry COBOL via COMMAREA."""

import os
from pathlib import Path
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
WORKER = ROOT / "build/genapp/GENAPP-API"


def call(transid: bytes, request_id: bytes, customer: int, policy: int = 0,
         *, storage: str) -> bytes:
    area = bytearray(b" " * 32500)
    area[:6] = request_id
    area[6:8] = b"00"
    area[8:18] = f"{customer:010d}".encode("ascii")
    area[18:28] = f"{policy:010d}".encode("ascii")
    frame = b"CCI3\0" + transid + b"COBCICS "
    frame += len(area).to_bytes(2, "big") + area + b"\0\0"
    result = subprocess.run([str(WORKER)], input=frame, capture_output=True,
        env=dict(os.environ, COBCICS_STORAGE_DIR=storage), timeout=30,
        check=False)
    assert result.returncode == 0, result.stderr.decode("utf-8", "replace")
    output = result.stdout
    assert output.startswith(b"CCO6\0\0\0\0\x04\0APIR"), output[:24]
    length = int.from_bytes(output[14:16], "big")
    assert length == 2048, length
    return output[16:16 + length]


def main() -> None:
    with tempfile.TemporaryDirectory() as storage:
        customer = call(b"GICU", b"01ICUS", 1, storage=storage)
        assert customer[6:8] == b"00", customer[:32]
        assert customer[18:28].strip() == b"Andrew", customer[18:48]
        assert customer[28:48].strip() == b"Pandy", customer[18:48]
        policy = call(b"GIPU", b"01IMOT", 2, 1, storage=storage)
        assert policy[6:8] == b"00", policy[:32]
        assert policy[100:115].strip() == b"FORD", policy[100:130]
        print("GenApp API worker called LGICUS01/LGIPOL01 and returned Db2 data OK")


if __name__ == "__main__":
    main()
