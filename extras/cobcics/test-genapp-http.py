#!/usr/bin/env python3
"""Exercise original GenApp business COBOL through the loopback HTTP adapter."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
from threading import Thread
from urllib.error import HTTPError
from urllib.request import Request, urlopen

import ibm_db

from setup_genapp_connection import connection_string


spec = importlib.util.spec_from_file_location(
    "genapp_http", Path(__file__).with_name("genapp_http.py"))
api = importlib.util.module_from_spec(spec)
spec.loader.exec_module(api)


def request(base: str, path: str, data=None):
    body = None if data is None else json.dumps(data).encode("utf-8")
    headers = {} if body is None else {"Content-Type": "application/json"}
    with urlopen(Request(base + path, data=body, headers=headers), timeout=35) as response:
        return response.status, json.load(response)


def one(database, sql: str):
    return ibm_db.fetch_tuple(ibm_db.exec_immediate(database, sql))


def main() -> None:
    database = ibm_db.connect(connection_string(), "", "")
    try:
        with tempfile.TemporaryDirectory() as storage:
            os.environ["COBCICS_STORAGE_DIR"] = storage
            server = api.ThreadingHTTPServer(("127.0.0.1", 0), api.Handler)
            thread = Thread(target=server.serve_forever, daemon=True)
            thread.start()
            base = f"http://127.0.0.1:{server.server_port}"
            try:
                status, customer = request(base, "/genapp/customers/1")
                assert status == 200 and customer["first_name"] == "Andrew"
                assert customer["last_name"] == "Pandy", customer
                status, policy = request(base,
                    "/genapp/policies/1?customer=2&type=M")
                assert status == 200 and policy["make"] == "FORD", policy
                for number, customer, kind, field, expected in (
                    (4, 8, "E", "fund_name", "LIONTMR"),
                    (6, 4, "H", "property_type", "HOUSE"),
                    (9, 5, "C", "address", "TURF MOOR")):
                    status, policy = request(base,
                        f"/genapp/policies/{number}?customer={customer}&type={kind}")
                    assert status == 200 and policy[field] == expected, policy
                try:
                    request(base, "/genapp/customers/9999999999")
                except HTTPError as error:
                    assert error.code == 404, error
                else:
                    raise AssertionError("missing customer returned success")
                status, created = request(base, "/genapp/customers", {
                    "first_name": "CodexHttp", "last_name": "Fixture",
                    "date_of_birth": "1980-01-02", "house_name": "Test House",
                    "house_number": "12", "postcode": "AB12CD",
                    "phone_mobile": "0123456789", "phone_home": "0987654321",
                    "email": "http@example.com"})
                customer_number = created["customer_number"]
                assert status == 201 and customer_number > 10, created
                assert one(database, "SELECT FIRSTNAME FROM GENAPPTEST.CUSTOMER "
                    f"WHERE CUSTOMERNUMBER={customer_number}")[0].strip() == "CodexHttp"
                assert one(database, "SELECT COUNT(*) FROM GENAPPTEST.CUSTOMER_SECURE "
                    f"WHERE CUSTOMERNUMBER={customer_number}")[0] == 1
                customer_key = f"{customer_number:010d}".encode().hex()
                assert one(database, "SELECT COUNT(*) FROM GENAPPTEST.KSDSCUST "
                    f"WHERE K=BX'{customer_key}'")[0] == 1
                # Force a duplicate FILE key after CUSTOMER INSERT. The
                # HTTP worker must back out the complete Db2 work unit.
                blocked_customer = customer_number + 1
                blocked_key = f"{blocked_customer:010d}".encode()
                blocked_record = blocked_key + b" " * 215
                ibm_db.exec_immediate(database,
                    "INSERT INTO GENAPPTEST.KSDSCUST(K,V) VALUES "
                    f"(BX'{blocked_key.hex()}',BX'{blocked_record.hex()}')")
                try:
                    request(base, "/genapp/customers", {
                        "first_name": "Rollback", "last_name": "Fixture",
                        "date_of_birth": "1980-01-02", "house_name": "Test House",
                        "house_number": "12", "postcode": "AB12CD",
                        "phone_mobile": "0123456789", "phone_home": "0987654321",
                        "email": "rollback@example.com"})
                except HTTPError as error:
                    assert error.code == 502, error
                else:
                    raise AssertionError("duplicate customer FILE key succeeded")
                assert one(database, "SELECT COUNT(*) FROM GENAPPTEST.CUSTOMER "
                    f"WHERE CUSTOMERNUMBER={blocked_customer}")[0] == 0
                assert one(database, "SELECT COUNT(*) FROM GENAPPTEST.CUSTOMER_SECURE "
                    f"WHERE CUSTOMERNUMBER={blocked_customer}")[0] == 0
                assert one(database, "SELECT COUNT(*) FROM GENAPPTEST.KSDSCUST "
                    f"WHERE K=BX'{blocked_key.hex()}'")[0] == 1
                status, created = request(base, "/genapp/policies/motor", {
                    "customer_number": 2, "issue_date": "2026-10-02",
                    "expiry_date": "2027-10-01", "make": "CodexHttp",
                    "model": "TestModel", "value": 9000,
                    "registration_number": "AB12CDE", "colour": "BLUE",
                    "cc": 1600, "manufactured": "2020-01-01",
                    "premium": 500, "accidents": 0})
                policy_number = created["policy_number"]
                assert status == 201 and policy_number > 10, created
                assert one(database, "SELECT MAKE FROM GENAPPTEST.MOTOR "
                    f"WHERE POLICYNUMBER={policy_number}")[0].strip() == "CodexHttp"
                assert one(database, "SELECT COUNT(*) FROM GENAPPTEST.POLICY "
                    f"WHERE POLICYNUMBER={policy_number} AND CUSTOMERNUMBER=2")[0] == 1
                policy_key = (f"M0000000002{policy_number:010d}").encode().hex()
                assert one(database, "SELECT COUNT(*) FROM GENAPPTEST.KSDSPOLY "
                    f"WHERE K=BX'{policy_key}'")[0] == 1
                status, policy = request(base,
                    f"/genapp/policies/{policy_number}?customer=2&type=M")
                assert status == 200 and policy["make"] == "CodexHttp", policy
                other_policies = (
                    ("endowment", "E", "ENDOWMENT", "FUNDNAME", "HttpFund",
                     "fund_name", {
                         "with_profits": "Y", "equities": "N",
                         "managed_fund": "N", "fund_name": "HttpFund",
                         "term": 10, "sum_assured": 50000,
                         "life_assured": "Test Life",
                         "padding_data": "HTTP-PADDING"}),
                    ("house", "H", "HOUSE", "PROPERTYTYPE", "HttpHouse",
                     "property_type", {
                         "property_type": "HttpHouse", "bedrooms": 3,
                         "value": 300000, "house_name": "Test House",
                         "house_number": "12", "postcode": "AB12CD"}),
                    ("commercial", "C", "COMMERCIAL", "ADDRESS", "HttpAddress",
                     "address", {
                         "address": "HttpAddress", "postcode": "AB12CD",
                         "latitude": "51.00000", "longitude": "-1.00000",
                         "customer": "Test Customer", "property_type": "OFFICES",
                         "fire_peril": 1, "fire_premium": 1000,
                         "crime_peril": 2, "crime_premium": 2000,
                         "flood_peril": 3, "flood_premium": 3000,
                         "weather_peril": 4, "weather_premium": 4000,
                         "status": 1, "reject_reason": "NONE"}),
                )
                for path, kind, table, column, expected, response_field, fields in other_policies:
                    status, created = request(base, f"/genapp/policies/{path}", {
                        "customer_number": 2, "issue_date": "2026-10-02",
                        "expiry_date": "2027-10-01", **fields})
                    number = created["policy_number"]
                    assert status == 201 and number > 10, created
                    assert one(database,
                        f"SELECT {column} FROM GENAPPTEST.{table} "
                        f"WHERE POLICYNUMBER={number}")[0].strip() == expected
                    if kind == "E":
                        assert one(database,
                            "SELECT PADDINGDATA FROM GENAPPTEST.ENDOWMENT "
                            f"WHERE POLICYNUMBER={number}")[0].strip() == "HTTP-PADDING"
                    key = (f"{kind}0000000002{number:010d}").encode().hex()
                    assert one(database, "SELECT COUNT(*) FROM GENAPPTEST.KSDSPOLY "
                        f"WHERE K=BX'{key}'")[0] == 1
                    status, policy = request(base,
                        f"/genapp/policies/{number}?customer=2&type={kind}")
                    assert status == 200 and policy[response_field] == expected, policy
                before_failed_add = one(database,
                    "SELECT COUNT(*) FROM GENAPPTEST.POLICY")[0]
                try:
                    request(base, "/genapp/policies/motor", {
                        "customer_number": 9999999999,
                        "issue_date": "2026-10-02",
                        "expiry_date": "2027-10-01", "make": "NeverSaved",
                        "model": "TestModel", "value": 9000,
                        "registration_number": "AB12CDE", "colour": "BLUE",
                        "cc": 1600, "manufactured": "2020-01-01",
                        "premium": 500, "accidents": 0})
                except HTTPError as error:
                    assert error.code == 502, error
                else:
                    raise AssertionError("invalid customer policy add succeeded")
                assert one(database,
                    "SELECT COUNT(*) FROM GENAPPTEST.POLICY")[0] == before_failed_add
                print("GenApp HTTP customer/four-policy inquiry, add and rollback OK")
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)
    finally:
        ibm_db.close(database)
        for loader in ("setup-genapp-business-db.py", "setup-genapp-file-test-db.py"):
            result = subprocess.run(["python3", str(Path(__file__).with_name(loader))],
                env=os.environ, capture_output=True, text=True, timeout=60,
                check=False)
            if result.returncode:
                raise AssertionError(f"fixture restore {loader}: {result.stderr}")


if __name__ == "__main__":
    main()
