//! Game AI chat glue from `src/bots/behavior/q3/ai-chat.ts`
//! (`game/ai_chat.c`: `BotInitialChat`, `BotNumInitialChats`,
//! `BotChat`, `BotReplyChat`, `BotChatTest`, `BotChatTime`,
//! `BotChatEndLevel`, `BotChatStartLevel`).
//!
//! Thin wrappers that feed game events (kills, deaths, level
//! transitions, random chatter) into the chat library with the bot's
//! chattiness characteristics gating each line.

use crate::behavior::library::character::{BotCharacterLibrary, Characteristic};
use crate::behavior::library::chat::{empty_chat_variables, BotChatLibrary, ChatDestination};
use crate::behavior::library::genetic::BotRandom;
use crate::behavior::q3::ai_context::GameAiContext;

/// Initial chat helper: queue a named initial chat when the
/// chattiness characteristic passes a unit draw.
pub fn bot_initial_chat(
    context: &mut GameAiContext,
    chat: &mut BotChatLibrary<'_>,
    characters: &BotCharacterLibrary,
    client: i32,
    name: &str,
    chattiness: i32,
    random: &mut dyn BotRandom,
) -> bool {
    let Some(state) = context.states.get(client) else {
        return false;
    };
    let chance = characters.bounded_float(state.character, chattiness, 0.0, 1.0);
    if random.next_unit() > chance {
        return false;
    }
    let handle = state.cs;
    let time = context.time;
    chat.initial_chat(handle, name, &[], ChatDestination::All, time, random)
}

/// Queue a kill chat line.
pub fn bot_chat_kill(
    context: &mut GameAiContext,
    chat: &mut BotChatLibrary<'_>,
    characters: &BotCharacterLibrary,
    client: i32,
    victim_name: &str,
    random: &mut dyn BotRandom,
) -> bool {
    let Some(state) = context.states.get(client) else {
        return false;
    };
    let chance = characters.bounded_float(state.character, Characteristic::CHAT_KILL, 0.0, 1.0);
    if random.next_unit() > chance {
        return false;
    }
    let handle = state.cs;
    let time = context.time;
    chat.initial_chat(
        handle,
        "kill",
        &[Some(victim_name.to_owned())],
        ChatDestination::All,
        time,
        random,
    )
}

/// Queue a death chat line.
pub fn bot_chat_death(
    context: &mut GameAiContext,
    chat: &mut BotChatLibrary<'_>,
    characters: &BotCharacterLibrary,
    client: i32,
    killer_name: &str,
    random: &mut dyn BotRandom,
) -> bool {
    let Some(state) = context.states.get(client) else {
        return false;
    };
    let chance = characters.bounded_float(state.character, Characteristic::CHAT_DEATH, 0.0, 1.0);
    if random.next_unit() > chance {
        return false;
    }
    let handle = state.cs;
    let time = context.time;
    chat.initial_chat(
        handle,
        "death",
        &[Some(killer_name.to_owned())],
        ChatDestination::All,
        time,
        random,
    )
}

/// Queue a random chat line.
pub fn bot_chat_random(
    context: &mut GameAiContext,
    chat: &mut BotChatLibrary<'_>,
    characters: &BotCharacterLibrary,
    client: i32,
    random: &mut dyn BotRandom,
) -> bool {
    bot_initial_chat(
        context,
        chat,
        characters,
        client,
        "random",
        Characteristic::CHAT_RANDOM,
        random,
    )
}

/// Queue a level-start chat line.
pub fn bot_chat_start_level(
    context: &mut GameAiContext,
    chat: &mut BotChatLibrary<'_>,
    characters: &BotCharacterLibrary,
    client: i32,
    random: &mut dyn BotRandom,
) -> bool {
    bot_initial_chat(
        context,
        chat,
        characters,
        client,
        "startlevel",
        Characteristic::CHAT_START_END_LEVEL,
        random,
    )
}

/// Queue a level-end chat line.
pub fn bot_chat_end_level(
    context: &mut GameAiContext,
    chat: &mut BotChatLibrary<'_>,
    characters: &BotCharacterLibrary,
    client: i32,
    random: &mut dyn BotRandom,
) -> bool {
    bot_initial_chat(
        context,
        chat,
        characters,
        client,
        "endlevel",
        Characteristic::CHAT_START_END_LEVEL,
        random,
    )
}

/// Test an incoming message and reply when a template matches.
pub fn bot_chat_reply(
    context: &mut GameAiContext,
    chat: &mut BotChatLibrary<'_>,
    client: i32,
    message: &str,
    time: f32,
    random: &mut dyn BotRandom,
) -> bool {
    let Some(matched) = chat.chat_test(message) else {
        return false;
    };
    let Some(state) = context.states.get(client) else {
        return false;
    };
    let handle = state.cs;
    chat.reply_chat(
        handle,
        &matched.text,
        ChatDestination::All,
        &matched.variables,
        time,
        random,
    )
}

/// Empty variable reply (tests and glue).
#[must_use]
pub fn bot_empty_reply_variables() -> [Option<String>; 8] {
    empty_chat_variables()
}
