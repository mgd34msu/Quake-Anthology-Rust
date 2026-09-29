//! Bot chat runtime from `src/bots/behavior/library/chat.ts`
//! (`be_ai_chat.c`: `BotAllocChatState`, `BotChat`, `BotReplyChat`,
//! `BotChatTest`, `BotConsoleMessage`, `BotEnterChat`, `BotNumInitialChats`,
//! `BotInitialChat`, `BotPrintReplyChat`, `BotChatLength`,
//! `BotExpandChatMessage`).
//!
//! Each bot owns a chat state: gender, name, console message queue,
//! match variables, and reply selection. Incoming console messages are
//! matched against templates; replies expand `0`-`7` variables, random
//! lists (`[name]`), and synonyms before queueing chat output with
//! characters-per-minute timing.

use std::collections::HashMap;

use super::chat_data::{
    unify_white_spaces, ChatData, MatchPiece, ReplyKeyMode, ReplyTest, CHAT_MESSAGE_SIZE, CHAT_VARIABLE_COUNT,
};
use super::genetic::BotRandom;
use crate::behavior::assets::BotSourceFiles;
use crate::error::BotsError;

/// Chat gender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChatGender {
    /// Genderless.
    Genderless = 0,
    /// Female.
    Female = 1,
    /// Male.
    Male = 2,
}

/// Chat destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChatDestination {
    /// Everyone.
    All = 0,
    /// Team.
    Team = 1,
    /// Tell.
    Tell = 2,
}

/// Captured match variables: 8 slots of optional text.
pub type ChatVariables = [Option<String>; CHAT_VARIABLE_COUNT];

/// Empty variable set.
#[must_use]
pub fn empty_chat_variables() -> ChatVariables {
    [None, None, None, None, None, None, None, None]
}

/// A matched console message.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMatch {
    /// Matched text.
    pub text: String,
    /// Message type.
    pub message_type: i32,
    /// Message subtype.
    pub subtype: i32,
    /// Captured variables.
    pub variables: ChatVariables,
}

/// Queued console message.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleChatMessage {
    /// Chat state handle.
    pub handle: i32,
    /// Queue time seconds.
    pub time: f32,
    /// Message type.
    pub message_type: i32,
    /// Message text.
    pub message: String,
}

/// Chat diagnostic.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatDiagnostic {
    /// Severity.
    pub severity: String,
    /// Message.
    pub message: String,
    /// Source.
    pub source: String,
}

/// Per-bot chat state.
#[derive(Debug, Clone)]
pub struct BotChatState {
    /// Bot gender.
    pub gender: ChatGender,
    /// Bot chat name.
    pub name: String,
    /// Characters per minute.
    pub cpm: i32,
    /// Match variables from the last match.
    pub variables: ChatVariables,
    /// Queued outgoing messages: (time, destination, text).
    pub outgoing: Vec<(f32, ChatDestination, String)>,
    /// Console message queue.
    pub console: Vec<ConsoleChatMessage>,
    /// Last chat time.
    pub last_chat_time: f32,
    /// Reply pause end.
    pub chat_pause: f32,
}

impl BotChatState {
    fn new(gender: ChatGender, name: &str) -> Self {
        Self {
            gender,
            name: name.to_owned(),
            cpm: 400,
            variables: empty_chat_variables(),
            outgoing: Vec::new(),
            console: Vec::new(),
            last_chat_time: 0.0,
            chat_pause: 0.0,
        }
    }
}

/// Bot chat library.
pub struct BotChatLibrary<'a> {
    files: &'a dyn BotSourceFiles,
    data: ChatData,
    states: Vec<Option<BotChatState>>,
    diagnostics: Vec<ChatDiagnostic>,
}

impl<'a> BotChatLibrary<'a> {
    /// New chat library over prepared files.
    pub fn new(files: &'a dyn BotSourceFiles) -> Self {
        Self {
            files,
            data: ChatData::default(),
            states: vec![None; 65],
            diagnostics: Vec::new(),
        }
    }

    /// Reported diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[ChatDiagnostic] {
        &self.diagnostics
    }

    /// Load a chat file (`BotLoadChatFile`).
    pub fn load_chat_file(&mut self, path: &str) -> Result<(), BotsError> {
        let bytes = self
            .files
            .read(path)
            .ok_or_else(|| BotsError::BotScript(format!("missing chat file {path}")))?;
        let text = String::from_utf8_lossy(&bytes);
        self.data = ChatData::parse(&text)?;
        Ok(())
    }

    /// Configuration counts.
    #[must_use]
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        self.data.counts()
    }

    /// Allocate a chat state (`BotAllocChatState`).
    pub fn alloc_chat_state(&mut self) -> Option<i32> {
        self.states.iter().position(Option::is_none).map(|index| {
            self.states[index] = Some(BotChatState::new(ChatGender::Genderless, "bot"));
            index as i32 + 1
        })
    }

    /// Free a chat state (`BotFreeChatState`).
    pub fn free_chat_state(&mut self, handle: i32) {
        if let Some(slot) = handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize))
        {
            *slot = None;
        }
    }

    /// Set chat metadata (`BotSetChatGender`, `BotSetChatName`,
    /// `BotSetChatCpm`).
    pub fn set_chat_meta(&mut self, handle: i32, gender: ChatGender, name: &str, cpm: i32) {
        if let Some(Some(state)) = handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize))
        {
            state.gender = gender;
            state.name = name.to_owned();
            state.cpm = cpm.max(1);
        }
    }

    fn state(&self, handle: i32) -> Option<&BotChatState> {
        handle
            .checked_sub(1)
            .and_then(|index| self.states.get(index as usize)?.as_ref())
    }

    fn state_mut(&mut self, handle: i32) -> Option<&mut BotChatState> {
        handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize)?.as_mut())
    }

    /// Queue a console message (`BotConsoleMessage`).
    pub fn console_message(&mut self, handle: i32, message_type: i32, message: &str, time: f32) {
        if let Some(state) = self.state_mut(handle) {
            state.console.push(ConsoleChatMessage {
                handle,
                time,
                message_type,
                message: message.to_owned(),
            });
        }
    }

    /// Remove console messages up to a time (`BotRemoveConsoleMessage`).
    pub fn remove_console_message(&mut self, handle: i32, time: f32) {
        if let Some(state) = self.state_mut(handle) {
            state.console.retain(|message| message.time > time);
        }
    }

    /// Enter chat mode for a tell (`BotEnterChat`).
    pub fn enter_chat(&mut self, handle: i32, _client: i32, _destination: ChatDestination) {
        if let Some(state) = self.state_mut(handle) {
            state.chat_pause = state.last_chat_time;
        }
    }

    /// Match a message against templates (`BotChatTest`).
    #[must_use]
    pub fn chat_test(&self, message: &str) -> Option<ChatMatch> {
        let unified = unify_white_spaces(message).to_lowercase();
        for template in &self.data.matches {
            if let Some(variables) = match_template(template, &unified) {
                return Some(ChatMatch {
                    text: message.to_owned(),
                    message_type: template.message_type,
                    subtype: template.subtype,
                    variables,
                });
            }
        }
        None
    }

    /// Reply to the last matched message (`BotReplyChat`).
    pub fn reply_chat(
        &mut self,
        handle: i32,
        message: &str,
        destination: ChatDestination,
        variables: &ChatVariables,
        time: f32,
        random: &mut dyn BotRandom,
    ) -> bool {
        let unified = unify_white_spaces(message).to_lowercase();
        let mut best: Option<(f32, String)> = None;
        for reply in &self.data.replies {
            if reply_keys_match(&reply.keys, variables, &unified)
                && best.as_ref().is_none_or(|(score, _)| reply.priority >= *score)
            {
                if let Some(text) = reply.messages.first() {
                    best = Some((reply.priority, text.clone()));
                }
            }
        }
        let Some((_, text)) = best else {
            return false;
        };
        let expanded = self.expand_chat_message(&text, variables, random);
        if let Some(state) = self.state_mut(handle) {
            state.outgoing.push((time, destination, expanded));
            state.variables = variables.clone();
            true
        } else {
            false
        }
    }

    /// Queue a free-form chat line (`BotChat`).
    pub fn chat(&mut self, handle: i32, destination: ChatDestination, text: &str, time: f32) {
        if let Some(state) = self.state_mut(handle) {
            state.outgoing.push((time, destination, text.to_owned()));
        }
    }

    /// Number of initial chats (`BotNumInitialChats`).
    #[must_use]
    pub fn num_initial_chats(&self, name: &str) -> usize {
        self.data.initial.messages(name).len()
    }

    /// Play an initial chat (`BotInitialChat`).
    pub fn initial_chat(
        &mut self,
        handle: i32,
        name: &str,
        variables: &[Option<String>],
        destination: ChatDestination,
        time: f32,
        random: &mut dyn BotRandom,
    ) -> bool {
        let messages = self.data.initial.messages(name).to_vec();
        if messages.is_empty() {
            return false;
        }
        let pick = (random.next_unit() * messages.len() as f32) as usize % messages.len();
        let mut slots = empty_chat_variables();
        for (index, value) in variables.iter().take(CHAT_VARIABLE_COUNT).enumerate() {
            slots[index] = value.clone();
        }
        let expanded = self.expand_chat_message(&messages[pick], &slots, random);
        if let Some(state) = self.state_mut(handle) {
            state.outgoing.push((time, destination, expanded));
            state.variables = slots;
            true
        } else {
            false
        }
    }

    /// Expand variables, randoms, and synonyms (`BotExpandChatMessage`).
    pub fn expand_chat_message(&self, text: &str, variables: &ChatVariables, random: &mut dyn BotRandom) -> String {
        let mut out = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '[' {
                let mut name = String::new();
                for d in chars.by_ref() {
                    if d == ']' {
                        break;
                    }
                    name.push(d);
                }
                if let Some(list) = self.data.random(&name) {
                    if !list.messages.is_empty() {
                        let pick = (random.next_unit() * list.messages.len() as f32) as usize % list.messages.len();
                        out.push_str(&list.messages[pick]);
                    }
                }
            } else if c.is_ascii_digit() {
                let slot = c.to_digit(10).unwrap_or(0) as usize;
                if let Some(value) = variables.get(slot).and_then(Clone::clone) {
                    out.push_str(&value);
                }
            } else {
                out.push(c);
            }
        }
        // Expand synonym head words in place.
        let mut expanded = out;
        for group in &self.data.synonyms {
            if let Some(head) = group.entries.first() {
                if expanded.contains(&head.text) {
                    let pick = group.pick(random.next_unit()).to_owned();
                    expanded = expanded.replacen(&head.text, &pick, 1);
                }
            }
        }
        truncate_chat(&expanded).to_owned()
    }

    /// Chat typing length in seconds (`BotChatLength`).
    #[must_use]
    pub fn chat_length(&self, handle: i32, text: &str) -> f32 {
        let cpm = self.state(handle).map_or(400, |state| state.cpm).max(1) as f32;
        text.len() as f32 / cpm * 60.0
    }

    /// Drain outgoing messages due at `time`.
    pub fn drain_outgoing(&mut self, handle: i32, time: f32) -> Vec<(ChatDestination, String)> {
        let mut due = Vec::new();
        if let Some(state) = self.state_mut(handle) {
            let mut pending = Vec::new();
            for (at, destination, text) in state.outgoing.drain(..) {
                if at <= time {
                    due.push((destination, text));
                } else {
                    pending.push((at, destination, text));
                }
            }
            state.outgoing = pending;
            if !due.is_empty() {
                state.last_chat_time = time;
            }
        }
        due
    }

    /// Inspect outgoing queue (tests).
    #[must_use]
    pub fn outgoing(&self, handle: i32) -> &[(f32, ChatDestination, String)] {
        self.state(handle).map_or(&[], |state| state.outgoing.as_slice())
    }

    /// Variable map helpers for game glue.
    #[must_use]
    pub fn variable_map(variables: &ChatVariables) -> HashMap<String, String> {
        variables
            .iter()
            .enumerate()
            .filter_map(|(index, value)| value.clone().map(|text| (index.to_string(), text)))
            .collect()
    }
}

fn match_template(template: &super::chat_data::MatchTemplate, unified: &str) -> Option<ChatVariables> {
    let mut variables = empty_chat_variables();
    let mut cursor = 0usize;
    for piece in &template.pieces {
        match piece {
            MatchPiece::Literal(text) => {
                let lower = text.to_lowercase();
                let rest = &unified[cursor.min(unified.len())..];
                let found = rest.find(&lower)?;
                cursor += found + lower.len();
            }
            MatchPiece::Variable(slot) => {
                let rest = unified[cursor.min(unified.len())..].trim_start();
                let word = rest.split_whitespace().next().unwrap_or("");
                variables[*slot] = Some(word.to_owned());
                cursor += rest.len() - rest[word.len()..].len();
            }
            MatchPiece::Synonym(_) => {
                let rest = unified[cursor.min(unified.len())..].trim_start();
                let word = rest.split_whitespace().next().unwrap_or("");
                if word.is_empty() {
                    return None;
                }
                cursor += rest.len() - rest[word.len()..].len();
            }
        }
    }
    Some(variables)
}

fn reply_keys_match(keys: &[super::chat_data::ReplyKey], variables: &ChatVariables, unified: &str) -> bool {
    if keys.is_empty() {
        return true;
    }
    let mut any_match = false;
    let mut any_key = false;
    for key in keys {
        let matched = match &key.test {
            ReplyTest::Equals { variable, text } => variables
                .get(*variable)
                .and_then(Clone::clone)
                .is_some_and(|value| value.eq_ignore_ascii_case(text)),
            ReplyTest::Contains { variable, text } => variables
                .get(*variable)
                .and_then(Clone::clone)
                .is_some_and(|value| value.to_lowercase().contains(&text.to_lowercase())),
            ReplyTest::Set { variable } => variables.get(*variable).and_then(Clone::clone).is_some(),
        };
        match key.mode {
            ReplyKeyMode::Any => {
                any_key = true;
                any_match |= matched;
            }
            ReplyKeyMode::And => {
                if !matched {
                    return false;
                }
            }
            ReplyKeyMode::Not => {
                if matched {
                    return false;
                }
            }
        }
    }
    let _ = unified;
    !any_key || any_match
}

fn truncate_chat(text: &str) -> &str {
    if text.len() <= CHAT_MESSAGE_SIZE {
        text
    } else {
        let mut end = CHAT_MESSAGE_SIZE;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        &text[..end]
    }
}
