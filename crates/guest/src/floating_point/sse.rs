//! SSE/SSE2/SSE3 instruction execution over exact binary arithmetic.
//!
//! Donor: `src/guest/floating-point/sse.ts`. Floating lanes decode to exact
//! values (honoring denormals-are-zero), convert with MXCSR rounding, and
//! signal through MXCSR mask bits; packed-integer lanes stay raw bytes.

use crate::core::contracts::{GuestFlag, GuestRegister};

use super::binary::{
    arithmetic, compare_binary, convert_binary, decode_binary, decode_binary32, encode_binary, from_integer,
    integer_conversion, read_bits, rounding, square_root, write_bits, zero, BigInt, BinaryOperation, BinaryResult,
    BinaryValue, BinaryWidth, BINARY32, BINARY64, FLAG_DENORMAL_OPERAND, FLAG_INVALID, FLAG_UNDERFLOW,
};
use super::contracts::{NumericError, NumericExecutionContext, NumericOperand, NumericPrefix};
use super::raw_sse::{execute_raw_sse, prepare_raw_sse};

fn general_register(index: usize) -> Result<GuestRegister, NumericError> {
    const NAMES: [GuestRegister; 16] = [
        GuestRegister::Rax,
        GuestRegister::Rcx,
        GuestRegister::Rdx,
        GuestRegister::Rbx,
        GuestRegister::Rsp,
        GuestRegister::Rbp,
        GuestRegister::Rsi,
        GuestRegister::Rdi,
        GuestRegister::R8,
        GuestRegister::R9,
        GuestRegister::R10,
        GuestRegister::R11,
        GuestRegister::R12,
        GuestRegister::R13,
        GuestRegister::R14,
        GuestRegister::R15,
    ];
    NAMES
        .get(index)
        .copied()
        .ok_or_else(|| NumericError::unsupported("Invalid general register index"))
}

fn check_xmm(state: &crate::core::registers::GuestProcessorState, index: usize) -> Result<(), NumericError> {
    if (index + 1)
        .checked_mul(16)
        .is_some_and(|end| end <= state.simd.xmm.len())
    {
        Ok(())
    } else {
        Err(NumericError::unsupported("Invalid XMM register index"))
    }
}

fn operand(context: &NumericExecutionContext) -> Result<NumericOperand, NumericError> {
    context
        .instruction
        .operand
        .ok_or_else(|| NumericError::unsupported("SSE instruction requires an operand"))
}

fn load(
    context: &mut NumericExecutionContext,
    byte_length: usize,
    require_alignment: bool,
) -> Result<Vec<u8>, NumericError> {
    let source = operand(context)?;
    if require_alignment && byte_length == 16 {
        aligned(context)?;
    }
    match source {
        NumericOperand::Register(index) => {
            check_xmm(context.state, index)?;
            let start = index * 16;
            Ok(context.state.simd.xmm[start..start + byte_length].to_vec())
        }
        NumericOperand::Memory(address) => Ok(context.memory.copy(address, byte_length)?),
    }
}

fn aligned(context: &NumericExecutionContext) -> Result<(), NumericError> {
    if let NumericOperand::Memory(address) = operand(context)? {
        if address.offset % 16 != 0 {
            return Err(NumericError::fault(13, "Unaligned 16-byte SIMD operand"));
        }
    }
    Ok(())
}

fn signal(context: &mut NumericExecutionContext, flags: u32) -> Result<(), NumericError> {
    let state = &mut context.state.simd;
    state.mxcsr |= flags;
    if flags & !(state.mxcsr >> 7) & 63 != 0 {
        return Err(NumericError::fault(19, "Unmasked SIMD floating-point exception"));
    }
    Ok(())
}

fn source_value(context: &NumericExecutionContext, bytes: &[u8], width: u32, offset: usize) -> BinaryValue {
    let value = if width == 32 {
        let word = u32::from_le_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]);
        decode_binary32(word)
    } else {
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes[offset..offset + 8]);
        decode_binary(&BigInt::from_u64(u64::from_le_bytes(word)), BinaryWidth::W64)
    };
    if matches!(value, BinaryValue::Finite { denormal: true, .. }) && context.state.simd.mxcsr & 64 != 0 {
        zero(value.negative())
    } else {
        value
    }
}

fn finish(context: &NumericExecutionContext, result: BinaryResult) -> BinaryResult {
    let flush = context.state.simd.mxcsr & 0x8000 != 0 && context.state.simd.mxcsr & 0x800 != 0;
    if flush && matches!(result.value, BinaryValue::Finite { denormal: true, .. }) && result.flags & FLAG_UNDERFLOW != 0
    {
        return BinaryResult {
            value: zero(result.value.negative()),
            flags: result.flags | FLAG_UNDERFLOW | super::binary::FLAG_PRECISION,
            rounded_up: false,
        };
    }
    result
}

fn lane(bytes: &[u8], index: usize, width: u32) -> BigInt {
    let offset = index * width as usize / 8;
    match width {
        8 => BigInt::from_u64(u64::from(bytes[offset])),
        16 => BigInt::from_u64(u64::from(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))),
        32 => BigInt::from_u64(u64::from(u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]))),
        64 => {
            let mut word = [0u8; 8];
            word.copy_from_slice(&bytes[offset..offset + 8]);
            BigInt::from_u64(u64::from_le_bytes(word))
        }
        _ => read_bits(&bytes[offset..offset + width as usize / 8]),
    }
}

fn set_lane(bytes: &mut [u8], index: usize, width: u32, value: &BigInt) {
    let offset = index * width as usize / 8;
    let masked = value.as_uint_n(width);
    match width {
        8 => bytes[offset] = masked.low_u64() as u8,
        16 => bytes[offset..offset + 2].copy_from_slice(&(masked.low_u64() as u16).to_le_bytes()),
        32 => bytes[offset..offset + 4].copy_from_slice(&(masked.low_u64() as u32).to_le_bytes()),
        64 => bytes[offset..offset + 8].copy_from_slice(&masked.low_u64().to_le_bytes()),
        _ => {
            let encoded = write_bits(&masked, width as usize / 8);
            bytes[offset..offset + encoded.len()].copy_from_slice(&encoded);
        }
    }
}

fn integer_source(context: &mut NumericExecutionContext, width: u32) -> Result<BigInt, NumericError> {
    let source = operand(context)?;
    match source {
        NumericOperand::Register(index) => {
            let register = general_register(index)?;
            let raw = context
                .state
                .registers
                .read(
                    register,
                    if width == 64 {
                        crate::core::contracts::GuestIntegerWidth::B64
                    } else {
                        crate::core::contracts::GuestIntegerWidth::B32
                    },
                    false,
                )
                .map_err(NumericError::Guest)?;
            Ok(BigInt::from_u64(raw).as_int_n(width))
        }
        NumericOperand::Memory(address) => {
            let bytes = context.memory.copy(address, width as usize / 8)?;
            Ok(read_bits(&bytes).as_int_n(width))
        }
    }
}

fn floating(context: &mut NumericExecutionContext, opcode: u8) -> Result<(), NumericError> {
    let instruction = context.instruction;
    let scalar = matches!(instruction.prefix, NumericPrefix::XF2 | NumericPrefix::XF3);
    let width = if matches!(instruction.prefix, NumericPrefix::X66 | NumericPrefix::XF2) {
        64
    } else {
        32
    };
    let count = if scalar { 1 } else { 128 / width };
    check_xmm(context.state, instruction.register_index)?;
    let start = instruction.register_index * 16;
    let left_bytes = context.state.simd.xmm[start..start + 16].to_vec();
    let right_bytes = load(context, if scalar { width / 8 } else { 16 }, true)?;
    let mut output = left_bytes.clone();
    let mode = rounding(context.state.simd.mxcsr >> 13);
    let mut flags = 0;
    for index in 0..count {
        let left = source_value(context, &left_bytes, width as u32, index * width / 8);
        let right = source_value(context, &right_bytes, width as u32, index * width / 8);
        if opcode == 0xc2 {
            let Some(immediate) = instruction.immediate else {
                return Err(NumericError::unsupported("CMP requires an immediate predicate"));
            };
            let predicate = immediate & 7;
            let comparison = compare_binary(&left, &right);
            let unordered = comparison == super::binary::BinaryComparison::Unordered;
            let less = comparison == super::binary::BinaryComparison::Less;
            let equal = comparison == super::binary::BinaryComparison::Equal;
            let yes = match predicate {
                0 => equal,
                1 => less,
                2 => less || equal,
                3 => unordered,
                4 => !equal,
                5 => !less,
                6 => !(less || equal),
                _ => !unordered,
            };
            let signaling_predicate = matches!(predicate, 1 | 2 | 5 | 6);
            let left_signal = matches!(left, BinaryValue::Nan { signaling, .. } if signaling || signaling_predicate);
            let right_signal = matches!(right, BinaryValue::Nan { signaling, .. } if signaling || signaling_predicate);
            if left_signal || right_signal {
                flags |= FLAG_INVALID;
            } else if matches!(left, BinaryValue::Finite { denormal: true, .. })
                || matches!(right, BinaryValue::Finite { denormal: true, .. })
            {
                flags |= FLAG_DENORMAL_OPERAND;
            }
            let mask = if yes {
                BigInt::one().shl_bits(width as u32).sub(&BigInt::one())
            } else {
                BigInt::zero()
            };
            set_lane(&mut output, index, width as u32, &mask);
            continue;
        }
        let result = if opcode == 0x51 {
            square_root(&right, if width == 64 { BINARY64 } else { BINARY32 }, mode)?
        } else if opcode == 0x5d || opcode == 0x5f {
            let comparison = compare_binary(&left, &right);
            let selected = if comparison == super::binary::BinaryComparison::Unordered
                || comparison == super::binary::BinaryComparison::Equal
            {
                right.clone()
            } else if (opcode == 0x5d && comparison == super::binary::BinaryComparison::Less)
                || (opcode == 0x5f && comparison == super::binary::BinaryComparison::Greater)
            {
                left.clone()
            } else {
                right.clone()
            };
            let special = matches!(left, BinaryValue::Nan { signaling: true, .. })
                || matches!(right, BinaryValue::Nan { signaling: true, .. });
            BinaryResult {
                value: selected,
                flags: if special {
                    FLAG_INVALID
                } else if matches!(left, BinaryValue::Finite { denormal: true, .. })
                    || matches!(right, BinaryValue::Finite { denormal: true, .. })
                {
                    FLAG_DENORMAL_OPERAND
                } else {
                    0
                },
                rounded_up: false,
            }
        } else {
            let operation = match opcode {
                0x58 => BinaryOperation::Add,
                0x59 => BinaryOperation::Multiply,
                0x5c => BinaryOperation::Subtract,
                _ => BinaryOperation::Divide,
            };
            arithmetic(
                operation,
                &left,
                &right,
                if width == 64 { BINARY64 } else { BINARY32 },
                mode,
                true,
            )?
        };
        let result = finish(context, result);
        flags |= result.flags;
        set_lane(
            &mut output,
            index,
            width as u32,
            &encode_binary(
                &result.value,
                if width == 64 {
                    BinaryWidth::W64
                } else {
                    BinaryWidth::W32
                },
            ),
        );
    }
    signal(context, flags)?;
    let start = instruction.register_index * 16;
    context.state.simd.xmm[start..start + 16].copy_from_slice(&output);
    Ok(())
}

fn conversions(context: &mut NumericExecutionContext, opcode: u8) -> Result<(), NumericError> {
    let instruction = context.instruction;
    check_xmm(context.state, instruction.register_index)?;
    let start = instruction.register_index * 16;
    let mode = rounding(context.state.simd.mxcsr >> 13);
    if opcode == 0x2a {
        if !matches!(instruction.prefix, NumericPrefix::XF2 | NumericPrefix::XF3) {
            return Err(NumericError::unsupported("MMX integer conversion is unsupported"));
        }
        let width = if matches!(instruction.prefix, NumericPrefix::XF2) {
            64
        } else {
            32
        };
        let source_width = if instruction.operand_bits == 64 { 64 } else { 32 };
        let source = integer_source(context, source_width)?;
        let result = finish(
            context,
            convert_binary(
                &from_integer(&source),
                if width == 64 { BINARY64 } else { BINARY32 },
                mode,
            )?,
        );
        signal(context, result.flags)?;
        let bits = encode_binary(
            &result.value,
            if width == 64 {
                BinaryWidth::W64
            } else {
                BinaryWidth::W32
            },
        );
        let mut lane_bytes = context.state.simd.xmm[start..start + 16].to_vec();
        set_lane(&mut lane_bytes, 0, width as u32, &bits);
        context.state.simd.xmm[start..start + 16].copy_from_slice(&lane_bytes);
        return Ok(());
    }
    if opcode == 0x2c || opcode == 0x2d {
        if !matches!(instruction.prefix, NumericPrefix::XF2 | NumericPrefix::XF3) {
            return Err(NumericError::unsupported("MMX float conversion is unsupported"));
        }
        let width = if matches!(instruction.prefix, NumericPrefix::XF2) {
            64
        } else {
            32
        };
        let output_width = if instruction.operand_bits == 64 { 64 } else { 32 };
        let bytes = load(context, width / 8, false)?;
        let input = source_value(context, &bytes, width as u32, 0);
        let result = integer_conversion(
            &input,
            output_width as u32,
            if opcode == 0x2c {
                super::binary::Rounding::Zero
            } else {
                mode
            },
        )?;
        signal(context, result.flags)?;
        context
            .state
            .registers
            .write(
                general_register(instruction.register_index)?,
                if output_width == 64 {
                    crate::core::contracts::GuestIntegerWidth::B64
                } else {
                    crate::core::contracts::GuestIntegerWidth::B32
                },
                result.value.as_uint_n(output_width as u32).low_u64(),
                false,
            )
            .map_err(NumericError::Guest)?;
        return Ok(());
    }
    let mut output = context.state.simd.xmm[start..start + 16].to_vec();
    let mut flags = 0;
    if opcode == 0x5a {
        let scalar = matches!(instruction.prefix, NumericPrefix::XF2 | NumericPrefix::XF3);
        let source_width = if matches!(instruction.prefix, NumericPrefix::X66 | NumericPrefix::XF2) {
            64
        } else {
            32
        };
        let target_width = if source_width == 32 { 64 } else { 32 };
        let count = if scalar { 1 } else { 2 };
        let source = load(context, count * source_width / 8, true)?;
        if !scalar {
            output.fill(0);
        }
        for index in 0..count {
            let value = source_value(context, &source, source_width as u32, index * source_width / 8);
            let denormal = matches!(value, BinaryValue::Finite { denormal: true, .. });
            let result = finish(
                context,
                convert_binary(&value, if target_width == 64 { BINARY64 } else { BINARY32 }, mode)?,
            );
            flags |= result.flags | if denormal { FLAG_DENORMAL_OPERAND } else { 0 };
            set_lane(
                &mut output,
                index,
                target_width as u32,
                &encode_binary(
                    &result.value,
                    if target_width == 64 {
                        BinaryWidth::W64
                    } else {
                        BinaryWidth::W32
                    },
                ),
            );
        }
    } else if opcode == 0x5b {
        let source = load(context, 16, true)?;
        for index in 0..4 {
            if matches!(instruction.prefix, NumericPrefix::None) {
                let result = convert_binary(&from_integer(&lane(&source, index, 32).as_int_n(32)), BINARY32, mode)?;
                flags |= result.flags;
                set_lane(&mut output, index, 32, &encode_binary(&result.value, BinaryWidth::W32));
            } else if matches!(instruction.prefix, NumericPrefix::X66 | NumericPrefix::XF3) {
                let result = integer_conversion(
                    &source_value(context, &source, 32, index * 4),
                    32,
                    if matches!(instruction.prefix, NumericPrefix::XF3) {
                        super::binary::Rounding::Zero
                    } else {
                        mode
                    },
                )?;
                flags |= result.flags;
                set_lane(&mut output, index, 32, &result.value);
            } else {
                return Err(NumericError::unsupported("Reserved 5B conversion prefix"));
            }
        }
    } else if opcode == 0xe6 {
        if matches!(instruction.prefix, NumericPrefix::XF3) {
            let source = load(context, 8, false)?;
            for index in 0..2 {
                let result = convert_binary(&from_integer(&lane(&source, index, 32).as_int_n(32)), BINARY64, mode)?;
                flags |= result.flags;
                set_lane(&mut output, index, 64, &encode_binary(&result.value, BinaryWidth::W64));
            }
        } else if matches!(instruction.prefix, NumericPrefix::X66 | NumericPrefix::XF2) {
            let source = load(context, 16, true)?;
            output.fill(0);
            for index in 0..2 {
                let result = integer_conversion(
                    &source_value(context, &source, 64, index * 8),
                    32,
                    if matches!(instruction.prefix, NumericPrefix::X66) {
                        super::binary::Rounding::Zero
                    } else {
                        mode
                    },
                )?;
                flags |= result.flags;
                set_lane(&mut output, index, 32, &result.value);
            }
        } else {
            return Err(NumericError::unsupported("Reserved E6 conversion prefix"));
        }
    } else {
        return Err(NumericError::unsupported("Unsupported conversion opcode"));
    }
    signal(context, flags)?;
    context.state.simd.xmm[start..start + 16].copy_from_slice(&output);
    Ok(())
}

fn packed_integer(context: &mut NumericExecutionContext, opcode: u8) -> Result<(), NumericError> {
    let instruction = context.instruction;
    if !matches!(instruction.prefix, NumericPrefix::X66) {
        return Err(NumericError::unsupported(
            "MMX packed integer instruction is unsupported",
        ));
    }
    check_xmm(context.state, instruction.register_index)?;
    let start = instruction.register_index * 16;
    let a = context.state.simd.xmm[start..start + 16].to_vec();
    let b = load(context, 16, true)?;
    let mut output = vec![0u8; 16];
    let add_width = match opcode {
        0xfc => 8,
        0xfd => 16,
        0xfe => 32,
        0xd4 => 64,
        _ => 0,
    };
    let subtract_width = match opcode {
        0xf8 => 8,
        0xf9 => 16,
        0xfa => 32,
        0xfb => 64,
        _ => 0,
    };
    if add_width != 0 || subtract_width != 0 {
        let width = if add_width != 0 { add_width } else { subtract_width };
        for index in 0..128 / width {
            let value = if add_width != 0 {
                lane(&a, index, width as u32).add(&lane(&b, index, width as u32))
            } else {
                lane(&a, index, width as u32).sub(&lane(&b, index, width as u32))
            };
            set_lane(&mut output, index, width as u32, &value);
        }
    } else if (0x64..=0x66).contains(&opcode) || (0x74..=0x76).contains(&opcode) {
        let width = match opcode % 16 {
            4 => 8,
            5 => 16,
            _ => 32,
        };
        for index in 0..128 / width {
            let left = lane(&a, index, width as u32).as_int_n(width as u32);
            let right = lane(&b, index, width as u32).as_int_n(width as u32);
            let yes = if opcode < 0x70 { left > right } else { left == right };
            set_lane(
                &mut output,
                index,
                width as u32,
                &if yes {
                    BigInt::zero().sub(&BigInt::one())
                } else {
                    BigInt::zero()
                },
            );
        }
    } else if (0x60..=0x62).contains(&opcode) || (0x68..=0x6a).contains(&opcode) || opcode == 0x6c || opcode == 0x6d {
        let width = if opcode == 0x6c || opcode == 0x6d {
            64
        } else {
            match opcode & 7 {
                0 => 8,
                1 => 16,
                _ => 32,
            }
        };
        let high = opcode >= 0x68 && opcode != 0x6c;
        let base = if high { 64 / width } else { 0 };
        for index in 0..64 / width {
            let left = lane(&a, base + index, width as u32);
            let right = lane(&b, base + index, width as u32);
            set_lane(&mut output, index * 2, width as u32, &left);
            set_lane(&mut output, index * 2 + 1, width as u32, &right);
        }
    } else if opcode == 0xd5 || opcode == 0xe4 || opcode == 0xe5 {
        for index in 0..8 {
            let left = if opcode == 0xe5 {
                lane(&a, index, 16).as_int_n(16)
            } else {
                lane(&a, index, 16)
            };
            let right = if opcode == 0xe5 {
                lane(&b, index, 16).as_int_n(16)
            } else {
                lane(&b, index, 16)
            };
            let product = left.mul(&right);
            let value = if opcode == 0xd5 { product } else { product.shr_bits(16) };
            set_lane(&mut output, index, 16, &value);
        }
    } else if opcode == 0xf4 {
        set_lane(&mut output, 0, 64, &lane(&a, 0, 32).mul(&lane(&b, 0, 32)));
        set_lane(&mut output, 1, 64, &lane(&a, 2, 32).mul(&lane(&b, 2, 32)));
    } else if opcode == 0xf5 {
        for index in 0..4 {
            let value = lane(&a, index * 2, 16)
                .as_int_n(16)
                .mul(&lane(&b, index * 2, 16).as_int_n(16))
                .add(
                    &lane(&a, index * 2 + 1, 16)
                        .as_int_n(16)
                        .mul(&lane(&b, index * 2 + 1, 16).as_int_n(16)),
                );
            set_lane(&mut output, index, 32, &value);
        }
    } else if opcode == 0xf6 {
        for half in 0..2 {
            let mut sum = BigInt::zero();
            let one = BigInt::one();
            for index in half * 8..half * 8 + 8 {
                let delta = lane(&a, index, 8).sub(&lane(&b, index, 8));
                sum = sum.add(&if delta.is_negative() {
                    one.mul(&delta.negated())
                } else {
                    delta
                });
            }
            set_lane(&mut output, half, 64, &sum);
        }
    } else if matches!(opcode, 0xd1 | 0xd2 | 0xd3 | 0xe1 | 0xe2 | 0xf1 | 0xf2 | 0xf3) {
        let width: u32 = match opcode & 15 {
            1 => 16,
            2 => 32,
            _ => 64,
        };
        let count = lane(&b, 0, 64);
        let width_big = BigInt::from_u64(width as u64);
        for index in 0..(128 / width) as usize {
            let value = lane(&a, index, width);
            let signed = value.as_int_n(width);
            let shifted = if opcode >= 0xf0 {
                if count >= width_big {
                    BigInt::zero()
                } else {
                    value.shl_bits(count.low_u64() as u32)
                }
            } else if opcode >= 0xe0 {
                if count >= width_big {
                    if signed.is_negative() {
                        BigInt::zero().sub(&BigInt::one())
                    } else {
                        BigInt::zero()
                    }
                } else if signed.is_negative() {
                    // Arithmetic right shift of a negative value.
                    let magnitude = signed.negated();
                    let shifted = magnitude.shr_bits(count.low_u64() as u32);
                    let dropped = !magnitude.as_uint_n(count.low_u64() as u32).is_zero();
                    shifted
                        .negated()
                        .sub(&if dropped { BigInt::one() } else { BigInt::zero() })
                } else {
                    signed.shr_bits(count.low_u64() as u32)
                }
            } else if count >= width_big {
                BigInt::zero()
            } else {
                value.shr_bits(count.low_u64() as u32)
            };
            set_lane(&mut output, index, width, &shifted);
        }
    } else {
        return Err(NumericError::unsupported(format!(
            "Unsupported packed integer opcode {opcode:x}"
        )));
    }
    context.state.simd.xmm[start..start + 16].copy_from_slice(&output);
    Ok(())
}

fn execute(context: &mut NumericExecutionContext) -> Result<(), NumericError> {
    let instruction = context.instruction;
    let Some(opcode) = instruction.secondary_opcode else {
        return Err(NumericError::unsupported("Missing SSE secondary opcode"));
    };
    if matches!(
        opcode,
        0x14 | 0x15 | 0x28 | 0x29 | 0x2e | 0x2f | 0x50 | 0x54 | 0x55 | 0x56 | 0x57 | 0xc6
    ) && !matches!(instruction.prefix, NumericPrefix::None | NumericPrefix::X66)
    {
        return Err(NumericError::unsupported(
            "Reserved mandatory prefix for SSE instruction",
        ));
    }
    if opcode == 0xae {
        let source = operand(context)?;
        let NumericOperand::Memory(address) = source else {
            return Err(NumericError::unsupported("MXCSR instruction requires memory"));
        };
        let Some(modrm) = instruction.modrm else {
            return Err(NumericError::unsupported("MXCSR instruction requires memory"));
        };
        match (modrm >> 3) & 7 {
            2 => {
                let bytes = context.memory.copy(address, 4)?;
                let value = read_bits(&bytes).low_u64() as u32;
                if u64::from(value) & !u64::from(context.state.simd.mxcsr_mask) != 0 {
                    return Err(NumericError::fault(13, "Reserved MXCSR bits are set"));
                }
                context.state.simd.mxcsr = value;
            }
            3 => {
                let value = context.state.simd.mxcsr;
                context
                    .memory
                    .write(address, &write_bits(&BigInt::from_u64(u64::from(value)), 4))?;
            }
            _ => {
                return Err(NumericError::unsupported(
                    "FXSAVE/FXRSTOR/fence opcode is unsupported by the numeric executor",
                ));
            }
        }
        return Ok(());
    }
    if matches!(opcode, 0x2a | 0x2c | 0x2d | 0x5a | 0x5b | 0xe6) {
        return conversions(context, opcode);
    }
    if let Some(raw) = prepare_raw_sse(opcode, instruction.prefix, instruction.register_index) {
        let source = operand(context)?;
        return execute_raw_sse(raw, source, context.state, context.memory);
    }
    check_xmm(context.state, instruction.register_index)?;
    if opcode == 0x6f || opcode == 0x7f {
        return Err(NumericError::unsupported("MMX move is unsupported"));
    }
    if opcode == 0x6e || opcode == 0x7e {
        if opcode == 0x7e && matches!(instruction.prefix, NumericPrefix::XF3) {
            let bytes = load(context, 8, false)?;
            let start = instruction.register_index * 16;
            context.state.simd.xmm[start..start + 16].fill(0);
            context.state.simd.xmm[start..start + 8].copy_from_slice(&bytes);
            return Ok(());
        }
        if !matches!(instruction.prefix, NumericPrefix::X66) {
            return Err(NumericError::unsupported("MMX MOVD/MOVQ is unsupported"));
        }
        let width: usize = if instruction.operand_bits == 64 { 64 } else { 32 };
        if opcode == 0x6e {
            let value = integer_source(context, width as u32)?;
            let start = instruction.register_index * 16;
            context.state.simd.xmm[start..start + 16].fill(0);
            let encoded = write_bits(&value, width / 8);
            context.state.simd.xmm[start..start + encoded.len()].copy_from_slice(&encoded);
        } else {
            let target = operand(context)?;
            let start = instruction.register_index * 16;
            let bits = read_bits(&context.state.simd.xmm[start..start + width / 8]);
            match target {
                NumericOperand::Register(index) => {
                    context
                        .state
                        .registers
                        .write(
                            general_register(index)?,
                            if width == 64 {
                                crate::core::contracts::GuestIntegerWidth::B64
                            } else {
                                crate::core::contracts::GuestIntegerWidth::B32
                            },
                            bits.low_u64(),
                            false,
                        )
                        .map_err(NumericError::Guest)?;
                }
                NumericOperand::Memory(address) => {
                    context.memory.write(address, &write_bits(&bits, width / 8))?;
                }
            }
        }
        return Ok(());
    }
    if opcode == 0xd6 && matches!(instruction.prefix, NumericPrefix::X66) {
        let target = operand(context)?;
        let start = instruction.register_index * 16;
        let bytes = context.state.simd.xmm[start..start + 8].to_vec();
        match target {
            NumericOperand::Register(index) => {
                check_xmm(context.state, index)?;
                let other = index * 16;
                context.state.simd.xmm[other..other + 16].fill(0);
                context.state.simd.xmm[other..other + 8].copy_from_slice(&bytes);
            }
            NumericOperand::Memory(address) => {
                context.memory.write(address, &bytes)?;
            }
        }
        return Ok(());
    }
    if matches!(opcode, 0x12 | 0x13 | 0x16 | 0x17) {
        if !matches!(instruction.prefix, NumericPrefix::None | NumericPrefix::X66) {
            return Err(NumericError::unsupported("SSE3 duplicate move is unsupported"));
        }
        let source = operand(context)?;
        let high = opcode == 0x16 || opcode == 0x17;
        let start = instruction.register_index * 16;
        if opcode == 0x13 || opcode == 0x17 {
            let NumericOperand::Memory(address) = source else {
                return Err(NumericError::unsupported("MOVL/MOVH store requires memory"));
            };
            let bytes =
                context.state.simd.xmm[start + if high { 8 } else { 0 }..start + if high { 16 } else { 8 }].to_vec();
            context.memory.write(address, &bytes)?;
        } else if let NumericOperand::Memory(address) = source {
            let bytes = context.memory.copy(address, 8)?;
            let at = start + if high { 8 } else { 0 };
            context.state.simd.xmm[at..at + 8].copy_from_slice(&bytes);
        } else if let NumericOperand::Register(index) = source {
            if matches!(instruction.prefix, NumericPrefix::X66) {
                return Err(NumericError::unsupported("MOVLPD/MOVHPD load requires memory"));
            }
            check_xmm(context.state, index)?;
            let other = index * 16;
            let bytes =
                context.state.simd.xmm[other + if high { 0 } else { 8 }..other + if high { 8 } else { 16 }].to_vec();
            let at = start + if high { 8 } else { 0 };
            context.state.simd.xmm[at..at + 8].copy_from_slice(&bytes);
        }
        return Ok(());
    }
    if matches!(opcode, 0x14 | 0x15 | 0xc6 | 0x70) {
        let source = load(context, 16, true)?;
        let start = instruction.register_index * 16;
        let original = context.state.simd.xmm[start..start + 16].to_vec();
        let mut output = original.clone();
        if opcode == 0x70 {
            let Some(immediate) = instruction.immediate else {
                return Err(NumericError::unsupported("Shuffle requires immediate"));
            };
            if matches!(instruction.prefix, NumericPrefix::X66) {
                for index in 0..4 {
                    let selected = lane(&source, ((immediate >> (2 * index)) & 3) as usize, 32);
                    set_lane(&mut output, index, 32, &selected);
                }
            } else if matches!(instruction.prefix, NumericPrefix::XF2 | NumericPrefix::XF3) {
                output.copy_from_slice(&source);
                let base = if matches!(instruction.prefix, NumericPrefix::XF3) {
                    4
                } else {
                    0
                };
                for index in 0..4 {
                    let selected = lane(&source, base + ((immediate >> (2 * index)) & 3) as usize, 16);
                    set_lane(&mut output, base + index, 16, &selected);
                }
            } else {
                return Err(NumericError::unsupported("MMX shuffle is unsupported"));
            }
        } else {
            let width = if matches!(instruction.prefix, NumericPrefix::X66) {
                64
            } else {
                32
            };
            if opcode == 0xc6 {
                let Some(immediate) = instruction.immediate else {
                    return Err(NumericError::unsupported("Shuffle requires immediate"));
                };
                for index in 0..128 / width {
                    let from = if index < 64 / width { &original } else { &source };
                    let selected =
                        (immediate >> (index * if width == 32 { 2 } else { 1 })) & if width == 32 { 3 } else { 1 };
                    let value = lane(from, selected as usize, width as u32);
                    set_lane(&mut output, index, width as u32, &value);
                }
            } else {
                let base = if opcode == 0x15 { 64 / width } else { 0 };
                for index in 0..64 / width {
                    let left = lane(&original, base + index, width as u32);
                    let right = lane(&source, base + index, width as u32);
                    set_lane(&mut output, index * 2, width as u32, &left);
                    set_lane(&mut output, index * 2 + 1, width as u32, &right);
                }
            }
        }
        context.state.simd.xmm[start..start + 16].copy_from_slice(&output);
        return Ok(());
    }
    if opcode == 0x50 || opcode == 0xd7 {
        if !matches!(operand(context)?, NumericOperand::Register(_)) {
            return Err(NumericError::unsupported("MOVMSK/PMOVMSKB requires a register source"));
        }
        let source = load(context, 16, true)?;
        let width = if opcode == 0xd7 {
            if !matches!(instruction.prefix, NumericPrefix::X66) {
                return Err(NumericError::unsupported("MMX PMOVMSKB is unsupported"));
            }
            8
        } else if matches!(instruction.prefix, NumericPrefix::X66) {
            64
        } else {
            32
        };
        let mut mask = BigInt::zero();
        for index in 0..128 / width {
            let bit = lane(&source, index, width as u32)
                .shr_bits(width as u32 - 1)
                .as_uint_n(1);
            if !bit.is_zero() {
                mask = mask.add(&BigInt::one().shl_bits(index as u32));
            }
        }
        context
            .state
            .registers
            .write(
                general_register(instruction.register_index)?,
                crate::core::contracts::GuestIntegerWidth::B32,
                mask.low_u64(),
                false,
            )
            .map_err(NumericError::Guest)?;
        return Ok(());
    }
    if opcode == 0x2e || opcode == 0x2f {
        let width = if matches!(instruction.prefix, NumericPrefix::X66) {
            64
        } else {
            32
        };
        let start = instruction.register_index * 16;
        let destination = context.state.simd.xmm[start..start + 16].to_vec();
        let left = source_value(context, &destination, width as u32, 0);
        let bytes = load(context, width / 8, false)?;
        let right = source_value(context, &bytes, width as u32, 0);
        let comparison = compare_binary(&left, &right);
        let nan = matches!(left, BinaryValue::Nan { signaling, .. } if opcode == 0x2f || signaling)
            || matches!(right, BinaryValue::Nan { signaling, .. } if opcode == 0x2f || signaling);
        let denormal = matches!(left, BinaryValue::Finite { denormal: true, .. })
            || matches!(right, BinaryValue::Finite { denormal: true, .. });
        signal(
            context,
            if nan {
                FLAG_INVALID
            } else if denormal {
                FLAG_DENORMAL_OPERAND
            } else {
                0
            },
        )?;
        let flags = &mut context.state.flags;
        flags.set(
            GuestFlag::Carry,
            comparison == super::binary::BinaryComparison::Less
                || comparison == super::binary::BinaryComparison::Unordered,
        );
        flags.set(
            GuestFlag::Zero,
            comparison == super::binary::BinaryComparison::Equal
                || comparison == super::binary::BinaryComparison::Unordered,
        );
        flags.set(
            GuestFlag::Parity,
            comparison == super::binary::BinaryComparison::Unordered,
        );
        flags.set(GuestFlag::Overflow, false);
        flags.set(GuestFlag::AuxiliaryCarry, false);
        flags.set(GuestFlag::Sign, false);
        return Ok(());
    }
    if matches!(opcode, 0x51 | 0x58 | 0x59 | 0x5c | 0x5d | 0x5e | 0x5f | 0xc2) {
        return floating(context, opcode);
    }
    if opcode == 0xc4 || opcode == 0xc5 {
        if !matches!(instruction.prefix, NumericPrefix::X66) || instruction.immediate.is_none() {
            return Err(NumericError::unsupported(
                "PINSRW/PEXTRW requires SSE2 prefix and immediate",
            ));
        }
        let index = (instruction.immediate.unwrap_or(0) & 7) as usize;
        if opcode == 0xc4 {
            let source = operand(context)?;
            let value = match source {
                NumericOperand::Register(source_index) => BigInt::from_u64(
                    context
                        .state
                        .registers
                        .read(
                            general_register(source_index)?,
                            crate::core::contracts::GuestIntegerWidth::B32,
                            false,
                        )
                        .map_err(NumericError::Guest)?,
                ),
                NumericOperand::Memory(address) => {
                    let bytes = context.memory.copy(address, 2)?;
                    read_bits(&bytes)
                }
            };
            let start = instruction.register_index * 16;
            let mut destination = context.state.simd.xmm[start..start + 16].to_vec();
            set_lane(&mut destination, index, 16, &value);
            context.state.simd.xmm[start..start + 16].copy_from_slice(&destination);
        } else {
            if !matches!(operand(context)?, NumericOperand::Register(_)) {
                return Err(NumericError::unsupported("PEXTRW requires XMM source"));
            }
            let source = load(context, 16, true)?;
            context
                .state
                .registers
                .write(
                    general_register(instruction.register_index)?,
                    crate::core::contracts::GuestIntegerWidth::B32,
                    lane(&source, index, 16).low_u64(),
                    false,
                )
                .map_err(NumericError::Guest)?;
        }
        return Ok(());
    }
    if matches!(opcode, 0x71..=0x73) {
        let target = operand(context)?;
        if !matches!(instruction.prefix, NumericPrefix::X66)
            || instruction.immediate.is_none()
            || instruction.modrm.is_none()
            || !matches!(target, NumericOperand::Register(_))
        {
            return Err(NumericError::unsupported(
                "Immediate packed shift requires SSE2 register and immediate",
            ));
        }
        let group = (instruction.modrm.unwrap_or(0) >> 3) & 7;
        let count = u32::from(instruction.immediate.unwrap_or(0));
        let NumericOperand::Register(target_index) = target else {
            return Err(NumericError::unsupported(
                "Immediate packed shift requires SSE2 register and immediate",
            ));
        };
        check_xmm(context.state, target_index)?;
        let start = target_index * 16;
        let original = context.state.simd.xmm[start..start + 16].to_vec();
        if opcode == 0x73 && (group == 3 || group == 7) {
            let shifted = if count >= 16 {
                BigInt::zero()
            } else if group == 3 {
                read_bits(&original).shr_bits(count * 8)
            } else {
                read_bits(&original).shl_bits(count * 8)
            };
            let encoded = write_bits(&shifted, 16);
            context.state.simd.xmm[start..start + 16].copy_from_slice(&encoded);
            return Ok(());
        }
        if !matches!(group, 2 | 4 | 6) {
            return Err(NumericError::unsupported("Reserved packed shift group"));
        }
        let width: u32 = if opcode == 0x71 {
            16
        } else if opcode == 0x72 {
            32
        } else {
            64
        };
        if width == 64 && group == 4 {
            return Err(NumericError::unsupported("PSRAQ is unavailable in legacy SSE2"));
        }
        let mut register = original.clone();
        for index in 0..(128 / width) as usize {
            let value = lane(&original, index, width);
            let signed = value.as_int_n(width);
            let shifted = if group == 4 {
                if count >= width {
                    if signed.is_negative() {
                        BigInt::zero().sub(&BigInt::one())
                    } else {
                        BigInt::zero()
                    }
                } else if signed.is_negative() {
                    let magnitude = signed.negated();
                    let shifted = magnitude.shr_bits(count);
                    let dropped = !magnitude.as_uint_n(count).is_zero();
                    shifted
                        .negated()
                        .sub(&if dropped { BigInt::one() } else { BigInt::zero() })
                } else {
                    signed.shr_bits(count)
                }
            } else if count >= width {
                BigInt::zero()
            } else if group == 2 {
                value.shr_bits(count)
            } else {
                value.shl_bits(count)
            };
            set_lane(&mut register, index, width, &shifted);
        }
        context.state.simd.xmm[start..start + 16].copy_from_slice(&register);
        return Ok(());
    }
    packed_integer(context, opcode)
}

/// Execute one `0x0f`-escaped SSE instruction.
pub fn execute_sse(context: NumericExecutionContext) -> Result<(), NumericError> {
    let mut context = context;
    execute(&mut context)
}
