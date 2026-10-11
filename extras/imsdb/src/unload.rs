//! Strict reader for the non-HALDB DFSURGU0 (DFSURGUF) unload variant used
//! by CardDemo. The VB RDW is retained in downloaded sequential files.
use crate::call_adapter::PcbEncoding;
use crate::Dbd;

pub struct UnloadedSegment {
    pub segment: String,
    pub parent_index: Option<usize>,
    pub data: Vec<u8>,
}

fn be16(bytes: &[u8]) -> usize {
    usize::from(u16::from_be_bytes([bytes[0], bytes[1]]))
}

pub fn parse_dfsurgu0(input: &[u8], dbd: &Dbd) -> Result<Vec<UnloadedSegment>, String> {
    if dbd.access != "HIDAM" && dbd.access != "HDAM" {
        return Err("DFSURGU0 import currently requires HDAM or HIDAM".into());
    }
    let roots: Vec<_> = dbd.segments.iter().filter(|s| s.parent.is_none()).collect();
    if roots.len() != 1
        || dbd.segments.iter().any(|s| {
            s.parent
                .as_deref()
                .is_some_and(|parent| parent != roots[0].name)
        })
    {
        return Err("DFSURGU0 import currently requires one root and direct children".into());
    }
    let mut offset = 0;
    let mut records = Vec::new();
    let mut current_root = None;
    let mut saw_header = false;
    let mut saw_trailer = false;
    while offset < input.len() {
        if input.len() - offset < 4 {
            return Err(format!("truncated RDW at offset {offset}"));
        }
        let length = be16(&input[offset..offset + 2]);
        if length < 4 || length > input.len() - offset || input[offset + 2..offset + 4] != [0, 0] {
            return Err(format!("invalid VB record at offset {offset}"));
        }
        let record = &input[offset..offset + length];
        offset += length;
        if !saw_header {
            if length != 88 || record[4..6] != [0, 0x80] {
                return Err("DFSURGU0 header record is missing or unsupported".into());
            }
            saw_header = true;
            continue;
        }
        if length == 88 && record[4..6] == [0, 0x98] {
            if offset != input.len() {
                return Err("DFSURGU0 trailer is not the final record".into());
            }
            saw_trailer = true;
            break;
        }
        if length < 40
            || record[5] != 0x80
            || record[6..8] != [0, 0x23]
            || be16(&record[8..10]) != length - 40
        {
            return Err(format!(
                "unsupported DFSURGUF prefix at offset {}",
                offset - length
            ));
        }
        let segment_code = usize::from(record[4]);
        let definition = segment_code
            .checked_sub(1)
            .and_then(|index| dbd.segments.get(index))
            .ok_or_else(|| format!("unknown DFSURGUF segment code {segment_code}"))?;
        let name: Vec<u8> = record[10..18]
            .iter()
            .map(|byte| PcbEncoding::Cp1047.decode_control(*byte))
            .collect();
        if name != format!("{:<8}", definition.name).as_bytes() {
            return Err(format!(
                "DFSURGUF segment name does not match code {segment_code}"
            ));
        }
        // DFSURGUF uses a 39-byte prefix followed by the segment and one
        // zero pad byte. The RDW record length is still segment length + 40.
        if record[length - 1] != 0 {
            return Err(format!("invalid DFSURGUF pad at offset {}", offset - 1));
        }
        let data = record[39..length - 1].to_vec();
        definition.validate_data(&data)?;
        let parent_index = if definition.parent.is_some() {
            Some(current_root.ok_or("child segment precedes its root")?)
        } else {
            current_root = Some(records.len());
            None
        };
        records.push(UnloadedSegment {
            segment: definition.name.clone(),
            parent_index,
            data,
        });
    }
    if !saw_header || !saw_trailer {
        return Err("DFSURGU0 header or trailer is missing".into());
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::parse_dbd;

    fn record(kind: u8, name: &str, data: &[u8]) -> Vec<u8> {
        let length = if kind == 0 || kind == 0x98 {
            88
        } else {
            40 + data.len()
        };
        let mut bytes = vec![0; length];
        bytes[0..2].copy_from_slice(&(length as u16).to_be_bytes());
        bytes[4] = if kind == 0x98 { 0 } else { kind };
        bytes[5] = if kind == 0x98 { 0x98 } else { 0x80 };
        if kind != 0 && kind != 0x98 {
            bytes[6..8].copy_from_slice(&[0, 0x23]);
            bytes[8..10].copy_from_slice(&(data.len() as u16).to_be_bytes());
            bytes[10..18].copy_from_slice(
                &PcbEncoding::Cp1047
                    .encode_control(format!("{name:<8}").as_bytes())
                    .unwrap(),
            );
            bytes[39..length - 1].copy_from_slice(data);
        }
        bytes
    }

    #[test]
    fn parses_parent_child_and_rejects_corrupt_prefix() {
        let dbd = parse_dbd(
            " DBD NAME=TEST,ACCESS=(HIDAM,VSAM)\n\
            SEGM NAME=ROOT,PARENT=0,BYTES=6\n\
            FIELD NAME=(ID,SEQ,U),START=1,BYTES=2,TYPE=P\n\
            SEGM NAME=CHILD,PARENT=((ROOT,)),BYTES=5\n\
            FIELD NAME=(SEQNO,SEQ,U),START=1,BYTES=2,TYPE=C\n\
            DBDGEN\n END\n",
        )
        .unwrap();
        let mut input = Vec::new();
        input.extend(record(0, "", &[]));
        input.extend(record(1, "ROOT", &[0x12, 0x3c, 0, 0xff, 0, 0]));
        input.extend(record(2, "CHILD", &[0x40, 0x41, 0x0e, 0x42, 0x0f]));
        input.extend(record(0x98, "", &[]));
        let rows = parse_dfsurgu0(&input, &dbd).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].parent_index, None);
        assert_eq!(rows[1].parent_index, Some(0));
        assert_eq!(rows[1].data, [0x40, 0x41, 0x0e, 0x42, 0x0f]);
        let mut broken = input.clone();
        broken[88 + 8] = 0xff;
        assert!(parse_dfsurgu0(&broken, &dbd).is_err());
        let mut broken = input.clone();
        broken[88 + 40 + 6 - 1] = 0xff;
        assert!(parse_dfsurgu0(&broken, &dbd).is_err());
        assert!(parse_dfsurgu0(&input[..input.len() - 1], &dbd).is_err());
    }
}
