//! x87 transcendental instructions over exact fixed-point series.
//!
//! Donor: `src/guest/floating-point/trigonometric.ts`. Intel's x87 target is
//! sin/cos(x * pi / p), where p is the 68-bit approximation of pi; polynomial
//! evaluation uses integer guard bits and no host floating-point
//! trigonometric operation participates.

use crate::error::GuestError;

use super::binary::{
    arithmetic, convert_binary, from_integer, round_rational, zero, BigInt, BinaryFormat, BinaryOperation,
    BinaryResult, BinaryValue, Rounding, BINARY80, FLAG_DENORMAL_OPERAND, FLAG_INVALID, FLAG_PRECISION,
};

/// Fixed-point fraction bits.
const FRACTION_BITS: u32 = 256;

/// 256-bit approximation of pi.
fn pi() -> BigInt {
    // Donor pi is 258 bits: 0x3243f6a8885a308d313198a2e03707344a4093822299f31d0082efa98ec4e6c89.
    BigInt::from_u128(0x3_243f_6a88_85a3_08d3)
        .shl_bits(192)
        .add(&BigInt::from_u128(0x1319_8a2e_0370_7344).shl_bits(128))
        .add(&BigInt::from_u128(0xa409_3822_299f_31d0_082e_fa98_ec4e_6c89))
}

/// 68-bit source approximation of pi.
fn source_pi() -> BigInt {
    pi().shr_bits(190)
}

/// Source pi scaled back to 256-bit fixed point.
fn source_pi_scaled() -> BigInt {
    source_pi().shl_bits(190)
}

fn scale() -> BigInt {
    BigInt::one().shl_bits(FRACTION_BITS)
}

/// Outcome of an x87 trigonometric instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum X87TrigonometricResult {
    /// Magnitude at or above 2^63: C2 set, operand unchanged.
    OutOfRange,
    /// Rounded result.
    Result(BinaryResult),
}

/// FSIN/FCOS with `mode` rounding.
pub fn x87_trigonometric(
    value: &BinaryValue,
    cosine: bool,
    mode: Rounding,
) -> Result<X87TrigonometricResult, GuestError> {
    if matches!(value, BinaryValue::Infinity { .. }) {
        return Ok(X87TrigonometricResult::Result(BinaryResult {
            value: super::binary::indefinite(),
            flags: FLAG_INVALID,
            rounded_up: false,
        }));
    }
    let BinaryValue::Finite {
        negative,
        coefficient,
        exponent,
        denormal,
    } = value
    else {
        return Ok(X87TrigonometricResult::Result(convert_binary(value, BINARY80, mode)?));
    };
    if coefficient.is_zero() {
        return Ok(X87TrigonometricResult::Result(BinaryResult {
            value: if cosine {
                from_integer(&BigInt::one())
            } else {
                value.clone()
            },
            flags: 0,
            rounded_up: false,
        }));
    }
    let magnitude = coefficient.bit_len() as i32 - 1 + exponent;
    if magnitude >= 63 {
        return Ok(X87TrigonometricResult::OutOfRange);
    }
    let flags = FLAG_PRECISION | if *denormal { FLAG_DENORMAL_OPERAND } else { 0 };
    if magnitude <= -128 {
        let approximation = if cosine {
            BinaryValue::Finite {
                negative: false,
                coefficient: scale().sub(&BigInt::one()),
                exponent: -(FRACTION_BITS as i32),
                denormal: false,
            }
        } else {
            BinaryValue::Finite {
                negative: *negative,
                coefficient: coefficient.mul(&pi()).shl_bits(128).div(&source_pi())?,
                exponent: exponent - 318,
                denormal: false,
            }
        };
        let result = convert_binary(&approximation, BINARY80, mode)?;
        return Ok(X87TrigonometricResult::Result(BinaryResult {
            flags: result.flags | flags,
            ..result
        }));
    }
    let shift = exponent + FRACTION_BITS as i32;
    let source_scaled = source_pi_scaled();
    let mut angle = if shift >= 0 {
        coefficient.shl_bits(shift as u32)
    } else {
        coefficient.shr_bits((-shift) as u32)
    }
    .rem(&source_scaled.add(&source_scaled))?;
    if angle > source_scaled {
        angle = angle.sub(&source_scaled.add(&source_scaled));
    }
    if *negative {
        angle = angle.negated();
    }
    angle = angle.mul(&pi()).div(&source_scaled)?;
    let squared = angle.mul(&angle);
    let mut term = if cosine { scale() } else { angle.clone() };
    let mut sum = term.clone();
    let mut index = BigInt::one();
    let two = BigInt::from_u64(2);
    while !term.is_zero() {
        let denominator = if cosine {
            two.mul(&index).sub(&BigInt::one()).mul(&two.mul(&index))
        } else {
            two.mul(&index).mul(&two.mul(&index).add(&BigInt::one()))
        };
        term = term
            .negated()
            .mul(&squared)
            .div(&scale().mul(&scale()).mul(&denominator))?;
        sum = sum.add(&term);
        index = index.add(&BigInt::one());
    }
    let result = convert_binary(
        &BinaryValue::Finite {
            negative: sum.is_negative(),
            coefficient: sum.abs(),
            exponent: -(FRACTION_BITS as i32),
            denormal: false,
        },
        BINARY80,
        mode,
    )?;
    Ok(X87TrigonometricResult::Result(BinaryResult {
        flags: result.flags | flags,
        ..result
    }))
}

/// FPATAN over Cartesian (y, x), including signed axes and infinities.
pub fn x87_arctangent(y: &BinaryValue, x: &BinaryValue, mode: Rounding) -> Result<BinaryResult, GuestError> {
    if matches!(y, BinaryValue::Nan { .. } | BinaryValue::Unsupported { .. })
        || matches!(x, BinaryValue::Nan { .. } | BinaryValue::Unsupported { .. })
    {
        return arithmetic(BinaryOperation::Add, y, x, BINARY80, mode, false);
    }
    let source_flags = if matches!(y, BinaryValue::Finite { denormal: true, .. })
        || matches!(x, BinaryValue::Finite { denormal: true, .. })
    {
        FLAG_DENORMAL_OPERAND
    } else {
        0
    };
    let angle_result = |angle: &BigInt| -> Result<BinaryResult, GuestError> {
        let result = round_rational(y.negative(), angle, &scale(), 0, BINARY80, mode)?;
        Ok(BinaryResult {
            flags: result.flags | source_flags | if angle.is_zero() { 0 } else { FLAG_PRECISION },
            ..result
        })
    };
    if let BinaryValue::Finite { coefficient, .. } = y {
        if coefficient.is_zero() {
            return if x.negative() {
                angle_result(&pi())
            } else {
                Ok(BinaryResult {
                    value: zero(y.negative()),
                    flags: source_flags,
                    rounded_up: false,
                })
            };
        }
    }
    if matches!(y, BinaryValue::Infinity { .. }) {
        let quarter = pi().div(&BigInt::from_u64(4))?;
        let angle = if matches!(x, BinaryValue::Infinity { .. }) {
            if x.negative() {
                pi().mul(&BigInt::from_u64(3)).div(&BigInt::from_u64(4))?
            } else {
                quarter
            }
        } else {
            pi().div(&BigInt::from_u64(2))?
        };
        return angle_result(&angle);
    }
    if matches!(x, BinaryValue::Infinity { .. }) {
        let angle = if x.negative() { pi() } else { BigInt::zero() };
        return angle_result(&angle);
    }
    let (
        BinaryValue::Finite {
            coefficient: y_coeff,
            exponent: y_exp,
            ..
        },
        BinaryValue::Finite {
            coefficient: x_coeff,
            exponent: x_exp,
            ..
        },
    ) = (y, x)
    else {
        return Err(GuestError::cpu("Unresolved arctangent operand"));
    };
    if x_coeff.is_zero() {
        return angle_result(&pi().div(&BigInt::from_u64(2))?);
    }
    let shift = y_exp - x_exp;
    let (numerator, denominator) = if shift >= 0 {
        (y_coeff.shl_bits(shift as u32), x_coeff.clone())
    } else {
        (y_coeff.clone(), x_coeff.shl_bits((-shift) as u32))
    };
    if !x.negative() && numerator.shl_bits(128) <= denominator {
        // atan(r) is just below r. Keep that side of exact binary boundaries
        // even when the ratio is far below the fixed-point polynomial's
        // resolution.
        let result = round_rational(
            y.negative(),
            &numerator.mul(&scale().sub(&BigInt::one())),
            &denominator.mul(&scale()),
            0,
            BINARY80,
            mode,
        )?;
        return Ok(BinaryResult {
            flags: result.flags | source_flags | FLAG_PRECISION,
            ..result
        });
    }
    let reciprocal = numerator > denominator;
    let mut z = if reciprocal {
        denominator.mul(&scale()).div(&numerator)?
    } else {
        numerator.mul(&scale()).div(&denominator)?
    };
    let reduced = z > scale().div(&BigInt::from_u64(2))?;
    if reduced {
        z = z.sub(&scale()).mul(&scale()).div(&z.add(&scale()))?;
    }
    let squared = z.mul(&z);
    let mut power = z.clone();
    let mut angle = z;
    let mut index = BigInt::one();
    let two = BigInt::from_u64(2);
    while !power.is_zero() {
        power = power.negated().mul(&squared).div(&scale().mul(&scale()))?;
        angle = angle.add(&power.div(&two.mul(&index).add(&BigInt::one()))?);
        index = index.add(&BigInt::one());
    }
    if reduced {
        angle = angle.add(&pi().div(&BigInt::from_u64(4))?);
    }
    if reciprocal {
        angle = pi().div(&BigInt::from_u64(2))?.sub(&angle);
    }
    if x.negative() {
        angle = pi().sub(&angle);
    }
    angle_result(&angle)
}

/// Re-export for binary-format callers.
#[allow(dead_code)]
pub(crate) fn binary80_format() -> BinaryFormat {
    BINARY80
}

#[cfg(test)]
mod tests {
    use super::super::binary::{convert_binary, decode_binary, encode_binary, BinaryWidth, BINARY64};
    use super::*;

    fn f80(value: f64) -> BinaryValue {
        decode_binary(&BigInt::from_bytes_le(&value.to_le_bytes()), BinaryWidth::W64)
    }

    #[test]
    fn sin_zero_is_signed_zero() {
        let result = x87_trigonometric(&zero(false), false, Rounding::Nearest).unwrap();
        let X87TrigonometricResult::Result(result) = result else {
            panic!("expected result");
        };
        assert!(matches!(
            result.value,
            BinaryValue::Finite { coefficient, .. } if coefficient.is_zero()
        ));
    }

    #[test]
    fn cos_zero_is_one() {
        let result = x87_trigonometric(&zero(false), true, Rounding::Nearest).unwrap();
        let X87TrigonometricResult::Result(result) = result else {
            panic!("expected result");
        };
        let bits = encode_binary(&result.value, BinaryWidth::W64);
        assert_eq!(bits.low_u64(), 0x3ff0_0000_0000_0000);
    }

    #[test]
    fn huge_arguments_are_out_of_range() {
        let huge = from_integer(&BigInt::one().shl_bits(70));
        assert_eq!(
            x87_trigonometric(&huge, false, Rounding::Nearest).unwrap(),
            X87TrigonometricResult::OutOfRange
        );
    }

    #[test]
    fn atan_unit_diagonal_is_pi_over_4() {
        let one = f80(1.0);
        let result = x87_arctangent(&one, &one, Rounding::Nearest).unwrap();
        // atan(1,1) = pi/4. The 80-bit result is donor-exact; the f64
        // comparison rounds through binary64 like readX87Return.
        let bits80 = encode_binary(&result.value, BinaryWidth::W80);
        assert_eq!(
            bits80.to_bytes_le(10),
            [0x35, 0xc2, 0x68, 0x21, 0xa2, 0xda, 0x0f, 0xc9, 0xfe, 0x3f]
        );
        let rounded = convert_binary(&result.value, BINARY64, Rounding::Nearest).unwrap();
        let bits = encode_binary(&rounded.value, BinaryWidth::W64);
        assert_eq!(bits.low_u64(), std::f64::consts::FRAC_PI_4.to_bits());
    }
}
