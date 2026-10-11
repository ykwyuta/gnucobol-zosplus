//! A length-checked DL/I call boundary for COBOL and PL/I front ends.
use crate::catalog::{Dbd, Pcb, Psb};
use crate::csv_load::{cp1047_ascii_map, cp1047_to_ascii_map};
use crate::dli::{ReadOp, ReadOutcome, Session, Status, WriteOp};
use crate::store::Database;

pub const PCB_HEADER_LEN: usize = 36;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcbEncoding {
    Ascii,
    Cp1047,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericByteOrder {
    Big,
    Little,
}
impl PcbEncoding {
    fn cp1047(self) -> bool {
        self == Self::Cp1047
    }
    pub fn encode_control(self, bytes: &[u8]) -> Result<Vec<u8>, String> {
        bytes
            .iter()
            .map(|byte| {
                if !byte.is_ascii() {
                    return Err("control text must be ASCII".into());
                }
                Ok(control_byte(*byte, self.cp1047()))
            })
            .collect()
    }
    pub fn decode_control(self, byte: u8) -> u8 {
        if self.cp1047() {
            cp1047_to_ascii_map()[byte as usize]
        } else {
            byte
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DliFunction {
    Gu,
    Gn,
    Gnp,
    Ghu,
    Ghn,
    Ghnp,
    Isrt,
    Repl,
    Dlet,
}
impl DliFunction {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name.trim().to_ascii_uppercase().as_str() {
            "GU" => Ok(Self::Gu),
            "GN" => Ok(Self::Gn),
            "GNP" => Ok(Self::Gnp),
            "GHU" => Ok(Self::Ghu),
            "GHN" => Ok(Self::Ghn),
            "GHNP" => Ok(Self::Ghnp),
            "ISRT" => Ok(Self::Isrt),
            "REPL" => Ok(Self::Repl),
            "DLET" => Ok(Self::Dlet),
            other => Err(format!("unsupported DL/I function {other}")),
        }
    }
    fn read(self) -> Option<ReadOp> {
        match self {
            Self::Gu => Some(ReadOp::Gu),
            Self::Gn => Some(ReadOp::Gn),
            Self::Gnp => Some(ReadOp::Gnp),
            Self::Ghu => Some(ReadOp::Ghu),
            Self::Ghn => Some(ReadOp::Ghn),
            Self::Ghnp => Some(ReadOp::Ghnp),
            _ => None,
        }
    }
}

fn control_byte(byte: u8, cp1047: bool) -> u8 {
    if cp1047 {
        cp1047_ascii_map()[byte as usize]
    } else {
        byte
    }
}

fn write_text(dst: &mut [u8], value: &str, cp1047: bool) -> Result<(), String> {
    if !value.is_ascii() || value.len() > dst.len() {
        return Err("PCB text does not fit its field".into());
    }
    dst.fill(control_byte(b' ', cp1047));
    for (target, byte) in dst.iter_mut().zip(value.bytes()) {
        *target = control_byte(byte, cp1047);
    }
    Ok(())
}

fn segment_level(dbd: &Dbd, name: &str) -> Result<usize, String> {
    let mut level = 0;
    let mut segment = Some(name);
    while let Some(current) = segment {
        level += 1;
        if level > 99 {
            return Err("segment level exceeds PCB field".into());
        }
        segment = dbd
            .segment(current)
            .ok_or_else(|| format!("unknown segment {current}"))?
            .parent
            .as_deref();
    }
    Ok(level)
}

/// The nine-field DB PCB mask used by the Bank of Z COBOL copybooks.
pub struct PcbMask<'a> {
    bytes: &'a mut [u8],
    cp1047: bool,
    key_capacity: usize,
}
impl<'a> PcbMask<'a> {
    pub fn new(
        bytes: &'a mut [u8],
        dbd: &Dbd,
        pcb: &Pcb,
        encoding: PcbEncoding,
    ) -> Result<Self, String> {
        let key_capacity = pcb.keylen.unwrap_or(0);
        if bytes.len() < PCB_HEADER_LEN + key_capacity {
            return Err(format!(
                "PCB mask needs at least {} bytes; got {}",
                PCB_HEADER_LEN + key_capacity,
                bytes.len()
            ));
        }
        let cp1047 = encoding.cp1047();
        write_text(&mut bytes[0..8], &dbd.name, cp1047)?;
        write_text(
            &mut bytes[12..16],
            pcb.procopt.as_deref().unwrap_or(""),
            cp1047,
        )?;
        bytes[16..20].fill(0);
        let count = u32::try_from(pcb.sensitive_segments.len())
            .map_err(|_| "too many sensitive segments")?;
        bytes[32..36].copy_from_slice(&count.to_be_bytes());
        Ok(Self {
            bytes,
            cp1047,
            key_capacity,
        })
    }

    pub fn apply(&mut self, dbd: &Dbd, outcome: &ReadOutcome) -> Result<(), String> {
        let level = if let Some(name) = &outcome.segment_name {
            if outcome.key_feedback.len() > self.key_capacity {
                return Err("key feedback exceeds PCB KEYLEN".into());
            }
            Some(segment_level(dbd, name)?)
        } else {
            None
        };
        for (target, byte) in self.bytes[10..12].iter_mut().zip(outcome.status.code()) {
            *target = control_byte(byte, self.cp1047);
        }
        if let (Some(name), Some(level)) = (&outcome.segment_name, level) {
            write_text(&mut self.bytes[8..10], &format!("{level:02}"), self.cp1047)?;
            write_text(&mut self.bytes[20..28], name, self.cp1047)?;
            self.bytes[28..32].copy_from_slice(&(outcome.key_feedback.len() as u32).to_be_bytes());
            self.bytes[PCB_HEADER_LEN..PCB_HEADER_LEN + outcome.key_feedback.len()]
                .copy_from_slice(&outcome.key_feedback);
        }
        Ok(())
    }
}

pub struct CallAdapter {
    session: Session,
    psb: Psb,
    dbds: Vec<Dbd>,
    current_segments: Vec<Option<String>>,
    pcb_encoding: PcbEncoding,
    numeric_byte_order: NumericByteOrder,
}
impl CallAdapter {
    pub fn new(database: Database, psb: Psb, pcb_encoding: PcbEncoding) -> Result<Self, String> {
        let dbds = psb
            .pcbs
            .iter()
            .filter_map(|pcb| pcb.dbd_name.as_deref())
            .map(|name| {
                database
                    .definition(name)
                    .ok_or_else(|| format!("missing DBD {name}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let current_segments = vec![None; psb.pcbs.len()];
        let session = Session::new(database, psb.clone())?;
        Ok(Self {
            session,
            psb,
            dbds,
            current_segments,
            pcb_encoding,
            numeric_byte_order: NumericByteOrder::Big,
        })
    }

    pub fn set_numeric_byte_order(&mut self, order: NumericByteOrder) {
        self.numeric_byte_order = order;
    }

    pub fn rollback(&mut self) {
        self.session.rollback();
        self.current_segments.fill(None);
    }

    pub fn commit(&mut self) -> Result<(), String> {
        self.session.commit().map_err(|e| e.to_string())?;
        self.current_segments.fill(None);
        Ok(())
    }

    /// Decode only SSA control bytes. The qualification value remains in its original bytes.
    pub fn call_encoded(
        &mut self,
        pcb_index: usize,
        function: &[u8],
        pcb_bytes: &mut [u8],
        io_area: &mut [u8],
        ssas: &[&[u8]],
    ) -> Result<Status, String> {
        let function: Vec<u8> = function
            .iter()
            .map(|byte| self.pcb_encoding.decode_control(*byte))
            .collect();
        let function = DliFunction::parse(
            std::str::from_utf8(&function).map_err(|_| "invalid function code")?,
        )?;
        let pcb = self
            .psb
            .pcbs
            .get(pcb_index)
            .ok_or("PCB index out of range")?;
        let dbd_name = pcb
            .dbd_name
            .as_deref()
            .ok_or("I/O and GSAM PCBs need another adapter")?;
        let dbd = self
            .dbds
            .iter()
            .find(|d| d.name == dbd_name)
            .ok_or("missing DBD")?;
        let normalized = ssas
            .iter()
            .map(|ssa| normalize_ssa(ssa, dbd, self.pcb_encoding, self.numeric_byte_order))
            .collect::<Result<Vec<_>, _>>()?;
        let refs: Vec<&[u8]> = normalized.iter().map(Vec::as_slice).collect();
        self.call(pcb_index, function, pcb_bytes, io_area, &refs)
    }

    /// `function` and SSA names/operators are ASCII control bytes. Segment bodies remain raw.
    pub fn call(
        &mut self,
        pcb_index: usize,
        function: DliFunction,
        pcb_bytes: &mut [u8],
        io_area: &mut [u8],
        ssas: &[&[u8]],
    ) -> Result<Status, String> {
        let pcb = self
            .psb
            .pcbs
            .get(pcb_index)
            .ok_or("PCB index out of range")?;
        let dbd_name = pcb
            .dbd_name
            .as_deref()
            .ok_or("I/O and GSAM PCBs need another adapter")?;
        let dbd = self
            .dbds
            .iter()
            .find(|d| d.name == dbd_name)
            .ok_or("missing DBD")?;
        let mut mask = PcbMask::new(pcb_bytes, dbd, pcb, self.pcb_encoding)?;
        let mut outcome = if let Some(op) = function.read() {
            self.session.read(pcb_index, op, ssas)?
        } else {
            match function {
                DliFunction::Isrt | DliFunction::Repl | DliFunction::Dlet => {
                    let segment = if let Some(ssa) = ssas.last() {
                        std::str::from_utf8(ssa.get(..8).ok_or("SSA is shorter than 8 bytes")?)
                            .map_err(|_| "SSA segment name is not ASCII")?
                            .trim()
                            .to_ascii_uppercase()
                    } else {
                        self.current_segments[pcb_index]
                            .clone()
                            .ok_or("write requires an SSA or current segment")?
                    };
                    let op = match function {
                        DliFunction::Isrt => WriteOp::Isrt,
                        DliFunction::Repl => WriteOp::Repl,
                        DliFunction::Dlet => WriteOp::Dlet,
                        _ => unreachable!(),
                    };
                    let converted = if function == DliFunction::Dlet {
                        None
                    } else {
                        Some(convert_numeric_fields(
                            dbd,
                            &segment,
                            io_area,
                            self.numeric_byte_order,
                        )?)
                    };
                    self.session
                        .write(pcb_index, op, &segment, converted.as_deref(), ssas)?
                }
                _ => unreachable!(),
            }
        };
        if function.read().is_some() {
            if let Some(record) = &outcome.record {
                if record.data.len() > io_area.len() {
                    return Err(format!(
                        "I/O area needs {} bytes; got {}",
                        record.data.len(),
                        io_area.len()
                    ));
                }
                let converted = convert_numeric_fields(
                    dbd,
                    &record.segment,
                    &record.data,
                    self.numeric_byte_order,
                )?;
                io_area[..record.data.len()].copy_from_slice(&converted);
            }
        }
        convert_key_feedback(dbd, &mut outcome, self.numeric_byte_order)?;
        if let Some(name) = &outcome.segment_name {
            self.current_segments[pcb_index] = Some(name.clone());
        } else if function.read().is_some() {
            self.current_segments[pcb_index] = None;
        }
        mask.apply(dbd, &outcome)?;
        Ok(outcome.status)
    }
}

fn normalize_ssa(
    raw: &[u8],
    dbd: &Dbd,
    encoding: PcbEncoding,
    order: NumericByteOrder,
) -> Result<Vec<u8>, String> {
    if raw.len() < 8 {
        return Err("SSA is shorter than 8 bytes".into());
    }
    let mut normalized = raw.to_vec();
    for byte in &mut normalized[..raw.len().min(9)] {
        *byte = encoding.decode_control(*byte);
    }
    if normalized.get(8) == Some(&b'(') {
        if normalized.len() < 20 {
            return Err("qualified SSA is too short".into());
        }
        for byte in &mut normalized[9..19] {
            *byte = encoding.decode_control(*byte);
        }
        let segment = std::str::from_utf8(&normalized[..8])
            .map_err(|_| "invalid SSA segment name")?
            .trim();
        let field = std::str::from_utf8(&normalized[9..17])
            .map_err(|_| "invalid SSA field name")?
            .trim();
        let field = dbd
            .segment(segment)
            .and_then(|segment| segment.fields.iter().find(|f| f.name == field))
            .ok_or_else(|| format!("unknown SSA field {segment}/{field}"))?;
        let close = 19 + field.bytes;
        let byte = normalized
            .get_mut(close)
            .ok_or("SSA value is shorter than DBD field")?;
        *byte = encoding.decode_control(*byte);
        if order == NumericByteOrder::Little
            && matches!(field.datatype.as_deref(), Some("INT" | "LONG"))
        {
            normalized[19..close].reverse();
        }
    }
    Ok(normalized)
}

fn convert_numeric_fields(
    dbd: &Dbd,
    segment: &str,
    bytes: &[u8],
    order: NumericByteOrder,
) -> Result<Vec<u8>, String> {
    let mut result = bytes.to_vec();
    if order == NumericByteOrder::Little {
        let definition = dbd.segment(segment).ok_or("unknown segment")?;
        for field in &definition.fields {
            if matches!(field.datatype.as_deref(), Some("INT" | "LONG")) {
                let range = field.start - 1..field.start - 1 + field.bytes;
                result
                    .get_mut(range)
                    .ok_or("numeric field exceeds segment")?
                    .reverse();
            }
        }
    }
    Ok(result)
}

fn convert_key_feedback(
    dbd: &Dbd,
    outcome: &mut ReadOutcome,
    order: NumericByteOrder,
) -> Result<(), String> {
    if order != NumericByteOrder::Little || outcome.record.is_none() {
        return Ok(());
    }
    let record = outcome.record.as_ref().unwrap();
    let mut offset = 0;
    let root = dbd
        .segments
        .iter()
        .find(|segment| segment.parent.is_none())
        .ok_or("DBD has no root")?;
    for segment in [root, dbd.segment(&record.segment).ok_or("unknown segment")?] {
        if offset > 0 && segment.name == root.name {
            break;
        }
        let key = segment.key().ok_or("segment has no sequence field")?;
        let end = offset + key.bytes;
        if matches!(key.datatype.as_deref(), Some("INT" | "LONG")) {
            outcome
                .key_feedback
                .get_mut(offset..end)
                .ok_or("key feedback is too short")?
                .reverse();
        }
        offset = end;
        if segment.name == record.segment {
            break;
        }
    }
    Ok(())
}
