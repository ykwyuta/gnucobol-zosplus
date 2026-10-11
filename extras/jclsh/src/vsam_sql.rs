//! Database storage for IDCAMS key-sequenced clusters. The catalog sidecar
//! stays local; K/V record bytes live only in the selected SQL table.
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use super::records::Layout;
use super::{checked_file, container_path, docker_host_path, Profile, Result};

pub(super) fn valid_table(table: &str) -> bool {
    let parts: Vec<_> = table.split('.').collect();
    table.len() < 64
        && (1..=2).contains(&parts.len())
        && parts.iter().all(|part| {
            let mut bytes = part.bytes();
            matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic() || b == b'_')
                && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 15) as usize] as char);
    }
    result
}

fn unhex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return Err("odd SQL hex record".to_string());
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char)
                .to_digit(16)
                .ok_or("invalid SQL hex record")?;
            let low = (pair[1] as char)
                .to_digit(16)
                .ok_or("invalid SQL hex record")?;
            Ok(((high << 4) | low) as u8)
        })
        .collect()
}

fn postgres(profile: &Profile, sql: &str) -> Result<String> {
    let mut command = Command::new("docker");
    command.args([
        "exec",
        "-i",
        &profile.vsam_postgres_container,
        "psql",
        "-X",
        "-q",
        "-A",
        "-t",
        "-F",
        "|",
        "-v",
        "ON_ERROR_STOP=1",
        "-U",
        &profile.vsam_postgres_user,
        "-d",
        &profile.vsam_postgres_database,
        "-f",
        "-",
    ]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("PostgreSQL docker exec: {error}"))?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(sql.as_bytes())
        .map_err(|error| format!("PostgreSQL input: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("PostgreSQL wait: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "PostgreSQL VSAM: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("PostgreSQL output: {error}"))
}

fn db2(
    profile: &Profile,
    workspace: &Path,
    verb: &str,
    table: &str,
    layout: Option<Layout>,
    path: Option<&Path>,
) -> Result<String> {
    let env_file = checked_file(workspace, &profile.env_file, "Db2 env file")?;
    let helper = checked_file(workspace, &profile.vsam_db2_helper, "Db2 VSAM helper")?;
    let helper_path = container_path(workspace, &helper)?;
    let mut command = Command::new("docker");
    command
        .args(["run", "--rm", "--env-file"])
        .arg(docker_host_path(&env_file))
        .args(["-e", &format!("DB2_SCHEMA={}", profile.schema), "-v"])
        .arg(format!("{}:/src", docker_host_path(workspace)))
        .args([
            "-w",
            "/src",
            "--entrypoint",
            "python3",
            &profile.vsam_db2_image,
            &helper_path,
            verb,
            table,
        ]);
    if let Some(layout) = layout {
        command.args([
            layout.lrecl.to_string(),
            layout.key_len.to_string(),
            layout.key_offset.to_string(),
        ]);
    }
    if let Some(path) = path {
        command.arg(container_path(workspace, path)?);
    }
    let output = command
        .output()
        .map_err(|error| format!("Db2 docker run: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Db2 VSAM: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("Db2 output: {error}"))
}

pub(super) fn create(
    profile: &Profile,
    workspace: &Path,
    table: &str,
    layout: Layout,
) -> Result<()> {
    if layout.key_len == 0
        || layout
            .key_offset
            .checked_add(layout.key_len)
            .is_none_or(|end| end > layout.lrecl)
    {
        return Err("SQL VSAM requires a valid keyed FB layout".to_string());
    }
    match profile.vsam_backend.as_str() {
        "postgres" => {
            postgres(
                profile,
                &format!("CREATE TABLE {table} (K BYTEA PRIMARY KEY, V BYTEA NOT NULL);\n"),
            )?;
        }
        "db2" => {
            db2(profile, workspace, "create", table, Some(layout), None)?;
        }
        _ => return Err("SQL VSAM backend not selected".to_string()),
    }
    Ok(())
}

pub(super) fn drop_table(profile: &Profile, workspace: &Path, table: &str) -> Result<()> {
    match profile.vsam_backend.as_str() {
        "postgres" => {
            postgres(profile, &format!("DROP TABLE {table};\n"))?;
        }
        "db2" => {
            db2(profile, workspace, "drop", table, None, None)?;
        }
        _ => return Err("SQL VSAM backend not selected".to_string()),
    }
    Ok(())
}

pub(super) fn replace(
    profile: &Profile,
    workspace: &Path,
    table: &str,
    layout: Layout,
    bytes: &[u8],
) -> Result<()> {
    if layout.lrecl == 0 || !bytes.len().is_multiple_of(layout.lrecl) {
        return Err("SQL VSAM input is not fixed-length FB".to_string());
    }
    match profile.vsam_backend.as_str() {
        "postgres" => {
            let mut sql = format!("BEGIN;\nDELETE FROM {table};\n");
            for record in bytes.chunks_exact(layout.lrecl) {
                let key = &record[layout.key_offset..layout.key_offset + layout.key_len];
                sql.push_str(&format!(
                    "INSERT INTO {table} (K,V) VALUES (decode('{}','hex'),decode('{}','hex'));\n",
                    hex(key),
                    hex(record)
                ));
            }
            sql.push_str("COMMIT;\n");
            postgres(profile, &sql)?;
        }
        "db2" => {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            let path = workspace.join(format!(
                "build/jclsh/sql-vsam-import-{}-{nonce}.dat",
                std::process::id()
            ));
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(&path, bytes).map_err(|e| e.to_string())?;
            let result = db2(
                profile,
                workspace,
                "replace",
                table,
                Some(layout),
                Some(&path),
            );
            let _ = fs::remove_file(&path);
            result?;
        }
        _ => return Err("SQL VSAM backend not selected".to_string()),
    }
    Ok(())
}

pub(super) fn export(
    profile: &Profile,
    workspace: &Path,
    table: &str,
    layout: Layout,
) -> Result<Vec<u8>> {
    let text = match profile.vsam_backend.as_str() {
        "postgres" => postgres(
            profile,
            &format!("SELECT encode(K,'hex'),encode(V,'hex') FROM {table} ORDER BY K;\n"),
        )?,
        "db2" => db2(profile, workspace, "export", table, None, None)?,
        _ => return Err("SQL VSAM backend not selected".to_string()),
    };
    let mut result = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let (key, record) = line.split_once('|').ok_or("malformed SQL VSAM export")?;
        let key = unhex(key)?;
        let record = unhex(record)?;
        if key.len() != layout.key_len
            || record.len() != layout.lrecl
            || record[layout.key_offset..layout.key_offset + layout.key_len] != key
        {
            return Err("SQL VSAM row violates keyed FB layout".to_string());
        }
        result.extend(record);
    }
    Ok(result)
}
