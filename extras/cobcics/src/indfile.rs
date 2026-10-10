//! Host side of the IND$FILE DFT (structured field) transfer protocol.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const WSF: u8 = 0xf3;
const AID_SF: u8 = 0x88;
const CHUNK: usize = 2048;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction { Get, Put }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase { Query, Open, DataAck, GetData, CloseAck, MessageOpenAck, MessageAck, Done }

pub struct Transfer {
    phase: Phase,
    direction: Direction,
    file: Option<File>,
    path: PathBuf,
    temporary: Option<PathBuf>,
    record: u32,
}

fn fields(record: &[u8]) -> Result<Vec<&[u8]>, String> {
    if record.first() != Some(&AID_SF) {
        return Err("IND$FILE expected a structured-field AID".to_owned());
    }
    let mut tail = &record[1..];
    let mut result = Vec::new();
    while !tail.is_empty() {
        if tail.len() < 3 {
            return Err("truncated IND$FILE structured field".to_owned());
        }
        let length = u16::from_be_bytes([tail[0], tail[1]]) as usize;
        if length < 3 || length > tail.len() {
            return Err("invalid IND$FILE structured-field length".to_owned());
        }
        result.push(&tail[..length]);
        tail = &tail[length..];
    }
    Ok(result)
}

fn wsf(parts: &[&[u8]]) -> Vec<u8> {
    let mut result = vec![WSF];
    for part in parts { result.extend_from_slice(part); }
    result
}

fn open(direction: Direction, message: bool) -> Vec<u8> {
    let mut result = vec![0x00, 0x23, 0xd0, 0x00, 0x12, 0x01, 0x06, 0x01, 0x01, 0x04, 0x03];
    result.extend_from_slice(if direction == Direction::Put {
        &[0x0a, 0x0a, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00]
    } else {
        &[0x0a, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x11, 0x01, 0x01, 0x00]
    });
    result.extend_from_slice(&[0x50, 0x05, 0x52, 0x03, 0xf0, 0x03, 0x09]);
    result.extend_from_slice(if message { b"FT:MSG " } else { b"FT:DATA" });
    debug_assert_eq!(result.len(), 0x23);
    result
}

fn data_insert(data: &[u8]) -> Result<Vec<u8>, String> {
    let size = u16::try_from(data.len() + 10).map_err(|_| "DFT data too large".to_owned())?;
    let data_length = u16::try_from(data.len() + 5).map_err(|_| "DFT data too large".to_owned())?;
    let mut result = Vec::with_capacity(size as usize);
    result.extend_from_slice(&size.to_be_bytes());
    result.extend_from_slice(&[0xd0, 0x47, 0x04, 0xc0, 0x80, 0x61]);
    result.extend_from_slice(&data_length.to_be_bytes());
    result.extend_from_slice(data);
    Ok(result)
}

fn command_parts(command: &str, root: &Path) -> Result<(Direction, PathBuf), String> {
    let mut words = command.trim().split_whitespace();
    if !words.next().is_some_and(|value| value.eq_ignore_ascii_case("IND$FILE")) {
        return Err("expected IND$FILE command".to_owned());
    }
    let direction = match words.next().map(str::to_ascii_uppercase).as_deref() {
        Some("GET") => Direction::Get,
        Some("PUT") => Direction::Put,
        _ => return Err("IND$FILE requires GET or PUT".to_owned()),
    };
    let name = words.next().ok_or("IND$FILE requires a host file name")?;
    let name = name.trim_matches('\'');
    if name.is_empty() || name == "." || name == ".." || name.len() > 255
        || !name.bytes().all(|byte| byte.is_ascii_alphanumeric()
            || matches!(byte, b'.' | b'_' | b'-' | b'(' | b')')) {
        return Err("IND$FILE host file name must be a simple data set name".to_owned());
    }
    Ok((direction, root.join(name.to_ascii_uppercase())))
}

impl Transfer {
    pub fn start(command: &str, root: &Path) -> Result<(Self, Vec<u8>), String> {
        let (direction, path) = command_parts(command, root)?;
        let (file, temporary) = if direction == Direction::Get {
            let canonical_root = root.canonicalize()
                .map_err(|err| format!("{}: {err}", root.display()))?;
            let canonical_path = path.canonicalize()
                .map_err(|err| format!("{}: {err}", path.display()))?;
            if !canonical_path.starts_with(canonical_root) {
                return Err("IND$FILE host file escapes its configured root".to_owned());
            }
            (File::open(&path).map_err(|err| format!("{}: {err}", path.display()))?, None)
        } else {
            fs::create_dir_all(root).map_err(|err| format!("{}: {err}", root.display()))?;
            let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let temporary = path.with_extension(format!("indfile-{}-{serial}.part", std::process::id()));
            (File::options().write(true).create_new(true).open(&temporary)
                .map_err(|err| format!("{}: {err}", temporary.display()))?,
             Some(temporary))
        };
        Ok((Self { phase: Phase::Query, direction, file: Some(file), path, temporary, record: 1 },
            vec![WSF, 0x00, 0x05, 0x01, 0xff, 0x02]))
    }

    pub fn done(&self) -> bool { self.phase == Phase::Done }

    fn send_next(&mut self) -> Result<Vec<u8>, String> {
        let mut data = vec![0_u8; CHUNK];
        let size = self.file.as_mut().unwrap().read(&mut data)
            .map_err(|err| format!("read transfer file: {err}"))?;
        if size == 0 {
            self.phase = Phase::CloseAck;
            return Ok(wsf(&[&[0x00, 0x05, 0xd0, 0x41, 0x12]]));
        }
        self.phase = Phase::DataAck;
        data.truncate(size);
        let payload = data_insert(&data)?;
        Ok(wsf(&[&[0x00, 0x0a, 0xd0, 0x47, 0x11, 0x01, 0x05, 0x00, 0x80, 0x00], &payload]))
    }

    fn get_next(&mut self) -> Vec<u8> {
        self.phase = Phase::GetData;
        wsf(&[
            &[0x00, 0x0f, 0xd0, 0x45, 0x11, 0x01, 0x05, 0x00, 0x06, 0x00,
              0x09, 0x05, 0x01, 0x03, 0x00],
            &[0x00, 0x09, 0xd0, 0x46, 0x11, 0x01, 0x04, 0x00, 0x80],
        ])
    }

    pub fn receive(&mut self, record: &[u8]) -> Result<Vec<u8>, String> {
        let fields = fields(record)?;
        if fields.is_empty() { return Err("empty IND$FILE response".to_owned()); }
        match self.phase {
            Phase::Query => {
                if !fields.iter().any(|field| field.len() >= 4
                    && field[2] == 0x81 && field[3] == 0x95) {
                    return Err("terminal does not support DFT IND$FILE".to_owned());
                }
                self.phase = Phase::Open;
                Ok(wsf(&[&open(self.direction, false)]))
            }
            Phase::Open | Phase::MessageOpenAck => {
                if !fields.iter().any(|field| field.len() == 5
                    && field[2..] == [0xd0, 0x00, 0x09]) {
                    return Err("IND$FILE expected Open acknowledgment".to_owned());
                }
                if self.phase == Phase::MessageOpenAck {
                    self.phase = Phase::MessageAck;
                    Ok(wsf(&[&data_insert(b"TRANS03")?]))
                } else if self.direction == Direction::Get {
                    self.send_next()
                } else {
                    Ok(self.get_next())
                }
            }
            Phase::DataAck | Phase::MessageAck => {
                if !fields.iter().any(|field| field.len() == 11
                    && field[2..5] == [0xd0, 0x47, 0x05]
                    && field[5..7] == [0x63, 0x06]
                    && u32::from_be_bytes(field[7..11].try_into().unwrap()) == self.record) {
                    return Err("IND$FILE expected matching data acknowledgment".to_owned());
                }
                self.record += 1;
                if self.phase == Phase::MessageAck {
                    self.phase = Phase::Done;
                    Ok(Vec::new())
                } else {
                    self.send_next()
                }
            }
            Phase::GetData => {
                if fields.iter().any(|field| field.len() == 9
                    && field[2..] == [0xd0, 0x46, 0x08, 0x69, 0x04, 0x22, 0x00]) {
                    self.phase = Phase::CloseAck;
                    return Ok(wsf(&[&[0x00, 0x05, 0xd0, 0x41, 0x12]]));
                }
                let field = fields.iter().find(|field| field.len() >= 16
                    && field[2..5] == [0xd0, 0x46, 0x05])
                    .ok_or("IND$FILE expected upload data")?;
                if field[5..7] != [0x63, 0x06]
                    || u32::from_be_bytes(field[7..11].try_into().unwrap()) != self.record
                    || field[11..14] != [0xc0, 0x80, 0x61] {
                    return Err("IND$FILE upload record header mismatch".to_owned());
                }
                let length = u16::from_be_bytes([field[14], field[15]]) as usize;
                if length < 5 || length - 5 != field.len() - 16 {
                    return Err("IND$FILE upload data length mismatch".to_owned());
                }
                self.file.as_mut().unwrap().write_all(&field[16..])
                    .map_err(|err| format!("write transfer file: {err}"))?;
                self.record += 1;
                Ok(self.get_next())
            }
            Phase::CloseAck => {
                if !fields.iter().any(|field| field.len() == 5
                    && field[2..] == [0xd0, 0x41, 0x09]) {
                    return Err("IND$FILE expected Close acknowledgment".to_owned());
                }
                self.file.as_mut().unwrap().flush()
                    .map_err(|err| format!("flush transfer file: {err}"))?;
                self.file.take();
                if let Some(temporary) = self.temporary.as_ref() {
                    fs::rename(&temporary, &self.path)
                        .map_err(|err| format!("commit transfer file: {err}"))?;
                    self.temporary = None;
                }
                self.phase = Phase::MessageOpenAck;
                self.record = 1;
                Ok(wsf(&[&open(Direction::Get, true)]))
            }
            Phase::Done => Err("IND$FILE transfer already complete".to_owned()),
        }
    }
}

impl Drop for Transfer {
    fn drop(&mut self) {
        self.file.take();
        if let Some(path) = self.temporary.take() {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root() -> PathBuf {
        std::env::temp_dir().join(format!("cobcics-indfile-test-{}-{}",
            std::process::id(), NEXT_TEMP.fetch_add(1, Ordering::Relaxed)))
    }

    fn answer(field: &[u8]) -> Vec<u8> {
        [&[AID_SF][..], field].concat()
    }

    fn upload_data(number: u32, bytes: &[u8]) -> Vec<u8> {
        let mut field = Vec::new();
        field.extend_from_slice(&u16::try_from(bytes.len() + 16).unwrap().to_be_bytes());
        field.extend_from_slice(&[0xd0, 0x46, 0x05, 0x63, 0x06]);
        field.extend_from_slice(&number.to_be_bytes());
        field.extend_from_slice(&[0xc0, 0x80, 0x61]);
        field.extend_from_slice(&u16::try_from(bytes.len() + 5).unwrap().to_be_bytes());
        field.extend_from_slice(bytes);
        answer(&field)
    }

    #[test]
    fn parses_command_and_rejects_path_escape() {
        let root = Path::new("storage");
        assert_eq!(command_parts("IND$FILE GET 'BANK.DATA' (BINARY", root).unwrap(),
            (Direction::Get, root.join("BANK.DATA")));
        assert!(command_parts("IND$FILE PUT ../secret", root).is_err());
    }

    #[test]
    fn dft_field_lengths_and_query_reply() {
        assert_eq!(open(Direction::Get, false).len(), 0x23);
        assert_eq!(open(Direction::Put, false).len(), 0x23);
        assert_eq!(data_insert(b"ABC").unwrap().len(), 13);
        assert!(fields(&[0x88, 0x00, 0x04, 0x81, 0x95]).is_ok());
        assert!(fields(&[0x88, 0x00, 0x05, 0x81, 0x95]).is_err());
    }

    #[test]
    fn abandoned_upload_removes_partial_file() {
        let root = test_root();
        let (transfer, query) = Transfer::start("IND$FILE PUT NEWDATA (BINARY", &root).unwrap();
        assert_eq!(query, [0xf3, 0, 5, 1, 0xff, 2]);
        let temporary = transfer.temporary.clone().unwrap();
        assert!(temporary.exists());
        drop(transfer);
        assert!(!temporary.exists());
        assert!(!root.join("NEWDATA").exists());
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn interrupted_upload_discards_data_and_rejects_repeated_record() {
        let root = test_root();
        let (mut transfer, _) = Transfer::start("IND$FILE PUT NEWDATA", &root).unwrap();
        let temporary = transfer.temporary.clone().unwrap();
        transfer.receive(&answer(&[0, 4, 0x81, 0x95])).unwrap();
        transfer.receive(&answer(&[0, 5, 0xd0, 0, 9])).unwrap();
        transfer.receive(&upload_data(1, b"partial")).unwrap();
        assert_eq!(fs::read(&temporary).unwrap(), b"partial");
        assert!(transfer.receive(&upload_data(1, b"duplicate")).is_err());
        let mut wrong_length = upload_data(2, b"bad");
        wrong_length[16] = 0;
        assert!(transfer.receive(&wrong_length).is_err());
        drop(transfer);
        assert!(!temporary.exists());
        assert!(!root.join("NEWDATA").exists());
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn download_rejects_wrong_ack_and_missing_file() {
        let root = test_root();
        fs::create_dir(&root).unwrap();
        assert!(Transfer::start("IND$FILE GET ABSENT", &root).is_err());
        fs::write(root.join("DATA"), b"original").unwrap();
        let (mut transfer, _) = Transfer::start("IND$FILE GET DATA", &root).unwrap();
        assert!(transfer.receive(&answer(&[0, 4, 0x81, 0x94])).is_err());
        transfer.receive(&answer(&[0, 4, 0x81, 0x95])).unwrap();
        transfer.receive(&answer(&[0, 5, 0xd0, 0, 9])).unwrap();
        assert!(transfer.receive(&answer(&[0, 11, 0xd0, 0x47, 0x05,
            0x63, 0x06, 0, 0, 0, 2])).is_err());
        assert_eq!(fs::read(root.join("DATA")).unwrap(), b"original");
        drop(transfer);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn simultaneous_uploads_have_distinct_partial_files() {
        let root = test_root();
        let (first, _) = Transfer::start("IND$FILE PUT SAME", &root).unwrap();
        let (second, _) = Transfer::start("IND$FILE PUT SAME", &root).unwrap();
        let first_path = first.temporary.clone().unwrap();
        let second_path = second.temporary.clone().unwrap();
        assert_ne!(first_path, second_path);
        assert!(first_path.exists() && second_path.exists());
        drop(first);
        drop(second);
        assert!(!first_path.exists() && !second_path.exists());
        fs::remove_dir(root).unwrap();
    }
}
