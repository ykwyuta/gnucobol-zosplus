#!/usr/bin/env python3
"""Translate a strict Bank of Z CICS subset to the local worker ABI.

Input and output are fixed-format COBOL. Every unsupported CICS command or
option fails with the line of the original EXEC CICS block.
"""

import argparse
import pathlib
import re
import sys


class CicsError(Exception):
    pass


START = re.compile(r"^\s*EXEC\s+CICS\b", re.I)
MAP = re.compile(r"^01\s+([A-Z0-9-]+)O\s+REDEFINES\s+\1I\.$", re.I)
FIELD = re.compile(r"^0[23]\s+([A-Z0-9-]+)O\s+PIC\s+", re.I)
OPTION = re.compile(
    r"([A-Z][A-Z0-9-]*)(?:\s*\(\s*("
    r"'[^']*'|[0-9]+|(?:LENGTH\s+OF\s+)?"
    r"[A-Z][A-Z0-9-]*(?:\s+OF\s+[A-Z][A-Z0-9-]*)*)\s*\))?", re.I)
DFHRESP = re.compile(r"DFHRESP\(([A-Z][A-Z0-9-]*)\)", re.I)
DFHVALUE = re.compile(r"DFHVALUE\(([A-Z][A-Z0-9-]*)\)", re.I)
RESP_CODES = {"NORMAL": 0, "MAPFAIL": 36, "NOTFND": 13,
              "INVREQ": 16, "NOTFINISHED": 113, "QIDERR": 44,
              "ITEMERR": 26, "LENGERR": 22, "NOSPACE": 18,
              "DUPREC": 14, "IOERR": 17}
VALUE_CODES = {"UCTRAN": 450, "NOUCTRAN": 451, "TRANIDONLY": 452,
               "NORMAL": 1016, "ABEND": 900, "SECERROR": 1214}


def find_end(code):
    """Locate END-EXEC outside quoted COBOL literals."""
    quote = None
    index = 0
    while index < len(code):
        char = code[index]
        if quote:
            if char == quote:
                if index + 1 < len(code) and code[index + 1] == quote:
                    index += 2
                    continue
                quote = None
        elif char in "'\"":
            quote = char
        elif (code[index:index + 8].upper() == "END-EXEC" and
              (index == 0 or not (code[index - 1].isalnum() or code[index - 1] == "-")) and
              (index + 8 == len(code) or not (code[index + 8].isalnum() or code[index + 8] == "-"))):
            return index
        index += 1
    return None


class MapDefinitions(dict):
    def __init__(self):
        super().__init__()
        self.extended = {}


def maps_from_copybook(copybook):
    maps = MapDefinitions()
    current = None
    occurs = 1
    last_field = None
    for line in copybook.splitlines():
        code = line[7:72].strip().upper() if len(line) >= 7 else line.strip().upper()
        match = MAP.fullmatch(code)
        if match:
            current = match.group(1)
            if current in maps:
                raise CicsError(f"duplicate symbolic map: {current}")
            maps[current] = []
            occurs = 1
            last_field = None
        elif code.startswith("01 "):
            current = None
            last_field = None
        elif current:
            group = re.fullmatch(r"02\s+([A-Z0-9-]+)G\s+OCCURS\s+([0-9]+)\.", code)
            if group:
                occurs = int(group.group(2))
                if not 1 <= occurs <= 99:
                    raise CicsError(f"invalid OCCURS count in {current}")
                continue
            match = FIELD.match(code)
            if match:
                if code.startswith("02 "):
                    occurs = 1
                last_field = match.group(1)
                maps[current].append((last_field, occurs))
            elif last_field:
                ext = re.fullmatch(r"0[23]\s+([A-Z0-9-]+)\s+PIC\s+X\.", code)
                if ext and ext.group(1) in (last_field + "C", last_field + "H"):
                    maps.extended.setdefault((current, last_field), set()).add(
                        ext.group(1)[-1])
    if not maps or any(not fields for fields in maps.values()):
        raise CicsError("copybook contains no symbolic output fields")
    return maps


def parse_options(text):
    options = {}
    pos = 0
    while pos < len(text):
        while pos < len(text) and text[pos].isspace():
            pos += 1
        if pos == len(text):
            break
        match = OPTION.match(text, pos)
        if not match:
            raise CicsError(f"invalid CICS option near {text[pos:pos+20]!r}")
        key = match.group(1).upper()
        if key in options:
            raise CicsError(f"duplicate CICS option: {key}")
        options[key] = match.group(2)
        pos = match.end()
        if pos < len(text) and not text[pos].isspace():
            raise CicsError(f"invalid CICS option near {text[pos:pos+20]!r}")
    return options


def identifier(value, option):
    if value is None or not re.fullmatch(
            r"[A-Z][A-Z0-9-]*(?:\s+OF\s+[A-Z][A-Z0-9-]*)*",
            value, re.I):
        raise CicsError(f"{option} requires a COBOL data name")
    return " ".join(value.upper().split())


def map_name(value):
    if value is None or not re.fullmatch(r"'[A-Z][A-Z0-9-]*'", value, re.I):
        raise CicsError("MAP requires a quoted literal name")
    return value[1:-1].upper()


def rewrite_dfhresp(line, number):
    """Expand supported DFHRESP constants in fixed-format code, outside literals."""
    if "DFHRESP" not in line.upper() and "DFHVALUE" not in line.upper():
        return line
    code = line[7:72]
    result = []
    quote = None
    index = 0
    while index < len(code):
        char = code[index]
        if quote:
            result.append(char)
            if char == quote:
                if index + 1 < len(code) and code[index + 1] == quote:
                    result.append(code[index + 1])
                    index += 1
                else:
                    quote = None
        elif char in "'\"":
            quote = char
            result.append(char)
        else:
            match = DFHRESP.match(code, index) or DFHVALUE.match(code, index)
            if match:
                name = match.group(1).upper()
                values = RESP_CODES if match.re is DFHRESP else VALUE_CODES
                if name not in values:
                    macro = "DFHRESP" if match.re is DFHRESP else "DFHVALUE"
                    raise CicsError(f"{number}: unsupported {macro} constant: {name}")
                result.append(str(values[name]))
                index = match.end()
                continue
            result.append(char)
        index += 1
    rewritten = "".join(result)
    if len(rewritten) > 65:
        raise CicsError(f"{number}: rewritten COBOL line exceeds column 72")
    suffix = line[72:].rstrip("\r\n")
    newline = "\r\n" if line.endswith("\r\n") else "\n" if line.endswith("\n") else ""
    if suffix:
        return line[:7] + rewritten.ljust(65) + suffix + newline
    return line[:7] + rewritten.rstrip() + newline


def response_targets(options):
    lines = []
    for option, eib in (("RESP", "EIBRESP"), ("RESP2", "EIBRESP2")):
        if option in options:
            lines.append(f"           MOVE {eib} TO "
                         f"{identifier(options[option], option)}\n")
    return lines


def statement(body, maps, period, expected_mapset=None, program_id=None,
              abend_label=None, mapfail_label=None):
    body = body.strip()
    aid_handler = re.fullmatch(r"HANDLE\s+AID\b(.*)", body, re.I | re.S)
    if aid_handler:
        options = parse_options(aid_handler.group(1))
        if not options or set(options) - {"CLEAR", "PF3"}:
            raise CicsError("HANDLE AID supports CLEAR and PF3")
        lines = []
        for aid, code in (("CLEAR", "6D"), ("PF3", "F3")):
            if aid in options:
                label = identifier(options[aid], aid)
                lines += [f"           IF EIBAID = X'{code}'\n",
                          f"             GO TO {label}\n",
                          "           END-IF\n"]
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        return lines
    condition_handler = re.fullmatch(r"HANDLE\s+CONDITION\b(.*)", body, re.I | re.S)
    if condition_handler:
        options = parse_options(condition_handler.group(1))
        if set(options) != {"MAPFAIL"}:
            raise CicsError("HANDLE CONDITION supports MAPFAIL only")
        identifier(options["MAPFAIL"], "MAPFAIL")
        return ["           CONTINUE" + (".\n" if period else "\n")]
    if not body:
        raise CicsError("empty EXEC CICS")
    handling = re.fullmatch(r"HANDLE\s+ABEND\b(.*)", body, re.I | re.S)
    if handling:
        options = parse_options(handling.group(1))
        if len(options) != 1 or not set(options) <= {"LABEL", "CANCEL", "RESET"}:
            raise CicsError("HANDLE ABEND requires LABEL, CANCEL or RESET")
        if "LABEL" in options:
            label = identifier(options["LABEL"], "LABEL")
            if abend_label and label != abend_label:
                raise CicsError("HANDLE ABEND LABEL does not match exit")
            lines = ["           MOVE 1 TO COBCICS-HANDLE-ABEND\n"]
        elif "CANCEL" in options:
            if options["CANCEL"] is not None:
                raise CicsError("HANDLE ABEND CANCEL takes no value")
            lines = ["           MOVE 0 TO COBCICS-HANDLE-ABEND\n"]
        else:
            if options["RESET"] is not None or not abend_label:
                raise CicsError("HANDLE ABEND RESET needs an earlier LABEL")
            lines = ["           MOVE 1 TO COBCICS-HANDLE-ABEND\n"]
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        return lines
    deedit = re.fullmatch(r"BIF\s+DEEDIT\b(.*)", body, re.I | re.S)
    if deedit:
        options = parse_options(deedit.group(1))
        unsupported = set(options) - {"FIELD", "LENGTH"}
        if unsupported:
            raise CicsError(f"unsupported BIF DEEDIT option: {sorted(unsupported)[0]}")
        field = identifier(options.get("FIELD"), "FIELD")
        length = options.get("LENGTH")
        if "LENGTH" in options and (length is None or not re.fullmatch(
                r"[A-Z][A-Z0-9-]*|[0-9]+", length, re.I)):
            raise CicsError("invalid BIF DEEDIT LENGTH")
        lines = [f"           MOVE LENGTH OF {field}\n",
                 "             TO COBCICS-LINK-CAPACITY\n"]
        if length is None:
            lines += [f"           MOVE LENGTH OF {field}\n",
                      "             TO COBCICS-LENGTH\n"]
        else:
            lines.append(f"           MOVE {length} TO COBCICS-LENGTH\n")
        lines += ['           CALL STATIC "COBCICS_DEEDIT"\n',
                  f"             USING {field} COBCICS-LENGTH\n",
                  "                   COBCICS-LINK-CAPACITY" +
                  (".\n" if period else "\n")]
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError("generated BIF DEEDIT line exceeds column 72")
        return lines
    abend = re.fullmatch(r"ABEND\b(.*)", body, re.I | re.S)
    if abend:
        options = parse_options(abend.group(1))
        unsupported = set(options) - {"ABCODE", "NODUMP", "CANCEL"}
        if unsupported:
            raise CicsError(f"unsupported ABEND option: {sorted(unsupported)[0]}")
        for flag in ("NODUMP", "CANCEL"):
            if flag in options and options[flag] is not None:
                raise CicsError(f"{flag} does not take a value")
        raw = options.get("ABCODE")
        if raw is None:
            if "ABCODE" in options:
                raise CicsError("ABCODE requires a 1-4 character literal or data name")
            code = '"USER"'
            length_lines = ["           MOVE 4 TO COBCICS-LENGTH\n"]
        else:
            if raw.startswith("'"):
                if not re.fullmatch(r"'[A-Z0-9 ]{1,4}'", raw, re.I):
                    raise CicsError("ABCODE requires a 1-4 character literal")
                code = f'"{raw[1:-1].upper():<4}"'
                length_lines = ["           MOVE 4 TO COBCICS-LENGTH\n"]
            else:
                code = identifier(raw, "ABCODE")
                length_lines = [f"           MOVE LENGTH OF {code}\n",
                                "             TO COBCICS-LENGTH\n"]
        if abend_label and "CANCEL" not in options:
            lines = [f"           MOVE {code} TO COBCICS-ABEND-CODE\n",
                     "           IF COBCICS-HANDLE-ABEND = 1\n",
                     "               MOVE 0 TO COBCICS-HANDLE-ABEND\n",
                     f"               GO TO {abend_label}\n",
                     "           END-IF\n"]
        else:
            lines = []
        lines += length_lines + ['           CALL STATIC "COBCICS_ABEND"\n',
                                 f"             USING {code} COBCICS-LENGTH\n",
                                 "           GOBACK" + (".\n" if period else "\n")]
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError("generated ABEND line exceeds column 72")
        return lines
    asktime = re.fullmatch(r"ASKTIME\b(.*)", body, re.I | re.S)
    if asktime:
        options = parse_options(asktime.group(1))
        unsupported = set(options) - {"ABSTIME", "RESP", "RESP2", "NOHANDLE"}
        if unsupported:
            raise CicsError(f"unsupported ASKTIME option: {sorted(unsupported)[0]}")
        area = identifier(options.get("ABSTIME"), "ABSTIME")
        lines = [f"           MOVE LENGTH OF {area}\n",
                 "             TO COBCICS-LENGTH\n",
                 '           CALL STATIC "COBCICS_ASKTIME"\n',
                 f"             USING {area} COBCICS-LENGTH\n",
                 "           MOVE ZERO TO EIBRESP EIBRESP2\n"]
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError("generated ASKTIME line exceeds column 72")
        return lines
    formattime = re.fullmatch(r"FORMATTIME\b(.*)", body, re.I | re.S)
    if formattime:
        options = parse_options(formattime.group(1))
        short_date = "YYDDD" if "YYDDD" in options else (
            "YYMMDD" if "YYMMDD" in options else None)
        if short_date:
            allowed = {"ABSTIME", short_date, "TIME", "RESP", "RESP2"}
            if short_date == "YYDDD":
                allowed.add("MILLISECONDS")
            unsupported = set(options) - allowed
            if unsupported:
                raise CicsError(f"unsupported FORMATTIME option: {sorted(unsupported)[0]}")
            abstime = identifier(options.get("ABSTIME"), "ABSTIME")
            date = identifier(options.get(short_date), short_date)
            clock = identifier(options.get("TIME"), "TIME")
            routine = ("COBCICS_FORMATTIME_YYDDD" if short_date == "YYDDD"
                       else "COBCICS_FORMATTIME_YYMMDD")
            lines = [f"           MOVE LENGTH OF {abstime}\n",
                     "             TO COBCICS-LENGTH\n",
                     f"           MOVE LENGTH OF {date}\n",
                     "             TO COBCICS-DATE-CAPACITY\n",
                     f"           MOVE LENGTH OF {clock}\n",
                     "             TO COBCICS-TIME-CAPACITY\n",
                     f'           CALL STATIC "{routine}"\n',
                     f"             USING {abstime} COBCICS-LENGTH\n",
                     f"                   {date}\n",
                     "                   COBCICS-DATE-CAPACITY\n",
                     f"                   {clock}\n",
                     "                   COBCICS-TIME-CAPACITY\n"]
            if short_date == "YYDDD":
                milli = identifier(options.get("MILLISECONDS"), "MILLISECONDS")
                lines.append(f"                   {milli}\n")
            lines.append("                   EIBRESP EIBRESP2\n")
            if "RESP" not in options and "RESP2" not in options:
                lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                          '             USING EIBRESP\n']
            lines += response_targets(options)
            if period:
                lines[-1] = lines[-1].rstrip() + ".\n"
            return lines
        unsupported = set(options) - {"ABSTIME", "DDMMYYYY", "MMDDYYYY", "MMDDYY", "TIME", "DATESEP", "TIMESEP", "NOHANDLE", "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported FORMATTIME option: {sorted(unsupported)[0]}")
        date_formats = {"DDMMYYYY", "MMDDYYYY", "MMDDYY"} & set(options)
        if len(date_formats) != 1:
            raise CicsError("FORMATTIME requires exactly one date format")
        abstime = identifier(options.get("ABSTIME"), "ABSTIME")
        date_format = next(iter(date_formats))
        date = identifier(options.get(date_format), date_format)
        clock = (identifier(options["TIME"], "TIME") if "TIME" in options
                 else "COBCICS-FORMATTIME-CLOCK")
        separator = "X'00'"
        if "DATESEP" in options:
            raw = options["DATESEP"]
            if raw is None:
                separator = '"/"'
            elif re.fullmatch(r"'[ -~]'", raw):
                separator = f'"{raw[1]}"'
            else:
                raise CicsError("DATESEP requires a single character literal")
        time_separator = None
        if "TIMESEP" in options:
            raw = options["TIMESEP"]
            if raw is None:
                time_separator = '":"'
            elif re.fullmatch(r"'[ -~]'", raw):
                time_separator = f'"{raw[1]}"'
            else:
                raise CicsError("TIMESEP requires a single character literal")
        routine = "COBCICS_FORMATTIME" + (
            "_MMDDYY" if date_format == "MMDDYY" else
            "_MMDD" if date_format == "MMDDYYYY" else "")
        if time_separator is not None:
            routine += "_TIMESEP"
        lines = [f"           MOVE LENGTH OF {abstime}\n",
                 "             TO COBCICS-LENGTH\n",
                 f"           MOVE LENGTH OF {date}\n",
                 "             TO COBCICS-DATE-CAPACITY\n",
                 f"           MOVE LENGTH OF {clock}\n",
                 "             TO COBCICS-TIME-CAPACITY\n",
                 f'           CALL STATIC "{routine}"\n',
                 f"             USING {abstime} COBCICS-LENGTH\n",
                 f"                   {date}\n",
                 "                   COBCICS-DATE-CAPACITY\n",
                 f"                   {clock}\n",
                 "                   COBCICS-TIME-CAPACITY\n",
                 f"                   {separator}\n"]
        if time_separator is not None:
            lines.append(f"                   {time_separator}\n")
        lines.append("                   EIBRESP EIBRESP2\n")
        if "RESP" not in options and "RESP2" not in options and "NOHANDLE" not in options:
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError("generated FORMATTIME line exceeds column 72")
        return lines
    returning = re.fullmatch(r"RETURN\b(.*)", body, re.I | re.S)
    if returning:
        options = parse_options(returning.group(1))
        unsupported = set(options) - {"TRANSID", "COMMAREA", "LENGTH", "IMMEDIATE", "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported RETURN option: {sorted(unsupported)[0]}")
        if "IMMEDIATE" in options and options["IMMEDIATE"] is not None:
            raise CicsError("RETURN IMMEDIATE does not take a value")
        lines = []
        if "TRANSID" in options:
            raw = options["TRANSID"]
            if raw is None:
                raise CicsError("TRANSID requires a literal or 4-byte area")
            if re.fullmatch(r"'[A-Z0-9]{1,4}'", raw, re.I):
                transid = f'"{raw[1:-1].upper():<4}"'
            else:
                transid = identifier(raw, "TRANSID")
            if "LENGTH" in options and "COMMAREA" not in options:
                raise CicsError("RETURN LENGTH requires COMMAREA")
            area = identifier(options["COMMAREA"], "COMMAREA") if "COMMAREA" in options else "COBCICS-COMMAREA"
            length = options.get("LENGTH", f"LENGTH OF {area}" if "COMMAREA" in options else "0")
            if length is None or not re.fullmatch(
                    r"[A-Z][A-Z0-9-]*|[0-9]+|LENGTH OF [A-Z][A-Z0-9-]*",
                    length, re.I):
                raise CicsError("invalid RETURN LENGTH")
            if length.isdigit() and int(length) > 2048:
                raise CicsError("RETURN COMMAREA exceeds 2048 bytes")
            lines += [f"           MOVE {length} TO COBCICS-LENGTH\n",
                      '           CALL STATIC "COBCICS_SET_RETURN"\n',
                      f"             USING {transid} {area} COBCICS-LENGTH\n"]
            if "IMMEDIATE" in options:
                lines.append('           CALL STATIC "COBCICS_SET_IMMEDIATE"\n')
        elif options:
            if set(options) - {"RESP", "RESP2"}:
                raise CicsError("RETURN options require TRANSID")
        lines += ["           MOVE ZERO TO EIBRESP EIBRESP2\n"]
        lines += response_targets(options)
        lines += ['           CALL STATIC "COBCICS_RETURN"\n',
                  "           GOBACK" + (".\n" if period else "\n")]
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError("generated RETURN line exceeds column 72")
        return lines
    terminal = re.fullmatch(r"(INQUIRE|SET)\s+TERMINAL\b(.*)",
                            body, re.I | re.S)
    if terminal:
        verb = terminal.group(1).upper()
        options = parse_options("TERMINAL" + terminal.group(2))
        unsupported = set(options) - {"TERMINAL", "UCTRANST", "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported {verb} TERMINAL option: {sorted(unsupported)[0]}")
        termid = identifier(options.get("TERMINAL"), "TERMINAL")
        value = identifier(options.get("UCTRANST"), "UCTRANST")
        if termid != "EIBTRMID":
            raise CicsError("TERMINAL currently requires EIBTRMID")
        lines = [f'           CALL STATIC "COBCICS_{verb}_TERMINAL"\n',
                 f"             USING {termid} {value}\n",
                 "                   EIBRESP EIBRESP2\n"]
        if "RESP" not in options and "RESP2" not in options:
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        return lines
    association = re.fullmatch(r"INQUIRE\s+ASSOCIATION\b(.*)",
                               body, re.I | re.S)
    if association:
        options = parse_options("ASSOCIATION" + association.group(1))
        required = ("ASSOCIATION", "ODAPPLID", "ODUSERID", "ODFACILNAME",
                    "ODNETWORKID", "ODFACILTYPE")
        unsupported = set(options) - set(required)
        if unsupported or set(options) != set(required):
            raise CicsError("INQUIRE ASSOCIATION requires Bank origin fields")
        if identifier(options["ASSOCIATION"], "ASSOCIATION") != "EIBTASKN":
            raise CicsError("ASSOCIATION currently requires EIBTASKN")
        values = [identifier(options[name], name) for name in required[1:]]
        lines = ['           CALL STATIC "COBCICS_INQUIRE_ASSOCIATION"\n']
        lines += [f"             USING {values[0]} {values[1]}\n",
                  f"                   {values[2]} {values[3]}\n",
                  f"                   {values[4]}\n"]
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError("generated INQUIRE ASSOCIATION line exceeds column 72")
        return lines
    container = re.fullmatch(r"(GET|PUT)\s+CONTAINER\b(.*)",
                             body, re.I | re.S)
    if container:
        verb = container.group(1).upper()
        options = parse_options("CONTAINER" + container.group(2))
        data_option = "INTO" if verb == "GET" else "FROM"
        unsupported = set(options) - {"CONTAINER", "CHANNEL", data_option,
                                      "FLENGTH", "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported {verb} CONTAINER option: {sorted(unsupported)[0]}")
        name = identifier(options.get("CONTAINER"), "CONTAINER")
        channel = (identifier(options["CHANNEL"], "CHANNEL")
                   if "CHANNEL" in options else "COBCICS-CURRENT-CHANNEL")
        data = identifier(options.get(data_option), data_option)
        length = (identifier(options["FLENGTH"], "FLENGTH")
                  if "FLENGTH" in options else "COBCICS-DATA-LENGTH")
        lines = [f"           MOVE LENGTH OF {data}\n",
                 "             TO COBCICS-DATA-CAPACITY\n"]
        if "FLENGTH" not in options:
            lines += ["           MOVE COBCICS-DATA-CAPACITY\n",
                      "             TO COBCICS-DATA-LENGTH\n"]
        lines += [
                 f'           CALL STATIC "COBCICS_{verb}_CONTAINER"\n',
                 f"             USING {name}\n",
                 f"                   {data} {length}\n",
                 "                   COBCICS-DATA-CAPACITY\n",
                 f"                   {channel}\n",
                 "                   EIBRESP EIBRESP2\n"]
        if "RESP" not in options and "RESP2" not in options:
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError(f"generated {verb} CONTAINER line exceeds column 72")
        return lines
    running = re.fullmatch(r"RUN\b(.*)", body, re.I | re.S)
    if running:
        options = parse_options(running.group(1))
        unsupported = set(options) - {"TRANSID", "CHANNEL", "CHILD", "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported RUN option: {sorted(unsupported)[0]}")
        transid = identifier(options.get("TRANSID"), "TRANSID")
        channel = identifier(options.get("CHANNEL"), "CHANNEL")
        child = identifier(options.get("CHILD"), "CHILD")
        lines = ['           CALL STATIC "COBCICS_RUN_ENTER"\n',
                 f"             USING {transid} {channel}\n",
                 f"                   {child} COBCICS-CHILD-PROGRAM\n",
                 "                   EIBRESP EIBRESP2\n",
                 "           IF EIBRESP = ZERO\n",
                 "               CALL COBCICS-CHILD-PROGRAM\n",
                 '               CALL STATIC "COBCICS_RUN_EXIT"\n',
                 "           END-IF\n"]
        if "RESP" not in options and "RESP2" not in options:
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        return lines
    fetching = re.fullmatch(r"FETCH\b(.*)", body, re.I | re.S)
    if fetching:
        options = parse_options(fetching.group(1))
        required = {"ANY", "CHANNEL", "NOSUSPEND", "COMPSTATUS", "ABCODE"}
        unsupported = set(options) - required - {"RESP", "RESP2"}
        if unsupported or not required <= set(options):
            raise CicsError("FETCH requires ANY CHANNEL NOSUSPEND COMPSTATUS ABCODE")
        if options["NOSUSPEND"] is not None:
            raise CicsError("FETCH NOSUSPEND takes no value")
        values = [identifier(options[name], name)
                  for name in ("ANY", "CHANNEL", "COMPSTATUS", "ABCODE")]
        lines = ['           CALL STATIC "COBCICS_FETCH_ANY"\n',
                 f"             USING {values[0]} {values[1]}\n",
                 f"                   {values[2]} {values[3]}\n",
                 "                   EIBRESP EIBRESP2\n"]
        if "RESP" not in options and "RESP2" not in options:
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError("generated FETCH line exceeds column 72")
        return lines
    assign = re.fullmatch(r"ASSIGN\b(.*)", body, re.I | re.S)
    if assign:
        options = parse_options(assign.group(1))
        unsupported = set(options) - {"APPLID", "PROGRAM", "ABCODE",
                                      "SYSID", "STARTCODE", "INVOKINGPROG",
                                      "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported ASSIGN option: {sorted(unsupported)[0]}")
        if not {"APPLID", "PROGRAM", "ABCODE", "SYSID", "STARTCODE",
                "INVOKINGPROG"} & set(options):
            raise CicsError("ASSIGN requires an output option")
        lines = ["           MOVE ZERO TO EIBRESP EIBRESP2\n"]
        if "ABCODE" in options:
            target = identifier(options["ABCODE"], "ABCODE")
            lines.append(f"           MOVE COBCICS-ABEND-CODE TO {target}\n")
        for option in ("APPLID", "PROGRAM", "SYSID", "STARTCODE",
                       "INVOKINGPROG"):
            if option not in options:
                continue
            target = identifier(options[option], option)
            if option == "PROGRAM":
                if program_id is None or len(program_id) > 8:
                    raise CicsError("ASSIGN PROGRAM requires a 1-8 character PROGRAM-ID")
                lines += [f"           MOVE LENGTH OF {target}\n",
                          "             TO COBCICS-LENGTH\n",
                          '           CALL STATIC "COBCICS_ASSIGN_PROGRAM"\n',
                          f'             USING {target} COBCICS-LENGTH\n',
                          f'                   "{program_id:<8}"\n']
            else:
                lines += [f"           MOVE LENGTH OF {target}\n",
                          "             TO COBCICS-LENGTH\n",
                          f'           CALL STATIC "COBCICS_ASSIGN_{option}"\n',
                          f"             USING {target} COBCICS-LENGTH\n"]
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError("generated ASSIGN line exceeds column 72")
        return lines
    counter = re.fullmatch(r"(DEFINE|DELETE|GET|QUERY)\s+COUNTER\b(.*)",
                           body, re.I | re.S)
    if counter:
        verb = counter.group(1).upper()
        options = parse_options("COUNTER" + counter.group(2))
        allowed = {"COUNTER", "POOL", "RESP", "RESP2"}
        if verb != "DELETE":
            allowed.add("VALUE")
        unsupported = set(options) - allowed
        if unsupported:
            raise CicsError(f"unsupported {verb} COUNTER option: {sorted(unsupported)[0]}")
        name = identifier(options.get("COUNTER"), "COUNTER")
        pool = identifier(options.get("POOL"), "POOL")
        lines = [f"           MOVE {name} TO COBCICS-COUNTER-NAME\n",
                 f"           MOVE {pool} TO COBCICS-COUNTER-POOL\n"]
        if verb == "DELETE":
            value = "COBCICS-COUNTER-VALUE"
        elif verb == "DEFINE" and options.get("VALUE", "").isdigit():
            lines.append(f"           MOVE {options['VALUE']}\n")
            lines.append("             TO COBCICS-COUNTER-VALUE\n")
            value = "COBCICS-COUNTER-VALUE"
        else:
            value = identifier(options.get("VALUE"), "VALUE")
        lines += [f'           CALL STATIC "COBCICS_COUNTER_{verb}"\n',
                  "             USING COBCICS-COUNTER-NAME\n",
                  "                   COBCICS-COUNTER-POOL\n",
                  f"                   {value} EIBRESP EIBRESP2\n"]
        if "RESP" not in options and "RESP2" not in options:
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError(f"generated {verb} COUNTER line exceeds column 72")
        return lines
    queue = re.fullmatch(r"(READQ|WRITEQ|DELETEQ)\s+(TS|TD)\b(.*)",
                         body, re.I | re.S)
    if queue:
        verb, kind = queue.group(1).upper(), queue.group(2).upper()
        options = parse_options(queue.group(3))
        if kind == "TD" and verb != "WRITEQ":
            raise CicsError(f"unsupported {verb} TD")
        allowed = {"QUEUE", "RESP", "RESP2", "NOHANDLE"}
        if verb == "READQ":
            allowed |= {"INTO", "LENGTH", "ITEM", "NEXT"}
        elif verb == "WRITEQ":
            allowed |= {"FROM", "LENGTH"}
            if kind == "TS":
                allowed |= {"ITEM", "REWRITE", "NOSUSPEND"}
        unsupported = set(options) - allowed
        if unsupported:
            raise CicsError(f"unsupported {verb} {kind} option: {sorted(unsupported)[0]}")
        if "NOHANDLE" in options and options["NOHANDLE"] is not None:
            raise CicsError("NOHANDLE does not take a value")
        raw_name = options.get("QUEUE")
        if raw_name and re.fullmatch(r"'[A-Z0-9]{1,8}'", raw_name, re.I):
            name = f'"{raw_name[1:-1].upper():<8}"'
        else:
            name = identifier(raw_name, "QUEUE")
        lines = [f"           MOVE {name} TO COBCICS-QUEUE-NAME\n"]
        if verb == "DELETEQ":
            lines += ['           CALL STATIC "COBCICS_TSQ_DELETE"\n',
                      "             USING COBCICS-QUEUE-NAME\n",
                      "                   EIBRESP EIBRESP2\n"]
        else:
            direction = "INTO" if verb == "READQ" else "FROM"
            area = identifier(options.get(direction), direction)
            lines += [f"           MOVE LENGTH OF {area}\n",
                      "             TO COBCICS-QUEUE-CAPACITY\n"]
            raw_length = options.get("LENGTH")
            if raw_length is None:
                lines += ["           MOVE COBCICS-QUEUE-CAPACITY\n",
                          "             TO COBCICS-QUEUE-LENGTH\n"]
            elif raw_length.isdigit() or re.fullmatch(
                    r"LENGTH\s+OF\s+[A-Z][A-Z0-9-]*", raw_length, re.I):
                lines += [f"           MOVE {raw_length}\n",
                          "             TO COBCICS-QUEUE-LENGTH\n"]
            else:
                raw_length = identifier(raw_length, "LENGTH")
                lines += [f"           MOVE {raw_length}\n",
                          "             TO COBCICS-QUEUE-LENGTH\n"]
            length = "COBCICS-QUEUE-LENGTH"
            if verb == "READQ" and "ITEM" in options and "NEXT" in options:
                raise CicsError("READQ TS cannot use ITEM and NEXT together")
            for flag in ("NEXT", "REWRITE", "NOSUSPEND"):
                if flag in options and options[flag] is not None:
                    raise CicsError(f"{flag} does not take a value")
            raw_item = options.get("ITEM", "0")
            if raw_item is None or not re.fullmatch(r"[0-9]+|[A-Z][A-Z0-9-]*", raw_item, re.I):
                raise CicsError("ITEM requires a number or data name")
            if kind == "TS":
                lines += [f"           MOVE {raw_item} TO COBCICS-QUEUE-ITEM\n"]
                if verb == "READQ":
                    mode = "01" if "NEXT" in options else "00"
                    lines += ['           CALL STATIC "COBCICS_TSQ_READ"\n',
                              f"             USING COBCICS-QUEUE-NAME {area}\n",
                              f"                   {length}\n",
                              "                   COBCICS-QUEUE-CAPACITY\n",
                              "                   COBCICS-QUEUE-ITEM\n",
                              f"                   X'{mode}' EIBRESP EIBRESP2\n"]
                else:
                    mode = "01" if "REWRITE" in options else "00"
                    lines += ['           CALL STATIC "COBCICS_TSQ_WRITE"\n',
                              f"             USING COBCICS-QUEUE-NAME {area}\n",
                              f"                   {length}\n",
                              "                   COBCICS-QUEUE-CAPACITY\n",
                              "                   COBCICS-QUEUE-ITEM\n",
                              f"                   X'{mode}' EIBRESP EIBRESP2\n"]
            else:
                lines += ['           CALL STATIC "COBCICS_TDQ_WRITE"\n',
                          f"             USING COBCICS-QUEUE-NAME {area}\n",
                          f"                   {length}\n",
                          "                   COBCICS-QUEUE-CAPACITY\n",
                          "                   EIBRESP EIBRESP2\n"]
            if verb == "READQ" and raw_length and not raw_length.isdigit() and not re.fullmatch(
                    r"LENGTH\s+OF\s+[A-Z][A-Z0-9-]*", raw_length, re.I):
                lines += ["           MOVE COBCICS-QUEUE-LENGTH\n",
                          f"             TO {raw_length}\n"]
        if ("RESP" not in options and "RESP2" not in options and
                "NOHANDLE" not in options):
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError(f"generated {verb} {kind} line exceeds column 72")
        return lines
    keyed_file = re.fullmatch(r"(READ|WRITE|REWRITE|DELETE)\b(.*)",
                              body, re.I | re.S)
    if keyed_file:
        verb = keyed_file.group(1).upper()
        options = parse_options(keyed_file.group(2))
        if "DATASET" in options:
            if "FILE" in options:
                raise CicsError(f"{verb} cannot specify FILE and DATASET")
            options["FILE"] = options.pop("DATASET")
        if not (verb == "WRITE" and
                options.get("FILE", "").upper() == "'ABNDFILE'"):
            allowed = {"FILE", "LENGTH", "RESP", "RESP2"}
            if verb == "READ":
                allowed |= {"INTO", "RIDFLD", "KEYLENGTH", "GTEQ",
                            "GENERIC", "UPDATE"}
            elif verb == "WRITE":
                allowed |= {"FROM", "RIDFLD", "KEYLENGTH"}
            elif verb == "REWRITE":
                allowed.add("FROM")
            else:
                allowed = {"FILE", "RIDFLD", "KEYLENGTH", "RESP", "RESP2"}
            unsupported = set(options) - allowed
            if unsupported:
                raise CicsError(f"unsupported {verb} FILE option: {sorted(unsupported)[0]}")
            file_name = options.get("FILE", "").upper()
            if re.fullmatch(r"'[A-Z0-9]{1,8}'", file_name):
                source_name = f'"{file_name[1:-1]}"'
            else:
                source_name = identifier(file_name, "FILE")
            lines = [f"           MOVE {source_name}\n",
                     "             TO COBCICS-FILE-NAME\n"]
            if verb != "DELETE":
                direction = "INTO" if verb == "READ" else "FROM"
                area = identifier(options.get(direction), direction)
                raw_length = options.get("LENGTH")
                if raw_length is None or not re.fullmatch(
                        r"[0-9]+|[A-Z][A-Z0-9-]*(?:\s+OF\s+[A-Z][A-Z0-9-]*)*"
                        r"|LENGTH\s+OF\s+[A-Z][A-Z0-9-]*", raw_length, re.I):
                    raise CicsError(f"{verb} FILE requires valid LENGTH")
                lines += [f"           MOVE {raw_length}\n",
                          "             TO COBCICS-FILE-LENGTH\n",
                          f"           MOVE LENGTH OF {area}\n",
                          "             TO COBCICS-FILE-CAPACITY\n"]
            if verb != "REWRITE":
                key = identifier(options.get("RIDFLD"), "RIDFLD")
                raw_key_length = options.get("KEYLENGTH")
                if raw_key_length is None or not re.fullmatch(
                        r"[0-9]+|[A-Z][A-Z0-9-]*|LENGTH\s+OF\s+[A-Z][A-Z0-9-]*",
                        raw_key_length, re.I):
                    raise CicsError(f"{verb} FILE requires KEYLENGTH")
                lines += [f"           MOVE {raw_key_length}\n",
                          "             TO COBCICS-FILE-KEY-LENGTH\n",
                          f"           MOVE LENGTH OF {key}\n",
                          "             TO COBCICS-FILE-KEY-CAPACITY\n"]
            if verb == "READ":
                for flag in ("GTEQ", "GENERIC", "UPDATE"):
                    if flag in options and options[flag] is not None:
                        raise CicsError(f"{flag} does not take a value")
                if "GENERIC" in options and "GTEQ" not in options:
                    raise CicsError("READ FILE GENERIC requires GTEQ")
                if "UPDATE" in options and "GTEQ" in options:
                    raise CicsError("READ FILE UPDATE cannot use GTEQ")
                mode = ((1 if "GTEQ" in options else 0) |
                        (2 if "GENERIC" in options else 0) |
                        (4 if "UPDATE" in options else 0))
                lines += ['           CALL STATIC "COBCICS_FILE_READ"\n',
                          "             USING COBCICS-FILE-NAME\n",
                          f"                   {area} COBCICS-FILE-LENGTH\n",
                          "                   COBCICS-FILE-CAPACITY\n",
                          f"                   {key}\n",
                          "                   COBCICS-FILE-KEY-LENGTH\n",
                          "                   COBCICS-FILE-KEY-CAPACITY\n",
                          f"                   X'{mode:02X}' EIBRESP EIBRESP2\n"]
                if not raw_length.isdigit() and not raw_length.upper().startswith("LENGTH OF "):
                    lines += ["           MOVE COBCICS-FILE-LENGTH\n",
                              f"             TO {raw_length}\n"]
            elif verb == "WRITE":
                lines += ['           CALL STATIC "COBCICS_FILE_WRITE"\n',
                          "             USING COBCICS-FILE-NAME\n",
                          f"                   {area} COBCICS-FILE-LENGTH\n",
                          "                   COBCICS-FILE-CAPACITY\n",
                          f"                   {key}\n",
                          "                   COBCICS-FILE-KEY-LENGTH\n",
                          "                   COBCICS-FILE-KEY-CAPACITY\n",
                          "                   EIBRESP EIBRESP2\n"]
            elif verb == "REWRITE":
                lines += ['           CALL STATIC "COBCICS_FILE_REWRITE"\n',
                          "             USING COBCICS-FILE-NAME\n",
                          f"                   {area} COBCICS-FILE-LENGTH\n",
                          "                   COBCICS-FILE-CAPACITY\n",
                          "                   EIBRESP EIBRESP2\n"]
            else:
                lines += ['           CALL STATIC "COBCICS_FILE_DELETE"\n',
                          "             USING COBCICS-FILE-NAME\n",
                          f"                   {key}\n",
                          "                   COBCICS-FILE-KEY-LENGTH\n",
                          "                   COBCICS-FILE-KEY-CAPACITY\n",
                          "                   EIBRESP EIBRESP2\n"]
            if "RESP" not in options and "RESP2" not in options:
                lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                          '             USING EIBRESP\n']
            lines += response_targets(options)
            if period:
                lines[-1] = lines[-1].rstrip() + ".\n"
            if any(len(line.rstrip("\n")) > 72 for line in lines):
                raise CicsError(f"generated {verb} FILE line exceeds column 72")
            return lines
    writing = re.fullmatch(r"WRITE\b(.*)", body, re.I | re.S)
    if writing:
        options = parse_options(writing.group(1))
        unsupported = set(options) - {"FILE", "FROM", "RIDFLD", "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported WRITE option: {sorted(unsupported)[0]}")
        if options.get("FILE", "").upper() != "'ABNDFILE'":
            raise CicsError("WRITE currently supports FILE('ABNDFILE')")
        area = identifier(options.get("FROM"), "FROM")
        key = identifier(options.get("RIDFLD"), "RIDFLD")
        lines = [f"           MOVE LENGTH OF {area}\n",
                 "             TO COBCICS-LENGTH\n",
                 f"           MOVE LENGTH OF {key}\n",
                 "             TO COBCICS-LINK-CAPACITY\n",
                 '           CALL STATIC "COBCICS_WRITE_ABNDFILE"\n',
                 f"             USING {area} COBCICS-LENGTH\n",
                 f"                   {key} COBCICS-LINK-CAPACITY\n",
                 "                   EIBRESP EIBRESP2\n"]
        if "RESP" not in options and "RESP2" not in options:
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError("generated WRITE line exceeds column 72")
        return lines
    enqueue = re.fullmatch(r"(ENQ|DEQ)\b(.*)", body, re.I | re.S)
    if enqueue:
        verb = enqueue.group(1).upper()
        options = parse_options(enqueue.group(2))
        unsupported = set(options) - {"RESOURCE", "LENGTH", "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported {verb} option: {sorted(unsupported)[0]}")
        resource = identifier(options.get("RESOURCE"), "RESOURCE")
        length = options.get("LENGTH", f"LENGTH OF {resource}")
        if length is None or not re.fullmatch(r"[0-9]+|[A-Z][A-Z0-9-]*|LENGTH OF [A-Z][A-Z0-9-]*", length, re.I):
            raise CicsError(f"invalid {verb} LENGTH")
        lines = [f"           MOVE {length} TO COBCICS-LENGTH\n",
                 f"           MOVE LENGTH OF {resource}\n",
                 "             TO COBCICS-LINK-CAPACITY\n",
                 f'           CALL STATIC "COBCICS_{verb}"\n',
                 f"             USING {resource} COBCICS-LENGTH\n",
                 "                   COBCICS-LINK-CAPACITY\n",
                 "                   EIBRESP EIBRESP2\n"]
        if "RESP" not in options and "RESP2" not in options:
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        return lines
    retrieve = re.fullmatch(r"RETRIEVE\b(.*)", body, re.I | re.S)
    if retrieve:
        options = parse_options(retrieve.group(1))
        unsupported = set(options) - {"INTO", "RESP", "RESP2", "NOHANDLE"}
        if unsupported:
            raise CicsError(f"unsupported RETRIEVE option: {sorted(unsupported)[0]}")
        area = identifier(options.get("INTO"), "INTO")
        lines = [f"           MOVE LENGTH OF {area}\n",
                 "             TO COBCICS-DATA-CAPACITY\n",
                 '           CALL STATIC "COBCICS_RETRIEVE"\n',
                 f"             USING {area} COBCICS-DATA-CAPACITY\n",
                 "                   EIBRESP EIBRESP2\n"]
        if "RESP" not in options and "RESP2" not in options and "NOHANDLE" not in options:
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        return lines
    syncpoint = re.fullmatch(r"SYNCPOINT\b(.*)", body, re.I | re.S)
    if syncpoint:
        options = parse_options(syncpoint.group(1))
        unsupported = set(options) - {"ROLLBACK", "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported SYNCPOINT option: {sorted(unsupported)[0]}")
        if "ROLLBACK" in options and options["ROLLBACK"] is not None:
            raise CicsError("SYNCPOINT ROLLBACK takes no value")
        mode = "01" if "ROLLBACK" in options else "00"
        lines = ['           CALL STATIC "COBCICS_SYNCPOINT"\n',
                 f"             USING X'{mode}' EIBRESP EIBRESP2\n"]
        if "RESP" not in options and "RESP2" not in options:
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        return lines
    starting = re.fullmatch(r"START\s+TRANSID\b(.*)", body, re.I | re.S)
    if starting:
        options = parse_options("TRANSID" + starting.group(1))
        unsupported = set(options) - {"TRANSID", "AFTER", "MINUTES",
                                      "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported START option: {sorted(unsupported)[0]}")
        transid = options.get("TRANSID", "").upper()
        minutes = options.get("MINUTES")
        if (not re.fullmatch(r"'[A-Z0-9]{1,4}'", transid) or
                options.get("AFTER", "bad") is not None or
                minutes is None or not re.fullmatch(
                    r"[0-9]+|[A-Z][A-Z0-9-]*", minutes, re.I)):
            raise CicsError("START requires TRANSID AFTER MINUTES")
        lines = [f"           MOVE {minutes} TO COBCICS-START-MINUTES\n",
                 '           CALL STATIC "COBCICS_START_AFTER"\n',
                 f'             USING "{transid[1:-1]:<4}"\n',
                 "                   COBCICS-START-MINUTES\n",
                 "                   EIBRESP EIBRESP2\n"]
        if "RESP" not in options and "RESP2" not in options:
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        return lines
    delay = re.fullmatch(r"DELAY\b(.*)", body, re.I | re.S)
    if delay:
        options = parse_options(delay.group(1))
        unsupported = set(options) - {"FOR", "SECONDS", "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported DELAY option: {sorted(unsupported)[0]}")
        if options.get("FOR") is not None or "FOR" not in options:
            raise CicsError("DELAY requires FOR SECONDS")
        seconds = options.get("SECONDS")
        if seconds is None or not re.fullmatch(r"[0-9]+|[A-Z][A-Z0-9-]*", seconds, re.I):
            raise CicsError("DELAY SECONDS requires a data name or 0-60")
        if seconds.isdigit() and int(seconds) > 60:
            raise CicsError("DELAY SECONDS requires 0-60")
        lines = [f"           MOVE {seconds} TO COBCICS-LENGTH\n",
                 '           CALL STATIC "COBCICS_DELAY"\n',
                 '             USING COBCICS-LENGTH\n',
                 '           MOVE ZERO TO EIBRESP EIBRESP2\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        return lines
    linking = re.fullmatch(r"(LINK|XCTL)\b(.*)", body, re.I | re.S)
    if linking:
        verb = linking.group(1).upper()
        options = parse_options(linking.group(2))
        allowed = {"PROGRAM", "COMMAREA", "LENGTH", "RESP", "RESP2",
                   "NOHANDLE"}
        if verb == "LINK":
            allowed.add("SYNCONRETURN")
        unsupported = set(options) - allowed
        if unsupported:
            raise CicsError(f"unsupported {verb} option: {sorted(unsupported)[0]}")
        if options.get("SYNCONRETURN") is not None:
            raise CicsError("SYNCONRETURN does not take a value")
        if options.get("NOHANDLE") is not None:
            raise CicsError("NOHANDLE does not take a value")
        if "PROGRAM" not in options or "COMMAREA" not in options:
            raise CicsError(f"{verb} requires PROGRAM and COMMAREA")
        raw_program = options["PROGRAM"]
        if raw_program is None:
            raise CicsError("PROGRAM requires a COBOL name or literal")
        if raw_program.startswith("'"):
            if not re.fullmatch(r"'[A-Z][A-Z0-9- ]{0,7}'", raw_program, re.I):
                raise CicsError("PROGRAM requires a 1-8 character name")
            program = f'"{raw_program[1:-1].strip().upper()}"'
        else:
            program = identifier(raw_program, "PROGRAM")
        area = identifier(options["COMMAREA"], "COMMAREA")
        length = options.get("LENGTH")
        if length is not None:
            if not re.fullmatch(r"(?:LENGTH\s+OF\s+)?[A-Z][A-Z0-9-]*(?:\s+OF\s+[A-Z][A-Z0-9-]*)*|[0-9]+", length, re.I):
                raise CicsError("invalid LINK LENGTH")
            if length.isdigit() and int(length) > 32767:
                raise CicsError("LINK COMMAREA exceeds 32767 bytes")
            length_lines = [f"           MOVE {length} TO COBCICS-LENGTH\n"]
        else:
            length_lines = [f"           MOVE LENGTH OF {area}\n",
                            "             TO COBCICS-LENGTH\n"]
        lines = [f"           MOVE LENGTH OF {area}\n",
                 "             TO COBCICS-LINK-CAPACITY\n"]
        lines += length_lines
        lines += ['           CALL STATIC "COBCICS_LINK_ENTER"\n',
                  '             USING COBCICS-LENGTH\n',
                  '                   COBCICS-LINK-CAPACITY\n',
                  '           CALL STATIC "COBCICS_LINK_CALLER"\n',
                  f'             USING "{(program_id or ""):<8}"\n',
                  f"           CALL {program} USING {area}\n",
                  '           CALL STATIC "COBCICS_LINK_EXIT"\n',
                  "           MOVE ZERO TO EIBRESP EIBRESP2\n"]
        if verb == "XCTL":
            lines += ['           CALL STATIC "COBCICS_RETURN"\n',
                      "           GOBACK\n"]
        else:
            lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError("generated LINK line exceeds column 72")
        return lines
    control = re.fullmatch(r"SEND\s+CONTROL\b(.*)", body, re.I | re.S)
    if control:
        options = parse_options(control.group(1))
        unsupported = set(options) - {"ERASE", "FREEKB", "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported SEND CONTROL option: {sorted(unsupported)[0]}")
        if "ERASE" not in options or "FREEKB" not in options:
            raise CicsError("SEND CONTROL currently requires ERASE FREEKB")
        if options["ERASE"] is not None or options["FREEKB"] is not None:
            raise CicsError("ERASE and FREEKB do not take values")
        lines = ["           MOVE ZERO TO EIBRESP EIBRESP2\n"]
        lines += response_targets(options)
        lines.append('           CALL STATIC "COBCICS_SEND_CONTROL"'
                     + (".\n" if period else "\n"))
        return lines
    send_text = re.fullmatch(r"SEND\s+TEXT\b(.*)", body, re.I | re.S)
    if send_text:
        options = parse_options("TEXT" + send_text.group(1))
        unsupported = set(options) - {"TEXT", "FROM", "LENGTH", "ERASE",
                                      "FREEKB", "WAIT", "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported SEND TEXT option: {sorted(unsupported)[0]}")
        if options.get("TEXT") is not None:
            raise CicsError("SEND TEXT does not take a value")
        if "FROM" not in options:
            raise CicsError("SEND TEXT requires FROM")
        area = identifier(options["FROM"], "FROM")
        if "ERASE" not in options or "FREEKB" not in options:
            raise CicsError("SEND TEXT currently requires ERASE FREEKB")
        if options["ERASE"] is not None or options["FREEKB"] is not None:
            raise CicsError("ERASE and FREEKB do not take values")
        if "WAIT" in options and options["WAIT"] is not None:
            raise CicsError("WAIT does not take a value")
        length = options.get("LENGTH")
        if "LENGTH" in options and (length is None or not re.fullmatch(
                r"(?:LENGTH\s+OF\s+)?[A-Z][A-Z0-9-]*(?:\s+OF\s+[A-Z][A-Z0-9-]*)*|[0-9]+", length, re.I)):
            raise CicsError("invalid SEND TEXT LENGTH")
        if length and length.isdigit() and int(length) > 1919:
            raise CicsError("SEND TEXT exceeds 1919 bytes")
        lines = [f"           MOVE LENGTH OF {area}\n",
                 "             TO COBCICS-TEXT-CAPACITY\n"]
        if length is None:
            lines += [f"           MOVE LENGTH OF {area}\n",
                      "             TO COBCICS-LENGTH\n"]
        else:
            lines.append(f"           MOVE {length} TO COBCICS-LENGTH\n")
        lines += ["           MOVE ZERO TO EIBRESP EIBRESP2\n"]
        lines += response_targets(options)
        lines += ['           CALL STATIC "COBCICS_SEND_TEXT"\n',
                  f"             USING {area} COBCICS-LENGTH\n",
                  "                   COBCICS-TEXT-CAPACITY" +
                  (".\n" if period else "\n")]
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError("generated SEND TEXT line exceeds column 72")
        return lines
    receive_data = re.fullmatch(r"RECEIVE\b(?!\s+MAP)(.*)",
                                body, re.I | re.S)
    if receive_data:
        options = parse_options(receive_data.group(1))
        unsupported = set(options) - {"INTO", "LENGTH", "RESP", "RESP2"}
        if unsupported:
            raise CicsError(f"unsupported RECEIVE option: {sorted(unsupported)[0]}")
        area = identifier(options.get("INTO"), "INTO")
        length = identifier(options.get("LENGTH"), "LENGTH")
        lines = [f"           MOVE LENGTH OF {area}\n",
                 "             TO COBCICS-LINK-CAPACITY\n",
                 '           CALL STATIC "COBCICS_RECEIVE_INTO"\n',
                 f"             USING {area} {length}\n",
                 "                   COBCICS-LINK-CAPACITY\n",
                 "                   EIBRESP EIBRESP2\n"]
        if "RESP" not in options and "RESP2" not in options:
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
        lines += response_targets(options)
        if period:
            lines[-1] = lines[-1].rstrip() + ".\n"
        if any(len(line.rstrip("\n")) > 72 for line in lines):
            raise CicsError("generated RECEIVE INTO line exceeds column 72")
        return lines
    match = re.match(r"(SEND|RECEIVE)\s+MAP\b(.*)\Z", body, re.I | re.S)
    if not match:
        raise CicsError(f"unsupported EXEC CICS command: {body.strip()}")
    verb = match.group(1).upper()
    options = parse_options("MAP" + match.group(2))
    allowed = {"MAP", "MAPSET", "RESP", "RESP2", "NOHANDLE",
               "FROM" if verb == "SEND" else "INTO"}
    if verb == "SEND":
        allowed |= {"ERASE", "DATAONLY", "ALARM", "FREEKB", "CURSOR", "MAPONLY"}
    else:
        allowed |= {"TERMINAL", "ASIS"}
    unsupported = set(options) - allowed
    if unsupported:
        raise CicsError(f"unsupported {verb} MAP option: {sorted(unsupported)[0]}")
    if "NOHANDLE" in options and options["NOHANDLE"] is not None:
        raise CicsError("NOHANDLE does not take a value")
    name = map_name(options.get("MAP"))
    if (name not in maps and verb == "SEND" and "MAPONLY" in options and
            expected_mapset and name == expected_mapset.upper() and len(maps) == 1):
        name = next(iter(maps))
    if (name not in maps and verb == "SEND" and "MAPONLY" in options and
            name.endswith("M") and name[:-1] in maps):
        name = name[:-1]
    if name not in maps:
        raise CicsError(f"symbolic map not found: {name}")
    target = "FROM" if verb == "SEND" else "INTO"
    if target in options and identifier(options[target], target) != name + ("O" if verb == "SEND" else "I"):
        raise CicsError(f"{target} must name {name}{'O' if verb == 'SEND' else 'I'}")
    if "MAPSET" in options and not re.fullmatch(r"'[A-Z][A-Z0-9-]*'", options["MAPSET"], re.I):
        raise CicsError("MAPSET requires a quoted literal name")
    if "MAPSET" in options and expected_mapset and options["MAPSET"][1:-1].upper() != expected_mapset.upper():
        raise CicsError(f"MAPSET does not match {expected_mapset}")
    if verb == "SEND":
        for flag in ("ERASE", "DATAONLY", "ALARM", "FREEKB", "CURSOR", "MAPONLY"):
            if flag in options and options[flag] is not None:
                raise CicsError(f"{flag} does not take a value")
        if "MAPONLY" in options and "DATAONLY" in options:
            raise CicsError("MAPONLY and DATAONLY cannot be combined")
        if "ERASE" in options and "DATAONLY" in options:
            raise CicsError("ERASE and DATAONLY cannot be combined")
        if "ALARM" in options and "DATAONLY" not in options:
            raise CicsError("ALARM currently requires DATAONLY")
    else:
        for flag in ("TERMINAL", "ASIS"):
            if flag in options and options[flag] is not None:
                raise CicsError(f"{flag} does not take a value")
    lines = []
    if verb == "RECEIVE":
        mode = "01" if "ASIS" in options else "00"
        lines += ['           CALL STATIC "COBCICS_SET_RECEIVE_ASIS"\n',
                  f"             USING X'{mode}'\n",
                  '           CALL STATIC "COBCICS_RECEIVE_STATUS"\n',
                  '             USING EIBRESP EIBRESP2\n']
        if mapfail_label and "NOHANDLE" not in options:
            lines += ["           IF EIBRESP = 36\n",
                      f"             GO TO {mapfail_label}\n",
                      "           END-IF\n"]
        if ("RESP" not in options and "RESP2" not in options and
                "NOHANDLE" not in options):
            lines += ['           CALL STATIC "COBCICS_REQUIRE_NORMAL"\n',
                      '             USING EIBRESP\n']
    else:
        lines.append("           MOVE ZERO TO EIBRESP EIBRESP2\n")
    lines += response_targets(options)
    if verb == "RECEIVE":
        lines.append("           IF EIBRESP = ZERO\n")
    else:
        action = 3 if "ALARM" in options else 2 if "DATAONLY" in options else 0
        lines += ['           CALL STATIC "COBCICS_SEND_MAP"\n',
                  f"             USING X'{action:02X}'\n"]
    for field, count in ([] if "MAPONLY" in options else maps[name]):
        if len(field) > 8:
            raise CicsError(f"field name exceeds pipe ABI: {field}")
        for index in range(1, count + 1):
            wire = field if count == 1 else f"{field[:4]:<4}{index:04d}"
            literal = f'"{wire:<8}"'
            suffix = "" if count == 1 else f"({index})"
            if verb == "RECEIVE":
                lines += [f"           MOVE LENGTH OF {field}I{suffix}\n",
                          f"             TO {field}L{suffix}\n",
                          '           CALL STATIC "COBCICS_GET_FIELD"\n',
                          f"             USING {literal}\n",
                          f"                   {field}I{suffix}\n",
                          f"                   {field}L{suffix}\n",
                          f"                   {field}F{suffix}\n"]
            else:
                lines += [f"           MOVE LENGTH OF {field}O{suffix}\n",
                          "             TO COBCICS-LENGTH\n",
                          '           CALL STATIC "COBCICS_SEND_FIELD"\n',
                          f"             USING {literal}\n",
                          f"                   {field}O{suffix}\n",
                          "                   COBCICS-LENGTH\n",
                          '           CALL STATIC "COBCICS_SEND_ATTRIBUTE"\n',
                          f"             USING {literal}\n",
                          f"                   {field}A{suffix}\n"]
                for attribute in sorted(getattr(maps, "extended", {}).get((name, field), ())):
                    lines += ['           CALL STATIC "COBCICS_SEND_EXT_ATTRIBUTE"\n',
                              f"             USING {literal}\n",
                              f"                   X'{ord(attribute):02X}'\n",
                              f"                   {field}{attribute}{suffix}\n"]
    if verb == "RECEIVE":
        lines.append("           END-IF\n")
    if period:
        lines[-1] = lines[-1].rstrip() + ".\n"
    if any(len(line.rstrip("\n")) > 72 for line in lines):
        raise CicsError(f"generated {verb} MAP line exceeds column 72")
    return lines


def transform(source, maps, expected_mapset=None):
    lines = source.splitlines(keepends=True)
    active_code = " ".join(line[7:72] for line in lines
                           if len(line) >= 7 and line[6] not in "*/Dd")
    handle_labels = re.findall(
        r"EXEC\s+CICS\s+HANDLE\s+ABEND\s+LABEL\s*\(\s*"
        r"([A-Z][A-Z0-9-]*)\s*\)\s*END-EXEC", active_code, re.I)
    if len({label.upper() for label in handle_labels}) > 1:
        raise CicsError("multiple HANDLE ABEND labels are not supported")
    abend_label = handle_labels[0].upper() if handle_labels else None
    if abend_label and not re.search(
            rf"(?im)^.{{7}}\s*{re.escape(abend_label)}\s+(?:SECTION\s*)?\.", source):
        raise CicsError(f"HANDLE ABEND label {abend_label} is not defined")
    if not re.search(r"(?im)^.{7}\s*WORKING-STORAGE\s+SECTION\s*\.", source):
        raise CicsError("WORKING-STORAGE SECTION is required")
    procedure_match = re.search(
        r"(?im)^.{7}\s*PROCEDURE\s+DIVISION"
        r"(?:\s+USING\s+([A-Z][A-Z0-9-]*))?\s*\.", source)
    if not procedure_match:
        raise CicsError("PROCEDURE DIVISION. is required")
    linkage_area = procedure_match.group(1)
    if linkage_area and linkage_area.upper() != "DFHCOMMAREA":
        raise CicsError("linked PROCEDURE DIVISION must use DFHCOMMAREA")
    # CICS exposes DFHCOMMAREA implicitly to some LINK programs. Their
    # source omits USING, but the generated COBOL CALL must bind that area.
    implicit_linkage = (not linkage_area and
        re.search(r"(?im)^.{7}\s*LINKAGE\s+SECTION\s*\.", source) and
        re.search(r"(?im)^.{7}\s*01\s+DFHCOMMAREA\b", source) and
        not re.search(r"\bRECEIVE\s+MAP\b", active_code, re.I))
    if implicit_linkage:
        linkage_area = "DFHCOMMAREA"
    program_match = re.search(
        r"(?im)^.{7}\s*PROGRAM-ID\s*\.\s*([A-Z][A-Z0-9-]*)\s*\.",
        source)
    program_id = program_match.group(1).upper() if program_match else None
    result = []
    in_cics = False
    body = []
    start_line = 0
    mapfail_label = None
    for number, line in enumerate(lines, 1):
        if len(line) >= 7 and line[6] in "*/Dd":
            if not in_cics:
                result.append(line)
            continue
        code = line[7:72] if len(line) >= 7 else line
        if not in_cics:
            if not linkage_area and re.match(r"\s*LINKAGE\s+SECTION\s*\.", code, re.I):
                result.append("      * CICS COMMAREA storage for terminal task.\n")
                continue
            if re.match(r"\s*WORKING-STORAGE\s+SECTION\s*\.", code, re.I):
                result.append(line)
                result.append("       01 COBCICS-LENGTH PIC S9(4) COMP.\n")
                result.append("       01 COBCICS-LINK-CAPACITY\n")
                result.append("          PIC S9(4) COMP.\n")
                result.append("       01 COBCICS-TEXT-CAPACITY\n")
                result.append("          PIC S9(4) COMP.\n")
                result.append("       01 COBCICS-DATE-CAPACITY\n")
                result.append("          PIC S9(4) COMP.\n")
                result.append("       01 COBCICS-TIME-CAPACITY\n")
                result.append("          PIC S9(4) COMP.\n")
                result.append("       01 COBCICS-FORMATTIME-CLOCK PIC X(8).\n")
                result.append("       01 COBCICS-DATA-CAPACITY PIC S9(8) COMP.\n")
                result.append("       01 COBCICS-DATA-LENGTH PIC S9(8) COMP.\n")
                result.append('       01 COBCICS-CURRENT-CHANNEL PIC X(16)\n')
                result.append('          VALUE "DFHEP.EVENT".\n')
                result.append("       01 COBCICS-QUEUE-CAPACITY PIC S9(8) COMP.\n")
                result.append("       01 COBCICS-QUEUE-LENGTH PIC S9(8) COMP.\n")
                result.append("       01 COBCICS-QUEUE-ITEM PIC S9(8) COMP.\n")
                result.append("       01 COBCICS-QUEUE-NAME PIC X(8).\n")
                result.append("       01 COBCICS-COUNTER-NAME PIC X(16).\n")
                result.append("       01 COBCICS-COUNTER-POOL PIC X(8).\n")
                result.append("       01 COBCICS-COUNTER-VALUE PIC S9(8) COMP.\n")
                result.append("       01 COBCICS-FILE-NAME PIC X(8).\n")
                result.append("       01 COBCICS-FILE-LENGTH PIC S9(8) COMP.\n")
                result.append("       01 COBCICS-FILE-CAPACITY PIC S9(8) COMP.\n")
                result.append("       01 COBCICS-FILE-KEY-LENGTH\n")
                result.append("          PIC S9(8) COMP.\n")
                result.append("       01 COBCICS-FILE-KEY-CAPACITY\n")
                result.append("          PIC S9(8) COMP.\n")
                result.append("       01 COBCICS-START-MINUTES\n")
                result.append("          PIC S9(8) COMP.\n")
                result.append("       01 COBCICS-CHILD-PROGRAM PIC X(8).\n")
                result.append("       01 COBCICS-COMMAREA PIC X(2048).\n")
                result.append("       01 COBCICS-HANDLE-ABEND PIC 9 VALUE ZERO.\n")
                result.append("       01 COBCICS-ABEND-CODE PIC X(4) VALUE SPACES.\n")
                if not re.search(r"(?im)^.{7}\s*01\s+EIBAID\b", source):
                    result.append("       01 EIBAID PIC X.\n")
                if not re.search(r"(?im)^.{7}\s*01\s+EIBCALEN\b", source):
                    result.append("       01 EIBCALEN PIC S9(4) COMP.\n")
                if not re.search(r"(?im)^.{7}\s*01\s+EIBTRNID\b", source):
                    result.append("       01 EIBTRNID PIC X(4).\n")
                if not re.search(r"(?im)^.{7}\s*01\s+EIBTRMID\b", source):
                    result.append("       01 EIBTRMID PIC X(4).\n")
                if not re.search(r"(?im)^.{7}\s*01\s+EIBTASKN\b", source):
                    result.append("       01 EIBTASKN PIC S9(7) COMP-3.\n")
                if not re.search(r"(?im)^.{7}\s*01\s+EIBRESP\b", source):
                    result.append("       01 EIBRESP PIC S9(8) COMP.\n")
                if not re.search(r"(?im)^.{7}\s*01\s+EIBRESP2\b", source):
                    result.append("       01 EIBRESP2 PIC S9(8) COMP.\n")
                continue
            if re.match(r"\s*PROCEDURE\s+DIVISION"
                        r"(?:\s+USING\s+DFHCOMMAREA)?\s*\.", code, re.I):
                result.append("       PROCEDURE DIVISION USING DFHCOMMAREA.\n"
                              if implicit_linkage else line)
                if linkage_area:
                    result.append('           CALL STATIC "COBCICS_LINK_START"\n')
                    result.append('             USING EIBAID EIBCALEN EIBTRNID.\n')
                else:
                    result.append('           CALL STATIC "COBCICS_START"\n')
                    result.append('             USING EIBAID EIBCALEN EIBTRNID\n')
                    result.append('           MOVE LENGTH OF COBCICS-COMMAREA\n')
                    result.append('             TO COBCICS-LENGTH\n')
                    result.append('           CALL STATIC "COBCICS_GET_COMMAREA"\n')
                    if re.search(r"(?im)^.{7}\s*01\s+DFHCOMMAREA\b", source):
                        result.append('             USING COBCICS-COMMAREA COBCICS-LENGTH\n')
                        result.append('           MOVE COBCICS-COMMAREA TO DFHCOMMAREA.\n')
                    else:
                        result.append('             USING COBCICS-COMMAREA COBCICS-LENGTH.\n')
                result.append('           MOVE "L001" TO EIBTRMID.\n')
                result.append('           CALL STATIC "COBCICS_TASK_NUMBER"\n')
                result.append('             USING EIBTASKN.\n')
                continue
            match = START.match(code)
            if not match:
                expanded = rewrite_dfhresp(line, number)
                expanded = re.sub(r'\bCALL\s+"(CEEDAYS|CEELOCT)"',
                                  lambda m: 'CALL STATIC "COBCICS_' +
                                  m.group(1).upper() + '"', expanded, flags=re.I)
                result.append(expanded)
                continue
            in_cics = True
            start_line = number
            code = code[match.end():]
        end = find_end(code)
        if end is not None:
            body.append(code[:end])
            tail = code[end + 8:].strip()
            if tail not in ("", "."):
                raise CicsError(f"{number}: text after END-EXEC")
            try:
                command = " ".join(body)
                handler = re.fullmatch(r"\s*HANDLE\s+CONDITION\b(.*)",
                                       command, re.I | re.S)
                if handler:
                    options = parse_options(handler.group(1))
                    if set(options) == {"MAPFAIL"}:
                        mapfail_label = identifier(options["MAPFAIL"], "MAPFAIL")
                result.extend(statement(command, maps, tail == ".",
                                        expected_mapset, program_id,
                                        abend_label, mapfail_label))
            except CicsError as error:
                raise CicsError(f"{start_line}: {error}") from error
            in_cics = False
            body = []
        else:
            body.append(code.rstrip())
    if in_cics:
        raise CicsError(f"{start_line}: missing END-EXEC")
    return "".join(result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=pathlib.Path)
    parser.add_argument("copybook", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    args = parser.parse_args()
    try:
        maps = maps_from_copybook(args.copybook.read_text(encoding="utf-8"))
        transformed = transform(args.source.read_text(encoding="utf-8"), maps,
                                args.copybook.stem)
        args.output.write_text(transformed, encoding="utf-8")
    except (OSError, UnicodeError, CicsError) as error:
        print(f"{args.source}: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
