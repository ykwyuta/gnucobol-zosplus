#!/usr/bin/env python3
"""Exercise TLS, NVT and 3287 transport paths against the Windows build."""

import os
import socket
import subprocess
import time
import uuid
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SERVER = ROOT / "extras/cobcics/target/debug/cobcicsd.exe"
WC3270 = Path(r"C:\Program Files\wc3270\wc3270.exe")
PR3287 = Path(r"C:\Program Files\wc3270\pr3287.exe")
OPENSSL = Path(r"C:\Program Files\Git\usr\bin\openssl.exe")
OUT = ROOT / "build/wc3270-transport"


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def stop(*processes):
    for process in processes:
        if process is None:
            continue
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


def wait_for(path, needle, process, seconds=12):
    for _ in range(int(seconds * 10)):
        content = path.read_text(errors="replace") if path.exists() else ""
        if needle in content:
            return content
        if process.poll() is not None:
            raise AssertionError(f"process exited before {needle}: {content[-1500:]}")
        time.sleep(0.1)
    raise AssertionError(f"missing {needle} in {path}: {content[-2500:]}")


def launch_server(listen, *options, env=None):
    log = OUT / f"server-{uuid.uuid4().hex[:8]}.log"
    output = log.open("w")
    host = subprocess.Popen(
        [str(SERVER), "--bms", str(OUT / "transport.bms"),
         "--listen", f"127.0.0.1:{listen}", *options],
        stdout=subprocess.DEVNULL, stderr=output,
        env=env,
        creationflags=subprocess.CREATE_NO_WINDOW,
    )
    time.sleep(0.2)
    if host.poll() is not None:
        raise AssertionError(log.read_text(errors="replace"))
    return host, output


def tls_wc3270(starttls=False):
    cert, key = OUT / "cert.pem", OUT / "key.pem"
    subprocess.run(
        [str(OPENSSL), "req", "-x509", "-newkey", "rsa:2048", "-nodes",
         "-keyout", str(key), "-out", str(cert), "-days", "1", "-subj", "/CN=localhost",
         "-addext", "subjectAltName=IP:127.0.0.1"],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    listen = port()
    options = ["--tn3270e", "--tls-cert", str(cert), "--tls-key", str(key)]
    if starttls:
        options.append("--starttls")
    host, output = launch_server(listen, *options)
    trace = OUT / f"{'starttls' if starttls else 'tls'}-{uuid.uuid4().hex[:8]}.trace"
    client = None
    try:
        target = f"Y:127.0.0.1:{listen}" if starttls else f"L:Y:127.0.0.1:{listen}"
        client = subprocess.Popen(
            [str(WC3270), "+S", "-trace", "-tracefile", str(trace),
             "-model", "3279-2", target],
            stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT,
            creationflags=subprocess.CREATE_NEW_CONSOLE,
        )
        content = wait_for(trace, "TLS TEST", client)
        assert "TN3270E option negotiation complete" in content
        assert "RCVD TN3270E(3270-DATA" in content
        print(f"wc3270 {'STARTTLS' if starttls else 'TLS tunnel'} and TN3270E display OK")
    finally:
        stop(client, host)
        output.close()


def nvt_upgrade():
    listen = port()
    host, output = launch_server(listen, "--nvt", "--tn3270e")
    try:
        with socket.create_connection(("127.0.0.1", listen), timeout=3) as client:
            client.settimeout(3)
            assert b"COBCICS NVT gateway" in client.recv(256)
            client.sendall(b"HELP\r\n")
            assert b"Commands: 3270" in client.recv(256)
            client.sendall(b"3270\r\n")
            assert client.recv(32).startswith(b"\xff\xfd")
        print("NVT command gateway and 3270 upgrade OK")
    finally:
        stop(host)
        output.close()


def nvt_wc3270():
    listen = port()
    script_port = port()
    host, output = launch_server(listen, "--nvt")
    trace = OUT / f"nvt-{uuid.uuid4().hex[:8]}.trace"
    client = None
    try:
        client = subprocess.Popen(
            [str(WC3270), "+S", "-nvt", "-trace", "-tracefile", str(trace),
             "-scriptport", f"127.0.0.1:{script_port}", f"127.0.0.1:{listen}"],
            stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT,
            creationflags=subprocess.CREATE_NEW_CONSOLE,
        )
        content = wait_for(trace, "COBCICS NVT gateway", client)
        assert "connected-nvt" in content or "NVT" in content
        with socket.create_connection(("127.0.0.1", script_port), timeout=5) as control:
            control.settimeout(5)
            lines = control.makefile("r", encoding="utf-8", errors="replace")
            for command in ("String(3270)", "Enter"):
                control.sendall((command + "\n").encode("ascii"))
                while True:
                    answer = lines.readline().strip()
                    if answer == "ok":
                        break
                    if answer == "error" or not answer:
                        raise AssertionError(f"wc3270 NVT command failed: {command}: {answer}")
        wait_for(trace, "TLS TEST", client)
        print("wc3270 NVT banner and 3270 upgrade OK")
    finally:
        stop(client, host)
        output.close()


def printer_negotiation():
    listen = port()
    trace_dir = OUT / f"printer-{uuid.uuid4().hex[:8]}"
    trace_dir.mkdir()
    host, output = launch_server(listen, "--printer-queue", "PRNT",
                                 env=dict(os.environ, COBCICS_STORAGE_DIR=str(trace_dir)))
    client = None
    try:
        client = subprocess.Popen(
            [str(PR3287), "-trace", "-tracedir", str(trace_dir),
             "-assoc", "COBCICS1", f"127.0.0.1:{listen}"],
            stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        for _ in range(100):
            files = list(trace_dir.glob("*"))
            content = "\n".join(path.read_text(errors="replace") for path in files)
            if "TN3270E option negotiation complete" in content:
                break
            if client.poll() is not None:
                raise AssertionError(f"pr3287 exited: {content[-1500:]}")
            time.sleep(0.1)
        else:
            raise AssertionError(f"pr3287 negotiation incomplete: {content[-2500:]}")
        assert "IBM-3287-1" in content
        print("pr3287 associated LU and SCS negotiation OK")
    finally:
        stop(client, host)
        output.close()


def printer_spool_delivery(utf8=False, japanese=False):
    storage = OUT / f"storage-{uuid.uuid4().hex[:8]}"
    storage.mkdir()
    env = dict(os.environ, COBCICS_STORAGE_DIR=str(storage))
    listen = port()
    options = ["--printer-queue", "PRNT"]
    if utf8:
        options.extend(["--printer-input", "utf8"])
    if japanese:
        options.extend(["--charset", "IBM-939"])
    host, output = launch_server(listen, *options, env=env)
    try:
        with socket.create_connection(("127.0.0.1", listen), timeout=3) as client:
            client.settimeout(3)
            client.recv(128)
            client.sendall(b"\xff\xfb\x28")  # WILL TN3270E
            client.sendall(b"\xff\xfa\x28\x02\x07IBM-3287-1\xff\xf0")
            client.sendall(b"\xff\xfa\x28\x03\x07\x03\xff\xf0")
            received = b""
            while b"\xff\xfa\x28\x03\x04\x03\xff\xf0" not in received:
                received += client.recv(512)
            hash_value = 14695981039346656037
            for byte in b"PRNT    ":
                hash_value = ((hash_value ^ byte) * 1099511628211) & ((1 << 64) - 1)
            spool = storage / f"cobcics-tdq-{hash_value:016x}.data"
            japanese_host = bytes.fromhex("0e 45 62 45 66 48 e7 0f 15")
            if japanese:
                data = "日本語\r\n".encode("utf-8") if utf8 else japanese_host
                expected = japanese_host
            else:
                data = b"ABC\r\n" if utf8 else b"\xc1\xc2\xc3\x15"
                expected = b"\xc1\xc2\xc3\x15"
            spool.write_bytes(len(data).to_bytes(4, "big") + data)
            received = b""
            while b"\x08\x00\x00\x00\x00\xff\xef" not in received:
                received += client.recv(512)
            assert b"\x01\x00\x00\x00\x00" + expected + b"\xff\xef" in received
            cursor = storage / f"cobcics-tdq-{hash_value:016x}.cursor"
            for _ in range(20):
                if cursor.exists():
                    break
                time.sleep(0.05)
            assert cursor.read_bytes() == (4 + len(data)).to_bytes(8, "big")
            print(f"TDQ {'UTF-8' if utf8 else 'host EBCDIC'}"
                  f"{' IBM-939 Japanese' if japanese else ''} to TN3270E SCS and PRINT-EOJ OK")
    finally:
        stop(host)
        output.close()


def main():
    assert SERVER.is_file() and WC3270.is_file() and PR3287.is_file() and OPENSSL.is_file()
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "transport.bms").write_text(
        "T DFHMSD TYPE=MAP\nSCREEN DFHMDI SIZE=(24,80)\n"
        "TITLE DFHMDF POS=(1,1),LENGTH=8,ATTRB=PROT,*\n"
        "  INITIAL='TLS TEST'\n"
        "COMMAND DFHMDF POS=(3,1),LENGTH=70,ATTRB=(UNPROT,IC)\n",
        encoding="ascii",
    )
    tls_wc3270()
    tls_wc3270(starttls=True)
    nvt_upgrade()
    nvt_wc3270()
    printer_negotiation()
    printer_spool_delivery()
    printer_spool_delivery(utf8=True)
    printer_spool_delivery(japanese=True)
    printer_spool_delivery(utf8=True, japanese=True)


if __name__ == "__main__":
    main()
