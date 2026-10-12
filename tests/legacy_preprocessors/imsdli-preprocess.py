#!/usr/bin/env python3
"""Translate the EXEC DLI forms used by CardDemo into CBLTDLI calls.

This is a fixed-format source transform. Unknown commands and clauses fail with
a source line number so an untranslated DLI request cannot silently disappear.
"""
import argparse
from pathlib import Path
import re

DATA_COMMANDS = {"GU", "GN", "GNP", "GHU", "GHN", "GHNP", "ISRT", "REPL", "DLET"}
HOLD = {"GU": "GHU", "GN": "GHN", "GNP": "GHNP"}
CLAUSES = ("PCB", "SEGMENT", "INTO", "FROM", "WHERE", "SEGLENGTH", "ID", "PSB")


def clauses(body, line_number):
    found = []
    pos = 0
    while pos < len(body):
        while pos < len(body) and body[pos].isspace():
            pos += 1
        if pos >= len(body):
            break
        match = re.match(r"([A-Z]+)\s*\(", body[pos:], re.I)
        if not match:
            word = re.match(r"[A-Z]+", body[pos:], re.I)
            if word and word.group().upper() in {"USING", "NODHABEND"}:
                pos += len(word.group())
                continue
            raise ValueError(f"line {line_number}: unsupported EXEC DLI operand {body[pos:]!r}")
        key = match.group(1).upper()
        if key not in CLAUSES:
            raise ValueError(f"line {line_number}: unsupported EXEC DLI clause {key}")
        start = pos + match.end()
        depth = 1
        end = start
        while end < len(body) and depth:
            if body[end] == "(":
                depth += 1
            elif body[end] == ")":
                depth -= 1
            end += 1
        if depth:
            raise ValueError(f"line {line_number}: unclosed {key} clause")
        found.append((key, body[start:end - 1].strip()))
        pos = end
    return found


def statement(text):
    return "           " + text + "\n"


def ssa_lines(index, segment, where, line_number):
    target = f"IMS-DLI-SSA-{index}"
    if not re.fullmatch(r"[A-Z0-9]{1,8}", segment, re.I):
        raise ValueError(f"line {line_number}: invalid segment name {segment!r}")
    if not where:
        return [statement(f"MOVE '{segment:<8}' TO {target}")]
    field, separator, value = where.partition("=")
    field, value = field.strip(), value.strip()
    if not separator or not re.fullmatch(r"[A-Z0-9]{1,8}", field, re.I) or not re.fullmatch(r"[A-Z0-9-]+", value, re.I):
        raise ValueError(f"line {line_number}: unsupported WHERE {where!r}")
    return [
        statement(f"MOVE SPACES TO {target}"),
        statement(f"STRING '{segment:<8}' '(' '{field:<8}' 'EQ'"),
        statement(f"    DELIMITED BY SIZE INTO {target} END-STRING"),
        statement("IF IMS-DLI-RC = ZERO"),
        statement(f"CALL STATIC 'IMSDSA' USING {target} {value}"),
        statement("    RETURNING IMS-DLI-RC"),
        statement("END-IF"),
    ]


def translate_command(command, body, line_number):
    operation = command.upper()
    parts = clauses(body, line_number)
    if operation in {"CHKP", "SCHD", "TERM"}:
        expected = {"CHKP": {"ID"}, "SCHD": {"PSB"}, "TERM": set()}[operation]
        actual = {key for key, _ in parts}
        if actual - expected:
            raise ValueError(f"line {line_number}: unsupported {operation} clauses {actual - expected}")
        arg = "SPACES" if operation == "TERM" else None
        for key, value in parts:
            if key == "PSB" and value.startswith("(") and value.endswith(")"):
                value = value[1:-1].strip()
            arg = value
        if not arg:
            raise ValueError(f"line {line_number}: {operation} requires an argument")
        return [
            statement(f"MOVE '{operation:<4}' TO IMS-DLI-FUNCTION"),
            statement(f"MOVE {arg} TO IMS-DLI-ARG"),
            statement("CALL 'IMSDEXEC' USING IMS-DLI-FUNCTION"),
            statement("    IMS-DLI-ARG DIBSTAT"),
        ]
    if operation not in DATA_COMMANDS:
        raise ValueError(f"line {line_number}: unsupported EXEC DLI command {operation}")
    values = {}
    segments = []
    where = {}
    for key, value in parts:
        if key == "SEGMENT":
            segments.append(value)
        elif key == "WHERE":
            if not segments:
                raise ValueError(f"line {line_number}: WHERE precedes SEGMENT")
            where[len(segments)] = value
        elif key in values:
            raise ValueError(f"line {line_number}: repeated {key}")
        else:
            values[key] = value
    if not segments or len(segments) > 2 or "PCB" not in values:
        raise ValueError(f"line {line_number}: expected PCB and one or two SEGMENT clauses")
    if values["PCB"] != "PAUT-PCB-NUM":
        raise ValueError(f"line {line_number}: unmapped PCB {values['PCB']}")
    area_key = "INTO" if operation in {"GU", "GN", "GNP", "GHU", "GHN", "GHNP"} else "FROM"
    if area_key not in values or any(key not in {"PCB", "INTO", "FROM", "SEGLENGTH"} for key in values):
        raise ValueError(f"line {line_number}: missing {area_key} or unsupported clause")
    if "SEGLENGTH" in values and values["SEGLENGTH"].upper() != f"LENGTH OF {values[area_key]}".upper():
        raise ValueError(f"line {line_number}: unsupported SEGLENGTH")
    lines = [statement("MOVE ZERO TO IMS-DLI-RC"),
             statement(f"MOVE '{HOLD.get(operation, operation):<4}' TO IMS-DLI-FUNCTION")]
    for index, segment in enumerate(segments, 1):
        lines += ssa_lines(index, segment, where.get(index), line_number)
    lines += [
        statement("IF IMS-DLI-RC = ZERO"),
        statement("CALL 'CBLTDLI' USING IMS-DLI-FUNCTION"),
        statement(f"    IMS-DLI-PCB {values[area_key]}"),
        statement("    " + " ".join(f"IMS-DLI-SSA-{n}" for n in range(1, len(segments) + 1))),
        statement("    RETURNING IMS-DLI-RC"),
        statement("END-IF"),
        statement("IF IMS-DLI-RC = 0"),
        statement("    MOVE IMS-DLI-STATUS TO DIBSTAT"),
        statement("ELSE"),
        statement("    MOVE 'AJ' TO DIBSTAT"),
        statement("END-IF"),
    ]
    return lines


def translate(source, dbd):
    if not re.fullmatch(r"[A-Z0-9]{1,8}", dbd):
        raise ValueError("DBD name must be 1..8 uppercase letters or digits")
    lines = source.splitlines(keepends=True)
    output = []
    index = 0
    injected = False
    while index < len(lines):
        line = lines[index]
        code = line[7:72] if len(line) > 7 else ""
        if (not injected and re.search(
                r"\b(?:LINKAGE\s+SECTION|PROCEDURE\s+DIVISION)\b",
                code, re.I)):
            output.extend([
                statement("01 DIBSTAT PIC X(2)."),
                statement("01 IMS-DLI-FUNCTION PIC X(4)."),
                statement("01 IMS-DLI-ARG PIC X(8)."),
                statement("01 IMS-DLI-RC PIC S9(9) COMP-5."),
                statement("01 IMS-DLI-SSA-1 PIC X(256)."),
                statement("01 IMS-DLI-SSA-2 PIC X(256)."),
                statement("01 IMS-DLI-PCB."),
                statement(f"   05 IMS-DLI-DBD PIC X(8) VALUE '{dbd}'."),
                statement("   05 FILLER PIC X(2)."),
                statement("   05 IMS-DLI-STATUS PIC X(2)."),
                statement("   05 FILLER PIC X(44)."),
            ])
            injected = True
        if len(line) > 6 and line[6] in "*/":
            output.append(line)
            index += 1
            continue
        if not re.search(r"\bEXEC\s+DLI\b", code, re.I):
            output.append(line)
            index += 1
            continue
        start = index + 1
        block = []
        while index < len(lines):
            current = lines[index]
            if len(current) > 6 and current[6] in "*/":
                raise ValueError(f"line {index + 1}: comment inside EXEC DLI block")
            block.append(current[7:72].strip())
            index += 1
            if re.search(r"\bEND-EXEC\b", block[-1], re.I):
                break
        joined = " ".join(block)
        match = re.fullmatch(r"\s*EXEC\s+DLI\s+(\w+)\b(.*?)\bEND-EXEC\b(\.)?\s*", joined, re.I)
        if not match:
            raise ValueError(f"line {start}: unclosed or unsupported EXEC DLI block")
        output.extend(translate_command(match.group(1), match.group(2), start))
        if match.group(3):
            output.append(statement("."))
    if not injected:
        raise ValueError("source has no PROCEDURE DIVISION or LINKAGE SECTION "
                         "for DIBSTAT insertion")
    return "".join(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--dbd", required=True)
    args = parser.parse_args()
    try:
        result = translate(args.input.read_text(encoding="utf-8"), args.dbd)
    except ValueError as error:
        parser.error(str(error))
    args.output.write_text(result, encoding="utf-8")


if __name__ == "__main__":
    main()
