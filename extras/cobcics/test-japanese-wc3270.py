#!/usr/bin/env python3
"""Verify IBM-939 Japanese display and input through a running wc3270."""

import argparse
import socket


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--script-port", type=int, default=33280)
    args = parser.parse_args()

    with socket.create_connection(("127.0.0.1", args.script_port), timeout=10) as connection:
        connection.settimeout(10)
        stream = connection.makefile("r", encoding="utf-8", errors="strict")

        def command(action: str) -> str:
            connection.sendall((action + "\n").encode("utf-8"))
            lines = []
            while True:
                line = stream.readline()
                if not line:
                    raise AssertionError(f"wc3270 closed during {action}")
                line = line.rstrip("\r\n")
                if line == "ok":
                    return "\n".join(lines)
                if line == "error":
                    raise AssertionError(f"wc3270 rejected {action}: {lines}")
                if line.startswith("data: "):
                    lines.append(line[6:])

        codepage = command("Query(CodePage)")
        assert "cp939" in codepage, codepage
        initial = command("Ascii")
        assert "日本語を入力" in initial, initial
        command("String(日本語)")
        command("Enter")
        echoed = command("Ascii")
        assert "日本語" in echoed, echoed
        assert "日本語を入力" not in echoed, echoed
        print("wc3270 cp939 Japanese display, input and COBOL echo OK")


if __name__ == "__main__":
    main()
