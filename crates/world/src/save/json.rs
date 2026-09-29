//! Lossless source-save JSON ported from `src/persistence/source-json.ts`.
//!
//! Quake II rerelease (`g_save.cpp`, jsoncpp) and TypeScript-donor saves
//! need the exact numeric literal text, including `NaN`/`Infinity`
//! spellings that strict JSON rejects. [`SaveNumber`] retains the literal;
//! [`SourceJson`] is the lossless value model; [`parse_source_json`] and
//! [`write_source_json`] round-trip it.

use crate::WorldError;

fn save_error(path: &str, message: &str) -> WorldError {
    WorldError::BadSave(format!("{path}: {message}"))
}

/// A preserved numeric literal from a source save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveNumber {
    text: String,
}

impl SaveNumber {
    /// Validate a numeric literal (`NaN`/`-Infinity` spellings included).
    pub fn new(text: &str) -> Result<Self, WorldError> {
        if !valid_number_text(text) {
            return Err(save_error("json", "invalid source number"));
        }
        Ok(Self { text: text.to_string() })
    }

    /// Build a literal from a runtime value (`-0` renders as `-0.0`).
    #[must_use]
    pub fn from_f64(value: f64) -> Self {
        Self {
            text: if value == 0.0 && value.is_sign_negative() {
                "-0.0".to_string()
            } else {
                format!("{value:?}")
            },
        }
    }

    /// Build a literal from an integer.
    #[must_use]
    pub fn from_i64(value: i64) -> Self {
        Self {
            text: format!("{value}"),
        }
    }

    /// Literal text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Literal as `f64` (`NaN`/`Infinity` spellings included).
    #[must_use]
    pub fn number(&self) -> f64 {
        match self.text.as_str() {
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            _ => self.text.parse().unwrap_or(f64::NAN),
        }
    }

    /// Literal as an integer.
    pub fn bigint(&self) -> Result<i128, WorldError> {
        self.text
            .parse::<i128>()
            .map_err(|_| save_error("json", "invalid source integer"))
    }
}

fn valid_number_text(text: &str) -> bool {
    if text == "NaN" || text == "Infinity" || text == "-Infinity" {
        return true;
    }
    let bytes = text.as_bytes();
    let mut offset = 0;
    if bytes.first() == Some(&b'-') {
        offset += 1;
    }
    if offset >= bytes.len() {
        return false;
    }
    if bytes[offset] == b'0' {
        offset += 1;
    } else if bytes[offset].is_ascii_digit() && bytes[offset] != b'0' {
        while offset < bytes.len() && bytes[offset].is_ascii_digit() {
            offset += 1;
        }
    } else {
        return false;
    }
    if bytes.get(offset) == Some(&b'.') {
        offset += 1;
        let start = offset;
        while offset < bytes.len() && bytes[offset].is_ascii_digit() {
            offset += 1;
        }
        if offset == start {
            return false;
        }
    }
    if bytes.get(offset) == Some(&b'e') || bytes.get(offset) == Some(&b'E') {
        offset += 1;
        if bytes.get(offset) == Some(&b'+') || bytes.get(offset) == Some(&b'-') {
            offset += 1;
        }
        let start = offset;
        while offset < bytes.len() && bytes[offset].is_ascii_digit() {
            offset += 1;
        }
        if offset == start {
            return false;
        }
    }
    offset == bytes.len()
}

/// Lossless source-save JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceJson {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// String.
    String(String),
    /// Preserved numeric literal.
    Number(SaveNumber),
    /// Array.
    Array(Vec<SourceJson>),
    /// Object (insertion order; duplicate keys resolve last-wins like the donor).
    Object(Vec<(String, SourceJson)>),
}

impl SourceJson {
    /// Look up an object member.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&SourceJson> {
        match self {
            Self::Object(members) => members
                .iter()
                .rev()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// Look up a mutable object member.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut SourceJson> {
        match self {
            Self::Object(members) => members
                .iter_mut()
                .rev()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// Insert or replace an object member.
    pub fn set(&mut self, key: &str, value: SourceJson) -> Result<(), WorldError> {
        match self {
            Self::Object(members) => {
                if let Some(slot) = members.iter_mut().find(|(name, _)| name == key) {
                    slot.1 = value;
                } else {
                    members.push((key.to_string(), value));
                }
                Ok(())
            }
            _ => Err(save_error(key, "expected an object")),
        }
    }

    /// Object member names in order.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        match self {
            Self::Object(members) => members.iter().map(|(name, _)| name.clone()).collect(),
            _ => Vec::new(),
        }
    }
}

struct Parser<'a> {
    text: &'a [u8],
    offset: usize,
}

impl Parser<'_> {
    fn fail(&self, message: &str) -> WorldError {
        save_error(&format!("json:{}", self.offset), message)
    }

    fn whitespace(&mut self) {
        while self.offset < self.text.len() && self.text[self.offset].is_ascii_whitespace() {
            self.offset += 1;
        }
    }

    fn string(&mut self) -> Result<String, WorldError> {
        if self.text.get(self.offset) != Some(&b'"') {
            return Err(self.fail("expected a string"));
        }
        let start = self.offset;
        self.offset += 1;
        loop {
            let Some(byte) = self.text.get(self.offset).copied() else {
                return Err(self.fail("unterminated string"));
            };
            self.offset += 1;
            if byte == b'\\' {
                if self.offset >= self.text.len() {
                    return Err(self.fail("unterminated string"));
                }
                self.offset += 1;
            } else if byte == b'"' {
                break;
            }
        }
        let raw = std::str::from_utf8(&self.text[start..self.offset]).map_err(|_| self.fail("invalid string"))?;
        unescape_json_string(raw).ok_or_else(|| self.fail("invalid string"))
    }

    fn value(&mut self) -> Result<SourceJson, WorldError> {
        self.whitespace();
        let Some(byte) = self.text.get(self.offset).copied() else {
            return Err(self.fail("expected a value"));
        };
        if byte == b'"' {
            return Ok(SourceJson::String(self.string()?));
        }
        if byte == b'{' || byte == b'[' {
            self.offset += 1;
            self.whitespace();
            let end = if byte == b'{' { b'}' } else { b']' };
            let mut object: Vec<(String, SourceJson)> = Vec::new();
            let mut array: Vec<SourceJson> = Vec::new();
            if self.text.get(self.offset) != Some(&end) {
                loop {
                    if byte == b'{' {
                        self.whitespace();
                        let key = self.string()?;
                        self.whitespace();
                        if self.text.get(self.offset) != Some(&b':') {
                            return Err(self.fail("expected ':'"));
                        }
                        self.offset += 1;
                        let value = self.value()?;
                        if let Some(slot) = object.iter_mut().find(|(name, _)| *name == key) {
                            slot.1 = value;
                        } else {
                            object.push((key, value));
                        }
                    } else {
                        array.push(self.value()?);
                    }
                    self.whitespace();
                    if self.text.get(self.offset) != Some(&b',') {
                        break;
                    }
                    self.offset += 1;
                    self.whitespace();
                }
            }
            if self.text.get(self.offset) != Some(&end) {
                return Err(self.fail(&format!("expected '{}'", end as char)));
            }
            self.offset += 1;
            return Ok(if byte == b'{' {
                SourceJson::Object(object)
            } else {
                SourceJson::Array(array)
            });
        }
        for (literal, value) in [
            ("null", SourceJson::Null),
            ("true", SourceJson::Bool(true)),
            ("false", SourceJson::Bool(false)),
        ] {
            if self.text[self.offset..].starts_with(literal.as_bytes()) {
                self.offset += literal.len();
                return Ok(value);
            }
        }
        let rest = &self.text[self.offset..];
        let length = number_prefix_length(rest);
        if length == 0 {
            return Err(self.fail("expected a value"));
        }
        let text = std::str::from_utf8(&rest[..length]).map_err(|_| self.fail("expected a value"))?;
        self.offset += length;
        Ok(SourceJson::Number(
            SaveNumber::new(text).map_err(|_| self.fail("invalid source number"))?,
        ))
    }
}

fn number_prefix_length(rest: &[u8]) -> usize {
    for literal in ["NaN", "-Infinity", "Infinity"] {
        if rest.starts_with(literal.as_bytes()) {
            return literal.len();
        }
    }
    let mut offset = 0;
    if rest.first() == Some(&b'-') {
        offset += 1;
    }
    if offset >= rest.len() {
        return 0;
    }
    if rest[offset] == b'0' {
        offset += 1;
    } else if rest[offset].is_ascii_digit() {
        while offset < rest.len() && rest[offset].is_ascii_digit() {
            offset += 1;
        }
    } else {
        return 0;
    }
    if rest.get(offset) == Some(&b'.') {
        let mut end = offset + 1;
        while end < rest.len() && rest[end].is_ascii_digit() {
            end += 1;
        }
        if end == offset + 1 {
            return 0;
        }
        offset = end;
    }
    if rest.get(offset) == Some(&b'e') || rest.get(offset) == Some(&b'E') {
        let mut end = offset + 1;
        if rest.get(end) == Some(&b'+') || rest.get(end) == Some(&b'-') {
            end += 1;
        }
        let start = end;
        while end < rest.len() && rest[end].is_ascii_digit() {
            end += 1;
        }
        if end == start {
            return 0;
        }
        offset = end;
    }
    offset
}

fn unescape_json_string(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'"' || bytes[bytes.len() - 1] != b'"' {
        return None;
    }
    let mut out = String::new();
    let mut offset = 1;
    while offset < bytes.len() - 1 {
        let byte = bytes[offset];
        offset += 1;
        if byte != b'\\' {
            let rest = std::str::from_utf8(&bytes[offset - 1..]).ok()?;
            let character = rest.chars().next()?;
            if (character as u32) < 32 {
                return None;
            }
            out.push(character);
            offset += character.len_utf8() - 1;
            continue;
        }
        let escape = *bytes.get(offset)?;
        offset += 1;
        match escape {
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'/' => out.push('/'),
            b'b' => out.push('\u{8}'),
            b'f' => out.push('\u{c}'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'u' => {
                let digits = std::str::from_utf8(bytes.get(offset..offset + 4)?).ok()?;
                let code = u32::from_str_radix(digits, 16).ok()?;
                offset += 4;
                out.push(char::from_u32(code)?);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Parse a source save document.
pub fn parse_source_json(text: &str) -> Result<SourceJson, WorldError> {
    let mut parser = Parser {
        text: text.as_bytes(),
        offset: 0,
    };
    let root = parser.value()?;
    parser.whitespace();
    if parser.offset != parser.text.len() {
        return Err(parser.fail("trailing save data"));
    }
    Ok(root)
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

fn render(value: &SourceJson, out: &mut String) {
    match value {
        SourceJson::Null => out.push_str("null"),
        SourceJson::Bool(true) => out.push_str("true"),
        SourceJson::Bool(false) => out.push_str("false"),
        SourceJson::String(text) => escape_into(text, out),
        SourceJson::Number(number) => out.push_str(number.text()),
        SourceJson::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                render(item, out);
            }
            out.push(']');
        }
        SourceJson::Object(members) => {
            out.push('{');
            for (index, (key, item)) in members.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                escape_into(key, out);
                out.push(':');
                render(item, out);
            }
            out.push('}');
        }
    }
}

/// Render compact source JSON (numeric literals keep their text).
#[must_use]
pub fn write_source_json(value: &SourceJson) -> String {
    let mut out = String::new();
    render(value, &mut out);
    out
}

/// Require an object at a path.
pub fn source_object<'a>(value: Option<&'a SourceJson>, path: &str) -> Result<&'a SourceJson, WorldError> {
    match value {
        Some(SourceJson::Object(_)) => Ok(value.expect("checked object")),
        _ => Err(save_error(path, "expected an object")),
    }
}

/// Read a numeric literal, defaulting when absent.
pub fn source_number(value: Option<&SourceJson>, path: &str, fallback: f64) -> Result<f64, WorldError> {
    match value {
        None => Ok(fallback),
        Some(SourceJson::Number(number)) => Ok(number.number()),
        _ => Err(save_error(path, "expected a numeric literal")),
    }
}

/// Read a boolean literal, defaulting to false when absent.
pub fn source_bool(value: Option<&SourceJson>, path: &str) -> Result<bool, WorldError> {
    match value {
        None => Ok(false),
        Some(SourceJson::Bool(value)) => Ok(*value),
        _ => Err(save_error(path, "expected boolean")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lossless_numbers_round_trip() {
        let text = r#"{"a":1,"b":-0.0,"c":1e3,"d":NaN,"e":-Infinity,"f":[true,null,"x"]}"#;
        let parsed = parse_source_json(text).unwrap();
        assert_eq!(write_source_json(&parsed), text);
        assert_eq!(
            parsed.get("a").unwrap(),
            &SourceJson::Number(SaveNumber::new("1").unwrap())
        );
        assert!(parsed.get("d").unwrap().get("x").is_none());
    }

    #[test]
    fn number_validation_matches_donor() {
        assert!(SaveNumber::new("01").is_err());
        assert!(SaveNumber::new("1.").is_err());
        assert!(SaveNumber::new("Infinityx").is_err());
        assert_eq!(SaveNumber::from_f64(-0.0).text(), "-0.0");
        assert_eq!(SaveNumber::from_i64(42).text(), "42");
        assert_eq!(SaveNumber::new("42").unwrap().bigint().unwrap(), 42);
        assert!(SaveNumber::new("4.5").unwrap().bigint().is_err());
    }

    #[test]
    fn malformed_documents_fail() {
        assert!(parse_source_json(r#"{"a":}"#).is_err());
        assert!(parse_source_json("[1,]").is_err());
        assert!(parse_source_json("1 2").is_err());
        assert!(parse_source_json("\"unterminated").is_err());
        assert!(parse_source_json("{,}").is_err());
    }

    #[test]
    fn object_helpers_read_with_fallbacks() {
        let parsed = parse_source_json(r#"{"n":2.5,"b":true,"o":{"x":1}}"#).unwrap();
        assert!(source_object(parsed.get("o"), "o").is_ok());
        assert!(source_object(parsed.get("n"), "n").is_err());
        assert_eq!(source_number(parsed.get("n"), "n", 0.0).unwrap(), 2.5);
        assert_eq!(source_number(parsed.get("missing"), "missing", 7.0).unwrap(), 7.0);
        assert!(source_number(parsed.get("b"), "b", 0.0).is_err());
        assert!(source_bool(parsed.get("b"), "b").unwrap());
        assert!(!source_bool(parsed.get("missing"), "missing").unwrap());
    }
}
