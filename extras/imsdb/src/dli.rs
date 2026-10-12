use crate::catalog::{Dbd, Psb};
use crate::store::{Database, Record, StoreError, Transaction};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOp {
    Gu,
    Gn,
    Gnp,
    Ghu,
    Ghn,
    Ghnp,
}
impl ReadOp {
    fn base(self) -> Self {
        match self {
            Self::Ghu => Self::Gu,
            Self::Ghn => Self::Gn,
            Self::Ghnp => Self::Gnp,
            other => other,
        }
    }
    fn is_hold(self) -> bool {
        matches!(self, Self::Ghu | Self::Ghn | Self::Ghnp)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOp {
    Isrt,
    Repl,
    Dlet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    HigherLevel,
    OtherSegmentType,
    NotFound,
    End,
    Duplicate,
    NotHeld,
    NotAllowed,
}
impl Status {
    pub fn code(self) -> [u8; 2] {
        match self {
            Self::Ok => *b"  ",
            Self::HigherLevel => *b"GA",
            Self::OtherSegmentType => *b"GK",
            Self::NotFound => *b"GE",
            Self::End => *b"GB",
            Self::Duplicate => *b"II",
            Self::NotHeld => *b"DJ",
            Self::NotAllowed => *b"AM",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReadOutcome {
    pub status: Status,
    pub record: Option<Record>,
    pub segment_name: Option<String>,
    pub key_feedback: Vec<u8>,
    pub secondary_key: bool,
}
impl ReadOutcome {
    fn found(record: Record, status: Status) -> Self {
        let mut key_feedback = record.root_key.clone();
        if record.parent.is_some() {
            key_feedback.extend_from_slice(&record.key);
        }
        Self {
            status,
            segment_name: Some(record.segment.clone()),
            key_feedback,
            secondary_key: false,
            record: Some(record),
        }
    }
    fn empty(status: Status) -> Self {
        Self {
            status,
            record: None,
            segment_name: None,
            key_feedback: Vec::new(),
            secondary_key: false,
        }
    }
}

#[derive(Default)]
struct Position {
    current: Option<u64>,
    parent: Option<u64>,
    exhausted: bool,
    held: Option<u64>,
    last_deleted: Option<Record>,
    secondary: Option<(Vec<u8>, u64)>,
}

pub struct Session {
    database: Database,
    psb: Psb,
    positions: Vec<Position>,
    transaction: Transaction,
    lock_timeout: Duration,
}

struct Ssa {
    segment: String,
    condition: Option<(String, Vec<u8>)>,
}
impl Ssa {
    fn parse(raw: &[u8], dbd: &Dbd) -> Result<Self, String> {
        if raw.len() < 8 {
            return Err("SSA must contain an 8-byte segment name".into());
        }
        let segment = std::str::from_utf8(&raw[..8])
            .map_err(|_| "SSA segment name is not ASCII")?
            .trim()
            .to_ascii_uppercase();
        let def = dbd
            .segment(&segment)
            .ok_or_else(|| format!("unknown SSA segment {segment}"))?;
        if raw.get(8).is_none_or(|byte| *byte == b' ') {
            return Ok(Self {
                segment,
                condition: None,
            });
        }
        if raw.get(8) != Some(&b'(') || raw.len() < 20 {
            return Err(format!(
                "{segment}: unsupported SSA command or qualification"
            ));
        }
        let field = std::str::from_utf8(&raw[9..17])
            .map_err(|_| "SSA field name is not ASCII")?
            .trim()
            .to_ascii_uppercase();
        let field_def = def
            .fields
            .iter()
            .find(|f| f.name == field)
            .ok_or_else(|| format!("{segment}: unknown SSA field {field}"))?;
        if &raw[17..19] != b"= " && &raw[17..19] != b"EQ" {
            return Err(format!(
                "{segment}: only equality SSA predicates are supported"
            ));
        }
        let value_end = 19 + field_def.bytes;
        if raw.get(value_end) != Some(&b')') {
            return Err(format!(
                "{segment}: SSA value length does not match DBD field {}",
                field_def.name
            ));
        }
        Ok(Self {
            segment,
            condition: Some((field, raw[19..value_end].to_vec())),
        })
    }
    fn matches(&self, record: &Record, dbd: &Dbd) -> bool {
        if record.segment != self.segment {
            return false;
        }
        let Some((field, value)) = &self.condition else {
            return true;
        };
        let Some(def) = dbd
            .segment(&self.segment)
            .and_then(|segment| segment.fields.iter().find(|f| &f.name == field))
        else {
            return false;
        };
        record.data.get(def.start - 1..def.start - 1 + def.bytes) == Some(value.as_slice())
    }
}

impl Session {
    pub fn indexed_fields(&self, pcb_index: usize) -> Result<Vec<(String, String, usize)>, String> {
        let pcb = self
            .psb
            .pcbs
            .get(pcb_index)
            .ok_or("PCB index out of range")?;
        let dbd = pcb.dbd_name.as_deref().ok_or("DB PCB has no DBDNAME")?;
        let mut result = Vec::new();
        if let Some(index) = pcb.procseq.as_deref() {
            result.push(self.procseq_field(index, dbd)?);
        }
        for (_, indexes) in &pcb.indices {
            for index in indexes {
                result.push(self.procseq_field(index, dbd)?);
            }
        }
        Ok(result)
    }
    pub fn procseq_field(&self, index: &str, dbd: &str) -> Result<(String, String, usize), String> {
        self.database
            .procseq_field(index, dbd)
            .map_err(|error| error.to_string())
    }
    pub fn new(database: Database, psb: Psb) -> Result<Self, String> {
        let dbds: Vec<Dbd> = psb
            .pcbs
            .iter()
            .filter_map(|pcb| pcb.dbd_name.as_deref())
            .filter_map(|name| database.definition(name))
            .collect();
        psb.validate(&dbds)?;
        for pcb in &psb.pcbs {
            if pcb.kind == "DB"
                && pcb
                    .dbd_name
                    .as_deref()
                    .and_then(|name| database.definition(name))
                    .is_some_and(|dbd| dbd.access == "INDEX")
            {
                if pcb.procseq.is_some() {
                    return Err("index PCB cannot specify PROCSEQ".into());
                }
                database
                    .index_pointer(pcb.dbd_name.as_deref().unwrap())
                    .map_err(|error| error.to_string())?;
            }
            if let Some(index) = pcb.procseq.as_deref() {
                if pcb.kind != "DB" {
                    return Err("PROCSEQ requires a DB PCB".into());
                }
                database
                    .procseq_field(index, pcb.dbd_name.as_deref().ok_or("missing DBDNAME")?)
                    .map_err(|error| error.to_string())?;
            }
            for (segment, indexes) in &pcb.indices {
                if pcb.kind != "DB" {
                    return Err("SENSEG INDICES requires a DB PCB".into());
                }
                for index in indexes {
                    let (target, _, _) = database
                        .procseq_field(index, pcb.dbd_name.as_deref().ok_or("missing DBDNAME")?)
                        .map_err(|error| error.to_string())?;
                    if &target != segment {
                        return Err(format!(
                            "{index}: SENSEG INDICES target is {target}, not {segment}"
                        ));
                    }
                }
            }
        }
        let positions = (0..psb.pcbs.len()).map(|_| Position::default()).collect();
        Ok(Self {
            transaction: database.begin(),
            database,
            psb,
            positions,
            lock_timeout: Duration::from_secs(5),
        })
    }
    pub fn set_lock_timeout(&mut self, timeout: Duration) {
        self.lock_timeout = timeout;
    }
    /// EXEC DLI get commands have update eligibility like their Get Hold call forms.
    pub fn read_exec(
        &mut self,
        pcb_index: usize,
        op: ReadOp,
        raw_ssas: &[&[u8]],
    ) -> Result<ReadOutcome, String> {
        let hold = match op.base() {
            ReadOp::Gu => ReadOp::Ghu,
            ReadOp::Gn => ReadOp::Ghn,
            ReadOp::Gnp => ReadOp::Ghnp,
            _ => unreachable!(),
        };
        self.read(pcb_index, hold, raw_ssas)
    }
    pub fn commit(&mut self) -> Result<(), StoreError> {
        self.commit_checkpoint(None)
    }
    pub fn commit_checkpoint(&mut self, checkpoint: Option<[u8; 8]>) -> Result<(), StoreError> {
        if let Some(id) = checkpoint {
            self.transaction.set_checkpoint(id);
        }
        let previous = std::mem::replace(&mut self.transaction, self.database.begin());
        self.positions = (0..self.psb.pcbs.len())
            .map(|_| Position::default())
            .collect();
        previous.commit()
    }
    pub fn rollback(&mut self) {
        let previous = std::mem::replace(&mut self.transaction, self.database.begin());
        previous.rollback();
        self.positions = (0..self.psb.pcbs.len())
            .map(|_| Position::default())
            .collect();
    }
    pub fn write(
        &mut self,
        pcb_index: usize,
        op: WriteOp,
        segment: &str,
        data: Option<&[u8]>,
        raw_ssas: &[&[u8]],
    ) -> Result<ReadOutcome, String> {
        let pcb = self
            .psb
            .pcbs
            .get(pcb_index)
            .ok_or("PCB index out of range")?;
        if pcb.kind != "DB" {
            return Err("GSAM and I/O PCBs require a separate adapter".into());
        }
        let dbd_name = pcb.dbd_name.as_deref().ok_or("DB PCB has no DBDNAME")?;
        let dbd = self.database.definition(dbd_name).ok_or("missing DBD")?;
        if dbd.access == "INDEX" {
            return Ok(ReadOutcome::empty(Status::NotAllowed));
        }
        let def = dbd
            .segment(segment)
            .ok_or_else(|| format!("unknown segment {segment}"))?;
        if !pcb.sensitive_segments.is_empty()
            && !pcb.sensitive_segments.iter().any(|s| s == segment)
        {
            return Err(format!("PCB is not sensitive to {segment}"));
        }
        let procopt = pcb.procopt.as_deref().unwrap_or("");
        let allowed = match op {
            WriteOp::Isrt => procopt.contains(['I', 'A', 'L']),
            WriteOp::Repl => procopt.contains(['R', 'A']),
            WriteOp::Dlet => procopt.contains(['D', 'A']),
        };
        if !allowed {
            return Ok(ReadOutcome::empty(Status::NotAllowed));
        }
        let ssas: Vec<Ssa> = raw_ssas
            .iter()
            .map(|raw| Ssa::parse(raw, &dbd))
            .collect::<Result<_, _>>()?;
        if ssas.last().is_some_and(|ssa| ssa.segment != segment) {
            return Err("last SSA does not name the update segment".into());
        }
        let position = &mut self.positions[pcb_index];
        match op {
            WriteOp::Isrt => {
                let bytes = data.ok_or("ISRT requires segment data")?;
                let parent = if let Some(expected) = def.parent.as_deref() {
                    let candidate = position
                        .current
                        .and_then(|id| self.transaction.record(id))
                        .filter(|record| record.segment == expected)
                        .or_else(|| {
                            position
                                .parent
                                .and_then(|id| self.transaction.record(id))
                                .filter(|record| record.segment == expected)
                        });
                    let Some(candidate) = candidate else {
                        return Ok(ReadOutcome::empty(Status::NotFound));
                    };
                    Some(candidate.id)
                } else {
                    None
                };
                let id = match self.transaction.insert(
                    dbd_name,
                    segment,
                    parent,
                    bytes.to_vec(),
                    self.lock_timeout,
                ) {
                    Ok(id) => id,
                    Err(StoreError::Duplicate(_)) => {
                        return Ok(ReadOutcome::empty(Status::Duplicate))
                    }
                    Err(StoreError::Missing(_)) => return Ok(ReadOutcome::empty(Status::NotFound)),
                    Err(error) => return Err(error.to_string()),
                };
                let record = self
                    .transaction
                    .record(id)
                    .ok_or("inserted record disappeared")?;
                position.current = Some(id);
                position.parent = parent.or(Some(id));
                position.held = None;
                position.last_deleted = None;
                Ok(ReadOutcome::found(record, Status::Ok))
            }
            WriteOp::Repl | WriteOp::Dlet => {
                let Some(id) = position.held.filter(|id| position.current == Some(*id)) else {
                    return Ok(ReadOutcome::empty(Status::NotHeld));
                };
                let Some(record) = self.transaction.record(id) else {
                    position.held = None;
                    return Ok(ReadOutcome::empty(Status::NotHeld));
                };
                if record.segment != segment {
                    return Ok(ReadOutcome::empty(Status::NotHeld));
                }
                if op == WriteOp::Repl {
                    let bytes = data.ok_or("REPL requires segment data")?;
                    match self
                        .transaction
                        .replace(id, bytes.to_vec(), self.lock_timeout)
                    {
                        Ok(()) => {}
                        Err(StoreError::Definition(_)) => {
                            return Ok(ReadOutcome::empty(Status::NotAllowed))
                        }
                        Err(error) => return Err(error.to_string()),
                    }
                    let updated = self
                        .transaction
                        .record(id)
                        .ok_or("replaced record disappeared")?;
                    Ok(ReadOutcome::found(updated, Status::Ok))
                } else {
                    delete_subtree(&mut self.transaction, &dbd, dbd_name, id, self.lock_timeout)?;
                    position.held = None;
                    position.last_deleted = Some(record.clone());
                    Ok(ReadOutcome::found(record, Status::Ok))
                }
            }
        }
    }
    pub fn read(
        &mut self,
        pcb_index: usize,
        op: ReadOp,
        raw_ssas: &[&[u8]],
    ) -> Result<ReadOutcome, String> {
        let pcb = self
            .psb
            .pcbs
            .get(pcb_index)
            .ok_or("PCB index out of range")?;
        if pcb.kind != "DB" {
            return Err("GSAM and I/O PCBs require a separate adapter".into());
        }
        let procopt = pcb.procopt.as_deref().unwrap_or("");
        if !procopt.contains(['G', 'R', 'D', 'A']) {
            return Err(format!("PCB {} does not permit get calls", pcb_index));
        }
        // A Get Hold on a get-only PCB cannot be followed by REPL or DLET.
        // Such reads must not retain update locks across other PCB reads.
        let update_hold = op.is_hold() && procopt.contains(['R', 'D', 'A']);
        let dbd_name = pcb.dbd_name.as_deref().ok_or("DB PCB has no DBDNAME")?;
        let dbd = self.database.definition(dbd_name).ok_or("missing DBD")?;
        if dbd.access == "INDEX" {
            let name = dbd_name.to_string();
            return self.read_index(pcb_index, &name, op, raw_ssas);
        }
        if let Some(index) = pcb.procseq.as_ref() {
            if op.base() != ReadOp::Gnp {
                let index = index.clone();
                let dbd_name = dbd_name.to_string();
                return self.read_procseq(pcb_index, &index, &dbd_name, op, raw_ssas);
            }
        }
        if op.base() == ReadOp::Gu && raw_ssas.len() == 1 && !pcb.indices.is_empty() {
            let raw = raw_ssas[0];
            if raw.len() >= 19 && raw[8] == b'(' {
                let segment = std::str::from_utf8(&raw[..8])
                    .map_err(|_| "invalid SSA segment")?
                    .trim()
                    .to_ascii_uppercase();
                let field = std::str::from_utf8(&raw[9..17])
                    .map_err(|_| "invalid SSA field")?
                    .trim()
                    .to_ascii_uppercase();
                let mut selected = None;
                for (sensitive, indexes) in &pcb.indices {
                    if *sensitive != segment {
                        continue;
                    }
                    for index in indexes {
                        let (target, name, length) = self
                            .database
                            .procseq_field(index, dbd_name)
                            .map_err(|error| error.to_string())?;
                        if target == segment && name == field {
                            if selected.is_some() {
                                return Err(format!("ambiguous SENSEG INDICES field {field}"));
                            }
                            selected = Some((index.clone(), length));
                        }
                    }
                }
                if let Some((index, length)) = selected {
                    return self.read_senseg_index(pcb_index, &index, length, op, raw);
                }
            }
        }
        let ssas: Vec<Ssa> = raw_ssas
            .iter()
            .map(|raw| Ssa::parse(raw, &dbd))
            .collect::<Result<_, _>>()?;
        for ssa in &ssas {
            if !pcb.sensitive_segments.is_empty() && !pcb.sensitive_segments.contains(&ssa.segment)
            {
                return Err(format!("PCB is not sensitive to {}", ssa.segment));
            }
        }
        let position = &mut self.positions[pcb_index];
        position.held = None;
        let previous = position
            .current
            .and_then(|id| self.transaction.record(id))
            .or_else(|| position.last_deleted.clone());
        let base = op.base();
        let found = match base {
            ReadOp::Gu => {
                position.current = None;
                position.parent = None;
                position.exhausted = false;
                position.last_deleted = None;
                if ssas.is_empty() {
                    let root = dbd.segments.first().ok_or("DBD has no segment")?;
                    self.transaction.first_child(dbd_name, None, &root.name)
                } else {
                    let mut parent = None;
                    let mut result = None;
                    for (level, ssa) in ssas.iter().enumerate() {
                        let def = dbd.segment(&ssa.segment).unwrap();
                        if (level == 0 && def.parent.is_some())
                            || (level > 0
                                && def.parent.as_deref()
                                    != result.as_ref().map(|r: &Record| r.segment.as_str()))
                        {
                            return Err("SSA hierarchy does not match DBD".into());
                        }
                        result = find_match(&self.transaction, &dbd, dbd_name, parent, ssa);
                        parent = result.as_ref().map(|r| r.id);
                        if result.is_none() {
                            break;
                        }
                    }
                    result
                }
            }
            ReadOp::Gn => {
                if ssas.len() > 1 {
                    return Err("GN currently accepts at most one SSA".into());
                }
                if position.exhausted {
                    position.current = None;
                    position.parent = None;
                    position.exhausted = false;
                }
                let mut next = if let Some(id) = position.current {
                    let record = self
                        .transaction
                        .record(id)
                        .or_else(|| position.last_deleted.clone())
                        .ok_or("current segment was deleted")?;
                    next_after(&self.transaction, &dbd, &record)
                } else {
                    let root = dbd.segments.first().ok_or("DBD has no segment")?;
                    self.transaction.first_child(dbd_name, None, &root.name)
                };
                while let Some(record) = next.as_ref() {
                    let sensitive = pcb.sensitive_segments.is_empty()
                        || pcb.sensitive_segments.contains(&record.segment);
                    if sensitive && ssas.first().is_none_or(|ssa| ssa.matches(record, &dbd)) {
                        break;
                    }
                    next = next_after(&self.transaction, &dbd, record);
                }
                if next.is_none() {
                    position.exhausted = true;
                }
                next
            }
            ReadOp::Gnp => {
                if ssas.len() > 1 {
                    return Err("GNP currently accepts at most one SSA".into());
                }
                let parent_id = position
                    .parent
                    .ok_or("GNP requires a current parent position")?;
                let parent = self
                    .transaction
                    .record(parent_id)
                    .ok_or("current parent was deleted")?;
                let child_segment = if let Some(ssa) = ssas.first() {
                    ssa.segment.as_str()
                } else {
                    dbd.segments
                        .iter()
                        .find(|s| s.parent.as_deref() == Some(&parent.segment))
                        .map(|s| s.name.as_str())
                        .ok_or("parent has no child segment")?
                };
                if dbd.segment(child_segment).and_then(|s| s.parent.as_deref())
                    != Some(parent.segment.as_str())
                {
                    return Err("GNP SSA does not name a child of the current parent".into());
                }
                let mut next = position
                    .current
                    .and_then(|id| {
                        self.transaction
                            .record(id)
                            .or_else(|| position.last_deleted.clone())
                    })
                    .filter(|record| {
                        record.parent == Some(parent_id) && record.segment == child_segment
                    })
                    .and_then(|record| self.transaction.next_sibling(&record))
                    .or_else(|| {
                        if position.current == Some(parent_id) {
                            self.transaction
                                .first_child(dbd_name, Some(parent_id), child_segment)
                        } else {
                            None
                        }
                    });
                while let Some(record) = next.as_ref() {
                    if ssas.first().is_none_or(|ssa| ssa.matches(record, &dbd)) {
                        break;
                    }
                    next = self.transaction.next_sibling(record);
                }
                next
            }
            ReadOp::Ghu | ReadOp::Ghn | ReadOp::Ghnp => unreachable!(),
        };
        if let Some(record) = found {
            let record = if update_hold {
                self.transaction
                    .hold(record.id, self.lock_timeout)
                    .map_err(|e| e.to_string())?
            } else {
                record
            };
            position.current = Some(record.id);
            position.parent = record.parent.or(Some(record.id));
            position.last_deleted = None;
            if update_hold {
                position.held = Some(record.id);
            }
            let status = if base == ReadOp::Gn && ssas.is_empty() {
                if let Some(previous) = previous {
                    let old_depth = depth(&dbd, &previous.segment);
                    let new_depth = depth(&dbd, &record.segment);
                    if new_depth < old_depth {
                        Status::HigherLevel
                    } else if new_depth == old_depth && record.segment != previous.segment {
                        Status::OtherSegmentType
                    } else {
                        Status::Ok
                    }
                } else {
                    Status::Ok
                }
            } else {
                Status::Ok
            };
            Ok(ReadOutcome::found(record, status))
        } else {
            Ok(ReadOutcome::empty(
                if base == ReadOp::Gn && ssas.iter().all(|ssa| ssa.condition.is_none()) {
                    Status::End
                } else {
                    Status::NotFound
                },
            ))
        }
    }

    fn read_procseq(
        &mut self,
        pcb_index: usize,
        index: &str,
        dbd_name: &str,
        op: ReadOp,
        raw_ssas: &[&[u8]],
    ) -> Result<ReadOutcome, String> {
        let (segment, field, length) = self
            .database
            .procseq_field(index, dbd_name)
            .map_err(|error| error.to_string())?;
        if raw_ssas.len() > 1 {
            return Err("PROCSEQ GU/GN accepts at most one root SSA".into());
        }
        let search = if let Some(raw) = raw_ssas.first() {
            if raw.len() < 8
                || std::str::from_utf8(&raw[..8])
                    .map_err(|_| "invalid SSA segment")?
                    .trim()
                    .to_ascii_uppercase()
                    != segment
            {
                return Err(format!("PROCSEQ SSA must name {segment}"));
            }
            if raw.get(8).is_none_or(|byte| *byte == b' ') {
                None
            } else {
                if raw.len() != 20 + length
                    || raw[8] != b'('
                    || &raw[17..19] != b"= " && &raw[17..19] != b"EQ"
                    || raw[19 + length] != b')'
                    || std::str::from_utf8(&raw[9..17])
                        .map_err(|_| "invalid SSA field")?
                        .trim()
                        .to_ascii_uppercase()
                        != field
                {
                    return Err(format!(
                        "PROCSEQ SSA requires {field} equality with {length} bytes"
                    ));
                }
                Some(&raw[19..19 + length])
            }
        } else {
            None
        };
        let pcb = &self.psb.pcbs[pcb_index];
        let procopt = pcb.procopt.as_deref().unwrap_or("");
        if !procopt.contains(['G', 'R', 'D', 'A']) {
            return Err(format!("PCB {pcb_index} does not permit get calls"));
        }
        let update_hold = op.is_hold() && procopt.contains(['R', 'D', 'A']);
        if !pcb.sensitive_segments.is_empty() && !pcb.sensitive_segments.contains(&segment) {
            return Err(format!("PCB is not sensitive to {segment}"));
        }
        let position = &mut self.positions[pcb_index];
        position.held = None;
        if op.base() == ReadOp::Gu {
            position.secondary = None;
            position.current = None;
            position.parent = None;
            position.exhausted = false;
        } else if position.exhausted {
            position.secondary = None;
            position.exhausted = false;
        }
        let after = if op.base() == ReadOp::Gn {
            position
                .secondary
                .as_ref()
                .map(|(key, id)| (key.as_slice(), *id))
        } else {
            None
        };
        let entry = self
            .transaction
            .secondary_entry(index, search, after)
            .map_err(|error| error.to_string())?;
        let Some((key, source_id, record)) = entry else {
            if op.base() == ReadOp::Gn {
                position.exhausted = true;
            }
            return Ok(ReadOutcome::empty(
                if search.is_some() || op.base() == ReadOp::Gu {
                    Status::NotFound
                } else {
                    Status::End
                },
            ));
        };
        let record = self
            .transaction
            .record(record.id)
            .ok_or("indexed target was deleted")?;
        let record = if update_hold {
            self.transaction
                .hold(record.id, self.lock_timeout)
                .map_err(|e| e.to_string())?
        } else {
            record
        };
        position.secondary = Some((key.clone(), source_id));
        position.current = Some(record.id);
        position.parent = Some(record.id);
        position.last_deleted = None;
        if update_hold {
            position.held = Some(record.id);
        }
        let mut outcome = ReadOutcome::found(record, Status::Ok);
        outcome.key_feedback = key;
        outcome.secondary_key = true;
        Ok(outcome)
    }

    fn read_index(
        &mut self,
        pcb_index: usize,
        index: &str,
        op: ReadOp,
        raw_ssas: &[&[u8]],
    ) -> Result<ReadOutcome, String> {
        if op.base() == ReadOp::Gnp {
            return Ok(ReadOutcome::empty(Status::NotAllowed));
        }
        if raw_ssas.len() > 1 {
            return Err("index PCB GU/GN accepts at most one pointer SSA".into());
        }
        let (pointer, key_field, _) = self
            .database
            .index_pointer(index)
            .map_err(|error| error.to_string())?;
        let pcb = &self.psb.pcbs[pcb_index];
        if !pcb.sensitive_segments.is_empty() && !pcb.sensitive_segments.contains(&pointer) {
            return Err(format!("PCB is not sensitive to {pointer}"));
        }
        let search = if let Some(raw) = raw_ssas.first() {
            let dbd = self.database.definition(index).ok_or("missing index DBD")?;
            let ssa = Ssa::parse(raw, &dbd)?;
            if ssa.segment != pointer {
                return Err(format!("index PCB SSA must name {pointer}"));
            }
            match ssa.condition {
                Some((field, value)) if field == key_field => Some(value),
                Some(_) => return Err(format!("index PCB SSA requires the full {key_field} key")),
                None => None,
            }
        } else {
            None
        };
        let position = &mut self.positions[pcb_index];
        position.held = None;
        if op.base() == ReadOp::Gu || position.exhausted {
            position.secondary = None;
            position.exhausted = false;
        }
        let after = if op.base() == ReadOp::Gn {
            position
                .secondary
                .as_ref()
                .map(|(key, id)| (key.as_slice(), *id))
        } else {
            None
        };
        let entry = self
            .transaction
            .secondary_pointer_entry(index, search.as_deref(), after)
            .map_err(|error| error.to_string())?;
        let Some((key, source_id, _)) = entry else {
            if op.base() == ReadOp::Gn {
                position.exhausted = true;
            }
            return Ok(ReadOutcome::empty(
                if search.is_some() || op.base() == ReadOp::Gu {
                    Status::NotFound
                } else {
                    Status::End
                },
            ));
        };
        position.secondary = Some((key.clone(), source_id));
        position.current = None;
        position.parent = None;
        position.last_deleted = None;
        let record = Record {
            id: source_id,
            dbd: index.to_string(),
            segment: pointer,
            parent: None,
            root_key: key.clone(),
            key: key.clone(),
            data: key,
        };
        Ok(ReadOutcome::found(record, Status::Ok))
    }

    fn read_senseg_index(
        &mut self,
        pcb_index: usize,
        index: &str,
        length: usize,
        op: ReadOp,
        raw: &[u8],
    ) -> Result<ReadOutcome, String> {
        if raw.len() != 20 + length
            || (&raw[17..19] != b"EQ" && &raw[17..19] != b"= ")
            || raw[19 + length] != b')'
        {
            return Err(format!("{index}: invalid secondary search SSA"));
        }
        let position = &mut self.positions[pcb_index];
        position.current = None;
        position.parent = None;
        position.held = None;
        position.exhausted = false;
        position.last_deleted = None;
        let entry = self
            .transaction
            .secondary_entry(index, Some(&raw[19..19 + length]), None)
            .map_err(|error| error.to_string())?;
        let Some((_, _, record)) = entry else {
            return Ok(ReadOutcome::empty(Status::NotFound));
        };
        let record = self
            .transaction
            .record(record.id)
            .ok_or("indexed target was deleted")?;
        let update_hold = op.is_hold()
            && self.psb.pcbs[pcb_index]
                .procopt
                .as_deref()
                .unwrap_or("")
                .contains(['R', 'D', 'A']);
        let record = if update_hold {
            self.transaction
                .hold(record.id, self.lock_timeout)
                .map_err(|error| error.to_string())?
        } else {
            record
        };
        position.current = Some(record.id);
        position.parent = Some(record.id);
        if update_hold {
            position.held = Some(record.id);
        }
        Ok(ReadOutcome::found(record, Status::Ok))
    }
}

fn delete_subtree(
    transaction: &mut Transaction,
    dbd: &Dbd,
    dbd_name: &str,
    id: u64,
    timeout: Duration,
) -> Result<(), String> {
    let record = transaction
        .record(id)
        .ok_or("segment to delete is missing")?;
    for child in dbd
        .segments
        .iter()
        .filter(|definition| definition.parent.as_deref() == Some(record.segment.as_str()))
    {
        for descendant in transaction.children(dbd_name, Some(id), &child.name) {
            delete_subtree(transaction, dbd, dbd_name, descendant.id, timeout)?;
        }
    }
    transaction
        .delete(id, timeout)
        .map_err(|error| error.to_string())
}

fn depth(dbd: &Dbd, segment: &str) -> usize {
    let mut result = 0;
    let mut current = dbd.segment(segment);
    while let Some(parent) = current.and_then(|s| s.parent.as_deref()) {
        result += 1;
        current = dbd.segment(parent);
    }
    result
}

fn find_match(
    transaction: &Transaction,
    dbd: &Dbd,
    dbd_name: &str,
    parent: Option<u64>,
    ssa: &Ssa,
) -> Option<Record> {
    let segment = dbd.segment(&ssa.segment)?;
    if let Some((field, value)) = &ssa.condition {
        if segment
            .key()
            .is_some_and(|key| &key.name == field && key.unique)
        {
            return transaction.get(dbd_name, parent, &ssa.segment, value);
        }
    }
    transaction
        .children(dbd_name, parent, &ssa.segment)
        .into_iter()
        .find(|r| ssa.matches(r, dbd))
}

fn next_after(transaction: &Transaction, dbd: &Dbd, record: &Record) -> Option<Record> {
    for child in dbd
        .segments
        .iter()
        .filter(|s| s.parent.as_deref() == Some(&record.segment))
    {
        if let Some(first) = transaction.first_child(&record.dbd, Some(record.id), &child.name) {
            return Some(first);
        }
    }
    let mut current = record.clone();
    loop {
        if let Some(next) = transaction.next_sibling(&current) {
            return Some(next);
        }
        let parent_id = current.parent?;
        let parent = transaction.record(parent_id)?;
        let mut after_current = false;
        for sibling in dbd
            .segments
            .iter()
            .filter(|s| s.parent.as_deref() == Some(&parent.segment))
        {
            if after_current {
                if let Some(first) =
                    transaction.first_child(&record.dbd, Some(parent_id), &sibling.name)
                {
                    return Some(first);
                }
            }
            if sibling.name == current.segment {
                after_current = true;
            }
        }
        current = parent;
    }
}
