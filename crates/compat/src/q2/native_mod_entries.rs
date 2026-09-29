//! Port of `src/compat/q2/native-mod-entries.ts`.
//! Bridges one native entry point: guest calls are intercepted while the exact
//! original frame bypasses the interceptor; recursive source calls still compose.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue};
use thiserror::Error;

/// Failures binding or invoking a native entry.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EntryError {
    /// No original body was registered for this entry.
    #[error("native entry {0:#x} has no registered original body")]
    UnknownEntry(u64),
    /// The binding was closed.
    #[error("native entry binding {0} is closed")]
    BindingClosed(String),
}

/// Original guest body behind an entry.
pub type OriginalFn = dyn Fn(&[GuestCallValue]) -> GuestCallResult;
/// Interceptor: receives the call plus a handle to the bypassed original.
pub type InterceptFn = dyn Fn(&[GuestCallValue], &dyn Fn(&[GuestCallValue]) -> GuestCallResult) -> GuestCallResult;

struct BindingSlot {
    id: String,
    execute: Rc<InterceptFn>,
    accepts: Rc<dyn Fn() -> bool>,
    active: bool,
}

struct Inner {
    originals: HashMap<u64, Rc<OriginalFn>>,
    bindings: HashMap<u64, BindingSlot>,
    /// Single-shot bypass tokens; each `original` call pushes one, consumed by
    /// the immediately following invoke of that entry only.
    bypass: Vec<u64>,
}

/// Headless native-entry host: registered original bodies plus interceptors.
#[derive(Debug, Clone, Default)]
pub struct SyntheticEntryHost {
    inner: Rc<RefCell<Inner>>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            originals: HashMap::new(),
            bindings: HashMap::new(),
            bypass: Vec::new(),
        }
    }
}

impl SyntheticEntryHost {
    /// Build an empty host.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the original guest body for an entry address.
    pub fn register_original(
        &self,
        address: GuestAddress,
        original: impl Fn(&[GuestCallValue]) -> GuestCallResult + 'static,
    ) {
        self.inner
            .borrow_mut()
            .originals
            .insert(address.offset, Rc::new(original));
    }

    /// Invoke an entry, running its interceptor unless bypassed or declined.
    pub fn invoke(&self, address: GuestAddress, values: &[GuestCallValue]) -> Result<GuestCallResult, EntryError> {
        let (execute, bypassed) = {
            let mut inner = self.inner.borrow_mut();
            if inner.bypass.last() == Some(&address.offset) {
                inner.bypass.pop();
                (None, true)
            } else {
                let slot = inner.bindings.get(&address.offset);
                let intercept = slot.map(|slot| slot.active && (slot.accepts)()).unwrap_or(false);
                let execute = if intercept {
                    slot.map(|slot| slot.execute.clone())
                } else {
                    None
                };
                (execute, false)
            }
        };
        if let Some(execute) = execute {
            let host = self.clone();
            return Ok(execute(values, &|nested| {
                host.invoke_original_bypassed(address, nested)
                    .unwrap_or(GuestCallResult::Void)
            }));
        }
        if bypassed {
            return self.run_original(address, values);
        }
        self.run_original(address, values)
    }

    fn run_original(&self, address: GuestAddress, values: &[GuestCallValue]) -> Result<GuestCallResult, EntryError> {
        let original = self
            .inner
            .borrow()
            .originals
            .get(&address.offset)
            .cloned()
            .ok_or(EntryError::UnknownEntry(address.offset))?;
        Ok(original(values))
    }

    fn invoke_original_bypassed(
        &self,
        address: GuestAddress,
        values: &[GuestCallValue],
    ) -> Result<GuestCallResult, EntryError> {
        self.inner.borrow_mut().bypass.push(address.offset);
        let result = self.invoke(address, values);
        // `invoke` consumes the token on the bypass path; drop it if invoke
        // failed first so a later call is not wrongly bypassed. The token is
        // gone while the original body runs, so a recursive source call made
        // by that body intercepts again.
        let mut inner = self.inner.borrow_mut();
        if inner.bypass.last() == Some(&address.offset) {
            inner.bypass.pop();
        }
        result
    }

    fn deactivate(&self, address: GuestAddress, id: &str) {
        if let Some(slot) = self.inner.borrow_mut().bindings.get_mut(&address.offset) {
            if slot.id == id {
                slot.active = false;
            }
        }
    }
}

/// Live interception of one native entry.
#[derive(Debug, Clone)]
pub struct NativeModEntryBinding {
    host: SyntheticEntryHost,
    address: GuestAddress,
    id: String,
}

impl NativeModEntryBinding {
    /// Call the original body, bypassing exactly this frame.
    pub fn original(&self, values: &[GuestCallValue]) -> Result<GuestCallResult, EntryError> {
        self.host.invoke_original_bypassed(self.address, values)
    }

    /// Remove the interception; the original body serves future calls.
    pub fn close(&self) {
        self.host.deactivate(self.address, &self.id);
    }
}

/// Intercept `address`, mirroring `bindNativeModEntry`.
pub fn bind_native_mod_entry(
    host: &SyntheticEntryHost,
    address: GuestAddress,
    id: &str,
    execute: impl Fn(&[GuestCallValue], &dyn Fn(&[GuestCallValue]) -> GuestCallResult) -> GuestCallResult + 'static,
    accepts: impl Fn() -> bool + 'static,
) -> Result<NativeModEntryBinding, EntryError> {
    if !host.inner.borrow().originals.contains_key(&address.offset) {
        return Err(EntryError::UnknownEntry(address.offset));
    }
    host.inner.borrow_mut().bindings.insert(
        address.offset,
        BindingSlot {
            id: id.to_string(),
            execute: Rc::new(execute),
            accepts: Rc::new(accepts),
            active: true,
        },
    );
    Ok(NativeModEntryBinding {
        host: host.clone(),
        address,
        id: id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn addr(offset: u64) -> GuestAddress {
        GuestAddress::new(7, offset)
    }

    #[test]
    fn intercept_wraps_original_and_close_restores_it() {
        let host = SyntheticEntryHost::new();
        host.register_original(addr(0x100), |_| GuestCallResult::Value(GuestCallValue::Int32(1)));
        let intercepted = Rc::new(Cell::new(0));
        let seen = intercepted.clone();
        let binding = bind_native_mod_entry(
            &host,
            addr(0x100),
            "mod:entry",
            move |values, original| {
                seen.set(seen.get() + 1);
                let result = original(values);
                assert!(matches!(result, GuestCallResult::Value(GuestCallValue::Int32(1))));
                GuestCallResult::Value(GuestCallValue::Int32(11))
            },
            || true,
        )
        .unwrap();
        let result = host.invoke(addr(0x100), &[]).unwrap();
        assert!(matches!(result, GuestCallResult::Value(GuestCallValue::Int32(11))));
        assert_eq!(intercepted.get(), 1);
        let direct = binding.original(&[]).unwrap();
        assert!(matches!(direct, GuestCallResult::Value(GuestCallValue::Int32(1))));
        assert_eq!(intercepted.get(), 1);
        binding.close();
        let restored = host.invoke(addr(0x100), &[]).unwrap();
        assert!(matches!(restored, GuestCallResult::Value(GuestCallValue::Int32(1))));
        assert_eq!(intercepted.get(), 1);
    }

    #[test]
    fn recursive_source_call_inside_original_still_intercepts() {
        let host = SyntheticEntryHost::new();
        let inner = host.clone();
        let recurse = Rc::new(Cell::new(true));
        let flag = recurse.clone();
        host.register_original(addr(0x200), move |_| {
            if flag.replace(false) {
                let nested = inner.invoke(addr(0x200), &[]).unwrap();
                assert!(matches!(nested, GuestCallResult::Value(GuestCallValue::Int32(5))));
            }
            GuestCallResult::Value(GuestCallValue::Int32(5))
        });
        let intercepted = Rc::new(Cell::new(0));
        let seen = intercepted.clone();
        bind_native_mod_entry(
            &host,
            addr(0x200),
            "mod:recursive",
            move |values, original| {
                seen.set(seen.get() + 1);
                original(values)
            },
            || true,
        )
        .unwrap();
        let result = host.invoke(addr(0x200), &[]).unwrap();
        assert!(matches!(result, GuestCallResult::Value(GuestCallValue::Int32(5))));
        assert_eq!(intercepted.get(), 2);
    }

    #[test]
    fn declined_or_missing_entries_fall_through_or_fail() {
        let host = SyntheticEntryHost::new();
        host.register_original(addr(0x300), |_| GuestCallResult::Void);
        bind_native_mod_entry(
            &host,
            addr(0x300),
            "mod:declined",
            |_, _| GuestCallResult::Value(GuestCallValue::Int32(9)),
            || false,
        )
        .unwrap();
        assert!(matches!(host.invoke(addr(0x300), &[]).unwrap(), GuestCallResult::Void));
        assert_eq!(host.invoke(addr(0x999), &[]), Err(EntryError::UnknownEntry(0x999)));
    }
}
