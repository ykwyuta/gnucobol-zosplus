//! LU1 SCS printer output backed by the same TDQ files as the CICS bridge.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::codec::HostCodec;

static ACTIVE: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

pub fn encode_utf8_record(data: &[u8], codec: HostCodec) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(data).map_err(|_| "printer TDQ record is not UTF-8".to_owned())?;
    let mut encoded = Vec::new();
    let mut segment = String::new();
    for ch in text.chars() {
        let control = match ch {
            '\r' => Some(0x0d),
            '\n' => Some(0x15),
            '\x0c' => Some(0x0c),
            _ => None,
        };
        if let Some(control) = control {
            encoded.extend_from_slice(&codec.encode(&segment)?);
            segment.clear();
            if control == 0x15 && encoded.last() == Some(&0x0d) {
                encoded.pop();
            }
            encoded.push(control);
        } else {
            segment.push(ch);
        }
    }
    encoded.extend_from_slice(&codec.encode(&segment)?);
    Ok(encoded)
}

pub fn queue_path(name: &[u8; 8]) -> PathBuf {
    let mut hash = 14695981039346656037_u64;
    for byte in name {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(1099511628211);
    }
    let directory = std::env::var_os("COBCICS_STORAGE_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            #[cfg(windows)]
            { std::env::var_os("TEMP").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")) }
            #[cfg(not(windows))]
            { PathBuf::from("/tmp") }
        });
    directory.join(format!("cobcics-tdq-{hash:016x}.data"))
}

pub struct PrinterQueue {
    path: PathBuf,
    cursor: PathBuf,
    offset: u64,
    leased: bool,
}

impl PrinterQueue {
    pub fn new(name: &[u8; 8]) -> Result<Self, String> {
        let path = queue_path(name);
        let mut active = ACTIVE.get_or_init(|| Mutex::new(HashSet::new()))
            .lock().map_err(|_| "printer lease lock poisoned".to_owned())?;
        if !active.insert(path.clone()) {
            return Err("printer queue already has an active 3287 session".to_owned());
        }
        let cursor = path.with_extension("cursor");
        let offset = fs::read(&cursor).ok()
            .filter(|bytes| bytes.len() == 8)
            .map(|bytes| u64::from_be_bytes(bytes.try_into().unwrap()))
            .unwrap_or(0);
        Ok(Self { path, cursor, offset, leased: true })
    }

    pub fn commit(&self) -> Result<(), String> {
        fs::write(&self.cursor, self.offset.to_be_bytes())
            .map_err(|err| format!("save printer cursor {}: {err}", self.cursor.display()))
    }

    pub fn next_batch(&mut self) -> Result<Vec<Vec<u8>>, String> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(format!("printer queue {}: {err}", self.path.display())),
        };
        if file.metadata().map_err(|err| err.to_string())?.len() < self.offset {
            self.offset = 0;
        }
        file.seek(SeekFrom::Start(self.offset)).map_err(|err| err.to_string())?;
        let mut batch = Vec::new();
        while batch.len() < 16 {
            let start = self.offset;
            let mut size = [0_u8; 4];
            match file.read_exact(&mut size) {
                Ok(()) => (),
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(err) => return Err(format!("read printer queue length: {err}")),
            }
            let size = u32::from_be_bytes(size) as usize;
            if size == 0 || size > 32763 {
                return Err(format!("invalid printer queue record length at {start}: {size}"));
            }
            let mut data = vec![0_u8; size];
            match file.read_exact(&mut data) {
                Ok(()) => (),
                Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(err) => return Err(format!("read printer queue record: {err}")),
            }
            self.offset = start + 4 + size as u64;
            batch.push(data);
        }
        Ok(batch)
    }
}

impl Drop for PrinterQueue {
    fn drop(&mut self) {
        if self.leased {
            if let Some(active) = ACTIVE.get() {
                if let Ok(mut active) = active.lock() {
                    active.remove(&self.path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn record(data: &[u8]) -> Vec<u8> {
        [u32::try_from(data.len()).unwrap().to_be_bytes().as_slice(), data].concat()
    }

    #[test]
    fn reads_complete_appended_tdq_records() {
        let path = std::env::temp_dir().join(format!("cobcics-printer-test-{}", std::process::id()));
        let mut queue = PrinterQueue {
            path: path.clone(), cursor: path.with_extension("cursor"),
            offset: 0, leased: false,
        };
        let mut file = File::create(&path).unwrap();
        file.write_all(&[0, 0, 0, 3, 0xc1]).unwrap();
        file.flush().unwrap();
        assert!(queue.next_batch().unwrap().is_empty());
        file.write_all(&[0xc2, 0xc3]).unwrap();
        file.flush().unwrap();
        assert_eq!(queue.next_batch().unwrap(), vec![vec![0xc1, 0xc2, 0xc3]]);
        queue.commit().unwrap();
        assert_eq!(fs::read(&queue.cursor).unwrap(), 7_u64.to_be_bytes());
        assert!(queue.next_batch().unwrap().is_empty());
        drop(file);
        fs::remove_file(&queue.cursor).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn utf8_tdq_text_becomes_ebcdic_scs() {
        assert_eq!(encode_utf8_record(b"ABC\r\n", HostCodec::Cp037).unwrap(),
            vec![0xc1, 0xc2, 0xc3, 0x15]);
        assert!(encode_utf8_record(&[0xff], HostCodec::Cp037).is_err());
        let japanese = HostCodec::parse("IBM-939").unwrap();
        let encoded = encode_utf8_record("日本語".as_bytes(), japanese).unwrap();
        assert_eq!(japanese.decode(&encoded).unwrap(), "日本語");
    }

    #[test]
    fn reconnect_resumes_after_committed_record_and_enforces_one_lease() {
        let name = format!("{:08X}", std::process::id());
        let name: [u8; 8] = name.as_bytes().try_into().unwrap();
        let path = queue_path(&name);
        let cursor = path.with_extension("cursor");
        let mut first = PrinterQueue::new(&name).unwrap();
        assert!(PrinterQueue::new(&name).is_err());
        fs::write(&path, record(b"FIRST")).unwrap();
        assert_eq!(first.next_batch().unwrap(), vec![b"FIRST".to_vec()]);
        first.commit().unwrap();
        drop(first);

        let mut second = PrinterQueue::new(&name).unwrap();
        assert!(second.next_batch().unwrap().is_empty());
        File::options().append(true).open(&path).unwrap()
            .write_all(&record(b"SECOND")).unwrap();
        assert_eq!(second.next_batch().unwrap(), vec![b"SECOND".to_vec()]);
        second.commit().unwrap();
        drop(second);
        assert_eq!(fs::read(&cursor).unwrap(),
            u64::try_from(record(b"FIRST").len() + record(b"SECOND").len())
                .unwrap().to_be_bytes());
        fs::remove_file(cursor).unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_invalid_tdq_record_length() {
        let path = std::env::temp_dir().join(format!(
            "cobcics-printer-bad-{}", std::process::id()));
        let mut queue = PrinterQueue {
            path: path.clone(), cursor: path.with_extension("cursor"),
            offset: 0, leased: false,
        };
        fs::write(&path, [0, 0, 0, 0]).unwrap();
        assert!(queue.next_batch().unwrap_err().contains("invalid printer queue record length"));
        fs::remove_file(path).unwrap();
    }
}
