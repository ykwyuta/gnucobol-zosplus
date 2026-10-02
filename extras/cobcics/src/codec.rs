//! Convert between UTF-8 worker text and the terminal's EBCDIC code page.

use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HostCodec {
    #[default]
    Cp037,
    Japanese(u16),
}

#[derive(Clone, Copy)]
enum JapaneseCode {
    Single(u8),
    Double(u16),
}

struct JapaneseTable {
    data: &'static [u8],
    reverse: HashMap<char, JapaneseCode>,
}

impl JapaneseTable {
    fn new(data: &'static [u8]) -> Self {
        assert_eq!(&data[..4], b"CJ01");
        let count = u32::from_be_bytes(data[1028..1032].try_into().unwrap()) as usize;
        assert_eq!(data.len(), 1032 + count * 6);
        let mut reverse = HashMap::new();
        for byte in 0..=255u8 {
            if byte == 0x0e || byte == 0x0f { continue; }
            if let Some(ch) = Self::scalar(data, 4 + usize::from(byte) * 4) {
                reverse.entry(ch).or_insert(JapaneseCode::Single(byte));
            }
        }
        for index in 0..count {
            let offset = 1032 + index * 6;
            if let Some(ch) = Self::scalar(data, offset + 2) {
                let code = u16::from_be_bytes(data[offset..offset + 2].try_into().unwrap());
                reverse.entry(ch).or_insert(JapaneseCode::Double(code));
            }
        }
        Self { data, reverse }
    }

    fn scalar(data: &[u8], offset: usize) -> Option<char> {
        char::from_u32(u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()))
    }

    fn single(&self, byte: u8) -> Option<char> {
        Self::scalar(self.data, 4 + usize::from(byte) * 4)
    }

    fn double(&self, code: u16) -> Option<char> {
        let count = (self.data.len() - 1032) / 6;
        let mut low = 0;
        let mut high = count;
        while low < high {
            let middle = low + (high - low) / 2;
            let offset = 1032 + middle * 6;
            let key = u16::from_be_bytes(self.data[offset..offset + 2].try_into().unwrap());
            if key < code { low = middle + 1; } else { high = middle; }
        }
        if low == count { return None; }
        let offset = 1032 + low * 6;
        (self.data[offset..offset + 2] == code.to_be_bytes())
            .then(|| Self::scalar(self.data, offset + 2)).flatten()
    }
}

fn japanese_table(number: u16) -> &'static JapaneseTable {
    static IBM930: OnceLock<JapaneseTable> = OnceLock::new();
    static IBM939: OnceLock<JapaneseTable> = OnceLock::new();
    static IBM1390: OnceLock<JapaneseTable> = OnceLock::new();
    static IBM1399: OnceLock<JapaneseTable> = OnceLock::new();
    match number {
        930 => IBM930.get_or_init(|| JapaneseTable::new(include_bytes!("../data/ibm930.bin"))),
        939 => IBM939.get_or_init(|| JapaneseTable::new(include_bytes!("../data/ibm939.bin"))),
        1390 => IBM1390.get_or_init(|| JapaneseTable::new(include_bytes!("../data/ibm1390.bin"))),
        1399 => IBM1399.get_or_init(|| JapaneseTable::new(include_bytes!("../data/ibm1399.bin"))),
        _ => unreachable!(),
    }
}

impl HostCodec {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name.to_ascii_uppercase().replace('-', "").as_str() {
            "CP037" | "IBM037" | "037" => Ok(Self::Cp037),
            "CP930" | "IBM930" | "930" => Ok(Self::Japanese(930)),
            "CP939" | "IBM939" | "939" => Ok(Self::Japanese(939)),
            "CP1390" | "IBM1390" | "1390" => Ok(Self::Japanese(1390)),
            "CP1399" | "IBM1399" | "1399" => Ok(Self::Japanese(1399)),
            _ => Err(format!("unsupported host code page: {name}")),
        }
    }

    pub fn encode(self, text: &str) -> Result<Vec<u8>, String> {
        let Self::Japanese(number) = self else { return encode(text); };
        let table = japanese_table(number);
        let mut result = Vec::with_capacity(text.len() + 4);
        let mut dbcs = false;
        for ch in text.chars() {
            let code = table.reverse.get(&ch)
                .ok_or_else(|| format!("U+{:04X} is not representable in IBM-{number}", ch as u32))?;
            match code {
                JapaneseCode::Single(byte) => {
                    if dbcs { result.push(0x0f); dbcs = false; }
                    result.push(*byte);
                }
                JapaneseCode::Double(pair) => {
                    if !dbcs { result.push(0x0e); dbcs = true; }
                    result.extend_from_slice(&pair.to_be_bytes());
                }
            }
        }
        if dbcs { result.push(0x0f); }
        Ok(result)
    }

    pub fn decode(self, bytes: &[u8]) -> Result<String, String> {
        let Self::Japanese(number) = self else { return decode(bytes); };
        let table = japanese_table(number);
        let mut result = String::new();
        let mut index = 0;
        let mut dbcs = false;
        while index < bytes.len() {
            let byte = bytes[index];
            if !dbcs && byte == 0x0e { dbcs = true; index += 1; continue; }
            if dbcs && byte == 0x0f { dbcs = false; index += 1; continue; }
            if byte == 0x0e || byte == 0x0f {
                return Err("invalid Japanese EBCDIC SO/SI sequence".to_owned());
            }
            if dbcs {
                let trail = *bytes.get(index + 1)
                    .ok_or_else(|| "incomplete Japanese EBCDIC DBCS pair".to_owned())?;
                let pair = u16::from_be_bytes([byte, trail]);
                result.push(table.double(pair).ok_or_else(||
                    format!("invalid IBM-{number} DBCS code {pair:04X}"))?);
                index += 2;
            } else {
                result.push(if byte == 0 { ' ' } else {
                    table.single(byte).ok_or_else(||
                        format!("invalid IBM-{number} SBCS byte {byte:02X}"))?
                });
                index += 1;
            }
        }
        if dbcs { return Err("incomplete Japanese EBCDIC SO/SI sequence".to_owned()); }
        Ok(result)
    }
}

const ASCII_TO_CP037: [u8; 128] = [
    0x00, 0x01, 0x02, 0x03, 0x37, 0x2d, 0x2e, 0x2f, 0x16, 0x05, 0x25, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, 0x11, 0x12, 0x13, 0x3c, 0x3d, 0x32, 0x26, 0x18, 0x19, 0x3f, 0x27, 0x1c, 0x1d, 0x1e, 0x1f,
    0x40, 0x5a, 0x7f, 0x7b, 0x5b, 0x6c, 0x50, 0x7d, 0x4d, 0x5d, 0x5c, 0x4e, 0x6b, 0x60, 0x4b, 0x61,
    0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0x7a, 0x5e, 0x4c, 0x7e, 0x6e, 0x6f,
    0x7c, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xd1, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6,
    0xd7, 0xd8, 0xd9, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xba, 0xe0, 0xbb, 0xb0, 0x6d,
    0x79, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96,
    0x97, 0x98, 0x99, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xc0, 0x4f, 0xd0, 0xa1, 0x07,
];

pub fn encode(text: &str) -> Result<Vec<u8>, String> {
    text.bytes()
        .map(|byte| {
            ASCII_TO_CP037
                .get(byte as usize)
                .copied()
                .ok_or_else(|| "CP037 prototype accepts ASCII text only".to_owned())
        })
        .collect()
}

pub fn decode(bytes: &[u8]) -> Result<String, String> {
    bytes
        .iter()
        .map(|byte| {
            if *byte == 0 {
                return Ok(' ');
            }
            ASCII_TO_CP037
                .iter()
                .position(|candidate| candidate == byte)
                .map(|index| index as u8 as char)
                .ok_or_else(|| format!("unsupported CP037 byte: {byte:02x}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cp037_round_trip() {
        let source = "Action 1. F3=Exit";
        assert_eq!(decode(&encode(source).unwrap()).unwrap(), source);
    }

    #[test]
    fn japanese_mixed_round_trip_and_shift_validation() {
        for number in [930, 939, 1390, 1399] {
            let codec = HostCodec::Japanese(number);
            let source = "A日本語B";
            let bytes = codec.encode(source).unwrap();
            assert!(bytes.windows(2).any(|pair| pair[0] == 0x0e));
            assert!(bytes.contains(&0x0f));
            assert_eq!(codec.decode(&bytes).unwrap(), source);
            let incomplete = codec.encode("日本語").unwrap();
            assert!(codec.decode(&incomplete[..incomplete.len() - 1]).is_err());
            assert!(codec.decode(&[0x0f]).is_err());
        }
        assert_eq!(HostCodec::Japanese(1399).encode("日本語").unwrap(),
                   [0x0e, 0x45, 0x62, 0x45, 0x66, 0x48, 0xe7, 0x0f]);
        assert!(HostCodec::Cp037.encode("日本語").is_err());
        assert!(HostCodec::parse("IBM-1399").is_ok());
        assert!(HostCodec::parse("UTF-8").is_err());
    }
}
