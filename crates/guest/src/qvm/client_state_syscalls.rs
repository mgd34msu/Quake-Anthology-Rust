//! Client-state traps: game state, snapshots, server commands, user commands.
//!
//! Provenance: `src/compat/qvm/client-state-syscalls.ts` (Q3 `cl_cgame.c`
//! client-state traps). Donor promises become direct returns; [`ClientStateHost`]
//! bundles the connection, snapshot source, and command owners the donor
//! receives separately.

use super::client_state::{AbiProfile, CallKind, GameStateRecord, HostCall, QvmRole, SyscallMemory, WireUserCommand};
use super::client_state_record::{
    QVM_GAME_STATE_BYTES, QVM_USER_COMMAND_BYTES, SourceSnapshot, UserCommandWrite, qvm_snapshot_bytes,
    write_game_state, write_snapshot, write_user_command,
};
use super::legacy_bot_abi::{
    CG_GETCURRENTCMDNUMBER, CG_GETCURRENTSNAPSHOTNUMBER, CG_GETGAMESTATE, CG_GETSERVERCOMMAND, CG_GETSNAPSHOT,
    CG_GETUSERCMD, CG_SETUSERCMDVALUE, UI_GETCONFIGSTRING,
};
use super::legacy_presentation::qvm_configstring;
use crate::error::GuestError;

/// Host client-state surface used by the traps.
pub trait ClientStateHost {
    /// Configstring value at a canonical index, if present.
    fn game_state_get(&mut self, index: usize) -> Option<String>;
    /// Detached copy of the full game-state record.
    fn game_state_record(&mut self) -> GameStateRecord;
    /// Current snapshot number and server time.
    fn snapshot_current(&mut self) -> (i32, i32);
    /// Retained snapshot by number, if any.
    fn snapshot_read(&mut self, number: i32) -> Option<SourceSnapshot>;
    /// Ping recorded for a retained snapshot, if any.
    fn snapshot_ping(&mut self, number: i32) -> Option<i32>;
    /// Server-command argv by number; the host installs argv before resolving.
    fn get_server_command(&mut self, number: i32) -> Option<Vec<String>>;
    /// Current outgoing user-command number.
    fn commands_current_number(&mut self) -> i32;
    /// Retained user command by number, if any.
    fn commands_read(&mut self, number: i32) -> Option<WireUserCommand>;
    /// Record the weapon/sensitivity user-command value.
    fn set_user_command_value(&mut self, weapon: i32, sensitivity: f32);
}

/// Dispatch a client-state trap. Returns `Ok(None)` when unhandled.
pub fn client_state_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    services: &mut dyn ClientStateHost,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine {
        return Ok(None);
    }
    let profile = call.abi_profile;
    if call.role == QvmRole::Ui && call.code == UI_GETCONFIGSTRING {
        let index = call.int(1)?;
        let word = call.int(2)?;
        let size = call.int(3)?;
        if index < 0 || index >= 1024 {
            return Ok(Some(0));
        }
        let value = services.game_state_get(qvm_configstring(index, profile)? as usize);
        match value {
            None => {
                if size != 0 {
                    let range = memory.span(word, 1, 0)?;
                    memory.set(range.start, 0)?;
                }
                Ok(Some(0))
            }
            Some(text) => {
                memory.write_string(word, &text, size as usize)?;
                Ok(Some(1))
            }
        }
    } else if call.role != QvmRole::Cgame {
        Ok(None)
    } else {
        match call.code {
            CG_GETGAMESTATE => {
                let record = services.game_state_record();
                write_game_state(memory, call.int(1)?, &record, profile)?;
                Ok(Some(0))
            }
            CG_GETCURRENTSNAPSHOTNUMBER => {
                let (number, time) = services.snapshot_current();
                memory.span(call.int(1)?, 4, 0)?;
                memory.span(call.int(2)?, 4, 0)?;
                let base = memory.pointer(call.int(1)?).expect("checked span");
                memory.write_i32(base, number)?;
                let base = memory.pointer(call.int(2)?).expect("checked span");
                memory.write_i32(base, time)?;
                Ok(Some(0))
            }
            CG_GETSNAPSHOT => {
                let number = call.int(1)?;
                let word = call.int(2)?;
                let snapshot = match services.snapshot_read(number) {
                    Some(snapshot) => snapshot,
                    None => return Ok(Some(0)),
                };
                let ping = services
                    .snapshot_ping(number)
                    .ok_or_else(|| GuestError::runtime("Retained snapshot has no source ping"))?;
                memory.span(word, qvm_snapshot_bytes(profile), 0)?;
                write_snapshot(memory, word, &snapshot, ping, profile)?;
                Ok(Some(1))
            }
            CG_GETSERVERCOMMAND => Ok(Some(i32::from(services.get_server_command(call.int(1)?).is_some()))),
            CG_GETCURRENTCMDNUMBER => Ok(Some(services.commands_current_number())),
            CG_GETUSERCMD => {
                let number = call.int(1)?;
                let word = call.int(2)?;
                let command = match services.commands_read(number) {
                    Some(command) => command,
                    None => return Ok(Some(0)),
                };
                memory.span(word, QVM_USER_COMMAND_BYTES, 0)?;
                write_user_command(memory, word, &command, profile, UserCommandWrite::Encode)?;
                Ok(Some(1))
            }
            CG_SETUSERCMDVALUE => {
                let weapon = call.int(1)?;
                let sensitivity = call.float(2)?;
                services.set_user_command_value(weapon, sensitivity);
                Ok(Some(0))
            }
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::client_state_record::PlayerStateFields;
    use super::*;
    use qa_core::math::Vec3;

    struct FakeState {
        log: Vec<String>,
    }

    fn player() -> PlayerStateFields {
        PlayerStateFields {
            command_time_ms: 0,
            movement_type: 0,
            bob_cycle: 0,
            movement_flags: 0,
            movement_time_ms: 0,
            origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            weapon_time_ms: 0,
            gravity: 800,
            speed: 0,
            delta_angles: [0, 0, 0],
            ground_entity_number: 0,
            legs_timer_ms: 0,
            legs_animation: 0,
            torso_timer_ms: 0,
            torso_animation: 0,
            movement_direction: 0,
            grapple_point: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            flags: 0,
            event_sequence: 0,
            events: [0, 0],
            event_parameters: [0, 0],
            external_event: 0,
            external_event_parameter: 0,
            external_event_time_ms: 0,
            client_number: 0,
            weapon: 0,
            weapon_state: 0,
            view_angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            view_height: 0,
            damage_event: 0,
            damage_yaw: 0,
            damage_pitch: 0,
            damage_count: 0,
            stats: [0; 16],
            persistent: [0; 16],
            powerups: [0; 16],
            ammo: [0; 16],
            generic1: 0,
            loop_sound: 0,
            jump_pad_entity: 0,
            ping_ms: 0,
            movement_frame_count: 0,
            jump_pad_frame: 0,
            entity_event_sequence: 0,
        }
    }

    impl ClientStateHost for FakeState {
        fn game_state_get(&mut self, index: usize) -> Option<String> {
            (index == 3).then(|| "\\map\\q3dm1".to_string())
        }
        fn game_state_record(&mut self) -> GameStateRecord {
            GameStateRecord { string_offsets: vec![8; 1024], string_data: vec![1u8; 16000], data_count: 9 }
        }
        fn snapshot_current(&mut self) -> (i32, i32) {
            (41, 4242)
        }
        fn snapshot_read(&mut self, number: i32) -> Option<SourceSnapshot> {
            (number == 41).then(|| SourceSnapshot {
                number: 41,
                server_time: 4242,
                flags: 5,
                area_mask: [0u8; 32],
                player_state: player(),
                entities: Vec::new(),
                server_command_sequence: 12,
            })
        }
        fn snapshot_ping(&mut self, number: i32) -> Option<i32> {
            (number == 41).then_some(37)
        }
        fn get_server_command(&mut self, number: i32) -> Option<Vec<String>> {
            (number == 12).then(|| vec!["print".to_string(), "hi".to_string()])
        }
        fn commands_current_number(&mut self) -> i32 {
            77
        }
        fn commands_read(&mut self, number: i32) -> Option<WireUserCommand> {
            (number == 77).then(|| WireUserCommand { server_time: 555, ..WireUserCommand::default() })
        }
        fn set_user_command_value(&mut self, weapon: i32, sensitivity: f32) {
            self.log.push(format!("usercmd {weapon} {sensitivity}"));
        }
    }

    fn cg(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Cgame, code, args, AbiProfile::Modern)
    }

    #[test]
    fn ui_getconfigstring() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        let mut services = FakeState { log: Vec::new() };
        let found = HostCall::engine(QvmRole::Ui, UI_GETCONFIGSTRING, &[3, 256, 64], AbiProfile::Modern);
        assert_eq!(client_state_syscall(&found, &mut memory, &mut services).unwrap(), Some(1));
        assert_eq!(memory.read_string(256).unwrap(), "\\map\\q3dm1");
        let missing = HostCall::engine(QvmRole::Ui, UI_GETCONFIGSTRING, &[4, 256, 64], AbiProfile::Modern);
        assert_eq!(client_state_syscall(&missing, &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(memory.get(256).unwrap(), 0);
        let bad = HostCall::engine(QvmRole::Ui, UI_GETCONFIGSTRING, &[2048, 256, 64], AbiProfile::Modern);
        assert_eq!(client_state_syscall(&bad, &mut memory, &mut services).unwrap(), Some(0));
    }

    #[test]
    fn get_game_state_and_snapshot_number() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        let mut services = FakeState { log: Vec::new() };
        assert_eq!(client_state_syscall(&cg(CG_GETGAMESTATE, &[8192]), &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(memory.read_i32(8192).unwrap(), 8);
        assert_eq!(memory.read_i32(8192 + 20096).unwrap(), 9);
        assert_eq!(client_state_syscall(&cg(CG_GETCURRENTSNAPSHOTNUMBER, &[256, 260]), &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(memory.read_i32(256).unwrap(), 41);
        assert_eq!(memory.read_i32(260).unwrap(), 4242);
    }

    #[test]
    fn get_snapshot_found_and_missing() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        let mut services = FakeState { log: Vec::new() };
        assert_eq!(client_state_syscall(&cg(CG_GETSNAPSHOT, &[41, 1024]), &mut memory, &mut services).unwrap(), Some(1));
        assert_eq!(memory.read_i32(1024).unwrap(), 5);
        assert_eq!(memory.read_i32(1024 + 4).unwrap(), 37);
        assert_eq!(client_state_syscall(&cg(CG_GETSNAPSHOT, &[40, 1024]), &mut memory, &mut services).unwrap(), Some(0));
    }

    #[test]
    fn server_command_and_user_commands() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        let mut services = FakeState { log: Vec::new() };
        assert_eq!(client_state_syscall(&cg(CG_GETSERVERCOMMAND, &[12]), &mut memory, &mut services).unwrap(), Some(1));
        assert_eq!(client_state_syscall(&cg(CG_GETSERVERCOMMAND, &[11]), &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(client_state_syscall(&cg(CG_GETCURRENTCMDNUMBER, &[]), &mut memory, &mut services).unwrap(), Some(77));
        assert_eq!(client_state_syscall(&cg(CG_GETUSERCMD, &[77, 256]), &mut memory, &mut services).unwrap(), Some(1));
        assert_eq!(memory.read_i32(256).unwrap(), 555);
        assert_eq!(client_state_syscall(&cg(CG_GETUSERCMD, &[76, 256]), &mut memory, &mut services).unwrap(), Some(0));
        assert_eq!(
            client_state_syscall(&cg(CG_SETUSERCMDVALUE, &[3, 2.5f32.to_bits() as i32]), &mut memory, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(services.log, vec!["usercmd 3 2.5".to_string()]);
    }

    #[test]
    fn routing() {
        let mut memory = SyscallMemory::new(65536).unwrap();
        let mut services = FakeState { log: Vec::new() };
        let game = HostCall::engine(QvmRole::Qagame, CG_GETSNAPSHOT, &[41, 1024], AbiProfile::Modern);
        assert_eq!(client_state_syscall(&game, &mut memory, &mut services).unwrap(), None);
        assert_eq!(client_state_syscall(&cg(999, &[]), &mut memory, &mut services).unwrap(), None);
    }
}
