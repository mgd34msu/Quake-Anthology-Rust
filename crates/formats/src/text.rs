use crate::FormatError;

pub struct Tokens<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) at: usize,
    punctuation: &'static [u8],
    block_comments: bool,
    line_comments: bool,
    limit: usize,
    quoted_escapes: bool,
    comma: bool,
}
impl<'a> Tokens<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            at: 0,
            punctuation: b"{}()",
            block_comments: true,
            line_comments: true,
            limit: usize::MAX,
            quoted_escapes: false,
            comma: false,
        }
    }
    pub fn entity(
        bytes: &'a [u8],
        punctuation: &'static [u8],
        block_comments: bool,
        limit: usize,
    ) -> Self {
        Self {
            bytes,
            at: 0,
            punctuation,
            block_comments,
            line_comments: true,
            limit,
            quoted_escapes: false,
            comma: false,
        }
    }
    /// Whitespace/quoted data words, with paths and punctuation kept literal.
    pub fn words(bytes: &'a [u8]) -> Self {
        Self {
            line_comments: false,
            ..Self::entity(bytes, b"", false, usize::MAX)
        }
    }
    pub fn quoted_escapes(mut self) -> Self {
        self.quoted_escapes = true;
        self
    }
    pub fn comma(bytes: &'a [u8]) -> Self {
        Self {
            comma: true,
            limit: 1024,
            punctuation: b",",
            ..Self::new(bytes)
        }
    }
    fn whitespace(&self, byte: u8) -> bool {
        byte <= 32 || (self.comma && byte >= 128)
    }
    pub fn next(&mut self) -> Result<Option<&'a [u8]>, FormatError> {
        loop {
            while self.bytes.get(self.at).is_some_and(|&b| self.whitespace(b)) {
                self.at += 1;
            }
            let rest = &self.bytes[self.at..];
            if self.line_comments && rest.starts_with(b"//") {
                self.at += rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
            } else if self.block_comments && rest.starts_with(b"/*") {
                let length = rest[2..]
                    .windows(2)
                    .position(|p| p == b"*/")
                    .ok_or(FormatError::Truncated)?;
                self.at += length + 4;
            } else {
                break;
            }
        }
        let Some(&first) = self.bytes.get(self.at) else {
            return Ok(None);
        };
        let start = self.at;
        self.at += 1;
        if first == b'"' {
            let mut length = 0;
            loop {
                match self.bytes.get(self.at + length) {
                    Some(b'"') => break,
                    Some(b'\\') if self.quoted_escapes => length += 2,
                    Some(_) => length += 1,
                    None => return Err(FormatError::Truncated),
                }
            }
            let result = &self.bytes[self.at..self.at + length];
            if result.len() >= self.limit {
                return Err(FormatError::InvalidRange);
            }
            self.at += length + 1;
            return Ok(Some(result));
        }
        if self.comma || !self.punctuation.contains(&first) {
            while self
                .bytes
                .get(self.at)
                .is_some_and(|&b| !self.whitespace(b) && !self.punctuation.contains(&b))
            {
                self.at += 1;
            }
        }
        if self.at - start >= self.limit {
            if self.comma {
                return Ok(Some(b""));
            }
            return Err(FormatError::InvalidRange);
        }
        Ok(Some(&self.bytes[start..self.at]))
    }
    pub fn require(&mut self, token: &[u8]) -> Result<(), FormatError> {
        if self.next()? == Some(token) {
            Ok(())
        } else {
            Err(FormatError::InvalidValue)
        }
    }
    pub fn value(&mut self) -> Result<&'a [u8], FormatError> {
        self.next()?.ok_or(FormatError::Truncated)
    }
    pub fn number(&mut self) -> Result<f64, FormatError> {
        let value = std::str::from_utf8(self.value()?)
            .map_err(|_| FormatError::InvalidValue)?
            .parse::<f64>()
            .map_err(|_| FormatError::InvalidValue)?;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(FormatError::InvalidValue)
        }
    }
    pub fn integer(&mut self, min: i32, max: i32) -> Result<i32, FormatError> {
        let value = self.number()?;
        if value < f64::from(min) || value > f64::from(max) || value.trunc() != value {
            return Err(FormatError::InvalidRange);
        }
        Ok(value as i32)
    }
    pub fn scalar(&mut self) -> Result<f32, FormatError> {
        let value = self.number()? as f32;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(FormatError::InvalidValue)
        }
    }
}
