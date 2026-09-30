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
}
