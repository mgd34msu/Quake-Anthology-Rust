//! Command text parsing ported from `src/core/commands/text.ts` (Quake
//! `common.c`, Q2 `q_shared.c`/`cmd.c`, Q3 `cmd.c`) with dialects from
//! `src/contracts/common.ts`.
//!
//! This module owns the pure text layer: source validation, tokenizing,
//! command splitting, macro expansion, and argument tails. Buffered
//! dispatch (`CommandBuffer`: insertions, aliases, `wait`, script reads) is
//! a follow-up built on these primitives.

use thiserror::Error;

/// Error for invalid command text.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CmdError {
    /// Command text must be source bytes (truncated at NUL, code points `<= 255`).
    #[error("Command text requires source bytes")]
    NonByteText,
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

/// Truncate at the first NUL and reject code points above 255.
pub fn source_command_text(input: &str) -> Result<String, CmdError> {
    let text = input.split('\0').next().unwrap_or("");
    if text.chars().any(|c| c as u32 > 255) {
        return Err(CmdError::NonByteText);
    }
    Ok(text.to_string())
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

fn is_whitespace(byte: u32, mode: TextMode) -> bool {
    byte <= 32 || (mode == TextMode::Source && byte >= 128)
}

/// Byte offset of the next command separator (`;` outside quotes, newline,
/// or Q3 carriage return), or the text length when there is none.
#[must_use]
pub fn command_separator_offset(text: &str, dialect: Dialect) -> usize {
    let mut quoted = false;
    for (offset, c) in text.char_indices() {
        if c == '"' {
            quoted = !quoted;
        }
        if (!quoted && c == ';') || c == '\n' || (dialect == Dialect::Q3 && c == '\r') {
            return offset;
        }
    }
    text.len()
}

struct ParsedToken {
    value: String,
    end: usize,
}

fn parse_token(
    chars: &[char],
    start: usize,
    dialect: Dialect,
    mode: TextMode,
) -> Result<Option<ParsedToken>, CmdError> {
    let mut offset = start;
    loop {
        while offset < chars.len() && is_whitespace(chars[offset] as u32, mode) {
            offset += 1;
        }
        if offset + 1 < chars.len() && chars[offset] == '/' && chars[offset + 1] == '/' {
            if dialect == Dialect::Q3 {
                return Ok(None);
            }
            while offset < chars.len() && chars[offset] != '\n' {
                offset += 1;
            }
            continue;
        }
        if dialect == Dialect::Q3 && offset + 1 < chars.len() && chars[offset] == '/' && chars[offset + 1] == '*' {
            let mut end = offset + 2;
            while end + 1 < chars.len() && !(chars[end] == '*' && chars[end + 1] == '/') {
                end += 1;
            }
            offset = if end + 1 < chars.len() { end + 2 } else { chars.len() };
            continue;
        }
        break;
    }
    if offset >= chars.len() {
        return Ok(None);
    }
    let quoted = chars[offset] == '"';
    if quoted {
        offset += 1;
    }
    let token_start = offset;
    if quoted {
        while offset < chars.len() && chars[offset] != '"' {
            offset += 1;
        }
    } else if dialect == Dialect::Q1Netquake && matches!(chars[offset], '{' | '}' | '(' | ')' | '\'' | ':') {
        offset += 1;
    } else {
        while offset < chars.len() && !is_whitespace(chars[offset] as u32, mode) {
            if dialect == Dialect::Q1Netquake && matches!(chars[offset], '{' | '}' | '(' | ')' | '\'' | ':') {
                break;
            }
            if dialect == Dialect::Q3
                && (chars[offset] == '"'
                    || (offset + 1 < chars.len() && chars[offset] == '/' && matches!(chars[offset + 1], '/' | '*')))
            {
                break;
            }
            offset += 1;
        }
    }
    let mut value: String = chars[token_start..offset].iter().collect();
    if dialect.is_q2() && value.len() >= 128 {
        if quoted {
            return Err(CmdError::TokenOverflow(
                "Quoted command token overflows source MAX_TOKEN_CHARS".to_string(),
            ));
        }
        value = String::new();
    } else if dialect.is_q1() && value.len() >= 1024 {
        return Err(CmdError::TokenOverflow(
            "Command token overflows source com_token".to_string(),
        ));
    }
    if quoted && offset < chars.len() && chars[offset] == '"' {
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
    let mut text = source_command_text(input)?;
    let mut budget = text.chars().count();
    if budget >= 1024 {
        print("Line exceeded 1024 chars, discarded.\n");
        return Ok(None);
    }
    let mut quoted = false;
    let mut count = 0;
    let mut offset = 0;
    loop {
        let chars: Vec<char> = text.chars().collect();
        if offset >= chars.len() {
            break;
        }
        if chars[offset] == '"' {
            quoted = !quoted;
        }
        if !quoted && chars[offset] == '$' {
            let token = parse_token(&chars, offset + 1, Dialect::Q2Classic, mode)?;
            if let Some(token) = token {
                let value = variable(&token.value);
                budget += value.chars().count();
                if budget >= 1024 {
                    print("Expanded line exceeded 1024 chars, discarded.\n");
                    return Ok(None);
                }
                let before: String = chars[..offset].iter().collect();
                let after: String = chars[token.end..].iter().collect();
                text = format!("{before}{value}{after}");
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
    let text = source_command_text(input)?;
    let chars: Vec<char> = text.chars().collect();
    let mut offset = 0;
    for _ in 0..count {
        while offset < chars.len() && is_whitespace(chars[offset] as u32, TextMode::Source) {
            offset += 1;
        }
        let token = parse_token(&chars, offset, dialect, TextMode::Source)?;
        if token.is_none() {
            return Ok(String::new());
        }
        offset = token.map_or(offset, |token| token.end);
    }
    while offset < chars.len() && is_whitespace(chars[offset] as u32, TextMode::Source) {
        offset += 1;
    }
    Ok(chars[offset..].iter().collect())
}

/// Tokenize one command line. Q2 trims trailing control bytes from the
/// argument text; Q3 rebuilds it from tokenized arguments.
pub fn tokenize_command(input: &str, dialect: Dialect, mode: TextMode) -> Result<CommandTokens, CmdError> {
    let text = source_command_text(input)?;
    let chars: Vec<char> = text.chars().collect();
    let maximum_tokens = if dialect == Dialect::Q3 { 1024 } else { 80 };
    let mut argv: Vec<String> = Vec::new();
    let mut args_text = String::new();
    let mut stored_bytes = 0;
    let mut offset = 0;
    while offset < chars.len() {
        while offset < chars.len()
            && is_whitespace(chars[offset] as u32, mode)
            && (dialect == Dialect::Q3 || chars[offset] != '\n')
        {
            offset += 1;
        }
        if dialect != Dialect::Q3 && offset < chars.len() && chars[offset] == '\n' {
            break;
        }
        if argv.len() == 1 {
            let tail: String = chars[offset..].iter().collect();
            args_text = if dialect.is_q2() {
                tail.trim_end_matches(|c: char| (c as u32) <= 32).to_string()
            } else {
                tail
            };
        }
        let token = parse_token(&chars, offset, dialect, mode)?;
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
        args_text = argv.iter().skip(1).cloned().collect::<Vec<_>>().join(" ");
    }
    Ok(CommandTokens { argv, args_text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_text_validates_bytes() {
        assert_eq!(source_command_text("say hi\0trailing"), Ok("say hi".to_string()));
        assert!(source_command_text("caf\u{20ac}").is_err());
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
}
