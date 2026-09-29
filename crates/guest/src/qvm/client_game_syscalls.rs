//! Server-side client traps: userinfo, user commands, drops, commands.
//!
//! Provenance: `src/compat/qvm/client-game-syscalls.ts` (Q3
//! `server/sv_game.c` client imports). Donor promises become direct returns.

use super::client_state::{AbiProfile, CallKind, HostCall, QvmRole, SyscallMemory, WireUserCommand};
use super::client_state_record::{write_user_command, UserCommandWrite, QVM_USER_COMMAND_BYTES};
use super::legacy_bot_abi::{G_DROP_CLIENT, G_GET_USERCMD, G_GET_USERINFO, G_SEND_SERVER_COMMAND, G_SET_USERINFO};
use crate::error::GuestError;

/// Host client-table surface used by the traps.
pub trait ClientGameHost {
    /// Selected ABI profile for user-command layout.
    fn abi_profile(&self) -> AbiProfile;
    /// Maximum client count.
    fn max_clients(&self) -> i32;
    /// Userinfo string for a slot.
    fn get_userinfo(&mut self, slot: i32) -> String;
    /// Set the userinfo string for a slot.
    fn set_userinfo(&mut self, slot: i32, value: &str);
    /// Current user command for a slot.
    fn get_user_command(&mut self, slot: i32) -> WireUserCommand;
    /// Drop a client with a reason.
    fn drop_client(&mut self, slot: i32, reason: &str);
    /// Send a server command to a slot (`-1` broadcasts).
    fn send_server_command(&mut self, slot: i32, text: &str);
}

fn check_client(slot: i32, services: &dyn ClientGameHost, operation: &str) -> Result<(), GuestError> {
    if slot < 0 || slot >= services.max_clients() {
        return Err(if operation == "SV_GetUsercmd" {
            GuestError::runtime(format!("{operation}: bad clientNum:{slot}"))
        } else {
            GuestError::runtime(format!("{operation}: bad index {slot}\n"))
        });
    }
    Ok(())
}

/// Dispatch a server-side client trap. Returns `Ok(None)` when unhandled.
pub fn client_game_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    services: &mut dyn ClientGameHost,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine || call.role != QvmRole::Qagame {
        return Ok(None);
    }
    match call.code {
        G_DROP_CLIENT => {
            let number = call.int(1)?;
            if number < 0 || number >= services.max_clients() {
                return Ok(Some(0));
            }
            let reason = memory.read_string(call.int(2)?)?;
            services.drop_client(number, &reason);
            Ok(Some(0))
        }
        G_SEND_SERVER_COMMAND => {
            let number = call.int(1)?;
            if number != -1 && (number < 0 || number >= services.max_clients()) {
                return Ok(Some(0));
            }
            let text = memory.read_string(call.int(2)?)?;
            services.send_server_command(number, &text);
            Ok(Some(0))
        }
        G_GET_USERINFO => {
            if call.int(3)? < 1 {
                return Err(GuestError::runtime(format!(
                    "SV_GetUserinfo: bufferSize == {}",
                    call.int(3)?
                )));
            }
            check_client(call.int(1)?, services, "SV_GetUserinfo")?;
            let value = services.get_userinfo(call.int(1)?);
            memory.write_string(call.int(2)?, &value, call.int(3)? as usize)?;
            Ok(Some(0))
        }
        G_SET_USERINFO => {
            check_client(call.int(1)?, services, "SV_SetUserinfo")?;
            let value = if call.int(2)? == 0 {
                String::new()
            } else {
                memory.read_string(call.int(2)?)?
            };
            services.set_userinfo(call.int(1)?, &value);
            Ok(Some(0))
        }
        G_GET_USERCMD => {
            check_client(call.int(1)?, services, "SV_GetUsercmd")?;
            let command = services.get_user_command(call.int(1)?);
            let profile = services.abi_profile();
            memory.span(call.int(2)?, QVM_USER_COMMAND_BYTES, 0)?;
            write_user_command(memory, call.int(2)?, &command, profile, UserCommandWrite::Encode)?;
            Ok(Some(0))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeClients {
        profile: AbiProfile,
        infos: Vec<String>,
        log: Vec<String>,
    }

    impl FakeClients {
        fn new() -> Self {
            Self {
                profile: AbiProfile::Modern,
                infos: vec!["\\name\\a".to_string(), String::new()],
                log: Vec::new(),
            }
        }
    }

    impl ClientGameHost for FakeClients {
        fn abi_profile(&self) -> AbiProfile {
            self.profile
        }
        fn max_clients(&self) -> i32 {
            2
        }
        fn get_userinfo(&mut self, slot: i32) -> String {
            self.infos[slot as usize].clone()
        }
        fn set_userinfo(&mut self, slot: i32, value: &str) {
            self.log.push(format!("userinfo {slot}={value}"));
            self.infos[slot as usize] = value.to_string();
        }
        fn get_user_command(&mut self, slot: i32) -> WireUserCommand {
            WireUserCommand {
                server_time: 1000 + slot,
                buttons: 3,
                ..WireUserCommand::default()
            }
        }
        fn drop_client(&mut self, slot: i32, reason: &str) {
            self.log.push(format!("drop {slot} {reason}"));
        }
        fn send_server_command(&mut self, slot: i32, text: &str) {
            self.log.push(format!("cmd {slot} {text}"));
        }
    }

    fn call(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Qagame, code, args, AbiProfile::Modern)
    }

    #[test]
    fn drop_client_validates_slot() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(512, "bye", 4).unwrap();
        let mut services = FakeClients::new();
        assert_eq!(
            client_game_syscall(&call(G_DROP_CLIENT, &[1, 512]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(
            client_game_syscall(&call(G_DROP_CLIENT, &[9, 512]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(services.log, vec!["drop 1 bye".to_string()]);
    }

    #[test]
    fn server_command_supports_broadcast() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(512, "print hi", 9).unwrap();
        let mut services = FakeClients::new();
        assert_eq!(
            client_game_syscall(&call(G_SEND_SERVER_COMMAND, &[-1, 512]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(
            client_game_syscall(&call(G_SEND_SERVER_COMMAND, &[-2, 512]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(
            client_game_syscall(&call(G_SEND_SERVER_COMMAND, &[0, 512]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(services.log.len(), 2);
    }

    #[test]
    fn userinfo_round_trip() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(512, "\\name\\b", 8).unwrap();
        let mut services = FakeClients::new();
        assert_eq!(
            client_game_syscall(&call(G_GET_USERINFO, &[0, 256, 64]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_string(256).unwrap(), "\\name\\a");
        assert_eq!(
            client_game_syscall(&call(G_SET_USERINFO, &[1, 512]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(
            client_game_syscall(&call(G_SET_USERINFO, &[1, 0]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert!(client_game_syscall(&call(G_GET_USERINFO, &[5, 256, 64]), &mut memory, &mut services).is_err());
        assert!(client_game_syscall(&call(G_GET_USERINFO, &[0, 256, 0]), &mut memory, &mut services).is_err());
        assert!(client_game_syscall(&call(G_SET_USERINFO, &[5, 0]), &mut memory, &mut services).is_err());
    }

    #[test]
    fn get_usercmd_writes_record() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut services = FakeClients::new();
        assert_eq!(
            client_game_syscall(&call(G_GET_USERCMD, &[1, 256]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_i32(256).unwrap(), 1001);
        assert_eq!(memory.read_i32(256 + 16).unwrap(), 3);
        assert!(client_game_syscall(&call(G_GET_USERCMD, &[7, 256]), &mut memory, &mut services).is_err());
    }

    #[test]
    fn routing() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut services = FakeClients::new();
        let other = HostCall::engine(QvmRole::Cgame, G_GET_USERCMD, &[0, 256], AbiProfile::Modern);
        assert_eq!(client_game_syscall(&other, &mut memory, &mut services).unwrap(), None);
        assert_eq!(
            client_game_syscall(&call(999, &[]), &mut memory, &mut services).unwrap(),
            None
        );
    }
}
