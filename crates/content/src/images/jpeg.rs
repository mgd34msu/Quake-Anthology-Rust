//! IJG JPEG decoder used by the Quake III renderer (`LoadJPG`).
//!
//! Donor provenance: `src/formats/images/jpeg.ts` (a translation of
//! `jdmarker.c`, `jdhuff.c`, `jdphuff.c`, `jdcoefct.c`, `jdinput.c`,
//! `jdmainct.c`, `jdapimin.c`, `jddctmgr.c`, `jidctflt.c`, `jdsample.c`
//! and `jdcolor.c`). Supports 8-bit sequential and progressive Huffman
//! JPEG; grayscale expands to RGBA.

use std::collections::HashMap;
use std::f32::consts::SQRT_2;
use std::fmt;

use super::{fail, ContentError};

/// Decoded renderer pixels, top-to-bottom and opaque (`JpegImage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes.
    pub pixels: Vec<u8>,
}

/// A defined IJG `error_exit` message (`JpegSourceError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegSourceError {
    /// Input name for diagnostics.
    pub input: String,
    /// Byte offset of the failure.
    pub offset: usize,
    /// IJG source message.
    pub source_message: String,
}

impl fmt::Display for JpegSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: JPEG: {}", self.input, self.offset, self.source_message)
    }
}

impl std::error::Error for JpegSourceError {}

impl From<JpegSourceError> for ContentError {
    fn from(error: JpegSourceError) -> Self {
        fail(&error.input, error.offset, format!("JPEG: {}", error.source_message))
    }
}

const NATURAL_ORDER: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21,
    28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54,
    47, 55, 62, 63,
];

struct Warnings<'a> {
    count: usize,
    print: &'a mut dyn FnMut(&str),
}

impl Warnings<'_> {
    fn emit(&mut self, message: String) {
        // jerror.c emit_message, with the renderer's default trace_level = 0.
        if self.count == 0 {
            let print = &mut *self.print;
            print(&message);
        }
        self.count += 1;
    }
}

struct Input<'a> {
    bytes: &'a [u8],
    source: String,
    offset: usize,
    unread_marker: u8,
    discarded_bytes: usize,
    warnings: Warnings<'a>,
}

impl Input<'_> {
    fn fail(&self, message: &str) -> ContentError {
        fail(&self.source, self.offset, format!("JPEG: {message}"))
    }

    fn source_fail(&self, message: String) -> ContentError {
        ContentError::from(JpegSourceError {
            input: self.source.clone(),
            offset: self.offset,
            source_message: message,
        })
    }

    fn byte(&mut self) -> Result<u8, ContentError> {
        let value = self
            .bytes
            .get(self.offset)
            .copied()
            .ok_or_else(|| self.fail("truncated input"))?;
        self.offset += 1;
        Ok(value)
    }

    fn word(&mut self) -> Result<u16, ContentError> {
        Ok(u16::from(self.byte()?) * 256 + u16::from(self.byte()?))
    }

    fn next_marker(&mut self) -> Result<(), ContentError> {
        loop {
            let mut marker = self.byte()?;
            while marker != 255 {
                self.discarded_bytes += 1;
                marker = self.byte()?;
            }
            loop {
                marker = self.byte()?;
                if marker != 255 {
                    break;
                }
            }
            if marker == 0 {
                self.discarded_bytes += 2;
                continue;
            }
            if self.discarded_bytes != 0 {
                self.warnings.emit(format!(
                    "Corrupt JPEG data: {} extraneous bytes before marker 0x{marker:02x}",
                    self.discarded_bytes
                ));
                self.discarded_bytes = 0;
            }
            self.unread_marker = marker;
            return Ok(());
        }
    }

    fn marker(&mut self) -> Result<u8, ContentError> {
        if self.unread_marker == 0 {
            self.next_marker()?;
        }
        let marker = self.unread_marker;
        self.unread_marker = 0;
        Ok(marker)
    }

    fn restart(&mut self, desired: u8) -> Result<(), ContentError> {
        if self.unread_marker == 0 {
            self.next_marker()?;
        }
        if self.unread_marker == 208 + desired {
            self.unread_marker = 0;
            return Ok(());
        }
        self.warnings.emit(format!(
            "Corrupt JPEG data: found marker 0x{:02x} instead of RST{desired}",
            self.unread_marker
        ));
        // jdmarker.c jpeg_resync_to_restart: retain nearby future markers,
        // search past stale markers, and consume distant restart markers.
        loop {
            let marker = self.unread_marker;
            if marker < 192 || marker == 208 + (desired + 7) % 8 || marker == 208 + (desired + 6) % 8 {
                self.next_marker()?;
            } else if !(208..=215).contains(&marker)
                || marker == 208 + (desired + 1) % 8
                || marker == 208 + (desired + 2) % 8
            {
                return Ok(());
            } else {
                self.unread_marker = 0;
                return Ok(());
            }
        }
    }

    fn skip(&mut self, size: usize) -> Result<(), ContentError> {
        if size == 0 {
            return Ok(());
        }
        if size > self.bytes.len() - self.offset {
            self.offset = self.bytes.len();
            return Err(self.fail("truncated input"));
        }
        self.offset += size;
        Ok(())
    }
}

struct Entropy {
    value: u32,
    remaining: i32,
    printed_end: bool,
}

impl Entropy {
    fn fill(&mut self, input: &mut Input<'_>, required: i32) -> Result<(), ContentError> {
        while self.remaining < 25 {
            let mut next = 0u32;
            if input.unread_marker != 0 {
                if self.remaining >= required {
                    break;
                }
                if !self.printed_end {
                    input
                        .warnings
                        .emit("Corrupt JPEG data: premature end of data segment".to_string());
                    self.printed_end = true;
                }
            } else {
                next = u32::from(input.byte()?);
                if next == 255 {
                    let mut following = input.byte()?;
                    while following == 255 {
                        following = input.byte()?;
                    }
                    if following != 0 {
                        input.unread_marker = following;
                        continue;
                    }
                }
            }
            self.value = self.value.wrapping_shl(8) | next;
            self.remaining += 8;
        }
        Ok(())
    }

    fn bits(&mut self, input: &mut Input<'_>, count: u32) -> Result<u32, ContentError> {
        if self.remaining < count as i32 {
            self.fill(input, count as i32)?;
        }
        self.remaining -= count as i32;
        Ok((self.value >> ((self.remaining & 31) as u32)) & ((1u32 << count) - 1))
    }

    fn signed(&mut self, input: &mut Input<'_>, count: u32) -> Result<i32, ContentError> {
        if count > 15 {
            return Err(input.fail("DC category exceeds source sign-extension table"));
        }
        let value = self.bits(input, count)?;
        if count > 0 && value < 1u32 << (count - 1) {
            return Ok(value as i32 + 1 - (1i32 << count));
        }
        Ok(value as i32)
    }

    fn align(&mut self) {
        // IJG discards unused bits at scan and restart boundaries.
        self.remaining = 0;
    }

    fn restart(&mut self, input: &mut Input<'_>, desired: u8) -> Result<(), ContentError> {
        input.discarded_bytes += (self.remaining / 8).max(0) as usize;
        self.align();
        input.restart(desired)?;
        self.printed_end = false;
        Ok(())
    }

    fn symbol(&mut self, input: &mut Input<'_>, table: &DerivedHuffman) -> Result<u8, ContentError> {
        if self.remaining < 8 {
            self.fill(input, 0)?;
        }
        if self.remaining >= 8 {
            // jdhuff.c HUFF_LOOKAHEAD fast path: one array probe per symbol.
            let look = ((self.value >> ((self.remaining - 8) as u32)) & 255) as usize;
            let nbits = table.look_nbits[look];
            if nbits != 0 {
                self.remaining -= i32::from(nbits);
                return Ok(table.look_sym[look]);
            }
        }
        // jpeg_huff_decode slow path: one bit at a time against maxcode/valptr.
        let mut code = self.bits(input, 1)? as i32;
        for length in 1..=16usize {
            if code <= table.maxcode[length] {
                let index = code + table.valptr[length];
                if let Some(&symbol) = table.symbols.get(index as usize) {
                    return Ok(symbol);
                }
                break;
            }
            code = code * 2 + self.bits(input, 1)? as i32;
        }
        // jpeg_huff_decode consumes the seventeenth bit before recovery.
        input.warnings.emit("Corrupt JPEG data: bad Huffman code".to_string());
        Ok(0)
    }
}

struct QuantTable {
    quantizer: [u16; 64],
    multipliers: [f32; 64],
}

struct Progression {
    coefficients: Vec<i16>,
    bits: [i8; 64],
    table: Option<QuantTable>,
}

struct Component {
    id: u8,
    h: usize,
    v: usize,
    quantizer: u8,
    width: usize,
    height: usize,
    stride: usize,
    pixels: Vec<u8>,
    output_samples: Option<Vec<u8>>,
    progression: Option<Progression>,
}

struct Frame {
    progressive: bool,
    width: usize,
    height: usize,
    max_h: usize,
    max_v: usize,
    columns: usize,
    rows: usize,
    components: Vec<Component>,
}

struct ComponentSpec {
    id: u8,
    h: usize,
    v: usize,
    quantizer: u8,
}

struct FrameHeader {
    progressive: bool,
    arithmetic: bool,
    precision: u8,
    width: usize,
    height: usize,
    components: Vec<ComponentSpec>,
}

struct ScanMember {
    id: u8,
    selector: u8,
}

struct ScanHeader {
    members: Vec<ScanMember>,
    start: u8,
    end: u8,
    approximation: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorSpace {
    Grayscale,
    Rgb,
    Ycbcr,
    Cmyk,
    Ycck,
    Unknown,
}

struct ScanComponent {
    component: usize,
    prediction_dc: i32,
    multipliers: [f32; 64],
}

/// IJG `d_derived_tbl` (jdhuff.c): 256-entry lookahead plus `maxcode`/`valptr`
/// for codes longer than 8 bits. Built once per DHT definition and borrowed
/// by every scan; never cloned per scan.
struct DerivedHuffman {
    /// Symbols in code order (`huffval`).
    symbols: [u8; 256],
    /// Largest code of each length, `-1` when the length is unused (`maxcode`).
    maxcode: [i32; 18],
    /// `symbols` index of each length's first code minus that code (`valptr`).
    valptr: [i32; 18],
    /// Fast path: code length for each 8-bit prefix, `0` when longer (`look_nbits`).
    look_nbits: [u8; 256],
    /// Fast path: symbol for each 8-bit prefix (`look_sym`).
    look_sym: [u8; 256],
}

struct HuffmanTable {
    counts: [u8; 16],
    values: Vec<u8>,
    derived: Option<DerivedHuffman>,
}

fn read_frame(
    input: &mut Input<'_>,
    progressive: bool,
    arithmetic: bool,
    duplicate: bool,
) -> Result<FrameHeader, ContentError> {
    let length = input.word()?;
    let precision = input.byte()?;
    let height = usize::from(input.word()?);
    let width = usize::from(input.word()?);
    let count = input.byte()?;
    if duplicate {
        return Err(input.source_fail("Invalid JPEG file structure: two SOF markers".to_string()));
    }
    if width == 0 || height == 0 || count == 0 {
        return Err(input.source_fail("Empty JPEG image (DNL not supported)".to_string()));
    }
    if length != 8 + u16::from(count) * 3 {
        return Err(input.source_fail("Bogus marker length".to_string()));
    }
    let mut components = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let id = input.byte()?;
        let sampling = input.byte()?;
        components.push(ComponentSpec {
            id,
            h: usize::from(sampling >> 4),
            v: usize::from(sampling & 15),
            quantizer: input.byte()?,
        });
    }
    Ok(FrameHeader {
        progressive,
        arithmetic,
        precision,
        width,
        height,
        components,
    })
}

fn alloc_bytes(input: &Input<'_>, length: usize, what: &str) -> Result<Vec<u8>, ContentError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| input.fail(&format!("JPEG {what} allocation of {length} bytes failed")))?;
    bytes.resize(length, 0);
    Ok(bytes)
}

fn alloc_i16(input: &Input<'_>, length: usize, what: &str) -> Result<Vec<i16>, ContentError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| input.fail(&format!("JPEG {what} allocation of {length} coefficients failed")))?;
    values.resize(length, 0);
    Ok(values)
}

// jdinput.c initial_setup runs only after the first complete SOS header.
fn prepare_frame(input: &mut Input<'_>, header: &FrameHeader, buffered: bool) -> Result<Frame, ContentError> {
    let count = header.components.len();
    if header.width > 65500 || header.height > 65500 {
        return Err(input.source_fail("Maximum supported image dimension is 65500 pixels".to_string()));
    }
    if header.precision != 8 {
        return Err(input.source_fail(format!("Unsupported JPEG data precision {}", header.precision)));
    }
    if count > 10 {
        return Err(input.source_fail(format!("Too many color components: {count}, max 10")));
    }
    let mut max_h = 1usize;
    let mut max_v = 1usize;
    for spec in &header.components {
        if spec.h < 1 || spec.h > 4 || spec.v < 1 || spec.v > 4 {
            return Err(input.source_fail("Bogus sampling factors".to_string()));
        }
        max_h = max_h.max(spec.h);
        max_v = max_v.max(spec.v);
    }
    let columns = header.width.div_ceil(8 * max_h);
    let rows = header.height.div_ceil(8 * max_v);
    let mut components = Vec::with_capacity(count);
    for spec in &header.components {
        let stride = columns * spec.h * 8;
        let width = (header.width * spec.h).div_ceil(max_h);
        let height = (header.height * spec.v).div_ceil(max_v);
        components.push(Component {
            id: spec.id,
            h: spec.h,
            v: spec.v,
            quantizer: spec.quantizer,
            width,
            height,
            stride,
            pixels: if buffered {
                alloc_bytes(input, stride * rows * spec.v * 8, "component plane")?
            } else {
                Vec::new()
            },
            output_samples: if buffered {
                None
            } else {
                Some(alloc_bytes(input, header.width * header.height, "component output")?)
            },
            progression: if buffered {
                Some(Progression {
                    coefficients: alloc_i16(input, stride * rows * spec.v * 8, "coefficient")?,
                    bits: [-1; 64],
                    table: None,
                })
            } else {
                None
            },
        });
    }
    Ok(Frame {
        progressive: header.progressive,
        width: header.width,
        height: header.height,
        max_h,
        max_v,
        columns,
        rows,
        components,
    })
}

fn read_scan_header(input: &mut Input<'_>, ids: &[u8]) -> Result<ScanHeader, ContentError> {
    let length = input.word()?;
    let count = input.byte()?;
    if !(1..=4).contains(&count) || length != 6 + u16::from(count) * 2 {
        return Err(input.source_fail("Bogus marker length".to_string()));
    }
    let mut members: Vec<ScanMember> = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let id = input.byte()?;
        let selector = input.byte()?;
        if !ids.contains(&id) {
            return Err(input.source_fail(format!("Invalid component ID {id} in SOS")));
        }
        for member in &mut members {
            if member.id == id {
                member.selector = selector;
            }
        }
        members.push(ScanMember { id, selector });
    }
    Ok(ScanHeader {
        members,
        start: input.byte()?,
        end: input.byte()?,
        approximation: input.byte()?,
    })
}

fn read_quantizers(input: &mut Input<'_>, tables: &mut HashMap<u8, [u16; 64]>) -> Result<(), ContentError> {
    let mut remaining = i32::from(input.word()?) - 2;
    while remaining > 0 {
        let selector = input.byte()?;
        let precision = selector >> 4;
        let id = selector & 15;
        if id > 3 {
            return Err(input.source_fail(format!("Bogus DQT index {id}")));
        }
        let mut table = [0u16; 64];
        for index in 0..64 {
            let value = if precision == 0 {
                u16::from(input.byte()?)
            } else {
                input.word()?
            };
            table[NATURAL_ORDER[index]] = value;
        }
        tables.insert(id, table);
        remaining -= if precision == 0 { 65 } else { 129 };
    }
    Ok(())
}

fn read_huffman(input: &mut Input<'_>, tables: &mut HashMap<u8, HuffmanTable>) -> Result<(), ContentError> {
    let mut remaining = i32::from(input.word()?) - 2;
    while remaining > 0 {
        let selector = input.byte()?;
        let mut counts = [0u8; 16];
        let mut total = 0i32;
        for count in counts.iter_mut() {
            let value = input.byte()?;
            *count = value;
            total += i32::from(value);
        }
        remaining -= 17;
        if total > 256 || total > remaining {
            return Err(input.source_fail("Bogus DHT counts".to_string()));
        }
        let mut values = Vec::with_capacity(total as usize);
        for _ in 0..total {
            values.push(input.byte()?);
        }
        remaining -= total;
        let table_index = if selector & 16 != 0 { selector - 16 } else { selector };
        if table_index >= 4 {
            return Err(input.source_fail(format!("Bogus DHT index {table_index}")));
        }
        tables.insert(
            selector,
            HuffmanTable {
                counts,
                values,
                derived: None,
            },
        );
    }
    Ok(())
}

// jdhuff.c derives tables at scan startup, not while reading DHT markers.
// jpeg_make_d_derived_tbl: canonical codes feed the lookahead plus
// maxcode/valptr; callers borrow the stored table instead of cloning it.
fn build_derived(input: &Input<'_>, raw: &HuffmanTable) -> Result<DerivedHuffman, ContentError> {
    let mut derived = DerivedHuffman {
        symbols: [0; 256],
        maxcode: [-1; 18],
        valptr: [0; 18],
        look_nbits: [0; 256],
        look_sym: [0; 256],
    };
    let mut code = 0u32;
    let mut value_index = 0usize;
    let mut symbol_index = 0usize;
    for (length0, count) in raw.counts.iter().enumerate() {
        let length = length0 as u32 + 1;
        // Short overfull codes write beyond IJG's 256-entry lookahead arrays.
        if length <= 8 && *count != 0 && code + u32::from(*count) > 1u32 << length {
            return Err(input.fail("oversubscribed Huffman table exceeds source lookahead allocation"));
        }
        let first_code = code;
        let first_symbol = symbol_index;
        let mut assigned = 0u32;
        for _ in 0..*count {
            let value = raw
                .values
                .get(value_index)
                .copied()
                .ok_or_else(|| input.fail("oversubscribed Huffman table exceeds source lookahead allocation"))?;
            value_index += 1;
            if code < 1u32 << length {
                derived.symbols[symbol_index] = value;
                symbol_index += 1;
                assigned += 1;
            }
            code += 1;
        }
        if assigned > 0 {
            derived.maxcode[length as usize] = (first_code + assigned - 1) as i32;
            derived.valptr[length as usize] = first_symbol as i32 - first_code as i32;
            if length <= 8 {
                let span = 1usize << (8 - length);
                for offset in 0..assigned {
                    let prefix = ((first_code + offset) << (8 - length)) as usize;
                    let symbol = derived.symbols[first_symbol + offset as usize];
                    for slot in prefix..prefix + span {
                        derived.look_nbits[slot] = length as u8;
                        derived.look_sym[slot] = symbol;
                    }
                }
            }
        }
        code *= 2;
    }
    derived.maxcode[17] = 0xFFFFF;
    Ok(derived)
}

fn derive_huffman(
    input: &mut Input<'_>,
    tables: &mut HashMap<u8, HuffmanTable>,
    selector: u8,
) -> Result<(), ContentError> {
    let raw = tables
        .get(&selector)
        .ok_or_else(|| input.source_fail(format!("Huffman table 0x{:02x} was not defined", selector & 15)))?;
    if raw.derived.is_some() {
        return Ok(());
    }
    let derived = {
        let raw = tables
            .get(&selector)
            .ok_or_else(|| input.source_fail(format!("Huffman table 0x{:02x} was not defined", selector & 15)))?;
        build_derived(input, raw)?
    };
    if let Some(raw) = tables.get_mut(&selector) {
        raw.derived = Some(derived);
    }
    Ok(())
}

fn derived_table<'a>(
    tables: &'a HashMap<u8, HuffmanTable>,
    selector: u8,
    input: &Input<'_>,
) -> Result<&'a DerivedHuffman, ContentError> {
    tables
        .get(&selector)
        .and_then(|raw| raw.derived.as_ref())
        .ok_or_else(|| input.source_fail(format!("Huffman table 0x{:02x} was not defined", selector & 15)))
}

// jdmarker.c get_dac validates these tables even for a Huffman frame.
fn read_arithmetic_tables(input: &mut Input<'_>) -> Result<(), ContentError> {
    let mut remaining = i32::from(input.word()?) - 2;
    while remaining > 0 {
        let index = input.byte()?;
        let value = input.byte()?;
        remaining -= 2;
        if index >= 32 {
            return Err(input.source_fail(format!("Bogus DAC index {index}")));
        }
        if index < 16 && (value & 15) > (value >> 4) {
            return Err(input.source_fail(format!("Bogus DAC value 0x{value:02x}")));
        }
    }
    Ok(())
}

// jdmarker.c reads fixed APP prefixes before skipping any declared tail.
fn read_jfif(input: &mut Input<'_>) -> Result<bool, ContentError> {
    let mut remaining = i32::from(input.word()?) - 2;
    let mut recognized = false;
    if remaining >= 14 {
        let mut prefix = [0u8; 14];
        for byte in &mut prefix {
            *byte = input.byte()?;
        }
        remaining -= 14;
        recognized = prefix[..5] == *b"JFIF\0";
        if recognized && prefix[5] != 1 {
            input.warnings.emit(format!(
                "Warning: unknown JFIF revision number {}.{:02}",
                prefix[5], prefix[6]
            ));
        }
    }
    input.skip(remaining.max(0) as usize)?;
    Ok(recognized)
}

fn read_adobe(input: &mut Input<'_>) -> Result<Option<u8>, ContentError> {
    let mut remaining = i32::from(input.word()?) - 2;
    let mut transform = None;
    if remaining >= 12 {
        let mut prefix = [0u8; 12];
        for byte in &mut prefix {
            *byte = input.byte()?;
        }
        remaining -= 12;
        if prefix[..5] == *b"Adobe" {
            transform = Some(prefix[11]);
        }
    }
    input.skip(remaining.max(0) as usize)?;
    Ok(transform)
}

fn descale(value: i32, bits: u32) -> i32 {
    ((i64::from(value) + (1i64 << (bits - 1))).div_euclid(1i64 << bits)) as i32
}

fn clamp(value: i32) -> u8 {
    value.clamp(0, 255) as u8
}

// jddctmgr.c start_pass: double-precision AA&N scaling, stored as FAST_FLOAT.
fn float_multipliers(quantizer: &[u16; 64]) -> [f32; 64] {
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
    for (index, value) in quantizer.iter().enumerate() {
        result[index] = (f64::from(*value) * SCALES[index / 8] * SCALES[index % 8]) as f32;
    }
    result
}

// jidctflt.c jpeg_idct_float: FAST_FLOAT is float in the pinned jconfig.h.
// Round arithmetic as well as assignments, without fused multiply-add.
fn inverse_pass(
    values: &[f32; 64],
    start: usize,
    step: usize,
    output: &mut [f32; 64],
    output_start: usize,
    output_step: usize,
) {
    let a0 = values[start];
    let a1 = values[start + step];
    let a2 = values[start + step * 2];
    let a3 = values[start + step * 3];
    let a4 = values[start + step * 4];
    let a5 = values[start + step * 5];
    let a6 = values[start + step * 6];
    let a7 = values[start + step * 7];
    let even10 = a0 + a4;
    let even11 = a0 - a4;
    let even13 = a2 + a6;
    let even12 = (a2 - a6) * SQRT_2 - even13;
    let tmp0 = even10 + even13;
    let tmp3 = even10 - even13;
    let tmp1 = even11 + even12;
    let tmp2 = even11 - even12;
    let z13 = a5 + a3;
    let z10 = a5 - a3;
    let z11 = a1 + a7;
    let z12 = a1 - a7;
    let tmp7 = z11 + z13;
    let odd11 = (z11 - z13) * SQRT_2;
    let z5 = (z10 + z12) * (1.847759065_f64 as f32);
    let odd10 = (1.082392200_f64 as f32) * z12 - z5;
    let odd12 = (-2.613125930_f64 as f32) * z10 + z5;
    let tmp6 = odd12 - tmp7;
    let tmp5 = odd11 - tmp6;
    let tmp4 = odd10 + tmp5;
    output[output_start] = tmp0 + tmp7;
    output[output_start + output_step * 7] = tmp0 - tmp7;
    output[output_start + output_step] = tmp1 + tmp6;
    output[output_start + output_step * 6] = tmp1 - tmp6;
    output[output_start + output_step * 2] = tmp2 + tmp5;
    output[output_start + output_step * 5] = tmp2 - tmp5;
    output[output_start + output_step * 4] = tmp3 + tmp4;
    output[output_start + output_step * 3] = tmp3 - tmp4;
}

// jdmaster.c prepare_range_limit_table and jidctflt.c's INT32-before-DESCALE.
fn idct_sample(value: f32) -> u8 {
    let index = descale(value.trunc() as i32, 3) & 1023;
    if index < 128 {
        return (index + 128) as u8;
    }
    if index < 512 {
        return 255;
    }
    if index < 896 {
        return 0;
    }
    (index - 896) as u8
}

#[allow(clippy::too_many_arguments)]
fn decode_block(
    input: &mut Input<'_>,
    entropy: &mut Entropy,
    frame: &mut Frame,
    scan: &mut ScanComponent,
    dc: &DerivedHuffman,
    ac: &DerivedHuffman,
    column: usize,
    row: usize,
    block: &mut [f32; 64],
) -> Result<(), ContentError> {
    let component = frame
        .components
        .get_mut(scan.component)
        .ok_or_else(|| input.fail("scan output uses an uninitialized source component pointer"))?;
    let offset = (row * (component.stride / 8) + column) * 64;
    if component.progression.is_none() {
        block.fill(0.0);
    } else {
        let storage = component
            .progression
            .as_ref()
            .ok_or_else(|| input.fail("buffered frame without coefficient storage"))?;
        let window = storage
            .coefficients
            .get(offset..offset + 64)
            .ok_or_else(|| input.fail("block outside coefficient storage"))?;
        for (slot, value) in block.iter_mut().zip(window.iter()) {
            *slot = f32::from(*value);
        }
    }
    let dc_size = entropy.symbol(input, dc)?;
    scan.prediction_dc = scan
        .prediction_dc
        .wrapping_add(entropy.signed(input, u32::from(dc_size))?);
    block[0] = ((scan.prediction_dc << 16) >> 16) as f32;
    let mut index = 1i32;
    while index < 64 {
        let symbol = entropy.symbol(input, ac)?;
        let run = i32::from(symbol >> 4);
        let size = symbol & 15;
        if size == 0 {
            if run != 15 {
                break;
            }
            index += 16;
        } else {
            index += run;
            // jpeg_natural_order has sixteen sentinel entries pointing at 63.
            let natural = NATURAL_ORDER[index.min(63) as usize];
            index += 1;
            block[natural] = entropy.signed(input, u32::from(size))? as f32;
        }
    }
    let component = frame
        .components
        .get_mut(scan.component)
        .ok_or_else(|| input.fail("scan output uses an uninitialized source component pointer"))?;
    if let Some(storage) = component.progression.as_mut() {
        let window = storage
            .coefficients
            .get_mut(offset..offset + 64)
            .ok_or_else(|| input.fail("block outside coefficient storage"))?;
        for (slot, value) in window.iter_mut().zip(block.iter()) {
            *slot = *value as i16;
        }
    } else {
        for (index, value) in block.iter_mut().enumerate() {
            *value *= scan.multipliers[index];
        }
    }
    Ok(())
}

fn inverse_block(block: &mut [f32; 64], workspace: &mut [f32; 64]) {
    for x in 0..8 {
        let mut dc_only = true;
        for y in 1..8 {
            dc_only = dc_only && block[y * 8 + x] == 0.0;
        }
        if dc_only {
            for y in 0..8 {
                workspace[y * 8 + x] = block[x];
            }
        } else {
            let snapshot = *block;
            inverse_pass(&snapshot, x, 8, workspace, x, 8);
        }
    }
    let snapshot = *workspace;
    for y in 0..8 {
        inverse_pass(&snapshot, y * 8, 1, block, y * 8, 1);
    }
}

fn write_block(
    input: &mut Input<'_>,
    component: &mut Component,
    column: usize,
    row: usize,
    block: &mut [f32; 64],
    workspace: &mut [f32; 64],
) -> Result<(), ContentError> {
    inverse_block(block, workspace);
    for y in 0..8 {
        let destination = (row * 8 + y) * component.stride + column * 8;
        if destination + 8 > component.pixels.len() {
            return Err(input.fail("block outside component plane"));
        }
        for x in 0..8 {
            component.pixels[destination + x] = idct_sample(block[y * 8 + x]);
        }
    }
    Ok(())
}

struct StripPlane {
    pixels: Vec<u8>,
    initialized: Vec<u8>,
    pointers: Vec<Option<usize>>,
    starts: [usize; 2],
}

// jdmainct.c's physical strip and two pointer lists. Keep initialization
// separate from bytes: native alloc_sarray does not zero its sample storage.
struct OnePassRows {
    context: bool,
    planes: Vec<StripPlane>,
    which: usize,
    output_row: usize,
    color_space: ColorSpace,
}

impl OnePassRows {
    fn new(frame: &Frame, color_space: ColorSpace) -> Self {
        let context = frame
            .components
            .iter()
            .any(|component| frame.max_h == component.h * 2 && frame.max_v == component.v * 2 && component.width > 2);
        let planes = frame
            .components
            .iter()
            .map(|component| {
                let v = component.v;
                let stride = component.width.div_ceil(8) * 8;
                let rows = v * if context { 10 } else { 8 };
                let starts = if context { [v, v * 13] } else { [0, 0] };
                let mut pointers = vec![None; if context { v * 24 } else { rows }];
                for start in starts {
                    for row in 0..rows {
                        pointers[start + row] = Some(row * stride);
                    }
                }
                if context {
                    for row in 0..v * 2 {
                        pointers[starts[1] + v * 6 + row] = Some((v * 8 + row) * stride);
                        pointers[starts[1] + v * 8 + row] = Some((v * 6 + row) * stride);
                    }
                    for row in 0..v {
                        pointers[starts[0] - v + row] = Some(0);
                    }
                }
                StripPlane {
                    pixels: vec![0; stride * rows],
                    initialized: vec![0; stride * rows],
                    pointers,
                    starts,
                }
            })
            .collect();
        Self {
            context,
            planes,
            which: 0,
            output_row: 0,
            color_space,
        }
    }

    fn plane(&self, input: &Input<'_>, index: usize) -> Result<&StripPlane, ContentError> {
        self.planes
            .get(index)
            .ok_or_else(|| input.fail("scan output uses an uninitialized source component pointer"))
    }

    fn row_pointer(
        &self,
        input: &Input<'_>,
        plane: &StripPlane,
        row: i64,
        which: usize,
    ) -> Result<usize, ContentError> {
        let index = plane.starts[which] as i64 + row;
        if index < 0 || index >= plane.pointers.len() as i64 {
            return Err(input.fail("row pointer outside source pointer allocation"));
        }
        plane.pointers[index as usize].ok_or_else(|| input.fail("uninitialized source row pointer"))
    }

    // Donor arity: mirrors jdmainct's strip-write operands one-to-one.
    #[allow(clippy::too_many_arguments)]
    fn write(
        &mut self,
        input: &mut Input<'_>,
        frame: &Frame,
        index: usize,
        column: usize,
        row: usize,
        block: &mut [f32; 64],
        workspace: &mut [f32; 64],
    ) -> Result<(), ContentError> {
        // alloc_funny_pointers allocates both top-level component lists together;
        // an extra list0 scan member can therefore address a list1 component.
        let physical = self.which * self.planes.len() + index;
        if self.context && physical >= self.planes.len() * 2 {
            return Err(input.fail("component pointer outside source xbuffer allocation"));
        }
        let plane_index = if self.context {
            physical % self.planes.len()
        } else {
            index
        };
        let which = usize::from(self.context && physical >= self.planes.len());
        if self.planes.get(plane_index).is_none() {
            return Err(input.fail("scan output uses an uninitialized source component pointer"));
        }
        inverse_block(block, workspace);
        for y in 0..8 {
            let plane = &self.planes[plane_index];
            let destination = self.row_pointer(input, plane, (row * 8 + y) as i64, which)? + column * 8;
            let plane = &mut self.planes[plane_index];
            if destination + 8 > plane.pixels.len() {
                return Err(input.fail("IDCT write outside source sample allocation"));
            }
            for x in 0..8 {
                plane.pixels[destination + x] = idct_sample(block[y * 8 + x]);
                plane.initialized[destination + x] = 1;
            }
        }
        let _ = frame;
        Ok(())
    }

    fn read(&self, input: &mut Input<'_>, plane_index: usize, row: i64, column: usize) -> Result<u8, ContentError> {
        let plane = self.plane(input, plane_index)?;
        let offset = self.row_pointer(input, plane, row, self.which)? + column;
        if offset >= plane.pixels.len() {
            return Err(input.fail("upsampling read outside source sample allocation"));
        }
        if plane.initialized[offset] != 1 {
            return Err(input.fail("upsampling reads an uninitialized source sample"));
        }
        Ok(plane.pixels[offset])
    }

    fn emit_group(
        &mut self,
        input: &mut Input<'_>,
        frame: &mut Frame,
        color_space: ColorSpace,
        group: usize,
    ) -> Result<(), ContentError> {
        if self.output_row >= frame.height {
            return Ok(());
        }
        for index in 0..frame.components.len() {
            let (h_ratio, v_ratio, width) = {
                let component = &frame.components[index];
                (frame.max_h / component.h, frame.max_v / component.v, component.width)
            };
            if h_ratio == 1 && v_ratio == 1 {
                continue;
            }
            // Non-fullsize methods upsample every row before output cropping.
            for y in 0..frame.max_v {
                let row = group * frame.components[index].v + y / v_ratio;
                for x in 0..frame.width {
                    let column = x / h_ratio;
                    let center = self.read(input, index, row as i64, column)?;
                    let mut value = center;
                    if h_ratio == 2 && width > 2 && (v_ratio == 1 || v_ratio == 2) {
                        let neighbor = if x % 2 == 0 {
                            column.saturating_sub(1)
                        } else {
                            (column + 1).min(width - 1)
                        };
                        let horizontal = self.read(input, index, row as i64, neighbor)?;
                        if v_ratio == 1 {
                            value = ((u16::from(center) * 3 + u16::from(horizontal) + if x % 2 == 0 { 1 } else { 2 })
                                >> 2) as u8;
                        } else {
                            let adjacent = row as i64 + if y % 2 == 0 { -1 } else { 1 };
                            let vertical = self.read(input, index, adjacent, column)?;
                            let diagonal = self.read(input, index, adjacent, neighbor)?;
                            value = ((u16::from(center) * 9
                                + u16::from(horizontal) * 3
                                + u16::from(vertical) * 3
                                + u16::from(diagonal)
                                + if x % 2 == 0 { 8 } else { 7 })
                                >> 4) as u8;
                        }
                    }
                    if self.output_row + y < frame.height {
                        let component = &mut frame.components[index];
                        let output = component
                            .output_samples
                            .as_mut()
                            .ok_or_else(|| input.fail("JPEG one-pass component lacks output samples"))?;
                        output[(self.output_row + y) * frame.width + x] = value;
                    }
                }
            }
        }
        // fullsize_upsample only aliases pointers. Its actual reads happen in
        // jdcolor, after every non-fullsize upsampler has completed the group.
        let height = frame.height.min(self.output_row + frame.max_v);
        if color_space == ColorSpace::Ycbcr || color_space == ColorSpace::Ycck {
            for y in 0..height - self.output_row {
                for x in 0..frame.width {
                    for index in 0..frame.components.len() {
                        self.read_fullsize(input, frame, index, group, x, y)?;
                    }
                }
            }
        } else {
            for index in 0..frame.components.len() {
                for x in 0..frame.width {
                    for y in 0..height - self.output_row {
                        self.read_fullsize(input, frame, index, group, x, y)?;
                    }
                }
            }
        }
        self.output_row += frame.max_v;
        Ok(())
    }

    fn read_fullsize(
        &self,
        input: &mut Input<'_>,
        frame: &mut Frame,
        index: usize,
        group: usize,
        x: usize,
        y: usize,
    ) -> Result<(), ContentError> {
        let component = frame
            .components
            .get(index)
            .ok_or_else(|| input.fail("JPEG missing color component"))?;
        if component.h != frame.max_h || component.v != frame.max_v {
            return Ok(());
        }
        let row = group * component.v + y;
        let value = self.read(input, index, row as i64, x)?;
        let component = frame
            .components
            .get_mut(index)
            .ok_or_else(|| input.fail("JPEG missing color component"))?;
        let output = component
            .output_samples
            .as_mut()
            .ok_or_else(|| input.fail("JPEG one-pass component lacks output samples"))?;
        output[(self.output_row + y) * frame.width + x] = value;
        Ok(())
    }

    fn finish_row(&mut self, input: &mut Input<'_>, frame: &mut Frame, i_mcu: usize) -> Result<(), ContentError> {
        if !self.context {
            for group in 0..8 {
                self.emit_group(input, frame, self.color_space, group)?;
            }
            return Ok(());
        }
        // The next strip is decoded before the previous postponed group. Bottom
        // pointer changes must happen after that postponed group is consumed.
        if i_mcu > 0 {
            self.emit_group(input, frame, self.color_space, 9)?;
        }
        let mut available = 7usize;
        if i_mcu == frame.rows - 1 {
            for index in 0..frame.components.len() {
                let (v, height) = {
                    let component = &frame.components[index];
                    (component.v, component.height)
                };
                let remainder = height % (v * 8);
                let rows_left = if remainder == 0 { v * 8 } else { remainder };
                if index == 0 {
                    available = rows_left.div_ceil(v);
                }
                let last = {
                    let plane = self.plane(input, index)?;
                    self.row_pointer(input, plane, rows_left as i64 - 1, self.which)?
                };
                let plane = &mut self.planes[index];
                for row in 0..v * 2 {
                    plane.pointers[plane.starts[self.which] + rows_left + row] = Some(last);
                }
            }
        }
        for group in 0..available {
            self.emit_group(input, frame, self.color_space, group)?;
        }
        if i_mcu == 0 {
            for index in 0..frame.components.len() {
                let v = frame.components[index].v;
                let plane = &mut self.planes[index];
                for start in plane.starts {
                    for row in 0..v {
                        let upper = plane.pointers[start + v * 9 + row]
                            .ok_or_else(|| input.fail("uninitialized source row pointer"))?;
                        let lower = plane.pointers[start + row]
                            .ok_or_else(|| input.fail("uninitialized source row pointer"))?;
                        plane.pointers[start - v + row] = Some(upper);
                        plane.pointers[start + v * 10 + row] = Some(lower);
                    }
                }
            }
        }
        self.which = if self.which == 0 { 1 } else { 0 };
        Ok(())
    }
}

// Donor arity: mirrors the sequential scan reader's operands one-to-one.
#[allow(clippy::too_many_arguments)]
fn read_scan(
    input: &mut Input<'_>,
    scan_header: &ScanHeader,
    frame: &mut Frame,
    quantizers: &HashMap<u8, [u16; 64]>,
    huffman: &mut HashMap<u8, HuffmanTable>,
    restart_interval: usize,
    buffered: bool,
    color_space: ColorSpace,
) -> Result<(), ContentError> {
    let mut members = Vec::with_capacity(scan_header.members.len());
    for member in &scan_header.members {
        let component = frame
            .components
            .iter()
            .position(|candidate| candidate.id == member.id)
            .ok_or_else(|| input.source_fail(format!("Invalid component ID {} in SOS", member.id)))?;
        members.push((component, member.selector));
    }
    // jdinput.c per_scan_setup limits interleaved MCU membership, not the frame.
    if members.len() > 1
        && members
            .iter()
            .map(|(component, _)| frame.components[*component].h * frame.components[*component].v)
            .sum::<usize>()
            > 10
    {
        return Err(input.source_fail("Sampling factors too large for interleaved scan".to_string()));
    }
    let mut prepared = Vec::with_capacity(members.len());
    for (component, selector) in members {
        let multipliers = if let Some(table) = frame.components[component]
            .progression
            .as_ref()
            .and_then(|storage| storage.table.as_ref())
        {
            table.multipliers
        } else {
            let quantizer = frame.components[component].quantizer;
            let table = quantizers
                .get(&quantizer)
                .ok_or_else(|| input.source_fail(format!("Quantization table 0x{quantizer:02x} was not defined")))?;
            let multipliers = float_multipliers(table);
            if let Some(storage) = frame.components[component].progression.as_mut() {
                storage.table = Some(QuantTable {
                    quantizer: *table,
                    multipliers,
                });
            }
            multipliers
        };
        prepared.push((component, selector, multipliers));
    }
    if scan_header.start != 0 || scan_header.end != 63 || scan_header.approximation != 0 {
        input
            .warnings
            .emit("Invalid SOS parameters for sequential JPEG".to_string());
    }
    let mut scans = Vec::with_capacity(prepared.len());
    for (component, selector, multipliers) in prepared {
        for table_selector in [selector >> 4, 16 + (selector & 15)] {
            if !huffman.contains_key(&table_selector) {
                return Err(input.source_fail(format!("Huffman table 0x{:02x} was not defined", table_selector & 15)));
            }
            derive_huffman(input, huffman, table_selector)?;
        }
        scans.push((
            ScanComponent {
                component,
                prediction_dc: 0,
                multipliers,
            },
            selector,
        ));
    }
    let tables = &*huffman;
    let mut scans = scans
        .into_iter()
        .map(|(scan, selector)| {
            let dc = derived_table(tables, selector >> 4, input)?;
            let ac = derived_table(tables, 16 + (selector & 15), input)?;
            Ok((scan, dc, ac))
        })
        .collect::<Result<Vec<(ScanComponent, &DerivedHuffman, &DerivedHuffman)>, ContentError>>()?;
    let single = if scans.len() == 1 {
        Some(scans[0].0.component)
    } else {
        None
    };
    let (columns, rows) = match single {
        None => (frame.columns, frame.rows),
        Some(component) => (
            frame.components[component].width.div_ceil(8),
            frame.components[component].height.div_ceil(8),
        ),
    };
    let mut entropy = Entropy {
        value: 0,
        remaining: 0,
        printed_end: false,
    };
    let mut blocks = [[0f32; 64]; 10];
    let mut workspace = [0f32; 64];
    let mut strip = if buffered {
        None
    } else {
        Some(OnePassRows::new(frame, color_space))
    };
    let mut restart = 0u8;
    for mcu in 0..columns * rows {
        if restart_interval != 0 && mcu > 0 && mcu % restart_interval == 0 {
            entropy.restart(input, restart)?;
            restart = (restart + 1) & 7;
            for scan in &mut scans {
                scan.0.prediction_dc = 0;
            }
        }
        let column = mcu % columns;
        let row = mcu / columns;
        let mut block_index = 0usize;
        let mut pending = Vec::new();
        for (index, entry) in scans.iter_mut().enumerate() {
            let (scan, dc, ac) = (&mut entry.0, entry.1, entry.2);
            let (h, v) = match single {
                None => (frame.components[scan.component].h, frame.components[scan.component].v),
                Some(_) => (1, 1),
            };
            for y in 0..v {
                for x in 0..h {
                    if block_index >= blocks.len() {
                        return Err(input.fail("JPEG MCU exceeds block storage"));
                    }
                    let block_column = column * h + x;
                    let block_row = row * v + y;
                    decode_block(
                        input,
                        &mut entropy,
                        frame,
                        scan,
                        dc,
                        ac,
                        block_column,
                        block_row,
                        &mut blocks[block_index],
                    )?;
                    block_index += 1;
                    let component = &frame.components[scan.component];
                    if strip.is_some() && block_column * 8 < component.width && block_row * 8 < component.height {
                        pending.push((
                            index,
                            block_column,
                            if single.is_none() { y } else { row % component.v },
                            block_index - 1,
                        ));
                    }
                }
            }
        }
        // jdhuff completes all MCU entropy before jdcoefct performs any IDCT.
        if let Some(strip) = strip.as_mut() {
            for (index, block_column, block_row, slot) in pending {
                let mut block = blocks[slot];
                strip.write(input, frame, index, block_column, block_row, &mut block, &mut workspace)?;
            }
            if column == columns - 1
                && (single.is_none()
                    || single.is_some_and(|component| {
                        row % frame.components[component].v == frame.components[component].v - 1 || row == rows - 1
                    }))
            {
                let i_mcu = single.map_or(row, |component| row / frame.components[component].v);
                strip.finish_row(input, frame, i_mcu)?;
            }
        }
    }
    entropy.align();
    Ok(())
}

// jdphuff.c decode_mcu_AC_first. EOB runs count blocks, including the block
// containing the symbol; restart boundaries reset both this count and DCs.
// Donor arity: mirrors decode_mcu_AC_first's operands one-to-one.
#[allow(clippy::too_many_arguments)]
fn progressive_ac_first(
    input: &mut Input<'_>,
    entropy: &mut Entropy,
    table: &DerivedHuffman,
    coefficients: &mut [i16],
    offset: usize,
    start: u8,
    end: u8,
    low: u8,
    eob_remaining: &mut u32,
) -> Result<(), ContentError> {
    if *eob_remaining > 0 {
        *eob_remaining -= 1;
        return Ok(());
    }
    let mut index = start;
    while index <= end {
        let symbol = entropy.symbol(input, table)?;
        let run = symbol >> 4;
        let size = symbol & 15;
        if size == 0 {
            if run < 15 {
                *eob_remaining = (1u32 << run) + entropy.bits(input, u32::from(run))? - 1;
                return Ok(());
            }
            index = index.saturating_add(16);
        } else {
            index = index.saturating_add(run);
            let natural = NATURAL_ORDER[usize::from(index.min(63))];
            index = index.saturating_add(1);
            coefficients[offset + natural] = (entropy.signed(input, u32::from(size))? << low) as i16;
        }
    }
    Ok(())
}

fn refine_coefficient(
    input: &mut Input<'_>,
    entropy: &mut Entropy,
    coefficients: &mut [i16],
    index: usize,
    bit: i16,
) -> Result<(), ContentError> {
    let value = coefficients
        .get(index)
        .copied()
        .ok_or_else(|| input.fail("progressive coefficient outside storage"))?;
    if entropy.bits(input, 1)? != 0 && (value & bit) == 0 {
        coefficients[index] = value + if value >= 0 { bit } else { -bit };
    }
    Ok(())
}

// jdphuff.c decode_mcu_AC_refine: zero runs skip only zero coefficients.
// Existing nonzero coefficients consume correction bits even during EOB runs.
// Donor arity: mirrors decode_mcu_AC_refine's operands one-to-one.
#[allow(clippy::too_many_arguments)]
fn progressive_ac_refine(
    input: &mut Input<'_>,
    entropy: &mut Entropy,
    table: &DerivedHuffman,
    coefficients: &mut [i16],
    offset: usize,
    start: u8,
    end: u8,
    low: u8,
    eob_remaining: &mut u32,
) -> Result<(), ContentError> {
    let bit = 1i16 << low;
    let mut index = start;
    if *eob_remaining == 0 {
        while index <= end {
            let symbol = entropy.symbol(input, table)?;
            let mut run = i32::from(symbol >> 4);
            let size = symbol & 15;
            let mut value = 0i16;
            if size != 0 {
                if size != 1 {
                    input.warnings.emit("Corrupt JPEG data: bad Huffman code".to_string());
                }
                value = if entropy.bits(input, 1)? == 0 { -bit } else { bit };
            } else if run < 15 {
                *eob_remaining = (1u32 << run) + entropy.bits(input, run as u32)?;
                break;
            }
            while index <= end {
                let position = offset + NATURAL_ORDER[usize::from(index)];
                let current = coefficients
                    .get(position)
                    .copied()
                    .ok_or_else(|| input.fail("progressive coefficient outside storage"))?;
                if current != 0 {
                    refine_coefficient(input, entropy, coefficients, position, bit)?;
                } else {
                    run -= 1;
                    if run < 0 {
                        break;
                    }
                }
                index = index.saturating_add(1);
                if index == 0 {
                    break;
                }
            }
            if value != 0 {
                let position = offset + NATURAL_ORDER[usize::from(index.min(63))];
                *coefficients
                    .get_mut(position)
                    .ok_or_else(|| input.fail("progressive coefficient outside storage"))? = value;
            }
            index = index.saturating_add(1);
            if index == 0 {
                break;
            }
        }
    }
    if *eob_remaining > 0 {
        while index <= end {
            let position = offset + NATURAL_ORDER[usize::from(index)];
            let current = coefficients
                .get(position)
                .copied()
                .ok_or_else(|| input.fail("progressive coefficient outside storage"))?;
            if current != 0 {
                refine_coefficient(input, entropy, coefficients, position, bit)?;
            }
            index = index.saturating_add(1);
            if index == 0 {
                break;
            }
        }
        *eob_remaining -= 1;
    }
    Ok(())
}

fn read_progressive_scan(
    input: &mut Input<'_>,
    scan_header: &ScanHeader,
    frame: &mut Frame,
    quantizers: &HashMap<u8, [u16; 64]>,
    huffman: &mut HashMap<u8, HuffmanTable>,
    restart_interval: usize,
) -> Result<(), ContentError> {
    let mut members = Vec::with_capacity(scan_header.members.len());
    for member in &scan_header.members {
        let component = frame
            .components
            .iter()
            .position(|candidate| candidate.id == member.id)
            .ok_or_else(|| input.source_fail(format!("Invalid component ID {} in SOS", member.id)))?;
        if frame.components[component].progression.is_none() {
            return Err(input.fail("progressive scan without coefficient storage"));
        }
        members.push((component, member.selector));
    }
    let high = scan_header.approximation >> 4;
    let low = scan_header.approximation & 15;
    if members.len() > 1
        && members
            .iter()
            .map(|(component, _)| frame.components[*component].h * frame.components[*component].v)
            .sum::<usize>()
            > 10
    {
        return Err(input.source_fail("Sampling factors too large for interleaved scan".to_string()));
    }
    for (component, _) in &members {
        if frame.components[*component]
            .progression
            .as_ref()
            .is_some_and(|state| state.table.is_some())
        {
            continue;
        }
        let quantizer = frame.components[*component].quantizer;
        let table = quantizers
            .get(&quantizer)
            .ok_or_else(|| input.source_fail(format!("Quantization table 0x{quantizer:02x} was not defined")))?;
        if let Some(storage) = frame.components[*component].progression.as_mut() {
            storage.table = Some(QuantTable {
                quantizer: *table,
                multipliers: float_multipliers(table),
            });
        }
    }
    let start = scan_header.start;
    let end = scan_header.end;
    if (if start == 0 {
        end != 0
    } else {
        start > end || end > 63 || members.len() != 1
    }) || (high != 0 && low != high - 1)
        || low > 13
    {
        return Err(input.source_fail(format!(
            "Invalid progressive parameters Ss={start} Se={end} Ah={high} Al={low}"
        )));
    }
    let mut entropy = Entropy {
        value: 0,
        remaining: 0,
        printed_end: false,
    };
    let mut eob_remaining = 0u32;
    for (component, _) in &members {
        let storage = frame.components[*component]
            .progression
            .as_mut()
            .ok_or_else(|| input.fail("progressive scan without coefficient storage"))?;
        if start > 0 && storage.bits[0] < 0 {
            input.warnings.emit(format!(
                "Inconsistent progression sequence for component {component} coefficient 0"
            ));
        }
        for index in usize::from(start)..=usize::from(end) {
            let previous = storage.bits[index];
            if high != (if previous < 0 { 0 } else { previous as u8 }) {
                input.warnings.emit(format!(
                    "Inconsistent progression sequence for component {component} coefficient {index}"
                ));
            }
            storage.bits[index] = low as i8;
        }
    }
    enum ProgressiveKind<'a> {
        DcRefine,
        DcFirst(&'a DerivedHuffman),
        AcFirst(&'a DerivedHuffman),
        AcRefine(&'a DerivedHuffman),
    }
    for (_, selector) in &members {
        if start == 0 && high != 0 {
            continue;
        }
        derive_huffman(
            input,
            huffman,
            if start == 0 {
                selector >> 4
            } else {
                16 + (selector & 15)
            },
        )?;
    }
    let tables = &*huffman;
    let mut scans = Vec::with_capacity(members.len());
    for (component, selector) in members {
        let kind = if start == 0 && high != 0 {
            ProgressiveKind::DcRefine
        } else {
            let table = derived_table(
                tables,
                if start == 0 {
                    selector >> 4
                } else {
                    16 + (selector & 15)
                },
                input,
            )?;
            if start == 0 {
                ProgressiveKind::DcFirst(table)
            } else if high == 0 {
                ProgressiveKind::AcFirst(table)
            } else {
                ProgressiveKind::AcRefine(table)
            }
        };
        scans.push((component, kind, 0i32));
    }
    let single = if scans.len() == 1 { Some(scans[0].0) } else { None };
    let (columns, rows) = match single {
        None => (frame.columns, frame.rows),
        Some(component) => (
            frame.components[component].width.div_ceil(8),
            frame.components[component].height.div_ceil(8),
        ),
    };
    let mut restart = 0u8;
    for mcu in 0..columns * rows {
        if restart_interval != 0 && mcu > 0 && mcu % restart_interval == 0 {
            entropy.restart(input, restart)?;
            restart = (restart + 1) & 7;
            eob_remaining = 0;
            for scan in &mut scans {
                scan.2 = 0;
            }
        }
        let column = mcu % columns;
        let row = mcu / columns;
        for scan in &mut scans {
            let (h, v) = match single {
                None => (frame.components[scan.0].h, frame.components[scan.0].v),
                Some(_) => (1, 1),
            };
            for y in 0..v {
                for x in 0..h {
                    let stride_blocks = frame.components[scan.0].stride / 8;
                    let offset = ((row * v + y) * stride_blocks + column * h + x) * 64;
                    match &scan.1 {
                        ProgressiveKind::DcRefine => {
                            if entropy.bits(input, 1)? != 0 {
                                let storage = frame.components[scan.0]
                                    .progression
                                    .as_mut()
                                    .ok_or_else(|| input.fail("progressive scan without coefficient storage"))?;
                                let slot = storage
                                    .coefficients
                                    .get_mut(offset)
                                    .ok_or_else(|| input.fail("progressive coefficient outside storage"))?;
                                *slot |= 1i16 << low;
                            }
                        }
                        ProgressiveKind::DcFirst(table) => {
                            let symbol = entropy.symbol(input, table)?;
                            let delta = entropy.signed(input, u32::from(symbol))?;
                            scan.2 = scan.2.wrapping_add(delta);
                            let dc = scan.2 << low;
                            let storage = frame.components[scan.0]
                                .progression
                                .as_mut()
                                .ok_or_else(|| input.fail("progressive scan without coefficient storage"))?;
                            *storage
                                .coefficients
                                .get_mut(offset)
                                .ok_or_else(|| input.fail("progressive coefficient outside storage"))? = dc as i16;
                        }
                        ProgressiveKind::AcFirst(table) => {
                            let storage = frame.components[scan.0]
                                .progression
                                .as_mut()
                                .ok_or_else(|| input.fail("progressive scan without coefficient storage"))?;
                            progressive_ac_first(
                                input,
                                &mut entropy,
                                table,
                                &mut storage.coefficients,
                                offset,
                                start,
                                end,
                                low,
                                &mut eob_remaining,
                            )?;
                        }
                        ProgressiveKind::AcRefine(table) => {
                            let storage = frame.components[scan.0]
                                .progression
                                .as_mut()
                                .ok_or_else(|| input.fail("progressive scan without coefficient storage"))?;
                            progressive_ac_refine(
                                input,
                                &mut entropy,
                                table,
                                &mut storage.coefficients,
                                offset,
                                start,
                                end,
                                low,
                                &mut eob_remaining,
                            )?;
                        }
                    }
                }
            }
        }
    }
    entropy.align();
    Ok(())
}

// jdcoefct.c decompress_smooth_data, applied only to still-zero low-frequency
// AC values whose bits remain unknown. Neighbors repeat at image edges.
fn smooth_progressive_block(
    component: &Component,
    state: &Progression,
    quantizer: &[u16; 64],
    column: usize,
    row: usize,
    coefficients: &mut [i16; 64],
) {
    let columns = component.width.div_ceil(8);
    let rows = component.height.div_ceil(8);
    let dc = |x: i64, y: i64| -> i32 {
        let x = x.clamp(0, columns as i64 - 1) as usize;
        let y = y.clamp(0, rows as i64 - 1) as usize;
        i32::from(state.coefficients[(y * (component.stride / 8) + x) * 64])
    };
    let q00 = i32::from(quantizer[0]);
    let center = dc(column as i64, row as i64);
    let above = dc(column as i64, row as i64 - 1);
    let below = dc(column as i64, row as i64 + 1);
    let left = dc(column as i64 - 1, row as i64);
    let right = dc(column as i64 + 1, row as i64);
    let numerators = [
        36 * q00 * (left - right),
        36 * q00 * (above - below),
        9 * q00 * (above + below - 2 * center),
        5 * q00
            * (dc(column as i64 - 1, row as i64 - 1)
                - dc(column as i64 + 1, row as i64 - 1)
                - dc(column as i64 - 1, row as i64 + 1)
                + dc(column as i64 + 1, row as i64 + 1)),
        9 * q00 * (left + right - 2 * center),
    ];
    for index in 1..=5usize {
        let low = state.bits[index];
        let natural = NATURAL_ORDER[index];
        if low == 0 || coefficients[natural] != 0 {
            continue;
        }
        let quant = i32::from(quantizer[natural]);
        let numerator = numerators[index - 1];
        // Math.trunc of a non-negative quotient is plain integer division.
        let mut prediction = (quant * 128 + numerator.abs()) / (quant * 256);
        if low > 0 {
            prediction = prediction.min((1 << low) - 1);
        }
        coefficients[natural] = if numerator < 0 {
            -(prediction as i16)
        } else {
            prediction as i16
        };
    }
}

fn finish_buffered(input: &mut Input<'_>, frame: &mut Frame) -> Result<(), ContentError> {
    let mut block = [0f32; 64];
    let mut workspace = [0f32; 64];
    let mut coefficients = [0i16; 64];
    // jdcoefct.c smoothing_ok checks the first six zigzag quantizers across
    // every component before enabling its division-based prediction pass.
    let smoothing_safe = frame.progressive
        && frame.components.iter().all(|component| {
            let Some(state) = component.progression.as_ref() else {
                return false;
            };
            let Some(table) = state.table.as_ref() else {
                return false;
            };
            if state.bits[0] < 0 {
                return false;
            }
            NATURAL_ORDER[..6].iter().all(|index| table.quantizer[*index] != 0)
        });
    for index in 0..frame.components.len() {
        let smoothing = {
            let component = &frame.components[index];
            let state = component
                .progression
                .as_ref()
                .ok_or_else(|| input.fail("buffered frame without coefficient storage"))?;
            smoothing_safe && state.bits[1..6].iter().any(|low| *low != 0)
        };
        let rows = frame.components[index].height.div_ceil(8);
        let columns = frame.components[index].width.div_ceil(8);
        for row in 0..rows {
            for column in 0..columns {
                let (multipliers, offset) = {
                    let component = &frame.components[index];
                    let state = component
                        .progression
                        .as_ref()
                        .ok_or_else(|| input.fail("buffered frame without coefficient storage"))?;
                    // jddctmgr.c leaves multiplier tables zero until the component's first
                    // scan latches a quantizer. An omitted component produces neutral samples.
                    let multipliers = state.table.as_ref().map_or([0f32; 64], |table| table.multipliers);
                    (multipliers, (row * (component.stride / 8) + column) * 64)
                };
                {
                    let component = &frame.components[index];
                    let state = component
                        .progression
                        .as_ref()
                        .ok_or_else(|| input.fail("buffered frame without coefficient storage"))?;
                    coefficients.copy_from_slice(&state.coefficients[offset..offset + 64]);
                }
                if smoothing {
                    let component = &frame.components[index];
                    let state = component
                        .progression
                        .as_ref()
                        .ok_or_else(|| input.fail("buffered frame without coefficient storage"))?;
                    let table = state
                        .table
                        .as_ref()
                        .ok_or_else(|| input.fail("buffered frame without coefficient storage"))?;
                    smooth_progressive_block(component, state, &table.quantizer, column, row, &mut coefficients);
                }
                for (slot, (value, multiplier)) in block.iter_mut().zip(coefficients.iter().zip(multipliers.iter())) {
                    *slot = f32::from(*value) * multiplier;
                }
                let component = &mut frame.components[index];
                write_block(input, component, column, row, &mut block, &mut workspace)?;
            }
        }
    }
    Ok(())
}

fn sample(input: &Input<'_>, component: &Component, x: usize, y: usize, frame: &Frame) -> Result<u8, ContentError> {
    if let Some(output) = component.output_samples.as_ref() {
        return output
            .get(y * frame.width + x)
            .copied()
            .ok_or_else(|| input.fail("JPEG sample outside component output"));
    }
    let h = frame.max_h / component.h;
    let v = frame.max_v / component.v;
    let cx = x / h;
    let cy = y / v;
    let center = component
        .pixels
        .get(cy * component.stride + cx)
        .copied()
        .ok_or_else(|| input.fail("JPEG sample outside component plane"))?;
    if h != 2 || component.width <= 2 || (v != 1 && v != 2) {
        return Ok(center);
    }
    let neighbor_x =
        (cx as i64 + if x.is_multiple_of(2) { -1 } else { 1 }).clamp(0, component.width as i64 - 1) as usize;
    let horizontal = component
        .pixels
        .get(cy * component.stride + neighbor_x)
        .copied()
        .ok_or_else(|| input.fail("JPEG sample outside component plane"))?;
    if v == 1 {
        let mixed = u16::from(center) * 3 + u16::from(horizontal) + (x % 2) as u16 + 1;
        return Ok((mixed >> 2) as u8);
    }
    let neighbor_y =
        (cy as i64 + if y.is_multiple_of(2) { -1 } else { 1 }).clamp(0, component.height as i64 - 1) as usize;
    let vertical = component
        .pixels
        .get(neighbor_y * component.stride + cx)
        .copied()
        .ok_or_else(|| input.fail("JPEG sample outside component plane"))?;
    let diagonal = component
        .pixels
        .get(neighbor_y * component.stride + neighbor_x)
        .copied()
        .ok_or_else(|| input.fail("JPEG sample outside component plane"))?;
    Ok(((u16::from(center) * 9
        + u16::from(horizontal) * 3
        + u16::from(vertical) * 3
        + u16::from(diagonal)
        + if x.is_multiple_of(2) { 8 } else { 7 })
        >> 4) as u8)
}

// jdapimin.c default_decompress_parms. Unknown Adobe transforms use the
// component-count-specific YCC fallback; JFIF takes priority for three components.
fn select_color_space(
    frame: &Frame,
    adobe_transform: Option<u8>,
    jfif: bool,
    warnings: &mut Warnings<'_>,
) -> ColorSpace {
    if frame.components.len() == 1 {
        return ColorSpace::Grayscale;
    }
    if frame.components.len() != 3 && frame.components.len() != 4 {
        return ColorSpace::Unknown;
    }
    if frame.components.len() == 4 {
        if adobe_transform.is_some_and(|transform| transform != 0 && transform != 2) {
            warnings.emit(format!(
                "Unknown Adobe color transform code {}",
                adobe_transform.unwrap_or(0)
            ));
        }
        return if adobe_transform.is_none_or(|transform| transform == 0) {
            ColorSpace::Cmyk
        } else {
            ColorSpace::Ycck
        };
    }
    if jfif {
        return ColorSpace::Ycbcr;
    }
    if let Some(transform) = adobe_transform {
        if transform != 0 && transform != 1 {
            warnings.emit(format!("Unknown Adobe color transform code {transform}"));
        }
        return if transform == 0 {
            ColorSpace::Rgb
        } else {
            ColorSpace::Ycbcr
        };
    }
    let red = frame.components.first().map(|component| component.id);
    let green = frame.components.get(1).map(|component| component.id);
    let blue = frame.components.get(2).map(|component| component.id);
    if red == Some(82) && green == Some(71) && blue == Some(66) {
        ColorSpace::Rgb
    } else {
        ColorSpace::Ycbcr
    }
}

fn render(input: &mut Input<'_>, frame: &Frame, color_space: ColorSpace) -> Result<JpegImage, ContentError> {
    if frame.components.len() == 2 {
        return Err(input.fail("two-component LoadJPG alpha writes exceed the source output allocation"));
    }
    let first = frame
        .components
        .first()
        .ok_or_else(|| input.fail("JPEG missing image component"))?;
    let second = frame.components.get(1);
    let third = frame.components.get(2);
    let mut pixels = alloc_bytes(input, frame.width * frame.height * 4, "output")?;
    if color_space == ColorSpace::Unknown {
        // null_convert emits N interleaved channels. LoadJPG overwrites alpha
        // in, and the renderer reads, only the first width * height * 4 bytes.
        for (index, slot) in pixels.iter_mut().enumerate() {
            let pixel = index / frame.components.len();
            let component = frame
                .components
                .get(index % frame.components.len())
                .ok_or_else(|| input.fail("JPEG missing null-conversion component"))?;
            *slot = if index % 4 == 3 {
                255
            } else {
                sample(input, component, pixel % frame.width, pixel / frame.width, frame)?
            };
        }
        return Ok(JpegImage {
            width: frame.width as u32,
            height: frame.height as u32,
            pixels,
        });
    }
    for y in 0..frame.height {
        for x in 0..frame.width {
            let offset = (y * frame.width + x) * 4;
            let a = sample(input, first, x, y, frame)?;
            match (second, third) {
                (Some(second), Some(third)) => {
                    let b = sample(input, second, x, y, frame)?;
                    let c = sample(input, third, x, y, frame)?;
                    if color_space == ColorSpace::Rgb || color_space == ColorSpace::Cmyk {
                        pixels[offset] = a;
                        pixels[offset + 1] = b;
                        pixels[offset + 2] = c;
                    } else {
                        let red = i32::from(a) + descale(91881 * (i32::from(c) - 128), 16);
                        let green =
                            i32::from(a) + descale(-22554 * (i32::from(b) - 128) - 46802 * (i32::from(c) - 128), 16);
                        let blue = i32::from(a) + descale(116130 * (i32::from(b) - 128), 16);
                        pixels[offset] = clamp(if color_space == ColorSpace::Ycck {
                            255 - red
                        } else {
                            red
                        });
                        pixels[offset + 1] = clamp(if color_space == ColorSpace::Ycck {
                            255 - green
                        } else {
                            green
                        });
                        pixels[offset + 2] = clamp(if color_space == ColorSpace::Ycck {
                            255 - blue
                        } else {
                            blue
                        });
                    }
                }
                _ => {
                    pixels[offset] = a;
                    pixels[offset + 1] = a;
                    pixels[offset + 2] = a;
                }
            }
            // LoadJPG overwrites the fourth output byte, including CMYK's decoded K.
            pixels[offset + 3] = 255;
        }
    }
    Ok(JpegImage {
        width: frame.width as u32,
        height: frame.height as u32,
        pixels,
    })
}

enum DecodeState {
    Header(FrameHeader),
    Ready {
        frame: Frame,
        color_space: ColorSpace,
        multiple_scans: bool,
    },
}

/// Decode top-to-bottom opaque renderer pixels; CMYK/YCCK retain Quake's CMY channels.
pub fn decode_jpeg_with_print(
    bytes: &[u8],
    source: &str,
    print: &mut dyn FnMut(&str),
) -> Result<JpegImage, ContentError> {
    let mut input = Input {
        bytes,
        source: source.to_string(),
        offset: 0,
        unread_marker: 0,
        discarded_bytes: 0,
        warnings: Warnings { count: 0, print },
    };
    let first = input.byte()?;
    let second = input.byte()?;
    if first != 255 || second != 216 {
        return Err(input.source_fail(format!("Not a JPEG file: starts with 0x{first:02x} 0x{second:02x}")));
    }
    let mut quantizers: HashMap<u8, [u16; 64]> = HashMap::new();
    let mut huffman: HashMap<u8, HuffmanTable> = HashMap::new();
    let mut state: Option<DecodeState> = None;
    let mut restart_interval = 0usize;
    let mut adobe_transform = None;
    let mut jfif = false;
    loop {
        let marker = input.marker()?;
        if marker == 217 {
            let Some(state) = state else {
                return Err(input.source_fail("JPEG datastream contains no image".to_string()));
            };
            match state {
                DecodeState::Header(_) => {
                    return Err(input.source_fail("Invalid JPEG file structure: missing SOS marker".to_string()));
                }
                DecodeState::Ready {
                    mut frame,
                    color_space,
                    multiple_scans,
                } => {
                    if multiple_scans {
                        finish_buffered(&mut input, &mut frame)?;
                    }
                    return render(&mut input, &frame, color_space);
                }
            }
        }
        if marker == 216 {
            return Err(input.source_fail("Invalid JPEG file structure: two SOI markers".to_string()));
        }
        if marker == 1 || (208..=215).contains(&marker) {
            continue;
        }
        if [195, 197, 198, 199, 200, 203, 205, 206, 207].contains(&marker) {
            return Err(input.source_fail(format!("Unsupported JPEG process: SOF type 0x{marker:02x}")));
        }
        if marker == 218 && state.is_none() {
            return Err(input.source_fail("Invalid JPEG file structure: SOS before SOF".to_string()));
        }
        if ![192, 193, 194, 196, 201, 202, 204, 218, 219, 220, 221, 254].contains(&marker)
            && !(224..=239).contains(&marker)
        {
            return Err(input.source_fail(format!("Unsupported marker type 0x{marker:02x}")));
        }
        if marker == 219 {
            read_quantizers(&mut input, &mut quantizers)?;
        } else if marker == 196 {
            read_huffman(&mut input, &mut huffman)?;
        } else if marker == 204 {
            read_arithmetic_tables(&mut input)?;
        } else if [192, 193, 194, 201, 202].contains(&marker) {
            let header = read_frame(
                &mut input,
                marker == 194 || marker == 202,
                marker == 201 || marker == 202,
                state.is_some(),
            )?;
            state = Some(DecodeState::Header(header));
        } else if marker == 221 {
            if input.word()? != 4 {
                return Err(input.source_fail("Bogus marker length".to_string()));
            }
            restart_interval = usize::from(input.word()?);
        } else if marker == 218 {
            let Some(current) = state else {
                return Err(input.source_fail("Invalid JPEG file structure: SOS before SOF".to_string()));
            };
            let ids = match &current {
                DecodeState::Header(header) => header
                    .components
                    .iter()
                    .map(|component| component.id)
                    .collect::<Vec<_>>(),
                DecodeState::Ready { frame, .. } => frame
                    .components
                    .iter()
                    .map(|component| component.id)
                    .collect::<Vec<_>>(),
            };
            let scan_header = read_scan_header(&mut input, &ids)?;
            state = Some(current);
            if matches!(
                &state,
                Some(DecodeState::Ready {
                    multiple_scans: false,
                    ..
                })
            ) {
                return Err(input.source_fail("Didn't expect more than one scan".to_string()));
            }
            if matches!(&state, Some(DecodeState::Header(_))) {
                let Some(DecodeState::Header(header)) = state else {
                    return Err(input.fail("JPEG missing frame header"));
                };
                let multiple_scans = header.progressive || scan_header.members.len() < header.components.len();
                let frame = prepare_frame(&mut input, &header, multiple_scans)?;
                let color_space = select_color_space(&frame, adobe_transform, jfif, &mut input.warnings);
                for component in &frame.components {
                    if frame.max_h % component.h != 0 || frame.max_v % component.v != 0 {
                        return Err(input.source_fail("Fractional sampling not implemented yet".to_string()));
                    }
                }
                if header.arithmetic {
                    return Err(
                        input.source_fail("Sorry, there are legal restrictions on arithmetic coding".to_string())
                    );
                }
                state = Some(DecodeState::Ready {
                    frame,
                    color_space,
                    multiple_scans,
                });
            }
            let Some(DecodeState::Ready {
                frame,
                color_space,
                multiple_scans,
            }) = state.as_mut()
            else {
                return Err(input.fail("JPEG missing decoded frame"));
            };
            if frame.progressive {
                read_progressive_scan(
                    &mut input,
                    &scan_header,
                    frame,
                    &quantizers,
                    &mut huffman,
                    restart_interval,
                )?;
            } else {
                let color_space = *color_space;
                let multiple_scans = *multiple_scans;
                read_scan(
                    &mut input,
                    &scan_header,
                    frame,
                    &quantizers,
                    &mut huffman,
                    restart_interval,
                    multiple_scans,
                    color_space,
                )?;
            }
        } else if marker == 224 {
            if read_jfif(&mut input)? {
                jfif = true;
            }
        } else if marker == 238 {
            if let Some(transform) = read_adobe(&mut input)? {
                adobe_transform = Some(transform);
            }
        } else {
            // skip_variable handles COM, other APP markers and DNL (which never
            // replaces SOF dimensions). Negative skip lengths consume nothing.
            let length = input.word()?;
            input.skip(usize::from(length).saturating_sub(2))?;
        }
    }
}

/// Decode top-to-bottom opaque renderer pixels, discarding IJG warnings.
pub fn decode_jpeg(bytes: &[u8], source: &str) -> Result<JpegImage, ContentError> {
    decode_jpeg_with_print(bytes, source, &mut |_| {})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![
            0xff,
            marker,
            ((payload.len() + 2) >> 8) as u8,
            ((payload.len() + 2) & 255) as u8,
        ];
        bytes.extend_from_slice(payload);
        bytes
    }

    fn dht_table(selector: u8, counts: &[u8; 16], values: &[u8]) -> Vec<u8> {
        let mut payload = vec![selector];
        payload.extend_from_slice(counts);
        payload.extend_from_slice(values);
        segment(0xc4, &payload)
    }

    /// Minimal 1x1 grayscale baseline JPEG: all-ones quantizer, DC category 0
    /// (`00`), AC EOB (`0000`), zero padding, then EOI.
    fn minimal_gray() -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8];
        let mut quantizer = vec![0u8];
        quantizer.extend_from_slice(&[1u8; 64]);
        bytes.extend(segment(0xdb, &quantizer));
        bytes.extend(segment(0xc0, &[8, 0, 1, 0, 1, 1, 1, 0x11, 0]));
        bytes.extend(dht_table(0, &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0]));
        bytes.extend(dht_table(0x10, &[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0]));
        bytes.extend(segment(0xda, &[1, 1, 0, 0, 63, 0]));
        bytes.push(0x00);
        bytes.extend_from_slice(&[0xff, 0xd9]);
        bytes
    }

    #[test]
    fn rejects_non_jpeg_magic() {
        let error = decode_jpeg(&[0, 1, 2, 3], "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: Not a JPEG file: starts with 0x00 0x01");
        assert_eq!(error.offset, 2);
    }

    #[test]
    fn rejects_structural_errors() {
        let error = decode_jpeg(&[0xff, 0xd8, 0xff, 0xd9], "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: JPEG datastream contains no image");

        let mut missing_sos = vec![0xff, 0xd8];
        missing_sos.extend(segment(0xc0, &[8, 0, 1, 0, 1, 1, 1, 0x11, 0]));
        missing_sos.extend_from_slice(&[0xff, 0xd9]);
        let error = decode_jpeg(&missing_sos, "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: Invalid JPEG file structure: missing SOS marker");

        let mut sos_first = vec![0xff, 0xd8];
        sos_first.extend(segment(0xda, &[1, 1, 0, 0, 63, 0]));
        let error = decode_jpeg(&sos_first, "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: Invalid JPEG file structure: SOS before SOF");

        let error = decode_jpeg(&[0xff, 0xd8, 0xff, 0xd8], "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: Invalid JPEG file structure: two SOI markers");

        let error = decode_jpeg(&[0xff, 0xd8, 0xff, 0xc3], "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: Unsupported JPEG process: SOF type 0xc3");

        let error = decode_jpeg(&[0xff, 0xd8, 0xff, 0x02], "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: Unsupported marker type 0x02");

        let mut empty = vec![0xff, 0xd8];
        empty.extend(segment(0xc0, &[8, 0, 0, 0, 1, 1, 1, 0x11, 0]));
        empty.extend_from_slice(&[0xff, 0xd9]);
        let error = decode_jpeg(&empty, "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: Empty JPEG image (DNL not supported)");
    }

    #[test]
    fn rejects_bogus_tables() {
        let mut bad_dqt = vec![0xff, 0xd8];
        bad_dqt.extend(segment(0xdb, &[0x04, 0x00]));
        let error = decode_jpeg(&bad_dqt, "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: Bogus DQT index 4");

        let mut bad_dht = vec![0xff, 0xd8];
        bad_dht.extend(segment(0xc4, &[0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        let error = decode_jpeg(&bad_dht, "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: Bogus DHT counts");

        let mut bad_dac = vec![0xff, 0xd8];
        bad_dac.extend(segment(0xcc, &[32, 0]));
        let error = decode_jpeg(&bad_dac, "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: Bogus DAC index 32");

        let mut bad_value = vec![0xff, 0xd8];
        bad_value.extend(segment(0xcc, &[0, 1]));
        let error = decode_jpeg(&bad_value, "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: Bogus DAC value 0x01");

        let error = decode_jpeg(&[0xff, 0xd8, 0xff, 0xdd, 0, 3], "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: Bogus marker length");
    }

    #[test]
    fn warns_about_extraneous_bytes() {
        let mut warnings = Vec::new();
        let error = decode_jpeg_with_print(&[0xff, 0xd8, 0, 1, 0xff, 0xd9], "<test>", &mut |text| {
            warnings.push(text.to_string());
        })
        .unwrap_err();
        assert_eq!(
            warnings,
            vec!["Corrupt JPEG data: 2 extraneous bytes before marker 0xd9"]
        );
        assert_eq!(error.message, "JPEG: JPEG datastream contains no image");
    }

    #[test]
    fn decodes_minimal_gray_block() {
        let image = decode_jpeg(&minimal_gray(), "<test>").unwrap();
        // A zero block inverse-transforms to mid-gray level samples.
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!(image.pixels, vec![128, 128, 128, 255]);
    }

    #[test]
    fn rejects_oversubscribed_huffman() {
        let mut bytes = vec![0xff, 0xd8];
        let mut quantizer = vec![0u8];
        quantizer.extend_from_slice(&[1u8; 64]);
        bytes.extend(segment(0xdb, &quantizer));
        bytes.extend(segment(0xc0, &[8, 0, 1, 0, 1, 1, 1, 0x11, 0]));
        bytes.extend(dht_table(
            0,
            &[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            &[0, 1, 2],
        ));
        bytes.extend(dht_table(0x10, &[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0]));
        bytes.extend(segment(0xda, &[1, 1, 0, 0, 63, 0]));
        bytes.extend_from_slice(&[0, 0xff, 0xd9]);
        let error = decode_jpeg(&bytes, "<test>").unwrap_err();
        assert_eq!(
            error.message,
            "JPEG: oversubscribed Huffman table exceeds source lookahead allocation"
        );
    }

    #[test]
    fn rejects_fractional_sampling() {
        let mut bytes = vec![0xff, 0xd8];
        bytes.extend(segment(0xc0, &[8, 0, 8, 0, 8, 2, 1, 0x31, 0, 2, 0x21, 0]));
        bytes.extend(segment(0xda, &[2, 1, 0, 2, 0, 0, 63, 0]));
        bytes.extend_from_slice(&[0xff, 0xd9]);
        let error = decode_jpeg(&bytes, "<test>").unwrap_err();
        assert_eq!(error.message, "JPEG: Fractional sampling not implemented yet");
    }

    #[test]
    fn rejects_two_component_output() {
        let mut bytes = vec![0xff, 0xd8];
        let mut quantizer = vec![0u8];
        quantizer.extend_from_slice(&[1u8; 64]);
        bytes.extend(segment(0xdb, &quantizer));
        bytes.extend(segment(0xc0, &[8, 0, 1, 0, 1, 2, 1, 0x11, 0, 2, 0x11, 0]));
        bytes.extend(dht_table(0, &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0]));
        bytes.extend(dht_table(0x10, &[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &[0]));
        bytes.extend(segment(0xda, &[2, 1, 0, 2, 0, 0, 63, 0]));
        bytes.extend_from_slice(&[0, 0, 0xff, 0xd9]);
        let error = decode_jpeg(&bytes, "<test>").unwrap_err();
        assert_eq!(
            error.message,
            "JPEG: two-component LoadJPG alpha writes exceed the source output allocation"
        );
    }

    #[test]
    fn source_error_converts() {
        let error = JpegSourceError {
            input: "<test>".to_string(),
            offset: 3,
            source_message: "Bogus marker length".to_string(),
        };
        assert_eq!(format!("{error}"), "<test>:3: JPEG: Bogus marker length");
        let converted = ContentError::from(error);
        assert_eq!(converted.message, "JPEG: Bogus marker length");
        assert_eq!(converted.offset, 3);
    }

    #[test]
    fn round_trips_encoder_output() {
        let mut pixels = Vec::with_capacity(16 * 16 * 4);
        for y in 0..16 {
            for x in 0..16 {
                pixels.extend_from_slice(&[200, (x * 16) as u8, (y * 16) as u8, 255]);
            }
        }
        let input = JpegImage {
            width: 16,
            height: 16,
            pixels,
        };
        let encoded = super::super::jpeg_encoder::encode_jpeg(&input, 95);
        let decoded = decode_jpeg(&encoded, "<test>").unwrap();
        assert_eq!((decoded.width, decoded.height), (16, 16));
        assert_eq!(decoded.pixels.len(), 16 * 16 * 4);
        for pixel in decoded.pixels.as_chunks::<4>().0 {
            assert_eq!(pixel[3], 255);
        }
        // Lossy 4:2:0 output keeps the red slope's average polarity.
        let left: u32 = decoded
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .take(16 * 8)
            .map(|pixel| u32::from(pixel[0]))
            .sum();
        let right: u32 = decoded
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .skip(16 * 8)
            .map(|pixel| u32::from(pixel[0]))
            .sum();
        assert!(left > 0 && right > 0, "{left} {right}");
    }

    fn fnv1a64(bytes: &[u8]) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }

    // Exact bytes of qfiles/q3a/lrctf/pak01/textures/alliance/curved_wall4b.jpg,
    // a 64x64 baseline RGB texture, embedded so the regression test is hermetic.
    const CORPUS_BASELINE_JPEG: &[u8] = &[
        255, 216, 255, 224, 0, 16, 74, 70, 73, 70, 0, 1, 1, 1, 1, 44, 1, 44, 0, 0, 255, 219, 0, 67, 0, 5, 3, 4, 4, 4,
        3, 5, 4, 4, 4, 5, 5, 5, 6, 7, 12, 8, 7, 7, 7, 7, 15, 11, 11, 9, 12, 17, 15, 18, 18, 17, 15, 17, 17, 19, 22, 28,
        23, 19, 20, 26, 21, 17, 17, 24, 33, 24, 26, 29, 29, 31, 31, 31, 19, 23, 34, 36, 34, 30, 36, 28, 30, 31, 30,
        255, 219, 0, 67, 1, 5, 5, 5, 7, 6, 7, 14, 8, 8, 14, 30, 20, 17, 20, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
        30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
        30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 255, 192, 0, 17, 8, 0, 64, 0, 64, 3, 1, 34, 0, 2, 17, 1, 3, 17, 1,
        255, 196, 0, 31, 0, 0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 255,
        196, 0, 181, 16, 0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 125, 1, 2, 3, 0, 4, 17, 5, 18, 33, 49, 65, 6, 19,
        81, 97, 7, 34, 113, 20, 50, 129, 145, 161, 8, 35, 66, 177, 193, 21, 82, 209, 240, 36, 51, 98, 114, 130, 9, 10,
        22, 23, 24, 25, 26, 37, 38, 39, 40, 41, 42, 52, 53, 54, 55, 56, 57, 58, 67, 68, 69, 70, 71, 72, 73, 74, 83, 84,
        85, 86, 87, 88, 89, 90, 99, 100, 101, 102, 103, 104, 105, 106, 115, 116, 117, 118, 119, 120, 121, 122, 131,
        132, 133, 134, 135, 136, 137, 138, 146, 147, 148, 149, 150, 151, 152, 153, 154, 162, 163, 164, 165, 166, 167,
        168, 169, 170, 178, 179, 180, 181, 182, 183, 184, 185, 186, 194, 195, 196, 197, 198, 199, 200, 201, 202, 210,
        211, 212, 213, 214, 215, 216, 217, 218, 225, 226, 227, 228, 229, 230, 231, 232, 233, 234, 241, 242, 243, 244,
        245, 246, 247, 248, 249, 250, 255, 196, 0, 31, 1, 0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 1, 2, 3,
        4, 5, 6, 7, 8, 9, 10, 11, 255, 196, 0, 181, 17, 0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 119, 0, 1, 2, 3,
        17, 4, 5, 33, 49, 6, 18, 65, 81, 7, 97, 113, 19, 34, 50, 129, 8, 20, 66, 145, 161, 177, 193, 9, 35, 51, 82,
        240, 21, 98, 114, 209, 10, 22, 36, 52, 225, 37, 241, 23, 24, 25, 26, 38, 39, 40, 41, 42, 53, 54, 55, 56, 57,
        58, 67, 68, 69, 70, 71, 72, 73, 74, 83, 84, 85, 86, 87, 88, 89, 90, 99, 100, 101, 102, 103, 104, 105, 106, 115,
        116, 117, 118, 119, 120, 121, 122, 130, 131, 132, 133, 134, 135, 136, 137, 138, 146, 147, 148, 149, 150, 151,
        152, 153, 154, 162, 163, 164, 165, 166, 167, 168, 169, 170, 178, 179, 180, 181, 182, 183, 184, 185, 186, 194,
        195, 196, 197, 198, 199, 200, 201, 202, 210, 211, 212, 213, 214, 215, 216, 217, 218, 226, 227, 228, 229, 230,
        231, 232, 233, 234, 242, 243, 244, 245, 246, 247, 248, 249, 250, 255, 218, 0, 12, 3, 1, 0, 2, 17, 3, 17, 0, 63,
        0, 240, 5, 219, 252, 52, 39, 247, 169, 105, 19, 239, 110, 175, 165, 62, 36, 25, 191, 139, 248, 104, 255, 0,
        102, 143, 253, 10, 157, 64, 9, 72, 244, 124, 180, 127, 227, 212, 0, 234, 74, 41, 118, 208, 0, 255, 0, 247, 213,
        20, 127, 192, 233, 187, 150, 128, 22, 150, 146, 145, 40, 0, 255, 0, 128, 127, 192, 105, 105, 63, 130, 150, 128,
        19, 238, 252, 180, 109, 249, 232, 255, 0, 106, 150, 128, 10, 93, 173, 73, 73, 242, 208, 3, 183, 82, 81, 69, 0,
        38, 218, 117, 55, 115, 82, 208, 1, 69, 39, 251, 84, 110, 160, 5, 164, 95, 247, 254, 106, 62, 90, 63, 221, 160,
        5, 164, 79, 191, 66, 238, 223, 254, 237, 31, 55, 251, 171, 64, 7, 240, 82, 210, 125, 239, 248, 13, 59, 119,
        201, 64, 13, 127, 239, 83, 159, 109, 53, 182, 255, 0, 189, 67, 127, 13, 0, 127, 255, 217,
    ];

    // 32x32 RGB gradient saved progressive (quality 85); the Steel corpus has
    // no progressive JPEG, so this fixture covers the progressive scan path.
    const PROGRESSIVE_JPEG: &[u8] = &[
        255, 216, 255, 224, 0, 16, 74, 70, 73, 70, 0, 1, 1, 0, 0, 1, 0, 1, 0, 0, 255, 219, 0, 67, 0, 5, 3, 4, 4, 4, 3,
        5, 4, 4, 4, 5, 5, 5, 6, 7, 12, 8, 7, 7, 7, 7, 15, 11, 11, 9, 12, 17, 15, 18, 18, 17, 15, 17, 17, 19, 22, 28,
        23, 19, 20, 26, 21, 17, 17, 24, 33, 24, 26, 29, 29, 31, 31, 31, 19, 23, 34, 36, 34, 30, 36, 28, 30, 31, 30,
        255, 219, 0, 67, 1, 5, 5, 5, 7, 6, 7, 14, 8, 8, 14, 30, 20, 17, 20, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
        30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
        30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 255, 194, 0, 17, 8, 0, 32, 0, 32, 3, 1, 34, 0, 2, 17, 1, 3, 17, 1,
        255, 196, 0, 22, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 5, 4, 7, 255, 196, 0, 24, 1, 0, 3, 1, 1, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 6, 5, 7, 255, 218, 0, 12, 3, 1, 0, 2, 16, 3, 16, 0, 0, 1, 203, 210, 189,
        32, 129, 43, 210, 98, 188, 164, 175, 75, 59, 144, 192, 149, 233, 51, 95, 255, 196, 0, 21, 16, 1, 1, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 255, 218, 0, 8, 1, 1, 0, 1, 5, 2, 50, 140, 99, 40, 202, 49, 140, 163, 24,
        202, 50, 140, 99, 40, 198, 50, 140, 163, 24, 202, 255, 196, 0, 21, 17, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 4, 0, 255, 218, 0, 8, 1, 3, 1, 1, 63, 1, 19, 97, 54, 19, 97, 54, 255, 196, 0, 21, 17, 1, 1, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 255, 218, 0, 8, 1, 2, 1, 1, 63, 1, 42, 42, 42, 42, 255, 196, 0, 20, 16,
        1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64, 255, 218, 0, 8, 1, 1, 0, 6, 63, 2, 7, 255, 196, 0, 21, 16,
        1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 97, 255, 218, 0, 8, 1, 1, 0, 1, 63, 33, 138, 8, 162, 130,
        40, 34, 138, 8, 160, 138, 40, 34, 255, 218, 0, 12, 3, 1, 0, 2, 0, 3, 0, 0, 0, 16, 3, 224, 188, 255, 196, 0, 22,
        17, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 33, 49, 255, 218, 0, 8, 1, 3, 1, 1, 63, 16, 155, 38,
        201, 178, 108, 255, 196, 0, 20, 17, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 32, 255, 218, 0, 8, 1, 2,
        1, 1, 63, 16, 31, 255, 0, 255, 196, 0, 21, 16, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 49, 255, 218,
        0, 8, 1, 1, 0, 1, 63, 16, 153, 34, 100, 217, 19, 36, 76, 153, 34, 100, 137, 179, 36, 76, 255, 217,
    ];

    #[test]
    fn decodes_corpus_baseline_pixel_identical() {
        let image = decode_jpeg(CORPUS_BASELINE_JPEG, "<test>").unwrap();
        assert_eq!((image.width, image.height), (64, 64));
        assert_eq!(image.pixels.len(), 64 * 64 * 4);
        assert_eq!(fnv1a64(&image.pixels), 13385976686403150900);
    }

    #[test]
    fn decodes_progressive_pixel_identical() {
        let image = decode_jpeg(PROGRESSIVE_JPEG, "<test>").unwrap();
        assert_eq!((image.width, image.height), (32, 32));
        assert_eq!(image.pixels.len(), 32 * 32 * 4);
        assert_eq!(fnv1a64(&image.pixels), 8454909788123546790);
    }
}
