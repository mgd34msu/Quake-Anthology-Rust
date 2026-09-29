//! Rerelease chat text from `src/bots/behavior/rerelease/chat-text.ts`.
//!
//! Q1 `chats.txt` names numbered localization families; the line is
//! selected from the bot's saved random stream.

use crate::behavior::rerelease::rng::{random_index, BotRandomT};

/// Resolve a chat locstring through numbered variants.
pub fn q1_bot_chat_text(
    locstring: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    random: &mut dyn BotRandomT,
) -> String {
    let key = locstring.strip_prefix('$').unwrap_or(locstring);
    let mut variants = Vec::new();
    for index in 0..64 {
        match lookup(&format!("{key}_{index}")) {
            Some(text) => variants.push(text),
            None => break,
        }
    }
    if !variants.is_empty() {
        return variants[random_index(random, variants.len())].clone();
    }
    lookup(key).unwrap_or_else(|| key.to_owned())
}
