//! Quake III bitstream and OOB message codecs.
//!
//! Donor provenance: `MessageWriter`, `MessageReader`,
//! `writeDeltaUserCommand`, and `readDeltaUserCommand` in
//! `src/network/q3/message.ts` (a port of id Software's `msg.c`), with
//! scalar angles from [`crate::angles`] and the Huffman layer from
//! [`crate::huffman`].
//!
//! The donor's `SourceMessageState` overflow/instrumentation counters are
//! engine diagnostics, not wire behavior, and are omitted. Scalar readers
//! keep the donor's `-1` overread sentinel instead of failing, so string
//! decoding matches byte for byte.

use thiserror::Error;

use crate::angles::{q3_angle_to_byte, q3_angle_to_short, AngleError};
use crate::huffman::{message_tree, HuffmanError};

/// Maximum message bytes (`MAX_MESSAGE_LENGTH`).
pub const MAX_MESSAGE_LENGTH: usize = 16384;
/// Maximum string characters (`MAX_STRING_CHARS`).
pub const MAX_STRING_CHARS: usize = 1024;
/// Maximum big-info-string characters (`BIG_INFO_STRING`).
pub const BIG_INFO_STRING: usize = 8192;

/// Message encoding mode (`MessageMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageMode {
    /// Huffman bitstream.
    #[default]
    Bitstream,
    /// Out-of-band aligned integers.
    Oob,
}

/// Error for Quake III message coding failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3MsgError {
    /// Bit widths must be nonzero within -31..=32 (OOB: 8, 16, 32).
    #[error("invalid message bit width")]
    BadWidth,
    /// The read ran past the message bits.
    #[error("truncated message bits at byte {0}")]
    Truncated(usize),
    /// The OOB read ran past the message.
    #[error("truncated OOB integer at byte {0}")]
    TruncatedOob(usize),
    /// The message exceeds the maximum length.
    #[error("message exceeds maximum length")]
    MessageTooLong,
    /// Invalid capacity or data length.
    #[error("invalid message data length")]
    BadLength,
    /// Keyed delta mask index is outside kbitmask.
    #[error("keyed delta mask index is outside kbitmask")]
    BadDeltaMask,
    /// Invalid Huffman tree path.
    #[error("{0}")]
    Huffman(#[from] HuffmanError),
    /// Undefined angle conversion.
    #[error("{0}")]
    Angle(#[from] AngleError),
}

fn width_of(bits: i32, mode: MessageMode) -> Result<u32, Q3MsgError> {
    if bits == 0 || !(-31..=32).contains(&bits) {
        return Err(Q3MsgError::BadWidth);
    }
    let width = bits.unsigned_abs();
    if mode == MessageMode::Oob && width != 8 && width != 16 && width != 32 {
        return Err(Q3MsgError::BadWidth);
    }
    Ok(width)
}

/// Sign-extend a decoded value, mirroring the donor's JS shift semantics
/// (shift counts wrap mod 32).
fn sign_extend(value: i32, bits: i32, sign_width: i32) -> i32 {
    if bits >= 0 {
        return value;
    }
    let test = 1i32.wrapping_shl(((sign_width - 1) & 31) as u32);
    if (value & test) == 0 {
        return value;
    }
    let shifted = 1i32.wrapping_shl((sign_width & 31) as u32);
    value | (-1 ^ shifted.wrapping_sub(1))
}

/// Quake III message writer (`MessageWriter`).
#[derive(Debug, Clone)]
pub struct Q3MsgWriter {
    buffer: Vec<u8>,
    position: usize,
    size: usize,
    overflowed: bool,
    mode: MessageMode,
}

impl Q3MsgWriter {
    /// Allocate a writer with `capacity` bytes in `mode`.
    pub fn new(mode: MessageMode, capacity: usize) -> Result<Self, Q3MsgError> {
        if capacity > MAX_MESSAGE_LENGTH {
            return Err(Q3MsgError::BadLength);
        }
        Ok(Self {
            buffer: vec![0; capacity],
            position: 0,
            size: 0,
            overflowed: false,
            mode,
        })
    }

    /// Bit position.
    #[must_use]
    pub fn bit_position(&self) -> usize {
        self.position
    }

    /// Encoding mode.
    #[must_use]
    pub fn mode(&self) -> MessageMode {
        self.mode
    }

    /// Byte length.
    #[must_use]
    pub fn byte_length(&self) -> usize {
        self.size
    }

    /// Whether a write overflowed the capacity.
    #[must_use]
    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Written bytes.
    #[must_use]
    pub fn to_bytes(&self) -> &[u8] {
        &self.buffer[..self.size]
    }

    /// Reset cursor and overflow state, keeping storage and mode.
    pub fn clear(&mut self) {
        self.position = 0;
        self.size = 0;
        self.overflowed = false;
    }

    /// Switch to bitstream mode.
    pub fn bitstream(&mut self) {
        self.mode = MessageMode::Bitstream;
    }

    /// Copy the writer, storage included.
    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Write `bits` of `value` (`MSG_WriteBits`).
    pub fn write_bits(&mut self, value: i32, bits: i32) -> Result<(), Q3MsgError> {
        if self.buffer.len() - self.size < 4 {
            self.overflowed = true;
            return Ok(());
        }
        let width = width_of(bits, MessageMode::Bitstream)? as usize;
        if self.mode == MessageMode::Oob {
            width_of(bits, self.mode)?;
            if width == 8 {
                self.buffer[self.size] = value as u8;
            } else if width == 16 {
                let bytes = (value as u16).to_le_bytes();
                self.buffer[self.size..self.size + 2].copy_from_slice(&bytes);
            } else {
                let bytes = value.to_le_bytes();
                self.buffer[self.size..self.size + 4].copy_from_slice(&bytes);
            }
            self.size += width >> 3;
            // The original OOB writer advances bit by eight for a long;
            // readers advance by 32.
            self.position += if width == 32 { 8 } else { width };
            return Ok(());
        }
        let mut remaining = value as u32;
        let mut encoded: Vec<u8> = Vec::new();
        for _ in 0..(width & 7) {
            encoded.push((remaining & 1) as u8);
            remaining >>= 1;
        }
        let tree = message_tree();
        for _ in (width & 7..width).step_by(8) {
            tree.encode_symbol((remaining & 255) as u8, &mut |bit| encoded.push(bit));
            remaining >>= 8;
        }
        let size = ((self.position + encoded.len()) >> 3) + 1;
        if size > self.buffer.len() {
            self.overflowed = true;
            return Ok(());
        }
        for bit in encoded {
            let offset = self.position >> 3;
            if (self.position & 7) == 0 {
                self.buffer[offset] = 0;
            }
            self.buffer[offset] |= bit << (self.position & 7);
            self.position += 1;
        }
        self.size = size;
        Ok(())
    }

    /// Write a byte.
    pub fn write_byte(&mut self, value: i32) -> Result<(), Q3MsgError> {
        self.write_bits(value, 8)
    }

    /// Write a char.
    pub fn write_char(&mut self, value: i32) -> Result<(), Q3MsgError> {
        self.write_bits(value, 8)
    }

    /// Write a short.
    pub fn write_short(&mut self, value: i32) -> Result<(), Q3MsgError> {
        self.write_bits(value, 16)
    }

    /// Write a long.
    pub fn write_long(&mut self, value: i32) -> Result<(), Q3MsgError> {
        self.write_bits(value, 32)
    }

    /// Write a float by bit pattern.
    pub fn write_float(&mut self, value: f32) -> Result<(), Q3MsgError> {
        self.write_long(value.to_bits() as i32)
    }

    /// Write raw bytes.
    pub fn write_data(&mut self, data: &[u8]) -> Result<(), Q3MsgError> {
        for byte in data {
            self.write_byte(i32::from(*byte))?;
        }
        Ok(())
    }

    fn write_text(&mut self, value: Option<&str>, limit: usize) -> Result<(), Q3MsgError> {
        let Some(text) = value else {
            return self.write_byte(0);
        };
        let nul = text.find('\0').unwrap_or(text.len());
        let text = &text[..nul];
        if text.chars().count() < limit {
            for ch in text.chars() {
                let byte = if ch as u32 > 127 { b'.' } else { ch as u8 };
                self.write_byte(i32::from(byte))?;
            }
        }
        self.write_byte(0)
    }

    /// Write a string (`writeString`).
    pub fn write_string(&mut self, value: Option<&str>) -> Result<(), Q3MsgError> {
        self.write_text(value, MAX_STRING_CHARS)
    }

    /// Write a big string (`writeBigString`).
    pub fn write_big_string(&mut self, value: Option<&str>) -> Result<(), Q3MsgError> {
        self.write_text(value, BIG_INFO_STRING)
    }

    /// Write an angle byte (`writeAngle`).
    pub fn write_angle(&mut self, value: f64) -> Result<(), Q3MsgError> {
        let byte = q3_angle_to_byte(value)?;
        self.write_byte(i32::from(byte))
    }

    /// Write a 16-bit angle (`writeAngle16`).
    pub fn write_angle16(&mut self, value: f64) -> Result<(), Q3MsgError> {
        let word = q3_angle_to_short(value)?;
        self.write_short(i32::from(word))
    }

    /// Write a delta integer (`writeDelta`).
    pub fn write_delta(&mut self, previous: i32, value: i32, bits: i32) -> Result<(), Q3MsgError> {
        self.write_bits(i32::from(previous != value), 1)?;
        if previous != value {
            self.write_bits(value, bits)?;
        }
        Ok(())
    }

    /// Write a keyed delta integer (`writeDeltaKey`).
    pub fn write_delta_key(&mut self, key: i32, previous: i32, value: i32, bits: i32) -> Result<(), Q3MsgError> {
        self.write_bits(i32::from(previous != value), 1)?;
        if previous != value {
            self.write_bits(value ^ key, bits)?;
        }
        Ok(())
    }

    /// Write a delta float (`writeDeltaFloat`).
    pub fn write_delta_float(&mut self, previous: f32, value: f32) -> Result<(), Q3MsgError> {
        self.write_bits(i32::from(previous != value), 1)?;
        if previous != value {
            self.write_bits(value.to_bits() as i32, 32)?;
        }
        Ok(())
    }

    /// Write a keyed delta float (`writeDeltaKeyFloat`).
    pub fn write_delta_key_float(&mut self, key: i32, previous: f32, value: f32) -> Result<(), Q3MsgError> {
        self.write_bits(i32::from(previous != value), 1)?;
        if previous != value {
            self.write_bits((value.to_bits() ^ key as u32) as i32, 32)?;
        }
        Ok(())
    }
}

/// Quake III message reader (`MessageReader`).
#[derive(Debug, Clone)]
pub struct Q3MsgReader<'a> {
    data: &'a [u8],
    position: usize,
    count: usize,
    mode: MessageMode,
}

impl<'a> Q3MsgReader<'a> {
    /// Borrow a message buffer.
    pub fn new(data: &'a [u8], mode: MessageMode) -> Result<Self, Q3MsgError> {
        if data.len() > MAX_MESSAGE_LENGTH {
            return Err(Q3MsgError::MessageTooLong);
        }
        Ok(Self {
            data,
            position: 0,
            count: 0,
            mode,
        })
    }

    /// Bit position.
    #[must_use]
    pub fn bit_position(&self) -> usize {
        self.position
    }

    /// Read count in bytes.
    #[must_use]
    pub fn read_count(&self) -> usize {
        self.count
    }

    /// Encoding mode.
    #[must_use]
    pub fn mode(&self) -> MessageMode {
        self.mode
    }

    /// Switch to bitstream mode.
    pub fn bitstream(&mut self) {
        self.mode = MessageMode::Bitstream;
    }

    /// Reset to the start in bitstream mode.
    pub fn begin_reading(&mut self) {
        self.position = 0;
        self.count = 0;
        self.mode = MessageMode::Bitstream;
    }

    /// Reset to the start in OOB mode.
    pub fn begin_reading_oob(&mut self) {
        self.position = 0;
        self.count = 0;
        self.mode = MessageMode::Oob;
    }

    fn get_bit(&mut self) -> Result<u8, Q3MsgError> {
        if self.position >= self.data.len() * 8 {
            return Err(Q3MsgError::Truncated(self.position >> 3));
        }
        let bit = (self.data[self.position >> 3] >> (self.position & 7)) & 1;
        self.position += 1;
        Ok(bit)
    }

    /// Read `bits` of an integer (`MSG_ReadBits`).
    pub fn read_bits(&mut self, bits: i32) -> Result<i32, Q3MsgError> {
        let width = if bits == 0 && self.mode == MessageMode::Bitstream {
            0
        } else {
            width_of(bits, self.mode)?
        } as usize;
        if self.mode == MessageMode::Oob {
            let need = width >> 3;
            if self.count + need > self.data.len() {
                return Err(Q3MsgError::TruncatedOob(self.count));
            }
            let value = if width == 8 {
                i32::from(self.data[self.count])
            } else if width == 16 {
                i32::from(u16::from_le_bytes([self.data[self.count], self.data[self.count + 1]]))
            } else {
                i32::from_le_bytes([
                    self.data[self.count],
                    self.data[self.count + 1],
                    self.data[self.count + 2],
                    self.data[self.count + 3],
                ])
            };
            self.count += need;
            self.position += width;
            return Ok(sign_extend(value, bits, width as i32));
        }
        let mut raw: u32 = 0;
        let low_bits = width & 7;
        for index in 0..low_bits {
            raw |= u32::from(self.get_bit()?) << index;
        }
        let tree = message_tree();
        for index in (low_bits..width).step_by(8) {
            let mut get_bit = || self.get_bit();
            let symbol = tree
                .decode_prefix_checked(&mut get_bit)?
                .ok_or(Q3MsgError::Huffman(HuffmanError::InvalidTree))?;
            raw |= u32::from(symbol) << index;
        }
        self.count = (self.position >> 3) + 1;
        // MSG_ReadBits mutates bits after raw low bits, including for
        // sign extension.
        Ok(sign_extend(raw as i32, bits, (width - low_bits) as i32))
    }

    fn read_scalar(&mut self, bits: i32) -> Result<i32, Q3MsgError> {
        let value = self.read_bits(bits)?;
        if self.count > self.data.len() {
            return Ok(-1);
        }
        Ok(value)
    }

    /// Read a byte; `-1` marks overread.
    pub fn read_byte(&mut self) -> Result<i32, Q3MsgError> {
        let value = self.read_scalar(8)?;
        if value == -1 {
            return Ok(-1);
        }
        Ok(value & 255)
    }

    /// Read a char; `-1` marks overread.
    pub fn read_char(&mut self) -> Result<i32, Q3MsgError> {
        Ok((self.read_scalar(8)? << 24) >> 24)
    }

    /// Read a short; `-1` marks overread.
    pub fn read_short(&mut self) -> Result<i32, Q3MsgError> {
        Ok((self.read_scalar(16)? << 16) >> 16)
    }

    /// Read a long; `-1` marks overread.
    pub fn read_long(&mut self) -> Result<i32, Q3MsgError> {
        self.read_scalar(32)
    }

    /// Read a float; `-1.0` marks overread.
    pub fn read_float(&mut self) -> Result<f32, Q3MsgError> {
        let value = self.read_bits(32)?;
        if self.count > self.data.len() {
            return Ok(-1.0);
        }
        Ok(f32::from_bits(value as u32))
    }

    /// Read raw bytes (`readData`); overread bytes wrap to 255 like the donor.
    pub fn read_data(&mut self, length: usize) -> Result<Vec<u8>, Q3MsgError> {
        if length > MAX_MESSAGE_LENGTH {
            return Err(Q3MsgError::BadLength);
        }
        let mut data = Vec::with_capacity(length);
        for _ in 0..length {
            data.push(self.read_byte()? as u8);
        }
        Ok(data)
    }

    fn read_text(&mut self, limit: usize, newline: bool, ascii: bool) -> Result<String, Q3MsgError> {
        let mut text = String::new();
        for _ in 0..limit - 1 {
            let ch = self.read_byte()?;
            if ch == -1 || ch == 0 || (newline && ch == 10) {
                break;
            }
            let byte = if ch == 37 || (ascii && ch > 127) {
                b'.'
            } else {
                ch as u8
            };
            text.push(char::from(byte));
        }
        Ok(text)
    }

    /// Read a string (`readString`).
    pub fn read_string(&mut self) -> Result<String, Q3MsgError> {
        self.read_text(MAX_STRING_CHARS, false, true)
    }

    /// Read a big string (`readBigString`).
    pub fn read_big_string(&mut self) -> Result<String, Q3MsgError> {
        self.read_text(BIG_INFO_STRING, false, false)
    }

    /// Read a string line (`readStringLine`).
    pub fn read_string_line(&mut self) -> Result<String, Q3MsgError> {
        self.read_text(MAX_STRING_CHARS, true, false)
    }

    /// Read a 16-bit angle in degrees (`readAngle16`).
    pub fn read_angle16(&mut self) -> Result<f64, Q3MsgError> {
        Ok(f64::from(self.read_short()?) * (360.0 / 65536.0))
    }

    /// Read a delta integer (`readDelta`).
    pub fn read_delta(&mut self, previous: i32, bits: i32) -> Result<i32, Q3MsgError> {
        if self.read_bits(1)? == 0 {
            return Ok(previous);
        }
        self.read_bits(bits)
    }

    /// Read a keyed delta integer (`readDeltaKey`).
    pub fn read_delta_key(&mut self, key: i32, previous: i32, bits: i32) -> Result<i32, Q3MsgError> {
        if self.read_bits(1)? == 0 {
            return Ok(previous);
        }
        let value = self.read_bits(bits)?;
        if !(0..32).contains(&bits) {
            return Err(Q3MsgError::BadDeltaMask);
        }
        let mask = 0xffff_ffffu32 >> (31 - bits as u32);
        Ok(value ^ ((key as u32 & mask) as i32))
    }

    /// Read a delta float (`readDeltaFloat`).
    pub fn read_delta_float(&mut self, previous: f32) -> Result<f32, Q3MsgError> {
        if self.read_bits(1)? == 0 {
            return Ok(previous);
        }
        Ok(f32::from_bits(self.read_bits(32)? as u32))
    }

    /// Read a keyed delta float (`readDeltaKeyFloat`).
    pub fn read_delta_key_float(&mut self, key: i32, previous: f32) -> Result<f32, Q3MsgError> {
        if self.read_bits(1)? == 0 {
            return Ok(previous);
        }
        Ok(f32::from_bits(self.read_bits(32)? as u32 ^ key as u32))
    }
}

/// Wire user command (`WireUserCommand`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WireUserCommand {
    /// Server time.
    pub server_time: i32,
    /// View angles as wire shorts.
    pub angles: [i16; 3],
    /// Forward, right, and up moves.
    pub moves: [i8; 3],
    /// Buttons.
    pub buttons: u16,
    /// Weapon.
    pub weapon: u8,
}

impl WireUserCommand {
    fn fields(&self) -> [i32; 8] {
        [
            i32::from(self.angles[0]),
            i32::from(self.angles[1]),
            i32::from(self.angles[2]),
            i32::from(self.moves[0]),
            i32::from(self.moves[1]),
            i32::from(self.moves[2]),
            i32::from(self.buttons),
            i32::from(self.weapon),
        ]
    }
}

const COMMAND_WIDTHS: [i32; 8] = [16, 16, 16, 8, 8, 8, 16, 8];

/// Write a delta user command (`writeDeltaUsercmd`).
///
/// `None` selects the plain encoding; `Some` selects the keyed encoding.
pub fn write_delta_user_command(
    writer: &mut Q3MsgWriter,
    from: &WireUserCommand,
    to: &WireUserCommand,
    key: Option<i32>,
) -> Result<(), Q3MsgError> {
    let difference = to.server_time.wrapping_sub(from.server_time);
    if difference < 256 {
        writer.write_bits(1, 1)?;
        writer.write_bits(difference, 8)?;
    } else {
        writer.write_bits(0, 1)?;
        writer.write_bits(to.server_time, 32)?;
    }
    let previous = from.fields();
    let next = to.fields();
    if key.is_some() {
        let changed = previous.iter().zip(next.iter()).any(|(a, b)| a != b);
        writer.write_bits(i32::from(changed), 1)?;
        if !changed {
            return Ok(());
        }
    }
    for (index, width) in COMMAND_WIDTHS.iter().enumerate() {
        let old = previous[index];
        let value = next[index];
        if let Some(key) = key {
            writer.write_delta_key(key ^ to.server_time, old, value, *width)?;
        } else {
            writer.write_delta(old, value, *width)?;
        }
    }
    Ok(())
}

/// Read a delta user command (`readDeltaUserCommand`).
pub fn read_delta_user_command(
    reader: &mut Q3MsgReader<'_>,
    from: &WireUserCommand,
    key: Option<i32>,
) -> Result<WireUserCommand, Q3MsgError> {
    let server_time = if reader.read_bits(1)? != 0 {
        from.server_time.wrapping_add(reader.read_bits(8)?)
    } else {
        reader.read_bits(32)?
    };
    if key.is_some() && reader.read_bits(1)? == 0 {
        return Ok(WireUserCommand {
            server_time,
            ..from.clone()
        });
    }
    let mut fields = [0i32; 8];
    let previous = from.fields();
    for (index, width) in COMMAND_WIDTHS.iter().enumerate() {
        fields[index] = if let Some(key) = key {
            reader.read_delta_key(key ^ server_time, previous[index], *width)?
        } else {
            reader.read_delta(previous[index], *width)?
        };
    }
    Ok(WireUserCommand {
        server_time,
        angles: [fields[0] as i16, fields[1] as i16, fields[2] as i16],
        moves: [
            ((fields[3] << 24) >> 24) as i8,
            ((fields[4] << 24) >> 24) as i8,
            ((fields[5] << 24) >> 24) as i8,
        ],
        buttons: fields[6] as u16,
        weapon: (fields[7] & 255) as u8,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn writer() -> Q3MsgWriter {
        Q3MsgWriter::new(MessageMode::Bitstream, MAX_MESSAGE_LENGTH).unwrap()
    }

    #[test]
    fn scalars_round_trip() {
        let mut writer = writer();
        writer.write_byte(200).unwrap();
        writer.write_char(-7).unwrap();
        writer.write_short(-300).unwrap();
        writer.write_long(-123_456).unwrap();
        writer.write_float(1.5).unwrap();
        writer.write_data(&[1, 2, 3]).unwrap();
        writer.write_string(Some("hello")).unwrap();
        writer.write_string(None).unwrap();
        writer.write_big_string(Some("big")).unwrap();
        let bytes = writer.to_bytes().to_vec();

        let mut reader = Q3MsgReader::new(&bytes, MessageMode::Bitstream).unwrap();
        assert_eq!(reader.read_byte().unwrap(), 200);
        assert_eq!(reader.read_char().unwrap(), -7);
        assert_eq!(reader.read_short().unwrap(), -300);
        assert_eq!(reader.read_long().unwrap(), -123_456);
        assert_eq!(reader.read_float().unwrap(), 1.5);
        assert_eq!(reader.read_data(3).unwrap(), vec![1, 2, 3]);
        assert_eq!(reader.read_string().unwrap(), "hello");
        assert_eq!(reader.read_string().unwrap(), "");
        assert_eq!(reader.read_big_string().unwrap(), "big");
    }

    #[test]
    fn oob_mode_round_trip() {
        let mut writer = Q3MsgWriter::new(MessageMode::Oob, MAX_MESSAGE_LENGTH).unwrap();
        writer.write_byte(200).unwrap();
        writer.write_short(0x1234).unwrap();
        writer.write_long(-99).unwrap();
        assert_eq!(writer.bit_position(), 8 + 16 + 8);
        let bytes = writer.to_bytes().to_vec();
        assert_eq!(bytes, vec![200, 0x34, 0x12, 157, 255, 255, 255]);

        let mut reader = Q3MsgReader::new(&bytes, MessageMode::Oob).unwrap();
        assert_eq!(reader.read_byte().unwrap(), 200);
        assert_eq!(reader.read_short().unwrap(), 0x1234);
        assert_eq!(reader.read_long().unwrap(), -99);
        assert_eq!(reader.bit_position(), 8 + 16 + 32);
    }

    #[test]
    fn angles_and_deltas_round_trip() {
        let mut writer = writer();
        writer.write_angle(90.0).unwrap();
        writer.write_angle16(90.0).unwrap();
        writer.write_delta(7, 7, 8).unwrap();
        writer.write_delta(7, 200, 8).unwrap();
        writer.write_delta_key(0x1234, 7, 200, 8).unwrap();
        writer.write_delta_float(1.0, 1.0).unwrap();
        writer.write_delta_float(1.0, 2.5).unwrap();
        writer.write_delta_key_float(99, 1.0, 2.5).unwrap();
        let bytes = writer.to_bytes().to_vec();

        let mut reader = Q3MsgReader::new(&bytes, MessageMode::Bitstream).unwrap();
        assert_eq!(reader.read_byte().unwrap(), 64);
        assert!((reader.read_angle16().unwrap() - 90.0).abs() < 0.01);
        assert_eq!(reader.read_delta(7, 8).unwrap(), 7);
        assert_eq!(reader.read_delta(7, 8).unwrap(), 200);
        assert_eq!(reader.read_delta_key(0x1234, 7, 8).unwrap(), 200);
        assert_eq!(reader.read_delta_float(1.0).unwrap(), 1.0);
        assert_eq!(reader.read_delta_float(1.0).unwrap(), 2.5);
        assert_eq!(reader.read_delta_key_float(99, 1.0).unwrap(), 2.5);
    }

    #[test]
    fn user_commands_round_trip_plain_and_keyed() {
        for key in [None, Some(0x5eed_1234)] {
            let from = WireUserCommand::default();
            let to = WireUserCommand {
                server_time: 1200,
                angles: [1000, -2000, 3000],
                moves: [100, -100, 10],
                buttons: 0x1f,
                weapon: 7,
            };
            let mut writer = writer();
            write_delta_user_command(&mut writer, &from, &to, key).unwrap();
            write_delta_user_command(&mut writer, &to, &to, key).unwrap();
            let mut far = to.clone();
            far.server_time = to.server_time + 500;
            write_delta_user_command(&mut writer, &to, &far, key).unwrap();
            let bytes = writer.to_bytes().to_vec();

            let mut reader = Q3MsgReader::new(&bytes, MessageMode::Bitstream).unwrap();
            let decoded = read_delta_user_command(&mut reader, &from, key).unwrap();
            assert_eq!(decoded, to);
            let decoded = read_delta_user_command(&mut reader, &decoded, key).unwrap();
            assert_eq!(decoded, to);
            let decoded = read_delta_user_command(&mut reader, &decoded, key).unwrap();
            assert_eq!(decoded, far);
        }
    }

    #[test]
    fn overread_keeps_sentinels_and_widths_reject() {
        let mut writer = writer();
        assert!(writer.write_bits(1, 0).is_err());
        assert!(writer.write_bits(1, 33).is_err());
        assert!(writer.write_bits(1, -32).is_err());
        let mut oob = Q3MsgWriter::new(MessageMode::Oob, MAX_MESSAGE_LENGTH).unwrap();
        assert!(oob.write_bits(1, 7).is_err());

        writer.write_byte(5).unwrap();
        let bytes = writer.to_bytes().to_vec();
        let mut reader = Q3MsgReader::new(&bytes, MessageMode::Bitstream).unwrap();
        assert_eq!(reader.read_byte().unwrap(), 5);
        assert!(reader.read_byte().is_err());

        // A scalar decode that ends exactly at the buffer edge reports
        // the -1 sentinel instead of failing; find one deterministically.
        let mut saw_sentinel = false;
        for byte in 0..=255u8 {
            let mut reader = Q3MsgReader::new(std::slice::from_ref(&byte), MessageMode::Bitstream).unwrap();
            if let Ok(-1) = reader.read_byte() {
                saw_sentinel = true;
                break;
            }
        }
        assert!(saw_sentinel);

        let empty: &[u8] = &[];
        let mut reader = Q3MsgReader::new(empty, MessageMode::Oob).unwrap();
        assert!(reader.read_byte().is_err());
        assert_eq!(sign_extend(0x80, -8, 8), -128);
        assert_eq!(sign_extend(0x7f, -8, 8), 0x7f);
    }
}
