//! User-mode x86-64 interpreter with decode caching and block execution.
//!
//! Donor: `src/guest/x64/cpu.ts` (`X64Cpu`). Each instruction lowers to a
//! semantic plan executed directly or through the integer kernel; decoded
//! bytes are retained so cached plans revalidate against live memory, and
//! unhooked runs chain plans into semantic and integer blocks. Host
//! addresses stop before instruction fetch.

use std::collections::HashMap;
use std::rc::Rc;

use crate::abi::GuestCpu;
use crate::core::callbacks::HookState;
use crate::core::contracts::{
    GuestAccess, GuestAddress, GuestException, GuestExecutionStop, GuestFlag, GuestInstruction, GuestIntegerWidth,
    GuestRegister,
};
use crate::core::memory::{ExecutableBlock, SparseGuestMemory};
use crate::core::registers::GuestProcessorState;
use crate::error::GuestError;
use crate::floating_point::contracts::NumericOperand;
use crate::floating_point::raw_sse::prepare_raw_sse;
use crate::x64::decoder::{
    canonical_address, guest_address, read_memory, write_memory, X64DecodeCursor, X64Operand, X64Repeat, X64Segment,
};
use crate::x64::integer_kernel::{
    prepare_x64_integer_plan, x64_integer_block_safe, X64IntegerBlockResult, X64IntegerKernel, X64IntegerPlan,
    X64IntegerStep,
};
use crate::x64::plan::{
    execute_x64_plan, make_x64_plan, x64_lock, NumericInstructionBase, X64Flow, X64PlanOperation, X64PlanSource,
    X64SemanticPlan, X64ShiftCount, X64SseOperand, X64_ADVANCE,
};
use crate::x86::arithmetic::{
    alu, quotient_fits_signed, result_flags, sign_extend, sign_extend_double, AluOperation, ShiftOperation,
};
use crate::x86::decoder::X86Error;

/// Decode-cache capacity before a full clear.
const CACHE_CAPACITY: usize = 32768;
/// Maximum cached block length.
const BLOCK_LIMIT: usize = 16;

const ARITHMETIC: [AluOperation; 8] = [
    AluOperation::Add,
    AluOperation::Or,
    AluOperation::Adc,
    AluOperation::Sbb,
    AluOperation::And,
    AluOperation::Sub,
    AluOperation::Xor,
    AluOperation::Cmp,
];

const SHIFTS: [ShiftOperation; 8] = [
    ShiftOperation::Rol,
    ShiftOperation::Ror,
    ShiftOperation::Rcl,
    ShiftOperation::Rcr,
    ShiftOperation::Shl,
    ShiftOperation::Shr,
    ShiftOperation::Shl,
    ShiftOperation::Sar,
];

/// Cached instruction: retained decode plus lowered plans.
#[derive(Debug, Clone)]
struct CachedInstruction {
    start: u64,
    decoded: crate::x64::decoder::X64DecodedInstruction,
    plan: Option<X64SemanticPlan>,
    integer: Option<X64IntegerPlan>,
    block_key: Option<u64>,
    managed_key: Option<u64>,
    unhooked_revision: Option<u64>,
}

/// Cached semantic block: chained plan starts.
#[derive(Debug, Clone)]
struct SemanticBlock {
    revision: u64,
    starts: Vec<u64>,
}

/// Cached integer block: steps plus its liveness guard.
#[derive(Debug, Clone)]
struct ManagedBlock {
    revision: u64,
    steps: Vec<X64IntegerStep>,
    guard: ExecutableBlock,
}

fn mask(width: GuestIntegerWidth) -> u64 {
    match width {
        GuestIntegerWidth::B8 => 0xff,
        GuestIntegerWidth::B16 => 0xffff,
        GuestIntegerWidth::B32 => 0xffff_ffff,
        GuestIntegerWidth::B64 => u64::MAX,
    }
}

/// User-mode x86-64 interpreter.
pub struct X64Cpu {
    /// Processor state.
    pub state: GuestProcessorState,
    /// Guest memory.
    pub memory: SparseGuestMemory,
    hooks: Option<Rc<HookState>>,
    instructions: HashMap<u64, CachedInstruction>,
    blocks: HashMap<u64, SemanticBlock>,
    managed_blocks: HashMap<u64, ManagedBlock>,
}

impl X64Cpu {
    /// CPU over `state` and `memory`.
    pub fn new(state: GuestProcessorState, memory: SparseGuestMemory) -> Result<Self, GuestError> {
        if state.architecture != crate::core::contracts::GuestArchitecture::X86_64 || memory.pointer_bytes() != 8 {
            return Err(GuestError::cpu(
                "X64Cpu requires x86-64 processor state and 64-bit guest memory",
            ));
        }
        Ok(Self {
            state,
            memory,
            hooks: None,
            instructions: HashMap::new(),
            blocks: HashMap::new(),
            managed_blocks: HashMap::new(),
        })
    }

    /// Installed hook state.
    #[must_use]
    pub fn hooks(&self) -> Option<&Rc<HookState>> {
        self.hooks.as_ref()
    }

    fn evidence_address(&self, offset: u64) -> GuestAddress {
        GuestAddress::new(self.memory.address_space(), offset)
    }

    fn hook_revision(&self) -> u64 {
        self.hooks
            .as_ref()
            .map_or(0, |hooks| hooks.callbacks.borrow().entry_revision())
    }

    fn is_host_call(&mut self, address: GuestAddress) -> Result<bool, GuestError> {
        match &self.hooks {
            None => Ok(false),
            Some(hooks) => {
                let rsp = self
                    .state
                    .registers
                    .read(GuestRegister::Rsp, GuestIntegerWidth::B64, false)
                    .unwrap_or(0);
                hooks.entry_rsp.set(rsp);
                hooks.callbacks.borrow_mut().enter(&mut self.memory, address)
            }
        }
    }

    fn entry_unhooked(&mut self, start: u64, revision: u64) -> bool {
        let cached = match self.instructions.get(&start) {
            Some(cached) => cached,
            None => return false,
        };
        if cached.unhooked_revision == Some(revision) {
            return true;
        }
        let unhooked = self
            .hooks
            .as_ref()
            .map_or(true, |hooks| hooks.callbacks.borrow().instruction_unhooked(start));
        if !unhooked {
            return false;
        }
        if let Some(cached) = self.instructions.get_mut(&start) {
            cached.unhooked_revision = Some(revision);
        }
        true
    }

    fn form_block(&mut self, first: u64, revision: u64) -> Option<SemanticBlock> {
        if let Some(cached) = self.instructions.get(&first) {
            if let Some(key) = cached.block_key {
                if let Some(block) = self.blocks.get(&key) {
                    if block.revision == revision {
                        return Some(block.clone());
                    }
                }
            }
        }
        let mut starts = Vec::new();
        let mut current = Some(first);
        let mut complete = true;
        while let Some(start) = current {
            let Some(cached) = self.instructions.get(&start) else {
                complete = false;
                break;
            };
            let Some(plan) = cached.plan.clone() else {
                break;
            };
            if !self.entry_unhooked(start, revision) {
                break;
            }
            starts.push(start);
            if plan.ends_block || starts.len() == BLOCK_LIMIT {
                break;
            }
            current = Some(plan.next_ip);
            if !self.instructions.contains_key(&plan.next_ip) {
                complete = false;
                current = None;
            }
        }
        if starts.is_empty() {
            return None;
        }
        let block = SemanticBlock { revision, starts };
        if complete {
            self.blocks.insert(first, block.clone());
            if let Some(cached) = self.instructions.get_mut(&first) {
                cached.block_key = Some(first);
            }
        }
        Some(block)
    }

    fn form_managed_block(&mut self, first: u64, revision: u64) -> Option<ManagedBlock> {
        if let Some(cached) = self.instructions.get(&first) {
            if let Some(key) = cached.managed_key {
                if let Some(block) = self.managed_blocks.get(&key) {
                    if block.revision == revision {
                        return Some(block.clone());
                    }
                }
            }
            if cached.integer.is_none() {
                return None;
            }
        } else {
            return None;
        }
        let mut steps = Vec::new();
        let mut bytes = Vec::new();
        let mut current = Some(first);
        let mut complete = true;
        while let Some(start) = current {
            let cached = match self.instructions.get(&start) {
                Some(cached) => cached.clone(),
                None => {
                    complete = false;
                    break;
                }
            };
            let Some(integer) = cached.integer.clone() else {
                break;
            };
            if !self.entry_unhooked(start, revision) {
                break;
            }
            steps.push(X64IntegerStep {
                start,
                integer: integer.clone(),
                safe: x64_integer_block_safe(&integer),
            });
            bytes.extend_from_slice(&cached.decoded.bytes);
            let kind = &integer.operation;
            if matches!(
                kind,
                crate::x64::integer_kernel::KernelOperation::Branch { .. }
                    | crate::x64::integer_kernel::KernelOperation::Jump { .. }
                    | crate::x64::integer_kernel::KernelOperation::Call { .. }
                    | crate::x64::integer_kernel::KernelOperation::Return { .. }
            ) || steps.len() == BLOCK_LIMIT
            {
                break;
            }
            let next = integer.original.next_ip;
            if !self.instructions.contains_key(&next) {
                complete = false;
                current = None;
            } else {
                current = Some(next);
            }
        }
        if steps.is_empty() {
            return None;
        }
        let guard = self.memory.retain_executable_block(first, &bytes)?;
        let block = ManagedBlock { revision, steps, guard };
        if complete {
            self.managed_blocks.insert(first, block.clone());
            if let Some(cached) = self.instructions.get_mut(&first) {
                cached.managed_key = Some(first);
            }
        }
        Some(block)
    }

    fn push_stack(&mut self, value: u64, width: GuestIntegerWidth) -> Result<(), X86Error> {
        let next = self
            .state
            .registers
            .read(GuestRegister::Rsp, GuestIntegerWidth::B64, false)?
            .wrapping_sub(width.bytes() as u64);
        let space = self.memory.address_space();
        write_memory(
            &mut self.memory,
            guest_address(space, next, GuestAccess::Write)?,
            width,
            value,
        )?;
        self.state
            .registers
            .write(GuestRegister::Rsp, GuestIntegerWidth::B64, next, false)?;
        Ok(())
    }

    fn pop_stack(&mut self, width: GuestIntegerWidth) -> Result<u64, X86Error> {
        let stack = self
            .state
            .registers
            .read(GuestRegister::Rsp, GuestIntegerWidth::B64, false)?;
        let space = self.memory.address_space();
        let value = read_memory(&mut self.memory, guest_address(space, stack, GuestAccess::Read)?, width)?;
        self.state.registers.write(
            GuestRegister::Rsp,
            GuestIntegerWidth::B64,
            stack.wrapping_add(width.bytes() as u64),
            false,
        )?;
        Ok(value)
    }
}

impl GuestCpu for X64Cpu {
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
        let mut kernel = X64IntegerKernel::new();
        let mut instructions = 0;
        let mut block: Option<SemanticBlock> = None;
        let mut block_index = 0;
        while instructions < instruction_budget {
            let start = self.state.instruction_pointer;
            if return_address.map_or(false, |target| target.offset == start) {
                return GuestExecutionStop::Return {
                    instructions,
                    address: self.evidence_address(start),
                };
            }
            let revision = self.hook_revision();
            if block
                .as_ref()
                .is_none_or(|block| block.revision != revision || block.starts.get(block_index) != Some(&start))
            {
                block = None;
            }
            // Integer-block fast path over retained executable bytes.
            let retained_start = block
                .as_ref()
                .and_then(|block| block.starts.get(block_index).copied())
                .or(if self.instructions.contains_key(&start) {
                    Some(start)
                } else {
                    None
                });
            if let Some(first) = retained_start {
                let has_integer = self
                    .instructions
                    .get(&first)
                    .is_some_and(|cached| cached.integer.is_some());
                if has_integer {
                    let prepared = self
                        .instructions
                        .get(&first)
                        .and_then(|cached| cached.managed_key)
                        .and_then(|key| self.managed_blocks.get(&key).cloned())
                        .filter(|block| block.revision == revision)
                        .or_else(|| self.form_managed_block(first, revision));
                    if let Some(mut prepared) = prepared {
                        if !prepared.guard.unchanged(&mut self.memory) {
                            if let Some(cached) = self.instructions.get_mut(&first) {
                                cached.managed_key = None;
                            }
                            self.managed_blocks.remove(&first);
                        } else {
                            let steps = prepared.steps.clone();
                            let guard = prepared.guard.clone();
                            let result = kernel.execute_block(
                                &steps,
                                instruction_budget - instructions,
                                return_address.map(|target| target.offset),
                                Some(&guard),
                                &mut self.state,
                                &mut self.memory,
                            );
                            match result {
                                X64IntegerBlockResult::Return { instructions: retired } => {
                                    instructions += retired;
                                    return GuestExecutionStop::Return {
                                        instructions,
                                        address: self.evidence_address(self.state.instruction_pointer),
                                    };
                                }
                                X64IntegerBlockResult::Complete { instructions: retired } => {
                                    instructions += retired;
                                    block = None;
                                    continue;
                                }
                                X64IntegerBlockResult::Fault {
                                    instructions: retired,
                                    error,
                                } => {
                                    instructions += retired;
                                    return self.map_fault(
                                        error,
                                        instructions,
                                        self.evidence_address(self.state.instruction_pointer),
                                        Vec::new(),
                                        None,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            let registers = self.state.registers.checkpoint();
            let flags = self.state.flags.value();
            let mut cursor_bytes = Vec::new();
            let mut cursor_opcode = None;
            let mut prepared_bytes: Option<Vec<u8>> = None;
            let mut prepared_opcode: Option<u8> = None;
            enum Step {
                HostCall(GuestAddress),
                Executed {
                    flow: X64Flow,
                    next_ip: u64,
                    prepared_ends_block: bool,
                },
            }
            let step: Result<Step, X86Error> = (|| {
                canonical_address(start)?;
                if block.is_none() {
                    if let Some(cached) = self.instructions.get(&start) {
                        if cached.plan.is_some() {
                            block = self.form_block(start, revision);
                            block_index = 0;
                        }
                    }
                }
                let retained = block
                    .as_ref()
                    .and_then(|block| block.starts.get(block_index).copied())
                    .and_then(|at| self.instructions.get(&at))
                    .or_else(|| self.instructions.get(&start));
                let retained_start = retained.map(|cached| cached.start);
                if block.is_none() && retained_start.is_none_or(|at| !self.entry_unhooked(at, revision)) {
                    let address = self.evidence_address(start);
                    if self.is_host_call(address)? {
                        return Ok(Step::HostCall(address));
                    }
                }
                let retained = match retained_start {
                    Some(at) => self.instructions.get(&at),
                    None => None,
                };
                let reused = retained
                    .filter(|cached| {
                        self.state.instruction_pointer == start && cached.decoded.retention.unchanged(&self.memory)
                    })
                    .map(|cached| (cached.decoded.clone(), cached.plan.clone(), cached.integer.clone()));
                if reused.is_none() && block.is_some() {
                    if let Some(owner) = block.as_ref().and_then(|block| block.starts.first()) {
                        let key = *owner;
                        let invalidate = self.instructions.get(&key).and_then(|cached| cached.block_key)
                            == block.as_ref().and_then(|_| Some(key));
                        if invalidate {
                            if let Some(cached) = self.instructions.get_mut(&key) {
                                cached.block_key = None;
                            }
                            self.blocks.remove(&key);
                        }
                    }
                    block = None;
                }
                if let Some((decoded, Some(plan), integer)) = &reused {
                    if self.state.instruction_pointer == start {
                        prepared_bytes = Some(decoded.bytes.clone());
                        prepared_opcode = Some(decoded.opcode);
                        let next_ip = plan.next_ip;
                        let ends_block = plan.ends_block;
                        let flow = match integer {
                            Some(integer) => kernel.execute(&integer, &mut self.state, &mut self.memory)?,
                            None => execute_x64_plan(&plan, &mut self.memory, &mut self.state)?,
                        };
                        return Ok(Step::Executed {
                            flow,
                            next_ip,
                            prepared_ends_block: ends_block,
                        });
                    }
                }
                block = None;
                let reuse = match reused {
                    Some((decoded, _, _)) if self.state.instruction_pointer == start => Some(decoded),
                    _ => None,
                };
                let had_reuse = reuse.is_some();
                let mut cursor = X64DecodeCursor::new(&mut self.memory, &mut self.state, reuse)?;
                let flow = execute_cursor(&mut cursor);
                let next_ip = cursor.next_ip();
                cursor_opcode = Some(cursor.opcode());
                cursor_bytes = cursor.bytes().to_vec();
                let flow = flow?;
                if !had_reuse {
                    let prepared = cursor.cache();
                    if self.instructions.len() >= CACHE_CAPACITY {
                        self.instructions.clear();
                        self.blocks.clear();
                        self.managed_blocks.clear();
                    }
                    if prepared.is_none() || cursor.start() != start {
                        self.instructions.remove(&start);
                    } else if let Some(prepared) = prepared {
                        let integer = cursor.plan.as_ref().and_then(prepare_x64_integer_plan);
                        self.instructions.insert(
                            start,
                            CachedInstruction {
                                start,
                                decoded: prepared,
                                plan: cursor.plan.clone(),
                                integer,
                                block_key: None,
                                managed_key: None,
                                unhooked_revision: None,
                            },
                        );
                    }
                }
                Ok(Step::Executed {
                    flow,
                    next_ip,
                    prepared_ends_block: false,
                })
            })();
            match step {
                Ok(Step::HostCall(address)) => {
                    return GuestExecutionStop::HostCall { instructions, address };
                }
                Ok(Step::Executed {
                    flow,
                    next_ip,
                    prepared_ends_block,
                }) => {
                    self.state.instruction_pointer = match flow {
                        X64Flow::Branch { target } => target,
                        _ => next_ip,
                    };
                    if block.is_some() {
                        block_index += 1;
                        let len = block.as_ref().map_or(0, |block| block.starts.len());
                        if prepared_ends_block || block_index >= len {
                            block = None;
                        }
                    }
                    if flow == X64Flow::Halt {
                        return GuestExecutionStop::Halt {
                            instructions: instructions + 1,
                            address: self.evidence_address(start),
                        };
                    }
                    if let X64Flow::Trap { vector } = flow {
                        return GuestExecutionStop::Exception {
                            instructions: instructions + 1,
                            exception: GuestException::Processor {
                                vector,
                                error_code: None,
                                instruction: self.evidence_address(start),
                                detail: "Software breakpoint".to_string(),
                            },
                        };
                    }
                }
                Err(error) => {
                    self.state.registers.restore(&registers).ok();
                    self.state.flags.set_value(flags);
                    self.state.instruction_pointer = start;
                    let bytes = if cursor_bytes.is_empty() {
                        prepared_bytes.unwrap_or_default()
                    } else {
                        cursor_bytes
                    };
                    let opcode = cursor_opcode.or(prepared_opcode);
                    return self.map_fault(error, instructions, self.evidence_address(start), bytes, opcode);
                }
            }
            instructions += 1;
        }
        let address = self.evidence_address(self.state.instruction_pointer);
        if return_address.map_or(false, |target| target.offset == address.offset) {
            return GuestExecutionStop::Return { instructions, address };
        }
        GuestExecutionStop::Budget { instructions }
    }
}

impl X64Cpu {
    fn map_fault(
        &self,
        error: X86Error,
        instructions: u64,
        address: GuestAddress,
        bytes: Vec<u8>,
        opcode: Option<u8>,
    ) -> GuestExecutionStop {
        match error {
            X86Error::Memory {
                access,
                address: offset,
                byte_length,
                detail,
            } => GuestExecutionStop::Exception {
                instructions,
                exception: GuestException::Memory {
                    access,
                    address: self.evidence_address(offset),
                    byte_length,
                    detail,
                },
            },
            X86Error::Fault { vector, detail, .. } => GuestExecutionStop::Exception {
                instructions,
                exception: GuestException::Processor {
                    vector,
                    error_code: if vector == 13 { Some(0) } else { None },
                    instruction: address,
                    detail,
                },
            },
            X86Error::Unsupported(detail) => GuestExecutionStop::Unsupported {
                instructions,
                instruction: GuestInstruction {
                    address,
                    bytes,
                    mnemonic: format!(
                        "opcode {}",
                        opcode.map_or("unknown".to_string(), |op| format!("{op:x}"))
                    ),
                },
                detail,
            },
        }
    }
}

fn planned(cursor: &mut X64DecodeCursor, operation: X64PlanOperation) -> Result<X64Flow, X86Error> {
    let plan = make_x64_plan(operation, cursor.next_ip(), cursor.lock);
    let flow = execute_x64_plan(&plan, &mut *cursor.memory, &mut *cursor.state)?;
    cursor.plan = Some(plan);
    Ok(flow)
}

fn branch_to(target: u64) -> Result<X64Flow, X86Error> {
    Ok(X64Flow::Branch {
        target: canonical_address(target)?,
    })
}

#[allow(clippy::too_many_lines)]
fn execute_cursor(cursor: &mut X64DecodeCursor) -> Result<X64Flow, X86Error> {
    let op = cursor.opcode();
    let width = cursor.width();
    if op < 0x40 && (op & 7) <= 5 {
        let operation = ARITHMETIC[(op >> 3) as usize];
        let form = op & 7;
        let bits = if form & 1 == 0 { GuestIntegerWidth::B8 } else { width };
        if form < 4 {
            let decoded = cursor.decode_modrm(bits)?;
            let (destination, source) = if form < 2 {
                (decoded.rm, X64PlanSource::Operand(X64Operand::Register(decoded.reg)))
            } else {
                (X64Operand::Register(decoded.reg), X64PlanSource::Operand(decoded.rm))
            };
            return planned(
                cursor,
                X64PlanOperation::Alu {
                    operation,
                    destination,
                    source,
                },
            );
        }
        let destination = cursor.register(0, bits)?;
        let source = cursor.immediate(bits)?;
        return planned(
            cursor,
            X64PlanOperation::Alu {
                operation,
                destination: X64Operand::Register(destination),
                source: X64PlanSource::Immediate(source),
            },
        );
    }
    if (0x50..=0x57).contains(&op) {
        let stack_width = cursor.stack_width();
        let register = cursor.register(op as usize - 0x50 + cursor.rex_b(), stack_width)?;
        return planned(
            cursor,
            X64PlanOperation::Push {
                source: X64PlanSource::Operand(X64Operand::Register(register)),
                width: stack_width,
            },
        );
    }
    if (0x58..=0x5f).contains(&op) {
        let stack_width = cursor.stack_width();
        let register = cursor.register(op as usize - 0x58 + cursor.rex_b(), stack_width)?;
        return planned(
            cursor,
            X64PlanOperation::Pop {
                destination: X64Operand::Register(register),
                width: stack_width,
            },
        );
    }
    if (0x70..=0x7f).contains(&op) {
        x64_lock(cursor.lock, None, false)?;
        let displacement = cursor.read_signed(1)?;
        return planned(
            cursor,
            X64PlanOperation::Branch {
                condition: Some(op & 15),
                displacement,
            },
        );
    }
    if (0x90..=0x97).contains(&op) {
        x64_lock(cursor.lock, None, false)?;
        let index = op as usize - 0x90 + cursor.rex_b();
        if index == 0 {
            return planned(cursor, X64PlanOperation::Nop);
        }
        let other = cursor.register(index, width)?;
        let other = X64Operand::Register(other);
        let value = cursor.read(&other)?;
        let rax = cursor.state.registers.read(GuestRegister::Rax, width, false)?;
        cursor.write(&other, rax)?;
        cursor.state.registers.write(GuestRegister::Rax, width, value, false)?;
        return Ok(X64_ADVANCE);
    }
    if (0xb0..=0xbf).contains(&op) {
        x64_lock(cursor.lock, None, false)?;
        let bits = if op < 0xb8 { GuestIntegerWidth::B8 } else { width };
        let value = cursor.read_unsigned(bits.bytes())?;
        let destination = cursor.register((op & 7) as usize + cursor.rex_b(), bits)?;
        return planned(
            cursor,
            X64PlanOperation::Move {
                destination: X64Operand::Register(destination),
                source: X64PlanSource::Immediate(value),
            },
        );
    }
    if (0xd8..=0xdf).contains(&op) {
        numeric(cursor, None)?;
        return Ok(X64_ADVANCE);
    }
    match op {
        0x0f => extended(cursor),
        0x63 => {
            x64_lock(cursor.lock, None, false)?;
            let decoded = cursor.decode_modrm(if width == GuestIntegerWidth::B16 {
                GuestIntegerWidth::B16
            } else {
                GuestIntegerWidth::B32
            })?;
            let destination = cursor.register(decoded.register_index, width)?;
            planned(
                cursor,
                X64PlanOperation::Extend {
                    destination,
                    source: decoded.rm,
                    signed: true,
                },
            )
        }
        0x68 | 0x6a => {
            x64_lock(cursor.lock, None, false)?;
            let stack_width = cursor.stack_width();
            let length = if op == 0x6a {
                1
            } else if stack_width == GuestIntegerWidth::B16 {
                2
            } else {
                4
            };
            let value = cursor.read_signed(length)? as u64;
            planned(
                cursor,
                X64PlanOperation::Push {
                    source: X64PlanSource::Immediate(value),
                    width: stack_width,
                },
            )
        }
        0x69 | 0x6b => {
            x64_lock(cursor.lock, None, false)?;
            let decoded = cursor.decode_modrm(width)?;
            let immediate = if op == 0x6b {
                cursor.read_signed(1)? as u64
            } else {
                cursor.immediate(width)?
            };
            planned(
                cursor,
                X64PlanOperation::Multiply {
                    destination: decoded.reg,
                    left: decoded.rm,
                    right: X64PlanSource::Immediate(immediate),
                },
            )
        }
        0x80 | 0x81 | 0x83 => {
            let decoded = cursor.decode_modrm(if op == 0x80 { GuestIntegerWidth::B8 } else { width })?;
            let operation = ARITHMETIC[decoded.extension];
            let immediate = if op == 0x83 {
                cursor.read_signed(1)? as u64
            } else {
                cursor.immediate(decoded.rm.width())?
            };
            planned(
                cursor,
                X64PlanOperation::Alu {
                    operation,
                    destination: decoded.rm,
                    source: X64PlanSource::Immediate(immediate),
                },
            )
        }
        0x84 | 0x85 => {
            let decoded = cursor.decode_modrm(if op == 0x84 { GuestIntegerWidth::B8 } else { width })?;
            planned(
                cursor,
                X64PlanOperation::Alu {
                    operation: AluOperation::Test,
                    destination: decoded.rm,
                    source: X64PlanSource::Operand(X64Operand::Register(decoded.reg)),
                },
            )
        }
        0x86 | 0x87 => {
            let decoded = cursor.decode_modrm(if op == 0x86 { GuestIntegerWidth::B8 } else { width })?;
            x64_lock(cursor.lock, Some(decoded.rm), true)?;
            cursor.writable(&decoded.rm)?;
            let value = cursor.read(&decoded.rm)?;
            let replacement = cursor.read(&X64Operand::Register(decoded.reg))?;
            cursor.write(&decoded.rm, replacement)?;
            cursor.write(&X64Operand::Register(decoded.reg), value)?;
            Ok(X64_ADVANCE)
        }
        0x88..=0x8b => {
            x64_lock(cursor.lock, None, false)?;
            let decoded = cursor.decode_modrm(if op & 1 == 0 { GuestIntegerWidth::B8 } else { width })?;
            let (destination, source) = if op < 0x8a {
                (decoded.rm, X64PlanSource::Operand(X64Operand::Register(decoded.reg)))
            } else {
                (X64Operand::Register(decoded.reg), X64PlanSource::Operand(decoded.rm))
            };
            planned(cursor, X64PlanOperation::Move { destination, source })
        }
        0x8d => {
            x64_lock(cursor.lock, None, false)?;
            let decoded = cursor.decode_modrm(width)?;
            let X64Operand::Memory(source) = decoded.rm else {
                return Err(X86Error::fault(6, "LEA requires a memory addressing form"));
            };
            planned(
                cursor,
                X64PlanOperation::Lea {
                    destination: decoded.reg,
                    source,
                },
            )
        }
        0x8f => {
            x64_lock(cursor.lock, None, false)?;
            let stack_width = cursor.stack_width();
            let decoded = cursor.decode_modrm(stack_width)?;
            if decoded.extension != 0 {
                return Err(X86Error::unsupported(format!("POP/XOP group /{}", decoded.extension)));
            }
            planned(
                cursor,
                X64PlanOperation::Pop {
                    destination: decoded.rm,
                    width: stack_width,
                },
            )
        }
        0x98 => {
            x64_lock(cursor.lock, None, false)?;
            let source_width = match width {
                GuestIntegerWidth::B64 => GuestIntegerWidth::B32,
                GuestIntegerWidth::B32 => GuestIntegerWidth::B16,
                _ => GuestIntegerWidth::B8,
            };
            let value = cursor.state.registers.read(GuestRegister::Rax, source_width, false)?;
            let extended = ((((value & mask(source_width)) << (64 - source_width.bits())) as i64)
                >> (64 - source_width.bits())) as u64;
            cursor
                .state
                .registers
                .write(GuestRegister::Rax, width, extended, false)?;
            Ok(X64_ADVANCE)
        }
        0x99 => {
            x64_lock(cursor.lock, None, false)?;
            let value = cursor.state.registers.read(GuestRegister::Rax, width, false)? & mask(width);
            let negative = value & (1u64 << (width.bits() - 1)) != 0;
            cursor
                .state
                .registers
                .write(GuestRegister::Rdx, width, if negative { mask(width) } else { 0 }, false)?;
            Ok(X64_ADVANCE)
        }
        0x9b => {
            numeric(cursor, None)?;
            Ok(X64_ADVANCE)
        }
        0x9c => {
            x64_lock(cursor.lock, None, false)?;
            let stack_width = cursor.stack_width();
            let value = cursor.state.flags.value() & !0x30000;
            let next = cursor
                .state
                .registers
                .read(GuestRegister::Rsp, GuestIntegerWidth::B64, false)?
                .wrapping_sub(stack_width.bytes() as u64);
            let space = cursor.memory.address_space();
            write_memory(
                &mut *cursor.memory,
                guest_address(space, next, GuestAccess::Write)?,
                stack_width,
                value,
            )?;
            cursor
                .state
                .registers
                .write(GuestRegister::Rsp, GuestIntegerWidth::B64, next, false)?;
            Ok(X64_ADVANCE)
        }
        0x9d => {
            x64_lock(cursor.lock, None, false)?;
            let stack_width = cursor.stack_width();
            let stack = cursor
                .state
                .registers
                .read(GuestRegister::Rsp, GuestIntegerWidth::B64, false)?;
            let space = cursor.memory.address_space();
            let value = read_memory(
                &mut *cursor.memory,
                guest_address(space, stack, GuestAccess::Read)?,
                stack_width,
            )?;
            cursor.state.registers.write(
                GuestRegister::Rsp,
                GuestIntegerWidth::B64,
                stack.wrapping_add(stack_width.bytes() as u64),
                false,
            )?;
            let mask = if stack_width == GuestIntegerWidth::B16 {
                0x4dd5
            } else {
                0x244dd5
            };
            cursor
                .state
                .flags
                .set_value((cursor.state.flags.value() & !mask & !0x10000) | (value & mask) | 2);
            Ok(X64_ADVANCE)
        }
        0x9e => {
            x64_lock(cursor.lock, None, false)?;
            let value = cursor
                .state
                .registers
                .read(GuestRegister::Rax, GuestIntegerWidth::B8, true)?;
            cursor
                .state
                .flags
                .set_value((cursor.state.flags.value() & !0xd5) | (value & 0xd5) | 2);
            Ok(X64_ADVANCE)
        }
        0x9f => {
            x64_lock(cursor.lock, None, false)?;
            let value = (cursor.state.flags.value() & 0xd5) | 2;
            cursor
                .state
                .registers
                .write(GuestRegister::Rax, GuestIntegerWidth::B8, value, true)?;
            Ok(X64_ADVANCE)
        }
        0xa0..=0xa3 => {
            x64_lock(cursor.lock, None, false)?;
            let bits = if op & 1 == 0 { GuestIntegerWidth::B8 } else { width };
            let raw = cursor.read_unsigned(cursor.address_bits() as usize / 8)?;
            let segment_base = cursor.segment.map_or(0, |segment| {
                cursor.state.segments[match segment {
                    X64Segment::Fs => GuestProcessorState::FS,
                    X64Segment::Gs => GuestProcessorState::GS,
                }]
                .base
            });
            let space = cursor.memory.address_space();
            let address = guest_address(
                space,
                raw.wrapping_add(segment_base),
                if op < 0xa2 {
                    GuestAccess::Read
                } else {
                    GuestAccess::Write
                },
            )?;
            if op < 0xa2 {
                let value = read_memory(&mut *cursor.memory, address, bits)?;
                cursor.state.registers.write(GuestRegister::Rax, bits, value, false)?;
            } else {
                let value = cursor.state.registers.read(GuestRegister::Rax, bits, false)?;
                write_memory(&mut *cursor.memory, address, bits, value)?;
            }
            Ok(X64_ADVANCE)
        }
        0xa4 | 0xa5 | 0xa6 | 0xa7 | 0xaa | 0xab | 0xac | 0xad | 0xae | 0xaf => string_op(cursor),
        0xa8 | 0xa9 => {
            let bits = if op == 0xa8 { GuestIntegerWidth::B8 } else { width };
            let destination = cursor.register(0, bits)?;
            let source = cursor.immediate(bits)?;
            planned(
                cursor,
                X64PlanOperation::Alu {
                    operation: AluOperation::Test,
                    destination: X64Operand::Register(destination),
                    source: X64PlanSource::Immediate(source),
                },
            )
        }
        0xc0 | 0xc1 | 0xd0 | 0xd1 | 0xd2 | 0xd3 => {
            let bits = if op & 1 == 0 { GuestIntegerWidth::B8 } else { width };
            let decoded = cursor.decode_modrm(bits)?;
            let count = if op < 0xd0 {
                X64ShiftCount::Immediate(u32::from(cursor.read_byte()?))
            } else if op < 0xd2 {
                X64ShiftCount::Immediate(1)
            } else {
                X64ShiftCount::Cl
            };
            planned(
                cursor,
                X64PlanOperation::Shift {
                    destination: decoded.rm,
                    operation: SHIFTS[decoded.extension],
                    count,
                },
            )
        }
        0xc2 | 0xc3 => {
            x64_lock(cursor.lock, None, false)?;
            let discard = if op == 0xc2 { cursor.read_unsigned(2)? } else { 0 };
            planned(cursor, X64PlanOperation::Return { discard })
        }
        0xc6 | 0xc7 => {
            x64_lock(cursor.lock, None, false)?;
            let decoded = cursor.decode_modrm(if op == 0xc6 { GuestIntegerWidth::B8 } else { width })?;
            if decoded.extension != 0 {
                return Err(X86Error::unsupported(format!(
                    "MOV/transactional group /{}",
                    decoded.extension
                )));
            }
            let value = cursor.immediate(decoded.rm.width())?;
            planned(
                cursor,
                X64PlanOperation::Move {
                    destination: decoded.rm,
                    source: X64PlanSource::Immediate(value),
                },
            )
        }
        0xc9 => {
            x64_lock(cursor.lock, None, false)?;
            let rbp = cursor
                .state
                .registers
                .read(GuestRegister::Rbp, GuestIntegerWidth::B64, false)?;
            cursor
                .state
                .registers
                .write(GuestRegister::Rsp, GuestIntegerWidth::B64, rbp, false)?;
            let stack_width = cursor.stack_width();
            let stack = cursor
                .state
                .registers
                .read(GuestRegister::Rsp, GuestIntegerWidth::B64, false)?;
            let space = cursor.memory.address_space();
            let value = read_memory(
                &mut *cursor.memory,
                guest_address(space, stack, GuestAccess::Read)?,
                stack_width,
            )?;
            cursor.state.registers.write(
                GuestRegister::Rsp,
                GuestIntegerWidth::B64,
                stack.wrapping_add(stack_width.bytes() as u64),
                false,
            )?;
            cursor
                .state
                .registers
                .write(GuestRegister::Rbp, stack_width, value, false)?;
            Ok(X64_ADVANCE)
        }
        0xcc => {
            x64_lock(cursor.lock, None, false)?;
            Ok(X64Flow::Trap { vector: 3 })
        }
        0xcd => {
            x64_lock(cursor.lock, None, false)?;
            let vector = cursor.read_byte()?;
            Err(X86Error::unsupported(format!("Software interrupt 0x{vector:x}")))
        }
        0xe0..=0xe3 => {
            x64_lock(cursor.lock, None, false)?;
            let displacement = cursor.read_signed(1)?;
            let address_width = if cursor.address_bits() == 32 {
                GuestIntegerWidth::B32
            } else {
                GuestIntegerWidth::B64
            };
            let mut count = cursor.state.registers.read(GuestRegister::Rcx, address_width, false)?;
            if op != 0xe3 {
                count = count.wrapping_sub(1) & mask(address_width);
                cursor
                    .state
                    .registers
                    .write(GuestRegister::Rcx, address_width, count, false)?;
            }
            let taken = if op == 0xe3 {
                count == 0
            } else {
                count != 0 && (op == 0xe2 || cursor.state.flags.get(GuestFlag::Zero) == (op == 0xe1))
            };
            if taken {
                branch_to(cursor.next_ip().wrapping_add(displacement as u64))
            } else {
                Ok(X64_ADVANCE)
            }
        }
        0xe8 | 0xe9 | 0xeb => {
            x64_lock(cursor.lock, None, false)?;
            let displacement = cursor.read_signed(if op == 0xeb { 1 } else { 4 })?;
            if op != 0xe8 {
                return planned(
                    cursor,
                    X64PlanOperation::Branch {
                        condition: None,
                        displacement,
                    },
                );
            }
            let target = cursor.next_ip().wrapping_add(displacement as u64);
            planned(
                cursor,
                X64PlanOperation::Call {
                    target: X64PlanSource::Immediate(target),
                },
            )
        }
        0xf4 => {
            x64_lock(cursor.lock, None, false)?;
            Err(X86Error::fault(13, "HLT is privileged in the user-mode guest"))
        }
        0xf5 => {
            x64_lock(cursor.lock, None, false)?;
            let carry = cursor.state.flags.get(GuestFlag::Carry);
            cursor.state.flags.set(GuestFlag::Carry, !carry);
            Ok(X64_ADVANCE)
        }
        0xf6 | 0xf7 => unary(cursor),
        0xf8 => {
            x64_lock(cursor.lock, None, false)?;
            cursor.state.flags.set(GuestFlag::Carry, false);
            Ok(X64_ADVANCE)
        }
        0xf9 => {
            x64_lock(cursor.lock, None, false)?;
            cursor.state.flags.set(GuestFlag::Carry, true);
            Ok(X64_ADVANCE)
        }
        0xfc => {
            x64_lock(cursor.lock, None, false)?;
            cursor.state.flags.set(GuestFlag::Direction, false);
            Ok(X64_ADVANCE)
        }
        0xfd => {
            x64_lock(cursor.lock, None, false)?;
            cursor.state.flags.set(GuestFlag::Direction, true);
            Ok(X64_ADVANCE)
        }
        0xfe | 0xff => group5(cursor),
        _ => Err(X86Error::unsupported(format!("Unsupported x86-64 opcode 0x{op:x}"))),
    }
}

fn group5(cursor: &mut X64DecodeCursor) -> Result<X64Flow, X86Error> {
    let width = cursor.width();
    let decoded = cursor.decode_modrm(if cursor.opcode() == 0xfe {
        GuestIntegerWidth::B8
    } else {
        width
    })?;
    if decoded.extension < 2 {
        return planned(
            cursor,
            X64PlanOperation::Increment {
                destination: decoded.rm,
                subtract: decoded.extension == 1,
            },
        );
    }
    x64_lock(cursor.lock, None, false)?;
    if cursor.opcode() == 0xfe {
        return Err(X86Error::fault(6, "Invalid byte INC/DEC group"));
    }
    if decoded.extension == 2 || decoded.extension == 4 {
        let operand = match decoded.rm {
            X64Operand::Register(mut operand) => {
                operand.width = GuestIntegerWidth::B64;
                X64Operand::Register(operand)
            }
            X64Operand::Memory(mut operand) => {
                operand.width = GuestIntegerWidth::B64;
                X64Operand::Memory(operand)
            }
        };
        return planned(
            cursor,
            if decoded.extension == 2 {
                X64PlanOperation::Call {
                    target: X64PlanSource::Operand(operand),
                }
            } else {
                X64PlanOperation::Jump {
                    target: X64PlanSource::Operand(operand),
                }
            },
        );
    }
    if decoded.extension == 6 {
        let stack_width = cursor.stack_width();
        let operand = match decoded.rm {
            X64Operand::Register(mut operand) => {
                operand.width = stack_width;
                X64Operand::Register(operand)
            }
            X64Operand::Memory(mut operand) => {
                operand.width = stack_width;
                X64Operand::Memory(operand)
            }
        };
        return planned(
            cursor,
            X64PlanOperation::Push {
                source: X64PlanSource::Operand(operand),
                width: stack_width,
            },
        );
    }
    Err(X86Error::unsupported(format!(
        "Far or unsupported FF group /{}",
        decoded.extension
    )))
}

fn unary(cursor: &mut X64DecodeCursor) -> Result<X64Flow, X86Error> {
    let width = if cursor.opcode() == 0xf6 {
        GuestIntegerWidth::B8
    } else {
        cursor.width()
    };
    let decoded = cursor.decode_modrm(width)?;
    if decoded.extension == 0 {
        let source = cursor.immediate(width)?;
        return planned(
            cursor,
            X64PlanOperation::Alu {
                operation: AluOperation::Test,
                destination: decoded.rm,
                source: X64PlanSource::Immediate(source),
            },
        );
    }
    if decoded.extension == 2 || decoded.extension == 3 {
        x64_lock(cursor.lock, Some(decoded.rm), true)?;
        cursor.writable(&decoded.rm)?;
        let value = cursor.read(&decoded.rm)?;
        let result = if decoded.extension == 2 {
            !value & mask(width)
        } else {
            alu(AluOperation::Sub, width.bits(), 0, value, &mut cursor.state.flags)
        };
        cursor.write(&decoded.rm, result)?;
        return Ok(X64_ADVANCE);
    }
    x64_lock(cursor.lock, None, false)?;
    if decoded.extension == 1 {
        return Err(X86Error::fault(6, "Undefined F6/F7 group /1"));
    }
    let value = cursor.read(&decoded.rm)?;
    if decoded.extension == 4 || decoded.extension == 5 {
        let signed = decoded.extension == 5;
        let accumulator = cursor.state.registers.read(GuestRegister::Rax, width, false)?;
        let full: i128 = if signed {
            let extend = |value: u64| sign_extend(value, width.bits());
            extend(accumulator) * extend(value)
        } else {
            (accumulator & mask(width)) as i128 * (value & mask(width)) as i128
        };
        let low = full as u64 & mask(width);
        let high = (full >> width.bits()) as u64 & mask(width);
        if width == GuestIntegerWidth::B8 {
            cursor
                .state
                .registers
                .write(GuestRegister::Rax, GuestIntegerWidth::B16, full as u64, false)?;
        } else {
            cursor.state.registers.write(GuestRegister::Rax, width, low, false)?;
            cursor.state.registers.write(GuestRegister::Rdx, width, high, false)?;
        }
        let overflow = if signed {
            sign_extend(low, width.bits()) != full
        } else {
            high != 0
        };
        cursor.state.flags.set(GuestFlag::Carry, overflow);
        cursor.state.flags.set(GuestFlag::Overflow, overflow);
        return Ok(X64_ADVANCE);
    }
    let signed = decoded.extension == 7;
    let raw_dividend: u128 = if width == GuestIntegerWidth::B8 {
        u128::from(
            cursor
                .state
                .registers
                .read(GuestRegister::Rax, GuestIntegerWidth::B16, false)?,
        )
    } else {
        (u128::from(cursor.state.registers.read(GuestRegister::Rdx, width, false)? & mask(width)) << width.bits())
            | u128::from(cursor.state.registers.read(GuestRegister::Rax, width, false)? & mask(width))
    };
    let double_bits = width.bits() * 2;
    let dividend: i128 = if signed {
        sign_extend_double(raw_dividend, double_bits)
    } else {
        raw_dividend as i128
    };
    let divisor: i128 = if signed {
        sign_extend(value, width.bits())
    } else {
        (value & mask(width)) as i128
    };
    if divisor == 0 {
        return Err(X86Error::fault(0, "Integer divide by zero"));
    }
    let quotient = dividend / divisor;
    let remainder = dividend % divisor;
    let fits = if signed {
        quotient_fits_signed(quotient, width.bits())
    } else {
        quotient >= 0 && (quotient as u64 & !mask(width)) == 0
    };
    if !fits {
        return Err(X86Error::fault(0, "Integer quotient overflow"));
    }
    if width == GuestIntegerWidth::B8 {
        cursor
            .state
            .registers
            .write(GuestRegister::Rax, GuestIntegerWidth::B8, quotient as u64, false)?;
        cursor
            .state
            .registers
            .write(GuestRegister::Rax, GuestIntegerWidth::B8, remainder as u64, true)?;
    } else {
        cursor
            .state
            .registers
            .write(GuestRegister::Rax, width, quotient as u64, false)?;
        cursor
            .state
            .registers
            .write(GuestRegister::Rdx, width, remainder as u64, false)?;
    }
    Ok(X64_ADVANCE)
}

fn string_op(cursor: &mut X64DecodeCursor) -> Result<X64Flow, X86Error> {
    x64_lock(cursor.lock, None, false)?;
    let op = cursor.opcode();
    let width = if op & 1 == 0 {
        GuestIntegerWidth::B8
    } else {
        cursor.width()
    };
    let repeated = cursor.repeat != X64Repeat::None;
    let address_width = if cursor.address_bits() == 32 {
        GuestIntegerWidth::B32
    } else {
        GuestIntegerWidth::B64
    };
    let mut count = cursor.state.registers.read(GuestRegister::Rcx, address_width, false)?;
    if repeated && count == 0 {
        return Ok(X64_ADVANCE);
    }
    let source_offset = cursor.state.registers.read(GuestRegister::Rsi, address_width, false)?;
    let destination_offset = cursor.state.registers.read(GuestRegister::Rdi, address_width, false)?;
    let delta = width.bytes() as i64
        * if cursor.state.flags.get(GuestFlag::Direction) {
            -1
        } else {
            1
        };
    let reads_source = matches!(op, 0xa4 | 0xa5 | 0xa6 | 0xa7 | 0xac | 0xad);
    let uses_destination = !matches!(op, 0xac | 0xad);
    let mut source = 0;
    if reads_source {
        let base = cursor.segment.map_or(0, |segment| {
            cursor.state.segments[match segment {
                X64Segment::Fs => GuestProcessorState::FS,
                X64Segment::Gs => GuestProcessorState::GS,
            }]
            .base
        });
        let space = cursor.memory.address_space();
        source = read_memory(
            &mut *cursor.memory,
            guest_address(space, source_offset.wrapping_add(base), GuestAccess::Read)?,
            width,
        )?;
    }
    if op == 0xa4 || op == 0xa5 {
        let space = cursor.memory.address_space();
        write_memory(
            &mut *cursor.memory,
            guest_address(space, destination_offset, GuestAccess::Write)?,
            width,
            source,
        )?;
    } else if op == 0xa6 || op == 0xa7 {
        let space = cursor.memory.address_space();
        let right = read_memory(
            &mut *cursor.memory,
            guest_address(space, destination_offset, GuestAccess::Read)?,
            width,
        )?;
        alu(AluOperation::Cmp, width.bits(), source, right, &mut cursor.state.flags);
    } else if op == 0xaa || op == 0xab {
        let value = cursor.state.registers.read(GuestRegister::Rax, width, false)?;
        let space = cursor.memory.address_space();
        write_memory(
            &mut *cursor.memory,
            guest_address(space, destination_offset, GuestAccess::Write)?,
            width,
            value,
        )?;
    } else if op == 0xac || op == 0xad {
        cursor.state.registers.write(GuestRegister::Rax, width, source, false)?;
    } else {
        let left = cursor.state.registers.read(GuestRegister::Rax, width, false)?;
        let space = cursor.memory.address_space();
        let right = read_memory(
            &mut *cursor.memory,
            guest_address(space, destination_offset, GuestAccess::Read)?,
            width,
        )?;
        alu(AluOperation::Cmp, width.bits(), left, right, &mut cursor.state.flags);
    }
    if reads_source {
        cursor.state.registers.write(
            GuestRegister::Rsi,
            address_width,
            source_offset.wrapping_add(delta as u64),
            false,
        )?;
    }
    if uses_destination {
        cursor.state.registers.write(
            GuestRegister::Rdi,
            address_width,
            destination_offset.wrapping_add(delta as u64),
            false,
        )?;
    }
    if !repeated {
        return Ok(X64_ADVANCE);
    }
    count = count.wrapping_sub(1) & mask(address_width);
    cursor
        .state
        .registers
        .write(GuestRegister::Rcx, address_width, count, false)?;
    let compares = matches!(op, 0xa6 | 0xa7 | 0xae | 0xaf);
    let keep = count != 0 && (!compares || cursor.state.flags.get(GuestFlag::Zero) == (cursor.repeat == X64Repeat::F3));
    // Each repeated element consumes one budget unit and is restartable at
    // its original RIP.
    if keep {
        branch_to(cursor.start())
    } else {
        Ok(X64_ADVANCE)
    }
}

fn extended(cursor: &mut X64DecodeCursor) -> Result<X64Flow, X86Error> {
    let op = cursor.read_byte()?;
    if (0x80..=0x8f).contains(&op) {
        x64_lock(cursor.lock, None, false)?;
        let displacement = cursor.read_signed(4)?;
        return planned(
            cursor,
            X64PlanOperation::Branch {
                condition: Some(op & 15),
                displacement,
            },
        );
    }
    if (0x40..=0x4f).contains(&op) {
        x64_lock(cursor.lock, None, false)?;
        let width = cursor.width();
        let decoded = cursor.decode_modrm(width)?;
        return planned(
            cursor,
            X64PlanOperation::ConditionalMove {
                destination: decoded.reg,
                source: decoded.rm,
                condition: op & 15,
            },
        );
    }
    if (0x90..=0x9f).contains(&op) {
        x64_lock(cursor.lock, None, false)?;
        let decoded = cursor.decode_modrm(GuestIntegerWidth::B8)?;
        return planned(
            cursor,
            X64PlanOperation::SetCondition {
                destination: decoded.rm,
                condition: op & 15,
            },
        );
    }
    if (0xc8..=0xcf).contains(&op) {
        x64_lock(cursor.lock, None, false)?;
        let width = if cursor.width() == GuestIntegerWidth::B64 {
            GuestIntegerWidth::B64
        } else {
            GuestIntegerWidth::B32
        };
        let index = op as usize - 0xc8 + cursor.rex_b();
        let register = cursor.register(index, width)?;
        let register = X64Operand::Register(register);
        let mut value = cursor.read(&register)?;
        let mut result = 0u64;
        for _ in 0..width.bytes() {
            result = (result << 8) | (value & 0xff);
            value >>= 8;
        }
        cursor.write(&register, result)?;
        return Ok(X64_ADVANCE);
    }
    match op {
        0x0b => Err(X86Error::fault(6, "UD2 invalid opcode")),
        0xa2 => {
            x64_lock(cursor.lock, None, false)?;
            // Virtual CPU feature enumeration, independent of the host.
            // CPUID register/feature encoding: Intel SDM Vol. 2A.
            let leaf = cursor
                .state
                .registers
                .read(GuestRegister::Rax, GuestIntegerWidth::B32, false)?;
            let (mut eax, mut ebx, mut ecx, mut edx) = (0, 0, 0, 0);
            if leaf == 0 {
                eax = 7;
                // EBX, EDX, ECX spell "QuakeTSguest".
                ebx = 0x6b61_7551;
                edx = 0x6753_5465;
                ecx = 0x7473_6575;
            } else if leaf == 1 {
                eax = 0x600;
                ebx = 0x10000;
                // x87, CMPXCHG8B, CMOV, SSE, SSE2.
                edx = 0x0600_8101;
            } else if leaf == 0x8000_0000 {
                eax = 0x8000_0001;
            } else if leaf == 0x8000_0001 {
                ecx = 1;
                edx = 0x2000_0000;
            }
            cursor
                .state
                .registers
                .write(GuestRegister::Rax, GuestIntegerWidth::B32, eax, false)?;
            cursor
                .state
                .registers
                .write(GuestRegister::Rbx, GuestIntegerWidth::B32, ebx, false)?;
            cursor
                .state
                .registers
                .write(GuestRegister::Rcx, GuestIntegerWidth::B32, ecx, false)?;
            cursor
                .state
                .registers
                .write(GuestRegister::Rdx, GuestIntegerWidth::B32, edx, false)?;
            Ok(X64_ADVANCE)
        }
        0x1e => {
            x64_lock(cursor.lock, None, false)?;
            let byte = cursor.read_byte()?;
            if cursor.repeat != X64Repeat::F3 || byte != 0xfa {
                return Err(X86Error::unsupported("Unsupported 0F 1E encoding"));
            }
            Ok(X64_ADVANCE)
        }
        0x1f => {
            x64_lock(cursor.lock, None, false)?;
            let width = cursor.width();
            cursor.decode_modrm(width)?;
            planned(cursor, X64PlanOperation::Nop)
        }
        0xaf => {
            x64_lock(cursor.lock, None, false)?;
            let width = cursor.width();
            let decoded = cursor.decode_modrm(width)?;
            planned(
                cursor,
                X64PlanOperation::Multiply {
                    destination: decoded.reg,
                    left: X64Operand::Register(decoded.reg),
                    right: X64PlanSource::Operand(decoded.rm),
                },
            )
        }
        0xb6 | 0xb7 | 0xbe | 0xbf => {
            x64_lock(cursor.lock, None, false)?;
            let source_width = if op & 1 == 0 {
                GuestIntegerWidth::B8
            } else {
                GuestIntegerWidth::B16
            };
            let decoded = cursor.decode_modrm(source_width)?;
            let width = cursor.width();
            let destination = cursor.register(decoded.register_index, width)?;
            planned(
                cursor,
                X64PlanOperation::Extend {
                    destination,
                    source: decoded.rm,
                    signed: op >= 0xbe,
                },
            )
        }
        0xb0 | 0xb1 => {
            let width = cursor.width();
            let decoded = cursor.decode_modrm(if op == 0xb0 { GuestIntegerWidth::B8 } else { width })?;
            x64_lock(cursor.lock, Some(decoded.rm), true)?;
            cursor.writable(&decoded.rm)?;
            let destination = cursor.read(&decoded.rm)?;
            let accumulator = cursor
                .state
                .registers
                .read(GuestRegister::Rax, decoded.rm.width(), false)?;
            alu(
                AluOperation::Cmp,
                decoded.rm.width().bits(),
                accumulator,
                destination,
                &mut cursor.state.flags,
            );
            if accumulator == destination {
                let source = cursor.read(&X64Operand::Register(decoded.reg))?;
                cursor.write(&decoded.rm, source)?;
            } else {
                cursor.write(&decoded.rm, destination)?;
                cursor
                    .state
                    .registers
                    .write(GuestRegister::Rax, decoded.rm.width(), destination, false)?;
            }
            Ok(X64_ADVANCE)
        }
        0xc0 | 0xc1 => {
            let width = cursor.width();
            let decoded = cursor.decode_modrm(if op == 0xc0 { GuestIntegerWidth::B8 } else { width })?;
            x64_lock(cursor.lock, Some(decoded.rm), true)?;
            cursor.writable(&decoded.rm)?;
            let destination = cursor.read(&decoded.rm)?;
            let source = cursor.read(&X64Operand::Register(decoded.reg))?;
            let sum = alu(
                AluOperation::Add,
                decoded.rm.width().bits(),
                destination,
                source,
                &mut cursor.state.flags,
            );
            let address = match decoded.rm {
                X64Operand::Memory(ref memory) => Some(cursor.address(memory, GuestAccess::Write)?),
                X64Operand::Register(_) => None,
            };
            cursor.write(&X64Operand::Register(decoded.reg), destination)?;
            match address {
                None => cursor.write(&decoded.rm, sum)?,
                Some(address) => write_memory(&mut *cursor.memory, address, decoded.rm.width(), sum)?,
            }
            Ok(X64_ADVANCE)
        }
        0xbc | 0xbd => {
            x64_lock(cursor.lock, None, false)?;
            if cursor.repeat == X64Repeat::F3 {
                return Err(X86Error::unsupported(
                    "TZCNT/LZCNT require an explicit CPU feature profile",
                ));
            }
            let width = cursor.width();
            let decoded = cursor.decode_modrm(width)?;
            let mut value = cursor.read(&decoded.rm)?;
            cursor.state.flags.set(GuestFlag::Zero, value == 0);
            if value != 0 {
                let mut index = 0u64;
                if op == 0xbc {
                    while value & 1 == 0 {
                        value >>= 1;
                        index += 1;
                    }
                } else {
                    while value > 1 {
                        value >>= 1;
                        index += 1;
                    }
                }
                cursor.write(&X64Operand::Register(decoded.reg), index)?;
            }
            Ok(X64_ADVANCE)
        }
        0xa3 | 0xab | 0xb3 | 0xbb | 0xba => bit_op(cursor, op),
        0xa4 | 0xa5 | 0xac | 0xad => double_shift(cursor, op),
        _ => {
            if (0x10..=0x17).contains(&op)
                || (0x28..=0x2f).contains(&op)
                || (0x50..=0x7f).contains(&op)
                || op == 0xae
                || op == 0xc2
                || (0xc4..=0xc6).contains(&op)
                || op >= 0xd0
            {
                numeric(cursor, Some(op))?;
                Ok(X64_ADVANCE)
            } else {
                Err(X86Error::unsupported(format!("Unsupported x86-64 0F opcode 0x{op:x}")))
            }
        }
    }
}

fn bit_op(cursor: &mut X64DecodeCursor, opcode: u8) -> Result<X64Flow, X86Error> {
    let width = cursor.width();
    let decoded = cursor.decode_modrm(width)?;
    let operation = if opcode == 0xba {
        decoded.extension
    } else if opcode == 0xa3 {
        4
    } else if opcode == 0xab {
        5
    } else if opcode == 0xb3 {
        6
    } else {
        7
    };
    if !(4..=7).contains(&operation) {
        return Err(X86Error::fault(6, "Invalid bit-test group"));
    }
    let raw_index = if opcode == 0xba {
        i64::from(cursor.read_byte()?)
    } else {
        let raw = cursor.read(&X64Operand::Register(decoded.reg))?;
        (((raw & mask(width)) << (64 - width.bits())) as i64) >> (64 - width.bits())
    };
    let shift_bits = if width == GuestIntegerWidth::B64 {
        6
    } else if width == GuestIntegerWidth::B32 {
        5
    } else {
        4
    };
    let bit = (raw_index as u64) & ((1 << shift_bits) - 1);
    let mut operand = decoded.rm;
    if matches!(operand, X64Operand::Memory(_)) && opcode != 0xba {
        let index = raw_index >> shift_bits;
        if let X64Operand::Memory(ref mut memory) = operand {
            memory.displacement += index * (width.bytes() as i64);
        }
    }
    x64_lock(cursor.lock, Some(operand), operation != 4)?;
    if operation != 4 {
        cursor.writable(&operand)?;
    }
    let value = cursor.read(&operand)?;
    let mask_bit = 1u64 << bit;
    cursor.state.flags.set(GuestFlag::Carry, value & mask_bit != 0);
    if operation != 4 {
        cursor.write(
            &operand,
            if operation == 5 {
                value | mask_bit
            } else if operation == 6 {
                value & !mask_bit
            } else {
                value ^ mask_bit
            },
        )?;
    }
    Ok(X64_ADVANCE)
}

fn double_shift(cursor: &mut X64DecodeCursor, opcode: u8) -> Result<X64Flow, X86Error> {
    x64_lock(cursor.lock, None, false)?;
    let width = cursor.width();
    let decoded = cursor.decode_modrm(width)?;
    let count = (if opcode & 1 == 0 {
        u32::from(cursor.read_byte()?)
    } else {
        cursor
            .state
            .registers
            .read(GuestRegister::Rcx, GuestIntegerWidth::B8, false)? as u32
    }) & if width == GuestIntegerWidth::B64 { 63 } else { 31 };
    if count == 0 {
        cursor.read(&decoded.rm)?;
        return Ok(X64_ADVANCE);
    }
    if count > width.bits() {
        return Err(X86Error::unsupported("Undefined SHLD/SHRD count exceeds operand width"));
    }
    cursor.writable(&decoded.rm)?;
    let destination = cursor.read(&decoded.rm)?;
    let source = cursor.read(&X64Operand::Register(decoded.reg))?;
    let right = opcode >= 0xac;
    let result = if right {
        ((destination >> count) | (source << (width.bits() - count))) & mask(width)
    } else {
        ((destination << count) | (source >> (width.bits() - count))) & mask(width)
    };
    let carry = (if right {
        destination >> (count - 1)
    } else {
        destination >> (width.bits() - count)
    } & 1)
        != 0;
    result_flags(width.bits(), result, &mut cursor.state.flags);
    cursor.state.flags.set(GuestFlag::Carry, carry);
    if count == 1 {
        cursor.state.flags.set(
            GuestFlag::Overflow,
            (destination ^ result) & (1u64 << (width.bits() - 1)) != 0,
        );
    }
    cursor.write(&decoded.rm, result)?;
    Ok(X64_ADVANCE)
}

fn numeric(cursor: &mut X64DecodeCursor, secondary_opcode: Option<u8>) -> Result<(), X86Error> {
    x64_lock(cursor.lock, None, false)?;
    let decoded = if cursor.opcode() == 0x9b {
        None
    } else {
        let width = cursor.width();
        Some(cursor.decode_modrm(width)?)
    };
    if let (Some(secondary), Some(decoded)) = (secondary_opcode, &decoded) {
        if let Some(operation) = prepare_raw_sse(secondary, cursor.numeric_prefix(), decoded.register_index) {
            let operand = match decoded.rm {
                X64Operand::Memory(memory) => X64SseOperand::Memory(memory),
                X64Operand::Register(_) => X64SseOperand::Register(decoded.rm_index),
            };
            planned(cursor, X64PlanOperation::RawSse { operation, operand })?;
            return Ok(());
        }
    }
    let immediate =
        secondary_opcode.is_some_and(|op| matches!(op, 0x70 | 0x71 | 0x72 | 0x73 | 0xc2 | 0xc4 | 0xc5 | 0xc6));
    let immediate = if immediate { Some(cursor.read_byte()?) } else { None };
    let width = cursor.width();
    let prefix = cursor.numeric_prefix();
    let opcode = cursor.opcode();
    let base = NumericInstructionBase {
        opcode,
        secondary_opcode,
        modrm: decoded.as_ref().map(|decoded| decoded.byte),
        register_index: decoded.as_ref().map_or(0, |decoded| decoded.register_index),
        prefix,
        operand_bits: width.bits() as u16,
        immediate,
    };
    match decoded.map(|decoded| decoded.rm) {
        Some(X64Operand::Memory(operand)) => {
            planned(
                cursor,
                X64PlanOperation::NumericMemory {
                    instruction: base,
                    operand,
                },
            )?;
        }
        _ => {
            let operand = decoded.map(|decoded| NumericOperand::Register(decoded.rm_index));
            planned(
                cursor,
                X64PlanOperation::Numeric {
                    instruction: base.with_operand(operand),
                },
            )?;
        }
    }
    Ok(())
}
