use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::bms::Map;
use crate::codec::HostCodec;
use crate::tn3270::{self, Event, Input, Telnet, TerminalModel};

pub trait ConnectionIo: Read + Write {
    fn set_timeout(&mut self, timeout: Option<Duration>) -> std::io::Result<()>;
}

impl ConnectionIo for TcpStream {
    fn set_timeout(&mut self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_read_timeout(timeout)
    }
}

impl ConnectionIo for rustls::StreamOwned<rustls::ServerConnection, TcpStream> {
    fn set_timeout(&mut self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.get_mut().set_read_timeout(timeout)
    }
}

#[derive(Clone)]
pub struct WorkerRouting {
    pub codec: HostCodec,
    pub tn3270e: bool,
    pub nvt: bool,
    pub printer_queue: Option<[u8; 8]>,
    pub printer_utf8: bool,
    pub indfile_root: Option<PathBuf>,
    pub initial_transid: [u8; 4],
    pub applid: [u8; 8],
    pub default_worker: Option<PathBuf>,
    pub routes: HashMap<[u8; 4], PathBuf>,
    pub maps: HashMap<[u8; 4], Map>,
}

impl Default for WorkerRouting {
    fn default() -> Self {
        Self {
            codec: HostCodec::Cp037,
            tn3270e: false,
            nvt: false,
            printer_queue: None,
            printer_utf8: false,
            indfile_root: None,
            initial_transid: *b"    ",
            applid: *b"COBCICS ",
            default_worker: None,
            routes: HashMap::new(),
            maps: HashMap::new(),
        }
    }
}

impl WorkerRouting {
    pub(crate) fn worker_for(&self, transid: &[u8; 4]) -> Result<Option<&Path>, String> {
        if let Some(path) = self.routes.get(transid) {
            return Ok(Some(path));
        }
        if let Some(path) = self.default_worker.as_deref() {
            return Ok(Some(path));
        }
        if self.routes.is_empty() {
            return Ok(None);
        }
        Err(format!(
            "no worker for TRANSID {}",
            String::from_utf8_lossy(transid)
        ))
    }
}

fn display_message(map: &mut Map, message: &str) {
    if let Some(field) = map
        .fields
        .iter_mut()
        .find(|field| field.name.as_deref() == Some("MESSAGE"))
    {
        field.initial = message.chars().take(field.length).collect();
    }
}

fn aid_message(input: &Input, codec: HostCodec) -> Result<String, String> {
    match input.aid {
        0x7d => {
            let action = input.field_text_with_codec("ACTION", codec)?.unwrap_or_default();
            Ok(format!("Enter: {}", action.trim()))
        }
        0x6d => Ok("Clear".to_owned()),
        0xf3 => Ok("PF3".to_owned()),
        0x7c => Ok("PF12".to_owned()),
        other => Ok(format!("AID {other:02X}")),
    }
}

fn frame_screen(record: &[u8], enhanced: bool) -> Vec<u8> {
    if enhanced {
        let mut data = vec![0, 0, 0, 0, 0];
        data.extend_from_slice(record);
        tn3270::frame_record(&data)
    } else {
        tn3270::frame_record(record)
    }
}

fn send_map<S: Read + Write>(stream: &mut S, map: &Map, codec: HostCodec, terminal: TerminalModel, enhanced: bool) -> Result<(), String> {
    let record = tn3270::render_map_for_terminal(map, codec, terminal)?;
    stream
        .write_all(&frame_screen(&record, enhanced))
        .map_err(|err| format!("send map: {err}"))
}

fn send_screen<S: Read + Write>(
    stream: &mut S,
    map: &Map,
    action: crate::worker::ScreenAction,
    changed_fields: &[usize],
    changed_attributes: &[usize],
    text: Option<&str>,
    codec: HostCodec,
    terminal: TerminalModel,
    enhanced: bool,
) -> Result<(), String> {
    match action {
        crate::worker::ScreenAction::None => Ok(()),
        crate::worker::ScreenAction::Map => send_map(stream, map, codec, terminal, enhanced),
        crate::worker::ScreenAction::Erase => stream
            .write_all(&frame_screen(&[
                if map.rows == 24 && map.columns == 80 { 0xf5 } else { 0x7e },
                0xc2,
            ], enhanced))
            .map_err(|err| format!("erase screen: {err}")),
        crate::worker::ScreenAction::DataOnly | crate::worker::ScreenAction::DataOnlyAlarm => {
            let record = tn3270::render_dataonly_for_terminal(
                map,
                changed_fields,
                changed_attributes,
                action == crate::worker::ScreenAction::DataOnlyAlarm,
                codec,
                terminal,
            )?;
            stream
                .write_all(&frame_screen(&record, enhanced))
                .map_err(|err| format!("send data-only map: {err}"))
        }
        crate::worker::ScreenAction::Text => {
            let record = tn3270::render_text_with_codec(
                text.ok_or("SEND TEXT has no text")?, codec)?;
            stream
                .write_all(&frame_screen(&record, enhanced))
                .map_err(|err| format!("send text: {err}"))
        }
    }
}

const MAX_IMMEDIATE_CHAIN: usize = 32;

fn run_worker_chain<S: Read + Write>(
    stream: &mut S,
    map: &mut Map,
    routing: &WorkerRouting,
    session: &mut crate::worker::Session,
    mut input: Input,
    terminal: TerminalModel,
    enhanced: bool,
) -> Result<bool, String> {
    for _ in 0..MAX_IMMEDIATE_CHAIN {
        if let Some(next_map) = routing.maps.get(&session.transid) {
            if map.mapset != next_map.mapset || map.name != next_map.name {
                *map = next_map.clone();
            }
        }
        let path = routing
            .worker_for(&session.transid)?
            .ok_or_else(|| "RETURN IMMEDIATE has no worker for its TRANSID".to_owned())?;
        let outcome = crate::worker::run_with_codec(
            path, map, &input, session, &routing.applid, routing.codec)?;
        send_screen(
            stream,
            map,
            outcome.screen,
            &outcome.changed_fields,
            &outcome.changed_attributes,
            outcome.text.as_deref(),
            routing.codec,
            terminal,
            enhanced,
        )?;
        let Some(next) = outcome.next else {
            return Ok(true);
        };
        *session = next;
        if !outcome.immediate {
            return Ok(false);
        }
        input = Input {
            aid: 0,
            cursor: None,
            fields: Vec::new(),
        };
    }
    Err(format!(
        "RETURN IMMEDIATE exceeded {MAX_IMMEDIATE_CHAIN} consecutive tasks"
    ))
}

/// A transport demonstration. The COBOL/CICS dispatcher will replace aid_message.
pub fn serve_connection(mut stream: TcpStream, mut map: Map) -> Result<(), String> {
    serve_connection_with_worker(&mut stream, &mut map, None)
}

pub fn serve_connection_with_worker(
    stream: &mut TcpStream,
    map: &mut Map,
    worker: Option<&Path>,
) -> Result<(), String> {
    let routing = WorkerRouting {
        default_worker: worker.map(Path::to_path_buf),
        ..WorkerRouting::default()
    };
    serve_connection_with_routing(stream, map, &routing)
}

pub fn serve_connection_with_routing(
    stream: &mut TcpStream,
    map: &mut Map,
    routing: &WorkerRouting,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|err| err.to_string())?;
    serve_connection_with_io(stream, map, routing)
}

/// Serve an established plaintext or TLS transport. The caller configures
/// the underlying socket's read timeout before entering this loop.
pub fn serve_connection_with_io<S: ConnectionIo>(
    stream: &mut S,
    map: &mut Map,
    routing: &WorkerRouting,
) -> Result<(), String> {
    let greeting = if routing.nvt {
        b"COBCICS NVT gateway\r\nType 3270 to open the terminal, HELP, or QUIT.\r\n> ".to_vec()
    } else if routing.tn3270e {
        Telnet::enhanced_greeting()
    } else {
        Telnet::greeting().to_vec()
    };
    stream
        .write_all(&greeting)
        .map_err(|err| format!("send Telnet greeting: {err}"))?;
    let mut telnet = if routing.nvt { Telnet::nvt() } else { Telnet::default() };
    let mut session = crate::worker::Session {
        transid: routing.initial_transid,
        ..crate::worker::Session::default()
    };
    let mut shown = false;
    let mut terminal = None;
    let mut printer = None;
    let mut transfer: Option<crate::indfile::Transfer> = None;
    let mut query_pending = false;
    let mut query_done = false;
    let mut buffer = [0u8; 4096];
    loop {
        if let Some(queue) = printer.as_mut() {
            let queue: &mut crate::printer::PrinterQueue = queue;
            let batch = queue.next_batch()?;
            if !batch.is_empty() {
                for record in batch {
                    let record = if routing.printer_utf8 {
                        crate::printer::encode_utf8_record(&record, routing.codec)?
                    } else {
                        record
                    };
                    stream.write_all(&telnet.frame_printer_data(&record)?)
                        .map_err(|err| format!("send 3287 print data: {err}"))?;
                }
                stream.write_all(&telnet.frame_print_eoj()?)
                    .map_err(|err| format!("send PRINT-EOJ: {err}"))?;
                queue.commit()?;
            }
        }
        let size = match stream.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(size) => size,
            Err(err) if printer.is_some() && matches!(err.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => continue,
            Err(err) => return Err(format!("read terminal: {err}")),
        };
        let events = telnet.feed(&buffer[..size])?;
        for event in events {
            match event {
                Event::Reply(reply) => stream
                    .write_all(&reply)
                    .map_err(|err| format!("send Telnet reply: {err}"))?,
                Event::TerminalType(name) => terminal = Some(TerminalModel::from_name(&name)?),
                Event::Printer(_) => (),
                Event::NvtLine(line) => {
                    match line.trim().to_ascii_uppercase().as_str() {
                        "3270" => {
                            telnet.begin_3270();
                            let greeting = if routing.tn3270e {
                                Telnet::enhanced_greeting()
                            } else {
                                Telnet::greeting().to_vec()
                            };
                            stream.write_all(&greeting)
                                .map_err(|err| format!("start 3270: {err}"))?;
                        }
                        "QUIT" | "EXIT" => return Ok(()),
                        "HELP" => stream.write_all(b"Commands: 3270, HELP, QUIT\r\n> ")
                            .map_err(|err| format!("NVT reply: {err}"))?,
                        _ => stream.write_all(b"Unknown command. Type HELP.\r\n> ")
                            .map_err(|err| format!("NVT reply: {err}"))?,
                    }
                }
                Event::Record(record) => {
                    if printer.is_some() {
                        // Basic SCS sessions do not request response messages.
                        continue;
                    }
                    let record = telnet.decode_3270(&record)?;
                    if query_pending {
                        let (rows, columns) = tn3270::query_usable_area(record)?;
                        let model = terminal.as_mut().ok_or("terminal model not negotiated")?;
                        model.rows = rows;
                        model.columns = columns;
                        query_pending = false;
                        query_done = true;
                        continue;
                    }
                    if !shown {
                        return Err("terminal data arrived before TN3270 negotiation".to_owned());
                    }
                    if let Some(active) = transfer.as_mut() {
                        let response = active.receive(record)?;
                        if !response.is_empty() {
                            stream.write_all(&telnet.frame_3270(&response))
                                .map_err(|err| format!("send IND$FILE: {err}"))?;
                        }
                        if active.done() {
                            transfer = None;
                            stream.set_timeout(None).map_err(|err| err.to_string())?;
                            let model = terminal.ok_or("3270 terminal model not negotiated")?;
                            send_map(stream, map, routing.codec, model, telnet.enhanced())?;
                        }
                        continue;
                    }
                    let input = tn3270::parse_input_with_codec(map, record, routing.codec)?;
                    if let Some(root) = routing.indfile_root.as_deref() {
                        let command = input.fields.iter()
                            .map(|(_, bytes)| routing.codec.decode(bytes))
                            .collect::<Result<Vec<_>, _>>()?
                            .into_iter()
                            .find(|value| value.trim_start().to_ascii_uppercase().starts_with("IND$FILE "));
                        if let Some(command) = command {
                            let (active, query) = crate::indfile::Transfer::start(&command, root)?;
                            stream.write_all(&telnet.frame_3270(&query))
                                .map_err(|err| format!("send IND$FILE query: {err}"))?;
                            stream.set_timeout(Some(Duration::from_secs(60)))
                                .map_err(|err| err.to_string())?;
                            transfer = Some(active);
                            continue;
                        }
                    }
                    let model = terminal.ok_or("3270 terminal model not negotiated")?;
                    let done = match routing.worker_for(&session.transid)? {
                        Some(_) => run_worker_chain(stream, map, routing, &mut session, input, model, telnet.enhanced())?,
                        None => {
                            display_message(map, &aid_message(&input, routing.codec)?);
                            send_map(stream, map, routing.codec, model, telnet.enhanced())?;
                            matches!(input.aid, 0xf3 | 0x7c)
                        }
                    };
                    if done {
                        return Ok(());
                    }
                }
            }
        }
        if telnet.ready() && !shown && !query_pending {
            if telnet.printer() {
                let name = routing.printer_queue.ok_or(
                    "printer session requested but --printer-queue is not configured")?;
                printer = Some(crate::printer::PrinterQueue::new(&name)?);
                stream.set_timeout(Some(Duration::from_millis(250)))
                    .map_err(|err| err.to_string())?;
                shown = true;
                continue;
            }
            let model = terminal.ok_or("3270 terminal model not negotiated")?;
            if telnet.enhanced() && model.extended && !query_done {
                stream.write_all(&telnet.frame_3270(&[0xf3, 0, 5, 1, 0xff, 2]))
                    .map_err(|err| format!("query terminal usable area: {err}"))?;
                query_pending = true;
                continue;
            }
            if routing.worker_for(&session.transid)?.is_some() {
                let input = Input {
                    aid: 0,
                    cursor: None,
                    fields: Vec::new(),
                };
                if run_worker_chain(stream, map, routing, &mut session, input, model, telnet.enhanced())? {
                    return Ok(());
                }
            } else {
                send_map(stream, map, routing.codec, model, telnet.enhanced())?;
            }
            stream.set_timeout(None).map_err(|err| err.to_string())?;
            shown = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn transid_routes_select_worker_before_default() {
        let routing = WorkerRouting {
            codec: HostCodec::Cp037,
            tn3270e: false,
            nvt: false,
            printer_queue: None,
            printer_utf8: false,
            indfile_root: None,
            initial_transid: *b"OMEN",
            applid: *b"BANKAPP ",
            default_worker: Some(PathBuf::from("fallback")),
            routes: HashMap::from([(*b"OMEN", PathBuf::from("menu"))]),
            maps: HashMap::new(),
        };
        assert_eq!(
            routing.worker_for(b"OMEN").unwrap(),
            Some(Path::new("menu"))
        );
        assert_eq!(
            routing.worker_for(b"INQ1").unwrap(),
            Some(Path::new("fallback"))
        );
        let restricted = WorkerRouting {
            default_worker: None,
            ..routing
        };
        assert!(restricted.worker_for(b"INQ1").unwrap_err().contains("INQ1"));
    }

    #[test]
    fn terminal_session_displays_menu_and_accepts_action() {
        let map = crate::bms::parse(include_str!(
            "../../../reference/Bank-of-Z-main/src/base/cics/bms/BNK1MAI.bms"
        ))
        .unwrap()
        .remove(0);
        let action = map
            .fields
            .iter()
            .find(|field| field.name.as_deref() == Some("ACTION"))
            .unwrap();
        let address = tn3270::address_bytes(action.data_address(map.columns)).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address_of_server = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(stream, map)
        });
        let mut client = TcpStream::connect(address_of_server).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut greeting = [0u8; 15];
        client.read_exact(&mut greeting).unwrap();
        assert_eq!(&greeting, Telnet::greeting());
        client
            .write_all(&[
                0xff, 0xfb, 24, // WILL TERMINAL-TYPE
                0xff, 0xfd, 0, // DO BINARY
                0xff, 0xfb, 0, // WILL BINARY
                0xff, 0xfd, 25, // DO EOR
                0xff, 0xfb, 25, // WILL EOR
            ])
            .unwrap();
        let mut request_type = [0u8; 6];
        client.read_exact(&mut request_type).unwrap();
        assert_eq!(request_type, [0xff, 0xfa, 24, 1, 0xff, 0xf0]);
        client
            .write_all(&[
                0xff, 0xfa, 24, 0, b'I', b'B', b'M', b'-', b'3', b'2', b'7', b'9', b'-', b'2',
                0xff, 0xf0,
            ])
            .unwrap();
        let mut first = [0u8; 4096];
        let size = client.read(&mut first).unwrap();
        assert_eq!(&first[..2], &[0xf5, 0xc2]);
        assert_eq!(&first[size - 2..size], &[0xff, 0xef]);
        let input = [
            0x7d, address[0], address[1], 0x11, address[0], address[1], 0xf1, 0xff, 0xef,
        ];
        client.write_all(&input).unwrap();
        let size = client.read(&mut first).unwrap();
        assert!(first[..size]
            .windows(6)
            .any(|bytes| bytes == crate::codec::encode("Enter:").unwrap()));
        client.write_all(&[0xf3, 0x40, 0x40, 0xff, 0xef]).unwrap();
        assert!(server.join().unwrap().is_ok());
    }
}
