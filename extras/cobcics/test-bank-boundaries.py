#!/usr/bin/env python3
"""Exercise Bank of Z screen boundary cases against Docker Db2 and TN3270."""

from pathlib import Path
import re
import socket
import subprocess
import time

import ibm_db
import importlib.util

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("screens", HERE / "test-bank-screens.py")
screens = importlib.util.module_from_spec(spec)
spec.loader.exec_module(screens)
ROOT = screens.ROOT
terminal = screens.terminal


def connect(port, choice=None):
    connection = socket.create_connection(("127.0.0.1", port), timeout=10)
    terminal.negotiate(connection)
    connection.settimeout(20)
    if choice is not None:
        screens.action(connection, choice)
    return connection


def customer_name():
    db = screens.db_connection()
    try:
        return ibm_db.fetch_tuple(ibm_db.exec_immediate(db,
            "SELECT CUSTOMER_FIRST_NAME,CUSTOMER_LAST_NAME,CUSTOMER_ADDR_LINE1 "
            "FROM BANKZTEST.CUSTOMER WHERE CUSTOMER_SORTCODE='987654' "
            "AND CUSTOMER_NUMBER='0000000001'"))
    finally:
        ibm_db.close(db)


def set_customer_name(first, last, address):
    db = screens.db_connection()
    try:
        for value in (first, last, address):
            assert "'" not in value
        ibm_db.exec_immediate(db,
            "UPDATE BANKZTEST.CUSTOMER SET "
            f"CUSTOMER_FIRST_NAME='{first}',CUSTOMER_LAST_NAME='{last}',"
            f"CUSTOMER_ADDR_LINE1='{address}' WHERE "
            "CUSTOMER_SORTCODE='987654' AND CUSTOMER_NUMBER='0000000001'")
    finally:
        ibm_db.close(db)


def extra_accounts(add):
    db = screens.db_connection()
    try:
        for number in range(3, 12):
            account = f"{number:08d}"
            if add:
                ibm_db.exec_immediate(db,
                    "INSERT INTO BANKZTEST.ACCOUNT SELECT ACCOUNT_EYECATCHER,"
                    "ACCOUNT_CUSTOMER_NUMBER,ACCOUNT_SORTCODE,"
                    f"'{account}',ACCOUNT_TYPE,ACCOUNT_INTEREST_RATE,"
                    "ACCOUNT_OPENED,ACCOUNT_OVERDRAFT_LIMIT,ACCOUNT_LAST_STATEMENT,"
                    "ACCOUNT_NEXT_STATEMENT,ACCOUNT_AVAILABLE_BALANCE,"
                    "ACCOUNT_ACTUAL_BALANCE FROM BANKZTEST.ACCOUNT WHERE "
                    "ACCOUNT_SORTCODE='987654' AND ACCOUNT_NUMBER='00000001'")
            else:
                ibm_db.exec_immediate(db,
                    "DELETE FROM BANKZTEST.ACCOUNT WHERE ACCOUNT_SORTCODE='987654' "
                    f"AND ACCOUNT_NUMBER='{account}'")
    finally:
        ibm_db.close(db)


def customer_account_count():
    db = screens.db_connection()
    try:
        return ibm_db.fetch_tuple(ibm_db.exec_immediate(db,
            "SELECT COUNT(*) FROM BANKZTEST.ACCOUNT WHERE "
            "ACCOUNT_SORTCODE='987654' AND "
            "ACCOUNT_CUSTOMER_NUMBER='0000000001'"))[0]
    finally:
        ibm_db.close(db)


def main():
    bms = ROOT / "reference/Bank-of-Z-main/src/base/cics/bms"
    workers = ROOT / "build/bank-syntax/screens"
    args = [str(ROOT / "build/bank-business/cobcicsd"),
            "--bms", str(bms / "BNK1MAI.bms"), "--transid", "OMEN",
            "--applid", "BANKAPP", "--route", f"OMEN={workers / 'BNKMENU'}"]
    for _, transid, mapset, _, worker in screens.ROUTES:
        args += ["--route", f"{transid}={workers / worker}",
                 "--map-route", f"{transid}={bms / (mapset + '.bms')}"]
    args += ["--listen", "127.0.0.1:0"]
    log_path = workers / "boundary-server.log"
    with log_path.open("w+") as log:
        server = subprocess.Popen(args, stderr=log)
        try:
            for _ in range(100):
                log.flush()
                log.seek(0)
                match = re.search(r"listening at 127\.0\.0\.1:(\d+)", log.read())
                if match:
                    break
                if server.poll() is not None:
                    raise AssertionError("server exited during startup")
                time.sleep(0.05)
            else:
                raise AssertionError("server did not announce a port")
            port = int(match.group(1))

            # Clear is a short-read AID; every worker erases and ends its task.
            for choice, transid, _, _, _ in screens.ROUTES:
                with connect(port, choice) as connection:
                    connection.sendall(b"\x6d\xff\xef")
                    cleared = terminal.read_record(connection)
                    assert cleared[:2] == b"\xf5\xc2", (transid, cleared[:10])
                print(f"R02-Clear: {transid} erased OK")

            # Cancel means leaving the display before PF5. Bank COBOL deletes
            # immediately on PF5 and does not present a confirmation page.
            before_account = screens.fixture_snapshot()
            with connect(port, "2") as connection:
                screens.enter_fields(connection, [(5, 18, "00000001")])
                menu = screens.aid(connection, 0xf3)
                assert "Main Menu" in menu.decode("cp037")
            assert screens.fixture_snapshot() == before_account
            print("A04: PF3 before PF5 leaves account and balance unchanged OK")

            before_customer = customer_name()
            try:
                with connect(port, "1") as connection:
                    screens.enter_fields(connection, [(5, 18, "0000000001")])
                    edit = screens.enter_fields(connection,
                        [(5, 18, "0000000001"), (7, 19, "987654"),
                         (8, 19, "0000000001")], 0x7a)
                    assert "Amend data then press" in edit.decode("cp037"), edit.decode("cp037", errors="replace")[-300:]
                    assert b"\x11" + screens.address(10, 18) + b"\x1d\x41" in edit
                    menu = screens.aid(connection, 0xf3)
                    assert "Main Menu" in menu.decode("cp037")
                assert customer_name() == before_customer
                print("D04-cancel: PF10 unlock, PF3 cancel, Db2 unchanged OK")

                with connect(port, "1") as connection:
                    screens.enter_fields(connection, [(5, 18, "0000000001")])
                    edit = screens.enter_fields(connection,
                        [(5, 18, "0000000001"), (7, 19, "987654"),
                         (8, 19, "0000000001")], 0x7a)
                    assert "Amend data then press" in edit.decode("cp037")
                    updated = screens.enter_fields(connection, [
                        (7, 19, "987654"), (8, 19, "0000000001"),
                        (9, 19, "Mr        "),
                        (10, 19, "Edited".ljust(50)),
                        (11, 19, "Customer".ljust(50)),
                        (12, 19, "1 Test Street".ljust(50)),
                        (13, 19, ""), (14, 19, "London".ljust(50)),
                        (15, 19, "SW1 1AA   "),
                        (16, 19, "United Kingdom".ljust(50)),
                        (17, 19, "02"), (17, 24, "01"), (17, 29, "1980"),
                        (18, 19, "700"),
                        (19, 19, "01"), (19, 24, "01"), (19, 29, "2027")])
                    assert "updated" in updated.decode("cp037", errors="replace").lower(), updated.decode("cp037", errors="replace")[-300:]
                after_customer = customer_name()
                assert after_customer[0].strip() == "Edited", after_customer
                assert after_customer[1:] == before_customer[1:], after_customer
                print("D04-update: only first name changed in Db2 OK")
            finally:
                set_customer_name(*(str(item).strip() for item in before_customer))
                assert customer_name() == before_customer

            try:
                extra_accounts(True)
                with connect(port, "A") as connection:
                    result = screens.enter_fields(connection,
                        [(5, 18, "0000000001")]).decode("cp037", errors="replace")
                    assert "00000010" in result and "00000011" not in result, result[-900:]
                    assert "11 accounts found" in result, result[-300:]
                print("L02: 11 Db2 rows, first 10 displayed, no 11th row OK")
            finally:
                extra_accounts(False)
                assert customer_account_count() == 2, "L02 fixture restore failed"

            # Deliberately malformed 3270 Read Modified records must be rejected.
            for label, fields in [
                ("protected", [(1, 2, "X")]),
                ("overlength", [(5, 17, "12")]),
            ]:
                with connect(port) as connection:
                    try:
                        screens.enter_fields(connection, fields)
                    except (AssertionError, OSError):
                        pass
                    else:
                        raise AssertionError(f"BMS {label} input was accepted")
                print(f"BMS-{label}: malformed Read Modified rejected OK")
        finally:
            server.terminate()
            server.wait(timeout=5)


if __name__ == "__main__":
    main()
