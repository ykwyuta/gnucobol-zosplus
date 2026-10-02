//! Minimal, strict TN3270 (RFC 1576) transport and 3270 model-2 data stream.

use crate::{bms::Map, codec};

const IAC: u8 = 0xff;
const DO: u8 = 0xfd;
const DONT: u8 = 0xfe;
const WILL: u8 = 0xfb;
const WONT: u8 = 0xfc;
const SB: u8 = 0xfa;
const SE: u8 = 0xf0;
const EOR: u8 = 0xef;
const BINARY: u8 = 0;
const TERMINAL_TYPE: u8 = 24;
const TELNET_EOR: u8 = 25;
const TN3270E: u8 = 40;

const ADDRESS_CODE: [u8; 64] = [
    0x40, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e, 0x4f,
    0x50, 0xd1, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0x5a, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f,
    0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f,
    0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0x7a, 0x7b, 0x7c, 0x7d, 0x7e, 0x7f,
];

pub fn address_bytes(address: usize) -> Result<[u8; 2], String> {
    if address > 0xfff {
        return Err(format!("3270 address exceeds 12 bits: {address}"));
    }
    Ok([ADDRESS_CODE[address >> 6], ADDRESS_CODE[address & 0x3f]])
}

pub fn parse_address(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() != 2 {
        return Err("3270 address requires two bytes".to_owned());
    }
    if bytes[0] & 0xc0 == 0 {
        return Ok((usize::from(bytes[0]) << 8) | usize::from(bytes[1]));
    }
    // In a 12-bit address only the low six bits carry position data.  Real
    // terminals can use either 01xxxxxx or 11xxxxxx for each encoded byte;
    // the canonical table above is only needed when writing addresses.
    let decode = |byte: u8| match byte & 0xc0 {
        0x40 | 0xc0 => Some(usize::from(byte & 0x3f)),
        _ => None,
    };
    let high = decode(bytes[0]).ok_or_else(|| "invalid 3270 address code".to_owned())?;
    let low = decode(bytes[1]).ok_or_else(|| "invalid 3270 address code".to_owned())?;
    Ok((high << 6) | low)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Reply(Vec<u8>),
    Record(Vec<u8>),
    TerminalType(String),
}

#[derive(Default)]
pub struct Telnet {
    state: State,
    record: Vec<u8>,
    subnegotiation: Vec<u8>,
    terminal_type_requested: bool,
    terminal_type: Option<String>,
    binary_from_client: bool,
    binary_to_client: bool,
    eor_from_client: bool,
    eor_to_client: bool,
}

#[derive(Default)]
enum State {
    #[default]
    Data,
    Iac,
    Negotiation(u8),
    Subnegotiation,
    SubnegotiationIac,
}

impl Telnet {
    pub fn greeting() -> &'static [u8] {
        &[
            IAC,
            DO,
            TERMINAL_TYPE,
            IAC,
            WILL,
            BINARY,
            IAC,
            DO,
            BINARY,
            IAC,
            WILL,
            TELNET_EOR,
            IAC,
            DO,
            TELNET_EOR,
        ]
    }

    pub fn ready(&self) -> bool {
        self.binary_from_client
            && self.binary_to_client
            && self.eor_from_client
            && self.eor_to_client
            && self.terminal_type.is_some()
    }

    pub fn terminal_type(&self) -> Option<&str> {
        self.terminal_type.as_deref()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Event>, String> {
        let mut events = Vec::new();
        for &byte in bytes {
            self.state = match std::mem::take(&mut self.state) {
                State::Data => {
                    if byte == IAC {
                        State::Iac
                    } else {
                        self.record.push(byte);
                        State::Data
                    }
                }
                State::Iac => match byte {
                    IAC => {
                        self.record.push(IAC);
                        State::Data
                    }
                    EOR => {
                        events.push(Event::Record(std::mem::take(&mut self.record)));
                        State::Data
                    }
                    DO | DONT | WILL | WONT => State::Negotiation(byte),
                    SB => {
                        self.subnegotiation.clear();
                        State::Subnegotiation
                    }
                    _ => State::Data,
                },
                State::Negotiation(command) => {
                    self.negotiate(command, byte, &mut events);
                    State::Data
                }
                State::Subnegotiation => {
                    if byte == IAC {
                        State::SubnegotiationIac
                    } else {
                        self.subnegotiation.push(byte);
                        State::Subnegotiation
                    }
                }
                State::SubnegotiationIac => match byte {
                    IAC => {
                        self.subnegotiation.push(IAC);
                        State::Subnegotiation
                    }
                    SE => {
                        self.finish_subnegotiation(&mut events)?;
                        State::Data
                    }
                    _ => return Err("invalid Telnet subnegotiation".to_owned()),
                },
            };
            if self.record.len() > 65536 || self.subnegotiation.len() > 256 {
                return Err("Telnet record exceeds limit".to_owned());
            }
        }
        Ok(events)
    }

    fn negotiate(&mut self, command: u8, option: u8, events: &mut Vec<Event>) {
        match (command, option) {
            (DO, BINARY) => self.binary_to_client = true,
            (WILL, BINARY) => self.binary_from_client = true,
            (DO, TELNET_EOR) => self.eor_to_client = true,
            (WILL, TELNET_EOR) => self.eor_from_client = true,
            (WILL, TERMINAL_TYPE) if !self.terminal_type_requested => {
                self.terminal_type_requested = true;
                events.push(Event::Reply(vec![IAC, SB, TERMINAL_TYPE, 1, IAC, SE]));
            }
            (DO, TN3270E) | (DO, TERMINAL_TYPE) => {
                events.push(Event::Reply(vec![IAC, WONT, option]));
            }
            (WILL, TN3270E) => events.push(Event::Reply(vec![IAC, DONT, option])),
            (DO, _) => events.push(Event::Reply(vec![IAC, WONT, option])),
            (WILL, _) => events.push(Event::Reply(vec![IAC, DONT, option])),
            (DONT, BINARY) => self.binary_to_client = false,
            (WONT, BINARY) => self.binary_from_client = false,
            (DONT, TELNET_EOR) => self.eor_to_client = false,
            (WONT, TELNET_EOR) => self.eor_from_client = false,
            _ => {}
        }
    }

    fn finish_subnegotiation(&mut self, events: &mut Vec<Event>) -> Result<(), String> {
        if self.subnegotiation.first() != Some(&TERMINAL_TYPE)
            || self.subnegotiation.get(1) != Some(&0)
        {
            return Ok(());
        }
        let terminal = std::str::from_utf8(&self.subnegotiation[2..])
            .map_err(|_| "invalid terminal type".to_owned())?
            .to_ascii_uppercase();
        if !matches!(terminal.as_str(), "IBM-3278-2" | "IBM-3279-2") {
            return Err(format!("unsupported 3270 terminal model: {terminal}"));
        }
        self.terminal_type = Some(terminal.clone());
        events.push(Event::TerminalType(terminal));
        Ok(())
    }
}

pub fn frame_record(record: &[u8]) -> Vec<u8> {
    let mut framed = Vec::with_capacity(record.len() + 2);
    for &byte in record {
        framed.push(byte);
        if byte == IAC {
            framed.push(IAC);
        }
    }
    framed.extend_from_slice(&[IAC, EOR]);
    framed
}

pub fn render_map(map: &Map) -> Result<Vec<u8>, String> {
    if map.rows != 24 || map.columns != 80 {
        return Err("only 24x80 BMS maps are supported".to_owned());
    }
    let mut record = vec![0xf5, 0xc2]; // Erase/Write and restore keyboard.
    for field in &map.fields {
        for index in 0..field.occurs {
            let value = field.value_at(index);
            record.push(0x11); // SBA
            record.extend_from_slice(&address_bytes(
                field.attribute_address_at(map.columns, index),
            )?);
            record.push(0x1d); // SF
            let mut attribute = if field.protected { 0xf0 } else { 0x40 };
            if field.intensified {
                attribute |= 0x08;
            }
            if field.hidden {
                attribute |= 0x0c;
            }
            if field.fset {
                attribute |= 0x01;
            }
            record.push(attribute);
            if field.justify_right {
                record.resize(record.len() + field.length - value.len(), 0x40);
            }
            record.extend_from_slice(&codec::encode(value)?);
            if !field.justify_right {
                record.resize(record.len() + field.length - value.len(), 0);
            }
            if field.initial_cursor {
                record.push(0x11); // SBA to the first data cell.
                record.extend_from_slice(&address_bytes(
                    field.attribute_address_at(map.columns, index) + 1,
                )?);
                record.push(0x13); // IC
            }
        }
    }
    Ok(record)
}

pub fn render_text(text: &str) -> Result<Vec<u8>, String> {
    if text.len() > 1919 || !text.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        return Err("SEND TEXT requires at most 1919 printable ASCII bytes".to_owned());
    }
    let mut record = vec![0xf5, 0xc2, 0x1d, 0xf0];
    record.extend_from_slice(&codec::encode(text)?);
    Ok(record)
}

pub fn render_dataonly(
    map: &Map,
    changed_fields: &[usize],
    changed_attributes: &[usize],
    alarm: bool,
) -> Result<Vec<u8>, String> {
    if map.rows != 24 || map.columns != 80 {
        return Err("only 24x80 BMS maps are supported".to_owned());
    }
    let mut record = vec![0xf1, if alarm { 0xc6 } else { 0xc2 }];
    for &index in changed_attributes {
        let field = map.fields.get(index)
            .ok_or_else(|| "data-only attribute index is out of range".to_owned())?;
        for occurrence in 0..field.occurs {
            record.push(0x11);
            record.extend_from_slice(&address_bytes(
                field.attribute_address_at(map.columns, occurrence),
            )?);
            record.push(0x1d);
            let mut attribute = if field.protected { 0xf0 } else { 0x40 };
            if field.intensified { attribute |= 0x08; }
            if field.hidden { attribute |= 0x0c; }
            if field.fset { attribute |= 0x01; }
            record.push(attribute);
        }
    }
    for &index in changed_fields {
        let field = map
            .fields
            .get(index)
            .ok_or_else(|| "data-only field index is out of range".to_owned())?;
        for occurrence in 0..field.occurs {
            let value = field.value_at(occurrence);
            record.push(0x11); // SBA to the data cell, without field attributes.
            record.extend_from_slice(&address_bytes(
                field.attribute_address_at(map.columns, occurrence) + 1,
            )?);
            if field.justify_right {
                record.resize(record.len() + field.length - value.len(), 0x40);
            }
            record.extend_from_slice(&codec::encode(value)?);
        }
    }
    Ok(record)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Input {
    pub aid: u8,
    pub cursor: Option<usize>,
    pub fields: Vec<(String, Vec<u8>)>,
}

impl Input {
    pub fn field_text(&self, name: &str) -> Result<Option<String>, String> {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, bytes)| codec::decode(bytes))
            .transpose()
    }
}

pub fn parse_input(map: &Map, record: &[u8]) -> Result<Input, String> {
    let aid = *record
        .first()
        .ok_or_else(|| "empty 3270 input".to_owned())?;
    if matches!(aid, 0x6d | 0x6c | 0x6e | 0x6b) {
        if record.len() != 1 {
            return Err("short-read AID has unexpected data".to_owned());
        }
        return Ok(Input {
            aid,
            cursor: None,
            fields: Vec::new(),
        });
    }
    if record.len() < 3 {
        return Err("3270 input lacks cursor address".to_owned());
    }
    let cursor = parse_address(&record[1..3])?;
    if cursor >= map.rows * map.columns {
        return Err("3270 cursor exceeds map".to_owned());
    }
    let mut fields: Vec<(String, Vec<u8>)> = Vec::new();
    let mut index = 3;
    while index < record.len() {
        if record[index] != 0x11 || index + 3 > record.len() {
            return Err("3270 input requires SBA before each field".to_owned());
        }
        let address = parse_address(&record[index + 1..index + 3])?;
        index += 3;
        let end = record[index..]
            .iter()
            .position(|byte| *byte == 0x11)
            .map(|offset| index + offset)
            .unwrap_or(record.len());
        let (field, occurrence) = map
            .fields
            .iter()
            .find_map(|field| {
                (0..field.occurs)
                    .find(|&index| field.attribute_address_at(map.columns, index) + 1 == address)
                    .map(|index| (field, index))
            })
            .ok_or_else(|| format!("input address {address} is not an unprotected field"))?;
        if end - index > field.length {
            return Err(format!(
                "input exceeds field {}",
                field.name.as_deref().unwrap_or("<unnamed>")
            ));
        }
        // A protected FSET field has its MDT set in the outgoing map. CICS
        // RECEIVE MAP returns its screen value to COBOL even though the user
        // cannot edit it (Bank of Z validates protected statement dates).
        if field.protected {
            if !field.fset {
                return Err(format!(
                    "input address {address} is not an unprotected field"
                ));
            }
            let supplied = codec::decode(&record[index..end])?;
            if supplied.trim_end() != field.value_at(occurrence).trim_end() {
                return Err(format!("protected FSET field at {address} was modified"));
            }
        }
        let name = field
            .name
            .as_ref()
            .ok_or_else(|| "unnamed input field".to_owned())?;
        let name = field.wire_name(occurrence).unwrap_or_else(|| name.clone());
        if fields.iter().any(|(seen, _)| seen == &name) {
            return Err(format!("duplicate input field: {name}"));
        }
        let mut data = record[index..end].to_vec();
        if field.justify_right {
            let text = codec::decode(&data)?;
            let text = text.trim();
            let fill = if field.justify_zero { b'0' } else { b' ' };
            let mut justified = vec![fill; field.length - text.len()];
            justified.extend_from_slice(text.as_bytes());
            data = codec::encode(std::str::from_utf8(&justified)
                .expect("BMS input is decoded to ASCII"))?;
        }
        fields.push((name, data));
        index = end;
    }
    Ok(Input {
        aid,
        cursor: Some(cursor),
        fields,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bms;

    fn menu() -> Map {
        bms::parse(include_str!(
            "../../../reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms"
        ))
        .unwrap()
        .remove(0)
    }

    fn account() -> Map {
        bms::parse(include_str!(
            "../../../reference/Bank-of-Z-main/src/base/cics/bms/BNK1DAM.bms"
        ))
        .unwrap()
        .remove(0)
    }

    #[test]
    fn renders_bank_menu_and_decodes_action() {
        let map = menu();
        let screen = render_map(&map).unwrap();
        assert_eq!(&screen[..2], &[0xf5, 0xc2]);
        assert!(screen
            .windows(3)
            .any(|bytes| bytes == codec::encode("F3=").unwrap()));
        let action = map
            .fields
            .iter()
            .find(|field| field.name.as_deref() == Some("ACTION"))
            .unwrap();
        let mut record = vec![0x7d]; // Enter
        record.extend_from_slice(&address_bytes(action.data_address(map.columns)).unwrap());
        record.push(0x11);
        record.extend_from_slice(&address_bytes(action.data_address(map.columns)).unwrap());
        record.extend_from_slice(&codec::encode("1").unwrap());
        let input = parse_input(&map, &record).unwrap();
        assert_eq!(input.field_text("ACTION").unwrap().as_deref(), Some("1"));
        assert_eq!(input.aid, 0x7d);
        assert_eq!(parse_input(&map, &[0xf3, 0x40, 0x40]).unwrap().aid, 0xf3); // PF3
        assert!(parse_input(&map, &[0x6d]).unwrap().fields.is_empty()); // Clear
    }

    #[test]
    fn genapp_right_zero_input_is_padded_to_field_width() {
        let source = include_str!(
            "../../../reference/cics-genapp/base/src/ssmap.bms"
        ).replace("INITIAL='Fund Name '    '", "INITIAL='Fund Name '");
        let maps = bms::parse(&source).unwrap();
        let map = &maps[0];
        let field = map.fields.iter()
            .find(|field| field.name.as_deref() == Some("ENT1CNO"))
            .unwrap();
        let address = address_bytes(field.data_address(map.columns)).unwrap();
        let mut record = vec![0x7d, 0x40, 0x40, 0x11, address[0], address[1]];
        record.extend_from_slice(&codec::encode("42").unwrap());
        let input = parse_input(map, &record).unwrap();
        assert_eq!(input.field_text("ENT1CNO").unwrap().as_deref(),
                   Some("0000000042"));
    }

    #[test]
    fn dataonly_writes_only_changed_data_and_sets_alarm() {
        let mut map = menu();
        let index = map
            .fields
            .iter()
            .position(|field| field.name.as_deref() == Some("MESSAGE"))
            .unwrap();
        map.fields[index].initial = "ERROR".to_owned();
        let plain = render_dataonly(&map, &[index], &[], false).unwrap();
        let alarm = render_dataonly(&map, &[index], &[], true).unwrap();
        assert_eq!(&plain[..2], &[0xf1, 0xc2]);
        assert_eq!(&alarm[..2], &[0xf1, 0xc6]);
        assert_eq!(plain.len(), 2 + 3 + 5);
        assert_eq!(
            &plain[2..5],
            &[
                0x11,
                address_bytes(map.fields[index].data_address(80)).unwrap()[0],
                address_bytes(map.fields[index].data_address(80)).unwrap()[1]
            ]
        );
        assert_eq!(&plain[5..], codec::encode("ERROR").unwrap());
    }

    #[test]
    fn rejects_input_to_protected_field() {
        let map = menu();
        let protected = &map.fields[0];
        let mut record = vec![0x7d, 0x40, 0x40, 0x11];
        record.extend_from_slice(&address_bytes(protected.data_address(map.columns)).unwrap());
        record.extend_from_slice(&codec::encode("X").unwrap());
        assert!(parse_input(&map, &record)
            .unwrap_err()
            .contains("not an unprotected field"));
    }

    #[test]
    fn protected_fset_returns_unchanged_value_to_bms() {
        let map = bms::parse("M DFHMSD TYPE=MAP\nP DFHMDI SIZE=(24,80)\nDATE DFHMDF POS=(2,1),LENGTH=4,ATTRB=(PROT,FSET),INITIAL='2024'\n")
            .unwrap().remove(0);
        let address = address_bytes(map.fields[0].data_address(map.columns)).unwrap();
        let mut record = vec![0x7d, 0x40, 0x40, 0x11, address[0], address[1]];
        record.extend_from_slice(&codec::encode("2024").unwrap());
        let input = parse_input(&map, &record).unwrap();
        assert_eq!(input.field_text("DATE").unwrap().as_deref(), Some("2024"));
        record.truncate(6);
        record.extend_from_slice(&codec::encode("2025").unwrap());
        assert!(parse_input(&map, &record).unwrap_err().contains("modified"));
    }

    #[test]
    fn repeated_bms_fields_have_distinct_screen_addresses() {
        let map = bms::parse("M DFHMSD TYPE=MAP\nP DFHMDI SIZE=(24,80)\nACCOUNT DFHMDF POS=(9,1),LENGTH=79,OCCURS=10,ATTRB=UNPROT\n")
            .unwrap().remove(0);
        let screen = render_map(&map).unwrap();
        assert_eq!(screen.iter().filter(|&&byte| byte == 0x11).count(), 10);
        let address = address_bytes(map.fields[0].attribute_address_at(80, 1) + 1).unwrap();
        let record = [0x7d, 0x40, 0x40, 0x11, address[0], address[1], 0xc1];
        let input = parse_input(&map, &record).unwrap();
        assert_eq!(input.field_text("ACCO0002").unwrap().as_deref(), Some("A"));
    }

    #[test]
    fn decodes_wc3270_action_field_address() {
        // wc3270 sends row 5, column 17 as C5 50 in the 12-bit address code.
        assert_eq!(address_bytes(4 * 80 + 16).unwrap(), [0xc5, 0x50]);
        assert_eq!(parse_address(&[0xc5, 0x50]).unwrap(), 4 * 80 + 16);
        assert_eq!(parse_address(&[0xc5, 0xd0]).unwrap(), 4 * 80 + 16);
        assert_eq!(parse_address(&[0x5d, 0x7f]).unwrap(), 1919);
        let record = [
            0x7d, 0xc5, 0x50, 0x11, 0xc5, 0x50, 0xe7, 0x11, 0x5d, 0x7f, 0x40,
        ];
        let input = parse_input(&menu(), &record).unwrap();
        assert_eq!(input.field_text("ACTION").unwrap().as_deref(), Some("X"));
        assert_eq!(input.fields.len(), 2); // ACTION plus protected FSET DUMMY
    }

    #[test]
    fn decodes_wc3270_account_pf3_record() {
        // Read Modified sent by wc3270 after PF3 on the account screen.
        // Several FSET fields are empty and some addresses use 11xxxxxx.
        let record = [
            0xf3, 0xc5, 0xd1, 0x11, 0xc7, 0xf4, 0x11, 0xc9, 0xc4, 0x11, 0x4a, 0xd4, 0x11, 0x4b,
            0xe4, 0x11, 0x4c, 0xf4, 0x11, 0x4e, 0xc4, 0x11, 0x4e, 0xc9, 0x11, 0x4e, 0x4e, 0x11,
            0x4f, 0xd4, 0x11, 0x50, 0xe4, 0x11, 0x50, 0xe9, 0x11, 0x50, 0x6e, 0x11, 0xd1, 0xf4,
            0x11, 0xd1, 0xf9, 0x11, 0xd1, 0x7e, 0x11, 0xd3, 0xc3, 0x11, 0xd4, 0xd3, 0x11, 0x5d,
            0x7f, 0x40,
        ];
        let input = parse_input(&account(), &record).unwrap();
        assert_eq!(input.aid, 0xf3);
    }

    #[test]
    fn telnet_negotiation_handles_split_records_and_escaping() {
        let mut telnet = Telnet::default();
        assert_eq!(telnet.feed(&[IAC, WILL]).unwrap(), vec![]);
        assert_eq!(
            telnet.feed(&[TERMINAL_TYPE]).unwrap(),
            vec![Event::Reply(vec![IAC, SB, TERMINAL_TYPE, 1, IAC, SE])]
        );
        telnet.feed(&[IAC, DO, BINARY, IAC, WILL, BINARY]).unwrap();
        telnet
            .feed(&[IAC, DO, TELNET_EOR, IAC, WILL, TELNET_EOR])
            .unwrap();
        assert_eq!(
            telnet
                .feed(&[IAC, SB, TERMINAL_TYPE, 0, b'I', b'B'])
                .unwrap(),
            vec![]
        );
        assert_eq!(
            telnet
                .feed(&[b'M', b'-', b'3', b'2', b'7', b'9', b'-', b'2', IAC, SE])
                .unwrap(),
            vec![Event::TerminalType("IBM-3279-2".to_owned())]
        );
        assert!(telnet.ready());
        assert_eq!(telnet.feed(&[0x7d, IAC]).unwrap(), vec![]);
        assert_eq!(
            telnet.feed(&[IAC, IAC, EOR]).unwrap(),
            vec![Event::Record(vec![0x7d, IAC])]
        );
        assert_eq!(frame_record(&[0xff, 0x00]), vec![IAC, IAC, 0, IAC, EOR]);
    }
}
