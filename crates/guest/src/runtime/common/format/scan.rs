//! UCRT numeric `scanf` over checked guest bytes.
//!
//! Donor: `src/guest/runtime/common/format/scan.ts`. Decimal input converts
//! directly to the destination precision, avoiding binary64 to binary32
//! double rounding.

use crate::core::contracts::GuestAddress;
use crate::core::memory::SparseGuestMemory;
use crate::error::GuestError;
use crate::floating_point::binary::{
    encode_binary, format_for, round_rational, write_bits, BigInt, BinaryValue, BinaryWidth,
    Rounding,
};
use crate::runtime::common::format::arguments::{
    FormatArgument, FormatArgumentType, FormatArguments, FormatDialect,
};
use crate::runtime::common::memory::{read_string, write_unsigned};

/// Scan conversion the runtime does not implement.
#[derive(Debug)]
pub struct UnsupportedGuestScan(pub String);

impl std::fmt::Display for UnsupportedGuestScan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsupported scanf conversion: {}", self.0)
    }
}

impl std::error::Error for UnsupportedGuestScan {}

impl From<UnsupportedGuestScan> for GuestError {
    fn from(error: UnsupportedGuestScan) -> Self {
        GuestError::unsupported(error.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanCode {
    Decimal,
    Integer,
    Float,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScanDirective {
    Space,
    Literal(char),
    Number {
        code: ScanCode,
        width: u64,
        suppressed: bool,
        bytes: usize,
    },
}

fn is_scan_whitespace(value: char) -> bool {
    matches!(value, '\t' | '\n' | '\x0b' | '\x0c' | '\r' | ' ')
}

fn is_ascii_digit(value: char) -> bool {
    value.is_ascii_digit()
}

fn directives(format: &str) -> Result<Vec<ScanDirective>, UnsupportedGuestScan> {
    let chars: Vec<char> = format.chars().collect();
    let mut result = Vec::new();
    let mut index = 0;
    let at = |index: usize| -> char { chars.get(index).copied().unwrap_or('\0') };
    while index < chars.len() {
        let value = chars[index];
        index += 1;
        if is_scan_whitespace(value) {
            result.push(ScanDirective::Space);
            continue;
        }
        if value != '%' {
            result.push(ScanDirective::Literal(value));
            continue;
        }
        if at(index) == '%' {
            index += 1;
            result.push(ScanDirective::Literal('%'));
            continue;
        }
        let suppressed = at(index) == '*';
        if suppressed {
            index += 1;
        }
        let mut digits = String::new();
        while index < chars.len() && is_ascii_digit(at(index)) {
            digits.push(at(index));
            index += 1;
        }
        let width = if digits.is_empty() {
            u64::MAX
        } else {
            match digits.parse::<u64>() {
                Ok(width) if width != 0 && width <= (1 << 53) - 1 => width,
                _ => {
                    return Err(UnsupportedGuestScan(
                        "invalid numeric scanf field width".to_string(),
                    ));
                }
            }
        };
        let long = at(index) == 'l';
        if long {
            index += 1;
        }
        let code = at(index);
        index += 1;
        if matches!(code, 'f' | 'F' | 'e' | 'E' | 'g' | 'G') {
            result.push(ScanDirective::Number {
                code: ScanCode::Float,
                width,
                suppressed,
                bytes: if long { 8 } else { 4 },
            });
        } else if (code == 'd' || code == 'i') && !long {
            result.push(ScanDirective::Number {
                code: if code == 'd' { ScanCode::Decimal } else { ScanCode::Integer },
                width,
                suppressed,
                bytes: 4,
            });
        } else {
            return Err(UnsupportedGuestScan(format!(
                "unsupported scanf conversion in {format}"
            )));
        }
    }
    Ok(result)
}

fn decimal_value(digits: &str) -> BigInt {
    let mut value = BigInt::zero();
    let ten = BigInt::from_u64(10);
    for digit in digits.bytes() {
        value = value
            .mul(&ten)
            .add(&BigInt::from_u64(u64::from(digit - b'0')));
    }
    value
}

fn finite_bytes(negative: bool, width: BinaryWidth, bytes: usize) -> Vec<u8> {
    write_bits(
        &encode_binary(
            &BinaryValue::Finite {
                negative,
                coefficient: BigInt::zero(),
                exponent: 0,
                denormal: false,
            },
            width,
        ),
        bytes,
    )
}

/// Convert decimal text directly to the destination precision.
fn decimal_bytes(token: &str, bytes: usize) -> Result<Vec<u8>, GuestError> {
    let negative = token.starts_with('-');
    let unsigned = token.strip_prefix('+').unwrap_or(token.strip_prefix('-').unwrap_or(token));
    let lower = unsigned.to_ascii_lowercase();
    let mut parts = lower.splitn(2, 'e');
    let mantissa = parts.next().unwrap_or("");
    let exponent_text = parts.next().unwrap_or("0");
    let dot = mantissa.find('.');
    let digits: String = mantissa
        .chars()
        .filter(|c| *c != '.')
        .collect::<String>()
        .trim_start_matches('0')
        .to_string();
    let parsed: i64 = exponent_text.parse().unwrap_or(if exponent_text.starts_with('-') {
        i64::MIN
    } else {
        i64::MAX
    });
    let exponent = parsed.saturating_sub(match dot {
        Some(dot) => (mantissa.len() - dot - 1) as i64,
        None => 0,
    });
    let width = if bytes == 4 { BinaryWidth::W32 } else { BinaryWidth::W64 };
    if digits.is_empty() {
        return Ok(finite_bytes(negative, width, bytes));
    }
    // Values beyond these bounds round to infinity or signed zero in both
    // supported formats.
    let magnitude = (exponent as i128) + (digits.len() as i128);
    if magnitude > 400 {
        return Ok(write_bits(
            &encode_binary(&BinaryValue::Infinity { negative }, width),
            bytes,
        ));
    }
    if magnitude < -400 {
        return Ok(finite_bytes(negative, width, bytes));
    }
    let coefficient = decimal_value(&digits);
    let mut power = BigInt::one();
    let ten = BigInt::from_u64(10);
    for _ in 0..exponent.unsigned_abs() as usize {
        power = power.mul(&ten);
    }
    let one = BigInt::one();
    let scaled = coefficient.mul(&power);
    let rounded = round_rational(
        negative,
        if exponent >= 0 { &scaled } else { &coefficient },
        if exponent >= 0 { &one } else { &power },
        0,
        format_for(width),
        Rounding::Nearest,
    )?;
    Ok(write_bits(&encode_binary(&rounded.value, width), bytes))
}

fn valid_float_token(token: &str) -> bool {
    let body = token.strip_prefix('+').unwrap_or(token.strip_prefix('-').unwrap_or(token));
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(at) => {
            let exponent = &body[at + 1..];
            let digits = exponent.strip_prefix('+').unwrap_or(exponent.strip_prefix('-').unwrap_or(exponent));
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
            (&body[..at], true)
        }
        None => (body, false),
    };
    let _ = exponent;
    if mantissa.is_empty() {
        return false;
    }
    match mantissa.find('.') {
        Some(dot) => {
            let (head, tail) = (&mantissa[..dot], &mantissa[dot + 1..]);
            if head.is_empty() && tail.is_empty() {
                return false;
            }
            head.bytes().all(|b| b.is_ascii_digit()) && tail.bytes().all(|b| b.is_ascii_digit())
        }
        None => mantissa.bytes().all(|b| b.is_ascii_digit()),
    }
}

fn valid_int_token(token: &str) -> bool {
    let body = token.strip_prefix('+').unwrap_or(token.strip_prefix('-').unwrap_or(token));
    if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit())
    } else {
        !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit())
    }
}

/// Scan the guest buffer at `input` with `format`; returns the assigned
/// count, or -1 when no conversion consumed input at end-of-input.
pub fn scan_windows_buffer(
    memory: &mut SparseGuestMemory,
    input: GuestAddress,
    capacity: u64,
    format: GuestAddress,
    arguments: Option<GuestAddress>,
) -> Result<i32, GuestError> {
    let format_text = read_string(memory, format, false)?;
    let parsed = directives(&format_text)?;
    let mut args = FormatArguments::open(memory, FormatDialect::Windows, arguments)?;
    let mut cursor: u64 = 0;
    let mut assigned: i32 = 0;
    let mut converted = false;

    macro_rules! peek {
        () => {{
            if cursor >= capacity {
                None
            } else {
                let byte = memory.read_u8(memory.offset(input, cursor as i64)?)?;
                if byte == 0 { None } else { Some(byte as char) }
            }
        }};
    }

    for directive in &parsed {
        match directive {
            ScanDirective::Space => {
                while let Some(c) = peek!() {
                    if !is_scan_whitespace(c) {
                        break;
                    }
                    cursor += 1;
                }
            }
            ScanDirective::Literal(value) => {
                if peek!() != Some(*value) {
                    return Ok(if !converted && peek!().is_none() {
                        -1
                    } else {
                        assigned
                    });
                }
                cursor += 1;
            }
            ScanDirective::Number {
                code,
                width,
                suppressed,
                bytes,
            } => {
                while let Some(c) = peek!() {
                    if !is_scan_whitespace(c) {
                        break;
                    }
                    cursor += 1;
                }
                if peek!().is_none() {
                    return Ok(if !converted { -1 } else { assigned });
                }
                let start = cursor;
                let mut token = String::new();
                macro_rules! current {
                    () => {{
                        if cursor - start < *width { peek!() } else { None }
                    }};
                }
                if matches!(current!(), Some('+') | Some('-')) {
                    token.push(peek!().expect("sign checked"));
                    cursor += 1;
                }
                if *code == ScanCode::Float {
                    if matches!(current!(), Some('i' | 'I' | 'n' | 'N')) {
                        while let Some(c) = current!() {
                            if !c.is_ascii_alphabetic() || token.len() >= 9 {
                                break;
                            }
                            token.push(c);
                            cursor += 1;
                        }
                        let body = token.strip_prefix('+').unwrap_or(token.strip_prefix('-').unwrap_or(token.as_str()));
                        if body.len() >= 3
                            && (body[..3].eq_ignore_ascii_case("inf")
                                || body[..3].eq_ignore_ascii_case("nan"))
                        {
                            return Err(UnsupportedGuestScan(
                                "scanf nonfinite text is not implemented".to_string(),
                            )
                            .into());
                        }
                        return Ok(assigned);
                    }
                    while matches!(current!(), Some(c) if c.is_ascii_digit()) {
                        token.push(peek!().expect("digit checked"));
                        cursor += 1;
                    }
                    let token_is_zero = token == "+0" || token == "-0" || token == "0";
                    if token_is_zero && matches!(current!(), Some('x' | 'X')) {
                        return Err(UnsupportedGuestScan(
                            "scanf hexadecimal floating input is not implemented".to_string(),
                        )
                        .into());
                    }
                    if current!() == Some('.') {
                        token.push('.');
                        cursor += 1;
                        while matches!(current!(), Some(c) if c.is_ascii_digit()) {
                            token.push(peek!().expect("digit checked"));
                            cursor += 1;
                        }
                    }
                    if matches!(current!(), Some('e' | 'E')) {
                        token.push(peek!().expect("exponent checked"));
                        cursor += 1;
                        if matches!(current!(), Some('+') | Some('-')) {
                            token.push(peek!().expect("sign checked"));
                            cursor += 1;
                        }
                        while matches!(current!(), Some(c) if c.is_ascii_digit()) {
                            token.push(peek!().expect("digit checked"));
                            cursor += 1;
                        }
                    }
                    if !valid_float_token(&token) {
                        return Ok(assigned);
                    }
                } else {
                    let mut base = 10;
                    if *code == ScanCode::Integer && current!() == Some('0') {
                        token.push('0');
                        cursor += 1;
                        base = 8;
                        if matches!(current!(), Some('x' | 'X')) {
                            token.push(peek!().expect("prefix checked"));
                            cursor += 1;
                            base = 16;
                        }
                    }
                    loop {
                        let digit = match current!() {
                            Some(c)
                                if base == 16 && c.is_ascii_hexdigit()
                                    || base == 8 && matches!(c, '0'..='7')
                                    || base == 10 && c.is_ascii_digit() =>
                            {
                                c
                            }
                            _ => break,
                        };
                        token.push(digit);
                        cursor += 1;
                    }
                    if !valid_int_token(&token) {
                        return Ok(assigned);
                    }
                }
                converted = true;
                if *suppressed {
                    continue;
                }
                let argument = args.next(memory, FormatArgumentType::Pointer)?;
                let FormatArgument::Integer(raw) = argument else {
                    return Err(GuestError::invalid("scanf destination must be a pointer"));
                };
                let destination = memory.pointer(raw)?.ok_or_else(|| {
                    GuestError::invalid("scanf destination is null")
                })?;
                if *code == ScanCode::Float {
                    let bytes = decimal_bytes(&token, *bytes)?;
                    memory.write(destination, &bytes)?;
                } else {
                    let unsigned = token.strip_prefix('+').unwrap_or(token.strip_prefix('-').unwrap_or(token.as_str()));
                    let (digits, radix) = if let Some(hex) = unsigned
                        .strip_prefix("0x")
                        .or_else(|| unsigned.strip_prefix("0X"))
                    {
                        (hex, 16u64)
                    } else if *code == ScanCode::Integer
                        && unsigned.len() >= 2
                        && unsigned.starts_with('0')
                        && unsigned.bytes().all(|b| matches!(b, b'0'..=b'7'))
                    {
                        (unsigned, 8u64)
                    } else {
                        (unsigned, 10u64)
                    };
                    let mut value: u64 = 0;
                    for digit in digits.bytes() {
                        let digit = u64::from(if digit.is_ascii_digit() {
                            digit - b'0'
                        } else if digit.is_ascii_lowercase() {
                            digit - b'a' + 10
                        } else {
                            digit - b'A' + 10
                        });
                        value = value.wrapping_mul(radix).wrapping_add(digit);
                    }
                    let signed = if token.starts_with('-') {
                        (value as i64).wrapping_neg() as u64
                    } else {
                        value
                    };
                    write_unsigned(memory, destination, 4, i128::from(signed as u32))?;
                }
                assigned += 1;
            }
        }
    }
    Ok(assigned)
}
