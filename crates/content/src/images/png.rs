//! PNG decoder and encoder ported from `src/formats/images/png.ts` and
//! `src/formats/images/png-encoder.ts` (GPL-2.0-or-later). Keeps palette
//! and gamma data alongside decoded RGBA pixels. DEFLATE is decoded by
//! hand (the donor uses `node:zlib`); the encoder emits stored blocks.

use crate::images::{fail, pixel_count, ContentError};
use qa_core::binary::{BinaryError, BinaryWriter};

/// Decoded PNG image: RGBA pixels plus PNG metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct PngImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes (`width * height * 4`).
    pub pixels: Vec<u8>,
    /// IHDR bit depth.
    pub bit_depth: u8,
    /// IHDR color type.
    pub color_type: u8,
    /// gAMA value, when present.
    pub gamma: Option<f64>,
    /// sRGB rendering intent, when present.
    pub srgb_intent: Option<u8>,
    /// Palette data for indexed images.
    pub indexed: Option<PngIndexed>,
}

/// Palette data for indexed (`colorType === 3`) images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngIndexed {
    /// Per-pixel palette indices, row-major.
    pub indices: Vec<u8>,
    /// Raw PLTE bytes (triplets).
    pub palette: Vec<u8>,
    /// Per-entry alpha (tRNS prefix, 255-filled).
    pub alpha: Vec<u8>,
}

const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (a, b, c) = (i32::from(a), i32::from(b), i32::from(c));
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            if crc & 1 == 0 {
                crc >>= 1;
            } else {
                crc = (crc >> 1) ^ 0xedb8_8320;
            }
        }
    }
    crc ^ 0xffff_ffff
}

fn adler32(bytes: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in bytes {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn read_u32_be(bytes: &[u8], offset: usize) -> u32 {
    (u32::from(bytes[offset]) << 24)
        | (u32::from(bytes[offset + 1]) << 16)
        | (u32::from(bytes[offset + 2]) << 8)
        | u32::from(bytes[offset + 3])
}

fn read_u16_be(bytes: &[u8], offset: usize) -> u16 {
    (u16::from(bytes[offset]) << 8) | u16::from(bytes[offset + 1])
}

fn chunk_name(bytes: &[u8], offset: usize) -> String {
    bytes[offset..offset + 4].iter().map(|&b| b as char).collect()
}

fn append_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) -> Result<(), ContentError> {
    let mut writer = BinaryWriter::new(data.len().saturating_add(12));
    writer.bytes(&(data.len() as u32).to_be_bytes())?;
    writer.bytes(chunk_type)?;
    writer.bytes(data)?;
    let mut crc_input = Vec::with_capacity(data.len().saturating_add(4));
    crc_input.extend_from_slice(chunk_type);
    crc_input.extend_from_slice(data);
    writer.bytes(&crc32(&crc_input).to_be_bytes())?;
    out.extend_from_slice(&writer.finish());
    Ok(())
}

struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    fn read_bits(&mut self, count: u8) -> Result<u32, &'static str> {
        let mut value: u32 = 0;
        for i in 0..count {
            let byte_index = self.bit_pos / 8;
            if byte_index >= self.data.len() {
                return Err("truncated input");
            }
            let bit = (self.data[byte_index] >> (self.bit_pos % 8)) & 1;
            value |= u32::from(bit) << i;
            self.bit_pos += 1;
        }
        Ok(value)
    }

    fn align_to_byte(&mut self) {
        self.bit_pos = self.bit_pos.div_ceil(8) * 8;
    }

    fn read_aligned_bytes(&mut self, count: usize) -> Result<&'a [u8], &'static str> {
        self.align_to_byte();
        let start = self.bit_pos / 8;
        let end = start.saturating_add(count);
        if end > self.data.len() {
            return Err("truncated input");
        }
        self.bit_pos = end * 8;
        Ok(&self.data[start..end])
    }
}

struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn build(lengths: &[u8]) -> Result<Self, &'static str> {
        let mut counts = [0u16; 16];
        for &len in lengths {
            if len as usize >= counts.len() {
                return Err("invalid code length");
            }
            counts[len as usize] += 1;
        }
        if counts[0] as usize == lengths.len() {
            return Err("no codes defined");
        }
        let mut left: i32 = 1;
        for count in counts.iter().skip(1) {
            left <<= 1;
            left -= i32::from(*count);
            if left < 0 {
                return Err("over-subscribed Huffman code");
            }
        }
        let mut offsets = [0u16; 16];
        let mut next = 0u16;
        for len in 1..16 {
            offsets[len] = next;
            next += counts[len];
        }
        let mut symbols = vec![0u16; next as usize];
        for (symbol, &len) in lengths.iter().enumerate() {
            if len != 0 {
                let slot = offsets[len as usize] as usize;
                if slot < symbols.len() {
                    symbols[slot] = symbol as u16;
                }
                offsets[len as usize] += 1;
            }
        }
        Ok(Self { counts, symbols })
    }

    fn decode(&self, reader: &mut BitReader<'_>) -> Result<u16, &'static str> {
        let mut code: u32 = 0;
        let mut first: u32 = 0;
        let mut index: usize = 0;
        for len in 1..16 {
            code |= reader.read_bits(1)?;
            let count = u32::from(self.counts[len]);
            if code.wrapping_sub(first) < count {
                return self
                    .symbols
                    .get(index + (code - first) as usize)
                    .copied()
                    .ok_or("invalid Huffman code");
            }
            index += count as usize;
            first = first.saturating_add(count) << 1;
            code <<= 1;
        }
        Err("invalid Huffman code")
    }
}

fn fixed_literal_huffman() -> Result<Huffman, &'static str> {
    let mut lengths = [8u8; 288];
    for (i, slot) in lengths.iter_mut().enumerate() {
        *slot = if i <= 143 {
            8
        } else if i <= 255 {
            9
        } else if i <= 279 {
            7
        } else {
            8
        };
    }
    Huffman::build(&lengths)
}

fn fixed_distance_huffman() -> Result<Huffman, &'static str> {
    Huffman::build(&[5u8; 32])
}

const CODE_LENGTH_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

fn dynamic_huffmans(reader: &mut BitReader<'_>) -> Result<(Huffman, Huffman), &'static str> {
    let hlit = reader.read_bits(5)? as usize + 257;
    let hdist = reader.read_bits(5)? as usize + 1;
    let hclen = reader.read_bits(4)? as usize + 4;
    let mut code_lengths = [0u8; 19];
    for i in 0..hclen {
        code_lengths[CODE_LENGTH_ORDER[i]] = reader.read_bits(3)? as u8;
    }
    let code_huffman = Huffman::build(&code_lengths)?;
    let mut lengths = Vec::with_capacity(hlit + hdist);
    while lengths.len() < hlit + hdist {
        let symbol = code_huffman.decode(reader)?;
        match symbol {
            0..=15 => lengths.push(symbol as u8),
            16 => {
                if lengths.is_empty() {
                    return Err("invalid repeat code");
                }
                let prev = lengths[lengths.len() - 1];
                let repeat = reader.read_bits(2)? as usize + 3;
                lengths.resize(lengths.len() + repeat, prev);
            }
            17 => {
                let repeat = reader.read_bits(3)? as usize + 3;
                lengths.resize(lengths.len() + repeat, 0);
            }
            18 => {
                let repeat = reader.read_bits(7)? as usize + 11;
                lengths.resize(lengths.len() + repeat, 0);
            }
            _ => return Err("invalid code-length symbol"),
        }
    }
    lengths.truncate(hlit + hdist);
    if lengths[256] == 0 {
        return Err("missing end-of-block code");
    }
    let literal = Huffman::build(&lengths[..hlit])?;
    let distance = Huffman::build(&lengths[hlit..])?;
    Ok((literal, distance))
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
];

fn decode_huffman_block(
    reader: &mut BitReader<'_>,
    literal: &Huffman,
    distance: &Huffman,
    out: &mut Vec<u8>,
    expected: u64,
) -> Result<(), &'static str> {
    loop {
        if out.len() as u64 > expected {
            return Ok(());
        }
        let symbol = literal.decode(reader)?;
        match symbol {
            0..=255 => {
                if out.len() < 1 << 31 {
                    out.push(symbol as u8);
                } else {
                    return Err("output too large");
                }
            }
            256 => return Ok(()),
            257..=285 => {
                let index = (symbol - 257) as usize;
                let length = u32::from(LENGTH_BASE[index]) + reader.read_bits(LENGTH_EXTRA[index])?;
                let dist_symbol = distance.decode(reader)?;
                if dist_symbol > 29 {
                    return Err("invalid distance code");
                }
                let dist =
                    u32::from(DIST_BASE[dist_symbol as usize]) + reader.read_bits(DIST_EXTRA[dist_symbol as usize])?;
                if dist == 0 || dist as usize > out.len() {
                    return Err("invalid match distance");
                }
                for _ in 0..length {
                    if out.len() as u64 > expected {
                        return Ok(());
                    }
                    let byte = out[out.len() - dist as usize];
                    if out.len() < 1 << 31 {
                        out.push(byte);
                    } else {
                        return Err("output too large");
                    }
                }
            }
            _ => return Err("invalid length code"),
        }
    }
}

fn inflate_zlib(data: &[u8], expected: u64) -> Result<Vec<u8>, &'static str> {
    if data.len() < 6 {
        return Err("truncated input");
    }
    let cmf = data[0];
    let flg = data[1];
    if cmf & 0x0f != 8 {
        return Err("unknown compression method");
    }
    if (u16::from(cmf) * 256 + u16::from(flg)) % 31 != 0 {
        return Err("bad header check");
    }
    if flg & 0x20 != 0 {
        return Err("preset dictionary not supported");
    }
    let mut reader = BitReader::new(&data[2..]);
    let mut out: Vec<u8> = Vec::new();
    loop {
        if out.len() as u64 > expected {
            return Ok(out);
        }
        let is_final = reader.read_bits(1)? != 0;
        let block_type = reader.read_bits(2)?;
        match block_type {
            0 => {
                let header = reader.read_aligned_bytes(4)?;
                let len = u16::from_le_bytes([header[0], header[1]]);
                let nlen = u16::from_le_bytes([header[2], header[3]]);
                if len != !nlen {
                    return Err("bad stored block lengths");
                }
                let bytes = reader.read_aligned_bytes(usize::from(len))?;
                out.extend_from_slice(bytes);
            }
            1 => {
                let literal = fixed_literal_huffman()?;
                let distance = fixed_distance_huffman()?;
                decode_huffman_block(&mut reader, &literal, &distance, &mut out, expected)?;
            }
            2 => {
                let (literal, distance) = dynamic_huffmans(&mut reader)?;
                decode_huffman_block(&mut reader, &literal, &distance, &mut out, expected)?;
            }
            _ => return Err("invalid block type"),
        }
        if is_final {
            break;
        }
    }
    if out.len() as u64 > expected {
        return Ok(out);
    }
    let trailer = reader.read_aligned_bytes(4)?;
    let expected_adler = (u32::from(trailer[0]) << 24)
        | (u32::from(trailer[1]) << 16)
        | (u32::from(trailer[2]) << 8)
        | u32::from(trailer[3]);
    if adler32(&out) != expected_adler {
        return Err("bad adler32");
    }
    Ok(out)
}

fn pass_dim(len: u32, start: u32, step: u32) -> u32 {
    if len <= start {
        0
    } else {
        (len - start).div_ceil(step)
    }
}

fn sample_value(rows: &[u8], row: usize, bit: u64, bit_depth: u8) -> u16 {
    let offset = row + (bit / 8) as usize;
    if bit_depth == 16 {
        read_u16_be(rows, offset)
    } else if bit_depth == 8 {
        u16::from(rows[offset])
    } else {
        let shift = 8u32 - u32::from(bit_depth) - (bit % 8) as u32;
        u16::from((rows[offset] >> shift) & ((1 << bit_depth) - 1))
    }
}

fn to_byte(value: u16, bit_depth: u8) -> u8 {
    if bit_depth == 16 {
        (value >> 8) as u8
    } else if bit_depth == 8 {
        value as u8
    } else {
        let max = (1u16 << bit_depth) - 1;
        (f64::from(value) * 255.0 / f64::from(max)).round() as u8
    }
}

/// Decode PNG bytes into RGBA pixels plus palette and gamma metadata.
pub fn decode_png(bytes: &[u8], source: &str) -> Result<PngImage, ContentError> {
    let png_fail = |offset: usize, message: String| -> BinaryError { fail(source, offset, format!("PNG: {message}")) };
    if bytes.len() < 33 || read_u32_be(bytes, 0) != 0x8950_4e47 || read_u32_be(bytes, 4) != 0x0d0a_1a0a {
        return Err(png_fail(0, "bad signature".to_string()));
    }
    let mut width: u32 = 0;
    let mut height: u32 = 0;
    let mut bit_depth: u8 = 0;
    let mut color_type: u8 = 0;
    let mut interlace: u8 = 0;
    let mut gamma: Option<f64> = None;
    let mut srgb_intent: Option<u8> = None;
    let mut palette: Option<Vec<u8>> = None;
    let mut transparency: Option<Vec<u8>> = None;
    let mut ended = false;
    let mut compressed_length: u64 = 0;
    let mut idat_ranges: Vec<(usize, usize)> = Vec::new();
    let mut offset = 8usize;
    while offset + 12 <= bytes.len() {
        let length = u64::from(read_u32_be(bytes, offset));
        let start = offset + 8;
        let end_wide = start as u64 + length;
        if end_wide + 4 > bytes.len() as u64 {
            return Err(png_fail(offset, "truncated chunk".to_string()));
        }
        let end = end_wide as usize;
        let chunk_type = chunk_name(bytes, offset + 4);
        if read_u32_be(bytes, end) != crc32(&bytes[offset + 4..end]) {
            return Err(png_fail(offset, format!("bad {chunk_type} CRC")));
        }
        if chunk_type == "IHDR" {
            if offset != 8 || length != 13 {
                return Err(png_fail(offset, "misplaced or malformed IHDR".to_string()));
            }
            width = read_u32_be(bytes, start);
            height = read_u32_be(bytes, start + 4);
            bit_depth = bytes[start + 8];
            color_type = bytes[start + 9];
            interlace = bytes[start + 12];
            if bytes[start + 10] != 0 || bytes[start + 11] != 0 || interlace > 1 {
                return Err(png_fail(
                    start + 10,
                    "unsupported compression, filter or interlace method".to_string(),
                ));
            }
            let valid = matches!(
                (color_type, bit_depth),
                (0, 1 | 2 | 4 | 8 | 16) | (3, 1 | 2 | 4 | 8) | (2 | 4 | 6, 8 | 16)
            );
            if !valid {
                return Err(png_fail(
                    start + 8,
                    format!("unsupported color type {color_type} at depth {bit_depth}"),
                ));
            }
        } else if chunk_type == "PLTE" {
            if length == 0 || length > 768 || length % 3 != 0 {
                return Err(png_fail(start, "invalid palette".to_string()));
            }
            palette = Some(bytes[start..end].to_vec());
        } else if chunk_type == "tRNS" {
            transparency = Some(bytes[start..end].to_vec());
        } else if chunk_type == "gAMA" {
            if length != 4 || read_u32_be(bytes, start) == 0 {
                return Err(png_fail(start, "invalid gamma".to_string()));
            }
            gamma = Some(f64::from(read_u32_be(bytes, start)) / 100_000.0);
        } else if chunk_type == "sRGB" {
            if length != 1 || bytes[start] > 3 {
                return Err(png_fail(start, "invalid sRGB intent".to_string()));
            }
            srgb_intent = Some(bytes[start]);
        } else if chunk_type == "IDAT" {
            idat_ranges.push((start, end));
            compressed_length += length;
        } else if chunk_type == "IEND" {
            if length != 0 {
                return Err(png_fail(start, "invalid IEND".to_string()));
            }
            ended = true;
            break;
        } else if bytes[offset + 4] & 32 == 0 {
            return Err(png_fail(offset, format!("unsupported critical chunk {chunk_type}")));
        }
        offset = end + 4;
    }
    let frame_bytes = match pixel_count(u64::from(width), u64::from(height)) {
        Some(count) => count.saturating_mul(4),
        None => u64::MAX,
    };
    if !ended || width == 0 || height == 0 || frame_bytes > 0x7fff_ffff || idat_ranges.is_empty() {
        return Err(png_fail(8, "missing chunks or invalid dimensions".to_string()));
    }
    if color_type == 3 && palette.is_none() {
        return Err(png_fail(8, "indexed image has no palette".to_string()));
    }
    let channels: u64 = match color_type {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        _ => 4,
    };
    let bits_per_pixel = channels * u64::from(bit_depth);
    let byte_stride = bits_per_pixel.div_ceil(8).max(1) as usize;
    let passes: &[(u32, u32, u32, u32)] = if interlace == 0 {
        &[(0, 0, 1, 1)]
    } else {
        &[
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ]
    };
    let mut raw_length: u64 = 0;
    for &(x, y, dx, dy) in passes {
        let w = pass_dim(width, x, dx);
        let h = pass_dim(height, y, dy);
        if w != 0 && h != 0 {
            raw_length += ((u64::from(w) * bits_per_pixel).div_ceil(8) + 1) * u64::from(h);
        }
    }
    let mut compressed: Vec<u8> = Vec::new();
    compressed.reserve_exact(compressed_length as usize);
    for &(start, end) in &idat_ranges {
        compressed.extend_from_slice(&bytes[start..end]);
    }
    let raw = match inflate_zlib(&compressed, raw_length) {
        Ok(raw) => raw,
        Err(message) => return Err(fail(source, 8, format!("PNG inflate: {message}"))),
    };
    if raw.len() as u64 != raw_length {
        return Err(png_fail(8, "unexpected decompressed size".to_string()));
    }
    if let Some(transparency) = transparency.as_ref() {
        let palette_entries = match palette.as_ref() {
            Some(entries) => entries.len() / 3,
            None => 0,
        };
        let invalid = (color_type == 0 && transparency.len() != 2)
            || (color_type == 2 && transparency.len() != 6)
            || (color_type == 3 && transparency.len() > palette_entries)
            || color_type == 4
            || color_type == 6;
        if invalid {
            return Err(png_fail(8, "invalid transparency chunk".to_string()));
        }
    }
    let total = frame_bytes as usize;
    let mut pixels = vec![0u8; total];
    let mut indices = if color_type == 3 {
        Some(vec![0u8; total / 4])
    } else {
        None
    };
    let palette_bytes: &[u8] = match palette.as_ref() {
        Some(entries) => entries,
        None => &[],
    };
    let transparency_bytes: &[u8] = match transparency.as_ref() {
        Some(entries) => entries,
        None => &[],
    };
    let has_transparency = transparency.is_some();
    let mut raw_offset = 0usize;
    for &(x, y, dx, dy) in passes {
        let w = pass_dim(width, x, dx);
        let h = pass_dim(height, y, dy);
        if w == 0 || h == 0 {
            continue;
        }
        let row_length = (u64::from(w) * bits_per_pixel).div_ceil(8) as usize;
        let mut rows = vec![0u8; row_length * h as usize];
        for py in 0..h {
            let row = py as usize * row_length;
            let filter = match raw.get(raw_offset) {
                Some(&filter) => filter,
                None => return Err(png_fail(8, "unexpected decompressed size".to_string())),
            };
            raw_offset += 1;
            if filter > 4 {
                return Err(png_fail(8, format!("unknown scanline filter {filter}")));
            }
            for byte_index in 0..row_length {
                let a = if byte_index >= byte_stride {
                    rows[row + byte_index - byte_stride]
                } else {
                    0
                };
                let b = if py > 0 { rows[row + byte_index - row_length] } else { 0 };
                let c = if py > 0 && byte_index >= byte_stride {
                    rows[row + byte_index - row_length - byte_stride]
                } else {
                    0
                };
                let prediction = match filter {
                    1 => a,
                    2 => b,
                    3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                    4 => paeth(a, b, c),
                    _ => 0,
                };
                let value = match raw.get(raw_offset) {
                    Some(&value) => value,
                    None => return Err(png_fail(8, "unexpected decompressed size".to_string())),
                };
                raw_offset += 1;
                rows[row + byte_index] = value.wrapping_add(prediction);
            }
            for px in 0..w {
                let bit = (u64::from(px) * channels) * u64::from(bit_depth);
                let index = ((u64::from(y) + u64::from(py) * u64::from(dy)) * u64::from(width)
                    + u64::from(x)
                    + u64::from(px) * u64::from(dx)) as usize;
                let out = index * 4;
                if color_type == 3 {
                    let value = sample_value(&rows, row, bit, bit_depth);
                    if value as usize * 3 + 3 > palette_bytes.len() {
                        return Err(png_fail(8, "palette index out of range".to_string()));
                    }
                    if let Some(slots) = indices.as_mut() {
                        slots[index] = value as u8;
                    }
                    let entry = value as usize * 3;
                    pixels[out] = palette_bytes[entry];
                    pixels[out + 1] = palette_bytes[entry + 1];
                    pixels[out + 2] = palette_bytes[entry + 2];
                    pixels[out + 3] = if has_transparency && (value as usize) < transparency_bytes.len() {
                        transparency_bytes[value as usize]
                    } else {
                        255
                    };
                } else if color_type == 0 || color_type == 4 {
                    let value = sample_value(&rows, row, bit, bit_depth);
                    let gray = to_byte(value, bit_depth);
                    pixels[out] = gray;
                    pixels[out + 1] = gray;
                    pixels[out + 2] = gray;
                    pixels[out + 3] = if color_type == 4 {
                        to_byte(
                            sample_value(&rows, row, bit + u64::from(bit_depth), bit_depth),
                            bit_depth,
                        )
                    } else if has_transparency && value == read_u16_be(transparency_bytes, 0) {
                        0
                    } else {
                        255
                    };
                } else {
                    let depth = u64::from(bit_depth);
                    let r = sample_value(&rows, row, bit, bit_depth);
                    let g = sample_value(&rows, row, bit + depth, bit_depth);
                    let b = sample_value(&rows, row, bit + depth * 2, bit_depth);
                    pixels[out] = to_byte(r, bit_depth);
                    pixels[out + 1] = to_byte(g, bit_depth);
                    pixels[out + 2] = to_byte(b, bit_depth);
                    pixels[out + 3] = if color_type == 6 {
                        to_byte(sample_value(&rows, row, bit + depth * 3, bit_depth), bit_depth)
                    } else if has_transparency
                        && r == read_u16_be(transparency_bytes, 0)
                        && g == read_u16_be(transparency_bytes, 2)
                        && b == read_u16_be(transparency_bytes, 4)
                    {
                        0
                    } else {
                        255
                    };
                }
            }
        }
    }
    let mut alpha = vec![255u8; palette_bytes.len() / 3];
    if color_type == 3 && has_transparency {
        alpha[..transparency_bytes.len()].copy_from_slice(transparency_bytes);
    }
    let indexed = if color_type == 3 {
        match (indices, palette) {
            (Some(slots), Some(entries)) => Some(PngIndexed {
                indices: slots,
                palette: entries,
                alpha,
            }),
            _ => None,
        }
    } else {
        None
    };
    Ok(PngImage {
        width,
        height,
        pixels,
        bit_depth,
        color_type,
        gamma,
        srgb_intent,
        indexed,
    })
}

fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    let blocks = raw.len().div_ceil(65535).max(1);
    let mut out = Vec::with_capacity(raw.len() + blocks * 5 + 6);
    out.extend_from_slice(&[0x78, 0x01]);
    for (index, block) in raw.chunks(65535).enumerate() {
        out.push(u8::from(index + 1 == blocks));
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

/// Encode RGBA pixels as an 8-bit truecolor PNG.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, ContentError> {
    let valid = (1..=16384).contains(&width)
        && (1..=16384).contains(&height)
        && rgba.len() as u64 == u64::from(width) * u64::from(height) * 4;
    if !valid {
        return Err(fail(
            "<png output>",
            0,
            "PNG dimensions do not match the RGBA framebuffer".to_string(),
        ));
    }
    let stride = width as usize * 4;
    let mut scanlines = vec![0u8; height as usize * (stride + 1)];
    for y in 0..height as usize {
        let target = y * (stride + 1) + 1;
        scanlines[target..target + stride].copy_from_slice(&rgba[y * stride..(y + 1) * stride]);
    }
    let idat = zlib_stored(&scanlines);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    let mut out = Vec::new();
    out.extend_from_slice(&SIGNATURE);
    append_chunk(&mut out, b"IHDR", &ihdr)?;
    append_chunk(&mut out, b"IDAT", &idat)?;
    append_chunk(&mut out, b"IEND", &[])?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_png(
        width: u32,
        height: u32,
        bit_depth: u8,
        color_type: u8,
        interlace: u8,
        before_idat: &[(&[u8; 4], &[u8])],
        raw: &[u8],
    ) -> Vec<u8> {
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&width.to_be_bytes());
        ihdr[4..8].copy_from_slice(&height.to_be_bytes());
        ihdr[8] = bit_depth;
        ihdr[9] = color_type;
        ihdr[12] = interlace;
        let mut out = Vec::new();
        out.extend_from_slice(&SIGNATURE);
        append_chunk(&mut out, b"IHDR", &ihdr).unwrap();
        for (name, data) in before_idat {
            append_chunk(&mut out, name, data).unwrap();
        }
        append_chunk(&mut out, b"IDAT", &zlib_stored(raw)).unwrap();
        append_chunk(&mut out, b"IEND", &[]).unwrap();
        out
    }

    fn decode(bytes: &[u8]) -> Result<PngImage, BinaryError> {
        decode_png(bytes, "<test>")
    }

    fn expect_error(bytes: &[u8], message: &str) -> BinaryError {
        let err = decode(bytes).unwrap_err();
        assert_eq!(err.input, "<test>");
        assert_eq!(err.message, message);
        err
    }

    #[test]
    fn round_trip_1x1() {
        let rgba = [10u8, 20, 30, 40];
        let encoded = encode_png(1, 1, &rgba).unwrap();
        let image = decode(&encoded).unwrap();
        assert_eq!(image.width, 1);
        assert_eq!(image.height, 1);
        assert_eq!(image.pixels, rgba);
        assert_eq!(image.bit_depth, 8);
        assert_eq!(image.color_type, 6);
        assert_eq!(image.indexed, None);
    }

    #[test]
    fn round_trip_3x2() {
        let rgba: Vec<u8> = (0..24u8).collect();
        let encoded = encode_png(3, 2, &rgba).unwrap();
        let image = decode(&encoded).unwrap();
        assert_eq!((image.width, image.height), (3, 2));
        assert_eq!(image.pixels, rgba);
        assert_eq!(image.bit_depth, 8);
        assert_eq!(image.color_type, 6);
    }

    #[test]
    fn round_trip_300x1() {
        let rgba: Vec<u8> = (0..300u32).flat_map(|i| [i as u8, 255 - i as u8, 7, 255]).collect();
        let encoded = encode_png(300, 1, &rgba).unwrap();
        let image = decode(&encoded).unwrap();
        assert_eq!((image.width, image.height), (300, 1));
        assert_eq!(image.pixels, rgba);
    }

    #[test]
    fn round_trip_multi_block() {
        let rgba: Vec<u8> = (0..300u32 * 55)
            .flat_map(|i| [(i % 251) as u8, (i % 251) as u8, (i % 251) as u8, 255])
            .collect();
        // 55 * (300 * 4 + 1) = 66055 bytes of scanlines, past one 65535-byte stored block.
        let encoded = encode_png(300, 55, &rgba).unwrap();
        let image = decode(&encoded).unwrap();
        assert_eq!((image.width, image.height), (300, 55));
        assert_eq!(image.pixels, rgba);
        assert_eq!(image.bit_depth, 8);
        assert_eq!(image.color_type, 6);
    }

    #[test]
    fn one_bit_gray() {
        let bytes = build_png(8, 1, 1, 0, 0, &[], &[0, 0b1011_0001]);
        let image = decode(&bytes).unwrap();
        let gray = [255u8, 0, 255, 255, 0, 0, 0, 255];
        let expected: Vec<u8> = gray.iter().flat_map(|&g| [g, g, g, 255]).collect();
        assert_eq!(image.pixels, expected);
        assert_eq!(image.bit_depth, 1);
    }

    #[test]
    fn four_bit_gray() {
        let bytes = build_png(4, 1, 4, 0, 0, &[], &[0, 0x1f, 0xa5]);
        let image = decode(&bytes).unwrap();
        let expected: Vec<u8> = [17u8, 255, 170, 85].iter().flat_map(|&g| [g, g, g, 255]).collect();
        assert_eq!(image.pixels, expected);
    }

    #[test]
    fn indexed_with_trns_alpha_fill() {
        let palette = [255u8, 0, 0, 0, 0, 255];
        let bytes = build_png(2, 1, 8, 3, 0, &[(b"PLTE", &palette), (b"tRNS", &[128u8])], &[0, 0, 1]);
        let image = decode(&bytes).unwrap();
        assert_eq!(image.pixels, vec![255, 0, 0, 128, 0, 0, 255, 255]);
        let indexed = image.indexed.unwrap();
        assert_eq!(indexed.indices, vec![0, 1]);
        assert_eq!(indexed.palette, palette);
        assert_eq!(indexed.alpha, vec![128, 255]);
    }

    #[test]
    fn sixteen_bit_gray_takes_high_byte() {
        let bytes = build_png(1, 1, 16, 0, 0, &[], &[0, 0x12, 0x34]);
        let image = decode(&bytes).unwrap();
        assert_eq!(image.pixels, vec![0x12, 0x12, 0x12, 255]);
    }

    #[test]
    fn sixteen_bit_rgb_takes_high_bytes() {
        let bytes = build_png(1, 1, 16, 2, 0, &[], &[0, 0xab, 0xcd, 0x12, 0x34, 0x00, 0xff]);
        let image = decode(&bytes).unwrap();
        assert_eq!(image.pixels, vec![0xab, 0x12, 0x00, 255]);
    }

    #[test]
    fn adam7_4x4() {
        let mut raw: Vec<u8> = Vec::new();
        let mut push = |values: &[u8]| {
            raw.push(0);
            for &v in values {
                raw.extend_from_slice(&[v, 0, 0, 255]);
            }
        };
        push(&[0]);
        push(&[2]);
        push(&[8, 10]);
        push(&[1, 3]);
        push(&[9, 11]);
        push(&[4, 5, 6, 7]);
        push(&[12, 13, 14, 15]);
        let bytes = build_png(4, 4, 8, 6, 1, &[], &raw);
        let image = decode(&bytes).unwrap();
        let expected: Vec<u8> = (0..16u8).flat_map(|v| [v, 0, 0, 255]).collect();
        assert_eq!(image.pixels, expected);
    }

    #[test]
    fn gamma_and_srgb() {
        let bytes = build_png(
            1,
            1,
            8,
            6,
            0,
            &[(b"gAMA", &[0, 0, 0xb1, 0x8f]), (b"sRGB", &[1])],
            &[0, 9, 9, 9, 255],
        );
        let image = decode(&bytes).unwrap();
        assert!((image.gamma.unwrap() - 0.45455).abs() < 1e-12);
        assert_eq!(image.srgb_intent, Some(1));
    }

    #[test]
    fn ancillary_chunks_are_skipped() {
        let bytes = build_png(1, 1, 8, 6, 0, &[(b"zTXt", b"Comment!")], &[0, 1, 2, 3, 4]);
        let image = decode(&bytes).unwrap();
        assert_eq!(image.pixels, vec![1, 2, 3, 4]);
    }

    #[test]
    fn unknown_critical_chunk_is_rejected() {
        let bytes = build_png(1, 1, 8, 6, 0, &[(b"ZZZZ", b"")], &[0, 1, 2, 3, 4]);
        expect_error(&bytes, "PNG: unsupported critical chunk ZZZZ");
    }

    #[test]
    fn bad_signature_is_rejected() {
        expect_error(b"nope", "PNG: bad signature");
    }

    #[test]
    fn bad_crc_is_rejected() {
        let mut bytes = build_png(1, 1, 8, 6, 0, &[], &[0, 1, 2, 3, 4]);
        let tail = bytes.len() - 1;
        bytes[tail] ^= 0xff;
        let err = expect_error(&bytes, "PNG: bad IEND CRC");
        assert_eq!(err.offset, bytes.len() - 12);
    }

    #[test]
    fn truncated_chunk_is_rejected() {
        let bytes = build_png(1, 1, 8, 6, 0, &[], &[0, 1, 2, 3, 4]);
        let cut = bytes[..bytes.len() - 17].to_vec();
        expect_error(&cut, "PNG: truncated chunk");
    }

    #[test]
    fn unknown_filter_is_rejected() {
        let bytes = build_png(1, 1, 8, 6, 0, &[], &[5, 1, 2, 3, 4]);
        expect_error(&bytes, "PNG: unknown scanline filter 5");
    }

    #[test]
    fn palette_index_out_of_range_is_rejected() {
        let bytes = build_png(1, 1, 8, 3, 0, &[(b"PLTE", &[1u8, 2, 3])], &[0, 1]);
        expect_error(&bytes, "PNG: palette index out of range");
    }

    #[test]
    fn invalid_trns_is_rejected() {
        let bytes = build_png(1, 1, 8, 6, 0, &[(b"tRNS", &[0u8])], &[0, 1, 2, 3, 4]);
        expect_error(&bytes, "PNG: invalid transparency chunk");
        let long = build_png(1, 1, 8, 3, 0, &[(b"PLTE", &[1u8, 2, 3]), (b"tRNS", &[1u8, 2])], &[0, 0]);
        expect_error(&long, "PNG: invalid transparency chunk");
    }

    #[test]
    fn gray_trns_marks_transparent_shade() {
        let bytes = build_png(2, 1, 8, 0, 0, &[(b"tRNS", &[0u8, 7])], &[0, 7, 8]);
        let image = decode(&bytes).unwrap();
        assert_eq!(image.pixels, vec![7, 7, 7, 0, 8, 8, 8, 255]);
    }

    #[test]
    fn inflate_failure_reports_png_inflate() {
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&1u32.to_be_bytes());
        ihdr[4..8].copy_from_slice(&1u32.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = 6;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SIGNATURE);
        append_chunk(&mut bytes, b"IHDR", &ihdr).unwrap();
        append_chunk(&mut bytes, b"IDAT", &[0x78, 0x01, 0x07, 0x00, 0x00, 0x00]).unwrap();
        append_chunk(&mut bytes, b"IEND", &[]).unwrap();
        let err = decode(&bytes).unwrap_err();
        assert_eq!(err.offset, 8);
        assert_eq!(err.message, "PNG inflate: invalid block type");
    }

    #[test]
    fn fixed_huffman_with_lz77_match() {
        struct Packer {
            bytes: Vec<u8>,
            acc: u32,
            nbits: u8,
        }
        impl Packer {
            fn push(&mut self, bits: u32, count: u8, msb_first: bool) {
                for i in 0..count {
                    let shift = if msb_first { count - 1 - i } else { i };
                    let bit = (bits >> shift) & 1;
                    self.acc |= bit << self.nbits;
                    self.nbits += 1;
                    if self.nbits == 8 {
                        self.bytes.push(self.acc as u8);
                        self.acc = 0;
                        self.nbits = 0;
                    }
                }
            }
            fn finish(mut self) -> Vec<u8> {
                if self.nbits > 0 {
                    self.bytes.push(self.acc as u8);
                }
                self.bytes
            }
        }
        let raw = [0u8, 7, 7, 7, 7, 0, 7, 7, 7, 7];
        let mut packer = Packer {
            bytes: vec![0x78, 0x01],
            acc: 0,
            nbits: 0,
        };
        packer.push(1, 1, false);
        packer.push(1, 2, false);
        packer.push(0x30, 8, true);
        packer.push(0x37, 8, true);
        packer.push(1, 7, true);
        packer.push(0, 5, true);
        packer.push(0x30, 8, true);
        packer.push(2, 7, true);
        packer.push(4, 5, true);
        packer.push(0, 1, false);
        packer.push(0, 7, true);
        let mut stream = packer.finish();
        stream.extend_from_slice(&adler32(&raw).to_be_bytes());
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&1u32.to_be_bytes());
        ihdr[4..8].copy_from_slice(&2u32.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = 6;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SIGNATURE);
        append_chunk(&mut bytes, b"IHDR", &ihdr).unwrap();
        append_chunk(&mut bytes, b"IDAT", &stream).unwrap();
        append_chunk(&mut bytes, b"IEND", &[]).unwrap();
        let image = decode(&bytes).unwrap();
        assert_eq!(image.pixels, vec![7, 7, 7, 7, 7, 7, 7, 7]);
    }

    #[test]
    fn encoder_rejects_bad_dimensions() {
        for (width, height, rgba) in [
            (0, 1, Vec::new()),
            (1, 0, Vec::new()),
            (16385, 1, Vec::new()),
            (1, 16385, Vec::new()),
            (1, 1, vec![1, 2, 3]),
            (2, 2, vec![0u8; 15]),
        ] {
            let err = encode_png(width, height, &rgba).unwrap_err();
            assert_eq!(
                err.to_string(),
                "<png output>:0: PNG dimensions do not match the RGBA framebuffer"
            );
        }
    }
}
