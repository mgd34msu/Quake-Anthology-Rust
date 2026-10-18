//! Load-sized storage for engine text. Mutation never grows it.
mod format;
pub use format::{fixed_float, native_number};
mod info;
mod numbers;
mod slots;
pub use info::{
    InfoBytes, InfoError, InfoRules, InfoUpdate, info_delimiter, info_set, info_valid_field,
};
pub use numbers::{
    atof_prefix, atof_prefix_bytes, atoi_prefix, decimal_prefix_end, hex_digit, integer_prefix,
};
pub use slots::{SlotsError, SlotsLimits, TextSlots};

/// Native inline names use atoi's prefix and wrap to the model ordinal width.
pub fn inline_model_index(name: &[u8]) -> Option<u32> {
    name.strip_prefix(b"*")
        .map(|suffix| atoi_prefix(suffix) as u32)
}

use std::fmt::{self, Write};

/// First unquoted semicolon or requested native line separator.
pub fn command_separator(bytes: &[u8], newline: bool, carriage_return: bool) -> usize {
    let mut quoted = false;
    bytes
        .iter()
        .position(|&byte| {
            if byte == b'"' {
                quoted = !quoted;
            }
            (!quoted && byte == b';')
                || (newline && byte == b'\n')
                || (carriage_return && byte == b'\r')
        })
        .unwrap_or(bytes.len())
}

/// A strict native string requires a terminator within the supplied boundary.
pub fn terminated(bytes: &[u8]) -> Option<&[u8]> {
    bytes
        .iter()
        .position(|&byte| byte == 0)
        .map(|end| &bytes[..end])
}
/// Fixed-width fields keep their entire boundary when no terminator is present.
pub fn until_nul(bytes: &[u8]) -> &[u8] {
    terminated(bytes).unwrap_or(bytes)
}

struct InfoPair {
    start: usize,
    key: std::ops::Range<usize>,
    value: std::ops::Range<usize>,
}
impl InfoPair {
    fn matches(&self, info: &[u8], key: &[u8], folded: bool) -> bool {
        if folded {
            crate::names::compare_folded(&info[self.key.clone()], key).is_eq()
        } else {
            &info[self.key.clone()] == key
        }
    }
}
fn info_pair(info: &[u8], start: usize) -> Option<InfoPair> {
    let first = start + usize::from(info.get(start) == Some(&b'\\'));
    let split = first + info.get(first..)?.iter().position(|&b| b == b'\\')?;
    let value = split + 1;
    let end = value
        + info[value..]
            .iter()
            .position(|&b| b == b'\\')
            .unwrap_or(info.len() - value);
    Some(InfoPair {
        start,
        key: first..split,
        value: value..end,
    })
}

/// Borrow native info fields in their original order, using the same parser
/// as lookup and removal. A trailing incomplete field ends iteration.
pub fn info_pairs(info: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    info_spans(info).map(|pair| (&info[pair.key], &info[pair.value]))
}

fn info_spans(info: &[u8]) -> impl Iterator<Item = InfoPair> {
    let mut cursor = 0;
    std::iter::from_fn(move || {
        let pair = info_pair(info, cursor)?;
        cursor = pair.value.end;
        Some(pair)
    })
}

/// Effective first-key changes, including removals as empty values. Reordering
/// fields and changing a later duplicate do not create native key updates.
pub fn info_changes<'a>(
    previous: &'a [u8],
    current: &'a [u8],
) -> impl Iterator<Item = (&'a [u8], &'a [u8])> {
    let changed = info_spans(previous).filter_map(move |pair| {
        let key = &previous[pair.key];
        if info_value(previous, key, false) != Some(pair.value.clone()) {
            return None;
        }
        let value = info_value(current, key, false).map_or(&b""[..], |range| &current[range]);
        (value != &previous[pair.value]).then_some((key, value))
    });
    let added = info_spans(current).filter_map(move |pair| {
        let key = &current[pair.key];
        let value = &current[pair.value.clone()];
        (info_value(previous, key, false).is_none()
            && info_value(current, key, false) == Some(pair.value)
            && !value.is_empty())
        .then_some((key, value))
    });
    changed.chain(added)
}

/// The first matching value in a native backslash-separated info string.
/// Callers select native key comparison; returned spans never own or alter text.
pub fn info_value(info: &[u8], key: &[u8], folded: bool) -> Option<std::ops::Range<usize>> {
    info_spans(info)
        .find(|pair| pair.matches(info, key, folded))
        .map(|pair| pair.value)
}

/// The first exact native info field, parsed with the common atoi semantics.
pub fn info_integer(info: &[u8], key: &[u8]) -> i32 {
    info_value(info, key, false).map_or(0, |span| atoi_prefix(&info[span]))
}

/// Compact a native info string in place; the caller owns its NUL terminator.
/// Matching and duplicate removal are caller-selected native rules.
pub fn info_remove(info: &mut [u8], key: &[u8], folded: bool, remove_all: bool) -> usize {
    let mut len = info.len();
    let mut cursor = 0;
    while let Some(pair) = info_pair(&info[..len], cursor) {
        if pair.matches(info, key, folded) {
            info.copy_within(pair.value.end..len, pair.start);
            len -= pair.value.end - pair.start;
            if !remove_all {
                break;
            }
        } else {
            cursor = pair.value.end;
        }
    }
    len
}

#[derive(Clone, Debug)]
pub struct FixedText<const N: usize> {
    bytes: [u8; N],
    len: usize,
}
impl<const N: usize> Default for FixedText<N> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
}
impl<const N: usize> FixedText<N> {
    pub const CAPACITY: usize = N;
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
    pub fn clear(&mut self) {
        self.len = 0;
    }
    pub fn replace(&mut self, range: std::ops::Range<usize>, text: &str) -> fmt::Result {
        self.replace_bytes(range, text.as_bytes())
    }
    pub fn replace_bytes(&mut self, range: std::ops::Range<usize>, text: &[u8]) -> fmt::Result {
        let len = self.len - range.len() + text.len();
        if len > N {
            return Err(fmt::Error);
        }
        self.bytes
            .copy_within(range.end..self.len, range.start + text.len());
        self.bytes[range.start..range.start + text.len()].copy_from_slice(text);
        self.len = len;
        Ok(())
    }
    pub fn set(&mut self, text: &str) -> fmt::Result {
        self.set_bytes(text.as_bytes())
    }
    pub fn set_bytes(&mut self, text: &[u8]) -> fmt::Result {
        if text.len() > N {
            return Err(fmt::Error);
        }
        self.bytes[..text.len()].copy_from_slice(text);
        self.len = text.len();
        Ok(())
    }
    pub fn append_bytes(&mut self, text: &[u8]) -> fmt::Result {
        let end = self.len.checked_add(text.len()).ok_or(fmt::Error)?;
        if end > N {
            return Err(fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(text);
        self.len = end;
        Ok(())
    }
    /// Apply the common native info-string mutation within this fixed storage.
    pub fn update_info(&mut self, update: InfoUpdate<'_>) -> Result<(), InfoError> {
        update.apply(&mut self.bytes, &mut self.len)
    }
}
impl<const N: usize> Write for FixedText<N> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.append_bytes(text.as_bytes())
    }
}
impl<const N: usize> AsRef<[u8]> for FixedText<N> {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}
