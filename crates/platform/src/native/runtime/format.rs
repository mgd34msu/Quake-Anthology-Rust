//! Checked printf conversion in the owned child. All native strings and va_list
//! slots are scalar copies; no mapped-byte borrow survives a write or IPC call.
use super::{Cursor, Memory};
use crate::native::{NativeAbi, NativeError};
use qa_core::text::FixedText;
use std::fmt::Write;

pub(super) const MAX_OUTPUT: usize = 1 << 20;
const MAX_FORMAT: usize = 8192;
const MAX_PRECISION: usize = 512;

#[derive(Clone, Copy)]
pub(crate) struct Capture {
    pub stack: u64,
    pub floats: u8,
}

enum Source {
    Registers {
        words: [u64; 13],
        floats: [u64; 8],
        gp: usize,
        fp: usize,
        end_fp: usize,
        overflow: u64,
    },
    SystemVList {
        address: u64,
        gp: u32,
        fp: u32,
        registers: u64,
        overflow: u64,
    },
    MicrosoftList(u64),
}
pub(super) struct Arguments<'a> {
    memory: &'a Memory,
    abi: NativeAbi,
    source: Source,
}
impl<'a> Arguments<'a> {
    pub fn direct(
        memory: &'a Memory,
        abi: NativeAbi,
        fixed: usize,
        words: [u64; 13],
        floats: [u64; 8],
        capture: Capture,
    ) -> Result<Self, NativeError> {
        if (abi == NativeAbi::SystemV && capture.floats > 8) || fixed > 6 {
            return Err(NativeError::Extent);
        }
        let overflow = capture
            .stack
            .checked_add(if abi == NativeAbi::SystemV { 8 } else { 40 })
            .ok_or(NativeError::Extent)?;
        Ok(Self {
            memory,
            abi,
            source: Source::Registers {
                words,
                floats,
                gp: fixed,
                fp: 0,
                end_fp: usize::from(capture.floats),
                overflow,
            },
        })
    }
    pub fn list(memory: &'a Memory, abi: NativeAbi, address: u64) -> Result<Self, NativeError> {
        let source = if abi == NativeAbi::SystemV {
            let gp = memory.unsigned(address, 4)? as u32;
            let fp = memory.unsigned(address.checked_add(4).ok_or(NativeError::Extent)?, 4)? as u32;
            if gp > 48 || gp % 8 != 0 || !(48..=176).contains(&fp) || (fp - 48) % 16 != 0 {
                return Err(NativeError::Extent);
            }
            let overflow =
                memory.unsigned(address.checked_add(8).ok_or(NativeError::Extent)?, 8)?;
            let registers =
                memory.unsigned(address.checked_add(16).ok_or(NativeError::Extent)?, 8)?;
            Source::SystemVList {
                address,
                gp,
                fp,
                registers,
                overflow,
            }
        } else {
            Source::MicrosoftList(address)
        };
        Ok(Self {
            memory,
            abi,
            source,
        })
    }
    fn next(&mut self, floating: bool) -> Result<u64, NativeError> {
        let memory = self.memory;
        let stack_word = |at: &mut u64| {
            let word = memory.unsigned(*at, 8)?;
            *at = at.checked_add(8).ok_or(NativeError::Extent)?;
            Ok(word)
        };
        match &mut self.source {
            Source::Registers {
                words,
                floats,
                gp,
                fp,
                end_fp,
                overflow,
            } => {
                if self.abi == NativeAbi::SystemV && floating {
                    if *fp < *end_fp {
                        let value = floats[*fp];
                        *fp += 1;
                        return Ok(value);
                    }
                    return stack_word(overflow);
                }
                let limit = if self.abi == NativeAbi::SystemV { 6 } else { 4 };
                if *gp < limit {
                    let value = words[*gp];
                    *gp += 1;
                    Ok(value)
                } else {
                    // Win64 consumes one positional slot for integers and
                    // promoted doubles; register doubles are mirrored in GP.
                    stack_word(overflow)
                }
            }
            Source::SystemVList {
                address,
                gp,
                fp,
                registers,
                overflow,
            } => {
                let (offset, stride, limit, field) = if floating {
                    (fp, 16, 176, 4)
                } else {
                    (gp, 8, 48, 0)
                };
                if *offset < limit {
                    let value = memory.unsigned(
                        registers
                            .checked_add(u64::from(*offset))
                            .ok_or(NativeError::Extent)?,
                        8,
                    )?;
                    *offset += stride;
                    memory.put(
                        address.checked_add(field).ok_or(NativeError::Extent)?,
                        4,
                        u64::from(*offset),
                    )?;
                    Ok(value)
                } else {
                    let value = stack_word(overflow)?;
                    memory.put(
                        address.checked_add(8).ok_or(NativeError::Extent)?,
                        8,
                        *overflow,
                    )?;
                    Ok(value)
                }
            }
            Source::MicrosoftList(at) => stack_word(at),
        }
    }
    fn integer(&mut self) -> Result<u64, NativeError> {
        self.next(false)
    }
    fn float(&mut self) -> Result<f64, NativeError> {
        self.next(true).map(f64::from_bits)
    }
}

struct Input<'a> {
    cursor: Cursor<'a>,
    read: usize,
}
impl Input<'_> {
    fn next(&mut self) -> Result<u8, NativeError> {
        self.read += 1;
        if self.read > MAX_FORMAT {
            return Err(NativeError::Extent);
        }
        self.cursor.read()
    }
}
struct Output<'a> {
    cursor: Option<Cursor<'a>>,
    capacity: usize,
    count: usize,
    truncate: bool,
}
impl Output<'_> {
    fn byte(&mut self, byte: u8) -> Result<(), NativeError> {
        if self.count == MAX_OUTPUT {
            return Err(NativeError::Extent);
        }
        if self.count < self.capacity.saturating_sub(1) {
            self.cursor
                .as_mut()
                .ok_or(NativeError::Extent)?
                .write(byte)?;
        } else if !self.truncate {
            return Err(NativeError::Extent);
        }
        self.count += 1;
        Ok(())
    }
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), NativeError> {
        for &byte in bytes {
            self.byte(byte)?;
        }
        Ok(())
    }
    fn repeat(&mut self, byte: u8, count: usize) -> Result<(), NativeError> {
        for _ in 0..count {
            self.byte(byte)?;
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<usize, NativeError> {
        if self.capacity != 0 {
            self.cursor.as_mut().ok_or(NativeError::Extent)?.write(0)?;
        }
        Ok(self.count)
    }
}
#[derive(Default)]
struct Flags {
    left: bool,
    plus: bool,
    space: bool,
    alternate: bool,
    zero: bool,
}
#[derive(Clone, Copy, Default)]
enum Length {
    #[default]
    Int,
    Char,
    Short,
    Long,
    LongLong,
    Pointer,
}
impl Length {
    fn bytes(self, abi: NativeAbi) -> usize {
        match self {
            Self::Char => 1,
            Self::Short => 2,
            Self::Int => 4,
            Self::Long if abi == NativeAbi::Microsoft => 4,
            _ => 8,
        }
    }
}
fn decimal(input: &mut Input<'_>, mut byte: u8) -> Result<(usize, u8), NativeError> {
    let mut value = 0usize;
    while byte.is_ascii_digit() {
        value = value
            .checked_mul(10)
            .and_then(|v| v.checked_add(usize::from(byte - b'0')))
            .filter(|&v| v <= MAX_OUTPUT)
            .ok_or(NativeError::Extent)?;
        byte = input.next()?;
    }
    Ok((value, byte))
}
fn field(
    output: &mut Output<'_>,
    prefix: &[u8],
    digits: &[u8],
    width: usize,
    zeroes: usize,
    flags: &Flags,
) -> Result<(), NativeError> {
    let count = prefix
        .len()
        .checked_add(zeroes)
        .and_then(|n| n.checked_add(digits.len()))
        .ok_or(NativeError::Extent)?;
    let padding = width.saturating_sub(count);
    if !flags.left && !flags.zero {
        output.repeat(b' ', padding)?;
    }
    output.bytes(prefix)?;
    if !flags.left && flags.zero {
        output.repeat(b'0', padding)?;
    }
    output.repeat(b'0', zeroes)?;
    output.bytes(digits)?;
    if flags.left {
        output.repeat(b' ', padding)?;
    }
    Ok(())
}
fn sign(value: f64, flags: &Flags) -> &'static [u8] {
    if value.is_sign_negative() {
        b"-"
    } else if flags.plus {
        b"+"
    } else if flags.space {
        b" "
    } else {
        b""
    }
}
fn exponent(bytes: &[u8]) -> Result<(usize, i32), NativeError> {
    let at = bytes
        .iter()
        .position(|&b| b == b'e')
        .ok_or(NativeError::Extent)?;
    let text = std::str::from_utf8(&bytes[at + 1..]).map_err(|_| NativeError::Extent)?;
    Ok((at, text.parse().map_err(|_| NativeError::Extent)?))
}
fn trim_fraction(text: &mut FixedText<1024>, before: usize) -> Result<(), NativeError> {
    let bytes = text.as_bytes();
    if !bytes[..before].contains(&b'.') {
        return Ok(());
    }
    let mut end = before;
    while end != 0 && bytes[end - 1] == b'0' {
        end -= 1;
    }
    if end != 0 && bytes[end - 1] == b'.' {
        end -= 1;
    }
    text.replace(end..before, "")
        .map_err(|_| NativeError::Extent)
}
fn float_text(
    value: f64,
    conversion: u8,
    precision: Option<usize>,
    alternate: bool,
) -> Result<FixedText<1024>, NativeError> {
    let mut text = FixedText::<1024>::default();
    let lower = conversion.to_ascii_lowercase();
    let explicit_precision = precision.is_some();
    let precision = precision.unwrap_or(if lower == b'a' { 13 } else { 6 });
    if precision > MAX_PRECISION {
        return Err(NativeError::Extent);
    }
    if value.is_nan() || value.is_infinite() {
        text.set(if value.is_nan() { "nan" } else { "inf" })
            .map_err(|_| NativeError::Extent)?;
    } else if lower == b'a' {
        hex_float(&mut text, value, precision, alternate)?;
        if !explicit_precision {
            let at = text
                .as_bytes()
                .iter()
                .position(|&b| b == b'p')
                .ok_or(NativeError::Extent)?;
            trim_fraction(&mut text, at)?;
            if alternate {
                let at = text
                    .as_bytes()
                    .iter()
                    .position(|&b| b == b'p')
                    .ok_or(NativeError::Extent)?;
                if !text.as_bytes()[..at].contains(&b'.') {
                    text.replace(at..at, ".").map_err(|_| NativeError::Extent)?;
                }
            }
        }
    } else if lower == b'f' {
        write!(text, "{:.*}", precision, value).map_err(|_| NativeError::Extent)?;
        if alternate && precision == 0 {
            text.append_bytes(b".").map_err(|_| NativeError::Extent)?;
        }
    } else {
        let significant = precision.max(1);
        let decimals = if lower == b'g' {
            significant - 1
        } else {
            precision
        };
        write!(text, "{:.*e}", decimals, value).map_err(|_| NativeError::Extent)?;
        let (at, power) = exponent(text.as_bytes())?;
        if lower == b'g' && power >= -4 && power < significant as i32 {
            text.clear();
            let digits = (significant as i32 - power - 1).max(0) as usize;
            write!(text, "{:.*}", digits, value).map_err(|_| NativeError::Extent)?;
            if alternate && digits == 0 {
                text.append_bytes(b".").map_err(|_| NativeError::Extent)?;
            }
            if !alternate {
                let length = text.as_bytes().len();
                trim_fraction(&mut text, length)?;
            }
        } else {
            if !alternate && lower == b'g' {
                trim_fraction(&mut text, at)?;
            }
            let (at, _) = exponent(text.as_bytes())?;
            if alternate && decimals == 0 {
                text.replace(at..at, ".").map_err(|_| NativeError::Extent)?;
            }
            let (at, _) = exponent(text.as_bytes())?;
            text.replace(at..text.as_bytes().len(), "")
                .map_err(|_| NativeError::Extent)?;
            write!(text, "e{}{power:02}", if power >= 0 { "+" } else { "" })
                .map_err(|_| NativeError::Extent)?;
            // The minimum two digits exclude the exponent sign.
            if power < 0 && power > -10 {
                let e = text
                    .as_bytes()
                    .iter()
                    .position(|&b| b == b'e')
                    .ok_or(NativeError::Extent)?;
                text.replace(e + 2..e + 2, "0")
                    .map_err(|_| NativeError::Extent)?;
            }
        }
    }
    if conversion.is_ascii_uppercase() {
        let mut upper = FixedText::<1024>::default();
        for &b in text.as_bytes() {
            upper
                .append_bytes(&[b.to_ascii_uppercase()])
                .map_err(|_| NativeError::Extent)?;
        }
        text = upper;
    }
    Ok(text)
}
fn hex_float(
    text: &mut FixedText<1024>,
    value: f64,
    precision: usize,
    alternate: bool,
) -> Result<(), NativeError> {
    let bits = value.to_bits();
    let raw_exponent = ((bits >> 52) & 2047) as i32;
    let exponent = if value == 0.0 {
        0
    } else {
        raw_exponent.max(1) - 1023
    };
    let mut leading = u8::from(raw_exponent != 0);
    let mut fraction = bits & ((1u64 << 52) - 1);
    if precision < 13 {
        let shift = (13 - precision) * 4;
        let mask = (1u64 << shift) - 1;
        let discarded = fraction & mask;
        let kept = fraction >> shift;
        let half = 1u64 << (shift - 1);
        let round = discarded > half
            || discarded == half && (kept & 1 != 0 || precision == 0 && leading & 1 != 0);
        fraction = (kept + u64::from(round)) << shift;
        if fraction >> 52 != 0 {
            fraction = 0;
            leading += 1;
        }
    }
    write!(text, "0x{leading:x}").map_err(|_| NativeError::Extent)?;
    if precision != 0 || alternate {
        text.append_bytes(b".").map_err(|_| NativeError::Extent)?;
    }
    for digit in 0..precision {
        let nibble = if digit < 13 {
            ((fraction >> (48 - digit * 4)) & 15) as usize
        } else {
            0
        };
        text.append_bytes(&[b"0123456789abcdef"[nibble]])
            .map_err(|_| NativeError::Extent)?;
    }
    write!(text, "p{}{exponent}", if exponent >= 0 { "+" } else { "" })
        .map_err(|_| NativeError::Extent)?;
    Ok(())
}

pub(super) fn write(
    memory: &Memory,
    destination: u64,
    capacity: usize,
    format: u64,
    mut arguments: Arguments<'_>,
    truncate: bool,
) -> Result<usize, NativeError> {
    let mut input = Input {
        cursor: memory.cursor(format, 1)?,
        read: 0,
    };
    let mut output = Output {
        cursor: if capacity == 0 {
            None
        } else {
            Some(memory.cursor(destination, 2)?)
        },
        capacity,
        count: 0,
        truncate,
    };
    loop {
        let mut byte = input.next()?;
        if byte == 0 {
            return output.finish();
        }
        if byte != b'%' {
            output.byte(byte)?;
            continue;
        }
        byte = input.next()?;
        if byte == b'%' {
            output.byte(b'%')?;
            continue;
        }
        let mut flags = Flags::default();
        loop {
            match byte {
                b'-' => flags.left = true,
                b'+' => flags.plus = true,
                b' ' => flags.space = true,
                b'#' => flags.alternate = true,
                b'0' => flags.zero = true,
                _ => break,
            }
            byte = input.next()?;
        }
        let width;
        if byte == b'*' {
            let value = arguments.integer()? as i32;
            if value < 0 {
                flags.left = true;
            }
            width = value.unsigned_abs() as usize;
            byte = input.next()?;
        } else {
            (width, byte) = decimal(&mut input, byte)?;
        }
        if width > MAX_OUTPUT {
            return Err(NativeError::Extent);
        }
        let precision = if byte == b'.' {
            byte = input.next()?;
            if byte == b'*' {
                let value = arguments.integer()? as i32;
                byte = input.next()?;
                (value >= 0).then_some(value as usize)
            } else {
                let value;
                (value, byte) = decimal(&mut input, byte)?;
                Some(value)
            }
        } else {
            None
        };
        if precision.is_some_and(|value| value > MAX_OUTPUT) {
            return Err(NativeError::Extent);
        }
        let mut length = Length::Int;
        match byte {
            b'h' => {
                byte = input.next()?;
                length = if byte == b'h' {
                    byte = input.next()?;
                    Length::Char
                } else {
                    Length::Short
                };
            }
            b'l' => {
                byte = input.next()?;
                length = if byte == b'l' {
                    byte = input.next()?;
                    Length::LongLong
                } else {
                    Length::Long
                };
            }
            b'j' | b'z' | b't' => {
                length = Length::Pointer;
                byte = input.next()?;
            }
            b'I' if arguments.abi == NativeAbi::Microsoft => {
                byte = input.next()?;
                length = Length::Pointer;
                if byte == b'3' || byte == b'6' {
                    let digit = byte;
                    byte = input.next()?;
                    if byte != if digit == b'3' { b'2' } else { b'4' } {
                        return Err(NativeError::Unsupported);
                    }
                    length = if digit == b'3' {
                        Length::Int
                    } else {
                        Length::LongLong
                    };
                    byte = input.next()?;
                }
            }
            _ => {}
        }
        if flags.left {
            flags.zero = false;
        }
        match byte {
            b'd' | b'i' | b'u' | b'o' | b'x' | b'X' => {
                let bits = length.bytes(arguments.abi) * 8;
                let word = arguments.integer()?;
                let word = if bits == 64 {
                    word
                } else {
                    word & ((1u64 << bits) - 1)
                };
                let signed = matches!(byte, b'd' | b'i');
                let value = if signed {
                    ((word << (64 - bits)) as i64) >> (64 - bits)
                } else {
                    0
                };
                let magnitude = if signed { value.unsigned_abs() } else { word };
                let radix = if byte == b'o' {
                    8
                } else if matches!(byte, b'x' | b'X') {
                    16
                } else {
                    10
                };
                let mut storage = [0u8; 64];
                let mut at = storage.len();
                let mut rest = magnitude;
                if rest != 0 || precision != Some(0) {
                    loop {
                        at -= 1;
                        storage[at] = (if byte == b'X' {
                            b"0123456789ABCDEF"
                        } else {
                            b"0123456789abcdef"
                        })[(rest % radix) as usize];
                        rest /= radix;
                        if rest == 0 {
                            break;
                        }
                    }
                }
                let digits = &storage[at..];
                let prefix: &[u8] = if signed {
                    if value < 0 {
                        b"-"
                    } else if flags.plus {
                        b"+"
                    } else if flags.space {
                        b" "
                    } else {
                        b""
                    }
                } else if flags.alternate && magnitude != 0 && byte == b'x' {
                    b"0x"
                } else if flags.alternate && magnitude != 0 && byte == b'X' {
                    b"0X"
                } else {
                    b""
                };
                let mut zeroes = precision.unwrap_or(0).saturating_sub(digits.len());
                if byte == b'o' && flags.alternate && (digits.first() != Some(&b'0')) && zeroes == 0
                {
                    zeroes = 1;
                }
                if precision.is_some() {
                    flags.zero = false;
                }
                field(&mut output, prefix, digits, width, zeroes, &flags)?;
            }
            b's' => {
                if !matches!(length, Length::Int) {
                    return Err(NativeError::Unsupported);
                }
                let pointer = arguments.integer()?;
                if precision == Some(0) {
                    flags.zero = false;
                    field(&mut output, b"", b"", width, 0, &flags)?;
                    continue;
                }
                if pointer == 0 {
                    let bytes = b"(null)";
                    let count = precision.unwrap_or(bytes.len()).min(bytes.len());
                    flags.zero = false;
                    field(&mut output, b"", &bytes[..count], width, 0, &flags)?;
                    continue;
                }
                let mut cursor = memory.cursor(pointer, 1)?;
                let mut scan = cursor.clone();
                let mut count = 0;
                while count < precision.unwrap_or(MAX_OUTPUT) {
                    if scan.read()? == 0 {
                        break;
                    }
                    count += 1;
                }
                if precision.is_none() && count == MAX_OUTPUT && scan.read()? != 0 {
                    return Err(NativeError::Extent);
                }
                let padding = width.saturating_sub(count);
                if !flags.left {
                    output.repeat(b' ', padding)?;
                }
                for _ in 0..count {
                    output.byte(cursor.read()?)?;
                }
                if flags.left {
                    output.repeat(b' ', padding)?;
                }
            }
            b'c' => {
                if !matches!(length, Length::Int) {
                    return Err(NativeError::Unsupported);
                }
                flags.zero = false;
                field(
                    &mut output,
                    b"",
                    &[arguments.integer()? as u8],
                    width,
                    0,
                    &flags,
                )?;
            }
            b'p' => {
                let pointer = arguments.integer()?;
                let mut text = FixedText::<1024>::default();
                if arguments.abi == NativeAbi::Microsoft {
                    write!(text, "{pointer:016X}").map_err(|_| NativeError::Extent)?;
                } else if pointer == 0 {
                    text.set("(nil)").map_err(|_| NativeError::Extent)?;
                } else {
                    write!(text, "{pointer:x}").map_err(|_| NativeError::Extent)?;
                }
                let prefix = if arguments.abi == NativeAbi::SystemV && pointer != 0 {
                    b"0x".as_slice()
                } else {
                    b""
                };
                field(&mut output, prefix, text.as_bytes(), width, 0, &flags)?;
            }
            b'n' => {
                let pointer = arguments.integer()?;
                memory.put(pointer, length.bytes(arguments.abi), output.count as u64)?;
            }
            b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A' => {
                if !matches!(length, Length::Int | Length::Long) {
                    return Err(NativeError::Unsupported);
                }
                let value = arguments.float()?;
                let text = float_text(value.abs(), byte, precision, flags.alternate)?;
                if !value.is_finite() {
                    flags.zero = false;
                }
                field(
                    &mut output,
                    sign(value, &flags),
                    text.as_bytes(),
                    width,
                    0,
                    &flags,
                )?;
            }
            _ => return Err(NativeError::Unsupported),
        }
    }
}
