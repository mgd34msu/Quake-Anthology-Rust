//! Command text parsing ported from `src/core/commands/text.ts` (Quake
//! `common.c`, Q2 `q_shared.c`/`cmd.c`, Q3 `cmd.c`) with dialects from
//! `src/contracts/common.ts`.
//!
//! This module owns the pure text layer: source validation, tokenizing,
//! command splitting, macro expansion, and argument tails. The buffered
//! queue in [`crate::cmd_buffer`] builds on these primitives.

use thiserror::Error;

/// Error for invalid command text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CmdError {
    /// A token overflowed its source buffer.
    #[error("{0}")]
    TokenOverflow(String),
    /// Token storage overflowed `cmd_tokenized`.
    #[error("Command tokens overflow source cmd_tokenized")]
    TokenizedOverflow,
    /// A prefix length must be a nonnegative integer.
    #[error("Command prefix length must be a nonnegative integer")]
    BadPrefix,
}

/// Command and cvar dialect. Dialects are independent of the selected
/// network codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// Quake NetQuake console.
    Q1Netquake,
    /// QuakeWorld console.
    Q1Quakeworld,
    /// Classic Quake II console.
    Q2Classic,
    /// Rerelease Quake II console.
    Q2Rerelease,
    /// Quake III console.
    Q3,
}

impl Dialect {
    /// Whether this is a Quake I family dialect.
    #[must_use]
    pub fn is_q1(self) -> bool {
        matches!(self, Dialect::Q1Netquake | Dialect::Q1Quakeworld)
    }

    /// Whether this is a Quake II family dialect.
    #[must_use]
    pub fn is_q2(self) -> bool {
        matches!(self, Dialect::Q2Classic | Dialect::Q2Rerelease)
    }
}

/// How high bytes lex: source text skips them (original signed-char
/// lexers), direct console input keeps extended glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMode {
    /// Parsed source text (scripts, configs).
    Source,
    /// Direct console input.
    Console,
}

/// Engine byte string: one byte per unit, NUL-truncated, the qsrc
/// `char[]` command/cvar text model. Bytes `0x80..=0xFF` are single
/// units (extended console glyphs), never multi-byte sequences, so
/// [`EngineText::len`] is the length the `8192`/`16384` buffer limits
/// and the `1024` line limit measure.
///
/// UTF-8 conversion happens only at the boundaries: [`EngineText::from`]
/// maps host `&str` in (chars `<= 255` map exactly, anything above
/// truncates to its low byte) and [`EngineText::to_display`] maps out
/// to the UI/font layer (Latin-1 bytes to chars, exact for `0..=255`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineText(Vec<u8>);

impl EngineText {
    /// Empty engine text.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Copy engine bytes, truncating at the first NUL like qsrc
    /// `strcpy` into `cmd_text`.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
        Self(bytes[..end].to_vec())
    }

    /// Borrow the engine bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Engine length in bytes: every byte counts once.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no bytes are queued.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Append one engine byte.
    pub fn push(&mut self, byte: u8) {
        self.0.push(byte);
    }

    /// Append another engine string's bytes.
    pub fn push_text(&mut self, other: &EngineText) {
        self.0.extend_from_slice(other.as_bytes());
    }

    /// Shorten to at most `len` bytes; any byte index is a valid cut.
    pub fn truncate(&mut self, len: usize) {
        self.0.truncate(len);
    }

    /// Split off the bytes at and after `at`; any byte index is valid.
    #[must_use]
    pub fn split_off(&mut self, at: usize) -> EngineText {
        Self(self.0.split_off(at))
    }

    /// Render for the UI/font boundary: each byte becomes the Latin-1
    /// char with that value, so engine bytes round-trip byte-exactly.
    #[must_use]
    pub fn to_display(&self) -> String {
        self.0.iter().map(|byte| char::from(*byte)).collect()
    }
}

impl From<&str> for EngineText {
    /// Map host text to engine bytes: truncate at the first NUL, then
    /// take each char's low byte. Chars `<= 255` map exactly (so
    /// `to_display` inverts this for engine-range text); anything above
    /// is host text the byte engine cannot name and truncates.
    fn from(input: &str) -> Self {
        let mut bytes = Vec::with_capacity(input.len());
        for c in input.chars() {
            if c == '\0' {
                break;
            }
            bytes.push(c as u8);
        }
        Self(bytes)
    }
}

impl From<Vec<u8>> for EngineText {
    /// Take ownership of engine bytes, truncating at the first NUL.
    fn from(mut bytes: Vec<u8>) -> Self {
        if let Some(end) = bytes.iter().position(|byte| *byte == 0) {
            bytes.truncate(end);
        }
        Self(bytes)
    }
}

/// Truncate at the first NUL and map host text to engine byte text.
/// Chars `<= 255` map exactly; anything above truncates to its low byte
/// (the UTF-8 boundary mapping; see [`EngineText`]).
pub fn source_command_text(input: &str) -> String {
    EngineText::from(input).to_display()
}

/// Fold ASCII uppercase to lowercase (Quake III name comparison).
#[must_use]
pub fn ascii_fold(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_uppercase() {
                char::from(c as u8 + 32)
            } else {
                c
            }
        })
        .collect()
}

fn is_whitespace(byte: u8, mode: TextMode) -> bool {
    byte <= 32 || (mode == TextMode::Source && byte >= 128)
}

/// Byte offset of the next command separator (`;` outside quotes, newline,
/// or Q3 carriage return), or the text length when there is none.
/// Separators are ASCII, so scanning host bytes finds the same offsets
/// as scanning chars, and the returned index is always a valid split.
#[must_use]
pub fn command_separator_offset(text: &str, dialect: Dialect) -> usize {
    command_separator_offset_bytes(text.as_bytes(), dialect)
}

/// Byte offset of the next command separator in engine bytes, or the
/// byte length when there is none.
#[must_use]
pub fn command_separator_offset_bytes(bytes: &[u8], dialect: Dialect) -> usize {
    let mut quoted = false;
    for (offset, byte) in bytes.iter().enumerate() {
        if *byte == b'"' {
            quoted = !quoted;
        }
        if (!quoted && *byte == b';') || *byte == b'\n' || (dialect == Dialect::Q3 && *byte == b'\r') {
            return offset;
        }
    }
    bytes.len()
}

struct ParsedToken {
    value: EngineText,
    end: usize,
}

fn parse_token(bytes: &[u8], start: usize, dialect: Dialect, mode: TextMode) -> Result<Option<ParsedToken>, CmdError> {
    let mut offset = start;
    loop {
        while offset < bytes.len() && is_whitespace(bytes[offset], mode) {
            offset += 1;
        }
        if offset + 1 < bytes.len() && bytes[offset] == b'/' && bytes[offset + 1] == b'/' {
            if dialect == Dialect::Q3 {
                return Ok(None);
            }
            while offset < bytes.len() && bytes[offset] != b'\n' {
                offset += 1;
            }
            continue;
        }
        if dialect == Dialect::Q3 && offset + 1 < bytes.len() && bytes[offset] == b'/' && bytes[offset + 1] == b'*' {
            let mut end = offset + 2;
            while end + 1 < bytes.len() && !(bytes[end] == b'*' && bytes[end + 1] == b'/') {
                end += 1;
            }
            offset = if end + 1 < bytes.len() { end + 2 } else { bytes.len() };
            continue;
        }
        break;
    }
    if offset >= bytes.len() {
        return Ok(None);
    }
    let quoted = bytes[offset] == b'"';
    if quoted {
        offset += 1;
    }
    let token_start = offset;
    if quoted {
        while offset < bytes.len() && bytes[offset] != b'"' {
            offset += 1;
        }
    } else if dialect == Dialect::Q1Netquake && matches!(bytes[offset], b'{' | b'}' | b'(' | b')' | b'\'' | b':') {
        offset += 1;
    } else {
        while offset < bytes.len() && !is_whitespace(bytes[offset], mode) {
            if dialect == Dialect::Q1Netquake && matches!(bytes[offset], b'{' | b'}' | b'(' | b')' | b'\'' | b':') {
                break;
            }
            if dialect == Dialect::Q3
                && (bytes[offset] == b'"'
                    || (offset + 1 < bytes.len() && bytes[offset] == b'/' && matches!(bytes[offset + 1], b'/' | b'*')))
            {
                break;
            }
            offset += 1;
        }
    }
    let mut value = EngineText::from_bytes(&bytes[token_start..offset]);
    if dialect.is_q2() && value.len() >= 128 {
        if quoted {
            return Err(CmdError::TokenOverflow(
                "Quoted command token overflows source MAX_TOKEN_CHARS".to_string(),
            ));
        }
        value = EngineText::new();
    } else if dialect.is_q1() && value.len() >= 1024 {
        return Err(CmdError::TokenOverflow(
            "Command token overflows source com_token".to_string(),
        ));
    }
    if quoted && offset < bytes.len() && bytes[offset] == b'"' {
        offset += 1;
    }
    Ok(Some(ParsedToken { value, end: offset }))
}

/// Expand unquoted `$cvar` macros Q2-style, repeatedly. Returns `None` when
/// the line is discarded (overlong, macro loop, or unmatched quote); each
/// discard prints its donor message through `print`.
pub fn expand_command_macros(
    input: &str,
    variable: &dyn Fn(&str) -> String,
    print: &mut dyn FnMut(&str),
    mode: TextMode,
) -> Result<Option<String>, CmdError> {
    let host = EngineText::from(input);
    Ok(expand_command_macros_bytes(host.as_bytes(), variable, print, mode)?.map(|text| text.to_display()))
}

/// Expand unquoted `$cvar` macros over engine bytes. The macro budget
/// counts bytes, matching Q2 `MAX_STRING_CHARS`.
pub(crate) fn expand_command_macros_bytes(
    input: &[u8],
    variable: &dyn Fn(&str) -> String,
    print: &mut dyn FnMut(&str),
    mode: TextMode,
) -> Result<Option<EngineText>, CmdError> {
    let mut text = EngineText::from_bytes(input);
    let mut budget = text.len();
    if budget >= 1024 {
        print("Line exceeded 1024 chars, discarded.\n");
        return Ok(None);
    }
    let mut quoted = false;
    let mut count = 0;
    let mut offset = 0;
    loop {
        let bytes = text.as_bytes();
        if offset >= bytes.len() {
            break;
        }
        if bytes[offset] == b'"' {
            quoted = !quoted;
        }
        if !quoted && bytes[offset] == b'$' {
            let token = parse_token(bytes, offset + 1, Dialect::Q2Classic, mode)?;
            if let Some(token) = token {
                let value = EngineText::from(variable(&token.value.to_display()).as_str());
                budget += value.len();
                if budget >= 1024 {
                    print("Expanded line exceeded 1024 chars, discarded.\n");
                    return Ok(None);
                }
                let mut expanded = EngineText::from_bytes(&bytes[..offset]);
                expanded.push_text(&value);
                expanded.push_text(&EngineText::from_bytes(&bytes[token.end..]));
                text = expanded;
                count += 1;
                if count == 100 {
                    print("Macro expansion loop, discarded.\n");
                    return Ok(None);
                }
                continue;
            }
        }
        offset += 1;
    }
    if quoted {
        print("Line has unmatched quote, discarded.\n");
        return Ok(None);
    }
    Ok(Some(text))
}

/// Tokenized command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandTokens {
    /// Argument vector.
    pub argv: Vec<String>,
    /// Raw text after the first token (Q3 joins tokenized arguments).
    pub args_text: String,
}

/// Retain quotes and punctuation when a dispatcher removes its own leading
/// tokens: the tail after skipping `count` tokens.
pub fn command_text_tail(input: &str, dialect: Dialect, count: usize) -> Result<String, CmdError> {
    let host = EngineText::from(input);
    Ok(command_text_tail_bytes(host.as_bytes(), dialect, count)?.to_display())
}

/// Tail after skipping `count` tokens, over engine bytes.
pub(crate) fn command_text_tail_bytes(input: &[u8], dialect: Dialect, count: usize) -> Result<EngineText, CmdError> {
    let bytes = EngineText::from_bytes(input);
    let bytes = bytes.as_bytes();
    let mut offset = 0;
    for _ in 0..count {
        while offset < bytes.len() && is_whitespace(bytes[offset], TextMode::Source) {
            offset += 1;
        }
        let token = parse_token(bytes, offset, dialect, TextMode::Source)?;
        if token.is_none() {
            return Ok(EngineText::new());
        }
        offset = token.map_or(offset, |token| token.end);
    }
    while offset < bytes.len() && is_whitespace(bytes[offset], TextMode::Source) {
        offset += 1;
    }
    Ok(EngineText::from_bytes(&bytes[offset..]))
}

/// Tokenize one command line. Q2 trims trailing control bytes from the
/// argument text; Q3 rebuilds it from tokenized arguments.
pub fn tokenize_command(input: &str, dialect: Dialect, mode: TextMode) -> Result<CommandTokens, CmdError> {
    let host = EngineText::from(input);
    let (argv, args_text) = tokenize_command_bytes(host.as_bytes(), dialect, mode)?;
    Ok(CommandTokens {
        argv: argv.iter().map(EngineText::to_display).collect(),
        args_text: args_text.to_display(),
    })
}

/// Tokenize one command line over engine bytes: argument vector plus the
/// raw text after the first token. Token storage counts bytes.
pub(crate) fn tokenize_command_bytes(
    input: &[u8],
    dialect: Dialect,
    mode: TextMode,
) -> Result<(Vec<EngineText>, EngineText), CmdError> {
    let text = EngineText::from_bytes(input);
    let bytes = text.as_bytes();
    let maximum_tokens = if dialect == Dialect::Q3 { 1024 } else { 80 };
    let mut argv: Vec<EngineText> = Vec::new();
    let mut args_text = EngineText::new();
    let mut stored_bytes = 0;
    let mut offset = 0;
    while offset < bytes.len() {
        while offset < bytes.len()
            && is_whitespace(bytes[offset], mode)
            && (dialect == Dialect::Q3 || bytes[offset] != b'\n')
        {
            offset += 1;
        }
        if dialect != Dialect::Q3 && offset < bytes.len() && bytes[offset] == b'\n' {
            break;
        }
        if argv.len() == 1 {
            let mut tail = EngineText::from_bytes(&bytes[offset..]);
            if dialect.is_q2() {
                let mut end = tail.len();
                while end > 0 && tail.as_bytes()[end - 1] <= 32 {
                    end -= 1;
                }
                tail.truncate(end);
            }
            args_text = tail;
        }
        let token = parse_token(bytes, offset, dialect, mode)?;
        let Some(token) = token else { break };
        offset = token.end;
        if argv.len() < maximum_tokens {
            stored_bytes += token.value.len() + 1;
            if dialect == Dialect::Q3 && stored_bytes > 9216 {
                return Err(CmdError::TokenizedOverflow);
            }
            argv.push(token.value);
        }
        if dialect == Dialect::Q3 && argv.len() == maximum_tokens {
            break;
        }
    }
    if dialect == Dialect::Q3 {
        let mut joined = EngineText::new();
        for (index, argument) in argv.iter().skip(1).enumerate() {
            if index > 0 {
                joined.push(b' ');
            }
            joined.push_text(argument);
        }
        args_text = joined;
    }
    Ok((argv, args_text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_text_validates_bytes() {
        assert_eq!(source_command_text("say hi\0trailing"), "say hi".to_string());
        assert_eq!(source_command_text("caf\u{20ac}"), "caf\u{ac}".to_string());
        assert_eq!(ascii_fold("Sv_Cheats"), "sv_cheats");
    }

    #[test]
    fn separators_respect_quotes_and_dialect() {
        assert_eq!(command_separator_offset("say \"a;b\"; wait", Dialect::Q2Classic), 9);
        assert_eq!(command_separator_offset("a\rb", Dialect::Q3), 1);
        assert_eq!(command_separator_offset("a\rb", Dialect::Q2Classic), 3);
        assert_eq!(command_separator_offset("plain", Dialect::Q3), 5);
    }

    #[test]
    fn engine_text_counts_every_byte_once() {
        let text = EngineText::from_bytes(b"say \x80\xff\n");
        assert_eq!(text.len(), 7);
        assert_eq!(text.as_bytes(), b"say \x80\xff\n");
        assert_eq!(command_separator_offset_bytes(text.as_bytes(), Dialect::Q2Classic), 6);
        let mut text = text;
        text.push(b'!');
        assert_eq!(text.len(), 8);
        text.truncate(7);
        assert_eq!(text.as_bytes(), b"say \x80\xff\n");
        let rest = text.split_off(4);
        assert_eq!(text.as_bytes(), b"say ");
        assert_eq!(rest.as_bytes(), b"\x80\xff\n");
        assert!(EngineText::new().is_empty());
        assert!(!rest.is_empty());
    }

    #[test]
    fn engine_text_truncates_at_nul() {
        assert_eq!(EngineText::from_bytes(b"ab\0cd").as_bytes(), b"ab");
        assert_eq!(EngineText::from("ab\0cd").as_bytes(), b"ab");
        assert_eq!(EngineText::from(vec![b'a', 0, b'b']).as_bytes(), b"a");
    }

    #[test]
    fn engine_text_round_trips_bytes_through_display() {
        let bytes: Vec<u8> = (1u8..=255).collect();
        let text = EngineText::from(bytes.clone());
        assert_eq!(text.len(), 255);
        assert_eq!(
            EngineText::from(text.to_display().as_str()).as_bytes(),
            bytes.as_slice()
        );
        assert_eq!(EngineText::from("caf\u{20ac}").as_bytes(), b"caf\xac");
    }

    #[test]
    fn tokenizer_follows_dialect_rules() {
        let tokens = tokenize_command("say \"hello world\" // comment", Dialect::Q2Classic, TextMode::Source).unwrap();
        assert_eq!(tokens.argv, vec!["say", "hello world"]);
        let tokens = tokenize_command("say hello // comment", Dialect::Q3, TextMode::Source).unwrap();
        assert_eq!(tokens.argv, vec!["say", "hello"]);
        assert_eq!(tokens.args_text, "hello");
        let tokens = tokenize_command("say /* x */ hi", Dialect::Q3, TextMode::Source).unwrap();
        assert_eq!(tokens.argv, vec!["say", "hi"]);
        let tokens = tokenize_command("bind x \"+attack\" \n", Dialect::Q2Classic, TextMode::Source).unwrap();
        assert_eq!(tokens.args_text, "x \"+attack\"");
        let tokens = tokenize_command("dprint {value}", Dialect::Q1Netquake, TextMode::Source).unwrap();
        assert_eq!(tokens.argv, vec!["dprint", "{", "value", "}"]);
        assert_eq!(
            command_text_tail("give health 100", Dialect::Q3, 1).unwrap(),
            "health 100"
        );
        assert_eq!(
            command_text_tail("give \"a b\" 100", Dialect::Q3, 1).unwrap(),
            "\"a b\" 100"
        );
    }

    #[test]
    fn tokenizer_keeps_high_bytes_byte_exact() {
        // Source mode keeps qsrc signed-char semantics: bare bytes >= 0x80
        // are separators, while quoted high bytes stay byte-exact.
        let tokens = tokenize_command("say \"\u{80}aÿ\" \u{ff}", Dialect::Q2Classic, TextMode::Source).unwrap();
        assert_eq!(tokens.argv, vec!["say".to_string(), "\u{80}aÿ".to_string()]);
        assert_eq!(tokens.args_text, "\"\u{80}aÿ\" \u{ff}");
        // Console mode treats high bytes as single content units.
        let (argv, args) = tokenize_command_bytes(b"say \x80\xff", Dialect::Q2Classic, TextMode::Console).unwrap();
        assert_eq!(argv[1].as_bytes(), b"\x80\xff");
        assert_eq!(args.as_bytes(), b"\x80\xff");
        assert_eq!(
            command_text_tail("give \u{80}health 100", Dialect::Q3, 1).unwrap(),
            "health 100"
        );
    }

    #[test]
    fn macro_expansion_matches_q2() {
        let mut printed = Vec::new();
        let expanded = expand_command_macros(
            "say $name !",
            &|name| {
                if name == "name" {
                    "bob".to_string()
                } else {
                    String::new()
                }
            },
            &mut |text| printed.push(text.to_string()),
            TextMode::Source,
        )
        .unwrap();
        assert_eq!(expanded, Some("say bob !".to_string()));
        let quoted = expand_command_macros(
            "say \"$name\"",
            &|_| "bob".to_string(),
            &mut |text| printed.push(text.to_string()),
            TextMode::Source,
        )
        .unwrap();
        assert_eq!(quoted, Some("say \"$name\"".to_string()));
        let looping = expand_command_macros(
            "say $loop",
            &|_| "$loop".to_string(),
            &mut |text| printed.push(text.to_string()),
            TextMode::Source,
        )
        .unwrap();
        assert_eq!(looping, None);
        assert!(printed.iter().any(|line| line.contains("Macro expansion loop")));
    }

    #[test]
    fn macro_expansion_splices_high_bytes() {
        let mut printed = Vec::new();
        let expanded = expand_command_macros(
            "say $greet !",
            &|name| {
                if name == "greet" {
                    "\u{80}hiÿ".to_string()
                } else {
                    String::new()
                }
            },
            &mut |text| printed.push(text.to_string()),
            TextMode::Source,
        )
        .unwrap();
        assert_eq!(expanded, Some("say \u{80}hiÿ !".to_string()));
        assert!(printed.is_empty());
    }
}
