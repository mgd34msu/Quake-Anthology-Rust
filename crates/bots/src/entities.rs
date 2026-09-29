//! Quake text tokenizer and entity parser translated from Quake III
//! Arena's `q_shared.c`/`cmd.c`, ported from
//! `src/formats/q3-map/entities.ts`. AAS reachability reads BSP entity
//! text through [`parse_entities`].

use std::collections::HashMap;

use thiserror::Error;

/// Maximum token length; tokens reaching it are rejected.
pub const TOKEN_MAX: usize = 1024;

/// Parse failure with source position.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{source_name}:{line}:{column}: {message}")]
pub struct EntityError {
    /// Source name.
    pub source_name: String,
    /// 1-based line.
    pub line: usize,
    /// 1-based column.
    pub column: usize,
    /// What went wrong.
    pub message: String,
}

/// One scanned token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Token text.
    pub value: String,
    /// 1-based line.
    pub line: usize,
    /// 1-based column.
    pub column: usize,
    /// Token was double-quoted.
    pub quoted: bool,
}

fn is_whitespace(byte: u8) -> bool {
    byte <= 32
}

/// Source tokenizer with `//` and `/* */` comment support.
#[derive(Debug, Clone)]
pub struct Tokenizer {
    source: Vec<u8>,
    name: String,
    offset: usize,
    line: usize,
    column: usize,
}

impl Tokenizer {
    /// Tokenize source text.
    #[must_use]
    pub fn new(text: &str, name: &str) -> Self {
        Self {
            source: text.as_bytes().to_vec(),
            name: name.to_string(),
            offset: 0,
            line: 1,
            column: 1,
        }
    }

    fn error(&self, message: &str, token: Option<&Token>) -> EntityError {
        let (line, column) = token.map_or((self.line, self.column), |t| (t.line, t.column));
        EntityError {
            source_name: self.name.clone(),
            line,
            column,
            message: message.to_string(),
        }
    }

    fn advance(&mut self) {
        let Some(&byte) = self.source.get(self.offset) else {
            return;
        };
        if byte == b'\r' {
            self.offset += 1;
            if self.source.get(self.offset) == Some(&b'\n') {
                self.offset += 1;
            }
            self.line += 1;
            self.column = 1;
            return;
        }
        self.offset += 1;
        if byte == b'\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
    }

    /// Scan the next token. With `allow_line_breaks` false, tokens after
    /// a crossed line break (entity values must sit on the key's line)
    /// scan as absent.
    pub fn next_token(&mut self, allow_line_breaks: bool) -> Result<Option<Token>, EntityError> {
        let mut crossed_line = false;
        loop {
            while self.offset < self.source.len() {
                let byte = self.source[self.offset];
                if !is_whitespace(byte) {
                    break;
                }
                if byte == b'\n' || byte == b'\r' {
                    crossed_line = true;
                }
                self.advance();
            }
            if self.source[self.offset..].starts_with(b"//") {
                self.advance();
                self.advance();
                while self.offset < self.source.len() {
                    let byte = self.source[self.offset];
                    if byte == b'\n' || byte == b'\r' {
                        break;
                    }
                    self.advance();
                }
                continue;
            }
            if self.source[self.offset..].starts_with(b"/*") {
                self.advance();
                self.advance();
                while self.offset < self.source.len() && !self.source[self.offset..].starts_with(b"*/") {
                    let byte = self.source[self.offset];
                    if byte == b'\n' || byte == b'\r' {
                        crossed_line = true;
                    }
                    self.advance();
                }
                if self.source[self.offset..].starts_with(b"*/") {
                    self.advance();
                    self.advance();
                }
                continue;
            }
            break;
        }
        if !allow_line_breaks && crossed_line {
            return Ok(None);
        }
        if self.offset >= self.source.len() {
            return Ok(None);
        }
        let (line, column) = (self.line, self.column);
        let quoted = self.source[self.offset] == b'"';
        let mut value = Vec::new();
        if quoted {
            self.advance();
            while self.offset < self.source.len() {
                let byte = self.source[self.offset];
                if byte == b'"' {
                    break;
                }
                value.push(byte);
                self.advance();
            }
            if self.source.get(self.offset) == Some(&b'"') {
                self.advance();
            }
        } else {
            while self.offset < self.source.len() {
                let byte = self.source[self.offset];
                if is_whitespace(byte) {
                    break;
                }
                value.push(byte);
                self.advance();
            }
        }
        if value.len() >= TOKEN_MAX {
            return Err(self.error(&format!("token is limited to {} characters", TOKEN_MAX - 1), None));
        }
        Ok(Some(Token {
            value: value.iter().map(|&b| char::from(b)).collect(),
            line,
            column,
            quoted,
        }))
    }
}

/// Parse `{ "key" "value" ... }` entity records. Keys and values keep
/// their raw byte content mapped to code points, matching Quake strings.
pub fn parse_entities(text: &str, name: &str) -> Result<Vec<HashMap<String, String>>, EntityError> {
    let mut tokenizer = Tokenizer::new(text, name);
    let mut entities = Vec::new();
    loop {
        let Some(opening) = tokenizer.next_token(true)? else {
            return Ok(entities);
        };
        if opening.value != "{" {
            return Err(tokenizer.error("expected \"{\"", Some(&opening)));
        }
        let mut entity = HashMap::new();
        loop {
            let Some(key) = tokenizer.next_token(true)? else {
                return Err(tokenizer.error("expected key or \"}\"", None));
            };
            if key.value == "}" {
                entities.push(entity);
                break;
            }
            let value = tokenizer.next_token(false)?;
            match value {
                None => {
                    return Err(tokenizer.error(&format!("missing value for entity key \"{}\"", key.value), None));
                }
                Some(value) if value.value == "}" => {
                    return Err(
                        tokenizer.error(&format!("missing value for entity key \"{}\"", key.value), Some(&value))
                    );
                }
                Some(value) => {
                    entity.insert(key.value, value.value);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_entities_with_comments() {
        let entities = parse_entities(
            "// leading\n{ \"classname\" \"worldspawn\" /* trailing */ }\n{ \"a\" \"b\" }",
            "test",
        )
        .unwrap();
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].get("classname").unwrap(), "worldspawn");
        assert_eq!(entities[1].get("a").unwrap(), "b");
    }

    #[test]
    fn rejects_missing_brace_and_value() {
        let error = parse_entities("{ \"a\" }", "test").unwrap_err();
        assert!(error.message.contains("missing value"));
        let error = parse_entities("\"a\"", "test").unwrap_err();
        assert!(error.message.contains("expected \"{\""));
        let error = parse_entities("{ \"a\"", "test").unwrap_err();
        assert!(error.message.contains("missing value"));
    }

    #[test]
    fn values_stay_on_the_key_line() {
        let error = parse_entities("{ \"a\"\n\"b\" }", "test").unwrap_err();
        assert!(error.message.contains("missing value"));
    }

    #[test]
    fn rejects_overlong_tokens() {
        let big = "x".repeat(TOKEN_MAX);
        let error = parse_entities(&format!("{{ \"a\" \"{big}\" }}"), "test").unwrap_err();
        assert!(error.message.contains("limited to"));
    }
}
