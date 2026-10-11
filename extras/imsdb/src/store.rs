use crate::catalog::{Dbd, Segment};
use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum StoreError {
    Definition(String),
    Missing(String),
    Duplicate(String),
    Conflict(String),
    Timeout,
    Io(std::io::Error),
    Corrupt(String),
}
impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Definition(s)
            | Self::Missing(s)
            | Self::Duplicate(s)
            | Self::Conflict(s)
            | Self::Corrupt(s) => write!(f, "{s}"),
            Self::Timeout => write!(f, "root lock timeout"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for StoreError {}
impl From<std::io::Error> for StoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub id: u64,
    pub dbd: String,
    pub segment: String,
    pub parent: Option<u64>,
    pub root_key: Vec<u8>,
    pub key: Vec<u8>,
    pub data: Vec<u8>,
}

type IndexKey = (String, Option<u64>, String, Vec<u8>, u64);
type SecondaryKey = (String, Vec<u8>, u64);

#[derive(Clone)]
struct KeyPart {
    start: usize,
    bytes: usize,
}

#[derive(Clone)]
struct SecondaryDef {
    index_dbd: String,
    field_name: String,
    target_dbd: String,
    target_segment: String,
    source_segment: String,
    key_parts: Vec<KeyPart>,
    search_bytes: usize,
    unique: bool,
}
impl SecondaryDef {
    fn key(
        &self,
        record: &Record,
        records: &BTreeMap<u64, Record>,
    ) -> Result<Option<(SecondaryKey, u64)>, StoreError> {
        if record.dbd != self.target_dbd || record.segment != self.source_segment {
            return Ok(None);
        }
        let mut key = Vec::new();
        for part in &self.key_parts {
            let bytes = record
                .data
                .get(part.start - 1..part.start - 1 + part.bytes)
                .ok_or_else(|| {
                    StoreError::Definition(format!(
                        "{}/{} is too short for secondary index {}",
                        record.dbd, record.segment, self.index_dbd
                    ))
                })?;
            key.extend_from_slice(bytes);
        }
        let mut target = record;
        while target.segment != self.target_segment {
            let parent = target.parent.ok_or_else(|| {
                StoreError::Corrupt(format!(
                    "{}: index source has no target ancestor",
                    self.index_dbd
                ))
            })?;
            target = records.get(&parent).ok_or_else(|| {
                StoreError::Corrupt(format!(
                    "{}: index source parent {parent} is missing",
                    self.index_dbd
                ))
            })?;
        }
        Ok(Some(((self.index_dbd.clone(), key, record.id), target.id)))
    }
}

#[derive(Clone, Default)]
struct State {
    records: BTreeMap<u64, Record>,
    // BTreeMap lets GU and same-parent sequential access avoid full scans.
    index: BTreeMap<IndexKey, ()>,
    secondary_index: BTreeMap<SecondaryKey, u64>,
    secondary_defs: Vec<SecondaryDef>,
    next_id: u64,
    latest_checkpoint: Option<[u8; 8]>,
}
impl State {
    fn secondary_entries(
        &self,
        record: &Record,
    ) -> Result<Vec<(SecondaryKey, u64, bool)>, StoreError> {
        let mut entries = Vec::new();
        for definition in &self.secondary_defs {
            if let Some((key, target_id)) = definition.key(record, &self.records)? {
                entries.push((key, target_id, definition.unique));
            }
        }
        Ok(entries)
    }

    fn check_secondary_unique(
        &self,
        entries: &[(SecondaryKey, u64, bool)],
    ) -> Result<(), StoreError> {
        for ((dbd, key, id), _, unique) in entries {
            if !unique {
                continue;
            }
            let start = (dbd.clone(), key.clone(), 0);
            if self.secondary_index.range(start..).next().is_some_and(
                |((found_dbd, found_key, found_id), _)| {
                    found_dbd == dbd && found_key == key && found_id != id
                },
            ) {
                return Err(StoreError::Duplicate(format!(
                    "{dbd}: duplicate secondary key"
                )));
            }
        }
        Ok(())
    }

    fn insert(&mut self, record: Record) -> Result<(), StoreError> {
        if self.records.contains_key(&record.id) {
            return Err(StoreError::Corrupt(format!(
                "duplicate record id {}",
                record.id
            )));
        }
        let index_key = (
            record.dbd.clone(),
            record.parent,
            record.segment.clone(),
            record.key.clone(),
            record.id,
        );
        let secondary = self.secondary_entries(&record)?;
        self.check_secondary_unique(&secondary)?;
        self.next_id = self.next_id.max(record.id + 1);
        self.index.insert(index_key, ());
        for (key, target_id, _) in secondary {
            self.secondary_index.insert(key, target_id);
        }
        self.records.insert(record.id, record);
        Ok(())
    }
    fn remove(&mut self, id: u64) -> Result<Record, StoreError> {
        let record = self
            .records
            .get(&id)
            .ok_or_else(|| StoreError::Missing(format!("record {id} not found")))?;
        let secondary = self.secondary_entries(record)?;
        let record = self.records.remove(&id).unwrap();
        self.index.remove(&(
            record.dbd.clone(),
            record.parent,
            record.segment.clone(),
            record.key.clone(),
            record.id,
        ));
        for (key, _, _) in secondary {
            self.secondary_index.remove(&key);
        }
        Ok(record)
    }

    fn replace_data(
        &mut self,
        record: &Record,
        old_data: &[u8],
        replay: bool,
    ) -> Result<(), StoreError> {
        let current = self
            .records
            .get(&record.id)
            .ok_or_else(|| StoreError::Missing(format!("record {} not found", record.id)))?;
        if !replay && current.data != old_data {
            return Err(StoreError::Conflict(format!(
                "record {} changed concurrently",
                record.id
            )));
        }
        let old_secondary = self.secondary_entries(current)?;
        let new_secondary = self.secondary_entries(record)?;
        self.check_secondary_unique(&new_secondary)?;
        for (key, _, _) in old_secondary {
            self.secondary_index.remove(&key);
        }
        for (key, target_id, _) in new_secondary {
            self.secondary_index.insert(key, target_id);
        }
        self.records.get_mut(&record.id).unwrap().data = record.data.clone();
        Ok(())
    }
    fn get(&self, dbd: &str, parent: Option<u64>, segment: &str, key: &[u8]) -> Option<&Record> {
        let start = (
            dbd.to_string(),
            parent,
            segment.to_string(),
            key.to_vec(),
            0,
        );
        let ((d, p, s, k, id), _) = self.index.range(start..).next()?;
        if d != dbd || *p != parent || s != segment || k != key {
            return None;
        }
        self.records.get(id)
    }
    fn children(&self, dbd: &str, parent: Option<u64>, segment: &str) -> Vec<Record> {
        let prefix = (dbd.to_string(), parent, segment.to_string(), Vec::new(), 0);
        self.index
            .range(prefix..)
            .take_while(|((d, p, s, _, _), _)| d == dbd && *p == parent && s == segment)
            .filter_map(|((_, _, _, _, id), _)| self.records.get(id).cloned())
            .collect()
    }
    fn first_child(&self, dbd: &str, parent: Option<u64>, segment: &str) -> Option<Record> {
        let start = (dbd.to_string(), parent, segment.to_string(), Vec::new(), 0);
        let ((d, p, s, _, id), _) = self.index.range(start..).next()?;
        if d != dbd || *p != parent || s != segment {
            return None;
        }
        self.records.get(id).cloned()
    }
    fn next_sibling(&self, record: &Record) -> Option<Record> {
        let start = (
            record.dbd.clone(),
            record.parent,
            record.segment.clone(),
            record.key.clone(),
            record.id.checked_add(1)?,
        );
        let ((dbd, parent, segment, _, id), _) = self.index.range(start..).next()?;
        if dbd != &record.dbd || *parent != record.parent || segment != &record.segment {
            return None;
        }
        self.records.get(id).cloned()
    }
}

struct Inner {
    dbds: HashMap<String, Dbd>,
    state: Mutex<State>,
    wal: Mutex<File>,
    locks: Mutex<HashMap<(String, Vec<u8>), u64>>,
    lock_changed: Condvar,
    next_tx: Mutex<u64>,
}
#[derive(Clone)]
pub struct Database(Arc<Inner>);

#[derive(Clone, Debug)]
enum Change {
    Insert(Record),
    Replace { record: Record, old_data: Vec<u8> },
    Delete { record: Record },
}

pub struct Transaction {
    database: Database,
    id: u64,
    changes: Vec<Change>,
    locks: Vec<(String, Vec<u8>)>,
    checkpoint: Option<[u8; 8]>,
}
impl Drop for Transaction {
    fn drop(&mut self) {
        self.release();
    }
}
impl Transaction {
    pub fn secondary_entry(
        &self,
        index_dbd: &str,
        search: Option<&[u8]>,
        after: Option<(&[u8], u64)>,
    ) -> Result<Option<(Vec<u8>, u64, Record)>, StoreError> {
        self.secondary_entry_mode(index_dbd, search, after, false)
    }
    pub fn secondary_pointer_entry(
        &self,
        index_dbd: &str,
        search: Option<&[u8]>,
        after: Option<(&[u8], u64)>,
    ) -> Result<Option<(Vec<u8>, u64, Record)>, StoreError> {
        self.secondary_entry_mode(index_dbd, search, after, true)
    }
    fn secondary_entry_mode(
        &self,
        index_dbd: &str,
        search: Option<&[u8]>,
        after: Option<(&[u8], u64)>,
        complete_key: bool,
    ) -> Result<Option<(Vec<u8>, u64, Record)>, StoreError> {
        if self.changes.is_empty() {
            let state = self.database.0.state.lock().unwrap();
            return secondary_entry_in_state(&state, index_dbd, search, after, complete_key);
        }
        let mut view = self.database.0.state.lock().unwrap().clone();
        for change in &self.changes {
            apply(&mut view, change, false)?;
        }
        secondary_entry_in_state(&view, index_dbd, search, after, complete_key)
    }
    fn release(&mut self) {
        let mut locks = self.database.0.locks.lock().unwrap();
        for key in self.locks.drain(..) {
            if locks.get(&key) == Some(&self.id) {
                locks.remove(&key);
            }
        }
        self.database.0.lock_changed.notify_all();
    }
    fn lock_root(
        &mut self,
        dbd: &str,
        root_key: &[u8],
        timeout: Duration,
    ) -> Result<(), StoreError> {
        let key = (dbd.to_string(), root_key.to_vec());
        if self.locks.contains(&key) {
            return Ok(());
        }
        // Enforce a total order on roots, so two UOWs cannot deadlock.
        if self.locks.last().is_some_and(|last| last > &key) {
            return Err(StoreError::Conflict(
                "root locks must be acquired in ascending key order".into(),
            ));
        }
        let deadline = Instant::now() + timeout;
        let mut locks = self.database.0.locks.lock().unwrap();
        loop {
            if !locks.contains_key(&key) {
                locks.insert(key.clone(), self.id);
                self.locks.push(key);
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(StoreError::Timeout);
            }
            let (new_locks, result) = self
                .database
                .0
                .lock_changed
                .wait_timeout(locks, remaining)
                .unwrap();
            locks = new_locks;
            if result.timed_out() && locks.contains_key(&key) {
                return Err(StoreError::Timeout);
            }
        }
    }
    pub fn record(&self, id: u64) -> Option<Record> {
        for change in self.changes.iter().rev() {
            match change {
                Change::Insert(record) | Change::Replace { record, .. } if record.id == id => {
                    return Some(record.clone())
                }
                Change::Delete { record } if record.id == id => return None,
                _ => {}
            }
        }
        self.database.record(id)
    }
    pub fn children(&self, dbd: &str, parent: Option<u64>, segment: &str) -> Vec<Record> {
        if self.changes.is_empty() {
            return self.database.children(dbd, parent, segment);
        }
        let mut records: BTreeMap<u64, Record> = self
            .database
            .children(dbd, parent, segment)
            .into_iter()
            .map(|record| (record.id, record))
            .collect();
        for change in &self.changes {
            match change {
                Change::Insert(record) | Change::Replace { record, .. }
                    if record.dbd == dbd
                        && record.parent == parent
                        && record.segment == segment =>
                {
                    records.insert(record.id, record.clone());
                }
                Change::Delete { record } => {
                    records.remove(&record.id);
                }
                _ => {}
            }
        }
        let mut values: Vec<Record> = records.into_values().collect();
        values.sort_by(|a, b| (&a.key, a.id).cmp(&(&b.key, b.id)));
        values
    }
    pub fn get(&self, dbd: &str, parent: Option<u64>, segment: &str, key: &[u8]) -> Option<Record> {
        if self.changes.is_empty() {
            return self.database.get(dbd, parent, segment, key);
        }
        self.children(dbd, parent, segment)
            .into_iter()
            .find(|r| r.key == key)
    }
    pub fn first_child(&self, dbd: &str, parent: Option<u64>, segment: &str) -> Option<Record> {
        if self.changes.is_empty() {
            self.database.first_child(dbd, parent, segment)
        } else {
            self.children(dbd, parent, segment).into_iter().next()
        }
    }
    pub fn next_sibling(&self, record: &Record) -> Option<Record> {
        if self.changes.is_empty() {
            return self.database.next_sibling(record);
        }
        self.children(&record.dbd, record.parent, &record.segment)
            .into_iter()
            .find(|candidate| (&candidate.key, candidate.id) > (&record.key, record.id))
    }
    pub fn hold(&mut self, id: u64, timeout: Duration) -> Result<Record, StoreError> {
        let record = self
            .record(id)
            .ok_or_else(|| StoreError::Missing(format!("record {id} not found")))?;
        self.lock_root(&record.dbd, &record.root_key, timeout)?;
        self.record(id)
            .ok_or_else(|| StoreError::Missing(format!("record {id} was deleted while waiting")))
    }
    pub fn insert(
        &mut self,
        dbd: &str,
        segment: &str,
        parent: Option<u64>,
        data: Vec<u8>,
        timeout: Duration,
    ) -> Result<u64, StoreError> {
        if self
            .database
            .0
            .dbds
            .get(dbd)
            .is_some_and(|definition| definition.access == "INDEX")
        {
            return Err(StoreError::Definition(format!(
                "{dbd}: index segments are maintained from the target database"
            )));
        }
        let def = self.database.segment(dbd, segment)?;
        def.validate_data(&data).map_err(StoreError::Definition)?;
        let key = def
            .extract_key(&data)
            .map_err(StoreError::Definition)?
            .to_vec();
        let unique = def.key().is_some_and(|field| field.unique);
        let root_key = match (&def.parent, parent) {
            (None, None) => key.clone(),
            (Some(expected), Some(id)) => {
                let parent_record = self
                    .record(id)
                    .ok_or_else(|| StoreError::Missing(format!("parent {id} not found")))?;
                if &parent_record.segment != expected || parent_record.dbd != dbd {
                    return Err(StoreError::Definition(format!(
                        "{segment} requires parent {expected}"
                    )));
                }
                parent_record.root_key.clone()
            }
            _ => return Err(StoreError::Definition(format!("{segment}: invalid parent"))),
        };
        self.lock_root(dbd, &root_key, timeout)?;
        if unique && self.get(dbd, parent, segment, &key).is_some() {
            return Err(StoreError::Duplicate(format!("{segment}: duplicate key")));
        }
        let mut state = self.database.0.state.lock().unwrap();
        let id = state.next_id.max(1);
        state.next_id = id + 1;
        self.changes.push(Change::Insert(Record {
            id,
            dbd: dbd.into(),
            segment: segment.into(),
            parent,
            root_key,
            key,
            data,
        }));
        Ok(id)
    }
    pub fn replace(&mut self, id: u64, data: Vec<u8>, timeout: Duration) -> Result<(), StoreError> {
        let record = self
            .record(id)
            .ok_or_else(|| StoreError::Missing(format!("record {id} not found")))?;
        let def = self.database.segment(&record.dbd, &record.segment)?;
        def.validate_data(&data).map_err(StoreError::Definition)?;
        if def.extract_key(&data).map_err(StoreError::Definition)? != record.key {
            return Err(StoreError::Definition(
                "sequence key cannot change in REPL".into(),
            ));
        }
        self.lock_root(&record.dbd, &record.root_key, timeout)?;
        let old_data = record.data.clone();
        self.changes.push(Change::Replace {
            record: Record { data, ..record },
            old_data,
        });
        Ok(())
    }
    pub fn delete(&mut self, id: u64, timeout: Duration) -> Result<(), StoreError> {
        let record = self
            .record(id)
            .ok_or_else(|| StoreError::Missing(format!("record {id} not found")))?;
        self.lock_root(&record.dbd, &record.root_key, timeout)?;
        self.changes.push(Change::Delete { record });
        Ok(())
    }
    pub fn commit(mut self) -> Result<(), StoreError> {
        if self.changes.is_empty() && self.checkpoint.is_none() {
            self.release();
            return Ok(());
        }
        let mut payload = Vec::new();
        if let Some(id) = self.checkpoint {
            payload.extend_from_slice(&id);
        }
        payload.extend_from_slice(&encode_changes(&self.changes));
        let len = u32::try_from(payload.len())
            .map_err(|_| StoreError::Definition("transaction exceeds 4 GiB".into()))?;
        if payload.len() > 64 * 1024 * 1024 {
            return Err(StoreError::Definition(
                "transaction exceeds the 64 MiB WAL frame limit".into(),
            ));
        }
        let mut state = self.database.0.state.lock().unwrap();
        let mut wal = self.database.0.wal.lock().unwrap();
        let start = wal.seek(SeekFrom::End(0))?;
        let mut inverses = Vec::with_capacity(self.changes.len());
        for change in &self.changes {
            let inverse = match inverse(&state, change) {
                Ok(inverse) => inverse,
                Err(error) => {
                    undo(&mut state, &inverses);
                    return Err(error);
                }
            };
            if let Err(error) = apply(&mut state, change, false) {
                undo(&mut state, &inverses);
                return Err(error);
            }
            inverses.push(inverse);
        }
        let write_result = (|| -> std::io::Result<()> {
            wal.write_all(if self.checkpoint.is_some() {
                b"IMS2"
            } else {
                b"IMS1"
            })?;
            wal.write_all(&len.to_le_bytes())?;
            wal.write_all(&checksum(&payload).to_le_bytes())?;
            wal.write_all(&payload)?;
            wal.sync_data()
        })();
        if let Err(error) = write_result {
            undo(&mut state, &inverses);
            wal.set_len(start)?;
            wal.seek(SeekFrom::End(0))?;
            return Err(StoreError::Io(error));
        }
        if let Some(id) = self.checkpoint {
            state.latest_checkpoint = Some(id);
        }
        drop(wal);
        drop(state);
        self.release();
        Ok(())
    }
    pub fn rollback(self) {}

    pub fn set_checkpoint(&mut self, id: [u8; 8]) {
        self.checkpoint = Some(id);
    }
}

fn inverse(state: &State, change: &Change) -> Result<Change, StoreError> {
    match change {
        Change::Insert(record) => Ok(Change::Delete {
            record: record.clone(),
        }),
        Change::Replace { record, .. } => {
            let current = state
                .records
                .get(&record.id)
                .ok_or_else(|| StoreError::Missing(format!("record {} not found", record.id)))?;
            Ok(Change::Replace {
                record: current.clone(),
                old_data: Vec::new(),
            })
        }
        Change::Delete { record } => {
            let current = state
                .records
                .get(&record.id)
                .ok_or_else(|| StoreError::Missing(format!("record {} not found", record.id)))?;
            Ok(Change::Insert(current.clone()))
        }
    }
}
fn undo(state: &mut State, inverses: &[Change]) {
    for change in inverses.iter().rev() {
        apply(state, change, true).expect("inverse of staged IMS transaction must succeed");
    }
}

fn apply(state: &mut State, change: &Change, replay: bool) -> Result<(), StoreError> {
    match change {
        Change::Insert(record) => state.insert(record.clone()),
        Change::Replace { record, old_data } => state.replace_data(record, old_data, replay),
        Change::Delete { record } => {
            if !replay
                && state
                    .records
                    .get(&record.id)
                    .is_some_and(|current| current.data != record.data)
            {
                return Err(StoreError::Conflict(format!(
                    "record {} changed concurrently",
                    record.id
                )));
            }
            let start = (
                record.dbd.clone(),
                Some(record.id),
                String::new(),
                Vec::new(),
                0,
            );
            if state
                .index
                .range(start..)
                .next()
                .is_some_and(|((dbd, parent, _, _, _), _)| {
                    dbd == &record.dbd && *parent == Some(record.id)
                })
            {
                return Err(StoreError::Conflict(
                    "cannot delete parent with children".into(),
                ));
            }
            state.remove(record.id)?;
            Ok(())
        }
    }
}

fn secondary_definitions(dbds: &HashMap<String, Dbd>) -> Result<Vec<SecondaryDef>, StoreError> {
    let mut definitions = Vec::new();
    for index_dbd in dbds.values().filter(|dbd| dbd.access == "INDEX") {
        for link in &index_dbd.logical_children {
            let field_name = link.index_field.as_ref().ok_or_else(|| {
                StoreError::Definition(format!(
                    "{}: INDEX field is required for materialized index",
                    index_dbd.name
                ))
            })?;
            let target_dbd = dbds.get(&link.target_dbd).ok_or_else(|| {
                StoreError::Definition(format!(
                    "{}: missing target DBD {}",
                    index_dbd.name, link.target_dbd
                ))
            })?;
            let target = target_dbd.segment(&link.target_segment).ok_or_else(|| {
                StoreError::Definition(format!(
                    "{}: missing target segment {}",
                    index_dbd.name, link.target_segment
                ))
            })?;
            let pointer = index_dbd.segment(&link.source_segment).ok_or_else(|| {
                StoreError::Definition(format!(
                    "{}: missing pointer segment {}",
                    index_dbd.name, link.source_segment
                ))
            })?;
            let pointer_key = pointer.key().ok_or_else(|| {
                StoreError::Definition(format!(
                    "{}: pointer segment has no sequence key",
                    index_dbd.name
                ))
            })?;
            let indexed_field = target_dbd
                .indexed_fields
                .iter()
                .find(|field| field.name == *field_name && field.target_segment == target.name);
            let (source, search_names, subsequence_names) = if let Some(field) = indexed_field {
                let source = target_dbd.segment(&field.source_segment).ok_or_else(|| {
                    StoreError::Definition(format!(
                        "{}: missing XDFLD source segment {}",
                        target_dbd.name, field.source_segment
                    ))
                })?;
                (
                    source,
                    field.search_fields.clone(),
                    field.subsequence_fields.clone(),
                )
            } else {
                (target, vec![field_name.clone()], Vec::new())
            };
            let mut ancestor = source;
            while ancestor.name != target.name {
                let parent = ancestor.parent.as_ref().ok_or_else(|| {
                    StoreError::Definition(format!(
                        "{}: XDFLD source is not below target",
                        target_dbd.name
                    ))
                })?;
                ancestor = target_dbd.segment(parent).ok_or_else(|| {
                    StoreError::Definition(format!(
                        "{}: missing XDFLD ancestor {parent}",
                        target_dbd.name
                    ))
                })?;
            }
            let mut key_parts = Vec::new();
            let mut search_bytes = 0;
            for name in &search_names {
                let field = source
                    .fields
                    .iter()
                    .find(|field| &field.name == name)
                    .ok_or_else(|| {
                        StoreError::Definition(format!(
                            "{}: missing XDFLD search FIELD {name}",
                            target_dbd.name
                        ))
                    })?;
                if field.start - 1 + field.bytes > source.min_bytes {
                    return Err(StoreError::Definition(format!(
                        "{}: XDFLD field exceeds minimum record length",
                        target_dbd.name
                    )));
                }
                search_bytes += field.bytes;
                key_parts.push(KeyPart {
                    start: field.start,
                    bytes: field.bytes,
                });
            }
            for name in &subsequence_names {
                let field = source
                    .fields
                    .iter()
                    .find(|field| &field.name == name)
                    .ok_or_else(|| {
                        StoreError::Definition(format!(
                            "{}: missing XDFLD SUBSEQ FIELD {name}",
                            target_dbd.name
                        ))
                    })?;
                if field.start - 1 + field.bytes > source.min_bytes {
                    return Err(StoreError::Definition(format!(
                        "{}: XDFLD field exceeds minimum record length",
                        target_dbd.name
                    )));
                }
                key_parts.push(KeyPart {
                    start: field.start,
                    bytes: field.bytes,
                });
            }
            let key_bytes: usize = key_parts.iter().map(|part| part.bytes).sum();
            if key_bytes != pointer_key.bytes {
                return Err(StoreError::Definition(format!(
                    "{}: secondary index sequence key length mismatch",
                    index_dbd.name
                )));
            }
            if !target_dbd.logical_children.iter().any(|reverse| {
                reverse.source_segment == link.target_segment
                    && reverse.target_segment == link.source_segment
                    && reverse.target_dbd == index_dbd.name
            }) {
                return Err(StoreError::Definition(format!(
                    "{}: missing reciprocal LCHILD in {}",
                    index_dbd.name, target_dbd.name
                )));
            }
            definitions.push(SecondaryDef {
                index_dbd: index_dbd.name.clone(),
                field_name: field_name.clone(),
                target_dbd: target_dbd.name.clone(),
                target_segment: target.name.clone(),
                source_segment: source.name.clone(),
                key_parts,
                search_bytes,
                unique: pointer_key.unique,
            });
        }
    }
    Ok(definitions)
}

impl Database {
    pub fn open(path: impl AsRef<Path>, dbds: Vec<Dbd>) -> Result<Self, StoreError> {
        let mut definitions = HashMap::new();
        for dbd in dbds {
            if definitions.insert(dbd.name.clone(), dbd).is_some() {
                return Err(StoreError::Definition("duplicate DBD".into()));
            }
        }
        let secondary_defs = secondary_definitions(&definitions)?;
        let mut wal = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        wal.try_lock().map_err(|e| {
            StoreError::Conflict(format!(
                "database WAL is already open or cannot be locked: {e}"
            ))
        })?;
        wal.seek(SeekFrom::Start(0))?;
        let mut state = State {
            secondary_defs,
            ..State::default()
        };
        let mut valid_end = 0;
        loop {
            let mut header = [0u8; 12];
            let mut got = 0;
            while got < 12 {
                let n = wal.read(&mut header[got..])?;
                if n == 0 {
                    break;
                }
                got += n;
            }
            if got == 0 {
                break;
            }
            if got < 12 {
                break;
            } // Interrupted final write has no commit record.
            if &header[..4] != b"IMS1" && &header[..4] != b"IMS2" {
                return Err(StoreError::Corrupt("invalid WAL magic".into()));
            }
            let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
            if len > 64 * 1024 * 1024 {
                return Err(StoreError::Corrupt("WAL frame too large".into()));
            }
            let expected = u32::from_le_bytes(header[8..12].try_into().unwrap());
            let mut payload = vec![0; len];
            if let Err(e) = wal.read_exact(&mut payload) {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    break;
                }
                return Err(e.into());
            }
            if checksum(&payload) != expected {
                return Err(StoreError::Corrupt("WAL checksum mismatch".into()));
            }
            let checkpoint = if &header[..4] == b"IMS2" {
                let id: [u8; 8] = payload
                    .get(..8)
                    .ok_or_else(|| StoreError::Corrupt("short checkpoint WAL frame".into()))?
                    .try_into()
                    .unwrap();
                Some(id)
            } else {
                None
            };
            let changes = decode_changes(if checkpoint.is_some() {
                &payload[8..]
            } else {
                &payload
            })?;
            for change in changes {
                let record = match &change {
                    Change::Insert(r)
                    | Change::Delete { record: r }
                    | Change::Replace { record: r, .. } => r,
                };
                let def = definitions
                    .get(&record.dbd)
                    .and_then(|d: &Dbd| d.segment(&record.segment))
                    .ok_or_else(|| {
                        StoreError::Corrupt(format!(
                            "WAL references unknown {}/{}",
                            record.dbd, record.segment
                        ))
                    })?;
                def.validate_data(&record.data)
                    .map_err(StoreError::Corrupt)?;
                if def.extract_key(&record.data).map_err(StoreError::Corrupt)? != record.key {
                    return Err(StoreError::Corrupt(
                        "WAL record key differs from segment data".into(),
                    ));
                }
                if matches!(&change, Change::Insert(_)) {
                    match (record.parent, &def.parent) {
                        (None, None) if record.root_key == record.key => {}
                        (Some(parent_id), Some(parent_name)) => {
                            let parent = state.records.get(&parent_id).ok_or_else(|| {
                                StoreError::Corrupt("WAL child has no parent".into())
                            })?;
                            if &parent.segment != parent_name || parent.root_key != record.root_key
                            {
                                return Err(StoreError::Corrupt(
                                    "WAL parent or root key mismatch".into(),
                                ));
                            }
                        }
                        _ => return Err(StoreError::Corrupt("WAL hierarchy mismatch".into())),
                    }
                    if def.key().is_some_and(|field| field.unique)
                        && state
                            .get(&record.dbd, record.parent, &record.segment, &record.key)
                            .is_some()
                    {
                        return Err(StoreError::Corrupt("duplicate unique key in WAL".into()));
                    }
                }
                apply(&mut state, &change, true)?;
            }
            if let Some(id) = checkpoint {
                state.latest_checkpoint = Some(id);
            }
            valid_end = wal.stream_position()?;
        }
        wal.set_len(valid_end)?;
        wal.seek(SeekFrom::End(0))?;
        Ok(Self(Arc::new(Inner {
            dbds: definitions,
            state: Mutex::new(state),
            wal: Mutex::new(wal),
            locks: Mutex::new(HashMap::new()),
            lock_changed: Condvar::new(),
            next_tx: Mutex::new(1),
        })))
    }
    pub fn begin(&self) -> Transaction {
        let mut next = self.0.next_tx.lock().unwrap();
        let id = *next;
        *next += 1;
        Transaction {
            database: self.clone(),
            id,
            changes: Vec::new(),
            locks: Vec::new(),
            checkpoint: None,
        }
    }
    pub fn segment(&self, dbd: &str, segment: &str) -> Result<&Segment, StoreError> {
        self.0
            .dbds
            .get(dbd)
            .and_then(|d| d.segment(segment))
            .ok_or_else(|| StoreError::Definition(format!("unknown {dbd}/{segment}")))
    }
    pub fn record(&self, id: u64) -> Option<Record> {
        self.0.state.lock().unwrap().records.get(&id).cloned()
    }
    pub fn get(&self, dbd: &str, parent: Option<u64>, segment: &str, key: &[u8]) -> Option<Record> {
        self.0
            .state
            .lock()
            .unwrap()
            .get(dbd, parent, segment, key)
            .cloned()
    }
    pub fn children(&self, dbd: &str, parent: Option<u64>, segment: &str) -> Vec<Record> {
        self.0.state.lock().unwrap().children(dbd, parent, segment)
    }
    pub fn first_child(&self, dbd: &str, parent: Option<u64>, segment: &str) -> Option<Record> {
        self.0
            .state
            .lock()
            .unwrap()
            .first_child(dbd, parent, segment)
    }
    pub fn next_sibling(&self, record: &Record) -> Option<Record> {
        self.0.state.lock().unwrap().next_sibling(record)
    }
    pub fn definition(&self, name: &str) -> Option<Dbd> {
        self.0.dbds.get(name).cloned()
    }
    pub fn count(&self) -> usize {
        self.0.state.lock().unwrap().records.len()
    }
    pub fn secondary_lookup(
        &self,
        index_dbd: &str,
        key: &[u8],
    ) -> Result<Option<Record>, StoreError> {
        let state = self.0.state.lock().unwrap();
        let matching: Vec<_> = state
            .secondary_defs
            .iter()
            .filter(|definition| definition.index_dbd == index_dbd)
            .collect();
        if matching.is_empty() {
            return Err(StoreError::Definition(format!(
                "{index_dbd}: no materialized secondary index"
            )));
        }
        if !matching
            .iter()
            .any(|definition| definition.search_bytes == key.len())
        {
            return Err(StoreError::Definition(format!(
                "{index_dbd}: search key has the wrong length"
            )));
        }
        let start = (index_dbd.to_string(), key.to_vec(), 0);
        let result = state.secondary_index.range(start..).next().and_then(
            |((dbd, found_key, _), target_id)| {
                if dbd == index_dbd && found_key.starts_with(key) {
                    state.records.get(target_id).cloned()
                } else {
                    None
                }
            },
        );
        Ok(result)
    }

    /// A PROCSEQ PCB currently uses one root-target index definition.
    pub fn procseq_field(
        &self,
        index_dbd: &str,
        target_dbd: &str,
    ) -> Result<(String, String, usize), StoreError> {
        let state = self.0.state.lock().unwrap();
        let definitions: Vec<_> = state
            .secondary_defs
            .iter()
            .filter(|def| def.index_dbd == index_dbd && def.target_dbd == target_dbd)
            .collect();
        if definitions.len() != 1 {
            return Err(StoreError::Definition(format!(
                "{index_dbd}: PROCSEQ requires exactly one index for {target_dbd}"
            )));
        }
        let def = definitions[0];
        let target = self
            .0
            .dbds
            .get(target_dbd)
            .and_then(|dbd| dbd.segment(&def.target_segment));
        if target.is_none_or(|segment| segment.parent.is_some()) {
            return Err(StoreError::Definition(format!(
                "{index_dbd}: PROCSEQ currently requires a root target"
            )));
        }
        Ok((
            def.target_segment.clone(),
            def.field_name.clone(),
            def.search_bytes,
        ))
    }

    pub fn index_pointer(&self, index_dbd: &str) -> Result<(String, String, usize), StoreError> {
        let dbd = self
            .0
            .dbds
            .get(index_dbd)
            .ok_or_else(|| StoreError::Definition(format!("missing DBD {index_dbd}")))?;
        if dbd.access != "INDEX" {
            return Err(StoreError::Definition(format!(
                "{index_dbd}: not an index DBD"
            )));
        }
        let state = self.0.state.lock().unwrap();
        let definitions: Vec<_> = state
            .secondary_defs
            .iter()
            .filter(|def| def.index_dbd == index_dbd)
            .collect();
        if definitions.len() != 1 {
            return Err(StoreError::Definition(format!(
                "{index_dbd}: direct PCB requires exactly one index definition"
            )));
        }
        let pointer = dbd
            .segment(&dbd.logical_children[0].source_segment)
            .ok_or_else(|| {
                StoreError::Definition(format!("{index_dbd}: missing pointer segment"))
            })?;
        let key = pointer.key().ok_or_else(|| {
            StoreError::Definition(format!("{index_dbd}: pointer segment has no key"))
        })?;
        if pointer.parent.is_some()
            || dbd.segments.len() != 1
            || key.start != 1
            || key.bytes != pointer.min_bytes
            || pointer.min_bytes != pointer.max_bytes
        {
            return Err(StoreError::Definition(format!(
                "{index_dbd}: direct PCB requires a key-only fixed root pointer segment"
            )));
        }
        Ok((pointer.name.clone(), key.name.clone(), key.bytes))
    }

    pub fn secondary_entry(
        &self,
        index_dbd: &str,
        search: Option<&[u8]>,
        after: Option<(&[u8], u64)>,
    ) -> Result<Option<(Vec<u8>, u64, Record)>, StoreError> {
        let state = self.0.state.lock().unwrap();
        secondary_entry_in_state(&state, index_dbd, search, after, false)
    }

    pub fn latest_checkpoint(&self) -> Option<[u8; 8]> {
        self.0.state.lock().unwrap().latest_checkpoint
    }
}

fn secondary_entry_in_state(
    state: &State,
    index_dbd: &str,
    search: Option<&[u8]>,
    after: Option<(&[u8], u64)>,
    complete_key: bool,
) -> Result<Option<(Vec<u8>, u64, Record)>, StoreError> {
    let definition = state
        .secondary_defs
        .iter()
        .find(|def| def.index_dbd == index_dbd)
        .ok_or_else(|| StoreError::Definition(format!("{index_dbd}: no secondary index")))?;
    let required_length = if complete_key {
        definition.key_parts.iter().map(|part| part.bytes).sum()
    } else {
        definition.search_bytes
    };
    if search.is_some_and(|key| key.len() != required_length) {
        return Err(StoreError::Definition(format!(
            "{index_dbd}: search key has the wrong length"
        )));
    }
    let start = if let Some((key, id)) = after {
        (index_dbd.to_string(), key.to_vec(), id.saturating_add(1))
    } else {
        (
            index_dbd.to_string(),
            search.unwrap_or_default().to_vec(),
            0,
        )
    };
    for ((dbd, key, source_id), target_id) in state.secondary_index.range(start..) {
        if dbd != index_dbd
            || search.is_some_and(|needle| {
                if complete_key {
                    key != needle
                } else {
                    !key.starts_with(needle)
                }
            })
        {
            break;
        }
        if let Some(record) = state.records.get(target_id) {
            return Ok(Some((key.clone(), *source_id, record.clone())));
        }
    }
    Ok(None)
}

fn checksum(data: &[u8]) -> u32 {
    data.iter().fold(2_166_136_261u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(16_777_619)
    })
}
fn put_bytes(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
}
fn put_record(out: &mut Vec<u8>, record: &Record) {
    out.extend_from_slice(&record.id.to_le_bytes());
    put_bytes(out, record.dbd.as_bytes());
    put_bytes(out, record.segment.as_bytes());
    out.extend_from_slice(&record.parent.unwrap_or(0).to_le_bytes());
    put_bytes(out, &record.root_key);
    put_bytes(out, &record.key);
    put_bytes(out, &record.data);
}
fn encode_changes(changes: &[Change]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(changes.len() as u32).to_le_bytes());
    for change in changes {
        match change {
            Change::Insert(r) => {
                out.push(1);
                put_record(&mut out, r);
            }
            Change::Replace { record, .. } => {
                out.push(2);
                put_record(&mut out, record);
            }
            Change::Delete { record } => {
                out.push(3);
                put_record(&mut out, record);
            }
        }
    }
    out
}
struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Decoder<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], StoreError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| StoreError::Corrupt("WAL length overflow".into()))?;
        let result = self
            .data
            .get(self.pos..end)
            .ok_or_else(|| StoreError::Corrupt("truncated WAL transaction".into()))?;
        self.pos = end;
        Ok(result)
    }
    fn u32(&mut self) -> Result<u32, StoreError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, StoreError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> Result<Vec<u8>, StoreError> {
        let len = self.u32()? as usize;
        Ok(self.take(len)?.to_vec())
    }
    fn string(&mut self) -> Result<String, StoreError> {
        String::from_utf8(self.bytes()?)
            .map_err(|_| StoreError::Corrupt("non-UTF8 WAL metadata".into()))
    }
    fn record(&mut self) -> Result<Record, StoreError> {
        let id = self.u64()?;
        let dbd = self.string()?;
        let segment = self.string()?;
        let parent = match self.u64()? {
            0 => None,
            n => Some(n),
        };
        Ok(Record {
            id,
            dbd,
            segment,
            parent,
            root_key: self.bytes()?,
            key: self.bytes()?,
            data: self.bytes()?,
        })
    }
}
fn decode_changes(data: &[u8]) -> Result<Vec<Change>, StoreError> {
    let mut decoder = Decoder { data, pos: 0 };
    let count = decoder.u32()?;
    if count > 1_000_000 {
        return Err(StoreError::Corrupt("too many WAL changes".into()));
    }
    let mut changes = Vec::new();
    for _ in 0..count {
        let op = decoder.take(1)?[0];
        let record = decoder.record()?;
        changes.push(match op {
            1 => Change::Insert(record),
            2 => Change::Replace {
                record,
                old_data: Vec::new(),
            },
            3 => Change::Delete { record },
            _ => return Err(StoreError::Corrupt("unknown WAL operation".into())),
        });
    }
    if decoder.pos != data.len() {
        return Err(StoreError::Corrupt("trailing WAL bytes".into()));
    }
    Ok(changes)
}
