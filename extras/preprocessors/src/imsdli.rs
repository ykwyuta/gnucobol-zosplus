//! Fixed-format EXEC DLI translation for the CardDemo command subset.
use regex::Regex;
use std::sync::OnceLock;

fn pattern(source: &'static str, cell: &'static OnceLock<Regex>) -> &'static Regex {
    cell.get_or_init(|| Regex::new(source).expect("valid IMS DLI pattern"))
}

fn name(value: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    pattern(r"(?i)^[A-Z0-9]{1,8}$", &RE).is_match(value)
}

fn area_name(value: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    pattern(r"(?i)^[A-Z0-9-]+$", &RE).is_match(value)
}

fn line(text: &str) -> String {
    format!("           {text}\n")
}

fn parse_clauses(body: &str, line_number: usize) -> Result<Vec<(String, String)>, String> {
    static KEY: OnceLock<Regex> = OnceLock::new();
    static WORD: OnceLock<Regex> = OnceLock::new();
    let key_re = pattern(r"(?i)^([A-Z]+)\s*\(", &KEY);
    let word_re = pattern(r"(?i)^[A-Z]+", &WORD);
    let mut result = Vec::new();
    let mut pos = 0;
    while pos < body.len() {
        pos += body[pos..].len() - body[pos..].trim_start().len();
        if pos == body.len() {
            break;
        }
        let rest = &body[pos..];
        let Some(caps) = key_re.captures(rest) else {
            if let Some(word) = word_re.find(rest) {
                if matches!(
                    rest[word.start()..word.end()].to_ascii_uppercase().as_str(),
                    "USING" | "NODHABEND"
                ) {
                    pos += word.end();
                    continue;
                }
            }
            return Err(format!(
                "line {line_number}: unsupported EXEC DLI operand {rest:?}"
            ));
        };
        let whole = caps.get(0).unwrap();
        let key = caps[1].to_ascii_uppercase();
        if !matches!(
            key.as_str(),
            "PCB" | "SEGMENT" | "INTO" | "FROM" | "WHERE" | "SEGLENGTH" | "ID" | "PSB"
        ) {
            return Err(format!(
                "line {line_number}: unsupported EXEC DLI clause {key}"
            ));
        }
        let start = pos + whole.end();
        let mut depth = 1;
        let mut end = start;
        for (offset, byte) in body.as_bytes()[start..].iter().enumerate() {
            if *byte == b'(' {
                depth += 1;
            } else if *byte == b')' {
                depth -= 1;
            }
            end = start + offset + 1;
            if depth == 0 {
                break;
            }
        }
        if depth != 0 {
            return Err(format!("line {line_number}: unclosed {key} clause"));
        }
        result.push((key, body[start..end - 1].trim().to_string()));
        pos = end;
    }
    Ok(result)
}

fn ssa_lines(
    index: usize,
    segment: &str,
    where_clause: Option<&str>,
    number: usize,
) -> Result<String, String> {
    let target = format!("IMS-DLI-SSA-{index}");
    if !name(segment) {
        return Err(format!("line {number}: invalid segment name {segment:?}"));
    }
    let mut result = String::new();
    if let Some(where_clause) = where_clause {
        let Some((field, value)) = where_clause.split_once('=') else {
            return Err(format!("line {number}: unsupported WHERE {where_clause:?}"));
        };
        let (field, value) = (field.trim(), value.trim());
        if !name(field) || !area_name(value) {
            return Err(format!("line {number}: unsupported WHERE {where_clause:?}"));
        }
        result.push_str(&line(&format!("MOVE SPACES TO {target}")));
        result.push_str(&line(&format!(
            "STRING '{segment:<8}' '(' '{field:<8}' 'EQ'"
        )));
        result.push_str(&line(&format!(
            "    DELIMITED BY SIZE INTO {target} END-STRING"
        )));
        result.push_str(&line("IF IMS-DLI-RC = ZERO"));
        result.push_str(&line(&format!(
            "CALL STATIC 'IMSDSA' USING {target} {value}"
        )));
        result.push_str(&line("    RETURNING IMS-DLI-RC"));
        result.push_str(&line("END-IF"));
    } else {
        result.push_str(&line(&format!("MOVE '{segment:<8}' TO {target}")));
    }
    Ok(result)
}

fn translate_command(command: &str, body: &str, number: usize) -> Result<String, String> {
    use std::collections::HashMap;
    let operation = command.to_ascii_uppercase();
    let parts = parse_clauses(body, number)?;
    if matches!(operation.as_str(), "CHKP" | "SCHD" | "TERM") {
        let expected = match operation.as_str() {
            "CHKP" => "ID",
            "SCHD" => "PSB",
            _ => "",
        };
        for (key, _) in &parts {
            if key != expected {
                return Err(format!(
                    "line {number}: unsupported {operation} clause {key}"
                ));
            }
        }
        let mut arg = if operation == "TERM" {
            "SPACES".to_string()
        } else {
            String::new()
        };
        for (_, value) in parts {
            arg = value
                .trim()
                .trim_start_matches('(')
                .trim_end_matches(')')
                .trim()
                .to_string();
        }
        if arg.is_empty() {
            return Err(format!("line {number}: {operation} requires an argument"));
        }
        return Ok(format!(
            "{}{}{}{}",
            line(&format!("MOVE '{operation:<4}' TO IMS-DLI-FUNCTION")),
            line(&format!("MOVE {arg} TO IMS-DLI-ARG")),
            line("CALL 'IMSDEXEC' USING IMS-DLI-FUNCTION"),
            line("    IMS-DLI-ARG DIBSTAT")
        ));
    }
    if !matches!(
        operation.as_str(),
        "GU" | "GN" | "GNP" | "GHU" | "GHN" | "GHNP" | "ISRT" | "REPL" | "DLET"
    ) {
        return Err(format!(
            "line {number}: unsupported EXEC DLI command {operation}"
        ));
    }
    let mut values = HashMap::new();
    let mut segments = Vec::new();
    let mut where_by_segment = HashMap::new();
    for (key, value) in parts {
        match key.as_str() {
            "SEGMENT" => segments.push(value),
            "WHERE" => {
                if segments.is_empty() {
                    return Err(format!("line {number}: WHERE precedes SEGMENT"));
                }
                where_by_segment.insert(segments.len(), value);
            }
            _ => {
                if values.insert(key.clone(), value).is_some() {
                    return Err(format!("line {number}: repeated {key}"));
                }
            }
        }
    }
    if segments.is_empty() || segments.len() > 2 || !values.contains_key("PCB") {
        return Err(format!(
            "line {number}: expected PCB and one or two SEGMENT clauses"
        ));
    }
    if values["PCB"] != "PAUT-PCB-NUM" {
        return Err(format!("line {number}: unmapped PCB {}", values["PCB"]));
    }
    let area_key = if matches!(
        operation.as_str(),
        "GU" | "GN" | "GNP" | "GHU" | "GHN" | "GHNP"
    ) {
        "INTO"
    } else {
        "FROM"
    };
    if !values.contains_key(area_key)
        || values
            .keys()
            .any(|key| !matches!(key.as_str(), "PCB" | "INTO" | "FROM" | "SEGLENGTH"))
    {
        return Err(format!(
            "line {number}: missing {area_key} or unsupported clause"
        ));
    }
    if let Some(length) = values.get("SEGLENGTH") {
        if !length.eq_ignore_ascii_case(&format!("LENGTH OF {}", values[area_key])) {
            return Err(format!("line {number}: unsupported SEGLENGTH"));
        }
    }
    let call = match operation.as_str() {
        "GU" => "GHU",
        "GN" => "GHN",
        "GNP" => "GHNP",
        _ => &operation,
    };
    let mut result = line("MOVE ZERO TO IMS-DLI-RC");
    result.push_str(&line(&format!("MOVE '{call:<4}' TO IMS-DLI-FUNCTION")));
    for (index, segment) in segments.iter().enumerate() {
        result.push_str(&ssa_lines(
            index + 1,
            segment,
            where_by_segment.get(&(index + 1)).map(String::as_str),
            number,
        )?);
    }
    for instruction in [
        "IF IMS-DLI-RC = ZERO".to_string(),
        "CALL 'CBLTDLI' USING IMS-DLI-FUNCTION".into(),
        format!("    IMS-DLI-PCB {}", values[area_key]),
        format!(
            "    {}",
            (1..=segments.len())
                .map(|index| format!("IMS-DLI-SSA-{index}"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        "    RETURNING IMS-DLI-RC".into(),
        "END-IF".into(),
        "IF IMS-DLI-RC = 0".into(),
        "    MOVE IMS-DLI-STATUS TO DIBSTAT".into(),
        "ELSE".into(),
        "    MOVE 'AJ' TO DIBSTAT".into(),
        "END-IF".into(),
    ] {
        result.push_str(&line(&instruction));
    }
    Ok(result)
}

fn code(line: &str) -> &str {
    line.get(7..line.len().min(72)).unwrap_or("")
}

pub fn translate(source: &str, dbd: &str) -> Result<String, String> {
    static EXEC: OnceLock<Regex> = OnceLock::new();
    static END: OnceLock<Regex> = OnceLock::new();
    static INJECT: OnceLock<Regex> = OnceLock::new();
    static BLOCK: OnceLock<Regex> = OnceLock::new();
    let exec = pattern(r"(?i)\bEXEC\s+DLI\b", &EXEC);
    let end = pattern(r"(?i)\bEND-EXEC\b", &END);
    let inject = pattern(
        r"(?i)\b(?:LINKAGE\s+SECTION|PROCEDURE\s+DIVISION)\b",
        &INJECT,
    );
    let block = pattern(
        r"(?is)^\s*EXEC\s+DLI\s+(\w+)\b(.*?)\bEND-EXEC\b(\.)?\s*$",
        &BLOCK,
    );
    if !name(dbd) || dbd.chars().any(|ch| ch.is_ascii_lowercase()) {
        return Err("DBD name must be 1..8 uppercase letters or digits".into());
    }
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let mut output = String::new();
    let mut index = 0;
    let mut injected = false;
    while index < lines.len() {
        let current = lines[index];
        if !injected && inject.is_match(code(current)) {
            for instruction in [
                "01 DIBSTAT PIC X(2).".to_string(),
                "01 IMS-DLI-FUNCTION PIC X(4).".into(),
                "01 IMS-DLI-ARG PIC X(8).".into(),
                "01 IMS-DLI-RC PIC S9(9) COMP-5.".into(),
                "01 IMS-DLI-SSA-1 PIC X(256).".into(),
                "01 IMS-DLI-SSA-2 PIC X(256).".into(),
                "01 IMS-DLI-PCB.".into(),
                format!("   05 IMS-DLI-DBD PIC X(8) VALUE '{dbd}'."),
                "   05 FILLER PIC X(2).".into(),
                "   05 IMS-DLI-STATUS PIC X(2).".into(),
                "   05 FILLER PIC X(44).".into(),
            ] {
                output.push_str(&line(&instruction));
            }
            injected = true;
        }
        if current
            .as_bytes()
            .get(6)
            .is_some_and(|marker| *marker == b'*' || *marker == b'/')
            || !exec.is_match(code(current))
        {
            output.push_str(current);
            index += 1;
            continue;
        }
        let start = index + 1;
        let mut pieces = Vec::new();
        while index < lines.len() {
            let item = lines[index];
            if item
                .as_bytes()
                .get(6)
                .is_some_and(|marker| *marker == b'*' || *marker == b'/')
            {
                return Err(format!("line {}: comment inside EXEC DLI block", index + 1));
            }
            let part = code(item).trim();
            pieces.push(part);
            index += 1;
            if end.is_match(part) {
                break;
            }
        }
        let joined = pieces.join(" ");
        let captures = block
            .captures(&joined)
            .ok_or_else(|| format!("line {start}: unclosed or unsupported EXEC DLI block"))?;
        output.push_str(&translate_command(&captures[1], &captures[2], start)?);
        if captures.get(3).is_some() {
            output.push_str(&line("."));
        }
    }
    if !injected {
        return Err(
            "source has no PROCEDURE DIVISION or LINKAGE SECTION for DIBSTAT insertion".into(),
        );
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::translate;

    #[test]
    fn qualified_get_and_services() {
        let source = "       LINKAGE SECTION.\n           EXEC DLI GU USING PCB(PAUT-PCB-NUM)\n               SEGMENT(PAUTSUM0) INTO(DATA)\n               WHERE(ACCNTID = PA-ACCT-ID) END-EXEC\n           EXEC DLI CHKP ID(WK-CHKPT-ID) END-EXEC\n";
        let result = translate(source, "DBPAUTP0").unwrap();
        assert!(result.contains("MOVE 'GHU ' TO IMS-DLI-FUNCTION"));
        assert!(result.contains("IMS-DLI-SSA-1 PA-ACCT-ID"));
        assert!(result.contains("MOVE WK-CHKPT-ID TO IMS-DLI-ARG"));
    }

    #[test]
    fn unsupported_clause_has_line() {
        let source = "       LINKAGE SECTION.\n           EXEC DLI GU PCB(PAUT-PCB-NUM)\n               SEGMENT(PAUTSUM0) INTO(DATA) BOGUS(1)\n           END-EXEC\n";
        assert!(translate(source, "DBPAUTP0")
            .unwrap_err()
            .contains("line 2: unsupported EXEC DLI clause BOGUS"));
    }
}
