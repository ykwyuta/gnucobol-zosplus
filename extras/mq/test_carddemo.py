#!/usr/bin/env python3
"""Exercise CardDemo through the IBM MQ C client bridge over TCP."""
import argparse
import ctypes
import os
import pathlib
import struct
import subprocess


def field(value: bytes, size: int) -> bytes:
    return value.ljust(size, b" ")[:size]


def integer(value: int = 0):
    return ctypes.create_string_buffer(struct.pack(">i", value))


def number(value) -> int:
    return struct.unpack(">i", value.raw[:4])[0]


def descriptor(queue: str):
    return ctypes.create_string_buffer(struct.pack(
        ">4sii48s48s", b"OD  ", 1, 1, field(queue.encode(), 48), b" " * 48))


def message(correlation: bytes, reply_to: str = ""):
    return ctypes.create_string_buffer(struct.pack(
        ">4siiiii8s24s24s48s48s",
        b"MD  ", 1, 8, -1, 0, 0, b"MQSTR   ",
        bytes(24), correlation, field(reply_to.encode(), 48), b" " * 48))


class Client:
    def __init__(self, bridge: pathlib.Path):
        self.lib = ctypes.CDLL(str(bridge.resolve()))
        self.hconn = integer()
        self.call("MQCONN", ctypes.create_string_buffer(b"QM1"), self.hconn)

    def call(self, name, *args, expected=0):
        cc, reason = integer(), integer()
        getattr(self.lib, name)(*(ctypes.byref(arg) for arg in (*args, cc, reason)))
        valid_cc = (0,) if expected == 0 else (1, 2)
        if number(reason) != expected or number(cc) not in valid_cc:
            raise RuntimeError(f"{name}: completion={number(cc)}, reason={number(reason)}")

    def open(self, queue: str, options: int):
        obj = integer()
        self.call("MQOPEN", self.hconn, descriptor(queue), integer(options), obj)
        return obj

    def put(self, obj, payload: bytes, correlation: bytes, reply_to: str):
        body = ctypes.create_string_buffer(payload)
        self.call("MQPUT", self.hconn, obj, message(correlation, reply_to),
                  ctypes.create_string_buffer(struct.pack(">4sii", b"PMO ", 1, 0)),
                  integer(len(payload)), body)

    def get(self, obj, correlation: bytes):
        body = ctypes.create_string_buffer(4096)
        data_length = integer()
        self.call("MQGET", self.hconn, obj, message(correlation),
                  ctypes.create_string_buffer(struct.pack(">4siii", b"GMO ", 1, 1, 10000)),
                  integer(len(body)), body, data_length)
        return body.raw[:number(data_length)]

    def close(self, obj):
        self.call("MQCLOSE", self.hconn, obj, integer())

    def disconnect(self):
        self.call("MQDISC", self.hconn)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("program", type=pathlib.Path)
    parser.add_argument("bridge", type=pathlib.Path)
    parser.add_argument("libcob", type=pathlib.Path)
    parser.add_argument("password_file", type=pathlib.Path)
    parser.add_argument("--account", action="store_true")
    parser.add_argument("--acct-data", type=pathlib.Path)
    args = parser.parse_args()
    if args.account and args.acct_data is None:
        parser.error("--account requires --acct-data")
    kind = "ACCT" if args.account else "DATE"
    request_queue = f"CARD.DEMO.REQUEST.{kind}"
    reply_queue = f"CARD.DEMO.REPLY.{kind}"
    env = os.environ.copy()
    env.update({
        "COB_PRE_LOAD": str(args.bridge.resolve()),
        "LD_LIBRARY_PATH": str(args.libcob.resolve()) +
                           (":" + env["LD_LIBRARY_PATH"] if env.get("LD_LIBRARY_PATH") else ""),
        "MQ_PASSWORD_FILE": str(args.password_file.resolve()),
        "COBCICS_MQ_TRIGGER_QUEUE": request_queue,
    })
    if args.account:
        env["CARDDEMO_ACCTDATA_FILE"] = str(args.acct_data.resolve())
    # The test process uses the same IBM C client as the COBOL subprocess.
    os.environ["MQ_PASSWORD_FILE"] = str(args.password_file.resolve())
    client = Client(args.bridge)
    incoming = client.open(request_queue, 16)
    outgoing = client.open(reply_queue, 2)
    try:
        correlation = os.urandom(24)
        payload = b"INQA00000000001" if args.account else b"DATE"
        client.put(incoming, payload, correlation, reply_queue)
        transid = b"CDRA" if args.account else b"CDRD"
        frame = b"CCI3\x00" + transid + b"CARDDEMO\x00\x00\x00\x00"
        result = subprocess.run([str(args.program.resolve())], input=frame,
                                capture_output=True, env=env, timeout=35)
        if result.returncode or not result.stdout.startswith(b"CCO7"):
            raise RuntimeError(f"CardDemo {kind} failed: exit={result.returncode}, "
                               f"stdout={result.stdout!r}, stderr={result.stderr!r}")
        body = client.get(outgoing, correlation)
        expected = b"ACCOUNT ID : 00000000001" if args.account else b"SYSTEM DATE : "
        if expected not in body or (not args.account and b"SYSTEM TIME : " not in body):
            raise RuntimeError(f"invalid CardDemo reply: {body!r}")
        print(f"CardDemo {args.program.name} IBM MQ TCP request/reply OK")
        print(body.rstrip().decode("utf-8", errors="replace"))
    finally:
        client.close(outgoing)
        client.close(incoming)
        client.disconnect()


if __name__ == "__main__":
    main()
