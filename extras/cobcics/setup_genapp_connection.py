"""Connection settings shared by GenApp's isolated Db2 fixture scripts."""

import os


def connection_string() -> str:
    required = ("DB2_HOST", "DB2_PORT", "DB2_DATABASE", "DB2_USER", "DB2_PASSWORD")
    missing = [name for name in required if not os.environ.get(name)]
    if missing:
        raise RuntimeError("missing Db2 settings: " + ", ".join(missing))
    return (f"DATABASE={os.environ['DB2_DATABASE']};"
            f"HOSTNAME={os.environ['DB2_HOST']};PORT={os.environ['DB2_PORT']};"
            f"PROTOCOL=TCPIP;UID={os.environ['DB2_USER']};"
            f"PWD={os.environ['DB2_PASSWORD']};")
