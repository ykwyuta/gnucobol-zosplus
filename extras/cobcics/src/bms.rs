//! BMS map definitions used by the TN3270 host and COBOL symbolic maps.

use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: Option<String>,
    pub row: usize,
    pub column: usize,
    pub length: usize,
    pub occurs: usize,
    pub picout: Option<String>,
    pub initial: String,
    pub occurrence_values: Vec<Option<String>>,
    pub protected: bool,
    pub numeric: bool,
    pub intensified: bool,
    pub hidden: bool,
    pub initial_cursor: bool,
    pub fset: bool,
    pub justify_right: bool,
    pub justify_zero: bool,
    pub color: Option<u8>,
    pub highlight: Option<u8>,
}

impl Field {
    pub fn value_at(&self, index: usize) -> &str {
        self.occurrence_values
            .get(index)
            .and_then(Option::as_deref)
            .unwrap_or(&self.initial)
    }

    pub fn wire_name(&self, index: usize) -> Option<String> {
        let name = self.name.as_ref()?;
        if self.occurs == 1 {
            Some(name.clone())
        } else {
            Some(format!("{:<4}{:04}", &name[..name.len().min(4)], index + 1))
        }
    }

    pub fn attribute_address(&self, columns: usize) -> usize {
        (self.row - 1) * columns + self.column - 1
    }

    pub fn data_address(&self, columns: usize) -> usize {
        self.attribute_address(columns) + 1
    }

    pub fn attribute_address_at(&self, columns: usize, index: usize) -> usize {
        self.attribute_address(columns) + index * (self.length + 1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Map {
    pub mapset: String,
    pub name: String,
    pub tioapfx: bool,
    pub extended_attributes: Vec<char>,
    pub rows: usize,
    pub columns: usize,
    pub fields: Vec<Field>,
}

fn statements(source: &str) -> Result<Vec<(usize, String)>, String> {
    let mut result = Vec::new();
    let mut pending = String::new();
    let mut first_line = 0;
    for (index, raw) in source.lines().enumerate() {
        let line_number = index + 1;
        if raw.trim().is_empty() || raw.starts_with('*') {
            continue;
        }
        // BMS columns count source characters; UTF-8 storage bytes must not
        // split a Japanese INITIAL literal at the continuation margin.
        let body = raw.chars().take(71).collect::<String>();
        let body = body.trim();
        if pending.is_empty() {
            first_line = line_number;
        } else if pending.bytes().filter(|byte| *byte == b'\'').count() % 2 == 0 {
            // BMS continuation inside an INITIAL literal joins the next
            // source fragment directly (for example "fund" + "s.").
            pending.push(' ');
        }
        pending.push_str(body);
        let continuation = raw.chars().nth(71).is_some_and(|ch| ch != ' ')
            || (raw.chars().count() < 72 && raw.trim_end().ends_with('*'));
        if continuation && pending.ends_with('*') {
            pending.pop();
        }
        if !continuation {
            result.push((first_line, std::mem::take(&mut pending)));
        }
    }
    if !pending.is_empty() {
        return Err(format!("line {first_line}: unfinished BMS continuation"));
    }
    Ok(result)
}

fn options(input: &str) -> Result<Vec<(String, String)>, String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut quote = false;
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\'' => {
                if quote && bytes.get(index + 1) == Some(&b'\'') {
                    index += 1;
                } else {
                    quote = !quote;
                }
            }
            b'(' if !quote => depth += 1,
            b')' if !quote => {
                if depth == 0 {
                    return Err("unmatched ')' in BMS options".to_owned());
                }
                depth -= 1;
            }
            b',' if !quote && depth == 0 => {
                parts.push(input[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    if depth != 0 || quote {
        return Err("unfinished BMS option".to_owned());
    }
    parts.push(input[start..].trim());
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (key, value) = part
                .split_once('=')
                .ok_or_else(|| format!("BMS option requires '=': {part}"))?;
            Ok((key.trim().to_ascii_uppercase(), value.trim().to_owned()))
        })
        .collect()
}

fn value<'a>(options: &'a [(String, String)], key: &str) -> Option<&'a str> {
    options
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

fn known_options(
    options: &[(String, String)],
    allowed: &[&str],
    line: usize,
) -> Result<(), String> {
    for (name, _) in options {
        if !allowed.contains(&name.as_str()) {
            return Err(format!("line {line}: unsupported BMS option {name}"));
        }
    }
    Ok(())
}

fn pair(value: &str) -> Result<(usize, usize), String> {
    let content = value
        .strip_prefix('(')
        .and_then(|v| v.strip_suffix(')'))
        .ok_or_else(|| format!("expected a pair: {value}"))?;
    let (first, second) = content
        .split_once(',')
        .ok_or_else(|| format!("expected a pair: {value}"))?;
    let parse = |part: &str| {
        part.trim()
            .parse::<usize>()
            .map_err(|_| format!("invalid BMS number: {part}"))
    };
    Ok((parse(first)?, parse(second)?))
}

fn quoted(value: &str) -> Result<String, String> {
    value
        .strip_prefix('\'')
        .and_then(|v| v.strip_suffix('\''))
        .map(|v| v.replace("''", "'"))
        .ok_or_else(|| format!("expected a quoted BMS value: {value}"))
}

pub fn parse(source: &str) -> Result<Vec<Map>, String> {
    let mut maps: Vec<Map> = Vec::new();
    let mut mapset = String::new();
    let mut tioapfx = false;
    let mut extended_attributes = Vec::new();
    for (line, statement) in statements(source)? {
        let words: Vec<&str> = statement.split_whitespace().collect();
        let Some(macro_index) = words
            .iter()
            .position(|word| matches!(*word, "DFHMSD" | "DFHMDI" | "DFHMDF"))
        else {
            continue;
        };
        let macro_name = words[macro_index];
        let label = if macro_index > 0 {
            Some(words[macro_index - 1].to_ascii_uppercase())
        } else {
            None
        };
        let option_start = statement
            .find(macro_name)
            .expect("macro token occurs in statement")
            + macro_name.len();
        let opts = options(statement[option_start..].trim())
            .map_err(|err| format!("line {line}: {err}"))?;
        match macro_name {
            "DFHMSD" => {
                known_options(
                    &opts,
                    &[
                        "TYPE", "MODE", "LANG", "STORAGE", "CTRL", "EXTATT", "TERM", "TIOAPFX",
                        "MAPATTS", "DSATTS",
                    ],
                    line,
                )?;
                if value(&opts, "TYPE") == Some("FINAL") {
                    continue;
                }
                if value(&opts, "MODE").is_some_and(|mode| mode != "INOUT") {
                    return Err(format!("line {line}: only MODE=INOUT is supported"));
                }
                if value(&opts, "LANG").is_some_and(|lang| lang != "COBOL") {
                    return Err(format!("line {line}: only LANG=COBOL is supported"));
                }
                mapset = label.ok_or_else(|| format!("line {line}: unnamed mapset"))?;
                tioapfx = match value(&opts, "TIOAPFX") {
                    Some("YES") => true,
                    Some("NO") | None => false,
                    Some(other) => return Err(format!("line {line}: unsupported TIOAPFX={other}")),
                };
                extended_attributes = attributes(value(&opts, "DSATTS"))?;
            }
            "DFHMDI" => {
                known_options(&opts, &["SIZE", "COLUMN", "LINE", "DSATTS"], line)?;
                if value(&opts, "LINE").is_some_and(|line| line != "1")
                    || value(&opts, "COLUMN").is_some_and(|column| column != "1")
                {
                    return Err(format!("line {line}: map LINE/COLUMN must be 1"));
                }
                let name = label.ok_or_else(|| format!("line {line}: unnamed map"))?;
                let (rows, columns) = pair(
                    value(&opts, "SIZE")
                        .ok_or_else(|| format!("line {line}: map SIZE is required"))?,
                )?;
                if mapset.is_empty()
                    || rows == 0
                    || columns == 0
                    || rows.checked_mul(columns).is_none_or(|size| size > 16383)
                {
                    return Err(format!("line {line}: invalid map geometry or mapset"));
                }
                maps.push(Map {
                    mapset: mapset.clone(),
                    name,
                    tioapfx,
                    extended_attributes: if value(&opts, "DSATTS").is_some() {
                        attributes(value(&opts, "DSATTS"))?
                    } else {
                        extended_attributes.clone()
                    },
                    rows,
                    columns,
                    fields: Vec::new(),
                });
            }
            "DFHMDF" => {
                known_options(
                    &opts,
                    &[
                        "POS", "LENGTH", "INITIAL", "ATTRB", "COLOR", "HILIGHT", "OUTLINE", "PS",
                        "SOSI", "PICOUT", "OCCURS", "JUSTIFY", "XINIT", "VALIDN",
                    ],
                    line,
                )?;
                let map = maps
                    .last_mut()
                    .ok_or_else(|| format!("line {line}: field precedes map"))?;
                let (row, column) = pair(
                    value(&opts, "POS")
                        .ok_or_else(|| format!("line {line}: field POS is required"))?,
                )?;
                let length = value(&opts, "LENGTH")
                    .ok_or_else(|| format!("line {line}: field LENGTH is required"))?
                    .parse::<usize>()
                    .map_err(|_| format!("line {line}: invalid field LENGTH"))?;
                let occurs = value(&opts, "OCCURS")
                    .map(str::parse::<usize>)
                    .transpose()
                    .map_err(|_| format!("line {line}: invalid OCCURS"))?
                    .unwrap_or(1);
                let picout = value(&opts, "PICOUT").map(quoted).transpose()?;
                if value(&opts, "INITIAL").is_some() && value(&opts, "XINIT").is_some() {
                    return Err(format!("line {line}: INITIAL and XINIT are mutually exclusive"));
                }
                let initial = if let Some(hex) = value(&opts, "XINIT") {
                    let hex = quoted(hex)?;
                    if hex.len() != length * 2 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        return Err(format!("line {line}: XINIT must contain LENGTH hexadecimal bytes"));
                    }
                    let mut decoded = String::with_capacity(length);
                    for pair in hex.as_bytes().chunks_exact(2) {
                        let digits = std::str::from_utf8(pair).expect("validated ASCII hex");
                        let byte = u8::from_str_radix(digits, 16).expect("validated hex digits");
                        if byte != 0 {
                            return Err(format!("line {line}: nonzero XINIT bytes are not yet supported"));
                        }
                        decoded.push('\0');
                    }
                    decoded
                } else {
                    value(&opts, "INITIAL")
                        .map(quoted)
                        .transpose()?
                        .unwrap_or_default()
                };
                let (justify_right, justify_zero) = match value(&opts, "JUSTIFY") {
                    None | Some("LEFT") | Some("(BLANK)") => (false, false),
                    Some("RIGHT") | Some("(RIGHT,BLANK)") => (true, false),
                    Some("(RIGHT,ZERO)") => (true, true),
                    Some(_) => return Err(format!("line {line}: invalid JUSTIFY")),
                };
                if value(&opts, "VALIDN").is_some_and(|rule| rule != "(MUSTENTER)") {
                    return Err(format!("line {line}: unsupported VALIDN rule"));
                }
                let attr = value(&opts, "ATTRB")
                    .map(|v| v.trim_matches(['(', ')']))
                    .unwrap_or("")
                    .to_ascii_uppercase();
                let attributes: HashSet<&str> = attr.split(',').map(str::trim).collect();
                let color = value(&opts, "COLOR").map(|name| match name {
                    "BLUE" => Ok(0xf1), "RED" => Ok(0xf2), "PINK" => Ok(0xf3),
                    "GREEN" => Ok(0xf4), "TURQUOISE" => Ok(0xf5),
                    "YELLOW" => Ok(0xf6), "NEUTRAL" => Ok(0xf7),
                    _ => Err(format!("line {line}: unsupported COLOR={name}")),
                }).transpose()?;
                let highlight = value(&opts, "HILIGHT").map(|name| match name {
                    "OFF" | "NORMAL" => Ok(0xf0), "BLINK" => Ok(0xf1),
                    "REVERSE" => Ok(0xf2), "UNDERLINE" => Ok(0xf4),
                    _ => Err(format!("line {line}: unsupported HILIGHT={name}")),
                }).transpose()?;
                let protected = attributes.contains("PROT")
                    || attributes.contains("ASKIP")
                    || (label.is_none() && !attributes.contains("UNPROT"));
                let start = row.saturating_sub(1) * map.columns + column.saturating_sub(1);
                let end = length
                    .checked_add(1)
                    .and_then(|stride| stride.checked_mul(occurs))
                    .and_then(|span| start.checked_add(span));
                let end_column = column.checked_add(length);
                if row == 0
                    || row > map.rows
                    || column == 0
                    || column > map.columns
                    || occurs == 0
                    || end.is_none_or(|end| end > map.rows * map.columns)
                    || end_column.is_none_or(|end| end > map.columns)
                {
                    return Err(format!("line {line}: field exceeds map or LENGTH"));
                }
                if label.as_ref().is_some_and(|name| {
                    map.fields
                        .iter()
                        .any(|field| field.name.as_ref() == Some(name))
                }) {
                    return Err(format!("line {line}: duplicate BMS field name"));
                }
                map.fields.push(Field {
                    name: label,
                    row,
                    column,
                    length,
                    occurs,
                    picout,
                    initial: initial.chars().take(length).collect(),
                    occurrence_values: vec![None; occurs],
                    protected,
                    numeric: attributes.contains("NUM"),
                    intensified: attributes.contains("BRT"),
                    hidden: attributes.contains("DRK"),
                    initial_cursor: attributes.contains("IC"),
                    fset: attributes.contains("FSET"),
                    justify_right,
                    justify_zero,
                    color,
                    highlight,
                });
            }
            _ => unreachable!(),
        }
    }
    if maps.is_empty() {
        return Err("BMS source contains no DFHMDI map".to_owned());
    }
    Ok(maps)
}

fn attributes(raw: Option<&str>) -> Result<Vec<char>, String> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let raw = raw.trim_matches(['(', ')']);
    let mut result = Vec::new();
    for (name, suffix) in [
        ("COLOR", 'C'),
        ("HILIGHT", 'H'),
        ("OUTLINE", 'U'),
        ("TRANSP", 'T'),
        ("VALIDN", 'V'),
        ("SOSI", 'M'),
        ("PS", 'P'),
    ] {
        if raw.split(',').any(|part| part.trim() == name) {
            result.push(suffix);
        }
    }
    for name in raw.split(',') {
        if !name.trim().is_empty()
            && !matches!(
                name.trim(),
                "COLOR" | "HILIGHT" | "OUTLINE" | "TRANSP" | "VALIDN" | "SOSI" | "PS"
            )
        {
            return Err(format!("unsupported DSATTS attribute: {name}"));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bank_of_z_menu() {
        let source =
            include_str!("../../../reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms");
        let maps = parse(source).unwrap();
        assert_eq!(maps.len(), 1);
        let map = &maps[0];
        assert_eq!(
            (&*map.mapset, &*map.name, map.rows, map.columns),
            ("BNK1MAI", "BNK1ME", 24, 80)
        );
        assert!(map.tioapfx);
        let action = map
            .fields
            .iter()
            .find(|f| f.name.as_deref() == Some("ACTION"))
            .unwrap();
        assert_eq!((action.row, action.column, action.length), (5, 16, 1));
        assert!(!action.protected);
        assert!(action.initial_cursor);
    }

    #[test]
    fn utf8_japanese_initial_is_preserved() {
        let source = "JPMAP DFHMSD TYPE=MAP\nJPSCR DFHMDI SIZE=(24,80)\nNAME DFHMDF POS=(2,1),LENGTH=12,ATTRB=UNPROT,INITIAL='日本語'\n";
        let map = parse(source).unwrap().remove(0);
        assert_eq!(map.fields[0].initial, "日本語");
        assert_eq!(map.fields[0].length, 12);
    }

    #[test]
    fn allows_attribute_to_overlay_previous_data() {
        let source = "M DFHMSD TYPE=MAP\nP DFHMDI SIZE=(24,80)\nA DFHMDF POS=(1,1),LENGTH=3\nB DFHMDF POS=(1,4),LENGTH=2\n";
        assert_eq!(parse(source).unwrap()[0].fields.len(), 2);
    }

    #[test]
    fn unsupported_bms_options_fail_at_source_line() {
        let source = "M DFHMSD TYPE=MAP,MODE=INOUT\nP DFHMDI SIZE=(24,80)\nA DFHMDF POS=(1,1),LENGTH=3,GRPNAME=G\n";
        assert!(parse(source)
            .unwrap_err()
            .contains("line 3: unsupported BMS option GRPNAME"));
    }

    #[test]
    fn joins_bms_initial_literal_without_inserting_a_space() {
        let source = "M DFHMSD TYPE=MAP\nP DFHMDI SIZE=(24,80)\nA DFHMDF POS=(1,1),LENGTH=15,INITIAL='Transfer fund*\n               s.'\n";
        let map = parse(source).unwrap().remove(0);
        assert_eq!(map.fields[0].initial, "Transfer funds.");
    }

    #[test]
    fn genapp_mapset_and_zero_initialized_fields() {
        let source = include_str!("../../../reference/cics-genapp/base/src/ssmap.bms");
        assert_eq!(source.matches("INITIAL='Fund Name '    '").count(), 1);
        let source = source.replace("INITIAL='Fund Name '    '", "INITIAL='Fund Name '");
        let maps = parse(&source).unwrap();
        assert_eq!(maps.len(), 6);
        assert_eq!(maps[0].name, "SSMAPC1");
        assert_eq!(maps[5].name, "SSMAPP5");
        let paid = maps[5].fields.iter()
            .find(|field| field.name.as_deref() == Some("ENP5PAD"))
            .unwrap();
        assert!(paid.justify_right && paid.justify_zero);
        assert_eq!(paid.initial, "\0".repeat(10));
    }
}
