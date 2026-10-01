//! Quake III client cvar services across seat registries.
//!
//! Port of `src/app/bootstrap/q3-client/cvars.ts` (`Q3ClientCvars`).
//! The owner set is injected through [`Q3ClientCvarOwners`]; handles,
//! revision tracking, and info-string projection follow the donor. The
//! Rust [`CvarRegistry`](qa_core::cvar::CvarRegistry) has no alias table,
//! so every name is already canonical and the donor's alias guard is
//! vacuous here. [`CvarHost`] is infallible, so donor failures record a
//! [`Q3ClientCvarError`] (drained with [`Q3ClientCvars::take_error`]) and
//! return the bridge's neutral value (`-1` handles, missing reads,
//! skipped registrations).

use qa_core::cmd::Dialect;
use qa_core::cvar::{flags, q2_flags, set_info_value, CvarError, CvarRegistry, CvarSnapshot, InfoOptions, InfoTarget};
use qa_guest::qvm::cvar_syscalls::{CvarHost, CvarValue, CvarVmBinding};
use thiserror::Error;

/// Maximum VM cvar handles (`MAX_CVARS`).
const MAX_CVARS: usize = 1024;
/// Info-string capacity used by the donor (`maximumLength` default).
const INFO_MAXIMUM_LENGTH: usize = 1024;

/// Quake III client cvar failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3ClientCvarError {
    /// Unknown guest cvars require a Q3 seat owner.
    #[error("Unknown guest cvars require a Q3 seat owner")]
    UnknownGuestCvar(String),
    /// VM cvar registration failed.
    #[error("VM cvar registration failed")]
    RegistrationFailed,
    /// Too many VM cvar handles (`MAX_CVARS`).
    #[error("MAX_CVARS")]
    MaxCvars,
    /// `Cvar_Update` handle out of range.
    #[error("Cvar_Update: handle out of range")]
    HandleOutOfRange(i32),
    /// Registry operation failed.
    #[error("{0}")]
    Registry(String),
}

impl From<CvarError> for Q3ClientCvarError {
    fn from(error: CvarError) -> Self {
        Self::Registry(error.to_string())
    }
}

/// Seat registries backing the client cvar service.
pub trait Q3ClientCvarOwners {
    /// Owning registry index for a cvar name.
    fn owner(&mut self, name: &str) -> usize;
    /// Visible registry indexes for info strings.
    fn visible(&self) -> Vec<usize>;
    /// Current registry index for a possibly retired owner.
    fn current(&mut self, owner: usize) -> usize;
    /// Borrow a registry.
    fn registry(&self, index: usize) -> &CvarRegistry;
    /// Borrow a registry mutably.
    fn registry_mut(&mut self, index: usize) -> &mut CvarRegistry;
    /// Print a diagnostic line.
    fn print(&mut self, text: &str);
}

/// One bound VM cvar handle.
#[derive(Debug, Clone)]
struct VmHandle {
    owner: usize,
    name: String,
    previous_owner: usize,
    previous: Option<CvarSnapshot>,
    revision: u32,
}

/// Client cvar services shared by the cgame and ui trap bridges.
pub struct Q3ClientCvars<S> {
    owners: S,
    handles: Vec<VmHandle>,
    error: Option<Q3ClientCvarError>,
}

impl<S: Q3ClientCvarOwners> Q3ClientCvars<S> {
    /// Wrap an owner set.
    pub fn new(owners: S) -> Self {
        Self {
            owners,
            handles: Vec::new(),
            error: None,
        }
    }

    /// Take the recorded donor failure, if any.
    #[must_use]
    pub fn take_error(&mut self) -> Option<Q3ClientCvarError> {
        self.error.take()
    }

    /// Borrow the owner set.
    #[must_use]
    pub fn owners(&self) -> &S {
        &self.owners
    }

    /// Borrow the owner set mutably.
    pub fn owners_mut(&mut self) -> &mut S {
        &mut self.owners
    }

    fn fail(&mut self, error: Q3ClientCvarError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    fn snapshot_value(value: &CvarSnapshot) -> CvarValue {
        CvarValue {
            value: value.value.clone(),
            numeric_value: value.numeric_value,
            integer_value: value.integer_value,
        }
    }

    fn try_register(
        &mut self,
        name: &str,
        default: &str,
        flags: i32,
    ) -> Result<Option<CvarSnapshot>, Q3ClientCvarError> {
        let owner = self.owners.owner(name);
        if self.owners.registry(owner).dialect() != Dialect::Q3 {
            let existing = self.owners.registry(owner).get(name);
            if existing.is_none() {
                return Err(Q3ClientCvarError::UnknownGuestCvar(name.to_string()));
            }
            return Ok(existing);
        }
        let registered = self.owners.registry_mut(owner).register(name, default, flags as u32)?;
        Ok(registered)
    }

    fn try_bind_vm(&mut self, name: &str, default: &str, flags: i32) -> Result<i32, Q3ClientCvarError> {
        let owner = self.owners.owner(name);
        let registered = self
            .try_register(name, default, flags)?
            .ok_or(Q3ClientCvarError::RegistrationFailed)?;
        for (index, entry) in self.handles.iter().enumerate() {
            if self.owners.current(entry.owner) == owner && entry.name == registered.name {
                return Ok(index as i32);
            }
        }
        if self.handles.len() >= MAX_CVARS {
            return Err(Q3ClientCvarError::MaxCvars);
        }
        self.handles.push(VmHandle {
            owner,
            name: registered.name,
            previous_owner: owner,
            previous: None,
            revision: 0,
        });
        Ok(self.handles.len() as i32 - 1)
    }

    fn try_read_vm(&mut self, handle: i32) -> Result<Option<CvarVmBinding>, Q3ClientCvarError> {
        if handle < 0 || handle as usize >= self.handles.len() {
            return Err(Q3ClientCvarError::HandleOutOfRange(handle));
        }
        let index = handle as usize;
        let owner = self.handles[index].owner;
        let current = self.owners.current(owner);
        let name = self.handles[index].name.clone();
        let Some(value) = self.owners.registry(current).get(&name) else {
            return Ok(None);
        };
        let entry = &mut self.handles[index];
        let changed = current != entry.previous_owner
            || entry.previous.as_ref().is_none_or(|previous| {
                value.value != previous.value
                    || value.numeric_value != previous.numeric_value
                    || value.integer_value != previous.integer_value
                    || value.modification_count != previous.modification_count
            });
        if changed {
            entry.revision = entry.revision.wrapping_add(1);
        }
        entry.previous_owner = current;
        entry.previous = Some(value.clone());
        Ok(Some(CvarVmBinding {
            modification_count: entry.revision as i32,
            value: value.value,
            numeric_value: value.numeric_value,
            integer_value: value.integer_value,
        }))
    }

    fn build_info_string(&mut self, flags: i32) -> String {
        let mut result = String::new();
        let flags = flags as u32;
        let target = if flags & flags::USER_INFO != 0 {
            InfoTarget::ClientUserinfo
        } else {
            InfoTarget::ServerInfo
        };
        let visible = self.owners.visible();
        for owner in visible {
            let snapshots = self.owners.registry(owner).snapshots(0);
            for value in snapshots {
                let dialect = self.owners.registry(owner).dialect();
                let projected = if dialect == Dialect::Q3 {
                    value.flags
                } else {
                    value.flags & (flags::USER_INFO | flags::SERVER_INFO)
                };
                if self.owners.owner(&value.name) != owner
                    || projected & flags == 0
                    || dialect.is_q2() && value.flags & q2_flags::PRIVATE != 0
                {
                    continue;
                }
                let options = InfoOptions {
                    dialect: Dialect::Q3,
                    maximum_length: INFO_MAXIMUM_LENGTH,
                    target,
                    server_high_characters: false,
                };
                let mut notes = Vec::new();
                match set_info_value(&result, &value.name, &value.value, options, &mut |line: &str| {
                    notes.push(line.to_string());
                }) {
                    Ok(next) => result = next,
                    Err(error) => {
                        self.fail(Q3ClientCvarError::from(error));
                        for note in notes {
                            self.owners.print(&note);
                        }
                        return result;
                    }
                }
                for note in notes {
                    self.owners.print(&note);
                }
            }
        }
        result
    }
}

impl<S: Q3ClientCvarOwners> CvarHost for Q3ClientCvars<S> {
    fn bind_vm(&mut self, name: &str, default: &str, flags: i32) -> i32 {
        match self.try_bind_vm(name, default, flags) {
            Ok(handle) => handle,
            Err(error) => {
                self.fail(error);
                -1
            }
        }
    }

    fn read_vm(&mut self, handle: i32) -> Option<CvarVmBinding> {
        match self.try_read_vm(handle) {
            Ok(binding) => binding,
            Err(error) => {
                self.fail(error);
                None
            }
        }
    }

    fn get(&mut self, name: &str) -> Option<CvarValue> {
        let owner = self.owners.owner(name);
        self.owners.registry(owner).get(name).as_ref().map(Self::snapshot_value)
    }

    fn set(&mut self, name: &str, value: &str) {
        let owner = self.owners.owner(name);
        if let Err(error) = self.owners.registry_mut(owner).set(name, value, false) {
            self.fail(Q3ClientCvarError::from(error));
        }
    }

    fn set_value(&mut self, name: &str, value: f32) {
        let owner = self.owners.owner(name);
        if let Err(error) = self.owners.registry_mut(owner).set_value(name, f64::from(value)) {
            self.fail(Q3ClientCvarError::from(error));
        }
    }

    fn reset(&mut self, name: &str) {
        let owner = self.owners.owner(name);
        if let Err(error) = self.owners.registry_mut(owner).reset(name, false) {
            self.fail(Q3ClientCvarError::from(error));
        }
    }

    fn register(&mut self, name: &str, default: &str, flags: i32) {
        if let Err(error) = self.try_register(name, default, flags) {
            self.fail(error);
        }
    }

    fn info_string(&mut self, flags: i32) -> String {
        self.build_info_string(flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Info flags that force Q3-seat ownership for unknown cvars.
    const INFO_FLAGS: u32 = flags::USER_INFO | flags::SERVER_INFO | flags::SYSTEM_INFO;

    struct StubOwners {
        registries: Vec<CvarRegistry>,
        printed: Vec<String>,
    }

    impl StubOwners {
        fn q3() -> Self {
            Self {
                registries: vec![CvarRegistry::new(Dialect::Q3)],
                printed: Vec::new(),
            }
        }

        fn mixed() -> Self {
            Self {
                registries: vec![CvarRegistry::new(Dialect::Q3), CvarRegistry::new(Dialect::Q2Classic)],
                printed: Vec::new(),
            }
        }
    }

    impl Q3ClientCvarOwners for StubOwners {
        fn owner(&mut self, name: &str) -> usize {
            if self.registries.len() > 1 && name.starts_with("q2_") {
                1
            } else {
                0
            }
        }

        fn visible(&self) -> Vec<usize> {
            (0..self.registries.len()).collect()
        }

        fn current(&mut self, owner: usize) -> usize {
            owner
        }

        fn registry(&self, index: usize) -> &CvarRegistry {
            &self.registries[index]
        }

        fn registry_mut(&mut self, index: usize) -> &mut CvarRegistry {
            &mut self.registries[index]
        }

        fn print(&mut self, text: &str) {
            self.printed.push(text.to_string());
        }
    }

    #[test]
    fn bind_versions_value_changes() {
        let mut cvars = Q3ClientCvars::new(StubOwners::q3());
        let handle = cvars.bind_vm("cg_drawfps", "1", 0);
        assert_eq!(handle, 0);
        assert_eq!(cvars.bind_vm("cg_drawfps", "0", 0), 0);
        let first = cvars.read_vm(handle).expect("binding");
        assert_eq!(first.modification_count, 1);
        assert_eq!(first.value, "1");
        let repeat = cvars.read_vm(handle).expect("binding");
        assert_eq!(repeat.modification_count, 1);
        cvars.set("cg_drawfps", "0");
        let changed = cvars.read_vm(handle).expect("binding");
        assert_eq!(changed.modification_count, 2);
        assert_eq!(changed.value, "0");
        assert_eq!(cvars.take_error(), None);
    }

    #[test]
    fn bad_handle_records_range_error() {
        let mut cvars = Q3ClientCvars::new(StubOwners::q3());
        assert!(cvars.read_vm(0).is_none());
        assert_eq!(cvars.take_error(), Some(Q3ClientCvarError::HandleOutOfRange(0)));
        assert!(cvars.read_vm(-1).is_none());
        assert_eq!(cvars.take_error(), Some(Q3ClientCvarError::HandleOutOfRange(-1)));
    }

    #[test]
    fn non_q3_owner_rejects_unknown_cvars() {
        let mut cvars = Q3ClientCvars::new(StubOwners::mixed());
        cvars.register("q2_missing", "1", INFO_FLAGS as i32);
        assert_eq!(
            cvars.take_error(),
            Some(Q3ClientCvarError::UnknownGuestCvar("q2_missing".to_string()))
        );
        cvars.owners_mut().registries[1]
            .register("q2_known", "1", 0)
            .expect("register");
        cvars.register("q2_known", "2", INFO_FLAGS as i32);
        assert_eq!(cvars.take_error(), None);
        assert_eq!(cvars.bind_vm("q2_missing", "1", 0), -1);
        assert_eq!(
            cvars.take_error(),
            Some(Q3ClientCvarError::UnknownGuestCvar("q2_missing".to_string()))
        );
    }

    #[test]
    fn info_string_projects_visible_flags() {
        let mut cvars = Q3ClientCvars::new(StubOwners::mixed());
        cvars.register("name", "player", flags::USER_INFO as i32);
        cvars.register("g_gametype", "0", flags::SERVER_INFO as i32);
        cvars.owners_mut().registries[1]
            .register("q2_private", "x", flags::USER_INFO | q2_flags::PRIVATE)
            .expect("register");
        let userinfo = cvars.info_string(flags::USER_INFO as i32);
        assert!(userinfo.contains("name"), "userinfo: {userinfo}");
        assert!(!userinfo.contains("g_gametype"), "userinfo: {userinfo}");
        assert!(!userinfo.contains("q2_private"), "userinfo: {userinfo}");
        let server = cvars.info_string(flags::SERVER_INFO as i32);
        assert!(server.contains("g_gametype"), "server: {server}");
        assert!(!server.contains("name"), "server: {server}");
        assert_eq!(cvars.take_error(), None);
    }

    #[test]
    fn get_set_value_and_reset_round_trip() {
        let mut cvars = Q3ClientCvars::new(StubOwners::q3());
        cvars.register("sensitivity", "5", 0);
        cvars.set_value("sensitivity", 9.5);
        let value = cvars.get("sensitivity").expect("value");
        assert_eq!(value.integer_value, 9);
        cvars.set("sensitivity", "3");
        assert_eq!(cvars.get("sensitivity").expect("value").value, "3");
        cvars.reset("sensitivity");
        assert_eq!(cvars.get("sensitivity").expect("value").value, "5");
        assert_eq!(cvars.take_error(), None);
    }
}
