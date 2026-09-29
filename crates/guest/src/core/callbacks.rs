//! Host-callback table: guest-owned trap addresses for host functions.
//!
//! Donor: `src/guest/core/callbacks.ts` (`GuestCallbackTable`). Host
//! callbacks have guest-owned trap addresses, never host function pointers.
//! Bound traps allocate 16 `INT3` bytes so a missed table dispatch traps
//! instead of executing host bytes as guest code.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::abi::runner::{GuestCallFailure, GuestCallRequest, GuestCallRunner};
use crate::core::contracts::{
    CallbackId, GuestAccess, GuestAddress, GuestCallContext, GuestCallResult, GuestCallSignature,
    GuestCallValue,
};
use crate::core::memory::SparseGuestMemory;
use crate::core::registers::GuestProcessorState;
use crate::error::GuestError;

/// Live dispatch access handed to a host callback: guest memory, processor
/// state, nested guest calls, and callback binding. The borrow is
/// short-lived: every accessor reborrows the runner, so closures must not
/// hold the returned references across a nested [`Self::invoke`].
pub struct HostCallContext<'r, 'm> {
    runner: &'r mut GuestCallRunner<'m>,
}

impl<'r, 'm> HostCallContext<'r, 'm> {
    /// Wrap the dispatching runner.
    pub fn new(runner: &'r mut GuestCallRunner<'m>) -> Self {
        Self { runner }
    }

    /// Guest memory of the dispatching CPU.
    pub fn memory(&mut self) -> &mut SparseGuestMemory {
        self.runner.cpu_parts().1
    }

    /// Processor state of the dispatching CPU.
    pub fn cpu_state(&mut self) -> &mut GuestProcessorState {
        self.runner.cpu_parts().0
    }

    /// Shared hook state (callback table).
    pub fn hooks(&self) -> &Rc<HookState> {
        self.runner.hooks()
    }

    /// Invoke a nested guest call re-entrantly.
    pub fn invoke(
        &mut self,
        request: &GuestCallRequest,
    ) -> Result<GuestCallResult, GuestCallFailure> {
        self.runner.invoke(request)
    }

    /// Bind a host callback to a fresh trap address in this guest.
    pub fn bind_callback(
        &mut self,
        callback: GuestHostCallback,
    ) -> Result<GuestAddress, GuestError> {
        let hooks = Rc::clone(self.hooks());
        let address = hooks
            .callbacks
            .borrow_mut()
            .bind(self.memory(), callback)?;
        Ok(address)
    }
}

/// Host implementation: dispatch access plus the donor's context and
/// arguments. Failures return [`GuestError`]; the runner maps them onto the
/// failing call.
pub type HostCallbackFn = Rc<
    dyn for<'r, 'm> Fn(
        &mut HostCallContext<'r, 'm>,
        &GuestCallContext,
        &[GuestCallValue],
    ) -> Result<GuestCallResult, GuestError>,
>;

/// Host callback served through a guest trap address.
pub struct GuestHostCallback {
    /// Callback identity.
    pub id: CallbackId,
    /// Call signature.
    pub signature: GuestCallSignature,
    /// Host implementation. Shared so runners can invoke without holding a
    /// table borrow across re-entrant nested calls.
    pub invoke: HostCallbackFn,
}

impl Clone for GuestHostCallback {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            signature: self.signature.clone(),
            invoke: Rc::clone(&self.invoke),
        }
    }
}

/// Re-entrant dispatch handle: cloned out of the table before invoking.
#[derive(Clone)]
pub struct CallbackHandle {
    /// Callback identity.
    pub id: CallbackId,
    /// Call signature.
    pub signature: GuestCallSignature,
    /// Host implementation.
    pub invoke: HostCallbackFn,
}

impl std::fmt::Debug for CallbackHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackHandle")
            .field("id", &self.id)
            .field("signature", &self.signature)
            .finish_non_exhaustive()
    }
}

/// Hook state shared between a CPU and its call runner: the callback table
/// plus the current-frame cells that inline-region gates read.
#[derive(Debug, Default)]
pub struct HookState {
    /// Shared callback table. Observers and entry gates must not re-enter
    /// it; callback dispatch clones [`CallbackHandle`]s instead.
    pub callbacks: RefCell<GuestCallbackTable>,
    /// Runner-assigned id of the innermost active call.
    pub call_id: Cell<u64>,
    /// Live stack pointer sampled by the CPU at each hooked entry.
    pub entry_rsp: Cell<u64>,
}

impl HookState {
    /// Fresh shared hook state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for GuestHostCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestHostCallback")
            .field("id", &self.id)
            .field("signature", &self.signature)
            .finish_non_exhaustive()
    }
}

/// Saved trap address with its binding state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedHostCallbackAddress {
    /// Callback identity.
    pub id: CallbackId,
    /// Trap offset.
    pub byte_offset: u64,
    /// Call signature.
    pub signature: GuestCallSignature,
    /// Whether a host implementation is bound.
    pub bound: bool,
}

struct CallbackEntry {
    id: CallbackId,
    signature: GuestCallSignature,
    address: GuestAddress,
    byte_offset: u64,
    callback: Option<GuestHostCallback>,
    accepts: Option<Rc<dyn Fn() -> bool>>,
}

impl std::fmt::Debug for CallbackEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackEntry")
            .field("id", &self.id)
            .field("byte_offset", &self.byte_offset)
            .field("bound", &self.callback.is_some())
            .finish()
    }
}

struct EntryObserver {
    notify: Rc<dyn Fn()>,
}

/// Host callbacks with guest-owned trap addresses.
#[derive(Default)]
pub struct GuestCallbackTable {
    by_id: HashMap<CallbackId, CallbackEntry>,
    by_address: HashMap<u64, CallbackId>,
    entry_observers: HashMap<u64, HashMap<u64, EntryObserver>>,
    entry_revision: u64,
    next_observer: u64,
}

impl std::fmt::Debug for GuestCallbackTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestCallbackTable")
            .field("entries", &self.by_id.len())
            .field("entry_revision", &self.entry_revision)
            .finish()
    }
}

impl GuestCallbackTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Current entry revision; changes whenever entries or observers change.
    #[must_use]
    pub const fn entry_revision(&self) -> u64 {
        self.entry_revision
    }

    /// Whether no entry or observer claims `byte_offset`.
    #[must_use]
    pub fn instruction_unhooked(&self, byte_offset: u64) -> bool {
        !self.by_address.contains_key(&byte_offset)
            && !self.entry_observers.contains_key(&byte_offset)
    }

    fn changed_entries(&mut self) {
        self.entry_revision = self.entry_revision.wrapping_add(1);
    }

    /// Hook a host-owned native entry without touching guest bytes.
    /// Returns an observer id for [`Self::unhook_entry`].
    pub fn bind_entry(
        &mut self,
        memory: &mut SparseGuestMemory,
        address: GuestAddress,
        callback: GuestHostCallback,
        accepts: Rc<dyn Fn() -> bool>,
    ) -> Result<(), GuestError> {
        memory.check(address, 1, GuestAccess::Execute)?;
        if callback.signature.abi.pointer_bytes() != memory.pointer_bytes()
            || self.by_address.contains_key(&address.offset)
        {
            return Err(GuestError::callback("Invalid or occupied native callback entry"));
        }
        let id = callback.id.clone();
        self.by_address.insert(address.offset, id.clone());
        // Entry hooks are keyed by address only; stash under a synthetic id.
        let signature = callback.signature.clone();
        let entry_id = id.clone();
        self.by_id.insert(
            id,
            CallbackEntry {
                id: entry_id,
                signature,
                address,
                byte_offset: address.offset,
                callback: Some(callback),
                accepts: Some(accepts),
            },
        );
        self.changed_entries();
        Ok(())
    }

    /// Remove a native entry hook.
    pub fn unhook_entry(&mut self, address: GuestAddress) {
        if let Some(id) = self.by_address.remove(&address.offset) {
            self.by_id.remove(&id);
            self.changed_entries();
        }
    }

    /// Run `before` at instruction entry; returns an observer id for
    /// [`Self::unobserve_entry`].
    pub fn observe_entry(
        &mut self,
        memory: &mut SparseGuestMemory,
        address: GuestAddress,
        before: Rc<dyn Fn()>,
    ) -> Result<u64, GuestError> {
        memory.check(address, 1, GuestAccess::Execute)?;
        let id = self.next_observer;
        self.next_observer += 1;
        self.entry_observers
            .entry(address.offset)
            .or_default()
            .insert(id, EntryObserver { notify: before });
        self.changed_entries();
        Ok(id)
    }

    /// Remove an entry observer.
    pub fn unobserve_entry(&mut self, address: GuestAddress, id: u64) {
        let mut empty = false;
        let mut changed = false;
        if let Some(observers) = self.entry_observers.get_mut(&address.offset) {
            if observers.remove(&id).is_some() {
                changed = true;
            }
            empty = observers.is_empty();
        }
        if changed {
            self.changed_entries();
        }
        if empty {
            self.entry_observers.remove(&address.offset);
        }
    }

    /// Called once at instruction entry. Runs observers and reports whether
    /// a callback claims this address.
    pub fn enter(
        &mut self,
        memory: &mut SparseGuestMemory,
        address: GuestAddress,
    ) -> Result<bool, GuestError> {
        if !self.entry_observers.contains_key(&address.offset)
            && !self.by_address.contains_key(&address.offset)
        {
            if address.space != memory.address_space() {
                memory.check(address, 1, GuestAccess::Execute)?;
            }
            return Ok(false);
        }
        memory.check(address, 1, GuestAccess::Execute)?;
        if let Some(observers) = self.entry_observers.get(&address.offset) {
            let ids: Vec<u64> = observers.keys().copied().collect();
            for id in ids {
                if let Some(observer) = self
                    .entry_observers
                    .get(&address.offset)
                    .and_then(|observers| observers.get(&id))
                {
                    (observer.notify)();
                }
            }
        }
        Ok(self.resolve(memory, address)?.is_some())
    }

    /// Bind a host callback to a fresh `INT3` trap mapping.
    pub fn bind(
        &mut self,
        memory: &mut SparseGuestMemory,
        callback: GuestHostCallback,
    ) -> Result<GuestAddress, GuestError> {
        if callback.signature.abi.pointer_bytes() != memory.pointer_bytes() {
            return Err(GuestError::callback(
                "Callback ABI differs from its guest address space",
            ));
        }
        if self.by_id.contains_key(&callback.id) {
            let same = self.by_id.get(&callback.id).is_some_and(|previous| {
                previous.callback.is_none() && previous.signature.same_as(&callback.signature)
            });
            let bound = self.by_id.get(&callback.id).is_some_and(|previous| previous.callback.is_some());
            if bound {
                return Err(GuestError::callback(format!(
                    "Callback {} is already bound",
                    callback.id
                )));
            }
            if !same {
                return Err(GuestError::callback(format!(
                    "Callback {} changed ABI or signature",
                    callback.id
                )));
            }
            let address = self.by_id.get_mut(&callback.id).map(|previous| {
                previous.callback = Some(callback);
                previous.address
            });
            self.changed_entries();
            return Ok(address.expect("callback entry checked above"));
        }
        let address = memory.allocate(&crate::core::contracts::GuestAllocationOptions {
            byte_length: 16,
            alignment: 16,
            permissions: crate::core::contracts::GuestPermissions::ReadWrite,
            label: format!("callback {}", callback.id),
        })?;
        memory.write(address, &[0xcc; 16])?;
        memory.protect(
            address,
            16,
            crate::core::contracts::GuestPermissions::ReadExecute,
        )?;
        let callback_id = callback.id.clone();
        let signature = callback.signature.clone();
        let entry = CallbackEntry {
            id: callback_id,
            signature,
            address,
            byte_offset: address.offset,
            callback: Some(callback),
            accepts: None,
        };
        let id = entry.id.clone();
        self.by_address.insert(address.offset, id.clone());
        self.by_id.insert(id, entry);
        self.changed_entries();
        Ok(address)
    }

    /// Unbind a callback, keeping its trap address allocated.
    pub fn unbind(&mut self, id: &CallbackId) {
        if let Some(entry) = self.by_id.get_mut(id) {
            entry.callback = None;
            self.changed_entries();
        }
    }

    /// Trap address of a bound callback, if any.
    #[must_use]
    pub fn address(&self, id: &CallbackId) -> Option<GuestAddress> {
        self.by_id.get(id).map(|entry| entry.address)
    }

    /// Whether `byte_offset` names a bound trap (not a bare entry hook).
    #[must_use]
    pub fn has_bound_trap(&self, byte_offset: u64) -> bool {
        self.by_address
            .get(&byte_offset)
            .and_then(|id| self.by_id.get(id))
            .is_some_and(|entry| entry.callback.is_some() && entry.accepts.is_none())
    }

    /// Resolve the callback at `address`, honouring entry-hook gates.
    pub fn resolve(
        &mut self,
        memory: &mut SparseGuestMemory,
        address: GuestAddress,
    ) -> Result<Option<&GuestHostCallback>, GuestError> {
        memory.check(address, 1, GuestAccess::Execute)?;
        let Some(id) = self.by_address.get(&address.offset).cloned() else {
            return Ok(None);
        };
        let Some(entry) = self.by_id.get(&id) else {
            return Ok(None);
        };
        if entry.accepts.as_ref().is_some_and(|accepts| !accepts()) {
            return Ok(None);
        }
        if entry.callback.is_none() {
            return Err(GuestError::callback(format!(
                "Guest callback {} at 0x{:x} is unbound",
                entry.id, entry.byte_offset
            )));
        }
        Ok(self.by_id.get(&id).and_then(|entry| entry.callback.as_ref()))
    }

    /// Clone a dispatch handle for the callback at `address`. The handle
    /// invokes without holding any table borrow, so nested calls re-enter
    /// safely.
    pub fn handle(
        &mut self,
        memory: &mut SparseGuestMemory,
        address: GuestAddress,
    ) -> Result<Option<CallbackHandle>, GuestError> {
        Ok(self.resolve(memory, address)?.map(|callback| CallbackHandle {
            id: callback.id.clone(),
            signature: callback.signature.clone(),
            invoke: Rc::clone(&callback.invoke),
        }))
    }

    /// Invoke the callback at `address` with live dispatch access.
    pub fn invoke(
        &mut self,
        memory: &mut SparseGuestMemory,
        dispatch: &mut HostCallContext<'_, '_>,
        address: GuestAddress,
        context: &GuestCallContext,
        arguments: &[GuestCallValue],
    ) -> Result<GuestCallResult, GuestError> {
        // Borrow dance: resolve validates, then invoke without holding the borrow.
        let handle = self.handle(memory, address)?.ok_or_else(|| {
            GuestError::callback(format!(
                "No host callback at guest address 0x{:x}",
                address.offset
            ))
        })?;
        (handle.invoke)(dispatch, context, arguments)
    }

    /// Snapshot the trap addresses with their binding states.
    #[must_use]
    pub fn checkpoint(&self) -> Vec<SavedHostCallbackAddress> {
        self.by_id
            .values()
            .filter(|entry| entry.accepts.is_none())
            .map(|entry| SavedHostCallbackAddress {
                id: entry.id.clone(),
                byte_offset: entry.byte_offset,
                signature: entry.signature.clone(),
                bound: entry.callback.is_some(),
            })
            .collect()
    }

    /// Restore traps into `memory`, resolving bound callbacks through `resolve`.
    pub fn restore(
        memory: &mut SparseGuestMemory,
        saved: &[SavedHostCallbackAddress],
        mut resolve: impl FnMut(&CallbackId) -> Option<GuestHostCallback>,
    ) -> Result<Self, GuestError> {
        let mut table = Self::new();
        for record in saved {
            if table.by_id.contains_key(&record.id)
                || table.by_address.contains_key(&record.byte_offset)
            {
                return Err(GuestError::callback(
                    "Duplicate restored guest callback identity or address",
                ));
            }
            let Some(address) = memory.pointer(record.byte_offset)? else {
                return Err(GuestError::callback("Guest callback cannot have a null address"));
            };
            memory.check(address, 16, GuestAccess::Execute)?;
            if record.signature.abi.pointer_bytes() != memory.pointer_bytes() {
                return Err(GuestError::callback(
                    "Saved callback ABI differs from the restored address space",
                ));
            }
            let callback = if record.bound { resolve(&record.id) } else { None };
            if record.bound && callback.is_none() {
                return Err(GuestError::callback(format!(
                    "Missing restored host callback {}",
                    record.id
                )));
            }
            if let Some(callback) = &callback {
                if callback.id != record.id || !callback.signature.same_as(&record.signature) {
                    return Err(GuestError::callback(format!(
                        "Restored callback {} has a different identity or signature",
                        record.id
                    )));
                }
            }
            let entry = CallbackEntry {
                id: record.id.clone(),
                signature: record.signature.clone(),
                address,
                byte_offset: record.byte_offset,
                callback,
                accepts: None,
            };
            table.by_address.insert(record.byte_offset, record.id.clone());
            table.by_id.insert(record.id.clone(), entry);
        }
        Ok(table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;

    use crate::core::contracts::{
        ContentDigest, GuestValueLayout, ModuleIdentity, NativeCallAbi,
    };

    fn test_module() -> ModuleIdentity {
        ModuleIdentity::new(
            ProviderId::new("test", "callbacks"),
            "test.so",
            ContentDigest::new("sha256", "abc"),
            "r1",
        )
    }

    fn test_signature() -> GuestCallSignature {
        GuestCallSignature {
            abi: NativeCallAbi::SystemVX86_64,
            parameters: vec![],
            result: Some(GuestValueLayout::Scalar(
                crate::core::contracts::GuestStorage::Int32,
            )),
            variadic: false,
        }
    }

    struct FakeCpu {
        state: crate::core::registers::GuestProcessorState,
        memory: SparseGuestMemory,
        hooks: Option<Rc<HookState>>,
    }

    impl crate::abi::GuestCpu for FakeCpu {
        fn parts(
            &mut self,
        ) -> (
            &mut crate::core::registers::GuestProcessorState,
            &mut SparseGuestMemory,
        ) {
            (&mut self.state, &mut self.memory)
        }

        fn set_hook_state(&mut self, hooks: Option<Rc<HookState>>) {
            self.hooks = hooks;
        }

        fn run(
            &mut self,
            _instruction_budget: u64,
            _return_address: Option<GuestAddress>,
        ) -> crate::core::contracts::GuestExecutionStop {
            crate::core::contracts::GuestExecutionStop::Halt {
                instructions: 0,
                address: GuestAddress::new(self.memory.address_space(), 0),
            }
        }
    }

    #[test]
    fn bind_invoke_unbind_round_trip() {
        let mut memory = SparseGuestMemory::new(test_module(), 8, 0x10000).unwrap();
        let mut table = GuestCallbackTable::new();
        let signature = test_signature();
        let address = table
            .bind(
                &mut memory,
                GuestHostCallback {
                    id: CallbackId::new("test", "trap"),
                    signature: signature.clone(),
                    invoke: Rc::new(|_, _, _| {
                        Ok(GuestCallResult::Value(GuestCallValue::Int32(7)))
                    }),
                },
            )
            .unwrap();
        assert!(table.has_bound_trap(address.offset));
        let context = GuestCallContext {
            module: test_module(),
            callback: crate::core::contracts::GuestCallbackReference::TypeScript {
                provider: ProviderId::new("test", "provider"),
                callback: CallbackId::new("test", "trap"),
            },
            parent: None,
            itself: None,
            other: None,
        };
        let state = crate::core::registers::GuestProcessorState::create(
            crate::core::registers::GuestProcessorInitialState {
                architecture: crate::core::contracts::GuestArchitecture::X86_64,
                instruction_pointer: 0x10000,
                stack_pointer: 0x20000,
                flags: 2,
                x87_control_word: 0x37f,
                mxcsr: 0x1f80,
                mxcsr_mask: 0xffff,
            },
        )
        .unwrap();
        let dispatch_memory = SparseGuestMemory::new(test_module(), 8, 0x10000).unwrap();
        let mut cpu = FakeCpu {
            state,
            memory: dispatch_memory,
            hooks: None,
        };
        let hooks = Rc::new(HookState::new());
        let trap = cpu
            .memory
            .map(&crate::core::contracts::GuestMapOptions {
                base: 0x10000,
                byte_length: 0x1000,
                permissions: crate::core::contracts::GuestPermissions::ReadExecute,
                label: "return".to_string(),
                bytes: None,
            })
            .unwrap();
        let mut runner =
            crate::abi::runner::GuestCallRunner::new(&mut cpu, Rc::clone(&hooks), trap, None)
                .unwrap();
        let mut dispatch = HostCallContext::new(&mut runner);
        let result = table
            .invoke(&mut memory, &mut dispatch, address, &context, &[])
            .unwrap();
        assert_eq!(result, GuestCallResult::Value(GuestCallValue::Int32(7)));
        table.unbind(&CallbackId::new("test", "trap"));
        assert!(!table.has_bound_trap(address.offset));
        let saved = table.checkpoint();
        assert_eq!(saved.len(), 1);
        assert!(!saved[0].bound);
    }
}
