//! One safe hot interpreter, preserving qsrc's native stack and byte-PC ABI.
use crate::memory::ModuleMemory;
use qa_formats::program::qvm::{Image, Opcode};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trap {
    Memory,
    OperandStack,
    ProgramStack,
    Target,
    Division,
    Budget,
    Syscall,
    Return,
    Continuation,
}
impl From<crate::memory::MemoryError> for Trap {
    fn from(_: crate::memory::MemoryError) -> Self {
        Self::Memory
    }
}

pub trait SystemCalls {
    fn begin_call(&mut self, _: &mut Vm) -> Result<(), Trap> {
        Ok(())
    }
    fn call(&mut self, vm: &mut Vm, number: u32, arguments: &[i32]) -> Result<i32, Trap>;
}
pub use crate::hooks::Hooks;

/// One owned syscall snapshot. It never borrows guest or engine memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Syscall {
    pub number: u32,
    arguments: [i32; 16],
    argument_count: usize,
}
impl Syscall {
    pub fn arguments(&self) -> &[i32] {
        &self.arguments[..self.argument_count]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Syscall(Syscall),
    Returned(i32),
}

/// Move-only execution state, independent of a Vm or service borrow. Complete,
/// trap, or explicitly abort each run before discarding it.
#[must_use]
#[derive(Debug)]
pub struct Run {
    owner: Arc<()>,
    state: Option<RunState>,
}

#[derive(Clone, Copy, Debug)]
struct Settings {
    remaining: u64,
    hooks: bool,
}

#[derive(Debug)]
struct RunState {
    id: u64,
    parent: Option<u64>,
    saved_program_stack: usize,
    entry_base: usize,
    saved_settings: Option<Settings>,
    stack: [i32; 256],
    depth: usize,
    pc: usize,
    program_stack: usize,
}

struct ActiveRun {
    id: u64,
    suspended: bool,
}

pub struct Vm {
    pub memory: ModuleMemory<'static>,
    pub image: Image,
    pub hooks: Hooks,
    pub(crate) projection: Option<crate::projection::Projection>,
    program_stack: usize,
    stack_bottom: usize,
    mask: u32,
    remaining: u64,
    hooks_enabled: bool,
    run_owner: Arc<()>,
    next_run: u64,
    active_run: Option<ActiveRun>,
}
impl Vm {
    pub fn load(image: Image) -> Result<Self, Trap> {
        let memory = ModuleMemory::load(0, image.memory_size, &image.initialized)?;
        let program_stack = memory.len();
        Ok(Self {
            mask: (memory.len() - 1) as u32,
            memory,
            image,
            hooks: Hooks::load(program_stack),
            projection: None,
            remaining: 0,
            hooks_enabled: false,
            run_owner: Arc::new(()),
            next_run: 0,
            active_run: None,
            program_stack,
            stack_bottom: program_stack.saturating_sub(65536),
        })
    }
    pub fn configure_q3_game(&mut self, layout: qa_core::primitives::Q3StatLayout) {
        self.projection = Some(crate::projection::Projection::load_q3(layout));
    }
    pub fn player_stat_layout(&self) -> qa_core::primitives::Q3StatLayout {
        self.projection
            .as_ref()
            .map_or(qa_core::primitives::Q3StatLayout::BASE, |projection| {
                projection.stat_layout()
            })
    }
    pub fn bind_player_values(
        &mut self,
        values: qa_core::primitives::PlayerValueBindings<'_>,
        registry: &qa_gameplay::registry::Registry,
    ) -> Result<(), crate::services::CallError> {
        if let Some(projection) = &mut self.projection {
            projection.bind_values(values, registry)?;
        }
        Ok(())
    }
    pub fn bind_client(
        &mut self,
        entities: &qa_world::entities::EntityTable,
        native: qa_core::primitives::NativeEntity,
        entity: qa_core::primitives::EntityId,
    ) -> Result<(), crate::services::CallError> {
        self.projection
            .as_mut()
            .ok_or(crate::services::CallError::Entity)?
            .bind_client(entities, native, entity)
    }
    pub fn publish_entities(
        &mut self,
        services: &mut crate::services::EngineServices<'_>,
        context: crate::services::CallContext,
        time: qa_core::sys_events::EventTime,
        completed_export: Option<u32>,
    ) -> Result<(), crate::services::CallError> {
        if let Some(projection) = &mut self.projection {
            projection.publish(
                &mut self.memory,
                services,
                context,
                None,
                time,
                completed_export,
            )?;
        }
        Ok(())
    }
    pub fn masked_address(&self, address: i32) -> u64 {
        u64::from(address as u32 & self.mask)
    }
    pub fn is_active(&self) -> bool {
        self.active_run.is_some()
    }
    /// Cold upper bound including the root. Every admitted activation consumes
    /// at least its 48-byte entry frame from this same native stack budget.
    pub fn activation_capacity(&self) -> usize {
        self.memory.len().saturating_sub(self.stack_bottom) / 48
    }
    /// Start an independent root, saving any suspended caller's budget/hooks.
    pub fn begin_root(
        &mut self,
        arguments: [i32; 10],
        budget: u64,
        hooks: bool,
    ) -> Result<Run, Trap> {
        self.begin(
            arguments,
            Some(Settings {
                remaining: budget,
                hooks,
            }),
        )
    }
    /// A callback into an active VM shares its remaining budget and hooks.
    /// An inactive VM starts with the supplied module settings.
    pub fn begin_callback(
        &mut self,
        arguments: [i32; 10],
        budget: u64,
        hooks: bool,
    ) -> Result<Run, Trap> {
        if self.active_run.is_some() {
            self.begin(arguments, None)
        } else {
            self.begin_root(arguments, budget, hooks)
        }
    }
    fn can_begin(&self) -> Result<(), Trap> {
        if self
            .active_run
            .as_ref()
            .is_some_and(|active| !active.suspended)
        {
            return Err(Trap::Continuation);
        }
        Ok(())
    }
    fn begin(&mut self, arguments: [i32; 10], settings: Option<Settings>) -> Result<Run, Trap> {
        self.can_begin()?;
        let id = self.next_run.checked_add(1).ok_or(Trap::Continuation)?;
        let saved_program_stack = self.program_stack;
        let entry_base = saved_program_stack
            .checked_sub(48)
            .filter(|&position| position >= self.stack_bottom)
            .ok_or(Trap::ProgramStack)?;
        self.memory.read(entry_base as u64, 48)?;
        self.memory.write_word(entry_base as u64, -1)?;
        self.memory.write_word(entry_base as u64 + 4, 0)?;
        for (index, arg) in arguments.into_iter().enumerate() {
            self.memory
                .write_word((entry_base + 8 + index * 4) as u64, arg)?;
        }
        let saved_settings = settings.map(|settings| {
            let saved = self.settings();
            self.set_settings(settings);
            saved
        });
        let state = RunState {
            id,
            parent: self.active_run.as_ref().map(|active| active.id),
            saved_program_stack,
            entry_base,
            saved_settings,
            stack: [0; 256],
            depth: 0,
            pc: 0,
            program_stack: entry_base,
        };
        self.next_run = id;
        self.active_run = Some(ActiveRun {
            id,
            suspended: false,
        });
        Ok(Run {
            owner: Arc::clone(&self.run_owner),
            state: Some(state),
        })
    }
    fn settings(&self) -> Settings {
        Settings {
            remaining: self.remaining,
            hooks: self.hooks_enabled,
        }
    }
    fn set_settings(&mut self, settings: Settings) {
        self.remaining = settings.remaining;
        self.hooks_enabled = settings.hooks;
    }
    fn check_run(&self, run: &Run, suspended: Option<bool>) -> Result<(), Trap> {
        if !Arc::ptr_eq(&self.run_owner, &run.owner) {
            return Err(Trap::Continuation);
        }
        let state = run.state.as_ref().ok_or(Trap::Continuation)?;
        let active = self.active_run.as_ref().ok_or(Trap::Continuation)?;
        if active.id != state.id || suspended.is_some_and(|waiting| waiting != active.suspended) {
            return Err(Trap::Continuation);
        }
        Ok(())
    }
    fn finish(&mut self, run: &mut Run) -> Result<(), Trap> {
        let state = run.state.take().ok_or(Trap::Continuation)?;
        self.program_stack = state.saved_program_stack;
        self.active_run = state.parent.map(|id| ActiveRun {
            id,
            suspended: true,
        });
        if let Some(settings) = state.saved_settings {
            self.set_settings(settings);
        }
        Ok(())
    }
    /// Run the existing opcode loop until a syscall or final return.
    pub fn advance(&mut self, run: &mut Run) -> Result<Step, Trap> {
        self.check_run(run, Some(false))?;
        self.advance_ready(run)
    }
    /// Reply to the suspended syscall, then continue the same opcode loop.
    /// The saved native return byte-PC remains writable guest memory.
    pub fn resume(&mut self, run: &mut Run, value: i32) -> Result<Step, Trap> {
        self.check_run(run, Some(true))?;
        let result = (|| {
            let state = run.state.as_mut().ok_or(Trap::Continuation)?;
            state.depth += 1;
            *state.stack.get_mut(state.depth).ok_or(Trap::OperandStack)? = value;
            let target = self.memory.read_word(state.program_stack as u64)?;
            state.pc = self.image.instruction_at(target).ok_or(Trap::Target)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.finish(run)?;
            return Err(error);
        }
        if let Some(active) = &mut self.active_run {
            active.suspended = false;
        }
        self.advance_ready(run)
    }
    /// Abort the current activation. Wrong-owner/order attempts preserve it.
    pub fn abort(&mut self, run: &mut Run) -> Result<(), Trap> {
        self.check_run(run, None)?;
        self.finish(run)
    }
    fn advance_ready(&mut self, run: &mut Run) -> Result<Step, Trap> {
        let state = run.state.as_mut().ok_or(Trap::Continuation)?;
        let result = if self.hooks_enabled {
            self.run::<true>(state)
        } else {
            self.run::<false>(state)
        };
        match result {
            Ok(Step::Syscall(call)) => {
                if let Some(active) = &mut self.active_run {
                    active.suspended = true;
                }
                Ok(Step::Syscall(call))
            }
            result => {
                self.finish(run)?;
                result
            }
        }
    }
    pub fn call(
        &mut self,
        calls: &mut impl SystemCalls,
        arguments: [i32; 10],
        budget: u64,
        hooks: bool,
    ) -> Result<i32, Trap> {
        self.can_begin()?;
        let saved = self.settings();
        self.set_settings(Settings {
            remaining: budget,
            hooks,
        });
        let result = (|| {
            calls.begin_call(self)?;
            let mut run = self.begin(arguments, None)?;
            self.drain(calls, &mut run)
        })();
        self.set_settings(saved);
        result
    }
    /// Enter below the same suspended syscall without reseeding the projection.
    pub(crate) fn call_nested(
        &mut self,
        calls: &mut impl SystemCalls,
        arguments: [i32; 10],
    ) -> Result<i32, Trap> {
        let mut run = self.begin(arguments, None)?;
        self.drain(calls, &mut run)
    }
    fn drain(&mut self, calls: &mut impl SystemCalls, run: &mut Run) -> Result<i32, Trap> {
        let mut step = self.advance(run)?;
        loop {
            match step {
                Step::Returned(value) => return Ok(value),
                Step::Syscall(call) => {
                    let value = match calls.call(self, call.number, call.arguments()) {
                        Ok(value) => value,
                        Err(error) => {
                            self.abort(run)?;
                            return Err(error);
                        }
                    };
                    step = self.resume(run, value)?;
                }
            }
        }
    }
    fn run<const HOOKS: bool>(&mut self, state: &mut RunState) -> Result<Step, Trap> {
        use Opcode::*;
        let stack = &mut state.stack;
        let mut depth = state.depth;
        let mut pc = state.pc;
        let mut program_stack = state.program_stack;
        loop {
            self.remaining = self.remaining.checked_sub(1).ok_or(Trap::Budget)?;
            let instruction = *self.image.instructions.get(pc).ok_or(Trap::Target)?;
            pc += 1;
            let op = instruction.opcode;
            let operand = instruction.operand;
            if HOOKS {
                self.hooks.instructions += 1;
            }
            match op {
                Ignore | Break => {}
                Enter => {
                    program_stack = program_stack
                        .checked_sub(operand as usize)
                        .filter(|&s| s >= self.stack_bottom)
                        .ok_or(Trap::ProgramStack)?;
                }
                Leave => {
                    program_stack = program_stack
                        .checked_add(operand as usize)
                        .filter(|&s| s <= state.entry_base)
                        .ok_or(Trap::ProgramStack)?;
                    let target = self.memory.read_word(program_stack as u64)?;
                    if target == -1 {
                        return if depth == 1 {
                            Ok(Step::Returned(stack[1]))
                        } else {
                            Err(Trap::Return)
                        };
                    }
                    pc = self.image.instruction_at(target).ok_or(Trap::Target)?;
                }
                Const | Local | Push => {
                    depth += 1;
                    let slot = stack.get_mut(depth).ok_or(Trap::OperandStack)?;
                    *slot = match op {
                        Const => operand,
                        Local => (program_stack as i32).wrapping_add(operand),
                        _ => 0,
                    };
                }
                Pop => {
                    depth = depth.checked_sub(1).ok_or(Trap::OperandStack)?;
                }
                Jump | Call => {
                    if depth == 0 {
                        return Err(Trap::OperandStack);
                    }
                    let target = stack[depth];
                    depth -= 1;
                    if op == Call {
                        if HOOKS {
                            self.hooks.calls += 1;
                        }
                        if target < 0 && program_stack > state.entry_base {
                            return Err(Trap::ProgramStack);
                        }
                        self.memory
                            .write_word(program_stack as u64, instruction.next_byte_pc as i32)?;
                        if target < 0 {
                            let number = (-1i32).wrapping_sub(target) as u32;
                            self.memory
                                .write_word(program_stack as u64 + 4, number as i32)?;
                            let mut arguments = [0; 16];
                            arguments[0] = number as i32;
                            let available =
                                ((self.memory.len() - program_stack - 4) / 4).min(arguments.len());
                            for (index, arg) in
                                arguments.iter_mut().take(available).enumerate().skip(1)
                            {
                                *arg = self
                                    .memory
                                    .read_word((program_stack + 4 + index * 4) as u64)?;
                            }
                            self.program_stack =
                                program_stack.checked_sub(4).ok_or(Trap::ProgramStack)?;
                            state.depth = depth;
                            state.pc = pc;
                            state.program_stack = program_stack;
                            return Ok(Step::Syscall(Syscall {
                                number,
                                arguments,
                                argument_count: available,
                            }));
                        }
                    }
                    pc = usize::try_from(target)
                        .ok()
                        .filter(|&i| i < self.image.instructions.len())
                        .ok_or(Trap::Target)?;
                }
                Load1 | Load2 | Load4 => {
                    if depth == 0 {
                        return Err(Trap::OperandStack);
                    }
                    let address = self.masked_address(stack[depth]);
                    stack[depth] = match op {
                        Load1 => i32::from(self.memory.read(address, 1)?[0]),
                        Load2 => {
                            let b = self.memory.read(address, 2)?;
                            i32::from(u16::from_le_bytes([b[0], b[1]]))
                        }
                        _ => self.memory.read_word(address)?,
                    };
                }
                Store1 | Store2 | Store4 => {
                    if depth < 2 {
                        return Err(Trap::OperandStack);
                    }
                    let width = match op {
                        Store1 => 1,
                        Store2 => 2,
                        _ => 4,
                    };
                    let address = (stack[depth - 1] as u32 & self.mask & !(width - 1)) as u64;
                    self.memory
                        .write(address, &stack[depth].to_le_bytes()[..width as usize])?;
                    depth -= 2;
                    if HOOKS {
                        self.hooks.stores += 1;
                        self.hooks.write(address as usize, width as usize);
                    }
                }
                Arg => {
                    if depth == 0 {
                        return Err(Trap::OperandStack);
                    }
                    self.memory
                        .write_word((program_stack + operand as usize) as u64, stack[depth])?;
                    depth -= 1;
                    if HOOKS {
                        self.hooks.stores += 1;
                        self.hooks.write(program_stack + operand as usize, 4);
                    }
                }
                BlockCopy => {
                    if depth < 2 {
                        return Err(Trap::OperandStack);
                    }
                    let from = stack[depth] as u32 & self.mask;
                    let to = stack[depth - 1] as u32 & self.mask;
                    let count =
                        ((from.wrapping_add(operand as u32) & self.mask) as i64) - i64::from(from);
                    let count =
                        ((to.wrapping_add(count as u32) & self.mask) as i64) - i64::from(to);
                    if (from | to | count as u32) & 3 != 0 {
                        return Err(Trap::Memory);
                    }
                    let words = (count / 4).max(0) as usize;
                    for i in (0..words).rev() {
                        let word = self.memory.read_word(u64::from(from) + i as u64 * 4)?;
                        self.memory.write_word(u64::from(to) + i as u64 * 4, word)?;
                    }
                    depth -= 2;
                    if HOOKS {
                        self.hooks.stores += words as u64;
                        self.hooks.write(to as usize, words * 4);
                    }
                }
                Eq | Ne | LtI | LeI | GtI | GeI | LtU | LeU | GtU | GeU | EqF | NeF | LtF | LeF
                | GtF | GeF => {
                    if depth < 2 {
                        return Err(Trap::OperandStack);
                    }
                    let a = stack[depth - 1];
                    let b = stack[depth];
                    depth -= 2;
                    let branch = match op {
                        Eq => a == b,
                        Ne => a != b,
                        LtI => a < b,
                        LeI => a <= b,
                        GtI => a > b,
                        GeI => a >= b,
                        LtU => (a as u32) < b as u32,
                        LeU => (a as u32) <= b as u32,
                        GtU => (a as u32) > b as u32,
                        GeU => (a as u32) >= b as u32,
                        EqF => f32::from_bits(a as u32) == f32::from_bits(b as u32),
                        NeF => f32::from_bits(a as u32) != f32::from_bits(b as u32),
                        LtF => f32::from_bits(a as u32) < f32::from_bits(b as u32),
                        LeF => f32::from_bits(a as u32) <= f32::from_bits(b as u32),
                        GtF => f32::from_bits(a as u32) > f32::from_bits(b as u32),
                        GeF => f32::from_bits(a as u32) >= f32::from_bits(b as u32),
                        _ => false,
                    };
                    if branch {
                        pc = operand as usize;
                    }
                }
                Sex8 | Sex16 | NegI | NegF | CvIF | CvFI => {
                    if depth == 0 {
                        return Err(Trap::OperandStack);
                    }
                    let a = stack[depth];
                    stack[depth] = match op {
                        Sex8 => i32::from(a as i8),
                        Sex16 => i32::from(a as i16),
                        NegI => a.wrapping_neg(),
                        NegF => (-f32::from_bits(a as u32)).to_bits() as i32,
                        CvIF => (a as f32).to_bits() as i32,
                        _ => qa_core::math::native_integer(f32::from_bits(a as u32)),
                    };
                }
                Bcom => {
                    // qsrc's interpreted ABI writes the previous operand, not
                    // the top operand. Compiled ABI lowering is separate work.
                    if depth == 0 {
                        return Err(Trap::OperandStack);
                    }
                    stack[depth - 1] = !stack[depth];
                }
                Add | Sub | DivI | DivU | ModI | ModU | MulI | MulU | Band | Bor | Bxor | Lsh
                | RshI | RshU | AddF | SubF | DivF | MulF => {
                    if depth < 2 {
                        return Err(Trap::OperandStack);
                    }
                    let a = stack[depth - 1];
                    let b = stack[depth];
                    depth -= 1;
                    stack[depth] =
                        match op {
                            Add => a.wrapping_add(b),
                            Sub => a.wrapping_sub(b),
                            MulI | MulU => a.wrapping_mul(b),
                            DivI => a.checked_div(b).ok_or(Trap::Division)?,
                            ModI => a.checked_rem(b).ok_or(Trap::Division)?,
                            DivU => (a as u32).checked_div(b as u32).ok_or(Trap::Division)? as i32,
                            ModU => (a as u32).checked_rem(b as u32).ok_or(Trap::Division)? as i32,
                            Band => a & b,
                            Bor => a | b,
                            Bxor => a ^ b,
                            Lsh => a.wrapping_shl(b as u32),
                            RshI => a.wrapping_shr(b as u32),
                            RshU => (a as u32).wrapping_shr(b as u32) as i32,
                            AddF => (f32::from_bits(a as u32) + f32::from_bits(b as u32)).to_bits()
                                as i32,
                            SubF => (f32::from_bits(a as u32) - f32::from_bits(b as u32)).to_bits()
                                as i32,
                            DivF => (f32::from_bits(a as u32) / f32::from_bits(b as u32)).to_bits()
                                as i32,
                            MulF => (f32::from_bits(a as u32) * f32::from_bits(b as u32)).to_bits()
                                as i32,
                            _ => return Err(Trap::Target),
                        };
                }
            }
        }
    }
}
