#!/usr/bin/env python3
"""Build GenApp's symbolic map and report each COBOL translation failure.

Writes converted source and a machine-readable report under build/genapp.
The command exits nonzero while any source remains unsupported.
"""

import json
import pathlib
import re
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[2]
SOURCE = ROOT / "reference/cics-genapp/base/src"
OUTPUT = ROOT / "build/genapp"


def run(*args: object) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(arg) for arg in args], cwd=ROOT, text=True, capture_output=True
    )


def diagnostic(result: subprocess.CompletedProcess[str]) -> str:
    return (result.stderr or result.stdout).strip()


def main() -> int:
    for path in (OUTPUT, OUTPUT / "cics", OUTPUT / "sql", OUTPUT / "normalized"):
        path.mkdir(parents=True, exist_ok=True)
    normalized = OUTPUT / "ssmap-normalized.bms"
    copybook = OUTPUT / "SSMAP.cpy"
    for command in (
        (sys.executable, "extras/cobcics/normalize-genapp-bms.py", normalized),
        ("cargo", "run", "--offline", "--quiet", "--manifest-path",
         "extras/cobcics/Cargo.toml", "--bin", "cobcics-mapgen", "--",
         normalized, copybook),
    ):
        result = run(*command)
        if result.returncode:
            print(diagnostic(result), file=sys.stderr)
            return 1

    rows = []
    for source in sorted(SOURCE.glob("*.cbl")):
        converted = OUTPUT / "cics" / f"{source.stem}.cob"
        rendered = OUTPUT / "sql" / f"{source.stem}.cob"
        converted.unlink(missing_ok=True)
        rendered.unlink(missing_ok=True)
        text = source.read_text(encoding="ascii")
        input_source = source
        abstime_pattern = re.compile(
            r"(01\s+(?:WS-ABSTIME|ABS-TIME)\s+PIC )"
            r"S9\(8\) COMP( VALUE \+0\.)",
            re.I)
        text, abstime_fixes = abstime_pattern.subn(
            r"\1S9(15) COMP-3\2", text)
        if source.stem.lower() in {"lgtestc1", "lgtestp1", "lgtestp2",
                                   "lgtestp3", "lgtestp4"}:
            marker = "       A-GAIN.\n"
            if text.count(marker) != 1:
                raise RuntimeError(f"{source.name} screen continuation no longer matches")
            # These sample programs SEND and immediately RECEIVE in one CICS
            # task.  The worker ABI resumes at the next terminal event.
            text = text.replace(marker,
                "           GO TO ENDIT-STARTIT.\n\n" + marker)
            text, returns = re.subn(
                r"(EXEC CICS RETURN\s+TRANSID\('[A-Z0-9]+'\)\s+"
                r"COMMAREA\(COMM-AREA\))",
                r"\1\n                LENGTH(1)", text, flags=re.I)
            if returns < 1:
                raise RuntimeError(f"{source.name} RETURN normalization no longer matches")
            if source.stem.lower() == "lgtestc1":
                text, spaces = re.subn(
                    r"(Inspect COMM-AREA Replacing All x'00'\s+by )x'40'",
                    r"\1x'20'", text, flags=re.I)
                if spaces != 2:
                    raise RuntimeError("LGTESTC1 ASCII space normalization changed")
            if source.stem.lower() in {"lgtestc1", "lgtestp1",
                                       "lgtestp2", "lgtestp3"}:
                text, branches = re.subn(
                    r"(             WHEN '4'\n)",
                    r"\1                 IF COBCICS-COMMAREA(1:1) NOT = 'U'\n",
                    text, count=1)
                if branches != 1:
                    raise RuntimeError(f"{source.name} update branch changed")
                transid = "SSC1" if source.stem.lower() == "lgtestc1" else \
                          "SSP" + source.stem[-1]
                stage = (
                    "\n                 MOVE COMM-AREA(1:128)\n"
                    "                   TO COBCICS-COMMAREA(2:128)\n"
                    "                 MOVE 'U' TO COBCICS-COMMAREA(1:1)\n"
                    "                 EXEC CICS RETURN\n"
                    f"                   TRANSID('{transid}')\n"
                    "                   COMMAREA(COBCICS-COMMAREA)\n"
                    "                   LENGTH(129) END-EXEC\n"
                    "                 END-IF\n"
                    "                 MOVE COBCICS-COMMAREA(2:128)\n"
                    "                   TO COMM-AREA(1:128)")
                text, receives = re.subn(
                    r"(?<=END-EXEC)\s+EXEC CICS RECEIVE MAP\('[^']+'\)"
                    r".*?END-EXEC", stage, text, count=1, flags=re.I | re.S)
                if receives != 1:
                    raise RuntimeError(f"{source.name} update RECEIVE changed")
            if source.stem.lower() == "lgtestp1":
                marker = ("           Move 'Error Updating Motor Policy'"
                          "    To  ERP1FLDO\n")
                if text.count(marker) != 1:
                    raise RuntimeError("LGTESTP1 update error message changed")
                text = text.replace(marker, marker +
                    "           MOVE CA-RETURN-CODE TO ERP1FLDO(30:2)\n")
            normalized = OUTPUT / "normalized" / source.name
            normalized.write_text(text, encoding="ascii")
            input_source = normalized
        if source.name.lower() == "lgwebst5.cbl":
            marker = "       PROGRAM-ID. LGWEBST5\n"
            if text.count(marker) != 1:
                raise RuntimeError("LGWEBST5 PROGRAM-ID normalization no longer matches")
            normalized = OUTPUT / "normalized" / source.name
            text = text.replace(marker, marker.rstrip("\n") + ".\n")
            normalized.write_text(text, encoding="ascii")
            input_source = normalized
        elif source.name.lower() == "lgupdb01.cbl":
            columns = "                      BROKERSREFERENCE\n               FROM POLICY"
            if text.count(columns) != 1:
                raise RuntimeError("LGUPDB01 cursor SELECT columns changed")
            text = text.replace(columns,
                "                      BROKERSREFERENCE,\n"
                "                      PAYMENT\n               FROM POLICY")
            marker = "       CLOSE-PCURSOR.\n"
            if text.count(marker) != 1:
                raise RuntimeError("LGUPDB01 cursor-close paragraph changed")
            head, tail = text.split(marker, 1)
            tail, fixes = re.subn(
                r"(When 0\s+)MOVE '00' TO CA-RETURN-CODE",
                r"\1CONTINUE", tail, count=1, flags=re.I)
            if fixes != 1:
                raise RuntimeError("LGUPDB01 cursor-close status changed")
            # The reference calls LGUPVS01 even when the policy timestamp
            # comparison returned 02. Do not overwrite the KSDS record for
            # an edit based on a stale screen.
            vsam_call = (
                "           EXEC CICS LINK Program(LGUPVS01)\n"
                "                Commarea(DFHCOMMAREA)\n"
                "                LENGTH(225)\n"
                "           END-EXEC."
            )
            if head.count(vsam_call) != 1:
                raise RuntimeError("LGUPDB01 update VSAM call changed")
            head = head.replace(vsam_call,
                "           IF CA-RETURN-CODE = '00'\n"
                + vsam_call.replace("           END-EXEC.",
                                    "           END-EXEC\n")
                + "           END-IF.")
            normalized = OUTPUT / "normalized" / source.name
            normalized.write_text(head + marker + tail, encoding="ascii")
            input_source = normalized
        elif source.name.lower() == "lgapol01.cbl":
            marker = "           EXEC CICS Link Program(LGAPDB01)\n"
            if text.count(marker) != 1:
                raise RuntimeError("LGAPOL01 add-policy LINK changed")
            # The reference passes the whole 32500-byte area even when no
            # ENDOWMENT padding was supplied. LGAPDB01 interprets every
            # extra byte as VARCHAR data, while its host field holds 3900.
            length = (
                "           EVALUATE CA-REQUEST-ID\n"
                "             WHEN '01AEND'\n"
                "               ADD 124 TO WS-REQUIRED-CA-LEN\n"
                "               IF CA-E-PADDING-DATA NOT = SPACES\n"
                "                 COMPUTE WS-REQUIRED-CA-LEN =\n"
                "                   WS-REQUIRED-CA-LEN + FUNCTION LENGTH(\n"
                "                   FUNCTION TRIM(CA-E-PADDING-DATA TRAILING))\n"
                "               END-IF\n"
                "             WHEN '01AHOU'\n"
                "               ADD 130 TO WS-REQUIRED-CA-LEN\n"
                "             WHEN '01AMOT'\n"
                "               ADD 137 TO WS-REQUIRED-CA-LEN\n"
                "             WHEN '01ACOM'\n"
                "               ADD 1174 TO WS-REQUIRED-CA-LEN\n"
                "           END-EVALUATE\n"
                "           IF WS-REQUIRED-CA-LEN > 4052\n"
                "             MOVE '98' TO CA-RETURN-CODE\n"
                "             EXEC CICS RETURN END-EXEC\n"
                "           END-IF\n")
            text = text.replace(marker, length + marker)
            text, count = re.subn(
                r"(EXEC CICS Link Program\(LGAPDB01\)\s+"
                r"Commarea\(DFHCOMMAREA\)\s+)LENGTH\(32500\)",
                r"\1LENGTH(WS-REQUIRED-CA-LEN)", text, count=1, flags=re.I)
            if count != 1:
                raise RuntimeError("LGAPOL01 add-policy LINK length changed")
            normalized = OUTPUT / "normalized" / source.name
            normalized.write_text(text, encoding="ascii")
            input_source = normalized
        elif abstime_fixes and input_source == source:
            normalized = OUTPUT / "normalized" / source.name
            normalized.write_text(text, encoding="ascii")
            input_source = normalized
        ims = bool(re.search(r"\b(?:EXEC\s+DLI|CBLTDLI|DFSDLI)\b", text, re.I))
        row = {"source": source.name, "ims_candidate": ims}
        if ims:
            row.update(stage="excluded_candidate", diagnostic="IMS/DLI call; review dependencies")
        else:
            result = run(sys.executable, "bin/cics-preprocess.py", input_source,
                         copybook, converted)
            if result.returncode:
                row.update(stage="cics", diagnostic=diagnostic(result))
            else:
                result = run(sys.executable, "bin/db2sql-preprocess.py",
                             "-I", SOURCE, converted, rendered)
                if result.returncode:
                    row.update(stage="sql", diagnostic=diagnostic(result))
                else:
                    row.update(stage="translated", diagnostic="")
        rows.append(row)
        message = row["diagnostic"].splitlines()[-1] if row["diagnostic"] else "OK"
        print(f"{source.name}: {row['stage']}: {message}")

    (OUTPUT / "audit.json").write_text(
        json.dumps(rows, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    complete = sum(row["stage"] == "translated" for row in rows)
    print(f"translated {complete}/{len(rows)}; report: {OUTPUT / 'audit.json'}")
    return 0 if complete == len(rows) else 1


if __name__ == "__main__":
    raise SystemExit(main())
