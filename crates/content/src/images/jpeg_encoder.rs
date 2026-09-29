//! Quake III SaveJPG JPEG compressor (owned pixels).
//!
//! Donor: `src/formats/images/jpeg-encoder.ts`, a TypeScript translation of
//! the configured IJG compressor behind SaveJPG (`jcparam.c`, `jccolor.c`,
//! `jcsample.c`, `jccoefct.c`, `jcdctmgr.c`, `jfdctflt.c`, `jchuff.c`,
//! `jcmarker.c`; copyright (C) 1991-1995 Thomas G. Lane).

use std::f32::consts::FRAC_1_SQRT_2;

use super::jpeg::JpegImage;
use super::{fail, ContentError};

/// Scanline order of the source pixels (`rowOrder`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowOrder {
    /// First pixel row is the top row.
    TopDown,
    /// First pixel row is the bottom row.
    BottomUp,
}

/// Output ownership (`JpegEncodingProfile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegEncodingProfile {
    /// Diagnostic calls own a growing output (`encode_jpeg`).
    DiagnosticOwned,
    /// Source calls borrow the fixed SaveJPG destination (`encode_jpeg_to`).
    SourceDestination {
        /// Scanline order of the source pixels.
        row_order: RowOrder,
    },
}

/// Zigzag index to natural coefficient order (`naturalOrder`).
const NATURAL_ORDER: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21,
    28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54,
    47, 55, 62, 63,
];

/// Base luminance quantizers in zigzag order (`luminanceQuantizers`).
const LUMINANCE_QUANTIZERS: [u8; 64] = [
    16, 11, 12, 14, 12, 10, 16, 14, 13, 14, 18, 17, 16, 19, 24, 40, 26, 24, 22, 22, 24, 49, 35, 37, 29, 40, 58, 51, 61,
    60, 57, 51, 56, 55, 64, 72, 92, 78, 64, 68, 87, 69, 55, 56, 80, 109, 81, 87, 95, 98, 103, 104, 103, 62, 77, 113,
    121, 112, 100, 120, 92, 101, 103, 99,
];

/// Base chrominance quantizers in zigzag order (`chrominanceQuantizers`).
const CHROMINANCE_QUANTIZERS: [u8; 64] = [
    17, 18, 18, 24, 21, 24, 47, 26, 26, 47, 99, 66, 56, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99,
];

/// Code counts per length for a Huffman table (`HuffmanTable.counts`).
struct HuffmanTable {
    counts: [u8; 16],
    values: &'static [u8],
}

const DC_LUMINANCE: HuffmanTable = HuffmanTable {
    counts: [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0],
    values: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
};
const DC_CHROMINANCE: HuffmanTable = HuffmanTable {
    counts: [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0],
    values: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
};
const AC_LUMINANCE: HuffmanTable = HuffmanTable {
    counts: [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 125],
    values: &[
        0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71,
        0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0, 0x24, 0x33, 0x62, 0x72,
        0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x34, 0x35, 0x36, 0x37,
        0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
        0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83,
        0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3,
        0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
        0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
        0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
    ],
};
const AC_CHROMINANCE: HuffmanTable = HuffmanTable {
    counts: [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 119],
    values: &[
        0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13, 0x22,
        0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0, 0x15, 0x62, 0x72, 0xd1,
        0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x35, 0x36,
        0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58,
        0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a,
        0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a,
        0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba,
        0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
        0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa,
    ],
};

/// Bounds-checked read with the donor message (`at`).
fn at<T: Copy>(values: &[T], index: usize) -> T {
    match values.get(index) {
        Some(&value) => value,
        None => panic!("JPEG encoder index {index}"),
    }
}

/// Canonical Huffman codes by symbol: (code value, code size) (`deriveCodes`).
type HuffmanCodes = [(u32, u8); 256];

fn derive_codes(table: &HuffmanTable) -> HuffmanCodes {
    let mut result = [(0u32, 0u8); 256];
    let mut code: u32 = 0;
    let mut index = 0;
    for (length, &count) in table.counts.iter().enumerate() {
        for _ in 0..count {
            result[at(table.values, index) as usize] = (code, (length + 1) as u8);
            code += 1;
            index += 1;
        }
        code *= 2;
    }
    result
}

/// DC and AC codes for one component kind (`luminanceCodes`).
struct CodeSet {
    dc: HuffmanCodes,
    ac: HuffmanCodes,
}

/// Backing store for the entropy output (`Output`).
enum OutputBuffer<'a> {
    Owned(Vec<u8>),
    Borrowed(&'a mut [u8]),
}

/// Bit writer with `0xFF` stuffing over a growing or fixed buffer (`Output`).
struct Output<'a> {
    bytes: OutputBuffer<'a>,
    length: usize,
    pending: u32,
    pending_bits: u32,
}

impl<'a> Output<'a> {
    fn owned() -> Output<'a> {
        Output {
            bytes: OutputBuffer::Owned(vec![0; 4096]),
            length: 0,
            pending: 0,
            pending_bits: 0,
        }
    }

    fn borrowed(destination: &'a mut [u8]) -> Output<'a> {
        Output {
            bytes: OutputBuffer::Borrowed(destination),
            length: 0,
            pending: 0,
            pending_bits: 0,
        }
    }

    fn capacity(&self) -> usize {
        match &self.bytes {
            OutputBuffer::Owned(bytes) => bytes.len(),
            OutputBuffer::Borrowed(bytes) => bytes.len(),
        }
    }

    fn byte(&mut self, value: u8) -> Result<(), ContentError> {
        if self.length == self.capacity() {
            match &mut self.bytes {
                OutputBuffer::Owned(bytes) => bytes.resize(bytes.len() * 2, 0),
                OutputBuffer::Borrowed(_) => {
                    return Err(fail(
                        "SaveJPG",
                        self.length,
                        "SaveJPG destination capacity exceeded; source buffer exhaustion is unsupported".to_string(),
                    ));
                }
            }
        }
        match &mut self.bytes {
            OutputBuffer::Owned(bytes) => bytes[self.length] = value,
            OutputBuffer::Borrowed(bytes) => bytes[self.length] = value,
        }
        self.length += 1;
        Ok(())
    }

    fn marker(&mut self, marker: u8, payload: &[u8]) -> Result<(), ContentError> {
        self.byte(255)?;
        self.byte(marker)?;
        self.byte(((payload.len() + 2) >> 8) as u8)?;
        self.byte(((payload.len() + 2) & 255) as u8)?;
        for &value in payload {
            self.byte(value)?;
        }
        Ok(())
    }

    fn bits(&mut self, value: i32, size: u32) -> Result<(), ContentError> {
        self.pending = self.pending * (1u32 << size) + ((value as u32) & ((1u32 << size) - 1));
        self.pending_bits += size;
        while self.pending_bits >= 8 {
            self.pending_bits -= 8;
            let byte = ((self.pending >> self.pending_bits) & 255) as u8;
            self.byte(byte)?;
            if byte == 255 {
                self.byte(0)?;
            }
        }
        self.pending &= (1u32 << self.pending_bits) - 1;
        Ok(())
    }

    fn symbol(&mut self, table: &HuffmanCodes, symbol: usize) -> Result<(), ContentError> {
        let (value, size) = at(&table[..], symbol);
        if size == 0 {
            panic!("JPEG encoder Huffman symbol {symbol}");
        }
        self.bits(value as i32, u32::from(size))
    }

    fn finish(&mut self) -> Result<(), ContentError> {
        // jchuff.c flush_bits emits seven ones, then discards the remaining bits.
        self.bits(127, 7)?;
        self.byte(255)?;
        self.byte(217)?;
        Ok(())
    }
}

/// Quality-scaled quantizers (`quantizers`).
fn quantizers(base: &[u8; 64], quality: u8) -> [u8; 64] {
    let bounded = u32::from(quality).clamp(1, 100);
    let scale = if bounded < 50 {
        5000 / bounded
    } else {
        200 - bounded * 2
    };
    let mut result = [0u8; 64];
    for (index, &value) in base.iter().enumerate() {
        result[index] = ((u32::from(value) * scale + 50) / 100).clamp(1, 255) as u8;
    }
    result
}

// jcdctmgr.c start_pass evaluates the reciprocal in double, then stores float.
fn divisors(table: &[u8; 64]) -> [f32; 64] {
    const SCALES: [f64; 8] = [
        1.0,
        1.387039845,
        1.306562965,
        1.175875602,
        1.0,
        0.785694958,
        0.541196100,
        0.275899379,
    ];
    let mut result = [0f32; 64];
    for (zigzag, &natural) in NATURAL_ORDER.iter().enumerate() {
        result[natural] = (1.0
            / (f64::from(at(&table[..], zigzag)) * at(&SCALES[..], natural / 8) * at(&SCALES[..], natural % 8) * 8.0))
            as f32;
    }
    result
}

// jfdctflt.c, with binary32 expressions and storage, without contraction.
#[allow(clippy::too_many_lines)]
fn forward_pass(data: &mut [f32; 64], start: usize, step: usize) {
    let tmp0 = data[start] + data[start + step * 7];
    let tmp7 = data[start] - data[start + step * 7];
    let tmp1 = data[start + step] + data[start + step * 6];
    let tmp6 = data[start + step] - data[start + step * 6];
    let tmp2 = data[start + step * 2] + data[start + step * 5];
    let tmp5 = data[start + step * 2] - data[start + step * 5];
    let tmp3 = data[start + step * 3] + data[start + step * 4];
    let tmp4 = data[start + step * 3] - data[start + step * 4];
    let even10 = tmp0 + tmp3;
    let even13 = tmp0 - tmp3;
    let even11 = tmp1 + tmp2;
    let even12 = tmp1 - tmp2;
    data[start] = even10 + even11;
    data[start + step * 4] = even10 - even11;
    let z1 = (even12 + even13) * FRAC_1_SQRT_2;
    data[start + step * 2] = even13 + z1;
    data[start + step * 6] = even13 - z1;
    let odd10 = tmp4 + tmp5;
    let odd11 = tmp5 + tmp6;
    let odd12 = tmp6 + tmp7;
    let z5 = (odd10 - odd12) * (0.382683433_f64 as f32);
    let z2 = (0.541196100_f64 as f32) * odd10 + z5;
    let z4 = (1.306562965_f64 as f32) * odd12 + z5;
    let z3 = odd11 * FRAC_1_SQRT_2;
    let z11 = tmp7 + z3;
    let z13 = tmp7 - z3;
    data[start + step * 5] = z13 + z2;
    data[start + step * 3] = z13 - z2;
    data[start + step] = z11 + z4;
    data[start + step * 7] = z11 - z4;
}

/// One sampled component plane (`Component`).
struct Component {
    sampling: usize,
    width: usize,
    height: usize,
    stride: usize,
    samples: Vec<u8>,
    divisors: [f32; 64],
    luma: bool,
    dc: i32,
}

fn component(width: usize, height: usize, sampling: usize, table: &[u8; 64]) -> Component {
    let stride = width.div_ceil(8) * 8;
    Component {
        sampling,
        width,
        height,
        stride,
        samples: vec![0; stride * sampling * 8],
        divisors: divisors(table),
        luma: sampling == 2,
        dc: 0,
    }
}

// jccolor.c rgb_ycc_start/rgb_ycc_convert, including chroma's half-minus-one.
fn color(image: &JpegImage, row_order: RowOrder, x: usize, y: usize, channel: usize) -> u8 {
    let width = image.width as usize;
    let height = image.height as usize;
    let row = if row_order == RowOrder::BottomUp {
        height - 1 - y
    } else {
        y
    };
    let offset = (row * width + x) * 4;
    let r = i32::from(at(&image.pixels, offset));
    let g = i32::from(at(&image.pixels, offset + 1));
    let b = i32::from(at(&image.pixels, offset + 2));
    let value = if channel == 0 {
        (19_595 * r + 38_470 * g + 7471 * b + 32_768) >> 16
    } else if channel == 1 {
        (-11_059 * r - 21_709 * g + 32_768 * b + 8_421_375) >> 16
    } else {
        (32_768 * r - 27_439 * g - 5329 * b + 8_421_375) >> 16
    };
    (value & 255) as u8
}

// jcprepct.c pads the converted row pair before downsampling, then repeats
// the last downsampled row. jcsample.c alternates chroma rounding bias 1,2.
fn fill_strip(image: &JpegImage, row_order: RowOrder, component: &mut Component, channel: usize, mcu_row: usize) {
    let width = image.width as usize;
    let height = image.height as usize;
    for row in 0..component.sampling * 8 {
        let y = (mcu_row * component.sampling * 8 + row).min(component.height - 1);
        for column in 0..component.stride {
            let x = column.min(component.width - 1);
            let sample = if channel == 0 {
                color(image, row_order, x, y, 0)
            } else {
                let x0 = (column * 2).min(width - 1);
                let x1 = (x0 + 1).min(width - 1);
                let y0 = y * 2;
                let y1 = (y0 + 1).min(height - 1);
                let sum = i32::from(color(image, row_order, x0, y0, channel))
                    + i32::from(color(image, row_order, x1, y0, channel))
                    + i32::from(color(image, row_order, x0, y1, channel))
                    + i32::from(color(image, row_order, x1, y1, channel));
                ((sum + 1 + (column % 2) as i32) >> 2) as u8
            };
            component.samples[row * component.stride + column] = sample;
        }
    }
}

fn transform(component: &Component, column: usize, row: usize, block: &mut [i16; 64], workspace: &mut [f32; 64]) {
    for y in 0..8 {
        for x in 0..8 {
            workspace[y * 8 + x] = f32::from(at(
                &component.samples,
                (row * 8 + y) * component.stride + column * 8 + x,
            )) - 128.0;
        }
    }
    for row in 0..8 {
        forward_pass(workspace, row * 8, 1);
    }
    for column in 0..8 {
        forward_pass(workspace, column, 8);
    }
    for index in 0..64 {
        let scaled = workspace[index] * at(&component.divisors[..], index);
        block[index] = ((scaled + 16_384.5).trunc() as i32 - 16_384) as i16;
    }
}

fn category(value: i32) -> u32 {
    if value == 0 {
        0
    } else {
        32 - value.abs().cast_unsigned().leading_zeros()
    }
}

fn write_block(
    output: &mut Output<'_>,
    component: &mut Component,
    block: &[i16; 64],
    luma: &CodeSet,
    chroma: &CodeSet,
) -> Result<(), ContentError> {
    let codes = if component.luma { luma } else { chroma };
    let dc = i32::from(at(&block[..], 0));
    let difference = dc - component.dc;
    let size = category(difference);
    output.symbol(&codes.dc, size as usize)?;
    if size > 0 {
        output.bits(if difference < 0 { difference - 1 } else { difference }, size)?;
    }
    component.dc = dc;
    let mut zeroes = 0;
    for index in 1..64 {
        let value = i32::from(at(&block[..], at(&NATURAL_ORDER[..], index)));
        if value == 0 {
            zeroes += 1;
        } else {
            while zeroes > 15 {
                output.symbol(&codes.ac, 240)?;
                zeroes -= 16;
            }
            let size = category(value);
            output.symbol(&codes.ac, (zeroes * 16 + size as i32) as usize)?;
            output.bits(if value < 0 { value - 1 } else { value }, size)?;
            zeroes = 0;
        }
    }
    if zeroes > 0 {
        output.symbol(&codes.ac, 0)?;
    }
    Ok(())
}

impl<'a> Output<'a> {
    fn into_owned(self) -> Vec<u8> {
        match self.bytes {
            OutputBuffer::Owned(mut bytes) => {
                bytes.truncate(self.length);
                bytes
            }
            OutputBuffer::Borrowed(_) => panic!("JPEG encoder cannot take ownership of a borrowed output"),
        }
    }
}

/// Validated `(width, height)` or `None` (`encodeJpeg` dimension guard).
fn checked_dimensions(image: &JpegImage) -> Option<(usize, usize)> {
    let (width, height) = (image.width, image.height);
    if width < 1 || height < 1 || width > 65500 || height > 65500 {
        None
    } else {
        Some((width as usize, height as usize))
    }
}

fn dht_payload(selector: u8, table: &HuffmanTable) -> Vec<u8> {
    let mut payload = Vec::with_capacity(1 + 16 + table.values.len());
    payload.push(selector);
    payload.extend_from_slice(&table.counts);
    payload.extend_from_slice(table.values);
    payload
}

/// Shared compressor body behind both profiles (`encodeJpeg`).
fn encode_inner(
    image: &JpegImage,
    quality: u8,
    row_order: RowOrder,
    output: &mut Output<'_>,
) -> Result<(), ContentError> {
    let width = image.width as usize;
    let height = image.height as usize;
    let y_table = quantizers(&LUMINANCE_QUANTIZERS, quality);
    let c_table = quantizers(&CHROMINANCE_QUANTIZERS, quality);
    let luma = CodeSet {
        dc: derive_codes(&DC_LUMINANCE),
        ac: derive_codes(&AC_LUMINANCE),
    };
    let chroma = CodeSet {
        dc: derive_codes(&DC_CHROMINANCE),
        ac: derive_codes(&AC_CHROMINANCE),
    };
    output.byte(255)?;
    output.byte(216)?;
    output.marker(224, &[74, 70, 73, 70, 0, 1, 1, 0, 0, 1, 0, 1, 0, 0])?;
    let mut y_payload = [0u8; 65];
    y_payload[1..].copy_from_slice(&y_table);
    output.marker(219, &y_payload)?;
    let mut c_payload = [1u8; 65];
    c_payload[1..].copy_from_slice(&c_table);
    output.marker(219, &c_payload)?;
    output.marker(
        192,
        &[
            8,
            (height >> 8) as u8,
            (height & 255) as u8,
            (width >> 8) as u8,
            (width & 255) as u8,
            3,
            1,
            34,
            0,
            2,
            17,
            1,
            3,
            17,
            1,
        ],
    )?;
    for (selector, table) in [
        (0, &DC_LUMINANCE),
        (16, &AC_LUMINANCE),
        (1, &DC_CHROMINANCE),
        (17, &AC_CHROMINANCE),
    ] {
        output.marker(196, &dht_payload(selector, table))?;
    }
    output.marker(218, &[3, 1, 0, 2, 17, 3, 17, 0, 63, 0])?;
    let chroma_width = width.div_ceil(2);
    let chroma_height = height.div_ceil(2);
    let mut components = [
        component(width, height, 2, &y_table),
        component(chroma_width, chroma_height, 1, &c_table),
        component(chroma_width, chroma_height, 1, &c_table),
    ];
    let mut block = [0i16; 64];
    let mut workspace = [0f32; 64];
    for mcu_row in 0..height.div_ceil(16) {
        for (channel, component) in components.iter_mut().enumerate() {
            fill_strip(image, row_order, component, channel, mcu_row);
        }
        for mcu_column in 0..width.div_ceil(16) {
            for component in components.iter_mut() {
                for row in 0..component.sampling {
                    for column in 0..component.sampling {
                        let block_column = mcu_column * component.sampling + column;
                        let block_row = mcu_row * component.sampling + row;
                        if block_column >= component.width.div_ceil(8) || block_row >= component.height.div_ceil(8) {
                            // jccoefct.c dummy right/bottom blocks retain the preceding DC.
                            block.fill(0);
                            block[0] = component.dc as i16;
                        } else {
                            transform(component, block_column, row, &mut block, &mut workspace);
                        }
                        write_block(output, component, &block, &luma, &chroma)?;
                    }
                }
            }
        }
    }
    output.finish()
}

/// Encode RGBA pixels to JPEG bytes, owning a growing output (`encodeJpeg`).
///
/// # Panics
///
/// Panics when the dimensions fall outside `1..65500` or the pixels are not
/// tightly packed RGBA.
pub fn encode_jpeg(image: &JpegImage, quality: u8) -> Vec<u8> {
    let (width, height) = match checked_dimensions(image) {
        Some(dimensions) => dimensions,
        None => panic!("JPEG encoder dimensions must be integers in 1..65500"),
    };
    if image.pixels.len() != width * height * 4 {
        panic!("JPEG encoder requires tightly packed RGBA pixels");
    }
    let mut output = Output::owned();
    match encode_inner(image, quality, RowOrder::TopDown, &mut output) {
        Ok(()) => output.into_owned(),
        Err(_) => panic!("SaveJPG destination capacity exceeded; source buffer exhaustion is unsupported"),
    }
}

/// Encode into the fixed SaveJPG destination, returning bytes written.
///
/// The destination must hold exactly `width * height * 4` bytes.
pub fn encode_jpeg_to(
    image: &JpegImage,
    quality: u8,
    row_order: RowOrder,
    destination: &mut [u8],
) -> Result<usize, ContentError> {
    let (width, height) = match checked_dimensions(image) {
        Some(dimensions) => dimensions,
        None => {
            return Err(fail(
                "SaveJPG",
                0,
                "JPEG encoder dimensions must be integers in 1..65500".to_string(),
            ));
        }
    };
    if image.pixels.len() != width * height * 4 {
        return Err(fail(
            "SaveJPG",
            0,
            "JPEG encoder requires tightly packed RGBA pixels".to_string(),
        ));
    }
    if destination.len() != width * height * 4 {
        return Err(fail(
            "SaveJPG",
            0,
            "SaveJPG destination requires exactly width*height*4 bytes".to_string(),
        ));
    }
    let mut output = Output::borrowed(destination);
    encode_inner(image, quality, row_order, &mut output)?;
    Ok(output.length)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient_image(width: u32, height: u32) -> JpegImage {
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[(x * 16) as u8, (y * 16) as u8, 128, 255]);
            }
        }
        JpegImage { width, height, pixels }
    }

    /// `(marker id, payload offset, payload length)` for each header segment.
    fn header_segments(bytes: &[u8]) -> Vec<(u8, usize, usize)> {
        let mut segments = Vec::new();
        let mut offset = 2;
        while offset + 4 <= bytes.len() {
            assert_eq!(bytes[offset], 255, "segment marker at {offset}");
            let id = bytes[offset + 1];
            let length = (u16::from(bytes[offset + 2]) << 8 | u16::from(bytes[offset + 3])) as usize;
            segments.push((id, offset + 4, length - 2));
            offset += 2 + length;
            if id == 218 {
                break;
            }
        }
        segments
    }

    #[test]
    fn structure_markers_and_dimensions() {
        let bytes = encode_jpeg(&gradient_image(16, 16), 95);
        assert_eq!(&bytes[0..2], &[255, 216]);
        assert_eq!(&bytes[bytes.len() - 2..], &[255, 217]);
        let segments = header_segments(&bytes);
        let ids: Vec<u8> = segments.iter().map(|&(id, _, _)| id).collect();
        assert_eq!(ids.iter().filter(|&&id| id == 224).count(), 1);
        assert_eq!(ids.iter().filter(|&&id| id == 219).count(), 2);
        assert_eq!(ids.iter().filter(|&&id| id == 196).count(), 4);
        assert_eq!(ids.iter().filter(|&&id| id == 218).count(), 1);
        let sof0 = segments.iter().find(|&&(id, _, _)| id == 192).unwrap();
        let payload = &bytes[sof0.1..sof0.1 + sof0.2];
        assert_eq!(payload[0], 8);
        assert_eq!(u16::from(payload[1]) << 8 | u16::from(payload[2]), 16);
        assert_eq!(u16::from(payload[3]) << 8 | u16::from(payload[4]), 16);
        assert_eq!(payload[5], 3);
    }

    #[test]
    fn borrowed_output_matches_owned() {
        let image = gradient_image(16, 16);
        let owned = encode_jpeg(&image, 95);
        let mut destination = vec![0u8; 16 * 16 * 4];
        let written = encode_jpeg_to(&image, 95, RowOrder::TopDown, &mut destination).unwrap();
        assert_eq!(&destination[..written], &owned[..]);
    }

    #[test]
    fn destination_size_is_exact() {
        let image = gradient_image(16, 16);
        let mut destination = vec![0u8; 100];
        let error = encode_jpeg_to(&image, 95, RowOrder::TopDown, &mut destination).unwrap_err();
        assert!(error
            .to_string()
            .contains("SaveJPG destination requires exactly width*height*4 bytes"));
    }

    #[test]
    fn destination_capacity_reports_source_exhaustion() {
        let image = gradient_image(8, 8);
        let mut destination = vec![0u8; 8 * 8 * 4];
        let error = encode_jpeg_to(&image, 95, RowOrder::TopDown, &mut destination).unwrap_err();
        assert!(error
            .to_string()
            .contains("SaveJPG destination capacity exceeded; source buffer exhaustion is unsupported"));
    }

    #[test]
    #[should_panic(expected = "JPEG encoder dimensions must be integers in 1..65500")]
    fn rejects_zero_width() {
        encode_jpeg(
            &JpegImage {
                width: 0,
                height: 16,
                pixels: Vec::new(),
            },
            95,
        );
    }

    #[test]
    #[should_panic(expected = "JPEG encoder requires tightly packed RGBA pixels")]
    fn rejects_ragged_pixels() {
        encode_jpeg(
            &JpegImage {
                width: 16,
                height: 16,
                pixels: vec![0; 100],
            },
            95,
        );
    }
}
