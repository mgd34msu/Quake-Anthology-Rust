//! Quake HUD text tokenizer.
//!
//! Ported from the TypeScript donor's `src/core/common-parse.ts` (`Tokenizer`).
//! Splits HUD layout sources into whitespace-delimited tokens, skipping `//`
//! and `/* */` comments. Quoted strings keep their inner spaces. Line and
//! column tracking treats CR, LF, and CRLF as one line break each.

use crate::error::ClientError;

/// Maximum token length, in characters, including the terminator slot.
///
/// Tokens reaching this length fail, matching the donor's `TOKEN_MAX` check.
pub(crate) const TOKEN_MAX: usize = 1024;

/// One token scanned from a HUD text source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HudToken {
    /// Token text without surrounding quotes.
    pub value: String,
    /// 1-based line where the token starts.
    pub line: i32,
    /// 1-based column where the token starts.
    pub column: i32,
    /// Whether the token was double-quoted.
    pub quoted: bool,
}

/// Cursor over a HUD text source producing [`HudToken`] values.
#[derive(Debug, Clone)]
pub(crate) struct HudTokenizer {
    source: Vec<char>,
    name: String,
    offset: usize,
    line: i32,
    column: i32,
}

impl HudTokenizer {
    /// Borrow the source name used in error messages.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Current 1-based line.
    #[must_use]
    pub fn line(&self) -> i32 {
        self.line
    }

    /// Current 1-based column.
    #[must_use]
    pub fn column(&self) -> i32 {
        self.column
    }

    /// Create a tokenizer over `source`, naming it `name` in error messages.
    #[must_use]
    pub fn new(source: &str, name: &str) -> Self {
        Self {
            source: source.chars().collect(),
            name: name.to_string(),
            offset: 0,
            line: 1,
            column: 1,
        }
    }

    /// Scan the next token, or `Ok(None)` at end of input.
    ///
    /// When `allow_line_breaks` is false, scanning stops (returning `Ok(None)`
    /// without consuming the token) if a line break or a line break inside a
    /// block comment was crossed while skipping whitespace and comments.
    /// Tokens of [`TOKEN_MAX`] characters or more fail with
    /// [`ClientError::BadUi`].
    pub fn next(&mut self, allow_line_breaks: bool) -> Result<Option<HudToken>, ClientError> {
        let mut crossed_line = false;

        loop {
            while self.offset < self.source.len() {
                let Some(character) = self.peek() else {
                    break;
                };
                if !is_whitespace(character) {
                    break;
                }
                if character == '\n' || character == '\r' {
                    crossed_line = true;
                }
                self.advance();
            }

            if self.starts_with("//") {
                self.advance();
                self.advance();
                while self.offset < self.source.len() {
                    match self.peek() {
                        Some('\n') | Some('\r') | None => break,
                        Some(_) => self.advance(),
                    }
                }
                continue;
            }

            if self.starts_with("/*") {
                self.advance();
                self.advance();
                while self.offset < self.source.len() && !self.starts_with("*/") {
                    match self.peek() {
                        Some('\n') | Some('\r') => crossed_line = true,
                        _ => {}
                    }
                    self.advance();
                }
                if self.starts_with("*/") {
                    self.advance();
                    self.advance();
                }
                continue;
            }

            break;
        }

        if !allow_line_breaks && crossed_line {
            return Ok(None);
        }
        if self.offset >= self.source.len() {
            return Ok(None);
        }

        let line = self.line();
        let column = self.column();
        let quoted = self.peek() == Some('"');
        let mut value = String::new();
        let mut length = 0usize;

        if quoted {
            self.advance();
            while self.offset < self.source.len() {
                match self.peek() {
                    Some('"') | None => break,
                    Some(character) => {
                        value.push(character);
                        length += 1;
                        self.advance();
                    }
                }
            }
            if self.peek() == Some('"') {
                self.advance();
            }
        } else {
            while self.offset < self.source.len() {
                match self.peek() {
                    Some(character) if !is_whitespace(character) => {
                        value.push(character);
                        length += 1;
                        self.advance();
                    }
                    _ => break,
                }
            }
        }

        if length >= TOKEN_MAX {
            return Err(ClientError::BadUi(format!(
                "{}:{line}:{column}: token is limited to {} characters",
                self.name(),
                TOKEN_MAX - 1
            )));
        }

        Ok(Some(HudToken {
            value,
            line,
            column,
            quoted,
        }))
    }

    /// Peek the current character without consuming it.
    fn peek(&self) -> Option<char> {
        self.source.get(self.offset).copied()
    }

    /// Whether the source at the cursor starts with `text`.
    fn starts_with(&self, text: &str) -> bool {
        for (index, expected) in text.chars().enumerate() {
            if self.source.get(self.offset + index).copied() != Some(expected) {
                return false;
            }
        }
        true
    }

    /// Consume one character, folding CRLF into a single line break.
    fn advance(&mut self) {
        let Some(character) = self.peek() else {
            return;
        };
        if character == '\r' {
            self.offset += 1;
            if self.peek() == Some('\n') {
                self.offset += 1;
            }
            self.line += 1;
            self.column = 1;
            return;
        }
        self.offset += 1;
        if character == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
    }
}

/// Whether a character is tokenizer whitespace (code point at most 32).
fn is_whitespace(character: char) -> bool {
    (character as u32) <= 32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(source: &str) -> Result<Vec<HudToken>, ClientError> {
        let mut tokenizer = HudTokenizer::new(source, "test.txt");
        let mut tokens = Vec::new();
        while let Some(token) = tokenizer.next(true)? {
            tokens.push(token);
        }
        Ok(tokens)
    }

    fn values(tokens: &[HudToken]) -> Vec<&str> {
        tokens.iter().map(|token| token.value.as_str()).collect()
    }

    #[test]
    fn splits_on_whitespace_with_positions() {
        let tokens = collect("slot 1 {").expect("tokenize");
        assert_eq!(values(&tokens), vec!["slot", "1", "{"]);
        assert_eq!((tokens[0].line, tokens[0].column), (1, 1));
        assert_eq!((tokens[1].line, tokens[1].column), (1, 6));
        assert_eq!((tokens[2].line, tokens[2].column), (1, 8));
        assert!(tokens.iter().all(|token| !token.quoted));
    }

    #[test]
    fn skips_line_comments() {
        let tokens = collect("slot // trailing\n1").expect("tokenize");
        assert_eq!(values(&tokens), vec!["slot", "1"]);
        assert_eq!((tokens[1].line, tokens[1].column), (2, 1));
    }

    #[test]
    fn skips_block_comments() {
        let tokens = collect("slot /* one /* two */ 1").expect("tokenize");
        assert_eq!(values(&tokens), vec!["slot", "1"]);
    }

    #[test]
    fn block_comment_newline_counts_as_crossed_line() {
        let mut tokenizer = HudTokenizer::new("a /* x\ny */ b", "test.txt");
        let first = tokenizer.next(false).expect("first").expect("token");
        assert_eq!(first.value, "a");
        assert!(tokenizer.next(false).expect("second").is_none());
        let resumed = tokenizer.next(true).expect("resumed").expect("token");
        assert_eq!(resumed.value, "b");
    }

    #[test]
    fn unterminated_block_comment_consumes_rest() {
        let tokens = collect("slot /* never ends").expect("tokenize");
        assert_eq!(values(&tokens), vec!["slot"]);
    }

    #[test]
    fn quoted_strings_keep_spaces() {
        let tokens = collect("label \"hello world\" next").expect("tokenize");
        assert_eq!(values(&tokens), vec!["label", "hello world", "next"]);
        assert!(tokens[1].quoted);
        assert!(!tokens[0].quoted);
    }

    #[test]
    fn unterminated_quote_reads_to_end() {
        let tokens = collect("\"abc").expect("tokenize");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].value, "abc");
        assert!(tokens[0].quoted);
    }

    #[test]
    fn crlf_counts_as_single_line_break() {
        let tokens = collect("a\r\nb\rc\nd").expect("tokenize");
        assert_eq!(values(&tokens), vec!["a", "b", "c", "d"]);
        assert_eq!(tokens[1].line, 2);
        assert_eq!(tokens[2].line, 3);
        assert_eq!(tokens[3].line, 4);
        assert_eq!(tokens[1].column, 1);
    }

    #[test]
    fn disallowed_line_break_stops_without_consuming() {
        let mut tokenizer = HudTokenizer::new("key value\nnext", "test.txt");
        let key = tokenizer.next(false).expect("key").expect("token");
        assert_eq!(key.value, "key");
        let value = tokenizer.next(false).expect("value").expect("token");
        assert_eq!(value.value, "value");
        assert!(tokenizer.next(false).expect("stop").is_none());
        let resumed = tokenizer.next(true).expect("resumed").expect("token");
        assert_eq!((resumed.value.as_str(), resumed.line), ("next", 2));
    }

    #[test]
    fn comment_only_line_stops_when_breaks_disallowed() {
        let mut tokenizer = HudTokenizer::new("a // note\nb", "test.txt");
        let first = tokenizer.next(false).expect("first").expect("token");
        assert_eq!(first.value, "a");
        assert!(tokenizer.next(false).expect("stop").is_none());
    }

    #[test]
    fn max_token_length_fails() {
        let long = "x".repeat(TOKEN_MAX);
        let mut tokenizer = HudTokenizer::new(&long, "wwheel.txt");
        let error = tokenizer.next(true).expect_err("must fail");
        assert_eq!(
            error.to_string(),
            format!("wwheel.txt:1:1: token is limited to {} characters", TOKEN_MAX - 1)
        );
    }

    #[test]
    fn token_just_under_limit_passes() {
        let long = "x".repeat(TOKEN_MAX - 1);
        let tokens = collect(&long).expect("tokenize");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].value.len(), TOKEN_MAX - 1);
    }

    #[test]
    fn control_characters_are_whitespace() {
        let tokens = collect("a\x00b\tc").expect("tokenize");
        assert_eq!(values(&tokens), vec!["a", "b", "c"]);
    }

    #[test]
    fn empty_source_yields_no_tokens() {
        assert!(collect("").expect("tokenize").is_empty());
        assert!(collect("  \n // only a comment").expect("tokenize").is_empty());
    }
}
