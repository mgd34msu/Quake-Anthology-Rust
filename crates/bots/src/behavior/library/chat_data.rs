//! Chat data from `src/bots/behavior/library/chat-data.ts`
//! (`be_ai_chat.c` parse half: synonyms, random strings, match
//! templates, reply chats, initial chats).
//!
//! Chat files declare `synonym`, `random`, `match`, and `reply` blocks.
//! Match templates capture variables (`0`-`7`) from player chat; replies
//! select messages whose keys test those variables.

use crate::behavior::library::structure::tokenize_bot_script;
use crate::error::BotsError;

/// Chat escape byte.
pub const CHAT_ESCAPE: char = '\x01';
/// Maximum chat message bytes.
pub const CHAT_MESSAGE_SIZE: usize = 256;
/// Match variable count.
pub const CHAT_VARIABLE_COUNT: usize = 8;

/// Compare chat text with optional case sensitivity (`ChatTextEquals`).
#[must_use]
pub fn chat_text_equals(expected: &str, input: Option<&str>, case_sensitive: bool) -> bool {
    let Some(input) = input else {
        return false;
    };
    if case_sensitive {
        expected == input
    } else {
        expected.eq_ignore_ascii_case(input)
    }
}

/// Collapse whitespace runs to single spaces (`UnifyWhiteSpaces`).
#[must_use]
pub fn unify_white_spaces(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut space = false;
    for c in input.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out.trim().to_owned()
}

/// One synonym entry with selection weight.
#[derive(Debug, Clone, PartialEq)]
pub struct Synonym {
    /// Text.
    pub text: String,
    /// Selection weight.
    pub weight: f32,
}

/// Synonym group: one head word plus weighted alternatives.
#[derive(Debug, Clone, PartialEq)]
pub struct SynonymGroup {
    /// Match context bits.
    pub context: u32,
    /// Entries (head first).
    pub entries: Vec<Synonym>,
}

impl SynonymGroup {
    /// Total selection weight.
    #[must_use]
    pub fn total_weight(&self) -> f32 {
        self.entries.iter().map(|entry| entry.weight).sum()
    }

    /// Pick an entry by unit draw.
    #[must_use]
    pub fn pick(&self, unit: f32) -> &str {
        let total = self.total_weight();
        if !(total > 0.0) || self.entries.is_empty() {
            return "";
        }
        let mut draw = unit * total;
        for entry in &self.entries {
            draw -= entry.weight.max(0.0);
            if draw <= 0.0 {
                return &entry.text;
            }
        }
        &self.entries.last().map_or("", |entry| entry.text.as_str())
    }
}

/// Named random message list.
#[derive(Debug, Clone, PartialEq)]
pub struct RandomChatList {
    /// List name.
    pub name: String,
    /// Messages.
    pub messages: Vec<String>,
}

/// One match template piece.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchPiece {
    /// Literal text that must appear.
    Literal(String),
    /// Variable capture slot.
    Variable(usize),
    /// Synonym group reference by head word.
    Synonym(String),
}

/// Match template: ordered pieces plus classification.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchTemplate {
    /// Match context bits.
    pub context: u32,
    /// Message type.
    pub message_type: i32,
    /// Message subtype.
    pub subtype: i32,
    /// Pieces in order.
    pub pieces: Vec<MatchPiece>,
}

/// Reply key test.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplyTest {
    /// Variable equals text.
    Equals {
        /// Variable slot.
        variable: usize,
        /// Expected text.
        text: String,
    },
    /// Variable contains text.
    Contains {
        /// Variable slot.
        variable: usize,
        /// Expected substring.
        text: String,
    },
    /// Variable is set.
    Set {
        /// Variable slot.
        variable: usize,
    },
}

/// One reply key: mode plus test.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplyKey {
    /// Key mode: any/and/not.
    pub mode: ReplyKeyMode,
    /// Test.
    pub test: ReplyTest,
}

/// Reply key combination mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKeyMode {
    /// Any key matches.
    Any,
    /// All keys match.
    And,
    /// No key matches.
    Not,
}

/// Reply chat: keys plus weighted messages.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplyChat {
    /// Keys.
    pub keys: Vec<ReplyKey>,
    /// Selection priority.
    pub priority: f32,
    /// Candidate messages.
    pub messages: Vec<String>,
}

/// Named initial-chat type with messages.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatType {
    /// Type name.
    pub name: String,
    /// Messages.
    pub messages: Vec<String>,
}

/// Initial chats keyed by type name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InitialChat {
    /// Chat types.
    pub types: Vec<ChatType>,
}

impl InitialChat {
    /// Messages for a type name.
    #[must_use]
    pub fn messages(&self, name: &str) -> &[String] {
        self.types
            .iter()
            .find(|chat| chat.name == name)
            .map_or(&[], |chat| chat.messages.as_slice())
    }
}

/// Parsed chat file.
#[derive(Debug, Clone, Default)]
pub struct ChatData {
    /// Synonym groups.
    pub synonyms: Vec<SynonymGroup>,
    /// Random lists.
    pub randoms: Vec<RandomChatList>,
    /// Match templates.
    pub matches: Vec<MatchTemplate>,
    /// Replies.
    pub replies: Vec<ReplyChat>,
    /// Initial chats.
    pub initial: InitialChat,
    /// Parse warnings.
    pub diagnostics: Vec<String>,
}

impl ChatData {
    /// Parse chat file text.
    pub fn parse(text: &str) -> Result<Self, BotsError> {
        let tokens = tokenize_bot_script(text);
        let mut data = Self::default();
        let mut parser = ChatParser {
            tokens: &tokens,
            index: 0,
        };
        while parser.index < tokens.len() {
            let keyword = parser.next()?;
            match keyword.to_ascii_lowercase().as_str() {
                "synonym" => data.synonyms.push(parser.parse_synonym()?),
                "random" => data.randoms.push(parser.parse_random()?),
                "match" => data.matches.push(parser.parse_match()?),
                "reply" => data.replies.push(parser.parse_reply()?),
                "chat" => data.parse_chat(&mut parser)?,
                _ => {
                    return Err(BotsError::BotScript(format!("unknown chat block '{keyword}'")));
                }
            }
        }
        Ok(data)
    }

    /// Find a synonym group by head word.
    #[must_use]
    pub fn synonym(&self, head: &str) -> Option<&SynonymGroup> {
        self.synonyms.iter().find(|group| {
            group
                .entries
                .first()
                .is_some_and(|entry| entry.text.eq_ignore_ascii_case(head))
        })
    }

    /// Find a random list by name.
    #[must_use]
    pub fn random(&self, name: &str) -> Option<&RandomChatList> {
        self.randoms.iter().find(|list| list.name.eq_ignore_ascii_case(name))
    }

    /// Configuration counts.
    #[must_use]
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        (
            self.synonyms.len(),
            self.randoms.len(),
            self.matches.len(),
            self.replies.len(),
        )
    }
}

struct ChatParser<'a> {
    tokens: &'a [String],
    index: usize,
}

impl ChatParser<'_> {
    fn next(&mut self) -> Result<String, BotsError> {
        let token = self
            .tokens
            .get(self.index)
            .cloned()
            .ok_or_else(|| BotsError::BotScript("unexpected end of chat file".to_owned()))?;
        self.index += 1;
        Ok(token)
    }

    fn expect(&mut self, text: &str) -> Result<(), BotsError> {
        let token = self.next()?;
        if !token.eq_ignore_ascii_case(text) {
            return Err(BotsError::BotScript(format!("expected '{text}', found '{token}'")));
        }
        Ok(())
    }

    fn parse_synonym(&mut self) -> Result<SynonymGroup, BotsError> {
        self.expect("{")?;
        let mut entries = Vec::new();
        let mut context = 0u32;
        while self.tokens.get(self.index).is_some_and(|token| token != "}") {
            let key = self.next()?;
            if key.eq_ignore_ascii_case("context") {
                context = self.next()?.parse::<u32>().unwrap_or(0);
                continue;
            }
            let weight = if self
                .tokens
                .get(self.index)
                .is_some_and(|token| token.parse::<f32>().is_ok())
            {
                self.next()?.parse::<f32>().unwrap_or(1.0)
            } else {
                1.0
            };
            entries.push(Synonym { text: key, weight });
        }
        self.expect("}")?;
        if entries.is_empty() {
            return Err(BotsError::BotScript("synonym block needs entries".to_owned()));
        }
        Ok(SynonymGroup { context, entries })
    }

    fn parse_random(&mut self) -> Result<RandomChatList, BotsError> {
        let name = self.next()?;
        self.expect("{")?;
        let mut messages = Vec::new();
        while self.tokens.get(self.index).is_some_and(|token| token != "}") {
            messages.push(self.next()?);
        }
        self.expect("}")?;
        Ok(RandomChatList { name, messages })
    }

    fn parse_match(&mut self) -> Result<MatchTemplate, BotsError> {
        self.expect("{")?;
        let mut template = MatchTemplate {
            context: 0,
            message_type: 0,
            subtype: 0,
            pieces: Vec::new(),
        };
        while self.tokens.get(self.index).is_some_and(|token| token != "}") {
            let key = self.next()?;
            match key.to_ascii_lowercase().as_str() {
                "context" => template.context = self.next()?.parse::<u32>().unwrap_or(0),
                "type" => template.message_type = self.next()?.parse::<i32>().unwrap_or(0),
                "subtype" => template.subtype = self.next()?.parse::<i32>().unwrap_or(0),
                "string" => {
                    let text = self.next()?;
                    template.pieces.push(piece_for(&text));
                }
                "variable" => {
                    let slot = self.next()?.parse::<usize>().unwrap_or(0).min(CHAT_VARIABLE_COUNT - 1);
                    template.pieces.push(MatchPiece::Variable(slot));
                }
                "synonym" => {
                    let head = self.next()?;
                    template.pieces.push(MatchPiece::Synonym(head));
                }
                _ => {
                    template.pieces.push(piece_for(&key));
                }
            }
        }
        self.expect("}")?;
        Ok(template)
    }

    fn parse_reply(&mut self) -> Result<ReplyChat, BotsError> {
        self.expect("{")?;
        let mut reply = ReplyChat {
            keys: Vec::new(),
            priority: 0.0,
            messages: Vec::new(),
        };
        while self.tokens.get(self.index).is_some_and(|token| token != "}") {
            let key = self.next()?;
            match key.to_ascii_lowercase().as_str() {
                "priority" => reply.priority = self.next()?.parse::<f32>().unwrap_or(0.0),
                "message" => reply.messages.push(self.next()?),
                "key" => {
                    let mode = self.next()?;
                    let mode = match mode.to_ascii_lowercase().as_str() {
                        "any" => ReplyKeyMode::Any,
                        "not" => ReplyKeyMode::Not,
                        _ => ReplyKeyMode::And,
                    };
                    let test_kind = self.next()?;
                    let variable = self.next()?.parse::<usize>().unwrap_or(0).min(CHAT_VARIABLE_COUNT - 1);
                    let text = if test_kind.eq_ignore_ascii_case("set") {
                        String::new()
                    } else {
                        self.next()?
                    };
                    let test = if test_kind.eq_ignore_ascii_case("contains") {
                        ReplyTest::Contains { variable, text }
                    } else if test_kind.eq_ignore_ascii_case("set") {
                        ReplyTest::Set { variable }
                    } else {
                        ReplyTest::Equals { variable, text }
                    };
                    reply.keys.push(ReplyKey { mode, test });
                }
                _ => reply.messages.push(key),
            }
        }
        self.expect("}")?;
        Ok(reply)
    }
}

fn piece_for(text: &str) -> MatchPiece {
    if text.len() == 1 && text.chars().all(|c| c.is_ascii_digit()) {
        MatchPiece::Variable(text.parse::<usize>().unwrap_or(0).min(CHAT_VARIABLE_COUNT - 1))
    } else {
        MatchPiece::Literal(text.to_owned())
    }
}

impl ChatData {
    fn parse_chat(&mut self, parser: &mut ChatParser<'_>) -> Result<(), BotsError> {
        let name = parser.next()?;
        parser.expect("{")?;
        let mut messages = Vec::new();
        while parser.tokens.get(parser.index).is_some_and(|token| token != "}") {
            messages.push(parser.next()?);
        }
        parser.expect("}")?;
        self.initial.types.push(ChatType { name, messages });
        Ok(())
    }
}
