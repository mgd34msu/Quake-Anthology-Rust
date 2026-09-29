//! User-mode i386 interpreter.
//!
//! Donor: `src/guest/x86/cpu.ts` (`I386Cpu`). All architectural values live
//! in the supplied core state. Each REP iteration consumes one budget unit
//! and leaves restartable IP/count/index state; failed instructions restore
//! the register, flag, and IP snapshot.

use std::rc::Rc;

use crate::abi::GuestCpu;
use crate::core::callbacks::HookState;
use crate::core::contracts::{
    GuestAccess, GuestAddress, GuestException, GuestExecutionStop, GuestFlag, GuestInstruction, GuestRegister,
};
use crate::core::memory::SparseGuestMemory;
use crate::core::registers::GuestProcessorState;
use crate::error::GuestError;
use crate::floating_point::contracts::{
    NumericExecutionContext, NumericExecutionResult, NumericInstruction, NumericOperand, NumericPrefix,
};
use crate::floating_point::execute_numeric_instruction;
use crate::x86::arithmetic::{
    alu, condition, quotient_fits_signed, result_flags, shift, sign_extend, sign_extend_double, signed_multiply,
};
use crate::x86::arithmetic::{AluOperation, ShiftOperation};
use crate::x86::decoder::{
    guest_address, register_operand, RepeatPrefix, SegmentName, X86Decoder, X86Error, X86Operand, X86Width,
};

/// Numeric executor hook (defaults to the exact x87/SSE executor).
pub type NumericExecutor = fn(NumericExecutionContext) -> Result<NumericExecutionResult, crate::error::GuestError>;

fn arithmetic_operation(index: usize) -> Result<AluOperation, X86Error> {
    match index {
        0 => Ok(AluOperation::Add),
        1 => Ok(AluOperation::Or),
        2 => Ok(AluOperation::Adc),
        3 => Ok(AluOperation::Sbb),
        4 => Ok(AluOperation::And),
        5 => Ok(AluOperation::Sub),
        6 => Ok(AluOperation::Xor),
        7 => Ok(AluOperation::Cmp),
        _ => Err(X86Error::unsupported(format!("Invalid ALU encoding {index}"))),
    }
}

fn shift_operation(index: usize) -> Result<ShiftOperation, X86Error> {
    match index {
        0 => Ok(ShiftOperation::Rol),
        1 => Ok(ShiftOperation::Ror),
        2 => Ok(ShiftOperation::Rcl),
        3 => Ok(ShiftOperation::Rcr),
        4 | 6 => Ok(ShiftOperation::Shl),
        5 => Ok(ShiftOperation::Shr),
        7 => Ok(ShiftOperation::Sar),
        _ => Err(X86Error::unsupported(format!("Invalid shift encoding {index}"))),
    }
}

fn operand_width(bits: u32) -> X86Width {
    match bits {
        16 => X86Width::W16,
        _ => X86Width::W32,
    }
}

fn stack_operand(offset: u64) -> X86Operand {
    X86Operand::Memory {
        offset: offset as u32 as u64,
        segment: SegmentName::Ss,
        stack_pointer_base: true,
    }
}

fn push(decoder: &mut X86Decoder, width: X86Width, value: u64) -> Result<(), X86Error> {
    let pointer = decoder
        .state
        .registers
        .read(GuestRegister::Rsp, X86Width::W32.register_width(), false)?
        .wrapping_sub(width.bytes() as u64) as u32 as u64;
    decoder.write(stack_operand(pointer), width, value)?;
    decoder
        .state
        .registers
        .write(GuestRegister::Rsp, X86Width::W32.register_width(), pointer, false)?;
    Ok(())
}

fn pop(decoder: &mut X86Decoder, width: X86Width) -> Result<u64, X86Error> {
    let pointer = decoder
        .state
        .registers
        .read(GuestRegister::Rsp, X86Width::W32.register_width(), false)?;
    let value = decoder.read(stack_operand(pointer), width)?;
    decoder.state.registers.write(
        GuestRegister::Rsp,
        X86Width::W32.register_width(),
        pointer.wrapping_add(width.bytes() as u64),
        false,
    )?;
    Ok(value)
}

fn lock(decoder: &X86Decoder, operand: X86Operand, allowed: bool) -> Result<(), X86Error> {
    if decoder.lock && (!allowed || !matches!(operand, X86Operand::Memory { .. })) {
        return Err(X86Error::fault(
            6,
            "LOCK requires a supported read-modify-write memory destination",
        ));
    }
    Ok(())
}

fn execute_alu(
    decoder: &mut X86Decoder,
    operation: AluOperation,
    width: X86Width,
    destination: X86Operand,
    right: u64,
) -> Result<(), X86Error> {
    let write = !matches!(operation, AluOperation::Cmp | AluOperation::Test);
    lock(decoder, destination, write)?;
    let left = decoder.read(destination, width)?;
    if write {
        decoder.check_write(destination, width)?;
    }
    let value = alu(operation, width.bits(), left, right, &mut decoder.state.flags);
    if write {
        decoder.write(destination, width, value)?;
    }
    Ok(())
}

fn incdec(decoder: &mut X86Decoder, operand: X86Operand, width: X86Width, decrement: bool) -> Result<(), X86Error> {
    let carry = decoder.state.flags.get(GuestFlag::Carry);
    execute_alu(
        decoder,
        if decrement {
            AluOperation::Sub
        } else {
            AluOperation::Add
        },
        width,
        operand,
        1,
    )?;
    decoder.state.flags.set(GuestFlag::Carry, carry);
    Ok(())
}

fn segment(index: usize) -> Result<SegmentName, X86Error> {
    match index {
        0 => Ok(SegmentName::Es),
        1 => Ok(SegmentName::Cs),
        2 => Ok(SegmentName::Ss),
        3 => Ok(SegmentName::Ds),
        4 => Ok(SegmentName::Fs),
        5 => Ok(SegmentName::Gs),
        _ => Err(X86Error::fault(6, "Invalid segment register encoding")),
    }
}

fn set_selector(decoder: &mut X86Decoder, segment: SegmentName, selector: u16) -> Result<(), X86Error> {
    if segment == SegmentName::Cs {
        return Err(X86Error::fault(6, "MOV cannot load CS"));
    }
    if selector != decoder.state.segments[segment.index()].selector {
        return Err(X86Error::unsupported(format!(
            "Segment descriptor resolution for {}=0x{selector:x} is unavailable",
            segment.label()
        )));
    }
    Ok(())
}

fn pop_segment(decoder: &mut X86Decoder, segment: SegmentName) -> Result<(), X86Error> {
    let width = operand_width(decoder.operand_bits);
    let selector = (pop(decoder, width)? & 0xffff) as u16;
    set_selector(decoder, segment, selector)
}

fn enter(decoder: &mut X86Decoder) -> Result<(), X86Error> {
    let allocation = decoder.immediate(X86Width::W16)?;
    let nesting = decoder.byte()? & 31;
    let width = operand_width(decoder.operand_bits);
    let prior_frame = decoder.register_value(5, width)?;
    let mut values = vec![prior_frame];
    let rsp = decoder.register_value(4, X86Width::W32)?;
    let frame = rsp.wrapping_sub(width.bytes() as u64) as u32 as u64;
    for level in 1..nesting {
        values.push(decoder.read(
            stack_operand(prior_frame.wrapping_sub(u64::from(level) * width.bytes() as u64)),
            width,
        )?);
    }
    if nesting != 0 {
        values.push(frame);
    }
    let rsp = decoder.register_value(4, X86Width::W32)?;
    let final_stack = rsp.wrapping_sub(values.len() as u64 * width.bytes() as u64) as u32 as u64;
    let operand = stack_operand(final_stack);
    let address = decoder.address(operand, values.len() * width.bytes(), GuestAccess::Write)?;
    decoder
        .memory
        .check(address, values.len() * width.bytes(), GuestAccess::Write)?;
    for value in values {
        push(decoder, width, value)?;
    }
    decoder
        .state
        .registers
        .write(GuestRegister::Rbp, width.register_width(), frame, false)?;
    decoder.state.registers.write(
        GuestRegister::Rsp,
        X86Width::W32.register_width(),
        final_stack.wrapping_sub(allocation),
        false,
    )?;
    Ok(())
}

fn address_width_bits(decoder: &X86Decoder) -> X86Width {
    operand_width(decoder.address_bits)
}

fn string_op(decoder: &mut X86Decoder) -> Result<(), X86Error> {
    let op = decoder.opcode;
    let width = if op & 1 == 0 {
        X86Width::W8
    } else {
        operand_width(decoder.operand_bits)
    };
    let repeated = decoder.repeat != RepeatPrefix::None;
    let addr_width = address_width_bits(decoder);
    let count = decoder.register_value(1, addr_width)?;
    if repeated && count == 0 {
        return Ok(());
    }
    let source = X86Operand::Memory {
        offset: decoder.register_value(6, addr_width)?,
        segment: decoder.segment.unwrap_or(SegmentName::Ds),
        stack_pointer_base: false,
    };
    let destination = X86Operand::Memory {
        offset: decoder.register_value(7, addr_width)?,
        segment: SegmentName::Es,
        stack_pointer_base: false,
    };
    let step = width.bytes() as i64
        * if decoder.state.flags.get(GuestFlag::Direction) {
            -1
        } else {
            1
        };
    let category = op & 0xfe;
    match category {
        0xa4 => {
            let value = decoder.read(source, width)?;
            decoder.write(destination, width, value)?;
        }
        0xa6 => {
            let left = decoder.read(source, width)?;
            let right = decoder.read(destination, width)?;
            alu(AluOperation::Cmp, width.bits(), left, right, &mut decoder.state.flags);
        }
        0xaa => {
            let value = decoder.read(register_operand(0, width)?, width)?;
            decoder.write(destination, width, value)?;
        }
        0xac => {
            let value = decoder.read(source, width)?;
            decoder.write(register_operand(0, width)?, width, value)?;
        }
        _ => {
            let left = decoder.read(register_operand(0, width)?, width)?;
            let right = decoder.read(destination, width)?;
            alu(AluOperation::Cmp, width.bits(), left, right, &mut decoder.state.flags);
        }
    }
    let wrap = if decoder.address_bits == 16 {
        0xffffu64
    } else {
        0xffff_ffff
    };
    if matches!(category, 0xa4 | 0xa6 | 0xac) {
        let X86Operand::Memory { offset, .. } = source else {
            unreachable!("source is memory");
        };
        decoder.state.registers.write(
            GuestRegister::Rsi,
            addr_width.register_width(),
            offset.wrapping_add(step as u64) & wrap,
            false,
        )?;
    }
    if category != 0xac {
        let X86Operand::Memory { offset, .. } = destination else {
            unreachable!("destination is memory");
        };
        decoder.state.registers.write(
            GuestRegister::Rdi,
            addr_width.register_width(),
            offset.wrapping_add(step as u64) & wrap,
            false,
        )?;
    }
    if repeated {
        decoder.state.registers.write(
            GuestRegister::Rcx,
            addr_width.register_width(),
            count.wrapping_sub(1) & wrap,
            false,
        )?;
        let compare = category == 0xa6 || category == 0xae;
        if count != 1 && (!compare || decoder.state.flags.get(GuestFlag::Zero) == (decoder.repeat == RepeatPrefix::F3))
        {
            decoder.cursor = decoder.start;
        }
    }
    Ok(())
}

fn unary(decoder: &mut X86Decoder, width: X86Width) -> Result<(), X86Error> {
    let decoded = decoder.modrm(width)?;
    lock(decoder, decoded.operand, decoded.group == 2 || decoded.group == 3)?;
    let operand = decoder.read(decoded.operand, width)?;
    if decoded.group == 0 || decoded.group == 1 {
        let immediate = decoder.immediate(width)?;
        return execute_alu(decoder, AluOperation::Test, width, decoded.operand, immediate);
    }
    if decoded.group == 2 {
        return decoder.write(decoded.operand, width, !operand & mask_for(width));
    }
    if decoded.group == 3 {
        decoder.check_write(decoded.operand, width)?;
        let value = alu(AluOperation::Sub, width.bits(), 0, operand, &mut decoder.state.flags);
        return decoder.write(decoded.operand, width, value);
    }
    let low = decoder.read(register_operand(0, width)?, width)?;
    if decoded.group == 4 || decoded.group == 5 {
        let signed = decoded.group == 5;
        let extend = |value: u64| -> i128 { sign_extend(value, width.bits()) };
        let product = if signed {
            extend(low) * extend(operand)
        } else {
            (low & mask_for(width)) as i128 * (operand & mask_for(width)) as i128
        };
        let overflow = if signed {
            extend(product as u64) != product
        } else {
            (product as u64 >> width.bits()) != 0
        };
        decoder.state.flags.set(GuestFlag::Carry, overflow);
        decoder.state.flags.set(GuestFlag::Overflow, overflow);
        if width == X86Width::W8 {
            decoder.state.registers.write(
                GuestRegister::Rax,
                X86Width::W16.register_width(),
                product as u64,
                false,
            )?;
        } else {
            decoder.write(register_operand(0, width)?, width, product as u64)?;
            decoder.write(
                register_operand(2, width)?,
                width,
                (product as u64).wrapping_shr(width.bits()),
            )?;
        }
        return Ok(());
    }
    let raw_dividend = if width == X86Width::W8 {
        decoder.read(register_operand(0, X86Width::W16)?, X86Width::W16)?
    } else {
        (decoder.read(register_operand(2, width)?, width)? << width.bits()) | low
    };
    let signed = decoded.group == 7;
    let divisor = if signed {
        sign_extend(operand, width.bits())
    } else {
        (operand & mask_for(width)) as i128
    };
    let double_bits = width.bits() * 2;
    let dividend = if signed {
        sign_extend_double(u128::from(raw_dividend), double_bits)
    } else {
        raw_dividend as i128
    };
    if divisor == 0 {
        return Err(X86Error::fault(0, "Integer division by zero"));
    }
    let quotient = dividend / divisor;
    let remainder = dividend % divisor;
    let fits = if signed {
        quotient_fits_signed(quotient, width.bits())
    } else {
        quotient >= 0 && (quotient as u64 & !mask_for(width)) == 0
    };
    if !fits {
        return Err(X86Error::fault(0, "Integer division quotient overflow"));
    }
    decoder.write(register_operand(0, width)?, width, quotient as u64)?;
    if width == X86Width::W8 {
        decoder.write(
            X86Operand::Register {
                register: GuestRegister::Rax,
                high_byte: true,
                index: 4,
            },
            width,
            remainder as u64,
        )?;
    } else {
        decoder.write(register_operand(2, width)?, width, remainder as u64)?;
    }
    Ok(())
}

fn mask_for(width: X86Width) -> u64 {
    if width == X86Width::W32 {
        0xffff_ffff
    } else if width == X86Width::W16 {
        0xffff
    } else {
        0xff
    }
}

fn bit_op(decoder: &mut X86Decoder, op: u8) -> Result<(), X86Error> {
    let width = operand_width(decoder.operand_bits);
    let decoded = decoder.modrm(width)?;
    let operation = if op == 0xba {
        decoded.group
    } else if op == 0xa3 {
        4
    } else if op == 0xab {
        5
    } else if op == 0xb3 {
        6
    } else {
        7
    };
    if operation < 4 {
        return Err(X86Error::fault(6, "Invalid bit-operation group selector"));
    }
    lock(decoder, decoded.operand, operation != 4)?;
    let index = if op == 0xba {
        i64::from(decoder.byte()?)
    } else {
        let raw = decoder.read(decoded.register, width)?;
        (((raw & mask_for(width)) << (64 - width.bits())) as i64) >> (64 - width.bits())
    };
    let bit_bits = if width == X86Width::W16 { 4 } else { 5 };
    let bit = (index as u64) & ((1 << bit_bits) - 1);
    let mut operand = decoded.operand;
    if matches!(operand, X86Operand::Memory { .. }) && op != 0xba {
        let X86Operand::Memory {
            offset,
            segment,
            stack_pointer_base,
        } = operand
        else {
            unreachable!("memory checked above");
        };
        let delta = (index - bit as i64) / width.bits() as i64 * (width.bytes() as i64);
        let wrap = if decoder.address_bits == 16 {
            0xffff
        } else {
            0xffff_ffff
        };
        operand = X86Operand::Memory {
            offset: offset.wrapping_add(delta as u64) & wrap,
            segment,
            stack_pointer_base,
        };
    }
    let value = decoder.read(operand, width)?;
    let mask = 1u64 << bit;
    if operation != 4 {
        decoder.check_write(operand, width)?;
    }
    decoder.state.flags.set(GuestFlag::Carry, value & mask != 0);
    if operation != 4 {
        decoder.write(
            operand,
            width,
            if operation == 5 {
                value | mask
            } else if operation == 6 {
                value & !mask
            } else {
                value ^ mask
            },
        )?;
    }
    Ok(())
}

fn double_shift(decoder: &mut X86Decoder, op: u8) -> Result<(), X86Error> {
    let width = operand_width(decoder.operand_bits);
    let decoded = decoder.modrm(width)?;
    let count = if op & 1 == 0 {
        u32::from(decoder.byte()?) & 31
    } else {
        decoder.read(register_operand(1, X86Width::W8)?, X86Width::W8)? as u32 & 31
    };
    if count > width.bits() {
        return Err(X86Error::unsupported(
            "SHLD/SHRD count exceeding operand width has undefined behavior",
        ));
    }
    let value = decoder.read(decoded.operand, width)?;
    let source = decoder.read(decoded.register, width)?;
    if count == 0 {
        return Ok(());
    }
    decoder.check_write(decoded.operand, width)?;
    let left = op < 0xac;
    let result = if left {
        ((value << count) | (source >> (width.bits() - count))) & mask_for(width)
    } else {
        ((value >> count) | (source << (width.bits() - count))) & mask_for(width)
    };
    let carry = ((if left {
        value >> (width.bits() - count)
    } else {
        value >> (count - 1)
    }) & 1)
        != 0;
    let old_overflow = decoder.state.flags.get(GuestFlag::Overflow);
    let old_auxiliary = decoder.state.flags.get(GuestFlag::AuxiliaryCarry);
    alu(AluOperation::Or, width.bits(), result, 0, &mut decoder.state.flags);
    decoder.state.flags.set(GuestFlag::Carry, carry);
    decoder.state.flags.set(GuestFlag::AuxiliaryCarry, old_auxiliary);
    decoder.state.flags.set(
        GuestFlag::Overflow,
        if count == 1 {
            (value ^ result) & (1u64 << (width.bits() - 1)) != 0
        } else {
            old_overflow
        },
    );
    decoder.write(decoded.operand, width, result)
}

fn compare_exchange8(decoder: &mut X86Decoder) -> Result<(), X86Error> {
    let decoded = decoder.modrm(X86Width::W32)?;
    if decoded.group != 1 || !matches!(decoded.operand, X86Operand::Memory { .. }) {
        return Err(X86Error::unsupported(
            "Only memory CMPXCHG8B is supported in 0F C7 group",
        ));
    }
    let address = decoder.address(decoded.operand, 8, GuestAccess::Read)?;
    let value = decoder.memory.read_u64(address)?;
    decoder.memory.check(address, 8, GuestAccess::Write)?;
    let expected = (decoder.read(register_operand(2, X86Width::W32)?, X86Width::W32)? << 32)
        | decoder.read(register_operand(0, X86Width::W32)?, X86Width::W32)?;
    let equal = value == expected;
    let replacement = if equal {
        (decoder.read(register_operand(1, X86Width::W32)?, X86Width::W32)? << 32)
            | decoder.read(register_operand(3, X86Width::W32)?, X86Width::W32)?
    } else {
        value
    };
    decoder.memory.write_u64(address, replacement)?;
    decoder.state.flags.set(GuestFlag::Zero, equal);
    if !equal {
        decoder.write(register_operand(0, X86Width::W32)?, X86Width::W32, value)?;
        decoder.write(register_operand(2, X86Width::W32)?, X86Width::W32, value >> 32)?;
    }
    Ok(())
}

fn floating(decoder: &mut X86Decoder, numeric: NumericExecutor, secondary_opcode: Option<u8>) -> Result<(), X86Error> {
    let decoded = if decoder.opcode == 0x9b || secondary_opcode == Some(0x77) {
        None
    } else {
        Some(decoder.modrm(operand_width(decoder.operand_bits))?)
    };
    let mut operand: Option<NumericOperand> = None;
    if let Some(decoded) = &decoded {
        operand = Some(match decoded.operand {
            X86Operand::Memory { .. } => {
                NumericOperand::Memory(decoder.address(decoded.operand, 1, GuestAccess::Read)?)
            }
            X86Operand::Register { index, .. } => NumericOperand::Register(index),
        });
    }
    let needs_immediate =
        secondary_opcode.is_some_and(|op| matches!(op, 0x70 | 0x71 | 0x72 | 0x73 | 0xc2 | 0xc4 | 0xc5 | 0xc6));
    let immediate = if needs_immediate { Some(decoder.byte()?) } else { None };
    let instruction = NumericInstruction {
        opcode: decoder.opcode,
        secondary_opcode,
        modrm: decoded.as_ref().map(|decoded| decoded.byte),
        operand,
        register_index: decoded.as_ref().map_or(0, |decoded| decoded.group),
        prefix: match decoder.repeat {
            RepeatPrefix::F2 => NumericPrefix::XF2,
            RepeatPrefix::F3 => NumericPrefix::XF3,
            RepeatPrefix::None => {
                if decoder.operand_bits == 16 {
                    NumericPrefix::X66
                } else {
                    NumericPrefix::None
                }
            }
        },
        operand_bits: decoder.operand_bits as u16,
        immediate,
    };
    let result = numeric(NumericExecutionContext {
        state: &mut *decoder.state,
        memory: &mut *decoder.memory,
        instruction,
    })?;
    match result {
        NumericExecutionResult::Executed => Ok(()),
        NumericExecutionResult::Unsupported { detail } => Err(X86Error::Unsupported(detail)),
        NumericExecutionResult::Exception { vector, detail } => Err(X86Error::fault(u32::from(vector), detail)),
    }
}

fn extended(decoder: &mut X86Decoder, numeric: NumericExecutor) -> Result<(), X86Error> {
    let op = decoder.byte()?;
    let width = operand_width(decoder.operand_bits);
    if decoder.lock && !matches!(op, 0xab | 0xb0 | 0xb1 | 0xb3 | 0xba | 0xbb | 0xc0 | 0xc1 | 0xc7) {
        return Err(X86Error::fault(6, "LOCK is invalid for this 0F opcode"));
    }
    if (0x80..=0x8f).contains(&op) {
        let relative = decoder.signed(width)?;
        let take = condition(op & 15, &decoder.state.flags)?;
        if take {
            decoder.cursor = (decoder.cursor as i64).wrapping_add(relative) as u64 & mask_for(width);
        }
        return Ok(());
    }
    if (0x90..=0x9f).contains(&op) {
        let decoded = decoder.modrm(X86Width::W8)?;
        let take = condition(op & 15, &decoder.state.flags)?;
        return decoder.write(decoded.operand, X86Width::W8, u64::from(take));
    }
    if (0x40..=0x4f).contains(&op) {
        let decoded = decoder.modrm(width)?;
        let value = decoder.read(decoded.operand, width)?;
        let take = condition(op & 15, &decoder.state.flags)?;
        if take {
            decoder.write(decoded.register, width, value)?;
        }
        return Ok(());
    }
    if (0xc8..=0xcf).contains(&op) {
        if width != X86Width::W32 {
            return Err(X86Error::unsupported(
                "16-bit BSWAP has undefined architectural behavior",
            ));
        }
        let operand = register_operand((op & 7) as usize, X86Width::W32)?;
        let value = decoder.read(operand, X86Width::W32)?;
        return decoder.write(
            operand,
            X86Width::W32,
            ((value & 0xff) << 24) | ((value & 0xff00) << 8) | ((value >> 8) & 0xff00) | ((value >> 24) & 0xff),
        );
    }
    match op {
        0x0b => Err(X86Error::fault(6, "UD2 invalid opcode")),
        0x1f => {
            let decoded = decoder.modrm(width)?;
            if decoded.group != 0 {
                return Err(X86Error::fault(6, "Invalid multi-byte NOP selector"));
            }
            Ok(())
        }
        0xa0 => push(
            decoder,
            width,
            u64::from(decoder.state.segments[SegmentName::Fs.index()].selector),
        ),
        0xa1 => pop_segment(decoder, SegmentName::Fs),
        0xa8 => push(
            decoder,
            width,
            u64::from(decoder.state.segments[SegmentName::Gs.index()].selector),
        ),
        0xa9 => pop_segment(decoder, SegmentName::Gs),
        0xaf => {
            let decoded = decoder.modrm(width)?;
            let left = decoder.read(decoded.register, width)?;
            let right = decoder.read(decoded.operand, width)?;
            let value = signed_multiply(width.bits(), left, right, &mut decoder.state.flags);
            decoder.write(decoded.register, width, value)
        }
        0xb6 | 0xb7 | 0xbe | 0xbf => {
            let bits = if op & 1 == 0 { X86Width::W8 } else { X86Width::W16 };
            let decoded = decoder.modrm(bits)?;
            let value = decoder.read(decoded.operand, bits)?;
            let extended = if op >= 0xbe {
                ((((value & mask_for(bits)) << (64 - bits.bits())) as i64) >> (64 - bits.bits())) as u64
            } else {
                value
            };
            decoder.write(register_operand(decoded.group, width)?, width, extended)
        }
        0xbc | 0xbd => {
            let decoded = decoder.modrm(width)?;
            let value = decoder.read(decoded.operand, width)? & mask_for(width);
            decoder.state.flags.set(GuestFlag::Zero, value == 0);
            if value != 0 {
                let mut bit: i64 = if op == 0xbc { 0 } else { width.bits() as i64 - 1 };
                while value & (1u64 << bit) == 0 {
                    bit += if op == 0xbc { 1 } else { -1 };
                }
                decoder.write(decoded.register, width, bit as u64)?;
            }
            Ok(())
        }
        0xb0 | 0xb1 | 0xc0 | 0xc1 => {
            let bits = if op & 1 == 0 { X86Width::W8 } else { width };
            let decoded = decoder.modrm(bits)?;
            lock(decoder, decoded.operand, true)?;
            let destination = decoder.read(decoded.operand, bits)?;
            let source = decoder.read(decoded.register, bits)?;
            decoder.check_write(decoded.operand, bits)?;
            if op < 0xc0 {
                let accumulator = decoder.read(register_operand(0, bits)?, bits)?;
                alu(
                    AluOperation::Cmp,
                    bits.bits(),
                    accumulator,
                    destination,
                    &mut decoder.state.flags,
                );
                if accumulator == destination {
                    decoder.write(decoded.operand, bits, source)?;
                } else {
                    decoder.write(decoded.operand, bits, destination)?;
                    decoder.write(register_operand(0, bits)?, bits, destination)?;
                }
            } else {
                let result = alu(
                    AluOperation::Add,
                    bits.bits(),
                    destination,
                    source,
                    &mut decoder.state.flags,
                );
                decoder.write(decoded.register, bits, destination)?;
                decoder.write(decoded.operand, bits, result)?;
            }
            Ok(())
        }
        0xa3 | 0xab | 0xb3 | 0xbb | 0xba => bit_op(decoder, op),
        0xa4 | 0xa5 | 0xac | 0xad => double_shift(decoder, op),
        0xc7 => compare_exchange8(decoder),
        _ => {
            if (0x10..=0x17).contains(&op)
                || (0x28..=0x2f).contains(&op)
                || (0x50..=0x7f).contains(&op)
                || (0xc2..=0xc6).contains(&op)
                || op >= 0xd0
                || op == 0xae
            {
                floating(decoder, numeric, Some(op))
            } else {
                Err(X86Error::unsupported(format!(
                    "Unsupported i386 opcode 0f {op:02x} at 0x{:x}",
                    decoder.start
                )))
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
fn execute(decoder: &mut X86Decoder, numeric: NumericExecutor) -> Result<bool, X86Error> {
    let op = decoder.opcode;
    let width = operand_width(decoder.operand_bits);
    if decoder.lock
        && !(op <= 0x3b && (op & 7) <= 3)
        && !matches!(
            op,
            0x80 | 0x81 | 0x82 | 0x83 | 0x86 | 0x87 | 0xf6 | 0xf7 | 0xfe | 0xff | 0x0f
        )
    {
        return Err(X86Error::fault(6, format!("LOCK is invalid for opcode 0x{op:x}")));
    }
    if op <= 0x3d && (op & 7) <= 5 {
        let operation = arithmetic_operation((op >> 3) as usize)?;
        let form = op & 7;
        let bits = if form & 1 == 0 { X86Width::W8 } else { width };
        if form <= 3 {
            let decoded = decoder.modrm(bits)?;
            let (destination, source) = if form < 2 {
                (decoded.operand, decoded.register)
            } else {
                (decoded.register, decoded.operand)
            };
            let right = decoder.read(source, bits)?;
            execute_alu(decoder, operation, bits, destination, right)?;
        } else {
            let immediate = decoder.immediate(bits)?;
            execute_alu(decoder, operation, bits, register_operand(0, bits)?, immediate)?;
        }
        return Ok(false);
    }
    if (0x40..=0x4f).contains(&op) {
        incdec(decoder, register_operand((op & 7) as usize, width)?, width, op >= 0x48)?;
        return Ok(false);
    }
    if (0x50..=0x57).contains(&op) {
        let value = decoder.read(register_operand((op & 7) as usize, width)?, width)?;
        push(decoder, width, value)?;
        return Ok(false);
    }
    if (0x58..=0x5f).contains(&op) {
        let value = pop(decoder, width)?;
        decoder.write(register_operand((op & 7) as usize, width)?, width, value)?;
        return Ok(false);
    }
    if (0x70..=0x7f).contains(&op) {
        let relative = decoder.signed(X86Width::W8)?;
        let take = condition(op & 15, &decoder.state.flags)?;
        if take {
            decoder.cursor = (decoder.cursor as i64).wrapping_add(relative) as u64 & mask_for(width);
        }
        return Ok(false);
    }
    if (0x91..=0x97).contains(&op) {
        let operand = register_operand((op & 7) as usize, width)?;
        let accumulator = register_operand(0, width)?;
        let left = decoder.read(accumulator, width)?;
        {
            let value = decoder.read(operand, width)?;
            decoder.write(accumulator, width, value)?;
        }
        decoder.write(operand, width, left)?;
        return Ok(false);
    }
    if (0xb0..=0xbf).contains(&op) {
        let bits = if op < 0xb8 { X86Width::W8 } else { width };
        let immediate = decoder.immediate(bits)?;
        decoder.write(register_operand((op & 7) as usize, bits)?, bits, immediate)?;
        return Ok(false);
    }
    if (0xd8..=0xdf).contains(&op) {
        floating(decoder, numeric, None)?;
        return Ok(false);
    }
    match op {
        0x0f => {
            extended(decoder, numeric)?;
            Ok(false)
        }
        0x06 => push(
            decoder,
            width,
            u64::from(decoder.state.segments[SegmentName::Es.index()].selector),
        )
        .map(|()| false),
        0x0e => push(
            decoder,
            width,
            u64::from(decoder.state.segments[SegmentName::Cs.index()].selector),
        )
        .map(|()| false),
        0x16 => push(
            decoder,
            width,
            u64::from(decoder.state.segments[SegmentName::Ss.index()].selector),
        )
        .map(|()| false),
        0x1e => push(
            decoder,
            width,
            u64::from(decoder.state.segments[SegmentName::Ds.index()].selector),
        )
        .map(|()| false),
        0x07 => pop_segment(decoder, SegmentName::Es).map(|()| false),
        0x17 => pop_segment(decoder, SegmentName::Ss).map(|()| false),
        0x1f => pop_segment(decoder, SegmentName::Ds).map(|()| false),
        0x60 => {
            let initial_sp = decoder.register_value(4, width)?;
            let stack = decoder.register_value(4, X86Width::W32)?;
            let target = stack_operand(stack.wrapping_sub(width.bits() as u64));
            let address = decoder.address(target, width.bits() as usize, GuestAccess::Write)?;
            decoder
                .memory
                .check(address, width.bits() as usize, GuestAccess::Write)?;
            for index in 0..8 {
                let value = if index == 4 {
                    initial_sp
                } else {
                    decoder.read(register_operand(index, width)?, width)?
                };
                push(decoder, width, value)?;
            }
            Ok(false)
        }
        0x61 => {
            for index in (0..8).rev() {
                let value = pop(decoder, width)?;
                if index != 4 {
                    decoder.write(register_operand(index, width)?, width, value)?;
                }
            }
            Ok(false)
        }
        0x68 => {
            let immediate = decoder.immediate(width)?;
            push(decoder, width, immediate)?;
            Ok(false)
        }
        0x6a => {
            let immediate = decoder.signed(X86Width::W8)? as u64;
            push(decoder, width, immediate)?;
            Ok(false)
        }
        0x69 | 0x6b => {
            let decoded = decoder.modrm(width)?;
            let immediate = if op == 0x6b {
                decoder.signed(X86Width::W8)? as u64
            } else {
                decoder.signed(width)? as u64
            };
            let value = signed_multiply(
                width.bits(),
                decoder.read(decoded.operand, width)?,
                immediate,
                &mut decoder.state.flags,
            );
            decoder.write(decoded.register, width, value)?;
            Ok(false)
        }
        0x80..=0x83 => {
            let bits = if op == 0x80 || op == 0x82 { X86Width::W8 } else { width };
            let decoded = decoder.modrm(bits)?;
            let immediate = if op == 0x83 {
                decoder.signed(X86Width::W8)? as u64
            } else {
                decoder.immediate(bits)?
            };
            execute_alu(
                decoder,
                arithmetic_operation(decoded.group)?,
                bits,
                decoded.operand,
                immediate,
            )?;
            Ok(false)
        }
        0x84 | 0x85 => {
            let bits = if op == 0x84 { X86Width::W8 } else { width };
            let decoded = decoder.modrm(bits)?;
            let right = decoder.read(decoded.register, bits)?;
            execute_alu(decoder, AluOperation::Test, bits, decoded.operand, right)?;
            Ok(false)
        }
        0x86 | 0x87 => {
            let bits = if op == 0x86 { X86Width::W8 } else { width };
            let decoded = decoder.modrm(bits)?;
            let old = decoder.read(decoded.operand, bits)?;
            let replacement = decoder.read(decoded.register, bits)?;
            lock(decoder, decoded.operand, true)?;
            decoder.write(decoded.operand, bits, replacement)?;
            decoder.write(decoded.register, bits, old)?;
            Ok(false)
        }
        0x88..=0x8b => {
            let bits = if op & 1 == 0 { X86Width::W8 } else { width };
            let decoded = decoder.modrm(bits)?;
            let (destination, source) = if op < 0x8a {
                (decoded.operand, decoded.register)
            } else {
                (decoded.register, decoded.operand)
            };
            let value = decoder.read(source, bits)?;
            decoder.write(destination, bits, value)?;
            Ok(false)
        }
        0x8c | 0x8e => {
            let decoded = decoder.modrm(X86Width::W16)?;
            let segment = segment(decoded.group)?;
            if op == 0x8c {
                let bits = if matches!(decoded.operand, X86Operand::Register { .. }) {
                    width
                } else {
                    X86Width::W16
                };
                decoder.write(
                    decoded.operand,
                    bits,
                    u64::from(decoder.state.segments[segment.index()].selector),
                )?;
            } else {
                let selector = decoder.read(decoded.operand, X86Width::W16)? as u16;
                set_selector(decoder, segment, selector)?;
            }
            Ok(false)
        }
        0x8d => {
            let decoded = decoder.modrm(width)?;
            let X86Operand::Memory { offset, .. } = decoded.operand else {
                return Err(X86Error::fault(6, "LEA requires a memory addressing form"));
            };
            decoder.write(decoded.register, width, offset)?;
            Ok(false)
        }
        0x8f => {
            let decoded = decoder.modrm(width)?;
            if decoded.group != 0 {
                return Err(X86Error::fault(6, "Invalid POP group selector"));
            }
            let value = pop(decoder, width)?;
            let target = match decoded.operand {
                X86Operand::Memory {
                    offset,
                    segment,
                    stack_pointer_base: true,
                } => {
                    let wrap = if decoder.address_bits == 16 {
                        0xffff
                    } else {
                        0xffff_ffff
                    };
                    X86Operand::Memory {
                        offset: offset.wrapping_add(width.bytes() as u64) & wrap,
                        segment,
                        stack_pointer_base: true,
                    }
                }
                _ => decoded.operand,
            };
            decoder.write(target, width, value)?;
            Ok(false)
        }
        0x90 => Ok(false),
        0x98 => {
            let value = decoder.read(register_operand(0, width)?, width)?;
            let extended = if width == X86Width::W16 {
                i64::from((value & 0xffff) as u16 as i8 as i16) as u64
            } else {
                i64::from((value & 0xffff_ffff) as u32 as i16 as i32) as u64
            };
            decoder.write(register_operand(0, width)?, width, extended)?;
            Ok(false)
        }
        0x99 => {
            let value = decoder.read(register_operand(0, width)?, width)? & mask_for(width);
            let negative = value & (1u64 << (width.bits() - 1)) != 0;
            decoder.write(
                register_operand(2, width)?,
                width,
                if negative { mask_for(width) } else { 0 },
            )?;
            Ok(false)
        }
        0x9b => {
            floating(decoder, numeric, None)?;
            Ok(false)
        }
        0x9c => push(decoder, width, (decoder.state.flags.value() & !0x30000) | 2).map(|()| false),
        0x9d => {
            let value = pop(decoder, width)?;
            let mask = if width == X86Width::W16 { 0x4dd5 } else { 0x244dd5 };
            decoder
                .state
                .flags
                .set_value(((decoder.state.flags.value() & !mask) | (value & mask) | 2) & !0x10000);
            Ok(false)
        }
        0x9e => {
            let value = decoder.read(register_operand(4, X86Width::W8)?, X86Width::W8)?;
            decoder
                .state
                .flags
                .set_value((decoder.state.flags.value() & !0xd5) | (value & 0xd5));
            Ok(false)
        }
        0x9f => decoder
            .write(
                register_operand(4, X86Width::W8)?,
                X86Width::W8,
                (decoder.state.flags.value() & 0xd5) | 2,
            )
            .map(|()| false),
        0xa0..=0xa3 => {
            let bits = if op & 1 == 0 { X86Width::W8 } else { width };
            let operand = X86Operand::Memory {
                offset: decoder.immediate(address_width_bits(decoder))?,
                segment: decoder.segment.unwrap_or(SegmentName::Ds),
                stack_pointer_base: false,
            };
            let accumulator = register_operand(0, bits)?;
            let (destination, source) = if op < 0xa2 {
                (accumulator, operand)
            } else {
                (operand, accumulator)
            };
            let value = decoder.read(source, bits)?;
            decoder.write(destination, bits, value)?;
            Ok(false)
        }
        0xa4 | 0xa5 | 0xa6 | 0xa7 | 0xaa | 0xab | 0xac | 0xad | 0xae | 0xaf => {
            string_op(decoder)?;
            Ok(false)
        }
        0xa8 | 0xa9 => {
            let bits = if op == 0xa8 { X86Width::W8 } else { width };
            let immediate = decoder.immediate(bits)?;
            execute_alu(decoder, AluOperation::Test, bits, register_operand(0, bits)?, immediate)?;
            Ok(false)
        }
        0xc0 | 0xc1 | 0xd0 | 0xd1 | 0xd2 | 0xd3 => {
            let bits = if op & 1 == 0 { X86Width::W8 } else { width };
            let decoded = decoder.modrm(bits)?;
            let count = if op < 0xd0 {
                u32::from(decoder.byte()?)
            } else if op < 0xd2 {
                1
            } else {
                decoder.read(register_operand(1, X86Width::W8)?, X86Width::W8)? as u32
            };
            let value = decoder.read(decoded.operand, bits)?;
            decoder.check_write(decoded.operand, bits)?;
            let result = shift(
                shift_operation(decoded.group)?,
                bits.bits(),
                value,
                count,
                &mut decoder.state.flags,
            );
            decoder.write(decoded.operand, bits, result)?;
            Ok(false)
        }
        0xc2 | 0xc3 => {
            let adjustment = if op == 0xc2 {
                decoder.immediate(X86Width::W16)?
            } else {
                0
            };
            let target = pop(decoder, width)?;
            decoder.cursor = target & mask_for(width);
            let rsp = decoder.register_value(4, X86Width::W32)?;
            decoder.state.registers.write(
                GuestRegister::Rsp,
                X86Width::W32.register_width(),
                rsp.wrapping_add(adjustment),
                false,
            )?;
            Ok(false)
        }
        0xc6 | 0xc7 => {
            let bits = if op == 0xc6 { X86Width::W8 } else { width };
            let decoded = decoder.modrm(bits)?;
            if decoded.group != 0 {
                return Err(X86Error::fault(6, "Invalid MOV immediate group selector"));
            }
            let immediate = decoder.immediate(bits)?;
            decoder.write(decoded.operand, bits, immediate)?;
            Ok(false)
        }
        0xc8 => {
            enter(decoder)?;
            Ok(false)
        }
        0xc9 => {
            let rbp = decoder.register_value(5, X86Width::W32)?;
            decoder
                .state
                .registers
                .write(GuestRegister::Rsp, X86Width::W32.register_width(), rbp, false)?;
            let value = pop(decoder, width)?;
            decoder.write(register_operand(5, width)?, width, value)?;
            Ok(false)
        }
        0xcc => Err(X86Error::fault(3, "INT3 breakpoint")),
        0xcd => {
            let vector = decoder.byte()?;
            Err(X86Error::fault_code(
                13,
                format!("INT {vector} requires a guest interrupt service"),
                0,
            ))
        }
        0xce => {
            if decoder.state.flags.get(GuestFlag::Overflow) {
                return Err(X86Error::fault(4, "INTO overflow trap"));
            }
            Ok(false)
        }
        0xd4 => {
            let base = u64::from(decoder.byte()?);
            if base == 0 {
                return Err(X86Error::fault(0, "AAM divide by zero"));
            }
            let value = decoder.read(register_operand(0, X86Width::W8)?, X86Width::W8)?;
            decoder.write(
                register_operand(0, X86Width::W16)?,
                X86Width::W16,
                ((value / base) << 8) | (value % base),
            )?;
            result_flags(8, value % base, &mut decoder.state.flags);
            Ok(false)
        }
        0xd5 => {
            let base = u64::from(decoder.byte()?);
            let result = decoder
                .read(register_operand(0, X86Width::W8)?, X86Width::W8)?
                .wrapping_add(decoder.read(register_operand(4, X86Width::W8)?, X86Width::W8)? * base)
                & 0xff;
            decoder.write(register_operand(0, X86Width::W16)?, X86Width::W16, result)?;
            result_flags(8, result, &mut decoder.state.flags);
            Ok(false)
        }
        0xd7 => {
            let addr_width = address_width_bits(decoder);
            let wrap = if decoder.address_bits == 16 {
                0xffff
            } else {
                0xffff_ffff
            };
            let offset = decoder
                .register_value(3, addr_width)?
                .wrapping_add(decoder.read(register_operand(0, X86Width::W8)?, X86Width::W8)?)
                & wrap;
            let value = decoder.read(
                X86Operand::Memory {
                    offset,
                    segment: decoder.segment.unwrap_or(SegmentName::Ds),
                    stack_pointer_base: false,
                },
                X86Width::W8,
            )?;
            decoder.write(register_operand(0, X86Width::W8)?, X86Width::W8, value)?;
            Ok(false)
        }
        0xe0..=0xe3 => {
            let displacement = decoder.signed(X86Width::W8)?;
            let addr_width = address_width_bits(decoder);
            let mut count = decoder.register_value(1, addr_width)?;
            if op != 0xe3 {
                count = count.wrapping_sub(1) & mask_for(addr_width);
                decoder.write(register_operand(1, addr_width)?, addr_width, count)?;
            }
            let take = if op == 0xe3 {
                count == 0
            } else {
                count != 0 && (op == 0xe2 || decoder.state.flags.get(GuestFlag::Zero) == (op == 0xe1))
            };
            if take {
                decoder.cursor = (decoder.cursor as i64).wrapping_add(displacement) as u64 & mask_for(width);
            }
            Ok(false)
        }
        0xe8 => {
            let displacement = decoder.signed(width)?;
            let next = decoder.cursor;
            push(decoder, width, next)?;
            decoder.cursor = (next as i64).wrapping_add(displacement) as u64 & mask_for(width);
            Ok(false)
        }
        0xe9 | 0xeb => {
            let displacement = decoder.signed(if op == 0xeb { X86Width::W8 } else { width })?;
            decoder.cursor = (decoder.cursor as i64).wrapping_add(displacement) as u64 & mask_for(width);
            Ok(false)
        }
        0xf4 => Ok(true),
        0xf5 => {
            let carry = decoder.state.flags.get(GuestFlag::Carry);
            decoder.state.flags.set(GuestFlag::Carry, !carry);
            Ok(false)
        }
        0xf6 | 0xf7 => {
            unary(decoder, if op == 0xf6 { X86Width::W8 } else { width })?;
            Ok(false)
        }
        0xf8 => {
            decoder.state.flags.set(GuestFlag::Carry, false);
            Ok(false)
        }
        0xf9 => {
            decoder.state.flags.set(GuestFlag::Carry, true);
            Ok(false)
        }
        0xfa | 0xfb => Err(X86Error::fault_code(
            13,
            "CLI/STI requires privileged guest execution",
            0,
        )),
        0xfc => {
            decoder.state.flags.set(GuestFlag::Direction, false);
            Ok(false)
        }
        0xfd => {
            decoder.state.flags.set(GuestFlag::Direction, true);
            Ok(false)
        }
        0xfe | 0xff => {
            let bits = if op == 0xfe { X86Width::W8 } else { width };
            let decoded = decoder.modrm(bits)?;
            if decoded.group <= 1 {
                incdec(decoder, decoded.operand, bits, decoded.group == 1)?;
            } else {
                lock(decoder, decoded.operand, false)?;
                if op == 0xfe {
                    return Err(X86Error::fault(6, "Invalid FE group selector"));
                }
                let target = decoder.read(decoded.operand, width)?;
                if decoded.group == 2 {
                    push(decoder, width, decoder.cursor)?;
                    decoder.cursor = target;
                } else if decoded.group == 4 {
                    decoder.cursor = target;
                } else if decoded.group == 6 {
                    push(decoder, width, target)?;
                } else {
                    return Err(X86Error::unsupported(format!(
                        "FF group /{} far transfer is unsupported",
                        decoded.group
                    )));
                }
            }
            Ok(false)
        }
        _ => Err(X86Error::unsupported(format!(
            "Unsupported i386 opcode 0x{op:02x} at 0x{:x}",
            decoder.start
        ))),
    }
}

/// User-mode i386 interpreter.
pub struct I386Cpu {
    /// Processor state.
    pub state: GuestProcessorState,
    /// Guest memory.
    pub memory: SparseGuestMemory,
    hooks: Option<Rc<HookState>>,
    numeric: NumericExecutor,
}

impl I386Cpu {
    /// CPU over `state` and `memory` with the default numeric executor.
    pub fn new(state: GuestProcessorState, memory: SparseGuestMemory) -> Result<Self, GuestError> {
        Self::with_numeric(state, memory, execute_numeric_instruction)
    }

    /// CPU with an injected numeric executor.
    pub fn with_numeric(
        state: GuestProcessorState,
        memory: SparseGuestMemory,
        numeric: NumericExecutor,
    ) -> Result<Self, GuestError> {
        if state.architecture != crate::core::contracts::GuestArchitecture::I386 || memory.pointer_bytes() != 4 {
            return Err(GuestError::cpu(
                "I386Cpu requires i386 state and a 32-bit guest address space",
            ));
        }
        Ok(Self {
            state,
            memory,
            hooks: None,
            numeric,
        })
    }

    /// Installed hook state.
    #[must_use]
    pub fn hooks(&self) -> Option<&Rc<HookState>> {
        self.hooks.as_ref()
    }
}

impl GuestCpu for I386Cpu {
    fn parts(&mut self) -> (&mut GuestProcessorState, &mut SparseGuestMemory) {
        (&mut self.state, &mut self.memory)
    }

    fn set_hook_state(&mut self, hooks: Option<Rc<HookState>>) {
        self.hooks = hooks;
    }

    fn run(&mut self, instruction_budget: u64, return_address: Option<GuestAddress>) -> GuestExecutionStop {
        if let Some(address) = return_address {
            if address.space != self.memory.address_space() {
                return GuestExecutionStop::Unsupported {
                    instructions: 0,
                    instruction: GuestInstruction {
                        address,
                        bytes: Vec::new(),
                        mnemonic: "unsupported".to_string(),
                    },
                    detail: "Return address belongs to another guest address space".to_string(),
                };
            }
        }
        let mut instructions = 0;
        while instructions < instruction_budget {
            let original_ip = self.state.instruction_pointer;
            let registers = self.state.registers.checkpoint();
            let flags = self.state.flags.value();
            let step = self.step(return_address, &mut instructions);
            match step {
                StepOutcome::Continue => {}
                StepOutcome::Stop(stop) => return stop,
                StepOutcome::Fault {
                    error,
                    decoded_cursor,
                    decoded_bytes,
                } => {
                    self.state.registers.restore(&registers).ok();
                    self.state.flags.set_value(flags);
                    self.state.instruction_pointer = original_ip;
                    let instruction_address = GuestAddress::new(
                        self.memory.address_space(),
                        self.state.segments[GuestProcessorState::CS]
                            .base
                            .wrapping_add(original_ip) as u32 as u64,
                    );
                    match error {
                        X86Error::Memory {
                            access,
                            address,
                            byte_length,
                            detail,
                        } => {
                            return GuestExecutionStop::Exception {
                                instructions,
                                exception: GuestException::Memory {
                                    access,
                                    address: GuestAddress::new(self.memory.address_space(), address),
                                    byte_length,
                                    detail,
                                },
                            };
                        }
                        X86Error::Fault {
                            vector,
                            error_code,
                            detail,
                        } => {
                            if (vector == 3 || vector == 4) && decoded_cursor.is_some() {
                                self.state.instruction_pointer = decoded_cursor.unwrap_or(original_ip);
                                instructions += 1;
                            }
                            return GuestExecutionStop::Exception {
                                instructions,
                                exception: GuestException::Processor {
                                    vector,
                                    error_code,
                                    instruction: instruction_address,
                                    detail,
                                },
                            };
                        }
                        X86Error::Unsupported(detail) => {
                            return GuestExecutionStop::Unsupported {
                                instructions,
                                instruction: GuestInstruction {
                                    address: instruction_address,
                                    bytes: decoded_bytes,
                                    mnemonic: "unsupported".to_string(),
                                },
                                detail,
                            };
                        }
                    }
                }
            }
        }
        GuestExecutionStop::Budget { instructions }
    }
}

enum StepOutcome {
    Continue,
    Stop(GuestExecutionStop),
    Fault {
        error: X86Error,
        decoded_cursor: Option<u64>,
        decoded_bytes: Vec<u8>,
    },
}

impl I386Cpu {
    fn step(&mut self, return_address: Option<GuestAddress>, instructions: &mut u64) -> StepOutcome {
        let cs_base = self.state.segments[GuestProcessorState::CS].base;
        let offset = cs_base.wrapping_add(self.state.instruction_pointer) as u32 as u64;
        let address = match guest_address(&mut self.memory, offset, GuestAccess::Execute, 1) {
            Ok(address) => address,
            Err(error) => {
                return StepOutcome::Fault {
                    error,
                    decoded_cursor: None,
                    decoded_bytes: Vec::new(),
                };
            }
        };
        if let Some(target) = return_address {
            if address.offset == target.offset {
                return StepOutcome::Stop(GuestExecutionStop::Return {
                    instructions: *instructions,
                    address,
                });
            }
        }
        if let Some(hooks) = self.hooks.clone() {
            hooks.entry_rsp.set(
                self.state
                    .registers
                    .read(GuestRegister::Rsp, X86Width::W32.register_width(), false)
                    .unwrap_or(0),
            );
            match hooks.callbacks.borrow_mut().enter(&mut self.memory, address) {
                Ok(true) => {
                    return StepOutcome::Stop(GuestExecutionStop::HostCall {
                        instructions: *instructions,
                        address,
                    });
                }
                Ok(false) => {}
                Err(error) => {
                    return StepOutcome::Fault {
                        error: X86Error::from(error),
                        decoded_cursor: None,
                        decoded_bytes: Vec::new(),
                    };
                }
            }
        }
        let numeric = self.numeric;
        let mut decoder = match X86Decoder::new(&mut self.state, &mut self.memory) {
            Ok(decoder) => decoder,
            Err(error) => {
                return StepOutcome::Fault {
                    error,
                    decoded_cursor: None,
                    decoded_bytes: Vec::new(),
                };
            }
        };
        let halt = match execute(&mut decoder, numeric) {
            Ok(halt) => halt,
            Err(error) => {
                return StepOutcome::Fault {
                    error,
                    decoded_cursor: Some(decoder.cursor),
                    decoded_bytes: decoder.bytes.clone(),
                };
            }
        };
        let cursor = decoder.cursor;
        drop(decoder);
        self.state.instruction_pointer = cursor;
        *instructions += 1;
        if halt {
            StepOutcome::Stop(GuestExecutionStop::Halt {
                instructions: *instructions,
                address,
            })
        } else {
            StepOutcome::Continue
        }
    }
}
