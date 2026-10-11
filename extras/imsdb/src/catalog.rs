use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub start: usize,
    pub bytes: usize,
    pub field_type: String,
    pub datatype: Option<String>,
    pub sequence: bool,
    pub unique: bool,
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub name: String,
    pub parent: Option<String>,
    pub min_bytes: usize,
    pub max_bytes: usize,
    pub fields: Vec<Field>,
}

impl Segment {
    pub fn key(&self) -> Option<&Field> {
        self.fields.iter().find(|f| f.sequence)
    }

    pub fn extract_key<'a>(&self, data: &'a [u8]) -> Result<&'a [u8], String> {
        let field = self
            .key()
            .ok_or_else(|| format!("{} has no sequence field", self.name))?;
        data.get(field.start - 1..field.start - 1 + field.bytes)
            .ok_or_else(|| format!("{} sequence field exceeds record", self.name))
    }

    pub fn validate_data(&self, data: &[u8]) -> Result<(), String> {
        if !(self.min_bytes..=self.max_bytes).contains(&data.len()) {
            return Err(format!(
                "{} requires {}..={} bytes; got {}",
                self.name,
                self.min_bytes,
                self.max_bytes,
                data.len()
            ));
        }
        self.extract_key(data)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Dbd {
    pub name: String,
    pub access: String,
    pub encoding: Option<String>,
    pub segments: Vec<Segment>,
    pub logical_children: Vec<LogicalChild>,
    pub indexed_fields: Vec<IndexedField>,
    pub gsam: Option<GsamDefinition>,
}

#[derive(Clone, Debug)]
pub struct LogicalChild {
    pub source_segment: String,
    pub target_segment: String,
    pub target_dbd: String,
    pub index_field: Option<String>,
}

#[derive(Clone, Debug)]
pub struct IndexedField {
    pub name: String,
    pub target_segment: String,
    pub source_segment: String,
    pub search_fields: Vec<String>,
    pub subsequence_fields: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct GsamDefinition {
    pub input_dd: String,
    pub output_dd: String,
    pub record_len: usize,
}

impl Dbd {
    pub fn segment(&self, name: &str) -> Option<&Segment> {
        self.segments.iter().find(|s| s.name == name)
    }
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let source = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        parse_dbd(&source).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[derive(Clone, Debug)]
pub struct Pcb {
    pub name: Option<String>,
    pub kind: String,
    pub dbd_name: Option<String>,
    pub procopt: Option<String>,
    pub procseq: Option<String>,
    pub keylen: Option<usize>,
    pub sensitive_segments: Vec<String>,
    pub indices: Vec<(String, Vec<String>)>,
}

#[derive(Clone, Debug)]
pub struct Psb {
    pub name: String,
    pub pcbs: Vec<Pcb>,
}

impl Psb {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let source = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        parse_psb(&source).map_err(|e| format!("{}: {e}", path.display()))
    }
    pub fn validate(&self, dbds: &[Dbd]) -> Result<(), String> {
        for pcb in &self.pcbs {
            if pcb.kind == "GSAM" {
                let name = pcb.dbd_name.as_deref().ok_or("GSAM PCB has no DBDNAME")?;
                if !dbds
                    .iter()
                    .any(|dbd| dbd.name == name && dbd.gsam.is_some())
                {
                    return Err(format!("{}: missing GSAM DBD {name}", self.name));
                }
                continue;
            }
            if pcb.kind != "DB" {
                continue;
            }
            let name = pcb.dbd_name.as_deref().ok_or("DB PCB has no DBDNAME")?;
            let dbd = dbds
                .iter()
                .find(|d| d.name == name)
                .ok_or_else(|| format!("{}: missing DBD {name}", self.name))?;
            for segment in &pcb.sensitive_segments {
                if dbd.segment(segment).is_none() {
                    return Err(format!("{}: {segment} is not in DBD {name}", self.name));
                }
            }
            for (segment, indexes) in &pcb.indices {
                if dbd.segment(segment).is_none()
                    || !pcb.sensitive_segments.contains(segment)
                    || indexes.is_empty()
                {
                    return Err(format!(
                        "{}: invalid SENSEG INDICES for {segment}",
                        self.name
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Statement {
    line: usize,
    op: String,
    args: String,
    label: Option<String>,
}

fn statements(source: &str) -> Result<Vec<Statement>, String> {
    const OPS: &[&str] = &[
        "DBD", "DATASET", "SEGM", "FIELD", "LCHILD", "XDFLD", "DBDGEN", "FINISH", "END", "PCB",
        "SENSEG", "PSBGEN", "TITLE", "PRINT", "EJECT", "SPACE", "DFSMARSH",
    ];
    let mut result: Vec<Statement> = Vec::new();
    for (index, full_line) in source.lines().enumerate() {
        if full_line.trim().is_empty() || full_line.starts_with('*') {
            continue;
        }
        // Column 72 is an assembly continuation indicator, not a parameter.
        let part: String = full_line.chars().take(71).collect();
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        let first = words[0].to_ascii_uppercase();
        let second = words.get(1).map(|w| w.to_ascii_uppercase());
        let (op, label, arg_start) = if OPS.contains(&first.as_str()) {
            (first, None, words[0].len())
        } else if let Some(second) = second.filter(|s| OPS.contains(&s.as_str())) {
            let pos = words[0].len() + trimmed[words[0].len()..].find(words[1]).unwrap();
            (second, Some(first), pos + words[1].len())
        } else {
            if full_line.chars().take_while(|c| c.is_whitespace()).count() < 15 {
                return Err(format!(
                    "line {}: unsupported macro {}",
                    index + 1,
                    words[0]
                ));
            }
            let previous = result
                .last_mut()
                .ok_or_else(|| format!("line {}: orphan continuation", index + 1))?;
            previous.args.push_str(trimmed);
            continue;
        };
        result.push(Statement {
            line: index + 1,
            op,
            args: trimmed[arg_start..].trim().to_string(),
            label,
        });
    }
    Ok(result)
}

fn params(args: &str) -> Vec<(String, String)> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let mut quote = false;
    for (i, c) in args.char_indices() {
        match c {
            '\'' => quote = !quote,
            '(' if !quote => depth += 1,
            ')' if !quote => depth -= 1,
            ',' if !quote && depth == 0 => {
                parts.push(&args[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&args[start..]);
    parts
        .into_iter()
        .filter_map(|p| p.trim().split_once('='))
        .map(|(k, v)| (k.trim().to_ascii_uppercase(), v.trim().to_string()))
        .collect()
}

fn value<'a>(p: &'a [(String, String)], key: &str) -> Option<&'a str> {
    p.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}
fn required<'a>(p: &'a [(String, String)], key: &str, line: usize) -> Result<&'a str, String> {
    value(p, key)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("line {line}: missing {key}"))
}
fn number(p: &[(String, String)], key: &str, line: usize) -> Result<usize, String> {
    required(p, key, line)?
        .trim_matches(|c| c == '(' || c == ')')
        .split(',')
        .next()
        .unwrap()
        .parse()
        .map_err(|_| format!("line {line}: invalid {key}"))
}
fn tuple_head(s: &str) -> &str {
    s.trim_start_matches('(')
        .split([',', ')'])
        .next()
        .unwrap_or(s)
        .trim()
}
fn field_names(raw: &str, line: usize) -> Result<Vec<String>, String> {
    let names: Vec<_> = raw
        .trim_matches(|c| c == '(' || c == ')')
        .split(',')
        .map(|name| name.trim().to_ascii_uppercase())
        .collect();
    if names.is_empty() || names.len() > 5 || names.iter().any(String::is_empty) {
        return Err(format!("line {line}: invalid XDFLD field list"));
    }
    Ok(names)
}
fn parent_name(s: &str) -> Option<String> {
    let name = tuple_head(s);
    if name == "0" || name.is_empty() {
        None
    } else {
        Some(name.to_ascii_uppercase())
    }
}

pub fn parse_dbd(source: &str) -> Result<Dbd, String> {
    let mut dbd: Option<Dbd> = None;
    for stmt in statements(source)? {
        let p = params(&stmt.args);
        match stmt.op.as_str() {
            "DBD" => {
                if dbd.is_some() {
                    return Err(format!("line {}: duplicate DBD", stmt.line));
                }
                let access = required(&p, "ACCESS", stmt.line)?
                    .trim_start_matches('(')
                    .split(',')
                    .next()
                    .unwrap()
                    .to_ascii_uppercase();
                dbd = Some(Dbd {
                    name: required(&p, "NAME", stmt.line)?.to_ascii_uppercase(),
                    access,
                    encoding: value(&p, "ENCODING").map(str::to_string),
                    segments: Vec::new(),
                    logical_children: Vec::new(),
                    indexed_fields: Vec::new(),
                    gsam: None,
                });
            }
            "DATASET" => {
                let dbd = dbd
                    .as_mut()
                    .ok_or_else(|| format!("line {}: DATASET before DBD", stmt.line))?;
                if dbd.access == "GSAM" {
                    let record = required(&p, "RECORD", stmt.line)?;
                    let record_len = tuple_head(record)
                        .parse::<usize>()
                        .map_err(|_| format!("line {}: invalid GSAM RECORD", stmt.line))?;
                    if record_len == 0 || value(&p, "RECFM") != Some("F") {
                        return Err(format!(
                            "line {}: only fixed GSAM records are supported",
                            stmt.line
                        ));
                    }
                    dbd.gsam = Some(GsamDefinition {
                        input_dd: required(&p, "DD1", stmt.line)?.to_ascii_uppercase(),
                        output_dd: required(&p, "DD2", stmt.line)?.to_ascii_uppercase(),
                        record_len,
                    });
                }
            }
            "SEGM" => {
                let dbd = dbd
                    .as_mut()
                    .ok_or_else(|| format!("line {}: SEGM before DBD", stmt.line))?;
                let bytes = required(&p, "BYTES", stmt.line)?;
                let sizes: Vec<usize> = bytes
                    .trim_matches(|c| c == '(' || c == ')')
                    .split(',')
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| {
                        s.trim()
                            .parse::<usize>()
                            .map_err(|_| format!("line {}: invalid BYTES", stmt.line))
                    })
                    .collect::<Result<_, _>>()?;
                let min_bytes = *sizes
                    .first()
                    .ok_or_else(|| format!("line {}: empty BYTES", stmt.line))?;
                let max_bytes = *sizes.get(1).unwrap_or(&min_bytes);
                if min_bytes == 0 || max_bytes < min_bytes {
                    return Err(format!("line {}: invalid segment length", stmt.line));
                }
                dbd.segments.push(Segment {
                    name: required(&p, "NAME", stmt.line)?.to_ascii_uppercase(),
                    parent: parent_name(required(&p, "PARENT", stmt.line)?),
                    min_bytes,
                    max_bytes,
                    fields: Vec::new(),
                });
            }
            "FIELD" => {
                let segment = dbd
                    .as_mut()
                    .and_then(|d| d.segments.last_mut())
                    .ok_or_else(|| format!("line {}: FIELD before SEGM", stmt.line))?;
                let raw = required(&p, "NAME", stmt.line)?;
                let flags: Vec<_> = raw
                    .trim_matches(|c| c == '(' || c == ')')
                    .split(',')
                    .map(str::trim)
                    .collect();
                let start = number(&p, "START", stmt.line)?;
                let bytes = number(&p, "BYTES", stmt.line)?;
                if start == 0 || bytes == 0 || start - 1 + bytes > segment.max_bytes {
                    return Err(format!(
                        "line {}: FIELD outside {}",
                        stmt.line, segment.name
                    ));
                }
                segment.fields.push(Field {
                    name: flags[0].to_ascii_uppercase(),
                    start,
                    bytes,
                    field_type: value(&p, "TYPE").unwrap_or("C").to_ascii_uppercase(),
                    datatype: value(&p, "DATATYPE").map(|s| s.to_ascii_uppercase()),
                    sequence: flags.contains(&"SEQ"),
                    unique: flags.contains(&"U"),
                });
            }
            "LCHILD" => {
                let dbd = dbd
                    .as_mut()
                    .ok_or_else(|| format!("line {}: LCHILD before DBD", stmt.line))?;
                let source_segment = dbd
                    .segments
                    .last()
                    .ok_or_else(|| format!("line {}: LCHILD before SEGM", stmt.line))?
                    .name
                    .clone();
                let raw = required(&p, "NAME", stmt.line)?;
                let names: Vec<_> = raw
                    .trim_matches(|c| c == '(' || c == ')')
                    .split(',')
                    .map(str::trim)
                    .collect();
                if names.len() != 2 {
                    return Err(format!("line {}: invalid LCHILD", stmt.line));
                }
                dbd.logical_children.push(LogicalChild {
                    source_segment,
                    target_segment: names[0].to_ascii_uppercase(),
                    target_dbd: names[1].to_ascii_uppercase(),
                    index_field: value(&p, "INDEX").map(str::to_ascii_uppercase),
                });
            }
            "XDFLD" => {
                let dbd = dbd
                    .as_mut()
                    .ok_or_else(|| format!("line {}: XDFLD before DBD", stmt.line))?;
                let target_segment = dbd
                    .segments
                    .last()
                    .ok_or_else(|| format!("line {}: XDFLD before SEGM", stmt.line))?
                    .name
                    .clone();
                if p.iter()
                    .any(|(key, _)| !matches!(key.as_str(), "NAME" | "SEGMENT" | "SRCH" | "SUBSEQ"))
                {
                    return Err(format!("line {}: unsupported XDFLD operand", stmt.line));
                }
                dbd.indexed_fields.push(IndexedField {
                    name: required(&p, "NAME", stmt.line)?.to_ascii_uppercase(),
                    source_segment: value(&p, "SEGMENT")
                        .unwrap_or(&target_segment)
                        .to_ascii_uppercase(),
                    target_segment,
                    search_fields: field_names(required(&p, "SRCH", stmt.line)?, stmt.line)?,
                    subsequence_fields: value(&p, "SUBSEQ")
                        .map(|raw| field_names(raw, stmt.line))
                        .transpose()?
                        .unwrap_or_default(),
                });
            }
            "DBDGEN" | "FINISH" | "END" | "TITLE" | "PRINT" | "EJECT" | "SPACE" | "DFSMARSH" => {}
            other => return Err(format!("line {}: {other} is invalid in DBD", stmt.line)),
        }
    }
    let dbd = dbd.ok_or("missing DBD")?;
    let mut names = HashSet::new();
    for segment in &dbd.segments {
        if !names.insert(&segment.name) {
            return Err(format!("duplicate segment {}", segment.name));
        }
        if let Some(parent) = &segment.parent {
            if parent == &segment.name || !names.contains(parent) {
                return Err(format!(
                    "{}: parent {parent} must precede child",
                    segment.name
                ));
            }
        }
    }
    Ok(dbd)
}

pub fn parse_psb(source: &str) -> Result<Psb, String> {
    let mut psb = Psb {
        name: String::new(),
        pcbs: Vec::new(),
    };
    for stmt in statements(source)? {
        let p = params(&stmt.args);
        match stmt.op.as_str() {
            "PCB" => psb.pcbs.push(Pcb {
                name: value(&p, "PCBNAME")
                    .or(stmt.label.as_deref())
                    .map(str::to_string),
                kind: required(&p, "TYPE", stmt.line)?.to_ascii_uppercase(),
                dbd_name: value(&p, "DBDNAME").map(|s| s.to_ascii_uppercase()),
                procopt: value(&p, "PROCOPT").map(str::to_string),
                procseq: value(&p, "PROCSEQ").map(|s| s.to_ascii_uppercase()),
                keylen: value(&p, "KEYLEN")
                    .map(|s| {
                        s.parse::<usize>()
                            .map_err(|_| format!("line {}: invalid KEYLEN", stmt.line))
                    })
                    .transpose()?,
                sensitive_segments: Vec::new(),
                indices: Vec::new(),
            }),
            "SENSEG" => {
                let segment = required(&p, "NAME", stmt.line)?.to_ascii_uppercase();
                let pcb = psb
                    .pcbs
                    .last_mut()
                    .ok_or_else(|| format!("line {}: SENSEG before PCB", stmt.line))?;
                pcb.sensitive_segments.push(segment.clone());
                if let Some(raw) = value(&p, "INDICES") {
                    let indexes: Vec<String> = raw
                        .trim_matches(|ch| ch == '(' || ch == ')')
                        .split(',')
                        .map(|name| name.trim().to_ascii_uppercase())
                        .collect();
                    if indexes.is_empty()
                        || indexes.len() > 32
                        || indexes.iter().any(String::is_empty)
                    {
                        return Err(format!("line {}: invalid SENSEG INDICES", stmt.line));
                    }
                    pcb.indices.push((segment, indexes));
                }
            }
            "PSBGEN" => psb.name = required(&p, "PSBNAME", stmt.line)?.to_ascii_uppercase(),
            "END" | "TITLE" | "PRINT" | "EJECT" | "SPACE" => {}
            other => return Err(format!("line {}: {other} is invalid in PSB", stmt.line)),
        }
    }
    if psb.name.is_empty() {
        return Err("missing PSBGEN".into());
    }
    Ok(psb)
}
