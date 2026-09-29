//! Guest call runner: synchronous nested guest entries with host dispatch.
//!
//! Donor: `src/guest/abi/runner.ts` (`GuestCallRunner`). Nested entries
//! preserve processor context and retain guest memory mutations. The donor's
//! generator steps become an explicit loop; loading slices yield through a
//! host `next_frame` callback instead of a promise.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use thiserror::Error;

use crate::abi::adapter::X86AbiAdapter;
use crate::abi::classify::plan_guest_call_layouts;
use crate::abi::values::inferred_layout;
use crate::abi::GuestCpu;
use crate::core::callbacks::{CallbackHandle, GuestHostCallback, HookState, HostCallContext};
use crate::core::contracts::{
    CallbackId, GuestAccess, GuestAddress, GuestCallContext, GuestCallResult,
    GuestCallbackReference, GuestCallSignature, GuestCallValue, GuestExecutionStop,
    GuestIntegerWidth, GuestRegister, GuestValueLayout, NativeCallAbi,
};
use crate::error::GuestError;

/// Guest call failure: an execution stop or an underlying guest error.
#[derive(Debug, Error)]
pub enum GuestCallFailure {
    /// Execution stopped before returning (budget, halt, exception,
    /// unsupported instruction).
    #[error("Guest call stopped: {stop:?}")]
    Stopped {
        /// Terminal stop.
        stop: GuestExecutionStop,
        /// Active call context.
        context: GuestCallContext,
    },
    /// Underlying guest failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// One guest call request.
#[derive(Debug, Clone)]
pub struct GuestCallRequest {
    /// Call target.
    pub target: GuestAddress,
    /// Call signature.
    pub signature: GuestCallSignature,
    /// Call arguments.
    pub arguments: Vec<GuestCallValue>,
    /// Call context.
    pub context: GuestCallContext,
    /// Instruction budget.
    pub instruction_budget: u64,
}

struct ActiveCall {
    context: GuestCallContext,
    remaining: u64,
    id: u64,
}

/// Continuation offered to an inline-region interceptor.
pub trait GuestInlineContinuation {
    /// Execute the region body, joining at the region end.
    fn execute(&mut self);
    /// Skip the region body, jumping to the region end.
    fn skip(&mut self);
}

#[derive(Default)]
struct RegionRuntime {
    executing: Vec<(u64, u64)>,
}

struct InlineRegion {
    entry: GuestAddress,
    join: GuestAddress,
    intercept: Box<dyn FnMut(&mut dyn GuestInlineContinuation) -> Result<(), GuestError>>,
    runtime: Rc<RefCell<RegionRuntime>>,
}

struct RecordingContinuation {
    choice: Option<bool>,
    failed: Option<String>,
}

impl GuestInlineContinuation for RecordingContinuation {
    fn execute(&mut self) {
        if self.choice.is_some() {
            self.failed = Some("Inline continuation was already consumed".to_string());
        } else {
            self.choice = Some(true);
        }
    }

    fn skip(&mut self) {
        if self.choice.is_some() {
            self.failed = Some("Inline continuation was already consumed".to_string());
        } else {
            self.choice = Some(false);
        }
    }
}

/// Synchronous nested guest entries over one CPU and callback table.
pub struct GuestCallRunner<'a> {
    cpu: &'a mut dyn GuestCpu,
    hooks: Rc<HookState>,
    return_address: GuestAddress,
    variadic_layouts: Option<
        Rc<dyn Fn(&CallbackHandle, &[GuestCallValue], &GuestCallContext) -> Vec<GuestValueLayout>>,
    >,
    active: Vec<ActiveCall>,
    regions: HashMap<u64, InlineRegion>,
    next_call_id: u64,
    callback_context: Option<GuestCallContext>,
    instructions_executed: u64,
    loading_suspended: bool,
    maximum_loading_slice_ms: f64,
}

impl<'a> GuestCallRunner<'a> {
    /// Runner over `cpu` with shared `hooks`. `return_address` must be
    /// executable.
    pub fn new(
        cpu: &'a mut dyn GuestCpu,
        hooks: Rc<HookState>,
        return_address: GuestAddress,
        variadic_layouts: Option<
            Rc<dyn Fn(&CallbackHandle, &[GuestCallValue], &GuestCallContext) -> Vec<GuestValueLayout>>,
        >,
    ) -> Result<Self, GuestError> {
        cpu.set_hook_state(Some(Rc::clone(&hooks)));
        let (_, memory) = cpu.parts();
        memory.check(return_address, 1, GuestAccess::Execute)?;
        Ok(Self {
            cpu,
            hooks,
            return_address,
            variadic_layouts,
            active: Vec::new(),
            regions: HashMap::new(),
            next_call_id: 1,
            callback_context: None,
            instructions_executed: 0,
            loading_suspended: false,
            maximum_loading_slice_ms: 0.0,
        })
    }

    /// Shared hook state.
    #[must_use]
    pub fn hooks(&self) -> &Rc<HookState> {
        &self.hooks
    }

    /// Borrow processor state and memory together.
    pub fn cpu_parts(
        &mut self,
    ) -> (
        &mut crate::core::registers::GuestProcessorState,
        &mut crate::core::memory::SparseGuestMemory,
    ) {
        self.cpu.parts()
    }

    /// Active call depth.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.active.len()
    }

    /// Innermost context: host-callback context, else innermost call.
    #[must_use]
    pub fn current_context(&self) -> Option<&GuestCallContext> {
        self.callback_context
            .as_ref()
            .or_else(|| self.active.last().map(|call| &call.context))
    }

    /// Total retired instructions.
    #[must_use]
    pub const fn instructions_executed(&self) -> u64 {
        self.instructions_executed
    }

    /// Slowest loading slice in milliseconds.
    #[must_use]
    pub const fn maximum_loading_slice_ms(&self) -> f64 {
        self.maximum_loading_slice_ms
    }

    /// Invoke a guest call synchronously.
    pub fn invoke(
        &mut self,
        request: &GuestCallRequest,
    ) -> Result<GuestCallResult, GuestCallFailure> {
        if self.loading_suspended {
            return Err(GuestError::callback("Guest loading call is suspended").into());
        }
        self.invoke_inner(request, u64::MAX, None)
    }

    /// Invoke a long loading call, yielding to `next_frame` every slice.
    pub fn invoke_loading(
        &mut self,
        request: &GuestCallRequest,
        next_frame: &mut dyn FnMut(),
    ) -> Result<GuestCallResult, GuestCallFailure> {
        if self.depth() != 0 {
            return Err(GuestError::callback("Guest loading requires an idle call runner").into());
        }
        self.invoke_inner(request, 16_384, Some(next_frame))
    }

    fn invoke_inner(
        &mut self,
        request: &GuestCallRequest,
        slice: u64,
        next_frame: Option<&mut dyn FnMut()>,
    ) -> Result<GuestCallResult, GuestCallFailure> {
        if request.instruction_budget == 0 {
            return Err(GuestError::callback("Guest instruction budget must be positive").into());
        }
        {
            let (_, memory) = self.cpu.parts();
            if request.context.module.id != memory.module().id
                || request.context.module.digest != memory.module().digest
            {
                return Err(
                    GuestError::callback("Call context belongs to a different guest module").into(),
                );
            }
        }
        let enclosing_remaining = self.active.last().map(|call| call.remaining);
        let context = match self.current_context() {
            None => request.context.clone(),
            Some(current) => GuestCallContext {
                parent: Some(Box::new(current.clone())),
                ..request.context.clone()
            },
        };
        let call_id = self.next_call_id;
        self.next_call_id += 1;
        let width = {
            let (_, memory) = self.cpu.parts();
            if memory.pointer_bytes() == 4 {
                GuestIntegerWidth::B32
            } else {
                GuestIntegerWidth::B64
            }
        };
        let (caller_instruction, caller_stack, saved) = {
            let (state, _) = self.cpu.parts();
            let saved = if self.active.is_empty() {
                None
            } else {
                Some(state.clone())
            };
            let caller_stack = state.registers.read(GuestRegister::Rsp, width, false)?;
            (state.instruction_pointer, caller_stack, saved)
        };
        let adapter = X86AbiAdapter::new(request.signature.abi);
        adapter.enter(
            self.cpu,
            request.target,
            &request.signature,
            &request.arguments,
            self.return_address,
        )?;
        let entry_stack = {
            let (state, _) = self.cpu.parts();
            state.registers.read(GuestRegister::Rsp, width, false)?
        };
        let layouts: Vec<GuestValueLayout> =
            if request.arguments.len() == request.signature.parameters.len() {
                request.signature.parameters.clone()
            } else {
                request
                    .arguments
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        request
                            .signature
                            .parameters
                            .get(index)
                            .cloned()
                            .unwrap_or_else(|| inferred_layout(value, request.signature.variadic))
                    })
                    .collect()
            };
        let plan = plan_guest_call_layouts(&request.signature, &layouts)?;
        self.hooks.call_id.set(call_id);
        self.active.push(ActiveCall {
            context,
            remaining: request
                .instruction_budget
                .min(enclosing_remaining.unwrap_or(request.instruction_budget)),
            id: call_id,
        });
        let outcome = self.run_loop(self.return_address, slice, next_frame);
        // Stops retain the actual faulting processor and memory for the
        // runtime's exception policy; the frame pops on every path.
        self.active.pop();
        outcome?;
        {
            let (state, memory) = self.cpu.parts();
            let restored_stack = state.registers.read(GuestRegister::Rsp, width, false)?;
            let expected = entry_stack
                .wrapping_add((memory.pointer_bytes() + plan.callee_pop_bytes) as u64);
            if restored_stack != expected {
                return Err(GuestError::callback(
                    "Guest returned with incorrect ABI stack cleanup",
                )
                .into());
            }
        }
        let result = X86AbiAdapter::new(request.signature.abi)
            .return_value(self.cpu, &request.signature)?;
        {
            let (state, _) = self.cpu.parts();
            if let Some(saved) = saved {
                *state = saved;
            } else {
                state
                    .registers
                    .write(GuestRegister::Rsp, width, caller_stack, false)?;
                state.instruction_pointer = caller_instruction;
            }
        }
        Ok(result)
    }

    fn run_loop(
        &mut self,
        return_address: GuestAddress,
        slice: u64,
        next_frame: Option<&mut dyn FnMut()>,
    ) -> Result<(), GuestCallFailure> {
        let mut next_frame = next_frame;
        let mut slice_remaining = slice;
        loop {
            let active_remaining = self.active.last().map_or(0, |call| call.remaining);
            if slice_remaining == 0 && active_remaining > 0 {
                let started = Instant::now();
                match next_frame.as_mut() {
                    Some(frame) => {
                        self.loading_suspended = true;
                        frame();
                        self.loading_suspended = false;
                    }
                    None => {
                        return Err(GuestError::callback("Synchronous guest call yielded").into());
                    }
                }
                let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                self.maximum_loading_slice_ms =
                    self.maximum_loading_slice_ms.max(elapsed);
                slice_remaining = slice;
                continue;
            }
            let budget = active_remaining.min(slice_remaining);
            self.hooks.call_id.set(self.active.last().map_or(0, |call| call.id));
            let raw_stop = self.cpu.run(budget, Some(return_address));
            let stop = match &raw_stop {
                GuestExecutionStop::Budget { .. } => {
                    let (state, _) = self.cpu.parts();
                    if state.instruction_pointer == return_address.offset {
                        GuestExecutionStop::Return {
                            instructions: raw_stop.instructions(),
                            address: return_address,
                        }
                    } else {
                        raw_stop
                    }
                }
                _ => raw_stop,
            };
            if stop.instructions() > active_remaining {
                return Err(GuestError::callback("CPU returned an invalid instruction count").into());
            }
            self.instructions_executed += stop.instructions();
            slice_remaining = slice_remaining.saturating_sub(stop.instructions());
            for frame in &mut self.active {
                frame.remaining = frame.remaining.saturating_sub(stop.instructions());
            }
            match stop {
                GuestExecutionStop::Return { .. } => return Ok(()),
                GuestExecutionStop::Budget { .. }
                    if self.active.last().map_or(false, |call| call.remaining > 0)
                        && slice_remaining == 0 =>
                {
                    continue;
                }
                GuestExecutionStop::HostCall { address, .. } => {
                    self.dispatch_host_call(address)?;
                    slice_remaining = slice_remaining.saturating_sub(1);
                    for frame in &mut self.active {
                        frame.remaining = frame.remaining.saturating_sub(1);
                    }
                }
                stop => {
                    let context = self
                        .active
                        .last()
                        .map(|call| call.context.clone())
                        .ok_or_else(|| {
                            GuestError::callback("Guest stopped with no active call")
                        })?;
                    return Err(GuestCallFailure::Stopped { stop, context });
                }
            }
        }
    }

    fn dispatch_host_call(&mut self, address: GuestAddress) -> Result<(), GuestCallFailure> {
        if self.regions.contains_key(&address.offset) {
            let remaining = self.active.last().map_or(0, |call| call.remaining);
            if remaining == 0 {
                let context = self
                    .active
                    .last()
                    .map(|call| call.context.clone())
                    .ok_or_else(|| GuestError::callback("Guest stopped with no active call"))?;
                return Err(GuestCallFailure::Stopped {
                    stop: GuestExecutionStop::Budget { instructions: 0 },
                    context,
                });
            }
            return self.intercept_region(address);
        }
        let handle = {
            let (_, memory) = self.cpu.parts();
            self.hooks
                .callbacks
                .borrow_mut()
                .handle(memory, address)?
        }
        .ok_or_else(|| GuestError::callback(format!("Unknown guest callback at 0x{:x}", address.offset)))?;
        let context = {
            let enclosing = self.active.last().map(|call| call.context.clone());
            let (_, memory) = self.cpu.parts();
            GuestCallContext {
                module: memory.module().clone(),
                callback: GuestCallbackReference::NativeGuest {
                    module: memory.module().clone(),
                    address,
                    abi: handle.signature.abi,
                },
                parent: enclosing.map(Box::new),
                itself: None,
                other: None,
            }
        };
        // Merge the enclosing entities, if any.
        let context = match self.active.last() {
            Some(active) => GuestCallContext {
                itself: active.context.itself.clone(),
                other: active.context.other.clone(),
                ..context
            },
            None => context,
        };
        let callback_adapter = X86AbiAdapter::new(handle.signature.abi);
        let fixed = callback_adapter.arguments(self.cpu, &handle.signature, &[])?;
        if handle.signature.variadic && self.variadic_layouts.is_none() {
            return Err(GuestError::callback("Variadic host callback requires a layout resolver").into());
        }
        let extra = match &self.variadic_layouts {
            Some(resolve) if handle.signature.variadic => resolve(&handle, &fixed, &context),
            _ => Vec::new(),
        };
        let arguments = if extra.is_empty() {
            fixed
        } else {
            callback_adapter.arguments(self.cpu, &handle.signature, &extra)?
        };
        let previous = self.callback_context.replace(context.clone());
        let outcome = {
            let mut dispatch = HostCallContext::new(self);
            (handle.invoke)(&mut dispatch, &context, &arguments)
        };
        self.callback_context = previous;
        let result = outcome?;
        callback_adapter.leave(self.cpu, &handle.signature, &result)?;
        Ok(())
    }

    /// Bind an inline region: `intercept` chooses per hit whether to execute
    /// the guest bytes from `entry` to `join` or skip them. The source
    /// adapter qualifies the fixed entry and join against its original
    /// artifact. Returns an unbind closure.
    pub fn bind_inline_region(
        &mut self,
        entry: GuestAddress,
        join: GuestAddress,
        abi: NativeCallAbi,
        intercept: impl FnMut(&mut dyn GuestInlineContinuation) -> Result<(), GuestError> + 'static,
        accepts: Rc<dyn Fn() -> bool>,
    ) -> Result<Box<dyn FnOnce()>, GuestError> {
        {
            let (_, memory) = self.cpu.parts();
            memory.check(entry, 1, GuestAccess::Execute)?;
            memory.check(join, 1, GuestAccess::Execute)?;
        }
        if entry.offset == join.offset || self.regions.contains_key(&entry.offset) {
            return Err(GuestError::callback("Invalid or occupied inline region"));
        }
        let runtime = Rc::new(RefCell::new(RegionRuntime::default()));
        let hooks = Rc::clone(&self.hooks);
        let gate_runtime = Rc::clone(&runtime);
        let gate: Rc<dyn Fn() -> bool> = Rc::new(move || {
            accepts()
                && !gate_runtime.borrow().executing.contains(&(
                    hooks.call_id.get(),
                    hooks.entry_rsp.get(),
                ))
        });
        {
            let (_, memory) = self.cpu.parts();
            self.hooks.callbacks.borrow_mut().bind_entry(
                memory,
                entry,
                GuestHostCallback {
                    id: CallbackId::new("inline", &format!("{:x}", entry.offset)),
                    signature: GuestCallSignature {
                        abi,
                        parameters: Vec::new(),
                        result: None,
                        variadic: false,
                    },
                    invoke: Rc::new(|_, _, _| {
                        Err(GuestError::callback(
                            "Inline region cannot be invoked as an ABI callback",
                        ))
                    }),
                },
                gate,
            )?;
        }
        self.regions.insert(
            entry.offset,
            InlineRegion {
                entry,
                join,
                intercept: Box::new(intercept),
                runtime,
            },
        );
        let hooks = Rc::clone(&self.hooks);
        // Unhook the table entry; the region record stays until
        // `unbind_inline_region` but is unreachable once unhooked.
        Ok(Box::new(move || {
            hooks.callbacks.borrow_mut().unhook_entry(entry);
        }) as Box<dyn FnOnce()>)
    }

    /// Remove an inline region and its entry hook.
    pub fn unbind_inline_region(&mut self, entry: GuestAddress) {
        if self.regions.remove(&entry.offset).is_some() {
            self.hooks.callbacks.borrow_mut().unhook_entry(entry);
        }
    }

    fn intercept_region(&mut self, address: GuestAddress) -> Result<(), GuestCallFailure> {
        let width = {
            let (_, memory) = self.cpu.parts();
            if memory.pointer_bytes() == 4 {
                GuestIntegerWidth::B32
            } else {
                GuestIntegerWidth::B64
            }
        };
        let (call_id, stack, entry, join) = {
            let (state, _) = self.cpu.parts();
            let call_id = self.active.last().map_or(0, |call| call.id);
            let stack = state.registers.read(GuestRegister::Rsp, width, false)?;
            let region = self.regions.get(&address.offset).ok_or_else(|| {
                GuestError::callback("Inline region disappeared during dispatch")
            })?;
            (call_id, stack, region.entry, region.join)
        };
        let mut continuation = RecordingContinuation {
            choice: None,
            failed: None,
        };
        {
            let region = self.regions.get_mut(&address.offset).ok_or_else(|| {
                GuestError::callback("Inline region disappeared during dispatch")
            })?;
            (region.intercept)(&mut continuation)?;
        }
        if let Some(failure) = continuation.failed {
            return Err(GuestError::callback(failure).into());
        }
        match continuation.choice {
            Some(true) => {
                {
                    let region = self.regions.get(&address.offset).ok_or_else(|| {
                        GuestError::callback("Inline region disappeared during dispatch")
                    })?;
                    region.runtime.borrow_mut().executing.push((call_id, stack));
                }
                let outcome = self.run_loop(join, u64::MAX, None);
                {
                    let region = self.regions.get(&address.offset).ok_or_else(|| {
                        GuestError::callback("Inline region disappeared during dispatch")
                    })?;
                    region.runtime.borrow_mut().executing.pop();
                }
                outcome?;
                let (state, _) = self.cpu.parts();
                let live_stack = state.registers.read(GuestRegister::Rsp, width, false)?;
                if self.active.last().map_or(0, |call| call.id) != call_id
                    || live_stack != stack
                    || state.instruction_pointer != join.offset
                {
                    return Err(GuestError::callback(
                        "Inline interceptor did not join its source frame",
                    )
                    .into());
                }
            }
            Some(false) => {
                let (state, _) = self.cpu.parts();
                let live_stack = state.registers.read(GuestRegister::Rsp, width, false)?;
                if state.instruction_pointer != entry.offset || live_stack != stack {
                    return Err(GuestError::callback(
                        "Inline continuation is not at its active source frame",
                    )
                    .into());
                }
                state.instruction_pointer = join.offset;
            }
            None => {
                return Err(GuestError::callback(
                    "Inline interceptor did not join its source frame",
                )
                .into());
            }
        }
        Ok(())
    }
}
