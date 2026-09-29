//! AI commands from `src/bots/behavior/q3/ai-command.ts`
//! (`game/ai_cmd.c`: `BotMatchMessage`, `BotQueueConsoleMessage`,
//! `BotRemoveConsoleMessage`, `BotNextConsoleMessage`,
//! `BotNumConsoleMessages`, `BotGetConsoleMessage`,
//! `BotCheckForTeamMates`, team-message handlers).
//!
//! Incoming team chat matches against templates; matched orders
//! (help, accompany, defend, camp, patrol, kill, ...) dispatch to
//! the voice handlers with the speaker as decision maker. Console
//! messages queue per bot with sender and addressee filters.

use crate::behavior::library::genetic::BotRandom;
use crate::behavior::q3::ai_context::GameAiContext;
use crate::behavior::q3::ai_definitions::{BotMessage, BotTeamTaskPreference};
use crate::behavior::q3::ai_orders::client_name;
use crate::behavior::q3::ai_team::{bot_client_on_same_team_from_name, bot_set_team_mate_task_preference};
use crate::behavior::q3::ai_voice;
use crate::behavior::q3::game_host::SourceBotGame;
use crate::behavior::q3::library::BotLibrary;

/// Queued console message.
#[derive(Debug, Clone, PartialEq)]
pub struct QueuedConsoleMessage {
    /// Recipient client.
    pub client: i32,
    /// Sender client.
    pub sender: i32,
    /// Addressee client, or -1 for all.
    pub addressee: i32,
    /// Message text.
    pub message: String,
    /// Queue time.
    pub time: f32,
}

/// Console message queue.
#[derive(Debug, Clone, Default)]
pub struct ConsoleMessageQueue {
    messages: Vec<QueuedConsoleMessage>,
}

impl ConsoleMessageQueue {
    /// Empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a message.
    pub fn queue(&mut self, message: QueuedConsoleMessage) {
        self.messages.push(message);
    }

    /// Next message for a client.
    #[must_use]
    pub fn next_for(&self, client: i32) -> Option<&QueuedConsoleMessage> {
        self.messages
            .iter()
            .find(|message| message.client == client || message.addressee == client)
    }

    /// Remove messages for a client up to a time.
    pub fn remove_for(&mut self, client: i32, time: f32) {
        self.messages
            .retain(|message| message.client != client || message.time > time);
    }

    /// Message count for a client.
    #[must_use]
    pub fn count_for(&self, client: i32) -> usize {
        self.messages.iter().filter(|message| message.client == client).count()
    }
}

/// Match a team message to an order id (`BotMatchMessage`).
#[must_use]
pub fn bot_match_message(message: &str) -> Option<i32> {
    let lower = message.to_lowercase();
    let table: &[(&str, i32)] = &[
        ("help", BotMessage::HELP),
        ("accompany", BotMessage::ACCOMPANY),
        ("defend", BotMessage::DEFENDKEYAREA),
        ("rush", BotMessage::RUSHBASE),
        ("get the flag", BotMessage::GETFLAG),
        ("getflag", BotMessage::GETFLAG),
        ("camp", BotMessage::CAMP),
        ("checkpoint", BotMessage::CHECKPOINT),
        ("patrol", BotMessage::PATROL),
        ("lead", BotMessage::LEADTHEWAY),
        ("get item", BotMessage::GETITEM),
        ("kill", BotMessage::KILL),
        ("where are you", BotMessage::WHEREAREYOU),
        ("return", BotMessage::RETURNFLAG),
        ("attack", BotMessage::ATTACKENEMYBASE),
        ("harvest", BotMessage::HARVEST),
        ("wait", BotMessage::WAIT),
        ("dismiss", BotMessage::DISMISS),
        ("formation", BotMessage::DOFORMATION),
        ("suicide", BotMessage::SUICIDE),
    ];
    table.iter().find(|(key, _)| lower.contains(key)).map(|(_, id)| *id)
}

/// Handle a matched team message.
pub fn bot_handle_team_message(
    context: &mut GameAiContext,
    game: &dyn SourceBotGame,
    client: i32,
    sender: i32,
    message_id: i32,
    _random: &mut dyn BotRandom,
) {
    match message_id {
        BotMessage::GETFLAG => ai_voice::bot_voice_chat_get_flag(context, game, client, sender),
        BotMessage::RUSHBASE | BotMessage::ATTACKENEMYBASE => {
            ai_voice::bot_voice_chat_offense(context, game, client, sender)
        }
        BotMessage::DEFENDKEYAREA => ai_voice::bot_voice_chat_defend(context, game, client, sender),
        BotMessage::RETURNFLAG => ai_voice::bot_voice_chat_defend_flag(context, game, client, sender),
        BotMessage::PATROL | BotMessage::DISMISS => ai_voice::bot_voice_chat_patrol(context, client, sender),
        BotMessage::CAMP => ai_voice::bot_voice_chat_camp(context, game, client, sender),
        BotMessage::ACCOMPANY | BotMessage::HELP => ai_voice::bot_voice_chat_follow_me(context, game, client, sender),
        BotMessage::TASKPREFERENCE => {
            let name = client_name(game, client);
            bot_set_team_mate_task_preference(context, &name, BotTeamTaskPreference::Attacker as i32);
        }
        BotMessage::SUICIDE => {
            if let Some(state) = context.states.get_mut(client) {
                state.bot_suicide = true;
            }
        }
        _ => {}
    }
}

/// Process queued console messages for a bot.
pub fn bot_process_console_messages(
    context: &mut GameAiContext,
    game: &dyn SourceBotGame,
    queue: &mut ConsoleMessageQueue,
    client: i32,
    random: &mut dyn BotRandom,
) {
    let due: Vec<QueuedConsoleMessage> = queue
        .messages
        .iter()
        .filter(|message| message.client == client)
        .cloned()
        .collect();
    for message in due {
        if let Some(id) = bot_match_message(&message.message) {
            bot_handle_team_message(context, game, client, message.sender, id, random);
        }
    }
    queue.remove_for(client, context.time);
}

/// Check for teammates and greet them (`BotCheckForTeamMates`).
pub fn bot_check_for_teammates(
    context: &mut GameAiContext,
    library: &mut BotLibrary<'_>,
    game: &dyn SourceBotGame,
    client: i32,
    random: &mut dyn BotRandom,
) {
    let mates = context
        .states
        .get(client)
        .map(|state| crate::behavior::q3::ai_orders::bot_teammates(game, state))
        .unwrap_or_default();
    if mates.is_empty() {
        return;
    }
    let Some(state) = context.states.get(client) else {
        return;
    };
    if state.enter_game_chat {
        return;
    }
    let handle = state.cs;
    let time = context.time;
    library.chat.initial_chat(
        handle,
        "entergame",
        &[],
        crate::behavior::library::chat::ChatDestination::Team,
        time,
        random,
    );
    if let Some(state) = context.states.get_mut(client) {
        state.enter_game_chat = true;
    }
}

/// Resolve an addressed teammate by name from a message.
#[must_use]
pub fn bot_addressed_teammate(game: &dyn SourceBotGame, context: &GameAiContext, client: i32, name: &str) -> i32 {
    bot_client_on_same_team_from_name(game, context, client, name)
}
