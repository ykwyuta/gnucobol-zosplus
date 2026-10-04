#!/usr/bin/env python3
"""Check BNKSTMT output against the dedicated, fixed PL1TEST fixture."""

import os
import subprocess
from pathlib import Path


def main():
    root = Path(__file__).resolve().parents[2]
    env = os.environ.copy()
    env["DB2_SCHEMA"] = "PL1TEST"
    env["DD_SORTCODE"] = str(root / "extras/pl1c/tests/card.txt")
    env["DD_DATECARD"] = str(root / "extras/pl1c/tests/datecard.txt")
    site = "/usr/local/lib64/python3.9/site-packages"
    env["LD_LIBRARY_PATH"] = f"{site}/clidriver/lib:{site}/ibm_db.libs"
    result = subprocess.run(
        [str(root / "build/pl1c/BNKSTMT")],
        cwd=root,
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode or result.stderr:
        raise AssertionError(
            f"BNKSTMT failed: rc={result.returncode}\n{result.stderr}\n{result.stdout}"
        )
    output = result.stdout
    account_one_marker = "GENERATING STATEMENT FOR ACCOUNT:  00000001"
    account_two_marker = "GENERATING STATEMENT FOR ACCOUNT:  00000002"
    assert output.count(account_one_marker) == 1
    assert output.count(account_two_marker) == 1
    first = output.split(account_one_marker, 1)[1].split(account_two_marker, 1)[0]
    second = output.split(account_two_marker, 1)[1]
    for expected in (
        "REPORTING MONTH FROM DATECARD:  202606",
        "STATEMENT PERIOD:  20260601  TO  20260630",
        "TOTAL STATEMENTS GENERATED:  2",
    ):
        assert expected in output, expected
    for expected in (
        "2026-06-05 09:15:30",
        "2026-06-10 14:30:00",
        "-50.00",
        "OPENING BALANCE:        $   1000.00",
        "TOTAL CREDITS:          $    150.00",
        "TOTAL DEBITS:           $     50.00",
        "CLOSING BALANCE:        $   1100.00",
        "TRANSACTION COUNT:          2",
    ):
        assert expected in first, expected
    assert "PL1OLD000003" not in first
    for expected in (
        "NO TRANSACTIONS FOR THIS PERIOD",
        "OPENING BALANCE:        $    500.00",
        "CLOSING BALANCE:        $    500.00",
        "TRANSACTION COUNT:          0",
    ):
        assert expected in second, expected
    assert output.count("MONTHLY ACCOUNT STATEMENT") == 2
    assert output.count("\f") == 2
    print("BNKSTMT PL1TEST report passed: 2 accounts, 2 June transactions, balances and pages")


if __name__ == "__main__":
    main()
