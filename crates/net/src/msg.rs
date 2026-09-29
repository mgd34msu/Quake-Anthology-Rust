//! Bounded byte-oriented message buffers shared by the Q1/QW/Q2 codecs.
//!
//! Donor provenance: `SizeBuf`, `SZ_*`, and `MSG_Write*` plus
//! `MessageReader` in `src/network/q1/message.ts`; `SizeBuf`, `SZ_*`,
//! `MSG_Write*`, and `MSG_Read*` in `src/network/q2/message.ts`.
//!
//! Wire scalar encodings (truncation order, flag-selected widths) live in
//! [`crate::angles`]; this module owns cursors, capacity, and the sticky
//! overflow/bad-read state. Reads return [`MsgError::Truncated`] at the
//! first overrun instead of the donor's `-1` sentinel flow, except for
//! [`MsgReader::read_string`], which mirrors the donor by returning the
//! partial text and leaving [`MsgReader::badread`] set for [`MsgReader::finish`].

use thiserror::Error;

use crate::angles::{
    q1_byte_angle, q1_byte_angle_to_degrees, q1_coord_to_short, q1_decode_short_angle, q1_encode_angle_flags,
    q1_encode_coord_flags, q1_encode_move_angle16, q1_short_to_coord, q2_angle_to_short, q2_byte_angle,
    q2_byte_angle_to_degrees, q2_coord_to_short, Q1AngleEncoding, Q1CoordEncoding,
};

/// Error for message buffer writes and reads.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MsgError {
    /// A write exceeded a buffer that disallows overflow.
    #[error("message overflow at byte {0}")]
    Overflow(usize),
    /// A read ran past the end of the message.
    #[error("truncated message at byte {0}")]
    Truncated(usize),
    /// A write reservation was negative or larger than the buffer.
    #[error("invalid message write size {0}")]
    BadSize(usize),
}

/// Fixed-capacity little-endian message writer (`SizeBuf` + `SZ_*`).
#[derive(Debug, Clone)]
pub struct MsgWriter {
    data: Vec<u8>,
    cursize: usize,
    overflowed: bool,
    allow_overflow: bool,
}

impl MsgWriter {
    /// Allocate a zeroed buffer with `maxsize` bytes.
    #[must_use]
    pub fn new(maxsize: usize, allow_overflow: bool) -> Self {
        Self {
            data: vec![0; maxsize],
            cursize: 0,
            overflowed: false,
            allow_overflow,
        }
    }

    /// Bytes written so far.
    #[must_use]
    pub fn cursize(&self) -> usize {
        self.cursize
    }

    /// Total capacity in bytes.
    #[must_use]
    pub fn maxsize(&self) -> usize {
        self.data.len()
    }

    /// Whether an allowed overflow reset the buffer.
    #[must_use]
    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Written bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.cursize]
    }

    /// Reset the cursor and overflow state, keeping the storage.
    pub fn clear(&mut self) {
        self.cursize = 0;
        self.overflowed = false;
    }

    /// Reserve `length` bytes, resetting on allowed overflow.
    pub fn reserve(&mut self, length: usize) -> Result<usize, MsgError> {
        if length > self.data.len() {
            return Err(MsgError::BadSize(length));
        }
        if self.cursize + length > self.data.len() {
            if !self.allow_overflow {
                return Err(MsgError::Overflow(self.cursize));
            }
            self.cursize = 0;
            self.overflowed = true;
        }
        let offset = self.cursize;
        self.cursize += length;
        Ok(offset)
    }

    /// Append raw bytes (`SZ_Write`).
    pub fn write_bytes(&mut self, data: &[u8]) -> Result<(), MsgError> {
        let offset = self.reserve(data.len())?;
        self.data[offset..offset + data.len()].copy_from_slice(data);
        Ok(())
    }

    /// Write one byte (`MSG_WriteByte`).
    pub fn write_byte(&mut self, value: u8) -> Result<(), MsgError> {
        let offset = self.reserve(1)?;
        self.data[offset] = value;
        Ok(())
    }

    /// Write one signed byte (`MSG_WriteChar`).
    pub fn write_char(&mut self, value: i8) -> Result<(), MsgError> {
        self.write_byte(value as u8)
    }

    /// Write a little-endian `i16` (`MSG_WriteShort`).
    pub fn write_short(&mut self, value: i16) -> Result<(), MsgError> {
        let offset = self.reserve(2)?;
        self.data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write a little-endian `u16`.
    pub fn write_word(&mut self, value: u16) -> Result<(), MsgError> {
        self.write_short(value as i16)
    }

    /// Write a little-endian `i32` (`MSG_WriteLong`).
    pub fn write_long(&mut self, value: i32) -> Result<(), MsgError> {
        let offset = self.reserve(4)?;
        self.data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write a binary32 float (`MSG_WriteFloat`).
    pub fn write_float(&mut self, value: f32) -> Result<(), MsgError> {
        let offset = self.reserve(4)?;
        self.data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write a little-endian 64-bit integer (`MSG_WriteLong64`).
    pub fn write_long64(&mut self, value: i64) -> Result<(), MsgError> {
        let offset = self.reserve(8)?;
        self.data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write a NUL-terminated byte string (`MSG_WriteString`).
    ///
    /// Bytes pass through unchanged; values above 127 are preserved as
    /// raw bytes, matching the donor's `charCodeAt & 255` masking for the
    /// ASCII range round-trip tests use.
    pub fn write_string(&mut self, value: &str) -> Result<(), MsgError> {
        self.write_string_opt(Some(value))
    }

    /// Write a nullable NUL-terminated string (`None` writes only NUL).
    pub fn write_string_opt(&mut self, value: Option<&str>) -> Result<(), MsgError> {
        if let Some(text) = value {
            let nul = text.find('\0').unwrap_or(text.len());
            self.write_bytes(&text.as_bytes()[..nul])?;
        }
        self.write_byte(0)
    }

    /// NetQuake truncated coordinate (`MSG_WriteCoord`).
    pub fn write_coord(&mut self, value: f64) -> Result<(), MsgError> {
        self.write_short(q1_coord_to_short(value))
    }

    /// NetQuake truncated angle byte (`MSG_WriteAngle`).
    pub fn write_angle(&mut self, value: f64) -> Result<(), MsgError> {
        self.write_byte(q1_byte_angle(value))
    }

    /// FitzQuake/RMQ coordinate under `PRFL_` flags (`MSG_WriteCoordFlags`).
    pub fn write_coord_flags(&mut self, value: f64, flags: u32) -> Result<(), MsgError> {
        match q1_encode_coord_flags(value, flags) {
            Q1CoordEncoding::Short(whole) => self.write_short(whole),
            Q1CoordEncoding::ShortByte { whole, fraction } => {
                self.write_short(whole)?;
                self.write_byte(fraction)
            }
            Q1CoordEncoding::Long(value) => self.write_long(value),
            Q1CoordEncoding::Float(value) => self.write_float(value),
        }
    }

    /// FitzQuake/RMQ angle under `PRFL_` flags (`MSG_WriteAngleFlags`).
    pub fn write_angle_flags(&mut self, value: f64, flags: u32) -> Result<(), MsgError> {
        match q1_encode_angle_flags(value, flags) {
            Q1AngleEncoding::Byte(value) => self.write_byte(value),
            Q1AngleEncoding::Short(value) => self.write_short(value),
            Q1AngleEncoding::Float(value) => self.write_float(value),
        }
    }

    /// FitzQuake/RMQ movement angle (`MSG_WriteAngle16`).
    pub fn write_move_angle16(&mut self, value: f64, flags: u32) -> Result<(), MsgError> {
        match q1_encode_move_angle16(value, flags) {
            Q1AngleEncoding::Byte(value) => self.write_byte(value),
            Q1AngleEncoding::Short(value) => self.write_short(value),
            Q1AngleEncoding::Float(value) => self.write_float(value),
        }
    }

    /// Quake II truncated coordinate (`MSG_WriteCoord`).
    pub fn write_q2_coord(&mut self, value: f64) -> Result<(), MsgError> {
        self.write_short(q2_coord_to_short(value))
    }

    /// Quake II position triple (`MSG_WritePos`).
    pub fn write_q2_pos(&mut self, pos: [f64; 3]) -> Result<(), MsgError> {
        for axis in pos {
            self.write_q2_coord(axis)?;
        }
        Ok(())
    }

    /// Quake II truncated angle byte (`MSG_WriteAngle`).
    pub fn write_q2_angle(&mut self, value: f64) -> Result<(), MsgError> {
        self.write_byte(q2_byte_angle(value))
    }

    /// Quake II 16-bit angle (`MSG_WriteAngle16`).
    pub fn write_q2_angle16(&mut self, value: f64) -> Result<(), MsgError> {
        self.write_word(q2_angle_to_short(value))
    }

    /// QuakeWorld truncated angle byte (`qwWriteAngle`).
    pub fn write_qw_angle(&mut self, value: f64) -> Result<(), MsgError> {
        self.write_byte(q2_byte_angle(value))
    }

    /// QuakeWorld 16-bit angle (`qwWriteAngle16`).
    pub fn write_qw_angle16(&mut self, value: f64) -> Result<(), MsgError> {
        self.write_word(q2_angle_to_short(value))
    }
}

/// Cursor reader over borrowed message bytes with a sticky bad-read flag.
#[derive(Debug, Clone)]
pub struct MsgReader<'a> {
    data: &'a [u8],
    offset: usize,
    badread: bool,
}

impl<'a> MsgReader<'a> {
    /// Borrow a message buffer.
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            offset: 0,
            badread: false,
        }
    }

    /// Current cursor offset.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Borrow a slice of the underlying buffer.
    #[must_use]
    pub fn data_slice(&self, start: usize, end: usize) -> &[u8] {
        &self.data[start.min(end)..end.min(self.data.len())]
    }

    /// Bytes remaining after the cursor.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.offset)
    }

    /// Whether any read ran past the end.
    #[must_use]
    pub fn badread(&self) -> bool {
        self.badread
    }

    /// Fail when any read so far overran the buffer (`finish` / `checkMessageRead`).
    pub fn finish(&self) -> Result<(), MsgError> {
        if self.badread {
            return Err(MsgError::Truncated(self.offset));
        }
        Ok(())
    }

    fn take(&mut self, length: usize) -> Option<usize> {
        if self.offset + length > self.data.len() {
            self.badread = true;
            return None;
        }
        let offset = self.offset;
        self.offset += length;
        Some(offset)
    }

    fn scalar(&mut self, length: usize) -> Result<usize, MsgError> {
        let offset = self.offset;
        self.take(length).ok_or(MsgError::Truncated(offset))
    }

    /// Read one byte (`Byte` / `MSG_ReadByte`).
    pub fn byte(&mut self) -> Result<u8, MsgError> {
        let offset = self.scalar(1)?;
        Ok(self.data[offset])
    }

    /// Read one signed byte (`Char` / `MSG_ReadChar`).
    pub fn char(&mut self) -> Result<i8, MsgError> {
        Ok(self.byte()? as i8)
    }

    /// Read a little-endian `i16` (`Short` / `MSG_ReadShort`).
    pub fn short(&mut self) -> Result<i16, MsgError> {
        let offset = self.scalar(2)?;
        Ok(i16::from_le_bytes([self.data[offset], self.data[offset + 1]]))
    }

    /// Read a little-endian `u16` (`MSG_ReadWord`).
    pub fn word(&mut self) -> Result<u16, MsgError> {
        Ok(self.short()? as u16)
    }

    /// Read a little-endian `i32` (`Long` / `MSG_ReadLong`).
    pub fn long(&mut self) -> Result<i32, MsgError> {
        let offset = self.scalar(4)?;
        Ok(i32::from_le_bytes([
            self.data[offset],
            self.data[offset + 1],
            self.data[offset + 2],
            self.data[offset + 3],
        ]))
    }

    /// Read a binary32 float (`Float` / `MSG_ReadFloat`).
    pub fn float(&mut self) -> Result<f32, MsgError> {
        Ok(f32::from_bits(self.long()? as u32))
    }

    /// Advance past `len` bytes without reading them.
    pub fn skip(&mut self, len: usize) -> Result<(), MsgError> {
        self.scalar(len).map(|_| ())
    }

    /// Read a little-endian `i64` (`MSG_ReadLong64`).
    ///
    /// The donor returns `-1` past the end and lets `checkMessageRead`
    /// report it later; here the overrun surfaces immediately and
    /// [`MsgReader::finish`] stays the explicit truncation gate.
    pub fn long64(&mut self) -> Result<i64, MsgError> {
        let offset = self.scalar(8)?;
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&self.data[offset..offset + 8]);
        Ok(i64::from_le_bytes(bytes))
    }

    fn text(&mut self, limit: usize, skip_newline: bool, stop_newline: bool) -> String {
        let mut out = String::new();
        while out.len() < limit {
            let Some(offset) = self.take(1) else {
                break;
            };
            let byte = self.data[offset] as i8;
            if byte == 0 || (stop_newline && byte == 10) {
                break;
            }
            if skip_newline && byte == 10 {
                continue;
            }
            out.push(char::from(self.data[offset]));
        }
        out
    }

    /// Read a NUL-terminated string (`String`, Q1/Q2 `MSG_ReadString`).
    ///
    /// Returns the partial text when the buffer ends first, leaving
    /// [`MsgReader::badread`] set exactly like the donor.
    pub fn string(&mut self, limit: usize) -> String {
        self.text(limit, false, false)
    }

    /// Read a string that drops line feeds (QuakeWorld `String`).
    pub fn string_quakeworld(&mut self, limit: usize) -> String {
        self.text(limit, true, false)
    }

    /// Read a string that stops at a line feed (`MSG_ReadStringLine`).
    pub fn string_line(&mut self, limit: usize) -> String {
        self.text(limit, false, true)
    }

    /// NetQuake coordinate (`Coord` / `MSG_ReadCoord`).
    pub fn coord(&mut self) -> Result<f64, MsgError> {
        Ok(q1_short_to_coord(self.short()?))
    }

    /// NetQuake angle (`Angle` / `MSG_ReadAngle`).
    pub fn angle(&mut self) -> Result<f64, MsgError> {
        Ok(q1_byte_angle_to_degrees(self.byte()?))
    }

    /// FitzQuake/RMQ coordinate under `PRFL_` flags (`CoordFlags`).
    pub fn coord_flags(&mut self, flags: u32) -> Result<f64, MsgError> {
        use crate::protocol::q1 as protocol;
        if (flags & protocol::PRFL_FLOATCOORD) != 0 {
            return Ok(f64::from(self.float()?));
        }
        if (flags & protocol::PRFL_INT32COORD) != 0 {
            return Ok(f64::from(self.long()?) / 16.0);
        }
        if (flags & protocol::PRFL_24BITCOORD) != 0 {
            return Ok(f64::from(self.short()?) + f64::from(self.byte()?) / 255.0);
        }
        self.coord()
    }

    /// FitzQuake/RMQ angle under `PRFL_` flags (`AngleFlags`).
    pub fn angle_flags(&mut self, flags: u32) -> Result<f64, MsgError> {
        use crate::protocol::q1 as protocol;
        if (flags & protocol::PRFL_FLOATANGLE) != 0 {
            return Ok(f64::from(self.float()?));
        }
        if (flags & protocol::PRFL_SHORTANGLE) != 0 {
            return Ok(q1_decode_short_angle(self.short()?));
        }
        self.angle()
    }

    /// FitzQuake/RMQ movement angle (`readMoveAngle16`).
    pub fn move_angle16(&mut self, flags: u32) -> Result<f64, MsgError> {
        use crate::protocol::q1 as protocol;
        if (flags & protocol::PRFL_FLOATANGLE) != 0 {
            return Ok(f64::from(self.float()?));
        }
        Ok(q1_decode_short_angle(self.short()?))
    }

    /// Quake II truncated angle byte decoded to degrees (`MSG_ReadAngle`).
    pub fn q2_angle(&mut self) -> Result<f64, MsgError> {
        Ok(q2_byte_angle_to_degrees(self.byte()?))
    }

    /// Quake II 16-bit angle decoded to degrees (`MSG_ReadAngle16`).
    pub fn q2_angle16(&mut self) -> Result<f64, MsgError> {
        Ok(crate::angles::q2_short_to_angle(self.short()?))
    }

    /// Quake II position triple (`MSG_ReadPos`).
    pub fn q2_pos(&mut self) -> Result<[f64; 3], MsgError> {
        Ok([self.coord()?, self.coord()?, self.coord()?])
    }

    /// QuakeWorld 16-bit angle decoded to degrees (`qwReadAngle16`).
    pub fn qw_angle16(&mut self) -> Result<f64, MsgError> {
        self.q2_angle16()
    }

    /// Borrow `length` bytes without copying (`bytes`).
    pub fn bytes(&mut self, length: usize) -> Result<&'a [u8], MsgError> {
        let offset = self.offset;
        let start = self.take(length).ok_or(MsgError::Truncated(offset))?;
        Ok(&self.data[start..start + length])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::q1 as protocol;

    #[test]
    fn scalars_round_trip() {
        let mut writer = MsgWriter::new(64, false);
        writer.write_byte(0xab).unwrap();
        writer.write_char(-7).unwrap();
        writer.write_short(-300).unwrap();
        writer.write_word(0xbeef).unwrap();
        writer.write_long(-123_456).unwrap();
        writer.write_float(1.5).unwrap();
        writer.write_string("hello").unwrap();
        writer.write_string_opt(None).unwrap();
        let bytes = writer.bytes().to_vec();

        let mut reader = MsgReader::new(&bytes);
        assert_eq!(reader.byte().unwrap(), 0xab);
        assert_eq!(reader.char().unwrap(), -7);
        assert_eq!(reader.short().unwrap(), -300);
        assert_eq!(reader.word().unwrap(), 0xbeef);
        assert_eq!(reader.long().unwrap(), -123_456);
        assert_eq!(reader.float().unwrap(), 1.5);
        assert_eq!(reader.string(2047), "hello");
        assert_eq!(reader.string(2047), "");
        assert_eq!(reader.remaining(), 0);
        reader.finish().unwrap();
    }

    #[test]
    fn overflow_modes_match_donor() {
        let mut strict = MsgWriter::new(2, false);
        strict.write_short(1).unwrap();
        assert_eq!(strict.write_byte(1), Err(MsgError::Overflow(2)));

        let mut lenient = MsgWriter::new(2, true);
        lenient.write_short(1).unwrap();
        lenient.write_byte(9).unwrap();
        assert!(lenient.overflowed());
        assert_eq!(lenient.bytes(), &[9]);
        assert_eq!(MsgWriter::new(1, false).reserve(2), Err(MsgError::BadSize(2)));
    }

    #[test]
    fn truncation_is_sticky_and_partial_strings_survive() {
        let mut reader = MsgReader::new(b"ab");
        assert_eq!(reader.string(2047), "ab");
        assert!(reader.badread());
        assert!(reader.finish().is_err());
        assert_eq!(reader.byte(), Err(MsgError::Truncated(2)));

        let mut reader = MsgReader::new(b"a\nb\0");
        assert_eq!(reader.string_quakeworld(2047), "ab");
        let mut reader = MsgReader::new(b"a\nb\0");
        assert_eq!(reader.string_line(2047), "a");
    }

    #[test]
    fn coord_and_angle_flags_round_trip() {
        for flags in [
            0,
            protocol::PRFL_SHORTANGLE,
            protocol::PRFL_FLOATANGLE,
            protocol::PRFL_24BITCOORD,
            protocol::PRFL_FLOATCOORD,
            protocol::PRFL_INT32COORD,
            protocol::PRFL_SHORTANGLE | protocol::PRFL_FLOATCOORD,
        ] {
            let mut writer = MsgWriter::new(32, false);
            writer.write_coord_flags(12.5, flags).unwrap();
            writer.write_angle_flags(90.0, flags).unwrap();
            writer.write_move_angle16(45.0, flags).unwrap();
            let bytes = writer.bytes().to_vec();
            let mut reader = MsgReader::new(&bytes);
            let coord = reader.coord_flags(flags).unwrap();
            let angle = reader.angle_flags(flags).unwrap();
            let moved = reader.move_angle16(flags).unwrap();
            reader.finish().unwrap();
            let expected_coord = match crate::angles::q1_encode_coord_flags(12.5, flags) {
                Q1CoordEncoding::Short(v) => f64::from(v) / 8.0,
                Q1CoordEncoding::ShortByte { whole, fraction } => f64::from(whole) + f64::from(fraction) / 255.0,
                Q1CoordEncoding::Long(v) => f64::from(v) / 16.0,
                Q1CoordEncoding::Float(v) => f64::from(v),
            };
            assert!((coord - expected_coord).abs() < 1e-6, "{flags} {coord}");
            assert!((angle - 90.0).abs() < 1.0, "{flags} {angle}");
            assert!((moved - 45.0).abs() < 1.0, "{flags} {moved}");
        }

        let mut writer = MsgWriter::new(32, false);
        writer.write_coord(-1.5).unwrap();
        writer.write_angle(90.0).unwrap();
        writer.write_q2_pos([1.0, 2.0, 3.0]).unwrap();
        writer.write_q2_angle(90.0).unwrap();
        writer.write_q2_angle16(90.0).unwrap();
        writer.write_qw_angle(180.0).unwrap();
        writer.write_qw_angle16(45.0).unwrap();
        let bytes = writer.bytes().to_vec();
        let mut reader = MsgReader::new(&bytes);
        assert_eq!(reader.coord().unwrap(), -1.5);
        assert_eq!(reader.angle().unwrap(), 90.0);
        assert_eq!(reader.q2_pos().unwrap(), [1.0, 2.0, 3.0]);
        assert_eq!(reader.q2_angle().unwrap(), 90.0);
        assert_eq!(reader.q2_angle16().unwrap(), 90.0);
        assert_eq!(reader.angle().unwrap(), -180.0);
        assert!((reader.qw_angle16().unwrap() - 45.0).abs() < 0.01);
        reader.finish().unwrap();
    }

    #[test]
    fn borrowed_bytes_share_input() {
        let input = [1u8, 2, 3, 4];
        let mut reader = MsgReader::new(&input);
        assert_eq!(reader.bytes(3).unwrap(), &[1, 2, 3]);
        assert!(reader.bytes(2).is_err());
    }
}
