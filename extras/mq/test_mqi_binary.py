#!/usr/bin/env python3
"""MQI TCP regression: binary payload and non-destructive truncation."""
import argparse
import ctypes
import os
import pathlib
import struct

from test_carddemo import Client, integer, message, number


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bridge", type=pathlib.Path)
    parser.add_argument("password_file", type=pathlib.Path)
    args = parser.parse_args()
    os.environ["MQ_PASSWORD_FILE"] = str(args.password_file.resolve())

    client = Client(args.bridge)
    obj = client.open("DEV.COBMQ.DEMO", 18)
    try:
        payload = b"\x00\xff\x81MQ\n\x00END"
        correlation = os.urandom(24)
        client.put(obj, payload, correlation, "")
        small = ctypes.create_string_buffer(4)
        data_length = integer()
        client.call(
            "MQGET", client.hconn, obj, message(correlation),
            ctypes.create_string_buffer(struct.pack(">4siii", b"GMO ", 1, 0, 0)),
            integer(4), small, data_length, expected=2080,
        )
        if number(data_length) != len(payload):
            raise RuntimeError("MQGET did not return the required binary length")
        if client.get(obj, correlation) != payload:
            raise RuntimeError("binary payload changed or was lost after truncation")
        print("MQI TCP binary payload and truncation retry OK")
    finally:
        client.close(obj)
        client.disconnect()


if __name__ == "__main__":
    main()
