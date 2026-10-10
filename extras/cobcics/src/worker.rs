//! Versioned, length-delimited pipe protocol for an isolated COBOL task.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::bms::Map;
use crate::codec::HostCodec;
use crate::tn3270::Input;

const MAX_FIELDS: usize = 64;
const MAX_COMMAREA: usize = 2048;
const MAX_TEXT: usize = 8192; // UTF-8 may use more bytes than terminal EBCDIC.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenAction {
    None,
    Map,
    Erase,
    DataOnly,
    DataOnlyAlarm,
    Text,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub next: Option<Session>,
    pub immediate: bool,
    pub screen: ScreenAction,
    pub changed_fields: Vec<usize>,
    pub changed_attributes: Vec<usize>,
    pub attribute_updates: Vec<(usize, bool, bool, bool)>,
    pub extended_updates: Vec<(usize, u8, u8)>,
    pub text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub transid: [u8; 4],
    pub commarea: Vec<u8>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            transid: *b"    ",
            commarea: Vec::new(),
        }
    }
}

pub fn transid(name: &str) -> Result<[u8; 4], String> {
    if name.is_empty()
        || name.len() > 4
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return Err(format!("invalid TRANSID: {name}"));
    }
    let mut result = [b' '; 4];
    result[..name.len()].copy_from_slice(name.as_bytes());
    Ok(result)
}

pub fn applid(name: &str) -> Result<[u8; 8], String> {
    if name.is_empty()
        || name.len() > 8
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return Err(format!("invalid APPLID: {name}"));
    }
    let mut result = [b' '; 8];
    result[..name.len()].copy_from_slice(name.as_bytes());
    Ok(result)
}

fn name_bytes(name: &str) -> Result<[u8; 8], String> {
    if name.is_empty()
        || name.len() > 8
        || !name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(format!("invalid worker field name: {name}"));
    }
    let mut result = [b' '; 8];
    result[..name.len()].copy_from_slice(name.as_bytes());
    Ok(result)
}

pub fn request(input: &Input, session: &Session, applid: &[u8; 8]) -> Result<Vec<u8>, String> {
    request_with_codec(input, session, applid, HostCodec::Cp037)
}

pub fn request_with_codec(input: &Input, session: &Session, applid: &[u8; 8],
                          codec: HostCodec) -> Result<Vec<u8>, String> {
    if input.fields.len() > MAX_FIELDS {
        return Err("too many modified fields".to_owned());
    }
    if session.commarea.len() > MAX_COMMAREA {
        return Err("COMMAREA exceeds 2048 bytes".to_owned());
    }
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"CCI3");
    bytes.push(input.aid);
    bytes.extend_from_slice(&session.transid);
    bytes.extend_from_slice(applid);
    bytes.extend_from_slice(&(session.commarea.len() as u16).to_be_bytes());
    bytes.extend_from_slice(&session.commarea);
    bytes.extend_from_slice(&(input.fields.len() as u16).to_be_bytes());
    for (name, raw) in &input.fields {
        let text = codec.decode(raw)?;
        if text.len() > 1024 {
            return Err(format!("input field {name} exceeds 1024 UTF-8 bytes"));
        }
        bytes.extend_from_slice(&name_bytes(name)?);
        bytes.extend_from_slice(&(text.len() as u16).to_be_bytes());
        bytes.extend_from_slice(text.as_bytes());
    }
    Ok(bytes)
}

#[cfg(test)]
fn response(map: &Map, bytes: &[u8]) -> Result<(Vec<(usize, usize, String)>, Outcome), String> {
    response_with_codec(map, bytes, HostCodec::Cp037)
}

fn response_with_codec(map: &Map, bytes: &[u8], codec: HostCodec)
    -> Result<(Vec<(usize, usize, String)>, Outcome), String> {
    if bytes.len() < 16 || !matches!(&bytes[..4], b"CCO5" | b"CCO6" | b"CCO7") {
        return Err("worker response has no CCO5/CCO6/CCO7 header".to_owned());
    }
    let count = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    if count > MAX_FIELDS {
        return Err("worker response has too many fields".to_owned());
    }
    let mut index = 6;
    let mut fields = Vec::new();
    for _ in 0..count {
        if bytes.len() - index < 10 {
            return Err("truncated worker field header".to_owned());
        }
        let name = std::str::from_utf8(&bytes[index..index + 8])
            .map_err(|_| "worker field name is not ASCII".to_owned())?
            .trim_end();
        let length = u16::from_be_bytes([bytes[index + 8], bytes[index + 9]]) as usize;
        index += 10;
        if length > 1024 || bytes.len() - index < length {
            return Err(format!("invalid worker field length: {name}"));
        }
        let (field_index, occurrence) = map
            .fields
            .iter()
            .enumerate()
            .find_map(|(field_index, field)| {
                (0..field.occurs)
                    .find(|&occurrence| field.wire_name(occurrence).as_deref() == Some(name))
                    .map(|occurrence| (field_index, occurrence))
            })
            .ok_or_else(|| format!("worker returned unknown field: {name}"))?;
        if fields
                .iter()
                .any(|(seen, item, _)| *seen == field_index && *item == occurrence)
        {
            return Err(format!("worker returned duplicate field: {name}"));
        }
        let text = std::str::from_utf8(&bytes[index..index + length])
            .map_err(|_| format!("worker field {name} is not UTF-8"))?;
        if codec.encode(text)?.len() > map.fields[field_index].length {
            return Err(format!("worker field {name} exceeds BMS EBCDIC length"));
        }
        fields.push((field_index, occurrence, text.to_owned()));
        index += length;
    }
    let mut attribute_updates = Vec::new();
    if matches!(&bytes[..4], b"CCO6" | b"CCO7") {
        if bytes.len() - index < 2 {
            return Err("truncated worker attribute count".to_owned());
        }
        let count = u16::from_be_bytes([bytes[index], bytes[index + 1]]) as usize;
        index += 2;
        if count > MAX_FIELDS || bytes.len() - index < count * 9 {
            return Err("invalid worker attribute count".to_owned());
        }
        for _ in 0..count {
            let name = std::str::from_utf8(&bytes[index..index + 8])
                .map_err(|_| "worker attribute name is not ASCII".to_owned())?
                .trim_end();
            let field_index = map.fields.iter().position(|field| {
                (0..field.occurs).any(|occurrence| {
                    field.wire_name(occurrence).as_deref() == Some(name)
                })
            }).ok_or_else(|| format!("worker returned unknown attribute: {name}"))?;
            if attribute_updates.iter().any(|(seen, _, _, _)| *seen == field_index) {
                return Err(format!("duplicate worker attribute: {name}"));
            }
            let value = bytes[index + 8];
            let (protected, hidden, fset) = match value {
                b'A' => (false, false, true),
                b')' => (true, false, true),
                b'0' => (true, true, false),
                _ => return Err(format!("unsupported BMS attribute {value:02X}: {name}")),
            };
            attribute_updates.push((field_index, protected, hidden, fset));
            index += 9;
        }
    }
    let mut extended_updates = Vec::new();
    if &bytes[..4] == b"CCO7" {
        if bytes.len() - index < 2 {
            return Err("truncated worker extended attribute count".to_owned());
        }
        let count = u16::from_be_bytes([bytes[index], bytes[index + 1]]) as usize;
        index += 2;
        if count > MAX_FIELDS * 2 || bytes.len() - index < count * 10 {
            return Err("invalid worker extended attribute count".to_owned());
        }
        for _ in 0..count {
            let name = std::str::from_utf8(&bytes[index..index + 8])
                .map_err(|_| "worker extended attribute name is not ASCII".to_owned())?
                .trim_end();
            let field_index = map.fields.iter().position(|field| {
                (0..field.occurs).any(|occurrence| {
                    field.wire_name(occurrence).as_deref() == Some(name)
                })
            }).ok_or_else(|| format!("worker returned unknown extended attribute: {name}"))?;
            let kind = bytes[index + 8];
            let value = bytes[index + 9];
            if !matches!((kind, value), (b'C', 0xf0..=0xff) |
                (b'H', 0xf0 | 0xf1 | 0xf2 | 0xf4)) {
                return Err(format!("invalid worker extended attribute {kind:02X}/{value:02X}"));
            }
            if extended_updates.iter().any(|(seen, old_kind, _)|
                *seen == field_index && *old_kind == kind) {
                return Err(format!("duplicate worker extended attribute: {name}"));
            }
            extended_updates.push((field_index, kind, value));
            index += 10;
        }
    }
    if bytes.len() - index < 8 {
        return Err("worker response lacks RETURN metadata".to_owned());
    }
    let screen = match bytes[index] {
        0 => ScreenAction::Map,
        1 if fields.is_empty() => ScreenAction::Erase,
        2 => ScreenAction::DataOnly,
        3 => ScreenAction::DataOnlyAlarm,
        4 if fields.is_empty() => ScreenAction::None,
        5 if fields.is_empty() => ScreenAction::Text,
        _ => return Err("worker returned invalid screen action".to_owned()),
    };
    index += 1;
    let immediate = match bytes[index] {
        0 => false,
        1 => true,
        _ => return Err("worker returned invalid IMMEDIATE flag".to_owned()),
    };
    index += 1;
    let transid = &bytes[index..index + 4];
    let commarea_len = u16::from_be_bytes([bytes[index + 4], bytes[index + 5]]) as usize;
    index += 6;
    if commarea_len > MAX_COMMAREA || bytes.len() - index < commarea_len + 2 {
        return Err("worker response has invalid COMMAREA length or trailing data".to_owned());
    }
    let commarea = bytes[index..index + commarea_len].to_vec();
    index += commarea_len;
    let text_len = u16::from_be_bytes([bytes[index], bytes[index + 1]]) as usize;
    index += 2;
    if text_len > MAX_TEXT || bytes.len() - index != text_len {
        return Err("worker response has invalid text length or trailing data".to_owned());
    }
    if screen != ScreenAction::Text && text_len != 0 {
        return Err("worker returned text without SEND TEXT".to_owned());
    }
    let text = if screen == ScreenAction::Text {
        let value = std::str::from_utf8(&bytes[index..])
            .map_err(|_| "worker text is not UTF-8".to_owned())?;
        if value.chars().any(char::is_control) || codec.encode(value)?.len() > 1919 {
            return Err("worker text is not printable or exceeds terminal length".to_owned());
        }
        Some(value.to_owned())
    } else {
        None
    };
    let next = if transid == b"    " {
        if immediate {
            return Err("worker returned IMMEDIATE without TRANSID".to_owned());
        }
        if commarea_len != 0 {
            return Err("worker returned COMMAREA without TRANSID".to_owned());
        }
        None
    } else {
        let name_end = transid.iter().position(|byte| *byte == b' ').unwrap_or(4);
        if name_end == 0
            || !transid[..name_end]
                .iter()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
            || !transid[name_end..].iter().all(|byte| *byte == b' ')
        {
            return Err("worker returned invalid TRANSID".to_owned());
        }
        let mut name = [0u8; 4];
        name.copy_from_slice(transid);
        Some(Session {
            transid: name,
            commarea,
        })
    };
    let mut changed_fields = Vec::new();
    for (field_index, _, _) in &fields {
        if !changed_fields.contains(field_index) {
            changed_fields.push(*field_index);
        }
    }
    let mut changed_attributes: Vec<usize> = attribute_updates.iter()
        .map(|(index, _, _, _)| *index).collect();
    for (index, _, _) in &extended_updates {
        if !changed_attributes.contains(index) { changed_attributes.push(*index); }
    }
    Ok((
        fields,
        Outcome {
            next,
            immediate,
            screen,
            changed_fields,
            changed_attributes,
            attribute_updates,
            extended_updates,
            text,
        },
    ))
}

pub fn run(
    worker: &Path,
    map: &mut Map,
    input: &Input,
    session: &Session,
    applid: &[u8; 8],
) -> Result<Outcome, String> {
    run_with_codec(worker, map, input, session, applid, HostCodec::Cp037)
}

pub fn run_with_codec(
    worker: &Path,
    map: &mut Map,
    input: &Input,
    session: &Session,
    applid: &[u8; 8],
    codec: HostCodec,
) -> Result<Outcome, String> {
    let request = request_with_codec(input, session, applid, codec)?;
    let mut child = Command::new(worker)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("start COBOL worker: {err}"))?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&request)
        .map_err(|err| format!("send worker request: {err}"))?;
    let output = child
        .wait_with_output()
        .map_err(|err| format!("wait for COBOL worker: {err}"))?;
    if std::env::var_os("COBCICS_WORKER_TRACE").is_some() && !output.stderr.is_empty() {
        eprintln!("COBOL worker diagnostics: {}",
                  String::from_utf8_lossy(&output.stderr).trim());
    }
    if !output.status.success() {
        return Err(format!(
            "COBOL worker exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let (fields, outcome) = response_with_codec(map, &output.stdout, codec)?;
    for &(index, protected, hidden, fset) in &outcome.attribute_updates {
        map.fields[index].protected = protected;
        map.fields[index].hidden = hidden;
        map.fields[index].fset = fset;
    }
    for &(index, kind, value) in &outcome.extended_updates {
        match kind {
            b'C' => map.fields[index].color = Some(value),
            b'H' => map.fields[index].highlight = Some(value),
            _ => unreachable!(),
        }
    }
    for (index, occurrence, value) in fields {
        if map.fields[index].occurs == 1 {
            map.fields[index].initial = value;
        } else {
            map.fields[index].occurrence_values[occurrence] = Some(value);
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn customer_edit_attribute_unlocks_dataonly_field() {
        let mut map = crate::bms::parse(include_str!(
            "../../../reference/Bank-of-Z-main/src/base/cics/bms/BNK1DCM.bms"
        )).unwrap().remove(0);
        let index = map.fields.iter().position(|field| {
            field.name.as_deref() == Some("CUSTFNAM")
        }).unwrap();
        assert!(map.fields[index].protected);
        let output = b"CCO6\0\0\0\x01CUSTFNAMA\x02\x00ODCS\0\0\0\0";
        let (_, outcome) = response(&map, output).unwrap();
        assert_eq!(outcome.changed_attributes, vec![index]);
        for (field, protected, hidden, fset) in outcome.attribute_updates {
            map.fields[field].protected = protected;
            map.fields[field].hidden = hidden;
            map.fields[field].fset = fset;
        }
        assert!(!map.fields[index].protected);
        let record = crate::tn3270::render_dataonly(&map, &[], &[index], false).unwrap();
        assert!(record.windows(2).any(|bytes| bytes == [0x1d, 0x41]));
    }

    #[test]
    fn cobol_color_and_highlight_update_dataonly_field() {
        let mut map = crate::bms::parse(include_str!(
            "../../../reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms"
        )).unwrap().remove(0);
        let index = map.fields.iter().position(|field|
            field.name.as_deref() == Some("MESSAGE")).unwrap();
        let mut output = b"CCO7\0\0\0\0\0\x02".to_vec();
        output.extend_from_slice(b"MESSAGE C\xf2");
        output.extend_from_slice(b"MESSAGE H\xf4");
        output.extend_from_slice(b"\x02\0OMEN\0\0\0\0");
        let (_, outcome) = response(&map, &output).unwrap();
        assert_eq!(outcome.changed_attributes, vec![index]);
        for (field, kind, value) in outcome.extended_updates {
            match kind {
                b'C' => map.fields[field].color = Some(value),
                b'H' => map.fields[field].highlight = Some(value),
                _ => unreachable!(),
            }
        }
        let record = crate::tn3270::render_dataonly_for_terminal(
            &map, &[], &[index], false, HostCodec::Cp037,
            crate::tn3270::TerminalModel::from_name("IBM-3279-2-E").unwrap()).unwrap();
        assert!(record.windows(8).any(|part|
            part == [0x29, 3, 0xc0, 0xf8, 0x42, 0xf2, 0x41, 0xf4]));
    }

    #[test]
    fn pipe_request_and_response_are_length_delimited() {
        let map = crate::bms::parse(include_str!(
            "../../../reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms"
        ))
        .unwrap()
        .remove(0);
        let input = Input {
            aid: 0x7d,
            cursor: None,
            fields: vec![("ACTION".to_owned(), crate::codec::encode("1").unwrap())],
        };
        let session = Session {
            transid: *b"OMEN",
            commarea: b"Y".to_vec(),
        };
        let bytes = request(&input, &session, b"BANKAPP ").unwrap();
        assert_eq!(&bytes[..19], b"CCI3\x7dOMENBANKAPP \x00\x01");
        assert_eq!(&bytes[19..20], b"Y");
        assert_eq!(&bytes[22..30], b"ACTION  ");
        let mut output = b"CCO5\0\x01MESSAGE ".to_vec();
        output.extend_from_slice(&5u16.to_be_bytes());
        output.extend_from_slice(b"Hello");
        output.extend_from_slice(b"\x00\x00OMEN\x00\x01Y\x00\x00");
        let (fields, outcome) = response(&map, &output).unwrap();
        assert_eq!(fields[0].2, "Hello");
        assert_eq!(outcome.next.unwrap(), session);
        assert_eq!(outcome.screen, ScreenAction::Map);
        assert_eq!(outcome.changed_fields.len(), 1);
        output.push(0);
        assert!(response(&map, &output).unwrap_err().contains("trailing"));
        output.truncate(output.len() - 1);
        output[6..14].copy_from_slice(b"UNKNOWN ");
        assert!(response(&map, &output)
            .unwrap_err()
            .contains("unknown field"));
    }

    #[test]
    fn japanese_worker_pipe_uses_utf8_and_bms_byte_capacity() {
        let codec = HostCodec::Japanese(1399);
        let map = crate::bms::parse("JPMAP DFHMSD TYPE=MAP\nJPSCR DFHMDI SIZE=(24,80)\nNAME DFHMDF POS=(2,1),LENGTH=8,ATTRB=UNPROT\n")
            .unwrap().remove(0);
        let input = Input { aid: 0x7d, cursor: None,
            fields: vec![("NAME".to_owned(), codec.encode("日本語").unwrap())] };
        let request = request_with_codec(&input, &Session::default(), b"COBCICS ", codec).unwrap();
        assert!(request.windows("日本語".len()).any(|part| part == "日本語".as_bytes()));
        let mut output = b"CCO5\0\x01NAME    ".to_vec();
        output.extend_from_slice(&("日本語".len() as u16).to_be_bytes());
        output.extend_from_slice("日本語".as_bytes());
        output.extend_from_slice(b"\0\0    \0\0\0\0");
        assert_eq!(response_with_codec(&map, &output, codec).unwrap().0[0].2, "日本語");
        assert!(response_with_codec(&map, &output, HostCodec::Cp037).is_err());
    }

    #[test]
    fn repeated_field_response_preserves_each_occurrence() {
        let mut map = crate::bms::parse(
            "M DFHMSD TYPE=MAP\nP DFHMDI SIZE=(24,80)\nACCOUNT DFHMDF POS=(9,1),LENGTH=79,OCCURS=10,ATTRB=PROT\n",
        )
        .unwrap()
        .remove(0);
        let mut output = b"CCO5\0\x02".to_vec();
        for (name, value) in [
            (b"ACCO0001", b"One".as_slice()),
            (b"ACCO0010", b"Ten".as_slice()),
        ] {
            output.extend_from_slice(name);
            output.extend_from_slice(&(value.len() as u16).to_be_bytes());
            output.extend_from_slice(value);
        }
        output.extend_from_slice(b"\x00\x00    \x00\x00\x00\x00");
        let (fields, outcome) = response(&map, &output).unwrap();
        assert_eq!(outcome.changed_fields, vec![0]);
        for (index, occurrence, value) in fields {
            map.fields[index].occurrence_values[occurrence] = Some(value);
        }
        assert_eq!(map.fields[0].value_at(0), "One");
        assert_eq!(map.fields[0].value_at(9), "Ten");
        assert_eq!(map.fields[0].value_at(1), "");
        assert!(crate::tn3270::render_map(&map).is_ok());
    }

    #[test]
    fn return_without_transid_ends_session_and_rejects_invalid_metadata() {
        let map = crate::bms::parse(include_str!(
            "../../../reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms"
        ))
        .unwrap()
        .remove(0);
        let mut output = b"CCO5\0\0\x01\x00    \0\0\0\0".to_vec();
        assert_eq!(
            response(&map, &output).unwrap(),
            (
                vec![],
                Outcome {
                    next: None,
                    immediate: false,
                    screen: ScreenAction::Erase,
                    changed_fields: vec![],
                    changed_attributes: vec![],
                    attribute_updates: vec![],
                    extended_updates: vec![],
                    text: None,
                }
            )
        );
        output.push(b'X');
        assert!(response(&map, &output).unwrap_err().contains("trailing"));
        output = b"CCO5\0\0\x01\x00    \0\x01X\0\0".to_vec();
        assert!(response(&map, &output)
            .unwrap_err()
            .contains("without TRANSID"));
        output[8..12].copy_from_slice(b"O EN");
        assert!(response(&map, &output)
            .unwrap_err()
            .contains("invalid TRANSID"));
        output[6] = 6;
        assert!(response(&map, &output)
            .unwrap_err()
            .contains("invalid screen action"));
    }

    #[test]
    fn immediate_requires_next_transid_and_allows_unchanged_screen() {
        let map = crate::bms::parse(include_str!(
            "../../../reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms"
        ))
        .unwrap()
        .remove(0);
        let mut output = b"CCO5\0\0\x04\x01ODCS\0\0\0\0".to_vec();
        let (_, outcome) = response(&map, &output).unwrap();
        assert_eq!(outcome.screen, ScreenAction::None);
        assert!(outcome.immediate);
        assert_eq!(outcome.next.unwrap().transid, *b"ODCS");
        output[8..12].copy_from_slice(b"    ");
        assert!(response(&map, &output)
            .unwrap_err()
            .contains("IMMEDIATE without TRANSID"));
        output[7] = 2;
        assert!(response(&map, &output)
            .unwrap_err()
            .contains("invalid IMMEDIATE flag"));
    }

    #[test]
    fn text_response_is_separate_from_map_fields() {
        let map = crate::bms::parse(include_str!(
            "../../../reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms"
        ))
        .unwrap()
        .remove(0);
        let mut output = b"CCO5\0\0\x05\x00    \0\0\0\rSession Ended".to_vec();
        let (_, outcome) = response(&map, &output).unwrap();
        assert_eq!(outcome.screen, ScreenAction::Text);
        assert_eq!(outcome.text.as_deref(), Some("Session Ended"));
        output[6] = 0;
        assert!(response(&map, &output)
            .unwrap_err()
            .contains("without SEND TEXT"));
    }

    #[test]
    fn transid_is_validated_and_space_padded() {
        assert_eq!(transid("OMEN").unwrap(), *b"OMEN");
        assert_eq!(transid("A1").unwrap(), *b"A1  ");
        for invalid in ["", "ABCDE", "oMen", "A B", "A-"] {
            assert!(transid(invalid).is_err());
        }
    }

    #[test]
    fn applid_is_validated_and_space_padded() {
        assert_eq!(applid("BANKAPP").unwrap(), *b"BANKAPP ");
        for invalid in ["", "TOOLONGID", "bankapp", "BANK APP"] {
            assert!(applid(invalid).is_err());
        }
    }
}
