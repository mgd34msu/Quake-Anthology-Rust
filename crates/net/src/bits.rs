//! Bounded LSB-first bit buffer. This is the shared mechanism that
//! per-family codecs build on: Q1/Q2 byte-oriented messages use aligned
//! writes, while the Q3 bitstream codec (with its Huffman layer) plugs in
//! during the net phase. Overflow sets a sticky flag instead of failing,
//! matching the donor message buffers.

use thiserror::Error;

/// Error for invalid bit widths and truncated reads.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BitError {
    /// Bit widths must run from 1 to 32.
    #[error("Invalid message bit width")]
    BadWidth,
    /// The read ran past the written bits.
    #[error("Truncated message bits at byte {0}")]
    Truncated(usize),
}

/// Fixed-capacity LSB-first bit writer.
#[derive(Debug, Clone)]
pub struct BitWriter {
    data: Vec<u8>,
    bits: usize,
    overflowed: bool,
}

impl BitWriter {
    /// Allocate a zeroed buffer with `capacity` bytes.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            data: vec![0; capacity],
            bits: 0,
            overflowed: false,
        }
    }

    /// Bits written so far.
    #[must_use]
    pub fn bit_position(&self) -> usize {
        self.bits
    }

    /// Bytes spanned by the written bits.
    #[must_use]
    pub fn byte_length(&self) -> usize {
        self.bits.div_ceil(8)
    }

    /// Whether a write overflowed the capacity.
    #[must_use]
    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Written bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.byte_length()]
    }

    /// Reset the cursor and overflow state, keeping the storage.
    pub fn clear(&mut self) {
        self.bits = 0;
        self.overflowed = false;
    }

    fn reserve(&mut self, bits: usize) -> bool {
        if self.overflowed {
            return false;
        }
        if self.bits + bits > self.data.len() * 8 {
            self.overflowed = true;
            return false;
        }
        true
    }

    /// Write the low `width` bits of `value`, LSB first.
    pub fn write_bits(&mut self, value: u32, width: usize) -> Result<(), BitError> {
        if width == 0 || width > 32 {
            return Err(BitError::BadWidth);
        }
        if !self.reserve(width) {
            return Ok(());
        }
        for index in 0..width {
            if (value >> index) & 1 == 1 {
                self.data[self.bits / 8] |= 1 << (self.bits % 8);
            }
            self.bits += 1;
        }
        Ok(())
    }

    /// Write one byte.
    pub fn write_byte(&mut self, value: u8) -> Result<(), BitError> {
        self.write_bits(u32::from(value), 8)
    }

    /// Write a little-endian `u16`.
    pub fn write_u16(&mut self, value: u16) -> Result<(), BitError> {
        self.write_bits(u32::from(value), 16)
    }

    /// Write a little-endian `i32`.
    pub fn write_i32(&mut self, value: i32) -> Result<(), BitError> {
        self.write_bits(value as u32, 32)
    }

    /// Write a binary32 value by bit pattern.
    pub fn write_f32(&mut self, value: f32) -> Result<(), BitError> {
        self.write_bits(value.to_bits(), 32)
    }
}

/// LSB-first bit reader over borrowed bytes.
#[derive(Debug, Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    bits: usize,
    limit_bits: usize,
}

impl<'a> BitReader<'a> {
    /// Borrow a buffer; the default limit spans the whole input.
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        let limit_bits = data.len() * 8;
        Self {
            data,
            bits: 0,
            limit_bits,
        }
    }

    /// Borrow a buffer with an explicit bit limit.
    #[must_use]
    pub fn with_bit_limit(data: &'a [u8], limit_bits: usize) -> Self {
        Self {
            data,
            bits: 0,
            limit_bits: limit_bits.min(data.len() * 8),
        }
    }

    /// Bits consumed so far.
    #[must_use]
    pub fn bit_position(&self) -> usize {
        self.bits
    }

    /// Read `width` bits, LSB first.
    pub fn read_bits(&mut self, width: usize) -> Result<u32, BitError> {
        if width == 0 || width > 32 {
            return Err(BitError::BadWidth);
        }
        if self.bits + width > self.limit_bits {
            return Err(BitError::Truncated(self.bits / 8));
        }
        let mut value = 0;
        for index in 0..width {
            if (self.data[self.bits / 8] >> (self.bits % 8)) & 1 == 1 {
                value |= 1 << index;
            }
            self.bits += 1;
        }
        Ok(value)
    }

    /// Read one byte.
    pub fn read_byte(&mut self) -> Result<u8, BitError> {
        Ok(self.read_bits(8)? as u8)
    }

    /// Read a little-endian `u16`.
    pub fn read_u16(&mut self) -> Result<u16, BitError> {
        Ok(self.read_bits(16)? as u16)
    }

    /// Read a little-endian `i32`.
    pub fn read_i32(&mut self) -> Result<i32, BitError> {
        Ok(self.read_bits(32)? as i32)
    }

    /// Read a binary32 value by bit pattern.
    pub fn read_f32(&mut self) -> Result<f32, BitError> {
        Ok(f32::from_bits(self.read_bits(32)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_pack_lsb_first() {
        let mut writer = BitWriter::new(4);
        writer.write_bits(0b101, 3).unwrap();
        writer.write_bits(0b11, 2).unwrap();
        writer.write_byte(0xa0).unwrap();
        assert_eq!(writer.bytes(), &[29, 20]);
        let mut reader = BitReader::new(writer.bytes());
        assert_eq!(reader.read_bits(3).unwrap(), 0b101);
        assert_eq!(reader.read_bits(2).unwrap(), 0b11);
        assert_eq!(reader.read_byte().unwrap(), 0xa0);
        assert!(reader.read_byte().is_err());
    }

    #[test]
    fn overflow_is_sticky() {
        let mut writer = BitWriter::new(1);
        writer.write_byte(0xff).unwrap();
        assert!(!writer.overflowed());
        writer.write_bits(1, 1).unwrap();
        assert!(writer.overflowed());
        assert_eq!(writer.bytes(), &[0xff]);
        writer.clear();
        assert!(!writer.overflowed());
        assert_eq!(writer.bit_position(), 0);
    }

    #[test]
    fn scalars_round_trip() {
        let mut writer = BitWriter::new(16);
        writer.write_u16(0x1234).unwrap();
        writer.write_i32(-7).unwrap();
        writer.write_f32(-0.0).unwrap();
        let mut reader = BitReader::new(writer.bytes());
        assert_eq!(reader.read_u16().unwrap(), 0x1234);
        assert_eq!(reader.read_i32().unwrap(), -7);
        assert_eq!(reader.read_f32().unwrap().to_bits(), (-0.0f32).to_bits());
    }
}
