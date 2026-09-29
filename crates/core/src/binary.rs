//! Bounded byte storage ported from `src/core/binary/index.ts`: a cursor
//! reader over borrowed bytes and a fixed-capacity writer. Per-family
//! codecs build on these primitives.

use thiserror::Error;

/// Error for out-of-range reads.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{input}:{offset}: {message}")]
pub struct BinaryError {
    /// Input name for diagnostics.
    pub input: String,
    /// Byte offset of the failure.
    pub offset: usize,
    /// What went wrong.
    pub message: String,
}

impl BinaryError {
    fn new(source: &str, offset: usize, message: impl Into<String>) -> Self {
        Self::custom(source, offset, message)
    }

    /// Build an error for a validation failure at an offset.
    pub fn custom(source: &str, offset: usize, message: impl Into<String>) -> Self {
        Self {
            input: source.to_string(),
            offset,
            message: message.into(),
        }
    }
}

/// Cursor reader over borrowed little-endian bytes.
#[derive(Debug, Clone)]
pub struct BinaryReader<'a> {
    data: &'a [u8],
    source: String,
    cursor: usize,
}

impl<'a> BinaryReader<'a> {
    /// Borrow an input buffer.
    #[must_use]
    pub fn new(data: &'a [u8], source: &str) -> Self {
        Self {
            data,
            source: source.to_string(),
            cursor: 0,
        }
    }

    /// Current cursor offset.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.cursor
    }

    /// Input length in bytes.
    #[must_use]
    pub fn length(&self) -> usize {
        self.data.len()
    }

    /// Bytes remaining after the cursor.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.data.len() - self.cursor
    }

    fn check(&self, offset: usize, length: usize) -> Result<(), BinaryError> {
        if length > self.data.len() || offset > self.data.len() - length {
            return Err(BinaryError::new(
                &self.source,
                offset,
                format!("range of {length} bytes exceeds {}-byte input", self.data.len()),
            ));
        }
        Ok(())
    }

    fn take(&mut self, length: usize) -> Result<usize, BinaryError> {
        let offset = self.cursor;
        self.check(offset, length)?;
        self.cursor += length;
        Ok(offset)
    }

    /// Move the cursor to an absolute offset.
    pub fn seek(&mut self, offset: usize) -> Result<(), BinaryError> {
        self.check(offset, 0)?;
        self.cursor = offset;
        Ok(())
    }

    /// Skip bytes.
    pub fn skip(&mut self, length: usize) -> Result<(), BinaryError> {
        self.take(length).map(|_| ())
    }

    /// Read a byte.
    pub fn u8(&mut self) -> Result<u8, BinaryError> {
        let offset = self.take(1)?;
        Ok(self.data[offset])
    }

    /// Read a signed byte.
    pub fn i8(&mut self) -> Result<i8, BinaryError> {
        Ok(self.u8()? as i8)
    }

    /// Read a little-endian `u16`.
    pub fn u16(&mut self) -> Result<u16, BinaryError> {
        let offset = self.take(2)?;
        Ok(u16::from_le_bytes([self.data[offset], self.data[offset + 1]]))
    }

    /// Read a little-endian `i16`.
    pub fn i16(&mut self) -> Result<i16, BinaryError> {
        Ok(self.u16()? as i16)
    }

    /// Read a little-endian `u32`.
    pub fn u32(&mut self) -> Result<u32, BinaryError> {
        let offset = self.take(4)?;
        Ok(u32::from_le_bytes(self.data[offset..offset + 4].try_into().unwrap()))
    }

    /// Read a little-endian `i32`.
    pub fn i32(&mut self) -> Result<i32, BinaryError> {
        Ok(self.u32()? as i32)
    }

    /// Read a little-endian `f32`.
    pub fn f32(&mut self) -> Result<f32, BinaryError> {
        Ok(f32::from_bits(self.u32()?))
    }

    /// Read a little-endian `f64`.
    pub fn f64(&mut self) -> Result<f64, BinaryError> {
        let offset = self.take(8)?;
        Ok(f64::from_bits(u64::from_le_bytes(
            self.data[offset..offset + 8].try_into().unwrap(),
        )))
    }

    /// Read a `f32` that must be finite.
    pub fn finite_f32(&mut self) -> Result<f32, BinaryError> {
        let offset = self.cursor;
        let value = self.f32()?;
        if !value.is_finite() {
            return Err(BinaryError::new(&self.source, offset, "non-finite float"));
        }
        Ok(value)
    }

    /// Read an owned copy of `length` bytes.
    pub fn bytes(&mut self, length: usize) -> Result<Vec<u8>, BinaryError> {
        let offset = self.take(length)?;
        Ok(self.data[offset..offset + length].to_vec())
    }

    /// Read a fixed-width NUL-terminated UTF-8 string (fails on invalid UTF-8).
    pub fn fixed_string(&mut self, length: usize) -> Result<String, BinaryError> {
        let bytes = self.bytes(length)?;
        let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
        String::from_utf8(bytes[..end].to_vec())
            .map_err(|_| BinaryError::new(&self.source, self.cursor - length, "invalid UTF-8 in fixed string"))
    }

    /// Read a fixed-width NUL-terminated Quake byte string. Quake strings
    /// preserve all eight bits; each byte maps to the same code point.
    pub fn fixed_byte_string(&mut self, length: usize) -> Result<String, BinaryError> {
        let bytes = self.bytes(length)?;
        let mut result = String::new();
        for byte in bytes {
            if byte == 0 {
                break;
            }
            result.push(char::from(byte));
        }
        Ok(result)
    }

    /// Borrow a checked range; offsets are relative to this reader's input.
    pub fn view(&self, offset: usize, length: usize) -> Result<&'a [u8], BinaryError> {
        self.check(offset, length)?;
        Ok(&self.data[offset..offset + length])
    }

    /// Read a sub-reader over a checked range.
    pub fn section(&self, offset: usize, length: usize) -> Result<BinaryReader<'a>, BinaryError> {
        self.check(offset, length)?;
        Ok(BinaryReader {
            data: &self.data[offset..offset + length],
            source: format!("{}@{offset}", self.source),
            cursor: 0,
        })
    }

    /// Consume an expected magic string.
    pub fn expect_magic(&mut self, magic: &str) -> Result<(), BinaryError> {
        let offset = self.cursor;
        let actual = self.fixed_string(magic.len())?;
        if actual != magic {
            return Err(BinaryError::new(
                &self.source,
                offset,
                format!("expected {magic:?}, got {actual:?}"),
            ));
        }
        Ok(())
    }

    /// Read a sub-reader over records; the length must be a multiple of the stride.
    pub fn records(&self, offset: usize, length: usize, stride: usize) -> Result<BinaryReader<'a>, BinaryError> {
        if stride == 0 || !length.is_multiple_of(stride) {
            return Err(BinaryError::new(
                &self.source,
                offset,
                format!("length {length} is not a multiple of record size {stride}"),
            ));
        }
        self.section(offset, length)
    }
}

/// Fixed-capacity little-endian writer.
#[derive(Debug, Clone)]
pub struct BinaryWriter {
    data: Vec<u8>,
    cursor: usize,
}

impl BinaryWriter {
    /// Allocate a zeroed buffer with `capacity` bytes.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            data: vec![0; capacity],
            cursor: 0,
        }
    }

    /// Bytes written so far.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.cursor
    }

    /// Total capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.data.len()
    }

    fn take(&mut self, length: usize) -> Result<usize, BinaryError> {
        if length > self.data.len() || self.cursor > self.data.len() - length {
            return Err(BinaryError::new(
                "<output>",
                self.cursor,
                format!("Binary output exceeds {}-byte capacity", self.data.len()),
            ));
        }
        let offset = self.cursor;
        self.cursor += length;
        Ok(offset)
    }

    /// Write a byte.
    pub fn u8(&mut self, value: u8) -> Result<(), BinaryError> {
        let offset = self.take(1)?;
        self.data[offset] = value;
        Ok(())
    }

    /// Write a signed byte.
    pub fn i8(&mut self, value: i8) -> Result<(), BinaryError> {
        self.u8(value as u8)
    }

    /// Write a little-endian `u16`.
    pub fn u16(&mut self, value: u16) -> Result<(), BinaryError> {
        let offset = self.take(2)?;
        self.data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write a little-endian `i16`.
    pub fn i16(&mut self, value: i16) -> Result<(), BinaryError> {
        self.u16(value as u16)
    }

    /// Write a little-endian `u32`.
    pub fn u32(&mut self, value: u32) -> Result<(), BinaryError> {
        let offset = self.take(4)?;
        self.data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write a little-endian `i32`.
    pub fn i32(&mut self, value: i32) -> Result<(), BinaryError> {
        self.u32(value as u32)
    }

    /// Write a little-endian `f32`.
    pub fn f32(&mut self, value: f32) -> Result<(), BinaryError> {
        self.u32(value.to_bits())
    }

    /// Write a little-endian `f64`.
    pub fn f64(&mut self, value: f64) -> Result<(), BinaryError> {
        let offset = self.take(8)?;
        self.data[offset..offset + 8].copy_from_slice(&value.to_bits().to_le_bytes());
        Ok(())
    }

    /// Write raw bytes.
    pub fn bytes(&mut self, value: &[u8]) -> Result<(), BinaryError> {
        let offset = self.take(value.len())?;
        self.data[offset..offset + value.len()].copy_from_slice(value);
        Ok(())
    }

    /// Finish and return the written prefix.
    #[must_use]
    pub fn finish(&self) -> Vec<u8> {
        self.data[..self.cursor].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_writer_round_trip() {
        let mut writer = BinaryWriter::new(32);
        writer.u8(0xab).unwrap();
        writer.i16(-300).unwrap();
        writer.u32(0xdead_beef).unwrap();
        writer.f32(1.5).unwrap();
        writer.bytes(b"hi").unwrap();
        let bytes = writer.finish();
        assert_eq!(writer.offset(), 13);

        let mut reader = BinaryReader::new(&bytes, "<test>");
        assert_eq!(reader.u8().unwrap(), 0xab);
        assert_eq!(reader.i16().unwrap(), -300);
        assert_eq!(reader.u32().unwrap(), 0xdead_beef);
        assert_eq!(reader.f32().unwrap(), 1.5);
        assert_eq!(reader.bytes(2).unwrap(), b"hi");
        assert_eq!(reader.remaining(), 0);
        assert!(reader.u8().is_err());
    }

    #[test]
    fn strings_and_sections_match_donor() {
        let mut reader = BinaryReader::new(b"IBSP\0name\0\0\0\xff\xfe", "<test>");
        reader.expect_magic("IBSP").unwrap();
        assert!(reader.expect_magic("nope").is_err());
        let mut reader = BinaryReader::new(b"name\0\0\0\xff\xfe", "<test>");
        assert_eq!(reader.fixed_string(5).unwrap(), "name");
        reader.skip(2).unwrap();
        assert_eq!(reader.fixed_byte_string(2).unwrap(), "ÿþ");
        let section = reader.section(0, 7).unwrap();
        assert_eq!(section.length(), 7);
        assert!(reader.section(0, 10).is_err());
        assert!(reader.records(0, 7, 2).is_err());
        let mut writer = BinaryWriter::new(1);
        writer.u8(1).unwrap();
        assert!(writer.u8(2).is_err());
    }
}
