use super::info_remove;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoBytes {
    Exact,
    Ascii7Printable,
}
#[derive(Clone, Copy)]
pub struct InfoRules {
    /// Includes the native terminating byte.
    pub bytes: usize,
    pub field_bytes: usize,
    pub filter: InfoBytes,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoError {
    Invalid,
    Capacity,
}
pub fn info_valid_field(text: &[u8], filter: InfoBytes) -> bool {
    text.iter().all(|&byte| {
        let tested = match filter {
            InfoBytes::Exact => byte,
            InfoBytes::Ascii7Printable => byte & 127,
        };
        !matches!(tested, b'\\' | b'"' | b';') && (filter != InfoBytes::Exact || tested >= 32)
    })
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
}
impl<'a> InfoUpdate<'a> {
    pub fn new(key: &'a [u8], value: &'a [u8], rules: InfoRules) -> Result<Self, InfoError> {
        if key.len() >= rules.field_bytes
            || value.len() >= rules.field_bytes
            || !info_valid_field(key, rules.filter)
            || !info_valid_field(value, rules.filter)
        {
            return Err(InfoError::Invalid);
        }
        Ok(Self { key, value, rules })
    }
    pub fn apply(self, output: &mut [u8], length: &mut usize) -> Result<(), InfoError> {
        let Self { key, value, rules } = self;
        if *length >= output.len() {
            return Err(InfoError::Capacity);
        }
        *length = info_remove(&mut output[..*length], key, false, true);
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
        for bytes in [b"\\".as_slice(), key, b"\\", value] {
            for &byte in bytes {
                let byte = match rules.filter {
                    InfoBytes::Exact => byte,
                    InfoBytes::Ascii7Printable => {
                        let byte = byte & 127;
                        if !(32..=126).contains(&byte) {
                            continue;
                        }
                        byte
                    }
                };
                output[*length] = byte;
                *length += 1;
            }
        }
        output[*length] = 0;
        Ok(())
    }
}
