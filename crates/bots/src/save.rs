//! Checkpoint values and the validating reader used by navigation
//! checkpoint restore, ported from `src/persistence/value.ts`
//! (`SaveReader`). Values are plain data; the reader pins the expected
//! shape with donor diagnostics.

use std::collections::BTreeMap;

use thiserror::Error;

/// Checkpoint decode failure with a dotted path.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{path}: {message}")]
pub struct SaveError {
    /// Dotted value path.
    pub path: String,
    /// What went wrong.
    pub message: String,
}

/// Checkpoint value.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveValue {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Integer.
    Int(i64),
    /// Float.
    Float(f64),
    /// String.
    Str(String),
    /// Raw bytes.
    Bytes(Vec<u8>),
    /// Array.
    List(Vec<SaveValue>),
    /// Record.
    Map(BTreeMap<String, SaveValue>),
}

impl SaveValue {
    /// Build a record from pairs.
    #[must_use]
    pub fn map(pairs: Vec<(&str, SaveValue)>) -> Self {
        SaveValue::Map(pairs.into_iter().map(|(key, value)| (key.to_string(), value)).collect())
    }
}

/// Validating checkpoint reader.
#[derive(Debug, Clone)]
pub struct SaveReader<'a> {
    value: &'a SaveValue,
    path: String,
}

impl<'a> SaveReader<'a> {
    /// Read a root value.
    #[must_use]
    pub fn new(value: &'a SaveValue, path: &str) -> Self {
        Self {
            value,
            path: path.to_string(),
        }
    }

    /// Fail with a message at this path.
    pub fn fail<T>(&self, message: &str) -> Result<T, SaveError> {
        Err(SaveError {
            path: self.path.clone(),
            message: message.to_string(),
        })
    }

    /// Read a record field.
    pub fn field(&self, name: &str) -> Result<SaveReader<'a>, SaveError> {
        match self.value {
            SaveValue::Map(map) => match map.get(name) {
                Some(value) => Ok(SaveReader {
                    value,
                    path: format!("{}.{}", self.path, name),
                }),
                None => self.fail("expected a record"),
            },
            _ => self.fail("expected a record"),
        }
    }

    /// Read a string.
    pub fn string(&self) -> Result<&str, SaveError> {
        match self.value {
            SaveValue::Str(value) => Ok(value),
            _ => self.fail("expected a string"),
        }
    }

    /// Read a boolean.
    pub fn boolean(&self) -> Result<bool, SaveError> {
        match self.value {
            SaveValue::Bool(value) => Ok(*value),
            _ => self.fail("expected a boolean"),
        }
    }

    /// Read a number (integer or float).
    pub fn number(&self) -> Result<f64, SaveError> {
        match self.value {
            SaveValue::Int(value) => Ok(*value as f64),
            SaveValue::Float(value) => Ok(*value),
            _ => self.fail("expected a number"),
        }
    }

    /// Read a finite number.
    pub fn finite(&self) -> Result<f64, SaveError> {
        let value = self.number()?;
        if value.is_finite() {
            Ok(value)
        } else {
            self.fail("expected a finite number")
        }
    }

    /// Read an integer at or above a minimum.
    pub fn integer(&self, minimum: i64) -> Result<i64, SaveError> {
        let value = match self.value {
            SaveValue::Int(value) => *value,
            SaveValue::Float(value) if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0 => *value as i64,
            _ => return self.fail("expected an integer in range"),
        };
        if value < minimum {
            return self.fail("expected an integer in range");
        }
        Ok(value)
    }

    /// Read an expected integer literal.
    pub fn literal_int(&self, expected: i64) -> Result<i64, SaveError> {
        match self.value {
            SaveValue::Int(value) if *value == expected => Ok(expected),
            _ => self.fail(&format!("expected {expected}")),
        }
    }

    /// Read an expected string literal.
    pub fn literal_str(&self, expected: &str) -> Result<(), SaveError> {
        match self.value {
            SaveValue::Str(value) if value == expected => Ok(()),
            _ => self.fail(&format!("expected {expected}")),
        }
    }

    /// Read an array.
    pub fn list<T>(&self, mut read: impl FnMut(&SaveReader<'a>) -> Result<T, SaveError>) -> Result<Vec<T>, SaveError> {
        match self.value {
            SaveValue::List(values) => values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    read(&SaveReader {
                        value,
                        path: format!("{}[{index}]", self.path),
                    })
                })
                .collect(),
            _ => self.fail("expected an array"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(value: &SaveValue) -> SaveReader<'_> {
        SaveReader::new(value, "root")
    }

    #[test]
    fn reads_typed_fields() {
        let value = SaveValue::map(vec![
            ("s", SaveValue::Str("hi".to_string())),
            ("b", SaveValue::Bool(true)),
            ("i", SaveValue::Int(3)),
            ("f", SaveValue::Float(1.5)),
            ("l", SaveValue::List(vec![SaveValue::Int(1)])),
        ]);
        let root = reader(&value);
        assert_eq!(root.field("s").unwrap().string().unwrap(), "hi");
        assert!(root.field("b").unwrap().boolean().unwrap());
        assert_eq!(root.field("i").unwrap().integer(0).unwrap(), 3);
        assert_eq!(root.field("f").unwrap().finite().unwrap(), 1.5);
        assert_eq!(root.field("i").unwrap().literal_int(3).unwrap(), 3);
        root.field("s").unwrap().literal_str("hi").unwrap();
        let list = root.field("l").unwrap().list(|entry| entry.integer(0)).unwrap();
        assert_eq!(list, vec![1]);
    }

    #[test]
    fn rejects_mismatches_with_paths() {
        let value = SaveValue::map(vec![("i", SaveValue::Str("x".to_string()))]);
        let root = reader(&value);
        let error = root.field("i").unwrap().integer(0).unwrap_err();
        assert_eq!(error.path, "root.i");
        let error = root.field("missing").unwrap_err();
        assert!(error.message.contains("record"));
        let bad = SaveValue::map(vec![("f", SaveValue::Float(f64::INFINITY))]);
        assert!(reader(&bad).field("f").unwrap().finite().is_err());
        let bad = SaveValue::map(vec![("i", SaveValue::Int(-1))]);
        assert!(reader(&bad).field("i").unwrap().integer(0).is_err());
    }
}
