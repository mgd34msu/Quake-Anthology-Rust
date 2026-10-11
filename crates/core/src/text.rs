//! Load-sized storage for engine text. Mutation never grows it.
use std::fmt::{self, Write};

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

/// The first matching value in a native backslash-separated info string.
/// Callers select native key comparison; returned spans never own or alter text.
pub fn info_value(info: &[u8], key: &[u8], folded: bool) -> Option<std::ops::Range<usize>> {
    let mut cursor = 0;
    while let Some(pair) = info_pair(info, cursor) {
        if pair.matches(info, key, folded) {
            return Some(pair.value);
        }
        cursor = pair.value.end;
    }
    None
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
        let len = self.len - range.len() + text.len();
        if len > N {
            return Err(fmt::Error);
        }
        self.bytes
            .copy_within(range.end..self.len, range.start + text.len());
        self.bytes[range.start..range.start + text.len()].copy_from_slice(text.as_bytes());
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
}
impl<const N: usize> Write for FixedText<N> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.append_bytes(text.as_bytes())
    }
}
