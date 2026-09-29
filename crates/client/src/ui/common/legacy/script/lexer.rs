//! Legacy menu script lexer.
//!
//! Ported from the TypeScript donor's `src/ui/common/legacy/script/lexer.ts`
//! (itself translated from Quake III Arena `botlib/l_script.c`,
//! Copyright (C) 1999-2005 Id Software, Inc. GPL-2.0-or-later). Tokenizes
//! script text into strings, literals, numbers, names, punctuation and
//! primitives, tracking line/column locations and reporting diagnostics.
//!
//! # Sync port notes
//!
//! * The donor is sync already; this port is sync as well.
//! * The donor accepts either a plain string or a `SourceScriptStorage`
//!   arena image. That sibling (`super::source_memory` / `super::memory`)
//!   is still a stub, so this lexer owns plain text (`&str` in,
//!   `Vec<char>` held) and exposes no `memory` option, `source_storage`
//!   getter, or `allocate_script_source`. The [`SOURCE_SCRIPT_BASE`]
//!   constant (donor `SOURCE_SCRIPT_BYTES` from `memory.ts`) is mirrored
//!   locally until the memory sibling lands.
//! * The donor writes every token through a caller-provided or internal
//!   `SourceTokenMemory` image. This port keeps that behavior at the API
//!   boundary ([`ScriptLexer::next_into`], `expect_*`, `check_token_type`,
//!   [`ScriptLexer::read_literal_into`]); the canonical [`ScriptToken`]
//!   records below are built directly from the same scan values the image
//!   words carry, which is what the donor decode reproduces. Internal reads
//!   that the donor aims at its own `output` member use a scratch image
//!   instead; the donor clears that member on every entry, so the decoded
//!   records are identical.
//! * Donor `RangeError` throws become [`ClientError::BadUi`] carrying the
//!   donor message. Donor `fail` throws become [`ScriptLanguageError`],
//!   which is also the public error for every fallible lexer method:
//!   internal memory-image failures surface as non-reported language
//!   errors so [`ScriptLexer::is_source_failure`] keeps the donor
//!   catch-and-continue behavior of `readExpectedInto`.
//! * `SaveReader` checkpoints become the typed [`ScriptLexerSaveState`];
//!   there is no `SaveReader` in this crate, so donor
//!   `readScriptDiagnostic` has no counterpart here.
//! * Assumed donor-derived imports once siblings land:
//!   `super::memory::{SourceScriptStorage, SOURCE_SCRIPT_BYTES}`,
//!   `super::memory::{ScriptMemory, ScriptMemoryCapture, ScriptMemoryRestore}`.

use std::error::Error;
use std::fmt;

use crate::error::ClientError;

use super::token_memory::{SourceTokenContext, SourceTokenMemory, SourceTokenSaveState};

/// Default maximum raw token length (donor `DEFAULT_SCRIPT_TOKEN_LIMIT`).
pub const DEFAULT_SCRIPT_TOKEN_LIMIT: usize = 1024;

/// Base of retained source pointers (donor `SOURCE_SCRIPT_BYTES` = 2148).
///
/// Mirrored locally until `super::memory` lands; see the module docs.
const SOURCE_SCRIPT_BASE: u32 = 2148;

/// Token type word values (donor `ScriptTokenType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptTokenType {
    /// Whitespace-delimited primitive word.
    Primitive = 0,
    /// Double-quoted string.
    String = 1,
    /// Single-quoted literal.
    Literal = 2,
    /// Number.
    Number = 3,
    /// Identifier.
    Name = 4,
    /// Punctuation operator.
    Punctuation = 5,
}

impl ScriptTokenType {
    /// Numeric type word.
    #[must_use]
    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

/// Diagnostic name for a token type word (donor `scriptTokenTypeName`).
#[must_use]
pub fn script_token_type_name(token_type: u32) -> &'static str {
    match token_type {
        1 => "string",
        2 => "literal",
        3 => "number",
        4 => "name",
        5 => "punctuation",
        _ => "",
    }
}

/// Diagnostic name for numeric subtype bits (donor `scriptNumberSubtypeName`).
pub fn script_number_subtype_name(subtype: u32) -> Result<String, ClientError> {
    let mut name: Option<String> = None;
    if subtype & NumberFlag::DECIMAL != 0 {
        name = Some("decimal".to_string());
    }
    if subtype & NumberFlag::HEX != 0 {
        name = Some("hex".to_string());
    }
    if subtype & NumberFlag::OCTAL != 0 {
        name = Some("octal".to_string());
    }
    if subtype & NumberFlag::BINARY != 0 {
        name = Some("binary".to_string());
    }
    let mut name = name.ok_or_else(|| {
        ClientError::BadUi("expected numeric subtype diagnostic reads an uninitialized source string".to_string())
    })?;
    if subtype & NumberFlag::LONG != 0 {
        name += " long";
    }
    if subtype & NumberFlag::UNSIGNED != 0 {
        name += " unsigned";
    }
    if subtype & NumberFlag::FLOAT != 0 {
        name += " float";
    }
    if subtype & NumberFlag::INTEGER != 0 {
        name += " integer";
    }
    Ok(name)
}

/// Strip surrounding quotes after cutting at the first NUL (donor `stripQuotes`).
fn strip_quotes(text: &str, quote: char) -> Result<String, ClientError> {
    let mut value = text.split('\0').next().unwrap_or("").to_string();
    if value.starts_with(quote) {
        value.remove(0);
    }
    if value.is_empty() {
        return Err(ClientError::BadUi(
            "StripQuotes reads before the source string allocation".to_string(),
        ));
    }
    if value.ends_with(quote) {
        value.pop();
    }
    Ok(value)
}

/// Strip surrounding double quotes (donor `stripDoubleQuotes`).
pub fn strip_double_quotes(text: &str) -> Result<String, ClientError> {
    strip_quotes(text, '"')
}

/// Strip surrounding single quotes (donor `stripSingleQuotes`).
pub fn strip_single_quotes(text: &str) -> Result<String, ClientError> {
    strip_quotes(text, '\'')
}

/// Numeric subtype bits (donor `NumberFlag`).
pub struct NumberFlag;

impl NumberFlag {
    /// Decimal base.
    pub const DECIMAL: u32 = 0x0008;
    /// Hexadecimal base.
    pub const HEX: u32 = 0x0100;
    /// Octal base.
    pub const OCTAL: u32 = 0x0200;
    /// Binary base.
    pub const BINARY: u32 = 0x0400;
    /// Floating-point value.
    pub const FLOAT: u32 = 0x0800;
    /// Integer value.
    pub const INTEGER: u32 = 0x1000;
    /// `l`/`L` suffix.
    pub const LONG: u32 = 0x2000;
    /// `u`/`U` suffix.
    pub const UNSIGNED: u32 = 0x4000;
}

/// Punctuation ids (donor `Punctuation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Punctuation {
    /// `>>=`.
    RightShiftAssign = 1,
    /// `<<=`.
    LeftShiftAssign = 2,
    /// `...`.
    Parameters = 3,
    /// `##`.
    PreprocessorMerge = 4,
    /// `&&`.
    LogicalAnd = 5,
    /// `||`.
    LogicalOr = 6,
    /// `>=`.
    GreaterOrEqual = 7,
    /// `<=`.
    LessOrEqual = 8,
    /// `==`.
    Equal = 9,
    /// `!=`.
    NotEqual = 10,
    /// `*=`.
    MultiplyAssign = 11,
    /// `/=`.
    DivideAssign = 12,
    /// `%=`.
    ModuloAssign = 13,
    /// `+=`.
    AddAssign = 14,
    /// `-=`.
    SubtractAssign = 15,
    /// `++`.
    Increment = 16,
    /// `--`.
    Decrement = 17,
    /// `&=`.
    BinaryAndAssign = 18,
    /// `|=`.
    BinaryOrAssign = 19,
    /// `^=`.
    BinaryXorAssign = 20,
    /// `>>`.
    RightShift = 21,
    /// `<<`.
    LeftShift = 22,
    /// `->`.
    PointerReference = 23,
    /// `::`.
    Scope = 24,
    /// `.*`.
    PointerToMember = 25,
    /// `*`.
    Multiply = 26,
    /// `/`.
    Divide = 27,
    /// `%`.
    Modulo = 28,
    /// `+`.
    Add = 29,
    /// `-`.
    Subtract = 30,
    /// `=`.
    Assign = 31,
    /// `&`.
    BinaryAnd = 32,
    /// `|`.
    BinaryOr = 33,
    /// `^`.
    BinaryXor = 34,
    /// `~`.
    BinaryNot = 35,
    /// `!`.
    LogicalNot = 36,
    /// `>`.
    Greater = 37,
    /// `<`.
    Less = 38,
    /// `.`.
    Reference = 39,
    /// `,`.
    Comma = 40,
    /// `;`.
    Semicolon = 41,
    /// `:`.
    Colon = 42,
    /// `?`.
    Question = 43,
    /// `(`.
    ParenthesisOpen = 44,
    /// `)`.
    ParenthesisClose = 45,
    /// `{`.
    BraceOpen = 46,
    /// `}`.
    BraceClose = 47,
    /// `[`.
    BracketOpen = 48,
    /// `]`.
    BracketClose = 49,
    /// `\`.
    Backslash = 50,
    /// `#`.
    Preprocessor = 51,
    /// `$`.
    Dollar = 52,
}

impl Punctuation {
    /// Numeric punctuation id.
    #[must_use]
    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// Punctuation for a numeric id, if valid.
    #[must_use]
    pub fn from_u32(id: u32) -> Option<Punctuation> {
        match id {
            1 => Some(Punctuation::RightShiftAssign),
            2 => Some(Punctuation::LeftShiftAssign),
            3 => Some(Punctuation::Parameters),
            4 => Some(Punctuation::PreprocessorMerge),
            5 => Some(Punctuation::LogicalAnd),
            6 => Some(Punctuation::LogicalOr),
            7 => Some(Punctuation::GreaterOrEqual),
            8 => Some(Punctuation::LessOrEqual),
            9 => Some(Punctuation::Equal),
            10 => Some(Punctuation::NotEqual),
            11 => Some(Punctuation::MultiplyAssign),
            12 => Some(Punctuation::DivideAssign),
            13 => Some(Punctuation::ModuloAssign),
            14 => Some(Punctuation::AddAssign),
            15 => Some(Punctuation::SubtractAssign),
            16 => Some(Punctuation::Increment),
            17 => Some(Punctuation::Decrement),
            18 => Some(Punctuation::BinaryAndAssign),
            19 => Some(Punctuation::BinaryOrAssign),
            20 => Some(Punctuation::BinaryXorAssign),
            21 => Some(Punctuation::RightShift),
            22 => Some(Punctuation::LeftShift),
            23 => Some(Punctuation::PointerReference),
            24 => Some(Punctuation::Scope),
            25 => Some(Punctuation::PointerToMember),
            26 => Some(Punctuation::Multiply),
            27 => Some(Punctuation::Divide),
            28 => Some(Punctuation::Modulo),
            29 => Some(Punctuation::Add),
            30 => Some(Punctuation::Subtract),
            31 => Some(Punctuation::Assign),
            32 => Some(Punctuation::BinaryAnd),
            33 => Some(Punctuation::BinaryOr),
            34 => Some(Punctuation::BinaryXor),
            35 => Some(Punctuation::BinaryNot),
            36 => Some(Punctuation::LogicalNot),
            37 => Some(Punctuation::Greater),
            38 => Some(Punctuation::Less),
            39 => Some(Punctuation::Reference),
            40 => Some(Punctuation::Comma),
            41 => Some(Punctuation::Semicolon),
            42 => Some(Punctuation::Colon),
            43 => Some(Punctuation::Question),
            44 => Some(Punctuation::ParenthesisOpen),
            45 => Some(Punctuation::ParenthesisClose),
            46 => Some(Punctuation::BraceOpen),
            47 => Some(Punctuation::BraceClose),
            48 => Some(Punctuation::BracketOpen),
            49 => Some(Punctuation::BracketClose),
            50 => Some(Punctuation::Backslash),
            51 => Some(Punctuation::Preprocessor),
            52 => Some(Punctuation::Dollar),
            _ => None,
        }
    }
}

/// Lexer behavior bits (donor `LexerFlag`).
pub struct LexerFlag;

impl LexerFlag {
    /// Default behavior.
    pub const NONE: u32 = 0;
    /// Suppress error diagnostics (failures still abort the token).
    pub const NO_ERRORS: u32 = 0x0001;
    /// Suppress warning diagnostics.
    pub const NO_WARNINGS: u32 = 0x0002;
    /// Do not concatenate adjacent quoted strings.
    pub const NO_STRING_CONCATENATION: u32 = 0x0004;
    /// Do not process backslash escapes in quoted strings.
    pub const NO_STRING_ESCAPES: u32 = 0x0008;
    /// Lex whitespace-delimited primitive words instead of typed tokens.
    pub const PRIMITIVE: u32 = 0x0010;
    /// Do not lex `0b` binary numbers.
    pub const NO_BINARY_NUMBERS: u32 = 0x0020;
    /// Parse numbers without value conversion (kept for flag parity).
    pub const NO_NUMBER_VALUES: u32 = 0x0040;
}

/// Source position of a token or diagnostic (donor `SourceLocation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    /// Source path.
    pub path: String,
    /// 1-based line.
    pub line: usize,
    /// 1-based column.
    pub column: usize,
}

/// Diagnostic severity (donor `ScriptDiagnostic` severity tag).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    /// Non-fatal diagnostic.
    Warning,
    /// Fatal diagnostic.
    Error,
}

impl DiagnosticSeverity {
    /// Donor severity tag.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DiagnosticSeverity::Warning => "warning",
            DiagnosticSeverity::Error => "error",
        }
    }
}

/// Optional diagnostic sink (donor `report` callback).
pub type DiagnosticReport = Option<Box<dyn FnMut(&ScriptDiagnostic)>>;

/// One reported lexer diagnostic (donor `ScriptDiagnostic`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptDiagnostic {
    /// Diagnostic severity.
    pub severity: DiagnosticSeverity,
    /// Diagnostic message.
    pub message: String,
    /// Position the diagnostic was reported at.
    pub location: SourceLocation,
}

/// One lexed token (donor `ScriptToken`).
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptToken {
    /// Double-quoted string; `text` keeps the quotes, `value` is unescaped.
    String {
        /// Full token text including quotes.
        text: String,
        /// Unescaped inner value.
        value: String,
        /// Length of `text` in characters.
        length: usize,
        /// Token location.
        location: SourceLocation,
        /// Whitespace skipped before this token.
        leading_whitespace: String,
        /// Lines crossed by that whitespace.
        lines_crossed: usize,
    },
    /// Single-quoted literal.
    Literal {
        /// Full token text including quotes.
        text: String,
        /// Unescaped inner value.
        value: String,
        /// Length of `text` in characters.
        length: usize,
        /// Token location.
        location: SourceLocation,
        /// Whitespace skipped before this token.
        leading_whitespace: String,
        /// Lines crossed by that whitespace.
        lines_crossed: usize,
    },
    /// Number; `text` excludes `l`/`u` suffixes.
    Number {
        /// Token text without suffixes.
        text: String,
        /// [`NumberFlag`] bits.
        flags: u32,
        /// Unsigned 32-bit integer value.
        integer_value: u32,
        /// Floating-point value.
        float_value: f64,
        /// Token location.
        location: SourceLocation,
        /// Whitespace skipped before this token.
        leading_whitespace: String,
        /// Lines crossed by that whitespace.
        lines_crossed: usize,
    },
    /// Name.
    Name {
        /// Token text.
        text: String,
        /// Name value (same as `text`).
        value: String,
        /// Length of `text` in characters.
        length: usize,
        /// Token location.
        location: SourceLocation,
        /// Whitespace skipped before this token.
        leading_whitespace: String,
        /// Lines crossed by that whitespace.
        lines_crossed: usize,
    },
    /// Punctuation.
    Punctuation {
        /// Token text.
        text: String,
        /// Punctuation value (same as `text`).
        value: String,
        /// Numeric punctuation id.
        punctuation: u32,
        /// Token location.
        location: SourceLocation,
        /// Whitespace skipped before this token.
        leading_whitespace: String,
        /// Lines crossed by that whitespace.
        lines_crossed: usize,
    },
    /// Primitive token.
    Primitive {
        /// Token text.
        text: String,
        /// Primitive value (same as `text`).
        value: String,
        /// Token location.
        location: SourceLocation,
        /// Whitespace skipped before this token.
        leading_whitespace: String,
        /// Lines crossed by that whitespace.
        lines_crossed: usize,
    },
}

impl ScriptToken {
    /// Donor `kind` tag.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            ScriptToken::String { .. } => "string",
            ScriptToken::Literal { .. } => "literal",
            ScriptToken::Number { .. } => "number",
            ScriptToken::Name { .. } => "name",
            ScriptToken::Punctuation { .. } => "punctuation",
            ScriptToken::Primitive { .. } => "primitive",
        }
    }

    /// Raw token text.
    #[must_use]
    pub fn text(&self) -> &str {
        match self {
            ScriptToken::String { text, .. }
            | ScriptToken::Literal { text, .. }
            | ScriptToken::Number { text, .. }
            | ScriptToken::Name { text, .. }
            | ScriptToken::Punctuation { text, .. }
            | ScriptToken::Primitive { text, .. } => text,
        }
    }

    /// Token location.
    #[must_use]
    pub fn location(&self) -> &SourceLocation {
        match self {
            ScriptToken::String { location, .. }
            | ScriptToken::Literal { location, .. }
            | ScriptToken::Number { location, .. }
            | ScriptToken::Name { location, .. }
            | ScriptToken::Punctuation { location, .. }
            | ScriptToken::Primitive { location, .. } => location,
        }
    }

    /// Whitespace skipped before this token.
    #[must_use]
    pub fn leading_whitespace(&self) -> &str {
        match self {
            ScriptToken::String { leading_whitespace, .. }
            | ScriptToken::Literal { leading_whitespace, .. }
            | ScriptToken::Number { leading_whitespace, .. }
            | ScriptToken::Name { leading_whitespace, .. }
            | ScriptToken::Punctuation { leading_whitespace, .. }
            | ScriptToken::Primitive { leading_whitespace, .. } => leading_whitespace,
        }
    }

    /// Lines crossed by the leading whitespace.
    #[must_use]
    pub fn lines_crossed(&self) -> usize {
        match self {
            ScriptToken::String { lines_crossed, .. }
            | ScriptToken::Literal { lines_crossed, .. }
            | ScriptToken::Number { lines_crossed, .. }
            | ScriptToken::Name { lines_crossed, .. }
            | ScriptToken::Punctuation { lines_crossed, .. }
            | ScriptToken::Primitive { lines_crossed, .. } => *lines_crossed,
        }
    }

    /// Token value, if the kind carries one (numbers carry values instead).
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        match self {
            ScriptToken::String { value, .. }
            | ScriptToken::Literal { value, .. }
            | ScriptToken::Name { value, .. }
            | ScriptToken::Punctuation { value, .. }
            | ScriptToken::Primitive { value, .. } => Some(value),
            ScriptToken::Number { .. } => None,
        }
    }
}

/// Decoded token plus its retained numeric words (donor `ScriptTokenRecord`).
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptTokenRecord {
    /// Decoded token.
    pub token: ScriptToken,
    /// Retained subtype word.
    pub subtype: u32,
    /// Retained integer value word.
    pub integer_value: u32,
    /// Retained float value.
    pub float_value: f64,
}

/// Lexer construction options (donor `ScriptLexerOptions`).
pub struct ScriptLexerOptions {
    /// [`LexerFlag`] bits.
    pub flags: u32,
    /// Maximum raw token length; must be at least 4.
    pub max_token_length: usize,
    /// Diagnostic sink.
    pub report: DiagnosticReport,
}

impl Default for ScriptLexerOptions {
    fn default() -> Self {
        ScriptLexerOptions {
            flags: LexerFlag::NONE,
            max_token_length: DEFAULT_SCRIPT_TOKEN_LIMIT,
            report: None,
        }
    }
}

/// One punctuation spelling and its id (donor `ScriptPunctuation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptPunctuation {
    /// Punctuation spelling.
    pub text: String,
    /// Numeric punctuation id.
    pub punctuation: u32,
}

/// Default punctuation spellings in donor order (donor `PUNCTUATIONS`).
const DEFAULT_PUNCTUATIONS: [(&str, u32); 52] = [
    (">>=", Punctuation::RightShiftAssign as u32),
    ("<<=", Punctuation::LeftShiftAssign as u32),
    ("...", Punctuation::Parameters as u32),
    ("##", Punctuation::PreprocessorMerge as u32),
    ("&&", Punctuation::LogicalAnd as u32),
    ("||", Punctuation::LogicalOr as u32),
    (">=", Punctuation::GreaterOrEqual as u32),
    ("<=", Punctuation::LessOrEqual as u32),
    ("==", Punctuation::Equal as u32),
    ("!=", Punctuation::NotEqual as u32),
    ("*=", Punctuation::MultiplyAssign as u32),
    ("/=", Punctuation::DivideAssign as u32),
    ("%=", Punctuation::ModuloAssign as u32),
    ("+=", Punctuation::AddAssign as u32),
    ("-=", Punctuation::SubtractAssign as u32),
    ("++", Punctuation::Increment as u32),
    ("--", Punctuation::Decrement as u32),
    ("&=", Punctuation::BinaryAndAssign as u32),
    ("|=", Punctuation::BinaryOrAssign as u32),
    ("^=", Punctuation::BinaryXorAssign as u32),
    (">>", Punctuation::RightShift as u32),
    ("<<", Punctuation::LeftShift as u32),
    ("->", Punctuation::PointerReference as u32),
    ("::", Punctuation::Scope as u32),
    (".*", Punctuation::PointerToMember as u32),
    ("*", Punctuation::Multiply as u32),
    ("/", Punctuation::Divide as u32),
    ("%", Punctuation::Modulo as u32),
    ("+", Punctuation::Add as u32),
    ("-", Punctuation::Subtract as u32),
    ("=", Punctuation::Assign as u32),
    ("&", Punctuation::BinaryAnd as u32),
    ("|", Punctuation::BinaryOr as u32),
    ("^", Punctuation::BinaryXor as u32),
    ("~", Punctuation::BinaryNot as u32),
    ("!", Punctuation::LogicalNot as u32),
    (">", Punctuation::Greater as u32),
    ("<", Punctuation::Less as u32),
    (".", Punctuation::Reference as u32),
    (",", Punctuation::Comma as u32),
    (";", Punctuation::Semicolon as u32),
    (":", Punctuation::Colon as u32),
    ("?", Punctuation::Question as u32),
    ("(", Punctuation::ParenthesisOpen as u32),
    (")", Punctuation::ParenthesisClose as u32),
    ("{", Punctuation::BraceOpen as u32),
    ("}", Punctuation::BraceClose as u32),
    ("[", Punctuation::BracketOpen as u32),
    ("]", Punctuation::BracketClose as u32),
    ("\\", Punctuation::Backslash as u32),
    ("#", Punctuation::Preprocessor as u32),
    ("$", Punctuation::Dollar as u32),
];

/// Default punctuation records.
fn default_punctuations() -> Vec<ScriptPunctuation> {
    DEFAULT_PUNCTUATIONS
        .iter()
        .map(|(text, punctuation)| ScriptPunctuation {
            text: (*text).to_string(),
            punctuation: *punctuation,
        })
        .collect()
}

/// One hash-chain link into the punctuation records.
#[derive(Debug, Clone, Copy)]
struct PunctuationLink {
    /// Index into the punctuation records.
    spec: usize,
    /// Next link (`links[next - 1]`), or 0 for the chain end.
    next: usize,
}

/// Longest-match punctuation lookup (donor `punctuationTable`).
#[derive(Debug, Clone)]
struct PunctuationTable {
    /// Chain heads by first byte (`links[heads[byte] - 1]`).
    heads: [usize; 256],
    /// Links sorted longest-first within each chain.
    links: Vec<PunctuationLink>,
}

/// Build the longest-match punctuation lookup (donor `punctuationTable`).
fn punctuation_table(punctuations: &[ScriptPunctuation]) -> Result<PunctuationTable, ClientError> {
    let mut heads = [0usize; 256];
    let mut links: Vec<PunctuationLink> = (0..punctuations.len())
        .map(|spec| PunctuationLink { spec, next: 0 })
        .collect();
    for index in 0..links.len() {
        let first = punctuations
            .get(index)
            .and_then(|spec| spec.text.chars().next())
            .ok_or_else(|| ClientError::BadUi("default punctuation record is missing".to_string()))?;
        if !first.is_ascii() {
            return Err(ClientError::BadUi(
                "punctuation table index exceeds its source signed-char allocation".to_string(),
            ));
        }
        let head = heads[first as usize];
        let mut pointer = head;
        let mut previous: Option<usize> = None;
        while pointer != 0 {
            let current = links
                .get(pointer - 1)
                .ok_or_else(|| ClientError::BadUi("default punctuation link is missing".to_string()))?;
            let current_len = punctuations.get(current.spec).map(|spec| spec.text.len()).unwrap_or(0);
            let added_len = punctuations.get(index).map(|spec| spec.text.len()).unwrap_or(0);
            if current_len < added_len {
                break;
            }
            previous = Some(pointer - 1);
            pointer = current.next;
        }
        if let Some(link) = links.get_mut(index) {
            link.next = pointer;
        }
        match previous {
            None => heads[first as usize] = index + 1,
            Some(slot) => {
                if let Some(link) = links.get_mut(slot) {
                    link.next = index + 1;
                }
            }
        }
    }
    Ok(PunctuationTable { heads, links })
}

/// Fatal script token failure (donor `ScriptLanguageError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptLanguageError {
    /// Failure diagnostic.
    pub diagnostic: ScriptDiagnostic,
    /// All diagnostics reported so far, including the failure.
    pub diagnostics: Vec<ScriptDiagnostic>,
}

impl ScriptLanguageError {
    /// Build a reported failure from the current diagnostic log.
    fn reported(diagnostic: ScriptDiagnostic, reported: &[ScriptDiagnostic]) -> Self {
        ScriptLanguageError {
            diagnostic,
            diagnostics: reported.to_vec(),
        }
    }

    /// Build a non-reported internal failure (donor `RangeError` sites).
    ///
    /// These errors are deliberately absent from the lexer failure set so
    /// [`ScriptLexer::is_source_failure`] keeps them propagating instead of
    /// converting them to a quiet `false`.
    fn internal(location: SourceLocation, error: ClientError) -> Self {
        let diagnostic = ScriptDiagnostic {
            severity: DiagnosticSeverity::Error,
            message: error.to_string(),
            location,
        };
        ScriptLanguageError {
            diagnostic: diagnostic.clone(),
            diagnostics: vec![diagnostic],
        }
    }
}

impl fmt::Display for ScriptLanguageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let location = &self.diagnostic.location;
        write!(
            f,
            "{}:{}:{}: {}",
            location.path, location.line, location.column, self.diagnostic.message
        )
    }
}

impl Error for ScriptLanguageError {}

/// Whether ASCII `character` is a decimal digit (donor `isDigit`).
fn is_digit(character: Option<char>) -> bool {
    matches!(character, Some('0'..='9'))
}

/// Whether `character` can start a name (donor `isNameStart`).
fn is_name_start(character: Option<char>) -> bool {
    matches!(character, Some('a'..='z' | 'A'..='Z' | '_'))
}

/// Whether `character` can continue a name (donor `isNameCharacter`).
fn is_name_character(character: Option<char>) -> bool {
    is_name_start(character) || is_digit(character)
}

/// Whether `character` scans inside a hex number (donor quirk: only `A`
/// is accepted as an uppercase digit).
fn is_source_number_hex_digit(character: Option<char>) -> bool {
    matches!(character, Some('0'..='9' | 'a'..='f' | 'A'))
}

/// Whether `character` scans inside a `\x` escape (donor quirk: the full
/// ASCII letter range, not just hex digits).
fn is_source_escape_hex_digit(character: Option<char>) -> bool {
    matches!(character, Some('0'..='9' | 'a'..='z' | 'A'..='Z'))
}

/// Whether `flags` carries `flag` (donor `hasFlag`).
fn has_flag(flags: u32, flag: u32) -> bool {
    flags & flag != 0
}

/// Record a cleared retained image decodes to (donor `readRecord` of zeros).
fn cleared_record(context: &SourceTokenContext) -> ScriptTokenRecord {
    ScriptTokenRecord {
        token: ScriptToken::Primitive {
            text: String::new(),
            value: String::new(),
            location: SourceLocation {
                path: context.path.clone(),
                line: 0,
                column: context.column.max(0) as usize,
            },
            leading_whitespace: context.leading_whitespace.clone(),
            lines_crossed: 0,
        },
        subtype: 0,
        integer_value: 0,
        float_value: 0.0,
    }
}

/// Text-source script lexer (donor `ScriptLexer`, string-source path).
pub struct ScriptLexer {
    chars: Vec<char>,
    path: String,
    flags: u32,
    max_token_length: usize,
    report: DiagnosticReport,
    reported: Vec<ScriptDiagnostic>,
    failures: Vec<ScriptDiagnostic>,
    offset: usize,
    line: usize,
    last_offset: usize,
    last_line: usize,
    whitespace_start: usize,
    whitespace_end: usize,
    column: usize,
    token_available: bool,
    retained: SourceTokenMemory,
    retained_record: Option<ScriptTokenRecord>,
    token_context: SourceTokenContext,
    punctuations: Vec<ScriptPunctuation>,
    punctuation_lookup: PunctuationTable,
}

impl ScriptLexer {
    /// Create a lexer over `source` reported under `path` (donor constructor).
    pub fn new(source: &str, path: &str, options: ScriptLexerOptions) -> Result<Self, ClientError> {
        if options.max_token_length < 4 {
            return Err(ClientError::BadUi(
                "maxTokenLength must be an integer of at least 4".to_string(),
            ));
        }
        let punctuations = default_punctuations();
        let punctuation_lookup = punctuation_table(&punctuations)?;
        Ok(ScriptLexer {
            chars: source.chars().collect(),
            path: path.to_string(),
            flags: options.flags,
            max_token_length: options.max_token_length,
            report: options.report,
            reported: Vec::new(),
            failures: Vec::new(),
            offset: 0,
            line: 1,
            last_offset: 0,
            last_line: 1,
            whitespace_start: 0,
            whitespace_end: 0,
            column: 1,
            token_available: false,
            retained: SourceTokenMemory::new(),
            retained_record: None,
            token_context: SourceTokenContext {
                path: path.to_string(),
                column: 1,
                leading_whitespace: String::new(),
            },
            punctuations,
            punctuation_lookup,
        })
    }

    /// Diagnostics reported so far (donor `diagnostics`).
    #[must_use]
    pub fn diagnostics(&self) -> &[ScriptDiagnostic] {
        &self.reported
    }

    /// Whether `error` is a token failure raised by this lexer (donor `isSourceFailure`).
    #[must_use]
    pub fn is_source_failure(&self, error: &ScriptLanguageError) -> bool {
        self.failures.contains(&error.diagnostic)
    }

    /// Current scan position (donor `currentLocation`).
    #[must_use]
    pub fn current_location(&self) -> SourceLocation {
        SourceLocation {
            path: self.path.clone(),
            line: self.line,
            column: self.column,
        }
    }

    /// Whether the scan reached the end of the source (donor `endOfScript`).
    #[must_use]
    pub fn end_of_script(&self) -> bool {
        self.offset >= self.chars.len()
    }

    /// Read the next token (donor `next`).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<ScriptToken>, ScriptLanguageError> {
        let mut scratch = SourceTokenMemory::new();
        Ok(self.next_into(&mut scratch)?.map(|record| record.token))
    }

    /// Active script flags (donor `getScriptFlags`).
    #[must_use]
    pub fn get_script_flags(&self) -> u32 {
        self.flags
    }

    /// Replace the active script flags (donor `setScriptFlags`).
    pub fn set_script_flags(&mut self, flags: u32) {
        self.flags = flags;
    }

    /// Punctuation spelling for a numeric id (donor `punctuationFromNum`).
    #[must_use]
    pub fn punctuation_from_num(&self, number: u32) -> String {
        self.punctuations
            .iter()
            .find(|spec| spec.punctuation == number)
            .map(|spec| spec.text.clone())
            .unwrap_or_else(|| "unkown punctuation".to_string())
    }

    /// Replace the punctuation set, or restore the default for `None` (donor `setPunctuations`).
    pub fn set_punctuations(&mut self, punctuations: Option<&[ScriptPunctuation]>) -> Result<(), ClientError> {
        let values: Vec<ScriptPunctuation> = match punctuations {
            Some(list) => list.to_vec(),
            None => default_punctuations(),
        };
        let table = punctuation_table(&values)?;
        self.punctuations = values;
        self.punctuation_lookup = table;
        Ok(())
    }

    /// Lines crossed by the last token read (donor `numLinesCrossed`).
    #[must_use]
    pub fn num_lines_crossed(&self) -> usize {
        self.line - self.last_line
    }

    /// Replay one recorded whitespace character, or 0 when exhausted (donor `nextWhitespaceChar`).
    pub fn next_whitespace_char(&mut self) -> Result<i32, ScriptLanguageError> {
        if self.whitespace_start == self.whitespace_end {
            return Ok(0);
        }
        let slot = self.whitespace_start;
        self.whitespace_start += 1;
        let Some(character) = self.chars.get(slot).copied() else {
            let here = self.current_location();
            return Err(ScriptLanguageError::internal(
                here,
                ClientError::BadUi("script whitespace pointer exceeds its source allocation".to_string()),
            ));
        };
        let byte = character as u32;
        Ok(if byte < 128 { byte as i32 } else { byte as i32 - 256 })
    }

    /// Rewind the last checked token (donor `rewindCheckedToken`).
    fn rewind_checked_token(&mut self) {
        self.offset = self.last_offset;
    }

    /// Push the retained token back for the next read (donor `unreadLast`).
    pub fn unread_last(&mut self) {
        self.token_available = true;
    }

    /// Require the next token to equal `text` (donor `expectTokenString`).
    pub fn expect_token_string(&mut self, text: &str) -> Result<bool, ScriptLanguageError> {
        let mut scratch = SourceTokenMemory::new();
        if !self.read_expected_into(&mut scratch)? {
            self.error(format!("couldn't find expected {text}"));
            return Ok(false);
        }
        let current = scratch.string().map_err(|error| self.internal(error))?;
        if current != text {
            self.error(format!("expected {text}, found {current}"));
            return Ok(false);
        }
        Ok(true)
    }

    /// Require a next token into `output` (donor `expectAnyToken`).
    pub fn expect_any_token(&mut self, output: &mut SourceTokenMemory) -> Result<bool, ScriptLanguageError> {
        if self.read_expected_into(output)? {
            return Ok(true);
        }
        self.error("couldn't read expected token".to_string());
        Ok(false)
    }

    /// Require a next token of a type into `output` (donor `expectTokenType`).
    ///
    /// The `subtype < 0` guard of the donor is unreachable: ids are `u32`.
    pub fn expect_token_type(
        &mut self,
        type_id: u32,
        subtype: u32,
        output: &mut SourceTokenMemory,
    ) -> Result<bool, ScriptLanguageError> {
        if !self.expect_any_token(output)? {
            return Ok(false);
        }
        let actual = output.token_type().map_err(|error| self.internal(error))? as u32;
        if actual != type_id {
            let name = script_token_type_name(type_id);
            if name.is_empty() {
                let here = self.current_location();
                return Err(ScriptLanguageError::internal(
                    here,
                    ClientError::BadUi(
                        "PS_ExpectTokenType diagnostic reads an uninitialized source string".to_string(),
                    ),
                ));
            }
            let found = output.string().map_err(|error| self.internal(error))?;
            self.error(format!("expected a {name}, found {found}"));
            return Ok(false);
        }
        if type_id == ScriptTokenType::Number.as_u32() {
            let out_subtype = output.subtype().map_err(|error| self.internal(error))? as u32;
            if out_subtype & subtype != subtype {
                let want = script_number_subtype_name(subtype).map_err(|error| self.internal(error))?;
                let found = output.string().map_err(|error| self.internal(error))?;
                self.error(format!("expected {want}, found {found}"));
            }
        }
        if type_id == ScriptTokenType::Punctuation.as_u32() {
            let out_subtype = output.subtype().map_err(|error| self.internal(error))? as u32;
            if out_subtype != subtype {
                let here = self.current_location();
                return Err(ScriptLanguageError::internal(
                    here,
                    ClientError::BadUi("PS_ExpectTokenType passes a punctuation struct to source %s".to_string()),
                ));
            }
        }
        Ok(true)
    }

    /// Consume the next token when it equals `text` (donor `checkTokenString`).
    pub fn check_token_string(&mut self, text: &str) -> Result<bool, ScriptLanguageError> {
        let mut scratch = SourceTokenMemory::new();
        if !self.read_expected_into(&mut scratch)? {
            return Ok(false);
        }
        let current = scratch.string().map_err(|error| self.internal(error))?;
        if current == text {
            return Ok(true);
        }
        self.rewind_checked_token();
        Ok(false)
    }

    /// Consume the next token when its type matches, copying it into `output` (donor `checkTokenType`).
    pub fn check_token_type(
        &mut self,
        type_id: u32,
        subtype: u32,
        output: &mut SourceTokenMemory,
    ) -> Result<bool, ScriptLanguageError> {
        let mut scratch = SourceTokenMemory::new();
        if !self.read_expected_into(&mut scratch)? {
            return Ok(false);
        }
        let actual = scratch.token_type().map_err(|error| self.internal(error))? as u32;
        let out_subtype = scratch.subtype().map_err(|error| self.internal(error))? as u32;
        if actual == type_id && out_subtype & subtype == subtype {
            let here = self.current_location();
            output
                .copy_from(&scratch)
                .map_err(|error| ScriptLanguageError::internal(here, error))?;
            return Ok(true);
        }
        self.rewind_checked_token();
        Ok(false)
    }

    /// Skip tokens until one equals `text` (donor `skipUntilString`).
    pub fn skip_until_string(&mut self, text: &str) -> Result<bool, ScriptLanguageError> {
        let mut scratch = SourceTokenMemory::new();
        while self.read_expected_into(&mut scratch)? {
            let current = scratch.string().map_err(|error| self.internal(error))?;
            if current == text {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Skip raw source until `text` starts here (donor `scriptSkipTo`).
    pub fn script_skip_to(&mut self, text: &str) -> bool {
        loop {
            self.read_whitespace();
            if self.at_end() {
                return false;
            }
            if self.source_starts_with(text) {
                return true;
            }
            self.advance(1);
        }
    }

    /// Read a signed float token (donor `readSignedFloat`).
    pub fn read_signed_float(&mut self) -> Result<f64, ScriptLanguageError> {
        let mut scratch = SourceTokenMemory::new();
        self.expect_any_token(&mut scratch)?;
        let mut sign = 1.0;
        let text = scratch.string().map_err(|error| self.internal(error))?;
        if text == "-" {
            sign = -1.0;
            self.expect_token_type(ScriptTokenType::Number.as_u32(), 0, &mut scratch)?;
        } else {
            let actual = scratch.token_type().map_err(|error| self.internal(error))? as u32;
            if actual != ScriptTokenType::Number.as_u32() {
                self.error(format!("expected float value, found {text}\n"));
            }
        }
        let value = scratch.float_value().map_err(|error| self.internal(error))?;
        Ok(sign * value)
    }

    /// Read a signed integer token (donor `readSignedInt`).
    pub fn read_signed_int(&mut self) -> Result<i32, ScriptLanguageError> {
        let mut scratch = SourceTokenMemory::new();
        self.expect_any_token(&mut scratch)?;
        let mut sign = 1i32;
        let text = scratch.string().map_err(|error| self.internal(error))?;
        if text == "-" {
            sign = -1;
            self.expect_token_type(ScriptTokenType::Number.as_u32(), NumberFlag::INTEGER, &mut scratch)?;
        } else {
            let actual = scratch.token_type().map_err(|error| self.internal(error))? as u32;
            let out_subtype = scratch.subtype().map_err(|error| self.internal(error))?;
            if actual != ScriptTokenType::Number.as_u32() || out_subtype == NumberFlag::FLOAT as i32 {
                self.error(format!("expected integer value, found {text}\n"));
            }
        }
        let value = scratch.integer_value().map_err(|error| self.internal(error))?;
        Ok((value as i32).wrapping_mul(sign))
    }

    /// Read a single-quoted literal into `output` (donor `readLiteralInto`).
    ///
    /// The caller positions the scan at the opening quote.
    pub fn read_literal_into(&mut self, output: &mut SourceTokenMemory) -> Result<bool, ScriptLanguageError> {
        match self.read_literal_inner(output) {
            Ok(value) => Ok(value),
            Err(error) if self.is_source_failure(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Literal body with source failures propagated to the caller.
    fn read_literal_inner(&mut self, output: &mut SourceTokenMemory) -> Result<bool, ScriptLanguageError> {
        output
            .set_token_type(ScriptTokenType::Literal.as_u32() as i32)
            .map_err(|error| self.internal(error))?;
        let first = self.take();
        let byte = u8::try_from(first).map_err(|_| self.byte_error())?;
        output.set_string_byte(0, byte).map_err(|error| self.internal(error))?;
        if self.literal_byte() == 0 {
            self.error("end of file before trailing '".to_string());
            return Ok(false);
        }
        let value = if self.literal_byte() == 92 {
            self.read_escape()? as u32
        } else {
            self.take()
        };
        let byte = u8::try_from(value).map_err(|_| self.byte_error())?;
        output.set_string_byte(1, byte).map_err(|error| self.internal(error))?;
        if self.literal_byte() != 39 {
            self.warn("too many characters in literal, ignored".to_string());
            while !matches!(self.literal_byte(), 0 | 39 | 10) {
                self.take();
            }
            if self.literal_byte() == 39 {
                self.take();
            }
        }
        let closing = self.take();
        let byte = u8::try_from(closing).map_err(|_| self.byte_error())?;
        output.set_string_byte(2, byte).map_err(|error| self.internal(error))?;
        output.set_string_byte(3, 0).map_err(|error| self.internal(error))?;
        let subtype = if value < 128 { value as i32 } else { value as i32 - 256 };
        output.set_subtype(subtype).map_err(|error| self.internal(error))?;
        Ok(true)
    }

    /// Read a token, converting source failures to `false` (donor `readExpectedInto`).
    fn read_expected_into(&mut self, output: &mut SourceTokenMemory) -> Result<bool, ScriptLanguageError> {
        match self.next_into(output) {
            Ok(record) => Ok(record.is_some()),
            Err(error) if self.is_source_failure(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Report an error diagnostic unless suppressed (donor `error`).
    fn error(&mut self, message: String) {
        if has_flag(self.flags, LexerFlag::NO_ERRORS) {
            return;
        }
        let diagnostic = ScriptDiagnostic {
            severity: DiagnosticSeverity::Error,
            message,
            location: self.current_location(),
        };
        self.reported.push(diagnostic.clone());
        if let Some(report) = self.report.as_mut() {
            report(&diagnostic);
        }
    }

    /// Read the next token into `output`, writing it even on failure (donor `nextInto`).
    pub fn next_into(
        &mut self,
        output: &mut SourceTokenMemory,
    ) -> Result<Option<ScriptTokenRecord>, ScriptLanguageError> {
        if self.token_available {
            self.token_available = false;
            let here = self.current_location();
            output
                .copy_from(&self.retained)
                .map_err(|error| ScriptLanguageError::internal(here, error))?;
            let record = self
                .retained_record
                .clone()
                .unwrap_or_else(|| cleared_record(&self.token_context));
            return Ok(Some(record));
        }

        self.last_offset = self.offset;
        self.last_line = self.line;
        self.whitespace_start = self.offset;
        let here = self.current_location();
        output
            .clear()
            .map_err(|error| ScriptLanguageError::internal(here, error))?;
        let pointer = self.source_pointer()?;
        let here = self.current_location();
        output
            .set_whitespace_start(pointer)
            .map_err(|error| ScriptLanguageError::internal(here, error))?;
        let whitespace_start = self.offset;
        let line_before_whitespace = self.line;
        self.read_whitespace();
        if self.at_end() {
            return Ok(None);
        }

        self.whitespace_end = self.offset;
        let pointer = self.source_pointer()?;
        let here = self.current_location();
        output
            .set_whitespace_end(pointer)
            .map_err(|error| ScriptLanguageError::internal(here, error))?;
        let line = self.retained_i32(self.line, "line")?;
        let here = self.current_location();
        output
            .set_line(line)
            .map_err(|error| ScriptLanguageError::internal(here, error))?;
        let crossed = self.retained_i32(self.line - line_before_whitespace, "lines crossed")?;
        let here = self.current_location();
        output
            .set_lines_crossed(crossed)
            .map_err(|error| ScriptLanguageError::internal(here, error))?;
        let leading_whitespace = self.source_slice(whitespace_start, self.offset);
        let lines_crossed = self.line - line_before_whitespace;
        let token_location = self.current_location();
        let character = self.peek(0);

        let (token, subtype, integer_value, float_value, primitive) = match character {
            Some(quote) if quote == '"' || quote == '\'' => {
                let (value, length) = self.read_quoted(quote, output)?;
                let subtype = output.subtype().map_err(|error| self.internal(error))? as u32;
                let text = format!("{quote}{value}{quote}");
                let token = if quote == '"' {
                    ScriptToken::String {
                        text,
                        value,
                        length,
                        location: token_location,
                        leading_whitespace,
                        lines_crossed,
                    }
                } else {
                    ScriptToken::Literal {
                        text,
                        value,
                        length,
                        location: token_location,
                        leading_whitespace,
                        lines_crossed,
                    }
                };
                (token, subtype, 0, 0.0, false)
            }
            Some(other) if is_digit(Some(other)) || (other == '.' && is_digit(self.peek(1))) => {
                let (text, flags, integer_value, float_value) = self.read_number(output)?;
                let token = ScriptToken::Number {
                    text,
                    flags,
                    integer_value,
                    float_value,
                    location: token_location,
                    leading_whitespace,
                    lines_crossed,
                };
                (token, flags, integer_value, float_value, false)
            }
            _ if has_flag(self.flags, LexerFlag::PRIMITIVE) => {
                let mut text = String::new();
                let mut length = 0usize;
                while let Some(next) = self.peek(0) {
                    if next as u32 >= 128 || next <= ' ' || next == ';' {
                        break;
                    }
                    if length >= self.max_token_length {
                        return Err(self.fail(format!(
                            "primitive token longer than MAX_TOKEN = {}",
                            self.max_token_length
                        )));
                    }
                    let byte = u8::try_from(next as u32).map_err(|_| self.byte_error())?;
                    output
                        .set_string_byte(length, byte)
                        .map_err(|error| self.internal(error))?;
                    self.advance(1);
                    text.push(next);
                    length += 1;
                }
                output
                    .set_string_byte(length, 0)
                    .map_err(|error| self.internal(error))?;
                let token = ScriptToken::Primitive {
                    text: text.clone(),
                    value: text,
                    location: token_location,
                    leading_whitespace,
                    lines_crossed,
                };
                (token, 0, 0, 0.0, true)
            }
            Some(other) if is_name_start(Some(other)) => {
                let value = self.read_name(output)?;
                let subtype = output.subtype().map_err(|error| self.internal(error))? as u32;
                let length = value.chars().count();
                let token = ScriptToken::Name {
                    text: value.clone(),
                    value,
                    length,
                    location: token_location,
                    leading_whitespace,
                    lines_crossed,
                };
                (token, subtype, 0, 0.0, false)
            }
            _ => {
                let Some((text, id)) = self.read_punctuation()? else {
                    return Err(self.fail("can't read token".to_string()));
                };
                output.write_string(&text).map_err(|error| self.internal(error))?;
                self.advance(text.chars().count());
                output
                    .set_token_type(ScriptTokenType::Punctuation.as_u32() as i32)
                    .map_err(|error| self.internal(error))?;
                output.set_subtype(id as i32).map_err(|error| self.internal(error))?;
                let token = ScriptToken::Punctuation {
                    text: text.clone(),
                    value: text,
                    punctuation: id,
                    location: token_location,
                    leading_whitespace,
                    lines_crossed,
                };
                (token, id, 0, 0.0, false)
            }
        };

        let record = ScriptTokenRecord {
            token,
            subtype,
            integer_value,
            float_value,
        };
        if !primitive {
            let column = self.retained_i32(record.token.location().column, "column")?;
            self.token_context = SourceTokenContext {
                path: self.path.clone(),
                column,
                leading_whitespace: record.token.leading_whitespace().to_string(),
            };
            let here = self.current_location();
            self.retained
                .copy_from(output)
                .map_err(|error| ScriptLanguageError::internal(here, error))?;
            self.retained_record = Some(record.clone());
        }
        Ok(Some(record))
    }

    /// Push `token` back for the next read (donor `unread`).
    pub fn unread(&mut self, token: &ScriptToken) -> Result<(), ScriptLanguageError> {
        let (type_id, subtype, integer_value, float_value) = match token {
            ScriptToken::String { length, .. } => (ScriptTokenType::String.as_u32(), *length, 0, 0.0),
            ScriptToken::Literal { length, .. } => (ScriptTokenType::Literal.as_u32(), *length, 0, 0.0),
            ScriptToken::Number {
                flags,
                integer_value,
                float_value,
                ..
            } => (
                ScriptTokenType::Number.as_u32(),
                *flags as usize,
                *integer_value,
                *float_value,
            ),
            ScriptToken::Name { length, .. } => (ScriptTokenType::Name.as_u32(), *length, 0, 0.0),
            ScriptToken::Punctuation { punctuation, .. } => {
                (ScriptTokenType::Punctuation.as_u32(), *punctuation as usize, 0, 0.0)
            }
            ScriptToken::Primitive { .. } => (ScriptTokenType::Primitive.as_u32(), 0, 0, 0.0),
        };
        let here = self.current_location();
        self.retained
            .clear()
            .map_err(|error| ScriptLanguageError::internal(here.clone(), error))?;
        self.retained
            .write_string(token.text())
            .map_err(|error| ScriptLanguageError::internal(here.clone(), error))?;
        self.retained
            .set_token_type(type_id as i32)
            .map_err(|error| ScriptLanguageError::internal(here.clone(), error))?;
        let subtype_word = self.retained_i32(subtype, "subtype")?;
        self.retained
            .set_subtype(subtype_word)
            .map_err(|error| ScriptLanguageError::internal(here.clone(), error))?;
        self.retained
            .set_integer_value(integer_value)
            .map_err(|error| ScriptLanguageError::internal(here.clone(), error))?;
        self.retained
            .set_float_value(float_value)
            .map_err(|error| ScriptLanguageError::internal(here.clone(), error))?;
        let line = self.retained_i32(token.location().line, "line")?;
        self.retained
            .set_line(line)
            .map_err(|error| ScriptLanguageError::internal(here.clone(), error))?;
        let crossed = self.retained_i32(token.lines_crossed(), "lines crossed")?;
        self.retained
            .set_lines_crossed(crossed)
            .map_err(|error| ScriptLanguageError::internal(here, error))?;
        self.retained_record = Some(ScriptTokenRecord {
            token: token.clone(),
            subtype: subtype_word as u32,
            integer_value,
            float_value,
        });
        let column = self.retained_i32(token.location().column, "column")?;
        self.token_context = SourceTokenContext {
            path: token.location().path.clone(),
            column,
            leading_whitespace: token.leading_whitespace().to_string(),
        };
        self.token_available = true;
        Ok(())
    }

    /// Rewind to the start and clear diagnostics (donor `reset`).
    pub fn reset(&mut self) -> Result<(), ScriptLanguageError> {
        self.offset = 0;
        self.line = 1;
        self.last_offset = 0;
        self.last_line = 1;
        self.whitespace_start = 0;
        self.whitespace_end = 0;
        self.column = 1;
        self.token_available = false;
        self.retained_record = None;
        let here = self.current_location();
        self.retained
            .clear()
            .map_err(|error| ScriptLanguageError::internal(here, error))?;
        self.reported.clear();
        Ok(())
    }

    /// Release lexer resources (donor `dispose`; a no-op over owned text).
    pub fn dispose(&self) {}

    /// Longest-match punctuation at the scan position (donor `readPunctuation`).
    fn read_punctuation(&self) -> Result<Option<(String, u32)>, ScriptLanguageError> {
        let Some(character) = self.peek(0) else {
            return Ok(None);
        };
        let code = character as u32;
        if code >= 256 {
            return Ok(None);
        }
        let mut pointer = self.punctuation_lookup.heads[code as usize];
        while pointer != 0 {
            let link = self.punctuation_lookup.links.get(pointer - 1).ok_or_else(|| {
                ScriptLanguageError::internal(
                    self.current_location(),
                    ClientError::BadUi("script punctuation head does not identify a default record".to_string()),
                )
            })?;
            let spec = self.punctuations.get(link.spec).ok_or_else(|| {
                ScriptLanguageError::internal(
                    self.current_location(),
                    ClientError::BadUi("script punctuation head does not identify a default record".to_string()),
                )
            })?;
            if self.source_starts_with(&spec.text) {
                return Ok(Some((spec.text.clone(), spec.punctuation)));
            }
            pointer = link.next;
        }
        Ok(None)
    }

    /// Skip whitespace and comments (donor `readWhitespace`).
    fn read_whitespace(&mut self) {
        while !self.at_end() {
            let code = self.peek(0).map(|character| character as u32);
            match code {
                Some(value) if value <= 32 || (128..=255).contains(&value) => {
                    self.advance(1);
                    continue;
                }
                _ => {}
            }
            if self.source_starts_with("//") {
                self.advance(2);
                while !self.at_end() && self.peek(0) != Some('\n') {
                    self.advance(1);
                }
                continue;
            }
            if self.source_starts_with("/*") {
                self.advance(2);
                while !self.at_end() && !self.source_starts_with("*/") {
                    self.advance(1);
                }
                if self.source_starts_with("*/") {
                    self.advance(2);
                }
                continue;
            }
            return;
        }
    }

    /// Read a quoted string or literal, returning its value and text length (donor `readQuoted`).
    fn read_quoted(
        &mut self,
        quote: char,
        output: &mut SourceTokenMemory,
    ) -> Result<(String, usize), ScriptLanguageError> {
        let mut value = String::new();
        let mut value_len = 0usize;
        let type_id = if quote == '"' {
            ScriptTokenType::String.as_u32() as i32
        } else {
            ScriptTokenType::Literal.as_u32() as i32
        };
        output.set_token_type(type_id).map_err(|error| self.internal(error))?;
        output
            .set_string_byte(0, quote as u8)
            .map_err(|error| self.internal(error))?;
        self.advance(1);

        loop {
            if value_len + 1 >= self.max_token_length - 2 {
                return Err(self.fail(format!("string longer than MAX_TOKEN = {}", self.max_token_length)));
            }
            let character = self.peek(0);
            if character.is_none() || character == Some('\0') {
                output
                    .set_string_byte(value_len + 1, 0)
                    .map_err(|error| self.internal(error))?;
                return Err(self.fail("missing trailing quote".to_string()));
            }
            if character == Some('\n') {
                output
                    .set_string_byte(value_len + 1, 0)
                    .map_err(|error| self.internal(error))?;
                let shown = value.split('\0').next().unwrap_or("");
                return Err(self.fail(format!("newline inside string {quote}{shown}")));
            }
            if character == Some('\\') && !has_flag(self.flags, LexerFlag::NO_STRING_ESCAPES) {
                let escaped = self.read_escape()?;
                output
                    .set_string_byte(value_len + 1, escaped as u8)
                    .map_err(|error| self.internal(error))?;
                value.push(escaped);
                value_len += 1;
                continue;
            }
            if character == Some(quote) {
                self.advance(1);
                if has_flag(self.flags, LexerFlag::NO_STRING_CONCATENATION) {
                    break;
                }
                let saved_offset = self.offset;
                let saved_line = self.line;
                let saved_column = self.column;
                self.read_whitespace();
                if self.peek(0) == Some(quote) {
                    self.advance(1);
                    continue;
                }
                self.offset = saved_offset;
                self.line = saved_line;
                self.column = saved_column;
                break;
            }
            let here = self.current_location();
            let next = character.ok_or_else(|| {
                ScriptLanguageError::internal(
                    here,
                    ClientError::BadUi("source string character is missing".to_string()),
                )
            })?;
            let byte = u8::try_from(next as u32).map_err(|_| self.byte_error())?;
            output
                .set_string_byte(value_len + 1, byte)
                .map_err(|error| self.internal(error))?;
            value.push(next);
            value_len += 1;
            self.advance(1);
        }

        let text_len = value_len + 2;
        output
            .set_string_byte(value_len + 1, quote as u8)
            .map_err(|error| self.internal(error))?;
        output
            .set_string_byte(value_len + 2, 0)
            .map_err(|error| self.internal(error))?;
        let subtype = self.retained_i32(text_len, "text length")?;
        let here = self.current_location();
        output
            .set_subtype(subtype)
            .map_err(|error| ScriptLanguageError::internal(here, error))?;
        Ok((value, text_len))
    }

    /// Read one backslash escape, returning the escaped character (donor `readEscape`).
    fn read_escape(&mut self) -> Result<char, ScriptLanguageError> {
        self.advance(1);
        let Some(character) = self.peek(0) else {
            self.error("unknown escape char".to_string());
            return Ok('\0');
        };
        match character {
            '\\' => {
                self.advance(1);
                Ok('\\')
            }
            'n' => {
                self.advance(1);
                Ok('\n')
            }
            'r' => {
                self.advance(1);
                Ok('\r')
            }
            't' => {
                self.advance(1);
                Ok('\t')
            }
            'v' => {
                self.advance(1);
                Ok('\u{0b}')
            }
            'b' => {
                self.advance(1);
                Ok('\u{08}')
            }
            'f' => {
                self.advance(1);
                Ok('\u{0c}')
            }
            'a' => {
                self.advance(1);
                Ok('\u{07}')
            }
            '\'' => {
                self.advance(1);
                Ok('\'')
            }
            '"' => {
                self.advance(1);
                Ok('"')
            }
            '?' => {
                self.advance(1);
                Ok('?')
            }
            'x' => {
                self.advance(1);
                let start = self.offset;
                while is_source_escape_hex_digit(self.peek(0)) {
                    self.advance(1);
                }
                let text = self.source_slice(start, self.offset);
                Ok(self.source_escape_hex_value(&text)? as u8 as char)
            }
            other if is_digit(Some(other)) => {
                let start = self.offset;
                while is_digit(self.peek(0)) {
                    self.advance(1);
                }
                let text = self.source_slice(start, self.offset);
                Ok(self.decimal_escape_value(&text)? as u8 as char)
            }
            _ => {
                self.error("unknown escape char".to_string());
                Ok('\0')
            }
        }
    }

    /// Value of a decimal escape (donor `decimalEscapeValue`).
    fn decimal_escape_value(&mut self, text: &str) -> Result<u32, ScriptLanguageError> {
        let mut value: u64 = 0;
        for character in text.chars() {
            value = value * 10 + u64::from(character as u8) - 48;
            if value > 0x7fff_ffff {
                let here = self.current_location();
                return Err(ScriptLanguageError::internal(
                    here,
                    ClientError::BadUi("decimal escape exceeds the source signed-int range".to_string()),
                ));
            }
        }
        self.finish_escape_value(value as u32)
    }

    /// Value of a `\x` escape (donor `sourceEscapeHexValue`).
    fn source_escape_hex_value(&mut self, text: &str) -> Result<u32, ScriptLanguageError> {
        let mut value: u64 = 0;
        for character in text.chars() {
            let code = character as u32;
            let digit = if character.is_ascii_digit() {
                code - 48
            } else if character.is_ascii_uppercase() {
                code - 55
            } else {
                code - 87
            };
            value = value * 16 + u64::from(digit);
            if value > 0x7fff_ffff {
                let here = self.current_location();
                return Err(ScriptLanguageError::internal(
                    here,
                    ClientError::BadUi("hex escape exceeds the source signed-int range".to_string()),
                ));
            }
        }
        self.finish_escape_value(value as u32)
    }

    /// Clamp an escape value to a byte, warning when it overflows (donor `finishEscapeValue`).
    fn finish_escape_value(&mut self, value: u32) -> Result<u32, ScriptLanguageError> {
        if value > 255 {
            self.offset = self.offset.saturating_sub(1);
            self.column = self.column.saturating_sub(1);
            self.warn("too large value in escape character".to_string());
            self.offset += 1;
            self.column += 1;
            return Ok(255);
        }
        Ok(value)
    }

    /// Read a name, returning its value (donor `readName`).
    fn read_name(&mut self, output: &mut SourceTokenMemory) -> Result<String, ScriptLanguageError> {
        let start = self.offset;
        output
            .set_token_type(ScriptTokenType::Name.as_u32() as i32)
            .map_err(|error| self.internal(error))?;
        while is_name_character(self.peek(0)) {
            let here = self.current_location();
            let next = self.peek(0).ok_or_else(|| {
                ScriptLanguageError::internal(here, ClientError::BadUi("source name character is missing".to_string()))
            })?;
            let byte = u8::try_from(next as u32).map_err(|_| self.byte_error())?;
            output
                .set_string_byte(self.offset - start, byte)
                .map_err(|error| self.internal(error))?;
            self.advance(1);
            if self.offset - start >= self.max_token_length {
                return Err(self.fail(format!("name longer than MAX_TOKEN = {}", self.max_token_length)));
            }
        }
        let value = self.source_slice(start, self.offset);
        let len = value.chars().count();
        output.set_string_byte(len, 0).map_err(|error| self.internal(error))?;
        let subtype = self.retained_i32(len, "text length")?;
        let here = self.current_location();
        output
            .set_subtype(subtype)
            .map_err(|error| ScriptLanguageError::internal(here, error))?;
        Ok(value)
    }

    /// Read a number, returning its text, flags, and values (donor `readNumber`).
    fn read_number(&mut self, output: &mut SourceTokenMemory) -> Result<(String, u32, u32, f64), ScriptLanguageError> {
        let start = self.offset;
        let mut flags = 0u32;
        output
            .set_token_type(ScriptTokenType::Number.as_u32() as i32)
            .map_err(|error| self.internal(error))?;

        if self.peek(0) == Some('0') && matches!(self.peek(1), Some('x' | 'X')) {
            self.append_number_byte(output, start)?;
            self.append_number_byte(output, start)?;
            while is_source_number_hex_digit(self.peek(0)) {
                self.append_number_byte(output, start)?;
                self.ensure_token_length(start, "hexadecimal number", 0)?;
            }
            flags |= NumberFlag::HEX;
            output.set_subtype(flags as i32).map_err(|error| self.internal(error))?;
        } else if self.peek(0) == Some('0')
            && matches!(self.peek(1), Some('b' | 'B'))
            && !has_flag(self.flags, LexerFlag::NO_BINARY_NUMBERS)
        {
            self.append_number_byte(output, start)?;
            self.append_number_byte(output, start)?;
            while matches!(self.peek(0), Some('0' | '1')) {
                self.append_number_byte(output, start)?;
                self.ensure_token_length(start, "binary number", 0)?;
            }
            flags |= NumberFlag::BINARY;
            output.set_subtype(flags as i32).map_err(|error| self.internal(error))?;
        } else {
            let mut octal = self.peek(0) == Some('0');
            let mut dots = 0u32;
            loop {
                match self.peek(0) {
                    Some('.') => dots += 1,
                    Some('8' | '9') => octal = false,
                    Some(other) if is_digit(Some(other)) => {}
                    _ => break,
                }
                self.append_number_byte(output, start)?;
                self.ensure_token_length(start, "number", 1)?;
            }
            flags |= if octal { NumberFlag::OCTAL } else { NumberFlag::DECIMAL };
            if dots > 0 {
                flags |= NumberFlag::FLOAT;
            }
            output.set_subtype(flags as i32).map_err(|error| self.internal(error))?;
        }

        let text_end = self.offset;
        for _ in 0..2 {
            match self.peek(0) {
                Some('l' | 'L') if !has_flag(flags, NumberFlag::LONG) => {
                    flags |= NumberFlag::LONG;
                    self.advance(1);
                    output.set_subtype(flags as i32).map_err(|error| self.internal(error))?;
                }
                Some('u' | 'U') if !has_flag(flags, NumberFlag::UNSIGNED) && !has_flag(flags, NumberFlag::FLOAT) => {
                    flags |= NumberFlag::UNSIGNED;
                    self.advance(1);
                    output.set_subtype(flags as i32).map_err(|error| self.internal(error))?;
                }
                _ => {}
            }
        }
        if !has_flag(flags, NumberFlag::FLOAT) {
            flags |= NumberFlag::INTEGER;
        }

        let text = self.source_slice(start, text_end);
        let len = text.chars().count();
        output.set_string_byte(len, 0).map_err(|error| self.internal(error))?;
        let (integer_value, float_value) = self.number_values(&text, flags)?;
        let here = self.current_location();
        output
            .set_integer_value(integer_value)
            .map_err(|error| ScriptLanguageError::internal(here.clone(), error))?;
        output
            .set_float_value(float_value)
            .map_err(|error| ScriptLanguageError::internal(here.clone(), error))?;
        output
            .set_subtype(flags as i32)
            .map_err(|error| ScriptLanguageError::internal(here, error))?;
        Ok((text, flags, integer_value, float_value))
    }

    /// Append the scan character to a number image (donor `readNumber` append).
    fn append_number_byte(&mut self, output: &mut SourceTokenMemory, start: usize) -> Result<(), ScriptLanguageError> {
        let here = self.current_location();
        let next = self.peek(0).ok_or_else(|| {
            ScriptLanguageError::internal(
                here,
                ClientError::BadUi("source number character is missing".to_string()),
            )
        })?;
        let byte = u8::try_from(next as u32).map_err(|_| self.byte_error())?;
        output
            .set_string_byte(self.offset - start, byte)
            .map_err(|error| self.internal(error))?;
        self.advance(1);
        Ok(())
    }

    /// Integer and float values of a number spelling (donor `numberValues`).
    fn number_values(&self, text: &str, flags: u32) -> Result<(u32, f64), ScriptLanguageError> {
        if has_flag(flags, NumberFlag::FLOAT) {
            let chars: Vec<char> = text.chars().collect();
            let mut value = 0.0f64;
            let mut divisor = 0u32;
            let mut index = 0usize;
            while index < chars.len() {
                if chars[index] == '.' {
                    if divisor != 0 {
                        return Ok((0, value));
                    }
                    divisor = 10;
                    index += 1;
                }
                let digit = chars.get(index).map(|next| *next as u32).unwrap_or(0) as f64 - 48.0;
                if divisor != 0 {
                    value += digit / f64::from(divisor);
                    divisor = divisor.wrapping_mul(10);
                } else {
                    value = value * 10.0 + digit;
                }
                index += 1;
            }
            let integer_value = value.trunc();
            if !integer_value.is_finite() || integer_value < 0.0 || integer_value > 4_294_967_295.0 {
                let here = self.current_location();
                return Err(ScriptLanguageError::internal(
                    here,
                    ClientError::BadUi(
                        "NumberValue floating conversion exceeds the source unsigned-long range".to_string(),
                    ),
                ));
            }
            return Ok((integer_value as u32, value));
        }
        let (radix, mut index) = if has_flag(flags, NumberFlag::HEX) {
            (16u32, 2usize)
        } else if has_flag(flags, NumberFlag::OCTAL) {
            (8u32, 1usize)
        } else if has_flag(flags, NumberFlag::BINARY) {
            (2u32, 2usize)
        } else {
            (10u32, 0usize)
        };
        let chars: Vec<char> = text.chars().collect();
        let mut value = 0u32;
        while index < chars.len() {
            let code = chars[index] as u32;
            let digit = if (97..=102).contains(&code) {
                code - 87
            } else if (65..=70).contains(&code) {
                code - 55
            } else {
                code - 48
            };
            value = value.wrapping_mul(radix).wrapping_add(digit);
            index += 1;
        }
        Ok((value, f64::from(value)))
    }

    /// Fail when a number outgrows the token limit (donor `ensureTokenLength`).
    fn ensure_token_length(&mut self, start: usize, label: &str, reserved: usize) -> Result<(), ScriptLanguageError> {
        if self.offset - start >= self.max_token_length - reserved {
            return Err(self.fail(format!("{label} longer than MAX_TOKEN = {}", self.max_token_length)));
        }
        Ok(())
    }

    /// Report a warning diagnostic unless suppressed (donor `warn`).
    fn warn(&mut self, message: String) {
        if has_flag(self.flags, LexerFlag::NO_WARNINGS) {
            return;
        }
        let diagnostic = ScriptDiagnostic {
            severity: DiagnosticSeverity::Warning,
            message,
            location: self.current_location(),
        };
        self.reported.push(diagnostic.clone());
        if let Some(report) = self.report.as_mut() {
            report(&diagnostic);
        }
    }

    /// Report a fatal diagnostic and build its failure (donor `fail`).
    fn fail(&mut self, message: String) -> ScriptLanguageError {
        let diagnostic = ScriptDiagnostic {
            severity: DiagnosticSeverity::Error,
            message,
            location: self.current_location(),
        };
        if !has_flag(self.flags, LexerFlag::NO_ERRORS) {
            self.reported.push(diagnostic.clone());
            if let Some(report) = self.report.as_mut() {
                report(&diagnostic);
            }
        }
        self.failures.push(diagnostic.clone());
        ScriptLanguageError::reported(diagnostic, &self.reported)
    }

    /// Wrap a retained-image failure as a non-reported language error.
    fn internal(&self, error: ClientError) -> ScriptLanguageError {
        ScriptLanguageError::internal(self.current_location(), error)
    }

    /// Non-reported failure for a character outside the source bytes.
    fn byte_error(&self) -> ScriptLanguageError {
        ScriptLanguageError::internal(
            self.current_location(),
            ClientError::BadUi("token_t string requires source bytes".to_string()),
        )
    }

    /// Whether the scan reached the end of the source or a NUL (donor `atEnd`).
    fn at_end(&self) -> bool {
        self.offset >= self.chars.len() || self.peek(0) == Some('\0')
    }

    /// Scan character `ahead` positions forward (donor `peek`).
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.offset.saturating_add(ahead)).copied()
    }

    /// Advance the scan, tracking lines and columns (donor `advance`).
    fn advance(&mut self, count: usize) {
        for _ in 0..count {
            if self.offset >= self.chars.len() {
                break;
            }
            let newline = self.peek(0) == Some('\n');
            self.offset += 1;
            if newline {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }
    }

    /// Retained source pointer of the scan position (donor `sourcePointer`).
    fn source_pointer(&self) -> Result<u32, ScriptLanguageError> {
        let offset = u32::try_from(self.offset).map_err(|_| {
            ScriptLanguageError::internal(
                self.current_location(),
                ClientError::BadUi("script token pointer exceeds its source allocation".to_string()),
            )
        })?;
        Ok(SOURCE_SCRIPT_BASE.saturating_add(offset))
    }

    /// Convert a coordinate for the retained image.
    fn retained_i32(&self, value: usize, what: &'static str) -> Result<i32, ScriptLanguageError> {
        i32::try_from(value).map_err(|_| {
            ScriptLanguageError::internal(
                self.current_location(),
                ClientError::BadUi(format!("script token {what} exceeds its retained range")),
            )
        })
    }

    /// Current literal byte, or 0 past the end (donor `readLiteralInto` byte).
    fn literal_byte(&self) -> u32 {
        if self.offset >= self.chars.len() {
            return 0;
        }
        self.peek(0).map(|next| next as u32).unwrap_or(0)
    }

    /// Consume one literal byte, unconditionally moving past the end (donor `readLiteralInto` take).
    fn take(&mut self) -> u32 {
        let value = self.literal_byte();
        self.offset += 1;
        self.column += 1;
        value
    }

    /// Whether the source starts with `text` at the scan position (donor `sourceStartsWith`).
    fn source_starts_with(&self, text: &str) -> bool {
        let mut tail = self.chars.get(self.offset..).unwrap_or(&[]).iter();
        text.chars().all(|want| tail.next() == Some(&want))
    }

    /// Source text between char offsets (donor `sourceSlice`).
    fn source_slice(&self, start: usize, end: usize) -> String {
        let len = self.chars.len();
        let start = start.min(len);
        let end = end.min(len);
        if start >= end {
            return String::new();
        }
        self.chars[start..end].iter().collect()
    }

    /// Checkpoint the lexer (donor `captureSaveState`).
    pub fn capture_save_state(&self) -> Result<ScriptLexerSaveState, ClientError> {
        Ok(ScriptLexerSaveState {
            source: self.chars.iter().collect(),
            path: self.path.clone(),
            flags: self.flags,
            max_token_length: self.max_token_length,
            offset: self.offset,
            line: self.line,
            last_offset: self.last_offset,
            last_line: self.last_line,
            whitespace_start: self.whitespace_start,
            whitespace_end: self.whitespace_end,
            column: self.column,
            token_available: self.token_available,
            retained: self.retained.capture_save_state()?,
            retained_record: self.retained_record.clone(),
            token_context: self.token_context.clone(),
            punctuations: self.punctuations.clone(),
            reported: self.reported.clone(),
        })
    }

    /// Restore a checkpoint (donor `restoreSaveState`).
    pub fn restore_save_state(state: &ScriptLexerSaveState, report: DiagnosticReport) -> Result<Self, ClientError> {
        if state.max_token_length < 4 {
            return Err(ClientError::BadUi("script.lexer: invalid token limit".to_string()));
        }
        let punctuation_lookup = punctuation_table(&state.punctuations)?;
        let mut lexer = ScriptLexer {
            chars: state.source.chars().collect(),
            path: state.path.clone(),
            flags: state.flags,
            max_token_length: state.max_token_length,
            report,
            reported: state.reported.clone(),
            failures: Vec::new(),
            offset: state.offset,
            line: state.line,
            last_offset: state.last_offset,
            last_line: state.last_line,
            whitespace_start: state.whitespace_start,
            whitespace_end: state.whitespace_end,
            column: state.column,
            token_available: state.token_available,
            retained: SourceTokenMemory::new(),
            retained_record: state.retained_record.clone(),
            token_context: state.token_context.clone(),
            punctuations: state.punctuations.clone(),
            punctuation_lookup,
        };
        lexer.retained.restore_save_state(&state.retained, false)?;
        Ok(lexer)
    }
}

/// Checkpoint of a lexer (donor `captureSaveState` record).
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptLexerSaveState {
    /// Full source text.
    pub source: String,
    /// Diagnostic path.
    pub path: String,
    /// Active script flags.
    pub flags: u32,
    /// Maximum raw token length.
    pub max_token_length: usize,
    /// Scan offset in characters.
    pub offset: usize,
    /// Scan line.
    pub line: usize,
    /// Offset at the last token start.
    pub last_offset: usize,
    /// Line at the last token start.
    pub last_line: usize,
    /// Recorded whitespace replay start.
    pub whitespace_start: usize,
    /// Recorded whitespace replay end.
    pub whitespace_end: usize,
    /// Scan column.
    pub column: usize,
    /// Whether a retained token is pushed back.
    pub token_available: bool,
    /// Retained token image.
    pub retained: SourceTokenSaveState,
    /// Canonical record of the retained token, if any.
    pub retained_record: Option<ScriptTokenRecord>,
    /// Decode context of the retained token.
    pub token_context: SourceTokenContext,
    /// Active punctuation records.
    pub punctuations: Vec<ScriptPunctuation>,
    /// Diagnostics reported so far.
    pub reported: Vec<ScriptDiagnostic>,
}

#[cfg(test)]
mod tests {
    use super::super::token_memory::SourceTokenMemory;
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn lex(source: &str) -> Result<Vec<ScriptToken>, ScriptLanguageError> {
        lex_with(source, ScriptLexerOptions::default())
    }

    fn lex_with(source: &str, options: ScriptLexerOptions) -> Result<Vec<ScriptToken>, ScriptLanguageError> {
        let mut lexer = ScriptLexer::new(source, "test", options).expect("lexer builds");
        let mut tokens = Vec::new();
        while let Some(token) = lexer.next()? {
            tokens.push(token);
        }
        assert!(lexer.end_of_script());
        Ok(tokens)
    }

    fn kinds(tokens: &[ScriptToken]) -> Vec<&str> {
        tokens.iter().map(ScriptToken::kind).collect()
    }

    fn texts(tokens: &[ScriptToken]) -> Vec<&str> {
        tokens.iter().map(ScriptToken::text).collect()
    }

    fn positions(tokens: &[ScriptToken]) -> Vec<(usize, usize)> {
        tokens
            .iter()
            .map(|token| (token.location().line, token.location().column))
            .collect()
    }

    #[test]
    fn token_vector_names_numbers_strings_punctuation() {
        let tokens = lex("name1 42 \"a\\tb\" += {\n'c' 0x10 010 0b11 1.5 .5 3u 4l << >>= // tail\n/* block */ ;")
            .expect("lexes");
        assert_eq!(
            kinds(&tokens),
            vec![
                "name",
                "number",
                "string",
                "punctuation",
                "punctuation",
                "literal",
                "number",
                "number",
                "number",
                "number",
                "number",
                "number",
                "number",
                "punctuation",
                "punctuation",
                "punctuation"
            ]
        );
        assert_eq!(
            texts(&tokens),
            vec![
                "name1", "42", "\"a\tb\"", "+=", "{", "'c'", "0x10", "010", "0b11", "1.5", ".5", "3", "4", "<<", ">>=",
                ";"
            ]
        );
        assert_eq!(
            positions(&tokens),
            vec![
                (1, 1),
                (1, 7),
                (1, 10),
                (1, 17),
                (1, 20),
                (2, 1),
                (2, 5),
                (2, 10),
                (2, 14),
                (2, 19),
                (2, 23),
                (2, 26),
                (2, 29),
                (2, 32),
                (2, 35),
                (3, 13)
            ]
        );
        assert_eq!(tokens[0].value(), Some("name1"));
        assert_eq!(tokens[2].value(), Some("a\tb"));
        assert_eq!(tokens[5].value(), Some("c"));
        assert_eq!(tokens[5].leading_whitespace(), "\n");
        assert_eq!(tokens[5].lines_crossed(), 1);
        assert_eq!(tokens[15].leading_whitespace(), " // tail\n/* block */ ");
        assert_eq!(tokens[15].lines_crossed(), 1);
        match &tokens[6] {
            ScriptToken::Number {
                flags,
                integer_value,
                float_value,
                ..
            } => {
                assert_eq!(*flags, NumberFlag::HEX | NumberFlag::INTEGER);
                assert_eq!(*integer_value, 16);
                assert_eq!(*float_value, 16.0);
            }
            other => panic!("expected hex number, got {other:?}"),
        }
        match &tokens[7] {
            ScriptToken::Number { integer_value, .. } => assert_eq!(*integer_value, 8),
            other => panic!("expected octal number, got {other:?}"),
        }
        match &tokens[8] {
            ScriptToken::Number {
                flags, integer_value, ..
            } => {
                assert_eq!(*flags, NumberFlag::BINARY | NumberFlag::INTEGER);
                assert_eq!(*integer_value, 3);
            }
            other => panic!("expected binary number, got {other:?}"),
        }
        match &tokens[9] {
            ScriptToken::Number {
                flags,
                integer_value,
                float_value,
                ..
            } => {
                assert_eq!(*flags, NumberFlag::DECIMAL | NumberFlag::FLOAT);
                assert_eq!(*integer_value, 1);
                assert_eq!(*float_value, 1.5);
            }
            other => panic!("expected float, got {other:?}"),
        }
        match &tokens[10] {
            ScriptToken::Number {
                float_value,
                integer_value,
                ..
            } => {
                assert_eq!(*integer_value, 0);
                assert_eq!(*float_value, 0.5);
            }
            other => panic!("expected dot float, got {other:?}"),
        }
        match &tokens[11] {
            ScriptToken::Number { flags, .. } => {
                assert_eq!(*flags, NumberFlag::DECIMAL | NumberFlag::UNSIGNED | NumberFlag::INTEGER)
            }
            other => panic!("expected unsigned, got {other:?}"),
        }
        match &tokens[12] {
            ScriptToken::Number { flags, .. } => {
                assert_eq!(*flags, NumberFlag::DECIMAL | NumberFlag::LONG | NumberFlag::INTEGER)
            }
            other => panic!("expected long, got {other:?}"),
        }
        match &tokens[13] {
            ScriptToken::Punctuation { punctuation, .. } => {
                assert_eq!(*punctuation, Punctuation::LeftShift.as_u32())
            }
            other => panic!("expected punct, got {other:?}"),
        }
        match &tokens[14] {
            ScriptToken::Punctuation { punctuation, .. } => {
                assert_eq!(*punctuation, Punctuation::RightShiftAssign.as_u32())
            }
            other => panic!("expected punct, got {other:?}"),
        }
    }

    #[test]
    fn positions_track_lines_and_trivia() {
        let tokens = lex("a\n\nb").expect("lexes");
        assert_eq!(positions(&tokens), vec![(1, 1), (3, 1)]);
        assert_eq!(tokens[1].leading_whitespace(), "\n\n");
        assert_eq!(tokens[1].lines_crossed(), 2);

        let tokens = lex("// c\nx").expect("lexes");
        assert_eq!(positions(&tokens), vec![(2, 1)]);
        assert_eq!(tokens[0].leading_whitespace(), "// c\n");
        assert_eq!(tokens[0].lines_crossed(), 1);

        let tokens = lex("/* a\nb */ y").expect("lexes");
        assert_eq!(positions(&tokens), vec![(2, 6)]);
        assert_eq!(tokens[0].leading_whitespace(), "/* a\nb */ ");
        assert_eq!(tokens[0].lines_crossed(), 1);

        let tokens = lex("/* never closed").expect("lexes");
        assert!(tokens.is_empty());
        let mut lexer = ScriptLexer::new("a\0b", "test", ScriptLexerOptions::default()).expect("lexer builds");
        assert_eq!(lexer.next().expect("lexes").expect("token").text(), "a");
        assert_eq!(lexer.next().expect("lexes"), None);
        assert!(!lexer.end_of_script());
    }

    #[test]
    fn escapes_decode_and_unknown_escapes_report_without_advancing() {
        let mut lexer =
            ScriptLexer::new("\"\\x41\\66\\q\"", "test", ScriptLexerOptions::default()).expect("lexer builds");
        let token = lexer.next().expect("lexes").expect("token");
        assert_eq!(token.text(), "\"AB\0q\"");
        assert_eq!(token.value(), Some("AB\0q"));
        assert_eq!(lexer.next().expect("lexes"), None);
        assert_eq!(lexer.diagnostics().len(), 1);
        assert_eq!(lexer.diagnostics()[0].message, "unknown escape char");
        assert_eq!(lexer.diagnostics()[0].severity, DiagnosticSeverity::Error);
    }

    #[test]
    fn adjacent_strings_concatenate_unless_disabled() {
        let tokens = lex("\"a\" /* gap */ \"b\"").expect("lexes");
        assert_eq!(texts(&tokens), vec!["\"ab\""]);
        assert_eq!(tokens[0].value(), Some("ab"));

        let options = ScriptLexerOptions {
            flags: LexerFlag::NO_STRING_CONCATENATION,
            ..ScriptLexerOptions::default()
        };
        let tokens = lex_with("\"a\" \"b\"", options).expect("lexes");
        assert_eq!(texts(&tokens), vec!["\"a\"", "\"b\""]);

        let options = ScriptLexerOptions {
            flags: LexerFlag::NO_STRING_ESCAPES,
            ..ScriptLexerOptions::default()
        };
        let tokens = lex_with("\"a\\nb\"", options).expect("lexes");
        assert_eq!(tokens[0].value(), Some("a\\nb"));
    }

    #[test]
    fn failures_report_exact_diagnostics_and_are_source_failures() {
        let mut lexer = ScriptLexer::new("\"abc", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let error = lexer.next().expect_err("missing quote fails");
        assert_eq!(error.diagnostic.message, "missing trailing quote");
        assert_eq!(
            error.diagnostic.location,
            SourceLocation {
                path: "p".to_string(),
                line: 1,
                column: 5
            }
        );
        assert_eq!(error.to_string(), "p:1:5: missing trailing quote");
        assert!(lexer.is_source_failure(&error));
        assert_eq!(lexer.diagnostics().len(), 1);

        let mut lexer = ScriptLexer::new("\"a\nb\"", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let error = lexer.next().expect_err("newline fails");
        assert_eq!(error.diagnostic.message, "newline inside string \"a");
        assert_eq!(error.diagnostic.location.column, 3);
        assert!(lexer.is_source_failure(&error));

        let mut lexer = ScriptLexer::new("@", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let error = lexer.next().expect_err("bad char fails");
        assert_eq!(error.diagnostic.message, "can't read token");
        assert!(lexer.is_source_failure(&error));

        let options = ScriptLexerOptions {
            max_token_length: 6,
            ..ScriptLexerOptions::default()
        };
        let mut lexer = ScriptLexer::new("abcdefg", "p", options).expect("lexer builds");
        let error = lexer.next().expect_err("long name fails");
        assert_eq!(error.diagnostic.message, "name longer than MAX_TOKEN = 6");

        let options = ScriptLexerOptions {
            flags: LexerFlag::NO_ERRORS,
            ..ScriptLexerOptions::default()
        };
        let mut lexer = ScriptLexer::new("@", "p", options).expect("lexer builds");
        let error = lexer.next().expect_err("still fails");
        assert!(lexer.is_source_failure(&error));
        assert!(lexer.diagnostics().is_empty());
        assert!(error.diagnostics.is_empty());
    }

    #[test]
    fn number_quirks_match_donor() {
        let tokens = lex("0x1G").expect("lexes");
        assert_eq!(texts(&tokens), vec!["0x1", "G"]);
        let tokens = lex("0b12").expect("lexes");
        assert_eq!(texts(&tokens), vec!["0b1", "2"]);
        let tokens = lex("018").expect("lexes");
        match &tokens[0] {
            ScriptToken::Number {
                flags, integer_value, ..
            } => {
                assert_eq!(*flags, NumberFlag::DECIMAL | NumberFlag::INTEGER);
                assert_eq!(*integer_value, 18);
            }
            other => panic!("expected decimal, got {other:?}"),
        }
        let tokens = lex("1.2.3").expect("lexes");
        match &tokens[0] {
            ScriptToken::Number {
                integer_value,
                float_value,
                ..
            } => {
                assert_eq!(*integer_value, 0);
                assert!((float_value - 1.2).abs() < 1e-12);
            }
            other => panic!("expected dotted float, got {other:?}"),
        }
        let tokens = lex("3lu 4ul").expect("lexes");
        for token in &tokens {
            match token {
                ScriptToken::Number { flags, .. } => assert_eq!(
                    *flags,
                    NumberFlag::DECIMAL | NumberFlag::LONG | NumberFlag::UNSIGNED | NumberFlag::INTEGER
                ),
                other => panic!("expected suffixed, got {other:?}"),
            }
        }
        let tokens = lex("1.5u").expect("lexes");
        assert_eq!(texts(&tokens), vec!["1.5", "u"]);

        let mut lexer = ScriptLexer::new("1.", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let error = lexer.next().expect_err("trailing dot fails");
        assert!(!lexer.is_source_failure(&error));
        assert!(error
            .diagnostic
            .message
            .contains("exceeds the source unsigned-long range"));
    }

    #[test]
    fn longest_match_punctuation_and_lookup() {
        let tokens = lex(">>= >> > == =").expect("lexes");
        let ids: Vec<u32> = tokens
            .iter()
            .map(|token| match token {
                ScriptToken::Punctuation { punctuation, .. } => *punctuation,
                other => panic!("expected punct, got {other:?}"),
            })
            .collect();
        assert_eq!(
            ids,
            vec![
                Punctuation::RightShiftAssign.as_u32(),
                Punctuation::RightShift.as_u32(),
                Punctuation::Greater.as_u32(),
                Punctuation::Equal.as_u32(),
                Punctuation::Assign.as_u32()
            ]
        );
        let lexer = ScriptLexer::new("", "p", ScriptLexerOptions::default()).expect("lexer builds");
        assert_eq!(lexer.punctuation_from_num(1), ">>=");
        assert_eq!(lexer.punctuation_from_num(99), "unkown punctuation");
    }

    #[test]
    fn expect_check_skip_and_pushback() {
        let mut lexer = ScriptLexer::new("a b c", "p", ScriptLexerOptions::default()).expect("lexer builds");
        assert!(!lexer.check_token_string("x").expect("checks"));
        assert!(lexer.check_token_string("a").expect("checks"));
        assert!(lexer.expect_token_string("b").expect("expects"));
        assert!(!lexer.expect_token_string("z").expect("expects"));
        assert_eq!(lexer.diagnostics().last().expect("diag").message, "expected z, found c");

        let mut lexer = ScriptLexer::new("name 7", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let mut output = SourceTokenMemory::new();
        assert!(lexer
            .check_token_type(ScriptTokenType::Name.as_u32(), 0, &mut output)
            .expect("checks"));
        assert_eq!(output.string().expect("string"), "name");
        assert!(!lexer
            .check_token_type(ScriptTokenType::Name.as_u32(), 0, &mut output)
            .expect("checks"));
        assert!(lexer.skip_until_string("7").expect("skips"));
        assert!(!lexer.skip_until_string("7").expect("skips"));

        let mut lexer = ScriptLexer::new("a b", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let first = lexer.next().expect("lexes").expect("token");
        lexer.unread_last();
        let replayed = lexer.next().expect("lexes").expect("token");
        assert_eq!(first, replayed);
        let second = lexer.next().expect("lexes").expect("token");
        lexer.unread(&first).expect("unreads");
        let pushed = lexer.next().expect("lexes").expect("token");
        assert_eq!(pushed, first);
        assert_eq!(second.text(), "b");

        let mut lexer =
            ScriptLexer::new("a /* skip */ target", "p", ScriptLexerOptions::default()).expect("lexer builds");
        assert!(lexer.script_skip_to("target"));
        assert_eq!(lexer.next().expect("lexes").expect("token").text(), "target");
        assert!(!lexer.script_skip_to("missing"));
    }

    #[test]
    fn signed_reads_and_expect_type_edges() {
        let mut lexer = ScriptLexer::new("- 1.5", "p", ScriptLexerOptions::default()).expect("lexer builds");
        assert_eq!(lexer.read_signed_float().expect("reads"), -1.5);
        let mut lexer = ScriptLexer::new("- 42", "p", ScriptLexerOptions::default()).expect("lexer builds");
        assert_eq!(lexer.read_signed_int().expect("reads"), -42);
        let mut lexer = ScriptLexer::new("7", "p", ScriptLexerOptions::default()).expect("lexer builds");
        assert_eq!(lexer.read_signed_int().expect("reads"), 7);

        let mut lexer = ScriptLexer::new("a", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let mut output = SourceTokenMemory::new();
        assert!(!lexer
            .expect_token_type(ScriptTokenType::Number.as_u32(), 0, &mut output)
            .expect("expects"));
        assert_eq!(
            lexer.diagnostics().last().expect("diag").message,
            "expected a number, found a"
        );

        let mut lexer = ScriptLexer::new("1.5", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let mut output = SourceTokenMemory::new();
        assert!(lexer
            .expect_token_type(
                ScriptTokenType::Number.as_u32(),
                NumberFlag::DECIMAL | NumberFlag::INTEGER,
                &mut output
            )
            .expect("expects"));
        assert_eq!(
            lexer.diagnostics().last().expect("diag").message,
            "expected decimal integer, found 1.5"
        );

        let mut lexer = ScriptLexer::new("1.5", "p", ScriptLexerOptions::default()).expect("lexer builds");
        assert_eq!(lexer.read_signed_int().expect("reads"), 1);
        let mut lexer = ScriptLexer::new("- 1.5", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let error = lexer.read_signed_int().expect_err("integer read of float fails");
        assert!(!lexer.is_source_failure(&error));

        let mut lexer = ScriptLexer::new("a", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let mut output = SourceTokenMemory::new();
        let error = lexer
            .expect_token_type(9, 0, &mut output)
            .expect_err("bad type id errors");
        assert!(!lexer.is_source_failure(&error));

        let mut lexer = ScriptLexer::new(";", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let mut output = SourceTokenMemory::new();
        let error = lexer
            .expect_token_type(
                ScriptTokenType::Punctuation.as_u32(),
                Punctuation::Comma.as_u32(),
                &mut output,
            )
            .expect_err("wrong punct id errors");
        assert!(error.diagnostic.message.contains("punctuation struct"));
    }

    #[test]
    fn literal_reader_and_whitespace_replay() {
        let mut lexer = ScriptLexer::new("'a'", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let mut output = SourceTokenMemory::new();
        assert!(lexer.read_literal_into(&mut output).expect("reads"));
        assert_eq!(output.string().expect("string"), "'a'");
        assert_eq!(output.subtype().expect("subtype"), 97);

        let mut lexer = ScriptLexer::new("'ab'", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let mut output = SourceTokenMemory::new();
        assert!(lexer.read_literal_into(&mut output).expect("reads"));
        assert_eq!(
            lexer.diagnostics().last().expect("diag").message,
            "too many characters in literal, ignored"
        );

        let mut lexer = ScriptLexer::new("'", "p", ScriptLexerOptions::default()).expect("lexer builds");
        let mut output = SourceTokenMemory::new();
        assert!(!lexer.read_literal_into(&mut output).expect("reads"));
        assert_eq!(
            lexer.diagnostics().last().expect("diag").message,
            "end of file before trailing '"
        );

        let mut lexer = ScriptLexer::new("  a", "p", ScriptLexerOptions::default()).expect("lexer builds");
        lexer.next().expect("lexes").expect("token");
        assert_eq!(lexer.next_whitespace_char().expect("replays"), 32);
        assert_eq!(lexer.next_whitespace_char().expect("replays"), 32);
        assert_eq!(lexer.next_whitespace_char().expect("replays"), 0);
    }

    #[test]
    fn reset_primitives_custom_punctuations_and_save_round_trip() {
        let mut lexer = ScriptLexer::new("a b", "p", ScriptLexerOptions::default()).expect("lexer builds");
        lexer.next().expect("lexes").expect("token");
        lexer.reset().expect("resets");
        assert!(lexer.diagnostics().is_empty());
        assert_eq!(lexer.next().expect("lexes").expect("token").text(), "a");
        lexer.dispose();

        let options = ScriptLexerOptions {
            flags: LexerFlag::PRIMITIVE,
            ..ScriptLexerOptions::default()
        };
        let tokens = lex_with("ab cd", options).expect("lexes");
        assert_eq!(kinds(&tokens), vec!["primitive", "primitive"]);
        assert_eq!(texts(&tokens), vec!["ab", "cd"]);

        let mut lexer = ScriptLexer::new("??", "p", ScriptLexerOptions::default()).expect("lexer builds");
        lexer
            .set_punctuations(Some(&[ScriptPunctuation {
                text: "??".to_string(),
                punctuation: 99,
            }]))
            .expect("sets");
        let token = lexer.next().expect("lexes").expect("token");
        match token {
            ScriptToken::Punctuation { punctuation, .. } => assert_eq!(punctuation, 99),
            other => panic!("expected custom punct, got {other:?}"),
        }
        lexer.set_punctuations(None).expect("restores");

        let mut lexer = ScriptLexer::new("a b", "p", ScriptLexerOptions::default()).expect("lexer builds");
        lexer.next().expect("lexes").expect("token");
        let state = lexer.capture_save_state().expect("captures");
        let mut restored = ScriptLexer::restore_save_state(&state, None).expect("restores");
        assert_eq!(restored.capture_save_state().expect("captures"), state);
        assert_eq!(restored.next().expect("lexes").expect("token").text(), "b");
        assert_eq!(restored.next().expect("lexes"), None);
    }

    #[test]
    fn report_callback_flags_helpers_and_limits() {
        let seen: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let pushed = Rc::clone(&seen);
        let options = ScriptLexerOptions {
            report: Some(Box::new(move |diagnostic: &ScriptDiagnostic| {
                pushed.borrow_mut().push(diagnostic.message.clone());
            })),
            ..ScriptLexerOptions::default()
        };
        let mut lexer = ScriptLexer::new("@", "p", options).expect("lexer builds");
        lexer.next().expect_err("fails");
        assert_eq!(*seen.borrow(), vec!["can't read token".to_string()]);

        let mut lexer = ScriptLexer::new("", "p", ScriptLexerOptions::default()).expect("lexer builds");
        assert_eq!(lexer.get_script_flags(), LexerFlag::NONE);
        lexer.set_script_flags(LexerFlag::NO_WARNINGS);
        assert_eq!(lexer.get_script_flags(), LexerFlag::NO_WARNINGS);
        assert_eq!(lexer.num_lines_crossed(), 0);

        assert_eq!(script_token_type_name(3), "number");
        assert_eq!(script_token_type_name(9), "");
        assert_eq!(
            script_number_subtype_name(NumberFlag::DECIMAL | NumberFlag::LONG | NumberFlag::INTEGER).expect("names"),
            "decimal long integer"
        );
        assert!(script_number_subtype_name(0).is_err());
        assert_eq!(strip_double_quotes("\"ab\"").expect("strips"), "ab");
        assert_eq!(strip_single_quotes("'a'").expect("strips"), "a");
        assert_eq!(strip_double_quotes("\"\"").expect("strips"), "");
        assert!(strip_double_quotes("\"").is_err());
        assert_eq!(strip_double_quotes("\"ab\0cd\"").expect("strips"), "ab");

        let options = ScriptLexerOptions {
            max_token_length: 3,
            ..ScriptLexerOptions::default()
        };
        assert!(ScriptLexer::new("a", "p", options).is_err());
    }
}
