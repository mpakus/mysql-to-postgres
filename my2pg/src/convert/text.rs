use super::{ConversionError, error};
use crate::model::ColumnPlan;

#[derive(Clone, Copy)]
enum Charset {
    Utf8,
    SingleByte(bool),
}

pub(super) struct Text<'a> {
    bytes: &'a [u8],
    charset: Charset,
    remove_nul: bool,
}
impl<'a> Text<'a> {
    pub(super) fn retained_bytes(&self) -> usize {
        self.bytes.len()
    }
    pub(super) fn new(column: &ColumnPlan, bytes: &'a [u8]) -> Result<Self, ConversionError> {
        Self::new_charset(
            column,
            bytes,
            column.charset.as_deref().unwrap_or("utf8mb4"),
        )
    }
    pub(super) fn metadata(column: &ColumnPlan, bytes: &'a [u8]) -> Result<Self, ConversionError> {
        Self::new_charset(column, bytes, "utf8mb4")
    }
    fn new_charset(
        column: &ColumnPlan,
        bytes: &'a [u8],
        name: &str,
    ) -> Result<Self, ConversionError> {
        let charset = match name {
            "utf8" | "utf8mb3" | "utf8mb4" | "binary" => {
                let decoded = std::str::from_utf8(bytes)
                    .map_err(|_| error(column, "invalid UTF-8 under strict charset policy"))?;
                if name == "utf8mb3" && decoded.chars().any(|ch| u32::from(ch) > 0xffff) {
                    return Err(error(
                        column,
                        "supplementary scalar exceeds explicit utf8mb3 policy",
                    ));
                }
                Charset::Utf8
            }
            "ascii" if bytes.iter().all(u8::is_ascii) => Charset::Utf8,
            "ascii" => return Err(error(column, "non-ASCII byte under ASCII charset policy")),
            "latin1" | "windows-1252" => Charset::SingleByte(true),
            "iso-8859-1" => Charset::SingleByte(false),
            _ => return Err(error(column, "unsupported strict charset policy")),
        };
        let mut text = Self {
            bytes,
            charset,
            remove_nul: column.transform.as_deref() == Some("remove-null-characters"),
        };
        if column.transform.as_deref() == Some("right-trim") {
            match charset {
                Charset::Utf8 => {
                    text.bytes = std::str::from_utf8(bytes)
                        .expect("validated UTF-8")
                        .trim_end()
                        .as_bytes()
                }
                Charset::SingleByte(cp1252) => {
                    let end = bytes
                        .iter()
                        .rposition(|byte| !decode(*byte, cp1252).is_whitespace())
                        .map_or(0, |index| index + 1);
                    text.bytes = &bytes[..end];
                }
            }
        }
        if !text.remove_nul && text.bytes.contains(&0) {
            return Err(error(
                column,
                "NUL is not representable in PostgreSQL text input",
            ));
        }
        Ok(text)
    }
    pub(super) fn visit(&self, mut output: impl FnMut(u8)) {
        match self.charset {
            Charset::Utf8 => {
                for byte in self.bytes {
                    if !self.remove_nul || *byte != 0 {
                        output(*byte);
                    }
                }
            }
            Charset::SingleByte(cp1252) => {
                for byte in self.bytes {
                    if self.remove_nul && *byte == 0 {
                        continue;
                    }
                    let mut encoded = [0u8; 4];
                    for byte in decode(*byte, cp1252).encode_utf8(&mut encoded).bytes() {
                        output(byte);
                    }
                }
            }
        }
    }
    pub(super) fn characters(&self) -> usize {
        match self.charset {
            Charset::Utf8 => std::str::from_utf8(self.bytes)
                .expect("validated UTF-8")
                .chars()
                .filter(|ch| !self.remove_nul || *ch != '\0')
                .count(),
            Charset::SingleByte(_) => self
                .bytes
                .iter()
                .filter(|byte| !self.remove_nul || **byte != 0)
                .count(),
        }
    }
}
fn decode(byte: u8, cp1252: bool) -> char {
    const CONTROL: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    if cp1252 && (0x80..=0x9f).contains(&byte) {
        CONTROL[usize::from(byte - 0x80)]
    } else {
        char::from(byte)
    }
}
