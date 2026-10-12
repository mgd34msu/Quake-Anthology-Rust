//! Command-boundary parsing from the original Q1/Q2/Q3 Cmd tokenizers.
use crate::{conversion::Text, text::MAX_TEXT, views::RuleSetId};
use qa_core::text::FixedText;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextError {
    TooLong,
    Nul,
    MacroLoop,
    UnmatchedQuote,
}
#[derive(Clone, Copy, Default)]
struct Span {
    start: u16,
    end: u16,
    raw: u16,
}
pub struct Tokens {
    spans: [Span; 1024],
    len: usize,
    raw: Range<usize>,
}
impl Default for Tokens {
    fn default() -> Self {
        Self {
            spans: [Span::default(); 1024],
            len: 0,
            raw: 0..0,
        }
    }
}

/// The console and a selected native CLIENT command share this bounded parser.
/// Token spans refer to its current line, never to retained packet storage.
#[derive(Default)]
pub struct ParsedCommand {
    pub(crate) line: FixedText<MAX_TEXT>,
    pub(crate) tokens: Tokens,
}
impl ParsedCommand {
    pub fn parse(&mut self, text: &[u8], source: RuleSetId) -> Result<(), TextError> {
        self.line.clear();
        self.tokens.len = 0;
        self.tokens.raw = 0..0;
        self.line
            .append_bytes(text)
            .map_err(|_| TextError::TooLong)?;
        tokenize_bytes(self.line.as_bytes(), source, &mut self.tokens)?;
        Ok(())
    }
    pub fn arguments(&self) -> Arguments<'_> {
        self.tokens.arguments(self.line.as_bytes())
    }
}
impl Tokens {
    fn arguments<'a>(&'a self, text: &'a [u8]) -> Arguments<'a> {
        let raw = &text[self.raw.clone()];
        Arguments {
            text,
            tokens: self,
            raw_bytes: raw,
            raw: std::str::from_utf8(raw).unwrap_or(""),
        }
    }
}
pub struct Arguments<'a> {
    text: &'a [u8],
    tokens: &'a Tokens,
    raw_bytes: &'a [u8],
    /// UTF-8 view for local Console callers; native arguments use raw_bytes().
    pub raw: &'a str,
}
impl<'a> Arguments<'a> {
    pub fn len(&self) -> usize {
        self.tokens.len
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(&self, index: usize) -> &'a str {
        std::str::from_utf8(self.get_bytes(index)).unwrap_or("")
    }
    pub fn get_bytes(&self, index: usize) -> &'a [u8] {
        self.tokens
            .spans
            .get(index)
            .filter(|_| index < self.len())
            .map_or(&[], |s| &self.text[s.start as usize..s.end as usize])
    }
    pub fn iter(&self) -> impl Iterator<Item = &'a str> + '_ {
        (0..self.len()).map(|i| self.get(i))
    }
    pub fn iter_bytes(&self) -> impl Iterator<Item = &'a [u8]> + '_ {
        (0..self.len()).map(|i| self.get_bytes(i))
    }
    pub fn raw_bytes(&self) -> &'a [u8] {
        self.raw_bytes
    }
    pub fn tail(&self, first: usize) -> &'a str {
        std::str::from_utf8(self.tail_bytes(first)).unwrap_or("")
    }
    pub fn tail_bytes(&self, first: usize) -> &'a [u8] {
        if first >= self.len() {
            &[]
        } else if first + 1 == self.len() {
            self.get_bytes(first)
        } else {
            &self.text[self.tokens.spans[first].raw as usize..]
        }
    }
    pub fn join<const N: usize>(
        &self,
        first: usize,
        output: &mut FixedText<N>,
    ) -> Result<(), TextError> {
        self.join_bytes(first, output)
    }
    pub fn join_bytes<const N: usize>(
        &self,
        first: usize,
        output: &mut FixedText<N>,
    ) -> Result<(), TextError> {
        output.clear();
        for i in first..self.len() {
            if i != first {
                output.append_bytes(b" ").map_err(|_| TextError::TooLong)?;
            }
            output
                .append_bytes(self.get_bytes(i))
                .map_err(|_| TextError::TooLong)?;
        }
        Ok(())
    }
}

pub fn tokenize<'a>(
    text: &'a str,
    source: RuleSetId,
    tokens: &'a mut Tokens,
) -> Result<Arguments<'a>, TextError> {
    tokenize_bytes(text.as_bytes(), source, tokens)
}

/// One tokenizer borrows native command bytes without sanitizing or recoding.
pub fn tokenize_bytes<'a>(
    text: &'a [u8],
    source: RuleSetId,
    tokens: &'a mut Tokens,
) -> Result<Arguments<'a>, TextError> {
    tokens.len = 0;
    tokens.raw = 0..0;
    if text.len() > 8192 {
        return Err(TextError::TooLong);
    }
    if text.contains(&0) {
        return Err(TextError::Nul);
    }
    let q3 = source == RuleSetId::Quake3;
    let q1 = source == RuleSetId::Quake;
    let maximum = if q3 { 1024 } else { 80 };
    let bytes = text;
    let mut at = 0;
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
        if tokens.len == 1 {
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
        let raw_start = at;
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
        if matches!(source, RuleSetId::Quake | RuleSetId::QuakeWorld) && value.len() >= 1024 {
            return Err(TextError::TooLong);
        }
        if matches!(source, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease) && value.len() >= 128 {
            if first == b'"' {
                return Err(TextError::TooLong);
            }
            value = &[];
        }
        if tokens.len < maximum {
            tokens.spans[tokens.len] = Span {
                start: start as u16,
                end: (start + value.len()) as u16,
                raw: raw_start as u16,
            };
            tokens.len += 1;
        }
        if q3 && tokens.len == maximum {
            break;
        }
    }
    let mut raw = &text[args_start..];
    if matches!(source, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease) {
        while raw.last().is_some_and(|&byte| byte <= b' ') {
            raw = &raw[..raw.len() - 1];
        }
    }
    tokens.raw = args_start..args_start + raw.len();
    Ok(tokens.arguments(text))
}

/// Q2 expands outside quotes, repeats substituted macros, and scopes loops to
/// the command. The lookup is a cold command-boundary operation.
pub fn expand<'a>(
    text: &mut FixedText<MAX_TEXT>,
    tokens: &mut Tokens,
    mut value: impl FnMut(&str) -> Option<Text<'a>>,
) -> Result<(), TextError> {
    expand_bytes(text, tokens, |name| value(std::str::from_utf8(name).ok()?))
}

/// The Q2 macro parser preserves native bytes around UTF-8 registry values.
pub fn expand_bytes<'a>(
    text: &mut FixedText<MAX_TEXT>,
    tokens: &mut Tokens,
    mut value: impl FnMut(&[u8]) -> Option<Text<'a>>,
) -> Result<(), TextError> {
    let mut replacements = 0;
    loop {
        let raw = text.as_bytes();
        if raw.len() >= 1024 {
            return Err(TextError::TooLong);
        }
        let mut quote = false;
        let mut found: Option<(Range<usize>, Text<'a>)> = None;
        for (at, byte) in raw.iter().copied().enumerate() {
            if byte == b'"' {
                quote = !quote;
            }
            if !quote && byte == b'$' {
                let token = tokenize_bytes(&raw[at + 1..], RuleSetId::Quake2, tokens)?;
                if !token.is_empty() {
                    let name = token.get_bytes(0);
                    let start = name.as_ptr() as usize - raw.as_ptr() as usize;
                    let mut end = start + name.len();
                    if raw.get(start.wrapping_sub(1)) == Some(&b'"') && raw.get(end) == Some(&b'"')
                    {
                        end += 1;
                    }
                    found = Some((at..end, value(name).unwrap_or(Text::Borrowed(""))));
                    break;
                }
            }
        }
        if let Some((range, replacement)) = found {
            replacements += 1;
            if replacements >= 100 {
                return Err(TextError::MacroLoop);
            }
            text.replace_bytes(range, replacement.as_str().as_bytes())
                .map_err(|_| TextError::TooLong)?;
        } else {
            return if quote {
                Err(TextError::UnmatchedQuote)
            } else {
                Ok(())
            };
        }
    }
}
