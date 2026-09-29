//! Untyped SSE moves and bitwise logic over raw XMM bytes.
//!
//! Donor: `src/guest/floating-point/raw-sse.ts`. These operations preserve
//! NaN payloads, signed zeros, and integer aliases by never decoding lanes.

use crate::core::memory::SparseGuestMemory;
use crate::core::registers::GuestProcessorState;

use super::contracts::{NumericError, NumericOperand, NumericPrefix};

/// Qualified raw SSE operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawSseOperation {
    /// Register/memory move preserving raw bytes.
    Move {
        /// XMM register index.
        register_index: usize,
        /// Transfer size (4, 8, or 16; short loads zero the upper lanes).
        size: usize,
        /// Store to memory/register rather than load.
        store: bool,
        /// Requires 16-byte memory alignment.
        aligned: bool,
    },
    /// Bitwise logic over 16 bytes.
    Logic {
        /// XMM register index.
        register_index: usize,
        /// Logic operation.
        operation: RawLogic,
    },
}

/// Bitwise logic operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawLogic {
    /// Bitwise AND.
    And,
    /// Bitwise AND-NOT (destination inverted).
    AndNot,
    /// Bitwise OR.
    Or,
    /// Bitwise XOR.
    Xor,
}

/// Qualify a secondary opcode as a raw move/logic operation, if applicable.
#[must_use]
pub fn prepare_raw_sse(opcode: u8, prefix: NumericPrefix, register_index: usize) -> Option<RawSseOperation> {
    let is_move = opcode == 0x10
        || opcode == 0x11
        || ((opcode == 0x28 || opcode == 0x29) && matches!(prefix, NumericPrefix::None | NumericPrefix::X66))
        || ((opcode == 0x6f || opcode == 0x7f) && matches!(prefix, NumericPrefix::X66 | NumericPrefix::XF3));
    if is_move {
        let scalar = (opcode == 0x10 || opcode == 0x11) && matches!(prefix, NumericPrefix::XF2 | NumericPrefix::XF3);
        let size = if scalar {
            if matches!(prefix, NumericPrefix::XF2) {
                8
            } else {
                4
            }
        } else {
            16
        };
        return Some(RawSseOperation::Move {
            register_index,
            size,
            store: opcode == 0x11 || opcode == 0x29 || opcode == 0x7f,
            aligned: opcode == 0x28
                || opcode == 0x29
                || ((opcode == 0x6f || opcode == 0x7f) && matches!(prefix, NumericPrefix::X66)),
        });
    }
    let is_logic = ((0x54..=0x57).contains(&opcode) && matches!(prefix, NumericPrefix::None | NumericPrefix::X66))
        || (matches!(opcode, 0xdb | 0xdf | 0xeb | 0xef) && matches!(prefix, NumericPrefix::X66));
    if is_logic {
        let operation = if opcode == 0x54 || opcode == 0xdb {
            RawLogic::And
        } else if opcode == 0x55 || opcode == 0xdf {
            RawLogic::AndNot
        } else if opcode == 0x56 || opcode == 0xeb {
            RawLogic::Or
        } else {
            RawLogic::Xor
        };
        return Some(RawSseOperation::Logic {
            register_index,
            operation,
        });
    }
    None
}

fn valid_register(xmm: &[u8], index: usize) -> bool {
    (index + 1).checked_mul(16).is_some_and(|end| end <= xmm.len())
}

/// Execute a qualified raw operation.
pub fn execute_raw_sse(
    operation: RawSseOperation,
    operand: NumericOperand,
    state: &mut GuestProcessorState,
    memory: &mut SparseGuestMemory,
) -> Result<(), NumericError> {
    let (register_index, is_move) = match operation {
        RawSseOperation::Move { register_index, .. } => (register_index, true),
        RawSseOperation::Logic { register_index, .. } => (register_index, false),
    };
    if !valid_register(&state.simd.xmm, register_index) {
        return Err(NumericError::unsupported("Invalid XMM register index"));
    }
    let destination = register_index * 16;
    if let NumericOperand::Memory(address) = operand {
        let aligned = match operation {
            RawSseOperation::Move { aligned, .. } => aligned,
            RawSseOperation::Logic { .. } => true,
        };
        if aligned && address.offset % 16 != 0 {
            return Err(NumericError::fault(13, "Unaligned 16-byte SIMD operand"));
        }
    }
    if let NumericOperand::Register(index) = operand {
        if !valid_register(&state.simd.xmm, index) {
            return Err(NumericError::unsupported("Invalid XMM register index"));
        }
    }
    if is_move {
        let RawSseOperation::Move { size, store, .. } = operation else {
            unreachable!("move checked above");
        };
        return execute_move(destination, size, store, operand, state, memory);
    }
    let RawSseOperation::Logic { operation, .. } = operation else {
        unreachable!("logic checked above");
    };
    let right: [u8; 16] = match operand {
        NumericOperand::Register(index) => {
            let start = index * 16;
            state.simd.xmm[start..start + 16]
                .try_into()
                .map_err(|_| NumericError::unsupported("Qualified SSE register range is incomplete"))?
        }
        NumericOperand::Memory(address) => {
            let bytes = memory.copy(address, 16)?;
            bytes
                .try_into()
                .map_err(|_| NumericError::unsupported("Qualified SSE source range is incomplete"))?
        }
    };
    for (index, right_byte) in right.iter().enumerate() {
        let left = state.simd.xmm[destination + index];
        state.simd.xmm[destination + index] = match operation {
            RawLogic::And => left & right_byte,
            RawLogic::AndNot => !left & right_byte,
            RawLogic::Or => left | right_byte,
            RawLogic::Xor => left ^ right_byte,
        };
    }
    Ok(())
}

fn execute_move(
    destination: usize,
    size: usize,
    store: bool,
    operand: NumericOperand,
    state: &mut GuestProcessorState,
    memory: &mut SparseGuestMemory,
) -> Result<(), NumericError> {
    match operand {
        NumericOperand::Register(index) => {
            let other = index * 16;
            if store {
                let bytes: Vec<u8> = state.simd.xmm[destination..destination + size].to_vec();
                state.simd.xmm[other..other + size].copy_from_slice(&bytes);
            } else {
                let bytes: Vec<u8> = state.simd.xmm[other..other + size].to_vec();
                state.simd.xmm[destination..destination + size].copy_from_slice(&bytes);
            }
            Ok(())
        }
        NumericOperand::Memory(address) => {
            if store {
                let bytes: Vec<u8> = state.simd.xmm[destination..destination + size].to_vec();
                memory.write(address, &bytes)?;
            } else {
                let bytes = memory.copy(address, size)?;
                state.simd.xmm[destination..destination + size].copy_from_slice(&bytes);
                if size < 16 {
                    state.simd.xmm[destination + size..destination + 16].fill(0);
                }
            }
            Ok(())
        }
    }
}
