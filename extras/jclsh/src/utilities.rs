//! Small, explicit z/OS utility subsets used by the checked-in JCL.
//! Unsupported control statements return RC=12 instead of succeeding silently.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::datasets::{Allocation, DataSets};
use super::records::{self, Layout};
use super::vsam_sql;
use super::Result;

pub(super) struct Output {
    pub rc: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub(super) fn is_builtin(program: &str) -> bool {
    matches!(
        program,
        "IEFBR14" | "IEBGENER" | "IDCAMS" | "SORT" | "DFHCSDUP"
    )
}

pub(super) fn run(
    program: &str,
    dds: &HashMap<String, PathBuf>,
    allocations: &[Allocation],
    datasets: &DataSets<'_>,
) -> Result<Output> {
    let result = match program {
        "IEFBR14" => Ok((0, "IEFBR14: DD dispositions processed\n".to_string())),
        "IEBGENER" => iebgener(dds),
        "IDCAMS" => idcams(dds, allocations, datasets),
        "SORT" => sort(dds),
        "DFHCSDUP" => dfhcsdup(dds),
        _ => return Err(format!("unknown built-in utility {program}")),
    };
    let (rc, text) = match result {
        Ok(value) => value,
        Err(message) => (12, format!("{program}: {message}\n")),
    };
    Ok(Output {
        rc,
        stdout: text.into_bytes(),
        stderr: Vec::new(),
    })
}

fn dd<'a>(dds: &'a HashMap<String, PathBuf>, name: &str) -> std::result::Result<&'a Path, String> {
    dds.get(name)
        .map(PathBuf::as_path)
        .ok_or_else(|| format!("required DD {name} is missing"))
}

fn cards(dds: &HashMap<String, PathBuf>, name: &str) -> std::result::Result<Vec<String>, String> {
    let Some(path) = dds.get(name) else {
        return Ok(Vec::new());
    };
    if path == Path::new("/dev/null") {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).map_err(|error| format!("{name}: {error}"))?;
    let mut result = Vec::new();
    let mut pending = String::new();
    let mut continued = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('*') {
            continue;
        }
        let first = line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        let new_command = matches!(
            first.as_str(),
            "DELETE"
                | "DEFINE"
                | "REPRO"
                | "SET"
                | "IF"
                | "LISTCAT"
                | "BLDINDEX"
                | "SORT"
                | "INCLUDE"
                | "OUTREC"
                | "OPTION"
                | "ADD"
                | "REMOVE"
                | "LIST"
        );
        if new_command && !pending.is_empty() && !continued {
            result.push(std::mem::take(&mut pending));
        }
        if !pending.is_empty() {
            pending.push(' ');
        }
        pending.push_str(line.trim_end_matches('-').trim_end());
        continued = line.ends_with('-');
    }
    if continued {
        return Err(format!("{name}: unterminated continuation"));
    }
    if !pending.is_empty() {
        result.push(pending);
    }
    Ok(result)
}

fn iebgener(dds: &HashMap<String, PathBuf>) -> std::result::Result<(i32, String), String> {
    let control = cards(dds, "SYSIN")?;
    if !control.is_empty() {
        return Err(format!("unsupported SYSIN control: {}", control.join(" ")));
    }
    let input = dd(dds, "SYSUT1")?;
    let output = dd(dds, "SYSUT2")?;
    let mut bytes = if input == Path::new("/dev/null") {
        Vec::new()
    } else {
        fs::read(input).map_err(|error| format!("SYSUT1: {error}"))?
    };
    if output != Path::new("/dev/null") {
        let input_layout = records::read_layout(input)?;
        if let Some(layout) = records::read_layout(output)?.or(input_layout) {
            if input_layout.is_some_and(|source| source.lrecl != layout.lrecl) {
                return Err("SYSUT1 and SYSUT2 FB LRECL differ".to_string());
            }
            bytes = records::normalize_fb(&bytes, layout, input_layout.is_some())?;
            records::write_layout(output, layout)?;
        }
        fs::write(output, &bytes).map_err(|error| format!("SYSUT2: {error}"))?;
    }
    Ok((0, format!("IEBGENER: copied {} bytes\n", bytes.len())))
}

fn operand(text: &str, key: &str) -> Option<String> {
    let upper = text.to_ascii_uppercase();
    let direct = format!("{key}(");
    let assigned = format!("{key}=(");
    let start = upper
        .find(&direct)
        .map(|at| at + direct.len())
        .or_else(|| upper.find(&assigned).map(|at| at + assigned.len()))?;
    let mut depth = 1;
    let mut quoted = false;
    for (offset, byte) in text.as_bytes()[start..].iter().enumerate() {
        match byte {
            b'\'' => quoted = !quoted,
            b'(' if !quoted => depth += 1,
            b')' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[start..start + offset].trim().to_ascii_uppercase());
                }
            }
            _ => {}
        }
    }
    None
}

fn idcams(
    dds: &HashMap<String, PathBuf>,
    allocations: &[Allocation],
    datasets: &DataSets<'_>,
) -> std::result::Result<(i32, String), String> {
    let mut maxcc = 0;
    let mut lastcc = 0;
    let mut report = String::new();
    for card in cards(dds, "SYSIN")? {
        let upper = card.to_ascii_uppercase();
        let mut words = upper.split_whitespace();
        let verb = words.next().unwrap_or("");
        let result = match verb {
            "DELETE" => {
                let name = words.next().ok_or("DELETE requires a data set name")?;
                let path = datasets
                    .target(name, true)
                    .map_err(|error| error.to_string())?
                    .ok_or("DELETE requires a cataloged data set")?;
                if path.exists() {
                    if let Some(table) = datasets.mapped_sql_table(name) {
                        let (profile, workspace) = datasets.sql_context();
                        vsam_sql::drop_table(profile, workspace, table)?;
                    }
                    fs::remove_file(&path).map_err(|error| error.to_string())?;
                    records::remove_layout(&path)?;
                    0
                } else {
                    8
                }
            }
            "DEFINE" => {
                let kind = words.next().unwrap_or("").split('(').next().unwrap_or("");
                if !matches!(
                    kind,
                    "CLUSTER" | "ALTERNATEINDEX" | "PATH" | "GENERATIONDATAGROUP"
                ) {
                    return Err(format!("unsupported DEFINE type in {card}"));
                }
                let name = operand(&card, "NAME")
                    .ok_or_else(|| format!("DEFINE {kind} requires NAME(...): {card}"))?;
                let path = catalog_path(datasets, &name)?;
                if path.exists() {
                    if kind == "GENERATIONDATAGROUP" {
                        12
                    } else {
                        8
                    }
                } else {
                    let contents = match kind {
                        "ALTERNATEINDEX" => {
                            let base =
                                operand(&card, "RELATE").ok_or("ALTERNATEINDEX requires RELATE")?;
                            let key =
                                operand(&card, "KEYS").ok_or("ALTERNATEINDEX requires KEYS")?;
                            let parts: Vec<_> = key
                                .split([',', ' '])
                                .filter(|part| !part.is_empty())
                                .collect();
                            if parts.len() != 2
                                || parts.iter().any(|part| part.parse::<usize>().is_err())
                            {
                                return Err(format!("invalid ALTERNATEINDEX KEYS: {key}"));
                            }
                            format!(
                                "#JCLSH-AIX\t{base}\t{}\t{}\t{}\n",
                                parts[0],
                                parts[1],
                                if upper.contains("NONUNIQUEKEY") {
                                    "NONUNIQUE"
                                } else {
                                    "UNIQUE"
                                }
                            )
                        }
                        "PATH" => {
                            let index =
                                operand(&card, "PATHENTRY").ok_or("PATH requires PATHENTRY")?;
                            let index_path = catalog_path(datasets, &index)?;
                            if !index_path.exists() {
                                return Err(format!("PATHENTRY {index} is not defined"));
                            }
                            format!("#JCLSH-PATH\t{index}\n")
                        }
                        "GENERATIONDATAGROUP" => {
                            let limit = operand(&card, "LIMIT")
                                .ok_or("GENERATIONDATAGROUP requires LIMIT")?;
                            let count: usize = limit
                                .parse()
                                .map_err(|_| format!("invalid GDG LIMIT: {limit}"))?;
                            if count == 0 {
                                return Err("GDG LIMIT must be positive".to_string());
                            }
                            format!("#JCLSH-GDG\t{count}\n")
                        }
                        _ => String::new(),
                    };
                    let layout = if kind == "CLUSTER" {
                        operand(&card, "RECORDSIZE").or_else(|| operand(&card, "RECSZ"))
                            .map(|size| {
                                let record_len = numeric_pair(&size)?.1;
                                let (key_len, key_offset) = operand(&card, "KEYS")
                                    .map(|value| numeric_pair(&value)).transpose()?
                                    .unwrap_or((0, 0));
                                Ok::<Layout, String>(Layout { lrecl: record_len, key_len, key_offset })
                            }).transpose()?
                    } else { None };
                    let sql_table = if kind == "CLUSTER" { datasets.sql_table(&name)? } else { None };
                    if let Some(table) = sql_table {
                        let (profile, workspace) = datasets.sql_context();
                        vsam_sql::create(profile, workspace, table,
                            layout.ok_or("SQL VSAM requires RECORDSIZE and KEYS")?)?;
                    }
                    let local_result = (|| {
                        fs::write(&path, contents).map_err(|error| error.to_string())?;
                        if let Some(layout) = layout { records::write_layout(&path, layout)?; }
                        Ok::<_, String>(())
                    })();
                    if let Err(error) = local_result {
                        if let Some(table) = sql_table {
                            let (profile, workspace) = datasets.sql_context();
                            let _ = vsam_sql::drop_table(profile, workspace, table);
                        }
                        return Err(error);
                    }
                    0
                }
            }
            "BLDINDEX" => {
                let base = operand(&card, "INDATASET").ok_or("BLDINDEX requires INDATASET")?;
                let index = operand(&card, "OUTDATASET").ok_or("BLDINDEX requires OUTDATASET")?;
                let base_path = catalog_path(datasets, &base)?;
                let index_path = catalog_path(datasets, &index)?;
                let metadata =
                    fs::read_to_string(&index_path).map_err(|error| format!("{index}: {error}"))?;
                let header = metadata.lines().next().unwrap_or("");
                let parts: Vec<_> = header.split('\t').collect();
                if parts.len() != 5 || parts[0] != "#JCLSH-AIX" || parts[1] != base {
                    return Err(format!("{index} is not an ALTERNATEINDEX for {base}"));
                }
                let key_len: usize = parts[2]
                    .parse()
                    .map_err(|_| "invalid ALTERNATEINDEX key length")?;
                let key_offset: usize = parts[3]
                    .parse()
                    .map_err(|_| "invalid ALTERNATEINDEX key offset")?;
                let base_layout = records::read_layout(&base_path)?;
                let bytes = if let Some(table) = datasets.sql_table_for_path(&base_path) {
                    let (profile, workspace) = datasets.sql_context();
                    vsam_sql::export(profile, workspace, table,
                        base_layout.ok_or("SQL VSAM base lacks FB layout")?)?
                } else {
                    fs::read(&base_path).map_err(|error| format!("{base}: {error}"))?
                };
                let mut entries = Vec::new();
                for (ordinal, record) in
                    records::records(&bytes, base_layout)?
                        .into_iter()
                        .enumerate()
                {
                    let key = record
                        .get(key_offset..key_offset + key_len)
                        .ok_or_else(|| {
                            format!(
                                "{base}: record {} is shorter than ALTERNATEINDEX key",
                                ordinal + 1
                            )
                        })?;
                    let hex: String = key.iter().map(|byte| format!("{byte:02X}")).collect();
                    entries.push((hex, ordinal));
                }
                entries.sort();
                if parts[4] == "UNIQUE" && entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                    return Err(format!("{index}: duplicate UNIQUEKEY"));
                }
                let mut output = format!("{header}\n");
                for (key, ordinal) in entries {
                    output.push_str(&format!("{key}\t{ordinal}\n"));
                }
                fs::write(&index_path, output).map_err(|error| format!("{index}: {error}"))?;
                0
            }
            "REPRO" => {
                let from = operand(&card, "INFILE")
                    .or_else(|| operand(&card, "IFILE"))
                    .ok_or_else(|| format!("REPRO requires INFILE/IFILE: {card}"))?;
                let to = operand(&card, "OUTFILE")
                    .or_else(|| operand(&card, "OFILE"))
                    .ok_or_else(|| format!("REPRO requires OUTFILE/OFILE: {card}"))?;
                let input = dd(dds, &from)?;
                let output = dd(dds, &to)?;
                let input_layout = records::read_layout(input)?;
                let input_sql = datasets.sql_table_for_allocation(input, allocations);
                let output_sql = datasets.sql_table_for_allocation(output, allocations);
                let mut bytes = if let Some(table) = input_sql {
                    let (profile, workspace) = datasets.sql_context();
                    vsam_sql::export(profile, workspace, table,
                        input_layout.ok_or("SQL VSAM input lacks FB layout")?)?
                } else {
                    fs::read(input).map_err(|error| error.to_string())?
                };
                if let Some(layout) = records::read_layout(output)?.or(input_layout) {
                    if input_layout.is_some_and(|source| source.lrecl != layout.lrecl) {
                        return Err("REPRO input and output FB LRECL differ".to_string());
                    }
                    bytes = records::normalize_fb(&bytes, layout, input_layout.is_some())?;
                    if let Some(table) = output_sql {
                        let (profile, workspace) = datasets.sql_context();
                        vsam_sql::replace(profile, workspace, table, layout, &bytes)?;
                    } else {
                        records::write_layout(output, layout)?;
                        fs::write(output, &bytes).map_err(|error| error.to_string())?;
                    }
                } else if output_sql.is_some() {
                    return Err("SQL VSAM output lacks FB layout".to_string());
                } else {
                    fs::write(output, &bytes).map_err(|error| error.to_string())?;
                }
                0
            }
            "LISTCAT" => {
                let level = operand(&card, "LEVEL").unwrap_or_default();
                let mut names: Vec<_> = datasets.catalog_names(&level);
                names.sort();
                for name in names {
                    report.push_str(&format!("NONVSAM ------- {name}\n"));
                }
                0
            }
            "SET" => {
                if upper.replace(' ', "") != "SETMAXCC=0" {
                    return Err(format!("unsupported SET statement: {card}"));
                }
                maxcc = 0;
                report.push_str("IDCAMS: MAXCC set to 0\n");
                continue;
            }
            "IF" => {
                let compact = upper.replace(' ', "");
                let reset = match compact.as_str() {
                    "IFMAXCC=8THENSETMAXCC=0" => maxcc == 8,
                    "IFMAXCCLE08THENSETMAXCC=0" => maxcc <= 8,
                    "IFLASTCC=12THENSETMAXCC=0" => lastcc == 12,
                    _ => return Err(format!("unsupported IF statement: {card}")),
                };
                if !compact.ends_with("THENSETMAXCC=0") {
                    return Err(format!("unsupported IF statement: {card}"));
                }
                if reset {
                    maxcc = 0;
                }
                continue;
            }
            _ => return Err(format!("unsupported control statement: {card}")),
        };
        lastcc = result;
        maxcc = maxcc.max(result);
        report.push_str(&format!("IDCAMS: {verb} RC={result}\n"));
    }
    Ok((maxcc, report))
}

fn numeric_pair(value: &str) -> std::result::Result<(usize, usize), String> {
    let parts: Vec<_> = value
        .split([',', ' '])
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() != 2 {
        return Err(format!("expected numeric pair: {value}"));
    }
    let first = parts[0]
        .parse()
        .map_err(|_| format!("invalid number: {value}"))?;
    let second = parts[1]
        .parse()
        .map_err(|_| format!("invalid number: {value}"))?;
    Ok((first, second))
}

fn catalog_path(datasets: &DataSets<'_>, name: &str) -> std::result::Result<PathBuf, String> {
    datasets
        .target(name, true)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("{name} requires a cataloged data set"))
}

fn dfhcsdup(dds: &HashMap<String, PathBuf>) -> std::result::Result<(i32, String), String> {
    let catalog = dd(dds, "DFHCSD")?;
    if catalog == Path::new("/dev/null") {
        return Err("DFHCSD must name a writable CSD data set".to_string());
    }
    let existing = fs::read_to_string(catalog).map_err(|error| format!("DFHCSD: {error}"))?;
    let mut resources: HashMap<(String, String, String), String> = HashMap::new();
    let mut lists: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for line in existing.lines() {
        let parts: Vec<_> = line.splitn(5, '\t').collect();
        match parts.as_slice() {
            ["R", group, kind, name, description] => {
                resources.insert(
                    ((*group).into(), (*kind).into(), (*name).into()),
                    (*description).into(),
                );
            }
            ["L", list, group] => {
                lists.insert(((*list).into(), (*group).into()));
            }
            _ => return Err("DFHCSD is not a jclsh CSD catalog".to_string()),
        }
    }
    let mut report = String::new();
    let mut changed = 0;
    for card in cards(dds, "SYSIN")? {
        let upper = card.to_ascii_uppercase();
        let mut words = upper.split_whitespace();
        let verb = words.next().unwrap_or("");
        match verb {
            "DEFINE" => {
                let kind = words.next().unwrap_or("").split('(').next().unwrap_or("");
                if kind.is_empty() {
                    return Err(format!("DEFINE requires a resource type: {card}"));
                }
                let name = operand(&card, kind)
                    .ok_or_else(|| format!("DEFINE requires {kind}(...): {card}"))?;
                let group = operand(&card, "GROUP")
                    .ok_or_else(|| format!("DEFINE requires GROUP(...): {card}"))?;
                resources.insert((group, kind.to_string(), name), upper);
                changed += 1;
            }
            "ADD" | "REMOVE" => {
                let group = operand(&card, "GROUP")
                    .ok_or_else(|| format!("{verb} requires GROUP(...): {card}"))?;
                let list = operand(&card, "LIST")
                    .ok_or_else(|| format!("{verb} requires LIST(...): {card}"))?;
                if verb == "ADD" {
                    lists.insert((list, group));
                } else {
                    lists.remove(&(list, group));
                }
                changed += 1;
            }
            "DELETE" => {
                let group = operand(&card, "GROUP")
                    .ok_or_else(|| format!("DELETE requires GROUP(...): {card}"))?;
                if !upper.split_whitespace().any(|part| part == "ALL") {
                    return Err(format!("only DELETE GROUP(...) ALL is supported: {card}"));
                }
                resources.retain(|(entry_group, _, _), _| entry_group != &group);
                lists.retain(|(_, entry_group)| entry_group != &group);
                changed += 1;
            }
            "LIST" => {
                let group = operand(&card, "GROUP")
                    .ok_or_else(|| format!("LIST requires GROUP(...): {card}"))?;
                let mut entries: Vec<_> = resources
                    .keys()
                    .filter(|(entry_group, _, _)| entry_group == &group)
                    .collect();
                entries.sort();
                for (_, kind, name) in entries {
                    report.push_str(&format!("DFHCSDUP {group} {kind} {name}\n"));
                }
            }
            _ => return Err(format!("unsupported DFHCSDUP control statement: {card}")),
        }
    }
    let mut lines = Vec::new();
    for ((group, kind, name), description) in resources {
        lines.push(format!("R\t{group}\t{kind}\t{name}\t{description}"));
    }
    for (list, group) in lists {
        lines.push(format!("L\t{list}\t{group}"));
    }
    lines.sort();
    let content = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    fs::write(catalog, content).map_err(|error| format!("DFHCSD: {error}"))?;
    report.push_str(&format!("DFHCSDUP: {changed} catalog update(s)\n"));
    Ok((0, report))
}

#[derive(Clone, Copy)]
struct Field {
    pos: usize,
    len: usize,
}

type SortSymbols = (HashMap<String, Field>, HashMap<String, Vec<u8>>);

fn csv(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut quoted = false;
    for (index, byte) in text.bytes().enumerate() {
        match byte {
            b'\'' => quoted = !quoted,
            b'(' if !quoted => depth += 1,
            b')' if !quoted => depth -= 1,
            b',' if !quoted && depth == 0 => {
                result.push(text[start..index].trim().to_ascii_uppercase());
                start = index + 1;
            }
            _ => {}
        }
    }
    result.push(text[start..].trim().to_ascii_uppercase());
    result
}

fn sort_symbols(dds: &HashMap<String, PathBuf>) -> std::result::Result<SortSymbols, String> {
    let mut fields = HashMap::new();
    let mut constants = HashMap::new();
    let Some(path) = dds.get("SYMNAMES") else {
        return Ok((fields, constants));
    };
    let text = fs::read_to_string(path).map_err(|error| format!("SYMNAMES: {error}"))?;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('*') {
            continue;
        }
        let tokens = csv(line);
        let name = tokens[0].clone();
        if tokens.len() == 4 && matches!(tokens[3].as_str(), "CH" | "ZD") {
            let pos: usize = tokens[1]
                .parse()
                .map_err(|_| format!("SYMNAMES invalid position: {line}"))?;
            let len: usize = tokens[2]
                .parse()
                .map_err(|_| format!("SYMNAMES invalid length: {line}"))?;
            if pos == 0 || len == 0 {
                return Err(format!("SYMNAMES invalid field: {line}"));
            }
            fields.insert(name, Field { pos: pos - 1, len });
        } else if tokens.len() == 2 && tokens[1].starts_with("C'") && tokens[1].ends_with('\'') {
            constants.insert(name, tokens[1].as_bytes()[2..tokens[1].len() - 1].to_vec());
        } else {
            return Err(format!("unsupported SYMNAMES definition: {line}"));
        }
    }
    Ok((fields, constants))
}

fn field_bytes(record: &[u8], field: Field) -> std::result::Result<&[u8], String> {
    record
        .get(field.pos..field.pos + field.len)
        .ok_or_else(|| "SORTIN contains a short record".to_string())
}

fn sort(dds: &HashMap<String, PathBuf>) -> std::result::Result<(i32, String), String> {
    let (symbols, constants) = sort_symbols(dds)?;
    let mut keys: Vec<(Field, bool)> = Vec::new();
    let mut include = None;
    let mut outrec = None;
    let mut copy = false;
    for card in cards(dds, "SYSIN")? {
        let upper = card.to_ascii_uppercase();
        if upper.trim() == "OPTION COPY" || upper.trim() == "SORT FIELDS=COPY" {
            copy = true;
        } else if upper.starts_with("SORT FIELDS=(") {
            let value = operand(&card, "FIELDS").ok_or("SORT FIELDS requires fields")?;
            let tokens = csv(&value);
            let mut index = 0;
            while index < tokens.len() {
                let (field, order, advance) = if let Some(field) = symbols.get(&tokens[index]) {
                    (*field, tokens.get(index + 1), 2)
                } else {
                    let pos: usize = tokens[index]
                        .parse()
                        .map_err(|_| format!("invalid SORT field: {card}"))?;
                    let len: usize = tokens
                        .get(index + 1)
                        .ok_or("missing SORT field length")?
                        .parse()
                        .map_err(|_| format!("invalid SORT field length: {card}"))?;
                    if pos == 0
                        || len == 0
                        || !matches!(tokens.get(index + 2).map(String::as_str), Some("CH" | "ZD"))
                    {
                        return Err(format!("unsupported SORT field: {card}"));
                    }
                    (Field { pos: pos - 1, len }, tokens.get(index + 3), 4)
                };
                let descending = match order.map(String::as_str) {
                    Some("A") => false,
                    Some("D") => true,
                    _ => return Err(format!("invalid SORT order: {card}")),
                };
                keys.push((field, descending));
                index += advance;
            }
        } else if upper.starts_with("INCLUDE COND=(") {
            include = Some(csv(&operand(&card, "COND").ok_or("INCLUDE requires COND")?));
        } else if upper.starts_with("OUTREC FIELDS=(") {
            outrec = Some(csv(
                &operand(&card, "FIELDS").ok_or("OUTREC requires FIELDS")?
            ));
        } else {
            return Err(format!("unsupported SORT control statement: {card}"));
        }
    }
    if !copy && keys.is_empty() {
        return Err("SORT requires SORT FIELDS or OPTION COPY".to_string());
    }
    let input_path = dd(dds, "SORTIN")?;
    let output_path = dd(dds, "SORTOUT")?;
    let input = fs::read(input_path).map_err(|error| error.to_string())?;
    let input_layout = records::read_layout(input_path)?;
    let output_layout = records::read_layout(output_path)?;
    let mut records: Vec<Vec<u8>> = records::records(&input, input_layout)?
        .into_iter()
        .map(|record| record.to_vec())
        .collect();
    for record in &records {
        for (field, _) in &keys {
            field_bytes(record, *field)?;
        }
    }
    if let Some(condition) = &include {
        let selected: Vec<bool> = records
            .iter()
            .map(|record| include_match(record, condition, &symbols, &constants))
            .collect::<std::result::Result<_, _>>()?;
        records = records
            .into_iter()
            .zip(selected)
            .filter_map(|(record, keep)| keep.then_some(record))
            .collect();
    }
    if !copy {
        records.sort_by(|a, b| {
            for (field, descending) in &keys {
                let cmp =
                    a[field.pos..field.pos + field.len].cmp(&b[field.pos..field.pos + field.len]);
                if !cmp.is_eq() {
                    return if *descending { cmp.reverse() } else { cmp };
                }
            }
            std::cmp::Ordering::Equal
        });
    }
    let fixed = input_layout.is_some() || output_layout.is_some();
    let mut output = Vec::new();
    let mut output_len = output_layout.map(|layout| layout.lrecl);
    for record in &records {
        let bytes = if let Some(fields) = &outrec {
            reformat(record, fields, &symbols)?
        } else {
            record.clone()
        };
        if fixed {
            if output_len.is_some_and(|length| length != bytes.len()) {
                return Err(format!(
                    "SORTOUT FB record length {} differs from LRECL {:?}",
                    bytes.len(),
                    output_len
                ));
            }
            output_len = Some(bytes.len());
        }
        output.extend(bytes);
        if !fixed {
            output.push(b'\n');
        }
    }
    if fixed {
        let mut layout = output_layout.or(input_layout).ok_or("FB layout missing")?;
        layout.lrecl = output_len.unwrap_or(layout.lrecl);
        layout.key_len = 0;
        layout.key_offset = 0;
        records::write_layout(output_path, layout)?;
    }
    fs::write(output_path, output).map_err(|error| error.to_string())?;
    Ok((0, format!("SORT: {} records written\n", records.len())))
}

fn include_match(
    record: &[u8],
    tokens: &[String],
    fields: &HashMap<String, Field>,
    constants: &HashMap<String, Vec<u8>>,
) -> std::result::Result<bool, String> {
    let mut index = 0;
    let mut total = false;
    let mut join = "OR";
    while index < tokens.len() {
        if index + 2 >= tokens.len() {
            return Err("INCLUDE COND requires field, comparison, constant".to_string());
        }
        let field = fields
            .get(&tokens[index])
            .ok_or_else(|| format!("unknown INCLUDE field {}", tokens[index]))?;
        let left = field_bytes(record, *field)?;
        let literal = constants
            .get(&tokens[index + 2])
            .cloned()
            .or_else(|| {
                let value = &tokens[index + 2];
                value
                    .strip_prefix("C'")
                    .and_then(|value| value.strip_suffix('\''))
                    .map(|value| value.as_bytes().to_vec())
            })
            .ok_or_else(|| format!("unknown INCLUDE constant {}", tokens[index + 2]))?;
        let cmp = left.cmp(literal.as_slice());
        let current = match tokens[index + 1].as_str() {
            "EQ" => cmp.is_eq(),
            "NE" => !cmp.is_eq(),
            "GT" => cmp.is_gt(),
            "GE" => cmp.is_ge(),
            "LT" => cmp.is_lt(),
            "LE" => cmp.is_le(),
            other => return Err(format!("unsupported INCLUDE comparison {other}")),
        };
        total = if join == "AND" {
            total && current
        } else {
            total || current
        };
        index += 3;
        if index < tokens.len() {
            join = match tokens[index].as_str() {
                "AND" => "AND",
                "OR" => "OR",
                other => return Err(format!("unsupported INCLUDE connector {other}")),
            };
            index += 1;
        }
    }
    Ok(total)
}

fn reformat(
    record: &[u8],
    tokens: &[String],
    symbols: &HashMap<String, Field>,
) -> std::result::Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if token == "X"
            || token.ends_with('X')
                && token[..token.len() - 1]
                    .bytes()
                    .all(|byte| byte.is_ascii_digit())
        {
            let count = if token == "X" {
                1
            } else {
                token[..token.len() - 1]
                    .parse()
                    .map_err(|_| "invalid OUTREC spacing")?
            };
            output.extend(vec![b' '; count]);
            index += 1;
            continue;
        }
        let (destination, source) = if let Some((to, from)) = token.split_once(':') {
            let to: usize = to
                .parse()
                .map_err(|_| format!("invalid OUTREC destination {token}"))?;
            if to == 0 {
                return Err("OUTREC positions start at 1".to_string());
            }
            (Some(to - 1), from)
        } else {
            (None, token.as_str())
        };
        let (field, advance) = if let Some(field) = symbols.get(source) {
            (*field, 1)
        } else {
            let pos: usize = source
                .parse()
                .map_err(|_| format!("unsupported OUTREC field {token}"))?;
            let len: usize = tokens
                .get(index + 1)
                .ok_or("OUTREC field length is missing")?
                .parse()
                .map_err(|_| "invalid OUTREC field length")?;
            if pos == 0 || len == 0 {
                return Err("OUTREC positions and lengths must be positive".to_string());
            }
            (Field { pos: pos - 1, len }, 2)
        };
        let mut bytes = field_bytes(record, field)?.to_vec();
        index += advance;
        if tokens
            .get(index)
            .is_some_and(|next| next.starts_with("EDIT=("))
        {
            let edit = tokens[index]
                .trim_start_matches("EDIT=(")
                .trim_end_matches(')');
            let decimals = edit.split('.').nth(1).map_or(0, str::len);
            if decimals > 0 && bytes.len() >= decimals {
                bytes.insert(bytes.len() - decimals, b'.');
            }
            index += 1;
        }
        if let Some(destination) = destination {
            if output.len() < destination + bytes.len() {
                output.resize(destination + bytes.len(), b' ');
            }
            output[destination..destination + bytes.len()].copy_from_slice(&bytes);
        } else {
            output.extend(bytes);
        }
    }
    Ok(output)
}
