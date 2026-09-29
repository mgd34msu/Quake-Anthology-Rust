//! Minimal JSON documents for settings files.
//!
//! The workspace keeps zero runtime dependencies, so seat settings, gyro
//! profiles, input routing, and server profiles parse and render through
//! this small value model instead of `serde_json`. It supports the JSON
//! the settings documents need (objects, arrays, strings with escapes,
//! finite numbers, booleans, null) and rejects trailing content.

use super::SettingsError;

/// Maximum parser nesting depth.
pub const MAX_DEPTH: usize = 64;

/// A JSON document value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Number (`f64`; settings documents use finite values).
    Number(f64),
    /// String.
    String(String),
    /// Array.
    Array(Vec<Json>),
    /// Object (insertion order preserved).
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Look up an object member.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Object(members) => members.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    /// Object member or null when absent (never for non-objects).
    #[must_use]
    pub fn get_or_null(&self, key: &str) -> &Json {
        self.get(key).unwrap_or(&Json::Null)
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    offset: usize,
}

/// Parse a whole JSON document.
pub fn parse_json(text: &str) -> Result<Json, SettingsError> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        offset: 0,
    };
    parser.skip_whitespace();
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.offset != parser.bytes.len() {
        return Err(SettingsError::BadValue("Unexpected trailing settings text".to_string()));
    }
    Ok(value)
}

impl Parser<'_> {
    fn skip_whitespace(&mut self) {
        while self.offset < self.bytes.len() && matches!(self.bytes[self.offset], b' ' | b'\t' | b'\n' | b'\r') {
            self.offset += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn expect(&mut self, byte: u8, what: &str) -> Result<(), SettingsError> {
        if self.peek() == Some(byte) {
            self.offset += 1;
            Ok(())
        } else {
            Err(SettingsError::BadValue(format!("Expected {what} in settings document")))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, SettingsError> {
        if depth > MAX_DEPTH {
            return Err(SettingsError::BadValue(
                "Settings document is too deeply nested".to_string(),
            ));
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(byte) if byte == b'-' || byte.is_ascii_digit() => self.number(),
            _ => Err(SettingsError::BadValue("Unexpected settings text".to_string())),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, SettingsError> {
        if self.bytes[self.offset..].starts_with(word.as_bytes()) {
            self.offset += word.len();
            Ok(value)
        } else {
            Err(SettingsError::BadValue("Unexpected settings text".to_string()))
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, SettingsError> {
        self.expect(b'{', "an object")?;
        let mut members = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.offset += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(SettingsError::BadValue("Expected a settings object key".to_string()));
            }
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':', "a colon")?;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            members.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.offset += 1;
                }
                Some(b'}') => {
                    self.offset += 1;
                    return Ok(Json::Object(members));
                }
                _ => return Err(SettingsError::BadValue("Expected a comma".to_string())),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, SettingsError> {
        self.expect(b'[', "an array")?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.offset += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.offset += 1;
                }
                Some(b']') => {
                    self.offset += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(SettingsError::BadValue("Expected a comma".to_string())),
            }
        }
    }

    fn string(&mut self) -> Result<String, SettingsError> {
        self.expect(b'"', "a string")?;
        let mut text = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(SettingsError::BadValue("Unterminated settings string".to_string()));
            };
            self.offset += 1;
            match byte {
                b'"' => return Ok(text),
                b'\\' => {
                    let Some(escape) = self.peek() else {
                        return Err(SettingsError::BadValue("Unterminated settings string".to_string()));
                    };
                    self.offset += 1;
                    match escape {
                        b'"' => text.push('"'),
                        b'\\' => text.push('\\'),
                        b'/' => text.push('/'),
                        b'b' => text.push('\u{8}'),
                        b'f' => text.push('\u{c}'),
                        b'n' => text.push('\n'),
                        b'r' => text.push('\r'),
                        b't' => text.push('\t'),
                        b'u' => {
                            if self.offset + 4 > self.bytes.len() {
                                return Err(SettingsError::BadValue("Invalid settings escape".to_string()));
                            }
                            let digits = &self.bytes[self.offset..self.offset + 4];
                            let digits = std::str::from_utf8(digits)
                                .map_err(|_| SettingsError::BadValue("Invalid settings escape".to_string()))?;
                            let code = u32::from_str_radix(digits, 16)
                                .map_err(|_| SettingsError::BadValue("Invalid settings escape".to_string()))?;
                            self.offset += 4;
                            text.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(SettingsError::BadValue("Invalid settings escape".to_string())),
                    }
                }
                0x00..=0x1f => return Err(SettingsError::BadValue("Invalid settings string".to_string())),
                _ => {
                    let start = self.offset - 1;
                    let rest = &self.bytes[start..];
                    let character = std::str::from_utf8(rest)
                        .map_err(|_| SettingsError::BadValue("Invalid settings string".to_string()))?
                        .chars()
                        .next()
                        .ok_or_else(|| SettingsError::BadValue("Invalid settings string".to_string()))?;
                    text.push(character);
                    self.offset = start + character.len_utf8();
                }
            }
        }
    }

    fn number(&mut self) -> Result<Json, SettingsError> {
        let start = self.offset;
        if self.peek() == Some(b'-') {
            self.offset += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.offset += 1;
            }
            Some(byte) if byte.is_ascii_digit() => {
                while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                    self.offset += 1;
                }
            }
            _ => return Err(SettingsError::BadValue("Invalid settings number".to_string())),
        }
        if self.peek() == Some(b'.') {
            self.offset += 1;
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return Err(SettingsError::BadValue("Invalid settings number".to_string()));
            }
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.offset += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.offset += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return Err(SettingsError::BadValue("Invalid settings number".to_string()));
            }
            while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                self.offset += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.offset])
            .map_err(|_| SettingsError::BadValue("Invalid settings number".to_string()))?;
        let value: f64 = text
            .parse()
            .map_err(|_| SettingsError::BadValue("Invalid settings number".to_string()))?;
        if !value.is_finite() {
            return Err(SettingsError::BadValue("Invalid settings number".to_string()));
        }
        Ok(Json::Number(value))
    }
}

fn escape_into(text: &str, out: &mut String) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            character if (character as u32) < 32 => {
                out.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => out.push(character),
        }
    }
    out.push('"');
}

fn render_number(value: f64, out: &mut String) {
    if value == value.trunc() && value.abs() < 1e15 {
        #[allow(clippy::cast_possible_truncation)]
        out.push_str(&format!("{}", value as i64));
    } else {
        out.push_str(&format!("{value}"));
    }
}

/// Render compact JSON.
#[must_use]
pub fn stringify(value: &Json) -> String {
    let mut out = String::new();
    render(value, &mut out, 0, false);
    out
}

/// Render two-space-indented JSON (donor `JSON.stringify(value, null, 2)`).
#[must_use]
pub fn stringify_pretty(value: &Json) -> String {
    let mut out = String::new();
    render(value, &mut out, 0, true);
    out
}

fn render(value: &Json, out: &mut String, indent: usize, pretty: bool) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Number(value) => render_number(*value, out),
        Json::String(text) => escape_into(text, out),
        Json::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if pretty {
                    out.push('\n');
                    out.push_str(&"  ".repeat(indent + 1));
                }
                render(item, out, indent + 1, pretty);
                if index + 1 < items.len() {
                    out.push(',');
                }
            }
            if pretty {
                out.push('\n');
                out.push_str(&"  ".repeat(indent));
            }
            out.push(']');
        }
        Json::Object(members) => {
            if members.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (index, (key, item)) in members.iter().enumerate() {
                if pretty {
                    out.push('\n');
                    out.push_str(&"  ".repeat(indent + 1));
                }
                escape_into(key, out);
                out.push(':');
                if pretty {
                    out.push(' ');
                }
                render(item, out, indent + 1, pretty);
                if index + 1 < members.len() {
                    out.push(',');
                }
            }
            if pretty {
                out.push('\n');
                out.push_str(&"  ".repeat(indent));
            }
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_documents() {
        let text = "{\"a\":[1,-2.5,true,null],\"b\":{\"c\":\"x\\ny\\u00e9\"}}";
        let value = parse_json(text).unwrap();
        assert_eq!(
            stringify(&value),
            "{\"a\":[1,-2.5,true,null],\"b\":{\"c\":\"x\\ny\u{e9}\"}}"
        );
        let pretty = stringify_pretty(&value);
        assert_eq!(parse_json(&pretty).unwrap(), value);
    }

    #[test]
    fn rejects_malformed_documents() {
        assert!(parse_json("{\"a\":}").is_err());
        assert!(parse_json("[1,]").is_err());
        assert!(parse_json("1 2").is_err());
        assert!(parse_json("\"unterminated").is_err());
        assert!(parse_json("1e999").is_err());
        let deep = "[".repeat(80);
        assert!(parse_json(&deep).is_err());
    }
}
