use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Layout {
    pub lrecl: usize,
    pub key_len: usize,
    pub key_offset: usize,
}

pub(super) fn metadata_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".jclsh-meta");
    PathBuf::from(name)
}

pub(super) fn read_layout(path: &Path) -> Result<Option<Layout>, String> {
    let sidecar = metadata_path(path);
    let text = match fs::read_to_string(&sidecar) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", sidecar.display())),
    };
    let fields: Vec<_> = text.trim().split('\t').collect();
    if fields.len() != 4 || fields[0] != "FB" {
        return Err(format!("{}: invalid record metadata", sidecar.display()));
    }
    let values: Vec<usize> = fields[1..]
        .iter()
        .map(|field| field.parse())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("{}: invalid record metadata", sidecar.display()))?;
    let layout = Layout {
        lrecl: values[0],
        key_len: values[1],
        key_offset: values[2],
    };
    if layout.lrecl == 0
        || layout.key_offset > layout.lrecl
        || layout.key_len > layout.lrecl - layout.key_offset
    {
        return Err(format!("{}: invalid record dimensions", sidecar.display()));
    }
    Ok(Some(layout))
}

pub(super) fn write_layout(path: &Path, layout: Layout) -> Result<(), String> {
    if layout.lrecl == 0
        || layout.key_offset > layout.lrecl
        || layout.key_len > layout.lrecl - layout.key_offset
    {
        return Err("invalid FB record dimensions".to_string());
    }
    let sidecar = metadata_path(path);
    fs::write(
        &sidecar,
        format!(
            "FB\t{}\t{}\t{}\n",
            layout.lrecl, layout.key_len, layout.key_offset
        ),
    )
    .map_err(|error| format!("{}: {error}", sidecar.display()))
}

pub(super) fn copy_layout(from: &Path, to: &Path) -> Result<(), String> {
    match read_layout(from)? {
        Some(layout) => write_layout(to, layout),
        None => remove_layout(to),
    }
}

pub(super) fn remove_layout(path: &Path) -> Result<(), String> {
    let sidecar = metadata_path(path);
    match fs::remove_file(&sidecar) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{}: {error}", sidecar.display())),
    }
}

pub(super) fn records(bytes: &[u8], layout: Option<Layout>) -> Result<Vec<&[u8]>, String> {
    if let Some(layout) = layout {
        if !bytes.len().is_multiple_of(layout.lrecl) {
            return Err(format!(
                "FB data length {} is not divisible by LRECL {}",
                bytes.len(),
                layout.lrecl
            ));
        }
        Ok(bytes.chunks_exact(layout.lrecl).collect())
    } else {
        Ok(bytes
            .split(|byte| *byte == b'\n')
            .filter(|record| !record.is_empty())
            .map(|record| record.strip_suffix(b"\r").unwrap_or(record))
            .collect())
    }
}

pub(super) fn normalize_fb(
    bytes: &[u8],
    layout: Layout,
    source_is_fb: bool,
) -> Result<Vec<u8>, String> {
    if source_is_fb {
        records(bytes, Some(layout))?;
        return Ok(bytes.to_vec());
    }
    let mut output = Vec::new();
    for record in records(bytes, None)? {
        if record.len() > layout.lrecl {
            return Err(format!(
                "FB input record length {} exceeds LRECL {}",
                record.len(),
                layout.lrecl
            ));
        }
        output.extend_from_slice(record);
        output.resize(output.len() + layout.lrecl - record.len(), b' ');
    }
    Ok(output)
}

pub(super) fn dcb_layout(dcb: &str) -> Result<Option<Layout>, String> {
    let upper = dcb
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .to_ascii_uppercase();
    if !upper.contains("RECFM=FB") {
        return Ok(None);
    }
    let lrecl = upper
        .split(',')
        .find_map(|part| part.trim().strip_prefix("LRECL="))
        .ok_or("DCB RECFM=FB requires LRECL")?
        .parse()
        .map_err(|_| "DCB LRECL must be a positive integer")?;
    Ok(Some(Layout {
        lrecl,
        key_len: 0,
        key_offset: 0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_raw_fb_from_line_input_even_when_byte_lengths_match() {
        let layout = Layout {
            lrecl: 4,
            key_len: 0,
            key_offset: 0,
        };
        assert_eq!(normalize_fb(b"A\nBB", layout, true).unwrap(), b"A\nBB");
        assert_eq!(normalize_fb(b"A\nBB", layout, false).unwrap(), b"A   BB  ");
        assert!(normalize_fb(b"ABCDE", layout, true).is_err());
    }
}
