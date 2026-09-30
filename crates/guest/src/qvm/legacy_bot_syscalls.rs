//! Legacy 1.16n/1.17 botlib boundary adaptations.
//!
//! Provenance: `src/compat/qvm/legacy-bot-syscalls.ts` (Q3 1.16n/1.17
//! `botlib.h` and `g_syscalls.c` boundary adaptations). Reuses
//! [`super::bot_library_syscalls`] hosts; [`BotActionFlags`] mirrors the
//! modern `BotActionFlag` values from `bots/behavior/library/actions.ts`.

use super::bot_library_syscalls::{BotLibraryHost, BotModuleServices, QVM_BOT_INPUT_BYTES};
use super::client_state::{AbiProfile, CallKind, HostCall, QvmRole, SyscallMemory, WireUserCommand};
use super::legacy_bot_abi::{
    BOTLIB_AI_ENTER_CHAT, BOTLIB_AI_LOAD_CHARACTER, BOTLIB_AI_SET_CHAT_NAME, BOTLIB_EA_GET_INPUT, BOTLIB_USER_COMMAND,
};
use crate::error::GuestError;

/// Modern action-flag bits consumed by the legacy remap.
pub struct BotActionFlags;

impl BotActionFlags {
    /// Attack bit.
    pub const ATTACK: i32 = 0x0000_0001;
    /// Use bit.
    pub const USE: i32 = 0x0000_0002;
    /// Respawn bit.
    pub const RESPAWN: i32 = 0x0000_0008;
    /// Jump bit.
    pub const JUMP: i32 = 0x0000_0010;
    /// Move-up bit.
    pub const MOVE_UP: i32 = 0x0000_0020;
    /// Crouch bit.
    pub const CROUCH: i32 = 0x0000_0080;
    /// Move-down bit.
    pub const MOVE_DOWN: i32 = 0x0000_0100;
    /// Move-forward bit.
    pub const MOVE_FORWARD: i32 = 0x0000_0200;
    /// Move-back bit.
    pub const MOVE_BACK: i32 = 0x0000_0800;
    /// Move-left bit.
    pub const MOVE_LEFT: i32 = 0x0000_1000;
    /// Move-right bit.
    pub const MOVE_RIGHT: i32 = 0x0000_2000;
    /// Delayed-jump bit.
    pub const DELAYED_JUMP: i32 = 0x0000_8000;
    /// Talk bit.
    pub const TALK: i32 = 0x0001_0000;
    /// Gesture bit.
    pub const GESTURE: i32 = 0x0002_0000;
    /// Walk bit.
    pub const WALK: i32 = 0x0008_0000;
}

/// Remap modern action flags to legacy bits.
///
/// Legacy jump/up and crouch/down intentionally share source bits.
pub fn legacy_bot_action_flags(flags: i32) -> i32 {
    let mut result = flags & 3;
    let fields: [(i32, i32); 11] = [
        (BotActionFlags::RESPAWN, 4),
        (BotActionFlags::JUMP | BotActionFlags::MOVE_UP, 8),
        (BotActionFlags::CROUCH | BotActionFlags::MOVE_DOWN, 16),
        (BotActionFlags::MOVE_FORWARD, 32),
        (BotActionFlags::MOVE_BACK, 64),
        (BotActionFlags::MOVE_LEFT, 128),
        (BotActionFlags::MOVE_RIGHT, 256),
        (BotActionFlags::DELAYED_JUMP, 512),
        (BotActionFlags::TALK, 1024),
        (BotActionFlags::GESTURE, 2048),
        (BotActionFlags::WALK, 4096),
    ];
    for (modern, legacy) in fields {
        if flags & modern != 0 {
            result |= legacy;
        }
    }
    result
}

/// Dispatch a legacy bot-library trap. Returns `Ok(None)` when unhandled.
///
/// Only `qagame` calls under the legacy profile reach this bridge; the
/// modern bridge runs first and this bridge handles the traps whose legacy
/// argument ABI differs.
pub fn legacy_bot_library_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    library: &mut dyn BotLibraryHost,
    services: &mut dyn BotModuleServices,
) -> Result<Option<i32>, GuestError> {
    if call.role != QvmRole::Qagame || call.abi_profile != AbiProfile::Legacy {
        return Ok(None);
    }
    if call.kind == CallKind::Extension {
        return match call.code {
            402 => {
                let name = memory.read_string(call.int(2)?)?;
                library.use_item(call.int(1)?, &name);
                Ok(Some(0))
            }
            403 => {
                let name = memory.read_string(call.int(2)?)?;
                library.drop_item(call.int(1)?, &name);
                Ok(Some(0))
            }
            404 => {
                let name = memory.read_string(call.int(2)?)?;
                library.use_inventory(call.int(1)?, &name);
                Ok(Some(0))
            }
            405 => {
                let name = memory.read_string(call.int(2)?)?;
                library.drop_inventory(call.int(1)?, &name);
                Ok(Some(0))
            }
            _ => Ok(None),
        };
    }
    match call.code {
        BOTLIB_AI_LOAD_CHARACTER => {
            let name = memory.read_string(call.int(1)?)?;
            Ok(Some(library.load_character(&name, call.int(2)? as f32)))
        }
        BOTLIB_AI_SET_CHAT_NAME => {
            let name = memory.read_string(call.int(2)?)?;
            library.set_chat_name(call.int(1)?, &name, None);
            Ok(Some(0))
        }
        BOTLIB_AI_ENTER_CHAT => {
            let client = call.int(2)?;
            library.enter_chat(call.int(1)?, client, i32::from(call.int(3)? == 1), Some(client));
            Ok(Some(0))
        }
        BOTLIB_EA_GET_INPUT => {
            let bytes = library.get_input_bytes(call.int(1)?, call.float(2)?);
            let range = memory.span(call.int(3)?, QVM_BOT_INPUT_BYTES, 0)?;
            memory.write_bytes(range.start, &bytes)?;
            let flags = memory.read_i32(range.start + 32)?;
            memory.write_i32(range.start + 32, legacy_bot_action_flags(flags))?;
            Ok(Some(0))
        }
        BOTLIB_USER_COMMAND => {
            let range = memory.span(call.int(2)?, 24, 0)?;
            let buttons = memory.get(range.start + 4)?;
            let command = WireUserCommand {
                server_time: memory.read_i32(range.start)?,
                angles: [
                    memory.read_i32(range.start + 8)?,
                    memory.read_i32(range.start + 12)?,
                    memory.read_i32(range.start + 16)?,
                ],
                buttons: i32::from(buttons & 31) | (if buttons & 128 == 0 { 0 } else { 2048 }),
                weapon: memory.get(range.start + 5)?,
                forwardmove: memory.read_i8(range.start + 20)?,
                rightmove: memory.read_i8(range.start + 21)?,
                upmove: memory.read_i8(range.start + 22)?,
            };
            services.user_command(call.int(1)?, command);
            Ok(Some(0))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::super::legacy_bot_abi::BOTLIB_EA_SAY;
    use super::*;

    struct FakeLibrary {
        log: Vec<String>,
    }

    macro_rules! stub_library {
        () => {
            fn set_libvar(&mut self, _name: &str, _value: &str) {}
            fn get_libvar(&mut self, _name: &str) -> String {
                String::new()
            }
            fn add_global_define(&mut self, _text: &str) -> bool {
                false
            }
            fn start_frame(&mut self, _time: f32) -> i32 {
                0
            }
            fn say(&mut self, _client: i32, _text: &str) {}
            fn say_team(&mut self, _client: i32, _text: &str) {}
            fn action_command(&mut self, _client: i32, _text: &str) {}
            fn action(&mut self, _client: i32, _code: i32) {}
            fn gesture(&mut self, _client: i32) {}
            fn talk(&mut self, _client: i32) {}
            fn attack(&mut self, _client: i32) {}
            fn use_action(&mut self, _client: i32) {}
            fn respawn(&mut self, _client: i32) {}
            fn crouch(&mut self, _client: i32) {}
            fn move_up(&mut self, _client: i32) {}
            fn move_down(&mut self, _client: i32) {}
            fn move_forward(&mut self, _client: i32) {}
            fn move_back(&mut self, _client: i32) {}
            fn move_left(&mut self, _client: i32) {}
            fn move_right(&mut self, _client: i32) {}
            fn select_weapon(&mut self, _client: i32, _weapon: i32) {}
            fn jump(&mut self, _client: i32) {}
            fn delayed_jump(&mut self, _client: i32) {}
            fn directed_move(&mut self, _client: i32, _direction: qa_core::math::Vec3, _speed: f32) {}
            fn view(&mut self, _client: i32, _angles: qa_core::math::Vec3) {}
            fn end_regular(&mut self, _client: i32, _think_time: f32) {}
            fn reset_input(&mut self, _client: i32) {}
            fn free_character(&mut self, _handle: i32) {}
            fn characteristic_float(&mut self, _handle: i32, _index: i32) -> f32 {
                0.0
            }
            fn characteristic_bounded_float(&mut self, _handle: i32, _index: i32, _min: f32, _max: f32) -> f32 {
                0.0
            }
            fn characteristic_integer(&mut self, _handle: i32, _index: i32) -> i32 {
                0
            }
            fn characteristic_bounded_integer(&mut self, _handle: i32, _index: i32, _min: i32, _max: i32) -> i32 {
                0
            }
            fn characteristic_string(&mut self, _handle: i32, _index: i32) -> String {
                String::new()
            }
            fn alloc_chat_state(&mut self) -> i32 {
                0
            }
            fn free_chat_state(&mut self, _handle: i32) {}
            fn queue_console_message(&mut self, _handle: i32, _kind: i32, _text: &str) {}
            fn remove_console_message(&mut self, _handle: i32, _kind: i32) {}
            fn num_console_messages(&mut self, _handle: i32) -> i32 {
                0
            }
            fn next_console_message(
                &mut self,
                _handle: i32,
            ) -> Option<super::super::bot_library_syscalls::BotConsoleMessage> {
                None
            }
            fn initial_chat(
                &mut self,
                _handle: i32,
                _text: Option<String>,
                _length: i32,
                _variables: [Option<String>; 8],
            ) {
            }
            fn reply_chat(
                &mut self,
                _handle: i32,
                _text: &str,
                _a: i32,
                _b: i32,
                _variables: [Option<String>; 8],
            ) -> bool {
                false
            }
            fn num_initial_chats(&mut self, _handle: i32, _text: Option<String>) -> i32 {
                0
            }
            fn chat_length(&mut self, _handle: i32) -> i32 {
                0
            }
            fn chat_message(&mut self, _handle: i32) -> String {
                String::new()
            }
            fn replace_synonyms(&mut self, _memory: &mut SyscallMemory, _word: i32, _context: i32) {}
            fn load_chat_file(&mut self, _handle: i32, _a: &str, _b: &str) -> bool {
                false
            }
            fn set_chat_gender(&mut self, _handle: i32, _gender: i32) {}
            fn find_match(
                &mut self,
                _text: &str,
                _handle: i32,
                _buffer: &mut super::super::bot_library_syscalls::ChatMatchBuffer,
            ) -> bool {
                false
            }
            fn match_variable(
                &mut self,
                _variables: &[super::super::bot_library_syscalls::ChatMatchVariable; 8],
                _a: i32,
                _b: i32,
            ) -> super::super::bot_library_syscalls::MatchVariableWrite {
                super::super::bot_library_syscalls::MatchVariableWrite::Clear
            }
            fn alloc_goal_state(&mut self, _client: i32) -> i32 {
                0
            }
            fn free_goal_state(&mut self, _handle: i32) {}
            fn reset_goal_state(&mut self, _handle: i32) {}
            fn reset_avoid_goals(&mut self, _handle: i32) {}
            fn push_goal(&mut self, _handle: i32, _goal: super::super::bot_navigation_records::BotGoal) {}
            fn pop_goal(&mut self, _handle: i32) {}
            fn empty_goal_stack(&mut self, _handle: i32) {}
            fn top_goal(&mut self, _handle: i32) -> Option<super::super::bot_navigation_records::BotGoal> {
                None
            }
            fn second_goal(&mut self, _handle: i32) -> Option<super::super::bot_navigation_records::BotGoal> {
                None
            }
            fn goal_name(&mut self, _handle: i32) -> String {
                String::new()
            }
            fn avoid_goal_time(&mut self, _handle: i32, _number: i32) -> f32 {
                0.0
            }
            fn set_avoid_goal_time(&mut self, _handle: i32, _number: i32, _time: f32) {}
            fn remove_from_avoid_goals(&mut self, _handle: i32, _number: i32) {}
            fn load_item_weights(&mut self, _handle: i32, _name: &str) -> i32 {
                0
            }
            fn free_item_weights(&mut self, _handle: i32) {}
            fn choose_ltg_item(
                &mut self,
                _handle: i32,
                _origin: qa_core::math::Vec3,
                _inventory: &dyn Fn(i32) -> Result<i32, GuestError>,
                _travel_flags: i32,
            ) -> bool {
                false
            }
            fn choose_nbg_item(
                &mut self,
                _handle: i32,
                _origin: qa_core::math::Vec3,
                _inventory: &dyn Fn(i32) -> Result<i32, GuestError>,
                _travel_flags: i32,
                _goal: Option<super::super::bot_navigation_records::BotGoal>,
                _range: f32,
            ) -> bool {
                false
            }
            fn item_goal_in_vis_but_not_visible(
                &mut self,
                _handle: i32,
                _view: qa_core::math::Vec3,
                _origin: qa_core::math::Vec3,
                _goal: &super::super::bot_navigation_records::BotGoal,
            ) -> bool {
                false
            }
            fn init_level_items(&mut self) {}
            fn update_entity_items(&mut self) {}
            fn level_item_goal(
                &mut self,
                _index: i32,
                _name: &str,
                _goal: &super::super::bot_navigation_records::BotGoal,
            ) -> Option<super::super::bot_navigation_records::BotGoal> {
                None
            }
            fn map_location_goal(
                &mut self,
                _name: &str,
                _goal: &super::super::bot_navigation_records::BotGoal,
            ) -> Option<super::super::bot_navigation_records::BotGoal> {
                None
            }
            fn next_camp_spot_goal(
                &mut self,
                _handle: i32,
                _goal: &super::super::bot_navigation_records::BotGoal,
            ) -> Option<super::super::bot_library_syscalls::CampSpotGoal> {
                None
            }
            fn dump_avoid_goals(&mut self, _handle: i32) {}
            fn dump_goal_stack(&mut self, _handle: i32) {}
            fn interbreed_goal_fuzzy_logic(&mut self, _a: i32, _b: i32, _c: i32) {}
            fn mutate_goal_fuzzy_logic(&mut self, _handle: i32, _range: f32) {}
            fn save_goal_fuzzy_logic(&mut self, _handle: i32, _name: &str) {}
            fn alloc_weapon_state(&mut self) -> i32 {
                0
            }
            fn free_weapon_state(&mut self, _handle: i32) {}
            fn reset_weapon_state(&mut self, _handle: i32) {}
            fn load_weapon_weights(&mut self, _handle: i32, _name: &str) -> i32 {
                0
            }
            fn choose_best_fight_weapon(
                &mut self,
                _handle: i32,
                _inventory: &dyn Fn(i32) -> Result<i32, GuestError>,
            ) -> i32 {
                0
            }
            fn weapon_info(&mut self, _handle: i32, _weapon: i32) -> Option<Vec<u8>> {
                None
            }
            fn genetic_selection(
                &mut self,
                _count: i32,
                _ranks: &dyn Fn(i32) -> Result<f32, GuestError>,
            ) -> Result<Option<(i32, i32, i32)>, GuestError> {
                Ok(None)
            }
            fn load_source(&mut self, _name: &str) -> i32 {
                0
            }
            fn free_source(&mut self, _handle: i32) -> bool {
                false
            }
            fn read_source_token(&mut self, _handle: i32) -> Option<super::super::client_script_syscalls::TokenRead> {
                None
            }
            fn source_file_and_line(&mut self, _handle: i32) -> Option<(String, i32)> {
                None
            }
        };
    }

    impl BotLibraryHost for FakeLibrary {
        stub_library!();

        fn get_input_bytes(&mut self, _client: i32, _think_time: f32) -> [u8; QVM_BOT_INPUT_BYTES] {
            let mut bytes = [0u8; QVM_BOT_INPUT_BYTES];
            let flags = BotActionFlags::JUMP | BotActionFlags::MOVE_FORWARD | BotActionFlags::ATTACK;
            bytes[32..36].copy_from_slice(&flags.to_le_bytes());
            bytes
        }
        fn use_item(&mut self, client: i32, name: &str) {
            self.log.push(format!("useitem {client} {name}"));
        }
        fn drop_item(&mut self, client: i32, name: &str) {
            self.log.push(format!("dropitem {client} {name}"));
        }
        fn use_inventory(&mut self, client: i32, name: &str) {
            self.log.push(format!("useinv {client} {name}"));
        }
        fn drop_inventory(&mut self, client: i32, name: &str) {
            self.log.push(format!("dropinv {client} {name}"));
        }
        fn load_character(&mut self, name: &str, skill: f32) -> i32 {
            self.log.push(format!("char {name} {skill}"));
            9
        }
        fn set_chat_name(&mut self, handle: i32, name: &str, extra: Option<i32>) {
            self.log.push(format!("chatname {handle} {name} {extra:?}"));
        }
        fn enter_chat(&mut self, handle: i32, a: i32, b: i32, legacy_extra: Option<i32>) {
            self.log.push(format!("enter {handle} {a} {b} {legacy_extra:?}"));
        }
    }

    struct FakeServices {
        log: Vec<String>,
    }

    impl BotModuleServices for FakeServices {
        fn setup(&mut self) -> i32 {
            0
        }
        fn shutdown(&mut self) -> i32 {
            0
        }
        fn load_map(&mut self, _name: &str) -> i32 {
            0
        }
        fn update_entity(&mut self, _memory: &mut SyscallMemory, _number: i32, _word: i32) -> i32 {
            0
        }
        fn snapshot_entity(&mut self, _client: i32, _sequence: i32) -> i32 {
            0
        }
        fn console_message(&mut self, _client: i32) -> Option<String> {
            None
        }
        fn user_command(&mut self, client: i32, command: WireUserCommand) {
            self.log
                .push(format!("usercmd {client} {} {}", command.server_time, command.buttons));
        }
        fn allocate_client(&mut self) -> i32 {
            0
        }
        fn free_client(&mut self, _client: i32) {}
    }

    fn legacy(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Qagame, code, args, AbiProfile::Legacy)
    }

    #[test]
    fn action_flag_remap() {
        assert_eq!(legacy_bot_action_flags(0), 0);
        assert_eq!(legacy_bot_action_flags(BotActionFlags::ATTACK | BotActionFlags::USE), 3);
        assert_eq!(
            legacy_bot_action_flags(BotActionFlags::JUMP | BotActionFlags::MOVE_UP),
            8
        );
        assert_eq!(
            legacy_bot_action_flags(BotActionFlags::CROUCH | BotActionFlags::MOVE_DOWN),
            16
        );
        assert_eq!(
            legacy_bot_action_flags(
                BotActionFlags::RESPAWN
                    | BotActionFlags::MOVE_FORWARD
                    | BotActionFlags::MOVE_BACK
                    | BotActionFlags::MOVE_LEFT
                    | BotActionFlags::MOVE_RIGHT
                    | BotActionFlags::DELAYED_JUMP
                    | BotActionFlags::TALK
                    | BotActionFlags::GESTURE
                    | BotActionFlags::WALK
            ),
            4 | 32 | 64 | 128 | 256 | 512 | 1024 | 2048 | 4096
        );
    }

    #[test]
    fn extension_item_traps() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(256, "quad", 5).unwrap();
        let mut library = FakeLibrary { log: Vec::new() };
        let mut services = FakeServices { log: Vec::new() };
        for (code, name) in [(402, "useitem"), (403, "dropitem"), (404, "useinv"), (405, "dropinv")] {
            let call = HostCall::extension(QvmRole::Qagame, code, &[1, 256], AbiProfile::Legacy);
            assert_eq!(
                legacy_bot_library_syscall(&call, &mut memory, &mut library, &mut services).unwrap(),
                Some(0),
                "trap {code}"
            );
            assert_eq!(library.log.last().unwrap(), &format!("{name} 1 quad"));
        }
        let call = HostCall::extension(QvmRole::Qagame, 406, &[1, 256], AbiProfile::Legacy);
        assert_eq!(
            legacy_bot_library_syscall(&call, &mut memory, &mut library, &mut services).unwrap(),
            None
        );
    }

    #[test]
    fn engine_adaptations() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(256, "sarge", 6).unwrap();
        let mut library = FakeLibrary { log: Vec::new() };
        let mut services = FakeServices { log: Vec::new() };
        assert_eq!(
            legacy_bot_library_syscall(
                &legacy(BOTLIB_AI_LOAD_CHARACTER, &[256, 4]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(9)
        );
        assert_eq!(
            legacy_bot_library_syscall(
                &legacy(BOTLIB_AI_SET_CHAT_NAME, &[3, 256]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            legacy_bot_library_syscall(
                &legacy(BOTLIB_AI_ENTER_CHAT, &[3, 5, 1]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            library.log,
            vec![
                "char sarge 4".to_string(),
                "chatname 3 sarge None".to_string(),
                "enter 3 5 1 Some(5)".to_string(),
            ]
        );
    }

    #[test]
    fn get_input_remaps_flags() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut library = FakeLibrary { log: Vec::new() };
        let mut services = FakeServices { log: Vec::new() };
        let think = 0.1f32.to_bits() as i32;
        assert_eq!(
            legacy_bot_library_syscall(
                &legacy(BOTLIB_EA_GET_INPUT, &[1, think, 1024]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        let expected =
            legacy_bot_action_flags(BotActionFlags::JUMP | BotActionFlags::MOVE_FORWARD | BotActionFlags::ATTACK);
        assert_eq!(memory.read_i32(1056).unwrap(), expected);
    }

    #[test]
    fn legacy_user_command_layout() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_i32(1024, 500).unwrap();
        memory.set(1028, 31 | 128).unwrap();
        memory.set(1029, 4).unwrap();
        memory.write_i32(1032, 10).unwrap();
        memory.write_i32(1036, 20).unwrap();
        memory.write_i32(1040, 30).unwrap();
        memory.write_i8(1044, -5).unwrap();
        memory.write_i8(1045, 6).unwrap();
        memory.write_i8(1046, 7).unwrap();
        let mut library = FakeLibrary { log: Vec::new() };
        let mut services = FakeServices { log: Vec::new() };
        assert_eq!(
            legacy_bot_library_syscall(
                &legacy(BOTLIB_USER_COMMAND, &[2, 1024]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(services.log, vec![format!("usercmd 2 500 {}", 31 | 2048)]);
    }

    #[test]
    fn routing() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut library = FakeLibrary { log: Vec::new() };
        let mut services = FakeServices { log: Vec::new() };
        let modern = HostCall::engine(QvmRole::Qagame, BOTLIB_AI_LOAD_CHARACTER, &[256, 4], AbiProfile::Modern);
        assert_eq!(
            legacy_bot_library_syscall(&modern, &mut memory, &mut library, &mut services).unwrap(),
            None
        );
        let other = HostCall::engine(QvmRole::Cgame, BOTLIB_AI_LOAD_CHARACTER, &[256, 4], AbiProfile::Legacy);
        assert_eq!(
            legacy_bot_library_syscall(&other, &mut memory, &mut library, &mut services).unwrap(),
            None
        );
        assert_eq!(
            legacy_bot_library_syscall(
                &legacy(BOTLIB_EA_SAY, &[1, 256]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            None
        );
    }
}
