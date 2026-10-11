//! Encode the reference IMS CSV load files into their DBD segment byte layouts.
use crate::catalog::{Dbd, Field};
use std::sync::OnceLock;

fn csv_row(line: &str) -> Result<Vec<String>, String> {
    let mut fields = Vec::new();
    let mut value = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                value.push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut value)),
            other => value.push(other),
        }
    }
    if quoted {
        return Err("unterminated CSV quote".into());
    }
    fields.push(value);
    Ok(fields)
}

pub(crate) fn cp1047_to_ascii_map() -> &'static [u8; 256] {
    static MAP: OnceLock<[u8; 256]> = OnceLock::new();
    MAP.get_or_init(|| {
        // Use the same table as GnuCOBOL's default EBCDIC mode.
        let table: Vec<u8> = include_str!("../../../config/default.ttbl")
            .lines()
            .filter_map(|line| line.split('#').next())
            .flat_map(str::split_whitespace)
            .take(256)
            .map(|hex| u8::from_str_radix(hex, 16).expect("GnuCOBOL EBCDIC table"))
            .collect();
        table
            .try_into()
            .expect("GnuCOBOL EBCDIC table has 256 bytes")
    })
}

pub(crate) fn cp1047_ascii_map() -> &'static [u8; 128] {
    static MAP: OnceLock<[u8; 128]> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut reverse = [0u8; 128];
        for (ebcdic, ascii) in cp1047_to_ascii_map().iter().enumerate() {
            if *ascii < 128 {
                reverse[*ascii as usize] = ebcdic as u8;
            }
        }
        reverse
    })
}

fn text(value: &str, width: usize, cp1047: bool) -> Result<Vec<u8>, String> {
    if !value.is_ascii() {
        return Err("CSV text must be ASCII for the Cp1047 loader".into());
    }
    if value.len() > width {
        return Err(format!("text exceeds {width} bytes"));
    }
    let mut out = vec![if cp1047 { 0x40 } else { b' ' }; width];
    for (target, byte) in out.iter_mut().zip(value.bytes()) {
        *target = if cp1047 {
            cp1047_ascii_map()[byte as usize]
        } else {
            byte
        };
    }
    Ok(out)
}

fn integer(value: &str, width: usize) -> Result<Vec<u8>, String> {
    let number: i64 = value
        .trim()
        .parse()
        .map_err(|_| format!("invalid binary integer {value:?}"))?;
    match width {
        4 => Ok(i32::try_from(number)
            .map_err(|_| format!("integer exceeds 4 bytes: {value}"))?
            .to_be_bytes()
            .to_vec()),
        8 => Ok(number.to_be_bytes().to_vec()),
        _ => Err(format!("unsupported binary integer width {width}")),
    }
}

fn packed(value: &str, width: usize, datatype: &str) -> Result<Vec<u8>, String> {
    let shape = datatype
        .strip_prefix("DECIMAL(")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| format!("unsupported decimal type {datatype}"))?;
    let (precision, scale) = shape
        .split_once(',')
        .ok_or_else(|| format!("invalid decimal type {datatype}"))?;
    let precision: usize = precision.parse().map_err(|_| "invalid precision")?;
    let scale: usize = scale.parse().map_err(|_| "invalid scale")?;
    if precision == 0 || scale > precision || width != (precision + 2) / 2 {
        return Err(format!("{datatype} does not fit {width} bytes"));
    }
    let value = value.trim();
    let (negative, unsigned) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || !fraction.bytes().all(|c| c.is_ascii_digit())
        || fraction.len() > scale
        || whole.len() > precision - scale
    {
        return Err(format!("invalid {datatype} value {value:?}"));
    }
    let mut digits = format!("{whole}{fraction}");
    digits.extend(std::iter::repeat_n('0', scale - fraction.len()));
    while digits.len() < precision {
        digits.insert(0, '0');
    }
    let mut nibbles = vec![0u8; width * 2 - precision - 1];
    nibbles.extend(digits.bytes().map(|digit| digit - b'0'));
    nibbles.push(if negative { 0x0d } else { 0x0c });
    Ok(nibbles
        .chunks_exact(2)
        .map(|pair| pair[0] << 4 | pair[1])
        .collect())
}

fn encode_field(field: &Field, value: &str, cp1047: bool) -> Result<Vec<u8>, String> {
    match field.datatype.as_deref() {
        Some("INT" | "LONG") => integer(value, field.bytes),
        Some("CHAR" | "TIMESTAMP") => text(value, field.bytes, cp1047),
        Some(kind) if kind.starts_with("DECIMAL(") => packed(value, field.bytes, kind),
        other => Err(format!("{}: unsupported DATATYPE {other:?}", field.name)),
    }
}

/// Encode a CSV with one column per non-overlapping DBD field into root segment bytes.
/// Binary INT/LONG fields use z/OS big-endian order; DECIMAL fields use signed packed decimal.
pub fn encode_root_csv(dbd: &Dbd, segment: &str, source: &str) -> Result<Vec<Vec<u8>>, String> {
    let definition = dbd.segment(segment).ok_or("unknown segment")?;
    if definition.parent.is_some() || definition.min_bytes != definition.max_bytes {
        return Err("CSV loading requires a fixed-length root segment".into());
    }
    let cp1047 = match dbd.encoding.as_deref() {
        Some(name) if name.eq_ignore_ascii_case("CP1047") => true,
        None => false,
        Some(name) => return Err(format!("unsupported DBD encoding {name}")),
    };
    let mut fields: Vec<&Field> = definition
        .fields
        .iter()
        .filter(|field| field.datatype.as_deref() != Some("BINARY"))
        .collect();
    fields.sort_by_key(|field| field.start);
    let mut next = 1;
    for field in &fields {
        if field.start != next {
            return Err(format!(
                "{}: CSV field layout has gap or overlap",
                field.name
            ));
        }
        next += field.bytes;
    }
    if next != definition.max_bytes + 1 {
        return Err("CSV fields do not cover the entire segment".into());
    }
    let mut records = Vec::new();
    for (line_number, line) in source.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let columns = csv_row(line).map_err(|e| format!("line {}: {e}", line_number + 1))?;
        if columns.len() != fields.len() {
            return Err(format!(
                "line {}: expected {} columns; got {}",
                line_number + 1,
                fields.len(),
                columns.len()
            ));
        }
        let mut record = Vec::with_capacity(definition.max_bytes);
        for (field, value) in fields.iter().zip(columns.iter()) {
            record.extend(
                encode_field(field, value, cp1047)
                    .map_err(|e| format!("line {}: {}: {e}", line_number + 1, field.name))?,
            );
        }
        definition
            .validate_data(&record)
            .map_err(|e| format!("line {}: {e}", line_number + 1))?;
        records.push(record);
    }
    Ok(records)
}
