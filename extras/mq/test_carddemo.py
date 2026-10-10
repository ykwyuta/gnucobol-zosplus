#!/usr/bin/env python3
"""Exercise CardDemo date or account transaction against IBM MQ Compose."""
import argparse
import base64
import os
import pathlib
import ssl
import subprocess
import urllib.error
import urllib.request


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
    password = args.password_file.read_text(encoding="utf-8").strip()
    auth = base64.b64encode(f"app:{password}".encode()).decode()
    base = os.environ.get("MQ_REST_URL", "https://localhost:9444")
    endpoint = f"{base}/ibmmq/rest/v2/messaging/qmgr/QM1/queue"
    context = ssl._create_unverified_context()  # Local Compose certificate.
    correlation = os.urandom(24).hex()
    headers = {
        "Authorization": f"Basic {auth}",
        "ibm-mq-rest-csrf-token": "cobmq-test",
        "Accept": "text/plain",
    }
    request_headers = {
        **headers,
        "Content-Type": "text/plain; charset=utf-8",
        "ibm-mq-md-correlationId": correlation,
        "ibm-mq-md-replyTo": reply_queue,
    }
    request_url = f"{endpoint}/{request_queue}/message"
    reply_url = (f"{endpoint}/{reply_queue}/message"
                 f"?correlationId={correlation}&wait=10000")
    request_body = b"INQA00000000001" if args.account else b"DATE"
    with urllib.request.urlopen(urllib.request.Request(
            request_url, data=request_body, headers=request_headers,
            method="POST"), context=context) as response:
        if response.status != 201:
            raise RuntimeError(f"request put failed: {response.status}")

    env = os.environ.copy()
    env.update({
        "COB_PRE_LOAD": str(args.bridge.resolve()),
        "LD_LIBRARY_PATH": str(args.libcob.resolve()),
        "MQ_PASSWORD_FILE": str(args.password_file.resolve()),
        "MQ_TLS_INSECURE": "1",
        "COBCICS_MQ_TRIGGER_QUEUE": request_queue,
    })
    if args.account:
        env["CARDDEMO_ACCTDATA_FILE"] = str(args.acct_data.resolve())
    transid = b"CDRA" if args.account else b"CDRD"
    frame = b"CCI3\x00" + transid + b"CARDDEMO\x00\x00\x00\x00"
    result = subprocess.run([str(args.program.resolve())], input=frame,
                            capture_output=True, env=env, timeout=35)
    if result.returncode or not result.stdout.startswith(b"CCO7"):
        raise RuntimeError(f"CardDemo {kind} failed: exit={result.returncode}, "
                           f"stdout={result.stdout!r}, stderr={result.stderr!r}")
    try:
        with urllib.request.urlopen(urllib.request.Request(
                reply_url, headers=headers, method="DELETE"),
                context=context) as response:
            body = response.read()
    except urllib.error.HTTPError as error:
        raise RuntimeError(f"reply get failed: {error.code}, "
                           f"{error.read()!r}") from error
    expected = b"ACCOUNT ID : 00000000001" if args.account else b"SYSTEM DATE : "
    if expected not in body or (not args.account and b"SYSTEM TIME : " not in body):
        raise RuntimeError(f"invalid CardDemo reply: {body!r}")
    print(f"CardDemo {args.program.name} IBM MQ request/reply OK")
    print(body.rstrip().decode("utf-8", errors="replace"))


if __name__ == "__main__":
    main()
