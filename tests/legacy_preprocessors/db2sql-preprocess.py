#!/usr/bin/env python3
"""Translate supported fixed-format EXEC SQL to ordinary GnuCOBOL calls.

The compiler driver invokes this stage only with --db2-sql. Unsupported SQL
is an error, never silently removed.
"""

import argparse
import pathlib
import re
import sys


class SqlError(Exception):
    pass


TOKEN = re.compile(
    r"\s+|--[^\n]*|/\*.*?\*/|'(?:''|[^'])*'|\"(?:\"\"|[^\"])*\"|"
    r":[A-Za-z][A-Za-z0-9_-]*|[A-Za-z_][A-Za-z0-9_-]*|"
    r"[0-9]+(?:\.[0-9]+)?|<=|>=|<>|!=|[^\s]",
    re.DOTALL,
)
HOST = re.compile(r":[A-Za-z][A-Za-z0-9_-]*\Z")
SQL_START = re.compile(r"^\s*EXEC\s+SQL\b", re.IGNORECASE)


def host_list(tokens):
    if not tokens:
        raise SqlError("INTO にはホスト変数が必要です")
    items = []
    current = []
    for token in tokens + [","]:
        if token == ",":
            if len(current) == 3 and current[1].upper() == "INDICATOR":
                current = [current[0], current[2]]
            if len(current) not in (1, 2) or \
                    any(not HOST.fullmatch(part) for part in current):
                raise SqlError("INTO のホスト変数一覧が不正です")
            items.append(tuple(part[1:] for part in current))
            current = []
        else:
            current.append(token)
    return items


def sql_parameters(tokens):
    inputs = []
    sql_tokens_out = []
    index = 0
    while index < len(tokens):
        token = tokens[index]
        if HOST.fullmatch(token):
            pair = [token[1:]]
            if index + 2 < len(tokens) and tokens[index + 1].upper() == "INDICATOR" \
                    and HOST.fullmatch(tokens[index + 2]):
                pair.append(tokens[index + 2][1:])
                index += 2
            elif index + 1 < len(tokens) and HOST.fullmatch(tokens[index + 1]):
                pair.append(tokens[index + 1][1:])
                index += 1
            inputs.append(tuple(pair))
            sql_tokens_out.append("?")
        else:
            sql_tokens_out.append(token)
        index += 1
    sql = " ".join(sql_tokens_out)
    if len(inputs) > 99 or len(sql) > 8192:
        raise SqlError("SQL またはホスト変数が上限を超えています")
    return sql, inputs


def metadata(inputs, outputs):
    flags = "".join("1" if len(item) == 2 else "0"
                    for item in inputs + outputs)
    return f"{len(inputs):02d},{len(outputs):02d}" + \
        (flags if "1" in flags else "")


def call_fields(items):
    return ["               " + name + "\n"
            for item in items for name in item]
def scan_sql_end(code, in_block_comment=False):
    """Find END-EXEC, retaining SQL block comment state between lines."""
    quote = None
    index = 0
    while index < len(code):
        char = code[index]
        if in_block_comment:
            if code.startswith("*/", index):
                in_block_comment = False
                index += 2
                continue
        elif quote:
            if char == quote:
                if index + 1 < len(code) and code[index + 1] == quote:
                    index += 2
                    continue
                quote = None
        elif code.startswith("--", index):
            return None, in_block_comment
        elif code.startswith("/*", index):
            in_block_comment = True
            index += 2
            continue
        elif char in "'\"":
            quote = char
        elif (code[index:index + 8].upper() == "END-EXEC" and
              (index == 0 or not (code[index - 1].isalnum() or code[index - 1] == "-")) and
              (index + 8 == len(code) or not (code[index + 8].isalnum() or code[index + 8] == "-"))):
            return index, in_block_comment
        index += 1
    return None, in_block_comment


def find_sql_end(code):
    return scan_sql_end(code)[0]


def sql_tokens(body):
    result = []
    pos = 0
    for match in TOKEN.finditer(body):
        if match.start() != pos:
            raise SqlError("SQL トークンを解析できません")
        token = match.group()
        pos = match.end()
        if not token.isspace() and not token.startswith(("--", "/*")):
            result.append(token)
    if pos != len(body):
        raise SqlError("SQL トークンを解析できません")
    return result


def cobol_lines(statement):
    """Emit Area B code within the 72-column fixed-format boundary."""
    words = statement.split(" ")
    result = []
    current = "           "
    for word in words:
        addition = word if current.endswith(" ") else " " + word
        if len(current + addition) > 72:
            result.append(current.rstrip() + "\n")
            current = "               " + word
        else:
            current += addition
    result.append(current.rstrip() + "\n")
    return result


def sql_literal_lines(sql):
    # A COBOL string literal cannot cross a fixed-format physical line.
    parts = []
    part = ""
    for char in sql:
        encoded = "''" if char == "'" else char
        if len(part) + len(encoded) > 44:
            parts.append(part)
            part = ""
        part += encoded
    if part:
        parts.append(part)
    lines = ["           MOVE SPACES TO DB2-SQL-TEXT\n",
             "           STRING\n"]
    for part in parts:
        lines.append("               '" + part + "'\n")
    lines.append("               INTO DB2-SQL-TEXT\n")
    lines.append("           END-STRING\n")
    return lines


def convert_sql(body, period, cursors=None, include_loader=None):
    tokens = sql_tokens(body)
    if not tokens:
        raise SqlError("空の EXEC SQL")
    head = tokens[0].upper()
    if head == "SET" and len(tokens) == 6 and HOST.fullmatch(tokens[1]) and \
            tokens[2] == "=" and [token.upper() for token in tokens[3:]] == \
            ["IDENTITY_VAL_LOCAL", "(", ")"]:
        return convert_sql("SELECT IDENTITY_VAL_LOCAL() INTO " + tokens[1] +
                           " FROM SYSIBM.SYSDUMMY1", period, cursors,
                           include_loader)
    if head == "INCLUDE":
        if len(tokens) != 2 or not re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]*",
                                                 tokens[1]):
            raise SqlError("INCLUDE のメンバー名が不正です")
        if tokens[1].upper() in ("SQLCA", "SQLDA"):
            return ["       COPY '" + tokens[1].lower() + ".cpy'.\n"]
        if include_loader is None:
            raise SqlError("SQL INCLUDE の探索ディレクトリが必要です")
        return include_loader(tokens[1])
    if head == "DECLARE" and any(token.upper() == "TABLE"
                                 for token in tokens[2:6]):
        return ["      * Db2 DECLARE TABLE: runtime code is not required\n"]
    if head == "DECLARE" and len(tokens) >= 5 and \
            re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]*", tokens[1]):
        index = 2
        if [token.upper() for token in tokens[index:index + 2]] == \
                ["INSENSITIVE", "SCROLL"]:
            index += 2
        if tokens[index].upper() != "CURSOR":
            raise SqlError("未対応の DECLARE CURSOR オプション")
        index += 1
        if [token.upper() for token in tokens[index:index + 2]] == \
                ["WITH", "HOLD"]:
            index += 2
        if len(tokens) <= index + 1 or tokens[index].upper() != "FOR":
            raise SqlError("DECLARE CURSOR には FOR SELECT が必要です")
        index += 1
        if cursors is None:
            raise SqlError("カーソル定義にはプログラム全体の変換が必要です")
        name = tokens[1].upper()
        if name in cursors:
            raise SqlError("カーソル名が重複しています: " + name)
        if tokens[index].upper() != "SELECT":
            raise SqlError("カーソルには SELECT が必要です")
        cursors[name] = sql_parameters(tokens[index:])
        return ["      * Db2 cursor " + name + " declared\n"]
    if head in ("COMMIT", "ROLLBACK"):
        if [t.upper() for t in tokens] not in ([head], [head, "WORK"]):
            raise SqlError(head + " の構文が不正です")
        return cobol_lines('CALL "DB2SQL_' + head +
                           '" USING SQLCODE SQLSTATE' +
                           ("." if period else ""))
    if head in ("OPEN", "CLOSE", "FETCH"):
        if cursors is None:
            raise SqlError("カーソル操作にはプログラム全体の変換が必要です")
        offset = 2 if head == "FETCH" and len(tokens) > 1 and \
            tokens[1].upper() == "FROM" else 1
        if len(tokens) <= offset:
            raise SqlError("カーソル名がありません")
        name = tokens[offset].upper()
        if name not in cursors:
            raise SqlError("未定義のカーソル: " + name)
        if head in ("OPEN", "CLOSE") and len(tokens) != offset + 1:
            raise SqlError(head + " の構文が不正です")
        result = ["           MOVE '" + name +
                  "' TO DB2-SQL-CURSOR\n"]
        if head == "OPEN":
            sql, inputs = cursors[name]
            result += sql_literal_lines(sql)
            result.append("           MOVE '" + metadata(inputs, []) +
                          "' TO DB2-SQL-META\n")
            result += cobol_lines('CALL "DB2SQL_OPEN" USING '
                                  'DB2-SQL-CURSOR DB2-SQL-TEXT '
                                  'DB2-SQL-META SQLCODE SQLSTATE')
            result += call_fields(inputs)
        elif head == "FETCH":
            if len(tokens) <= offset + 2 or tokens[offset + 1].upper() != "INTO":
                raise SqlError("FETCH ... INTO が必要です")
            outputs = host_list(tokens[offset + 2:])
            result.append("           MOVE '" + metadata([], outputs) +
                          "' TO DB2-SQL-META\n")
            result += cobol_lines('CALL "DB2SQL_FETCH" USING '
                                  'DB2-SQL-CURSOR DB2-SQL-META '
                                  'SQLCODE SQLSTATE')
            result += call_fields(outputs)
        else:
            result += cobol_lines('CALL "DB2SQL_CLOSE_CURSOR" USING '
                                  'DB2-SQL-CURSOR SQLCODE SQLSTATE')
        if period:
            result[-1] = result[-1].rstrip("\n") + ".\n"
        return result
    if head not in ("INSERT", "SELECT", "UPDATE", "DELETE"):
        raise SqlError("未対応の SQL 文: " + head)

    outputs = []
    if head == "SELECT":
        into = next((i for i, t in enumerate(tokens) if t.upper() == "INTO"), -1)
        from_index = next((i for i, t in enumerate(tokens) if t.upper() == "FROM"), -1)
        if into < 0 or from_index <= into + 1:
            raise SqlError("SELECT ... INTO ... FROM が必要")
        outputs = host_list(tokens[into + 1:from_index])
        tokens = tokens[:into] + tokens[from_index:]

    sql, inputs = sql_parameters(tokens)
    if len(outputs) > 99:
        raise SqlError("出力ホスト変数は 99 個まで")
    result = sql_literal_lines(sql)
    result.append("           MOVE '" + metadata(inputs, outputs) +
                  "' TO DB2-SQL-META\n")
    result += cobol_lines('CALL "DB2SQL_EXEC" USING DB2-SQL-TEXT '
                          'DB2-SQL-META SQLCODE SQLSTATE')
    # Continue the CALL on subsequent Area B lines; the last line carries
    # the source period, if present.
    result += call_fields(inputs + outputs)
    if period:
        result[-1] = result[-1].rstrip("\n") + ".\n"
    return result


def transform(source, include_dirs=(), include_stack=(), cursors=None,
              included=False):
    result = []
    if cursors is None:
        cursors = {}
    in_sql = False
    in_block_comment = False
    sql_body = []
    sql_line = 0
    have_working_storage = included

    def load_include(member):
        for directory in include_dirs:
            candidate = pathlib.Path(directory) / (member + ".cpy")
            if candidate.is_file():
                resolved = candidate.resolve()
                if resolved in include_stack:
                    raise SqlError("循環した SQL INCLUDE: " + member)
                rendered = transform(candidate.read_text(encoding="utf-8"),
                                     include_dirs, include_stack + (resolved,),
                                     cursors, included=True)
                return [rendered]
        raise SqlError("SQL INCLUDE が見つかりません: " + member)
    for lineno, line in enumerate(source.splitlines(keepends=True), 1):
        if len(line) >= 7 and line[6] in "*/":
            if not in_sql:
                result.append(line)
            continue
        code = line[7:72] if len(line) >= 7 else line
        if not in_sql:
            if re.search(r"\bWORKING-STORAGE\s+SECTION\s*\.", code, re.I):
                have_working_storage = True
                result.append(line)
                result.extend([
                    "       01 DB2-SQL-TEXT PIC X(8192).\n",
                    "       01 DB2-SQL-META PIC X(203).\n",
                    "       01 DB2-SQL-CURSOR PIC X(64).\n",
                ])
                continue
            match = SQL_START.search(code)
            if not match:
                result.append(line)
                continue
            in_sql = True
            in_block_comment = False
            sql_line = lineno
            code = code[match.end():]
        end, in_block_comment = scan_sql_end(code, in_block_comment)
        if end is not None:
            sql_body.append(code[:end])
            tail = code[end + 8:].strip()
            if tail not in ("", "."):
                raise SqlError(f"{lineno}: END-EXEC の後に余分な文字があります")
            if not have_working_storage:
                raise SqlError(f"{sql_line}: WORKING-STORAGE SECTION が必要です")
            try:
                result.extend(convert_sql("\n".join(sql_body), tail == ".",
                                          cursors, load_include))
            except SqlError as exc:
                raise SqlError(f"{sql_line}: {exc}") from exc
            in_sql = False
            sql_body = []
        else:
            sql_body.append(code.rstrip())
    if in_sql:
        raise SqlError(f"{sql_line}: END-EXEC がありません")
    return "".join(result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("-I", "--include-dir", action="append", default=[],
                        type=pathlib.Path)
    args = parser.parse_args()
    try:
        source = args.source.read_text(encoding="utf-8")
        rendered = transform(source, (args.source.parent, *args.include_dir),
                             (args.source.resolve(),))
        with args.output.open("w", encoding="utf-8", newline="") as output:
            output.write(rendered)
    except (OSError, UnicodeError, SqlError) as exc:
        print(f"{args.source}: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
