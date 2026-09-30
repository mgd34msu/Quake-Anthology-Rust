//! Checkpoint value readers ported from `src/persistence/value.ts` and
//! the `readDigest`/`readVector` helpers in `src/persistence/shared.ts`.
//!
//! This mirrors the read half of `qa-world`'s save envelope (`SaveJson`,
//! [`SaveReader`], [`namespaced`]); `qa-content` cannot depend on
//! `qa-world`, and content declaration readers only consume values, so
//! the checkpoint encode/decode codec and writers stay out.

use qa_core::math::Vec3;
use thiserror::Error;

use crate::contract::{is_content_digest, ContentDigest};

/// Checkpoint format failure (donor `SaveFormatError`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct ValueError(pub String);

/// Build a `path: message` save failure.
pub fn save_error(path: &str, message: &str) -> ValueError {
    ValueError(format!("{path}: {message}"))
}

/// Checkpoint value: plain data plus tagged big integers and raw bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveJson {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Number, including non-finite values and `-0`.
    Number(f64),
    /// Big integer.
    BigInt(i128),
    /// Raw bytes.
    Bytes(Vec<u8>),
    /// String.
    String(String),
    /// Array.
    Array(Vec<SaveJson>),
    /// Object (insertion order).
    Object(Vec<(String, SaveJson)>),
}

impl SaveJson {
    /// Look up an object member.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&SaveJson> {
        match self {
            Self::Object(members) => members
                .iter()
                .rev()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }
}

/// Object builder preserving insertion order.
#[must_use]
pub fn obj(members: Vec<(&str, SaveJson)>) -> SaveJson {
    SaveJson::Object(
        members
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect(),
    )
}

/// Array builder.
#[must_use]
pub fn arr(items: Vec<SaveJson>) -> SaveJson {
    SaveJson::Array(items)
}

/// String builder.
#[must_use]
pub fn str(value: &str) -> SaveJson {
    SaveJson::String(value.to_string())
}

/// Number builder.
#[must_use]
pub fn num(value: f64) -> SaveJson {
    SaveJson::Number(value)
}

/// Integer builder.
#[must_use]
pub fn int(value: i64) -> SaveJson {
    #[allow(clippy::cast_precision_loss)]
    SaveJson::Number(value as f64)
}

/// Boolean builder.
#[must_use]
pub fn boolean(value: bool) -> SaveJson {
    SaveJson::Bool(value)
}

/// Small boundary reader: every returned type is built from checked fields.
#[derive(Debug, Clone)]
pub struct SaveReader<'a> {
    /// Current value (`None` is a missing record field).
    pub value: Option<&'a SaveJson>,
    path: String,
}

impl<'a> SaveReader<'a> {
    /// Borrow a root value.
    #[must_use]
    pub fn new(value: &'a SaveJson) -> Self {
        Self {
            value: Some(value),
            path: "save".to_string(),
        }
    }

    /// Borrow a root value with an explicit path.
    #[must_use]
    pub fn at(value: &'a SaveJson, path: &str) -> Self {
        Self {
            value: Some(value),
            path: path.to_string(),
        }
    }

    /// Current path (for diagnostics).
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Whether the field is absent.
    #[must_use]
    pub fn is_missing(&self) -> bool {
        self.value.is_none()
    }

    /// Fail with a `path: message` error.
    pub fn fail(&self, message: &str) -> ValueError {
        save_error(&self.path, message)
    }

    /// Read a record field.
    pub fn field(&self, name: &str) -> SaveReader<'a> {
        let path = format!("{}.{name}", self.path);
        match self.value {
            Some(SaveJson::Object(members)) => SaveReader {
                value: members
                    .iter()
                    .rev()
                    .find(|(key, _)| key == name)
                    .map(|(_, value)| value),
                path,
            },
            _ => SaveReader { value: None, path },
        }
    }

    /// Require the current value to exist.
    fn require(&self) -> Result<&'a SaveJson, ValueError> {
        self.value.ok_or_else(|| self.fail("expected a record"))
    }

    /// Read a string.
    pub fn string(&self) -> Result<String, ValueError> {
        match self.require()? {
            SaveJson::String(value) => Ok(value.clone()),
            _ => Err(self.fail("expected a string")),
        }
    }

    /// Read a boolean.
    pub fn boolean(&self) -> Result<bool, ValueError> {
        match self.require()? {
            SaveJson::Bool(value) => Ok(*value),
            _ => Err(self.fail("expected a boolean")),
        }
    }

    /// Read a number (finite or not).
    pub fn number(&self) -> Result<f64, ValueError> {
        match self.require()? {
            SaveJson::Number(value) => Ok(*value),
            _ => Err(self.fail("expected a number")),
        }
    }

    /// Read a finite number.
    pub fn finite(&self) -> Result<f64, ValueError> {
        let value = self.number()?;
        if !value.is_finite() {
            return Err(self.fail("expected a finite number"));
        }
        Ok(value)
    }

    /// Read a safe integer at or above `minimum`.
    pub fn integer(&self, minimum: i64) -> Result<i64, ValueError> {
        let value = self.number()?;
        if value.trunc() != value || !(-9_007_199_254_740_991.0..=9_007_199_254_740_991.0).contains(&value) {
            return Err(self.fail("expected an integer in range"));
        }
        #[allow(clippy::cast_possible_truncation)]
        let integer = value as i64;
        if integer < minimum {
            return Err(self.fail("expected an integer in range"));
        }
        Ok(integer)
    }

    /// Read a big integer.
    pub fn bigint(&self) -> Result<i128, ValueError> {
        match self.require()? {
            SaveJson::BigInt(value) => Ok(*value),
            _ => Err(self.fail("expected a bigint")),
        }
    }

    /// Read raw checkpoint bytes.
    pub fn bytes(&self) -> Result<Vec<u8>, ValueError> {
        match self.require()? {
            SaveJson::Bytes(value) => Ok(value.clone()),
            _ => Err(self.fail("expected raw checkpoint bytes")),
        }
    }

    /// Require an exact string.
    pub fn literal_str(&self, expected: &str) -> Result<String, ValueError> {
        let value = self.string()?;
        if value != expected {
            return Err(self.fail(&format!("expected {expected}")));
        }
        Ok(value)
    }

    /// Require an exact integer.
    pub fn literal_i64(&self, expected: i64) -> Result<i64, ValueError> {
        let value = self.integer(i64::MIN)?;
        if value != expected {
            return Err(self.fail(&format!("expected {expected}")));
        }
        Ok(value)
    }

    /// Require an exact boolean.
    pub fn literal_bool(&self, expected: bool) -> Result<bool, ValueError> {
        let value = self.boolean()?;
        if value != expected {
            return Err(self.fail(&format!("expected {expected}")));
        }
        Ok(value)
    }

    /// Require one of several strings.
    pub fn choice_str(&self, choices: &[&str]) -> Result<String, ValueError> {
        let value = self.string()?;
        if choices.iter().any(|choice| *choice == value) {
            Ok(value)
        } else {
            Err(self.fail(&format!("expected {}", choices.join(" or "))))
        }
    }

    /// Require one of several integers.
    pub fn choice_i64(&self, choices: &[i64]) -> Result<i64, ValueError> {
        let value = self.integer(i64::MIN)?;
        if choices.contains(&value) {
            Ok(value)
        } else {
            Err(self.fail("expected an integer in range"))
        }
    }

    /// Read an array.
    pub fn list<T, E>(&self, mut read: impl FnMut(SaveReader<'a>) -> Result<T, E>) -> Result<Vec<T>, E>
    where
        E: From<ValueError>,
    {
        match self.require()? {
            SaveJson::Array(items) => items
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    read(SaveReader {
                        value: Some(item),
                        path: format!("{}[{index}]", self.path),
                    })
                })
                .collect(),
            _ => Err(self.fail("expected an array").into()),
        }
    }

    /// Read a nullable value (`null` maps to `None`).
    pub fn nullable<T, E>(&self, mut read: impl FnMut(SaveReader<'a>) -> Result<T, E>) -> Result<Option<T>, E>
    where
        E: From<ValueError>,
    {
        match self.value {
            Some(SaveJson::Null) => Ok(None),
            _ => read(self.clone()).map(Some),
        }
    }
}

/// Read a `namespace:name` identity.
pub fn namespaced(reader: SaveReader) -> Result<String, ValueError> {
    let value = reader.string()?;
    match value.find(':') {
        Some(colon) if colon > 0 && colon + 1 < value.len() => Ok(value),
        _ => Err(reader.fail("expected a namespaced identity")),
    }
}

/// Read a content digest.
pub fn read_digest(reader: SaveReader) -> Result<ContentDigest, ValueError> {
    let value = reader.string()?;
    if !is_content_digest(&value) {
        return Err(reader.fail("expected a SHA-256 content digest"));
    }
    Ok(ContentDigest(value))
}

/// Read a vector (donor `number()` accepts non-finite components).
pub fn read_vector(reader: SaveReader) -> Result<Vec3, ValueError> {
    #[allow(clippy::cast_possible_truncation)]
    Ok(Vec3 {
        x: reader.field("x").number()? as f32,
        y: reader.field("y").number()? as f32,
        z: reader.field("z").number()? as f32,
    })
}

/// Strict JSON text parser (`JSON.parse` semantics) producing [`SaveJson`].
///
/// Used for authored declaration files (mod declarations, `mapdb.json`),
/// not for tagged checkpoint envelopes. Duplicate object keys keep the
/// last value, matching `JSON.parse`.
pub fn parse_save_json(text: &str) -> Result<SaveJson, ValueError> {
    let mut parser = JsonParser {
        bytes: text.as_bytes(),
        index: 0,
    };
    parser.skip_whitespace();
    let value = parser.value()?;
    parser.skip_whitespace();
    if parser.index != parser.bytes.len() {
        return Err(save_error("json", "unexpected trailing text"));
    }
    Ok(value)
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl JsonParser<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.index += 1;
        Some(byte)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.index += 1;
        }
    }

    fn fail(&self, message: &str) -> ValueError {
        save_error(&format!("json@{}", self.index), message)
    }

    fn expect(&mut self, byte: u8, what: &str) -> Result<(), ValueError> {
        if self.next() == Some(byte) {
            Ok(())
        } else {
            Err(self.fail(&format!("expected {what}")))
        }
    }

    fn value(&mut self) -> Result<SaveJson, ValueError> {
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(SaveJson::String(self.string()?)),
            Some(b't') => self.keyword("true", SaveJson::Bool(true)),
            Some(b'f') => self.keyword("false", SaveJson::Bool(false)),
            Some(b'n') => self.keyword("null", SaveJson::Null),
            Some(byte) if byte == b'-' || byte.is_ascii_digit() => self.number(),
            _ => Err(self.fail("expected a value")),
        }
    }

    fn keyword(&mut self, word: &str, value: SaveJson) -> Result<SaveJson, ValueError> {
        if self.bytes[self.index..].starts_with(word.as_bytes()) {
            self.index += word.len();
            Ok(value)
        } else {
            Err(self.fail("expected a value"))
        }
    }

    fn object(&mut self) -> Result<SaveJson, ValueError> {
        self.expect(b'{', "'{'")?;
        let mut members = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.index += 1;
            return Ok(SaveJson::Object(members));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.fail("expected a string key"));
            }
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':', "':'")?;
            self.skip_whitespace();
            let value = self.value()?;
            members.push((key, value));
            self.skip_whitespace();
            match self.next() {
                Some(b',') => continue,
                Some(b'}') => return Ok(SaveJson::Object(members)),
                _ => return Err(self.fail("expected ',' or '}'")),
            }
        }
    }

    fn array(&mut self) -> Result<SaveJson, ValueError> {
        self.expect(b'[', "'['")?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.index += 1;
            return Ok(SaveJson::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value()?);
            self.skip_whitespace();
            match self.next() {
                Some(b',') => continue,
                Some(b']') => return Ok(SaveJson::Array(items)),
                _ => return Err(self.fail("expected ',' or ']'")),
            }
        }
    }

    fn string(&mut self) -> Result<String, ValueError> {
        self.expect(b'"', "'\"'")?;
        let mut out = String::new();
        loop {
            match self.next() {
                None => return Err(self.fail("unterminated string")),
                Some(b'"') => return Ok(out),
                Some(b'\\') => match self.next() {
                    Some(b'"') => out.push('"'),
                    Some(b'\\') => out.push('\\'),
                    Some(b'/') => out.push('/'),
                    Some(b'b') => out.push('\u{0008}'),
                    Some(b'f') => out.push('\u{000C}'),
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'u') => {
                        let high = self.hex4()?;
                        if (0xD800..0xDC00).contains(&high) {
                            if self.next() == Some(b'\\') && self.next() == Some(b'u') {
                                let low = self.hex4()?;
                                if (0xDC00..0xE000).contains(&low) {
                                    let code = 0x1_0000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                                    out.push(char::from_u32(code).ok_or_else(|| self.fail("invalid escape"))?);
                                } else {
                                    return Err(self.fail("invalid escape"));
                                }
                            } else {
                                return Err(self.fail("invalid escape"));
                            }
                        } else if (0xDC00..0xE000).contains(&high) {
                            return Err(self.fail("invalid escape"));
                        } else {
                            out.push(char::from_u32(high).ok_or_else(|| self.fail("invalid escape"))?);
                        }
                    }
                    _ => return Err(self.fail("invalid escape")),
                },
                Some(byte) if byte < 0x20 => return Err(self.fail("unescaped control character")),
                Some(_) => {
                    // Collect a run of non-special bytes, then decode UTF-8.
                    let start = self.index - 1;
                    loop {
                        match self.peek() {
                            Some(next) if next != b'"' && next != b'\\' && next >= 0x20 => {
                                self.index += 1;
                            }
                            _ => break,
                        }
                    }
                    let run = &self.bytes[start..self.index];
                    out.push_str(std::str::from_utf8(run).map_err(|_| self.fail("invalid UTF-8"))?);
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, ValueError> {
        if self.index + 4 > self.bytes.len() {
            return Err(self.fail("invalid escape"));
        }
        let digits = &self.bytes[self.index..self.index + 4];
        let mut value = 0u32;
        for digit in digits {
            value = value * 16
                + match digit {
                    b'0'..=b'9' => u32::from(digit - b'0'),
                    b'a'..=b'f' => u32::from(digit - b'a') + 10,
                    b'A'..=b'F' => u32::from(digit - b'A') + 10,
                    _ => return Err(self.fail("invalid escape")),
                };
        }
        self.index += 4;
        Ok(value)
    }

    fn number(&mut self) -> Result<SaveJson, ValueError> {
        let start = self.index;
        if self.peek() == Some(b'-') {
            self.index += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.index += 1;
            }
            Some(byte) if byte.is_ascii_digit() => {
                while self.peek().is_some_and(|next| next.is_ascii_digit()) {
                    self.index += 1;
                }
            }
            _ => return Err(self.fail("expected a number")),
        }
        if self.peek() == Some(b'.') {
            self.index += 1;
            if !self.peek().is_some_and(|next| next.is_ascii_digit()) {
                return Err(self.fail("expected a number"));
            }
            while self.peek().is_some_and(|next| next.is_ascii_digit()) {
                self.index += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.index += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.index += 1;
            }
            if !self.peek().is_some_and(|next| next.is_ascii_digit()) {
                return Err(self.fail("expected a number"));
            }
            while self.peek().is_some_and(|next| next.is_ascii_digit()) {
                self.index += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.index]).map_err(|_| self.fail("expected a number"))?;
        text.parse::<f64>()
            .map(SaveJson::Number)
            .map_err(|_| self.fail("expected a number"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_records() {
        let value = obj(vec![
            ("name", str("q1:shells")),
            ("count", num(5.0)),
            ("tags", arr(vec![str("a"), str("b")])),
            ("maybe", SaveJson::Null),
            ("enabled", boolean(true)),
        ]);
        let reader = SaveReader::new(&value);
        assert_eq!(namespaced(reader.field("name")).unwrap(), "q1:shells");
        assert_eq!(reader.field("count").integer(0).unwrap(), 5);
        assert_eq!(
            reader.field("tags").list(|item| item.string()).unwrap(),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(reader.field("maybe").nullable(|item| item.string()).unwrap(), None);
        assert!(reader.field("enabled").boolean().unwrap());
        assert!(reader.field("missing").is_missing());
        assert!(reader.field("count").string().is_err());
    }

    #[test]
    fn reads_digests_and_vectors() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        let value = obj(vec![
            ("digest", str(&digest)),
            ("offset", obj(vec![("x", num(1.0)), ("y", num(2.0)), ("z", num(3.0))])),
        ]);
        let reader = SaveReader::new(&value);
        assert_eq!(read_digest(reader.field("digest")).unwrap().as_str(), digest);
        let vector = read_vector(reader.field("offset")).unwrap();
        assert_eq!((vector.x, vector.y, vector.z), (1.0, 2.0, 3.0));
    }

    #[test]
    fn parses_json_text() {
        let value = parse_save_json(
            r#"{"name": "q1:shells", "count": -12.5e2, "tags": ["a", "b\nc", "𝄞"], "nil": null, "ok": true, "esc": "A\u0041\ud834\udd1e"}"#,
        )
        .unwrap();
        let reader = SaveReader::new(&value);
        assert_eq!(reader.field("name").string().unwrap(), "q1:shells");
        assert_eq!(reader.field("count").number().unwrap(), -1250.0);
        assert_eq!(
            reader.field("tags").list(|item| item.string()).unwrap(),
            vec!["a".to_string(), "b\nc".to_string(), "𝄞".to_string()]
        );
        assert_eq!(reader.field("esc").string().unwrap(), "AA𝄞");
        assert!(reader.field("ok").boolean().unwrap());
        assert!(parse_save_json(r#"{"a": 01}"#).is_err());
        assert!(parse_save_json(r#"{"a": }"#).is_err());
        assert!(parse_save_json(r"[1, 2] trailing").is_err());
        assert!(parse_save_json("\"\\ud800\"").is_err());
    }
}
