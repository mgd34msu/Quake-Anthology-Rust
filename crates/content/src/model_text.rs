//! MD5 and animation.cfg text tokens (`src/formats/q3-model/text.ts`).
//!
//! Donor provenance: `ModelTokens`, `indexedRecords`, and `at` in
//! `src/formats/q3-model/text.ts`, with the MD5 token grammar derived
//! from q2repro `models.c`. The donor `ModelTextError` displays as
//! `source:offset: message`, exactly like
//! [`qa_core::binary::BinaryError`], which this port reports instead.

use qa_core::binary::BinaryError;
use qa_core::math::{vec3, Vec3};

fn is_token_whitespace(character: char) -> bool {
    matches!(
        character,
        '\t' | '\n' | '\u{000b}' | '\u{000c}' | '\r' | ' ' | '\u{00a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// Token reader over model text (`ModelTokens`).
#[derive(Debug, Clone)]
pub struct ModelTokens {
    chars: Vec<char>,
    offset: usize,
    source: String,
}

impl ModelTokens {
    /// Borrow model text under a source name.
    #[must_use]
    pub fn new(text: &str, source: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            offset: 0,
            source: source.to_string(),
        }
    }

    /// Fail at the current offset.
    pub fn fail<T>(&self, message: String) -> Result<T, BinaryError> {
        Err(BinaryError {
            input: self.source.clone(),
            offset: self.offset,
            message,
        })
    }

    fn starts_with(&self, text: &str) -> bool {
        let pattern: Vec<char> = text.chars().collect();
        self.chars[self.offset..].starts_with(&pattern)
    }

    /// Read the next token, or `None` at the end of input.
    pub fn next_token(&mut self) -> Result<Option<String>, BinaryError> {
        loop {
            while self.offset < self.chars.len() && is_token_whitespace(self.chars[self.offset]) {
                self.offset += 1;
            }
            if self.starts_with("//") {
                while self.offset < self.chars.len() && self.chars[self.offset] != '\n' {
                    self.offset += 1;
                }
                continue;
            }
            if self.starts_with("/*") {
                let mut end = None;
                for position in self.offset + 2..self.chars.len().saturating_sub(1) {
                    if self.chars[position] == '*' && self.chars[position + 1] == '/' {
                        end = Some(position);
                        break;
                    }
                }
                match end {
                    Some(end) => {
                        self.offset = end + 2;
                        continue;
                    }
                    None => return self.fail("unterminated comment".to_string()),
                }
            }
            break;
        }
        if self.offset == self.chars.len() {
            return Ok(None);
        }
        let start = self.offset;
        self.offset += 1;
        let first = self.chars[start];
        if first == '"' {
            let mut end = None;
            for position in self.offset..self.chars.len() {
                if self.chars[position] == '"' {
                    end = Some(position);
                    break;
                }
            }
            match end {
                Some(end) => {
                    let token: String = self.chars[self.offset..end].iter().collect();
                    self.offset = end + 1;
                    return Ok(Some(token));
                }
                None => return self.fail("unterminated quoted token".to_string()),
            }
        }
        if matches!(first, '{' | '}' | '(' | ')') {
            return Ok(Some(first.to_string()));
        }
        while self.offset < self.chars.len() {
            let character = self.chars[self.offset];
            if is_token_whitespace(character) || matches!(character, '{' | '}' | '(' | ')') {
                break;
            }
            self.offset += 1;
        }
        Ok(Some(self.chars[start..self.offset].iter().collect()))
    }

    /// Read the next token, failing at the end of input.
    pub fn token(&mut self) -> Result<String, BinaryError> {
        match self.next_token()? {
            Some(token) => Ok(token),
            None => self.fail("unexpected end of model text".to_string()),
        }
    }

    /// Consume an expected keyword.
    pub fn expect(&mut self, expected: &str) -> Result<(), BinaryError> {
        let actual = self.token()?;
        if actual != expected {
            return self.fail(format!("expected {expected:?}, got {actual:?}"));
        }
        Ok(())
    }

    /// Read a finite floating-point number.
    pub fn number(&mut self) -> Result<f64, BinaryError> {
        let token = self.token()?;
        let value: f64 = token.parse().unwrap_or(f64::NAN);
        if token.is_empty() || !value.is_finite() {
            return self.fail(format!("invalid number {token:?}"));
        }
        Ok(value)
    }

    /// Read a binary32 floating-point number.
    pub fn float(&mut self) -> Result<f32, BinaryError> {
        let value = self.number()? as f32;
        if !value.is_finite() {
            return self.fail("number exceeds binary32".to_string());
        }
        Ok(value)
    }

    /// Read an integer within bounds.
    pub fn integer(&mut self, minimum: i32, maximum: i32) -> Result<i32, BinaryError> {
        let value = self.number()?;
        if value.fract() != 0.0 || value < f64::from(minimum) || value > f64::from(maximum) {
            return self.fail(format!("integer {value} outside {minimum}..{maximum}"));
        }
        Ok(value as i32)
    }

    /// Read a parenthesized vector.
    pub fn vector(&mut self) -> Result<Vec3, BinaryError> {
        self.expect("(")?;
        let result = vec3(self.float()?, self.float()?, self.float()?);
        self.expect(")")?;
        Ok(result)
    }

    /// Fail on trailing tokens.
    pub fn end(&mut self) -> Result<(), BinaryError> {
        if self.next_token()?.is_some() {
            return self.fail("unexpected trailing token".to_string());
        }
        Ok(())
    }
}

/// Index into a record list (`at`).
///
/// # Panics
///
/// Panics when the index is out of bounds.
pub fn at<'a, T>(values: &'a [T], index: usize, label: &str) -> &'a T {
    values.get(index).unwrap_or_else(|| panic!("Missing {label} {index}"))
}

/// Read records in `kind index` order (`indexedRecords`).
pub fn indexed_records<T>(
    tokens: &mut ModelTokens,
    count: usize,
    kind: &str,
    mut read: impl FnMut(&mut ModelTokens) -> Result<T, BinaryError>,
) -> Result<Vec<T>, BinaryError> {
    use std::collections::HashMap;
    let mut records: HashMap<usize, T> = HashMap::new();
    for _ in 0..count {
        tokens.expect(kind)?;
        let index = tokens.integer(0, count as i32 - 1)? as usize;
        if records.contains_key(&index) {
            return tokens.fail(format!("duplicate {kind} {index}"));
        }
        records.insert(index, read(tokens)?);
    }
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        match records.remove(&index) {
            Some(record) => result.push(record),
            None => return tokens.fail(format!("missing {kind} {index}")),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_skip_comments_and_whitespace() {
        let mut tokens = ModelTokens::new("  // line\nmesh { /* block */ vert 0 ( 1 2 3 ) } \"quoted\" ", "<test>");
        assert_eq!(tokens.token().unwrap(), "mesh");
        assert_eq!(tokens.token().unwrap(), "{");
        assert_eq!(tokens.token().unwrap(), "vert");
        assert_eq!(tokens.integer(0, 100).unwrap(), 0);
        assert_eq!(tokens.vector().unwrap(), vec3(1.0, 2.0, 3.0));
        assert_eq!(tokens.token().unwrap(), "}");
        assert_eq!(tokens.token().unwrap(), "quoted");
        tokens.end().unwrap();
    }

    #[test]
    fn tokens_reject_bad_input() {
        let mut tokens = ModelTokens::new("mesh /* unterminated", "<test>");
        assert_eq!(tokens.token().unwrap(), "mesh");
        let error = tokens.token().unwrap_err();
        assert!(error.message.contains("unterminated comment"), "{}", error.message);
        let mut tokens = ModelTokens::new("\"unterminated", "<test>");
        let error = tokens.token().unwrap_err();
        assert!(error.message.contains("unterminated quoted token"), "{}", error.message);
        let mut tokens = ModelTokens::new("mesh", "<test>");
        tokens.token().unwrap();
        let error = tokens.token().unwrap_err();
        assert!(
            error.message.contains("unexpected end of model text"),
            "{}",
            error.message
        );
        let mut tokens = ModelTokens::new("word", "<test>");
        let error = tokens.number().unwrap_err();
        assert!(error.message.contains("invalid number"), "{}", error.message);
        let mut tokens = ModelTokens::new("1e999", "<test>");
        let error = tokens.float().unwrap_err();
        assert!(error.message.contains("invalid number"), "{}", error.message);
        let mut tokens = ModelTokens::new("1e39", "<test>");
        let error = tokens.float().unwrap_err();
        assert!(error.message.contains("number exceeds binary32"), "{}", error.message);
        let mut tokens = ModelTokens::new("1.5", "<test>");
        let error = tokens.integer(0, 10).unwrap_err();
        assert!(error.message.contains("outside 0..10"), "{}", error.message);
        let mut tokens = ModelTokens::new("mesh 0 5 mesh 0 6", "<test>");
        let error = indexed_records(&mut tokens, 2, "mesh", |tokens| tokens.integer(0, 10)).unwrap_err();
        assert!(error.message.contains("duplicate mesh 0"), "{}", error.message);
    }
}
