//! Shared borrowed native message streams. No packet owns a growable buffer.
//! Q3 msg.c writes low remainder bits, then fixed Huffman codes for whole bytes.
mod adaptive;
mod codes;
pub use adaptive::AdaptiveHuffman;
use codes::CODES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// NQ/QW/Q2 messages and every out-of-band header, little endian.
    Bytes,
    /// Uncompressed LSB-first bits for native dialects that select them.
    Bits,
    /// Q3 protocol 68 MSG bitstream (not adaptive connect compression).
    Q3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Width,
    Truncated,
    Capacity,
    Symbol,
    OutOfBand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub byte: usize,
    pub kind: ErrorKind,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "message {:?} at byte {}", self.kind, self.byte)
    }
}
impl std::error::Error for Error {}

const fn decode_table() -> [(u16, u8); 2048] {
    let mut table = [(256, 0); 2048];
    let mut symbol = 0;
    while symbol < CODES.len() {
        let (code, width) = CODES[symbol];
        let mut suffix = 0;
        while suffix < 1 << (11 - width) {
            table[code as usize | (suffix << width)] = (symbol as u16, width);
            suffix += 1;
        }
        symbol += 1;
    }
    table
}
const DECODE: [(u16, u8); 2048] = decode_table();

pub const COMMAND_STRING_BYTES: usize = 1024;
pub const LEGACY_STRING_BYTES: usize = 2048;

pub(crate) const Q3_MAX_BYTE_BITS: usize = {
    let mut maximum = 0;
    let mut index = 0;
    while index < CODES.len() {
        if CODES[index].1 as usize > maximum {
            maximum = CODES[index].1 as usize;
        }
        index += 1;
    }
    maximum
};

/// Maximum native Huffman width, including the raw low remainder bits.
pub(crate) const fn q3_max_bits(width: u8) -> usize {
    (width % 8) as usize + (width / 8) as usize * Q3_MAX_BYTE_BITS
}

/// The caller has already checked the native bit width.
pub(crate) fn q3_bits(value: u32, width: u8) -> usize {
    let mut shift = width % 8;
    let mut bits = usize::from(shift);
    while shift < width {
        bits += usize::from(CODES[((value >> shift) & 255) as usize].1);
        shift += 8;
    }
    bits
}

/// Native string boundaries are wire policy, separate from a caller's storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StringPolicy {
    maximum: usize,
    signed_terminator: bool,
    stop_at_limit: bool,
    sanitize: bool,
    sanitize_high: bool,
    eof_terminator: bool,
}
impl StringPolicy {
    /// NQ/QW/Q2 MSG_ReadChar strings: 0xff terminates, and a full native
    /// 2048-byte buffer stops before reading the next byte.
    pub const LEGACY: Self = Self {
        maximum: LEGACY_STRING_BYTES - 1,
        signed_terminator: true,
        stop_at_limit: true,
        sanitize: false,
        sanitize_high: false,
        eof_terminator: false,
    };
    /// Rerelease/q2proto strings borrow every byte through their NUL.
    pub const UNSIGNED: Self = Self {
        maximum: usize::MAX,
        signed_terminator: false,
        stop_at_limit: false,
        sanitize: false,
        sanitize_high: false,
        eof_terminator: false,
    };
    /// Q3 MSG_ReadString keeps its original 1024-byte buffer and conversion.
    pub const Q3: Self = Self {
        maximum: COMMAND_STRING_BYTES - 1,
        signed_terminator: false,
        stop_at_limit: true,
        sanitize: true,
        sanitize_high: true,
        eof_terminator: false,
    };
    /// Native MSG_ReadBigString sanitizes percent signs, preserving high bytes.
    pub const Q3_BIG: Self = Self {
        maximum: 8191,
        sanitize_high: false,
        ..Self::Q3
    };
    pub const fn limited(self, maximum: usize) -> Self {
        Self { maximum, ..self }
    }
    pub(crate) const fn allow_eof(self) -> Self {
        Self {
            eof_terminator: true,
            ..self
        }
    }
    pub(crate) const fn reserved(self, bytes: usize) -> Self {
        Self {
            maximum: self.maximum.saturating_sub(bytes),
            ..self
        }
    }
    fn terminates(self, byte: u8) -> bool {
        byte == 0 || (self.signed_terminator && byte == 255)
    }
    /// Connectionless native strings may end at the packet's EOF.
    pub(crate) fn tail(self, text: &[u8]) -> &[u8] {
        let length = text
            .iter()
            .position(|&byte| self.terminates(byte))
            .unwrap_or(text.len());
        &text[..length.min(self.maximum)]
    }
    /// Generated strings leave room for the terminator in a native reader
    /// that stops at its fixed buffer limit. Raw native packets are not mapped.
    pub(crate) fn payload(self, text: &[u8]) -> &[u8] {
        let text = self.tail(text);
        let maximum = if self.stop_at_limit {
            self.maximum.saturating_sub(1)
        } else {
            self.maximum
        };
        &text[..text.len().min(maximum)]
    }
    pub fn truncates(self, text: &[u8]) -> bool {
        text.get(self.payload(text).len())
            .is_some_and(|&byte| !self.terminates(byte))
    }
    pub(crate) fn read_byte(self, byte: u8) -> u8 {
        if self.sanitize && ((self.sanitize_high && byte > 127) || byte == b'%') {
            b'.'
        } else {
            byte
        }
    }
    pub(crate) fn write_byte(self, byte: u8) -> u8 {
        if self.sanitize && byte > 127 {
            b'.'
        } else {
            byte
        }
    }
    /// Native Q3 writers emit an empty string on an oversized input; existing
    /// legacy generated rows retain their destination truncation policy.
    pub(crate) fn write_bytes(self, text: &[u8]) -> impl Iterator<Item = u8> + '_ {
        let text = if self.sanitize {
            let n = qa_core::text::until_nul(text).len();
            &text[..if n <= self.maximum { n } else { 0 }]
        } else {
            self.payload(text)
        };
        text.iter()
            .copied()
            .map(move |byte| self.write_byte(byte))
            .chain(std::iter::once(0))
    }
}

fn valid_width(encoding: Encoding, width: u8) -> bool {
    match encoding {
        Encoding::Bytes => matches!(width, 8 | 16 | 32),
        Encoding::Bits | Encoding::Q3 => (1..=32).contains(&width),
    }
}

/// Offset errors drop one message at its boundary; the reader never grows data.
#[derive(Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    encoding: Encoding,
    bit: usize,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], encoding: Encoding) -> Self {
        Self {
            data,
            encoding,
            bit: 0,
        }
    }
    pub fn out_of_band(data: &'a [u8]) -> Result<Self, Error> {
        let mut reader = Self::new(data, Encoding::Bytes);
        if reader.read_bits(32)? != u32::MAX {
            return Err(Error {
                byte: 0,
                kind: ErrorKind::OutOfBand,
            });
        }
        Ok(reader)
    }
    pub fn bit_position(&self) -> usize {
        self.bit
    }
    pub fn byte_position(&self) -> usize {
        self.bit / 8
    }
    pub(crate) fn error(&self, kind: ErrorKind) -> Error {
        Error {
            byte: self.byte_position(),
            kind,
        }
    }
    fn raw(&mut self, width: u8) -> Result<u32, Error> {
        let start = self.byte_position();
        let shift = self.bit % 8;
        let size = (shift + usize::from(width)).div_ceil(8);
        let Some(bytes) = self.data.get(start..start.saturating_add(size)) else {
            return Err(Error {
                byte: self.data.len(),
                kind: ErrorKind::Truncated,
            });
        };
        let mut value = 0u64;
        for (i, &byte) in bytes.iter().enumerate() {
            value |= u64::from(byte) << (i * 8);
        }
        self.bit += usize::from(width);
        Ok(((value >> shift) & ((1u64 << width) - 1)) as u32)
    }
    fn symbol(&mut self) -> Result<u32, Error> {
        let start = self.bit / 8;
        let shift = self.bit % 8;
        let Some(&first) = self.data.get(start) else {
            return Err(Error {
                byte: self.data.len(),
                kind: ErrorKind::Truncated,
            });
        };
        let lookahead = u32::from(first)
            | self.data.get(start + 1).map_or(0, |b| u32::from(*b)) << 8
            | self.data.get(start + 2).map_or(0, |b| u32::from(*b)) << 16;
        let (symbol, width) = DECODE[((lookahead >> shift) & 2047) as usize];
        if usize::from(width) > (self.data.len() - start) * 8 - shift {
            // The former bit walk consumed every available bit before this error.
            self.bit = self.data.len() * 8;
            return Err(Error {
                byte: self.data.len(),
                kind: ErrorKind::Truncated,
            });
        }
        self.bit += usize::from(width);
        if symbol < 256 {
            Ok(u32::from(symbol))
        } else {
            Err(self.error(ErrorKind::Symbol))
        }
    }
    pub fn read_bits(&mut self, width: u8) -> Result<u32, Error> {
        if !valid_width(self.encoding, width) {
            return Err(self.error(ErrorKind::Width));
        }
        if self.encoding != Encoding::Q3 {
            return self.raw(width);
        }
        let low = width % 8;
        let mut value = if low == 0 { 0 } else { self.raw(low)? };
        let mut shift = low;
        while shift < width {
            value |= self.symbol()? << shift;
            shift += 8;
        }
        Ok(value)
    }
    pub fn read_signed(&mut self, width: u8) -> Result<i32, Error> {
        let value = self.read_bits(width)?;
        Ok(sign_extend(value, width))
    }
    pub fn read_float(&mut self) -> Result<f32, Error> {
        self.read_bits(32).map(f32::from_bits)
    }
    pub fn read_data(&mut self, output: &mut [u8]) -> Result<(), Error> {
        for byte in output {
            *byte = self.read_bits(8)? as u8;
        }
        Ok(())
    }
    fn string(
        &mut self,
        policy: StringPolicy,
        mut store: impl FnMut(usize, u8),
    ) -> Result<usize, Error> {
        let mut length = 0;
        loop {
            if policy.stop_at_limit && length == policy.maximum {
                return Ok(length);
            }
            if policy.eof_terminator && self.bit == self.data.len() * 8 {
                // MSG_ReadChar advances its cursor on the failed EOF read.
                // Accepted command effects precede any subsequent badread.
                self.bit += 8;
                return if length <= policy.maximum {
                    Ok(length)
                } else {
                    Err(self.error(ErrorKind::Capacity))
                };
            }
            let byte = self.read_bits(8)? as u8;
            if policy.terminates(byte) {
                return if length <= policy.maximum {
                    Ok(length)
                } else {
                    Err(self.error(ErrorKind::Capacity))
                };
            }
            if length < policy.maximum {
                store(length, policy.read_byte(byte));
            }
            length += 1;
        }
    }
    /// Borrow a raw byte string and leave the cursor at its native boundary.
    pub fn read_string(&mut self, policy: StringPolicy) -> Result<&'a [u8], Error> {
        if self.encoding != Encoding::Bytes || policy.sanitize {
            return Err(self.error(ErrorKind::Width));
        }
        let start = self.byte_position();
        let length = self.string(policy, |_, _| {})?;
        Ok(&self.data[start..start + length])
    }
    /// Fixed caller scratch is needed only when native decoding changes bytes.
    pub fn read_string_into<'b>(
        &mut self,
        policy: StringPolicy,
        output: &'b mut [u8],
    ) -> Result<&'b [u8], Error> {
        if policy.maximum >= output.len() {
            return Err(self.error(ErrorKind::Capacity));
        }
        let length = self.string(policy, |index, byte| output[index] = byte)?;
        output[length] = 0;
        Ok(&output[..length])
    }
}

pub struct Writer<'a> {
    data: &'a mut [u8],
    encoding: Encoding,
    bit: usize,
}
impl<'a> Writer<'a> {
    /// Continue a native byte message without rewriting its existing prefix.
    pub fn append_bytes(data: &'a mut [u8], written: usize) -> Result<Self, Error> {
        let bit = written
            .checked_mul(8)
            .filter(|_| written <= data.len())
            .ok_or(Error {
                byte: written,
                kind: ErrorKind::Capacity,
            })?;
        Ok(Self {
            data,
            encoding: Encoding::Bytes,
            bit,
        })
    }
    pub fn new(data: &'a mut [u8], encoding: Encoding) -> Self {
        Self {
            data,
            encoding,
            bit: 0,
        }
    }
    pub fn out_of_band(data: &'a mut [u8]) -> Result<Self, Error> {
        let mut writer = Self::new(data, Encoding::Bytes);
        writer.write_bits(u32::MAX, 32)?;
        Ok(writer)
    }
    pub fn bit_position(&self) -> usize {
        self.bit
    }
    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.size()]
    }
    /// Fill an already-written byte prefix after its variable-length body.
    pub fn patch_byte(&mut self, position: usize, value: u8) -> Result<(), Error> {
        if self.encoding != Encoding::Bytes || position >= self.size() {
            return Err(self.error(ErrorKind::Width));
        }
        self.data[position] = value;
        Ok(())
    }
    pub fn size(&self) -> usize {
        if self.encoding == Encoding::Q3 && self.bit != 0 {
            self.bit / 8 + 1
        } else {
            self.bit.div_ceil(8)
        }
    }
    fn error(&self, kind: ErrorKind) -> Error {
        Error {
            byte: self.bit / 8,
            kind,
        }
    }
    fn raw(&mut self, value: u32, width: u8) {
        let start = self.bit / 8;
        let shift = self.bit % 8;
        let mask = ((1u64 << width) - 1) << shift;
        let value = u64::from(value) << shift;
        for i in 0..(shift + usize::from(width)).div_ceil(8) {
            let byte_mask = (mask >> (i * 8)) as u8;
            self.data[start + i] =
                (self.data[start + i] & !byte_mask) | ((value >> (i * 8)) as u8 & byte_mask);
        }
        self.bit += usize::from(width);
    }
    pub fn write_bits(&mut self, value: u32, width: u8) -> Result<(), Error> {
        if !valid_width(self.encoding, width) {
            return Err(self.error(ErrorKind::Width));
        }
        let low = width % 8;
        let bits = if self.encoding == Encoding::Q3 {
            q3_bits(value, width)
        } else {
            usize::from(width)
        };
        let end = self
            .bit
            .checked_add(bits)
            .ok_or_else(|| self.error(ErrorKind::Capacity))?;
        let bytes = if self.encoding == Encoding::Q3 {
            end / 8 + 1
        } else {
            end.div_ceil(8)
        };
        if bytes > self.data.len() {
            return Err(self.error(ErrorKind::Capacity));
        }
        // Initialize newly exposed bytes, including Q3's aligned padding byte.
        // Previously written partial bytes retain their bits and zero padding.
        if self.encoding != Encoding::Bytes {
            let initialized = self.size();
            self.data[initialized..bytes].fill(0);
        }
        if self.encoding != Encoding::Q3 {
            self.raw(value, width);
        } else {
            if low != 0 {
                self.raw(value, low);
            }
            let mut shift = low;
            while shift < width {
                let (code, length) = CODES[((value >> shift) & 255) as usize];
                self.raw(code, length);
                shift += 8;
            }
        }
        Ok(())
    }
    pub fn write_float(&mut self, value: f32) -> Result<(), Error> {
        self.write_bits(value.to_bits(), 32)
    }
    pub fn write_data(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.write_data_iter(bytes.iter().copied())
    }
    pub fn write_data_iter(&mut self, bytes: impl IntoIterator<Item = u8>) -> Result<(), Error> {
        for byte in bytes {
            self.write_bits(u32::from(byte), 8)?;
        }
        Ok(())
    }
    /// Append a checked standalone Q3 MSG body at this stream's current bit
    /// position. The same raw bit writer preserves its Huffman codes and native
    /// string bytes rather than decoding and encoding the body a second time.
    pub fn append_encoded_q3(&mut self, bytes: &[u8], bits: usize) -> Result<(), Error> {
        if self.encoding != Encoding::Q3 {
            return Err(self.error(ErrorKind::Width));
        }
        if bits > bytes.len().saturating_mul(8) {
            return Err(self.error(ErrorKind::Truncated));
        }
        let end = self
            .bit
            .checked_add(bits)
            .ok_or_else(|| self.error(ErrorKind::Capacity))?;
        // Q3 MSG cursize includes a trailing byte even at an exact bit boundary.
        if end / 8 + 1 > self.data.len() {
            return Err(self.error(ErrorKind::Capacity));
        }
        let mut reader = Reader::new(bytes, Encoding::Bits);
        let mut remaining = bits;
        while remaining != 0 {
            let width = remaining.min(32) as u8;
            self.raw(reader.raw(width)?, width);
            remaining -= usize::from(width);
        }
        Ok(())
    }
    /// Write a generated raw string at its destination's native boundary.
    pub fn write_string(&mut self, policy: StringPolicy, text: &[u8]) -> Result<(), Error> {
        if self.encoding != Encoding::Bytes && !(self.encoding == Encoding::Q3 && policy.sanitize) {
            return Err(self.error(ErrorKind::Width));
        }
        self.write_data_iter(policy.write_bytes(text))
    }
}

pub(crate) fn sign_extend(word: u32, bits: u8) -> i32 {
    ((word << (32 - bits)) as i32) >> (32 - bits)
}
