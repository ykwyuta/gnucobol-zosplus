//! TN3270 transport and formatted 3270 display data stream.

use crate::{bms::{Field, Map}, codec::HostCodec};

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
    if address > 0x3fff {
        return Err(format!("3270 address exceeds 14 bits: {address}"));
    }
    if address > 0xfff {
        return Ok([(address >> 8) as u8, address as u8]);
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

/// Dimensions from the Usable Area Query Reply (QCODE 0x81).
pub fn query_usable_area(record: &[u8]) -> Result<(usize, usize), String> {
    if record.first() != Some(&0x88) {
        return Err("expected a 3270 structured-field AID".to_owned());
    }
    let mut tail = &record[1..];
    while !tail.is_empty() {
        if tail.len() < 4 {
            return Err("truncated 3270 Query Reply".to_owned());
        }
        let length = u16::from_be_bytes([tail[0], tail[1]]) as usize;
        if length < 4 || length > tail.len() {
            return Err("invalid 3270 Query Reply length".to_owned());
        }
        let field = &tail[..length];
        if field[2..4] == [0x81, 0x81] {
            if field.len() < 10 {
                return Err("short 3270 Usable Area Query Reply".to_owned());
            }
            let columns = u16::from_be_bytes([field[6], field[7]]) as usize;
            let rows = u16::from_be_bytes([field[8], field[9]]) as usize;
            if columns == 0 || rows == 0 || columns * rows > 16383 {
                return Err("invalid 3270 usable area dimensions".to_owned());
            }
            return Ok((rows, columns));
        }
        tail = &tail[length..];
    }
    Err("3270 Query Reply lacks Usable Area".to_owned())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalModel {
    pub rows: usize,
    pub columns: usize,
    pub extended: bool,
}

impl TerminalModel {
    pub fn from_name(name: &str) -> Result<Self, String> {
        let name = name.to_ascii_uppercase();
        if name == "IBM-DYNAMIC" {
            // The actual usable area arrives in a Query Reply.
            return Ok(Self { rows: 24, columns: 80, extended: true });
        }
        let (base, suffix) = name.strip_prefix("IBM-")
            .and_then(|name| name.split_once('-'))
            .ok_or_else(|| format!("unsupported 3270 terminal model: {name}"))?;
        if !matches!(base, "3278" | "3279") {
            return Err(format!("unsupported 3270 terminal model: {name}"));
        }
        let (rows, columns) = match suffix {
            "2" | "2-E" => (24, 80),
            "3" | "3-E" => (32, 80),
            "4" | "4-E" => (43, 80),
            "5" | "5-E" => (27, 132),
            _ => return Err(format!("unsupported 3270 terminal model: {name}")),
        };
        Ok(Self { rows, columns, extended: base == "3279" || suffix.ends_with("-E") })
    }

    pub fn supports(&self, map: &Map) -> bool {
        (map.rows == 24 && map.columns == 80)
            || (map.rows == self.rows && map.columns == self.columns)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Reply(Vec<u8>),
    Record(Vec<u8>),
    TerminalType(String),
    Printer(String),
    NvtLine(String),
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
    tn3270e: bool,
    tn3270e_ready: bool,
    tn3270e_functions_pending: bool,
    printer: bool,
    printer_stream: Option<u8>,
    nvt: bool,
    nvt_upgraded: bool,
    nvt_line: Vec<u8>,
    nvt_last_cr: bool,
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
    pub fn nvt() -> Self {
        Self { nvt: true, ..Self::default() }
    }

    pub fn begin_3270(&mut self) {
        self.nvt = false;
        self.nvt_upgraded = true;
        self.nvt_line.clear();
    }

    pub fn enhanced_greeting() -> Vec<u8> {
        let mut greeting = vec![IAC, DO, TN3270E];
        greeting.extend_from_slice(Self::greeting());
        greeting
    }

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
        if self.nvt { return false; }
        (self.tn3270e && self.tn3270e_ready && self.terminal_type.is_some())
            || (!self.tn3270e && self.binary_from_client
            && self.binary_to_client
            && self.eor_from_client
            && self.eor_to_client
            && self.terminal_type.is_some())
    }

    pub fn enhanced(&self) -> bool {
        self.tn3270e && self.tn3270e_ready
    }

    pub fn printer(&self) -> bool {
        self.enhanced() && self.printer
    }

    pub fn frame_scs(&self, record: &[u8]) -> Result<Vec<u8>, String> {
        if !self.printer() || self.printer_stream != Some(3) {
            return Err("SCS output requires a TN3270E printer".to_owned());
        }
        let mut data = vec![1, 0, 0, 0, 0];
        data.extend_from_slice(record);
        Ok(frame_record(&data))
    }

    pub fn frame_printer_data(&self, record: &[u8]) -> Result<Vec<u8>, String> {
        if !self.printer() {
            return Err("printer output requires a TN3270E printer".to_owned());
        }
        if self.printer_stream == Some(3) {
            return self.frame_scs(record);
        }
        let mut data = vec![0, 0, 0, 0, 0, 0xf1, 0xc2];
        data.extend_from_slice(record);
        Ok(frame_record(&data))
    }

    pub fn frame_print_eoj(&self) -> Result<Vec<u8>, String> {
        if !self.printer() {
            return Err("PRINT-EOJ requires a TN3270E printer".to_owned());
        }
        Ok(frame_record(&[8, 0, 0, 0, 0]))
    }

    pub fn frame_3270(&self, record: &[u8]) -> Vec<u8> {
        if !self.enhanced() { return frame_record(record); }
        let mut data = vec![0, 0, 0, 0, 0];
        data.extend_from_slice(record);
        frame_record(&data)
    }

    pub fn decode_3270<'a>(&self, record: &'a [u8]) -> Result<&'a [u8], String> {
        if !self.enhanced() { return Ok(record); }
        if record.len() < 5 || record[0] != 0 || record[1..5] != [0, 0, 0, 0] {
            return Err("unsupported TN3270E data header".to_owned());
        }
        Ok(&record[5..])
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
                    } else if self.nvt {
                        if byte == b'\r' || byte == b'\n' {
                            if !(byte == b'\n' && self.nvt_last_cr) {
                                let line = String::from_utf8(std::mem::take(&mut self.nvt_line))
                                    .map_err(|_| "NVT command is not UTF-8".to_owned())?;
                                events.push(Event::NvtLine(line));
                            }
                            self.nvt_last_cr = byte == b'\r';
                        } else {
                            self.nvt_last_cr = false;
                            self.nvt_line.push(byte);
                        }
                        State::Data
                    } else {
                        self.record.push(byte);
                        State::Data
                    }
                }
                State::Iac => match byte {
                    IAC => {
                        if self.nvt { self.nvt_line.push(IAC); }
                        else { self.record.push(IAC); }
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
            if self.record.len() > 65536 || self.subnegotiation.len() > 256
                || self.nvt_line.len() > 4096 {
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
            (WILL, TN3270E) => {
                self.tn3270e = true;
                events.push(Event::Reply(vec![IAC, SB, TN3270E, 8, 2, IAC, SE]));
            }
            (WONT, TN3270E) => {
                self.tn3270e = false;
                self.tn3270e_ready = false;
            }
            (DO, TN3270E) | (DO, TERMINAL_TYPE) => {
                events.push(Event::Reply(vec![IAC, WONT, option]));
            }
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
        if self.subnegotiation.first() == Some(&TN3270E) && self.tn3270e {
            return self.finish_tn3270e(events);
        }
        if self.subnegotiation.first() != Some(&TERMINAL_TYPE)
            || self.subnegotiation.get(1) != Some(&0)
        {
            return Ok(());
        }
        let mut terminal = std::str::from_utf8(&self.subnegotiation[2..])
            .map_err(|_| "invalid terminal type".to_owned())?
            .to_ascii_uppercase();
        // wc3270 -nvt advertises xterm even after a host-initiated switch to
        // 3270 mode. Its 3270 screen then uses the default Model 2 geometry.
        if self.nvt_upgraded && terminal == "XTERM" {
            terminal = "IBM-3279-2-E".to_owned();
        }
        if terminal == "IBM-3287-1" && !self.tn3270e {
            return Err("3287 printer requires TN3270E".to_owned());
        }
        if terminal != "IBM-3287-1" {
            TerminalModel::from_name(&terminal)?;
        }
        self.terminal_type = Some(terminal.clone());
        if terminal == "IBM-3287-1" {
            self.printer = true;
            events.push(Event::Printer(terminal));
        } else {
            events.push(Event::TerminalType(terminal));
        }
        Ok(())
    }

    fn finish_tn3270e(&mut self, events: &mut Vec<Event>) -> Result<(), String> {
        let data = &self.subnegotiation;
        if data.starts_with(&[TN3270E, 2, 7]) {
            let tail = &data[3..];
            let split = tail.iter().position(|byte| *byte == 1 || *byte == 0)
                .unwrap_or(tail.len());
            let terminal = std::str::from_utf8(&tail[..split])
                .map_err(|_| "invalid TN3270E device type".to_owned())?
                .to_ascii_uppercase();
            let printer = terminal == "IBM-3287-1";
            if !printer && TerminalModel::from_name(&terminal).is_err() {
                events.push(Event::Reply(vec![IAC, SB, TN3270E, 2, 6, 5, 4, IAC, SE]));
                return Ok(());
            }
            let lu = if split == tail.len() {
                if printer { b"COBCICSP".as_slice() } else { b"COBCICS1".as_slice() }
            } else if tail[split] == 1 || (printer && tail[split] == 0) {
                &tail[split + 1..]
            } else {
                events.push(Event::Reply(vec![IAC, SB, TN3270E, 2, 6, 5, 2, IAC, SE]));
                return Ok(());
            };
            if lu.is_empty() || lu.len() > 8 || !lu.iter().all(u8::is_ascii_alphanumeric) {
                events.push(Event::Reply(vec![IAC, SB, TN3270E, 2, 6, 5, 3, IAC, SE]));
                return Ok(());
            }
            let mut reply = vec![IAC, SB, TN3270E, 2, 4];
            reply.extend_from_slice(terminal.as_bytes());
            reply.push(1);
            reply.extend_from_slice(lu);
            reply.extend_from_slice(&[IAC, SE]);
            events.push(Event::Reply(reply));
            self.terminal_type = Some(terminal.clone());
            self.printer = printer;
            if printer {
                events.push(Event::Printer(terminal));
            } else {
                events.push(Event::TerminalType(terminal));
            }
        } else if data.starts_with(&[TN3270E, 3, 7]) {
            if self.printer {
                // Prefer LU1 SCS, but accept LU3 3270 data-stream control.
                let stream = if data[3..].contains(&3) { 3 }
                    else if data[3..].contains(&1) { 1 }
                    else { 0 };
                if stream != 0 {
                    self.printer_stream = Some(stream);
                    events.push(Event::Reply(vec![IAC, SB, TN3270E, 3, 4, stream, IAC, SE]));
                    self.tn3270e_ready = true;
                } else {
                    events.push(Event::Reply(vec![IAC, SB, TN3270E, 3, 7, 3, IAC, SE]));
                    self.tn3270e_functions_pending = true;
                }
            } else if data.len() == 3 {
                events.push(Event::Reply(vec![IAC, SB, TN3270E, 3, 4, IAC, SE]));
                self.tn3270e_ready = true;
                self.tn3270e_functions_pending = false;
            } else {
                events.push(Event::Reply(vec![IAC, SB, TN3270E, 3, 7, IAC, SE]));
                self.tn3270e_functions_pending = true;
            }
        } else if data.as_slice() == [TN3270E, 3, 4] && self.tn3270e_functions_pending {
            self.tn3270e_ready = true;
            self.tn3270e_functions_pending = false;
        } else if data.as_slice() == [TN3270E, 3, 4, 3] && self.printer
            && self.tn3270e_functions_pending {
            self.tn3270e_ready = true;
            self.tn3270e_functions_pending = false;
            self.printer_stream = Some(3);
        } else if data.as_slice() == [TN3270E, 3, 4, 1] && self.printer
            && self.tn3270e_functions_pending {
            self.tn3270e_ready = true;
            self.tn3270e_functions_pending = false;
            self.printer_stream = Some(1);
        }
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
    render_map_with_codec(map, HostCodec::Cp037)
}

pub fn render_map_with_codec(map: &Map, codec: HostCodec) -> Result<Vec<u8>, String> {
    render_map_for_terminal(map, codec, TerminalModel { rows: 24, columns: 80, extended: false })
}

fn field_attribute(field: &Field) -> u8 {
    let mut attribute = if field.protected { 0xf0 } else if field.numeric { 0x50 } else { 0x40 };
    if field.intensified { attribute |= 0x08; }
    if field.hidden { attribute |= 0x0c; }
    if field.fset { attribute |= 0x01; }
    attribute
}

fn append_field_start(record: &mut Vec<u8>, field: &Field, codec: HostCodec, extended: bool) {
    let attribute = field_attribute(field);
    let input_control = matches!(codec, HostCodec::Japanese(_)) && !field.protected;
    if extended && (field.color.is_some() || field.highlight.is_some() || input_control) {
        let count = 1 + usize::from(field.color.is_some())
            + usize::from(field.highlight.is_some()) + usize::from(input_control);
        record.extend_from_slice(&[0x29, count as u8, 0xc0, attribute]);
        if let Some(color) = field.color { record.extend_from_slice(&[0x42, color]); }
        if let Some(highlight) = field.highlight { record.extend_from_slice(&[0x41, highlight]); }
        if input_control { record.extend_from_slice(&[0xfe, 0x01]); }
    } else if input_control {
        record.extend_from_slice(&[0x29, 0x02, 0xc0, attribute, 0xfe, 0x01]);
    } else {
        record.extend_from_slice(&[0x1d, attribute]);
    }
}

pub fn render_map_for_terminal(
    map: &Map, codec: HostCodec, terminal: TerminalModel,
) -> Result<Vec<u8>, String> {
    if !terminal.supports(map) {
        return Err(format!("BMS map {}x{} exceeds terminal {}x{}",
            map.rows, map.columns, terminal.rows, terminal.columns));
    }
    // Erase/Write Alternate selects the negotiated model's larger buffer.
    let command = if map.rows == 24 && map.columns == 80 { 0xf5 } else { 0x7e };
    let mut record = vec![command, 0xc2]; // Erase and restore keyboard.
    for field in &map.fields {
        for index in 0..field.occurs {
            let value = field.value_at(index);
            let encoded = codec.encode(value)?;
            if encoded.len() > field.length {
                return Err(format!("BMS field {} exceeds {} EBCDIC bytes",
                    field.name.as_deref().unwrap_or("<unnamed>"), field.length));
            }
            record.push(0x11); // SBA
            record.extend_from_slice(&address_bytes(
                field.attribute_address_at(map.columns, index),
            )?);
            append_field_start(&mut record, field, codec, terminal.extended);
            if field.justify_right {
                record.resize(record.len() + field.length - encoded.len(), 0x40);
            }
            record.extend_from_slice(&encoded);
            if !field.justify_right {
                record.resize(record.len() + field.length - encoded.len(), 0);
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
    render_text_with_codec(text, HostCodec::Cp037)
}

pub fn render_text_with_codec(text: &str, codec: HostCodec) -> Result<Vec<u8>, String> {
    if text.chars().any(char::is_control) {
        return Err("SEND TEXT contains a control character".to_owned());
    }
    let encoded = codec.encode(text)?;
    if encoded.len() > 1919 {
        return Err("SEND TEXT exceeds 1919 EBCDIC bytes".to_owned());
    }
    let mut record = vec![0xf5, 0xc2, 0x1d, 0xf0];
    record.extend_from_slice(&encoded);
    Ok(record)
}

pub fn render_dataonly(
    map: &Map,
    changed_fields: &[usize],
    changed_attributes: &[usize],
    alarm: bool,
) -> Result<Vec<u8>, String> {
    render_dataonly_with_codec(map, changed_fields, changed_attributes, alarm,
                               HostCodec::Cp037)
}

pub fn render_dataonly_with_codec(
    map: &Map,
    changed_fields: &[usize],
    changed_attributes: &[usize],
    alarm: bool,
    codec: HostCodec,
) -> Result<Vec<u8>, String> {
    render_dataonly_for_terminal(map, changed_fields, changed_attributes, alarm, codec,
        TerminalModel { rows: 24, columns: 80, extended: false })
}

pub fn render_dataonly_for_terminal(
    map: &Map,
    changed_fields: &[usize],
    changed_attributes: &[usize],
    alarm: bool,
    codec: HostCodec,
    terminal: TerminalModel,
) -> Result<Vec<u8>, String> {
    if !terminal.supports(map) {
        return Err(format!("BMS map {}x{} exceeds terminal {}x{}",
            map.rows, map.columns, terminal.rows, terminal.columns));
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
            append_field_start(&mut record, field, codec, terminal.extended);
        }
    }
    for &index in changed_fields {
        let field = map
            .fields
            .get(index)
            .ok_or_else(|| "data-only field index is out of range".to_owned())?;
        for occurrence in 0..field.occurs {
            let value = field.value_at(occurrence);
            let encoded = codec.encode(value)?;
            if encoded.len() > field.length {
                return Err(format!("BMS field {} exceeds {} EBCDIC bytes",
                    field.name.as_deref().unwrap_or("<unnamed>"), field.length));
            }
            record.push(0x11); // SBA to the data cell, without field attributes.
            record.extend_from_slice(&address_bytes(
                field.attribute_address_at(map.columns, occurrence) + 1,
            )?);
            if field.justify_right {
                record.resize(record.len() + field.length - encoded.len(), 0x40);
            }
            record.extend_from_slice(&encoded);
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
        self.field_text_with_codec(name, HostCodec::Cp037)
    }

    pub fn field_text_with_codec(&self, name: &str, codec: HostCodec) -> Result<Option<String>, String> {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, bytes)| codec.decode(bytes))
            .transpose()
    }
}

pub fn parse_input(map: &Map, record: &[u8]) -> Result<Input, String> {
    parse_input_with_codec(map, record, HostCodec::Cp037)
}

pub fn parse_input_with_codec(map: &Map, record: &[u8], codec: HostCodec) -> Result<Input, String> {
    let aid = *record
        .first()
        .ok_or_else(|| "empty 3270 input".to_owned())?;
    if matches!(aid, 0x6d | 0x6c | 0x6e | 0x6b) {
        if record.len() != 1 && record.len() != 3 {
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
        // Read Modified All may include an SF before the field data.
        if record.get(index) == Some(&0x1d) {
            if index + 2 > record.len() {
                return Err("truncated inbound field attribute".to_owned());
            }
            index += 2;
        }
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
            let supplied = codec.decode(&record[index..end])?;
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
        // Reject malformed SO/SI and unmappable terminal bytes at the input
        // boundary, including fields which are not right-justified.
        let decoded = codec.decode(&data)?;
        if field.justify_right {
            let text = decoded.trim();
            let encoded = codec.encode(text)?;
            if encoded.len() > field.length {
                return Err("justified input exceeds BMS field".to_owned());
            }
            let fill = if field.justify_zero { codec.encode("0")?[0] } else { 0x40 };
            let mut justified = vec![fill; field.length - encoded.len()];
            justified.extend_from_slice(&encoded);
            data = justified;
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
    use crate::{bms, codec};

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
        let source = include_str!("../tests/fixtures/ssmap.bms");
        let maps = bms::parse(source).unwrap();
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
    fn japanese_map_and_modified_field_use_mixed_ebcdic() {
        let codec = HostCodec::parse("IBM-1399").unwrap();
        let map = bms::parse("JPMAP DFHMSD TYPE=MAP\nJPSCR DFHMDI SIZE=(24,80)\nNAME DFHMDF POS=(2,1),LENGTH=12,ATTRB=UNPROT,INITIAL='日本語'\n")
            .unwrap().remove(0);
        let encoded = codec.encode("日本語").unwrap();
        let screen = render_map_with_codec(&map, codec).unwrap();
        assert!(screen.windows(encoded.len()).any(|part| part == encoded));
        assert!(screen.windows(6).any(|part| part == [0x29, 0x02, 0xc0, 0x40, 0xfe, 0x01]));
        let field = &map.fields[0];
        let address = address_bytes(field.data_address(map.columns)).unwrap();
        let mut record = vec![0x7d, 0x40, 0x40, 0x11, address[0], address[1]];
        record.extend_from_slice(&codec.encode("A日本語B").unwrap());
        let input = parse_input_with_codec(&map, &record, codec).unwrap();
        assert_eq!(input.field_text_with_codec("NAME", codec).unwrap().as_deref(),
                   Some("A日本語B"));
        record.pop();
        record.pop(); // omit SI and the trailing SBCS B
        assert!(parse_input_with_codec(&map, &record, codec).is_err());
        assert!(render_map(&map).is_err()); // CP037 cannot represent the text.
        let mut boundary = map.clone();
        boundary.fields[0].length = encoded.len();
        assert!(render_map_with_codec(&boundary, codec).is_ok());
        let mut exact_input = vec![0x7d, 0x40, 0x40, 0x11, address[0], address[1]];
        exact_input.extend_from_slice(&encoded);
        assert!(parse_input_with_codec(&boundary, &exact_input, codec).is_ok());
        exact_input.extend_from_slice(&codec.encode("A").unwrap());
        assert!(parse_input_with_codec(&boundary, &exact_input, codec).is_err());
        boundary.fields[0].length -= 1;
        assert!(render_map_with_codec(&boundary, codec).is_err());
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

    #[test]
    fn terminal_models_and_extended_bms_attributes() {
        for (name, rows, columns) in [
            ("IBM-3278-2", 24, 80), ("IBM-3279-3-E", 32, 80),
            ("IBM-3279-4-E", 43, 80), ("IBM-3279-5-E", 27, 132),
        ] {
            let model = TerminalModel::from_name(name).unwrap();
            assert_eq!((model.rows, model.columns), (rows, columns));
        }
        let source = "M DFHMSD TYPE=MAP\nP DFHMDI SIZE=(27,132)\n\
N DFHMDF POS=(27,120),LENGTH=10,ATTRB=(NUM,IC),COLOR=GREEN,*\n\
  HILIGHT=UNDERLINE,INITIAL='123'\n";
        let map = bms::parse(source).unwrap().remove(0);
        let model = TerminalModel::from_name("IBM-3279-5-E").unwrap();
        let screen = render_map_for_terminal(&map, HostCodec::Cp037, model).unwrap();
        assert_eq!(screen[0], 0x7e); // alternate-size Erase/Write
        assert!(screen.windows(8).any(|part| part ==
            [0x29, 3, 0xc0, 0x50, 0x42, 0xf4, 0x41, 0xf4]));
        assert!(screen.windows(3).any(|part| part ==
            [0x11, address_bytes(26 * 132 + 119).unwrap()[0],
             address_bytes(26 * 132 + 119).unwrap()[1]]));
        assert!(render_map_for_terminal(&map, HostCodec::Cp037,
            TerminalModel::from_name("IBM-3279-4-E").unwrap()).is_err());
        assert_eq!(address_bytes(5000).unwrap(), [0x13, 0x88]);
        assert_eq!(parse_address(&[0x13, 0x88]).unwrap(), 5000);
    }

    #[test]
    fn tn3270e_basic_device_and_function_negotiation() {
        let mut telnet = Telnet::default();
        assert_eq!(telnet.feed(&[IAC, WILL, TN3270E]).unwrap(),
            vec![Event::Reply(vec![IAC, SB, TN3270E, 8, 2, IAC, SE])]);
        let device = b"IBM-3279-4-E";
        let mut request = vec![IAC, SB, TN3270E, 2, 7];
        request.extend_from_slice(device);
        request.extend_from_slice(&[IAC, SE]);
        let events = telnet.feed(&request).unwrap();
        assert!(events.contains(&Event::TerminalType("IBM-3279-4-E".to_owned())));
        assert!(events.iter().any(|event| matches!(event, Event::Reply(reply)
            if reply.starts_with(&[IAC, SB, TN3270E, 2, 4]))));
        assert_eq!(telnet.feed(&[IAC, SB, TN3270E, 3, 7, 2, IAC, SE]).unwrap(),
            vec![Event::Reply(vec![IAC, SB, TN3270E, 3, 7, IAC, SE])]);
        assert!(!telnet.ready());
        telnet.feed(&[IAC, SB, TN3270E, 3, 4, IAC, SE]).unwrap();
        assert!(telnet.ready());
        assert_eq!(telnet.frame_3270(&[0xf5, 0xc2]),
            vec![0, 0, 0, 0, 0, 0xf5, 0xc2, IAC, EOR]);
        assert_eq!(telnet.decode_3270(&[0, 0, 0, 0, 0, 0x7d]).unwrap(), &[0x7d]);
    }

    #[test]
    fn tn3270e_printer_scs_negotiation_and_records() {
        let mut telnet = Telnet::default();
        telnet.feed(&[IAC, WILL, TN3270E]).unwrap();
        let mut request = vec![IAC, SB, TN3270E, 2, 7];
        request.extend_from_slice(b"IBM-3287-1");
        request.extend_from_slice(&[0, b'C', b'O', b'B', b'C', b'I', b'C', b'S', b'1', IAC, SE]);
        let events = telnet.feed(&request).unwrap();
        assert!(events.contains(&Event::Printer("IBM-3287-1".to_owned())));
        assert!(events.iter().any(|event| matches!(event, Event::Reply(reply)
            if reply.starts_with(&[IAC, SB, TN3270E, 2, 4]))));
        assert_eq!(telnet.feed(&[IAC, SB, TN3270E, 3, 7, 3, IAC, SE]).unwrap(),
            vec![Event::Reply(vec![IAC, SB, TN3270E, 3, 4, 3, IAC, SE])]);
        assert!(telnet.printer());
        assert_eq!(telnet.frame_scs(&[0xc1, 0xff]).unwrap(),
            [vec![1, 0, 0, 0, 0, 0xc1, 0xff, 0xff], vec![IAC, EOR]].concat());
        assert_eq!(telnet.frame_print_eoj().unwrap(), vec![8, 0, 0, 0, 0, IAC, EOR]);
    }

    #[test]
    fn nvt_line_mode_then_3270_upgrade() {
        let mut telnet = Telnet::nvt();
        assert_eq!(telnet.feed(b"HELP\r\n3270\n").unwrap(),
            vec![Event::NvtLine("HELP".to_owned()), Event::NvtLine("3270".to_owned())]);
        assert!(!telnet.ready());
        telnet.begin_3270();
        assert_eq!(telnet.feed(&[0x7d, IAC, EOR]).unwrap(),
            vec![Event::Record(vec![0x7d])]);
        let mut type_reply = vec![IAC, SB, TERMINAL_TYPE, 0];
        type_reply.extend_from_slice(b"xterm");
        type_reply.extend_from_slice(&[IAC, SE]);
        assert_eq!(telnet.feed(&type_reply).unwrap(),
            vec![Event::TerminalType("IBM-3279-2-E".to_owned())]);
        assert_eq!(telnet.terminal_type(), Some("IBM-3279-2-E"));
    }

    #[test]
    fn usable_area_reply_reports_extended_geometry() {
        assert_eq!(query_usable_area(&[0x88, 0, 10, 0x81, 0x81, 1, 0, 0, 100, 0, 50]).unwrap(),
            (50, 100));
        assert!(query_usable_area(&[0x88, 0, 10, 0x81, 0x81, 1, 0, 0, 100, 0, 0]).is_err());
    }

    #[test]
    fn tn3270e_printer_lu3_fallback() {
        let mut telnet = Telnet::default();
        telnet.feed(&[IAC, WILL, TN3270E]).unwrap();
        let mut request = vec![IAC, SB, TN3270E, 2, 7];
        request.extend_from_slice(b"IBM-3287-1");
        request.extend_from_slice(&[IAC, SE]);
        telnet.feed(&request).unwrap();
        assert_eq!(telnet.feed(&[IAC, SB, TN3270E, 3, 7, 1, IAC, SE]).unwrap(),
            vec![Event::Reply(vec![IAC, SB, TN3270E, 3, 4, 1, IAC, SE])]);
        assert_eq!(telnet.frame_printer_data(&[0xc1, 0xc2]).unwrap(),
            vec![0, 0, 0, 0, 0, 0xf1, 0xc2, 0xc1, 0xc2, IAC, EOR]);
    }

    #[test]
    fn read_modified_all_with_field_attribute_and_short_aid() {
        let map = menu();
        let action = map.fields.iter().find(|field| field.name.as_deref() == Some("ACTION")).unwrap();
        let address = address_bytes(action.data_address(map.columns)).unwrap();
        let record = [0x7d, 0x40, 0x40, 0x11, address[0], address[1], 0x1d, 0x40, 0xf1];
        assert_eq!(parse_input(&map, &record).unwrap().field_text("ACTION").unwrap(),
            Some("1".to_owned()));
        assert_eq!(parse_input(&map, &[0x6c, 0x40, 0x40]).unwrap().aid, 0x6c);
    }
}
