//! TypeScript-tolerant scanning for inventory tooling.
//!
//! The donor census drives the TypeScript compiler API. This module provides
//! the portable equivalent: a comment/string/template/regex-aware tokenizer,
//! a declaration walker reporting `SyntaxKind`-named records with source
//! ranges, import specifiers, and syntactic diagnostics, plus token hashing
//! for signature and body matching.

use crate::sha256::hash_hex;

/// Token kind (stable codes feed token hashes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokKind {
    /// Identifier or keyword.
    Ident = 1,
    /// String literal.
    Str = 2,
    /// Numeric literal.
    Number = 3,
    /// Regular expression literal.
    Regex = 4,
    /// Template head (`` `...${ ``).
    TmplHead = 5,
    /// Template middle (`` }...${ ``).
    TmplMid = 6,
    /// Template tail (`` }...` ``).
    TmplTail = 7,
    /// Template without substitutions.
    TmplNoSub = 8,
    /// Punctuator.
    Punct = 9,
}

/// A lexical token with byte offsets.
#[derive(Debug, Clone, Copy)]
pub struct Token {
    /// Token kind.
    pub kind: TokKind,
    /// Byte start.
    pub start: usize,
    /// Byte end.
    pub end: usize,
}

/// A comment with byte offsets.
#[derive(Debug, Clone)]
pub struct Comment {
    /// Byte start (at the opening slash).
    pub start: usize,
    /// Byte end (past the closer or newline).
    pub end: usize,
    /// Comment text.
    pub text: String,
}

/// Tokenizer output.
pub struct Scan {
    /// Tokens (trivia excluded).
    pub tokens: Vec<Token>,
    /// Comments.
    pub comments: Vec<Comment>,
    /// Tokenizer diagnostics (`{offset}: {message}`).
    pub diagnostics: Vec<String>,
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_alphabetic()
}

fn is_ident_part(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_alphanumeric() || ch == '\u{200C}' || ch == '\u{200D}'
}

fn regex_keyword(text: &str) -> bool {
    matches!(
        text,
        "return" | "typeof" | "case" | "do" | "else" | "in" | "of" | "new" | "delete" | "void" | "instanceof" | "yield" | "await" | "throw"
    )
}

struct Tokenizer<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
    tokens: Vec<Token>,
    comments: Vec<Comment>,
    diagnostics: Vec<String>,
    template_depths: Vec<usize>,
    brace_depth: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            pos: 0,
            tokens: Vec::new(),
            comments: Vec::new(),
            diagnostics: Vec::new(),
            template_depths: Vec::new(),
            brace_depth: 0,
        }
    }

    fn text(&self, start: usize, end: usize) -> &str {
        &self.source[start..end.min(self.source.len())]
    }

    fn peek_char(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }

    fn slash_is_regex(&self) -> bool {
        let Some(prev) = self.tokens.last() else {
            return true;
        };
        match prev.kind {
            TokKind::Ident => regex_keyword(self.text(prev.start, prev.end)),
            TokKind::Str | TokKind::Number | TokKind::Regex | TokKind::TmplTail | TokKind::TmplNoSub => false,
            TokKind::TmplHead | TokKind::TmplMid => true,
            TokKind::Punct => {
                let text = self.text(prev.start, prev.end);
                !matches!(text, ")" | "]" | "++" | "--")
            }
        }
    }

    fn tokenize(mut self) -> Scan {
        while self.pos < self.bytes.len() {
            let ch = match self.peek_char() {
                Some(ch) => ch,
                None => break,
            };
            if ch.is_whitespace() {
                self.pos += ch.len_utf8();
                continue;
            }
            if ch == '/' && self.bytes.get(self.pos + 1) == Some(&b'/') {
                let start = self.pos;
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
                self.comments.push(Comment {
                    start,
                    end: self.pos,
                    text: self.text(start, self.pos).to_owned(),
                });
                continue;
            }
            if ch == '/' && self.bytes.get(self.pos + 1) == Some(&b'*') {
                let start = self.pos;
                match self.source[self.pos + 2..].find("*/") {
                    Some(end) => {
                        self.pos += end + 4;
                        self.comments.push(Comment {
                            start,
                            end: self.pos,
                            text: self.text(start, self.pos).to_owned(),
                        });
                    }
                    None => {
                        self.diagnostics.push(format!("{start}: Unterminated block comment"));
                        self.comments.push(Comment {
                            start,
                            end: self.bytes.len(),
                            text: self.text(start, self.bytes.len()).to_owned(),
                        });
                        self.pos = self.bytes.len();
                    }
                }
                continue;
            }
            if ch == '\'' || ch == '"' {
                self.string_token(ch);
                continue;
            }
            if ch == '`' {
                self.template_token();
                continue;
            }
            if ch == '/' && self.slash_is_regex() && !matches!(self.bytes.get(self.pos + 1), Some(b'/') | Some(b'*')) {
                self.regex_token();
                continue;
            }
            if ch == '#' && self.source[self.pos + 1..].chars().next().is_some_and(is_ident_start) {
                let start = self.pos;
                self.pos += 1;
                self.consume_ident();
                self.tokens.push(Token {
                    kind: TokKind::Ident,
                    start,
                    end: self.pos,
                });
                continue;
            }
            if is_ident_start(ch) {
                let start = self.pos;
                self.consume_ident();
                self.tokens.push(Token {
                    kind: TokKind::Ident,
                    start,
                    end: self.pos,
                });
                continue;
            }
            if ch.is_ascii_digit() || (ch == '.' && self.bytes.get(self.pos + 1).is_some_and(u8::is_ascii_digit)) {
                self.number_token();
                continue;
            }
            if ch == '}' && self.template_depths.last() == Some(&self.brace_depth) {
                self.template_continuation();
                continue;
            }
            let start = self.pos;
            let punct = self.consume_punct();
            if punct == "{" {
                self.brace_depth += 1;
            } else if punct == "}" {
                self.brace_depth = self.brace_depth.saturating_sub(1);
            }
            self.tokens.push(Token {
                kind: TokKind::Punct,
                start,
                end: self.pos,
            });
        }
        Scan {
            tokens: self.tokens,
            comments: self.comments,
            diagnostics: self.diagnostics,
        }
    }

    fn consume_ident(&mut self) {
        while let Some(ch) = self.peek_char() {
            if is_ident_part(ch) {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
    }

    fn string_token(&mut self, quote: char) {
        let start = self.pos;
        self.pos += 1;
        let mut closed = false;
        while let Some(ch) = self.peek_char() {
            if ch == '\\' {
                self.pos += 1;
                if let Some(next) = self.peek_char() {
                    self.pos += next.len_utf8();
                }
                continue;
            }
            self.pos += ch.len_utf8();
            if ch == quote {
                closed = true;
                break;
            }
            if ch == '\n' {
                break;
            }
        }
        if !closed {
            self.diagnostics.push(format!("{start}: Unterminated string literal"));
        }
        self.tokens.push(Token {
            kind: TokKind::Str,
            start,
            end: self.pos,
        });
    }

    fn template_token(&mut self) {
        let start = self.pos;
        self.pos += 1;
        while let Some(ch) = self.peek_char() {
            if ch == '\\' {
                self.pos += 1;
                if let Some(next) = self.peek_char() {
                    self.pos += next.len_utf8();
                }
                continue;
            }
            if ch == '`' {
                self.pos += 1;
                self.tokens.push(Token {
                    kind: TokKind::TmplNoSub,
                    start,
                    end: self.pos,
                });
                return;
            }
            if ch == '$' && self.bytes.get(self.pos + 1) == Some(&b'{') {
                self.pos += 2;
                self.tokens.push(Token {
                    kind: TokKind::TmplHead,
                    start,
                    end: self.pos,
                });
                self.template_depths.push(self.brace_depth);
                return;
            }
            if ch == '\n' {
                self.pos += 1;
                continue;
            }
            self.pos += ch.len_utf8();
        }
        self.diagnostics.push(format!("{start}: Unterminated template literal"));
        self.tokens.push(Token {
            kind: TokKind::TmplNoSub,
            start,
            end: self.pos,
        });
    }

    fn template_continuation(&mut self) {
        self.template_depths.pop();
        self.pos += 1;
        let start = self.pos - 1;
        while let Some(ch) = self.peek_char() {
            if ch == '\\' {
                self.pos += 1;
                if let Some(next) = self.peek_char() {
                    self.pos += next.len_utf8();
                }
                continue;
            }
            if ch == '`' {
                self.pos += 1;
                self.tokens.push(Token {
                    kind: TokKind::TmplTail,
                    start,
                    end: self.pos,
                });
                return;
            }
            if ch == '$' && self.bytes.get(self.pos + 1) == Some(&b'{') {
                self.pos += 2;
                self.tokens.push(Token {
                    kind: TokKind::TmplMid,
                    start,
                    end: self.pos,
                });
                self.template_depths.push(self.brace_depth);
                return;
            }
            self.pos += ch.len_utf8();
        }
        self.diagnostics.push(format!("{start}: Unterminated template literal"));
        self.tokens.push(Token {
            kind: TokKind::TmplTail,
            start,
            end: self.pos,
        });
    }

    fn regex_token(&mut self) {
        let start = self.pos;
        self.pos += 1;
        let mut in_class = false;
        let mut closed = false;
        while let Some(ch) = self.peek_char() {
            if ch == '\\' {
                self.pos += 1;
                if let Some(next) = self.peek_char() {
                    self.pos += next.len_utf8();
                }
                continue;
            }
            if ch == '\n' {
                break;
            }
            self.pos += ch.len_utf8();
            if ch == '[' {
                in_class = true;
            } else if ch == ']' {
                in_class = false;
            } else if ch == '/' && !in_class {
                closed = true;
                break;
            }
        }
        if closed {
            while let Some(ch) = self.peek_char() {
                if is_ident_part(ch) {
                    self.pos += ch.len_utf8();
                } else {
                    break;
                }
            }
        } else {
            self.diagnostics.push(format!("{start}: Unterminated regular expression"));
        }
        self.tokens.push(Token {
            kind: TokKind::Regex,
            start,
            end: self.pos,
        });
    }

    fn number_token(&mut self) {
        let start = self.pos;
        if self.source[self.pos..].starts_with("0x")
            || self.source[self.pos..].starts_with("0X")
            || self.source[self.pos..].starts_with("0b")
            || self.source[self.pos..].starts_with("0B")
            || self.source[self.pos..].starts_with("0o")
            || self.source[self.pos..].starts_with("0O")
        {
            self.pos += 2;
            while let Some(ch) = self.peek_char() {
                if ch.is_ascii_hexdigit() || ch == '_' {
                    self.pos += ch.len_utf8();
                } else {
                    break;
                }
            }
        } else {
            while let Some(ch) = self.peek_char() {
                if ch.is_ascii_digit() || ch == '_' {
                    self.pos += ch.len_utf8();
                } else {
                    break;
                }
            }
            if self.peek_char() == Some('.') && self.source[self.pos + 1..].chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
                self.pos += 1;
                while let Some(ch) = self.peek_char() {
                    if ch.is_ascii_digit() || ch == '_' {
                        self.pos += ch.len_utf8();
                    } else {
                        break;
                    }
                }
            }
            if matches!(self.peek_char(), Some('e') | Some('E')) {
                let mut lookahead = self.pos + 1;
                if matches!(self.bytes.get(lookahead), Some(b'+') | Some(b'-')) {
                    lookahead += 1;
                }
                if self.bytes.get(lookahead).is_some_and(u8::is_ascii_digit) {
                    self.pos = lookahead + 1;
                    while let Some(ch) = self.peek_char() {
                        if ch.is_ascii_digit() || ch == '_' {
                            self.pos += ch.len_utf8();
                        } else {
                            break;
                        }
                    }
                }
            }
        }
        if self.peek_char() == Some('n') {
            self.pos += 1;
        }
        self.tokens.push(Token {
            kind: TokKind::Number,
            start,
            end: self.pos,
        });
    }

    fn consume_punct(&mut self) -> &str {
        let rest = &self.source[self.pos..];
        for candidate in [">>>=", "...", "===", "!==", ">>>", "<<=", ">>=", "**=", "??=", "||=", "&&=", "=>", "==", "!=", "<=", ">=", "&&", "||", "??", "?.", "++", "--", "<<", ">>", "**", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^="] {
            if rest.starts_with(candidate) {
                // A `>=`-family token immediately followed by `>` never occurs in valid
                // TS; prefer the shorter token so `>=>` lexes as `>` + `=>` (this is the
                // tokenizer-side half of TypeScript's `reScanGreaterToken` behavior).
                if matches!(candidate, ">=" | ">>=" | ">>>=") && rest[candidate.len()..].starts_with('>') {
                    continue;
                }
                // `?.` followed by a digit is `?` + a fractional literal, not optional access.
                if candidate == "?." && rest[2..].chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
                    continue;
                }
                self.pos += candidate.len();
                return &self.source[self.pos - candidate.len()..self.pos];
            }
        }
        let ch = self.peek_char().unwrap_or('?');
        self.pos += ch.len_utf8();
        &self.source[self.pos - ch.len_utf8()..self.pos]
    }
}

/// Tokenize TypeScript source.
#[must_use]
pub fn tokenize(source: &str) -> Scan {
    Tokenizer::new(source).tokenize()
}

/// Hash a token stream the way the donor hashes scanner output.
#[must_use]
pub fn token_hash(source: &str) -> String {
    let scan = tokenize(source);
    let mut pairs = String::from("[");
    for (index, token) in scan.tokens.iter().enumerate() {
        if index > 0 {
            pairs.push(',');
        }
        pairs.push_str(&format!("[{},", token.kind as u8));
        push_json_string(&mut pairs, &source[token.start..token.end]);
        pairs.push(']');
    }
    pairs.push(']');
    hash_hex(pairs.as_bytes())
}

fn push_json_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{09}' => out.push_str("\\t"),
            '\u{0A}' => out.push_str("\\n"),
            '\u{0C}' => out.push_str("\\f"),
            '\u{0D}' => out.push_str("\\r"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
}

/// Byte offsets of every line start.
#[must_use]
pub fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

/// 1-based line and column for a byte offset.
#[must_use]
pub fn line_col(starts: &[usize], offset: usize) -> (usize, usize) {
    let line = starts.partition_point(|start| *start <= offset);
    let start = starts.get(line.saturating_sub(1)).copied().unwrap_or(0);
    (line.max(1), offset.saturating_sub(start) + 1)
}

/// 1-based line for a byte offset.
#[must_use]
pub fn line_of(starts: &[usize], offset: usize) -> usize {
    starts.partition_point(|start| *start <= offset).max(1)
}

/// A declared construct with `SyntaxKind`-compatible naming.
#[derive(Debug, Clone)]
pub struct DeclInfo {
    /// Syntax kind name (`FunctionDeclaration`, `MethodSignature`, ...).
    pub kind: &'static str,
    /// Inferred name.
    pub name: String,
    /// Byte start (first token).
    pub start: usize,
    /// Byte end (past the last token).
    pub end: usize,
    /// Body block range, when present.
    pub body: Option<(usize, usize)>,
    /// Members position (past `{`) for class/interface/enum shapes.
    pub members_pos: Option<usize>,
    /// Initializer start, when present.
    pub initializer_start: Option<usize>,
    /// Enclosing class/module/function names, innermost first.
    pub enclosing: Vec<String>,
    /// Own plus enclosing `export`/`default` modifiers.
    pub exports: Vec<String>,
}

/// Declaration scan output.
pub struct ScanDeclarations {
    /// Declarations in source order.
    pub declarations: Vec<DeclInfo>,
    /// Import/export module specifiers with quotes.
    pub imports: Vec<String>,
    /// Syntactic diagnostics (`{offset}: {message}`).
    pub diagnostics: Vec<String>,
}

struct Frame {
    kind: &'static str,
    name: String,
    exports: Vec<String>,
}

struct Walker<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    decls: Vec<DeclInfo>,
    imports: Vec<String>,
    diagnostics: Vec<String>,
    frames: Vec<Frame>,
    depth: usize,
}

fn is_function_frame(kind: &str) -> bool {
    matches!(
        kind,
        "FunctionDeclaration"
            | "FunctionExpression"
            | "ArrowFunction"
            | "MethodDeclaration"
            | "GetAccessor"
            | "SetAccessor"
            | "Constructor"
            | "MethodSignature"
            | "CallSignature"
            | "ConstructSignature"
            | "FunctionType"
            | "ConstructorType"
    )
}

fn is_frame_named(kind: &str) -> bool {
    matches!(kind, "ClassDeclaration" | "ClassExpression" | "ModuleDeclaration") || is_function_frame(kind)
}

impl<'a> Walker<'a> {
    fn new(source: &'a str, tokens: Vec<Token>, diagnostics: Vec<String>) -> Self {
        Self {
            source,
            tokens,
            pos: 0,
            decls: Vec::new(),
            imports: Vec::new(),
            diagnostics,
            frames: Vec::new(),
            depth: 0,
        }
    }

    fn text(&self, token: Token) -> &str {
        &self.source[token.start..token.end.min(self.source.len())]
    }

    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<Token> {
        self.tokens.get(self.pos + offset).copied()
    }

    fn peek_text(&self, offset: usize) -> Option<&str> {
        self.peek_at(offset).map(|token| self.text(token))
    }

    fn at_ident(&self, offset: usize, name: &str) -> bool {
        matches!(self.peek_at(offset), Some(token) if token.kind == TokKind::Ident && self.text(token) == name)
    }

    fn at_punct(&self, offset: usize, punct: &str) -> bool {
        matches!(self.peek_at(offset), Some(token) if token.kind == TokKind::Punct && self.text(token) == punct)
    }

    fn at_eof(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.peek()?;
        self.pos += 1;
        Some(token)
    }

    fn error(&mut self, offset: usize, message: &str) {
        self.diagnostics.push(format!("{offset}: {message}"));
    }

    fn error_here(&mut self, message: &str) {
        let offset = self.peek().map_or(self.source.len(), |token| token.start);
        self.error(offset, message);
    }

    fn guarded<T>(&mut self, fallback: T, work: impl FnOnce(&mut Self) -> T) -> T {
        if self.depth > 400 {
            self.error_here("Nesting exceeds parser depth");
            return fallback;
        }
        self.depth += 1;
        let result = work(self);
        self.depth -= 1;
        result
    }

    fn enclosing(&self) -> Vec<String> {
        self.frames.iter().rev().filter(|frame| is_frame_named(frame.kind)).map(|frame| frame.name.clone()).collect()
    }

    fn exports(&self, own: &[String]) -> Vec<String> {
        let mut exports = own.to_vec();
        for frame in self.frames.iter().rev() {
            exports.extend(frame.exports.iter().cloned());
        }
        exports
    }

    fn record(
        &mut self,
        kind: &'static str,
        name: String,
        start: usize,
        end: usize,
        body: Option<(usize, usize)>,
        members_pos: Option<usize>,
        initializer_start: Option<usize>,
        own_exports: &[String],
    ) {
        let enclosing = self.enclosing();
        let exports = self.exports(own_exports);
        self.decls.push(DeclInfo {
            kind,
            name,
            start,
            end,
            body,
            members_pos,
            initializer_start,
            enclosing,
            exports,
        });
    }

    fn match_paren(&self, open: usize) -> Option<usize> {
        let mut depth = 0;
        let mut index = open;
        while let Some(token) = self.tokens.get(index) {
            let text = &self.source[token.start..token.end.min(self.source.len())];
            if token.kind == TokKind::Punct {
                if text == "(" || text == "[" || text == "{" {
                    depth += 1;
                } else if text == ")" || text == "]" || text == "}" {
                    if depth == 0 {
                        return None;
                    }
                    depth -= 1;
                    if depth == 0 {
                        return Some(index);
                    }
                }
            }
            index += 1;
        }
        None
    }

    fn match_angle(&self, open: usize) -> Option<usize> {
        let mut depth: i32 = 0;
        let mut index = open;
        while let Some(token) = self.tokens.get(index) {
            let text = &self.source[token.start..token.end.min(self.source.len())];
            if token.kind == TokKind::Punct {
                match text {
                    "<" => depth += 1,
                    ">" => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(index);
                        }
                        if depth < 0 {
                            return None;
                        }
                    }
                    ">>" => {
                        depth -= 2;
                        if depth <= 0 {
                            return if depth == 0 { Some(index) } else { None };
                        }
                    }
                    ">>>" => {
                        depth -= 3;
                        if depth <= 0 {
                            return if depth == 0 { Some(index) } else { None };
                        }
                    }
                    ">=" | ">>=" | ">>>=" | "=>" | "->" => return None,
                    "(" => {
                        if let Some(close) = self.match_paren(index) {
                            if self.tokens.get(close + 1).is_some_and(|token| self.text(*token) == "=>") {
                                index = close + 2;
                                continue;
                            }
                            index = close + 1;
                            continue;
                        }
                        return None;
                    }
                    "[" | "{" => {
                        match self.match_paren(index) {
                            Some(close) => {
                                index = close + 1;
                                continue;
                            }
                            None => return None,
                        }
                    }
                    ";" | "}" => return None,
                    _ => {}
                }
            }
            index += 1;
        }
        None
    }

    fn skip_balanced(&mut self) {
        if let Some(close) = self.peek().and_then(|_| self.match_paren(self.pos)) {
            self.pos = close + 1;
        } else {
            self.bump();
        }
    }

    fn recover(&mut self) {
        let mut depth = 0;
        while let Some(token) = self.peek() {
            let text = self.text(token);
            if token.kind == TokKind::Punct {
                match text {
                    "(" | "[" | "{" => depth += 1,
                    ")" | "]" => {
                        if depth == 0 {
                            return;
                        }
                        depth -= 1;
                    }
                    "}" => {
                        if depth == 0 {
                            return;
                        }
                        depth -= 1;
                    }
                    ";" => {
                        if depth == 0 {
                            self.bump();
                            return;
                        }
                    }
                    _ => {}
                }
            }
            self.bump();
        }
    }

    fn collect_modifiers(&mut self) -> Vec<String> {
        let mut own = Vec::new();
        loop {
            let Some(token) = self.peek() else { break };
            if token.kind != TokKind::Ident {
                break;
            }
            let text = self.text(token);
            if matches!(
                text,
                "export" | "default" | "declare" | "abstract" | "async" | "public" | "private" | "protected" | "static" | "readonly" | "override"
            ) {
                if text == "export" || text == "default" {
                    own.push(text.to_owned());
                }
                self.bump();
            } else {
                break;
            }
        }
        own
    }

    fn chain_continues(&self, index: usize) -> bool {
        self.tokens.get(index + 1).is_some_and(|next| {
            next.kind == TokKind::Punct && matches!(self.text(*next), "." | "?." | "(" | "[" | "!" | "<")
        })
    }

    fn callee_text(&self, paren: usize) -> String {
        let mut start = paren;
        let mut end = paren.saturating_sub(1);
        let mut index = paren;
        while index > 0 {
            index -= 1;
            let token = self.tokens[index];
            let text = &self.source[token.start..token.end.min(self.source.len())];
            if token.kind == TokKind::Ident {
                let member_name = index > 0
                    && self.tokens[index - 1].kind == TokKind::Punct
                    && matches!(self.text(self.tokens[index - 1]), "." | "?.");
                if !member_name
                    && matches!(
                        text,
                        "new" | "typeof" | "return" | "case" | "do" | "else" | "in" | "of" | "delete" | "void" | "instanceof" | "yield" | "await" | "throw" | "if" | "for" | "while" | "switch" | "catch" | "with"
                    )
                {
                    break;
                }
                start = index;
            } else if token.kind == TokKind::Punct && (text == "." || text == "?." || text == "!" || text == ")" || text == "]") {
                if text == "!" {
                    if self.chain_continues(index) {
                        start = index;
                    } else {
                        break;
                    }
                } else if text == ")" || text == "]" {
                    if !self.chain_continues(index) {
                        break;
                    }
                    let mut depth = 1;
                    let mut back = index;
                    while back > 0 && depth > 0 {
                        back -= 1;
                        let inner = &self.source[self.tokens[back].start..self.tokens[back].end.min(self.source.len())];
                        if self.tokens[back].kind == TokKind::Punct {
                            if inner == ")" || inner == "]" {
                                depth += 1;
                            } else if inner == "(" || inner == "[" {
                                depth -= 1;
                            }
                        }
                    }
                    index = back;
                    start = back;
                } else {
                    start = index;
                }
            } else if token.kind == TokKind::Punct && (text == ">" || text == ">>" || text == ">>>") {
                let mut angles = if text == ">" { 1 } else if text == ">>" { 2 } else { 3 };
                let mut back = index;
                while back > 0 && angles > 0 {
                    back -= 1;
                    let inner = &self.source[self.tokens[back].start..self.tokens[back].end.min(self.source.len())];
                    if self.tokens[back].kind != TokKind::Punct {
                        continue;
                    }
                    if inner == "<" {
                        angles -= 1;
                    } else if inner == ">" {
                        angles += 1;
                    } else if inner == ">>" {
                        angles += 2;
                    } else if inner == ">>>" {
                        angles += 3;
                    } else if inner == ")" || inner == "]" || inner == "}" {
                        let mut depth = 1;
                        while back > 0 && depth > 0 {
                            back -= 1;
                            let nested = &self.source[self.tokens[back].start..self.tokens[back].end.min(self.source.len())];
                            if self.tokens[back].kind == TokKind::Punct {
                                if nested == ")" || nested == "]" || nested == "}" {
                                    depth += 1;
                                } else if nested == "(" || nested == "[" || nested == "{" {
                                    depth -= 1;
                                }
                            }
                        }
                    } else if inner == "(" || inner == "[" || inner == "{" || inner == ";" {
                        break;
                    }
                }
                if angles == 0 {
                    index = back;
                    start = back;
                    end = back.saturating_sub(1);
                } else {
                    break;
                }
            } else if matches!(token.kind, TokKind::Str | TokKind::Number | TokKind::TmplTail | TokKind::TmplNoSub) {
                start = index;
            } else {
                break;
            }
        }
        if start >= paren || end < start {
            return String::new();
        }
        let mut callee = self.source[self.tokens[start].start..self.tokens[end].end.min(self.source.len())].trim().to_owned();
        if callee.ends_with("?.") {
            callee.truncate(callee.len() - 2);
            callee = callee.trim_end().to_owned();
        }
        callee
    }

    fn skip_type_params_decl(&mut self) {
        if !self.at_punct(0, "<") {
            return;
        }
        let Some(close) = self.match_angle(self.pos) else {
            return;
        };
        let mut index = self.pos + 1;
        while index < close {
            let token = self.tokens[index];
            if token.kind == TokKind::Ident && matches!(self.text(token), "const" | "in" | "out") {
                index += 1;
                continue;
            }
            if token.kind == TokKind::Ident {
                let name = self.text(token).to_owned();
                self.record("TypeParameterDeclaration", name, token.start, token.end, None, None, None, &[]);
                index += 1;
                if index < close && matches!(self.text(self.tokens[index]), "extends" | "=") {
                    index += 1;
                    let mut depth = 0i32;
                    while index < close {
                        let text = self.text(self.tokens[index]);
                        if matches!(text, "(" | "[" | "{") {
                            depth += 1;
                        } else if matches!(text, ")" | "]" | "}") {
                            depth -= 1;
                        } else if depth == 0 && text == "," {
                            break;
                        }
                        index += 1;
                    }
                }
            } else if self.text(token) == "," {
                index += 1;
            } else if matches!(self.text(token), "(" | "[" | "{") {
                index = self.match_paren(index).map_or(close, |found| found + 1);
            } else {
                index += 1;
            }
        }
        self.pos = close + 1;
    }

    fn skip_type_args(&mut self) {
        if !self.at_punct(0, "<") {
            return;
        }
        if let Some(close) = self.match_angle(self.pos) {
            let saved = self.pos;
            self.pos += 1;
            while self.pos < close {
                if !self.parse_type_inner() {
                    self.pos += 1;
                }
            }
            self.pos = close + 1;
            let _ = saved;
        }
    }

    fn parse_type(&mut self) {
        self.guarded((), |walker| {
            walker.parse_type_inner();
        });
    }

    fn parse_type_inner(&mut self) -> bool {
        let mut consumed = false;
        let mut expect_operand = true;
        let mut extends_seen = false;
        let mut question_seen = false;
        loop {
            let Some(token) = self.peek() else { break };
            let text = self.text(token).to_owned();
            if expect_operand {
                match token.kind {
                    TokKind::Punct => match text.as_str() {
                        "(" => {
                            let Some(close) = self.match_paren(self.pos) else {
                                self.error_here("Unbalanced parenthesis in type");
                                self.bump();
                                break;
                            };
                            if self.tokens.get(close + 1).is_some_and(|token| self.text(*token) == "=>") {
                                let start = self.pos;
                                let save_decls = self.decls.len();
                                let save_diags = self.diagnostics.len();
                                self.parse_parameters("function-type");
                                if self.diagnostics.len() == save_diags && self.at_punct(0, "=>") {
                                    self.bump();
                                    self.parse_type();
                                    let start_offset = self.tokens[start].start;
                                    let end = self.tokens.get(self.pos.saturating_sub(1)).map_or(start_offset, |token| token.end);
                                    self.record("FunctionType", "anonymous".to_owned(), start_offset, end.max(start_offset), None, None, None, &[]);
                                } else {
                                    self.pos = start;
                                    self.decls.truncate(save_decls);
                                    self.diagnostics.truncate(save_diags);
                                    self.bump();
                                    self.parse_type();
                                    if self.at_punct(0, ")") {
                                        self.bump();
                                    } else {
                                        self.error_here("Unbalanced parenthesis in type");
                                    }
                                }
                            } else {
                                self.bump();
                                self.parse_type();
                                if self.at_punct(0, ")") {
                                    self.bump();
                                } else {
                                    self.error_here("Unbalanced parenthesis in type");
                                }
                            }
                            consumed = true;
                            expect_operand = false;
                        }
                        "[" => {
                            self.parse_type_brackets();
                            consumed = true;
                            expect_operand = false;
                        }
                        "{" => {
                            self.parse_type_literal();
                            consumed = true;
                            expect_operand = false;
                        }
                        "|" | "..." | "-" | "+" => {
                            self.bump();
                            consumed = true;
                        }
                        _ => break,
                    },
                    TokKind::Ident => {
                        if matches!(text.as_str(), "readonly" | "unique" | "abstract" | "in" | "out" | "const" | "keyof") {
                            self.bump();
                            consumed = true;
                            continue;
                        }
                        if text == "new" && self.at_punct(1, "(") {
                            self.parse_function_type(true);
                        } else if text == "infer" {
                            self.bump();
                            if matches!(self.peek(), Some(next) if next.kind == TokKind::Ident) {
                                self.bump();
                            }
                        } else if text == "import" && self.at_punct(1, "(") {
                            self.bump();
                            self.bump();
                            while !self.at_eof() && !self.at_punct(0, ")") {
                                self.bump();
                            }
                            if self.at_punct(0, ")") {
                                self.bump();
                            }
                            while self.at_punct(0, ".") {
                                self.bump();
                                if matches!(self.peek(), Some(next) if next.kind == TokKind::Ident) {
                                    self.bump();
                                }
                            }
                            if self.at_punct(0, "<") {
                                self.skip_type_args();
                            }
                        } else if text == "typeof" {
                            self.bump();
                            if self.at_punct(0, "(") {
                                self.bump();
                                self.scan_expression(None);
                                if self.at_punct(0, ")") {
                                    self.bump();
                                }
                            } else if matches!(self.peek(), Some(next) if next.kind == TokKind::Ident) {
                                self.bump();
                                while self.at_punct(0, ".") {
                                    self.bump();
                                    if matches!(self.peek(), Some(next) if next.kind == TokKind::Ident) {
                                        self.bump();
                                    }
                                }
                            }
                        } else if text == "asserts" {
                            self.bump();
                            if matches!(self.peek(), Some(next) if next.kind == TokKind::Ident) {
                                self.bump();
                            }
                        } else {
                            self.bump();
                            if self.at_punct(0, "<") {
                                self.skip_type_args();
                            }
                        }
                        consumed = true;
                        expect_operand = false;
                    }
                    TokKind::Str | TokKind::Number | TokKind::TmplNoSub => {
                        self.bump();
                        consumed = true;
                        expect_operand = false;
                    }
                    TokKind::TmplHead => {
                        self.skip_template_type();
                        consumed = true;
                        expect_operand = false;
                    }
                    _ => break,
                }
            } else {
                match token.kind {
                    TokKind::Punct => match text.as_str() {
                        "[" => {
                            self.parse_type_brackets();
                            consumed = true;
                        }
                        "<" => {
                            let before = self.pos;
                            self.skip_type_args();
                            if self.pos == before {
                                self.bump();
                                expect_operand = true;
                            }
                            consumed = true;
                        }
                        "?" => {
                            if !extends_seen || question_seen {
                                break;
                            }
                            self.bump();
                            question_seen = true;
                            consumed = true;
                            expect_operand = true;
                        }
                        ":" => {
                            if !extends_seen || !question_seen {
                                break;
                            }
                            self.bump();
                            extends_seen = false;
                            question_seen = false;
                            consumed = true;
                            expect_operand = true;
                        }
                        "|" | "&" | "." => {
                            self.bump();
                            consumed = true;
                            expect_operand = true;
                        }
                        _ => break,
                    },
                    TokKind::Ident => {
                        if text.as_str() == "extends" {
                            self.bump();
                            extends_seen = true;
                            question_seen = false;
                            consumed = true;
                            expect_operand = true;
                        } else if matches!(
                            text.as_str(),
                            "is" | "in" | "out" | "keyof" | "typeof" | "readonly" | "unique" | "abstract" | "infer"
                        ) {
                            self.bump();
                            consumed = true;
                            expect_operand = true;
                        } else {
                            break;
                        }
                    }
                    _ => break,
                }
            }
        }
        consumed
    }

    fn parse_type_brackets(&mut self) {
        self.bump();
        if self.at_punct(0, "]") {
            self.bump();
            return;
        }
        loop {
            if self.at_punct(0, "...") {
                self.bump();
            }
            if matches!(self.peek(), Some(token) if token.kind == TokKind::Ident)
                && (self.at_punct(1, ":") || (self.at_punct(1, "?") && self.at_punct(2, ":")))
            {
                self.bump();
                if self.at_punct(0, "?") {
                    self.bump();
                }
                self.bump();
            }
            self.parse_type();
            if self.at_punct(0, ",") {
                self.bump();
            } else {
                break;
            }
        }
        if self.at_punct(0, "]") {
            self.bump();
        } else {
            self.error_here("Unbalanced brackets in type");
        }
    }

    fn skip_template_type(&mut self) {
        let mut depth = 0;
        while let Some(token) = self.peek() {
            match token.kind {
                TokKind::TmplHead => {
                    depth += 1;
                    self.bump();
                }
                TokKind::TmplMid => {
                    self.bump();
                }
                TokKind::TmplTail => {
                    self.bump();
                    if depth <= 1 {
                        return;
                    }
                    depth -= 1;
                }
                TokKind::TmplNoSub => {
                    self.bump();
                    if depth == 0 {
                        return;
                    }
                }
                _ => {
                    if token.kind == TokKind::Punct && self.text(token) == "}" && depth > 0 {
                        self.bump();
                    } else {
                        self.bump();
                    }
                }
            }
        }
    }

    fn parse_function_type(&mut self, constructor: bool) {
        let start_token = self.peek().map(|token| token.start).unwrap_or(self.source.len());
        if constructor {
            self.bump();
        }
        self.parse_parameters("FunctionType");
        if self.at_punct(0, "=>") {
            self.bump();
            self.parse_type();
        }
        let end = self.tokens.get(self.pos.saturating_sub(1)).map_or(start_token, |token| token.end);
        self.record(
            if constructor { "ConstructorType" } else { "FunctionType" },
            "anonymous".to_owned(),
            start_token,
            end,
            None,
            None,
            None,
            &[],
        );
    }

    fn parse_type_literal(&mut self) {
        self.guarded((), |walker| {
            walker.bump();
            while !walker.at_eof() && !walker.at_punct(0, "}") {
                if walker.at_punct(0, ";") || walker.at_punct(0, ",") {
                    walker.bump();
                    continue;
                }
                walker.parse_interface_member();
            }
            if walker.at_punct(0, "}") {
                walker.bump();
            }
        });
    }

    fn parse_parameters(&mut self, owner: &str) {
        if !self.at_punct(0, "(") {
            self.error_here("Expected a parameter list");
            return;
        }
        let _ = owner;
        self.bump();
        while !self.at_eof() && !self.at_punct(0, ")") {
            if self.at_punct(0, ",") {
                self.bump();
                continue;
            }
            while self.peek().is_some_and(|token| {
                token.kind == TokKind::Ident
                    && matches!(self.text(token), "public" | "private" | "protected" | "readonly" | "override")
            }) && self.peek_at(1).is_some_and(|token| {
                token.kind == TokKind::Ident || (token.kind == TokKind::Punct && matches!(self.text(token), "{" | "["))
            }) {
                self.bump();
            }
            if self.at_punct(0, "...") {
                self.bump();
            }
            let Some(name_token) = self.peek() else { break };
            if name_token.kind == TokKind::Punct && (self.text(name_token) == "{" || self.text(name_token) == "[") {
                let pattern_start = name_token.start;
                let pattern_end = self.match_paren(self.pos).map_or(name_token.end, |close| self.tokens[close].end);
                let name = self.source[pattern_start..pattern_end.min(self.source.len())].to_owned();
                self.parse_binding_pattern();
                if self.at_punct(0, ":") {
                    self.bump();
                    self.parse_type();
                }
                let mut initializer_start = None;
                if self.at_punct(0, "=") {
                    self.bump();
                    initializer_start = self.peek().map(|token| token.start);
                    self.scan_expression(None);
                }
                self.record("Parameter", name, pattern_start, self.tokens.get(self.pos.saturating_sub(1)).map_or(pattern_end, |token| token.end), None, None, initializer_start, &[]);
            } else if name_token.kind == TokKind::Ident {
                let name = self.text(name_token).to_owned();
                let start = name_token.start;
                self.bump();
                if self.at_punct(0, "?") {
                    self.bump();
                }
                if self.at_punct(0, ":") {
                    self.bump();
                    self.parse_type();
                }
                let mut initializer_start = None;
                if self.at_punct(0, "=") {
                    self.bump();
                    initializer_start = self.peek().map(|token| token.start);
                    self.scan_expression(None);
                }
                let end = self.tokens.get(self.pos.saturating_sub(1)).map_or(start, |token| token.end);
                self.record("Parameter", name, start, end.max(start), None, None, initializer_start, &[]);
            } else {
                self.error_here("Unexpected parameter");
                self.recover();
                break;
            }
        }
        if self.at_punct(0, ")") {
            self.bump();
        } else {
            self.error_here("Unbalanced parameter list");
        }
    }

    fn parse_binding_pattern(&mut self) {
        self.guarded((), |walker| {
            let Some(open) = walker.peek() else { return };
            let closer = if walker.text(open) == "{" { "}" } else { "]" };
            walker.bump();
            while !walker.at_eof() && !walker.at_punct(0, closer) {
                if walker.at_punct(0, ",") || walker.at_punct(0, ";") {
                    walker.bump();
                    continue;
                }
                if walker.at_punct(0, "...") {
                    walker.bump();
                    if matches!(walker.peek(), Some(next) if next.kind == TokKind::Ident) {
                        let token = walker.bump().expect("binding rest");
                        let name = walker.text(token).to_owned();
                        walker.record("BindingElement", name, token.start, token.end, None, None, None, &[]);
                    }
                    continue;
                }
                let Some(token) = walker.peek() else { break };
                if token.kind == TokKind::Punct && (walker.text(token) == "{" || walker.text(token) == "[") {
                    walker.parse_binding_pattern();
                    if walker.at_punct(0, "=") {
                        walker.bump();
                        walker.scan_expression(None);
                    }
                    continue;
                }
                if token.kind != TokKind::Ident && token.kind != TokKind::Str && token.kind != TokKind::Number {
                    walker.error_here("Unexpected binding element");
                    walker.recover();
                    break;
                }
                let key = walker.text(token).to_owned();
                let start = token.start;
                walker.bump();
                let mut name = key;
                if walker.at_punct(0, ":") {
                    walker.bump();
                    match walker.peek() {
                        Some(next) if walker.text(next) == "{" || walker.text(next) == "[" => {
                            walker.parse_binding_pattern();
                        }
                        Some(next) if next.kind == TokKind::Ident => {
                            name = walker.text(next).to_owned();
                            walker.bump();
                        }
                        _ => {
                            walker.error_here("Unexpected binding target");
                            walker.recover();
                            break;
                        }
                    }
                }
                let mut initializer_start = None;
                if walker.at_punct(0, "=") {
                    walker.bump();
                    initializer_start = walker.peek().map(|token| token.start);
                    walker.scan_expression(None);
                }
                let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(start, |token| token.end);
                walker.record("BindingElement", name, start, end.max(start), None, None, initializer_start, &[]);
            }
            if walker.at_punct(0, closer) {
                walker.bump();
            }
        });
    }

    fn parse_block_body(&mut self) -> Option<(usize, usize)> {
        if !self.at_punct(0, "{") {
            return None;
        }
        let open = self.bump().expect("block open");
        self.guarded((), |walker| {
            while !walker.at_eof() && !walker.at_punct(0, "}") {
                walker.parse_statement();
            }
        });
        if self.at_punct(0, "}") {
            let close = self.bump().expect("block close");
            Some((open.start, close.end))
        } else {
            self.error_here("Unbalanced block");
            Some((open.start, self.source.len()))
        }
    }

    fn parse_function(&mut self, declaration: bool, context: Option<String>, own_exports: &[String], start_override: Option<usize>) {
        self.guarded((), |walker| {
            let start_token = walker.peek().map(|token| token.start).unwrap_or(walker.source.len());
            let start = start_override.unwrap_or(start_token);
            walker.bump();
            if walker.at_punct(0, "*") {
                walker.bump();
            }
            let mut name = context;
            if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident) {
                let token = walker.bump().expect("function name");
                name = Some(walker.text(token).to_owned());
            }
            walker.skip_type_params_decl();
            if walker.at_punct(0, "(") {
                walker.parse_parameters("function");
            } else {
                walker.error_here("Expected a parameter list");
                walker.recover();
                return;
            }
            if walker.at_punct(0, ":") {
                walker.bump();
                walker.parse_type();
            }
            let kind = if declaration { "FunctionDeclaration" } else { "FunctionExpression" };
            let name = name.unwrap_or_else(|| "anonymous".to_owned());
            if walker.at_punct(0, "{") {
                let open = walker.bump().expect("function body");
                walker.frames.push(Frame {
                    kind,
                    name: name.clone(),
                    exports: own_exports.to_vec(),
                });
                walker.guarded((), |inner| {
                    while !inner.at_eof() && !inner.at_punct(0, "}") {
                        inner.parse_statement();
                    }
                });
                let body = if walker.at_punct(0, "}") {
                    let close = walker.bump().expect("function close");
                    Some((open.start, close.end))
                } else {
                    walker.error_here("Unbalanced function body");
                    Some((open.start, walker.source.len()))
                };
                walker.frames.pop();
                let end = body.map_or(start_token, |(_, end)| end);
                walker.record(kind, name.clone(), start, end.max(start), body, None, None, own_exports);
            } else {
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(start, |token| token.end);
                walker.record(kind, name, start, end.max(start), None, None, None, own_exports);
            }
        });
    }

    fn match_angle_before(&self, index: usize) -> Option<usize> {
        if index == 0 || !matches!(self.text(self.tokens[index - 1]), ">" | ">>" | ">>>") {
            return None;
        }
        let mut balance = 0i32;
        let mut scan = index;
        while scan > 0 {
            scan -= 1;
            let text = self.text(self.tokens[scan]);
            if text == ">" || text == ">>" || text == ">>>" {
                balance += 1;
            } else if text == "<" {
                balance -= 1;
                if balance == 0 {
                    return Some(scan);
                }
            } else if matches!(text, ";" | "{" | "}" | "(" | ")") {
                return None;
            }
        }
        None
    }

    fn arrow_start_from(&self, known: usize) -> usize {
        let mut begin = known.min(self.tokens.len().saturating_sub(1));
        if let Some(open) = self.match_angle_before(begin) {
            begin = open;
        }
        if begin > 0 {
            let before = self.tokens[begin - 1];
            if before.kind == TokKind::Ident && self.text(before) == "async" {
                begin -= 1;
            }
        }
        self.tokens[begin].start
    }

    fn arrow_equals_name_at(&self, head: usize) -> Option<String> {
        let mut index = head;
        if let Some(open) = self.match_angle_before(index) {
            index = open;
        }
        if index > 0 {
            let before = self.tokens[index - 1];
            if before.kind == TokKind::Ident && self.text(before) == "async" {
                index -= 1;
            }
        }
        self.equals_name_before(index)
    }

    fn parse_arrow_at(&mut self, arrow: usize, context: Option<String>, start: usize) {
        self.guarded((), |walker| {
            walker.pos = arrow + 1;
            let body_start_token = walker.peek().map(|token| token.start).unwrap_or(walker.source.len());
            let name = context.unwrap_or_else(|| "anonymous".to_owned());
            walker.frames.push(Frame {
                kind: "ArrowFunction",
                name: name.clone(),
                exports: Vec::new(),
            });
            let (body, end) = if walker.at_punct(0, "{") {
                let open = walker.bump().expect("arrow block");
                walker.guarded((), |inner| {
                    while !inner.at_eof() && !inner.at_punct(0, "}") {
                        inner.parse_statement();
                    }
                });
                if walker.at_punct(0, "}") {
                    let close = walker.bump().expect("arrow close");
                    (Some((open.start, close.end)), close.end)
                } else {
                    walker.error_here("Unbalanced arrow body");
                    (Some((open.start, walker.source.len())), walker.source.len())
                }
            } else {
                walker.scan_expression(None);
                let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(body_start_token, |token| token.end);
                (Some((body_start_token, end)), end)
            };
            walker.frames.pop();
            walker.record("ArrowFunction", name, start, end.max(start), body, None, None, &[]);
        });
    }

    fn parse_class(&mut self, declaration: bool, context: Option<String>, own_exports: &[String], start_override: Option<usize>) {
        self.guarded((), |walker| {
            let start_token = walker.peek().map(|token| token.start).unwrap_or(walker.source.len());
            let start = start_override.unwrap_or(start_token);
            walker.bump();
            let mut name = context;
            if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident) {
                let token = walker.bump().expect("class name");
                name = Some(walker.text(token).to_owned());
            }
            walker.skip_type_params_decl();
            if walker.at_ident(0, "extends") {
                walker.bump();
                while let Some(next) = walker.peek() {
                    if next.kind == TokKind::Ident || (next.kind == TokKind::Punct && matches!(walker.text(next), "." | "?.")) {
                        walker.bump();
                    } else {
                        break;
                    }
                }
                if walker.at_punct(0, "<") {
                    walker.skip_type_args();
                }
            }
            if walker.at_ident(0, "implements") {
                walker.bump();
                loop {
                    walker.parse_type();
                    if walker.at_punct(0, ",") {
                        walker.bump();
                    } else {
                        break;
                    }
                }
            }
            let kind = if declaration { "ClassDeclaration" } else { "ClassExpression" };
            let name = name.unwrap_or_else(|| "anonymous".to_owned());
            if !walker.at_punct(0, "{") {
                walker.error_here("Expected a class body");
                walker.record(kind, name, start, start, None, None, None, own_exports);
                return;
            }
            let open = walker.bump().expect("class open");
            let members_pos = Some(open.end);
            walker.frames.push(Frame {
                kind,
                name: name.clone(),
                exports: own_exports.to_vec(),
            });
            let frame_index = walker.decls.len();
            walker.guarded((), |inner| {
                while !inner.at_eof() && !inner.at_punct(0, "}") {
                    let before = inner.pos;
                    inner.parse_class_member();
                    if inner.pos == before && !inner.at_eof() {
                        inner.bump();
                    }
                }
            });
            if walker.at_punct(0, "}") {
                walker.bump();
            } else {
                walker.error_here("Unbalanced class body");
            }
            let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(open.end, |token| token.end);
            walker.frames.pop();
            let enclosing = walker.enclosing();
            let exports = walker.exports(own_exports);
            walker.decls.insert(
                frame_index,
                DeclInfo {
                    kind,
                    name,
                    start,
                    end: end.max(start),
                    body: None,
                    members_pos,
                    initializer_start: None,
                    enclosing,
                    exports,
                },
            );
        });
    }

    fn member_name(&mut self) -> Option<(String, usize)> {
        let token = self.peek()?;
        if token.kind == TokKind::Punct && self.text(token) == "[" {
            let close = self.match_paren(self.pos)?;
            let start = token.start;
            let end = self.tokens[close].end;
            let name = self.source[start..end.min(self.source.len())].to_owned();
            self.pos = close + 1;
            return Some((name, start));
        }
        if matches!(token.kind, TokKind::Ident | TokKind::Str | TokKind::Number) {
            let name = self.text(token).to_owned();
            let start = token.start;
            self.bump();
            return Some((name, start));
        }
        None
    }

    fn parse_method_common(
        &mut self,
        start: usize,
        name: String,
        kind: &'static str,
        in_class: bool,
    ) {
        self.guarded((), |walker| {
            walker.skip_type_params_decl();
            if walker.at_punct(0, "(") {
                walker.parse_parameters("method");
            } else {
                walker.error_here("Expected a parameter list");
                walker.recover();
                return;
            }
            if walker.at_punct(0, ":") {
                walker.bump();
                walker.parse_type();
            }
            if walker.at_punct(0, "{") {
                let open = walker.bump().expect("method body");
                walker.frames.push(Frame {
                    kind,
                    name: name.clone(),
                    exports: Vec::new(),
                });
                walker.guarded((), |inner| {
                    while !inner.at_eof() && !inner.at_punct(0, "}") {
                        inner.parse_statement();
                    }
                });
                let body = if walker.at_punct(0, "}") {
                    let close = walker.bump().expect("method close");
                    Some((open.start, close.end))
                } else {
                    walker.error_here("Unbalanced method body");
                    Some((open.start, walker.source.len()))
                };
                walker.frames.pop();
                let end = body.map_or(start, |(_, end)| end);
                walker.record(kind, name, start, end.max(start), body, None, None, &[]);
            } else {
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(start, |token| token.end);
                walker.record(kind, name, start, end.max(start), None, None, None, &[]);
            }
            let _ = in_class;
        });
    }

    fn parse_class_member(&mut self) {
        self.guarded((), |walker| {
            if walker.at_punct(0, ";") {
                walker.bump();
                return;
            }
            while walker.at_punct(0, "@") {
                walker.skip_decorator();
            }
            let start = walker.peek().map(|token| token.start).unwrap_or(walker.source.len());
            while walker.peek().is_some_and(|token| {
                token.kind == TokKind::Ident
                    && matches!(
                        walker.text(token),
                        "public" | "private" | "protected" | "static" | "readonly" | "abstract" | "declare" | "override" | "async"
                    )
            }) && walker.peek_at(1).is_some_and(|token| {
                token.kind == TokKind::Ident
                    || token.kind == TokKind::Str
                    || token.kind == TokKind::Number
                    || (token.kind == TokKind::Punct && matches!(walker.text(token), "[" | "*" | "{"))
            }) {
                if walker.at_ident(0, "static") && walker.at_punct(1, "{") {
                    walker.bump();
                    walker.parse_block_body();
                    return;
                }
                if walker.at_ident(0, "async")
                    && !matches!(walker.peek_text(1), Some("(") | Some("<"))
                    && walker.peek_at(1).is_some_and(|token| token.kind == TokKind::Ident)
                {
                    walker.bump();
                    break;
                }
                if walker.at_ident(0, "async") {
                    break;
                }
                walker.bump();
            }
            if walker.at_punct(0, "*") {
                walker.bump();
            }
            if walker.at_ident(0, "constructor") && walker.at_punct(1, "(") {
                walker.bump();
                walker.parse_method_common(start, "constructor".to_owned(), "Constructor", true);
                return;
            }
            if matches!(walker.peek_text(0), Some("get") | Some("set")) {
                let accessor = walker.peek_text(0).unwrap_or("get").to_owned();
                let is_get = accessor == "get";
                let next = walker.peek_at(1);
                let followed = walker.peek_at(2);
                let nameish = next.is_some_and(|token| {
                    matches!(token.kind, TokKind::Ident | TokKind::Str | TokKind::Number)
                        || (token.kind == TokKind::Punct && walker.text(token) == "[")
                });
                if nameish && followed.is_some_and(|token| token.kind == TokKind::Punct && walker.text(token) == "(") {
                    walker.bump();
                    let (name, _) = walker.member_name().expect("accessor name");
                    walker.parse_method_common(start, name, if is_get { "GetAccessor" } else { "SetAccessor" }, true);
                    return;
                }
            }
            if walker.at_punct(0, "[")
                && walker.peek_at(1).is_some_and(|token| token.kind == TokKind::Ident)
                && walker.peek_text(2) == Some(":")
            {
                while !walker.at_eof() && !walker.at_punct(0, ";") && !walker.at_punct(0, "}") {
                    if walker.at_punct(0, "(") || walker.at_punct(0, "[") || walker.at_punct(0, "{") {
                        walker.skip_balanced();
                    } else if walker.at_punct(0, "<") {
                        if let Some(close) = walker.match_angle(walker.pos) {
                            walker.pos = close + 1;
                        } else {
                            walker.bump();
                        }
                    } else {
                        walker.bump();
                    }
                }
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                return;
            }
            let Some((name, _)) = walker.member_name() else {
                walker.error_here("Unexpected class member");
                walker.recover();
                return;
            };
            if walker.at_punct(0, "?") {
                walker.bump();
            }
            if walker.at_punct(0, "<") && walker.match_angle(walker.pos).is_some() {
                let close = walker.match_angle(walker.pos).expect("member type args");
                if walker.tokens.get(close + 1).is_some_and(|token| walker.text(*token) == "(") {
                    walker.parse_method_common(start, name, "MethodDeclaration", true);
                    return;
                }
            }
            if walker.at_punct(0, "(") {
                walker.parse_method_common(start, name, "MethodDeclaration", true);
                return;
            }
            if walker.at_punct(0, "!") {
                walker.bump();
            }
            if walker.at_punct(0, ":") {
                walker.bump();
                walker.parse_type();
            }
            let mut initializer_start = None;
            if walker.at_punct(0, "=") {
                walker.bump();
                initializer_start = walker.peek().map(|token| token.start);
                walker.scan_expression(Some(name.clone()));
            }
            if walker.at_punct(0, ";") {
                walker.bump();
            }
            let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(start, |token| token.end);
            walker.record("PropertyDeclaration", name, start, end.max(start), None, None, initializer_start, &[]);
        });
    }

    fn skip_decorator(&mut self) {
        self.bump();
        while let Some(token) = self.peek() {
            if token.kind == TokKind::Ident || (token.kind == TokKind::Punct && matches!(self.text(token), "." | "?."))
            {
                self.bump();
            } else if token.kind == TokKind::Punct && self.text(token) == "(" {
                let open = self.pos;
                self.bump();
                while !self.at_eof() && !self.at_punct(0, ")") {
                    if self.at_punct(0, ",") {
                        self.bump();
                        continue;
                    }
                    let callee = self.callee_text(open);
                    let before = self.pos;
                    let name = if callee.is_empty() { None } else { Some(format!("{callee} callback")) };
                    self.scan_expression(name);
                    if self.pos == before {
                        self.error_here("Unexpected token in arguments");
                        self.bump();
                    }
                }
                if self.at_punct(0, ")") {
                    self.bump();
                }
                break;
            } else {
                break;
            }
        }
    }

    fn parse_interface(&mut self, start: usize, own_exports: &[String]) {
        self.guarded((), |walker| {
            walker.bump();
            let mut name = "anonymous".to_owned();
            if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident) {
                let token = walker.bump().expect("interface name");
                name = walker.text(token).to_owned();
            }
            walker.skip_type_params_decl();
            if walker.at_ident(0, "extends") {
                walker.bump();
                loop {
                    walker.parse_type();
                    if walker.at_punct(0, ",") {
                        walker.bump();
                    } else {
                        break;
                    }
                }
            }
            if !walker.at_punct(0, "{") {
                walker.error_here("Expected an interface body");
                walker.record("InterfaceDeclaration", name, start, start, None, None, None, own_exports);
                return;
            }
            let open = walker.bump().expect("interface open");
            let members_pos = Some(open.end);
            walker.frames.push(Frame {
                kind: "InterfaceDeclaration",
                name: name.clone(),
                exports: own_exports.to_vec(),
            });
            let frame_index = walker.decls.len();
            walker.guarded((), |inner| {
                while !inner.at_eof() && !inner.at_punct(0, "}") {
                    if inner.at_punct(0, ";") || inner.at_punct(0, ",") {
                        inner.bump();
                        continue;
                    }
                    let before = inner.pos;
                    inner.parse_interface_member();
                    if inner.pos == before && !inner.at_eof() {
                        inner.bump();
                    }
                }
            });
            if walker.at_punct(0, "}") {
                walker.bump();
            } else {
                walker.error_here("Unbalanced interface body");
            }
            let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(open.end, |token| token.end);
            walker.frames.pop();
            let enclosing = walker.enclosing();
            let exports = walker.exports(own_exports);
            walker.decls.insert(
                frame_index,
                DeclInfo {
                    kind: "InterfaceDeclaration",
                    name,
                    start,
                    end: end.max(start),
                    body: None,
                    members_pos,
                    initializer_start: None,
                    enclosing,
                    exports,
                },
            );
        });
    }

    fn parse_interface_member(&mut self) {
        self.guarded((), |walker| {
            let start = walker.peek().map(|token| token.start).unwrap_or(walker.source.len());
            loop {
                if walker.at_ident(0, "readonly") {
                    walker.bump();
                } else if (walker.at_punct(0, "-") || walker.at_punct(0, "+"))
                    && (walker.at_ident(1, "readonly") || walker.at_punct(1, "?"))
                {
                    walker.bump();
                    walker.bump();
                } else {
                    break;
                }
            }
            if walker.at_punct(0, "(") {
                walker.parse_parameters("signature");
                if walker.at_punct(0, ":") {
                    walker.bump();
                    walker.parse_type();
                }
                if walker.at_punct(0, ";") || walker.at_punct(0, ",") {
                    walker.bump();
                }
                let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(start, |token| token.end);
                walker.record("CallSignature", "call".to_owned(), start, end.max(start), None, None, None, &[]);
                return;
            }
            if walker.at_ident(0, "new") && walker.at_punct(1, "(") {
                walker.bump();
                walker.parse_parameters("signature");
                if walker.at_punct(0, ":") {
                    walker.bump();
                    walker.parse_type();
                }
                if walker.at_punct(0, ";") || walker.at_punct(0, ",") {
                    walker.bump();
                }
                let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(start, |token| token.end);
                walker.record("ConstructSignature", "constructor".to_owned(), start, end.max(start), None, None, None, &[]);
                return;
            }
            if matches!(walker.peek_text(0), Some("get") | Some("set")) {
                let accessor = walker.peek_text(0).unwrap_or("get").to_owned();
                let is_get = accessor == "get";
                let next = walker.peek_at(1);
                let followed = walker.peek_at(2);
                let nameish = next.is_some_and(|token| {
                    matches!(token.kind, TokKind::Ident | TokKind::Str | TokKind::Number)
                        || (token.kind == TokKind::Punct && walker.text(token) == "[")
                });
                if nameish && followed.is_some_and(|token| token.kind == TokKind::Punct && walker.text(token) == "(") {
                    walker.bump();
                    let (name, _) = walker.member_name().expect("accessor name");
                    walker.skip_type_params_decl();
                    walker.parse_parameters("accessor");
                    if walker.at_punct(0, ":") {
                        walker.bump();
                        walker.parse_type();
                    }
                    if walker.at_punct(0, ";") || walker.at_punct(0, ",") {
                        walker.bump();
                    }
                    let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(start, |token| token.end);
                    walker.record(if is_get { "GetAccessor" } else { "SetAccessor" }, name, start, end.max(start), None, None, None, &[]);
                    return;
                }
            }
            if walker.at_punct(0, "[") {
                while !walker.at_eof() && !walker.at_punct(0, ";") && !walker.at_punct(0, ",") && !walker.at_punct(0, "}") {
                    if walker.at_punct(0, "(") || walker.at_punct(0, "[") || walker.at_punct(0, "{") {
                        walker.skip_balanced();
                    } else if walker.at_punct(0, "<") {
                        if let Some(close) = walker.match_angle(walker.pos) {
                            walker.pos = close + 1;
                        } else {
                            walker.bump();
                        }
                    } else {
                        walker.bump();
                    }
                }
                if walker.at_punct(0, ";") || walker.at_punct(0, ",") {
                    walker.bump();
                }
                return;
            }
            let Some((name, _)) = walker.member_name() else {
                walker.error_here("Unexpected interface member");
                walker.recover();
                return;
            };
            if walker.at_punct(0, "?") {
                walker.bump();
            }
            if walker.at_punct(0, "<") {
                let close = walker.match_angle(walker.pos);
                if close.is_some_and(|found| walker.tokens.get(found + 1).is_some_and(|token| walker.text(*token) == "(")) {
                    walker.skip_type_params_decl();
                    walker.parse_parameters("method");
                    if walker.at_punct(0, ":") {
                        walker.bump();
                        walker.parse_type();
                    }
                    if walker.at_punct(0, ";") || walker.at_punct(0, ",") {
                        walker.bump();
                    }
                    let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(start, |token| token.end);
                    walker.record("MethodSignature", name, start, end.max(start), None, None, None, &[]);
                    return;
                }
            }
            if walker.at_punct(0, "(") {
                walker.parse_parameters("method");
                if walker.at_punct(0, ":") {
                    walker.bump();
                    walker.parse_type();
                }
                if walker.at_punct(0, ";") || walker.at_punct(0, ",") {
                    walker.bump();
                }
                let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(start, |token| token.end);
                walker.record("MethodSignature", name, start, end.max(start), None, None, None, &[]);
                return;
            }
            if walker.at_punct(0, ":") {
                walker.bump();
                walker.parse_type();
            }
            if walker.at_punct(0, ";") || walker.at_punct(0, ",") {
                walker.bump();
            }
            let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(start, |token| token.end);
            walker.record("PropertySignature", name, start, end.max(start), None, None, None, &[]);
        });
    }

    fn parse_enum(&mut self, start: usize, own_exports: &[String]) {
        self.guarded((), |walker| {
            walker.bump();
            let mut name = "anonymous".to_owned();
            if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident) {
                let token = walker.bump().expect("enum name");
                name = walker.text(token).to_owned();
            }
            if !walker.at_punct(0, "{") {
                walker.error_here("Expected an enum body");
                walker.record("EnumDeclaration", name, start, start, None, None, None, own_exports);
                return;
            }
            let open = walker.bump().expect("enum open");
            let members_pos = Some(open.end);
            walker.frames.push(Frame {
                kind: "EnumDeclaration",
                name: name.clone(),
                exports: own_exports.to_vec(),
            });
            let frame_index = walker.decls.len();
            walker.guarded((), |inner| {
                while !inner.at_eof() && !inner.at_punct(0, "}") {
                    if inner.at_punct(0, ",") || inner.at_punct(0, ";") {
                        inner.bump();
                        continue;
                    }
                    let member_start = inner.peek().map(|token| token.start).unwrap_or(inner.source.len());
                    let member = inner.member_name();
                    let Some((name, _)) = member else {
                        inner.error_here("Unexpected enum member");
                        inner.recover();
                        break;
                    };
                    let mut initializer_start = None;
                    if inner.at_punct(0, "=") {
                        inner.bump();
                        initializer_start = inner.peek().map(|token| token.start);
                        inner.scan_expression(None);
                    }
                    if inner.at_punct(0, ",") || inner.at_punct(0, ";") {
                        inner.bump();
                    }
                    let end = inner.tokens.get(inner.pos.saturating_sub(1)).map_or(member_start, |token| token.end);
                    inner.record("EnumMember", name, member_start, end.max(member_start), None, None, initializer_start, &[]);
                }
            });
            if walker.at_punct(0, "}") {
                walker.bump();
            } else {
                walker.error_here("Unbalanced enum body");
            }
            let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(open.end, |token| token.end);
            walker.frames.pop();
            let enclosing = walker.enclosing();
            let exports = walker.exports(own_exports);
            walker.decls.insert(
                frame_index,
                DeclInfo {
                    kind: "EnumDeclaration",
                    name,
                    start,
                    end: end.max(start),
                    body: None,
                    members_pos,
                    initializer_start: None,
                    enclosing,
                    exports,
                },
            );
        });
    }

    fn parse_type_alias(&mut self, start: usize, own_exports: &[String]) {
        self.guarded((), |walker| {
            walker.bump();
            let mut name = "anonymous".to_owned();
            if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident) {
                let token = walker.bump().expect("alias name");
                name = walker.text(token).to_owned();
            }
            walker.skip_type_params_decl();
            if walker.at_punct(0, "=") {
                walker.bump();
                walker.parse_type();
            } else {
                walker.error_here("Expected = in a type alias");
            }
            if walker.at_punct(0, ";") {
                walker.bump();
            }
            let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(start, |token| token.end);
            walker.record("TypeAliasDeclaration", name, start, end.max(start), None, None, None, own_exports);
        });
    }

    fn parse_module(&mut self, start: usize, own_exports: &[String]) {
        self.guarded((), |walker| {
            walker.bump();
            let mut name = "anonymous".to_owned();
            if matches!(walker.peek(), Some(token) if token.kind == TokKind::Str) {
                let token = walker.bump().expect("module name");
                name = walker.text(token).to_owned();
            } else if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident) {
                let first = walker.bump().expect("module name");
                name = walker.text(first).to_owned();
                while walker.at_punct(0, ".") {
                    walker.bump();
                    if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident) {
                        let part = walker.bump().expect("module part");
                        name.push('.');
                        name.push_str(walker.text(part));
                    }
                }
            }
            if !walker.at_punct(0, "{") {
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(start, |token| token.end);
                walker.record("ModuleDeclaration", name, start, end.max(start), None, None, None, own_exports);
                return;
            }
            let open = walker.bump().expect("module open");
            walker.frames.push(Frame {
                kind: "ModuleDeclaration",
                name: name.clone(),
                exports: own_exports.to_vec(),
            });
            let frame_index = walker.decls.len();
            walker.guarded((), |inner| {
                while !inner.at_eof() && !inner.at_punct(0, "}") {
                    inner.parse_statement();
                }
            });
            if walker.at_punct(0, "}") {
                walker.bump();
            } else {
                walker.error_here("Unbalanced module body");
            }
            let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(open.end, |token| token.end);
            walker.frames.pop();
            let enclosing = walker.enclosing();
            let exports = walker.exports(own_exports);
            walker.decls.insert(
                frame_index,
                DeclInfo {
                    kind: "ModuleDeclaration",
                    name,
                    start,
                    end: end.max(start),
                    body: None,
                    members_pos: Some(open.end),
                    initializer_start: None,
                    enclosing,
                    exports,
                },
            );
        });
    }

    fn parse_variable_declarators(&mut self, own_exports: &[String]) {
        loop {
            let Some(token) = self.peek() else { break };
            if token.kind == TokKind::Punct && (self.text(token) == "{" || self.text(token) == "[") {
                let pattern_start = token.start;
                let pattern_end = self.match_paren(self.pos).map_or(token.end, |close| self.tokens[close].end);
                let name = self.source[pattern_start..pattern_end.min(self.source.len())].to_owned();
                self.parse_binding_pattern();
                if self.at_punct(0, ":") {
                    self.bump();
                    self.parse_type();
                }
                let mut initializer_start = None;
                if self.at_punct(0, "=") {
                    self.bump();
                    initializer_start = self.peek().map(|token| token.start);
                    self.scan_expression(Some(name.clone()));
                }
                let end = self.tokens.get(self.pos.saturating_sub(1)).map_or(pattern_start, |token| token.end);
                self.record("VariableDeclaration", name, pattern_start, end.max(pattern_start), None, None, initializer_start, own_exports);
            } else if token.kind == TokKind::Ident {
                let name = self.text(token).to_owned();
                let start = token.start;
                self.bump();
                if self.at_punct(0, "!") {
                    self.bump();
                }
                if self.at_punct(0, ":") {
                    self.bump();
                    self.parse_type();
                }
                let mut initializer_start = None;
                if self.at_punct(0, "=") {
                    self.bump();
                    initializer_start = self.peek().map(|token| token.start);
                    self.scan_expression(Some(name.clone()));
                }
                let end = self.tokens.get(self.pos.saturating_sub(1)).map_or(start, |token| token.end);
                self.record("VariableDeclaration", name, start, end.max(start), None, None, initializer_start, own_exports);
            } else {
                break;
            }
            if self.at_punct(0, ",") {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn parse_import(&mut self) {
        self.guarded((), |walker| {
            walker.bump();
            if matches!(walker.peek(), Some(token) if token.kind == TokKind::Str) {
                let spec = walker.bump().expect("import spec");
                walker.imports.push(walker.text(spec).to_owned());
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                return;
            }
            if walker.at_ident(0, "type") {
                walker.bump();
                if matches!(walker.peek(), Some(token) if token.kind == TokKind::Str) {
                    let spec = walker.bump().expect("import spec");
                    walker.imports.push(walker.text(spec).to_owned());
                    if walker.at_punct(0, ";") {
                        walker.bump();
                    }
                    return;
                }
            }
            if walker.at_punct(0, "(") || walker.peek().is_none() {
                walker.skip_balanced();
                return;
            }
            let clause_start = walker.peek().map(|token| token.start).unwrap_or(walker.source.len());
            let mut default_name: Option<String> = None;
            if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident) {
                let token = walker.bump().expect("default import");
                default_name = Some(walker.text(token).to_owned());
            }
            if default_name.is_some() && walker.at_punct(0, ",") {
                walker.bump();
            }
            if walker.at_punct(0, "{") {
                walker.bump();
                while !walker.at_eof() && !walker.at_punct(0, "}") {
                    if walker.at_punct(0, ",") {
                        walker.bump();
                        continue;
                    }
                    if walker.at_ident(0, "type") {
                        walker.bump();
                    }
                    let spec_start = walker.peek().map(|token| token.start).unwrap_or(walker.source.len());
                    if !matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident || token.kind == TokKind::Str) {
                        walker.error_here("Unexpected import specifier");
                        walker.recover();
                        break;
                    }
                    let first = walker.bump().expect("import name");
                    let first_text = walker.text(first).to_owned();
                    let mut local = first_text;
                    if walker.at_ident(0, "as") {
                        walker.bump();
                        if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident) {
                            let alias = walker.bump().expect("import alias");
                            local = walker.text(alias).to_owned();
                        }
                    }
                    let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(spec_start, |token| token.end);
                    walker.record("ImportSpecifier", local, spec_start, end.max(spec_start), None, None, None, &[]);
                }
                if walker.at_punct(0, "}") {
                    walker.bump();
                }
            } else if walker.at_punct(0, "*") {
                let star = walker.bump().expect("namespace star");
                if walker.at_ident(0, "as") {
                    walker.bump();
                }
                let mut name = "anonymous".to_owned();
                if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident) {
                    let token = walker.bump().expect("namespace name");
                    name = walker.text(token).to_owned();
                }
                let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(star.start, |token| token.end);
                walker.record("NamespaceImport", name, star.start, end.max(star.start), None, None, None, &[]);
            }
            let clause_end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(clause_start, |token| token.end);
            walker.record(
                "ImportClause",
                default_name.unwrap_or_else(|| "anonymous".to_owned()),
                clause_start,
                clause_end.max(clause_start),
                None,
                None,
                None,
                &[],
            );
            if walker.at_ident(0, "from") {
                walker.bump();
                if matches!(walker.peek(), Some(token) if token.kind == TokKind::Str) {
                    let spec = walker.bump().expect("from spec");
                    walker.imports.push(walker.text(spec).to_owned());
                }
            }
            if walker.at_punct(0, ";") {
                walker.bump();
            }
        });
    }

    fn parse_export_statement(&mut self, _start: usize, own_exports: &[String]) {
        self.guarded((), |walker| {
            walker.bump();
            if walker.at_ident(0, "type") && (walker.at_punct(1, "{") || walker.at_punct(1, "*")) {
                walker.bump();
            }
            if walker.at_punct(0, "*") {
                walker.bump();
                if walker.at_ident(0, "as") {
                    walker.bump();
                    if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident) {
                        walker.bump();
                    }
                }
                if walker.at_ident(0, "from") {
                    walker.bump();
                    if matches!(walker.peek(), Some(token) if token.kind == TokKind::Str) {
                        let spec = walker.bump().expect("export spec");
                        walker.imports.push(walker.text(spec).to_owned());
                    }
                }
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                return;
            }
            if walker.at_punct(0, "{") {
                walker.bump();
                while !walker.at_eof() && !walker.at_punct(0, "}") {
                    if walker.at_punct(0, ",") {
                        walker.bump();
                        continue;
                    }
                    if walker.at_ident(0, "type") {
                        walker.bump();
                    }
                    let spec_start = walker.peek().map(|token| token.start).unwrap_or(walker.source.len());
                    if !matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident || token.kind == TokKind::Str) {
                        walker.error_here("Unexpected export specifier");
                        walker.recover();
                        break;
                    }
                    let first = walker.bump().expect("export name");
                    let mut exported = walker.text(first).to_owned();
                    if walker.at_ident(0, "as") {
                        walker.bump();
                        if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident || token.kind == TokKind::Str) {
                            let alias = walker.bump().expect("export alias");
                            exported = walker.text(alias).to_owned();
                        }
                    }
                    let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(spec_start, |token| token.end);
                    walker.record("ExportSpecifier", exported, spec_start, end.max(spec_start), None, None, None, &[]);
                }
                if walker.at_punct(0, "}") {
                    walker.bump();
                }
                if walker.at_ident(0, "from") {
                    walker.bump();
                    if matches!(walker.peek(), Some(token) if token.kind == TokKind::Str) {
                        let spec = walker.bump().expect("export from");
                        walker.imports.push(walker.text(spec).to_owned());
                    }
                }
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                return;
            }
            if walker.at_punct(0, "=") {
                walker.bump();
                walker.scan_expression(None);
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                return;
            }
            if walker.at_ident(0, "as") {
                while !walker.at_eof() && !walker.at_punct(0, ";") {
                    walker.bump();
                }
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                return;
            }
            let mut extended = own_exports.to_vec();
            extended.push("export".to_owned());
            if walker.at_ident(0, "default") {
                extended.push("default".to_owned());
                walker.bump();
                if walker.at_ident(0, "function") {
                    let start = walker.peek().map(|token| token.start).unwrap_or(walker.source.len());
                    let _ = start;
                    walker.parse_function(true, None, &extended, Some(_start));
                    return;
                }
                if walker.at_ident(0, "class") {
                    walker.parse_class(true, None, &extended, Some(_start));
                    return;
                }
                if walker.at_ident(0, "abstract") {
                    walker.bump();
                    if walker.at_ident(0, "class") {
                        walker.parse_class(true, None, &extended, Some(_start));
                        return;
                    }
                }
                walker.scan_expression(None);
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                return;
            }
            if walker.at_ident(0, "declare") {
                walker.bump();
            }
            if walker.at_ident(0, "async") && walker.at_ident(1, "function") {
                walker.bump();
            }
            walker.parse_export_declaration(_start, &extended);
        });
    }

    fn parse_export_declaration(&mut self, start: usize, own_exports: &[String]) {
        if self.at_ident(0, "function") {
            self.parse_function(true, None, own_exports, Some(start));
        } else if self.at_ident(0, "class") {
            self.parse_class(true, None, own_exports, Some(start));
        } else if self.at_ident(0, "abstract") && self.at_ident(1, "class") {
            self.bump();
            self.parse_class(true, None, own_exports, Some(start));
        } else if self.at_ident(0, "interface") {
            self.parse_interface(start, own_exports);
        } else if self.at_ident(0, "enum") {
            self.parse_enum(start, own_exports);
        } else if self.at_ident(0, "type") && self.type_alias_ahead() {
            self.parse_type_alias(start, own_exports);
        } else if matches!(self.peek_text(0), Some("namespace") | Some("module")) {
            self.parse_module(start, own_exports);
        } else if matches!(self.peek_text(0), Some("const") | Some("let") | Some("var")) {
            self.bump();
            self.parse_variable_declarators(own_exports);
            if self.at_punct(0, ";") {
                self.bump();
            }
        } else if self.at_ident(0, "import") {
            while !self.at_eof() && !self.at_punct(0, ";") {
                self.bump();
            }
            if self.at_punct(0, ";") {
                self.bump();
            }
        } else {
            self.error_here("Unexpected export declaration");
            self.recover();
        }
    }

    fn type_alias_ahead(&self) -> bool {
        if !matches!(self.peek_at(1), Some(token) if token.kind == TokKind::Ident) {
            return false;
        }
        if self.at_punct(2, "=") {
            return true;
        }
        if self.at_punct(2, "<") {
            if let Some(close) = self.match_angle(self.pos + 2) {
                return self.tokens.get(close + 1).is_some_and(|token| self.text(*token) == "=");
            }
        }
        false
    }

    fn parse_statement(&mut self) {
        let before = self.pos;
        self.parse_statement_inner();
        if self.pos == before && !self.at_eof() {
            self.bump();
        }
    }

    fn parse_statement_inner(&mut self) {
        self.guarded((), |walker| {
            if walker.at_punct(0, ";") {
                walker.bump();
                return;
            }
            if walker.at_punct(0, "{") {
                walker.parse_block_body();
                return;
            }
            if walker.at_eof() {
                return;
            }
            while walker.at_punct(0, "@") {
                walker.skip_decorator();
            }
            let start = walker.peek().map(|token| token.start).unwrap_or(walker.source.len());
            if walker.at_ident(0, "export") {
                walker.parse_export_statement(start, &[]);
                return;
            }
            let modifiers = walker.collect_modifiers();
            let own: Vec<String> = modifiers.into_iter().filter(|modifier| modifier == "export" || modifier == "default").collect();
            if walker.at_ident(0, "import") && own.iter().any(|modifier| modifier == "export") {
                while !walker.at_eof() && !walker.at_punct(0, ";") {
                    walker.bump();
                }
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                return;
            }
            if walker.at_ident(0, "function") {
                walker.parse_function(true, None, &own, Some(start));
                return;
            }
            if walker.at_ident(0, "class") || (walker.at_ident(0, "abstract") && walker.at_ident(1, "class")) {
                if walker.at_ident(0, "abstract") {
                    walker.bump();
                }
                walker.parse_class(true, None, &own, Some(start));
                return;
            }
            if walker.at_ident(0, "interface") {
                walker.parse_interface(start, &own);
                return;
            }
            if walker.at_ident(0, "enum") {
                walker.parse_enum(start, &own);
                return;
            }
            if walker.at_ident(0, "type") && walker.type_alias_ahead() {
                walker.parse_type_alias(start, &own);
                return;
            }
            if matches!(walker.peek_text(0), Some("namespace") | Some("module")) {
                walker.parse_module(start, &own);
                return;
            }
            if matches!(walker.peek_text(0), Some("const") | Some("let") | Some("var")) {
                walker.bump();
                walker.parse_variable_declarators(&own);
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                return;
            }
            if walker.at_ident(0, "import") {
                walker.parse_import();
                return;
            }
            if walker.at_ident(0, "if") {
                walker.bump();
                if walker.at_punct(0, "(") {
                    walker.bump();
                    walker.scan_expression(None);
                    if walker.at_punct(0, ")") {
                        walker.bump();
                    }
                }
                walker.parse_statement();
                if walker.at_ident(0, "else") {
                    walker.bump();
                    walker.parse_statement();
                }
                return;
            }
            if walker.at_ident(0, "for") {
                walker.parse_for();
                return;
            }
            if walker.at_ident(0, "while") {
                walker.bump();
                if walker.at_punct(0, "(") {
                    walker.bump();
                    walker.scan_expression(None);
                    if walker.at_punct(0, ")") {
                        walker.bump();
                    }
                }
                walker.parse_statement();
                return;
            }
            if walker.at_ident(0, "do") {
                walker.bump();
                walker.parse_statement();
                if walker.at_ident(0, "while") {
                    walker.bump();
                    if walker.at_punct(0, "(") {
                        walker.bump();
                        walker.scan_expression(None);
                        if walker.at_punct(0, ")") {
                            walker.bump();
                        }
                    }
                    if walker.at_punct(0, ";") {
                        walker.bump();
                    }
                }
                return;
            }
            if walker.at_ident(0, "switch") {
                walker.bump();
                if walker.at_punct(0, "(") {
                    walker.bump();
                    walker.scan_expression(None);
                    if walker.at_punct(0, ")") {
                        walker.bump();
                    }
                }
                if walker.at_punct(0, "{") {
                    walker.bump();
                    while !walker.at_eof() && !walker.at_punct(0, "}") {
                        if walker.at_ident(0, "case") {
                            walker.bump();
                            walker.scan_expression(None);
                            if walker.at_punct(0, ":") {
                                walker.bump();
                            }
                        } else if walker.at_ident(0, "default") {
                            walker.bump();
                            if walker.at_punct(0, ":") {
                                walker.bump();
                            }
                        } else {
                            walker.parse_statement();
                        }
                    }
                    if walker.at_punct(0, "}") {
                        walker.bump();
                    }
                }
                return;
            }
            if walker.at_ident(0, "try") {
                walker.bump();
                walker.parse_block_body();
                if walker.at_ident(0, "catch") {
                    walker.bump();
                    if walker.at_punct(0, "(") {
                        walker.bump();
                        if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident) {
                            let token = walker.bump().expect("catch binding");
                            let name = walker.text(token).to_owned();
                            let mut end = token.end;
                            if walker.at_punct(0, ":") {
                                walker.bump();
                                walker.parse_type();
                                end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(end, |token| token.end);
                            }
                            walker.record("VariableDeclaration", name, token.start, end.max(token.start), None, None, None, &[]);
                        } else if walker.at_punct(0, "{") || walker.at_punct(0, "[") {
                            let pattern_start = walker.peek().map(|token| token.start).unwrap_or(walker.source.len());
                            let pattern_end = walker.match_paren(walker.pos).map_or(pattern_start, |close| walker.tokens[close].end);
                            let name = walker.source[pattern_start..pattern_end.min(walker.source.len())].to_owned();
                            walker.parse_binding_pattern();
                            if walker.at_punct(0, ":") {
                                walker.bump();
                                walker.parse_type();
                            }
                            let end = walker.tokens.get(walker.pos.saturating_sub(1)).map_or(pattern_start, |token| token.end);
                            walker.record("VariableDeclaration", name, pattern_start, end.max(pattern_start), None, None, None, &[]);
                        }
                        if walker.at_punct(0, ")") {
                            walker.bump();
                        }
                    }
                    walker.parse_block_body();
                }
                if walker.at_ident(0, "finally") {
                    walker.bump();
                    walker.parse_block_body();
                }
                return;
            }
            if walker.at_ident(0, "with") {
                walker.bump();
                if walker.at_punct(0, "(") {
                    walker.bump();
                    walker.scan_expression(None);
                    if walker.at_punct(0, ")") {
                        walker.bump();
                    }
                }
                walker.parse_statement();
                return;
            }
            if walker.at_ident(0, "return") || walker.at_ident(0, "throw") {
                walker.bump();
                if !walker.at_punct(0, ";") && !walker.at_punct(0, "}") && !walker.at_eof() {
                    walker.scan_expression(None);
                }
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                return;
            }
            if matches!(walker.peek_text(0), Some("break") | Some("continue") | Some("debugger")) {
                walker.bump();
                if matches!(walker.peek(), Some(token) if token.kind == TokKind::Ident) {
                    walker.bump();
                }
                if walker.at_punct(0, ";") {
                    walker.bump();
                }
                return;
            }
            if walker.peek().is_some_and(|token| token.kind == TokKind::Ident) && walker.at_punct(1, ":") {
                walker.bump();
                walker.bump();
                walker.parse_statement();
                return;
            }
            walker.scan_expression(None);
            if walker.at_punct(0, ";") {
                walker.bump();
            }
        });
    }

    fn parse_for(&mut self) {
        self.bump();
        if self.at_ident(0, "await") {
            self.bump();
        }
        if !self.at_punct(0, "(") {
            self.error_here("Expected a for header");
            return;
        }
        self.bump();
        if matches!(self.peek_text(0), Some("const") | Some("let") | Some("var")) {
            self.bump();
            self.parse_variable_declarators(&[]);
        } else if !self.at_punct(0, ";") {
            self.scan_expression(None);
        }
        if matches!(self.peek_text(0), Some("in") | Some("of")) {
            self.bump();
            self.scan_expression(None);
            if self.at_punct(0, ")") {
                self.bump();
            }
            self.parse_statement();
            return;
        }
        if self.at_punct(0, ";") {
            self.bump();
        }
        if !self.at_punct(0, ";") && !self.at_punct(0, ")") {
            self.scan_expression(None);
        }
        if self.at_punct(0, ";") {
            self.bump();
        }
        if !self.at_punct(0, ")") {
            self.scan_expression(None);
        }
        if self.at_punct(0, ")") {
            self.bump();
        }
        self.parse_statement();
    }

    fn lhs_text(&self, equals: usize) -> String {
        let mut start = equals;
        let mut index = equals;
        while index > 0 {
            index -= 1;
            let token = self.tokens[index];
            let text = self.text(token);
            if token.kind == TokKind::Ident {
                if matches!(
                    text,
                    "new" | "typeof" | "return" | "case" | "do" | "else" | "in" | "of" | "delete" | "void" | "instanceof" | "yield" | "await" | "throw" | "if" | "for" | "while" | "switch" | "catch" | "with" | "const" | "let" | "var"
                ) {
                    break;
                }
                start = index;
            } else if token.kind == TokKind::Punct && (text == "." || text == "?." || text == "!") {
                if text == "!" && index + 1 != equals && !self.chain_continues(index) {
                    break;
                }
                start = index;
            } else if token.kind == TokKind::Punct && (text == ")" || text == "]") {
                if index + 1 != equals && !self.chain_continues(index) {
                    break;
                }
                let mut depth = 1;
                let mut back = index;
                while back > 0 && depth > 0 {
                    back -= 1;
                    let inner = self.text(self.tokens[back]);
                    if self.tokens[back].kind == TokKind::Punct {
                        if inner == ")" || inner == "]" {
                            depth += 1;
                        } else if inner == "(" || inner == "[" {
                            depth -= 1;
                        }
                    }
                }
                index = back;
                start = back;
            } else {
                break;
            }
        }
        self.source[self.tokens[start].start..self.tokens[equals.saturating_sub(1).max(start)].end.min(self.source.len())].trim().to_owned()
    }

    fn arrow_ahead(&self) -> Option<usize> {
        if self.at_punct(0, "=>") {
            return Some(self.pos);
        }
        if self.peek().is_some_and(|token| token.kind == TokKind::Ident) && self.at_punct(1, "=>") {
            return Some(self.pos + 1);
        }
        if self.at_punct(0, "(") {
            if let Some(close) = self.match_paren(self.pos) {
                if self.tokens.get(close + 1).is_some_and(|token| self.text(*token) == "=>") {
                    return Some(close + 1);
                }
                if let Some(found) = self.colon_scan_arrow(close) {
                    return Some(found);
                }
            }
            return None;
        }
        if self.at_punct(0, "<") {
            if let Some(close) = self.match_angle(self.pos) {
                if self.tokens.get(close + 1).is_some_and(|token| self.text(*token) == "(") {
                    if let Some(params) = self.match_paren(close + 1) {
                        if self.tokens.get(params + 1).is_some_and(|token| self.text(*token) == "=>") {
                            return Some(params + 1);
                        }
                        if let Some(found) = self.colon_scan_arrow(params) {
                            return Some(found);
                        }
                    }
                }
            }
        }
        None
    }

    fn colon_scan_arrow(&self, close: usize) -> Option<usize> {
        if !self.tokens.get(close + 1).is_some_and(|token| self.text(*token) == ":") {
            return None;
        }
        let mut index = close + 2;
        let mut depth = 0;
        let mut angles = 0;
        while let Some(token) = self.tokens.get(index) {
            let text = self.text(*token);
            if token.kind == TokKind::Punct {
                if text == "(" {
                    if let Some(inner) = self.match_paren(index) {
                        if self.tokens.get(inner + 1).is_some_and(|token| self.text(*token) == "=>")
                            && self.simple_params(index, inner)
                        {
                            index = inner + 2;
                            continue;
                        }
                        depth += 1;
                    } else {
                        break;
                    }
                } else if text == "[" || text == "{" {
                    depth += 1;
                } else if text == ")" || text == "]" || text == "}" {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                } else if text == "<" {
                    angles += 1;
                } else if text == ">" {
                    angles -= 1;
                } else if text == ">>" {
                    angles -= 2;
                } else if text == ">>>" {
                    angles -= 3;
                } else if depth == 0 && angles == 0 && (text == ";" || text == "," || text == "=") {
                    break;
                } else if depth == 0 && angles == 0 && text == "=>" {
                    return Some(index);
                }
            }
            index += 1;
        }
        None
    }

    fn simple_params(&self, open: usize, close: usize) -> bool {
        let mut depth = 0;
        let mut index = open + 1;
        while index < close {
            let token = self.tokens[index];
            if token.kind == TokKind::Punct {
                let text = self.text(token);
                if text == "(" || text == "[" || text == "{" {
                    depth += 1;
                } else if text == ")" || text == "]" || text == "}" {
                    depth -= 1;
                } else if depth == 0 && text == "=>" {
                    return false;
                }
            }
            index += 1;
        }
        true
    }

    fn scan_expression(&mut self, context: Option<String>) {
        self.guarded((), |walker| {
            let mut context = context;
            let direct = walker.scan_expression_direct(&mut context);
            walker.scan_expression_rest(&mut context, !direct);
        });
    }

    fn scan_expression_direct(&mut self, context: &mut Option<String>) -> bool {
        if self.at_ident(0, "async") && self.at_ident(1, "function") {
            let start = self.peek().map(|token| token.start).unwrap_or(self.source.len());
            let name = context.take().or_else(|| self.equals_name());
            self.bump();
            self.parse_function(false, name, &[], Some(start));
            true
        } else if self.at_ident(0, "function") {
            let name = context.take().or_else(|| self.equals_name());
            self.parse_function(false, name, &[], None);
            true
        } else if self.at_ident(0, "class") {
            let name = context.take().or_else(|| self.equals_name());
            self.parse_class(false, name, &[], None);
            true
        } else if self.at_ident(0, "async") && self.arrow_ahead_at(1).is_some() {
            let arrow = self.arrow_ahead_at(1).expect("async arrow");
            let name = context.take().or_else(|| self.equals_name());
            let start = self.arrow_start_from(self.pos);
            self.bump();
            self.parse_arrow_params(arrow);
            self.parse_arrow_at(arrow, name, start);
            true
        } else if self.arrow_ahead().is_some() {
            let arrow = self.arrow_ahead().expect("leading arrow");
            let name = context.take().or_else(|| self.equals_name());
            let start = self.arrow_start_from(self.pos);
            if self.at_punct(0, "(") || self.peek().is_some_and(|token| token.kind == TokKind::Ident) || self.at_punct(0, "<") {
                self.parse_arrow_params(arrow);
            }
            self.parse_arrow_at(arrow, name, start);
            true
        } else {
            false
        }
    }

    fn arrow_ahead_at(&self, offset: usize) -> Option<usize> {
        let saved = self.pos;
        let _ = saved;
        if self.peek_at(offset).is_none() {
            return None;
        }
        if offset == 1 && self.at_ident(0, "async") {
            if self.peek_at(1).is_some_and(|token| token.kind == TokKind::Ident) && self.at_punct(2, "=>") {
                return Some(self.pos + 2);
            }
            if self.at_punct(1, "(") {
                if let Some(close) = self.match_paren(self.pos + 1) {
                    if self.tokens.get(close + 1).is_some_and(|token| self.text(*token) == "=>") {
                        return Some(close + 1);
                    }
                    if let Some(found) = self.colon_scan_arrow(close) {
                        return Some(found);
                    }
                }
            }
            if self.at_punct(1, "<") {
                if let Some(close) = self.match_angle(self.pos + 1) {
                    if self.tokens.get(close + 1).is_some_and(|token| self.text(*token) == "(") {
                        if let Some(params) = self.match_paren(close + 1) {
                            if self.tokens.get(params + 1).is_some_and(|token| self.text(*token) == "=>") {
                                return Some(params + 1);
                            }
                            if let Some(found) = self.colon_scan_arrow(params) {
                                return Some(found);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn parse_arrow_params(&mut self, arrow: usize) {
        if self.at_punct(0, "<") {
            if let Some(close) = self.match_angle(self.pos) {
                if self.tokens.get(close + 1).is_some_and(|token| self.text(*token) == "(") {
                    let saved_end = close;
                    let _ = saved_end;
                    self.skip_type_params_decl();
                }
            }
        }
        if self.at_punct(0, "(") {
            if let Some(close) = self.match_paren(self.pos) {
                if close < arrow {
                    self.parse_parameters("arrow");
                    if self.at_punct(0, ":") {
                        self.bump();
                        self.parse_type();
                    }
                }
            }
        } else if self.peek().is_some_and(|token| token.kind == TokKind::Ident) && self.at_punct(1, "=>") {
            let token = self.bump().expect("arrow param");
            let name = self.text(token).to_owned();
            self.record("Parameter", name, token.start, token.end, None, None, None, &[]);
        }
        let _ = arrow;
    }

    fn scan_expression_rest(&mut self, context: &mut Option<String>, mut first: bool) {
        loop {
            let Some(token) = self.peek() else { break };
            if token.kind == TokKind::Punct {
                match self.text(token) {
                    "," | ";" | ")" | "]" | "}" | ":" => break,
                    "?" => {
                        self.bump();
                        self.scan_expression(None);
                        if self.at_punct(0, ":") {
                            self.bump();
                            self.scan_expression(None);
                        }
                    }
                    "(" => {
                        if let Some(arrow) = self.arrow_ahead() {
                            let name = if first { context.take() } else { None }.or_else(|| self.arrow_equals_name_at(self.pos));
                            let start = self.arrow_start_from(self.pos);
                            self.parse_arrow_params(arrow);
                            self.parse_arrow_at(arrow, name, start);
                        } else {
                            let open = self.pos;
                            let callee = self.callee_text(open);
                            self.bump();
                            while !self.at_eof() && !self.at_punct(0, ")") {
                                if self.at_punct(0, ",") {
                                    self.bump();
                                    continue;
                                }
                                if self.at_punct(0, "...") {
                                    self.bump();
                                }
                                let name = if callee.is_empty() { None } else { Some(format!("{callee} callback")) };
                                let before = self.pos;
                                self.scan_expression(name);
                                if self.pos == before {
                                    self.error_here("Unexpected token in arguments");
                                    self.bump();
                                }
                            }
                            if self.at_punct(0, ")") {
                                self.bump();
                            } else {
                                self.error_here("Unbalanced parenthesis in expression");
                            }
                        }
                    }
                    "[" => {
                        self.bump();
                        while !self.at_eof() && !self.at_punct(0, "]") {
                            if self.at_punct(0, ",") {
                                self.bump();
                                continue;
                            }
                            if self.at_punct(0, "...") {
                                self.bump();
                            }
                            let before = self.pos;
                            self.scan_expression(None);
                            if self.pos == before {
                                self.error_here("Unexpected token in array literal");
                                self.bump();
                            }
                        }
                        if self.at_punct(0, "]") {
                            self.bump();
                        } else {
                            self.error_here("Unbalanced brackets in expression");
                        }
                    }
                    "{" => {
                        self.parse_object();
                    }
                    "<" => {
                        let mut arrow_params = false;
                        let mut call_args = false;
                        if let Some(close) = self.match_angle(self.pos) {
                            if self.tokens.get(close + 1).is_some_and(|token| self.text(*token) == "(") {
                                if let Some(params) = self.match_paren(close + 1) {
                                    if self.tokens.get(params + 1).is_some_and(|token| self.text(*token) == "=>")
                                        || self.colon_scan_arrow(params).is_some()
                                    {
                                        arrow_params = true;
                                    } else {
                                        call_args = true;
                                    }
                                } else {
                                    call_args = true;
                                }
                            }
                        }
                        if arrow_params {
                            self.skip_type_params_decl();
                        } else if call_args {
                            self.skip_type_args();
                        } else {
                            self.bump();
                        }
                    }
                    "=>" => {
                        let name = if first { context.take() } else { None }
                            .or_else(|| self.pos.checked_sub(1).and_then(|param| self.arrow_equals_name_at(param)));
                        let start = self.arrow_start_from(self.pos.saturating_sub(1));
                        let arrow = self.pos;
                        self.parse_arrow_at(arrow, name, start);
                    }
                    _ => {
                        self.bump();
                    }
                }
            } else if token.kind == TokKind::Ident {
                let text = self.text(token).to_owned();
                let member_access = self.pos > 0
                    && self.tokens[self.pos - 1].kind == TokKind::Punct
                    && matches!(self.text(self.tokens[self.pos - 1]), "." | "?.");
                if member_access && matches!(text.as_str(), "function" | "class" | "as" | "satisfies") {
                    self.bump();
                } else if text == "function" {
                    let name = if first { context.take() } else { None }.or_else(|| self.equals_name());
                    self.parse_function(false, name, &[], None);
                } else if text == "class" {
                    let name = if first { context.take() } else { None }.or_else(|| self.equals_name());
                    self.parse_class(false, name, &[], None);
                } else if text == "async" && self.at_ident(1, "function") {
                    let start = token.start;
                    let name = if first { context.take() } else { None }.or_else(|| self.equals_name());
                    self.bump();
                    self.parse_function(false, name, &[], Some(start));
                } else if text == "new" {
                    self.bump();
                    while let Some(next) = self.peek() {
                        if next.kind == TokKind::Ident || (next.kind == TokKind::Punct && matches!(self.text(next), "." | "?.")) {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    if self.at_punct(0, "<") {
                        self.skip_type_args();
                    }
                    if self.at_punct(0, "(") {
                        self.bump();
                        while !self.at_eof() && !self.at_punct(0, ")") {
                            if self.at_punct(0, ",") {
                                self.bump();
                                continue;
                            }
                            if self.at_punct(0, "...") {
                                self.bump();
                            }
                            let before = self.pos;
                            self.scan_expression(None);
                            if self.pos == before {
                                self.error_here("Unexpected token in arguments");
                                self.bump();
                            }
                        }
                        if self.at_punct(0, ")") {
                            self.bump();
                        }
                    }
                } else if text == "as" || text == "satisfies" {
                    self.bump();
                    if self.at_ident(0, "const") {
                        self.bump();
                    } else {
                        self.parse_type();
                    }
                } else if self.at_punct(1, "=>") {
                    let name = if first { context.take() } else { None }.or_else(|| self.arrow_equals_name_at(self.pos));
                    let param_index = self.pos;
                    let param = self.bump().expect("arrow param");
                    let param_name = self.text(param).to_owned();
                    self.record("Parameter", param_name, param.start, param.end, None, None, None, &[]);
                    let arrow = self.pos;
                    let start = self.arrow_start_from(param_index);
                    self.parse_arrow_at(arrow, name, start);
                } else {
                    self.bump();
                }
            } else if token.kind == TokKind::TmplHead {
                self.bump();
                self.scan_expression(None);
                while self.peek().is_some_and(|next| next.kind == TokKind::TmplMid) {
                    self.bump();
                    self.scan_expression(None);
                }
                if self.peek().is_some_and(|next| next.kind == TokKind::TmplTail) {
                    self.bump();
                }
            } else {
                self.bump();
            }
            first = false;
        }
    }

    fn equals_name(&self) -> Option<String> {
        if self.pos == 0 {
            return None;
        }
        self.equals_name_before(self.pos)
    }

    fn equals_name_before(&self, index: usize) -> Option<String> {
        if index == 0 {
            return None;
        }
        let prev = self.tokens[index - 1];
        if prev.kind == TokKind::Punct && self.text(prev) == "=" {
            let name = self.lhs_text(index - 1);
            if name.is_empty() {
                None
            } else {
                Some(name)
            }
        } else {
            None
        }
    }

    fn parse_object(&mut self) {
        self.guarded((), |walker| {
            walker.bump();
            while !walker.at_eof() && !walker.at_punct(0, "}") {
                if walker.at_punct(0, ",") || walker.at_punct(0, ";") {
                    walker.bump();
                    continue;
                }
                if walker.at_punct(0, "...") {
                    walker.bump();
                    walker.scan_expression(None);
                    continue;
                }
                let start = walker.peek().map(|token| token.start).unwrap_or(walker.source.len());
                if walker.at_ident(0, "async") && !matches!(walker.peek_text(1), Some("(") | Some(":") | Some(",") | Some("}")) {
                    walker.bump();
                }
                if walker.at_punct(0, "*") {
                    walker.bump();
                }
                if matches!(walker.peek_text(0), Some("get") | Some("set")) {
                    let accessor = walker.peek_text(0).unwrap_or("get").to_owned();
                    let is_get = accessor == "get";
                    let next = walker.peek_at(1);
                    let followed = walker.peek_at(2);
                    let nameish = next.is_some_and(|token| {
                        matches!(token.kind, TokKind::Ident | TokKind::Str | TokKind::Number)
                            || (token.kind == TokKind::Punct && walker.text(token) == "[")
                    });
                    if nameish && followed.is_some_and(|token| token.kind == TokKind::Punct && walker.text(token) == "(") {
                        walker.bump();
                        let (name, _) = walker.member_name().expect("object accessor");
                        walker.parse_method_common(start, name, if is_get { "GetAccessor" } else { "SetAccessor" }, false);
                        continue;
                    }
                }
                let Some((name, _)) = walker.member_name() else {
                    walker.error_here("Unexpected object member");
                    walker.recover();
                    break;
                };
                if walker.at_punct(0, "<") {
                    let close = walker.match_angle(walker.pos);
                    if close.is_some_and(|found| walker.tokens.get(found + 1).is_some_and(|token| walker.text(*token) == "(")) {
                        walker.parse_method_common(start, name, "MethodDeclaration", false);
                        continue;
                    }
                }
                if walker.at_punct(0, "(") {
                    walker.parse_method_common(start, name, "MethodDeclaration", false);
                    continue;
                }
                if walker.at_punct(0, ":") {
                    walker.bump();
                    walker.scan_expression(Some(name));
                    continue;
                }
                if walker.at_punct(0, "=") {
                    walker.bump();
                    walker.scan_expression(Some(name));
                    continue;
                }
            }
            if walker.at_punct(0, "}") {
                walker.bump();
            } else {
                walker.error_here("Unbalanced object literal");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl_kinds(source: &str) -> Vec<(String, String)> {
        scan_declarations(source)
            .declarations
            .iter()
            .map(|decl| (decl.kind.to_owned(), decl.name.clone()))
            .collect()
    }

    #[test]
    fn finds_function_forms() {
        let decls = scan_declarations(
            "export function top(a: number): string { return String(a); }\n\
             const expr = function named() { return 1; };\n\
             const anon = function () { return 2; };\n\
             const arrow = (x: number): number => x + 1;\n\
             const single = y => y;\n\
             const asyncArrow = async (z: number) => z;\n",
        );
        assert!(decls.diagnostics.is_empty(), "{:?}", decls.diagnostics);
        let functions: Vec<(&str, &str)> = decls
            .declarations
            .iter()
            .filter(|decl| matches!(decl.kind, "FunctionDeclaration" | "FunctionExpression" | "ArrowFunction"))
            .map(|decl| (decl.kind, decl.name.as_str()))
            .collect();
        assert_eq!(
            functions,
            vec![
                ("FunctionDeclaration", "top"),
                ("FunctionExpression", "named"),
                ("FunctionExpression", "anon"),
                ("ArrowFunction", "arrow"),
                ("ArrowFunction", "single"),
                ("ArrowFunction", "asyncArrow"),
            ]
        );
        for decl in decls.declarations.iter().filter(|decl| decl.kind == "ArrowFunction" || decl.kind.starts_with("Function")) {
            assert!(decl.body.is_some(), "{}", decl.name);
        }
        let params: Vec<&str> = decls
            .declarations
            .iter()
            .filter(|decl| decl.kind == "Parameter")
            .map(|decl| decl.name.as_str())
            .collect();
        assert_eq!(params, vec!["a", "x", "y", "z"]);
    }

    #[test]
    fn infers_callback_and_anonymous_names() {
        let decls = scan_declarations(
            "schedule(function () { work(); });\n\
             const value = compute(() => 1, other);\n\
             const mixed = flag ? function () { return 1; } : function () { return 2; };\n",
        );
        assert!(decls.diagnostics.is_empty(), "{:?}", decls.diagnostics);
        let names: Vec<&str> = decls
            .declarations
            .iter()
            .filter(|decl| decl.kind == "FunctionExpression" || decl.kind == "ArrowFunction")
            .map(|decl| decl.name.as_str())
            .collect();
        assert_eq!(names, vec!["schedule callback", "compute callback", "anonymous", "anonymous"]);
    }

    #[test]
    fn finds_class_members() {
        let decls = scan_declarations(
            "export class Widget extends Base implements Shape {\n\
             \x20 private count = 0;\n\
             \x20 constructor(public label: string) { this.count = 1; }\n\
             \x20 get size(): number { return this.count; }\n\
             \x20 set size(value: number) { this.count = value; }\n\
             \x20 async run<T>(input: T): Promise<T> { return input; }\n\
             \x20 static create(): Widget { return new Widget(); }\n\
             }\n",
        );
        assert!(decls.diagnostics.is_empty(), "{:?}", decls.diagnostics);
        let members: Vec<(&str, &str)> = decls
            .declarations
            .iter()
            .filter(|decl| decl.kind != "ClassDeclaration" && decl.kind != "Parameter" && decl.kind != "TypeParameterDeclaration")
            .map(|decl| (decl.kind, decl.name.as_str()))
            .collect();
        assert_eq!(
            members,
            vec![
                ("PropertyDeclaration", "count"),
                ("Constructor", "constructor"),
                ("GetAccessor", "size"),
                ("SetAccessor", "size"),
                ("MethodDeclaration", "run"),
                ("MethodDeclaration", "create"),
            ]
        );
        assert!(decls.declarations.iter().any(|decl| decl.kind == "TypeParameterDeclaration" && decl.name == "T"));
        for member in decls.declarations.iter().filter(|decl| decl.kind == "MethodDeclaration") {
            assert_eq!(member.exports, vec!["export".to_owned()]);
            assert_eq!(member.enclosing, vec!["Widget".to_owned()]);
        }
    }

    #[test]
    fn finds_object_methods_and_properties() {
        let decls = scan_declarations(
            "const table = {\n\
             \x20 plain: 1,\n\
             \x20 method(a: number) { return a; },\n\
             \x20 async deferred() { return 2; },\n\
             \x20 get value() { return 3; },\n\
             \x20 nested: { deep() { return 4; } },\n\
             \x20 handler: function () { return 5; },\n\
             \x20 arrow: () => 6,\n\
             };\n",
        );
        assert!(decls.diagnostics.is_empty(), "{:?}", decls.diagnostics);
        let kinds = decl_kinds("const table = { method(a: number) { return a; }, get value() { return 3; }, handler: function () { return 5; }, arrow: () => 6 };");
        let functions: Vec<(&str, &str)> = kinds.iter().map(|(kind, name)| (kind.as_str(), name.as_str())).filter(|(kind, _)| *kind != "VariableDeclaration" && *kind != "Parameter").collect();
        assert_eq!(
            functions,
            vec![
                ("MethodDeclaration", "method"),
                ("GetAccessor", "value"),
                ("FunctionExpression", "handler"),
                ("ArrowFunction", "arrow"),
            ]
        );
        let nested: Vec<(&str, &str)> = decls
            .declarations
            .iter()
            .filter(|decl| decl.kind == "MethodDeclaration")
            .map(|decl| (decl.kind, decl.name.as_str()))
            .collect();
        assert!(nested.contains(&("MethodDeclaration", "deep")));
    }

    #[test]
    fn finds_type_level_declarations() {
        let decls = scan_declarations(
            "export interface Shape { readonly kind: string; area(scale: number): number; (): void; new (): Shape; }\n\
             export type Handler = (event: string) => void;\n\
             export type Factory = new (name: string) => Handler;\n\
             export enum Mode { Fast = 1, Slow }\n\
             const pair: [Handler, Handler] = [() => {}, () => {}];\n",
        );
        assert!(decls.diagnostics.is_empty(), "{:?}", decls.diagnostics);
        let kinds = decl_kinds(
            "export interface Shape { readonly kind: string; area(scale: number): number; (): void; new (): Shape; }\n\
             export type Handler = (event: string) => void;\n",
        );
        let names: Vec<(&str, &str)> = kinds.iter().map(|(kind, name)| (kind.as_str(), name.as_str())).collect();
        assert!(names.contains(&("InterfaceDeclaration", "Shape")));
        assert!(names.contains(&("PropertySignature", "kind")));
        assert!(names.contains(&("MethodSignature", "area")));
        assert!(names.contains(&("CallSignature", "call")));
        assert!(names.contains(&("ConstructSignature", "constructor")));
        assert!(names.contains(&("TypeAliasDeclaration", "Handler")));
        assert!(names.contains(&("FunctionType", "anonymous")));
        let enum_kinds = decl_kinds("export enum Mode { Fast = 1, Slow }");
        assert!(enum_kinds.iter().any(|(kind, name)| kind == "EnumMember" && name == "Fast"));
        let member = decls.declarations.iter().find(|decl| decl.kind == "EnumMember" && decl.name == "Slow").expect("slow member");
        assert!(member.initializer_start.is_none());
    }

    #[test]
    fn finds_imports_exports_and_modules() {
        let decls = scan_declarations(
            "import runner, { alpha, beta as gamma } from \"./runner.ts\";\n\
             import * as helpers from \"./helpers.ts\";\n\
             import \"./side-effect.ts\";\n\
             export { alpha as exposed };\n\
             export default function main(): void {}\n\
             declare module \"ambient\" { export const marker: number; }\n",
        );
        assert!(decls.diagnostics.is_empty(), "{:?}", decls.diagnostics);
        assert_eq!(decls.imports, vec!["\"./runner.ts\"", "\"./helpers.ts\"", "\"./side-effect.ts\""]);
        let main = decls.declarations.iter().find(|decl| decl.name == "main").expect("main");
        assert_eq!(main.kind, "FunctionDeclaration");
        assert_eq!(main.exports, vec!["export".to_owned(), "default".to_owned()]);
        let spec = decls.declarations.iter().find(|decl| decl.kind == "ImportSpecifier" && decl.name == "gamma").expect("import spec");
        let _ = spec;
        let marker = decls.declarations.iter().find(|decl| decl.name == "marker").expect("marker");
        assert_eq!(marker.exports, vec!["export".to_owned()]);
        assert_eq!(marker.enclosing, vec!["\"ambient\"".to_owned()]);
    }

    #[test]
    fn reports_malformed_input() {
        let decls = scan_declarations("const broken = \"unterminated;\nfunction half(\n");
        assert!(!decls.diagnostics.is_empty());
        let decls = scan_declarations("function fine() { return 1; }\n");
        assert!(decls.diagnostics.is_empty());
    }

    #[test]
    fn signatures_and_bodies_slice_source() {
        let source = "export function compute(input: number): number {\n  return input * 2;\n}\n";
        let decls = scan_declarations(source);
        let compute = decls.declarations.iter().find(|decl| decl.name == "compute").expect("compute");
        let (body_start, body_end) = compute.body.expect("body");
        assert_eq!(&source[body_start..body_end], "{\n  return input * 2;\n}");
        assert_eq!(source[compute.start..body_start].trim(), "export function compute(input: number): number");
        assert_eq!(line_of(&line_starts(source), compute.start), 1);
        assert_eq!(line_col(&line_starts(source), compute.end - 1), (3, 1));
    }

    #[test]
    fn token_hash_is_deterministic() {
        assert_eq!(token_hash("(a: number) => a"), token_hash("(a:number)=>a"));
        assert_ne!(token_hash("a"), token_hash("b"));
    }

    #[test]
    fn skips_regex_and_templates() {
        let decls = scan_declarations(
            "const pattern = /function\\s+(\\w+)/g;\n\
             const text = `template ${pattern} tail`;\n\
             const division = count / total / 2;\n",
        );
        assert!(decls.diagnostics.is_empty(), "{:?}", decls.diagnostics);
        assert!(decls.declarations.iter().all(|decl| decl.kind == "VariableDeclaration"));
    }

    #[test]
    fn parses_dense_corpus_patterns() {
        let decls = scan_declarations(
            "interface I { withProtection?<T>(x: T): void; team: `${string}:${string}`; }\n\
             class C { withP?<T>(x: T): T { return x; } }\n\
             const leaf = at<Exclude<D, { readonly kind: \"q1-bsp\" }>[\"l\"][number]>(m, 1);\n\
             class D { async m(epoch: number): Promise<void> {\n\
             const publish=async():Promise<void>=>{\n\
             if(epoch!==this.w||this.c)return;\n\
             };\n\
             } }\n\
             const f = async <T>(x: T): Promise<T> => x;\n\
             if (a>=b) { y(); }\n",
        );
        assert!(decls.diagnostics.is_empty(), "{:?}", decls.diagnostics);
        let kinds = decl_kinds(
            "interface I { withProtection?<T>(x: T): void; }\n\
             class C { withP?<T>(x: T): T { return x; } }\n",
        );
        assert!(kinds.contains(&("MethodSignature".to_owned(), "withProtection".to_owned())), "{kinds:?}");
        assert!(kinds.contains(&("MethodDeclaration".to_owned(), "withP".to_owned())), "{kinds:?}");
    }

    #[test]
    fn names_arrows_like_inferred_name() {
        let decls = scan_declarations(
            "svMainHooks.spawnServer = (mapname: string): void => {};\n\
             x.y = async (a) => {};\n\
             x = async y => {};\n\
             x = <T>(a: T): T => a;\n\
             promise.catch(() => {});\n\
             describe.if(flag)(\"name\", () => {});\n\
             if (!real) Cmd_AddCommand(\"x\", () => {});\n\
             void (async () => {\n  for await (const c of s) {\n    g();\n  }\n})();\n",
        );
        assert!(decls.diagnostics.is_empty(), "{:?}", decls.diagnostics);
        let arrows: Vec<(String, usize)> = decls
            .declarations
            .iter()
            .filter(|decl| decl.kind == "ArrowFunction")
            .map(|decl| (decl.name.clone(), decl.start))
            .collect();
        assert_eq!(
            arrows,
            [
                ("svMainHooks.spawnServer".to_owned(), 26),
                ("x.y".to_owned(), 63),
                ("x".to_owned(), 84),
                ("x".to_owned(), 103),
                ("promise.catch callback".to_owned(), 136),
                ("describe.if(flag) callback".to_owned(), 173),
                ("Cmd_AddCommand callback".to_owned(), 215),
                ("anonymous".to_owned(), 232),
            ]
        );
    }
}

/// Scan declarations, imports, and diagnostics from TypeScript source.
#[must_use]
pub fn scan_declarations(source: &str) -> ScanDeclarations {
    let scan = tokenize(source);
    let mut walker = Walker::new(source, scan.tokens, scan.diagnostics);
    while !walker.at_eof() {
        walker.parse_statement();
    }
    walker.decls.sort_by(|left, right| left.start.cmp(&right.start).then_with(|| left.end.cmp(&right.end)));
    ScanDeclarations {
        declarations: walker.decls,
        imports: walker.imports,
        diagnostics: walker.diagnostics,
    }
}
