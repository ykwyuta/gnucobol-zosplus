#!/usr/bin/env python3
"""Verify that wc3270 decodes model 2-5 BMS screens in TN3270 and TN3270E."""

import socket
import subprocess
import time
import uuid
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SERVER = ROOT / "extras/cobcics/target/debug/cobcicsd.exe"
CLIENT = Path(r"C:\Program Files\wc3270\wc3270.exe")
OUTPUT = ROOT / "build/wc3270-models"


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def case(model, rows, columns, enhanced, oversize=None):
    mode = "e" if enhanced else "n"
    source = OUTPUT / f"model{model}{'-oversize' if oversize else ''}.bms"
    source.write_text(
        "TEST DFHMSD TYPE=MAP\n"
        f"SCREEN DFHMDI SIZE=({rows},{columns})\n"
        "TITLE DFHMDF POS=(1,1),LENGTH=18,ATTRB=PROT,COLOR=RED,*\n"
        "  HILIGHT=UNDERLINE,INITIAL='WC3270 MODEL TEST'\n"
        f"LAST DFHMDF POS=({rows},1),LENGTH=12,ATTRB=UNPROT,*\n"
        "  COLOR=GREEN,INITIAL='BOTTOM FIELD'\n",
        encoding="utf-8",
    )
    trace = OUTPUT / f"model{model}-{mode}{'-oversize' if oversize else ''}-{uuid.uuid4().hex[:8]}.trace"
    log = OUTPUT / f"model{model}-{mode}.log"
    port = free_port()
    with log.open("w") as output:
        host = subprocess.Popen(
            [str(SERVER), "--bms", str(source), "--listen", f"127.0.0.1:{port}"]
            + (["--tn3270e"] if enhanced else []),
            stdout=subprocess.DEVNULL, stderr=output,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
        client = None
        try:
            time.sleep(0.2)
            assert host.poll() is None, f"server exited: {log.read_text(errors='replace')}"
            target = f"127.0.0.1:{port}" if enhanced else f"N:127.0.0.1:{port}"
            options = [str(CLIENT), "+S", "-trace", "-tracefile", str(trace),
                       "-model", f"3279-{model}"]
            if oversize:
                options.extend(["-oversize", oversize])
            client = subprocess.Popen(
                [*options, target],
                stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT,
                creationflags=subprocess.CREATE_NEW_CONSOLE,
            )
            for _ in range(100):
                trace_text = trace.read_text(errors="replace") if trace.exists() else ""
                if "'BOTTOM FIELD'" in trace_text and "foreground(red)" in trace_text:
                    break
                if client.poll() is not None:
                    raise AssertionError(f"wc3270 exited: {log.read_text(errors='replace')}")
                time.sleep(0.1)
            else:
                raise AssertionError(f"wc3270 did not decode screen: {trace}\n{trace_text[-3000:]}")
            assert f"{rows} rows x {columns} cols" in trace_text, trace_text[:700]
            if enhanced:
                assert "TN3270E option negotiation complete" in trace_text, trace_text[-3000:]
                assert "RCVD TN3270E(3270-DATA" in trace_text, trace_text[-3000:]
            else:
                assert "TN3270E option negotiation complete" not in trace_text
                assert "connected-3270" in trace_text or "connected-tn3270" in trace_text
            command = "EraseWrite" if model == 2 else "EraseWriteAlternate"
            assert command in trace_text and "foreground(red)" in trace_text
            assert "highlighting(underscore)" in trace_text
            assert "round(green)" in trace_text
            print(f"wc3270 model {model} {'TN3270E' if enhanced else 'TN3270'}: "
                  f"{rows}x{columns}, color, underline OK")
        finally:
            for process in (client, host):
                if process is None:
                    continue
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)


def main():
    assert SERVER.is_file(), f"build the server first: {SERVER}"
    assert CLIENT.is_file(), f"wc3270 not installed: {CLIENT}"
    OUTPUT.mkdir(parents=True, exist_ok=True)
    for model, rows, columns in [(2, 24, 80), (3, 32, 80),
                                 (4, 43, 80), (5, 27, 132)]:
        case(model, rows, columns, True)
    case(2, 24, 80, False)
    case(2, 50, 100, True, oversize="100x50")


if __name__ == "__main__":
    main()
