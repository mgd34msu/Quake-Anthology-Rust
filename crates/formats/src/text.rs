use crate::FormatError;

pub(crate) struct Tokens<'a> {
    pub bytes: &'a [u8],
    pub at: usize,
}
impl<'a> Tokens<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }
    pub fn next(&mut self) -> Result<Option<&'a [u8]>, FormatError> {
        loop {
            while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
                self.at += 1;
            }
            let rest = &self.bytes[self.at..];
            if rest.starts_with(b"//") {
                self.at += rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
            } else if rest.starts_with(b"/*") {
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
            let length = self.bytes[self.at..]
                .iter()
                .position(|&b| b == b'"')
                .ok_or(FormatError::Truncated)?;
            let result = &self.bytes[self.at..self.at + length];
            self.at += length + 1;
            return Ok(Some(result));
        }
        if !b"{}()".contains(&first) {
            while self
                .bytes
                .get(self.at)
                .is_some_and(|b| !b.is_ascii_whitespace() && !b"{}()".contains(b))
            {
                self.at += 1;
            }
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
