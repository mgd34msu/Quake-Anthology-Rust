//! x87 floating-point instruction execution.
//!
//! Donor: `src/guest/floating-point/x87.ts`. Eight physical 80-bit slots with
//! TOP in the status word and source tag bits; deferred masked exceptions
//! retire normally while unmasked ones raise vector 16 at the next waiting
//! instruction.

use crate::core::contracts::GuestFlag;

use super::binary::{
    arithmetic, compare_binary, convert_binary, decode_binary, encode_binary, format_for,
    from_integer, indefinite, integer_conversion, read_bits, round_integral, rounding,
    square_root, write_bits, zero, BigInt, BinaryComparison, BinaryFormat, BinaryOperation,
    BinaryResult, BinaryValue, BinaryWidth, Rounding, BINARY80, FLAG_DENORMAL_OPERAND,
    FLAG_INVALID, FLAG_OVERFLOW, FLAG_PRECISION, FLAG_UNDERFLOW,
};
use super::contracts::{NumericError, NumericExecutionContext, NumericOperand};
use super::trigonometric::{x87_arctangent, x87_trigonometric, X87TrigonometricResult};
use crate::core::registers::GuestX87State;
use crate::error::GuestError;

/// Internal x87 control flow: deferred unmasked exceptions abort the
/// instruction but still retire it.
#[derive(Debug)]
enum X87Error {
    /// Unmasked exception deferred to the next waiting instruction.
    Deferred,
    /// Numeric failure.
    Numeric(NumericError),
}

impl From<NumericError> for X87Error {
    fn from(error: NumericError) -> Self {
        Self::Numeric(error)
    }
}

impl From<GuestError> for X87Error {
    fn from(error: GuestError) -> Self {
        Self::Numeric(NumericError::Guest(error))
    }
}

fn top(state: &GuestX87State) -> usize {
    ((state.status_word >> 11) & 7) as usize
}

fn physical(state: &GuestX87State, index: usize) -> usize {
    (top(state) + index) & 7
}

fn tag(state: &GuestX87State, slot: usize) -> u16 {
    (state.tag_word >> (slot * 2)) & 3
}

fn set_tag(state: &mut GuestX87State, slot: usize, value: u16) {
    state.tag_word = (state.tag_word & !(3 << (slot * 2))) | ((value & 3) << (slot * 2));
}

fn set_top(state: &mut GuestX87State, value: usize) {
    state.status_word = (state.status_word & !0x3800) | (((value & 7) as u16) << 11);
}

/// Raise status flags; unmasked non-precision exceptions defer, aborting the
/// instruction while still retiring it.
fn raise(
    state: &mut GuestX87State,
    flags: u32,
    rounded_up: bool,
    register_wrapped: bool,
) -> Result<(), X87Error> {
    state.status_word =
        (state.status_word & !0x200) | (u16::from(rounded_up) << 9) | flags as u16;
    let unmasked = flags & !(state.control_word as u32) & 63;
    if unmasked != 0 {
        state.status_word |= 0x8080;
        if unmasked
            & !(FLAG_PRECISION
                | if register_wrapped {
                    FLAG_OVERFLOW | FLAG_UNDERFLOW
                } else {
                    0
                })
            != 0
        {
            return Err(X87Error::Deferred);
        }
    }
    Ok(())
}

fn stack_fault(state: &mut GuestX87State, overflowed: bool) -> Result<(), X87Error> {
    raise(state, FLAG_INVALID | 64, overflowed, false)
}

/// Read stack register `index` (0 is TOP).
pub fn read_x87_register(
    state: &mut GuestX87State,
    index: usize,
) -> Result<BinaryValue, NumericError> {
    read_register(state, index).map_err(|error| match error {
        X87Error::Deferred => NumericError::fault(16, "Deferred x87 stack fault"),
        X87Error::Numeric(error) => error,
    })
}

fn read_register(state: &mut GuestX87State, index: usize) -> Result<BinaryValue, X87Error> {
    let slot = physical(state, index);
    if tag(state, slot) == 3 {
        stack_fault(state, false)?;
        return Ok(indefinite());
    }
    Ok(decode_binary(
        &read_bits(&state.registers[slot * 10..slot * 10 + 10]),
        BinaryWidth::W80,
    ))
}

/// Write stack register `index`, updating its tag.
pub fn write_x87_register(state: &mut GuestX87State, index: usize, value: &BinaryValue) {
    let slot = physical(state, index);
    let bits = encode_binary(value, BinaryWidth::W80);
    let bytes = write_bits(&bits, 10);
    state.registers[slot * 10..slot * 10 + 10].copy_from_slice(&bytes);
    let tag_value = match value {
        BinaryValue::Finite {
            coefficient,
            denormal,
            ..
        } => {
            if coefficient.is_zero() {
                1
            } else if *denormal {
                2
            } else {
                0
            }
        }
        _ => 2,
    };
    set_tag(state, slot, tag_value);
}

fn raw_register(state: &mut GuestX87State, index: usize) -> Result<BigInt, X87Error> {
    let slot = physical(state, index);
    if tag(state, slot) == 3 {
        stack_fault(state, false)?;
        return Ok(encode_binary(&indefinite(), BinaryWidth::W80));
    }
    Ok(read_bits(&state.registers[slot * 10..slot * 10 + 10]))
}

fn write_raw_register(state: &mut GuestX87State, index: usize, bits: &BigInt) {
    let value = decode_binary(bits, BinaryWidth::W80);
    write_x87_register(state, index, &value);
    let bytes = write_bits(bits, 10);
    let slot = physical(state, index);
    state.registers[slot * 10..slot * 10 + 10].copy_from_slice(&bytes);
    let exponent_zero = bits.shr_bits(64).low_u64() as u16 & 0x7fff == 0;
    if exponent_zero && !bits.as_uint_n(64).is_zero() {
        set_tag(state, physical(state, index), 2);
    }
}

fn push_raw(state: &mut GuestX87State, bits: &BigInt) -> Result<(), X87Error> {
    let next = top(state).wrapping_sub(1) & 7;
    let bits = if tag(state, next) != 3 {
        stack_fault(state, true)?;
        encode_binary(&indefinite(), BinaryWidth::W80)
    } else {
        state.status_word &= !0x200;
        bits.clone()
    };
    set_top(state, next);
    write_raw_register(state, 0, &bits);
    Ok(())
}

/// Push a value onto the register stack.
pub fn push_x87(state: &mut GuestX87State, value: &BinaryValue) -> Result<(), NumericError> {
    push_value(state, value).map_err(|error| match error {
        X87Error::Deferred => NumericError::fault(16, "Deferred x87 stack fault"),
        X87Error::Numeric(error) => error,
    })
}

fn push_value(state: &mut GuestX87State, value: &BinaryValue) -> Result<(), X87Error> {
    let next = top(state).wrapping_sub(1) & 7;
    let value = if tag(state, next) != 3 {
        stack_fault(state, true)?;
        indefinite()
    } else {
        state.status_word &= !0x200;
        value.clone()
    };
    set_top(state, next);
    write_x87_register(state, 0, &value);
    Ok(())
}

/// Pop the register stack.
pub fn pop_x87(state: &mut GuestX87State) {
    let current = top(state);
    set_tag(state, current, 3);
    set_top(state, current + 1);
}

/// Reset the x87 unit (`FINIT`).
pub fn initialize_x87(state: &mut GuestX87State) {
    state.control_word = 0x37f;
    state.status_word = 0;
    state.tag_word = 0xffff;
    state.last_opcode = 0;
    state.instruction_pointer = 0;
    state.data_pointer = 0;
    state.instruction_selector = 0;
    state.data_selector = 0;
}

fn result_format(state: &GuestX87State) -> Result<BinaryFormat, NumericError> {
    match (state.control_word >> 8) & 3 {
        0 => Ok(BinaryFormat {
            precision: 24,
            ..BINARY80
        }),
        2 => Ok(BinaryFormat {
            precision: 53,
            ..BINARY80
        }),
        3 => Ok(BINARY80),
        _ => Err(NumericError::unsupported(
            "Reserved x87 precision-control encoding",
        )),
    }
}

fn commit(
    state: &mut GuestX87State,
    index: usize,
    result: &BinaryResult,
    register_wrapped: bool,
) -> Result<(), X87Error> {
    raise(state, result.flags, result.rounded_up, register_wrapped)?;
    write_x87_register(state, index, &result.value);
    Ok(())
}

/// Push an ABI floating-point return value.
pub fn write_x87_return(
    state: &mut GuestX87State,
    value: f64,
    storage: BinaryWidth,
) -> Result<(), NumericError> {
    let decoded = match storage {
        BinaryWidth::W32 => decode_binary(
            &BigInt::from_u64(u64::from((value as f32).to_bits())),
            BinaryWidth::W32,
        ),
        _ => decode_binary(&BigInt::from_u64(value.to_bits()), BinaryWidth::W64),
    };
    let converted = convert_binary(
        &decoded,
        BINARY80,
        rounding(u32::from(state.control_word >> 10)),
    )?;
    raise(state, converted.flags, false, false).map_err(|error| match error {
        X87Error::Deferred => NumericError::fault(16, "Unmasked x87 return exception"),
        X87Error::Numeric(error) => error,
    })?;
    push_x87(state, &converted.value)
}

/// Read and convert the ABI floating-point return value (the caller pops).
pub fn read_x87_return(
    state: &mut GuestX87State,
    storage: BinaryWidth,
) -> Result<f64, NumericError> {
    let value = read_x87_register(state, 0)?;
    let result = convert_binary(
        &value,
        format_for(storage),
        rounding(u32::from(state.control_word >> 10)),
    )?;
    let bits = encode_binary(&result.value, storage);
    Ok(match storage {
        BinaryWidth::W32 => f64::from(f32::from_bits(bits.low_u64() as u32)),
        _ => f64::from_bits(bits.low_u64()),
    })
}

fn compare(
    context: &mut NumericExecutionContext,
    right: &BinaryValue,
    unordered_quiet: bool,
    integer_flags: bool,
) -> Result<(), X87Error> {
    let left = read_register(&mut context.state.x87, 0)?;
    let ordering = compare_binary(&left, right);
    let invalid_operand = matches!(left, BinaryValue::Unsupported { .. })
        || matches!(right, BinaryValue::Unsupported { .. })
        || matches!(left, BinaryValue::Nan { signaling, .. } if !unordered_quiet || signaling)
        || matches!(right, BinaryValue::Nan { signaling: true, .. })
        || (!unordered_quiet && matches!(right, BinaryValue::Nan { .. }));
    let denormal = matches!(left, BinaryValue::Finite { denormal: true, .. })
        || matches!(right, BinaryValue::Finite { denormal: true, .. });
    raise(
        &mut context.state.x87,
        if invalid_operand {
            FLAG_INVALID
        } else if denormal {
            FLAG_DENORMAL_OPERAND
        } else {
            0
        },
        false,
        false,
    )?;
    if integer_flags {
        let flags = &mut context.state.flags;
        flags.set(
            GuestFlag::Carry,
            ordering == BinaryComparison::Less || ordering == BinaryComparison::Unordered,
        );
        flags.set(
            GuestFlag::Zero,
            ordering == BinaryComparison::Equal || ordering == BinaryComparison::Unordered,
        );
        flags.set(GuestFlag::Parity, ordering == BinaryComparison::Unordered);
        flags.set(GuestFlag::Overflow, false);
        flags.set(GuestFlag::Sign, false);
        flags.set(GuestFlag::AuxiliaryCarry, false);
    } else {
        let state = &mut context.state.x87;
        state.status_word = (state.status_word & !0x4500)
            | match ordering {
                BinaryComparison::Less => 0x100,
                BinaryComparison::Equal => 0x4000,
                BinaryComparison::Unordered => 0x4500,
                BinaryComparison::Greater => 0,
            };
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn binary_instruction(
    context: &mut NumericExecutionContext,
    group: u8,
    right: &BinaryValue,
    destination: usize,
    reversed: bool,
    pop: bool,
) -> Result<(), X87Error> {
    if group == 2 || group == 3 {
        compare(context, right, false, false)?;
        if group == 3 {
            pop_x87(&mut context.state.x87);
        }
        return Ok(());
    }
    let operation = match group {
        0 => BinaryOperation::Add,
        1 => BinaryOperation::Multiply,
        4 | 5 => BinaryOperation::Subtract,
        _ => BinaryOperation::Divide,
    };
    let left = read_register(&mut context.state.x87, destination)?;
    let reverse = (group == 5 || group == 7) != reversed;
    let format = result_format(&context.state.x87)?;
    let mode = rounding(u32::from(context.state.x87.control_word >> 10));
    let (first, second) = if reverse {
        (right, &left)
    } else {
        (&left, right)
    };
    let mut result = arithmetic(operation, first, second, format, mode, false)?;
    let mut wrapped = false;
    if context.state.x87.control_word as u32 & (FLAG_OVERFLOW | FLAG_UNDERFLOW)
        != FLAG_OVERFLOW | FLAG_UNDERFLOW
    {
        let unlimited = arithmetic(
            operation,
            first,
            second,
            BinaryFormat {
                minimum_exponent: -100_000,
                maximum_exponent: 100_000,
                ..format
            },
            mode,
            false,
        )?;
        if let BinaryValue::Finite {
            coefficient,
            exponent,
            ..
        } = &unlimited.value
        {
            if !coefficient.is_zero() {
                let exponent = coefficient.bit_len() as i32 - 1 + exponent;
                let exception = if exponent > format.maximum_exponent {
                    FLAG_OVERFLOW
                } else if exponent < format.minimum_exponent {
                    FLAG_UNDERFLOW
                } else {
                    0
                };
                if exception & !(context.state.x87.control_word as u32) != 0 {
                    let adjusted = match &unlimited.value {
                        BinaryValue::Finite {
                            negative,
                            coefficient,
                            exponent,
                            denormal,
                        } => BinaryValue::Finite {
                            negative: *negative,
                            coefficient: coefficient.clone(),
                            exponent: exponent
                                + if exception == FLAG_OVERFLOW {
                                    -24_576
                                } else {
                                    24_576
                                },
                            denormal: *denormal,
                        },
                        _ => unlimited.value.clone(),
                    };
                    result = BinaryResult {
                        value: adjusted,
                        flags: unlimited.flags | exception,
                        rounded_up: unlimited.rounded_up,
                    };
                    wrapped = true;
                }
            }
        }
    }
    commit(&mut context.state.x87, destination, &result, wrapped)?;
    if pop {
        pop_x87(&mut context.state.x87);
    }
    Ok(())
}

fn environment_bytes(state: &GuestX87State, short: bool) -> Vec<u8> {
    let mut bytes = vec![0u8; if short { 14 } else { 28 }];
    let set16 = |bytes: &mut Vec<u8>, offset: usize, value: u16| {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    };
    let set32 = |bytes: &mut Vec<u8>, offset: usize, value: u32| {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    };
    set16(&mut bytes, 0, state.control_word);
    set16(&mut bytes, if short { 2 } else { 4 }, state.status_word);
    set16(&mut bytes, if short { 4 } else { 8 }, state.tag_word);
    if short {
        set16(&mut bytes, 6, state.instruction_pointer as u16);
        set16(&mut bytes, 8, state.instruction_selector);
        set16(&mut bytes, 10, state.data_pointer as u16);
        set16(&mut bytes, 12, state.data_selector);
    } else {
        set32(&mut bytes, 12, state.instruction_pointer as u32);
        set16(&mut bytes, 16, state.instruction_selector);
        set16(&mut bytes, 18, state.last_opcode);
        set32(&mut bytes, 20, state.data_pointer as u32);
        set16(&mut bytes, 24, state.data_selector);
    }
    bytes
}

fn restore_environment(
    state: &mut GuestX87State,
    bytes: &[u8],
    short: bool,
) -> Result<(), NumericError> {
    let get16 = |offset: usize| -> Result<u16, NumericError> {
        bytes
            .get(offset..offset + 2)
            .and_then(|chunk| chunk.try_into().ok())
            .map(u16::from_le_bytes)
            .ok_or_else(|| NumericError::unsupported("Truncated x87 environment image"))
    };
    let get32 = |offset: usize| -> Result<u32, NumericError> {
        bytes
            .get(offset..offset + 4)
            .and_then(|chunk| chunk.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| NumericError::unsupported("Truncated x87 environment image"))
    };
    state.control_word = get16(0)?;
    state.status_word = get16(if short { 2 } else { 4 })?;
    state.tag_word = get16(if short { 4 } else { 8 })?;
    if short {
        state.instruction_pointer = u64::from(get16(6)?);
    } else {
        state.instruction_pointer = u64::from(get32(12)?);
    }
    state.instruction_selector = get16(if short { 8 } else { 16 })?;
    if short {
        state.data_pointer = u64::from(get16(10)?);
    } else {
        state.data_pointer = u64::from(get32(20)?);
    }
    state.data_selector = get16(if short { 12 } else { 24 })?;
    state.last_opcode = if short { 0 } else { get16(18)? & 0x7ff };
    sync_exception_summary(state);
    Ok(())
}

fn sync_exception_summary(state: &mut GuestX87State) {
    if state.status_word as u32 & !(state.control_word as u32) & 63 != 0 {
        state.status_word |= 0x8080;
    } else {
        state.status_word &= !0x8080;
    }
}

fn execute(context: &mut NumericExecutionContext) -> Result<(), X87Error> {
    let instruction = context.instruction.clone();
    let opcode = instruction.opcode;
    let modrm = instruction.modrm;
    let group = modrm.map_or(0, |modrm| (modrm >> 3) & 7);
    let no_wait = (opcode == 0xdb && matches!(modrm, Some(0xe2 | 0xe3)))
        || (opcode == 0xdf && modrm == Some(0xe0))
        || ((opcode == 0xd9 || opcode == 0xdd)
            && matches!(instruction.operand, Some(NumericOperand::Memory(_)))
            && (group == 6 || group == 7));
    if !no_wait
        && context.state.x87.status_word as u32 & !(context.state.x87.control_word as u32) & 63
            != 0
    {
        return Err(NumericError::fault(16, "Pending x87 floating-point exception").into());
    }
    if opcode == 0x9b {
        return Ok(());
    }
    let (Some(modrm), Some(operand)) = (modrm, instruction.operand) else {
        return Err(NumericError::unsupported("x87 instruction requires ModRM and an operand").into());
    };
    if opcode == 0xdb && modrm == 0xe2 {
        context.state.x87.status_word &= 0x7f00;
        return Ok(());
    }
    if opcode == 0xdb && modrm == 0xe3 {
        initialize_x87(&mut context.state.x87);
        return Ok(());
    }
    if opcode == 0xdf && modrm == 0xe0 {
        let status = context.state.x87.status_word;
        context
            .state
            .registers
            .write(
                crate::core::contracts::GuestRegister::Rax,
                crate::core::contracts::GuestIntegerWidth::B16,
                u64::from(status),
                false,
            )
            .map_err(NumericError::Guest)?;
        return Ok(());
    }
    let control_memory = matches!(operand, NumericOperand::Memory(_))
        && (opcode == 0xd9 || opcode == 0xdd)
        && group >= 4;
    if !control_memory {
        let ip = context.state.instruction_pointer;
        let cs = context.state.segments[crate::core::registers::GuestProcessorState::CS].selector;
        let state = &mut context.state.x87;
        state.last_opcode = (u16::from(opcode & 7) << 8) | u16::from(modrm);
        state.instruction_pointer = ip;
        state.instruction_selector = cs;
    }
    if let NumericOperand::Memory(address) = operand {
        return execute_memory(context, address, group);
    }
    execute_register(context, modrm, group)
}

fn execute_memory(
    context: &mut NumericExecutionContext,
    address: crate::core::contracts::GuestAddress,
    group: u8,
) -> Result<(), X87Error> {
    let instruction = context.instruction.clone();
    let opcode = instruction.opcode;
    let short = instruction.operand_bits == 16;
    if (opcode == 0xd9 || opcode == 0xdd) && (group == 4 || group == 6) {
        let size = if short { 14 } else { 28 };
        if group == 4 {
            let bytes = context
                .memory
                .copy(address, size + if opcode == 0xdd { 80 } else { 0 })?;
            let (env, regs) = bytes.split_at(size);
            restore_environment(&mut context.state.x87, env, short)?;
            if opcode == 0xdd {
                for index in 0..8 {
                    let chunk = regs[index * 10..index * 10 + 10].to_vec();
                    let slot = physical(&context.state.x87, index);
                    context.state.x87.registers[slot * 10..slot * 10 + 10]
                        .copy_from_slice(&chunk);
                }
            }
        } else {
            let mut bytes = vec![0u8; size + if opcode == 0xdd { 80 } else { 0 }];
            bytes[..size].copy_from_slice(&environment_bytes(&context.state.x87, short));
            if opcode == 0xdd {
                for index in 0..8 {
                    let slot = physical(&context.state.x87, index);
                    let chunk = context.state.x87.registers[slot * 10..slot * 10 + 10].to_vec();
                    bytes[size + index * 10..size + index * 10 + 10].copy_from_slice(&chunk);
                }
            }
            context.memory.write(address, &bytes)?;
            if opcode == 0xdd {
                initialize_x87(&mut context.state.x87);
            } else {
                context.state.x87.control_word |= 63;
            }
        }
        return Ok(());
    }
    if opcode == 0xd9 && group == 5 {
        let bytes = context.memory.copy(address, 2)?;
        context.state.x87.control_word = read_bits(&bytes).low_u64() as u16;
        sync_exception_summary(&mut context.state.x87);
        return Ok(());
    }
    if (opcode == 0xd9 || opcode == 0xdd) && group == 7 {
        let value = if opcode == 0xd9 {
            context.state.x87.control_word
        } else {
            context.state.x87.status_word
        };
        context.memory.write(
            address,
            &write_bits(&BigInt::from_u64(u64::from(value)), 2),
        )?;
        return Ok(());
    }
    let ds = context.state.segments[crate::core::registers::GuestProcessorState::DS].selector;
    context.state.x87.data_pointer = address.offset;
    context.state.x87.data_selector = ds;
    if matches!(opcode, 0xd8 | 0xdc | 0xda | 0xde) {
        let right = if opcode == 0xda || opcode == 0xde {
            let width = if opcode == 0xda { 4 } else { 2 };
            let bytes = context.memory.copy(address, width)?;
            from_integer(&read_bits(&bytes).as_int_n(if opcode == 0xda { 32 } else { 16 }))
        } else {
            let width = if opcode == 0xd8 {
                BinaryWidth::W32
            } else {
                BinaryWidth::W64
            };
            let bytes = context.memory.copy(address, width.bits() as usize / 8)?;
            decode_binary(&read_bits(&bytes), width)
        };
        binary_instruction(context, group, &right, 0, false, false)?;
        return Ok(());
    }
    let real_width = if opcode == 0xd9 {
        32
    } else if opcode == 0xdd {
        64
    } else {
        80
    };
    let integer_width = if opcode == 0xdb {
        32
    } else if opcode == 0xdf && (group == 5 || group == 7) {
        64
    } else {
        16
    };
    let real = opcode == 0xd9 || opcode == 0xdd || (opcode == 0xdb && (group == 5 || group == 7));
    let load = group == 0 || (group == 5 && (opcode == 0xdb || opcode == 0xdf));
    let store = group == 2
        || group == 3
        || group == 7
        || (group == 1 && matches!(opcode, 0xdb | 0xdd | 0xdf));
    if !load && !store {
        return Err(NumericError::unsupported(format!(
            "Unsupported x87 memory opcode {opcode:x}/{group}"
        ))
        .into());
    }
    if load {
        if real_width == 80 && real {
            let bytes = context.memory.copy(address, 10)?;
            push_raw(&mut context.state.x87, &read_bits(&bytes))?;
            return Ok(());
        }
        let value = if real {
            let width = match real_width {
                32 => BinaryWidth::W32,
                64 => BinaryWidth::W64,
                _ => BinaryWidth::W80,
            };
            let bytes = context.memory.copy(address, real_width / 8)?;
            decode_binary(&read_bits(&bytes), width)
        } else {
            let bytes = context.memory.copy(address, integer_width / 8)?;
            from_integer(&read_bits(&bytes).as_int_n(integer_width as u32))
        };
        let mode = rounding(u32::from(context.state.x87.control_word >> 10));
        let result = convert_binary(&value, BINARY80, mode)?;
        let denormal = matches!(value, BinaryValue::Finite { denormal: true, .. });
        raise(
            &mut context.state.x87,
            result.flags | if denormal { FLAG_DENORMAL_OPERAND } else { 0 },
            false,
            false,
        )?;
        push_value(&mut context.state.x87, &result.value)?;
    } else {
        let value = read_register(&mut context.state.x87, 0)?;
        if real && group != 1 {
            let mode = rounding(u32::from(context.state.x87.control_word >> 10));
            let result = if real_width == 80 {
                BinaryResult {
                    value: value.clone(),
                    flags: 0,
                    rounded_up: false,
                }
            } else {
                convert_binary(
                    &value,
                    format_for(match real_width {
                        32 => BinaryWidth::W32,
                        _ => BinaryWidth::W64,
                    }),
                    mode,
                )?
            };
            let tiny = matches!(result.value, BinaryValue::Finite { denormal: true, .. });
            let range_exception = (result.flags & FLAG_OVERFLOW)
                | if tiny || result.flags & FLAG_UNDERFLOW != 0 {
                    FLAG_UNDERFLOW
                } else {
                    0
                };
            if range_exception & !(context.state.x87.control_word as u32) != 0 {
                raise(&mut context.state.x87, range_exception, false, false)?;
            }
            raise(&mut context.state.x87, result.flags, result.rounded_up, false)?;
            let bits = if real_width == 80 {
                raw_register(&mut context.state.x87, 0)?
            } else {
                encode_binary(
                    &result.value,
                    match real_width {
                        32 => BinaryWidth::W32,
                        _ => BinaryWidth::W64,
                    },
                )
            };
            context
                .memory
                .write(address, &write_bits(&bits, real_width / 8))?;
        } else {
            let width = if opcode == 0xdd && group == 1 {
                64
            } else {
                integer_width
            };
            let mode = if group == 1 {
                Rounding::Zero
            } else {
                rounding(u32::from(context.state.x87.control_word >> 10))
            };
            let result = integer_conversion(&value, width as u32, mode)?;
            raise(&mut context.state.x87, result.flags, result.rounded_up, false)?;
            context.memory.write(
                address,
                &write_bits(&result.value.as_uint_n(width as u32), width / 8),
            )?;
        }
        if group == 1 || group == 3 || group == 7 {
            pop_x87(&mut context.state.x87);
        }
    }
    Ok(())
}

fn execute_register(
    context: &mut NumericExecutionContext,
    modrm: u8,
    group: u8,
) -> Result<(), X87Error> {
    let opcode = context.instruction.opcode;
    let index = (modrm & 7) as usize;
    if opcode == 0xd8 {
        let right = read_register(&mut context.state.x87, index)?;
        binary_instruction(context, group, &right, 0, false, false)?;
        return Ok(());
    }
    if opcode == 0xdc || opcode == 0xde {
        if opcode == 0xde && modrm == 0xd9 {
            let right = read_register(&mut context.state.x87, 1)?;
            compare(context, &right, false, false)?;
            pop_x87(&mut context.state.x87);
            pop_x87(&mut context.state.x87);
            return Ok(());
        }
        if group == 2 || group == 3 {
            return Err(
                NumericError::unsupported("Reserved x87 register comparison encoding").into(),
            );
        }
        let right = read_register(&mut context.state.x87, 0)?;
        binary_instruction(context, group, &right, index, true, opcode == 0xde)?;
        return Ok(());
    }
    if opcode == 0xd9 {
        return execute_d9_special(context, modrm, group, index);
    }
    if opcode == 0xdd {
        if group == 0 {
            let slot = physical(&context.state.x87, index);
            set_tag(&mut context.state.x87, slot, 3);
            return Ok(());
        }
        if group == 2 || group == 3 {
            let bits = raw_register(&mut context.state.x87, 0)?;
            write_raw_register(&mut context.state.x87, index, &bits);
            if group == 3 {
                pop_x87(&mut context.state.x87);
            }
            return Ok(());
        }
        if group == 4 || group == 5 {
            let right = read_register(&mut context.state.x87, index)?;
            compare(context, &right, true, false)?;
            if group == 5 {
                pop_x87(&mut context.state.x87);
            }
            return Ok(());
        }
    }
    if opcode == 0xda && modrm == 0xe9 {
        let right = read_register(&mut context.state.x87, 1)?;
        compare(context, &right, true, false)?;
        pop_x87(&mut context.state.x87);
        pop_x87(&mut context.state.x87);
        return Ok(());
    }
    if (opcode == 0xdb || opcode == 0xdf) && (group == 5 || group == 6) {
        let right = read_register(&mut context.state.x87, index)?;
        compare(context, &right, group == 5, true)?;
        if opcode == 0xdf {
            pop_x87(&mut context.state.x87);
        }
        return Ok(());
    }
    if (opcode == 0xda || opcode == 0xdb) && group <= 3 {
        let flags = &context.state.flags;
        let condition = match group {
            0 => flags.get(GuestFlag::Carry),
            1 => flags.get(GuestFlag::Zero),
            2 => flags.get(GuestFlag::Carry) || flags.get(GuestFlag::Zero),
            _ => flags.get(GuestFlag::Parity),
        };
        let source = read_register(&mut context.state.x87, index)?;
        read_register(&mut context.state.x87, 0)?;
        if condition == (opcode == 0xda) {
            write_x87_register(&mut context.state.x87, 0, &source);
        }
        return Ok(());
    }
    Err(NumericError::unsupported(format!("Unsupported x87 opcode {opcode:x} {modrm:x}")).into())
}

fn execute_d9_special(
    context: &mut NumericExecutionContext,
    modrm: u8,
    group: u8,
    index: usize,
) -> Result<(), X87Error> {
    if group == 0 {
        let bits = raw_register(&mut context.state.x87, index)?;
        push_raw(&mut context.state.x87, &bits)?;
        return Ok(());
    }
    if group == 1 {
        let a = raw_register(&mut context.state.x87, 0)?;
        let b = raw_register(&mut context.state.x87, index)?;
        write_raw_register(&mut context.state.x87, 0, &b);
        write_raw_register(&mut context.state.x87, index, &a);
        return Ok(());
    }
    match modrm {
        0xd0 => Ok(()),
        0xe0 | 0xe1 => {
            let bits = raw_register(&mut context.state.x87, 0)?;
            let toggled = if modrm == 0xe0 {
                if bits.bit(79) {
                    bits.sub(&BigInt::one().shl_bits(79))
                } else {
                    bits.add(&BigInt::one().shl_bits(79))
                }
            } else {
                bits.as_uint_n(79)
            };
            write_raw_register(&mut context.state.x87, 0, &toggled);
            Ok(())
        }
        0xe4 => {
            compare(context, &zero(false), false, false)?;
            Ok(())
        }
        0xe5 => {
            let empty = tag(&context.state.x87, top(&context.state.x87)) == 3;
            let value = if empty {
                zero(false)
            } else {
                read_register(&mut context.state.x87, 0)?
            };
            let bits = if empty {
                0x4100
            } else {
                match &value {
                    BinaryValue::Unsupported { .. } => 0,
                    BinaryValue::Nan { .. } => 0x100,
                    BinaryValue::Infinity { .. } => 0x500,
                    BinaryValue::Finite {
                        coefficient,
                        denormal,
                        ..
                    } => {
                        if coefficient.is_zero() {
                            0x4000
                        } else if *denormal {
                            0x4400
                        } else {
                            0x400
                        }
                    }
                }
            };
            let state = &mut context.state.x87;
            state.status_word =
                (state.status_word & !0x4700) | bits | (u16::from(value.negative()) << 9);
            Ok(())
        }
        0xeb => {
            let constant = BinaryValue::Finite {
                negative: false,
                coefficient: BigInt::from_bytes_le(&[
                    0x4c, 0x23, 0x8c, 0x16, 0x22, 0xaa, 0xfd, 0x90, 0x0c,
                ]),
                exponent: -66,
                denormal: false,
            };
            let mode = rounding(u32::from(context.state.x87.control_word >> 10));
            let converted = convert_binary(&constant, BINARY80, mode)?;
            push_value(&mut context.state.x87, &converted.value)?;
            Ok(())
        }
        0xe8 | 0xee => {
            let value = if modrm == 0xe8 {
                from_integer(&BigInt::one())
            } else {
                zero(false)
            };
            push_value(&mut context.state.x87, &value)?;
            Ok(())
        }
        0xf6 | 0xf7 => {
            let current = top(&context.state.x87);
            set_top(
                &mut context.state.x87,
                current.wrapping_add(if modrm == 0xf6 { 7 } else { 1 }) & 7,
            );
            context.state.x87.status_word &= !0x200;
            Ok(())
        }
        0xf3 => {
            let first = read_register(&mut context.state.x87, 1)?;
            let second = read_register(&mut context.state.x87, 0)?;
            let mode = rounding(u32::from(context.state.x87.control_word >> 10));
            let result = x87_arctangent(&first, &second, mode)?;
            commit(&mut context.state.x87, 1, &result, false)?;
            pop_x87(&mut context.state.x87);
            Ok(())
        }
        0xfa => {
            let value = read_register(&mut context.state.x87, 0)?;
            let format = result_format(&context.state.x87)?;
            let mode = rounding(u32::from(context.state.x87.control_word >> 10));
            let result = square_root(&value, format, mode)?;
            commit(&mut context.state.x87, 0, &result, false)?;
            Ok(())
        }
        0xfe | 0xff => {
            let value = read_register(&mut context.state.x87, 0)?;
            let mode = rounding(u32::from(context.state.x87.control_word >> 10));
            let result = x87_trigonometric(&value, modrm == 0xff, mode)?;
            match result {
                X87TrigonometricResult::OutOfRange => {
                    context.state.x87.status_word |= 0x400;
                }
                X87TrigonometricResult::Result(result) => {
                    context.state.x87.status_word &= !0x400;
                    commit(&mut context.state.x87, 0, &result, false)?;
                }
            }
            Ok(())
        }
        0xfc => {
            let value = read_register(&mut context.state.x87, 0)?;
            let mode = rounding(u32::from(context.state.x87.control_word >> 10));
            let result = round_integral(&value, mode)?;
            commit(&mut context.state.x87, 0, &result, false)?;
            Ok(())
        }
        _ => Err(NumericError::unsupported(format!(
            "Unsupported x87 special opcode d9 {modrm:x}"
        ))
        .into()),
    }
}

/// Execute one x87 instruction (0x9b wait plus 0xd8..0xdf).
pub fn execute_x87(context: NumericExecutionContext) -> Result<(), NumericError> {
    let mut context = context;
    match execute(&mut context) {
        Ok(()) => Ok(()),
        Err(X87Error::Deferred) => Ok(()),
        Err(X87Error::Numeric(error)) => Err(error),
    }
}
