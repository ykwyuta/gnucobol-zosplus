#!/usr/bin/env python3
#
# Copyright (C) 2026 Free Software Foundation, Inc.
#
# This file is part of GnuCOBOL.
#
# The GnuCOBOL compiler is free software: you can redistribute it
# and/or modify it under the terms of the GNU General Public License
# as published by the Free Software Foundation, either version 3 of the
# License, or (at your option) any later version.
#
# GnuCOBOL is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with GnuCOBOL.  If not, see <https://www.gnu.org/licenses/>.

"""Generate libcob/charset_tables.h - the single byte (SBCS) tables for the
EBCDIC data charsets (see docs/proposal/ebcdic-internal-encoding-ja.md).

Each charset gets a reversible (bijective) mapping between its 256 EBCDIC
code points and 256 host bytes:
* IBM-037: host is ISO-8859-1
* IBM-930 / 939 / 1390 / 1399 (Japanese): host is the single byte part of
  Shift_JIS / CP932, i.e. ASCII + JIS X 0201 half-width katakana
  (0xA1 - 0xDF); YEN SIGN and OVERLINE are mapped to 0x5C / 0x7E if the
  EBCDIC charset has no REVERSE SOLIDUS / TILDE (JIS X 0201 Roman)
EBCDIC code points without a host equivalent (and unassigned ones) are
mapped to otherwise unused host bytes, so data is never lost when it is
converted back and forth.

Usage: gen-charset-tables.py > libcob/charset_tables.h
Needs python3 and iconv (glibc) with IBM930, IBM939, IBM1390, IBM1399.
"""

import codecs
import subprocess
import sys

CHARSETS = [
    # C name, display name, ccsid, iconv name, mixed (SO/SI), host
    ("ibm037", "IBM-037", 37, None, False, "latin1"),
    ("ibm930", "IBM-930", 930, "IBM930", True, "sjis"),
    ("ibm939", "IBM-939", 939, "IBM939", True, "sjis"),
    ("ibm1390", "IBM-1390", 1390, "IBM1390", True, "sjis"),
    ("ibm1399", "IBM-1399", 1399, "IBM1399", True, "sjis"),
]

SO, SI = 0x0E, 0x0F


def ebcdic_codepoint(iconv_name, byte):
    """Unicode code point of a single EBCDIC byte, None if unassigned."""
    if byte < 0x40 or byte == 0xFF:
        # control characters are identical in all EBCDIC charsets
        return ord(bytes([byte]).decode("cp037"))
    if iconv_name is None:
        return ord(bytes([byte]).decode("cp037"))
    res = subprocess.run(["iconv", "-f", iconv_name, "-t", "UTF-32BE"],
                         input=bytes([byte]), capture_output=True)
    if res.returncode != 0 or len(res.stdout) != 4:
        return None
    cp = int.from_bytes(res.stdout, "big")
    if cp in (0x1A, 0xFFFD):
        # substitution character - unassigned
        return None
    return cp


def host_byte(cp, host, have_backslash, have_tilde):
    if cp is None:
        return None
    if host == "latin1":
        return cp if cp < 0x100 else None
    if cp < 0x80:
        return cp
    if 0x80 <= cp <= 0x9F:
        # C1 controls, kept as raw bytes
        return cp
    if 0xFF61 <= cp <= 0xFF9F:
        return cp - 0xFF61 + 0xA1
    if cp == 0xA5 and not have_backslash:
        return 0x5C
    if cp == 0x203E and not have_tilde:
        return 0x7E
    return None


def build(iconv_name, mixed, host):
    cps = [ebcdic_codepoint(iconv_name, b) for b in range(256)]
    have_backslash = 0x5C in cps
    have_tilde = 0x7E in cps
    to_host = [None] * 256
    used = set()
    for b in range(256):
        if mixed and b in (SO, SI):
            h = b
        else:
            h = host_byte(cps[b], host, have_backslash, have_tilde)
        if h is not None and h not in used:
            to_host[b] = h
            used.add(h)
    # reversible fill: unmapped EBCDIC bytes get unused host bytes,
    # preferring the identical value
    free = [h for h in range(256) if h not in used]
    for b in range(256):
        if to_host[b] is None and b not in used and b in free:
            to_host[b] = b
            used.add(b)
            free.remove(b)
    for b in range(256):
        if to_host[b] is None:
            to_host[b] = free.pop(0)
    from_host = [0] * 256
    for b, h in enumerate(to_host):
        from_host[h] = b
    assert sorted(to_host) == list(range(256))
    return to_host, from_host


def ctype_tables(to_host, from_host):
    ctype = [0] * 256
    upper = list(range(256))
    lower = list(range(256))
    for b in range(256):
        h = to_host[b]
        if 0x30 <= h <= 0x39:
            ctype[b] |= 0x01            # COB_CT_DIGIT
        if 0x41 <= h <= 0x5A:
            ctype[b] |= 0x02            # COB_CT_UPPER
            lower[b] = from_host[h + 0x20]
        if 0x61 <= h <= 0x7A:
            ctype[b] |= 0x04            # COB_CT_LOWER
            upper[b] = from_host[h - 0x20]
        if h == 0x20:
            ctype[b] |= 0x08 | 0x10     # COB_CT_SPACE | COB_CT_WHITE
        if 0x09 <= h <= 0x0D:
            ctype[b] |= 0x10            # COB_CT_WHITE
    return ctype, upper, lower


def table(name, values):
    lines = ["static const unsigned char %s[256] = {" % name]
    for i in range(0, 256, 16):
        row = ", ".join("0x%02X" % v for v in values[i:i + 16])
        lines.append("\t" + row + ("," if i < 240 else ""))
    lines.append("};")
    return "\n".join(lines)


def main():
    out = []
    out.append("/* Generated by build_aux/gen-charset-tables.py - do not edit */")
    out.append("")
    out.append("/* SBCS tables for the EBCDIC data charsets, see charset.c */")
    for cname, dname, ccsid, iconv_name, mixed, host in CHARSETS:
        to_host, from_host = build(iconv_name, mixed, host)
        ctype, upper, lower = ctype_tables(to_host, from_host)
        out.append("")
        out.append("/* %s (CCSID %d), host: %s */" % (
            dname, ccsid, "ISO-8859-1" if host == "latin1"
            else "single byte part of Shift_JIS"))
        out.append(table("%s_to_host" % cname, to_host))
        out.append(table("%s_from_host" % cname, from_host))
        out.append(table("%s_ctype" % cname, ctype))
        out.append(table("%s_upper" % cname, upper))
        out.append(table("%s_lower" % cname, lower))
    out.append("")
    sys.stdout.write("\n".join(out))


if __name__ == "__main__":
    main()
