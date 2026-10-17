use super::info_remove;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoBytes {
    Exact,
    Ascii7Printable,
    /// Preserve high bits and every byte above carriage return.
    AboveCarriageReturn,
    /// Mask high bits, retaining bytes 32 through 127 inclusive.
    Ascii7,
    LowerAscii7,
}
impl InfoBytes {
    fn valid(self, byte: u8) -> bool {
        match self {
            Self::Exact => !info_delimiter(byte) && byte >= 32,
            Self::Ascii7Printable => !info_delimiter(byte & 127),
            Self::AboveCarriageReturn | Self::Ascii7 | Self::LowerAscii7 => {
                !matches!(byte, b'\\' | b'"')
            }
        }
    }
    fn output(self, byte: u8) -> Option<u8> {
        match self {
            Self::Exact => Some(byte),
            Self::Ascii7Printable => {
                let byte = byte & 127;
                (32..=126).contains(&byte).then_some(byte)
            }
            Self::AboveCarriageReturn => (byte > 13).then_some(byte),
            Self::Ascii7 | Self::LowerAscii7 => {
                let byte = byte & 127;
                (byte >= 32).then_some(if self == Self::LowerAscii7 {
                    byte.to_ascii_lowercase()
                } else {
                    byte
                })
            }
        }
    }
}
#[derive(Clone, Copy)]
pub struct InfoRules {
    /// Includes the native terminating byte.
    pub bytes: usize,
    pub key_bytes: usize,
    pub value_bytes: usize,
    pub filter: InfoBytes,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoError {
    Invalid,
    Capacity,
}
pub fn info_delimiter(byte: u8) -> bool {
    matches!(byte, b'\\' | b'"' | b';')
}
pub fn info_valid_field(text: &[u8], filter: InfoBytes) -> bool {
    text.iter().all(|&byte| filter.valid(byte))
}

/// Set an exact native key, removing all its previous rows. Validation leaves
/// the original text intact; a capacity failure retains the key's removal.
/// `length` excludes the NUL and changes even when the replacement cannot fit.
pub fn info_set(
    output: &mut [u8],
    length: &mut usize,
    key: &[u8],
    value: &[u8],
    rules: InfoRules,
) -> Result<(), InfoError> {
    InfoUpdate::new(key, value, rules)?.apply(output, length)
}
/// A native boundary can validate the fields before touching the target.
pub struct InfoUpdate<'a> {
    key: &'a [u8],
    value: &'a [u8],
    rules: InfoRules,
    remove_all: bool,
    prepend: bool,
}
impl<'a> InfoUpdate<'a> {
    pub fn new(key: &'a [u8], value: &'a [u8], rules: InfoRules) -> Result<Self, InfoError> {
        if key.len() >= rules.key_bytes
            || value.len() >= rules.value_bytes
            || !info_valid_field(key, rules.filter)
            || !info_valid_field(value, rules.filter)
        {
            return Err(InfoError::Invalid);
        }
        Ok(Self {
            key,
            value,
            rules,
            remove_all: true,
            prepend: false,
        })
    }
    /// Keep later duplicates, as native callers of Info_RemoveKey do.
    pub fn first_only(mut self) -> Self {
        self.remove_all = false;
        self
    }
    /// Place the replacement before retained rows, as Q3's native setter does.
    pub fn prepend(mut self) -> Self {
        self.prepend = true;
        self
    }
    pub fn apply(self, output: &mut [u8], length: &mut usize) -> Result<(), InfoError> {
        let Self {
            key,
            value,
            rules,
            remove_all,
            prepend,
        } = self;
        if *length >= output.len() {
            return Err(InfoError::Capacity);
        }
        *length = info_remove(&mut output[..*length], key, false, remove_all);
        output[*length] = 0;
        if value.is_empty() {
            return Ok(());
        }
        let end = length
            .checked_add(key.len())
            .and_then(|n| n.checked_add(value.len()))
            .and_then(|n| n.checked_add(2))
            .ok_or(InfoError::Capacity)?;
        if end >= rules.bytes.min(output.len()) {
            return Err(InfoError::Capacity);
        }
        let start = *length;
        for bytes in [b"\\".as_slice(), key, b"\\", value] {
            for &byte in bytes {
                let Some(byte) = rules.filter.output(byte) else {
                    continue;
                };
                output[*length] = byte;
                *length += 1;
            }
        }
        if prepend {
            output[..*length].rotate_right(*length - start);
        }
        output[*length] = 0;
        Ok(())
    }
}
