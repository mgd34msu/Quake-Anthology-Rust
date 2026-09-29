//! QVM entity-token stream (`G_GET_ENTITY_TOKEN`).
//!
//! Provenance: `src/compat/qvm/entity-tokens.ts` (Q3 `server/sv_game.c`
//! token consumption: consume before copying, retain a final nonempty token
//! at EOF).
//!
//! [`CommonParseCursor`] and [`CommonParseState`] mirror the used surface of
//! `src/core/common-parse.ts` (`COM_Parse` with the Linux/QVM signed-char
//! byte profile); save state uses the [`super::game_data`] profile-value
//! mirror of `SaveReader` instead of the persistence crate.

use super::game_data::{
    CallKind, ProfileReader, ProfileValue, QvmGameImport, QvmHostCall, QvmRole,
};
use crate::error::GuestError;

/// Maximum token storage in bytes.
const MAX_TOKEN_CHARS: usize = 1024;

/// Byte cursor over a Latin-1 source string.
#[derive(Debug, Clone)]
pub struct CommonParseCursor {
    /// Source bytes.
    pub source: Vec<u8>,
    terminator: usize,
    current_offset: Option<usize>,
}

impl CommonParseCursor {
    /// Build a cursor over Latin-1 bytes.
    #[must_use]
    pub fn new(source: Vec<u8>) -> Self {
        let terminator = source.iter().position(|byte| *byte == 0).unwrap_or(source.len());
        Self {
            source,
            terminator,
            current_offset: Some(0),
        }
    }

    /// Current offset (`None` once exhausted).
    #[must_use]
    pub fn offset(&self) -> Option<usize> {
        self.current_offset
    }

    /// Set the offset, validating it against the C byte string.
    pub fn set_offset(&mut self, value: Option<usize>) -> Result<(), GuestError> {
        if let Some(offset) = value {
            if offset > self.terminator {
                return Err(GuestError::invalid("COM_Parse cursor is outside its C byte string"));
            }
        }
        self.current_offset = value;
        Ok(())
    }

    fn signed_byte(&self, offset: usize) -> i32 {
        if offset == self.source.len() {
            return 0;
        }
        if offset > self.source.len() {
            return 0;
        }
        let byte = self.source[offset];
        if byte >= 128 {
            i32::from(byte) - 256
        } else {
            i32::from(byte)
        }
    }

    fn char_at(&self, offset: usize) -> char {
        self.source.get(offset).map(|byte| *byte as char).unwrap_or('\0')
    }
}

/// `COM_Parse` session state: shared token, line, and session name.
#[derive(Debug, Clone, Default)]
pub struct CommonParseState {
    token: String,
    line: i32,
    name: String,
}

impl CommonParseState {
    /// Current shared token.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Current line number.
    #[must_use]
    pub fn line(&self) -> i32 {
        self.line
    }

    /// Capture `{ token, line, name }` save state.
    #[must_use]
    pub fn capture_save_state(&self) -> ProfileValue {
        ProfileValue::record(vec![
            ("token", ProfileValue::Str(self.token.clone())),
            ("line", ProfileValue::Int(i64::from(self.line))),
            ("name", ProfileValue::Str(self.name.clone())),
        ])
    }

    /// Restore save state, validating source storage extents.
    pub fn restore_save_state(&mut self, value: &ProfileValue) -> Result<(), GuestError> {
        let reader = ProfileReader::new(value);
        let token = reader.field("token")?.string()?;
        let line = reader.field("line")?.integer(0)?;
        let name = reader.field("name")?.string()?;
        if token.len() >= MAX_TOKEN_CHARS || name.len() >= MAX_TOKEN_CHARS {
            return reader.fail("parser string exceeds source storage");
        }
        self.token = token;
        self.line = line as i32;
        self.name = name;
        Ok(())
    }

    /// Parse the next token (`COM_Parse` semantics).
    pub fn parse(
        &mut self,
        cursor: &mut CommonParseCursor,
        allow_line_breaks: bool,
    ) -> Result<String, GuestError> {
        let Some(mut data) = cursor.current_offset else {
            self.token.clear();
            return Ok(String::new());
        };
        self.token.clear();
        let mut has_new_lines = false;
        loop {
            loop {
                let c = cursor.signed_byte(data);
                if c > 32 {
                    break;
                }
                if c == 0 {
                    cursor.current_offset = None;
                    return Ok(String::new());
                }
                if c == 10 {
                    self.line = self.line.wrapping_add(1);
                    has_new_lines = true;
                }
                data += 1;
            }
            if has_new_lines && !allow_line_breaks {
                cursor.current_offset = Some(data);
                return Ok(String::new());
            }
            let c = cursor.signed_byte(data);
            if c == 47 && cursor.signed_byte(data + 1) == 47 {
                data += 2;
                while {
                    let c = cursor.signed_byte(data);
                    c != 0 && c != 10
                } {
                    data += 1;
                }
            } else if c == 47 && cursor.signed_byte(data + 1) == 42 {
                data += 2;
                while cursor.signed_byte(data) != 0
                    && (cursor.signed_byte(data) != 42 || cursor.signed_byte(data + 1) != 47)
                {
                    data += 1;
                }
                if cursor.signed_byte(data) != 0 {
                    data += 2;
                }
            } else {
                break;
            }
        }
        let c = cursor.signed_byte(data);
        if c == 34 {
            data += 1;
            loop {
                let c = cursor.signed_byte(data);
                data += 1;
                if c == 34 || c == 0 {
                    if self.token.len() == MAX_TOKEN_CHARS {
                        return Err(GuestError::invalid(
                            "COM_Parse quoted token terminator exceeds 1024-byte storage",
                        ));
                    }
                    cursor.current_offset = if c == 0 { None } else { Some(data) };
                    return Ok(self.token.clone());
                }
                if self.token.len() < MAX_TOKEN_CHARS {
                    self.token.push(cursor.char_at(data - 1));
                }
            }
        }
        loop {
            if self.token.len() < MAX_TOKEN_CHARS {
                self.token.push(cursor.char_at(data));
            }
            data += 1;
            let c = cursor.signed_byte(data);
            if c == 10 {
                self.line = self.line.wrapping_add(1);
            }
            if c <= 32 {
                break;
            }
        }
        if self.token.len() == MAX_TOKEN_CHARS {
            self.token.clear();
        }
        cursor.current_offset = Some(data);
        Ok(self.token.clone())
    }
}

/// Entity-token services consumed by the syscall bridge.
pub trait QvmEntityTokenServices {
    /// Consume the next token; `ended` reports cursor exhaustion.
    fn entity_token(&mut self) -> (String, bool);
}

/// Per-instance entity-token stream with engine `COM_Parse` semantics.
#[derive(Debug)]
pub struct QvmEntityTokens {
    cursor: CommonParseCursor,
    parser: CommonParseState,
}

impl QvmEntityTokens {
    /// Build a stream over `source`.
    #[must_use]
    pub fn new(source: Vec<u8>) -> Self {
        Self {
            cursor: CommonParseCursor::new(source),
            parser: CommonParseState::default(),
        }
    }

    /// Consume the next token.
    pub fn entity_token(&mut self) -> Result<(String, bool), GuestError> {
        let token = self.parser.parse(&mut self.cursor, true)?;
        Ok((token, self.cursor.current_offset.is_none()))
    }

    /// Capture `{ source, cursor, parser }` save state.
    #[must_use]
    pub fn capture_save_state(&self) -> ProfileValue {
        ProfileValue::record(vec![
            ("source", ProfileValue::Bytes(self.cursor.source.clone())),
            (
                "cursor",
                match self.cursor.current_offset {
                    Some(offset) => ProfileValue::Int(offset as i64),
                    None => ProfileValue::Null,
                },
            ),
            ("parser", self.parser.capture_save_state()),
        ])
    }

    /// Restore save state.
    pub fn restore_save_state(&mut self, value: &ProfileValue) -> Result<(), GuestError> {
        if matches!(value, ProfileValue::Undefined) && self.cursor.source.is_empty() {
            self.cursor.current_offset = Some(0);
            self.parser = CommonParseState::default();
            return Ok(());
        }
        let reader = ProfileReader::new(value);
        let source = reader.field("source")?.bytes()?;
        if source != self.cursor.source {
            return reader.field("source")?.fail("entity token source differs");
        }
        let cursor = reader.field("cursor")?.nullable(|cell| cell.integer(0))?;
        self.parser.restore_save_state(reader.field("parser")?.value())?;
        self.cursor.set_offset(cursor.map(|offset| offset as usize))?;
        Ok(())
    }
}

impl QvmEntityTokenServices for QvmEntityTokens {
    fn entity_token(&mut self) -> (String, bool) {
        self.entity_token().unwrap_or_else(|_| (String::new(), true))
    }
}

/// `G_GET_ENTITY_TOKEN` bridge; returns `None` for other traps.
pub fn qvm_entity_token_syscall(
    call: &QvmHostCall,
    services: &mut dyn QvmEntityTokenServices,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine
        || call.role != QvmRole::Qagame
        || call.code != QvmGameImport::G_GET_ENTITY_TOKEN
    {
        return Ok(None);
    }
    let (token, ended) = services.entity_token();
    call.guest
        .write_string(call.int(1)?, &token, call.int(2)? as usize)?;
    Ok(Some(i32::from(!ended || !token.is_empty())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    use std::cell::RefCell;

    fn stream(text: &str) -> QvmEntityTokens {
        QvmEntityTokens::new(text.as_bytes().to_vec())
    }

    #[test]
    fn tokens_skip_comments_and_whitespace() {
        let mut tokens = stream("{\n// comment\n\"key\" value /* block */\n}");
        let mut parsed = Vec::new();
        loop {
            let (token, ended) = tokens.entity_token().unwrap();
            parsed.push(token);
            if ended {
                break;
            }
        }
        assert_eq!(parsed, vec!["{", "key", "value", "}", ""]);
    }

    #[test]
    fn quoted_tokens_preserve_inner_spaces() {
        let mut tokens = stream("\"a b\" c");
        assert_eq!(tokens.entity_token().unwrap().0, "a b");
        assert_eq!(tokens.entity_token().unwrap().0, "c");
    }

    #[test]
    fn save_state_round_trips_mid_stream() {
        let mut tokens = stream("one two three");
        assert_eq!(tokens.entity_token().unwrap().0, "one");
        let saved = tokens.capture_save_state();
        assert_eq!(tokens.entity_token().unwrap().0, "two");
        tokens.restore_save_state(&saved).unwrap();
        assert_eq!(tokens.entity_token().unwrap().0, "two");
        let mut fresh = stream("");
        fresh.restore_save_state(&ProfileValue::Undefined).unwrap();
        assert_eq!(fresh.entity_token().unwrap(), (String::new(), true));
    }

    #[test]
    fn syscall_consumes_before_copying() {
        use super::super::game_data::{AbiProfile, QvmSharedMemory};
        let memory = QvmSharedMemory::new(256).unwrap();
        let call = QvmHostCall {
            kind: CallKind::Engine,
            role: QvmRole::Qagame,
            code: QvmGameImport::G_GET_ENTITY_TOKEN,
            words: vec![QvmGameImport::G_GET_ENTITY_TOKEN, 64, 64],
            guest: memory.clone(),
            abi_profile: AbiProfile::Modern,
            command_arguments: None,
        };
        let mut tokens = stream("hello");
        assert_eq!(qvm_entity_token_syscall(&call, &mut tokens).unwrap(), Some(1));
        assert_eq!(memory.read_string(64).unwrap(), "hello");
        assert_eq!(qvm_entity_token_syscall(&call, &mut tokens).unwrap(), Some(0));
        let other = QvmHostCall {
            code: QvmGameImport::G_TRACE,
            ..call
        };
        let calls = Rc::new(RefCell::new(0u32));
        struct Counting {
            calls: Rc<RefCell<u32>>,
        }
        impl QvmEntityTokenServices for Counting {
            fn entity_token(&mut self) -> (String, bool) {
                *self.calls.borrow_mut() += 1;
                (String::new(), true)
            }
        }
        let mut counting = Counting { calls: Rc::clone(&calls) };
        assert_eq!(qvm_entity_token_syscall(&other, &mut counting).unwrap(), None);
        assert_eq!(*calls.borrow(), 0);
    }
}
