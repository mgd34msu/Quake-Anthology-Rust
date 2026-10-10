//! Shared borrowed native message streams. No packet owns a growable buffer.
//! Q3 msg.c writes low remainder bits, then fixed Huffman codes for whole bytes.
mod codes;
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

fn valid_width(encoding: Encoding, width: u8) -> bool {
    match encoding {
        Encoding::Bytes => matches!(width, 8 | 16 | 32),
        Encoding::Bits | Encoding::Q3 => (1..=32).contains(&width),
    }
}

/// Offset errors drop one message at its boundary; the reader never grows data.
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
    fn error(&self, kind: ErrorKind) -> Error {
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
        let shift = 32 - width;
        Ok(((value << shift) as i32) >> shift)
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
}

pub struct Writer<'a> {
    data: &'a mut [u8],
    encoding: Encoding,
    bit: usize,
}
impl<'a> Writer<'a> {
    pub fn new(data: &'a mut [u8], encoding: Encoding) -> Self {
        // Byte streams overwrite every exposed byte; bit streams retain zero padding.
        if encoding != Encoding::Bytes {
            data.fill(0);
        }
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
            let mut bits = usize::from(low);
            let mut shift = low;
            while shift < width {
                bits += usize::from(CODES[((value >> shift) & 255) as usize].1);
                shift += 8;
            }
            bits
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
        for &byte in bytes {
            self.write_bits(u32::from(byte), 8)?;
        }
        Ok(())
    }
}
