//! x86-64 semantic plans: decode once, execute directly or via the kernel.
//!
//! Donor: `src/guest/x64/plan.ts`. The CPU lowers each instruction to one
//! [`X64PlanOperation`]; [`execute_x64_plan`] runs it with the same
//! read/check/write ordering the kernel relies on for rollback boundaries.

use crate::core::contracts::{GuestAccess, GuestIntegerWidth};
use crate::core::memory::SparseGuestMemory;
use crate::core::registers::GuestProcessorState;
use crate::floating_point::contracts::{
    NumericExecutionContext, NumericExecutionResult, NumericInstruction, NumericOperand,
};
use crate::floating_point::execute_numeric_instruction;
use crate::floating_point::raw_sse::{execute_raw_sse, RawSseOperation};
use crate::x64::decoder::{
    canonical_address, guest_address, operand_address, read_operand, writable_operand, write_operand, X64MemoryOperand,
    X64Operand, X64RegisterOperand,
};
use crate::x86::arithmetic::{alu, condition, shift, signed_multiply, AluOperation, ShiftOperation};
use crate::x86::decoder::X86Error;

/// Control-flow outcome of one plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X64Flow {
    /// Advance to the plan's next IP.
    Advance,
    /// Branch to a canonical target.
    Branch {
        /// Canonical target.
        target: u64,
    },
    /// Halt execution.
    Halt,
    /// Software breakpoint trap.
    Trap {
        /// Trap vector.
        vector: u32,
    },
}

/// Advance flow.
pub const X64_ADVANCE: X64Flow = X64Flow::Advance;

/// Plan source: operand or immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X64PlanSource {
    /// Operand source.
    Operand(X64Operand),
    /// Immediate source.
    Immediate(u64),
}

/// One lowered semantic operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum X64PlanOperation {
    /// No operation.
    Nop,
    /// Move source to destination.
    Move {
        /// Destination.
        destination: X64Operand,
        /// Source.
        source: X64PlanSource,
    },
    /// Extend source into a register.
    Extend {
        /// Destination register.
        destination: X64RegisterOperand,
        /// Source.
        source: X64Operand,
        /// Sign extension (else zero extension).
        signed: bool,
    },
    /// Increment or decrement in place (preserving carry).
    Increment {
        /// Destination.
        destination: X64Operand,
        /// Decrement rather than increment.
        subtract: bool,
    },
    /// Shift or rotate in place.
    Shift {
        /// Destination.
        destination: X64Operand,
        /// Shift operation.
        operation: ShiftOperation,
        /// Count: immediate or CL.
        count: X64ShiftCount,
    },
    /// Signed multiply into a register.
    Multiply {
        /// Destination register.
        destination: X64RegisterOperand,
        /// Left operand.
        left: X64Operand,
        /// Right source.
        right: X64PlanSource,
    },
    /// Conditional move into a register.
    ConditionalMove {
        /// Destination register.
        destination: X64RegisterOperand,
        /// Source.
        source: X64Operand,
        /// Condition code.
        condition: u8,
    },
    /// Set byte from a condition.
    SetCondition {
        /// Destination.
        destination: X64Operand,
        /// Condition code.
        condition: u8,
    },
    /// Load effective address.
    Lea {
        /// Destination register.
        destination: X64RegisterOperand,
        /// Source memory form.
        source: X64MemoryOperand,
    },
    /// ALU operation.
    Alu {
        /// ALU operation.
        operation: AluOperation,
        /// Destination.
        destination: X64Operand,
        /// Source.
        source: X64PlanSource,
    },
    /// Conditional or unconditional relative branch.
    Branch {
        /// Condition code, or `None` for unconditional.
        condition: Option<u8>,
        /// Signed displacement from next IP.
        displacement: i64,
    },
    /// Absolute indirect jump.
    Jump {
        /// Target source.
        target: X64PlanSource,
    },
    /// Absolute call (direct or indirect).
    Call {
        /// Target source.
        target: X64PlanSource,
    },
    /// Push source.
    Push {
        /// Source.
        source: X64PlanSource,
        /// Stack width.
        width: GuestIntegerWidth,
    },
    /// Pop into destination.
    Pop {
        /// Destination.
        destination: X64Operand,
        /// Stack width.
        width: GuestIntegerWidth,
    },
    /// Return, discarding `discard` argument bytes.
    Return {
        /// Argument bytes to discard.
        discard: u64,
    },
    /// Numeric instruction with a register/absent operand.
    Numeric {
        /// Decoded numeric instruction.
        instruction: NumericInstruction,
    },
    /// Numeric instruction with a memory operand resolved at execution.
    NumericMemory {
        /// Decoded numeric instruction without its operand.
        instruction: NumericInstructionBase,
        /// Memory operand.
        operand: X64MemoryOperand,
    },
    /// Raw SSE move/logic.
    RawSse {
        /// Qualified operation.
        operation: RawSseOperation,
        /// Operand: memory form or XMM register index.
        operand: X64SseOperand,
    },
}

/// Shift count: immediate or CL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X64ShiftCount {
    /// Immediate count.
    Immediate(u32),
    /// CL count.
    Cl,
}

/// SSE operand: unresolved memory or an XMM register index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X64SseOperand {
    /// Memory operand resolved at execution.
    Memory(X64MemoryOperand),
    /// XMM register index.
    Register(usize),
}

/// Numeric instruction fields shared by register and memory forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumericInstructionBase {
    /// Primary opcode byte.
    pub opcode: u8,
    /// Secondary opcode byte for `0x0f` escapes.
    pub secondary_opcode: Option<u8>,
    /// ModRM byte, if present.
    pub modrm: Option<u8>,
    /// Register field of ModRM.
    pub register_index: usize,
    /// SIMD mandatory prefix.
    pub prefix: crate::floating_point::contracts::NumericPrefix,
    /// Operand width in bits.
    pub operand_bits: u16,
    /// Trailing immediate byte, if present.
    pub immediate: Option<u8>,
}

impl NumericInstructionBase {
    /// Complete the instruction with `operand`.
    #[must_use]
    pub const fn with_operand(self, operand: Option<NumericOperand>) -> NumericInstruction {
        NumericInstruction {
            opcode: self.opcode,
            secondary_opcode: self.secondary_opcode,
            modrm: self.modrm,
            operand,
            register_index: self.register_index,
            prefix: self.prefix,
            operand_bits: self.operand_bits,
            immediate: self.immediate,
        }
    }
}

/// Lowered plan: operation, next IP, LOCK prefix, block boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct X64SemanticPlan {
    /// Lowered operation.
    pub operation: X64PlanOperation,
    /// Next IP after the instruction.
    pub next_ip: u64,
    /// LOCK prefix present.
    pub lock: bool,
    /// Ends a cached semantic block.
    pub ends_block: bool,
}

/// Enforce LOCK-prefix rules for a memory read-modify-write.
pub fn x64_lock(lock: bool, destination: Option<X64Operand>, permitted: bool) -> Result<(), X86Error> {
    if lock && (!permitted || !matches!(destination, Some(X64Operand::Memory(_)))) {
        return Err(X86Error::fault(
            6,
            "LOCK requires a supported memory read-modify-write operand",
        ));
    }
    Ok(())
}

/// Lower an operation with its next IP, deriving the block boundary.
#[must_use]
pub fn make_x64_plan(operation: X64PlanOperation, next_ip: u64, lock: bool) -> X64SemanticPlan {
    let ends_block = match &operation {
        X64PlanOperation::Branch { .. }
        | X64PlanOperation::Return { .. }
        | X64PlanOperation::NumericMemory { .. }
        | X64PlanOperation::Call { .. }
        | X64PlanOperation::Jump { .. }
        | X64PlanOperation::Push { .. } => true,
        X64PlanOperation::Pop { destination, .. } => {
            matches!(destination, X64Operand::Memory(_))
        }
        X64PlanOperation::Increment { destination, .. } => {
            matches!(destination, X64Operand::Memory(_))
        }
        X64PlanOperation::Shift { destination, .. } | X64PlanOperation::SetCondition { destination, .. } => {
            matches!(destination, X64Operand::Memory(_))
        }
        X64PlanOperation::RawSse { operation, operand } => {
            matches!(operation, RawSseOperation::Move { store: true, .. })
                && matches!(operand, X64SseOperand::Memory(_))
        }
        X64PlanOperation::Move { destination, .. } => {
            matches!(destination, X64Operand::Memory(_))
        }
        X64PlanOperation::Alu {
            operation, destination, ..
        } => {
            matches!(destination, X64Operand::Memory(_)) && !matches!(operation, AluOperation::Cmp | AluOperation::Test)
        }
        _ => false,
    };
    X64SemanticPlan {
        operation,
        next_ip,
        lock,
        ends_block,
    }
}

fn plan_source(
    memory: &mut SparseGuestMemory,
    state: &GuestProcessorState,
    source: &X64PlanSource,
    next_ip: u64,
) -> Result<u64, X86Error> {
    match source {
        X64PlanSource::Immediate(value) => Ok(*value),
        X64PlanSource::Operand(operand) => read_operand(memory, state, operand, next_ip),
    }
}

/// Execute one semantic plan.
pub fn execute_x64_plan(
    plan: &X64SemanticPlan,
    memory: &mut SparseGuestMemory,
    state: &mut GuestProcessorState,
) -> Result<X64Flow, X86Error> {
    match &plan.operation {
        X64PlanOperation::Nop => {
            x64_lock(plan.lock, None, false)?;
            Ok(X64_ADVANCE)
        }
        X64PlanOperation::Shift {
            destination,
            operation,
            count,
        } => {
            let count = match count {
                X64ShiftCount::Immediate(count) => *count,
                X64ShiftCount::Cl => {
                    state
                        .registers
                        .read(crate::core::contracts::GuestRegister::Rcx, GuestIntegerWidth::B8, false)?
                        as u32
                }
            };
            x64_lock(plan.lock, None, false)?;
            writable_operand(memory, state, destination, plan.next_ip)?;
            let value = read_operand(memory, state, destination, plan.next_ip)?;
            let result = shift(*operation, destination.width().bits(), value, count, &mut state.flags);
            write_operand(memory, state, destination, plan.next_ip, result)?;
            Ok(X64_ADVANCE)
        }
        X64PlanOperation::Multiply {
            destination,
            left,
            right,
        } => {
            x64_lock(plan.lock, None, false)?;
            let left = read_operand(memory, state, left, plan.next_ip)?;
            let right = plan_source(memory, state, right, plan.next_ip)?;
            let value = signed_multiply(destination.width.bits(), left, right, &mut state.flags);
            write_operand(memory, state, &X64Operand::Register(*destination), plan.next_ip, value)?;
            Ok(X64_ADVANCE)
        }
        X64PlanOperation::ConditionalMove {
            destination,
            source,
            condition: code,
        } => {
            x64_lock(plan.lock, None, false)?;
            let value = read_operand(memory, state, source, plan.next_ip)?;
            if condition(*code, &state.flags)? {
                write_operand(memory, state, &X64Operand::Register(*destination), plan.next_ip, value)?;
            } else if destination.width == GuestIntegerWidth::B32 {
                // A declined 32-bit CMOV still clears the upper half.
                let current = read_operand(memory, state, &X64Operand::Register(*destination), plan.next_ip)?;
                write_operand(
                    memory,
                    state,
                    &X64Operand::Register(*destination),
                    plan.next_ip,
                    current,
                )?;
            }
            Ok(X64_ADVANCE)
        }
        X64PlanOperation::SetCondition {
            destination,
            condition: code,
        } => {
            x64_lock(plan.lock, None, false)?;
            let value = u64::from(condition(*code, &state.flags)?);
            write_operand(memory, state, destination, plan.next_ip, value)?;
            Ok(X64_ADVANCE)
        }
        X64PlanOperation::Extend {
            destination,
            source,
            signed,
        } => {
            x64_lock(plan.lock, None, false)?;
            let value = read_operand(memory, state, source, plan.next_ip)?;
            let extended = if *signed {
                let bits = source.width().bits();
                (((value << (64 - bits)) as i64) >> (64 - bits)) as u64
            } else {
                value
            };
            write_operand(
                memory,
                state,
                &X64Operand::Register(*destination),
                plan.next_ip,
                extended,
            )?;
            Ok(X64_ADVANCE)
        }
        X64PlanOperation::Increment { destination, subtract } => {
            x64_lock(plan.lock, Some(*destination), true)?;
            writable_operand(memory, state, destination, plan.next_ip)?;
            let carry = state.flags.get(crate::core::contracts::GuestFlag::Carry);
            let value = alu(
                if *subtract {
                    AluOperation::Sub
                } else {
                    AluOperation::Add
                },
                destination.width().bits(),
                read_operand(memory, state, destination, plan.next_ip)?,
                1,
                &mut state.flags,
            );
            write_operand(memory, state, destination, plan.next_ip, value)?;
            state.flags.set(crate::core::contracts::GuestFlag::Carry, carry);
            Ok(X64_ADVANCE)
        }
        X64PlanOperation::Numeric { instruction } => {
            x64_lock(plan.lock, None, false)?;
            run_numeric(memory, state, *instruction)
        }
        X64PlanOperation::NumericMemory { instruction, operand } => {
            x64_lock(plan.lock, None, false)?;
            let address = operand_address(memory, state, operand, plan.next_ip, GuestAccess::Read)?;
            run_numeric(
                memory,
                state,
                instruction.with_operand(Some(NumericOperand::Memory(address))),
            )
        }
        X64PlanOperation::RawSse { operation, operand } => {
            x64_lock(plan.lock, None, false)?;
            let operand = match operand {
                X64SseOperand::Register(index) => NumericOperand::Register(*index),
                X64SseOperand::Memory(memory_operand) => NumericOperand::Memory(operand_address(
                    memory,
                    state,
                    memory_operand,
                    plan.next_ip,
                    GuestAccess::Read,
                )?),
            };
            execute_raw_sse(*operation, operand, state, memory).map_err(|error| match error {
                crate::floating_point::contracts::NumericError::Unsupported(detail) => X86Error::unsupported(detail),
                crate::floating_point::contracts::NumericError::Fault { vector, detail } => {
                    X86Error::fault(u32::from(vector), detail)
                }
                crate::floating_point::contracts::NumericError::Guest(error) => X86Error::from(error),
            })?;
            Ok(X64_ADVANCE)
        }
        X64PlanOperation::Move { destination, source } => {
            x64_lock(plan.lock, None, false)?;
            let value = plan_source(memory, state, source, plan.next_ip)?;
            write_operand(memory, state, destination, plan.next_ip, value)?;
            Ok(X64_ADVANCE)
        }
        X64PlanOperation::Lea { destination, source } => {
            x64_lock(plan.lock, None, false)?;
            let offset = crate::x64::decoder::effective_operand_offset(state, source, plan.next_ip)?;
            write_operand(memory, state, &X64Operand::Register(*destination), plan.next_ip, offset)?;
            Ok(X64_ADVANCE)
        }
        X64PlanOperation::Alu {
            operation,
            destination,
            source,
        } => {
            let right = plan_source(memory, state, source, plan.next_ip)?;
            let writes = !matches!(operation, AluOperation::Cmp | AluOperation::Test);
            x64_lock(plan.lock, Some(*destination), writes)?;
            if writes {
                writable_operand(memory, state, destination, plan.next_ip)?;
            }
            let result = alu(
                *operation,
                destination.width().bits(),
                read_operand(memory, state, destination, plan.next_ip)?,
                right,
                &mut state.flags,
            );
            if writes {
                write_operand(memory, state, destination, plan.next_ip, result)?;
            }
            Ok(X64_ADVANCE)
        }
        X64PlanOperation::Branch {
            condition: selected,
            displacement,
        } => {
            x64_lock(plan.lock, None, false)?;
            let taken = match selected {
                None => true,
                Some(code) => condition(*code, &state.flags)?,
            };
            if taken {
                Ok(X64Flow::Branch {
                    target: canonical_address(plan.next_ip.wrapping_add(*displacement as u64))?,
                })
            } else {
                Ok(X64_ADVANCE)
            }
        }
        X64PlanOperation::Jump { target } | X64PlanOperation::Call { target } => {
            x64_lock(plan.lock, None, false)?;
            let target = canonical_address(plan_source(memory, state, target, plan.next_ip)?)?;
            if matches!(plan.operation, X64PlanOperation::Call { .. }) {
                let stack = state
                    .registers
                    .read(
                        crate::core::contracts::GuestRegister::Rsp,
                        GuestIntegerWidth::B64,
                        false,
                    )?
                    .wrapping_sub(8);
                let space = memory.address_space();
                memory.write_u64(guest_address(space, stack, GuestAccess::Write)?, plan.next_ip)?;
                state.registers.write(
                    crate::core::contracts::GuestRegister::Rsp,
                    GuestIntegerWidth::B64,
                    stack,
                    false,
                )?;
            }
            Ok(X64Flow::Branch { target })
        }
        X64PlanOperation::Push { source, width } => {
            x64_lock(plan.lock, None, false)?;
            let value = plan_source(memory, state, source, plan.next_ip)?;
            let stack = state
                .registers
                .read(
                    crate::core::contracts::GuestRegister::Rsp,
                    GuestIntegerWidth::B64,
                    false,
                )?
                .wrapping_sub(width.bytes() as u64);
            let space = memory.address_space();
            let address = guest_address(space, stack, GuestAccess::Write)?;
            if *width == GuestIntegerWidth::B64 {
                memory.write_u64(address, value)?;
            } else {
                memory.write_u16(address, value as u16)?;
            }
            state.registers.write(
                crate::core::contracts::GuestRegister::Rsp,
                GuestIntegerWidth::B64,
                stack,
                false,
            )?;
            Ok(X64_ADVANCE)
        }
        X64PlanOperation::Pop { destination, width } => {
            x64_lock(plan.lock, None, false)?;
            let stack = state.registers.read(
                crate::core::contracts::GuestRegister::Rsp,
                GuestIntegerWidth::B64,
                false,
            )?;
            let space = memory.address_space();
            let address = guest_address(space, stack, GuestAccess::Read)?;
            let value = if *width == GuestIntegerWidth::B64 {
                memory.read_u64(address)?
            } else {
                u64::from(memory.read_u16(address)?)
            };
            state.registers.write(
                crate::core::contracts::GuestRegister::Rsp,
                GuestIntegerWidth::B64,
                stack.wrapping_add(width.bytes() as u64),
                false,
            )?;
            write_operand(memory, state, destination, plan.next_ip, value)?;
            Ok(X64_ADVANCE)
        }
        X64PlanOperation::Return { discard } => {
            x64_lock(plan.lock, None, false)?;
            let stack = state.registers.read(
                crate::core::contracts::GuestRegister::Rsp,
                GuestIntegerWidth::B64,
                false,
            )?;
            let space = memory.address_space();
            let target = memory.read_u64(guest_address(space, stack, GuestAccess::Read)?)?;
            state.registers.write(
                crate::core::contracts::GuestRegister::Rsp,
                GuestIntegerWidth::B64,
                stack.wrapping_add(8),
                false,
            )?;
            let branch = canonical_address(target)?;
            let stack = state.registers.read(
                crate::core::contracts::GuestRegister::Rsp,
                GuestIntegerWidth::B64,
                false,
            )?;
            state.registers.write(
                crate::core::contracts::GuestRegister::Rsp,
                GuestIntegerWidth::B64,
                stack.wrapping_add(*discard),
                false,
            )?;
            Ok(X64Flow::Branch { target: branch })
        }
    }
}

fn run_numeric(
    memory: &mut SparseGuestMemory,
    state: &mut GuestProcessorState,
    instruction: NumericInstruction,
) -> Result<X64Flow, X86Error> {
    match execute_numeric_instruction(NumericExecutionContext {
        state,
        memory,
        instruction,
    })? {
        NumericExecutionResult::Executed => Ok(X64_ADVANCE),
        NumericExecutionResult::Exception { vector, detail } => Err(X86Error::fault(u32::from(vector), detail)),
        NumericExecutionResult::Unsupported { detail } => Err(X86Error::unsupported(detail)),
    }
}
