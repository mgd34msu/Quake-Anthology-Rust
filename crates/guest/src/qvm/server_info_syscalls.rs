//! Server information traps: configstrings and server info.
//!
//! Provenance: `src/compat/qvm/server-info-syscalls.ts` (Q3
//! `server/sv_game.c` information traps shared by primary and component
//! modules).

use super::client_state::{AbiProfile, HostCall, QvmRole, SyscallMemory};
use super::legacy_bot_abi::{G_GET_CONFIGSTRING, G_GET_SERVERINFO, G_SET_CONFIGSTRING};
use super::legacy_presentation::qvm_configstring;
use crate::error::GuestError;

/// Host information surface used by the traps.
pub trait ServerInformationHost {
    /// Selected ABI profile for configstring index translation.
    fn abi_profile(&self) -> AbiProfile;
    /// Configstring value at a canonical index.
    fn config_get(&mut self, index: i32) -> String;
    /// Set a configstring at a canonical index.
    fn config_set(&mut self, index: i32, value: &str);
    /// Server-info string (the `ServerInfo` cvar info string in the donor).
    fn server_info(&mut self) -> String;
}

fn capacity(size: i32, operation: &str) -> Result<(), GuestError> {
    if size < 1 {
        return Err(GuestError::runtime(format!("{operation}: bufferSize == {size}")));
    }
    Ok(())
}

fn config_index(index: i32, operation: &str) -> Result<(), GuestError> {
    if !(0..1024).contains(&index) {
        return Err(GuestError::runtime(format!("{operation}: bad index {index}\n")));
    }
    Ok(())
}

/// Dispatch a server-information trap. Returns `Ok(None)` when unhandled.
pub fn server_information_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    services: &mut dyn ServerInformationHost,
) -> Result<Option<i32>, GuestError> {
    if call.kind != super::client_state::CallKind::Engine || call.role != QvmRole::Qagame {
        return Ok(None);
    }
    match call.code {
        G_SET_CONFIGSTRING => {
            let index = call.int(1)?;
            config_index(index, "SV_SetConfigstring")?;
            let value = if call.int(2)? == 0 {
                String::new()
            } else {
                memory.read_string(call.int(2)?)?
            };
            services.config_set(qvm_configstring(index, services.abi_profile())?, &value);
            Ok(Some(0))
        }
        G_GET_CONFIGSTRING => {
            let index = call.int(1)?;
            let word = call.int(2)?;
            let size = call.int(3)?;
            capacity(size, "SV_GetConfigstring")?;
            config_index(index, "SV_GetConfigstring")?;
            let value = services.config_get(qvm_configstring(index, services.abi_profile())?);
            memory.write_string(word, &value, size as usize)?;
            Ok(Some(0))
        }
        G_GET_SERVERINFO => {
            let word = call.int(1)?;
            let size = call.int(2)?;
            capacity(size, "SV_GetServerinfo")?;
            let info = services.server_info();
            memory.write_string(word, &info, size as usize)?;
            Ok(Some(0))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::super::client_state::CallKind;
    use super::*;

    struct FakeInfo {
        profile: AbiProfile,
        configs: Vec<String>,
        log: Vec<String>,
    }

    impl FakeInfo {
        fn new(profile: AbiProfile) -> Self {
            Self {
                profile,
                configs: vec![String::new(); 1024],
                log: Vec::new(),
            }
        }
    }

    impl ServerInformationHost for FakeInfo {
        fn abi_profile(&self) -> AbiProfile {
            self.profile
        }
        fn config_get(&mut self, index: i32) -> String {
            self.configs[index as usize].clone()
        }
        fn config_set(&mut self, index: i32, value: &str) {
            self.log.push(format!("set {index}={value}"));
            self.configs[index as usize] = value.to_string();
        }
        fn server_info(&mut self) -> String {
            "\\map\\q3dm1".to_string()
        }
    }

    fn call(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Qagame, code, args, AbiProfile::Modern)
    }

    #[test]
    fn set_and_get_configstring() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(512, "hello", 6).unwrap();
        let mut services = FakeInfo::new(AbiProfile::Modern);
        assert_eq!(
            server_information_syscall(&call(G_SET_CONFIGSTRING, &[10, 512]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(
            server_information_syscall(&call(G_GET_CONFIGSTRING, &[10, 256, 64]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_string(256).unwrap(), "hello");
        assert_eq!(services.log, vec!["set 10=hello".to_string()]);
    }

    #[test]
    fn set_with_null_value_clears() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut services = FakeInfo::new(AbiProfile::Modern);
        assert_eq!(
            server_information_syscall(&call(G_SET_CONFIGSTRING, &[10, 0]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(services.configs[10], "");
    }

    #[test]
    fn get_serverinfo() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut services = FakeInfo::new(AbiProfile::Modern);
        assert_eq!(
            server_information_syscall(&call(G_GET_SERVERINFO, &[256, 64]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_string(256).unwrap(), "\\map\\q3dm1");
    }

    #[test]
    fn rejects_bad_index_and_capacity() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut services = FakeInfo::new(AbiProfile::Modern);
        assert!(server_information_syscall(&call(G_SET_CONFIGSTRING, &[1024, 0]), &mut memory, &mut services).is_err());
        assert!(
            server_information_syscall(&call(G_GET_CONFIGSTRING, &[-1, 256, 64]), &mut memory, &mut services).is_err()
        );
        assert!(
            server_information_syscall(&call(G_GET_CONFIGSTRING, &[1, 256, 0]), &mut memory, &mut services).is_err()
        );
        assert!(server_information_syscall(&call(G_GET_SERVERINFO, &[256, 0]), &mut memory, &mut services).is_err());
    }

    #[test]
    fn legacy_index_translation_and_routing() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(512, "v", 2).unwrap();
        let mut services = FakeInfo::new(AbiProfile::Legacy);
        assert_eq!(
            server_information_syscall(&call(G_SET_CONFIGSTRING, &[12, 512]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(services.log, vec!["set 20=v".to_string()]);
        let other = HostCall {
            kind: CallKind::Engine,
            role: QvmRole::Cgame,
            ..call(G_GET_SERVERINFO, &[256, 64])
        };
        assert_eq!(
            server_information_syscall(&other, &mut memory, &mut services).unwrap(),
            None
        );
        assert_eq!(
            server_information_syscall(&call(999, &[]), &mut memory, &mut services).unwrap(),
            None
        );
    }
}
