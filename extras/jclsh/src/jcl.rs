use std::collections::{HashMap, HashSet, VecDeque};

use super::{Dd, DdKind, Job, Result, Step};

pub(super) type Catalog = HashMap<String, HashMap<String, String>>;

#[derive(Clone, Debug)]
pub(super) struct CondTest {
    pub code: i32,
    pub operator: String,
    pub step: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CondMode {
    Normal,
    Even,
    Only,
}

#[derive(Clone, Debug)]
pub(super) struct CondSpec {
    pub tests: Vec<CondTest>,
    pub mode: CondMode,
}

#[derive(Clone, Debug)]
pub(super) struct IfGuard {
    pub expression: String,
    pub then_branch: bool,
}

#[derive(Clone, Debug)]
struct Card {
    line: usize,
    name: String,
    operation: String,
    operands: String,
    data: Vec<String>,
}

#[derive(Clone, Debug)]
struct RawDd {
    name: String,
    cards: Vec<Card>,
}

#[derive(Clone, Debug)]
struct RawStep {
    name: String,
    line: usize,
    operands: String,
    dds: Vec<RawDd>,
    guards: Vec<IfGuard>,
    symbols: HashMap<String, String>,
    inherited_conditions: Vec<CondSpec>,
    call_stack: Vec<String>,
    local_steps: HashSet<String>,
}

#[derive(Clone, Debug)]
struct Procedure {
    defaults: HashMap<String, String>,
    steps: Vec<RawStep>,
}

fn name(value: &str, line: usize) -> Result<String> {
    if let Some(site) = value
        .strip_prefix('<')
        .and_then(|text| text.strip_suffix('>'))
    {
        if !site.is_empty()
            && site.len() <= 32
            && site.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Ok(value.to_ascii_uppercase());
        }
    }
    if value.is_empty()
        || value.len() > 20
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"$#@_.".contains(&byte))
    {
        return Err(format!("line {line}: invalid JCL name {value:?}"));
    }
    Ok(value.to_ascii_uppercase())
}

fn split_top(value: &str) -> Result<Vec<String>> {
    let mut parts = Vec::new();
    let mut quote = false;
    let mut depth = 0_i32;
    let mut start = 0;
    for (index, c) in value.char_indices() {
        if c == '\'' {
            quote = !quote;
        } else if !quote {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth < 0 {
                        return Err("unbalanced JCL parentheses".to_string());
                    }
                }
                ',' if depth == 0 => {
                    parts.push(value[start..index].trim().to_string());
                    start = index + 1;
                }
                _ => {}
            }
        }
    }
    if quote || depth != 0 {
        return Err("unterminated JCL quote or parentheses".to_string());
    }
    parts.push(value[start..].trim().to_string());
    Ok(parts)
}

fn instream_delimiter(operands: &str) -> Result<Option<String>> {
    let parts = split_top(operands)?;
    if parts.first().is_some_and(|part| part == "*") {
        return Ok(Some("/*".to_string()));
    }
    if parts.first().is_some_and(|part| part == "DATA") {
        let delimiter = parts
            .iter()
            .find_map(|part| part.strip_prefix("DLM="))
            .unwrap_or("/*");
        return Ok(Some(delimiter.to_string()));
    }
    Ok(None)
}

fn cards(source: &str) -> Result<Vec<Card>> {
    let mut result: Vec<Card> = Vec::new();
    let mut inline: Option<String> = None;
    for (index, raw) in source.lines().enumerate() {
        let line = index + 1;
        let raw = raw.trim_end_matches('\r');
        // JCL columns 73-80 are optional sequence characters, not operands.
        let raw = if raw.starts_with("//") {
            raw.char_indices()
                .nth(72)
                .map_or(raw, |(end, _)| &raw[..end])
        } else {
            raw
        };
        if let Some(delimiter) = &inline {
            if raw.trim() == delimiter {
                inline = None;
                continue;
            }
            if delimiter != "/*" || !raw.starts_with("//") {
                result.last_mut().unwrap().data.push(raw.to_string());
                continue;
            }
            inline = None;
        }
        if raw.trim().is_empty() || raw.trim() == "/*" || raw.starts_with("//*") {
            continue;
        }
        let card = raw
            .strip_prefix("//")
            .ok_or_else(|| format!("line {line}: expected // JCL statement"))?;
        if card.trim().is_empty() {
            continue;
        }
        let (card_name, body) = if card.starts_with(char::is_whitespace) {
            (String::new(), card.trim())
        } else {
            let end = card.find(char::is_whitespace).unwrap_or(card.len());
            (name(&card[..end], line)?, card[end..].trim())
        };
        let split = body.find(char::is_whitespace).unwrap_or(body.len());
        let operation = body[..split].to_ascii_uppercase();
        let operands = body[split..].trim();
        let known = matches!(
            operation.as_str(),
            "JOB"
                | "EXEC"
                | "DD"
                | "PROC"
                | "PEND"
                | "SET"
                | "IF"
                | "ELSE"
                | "ENDIF"
                | "JCLLIB"
                | "INCLUDE"
        );
        if card_name.is_empty() && !known {
            let previous = result
                .last_mut()
                .ok_or_else(|| format!("line {line}: orphan continuation"))?;
            if !previous.operands.ends_with(',') {
                return Err(format!("line {line}: unexpected JCL continuation"));
            }
            previous.operands.push_str(body);
            if previous.operation == "DD" {
                inline = instream_delimiter(&previous.operands)?;
            }
            continue;
        }
        if operation.is_empty() {
            return Err(format!("line {line}: missing JCL operation"));
        }
        result.push(Card {
            line,
            name: card_name,
            operation: operation.clone(),
            operands: operands.to_string(),
            data: Vec::new(),
        });
        if operation == "DD" {
            inline = instream_delimiter(operands)?;
        }
    }
    if inline.is_some() && inline.as_deref() != Some("/*") {
        return Err("unterminated DD DATA delimiter".to_string());
    }
    Ok(result)
}

fn library_order(card: &Card) -> Result<Vec<String>> {
    let value = card
        .operands
        .strip_prefix("ORDER=")
        .ok_or_else(|| format!("line {}: unsupported JCLLIB operand", card.line))?;
    let value = value
        .strip_prefix('(')
        .and_then(|v| v.strip_suffix(')'))
        .unwrap_or(value);
    split_top(value)?
        .iter()
        .map(|part| {
            let library = part.trim().trim_matches('\'').to_ascii_uppercase();
            if library.is_empty()
                || !library
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".$#@_<>".contains(&b))
            {
                return Err(format!("line {}: invalid JCLLIB library {part}", card.line));
            }
            Ok(library)
        })
        .collect()
}

fn catalog_member<'a>(
    catalog: &'a Catalog,
    order: &[String],
    member: &str,
    line: usize,
) -> Result<&'a str> {
    for library in order {
        if let Some(source) = catalog.get(library).and_then(|members| members.get(member)) {
            return Ok(source);
        }
    }
    Err(format!(
        "line {line}: member {member} not found in JCLLIB ORDER"
    ))
}

fn expand_includes(
    input: Vec<Card>,
    catalog: &Catalog,
    order: &mut Vec<String>,
    stack: &mut Vec<String>,
) -> Result<Vec<Card>> {
    let mut output = Vec::new();
    for card in input {
        if card.operation == "JCLLIB" {
            *order = library_order(&card)?;
            output.push(card);
            continue;
        }
        if card.operation != "INCLUDE" {
            output.push(card);
            continue;
        }
        let member = card
            .operands
            .strip_prefix("MEMBER=")
            .ok_or_else(|| format!("line {}: INCLUDE requires MEMBER=", card.line))?;
        let member = name(member.trim(), card.line)?;
        if stack.contains(&member) {
            return Err(format!("line {}: recursive INCLUDE {member}", card.line));
        }
        let source = catalog_member(catalog, order, &member, card.line)?;
        stack.push(member);
        output.extend(expand_includes(cards(source)?, catalog, order, stack)?);
        stack.pop();
    }
    Ok(output)
}

fn catalog_procedure(
    source: &str,
    requested: &str,
    catalog: &Catalog,
    order: &[String],
) -> Result<Procedure> {
    let mut member_cards = cards(source)?;
    let mut local_order = order.to_vec();
    member_cards = expand_includes(member_cards, catalog, &mut local_order, &mut Vec::new())?;
    if member_cards
        .first()
        .is_none_or(|card| card.operation != "PROC")
    {
        member_cards.insert(
            0,
            Card {
                line: 1,
                name: requested.to_string(),
                operation: "PROC".to_string(),
                operands: String::new(),
                data: Vec::new(),
            },
        );
    } else if member_cards[0].name.is_empty() {
        member_cards[0].name = requested.to_string();
    }
    if member_cards[0].name != requested {
        return Err(format!(
            "cataloged PROC member {requested} has mismatched name {}",
            member_cards[0].name
        ));
    }
    if member_cards
        .last()
        .is_none_or(|card| card.operation != "PEND")
    {
        member_cards.push(Card {
            line: 0,
            name: String::new(),
            operation: "PEND".to_string(),
            operands: String::new(),
            data: Vec::new(),
        });
    }
    member_cards.insert(
        0,
        Card {
            line: 0,
            name: "CATJOB".to_string(),
            operation: "JOB".to_string(),
            operands: "'CATALOG'".to_string(),
            data: Vec::new(),
        },
    );
    member_cards.push(Card {
        line: 0,
        name: "CATSTEP".to_string(),
        operation: "EXEC".to_string(),
        operands: "PGM=CATDUMMY".to_string(),
        data: Vec::new(),
    });
    let (_, _, _, _, _, mut procedures) = raw_job(member_cards)?;
    procedures
        .remove(requested)
        .ok_or_else(|| format!("cataloged PROC {requested} has no definition"))
}

fn assignments(text: &str, line: usize) -> Result<HashMap<String, String>> {
    let mut result = HashMap::new();
    if text.trim().is_empty() {
        return Ok(result);
    }
    for part in split_top(text)? {
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| format!("line {line}: expected symbol=value"))?;
        let key = name(key.trim(), line)?;
        if result
            .insert(key.clone(), value.trim().to_string())
            .is_some()
        {
            return Err(format!("line {line}: duplicate symbol {key}"));
        }
    }
    Ok(result)
}

fn substitute(input: &str, symbols: &HashMap<String, String>, line: usize) -> Result<String> {
    substitute_inner(input, symbols, line, &mut Vec::new())
}

fn substitute_inner(
    input: &str,
    symbols: &HashMap<String, String>,
    line: usize,
    visiting: &mut Vec<String>,
) -> Result<String> {
    let bytes = input.as_bytes();
    let mut result = String::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'&' {
            if bytes.get(index + 1) == Some(&b'&') {
                result.push_str("&&");
                index += 2;
                continue;
            }
            let start = index + 1;
            let mut end = start;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || b"$#@_".contains(&bytes[end]))
            {
                end += 1;
            }
            if end > start {
                let key = input[start..end].to_ascii_uppercase();
                let value = symbols
                    .get(&key)
                    .ok_or_else(|| format!("line {line}: unresolved JCL symbol &{key}"))?;
                if visiting.contains(&key) {
                    return Err(format!("line {line}: cyclic JCL symbol &{key}"));
                }
                visiting.push(key);
                result.push_str(&substitute_inner(value, symbols, line, visiting)?);
                visiting.pop();
                index = if bytes.get(end) == Some(&b'.') {
                    end + 1
                } else {
                    end
                };
                continue;
            }
        }
        let c = input[index..].chars().next().unwrap();
        result.push(c);
        index += c.len_utf8();
    }
    Ok(result)
}

fn add_dd(target: &mut Vec<RawDd>, card: Card) -> Result<()> {
    if card.name.is_empty() {
        let prior = target
            .last_mut()
            .ok_or_else(|| format!("line {}: DD concatenation without first DD", card.line))?;
        prior.cards.push(card);
    } else {
        if target.iter().any(|dd| dd.name == card.name) {
            return Err(format!("line {}: duplicate DD {}", card.line, card.name));
        }
        target.push(RawDd {
            name: card.name.clone(),
            cards: vec![card],
        });
    }
    Ok(())
}

type RawJob = (
    String,
    String,
    Option<CondSpec>,
    Vec<RawDd>,
    Vec<RawStep>,
    HashMap<String, Procedure>,
);

fn raw_job(cards: Vec<Card>) -> Result<RawJob> {
    let mut job_name = None;
    let mut attributes = String::new();
    let mut job_condition = None;
    let mut steps = Vec::new();
    let mut joblib = Vec::new();
    let mut procedures = HashMap::new();
    let mut current_proc: Option<(String, Procedure)> = None;
    let mut symbols = HashMap::new();
    let mut guards: Vec<IfGuard> = Vec::new();
    for card in cards {
        match card.operation.as_str() {
            "JOB" => {
                if job_name.is_some() || current_proc.is_some() {
                    return Err(format!("line {}: multiple or misplaced JOB", card.line));
                }
                job_name = Some(name(&card.name, card.line)?);
                attributes = card.operands.clone();
                for part in split_top(&card.operands)? {
                    if let Some(value) = part.strip_prefix("COND=") {
                        job_condition = Some(parse_cond(value, card.line)?);
                    }
                }
            }
            "PROC" => {
                if current_proc.is_some() {
                    return Err(format!("line {}: nested PROC is unsupported", card.line));
                }
                let proc_name = name(&card.name, card.line)?;
                if procedures.contains_key(&proc_name) {
                    return Err(format!("line {}: duplicate PROC {proc_name}", card.line));
                }
                current_proc = Some((
                    proc_name,
                    Procedure {
                        defaults: assignments(&card.operands, card.line)?,
                        steps: Vec::new(),
                    },
                ));
            }
            "PEND" => {
                let (proc_name, procedure) = current_proc
                    .take()
                    .ok_or_else(|| format!("line {}: PEND without PROC", card.line))?;
                procedures.insert(proc_name, procedure);
            }
            "SET" => {
                for (key, value) in assignments(&card.operands, card.line)? {
                    symbols.insert(key, value);
                }
            }
            "IF" => {
                let condition = card.operands.trim();
                let condition = condition
                    .strip_suffix("THEN")
                    .ok_or_else(|| format!("line {}: IF requires THEN", card.line))?
                    .trim();
                if condition.is_empty() {
                    return Err(format!("line {}: IF requires a condition", card.line));
                }
                guards.push(IfGuard {
                    expression: condition.to_string(),
                    then_branch: true,
                });
            }
            "ELSE" => {
                let last = guards
                    .last_mut()
                    .ok_or_else(|| format!("line {}: ELSE without IF", card.line))?;
                if !last.then_branch {
                    return Err(format!("line {}: duplicate ELSE", card.line));
                }
                last.then_branch = false;
            }
            "ENDIF" => {
                guards
                    .pop()
                    .ok_or_else(|| format!("line {}: ENDIF without IF", card.line))?;
            }
            "EXEC" => {
                let step = RawStep {
                    name: name(&card.name, card.line)?,
                    line: card.line,
                    operands: card.operands,
                    dds: Vec::new(),
                    guards: guards.clone(),
                    symbols: symbols.clone(),
                    inherited_conditions: Vec::new(),
                    call_stack: Vec::new(),
                    local_steps: HashSet::new(),
                };
                if let Some((_, procedure)) = current_proc.as_mut() {
                    procedure.steps.push(step);
                } else {
                    if job_name.is_none() {
                        return Err(format!("line {}: EXEC before JOB", card.line));
                    }
                    steps.push(step);
                }
            }
            "DD" => {
                if current_proc.is_none() && steps.is_empty() {
                    if card.name != "JOBLIB" && (joblib.is_empty() || !card.name.is_empty()) {
                        return Err(format!("line {}: DD before EXEC", card.line));
                    }
                    add_dd(&mut joblib, card)?;
                    continue;
                }
                let target = if let Some((_, procedure)) = current_proc.as_mut() {
                    &mut procedure.steps
                } else {
                    &mut steps
                };
                let step = target
                    .last_mut()
                    .ok_or_else(|| format!("line {}: DD before EXEC", card.line))?;
                add_dd(&mut step.dds, card)?;
            }
            "JCLLIB" => {
                if !card.operands.starts_with("ORDER=") {
                    return Err(format!("line {}: unsupported JCLLIB operand", card.line));
                }
                attributes.push_str(&format!(" JCLLIB({})", card.operands));
            }
            "INCLUDE" => {
                return Err(format!(
                    "line {}: INCLUDE requires a configured library",
                    card.line
                ))
            }
            other => {
                return Err(format!(
                    "line {}: unsupported JCL operation {other}",
                    card.line
                ))
            }
        }
    }
    if current_proc.is_some() {
        return Err("unterminated PROC".to_string());
    }
    if !guards.is_empty() {
        return Err("unterminated IF/THEN".to_string());
    }
    let job_name = job_name.ok_or("missing JOB statement")?;
    if steps.is_empty() {
        return Err("JOB has no EXEC step".to_string());
    }
    Ok((
        job_name,
        attributes,
        job_condition,
        joblib,
        steps,
        procedures,
    ))
}

fn parse_cond(input: &str, line: usize) -> Result<CondSpec> {
    let input = input.trim();
    if input == "EVEN" || input == "ONLY" {
        return Ok(CondSpec {
            tests: Vec::new(),
            mode: if input == "EVEN" {
                CondMode::Even
            } else {
                CondMode::Only
            },
        });
    }
    let inner = input
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| format!("line {line}: invalid COND"))?;
    let items = if inner.starts_with('(') {
        split_top(inner)?
    } else {
        vec![inner.to_string()]
    };
    let mut spec = CondSpec {
        tests: Vec::new(),
        mode: CondMode::Normal,
    };
    for item in items {
        if item == "EVEN" || item == "ONLY" {
            if spec.mode != CondMode::Normal {
                return Err(format!("line {line}: duplicate COND mode"));
            }
            spec.mode = if item == "EVEN" {
                CondMode::Even
            } else {
                CondMode::Only
            };
            continue;
        }
        let tuple = item.trim().trim_start_matches('(').trim_end_matches(')');
        let fields = split_top(tuple)?;
        if fields.len() < 2 || fields.len() > 3 {
            return Err(format!("line {line}: invalid COND test"));
        }
        let code = fields[0]
            .parse::<i32>()
            .map_err(|_| format!("line {line}: invalid COND code"))?;
        if !(0..=4095).contains(&code)
            || !matches!(fields[1].as_str(), "EQ" | "NE" | "LT" | "LE" | "GT" | "GE")
        {
            return Err(format!("line {line}: invalid COND comparison"));
        }
        spec.tests.push(CondTest {
            code,
            operator: fields[1].clone(),
            step: fields.get(2).map(|value| value.to_ascii_uppercase()),
        });
    }
    Ok(spec)
}

type ExecSpec = (String, bool, HashMap<String, String>, Option<CondSpec>);

fn parse_exec(operands: &str, line: usize) -> Result<ExecSpec> {
    let parts = split_top(operands)?;
    let first = parts
        .first()
        .ok_or_else(|| format!("line {line}: empty EXEC"))?;
    let (target, is_proc) = if let Some(pgm) = first.strip_prefix("PGM=") {
        (name(pgm, line)?, false)
    } else if let Some(proc_name) = first.strip_prefix("PROC=") {
        (name(proc_name, line)?, true)
    } else {
        (name(first, line)?, true)
    };
    let mut options = HashMap::new();
    let mut cond = None;
    for part in parts.iter().skip(1) {
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| format!("line {line}: invalid EXEC operand {part}"))?;
        let key = key.to_ascii_uppercase();
        if key == "COND" {
            cond = Some(parse_cond(value, line)?);
        } else if options.insert(key.clone(), value.to_string()).is_some() {
            return Err(format!("line {line}: duplicate EXEC operand {key}"));
        }
    }
    Ok((target, is_proc, options, cond))
}

fn merge_dd(base: &RawDd, override_dd: &RawDd) -> Result<RawDd> {
    if base.cards.len() != 1 || override_dd.cards.len() != 1 {
        return Ok(override_dd.clone());
    }
    let original = &base.cards[0];
    let changed = &override_dd.cards[0];
    if instream_delimiter(&changed.operands)?.is_some()
        || instream_delimiter(&original.operands)?.is_some()
        || changed.operands == "DUMMY"
        || original.operands == "DUMMY"
    {
        return Ok(override_dd.clone());
    }
    let mut parts = split_top(&original.operands)?;
    for value in split_top(&changed.operands)? {
        let key = value.split_once('=').map(|(key, _)| key).unwrap_or(&value);
        if let Some(index) = parts
            .iter()
            .position(|part| part.split_once('=').map(|(key, _)| key).unwrap_or(part) == key)
        {
            parts[index] = value;
        } else {
            parts.push(value);
        }
    }
    let mut result = base.clone();
    result.cards[0].operands = parts.join(",");
    Ok(result)
}

fn parse_dd(card: &Card, symbols: &HashMap<String, String>) -> Result<(DdKind, Option<String>)> {
    let operands = substitute(&card.operands, symbols, card.line)?;
    let parts = split_top(&operands)?;
    if parts
        .first()
        .is_some_and(|part| part == "*" || part == "DATA")
    {
        return Ok((DdKind::Inline(card.data.clone()), None));
    }
    if parts.first().is_some_and(|part| part == "DUMMY") {
        return Ok((DdKind::Dummy, None));
    }
    if parts.iter().any(|part| part.starts_with("SYSOUT=")) {
        return Ok((DdKind::Sysout, None));
    }
    let mut dsn = String::new();
    let mut disp = None;
    let mut dcb = None;
    for part in parts {
        let Some((key, value)) = part.split_once('=') else {
            return Err(format!("line {}: unsupported DD operand {part}", card.line));
        };
        match key {
            "DSN" | "DSNAME" => dsn = value.to_string(),
            "DISP" => disp = Some(value.to_string()),
            "DCB" => dcb = Some(value.to_string()),
            "UNIT" | "SPACE" | "VOL" | "LABEL" | "AMP" | "STORCLAS" | "DATACLAS" | "MGMTCLAS" => {}
            _ => return Err(format!("line {}: unsupported DD operand {key}", card.line)),
        }
    }
    let disp = disp.unwrap_or_else(|| {
        if dsn.is_empty() {
            "(NEW,DELETE)".to_string()
        } else {
            "SHR".to_string()
        }
    });
    Ok((DdKind::Dataset(vec![(dsn, disp)]), dcb))
}

fn expand_dd(raw: &RawDd, symbols: &HashMap<String, String>) -> Result<Dd> {
    let first = raw.cards.first().unwrap();
    let (mut kind, dcb) = parse_dd(first, symbols)?;
    for card in raw.cards.iter().skip(1) {
        let (next, next_dcb) = parse_dd(card, symbols)?;
        if next_dcb != dcb && next_dcb.is_some() {
            return Err(format!(
                "line {}: concatenated DD has conflicting DCB",
                card.line
            ));
        }
        match (&mut kind, next) {
            (DdKind::Dataset(entries), DdKind::Dataset(mut more)) => entries.append(&mut more),
            _ => return Err(format!("line {}: unsupported DD concatenation", card.line)),
        }
    }
    Ok(Dd {
        name: raw.name.clone(),
        line: first.line,
        kind,
        dcb,
    })
}

fn unique_step_name(name: &str, seen: &mut HashMap<String, usize>) -> String {
    let count = seen.entry(name.to_string()).or_default();
    *count += 1;
    if *count == 1 {
        name.to_string()
    } else {
        format!("{name}#{count}")
    }
}

fn matches_step(actual: &str, requested: &str) -> bool {
    actual == requested
        || actual
            .split_once('#')
            .is_some_and(|(base, _)| base == requested)
}

#[cfg(test)]
pub(super) fn parse_jcl(source: &str) -> Result<Job> {
    parse_jcl_with_catalog(source, &Catalog::new())
}

pub(super) fn parse_jcl_with_catalog(source: &str, catalog: &Catalog) -> Result<Job> {
    let mut order = Vec::new();
    let expanded_cards = expand_includes(cards(source)?, catalog, &mut order, &mut Vec::new())?;
    let (name, attributes, job_condition, raw_joblib, raw_steps, mut procedures) =
        raw_job(expanded_cards)?;
    let joblib = raw_joblib
        .iter()
        .map(|dd| expand_dd(dd, &HashMap::new()))
        .collect::<Result<Vec<_>>>()?;
    let mut steps = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut pending: VecDeque<RawStep> = raw_steps.into();
    while let Some(raw) = pending.pop_front() {
        let call_exec = substitute(&raw.operands, &raw.symbols, raw.line)?;
        let (target, is_proc, options, mut call_cond) = parse_exec(&call_exec, raw.line)?;
        if let Some(cond) = &mut call_cond {
            if let Some((prefix, _)) = raw.name.rsplit_once('.') {
                for test in &mut cond.tests {
                    if let Some(reference) = &test.step {
                        if raw.local_steps.contains(reference) {
                            test.step = Some(format!("{prefix}.{reference}"));
                        }
                    }
                }
            }
        }
        if !is_proc {
            let pgm = target;
            let dds = raw
                .dds
                .iter()
                .map(|dd| expand_dd(dd, &raw.symbols))
                .collect::<Result<Vec<_>>>()?;
            let step_name = unique_step_name(&raw.name, &mut seen);
            if !options.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "PARM" | "REGION" | "TIME" | "DYNAMNBR" | "MEMLIMIT"
                )
            }) {
                return Err(format!("line {}: unsupported EXEC option", raw.line));
            }
            let parm = options
                .get("PARM")
                .map(|value| value.trim_matches('\'').to_string());
            let mut conditions = raw.inherited_conditions;
            conditions.extend(call_cond);
            steps.push(Step {
                name: step_name,
                line: raw.line,
                pgm,
                parm,
                dds,
                conditions,
                guards: raw.guards,
            });
            continue;
        }
        if raw.call_stack.contains(&target) || raw.call_stack.len() >= 15 {
            return Err(format!(
                "line {}: recursive or too deeply nested PROC {target}",
                raw.line
            ));
        }
        if !procedures.contains_key(&target) {
            let source = catalog_member(catalog, &order, &target, raw.line)?;
            procedures.insert(
                target.clone(),
                catalog_procedure(source, &target, catalog, &order)?,
            );
        }
        let procedure = procedures
            .get(&target)
            .ok_or_else(|| format!("line {}: unresolved PROC {target}", raw.line))?;
        let mut symbols = raw.symbols.clone();
        symbols.extend(procedure.defaults.clone());
        for (key, value) in &options {
            symbols.insert(key.clone(), value.clone());
        }
        let proc_names: HashSet<_> = procedure
            .steps
            .iter()
            .map(|step| step.name.clone())
            .collect();
        for override_dd in &raw.dds {
            let Some((step_name, _)) = override_dd.name.split_once('.') else {
                return Err(format!(
                    "line {}: PROC DD override requires step.ddname",
                    override_dd.cards[0].line
                ));
            };
            if !proc_names.contains(step_name) {
                return Err(format!(
                    "line {}: PROC DD override references unknown step {step_name}",
                    override_dd.cards[0].line
                ));
            }
        }
        let mut children = Vec::new();
        for proc_step in &procedure.steps {
            let expanded_name = format!("{}.{}", raw.name, proc_step.name);
            let exec = substitute(&proc_step.operands, &symbols, proc_step.line)?;
            let mut dds = proc_step.dds.clone();
            for override_dd in &raw.dds {
                let Some((step_name, dd_name)) = override_dd.name.split_once('.') else {
                    return Err(format!(
                        "line {}: PROC DD override requires step.ddname",
                        override_dd.cards[0].line
                    ));
                };
                if step_name != proc_step.name {
                    continue;
                }
                let mut adjusted = override_dd.clone();
                adjusted.name = dd_name.to_string();
                if let Some(index) = dds.iter().position(|dd| dd.name == dd_name) {
                    dds[index] = merge_dd(&dds[index], &adjusted)?;
                } else {
                    dds.push(adjusted);
                }
            }
            let mut conditions = raw.inherited_conditions.clone();
            conditions.extend(call_cond.clone());
            let mut guards = raw.guards.clone();
            guards.extend(proc_step.guards.clone());
            let mut call_stack = raw.call_stack.clone();
            call_stack.push(target.clone());
            let mut child_symbols = symbols.clone();
            child_symbols.extend(proc_step.symbols.clone());
            children.push(RawStep {
                name: expanded_name,
                line: proc_step.line,
                operands: exec,
                dds,
                guards,
                symbols: child_symbols,
                inherited_conditions: conditions,
                call_stack,
                local_steps: proc_names.clone(),
            });
        }
        for child in children.into_iter().rev() {
            pending.push_front(child);
        }
    }
    Ok(Job {
        name,
        attributes,
        steps,
        job_condition,
        joblib,
    })
}

pub(super) fn cond_bypasses(
    spec: &CondSpec,
    previous: &[(String, Option<i32>)],
    abended: bool,
) -> Result<bool> {
    if spec.mode == CondMode::Only && !abended {
        return Ok(true);
    }
    if spec.mode == CondMode::Normal && abended {
        return Ok(true);
    }
    for test in &spec.tests {
        let candidates: Vec<_> = if let Some(target) = &test.step {
            let matched: Vec<_> = previous
                .iter()
                .filter(|(name, _)| matches_step(name, target))
                .collect();
            if matched.len() > 1 {
                return Err(format!("ambiguous COND step {target}"));
            }
            if matched.is_empty() {
                return Err(format!("COND references unknown previous step {target}"));
            }
            matched
        } else {
            previous.iter().collect()
        };
        for (_, rc) in candidates {
            let Some(rc) = rc else { continue };
            let matched = match test.operator.as_str() {
                "EQ" => test.code == *rc,
                "NE" => test.code != *rc,
                "LT" => test.code < *rc,
                "LE" => test.code <= *rc,
                "GT" => test.code > *rc,
                "GE" => test.code >= *rc,
                _ => return Err(format!("invalid COND operator {}", test.operator)),
            };
            if matched {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(super) fn guards_allow(
    guards: &[IfGuard],
    previous: &[(String, Option<i32>)],
    abends: &HashMap<String, String>,
) -> Result<bool> {
    for guard in guards {
        let value = evaluate_if(&guard.expression, previous, abends)?;
        if value != guard.then_branch {
            return Ok(false);
        }
    }
    Ok(true)
}

fn prior_step<'a>(
    step: &str,
    previous: &'a [(String, Option<i32>)],
) -> Result<&'a (String, Option<i32>)> {
    let mut matches = previous.iter().filter(|(name, _)| matches_step(name, step));
    let first = matches
        .next()
        .ok_or_else(|| format!("IF references unknown step {step}"))?;
    if matches.next().is_some() {
        return Err(format!("IF references ambiguous step {step}"));
    }
    Ok(first)
}

fn evaluate_if(
    expression: &str,
    previous: &[(String, Option<i32>)],
    abends: &HashMap<String, String>,
) -> Result<bool> {
    let mut expression = expression.trim();
    while expression.starts_with('(') && expression.ends_with(')') {
        let mut depth = 0;
        let mut encloses_all = true;
        for (index, ch) in expression.char_indices() {
            if ch == '(' {
                depth += 1;
            }
            if ch == ')' {
                depth -= 1;
            }
            if depth == 0 && index + ch.len_utf8() < expression.len() {
                encloses_all = false;
                break;
            }
        }
        if !encloses_all {
            break;
        }
        expression = expression[1..expression.len() - 1].trim();
    }
    for operators in [[" OR ", " | "], [" AND ", " & "]] {
        let mut depth = 0;
        for (index, ch) in expression.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                if let Some(operator) = operators
                    .iter()
                    .find(|operator| expression[index..].starts_with(**operator))
                {
                    let left = evaluate_if(&expression[..index], previous, abends)?;
                    if *operator == " OR " || *operator == " | " {
                        return if left {
                            Ok(true)
                        } else {
                            evaluate_if(&expression[index + operator.len()..], previous, abends)
                        };
                    }
                    return if !left {
                        Ok(false)
                    } else {
                        evaluate_if(&expression[index + operator.len()..], previous, abends)
                    };
                }
            }
        }
    }
    if let Some(rest) = expression.strip_prefix("NOT ") {
        return evaluate_if(rest, previous, abends).map(|value| !value);
    }
    if let Some(rest) = expression.strip_prefix('¬') {
        return evaluate_if(rest, previous, abends).map(|value| !value);
    }
    let upper = expression.to_ascii_uppercase();
    if upper == "ABEND" {
        return Ok(!abends.is_empty());
    }
    if let Some(code) = upper.strip_prefix("ABENDCC=") {
        return Ok(previous
            .iter()
            .rev()
            .find_map(|(name, _)| abends.get(name))
            .is_some_and(|actual| actual == code));
    }
    if let Some((step, code)) = upper.split_once(".ABENDCC=") {
        return Ok(abends
            .get(&prior_step(step, previous)?.0)
            .is_some_and(|actual| actual == code));
    }
    if let Some(step) = upper.strip_suffix(".ABEND") {
        return Ok(abends.contains_key(&prior_step(step, previous)?.0));
    }
    if let Some(step) = upper.strip_suffix(".RUN") {
        let prior = prior_step(step, previous)?;
        return Ok(prior.1.is_some() || abends.contains_key(&prior.0));
    }
    let fields: Vec<_> = expression.split_whitespace().collect();
    if fields.len() != 3 {
        return Err(format!("unsupported IF expression {expression:?}"));
    }
    let left = fields[0].to_ascii_uppercase();
    if left == "ABENDCC" || left.ends_with(".ABENDCC") {
        let actual = if left == "ABENDCC" {
            previous.iter().rev().find_map(|(name, _)| abends.get(name))
        } else {
            abends.get(&prior_step(left.trim_end_matches(".ABENDCC"), previous)?.0)
        };
        let desired = fields[2].to_ascii_uppercase();
        return match fields[1] {
            "=" | "EQ" => Ok(actual.is_some_and(|code| code == &desired)),
            "<>" | "NE" => Ok(actual.is_some_and(|code| code != &desired)),
            _ => Err(format!("unsupported IF operator {}", fields[1])),
        };
    }
    if left == "ABEND" || left.ends_with(".ABEND") || left.ends_with(".RUN") {
        let actual = if left == "ABEND" {
            !abends.is_empty()
        } else if let Some(step) = left.strip_suffix(".ABEND") {
            abends.contains_key(&prior_step(step, previous)?.0)
        } else if let Some(step) = left.strip_suffix(".RUN") {
            let prior = prior_step(step, previous)?;
            prior.1.is_some() || abends.contains_key(&prior.0)
        } else {
            false
        };
        let desired = match fields[2].to_ascii_uppercase().as_str() {
            "TRUE" => true,
            "FALSE" => false,
            _ => return Err(format!("invalid IF boolean {expression:?}")),
        };
        return match fields[1] {
            "=" | "EQ" => Ok(actual == desired),
            "<>" | "NE" => Ok(actual != desired),
            _ => Err(format!("unsupported IF operator {}", fields[1])),
        };
    }
    let right = fields[2]
        .parse::<i32>()
        .map_err(|_| format!("invalid IF RC {expression:?}"))?;
    let value = if left == "RC" || left == "MAXCC" {
        previous.iter().filter_map(|(_, rc)| *rc).max().unwrap_or(0)
    } else if let Some(step) = left.strip_suffix(".RC") {
        let prior = prior_step(step, previous)?;
        let Some(rc) = prior.1 else {
            return Ok(false);
        };
        rc
    } else {
        return Err(format!("unsupported IF variable {left}"));
    };
    match fields[1] {
        "=" | "EQ" => Ok(value == right),
        "<>" | "^=" | "NE" => Ok(value != right),
        ">" | "GT" => Ok(value > right),
        ">=" | "GE" => Ok(value >= right),
        "<" | "LT" => Ok(value < right),
        "<=" | "LE" => Ok(value <= right),
        other => Err(format!("unsupported IF operator {other}")),
    }
}

#[cfg(test)]
mod sequence_columns_tests {
    use super::cards;

    #[test]
    fn continuation_ignores_sequence_columns() {
        let first = "//            DCB=(LRECL=100,RECFM=FB),";
        let second = "//            UNIT=3390";
        let source = format!(
            "//JOB JOB 'X'\n//STEP EXEC PGM=IEFBR14\n//OUT DD DSN=DATA,\n{first:<72}00640019\n{second:<72}00650019\n"
        );
        let parsed = cards(&source).unwrap();
        assert_eq!(
            parsed[2].operands,
            "DSN=DATA,DCB=(LRECL=100,RECFM=FB),UNIT=3390"
        );
    }
}
