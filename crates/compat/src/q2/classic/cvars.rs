//! Donor: `src/compat/q2/classic/cvars.ts` — the native `cvar_t` view of the
//! engine registry.
//!
//! Bridges the engine-owned variable registry to stable 28-byte guest
//! `cvar_t` records chained through their `next` pointers.

use std::collections::HashMap;

use qa_guest::core::contracts::{GuestAddress, GuestAllocationOptions};
use qa_guest::core::memory::SparseGuestMemory;

use super::layout::ClassicResult;
use super::records::allocate_classic_string;

/// One engine-side variable snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicCvarState {
    /// Variable name.
    pub name: String,
    /// Current string value.
    pub value: String,
    /// Latched value awaiting application, if any.
    pub latched: Option<String>,
    /// Variable flags.
    pub flags: i32,
    /// Modified since last read.
    pub modified: bool,
    /// Numeric value.
    pub numeric: f32,
}

/// Synthetic engine-side variable registry backing the guest view.
#[derive(Debug, Default)]
pub struct ClassicCvarRegistry {
    vars: Vec<ClassicCvarState>,
}

impl ClassicCvarRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self { vars: Vec::new() }
    }

    /// Register a variable, keeping the existing value when present.
    pub fn register(&mut self, name: &str, value: &str, flags: i32) {
        if let Some(state) = self.vars.iter_mut().find(|state| state.name == name) {
            state.flags = flags;
            return;
        }
        self.vars.push(ClassicCvarState {
            name: name.to_string(),
            value: value.to_string(),
            latched: None,
            flags,
            modified: false,
            numeric: value.parse().unwrap_or(0.0),
        });
    }

    /// Set a variable value (forced sets also clear any latch).
    pub fn set(&mut self, name: &str, value: &str, force: bool) {
        if let Some(state) = self.vars.iter_mut().find(|state| state.name == name) {
            state.value = value.to_string();
            state.modified = true;
            state.numeric = value.parse().unwrap_or(0.0);
            if force {
                state.latched = None;
            }
            return;
        }
        self.register(name, value, 0);
        if let Some(state) = self.vars.iter_mut().find(|state| state.name == name) {
            state.modified = true;
        }
    }

    /// Find one variable by name.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&ClassicCvarState> {
        self.vars.iter().find(|state| state.name == name)
    }

    /// Numeric value of a variable, or zero when absent.
    #[must_use]
    pub fn variable_value(&self, name: &str) -> f32 {
        self.find(name).map_or(0.0, |state| state.numeric)
    }

    /// Snapshot every variable in registration order.
    #[must_use]
    pub fn snapshots(&self) -> Vec<ClassicCvarState> {
        self.vars.clone()
    }
}

/// Guest offsets of one `cvar_t` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GuestCvar {
    address: GuestAddress,
    string: GuestAddress,
    latched: GuestAddress,
    flags: GuestAddress,
    modified: GuestAddress,
    value: GuestAddress,
    next: GuestAddress,
}

/// Native `cvar_t` records as a stable guest view of the registry.
#[derive(Debug)]
pub struct ClassicQ2Cvars {
    registry: ClassicCvarRegistry,
    records: HashMap<String, GuestCvar>,
    strings: HashMap<String, GuestAddress>,
}

impl ClassicQ2Cvars {
    /// Create the guest view over a registry.
    #[must_use]
    pub fn new(registry: ClassicCvarRegistry) -> Self {
        Self {
            registry,
            records: HashMap::new(),
            strings: HashMap::new(),
        }
    }

    /// The engine-side registry.
    #[must_use]
    pub fn registry(&self) -> &ClassicCvarRegistry {
        &self.registry
    }

    /// The engine-side registry, mutably.
    pub fn registry_mut(&mut self) -> &mut ClassicCvarRegistry {
        &mut self.registry
    }

    /// Intern a guest string, reusing the previous allocation.
    pub fn string(
        &mut self,
        memory: &mut SparseGuestMemory,
        value: &str,
    ) -> ClassicResult<GuestAddress> {
        if let Some(address) = self.strings.get(value) {
            return Ok(*address);
        }
        let address = allocate_classic_string(memory, value)?;
        self.strings.insert(value.to_string(), address);
        Ok(address)
    }

    /// Guest record for a variable, refreshing the view first.
    pub fn pointer(
        &mut self,
        memory: &mut SparseGuestMemory,
        name: &str,
    ) -> ClassicResult<Option<GuestAddress>> {
        if self.registry.find(name).is_none() {
            return Ok(None);
        }
        self.refresh(memory)?;
        Ok(self.records.get(name).map(|record| record.address))
    }

    fn update(
        &mut self,
        memory: &mut SparseGuestMemory,
        state: &ClassicCvarState,
    ) -> ClassicResult<GuestCvar> {
        if !self.records.contains_key(&state.name) {
            let address = memory.allocate(&GuestAllocationOptions {
                byte_length: 28,
                alignment: 4,
                permissions: qa_guest::core::contracts::GuestPermissions::ReadWrite,
                label: format!("API 3 cvar {}", state.name),
            })?;
            self.records.insert(
                state.name.clone(),
                GuestCvar {
                    address,
                    string: memory.offset(address, 4)?,
                    latched: memory.offset(address, 8)?,
                    flags: memory.offset(address, 12)?,
                    modified: memory.offset(address, 16)?,
                    value: memory.offset(address, 20)?,
                    next: memory.offset(address, 24)?,
                },
            );
        }
        let record = self.records[&state.name];
        let name = self.string(memory, &state.name)?;
        memory.write_pointer(record.address, Some(name))?;
        let value = self.string(memory, &state.value)?;
        memory.write_pointer(record.string, Some(value))?;
        let latched = match &state.latched {
            Some(latched) => Some(self.string(memory, latched)?),
            None => None,
        };
        memory.write_pointer(record.latched, latched)?;
        memory.write_i32(record.flags, state.flags)?;
        memory.write_i32(record.modified, i32::from(state.modified))?;
        memory.write_f32(record.value, state.numeric)?;
        Ok(record)
    }

    /// Rewrite every guest record and rechain the `next` pointers.
    pub fn refresh(&mut self, memory: &mut SparseGuestMemory) -> ClassicResult<()> {
        let snapshots = self.registry.snapshots();
        let mut records = Vec::with_capacity(snapshots.len());
        for state in &snapshots {
            records.push(self.update(memory, state)?);
        }
        for (index, record) in records.iter().enumerate() {
            let next = records.get(index + 1).map(|record| record.address);
            memory.write_pointer(record.next, next)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, ModuleIdentity};
    use super::super::records::read_classic_string;

    fn test_memory() -> SparseGuestMemory {
        SparseGuestMemory::new(
            ModuleIdentity::new(
                ProviderId::new("q2", "cvar-test"),
                "cvars",
                ContentDigest::new("sha256", "0"),
                "test",
            ),
            4,
            0x10000,
        )
        .unwrap()
    }

    #[test]
    fn records_mirror_registry_and_chain_next() {
        let mut memory = test_memory();
        let mut cvars = ClassicQ2Cvars::new(ClassicCvarRegistry::new());
        cvars.registry_mut().register("dmflags", "64", 1);
        cvars.registry_mut().register("coop", "0", 0);
        let dmflags = cvars.pointer(&mut memory, "dmflags").unwrap().unwrap();
        let coop = cvars.pointer(&mut memory, "coop").unwrap().unwrap();
        assert_ne!(dmflags, coop);
        assert!(cvars.pointer(&mut memory, "missing").unwrap().is_none());
        let name = memory.read_pointer(dmflags).unwrap().unwrap();
        assert_eq!(read_classic_string(&mut memory, Some(name), 64).unwrap(), "dmflags");
        let value = memory.read_pointer(memory.offset(dmflags, 4).unwrap()).unwrap().unwrap();
        assert_eq!(read_classic_string(&mut memory, Some(value), 64).unwrap(), "64");
        assert_eq!(memory.read_i32(memory.offset(dmflags, 12).unwrap()).unwrap(), 1);
        assert_eq!(memory.read_f32(memory.offset(dmflags, 20).unwrap()).unwrap(), 64.0);
        let next = memory.read_pointer(memory.offset(dmflags, 24).unwrap()).unwrap().unwrap();
        assert_eq!(next, coop);
        assert!(memory.read_pointer(memory.offset(coop, 24).unwrap()).unwrap().is_none());
        assert_eq!(cvars.string(&mut memory, "dmflags").unwrap(), name);
    }

    #[test]
    fn sets_update_values_and_modified_bits() {
        let mut memory = test_memory();
        let mut cvars = ClassicQ2Cvars::new(ClassicCvarRegistry::new());
        cvars.registry_mut().register("skill", "1", 0);
        cvars.registry_mut().set("skill", "3", false);
        assert_eq!(cvars.registry().variable_value("skill"), 3.0);
        assert_eq!(cvars.registry().variable_value("absent"), 0.0);
        let record = cvars.pointer(&mut memory, "skill").unwrap().unwrap();
        let value = memory.read_pointer(memory.offset(record, 4).unwrap()).unwrap().unwrap();
        assert_eq!(read_classic_string(&mut memory, Some(value), 64).unwrap(), "3");
        assert_eq!(memory.read_i32(memory.offset(record, 16).unwrap()).unwrap(), 1);
        cvars.registry_mut().set("fresh", "7", true);
        assert!(cvars.pointer(&mut memory, "fresh").unwrap().is_some());
    }
}
