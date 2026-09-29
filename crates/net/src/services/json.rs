//! Minimal JSON save parser shared by service restores.
//!
//! Parses donor `JSON.stringify` saves into
//! [`Json`](crate::common::session::Json). Encoding reuses
//! [`canonical`](crate::common::session::canonical), which emits valid JSON
//! with sorted keys.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::common::session::Json;

/// Error for malformed JSON saves.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JsonError {
    /// Unexpected end of input.
    #[error("Unexpected end of JSON input")]
    UnexpectedEnd,
    /// Unexpected character at a byte offset.
    #[error("Unexpected JSON character at byte {0}")]
    UnexpectedChar(usize),
    /// Invalid string escape.
    #[error("Invalid JSON string escape")]
    BadEscape,
    /// Invalid number.
    #[error("Invalid JSON number")]
    BadNumber,
    /// Trailing bytes after the document.
    #[error("Trailing JSON bytes")]
    Trailing,
}

/// Parse a JSON document (`JSON.parse` for saves).
pub fn parse_json(text: &str) -> Result<Json, JsonError> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        offset: 0,
    };
    parser.skip_whitespace();
    let value = parser.value()?;
    parser.skip_whitespace();
    if parser.offset != parser.bytes.len() {
        return Err(JsonError::Trailing);
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl Parser<'_> {
    fn skip_whitespace(&mut self) {
        while self.offset < self.bytes.len() && self.bytes[self.offset].is_ascii_whitespace() {
            self.offset += 1;
        }
    }

    fn peek(&self) -> Result<u8, JsonError> {
        self.bytes.get(self.offset).copied().ok_or(JsonError::UnexpectedEnd)
    }

    fn expect(&mut self, byte: u8) -> Result<(), JsonError> {
        if self.peek()? != byte {
            return Err(JsonError::UnexpectedChar(self.offset));
        }
        self.offset += 1;
        Ok(())
    }

    fn value(&mut self) -> Result<Json, JsonError> {
        match self.peek()? {
            b'n' => self.literal("null", Json::Null),
            b't' => self.literal("true", Json::Bool(true)),
            b'f' => self.literal("false", Json::Bool(false)),
            b'"' => Ok(Json::String(self.string()?)),
            b'[' => self.array(),
            b'{' => self.object(),
            _ => self.number(),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, JsonError> {
        if self.bytes[self.offset..].starts_with(word.as_bytes()) {
            self.offset += word.len();
            Ok(value)
        } else {
            Err(JsonError::UnexpectedChar(self.offset))
        }
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.expect(b'"')?;
        let mut text = String::new();
        loop {
            let byte = self.peek()?;
            if byte == b'"' {
                self.offset += 1;
                return Ok(text);
            }
            if byte == b'\\' {
                self.offset += 1;
                match self.peek()? {
                    b'"' => text.push('"'),
                    b'\\' => text.push('\\'),
                    b'/' => text.push('/'),
                    b'b' => text.push('\u{0008}'),
                    b'f' => text.push('\u{000C}'),
                    b'n' => text.push('\n'),
                    b'r' => text.push('\r'),
                    b't' => text.push('\t'),
                    b'u' => {
                        self.offset += 1;
                        let mut code = self.hex4()?;
                        if (0xd800..0xdc00).contains(&code) {
                            if self.bytes.get(self.offset) == Some(&b'\\')
                                && self.bytes.get(self.offset + 1) == Some(&b'u')
                            {
                                self.offset += 2;
                                let low = self.hex4()?;
                                if (0xdc00..0xe000).contains(&low) {
                                    code = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
                                } else {
                                    return Err(JsonError::BadEscape);
                                }
                            } else {
                                return Err(JsonError::BadEscape);
                            }
                        }
                        text.push(char::from_u32(code).ok_or(JsonError::BadEscape)?);
                        continue;
                    }
                    _ => return Err(JsonError::BadEscape),
                }
                self.offset += 1;
                continue;
            }
            if byte < 0x20 {
                return Err(JsonError::BadEscape);
            }
            let rest = &self.bytes[self.offset..];
            let length = utf8_length(byte).ok_or(JsonError::UnexpectedChar(self.offset))?;
            if rest.len() < length {
                return Err(JsonError::UnexpectedEnd);
            }
            let chunk = std::str::from_utf8(&rest[..length]).map_err(|_| JsonError::UnexpectedChar(self.offset))?;
            text.push_str(chunk);
            self.offset += length;
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        if self.offset + 4 > self.bytes.len() {
            return Err(JsonError::UnexpectedEnd);
        }
        let text = std::str::from_utf8(&self.bytes[self.offset..self.offset + 4]).map_err(|_| JsonError::BadEscape)?;
        let value = u32::from_str_radix(text, 16).map_err(|_| JsonError::BadEscape)?;
        self.offset += 4;
        Ok(value)
    }

    fn array(&mut self) -> Result<Json, JsonError> {
        self.expect(b'[')?;
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.peek()? == b']' {
            self.offset += 1;
            return Ok(Json::Array(values));
        }
        loop {
            self.skip_whitespace();
            values.push(self.value()?);
            self.skip_whitespace();
            match self.peek()? {
                b',' => {
                    self.offset += 1;
                }
                b']' => {
                    self.offset += 1;
                    return Ok(Json::Array(values));
                }
                _ => return Err(JsonError::UnexpectedChar(self.offset)),
            }
        }
    }

    fn object(&mut self) -> Result<Json, JsonError> {
        self.expect(b'{')?;
        let mut fields = BTreeMap::new();
        self.skip_whitespace();
        if self.peek()? == b'}' {
            self.offset += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_whitespace();
            if self.peek()? != b'"' {
                return Err(JsonError::UnexpectedChar(self.offset));
            }
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            fields.insert(key, self.value()?);
            self.skip_whitespace();
            match self.peek()? {
                b',' => {
                    self.offset += 1;
                }
                b'}' => {
                    self.offset += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err(JsonError::UnexpectedChar(self.offset)),
            }
        }
    }

    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.offset;
        if self.peek()? == b'-' {
            self.offset += 1;
        }
        if self.peek()? == b'0' {
            self.offset += 1;
            if self.bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
                return Err(JsonError::BadNumber);
            }
        } else {
            let mut digits = 0;
            while self.offset < self.bytes.len() && self.bytes[self.offset].is_ascii_digit() {
                self.offset += 1;
                digits += 1;
            }
            if digits == 0 {
                return Err(JsonError::BadNumber);
            }
        }
        if self.bytes.get(self.offset) == Some(&b'.') {
            self.offset += 1;
            let mut fraction = 0;
            while self.offset < self.bytes.len() && self.bytes[self.offset].is_ascii_digit() {
                self.offset += 1;
                fraction += 1;
            }
            if fraction == 0 {
                return Err(JsonError::BadNumber);
            }
        }
        if matches!(self.bytes.get(self.offset), Some(b'e' | b'E')) {
            self.offset += 1;
            if matches!(self.bytes.get(self.offset), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            let mut exponent = 0;
            while self.offset < self.bytes.len() && self.bytes[self.offset].is_ascii_digit() {
                self.offset += 1;
                exponent += 1;
            }
            if exponent == 0 {
                return Err(JsonError::BadNumber);
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.offset]).map_err(|_| JsonError::BadNumber)?;
        text.parse::<f64>().map(Json::Number).map_err(|_| JsonError::BadNumber)
    }
}

fn utf8_length(first: u8) -> Option<usize> {
    if first < 0x80 {
        Some(1)
    } else if first >= 0xc2 && first < 0xe0 {
        Some(2)
    } else if first >= 0xe0 && first < 0xf0 {
        Some(3)
    } else if first >= 0xf0 && first < 0xf5 {
        Some(4)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_documents() {
        assert_eq!(parse_json("null").unwrap(), Json::Null);
        assert_eq!(
            parse_json("[1, \"a\\n\", true, null]").unwrap(),
            Json::Array(vec![
                Json::Number(1.0),
                Json::String("a\n".to_owned()),
                Json::Bool(true),
                Json::Null,
            ])
        );
        assert!(parse_json("{\"a\":01}").is_err());
        assert!(parse_json("[1,]").is_err());
    }
}
