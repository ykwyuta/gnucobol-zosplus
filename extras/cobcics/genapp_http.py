#!/usr/bin/env python3
"""Loopback HTTP adapter for GenApp's original COMMAREA business programs."""

import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
from urllib.parse import parse_qs, urlsplit


ROOT = Path(__file__).resolve().parents[2]
WORKER = ROOT / "build/genapp/GENAPP-API"
AREA_SIZE = 32500


class BadInput(ValueError):
    pass


def number(value, width: int) -> bytes:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise BadInput("expected a non-negative integer")
    digits = str(value)
    if len(digits) > width:
        raise BadInput("number exceeds COBOL field width")
    return digits.zfill(width).encode("ascii")


def fixed(value, width: int) -> bytes:
    if not isinstance(value, str):
        raise BadInput("expected a text field")
    try:
        data = value.encode("ascii")
    except UnicodeEncodeError as error:
        raise BadInput("this COMMAREA field requires ASCII") from error
    if len(data) > width:
        raise BadInput("text exceeds COBOL field width")
    return data.ljust(width, b" ")


def area(request_id: bytes, customer: int, policy: int = 0) -> bytearray:
    result = bytearray(b" " * AREA_SIZE)
    result[:6] = request_id
    result[6:8] = b"00"
    result[8:18] = number(customer, 10)
    result[18:28] = number(policy, 10)
    return result


def run_business(transid: bytes, payload: bytearray) -> bytes:
    request = (b"CCI3\0" + transid + b"COBCICS " +
               len(payload).to_bytes(2, "big") + payload + b"\0\0")
    result = subprocess.run([str(WORKER)], input=request, capture_output=True,
        env=os.environ, timeout=30, check=False)
    if result.returncode:
        print(result.stderr.decode("utf-8", "replace")[-1000:], file=sys.stderr)
        raise RuntimeError("GenApp worker failed")
    output = result.stdout
    if not output.startswith(b"CCO6\0\0\0\0\x04\0APIR") or len(output) < 16:
        raise RuntimeError("invalid GenApp worker response")
    length = int.from_bytes(output[14:16], "big")
    if length != 2048 or len(output) < 16 + length:
        raise RuntimeError("invalid GenApp COMMAREA response")
    return output[16:16 + length]


def text_at(payload: bytes, start: int, width: int) -> str:
    return payload[start:start + width].decode("ascii").strip()


def result_or_error(payload: bytes) -> None:
    code = payload[6:8]
    if code == b"00":
        return
    if code == b"01":
        raise LookupError("record not found")
    raise RuntimeError(f"GenApp return code {code.decode('ascii', 'replace')}")


def customer_details(payload: bytes) -> dict:
    return {
        "customer_number": int(text_at(payload, 8, 10)),
        "first_name": text_at(payload, 18, 10),
        "last_name": text_at(payload, 28, 20),
        "date_of_birth": text_at(payload, 48, 10),
        "house_name": text_at(payload, 58, 20),
        "house_number": text_at(payload, 78, 4),
        "postcode": text_at(payload, 82, 8),
        "phone_mobile": text_at(payload, 93, 20),
        "phone_home": text_at(payload, 113, 20),
        "email": text_at(payload, 133, 100),
    }


def motor_details(payload: bytes) -> dict:
    return {
        "policy_number": int(text_at(payload, 18, 10)),
        "customer_number": int(text_at(payload, 8, 10)),
        "issue_date": text_at(payload, 28, 10),
        "expiry_date": text_at(payload, 38, 10),
        "make": text_at(payload, 100, 15),
        "model": text_at(payload, 115, 15),
        "value": int(text_at(payload, 130, 6)),
        "registration_number": text_at(payload, 136, 7),
        "colour": text_at(payload, 143, 8),
        "cc": int(text_at(payload, 151, 4)),
        "manufactured": text_at(payload, 155, 10),
        "premium": int(text_at(payload, 165, 6)),
        "accidents": int(text_at(payload, 171, 6)),
    }


def endowment_details(payload: bytes) -> dict:
    return {
        "policy_number": int(text_at(payload, 18, 10)),
        "customer_number": int(text_at(payload, 8, 10)),
        "with_profits": text_at(payload, 100, 1),
        "equities": text_at(payload, 101, 1),
        "managed_fund": text_at(payload, 102, 1),
        "fund_name": text_at(payload, 103, 10),
        "term": int(text_at(payload, 113, 2)),
        "sum_assured": int(text_at(payload, 115, 6)),
        "life_assured": text_at(payload, 121, 31),
    }


def house_details(payload: bytes) -> dict:
    return {
        "policy_number": int(text_at(payload, 18, 10)),
        "customer_number": int(text_at(payload, 8, 10)),
        "property_type": text_at(payload, 100, 15),
        "bedrooms": int(text_at(payload, 115, 3)),
        "value": int(text_at(payload, 118, 8)),
        "house_name": text_at(payload, 126, 20),
        "house_number": text_at(payload, 146, 4),
        "postcode": text_at(payload, 150, 8),
    }


COMMERCIAL_TEXT = (
    (100, 255, "address"), (355, 8, "postcode"),
    (363, 11, "latitude"), (374, 11, "longitude"),
    (385, 255, "customer"), (640, 255, "property_type"),
    (947, 255, "reject_reason"),
)
COMMERCIAL_NUMBERS = (
    (895, 4, "fire_peril"), (899, 8, "fire_premium"),
    (907, 4, "crime_peril"), (911, 8, "crime_premium"),
    (919, 4, "flood_peril"), (923, 8, "flood_premium"),
    (931, 4, "weather_peril"), (935, 8, "weather_premium"),
    (943, 4, "status"),
)


def commercial_details(payload: bytes) -> dict:
    result = {
        "policy_number": int(text_at(payload, 18, 10)),
        "customer_number": int(text_at(payload, 8, 10)),
    }
    for offset, width, name in COMMERCIAL_TEXT:
        result[name] = text_at(payload, offset, width)
    for offset, width, name in COMMERCIAL_NUMBERS:
        result[name] = int(text_at(payload, offset, width))
    return result


POLICY_TYPES = {
    "M": (b"01IMOT", b"01AMOT", motor_details),
    "E": (b"01IEND", b"01AEND", endowment_details),
    "H": (b"01IHOU", b"01AHOU", house_details),
    "C": (b"01ICOM", b"01ACOM", commercial_details),
}

POLICY_PATHS = {"motor": "M", "endowment": "E", "house": "H",
                "commercial": "C"}


def policy_area(request_id: bytes, data: dict) -> bytearray:
    payload = area(request_id, data["customer_number"])
    for offset, width, name in ((28, 10, "issue_date"),
                                (38, 10, "expiry_date")):
        payload[offset:offset + width] = fixed(data[name], width)
    payload[74:84] = number(0, 10)
    payload[94:100] = number(0, 6)
    return payload


def put_fields(payload: bytearray, data: dict, text_fields, number_fields) -> None:
    for offset, width, name in text_fields:
        payload[offset:offset + width] = fixed(data[name], width)
    for offset, width, name in number_fields:
        payload[offset:offset + width] = number(data[name], width)


def policy_add_area(policy_type: str, data: dict) -> bytearray:
    payload = policy_area(POLICY_TYPES[policy_type][1], data)
    if policy_type == "M":
        put_fields(payload, data,
            ((100, 15, "make"), (115, 15, "model"),
             (136, 7, "registration_number"), (143, 8, "colour"),
             (155, 10, "manufactured")),
            ((130, 6, "value"), (151, 4, "cc"),
             (165, 6, "premium"), (171, 6, "accidents")))
    elif policy_type == "E":
        put_fields(payload, data,
            ((100, 1, "with_profits"), (101, 1, "equities"),
             (102, 1, "managed_fund"), (103, 10, "fund_name"),
             (121, 31, "life_assured")),
            ((113, 2, "term"), (115, 6, "sum_assured")))
        padding = data.get("padding_data", "")
        if not isinstance(padding, str):
            raise BadInput("padding_data must be text")
        try:
            padding_bytes = padding.encode("ascii")
        except UnicodeEncodeError as error:
            raise BadInput("padding_data requires ASCII") from error
        if len(padding_bytes) > 3900:
            raise BadInput("padding_data exceeds COBOL VARCHAR capacity")
        payload[152:152 + len(padding_bytes)] = padding_bytes
    elif policy_type == "H":
        put_fields(payload, data,
            ((100, 15, "property_type"), (126, 20, "house_name"),
             (146, 4, "house_number"), (150, 8, "postcode")),
            ((115, 3, "bedrooms"), (118, 8, "value")))
    elif policy_type == "C":
        put_fields(payload, data, COMMERCIAL_TEXT, COMMERCIAL_NUMBERS)
    return payload


class Handler(BaseHTTPRequestHandler):
    def send_json(self, status: int, value: dict) -> None:
        data = json.dumps(value, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def handle_request(self) -> None:
        parsed = urlsplit(self.path)
        parts = parsed.path.strip("/").split("/")
        if self.command == "GET" and len(parts) == 3 and parts[:2] == ["genapp", "customers"]:
            customer = int(parts[2])
            output = run_business(b"GICU", area(b"01ICUS", customer))
            result_or_error(output)
            self.send_json(200, customer_details(output))
        elif self.command == "GET" and len(parts) == 3 and parts[:2] == ["genapp", "policies"]:
            policy = int(parts[2])
            query = parse_qs(parsed.query)
            if len(query.get("type", [])) != 1 or query["type"][0] not in POLICY_TYPES \
                    or len(query.get("customer", [])) != 1:
                raise BadInput("policy inquiry requires customer and type=M/E/H/C")
            customer = int(query["customer"][0])
            request_id, _, details = POLICY_TYPES[query["type"][0]]
            output = run_business(b"GIPU", area(request_id, customer, policy))
            result_or_error(output)
            self.send_json(200, details(output))
        elif self.command == "POST" and parts == ["genapp", "customers"]:
            data = self.read_json()
            payload = area(b"01ACUS", 0)
            fields = ((18, 10, "first_name"), (28, 20, "last_name"),
                      (48, 10, "date_of_birth"), (58, 20, "house_name"),
                      (78, 4, "house_number"), (82, 8, "postcode"),
                      (93, 20, "phone_mobile"), (113, 20, "phone_home"),
                      (133, 100, "email"))
            for offset, width, key in fields:
                payload[offset:offset + width] = fixed(data[key], width)
            payload[90:93] = b"000"
            output = run_business(b"GACU", payload)
            result_or_error(output)
            self.send_json(201, {"customer_number": int(text_at(output, 8, 10))})
        elif self.command == "POST" and len(parts) == 3 \
                and parts[:2] == ["genapp", "policies"] \
                and parts[2] in POLICY_PATHS:
            data = self.read_json()
            payload = policy_add_area(POLICY_PATHS[parts[2]], data)
            output = run_business(b"GAPU", payload)
            result_or_error(output)
            self.send_json(201, {"policy_number": int(text_at(output, 18, 10))})
        else:
            self.send_json(404, {"error": "endpoint not found"})

    def read_json(self) -> dict:
        length = int(self.headers.get("Content-Length", "0"))
        if length < 1 or length > 8192:
            raise BadInput("JSON body must be 1..8192 bytes")
        result = json.loads(self.rfile.read(length))
        if not isinstance(result, dict):
            raise BadInput("JSON body must be an object")
        return result

    def do_GET(self) -> None:
        self.dispatch()

    def do_POST(self) -> None:
        self.dispatch()

    def dispatch(self) -> None:
        try:
            self.handle_request()
        except (BadInput, KeyError, ValueError, TypeError) as error:
            self.send_json(400, {"error": str(error)})
        except LookupError as error:
            self.send_json(404, {"error": str(error)})
        except (RuntimeError, subprocess.TimeoutExpired) as error:
            self.send_json(502, {"error": str(error)})


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--listen", default="127.0.0.1:8088")
    args = parser.parse_args()
    host, port = args.listen.rsplit(":", 1)
    server = ThreadingHTTPServer((host, int(port)), Handler)
    print(f"GenApp HTTP listening at {server.server_address}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
