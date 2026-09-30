//! Minimal JSON documents for tooling manifests and evidence files.
//!
//! The workspace keeps zero runtime dependencies, so tooling parses and
//! renders JSON through this small value model instead of `serde_json`.
//! Parsed numbers retain their source text (byte-stable round trips) alongside
//! the `f64` value; canonical rendering re-renders numbers the way
//! `JSON.stringify` does so fingerprints stay deterministic.

use crate::error::ToolsError;

/// Maximum parser nesting depth.
pub const MAX_DEPTH: usize = 128;

/// A JSON number: source text plus its `f64` value.
#[derive(Debug, Clone)]
pub struct Number {
    text: String,
    value: f64,
}

impl Number {
    /// Build a number from an `f64`, rendering text canonically.
    #[must_use]
    pub fn from_f64(value: f64) -> Self {
        let text = render_number(value);
        Self { text, value }
    }

    /// Build a number from raw source text and its parsed value.
    #[must_use]
    pub fn raw(text: String, value: f64) -> Self {
        Self { text, value }
    }

    /// The numeric value.
    #[must_use]
    pub fn value(&self) -> f64 {
        self.value
    }

    /// The rendered text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        self.value.to_bits() == other.value.to_bits()
    }
}

/// A JSON document value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Number.
    Number(Number),
    /// String.
    String(String),
    /// Array.
    Array(Vec<Json>),
    /// Object (insertion order preserved).
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Build a string value.
    #[must_use]
    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    /// Build an integer value.
    #[must_use]
    pub fn int(value: i64) -> Self {
        Self::Number(Number::raw(value.to_string(), value as f64))
    }

    /// Build an unsigned integer value.
    #[must_use]
    pub fn uint(value: u64) -> Self {
        Self::Number(Number::raw(value.to_string(), value as f64))
    }

    /// Build a floating-point value.
    #[must_use]
    pub fn float(value: f64) -> Self {
        Self::Number(Number::from_f64(value))
    }

    /// Build a boolean value.
    #[must_use]
    pub fn boolean(value: bool) -> Self {
        Self::Bool(value)
    }

    /// Build an array value.
    #[must_use]
    pub fn array(items: Vec<Json>) -> Self {
        Self::Array(items)
    }

    /// Build an object value from pairs.
    #[must_use]
    pub fn object(pairs: Vec<(String, Json)>) -> Self {
        Self::Object(pairs)
    }

    /// Look up an object member.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Object(members) => members.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    /// View as a string.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    /// View as an `f64`.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(number) => Some(number.value()),
            _ => None,
        }
    }

    /// View as a boolean.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// View as an array.
    #[must_use]
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// View as an object.
    #[must_use]
    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Self::Object(members) => Some(members),
            _ => None,
        }
    }

    /// Whether this is null.
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Render compact JSON.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        render_compact(self, &mut out);
        out
    }

    /// Render two-space indented JSON (`JSON.stringify(value, null, 2)` style).
    #[must_use]
    pub fn render_pretty(&self) -> String {
        let mut out = String::new();
        render_pretty(self, &mut out, 0);
        out
    }

    /// Render canonical JSON: sorted object keys, compact separators, and
    /// canonically rendered numbers. Fails on non-finite numbers.
    pub fn render_canonical(&self) -> Result<String, ToolsError> {
        let mut out = String::new();
        render_canonical(self, &mut out)?;
        Ok(out)
    }
}

/// Escape a string the way `JSON.stringify` does.
pub fn escape_string(value: &str, out: &mut String) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{09}' => out.push_str("\\t"),
            '\u{0A}' => out.push_str("\\n"),
            '\u{0C}' => out.push_str("\\f"),
            '\u{0D}' => out.push_str("\\r"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
}

/// Render an `f64` the way `JSON.stringify` renders finite numbers.
#[must_use]
pub fn render_number(value: f64) -> String {
    if value == 0.0 {
        return "0".to_owned();
    }
    if !value.is_finite() {
        return "null".to_owned();
    }
    let abs = value.abs();
    if abs >= 1e21 || abs < 1e-6 {
        return render_exponential(value);
    }
    let plain = format!("{value}");
    if plain.contains(['e', 'E']) {
        return render_exponential(value);
    }
    plain
}

fn render_exponential(value: f64) -> String {
    let raw = format!("{value:.17e}");
    let exp_pos = raw.find('e').unwrap_or(raw.len());
    let base_exponent: i32 = raw[exp_pos + 1..].parse().unwrap_or(0);
    let negative = raw.starts_with('-');
    let digits: Vec<u8> = raw[..exp_pos].bytes().filter(|byte| byte.is_ascii_digit()).collect();
    for precision in 1..=digits.len().max(17) {
        let mut kept: Vec<u8> = digits.iter().copied().take(precision).collect();
        while kept.len() < precision {
            kept.push(b'0');
        }
        let mut exponent = base_exponent;
        let next = digits.get(precision).copied().unwrap_or(b'0');
        if next >= b'5' {
            let mut index = kept.len();
            loop {
                if index == 0 {
                    kept = vec![b'1'];
                    kept.extend(std::iter::repeat_n(b'0', precision.saturating_sub(1)));
                    exponent += 1;
                    break;
                }
                index -= 1;
                if kept[index] < b'9' {
                    kept[index] += 1;
                    break;
                }
                kept[index] = b'0';
            }
        }
        while kept.len() > 1 && kept.last() == Some(&b'0') {
            kept.pop();
        }
        let mantissa: Vec<char> = kept.iter().map(|byte| *byte as char).collect();
        let candidate = render_exp_candidate(negative, &mantissa, exponent);
        if candidate.parse::<f64>().unwrap_or(f64::NAN).to_bits() == value.to_bits() {
            return candidate;
        }
    }
    let mantissa: Vec<char> = digits.iter().map(|byte| *byte as char).collect();
    render_exp_candidate(negative, &mantissa, base_exponent)
}

fn render_exp_candidate(negative: bool, mantissa: &[char], exponent: i32) -> String {
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push(mantissa[0]);
    if mantissa.len() > 1 {
        out.push('.');
        out.extend(mantissa[1..].iter());
    }
    out.push('e');
    out.push_str(&format!("{exponent:+}"));
    out
}

fn render_compact(value: &Json, out: &mut String) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Json::Number(number) => out.push_str(number.text()),
        Json::String(value) => escape_string(value, out),
        Json::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                render_compact(item, out);
            }
            out.push(']');
        }
        Json::Object(members) => {
            out.push('{');
            for (index, (key, member)) in members.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                escape_string(key, out);
                out.push(':');
                render_compact(member, out);
            }
            out.push('}');
        }
    }
}

fn render_canonical(value: &Json, out: &mut String) -> Result<(), ToolsError> {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Json::Number(number) => {
            if !number.value().is_finite() {
                return Err(ToolsError::invalid("Fingerprints require finite JSON data"));
            }
            out.push_str(&render_number(number.value()));
        }
        Json::String(value) => escape_string(value, out),
        Json::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                render_canonical(item, out)?;
            }
            out.push(']');
        }
        Json::Object(members) => {
            let mut sorted: Vec<&(String, Json)> = members.iter().collect();
            sorted.sort_by(|left, right| left.0.cmp(&right.0));
            out.push('{');
            for (index, (key, member)) in sorted.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                escape_string(key, out);
                out.push(':');
                render_canonical(member, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn render_pretty(value: &Json, out: &mut String, depth: usize) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        Json::Number(number) => out.push_str(number.text()),
        Json::String(value) => escape_string(value, out),
        Json::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                out.push_str(&"  ".repeat(depth + 1));
                render_pretty(item, out, depth + 1);
                if index + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&"  ".repeat(depth));
            out.push(']');
        }
        Json::Object(members) => {
            if members.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            for (index, (key, member)) in members.iter().enumerate() {
                out.push_str(&"  ".repeat(depth + 1));
                escape_string(key, out);
                out.push_str(": ");
                render_pretty(member, out, depth + 1);
                if index + 1 < members.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&"  ".repeat(depth));
            out.push('}');
        }
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    offset: usize,
    jsonc: bool,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str, jsonc: bool) -> Self {
        Self {
            bytes: text.as_bytes(),
            offset: 0,
            jsonc,
        }
    }

    fn error(&self, message: &str) -> ToolsError {
        ToolsError::parse(format!("Invalid JSON at byte {}: {message}", self.offset))
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn skip_whitespace(&mut self) {
        loop {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                self.offset += 1;
            }
            if !self.jsonc {
                return;
            }
            if self.bytes.get(self.offset..self.offset + 2) == Some(b"//".as_slice()) {
                while !matches!(self.peek(), None | Some(b'\n')) {
                    self.offset += 1;
                }
                continue;
            }
            if self.bytes.get(self.offset..self.offset + 2) == Some(b"/*".as_slice()) {
                self.offset += 2;
                while self.bytes.get(self.offset..self.offset + 2) != Some(b"*/".as_slice()) {
                    if self.peek().is_none() {
                        return;
                    }
                    self.offset += 1;
                }
                self.offset += 2;
                continue;
            }
            return;
        }
    }

    fn literal(&mut self, text: &str) -> Result<(), ToolsError> {
        if self.bytes.get(self.offset..self.offset + text.len()) == Some(text.as_bytes()) {
            self.offset += text.len();
            Ok(())
        } else {
            Err(self.error("unexpected literal"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, ToolsError> {
        if depth > MAX_DEPTH {
            return Err(self.error("nesting exceeds depth limit"));
        }
        self.skip_whitespace();
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => {
                self.literal("true")?;
                Ok(Json::Bool(true))
            }
            Some(b'f') => {
                self.literal("false")?;
                Ok(Json::Bool(false))
            }
            Some(b'n') => {
                self.literal("null")?;
                Ok(Json::Null)
            }
            Some(ch) if ch == b'-' || ch.is_ascii_digit() => self.number(),
            _ => Err(self.error("unexpected character")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, ToolsError> {
        self.offset += 1;
        let mut members = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.offset += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error("object keys must be strings"));
            }
            let key = self.string()?;
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err(self.error("object members require a colon"));
            }
            self.offset += 1;
            let member = self.value(depth + 1)?;
            members.push((key, member));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.offset += 1;
                    self.skip_whitespace();
                    if self.jsonc && self.peek() == Some(b'}') {
                        self.offset += 1;
                        return Ok(Json::Object(members));
                    }
                }
                Some(b'}') => {
                    self.offset += 1;
                    return Ok(Json::Object(members));
                }
                _ => return Err(self.error("object members require a comma")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, ToolsError> {
        self.offset += 1;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.offset += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.offset += 1;
                    self.skip_whitespace();
                    if self.jsonc && self.peek() == Some(b']') {
                        self.offset += 1;
                        return Ok(Json::Array(items));
                    }
                }
                Some(b']') => {
                    self.offset += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.error("array items require a comma")),
            }
        }
    }

    fn string(&mut self) -> Result<String, ToolsError> {
        debug_assert_eq!(self.peek(), Some(b'"'));
        self.offset += 1;
        let mut out = String::new();
        loop {
            let byte = self.peek().ok_or_else(|| self.error("unterminated string"))?;
            match byte {
                b'"' => {
                    self.offset += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.offset += 1;
                    let escape = self.peek().ok_or_else(|| self.error("unterminated escape"))?;
                    self.offset += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let high = self.hex4()?;
                            if (0xD800..0xDC00).contains(&high) {
                                if self.bytes.get(self.offset..self.offset + 2) == Some(b"\\u".as_slice()) {
                                    self.offset += 2;
                                    let low = self.hex4()?;
                                    if (0xDC00..0xE000).contains(&low) {
                                        let scalar = 0x1_0000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                                        out.push(
                                            char::from_u32(scalar).ok_or_else(|| self.error("invalid code point"))?,
                                        );
                                        continue;
                                    }
                                    return Err(self.error("invalid low surrogate"));
                                }
                                return Err(self.error("unpaired surrogate"));
                            }
                            if (0xDC00..0xE000).contains(&high) {
                                return Err(self.error("unpaired surrogate"));
                            }
                            out.push(char::from_u32(high).ok_or_else(|| self.error("invalid code point"))?);
                        }
                        _ => return Err(self.error("invalid escape")),
                    }
                }
                0x00..=0x1F => return Err(self.error("unescaped control character")),
                _ => {
                    if byte < 0x80 {
                        out.push(byte as char);
                        self.offset += 1;
                    } else {
                        let width = match byte {
                            0xC2..=0xDF => 2,
                            0xE0..=0xEF => 3,
                            0xF0..=0xF4 => 4,
                            _ => return Err(self.error("invalid UTF-8")),
                        };
                        let end = self.offset + width;
                        if end > self.bytes.len() {
                            return Err(self.error("invalid UTF-8"));
                        }
                        let text = std::str::from_utf8(&self.bytes[self.offset..end])
                            .map_err(|_| self.error("invalid UTF-8"))?;
                        let ch = text.chars().next().ok_or_else(|| self.error("invalid UTF-8"))?;
                        out.push(ch);
                        self.offset += width;
                    }
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, ToolsError> {
        if self.offset + 4 > self.bytes.len() {
            return Err(self.error("truncated unicode escape"));
        }
        let text = std::str::from_utf8(&self.bytes[self.offset..self.offset + 4])
            .map_err(|_| self.error("invalid unicode escape"))?;
        let value = u32::from_str_radix(text, 16).map_err(|_| self.error("invalid unicode escape"))?;
        self.offset += 4;
        Ok(value)
    }

    fn number(&mut self) -> Result<Json, ToolsError> {
        let start = self.offset;
        if self.peek() == Some(b'-') {
            self.offset += 1;
        }
        match self.peek() {
            Some(b'0') => self.offset += 1,
            Some(ch) if ch.is_ascii_digit() => {
                while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                    self.offset += 1;
                }
            }
            _ => return Err(self.error("invalid number")),
        }
        if self.peek() == Some(b'.') {
            self.offset += 1;
            if !self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                return Err(self.error("invalid number"));
            }
            while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                self.offset += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.offset += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            if !self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                return Err(self.error("invalid number"));
            }
            while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                self.offset += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.offset]).map_err(|_| self.error("invalid number"))?;
        let value: f64 = text.parse().map_err(|_| self.error("invalid number"))?;
        Ok(Json::Number(Number::raw(text.to_owned(), value)))
    }
}

/// Parse a whole JSON document, rejecting trailing content.
pub fn parse_json(text: &str) -> Result<Json, ToolsError> {
    let mut parser = Parser::new(text, false);
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.peek().is_some() {
        return Err(parser.error("trailing content"));
    }
    Ok(value)
}

/// Parse a JSONC document (comments and trailing commas allowed).
pub fn parse_jsonc(text: &str) -> Result<Json, ToolsError> {
    let mut parser = Parser::new(text, true);
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.peek().is_some() {
        return Err(parser.error("trailing content"));
    }
    Ok(value)
}

/// Order-insensitive deep equality over JSON data (donor `isDeepStrictEqual`
/// / `toEqual` semantics as used by the reference captures: object key order
/// is ignored, arrays stay ordered, and numbers compare by bit pattern, so
/// `-0` differs from `0` while identical NaN bit patterns compare equal).
#[must_use]
pub fn deep_strict_equal(left: &Json, right: &Json) -> bool {
    match (left, right) {
        (Json::Null, Json::Null) => true,
        (Json::Bool(a), Json::Bool(b)) => a == b,
        (Json::Number(a), Json::Number(b)) => a == b,
        (Json::String(a), Json::String(b)) => a == b,
        (Json::Array(a), Json::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(one, other)| deep_strict_equal(one, other))
        }
        (Json::Object(a), Json::Object(b)) => {
            a.len() == b.len()
                && a.iter().all(|(key, value)| {
                    b.iter()
                        .find(|(other, _)| other == key)
                        .is_some_and(|(_, other)| deep_strict_equal(value, other))
                })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_equality_ignores_object_key_order() {
        let left = parse_json(r#"{"b": [1, {"x": true}], "a": -0.0}"#).unwrap();
        let right = parse_json(r#"{"a": -0.0, "b": [1, {"x": true}]}"#).unwrap();
        assert!(deep_strict_equal(&left, &right));
        let reordered_array = parse_json(r#"{"a": -0.0, "b": [{"x": true}, 1]}"#).unwrap();
        assert!(!deep_strict_equal(&left, &reordered_array));
        let positive_zero = parse_json(r#"{"a": 0.0, "b": [1, {"x": true}]}"#).unwrap();
        assert!(!deep_strict_equal(&left, &positive_zero));
        assert!(!deep_strict_equal(&left, &Json::Null));
    }

    #[test]
    fn parses_multibyte_strings() {
        let value = parse_json("{\"a\":\"héllo→\\u00e9\",\"b\":\"a😀\",\"c\":\"😀\"}").unwrap();
        assert_eq!(value.get("a").unwrap().as_str(), Some("héllo→é"));
        assert_eq!(value.get("b").unwrap().as_str(), Some("a😀"));
        assert_eq!(value.get("c").unwrap().as_str(), Some("😀"));
        assert!(parse_json("{\"a\":\"\u{0}escaped\"}").is_err());
    }

    #[test]
    fn round_trip_preserves_number_text() {
        let value = parse_json("{\"a\":1.50,\"b\":-0.0,\"c\":1e3}").unwrap();
        assert_eq!(value.render(), "{\"a\":1.50,\"b\":-0.0,\"c\":1e3}");
    }

    #[test]
    fn pretty_matches_stringify_spacing() {
        let value = parse_json("{\"a\":[1,{\"b\":null}],\"c\":{}}").unwrap();
        assert_eq!(
            value.render_pretty(),
            "{\n  \"a\": [\n    1,\n    {\n      \"b\": null\n    }\n  ],\n  \"c\": {}\n}"
        );
    }

    #[test]
    fn canonical_sorts_keys_and_numbers() {
        let value = parse_json("{\"b\":1.50,\"a\":[3,2]}").unwrap();
        assert_eq!(value.render_canonical().unwrap(), "{\"a\":[3,2],\"b\":1.5}");
    }

    #[test]
    fn canonical_rejects_non_finite() {
        let value = Json::float(f64::INFINITY);
        assert!(value.render_canonical().is_err());
    }

    #[test]
    fn string_escapes_match() {
        let value = Json::string("a\"b\\c\nd\u{1}e\u{7f}f");
        assert_eq!(value.render(), "\"a\\\"b\\\\c\\nd\\u0001e\u{7f}f\"");
    }

    #[test]
    fn surrogate_pairs_decode() {
        let value = parse_json("\"\\uD83D\\uDE00\"").unwrap();
        assert_eq!(value, Json::string("\u{1F600}"));
        assert!(parse_json("\"\\uD83D\"").is_err());
        assert!(parse_json("\"\\uDE00\"").is_err());
    }

    #[test]
    fn trailing_content_rejected() {
        assert!(parse_json("{} {}").is_err());
        assert!(parse_json("1,").is_err());
    }

    #[test]
    fn jsonc_accepts_comments_and_trailing_commas() {
        let value = parse_jsonc("{\n// line\n\"a\": 1, /* block */\n\"b\": [2,],\n}").unwrap();
        assert_eq!(value.render(), "{\"a\":1,\"b\":[2]}");
        assert!(parse_json("{\"a\":1,}").is_err());
    }

    #[test]
    fn number_rendering_matches_common_cases() {
        assert_eq!(render_number(0.30000000000000004), "0.30000000000000004");
        assert_eq!(render_number(-0.0), "0");
        assert_eq!(render_number(1e21), "1e+21");
        assert_eq!(render_number(1e-7), "1e-7");
        assert_eq!(render_number(0.10000000149011612), "0.10000000149011612");
        assert_eq!(render_number(123.0), "123");
    }
}
