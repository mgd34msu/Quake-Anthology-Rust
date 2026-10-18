//! Command-boundary parsing from the original Q1/Q2/Q3 Cmd tokenizers.
use crate::{conversion::Text, text::MAX_TEXT, views::RuleSetId};
use qa_core::{primitives::CommandArgs, text::FixedText};
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
    pub fn clear(&mut self) {
        self.line.clear();
        self.tokens.len = 0;
        self.tokens.raw = 0..0;
    }
    pub fn parse(&mut self, text: &[u8], source: RuleSetId) -> Result<(), TextError> {
        self.set_raw(text)?;
        tokenize_bytes(self.line.as_bytes(), source, &mut self.tokens)?;
        Ok(())
    }
    /// Capture one command without applying command-buffer separators.
    pub fn set_raw(&mut self, text: &[u8]) -> Result<(), TextError> {
        self.clear();
        if text.contains(&0) {
            return Err(TextError::Nul);
        }
        self.line.append_bytes(text).map_err(|_| TextError::TooLong)
    }
    pub fn parse_engine(&mut self, text: &[u8], source: RuleSetId) -> Result<(), TextError> {
        self.set_raw(text)?;
        tokenize_engine_bytes(self.line.as_bytes(), source, &mut self.tokens)?;
        Ok(())
    }
    pub fn arguments(&self) -> Arguments<'_> {
        self.tokens.arguments(self.line.as_bytes())
    }
    pub fn raw_bytes(&self) -> &[u8] {
        self.line.as_bytes()
    }
}
impl Tokens {
    fn arguments<'a>(&'a self, text: &'a [u8]) -> Arguments<'a> {
        let raw = &text[self.raw.clone()];
        Arguments {
            storage: ArgumentStorage::Parsed { text, tokens: self },
            raw_bytes: raw,
            raw: std::str::from_utf8(raw).unwrap_or(""),
        }
    }
}
#[derive(Clone, Copy)]
enum ArgumentStorage<'a> {
    Parsed { text: &'a [u8], tokens: &'a Tokens },
    Borrowed(CommandArgs<'a>),
}
#[derive(Clone, Copy)]
pub struct Arguments<'a> {
    storage: ArgumentStorage<'a>,
    raw_bytes: &'a [u8],
    /// UTF-8 view for local Console callers; native arguments use raw_bytes().
    pub raw: &'a str,
}
impl Default for Arguments<'_> {
    fn default() -> Self {
        Self::from_command(CommandArgs::default())
    }
}
impl<'a> Arguments<'a> {
    pub fn from_command(args: CommandArgs<'a>) -> Self {
        Self {
            storage: ArgumentStorage::Borrowed(args),
            raw_bytes: args.raw,
            raw: std::str::from_utf8(args.raw).unwrap_or(""),
        }
    }
    pub fn len(&self) -> usize {
        match self.storage {
            ArgumentStorage::Parsed { tokens, .. } => tokens.len,
            ArgumentStorage::Borrowed(args) => args.argv.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(&self, index: usize) -> &'a str {
        std::str::from_utf8(self.get_bytes(index)).unwrap_or("")
    }
    pub fn get_bytes(&self, index: usize) -> &'a [u8] {
        match self.storage {
            ArgumentStorage::Parsed { text, tokens } => tokens
                .spans
                .get(index)
                .filter(|_| index < tokens.len)
                .map_or(&[], |s| &text[s.start as usize..s.end as usize]),
            ArgumentStorage::Borrowed(args) => args.argv.get(index).copied().unwrap_or(&[]),
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = &'a str> + '_ {
        (0..self.len()).map(|i| self.get(i))
    }
    pub fn iter_bytes(&self) -> impl Iterator<Item = &'a [u8]> + '_ {
        (0..self.len()).map(|i| self.get_bytes(i))
    }
    /// Borrow the common ABI argv only for synchronous dispatch. Parsed spans
    /// keep their source bytes; already-projected arguments need no scratch.
    pub fn with_command_args<R>(&self, call: impl FnOnce(CommandArgs<'_>) -> R) -> R {
        if let ArgumentStorage::Borrowed(args) = self.storage {
            return call(args);
        }
        let mut argv = [&[][..]; 1024];
        for (to, from) in argv.iter_mut().zip(self.iter_bytes()) {
            *to = from;
        }
        call(CommandArgs {
            argv: &argv[..self.len()],
            raw: self.raw_bytes(),
        })
    }
    pub fn raw_bytes(&self) -> &'a [u8] {
        self.raw_bytes
    }
    pub fn tail(&self, first: usize) -> Option<&'a str> {
        self.tail_bytes(first)
            .map(|text| std::str::from_utf8(text).unwrap_or(""))
    }
    /// Only parsed spans retain the original suffix at an arbitrary token.
    pub fn tail_bytes(&self, first: usize) -> Option<&'a [u8]> {
        let ArgumentStorage::Parsed { text, tokens } = self.storage else {
            return None;
        };
        Some(if first >= self.len() {
            &[]
        } else if first + 1 == self.len() {
            self.get_bytes(first)
        } else {
            &text[tokens.spans[first].raw as usize..]
        })
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
    let maximum = if source == RuleSetId::Quake3 {
        tokens.spans.len()
    } else {
        80
    };
    tokenize_limit(text, source, tokens, maximum, true)
}
/// Engine commands keep native lexical rules with the full owned token capacity.
pub fn tokenize_engine<'a>(
    text: &'a str,
    source: RuleSetId,
    tokens: &'a mut Tokens,
) -> Result<Arguments<'a>, TextError> {
    tokenize_engine_bytes(text.as_bytes(), source, tokens)
}
pub fn tokenize_engine_bytes<'a>(
    text: &'a [u8],
    source: RuleSetId,
    tokens: &'a mut Tokens,
) -> Result<Arguments<'a>, TextError> {
    let maximum = tokens.spans.len();
    tokenize_limit(text, source, tokens, maximum, false)
}
fn tokenize_limit<'a>(
    text: &'a [u8],
    source: RuleSetId,
    tokens: &'a mut Tokens,
    maximum: usize,
    native_limits: bool,
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
        if native_limits
            && matches!(source, RuleSetId::Quake | RuleSetId::QuakeWorld)
            && value.len() >= 1024
        {
            return Err(TextError::TooLong);
        }
        if native_limits
            && matches!(source, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
            && value.len() >= 128
        {
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
    value: impl FnMut(&str) -> Option<Text<'a>>,
) -> Result<(), TextError> {
    expand_text(text, tokens, 1024, true, value)
}

/// The Q2 macro parser preserves native bytes around UTF-8 registry values.
pub fn expand_bytes<'a>(
    text: &mut FixedText<MAX_TEXT>,
    tokens: &mut Tokens,
    value: impl FnMut(&[u8]) -> Option<Text<'a>>,
) -> Result<(), TextError> {
    expand_limit(text, tokens, 1024, true, value)
}
/// Engine-owned lines use their full buffer while guest macro limits stay native.
pub fn expand_engine<'a>(
    text: &mut FixedText<MAX_TEXT>,
    tokens: &mut Tokens,
    value: impl FnMut(&str) -> Option<Text<'a>>,
) -> Result<(), TextError> {
    expand_text(text, tokens, MAX_TEXT, false, value)
}
pub fn expand_engine_bytes<'a>(
    text: &mut FixedText<MAX_TEXT>,
    tokens: &mut Tokens,
    value: impl FnMut(&[u8]) -> Option<Text<'a>>,
) -> Result<(), TextError> {
    expand_limit(text, tokens, MAX_TEXT, false, value)
}
fn expand_text<'a>(
    text: &mut FixedText<MAX_TEXT>,
    tokens: &mut Tokens,
    maximum: usize,
    native_limits: bool,
    mut value: impl FnMut(&str) -> Option<Text<'a>>,
) -> Result<(), TextError> {
    expand_limit(text, tokens, maximum, native_limits, |name| {
        value(std::str::from_utf8(name).ok()?)
    })
}
fn expand_limit<'a>(
    text: &mut FixedText<MAX_TEXT>,
    tokens: &mut Tokens,
    maximum: usize,
    native_limits: bool,
    mut value: impl FnMut(&[u8]) -> Option<Text<'a>>,
) -> Result<(), TextError> {
    let mut replacements = 0;
    loop {
        let raw = text.as_bytes();
        if raw.len() >= maximum {
            return Err(TextError::TooLong);
        }
        let mut quote = false;
        let mut found: Option<(Range<usize>, Text<'a>)> = None;
        for (at, byte) in raw.iter().copied().enumerate() {
            if byte == b'"' {
                quote = !quote;
            }
            if !quote && byte == b'$' {
                let token_limit = if native_limits {
                    80
                } else {
                    tokens.spans.len()
                };
                let token = tokenize_limit(
                    &raw[at + 1..],
                    RuleSetId::Quake2,
                    tokens,
                    token_limit,
                    native_limits,
                )?;
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
