#!/usr/bin/env python3
"""Copy IBM GenApp's BMS source with its malformed INITIAL literal repaired.

The reference checkout remains untouched. Fail if the upstream text changes so
that a new source revision is reviewed instead of silently receiving this fix.
"""

import pathlib
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]
SOURCE = ROOT / "reference/cics-genapp/base/src/ssmap.bms"
BAD = "INITIAL='Fund Name '    '"
GOOD = "INITIAL='Fund Name '"


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: normalize-genapp-bms.py OUTPUT", file=sys.stderr)
        return 2
    text = SOURCE.read_text(encoding="ascii")
    if text.count(BAD) != 1:
        print(f"expected exactly one malformed INITIAL in {SOURCE}", file=sys.stderr)
        return 1
    destination = pathlib.Path(sys.argv[1])
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(text.replace(BAD, GOOD), encoding="ascii")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
