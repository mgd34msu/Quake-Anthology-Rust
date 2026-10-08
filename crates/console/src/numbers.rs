//! Native console numeric views. No locale or C runtime is used.
use crate::views::Source;

pub fn number(text: &str, source: Source) -> f32 {
    if matches!(source, Source::Quake | Source::QuakeWorld) {
        quake(text.as_bytes())
    } else {
        atof(text)
    }
}

pub fn integer(text: &str) -> i32 {
    let text = text.trim_start_matches([' ', '\t', '\n', '\r', '\u{b}', '\u{c}']);
    let (negative, bytes) = match text.as_bytes().first() {
        Some(b'-') => (true, &text.as_bytes()[1..]),
        Some(b'+') => (false, &text.as_bytes()[1..]),
        _ => (false, text.as_bytes()),
    };
    let limit = if negative { 2147483648u32 } else { 2147483647 };
    let mut value = 0u32;
    for &byte in bytes {
        if !byte.is_ascii_digit() {
            break;
        }
        let digit = u32::from(byte - b'0');
        value = if value > (limit - digit) / 10 {
            limit
        } else {
            value * 10 + digit
        };
    }
    if negative {
        value.wrapping_neg() as i32
    } else {
        value as i32
    }
}

fn quake(mut bytes: &[u8]) -> f32 {
    let sign = if bytes.first() == Some(&b'-') {
        bytes = &bytes[1..];
        -1.0
    } else {
        1.0
    };
    let mut value = 0.0f64;
    if bytes.starts_with(b"0x") || bytes.starts_with(b"0X") {
        for &byte in &bytes[2..] {
            if !byte.is_ascii_hexdigit() {
                break;
            }
            let (base, extra) = if byte.is_ascii_digit() {
                (b'0', 0.0)
            } else if byte.is_ascii_lowercase() {
                (b'a', 10.0)
            } else {
                (b'A', 10.0)
            };
            value = value * 16.0 + f64::from(byte) - f64::from(base) + extra;
        }
    } else if bytes.first() == Some(&b'\'') {
        value = f64::from(bytes.get(1).copied().unwrap_or(0) as i8);
    } else {
        let mut places = 0;
        let mut decimal = false;
        for &byte in bytes {
            if byte == b'.' {
                decimal = true;
                places = 0;
                continue;
            }
            if !byte.is_ascii_digit() {
                break;
            }
            value = value * 10.0 + f64::from(byte) - f64::from(b'0');
            places += usize::from(decimal);
        }
        for _ in 0..places {
            value /= 10.0;
        }
    }
    (value * sign) as f32
}

fn atof(text: &str) -> f32 {
    let text = text.trim_start_matches([' ', '\t', '\n', '\r', '\u{b}', '\u{c}']);
    let (negative, unsigned) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let sign = if negative { -1.0 } else { 1.0 };
    let bytes = unsigned.as_bytes();
    if bytes
        .get(..3)
        .is_some_and(|s| s.eq_ignore_ascii_case(b"inf"))
    {
        return sign * f32::INFINITY;
    }
    if bytes
        .get(..3)
        .is_some_and(|s| s.eq_ignore_ascii_case(b"nan"))
    {
        return sign * f32::NAN;
    }
    if bytes.starts_with(b"0x") || bytes.starts_with(b"0X") {
        return sign * hexadecimal(&bytes[2..]);
    }
    let mut end = 0;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    if bytes.get(end) == Some(&b'.') {
        end += 1;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
    }
    let mut exponent = end;
    if matches!(bytes.get(exponent), Some(b'e' | b'E')) {
        exponent += 1;
        if matches!(bytes.get(exponent), Some(b'-' | b'+')) {
            exponent += 1;
        }
        let digits = exponent;
        while bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
            exponent += 1;
        }
        if exponent != digits {
            end = exponent;
        }
    }
    // The consumed prefix contains ASCII only, so it ends at a UTF-8 boundary.
    match unsigned[..end].parse::<f64>() {
        Ok(value) => (value * f64::from(sign)) as f32,
        Err(_) => 0.0,
    }
}

fn hexadecimal(bytes: &[u8]) -> f32 {
    let mut mantissa = 0u64;
    let mut kept = 0;
    let mut discarded = 0i64;
    let mut fractional = 0i64;
    let mut dot = false;
    let mut sticky = false;
    let mut end = 0;
    while let Some(&byte) = bytes.get(end) {
        if byte == b'.' && !dot {
            dot = true;
            end += 1;
            continue;
        }
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => break,
        };
        fractional += i64::from(dot);
        if kept != 0 || digit != 0 {
            if kept < 15 {
                mantissa = mantissa * 16 + u64::from(digit);
                kept += 1;
            } else {
                discarded += 1;
                sticky |= digit != 0;
            }
        }
        end += 1;
    }
    let mut exponent = 0i64;
    if matches!(bytes.get(end), Some(b'p' | b'P')) {
        end += 1;
        let sign = if bytes.get(end) == Some(&b'-') {
            end += 1;
            -1
        } else {
            if bytes.get(end) == Some(&b'+') {
                end += 1;
            }
            1
        };
        while let Some(&digit @ b'0'..=b'9') = bytes.get(end) {
            exponent = (exponent * 10 + i64::from(digit - b'0')).min(100000);
            end += 1;
        }
        exponent *= sign;
    }
    if mantissa == 0 {
        return 0.0;
    }
    let trim = (64 - mantissa.leading_zeros()).saturating_sub(53);
    if trim != 0 {
        let remainder = mantissa & ((1u64 << trim) - 1);
        mantissa >>= trim;
        let half = 1u64 << (trim - 1);
        if remainder > half || (remainder == half && (sticky || mantissa & 1 != 0)) {
            mantissa += 1;
        }
    }
    let exponent = exponent - 4 * fractional + 4 * discarded + i64::from(trim);
    if exponent > 1024 {
        return f32::INFINITY;
    }
    if exponent < -1200 {
        return 0.0;
    }
    (mantissa as f64 * 2.0f64.powi(exponent as i32)) as f32
}
