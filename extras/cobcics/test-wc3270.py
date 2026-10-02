#!/usr/bin/env python3
"""Check a running wc3270 session against the original BNKMENU worker."""

import argparse
from pathlib import Path
import socket
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=33271)
    parser.add_argument("--trace-file", type=Path)
    parser.add_argument("--menu-only", action="store_true")
    parser.add_argument("--account-transition", action="store_true")
    parser.add_argument("--transfer-transition", action="store_true")
    args = parser.parse_args()

    with socket.create_connection(("127.0.0.1", args.port), timeout=10) as connection:
        connection.settimeout(10)
        stream = connection.makefile("r", encoding="utf-8", errors="replace")

        def command(action):
            connection.sendall((action + "\n").encode("ascii"))
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
                lines.append(line.removeprefix("data: "))

        model = command("Query(Model)")
        assert "3279-2" in model, model
        menu = command("Ascii")
        assert "CICS Banking Sample Application - Main Menu" in menu, menu
        assert "Action . . . ." in menu, menu
        print("wc3270 menu displayed:")
        print("\n".join(line for line in menu.splitlines()
                        if "Main Menu" in line or "Action . . . ." in line))
        if args.menu_only:
            return
        if args.account_transition or args.transfer_transition:
            choice = "7" if args.transfer_transition else "2"
            title = "Transfer funds" if args.transfer_transition else "Provide an ACCOUNT number"
            screen_name = "Transfer Funds" if args.transfer_transition else "Display Account"
            command(f"String({choice})")
            command("Enter")
            next_screen = command("Ascii")
            assert title.lower() in next_screen.lower(), next_screen
            print(f"wc3270 option {choice}: {screen_name} screen")
            command("PF(3)")
            menu_again = command("Ascii")
            assert "CICS Banking Sample Application - Main Menu" in menu_again
            print("wc3270 PF3: Main Menu restored")
            return

        command("String(X)")
        command("Enter")
        invalid = command("Ascii")
        assert "You must enter a valid value (1-7 or A)." in invalid, invalid
        print("wc3270 invalid Action: You must enter a valid value (1-7 or A).")

        try:
            command("PF(3)")
        except (ConnectionResetError, BrokenPipeError):
            # wc3270 closes its script socket when the host ends the session.
            pass
        if args.trace_file:
            for _ in range(30):
                trace = args.trace_file.read_text(errors="replace")
                if "Session Ended" in trace:
                    break
                time.sleep(0.1)
            else:
                raise AssertionError("wc3270 did not receive Session Ended")
            print("wc3270 PF3: Session Ended")
        else:
            print("wc3270 PF3 sent; terminal session closed")


if __name__ == "__main__":
    main()
