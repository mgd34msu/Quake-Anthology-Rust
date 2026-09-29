//! x86-64 integer kernel: prepared plans and block execution.
//!
//! Donor: `src/guest/x64/integer-kernel.ts` (`X64IntegerKernel`). Static
//! lowering reduces register-only operations to direct slot access; block
//! execution runs up to 16 steps with rollback boundaries for faulting
//! stores and stops while admitted code remains unchanged. The donor's
//! `DataView` word games collapse to native `u64` slot access with
//! identical flag semantics.

use crate::core::contracts::{GuestAccess, GuestIntegerWidth, GuestRegister};
use crate::core::memory::{ExecutableBlock, SparseGuestMemory};
use crate::core::registers::GuestProcessorState;
use crate::x64::decoder::{
    canonical_address, guest_address, operand_address, read_memory, write_memory, X64MemoryOperand,
    X64Operand, X64RegisterOperand,
};
use crate::x64::plan::{
    execute_x64_plan, x64_lock, NumericInstructionBase, X64Flow, X64PlanOperation, X64PlanSource,
    X64SemanticPlan, X64SseOperand, X64ShiftCount, X64_ADVANCE,
};
use crate::x86::arithmetic::{alu, condition, AluOperation, ShiftOperation};
use crate::x86::decoder::X86Error;
use crate::floating_point::raw_sse::RawSseOperation;

/// Kernel operand: register slot or unresolved memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelOperand {
    /// Register slot.
    Register {
        /// Register.
        register: GuestRegister,
        /// Operand width.
        width: GuestIntegerWidth,
        /// High-byte alias.
        high_byte: bool,
    },
    /// Memory form.
    Memory {
        /// Unresolved memory operand.
        source: X64MemoryOperand,
        /// Operand width.
        width: GuestIntegerWidth,
    },
}

impl KernelOperand {
    /// Operand width.
    #[must_use]
    pub const fn width(self) -> GuestIntegerWidth {
        match self {
            Self::Register { width, .. } | Self::Memory { width, .. } => width,
        }
    }
}

/// Kernel source: operand or immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelSource {
    /// Operand source.
    Operand(KernelOperand),
    /// Immediate source.
    Immediate(u64),
}

/// Kernel control target: operand or canonicalized constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlTarget {
    /// Operand target.
    Operand(KernelOperand),
    /// Constant target.
    Constant {
        /// Raw value.
        value: u64,
        /// Already canonical.
        canonical: bool,
    },
}

/// Register fast-path source: register or immediate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordSource {
    /// Source register, or `None` for an immediate.
    pub register: Option<GuestRegister>,
    /// Immediate value.
    pub immediate: u64,
}

/// Register fast-path destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordDestination {
    /// Destination register.
    pub register: GuestRegister,
    /// Destination width (32 or 64).
    pub width: GuestIntegerWidth,
}

/// Lowered effective address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveAddress {
    /// Base register, if any.
    pub base: Option<GuestRegister>,
    /// Index register, if any.
    pub index: Option<GuestRegister>,
    /// Index shift (0-3).
    pub shift: u32,
    /// Displacement plus RIP base, if relative.
    pub displacement: u64,
    /// Address width in bits.
    pub address_bits: u32,
}

/// Prepared kernel operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelOperation {
    /// No operation.
    Nop,
    /// Move source to destination.
    Move {
        /// Destination.
        destination: KernelOperand,
        /// Source.
        source: KernelSource,
    },
    /// Register-to-register/immediate move (32/64-bit).
    RegisterMove {
        /// Destination.
        destination: WordDestination,
        /// Source.
        source: WordSource,
    },
    /// Register ALU (32/64-bit).
    RegisterAlu {
        /// Destination.
        destination: WordDestination,
        /// Source.
        source: WordSource,
        /// ALU operation.
        operation: AluOperation,
    },
    /// Extend source into a register.
    Extend {
        /// Destination register.
        destination: KernelOperand,
        /// Source.
        source: KernelOperand,
        /// Sign extension.
        signed: bool,
    },
    /// Increment or decrement in place.
    Increment {
        /// Destination.
        destination: KernelOperand,
        /// Decrement.
        subtract: bool,
    },
    /// Conditional move.
    ConditionalMove {
        /// Destination register.
        destination: KernelOperand,
        /// Source.
        source: KernelOperand,
        /// Condition code.
        condition: u8,
    },
    /// Set byte from a condition.
    SetCondition {
        /// Destination.
        destination: KernelOperand,
        /// Condition code.
        condition: u8,
    },
    /// Load effective address.
    Lea {
        /// Destination register.
        destination: KernelOperand,
        /// Lowered address.
        source: EffectiveAddress,
    },
    /// ALU operation.
    Alu {
        /// ALU operation.
        operation: AluOperation,
        /// Destination.
        destination: KernelOperand,
        /// Source.
        source: KernelSource,
    },
    /// Relative branch to a constant target.
    Branch {
        /// Condition code, or `None`.
        condition: Option<u8>,
        /// Constant target.
        target: ControlTarget,
    },
    /// Absolute indirect jump.
    Jump {
        /// Target.
        target: ControlTarget,
    },
    /// Absolute call.
    Call {
        /// Target.
        target: ControlTarget,
    },
    /// Push source.
    Push {
        /// Source.
        source: KernelSource,
        /// Stack width.
        width: GuestIntegerWidth,
    },
    /// Pop into destination.
    Pop {
        /// Destination.
        destination: KernelOperand,
        /// Stack width.
        width: GuestIntegerWidth,
    },
    /// Return, discarding argument bytes.
    Return {
        /// Argument bytes to discard.
        discard: u64,
    },
    /// Shift/rotate (delegated to the semantic executor).
    Shift {
        /// Destination.
        destination: X64Operand,
        /// Shift operation.
        operation: ShiftOperation,
        /// Count.
        count: X64ShiftCount,
    },
    /// Signed multiply (delegated).
    Multiply {
        /// Destination register.
        destination: X64RegisterOperand,
        /// Left operand.
        left: X64Operand,
        /// Right source.
        right: X64PlanSource,
    },
    /// Numeric instruction (delegated).
    Numeric {
        /// Decoded numeric instruction.
        instruction: crate::floating_point::contracts::NumericInstruction,
    },
    /// Numeric instruction with memory (delegated).
    NumericMemory {
        /// Base instruction.
        instruction: NumericInstructionBase,
        /// Memory operand.
        operand: X64MemoryOperand,
    },
    /// Raw SSE operation (delegated).
    RawSse {
        /// Qualified operation.
        operation: RawSseOperation,
        /// Operand.
        operand: X64SseOperand,
    },
}

/// Prepared plan: kernel operation plus its semantic original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct X64IntegerPlan {
    /// Prepared operation.
    pub operation: KernelOperation,
    /// Semantic original (next IP, LOCK, executor fallback).
    pub original: X64SemanticPlan,
}

/// One block step: start address, plan, and rollback boundary flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct X64IntegerStep {
    /// Step start IP.
    pub start: u64,
    /// Prepared plan.
    pub integer: X64IntegerPlan,
    /// Finishes every possible faulting read/check before touching
    /// architectural state.
    pub safe: bool,
}

/// Block execution outcome.
#[derive(Debug)]
pub enum X64IntegerBlockResult {
    /// Block completed.
    Complete {
        /// Retired instructions.
        instructions: u64,
    },
    /// Returned through the return address.
    Return {
        /// Retired instructions.
        instructions: u64,
    },
    /// Faulted mid-block with IP at the faulting step start.
    Fault {
        /// Retired instructions.
        instructions: u64,
        /// Fault.
        error: X86Error,
    },
}

/// Whether a plan finishes every possible faulting read/check before
/// touching architectural state.
#[must_use]
pub fn x64_integer_block_safe(plan: &X64IntegerPlan) -> bool {
    match &plan.operation {
        KernelOperation::Nop
        | KernelOperation::Extend { .. }
        | KernelOperation::RegisterMove { .. }
        | KernelOperation::RegisterAlu { .. }
        | KernelOperation::ConditionalMove { .. } => true,
        KernelOperation::SetCondition { destination, .. } => {
            matches!(destination, KernelOperand::Register { .. })
        }
        KernelOperation::Shift { .. } => {
            // Shift destinations lower through the semantic executor, which
            // pre-checks memory writes; only register forms skip rollback.
            matches!(
                &plan.original.operation,
                X64PlanOperation::Shift { destination, .. }
                    if matches!(destination, X64Operand::Register(_))
            )
        }
        KernelOperation::Multiply { .. } => true,
        KernelOperation::Numeric { instruction } => {
            numeric_safe(instruction.opcode, instruction.secondary_opcode)
        }
        KernelOperation::NumericMemory { instruction, .. } => {
            numeric_safe(instruction.opcode, instruction.secondary_opcode)
        }
        KernelOperation::Increment { destination, .. } => {
            matches!(destination, KernelOperand::Register { .. })
        }
        KernelOperation::RawSse { operation, operand } => {
            !matches!(operation, RawSseOperation::Move { store: true, .. })
                || !matches!(operand, X64SseOperand::Memory(_))
        }
        KernelOperation::Move { destination, .. } => {
            matches!(destination, KernelOperand::Register { .. })
        }
        KernelOperation::Lea { .. }
        | KernelOperation::Branch { .. }
        | KernelOperation::Return { .. }
        | KernelOperation::Jump { .. } => true,
        KernelOperation::Call { .. } | KernelOperation::Push { .. } => false,
        KernelOperation::Pop { destination, .. } => {
            matches!(destination, KernelOperand::Register { .. })
        }
        KernelOperation::Alu {
            operation,
            destination,
            ..
        } => {
            matches!(destination, KernelOperand::Register { .. })
                || matches!(operation, AluOperation::Cmp | AluOperation::Test)
        }
    }
}

fn numeric_safe(opcode: u8, secondary: Option<u8>) -> bool {
    opcode == 0x0f
        && secondary.is_some_and(|op| {
            matches!(
                op,
                0x2a | 0x2c | 0x2d | 0x2e | 0x2f | 0x50 | 0x51 | 0x58 | 0x59 | 0x5a | 0x5b | 0x5c
                    | 0x5d | 0x5e | 0x5f | 0xc2 | 0xe6
            )
        })
}

fn kernel_operand(value: X64Operand) -> KernelOperand {
    match value {
        X64Operand::Register(operand) => KernelOperand::Register {
            register: operand.register,
            width: operand.width,
            high_byte: operand.high_byte,
        },
        X64Operand::Memory(operand) => KernelOperand::Memory {
            source: operand,
            width: operand.width,
        },
    }
}

fn kernel_source(value: &X64PlanSource) -> KernelSource {
    match value {
        X64PlanSource::Operand(operand) => KernelSource::Operand(kernel_operand(*operand)),
        X64PlanSource::Immediate(value) => KernelSource::Immediate(*value),
    }
}

fn constant_target(value: u64) -> ControlTarget {
    ControlTarget::Constant {
        value,
        canonical: value <= crate::x64::decoder::CANONICAL_LOW_MAX
            || value >= crate::x64::decoder::CANONICAL_HIGH_MIN,
    }
}

fn register_operation(
    destination: KernelOperand,
    source: KernelSource,
) -> Option<(WordDestination, WordSource)> {
    let KernelOperand::Register {
        register,
        width,
        high_byte: _,
    } = destination
    else {
        return None;
    };
    if width != GuestIntegerWidth::B32 && width != GuestIntegerWidth::B64 {
        return None;
    }
    match source {
        KernelSource::Operand(KernelOperand::Memory { .. }) => None,
        KernelSource::Operand(KernelOperand::Register {
            width: source_width,
            ..
        }) if source_width != width => None,
        KernelSource::Operand(KernelOperand::Register {
            register: source_register,
            ..
        }) => Some((
            WordDestination { register, width },
            WordSource {
                register: Some(source_register),
                immediate: 0,
            },
        )),
        KernelSource::Immediate(value) => Some((
            WordDestination { register, width },
            WordSource {
                register: None,
                immediate: value,
            },
        )),
    }
}

/// Lower a semantic plan to a kernel plan. Preparation only reduces static
/// instruction fields; no guest data executes here.
#[must_use]
pub fn prepare_x64_integer_plan(plan: &X64SemanticPlan) -> Option<X64IntegerPlan> {
    let operation = match &plan.operation {
        X64PlanOperation::Nop => KernelOperation::Nop,
        X64PlanOperation::RawSse { operation, operand } => KernelOperation::RawSse {
            operation: *operation,
            operand: *operand,
        },
        X64PlanOperation::Numeric { instruction } => KernelOperation::Numeric {
            instruction: *instruction,
        },
        X64PlanOperation::NumericMemory {
            instruction,
            operand,
        } => KernelOperation::NumericMemory {
            instruction: *instruction,
            operand: *operand,
        },
        X64PlanOperation::Shift {
            destination,
            operation,
            count,
        } => KernelOperation::Shift {
            destination: *destination,
            operation: *operation,
            count: *count,
        },
        X64PlanOperation::Multiply {
            destination,
            left,
            right,
        } => KernelOperation::Multiply {
            destination: *destination,
            left: *left,
            right: right.clone(),
        },
        X64PlanOperation::ConditionalMove {
            destination,
            source,
            condition: code,
        } => KernelOperation::ConditionalMove {
            destination: kernel_operand(X64Operand::Register(*destination)),
            source: kernel_operand(*source),
            condition: *code,
        },
        X64PlanOperation::SetCondition {
            destination,
            condition,
        } => KernelOperation::SetCondition {
            destination: kernel_operand(*destination),
            condition: *condition,
        },
        X64PlanOperation::Extend {
            destination,
            source,
            signed,
        } => {
            let destination = kernel_operand(X64Operand::Register(*destination));
            if !matches!(destination, KernelOperand::Register { .. }) {
                return None;
            }
            KernelOperation::Extend {
                destination,
                source: kernel_operand(*source),
                signed: *signed,
            }
        }
        X64PlanOperation::Increment {
            destination,
            subtract,
        } => KernelOperation::Increment {
            destination: kernel_operand(*destination),
            subtract: *subtract,
        },
        X64PlanOperation::Move {
            destination,
            source,
        } => {
            let destination = kernel_operand(*destination);
            let input = kernel_source(source);
            match register_operation(destination, input) {
                Some((destination, source)) => KernelOperation::RegisterMove {
                    destination,
                    source,
                },
                None => KernelOperation::Move {
                    destination,
                    source: input,
                },
            }
        }
        X64PlanOperation::Alu {
            operation,
            destination,
            source,
        } => {
            let destination = kernel_operand(*destination);
            let input = kernel_source(source);
            match register_operation(destination, input) {
                Some((destination, source)) => KernelOperation::RegisterAlu {
                    destination,
                    source,
                    operation: *operation,
                },
                None => KernelOperation::Alu {
                    operation: *operation,
                    destination,
                    source: input,
                },
            }
        }
        X64PlanOperation::Branch {
            condition: code,
            displacement,
        } => KernelOperation::Branch {
            condition: *code,
            target: constant_target(plan.next_ip.wrapping_add(*displacement as u64)),
        },
        X64PlanOperation::Jump { target } | X64PlanOperation::Call { target } => {
            let target = match target {
                X64PlanSource::Immediate(value) => constant_target(*value),
                X64PlanSource::Operand(operand) => {
                    ControlTarget::Operand(kernel_operand(*operand))
                }
            };
            if matches!(plan.operation, X64PlanOperation::Jump { .. }) {
                KernelOperation::Jump { target }
            } else {
                KernelOperation::Call { target }
            }
        }
        X64PlanOperation::Push { source, width } => KernelOperation::Push {
            source: kernel_source(source),
            width: *width,
        },
        X64PlanOperation::Pop { destination, width } => KernelOperation::Pop {
            destination: kernel_operand(*destination),
            width: *width,
        },
        X64PlanOperation::Return { discard } => KernelOperation::Return { discard: *discard },
        X64PlanOperation::Lea {
            destination,
            source,
        } => {
            let destination = kernel_operand(X64Operand::Register(*destination));
            if !matches!(destination, KernelOperand::Register { .. }) {
                return None;
            }
            let shift = match source.scale {
                1 => 0,
                2 => 1,
                4 => 2,
                8 => 3,
                _ => return None,
            };
            let displacement = (source.displacement as u64)
                .wrapping_add(if source.rip_relative { plan.next_ip } else { 0 });
            KernelOperation::Lea {
                destination,
                source: EffectiveAddress {
                    base: if source.rip_relative {
                        None
                    } else {
                        source.base
                    },
                    index: source.index,
                    shift,
                    displacement,
                    address_bits: source.address_bits,
                },
            }
        }
    };
    Some(X64IntegerPlan {
        operation,
        original: plan.clone(),
    })
}

fn mask(width: GuestIntegerWidth) -> u64 {
    match width {
        GuestIntegerWidth::B8 => 0xff,
        GuestIntegerWidth::B16 => 0xffff,
        GuestIntegerWidth::B32 => 0xffff_ffff,
        GuestIntegerWidth::B64 => u64::MAX,
    }
}

/// One kernel belongs to one run invocation.
#[derive(Debug, Default)]
pub struct X64IntegerKernel {
    value: u64,
    checkpoint: Vec<u8>,
}

impl X64IntegerKernel {
    /// Fresh kernel with scratch state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Execute a block of steps. Stores retain rollback boundaries and the
    /// block continues only while admitted code remains unchanged.
    pub fn execute_block(
        &mut self,
        steps: &[X64IntegerStep],
        budget: u64,
        returned: Option<u64>,
        guard: Option<&ExecutableBlock>,
        state: &mut GuestProcessorState,
        memory: &mut SparseGuestMemory,
    ) -> X64IntegerBlockResult {
        let mut instructions = 0;
        let mut current = state.instruction_pointer;
        let mut next = current;
        let after_store = |memory: &SparseGuestMemory| -> bool {
            guard.map_or(true, |guard| guard.after_store(memory))
        };
        for step in steps {
            if instructions == budget {
                break;
            }
            current = step.start;
            if Some(current) == returned {
                state.instruction_pointer = current;
                return X64IntegerBlockResult::Return { instructions };
            }
            let operation = &step.integer.operation;
            let original = &step.integer.original;
            next = original.next_ip;
            if !step.safe {
                state.instruction_pointer = current;
                let observed = memory.has_write_observers();
                let needs_checkpoint = observed
                    || matches!(
                        operation,
                        KernelOperation::Pop { .. }
                            | KernelOperation::Numeric { .. }
                            | KernelOperation::NumericMemory { .. }
                            | KernelOperation::Shift { .. }
                            | KernelOperation::Multiply { .. }
                    );
                if !needs_checkpoint {
                    match self.execute(&step.integer, state, memory) {
                        Ok(flow) => {
                            instructions += 1;
                            if let X64Flow::Branch { target } = flow {
                                next = target;
                                break;
                            }
                            if !after_store(memory) {
                                break;
                            }
                            continue;
                        }
                        Err(error) => {
                            state.instruction_pointer = current;
                            return X64IntegerBlockResult::Fault {
                                instructions,
                                error,
                            };
                        }
                    }
                }
                let registers = state.registers.checkpoint();
                self.checkpoint = registers.clone();
                let flags = state.flags.value();
                match self.execute(&step.integer, state, memory) {
                    Ok(flow) => {
                        if let X64Flow::Branch { target } = flow {
                            next = target;
                            instructions += 1;
                            break;
                        }
                    }
                    Err(error) => {
                        state.registers.restore(&registers).ok();
                        state.flags.set_value(flags);
                        state.instruction_pointer = current;
                        return X64IntegerBlockResult::Fault {
                            instructions,
                            error,
                        };
                    }
                }
                instructions += 1;
                if observed || !after_store(memory) {
                    break;
                }
                continue;
            }
            match self.execute_safe(operation, original, state, memory) {
                Ok(SafeFlow::Next) => {}
                Ok(SafeFlow::Goto(target)) => {
                    next = target;
                }
                Err(error) => {
                    state.instruction_pointer = current;
                    return X64IntegerBlockResult::Fault {
                        instructions,
                        error,
                    };
                }
            }
            instructions += 1;
        }
        state.instruction_pointer = next;
        X64IntegerBlockResult::Complete { instructions }
    }

    /// Execute one prepared plan.
    pub fn execute(
        &mut self,
        plan: &X64IntegerPlan,
        state: &mut GuestProcessorState,
        memory: &mut SparseGuestMemory,
    ) -> Result<X64Flow, X86Error> {
        let original = &plan.original;
        match &plan.operation {
            KernelOperation::RegisterMove {
                destination,
                source,
            } => {
                x64_lock(original.lock, None, false)?;
                self.register_move(destination, source, state)?;
                Ok(X64_ADVANCE)
            }
            KernelOperation::RegisterAlu {
                destination,
                source,
                operation,
            } => {
                self.register_alu(*operation, destination, source, original.lock, state)?;
                Ok(X64_ADVANCE)
            }
            KernelOperation::ConditionalMove {
                destination,
                source,
                condition: code,
            } => {
                x64_lock(original.lock, None, false)?;
                self.conditional_move(destination, source, *code, original, state, memory)?;
                Ok(X64_ADVANCE)
            }
            KernelOperation::SetCondition {
                destination,
                condition: code,
            } => {
                x64_lock(original.lock, None, false)?;
                self.value = u64::from(condition(*code, &state.flags)?);
                self.write_operand(destination, original.next_ip, state, memory)?;
                Ok(X64_ADVANCE)
            }
            KernelOperation::Shift { .. }
            | KernelOperation::Multiply { .. }
            | KernelOperation::Numeric { .. }
            | KernelOperation::NumericMemory { .. }
            | KernelOperation::RawSse { .. } => execute_x64_plan(original, memory, state),
            KernelOperation::Nop => {
                x64_lock(original.lock, None, false)?;
                Ok(X64_ADVANCE)
            }
            KernelOperation::Extend {
                destination,
                source,
                signed,
            } => {
                x64_lock(original.lock, None, false)?;
                self.extend(destination, source, *signed, original.next_ip, state, memory)?;
                Ok(X64_ADVANCE)
            }
            KernelOperation::Increment {
                destination,
                subtract,
            } => {
                let x64_destination = kernel_to_x64(*destination);
                x64_lock(original.lock, x64_destination, true)?;
                let address = self.precheck(destination, original.next_ip, true, state, memory)?;
                let carry = state.flags.get(crate::core::contracts::GuestFlag::Carry);
                match address {
                    Some(address) => self.read_memory(address, destination.width(), memory)?,
                    None => self.read_operand(destination, original.next_ip, state, memory)?,
                }
                let value = alu(
                    if *subtract {
                        AluOperation::Sub
                    } else {
                        AluOperation::Add
                    },
                    destination.width().bits(),
                    self.value,
                    1,
                    &mut state.flags,
                );
                self.value = value;
                match address {
                    Some(address) => self.write_memory(address, destination.width(), memory)?,
                    None => self.write_operand(destination, original.next_ip, state, memory)?,
                }
                state.flags.set(crate::core::contracts::GuestFlag::Carry, carry);
                Ok(X64_ADVANCE)
            }
            KernelOperation::Move {
                destination,
                source,
            } => {
                x64_lock(original.lock, None, false)?;
                self.read_source(source, original.next_ip, state, memory)?;
                self.write_operand(destination, original.next_ip, state, memory)?;
                Ok(X64_ADVANCE)
            }
            KernelOperation::Lea {
                destination,
                source,
            } => {
                x64_lock(original.lock, None, false)?;
                self.address(source, state)?;
                self.write_operand(destination, original.next_ip, state, memory)?;
                Ok(X64_ADVANCE)
            }
            KernelOperation::Alu {
                operation,
                destination,
                source,
            } => {
                // Source reads precede LOCK/write admission in the original
                // semantic handler.
                self.read_source(source, original.next_ip, state, memory)?;
                let right = self.value;
                let writes = !matches!(operation, AluOperation::Cmp | AluOperation::Test);
                let x64_destination = kernel_to_x64(*destination);
                x64_lock(original.lock, x64_destination, writes)?;
                let address = self.precheck(destination, original.next_ip, writes, state, memory)?;
                match address {
                    Some(address) => self.read_memory(address, destination.width(), memory)?,
                    None => self.read_operand(destination, original.next_ip, state, memory)?,
                }
                let value = alu(
                    *operation,
                    destination.width().bits(),
                    self.value,
                    right,
                    &mut state.flags,
                );
                self.value = value;
                if writes {
                    match address {
                        Some(address) => self.write_memory(address, destination.width(), memory)?,
                        None => self.write_operand(destination, original.next_ip, state, memory)?,
                    }
                }
                Ok(X64_ADVANCE)
            }
            KernelOperation::Branch { condition: selected, target } => {
                x64_lock(original.lock, None, false)?;
                if selected.map_or(Ok(true), |code| condition(code, &state.flags))? {
                    Ok(X64Flow::Branch {
                        target: self.target(target, original.next_ip, state, memory)?,
                    })
                } else {
                    Ok(X64_ADVANCE)
                }
            }
            KernelOperation::Return { discard } => {
                x64_lock(original.lock, None, false)?;
                Ok(X64Flow::Branch {
                    target: self.ret(*discard, state, memory)?,
                })
            }
            KernelOperation::Jump { target } | KernelOperation::Call { target } => {
                x64_lock(original.lock, None, false)?;
                let target = self.target(target, original.next_ip, state, memory)?;
                if matches!(plan.operation, KernelOperation::Call { .. }) {
                    let stack = state
                        .registers
                        .read(GuestRegister::Rsp, GuestIntegerWidth::B64, false)?
                        .wrapping_sub(8);
                    let space = memory.address_space();
                    memory.write_u64(
                        guest_address(space, stack, GuestAccess::Write)?,
                        original.next_ip,
                    )?;
                    state.registers.write(
                        GuestRegister::Rsp,
                        GuestIntegerWidth::B64,
                        stack,
                        false,
                    )?;
                }
                Ok(X64Flow::Branch { target })
            }
            KernelOperation::Push { source, width } => {
                x64_lock(original.lock, None, false)?;
                self.read_source(source, original.next_ip, state, memory)?;
                let stack = state
                    .registers
                    .read(GuestRegister::Rsp, GuestIntegerWidth::B64, false)?
                    .wrapping_sub(width.bytes() as u64);
                let space = memory.address_space();
                self.write_memory(
                    guest_address(space, stack, GuestAccess::Write)?,
                    *width,
                    memory,
                )?;
                state.registers.write(
                    GuestRegister::Rsp,
                    GuestIntegerWidth::B64,
                    stack,
                    false,
                )?;
                Ok(X64_ADVANCE)
            }
            KernelOperation::Pop { destination, width } => {
                x64_lock(original.lock, None, false)?;
                self.pop(*width, state, memory)?;
                self.write_operand(destination, original.next_ip, state, memory)?;
                Ok(X64_ADVANCE)
            }
        }
    }

    /// Execute one safe step inline: memory destinations are absent (or the
    /// operation is non-writing), so no rollback boundary is needed.
    fn execute_safe(
        &mut self,
        operation: &KernelOperation,
        original: &X64SemanticPlan,
        state: &mut GuestProcessorState,
        memory: &mut SparseGuestMemory,
    ) -> Result<SafeFlow, X86Error> {
        match operation {
            KernelOperation::RegisterMove {
                destination,
                source,
            } => {
                x64_lock(original.lock, None, false)?;
                self.register_move(destination, source, state)?;
            }
            KernelOperation::RegisterAlu {
                destination,
                source,
                operation,
            } => {
                self.register_alu(*operation, destination, source, original.lock, state)?;
            }
            KernelOperation::ConditionalMove {
                destination,
                source,
                condition: code,
            } => {
                self.conditional_move(destination, source, *code, original, state, memory)?;
            }
            KernelOperation::SetCondition {
                destination,
                condition: code,
            } => {
                x64_lock(original.lock, None, false)?;
                self.value = u64::from(condition(*code, &state.flags)?);
                self.write_operand(destination, original.next_ip, state, memory)?;
            }
            KernelOperation::Shift { .. }
            | KernelOperation::Multiply { .. }
            | KernelOperation::Numeric { .. }
            | KernelOperation::NumericMemory { .. }
            | KernelOperation::RawSse { .. } => {
                execute_x64_plan(original, memory, state)?;
            }
            KernelOperation::Nop => {
                x64_lock(original.lock, None, false)?;
            }
            KernelOperation::Extend {
                destination,
                source,
                signed,
            } => {
                x64_lock(original.lock, None, false)?;
                self.extend(destination, source, *signed, original.next_ip, state, memory)?;
            }
            KernelOperation::Increment {
                destination,
                subtract,
            } => {
                x64_lock(original.lock, kernel_to_x64(*destination), true)?;
                let carry = state.flags.get(crate::core::contracts::GuestFlag::Carry);
                self.read_operand(destination, original.next_ip, state, memory)?;
                let value = alu(
                    if *subtract {
                        AluOperation::Sub
                    } else {
                        AluOperation::Add
                    },
                    destination.width().bits(),
                    self.value,
                    1,
                    &mut state.flags,
                );
                self.value = value;
                self.write_operand(destination, original.next_ip, state, memory)?;
                state.flags.set(crate::core::contracts::GuestFlag::Carry, carry);
            }
            KernelOperation::Move {
                destination,
                source,
            } => {
                x64_lock(original.lock, None, false)?;
                self.read_source(source, original.next_ip, state, memory)?;
                self.write_operand(destination, original.next_ip, state, memory)?;
            }
            KernelOperation::Lea {
                destination,
                source,
            } => {
                x64_lock(original.lock, None, false)?;
                self.address(source, state)?;
                self.write_operand(destination, original.next_ip, state, memory)?;
            }
            KernelOperation::Alu {
                operation,
                destination,
                source,
            } => {
                self.read_source(source, original.next_ip, state, memory)?;
                let right = self.value;
                let writes = !matches!(operation, AluOperation::Cmp | AluOperation::Test);
                x64_lock(original.lock, kernel_to_x64(*destination), writes)?;
                self.read_operand(destination, original.next_ip, state, memory)?;
                let value = alu(
                    *operation,
                    destination.width().bits(),
                    self.value,
                    right,
                    &mut state.flags,
                );
                self.value = value;
                if writes {
                    self.write_operand(destination, original.next_ip, state, memory)?;
                }
            }
            KernelOperation::Branch { condition: selected, target } => {
                x64_lock(original.lock, None, false)?;
                if selected.map_or(Ok(true), |code| condition(code, &state.flags))? {
                    return Ok(SafeFlow::Goto(self.target(target, original.next_ip, state, memory)?));
                }
            }
            KernelOperation::Return { discard } => {
                x64_lock(original.lock, None, false)?;
                return Ok(SafeFlow::Goto(self.ret(*discard, state, memory)?));
            }
            KernelOperation::Jump { target } => {
                x64_lock(original.lock, None, false)?;
                return Ok(SafeFlow::Goto(self.target(target, original.next_ip, state, memory)?));
            }
            KernelOperation::Pop { destination, width } => {
                x64_lock(original.lock, None, false)?;
                self.pop(*width, state, memory)?;
                self.write_operand(destination, original.next_ip, state, memory)?;
            }
            KernelOperation::Push { .. } | KernelOperation::Call { .. } => {
                return Err(X86Error::unsupported(
                    "A memory-writing instruction cannot run in a read-only integer block",
                ));
            }
        }
        Ok(SafeFlow::Next)
    }

    fn precheck(
        &self,
        destination: &KernelOperand,
        next_ip: u64,
        write: bool,
        state: &GuestProcessorState,
        memory: &mut SparseGuestMemory,
    ) -> Result<Option<crate::core::contracts::GuestAddress>, X86Error> {
        let KernelOperand::Memory { source, width } = destination else {
            return Ok(None);
        };
        let access = if write {
            GuestAccess::Write
        } else {
            GuestAccess::Read
        };
        let address = operand_address(memory, state, source, next_ip, access)?;
        if write {
            memory.check(address, width.bytes(), GuestAccess::Write)?;
        }
        Ok(Some(address))
    }

    fn ret(
        &mut self,
        discard: u64,
        state: &mut GuestProcessorState,
        memory: &mut SparseGuestMemory,
    ) -> Result<u64, X86Error> {
        let stack = state.registers.read(GuestRegister::Rsp, GuestIntegerWidth::B64, false)?;
        let space = memory.address_space();
        let target = canonical_address(memory.read_u64(guest_address(
            space,
            stack,
            GuestAccess::Read,
        )?)?)?;
        state.registers.write(
            GuestRegister::Rsp,
            GuestIntegerWidth::B64,
            stack.wrapping_add(8).wrapping_add(discard),
            false,
        )?;
        Ok(target)
    }

    fn target(
        &mut self,
        target: &ControlTarget,
        next_ip: u64,
        state: &mut GuestProcessorState,
        memory: &mut SparseGuestMemory,
    ) -> Result<u64, X86Error> {
        match target {
            ControlTarget::Constant { value, canonical } => {
                if *canonical {
                    Ok(*value)
                } else {
                    canonical_address(*value)
                }
            }
            ControlTarget::Operand(operand) => {
                if operand.width() == GuestIntegerWidth::B64 {
                    match operand {
                        KernelOperand::Register { register, .. } => canonical_address(
                            state.registers.read(*register, GuestIntegerWidth::B64, false)?,
                        ),
                        KernelOperand::Memory { source, .. } => canonical_address(memory.read_u64(
                            operand_address(memory, state, source, next_ip, GuestAccess::Read)?,
                        )?),
                    }
                } else {
                    self.read_operand(operand, next_ip, state, memory)?;
                    Ok(self.value & 0xffff_ffff)
                }
            }
        }
    }

    fn extend(
        &mut self,
        destination: &KernelOperand,
        source: &KernelOperand,
        signed: bool,
        next_ip: u64,
        state: &mut GuestProcessorState,
        memory: &mut SparseGuestMemory,
    ) -> Result<(), X86Error> {
        self.read_operand(source, next_ip, state, memory)?;
        if signed {
            let bits = source.width().bits();
            self.value = ((((self.value & mask(source.width())) << (64 - bits)) as i64)
                >> (64 - bits)) as u64;
        }
        self.write_operand(destination, next_ip, state, memory)
    }

    fn conditional_move(
        &mut self,
        destination: &KernelOperand,
        source: &KernelOperand,
        code: u8,
        original: &X64SemanticPlan,
        state: &mut GuestProcessorState,
        memory: &mut SparseGuestMemory,
    ) -> Result<(), X86Error> {
        x64_lock(original.lock, None, false)?;
        self.read_operand(source, original.next_ip, state, memory)?;
        if condition(code, &state.flags)? {
            self.write_operand(destination, original.next_ip, state, memory)?;
        } else if destination.width() == GuestIntegerWidth::B32 {
            self.read_operand(destination, original.next_ip, state, memory)?;
            self.write_operand(destination, original.next_ip, state, memory)?;
        }
        Ok(())
    }

    fn pop(
        &mut self,
        width: GuestIntegerWidth,
        state: &mut GuestProcessorState,
        memory: &mut SparseGuestMemory,
    ) -> Result<(), X86Error> {
        let stack = state.registers.read(GuestRegister::Rsp, GuestIntegerWidth::B64, false)?;
        let space = memory.address_space();
        self.read_memory(
            guest_address(space, stack, GuestAccess::Read)?,
            width,
            memory,
        )?;
        state.registers.write(
            GuestRegister::Rsp,
            GuestIntegerWidth::B64,
            stack.wrapping_add(width.bytes() as u64),
            false,
        )?;
        Ok(())
    }

    fn read_source(
        &mut self,
        source: &KernelSource,
        next_ip: u64,
        state: &mut GuestProcessorState,
        memory: &mut SparseGuestMemory,
    ) -> Result<(), X86Error> {
        match source {
            KernelSource::Immediate(value) => {
                self.value = *value;
                Ok(())
            }
            KernelSource::Operand(operand) => self.read_operand(operand, next_ip, state, memory),
        }
    }

    fn read_operand(
        &mut self,
        operand: &KernelOperand,
        next_ip: u64,
        state: &mut GuestProcessorState,
        memory: &mut SparseGuestMemory,
    ) -> Result<(), X86Error> {
        match operand {
            KernelOperand::Memory { source, width } => {
                let address = operand_address(memory, state, source, next_ip, GuestAccess::Read)?;
                self.read_memory(address, *width, memory)
            }
            KernelOperand::Register {
                register,
                width,
                high_byte,
            } => {
                self.value = state.registers.read(*register, *width, *high_byte)?;
                Ok(())
            }
        }
    }

    fn write_operand(
        &mut self,
        destination: &KernelOperand,
        next_ip: u64,
        state: &mut GuestProcessorState,
        memory: &mut SparseGuestMemory,
    ) -> Result<(), X86Error> {
        match destination {
            KernelOperand::Memory { source, width } => {
                let address = operand_address(memory, state, source, next_ip, GuestAccess::Write)?;
                self.write_memory(address, *width, memory)
            }
            KernelOperand::Register {
                register,
                width,
                high_byte,
            } => {
                state.registers.write(*register, *width, self.value, *high_byte)?;
                Ok(())
            }
        }
    }

    fn register_move(
        &mut self,
        destination: &WordDestination,
        source: &WordSource,
        state: &mut GuestProcessorState,
    ) -> Result<(), X86Error> {
        let value = match source.register {
            Some(register) => state.registers.read(register, destination.width, false)?,
            None => source.immediate,
        };
        let value = if destination.width == GuestIntegerWidth::B32 {
            value & 0xffff_ffff
        } else {
            value
        };
        state.registers.write(destination.register, destination.width, value, false)?;
        Ok(())
    }

    fn register_alu(
        &mut self,
        operation: AluOperation,
        destination: &WordDestination,
        source: &WordSource,
        lock: bool,
        state: &mut GuestProcessorState,
    ) -> Result<(), X86Error> {
        let right = match source.register {
            Some(register) => state.registers.read(register, destination.width, false)?,
            None => source.immediate,
        };
        let right = if destination.width == GuestIntegerWidth::B32 {
            right & 0xffff_ffff
        } else {
            right
        };
        x64_lock(lock, None, false)?;
        let left = state.registers.read(destination.register, destination.width, false)?;
        let value = alu(
            operation,
            destination.width.bits(),
            left,
            right,
            &mut state.flags,
        );
        if !matches!(operation, AluOperation::Cmp | AluOperation::Test) {
            state.registers.write(destination.register, destination.width, value, false)?;
        }
        Ok(())
    }

    fn read_memory(
        &mut self,
        address: crate::core::contracts::GuestAddress,
        width: GuestIntegerWidth,
        memory: &mut SparseGuestMemory,
    ) -> Result<(), X86Error> {
        self.value = read_memory(memory, address, width)?;
        Ok(())
    }

    fn write_memory(
        &mut self,
        address: crate::core::contracts::GuestAddress,
        width: GuestIntegerWidth,
        memory: &mut SparseGuestMemory,
    ) -> Result<(), X86Error> {
        write_memory(memory, address, width, self.value)?;
        Ok(())
    }

    fn address(
        &mut self,
        address: &EffectiveAddress,
        state: &GuestProcessorState,
    ) -> Result<(), X86Error> {
        let base = match address.base {
            Some(base) => state.registers.read(
                base,
                if address.address_bits == 32 {
                    GuestIntegerWidth::B32
                } else {
                    GuestIntegerWidth::B64
                },
                false,
            )?,
            None => 0,
        };
        let index = match address.index {
            Some(index) => state.registers.read(
                index,
                if address.address_bits == 32 {
                    GuestIntegerWidth::B32
                } else {
                    GuestIntegerWidth::B64
                },
                false,
            )?,
            None => 0,
        };
        let offset = base
            .wrapping_add(index << address.shift)
            .wrapping_add(address.displacement);
        self.value = if address.address_bits == 32 {
            offset & 0xffff_ffff
        } else {
            offset
        };
        Ok(())
    }
}

enum SafeFlow {
    Next,
    Goto(u64),
}

fn kernel_to_x64(operand: KernelOperand) -> Option<X64Operand> {
    match operand {
        KernelOperand::Register {
            register,
            width,
            high_byte,
        } => Some(X64Operand::Register(X64RegisterOperand {
            register,
            width,
            high_byte,
        })),
        KernelOperand::Memory { source, .. } => Some(X64Operand::Memory(source)),
    }
}
