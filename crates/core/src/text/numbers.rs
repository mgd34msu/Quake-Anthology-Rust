//! Locale-independent native strtod prefix semantics.

/// Return the parsed binary64 value and byte count, including leading ASCII
/// whitespace on success. An invalid prefix consumes nothing.
pub fn atof_prefix(text: &str) -> (f64, usize) {
    let bytes = text.as_bytes();
    let mut at = 0;
    while bytes
        .get(at)
        .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 11 | 12))
    {
        at += 1;
    }
    let negative = bytes.get(at) == Some(&b'-');
    if matches!(bytes.get(at), Some(b'+' | b'-')) {
        at += 1;
    }
    let start = at;
    let tail = &bytes[at..];
    let special = if tail
        .get(..3)
        .is_some_and(|b| b.eq_ignore_ascii_case(b"inf"))
    {
        at += if tail
            .get(..8)
            .is_some_and(|b| b.eq_ignore_ascii_case(b"infinity"))
        {
            8
        } else {
            3
        };
        Some(f64::INFINITY)
    } else if tail
        .get(..3)
        .is_some_and(|b| b.eq_ignore_ascii_case(b"nan"))
    {
        at += 3;
        let mut payload = 0;
        if bytes.get(at) == Some(&b'(') {
            let first = at + 1;
            let mut end = first;
            while bytes
                .get(end)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                end += 1;
            }
            if bytes.get(end) == Some(&b')') {
                payload = nan_payload(&bytes[first..end]);
                at = end + 1;
            }
        }
        Some(f64::from_bits(
            0x7ff8_0000_0000_0000 | (payload & 0x000f_ffff_ffff_ffff),
        ))
    } else {
        None
    };
    let value = if let Some(value) = special {
        value
    } else if tail.starts_with(b"0x") || tail.starts_with(b"0X") {
        if let Some((value, end)) = hexadecimal(bytes, at + 2) {
            at = end;
            value
        } else {
            at = start + 1;
            0.0
        }
    } else {
        let Some(end) = decimal_prefix_end(bytes, at) else {
            return (0.0, 0);
        };
        at = end;
        // Only the ASCII numeric prefix is indexed, so these are UTF-8 boundaries.
        text[start..at].parse::<f64>().unwrap_or(0.0)
    };
    (if negative { -value } else { value }, at)
}

/// Scan unsigned decimal digits, an optional point and a complete exponent.
/// Whitespace, signs and special-value policy belong to the caller.
pub fn decimal_prefix_end(bytes: &[u8], mut at: usize) -> Option<usize> {
    let mut digits = 0;
    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
        digits += 1;
    }
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
            digits += 1;
        }
    }
    (digits != 0).then(|| exponent(bytes, at, b'e').1)
}

pub fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
fn nan_payload(mut bytes: &[u8]) -> u64 {
    let base = if bytes.starts_with(b"0x") || bytes.starts_with(b"0X") {
        bytes = &bytes[2..];
        16
    } else if bytes.first() == Some(&b'0') {
        8
    } else {
        10
    };
    if bytes.is_empty() {
        return 0;
    }
    let mut value = 0u64;
    for &byte in bytes {
        let Some(digit) = hex_digit(byte).filter(|&v| v < base) else {
            return 0;
        };
        value = value
            .saturating_mul(u64::from(base))
            .saturating_add(u64::from(digit));
    }
    value
}
/// An incomplete exponent leaves its marker unconsumed.
fn exponent(bytes: &[u8], start: usize, marker: u8) -> (i64, usize) {
    if !bytes
        .get(start)
        .is_some_and(|&b| b.eq_ignore_ascii_case(&marker))
    {
        return (0, start);
    }
    let mut at = start + 1;
    let negative = bytes.get(at) == Some(&b'-');
    if matches!(bytes.get(at), Some(b'+' | b'-')) {
        at += 1;
    }
    let first = at;
    let mut value = 0i64;
    while let Some(&byte @ b'0'..=b'9') = bytes.get(at) {
        value = value
            .saturating_mul(10)
            .saturating_add(i64::from(byte - b'0'));
        at += 1;
    }
    if at == first {
        (0, start)
    } else {
        (if negative { -value } else { value }, at)
    }
}
fn hexadecimal(bytes: &[u8], start: usize) -> Option<(f64, usize)> {
    let mut at = start;
    let mut mantissa = 0u64;
    let mut kept = 0;
    let mut digits = 0;
    let mut discarded = 0i64;
    let mut fractional = 0i64;
    let mut dot = false;
    let mut sticky = false;
    while let Some(&byte) = bytes.get(at) {
        if byte == b'.' && !dot {
            dot = true;
            at += 1;
            continue;
        }
        let Some(digit) = hex_digit(byte) else {
            break;
        };
        digits += 1;
        fractional += i64::from(dot);
        if kept != 0 || digit != 0 {
            if kept < 16 {
                mantissa = mantissa * 16 + u64::from(digit);
                kept += 1;
            } else {
                discarded += 1;
                sticky |= digit != 0;
            }
        }
        at += 1;
    }
    if digits == 0 {
        return None;
    }
    let (power, at) = exponent(bytes, at, b'p');
    let power = power
        .saturating_sub(fractional.saturating_mul(4))
        .saturating_add(discarded.saturating_mul(4));
    Some((binary64(mantissa, power, sticky), at))
}
/// One nearest-even rounding, including the subnormal boundary; multiplying
/// an already rounded mantissa by a power of two could double-round here.
fn binary64(mantissa: u64, power: i64, sticky: bool) -> f64 {
    if mantissa == 0 {
        return 0.0;
    }
    let bits = 64 - mantissa.leading_zeros();
    let mut exponent = power.saturating_add(i64::from(bits - 1));
    if exponent > 1023 {
        return f64::INFINITY;
    }
    if exponent < -1022 {
        let shift = power.saturating_add(1074);
        let value = if shift >= 0 {
            mantissa << shift as u32
        } else {
            round_right(mantissa, shift.saturating_neg().min(65) as u32, sticky)
        };
        return f64::from_bits(value);
    }
    let mut value = if bits <= 53 {
        mantissa << (53 - bits)
    } else {
        round_right(mantissa, bits - 53, sticky)
    };
    if value == 1 << 53 {
        value >>= 1;
        exponent += 1;
    }
    if exponent > 1023 {
        return f64::INFINITY;
    }
    f64::from_bits(((exponent + 1023) as u64) << 52 | (value & 0x000f_ffff_ffff_ffff))
}
fn round_right(value: u64, shift: u32, sticky: bool) -> u64 {
    if shift > 64 {
        return 0;
    }
    let (high, low, half) = if shift == 64 {
        (0, value, 1 << 63)
    } else {
        (value >> shift, value & ((1 << shift) - 1), 1 << (shift - 1))
    };
    high + u64::from(low > half || (low == half && (sticky || high & 1 != 0)))
}
