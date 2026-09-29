//! x86 ABI adapter: marshal calls between host values and guest state.
//!
//! Donor: `src/guest/abi/adapter.ts` (`X86AbiAdapter`). `enter` builds the
//! guest stack frame (temporaries, hidden result buffer, return address) and
//! jumps to the target; `leave` writes the host result and returns through
//! the saved address.

use crate::abi::classify::{
    plan_guest_call, plan_guest_call_layouts, AbiArgument, AbiCallPlan, AbiLocation, AbiResult,
};
use crate::abi::values::{
    align_down, argument_bytes, decode_value, encode_argument_value, encode_integer_value,
    encode_value, inferred_layout, value_alignment, value_bytes,
};
use crate::abi::GuestCpu;
use crate::core::contracts::{
    GuestAccess, GuestAddress, GuestCallResult, GuestCallSignature, GuestCallValue, GuestFlag,
    GuestIntegerWidth, GuestRegister, GuestStorage, GuestValueLayout, NativeCallAbi,
};
use crate::core::memory::SparseGuestMemory;
use crate::core::registers::GuestProcessorState;
use crate::error::GuestError;
use crate::floating_point::binary::BinaryWidth;
use crate::floating_point::x87::{read_x87_return, write_x87_return};

/// Non-null ABI address in `memory`.
pub fn guest_pointer(memory: &SparseGuestMemory, raw: u64) -> Result<GuestAddress, GuestError> {
    memory
        .pointer(raw)?
        .ok_or_else(|| GuestError::abi("ABI address cannot be null"))
}

fn register_width(state: &GuestProcessorState) -> GuestIntegerWidth {
    match state.architecture {
        crate::core::contracts::GuestArchitecture::I386 => GuestIntegerWidth::B32,
        crate::core::contracts::GuestArchitecture::X86_64 => GuestIntegerWidth::B64,
    }
}

fn stack_pointer(state: &GuestProcessorState) -> Result<u64, GuestError> {
    let width = register_width(state);
    state.registers.read(GuestRegister::Rsp, width, false)
}

fn read_locations(
    state: &GuestProcessorState,
    memory: &mut SparseGuestMemory,
    locations: &[AbiLocation],
    size: usize,
) -> Result<Vec<u8>, GuestError> {
    let mut output = vec![0u8; size];
    for location in locations {
        if location.offset() + location.bytes() > size {
            return Err(GuestError::abi("ABI part exceeds its value"));
        }
        match location {
            AbiLocation::Integer {
                register, offset, bytes,
            } => {
                let value = state.registers.read(*register, register_width(state), false)?;
                match bytes {
                    8 => output[*offset..*offset + 8].copy_from_slice(&value.to_le_bytes()),
                    4 => output[*offset..*offset + 4]
                        .copy_from_slice(&(value as u32).to_le_bytes()),
                    2 => output[*offset..*offset + 2]
                        .copy_from_slice(&(value as u16).to_le_bytes()),
                    1 => output[*offset] = value as u8,
                    _ => {
                        for (index, slot) in output[*offset..*offset + bytes].iter_mut().enumerate() {
                            *slot = (value >> (index * 8)) as u8;
                        }
                    }
                }
            }
            AbiLocation::Sse {
                register, offset, bytes,
            } => {
                let base = register * 16;
                if base + bytes > state.simd.xmm.len() {
                    return Err(GuestError::abi("ABI XMM register is unavailable"));
                }
                output[*offset..*offset + bytes]
                    .copy_from_slice(&state.simd.xmm[base..base + bytes]);
            }
            AbiLocation::Stack {
                stack_offset, offset, bytes,
            } => {
                let address = guest_pointer(
                    memory,
                    stack_pointer(state)?.wrapping_add(*stack_offset as u64),
                )?;
                memory.copy_into(address, &mut output, *offset, *bytes)?;
            }
        }
    }
    Ok(output)
}

fn write_locations(
    state: &mut GuestProcessorState,
    memory: &mut SparseGuestMemory,
    locations: &[AbiLocation],
    bytes: &[u8],
) -> Result<(), GuestError> {
    for location in locations {
        let end = location.offset() + location.bytes();
        let part = bytes.get(location.offset()..end).ok_or_else(|| {
            GuestError::abi("ABI value is shorter than its assigned location")
        })?;
        match location {
            AbiLocation::Integer { register, bytes, .. } => {
                let mut raw = 0u64;
                for (index, byte) in part.iter().enumerate() {
                    raw |= u64::from(*byte) << (index * 8);
                }
                let _ = bytes;
                state
                    .registers
                    .write(*register, register_width(state), raw, false)?;
            }
            AbiLocation::Sse { register, .. } => {
                let base = register * 16;
                if base + part.len() > state.simd.xmm.len() {
                    return Err(GuestError::abi("ABI XMM register is unavailable"));
                }
                state.simd.xmm[base..base + part.len()].copy_from_slice(part);
            }
            AbiLocation::Stack { stack_offset, .. } => {
                let address = guest_pointer(
                    memory,
                    stack_pointer(state)?.wrapping_add(*stack_offset as u64),
                )?;
                memory.write(address, part)?;
            }
        }
    }
    Ok(())
}

fn read_scalar(
    state: &GuestProcessorState,
    memory: &mut SparseGuestMemory,
    layout: &GuestValueLayout,
    locations: &[AbiLocation],
) -> Result<Option<GuestCallValue>, GuestError> {
    let GuestValueLayout::Scalar(storage) = layout else {
        return Ok(None);
    };
    if locations.len() != 1 || locations[0].offset() != 0 {
        return Ok(None);
    }
    let location = &locations[0];
    if storage.is_float() {
        let width = if *storage == GuestStorage::Float32 { 4 } else { 8 };
        if location.bytes() != width {
            return Ok(None);
        }
        match location {
            AbiLocation::Stack { stack_offset, .. } => {
                let address = guest_pointer(
                    memory,
                    stack_pointer(state)?.wrapping_add(*stack_offset as u64),
                )?;
                if *storage == GuestStorage::Float32 {
                    return Ok(Some(GuestCallValue::Float32(memory.read_f32(address)?)));
                }
                return Ok(Some(GuestCallValue::Float64(memory.read_f64(address)?)));
            }
            AbiLocation::Sse { register, .. } => {
                let base = register * 16;
                if base + width > state.simd.xmm.len() {
                    return Err(GuestError::abi("ABI XMM register is unavailable"));
                }
                if *storage == GuestStorage::Float32 {
                    let mut word = [0u8; 4];
                    word.copy_from_slice(&state.simd.xmm[base..base + 4]);
                    return Ok(Some(GuestCallValue::Float32(f32::from_le_bytes(word))));
                }
                let mut word = [0u8; 8];
                word.copy_from_slice(&state.simd.xmm[base..base + 8]);
                return Ok(Some(GuestCallValue::Float64(f64::from_le_bytes(word))));
            }
            AbiLocation::Integer { .. } => return Ok(None),
        }
    }
    if matches!(location, AbiLocation::Sse { .. })
        || !matches!(location.bytes(), 1 | 2 | 4 | 8)
    {
        return Ok(None);
    }
    let raw = match location {
        AbiLocation::Integer { register, .. } => {
            state.registers.read(*register, register_width(state), false)?
        }
        AbiLocation::Stack { stack_offset, .. } => {
            let address = guest_pointer(
                memory,
                stack_pointer(state)?.wrapping_add(*stack_offset as u64),
            )?;
            match location.bytes() {
                8 => memory.read_u64(address)?,
                4 => u64::from(memory.read_u32(address)?),
                2 => u64::from(memory.read_u16(address)?),
                _ => u64::from(memory.read_u8(address)?),
            }
        }
        AbiLocation::Sse { .. } => return Ok(None),
    };
    Ok(Some(match storage {
        GuestStorage::Int8 => GuestCallValue::Int32(i32::from(raw as u8 as i8)),
        GuestStorage::Uint8 => GuestCallValue::Uint32(u32::from(raw as u8)),
        GuestStorage::Int16 => GuestCallValue::Int32(i32::from(raw as u16 as i16)),
        GuestStorage::Uint16 => GuestCallValue::Uint32(u32::from(raw as u16)),
        GuestStorage::Int32 => GuestCallValue::Int32(raw as u32 as i32),
        GuestStorage::Uint32 => GuestCallValue::Uint32(raw as u32),
        GuestStorage::Int64 => GuestCallValue::Int64(raw as i64),
        GuestStorage::Uint64 => GuestCallValue::Uint64(raw),
        GuestStorage::Pointer => {
            let masked = if memory.pointer_bytes() == 4 {
                raw & 0xffff_ffff
            } else {
                raw
            };
            GuestCallValue::Pointer(memory.pointer(masked)?)
        }
        GuestStorage::Float32 | GuestStorage::Float64 => return Ok(None),
    }))
}

fn read_argument(
    state: &GuestProcessorState,
    memory: &mut SparseGuestMemory,
    argument: &AbiArgument,
) -> Result<GuestCallValue, GuestError> {
    if !argument.indirect {
        if let Some(value) = read_scalar(state, memory, &argument.layout, &argument.locations)? {
            return Ok(value);
        }
        let bytes = read_locations(
            state,
            memory,
            &argument.locations,
            argument_bytes(&argument.layout, memory.pointer_bytes()),
        )?;
        if let GuestValueLayout::Aggregate(layout) = &argument.layout {
            return Ok(GuestCallValue::Aggregate {
                layout: layout.clone(),
                bytes,
            });
        }
        let width = value_bytes(&argument.layout, memory.pointer_bytes());
        return decode_value(&argument.layout, &bytes[..width], memory);
    }
    let pointer_layout = GuestValueLayout::Scalar(GuestStorage::Pointer);
    let pointer = read_scalar(state, memory, &pointer_layout, &argument.locations)?.map_or_else(
        || {
            let bytes = read_locations(state, memory, &argument.locations, memory.pointer_bytes())?;
            decode_value(&pointer_layout, &bytes, memory)
        },
        Ok,
    )?;
    let GuestCallValue::Pointer(Some(address)) = pointer else {
        return Err(GuestError::abi("Indirect aggregate has a null guest address"));
    };
    let bytes = memory.copy(address, value_bytes(&argument.layout, memory.pointer_bytes()))?;
    if let GuestValueLayout::Aggregate(layout) = &argument.layout {
        return Ok(GuestCallValue::Aggregate {
            layout: layout.clone(),
            bytes,
        });
    }
    decode_value(&argument.layout, &bytes, memory)
}

fn return_buffer(
    state: &GuestProcessorState,
    memory: &mut SparseGuestMemory,
    plan: &AbiCallPlan,
) -> Result<GuestAddress, GuestError> {
    let AbiResult::Memory { pointer, .. } = &plan.result else {
        return Err(GuestError::abi("Call does not return an aggregate in memory"));
    };
    match read_argument(state, memory, pointer)? {
        GuestCallValue::Pointer(Some(address)) => Ok(address),
        _ => Err(GuestError::abi("Aggregate return pointer is null")),
    }
}

/// Calling-convention adapter between host values and guest CPU state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X86AbiAdapter {
    /// Calling convention.
    pub abi: NativeCallAbi,
}

impl X86AbiAdapter {
    /// Adapter for `abi`.
    #[must_use]
    pub const fn new(abi: NativeCallAbi) -> Self {
        Self { abi }
    }

    fn check(
        &self,
        state: &GuestProcessorState,
        memory: &SparseGuestMemory,
        signature: &GuestCallSignature,
    ) -> Result<(), GuestError> {
        let i386 = state.architecture == crate::core::contracts::GuestArchitecture::I386;
        if signature.abi != self.abi
            || memory.pointer_bytes() != self.abi.pointer_bytes()
            || i386 != (self.abi.pointer_bytes() == 4)
        {
            return Err(GuestError::abi(
                "ABI, signature, processor, and memory architecture disagree",
            ));
        }
        Ok(())
    }

    /// Marshal arguments into guest state and jump to `target`, pushing
    /// `return_address`.
    pub fn enter(
        &self,
        cpu: &mut dyn GuestCpu,
        target: GuestAddress,
        signature: &GuestCallSignature,
        arguments: &[GuestCallValue],
        return_address: GuestAddress,
    ) -> Result<(), GuestError> {
        let (state, memory) = cpu.parts();
        self.check(state, memory, signature)?;
        memory.check(target, 1, GuestAccess::Execute)?;
        memory.check(return_address, 1, GuestAccess::Execute)?;
        let layouts: Vec<GuestValueLayout> = if arguments.len() == signature.parameters.len() {
            signature.parameters.clone()
        } else {
            arguments
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    signature
                        .parameters
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| inferred_layout(value, signature.variadic))
                })
                .collect()
        };
        let plan = plan_guest_call_layouts(signature, &layouts)?;
        let word = self.abi.pointer_bytes();
        let mut encoded: Vec<EncodedArgument> = Vec::with_capacity(arguments.len());
        for (index, value) in arguments.iter().enumerate() {
            let layout = layouts.get(index).ok_or_else(|| {
                GuestError::abi("Argument layout missing")
            })?;
            let argument = plan.arguments.get(index).ok_or_else(|| {
                GuestError::abi("Argument allocation missing")
            })?;
            let single = if argument.locations.len() == 1 {
                Some(&argument.locations[0])
            } else {
                None
            };
            if !argument.indirect
                && matches!(single, Some(AbiLocation::Integer { offset: 0, .. }))
                && matches!(
                    layout,
                    GuestValueLayout::Scalar(storage)
                        if !storage.is_float()
                )
            {
                let GuestValueLayout::Scalar(storage) = layout else {
                    unreachable!("scalar checked above");
                };
                let raw = encode_integer_value(*storage, value, memory)?;
                let extended = match storage {
                    GuestStorage::Int8 => (raw as u8 as i8 as i32) as u32 as u64,
                    GuestStorage::Int16 => (raw as u16 as i16 as i32) as u32 as u64,
                    _ => raw,
                };
                encoded.push(EncodedArgument::Integer(extended));
            } else {
                encoded.push(EncodedArgument::Bytes(encode_argument_value(
                    layout, value, memory,
                )?));
            }
        }
        let caller_stack = stack_pointer(state)?;
        let mut temporary_top = caller_stack;
        let mut reserve = |memory: &mut SparseGuestMemory,
                           size: usize,
                           alignment: usize|
         -> Result<GuestAddress, GuestError> {
            temporary_top = align_down(
                temporary_top.wrapping_sub(size as u64),
                alignment as u64,
            );
            guest_pointer(memory, temporary_top)
        };
        let output = match &plan.result {
            AbiResult::Memory { layout, .. } => Some(reserve(
                memory,
                value_bytes(layout, word),
                16.max(value_alignment(layout, word)),
            )?),
            _ => None,
        };
        let mut indirect = Vec::with_capacity(plan.arguments.len());
        for argument in &plan.arguments {
            if argument.indirect {
                indirect.push(Some(reserve(memory, value_bytes(&argument.layout, word), 16)?));
            } else {
                indirect.push(None);
            }
        }
        let entry_stack = align_down(
            temporary_top.wrapping_sub((plan.stack_bytes - word) as u64),
            plan.stack_alignment as u64,
        )
        .wrapping_sub(word as u64);
        let frame_size = caller_stack.wrapping_sub(entry_stack);
        if frame_size > caller_stack {
            return Err(GuestError::abi("Guest call frame exceeds safe bounds"));
        }
        let frame_base = guest_pointer(memory, entry_stack)?;
        memory.check(frame_base, frame_size as usize, GuestAccess::Write)?;
        memory.write_pointer(frame_base, Some(return_address))?;
        state
            .registers
            .write(GuestRegister::Rsp, register_width(state), entry_stack, false)?;
        if let (Some(output), AbiResult::Memory { layout, pointer }) = (output, &plan.result) {
            memory.write(output, &vec![0u8; value_bytes(layout, word)])?;
            let bytes = encode_value(
                &GuestValueLayout::Scalar(GuestStorage::Pointer),
                &GuestCallValue::Pointer(Some(output)),
                memory,
            )?;
            write_locations(state, memory, &pointer.locations, &bytes)?;
        }
        for (index, argument) in plan.arguments.iter().enumerate() {
            let bytes = encoded.get(index).ok_or_else(|| {
                GuestError::abi("Argument allocation missing")
            })?;
            let temporary = indirect.get(index).copied().flatten();
            match (bytes, temporary) {
                (EncodedArgument::Integer(raw), _) => {
                    let Some(AbiLocation::Integer { register, .. }) = argument.locations.first()
                    else {
                        return Err(GuestError::abi("Integer argument has no assigned register"));
                    };
                    state
                        .registers
                        .write(*register, register_width(state), *raw, false)?;
                }
                (EncodedArgument::Bytes(bytes), None) => {
                    write_locations(state, memory, &argument.locations, bytes)?;
                }
                (EncodedArgument::Bytes(bytes), Some(temporary)) => {
                    memory.write(temporary, bytes)?;
                    let encoded = encode_value(
                        &GuestValueLayout::Scalar(GuestStorage::Pointer),
                        &GuestCallValue::Pointer(Some(temporary)),
                        memory,
                    )?;
                    write_locations(state, memory, &argument.locations, &encoded)?;
                }
            }
        }
        if self.abi == NativeCallAbi::SystemVX86_64 && signature.variadic {
            state.registers.write(
                GuestRegister::Rax,
                GuestIntegerWidth::B64,
                plan.vector_registers as u64,
                false,
            )?;
        }
        state.flags.set(GuestFlag::Direction, false);
        state.instruction_pointer = target.offset;
        Ok(())
    }

    /// Read call arguments from guest state. Additional variadic layouts
    /// come from the host API contract or a format-string parser.
    pub fn arguments(
        &self,
        cpu: &mut dyn GuestCpu,
        signature: &GuestCallSignature,
        variadic_layouts: &[GuestValueLayout],
    ) -> Result<Vec<GuestCallValue>, GuestError> {
        let (state, memory) = cpu.parts();
        self.check(state, memory, signature)?;
        let plan = if variadic_layouts.is_empty() {
            plan_guest_call(signature, None)?
        } else {
            let layouts: Vec<GuestValueLayout> = signature
                .parameters
                .iter()
                .cloned()
                .chain(variadic_layouts.iter().cloned())
                .collect();
            plan_guest_call_layouts(signature, &layouts)?
        };
        plan.arguments
            .iter()
            .map(|argument| read_argument(state, memory, argument))
            .collect()
    }

    /// Read one call argument from guest state.
    pub fn argument(
        &self,
        cpu: &mut dyn GuestCpu,
        signature: &GuestCallSignature,
        index: usize,
    ) -> Result<GuestCallValue, GuestError> {
        let (state, memory) = cpu.parts();
        self.check(state, memory, signature)?;
        let plan = plan_guest_call(signature, None)?;
        let argument = plan
            .arguments
            .get(index)
            .ok_or_else(|| GuestError::abi("Guest argument index is outside its signature"))?;
        read_argument(state, memory, argument)
    }

    /// Read the call result from guest state, popping x87 returns.
    pub fn return_value(
        &self,
        cpu: &mut dyn GuestCpu,
        signature: &GuestCallSignature,
    ) -> Result<GuestCallResult, GuestError> {
        let (state, memory) = cpu.parts();
        self.check(state, memory, signature)?;
        let plan = plan_guest_call(signature, None)?;
        match &plan.result {
            AbiResult::Void => Ok(GuestCallResult::Void),
            AbiResult::Registers { layout, locations } => {
                if let Some(value) = read_scalar(state, memory, layout, locations)? {
                    return Ok(GuestCallResult::Value(value));
                }
                let bytes = read_locations(
                    state,
                    memory,
                    locations,
                    value_bytes(layout, self.abi.pointer_bytes()),
                )?;
                if let GuestValueLayout::Aggregate(record) = layout {
                    return Ok(GuestCallResult::Value(GuestCallValue::Aggregate {
                        layout: record.clone(),
                        bytes,
                    }));
                }
                Ok(GuestCallResult::Value(decode_value(layout, &bytes, memory)?))
            }
            AbiResult::Memory { layout, .. } => {
                let address = guest_pointer(
                    memory,
                    state
                        .registers
                        .read(GuestRegister::Rax, register_width(state), false)?,
                )?;
                let bytes = memory.copy(address, value_bytes(layout, self.abi.pointer_bytes()))?;
                if let GuestValueLayout::Aggregate(record) = layout {
                    return Ok(GuestCallResult::Value(GuestCallValue::Aggregate {
                        layout: record.clone(),
                        bytes,
                    }));
                }
                Ok(GuestCallResult::Value(decode_value(layout, &bytes, memory)?))
            }
            AbiResult::X87 { storage } => {
                let width = if *storage == GuestStorage::Float32 {
                    BinaryWidth::W32
                } else {
                    BinaryWidth::W64
                };
                let value = read_x87_return(&mut state.x87, width)
                    .map_err(|error| GuestError::abi(format!("x87 return read failed: {error:?}")))?;
                let top = ((state.x87.status_word >> 11) & 7) as u16;
                state.x87.tag_word |= 3 << (top * 2);
                state.x87.status_word =
                    state.x87.status_word & !0x3800 | (((top + 1) & 7) << 11);
                Ok(GuestCallResult::Value(if *storage == GuestStorage::Float32 {
                    GuestCallValue::Float32(value as f32)
                } else {
                    GuestCallValue::Float64(value)
                }))
            }
        }
    }

    /// Write a trapped host function's result and return through the saved
    /// address. Called at the trap entry, before any guest prologue.
    pub fn leave(
        &self,
        cpu: &mut dyn GuestCpu,
        signature: &GuestCallSignature,
        result: &GuestCallResult,
    ) -> Result<(), GuestError> {
        let (state, memory) = cpu.parts();
        self.check(state, memory, signature)?;
        let plan = plan_guest_call(signature, None)?;
        let word = self.abi.pointer_bytes();
        let entry_stack = stack_pointer(state)?;
        let stack_address = guest_pointer(memory, entry_stack)?;
        let address = if word == 8 {
            memory.read_u64(stack_address)?
        } else {
            u64::from(memory.read_u32(stack_address)?)
        };
        if address == 0 {
            return Err(GuestError::abi("Guest return address is null"));
        }
        match (&plan.result, result) {
            (AbiResult::Void, GuestCallResult::Void) => {}
            (AbiResult::Void, _) => {
                return Err(GuestError::abi("Void callback returned a value"));
            }
            (_, GuestCallResult::Void) => {
                return Err(GuestError::abi("Nonvoid callback did not return a value"));
            }
            (AbiResult::X87 { storage }, GuestCallResult::Value(value)) => {
                let scalar = match value {
                    GuestCallValue::Float32(value) => f64::from(*value),
                    GuestCallValue::Float64(value) => *value,
                    _ => {
                        return Err(GuestError::abi(
                            "x87 callback result must be floating point",
                        ));
                    }
                };
                write_x87_return(
                    &mut state.x87,
                    scalar,
                    if *storage == GuestStorage::Float32 {
                        BinaryWidth::W32
                    } else {
                        BinaryWidth::W64
                    },
                )
                .map_err(|error| GuestError::abi(format!("x87 return write failed: {error:?}")))?;
            }
            (AbiResult::Registers { layout, locations }, GuestCallResult::Value(value)) => {
                let single_integer = locations.len() == 1
                    && matches!(locations[0], AbiLocation::Integer { offset: 0, .. })
                    && matches!(
                        layout,
                        GuestValueLayout::Scalar(storage) if !storage.is_float()
                    );
                if single_integer {
                    let AbiLocation::Integer { register, .. } = locations[0] else {
                        unreachable!("single integer checked above");
                    };
                    let GuestValueLayout::Scalar(storage) = layout else {
                        unreachable!("scalar checked above");
                    };
                    let raw = encode_integer_value(*storage, value, memory)?;
                    state
                        .registers
                        .write(register, register_width(state), raw, false)?;
                } else {
                    let bytes = encode_value(layout, value, memory)?;
                    write_locations(state, memory, locations, &bytes)?;
                }
            }
            (AbiResult::Memory { layout, .. }, GuestCallResult::Value(value)) => {
                let bytes = encode_value(layout, value, memory)?;
                let destination = return_buffer(state, memory, &plan)?;
                memory.write(destination, &bytes)?;
                state.registers.write(
                    GuestRegister::Rax,
                    register_width(state),
                    destination.offset,
                    false,
                )?;
            }
        }
        state.registers.write(
            GuestRegister::Rsp,
            register_width(state),
            entry_stack.wrapping_add((word + plan.callee_pop_bytes) as u64),
            false,
        )?;
        state.instruction_pointer = address;
        Ok(())
    }
}

enum EncodedArgument {
    Integer(u64),
    Bytes(Vec<u8>),
}
