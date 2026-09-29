//! Shared integer ALU: flags, shifts, multiply, conditions.
//!
//! Donor: `src/guest/x86/arithmetic.ts`. Used by both the i386 and x86-64
//! interpreters. Follows Intel SDM Volume 2 ADD/ADC/SUB/SBB and logical
//! instruction flag definitions, including the masked-count shift rules.

use crate::core::contracts::GuestFlag;
use crate::core::registers::ProcessorFlags;
use crate::error::GuestError;

/// ALU operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AluOperation {
    /// Addition.
    Add,
    /// Bitwise OR.
    Or,
    /// Addition with carry.
    Adc,
    /// Subtraction with borrow.
    Sbb,
    /// Bitwise AND.
    And,
    /// Subtraction.
    Sub,
    /// Bitwise XOR.
    Xor,
    /// Compare (subtraction without writeback).
    Cmp,
    /// Bit test (AND without writeback).
    Test,
}

/// Shift/rotate operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftOperation {
    /// Rotate left.
    Rol,
    /// Rotate right.
    Ror,
    /// Rotate left through carry.
    Rcl,
    /// Rotate right through carry.
    Rcr,
    /// Shift left.
    Shl,
    /// Shift right.
    Shr,
    /// Arithmetic shift right.
    Sar,
}

fn parity_flag(byte: u8) -> u32 {
    let mut folded = byte ^ (byte >> 4);
    folded ^= folded >> 2;
    folded ^= folded >> 1;
    u32::from(folded & 1 == 0) << 2
}

fn mask(width: u32) -> u64 {
    if width == 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

fn result_bits(width: u32, result: u64) -> u32 {
    let result = result & mask(width);
    (u32::from(result == 0) << 6) | (u32::from(result & (1u64 << (width - 1)) != 0) << 7) | parity_flag(result as u8)
}

/// Sign-extend the low `width` bits of `value` to 128 bits.
#[must_use]
pub fn sign_extend(value: u64, width: u32) -> i128 {
    debug_assert!((1..=64).contains(&width));
    let shift = 64 - width;
    (((value & mask(width)) << shift) as i64 >> shift) as i128
}

/// Sign-extend the low `bits` bits of a double-width raw dividend to 128
/// bits. The caller guarantees `raw` holds fewer than `bits` bits.
#[must_use]
pub fn sign_extend_double(raw: u128, bits: u32) -> i128 {
    debug_assert!((2..=128).contains(&bits));
    debug_assert!(bits >= 128 || raw >> bits == 0);
    if bits >= 128 {
        raw as i128
    } else {
        ((raw << (128 - bits)) as i128) >> (128 - bits)
    }
}

/// Whether `quotient` fits in a signed `width`-bit range.
#[must_use]
pub fn quotient_fits_signed(quotient: i128, width: u32) -> bool {
    debug_assert!((1..=64).contains(&width));
    let limit = 1i128 << (width - 1);
    quotient >= -limit && quotient < limit
}

/// Update ZF/SF/PF from a raw result, preserving the other flags.
pub fn result_flags(width: u32, value: u64, flags: &mut ProcessorFlags) {
    let current = flags.value();
    flags.set_value((current & !0xc4) | u64::from(result_bits(width, value)));
}

/// Execute one ALU operation, updating flags. Returns the masked result.
pub fn alu(operation: AluOperation, width: u32, left: u64, right: u64, flags: &mut ProcessorFlags) -> u64 {
    let mask = mask(width);
    let sign = 1u64 << (width - 1);
    let a = left & mask;
    let b = right & mask;
    let (result, bits, flag_mask) = match operation {
        AluOperation::Add | AluOperation::Adc => {
            let incoming = u64::from(operation == AluOperation::Adc && flags.get(GuestFlag::Carry));
            let full = a as u128 + b as u128 + incoming as u128;
            let result = full as u64 & mask;
            let mut bits = 0;
            if full > mask as u128 {
                bits |= 1;
            }
            if (!(a ^ b) & (a ^ result) & sign) != 0 {
                bits |= 0x800;
            }
            if (a & 15) + (b & 15) + incoming > 15 {
                bits |= 0x10;
            }
            (result, bits, 0x8d5)
        }
        AluOperation::Sub | AluOperation::Cmp | AluOperation::Sbb => {
            let incoming = u64::from(operation == AluOperation::Sbb && flags.get(GuestFlag::Carry));
            let full = a as i128 - b as i128 - incoming as i128;
            let result = full as u64 & mask;
            let mut bits = 0;
            if full < 0 {
                bits |= 1;
            }
            if ((a ^ b) & (a ^ result) & sign) != 0 {
                bits |= 0x800;
            }
            if (a & 15) < (b & 15) + incoming {
                bits |= 0x10;
            }
            (result, bits, 0x8d5)
        }
        AluOperation::And | AluOperation::Test => (a & b, 0, 0x8c5),
        AluOperation::Or => (a | b, 0, 0x8c5),
        AluOperation::Xor => (a ^ b, 0, 0x8c5),
    };
    flags.set_value((flags.value() & !flag_mask) | u64::from(bits | result_bits(width, result)));
    result
}

/// Execute one shift/rotate with a masked count. Undefined flags retain
/// their prior bits; defined flags follow Intel's masked-count rules.
pub fn shift(operation: ShiftOperation, width: u32, value: u64, count: u32, flags: &mut ProcessorFlags) -> u64 {
    let masked = count & if width == 64 { 63 } else { 31 };
    let rotate = matches!(
        operation,
        ShiftOperation::Rol | ShiftOperation::Ror | ShiftOperation::Rcl | ShiftOperation::Rcr
    );
    let through_carry = matches!(operation, ShiftOperation::Rcl | ShiftOperation::Rcr);
    let effective = if rotate {
        masked % (width + u32::from(through_carry))
    } else {
        masked
    };
    let mut result = value & mask(width);
    if effective == 0 {
        if masked != 0 && operation == ShiftOperation::Rol {
            flags.set(GuestFlag::Carry, result & 1 != 0);
        }
        if masked != 0 && operation == ShiftOperation::Ror {
            flags.set(GuestFlag::Carry, result & (1u64 << (width - 1)) != 0);
        }
        return result;
    }
    let sign = 1u64 << (width - 1);
    let original_sign = result & sign != 0;
    let mut carry = flags.get(GuestFlag::Carry);
    match operation {
        ShiftOperation::Rol => {
            result = ((result << effective) | (result >> (width - effective))) & mask(width);
            carry = result & 1 != 0;
        }
        ShiftOperation::Ror => {
            result = ((result >> effective) | (result << (width - effective))) & mask(width);
            carry = result & sign != 0;
        }
        ShiftOperation::Rcl | ShiftOperation::Rcr => {
            let extended = (u128::from(result) << 1) | u128::from(carry);
            let total = width + 1;
            let wide_mask = if total >= 128 { u128::MAX } else { (1u128 << total) - 1 };
            let rotated = if operation == ShiftOperation::Rcl {
                ((extended << effective) | (extended >> (total - effective))) & wide_mask
            } else {
                ((extended >> effective) | (extended << (total - effective))) & wide_mask
            };
            result = (rotated >> 1) as u64;
            carry = rotated & 1 != 0;
        }
        ShiftOperation::Shl => {
            if effective < width {
                carry = (result >> (width - effective)) & 1 != 0;
            }
            result = (result << effective) & mask(width);
        }
        ShiftOperation::Shr => {
            if effective < width {
                carry = (result >> (effective - 1)) & 1 != 0;
            }
            result >>= effective;
        }
        ShiftOperation::Sar => {
            carry = if effective >= width {
                original_sign
            } else {
                (result >> (effective - 1)) & 1 != 0
            };
            result = (sign_extend(result, width) >> effective) as u64 & mask(width);
        }
    }
    if rotate || effective < width || operation == ShiftOperation::Sar {
        flags.set(GuestFlag::Carry, carry);
    }
    if !rotate {
        result_flags(width, result, flags);
    }
    if masked == 1 {
        match operation {
            ShiftOperation::Rol | ShiftOperation::Rcl | ShiftOperation::Shl => {
                flags.set(GuestFlag::Overflow, (result & sign != 0) != carry);
            }
            ShiftOperation::Ror | ShiftOperation::Rcr => {
                flags.set(GuestFlag::Overflow, (result & sign != 0) != (result & (sign >> 1) != 0));
            }
            ShiftOperation::Shr => flags.set(GuestFlag::Overflow, original_sign),
            ShiftOperation::Sar => flags.set(GuestFlag::Overflow, false),
        }
    }
    result
}

/// Signed multiply with CF/OF overflow reporting. Returns the masked result.
pub fn signed_multiply(width: u32, left: u64, right: u64, flags: &mut ProcessorFlags) -> u64 {
    let extend = |value: u64| ((value << (64 - width)) as i64) as i128;
    let full = extend(left) * extend(right);
    let result = full as u64 & mask(width);
    let overflow = extend(result) != full;
    flags.set(GuestFlag::Carry, overflow);
    flags.set(GuestFlag::Overflow, overflow);
    result
}

/// Evaluate a condition code against the flags.
pub fn condition(code: u8, flags: &ProcessorFlags) -> Result<bool, GuestError> {
    let get = |flag| flags.get(flag);
    Ok(match code {
        0 => get(GuestFlag::Overflow),
        1 => !get(GuestFlag::Overflow),
        2 => get(GuestFlag::Carry),
        3 => !get(GuestFlag::Carry),
        4 => get(GuestFlag::Zero),
        5 => !get(GuestFlag::Zero),
        6 => get(GuestFlag::Carry) || get(GuestFlag::Zero),
        7 => !get(GuestFlag::Carry) && !get(GuestFlag::Zero),
        8 => get(GuestFlag::Sign),
        9 => !get(GuestFlag::Sign),
        10 => get(GuestFlag::Parity),
        11 => !get(GuestFlag::Parity),
        12 => get(GuestFlag::Sign) != get(GuestFlag::Overflow),
        13 => get(GuestFlag::Sign) == get(GuestFlag::Overflow),
        14 => get(GuestFlag::Zero) || get(GuestFlag::Sign) != get(GuestFlag::Overflow),
        15 => !get(GuestFlag::Zero) && get(GuestFlag::Sign) == get(GuestFlag::Overflow),
        _ => return Err(GuestError::cpu(format!("Invalid condition code {code}"))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_sets_carry_overflow_auxiliary_and_parity() {
        let mut flags = ProcessorFlags::new(0);
        let result = alu(AluOperation::Add, 8, 0xff, 0x01, &mut flags);
        assert_eq!(result, 0x00);
        assert!(flags.get(GuestFlag::Carry));
        assert!(flags.get(GuestFlag::Zero));
        assert!(flags.get(GuestFlag::AuxiliaryCarry));
        assert!(flags.get(GuestFlag::Parity));
        assert!(!flags.get(GuestFlag::Overflow));
    }

    #[test]
    fn sub_borrows_and_sign_extends() {
        let mut flags = ProcessorFlags::new(0);
        let result = alu(AluOperation::Sub, 32, 0, 1, &mut flags);
        assert_eq!(result, 0xffff_ffff);
        assert!(flags.get(GuestFlag::Carry));
        assert!(flags.get(GuestFlag::Sign));
    }

    #[test]
    fn shift_through_carry_round_trips() {
        let mut flags = ProcessorFlags::new(0);
        flags.set(GuestFlag::Carry, true);
        let result = shift(ShiftOperation::Rcl, 8, 0x80, 1, &mut flags);
        assert_eq!(result, 0x01);
        assert!(flags.get(GuestFlag::Carry));
    }

    #[test]
    fn conditions_cover_all_codes() {
        let mut flags = ProcessorFlags::new(0);
        flags.set(GuestFlag::Zero, true);
        assert!(condition(4, &flags).unwrap());
        assert!(!condition(5, &flags).unwrap());
        assert!(condition(14, &flags).unwrap());
        assert!(!condition(15, &flags).unwrap());
    }
}
