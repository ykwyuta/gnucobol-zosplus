#!/usr/bin/env python3
"""Validate the Bank customer edit controls using the installed wc3270."""

import socket
import subprocess
import time
from pathlib import Path

WC3270 = r"C:\Program Files\wc3270\wc3270.exe"
HOST_PORT = 33327
SCRIPT_PORT = 33328


def main():
    diagnostic = Path(__file__).resolve().parents[2] / "build/bank-syntax/screens/wc3270-boundary.log"
    output = diagnostic.open("w")
    startup = subprocess.STARTUPINFO()
    startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    startup.wShowWindow = subprocess.SW_HIDE
    process = subprocess.Popen(
        [WC3270, "+S", "-model", "3279-2", "-tn", "IBM-3279-2", "-scriptport",
         f"127.0.0.1:{SCRIPT_PORT}", f"N:127.0.0.1:{HOST_PORT}"],
        stdin=subprocess.DEVNULL, stdout=output,
        stderr=subprocess.STDOUT, startupinfo=startup,
        creationflags=subprocess.CREATE_NEW_CONSOLE)
    try:
        for _ in range(100):
            try:
                connection = socket.create_connection(("127.0.0.1", SCRIPT_PORT), timeout=1)
                break
            except OSError:
                if process.poll() is not None:
                    raise AssertionError(f"wc3270 exited with {process.returncode}")
                time.sleep(0.1)
        else:
            raise AssertionError("wc3270 script port did not open")
        with connection:
            connection.settimeout(90)
            stream = connection.makefile("r", encoding="utf-8", errors="replace")

            def command(action):
                connection.sendall((action + "\n").encode("ascii"))
                data = []
                while True:
                    line = stream.readline()
                    if not line:
                        raise AssertionError(f"wc3270 disconnected during {action}")
                    line = line.rstrip("\r\n")
                    if line == "ok":
                        return "\n".join(data)
                    if line == "error":
                        raise AssertionError(f"wc3270 rejected {action}: {data}")
                    data.append(line.removeprefix("data: "))

            assert "Main Menu" in command("Ascii")
            command("String(1)")
            command("Enter")
            assert "Provide a CUSTOMER number" in command("Ascii")
            command("String(0000000001)")
            command("Enter")
            displayed = command("Ascii")
            assert "Test" in displayed and "Customer" in displayed
            command("PF(10)")
            editable = command("Ascii")
            assert "Amend data then press <ENTER>" in editable, editable
            command("MoveCursor(9,18)")
            command("EraseEOF")
            command("String(Edited)")
            typed = command("Ascii")
            assert "Edited" in typed, typed
            command("PF(3)")
            assert "Main Menu" in command("Ascii")
            print("wc3270: PF10 unlocked first name, text entered, PF3 canceled OK")

            command("MoveCursor(0,1)")
            try:
                command("String(X)")
            except AssertionError:
                pass
            assert "BNK1MA" in command("Ascii")
            command("Reset")
            print("wc3270: protected map identifier cannot be changed OK")

            command("MoveCursor(4,16)")
            try:
                command("String(12)")
            except AssertionError:
                pass
            screen = command("Ascii").splitlines()
            assert screen[4][16] in "12" and screen[4][17] == " ", screen[4]
            print("wc3270: one-character Action field rejects second character OK")
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)
        output.close()


if __name__ == "__main__":
    main()
