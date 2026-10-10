#!/usr/bin/env python3
"""Exercise IBM-939 Japanese input and output with the current Rust host."""

import socket
import subprocess
import time
import uuid
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SERVER = ROOT / "extras/cobcics/target/debug/cobcicsd.exe"
WC3270 = Path(r"C:\Program Files\wc3270\wc3270.exe")
OPENSSL = Path(r"C:\Program Files\Git\usr\bin\openssl.exe")
OUTPUT = ROOT / "build/wc3270-japanese-host"


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def stop(*processes):
    for process in processes:
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)


def connect_script(port, process):
    for _ in range(100):
        try:
            connection = socket.create_connection(("127.0.0.1", port), timeout=1)
            connection.settimeout(10)
            return connection
        except OSError:
            if process.poll() is not None:
                raise AssertionError("wc3270 exited before opening its script port")
            time.sleep(0.1)
    raise AssertionError("wc3270 script port did not open")


def command(connection, lines, action):
    connection.sendall((action + "\n").encode("utf-8"))
    data = []
    while True:
        line = lines.readline()
        if not line:
            raise AssertionError(f"wc3270 disconnected during {action}")
        line = line.rstrip("\r\n")
        if line == "ok":
            return "\n".join(data)
        if line == "error":
            raise AssertionError(f"wc3270 rejected {action}: {data}")
        if line.startswith("data: "):
            data.append(line[6:])


def wait_screen(connection, lines, expected):
    for _ in range(100):
        screen = command(connection, lines, "Ascii")
        if expected in screen:
            return screen
        time.sleep(0.1)
    raise AssertionError(f"missing {expected!r} from wc3270 screen: {screen}")


def case(mode, cert, key):
    listen, script_port = free_port(), free_port()
    while script_port == listen:
        script_port = free_port()
    options = ["--bms", str(OUTPUT / "japanese.bms"), "--charset", "IBM-939",
               "--tn3270e", "--listen", f"127.0.0.1:{listen}"]
    if mode in ("tls", "starttls"):
        options.extend(["--tls-cert", str(cert), "--tls-key", str(key)])
    if mode == "starttls":
        options.append("--starttls")
    if mode == "nvt":
        options.append("--nvt")
    target = f"127.0.0.1:{listen}"
    if mode == "tls":
        target = "L:Y:" + target
    elif mode == "starttls":
        target = "Y:" + target
    log = OUTPUT / f"{mode}-{uuid.uuid4().hex[:8]}.log"
    trace = OUTPUT / f"{mode}-{uuid.uuid4().hex[:8]}.trace"
    with log.open("w") as output:
        host = subprocess.Popen([str(SERVER), *options], stdout=subprocess.DEVNULL,
                                stderr=output, creationflags=subprocess.CREATE_NO_WINDOW)
        client = None
        try:
            time.sleep(0.2)
            assert host.poll() is None, log.read_text(errors="replace")
            args = [str(WC3270), "+S", "-model", "3279-2", "-codepage", "cp939",
                    "-utf8", "-trace", "-tracefile", str(trace),
                    "-scriptport", f"127.0.0.1:{script_port}"]
            if mode == "nvt":
                args.append("-nvt")
            client = subprocess.Popen([*args, target], stdin=subprocess.DEVNULL,
                                      stdout=output, stderr=subprocess.STDOUT,
                                      creationflags=subprocess.CREATE_NEW_CONSOLE)
            with connect_script(script_port, client) as connection:
                lines = connection.makefile("r", encoding="utf-8", errors="strict")
                assert "cp939" in command(connection, lines, "Query(CodePage)")
                if mode == "nvt":
                    command(connection, lines, "String(3270)")
                    command(connection, lines, "Enter")
                wait_screen(connection, lines, "日本語を入力")
                command(connection, lines, "String(日本語)")
                command(connection, lines, "Enter")
                screen = wait_screen(connection, lines, "Enter:")
                assert "日本語" in screen and "日本語を入力" not in screen, screen
            assert host.poll() is None, log.read_text(errors="replace")
            print(f"wc3270 IBM-939 Japanese input and display ({mode}) OK")
        finally:
            stop(client, host)


def main():
    assert SERVER.is_file() and WC3270.is_file() and OPENSSL.is_file()
    OUTPUT.mkdir(parents=True, exist_ok=True)
    (OUTPUT / "japanese.bms").write_text(
        "JP DFHMSD TYPE=MAP\nSCREEN DFHMDI SIZE=(24,80)\n"
        "MESSAGE DFHMDF POS=(2,2),LENGTH=32,ATTRB=PROT,*\n"
        "  INITIAL='日本語を入力'\n"
        "ACTION DFHMDF POS=(4,2),LENGTH=20,ATTRB=(UNPROT,IC)\n",
        encoding="utf-8",
    )
    cert, key = OUTPUT / "cert.pem", OUTPUT / "key.pem"
    subprocess.run([str(OPENSSL), "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                    "-keyout", str(key), "-out", str(cert), "-days", "1",
                    "-subj", "/CN=localhost", "-addext", "subjectAltName=IP:127.0.0.1"],
                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for mode in ("plain", "tls", "starttls", "nvt"):
        case(mode, cert, key)


if __name__ == "__main__":
    main()
