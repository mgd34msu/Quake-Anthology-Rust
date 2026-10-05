//! Numeric semantics: qsrc behavior per dialect (Q1/Q2 C float semantics
//! with `f32` storage, Q3 binary32, QVM `OP_CVFI`). The `DonorBinary64` and
//! `DonorSource` names are retained only until the gameplay verticals delete
//! the per-operation wrappers; they no longer reproduce any donor port.

use thiserror::Error;

/// Error for conversions outside the defined signed 32-bit range.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NumericError {
    /// A float-to-int conversion left the `i32` range (or was NaN).
    #[error("Float-to-int conversion is outside the defined signed 32-bit range")]
    IntRange,
    /// The profile needs a native arithmetic backend this build does not have.
    #[error("Numeric profile {0} requires its own arithmetic backend")]
    UnsupportedProfile(String),
    /// Native number text must be byte characters.
    #[error("Native numbers require byte characters")]
    NonByteText,
}

/// C-style signed 32-bit wraparound (`value | 0`).
#[must_use]
pub fn wrap_i32(value: i64) -> i32 {
    value as i32
}

/// C-style unsigned 32-bit wraparound (`value >>> 0`).
#[must_use]
pub fn wrap_u32(value: i64) -> u32 {
    value as u32
}

/// JavaScript `ToInt32` semantics for float inputs: truncate toward zero,
/// reduce modulo 2^32 into the signed range; non-finite values map to 0.
#[must_use]
pub fn float_to_wrapped_i32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let reduced = value.trunc().rem_euclid(4_294_967_296.0);
    if reduced >= 2_147_483_648.0 {
        (reduced - 4_294_967_296.0) as i32
    } else {
        reduced as i32
    }
}

/// JavaScript `ToUint32` semantics for float inputs.
#[must_use]
pub fn float_to_wrapped_u32(value: f64) -> u32 {
    float_to_wrapped_i32(value) as u32
}

/// Round a value to the nearest representable IEEE-754 binary32 value.
#[must_use]
pub fn store_f32(value: f64) -> f32 {
    value as f32
}

/// QVM `CVFI4`: consumes binary32, returns `INT_MIN` for NaN/out-of-range.
#[must_use]
pub fn qvm_float_to_int(value: f32) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        value.trunc() as i32
    } else {
        i32::MIN
    }
}

/// C-style truncation that rejects NaN and out-of-range input instead of
/// producing an indefinite value.
pub fn checked_float_to_int(value: f64) -> Result<i32, NumericError> {
    let integer = value.trunc();
    if !integer.is_finite() || integer < -2_147_483_648.0 || integer > 2_147_483_647.0 {
        return Err(NumericError::IntRange);
    }
    Ok(integer as i32)
}

/// Bit pattern of a binary32 value.
#[must_use]
pub fn float32_to_bits(value: f32) -> u32 {
    value.to_bits()
}

/// Binary32 value for a bit pattern.
#[must_use]
pub fn bits_to_float32(bits: u32) -> f32 {
    f32::from_bits(bits)
}

/// One Quake `Q_rand` update: `seed * 69069 + 1` with 32-bit wraparound.
#[must_use]
pub fn q_rand(seed: i32) -> i32 {
    seed.wrapping_mul(69_069).wrapping_add(1)
}

/// Result of one `Q_rand` update plus its derived fraction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RandomStep {
    /// Updated seed.
    pub seed: i32,
    /// Derived fraction.
    pub value: f64,
}

/// One `Q_rand` update plus Quake's `[0, 1)` fraction.
#[must_use]
pub fn q_random(seed: i32) -> RandomStep {
    let next = q_rand(seed);
    RandomStep {
        seed: next,
        value: f64::from((next & 0xffff) as u16) / 65_536.0,
    }
}

/// One `Q_rand` update plus Quake's `[-1, 1)` fraction.
#[must_use]
pub fn q_crandom(seed: i32) -> RandomStep {
    let step = q_random(seed);
    RandomStep {
        seed: step.seed,
        value: 2.0 * (step.value - 0.5),
    }
}

/// Which arithmetic a call chain evaluates with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arithmetic {
    /// Round every operation to binary32 (Q3 binary32).
    Binary32EachOp,
    /// C float semantics with binary32 storage (Q1/Q2 engine code and
    /// QuakeC opcodes, which store a float after every op). The name is a
    /// retained alias; behavior follows the originals, not any donor port.
    DonorBinary64(DonorSource),
    /// Native x87 precision; needs its own backend.
    X87 {
        /// Mantissa precision in bits.
        precision_bits: X87Precision,
        /// Rounding direction.
        rounding: Rounding,
    },
    /// Native SSE arithmetic.
    Sse {
        /// Flush-to-zero enabled.
        flush_to_zero: bool,
        /// Denormals-are-zero enabled.
        denormals_are_zero: bool,
        /// Rounding direction.
        rounding: Rounding,
    },
}

/// Which original engine the C-float path models. The name is a retained
/// alias; behavior follows qsrc (WinQuake `mathlib.c`, Quake 2 `q_shared.c`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DonorSource {
    /// Quake I C float semantics.
    Q1,
    /// Quake II C float semantics.
    Q2,
}

/// x87 precision control settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X87Precision {
    /// 24-bit (binary32) precision.
    Bits24,
    /// 53-bit (binary64) precision.
    Bits53,
    /// 64-bit (extended) precision.
    Bits64,
}

/// IEEE-754 rounding directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    /// Round to nearest, ties to even.
    NearestEven,
    /// Round toward zero.
    TowardZero,
    /// Round toward positive infinity.
    TowardPositive,
    /// Round toward negative infinity.
    TowardNegative,
}

/// How a call chain converts floats to `i32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatToInt {
    /// QVM indefinite value (`INT_MIN`) on NaN/out-of-range.
    QvmIndefinite,
    /// x86 indefinite value; needs its own backend.
    X86Indefinite,
    /// Reject NaN/out-of-range with an error.
    CheckedTruncation,
}

/// Arithmetic selection for one owner. Storage is always binary32 and
/// integer overflow always wraps at 32 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericProfile {
    /// Stable implementation name, e.g. `"q3:binary32"`.
    pub id: &'static str,
    /// Operation rounding.
    pub arithmetic: Arithmetic,
    /// Float-to-int conversion rule.
    pub float_to_int: FloatToInt,
}

/// Q3 binary32 each-operation profile.
pub const Q3_BINARY32_PROFILE: NumericProfile = NumericProfile {
    id: "q3:binary32",
    arithmetic: Arithmetic::Binary32EachOp,
    float_to_int: FloatToInt::QvmIndefinite,
};

/// Q1 C float profile: `f32` storage with per-operation rounding, matching
/// WinQuake `float` declarations (`mathlib.c`) and QuakeC's store-after-op.
/// C `if (x)` is `x != 0.0`, true for NaN.
pub const Q1_DONOR_PROFILE: NumericProfile = NumericProfile {
    id: "q1:donor-binary64",
    arithmetic: Arithmetic::DonorBinary64(DonorSource::Q1),
    float_to_int: FloatToInt::CheckedTruncation,
};

/// Q2 C float profile: `f32` storage with per-operation rounding, matching
/// Quake 2 `float` declarations (`q_shared.c`, game `*.c`).
pub const Q2_DONOR_PROFILE: NumericProfile = NumericProfile {
    id: "q2:donor-binary64",
    arithmetic: Arithmetic::DonorBinary64(DonorSource::Q2),
    float_to_int: FloatToInt::CheckedTruncation,
};

/// Arithmetic operations bound to one profile. Selected once per owner.
#[derive(Debug, Clone, Copy)]
pub struct NumericOps {
    /// The selected profile.
    pub profile: NumericProfile,
    round_each_op: bool,
}

impl NumericOps {
    /// Select the operations for a profile. Unsupported native modes fail.
    pub fn select(profile: NumericProfile) -> Result<Self, NumericError> {
        let round_each_op = match profile.arithmetic {
            Arithmetic::Binary32EachOp => true,
            Arithmetic::DonorBinary64(_) => true,
            Arithmetic::Sse {
                flush_to_zero: false,
                denormals_are_zero: false,
                rounding: Rounding::NearestEven,
            } => true,
            Arithmetic::Sse { .. } | Arithmetic::X87 { .. } => {
                return Err(NumericError::UnsupportedProfile(profile.id.to_string()));
            }
        };
        if profile.float_to_int == FloatToInt::X86Indefinite {
            return Err(NumericError::UnsupportedProfile(profile.id.to_string()));
        }
        Ok(Self { profile, round_each_op })
    }

    fn round(&self, value: f64) -> f64 {
        if self.round_each_op {
            f64::from(value as f32)
        } else {
            value
        }
    }

    /// Store a scalar as binary32.
    #[must_use]
    pub fn store(&self, value: f64) -> f32 {
        value as f32
    }

    /// Profile-rounded addition.
    #[must_use]
    pub fn add(&self, left: f64, right: f64) -> f64 {
        self.round(left + right)
    }

    /// Profile-rounded subtraction.
    #[must_use]
    pub fn sub(&self, left: f64, right: f64) -> f64 {
        self.round(left - right)
    }

    /// Profile-rounded multiplication.
    #[must_use]
    pub fn mul(&self, left: f64, right: f64) -> f64 {
        self.round(left * right)
    }

    /// Profile-rounded division.
    #[must_use]
    pub fn div(&self, left: f64, right: f64) -> f64 {
        self.round(left / right)
    }

    /// Profile-rounded square root.
    #[must_use]
    pub fn sqrt(&self, value: f64) -> f64 {
        self.round(value.sqrt())
    }

    /// Profile float-to-int conversion.
    pub fn to_int32(&self, value: f64) -> Result<i32, NumericError> {
        match self.profile.float_to_int {
            FloatToInt::QvmIndefinite => Ok(qvm_float_to_int(value as f32)),
            FloatToInt::CheckedTruncation => checked_float_to_int(value),
            FloatToInt::X86Indefinite => Err(NumericError::UnsupportedProfile(self.profile.id.to_string())),
        }
    }
}

fn validate_byte_text(text: &str) -> Result<(), NumericError> {
    if text.chars().any(|c| c as u32 > 255) {
        return Err(NumericError::NonByteText);
    }
    Ok(())
}

fn is_space(byte: u8) -> bool {
    byte == 32 || (9..=13).contains(&byte)
}

/// Parse a source byte string with the observed i386 glibc `atoi` profile:
/// skips whitespace, reads an optional sign and decimal digits, saturates
/// at the `i32` limits on overflow, stops at the first NUL or non-digit.
pub fn native_atoi(text: &str) -> Result<i32, NumericError> {
    validate_byte_text(text)?;
    let bytes: Vec<u8> = text.chars().map(|c| c as u8).collect();
    let mut offset = 0;
    while offset < bytes.len() && is_space(bytes[offset]) {
        offset += 1;
    }
    let mut negative = false;
    if offset < bytes.len() && (bytes[offset] == b'+' || bytes[offset] == b'-') {
        negative = bytes[offset] == b'-';
        offset += 1;
    }
    let limit: u32 = if negative { 2_147_483_648 } else { 2_147_483_647 };
    let mut magnitude: u32 = 0;
    let mut overflow = false;
    while offset < bytes.len() {
        let byte = bytes[offset];
        if byte == 0 || !byte.is_ascii_digit() {
            break;
        }
        let digit = u32::from(byte - b'0');
        if !overflow {
            if magnitude > (limit - digit) / 10 {
                magnitude = limit;
                overflow = true;
            } else {
                magnitude = magnitude * 10 + digit;
            }
        }
        offset += 1;
    }
    if magnitude == 0 {
        return Ok(0);
    }
    if negative {
        Ok(if magnitude == 2_147_483_648 {
            i32::MIN
        } else {
            -(magnitude as i32)
        })
    } else {
        Ok(magnitude as i32)
    }
}

fn lower_ascii(byte: u8) -> u8 {
    if byte.is_ascii_uppercase() {
        byte + 32
    } else {
        byte
    }
}

fn matches_ascii_word(bytes: &[u8], offset: usize, word: &[u8]) -> bool {
    if offset + word.len() > bytes.len() {
        return false;
    }
    for (index, expected) in word.iter().enumerate() {
        let byte = bytes[offset + index];
        if byte == 0 || lower_ascii(byte) != *expected {
            return false;
        }
    }
    true
}

fn double_from_bits(bits: u64) -> f64 {
    f64::from_bits(bits)
}

fn nan_with_payload(negative: bool, payload: u64) -> f64 {
    let sign = if negative { 1 << 63 } else { 0 };
    let fraction = (1 << 51) | (payload & ((1 << 51) - 1));
    double_from_bits(sign | (0x7ff << 52) | fraction)
}

fn digit_value(byte: u8, radix: u32) -> i32 {
    let value = if byte.is_ascii_digit() {
        i32::from(byte - b'0')
    } else if (b'a'..=b'f').contains(&byte) {
        i32::from(byte - b'a') + 10
    } else if (b'A'..=b'F').contains(&byte) {
        i32::from(byte - b'A') + 10
    } else {
        -1
    };
    if value >= 0 && (value as u32) < radix {
        value
    } else {
        -1
    }
}

fn parse_nan_payload(bytes: &[u8], offset: usize) -> u64 {
    if bytes.get(offset) != Some(&b'(') {
        return 0;
    }
    let mut end = offset + 1;
    while end < bytes.len() {
        let byte = bytes[end];
        let payload_char = byte.is_ascii_alphanumeric() || byte == b'_';
        if !payload_char {
            break;
        }
        end += 1;
    }
    if bytes.get(end) != Some(&b')') || end == offset + 1 {
        return 0;
    }
    let mut cursor = offset + 1;
    let mut radix: u64 = 10;
    if bytes.get(cursor) == Some(&b'0') {
        let next = bytes.get(cursor + 1).copied().unwrap_or(0);
        let after = bytes.get(cursor + 2).copied().unwrap_or(0);
        if (next == b'x' || next == b'X') && digit_value(after, 16) >= 0 {
            radix = 16;
            cursor += 2;
        } else {
            radix = 8;
        }
    }
    let mut payload: u64 = 0;
    let mut saw_digit = false;
    while cursor < end {
        let digit = digit_value(bytes[cursor], radix as u32);
        if digit < 0 {
            return 0;
        }
        saw_digit = true;
        payload = payload.saturating_mul(radix).saturating_add(digit as u64);
        cursor += 1;
    }
    if saw_digit {
        payload
    } else {
        0
    }
}

/// Exact hexadecimal significand: top 128 significant bits, total
/// significant-bit count, OR of dropped low bits, and value mod 2^64.
struct HexSignificand {
    top: u128,
    total_bits: u64,
    sticky: bool,
    low: u64,
}

impl HexSignificand {
    /// Exact bit `index` (0 = LSB) for values with at most 192 significant
    /// bits; wider literals keep only the OR of the middle band.
    fn bit(&self, index: u64) -> bool {
        if index >= self.total_bits {
            return false;
        }
        if self.total_bits <= 128 {
            return (self.top >> index) & 1 == 1;
        }
        let below = self.total_bits - 128;
        if index >= below {
            return (self.top >> (index - below)) & 1 == 1;
        }
        if index < 64 {
            return (self.low >> index) & 1 == 1;
        }
        false
    }

    /// OR of bits in `[0, end)`; exact unless the literal carries more than
    /// 192 significant bits with set bits only in the unknown middle band.
    fn or_below(&self, end: u64) -> bool {
        if self.total_bits <= 128 {
            if end >= 128 {
                return self.top != 0;
            }
            let mask = if end == 0 { 0 } else { (1 << end) - 1 };
            return self.top & mask != 0;
        }
        let below = self.total_bits - 128;
        let known_end = end.min(64);
        if known_end > 0 {
            let mask = if known_end == 64 {
                u64::MAX
            } else {
                (1 << known_end) - 1
            };
            if self.low & mask != 0 {
                return true;
            }
        }
        if end > below {
            let acc_end = (end - below).min(128);
            let mask = if acc_end >= 128 { u128::MAX } else { (1 << acc_end) - 1 };
            if self.top & mask != 0 {
                return true;
            }
        }
        end > 64 && self.sticky
    }
}

fn hex_significand(digits: &[u8]) -> HexSignificand {
    let mut significant = digits.iter().copied().skip_while(|digit| *digit == 0);
    let Some(first) = significant.next() else {
        return HexSignificand {
            top: 0,
            total_bits: 0,
            sticky: false,
            low: 0,
        };
    };
    let norm = first.leading_zeros() - 4;
    let mut total = u64::from(4 - norm);
    let mut top: u128 = u128::from(first);
    let mut stored_digits: u32 = 1;
    let mut low: u64 = u64::from(first);
    let mut sticky = false;
    let mut first_dropped: Option<u8> = None;
    for digit in significant {
        total += 4;
        low = low.wrapping_mul(16).wrapping_add(u64::from(digit));
        if stored_digits < 32 {
            top = (top << 4) | u128::from(digit);
            stored_digits += 1;
        } else if first_dropped.is_none() {
            first_dropped = Some(digit);
        } else {
            sticky |= digit != 0;
        }
    }
    if stored_digits == 32 {
        if let Some(digit) = first_dropped {
            top <<= norm;
            if norm > 0 {
                top |= u128::from(digit >> (4 - norm));
                sticky |= digit & ((1 << (4 - norm)) - 1) != 0;
            } else {
                sticky |= digit != 0;
            }
        }
    }
    HexSignificand {
        top,
        total_bits: total,
        sticky,
        low,
    }
}

fn finite_double(negative: bool, exponent: u32, fraction: u64) -> f64 {
    let sign = if negative { 1 << 63 } else { 0 };
    double_from_bits(sign | (u64::from(exponent) << 52) | fraction)
}

fn convert_hex(significand: &HexSignificand, exponent2: i128, negative: bool) -> f64 {
    if significand.total_bits == 0 {
        return if negative { -0.0 } else { 0.0 };
    }
    let total = significand.total_bits as i128;
    let binary_exponent = total - 1 + exponent2;
    if binary_exponent > 1023 {
        return if negative { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    if binary_exponent < -1075 {
        return if negative { -0.0 } else { 0.0 };
    }
    if binary_exponent >= -1022 {
        let shift = total - 53;
        let mut rounded: u64;
        if shift <= 0 {
            // A non-positive shift implies at most 53 significant bits, so the
            // accumulator holds the whole significand.
            rounded = (significand.top << ((-shift) as u32)) as u64;
        } else {
            let below = (total as u64).saturating_sub(128);
            let shift_u64 = shift as u64;
            let quotient = if total <= 128 {
                (significand.top >> shift_u64) as u64
            } else {
                (significand.top >> (shift_u64 - below)) as u64
            };
            let round_bit = significand.bit(shift_u64 - 1);
            let sticky = significand.or_below(shift_u64 - 1);
            rounded = quotient;
            if round_bit && (sticky || quotient & 1 == 1) {
                rounded = quotient + 1;
            }
        }
        let mut exponent = binary_exponent;
        if rounded == 1 << 53 {
            rounded >>= 1;
            exponent += 1;
            if exponent > 1023 {
                return if negative { f64::NEG_INFINITY } else { f64::INFINITY };
            }
        }
        return finite_double(negative, (exponent + 1023) as u32, rounded - (1 << 52));
    }
    let subnormal_shift = exponent2 + 1074;
    if subnormal_shift >= 0 {
        let shift = subnormal_shift as u32;
        if shift > 51 {
            return if negative { -0.0 } else { 0.0 };
        }
        let fraction = ((significand.low % (1 << (52 - shift))) << shift) & ((1 << 52) - 1);
        if fraction == 0 {
            return if negative { -0.0 } else { 0.0 };
        }
        if fraction == 1 << 52 {
            return finite_double(negative, 1, 0);
        }
        return finite_double(negative, 0, fraction);
    }
    let shift = (-subnormal_shift) as u64;
    if shift >= significand.total_bits {
        return if negative { -0.0 } else { 0.0 };
    }
    let below = (total as u64).saturating_sub(128);
    let quotient = if total <= 128 {
        (significand.top >> shift) as u64
    } else {
        (significand.top >> (shift - below)) as u64
    };
    let round_bit = significand.bit(shift - 1);
    let sticky = significand.or_below(shift - 1);
    let mut fraction = quotient;
    if round_bit && (sticky || quotient & 1 == 1) {
        fraction = quotient + 1;
    }
    if fraction == 0 {
        return if negative { -0.0 } else { 0.0 };
    }
    if fraction == 1 << 52 {
        return finite_double(negative, 1, 0);
    }
    finite_double(negative, 0, fraction & ((1 << 52) - 1))
}

fn parse_signed_decimal_exponent(bytes: &[u8], offset: usize) -> i128 {
    let mut cursor = offset;
    let mut negative = false;
    if matches!(bytes.get(cursor), Some(b'+') | Some(b'-')) {
        negative = bytes[cursor] == b'-';
        cursor += 1;
    }
    let mut value: i128 = 0;
    while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
        value = value
            .saturating_mul(10)
            .saturating_add(i128::from(bytes[cursor] - b'0'));
        cursor += 1;
    }
    if negative {
        -value
    } else {
        value
    }
}

fn parse_hex(bytes: &[u8], offset: usize, negative: bool) -> Option<f64> {
    if bytes.get(offset) != Some(&b'0') {
        return None;
    }
    let marker = bytes.get(offset + 1).copied().unwrap_or(0);
    if marker != b'x' && marker != b'X' {
        return None;
    }
    let mut cursor = offset + 2;
    let mut digits: Vec<u8> = Vec::new();
    let mut fraction_digits: u64 = 0;
    let mut after_point = false;
    let mut saw_point = false;
    let mut saw_digit = false;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        let digit = digit_value(byte, 16);
        if digit >= 0 {
            digits.push(digit as u8);
            if after_point {
                fraction_digits += 1;
            }
            saw_digit = true;
            cursor += 1;
        } else if byte == b'.' && !saw_point {
            saw_point = true;
            after_point = true;
            cursor += 1;
        } else {
            break;
        }
    }
    if !saw_digit {
        return None;
    }
    let mut parsed_exponent: i128 = 0;
    if matches!(bytes.get(cursor), Some(b'p') | Some(b'P')) {
        let mut exponent_digits = cursor + 1;
        if matches!(bytes.get(exponent_digits), Some(b'+') | Some(b'-')) {
            exponent_digits += 1;
        }
        if bytes.get(exponent_digits).is_some_and(u8::is_ascii_digit) {
            parsed_exponent = parse_signed_decimal_exponent(bytes, cursor + 1);
        }
    }
    let significand = hex_significand(&digits);
    Some(convert_hex(
        &significand,
        parsed_exponent - i128::from(fraction_digits) * 4,
        negative,
    ))
}

/// Parse a C-locale source byte string with the observed glibc `atof`
/// profile: skips whitespace, reads an optional sign, `inf`/`nan` words
/// (with optional payload), hexadecimal floats, or a decimal mantissa with
/// an optional exponent. Returns `0` when no digits follow.
pub fn native_atof(text: &str) -> Result<f64, NumericError> {
    validate_byte_text(text)?;
    let bytes: Vec<u8> = text.chars().map(|c| c as u8).collect();
    let mut offset = 0;
    while offset < bytes.len() && is_space(bytes[offset]) {
        offset += 1;
    }
    let signed_start = offset;
    let mut negative = false;
    if matches!(bytes.get(offset), Some(b'+') | Some(b'-')) {
        negative = bytes[offset] == b'-';
        offset += 1;
    }
    if matches_ascii_word(&bytes, offset, b"inf") {
        return Ok(if negative { f64::NEG_INFINITY } else { f64::INFINITY });
    }
    if matches_ascii_word(&bytes, offset, b"nan") {
        return Ok(nan_with_payload(negative, parse_nan_payload(&bytes, offset + 3)));
    }
    if let Some(value) = parse_hex(&bytes, offset, negative) {
        return Ok(value);
    }
    let mut cursor = offset;
    let mut saw_digit = false;
    while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
        saw_digit = true;
        cursor += 1;
    }
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            saw_digit = true;
            cursor += 1;
        }
    }
    if !saw_digit {
        return Ok(0.0);
    }
    if matches!(bytes.get(cursor), Some(b'e') | Some(b'E')) {
        let mut exponent_digits = cursor + 1;
        if matches!(bytes.get(exponent_digits), Some(b'+') | Some(b'-')) {
            exponent_digits += 1;
        }
        if bytes.get(exponent_digits).is_some_and(u8::is_ascii_digit) {
            cursor = exponent_digits + 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
        }
    }
    let slice: String = bytes[signed_start..cursor].iter().map(|b| *b as char).collect();
    Ok(slice.parse::<f64>().unwrap_or(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qvm_float_to_int_matches_qsrc_edges() {
        assert_eq!(qvm_float_to_int(1.9), 1);
        assert_eq!(qvm_float_to_int(-1.9), -1);
        assert_eq!(qvm_float_to_int(-0.5), 0);
        assert_eq!(qvm_float_to_int(f32::NAN), i32::MIN);
        assert_eq!(qvm_float_to_int(f32::INFINITY), i32::MIN);
        assert_eq!(qvm_float_to_int(-2_147_483_648.0), i32::MIN);
        assert_eq!(qvm_float_to_int(2_147_483_647.0), i32::MIN);
        assert_eq!(qvm_float_to_int(2_147_483_648.0), i32::MIN);
        assert_eq!(qvm_float_to_int(2_147_483_520.0), 2_147_483_520);
    }

    #[test]
    fn checked_float_to_int_rejects_edges() {
        assert_eq!(checked_float_to_int(1.9), Ok(1));
        assert_eq!(checked_float_to_int(-0.5), Ok(0));
        assert_eq!(checked_float_to_int(f64::NAN), Err(NumericError::IntRange));
        assert_eq!(checked_float_to_int(2_147_483_648.0), Err(NumericError::IntRange));
        assert_eq!(checked_float_to_int(-2_147_483_649.0), Err(NumericError::IntRange));
        assert_eq!(checked_float_to_int(2_147_483_647.0), Ok(2_147_483_647));
    }

    #[test]
    fn q_rand_sequence_matches_qsrc() {
        let mut seed = 1;
        let mut values = Vec::new();
        for _ in 0..4 {
            seed = q_rand(seed);
            values.push(seed);
        }
        assert_eq!(values, [69_070, 475_628_535, -1_017_563_188, 772_999_773]);
        let step = q_random(1);
        assert_eq!(step.seed, 69_070);
        assert!((step.value - f64::from(69_070 & 0xffff) / 65_536.0).abs() < f64::EPSILON);
        let centered = q_crandom(1);
        assert!((-1.0..1.0).contains(&centered.value));
    }

    #[test]
    fn numeric_ops_follow_selected_profile() {
        let binary32 = NumericOps::select(Q3_BINARY32_PROFILE).unwrap();
        assert_eq!(binary32.add(16_777_216.0, 1.0), 16_777_216.0);
        let q1 = NumericOps::select(Q1_DONOR_PROFILE).unwrap();
        assert_eq!(q1.add(16_777_216.0, 1.0), 16_777_216.0);
        let q2 = NumericOps::select(Q2_DONOR_PROFILE).unwrap();
        assert_eq!(q2.mul(16_777_216.0, 1.1), 18_454_938.0);
        assert_eq!(binary32.to_int32(f64::NAN), Ok(i32::MIN));
        assert_eq!(q1.to_int32(f64::NAN), Err(NumericError::IntRange));
        let x87 = NumericProfile {
            id: "native:x87",
            arithmetic: Arithmetic::X87 {
                precision_bits: X87Precision::Bits64,
                rounding: Rounding::NearestEven,
            },
            float_to_int: FloatToInt::CheckedTruncation,
        };
        assert!(NumericOps::select(x87).is_err());
    }

    #[test]
    fn native_atoi_matches_glibc_profile() {
        assert_eq!(native_atoi("  -42x"), Ok(-42));
        assert_eq!(native_atoi("+17"), Ok(17));
        assert_eq!(native_atoi("9999999999"), Ok(2_147_483_647));
        assert_eq!(native_atoi("-9999999999"), Ok(i32::MIN));
        assert_eq!(native_atoi("12\x1c"), Ok(12));
        assert_eq!(native_atoi(""), Ok(0));
        assert_eq!(native_atoi("--5"), Ok(0));
        assert!(native_atoi("€.5").is_err());
    }

    #[test]
    fn native_atof_matches_glibc_profile() {
        assert_eq!(native_atof("  -1.5"), Ok(-1.5));
        assert_eq!(native_atof("1e3"), Ok(1000.0));
        assert_eq!(native_atof("INF"), Ok(f64::INFINITY));
        assert_eq!(native_atof("-infinity-and-more"), Ok(f64::NEG_INFINITY));
        assert!(native_atof("nan").unwrap().is_nan());
        assert!(native_atof("NAN(0x10)").unwrap().is_nan());
        assert_eq!(native_atof("0x1p4"), Ok(16.0));
        assert_eq!(native_atof("0x1.8p1"), Ok(3.0));
        assert_eq!(native_atof("no-digits"), Ok(0.0));
        assert_eq!(native_atof("-"), Ok(0.0));
        assert_eq!(native_atof("2.5trailing"), Ok(2.5));
        assert_eq!(native_atof("0x0.0000000000000000000000001p-126"), Ok(2f64.powi(-226)));
        assert_eq!(native_atof("0x1p-1100"), Ok(0.0));
        assert_eq!(native_atof("0x1p-1074"), Ok(5e-324));
        assert_eq!(native_atof("0x1p1024"), Ok(f64::INFINITY));
    }
}
