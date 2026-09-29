//! Deterministic `printf` float conversion over guest binary values.
//!
//! Donor: `src/guest/runtime/common/format/float.ts`. Decimal conversion
//! rounds the guest binary value itself, including x87 values beyond
//! binary64. No host float formatting is used.

use crate::floating_point::binary::{BigInt, BinaryValue, Rounding};

/// Printf rounding: an IEEE mode, or UCRT legacy round-half-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatRounding {
    /// IEEE rounding mode.
    Ieee(Rounding),
    /// Round half away from zero (UCRT default).
    LegacyNearest,
}

struct Finite<'a> {
    negative: bool,
    coefficient: &'a BigInt,
    exponent: i32,
    denormal: bool,
}

fn as_finite(value: &BinaryValue) -> Option<Finite<'_>> {
    match value {
        BinaryValue::Finite {
            negative,
            coefficient,
            exponent,
            denormal,
        } => Some(Finite {
            negative: *negative,
            coefficient,
            exponent: *exponent,
            denormal: *denormal,
        }),
        _ => None,
    }
}

fn pow10(exp: usize) -> BigInt {
    let mut result = BigInt::one();
    let ten = BigInt::from_u64(10);
    for _ in 0..exp {
        result = result.mul(&ten);
    }
    result
}

/// Render a non-negative integer in `radix` (8, 10, or 16), lowercase.
fn to_radix(value: &BigInt, radix: u32) -> String {
    debug_assert!(!value.is_negative());
    if value.is_zero() {
        return "0".to_string();
    }
    let divisor = BigInt::from_u64(u64::from(radix));
    let mut digits = Vec::new();
    let mut current = value.clone();
    while !current.is_zero() {
        let (quotient, remainder) = current.divmod(&divisor).expect("nonzero radix divisor");
        let digit = remainder.low_u64() as u32;
        digits.push(char::from_digit(digit, radix).expect("radix digit"));
        current = quotient;
    }
    digits.iter().rev().collect()
}

fn decimal(value: &BigInt) -> String {
    to_radix(value, 10)
}

fn quotient(numerator: &BigInt, denominator: &BigInt, negative: bool, mode: FormatRounding) -> BigInt {
    let (value, remainder) = numerator.divmod(denominator).expect("nonzero divisor");
    let two = BigInt::from_u64(2);
    let doubled = remainder.mul(&two);
    let up = !remainder.is_zero()
        && match mode {
            FormatRounding::Ieee(Rounding::Nearest) => {
                doubled > *denominator || (doubled == *denominator && (value.low_u64() & 1) != 0)
            }
            FormatRounding::LegacyNearest => doubled >= *denominator,
            FormatRounding::Ieee(Rounding::Up) => !negative,
            FormatRounding::Ieee(Rounding::Down) => negative,
            FormatRounding::Ieee(Rounding::Zero) => false,
        };
    if up {
        value.add(&BigInt::one())
    } else {
        value
    }
}

fn ratio(value: &Finite<'_>) -> (BigInt, BigInt) {
    if value.exponent >= 0 {
        (value.coefficient.shl_bits(value.exponent as u32), BigInt::one())
    } else {
        (
            value.coefficient.clone(),
            BigInt::one().shl_bits((-value.exponent) as u32),
        )
    }
}

fn scaled(value: &Finite<'_>, decimal_places: i64, mode: FormatRounding) -> BigInt {
    let (numerator, denominator) = ratio(value);
    if decimal_places >= 0 {
        let scaled = numerator.mul(&pow10(decimal_places as usize));
        quotient(&scaled, &denominator, value.negative, mode)
    } else {
        let scaled = denominator.mul(&pow10((-decimal_places) as usize));
        quotient(&numerator, &scaled, value.negative, mode)
    }
}

fn decimal_exponent(value: &Finite<'_>) -> i64 {
    if value.coefficient.is_zero() {
        return 0;
    }
    let (numerator, denominator) = ratio(value);
    let mut exponent = decimal(&numerator).len() as i64 - decimal(&denominator).len() as i64;
    let below = if exponent >= 0 {
        numerator < denominator.mul(&pow10(exponent as usize))
    } else {
        numerator.mul(&pow10((-exponent) as usize)) < denominator
    };
    if below {
        exponent -= 1;
    }
    exponent
}

fn fixed_digits(digits: &str, places: usize, point: bool) -> String {
    if places == 0 {
        return if point {
            format!("{digits}.")
        } else {
            digits.to_string()
        };
    }
    let padded = format!("{digits:0>width$}", width = places + 1);
    format!(
        "{}.{}",
        &padded[..padded.len() - places],
        &padded[padded.len() - places..]
    )
}

fn exponent_suffix(exponent: i64, marker: char, minimum: usize) -> String {
    let sign = if exponent < 0 { '-' } else { '+' };
    let digits = exponent.abs().to_string();
    format!("{marker}{sign}{digits:0>minimum$}")
}

fn significant(value: &Finite<'_>, precision: usize, mode: FormatRounding) -> (String, i64) {
    let mut exponent = decimal_exponent(value);
    let mut digits = decimal(&scaled(value, precision as i64 - 1 - exponent, mode));
    if digits.len() > precision {
        exponent += 1;
        digits.truncate(precision);
    }
    (format!("{digits:0>precision$}"), exponent)
}

fn hex(
    value: &Finite<'_>,
    precision: Option<usize>,
    alternate: bool,
    mode: FormatRounding,
    windows: bool,
    extended: bool,
) -> String {
    let fraction_bits: i64 = if extended { 63 } else { 52 };
    let mut exponent = if value.coefficient.is_zero() {
        0
    } else if value.denormal {
        if extended {
            -16382
        } else {
            -1022
        }
    } else {
        i64::from(value.exponent) + fraction_bits
    };
    // glibc writes x87's explicit integer bit as the top bit of one hex digit.
    let leading_bits: i64 = if extended && !windows { 4 } else { 1 };
    if !value.coefficient.is_zero() {
        exponent -= leading_bits - 1;
    }
    let fraction = fraction_bits - leading_bits + 1;
    let natural = (fraction + 3) / 4;
    let places: i64 = match precision {
        Some(places) => places as i64,
        None => {
            if windows {
                13
            } else {
                natural
            }
        }
    };
    let shift = places * 4 - fraction;
    let mut digits = if shift >= 0 {
        to_radix(&value.coefficient.shl_bits(shift as u32), 16)
    } else {
        to_radix(
            &quotient(
                value.coefficient,
                &BigInt::one().shl_bits((-shift) as u32),
                value.negative,
                mode,
            ),
            16,
        )
    };
    let width = (places + 1) as usize;
    if digits.len() < width {
        digits = format!("{digits:0>width$}");
    }
    if digits.len() > width {
        digits = format!("1{}", "0".repeat(places as usize));
        exponent += 4;
    }
    let places_usize = places as usize;
    let head = digits[..digits.len() - places_usize.min(digits.len())].to_string();
    let tail = if places == 0 {
        String::new()
    } else {
        digits[digits.len() - places_usize..].to_string()
    };
    let fraction_text = if precision.is_none() && !windows {
        tail.trim_end_matches('0').to_string()
    } else {
        tail
    };
    format!(
        "0x{head}{}{}",
        if fraction_text.is_empty() && !alternate {
            String::new()
        } else {
            format!(".{fraction_text}")
        },
        exponent_suffix(exponent, 'p', 1)
    )
}

/// Format `value` with `printf` conversion `conversion`.
#[allow(clippy::too_many_arguments)]
pub fn format_float(
    value: &BinaryValue,
    conversion: char,
    precision: Option<usize>,
    alternate: bool,
    mode: FormatRounding,
    windows: bool,
    extended: bool,
    exponent_digits: usize,
) -> String {
    let lower = conversion.to_ascii_lowercase();
    let upper = conversion != lower;
    let text = if matches!(value, BinaryValue::Infinity { .. }) {
        "inf".to_string()
    } else if matches!(value, BinaryValue::Unsupported { .. }) {
        if windows {
            "nan(ind)".to_string()
        } else {
            "nan".to_string()
        }
    } else if let BinaryValue::Nan {
        negative,
        payload,
        signaling,
    } = value
    {
        if windows && *signaling {
            "nan(snan)".to_string()
        } else if windows && *negative && *payload == BigInt::one().shl_bits(62) {
            "nan(ind)".to_string()
        } else {
            "nan".to_string()
        }
    } else if lower == 'a' {
        let finite = as_finite(value).expect("finite hex conversion");
        hex(&finite, precision, alternate, mode, windows, extended)
    } else if lower == 'f' {
        let places = precision.unwrap_or(6);
        let finite = as_finite(value).expect("finite decimal conversion");
        fixed_digits(&decimal(&scaled(&finite, places as i64, mode)), places, alternate)
    } else {
        let places = precision.unwrap_or(6);
        let count = if lower == 'e' { places + 1 } else { places.max(1) };
        let finite = as_finite(value).expect("finite decimal conversion");
        let (digits, exponent) = significant(&finite, count, mode);
        if lower == 'e' || exponent < -4 || exponent >= count as i64 {
            let mut tail = digits[1..].to_string();
            if lower == 'g' && !alternate {
                tail = tail.trim_end_matches('0').to_string();
            }
            format!(
                "{}{}{}",
                &digits[..1],
                if tail.is_empty() && !alternate {
                    String::new()
                } else {
                    format!(".{tail}")
                },
                exponent_suffix(exponent, 'e', exponent_digits)
            )
        } else {
            let decimal_places = (count as i64 - 1 - exponent) as usize;
            let mut text = fixed_digits(&digits, decimal_places, alternate);
            if !alternate && text.contains('.') {
                text = text.trim_end_matches('0').to_string();
                text = text.strip_suffix('.').unwrap_or(&text).to_string();
            }
            text
        }
    };
    if upper {
        text.to_uppercase()
    } else {
        text
    }
}
