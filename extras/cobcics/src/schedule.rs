//! Dispatch committed CICS START requests from the shared storage directory.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::bms::Map;
use crate::server::WorkerRouting;
use crate::tn3270::Input;
use crate::worker::{self, Session};

fn storage_dir() -> PathBuf {
    if let Some(value) = std::env::var_os("COBCICS_STORAGE_DIR") {
        if !value.is_empty() {
            return PathBuf::from(value);
        }
    }
    std::env::temp_dir()
}

fn current_time() -> u64 {
    if let Ok(value) = std::env::var("COBCICS_NOW") {
        if let Ok(parsed) = value.parse::<u64>() {
            return parsed;
        }
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn due_time(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    let body = name
        .strip_prefix("cobcics-start-")?
        .strip_suffix(".task")?;
    let mut components = body.split('-');
    let due = components.next()?.parse::<u64>().ok()?;
    components.next()?.parse::<u32>().ok()?;
    components.next()?.parse::<u32>().ok()?;
    if components.next().is_some() {
        return None;
    }
    Some(due)
}

fn claim_due(directory: &Path, now: u64) -> Result<Vec<PathBuf>, String> {
    let mut claimed = Vec::new();
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(claimed),
        Err(error) => return Err(format!("read scheduled tasks: {error}")),
    };
    for entry in entries {
        let entry = entry.map_err(|error| format!("scheduled task entry: {error}"))?;
        let path = entry.path();
        if !due_time(&path).is_some_and(|due| due <= now) {
            continue;
        }
        let target = path.with_extension("run");
        match fs::rename(&path, &target) {
            Ok(()) => claimed.push(target),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("claim {}: {error}", path.display())),
        }
    }
    Ok(claimed)
}

fn task_transid(path: &Path) -> Result<[u8; 4], String> {
    let raw = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    if raw.len() != 4 {
        return Err(format!("invalid scheduled task length: {}", path.display()));
    }
    let name = std::str::from_utf8(&raw)
        .map_err(|_| format!("invalid scheduled TRANSID: {}", path.display()))?
        .trim_end();
    let parsed = worker::transid(name)?;
    if raw != parsed {
        return Err(format!("invalid scheduled TRANSID padding: {}", path.display()));
    }
    Ok(parsed)
}

pub fn dispatch_due(directory: &Path, now: u64, routing: &WorkerRouting,
                    default_map: &Map) -> Result<usize, String> {
    let mut completed = 0;
    for path in claim_due(directory, now)? {
        let result = (|| {
            let transid = task_transid(&path)?;
            let worker_path = routing
                .worker_for(&transid)?
                .ok_or_else(|| "scheduled TRANSID has no worker".to_owned())?;
            let mut map = routing.maps.get(&transid)
                .cloned()
                .unwrap_or_else(|| default_map.clone());
            let outcome = worker::run(
                worker_path,
                &mut map,
                &Input { aid: 0, cursor: None, fields: Vec::new() },
                &Session { transid, commarea: Vec::new() },
                &routing.applid,
            )?;
            if outcome.next.is_some() {
                return Err("scheduled task returned a terminal TRANSID".to_owned());
            }
            Ok::<(), String>(())
        })();
        match result {
            Ok(()) => {
                fs::remove_file(&path)
                    .map_err(|error| format!("remove {}: {error}", path.display()))?;
                completed += 1;
            }
            Err(error) => {
                let failed = path.with_extension("failed");
                fs::rename(&path, &failed)
                    .map_err(|io| format!("mark {} failed: {io}", path.display()))?;
                eprintln!("cobcicsd: scheduled task {}: {error}", failed.display());
            }
        }
    }
    Ok(completed)
}

pub fn start(routing: WorkerRouting, default_map: Map) {
    std::thread::spawn(move || {
        let directory = storage_dir();
        loop {
            if let Err(error) = dispatch_due(&directory, current_time(), &routing, &default_map) {
                eprintln!("cobcicsd: scheduler: {error}");
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_only_due_start_files() {
        let directory = std::env::temp_dir().join(format!(
            "cobcics-schedule-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let due = directory.join("cobcics-start-1060-1-1.task");
        let future = directory.join("cobcics-start-1120-1-2.task");
        fs::write(&due, b"SSST").unwrap();
        fs::write(&future, b"SSST").unwrap();
        let claimed = claim_due(&directory, 1060).unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(task_transid(&claimed[0]).unwrap(), *b"SSST");
        assert!(future.exists());
        fs::remove_file(&claimed[0]).unwrap();
        fs::remove_file(&future).unwrap();
        fs::remove_dir(&directory).unwrap();
    }
}
