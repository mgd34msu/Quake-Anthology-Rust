//! QVM interpreter: prepared bytecode execution with hooks and regions.
//!
//! Port of `src/compat/qvm/interpreter.ts`.
//!
//! Translated from Quake III Arena qcommon/vm.c and vm_interpreted.c.
//! Copyright (C) 1999-2005 Id Software, Inc.
//! SPDX-License-Identifier: GPL-2.0-or-later
//!
//! Sync-port design (documented deviations from the donor):
//!
//! - The donor is sync-or-async (`QvmSystemCallResult = number |
//!   Promise<number>`). This port is fully synchronous: host calls take
//!   shared `&dyn` callbacks (`QvmSystemCallHandler`, hooks, observers, region
//!   bindings) and return values directly. There are no `invoke_async`,
//!   `proceed_async`, or pending-child states.
//! - The donor stores the system call in the constructor; here the host is
//!   passed to every [`QvmInterpreter::invoke`]. This keeps one `&mut` path
//!   through the interpreter so recursive guest calls from host code stay
//!   sound without self-referential structs.
//! - Host callbacks (`QvmFunctionHook`, observers, resolvers, region and
//!   branch bindings) are `Fn` (shared) rather than `FnMut` so recursive
//!   guest calls reenter them exactly like the donor's plain functions; hosts
//!   keep mutable state in their own interior mutability.
//! - `bind_function`/`bind_invocation`/`observe_function` return tokens
//!   removed with `unbind_function`/`unobserve_function` (Rust cannot hand out
//!   self-borrowing unsubscribe closures).
//! - Store effects run inline: `QvmMemory` has no back-reference to the
//!   interpreter, so there is no `store_effect` routing or `publish_effect`
//!   program-stack adjustment.
//! - `evaluate_counter` takes the nested call arguments directly instead of an
//!   `execute` closure; the only donor caller passes a plain nested call.
//! - The donor's `CommonError` drop/fatal codes map to [`GuestError::Cpu`] /
//!   [`GuestError::Runtime`] with the code preserved as a message prefix
//!   (`qvm_drop_error` / `qvm_fatal_error`).
//! - Async-only machinery (validations, pending syscall children,
//!   cancellation failure slots) has no sync equivalent and is omitted; error
//!   propagation is direct.
//!
//! Hot path: the loop decodes from slices and never allocates per
//! instruction; host interaction allocates only at trap/hook boundaries.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::error::GuestError;

use super::allocation::{QvmAllocation, QvmAllocationProfile};
use super::image::{QvmDataImage, QvmImage, QvmInstruction, QvmOpcode, QvmOperand, QVM_MAX_PRIVATE_ARGUMENT_WORDS};
use super::memory::{QvmMemory, QvmSpan, QvmWritableView};
use super::operations::{evaluate_binary, evaluate_branch, evaluate_unary};
use super::regions::{qualify_qvm_region, qualify_qvm_region_evaluation, QvmRegionAccess, QvmRegionEvaluation};
use super::registry::{QvmExecutionProfile, VmRegistration};
use super::symbols::{QvmSymbolLoadOptions, QvmSymbols};

/// Ten public `vmMain` argument words.
pub type QvmArguments = [i32; 10];

/// Capability for one live intercepted call, issued and checked by its interpreter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmCancellationScope {
    pub(crate) id: u64,
}

/// Declared evaluation stack reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmEvaluationStack {
    /// Reservation start (inclusive, aligned).
    pub start: usize,
    /// Reservation end (exclusive, aligned).
    pub end: usize,
}

/// Read-only region evaluation attached to an invocation.
#[derive(Debug, Clone)]
pub struct QvmReadOnlyEvaluation {
    /// Optional declared stack reservation.
    pub stack: Option<QvmEvaluationStack>,
    /// Qualified region.
    pub region: QvmRegionEvaluation,
    /// Live-in values.
    pub inputs: Vec<i32>,
}

/// Execution semantics: interpreted loop or compiled control stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmSemantics {
    /// Pinned interpreter (`OP_BCOM` complements the previous operand slot;
    /// `OP_BLOCK_COPY` wraps with dword alignment).
    Interpreted,
    /// Compiled VM (`OP_BCOM` is unary; `OP_BLOCK_COPY` is a checked byte copy).
    Compiled,
}

/// Interpreter fault that drops to the console (`CommonError` `"drop"`).
pub fn qvm_drop_error(detail: impl Into<String>) -> GuestError {
    GuestError::cpu(format!("drop: {}", detail.into()))
}

/// Engine bug (`CommonError` `"fatal"`).
pub fn qvm_fatal_error(detail: impl Into<String>) -> GuestError {
    GuestError::runtime(format!("fatal: {}", detail.into()))
}

/// Cancellation signal for one intercepted call.
fn cancel_signal(call_id: u64) -> GuestError {
    GuestError::callback(format!("QVM function scope cancelled ({call_id})"))
}

fn is_cancel_for(error: &GuestError, call_id: u64) -> bool {
    matches!(error, GuestError::Callback(message) if *message == format!("QVM function scope cancelled ({call_id})"))
}

/// Host entry: handles one engine trap synchronously.
///
/// Shared (`&self`) so recursive guest calls reenter the same host exactly
/// like the donor's plain functions; hosts keep mutable state in their own
/// interior mutability.
pub trait QvmSystemCallHandler {
    /// Handle `call` and return the trap result.
    fn handle_syscall(&self, call: &mut QvmSyscall<'_, '_>) -> Result<i32, GuestError>;
}

impl<F> QvmSystemCallHandler for F
where
    F: for<'a, 'c> Fn(&mut QvmSyscall<'a, 'c>) -> Result<i32, GuestError>,
{
    fn handle_syscall(&self, call: &mut QvmSyscall<'_, '_>) -> Result<i32, GuestError> {
        self(call)
    }
}

/// Function replacement or wrapper hook.
pub type QvmFunctionHook = Rc<dyn for<'a, 'c, 'o> Fn(&mut QvmFunctionCall<'a, 'c, 'o>) -> Result<i32, GuestError>>;

/// Function entry observer.
pub type QvmFunctionObserver = Rc<dyn for<'a, 'c> Fn(&mut QvmFunctionObservation<'a, 'c>) -> Result<(), GuestError>>;

/// Resolver for live guest callback pointers.
pub type QvmFunctionResolver = Rc<dyn Fn(usize, i32, &[i32]) -> Option<QvmFunctionHook>>;

/// Conditional-branch decision override.
pub type QvmBranchDecide = Rc<dyn Fn(bool, &mut dyn FnMut(&QvmCancellationScope) -> GuestError) -> bool>;

/// Region execution decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmRegionDecision {
    /// Run the original region body.
    Execute,
    /// Skip to the region join.
    Skip,
}

/// Region entry handler.
pub type QvmRegionRun = Rc<dyn for<'a, 'c> Fn(&mut QvmRegionControl<'a, 'c>) -> Result<QvmRegionDecision, GuestError>>;

/// Region completion handler.
pub type QvmRegionCompleted = Rc<dyn for<'a, 'c> Fn(&mut QvmRegionControl<'a, 'c>) -> Result<(), GuestError>>;

/// Original conditional decision binding for one invocation.
pub struct QvmBranchBinding {
    /// Owning-function instruction index of the conditional.
    pub instruction_index: usize,
    /// Decision override.
    pub decide: QvmBranchDecide,
}

impl std::fmt::Debug for QvmBranchBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmBranchBinding")
            .field("instruction_index", &self.instruction_index)
            .finish_non_exhaustive()
    }
}

/// Closed, stack-neutral original region binding for one invocation.
pub struct QvmRegionBinding {
    /// Region entry instruction index.
    pub entry: usize,
    /// Region join instruction index.
    pub join: usize,
    /// Entry handler returning one execution decision.
    pub run: QvmRegionRun,
    /// Optional completion handler.
    pub completed: Option<QvmRegionCompleted>,
}

impl std::fmt::Debug for QvmRegionBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmRegionBinding")
            .field("entry", &self.entry)
            .field("join", &self.join)
            .finish_non_exhaustive()
    }
}

impl Clone for QvmRegionBinding {
    fn clone(&self) -> Self {
        Self {
            entry: self.entry,
            join: self.join,
            run: Rc::clone(&self.run),
            completed: self.completed.as_ref().map(Rc::clone),
        }
    }
}

/// Token removing a function hook installed by `bind_function`/`bind_invocation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmHookToken {
    entry: usize,
    generation: u64,
}

/// Token removing a function observer installed by `observe_function`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmObserverToken {
    entry: usize,
    id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookScope {
    Calls,
    Invocations,
}

struct FunctionBinding {
    hook: QvmFunctionHook,
    scope: HookScope,
    generation: u64,
}

struct ObserverEntry {
    id: u64,
    observe: QvmFunctionObserver,
    active: bool,
}

struct CancellationState {
    scope_id: u64,
    requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegionState {
    Ready,
    Running,
    Complete,
}

struct SourceRegion {
    binding: QvmRegionBinding,
    join_pc: i32,
    stack: usize,
    frame_size: usize,
    state: RegionState,
}

struct CallEvaluation {
    region: QvmRegionEvaluation,
    inputs: Vec<i32>,
    frame_size: usize,
}

struct ActiveCall {
    id: u64,
    stack: usize,
    return_pc: i32,
    operand_depth: usize,
    parent: Option<u64>,
    active: bool,
    branches: Option<HashMap<i32, QvmBranchDecide>>,
    regions: Vec<SourceRegion>,
    region_entries: HashMap<i32, usize>,
    region_joins: HashMap<i32, usize>,
    evaluation: Option<CallEvaluation>,
    cancellation: Option<CancellationState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RegionEvalKey {
    instruction: usize,
    entry: usize,
    join: usize,
    inputs: Vec<usize>,
    result: Option<usize>,
}

struct CounterQualification {
    functions: Vec<usize>,
    ranges: Vec<(i32, i32)>,
}

struct CounterState {
    address: usize,
    value: i32,
    stack_start: usize,
    stack_end: usize,
    functions: Vec<usize>,
    ranges: Vec<(i32, i32)>,
    remaining: i64,
}

struct QvmCore {
    memory: QvmMemory,
    symbols: QvmSymbols,
    allocations: Vec<QvmAllocation>,
    symbol_allocations: Rc<RefCell<Vec<QvmAllocation>>>,
    program_stack: usize,
    call_level: i32,
    breaks: i32,
    debug: bool,
    hooks: HashMap<usize, FunctionBinding>,
    next_hook_generation: u64,
    caller_argument_bytes: HashMap<usize, usize>,
    observers: HashMap<usize, Vec<ObserverEntry>>,
    next_observer_id: u64,
    resolver: Option<QvmFunctionResolver>,
    scopes: HashMap<u64, u64>,
    next_scope_id: u64,
    calls: Vec<ActiveCall>,
    next_call_id: u64,
    branch_function_ends: HashMap<usize, usize>,
    qualified_regions: HashMap<(usize, usize, usize), i32>,
    qualified_evaluations: HashMap<RegionEvalKey, i32>,
    read_only_regions: HashMap<RegionEvalKey, i32>,
    counter_functions: HashMap<Vec<usize>, CounterQualification>,
    counter: Option<CounterState>,
    root_active: bool,
    source_data_end: usize,
    memory_initialized_end: usize,
    registration: Option<VmRegistration>,
    semantics: QvmSemantics,
}

struct QvmProgram {
    source: String,
    code: Vec<i32>,
    instruction_pointers: Vec<i32>,
    instructions: Vec<QvmInstruction>,
    data_mask: usize,
}

struct InterpCtx<'c> {
    core: &'c mut QvmCore,
    program: &'c QvmProgram,
}

/// Shared recursive-call and cancellation control carried by every host handle.
pub struct HostControl<'a, 'c> {
    ctx: &'a mut InterpCtx<'c>,
    host: &'a dyn QvmSystemCallHandler,
    scope: Option<u64>,
}

impl<'a, 'c> HostControl<'a, 'c> {
    /// Recursive guest entry; valid only while the owning callback is active.
    pub fn invoke(
        &mut self,
        args: &[i32],
        entry: usize,
        evaluation: Option<QvmReadOnlyEvaluation>,
    ) -> Result<i32, GuestError> {
        self.ctx.core.live()?;
        self.check_cancellation()?;
        let debug = self.ctx.core.debug;
        let program = self.ctx.program;
        let qualified = match evaluation {
            Some(evaluation) => Some(qualify_evaluation(&mut *self.ctx.core, program, entry, evaluation)?),
            None => None,
        };
        let mut ops = OperandStack::new(debug);
        run_loop(
            &mut *self.ctx,
            self.host,
            &mut ops,
            args,
            entry,
            None,
            self.scope,
            qualified,
            None,
        )
    }

    /// Cancel the intercepted call that owns `scope`. Always returns the
    /// cancellation error for the caller to propagate.
    pub fn cancel_function(&mut self, scope: &QvmCancellationScope) -> GuestError {
        match self.cancel_inner(scope) {
            Ok(signal) => signal,
            Err(error) => error,
        }
    }

    fn cancel_inner(&mut self, scope: &QvmCancellationScope) -> Result<GuestError, GuestError> {
        let target = *self
            .ctx
            .core
            .scopes
            .get(&scope.id)
            .ok_or_else(|| GuestError::invalid("QVM cancellation scope belongs to another interpreter"))?;
        let position = find_call(&self.ctx.core.calls, target)
            .ok_or_else(|| GuestError::invalid("QVM cancellation scope has expired"))?;
        if !self.ctx.core.calls[position].active {
            return Err(GuestError::invalid("QVM cancellation scope has expired"));
        }
        let cancellation = self.ctx.core.calls[position]
            .cancellation
            .as_ref()
            .ok_or_else(|| GuestError::invalid("QVM cancellation scope has already been used"))?;
        if cancellation.requested {
            return Err(GuestError::invalid("QVM cancellation scope has already been used"));
        }
        let mut current = self.scope;
        while current != Some(target) {
            let Some(id) = current else {
                return Err(GuestError::invalid(
                    "QVM cancellation scope is not an ancestor of this call",
                ));
            };
            let position = find_call(&self.ctx.core.calls, id)
                .ok_or_else(|| GuestError::invalid("QVM cancellation scope is not an ancestor of this call"))?;
            current = self.ctx.core.calls[position].parent;
        }
        self.check_cancellation()?;
        if let Some(cancellation) = self.ctx.core.calls[position].cancellation.as_mut() {
            cancellation.requested = true;
        }
        Ok(cancel_signal(target))
    }

    fn check_cancellation(&self) -> Result<(), GuestError> {
        check_chain(&self.ctx.core.calls, self.scope)
    }
}

fn find_call(calls: &[ActiveCall], id: u64) -> Option<usize> {
    calls.iter().position(|call| call.id == id)
}

fn check_chain(calls: &[ActiveCall], scope: Option<u64>) -> Result<(), GuestError> {
    let mut current = scope;
    let mut cancelled: Option<GuestError> = None;
    while let Some(id) = current {
        let Some(position) = find_call(calls, id) else {
            break;
        };
        let call = &calls[position];
        if call.active {
            if let Some(cancellation) = call.cancellation.as_ref() {
                if cancellation.requested {
                    cancelled = Some(cancel_signal(call.id));
                }
            }
        }
        current = call.parent;
    }
    if let Some(signal) = cancelled {
        return Err(signal);
    }
    Ok(())
}

/// Live syscall frame: trap words, memory, and recursive entry.
pub struct QvmSyscall<'a, 'c> {
    /// Live little-endian words: syscall number, then its arguments.
    pub words: QvmWritableView,
    /// Raw allocation read by `LOAD`/`STORE`.
    pub memory: QvmMemory,
    /// Masked guest memory.
    pub guest: QvmMemory,
    control: HostControl<'a, 'c>,
}

impl<'a, 'c> QvmSyscall<'a, 'c> {
    /// Recursive guest entry; valid only while this callback is active.
    pub fn invoke(
        &mut self,
        args: &[i32],
        entry: usize,
        evaluation: Option<QvmReadOnlyEvaluation>,
    ) -> Result<i32, GuestError> {
        self.control.invoke(args, entry, evaluation)
    }

    /// Cancel an intercepted ancestor call. Returns the error to propagate.
    pub fn cancel_function(&mut self, scope: &QvmCancellationScope) -> GuestError {
        self.control.cancel_function(scope)
    }
}

impl<'a, 'c> std::fmt::Debug for QvmSyscall<'a, 'c> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmSyscall").finish_non_exhaustive()
    }
}

/// Intercepted original function call.
pub struct QvmFunctionCall<'a, 'c, 'o> {
    /// Called instruction index.
    pub instruction_index: usize,
    /// Exact original `CALL` instruction; `None` for direct host entry.
    pub caller_instruction: Option<usize>,
    /// Live argument words bounded by the caller frame and `OP_ARG` extent.
    pub words: QvmWritableView,
    /// Raw allocation.
    pub memory: QvmMemory,
    /// Masked guest memory.
    pub guest: QvmMemory,
    control: HostControl<'a, 'c>,
    ops: &'o mut OperandStack,
    call_id: u64,
    proceeded: bool,
}

impl<'a, 'c, 'o> QvmFunctionCall<'a, 'c, 'o> {
    /// Recursive guest entry.
    pub fn invoke(
        &mut self,
        args: &[i32],
        entry: usize,
        evaluation: Option<QvmReadOnlyEvaluation>,
    ) -> Result<i32, GuestError> {
        self.control.invoke(args, entry, evaluation)
    }

    /// Cancel an intercepted ancestor call. Returns the error to propagate.
    pub fn cancel_function(&mut self, scope: &QvmCancellationScope) -> GuestError {
        self.control.cancel_function(scope)
    }

    /// Reenter the current invocation as a syscall frame over detached `words`
    /// (used to dispatch engine services from source callbacks).
    pub fn reenter_as_syscall(&mut self, words: QvmWritableView) -> QvmSyscall<'_, 'c> {
        QvmSyscall {
            words,
            memory: self.memory.clone(),
            guest: self.guest.clone(),
            control: HostControl {
                ctx: &mut *self.control.ctx,
                host: self.control.host,
                scope: self.control.scope,
            },
        }
    }

    /// Open the single cancellation scope for this call, before proceeding.
    pub fn cancellation_scope(&mut self) -> Result<QvmCancellationScope, GuestError> {
        if self.proceeded {
            return Err(GuestError::invalid(
                "QVM cancellation scope must open once before proceeding",
            ));
        }
        let position = find_call(&self.control.ctx.core.calls, self.call_id)
            .ok_or_else(|| GuestError::invalid("QVM function invocation has expired"))?;
        if self.control.ctx.core.calls[position].cancellation.is_some() {
            return Err(GuestError::invalid(
                "QVM cancellation scope must open once before proceeding",
            ));
        }
        let id = self.control.ctx.core.next_scope_id;
        self.control.ctx.core.next_scope_id += 1;
        self.control.ctx.core.calls[position].cancellation = Some(CancellationState {
            scope_id: id,
            requested: false,
        });
        self.control.ctx.core.scopes.insert(id, self.call_id);
        Ok(QvmCancellationScope { id })
    }

    /// Bind original conditional decisions once, before proceeding.
    pub fn branches(&mut self, bindings: Vec<QvmBranchBinding>) -> Result<(), GuestError> {
        if self.proceeded {
            return Err(GuestError::invalid("QVM branches must bind once before proceeding"));
        }
        let position = find_call(&self.control.ctx.core.calls, self.call_id)
            .ok_or_else(|| GuestError::invalid("QVM function invocation has expired"))?;
        if self.control.ctx.core.calls[position].branches.is_some() {
            return Err(GuestError::invalid("QVM branches must bind once before proceeding"));
        }
        let entry = self.instruction_index;
        let end = match self.control.ctx.core.branch_function_ends.get(&entry) {
            Some(end) => *end,
            None => {
                let mut end = entry + 1;
                while end < self.control.ctx.program.instruction_pointers.len()
                    && code_word(self.control.ctx.program, target_pc(self.control.ctx.program, end)?)?
                        != QvmOpcode::OpEnter as i32
                {
                    end += 1;
                }
                self.control.ctx.core.branch_function_ends.insert(entry, end);
                end
            }
        };
        let mut branches = HashMap::new();
        for binding in bindings {
            if binding.instruction_index <= entry || binding.instruction_index >= end {
                return Err(GuestError::invalid("QVM branch is outside its owning function"));
            }
            let pc = target_pc(self.control.ctx.program, binding.instruction_index)?;
            let opcode = code_word(self.control.ctx.program, pc)?;
            if opcode < QvmOpcode::OpEq as i32 || opcode > QvmOpcode::OpGef as i32 || branches.contains_key(&pc) {
                return Err(GuestError::invalid(
                    "QVM branch requires a distinct original conditional instruction",
                ));
            }
            branches.insert(pc, binding.decide);
        }
        self.control.ctx.core.calls[position].branches = Some(branches);
        Ok(())
    }

    /// Bind closed, stack-neutral original regions once, before proceeding.
    pub fn regions(&mut self, bindings: Vec<QvmRegionBinding>) -> Result<(), GuestError> {
        if self.proceeded {
            return Err(GuestError::invalid("QVM regions must bind once before proceeding"));
        }
        let position = find_call(&self.control.ctx.core.calls, self.call_id)
            .ok_or_else(|| GuestError::invalid("QVM function invocation has expired"))?;
        if !self.control.ctx.core.calls[position].regions.is_empty() {
            return Err(GuestError::invalid("QVM regions must bind once before proceeding"));
        }
        let mut sorted = bindings;
        sorted.sort_by_key(|binding| binding.entry);
        let owner = self.instruction_index;
        let mut previous = owner;
        let mut regions = Vec::new();
        let mut entries = HashMap::new();
        let mut joins = HashMap::new();
        for binding in sorted {
            if binding.entry < previous {
                return Err(GuestError::invalid("QVM original regions overlap"));
            }
            let key = (owner, binding.entry, binding.join);
            let frame_size = match self.control.ctx.core.qualified_regions.get(&key) {
                Some(size) => *size,
                None => {
                    let size = qualify_qvm_region(
                        &self.control.ctx.program.instructions,
                        owner,
                        binding.entry,
                        binding.join,
                    )?;
                    self.control.ctx.core.qualified_regions.insert(key, size);
                    size
                }
            };
            let stack = self.control.ctx.core.calls[position].stack;
            let join_pc = target_pc(self.control.ctx.program, binding.join)?;
            let entry_pc = target_pc(self.control.ctx.program, binding.entry)?;
            let index = regions.len();
            regions.push(SourceRegion {
                binding,
                join_pc,
                stack: stack - frame_size as usize,
                frame_size: frame_size as usize,
                state: RegionState::Ready,
            });
            entries.insert(entry_pc, index);
            joins.insert(join_pc, index);
            previous = regions[index].binding.join;
        }
        let call = &mut self.control.ctx.core.calls[position];
        call.regions = regions;
        call.region_entries = entries;
        call.region_joins = joins;
        Ok(())
    }

    /// Run a qualified standalone region in this function's frame.
    pub fn evaluate_region(&mut self, region: &QvmRegionEvaluation, inputs: &[i32]) -> Result<i32, GuestError> {
        self.begin()?;
        let key = RegionEvalKey {
            instruction: self.instruction_index,
            entry: region.entry,
            join: region.join,
            inputs: region.inputs.clone(),
            result: region.result,
        };
        let frame_size = match self.control.ctx.core.qualified_evaluations.get(&key) {
            Some(size) => *size,
            None => {
                let size = qualify_qvm_region_evaluation(
                    &self.control.ctx.program.instructions,
                    self.instruction_index,
                    region,
                    QvmRegionAccess::Source,
                )?;
                self.control.ctx.core.qualified_evaluations.insert(key, size);
                size
            }
        };
        if inputs.len() != region.inputs.len() {
            return Err(GuestError::invalid(
                "QVM region live-ins differ from its qualified frame",
            ));
        }
        let position = find_call(&self.control.ctx.core.calls, self.call_id)
            .ok_or_else(|| GuestError::invalid("QVM function invocation has expired"))?;
        self.control.ctx.core.calls[position].evaluation = Some(CallEvaluation {
            region: region.clone(),
            inputs: inputs.to_vec(),
            frame_size: frame_size as usize,
        });
        let entry = self.instruction_index;
        let call_id = self.call_id;
        let scope = self.control.scope;
        let host = self.control.host;
        run_loop(
            &mut *self.control.ctx,
            host,
            &mut *self.ops,
            &[],
            entry,
            Some(call_id),
            scope,
            None,
            None,
        )
    }

    /// Deliver a synchronous host effect under this live source invocation.
    pub fn effect(&mut self, perform: &mut dyn FnMut() -> Result<(), GuestError>) -> Result<(), GuestError> {
        self.control.ctx.core.live()?;
        check_chain(&self.control.ctx.core.calls, Some(self.call_id))?;
        perform()
    }

    /// Run the original body once with its caller stack and argument addresses.
    pub fn proceed(&mut self) -> Result<i32, GuestError> {
        self.begin()?;
        let entry = self.instruction_index;
        let call_id = self.call_id;
        let scope = self.control.scope;
        let host = self.control.host;
        run_loop(
            &mut *self.control.ctx,
            host,
            &mut *self.ops,
            &[],
            entry,
            Some(call_id),
            scope,
            None,
            None,
        )
    }

    fn begin(&mut self) -> Result<(), GuestError> {
        if self.proceeded {
            return Err(GuestError::invalid("QVM function continuation can only run once"));
        }
        self.proceeded = true;
        Ok(())
    }
}

impl<'a, 'c, 'o> std::fmt::Debug for QvmFunctionCall<'a, 'c, 'o> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmFunctionCall")
            .field("instruction_index", &self.instruction_index)
            .field("caller_instruction", &self.caller_instruction)
            .finish_non_exhaustive()
    }
}

/// Read-only view of a function entry for observers.
pub struct QvmFunctionObservation<'a, 'c> {
    /// Entered instruction index.
    pub instruction_index: usize,
    words: QvmWritableView,
    control: HostControl<'a, 'c>,
}

impl<'a, 'c> QvmFunctionObservation<'a, 'c> {
    /// Recursive guest entry.
    pub fn invoke(
        &mut self,
        args: &[i32],
        entry: usize,
        evaluation: Option<QvmReadOnlyEvaluation>,
    ) -> Result<i32, GuestError> {
        self.control.invoke(args, entry, evaluation)
    }

    /// Cancel an intercepted ancestor call. Returns the error to propagate.
    pub fn cancel_function(&mut self, scope: &QvmCancellationScope) -> GuestError {
        self.control.cancel_function(scope)
    }

    /// Read caller argument word `index`.
    pub fn argument(&self, index: usize) -> Result<i32, GuestError> {
        if index >= self.words.len() / 4 {
            return Err(GuestError::invalid("QVM argument index outside source call"));
        }
        self.words.get_i32(index * 4)
    }
}

impl<'a, 'c> std::fmt::Debug for QvmFunctionObservation<'a, 'c> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmFunctionObservation")
            .field("instruction_index", &self.instruction_index)
            .finish_non_exhaustive()
    }
}

/// Control surface for one region callback.
pub struct QvmRegionControl<'a, 'c> {
    control: HostControl<'a, 'c>,
    region_stack: usize,
    frame_size: usize,
}

impl<'a, 'c> QvmRegionControl<'a, 'c> {
    /// Recursive guest entry.
    pub fn invoke(&mut self, args: &[i32], entry: usize) -> Result<i32, GuestError> {
        self.control.invoke(args, entry, None)
    }

    /// Cancel an intercepted ancestor call. Returns the error to propagate.
    pub fn cancel_function(&mut self, scope: &QvmCancellationScope) -> GuestError {
        self.control.cancel_function(scope)
    }

    /// Read an aligned word in the original function's local frame.
    pub fn local_word(&mut self, offset: usize) -> Result<i32, GuestError> {
        if offset < 8 || !offset.is_multiple_of(4) || offset + 4 > self.frame_size {
            return Err(GuestError::invalid("QVM region local is outside its original frame"));
        }
        let address = self.region_stack + offset;
        read_word(self.control.ctx.core, address)
    }
}

impl<'a, 'c> std::fmt::Debug for QvmRegionControl<'a, 'c> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmRegionControl").finish_non_exhaustive()
    }
}

/// Operand stack: 256 slots, cells start uninitialized like the donor.
pub struct OperandStack {
    cells: [Option<i32>; 256],
    depth: usize,
    debug: bool,
}

impl std::fmt::Debug for OperandStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperandStack")
            .field("depth", &self.depth)
            .finish_non_exhaustive()
    }
}

impl OperandStack {
    /// Fresh stack.
    #[must_use]
    pub fn new(debug: bool) -> Self {
        Self {
            cells: [None; 256],
            depth: 0,
            debug,
        }
    }

    /// Current depth.
    #[must_use]
    pub fn count(&self) -> usize {
        self.depth
    }

    /// Reserve one slot without writing it.
    pub fn reserve(&mut self) -> Result<(), GuestError> {
        if self.depth == 255 {
            if self.debug {
                return Err(qvm_drop_error("VM opStack overflow"));
            }
            return Err(GuestError::invalid("QVM operand stack overflow"));
        }
        self.depth += 1;
        Ok(())
    }

    /// Push a word.
    pub fn push(&mut self, word: i32) -> Result<(), GuestError> {
        self.reserve()?;
        self.cells[self.depth] = Some(word);
        Ok(())
    }

    /// Peek at the top word.
    pub fn peek(&self) -> Result<i32, GuestError> {
        self.cells[self.depth].ok_or_else(|| {
            if self.debug {
                qvm_drop_error("QVM reads an uninitialized operand")
            } else {
                GuestError::invalid("QVM reads an uninitialized operand")
            }
        })
    }

    /// Overwrite the top word.
    pub fn set(&mut self, word: i32) {
        self.cells[self.depth] = Some(word);
    }

    /// Pop the top word.
    pub fn pop(&mut self) -> Result<i32, GuestError> {
        let word = self.peek()?;
        self.drop_top()?;
        Ok(word)
    }

    /// Drop the top word.
    pub fn drop_top(&mut self) -> Result<(), GuestError> {
        if self.depth == 0 {
            if self.debug {
                return Err(qvm_drop_error("VM opStack underflow"));
            }
            return Err(GuestError::invalid("QVM operand stack underflow"));
        }
        self.depth -= 1;
        Ok(())
    }

    /// Pinned-interpreter `OP_BCOM`: complement the previous slot in place.
    pub fn complement_previous(&mut self) -> Result<(), GuestError> {
        if self.depth == 0 {
            if self.debug {
                return Err(qvm_drop_error("VM opStack underflow"));
            }
            return Err(GuestError::invalid("QVM operand stack underflow"));
        }
        let top = self.peek()?;
        self.cells[self.depth - 1] = Some(!top);
        Ok(())
    }

    /// Final result: exactly one word must remain.
    pub fn result(&self) -> Result<i32, GuestError> {
        if self.depth != 1 {
            return Err(qvm_drop_error(format!("Interpreter error: opStack = {}", self.depth)));
        }
        self.peek()
    }

    /// Truncate back to a caller depth, clearing dropped cells.
    pub fn truncate(&mut self, depth: usize) -> Result<(), GuestError> {
        if depth > self.depth {
            return Err(GuestError::invalid("QVM cancellation lost its caller operands"));
        }
        while self.depth > depth {
            self.cells[self.depth] = None;
            self.depth -= 1;
        }
        Ok(())
    }
}

fn target_pc(program: &QvmProgram, index: usize) -> Result<i32, GuestError> {
    program
        .instruction_pointers
        .get(index)
        .copied()
        .ok_or_else(|| GuestError::invalid(format!("{}: invalid QVM instruction index {index}", program.source)))
}

fn code_word(program: &QvmProgram, pc: i32) -> Result<i32, GuestError> {
    if pc < 0 {
        return Err(GuestError::invalid(format!(
            "{}: invalid QVM byte PC {pc}",
            program.source
        )));
    }
    program
        .code
        .get(pc as usize)
        .copied()
        .ok_or_else(|| GuestError::invalid(format!("{}: invalid QVM byte PC {pc}", program.source)))
}

fn source_instruction(program: &QvmProgram, pc: i32) -> Result<usize, GuestError> {
    let mut low = 0i64;
    let mut high = program.instruction_pointers.len() as i64 - 1;
    while low <= high {
        let middle = ((low + high) as u64 >> 1) as usize;
        let candidate = program.instruction_pointers[middle];
        if candidate == pc {
            return Ok(middle);
        }
        if candidate < pc {
            low = middle as i64 + 1;
        } else {
            high = middle as i64 - 1;
        }
    }
    Err(GuestError::invalid(format!(
        "{}: caller PC is not an original instruction",
        program.source
    )))
}

fn source_argument_bytes(ctx: &mut InterpCtx<'_>, caller_instruction: usize) -> Result<usize, GuestError> {
    if let Some(retained) = ctx.core.caller_argument_bytes.get(&caller_instruction) {
        return Ok(*retained);
    }
    let mut index = caller_instruction as i64;
    while index >= 0 {
        let instruction = &ctx.program.instructions[index as usize];
        if instruction.opcode == QvmOpcode::OpEnter {
            let QvmOperand::Word(frame) = instruction.operand else {
                return Err(GuestError::invalid("QVM source call lacks an aligned caller frame"));
            };
            if frame < 8 || frame % 4 != 0 {
                return Err(GuestError::invalid("QVM source call lacks an aligned caller frame"));
            }
            let bytes = ((frame - 8) as usize).min(QVM_MAX_PRIVATE_ARGUMENT_WORDS * 4);
            ctx.core.caller_argument_bytes.insert(caller_instruction, bytes);
            return Ok(bytes);
        }
        index -= 1;
    }
    Err(GuestError::invalid("QVM source call has no original caller frame"))
}

fn mask_address(word: i32, mask: usize) -> usize {
    (word as u32 as usize) & mask
}

fn read_word(core: &QvmCore, address: usize) -> Result<i32, GuestError> {
    if let Some(counter) = core.counter.as_ref() {
        if address + 4 > counter.address && address < counter.address + 4 {
            if address != counter.address {
                return Err(GuestError::invalid(
                    "QVM counter evaluation cannot partially read its isolated word",
                ));
            }
            return Ok(counter.value);
        }
    }
    core.memory.get_i32(address)
}

fn write_word(core: &mut QvmCore, sp: usize, address: usize, word: i32) -> Result<(), GuestError> {
    if let Some(counter) = core.counter.as_mut() {
        if address == counter.address {
            counter.value = word;
            return Ok(());
        }
        let floor = counter.stack_start.max(sp);
        if address < floor || address + 4 > counter.stack_end {
            return Err(GuestError::invalid(
                "QVM counter evaluation attempted an unrelated source write",
            ));
        }
        return core.memory.set_i32_unobserved(address, word);
    }
    if core.memory.observes_writes() {
        core.memory.set_i32(address, word)
    } else {
        core.memory.set_i32_unobserved(address, word)
    }
}

fn counter_narrow_read(core: &QvmCore, address: usize, length: usize) -> Result<(), GuestError> {
    if let Some(counter) = core.counter.as_ref() {
        if address + length > counter.address && address < counter.address + 4 {
            return Err(GuestError::invalid(
                "QVM counter evaluation cannot partially read its isolated word",
            ));
        }
    }
    Ok(())
}

fn counter_narrow_write(
    core: &mut QvmCore,
    sp: usize,
    address: usize,
    length: usize,
    value: i32,
) -> Result<bool, GuestError> {
    let Some(counter) = core.counter.as_mut() else {
        return Ok(false);
    };
    if address == counter.address && length == 4 {
        counter.value = value;
        return Ok(true);
    }
    let floor = counter.stack_start.max(sp);
    if address < floor || address + length > counter.stack_end {
        return Err(GuestError::invalid(
            "QVM counter evaluation attempted an unrelated source write",
        ));
    }
    Ok(false)
}

impl QvmCore {
    fn live(&self) -> Result<(), GuestError> {
        self.memory.assert_not_publishing()?;
        self.memory.assert_live()?;
        if let Some(registration) = self.registration.as_ref() {
            if registration.binding().is_freed() {
                return Err(GuestError::invalid("QVM registration has been freed"));
            }
        }
        for allocation in &self.allocations {
            if !allocation.is_live() {
                return Err(GuestError::invalid("QVM allocation has been released"));
            }
        }
        for allocation in self.symbol_allocations.borrow().iter() {
            if !allocation.is_live() {
                return Err(GuestError::invalid("QVM allocation has been released"));
            }
        }
        Ok(())
    }

    fn evaluation_stack_start(&self, stack: Option<QvmEvaluationStack>) -> Result<usize, GuestError> {
        let Some(stack) = stack else {
            return Ok(self.source_data_end.next_multiple_of(4));
        };
        if stack.start % 4 != 0
            || stack.end % 4 != 0
            || stack.start < self.memory_initialized_end
            || stack.start >= stack.end
            || stack.end > self.memory.len()
            || self.program_stack > stack.end
        {
            return Err(GuestError::invalid(
                "QVM evaluation stack is outside its declared source reservation or active caller",
            ));
        }
        Ok(stack.start)
    }
}

/// Owns prepared bytecode and its private data for one module instance.
pub struct QvmInterpreter {
    core: QvmCore,
    program: QvmProgram,
}

impl std::fmt::Debug for QvmInterpreter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmInterpreter")
            .field("source", &self.program.source)
            .field("program_stack", &self.core.program_stack)
            .finish_non_exhaustive()
    }
}

impl QvmInterpreter {
    /// Prepare `image` for execution. The host is supplied per invocation (see
    /// the module docs); `registration` may be `None` for standalone use.
    pub fn new(
        image: &QvmImage,
        mut profile: QvmAllocationProfile,
        registration: Option<VmRegistration>,
        semantics: QvmSemantics,
    ) -> Result<Self, GuestError> {
        let mut allocations: Vec<QvmAllocation> = Vec::new();
        let accounted = !matches!(profile, QvmAllocationProfile::Unaccounted);
        let mut memory_bytes = if accounted {
            let mut allocation = profile.allocate("VM_Create:dataBase", &image.source, image.allocated_data_length)?;
            let bytes = allocation.take_bytes()?;
            allocations.push(allocation);
            bytes
        } else {
            vec![0; image.allocated_data_length]
        };
        if memory_bytes.len() != image.allocated_data_length {
            return Err(GuestError::invalid("QVM arena issued a short data allocation"));
        }
        if image.initialized_data.len() > image.allocated_data_length {
            return Err(GuestError::bad_image(
                "qvm",
                "QVM initialized data exceeds its allocation",
            ));
        }
        if accounted {
            allocations.push(profile.allocate(
                "VM_Create:instructionPointers",
                &image.source,
                image.instructions.len() * 4,
            )?);
            allocations.push(profile.allocate("VM_PrepareInterpreter", &image.source, image.code_length * 4)?);
        }
        memory_bytes[..image.initialized_data.len()].copy_from_slice(&image.initialized_data);
        if let Some(registration) = registration.as_ref() {
            registration.bind_data(memory_bytes.len());
        }
        let memory = QvmMemory::new(memory_bytes)?;
        let data_mask = image.allocated_data_length - 1;
        let program_stack = image.allocated_data_length;
        if let Some(registration) = registration.as_ref() {
            registration.bind_instruction_pointers_length(image.instructions.len() * 4);
        }
        let mut instruction_pointers = vec![0i32; image.instructions.len()];
        for (index, instruction) in image.instructions.iter().enumerate() {
            instruction_pointers[index] = instruction.byte_offset as i32;
        }
        if let Some(registration) = registration.as_ref() {
            registration.bind_code_length(image.code_length);
        }
        // Source preparation expands each code byte to an int slot. Operand
        // tails and alignment slots remain zero, and return PCs address these
        // slots too.
        let mut code = vec![0i32; image.code_length];
        for instruction in &image.instructions {
            let width = instruction.opcode.operand_width();
            if instruction.byte_offset > image.code_length || width + 1 > image.code_length - instruction.byte_offset {
                return Err(GuestError::bad_image("qvm", "QVM instruction exceeds code section"));
            }
            code[instruction.byte_offset] = instruction.opcode as i32;
            match instruction.operand {
                QvmOperand::None => {}
                QvmOperand::Byte(byte) => {
                    code[instruction.byte_offset + 1] = i32::from(byte);
                }
                QvmOperand::Word(word) => {
                    code[instruction.byte_offset + 1] = if instruction.opcode.is_branch() {
                        *instruction_pointers.get(word as usize).ok_or_else(|| {
                            GuestError::bad_image("qvm", format!("QVM branch target {word} outside instruction table"))
                        })?
                    } else {
                        word
                    };
                }
            }
        }
        if let Some(registration) = registration.as_ref() {
            registration.bind_interpreter(code.len(), instruction_pointers.len() * 4, memory.len());
        }
        let program = QvmProgram {
            source: image.source.clone(),
            code,
            instruction_pointers: instruction_pointers.clone(),
            instructions: image.instructions.clone(),
            data_mask,
        };
        let live_memory = memory.clone();
        let live_registration = registration.clone();
        let live_allocations = allocations.clone();
        let symbol_allocations: Rc<RefCell<Vec<QvmAllocation>>> = Rc::new(RefCell::new(Vec::new()));
        let symbol_blocks = Rc::clone(&symbol_allocations);
        let live_symbols = Rc::clone(&symbol_allocations);
        let symbols = QvmSymbols::new(
            instruction_pointers,
            Box::new(move |bytes, resource| {
                if !accounted {
                    return Ok(vec![0; bytes]);
                }
                let mut allocation = profile.allocate("VM_LoadSymbols", resource, bytes)?;
                let owned = allocation.take_bytes()?;
                symbol_blocks.borrow_mut().push(allocation);
                Ok(owned)
            }),
            Box::new(move || {
                live_memory.assert_not_publishing()?;
                live_memory.assert_live()?;
                if let Some(registration) = live_registration.as_ref() {
                    if registration.binding().is_freed() {
                        return Err(GuestError::invalid("QVM registration has been freed"));
                    }
                }
                for allocation in &live_allocations {
                    if !allocation.is_live() {
                        return Err(GuestError::invalid("QVM allocation has been released"));
                    }
                }
                for allocation in live_symbols.borrow().iter() {
                    if !allocation.is_live() {
                        return Err(GuestError::invalid("QVM allocation has been released"));
                    }
                }
                Ok(())
            }),
        );
        Ok(Self {
            core: QvmCore {
                memory,
                symbols,
                allocations,
                symbol_allocations,
                program_stack,
                call_level: 0,
                breaks: 0,
                debug: false,
                hooks: HashMap::new(),
                next_hook_generation: 1,
                caller_argument_bytes: HashMap::new(),
                observers: HashMap::new(),
                next_observer_id: 1,
                resolver: None,
                scopes: HashMap::new(),
                next_scope_id: 1,
                calls: Vec::new(),
                next_call_id: 1,
                branch_function_ends: HashMap::new(),
                qualified_regions: HashMap::new(),
                qualified_evaluations: HashMap::new(),
                read_only_regions: HashMap::new(),
                counter_functions: HashMap::new(),
                counter: None,
                root_active: false,
                source_data_end: image.data_length + image.literal_length + image.bss_length,
                memory_initialized_end: image.data_length + image.literal_length,
                registration,
                semantics,
            },
            program,
        })
    }

    /// Whether a root invocation is running.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.core.root_active
    }

    /// Current program stack pointer.
    #[must_use]
    pub fn stack_pointer(&self) -> usize {
        self.core.program_stack
    }

    /// Shared masked memory.
    #[must_use]
    pub fn memory(&self) -> QvmMemory {
        self.core.memory.clone()
    }

    /// Shared masked memory (donor `addressSpace` alias).
    #[must_use]
    pub fn address_space(&self) -> QvmMemory {
        self.core.memory.clone()
    }

    /// Symbol table.
    #[must_use]
    pub fn symbols(&self) -> &QvmSymbols {
        &self.core.symbols
    }

    /// Current call level (drives [`Self::indent`]).
    #[must_use]
    pub fn call_level(&self) -> i32 {
        self.core.call_level
    }

    /// Replace the call level.
    pub fn set_call_level(&mut self, level: i32) {
        self.core.call_level = level;
    }

    /// Count of `OP_BREAK` executions.
    #[must_use]
    pub fn break_count(&self) -> i32 {
        self.core.breaks
    }

    /// Whether the running invocation uses the debug profile.
    #[must_use]
    pub fn debug_enabled(&self) -> bool {
        self.core.debug
    }

    /// Prepared code length in slots.
    pub fn code_length(&self) -> Result<usize, GuestError> {
        self.core.live()?;
        Ok(self.program.code.len())
    }

    /// Instruction-pointer table length in bytes.
    pub fn instruction_pointers_length(&self) -> Result<usize, GuestError> {
        self.core.live()?;
        Ok(self.program.instruction_pointers.len() * 4)
    }

    /// Two spaces per call level, capped at level 20.
    pub fn indent(&self) -> Result<String, GuestError> {
        if self.core.call_level < 0 {
            return Err(GuestError::invalid("VM indentation requires a nonnegative call level"));
        }
        Ok(" ".repeat(2 * self.core.call_level.min(20) as usize))
    }

    /// Print a stack trace from `program_counter`/`program_stack`.
    pub fn stack_trace(
        &self,
        mut program_counter: i32,
        mut program_stack: usize,
        print: &mut dyn FnMut(&str),
    ) -> Result<(), GuestError> {
        self.core.live()?;
        let mut count = 0;
        loop {
            print(&format!("{}\n", self.core.symbols.value_to_symbol(program_counter)?));
            program_stack = read_word(&self.core, program_stack + 4)? as usize;
            program_counter = read_word(&self.core, program_stack)?;
            count += 1;
            if program_counter == -1 || count >= 32 {
                break;
            }
        }
        Ok(())
    }

    /// Load `vm/*.map` symbols.
    pub fn load_symbols(&mut self, options: QvmSymbolLoadOptions) -> Result<(), GuestError> {
        self.core.symbols.load(options)
    }

    /// Mask a guest word to a live span.
    pub fn pointer(&self, word: i32) -> Result<Option<QvmSpan>, GuestError> {
        self.core.live()?;
        self.core.memory.pointer(word)
    }

    /// Bind a source `OP_CALL` target in the immutable instruction table.
    pub fn bind_function(
        &mut self,
        instruction_index: usize,
        hook: QvmFunctionHook,
    ) -> Result<QvmHookToken, GuestError> {
        self.bind_entry(instruction_index, hook, HookScope::Calls)
    }

    /// Bind direct host entry too, keeping the same body and cancellation scope.
    pub fn bind_invocation(
        &mut self,
        instruction_index: usize,
        hook: QvmFunctionHook,
    ) -> Result<QvmHookToken, GuestError> {
        self.bind_entry(instruction_index, hook, HookScope::Invocations)
    }

    fn bind_entry(
        &mut self,
        instruction_index: usize,
        hook: QvmFunctionHook,
        scope: HookScope,
    ) -> Result<QvmHookToken, GuestError> {
        self.core.live()?;
        let pc = target_pc(&self.program, instruction_index)
            .map_err(|_| GuestError::invalid("QVM hook requires a function entry instruction"))?;
        if code_word(&self.program, pc)? != QvmOpcode::OpEnter as i32 {
            return Err(GuestError::invalid("QVM hook requires a function entry instruction"));
        }
        if self.core.hooks.contains_key(&instruction_index) {
            return Err(GuestError::invalid("QVM function already has a hook"));
        }
        let generation = self.core.next_hook_generation;
        self.core.next_hook_generation += 1;
        self.core.hooks.insert(
            instruction_index,
            FunctionBinding {
                hook,
                scope,
                generation,
            },
        );
        Ok(QvmHookToken {
            entry: instruction_index,
            generation,
        })
    }

    /// Remove a hook installed by `bind_function`/`bind_invocation`.
    pub fn unbind_function(&mut self, token: QvmHookToken) {
        if self
            .core
            .hooks
            .get(&token.entry)
            .is_some_and(|binding| binding.generation == token.generation)
        {
            self.core.hooks.remove(&token.entry);
        }
    }

    /// Resolve live guest callback pointers for modules that request it.
    pub fn bind_function_resolver(&mut self, resolve: QvmFunctionResolver) -> Result<(), GuestError> {
        self.core.live()?;
        if self.core.resolver.is_some() {
            return Err(GuestError::invalid("QVM already has a function resolver"));
        }
        self.core.resolver = Some(resolve);
        Ok(())
    }

    /// Remove the function resolver.
    pub fn unbind_function_resolver(&mut self) {
        self.core.resolver = None;
    }

    /// Observe a function entry; observers share entries with each other and
    /// with the optional replacement hook.
    pub fn observe_function(
        &mut self,
        instruction_index: usize,
        observe: QvmFunctionObserver,
    ) -> Result<QvmObserverToken, GuestError> {
        self.core.live()?;
        let pc = target_pc(&self.program, instruction_index)
            .map_err(|_| GuestError::invalid("QVM observer requires a function entry instruction"))?;
        if code_word(&self.program, pc)? != QvmOpcode::OpEnter as i32 {
            return Err(GuestError::invalid(
                "QVM observer requires a function entry instruction",
            ));
        }
        let id = self.core.next_observer_id;
        self.core.next_observer_id += 1;
        self.core
            .observers
            .entry(instruction_index)
            .or_default()
            .push(ObserverEntry {
                id,
                observe,
                active: true,
            });
        Ok(QvmObserverToken {
            entry: instruction_index,
            id,
        })
    }

    /// Remove an observer installed by `observe_function`.
    pub fn unobserve_function(&mut self, token: QvmObserverToken) {
        if let Some(entries) = self.core.observers.get_mut(&token.entry) {
            for entry in entries.iter_mut() {
                if entry.id == token.id {
                    entry.active = false;
                }
            }
            entries.retain(|entry| entry.active);
            if entries.is_empty() {
                self.core.observers.remove(&token.entry);
            }
        }
    }

    /// Restore a checkpointed allocation image.
    pub fn restore_data(&mut self, data: &[u8]) -> Result<(), GuestError> {
        if self.core.root_active {
            return Err(GuestError::invalid("Cannot restore an active QVM"));
        }
        self.core.live()?;
        if data.len() != self.core.memory.len() {
            return Err(GuestError::invalid("QVM checkpoint allocation mismatch"));
        }
        self.core.memory.clear_write_observers()?;
        self.core.memory.write_bytes(0, data)?;
        Ok(())
    }

    /// `VM_Restart`: zero the original allocation and copy fresh data.
    pub fn restart(&mut self, image: &QvmDataImage) -> Result<(), GuestError> {
        if self.core.root_active {
            return Err(GuestError::invalid("Cannot restart an active QVM"));
        }
        self.core.live()?;
        if image.allocated_data_length > self.core.memory.len() {
            return Err(GuestError::invalid("QVM restart would exceed its original allocation"));
        }
        self.core.memory.clear_write_observers()?;
        self.core.memory.fill_bytes(0, image.allocated_data_length, 0)?;
        self.core.memory.write_bytes(0, &image.initialized_data)?;
        Ok(())
    }

    /// Root invocation: run `entry` with `args` under `host`.
    pub fn invoke(
        &mut self,
        host: &dyn QvmSystemCallHandler,
        args: &[i32],
        entry: usize,
        evaluation: Option<QvmReadOnlyEvaluation>,
    ) -> Result<i32, GuestError> {
        if self.core.root_active {
            return Err(GuestError::invalid(
                "QVM is already active; recursive calls belong to the current syscall",
            ));
        }
        self.core.live()?;
        self.core.root_active = true;
        let result = self.invoke_inner(host, args, entry, evaluation);
        self.core.root_active = false;
        result
    }

    fn invoke_inner(
        &mut self,
        host: &dyn QvmSystemCallHandler,
        args: &[i32],
        entry: usize,
        evaluation: Option<QvmReadOnlyEvaluation>,
    ) -> Result<i32, GuestError> {
        let profile = self
            .core
            .registration
            .as_ref()
            .map(|registration| registration.execution_profile())
            .unwrap_or(QvmExecutionProfile::Release);
        let debug = matches!(profile, QvmExecutionProfile::Debug { .. });
        let qualified = match evaluation {
            Some(evaluation) => Some(self.qualify_read_only(entry, evaluation)?),
            None => None,
        };
        let mut ctx = InterpCtx {
            core: &mut self.core,
            program: &self.program,
        };
        let mut ops = OperandStack::new(debug);
        run_loop(
            &mut ctx,
            host,
            &mut ops,
            args,
            entry,
            None,
            None,
            qualified,
            Some(profile),
        )
    }

    fn qualify_read_only(
        &mut self,
        entry: usize,
        evaluation: QvmReadOnlyEvaluation,
    ) -> Result<QualifiedReadOnly, GuestError> {
        qualify_evaluation(&mut self.core, &self.program, entry, evaluation)
    }

    /// Run an original counter leaf with one virtual word; no source stores or
    /// host callbacks escape. Returns the isolated word after the nested call.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_counter(
        &mut self,
        host: &dyn QvmSystemCallHandler,
        address: usize,
        initial: i32,
        functions: &[usize],
        stack: Option<QvmEvaluationStack>,
        args: &[i32],
        entry: usize,
    ) -> Result<i32, GuestError> {
        self.core.live()?;
        self.core.memory.data_view(address, 4)?;
        if !address.is_multiple_of(4) || address + 4 > self.core.source_data_end || self.core.counter.is_some() {
            return Err(GuestError::invalid(
                "QVM counter evaluation requires one live aligned source word and a fresh scope",
            ));
        }
        let key = functions.to_vec();
        if let std::collections::hash_map::Entry::Vacant(entry) = self.core.counter_functions.entry(key.clone()) {
            let mut admitted = Vec::new();
            for function in functions {
                if admitted.contains(function) {
                    return Err(GuestError::invalid(
                        "QVM counter evaluation requires distinct original functions",
                    ));
                }
                admitted.push(*function);
            }
            if admitted.is_empty() {
                return Err(GuestError::invalid(
                    "QVM counter evaluation requires distinct original functions",
                ));
            }
            let mut ranges = Vec::new();
            for function in &admitted {
                if self
                    .program
                    .instructions
                    .get(*function)
                    .map(|instruction| instruction.opcode)
                    != Some(QvmOpcode::OpEnter)
                {
                    return Err(GuestError::invalid(
                        "QVM counter evaluation entry is not an original function",
                    ));
                }
                let mut end = function + 1;
                while end < self.program.instructions.len()
                    && self.program.instructions[end].opcode != QvmOpcode::OpEnter
                {
                    end += 1;
                }
                let start = target_pc(&self.program, *function)?;
                let finish = if end == self.program.instructions.len() {
                    self.program.code.len() as i32
                } else {
                    target_pc(&self.program, end)?
                };
                ranges.push((start, finish));
            }
            entry.insert(CounterQualification {
                functions: admitted,
                ranges,
            });
        }
        let stack_start = self.core.evaluation_stack_start(stack)?;
        if address + 4 > stack_start {
            return Err(GuestError::invalid("QVM counter word overlaps its evaluation stack"));
        }
        let qualification = self
            .core
            .counter_functions
            .get(&key)
            .ok_or_else(|| GuestError::invalid("QVM counter evaluation lost its qualification"))?;
        self.core.counter = Some(CounterState {
            address,
            value: initial,
            stack_start,
            stack_end: self.core.program_stack,
            functions: qualification.functions.clone(),
            ranges: qualification.ranges.clone(),
            remaining: 100_000,
        });
        // Runs through invoke_inner (not invoke) so nested evaluation from host
        // code works like the donor; the global counter guards the scope.
        let result = self.invoke_inner(host, args, entry, None);
        let value = self.core.counter.as_ref().map(|counter| counter.value);
        self.core.counter = None;
        result?;
        value.ok_or_else(|| GuestError::invalid("QVM counter evaluation lost its isolated word"))
    }
}

struct QualifiedReadOnly {
    region: QvmRegionEvaluation,
    inputs: Vec<i32>,
    frame_size: usize,
    stack: Option<QvmEvaluationStack>,
}

/// Qualify a read-only region evaluation against the program, caching the
/// frame size on the core. Shared by root entry and recursive host entry.
fn qualify_evaluation(
    core: &mut QvmCore,
    program: &QvmProgram,
    entry: usize,
    evaluation: QvmReadOnlyEvaluation,
) -> Result<QualifiedReadOnly, GuestError> {
    let key = RegionEvalKey {
        instruction: entry,
        entry: evaluation.region.entry,
        join: evaluation.region.join,
        inputs: evaluation.region.inputs.clone(),
        result: evaluation.region.result,
    };
    let frame_size = match core.read_only_regions.get(&key) {
        Some(size) => *size,
        None => {
            let size = qualify_qvm_region_evaluation(
                &program.instructions,
                entry,
                &evaluation.region,
                QvmRegionAccess::ReadOnly,
            )?;
            core.read_only_regions.insert(key, size);
            size
        }
    };
    if evaluation.inputs.len() != evaluation.region.inputs.len() {
        return Err(GuestError::invalid(
            "Read-only QVM region live-ins differ from its qualified frame",
        ));
    }
    Ok(QualifiedReadOnly {
        region: evaluation.region,
        inputs: evaluation.inputs,
        frame_size: frame_size as usize,
        stack: evaluation.stack,
    })
}

struct DriveEvaluation {
    region: QvmRegionEvaluation,
    inputs: Vec<i32>,
    frame_size: usize,
}

struct Drive<'a, 'c, 'o> {
    ctx: &'a mut InterpCtx<'c>,
    host: &'a dyn QvmSystemCallHandler,
    ops: &'o mut OperandStack,
    sp: usize,
    pc: i32,
    entry: usize,
    scope: Option<u64>,
    source_call: Option<u64>,
    source_operand_depth: usize,
    evaluation: Option<DriveEvaluation>,
    eval_stack: usize,
    evaluation_started: bool,
    returns: Option<Vec<(usize, i32)>>,
    profile_symbol: Option<usize>,
    trace: u8,
    break_function: i32,
    debug: bool,
    evaluation_floor: Option<usize>,
}

#[allow(clippy::too_many_arguments)]
fn run_loop(
    ctx: &mut InterpCtx<'_>,
    host: &dyn QvmSystemCallHandler,
    ops: &mut OperandStack,
    args: &[i32],
    entry: usize,
    source_call: Option<u64>,
    parent_scope: Option<u64>,
    read_only: Option<QualifiedReadOnly>,
    profile: Option<QvmExecutionProfile>,
) -> Result<i32, GuestError> {
    if source_call.is_none() {
        if entry == 0 {
            if args.len() != 10 {
                return Err(GuestError::invalid("QVM vmMain requires ten public argument words"));
            }
        } else {
            if args.len() > QVM_MAX_PRIVATE_ARGUMENT_WORDS {
                return Err(GuestError::invalid("QVM private call exceeds OP_ARG argument capacity"));
            }
            if ctx
                .program
                .instructions
                .get(entry)
                .map(|instruction| instruction.opcode)
                != Some(QvmOpcode::OpEnter)
            {
                return Err(GuestError::invalid(
                    "QVM private invocation requires an original function entry",
                ));
            }
        }
    }
    let profile = profile.unwrap_or_else(|| {
        ctx.core
            .registration
            .as_ref()
            .map(|registration| registration.execution_profile())
            .unwrap_or(QvmExecutionProfile::Release)
    });
    let debug = matches!(profile, QvmExecutionProfile::Debug { .. });
    let (trace, break_function) = match profile {
        QvmExecutionProfile::Release => (0, 0),
        QvmExecutionProfile::Debug { trace, break_function } => (trace, break_function),
    };
    if source_call.is_none() {
        if let Some(registration) = ctx.core.registration.as_ref() {
            registration.print_call(args.first().copied().unwrap_or(0));
        }
    }
    let evaluation_floor = if let Some(counter) = ctx.core.counter.as_ref() {
        Some(counter.stack_start)
    } else if let Some(qualified) = read_only.as_ref() {
        Some(ctx.core.evaluation_stack_start(qualified.stack)?)
    } else {
        None
    };
    let entry_stack = ctx.core.program_stack;
    let previous_call_level = ctx.core.call_level;
    let previous_debug = ctx.core.debug;
    let argument_words = 10.max(args.len());
    let sp = if let Some(call_id) = source_call {
        let position = find_call(&ctx.core.calls, call_id)
            .ok_or_else(|| GuestError::invalid("QVM function invocation has expired"))?;
        ctx.core.calls[position].stack
    } else {
        let raw = entry_stack as i64 - 8 - argument_words as i64 * 4;
        check_stack(ctx.program.source.as_str(), raw, debug)?
    };
    if let Some(floor) = evaluation_floor {
        if sp < floor {
            return Err(GuestError::invalid("QVM evaluation stack would overlap source data"));
        }
    }
    ctx.core.debug = debug;
    let eval_stack = sp;
    let scope = source_call.or(parent_scope);
    let evaluation = if let Some(qualified) = read_only {
        Some(DriveEvaluation {
            region: qualified.region,
            inputs: qualified.inputs,
            frame_size: qualified.frame_size,
        })
    } else if let Some(call_id) = source_call {
        let position = find_call(&ctx.core.calls, call_id)
            .ok_or_else(|| GuestError::invalid("QVM function invocation has expired"))?;
        ctx.core.calls[position]
            .evaluation
            .as_ref()
            .map(|evaluation| DriveEvaluation {
                region: evaluation.region.clone(),
                inputs: evaluation.inputs.clone(),
                frame_size: evaluation.frame_size,
            })
    } else {
        None
    };
    let source_operand_depth = source_call
        .and_then(|call_id| find_call(&ctx.core.calls, call_id))
        .map_or(0, |position| ctx.core.calls[position].operand_depth);
    let mut returns: Option<Vec<(usize, i32)>> = None;
    if ctx.core.semantics == QvmSemantics::Compiled {
        let return_pc = source_call
            .and_then(|call_id| find_call(&ctx.core.calls, call_id))
            .map_or(-1, |position| ctx.core.calls[position].return_pc);
        returns = Some(vec![(sp, return_pc)]);
    }
    if source_call.is_none() {
        write_word(ctx.core, sp, sp, -1)?;
        write_word(ctx.core, sp, sp + 4, 0)?;
        for index in 0..argument_words {
            write_word(ctx.core, sp, sp + 8 + index * 4, args.get(index).copied().unwrap_or(0))?;
        }
        ctx.core.call_level = 0;
        if let Some(registration) = ctx.core.registration.as_ref() {
            registration.debug(0);
        }
        let hook = ctx
            .core
            .hooks
            .get(&entry)
            .filter(|binding| {
                binding.scope == HookScope::Invocations && evaluation.is_none() && ctx.core.counter.is_none()
            })
            .map(|binding| Rc::clone(&binding.hook));
        if let Some(hook) = hook {
            ctx.core.program_stack = sp.saturating_sub(4);
            let result = intercept(
                ctx,
                host,
                ops,
                sp,
                -1,
                entry,
                Some(hook),
                Vec::new(),
                Some(argument_words * 4),
                parent_scope,
            );
            ctx.core.program_stack = entry_stack;
            return result;
        }
    }
    let pc = target_pc(ctx.program, entry)?;
    let profile_symbol = if debug {
        ctx.core.symbols.function_symbol_index(0)?
    } else {
        None
    };
    let mut drive = Drive {
        ctx,
        host,
        ops,
        sp,
        pc,
        entry,
        scope,
        source_call,
        source_operand_depth,
        evaluation,
        eval_stack,
        evaluation_started: false,
        returns,
        profile_symbol,
        trace,
        break_function,
        debug,
        evaluation_floor,
    };
    let result = drive_loop(&mut drive);
    drive.ctx.core.program_stack = entry_stack;
    if drive.source_call.is_some() || parent_scope.is_some() {
        drive.ctx.core.debug = previous_debug;
        drive.ctx.core.call_level = previous_call_level;
    }
    result
}

fn drive_loop(drive: &mut Drive<'_, '_, '_>) -> Result<i32, GuestError> {
    loop {
        if let Some(counter) = drive.ctx.core.counter.as_mut() {
            counter.remaining -= 1;
            let pc = drive.pc;
            if counter.remaining < 0 || !counter.ranges.iter().any(|(start, end)| pc >= *start && pc < *end) {
                return Err(GuestError::invalid(
                    "QVM counter evaluation escaped its admitted original functions or instruction budget",
                ));
            }
        }
        let in_counter = drive.ctx.core.counter.is_some();
        let owner = if in_counter { None } else { drive.scope };
        if let Some(evaluation) = drive.evaluation.as_ref() {
            let join_pc = target_pc(drive.ctx.program, evaluation.region.join)?;
            if drive.sp == drive.eval_stack.wrapping_sub(evaluation.frame_size) && drive.pc == join_pc {
                if drive.ops.count() != drive.source_operand_depth {
                    return Err(GuestError::invalid("QVM region evaluation lost its caller operands"));
                }
                return match evaluation.region.result {
                    None => Ok(0),
                    Some(offset) => read_word(drive.ctx.core, drive.sp + offset),
                };
            }
        }
        if let Some(owner_id) = owner {
            if region_step(drive, owner_id)? {
                continue;
            }
        }
        if drive.debug {
            if drive.pc < 0 || drive.pc as usize >= drive.ctx.program.code.len() {
                return Err(qvm_drop_error("VM pc out of range"));
            }
            if drive.sp <= drive.ctx.core.memory.len().saturating_sub(0x20000) {
                return Err(qvm_drop_error("VM stack overflow"));
            }
            if !drive.sp.is_multiple_of(4) {
                return Err(qvm_drop_error("VM program stack misaligned"));
            }
        }
        let opcode_value = code_word(drive.ctx.program, drive.pc)?;
        drive.pc += 1;
        let opcode = u8::try_from(opcode_value)
            .ok()
            .and_then(|byte| QvmOpcode::from_u8(byte).ok());
        if drive.profile_symbol.is_some() {
            if drive.trace > 1 {
                let Some(opcode) = opcode else {
                    return Err(qvm_drop_error("Bad VM instruction"));
                };
                let indent = "  ".repeat(drive.ctx.core.call_level.clamp(0, 20) as usize);
                let text = format!("{indent}{} {}\n", drive.ops.count(), opcode.name());
                if let Some(registration) = drive.ctx.core.registration.as_ref() {
                    registration.print(&text);
                }
            }
            if let Some(symbol) = drive.profile_symbol {
                drive.ctx.core.symbols.add_profile_count(symbol, 1);
            }
        }
        let Some(opcode) = opcode else {
            // The release interpreter has no default trap. A return into an
            // operand slot can encounter a non-opcode integer as a nop.
            if drive.debug {
                return Err(qvm_drop_error("Bad VM instruction"));
            }
            continue;
        };
        match opcode {
            QvmOpcode::OpUndef | QvmOpcode::OpIgnore => {
                if drive.debug {
                    return Err(qvm_drop_error("Bad VM instruction"));
                }
            }
            QvmOpcode::OpBreak => {
                if in_counter {
                    return Err(GuestError::invalid(
                        "QVM counter evaluation cannot trigger a debug break",
                    ));
                }
                drive.ctx.core.breaks = drive.ctx.core.breaks.wrapping_add(1);
            }
            QvmOpcode::OpConst => {
                let word = code_word(drive.ctx.program, drive.pc)?;
                drive.pc += 4;
                drive.ops.push(word)?;
            }
            QvmOpcode::OpLocal => {
                let word = code_word(drive.ctx.program, drive.pc)?;
                drive.pc += 4;
                drive.ops.push((drive.sp as i32).wrapping_add(word))?;
            }
            QvmOpcode::OpPush => {
                drive.ops.reserve()?;
            }
            QvmOpcode::OpPop => {
                drive.ops.drop_top()?;
            }
            QvmOpcode::OpEnter => {
                op_enter(drive)?;
            }
            QvmOpcode::OpLeave => {
                if let Some(value) = op_leave(drive)? {
                    return Ok(value);
                }
            }
            QvmOpcode::OpCall => {
                op_call(drive, in_counter)?;
            }
            QvmOpcode::OpJump => {
                let target = drive.ops.pop()?;
                let index = usize::try_from(target).map_err(|_| {
                    GuestError::invalid(format!(
                        "{}: invalid QVM instruction index {target}",
                        drive.ctx.program.source
                    ))
                })?;
                drive.pc = target_pc(drive.ctx.program, index)?;
            }
            QvmOpcode::OpLoad1 => {
                let address = mask_address(drive.ops.peek()?, drive.ctx.program.data_mask);
                counter_narrow_read(drive.ctx.core, address, 1)?;
                let value = drive.ctx.core.memory.get_u8(address)?;
                drive.ops.set(i32::from(value));
            }
            QvmOpcode::OpLoad2 => {
                let address = mask_address(drive.ops.peek()?, drive.ctx.program.data_mask);
                drive.ctx.core.memory.data_view(address, 2)?;
                counter_narrow_read(drive.ctx.core, address, 2)?;
                let value = drive.ctx.core.memory.get_u16(address)?;
                drive.ops.set(i32::from(value));
            }
            QvmOpcode::OpLoad4 => {
                let word = drive.ops.peek()?;
                if drive.debug && word & 3 != 0 {
                    return Err(qvm_drop_error("OP_LOAD4 misaligned"));
                }
                let value = read_word(drive.ctx.core, mask_address(word, drive.ctx.program.data_mask))?;
                drive.ops.set(value);
            }
            QvmOpcode::OpStore1 => {
                let value = drive.ops.pop()?;
                let address = mask_address(drive.ops.pop()?, drive.ctx.program.data_mask);
                let sp = drive.sp;
                let handled = counter_narrow_write(drive.ctx.core, sp, address, 1, value)?;
                if !handled {
                    if in_counter {
                        drive.ctx.core.memory.set_u8_unobserved(address, value as u8)?;
                    } else {
                        drive.ctx.core.memory.set_u8(address, value as u8)?;
                    }
                }
            }
            QvmOpcode::OpStore2 => {
                let value = drive.ops.pop()?;
                let address = mask_address(drive.ops.pop()?, drive.ctx.program.data_mask & !1);
                drive.ctx.core.memory.data_view(address, 2)?;
                let sp = drive.sp;
                let handled = counter_narrow_write(drive.ctx.core, sp, address, 2, value)?;
                if !handled {
                    if in_counter {
                        drive.ctx.core.memory.set_u16_unobserved(address, value as u16)?;
                    } else {
                        drive.ctx.core.memory.set_u16(address, value as u16)?;
                    }
                }
            }
            QvmOpcode::OpStore4 => {
                let value = drive.ops.pop()?;
                let address = mask_address(drive.ops.pop()?, drive.ctx.program.data_mask & !3);
                let sp = drive.sp;
                write_word(drive.ctx.core, sp, address, value)?;
            }
            QvmOpcode::OpArg => {
                let offset = code_word(drive.ctx.program, drive.pc)?;
                drive.pc += 1;
                let value = drive.ops.pop()?;
                let at = drive.sp as i64 + i64::from(offset);
                if at < 0 {
                    return Err(GuestError::memory_fault(
                        "out-of-bounds",
                        0,
                        4,
                        "write",
                        "QVM raw memory range exceeds allocation",
                    ));
                }
                let sp = drive.sp;
                write_word(drive.ctx.core, sp, at as usize, value)?;
            }
            QvmOpcode::OpBlockCopy => {
                op_block_copy(drive, in_counter)?;
            }
            QvmOpcode::OpBcom => {
                if drive.ctx.core.semantics == QvmSemantics::Compiled {
                    let top = drive.ops.peek()?;
                    drive.ops.set(!top);
                } else {
                    drive.ops.complement_previous()?;
                }
            }
            QvmOpcode::OpSex8
            | QvmOpcode::OpSex16
            | QvmOpcode::OpNegi
            | QvmOpcode::OpNegf
            | QvmOpcode::OpCvif
            | QvmOpcode::OpCvfi => {
                let top = drive.ops.peek()?;
                let value = evaluate_unary(opcode, top)?;
                drive.ops.set(value);
            }
            QvmOpcode::OpAdd
            | QvmOpcode::OpSub
            | QvmOpcode::OpDivi
            | QvmOpcode::OpDivu
            | QvmOpcode::OpModi
            | QvmOpcode::OpModu
            | QvmOpcode::OpMuli
            | QvmOpcode::OpMulu
            | QvmOpcode::OpBand
            | QvmOpcode::OpBor
            | QvmOpcode::OpBxor
            | QvmOpcode::OpLsh
            | QvmOpcode::OpRshi
            | QvmOpcode::OpRshu
            | QvmOpcode::OpAddf
            | QvmOpcode::OpSubf
            | QvmOpcode::OpDivf
            | QvmOpcode::OpMulf => {
                let right = drive.ops.pop()?;
                let left = drive.ops.peek()?;
                let value = evaluate_binary(opcode, left, right)?;
                drive.ops.set(value);
            }
            _ if opcode.is_branch() => {
                op_branch(drive, opcode, owner)?;
            }
            _ => {
                if drive.debug {
                    return Err(qvm_drop_error("Bad VM instruction"));
                }
            }
        }
    }
}

fn check_stack(source: &str, raw: i64, debug: bool) -> Result<usize, GuestError> {
    if raw < 0 {
        return Err(GuestError::memory_fault(
            "out-of-bounds",
            0,
            0,
            "access",
            format!("{source}: QVM memory access {raw}+0 exceeds allocation"),
        ));
    }
    if raw % 4 != 0 {
        if debug {
            return Err(qvm_drop_error("VM program stack misaligned"));
        }
        return Err(GuestError::invalid("QVM program stack is misaligned"));
    }
    Ok(raw as usize)
}

fn op_enter(drive: &mut Drive<'_, '_, '_>) -> Result<(), GuestError> {
    if drive.debug {
        drive.profile_symbol = drive.ctx.core.symbols.function_symbol_index(drive.pc)?;
    }
    let size = code_word(drive.ctx.program, drive.pc)?;
    if let Some(floor) = drive.evaluation_floor {
        if size < 0 || size % 4 != 0 || drive.sp as i64 - i64::from(size) < floor as i64 {
            return Err(GuestError::invalid("QVM evaluation stack would overlap source data"));
        }
    }
    drive.sp = check_stack(
        drive.ctx.program.source.as_str(),
        drive.sp as i64 - i64::from(size),
        drive.debug,
    )?;
    drive.pc += 4;
    if drive.evaluation.is_some() && !drive.evaluation_started {
        let entry_pc = target_pc(drive.ctx.program, drive.entry)?;
        if drive.pc - 5 == entry_pc {
            drive.evaluation_started = true;
            let (offsets, inputs, entry) = {
                let evaluation = drive
                    .evaluation
                    .as_ref()
                    .ok_or_else(|| GuestError::invalid("Missing QVM region live-in"))?;
                (
                    evaluation.region.inputs.clone(),
                    evaluation.inputs.clone(),
                    evaluation.region.entry,
                )
            };
            for (index, offset) in offsets.iter().enumerate() {
                let value = inputs
                    .get(index)
                    .copied()
                    .ok_or_else(|| GuestError::invalid("Missing QVM region live-in"))?;
                let sp = drive.sp;
                write_word(drive.ctx.core, sp, sp + *offset, value)?;
            }
            drive.pc = target_pc(drive.ctx.program, entry)?;
        }
    }
    if drive.debug {
        let sp = drive.sp;
        write_word(drive.ctx.core, sp, sp + 4, (sp as i32).wrapping_add(size))?;
        if drive.trace != 0 {
            let symbol = drive.ctx.core.symbols.value_to_symbol(drive.pc - 5)?;
            let indent = "  ".repeat(drive.ctx.core.call_level.clamp(0, 20) as usize);
            if let Some(registration) = drive.ctx.core.registration.as_ref() {
                registration.print(&format!("{indent}---> {symbol}\n"));
            }
            if drive.break_function != 0 && drive.pc - 5 == drive.break_function {
                drive.ctx.core.breaks = drive.ctx.core.breaks.wrapping_add(1);
            }
            drive.ctx.core.call_level = drive.ctx.core.call_level.wrapping_add(1);
        }
    }
    Ok(())
}

fn op_leave(drive: &mut Drive<'_, '_, '_>) -> Result<Option<i32>, GuestError> {
    let size = code_word(drive.ctx.program, drive.pc)?;
    drive.sp = check_stack(
        drive.ctx.program.source.as_str(),
        drive.sp as i64 + i64::from(size),
        drive.debug,
    )?;
    let sp = drive.sp;
    let target = match drive.returns.as_mut() {
        None => read_word(drive.ctx.core, sp)?,
        Some(returns) => {
            let (saved_sp, target) = returns
                .pop()
                .ok_or_else(|| GuestError::invalid("QVM function returned with an invalid program stack"))?;
            if saved_sp != sp {
                return Err(GuestError::invalid(
                    "QVM function returned with an invalid program stack",
                ));
            }
            target
        }
    };
    if drive.debug {
        drive.profile_symbol = drive.ctx.core.symbols.function_symbol_index(target)?;
        if drive.trace != 0 {
            drive.ctx.core.call_level = drive.ctx.core.call_level.wrapping_sub(1);
            let symbol = drive.ctx.core.symbols.value_to_symbol(target)?;
            let indent = "  ".repeat(drive.ctx.core.call_level.clamp(0, 20) as usize);
            if let Some(registration) = drive.ctx.core.registration.as_ref() {
                registration.print(&format!("{indent}<--- {symbol}\n"));
            }
        }
    }
    if let Some(call_id) = drive.source_call {
        let position = find_call(&drive.ctx.core.calls, call_id)
            .ok_or_else(|| GuestError::invalid("QVM function invocation has expired"))?;
        let (stack, return_pc) = {
            let call = &drive.ctx.core.calls[position];
            (call.stack, call.return_pc)
        };
        let returns_empty = drive.returns.as_ref().is_none_or(|returns| returns.is_empty());
        if sp == stack && returns_empty {
            if target != return_pc || drive.ops.count() != drive.source_operand_depth + 1 {
                return Err(GuestError::invalid(
                    "QVM function returned with an invalid caller stack",
                ));
            }
            return Ok(Some(drive.ops.pop()?));
        }
    }
    if target == -1 {
        return Ok(Some(drive.ops.result()?));
    }
    drive.pc = target;
    Ok(None)
}

#[allow(clippy::too_many_lines)]
fn op_call(drive: &mut Drive<'_, '_, '_>, in_counter: bool) -> Result<(), GuestError> {
    let return_pc = drive.pc;
    let sp = drive.sp;
    write_word(drive.ctx.core, sp, sp, drive.pc)?;
    let target = drive.ops.pop()?;
    if target >= 0 {
        let index = usize::try_from(target).map_err(|_| {
            GuestError::invalid(format!(
                "{}: invalid QVM instruction index {target}",
                drive.ctx.program.source
            ))
        })?;
        if in_counter {
            let admitted = drive
                .ctx
                .core
                .counter
                .as_ref()
                .is_some_and(|counter| counter.functions.contains(&index));
            if !admitted {
                return Err(GuestError::invalid(
                    "QVM counter evaluation called an undeclared source function",
                ));
            }
        }
        let (hook, observers) = if in_counter {
            (None, Vec::new())
        } else {
            resolve_call_target(drive, sp, return_pc, index)?
        };
        if hook.is_none() && observers.is_empty() {
            if let Some(returns) = drive.returns.as_mut() {
                returns.push((sp, return_pc));
            }
            drive.pc = target_pc(drive.ctx.program, index)?;
            return Ok(());
        }
        let saved_call_level = drive.ctx.core.call_level;
        let saved_stack = drive.ctx.core.program_stack;
        drive.ctx.core.program_stack = sp.saturating_sub(4);
        let scope = drive.scope;
        let result = intercept(
            &mut *drive.ctx,
            drive.host,
            &mut *drive.ops,
            sp,
            return_pc,
            index,
            hook,
            observers,
            None,
            scope,
        );
        drive.ctx.core.call_level = saved_call_level;
        drive.ctx.core.program_stack = saved_stack;
        let value = result?;
        drive.ops.push(value)?;
        drive.pc = if drive.returns.is_none() {
            read_word(drive.ctx.core, sp)?
        } else {
            return_pc
        };
        return Ok(());
    }
    if in_counter {
        return Err(GuestError::invalid(
            "QVM counter evaluation cannot call an engine service",
        ));
    }
    if drive.trace != 0 {
        let indent = "  ".repeat(drive.ctx.core.call_level.clamp(0, 20) as usize);
        if let Some(registration) = drive.ctx.core.registration.as_ref() {
            registration.print(&format!("{indent}---> systemcall({})\n", -1 - target));
        }
    }
    let saved_call_level = drive.ctx.core.call_level;
    drive.ctx.core.program_stack = sp.saturating_sub(4);
    let saved_frame = if drive.debug {
        Some(read_word(drive.ctx.core, sp + 4)?)
    } else {
        None
    };
    write_word(drive.ctx.core, sp, sp + 4, -1 - target)?;
    let value = trap(drive, sp, drive.scope)?;
    if let Some(frame) = saved_frame {
        write_word(drive.ctx.core, sp, sp + 4, frame)?;
    }
    drive.ops.push(value)?;
    drive.pc = if drive.returns.is_none() {
        read_word(drive.ctx.core, sp)?
    } else {
        return_pc
    };
    drive.ctx.core.call_level = saved_call_level;
    if drive.trace != 0 {
        let symbol = drive.ctx.core.symbols.value_to_symbol(drive.pc)?;
        let indent = "  ".repeat(drive.ctx.core.call_level.clamp(0, 20) as usize);
        if let Some(registration) = drive.ctx.core.registration.as_ref() {
            registration.print(&format!("{indent}<--- {symbol}\n"));
        }
    }
    Ok(())
}

fn resolve_call_target(
    drive: &mut Drive<'_, '_, '_>,
    sp: usize,
    return_pc: i32,
    index: usize,
) -> Result<(Option<QvmFunctionHook>, Vec<QvmFunctionObserver>), GuestError> {
    let binding = drive.ctx.core.hooks.get(&index).map(|binding| Rc::clone(&binding.hook));
    let observers: Vec<QvmFunctionObserver> = drive
        .ctx
        .core
        .observers
        .get(&index)
        .map(|entries| {
            entries
                .iter()
                .filter(|entry| entry.active)
                .map(|entry| Rc::clone(&entry.observe))
                .collect()
        })
        .unwrap_or_default();
    let hook = match binding {
        Some(hook) => Some(hook),
        None => {
            let Some(resolver) = drive.ctx.core.resolver.clone() else {
                return Ok((None, observers));
            };
            let first = read_word(drive.ctx.core, sp + 8)?;
            let caller = source_instruction(drive.ctx.program, return_pc - 1)?;
            let bytes = source_argument_bytes(drive.ctx, caller)?;
            let mut arg_words = Vec::with_capacity(bytes / 4);
            for slot in 0..bytes / 4 {
                arg_words.push(read_word(drive.ctx.core, sp + 8 + slot * 4)?);
            }
            let resolved = resolver(index, first, &arg_words);
            if resolved.is_some()
                && code_word(drive.ctx.program, target_pc(drive.ctx.program, index)?)? != QvmOpcode::OpEnter as i32
            {
                return Err(GuestError::invalid(
                    "QVM resolver requires a function entry instruction",
                ));
            }
            resolved
        }
    };
    Ok((hook, observers))
}

fn op_block_copy(drive: &mut Drive<'_, '_, '_>, in_counter: bool) -> Result<(), GuestError> {
    if in_counter {
        return Err(GuestError::invalid("QVM counter evaluation cannot copy source memory"));
    }
    let source = drive.ops.pop()?;
    let destination = drive.ops.pop()?;
    let count = code_word(drive.ctx.program, drive.pc)?;
    drive.pc += 4;
    if drive.ctx.core.semantics == QvmSemantics::Interpreted {
        let mask = drive.ctx.program.data_mask;
        let from = mask_address(source, mask) as i32;
        let to = mask_address(destination, mask) as i32;
        let mask32 = mask as i32;
        let source_count = (((from as i64 + i64::from(count)) as i32) & mask32).wrapping_sub(from);
        let copied = (((to as i64 + i64::from(source_count)) as i32) & mask32).wrapping_sub(to);
        if (from | to | copied) & 3 != 0 {
            return Err(qvm_drop_error("OP_BLOCK_COPY not dword aligned"));
        }
        for word in (0..(copied >> 2)).rev() {
            let value = read_word(drive.ctx.core, (from + word * 4) as usize)?;
            let sp = drive.sp;
            write_word(drive.ctx.core, sp, (to + word * 4) as usize, value)?;
        }
        return Ok(());
    }
    let mask = drive.ctx.program.data_mask as i64;
    if count < 0
        || source < 0
        || destination < 0
        || i64::from(source) + i64::from(count) > mask
        || i64::from(destination) + i64::from(count) > mask
    {
        return Err(qvm_drop_error("OP_BLOCK_COPY out of range"));
    }
    // The compiled VM accepts unaligned literals with a checked byte copy.
    drive
        .ctx
        .core
        .memory
        .copy_bytes(destination as usize, source as usize, count as usize)?;
    Ok(())
}

fn op_branch(drive: &mut Drive<'_, '_, '_>, opcode: QvmOpcode, owner: Option<u64>) -> Result<(), GuestError> {
    let right = drive.ops.pop()?;
    let left = drive.ops.pop()?;
    let target = code_word(drive.ctx.program, drive.pc)?;
    let taken = evaluate_branch(opcode, left, right)?;
    let decide = match owner {
        Some(owner_id) => {
            let position = find_call(&drive.ctx.core.calls, owner_id);
            position.and_then(|position| {
                drive.ctx.core.calls[position]
                    .branches
                    .as_ref()
                    .and_then(|branches| branches.get(&(drive.pc - 1)))
                    .map(Rc::clone)
            })
        }
        None => None,
    };
    let Some(decide) = decide else {
        drive.pc = if taken { target } else { drive.pc + 4 };
        return Ok(());
    };
    let owner_id = owner.ok_or_else(|| GuestError::invalid("QVM branch invocation has expired"))?;
    let position = find_call(&drive.ctx.core.calls, owner_id)
        .ok_or_else(|| GuestError::invalid("QVM branch invocation has expired"))?;
    if !drive.ctx.core.calls[position].active {
        return Err(GuestError::invalid("QVM branch invocation has expired"));
    }
    let sp = drive.sp;
    let previous_stack = drive.ctx.core.program_stack;
    drive.ctx.core.program_stack = sp.saturating_sub(4);
    let mut control = HostControl {
        ctx: &mut *drive.ctx,
        host: drive.host,
        scope: drive.scope,
    };
    let mut cancel = |scope: &QvmCancellationScope| control.cancel_function(scope);
    let chosen = decide(taken, &mut cancel);
    drive.ctx.core.program_stack = previous_stack;
    check_chain(&drive.ctx.core.calls, drive.scope)?;
    drive.ctx.core.live()?;
    let position = find_call(&drive.ctx.core.calls, owner_id)
        .ok_or_else(|| GuestError::invalid("QVM branch invocation is no longer current"))?;
    if !drive.ctx.core.calls[position].active {
        return Err(GuestError::invalid("QVM branch invocation is no longer current"));
    }
    drive.pc = if chosen { target } else { drive.pc + 4 };
    Ok(())
}

/// Run region completion/entry checks; returns true when the caller must
/// continue the loop (a region was skipped).
fn region_step(drive: &mut Drive<'_, '_, '_>, owner_id: u64) -> Result<bool, GuestError> {
    let position = find_call(&drive.ctx.core.calls, owner_id)
        .ok_or_else(|| GuestError::invalid("QVM region invocation has expired"))?;
    let call = &drive.ctx.core.calls[position];
    if !call.active {
        return Ok(false);
    }
    if call.regions.is_empty() {
        return Ok(false);
    }
    let pc = drive.pc;
    let sp = drive.sp;
    if let Some(index) = call.region_joins.get(&pc).copied() {
        let region = &drive.ctx.core.calls[position].regions[index];
        if sp == region.stack && region.state == RegionState::Running {
            drive.ctx.core.calls[position].regions[index].state = RegionState::Complete;
            region_callback(drive, owner_id, index, true)?;
        }
    }
    let position = find_call(&drive.ctx.core.calls, owner_id)
        .ok_or_else(|| GuestError::invalid("QVM region invocation has expired"))?;
    if let Some(index) = drive.ctx.core.calls[position].region_entries.get(&pc).copied() {
        let region = &drive.ctx.core.calls[position].regions[index];
        if sp == region.stack {
            if region.state != RegionState::Ready {
                return Err(GuestError::invalid("QVM region can execute only once per invocation"));
            }
            drive.ctx.core.calls[position].regions[index].state = RegionState::Running;
            let execute = region_callback(drive, owner_id, index, false)?;
            if !execute {
                let join = drive.ctx.core.calls[position].regions[index].join_pc;
                drive.ctx.core.calls[position].regions[index].state = RegionState::Complete;
                drive.pc = join;
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn region_callback(
    drive: &mut Drive<'_, '_, '_>,
    owner_id: u64,
    index: usize,
    complete: bool,
) -> Result<bool, GuestError> {
    let position = find_call(&drive.ctx.core.calls, owner_id)
        .ok_or_else(|| GuestError::invalid("QVM region invocation has expired"))?;
    let (run, completed, region_stack, frame_size, operand_depth) = {
        let call = &drive.ctx.core.calls[position];
        if !call.active {
            return Err(GuestError::invalid("QVM region invocation has expired"));
        }
        let region = &call.regions[index];
        (
            Rc::clone(&region.binding.run),
            region.binding.completed.as_ref().map(Rc::clone),
            region.stack,
            region.frame_size,
            call.operand_depth,
        )
    };
    if drive.sp != region_stack || drive.ops.count() != operand_depth {
        return Err(GuestError::invalid(
            "QVM region lost its original frame or operand boundary",
        ));
    }
    let previous_stack = drive.ctx.core.program_stack;
    drive.ctx.core.program_stack = region_stack.saturating_sub(4);
    let mut control = QvmRegionControl {
        control: HostControl {
            ctx: &mut *drive.ctx,
            host: drive.host,
            scope: drive.scope,
        },
        region_stack,
        frame_size,
    };
    let result = if complete {
        if let Some(completed) = completed {
            completed(&mut control)?;
        }
        Ok(true)
    } else {
        match run(&mut control)? {
            QvmRegionDecision::Execute => Ok(true),
            QvmRegionDecision::Skip => Ok(false),
        }
    };
    drive.ctx.core.program_stack = previous_stack;
    result
}

fn trap(drive: &mut Drive<'_, '_, '_>, sp: usize, scope: Option<u64>) -> Result<i32, GuestError> {
    let length = drive.ctx.core.memory.len();
    let words = drive.ctx.core.memory.data_view(sp + 4, length - sp - 4)?;
    let memory = drive.ctx.core.memory.clone();
    let mut call = QvmSyscall {
        words,
        memory: memory.clone(),
        guest: memory,
        control: HostControl {
            ctx: &mut *drive.ctx,
            host: drive.host,
            scope,
        },
    };
    let value = drive.host.handle_syscall(&mut call)?;
    drop(call);
    check_chain(&drive.ctx.core.calls, scope)?;
    drive.ctx.core.live()?;
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
fn intercept(
    ctx: &mut InterpCtx<'_>,
    host: &dyn QvmSystemCallHandler,
    ops: &mut OperandStack,
    sp: usize,
    return_pc: i32,
    entry: usize,
    hook: Option<QvmFunctionHook>,
    observers: Vec<QvmFunctionObserver>,
    argument_bytes: Option<usize>,
    parent: Option<u64>,
) -> Result<i32, GuestError> {
    let call_id = ctx.core.next_call_id;
    ctx.core.next_call_id += 1;
    let operand_depth = ops.count();
    let caller_instruction = if return_pc < 0 {
        None
    } else {
        Some(source_instruction(ctx.program, return_pc - 1)?)
    };
    let bytes = match argument_bytes {
        Some(bytes) => bytes,
        None => {
            let caller =
                caller_instruction.ok_or_else(|| GuestError::invalid("QVM host call lost its argument frame"))?;
            let mut nested = InterpCtx {
                core: &mut *ctx.core,
                program: ctx.program,
            };
            source_argument_bytes(&mut nested, caller)?
        }
    };
    ctx.core.memory.data_view(sp + 8, bytes)?;
    let words = ctx.core.memory.data_view(sp + 8, bytes)?;
    ctx.core.calls.push(ActiveCall {
        id: call_id,
        stack: sp,
        return_pc,
        operand_depth,
        parent,
        active: true,
        branches: None,
        regions: Vec::new(),
        region_entries: HashMap::new(),
        region_joins: HashMap::new(),
        evaluation: None,
        cancellation: None,
    });
    for observer in &observers {
        let mut observation = QvmFunctionObservation {
            instruction_index: entry,
            words: words.clone(),
            control: HostControl {
                ctx: &mut *ctx,
                host,
                scope: Some(call_id),
            },
        };
        let result = observer(&mut observation);
        drop(observation);
        result?;
    }
    let mut call = QvmFunctionCall {
        instruction_index: entry,
        caller_instruction,
        words,
        memory: ctx.core.memory.clone(),
        guest: ctx.core.memory.clone(),
        control: HostControl {
            ctx: &mut *ctx,
            host,
            scope: Some(call_id),
        },
        ops: &mut *ops,
        call_id,
        proceeded: false,
    };
    let outcome = match hook {
        Some(hook) => hook(&mut call),
        None => call.proceed(),
    };
    drop(call);
    match outcome {
        Ok(value) => {
            check_chain(&ctx.core.calls, Some(call_id))?;
            finish_call(ctx.core, call_id);
            Ok(value)
        }
        Err(error) => {
            let requested = ctx
                .core
                .calls
                .iter()
                .find(|call| call.id == call_id)
                .and_then(|call| call.cancellation.as_ref())
                .is_some_and(|cancellation| cancellation.requested);
            finish_call(ctx.core, call_id);
            if requested && is_cancel_for(&error, call_id) {
                ops.truncate(operand_depth)?;
                let floor = sp;
                write_word(ctx.core, floor, sp, return_pc)?;
                return Ok(0);
            }
            Err(error)
        }
    }
}

fn finish_call(core: &mut QvmCore, call_id: u64) {
    if let Some(position) = find_call(&core.calls, call_id) {
        core.calls[position].active = false;
        core.calls[position].branches = None;
        core.calls[position].regions.clear();
        core.calls[position].region_entries.clear();
        core.calls[position].region_joins.clear();
        if let Some(cancellation) = core.calls[position].cancellation.take() {
            core.scopes.remove(&cancellation.scope_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn assemble(program: Vec<(QvmOpcode, QvmOperand)>, data_length: usize, init: &[u8]) -> QvmImage {
        let mut offset = 0usize;
        let instructions = program
            .into_iter()
            .map(|(opcode, operand)| {
                let instruction = QvmInstruction {
                    byte_offset: offset,
                    opcode,
                    operand,
                };
                offset += 1 + opcode.operand_width();
                instruction
            })
            .collect();
        let allocated = data_length.max(256).next_power_of_two();
        let mut initialized_data = vec![0; data_length];
        initialized_data[..init.len().min(data_length)].copy_from_slice(&init[..init.len().min(data_length)]);
        QvmImage {
            source: "test".to_string(),
            instructions,
            code_offset: 0,
            code_length: offset,
            data_length,
            literal_length: 0,
            bss_length: 0,
            initialized_data,
            allocated_data_length: allocated,
            data_mask: (allocated - 1) as i32,
        }
    }

    fn runner(program: Vec<(QvmOpcode, QvmOperand)>, semantics: QvmSemantics) -> QvmInterpreter {
        let image = assemble(program, 256, &[]);
        QvmInterpreter::new(&image, QvmAllocationProfile::Unaccounted, None, semantics).unwrap()
    }

    fn word(value: i32) -> QvmOperand {
        QvmOperand::Word(value)
    }

    struct TrapHost {
        seen: Rc<RefCell<Vec<i32>>>,
    }

    impl QvmSystemCallHandler for TrapHost {
        fn handle_syscall(&self, call: &mut QvmSyscall<'_, '_>) -> Result<i32, GuestError> {
            let trap = call.words.get_i32(0)?;
            self.seen.borrow_mut().push(trap);
            Ok(trap * 10 + 7)
        }
    }

    #[test]
    fn arithmetic_and_return() {
        use QvmOpcode as O;
        let mut vm = runner(
            vec![
                (O::OpEnter, word(8)),
                (O::OpConst, word(40)),
                (O::OpConst, word(2)),
                (O::OpAdd, QvmOperand::None),
                (O::OpLeave, word(8)),
            ],
            QvmSemantics::Interpreted,
        );
        let host = TrapHost {
            seen: Rc::new(RefCell::new(Vec::new())),
        };
        assert_eq!(vm.invoke(&host, &[0; 10], 0, None).unwrap(), 42);
        assert_eq!(vm.stack_pointer(), 256);
        assert!(!vm.is_active());
    }

    #[test]
    fn negative_call_traps_with_words_view() {
        use QvmOpcode as O;
        let mut vm = runner(
            vec![
                (O::OpEnter, word(8)),
                (O::OpConst, word(-1)),
                (O::OpCall, QvmOperand::None),
                (O::OpLeave, word(8)),
            ],
            QvmSemantics::Interpreted,
        );
        let seen = Rc::new(RefCell::new(Vec::new()));
        let host = TrapHost { seen: Rc::clone(&seen) };
        assert_eq!(vm.invoke(&host, &[0; 10], 0, None).unwrap(), 7);
        assert_eq!(*seen.borrow(), vec![0]);
    }

    #[test]
    fn branches_jump_to_indices() {
        use QvmOpcode as O;
        let program = vec![
            (O::OpEnter, word(8)),
            (O::OpConst, word(5)),
            (O::OpConst, word(5)),
            (O::OpEq, word(6)),
            (O::OpConst, word(1)),
            (O::OpLeave, word(8)),
            (O::OpConst, word(2)),
            (O::OpLeave, word(8)),
        ];
        let mut vm = runner(program, QvmSemantics::Interpreted);
        let host = TrapHost {
            seen: Rc::new(RefCell::new(Vec::new())),
        };
        assert_eq!(vm.invoke(&host, &[0; 10], 0, None).unwrap(), 2);
    }

    #[test]
    fn locals_round_trip_through_memory() {
        use QvmOpcode as O;
        let mut vm = runner(
            vec![
                (O::OpEnter, word(12)),
                (O::OpLocal, word(8)),
                (O::OpConst, word(0x1234_5678)),
                (O::OpStore4, QvmOperand::None),
                (O::OpLocal, word(8)),
                (O::OpLoad4, QvmOperand::None),
                (O::OpLeave, word(12)),
            ],
            QvmSemantics::Interpreted,
        );
        let host = TrapHost {
            seen: Rc::new(RefCell::new(Vec::new())),
        };
        assert_eq!(vm.invoke(&host, &[0; 10], 0, None).unwrap(), 0x1234_5678);
    }

    #[test]
    fn direct_calls_link_and_return() {
        use QvmOpcode as O;
        let mut vm = runner(
            vec![
                (O::OpEnter, word(8)),
                (O::OpConst, word(4)),
                (O::OpCall, QvmOperand::None),
                (O::OpLeave, word(8)),
                (O::OpEnter, word(8)),
                (O::OpConst, word(7)),
                (O::OpLeave, word(8)),
            ],
            QvmSemantics::Compiled,
        );
        let host = TrapHost {
            seen: Rc::new(RefCell::new(Vec::new())),
        };
        assert_eq!(vm.invoke(&host, &[0; 10], 0, None).unwrap(), 7);
    }

    #[test]
    fn hooks_replace_or_wrap_the_body() {
        use QvmOpcode as O;
        let program = || {
            vec![
                (O::OpEnter, word(8)),
                (O::OpConst, word(4)),
                (O::OpCall, QvmOperand::None),
                (O::OpLeave, word(8)),
                (O::OpEnter, word(8)),
                (O::OpConst, word(7)),
                (O::OpLeave, word(8)),
            ]
        };
        let host = TrapHost {
            seen: Rc::new(RefCell::new(Vec::new())),
        };
        let mut vm = runner(program(), QvmSemantics::Interpreted);
        let token = vm.bind_function(4, Rc::new(|_call| Ok(99))).unwrap();
        assert_eq!(vm.invoke(&host, &[0; 10], 0, None).unwrap(), 99);
        vm.unbind_function(token);
        let mut vm = runner(program(), QvmSemantics::Interpreted);
        vm.bind_function(
            4,
            Rc::new(|call| {
                let inner = call.proceed()?;
                Ok(inner * 2)
            }),
        )
        .unwrap();
        assert_eq!(vm.invoke(&host, &[0; 10], 0, None).unwrap(), 14);
    }

    #[test]
    fn observers_read_caller_arguments() {
        use QvmOpcode as O;
        let mut vm = runner(
            vec![
                (O::OpEnter, word(16)),
                (O::OpConst, word(5)),
                (O::OpArg, QvmOperand::Byte(8)),
                (O::OpConst, word(6)),
                (O::OpCall, QvmOperand::None),
                (O::OpLeave, word(16)),
                (O::OpEnter, word(8)),
                (O::OpConst, word(0)),
                (O::OpLeave, word(8)),
            ],
            QvmSemantics::Interpreted,
        );
        let got = Rc::new(RefCell::new(-1));
        let got_clone = Rc::clone(&got);
        let token = vm
            .observe_function(
                6,
                Rc::new(move |call| {
                    *got_clone.borrow_mut() = call.argument(0)?;
                    Ok(())
                }),
            )
            .unwrap();
        let host = TrapHost {
            seen: Rc::new(RefCell::new(Vec::new())),
        };
        assert_eq!(vm.invoke(&host, &[0; 10], 0, None).unwrap(), 0);
        assert_eq!(*got.borrow(), 5);
        vm.unobserve_function(token);
        assert!(vm.observe_function(1, Rc::new(|_| Ok(()))).is_err());
    }

    #[test]
    fn cancellation_returns_zero_from_the_call() {
        use QvmOpcode as O;
        let mut vm = runner(
            vec![
                (O::OpEnter, word(8)),
                (O::OpConst, word(4)),
                (O::OpCall, QvmOperand::None),
                (O::OpLeave, word(8)),
                (O::OpEnter, word(8)),
                (O::OpConst, word(7)),
                (O::OpLeave, word(8)),
            ],
            QvmSemantics::Interpreted,
        );
        vm.bind_function(
            4,
            Rc::new(|call| {
                let scope = call.cancellation_scope()?;
                Err(call.cancel_function(&scope))
            }),
        )
        .unwrap();
        let host = TrapHost {
            seen: Rc::new(RefCell::new(Vec::new())),
        };
        assert_eq!(vm.invoke(&host, &[0; 10], 0, None).unwrap(), 0);
    }

    #[test]
    fn invocation_hooks_override_branches() {
        use QvmOpcode as O;
        let mut vm = runner(
            vec![
                (O::OpEnter, word(8)),
                (O::OpConst, word(1)),
                (O::OpConst, word(2)),
                (O::OpEq, word(6)),
                (O::OpConst, word(10)),
                (O::OpLeave, word(8)),
                (O::OpConst, word(20)),
                (O::OpLeave, word(8)),
            ],
            QvmSemantics::Interpreted,
        );
        vm.bind_invocation(
            0,
            Rc::new(|call| {
                call.branches(vec![QvmBranchBinding {
                    instruction_index: 3,
                    decide: Rc::new(|_taken, _cancel| true),
                }])?;
                call.proceed()
            }),
        )
        .unwrap();
        let host = TrapHost {
            seen: Rc::new(RefCell::new(Vec::new())),
        };
        assert_eq!(vm.invoke(&host, &[0; 10], 0, None).unwrap(), 20);
    }

    #[test]
    fn block_copy_moves_words() {
        use QvmOpcode as O;
        let image = assemble(
            vec![
                (O::OpEnter, word(8)),
                (O::OpConst, word(48)),
                (O::OpConst, word(16)),
                (O::OpBlockCopy, word(4)),
                (O::OpConst, word(0)),
                (O::OpLeave, word(8)),
            ],
            256,
            &[],
        );
        let mut vm = QvmInterpreter::new(
            &image,
            QvmAllocationProfile::Unaccounted,
            None,
            QvmSemantics::Interpreted,
        )
        .unwrap();
        vm.memory().write_bytes(16, &[1, 2, 3, 4]).unwrap();
        let host = TrapHost {
            seen: Rc::new(RefCell::new(Vec::new())),
        };
        assert_eq!(vm.invoke(&host, &[0; 10], 0, None).unwrap(), 0);
        assert_eq!(vm.memory().read_bytes(48, 4).unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn operand_errors_surface() {
        use QvmOpcode as O;
        let mut vm = runner(
            vec![
                (O::OpEnter, word(8)),
                (O::OpPop, QvmOperand::None),
                (O::OpLeave, word(8)),
            ],
            QvmSemantics::Interpreted,
        );
        let host = TrapHost {
            seen: Rc::new(RefCell::new(Vec::new())),
        };
        assert!(vm.invoke(&host, &[0; 10], 0, None).is_err());
        assert!(vm.invoke(&host, &[0; 3], 0, None).is_err());
    }

    #[test]
    fn counter_evaluation_isolates_one_word() {
        use QvmOpcode as O;
        let image = assemble(
            vec![
                (O::OpEnter, word(8)),
                (O::OpConst, word(8)),
                (O::OpConst, word(8)),
                (O::OpLoad4, QvmOperand::None),
                (O::OpConst, word(1)),
                (O::OpAdd, QvmOperand::None),
                (O::OpStore4, QvmOperand::None),
                (O::OpConst, word(0)),
                (O::OpLeave, word(8)),
            ],
            64,
            &[],
        );
        let mut vm = QvmInterpreter::new(
            &image,
            QvmAllocationProfile::Unaccounted,
            None,
            QvmSemantics::Interpreted,
        )
        .unwrap();
        let host = TrapHost {
            seen: Rc::new(RefCell::new(Vec::new())),
        };
        let value = vm.evaluate_counter(&host, 8, 41, &[0], None, &[0; 10], 0).unwrap();
        assert_eq!(value, 42);
        assert_eq!(vm.memory().get_i32(8).unwrap(), 0);
    }

    #[test]
    fn restart_and_restore_replace_data() {
        use QvmOpcode as O;
        let image = assemble(
            vec![
                (O::OpEnter, word(8)),
                (O::OpBreak, QvmOperand::None),
                (O::OpConst, word(0)),
                (O::OpLeave, word(8)),
            ],
            256,
            &[],
        );
        let mut vm = QvmInterpreter::new(
            &image,
            QvmAllocationProfile::Unaccounted,
            None,
            QvmSemantics::Interpreted,
        )
        .unwrap();
        let host = TrapHost {
            seen: Rc::new(RefCell::new(Vec::new())),
        };
        vm.invoke(&host, &[0; 10], 0, None).unwrap();
        assert_eq!(vm.break_count(), 1);
        vm.memory().write_bytes(0, &[9, 9, 9, 9]).unwrap();
        let snapshot = vm.memory().snapshot();
        vm.memory().write_bytes(0, &[1, 1, 1, 1]).unwrap();
        vm.restore_data(&snapshot).unwrap();
        assert_eq!(vm.memory().read_bytes(0, 4).unwrap(), vec![9, 9, 9, 9]);
        assert_eq!(vm.indent().unwrap(), "");
        vm.set_call_level(2);
        assert_eq!(vm.indent().unwrap(), "    ");
    }
}
