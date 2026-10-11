//! Byte-preserving interchange between a raw fixed-block file and jclsh's
//! local FB/VSAM representation (raw data plus a .jclsh-meta sidecar).
use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layout {
    lrecl: usize,
    key_len: usize,
    key_offset: usize,
}

impl Layout {
    fn validate(self) -> Result<Self> {
        if self.lrecl == 0 {
            return Err("LRECL must be greater than zero".into());
        }
        if self.lrecl > 64 * 1024 * 1024 {
            return Err("LRECL exceeds the local VSAM size limit".into());
        }
        if !matches!(self.key_offset.checked_add(self.key_len), Some(n) if n <= self.lrecl) {
            return Err("key extends beyond LRECL".into());
        }
        Ok(self)
    }
}

fn meta_path(data: &Path) -> PathBuf {
    let mut name: OsString = data.as_os_str().to_owned();
    name.push(".jclsh-meta");
    PathBuf::from(name)
}

fn read_layout(data: &Path) -> Result<Layout> {
    let content = fs::read_to_string(meta_path(data))?;
    let parts: Vec<&str> = content.trim().split('\t').collect();
    if parts.len() != 4 || parts[0] != "FB" {
        return Err("expected FB<TAB>LRECL<TAB>KEYLEN<TAB>KEYOFFSET metadata".into());
    }
    Layout {
        lrecl: parts[1].parse()?,
        key_len: parts[2].parse()?,
        key_offset: parts[3].parse()?,
    }
    .validate()
}

fn temp_path(dest: &Path) -> Result<PathBuf> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let mut name = dest.as_os_str().to_owned();
    name.push(format!(".fbconvert-{}-{nonce}.tmp", std::process::id()));
    Ok(PathBuf::from(name))
}

fn ensure_new(path: &Path) -> Result<()> {
    if path.exists() {
        return Err(format!("destination already exists: {}", path.display()).into());
    }
    Ok(())
}

fn copy_records(source: &Path, target: &Path, layout: Layout) -> Result<u64> {
    let mut input = File::open(source)?;
    let source_info = input.metadata()?;
    if !source_info.is_file() {
        return Err("source is not a regular file".into());
    }
    if layout.key_len != 0 && source_info.len() > 64 * 1024 * 1024 {
        return Err("keyed file exceeds the local VSAM size limit".into());
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let mut seen = HashSet::new();
    let mut record = vec![0u8; layout.lrecl];
    let mut count = 0u64;
    loop {
        let mut filled = 0;
        while filled < record.len() {
            match input.read(&mut record[filled..])? {
                0 if filled == 0 => {
                    output.sync_all()?;
                    return Ok(count);
                }
                0 => {
                    return Err(format!(
                        "incomplete record {}: got {filled} of {} bytes",
                        count + 1,
                        record.len()
                    )
                    .into())
                }
                n => filled += n,
            }
        }
        if layout.key_len != 0 {
            let key = record[layout.key_offset..layout.key_offset + layout.key_len].to_vec();
            if !seen.insert(key) {
                return Err(format!("duplicate key in record {}", count + 1).into());
            }
        }
        output.write_all(&record)?;
        count += 1;
    }
}

fn export(dataset: &Path, output: &Path, expected_lrecl: Option<usize>) -> Result<u64> {
    let layout = read_layout(dataset)?;
    if matches!(expected_lrecl, Some(n) if n != layout.lrecl) {
        return Err("requested LRECL differs from dataset metadata".into());
    }
    ensure_new(output)?;
    let staged = temp_path(output)?;
    let result: Result<u64> = (|| {
        let count = copy_records(dataset, &staged, layout)?;
        fs::hard_link(&staged, output)?;
        Ok(count)
    })();
    let _ = fs::remove_file(staged);
    result
}

fn import(source: &Path, dataset: &Path, layout: Layout) -> Result<u64> {
    let layout = layout.validate()?;
    let metadata = meta_path(dataset);
    ensure_new(dataset)?;
    ensure_new(&metadata)?;
    let staged_data = temp_path(dataset)?;
    let staged_meta = temp_path(&metadata)?;
    let result: Result<u64> = (|| {
        let count = copy_records(source, &staged_data, layout)?;
        let mut meta = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged_meta)?;
        writeln!(
            meta,
            "FB\t{}\t{}\t{}",
            layout.lrecl, layout.key_len, layout.key_offset
        )?;
        meta.sync_all()?;
        fs::hard_link(&staged_data, dataset)?;
        if let Err(error) = fs::hard_link(&staged_meta, &metadata) {
            let _ = fs::remove_file(dataset);
            return Err(error.into());
        }
        Ok(count)
    })();
    let _ = fs::remove_file(staged_data);
    let _ = fs::remove_file(staged_meta);
    result
}

fn usage() -> &'static str {
    "Usage:\n  fbconvert import <raw-file> <dataset> --lrecl N [--key-length N --key-offset N]\n  fbconvert export <dataset> <raw-file> [--lrecl N]\n\nBoth files contain exactly LRECL bytes per record, with no delimiters.\nImport writes .jclsh-meta; export writes data without a sidecar. Existing destinations are never replaced."
}

fn run() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let mode = args.next().ok_or_else(|| usage().to_string())?;
    if mode.to_str() == Some("--help") || mode.to_str() == Some("-h") {
        println!("{}", usage());
        return Ok(());
    }
    let source = PathBuf::from(args.next().ok_or_else(|| usage().to_string())?);
    let target = PathBuf::from(args.next().ok_or_else(|| usage().to_string())?);
    let mut lrecl = None;
    let mut key_len = None;
    let mut key_offset = None;
    while let Some(option) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {:?}", option))?;
        let number: usize = value.to_string_lossy().parse()?;
        match option.to_str() {
            Some("--lrecl") if lrecl.replace(number).is_none() => {}
            Some("--key-length") if key_len.replace(number).is_none() => {}
            Some("--key-offset") if key_offset.replace(number).is_none() => {}
            _ => return Err(format!("unknown or duplicate option: {:?}", option).into()),
        }
    }
    let count = match mode.to_str() {
        Some("import") => import(
            &source,
            &target,
            Layout {
                lrecl: lrecl.ok_or("import requires --lrecl")?,
                key_len: key_len.unwrap_or(0),
                key_offset: key_offset.unwrap_or(0),
            },
        )?,
        Some("export") if key_len.is_none() && key_offset.is_none() => {
            export(&source, &target, lrecl)?
        }
        _ => return Err(usage().into()),
    };
    println!("{count} records converted");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("fbconvert: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> PathBuf {
        let path = temp_path(&env::temp_dir().join("fbconvert-test")).unwrap();
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn binary_round_trip_preserves_every_byte() {
        let dir = directory();
        let input = dir.join("external.fb");
        let dataset = dir.join("local.vsam");
        let output = dir.join("recovered.fb");
        let bytes = [0xc1, 0x00, 0x12, 0x3c, 0xc2, 0xff, 0x0a, 0x0d];
        fs::write(&input, bytes).unwrap();
        let layout = Layout {
            lrecl: 4,
            key_len: 1,
            key_offset: 0,
        };
        assert_eq!(import(&input, &dataset, layout).unwrap(), 2);
        assert_eq!(read_layout(&dataset).unwrap(), layout);
        assert_eq!(export(&dataset, &output, Some(4)).unwrap(), 2);
        assert_eq!(fs::read(output).unwrap(), bytes);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_partial_records_without_publishing_dataset() {
        let dir = directory();
        let input = dir.join("bad.fb");
        let dataset = dir.join("local.vsam");
        fs::write(&input, [1, 2, 3]).unwrap();
        assert!(import(
            &input,
            &dataset,
            Layout {
                lrecl: 2,
                key_len: 0,
                key_offset: 0
            }
        )
        .is_err());
        assert!(!dataset.exists());
        assert!(!meta_path(&dataset).exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_duplicate_keys_and_existing_outputs() {
        let dir = directory();
        let input = dir.join("bad.fb");
        let dataset = dir.join("local.vsam");
        fs::write(&input, [1, 2, 1, 3]).unwrap();
        assert!(import(
            &input,
            &dataset,
            Layout {
                lrecl: 2,
                key_len: 1,
                key_offset: 0
            }
        )
        .is_err());
        fs::write(&dataset, b"existing").unwrap();
        assert!(import(
            &input,
            &dataset,
            Layout {
                lrecl: 2,
                key_len: 0,
                key_offset: 0
            }
        )
        .is_err());
        assert_eq!(fs::read(&dataset).unwrap(), b"existing");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn export_rejects_bad_length_and_does_not_replace_output() {
        let dir = directory();
        let dataset = dir.join("local.dat");
        let output = dir.join("external.fb");
        fs::write(&dataset, [0xc1, 0x12, 0x3c]).unwrap();
        fs::write(meta_path(&dataset), b"FB\t2\t0\t0\n").unwrap();
        assert!(export(&dataset, &output, None).is_err());
        assert!(!output.exists());
        fs::write(&dataset, [0xc1, 0x12]).unwrap();
        fs::write(&output, b"existing").unwrap();
        assert!(export(&dataset, &output, None).is_err());
        assert_eq!(fs::read(&output).unwrap(), b"existing");
        fs::remove_dir_all(dir).unwrap();
    }
}
