//! Common-lived VM table: reservation, binding, and info printing.
//!
//! Port of `src/compat/qvm/registry.ts` (`VM_Create`, `VM_Free`, `VM_Clear`,
//! `VM_Call` and `VM_VmInfo_f` from id Software's `qcommon/vm.c`; Copyright (C)
//! 1999-2005 Id Software, Inc., GPL-2.0-or-later).
//!
//! Slots survive individual module and filesystem lifetimes. The donor
//! `VmRegistration` interface becomes a cloneable handle over shared slot
//! state; stale handles (after `free` or `clear`) fail instead of aliasing a
//! recycled slot.
//!
//! Sync-port note: the donor binding holds a live interpreter reference for
//! `printProfile`; Rust ownership forbids that back-reference, so
//! [`VmRegistry::print_profile`] takes the symbols explicitly and only prints
//! when the last-called slot is interpreted.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::GuestError;

use super::interpreter::qvm_fatal_error;
use super::symbols::QvmSymbols;

/// Explicit replacement for the source `DEBUG_VM` build and debugger controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmExecutionProfile {
    /// Release interpreter.
    Release,
    /// Debug interpreter with trace level and break function.
    Debug {
        /// Trace verbosity (0, 1, or 2).
        trace: u8,
        /// Byte offset breaking into the debugger (0 disables).
        break_function: i32,
    },
}

impl QvmExecutionProfile {
    /// Debug profile with a validated trace level.
    pub fn debug(trace: u8, break_function: i32) -> Result<Self, GuestError> {
        if trace > 2 {
            return Err(GuestError::invalid("QVM debug trace must be 0, 1, or 2"));
        }
        Ok(Self::Debug { trace, break_function })
    }
}

/// What a registered slot currently holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmBinding {
    /// Reserved but not yet bound.
    Initializing,
    /// Bound interpreted image.
    Interpreted,
    /// Bound TypeScript replacement.
    TypeScript,
    /// Freed (or never reserved).
    Freed,
}

impl VmBinding {
    /// Whether the slot no longer holds a live module.
    #[must_use]
    pub fn is_freed(self) -> bool {
        self == Self::Freed
    }
}

#[derive(Debug, Clone)]
enum SlotRecord {
    Empty,
    Initializing {
        data_length: usize,
        table_length: usize,
        code_length: usize,
    },
    Interpreted {
        data_length: usize,
        table_length: usize,
        code_length: usize,
    },
    TypeScript,
}

#[derive(Debug)]
struct Slot {
    name: Option<String>,
    generation: u64,
    record: SlotRecord,
}

impl Slot {
    fn empty() -> Self {
        Self {
            name: None,
            generation: 0,
            record: SlotRecord::Empty,
        }
    }

    fn binding(&self) -> VmBinding {
        if self.name.is_none() {
            return VmBinding::Freed;
        }
        match self.record {
            SlotRecord::Empty => VmBinding::Freed,
            SlotRecord::Initializing { .. } => VmBinding::Initializing,
            SlotRecord::Interpreted { .. } => VmBinding::Interpreted,
            SlotRecord::TypeScript => VmBinding::TypeScript,
        }
    }
}

struct RegistryInner {
    slots: [Slot; 3],
    last_called: Option<(usize, u64)>,
    debug_level: i32,
    print: Box<dyn FnMut(&str)>,
    execution_profile: Box<dyn FnMut() -> QvmExecutionProfile>,
}

impl std::fmt::Debug for RegistryInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryInner")
            .field("last_called", &self.last_called)
            .field("debug_level", &self.debug_level)
            .finish_non_exhaustive()
    }
}

/// Handle to one reserved VM slot. Cloneable; stale handles fail.
#[derive(Debug, Clone)]
pub struct VmRegistration {
    inner: Rc<RefCell<RegistryInner>>,
    slot: usize,
    generation: u64,
    name: String,
}

impl VmRegistration {
    /// Registered name (truncated like the source).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Current binding, or freed when the handle is stale.
    #[must_use]
    pub fn binding(&self) -> VmBinding {
        let inner = self.inner.borrow();
        if inner.slots[self.slot].generation != self.generation {
            return VmBinding::Freed;
        }
        inner.slots[self.slot].binding()
    }

    fn current(&self) -> Result<(), GuestError> {
        if self.binding() == VmBinding::Freed {
            return Err(GuestError::invalid("VM registration has been freed"));
        }
        Ok(())
    }

    /// Active execution profile (release when stale; every interpreter read is
    /// preceded by a liveness check that rejects freed registrations).
    pub fn execution_profile(&self) -> QvmExecutionProfile {
        if self.current().is_err() {
            return QvmExecutionProfile::Release;
        }
        (self.inner.borrow_mut().execution_profile)()
    }

    /// Record the data allocation length.
    pub fn bind_data(&self, length: usize) {
        if self.preparing().is_err() {
            return;
        }
        let mut inner = self.inner.borrow_mut();
        if let SlotRecord::Initializing { data_length, .. } = &mut inner.slots[self.slot].record {
            *data_length = length;
        }
    }

    /// Record the instruction-pointer table length.
    pub fn bind_instruction_pointers_length(&self, length: usize) {
        if self.preparing().is_err() {
            return;
        }
        let mut inner = self.inner.borrow_mut();
        if let SlotRecord::Initializing { table_length, .. } = &mut inner.slots[self.slot].record {
            *table_length = length;
        }
    }

    /// Record the code length.
    pub fn bind_code_length(&self, length: usize) {
        if self.preparing().is_err() {
            return;
        }
        let mut inner = self.inner.borrow_mut();
        if let SlotRecord::Initializing { code_length, .. } = &mut inner.slots[self.slot].record {
            *code_length = length;
        }
    }

    /// Bind an interpreted image with its prepared lengths.
    pub fn bind_interpreter(&self, code_length: usize, table_length: usize, data_length: usize) {
        if self.preparing().is_err() {
            return;
        }
        self.inner.borrow_mut().slots[self.slot].record = SlotRecord::Interpreted {
            data_length,
            table_length,
            code_length,
        };
    }

    /// Bind a TypeScript replacement.
    pub fn bind_typescript(&self) {
        if self.preparing().is_err() {
            return;
        }
        self.inner.borrow_mut().slots[self.slot].record = SlotRecord::TypeScript;
    }

    fn preparing(&self) -> Result<(), GuestError> {
        self.current()?;
        if self.binding() != VmBinding::Initializing {
            return Err(GuestError::invalid("VM registration is already bound"));
        }
        Ok(())
    }

    /// Mark this VM as the last called.
    pub fn called(&self) {
        if self.current().is_ok() {
            self.inner.borrow_mut().last_called = Some((self.slot, self.generation));
        }
    }

    /// Set the registry debug level.
    pub fn debug(&self, level: i32) {
        if self.current().is_ok() {
            self.inner.borrow_mut().debug_level = level;
        }
    }

    /// Print through the registry sink.
    pub fn print(&self, text: &str) {
        if self.current().is_ok() {
            (self.inner.borrow_mut().print)(text);
        }
    }

    /// Trace a `VM_Call` when debugging.
    pub fn print_call(&self, callnum: i32) {
        if self.current().is_err() {
            return;
        }
        let mut inner = self.inner.borrow_mut();
        if inner.debug_level != 0 {
            (inner.print)(&format!("VM_Call( {callnum} )\n"));
        }
    }

    /// Free the slot. Idempotent; other handles to the slot go stale.
    pub fn free(&self) {
        let mut inner = self.inner.borrow_mut();
        if inner.slots[self.slot].generation != self.generation {
            return;
        }
        inner.slots[self.slot] = Slot::empty();
        inner.slots[self.slot].generation = self.generation + 1;
        inner.last_called = None;
    }
}

fn lower(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_uppercase() {
                (character as u8 + 32) as char
            } else {
                character
            }
        })
        .collect()
}

/// Common-lived VM table with three slots.
#[derive(Debug, Clone)]
pub struct VmRegistry {
    inner: Rc<RefCell<RegistryInner>>,
}

impl Default for VmRegistry {
    fn default() -> Self {
        Self::new(Box::new(|_| {}), Box::new(|| QvmExecutionProfile::Release))
    }
}

impl VmRegistry {
    /// Fresh registry with `print` and `execution_profile` sinks.
    pub fn new(print: Box<dyn FnMut(&str)>, execution_profile: Box<dyn FnMut() -> QvmExecutionProfile>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(RegistryInner {
                slots: [Slot::empty(), Slot::empty(), Slot::empty()],
                last_called: None,
                debug_level: 0,
                print,
                execution_profile,
            })),
        }
    }

    /// Set the debug level.
    pub fn debug(&self, level: i32) {
        self.inner.borrow_mut().debug_level = level;
    }

    /// Reserve `name`, returning the existing registration on re-reserve.
    pub fn reserve(&self, name: &str) -> Result<VmRegistration, GuestError> {
        let end = name.find('\0').unwrap_or(name.len());
        let source_name = &name[..end];
        if source_name.is_empty() {
            return Err(qvm_fatal_error("VM_Create: bad parms"));
        }
        let inner = Rc::clone(&self.inner);
        {
            let state = inner.borrow();
            for slot in [0, 1, 2] {
                if let Some(existing) = state.slots[slot].name.as_ref() {
                    if lower(existing) == lower(source_name) {
                        return Ok(VmRegistration {
                            inner: Rc::clone(&inner),
                            slot,
                            generation: state.slots[slot].generation,
                            name: existing.clone(),
                        });
                    }
                }
            }
        }
        let mut state = inner.borrow_mut();
        let Some(slot) = [0, 1, 2].into_iter().find(|slot| state.slots[*slot].name.is_none()) else {
            return Err(qvm_fatal_error("VM_Create: no free vm_t"));
        };
        let short: String = source_name.chars().take(63).collect();
        let generation = state.slots[slot].generation;
        state.slots[slot].name = Some(short.clone());
        state.slots[slot].record = SlotRecord::Initializing {
            code_length: 0,
            table_length: 0,
            data_length: 1,
        };
        Ok(VmRegistration {
            inner,
            slot,
            generation,
            name: short,
        })
    }

    /// Free every slot.
    pub fn clear(&self) {
        let mut inner = self.inner.borrow_mut();
        for slot in inner.slots.iter_mut() {
            let generation = slot.generation + 1;
            *slot = Slot::empty();
            slot.generation = generation;
        }
        inner.last_called = None;
    }

    /// Print the registered VMs (`VM_VmInfo_f`).
    pub fn print_info(&self, print: &mut dyn FnMut(&str)) {
        print("Registered virtual machines:\n");
        let inner = self.inner.borrow();
        for cell in inner.slots.iter() {
            let Some(name) = cell.name.as_ref() else {
                break;
            };
            print(&format!("{name} : "));
            if matches!(cell.record, SlotRecord::TypeScript) {
                print("TypeScript replacement\n");
                continue;
            }
            print("interpreted\n");
            print(&format!("    code length : {:>7}\n", Self::code_length(cell)));
            print(&format!("    table length: {:>7}\n", Self::table_length(cell)));
            print(&format!("    data length : {:>7}\n", Self::data_length(cell)));
        }
    }

    /// Print the last-called interpreted VM's profile (a no-op otherwise).
    pub fn print_profile(
        &self,
        print: &mut dyn FnMut(&str),
        symbols: &mut QvmSymbols,
        debug_enabled: bool,
    ) -> Result<(), GuestError> {
        let interpreted = {
            let inner = self.inner.borrow();
            inner.last_called.is_some_and(|(slot, generation)| {
                inner.slots[slot].generation == generation
                    && matches!(inner.slots[slot].record, SlotRecord::Interpreted { .. })
            })
        };
        if interpreted {
            symbols.print_profile(print, debug_enabled)?;
        }
        Ok(())
    }

    fn code_length(cell: &Slot) -> usize {
        match &cell.record {
            SlotRecord::Interpreted { code_length, .. } | SlotRecord::Initializing { code_length, .. } => *code_length,
            _ => 0,
        }
    }

    fn table_length(cell: &Slot) -> usize {
        match &cell.record {
            SlotRecord::Interpreted { table_length, .. } | SlotRecord::Initializing { table_length, .. } => {
                *table_length
            }
            _ => 0,
        }
    }

    fn data_length(cell: &Slot) -> usize {
        match &cell.record {
            SlotRecord::Interpreted { data_length, .. } | SlotRecord::Initializing { data_length, .. } => *data_length,
            _ => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn registry() -> (VmRegistry, Rc<RefCell<Vec<String>>>) {
        let printed = Rc::new(RefCell::new(Vec::new()));
        let printed_clone = Rc::clone(&printed);
        let registry = VmRegistry::new(
            Box::new(move |text| printed_clone.borrow_mut().push(text.to_string())),
            Box::new(|| QvmExecutionProfile::Release),
        );
        (registry, printed)
    }

    #[test]
    fn reserve_reuses_names_case_insensitively() {
        let (registry, _) = registry();
        let first = registry.reserve("qagame").unwrap();
        assert_eq!(first.binding(), VmBinding::Initializing);
        let second = registry.reserve("QAGAME").unwrap();
        assert_eq!(second.name(), "qagame");
        first.bind_typescript();
        assert_eq!(second.binding(), VmBinding::TypeScript);
    }

    #[test]
    fn third_slot_exhaustion_is_fatal() {
        let (registry, _) = registry();
        registry.reserve("one").unwrap();
        registry.reserve("two").unwrap();
        registry.reserve("three").unwrap();
        let error = registry.reserve("four").unwrap_err();
        assert_eq!(error.to_string(), "fatal: VM_Create: no free vm_t");
        assert!(registry.reserve("").is_err());
    }

    #[test]
    fn freed_handles_go_stale() {
        let (registry, _) = registry();
        let registration = registry.reserve("qagame").unwrap();
        registration.free();
        assert_eq!(registration.binding(), VmBinding::Freed);
        registration.free();
        let fresh = registry.reserve("qagame").unwrap();
        assert_eq!(fresh.binding(), VmBinding::Initializing);
        assert_eq!(registration.binding(), VmBinding::Freed);
    }

    #[test]
    fn info_lists_slots_until_first_empty() {
        let (registry, printed) = registry();
        let game = registry.reserve("qagame").unwrap();
        game.bind_data(256);
        game.bind_instruction_pointers_length(12);
        game.bind_code_length(40);
        game.bind_interpreter(40, 12, 256);
        game.called();
        let ui = registry.reserve("ui").unwrap();
        ui.bind_typescript();
        {
            let output = Rc::new(RefCell::new(Vec::new()));
            let output_clone = Rc::clone(&output);
            registry.print_info(&mut move |text| output_clone.borrow_mut().push(text.to_string()));
            let joined = output.borrow().join("");
            assert!(joined.contains("qagame : interpreted"));
            assert!(joined.contains("code length :      40"));
            assert!(joined.contains("ui : TypeScript replacement"));
        }
        assert_eq!(printed.borrow().len(), 0);
        registry.debug(1);
        game.print_call(3);
        assert_eq!(*printed.borrow(), vec!["VM_Call( 3 )\n".to_string()]);
    }

    #[test]
    fn profile_prints_only_for_last_called_interpreted() {
        let (registry, _) = registry();
        let game = registry.reserve("qagame").unwrap();
        game.bind_interpreter(8, 4, 64);
        game.called();
        let mut symbols = QvmSymbols::new(vec![0], Box::new(|len, _| Ok(vec![0; len])), Box::new(|| Ok(())));
        let output = Rc::new(RefCell::new(Vec::new()));
        let output_clone = Rc::clone(&output);
        registry
            .print_profile(
                &mut move |text| output_clone.borrow_mut().push(text.to_string()),
                &mut symbols,
                false,
            )
            .unwrap();
        assert!(output.borrow().is_empty());
        registry.clear();
        assert_eq!(game.binding(), VmBinding::Freed);
    }
}
