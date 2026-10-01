//! Cgame status visibility masking for `cg_drawstatus`.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q3-client/status.ts`
//! (`effectiveStatusCvar`, `cgameStatusCvars`, `CgameStatusView`).
//! The Rust [`CvarHost`] surface carries no cvar names on reads, so the
//! wrapper records `cg_drawstatus` handles at bind time and keys its
//! change detection on the observed value instead of the source
//! modification count.

use std::collections::{HashMap, HashSet};

use qa_guest::qvm::client_state::{AbiProfile, HostCall, QvmRole, SyscallMemory};
use qa_guest::qvm::cvar_syscalls::{cvar_syscall, CvarHost, CvarValue, CvarVmBinding, QVM_CVAR_BYTES};
use qa_guest::qvm::legacy_bot_abi::{CG_CVAR_REGISTER, CG_CVAR_UPDATE};
use qa_guest::GuestError;

/// Status cvar masked while the destination hides source status.
const DRAW_STATUS: &str = "cg_drawstatus";

/// Cgame sees its effective status permission; archived engine cvars remain untouched.
#[must_use]
pub fn effective_status_cvar(value: &CvarValue, name: &str, visible: bool) -> CvarValue {
    if !name.eq_ignore_ascii_case(DRAW_STATUS) || visible {
        return value.clone();
    }
    CvarValue {
        value: "0".to_string(),
        numeric_value: 0.0,
        integer_value: 0,
    }
}

/// Last masked read, used to version guest-visible changes.
#[derive(Debug, Clone, PartialEq)]
struct MaskedPrevious {
    value: String,
    numeric_value: f32,
    integer_value: i32,
    visible: bool,
}

/// [`CvarHost`] wrapper that masks `cg_drawstatus` while hidden.
pub struct CgameStatusCvars<S, V> {
    inner: S,
    visible: V,
    draw_handles: HashSet<i32>,
    previous: Option<MaskedPrevious>,
    revision: i32,
}

impl<S: CvarHost, V: FnMut() -> bool> CgameStatusCvars<S, V> {
    /// Wrap a source cvar service with a visibility probe.
    pub fn new(source: S, visible: V) -> Self {
        Self {
            inner: source,
            visible,
            draw_handles: HashSet::new(),
            previous: None,
            revision: 0,
        }
    }

    /// Whether a VM handle names the masked status cvar.
    #[must_use]
    pub fn is_draw_status_handle(&self, handle: i32) -> bool {
        self.draw_handles.contains(&handle)
    }

    fn observe(&mut self, value: &CvarValue) -> CvarValue {
        let permission = (self.visible)();
        let changed = self.previous.as_ref().is_none_or(|previous| {
            previous.visible != permission
                || previous.value != value.value
                || previous.numeric_value != value.numeric_value
                || previous.integer_value != value.integer_value
        });
        if changed {
            if self.revision == i32::MAX {
                panic!("Cgame status cvar version exhausted");
            }
            self.revision += 1;
            self.previous = Some(MaskedPrevious {
                value: value.value.clone(),
                numeric_value: value.numeric_value,
                integer_value: value.integer_value,
                visible: permission,
            });
        }
        effective_status_cvar(value, DRAW_STATUS, permission)
    }

    fn binding(&mut self, value: &CvarVmBinding) -> CvarVmBinding {
        let observed = self.observe(&CvarValue {
            value: value.value.clone(),
            numeric_value: value.numeric_value,
            integer_value: value.integer_value,
        });
        CvarVmBinding {
            modification_count: self.revision,
            value: observed.value,
            numeric_value: observed.numeric_value,
            integer_value: observed.integer_value,
        }
    }
}

impl<S: CvarHost, V: FnMut() -> bool> CvarHost for CgameStatusCvars<S, V> {
    fn bind_vm(&mut self, name: &str, default: &str, flags: i32) -> i32 {
        let handle = self.inner.bind_vm(name, default, flags);
        if name.eq_ignore_ascii_case(DRAW_STATUS) {
            self.draw_handles.insert(handle);
        } else {
            self.draw_handles.remove(&handle);
        }
        handle
    }

    fn read_vm(&mut self, handle: i32) -> Option<CvarVmBinding> {
        let value = self.inner.read_vm(handle)?;
        if self.draw_handles.contains(&handle) {
            Some(self.binding(&value))
        } else {
            Some(value)
        }
    }

    fn get(&mut self, name: &str) -> Option<CvarValue> {
        let value = self.inner.get(name)?;
        if name.eq_ignore_ascii_case(DRAW_STATUS) {
            Some(self.observe(&value))
        } else {
            Some(value)
        }
    }

    fn set(&mut self, name: &str, value: &str) {
        self.inner.set(name, value);
    }

    fn set_value(&mut self, name: &str, value: f32) {
        self.inner.set_value(name, value);
    }

    fn reset(&mut self, name: &str) {
        self.inner.reset(name);
    }

    fn register(&mut self, name: &str, default: &str, flags: i32) {
        self.inner.register(name, default, flags);
    }

    fn info_string(&mut self, flags: i32) -> String {
        self.inner.info_string(flags)
    }
}

/// Tracks only original `vmCvar` registrations so a frame-scoped mask can
/// release its guest cache.
pub struct CgameStatusView<S, V> {
    cvars: CgameStatusCvars<S, V>,
    records: HashMap<i32, i32>,
}

impl<S: CvarHost, V: FnMut() -> bool> CgameStatusView<S, V> {
    /// Wrap a source cvar service with a visibility probe.
    pub fn new(source: S, visible: V) -> Self {
        Self {
            cvars: CgameStatusCvars::new(source, visible),
            records: HashMap::new(),
        }
    }

    /// Masked cvar service shared with the trap host.
    pub fn cvars(&mut self) -> &mut CgameStatusCvars<S, V> {
        &mut self.cvars
    }

    /// Dispatch a cgame cvar trap, tracking status registrations.
    pub fn syscall(&mut self, call: &HostCall, memory: &mut SyscallMemory) -> Result<Option<i32>, GuestError> {
        let result = cvar_syscall(call, memory, &mut self.cvars)?;
        if result.is_some() && (call.code == CG_CVAR_REGISTER || call.code == CG_CVAR_UPDATE) {
            let pointer = call.int(1)?;
            if memory.pointer(pointer).is_some() {
                let range = memory.span(pointer, QVM_CVAR_BYTES, 0)?;
                let handle = memory.read_i32(range.start)?;
                if self.cvars.is_draw_status_handle(handle) {
                    self.records.insert(pointer, handle);
                } else {
                    self.records.remove(&pointer);
                }
            }
        }
        Ok(result)
    }

    /// Re-run update traps for tracked status registrations.
    pub fn refresh(&mut self, memory: &mut SyscallMemory) -> Result<(), GuestError> {
        let pointers: Vec<(i32, i32)> = self
            .records
            .iter()
            .map(|(pointer, handle)| (*pointer, *handle))
            .collect();
        for (pointer, handle) in pointers {
            let range = memory.span(pointer, QVM_CVAR_BYTES, 0)?;
            if memory.read_i32(range.start)? != handle {
                self.records.remove(&pointer);
                continue;
            }
            let call = HostCall::engine(QvmRole::Cgame, CG_CVAR_UPDATE, &[pointer], AbiProfile::default());
            cvar_syscall(&call, memory, &mut self.cvars)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct StubCvars {
        values: HashMap<String, CvarValue>,
        bindings: HashMap<i32, CvarVmBinding>,
        names: HashMap<i32, String>,
        next: i32,
    }

    impl StubCvars {
        fn new() -> Self {
            Self {
                values: HashMap::from([
                    (
                        "cg_drawstatus".to_string(),
                        CvarValue {
                            value: "1".to_string(),
                            numeric_value: 1.0,
                            integer_value: 1,
                        },
                    ),
                    (
                        "cg_drawfps".to_string(),
                        CvarValue {
                            value: "1".to_string(),
                            numeric_value: 1.0,
                            integer_value: 1,
                        },
                    ),
                ]),
                bindings: HashMap::new(),
                names: HashMap::new(),
                next: 0,
            }
        }
    }

    impl CvarHost for StubCvars {
        fn bind_vm(&mut self, name: &str, default: &str, _flags: i32) -> i32 {
            let value = self.values.get(name).cloned().unwrap_or(CvarValue {
                value: default.to_string(),
                numeric_value: 0.0,
                integer_value: 0,
            });
            let handle = self.next;
            self.next += 1;
            self.names.insert(handle, name.to_string());
            self.bindings.insert(
                handle,
                CvarVmBinding {
                    modification_count: 7,
                    value: value.value,
                    numeric_value: value.numeric_value,
                    integer_value: value.integer_value,
                },
            );
            handle
        }

        fn read_vm(&mut self, handle: i32) -> Option<CvarVmBinding> {
            self.bindings.get(&handle).cloned()
        }

        fn get(&mut self, name: &str) -> Option<CvarValue> {
            self.values.get(&name.to_ascii_lowercase()).cloned()
        }

        fn set(&mut self, name: &str, value: &str) {
            if let Some(entry) = self.values.get_mut(name) {
                entry.value = value.to_string();
            }
            for (handle, bound) in self.names.clone() {
                if bound == name {
                    if let Some(binding) = self.bindings.get_mut(&handle) {
                        binding.value = value.to_string();
                    }
                }
            }
        }

        fn set_value(&mut self, _name: &str, _value: f32) {}
        fn reset(&mut self, _name: &str) {}
        fn register(&mut self, _name: &str, _default: &str, _flags: i32) {}
        fn info_string(&mut self, _flags: i32) -> String {
            String::new()
        }
    }

    fn masked() -> CgameStatusCvars<StubCvars, impl FnMut() -> bool> {
        CgameStatusCvars::new(StubCvars::new(), || false)
    }

    #[test]
    fn hidden_status_reads_zero() {
        let mut cvars = masked();
        let value = cvars.get("cg_drawstatus").expect("status cvar");
        assert_eq!(value.value, "0");
        assert_eq!(value.numeric_value, 0.0);
        assert_eq!(value.integer_value, 0);
        let other = cvars.get("cg_drawfps").expect("other cvar");
        assert_eq!(other.value, "1");
    }

    #[test]
    fn visible_status_passes_through() {
        let mut cvars = CgameStatusCvars::new(StubCvars::new(), || true);
        let value = cvars.get("CG_DrawStatus").expect("status cvar");
        assert_eq!(value.value, "1");
    }

    #[test]
    fn masked_reads_version_changes() {
        let mut cvars = masked();
        let handle = cvars.bind_vm("cg_drawstatus", "1", 0);
        let first = cvars.read_vm(handle).expect("binding");
        assert_eq!(first.modification_count, 1);
        assert_eq!(first.value, "0");
        let repeat = cvars.read_vm(handle).expect("binding");
        assert_eq!(repeat.modification_count, 1);
        cvars.set("cg_drawstatus", "2");
        let changed = cvars.read_vm(handle).expect("binding");
        assert_eq!(changed.modification_count, 2);
        assert_eq!(changed.value, "0");
        let plain = cvars.bind_vm("cg_drawfps", "1", 0);
        let unmasked = cvars.read_vm(plain).expect("binding");
        assert_eq!(unmasked.modification_count, 7);
    }

    #[test]
    fn visibility_flip_reversions() {
        let visible = std::rc::Rc::new(std::cell::Cell::new(false));
        let probe = visible.clone();
        let mut cvars = CgameStatusCvars::new(StubCvars::new(), move || probe.get());
        let handle = cvars.bind_vm("cg_drawstatus", "1", 0);
        assert_eq!(cvars.read_vm(handle).expect("binding").value, "0");
        visible.set(true);
        let shown = cvars.read_vm(handle).expect("binding");
        assert_eq!(shown.value, "1");
        assert_eq!(shown.modification_count, 2);
    }

    #[test]
    fn status_view_tracks_and_refreshes_registrations() {
        let mut memory = SyscallMemory::new(4096).expect("memory");
        let name = 64i32;
        memory.write_string(name, "cg_drawstatus", 64).expect("name");
        let record = 512i32;
        // Default string pointer must be readable; reuse the name storage.
        let mut view = CgameStatusView::new(StubCvars::new(), || false);
        let call = HostCall::engine(
            QvmRole::Cgame,
            CG_CVAR_REGISTER,
            &[record, name, name, 0],
            AbiProfile::Modern,
        );
        assert_eq!(view.syscall(&call, &mut memory).expect("register"), Some(0));
        assert_eq!(view.records.len(), 1);
        view.refresh(&mut memory).expect("refresh");
        assert_eq!(view.records.len(), 1);
        // A repurposed guest slot releases its tracking.
        let base = memory.pointer(record).expect("record");
        memory.write_i32(base, 99).expect("handle");
        view.refresh(&mut memory).expect("refresh");
        assert!(view.records.is_empty());
    }

    #[test]
    fn status_view_ignores_foreign_cvars() {
        let mut memory = SyscallMemory::new(4096).expect("memory");
        let name = 64i32;
        memory.write_string(name, "cg_drawfps", 64).expect("name");
        let record = 512i32;
        let mut view = CgameStatusView::new(StubCvars::new(), || false);
        let call = HostCall::engine(
            QvmRole::Cgame,
            CG_CVAR_REGISTER,
            &[record, name, name, 0],
            AbiProfile::Modern,
        );
        assert_eq!(view.syscall(&call, &mut memory).expect("register"), Some(0));
        assert!(view.records.is_empty());
    }
}
