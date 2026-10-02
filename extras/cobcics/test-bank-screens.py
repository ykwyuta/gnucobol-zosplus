#!/usr/bin/env python3
"""Run Bank of Z menu-to-screen and PF3 cases against the linked workers."""

import importlib.util
from pathlib import Path
import re
import socket
import subprocess
import os
from decimal import Decimal

import ibm_db


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "cobcics_e2e", Path(__file__).with_name("test-e2e.py"))
terminal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(terminal)

ROUTES = [
    ("1", "ODCS", "BNK1DCM", "BNK1DC", "BNKLINK"),
    ("2", "ODAC", "BNK1DAM", "BNK1DA", "BNK1DAC"),
    ("3", "OCCS", "BNK1CCM", "BNK1CC", "BNK1CCS"),
    ("4", "OCAC", "BNK1CAM", "BNK1CA", "BNK1CAC"),
    ("5", "OUAC", "BNK1UAM", "BNK1UA", "BNK1UAC"),
    ("6", "OCRA", "BNK1CDM", "BNK1CD", "BNKLINK"),
    ("7", "OTFN", "BNK1TFM", "BNK1TF", "BNK1TFN"),
    ("A", "OCCA", "BNK1ACC", "BNK1ACC", "BNK1CCA"),
]


def aid(connection, value):
    connection.sendall(bytes([value, 0x40, 0x40, 0xff, 0xef]))
    return terminal.read_record(connection)


def action(connection, choice):
    address = bytes([0xc5, 0x50])
    connection.sendall(b"\x7d" + address + b"\x11" + address +
                       choice.encode("cp037") + b"\xff\xef")
    return terminal.read_record(connection)


ADDRESS_CODE = bytes.fromhex(
    "40 c1 c2 c3 c4 c5 c6 c7 c8 c9 4a 4b 4c 4d 4e 4f "
    "50 d1 d2 d3 d4 d5 d6 d7 d8 d9 5a 5b 5c 5d 5e 5f "
    "60 61 62 63 64 65 66 67 68 69 6a 6b 6c 6d 6e 6f "
    "f0 f1 f2 f3 f4 f5 f6 f7 f8 f9 7a 7b 7c 7d 7e 7f")


def address(row, column):
    offset = (row - 1) * 80 + column - 1
    return bytes([ADDRESS_CODE[offset >> 6], ADDRESS_CODE[offset & 63]])


def enter_fields(connection, fields, aid_value=0x7d):
    record = bytearray([aid_value, 0x40, 0x40])
    for row, column, value in fields:
        record += b"\x11" + address(row, column) + value.encode("cp037")
    connection.sendall(record + b"\xff\xef")
    return terminal.read_record(connection)


def db_connection():
    dsn = (f"DATABASE={os.environ['DB2_DATABASE']};"
           f"HOSTNAME={os.environ['DB2_HOST']};PORT={os.environ['DB2_PORT']};"
           f"PROTOCOL=TCPIP;UID={os.environ['DB2_USER']};"
           f"PWD={os.environ['DB2_PASSWORD']};")
    return ibm_db.connect(dsn, "", "")


def fixture_snapshot():
    connection = db_connection()
    try:
        statement = ibm_db.exec_immediate(connection,
            "SELECT ACCOUNT_NUMBER,ACCOUNT_AVAILABLE_BALANCE,ACCOUNT_ACTUAL_BALANCE,"
            "ACCOUNT_TYPE,ACCOUNT_INTEREST_RATE,ACCOUNT_OVERDRAFT_LIMIT "
            "FROM BANKZTEST.ACCOUNT WHERE ACCOUNT_SORTCODE='987654' "
            "AND ACCOUNT_NUMBER IN ('00000001','00000002') ORDER BY ACCOUNT_NUMBER")
        rows = []
        while (row := ibm_db.fetch_tuple(statement)):
            rows.append(tuple(str(value).strip() for value in row))
        count = ibm_db.fetch_tuple(ibm_db.exec_immediate(connection,
            "SELECT COUNT(*) FROM BANKZTEST.PROCTRAN WHERE PROCTRAN_SORTCODE='987654'"))[0]
        return rows, count
    finally:
        ibm_db.close(connection)


def restore_fixture(before):
    """Undo only writes made against the disposable BANKZTEST fixture."""
    rows, count = before
    if count != 0:
        raise AssertionError("write tests require an empty BANKZTEST.PROCTRAN")
    connection = db_connection()
    try:
        for number, available, actual, account_type, rate, overdraft in rows:
            ibm_db.exec_immediate(connection,
                "UPDATE BANKZTEST.ACCOUNT SET "
                f"ACCOUNT_AVAILABLE_BALANCE={available}, "
                f"ACCOUNT_ACTUAL_BALANCE={actual}, "
                f"ACCOUNT_TYPE='{account_type}', "
                f"ACCOUNT_INTEREST_RATE={rate}, "
                f"ACCOUNT_OVERDRAFT_LIMIT={overdraft} "
                "WHERE ACCOUNT_SORTCODE='987654' "
                f"AND ACCOUNT_NUMBER='{number}'")
        ibm_db.exec_immediate(connection,
            "DELETE FROM BANKZTEST.PROCTRAN WHERE PROCTRAN_SORTCODE='987654' "
            "AND PROCTRAN_NUMBER IN ('00000001','00000002')")
    finally:
        ibm_db.close(connection)


def account_creation_state():
    connection = db_connection()
    try:
        account_count = ibm_db.fetch_tuple(ibm_db.exec_immediate(connection,
            "SELECT COUNT(*) FROM BANKZTEST.ACCOUNT "
            "WHERE ACCOUNT_SORTCODE='987654' AND ACCOUNT_NUMBER='00000003'"))[0]
        control = ibm_db.fetch_tuple(ibm_db.exec_immediate(connection,
            "SELECT CONTROL_VALUE_NUM FROM BANKZTEST.CONTROL "
            "WHERE CONTROL_NAME='987654-ACCOUNT-LAST'"))[0]
        return account_count, control
    finally:
        ibm_db.close(connection)


def restore_account_creation(control):
    connection = db_connection()
    try:
        ibm_db.exec_immediate(connection,
            "DELETE FROM BANKZTEST.ACCOUNT WHERE ACCOUNT_SORTCODE='987654' "
            "AND ACCOUNT_NUMBER='00000003'")
        ibm_db.exec_immediate(connection,
            "DELETE FROM BANKZTEST.PROCTRAN WHERE PROCTRAN_SORTCODE='987654' "
            "AND PROCTRAN_NUMBER='00000003'")
        ibm_db.exec_immediate(connection,
            "UPDATE BANKZTEST.CONTROL SET CONTROL_VALUE_NUM=" + str(control) +
            " WHERE CONTROL_NAME='987654-ACCOUNT-LAST'")
    finally:
        ibm_db.close(connection)


def customer_creation_state():
    connection = db_connection()
    try:
        count = ibm_db.fetch_tuple(ibm_db.exec_immediate(connection,
            "SELECT COUNT(*) FROM BANKZTEST.CUSTOMER "
            "WHERE CUSTOMER_SORTCODE='987654' AND CUSTOMER_NUMBER='0000000002'"))[0]
        control = ibm_db.fetch_tuple(ibm_db.exec_immediate(connection,
            "SELECT CONTROL_VALUE_NUM FROM BANKZTEST.CONTROL "
            "WHERE CONTROL_NAME='BANKZCUST987654'"))[0]
        return count, control
    finally:
        ibm_db.close(connection)


def customer_credit():
    connection = db_connection()
    try:
        return ibm_db.fetch_tuple(ibm_db.exec_immediate(connection,
            "SELECT CUSTOMER_CREDIT_SCORE,CUSTOMER_CS_REVIEW_DATE "
            "FROM BANKZTEST.CUSTOMER WHERE CUSTOMER_SORTCODE='987654' "
            "AND CUSTOMER_NUMBER='0000000002'"))
    finally:
        ibm_db.close(connection)


def restore_customer_creation(control):
    connection = db_connection()
    try:
        ibm_db.exec_immediate(connection,
            "DELETE FROM BANKZTEST.CUSTOMER WHERE CUSTOMER_SORTCODE='987654' "
            "AND CUSTOMER_NUMBER='0000000002'")
        ibm_db.exec_immediate(connection,
            "DELETE FROM BANKZTEST.PROCTRAN WHERE PROCTRAN_SORTCODE='987654' "
            "AND PROCTRAN_NUMBER='00000000'")
        ibm_db.exec_immediate(connection,
            "UPDATE BANKZTEST.CONTROL SET CONTROL_VALUE_NUM=" + str(control) +
            " WHERE CONTROL_NAME='BANKZCUST987654'")
    finally:
        ibm_db.close(connection)


def main():
    screens = ROOT / "build/bank-syntax/screens"
    bms = ROOT / "reference/Bank-of-Z-main/src/base/cics/bms"
    args = [str(ROOT / "build/bank-business/cobcicsd"),
            "--bms", str(bms / "BNK1MAI.bms"), "--transid", "OMEN",
            "--applid", "BANKAPP", "--route", f"OMEN={screens / 'BNKMENU'}"]
    for _, transid, mapset, _, worker in ROUTES:
        args += ["--route", f"{transid}={screens / worker}",
                 "--map-route", f"{transid}={bms / (mapset + '.bms')}"]
    args += ["--listen", "127.0.0.1:0"]
    log_path = ROOT / "build/bank-syntax/screens/server.log"
    with log_path.open("w+") as log:
        server = subprocess.Popen(args, stderr=log)
        try:
            import time
            for _ in range(100):
                log.flush()
                log.seek(0)
                match = re.search(r"listening at 127\.0\.0\.1:(\d+)", log.read())
                if match:
                    break
                if server.poll() is not None:
                    raise AssertionError("cobcicsd exited during startup")
                time.sleep(0.05)
            else:
                raise AssertionError("cobcicsd did not announce its port")
            port = int(match.group(1))
            for choice, transid, _, map_name, _ in ROUTES:
                with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                    connection.settimeout(15)
                    menu = terminal.negotiate(connection)
                    assert "Main Menu".encode("cp037") in menu, choice
                    screen = action(connection, choice)
                    assert map_name.encode("cp037") in screen, (
                        f"{choice} {transid}: {screen.decode('cp037', errors='replace')[:200]}")
                    returned = aid(connection, 0xf3)
                    assert "Main Menu".encode("cp037") in returned, (
                        f"{choice} {transid} PF3: {returned.decode('cp037', errors='replace')[:200]}")
                    print(f"M01/R01: {choice} -> {transid} -> OMEN OK")
            for choice, transid, _, _, _ in ROUTES:
                with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                    connection.settimeout(15)
                    terminal.negotiate(connection)
                    action(connection, choice)
                    ended = aid(connection, 0x7c)
                    assert "Session Ended".encode("cp037") in ended, (
                        f"R02: {transid} PF12 did not end the session")
                    print(f"R02: {transid} PF12 session end OK")
            with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                connection.settimeout(15)
                terminal.negotiate(connection)
                invalid = action(connection, "X")
                assert "You must enter a valid value (1-7 or A).".encode("cp037") in invalid
                print("M03: invalid menu Action OK")
                ended = aid(connection, 0xf3)
                assert "Session Ended".encode("cp037") in ended
                print("M04: PF3 session end OK")
            before = fixture_snapshot()
            failures = []
            for choice, case, inputs, expected in [
                (None, "M02", None, "valid value"),
                ("3", "C01", [(7, 19, " ")], "valid titles"),
                ("4", "N01-invalid-type", [(6, 24, "0000000001"),
                                            (7, 24, "INVALID")], "Account Type"),
                ("6", "B02", [(8, 18, "00000001"),
                                (8, 36, "+"), (8, 38, "ABCDEFGHIJKL")], "numeric"),
                ("7", "T02-invalid-amount", [(8, 24, "00000001"),
                                               (8, 53, "00000002"),
                                               (9, 24, "ABCDEFGHIJKL")], "numeric"),
                ("A", "L02", [(5, 18, "0000000002")], "Unable to find customer"),
            ]:
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                        connection.settimeout(15)
                        terminal.negotiate(connection)
                        if choice is None:
                            result = action(connection, "")
                        else:
                            action(connection, choice)
                            result = enter_fields(connection, inputs)
                        decoded = result.decode("cp037", errors="replace")
                        if expected.lower() not in decoded.lower():
                            failures.append(f"{case}: missing {expected}; {decoded[:400]}")
                            print(f"{case}: FAIL ({expected} missing)")
                        else:
                            print(f"{case}: validation/display OK")
                except (AssertionError, OSError) as error:
                    failures.append(f"{case}: {error}")
                    print(f"{case}: FAIL ({error})")
            for choice, case, inputs, expected in [
                ("1", "D01", [(5, 18, "0000000001")], ["Test", "Customer"]),
                ("2", "A01", [(5, 18, "00000001")], ["CURRENT", "1000"]),
                ("A", "L01", [(5, 18, "0000000001")], ["00000001", "00000002"]),
            ]:
                with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                    connection.settimeout(15)
                    terminal.negotiate(connection)
                    action(connection, choice)
                    result = enter_fields(connection, inputs)
                    for fragment in expected:
                        assert fragment.encode("cp037") in result, (
                            f"{case}: missing {fragment}; {result.decode('cp037', errors='replace')[:400]}")
                    print(f"{case}: Db2-backed screen fields OK")
            for choice, case, inputs, expected in [
                ("1", "D02", [(5, 18, "0000000002")], "not found"),
                ("2", "A02", [(5, 18, "00000003")], "not found"),
                ("5", "U01", [(5, 18, "00000001")], "00000001"),
                ("7", "T02", [(8, 24, "00000001"),
                                (8, 53, "00000001"),
                                (9, 24, "0000000010.00")], "different"),
            ]:
                with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                    connection.settimeout(15)
                    terminal.negotiate(connection)
                    action(connection, choice)
                    result = enter_fields(connection, inputs)
                    decoded = result.decode("cp037", errors="replace")
                    if expected.lower() not in decoded.lower():
                        failures.append(f"{case}: missing {expected}; {decoded[:400]}")
                        print(f"{case}: FAIL ({expected} missing)")
                    else:
                        print(f"{case}: validation/display OK")
            assert fixture_snapshot() == before, "read-only screens changed Db2 fixture"
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                    connection.settimeout(15)
                    terminal.negotiate(connection)
                    action(connection, "5")
                    enter_fields(connection, [(5, 18, "00000001")])
                    result = enter_fields(connection,
                        [(5, 18, "00000001"), (10, 21, "CURRENT "),
                         (11, 21, "1.50"), (13, 21, "200"),
                         (12, 21, "01"), (12, 26, "01"), (12, 31, "2024"),
                         (14, 21, "01"), (14, 26, "07"), (14, 31, "2024"),
                         (15, 21, "01"), (15, 26, "08"), (15, 31, "2024")], 0xf5)
                after_rows, history = fixture_snapshot()
                if (Decimal(after_rows[0][5]) != Decimal("200") or
                    "successfully applied" not in result.decode("cp037", errors="replace")):
                    failures.append("U02: update did not change overdraft; "
                                    f"row={after_rows[0]}, screen="
                                    f"{result.decode('cp037', errors='replace')[-280:]}")
                    print("U02: FAIL (overdraft unchanged)")
                else:
                    print("U02: overdraft=200, confirmation displayed OK")
            except (AssertionError, OSError) as error:
                failures.append(f"U02: {error}")
                print(f"U02: FAIL ({error})")
            finally:
                restore_fixture(before)
                assert fixture_snapshot() == before, "U02: fixture restore failed"
            customer_count, customer_counter = customer_creation_state()
            assert customer_count == 0 and customer_counter == 1, (
                "C02 requires the untouched customer counter and number 0000000002")
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                    terminal.negotiate(connection)
                    connection.settimeout(90)
                    action(connection, "3")
                    created = enter_fields(connection, [
                        (7, 19, "Mr        "), (7, 47, "Alice"),
                        (8, 19, "A"), (8, 36, "Banktest"),
                        (10, 19, "2 Test Street"), (11, 19, ""),
                        (12, 19, "London"), (13, 19, "SW1 1AA"),
                        (14, 19, "United Kingdom"),
                        (15, 19, "01"), (15, 24, "01"), (15, 29, "1990")])
                count_after, counter_after = customer_creation_state()
                if (count_after != 1 or counter_after != 2 or
                    "successfully created" not in created.decode("cp037", errors="replace")):
                    failures.append("C02: creation failed; state="
                        f"{(count_after, counter_after)}, screen="
                        f"{created.decode('cp037', errors='replace')[-280:]}")
                    print("C02: FAIL (customer not created)")
                else:
                    print("C02: customer 0000000002 created, counter=2 OK")
                    score, review = customer_credit()
                    if (not 1 <= score <= 999 or
                        str(score).encode("cp037") not in created):
                        failures.append(f"C03: invalid credit score {score}, review={review}")
                        print("C03: FAIL (credit score mismatch)")
                    else:
                        print(f"C03: credit score={score}, review={review} OK")
                    with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                        connection.settimeout(15)
                        terminal.negotiate(connection)
                        action(connection, "1")
                        enter_fields(connection, [(5, 18, "0000000002")])
                        deleted = enter_fields(connection,
                            [(5, 18, "0000000002"),
                             (7, 19, "987654"),
                             (8, 19, "0000000002")], 0xf5)
                    remaining, _ = customer_creation_state()
                    if (remaining != 0 or
                        "deleted" not in deleted.decode("cp037", errors="replace").lower()):
                        failures.append("D03: customer delete failed; "
                            f"remaining={remaining}, screen="
                            f"{deleted.decode('cp037', errors='replace')[-280:]}")
                        print("D03: FAIL (customer remains)")
                    else:
                        print("D03: customer 0000000002 deleted OK")
            except (AssertionError, OSError) as error:
                failures.append(f"C02: {error}")
                print(f"C02: FAIL ({error})")
            finally:
                restore_customer_creation(customer_counter)
                assert customer_creation_state() == (0, customer_counter), (
                    "C02: restore failed")
            account_count, counter = account_creation_state()
            assert account_count == 0 and counter == 2, (
                "N01/A03 require the untouched account counter and number 00000003")
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                    connection.settimeout(15)
                    terminal.negotiate(connection)
                    action(connection, "4")
                    created = enter_fields(connection,
                        [(6, 24, "0000000001"), (7, 24, "SAVING  "),
                         (8, 24, "1.00"), (9, 24, "0")])
                count_after, counter_after = account_creation_state()
                if (count_after != 1 or counter_after != 3 or
                    "successfully created" not in created.decode("cp037", errors="replace")):
                    failures.append("N01: creation failed; state="
                        f"{(count_after, counter_after)}, screen="
                        f"{created.decode('cp037', errors='replace')[-280:]}")
                    print("N01: FAIL (account not created)")
                else:
                    print("N01: account 00000003 created, counter=3 OK")
                    with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                        connection.settimeout(15)
                        terminal.negotiate(connection)
                        action(connection, "2")
                        enter_fields(connection, [(5, 18, "00000003")])
                        deleted = enter_fields(connection,
                            [(5, 18, "00000003"),
                             (9, 21, "00000003  ")], 0xf5)
                    count_deleted, _ = account_creation_state()
                    if count_deleted != 0 or "successfully deleted" not in deleted.decode("cp037", errors="replace"):
                        failures.append("A03: deletion failed; "
                            f"count={count_deleted}, screen="
                            f"{deleted.decode('cp037', errors='replace')[-280:]}")
                        print("A03: FAIL (account remains)")
                    else:
                        print("A03: account 00000003 deleted OK")
            except (AssertionError, OSError) as error:
                failures.append(f"N01/A03: {error}")
                print(f"N01/A03: FAIL ({error})")
            finally:
                restore_account_creation(counter)
                assert account_creation_state() == (0, counter), "N01/A03: restore failed"
            for choice, case, inputs, expected_amounts, min_history in [
                ("6", "B01", [(8, 18, "00000001"), (8, 36, "+"),
                                (8, 38, "0000000010.00")],
                 (Decimal("1010.00"), Decimal("500.00")), 1),
                ("6", "B01-debit", [(8, 18, "00000001"), (8, 36, "-"),
                                      (8, 38, "0000000010.00")],
                 (Decimal("990.00"), Decimal("500.00")), 1),
                ("6", "B03-teller-overdraft", [(8, 18, "00000001"),
                                                  (8, 36, "-"),
                                                  (8, 38, "0000002000.00")],
                 (Decimal("-1000.00"), Decimal("500.00")), 1),
                ("7", "T01", [(8, 24, "00000001"), (8, 53, "00000002"),
                                (9, 24, "0000000010.00")],
                 (Decimal("990.00"), Decimal("510.00")), 2),
                ("7", "T03-overdraft", [(8, 24, "00000001"),
                                           (8, 53, "00000002"),
                                           (9, 24, "0000002000.00")],
                 (Decimal("-1000.00"), Decimal("2500.00")), 2),
            ]:
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=10) as connection:
                        connection.settimeout(15)
                        terminal.negotiate(connection)
                        action(connection, choice)
                        result = enter_fields(connection, inputs)
                    after_rows, history = fixture_snapshot()
                    actual_amounts = tuple(Decimal(row[1]) for row in after_rows)
                    if actual_amounts != expected_amounts or history < min_history:
                        decoded = result.decode("cp037", errors="replace")
                        failures.append(f"{case}: balances={actual_amounts}, "
                                        f"history={history}; screen={decoded[-280:]}")
                        print(f"{case}: FAIL (balance or history mismatch)")
                    else:
                        print(f"{case}: balances={actual_amounts}, history={history} OK")
                except (AssertionError, OSError) as error:
                    failures.append(f"{case}: {error}")
                    print(f"{case}: FAIL ({error})")
                finally:
                    restore_fixture(before)
                    assert fixture_snapshot() == before, f"{case}: fixture restore failed"
            for failure in failures:
                print(failure)
            assert not failures, f"{len(failures)} screen scenarios failed"
        finally:
            server.terminate()
            server.wait(timeout=5)


if __name__ == "__main__":
    main()
