#!/usr/bin/env python3
"""End-to-end IND$FILE DFT GET/PUT with the installed x3270 script client."""

import os
import socket
import subprocess
import time
import uuid
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SERVER = ROOT / "extras/cobcics/target/debug/cobcicsd.exe"
CLIENT = Path(r"C:\Program Files\wc3270\s3270.exe")
WC3270 = Path(r"C:\Program Files\wc3270\wc3270.exe")
OUTPUT = ROOT / "build/indfile-client"


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def case(direction, mode, payload, label=""):
    name = f"{direction}-{mode}-{uuid.uuid4().hex[:8]}"
    directory = OUTPUT / name
    directory.mkdir(parents=True)
    hostfile = "DOWNLOAD" if direction == "receive" else "UPLOAD"
    remote = directory / hostfile
    local = directory / "local.dat"
    if direction == "receive":
        remote.write_bytes(payload)
    else:
        local.write_bytes(payload)
    trace = directory / "client.trace"
    log = directory / "server.log"
    listen = port()
    with log.open("w") as output:
        server = subprocess.Popen(
            [str(SERVER), "--bms", str(OUTPUT / "command.bms"),
             "--tn3270e", "--indfile-root", str(directory),
             "--listen", f"127.0.0.1:{listen}"],
            stdout=subprocess.DEVNULL, stderr=output,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        try:
            cr = ",Cr=keep" if mode == "ascii" else ""
            command = (f"Transfer(Direction={direction},HostFile={hostfile},"
                       f"LocalFile={local},Host=cics,Mode={mode}{cr})\nQuit()\n")
            result = subprocess.run(
                [str(CLIENT), "-trace", "-tracefile", str(trace),
                 f"127.0.0.1:{listen}"],
                input=command, text=True, capture_output=True,
                timeout=20, creationflags=subprocess.CREATE_NO_WINDOW,
            )
            assert result.returncode == 0, result.stderr
            assert remote.exists() and local.exists(), (result.stdout, log.read_text())
            assert remote.read_bytes() == payload, (remote.read_bytes()[:80], payload[:80])
            assert local.read_bytes() == payload, (local.read_bytes()[:80], payload[:80])
            content = trace.read_text(errors="replace")
            assert "FileTransferData Open('FT:DATA')" in content
            assert "FileTransferData Open('FT:MSG')" in content
            assert "FileTransferData Data(rec=1) 7 bytes" in content
            print(f"IND$FILE DFT {direction} {mode}{' ' + label if label else ''}: "
                  f"{len(payload)} bytes OK")
        finally:
            server.terminate()
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait(timeout=5)


def wc_case():
    directory = OUTPUT / f"wc-{uuid.uuid4().hex[:8]}"
    directory.mkdir()
    payload = "wc3270 日本語 IND$FILE DFT\r\n".encode("utf-8")
    (directory / "DOWNLOAD").write_bytes(payload)
    local = directory / "local.dat"
    listen, script_port = port(), port()
    trace = directory / "wc.trace"
    with (directory / "server.log").open("w") as output:
        server = subprocess.Popen(
            [str(SERVER), "--bms", str(OUTPUT / "command.bms"),
             "--tn3270e", "--indfile-root", str(directory),
             "--listen", f"127.0.0.1:{listen}"],
            stdout=subprocess.DEVNULL, stderr=output,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        client = subprocess.Popen(
            [str(WC3270), "+S", "-trace", "-tracefile", str(trace),
             "-model", "3279-2", "-scriptport", f"127.0.0.1:{script_port}",
             f"127.0.0.1:{listen}"],
            stdin=subprocess.DEVNULL, stdout=output, stderr=output,
            creationflags=subprocess.CREATE_NEW_CONSOLE,
        )
        try:
            for _ in range(50):
                try:
                    connection = socket.create_connection(("127.0.0.1", script_port), timeout=1)
                    break
                except OSError:
                    time.sleep(0.1)
            else:
                raise AssertionError("wc3270 script port did not open")
            with connection:
                connection.settimeout(10)
                stream = connection.makefile("r", encoding="utf-8", errors="replace")
                def transfer(direction, hostfile, source):
                    command = (f"Transfer(Direction={direction},HostFile={hostfile},"
                               f"LocalFile={source},Host=cics,Mode=binary)\n")
                    connection.sendall(command.encode("ascii"))
                    while True:
                        line = stream.readline().strip()
                        if line == "ok":
                            return
                        if line == "error" or not line:
                            raise AssertionError(f"wc3270 Transfer failed: {line}")

                transfer("receive", "DOWNLOAD", local)
                assert local.read_bytes() == payload
                upload = directory / "local-upload.dat"
                upload.write_bytes(payload)
                transfer("send", "UPLOAD", upload)
            assert local.read_bytes() == payload
            assert (directory / "UPLOAD").read_bytes() == payload
            assert "FileTransferData Open('FT:MSG')" in trace.read_text(errors="replace")
            print("wc3270 Transfer action: IND$FILE DFT UTF-8 Japanese receive/send OK")
        finally:
            for process in (client, server):
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)


def main():
    assert SERVER.is_file() and CLIENT.is_file() and WC3270.is_file()
    OUTPUT.mkdir(parents=True, exist_ok=True)
    (OUTPUT / "command.bms").write_text(
        "FT DFHMSD TYPE=MAP\nSCREEN DFHMDI SIZE=(24,80)\n"
        "TITLE DFHMDF POS=(1,1),LENGTH=18,ATTRB=PROT,*\n"
        "  INITIAL='IND$FILE COMMAND'\n"
        "COMMAND DFHMDF POS=(3,1),LENGTH=74,ATTRB=(UNPROT,IC)\n",
        encoding="ascii",
    )
    binary = os.urandom(6000) + b"\xff\x00\r\n"
    for direction in ("receive", "send"):
        case(direction, "binary", binary)
        case(direction, "ascii", b"Plain ASCII text 123")
        case(direction, "binary", "日本語を転送\r\n".encode("utf-8"), "UTF-8 Japanese")
        # IBM-939 SO/DBCS/SI bytes for 日本語, independently verified by iconv.
        case(direction, "binary", bytes.fromhex("0e 45 62 45 66 48 e7 0f"),
             "IBM-939 Japanese")
    wc_case()


if __name__ == "__main__":
    main()
