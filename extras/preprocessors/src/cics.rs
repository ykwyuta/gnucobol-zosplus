//! Strict fixed-format EXEC CICS translation for the local worker ABI.
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

fn regex(source: &'static str, cell: &'static OnceLock<Regex>) -> &'static Regex {
    cell.get_or_init(|| Regex::new(source).expect("valid CICS preprocessor pattern"))
}
fn matches(source: &str, pattern: &str) -> bool {
    Regex::new(pattern).expect("valid pattern").is_match(source)
}
fn upper(value: &str) -> String {
    value.to_ascii_uppercase()
}
fn emit(value: &str) -> String {
    format!("{value}\n")
}
fn finish(lines: &mut [String], period: bool) {
    if period {
        if let Some(last) = lines.last_mut() {
            *last = format!("{}.\n", last.trim_end_matches('\n').trim_end());
        }
    }
}
fn bounded(lines: &[String], kind: &str) -> Result<(), String> {
    if lines
        .iter()
        .any(|line| line.trim_end_matches('\n').len() > 72)
    {
        Err(format!("generated {kind} line exceeds column 72"))
    } else {
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct Maps {
    pub fields: BTreeMap<String, Vec<(String, usize)>>,
    pub extended: BTreeMap<(String, String), BTreeSet<char>>,
}

pub fn maps_from_copybook(source: &str) -> Result<Maps, String> {
    static MAP: OnceLock<Regex> = OnceLock::new();
    static GROUP: OnceLock<Regex> = OnceLock::new();
    static FIELD: OnceLock<Regex> = OnceLock::new();
    static EXT: OnceLock<Regex> = OnceLock::new();
    let map = regex(
        r"(?i)^01\s+([A-Z0-9-]+)O\s+REDEFINES\s+([A-Z0-9-]+)I\.$",
        &MAP,
    );
    let group = regex(r"(?i)^02\s+([A-Z0-9-]+)G\s+OCCURS\s+([0-9]+)\.$", &GROUP);
    let field = regex(r"(?i)^0[23]\s+([A-Z0-9-]+)O\s+PIC\s+", &FIELD);
    let ext = regex(r"(?i)^0[23]\s+([A-Z0-9-]+)\s+PIC\s+X\.$", &EXT);
    let mut maps = Maps::default();
    let mut current: Option<String> = None;
    let mut occurs = 1;
    let mut last_field = None;
    for line in source.lines() {
        let code = line
            .get(7..line.len().min(72))
            .unwrap_or(line)
            .trim()
            .to_ascii_uppercase();
        if let Some(captures) = map.captures(&code) {
            if captures[1] == captures[2] {
                let name = captures[1].to_string();
                if maps.fields.contains_key(&name) {
                    return Err(format!("duplicate symbolic map: {name}"));
                }
                maps.fields.insert(name.clone(), Vec::new());
                current = Some(name);
                occurs = 1;
                last_field = None;
                continue;
            }
        }
        if code.starts_with("01 ") {
            current = None;
            last_field = None;
            continue;
        }
        let Some(ref name) = current else {
            continue;
        };
        if let Some(captures) = group.captures(&code) {
            occurs = captures[2]
                .parse::<usize>()
                .map_err(|error| error.to_string())?;
            if !(1..=99).contains(&occurs) {
                return Err(format!("invalid OCCURS count in {name}"));
            }
            continue;
        }
        if let Some(captures) = field.captures(&code) {
            if code.starts_with("02 ") {
                occurs = 1;
            }
            let item = captures[1].to_string();
            maps.fields
                .get_mut(name)
                .unwrap()
                .push((item.clone(), occurs));
            last_field = Some(item);
        } else if let (Some(last), Some(captures)) = (&last_field, ext.captures(&code)) {
            let candidate = &captures[1];
            if candidate == format!("{last}C") || candidate == format!("{last}H") {
                maps.extended
                    .entry((name.clone(), last.clone()))
                    .or_default()
                    .insert(candidate.chars().last().unwrap());
            }
        }
    }
    if maps.fields.is_empty() || maps.fields.values().any(Vec::is_empty) {
        return Err("copybook contains no symbolic output fields".into());
    }
    Ok(maps)
}

type Options = BTreeMap<String, Option<String>>;
fn options(source: &str) -> Result<Options, String> {
    static OPTION: OnceLock<Regex> = OnceLock::new();
    let option = regex(
        r"(?i)^([A-Z][A-Z0-9-]*)(?:\s*\(\s*('[^']*'|[0-9]+|(?:LENGTH\s+OF\s+)?[A-Z][A-Z0-9-]*(?:\s+OF\s+[A-Z][A-Z0-9-]*)*)\s*\))?",
        &OPTION,
    );
    let mut result = Options::new();
    let mut pos = 0;
    while pos < source.len() {
        pos += source[pos..].len() - source[pos..].trim_start().len();
        if pos == source.len() {
            break;
        }
        let rest = &source[pos..];
        let captures = option
            .captures(rest)
            .ok_or_else(|| format!("invalid CICS option near {:?}", &rest[..rest.len().min(20)]))?;
        let whole = captures.get(0).unwrap();
        let key = upper(&captures[1]);
        if result
            .insert(
                key.clone(),
                captures.get(2).map(|value| value.as_str().to_string()),
            )
            .is_some()
        {
            return Err(format!("duplicate CICS option: {key}"));
        }
        pos += whole.end();
        if pos < source.len() && !source.as_bytes()[pos].is_ascii_whitespace() {
            return Err(format!(
                "invalid CICS option near {:?}",
                &source[pos..source.len().min(pos + 20)]
            ));
        }
    }
    Ok(result)
}

fn opt<'a>(options: &'a Options, name: &str) -> Option<&'a str> {
    options.get(name).and_then(Option::as_deref)
}
fn identifier(value: Option<&str>, option: &str) -> Result<String, String> {
    let value = value.ok_or_else(|| format!("{option} requires a COBOL data name"))?;
    if !matches(value, r"(?i)^[A-Z][A-Z0-9-]*(?:\s+OF\s+[A-Z][A-Z0-9-]*)*$") {
        return Err(format!("{option} requires a COBOL data name"));
    }
    Ok(value
        .split_whitespace()
        .map(upper)
        .collect::<Vec<_>>()
        .join(" "))
}
fn only(options: &Options, allowed: &[&str], command: &str) -> Result<(), String> {
    if let Some(key) = options.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("unsupported {command} option: {key}"));
    }
    Ok(())
}
fn targets(options: &Options) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for (option, eib) in [("RESP", "EIBRESP"), ("RESP2", "EIBRESP2")] {
        if options.contains_key(option) {
            lines.push(emit(&format!(
                "           MOVE {eib} TO {}",
                identifier(opt(options, option), option)?
            )));
        }
    }
    Ok(lines)
}
fn require_normal(lines: &mut Vec<String>, options: &Options) {
    if !options.contains_key("RESP")
        && !options.contains_key("RESP2")
        && !options.contains_key("NOHANDLE")
    {
        lines.push(emit("           CALL STATIC \"COBCICS_REQUIRE_NORMAL\""));
        lines.push(emit("             USING EIBRESP"));
    }
}

fn find_end(code: &str) -> Option<usize> {
    let bytes = code.as_bytes();
    let mut quote = None;
    let mut index = 0;
    while index < bytes.len() {
        let ch = bytes[index];
        if let Some(delimiter) = quote {
            if ch == delimiter {
                if bytes.get(index + 1) == Some(&delimiter) {
                    index += 2;
                    continue;
                }
                quote = None;
            }
        } else if ch == b'\'' || ch == b'"' {
            quote = Some(ch);
        } else if bytes
            .get(index..index + 8)
            .is_some_and(|part| part.eq_ignore_ascii_case(b"END-EXEC"))
            && (index == 0
                || !(bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'-'))
            && (index + 8 == bytes.len()
                || !(bytes[index + 8].is_ascii_alphanumeric() || bytes[index + 8] == b'-'))
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn rewrite_constants(line: &str, number: usize) -> Result<String, String> {
    if !upper(line).contains("DFHRESP") && !upper(line).contains("DFHVALUE") {
        return Ok(line.into());
    }
    static MACRO: OnceLock<Regex> = OnceLock::new();
    let macro_re = regex(r"(?i)^(DFHRESP|DFHVALUE)\(([A-Z][A-Z0-9-]*)\)", &MACRO);
    let code = line.get(7..line.len().min(72)).unwrap_or("");
    let mut result = String::new();
    let mut quote = None;
    let mut index = 0;
    while index < code.len() {
        let ch = code.as_bytes()[index];
        if let Some(delimiter) = quote {
            result.push(ch as char);
            if ch == delimiter {
                if code.as_bytes().get(index + 1) == Some(&delimiter) {
                    result.push(delimiter as char);
                    index += 1;
                } else {
                    quote = None;
                }
            }
        } else if ch == b'\'' || ch == b'"' {
            quote = Some(ch);
            result.push(ch as char);
        } else if let Some(caps) = macro_re.captures(&code[index..]) {
            let kind = upper(&caps[1]);
            let name = upper(&caps[2]);
            let value = if kind == "DFHRESP" {
                match name.as_str() {
                    "NORMAL" => 0,
                    "MAPFAIL" => 36,
                    "NOTFND" => 13,
                    "INVREQ" => 16,
                    "NOTFINISHED" => 113,
                    "QIDERR" => 44,
                    "ITEMERR" => 26,
                    "LENGERR" => 22,
                    "NOSPACE" => 18,
                    "DUPREC" => 14,
                    "IOERR" => 17,
                    _ => return Err(format!("{number}: unsupported {kind} constant: {name}")),
                }
            } else {
                match name.as_str() {
                    "UCTRAN" => 450,
                    "NOUCTRAN" => 451,
                    "TRANIDONLY" => 452,
                    "NORMAL" => 1016,
                    "ABEND" => 900,
                    "SECERROR" => 1214,
                    _ => return Err(format!("{number}: unsupported {kind} constant: {name}")),
                }
            };
            result.push_str(&value.to_string());
            index += caps.get(0).unwrap().end();
            continue;
        } else {
            result.push(ch as char);
        }
        index += 1;
    }
    if result.len() > 65 {
        return Err(format!("{number}: rewritten COBOL line exceeds column 72"));
    }
    let suffix = line.get(72..).unwrap_or("").trim_end_matches(['\r', '\n']);
    let newline = if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    if suffix.is_empty() {
        Ok(format!("{}{}{}", &line[..7], result.trim_end(), newline))
    } else {
        Ok(format!("{}{result:<65}{suffix}{newline}", &line[..7]))
    }
}

const STORAGE: &str = "       01 COBCICS-LENGTH PIC S9(4) COMP.\n       01 COBCICS-LINK-CAPACITY\n          PIC S9(4) COMP.\n       01 COBCICS-TEXT-CAPACITY\n          PIC S9(4) COMP.\n       01 COBCICS-DATE-CAPACITY\n          PIC S9(4) COMP.\n       01 COBCICS-TIME-CAPACITY\n          PIC S9(4) COMP.\n       01 COBCICS-FORMATTIME-CLOCK PIC X(8).\n       01 COBCICS-DATA-CAPACITY PIC S9(8) COMP.\n       01 COBCICS-DATA-LENGTH PIC S9(8) COMP.\n       01 COBCICS-CURRENT-CHANNEL PIC X(16)\n          VALUE \"DFHEP.EVENT\".\n       01 COBCICS-QUEUE-CAPACITY PIC S9(8) COMP.\n       01 COBCICS-QUEUE-LENGTH PIC S9(8) COMP.\n       01 COBCICS-QUEUE-ITEM PIC S9(8) COMP.\n       01 COBCICS-QUEUE-NAME PIC X(8).\n       01 COBCICS-COUNTER-NAME PIC X(16).\n       01 COBCICS-COUNTER-POOL PIC X(8).\n       01 COBCICS-COUNTER-VALUE PIC S9(8) COMP.\n       01 COBCICS-FILE-NAME PIC X(8).\n       01 COBCICS-FILE-LENGTH PIC S9(8) COMP.\n       01 COBCICS-FILE-CAPACITY PIC S9(8) COMP.\n       01 COBCICS-FILE-KEY-LENGTH\n          PIC S9(8) COMP.\n       01 COBCICS-FILE-KEY-CAPACITY\n          PIC S9(8) COMP.\n       01 COBCICS-START-MINUTES\n          PIC S9(8) COMP.\n       01 COBCICS-CHILD-PROGRAM PIC X(8).\n       01 COBCICS-COMMAREA PIC X(2048).\n       01 COBCICS-HANDLE-ABEND PIC 9 VALUE ZERO.\n       01 COBCICS-ABEND-CODE PIC X(4) VALUE SPACES.\n";

pub fn translate(source: &str, maps: &Maps, mapset: &str) -> Result<String, String> {
    static WORKING: OnceLock<Regex> = OnceLock::new();
    static PROCEDURE: OnceLock<Regex> = OnceLock::new();
    static PROGRAM: OnceLock<Regex> = OnceLock::new();
    static START: OnceLock<Regex> = OnceLock::new();
    let working = regex(r"(?im)^.{7}\s*WORKING-STORAGE\s+SECTION\s*\.", &WORKING);
    let procedure = regex(
        r"(?im)^.{7}\s*PROCEDURE\s+DIVISION(?:\s+USING\s+([A-Z][A-Z0-9-]*))?\s*\.",
        &PROCEDURE,
    );
    let program = regex(
        r"(?im)^.{7}\s*PROGRAM-ID\s*\.\s*([A-Z][A-Z0-9-]*)\s*\.",
        &PROGRAM,
    );
    let start = regex(r"(?i)^\s*EXEC\s+CICS\b", &START);
    if !working.is_match(source) {
        return Err("WORKING-STORAGE SECTION is required".into());
    }
    let proc_caps = procedure
        .captures(source)
        .ok_or("PROCEDURE DIVISION. is required")?;
    let linkage_area = proc_caps.get(1).map(|area| upper(area.as_str()));
    if linkage_area
        .as_deref()
        .is_some_and(|area| area != "DFHCOMMAREA")
    {
        return Err("linked PROCEDURE DIVISION must use DFHCOMMAREA".into());
    }
    let active: String = source
        .lines()
        .filter(|line| {
            line.as_bytes()
                .get(6)
                .is_some_and(|marker| !b"*/Dd".contains(marker))
        })
        .map(|line| line.get(7..line.len().min(72)).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    let implicit = linkage_area.is_none()
        && matches(source, r"(?im)^.{7}\s*LINKAGE\s+SECTION\s*\.")
        && matches(source, r"(?im)^.{7}\s*01\s+DFHCOMMAREA\b")
        && !matches(&active, r"(?i)\bRECEIVE\s+MAP\b");
    let linked = linkage_area.is_some() || implicit;
    let program_id = program.captures(source).map(|caps| upper(&caps[1]));
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let mut output = String::new();
    let mut in_cics = false;
    let mut body = Vec::new();
    let mut start_line = 0;
    let mut mapfail_label: Option<String> = None;
    let mut abend_label: Option<String> = None;
    let abend_re = Regex::new(
        r"(?i)EXEC\s+CICS\s+HANDLE\s+ABEND\s+LABEL\s*\(\s*([A-Z][A-Z0-9-]*)\s*\)\s*END-EXEC",
    )
    .unwrap();
    for caps in abend_re.captures_iter(&active) {
        let label = upper(&caps[1]);
        if abend_label
            .as_ref()
            .is_some_and(|previous| previous != &label)
        {
            return Err("multiple HANDLE ABEND labels are not supported".into());
        }
        abend_label = Some(label);
    }
    if let Some(label) = &abend_label {
        let pattern = format!(
            r"(?im)^.{{7}}\s*{}\s+(?:SECTION\s*)?\.",
            regex::escape(label)
        );
        if !matches(source, &pattern) {
            return Err(format!("HANDLE ABEND label {label} is not defined"));
        }
    }
    for (number, original) in lines.iter().enumerate() {
        if original
            .as_bytes()
            .get(6)
            .is_some_and(|marker| b"*/Dd".contains(marker))
        {
            if !in_cics {
                output.push_str(original);
            }
            continue;
        }
        let mut code = original.get(7..original.len().min(72)).unwrap_or(original);
        if !in_cics {
            if !linked && matches(code, r"(?i)^\s*LINKAGE\s+SECTION\s*\.") {
                output.push_str("      * CICS COMMAREA storage for terminal task.\n");
                continue;
            }
            if matches(code, r"(?i)^\s*WORKING-STORAGE\s+SECTION\s*\.") {
                output.push_str(original);
                output.push_str(STORAGE);
                for (name, declaration) in [
                    ("EIBAID", "01 EIBAID PIC X."),
                    ("EIBCALEN", "01 EIBCALEN PIC S9(4) COMP."),
                    ("EIBTRNID", "01 EIBTRNID PIC X(4)."),
                    ("EIBTRMID", "01 EIBTRMID PIC X(4)."),
                    ("EIBTASKN", "01 EIBTASKN PIC S9(7) COMP-3."),
                    ("EIBRESP", "01 EIBRESP PIC S9(8) COMP."),
                    ("EIBRESP2", "01 EIBRESP2 PIC S9(8) COMP."),
                ] {
                    if !matches(source, &format!(r"(?im)^.{{7}}\s*01\s+{name}\b")) {
                        output.push_str(&format!("       {declaration}\n"));
                    }
                }
                continue;
            }
            if matches(
                code,
                r"(?i)^\s*PROCEDURE\s+DIVISION(?:\s+USING\s+DFHCOMMAREA)?\s*\.",
            ) {
                if implicit {
                    output.push_str("       PROCEDURE DIVISION USING DFHCOMMAREA.\n");
                } else {
                    output.push_str(original);
                }
                if linked {
                    output.push_str("           CALL STATIC \"COBCICS_LINK_START\"\n             USING EIBAID EIBCALEN EIBTRNID.\n");
                } else {
                    output.push_str("           CALL STATIC \"COBCICS_START\"\n             USING EIBAID EIBCALEN EIBTRNID\n           MOVE LENGTH OF COBCICS-COMMAREA\n             TO COBCICS-LENGTH\n           CALL STATIC \"COBCICS_GET_COMMAREA\"\n");
                    if matches(source, r"(?im)^.{7}\s*01\s+DFHCOMMAREA\b") {
                        output.push_str("             USING COBCICS-COMMAREA COBCICS-LENGTH\n           MOVE COBCICS-COMMAREA TO DFHCOMMAREA.\n");
                    } else {
                        output.push_str("             USING COBCICS-COMMAREA COBCICS-LENGTH.\n");
                    }
                }
                output.push_str("           MOVE \"L001\" TO EIBTRMID.\n           CALL STATIC \"COBCICS_TASK_NUMBER\"\n             USING EIBTASKN.\n");
                continue;
            }
            let Some(found) = start.find(code) else {
                let mut expanded = rewrite_constants(original, number + 1)?;
                let cee = Regex::new(r#"(?i)\bCALL\s+"(CEEDAYS|CEELOCT)""#).unwrap();
                expanded = cee
                    .replace_all(&expanded, |captures: &regex::Captures| {
                        format!("CALL STATIC \"COBCICS_{}\"", upper(&captures[1]))
                    })
                    .into_owned();
                output.push_str(&expanded);
                continue;
            };
            in_cics = true;
            start_line = number + 1;
            code = &code[found.end()..];
        }
        if let Some(end) = find_end(code) {
            body.push(code[..end].to_string());
            let tail = code[end + 8..].trim();
            if tail != "" && tail != "." {
                return Err(format!("{}: text after END-EXEC", number + 1));
            }
            let command = body.join(" ");
            if matches(&command, r"(?is)^\s*HANDLE\s+CONDITION\b") {
                let rest = Regex::new(r"(?is)^\s*HANDLE\s+CONDITION\b(.*)$")
                    .unwrap()
                    .captures(&command)
                    .unwrap()[1]
                    .to_string();
                let parsed = options(&rest).map_err(|error| format!("{start_line}: {error}"))?;
                if parsed.len() == 1 && parsed.contains_key("MAPFAIL") {
                    mapfail_label = Some(
                        identifier(opt(&parsed, "MAPFAIL"), "MAPFAIL")
                            .map_err(|error| format!("{start_line}: {error}"))?,
                    );
                }
            }
            let translated = statement(
                &command,
                maps,
                tail == ".",
                mapset,
                program_id.as_deref(),
                abend_label.as_deref(),
                mapfail_label.as_deref(),
            )
            .map_err(|error| format!("{start_line}: {error}"))?;
            for item in translated {
                output.push_str(&item);
            }
            in_cics = false;
            body.clear();
        } else {
            body.push(code.trim_end().to_string());
        }
    }
    if in_cics {
        return Err(format!("{start_line}: missing END-EXEC"));
    }
    Ok(output)
}

fn statement(
    body: &str,
    maps: &Maps,
    period: bool,
    mapset: &str,
    program_id: Option<&str>,
    abend_label: Option<&str>,
    mapfail_label: Option<&str>,
) -> Result<Vec<String>, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("empty EXEC CICS".into());
    }
    let mut result = Vec::new();
    if matches(body, r"(?is)^HANDLE\s+CONDITION\b") {
        let parsed = options(
            Regex::new(r"(?is)^HANDLE\s+CONDITION\b(.*)$")
                .unwrap()
                .captures(body)
                .unwrap()
                .get(1)
                .unwrap()
                .as_str(),
        )?;
        if parsed.len() != 1 || !parsed.contains_key("MAPFAIL") {
            return Err("HANDLE CONDITION supports MAPFAIL only".into());
        }
        identifier(opt(&parsed, "MAPFAIL"), "MAPFAIL")?;
        result.push(emit("           CONTINUE"));
        finish(&mut result, period);
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^HANDLE\s+AID\b(.*)$")
        .unwrap()
        .captures(body)
    {
        let parsed = options(&caps[1])?;
        if parsed.is_empty() || parsed.keys().any(|key| key != "CLEAR" && key != "PF3") {
            return Err("HANDLE AID supports CLEAR and PF3".into());
        }
        for (aid, code) in [("CLEAR", "6D"), ("PF3", "F3")] {
            if parsed.contains_key(aid) {
                let label = identifier(opt(&parsed, aid), aid)?;
                result.extend([
                    emit(&format!("           IF EIBAID = X'{code}'")),
                    emit(&format!("             GO TO {label}")),
                    emit("           END-IF"),
                ]);
            }
        }
        finish(&mut result, period);
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^HANDLE\s+ABEND\b(.*)$")
        .unwrap()
        .captures(body)
    {
        let parsed = options(&caps[1])?;
        if parsed.len() != 1
            || parsed
                .keys()
                .any(|key| !["LABEL", "CANCEL", "RESET"].contains(&key.as_str()))
        {
            return Err("HANDLE ABEND requires LABEL, CANCEL or RESET".into());
        }
        if parsed.contains_key("LABEL") {
            let label = identifier(opt(&parsed, "LABEL"), "LABEL")?;
            if abend_label.is_some_and(|previous| previous != label) {
                return Err("HANDLE ABEND LABEL does not match exit".into());
            }
            result.push(emit("           MOVE 1 TO COBCICS-HANDLE-ABEND"));
        } else if parsed.contains_key("CANCEL") {
            if opt(&parsed, "CANCEL").is_some() {
                return Err("HANDLE ABEND CANCEL takes no value".into());
            }
            result.push(emit("           MOVE 0 TO COBCICS-HANDLE-ABEND"));
        } else {
            if opt(&parsed, "RESET").is_some() || abend_label.is_none() {
                return Err("HANDLE ABEND RESET needs an earlier LABEL".into());
            }
            result.push(emit("           MOVE 1 TO COBCICS-HANDLE-ABEND"));
        }
        finish(&mut result, period);
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^ABEND\b(.*)$").unwrap().captures(body) {
        let parsed = options(&caps[1])?;
        only(&parsed, &["ABCODE", "NODUMP", "CANCEL"], "ABEND")?;
        for flag in ["NODUMP", "CANCEL"] {
            if opt(&parsed, flag).is_some() {
                return Err(format!("{flag} does not take a value"));
            }
        }
        let (code, length_lines) = if let Some(raw) = opt(&parsed, "ABCODE") {
            if raw.starts_with('\'') {
                if !matches(raw, r"(?i)^'[A-Z0-9 ]{1,4}'$") {
                    return Err("ABCODE requires a 1-4 character literal".into());
                }
                (
                    format!("\"{:<4}\"", upper(&raw[1..raw.len() - 1])),
                    vec![emit("           MOVE 4 TO COBCICS-LENGTH")],
                )
            } else {
                let code = identifier(Some(raw), "ABCODE")?;
                (
                    code.clone(),
                    vec![
                        emit(&format!("           MOVE LENGTH OF {code}")),
                        emit("             TO COBCICS-LENGTH"),
                    ],
                )
            }
        } else if parsed.contains_key("ABCODE") {
            return Err("ABCODE requires a 1-4 character literal or data name".into());
        } else {
            (
                "\"USER\"".into(),
                vec![emit("           MOVE 4 TO COBCICS-LENGTH")],
            )
        };
        if let Some(label) = abend_label {
            if !parsed.contains_key("CANCEL") {
                result.extend([
                    emit(&format!("           MOVE {code} TO COBCICS-ABEND-CODE")),
                    emit("           IF COBCICS-HANDLE-ABEND = 1"),
                    emit("               MOVE 0 TO COBCICS-HANDLE-ABEND"),
                    emit(&format!("               GO TO {label}")),
                    emit("           END-IF"),
                ]);
            }
        }
        result.extend(length_lines);
        result.extend([
            emit("           CALL STATIC \"COBCICS_ABEND\""),
            emit(&format!("             USING {code} COBCICS-LENGTH")),
            emit("           GOBACK"),
        ]);
        finish(&mut result, period);
        bounded(&result, "ABEND")?;
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^BIF\s+DEEDIT\b(.*)$")
        .unwrap()
        .captures(body)
    {
        let parsed = options(&caps[1])?;
        only(&parsed, &["FIELD", "LENGTH"], "BIF DEEDIT")?;
        let field = identifier(opt(&parsed, "FIELD"), "FIELD")?;
        let length = opt(&parsed, "LENGTH");
        if parsed.contains_key("LENGTH")
            && !length.is_some_and(|value| matches(value, r"(?i)^(?:[A-Z][A-Z0-9-]*|[0-9]+)$"))
        {
            return Err("invalid BIF DEEDIT LENGTH".into());
        }
        result.extend([
            emit(&format!("           MOVE LENGTH OF {field}")),
            emit("             TO COBCICS-LINK-CAPACITY"),
        ]);
        if let Some(length) = length {
            result.push(emit(&format!("           MOVE {length} TO COBCICS-LENGTH")));
        } else {
            result.extend([
                emit(&format!("           MOVE LENGTH OF {field}")),
                emit("             TO COBCICS-LENGTH"),
            ]);
        }
        result.extend([
            emit("           CALL STATIC \"COBCICS_DEEDIT\""),
            emit(&format!("             USING {field} COBCICS-LENGTH")),
            emit("                   COBCICS-LINK-CAPACITY"),
        ]);
        finish(&mut result, period);
        bounded(&result, "BIF DEEDIT")?;
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^(SEND|RECEIVE)\s+MAP\b(.*)$")
        .unwrap()
        .captures(body)
    {
        return map_statement(
            &upper(&caps[1]),
            &caps[2],
            maps,
            period,
            mapset,
            mapfail_label,
        );
    }
    if let Some(caps) = Regex::new(r"(?is)^ASKTIME\b(.*)$").unwrap().captures(body) {
        let parsed = options(&caps[1])?;
        only(
            &parsed,
            &["ABSTIME", "RESP", "RESP2", "NOHANDLE"],
            "ASKTIME",
        )?;
        let area = identifier(opt(&parsed, "ABSTIME"), "ABSTIME")?;
        result.extend([
            emit(&format!("           MOVE LENGTH OF {area}")),
            emit("             TO COBCICS-LENGTH"),
            emit("           CALL STATIC \"COBCICS_ASKTIME\""),
            emit(&format!("             USING {area} COBCICS-LENGTH")),
            emit("           MOVE ZERO TO EIBRESP EIBRESP2"),
        ]);
        result.extend(targets(&parsed)?);
        finish(&mut result, period);
        bounded(&result, "ASKTIME")?;
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^FORMATTIME\b(.*)$")
        .unwrap()
        .captures(body)
    {
        let parsed = options(&caps[1])?;
        let abstime = identifier(opt(&parsed, "ABSTIME"), "ABSTIME")?;
        let short = if parsed.contains_key("YYDDD") {
            Some("YYDDD")
        } else if parsed.contains_key("YYMMDD") {
            Some("YYMMDD")
        } else {
            None
        };
        if let Some(kind) = short {
            let mut allowed = vec!["ABSTIME", kind, "TIME", "RESP", "RESP2"];
            if kind == "YYDDD" {
                allowed.push("MILLISECONDS");
            }
            only(&parsed, &allowed, "FORMATTIME")?;
            let date = identifier(opt(&parsed, kind), kind)?;
            let clock = identifier(opt(&parsed, "TIME"), "TIME")?;
            let routine = if kind == "YYDDD" {
                "COBCICS_FORMATTIME_YYDDD"
            } else {
                "COBCICS_FORMATTIME_YYMMDD"
            };
            result.extend([
                emit(&format!("           MOVE LENGTH OF {abstime}")),
                emit("             TO COBCICS-LENGTH"),
                emit(&format!("           MOVE LENGTH OF {date}")),
                emit("             TO COBCICS-DATE-CAPACITY"),
                emit(&format!("           MOVE LENGTH OF {clock}")),
                emit("             TO COBCICS-TIME-CAPACITY"),
                emit(&format!("           CALL STATIC \"{routine}\"")),
                emit(&format!("             USING {abstime} COBCICS-LENGTH")),
                emit(&format!("                   {date}")),
                emit("                   COBCICS-DATE-CAPACITY"),
                emit(&format!("                   {clock}")),
                emit("                   COBCICS-TIME-CAPACITY"),
            ]);
            if kind == "YYDDD" {
                result.push(emit(&format!(
                    "                   {}",
                    identifier(opt(&parsed, "MILLISECONDS"), "MILLISECONDS")?
                )));
            }
            result.push(emit("                   EIBRESP EIBRESP2"));
            require_normal(&mut result, &parsed);
            result.extend(targets(&parsed)?);
            finish(&mut result, period);
            return Ok(result);
        }
        only(
            &parsed,
            &[
                "ABSTIME", "DDMMYYYY", "MMDDYYYY", "MMDDYY", "TIME", "DATESEP", "TIMESEP",
                "NOHANDLE", "RESP", "RESP2",
            ],
            "FORMATTIME",
        )?;
        let dates: Vec<_> = ["DDMMYYYY", "MMDDYYYY", "MMDDYY"]
            .into_iter()
            .filter(|name| parsed.contains_key(*name))
            .collect();
        if dates.len() != 1 {
            return Err("FORMATTIME requires exactly one date format".into());
        }
        let format = dates[0];
        let date = identifier(opt(&parsed, format), format)?;
        let clock = if parsed.contains_key("TIME") {
            identifier(opt(&parsed, "TIME"), "TIME")?
        } else {
            "COBCICS-FORMATTIME-CLOCK".into()
        };
        let date_separator = if parsed.contains_key("DATESEP") {
            separator(opt(&parsed, "DATESEP"), '/', "DATESEP")?
        } else {
            "X'00'".into()
        };
        let time_separator = if parsed.contains_key("TIMESEP") {
            Some(separator(opt(&parsed, "TIMESEP"), ':', "TIMESEP")?)
        } else {
            None
        };
        let routine = format!(
            "COBCICS_FORMATTIME{}{}",
            if format == "MMDDYY" {
                "_MMDDYY"
            } else if format == "MMDDYYYY" {
                "_MMDD"
            } else {
                ""
            },
            if time_separator.is_some() {
                "_TIMESEP"
            } else {
                ""
            }
        );
        result.extend([
            emit(&format!("           MOVE LENGTH OF {abstime}")),
            emit("             TO COBCICS-LENGTH"),
            emit(&format!("           MOVE LENGTH OF {date}")),
            emit("             TO COBCICS-DATE-CAPACITY"),
            emit(&format!("           MOVE LENGTH OF {clock}")),
            emit("             TO COBCICS-TIME-CAPACITY"),
            emit(&format!("           CALL STATIC \"{routine}\"")),
            emit(&format!("             USING {abstime} COBCICS-LENGTH")),
            emit(&format!("                   {date}")),
            emit("                   COBCICS-DATE-CAPACITY"),
            emit(&format!("                   {clock}")),
            emit("                   COBCICS-TIME-CAPACITY"),
            emit(&format!("                   {date_separator}")),
        ]);
        if let Some(time) = time_separator {
            result.push(emit(&format!("                   {time}")));
        }
        result.push(emit("                   EIBRESP EIBRESP2"));
        require_normal(&mut result, &parsed);
        result.extend(targets(&parsed)?);
        finish(&mut result, period);
        bounded(&result, "FORMATTIME")?;
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^SYNCPOINT\b(.*)$")
        .unwrap()
        .captures(body)
    {
        let parsed = options(&caps[1])?;
        only(&parsed, &["ROLLBACK", "RESP", "RESP2"], "SYNCPOINT")?;
        if opt(&parsed, "ROLLBACK").is_some() {
            return Err("SYNCPOINT ROLLBACK takes no value".into());
        }
        let mode = if parsed.contains_key("ROLLBACK") {
            "01"
        } else {
            "00"
        };
        result.push(emit("           CALL STATIC \"COBCICS_SYNCPOINT\""));
        result.push(emit(&format!(
            "             USING X'{mode}' EIBRESP EIBRESP2"
        )));
        require_normal(&mut result, &parsed);
        result.extend(targets(&parsed)?);
        finish(&mut result, period);
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^DELAY\b(.*)$").unwrap().captures(body) {
        let parsed = options(&caps[1])?;
        only(&parsed, &["FOR", "SECONDS", "RESP", "RESP2"], "DELAY")?;
        if !parsed.contains_key("FOR") || opt(&parsed, "FOR").is_some() {
            return Err("DELAY requires FOR SECONDS".into());
        }
        let seconds =
            opt(&parsed, "SECONDS").ok_or("DELAY SECONDS requires a data name or 0-60")?;
        if !matches(seconds, r"(?i)^(?:[0-9]+|[A-Z][A-Z0-9-]*)$")
            || seconds.parse::<u32>().is_ok_and(|number| number > 60)
        {
            return Err("DELAY SECONDS requires 0-60".into());
        }
        result.extend([
            emit(&format!("           MOVE {seconds} TO COBCICS-LENGTH")),
            emit("           CALL STATIC \"COBCICS_DELAY\""),
            emit("             USING COBCICS-LENGTH"),
            emit("           MOVE ZERO TO EIBRESP EIBRESP2"),
        ]);
        result.extend(targets(&parsed)?);
        finish(&mut result, period);
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^RETRIEVE\b(.*)$").unwrap().captures(body) {
        let parsed = options(&caps[1])?;
        only(&parsed, &["INTO", "RESP", "RESP2", "NOHANDLE"], "RETRIEVE")?;
        let area = identifier(opt(&parsed, "INTO"), "INTO")?;
        result.extend([
            emit(&format!("           MOVE LENGTH OF {area}")),
            emit("             TO COBCICS-DATA-CAPACITY"),
            emit("           CALL STATIC \"COBCICS_RETRIEVE\""),
            emit(&format!("             USING {area} COBCICS-DATA-CAPACITY")),
            emit("                   EIBRESP EIBRESP2"),
        ]);
        require_normal(&mut result, &parsed);
        result.extend(targets(&parsed)?);
        finish(&mut result, period);
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^START\s+TRANSID\b(.*)$")
        .unwrap()
        .captures(body)
    {
        let parsed = options(&format!("TRANSID{}", &caps[1]))?;
        only(
            &parsed,
            &["TRANSID", "AFTER", "MINUTES", "RESP", "RESP2"],
            "START",
        )?;
        let transid = upper(opt(&parsed, "TRANSID").unwrap_or(""));
        let minutes = opt(&parsed, "MINUTES");
        if !matches(&transid, r"^'[A-Z0-9]{1,4}'$")
            || !parsed.contains_key("AFTER")
            || opt(&parsed, "AFTER").is_some()
            || !minutes.is_some_and(|value| matches(value, r"(?i)^(?:[0-9]+|[A-Z][A-Z0-9-]*)$"))
        {
            return Err("START requires TRANSID AFTER MINUTES".into());
        }
        result.extend([
            emit(&format!(
                "           MOVE {} TO COBCICS-START-MINUTES",
                minutes.unwrap()
            )),
            emit("           CALL STATIC \"COBCICS_START_AFTER\""),
            emit(&format!(
                "             USING \"{:<4}\"",
                &transid[1..transid.len() - 1]
            )),
            emit("                   COBCICS-START-MINUTES"),
            emit("                   EIBRESP EIBRESP2"),
        ]);
        require_normal(&mut result, &parsed);
        result.extend(targets(&parsed)?);
        finish(&mut result, period);
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^WRITE\b(.*)$").unwrap().captures(body) {
        let parsed = options(&caps[1])?;
        if opt(&parsed, "FILE").is_some_and(|name| upper(name) == "'ABNDFILE'") {
            only(
                &parsed,
                &["FILE", "FROM", "RIDFLD", "RESP", "RESP2"],
                "WRITE",
            )?;
            let area = identifier(opt(&parsed, "FROM"), "FROM")?;
            let key = identifier(opt(&parsed, "RIDFLD"), "RIDFLD")?;
            result.extend([
                emit(&format!("           MOVE LENGTH OF {area}")),
                emit("             TO COBCICS-LENGTH"),
                emit(&format!("           MOVE LENGTH OF {key}")),
                emit("             TO COBCICS-LINK-CAPACITY"),
                emit("           CALL STATIC \"COBCICS_WRITE_ABNDFILE\""),
                emit(&format!("             USING {area} COBCICS-LENGTH")),
                emit(&format!("                   {key} COBCICS-LINK-CAPACITY")),
                emit("                   EIBRESP EIBRESP2"),
            ]);
            require_normal(&mut result, &parsed);
            result.extend(targets(&parsed)?);
            finish(&mut result, period);
            bounded(&result, "WRITE")?;
            return Ok(result);
        }
    }
    if let Some(caps) = Regex::new(r"(?is)^SEND\s+CONTROL\b(.*)$")
        .unwrap()
        .captures(body)
    {
        let parsed = options(&caps[1])?;
        only(
            &parsed,
            &["ERASE", "FREEKB", "RESP", "RESP2"],
            "SEND CONTROL",
        )?;
        if !parsed.contains_key("ERASE") || !parsed.contains_key("FREEKB") {
            return Err("SEND CONTROL currently requires ERASE FREEKB".into());
        }
        if opt(&parsed, "ERASE").is_some() || opt(&parsed, "FREEKB").is_some() {
            return Err("ERASE and FREEKB do not take values".into());
        }
        result.push(emit("           MOVE ZERO TO EIBRESP EIBRESP2"));
        result.extend(targets(&parsed)?);
        result.push(emit("           CALL STATIC \"COBCICS_SEND_CONTROL\""));
        finish(&mut result, period);
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^SEND\s+TEXT\b(.*)$")
        .unwrap()
        .captures(body)
    {
        let parsed = options(&format!("TEXT{}", &caps[1]))?;
        only(
            &parsed,
            &[
                "TEXT", "FROM", "LENGTH", "ERASE", "FREEKB", "WAIT", "RESP", "RESP2",
            ],
            "SEND TEXT",
        )?;
        if opt(&parsed, "TEXT").is_some() {
            return Err("SEND TEXT does not take a value".into());
        }
        if !parsed.contains_key("FROM") {
            return Err("SEND TEXT requires FROM".into());
        }
        let area = identifier(opt(&parsed, "FROM"), "FROM")?;
        if !parsed.contains_key("ERASE") || !parsed.contains_key("FREEKB") {
            return Err("SEND TEXT currently requires ERASE FREEKB".into());
        }
        if opt(&parsed, "ERASE").is_some() || opt(&parsed, "FREEKB").is_some() {
            return Err("ERASE and FREEKB do not take values".into());
        }
        if opt(&parsed, "WAIT").is_some() {
            return Err("WAIT does not take a value".into());
        }
        let length = opt(&parsed, "LENGTH");
        if parsed.contains_key("LENGTH") && !length.is_some_and(|value| {
            matches(
                value,
                r"(?i)^(?:(?:LENGTH\s+OF\s+)?[A-Z][A-Z0-9-]*(?:\s+OF\s+[A-Z][A-Z0-9-]*)*|[0-9]+)$",
            )
        }) {
            return Err("invalid SEND TEXT LENGTH".into());
        }
        if length
            .and_then(|value| value.parse::<usize>().ok())
            .is_some_and(|number| number > 1919)
        {
            return Err("SEND TEXT exceeds 1919 bytes".into());
        }
        result.extend([
            emit(&format!("           MOVE LENGTH OF {area}")),
            emit("             TO COBCICS-TEXT-CAPACITY"),
        ]);
        if let Some(length) = length {
            result.push(emit(&format!("           MOVE {length} TO COBCICS-LENGTH")));
        } else {
            result.extend([
                emit(&format!("           MOVE LENGTH OF {area}")),
                emit("             TO COBCICS-LENGTH"),
            ]);
        }
        result.push(emit("           MOVE ZERO TO EIBRESP EIBRESP2"));
        result.extend(targets(&parsed)?);
        result.extend([
            emit("           CALL STATIC \"COBCICS_SEND_TEXT\""),
            emit(&format!("             USING {area} COBCICS-LENGTH")),
            emit("                   COBCICS-TEXT-CAPACITY"),
        ]);
        finish(&mut result, period);
        bounded(&result, "SEND TEXT")?;
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^ASSIGN\b(.*)$").unwrap().captures(body) {
        let parsed = options(&caps[1])?;
        only(
            &parsed,
            &[
                "APPLID",
                "PROGRAM",
                "ABCODE",
                "SYSID",
                "STARTCODE",
                "INVOKINGPROG",
                "RESP",
                "RESP2",
            ],
            "ASSIGN",
        )?;
        if ![
            "APPLID",
            "PROGRAM",
            "ABCODE",
            "SYSID",
            "STARTCODE",
            "INVOKINGPROG",
        ]
        .iter()
        .any(|name| parsed.contains_key(*name))
        {
            return Err("ASSIGN requires an output option".into());
        }
        result.push(emit("           MOVE ZERO TO EIBRESP EIBRESP2"));
        if parsed.contains_key("ABCODE") {
            result.push(emit(&format!(
                "           MOVE COBCICS-ABEND-CODE TO {}",
                identifier(opt(&parsed, "ABCODE"), "ABCODE")?
            )));
        }
        for name in ["APPLID", "PROGRAM", "SYSID", "STARTCODE", "INVOKINGPROG"] {
            if !parsed.contains_key(name) {
                continue;
            }
            let target = identifier(opt(&parsed, name), name)?;
            result.extend([
                emit(&format!("           MOVE LENGTH OF {target}")),
                emit("             TO COBCICS-LENGTH"),
            ]);
            if name == "PROGRAM" {
                let id = program_id.ok_or("ASSIGN PROGRAM requires a 1-8 character PROGRAM-ID")?;
                if id.len() > 8 {
                    return Err("ASSIGN PROGRAM requires a 1-8 character PROGRAM-ID".into());
                }
                result.extend([
                    emit("           CALL STATIC \"COBCICS_ASSIGN_PROGRAM\""),
                    emit(&format!("             USING {target} COBCICS-LENGTH")),
                    emit(&format!("                   \"{id:<8}\"")),
                ]);
            } else {
                result.extend([
                    emit(&format!("           CALL STATIC \"COBCICS_ASSIGN_{name}\"")),
                    emit(&format!("             USING {target} COBCICS-LENGTH")),
                ]);
            }
        }
        result.extend(targets(&parsed)?);
        finish(&mut result, period);
        bounded(&result, "ASSIGN")?;
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^(INQUIRE|SET)\s+TERMINAL\b(.*)$")
        .unwrap()
        .captures(body)
    {
        let verb = upper(&caps[1]);
        let parsed = options(&format!("TERMINAL{}", &caps[2]))?;
        only(
            &parsed,
            &["TERMINAL", "UCTRANST", "RESP", "RESP2"],
            &format!("{verb} TERMINAL"),
        )?;
        let termid = identifier(opt(&parsed, "TERMINAL"), "TERMINAL")?;
        let value = identifier(opt(&parsed, "UCTRANST"), "UCTRANST")?;
        if termid != "EIBTRMID" {
            return Err("TERMINAL currently requires EIBTRMID".into());
        }
        result.extend([
            emit(&format!(
                "           CALL STATIC \"COBCICS_{verb}_TERMINAL\""
            )),
            emit(&format!("             USING {termid} {value}")),
            emit("                   EIBRESP EIBRESP2"),
        ]);
        require_normal(&mut result, &parsed);
        result.extend(targets(&parsed)?);
        finish(&mut result, period);
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^INQUIRE\s+ASSOCIATION\b(.*)$")
        .unwrap()
        .captures(body)
    {
        let parsed = options(&format!("ASSOCIATION{}", &caps[1]))?;
        let required = [
            "ASSOCIATION",
            "ODAPPLID",
            "ODUSERID",
            "ODFACILNAME",
            "ODNETWORKID",
            "ODFACILTYPE",
        ];
        if parsed.len() != required.len() || required.iter().any(|name| !parsed.contains_key(*name))
        {
            return Err("INQUIRE ASSOCIATION requires Bank origin fields".into());
        }
        if identifier(opt(&parsed, "ASSOCIATION"), "ASSOCIATION")? != "EIBTASKN" {
            return Err("ASSOCIATION currently requires EIBTASKN".into());
        }
        let values: Vec<_> = required[1..]
            .iter()
            .map(|name| identifier(opt(&parsed, name), name))
            .collect::<Result<_, _>>()?;
        result.extend([
            emit("           CALL STATIC \"COBCICS_INQUIRE_ASSOCIATION\""),
            emit(&format!("             USING {} {}", values[0], values[1])),
            emit(&format!("                   {} {}", values[2], values[3])),
            emit(&format!("                   {}", values[4])),
        ]);
        finish(&mut result, period);
        bounded(&result, "INQUIRE ASSOCIATION")?;
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^RUN\b(.*)$").unwrap().captures(body) {
        let parsed = options(&caps[1])?;
        only(
            &parsed,
            &["TRANSID", "CHANNEL", "CHILD", "RESP", "RESP2"],
            "RUN",
        )?;
        let transid = identifier(opt(&parsed, "TRANSID"), "TRANSID")?;
        let channel = identifier(opt(&parsed, "CHANNEL"), "CHANNEL")?;
        let child = identifier(opt(&parsed, "CHILD"), "CHILD")?;
        result.extend([
            emit("           CALL STATIC \"COBCICS_RUN_ENTER\""),
            emit(&format!("             USING {transid} {channel}")),
            emit(&format!("                   {child} COBCICS-CHILD-PROGRAM")),
            emit("                   EIBRESP EIBRESP2"),
            emit("           IF EIBRESP = ZERO"),
            emit("               CALL COBCICS-CHILD-PROGRAM"),
            emit("               CALL STATIC \"COBCICS_RUN_EXIT\""),
            emit("           END-IF"),
        ]);
        require_normal(&mut result, &parsed);
        result.extend(targets(&parsed)?);
        finish(&mut result, period);
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^FETCH\b(.*)$").unwrap().captures(body) {
        let parsed = options(&caps[1])?;
        let required = ["ANY", "CHANNEL", "NOSUSPEND", "COMPSTATUS", "ABCODE"];
        if parsed
            .keys()
            .any(|key| !required.contains(&key.as_str()) && key != "RESP" && key != "RESP2")
            || required.iter().any(|name| !parsed.contains_key(*name))
        {
            return Err("FETCH requires ANY CHANNEL NOSUSPEND COMPSTATUS ABCODE".into());
        }
        if opt(&parsed, "NOSUSPEND").is_some() {
            return Err("FETCH NOSUSPEND takes no value".into());
        }
        let values: Vec<_> = ["ANY", "CHANNEL", "COMPSTATUS", "ABCODE"]
            .iter()
            .map(|name| identifier(opt(&parsed, name), name))
            .collect::<Result<_, _>>()?;
        result.extend([
            emit("           CALL STATIC \"COBCICS_FETCH_ANY\""),
            emit(&format!("             USING {} {}", values[0], values[1])),
            emit(&format!("                   {} {}", values[2], values[3])),
            emit("                   EIBRESP EIBRESP2"),
        ]);
        require_normal(&mut result, &parsed);
        result.extend(targets(&parsed)?);
        finish(&mut result, period);
        bounded(&result, "FETCH")?;
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^(DEFINE|DELETE|GET|QUERY)\s+COUNTER\b(.*)$")
        .unwrap()
        .captures(body)
    {
        return counter_statement(&upper(&caps[1]), &caps[2], period);
    }
    if let Some(caps) = Regex::new(r"(?is)^(GET|PUT)\s+CONTAINER\b(.*)$")
        .unwrap()
        .captures(body)
    {
        return container_statement(&upper(&caps[1]), &caps[2], period);
    }
    if let Some(caps) = Regex::new(r"(?is)^RECEIVE\b(.*)$").unwrap().captures(body) {
        return receive_statement(&caps[1], period);
    }
    if let Some(caps) = Regex::new(r"(?is)^(ENQ|DEQ)\b(.*)$")
        .unwrap()
        .captures(body)
    {
        return enqueue_statement(&upper(&caps[1]), &caps[2], period);
    }
    if let Some(caps) = Regex::new(r"(?is)^(READ|WRITE|REWRITE|DELETE)\b(.*)$")
        .unwrap()
        .captures(body)
    {
        return file_statement(&upper(&caps[1]), &caps[2], period);
    }
    if let Some(caps) = Regex::new(r"(?is)^(READQ|WRITEQ|DELETEQ)\s+(TS|TD)\b(.*)$")
        .unwrap()
        .captures(body)
    {
        return queue_statement(&upper(&caps[1]), &upper(&caps[2]), &caps[3], period);
    }
    if let Some(caps) = Regex::new(r"(?is)^RETURN\b(.*)$").unwrap().captures(body) {
        let parsed = options(&caps[1])?;
        only(
            &parsed,
            &[
                "TRANSID",
                "COMMAREA",
                "LENGTH",
                "IMMEDIATE",
                "RESP",
                "RESP2",
            ],
            "RETURN",
        )?;
        if opt(&parsed, "IMMEDIATE").is_some() {
            return Err("RETURN IMMEDIATE does not take a value".into());
        }
        if parsed.contains_key("TRANSID") {
            let raw = opt(&parsed, "TRANSID").ok_or("TRANSID requires a literal or 4-byte area")?;
            let transid = if matches(raw, r"(?i)^'[A-Z0-9]{1,4}'$") {
                format!("\"{:<4}\"", upper(&raw[1..raw.len() - 1]))
            } else {
                identifier(Some(raw), "TRANSID")?
            };
            if parsed.contains_key("LENGTH") && !parsed.contains_key("COMMAREA") {
                return Err("RETURN LENGTH requires COMMAREA".into());
            }
            let area = if parsed.contains_key("COMMAREA") {
                identifier(opt(&parsed, "COMMAREA"), "COMMAREA")?
            } else {
                "COBCICS-COMMAREA".into()
            };
            let default = if parsed.contains_key("COMMAREA") {
                format!("LENGTH OF {area}")
            } else {
                "0".into()
            };
            let length = opt(&parsed, "LENGTH").unwrap_or(&default);
            if !matches(
                length,
                r"(?i)^(?:[A-Z][A-Z0-9-]*|[0-9]+|LENGTH OF [A-Z][A-Z0-9-]*)$",
            ) {
                return Err("invalid RETURN LENGTH".into());
            }
            if length.parse::<usize>().is_ok_and(|number| number > 2048) {
                return Err("RETURN COMMAREA exceeds 2048 bytes".into());
            }
            result.push(emit(&format!("           MOVE {length} TO COBCICS-LENGTH")));
            result.push(emit("           CALL STATIC \"COBCICS_SET_RETURN\""));
            result.push(emit(&format!(
                "             USING {transid} {area} COBCICS-LENGTH"
            )));
            if parsed.contains_key("IMMEDIATE") {
                result.push(emit("           CALL STATIC \"COBCICS_SET_IMMEDIATE\""));
            }
        } else if parsed.keys().any(|key| key != "RESP" && key != "RESP2") {
            return Err("RETURN options require TRANSID".into());
        }
        result.push(emit("           MOVE ZERO TO EIBRESP EIBRESP2"));
        result.extend(targets(&parsed)?);
        result.push(emit("           CALL STATIC \"COBCICS_RETURN\""));
        result.push(emit("           GOBACK"));
        finish(&mut result, period);
        bounded(&result, "RETURN")?;
        return Ok(result);
    }
    if let Some(caps) = Regex::new(r"(?is)^(LINK|XCTL)\b(.*)$")
        .unwrap()
        .captures(body)
    {
        let verb = upper(&caps[1]);
        let parsed = options(&caps[2])?;
        let mut allowed = vec!["PROGRAM", "COMMAREA", "LENGTH", "RESP", "RESP2", "NOHANDLE"];
        if verb == "LINK" {
            allowed.push("SYNCONRETURN");
        }
        only(&parsed, &allowed, &verb)?;
        for flag in ["SYNCONRETURN", "NOHANDLE"] {
            if opt(&parsed, flag).is_some() {
                return Err(format!("{flag} does not take a value"));
            }
        }
        if !parsed.contains_key("PROGRAM") || !parsed.contains_key("COMMAREA") {
            return Err(format!("{verb} requires PROGRAM and COMMAREA"));
        }
        let raw = opt(&parsed, "PROGRAM").ok_or("PROGRAM requires a COBOL name or literal")?;
        let program = if raw.starts_with('\'') {
            if !matches(raw, r"(?i)^'[A-Z][A-Z0-9- ]{0,7}'$") {
                return Err("PROGRAM requires a 1-8 character name".into());
            }
            format!("\"{}\"", upper(raw[1..raw.len() - 1].trim()))
        } else {
            identifier(Some(raw), "PROGRAM")?
        };
        let area = identifier(opt(&parsed, "COMMAREA"), "COMMAREA")?;
        result.extend([
            emit(&format!("           MOVE LENGTH OF {area}")),
            emit("             TO COBCICS-LINK-CAPACITY"),
        ]);
        if let Some(length) = opt(&parsed, "LENGTH") {
            if !matches(
                length,
                r"(?i)^(?:(?:LENGTH\s+OF\s+)?[A-Z][A-Z0-9-]*(?:\s+OF\s+[A-Z][A-Z0-9-]*)*|[0-9]+)$",
            ) {
                return Err("invalid LINK LENGTH".into());
            }
            if length.parse::<usize>().is_ok_and(|number| number > 32767) {
                return Err("LINK COMMAREA exceeds 32767 bytes".into());
            }
            result.push(emit(&format!("           MOVE {length} TO COBCICS-LENGTH")));
        } else {
            result.extend([
                emit(&format!("           MOVE LENGTH OF {area}")),
                emit("             TO COBCICS-LENGTH"),
            ]);
        }
        result.extend([
            emit("           CALL STATIC \"COBCICS_LINK_ENTER\""),
            emit("             USING COBCICS-LENGTH"),
            emit("                   COBCICS-LINK-CAPACITY"),
            emit("           CALL STATIC \"COBCICS_LINK_CALLER\""),
            emit(&format!(
                "             USING \"{:<8}\"",
                program_id.unwrap_or("")
            )),
            emit(&format!("           CALL {program} USING {area}")),
            emit("           CALL STATIC \"COBCICS_LINK_EXIT\""),
            emit("           MOVE ZERO TO EIBRESP EIBRESP2"),
        ]);
        if verb == "XCTL" {
            result.extend([
                emit("           CALL STATIC \"COBCICS_RETURN\""),
                emit("           GOBACK"),
            ]);
        } else {
            result.extend(targets(&parsed)?);
        }
        finish(&mut result, period);
        bounded(&result, "LINK")?;
        return Ok(result);
    }
    Err(format!("unsupported EXEC CICS command: {body}"))
}

fn counter_statement(verb: &str, tail: &str, period: bool) -> Result<Vec<String>, String> {
    let parsed = options(&format!("COUNTER{tail}"))?;
    let mut allowed = vec!["COUNTER", "POOL", "RESP", "RESP2"];
    if verb != "DELETE" {
        allowed.push("VALUE");
    }
    only(&parsed, &allowed, &format!("{verb} COUNTER"))?;
    let name = identifier(opt(&parsed, "COUNTER"), "COUNTER")?;
    let pool = identifier(opt(&parsed, "POOL"), "POOL")?;
    let mut lines = vec![
        emit(&format!("           MOVE {name} TO COBCICS-COUNTER-NAME")),
        emit(&format!("           MOVE {pool} TO COBCICS-COUNTER-POOL")),
    ];
    let value = if verb == "DELETE" {
        "COBCICS-COUNTER-VALUE".into()
    } else if verb == "DEFINE"
        && opt(&parsed, "VALUE")
            .is_some_and(|value| value.bytes().all(|byte| byte.is_ascii_digit()))
    {
        lines.extend([
            emit(&format!(
                "           MOVE {}",
                opt(&parsed, "VALUE").unwrap()
            )),
            emit("             TO COBCICS-COUNTER-VALUE"),
        ]);
        "COBCICS-COUNTER-VALUE".into()
    } else {
        identifier(opt(&parsed, "VALUE"), "VALUE")?
    };
    lines.extend([
        emit(&format!(
            "           CALL STATIC \"COBCICS_COUNTER_{verb}\""
        )),
        emit("             USING COBCICS-COUNTER-NAME"),
        emit("                   COBCICS-COUNTER-POOL"),
        emit(&format!("                   {value} EIBRESP EIBRESP2")),
    ]);
    require_normal(&mut lines, &parsed);
    lines.extend(targets(&parsed)?);
    finish(&mut lines, period);
    bounded(&lines, &format!("{verb} COUNTER"))?;
    Ok(lines)
}

fn container_statement(verb: &str, tail: &str, period: bool) -> Result<Vec<String>, String> {
    let parsed = options(&format!("CONTAINER{tail}"))?;
    let direction = if verb == "GET" { "INTO" } else { "FROM" };
    only(
        &parsed,
        &[
            "CONTAINER",
            "CHANNEL",
            direction,
            "FLENGTH",
            "RESP",
            "RESP2",
        ],
        &format!("{verb} CONTAINER"),
    )?;
    let name = identifier(opt(&parsed, "CONTAINER"), "CONTAINER")?;
    let channel = if parsed.contains_key("CHANNEL") {
        identifier(opt(&parsed, "CHANNEL"), "CHANNEL")?
    } else {
        "COBCICS-CURRENT-CHANNEL".into()
    };
    let data = identifier(opt(&parsed, direction), direction)?;
    let length = if parsed.contains_key("FLENGTH") {
        identifier(opt(&parsed, "FLENGTH"), "FLENGTH")?
    } else {
        "COBCICS-DATA-LENGTH".into()
    };
    let mut lines = vec![
        emit(&format!("           MOVE LENGTH OF {data}")),
        emit("             TO COBCICS-DATA-CAPACITY"),
    ];
    if !parsed.contains_key("FLENGTH") {
        lines.extend([
            emit("           MOVE COBCICS-DATA-CAPACITY"),
            emit("             TO COBCICS-DATA-LENGTH"),
        ]);
    }
    lines.extend([
        emit(&format!(
            "           CALL STATIC \"COBCICS_{verb}_CONTAINER\""
        )),
        emit(&format!("             USING {name}")),
        emit(&format!("                   {data} {length}")),
        emit("                   COBCICS-DATA-CAPACITY"),
        emit(&format!("                   {channel}")),
        emit("                   EIBRESP EIBRESP2"),
    ]);
    require_normal(&mut lines, &parsed);
    lines.extend(targets(&parsed)?);
    finish(&mut lines, period);
    bounded(&lines, &format!("{verb} CONTAINER"))?;
    Ok(lines)
}

fn receive_statement(tail: &str, period: bool) -> Result<Vec<String>, String> {
    let parsed = options(tail)?;
    only(&parsed, &["INTO", "LENGTH", "RESP", "RESP2"], "RECEIVE")?;
    let area = identifier(opt(&parsed, "INTO"), "INTO")?;
    let length = identifier(opt(&parsed, "LENGTH"), "LENGTH")?;
    let mut lines = vec![
        emit(&format!("           MOVE LENGTH OF {area}")),
        emit("             TO COBCICS-LINK-CAPACITY"),
        emit("           CALL STATIC \"COBCICS_RECEIVE_INTO\""),
        emit(&format!("             USING {area} {length}")),
        emit("                   COBCICS-LINK-CAPACITY"),
        emit("                   EIBRESP EIBRESP2"),
    ];
    require_normal(&mut lines, &parsed);
    lines.extend(targets(&parsed)?);
    finish(&mut lines, period);
    bounded(&lines, "RECEIVE INTO")?;
    Ok(lines)
}

fn enqueue_statement(verb: &str, tail: &str, period: bool) -> Result<Vec<String>, String> {
    let parsed = options(tail)?;
    only(&parsed, &["RESOURCE", "LENGTH", "RESP", "RESP2"], verb)?;
    let resource = identifier(opt(&parsed, "RESOURCE"), "RESOURCE")?;
    let default = format!("LENGTH OF {resource}");
    let length = opt(&parsed, "LENGTH").unwrap_or(&default);
    if !matches(
        length,
        r"(?i)^(?:[0-9]+|[A-Z][A-Z0-9-]*|LENGTH OF [A-Z][A-Z0-9-]*)$",
    ) {
        return Err(format!("invalid {verb} LENGTH"));
    }
    let mut lines = vec![
        emit(&format!("           MOVE {length} TO COBCICS-LENGTH")),
        emit(&format!("           MOVE LENGTH OF {resource}")),
        emit("             TO COBCICS-LINK-CAPACITY"),
        emit(&format!("           CALL STATIC \"COBCICS_{verb}\"")),
        emit(&format!("             USING {resource} COBCICS-LENGTH")),
        emit("                   COBCICS-LINK-CAPACITY"),
        emit("                   EIBRESP EIBRESP2"),
    ];
    require_normal(&mut lines, &parsed);
    lines.extend(targets(&parsed)?);
    finish(&mut lines, period);
    Ok(lines)
}

fn file_statement(verb: &str, tail: &str, period: bool) -> Result<Vec<String>, String> {
    let mut parsed = options(tail)?;
    if let Some(dataset) = parsed.remove("DATASET") {
        if parsed.contains_key("FILE") {
            return Err(format!("{verb} cannot specify FILE and DATASET"));
        }
        parsed.insert("FILE".into(), dataset);
    }
    if verb == "WRITE" && opt(&parsed, "FILE").is_some_and(|name| upper(name) == "'ABNDFILE'") {
        only(
            &parsed,
            &["FILE", "FROM", "RIDFLD", "RESP", "RESP2"],
            "WRITE",
        )?;
        let area = identifier(opt(&parsed, "FROM"), "FROM")?;
        let key = identifier(opt(&parsed, "RIDFLD"), "RIDFLD")?;
        let mut lines = vec![
            emit(&format!("           MOVE LENGTH OF {area}")),
            emit("             TO COBCICS-LENGTH"),
            emit(&format!("           MOVE LENGTH OF {key}")),
            emit("             TO COBCICS-LINK-CAPACITY"),
            emit("           CALL STATIC \"COBCICS_WRITE_ABNDFILE\""),
            emit(&format!("             USING {area} COBCICS-LENGTH")),
            emit(&format!("                   {key} COBCICS-LINK-CAPACITY")),
            emit("                   EIBRESP EIBRESP2"),
        ];
        require_normal(&mut lines, &parsed);
        lines.extend(targets(&parsed)?);
        finish(&mut lines, period);
        bounded(&lines, "WRITE")?;
        return Ok(lines);
    }
    let allowed = match verb {
        "READ" => vec![
            "FILE",
            "LENGTH",
            "RESP",
            "RESP2",
            "INTO",
            "RIDFLD",
            "KEYLENGTH",
            "GTEQ",
            "GENERIC",
            "UPDATE",
        ],
        "WRITE" => vec![
            "FILE",
            "LENGTH",
            "RESP",
            "RESP2",
            "FROM",
            "RIDFLD",
            "KEYLENGTH",
        ],
        "REWRITE" => vec!["FILE", "LENGTH", "RESP", "RESP2", "FROM"],
        _ => vec!["FILE", "RIDFLD", "KEYLENGTH", "RESP", "RESP2"],
    };
    only(&parsed, &allowed, &format!("{verb} FILE"))?;
    let raw = opt(&parsed, "FILE").unwrap_or("");
    let source_name = if matches(raw, r"(?i)^'[A-Z0-9]{1,8}'$") {
        format!("\"{}\"", upper(&raw[1..raw.len() - 1]))
    } else {
        identifier(Some(raw), "FILE")?
    };
    let mut lines = vec![
        emit(&format!("           MOVE {source_name}")),
        emit("             TO COBCICS-FILE-NAME"),
    ];
    let mut area = String::new();
    let mut raw_length = "";
    if verb != "DELETE" {
        let direction = if verb == "READ" { "INTO" } else { "FROM" };
        area = identifier(opt(&parsed, direction), direction)?;
        raw_length = opt(&parsed, "LENGTH").unwrap_or("");
        if !matches(
            raw_length,
            r"(?i)^(?:[0-9]+|[A-Z][A-Z0-9-]*(?:\s+OF\s+[A-Z][A-Z0-9-]*)*|LENGTH\s+OF\s+[A-Z][A-Z0-9-]*)$",
        ) {
            return Err(format!("{verb} FILE requires valid LENGTH"));
        }
        lines.extend([
            emit(&format!("           MOVE {raw_length}")),
            emit("             TO COBCICS-FILE-LENGTH"),
            emit(&format!("           MOVE LENGTH OF {area}")),
            emit("             TO COBCICS-FILE-CAPACITY"),
        ]);
    }
    let mut key = String::new();
    if verb != "REWRITE" {
        key = identifier(opt(&parsed, "RIDFLD"), "RIDFLD")?;
        let length = opt(&parsed, "KEYLENGTH").unwrap_or("");
        if !matches(
            length,
            r"(?i)^(?:[0-9]+|[A-Z][A-Z0-9-]*|LENGTH\s+OF\s+[A-Z][A-Z0-9-]*)$",
        ) {
            return Err(format!("{verb} FILE requires KEYLENGTH"));
        }
        lines.extend([
            emit(&format!("           MOVE {length}")),
            emit("             TO COBCICS-FILE-KEY-LENGTH"),
            emit(&format!("           MOVE LENGTH OF {key}")),
            emit("             TO COBCICS-FILE-KEY-CAPACITY"),
        ]);
    }
    match verb {
        "READ" => {
            for flag in ["GTEQ", "GENERIC", "UPDATE"] {
                if opt(&parsed, flag).is_some() {
                    return Err(format!("{flag} does not take a value"));
                }
            }
            if parsed.contains_key("GENERIC") && !parsed.contains_key("GTEQ") {
                return Err("READ FILE GENERIC requires GTEQ".into());
            }
            if parsed.contains_key("UPDATE") && parsed.contains_key("GTEQ") {
                return Err("READ FILE UPDATE cannot use GTEQ".into());
            }
            let mode = (if parsed.contains_key("GTEQ") { 1 } else { 0 })
                | (if parsed.contains_key("GENERIC") { 2 } else { 0 })
                | (if parsed.contains_key("UPDATE") { 4 } else { 0 });
            lines.extend([
                emit("           CALL STATIC \"COBCICS_FILE_READ\""),
                emit("             USING COBCICS-FILE-NAME"),
                emit(&format!("                   {area} COBCICS-FILE-LENGTH")),
                emit("                   COBCICS-FILE-CAPACITY"),
                emit(&format!("                   {key}")),
                emit("                   COBCICS-FILE-KEY-LENGTH"),
                emit("                   COBCICS-FILE-KEY-CAPACITY"),
                emit(&format!(
                    "                   X'{mode:02X}' EIBRESP EIBRESP2"
                )),
            ]);
            if raw_length.parse::<u32>().is_err() && !upper(raw_length).starts_with("LENGTH OF ") {
                lines.extend([
                    emit("           MOVE COBCICS-FILE-LENGTH"),
                    emit(&format!("             TO {raw_length}")),
                ]);
            }
        }
        "WRITE" => lines.extend([
            emit("           CALL STATIC \"COBCICS_FILE_WRITE\""),
            emit("             USING COBCICS-FILE-NAME"),
            emit(&format!("                   {area} COBCICS-FILE-LENGTH")),
            emit("                   COBCICS-FILE-CAPACITY"),
            emit(&format!("                   {key}")),
            emit("                   COBCICS-FILE-KEY-LENGTH"),
            emit("                   COBCICS-FILE-KEY-CAPACITY"),
            emit("                   EIBRESP EIBRESP2"),
        ]),
        "REWRITE" => lines.extend([
            emit("           CALL STATIC \"COBCICS_FILE_REWRITE\""),
            emit("             USING COBCICS-FILE-NAME"),
            emit(&format!("                   {area} COBCICS-FILE-LENGTH")),
            emit("                   COBCICS-FILE-CAPACITY"),
            emit("                   EIBRESP EIBRESP2"),
        ]),
        _ => lines.extend([
            emit("           CALL STATIC \"COBCICS_FILE_DELETE\""),
            emit("             USING COBCICS-FILE-NAME"),
            emit(&format!("                   {key}")),
            emit("                   COBCICS-FILE-KEY-LENGTH"),
            emit("                   COBCICS-FILE-KEY-CAPACITY"),
            emit("                   EIBRESP EIBRESP2"),
        ]),
    }
    require_normal(&mut lines, &parsed);
    lines.extend(targets(&parsed)?);
    finish(&mut lines, period);
    bounded(&lines, &format!("{verb} FILE"))?;
    Ok(lines)
}

fn queue_statement(
    verb: &str,
    kind: &str,
    tail: &str,
    period: bool,
) -> Result<Vec<String>, String> {
    let parsed = options(tail)?;
    if kind == "TD" && verb != "WRITEQ" {
        return Err(format!("unsupported {verb} TD"));
    }
    let mut allowed = vec!["QUEUE", "RESP", "RESP2", "NOHANDLE"];
    if verb == "READQ" {
        allowed.extend(["INTO", "LENGTH", "ITEM", "NEXT"]);
    } else if verb == "WRITEQ" {
        allowed.extend(["FROM", "LENGTH"]);
        if kind == "TS" {
            allowed.extend(["ITEM", "REWRITE", "NOSUSPEND"]);
        }
    }
    only(&parsed, &allowed, &format!("{verb} {kind}"))?;
    if opt(&parsed, "NOHANDLE").is_some() {
        return Err("NOHANDLE does not take a value".into());
    }
    let raw_name = opt(&parsed, "QUEUE").unwrap_or("");
    let name = if matches(raw_name, r"(?i)^'[A-Z0-9]{1,8}'$") {
        format!("\"{:<8}\"", upper(&raw_name[1..raw_name.len() - 1]))
    } else {
        identifier(Some(raw_name), "QUEUE")?
    };
    let mut lines = vec![emit(&format!(
        "           MOVE {name} TO COBCICS-QUEUE-NAME"
    ))];
    if verb == "DELETEQ" {
        lines.extend([
            emit("           CALL STATIC \"COBCICS_TSQ_DELETE\""),
            emit("             USING COBCICS-QUEUE-NAME"),
            emit("                   EIBRESP EIBRESP2"),
        ]);
    } else {
        let direction = if verb == "READQ" { "INTO" } else { "FROM" };
        let area = identifier(opt(&parsed, direction), direction)?;
        lines.extend([
            emit(&format!("           MOVE LENGTH OF {area}")),
            emit("             TO COBCICS-QUEUE-CAPACITY"),
        ]);
        let raw_length = opt(&parsed, "LENGTH");
        let length = if let Some(raw) = raw_length {
            if raw.bytes().all(|byte| byte.is_ascii_digit())
                || matches(raw, r"(?i)^LENGTH\s+OF\s+[A-Z][A-Z0-9-]*$")
            {
                raw.to_string()
            } else {
                identifier(Some(raw), "LENGTH")?
            }
        } else {
            "COBCICS-QUEUE-CAPACITY".into()
        };
        lines.extend([
            emit(&format!("           MOVE {length}")),
            emit("             TO COBCICS-QUEUE-LENGTH"),
        ]);
        if verb == "READQ" && parsed.contains_key("ITEM") && parsed.contains_key("NEXT") {
            return Err("READQ TS cannot use ITEM and NEXT together".into());
        }
        for flag in ["NEXT", "REWRITE", "NOSUSPEND"] {
            if opt(&parsed, flag).is_some() {
                return Err(format!("{flag} does not take a value"));
            }
        }
        let item = opt(&parsed, "ITEM").unwrap_or("0");
        if !matches(item, r"(?i)^(?:[0-9]+|[A-Z][A-Z0-9-]*)$") {
            return Err("ITEM requires a number or data name".into());
        }
        if kind == "TS" {
            lines.push(emit(&format!(
                "           MOVE {item} TO COBCICS-QUEUE-ITEM"
            )));
            let routine = if verb == "READQ" {
                "COBCICS_TSQ_READ"
            } else {
                "COBCICS_TSQ_WRITE"
            };
            let mode = if (verb == "READQ" && parsed.contains_key("NEXT"))
                || (verb == "WRITEQ" && parsed.contains_key("REWRITE"))
            {
                "01"
            } else {
                "00"
            };
            lines.extend([
                emit(&format!("           CALL STATIC \"{routine}\"")),
                emit(&format!("             USING COBCICS-QUEUE-NAME {area}")),
                emit("                   COBCICS-QUEUE-LENGTH"),
                emit("                   COBCICS-QUEUE-CAPACITY"),
                emit("                   COBCICS-QUEUE-ITEM"),
                emit(&format!("                   X'{mode}' EIBRESP EIBRESP2")),
            ]);
        } else {
            lines.extend([
                emit("           CALL STATIC \"COBCICS_TDQ_WRITE\""),
                emit(&format!("             USING COBCICS-QUEUE-NAME {area}")),
                emit("                   COBCICS-QUEUE-LENGTH"),
                emit("                   COBCICS-QUEUE-CAPACITY"),
                emit("                   EIBRESP EIBRESP2"),
            ]);
        }
        if verb == "READQ"
            && raw_length.is_some_and(|raw| {
                raw.parse::<u32>().is_err() && !matches(raw, r"(?i)^LENGTH\s+OF\s+[A-Z][A-Z0-9-]*$")
            })
        {
            lines.extend([
                emit("           MOVE COBCICS-QUEUE-LENGTH"),
                emit(&format!("             TO {}", raw_length.unwrap())),
            ]);
        }
    }
    require_normal(&mut lines, &parsed);
    lines.extend(targets(&parsed)?);
    finish(&mut lines, period);
    bounded(&lines, &format!("{verb} {kind}"))?;
    Ok(lines)
}

fn separator(value: Option<&str>, default: char, name: &str) -> Result<String, String> {
    match value {
        None => Ok(format!("\"{default}\"")),
        Some(raw)
            if raw.starts_with('\'')
                && raw.ends_with('\'')
                && raw.len() == 3
                && (b' '..=b'~').contains(&raw.as_bytes()[1]) =>
        {
            Ok(format!("\"{}\"", &raw[1..2]))
        }
        _ => Err(format!("{name} requires a single character literal")),
    }
}

fn map_statement(
    verb: &str,
    tail: &str,
    maps: &Maps,
    period: bool,
    mapset: &str,
    mapfail_label: Option<&str>,
) -> Result<Vec<String>, String> {
    let parsed = options(&format!("MAP{tail}"))?;
    let target = if verb == "SEND" { "FROM" } else { "INTO" };
    let mut allowed = vec!["MAP", "MAPSET", "RESP", "RESP2", "NOHANDLE", target];
    if verb == "SEND" {
        allowed.extend(["ERASE", "DATAONLY", "ALARM", "FREEKB", "CURSOR", "MAPONLY"]);
    } else {
        allowed.extend(["TERMINAL", "ASIS"]);
    }
    only(&parsed, &allowed, &format!("{verb} MAP"))?;
    if opt(&parsed, "NOHANDLE").is_some() {
        return Err("NOHANDLE does not take a value".into());
    }
    let raw_name = opt(&parsed, "MAP").ok_or("MAP requires a quoted literal name")?;
    if !matches(raw_name, r"(?i)^'[A-Z][A-Z0-9-]*'$") {
        return Err("MAP requires a quoted literal name".into());
    }
    let mut name = upper(&raw_name[1..raw_name.len() - 1]);
    if !maps.fields.contains_key(&name)
        && verb == "SEND"
        && parsed.contains_key("MAPONLY")
        && upper(mapset) == name
        && maps.fields.len() == 1
    {
        name = maps.fields.keys().next().unwrap().clone();
    }
    if !maps.fields.contains_key(&name)
        && verb == "SEND"
        && parsed.contains_key("MAPONLY")
        && name.ends_with('M')
        && maps.fields.contains_key(&name[..name.len() - 1])
    {
        name.pop();
    }
    let fields = maps
        .fields
        .get(&name)
        .ok_or_else(|| format!("symbolic map not found: {name}"))?;
    if parsed.contains_key(target)
        && identifier(opt(&parsed, target), target)?
            != format!("{name}{}", if verb == "SEND" { "O" } else { "I" })
    {
        return Err(format!(
            "{target} must name {name}{}",
            if verb == "SEND" { "O" } else { "I" }
        ));
    }
    if let Some(raw) = opt(&parsed, "MAPSET") {
        if !matches(raw, r"(?i)^'[A-Z][A-Z0-9-]*'$") {
            return Err("MAPSET requires a quoted literal name".into());
        }
        if !mapset.is_empty() && upper(&raw[1..raw.len() - 1]) != upper(mapset) {
            return Err(format!("MAPSET does not match {mapset}"));
        }
    } else if parsed.contains_key("MAPSET") {
        return Err("MAPSET requires a quoted literal name".into());
    }
    let flags = if verb == "SEND" {
        vec!["ERASE", "DATAONLY", "ALARM", "FREEKB", "CURSOR", "MAPONLY"]
    } else {
        vec!["TERMINAL", "ASIS"]
    };
    for flag in flags {
        if opt(&parsed, flag).is_some() {
            return Err(format!("{flag} does not take a value"));
        }
    }
    if verb == "SEND" {
        if parsed.contains_key("MAPONLY") && parsed.contains_key("DATAONLY") {
            return Err("MAPONLY and DATAONLY cannot be combined".into());
        }
        if parsed.contains_key("ERASE") && parsed.contains_key("DATAONLY") {
            return Err("ERASE and DATAONLY cannot be combined".into());
        }
        if parsed.contains_key("ALARM") && !parsed.contains_key("DATAONLY") {
            return Err("ALARM currently requires DATAONLY".into());
        }
    }
    let mut lines = Vec::new();
    if verb == "RECEIVE" {
        let mode = if parsed.contains_key("ASIS") {
            "01"
        } else {
            "00"
        };
        lines.extend([
            emit("           CALL STATIC \"COBCICS_SET_RECEIVE_ASIS\""),
            emit(&format!("             USING X'{mode}'")),
            emit("           CALL STATIC \"COBCICS_RECEIVE_STATUS\""),
            emit("             USING EIBRESP EIBRESP2"),
        ]);
        if let Some(label) = mapfail_label {
            if !parsed.contains_key("NOHANDLE") {
                lines.extend([
                    emit("           IF EIBRESP = 36"),
                    emit(&format!("             GO TO {label}")),
                    emit("           END-IF"),
                ]);
            }
        }
        require_normal(&mut lines, &parsed);
    } else {
        lines.push(emit("           MOVE ZERO TO EIBRESP EIBRESP2"));
    }
    lines.extend(targets(&parsed)?);
    if verb == "RECEIVE" {
        lines.push(emit("           IF EIBRESP = ZERO"));
    } else {
        let action = if parsed.contains_key("ALARM") {
            3
        } else if parsed.contains_key("DATAONLY") {
            2
        } else {
            0
        };
        lines.extend([
            emit("           CALL STATIC \"COBCICS_SEND_MAP\""),
            emit(&format!("             USING X'{action:02X}'")),
        ]);
    }
    if !parsed.contains_key("MAPONLY") {
        for (field, count) in fields {
            if field.len() > 8 {
                return Err(format!("field name exceeds pipe ABI: {field}"));
            }
            for index in 1..=*count {
                let wire = if *count == 1 {
                    field.clone()
                } else {
                    format!("{:<4}{index:04}", &field[..field.len().min(4)])
                };
                let literal = format!("\"{wire:<8}\"");
                let suffix = if *count == 1 {
                    String::new()
                } else {
                    format!("({index})")
                };
                if verb == "RECEIVE" {
                    lines.extend([
                        emit(&format!("           MOVE LENGTH OF {field}I{suffix}")),
                        emit(&format!("             TO {field}L{suffix}")),
                        emit("           CALL STATIC \"COBCICS_GET_FIELD\""),
                        emit(&format!("             USING {literal}")),
                        emit(&format!("                   {field}I{suffix}")),
                        emit(&format!("                   {field}L{suffix}")),
                        emit(&format!("                   {field}F{suffix}")),
                    ]);
                } else {
                    lines.extend([
                        emit(&format!("           MOVE LENGTH OF {field}O{suffix}")),
                        emit("             TO COBCICS-LENGTH"),
                        emit("           CALL STATIC \"COBCICS_SEND_FIELD\""),
                        emit(&format!("             USING {literal}")),
                        emit(&format!("                   {field}O{suffix}")),
                        emit("                   COBCICS-LENGTH"),
                        emit("           CALL STATIC \"COBCICS_SEND_ATTRIBUTE\""),
                        emit(&format!("             USING {literal}")),
                        emit(&format!("                   {field}A{suffix}")),
                    ]);
                    if let Some(extended) = maps.extended.get(&(name.clone(), field.clone())) {
                        for attribute in extended {
                            lines.extend([
                                emit("           CALL STATIC \"COBCICS_SEND_EXT_ATTRIBUTE\""),
                                emit(&format!("             USING {literal}")),
                                emit(&format!("                   X'{:02X}'", *attribute as u32)),
                                emit(&format!("                   {field}{attribute}{suffix}")),
                            ]);
                        }
                    }
                }
            }
        }
    }
    if verb == "RECEIVE" {
        lines.push(emit("           END-IF"));
    }
    finish(&mut lines, period);
    bounded(&lines, &format!("{verb} MAP"))?;
    Ok(lines)
}
