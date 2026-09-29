//! Checkpoint value envelope ported from `src/persistence/value.ts`.
//!
//! Checkpoints are plain data: structs serialize to [`SaveJson`], and
//! [`encode_checkpoint_value`] renders the tagged JSON bytes the unified
//! save framing stores. Non-finite/`-0` numbers, big integers, and raw
//! bytes travel as `$qts`-tagged records; [`SaveReader`] validates every
//! field on the way back. Base64 must be canonical (donor
//! `canonicalBase64`): correct padding and zeroed trailing bits.

use crate::WorldError;

use super::json::{parse_source_json, SourceJson};

/// Build a `path: message` save failure (donor `SaveFormatError` shape).
pub fn save_error(path: &str, message: &str) -> WorldError {
    WorldError::BadSave(format!("{path}: {message}"))
}

/// Checkpoint value: plain data plus tagged big integers and raw bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveJson {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Number, including non-finite values and `-0` (tagged on encode).
    Number(f64),
    /// Big integer (tagged on encode).
    BigInt(i128),
    /// Raw bytes (base64-tagged on encode).
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
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

/// Array builder.
#[must_use]
pub fn arr(items: Vec<SaveJson>) -> SaveJson {
    SaveJson::Array(items)
}

/// String value.
#[must_use]
pub fn str(value: &str) -> SaveJson {
    SaveJson::String(value.to_string())
}

/// Finite-or-not number value.
#[must_use]
pub fn num(value: f64) -> SaveJson {
    SaveJson::Number(value)
}

/// Integer number value.
#[must_use]
pub fn int(value: i64) -> SaveJson {
    SaveJson::Number(value as f64)
}

/// Boolean value.
#[must_use]
pub fn boolean(value: bool) -> SaveJson {
    SaveJson::Bool(value)
}

const BASE64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode bytes as standard padded base64.
#[must_use]
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let word = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | (chunk.get(2).copied().unwrap_or(0) as u32);
        out.push(BASE64_ALPHABET[(word >> 18) as usize & 63] as char);
        out.push(BASE64_ALPHABET[(word >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(BASE64_ALPHABET[(word >> 6) as usize & 63] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(BASE64_ALPHABET[word as usize & 63] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_digit(byte: u8) -> Option<u32> {
    match byte {
        b'A'..=b'Z' => Some((byte - b'A') as u32),
        b'a'..=b'z' => Some((byte - b'a') as u32 + 26),
        b'0'..=b'9' => Some((byte - b'0') as u32 + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decode canonical base64 (padding plus zeroed trailing bits enforced).
fn base64_decode(text: &str) -> Result<Vec<u8>, ()> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(());
    }
    let padding = if text.ends_with("==") {
        2
    } else if text.ends_with('=') {
        1
    } else {
        0
    };
    let end = bytes.len() - padding;
    let mut digits: Vec<u32> = Vec::with_capacity(end);
    for &byte in &bytes[..end] {
        digits.push(base64_digit(byte).ok_or(())?);
    }
    for &byte in &bytes[end..] {
        if byte != b'=' {
            return Err(());
        }
    }
    if let Some(&last) = digits.last() {
        if padding == 2 && last & 15 != 0 {
            return Err(());
        }
        if padding == 1 && last & 3 != 0 {
            return Err(());
        }
    }
    let mut out = Vec::new();
    for chunk in digits.chunks(4) {
        let mut word = 0u32;
        for (index, &digit) in chunk.iter().enumerate() {
            word |= digit << (18 - index * 6);
        }
        let count = if chunk.len() == 4 { 3 } else { chunk.len() - 1 };
        for index in 0..count {
            out.push(((word >> (16 - index * 8)) & 0xff) as u8);
        }
    }
    Ok(out)
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
    if value == 0.0 && value.is_sign_negative() {
        out.push_str(r#"{"$qts":"number","value":"-0"}"#);
    } else if value.is_nan() {
        out.push_str(r#"{"$qts":"number","value":"NaN"}"#);
    } else if value == f64::INFINITY {
        out.push_str(r#"{"$qts":"number","value":"Infinity"}"#);
    } else if value == f64::NEG_INFINITY {
        out.push_str(r#"{"$qts":"number","value":"-Infinity"}"#);
    } else {
        out.push_str(&format!("{value:?}"));
    }
}

fn render(value: &SaveJson, out: &mut String) {
    match value {
        SaveJson::Null => out.push_str("null"),
        SaveJson::Bool(true) => out.push_str("true"),
        SaveJson::Bool(false) => out.push_str("false"),
        SaveJson::Number(value) => render_number(*value, out),
        SaveJson::BigInt(value) => {
            out.push_str(r#"{"$qts":"bigint","value":""#);
            out.push_str(&format!("{value}"));
            out.push_str(r#""}"#);
        }
        SaveJson::Bytes(bytes) => {
            out.push_str(r#"{"$qts":"bytes","value":""#);
            out.push_str(&base64_encode(bytes));
            out.push_str(r#""}"#);
        }
        SaveJson::String(text) => escape_into(text, out),
        SaveJson::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                render(item, out);
            }
            out.push(']');
        }
        SaveJson::Object(members) => {
            for (key, _) in members {
                assert_ne!(*key, "$qts", "checkpoint contains reserved $qts key");
            }
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

/// Encode a checkpoint value to tagged JSON bytes.
#[must_use]
pub fn encode_checkpoint_value(value: &SaveJson) -> Vec<u8> {
    let mut out = String::new();
    render(value, &mut out);
    out.into_bytes()
}

fn valid_bigint_text(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() {
        return false;
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return false;
    }
    digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn convert(source: &SourceJson, path: &str) -> Result<SaveJson, WorldError> {
    match source {
        SourceJson::Null => Ok(SaveJson::Null),
        SourceJson::Bool(value) => Ok(SaveJson::Bool(*value)),
        SourceJson::String(text) => Ok(SaveJson::String(text.clone())),
        SourceJson::Number(number) => {
            // Strict checkpoint JSON rejects the source-only spellings.
            if matches!(number.text(), "NaN" | "Infinity" | "-Infinity") {
                return Err(save_error(path, "invalid tagged checkpoint value"));
            }
            Ok(SaveJson::Number(number.number()))
        }
        SourceJson::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| convert(item, &format!("{path}[{index}]")))
            .collect::<Result<Vec<_>, _>>()
            .map(SaveJson::Array),
        SourceJson::Object(members) => {
            if let Some(tag) = members.iter().find(|(name, _)| name == "$qts") {
                if members.len() != 2 {
                    return Err(save_error(path, "invalid tagged checkpoint value"));
                }
                let encoded = members.iter().find(|(name, _)| name == "value").map(|(_, value)| value);
                let SourceJson::String(encoded) =
                    encoded.ok_or_else(|| save_error(path, "invalid tagged checkpoint value"))?
                else {
                    return Err(save_error(path, "invalid tagged checkpoint value"));
                };
                let SourceJson::String(tag) = &tag.1 else {
                    return Err(save_error(path, "invalid tagged checkpoint value"));
                };
                match tag.as_str() {
                    "bigint" if valid_bigint_text(encoded) => encoded
                        .parse::<i128>()
                        .map(SaveJson::BigInt)
                        .map_err(|_| save_error(path, "invalid tagged checkpoint value")),
                    "bytes" => base64_decode(encoded)
                        .map(SaveJson::Bytes)
                        .map_err(|_| save_error(path, "unknown tagged checkpoint value")),
                    "number" => match encoded.as_str() {
                        "-0" => Ok(SaveJson::Number(-0.0)),
                        "NaN" => Ok(SaveJson::Number(f64::NAN)),
                        "Infinity" => Ok(SaveJson::Number(f64::INFINITY)),
                        "-Infinity" => Ok(SaveJson::Number(f64::NEG_INFINITY)),
                        _ => Err(save_error(path, "unknown tagged checkpoint value")),
                    },
                    _ => Err(save_error(path, "unknown tagged checkpoint value")),
                }
            } else {
                members
                    .iter()
                    .map(|(key, item)| convert(item, &format!("{path}.{key}")).map(|value| (key.clone(), value)))
                    .collect::<Result<Vec<_>, _>>()
                    .map(SaveJson::Object)
            }
        }
    }
}

/// Decode tagged checkpoint JSON bytes.
pub fn decode_checkpoint_value(bytes: &[u8]) -> Result<SaveJson, WorldError> {
    let text = std::str::from_utf8(bytes).map_err(|_| save_error("checkpoint", "invalid checkpoint text"))?;
    let parsed = parse_source_json(text).map_err(|_| save_error("checkpoint", "invalid checkpoint text"))?;
    convert(&parsed, "checkpoint")
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
    pub fn fail(&self, message: &str) -> WorldError {
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

    /// Require a record field to exist, failing `expected a record` otherwise.
    fn require(&self) -> Result<&'a SaveJson, WorldError> {
        self.value.ok_or_else(|| self.fail("expected a record"))
    }

    /// Read a string.
    pub fn string(&self) -> Result<String, WorldError> {
        match self.require()? {
            SaveJson::String(value) => Ok(value.clone()),
            _ => Err(self.fail("expected a string")),
        }
    }

    /// Read a boolean.
    pub fn boolean(&self) -> Result<bool, WorldError> {
        match self.require()? {
            SaveJson::Bool(value) => Ok(*value),
            _ => Err(self.fail("expected a boolean")),
        }
    }

    /// Read a number (finite or not).
    pub fn number(&self) -> Result<f64, WorldError> {
        match self.require()? {
            SaveJson::Number(value) => Ok(*value),
            _ => Err(self.fail("expected a number")),
        }
    }

    /// Read a finite number.
    pub fn finite(&self) -> Result<f64, WorldError> {
        let value = self.number()?;
        if !value.is_finite() {
            return Err(self.fail("expected a finite number"));
        }
        Ok(value)
    }

    /// Read a safe integer at or above `minimum`.
    pub fn integer(&self, minimum: i64) -> Result<i64, WorldError> {
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
    pub fn bigint(&self) -> Result<i128, WorldError> {
        match self.require()? {
            SaveJson::BigInt(value) => Ok(*value),
            _ => Err(self.fail("expected a bigint")),
        }
    }

    /// Read raw checkpoint bytes.
    pub fn bytes(&self) -> Result<Vec<u8>, WorldError> {
        match self.require()? {
            SaveJson::Bytes(value) => Ok(value.clone()),
            _ => Err(self.fail("expected raw checkpoint bytes")),
        }
    }

    /// Require an exact string.
    pub fn literal_str(&self, expected: &str) -> Result<String, WorldError> {
        let value = self.string()?;
        if value != expected {
            return Err(self.fail(&format!("expected {expected}")));
        }
        Ok(value)
    }

    /// Require an exact integer.
    pub fn literal_i64(&self, expected: i64) -> Result<i64, WorldError> {
        let value = self.integer(i64::MIN)?;
        if value != expected {
            return Err(self.fail(&format!("expected {expected}")));
        }
        Ok(value)
    }

    /// Require an exact boolean.
    pub fn literal_bool(&self, expected: bool) -> Result<bool, WorldError> {
        let value = self.boolean()?;
        if value != expected {
            return Err(self.fail(&format!("expected {expected}")));
        }
        Ok(value)
    }

    /// Require one of several strings.
    pub fn choice_str(&self, choices: &[&str]) -> Result<String, WorldError> {
        let value = self.string()?;
        if choices.iter().any(|choice| *choice == value) {
            Ok(value)
        } else {
            Err(self.fail(&format!("expected {}", choices.join(" or "))))
        }
    }

    /// Require one of several integers.
    pub fn choice_i64(&self, choices: &[i64]) -> Result<i64, WorldError> {
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
        E: From<WorldError>,
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
        E: From<WorldError>,
    {
        match self.value {
            Some(SaveJson::Null) => Ok(None),
            _ => read(self.clone()).map(Some),
        }
    }
}

/// Read a `namespace:name` identity.
pub fn namespaced(reader: SaveReader) -> Result<String, WorldError> {
    let value = reader.string()?;
    match value.find(':') {
        Some(colon) if colon > 0 && colon + 1 < value.len() => Ok(value),
        _ => Err(reader.fail("expected a namespaced identity")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(value: &SaveJson) -> SaveJson {
        let bytes = encode_checkpoint_value(value);
        let decoded = decode_checkpoint_value(&bytes).unwrap();
        // `-0`/`NaN` need bit comparison.
        assert_eq!(
            encode_checkpoint_value(&decoded),
            bytes,
            "re-encode must be byte-stable"
        );
        decoded
    }

    #[test]
    fn tagged_values_round_trip() {
        let value = obj(vec![
            ("plain", num(1.5)),
            ("negzero", num(-0.0)),
            ("nan", num(f64::NAN)),
            ("inf", num(f64::INFINITY)),
            ("ninf", num(f64::NEG_INFINITY)),
            ("big", SaveJson::BigInt(-123456789012345678901234567890)),
            ("bytes", SaveJson::Bytes(vec![0, 1, 2, 250, 255])),
            ("empty", SaveJson::Bytes(Vec::new())),
            ("nested", arr(vec![boolean(true), SaveJson::Null, str("x")])),
        ]);
        let decoded = round_trip(&value);
        assert_eq!(decoded.get("plain"), Some(&SaveJson::Number(1.5)));
        match decoded.get("negzero") {
            Some(SaveJson::Number(value)) => assert_eq!(value.to_bits(), (-0.0f64).to_bits()),
            other => panic!("unexpected -0: {other:?}"),
        }
        match decoded.get("nan") {
            Some(SaveJson::Number(value)) => assert!(value.is_nan()),
            other => panic!("unexpected NaN: {other:?}"),
        }
        match decoded.get("big") {
            Some(SaveJson::BigInt(value)) => assert_eq!(*value, -123456789012345678901234567890),
            other => panic!("unexpected bigint: {other:?}"),
        }
        assert_eq!(decoded.get("bytes"), Some(&SaveJson::Bytes(vec![0, 1, 2, 250, 255])));
        let text = String::from_utf8(encode_checkpoint_value(&value)).unwrap();
        assert!(text.contains(r#""$qts":"bytes""#));
        assert!(text.contains(r#""$qts":"bigint""#));
    }

    #[test]
    fn canonical_base64_is_enforced() {
        assert_eq!(base64_decode("").unwrap(), Vec::<u8>::new());
        assert_eq!(base64_decode("YWI=").unwrap(), b"ab");
        assert_eq!(base64_decode("YQ==").unwrap(), b"a");
        // Non-canonical trailing bits.
        assert!(base64_decode("YR==").is_err());
        assert!(base64_decode("YWF=").is_err());
        assert!(base64_decode(" wider ").is_err());
        assert!(decode_checkpoint_value(br#"{"$qts":"bytes","value":"!!"}"#).is_err());
        assert!(decode_checkpoint_value(br#"{"$qts":"bigint","value":"01"}"#).is_err());
        assert!(decode_checkpoint_value(br#"{"$qts":"number","value":"1"}"#).is_err());
        assert!(decode_checkpoint_value(br#"{"$qts":"mystery","value":"eA=="}"#).is_err());
        assert!(decode_checkpoint_value(br#"{"$qts":"bytes"}"#).is_err());
    }

    #[test]
    fn reader_validates_fields() {
        let value = obj(vec![
            ("name", str("q3:game")),
            ("count", int(3)),
            ("maybe", SaveJson::Null),
            ("items", arr(vec![int(1), int(2)])),
        ]);
        let reader = SaveReader::new(&value);
        assert_eq!(namespaced(reader.field("name")).unwrap(), "q3:game");
        assert!(namespaced(reader.field("count")).is_err());
        assert_eq!(reader.field("count").integer(0).unwrap(), 3);
        assert!(reader.field("count").integer(4).is_err());
        assert_eq!(reader.field("maybe").nullable(|value| value.string()).unwrap(), None);
        assert!(reader.field("missing").is_missing());
        assert!(reader.field("missing").string().is_err());
        assert_eq!(reader.field("items").list(|item| item.integer(0)).unwrap(), vec![1, 2]);
        assert!(reader.field("name").list(|item| item.integer(0)).is_err());
        let bad = str("nope");
        assert!(SaveReader::new(&bad).field("x").string().is_err());
    }
}
