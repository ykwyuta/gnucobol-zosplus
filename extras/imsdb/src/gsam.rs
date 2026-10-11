//! Fixed-record GSAM adapter for the CardDemo DBDs.
use crate::call_adapter::PcbEncoding;
use crate::catalog::{GsamDefinition, Pcb};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

pub struct GsamSession {
    dbd_name: String,
    procopt: String,
    definition: GsamDefinition,
    encoding: PcbEncoding,
    input: Option<File>,
    input_position: u64,
    pending: Vec<Vec<u8>>,
}

impl GsamSession {
    pub fn new(
        dbd_name: &str,
        pcb: &Pcb,
        definition: GsamDefinition,
        encoding: PcbEncoding,
    ) -> Self {
        Self {
            dbd_name: dbd_name.to_owned(),
            procopt: pcb.procopt.clone().unwrap_or_default(),
            definition,
            encoding,
            input: None,
            input_position: 0,
            pending: Vec::new(),
        }
    }

    fn dd_path(name: &str) -> Result<PathBuf, String> {
        std::env::var_os(format!("DD_{name}"))
            .map(PathBuf::from)
            .ok_or_else(|| format!("GSAM requires DD_{name}"))
    }

    fn input(&mut self) -> Result<&mut File, String> {
        if self.input.is_none() {
            let path = Self::dd_path(&self.definition.input_dd)?;
            self.input = Some(File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?);
        }
        Ok(self.input.as_mut().unwrap())
    }

    fn pcb(&self, pcb: &mut [u8], status: &str, rsa: Option<[u8; 8]>) -> Result<(), String> {
        if pcb.len() < 44 {
            return Err("GSAM PCB requires at least 44 bytes".into());
        }
        fn field(dst: &mut [u8], value: &str, encoding: PcbEncoding) -> Result<(), String> {
            let bytes = encoding.encode_control(value.as_bytes())?;
            if bytes.len() > dst.len() {
                return Err("GSAM PCB text exceeds its field".into());
            }
            dst.fill(encoding.encode_control(b" ")?[0]);
            dst[..bytes.len()].copy_from_slice(&bytes);
            Ok(())
        }
        field(&mut pcb[0..8], &self.dbd_name, self.encoding)?;
        field(&mut pcb[8..10], "00", self.encoding)?;
        field(&mut pcb[10..12], status, self.encoding)?;
        field(&mut pcb[12..16], &self.procopt, self.encoding)?;
        pcb[16..20].fill(0);
        field(&mut pcb[20..28], "", self.encoding)?;
        pcb[28..32].copy_from_slice(&12u32.to_be_bytes());
        pcb[32..36].fill(0);
        if let Some(rsa) = rsa {
            pcb[36..44].copy_from_slice(&rsa);
        }
        Ok(())
    }

    pub fn call(
        &mut self,
        function: &str,
        pcb: &mut [u8],
        io: &mut [u8],
        rsa_area: Option<&mut [u8]>,
    ) -> Result<(), String> {
        let len = self.definition.record_len;
        if rsa_area.as_ref().is_some_and(|rsa| rsa.len() != 8) {
            return Err("GSAM RSA must be eight bytes".into());
        }
        let operation = function.trim().to_ascii_uppercase();
        if operation == "OPEN" || operation == "CLSE" {
            self.input = None;
            self.input_position = 0;
            self.pcb(pcb, "", None)?;
            return Ok(());
        }
        if io.len() < len {
            return Err(format!("GSAM I/O area requires at least {len} bytes"));
        }
        let (status, returned_rsa) = match operation.as_str() {
            "GN" | "GU" if self.procopt.contains(['L', 'S', 'G']) => {
                let ordinal = if operation == "GU" {
                    let Some(rsa) = rsa_area.as_ref() else {
                        self.pcb(pcb, "AH", None)?;
                        return Ok(());
                    };
                    let mut token = [0u8; 8];
                    token.copy_from_slice(rsa);
                    let ordinal = u64::from_be_bytes(token);
                    if ordinal == 0 {
                        self.pcb(pcb, "AJ", None)?;
                        return Ok(());
                    }
                    ordinal
                } else {
                    self.input_position + 1
                };
                let offset = (ordinal - 1)
                    .checked_mul(len as u64)
                    .ok_or("GSAM RSA offset overflow")?;
                let file = self.input()?;
                file.seek(SeekFrom::Start(offset))
                    .map_err(|e| e.to_string())?;
                let mut data = vec![0; len];
                let mut count = 0;
                while count < len {
                    let n = file.read(&mut data[count..]).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    count += n;
                }
                if count == 0 {
                    if operation == "GN" {
                        self.input_position = 0;
                    }
                    ("GB", None)
                } else if count < len {
                    ("AF", None)
                } else {
                    io[..len].copy_from_slice(&data);
                    self.input_position = ordinal;
                    ("", Some(ordinal.to_be_bytes()))
                }
            }
            "ISRT" if self.procopt.contains(['L', 'I']) => {
                let path = Self::dd_path(&self.definition.output_dd)?;
                let existing = if path.exists() {
                    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                    if size % len as u64 != 0 {
                        return Err("GSAM output has a partial fixed record".into());
                    }
                    size / len as u64
                } else {
                    0
                };
                let ordinal = existing + self.pending.len() as u64 + 1;
                self.pending.push(io[..len].to_vec());
                ("", Some(ordinal.to_be_bytes()))
            }
            _ => ("AM", None),
        };
        self.pcb(pcb, status, returned_rsa)?;
        if let (Some(rsa), Some(area)) = (returned_rsa, rsa_area) {
            area.copy_from_slice(&rsa);
        }
        Ok(())
    }

    pub fn commit(&mut self) -> Result<(), String> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let path = Self::dd_path(&self.definition.output_dd)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let original_len = file.metadata().map_err(|e| e.to_string())?.len();
        if original_len % self.definition.record_len as u64 != 0 {
            return Err("GSAM output has a partial fixed record".into());
        }
        file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
        let write_result = self
            .pending
            .iter()
            .try_for_each(|record| file.write_all(record))
            .and_then(|()| file.sync_data());
        if let Err(error) = write_result {
            file.set_len(original_len).map_err(|restore| {
                format!("GSAM write failed: {error}; truncate failed: {restore}")
            })?;
            file.sync_data().map_err(|restore| {
                format!("GSAM write failed: {error}; restore sync failed: {restore}")
            })?;
            return Err(format!("GSAM write failed: {error}"));
        }
        self.pending.clear();
        Ok(())
    }

    pub fn rollback(&mut self) {
        self.pending.clear();
    }
}
