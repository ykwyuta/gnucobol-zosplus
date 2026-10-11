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
            record: Some(record),
        }
    }
    fn empty(status: Status) -> Self {
        Self {
            status,
            record: None,
            segment_name: None,
            key_feedback: Vec::new(),
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
    pub fn new(database: Database, psb: Psb) -> Result<Self, String> {
        let dbds: Vec<Dbd> = psb
            .pcbs
            .iter()
            .filter_map(|pcb| pcb.dbd_name.as_deref())
            .filter_map(|name| database.definition(name))
            .collect();
        psb.validate(&dbds)?;
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
        let dbd_name = pcb.dbd_name.as_deref().ok_or("DB PCB has no DBDNAME")?;
        let dbd = self.database.definition(dbd_name).ok_or("missing DBD")?;
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
            let record = if op.is_hold() {
                self.transaction
                    .hold(record.id, self.lock_timeout)
                    .map_err(|e| e.to_string())?
            } else {
                record
            };
            position.current = Some(record.id);
            position.parent = record.parent.or(Some(record.id));
            position.last_deleted = None;
            if op.is_hold() {
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
