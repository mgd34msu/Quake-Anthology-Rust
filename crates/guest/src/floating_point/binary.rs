//! Exact binary floating-point arithmetic over arbitrary precision.
//!
//! Donor: `src/guest/floating-point/binary.ts`. Values decode to exact
//! coefficient/exponent pairs, arithmetic runs on exact rationals, and a
//! single rounding step produces the result with x87/SSE status flags. The
//! donor's `bigint` becomes the [`BigInt`] below (sign-magnitude, base
//! 2^32); all helpers preserve the donor's bit-level semantics.

use std::cmp::Ordering;

use crate::error::GuestError;

/// Invalid-operation flag.
pub const FLAG_INVALID: u32 = 1;
/// Denormal-operand flag.
pub const FLAG_DENORMAL_OPERAND: u32 = 2;
/// Zero-divide flag.
pub const FLAG_ZERO_DIVIDE: u32 = 4;
/// Overflow flag.
pub const FLAG_OVERFLOW: u32 = 8;
/// Underflow flag.
pub const FLAG_UNDERFLOW: u32 = 16;
/// Precision flag.
pub const FLAG_PRECISION: u32 = 32;

/// Rounding direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rounding {
    /// Round to nearest, ties to even.
    Nearest,
    /// Round toward negative infinity.
    Down,
    /// Round toward positive infinity.
    Up,
    /// Round toward zero.
    Zero,
}

/// Decode a 2-bit rounding-control field.
#[must_use]
pub const fn rounding(bits: u32) -> Rounding {
    match bits & 3 {
        0 => Rounding::Nearest,
        1 => Rounding::Down,
        2 => Rounding::Up,
        _ => Rounding::Zero,
    }
}

/// Binary interchange width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryWidth {
    /// 32-bit single.
    W32,
    /// 64-bit double.
    W64,
    /// 80-bit extended.
    W80,
}

impl BinaryWidth {
    /// Width in bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        match self {
            Self::W32 => 32,
            Self::W64 => 64,
            Self::W80 => 80,
        }
    }
}

/// Rounding format: precision plus exponent range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinaryFormat {
    /// Significand precision in bits.
    pub precision: i32,
    /// Minimum binary exponent.
    pub minimum_exponent: i32,
    /// Maximum binary exponent.
    pub maximum_exponent: i32,
}

/// Binary32 format.
pub const BINARY32: BinaryFormat = BinaryFormat {
    precision: 24,
    minimum_exponent: -126,
    maximum_exponent: 127,
};
/// Binary64 format.
pub const BINARY64: BinaryFormat = BinaryFormat {
    precision: 53,
    minimum_exponent: -1022,
    maximum_exponent: 1023,
};
/// Binary80 format.
pub const BINARY80: BinaryFormat = BinaryFormat {
    precision: 64,
    minimum_exponent: -16382,
    maximum_exponent: 16383,
};

/// Format for an interchange width.
#[must_use]
pub const fn format_for(width: BinaryWidth) -> BinaryFormat {
    match width {
        BinaryWidth::W32 => BINARY32,
        BinaryWidth::W64 => BINARY64,
        BinaryWidth::W80 => BINARY80,
    }
}

/// Sign-magnitude arbitrary-precision integer (little-endian base-2^32 limbs).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BigInt {
    negative: bool,
    limbs: Vec<u32>,
}

impl BigInt {
    /// Zero.
    #[must_use]
    pub fn zero() -> Self {
        Self::default()
    }

    /// One.
    #[must_use]
    pub fn one() -> Self {
        Self::from_u64(1)
    }

    /// Value from `u64`.
    #[must_use]
    pub fn from_u64(value: u64) -> Self {
        let mut result = Self::default();
        if value != 0 {
            result.limbs.push(value as u32);
            let high = (value >> 32) as u32;
            if high != 0 {
                result.limbs.push(high);
            }
        }
        result
    }

    /// Value from `u128`.
    #[must_use]
    pub fn from_u128(value: u128) -> Self {
        let mut result = Self::from_u64(value as u64);
        let high = Self::from_u64((value >> 64) as u64).shl_bits(64);
        result = result.add(&high);
        result
    }

    /// Value from `i64`.
    #[must_use]
    pub fn from_i64(value: i64) -> Self {
        if value >= 0 {
            Self::from_u64(value as u64)
        } else {
            let mut result = Self::from_u64(value.unsigned_abs());
            result.negative = true;
            result
        }
    }

    /// Value from little-endian bytes.
    #[must_use]
    pub fn from_bytes_le(bytes: &[u8]) -> Self {
        let mut result = Self::default();
        for chunk in bytes.chunks(4) {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            result.limbs.push(u32::from_le_bytes(word));
        }
        result.normalize();
        result
    }

    /// Whether the value is zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    /// Whether the value is negative.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.negative && !self.is_zero()
    }

    /// Bit length (0 for zero).
    #[must_use]
    pub fn bit_len(&self) -> usize {
        let Some(high) = self.limbs.last() else {
            return 0;
        };
        (self.limbs.len() - 1) * 32 + (32 - high.leading_zeros() as usize)
    }

    /// Low 64 bits.
    #[must_use]
    pub fn low_u64(&self) -> u64 {
        let low = self.limbs.first().copied().unwrap_or(0);
        let high = self.limbs.get(1).copied().unwrap_or(0);
        u64::from(low) | (u64::from(high) << 32)
    }

    /// Value masked to `bits` bits (unsigned wrap).
    #[must_use]
    pub fn as_uint_n(&self, bits: u32) -> Self {
        if bits == 0 {
            return Self::zero();
        }
        let full_limbs = (bits / 32) as usize;
        let spare = bits % 32;
        let mut limbs: Vec<u32> = self.limbs.iter().take(full_limbs).copied().collect();
        if spare != 0 {
            if let Some(limb) = self.limbs.get(full_limbs) {
                limbs.push(limb & ((1u32 << spare) - 1));
            }
        }
        // A negative donor BigInt masked with asUintN wraps modulo 2^bits.
        if self.is_negative() {
            let modulus = Self::one().shl_bits(bits);
            let magnitude = Self {
                negative: false,
                limbs,
            };
            return modulus.sub(&magnitude).as_uint_n(bits);
        }
        let mut result = Self {
            negative: false,
            limbs,
        };
        result.normalize();
        result
    }

    /// Value wrapped to a signed `bits`-bit range.
    #[must_use]
    pub fn as_int_n(&self, bits: u32) -> Self {
        let wrapped = self.as_uint_n(bits);
        if bits == 0 || !wrapped.bit(bits - 1) {
            return wrapped;
        }
        let modulus = Self::one().shl_bits(bits);
        modulus.sub(&wrapped).negated()
    }

    /// Signed 64-bit value after `bits`-bit wrap.
    #[must_use]
    pub fn as_int_n_i64(&self, bits: u32) -> i64 {
        let wrapped = self.as_int_n(bits);
        if wrapped.is_negative() {
            -(wrapped.low_u64() as i64)
        } else {
            wrapped.low_u64() as i64
        }
    }

    /// Test bit `index`.
    #[must_use]
    pub fn bit(&self, index: u32) -> bool {
        self.limbs
            .get((index / 32) as usize)
            .is_some_and(|limb| limb & (1 << (index % 32)) != 0)
    }

    /// Set bit `index`.
    pub fn set_bit(&mut self, index: u32) {
        let limb = (index / 32) as usize;
        if self.limbs.len() <= limb {
            self.limbs.resize(limb + 1, 0);
        }
        self.limbs[limb] |= 1 << (index % 32);
        self.normalize();
    }

    /// Logical left shift by `bits`.
    #[must_use]
    pub fn shl_bits(&self, bits: u32) -> Self {
        if self.is_zero() {
            return Self::zero();
        }
        let limb_shift = (bits / 32) as usize;
        let bit_shift = bits % 32;
        let mut limbs = vec![0; self.limbs.len() + limb_shift + 1];
        let mut carry = 0u64;
        for (index, limb) in self.limbs.iter().enumerate() {
            let wide = (u64::from(*limb) << bit_shift) | carry;
            limbs[index + limb_shift] = wide as u32;
            carry = wide >> 32;
        }
        limbs[self.limbs.len() + limb_shift] = carry as u32;
        let mut result = Self {
            negative: self.negative,
            limbs,
        };
        result.normalize();
        result
    }

    /// Logical right shift by `bits` (toward zero for negatives).
    #[must_use]
    pub fn shr_bits(&self, bits: u32) -> Self {
        let limb_shift = (bits / 32) as usize;
        let bit_shift = bits % 32;
        if self.limbs.len() <= limb_shift {
            return Self::zero();
        }
        let mut limbs = Vec::with_capacity(self.limbs.len() - limb_shift);
        let mut carry = 0u32;
        for limb in self.limbs.iter().skip(limb_shift).rev() {
            let wide = (u64::from(carry) << 32) | u64::from(*limb);
            limbs.push((wide >> bit_shift) as u32);
            carry = if bit_shift == 0 {
                0
            } else {
                (*limb & ((1 << bit_shift) - 1)) << (32 - bit_shift)
            };
        }
        limbs.reverse();
        let mut result = Self {
            negative: self.negative,
            limbs,
        };
        result.normalize();
        result
    }

    /// Negation.
    #[must_use]
    pub fn negated(&self) -> Self {
        let mut result = self.clone();
        if !result.is_zero() {
            result.negative = !result.negative;
        }
        result
    }

    /// Absolute value.
    #[must_use]
    pub fn abs(&self) -> Self {
        let mut result = self.clone();
        result.negative = false;
        result
    }

    /// Signed addition.
    #[must_use]
    pub fn add(&self, other: &Self) -> Self {
        if self.negative == other.negative {
            let mut result = Self {
                negative: self.negative,
                limbs: Self::add_mag(&self.limbs, &other.limbs),
            };
            result.normalize();
            result
        } else if Self::cmp_mag(&self.limbs, &other.limbs) != Ordering::Less {
            let mut result = Self {
                negative: self.negative,
                limbs: Self::sub_mag(&self.limbs, &other.limbs),
            };
            result.normalize();
            result
        } else {
            let mut result = Self {
                negative: other.negative,
                limbs: Self::sub_mag(&other.limbs, &self.limbs),
            };
            result.normalize();
            result
        }
    }

    /// Signed subtraction.
    #[must_use]
    pub fn sub(&self, other: &Self) -> Self {
        self.add(&other.negated())
    }

    /// Signed multiplication.
    #[must_use]
    pub fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return Self::zero();
        }
        let mut limbs = vec![0u32; self.limbs.len() + other.limbs.len()];
        for (i, a) in self.limbs.iter().enumerate() {
            let mut carry = 0u64;
            for (j, b) in other.limbs.iter().enumerate() {
                let wide = u64::from(*a) * u64::from(*b) + u64::from(limbs[i + j]) + carry;
                limbs[i + j] = wide as u32;
                carry = wide >> 32;
            }
            let mut k = i + other.limbs.len();
            while carry != 0 {
                let wide = u64::from(limbs[k]) + carry;
                limbs[k] = wide as u32;
                carry = wide >> 32;
                k += 1;
            }
        }
        let mut result = Self {
            negative: self.negative != other.negative,
            limbs,
        };
        result.normalize();
        result
    }

    /// Truncated division with remainder. Signs follow the donor (`bigint`
    /// division truncates toward zero; the remainder takes the dividend sign).
    pub fn divmod(&self, other: &Self) -> Result<(Self, Self), GuestError> {
        if other.is_zero() {
            return Err(GuestError::cpu("Integer division by zero in guest FP arithmetic"));
        }
        let (mut quotient, mut remainder) = Self::divmod_mag(&self.limbs, &other.limbs);
        quotient.negative = self.negative != other.negative;
        remainder.negative = self.negative;
        quotient.normalize();
        remainder.normalize();
        Ok((quotient, remainder))
    }

    /// Truncated quotient.
    pub fn div(&self, other: &Self) -> Result<Self, GuestError> {
        Ok(self.divmod(other)?.0)
    }

    /// Remainder (dividend sign).
    pub fn rem(&self, other: &Self) -> Result<Self, GuestError> {
        Ok(self.divmod(other)?.1)
    }

    /// Little-endian byte image of exactly `len` bytes (low bytes kept).
    #[must_use]
    pub fn to_bytes_le(&self, len: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; len];
        for (index, limb) in self.limbs.iter().enumerate() {
            let offset = index * 4;
            if offset >= len {
                break;
            }
            let chunk = &limb.to_le_bytes();
            let take = (len - offset).min(4);
            bytes[offset..offset + take].copy_from_slice(&chunk[..take]);
        }
        bytes
    }

    fn normalize(&mut self) {
        while self.limbs.last() == Some(&0) {
            self.limbs.pop();
        }
        if self.limbs.is_empty() {
            self.negative = false;
        }
    }

    fn cmp_mag(left: &[u32], right: &[u32]) -> Ordering {
        let (left_len, right_len) = (mag_len(left), mag_len(right));
        if left_len != right_len {
            return left_len.cmp(&right_len);
        }
        for (a, b) in left.iter().zip(right.iter()).rev() {
            if a != b {
                return a.cmp(b);
            }
        }
        Ordering::Equal
    }

    fn add_mag(left: &[u32], right: &[u32]) -> Vec<u32> {
        let mut limbs = Vec::with_capacity(left.len().max(right.len()) + 1);
        let mut carry = 0u64;
        for index in 0..left.len().max(right.len()) {
            let wide = u64::from(left.get(index).copied().unwrap_or(0))
                + u64::from(right.get(index).copied().unwrap_or(0))
                + carry;
            limbs.push(wide as u32);
            carry = wide >> 32;
        }
        if carry != 0 {
            limbs.push(carry as u32);
        }
        limbs
    }

    /// Magnitude subtraction, requiring `left >= right`.
    fn sub_mag(left: &[u32], right: &[u32]) -> Vec<u32> {
        let mut limbs = Vec::with_capacity(left.len());
        let mut borrow = 0i64;
        for index in 0..left.len() {
            let wide = i64::from(left[index]) - i64::from(right.get(index).copied().unwrap_or(0))
                - borrow;
            if wide < 0 {
                limbs.push((wide + 0x1_0000_0000) as u32);
                borrow = 1;
            } else {
                limbs.push(wide as u32);
                borrow = 0;
            }
        }
        debug_assert_eq!(borrow, 0, "magnitude subtraction underflow");
        limbs
    }

    /// Binary long division over magnitudes.
    fn divmod_mag(left: &[u32], right: &[u32]) -> (Self, Self) {
        let mut quotient = Self::zero();
        let mut remainder = Self::zero();
        let left_bits = mag_bits(left);
        for index in (0..left_bits).rev() {
            remainder = remainder.shl_bits(1);
            if left[(index / 32) as usize] & (1 << (index % 32)) != 0 {
                if remainder.limbs.is_empty() {
                    remainder.limbs.push(0);
                }
                remainder.limbs[0] |= 1;
            }
            if Self::cmp_mag(&remainder.limbs, right) != Ordering::Less {
                remainder.limbs = Self::sub_mag(&remainder.limbs, right);
                remainder.normalize();
                quotient.set_bit(index);
            }
        }
        (quotient, remainder)
    }
}

impl PartialOrd for BigInt {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BigInt {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.is_negative(), other.is_negative()) {
            (true, false) => {
                if self.is_zero() && other.is_zero() {
                    Ordering::Equal
                } else {
                    Ordering::Less
                }
            }
            (false, true) => {
                if self.is_zero() && other.is_zero() {
                    Ordering::Equal
                } else {
                    Ordering::Greater
                }
            }
            (false, false) => BigInt::cmp_mag(&self.limbs, &other.limbs),
            (true, true) => BigInt::cmp_mag(&other.limbs, &self.limbs),
        }
    }
}

fn mag_len(limbs: &[u32]) -> usize {
    let mut len = limbs.len();
    while len > 0 && limbs[len - 1] == 0 {
        len -= 1;
    }
    len
}

fn mag_bits(limbs: &[u32]) -> u32 {
    let len = mag_len(limbs);
    if len == 0 {
        return 0;
    }
    ((len - 1) * 32 + (32 - limbs[len - 1].leading_zeros() as usize)) as u32
}

/// Exact decoded value: finite coefficient/exponent, infinity, NaN payload,
/// or an 80-bit unsupported encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryValue {
    /// Finite value: `sign * coefficient * 2^exponent`.
    Finite {
        /// Negative sign.
        negative: bool,
        /// Non-negative coefficient.
        coefficient: BigInt,
        /// Binary exponent.
        exponent: i32,
        /// Denormal source encoding.
        denormal: bool,
    },
    /// Infinity.
    Infinity {
        /// Negative sign.
        negative: bool,
    },
    /// NaN with a 63-bit payload (quiet bit at bit 62).
    Nan {
        /// Negative sign.
        negative: bool,
        /// Payload with the quiet bit at bit 62.
        payload: BigInt,
        /// Signaling NaN.
        signaling: bool,
    },
    /// 80-bit unsupported encoding (nonzero exponent, zero integer bit).
    Unsupported {
        /// Negative sign.
        negative: bool,
    },
}

impl BinaryValue {
    /// Value sign.
    #[must_use]
    pub const fn negative(&self) -> bool {
        match self {
            Self::Finite { negative, .. }
            | Self::Infinity { negative }
            | Self::Nan { negative, .. }
            | Self::Unsupported { negative } => *negative,
        }
    }
}

/// Rounded result with status flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryResult {
    /// Rounded value.
    pub value: BinaryValue,
    /// Status flags.
    pub flags: u32,
    /// Whether rounding moved away from truncation.
    pub rounded_up: bool,
}

/// Signed zero.
#[must_use]
pub fn zero(negative: bool) -> BinaryValue {
    BinaryValue::Finite {
        negative,
        coefficient: BigInt::zero(),
        exponent: 0,
        denormal: false,
    }
}

/// x87 real indefinite NaN.
#[must_use]
pub fn indefinite() -> BinaryValue {
    BinaryValue::Nan {
        negative: true,
        payload: BigInt::one().shl_bits(62),
        signaling: false,
    }
}

/// Exact value of an integer.
#[must_use]
pub fn from_integer(value: &BigInt) -> BinaryValue {
    BinaryValue::Finite {
        negative: value.is_negative(),
        coefficient: value.abs(),
        exponent: 0,
        denormal: false,
    }
}

/// Sign negation.
#[must_use]
pub fn negate(value: &BinaryValue) -> BinaryValue {
    match value.clone() {
        BinaryValue::Finite {
            negative,
            coefficient,
            exponent,
            denormal,
        } => BinaryValue::Finite {
            negative: !negative,
            coefficient,
            exponent,
            denormal,
        },
        BinaryValue::Infinity { negative } => BinaryValue::Infinity { negative: !negative },
        BinaryValue::Nan {
            negative,
            payload,
            signaling,
        } => BinaryValue::Nan {
            negative: !negative,
            payload,
            signaling,
        },
        BinaryValue::Unsupported { negative } => BinaryValue::Unsupported {
            negative: !negative,
        },
    }
}

/// Quiet a NaN (set the quiet bit, clear signaling).
#[must_use]
pub fn quiet(value: &BinaryValue) -> BinaryValue {
    match value {
        BinaryValue::Nan {
            negative,
            payload,
            ..
        } => BinaryValue::Nan {
            negative: *negative,
            payload: payload.add(&BigInt::one().shl_bits(62)),
            signaling: false,
        },
        _ => value.clone(),
    }
}

/// Read little-endian bytes as an unsigned integer.
#[must_use]
pub fn read_bits(bytes: &[u8]) -> BigInt {
    BigInt::from_bytes_le(bytes)
}

/// Write an unsigned integer as little-endian bytes.
#[must_use]
pub fn write_bits(value: &BigInt, byte_length: usize) -> Vec<u8> {
    value.as_uint_n(byte_length as u32 * 8).to_bytes_le(byte_length)
}

/// Decode an IEEE binary32 word without rounding its stored significand.
#[must_use]
pub fn decode_binary32(bits: u32) -> BinaryValue {
    let negative = bits >> 31 != 0;
    let exponent = (bits >> 23) & 255;
    let fraction = bits & 0x7f_ffff;
    if exponent == 255 {
        if fraction == 0 {
            return BinaryValue::Infinity { negative };
        }
        return BinaryValue::Nan {
            negative,
            payload: BigInt::from_u64(u64::from(fraction)).shl_bits(40),
            signaling: fraction & 0x40_0000 == 0,
        };
    }
    BinaryValue::Finite {
        negative,
        coefficient: BigInt::from_u64(u64::from(if exponent == 0 {
            fraction
        } else {
            fraction | 0x80_0000
        })),
        exponent: (if exponent == 0 { 1 } else { exponent }) as i32 - 150,
        denormal: exponent == 0 && fraction != 0,
    }
}

/// Decode an interchange encoding of `width` bits.
#[must_use]
pub fn decode_binary(bits: &BigInt, width: BinaryWidth) -> BinaryValue {
    let fraction_bits = match width {
        BinaryWidth::W32 => 23,
        BinaryWidth::W64 => 52,
        BinaryWidth::W80 => 63,
    };
    let storage_bits = if width == BinaryWidth::W80 {
        64
    } else {
        fraction_bits
    };
    let exponent_bits = match width {
        BinaryWidth::W32 => 8,
        BinaryWidth::W64 => 11,
        BinaryWidth::W80 => 15,
    };
    let exponent_mask = (1u32 << exponent_bits) - 1;
    let bias = (1 << (exponent_bits - 1)) - 1;
    let negative = bits.bit(width.bits() - 1);
    let exponent = bits.shr_bits(storage_bits).low_u64() as u32 & exponent_mask;
    let fraction = bits.as_uint_n(fraction_bits);
    let integer = if width == BinaryWidth::W80 {
        u64::from(bits.bit(63))
    } else if exponent == 0 {
        0
    } else {
        1
    };
    if width == BinaryWidth::W80 && exponent != 0 && integer == 0 {
        return BinaryValue::Unsupported { negative };
    }
    if exponent == exponent_mask {
        if fraction.is_zero() {
            return BinaryValue::Infinity { negative };
        }
        let payload = fraction.shl_bits(63 - fraction_bits);
        return BinaryValue::Nan {
            negative,
            signaling: !payload.bit(62),
            payload,
        };
    }
    let coefficient = if width == BinaryWidth::W80 && integer == 1 {
        fraction.add(&BigInt::one().shl_bits(63))
    } else if width != BinaryWidth::W80 && exponent != 0 {
        fraction.add(&BigInt::one().shl_bits(fraction_bits))
    } else {
        fraction
    };
    BinaryValue::Finite {
        negative,
        denormal: exponent == 0 && !coefficient.is_zero() && integer == 0,
        coefficient,
        exponent: (if exponent == 0 { 1 } else { exponent }) as i32 - bias - fraction_bits as i32,
    }
}

fn compare_scaled(left: &BigInt, right: &BigInt, shift: i32) -> Ordering {
    if shift >= 0 {
        left.shl_bits(shift as u32).cmp(right)
    } else {
        left.cmp(&right.shl_bits((-shift) as u32))
    }
}

fn ratio_exponent(numerator: &BigInt, denominator: &BigInt) -> i32 {
    let mut exponent = numerator.bit_len() as i32 - denominator.bit_len() as i32;
    if compare_scaled(numerator, denominator, -exponent) == Ordering::Less {
        exponent -= 1;
    }
    exponent
}

struct RoundedQuotient {
    quotient: BigInt,
    inexact: bool,
    up: bool,
}

fn rounded_quotient(
    numerator: &BigInt,
    denominator: &BigInt,
    negative: bool,
    mode: Rounding,
) -> Result<RoundedQuotient, GuestError> {
    let (quotient, remainder) = numerator.divmod(denominator)?;
    let inexact = !remainder.is_zero();
    let twice = remainder.add(&remainder);
    let up = inexact
        && match mode {
            Rounding::Nearest => {
                twice.cmp(denominator) == Ordering::Greater
                    || (twice == *denominator && quotient.bit(0))
            }
            Rounding::Up => !negative,
            Rounding::Down => negative,
            Rounding::Zero => false,
        };
    Ok(RoundedQuotient {
        quotient: if up {
            quotient.add(&BigInt::one())
        } else {
            quotient
        },
        inexact,
        up,
    })
}

/// Round an exact rational (`numerator/denominator * 2^exponent`) to `format`.
pub fn round_rational(
    negative: bool,
    numerator: &BigInt,
    denominator: &BigInt,
    exponent: i32,
    format: BinaryFormat,
    mode: Rounding,
) -> Result<BinaryResult, GuestError> {
    if numerator.is_zero() {
        return Ok(BinaryResult {
            value: zero(negative),
            flags: 0,
            rounded_up: false,
        });
    }
    let top = ratio_exponent(numerator, denominator) + exponent;
    let quantum = (top - format.precision + 1).max(format.minimum_exponent - format.precision + 1);
    let shift = exponent - quantum;
    let rounded = if shift >= 0 {
        rounded_quotient(&numerator.shl_bits(shift as u32), denominator, negative, mode)?
    } else {
        rounded_quotient(
            numerator,
            &denominator.shl_bits((-shift) as u32),
            negative,
            mode,
        )?
    };
    let result_top = rounded.quotient.bit_len() as i32 - 1 + quantum;
    if result_top > format.maximum_exponent {
        let infinite = mode == Rounding::Nearest
            || (mode == Rounding::Up && !negative)
            || (mode == Rounding::Down && negative);
        return Ok(BinaryResult {
            value: if infinite {
                BinaryValue::Infinity { negative }
            } else {
                BinaryValue::Finite {
                    negative,
                    coefficient: BigInt::one()
                        .shl_bits(format.precision as u32)
                        .sub(&BigInt::one()),
                    exponent: format.maximum_exponent - format.precision + 1,
                    denormal: false,
                }
            },
            flags: FLAG_OVERFLOW | FLAG_PRECISION,
            rounded_up: infinite,
        });
    }
    let tiny = result_top < format.minimum_exponent;
    Ok(BinaryResult {
        value: BinaryValue::Finite {
            negative,
            coefficient: rounded.quotient,
            exponent: quantum,
            denormal: tiny,
        }
        .normalized_denormal(),
        flags: if rounded.inexact {
            FLAG_PRECISION | if tiny { FLAG_UNDERFLOW } else { 0 }
        } else {
            0
        },
        rounded_up: rounded.up,
    })
}

trait NormalizedDenormal {
    fn normalized_denormal(self) -> Self;
}

impl NormalizedDenormal for BinaryValue {
    fn normalized_denormal(self) -> Self {
        match self {
            BinaryValue::Finite {
                negative,
                coefficient,
                exponent,
                denormal,
            } => BinaryValue::Finite {
                negative,
                exponent,
                denormal: denormal && !coefficient.is_zero(),
                coefficient,
            },
            _ => self,
        }
    }
}

/// Convert a value to `format` with `mode` rounding.
pub fn convert_binary(
    value: &BinaryValue,
    format: BinaryFormat,
    mode: Rounding,
) -> Result<BinaryResult, GuestError> {
    match value {
        BinaryValue::Unsupported { .. } => Ok(BinaryResult {
            value: indefinite(),
            flags: FLAG_INVALID,
            rounded_up: false,
        }),
        BinaryValue::Nan { signaling, .. } => Ok(BinaryResult {
            value: quiet(value),
            flags: if *signaling { FLAG_INVALID } else { 0 },
            rounded_up: false,
        }),
        BinaryValue::Infinity { .. } => Ok(BinaryResult {
            value: value.clone(),
            flags: 0,
            rounded_up: false,
        }),
        BinaryValue::Finite {
            negative,
            coefficient,
            exponent,
            ..
        } => round_rational(*negative, coefficient, &BigInt::one(), *exponent, format, mode),
    }
}

/// Encode a rounded value into an interchange width.
pub fn encode_binary(value: &BinaryValue, width: BinaryWidth) -> BigInt {
    let format = format_for(width);
    let fraction_bits = match width {
        BinaryWidth::W32 => 23,
        BinaryWidth::W64 => 52,
        BinaryWidth::W80 => 63,
    };
    let storage_bits = if width == BinaryWidth::W80 {
        64
    } else {
        fraction_bits
    };
    let exponent_mask: u32 = match width {
        BinaryWidth::W32 => 255,
        BinaryWidth::W64 => 2047,
        BinaryWidth::W80 => 32767,
    };
    let sign = if value.negative() {
        BigInt::one().shl_bits(width.bits() - 1)
    } else {
        BigInt::zero()
    };
    let explicit = if width == BinaryWidth::W80 {
        BigInt::one().shl_bits(63)
    } else {
        BigInt::zero()
    };
    match value {
        BinaryValue::Unsupported { .. } => return encode_binary(&indefinite(), width),
        BinaryValue::Infinity { .. } => {
            return sign.add(
                &BigInt::from_u64(u64::from(exponent_mask)).shl_bits(storage_bits),
            ).add(&explicit);
        }
        BinaryValue::Nan { payload, .. } => {
            let mut shifted = payload.shr_bits(63 - fraction_bits);
            if shifted.is_zero() {
                shifted = BigInt::one();
            }
            return sign
                .add(&BigInt::from_u64(u64::from(exponent_mask)).shl_bits(storage_bits))
                .add(&explicit)
                .add(&shifted);
        }
        BinaryValue::Finite {
            coefficient,
            exponent,
            ..
        } => {
            if coefficient.is_zero() {
                return sign;
            }
            let top = coefficient.bit_len() as i32 - 1 + exponent;
            let quantum = (top - fraction_bits as i32)
                .max(format.minimum_exponent - fraction_bits as i32);
            let shift = exponent - quantum;
            let significand = if shift >= 0 {
                coefficient.shl_bits(shift as u32)
            } else {
                coefficient.shr_bits((-shift) as u32)
            };
            let exponent_field = if top < format.minimum_exponent {
                0
            } else {
                (top + (1 - format.minimum_exponent)) as u32
            };
            let fraction = if width == BinaryWidth::W80 {
                significand
            } else {
                significand.as_uint_n(fraction_bits)
            };
            sign.add(&BigInt::from_u64(u64::from(exponent_field)).shl_bits(storage_bits))
                .add(&fraction)
        }
    }
}

fn propagate_nan(
    left: &BinaryValue,
    right: &BinaryValue,
    sse_selection: bool,
) -> Option<BinaryResult> {
    if matches!(left, BinaryValue::Unsupported { .. })
        || matches!(right, BinaryValue::Unsupported { .. })
    {
        return Some(BinaryResult {
            value: indefinite(),
            flags: FLAG_INVALID,
            rounded_up: false,
        });
    }
    let left_nan = matches!(left, BinaryValue::Nan { .. });
    let right_nan = matches!(right, BinaryValue::Nan { .. });
    if !left_nan && !right_nan {
        return None;
    }
    let left_signaling = matches!(left, BinaryValue::Nan { signaling: true, .. });
    let right_signaling = matches!(right, BinaryValue::Nan { signaling: true, .. });
    let flags = if left_signaling || right_signaling {
        FLAG_INVALID
    } else {
        0
    };
    let mut selected = if left_nan { left } else { right };
    if !sse_selection {
        if let (
            BinaryValue::Nan {
                payload: left_payload,
                signaling: left_sig,
                negative: left_neg,
            },
            BinaryValue::Nan {
                payload: right_payload,
                signaling: right_sig,
                negative: right_neg,
            },
        ) = (left, right)
        {
            if left_sig != right_sig {
                selected = if *left_sig { right } else { left };
            } else if right_payload > left_payload
                || (right_payload == left_payload && !right_neg && *left_neg)
            {
                selected = right;
            }
        }
    }
    Some(BinaryResult {
        value: quiet(selected),
        flags,
        rounded_up: false,
    })
}

/// Four basic operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOperation {
    /// Addition.
    Add,
    /// Subtraction.
    Subtract,
    /// Multiplication.
    Multiply,
    /// Division.
    Divide,
}

/// Exact four-function arithmetic with one rounding step.
pub fn arithmetic(
    operation: BinaryOperation,
    left: &BinaryValue,
    right: &BinaryValue,
    format: BinaryFormat,
    mode: Rounding,
    sse_selection: bool,
) -> Result<BinaryResult, GuestError> {
    if let Some(nan) = propagate_nan(left, right, sse_selection) {
        return Ok(nan);
    }
    if !matches!(
        left,
        BinaryValue::Finite { .. } | BinaryValue::Infinity { .. }
    ) || !matches!(
        right,
        BinaryValue::Finite { .. } | BinaryValue::Infinity { .. }
    ) {
        return Err(GuestError::cpu("Unresolved special operand"));
    }
    let source_flags = if matches!(left, BinaryValue::Finite { denormal: true, .. })
        || matches!(right, BinaryValue::Finite { denormal: true, .. })
    {
        FLAG_DENORMAL_OPERAND
    } else {
        0
    };
    let right_sign = if operation == BinaryOperation::Subtract {
        !right.negative()
    } else {
        right.negative()
    };
    if operation == BinaryOperation::Add || operation == BinaryOperation::Subtract {
        if matches!(left, BinaryValue::Infinity { .. })
            || matches!(right, BinaryValue::Infinity { .. })
        {
            if matches!(left, BinaryValue::Infinity { .. })
                && matches!(right, BinaryValue::Infinity { .. })
                && left.negative() != right_sign
            {
                return Ok(BinaryResult {
                    value: indefinite(),
                    flags: FLAG_INVALID | source_flags,
                    rounded_up: false,
                });
            }
            let value = if matches!(left, BinaryValue::Infinity { .. }) {
                left.clone()
            } else {
                BinaryValue::Infinity { negative: right_sign }
            };
            return Ok(BinaryResult {
                value,
                flags: source_flags,
                rounded_up: false,
            });
        }
        let (
            BinaryValue::Finite {
                negative: left_neg,
                coefficient: left_coeff,
                exponent: left_exp,
                ..
            },
            BinaryValue::Finite {
                coefficient: right_coeff,
                exponent: right_exp,
                ..
            },
        ) = (left, right)
        else {
            return Err(GuestError::cpu("Unresolved special operand"));
        };
        let exponent = (*left_exp).min(*right_exp);
        let left_mag = if *left_neg {
            left_coeff.negated()
        } else {
            left_coeff.clone()
        }
        .shl_bits((left_exp - exponent) as u32);
        let right_mag = if right_sign {
            right_coeff.negated()
        } else {
            right_coeff.clone()
        }
        .shl_bits((right_exp - exponent) as u32);
        let sum = left_mag.add(&right_mag);
        let sign = if sum.is_negative() {
            true
        } else if sum.is_zero() {
            if left.negative() == right_sign {
                left.negative()
            } else {
                mode == Rounding::Down
            }
        } else {
            false
        };
        let rounded = round_rational(
            sign,
            &sum.abs(),
            &BigInt::one(),
            exponent,
            format,
            mode,
        )?;
        return Ok(BinaryResult {
            flags: rounded.flags | source_flags,
            ..rounded
        });
    }
    let sign = left.negative() != right.negative();
    if operation == BinaryOperation::Multiply {
        if matches!(left, BinaryValue::Infinity { .. }) || matches!(right, BinaryValue::Infinity { .. })
        {
            let zero_operand = matches!(left, BinaryValue::Finite { coefficient, .. } if coefficient.is_zero())
                || matches!(right, BinaryValue::Finite { coefficient, .. } if coefficient.is_zero());
            if zero_operand {
                return Ok(BinaryResult {
                    value: indefinite(),
                    flags: FLAG_INVALID | source_flags,
                    rounded_up: false,
                });
            }
            return Ok(BinaryResult {
                value: BinaryValue::Infinity { negative: sign },
                flags: source_flags,
                rounded_up: false,
            });
        }
        let (
            BinaryValue::Finite {
                coefficient: left_coeff,
                exponent: left_exp,
                ..
            },
            BinaryValue::Finite {
                coefficient: right_coeff,
                exponent: right_exp,
                ..
            },
        ) = (left, right)
        else {
            return Err(GuestError::cpu("Unresolved special operand"));
        };
        let rounded = round_rational(
            sign,
            &left_coeff.mul(right_coeff),
            &BigInt::one(),
            left_exp + right_exp,
            format,
            mode,
        )?;
        return Ok(BinaryResult {
            flags: rounded.flags | source_flags,
            ..rounded
        });
    }
    // Division.
    if matches!(left, BinaryValue::Infinity { .. }) && matches!(right, BinaryValue::Infinity { .. })
    {
        return Ok(BinaryResult {
            value: indefinite(),
            flags: FLAG_INVALID | source_flags,
            rounded_up: false,
        });
    }
    if matches!(left, BinaryValue::Infinity { .. }) {
        return Ok(BinaryResult {
            value: BinaryValue::Infinity { negative: sign },
            flags: source_flags,
            rounded_up: false,
        });
    }
    if matches!(right, BinaryValue::Infinity { .. }) {
        return Ok(BinaryResult {
            value: zero(sign),
            flags: source_flags,
            rounded_up: false,
        });
    }
    let (
        BinaryValue::Finite {
            coefficient: left_coeff,
            exponent: left_exp,
            ..
        },
        BinaryValue::Finite {
            coefficient: right_coeff,
            exponent: right_exp,
            ..
        },
    ) = (left, right)
    else {
        return Err(GuestError::cpu("Unresolved special operand"));
    };
    if right_coeff.is_zero() {
        return Ok(BinaryResult {
            value: if left_coeff.is_zero() {
                indefinite()
            } else {
                BinaryValue::Infinity { negative: sign }
            },
            flags: source_flags | if left_coeff.is_zero() {
                FLAG_INVALID
            } else {
                FLAG_ZERO_DIVIDE
            },
            rounded_up: false,
        });
    }
    let rounded = round_rational(
        sign,
        left_coeff,
        right_coeff,
        left_exp - right_exp,
        format,
        mode,
    )?;
    Ok(BinaryResult {
        flags: rounded.flags | source_flags,
        ..rounded
    })
}

/// Total comparison with an unordered class for NaNs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryComparison {
    /// Less than.
    Less,
    /// Equal (including signed zeros).
    Equal,
    /// Greater than.
    Greater,
    /// Unordered (NaN or unsupported operand).
    Unordered,
}

/// Compare two values.
#[must_use]
pub fn compare_binary(left: &BinaryValue, right: &BinaryValue) -> BinaryComparison {
    if matches!(left, BinaryValue::Nan { .. } | BinaryValue::Unsupported { .. })
        || matches!(right, BinaryValue::Nan { .. } | BinaryValue::Unsupported { .. })
    {
        return BinaryComparison::Unordered;
    }
    if let (
        BinaryValue::Finite { coefficient: left_coeff, .. },
        BinaryValue::Finite { coefficient: right_coeff, .. },
    ) = (left, right)
    {
        if left_coeff.is_zero() && right_coeff.is_zero() {
            return BinaryComparison::Equal;
        }
    }
    if left.negative() != right.negative() {
        return if left.negative() {
            BinaryComparison::Less
        } else {
            BinaryComparison::Greater
        };
    }
    let comparison = match (left, right) {
        (BinaryValue::Infinity { .. }, BinaryValue::Infinity { .. }) => Ordering::Equal,
        (BinaryValue::Infinity { .. }, _) => Ordering::Greater,
        (_, BinaryValue::Infinity { .. }) => Ordering::Less,
        (
            BinaryValue::Finite {
                coefficient: left_coeff,
                exponent: left_exp,
                ..
            },
            BinaryValue::Finite {
                coefficient: right_coeff,
                exponent: right_exp,
                ..
            },
        ) => compare_scaled(left_coeff, right_coeff, left_exp - right_exp),
        _ => Ordering::Equal,
    };
    let comparison = if left.negative() {
        comparison.reverse()
    } else {
        comparison
    };
    match comparison {
        Ordering::Less => BinaryComparison::Less,
        Ordering::Greater => BinaryComparison::Greater,
        Ordering::Equal => BinaryComparison::Equal,
    }
}

/// Convert a value to a signed integer of `width` bits, saturating to the
/// x86 indefinite value on overflow or special input.
pub struct IntegerConversion {
    /// Converted (or indefinite) value.
    pub value: BigInt,
    /// Status flags.
    pub flags: u32,
    /// Whether rounding moved away from truncation.
    pub rounded_up: bool,
}

/// Integer conversion with `mode` rounding.
pub fn integer_conversion(
    value: &BinaryValue,
    width: u32,
    mode: Rounding,
) -> Result<IntegerConversion, GuestError> {
    let bad = || IntegerConversion {
        value: BigInt::one().shl_bits(width - 1).negated(),
        flags: FLAG_INVALID,
        rounded_up: false,
    };
    let BinaryValue::Finite {
        negative,
        coefficient,
        exponent,
        ..
    } = value
    else {
        return Ok(bad());
    };
    let rounded = if *exponent >= 0 {
        rounded_quotient(
            &coefficient.shl_bits(*exponent as u32),
            &BigInt::one(),
            *negative,
            mode,
        )?
    } else {
        rounded_quotient(
            coefficient,
            &BigInt::one().shl_bits((-exponent) as u32),
            *negative,
            mode,
        )?
    };
    let signed = if *negative {
        rounded.quotient.negated()
    } else {
        rounded.quotient
    };
    let minimum = BigInt::one().shl_bits(width - 1).negated();
    let maximum = BigInt::one().shl_bits(width - 1);
    if signed < minimum || signed >= maximum {
        return Ok(bad());
    }
    Ok(IntegerConversion {
        value: signed,
        flags: if rounded.inexact { FLAG_PRECISION } else { 0 },
        rounded_up: rounded.up,
    })
}

/// Round a value to an integral value with `mode` rounding.
pub fn round_integral(value: &BinaryValue, mode: Rounding) -> Result<BinaryResult, GuestError> {
    if !matches!(value, BinaryValue::Finite { .. }) {
        return convert_binary(value, BINARY80, mode);
    }
    let BinaryValue::Finite {
        negative,
        coefficient,
        exponent,
        denormal,
    } = value
    else {
        unreachable!("finite checked above");
    };
    if *exponent >= 0 {
        return Ok(BinaryResult {
            value: value.clone(),
            flags: 0,
            rounded_up: false,
        });
    }
    let rounded = rounded_quotient(
        coefficient,
        &BigInt::one().shl_bits((-exponent) as u32),
        *negative,
        mode,
    )?;
    Ok(BinaryResult {
        value: BinaryValue::Finite {
            negative: *negative,
            coefficient: rounded.quotient,
            exponent: 0,
            denormal: false,
        },
        flags: (if rounded.inexact { FLAG_PRECISION } else { 0 })
            | if *denormal { FLAG_DENORMAL_OPERAND } else { 0 },
        rounded_up: rounded.up,
    })
}

fn integer_square_root(value: &BigInt) -> Result<BigInt, GuestError> {
    if value.bit_len() <= 1 {
        return Ok(value.clone());
    }
    let mut estimate = BigInt::one().shl_bits(value.bit_len().div_ceil(2) as u32);
    loop {
        let next = estimate.add(&value.div(&estimate)?).shr_bits(1);
        if next >= estimate {
            return Ok(estimate);
        }
        estimate = next;
    }
}

/// Exact square root with one rounding step.
pub fn square_root(
    value: &BinaryValue,
    format: BinaryFormat,
    mode: Rounding,
) -> Result<BinaryResult, GuestError> {
    if matches!(value, BinaryValue::Nan { .. } | BinaryValue::Unsupported { .. }) {
        return convert_binary(value, format, mode);
    }
    if let BinaryValue::Finite { coefficient, .. } = value {
        if coefficient.is_zero() {
            return Ok(BinaryResult {
                value: value.clone(),
                flags: 0,
                rounded_up: false,
            });
        }
    }
    if value.negative() {
        return Ok(BinaryResult {
            value: indefinite(),
            flags: FLAG_INVALID,
            rounded_up: false,
        });
    }
    if matches!(value, BinaryValue::Infinity { .. }) {
        return Ok(BinaryResult {
            value: value.clone(),
            flags: 0,
            rounded_up: false,
        });
    }
    let BinaryValue::Finite {
        coefficient,
        exponent,
        denormal,
        ..
    } = value
    else {
        unreachable!("specials handled above");
    };
    let top = (coefficient.bit_len() as i32 - 1 + exponent).div_euclid(2);
    let quantum = (top - format.precision + 1).max(format.minimum_exponent - format.precision + 1);
    let shift = exponent - quantum * 2;
    let (numerator, denominator) = if shift >= 0 {
        (coefficient.shl_bits(shift as u32), BigInt::one())
    } else {
        (coefficient.clone(), BigInt::one().shl_bits((-shift) as u32))
    };
    let floor = integer_square_root(&numerator.div(&denominator)?)?;
    let inexact = floor.mul(&floor).mul(&denominator) != numerator;
    let halfway = floor
        .mul(&BigInt::from_u64(2))
        .add(&BigInt::one())
        .mul(&floor.mul(&BigInt::from_u64(2)).add(&BigInt::one()))
        .mul(&denominator);
    let four_numerator = numerator.mul(&BigInt::from_u64(4));
    let up = inexact
        && (mode == Rounding::Up
            || (mode == Rounding::Nearest
                && (four_numerator.cmp(&halfway) == Ordering::Greater
                    || (four_numerator == halfway && floor.bit(0)))));
    Ok(BinaryResult {
        value: BinaryValue::Finite {
            negative: false,
            coefficient: if up {
                floor.add(&BigInt::one())
            } else {
                floor
            },
            exponent: quantum,
            denormal: false,
        },
        flags: (if inexact { FLAG_PRECISION } else { 0 })
            | if *denormal { FLAG_DENORMAL_OPERAND } else { 0 },
        rounded_up: up,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bigint_divmod_matches_u64_arithmetic() {
        let left = BigInt::from_u64(0x1234_5678_9abc_def0);
        let right = BigInt::from_u64(0x1111_1111);
        let (quotient, remainder) = left.divmod(&right).unwrap();
        assert_eq!(
            quotient.low_u64(),
            0x1234_5678_9abc_def0u64 / 0x1111_1111u64
        );
        assert_eq!(
            remainder.low_u64(),
            0x1234_5678_9abc_def0u64 % 0x1111_1111u64
        );
    }

    #[test]
    fn binary32_round_trip_preserves_bits() {
        for bits in [0x3f80_0000u32, 0xc000_0000, 0x0000_0001, 0xff80_0000, 0x7fc0_0001] {
            let value = decode_binary32(bits);
            let encoded = encode_binary(&value, BinaryWidth::W32);
            assert_eq!(encoded.low_u64() as u32, bits, "bits 0x{bits:08x}");
        }
    }

    #[test]
    fn add_and_divide_match_exact_rationals() {
        let one = from_integer(&BigInt::one());
        let two = from_integer(&BigInt::from_u64(2));
        let sum = arithmetic(BinaryOperation::Add, &one, &two, BINARY64, Rounding::Nearest, false).unwrap();
        assert_eq!(sum.flags, 0);
        let ratio = arithmetic(
            BinaryOperation::Divide,
            &one,
            &from_integer(&BigInt::from_u64(8)),
            BINARY64,
            Rounding::Nearest,
            false,
        )
        .unwrap();
        assert_eq!(encode_binary(&ratio.value, BinaryWidth::W64).low_u64(), 0x3fc0_0000_0000_0000);
    }

    #[test]
    fn sqrt_two_rounds_to_nearest_even() {
        let two = from_integer(&BigInt::from_u64(2));
        let root = square_root(&two, BINARY64, Rounding::Nearest).unwrap();
        assert_eq!(encode_binary(&root.value, BinaryWidth::W64).low_u64(), 0x3ff6_a09e_667f_3bcd);
    }

    #[test]
    fn integer_conversion_saturates_to_indefinite() {
        let huge = from_integer(&BigInt::one().shl_bits(70));
        let converted = integer_conversion(&huge, 32, Rounding::Nearest).unwrap();
        assert_eq!(converted.flags, FLAG_INVALID);
        assert_eq!(converted.value, BigInt::one().shl_bits(31).negated());
    }
}
