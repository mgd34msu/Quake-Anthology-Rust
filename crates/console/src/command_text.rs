//! Command-boundary parsing from the original Q1/Q2/Q3 Cmd tokenizers.
use crate::views::Source;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextError {
    TooLong,
    Nul,
    MacroLoop,
    UnmatchedQuote,
}
pub struct Arguments<'a> {
    pub values: Vec<&'a str>,
    pub raw: &'a str,
}
impl<'a> Arguments<'a> {
    pub fn get(&self, index: usize) -> &'a str {
        self.values.get(index).copied().unwrap_or("")
    }
    pub fn tail(&self, first: usize) -> String {
        self.values.get(first..).unwrap_or(&[]).join(" ")
    }
}

pub fn tokenize(text: &str, source: Source) -> Result<Arguments<'_>, TextError> {
    if text.len() > 8192 {
        return Err(TextError::TooLong);
    }
    if text.contains('\0') {
        return Err(TextError::Nul);
    }
    let q3 = source == Source::Quake3;
    let q1 = source == Source::Quake;
    let maximum = if q3 { 1024 } else { 80 };
    let bytes = text.as_bytes();
    let mut at = 0;
    let mut values = Vec::with_capacity(maximum.min(text.len() + 1));
    let mut args_start = text.len();
    while at < bytes.len() {
        while bytes
            .get(at)
            .is_some_and(|&b| b <= 32 && (q3 || b != b'\n'))
        {
            at += 1;
        }
        if at == bytes.len() || (!q3 && bytes[at] == b'\n') {
            break;
        }
        if values.len() == 1 {
            args_start = at;
        }
        if q3 && bytes[at..].starts_with(b"//") {
            break;
        }
        if !q3 {
            loop {
                while bytes.get(at).is_some_and(|&b| b <= 32) {
                    at += 1;
                }
                if !bytes.get(at..).is_some_and(|s| s.starts_with(b"//")) {
                    break;
                }
                at += bytes[at..]
                    .iter()
                    .position(|&b| b == b'\n')
                    .unwrap_or(bytes.len() - at);
            }
            if at == bytes.len() {
                break;
            }
        }
        if q3 && bytes[at..].starts_with(b"/*") {
            let Some(length) = bytes[at + 2..].windows(2).position(|w| w == b"*/") else {
                break;
            };
            at += length + 4;
            continue;
        }
        let first = bytes[at];
        let start;
        let end;
        if first == b'"' {
            at += 1;
            start = at;
            while at < bytes.len() && bytes[at] != b'"' {
                at += 1;
            }
            end = at;
            at += usize::from(at < bytes.len());
        } else {
            start = at;
            if q1 && b"{}()':".contains(&first) {
                at += 1;
            } else {
                while at < bytes.len() && bytes[at] > 32 {
                    if (q1 && b"{}()':".contains(&bytes[at]))
                        || (q3
                            && (bytes[at] == b'"'
                                || bytes[at..].starts_with(b"//")
                                || bytes[at..].starts_with(b"/*")))
                    {
                        break;
                    }
                    at += 1;
                }
            }
            end = at;
        }
        let mut value = &text[start..end];
        if matches!(source, Source::Quake | Source::QuakeWorld) && value.len() >= 1024 {
            return Err(TextError::TooLong);
        }
        if matches!(source, Source::Quake2 | Source::Quake2Rerelease) && value.len() >= 128 {
            if first == b'"' {
                return Err(TextError::TooLong);
            }
            value = "";
        }
        if values.len() < maximum {
            values.push(value);
        }
        if q3 && values.len() == maximum {
            break;
        }
    }
    let mut raw = &text[args_start..];
    if matches!(source, Source::Quake2 | Source::Quake2Rerelease) {
        raw = raw.trim_end_matches(|c: char| c.is_ascii() && c <= ' ');
    }
    Ok(Arguments { values, raw })
}

/// Q2 expands outside quotes, repeats substituted macros, and scopes loops to
/// the command. The lookup is a cold command-boundary operation.
pub fn expand(
    text: &str,
    mut value: impl FnMut(&str) -> Option<String>,
) -> Result<String, TextError> {
    let mut text = text.to_owned();
    let mut replacements = 0;
    loop {
        if text.len() >= 1024 {
            return Err(TextError::TooLong);
        }
        let mut quote = false;
        let mut found = None;
        for (at, &byte) in text.as_bytes().iter().enumerate() {
            if byte == b'"' {
                quote = !quote;
            }
            if !quote && byte == b'$' {
                let after = &text[at + 1..];
                let token = tokenize(after, Source::Quake2)?;
                if let Some(name) = token.values.first() {
                    let start = name.as_ptr() as usize - text.as_ptr() as usize;
                    let mut end = start + name.len();
                    if text.as_bytes().get(start.wrapping_sub(1)) == Some(&b'"')
                        && text.as_bytes().get(end) == Some(&b'"')
                    {
                        end += 1;
                    }
                    found = Some((at..end, value(name).unwrap_or_default()));
                    break;
                }
            }
        }
        if let Some((range, replacement)) = found {
            replacements += 1;
            if replacements >= 100 {
                return Err(TextError::MacroLoop);
            }
            text.replace_range(range, &replacement);
        } else {
            return if quote {
                Err(TextError::UnmatchedQuote)
            } else {
                Ok(text)
            };
        }
    }
}
