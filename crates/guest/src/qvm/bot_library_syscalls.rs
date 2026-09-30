//! Bot-library trap bridge for game modules.
//!
//! Provenance: `src/compat/qvm/bot-library-syscalls.ts`. [`BotLibraryHost`]
//! and [`BotModuleServices`] are local mirrors of the `BotLibrary` and
//! module-lifetime surfaces the donor consumes; [`touching_goal`],
//! [`string_contains`], and [`unify_white_spaces`] are local mirrors of the
//! pure `bots/behavior/library/{goals,chat}.ts` helpers. The chat match
//! buffer, console-message record, and script-token writes reuse the donor's
//! exact layouts. Donor promises become direct returns.

use qa_core::math::Vec3;

use super::bot_navigation_records::{read_bot_goal, write_bot_goal, BotGoal, GoalWriteFields, QVM_BOT_GOAL_BYTES};
use super::client_script_syscalls::{write_script_token, TokenRead};
use super::client_state::{CallKind, HostCall, QvmRole, SyscallMemory, WireUserCommand};
use super::client_state_record::read_user_command;
use super::legacy_bot_abi::{
    BOTLIB_AI_ALLOC_CHAT_STATE, BOTLIB_AI_ALLOC_GOAL_STATE, BOTLIB_AI_ALLOC_WEAPON_STATE, BOTLIB_AI_AVOID_GOAL_TIME,
    BOTLIB_AI_CHARACTERISTIC_BFLOAT, BOTLIB_AI_CHARACTERISTIC_BINTEGER, BOTLIB_AI_CHARACTERISTIC_FLOAT,
    BOTLIB_AI_CHARACTERISTIC_INTEGER, BOTLIB_AI_CHARACTERISTIC_STRING, BOTLIB_AI_CHAT_LENGTH,
    BOTLIB_AI_CHOOSE_BEST_FIGHT_WEAPON, BOTLIB_AI_CHOOSE_LTG_ITEM, BOTLIB_AI_CHOOSE_NBG_ITEM,
    BOTLIB_AI_DUMP_AVOID_GOALS, BOTLIB_AI_DUMP_GOAL_STACK, BOTLIB_AI_EMPTY_GOAL_STACK, BOTLIB_AI_ENTER_CHAT,
    BOTLIB_AI_FIND_MATCH, BOTLIB_AI_FREE_CHARACTER, BOTLIB_AI_FREE_CHAT_STATE, BOTLIB_AI_FREE_GOAL_STATE,
    BOTLIB_AI_FREE_ITEM_WEIGHTS, BOTLIB_AI_FREE_WEAPON_STATE, BOTLIB_AI_GENETIC_PARENTS_AND_CHILD_SELECTION,
    BOTLIB_AI_GET_CHAT_MESSAGE, BOTLIB_AI_GET_LEVEL_ITEM_GOAL, BOTLIB_AI_GET_MAP_LOCATION_GOAL,
    BOTLIB_AI_GET_NEXT_CAMP_SPOT_GOAL, BOTLIB_AI_GET_SECOND_GOAL, BOTLIB_AI_GET_TOP_GOAL, BOTLIB_AI_GET_WEAPON_INFO,
    BOTLIB_AI_GOAL_NAME, BOTLIB_AI_INITIAL_CHAT, BOTLIB_AI_INIT_LEVEL_ITEMS, BOTLIB_AI_INTERBREED_GOAL_FUZZY_LOGIC,
    BOTLIB_AI_ITEM_GOAL_IN_VIS_BUT_NOT_VISIBLE, BOTLIB_AI_LOAD_CHARACTER, BOTLIB_AI_LOAD_CHAT_FILE,
    BOTLIB_AI_LOAD_ITEM_WEIGHTS, BOTLIB_AI_LOAD_WEAPON_WEIGHTS, BOTLIB_AI_MATCH_VARIABLE,
    BOTLIB_AI_MUTATE_GOAL_FUZZY_LOGIC, BOTLIB_AI_NEXT_CONSOLE_MESSAGE, BOTLIB_AI_NUM_CONSOLE_MESSAGE,
    BOTLIB_AI_NUM_INITIAL_CHATS, BOTLIB_AI_POP_GOAL, BOTLIB_AI_PUSH_GOAL, BOTLIB_AI_QUEUE_CONSOLE_MESSAGE,
    BOTLIB_AI_REMOVE_CONSOLE_MESSAGE, BOTLIB_AI_REMOVE_FROM_AVOID_GOALS, BOTLIB_AI_REPLACE_SYNONYMS,
    BOTLIB_AI_REPLY_CHAT, BOTLIB_AI_RESET_AVOID_GOALS, BOTLIB_AI_RESET_GOAL_STATE, BOTLIB_AI_RESET_WEAPON_STATE,
    BOTLIB_AI_SAVE_GOAL_FUZZY_LOGIC, BOTLIB_AI_SET_AVOID_GOAL_TIME, BOTLIB_AI_SET_CHAT_GENDER, BOTLIB_AI_SET_CHAT_NAME,
    BOTLIB_AI_STRING_CONTAINS, BOTLIB_AI_TOUCHING_GOAL, BOTLIB_AI_UNIFY_WHITE_SPACES, BOTLIB_AI_UPDATE_ENTITY_ITEMS,
    BOTLIB_EA_ACTION, BOTLIB_EA_ATTACK, BOTLIB_EA_COMMAND, BOTLIB_EA_CROUCH, BOTLIB_EA_DELAYED_JUMP,
    BOTLIB_EA_END_REGULAR, BOTLIB_EA_GESTURE, BOTLIB_EA_GET_INPUT, BOTLIB_EA_JUMP, BOTLIB_EA_MOVE, BOTLIB_EA_MOVE_BACK,
    BOTLIB_EA_MOVE_DOWN, BOTLIB_EA_MOVE_FORWARD, BOTLIB_EA_MOVE_LEFT, BOTLIB_EA_MOVE_RIGHT, BOTLIB_EA_MOVE_UP,
    BOTLIB_EA_RESET_INPUT, BOTLIB_EA_RESPAWN, BOTLIB_EA_SAY, BOTLIB_EA_SAY_TEAM, BOTLIB_EA_SELECT_WEAPON,
    BOTLIB_EA_TALK, BOTLIB_EA_USE, BOTLIB_EA_VIEW, BOTLIB_GET_CONSOLE_MESSAGE, BOTLIB_GET_SNAPSHOT_ENTITY,
    BOTLIB_LIBVAR_GET, BOTLIB_LIBVAR_SET, BOTLIB_LOAD_MAP, BOTLIB_PC_ADD_GLOBAL_DEFINE, BOTLIB_PC_FREE_SOURCE,
    BOTLIB_PC_LOAD_SOURCE, BOTLIB_PC_READ_TOKEN, BOTLIB_PC_SOURCE_FILE_AND_LINE, BOTLIB_SETUP, BOTLIB_SHUTDOWN,
    BOTLIB_START_FRAME, BOTLIB_UPDATENTITY, BOTLIB_USER_COMMAND, G_BOT_ALLOCATE_CLIENT, G_BOT_FREE_CLIENT,
};
use crate::error::GuestError;

/// Byte length of the console-message record.
pub const QVM_BOT_CONSOLE_MESSAGE_BYTES: usize = 276;
/// Byte length of the chat-match record.
pub const QVM_BOT_MATCH_BYTES: usize = 328;
/// Byte length of elementary-action input.
pub const QVM_BOT_INPUT_BYTES: usize = 40;

/// Console message record.
#[derive(Debug, Clone, PartialEq)]
pub struct BotConsoleMessage {
    /// Message handle.
    pub handle: i32,
    /// Message time.
    pub time: f32,
    /// Message type.
    pub kind: i32,
    /// Message text.
    pub message: String,
}

/// Chat-match variable slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChatMatchVariable {
    /// Byte offset.
    pub offset: i8,
    /// Byte length.
    pub length: i32,
}

/// Owned chat-match buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMatchBuffer {
    /// Match string bytes.
    pub string: [u8; 256],
    /// Match type.
    pub match_type: i32,
    /// Match subtype.
    pub subtype: i32,
    /// Variable slots.
    pub variables: [ChatMatchVariable; 8],
}

/// Match-variable write selected by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchVariableWrite {
    /// Publish the guest string at `offset` past the match record.
    Publish {
        /// Byte offset past the match record base.
        offset: usize,
        /// Destination capacity.
        capacity: usize,
    },
    /// Clear the destination byte.
    Clear,
}

/// Camp-spot goal plus the next camp index.
#[derive(Debug, Clone, PartialEq)]
pub struct CampSpotGoal {
    /// Camp-spot goal.
    pub goal: BotGoal,
    /// Next camp index.
    pub next: i32,
}

/// Whether an origin touches a goal's expanded bounds.
pub fn touching_goal(origin: &Vec3, goal: &BotGoal) -> bool {
    let min = Vec3 {
        x: goal.mins.x - 15.0 + goal.origin.x,
        y: goal.mins.y - 15.0 + goal.origin.y,
        z: goal.mins.z - 32.0 + goal.origin.z,
    };
    let max = Vec3 {
        x: goal.maxs.x + 15.0 + goal.origin.x,
        y: goal.maxs.y + 15.0 + goal.origin.y,
        z: goal.maxs.z + 24.0 + goal.origin.z,
    };
    origin.x >= min.x
        && origin.x <= max.x
        && origin.y >= min.y
        && origin.y <= max.y
        && origin.z >= min.z
        && origin.z <= max.z
}

fn ascii_upper(byte: u8) -> u8 {
    if byte.is_ascii_lowercase() {
        byte - 32
    } else {
        byte
    }
}

/// Substring search over optional chat texts; null inputs yield -1.
pub fn string_contains(text: Option<&str>, part: Option<&str>, case_sensitive: bool) -> i32 {
    let (Some(text), Some(part)) = (text, part) else {
        return -1;
    };
    let normalize = |value: &str| -> Vec<u8> {
        if case_sensitive {
            value.bytes().collect()
        } else {
            value.bytes().map(ascii_upper).collect()
        }
    };
    let haystack = normalize(text);
    let needle = normalize(part);
    if needle.is_empty() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle.as_slice())
        .map_or(-1, |index| index as i32)
}

fn is_white_space(byte: u8) -> bool {
    !(byte.is_ascii_alphanumeric() || b"()?:'/,.[]-_+=".contains(&byte))
}

/// Collapse whitespace runs in a guest string in place, following the
/// donor's moving-pointer `memmove` behavior.
pub fn unify_white_spaces(memory: &mut SyscallMemory, word: i32) -> Result<(), GuestError> {
    let base = memory
        .pointer(word)
        .ok_or_else(|| GuestError::invalid("bot chat buffer requires a nonnull pointer"))?;
    let at = |memory: &SyscallMemory, index: usize| -> Result<u8, GuestError> {
        memory
            .get(base + index)
            .map_err(|_| GuestError::invalid("bot chat byte exceeds allocation"))
    };
    let mut pointer = 0usize;
    let mut old = 0usize;
    while at(memory, pointer)? != 0 {
        while at(memory, pointer)? != 0 && is_white_space(at(memory, pointer)?) {
            pointer += 1;
        }
        if pointer > old {
            if old > 0 && at(memory, pointer)? != 0 {
                memory.set(base + old, 32)?;
                old += 1;
            }
            if pointer > old {
                let mut end = pointer;
                while at(memory, end)? != 0 {
                    end += 1;
                }
                let count = end + 1 - pointer;
                let mut tail = Vec::with_capacity(count);
                for index in 0..count {
                    tail.push(at(memory, pointer + index)?);
                }
                for (index, byte) in tail.iter().enumerate() {
                    memory.set(base + old + index, *byte)?;
                }
            }
        }
        while at(memory, pointer)? != 0 && !is_white_space(at(memory, pointer)?) {
            pointer += 1;
        }
        old = pointer;
    }
    Ok(())
}

/// Read an owned chat-match buffer from guest memory.
pub fn read_chat_match(memory: &SyscallMemory, word: i32) -> Result<ChatMatchBuffer, GuestError> {
    let range = memory.span(word, QVM_BOT_MATCH_BYTES, 0)?;
    let mut string = [0u8; 256];
    string.copy_from_slice(memory.read_bytes(range.start, 256)?);
    let mut variables = [ChatMatchVariable::default(); 8];
    for (index, slot) in variables.iter_mut().enumerate() {
        slot.offset = memory.read_i8(range.start + 264 + index * 8)?;
        slot.length = memory.read_i32(range.start + 268 + index * 8)?;
    }
    Ok(ChatMatchBuffer {
        string,
        match_type: memory.read_i32(range.start + 256)?,
        subtype: memory.read_i32(range.start + 260)?,
        variables,
    })
}

/// Write an owned chat-match buffer to guest memory.
pub fn write_chat_match(memory: &mut SyscallMemory, word: i32, buffer: &ChatMatchBuffer) -> Result<(), GuestError> {
    let range = memory.span(word, QVM_BOT_MATCH_BYTES, 0)?;
    memory.write_bytes(range.start, &buffer.string)?;
    memory.write_i32(range.start + 256, buffer.match_type)?;
    memory.write_i32(range.start + 260, buffer.subtype)?;
    for (index, slot) in buffer.variables.iter().enumerate() {
        memory.write_i8(range.start + 264 + index * 8, slot.offset)?;
        memory.write_i32(range.start + 268 + index * 8, slot.length)?;
    }
    Ok(())
}

/// Host bot-library surface: variables, actions, characters, chat, goals,
/// weapons, sources, and genetic selection.
#[allow(clippy::too_many_lines)]
pub trait BotLibraryHost {
    /// Set a library variable.
    fn set_libvar(&mut self, name: &str, value: &str);
    /// Get a library variable string.
    fn get_libvar(&mut self, name: &str) -> String;
    /// Add a global define.
    fn add_global_define(&mut self, text: &str) -> bool;
    /// Start a library frame.
    fn start_frame(&mut self, time: f32) -> i32;
    /// Say a chat line.
    fn say(&mut self, client: i32, text: &str);
    /// Say a team chat line.
    fn say_team(&mut self, client: i32, text: &str);
    /// Send an action command.
    fn action_command(&mut self, client: i32, text: &str);
    /// Send an action code.
    fn action(&mut self, client: i32, code: i32);
    /// Play a gesture.
    fn gesture(&mut self, client: i32);
    /// Talk.
    fn talk(&mut self, client: i32);
    /// Attack.
    fn attack(&mut self, client: i32);
    /// Use.
    fn use_action(&mut self, client: i32);
    /// Respawn.
    fn respawn(&mut self, client: i32);
    /// Crouch.
    fn crouch(&mut self, client: i32);
    /// Move up.
    fn move_up(&mut self, client: i32);
    /// Move down.
    fn move_down(&mut self, client: i32);
    /// Move forward.
    fn move_forward(&mut self, client: i32);
    /// Move back.
    fn move_back(&mut self, client: i32);
    /// Move left.
    fn move_left(&mut self, client: i32);
    /// Move right.
    fn move_right(&mut self, client: i32);
    /// Select a weapon.
    fn select_weapon(&mut self, client: i32, weapon: i32);
    /// Jump.
    fn jump(&mut self, client: i32);
    /// Delayed jump.
    fn delayed_jump(&mut self, client: i32);
    /// Directed move.
    fn directed_move(&mut self, client: i32, direction: Vec3, speed: f32);
    /// Set view angles.
    fn view(&mut self, client: i32, angles: Vec3);
    /// End regular action updates.
    fn end_regular(&mut self, client: i32, think_time: f32);
    /// Action input bytes for a client.
    fn get_input_bytes(&mut self, client: i32, think_time: f32) -> [u8; QVM_BOT_INPUT_BYTES];
    /// Reset action input.
    fn reset_input(&mut self, client: i32);
    /// Use an item by name (legacy extension trap).
    fn use_item(&mut self, client: i32, name: &str);
    /// Drop an item by name (legacy extension trap).
    fn drop_item(&mut self, client: i32, name: &str);
    /// Use an inventory item by name (legacy extension trap).
    fn use_inventory(&mut self, client: i32, name: &str);
    /// Drop an inventory item by name (legacy extension trap).
    fn drop_inventory(&mut self, client: i32, name: &str);
    /// Load a character, returning its handle.
    fn load_character(&mut self, name: &str, skill: f32) -> i32;
    /// Free a character.
    fn free_character(&mut self, handle: i32);
    /// Character float characteristic.
    fn characteristic_float(&mut self, handle: i32, index: i32) -> f32;
    /// Character bounded-float characteristic.
    fn characteristic_bounded_float(&mut self, handle: i32, index: i32, min: f32, max: f32) -> f32;
    /// Character integer characteristic.
    fn characteristic_integer(&mut self, handle: i32, index: i32) -> i32;
    /// Character bounded-integer characteristic.
    fn characteristic_bounded_integer(&mut self, handle: i32, index: i32, min: i32, max: i32) -> i32;
    /// Character string characteristic.
    fn characteristic_string(&mut self, handle: i32, index: i32) -> String;
    /// Allocate a chat state.
    fn alloc_chat_state(&mut self) -> i32;
    /// Free a chat state.
    fn free_chat_state(&mut self, handle: i32);
    /// Queue a console message.
    fn queue_console_message(&mut self, handle: i32, kind: i32, text: &str);
    /// Remove console messages.
    fn remove_console_message(&mut self, handle: i32, kind: i32);
    /// Number of console messages.
    fn num_console_messages(&mut self, handle: i32) -> i32;
    /// Next console message.
    fn next_console_message(&mut self, handle: i32) -> Option<BotConsoleMessage>;
    /// Start an initial chat.
    fn initial_chat(&mut self, handle: i32, text: Option<String>, length: i32, variables: [Option<String>; 8]);
    /// Reply to a chat.
    fn reply_chat(&mut self, handle: i32, text: &str, a: i32, b: i32, variables: [Option<String>; 8]) -> bool;
    /// Number of initial chats.
    fn num_initial_chats(&mut self, handle: i32, text: Option<String>) -> i32;
    /// Chat length.
    fn chat_length(&mut self, handle: i32) -> i32;
    /// Enter a chat.
    fn enter_chat(&mut self, handle: i32, a: i32, b: i32, legacy_extra: Option<i32>);
    /// Current chat message.
    fn chat_message(&mut self, handle: i32) -> String;
    /// Replace synonyms in the guest string at `word`.
    fn replace_synonyms(&mut self, memory: &mut SyscallMemory, word: i32, context: i32);
    /// Load a chat file; `true` means success.
    fn load_chat_file(&mut self, handle: i32, a: &str, b: &str) -> bool;
    /// Set chat gender.
    fn set_chat_gender(&mut self, handle: i32, gender: i32);
    /// Set chat name.
    fn set_chat_name(&mut self, handle: i32, name: &str, extra: Option<i32>);
    /// Find a chat match, filling the buffer.
    fn find_match(&mut self, text: &str, handle: i32, buffer: &mut ChatMatchBuffer) -> bool;
    /// Select a match-variable write.
    fn match_variable(&mut self, variables: &[ChatMatchVariable; 8], a: i32, b: i32) -> MatchVariableWrite;
    /// Allocate a goal state.
    fn alloc_goal_state(&mut self, client: i32) -> i32;
    /// Free a goal state.
    fn free_goal_state(&mut self, handle: i32);
    /// Reset a goal state.
    fn reset_goal_state(&mut self, handle: i32);
    /// Reset avoid goals.
    fn reset_avoid_goals(&mut self, handle: i32);
    /// Push a goal.
    fn push_goal(&mut self, handle: i32, goal: BotGoal);
    /// Pop a goal.
    fn pop_goal(&mut self, handle: i32);
    /// Empty the goal stack.
    fn empty_goal_stack(&mut self, handle: i32);
    /// Top goal.
    fn top_goal(&mut self, handle: i32) -> Option<BotGoal>;
    /// Second goal.
    fn second_goal(&mut self, handle: i32) -> Option<BotGoal>;
    /// Goal name.
    fn goal_name(&mut self, handle: i32) -> String;
    /// Avoid-goal time.
    fn avoid_goal_time(&mut self, handle: i32, number: i32) -> f32;
    /// Set avoid-goal time.
    fn set_avoid_goal_time(&mut self, handle: i32, number: i32, time: f32);
    /// Remove from avoid goals.
    fn remove_from_avoid_goals(&mut self, handle: i32, number: i32);
    /// Load item weights, returning the config handle.
    fn load_item_weights(&mut self, handle: i32, name: &str) -> i32;
    /// Free item weights.
    fn free_item_weights(&mut self, handle: i32);
    /// Choose a long-term-goal item.
    fn choose_ltg_item(
        &mut self,
        handle: i32,
        origin: Vec3,
        inventory: &dyn Fn(i32) -> Result<i32, GuestError>,
        travel_flags: i32,
    ) -> bool;
    /// Choose a nearby-goal item.
    fn choose_nbg_item(
        &mut self,
        handle: i32,
        origin: Vec3,
        inventory: &dyn Fn(i32) -> Result<i32, GuestError>,
        travel_flags: i32,
        goal: Option<BotGoal>,
        range: f32,
    ) -> bool;
    /// Visibility probe for an item goal.
    fn item_goal_in_vis_but_not_visible(&mut self, handle: i32, view: Vec3, origin: Vec3, goal: &BotGoal) -> bool;
    /// Initialize level items.
    fn init_level_items(&mut self);
    /// Update entity items.
    fn update_entity_items(&mut self);
    /// Level-item goal.
    fn level_item_goal(&mut self, index: i32, name: &str, goal: &BotGoal) -> Option<BotGoal>;
    /// Map-location goal.
    fn map_location_goal(&mut self, name: &str, goal: &BotGoal) -> Option<BotGoal>;
    /// Next camp-spot goal.
    fn next_camp_spot_goal(&mut self, handle: i32, goal: &BotGoal) -> Option<CampSpotGoal>;
    /// Dump avoid goals.
    fn dump_avoid_goals(&mut self, handle: i32);
    /// Dump the goal stack.
    fn dump_goal_stack(&mut self, handle: i32);
    /// Interbreed goal fuzzy logic.
    fn interbreed_goal_fuzzy_logic(&mut self, a: i32, b: i32, c: i32);
    /// Mutate goal fuzzy logic.
    fn mutate_goal_fuzzy_logic(&mut self, handle: i32, range: f32);
    /// Save goal fuzzy logic.
    fn save_goal_fuzzy_logic(&mut self, handle: i32, name: &str);
    /// Allocate a weapon state.
    fn alloc_weapon_state(&mut self) -> i32;
    /// Free a weapon state.
    fn free_weapon_state(&mut self, handle: i32);
    /// Reset a weapon state.
    fn reset_weapon_state(&mut self, handle: i32);
    /// Load weapon weights, returning the config handle.
    fn load_weapon_weights(&mut self, handle: i32, name: &str) -> i32;
    /// Choose the best fight weapon.
    fn choose_best_fight_weapon(&mut self, handle: i32, inventory: &dyn Fn(i32) -> Result<i32, GuestError>) -> i32;
    /// Weapon info bytes.
    fn weapon_info(&mut self, handle: i32, weapon: i32) -> Option<Vec<u8>>;
    /// Genetic parents-and-child selection.
    fn genetic_selection(
        &mut self,
        count: i32,
        ranks: &dyn Fn(i32) -> Result<f32, GuestError>,
    ) -> Result<Option<(i32, i32, i32)>, GuestError>;
    /// Load a precompiler source, returning its handle.
    fn load_source(&mut self, name: &str) -> i32;
    /// Free a precompiler source.
    fn free_source(&mut self, handle: i32) -> bool;
    /// Read a precompiler token.
    fn read_source_token(&mut self, handle: i32) -> Option<TokenRead>;
    /// Precompiler source position: filename plus line.
    fn source_file_and_line(&mut self, handle: i32) -> Option<(String, i32)>;
}

/// Host module-lifetime surface for bot-library setup and client slots.
pub trait BotModuleServices {
    /// Set up the library.
    fn setup(&mut self) -> i32;
    /// Shut down the library.
    fn shutdown(&mut self) -> i32;
    /// Load a map.
    fn load_map(&mut self, name: &str) -> i32;
    /// Update an entity from the guest record at `word`.
    fn update_entity(&mut self, memory: &mut SyscallMemory, number: i32, word: i32) -> i32;
    /// Snapshot entity.
    fn snapshot_entity(&mut self, client: i32, sequence: i32) -> i32;
    /// Console message for a client.
    fn console_message(&mut self, client: i32) -> Option<String>;
    /// Deliver a user command.
    fn user_command(&mut self, client: i32, command: WireUserCommand);
    /// Allocate a client slot.
    fn allocate_client(&mut self) -> i32;
    /// Free a client slot.
    fn free_client(&mut self, client: i32);
}

fn read_goal(memory: &SyscallMemory, word: i32) -> Result<BotGoal, GuestError> {
    let range = memory.span(word, QVM_BOT_GOAL_BYTES, 0)?;
    read_bot_goal(memory.read_bytes(range.start, QVM_BOT_GOAL_BYTES)?)
}

fn write_goal(
    memory: &mut SyscallMemory,
    word: i32,
    goal: &BotGoal,
    fields: GoalWriteFields,
) -> Result<(), GuestError> {
    let mut record = vec![0u8; QVM_BOT_GOAL_BYTES];
    write_bot_goal(&mut record, goal, fields)?;
    let range = memory.span(word, QVM_BOT_GOAL_BYTES, 0)?;
    let needed = match fields {
        GoalWriteFields::Full => QVM_BOT_GOAL_BYTES,
        GoalWriteFields::LevelItem => 52,
        GoalWriteFields::Location => 44,
    };
    memory.write_bytes(range.start, &record[..needed])?;
    Ok(())
}

fn read_string_at(memory: &SyscallMemory, offset: usize) -> Result<String, GuestError> {
    let tail = memory
        .as_slice()
        .get(offset..)
        .ok_or_else(|| GuestError::invalid("QVM string has no terminator before the allocation ends"))?;
    let end = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| GuestError::invalid("QVM string has no terminator before the allocation ends"))?;
    Ok(tail[..end].iter().map(|byte| char::from(*byte)).collect())
}

/// Dispatch a bot-library trap. Returns `Ok(None)` when unhandled.
#[allow(clippy::too_many_lines)]
pub fn bot_library_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    library: &mut dyn BotLibraryHost,
    services: &mut dyn BotModuleServices,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine || call.role != QvmRole::Qagame {
        return Ok(None);
    }
    let nullable = |memory: &SyscallMemory, index: usize| -> Result<Option<String>, GuestError> {
        if call.int(index)? == 0 {
            Ok(None)
        } else {
            Ok(Some(memory.read_string(call.int(index)?)?))
        }
    };
    match call.code {
        G_BOT_ALLOCATE_CLIENT => Ok(Some(services.allocate_client())),
        G_BOT_FREE_CLIENT => {
            services.free_client(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_SETUP => Ok(Some(services.setup())),
        BOTLIB_SHUTDOWN => Ok(Some(services.shutdown())),
        BOTLIB_LIBVAR_SET => {
            let name = memory.read_string(call.int(1)?)?;
            let value = memory.read_string(call.int(2)?)?;
            library.set_libvar(&name, &value);
            Ok(Some(0))
        }
        BOTLIB_LIBVAR_GET => {
            let value = library.get_libvar(&memory.read_string(call.int(1)?)?);
            memory.write_string(call.int(2)?, &value, call.int(3)? as usize)?;
            Ok(Some(0))
        }
        BOTLIB_PC_ADD_GLOBAL_DEFINE => {
            let text = memory.read_string(call.int(1)?)?;
            Ok(Some(i32::from(library.add_global_define(&text))))
        }
        BOTLIB_START_FRAME => Ok(Some(library.start_frame(call.float(1)?))),
        BOTLIB_LOAD_MAP => {
            let name = memory.read_string(call.int(1)?)?;
            Ok(Some(services.load_map(&name)))
        }
        BOTLIB_UPDATENTITY => Ok(Some(services.update_entity(memory, call.int(1)?, call.int(2)?))),
        BOTLIB_GET_SNAPSHOT_ENTITY => Ok(Some(services.snapshot_entity(call.int(1)?, call.int(2)?))),
        BOTLIB_GET_CONSOLE_MESSAGE => match services.console_message(call.int(1)?) {
            None => Ok(Some(0)),
            Some(message) => {
                memory.write_string(call.int(2)?, &message, call.int(3)? as usize)?;
                Ok(Some(1))
            }
        },
        BOTLIB_USER_COMMAND => {
            let command = read_user_command(memory, call.int(2)?, call.abi_profile)?;
            services.user_command(call.int(1)?, command);
            Ok(Some(0))
        }
        BOTLIB_EA_SAY => {
            let text = memory.read_string(call.int(2)?)?;
            library.say(call.int(1)?, &text);
            Ok(Some(0))
        }
        BOTLIB_EA_SAY_TEAM => {
            let text = memory.read_string(call.int(2)?)?;
            library.say_team(call.int(1)?, &text);
            Ok(Some(0))
        }
        BOTLIB_EA_COMMAND => {
            let text = memory.read_string(call.int(2)?)?;
            library.action_command(call.int(1)?, &text);
            Ok(Some(0))
        }
        BOTLIB_EA_ACTION => {
            library.action(call.int(1)?, call.int(2)?);
            Ok(Some(0))
        }
        BOTLIB_EA_GESTURE => {
            library.gesture(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_TALK => {
            library.talk(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_ATTACK => {
            library.attack(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_USE => {
            library.use_action(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_RESPAWN => {
            library.respawn(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_CROUCH => {
            library.crouch(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_MOVE_UP => {
            library.move_up(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_MOVE_DOWN => {
            library.move_down(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_MOVE_FORWARD => {
            library.move_forward(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_MOVE_BACK => {
            library.move_back(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_MOVE_LEFT => {
            library.move_left(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_MOVE_RIGHT => {
            library.move_right(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_SELECT_WEAPON => {
            library.select_weapon(call.int(1)?, call.int(2)?);
            Ok(Some(0))
        }
        BOTLIB_EA_JUMP => {
            library.jump(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_DELAYED_JUMP => {
            library.delayed_jump(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_EA_MOVE => {
            let direction = memory.read_vec3_ptr(call.int(2)?)?;
            library.directed_move(call.int(1)?, direction, call.float(3)?);
            Ok(Some(0))
        }
        BOTLIB_EA_VIEW => {
            let angles = memory.read_vec3_ptr(call.int(2)?)?;
            library.view(call.int(1)?, angles);
            Ok(Some(0))
        }
        BOTLIB_EA_END_REGULAR => {
            library.end_regular(call.int(1)?, call.float(2)?);
            Ok(Some(0))
        }
        BOTLIB_EA_GET_INPUT => {
            let bytes = library.get_input_bytes(call.int(1)?, call.float(2)?);
            let range = memory.span(call.int(3)?, QVM_BOT_INPUT_BYTES, 0)?;
            memory.write_bytes(range.start, &bytes)?;
            Ok(Some(0))
        }
        BOTLIB_EA_RESET_INPUT => {
            library.reset_input(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_LOAD_CHARACTER => {
            let name = memory.read_string(call.int(1)?)?;
            Ok(Some(library.load_character(&name, call.float(2)?)))
        }
        BOTLIB_AI_FREE_CHARACTER => {
            library.free_character(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_CHARACTERISTIC_FLOAT => Ok(Some(
            library.characteristic_float(call.int(1)?, call.int(2)?).to_bits() as i32
        )),
        BOTLIB_AI_CHARACTERISTIC_BFLOAT => Ok(Some(
            library
                .characteristic_bounded_float(call.int(1)?, call.int(2)?, call.float(3)?, call.float(4)?)
                .to_bits() as i32,
        )),
        BOTLIB_AI_CHARACTERISTIC_INTEGER => Ok(Some(library.characteristic_integer(call.int(1)?, call.int(2)?))),
        BOTLIB_AI_CHARACTERISTIC_BINTEGER => Ok(Some(library.characteristic_bounded_integer(
            call.int(1)?,
            call.int(2)?,
            call.int(3)?,
            call.int(4)?,
        ))),
        BOTLIB_AI_CHARACTERISTIC_STRING => {
            let value = library.characteristic_string(call.int(1)?, call.int(2)?);
            memory.write_string(call.int(3)?, &value, call.int(4)? as usize)?;
            Ok(Some(0))
        }
        BOTLIB_AI_ALLOC_CHAT_STATE => Ok(Some(library.alloc_chat_state())),
        BOTLIB_AI_FREE_CHAT_STATE => {
            library.free_chat_state(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_QUEUE_CONSOLE_MESSAGE => {
            let text = memory.read_string(call.int(3)?)?;
            library.queue_console_message(call.int(1)?, call.int(2)?, &text);
            Ok(Some(0))
        }
        BOTLIB_AI_REMOVE_CONSOLE_MESSAGE => {
            library.remove_console_message(call.int(1)?, call.int(2)?);
            Ok(Some(0))
        }
        BOTLIB_AI_NEXT_CONSOLE_MESSAGE => match library.next_console_message(call.int(1)?) {
            None => Ok(Some(0)),
            Some(message) => {
                let out_word = call.int(2)?;
                memory.span(out_word, QVM_BOT_CONSOLE_MESSAGE_BYTES, 0)?;
                let base = memory.pointer(out_word).expect("checked span");
                memory.write_i32(base, message.handle)?;
                memory.write_f32(base + 4, message.time)?;
                memory.write_i32(base + 8, message.kind)?;
                let text_word = out_word
                    .checked_add(12)
                    .ok_or_else(|| GuestError::invalid("QVM console message offset overflows its pointer"))?;
                memory.write_string(text_word, &message.message, 256)?;
                memory.write_i32(base + 268, 0)?;
                memory.write_i32(base + 272, 0)?;
                Ok(Some(message.handle))
            }
        },
        BOTLIB_AI_NUM_CONSOLE_MESSAGE => Ok(Some(library.num_console_messages(call.int(1)?))),
        BOTLIB_AI_INITIAL_CHAT => {
            let text = nullable(memory, 2)?;
            let variables = [
                nullable(memory, 4)?,
                nullable(memory, 5)?,
                nullable(memory, 6)?,
                nullable(memory, 7)?,
                nullable(memory, 8)?,
                nullable(memory, 9)?,
                nullable(memory, 10)?,
                nullable(memory, 11)?,
            ];
            library.initial_chat(call.int(1)?, text, call.int(3)?, variables);
            Ok(Some(0))
        }
        BOTLIB_AI_REPLY_CHAT => {
            let text = memory.read_string(call.int(2)?)?;
            let variables = [
                nullable(memory, 5)?,
                nullable(memory, 6)?,
                nullable(memory, 7)?,
                nullable(memory, 8)?,
                nullable(memory, 9)?,
                nullable(memory, 10)?,
                nullable(memory, 11)?,
                nullable(memory, 12)?,
            ];
            Ok(Some(i32::from(library.reply_chat(
                call.int(1)?,
                &text,
                call.int(3)?,
                call.int(4)?,
                variables,
            ))))
        }
        BOTLIB_AI_CHAT_LENGTH => Ok(Some(library.chat_length(call.int(1)?))),
        BOTLIB_AI_NUM_INITIAL_CHATS => Ok(Some(library.num_initial_chats(call.int(1)?, nullable(memory, 2)?))),
        BOTLIB_AI_GET_CHAT_MESSAGE => {
            let message = library.chat_message(call.int(1)?);
            memory.write_string(call.int(2)?, &message, call.int(3)? as usize)?;
            Ok(Some(0))
        }
        BOTLIB_AI_ENTER_CHAT => {
            library.enter_chat(call.int(1)?, call.int(2)?, call.int(3)?, None);
            Ok(Some(0))
        }
        BOTLIB_AI_STRING_CONTAINS => {
            let text = nullable(memory, 1)?;
            let part = nullable(memory, 2)?;
            Ok(Some(string_contains(
                text.as_deref(),
                part.as_deref(),
                call.int(3)? != 0,
            )))
        }
        BOTLIB_AI_FIND_MATCH => {
            let text = memory.read_string(call.int(1)?)?;
            let out_word = call.int(2)?;
            let mut buffer = read_chat_match(memory, out_word)?;
            let found = library.find_match(&text, call.int(3)?, &mut buffer);
            write_chat_match(memory, out_word, &buffer)?;
            Ok(Some(i32::from(found)))
        }
        BOTLIB_AI_MATCH_VARIABLE => {
            let record_word = call.int(1)?;
            let buffer = read_chat_match(memory, record_word)?;
            match library.match_variable(&buffer.variables, call.int(2)?, call.int(4)?) {
                MatchVariableWrite::Publish { offset, capacity } => {
                    let base = memory.pointer(record_word).expect("checked span");
                    let start = base
                        .checked_add(offset)
                        .ok_or_else(|| GuestError::invalid("QVM match variable offset overflows its record"))?;
                    let text = read_string_at(memory, start)?;
                    memory.write_string(call.int(3)?, &text, capacity)?;
                }
                MatchVariableWrite::Clear => {
                    let range = memory.span(call.int(3)?, 1, 0)?;
                    memory.set(range.start, 0)?;
                }
            }
            Ok(Some(0))
        }
        BOTLIB_AI_UNIFY_WHITE_SPACES => {
            unify_white_spaces(memory, call.int(1)?)?;
            Ok(Some(0))
        }
        BOTLIB_AI_REPLACE_SYNONYMS => {
            library.replace_synonyms(memory, call.int(1)?, call.int(2)?);
            Ok(Some(0))
        }
        BOTLIB_AI_LOAD_CHAT_FILE => {
            let a = memory.read_string(call.int(2)?)?;
            let b = memory.read_string(call.int(3)?)?;
            Ok(Some(if library.load_chat_file(call.int(1)?, &a, &b) {
                0
            } else {
                8
            }))
        }
        BOTLIB_AI_SET_CHAT_GENDER => {
            library.set_chat_gender(call.int(1)?, call.int(2)?);
            Ok(Some(0))
        }
        BOTLIB_AI_SET_CHAT_NAME => {
            let name = memory.read_string(call.int(2)?)?;
            library.set_chat_name(call.int(1)?, &name, Some(call.int(3)?));
            Ok(Some(0))
        }
        BOTLIB_AI_ALLOC_GOAL_STATE => Ok(Some(library.alloc_goal_state(call.int(1)?))),
        BOTLIB_AI_FREE_GOAL_STATE => {
            library.free_goal_state(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_RESET_GOAL_STATE => {
            library.reset_goal_state(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_RESET_AVOID_GOALS => {
            library.reset_avoid_goals(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_PUSH_GOAL => {
            let goal = read_goal(memory, call.int(2)?)?;
            library.push_goal(call.int(1)?, goal);
            Ok(Some(0))
        }
        BOTLIB_AI_POP_GOAL => {
            library.pop_goal(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_EMPTY_GOAL_STACK => {
            library.empty_goal_stack(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_GET_TOP_GOAL | BOTLIB_AI_GET_SECOND_GOAL => {
            let goal = if call.code == BOTLIB_AI_GET_TOP_GOAL {
                library.top_goal(call.int(1)?)
            } else {
                library.second_goal(call.int(1)?)
            };
            match goal {
                None => Ok(Some(0)),
                Some(goal) => {
                    write_goal(memory, call.int(2)?, &goal, GoalWriteFields::Full)?;
                    Ok(Some(1))
                }
            }
        }
        BOTLIB_AI_GOAL_NAME => {
            let name = library.goal_name(call.int(1)?);
            memory.write_string(call.int(2)?, &name, call.int(3)? as usize)?;
            Ok(Some(0))
        }
        BOTLIB_AI_AVOID_GOAL_TIME => Ok(Some(
            library.avoid_goal_time(call.int(1)?, call.int(2)?).to_bits() as i32
        )),
        BOTLIB_AI_SET_AVOID_GOAL_TIME => {
            library.set_avoid_goal_time(call.int(1)?, call.int(2)?, call.float(3)?);
            Ok(Some(0))
        }
        BOTLIB_AI_REMOVE_FROM_AVOID_GOALS => {
            library.remove_from_avoid_goals(call.int(1)?, call.int(2)?);
            Ok(Some(0))
        }
        BOTLIB_AI_LOAD_ITEM_WEIGHTS => {
            let name = memory.read_string(call.int(2)?)?;
            Ok(Some(library.load_item_weights(call.int(1)?, &name)))
        }
        BOTLIB_AI_FREE_ITEM_WEIGHTS => {
            library.free_item_weights(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_CHOOSE_LTG_ITEM => {
            let origin = memory.read_vec3_ptr(call.int(2)?)?;
            let array = call.int(3)?;
            let inventory = |item: i32| -> Result<i32, GuestError> {
                if item < 0 {
                    return Err(GuestError::invalid("QVM bot inventory index is negative"));
                }
                let range = memory.span(array, 4, i64::from(item) * 4)?;
                memory.read_i32(range.start)
            };
            Ok(Some(i32::from(library.choose_ltg_item(
                call.int(1)?,
                origin,
                &inventory,
                call.int(4)?,
            ))))
        }
        BOTLIB_AI_CHOOSE_NBG_ITEM => {
            let origin = memory.read_vec3_ptr(call.int(2)?)?;
            let array = call.int(3)?;
            let inventory = |item: i32| -> Result<i32, GuestError> {
                if item < 0 {
                    return Err(GuestError::invalid("QVM bot inventory index is negative"));
                }
                let range = memory.span(array, 4, i64::from(item) * 4)?;
                memory.read_i32(range.start)
            };
            let goal = if call.int(5)? == 0 {
                None
            } else {
                Some(read_goal(memory, call.int(5)?)?)
            };
            Ok(Some(i32::from(library.choose_nbg_item(
                call.int(1)?,
                origin,
                &inventory,
                call.int(4)?,
                goal,
                call.float(6)?,
            ))))
        }
        BOTLIB_AI_TOUCHING_GOAL => {
            let origin = memory.read_vec3_ptr(call.int(1)?)?;
            let goal = read_goal(memory, call.int(2)?)?;
            Ok(Some(i32::from(touching_goal(&origin, &goal))))
        }
        BOTLIB_AI_ITEM_GOAL_IN_VIS_BUT_NOT_VISIBLE => {
            let view = memory.read_vec3_ptr(call.int(2)?)?;
            let origin = memory.read_vec3_ptr(call.int(3)?)?;
            let goal = read_goal(memory, call.int(4)?)?;
            Ok(Some(i32::from(library.item_goal_in_vis_but_not_visible(
                call.int(1)?,
                view,
                origin,
                &goal,
            ))))
        }
        BOTLIB_AI_INIT_LEVEL_ITEMS => {
            library.init_level_items();
            Ok(Some(0))
        }
        BOTLIB_AI_UPDATE_ENTITY_ITEMS => {
            library.update_entity_items();
            Ok(Some(0))
        }
        BOTLIB_AI_ALLOC_WEAPON_STATE => Ok(Some(library.alloc_weapon_state())),
        BOTLIB_AI_FREE_WEAPON_STATE => {
            library.free_weapon_state(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_RESET_WEAPON_STATE => {
            library.reset_weapon_state(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_LOAD_WEAPON_WEIGHTS => {
            let name = memory.read_string(call.int(2)?)?;
            Ok(Some(library.load_weapon_weights(call.int(1)?, &name)))
        }
        BOTLIB_AI_CHOOSE_BEST_FIGHT_WEAPON => {
            let array = call.int(2)?;
            let inventory = |item: i32| -> Result<i32, GuestError> {
                if item < 0 {
                    return Err(GuestError::invalid("QVM bot inventory index is negative"));
                }
                let range = memory.span(array, 4, i64::from(item) * 4)?;
                memory.read_i32(range.start)
            };
            Ok(Some(library.choose_best_fight_weapon(call.int(1)?, &inventory)))
        }
        BOTLIB_AI_GET_WEAPON_INFO => {
            if let Some(bytes) = library.weapon_info(call.int(1)?, call.int(2)?) {
                let out_word = call.int(3)?;
                memory.span(out_word, bytes.len(), 0)?;
                let base = memory.pointer(out_word).expect("checked span");
                memory.write_bytes(base, &bytes)?;
            }
            Ok(Some(0))
        }
        BOTLIB_AI_GET_LEVEL_ITEM_GOAL | BOTLIB_AI_GET_MAP_LOCATION_GOAL => {
            let location = call.code == BOTLIB_AI_GET_MAP_LOCATION_GOAL;
            let output = if location { 2 } else { 3 };
            let goal = read_goal(memory, call.int(output)?)?;
            if location {
                let name = memory.read_string(call.int(1)?)?;
                match library.map_location_goal(&name, &goal) {
                    None => Ok(Some(0)),
                    Some(found) => {
                        write_goal(memory, call.int(output)?, &found, GoalWriteFields::Location)?;
                        Ok(Some(1))
                    }
                }
            } else {
                let name = memory.read_string(call.int(2)?)?;
                match library.level_item_goal(call.int(1)?, &name, &goal) {
                    None => Ok(Some(0)),
                    Some(found) => {
                        let number = found.number;
                        write_goal(memory, call.int(output)?, &found, GoalWriteFields::LevelItem)?;
                        Ok(Some(number))
                    }
                }
            }
        }
        BOTLIB_AI_GET_NEXT_CAMP_SPOT_GOAL => {
            let goal = read_goal(memory, call.int(2)?)?;
            match library.next_camp_spot_goal(call.int(1)?, &goal) {
                None => Ok(Some(0)),
                Some(spot) => {
                    write_goal(memory, call.int(2)?, &spot.goal, GoalWriteFields::Full)?;
                    Ok(Some(spot.next))
                }
            }
        }
        BOTLIB_AI_DUMP_AVOID_GOALS => {
            library.dump_avoid_goals(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_DUMP_GOAL_STACK => {
            library.dump_goal_stack(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_INTERBREED_GOAL_FUZZY_LOGIC => {
            library.interbreed_goal_fuzzy_logic(call.int(1)?, call.int(2)?, call.int(3)?);
            Ok(Some(0))
        }
        BOTLIB_AI_MUTATE_GOAL_FUZZY_LOGIC => {
            library.mutate_goal_fuzzy_logic(call.int(1)?, call.float(2)?);
            Ok(Some(0))
        }
        BOTLIB_AI_SAVE_GOAL_FUZZY_LOGIC => {
            let name = memory.read_string(call.int(2)?)?;
            library.save_goal_fuzzy_logic(call.int(1)?, &name);
            Ok(Some(0))
        }
        BOTLIB_AI_GENETIC_PARENTS_AND_CHILD_SELECTION => {
            let array = call.int(2)?;
            let ranks = |index: i32| -> Result<f32, GuestError> {
                if index < 0 {
                    return Err(GuestError::invalid("QVM genetic rank index is negative"));
                }
                let range = memory.span(array, 4, i64::from(index) * 4)?;
                memory.read_f32(range.start)
            };
            match library.genetic_selection(call.int(1)?, &ranks)? {
                None => Ok(Some(0)),
                Some((parent1, parent2, child)) => {
                    for (word_index, value) in [(3, parent1), (4, parent2), (5, child)] {
                        let word = call.int(word_index)?;
                        memory.span(word, 4, 0)?;
                        let base = memory.pointer(word).expect("checked span");
                        memory.write_i32(base, value)?;
                    }
                    Ok(Some(1))
                }
            }
        }
        BOTLIB_PC_LOAD_SOURCE => {
            let name = memory.read_string(call.int(1)?)?;
            Ok(Some(library.load_source(&name)))
        }
        BOTLIB_PC_FREE_SOURCE => Ok(Some(i32::from(library.free_source(call.int(1)?)))),
        BOTLIB_PC_READ_TOKEN => match library.read_source_token(call.int(1)?) {
            None => Ok(Some(0)),
            Some(outcome) => {
                write_script_token(memory, call.int(2)?, &outcome.token)?;
                Ok(Some(i32::from(outcome.read)))
            }
        },
        BOTLIB_PC_SOURCE_FILE_AND_LINE => match library.source_file_and_line(call.int(1)?) {
            None => Ok(Some(0)),
            Some((filename, line)) => {
                memory.write_string(call.int(2)?, &filename, 128)?;
                memory.span(call.int(3)?, 4, 0)?;
                let base = memory.pointer(call.int(3)?).expect("checked span");
                memory.write_i32(base, line)?;
                Ok(Some(1))
            }
        },
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::super::client_script_syscalls::{ScriptToken, ScriptTokenKind};
    use super::super::client_state::AbiProfile;
    use super::super::client_state_record::UserCommandWrite;
    use super::*;

    struct FakeLibrary {
        log: Vec<String>,
    }

    fn goal(number: i32) -> BotGoal {
        BotGoal {
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            area: 4,
            mins: Vec3 {
                x: -1.0,
                y: -1.0,
                z: -1.0,
            },
            maxs: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
            entity: 5,
            number,
            flags: 6,
            item_info: 7,
        }
    }

    #[allow(clippy::too_many_lines)]
    impl BotLibraryHost for FakeLibrary {
        fn set_libvar(&mut self, name: &str, value: &str) {
            self.log.push(format!("libvar {name}={value}"));
        }
        fn get_libvar(&mut self, name: &str) -> String {
            format!("v:{name}")
        }
        fn add_global_define(&mut self, text: &str) -> bool {
            self.log.push(format!("define {text}"));
            true
        }
        fn start_frame(&mut self, time: f32) -> i32 {
            self.log.push(format!("frame {time}"));
            0
        }
        fn say(&mut self, client: i32, text: &str) {
            self.log.push(format!("say {client} {text}"));
        }
        fn say_team(&mut self, client: i32, text: &str) {
            self.log.push(format!("sayteam {client} {text}"));
        }
        fn action_command(&mut self, client: i32, text: &str) {
            self.log.push(format!("cmd {client} {text}"));
        }
        fn action(&mut self, client: i32, code: i32) {
            self.log.push(format!("action {client} {code}"));
        }
        fn gesture(&mut self, client: i32) {
            self.log.push(format!("gesture {client}"));
        }
        fn talk(&mut self, client: i32) {
            self.log.push(format!("talk {client}"));
        }
        fn attack(&mut self, client: i32) {
            self.log.push(format!("attack {client}"));
        }
        fn use_action(&mut self, client: i32) {
            self.log.push(format!("use {client}"));
        }
        fn respawn(&mut self, client: i32) {
            self.log.push(format!("respawn {client}"));
        }
        fn crouch(&mut self, client: i32) {
            self.log.push(format!("crouch {client}"));
        }
        fn move_up(&mut self, client: i32) {
            self.log.push(format!("up {client}"));
        }
        fn move_down(&mut self, client: i32) {
            self.log.push(format!("down {client}"));
        }
        fn move_forward(&mut self, client: i32) {
            self.log.push(format!("fwd {client}"));
        }
        fn move_back(&mut self, client: i32) {
            self.log.push(format!("back {client}"));
        }
        fn move_left(&mut self, client: i32) {
            self.log.push(format!("left {client}"));
        }
        fn move_right(&mut self, client: i32) {
            self.log.push(format!("right {client}"));
        }
        fn select_weapon(&mut self, client: i32, weapon: i32) {
            self.log.push(format!("weapon {client} {weapon}"));
        }
        fn jump(&mut self, client: i32) {
            self.log.push(format!("jump {client}"));
        }
        fn delayed_jump(&mut self, client: i32) {
            self.log.push(format!("djump {client}"));
        }
        fn directed_move(&mut self, client: i32, _direction: Vec3, speed: f32) {
            self.log.push(format!("move {client} {speed}"));
        }
        fn view(&mut self, client: i32, _angles: Vec3) {
            self.log.push(format!("view {client}"));
        }
        fn end_regular(&mut self, client: i32, think_time: f32) {
            self.log.push(format!("end {client} {think_time}"));
        }
        fn get_input_bytes(&mut self, _client: i32, _think_time: f32) -> [u8; QVM_BOT_INPUT_BYTES] {
            [7u8; QVM_BOT_INPUT_BYTES]
        }
        fn reset_input(&mut self, client: i32) {
            self.log.push(format!("reset {client}"));
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
        fn free_character(&mut self, handle: i32) {
            self.log.push(format!("freechar {handle}"));
        }
        fn characteristic_float(&mut self, _handle: i32, _index: i32) -> f32 {
            1.5
        }
        fn characteristic_bounded_float(&mut self, _handle: i32, _index: i32, _min: f32, _max: f32) -> f32 {
            2.5
        }
        fn characteristic_integer(&mut self, _handle: i32, _index: i32) -> i32 {
            11
        }
        fn characteristic_bounded_integer(&mut self, _handle: i32, _index: i32, _min: i32, _max: i32) -> i32 {
            12
        }
        fn characteristic_string(&mut self, _handle: i32, _index: i32) -> String {
            "aggressive".to_string()
        }
        fn alloc_chat_state(&mut self) -> i32 {
            3
        }
        fn free_chat_state(&mut self, handle: i32) {
            self.log.push(format!("freechat {handle}"));
        }
        fn queue_console_message(&mut self, handle: i32, kind: i32, text: &str) {
            self.log.push(format!("queue {handle} {kind} {text}"));
        }
        fn remove_console_message(&mut self, handle: i32, kind: i32) {
            self.log.push(format!("remove {handle} {kind}"));
        }
        fn num_console_messages(&mut self, _handle: i32) -> i32 {
            2
        }
        fn next_console_message(&mut self, handle: i32) -> Option<BotConsoleMessage> {
            (handle == 3).then(|| BotConsoleMessage {
                handle: 3,
                time: 1.0,
                kind: 4,
                message: "hello".to_string(),
            })
        }
        fn initial_chat(&mut self, handle: i32, text: Option<String>, length: i32, variables: [Option<String>; 8]) {
            self.log.push(format!(
                "initial {handle} {text:?} {length} {}",
                variables.iter().flatten().count()
            ));
        }
        fn reply_chat(&mut self, _handle: i32, _text: &str, _a: i32, _b: i32, _variables: [Option<String>; 8]) -> bool {
            true
        }
        fn num_initial_chats(&mut self, _handle: i32, _text: Option<String>) -> i32 {
            5
        }
        fn chat_length(&mut self, _handle: i32) -> i32 {
            6
        }
        fn enter_chat(&mut self, handle: i32, a: i32, b: i32, legacy_extra: Option<i32>) {
            self.log.push(format!("enter {handle} {a} {b} {legacy_extra:?}"));
        }
        fn chat_message(&mut self, _handle: i32) -> String {
            "chat!".to_string()
        }
        fn replace_synonyms(&mut self, _memory: &mut SyscallMemory, word: i32, context: i32) {
            self.log.push(format!("synonyms {word} {context}"));
        }
        fn load_chat_file(&mut self, _handle: i32, _a: &str, _b: &str) -> bool {
            true
        }
        fn set_chat_gender(&mut self, handle: i32, gender: i32) {
            self.log.push(format!("gender {handle} {gender}"));
        }
        fn set_chat_name(&mut self, handle: i32, name: &str, extra: Option<i32>) {
            self.log.push(format!("chatname {handle} {name} {extra:?}"));
        }
        fn find_match(&mut self, _text: &str, _handle: i32, buffer: &mut ChatMatchBuffer) -> bool {
            buffer.match_type = 9;
            true
        }
        fn match_variable(&mut self, _variables: &[ChatMatchVariable; 8], a: i32, _b: i32) -> MatchVariableWrite {
            if a == 0 {
                MatchVariableWrite::Publish {
                    offset: 0,
                    capacity: 64,
                }
            } else {
                MatchVariableWrite::Clear
            }
        }
        fn alloc_goal_state(&mut self, client: i32) -> i32 {
            100 + client
        }
        fn free_goal_state(&mut self, handle: i32) {
            self.log.push(format!("freegoal {handle}"));
        }
        fn reset_goal_state(&mut self, handle: i32) {
            self.log.push(format!("resetgoal {handle}"));
        }
        fn reset_avoid_goals(&mut self, handle: i32) {
            self.log.push(format!("resetavoid {handle}"));
        }
        fn push_goal(&mut self, handle: i32, goal: BotGoal) {
            self.log.push(format!("push {handle} {}", goal.area));
        }
        fn pop_goal(&mut self, handle: i32) {
            self.log.push(format!("pop {handle}"));
        }
        fn empty_goal_stack(&mut self, handle: i32) {
            self.log.push(format!("empty {handle}"));
        }
        fn top_goal(&mut self, _handle: i32) -> Option<BotGoal> {
            Some(goal(21))
        }
        fn second_goal(&mut self, _handle: i32) -> Option<BotGoal> {
            None
        }
        fn goal_name(&mut self, _handle: i32) -> String {
            "goal-a".to_string()
        }
        fn avoid_goal_time(&mut self, _handle: i32, _number: i32) -> f32 {
            3.5
        }
        fn set_avoid_goal_time(&mut self, handle: i32, number: i32, time: f32) {
            self.log.push(format!("avoidtime {handle} {number} {time}"));
        }
        fn remove_from_avoid_goals(&mut self, handle: i32, number: i32) {
            self.log.push(format!("unavoid {handle} {number}"));
        }
        fn load_item_weights(&mut self, _handle: i32, _name: &str) -> i32 {
            13
        }
        fn free_item_weights(&mut self, handle: i32) {
            self.log.push(format!("freeweights {handle}"));
        }
        fn choose_ltg_item(
            &mut self,
            _handle: i32,
            _origin: Vec3,
            inventory: &dyn Fn(i32) -> Result<i32, GuestError>,
            _travel_flags: i32,
        ) -> bool {
            inventory(0).unwrap_or(0) > 0
        }
        fn choose_nbg_item(
            &mut self,
            _handle: i32,
            _origin: Vec3,
            _inventory: &dyn Fn(i32) -> Result<i32, GuestError>,
            _travel_flags: i32,
            goal: Option<BotGoal>,
            _range: f32,
        ) -> bool {
            goal.is_some()
        }
        fn item_goal_in_vis_but_not_visible(
            &mut self,
            _handle: i32,
            _view: Vec3,
            _origin: Vec3,
            _goal: &BotGoal,
        ) -> bool {
            true
        }
        fn init_level_items(&mut self) {
            self.log.push("initlevel".to_string());
        }
        fn update_entity_items(&mut self) {
            self.log.push("updateitems".to_string());
        }
        fn level_item_goal(&mut self, _index: i32, _name: &str, _goal: &BotGoal) -> Option<BotGoal> {
            Some(goal(31))
        }
        fn map_location_goal(&mut self, _name: &str, _goal: &BotGoal) -> Option<BotGoal> {
            Some(goal(32))
        }
        fn next_camp_spot_goal(&mut self, _handle: i32, _goal: &BotGoal) -> Option<CampSpotGoal> {
            Some(CampSpotGoal {
                goal: goal(33),
                next: 2,
            })
        }
        fn dump_avoid_goals(&mut self, handle: i32) {
            self.log.push(format!("dumpavoid {handle}"));
        }
        fn dump_goal_stack(&mut self, handle: i32) {
            self.log.push(format!("dumpstack {handle}"));
        }
        fn interbreed_goal_fuzzy_logic(&mut self, a: i32, b: i32, c: i32) {
            self.log.push(format!("interbreed {a} {b} {c}"));
        }
        fn mutate_goal_fuzzy_logic(&mut self, handle: i32, range: f32) {
            self.log.push(format!("mutate {handle} {range}"));
        }
        fn save_goal_fuzzy_logic(&mut self, handle: i32, name: &str) {
            self.log.push(format!("savefuzzy {handle} {name}"));
        }
        fn alloc_weapon_state(&mut self) -> i32 {
            41
        }
        fn free_weapon_state(&mut self, handle: i32) {
            self.log.push(format!("freeweapon {handle}"));
        }
        fn reset_weapon_state(&mut self, handle: i32) {
            self.log.push(format!("resetweapon {handle}"));
        }
        fn load_weapon_weights(&mut self, _handle: i32, _name: &str) -> i32 {
            42
        }
        fn choose_best_fight_weapon(
            &mut self,
            _handle: i32,
            _inventory: &dyn Fn(i32) -> Result<i32, GuestError>,
        ) -> i32 {
            7
        }
        fn weapon_info(&mut self, handle: i32, _weapon: i32) -> Option<Vec<u8>> {
            (handle == 41).then(|| vec![1, 2, 3, 4])
        }
        fn genetic_selection(
            &mut self,
            count: i32,
            ranks: &dyn Fn(i32) -> Result<f32, GuestError>,
        ) -> Result<Option<(i32, i32, i32)>, GuestError> {
            if count < 2 {
                return Ok(None);
            }
            let _ = ranks(0)?;
            Ok(Some((1, 2, 3)))
        }
        fn load_source(&mut self, name: &str) -> i32 {
            self.log.push(format!("loadsrc {name}"));
            51
        }
        fn free_source(&mut self, _handle: i32) -> bool {
            true
        }
        fn read_source_token(&mut self, handle: i32) -> Option<TokenRead> {
            (handle == 51).then(|| TokenRead {
                token: ScriptToken {
                    text: "name".to_string(),
                    kind: ScriptTokenKind::Name,
                    subtype: 0,
                    integer_value: 0,
                    float_value: 0.0,
                },
                read: true,
            })
        }
        fn source_file_and_line(&mut self, handle: i32) -> Option<(String, i32)> {
            (handle == 51).then(|| ("bot.c".to_string(), 10))
        }
    }

    struct FakeServices {
        log: Vec<String>,
    }

    impl BotModuleServices for FakeServices {
        fn setup(&mut self) -> i32 {
            1
        }
        fn shutdown(&mut self) -> i32 {
            2
        }
        fn load_map(&mut self, name: &str) -> i32 {
            self.log.push(format!("map {name}"));
            3
        }
        fn update_entity(&mut self, _memory: &mut SyscallMemory, number: i32, word: i32) -> i32 {
            self.log.push(format!("entity {number} {word}"));
            4
        }
        fn snapshot_entity(&mut self, client: i32, sequence: i32) -> i32 {
            1000 + client + sequence
        }
        fn console_message(&mut self, client: i32) -> Option<String> {
            (client == 1).then(|| "msg".to_string())
        }
        fn user_command(&mut self, client: i32, command: WireUserCommand) {
            self.log.push(format!("usercmd {client} {}", command.server_time));
        }
        fn allocate_client(&mut self) -> i32 {
            6
        }
        fn free_client(&mut self, client: i32) {
            self.log.push(format!("free {client}"));
        }
    }

    fn game(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Qagame, code, args, AbiProfile::Modern)
    }

    fn harness() -> (SyscallMemory, FakeLibrary, FakeServices) {
        (
            SyscallMemory::new(65536).unwrap(),
            FakeLibrary { log: Vec::new() },
            FakeServices { log: Vec::new() },
        )
    }

    fn write_goal(memory: &mut SyscallMemory, at: i32, number: i32) {
        let mut record = vec![0u8; QVM_BOT_GOAL_BYTES];
        write_bot_goal(&mut record, &goal(number), GoalWriteFields::Full).unwrap();
        memory.write_bytes(at as usize, &record).unwrap();
    }

    #[test]
    fn lifecycle_and_vars() {
        let (mut memory, mut library, mut services) = harness();
        memory.write_string(256, "skill", 6).unwrap();
        memory.write_string(512, "4", 2).unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(G_BOT_ALLOCATE_CLIENT, &[]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(6)
        );
        assert_eq!(
            bot_library_syscall(&game(G_BOT_FREE_CLIENT, &[6]), &mut memory, &mut library, &mut services).unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(&game(BOTLIB_SETUP, &[]), &mut memory, &mut library, &mut services).unwrap(),
            Some(1)
        );
        assert_eq!(
            bot_library_syscall(&game(BOTLIB_SHUTDOWN, &[]), &mut memory, &mut library, &mut services).unwrap(),
            Some(2)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_LIBVAR_SET, &[256, 512]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_LIBVAR_GET, &[256, 1024, 64]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_string(1024).unwrap(), "v:skill");
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_PC_ADD_GLOBAL_DEFINE, &[256]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        let time = 1.0f32.to_bits() as i32;
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_START_FRAME, &[time]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        memory.write_string(256, "q3dm1", 6).unwrap();
        assert_eq!(
            bot_library_syscall(&game(BOTLIB_LOAD_MAP, &[256]), &mut memory, &mut library, &mut services).unwrap(),
            Some(3)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_UPDATENTITY, &[5, 1024]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(4)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_GET_SNAPSHOT_ENTITY, &[1, 2]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1003)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_GET_CONSOLE_MESSAGE, &[1, 1024, 64]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_string(1024).unwrap(), "msg");
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_GET_CONSOLE_MESSAGE, &[2, 1024, 64]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
    }

    #[test]
    fn user_command_delivery() {
        let (mut memory, mut library, mut services) = harness();
        let command = WireUserCommand {
            server_time: 777,
            buttons: 5,
            ..WireUserCommand::default()
        };
        super::super::client_state_record::write_user_command(
            &mut memory,
            1024,
            &command,
            AbiProfile::Modern,
            UserCommandWrite::Encode,
        )
        .unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_USER_COMMAND, &[1, 1024]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(services.log, vec!["usercmd 1 777".to_string()]);
    }

    #[test]
    fn elementary_actions() {
        let (mut memory, mut library, mut services) = harness();
        memory.write_string(256, "hi", 3).unwrap();
        memory.write_vec3(512, &Vec3 { x: 1.0, y: 0.0, z: 0.0 }).unwrap();
        let speed = 200.0f32.to_bits() as i32;
        let arms = [
            (BOTLIB_EA_SAY, vec![1, 256]),
            (BOTLIB_EA_SAY_TEAM, vec![1, 256]),
            (BOTLIB_EA_COMMAND, vec![1, 256]),
            (BOTLIB_EA_ACTION, vec![1, 3]),
            (BOTLIB_EA_GESTURE, vec![1]),
            (BOTLIB_EA_TALK, vec![1]),
            (BOTLIB_EA_ATTACK, vec![1]),
            (BOTLIB_EA_USE, vec![1]),
            (BOTLIB_EA_RESPAWN, vec![1]),
            (BOTLIB_EA_CROUCH, vec![1]),
            (BOTLIB_EA_MOVE_UP, vec![1]),
            (BOTLIB_EA_MOVE_DOWN, vec![1]),
            (BOTLIB_EA_MOVE_FORWARD, vec![1]),
            (BOTLIB_EA_MOVE_BACK, vec![1]),
            (BOTLIB_EA_MOVE_LEFT, vec![1]),
            (BOTLIB_EA_MOVE_RIGHT, vec![1]),
            (BOTLIB_EA_SELECT_WEAPON, vec![1, 5]),
            (BOTLIB_EA_JUMP, vec![1]),
            (BOTLIB_EA_DELAYED_JUMP, vec![1]),
            (BOTLIB_EA_MOVE, vec![1, 512, speed]),
            (BOTLIB_EA_VIEW, vec![1, 512]),
            (BOTLIB_EA_END_REGULAR, vec![1, speed]),
            (BOTLIB_EA_RESET_INPUT, vec![1]),
        ];
        for (code, args) in &arms {
            assert_eq!(
                bot_library_syscall(&game(*code, args), &mut memory, &mut library, &mut services).unwrap(),
                Some(0),
                "trap {code}"
            );
        }
        assert_eq!(library.log.len(), arms.len());
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_EA_GET_INPUT, &[1, speed, 1024]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_bytes(1024, 40).unwrap(), &[7u8; 40]);
    }

    #[test]
    fn characteristics() {
        let (mut memory, mut library, mut services) = harness();
        memory.write_string(256, "sarge", 6).unwrap();
        let skill = 4.0f32.to_bits() as i32;
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_LOAD_CHARACTER, &[256, skill]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(9)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_FREE_CHARACTER, &[9]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_CHARACTERISTIC_FLOAT, &[9, 1]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1.5f32.to_bits() as i32)
        );
        let min = 0.0f32.to_bits() as i32;
        let max = 1.0f32.to_bits() as i32;
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_CHARACTERISTIC_BFLOAT, &[9, 1, min, max]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(2.5f32.to_bits() as i32)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_CHARACTERISTIC_INTEGER, &[9, 1]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(11)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_CHARACTERISTIC_BINTEGER, &[9, 1, 0, 10]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(12)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_CHARACTERISTIC_STRING, &[9, 1, 1024, 64]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_string(1024).unwrap(), "aggressive");
    }

    #[test]
    fn chat_states_and_messages() {
        let (mut memory, mut library, mut services) = harness();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_ALLOC_CHAT_STATE, &[]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(3)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_FREE_CHAT_STATE, &[3]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        memory.write_string(256, "boom", 5).unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_QUEUE_CONSOLE_MESSAGE, &[3, 1, 256]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_REMOVE_CONSOLE_MESSAGE, &[3, 1]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_NUM_CONSOLE_MESSAGE, &[3]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(2)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_NEXT_CONSOLE_MESSAGE, &[3, 1024]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(3)
        );
        assert_eq!(memory.read_i32(1024).unwrap(), 3);
        assert_eq!(memory.read_f32(1028).unwrap(), 1.0);
        assert_eq!(memory.read_i32(1032).unwrap(), 4);
        assert_eq!(memory.read_string(1036).unwrap(), "hello");
        assert_eq!(memory.read_i32(1292).unwrap(), 0);
        assert_eq!(memory.read_i32(1296).unwrap(), 0);
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_NEXT_CONSOLE_MESSAGE, &[9, 1024]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
    }

    #[test]
    fn chat_flows_and_string_contains() {
        let (mut memory, mut library, mut services) = harness();
        memory.write_string(256, "hey", 4).unwrap();
        memory.write_string(512, "var", 4).unwrap();
        let args = [3, 256, 8, 512, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_INITIAL_CHAT, &args),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        let args = [3, 256, 1, 2, 512, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_REPLY_CHAT, &args),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_CHAT_LENGTH, &[3]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(6)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_ENTER_CHAT, &[3, 1, 2]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_NUM_INITIAL_CHATS, &[3, 256]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(5)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_GET_CHAT_MESSAGE, &[3, 1024, 64]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_string(1024).unwrap(), "chat!");
        assert!(library.log.iter().any(|line| line == "enter 3 1 2 None"));
        assert!(library.log.iter().any(|line| line == "initial 3 Some(\"hey\") 8 1"));
    }

    #[test]
    fn chat_message_write_missing_arm() {
        let (mut memory, mut library, mut services) = harness();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_GET_CHAT_MESSAGE, &[3, 1024, 64]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_string(1024).unwrap(), "chat!");
    }

    #[test]
    fn string_contains_cases() {
        assert_eq!(string_contains(Some("Hello World"), Some("world"), false), 6);
        assert_eq!(string_contains(Some("Hello World"), Some("world"), true), -1);
        assert_eq!(string_contains(Some("abc"), Some(""), false), 0);
        assert_eq!(string_contains(None, Some("x"), false), -1);
        assert_eq!(string_contains(Some("x"), None, false), -1);
        let (mut memory, mut library, mut services) = harness();
        memory.write_string(256, "Hello World", 12).unwrap();
        memory.write_string(512, "WORLD", 6).unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_STRING_CONTAINS, &[256, 512, 0]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(6)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_STRING_CONTAINS, &[0, 512, 0]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(-1)
        );
    }

    #[test]
    fn unify_white_spaces_collapses_runs() {
        let (mut memory, _library, _services) = harness();
        memory.write_string(256, "  hello\t\tworld  ", 18).unwrap();
        unify_white_spaces(&mut memory, 256).unwrap();
        assert_eq!(memory.read_string(256).unwrap(), "hello world");
        assert!(unify_white_spaces(&mut memory, 0).is_err());
    }

    #[test]
    fn find_match_and_match_variable() {
        let (mut memory, mut library, mut services) = harness();
        memory.write_string(256, "hello there", 12).unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_FIND_MATCH, &[256, 1024, 3]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_i32(1024 + 256).unwrap(), 9);
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_MATCH_VARIABLE, &[1024, 0, 2048, 0]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_MATCH_VARIABLE, &[1024, 1, 2048, 0]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(memory.get(2048).unwrap(), 0);
    }

    #[test]
    fn chat_files_gender_and_name() {
        let (mut memory, mut library, mut services) = harness();
        memory.write_string(256, "a", 2).unwrap();
        memory.write_string(512, "b", 2).unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_LOAD_CHAT_FILE, &[3, 256, 512]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_SET_CHAT_GENDER, &[3, 1]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_SET_CHAT_NAME, &[3, 256, 7]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_REPLACE_SYNONYMS, &[256, 1]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert!(library.log.iter().any(|line| line == "chatname 3 a Some(7)"));
    }

    #[test]
    fn goal_stack_and_choice() {
        let (mut memory, mut library, mut services) = harness();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_ALLOC_GOAL_STATE, &[1]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(101)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_FREE_GOAL_STATE, &[101]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_RESET_GOAL_STATE, &[101]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_RESET_AVOID_GOALS, &[101]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        write_goal(&mut memory, 2048, 40);
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_PUSH_GOAL, &[101, 2048]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_POP_GOAL, &[101]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_EMPTY_GOAL_STACK, &[101]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_GET_TOP_GOAL, &[101, 3072]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_i32(3072 + 44).unwrap(), 21);
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_GET_SECOND_GOAL, &[101, 3072]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_GOAL_NAME, &[101, 1024, 64]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_string(1024).unwrap(), "goal-a");
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_AVOID_GOAL_TIME, &[101, 2]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(3.5f32.to_bits() as i32)
        );
        let time = 2.0f32.to_bits() as i32;
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_SET_AVOID_GOAL_TIME, &[101, 2, time]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_REMOVE_FROM_AVOID_GOALS, &[101, 2]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        memory.write_string(256, "items.c", 8).unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_LOAD_ITEM_WEIGHTS, &[101, 256]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(13)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_FREE_ITEM_WEIGHTS, &[13]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
    }

    #[test]
    fn item_choice_and_touching() {
        let (mut memory, mut library, mut services) = harness();
        memory.write_vec3(256, &Vec3 { x: 1.0, y: 2.0, z: 3.0 }).unwrap();
        memory.write_i32(512, 3).unwrap();
        write_goal(&mut memory, 2048, 40);
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_CHOOSE_LTG_ITEM, &[101, 256, 512, 7]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        memory.write_i32(512, 0).unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_CHOOSE_LTG_ITEM, &[101, 256, 512, 7]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        let range = 64.0f32.to_bits() as i32;
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_CHOOSE_NBG_ITEM, &[101, 256, 512, 7, 2048, range]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_CHOOSE_NBG_ITEM, &[101, 256, 512, 7, 0, range]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_TOUCHING_GOAL, &[256, 2048]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        memory
            .write_vec3(
                256,
                &Vec3 {
                    x: 500.0,
                    y: 0.0,
                    z: 0.0,
                },
            )
            .unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_TOUCHING_GOAL, &[256, 2048]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        memory
            .write_vec3(
                768,
                &Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 60.0,
                },
            )
            .unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_ITEM_GOAL_IN_VIS_BUT_NOT_VISIBLE, &[101, 768, 256, 2048]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_INIT_LEVEL_ITEMS, &[]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_UPDATE_ENTITY_ITEMS, &[]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
    }

    #[test]
    fn weapons() {
        let (mut memory, mut library, mut services) = harness();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_ALLOC_WEAPON_STATE, &[]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(41)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_FREE_WEAPON_STATE, &[41]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_RESET_WEAPON_STATE, &[41]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        memory.write_string(256, "weapons.c", 10).unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_LOAD_WEAPON_WEIGHTS, &[41, 256]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(42)
        );
        memory.write_i32(512, 1).unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_CHOOSE_BEST_FIGHT_WEAPON, &[41, 512]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(7)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_GET_WEAPON_INFO, &[41, 7, 1024]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_bytes(1024, 4).unwrap(), &[1, 2, 3, 4]);
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_GET_WEAPON_INFO, &[9, 7, 1024]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
    }

    #[test]
    fn level_item_map_location_and_camp_goals() {
        let (mut memory, mut library, mut services) = harness();
        write_goal(&mut memory, 2048, 40);
        memory.write_string(256, "item", 5).unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_GET_LEVEL_ITEM_GOAL, &[2, 256, 2048]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(31)
        );
        assert_eq!(memory.read_i32(2048 + 44).unwrap(), 31);
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_GET_MAP_LOCATION_GOAL, &[256, 2048]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_GET_NEXT_CAMP_SPOT_GOAL, &[101, 2048]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(2)
        );
        assert_eq!(memory.read_i32(2048 + 44).unwrap(), 33);
    }

    #[test]
    fn fuzzy_genetic_and_dumps() {
        let (mut memory, mut library, mut services) = harness();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_DUMP_AVOID_GOALS, &[101]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_DUMP_GOAL_STACK, &[101]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_INTERBREED_GOAL_FUZZY_LOGIC, &[1, 2, 3]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        let range = 0.5f32.to_bits() as i32;
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_MUTATE_GOAL_FUZZY_LOGIC, &[1, range]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        memory.write_string(256, "fuzzy.c", 8).unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_AI_SAVE_GOAL_FUZZY_LOGIC, &[1, 256]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        memory.write_f32(512, 0.9).unwrap();
        memory.write_f32(516, 0.1).unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(
                    BOTLIB_AI_GENETIC_PARENTS_AND_CHILD_SELECTION,
                    &[2, 512, 1024, 1028, 1032]
                ),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_i32(1024).unwrap(), 1);
        assert_eq!(memory.read_i32(1028).unwrap(), 2);
        assert_eq!(memory.read_i32(1032).unwrap(), 3);
        assert_eq!(
            bot_library_syscall(
                &game(
                    BOTLIB_AI_GENETIC_PARENTS_AND_CHILD_SELECTION,
                    &[1, 512, 1024, 1028, 1032]
                ),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
    }

    #[test]
    fn precompiler_sources() {
        let (mut memory, mut library, mut services) = harness();
        memory.write_string(256, "script.c", 9).unwrap();
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_PC_LOAD_SOURCE, &[256]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(51)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_PC_FREE_SOURCE, &[51]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_PC_READ_TOKEN, &[51, 1024]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_i32(1024).unwrap(), 4);
        assert_eq!(memory.read_string(1040).unwrap(), "name");
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_PC_READ_TOKEN, &[9, 1024]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_PC_SOURCE_FILE_AND_LINE, &[51, 2048, 3072]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_string(2048).unwrap(), "bot.c");
        assert_eq!(memory.read_i32(3072).unwrap(), 10);
        assert_eq!(
            bot_library_syscall(
                &game(BOTLIB_PC_SOURCE_FILE_AND_LINE, &[9, 2048, 3072]),
                &mut memory,
                &mut library,
                &mut services
            )
            .unwrap(),
            Some(0)
        );
    }

    #[test]
    fn routing_and_unhandled() {
        let (mut memory, mut library, mut services) = harness();
        let other = HostCall::engine(QvmRole::Cgame, BOTLIB_SETUP, &[], AbiProfile::Modern);
        assert_eq!(
            bot_library_syscall(&other, &mut memory, &mut library, &mut services).unwrap(),
            None
        );
        assert_eq!(
            bot_library_syscall(&game(208, &[]), &mut memory, &mut library, &mut services).unwrap(),
            None
        );
        assert_eq!(
            bot_library_syscall(&game(999, &[]), &mut memory, &mut library, &mut services).unwrap(),
            None
        );
    }
}
