//! Load-sized storage for engine text. Mutation never grows it.
use std::fmt::{self, Write};

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
        if text.len() > N {
            return Err(fmt::Error);
        }
        self.clear();
        self.write_str(text)
    }
}
impl<const N: usize> Write for FixedText<N> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len + text.len();
        if end > N {
            return Err(fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}
