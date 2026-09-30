//! QVM arithmetic and branch conditions.
//!
//! Port of `src/compat/qvm/operations.ts` (translated from Quake III Arena's
//! `qcommon/vm_interpreted.c`; Copyright (C) 1999-2005 Id Software, Inc.,
//! GPL-2.0-or-later).
//!
//! Operands and results are signed int32 words, including binary32 payloads.
//! Float conversions reuse `qa_core::numeric` (`qvm_float_to_int`,
//! `float32_to_bits`, `bits_to_float32`). The donor types opcode classes
//! statically; here the class is checked at runtime and a wrong-class opcode
//! fails with [`GuestError::InvalidArgument`](crate::error::GuestError).

use qa_core::numeric::{bits_to_float32, float32_to_bits, qvm_float_to_int};

use crate::error::GuestError;

use super::image::QvmOpcode;

/// Evaluate a unary opcode (`SEX8`, `SEX16`, `NEGI`, `NEGF`, `CVIF`, `CVFI`).
pub fn evaluate_unary(opcode: QvmOpcode, word: i32) -> Result<i32, GuestError> {
    match opcode {
        QvmOpcode::OpSex8 => Ok(word << 24 >> 24),
        QvmOpcode::OpSex16 => Ok(word << 16 >> 16),
        QvmOpcode::OpNegi => Ok(word.wrapping_neg()),
        QvmOpcode::OpNegf => Ok(float32_to_bits(-bits_to_float32(word as u32)) as i32),
        QvmOpcode::OpCvif => Ok(float32_to_bits(word as f32) as i32),
        QvmOpcode::OpCvfi => Ok(qvm_float_to_int(bits_to_float32(word as u32))),
        other => Err(GuestError::invalid(format!(
            "Unsupported QVM unary opcode {}",
            other.name()
        ))),
    }
}

fn shift_count(word: i32) -> Result<u32, GuestError> {
    if !(0..32).contains(&word) {
        return Err(GuestError::invalid("QVM shift count outside 0..31"));
    }
    Ok(word as u32)
}

/// Evaluate a binary opcode. Left is source r1, right is r0; integer overflow
/// wraps at 32 bits.
pub fn evaluate_binary(opcode: QvmOpcode, left: i32, right: i32) -> Result<i32, GuestError> {
    match opcode {
        QvmOpcode::OpAdd => Ok(left.wrapping_add(right)),
        QvmOpcode::OpSub => Ok(left.wrapping_sub(right)),
        QvmOpcode::OpDivi | QvmOpcode::OpModi => {
            if right == 0 {
                return Err(GuestError::invalid("QVM integer division or modulo by zero"));
            }
            if left == i32::MIN && right == -1 {
                return Err(GuestError::invalid("QVM signed division or modulo overflow"));
            }
            Ok(if opcode == QvmOpcode::OpDivi {
                left / right
            } else {
                left % right
            })
        }
        QvmOpcode::OpDivu | QvmOpcode::OpModu => {
            if right == 0 {
                return Err(GuestError::invalid("QVM integer division or modulo by zero"));
            }
            let left = left as u32;
            let right = right as u32;
            Ok(if opcode == QvmOpcode::OpDivu {
                (left / right) as i32
            } else {
                (left % right) as i32
            })
        }
        QvmOpcode::OpMuli | QvmOpcode::OpMulu => Ok(left.wrapping_mul(right)),
        QvmOpcode::OpBand => Ok(left & right),
        QvmOpcode::OpBor => Ok(left | right),
        QvmOpcode::OpBxor => Ok(left ^ right),
        QvmOpcode::OpLsh => Ok(left.wrapping_shl(shift_count(right)?)),
        QvmOpcode::OpRshi => Ok(left.wrapping_shr(shift_count(right)?)),
        QvmOpcode::OpRshu => Ok((left as u32).wrapping_shr(shift_count(right)?) as i32),
        QvmOpcode::OpAddf => Ok(float32_to_bits(bits_to_float32(left as u32) + bits_to_float32(right as u32)) as i32),
        QvmOpcode::OpSubf => Ok(float32_to_bits(bits_to_float32(left as u32) - bits_to_float32(right as u32)) as i32),
        // The donor divides in binary64, then rounds once to binary32; dividing
        // directly in binary32 could double-round.
        QvmOpcode::OpDivf => {
            let quotient = f64::from(bits_to_float32(left as u32)) / f64::from(bits_to_float32(right as u32));
            Ok(float32_to_bits(quotient as f32) as i32)
        }
        QvmOpcode::OpMulf => Ok(float32_to_bits(bits_to_float32(left as u32) * bits_to_float32(right as u32)) as i32),
        other => Err(GuestError::invalid(format!(
            "Unsupported QVM binary opcode {}",
            other.name()
        ))),
    }
}

/// Evaluate a branch condition. Left is source r1, right is r0; float
/// conditions use IEEE unordered comparisons.
pub fn evaluate_branch(opcode: QvmOpcode, left: i32, right: i32) -> Result<bool, GuestError> {
    match opcode {
        QvmOpcode::OpEq => Ok(left == right),
        QvmOpcode::OpNe => Ok(left != right),
        QvmOpcode::OpLti => Ok(left < right),
        QvmOpcode::OpLei => Ok(left <= right),
        QvmOpcode::OpGti => Ok(left > right),
        QvmOpcode::OpGei => Ok(left >= right),
        QvmOpcode::OpLtu => Ok((left as u32) < right as u32),
        QvmOpcode::OpLeu => Ok((left as u32) <= right as u32),
        QvmOpcode::OpGtu => Ok((left as u32) > right as u32),
        QvmOpcode::OpGeu => Ok((left as u32) >= right as u32),
        QvmOpcode::OpEqf => Ok(bits_to_float32(left as u32) == bits_to_float32(right as u32)),
        QvmOpcode::OpNef => Ok(bits_to_float32(left as u32) != bits_to_float32(right as u32)),
        QvmOpcode::OpLtf => Ok(bits_to_float32(left as u32) < bits_to_float32(right as u32)),
        QvmOpcode::OpLef => Ok(bits_to_float32(left as u32) <= bits_to_float32(right as u32)),
        QvmOpcode::OpGtf => Ok(bits_to_float32(left as u32) > bits_to_float32(right as u32)),
        QvmOpcode::OpGef => Ok(bits_to_float32(left as u32) >= bits_to_float32(right as u32)),
        other => Err(GuestError::invalid(format!(
            "Unsupported QVM branch opcode {}",
            other.name()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unary_covers_sign_extension_and_conversions() {
        assert_eq!(evaluate_unary(QvmOpcode::OpSex8, 0xFF).unwrap(), -1);
        assert_eq!(evaluate_unary(QvmOpcode::OpSex16, 0x8000).unwrap(), -32768);
        assert_eq!(evaluate_unary(QvmOpcode::OpNegi, 5).unwrap(), -5);
        assert_eq!(evaluate_unary(QvmOpcode::OpNegi, i32::MIN).unwrap(), i32::MIN);
        let neg = evaluate_unary(QvmOpcode::OpNegf, float32_to_bits(2.0) as i32).unwrap();
        assert_eq!(bits_to_float32(neg as u32), -2.0);
        let as_float = evaluate_unary(QvmOpcode::OpCvif, 3).unwrap();
        assert_eq!(bits_to_float32(as_float as u32), 3.0);
        assert_eq!(
            evaluate_unary(QvmOpcode::OpCvfi, float32_to_bits(2.75) as i32).unwrap(),
            2
        );
        assert_eq!(
            evaluate_unary(QvmOpcode::OpCvfi, float32_to_bits(f32::NAN) as i32).unwrap(),
            i32::MIN
        );
        assert!(evaluate_unary(QvmOpcode::OpAdd, 0).is_err());
    }

    #[test]
    fn binary_integer_edges_match_source() {
        assert_eq!(evaluate_binary(QvmOpcode::OpAdd, i32::MAX, 1).unwrap(), i32::MIN);
        assert_eq!(evaluate_binary(QvmOpcode::OpDivi, -7, 2).unwrap(), -3);
        assert_eq!(evaluate_binary(QvmOpcode::OpModi, -7, 2).unwrap(), -1);
        assert_eq!(evaluate_binary(QvmOpcode::OpDivu, -1, 2).unwrap(), 0x7FFF_FFFF);
        assert!(evaluate_binary(QvmOpcode::OpDivi, 1, 0).is_err());
        assert!(evaluate_binary(QvmOpcode::OpModu, 1, 0).is_err());
        assert!(evaluate_binary(QvmOpcode::OpDivi, i32::MIN, -1).is_err());
        assert_eq!(evaluate_binary(QvmOpcode::OpMuli, 0x1_0000, 0x1_0000).unwrap(), 0);
        assert_eq!(evaluate_binary(QvmOpcode::OpLsh, 1, 31).unwrap(), i32::MIN);
        assert_eq!(evaluate_binary(QvmOpcode::OpRshi, -8, 2).unwrap(), -2);
        assert_eq!(evaluate_binary(QvmOpcode::OpRshu, -8, 2).unwrap(), 0x3FFF_FFFE);
        assert!(evaluate_binary(QvmOpcode::OpLsh, 1, 32).is_err());
        assert!(evaluate_binary(QvmOpcode::OpLsh, 1, -1).is_err());
        assert!(evaluate_binary(QvmOpcode::OpBcom, 1, 2).is_err());
    }

    #[test]
    fn binary_floats_round_once() {
        let bit = |value: f32| float32_to_bits(value) as i32;
        let add = evaluate_binary(QvmOpcode::OpAddf, bit(1.5), bit(2.25)).unwrap();
        assert_eq!(bits_to_float32(add as u32), 3.75);
        let div = evaluate_binary(QvmOpcode::OpDivf, bit(1.0), bit(3.0)).unwrap();
        assert_eq!(bits_to_float32(div as u32), 1.0f32 / 3.0f32);
        let zero = evaluate_binary(QvmOpcode::OpDivf, bit(1.0), bit(0.0)).unwrap();
        assert_eq!(bits_to_float32(zero as u32), f32::INFINITY);
    }

    #[test]
    fn branches_use_unordered_float_comparisons() {
        assert!(evaluate_branch(QvmOpcode::OpEq, 3, 3).unwrap());
        assert!(!evaluate_branch(QvmOpcode::OpLtu, -1, 1).unwrap());
        assert!(evaluate_branch(QvmOpcode::OpGtu, -1, 1).unwrap());
        let nan = float32_to_bits(f32::NAN) as i32;
        assert!(!evaluate_branch(QvmOpcode::OpEqf, nan, nan).unwrap());
        assert!(evaluate_branch(QvmOpcode::OpNef, nan, nan).unwrap());
        assert!(!evaluate_branch(QvmOpcode::OpLtf, nan, 0).unwrap());
        assert!(evaluate_branch(QvmOpcode::OpAdd, 0, 0).is_err());
    }
}
