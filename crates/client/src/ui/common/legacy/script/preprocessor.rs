//! Menu script preprocessor.
//!
//! Ported from the TypeScript donor's
//! `src/ui/common/legacy/script/preprocessor.ts` (itself translated from Quake
//! III Arena `botlib/l_precomp.c`). Handles `#include`, `#define`/`#undef`,
//! `#if`/`#ifdef`/`#ifndef`/`#elif`/`#else`/`#endif`, `#eval`/`#evalfloat`,
//! `$evalint`/`$evalfloat`, macro expansion with `#` stringizing and `##`
//! token merging, and the builtin macros `__LINE__`, `__FILE__`, `__DATE__`
//! and `__TIME__`.
//!
//! # Sync port notes
//!
//! * The donor is sync-or-async (`AsyncIncludeResolver`, `nextRecordAsync`).
//!   This port is sync only: [`IncludeResolver::resolve`] is a plain sync
//!   callback and there is no `next_async`. Pass includes through the
//!   injected resolver instead of awaiting them.
//! * The donor threads caller-owned `ScriptMemory` arenas, `PrecompMemory`,
//!   `SourceRecord`, `SourceTokenMemory` and save/restore snapshots through
//!   every structure. Those siblings do not exist in this crate yet, so the
//!   engine below owns plain Rust structures (`HashMap` macros, a `Vec`
//!   indent stack, a `VecDeque` pushback queue) and exposes no
//!   `captureSaveState`/`restoreSaveState`, `memory` option, or
//!   `SourceTokenMemory` surface. `dispose`/`dispose_record_only` are kept as
//!   documented no-ops because `Drop` owns the memory.
//! * Token, location and diagnostic types mirror the donor `lexer.ts` exports
//!   locally until the sibling `lexer` task lands (see assumed imports in the
//!   module docs of `super`). The internal byte lexer below replicates the
//!   donor `ScriptLexer` token flow (comments, escapes, concatenation,
//!   number bases and suffixes, longest-match punctuation).
//! * `ScriptGlobalDefines::remove` marks the entry removed and later
//!   traversal skips it. The donor frees without unlinking so later traversal
//!   reaches a dangling id; the skip is a deliberate deviation from that trap.
//! * [`ScriptPreprocessorOptions::globals`] takes an owned clone of
//!   [`ScriptGlobalDefines`]; the donor borrows the live object. Snapshot
//!   with [`ScriptGlobalDefines::snapshot`] or clone the value before opening.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::ClientError;

/// Maximum raw token length (`MAX_TOKEN`), shared with the donor lexer.
pub const DEFAULT_SCRIPT_TOKEN_LIMIT: usize = 1024;

/// Maximum `#define` parameters, matching the donor's private limit.
pub const MAX_DEFINE_PARAMETERS: usize = 128;

/// Number of buckets in the donor-compatible define hash table.
const DEFINE_HASH_BUCKETS: usize = 1024;

/// Raw script token type ids (`ScriptTokenType` in the donor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ScriptTokenType {
    /// Quake Primitive token.
    Primitive = 0,
    /// Double-quoted string.
    String = 1,
    /// Single-quoted literal.
    Literal = 2,
    /// Number.
    Number = 3,
    /// Name.
    Name = 4,
    /// Punctuation.
    Punctuation = 5,
}

impl ScriptTokenType {
    /// Numeric id of this token type.
    #[must_use]
    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

/// Donor `scriptTokenTypeName`: prose name, or `""` for unknown ids.
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

/// Number subtype bit flags (`NumberFlag` in the donor).
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
    /// Floating point value.
    pub const FLOAT: u32 = 0x0800;
    /// Integer value.
    pub const INTEGER: u32 = 0x1000;
    /// Long suffix.
    pub const LONG: u32 = 0x2000;
    /// Unsigned suffix.
    pub const UNSIGNED: u32 = 0x4000;
}

/// Donor `scriptNumberSubtypeName`: prose description of a number subtype.
pub fn script_number_subtype_name(subtype: u32) -> Result<String, ClientError> {
    let mut name: Option<&str> = None;
    if subtype & NumberFlag::DECIMAL != 0 {
        name = Some("decimal");
    }
    if subtype & NumberFlag::HEX != 0 {
        name = Some("hex");
    }
    if subtype & NumberFlag::OCTAL != 0 {
        name = Some("octal");
    }
    if subtype & NumberFlag::BINARY != 0 {
        name = Some("binary");
    }
    let Some(base) = name else {
        return Err(ClientError::BadUi(
            "expected numeric subtype diagnostic reads an uninitialized source string".to_string(),
        ));
    };
    let mut text = base.to_string();
    if subtype & NumberFlag::LONG != 0 {
        text.push_str(" long");
    }
    if subtype & NumberFlag::UNSIGNED != 0 {
        text.push_str(" unsigned");
    }
    if subtype & NumberFlag::FLOAT != 0 {
        text.push_str(" float");
    }
    if subtype & NumberFlag::INTEGER != 0 {
        text.push_str(" integer");
    }
    Ok(text)
}

/// Punctuation ids (`Punctuation` in the donor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
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
    /// Numeric id of this punctuation.
    #[must_use]
    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// Look up a punctuation by numeric id.
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

/// Default punctuation table in donor order (longest match wins per prefix).
fn default_punctuations() -> Vec<ScriptPunctuation> {
    [
        (">>=", Punctuation::RightShiftAssign),
        ("<<=", Punctuation::LeftShiftAssign),
        ("...", Punctuation::Parameters),
        ("##", Punctuation::PreprocessorMerge),
        ("&&", Punctuation::LogicalAnd),
        ("||", Punctuation::LogicalOr),
        (">=", Punctuation::GreaterOrEqual),
        ("<=", Punctuation::LessOrEqual),
        ("==", Punctuation::Equal),
        ("!=", Punctuation::NotEqual),
        ("*=", Punctuation::MultiplyAssign),
        ("/=", Punctuation::DivideAssign),
        ("%=", Punctuation::ModuloAssign),
        ("+=", Punctuation::AddAssign),
        ("-=", Punctuation::SubtractAssign),
        ("++", Punctuation::Increment),
        ("--", Punctuation::Decrement),
        ("&=", Punctuation::BinaryAndAssign),
        ("|=", Punctuation::BinaryOrAssign),
        ("^=", Punctuation::BinaryXorAssign),
        (">>", Punctuation::RightShift),
        ("<<", Punctuation::LeftShift),
        ("->", Punctuation::PointerReference),
        ("::", Punctuation::Scope),
        (".*", Punctuation::PointerToMember),
        ("*", Punctuation::Multiply),
        ("/", Punctuation::Divide),
        ("%", Punctuation::Modulo),
        ("+", Punctuation::Add),
        ("-", Punctuation::Subtract),
        ("=", Punctuation::Assign),
        ("&", Punctuation::BinaryAnd),
        ("|", Punctuation::BinaryOr),
        ("^", Punctuation::BinaryXor),
        ("~", Punctuation::BinaryNot),
        ("!", Punctuation::LogicalNot),
        (">", Punctuation::Greater),
        ("<", Punctuation::Less),
        (".", Punctuation::Reference),
        (",", Punctuation::Comma),
        (";", Punctuation::Semicolon),
        (":", Punctuation::Colon),
        ("?", Punctuation::Question),
        ("(", Punctuation::ParenthesisOpen),
        (")", Punctuation::ParenthesisClose),
        ("{", Punctuation::BraceOpen),
        ("}", Punctuation::BraceClose),
        ("[", Punctuation::BracketOpen),
        ("]", Punctuation::BracketClose),
        ("\\", Punctuation::Backslash),
        ("#", Punctuation::Preprocessor),
        ("$", Punctuation::Dollar),
    ]
    .into_iter()
    .map(|(text, punctuation)| ScriptPunctuation {
        text: text.to_string(),
        punctuation: punctuation.as_u32(),
    })
    .collect()
}

/// Punctuation override entry (`ScriptPunctuation` in the donor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptPunctuation {
    /// Punctuation text.
    pub text: String,
    /// Numeric punctuation id.
    pub punctuation: u32,
}

/// Source position (`SourceLocation` in the donor).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceLocation {
    /// Source path.
    pub path: String,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub column: usize,
}

/// Diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticSeverity {
    /// Warning diagnostic.
    Warning,
    /// Error diagnostic.
    Error,
}

/// Recorded warning or error (`ScriptDiagnostic` in the donor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptDiagnostic {
    /// Severity.
    pub severity: DiagnosticSeverity,
    /// Message text (donor messages, including quirks, preserved verbatim).
    pub message: String,
    /// Location where the diagnostic was recorded.
    pub location: SourceLocation,
}

/// Lexed script token (`ScriptToken` in the donor).
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptToken {
    /// Double-quoted string; `text` keeps the quotes, `value` is unescaped.
    String {
        /// Full token text including quotes.
        text: String,
        /// Unescaped inner value.
        value: String,
        /// Length of `text`.
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
        /// Length of `text`.
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
        /// Floating point value.
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
        /// Length of `text`.
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

    /// Numeric [`ScriptTokenType`] id.
    #[must_use]
    pub fn type_id(&self) -> u32 {
        match self {
            ScriptToken::Primitive { .. } => ScriptTokenType::Primitive.as_u32(),
            ScriptToken::String { .. } => ScriptTokenType::String.as_u32(),
            ScriptToken::Literal { .. } => ScriptTokenType::Literal.as_u32(),
            ScriptToken::Number { .. } => ScriptTokenType::Number.as_u32(),
            ScriptToken::Name { .. } => ScriptTokenType::Name.as_u32(),
            ScriptToken::Punctuation { .. } => ScriptTokenType::Punctuation.as_u32(),
        }
    }

    /// Full token text.
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

    /// Value text for every kind except numbers.
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

    /// Whether this is a punctuation with the given id.
    #[must_use]
    pub fn is_punctuation(&self, punctuation: Punctuation) -> bool {
        matches!(self, ScriptToken::Punctuation { punctuation: id, .. } if *id == punctuation.as_u32())
    }

    /// Clear leading whitespace, matching stored define copies in the donor.
    fn clear_whitespace(&mut self) {
        match self {
            ScriptToken::String {
                leading_whitespace,
                lines_crossed,
                ..
            }
            | ScriptToken::Literal {
                leading_whitespace,
                lines_crossed,
                ..
            }
            | ScriptToken::Number {
                leading_whitespace,
                lines_crossed,
                ..
            }
            | ScriptToken::Name {
                leading_whitespace,
                lines_crossed,
                ..
            }
            | ScriptToken::Punctuation {
                leading_whitespace,
                lines_crossed,
                ..
            }
            | ScriptToken::Primitive {
                leading_whitespace,
                lines_crossed,
                ..
            } => {
                leading_whitespace.clear();
                *lines_crossed = 0;
            }
        }
    }
}

/// Token plus numeric record fields (`ScriptTokenRecord` in the donor).
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptTokenRecord {
    /// The token.
    pub token: ScriptToken,
    /// Subtype: length for names/strings, flags for numbers, id for punctuation.
    pub subtype: u32,
    /// Integer value for numbers.
    pub integer_value: u32,
    /// Floating point value for numbers.
    pub float_value: f64,
}

/// Root or included script source (`ScriptSource` in the donor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptSource {
    /// Canonical path of the source.
    pub path: String,
    /// Source text.
    pub text: String,
}

/// Reader position (`ScriptSourcePosition` in the donor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptSourcePosition {
    /// Root source path (never the current include).
    pub filename: String,
    /// Current 1-based line.
    pub line: usize,
}

/// Include request flavor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IncludeKind {
    /// `#include "path"`.
    Quoted,
    /// `#include <path>`.
    System,
}

/// Include request (`IncludeRequest` in the donor).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncludeRequest {
    /// Request flavor.
    pub kind: IncludeKind,
    /// Path of the source containing the directive.
    pub from_path: String,
    /// Requested path (unquoted value, or joined `<...>` pieces).
    pub requested_path: String,
    /// Engine include path, omitted when empty (quoted only).
    pub include_path: Option<String>,
}

/// Sync include resolver (`IncludeResolver` in the donor).
///
/// The donor also exports `AsyncIncludeResolver`, whose promise reads collapse
/// into this sync callback in the Rust port; there is no async read path.
pub trait IncludeResolver {
    /// Resolve an include request, or `Ok(None)` when the file is missing.
    fn resolve(&mut self, request: &IncludeRequest) -> Result<Option<ScriptSource>, ClientError>;
}

impl<F> IncludeResolver for F
where
    F: FnMut(&IncludeRequest) -> Result<Option<ScriptSource>, ClientError>,
{
    fn resolve(&mut self, request: &IncludeRequest) -> Result<Option<ScriptSource>, ClientError> {
        self(request)
    }
}

/// Resolver that reports every include as missing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullIncludeResolver;

impl IncludeResolver for NullIncludeResolver {
    fn resolve(&mut self, _request: &IncludeRequest) -> Result<Option<ScriptSource>, ClientError> {
        Ok(None)
    }
}

/// Calendar time for `__DATE__`/`__TIME__` (donor `now?: () => Date`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptDateTime {
    /// Full year.
    pub year: i32,
    /// Month of year, 1-12.
    pub month: u32,
    /// Day of month, 1-31.
    pub day: u32,
    /// Hour of day, 0-23.
    pub hour: u32,
    /// Minute of hour, 0-59.
    pub minute: u32,
    /// Second of minute, 0-59.
    pub second: u32,
}

/// Current UTC calendar time, used when no `now` callback is installed.
fn system_date_time() -> ScriptDateTime {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let days = seconds.div_euclid(86_400);
    let second_of_day = seconds.rem_euclid(86_400);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = (if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    }) as u32;
    ScriptDateTime {
        year: (if month <= 2 { year + 1 } else { year }) as i32,
        month,
        day,
        hour: (second_of_day / 3600) as u32,
        minute: ((second_of_day % 3600) / 60) as u32,
        second: (second_of_day % 60) as u32,
    }
}

/// Minimal big unsigned integer (little-endian `u64` limbs) for `%1.2f`/`%.6f`
/// emulation of huge magnitudes, matching the donor's `BigInt` path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BigUint(Vec<u64>);

impl BigUint {
    fn from_u128(mut value: u128) -> BigUint {
        let mut limbs = Vec::new();
        while value != 0 {
            limbs.push((value & 0xffff_ffff_ffff_ffff) as u64);
            value >>= 64;
        }
        BigUint(limbs)
    }

    fn is_zero(&self) -> bool {
        self.0.iter().all(|limb| *limb == 0)
    }

    fn normalize(&mut self) {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
    }

    fn shl_assign(&mut self, bits: u32) {
        if self.is_zero() {
            return;
        }
        let words = (bits / 64) as usize;
        let small = bits % 64;
        if small != 0 {
            let mut carry = 0u64;
            for limb in self.0.iter_mut() {
                let next = *limb >> (64 - small);
                *limb = (*limb << small) | carry;
                carry = next;
            }
            if carry != 0 {
                self.0.push(carry);
            }
        }
        if words != 0 {
            let mut shifted = vec![0u64; words];
            shifted.append(&mut self.0);
            self.0 = shifted;
        }
    }

    /// Divide by a small divisor, returning the remainder.
    fn divmod_small(&mut self, divisor: u64) -> u64 {
        let mut remainder: u128 = 0;
        for limb in self.0.iter_mut().rev() {
            let current = (remainder << 64) | u128::from(*limb);
            *limb = (current / u128::from(divisor)) as u64;
            remainder = current % u128::from(divisor);
        }
        self.normalize();
        remainder as u64
    }

    fn to_decimal_string(&self) -> String {
        if self.is_zero() {
            return "0".to_string();
        }
        let mut working = self.clone();
        let mut groups: Vec<u32> = Vec::new();
        while !working.is_zero() {
            groups.push(working.divmod_small(1_000_000_000) as u32);
        }
        let mut text = String::new();
        let mut groups = groups.into_iter().rev();
        if let Some(high) = groups.next() {
            text.push_str(&high.to_string());
            for group in groups {
                text.push_str(&format!("{group:09}"));
            }
        }
        text
    }
}

/// Donor `evaluationDecimal`: `sprintf("%1.2f", magnitude)` with binary64
/// round-half-even, plus the 6-digit debug variant. `magnitude` must be
/// non-negative (callers pass the absolute value).
fn evaluation_decimal(magnitude: f64, precision: u32) -> String {
    let bits = magnitude.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1 << 52) - 1);
    if exponent == 0x7ff {
        return if fraction == 0 {
            "inf".to_string()
        } else {
            "nan".to_string()
        };
    }
    let significand: u128 = if exponent == 0 {
        u128::from(fraction)
    } else {
        u128::from(fraction | (1 << 52))
    };
    let shift: i32 = if exponent == 0 { -1074 } else { exponent - 1075 };
    let scale = 10u128.pow(precision);
    if shift >= 0 {
        let mut rounded = BigUint::from_u128(significand * scale);
        rounded.shl_assign(shift as u32);
        let scale_small = scale as u64;
        let fraction_part = rounded.divmod_small(scale_small);
        return format!(
            "{}.{:0width$}",
            rounded.to_decimal_string(),
            fraction_part,
            width = precision as usize
        );
    }
    let scaled: u128 = significand * scale;
    let shift_bits = (-shift) as u32;
    // `scaled` is below 2^74, so shifts of 77+ always round to zero.
    let rounded: u128 = if shift_bits >= 77 {
        0
    } else {
        let divisor: u128 = 1 << shift_bits;
        let lower = scaled / divisor;
        let remainder = scaled % divisor;
        if remainder * 2 > divisor || (remainder * 2 == divisor && !lower.is_multiple_of(2)) {
            lower + 1
        } else {
            lower
        }
    };
    format!(
        "{}.{:0width$}",
        rounded / scale,
        rounded % scale,
        width = precision as usize
    )
}

/// Donor-compatible define hash: signed bytes times `(119 + index)`, i32
/// wrapping, then folded into [`DEFINE_HASH_BUCKETS`] buckets.
fn define_bucket(name: &str) -> usize {
    let mut hash: i32 = 0;
    for (index, byte) in name.bytes().enumerate() {
        if byte == 0 {
            break;
        }
        let signed = if byte < 128 { byte as i32 } else { byte as i32 - 256 };
        hash = hash.wrapping_add(signed.wrapping_mul(119 + index as i32));
    }
    (hash ^ (hash >> 10) ^ (hash >> 20)) as usize & (DEFINE_HASH_BUCKETS - 1)
}

/// Donor `asciiFold(sourceCommandText(...))` comparison input for include
/// recursion detection: truncate at NUL, reject non-Latin-1, lowercase ASCII.
fn folded_command_text(input: &str) -> Result<String, String> {
    let truncated = input.split('\0').next().unwrap_or("");
    let mut folded = String::with_capacity(truncated.len());
    for character in truncated.chars() {
        if character as u32 > 255 {
            return Err("Command text requires source bytes".to_string());
        }
        folded.push(character.to_ascii_lowercase());
    }
    Ok(folded.chars().take(99_999).collect())
}

/// Stored token: record fields plus the donor `unsupported` profile marker.
///
/// Tokens produced by `#eval`/`#evalfloat` and `#` stringizing carry an
/// `unsupported` reason; reading them through [`ScriptSourceReader::next`]
/// fails exactly like the donor's `currentRecord` check.
#[derive(Debug, Clone, PartialEq)]
struct StoredToken {
    token: ScriptToken,
    subtype: u32,
    integer_value: u32,
    float_value: f64,
    unsupported: Option<String>,
}

impl StoredToken {
    fn record(&self) -> ScriptTokenRecord {
        ScriptTokenRecord {
            token: self.token.clone(),
            subtype: self.subtype,
            integer_value: self.integer_value,
            float_value: self.float_value,
        }
    }

    fn from_record(record: &ScriptTokenRecord) -> StoredToken {
        StoredToken {
            token: record.token.clone(),
            subtype: record.subtype,
            integer_value: record.integer_value,
            float_value: record.float_value,
            unsupported: None,
        }
    }

    fn cleared(mut self) -> StoredToken {
        self.token.clear_whitespace();
        self
    }
}

/// Internal engine failure: recorded source failure, internal range-style
/// error, external resolver error, or the donor `SourceReadFalse` signal.
#[derive(Debug, Clone, PartialEq)]
enum EngineError {
    Fail(ScriptDiagnostic),
    Internal(String),
    External(ClientError),
    ReadFalse,
}

impl EngineError {
    fn client_error(&self) -> ClientError {
        match self {
            EngineError::Fail(diagnostic) => ClientError::BadUi(rendered_diagnostic(diagnostic)),
            EngineError::Internal(message) | EngineError::External(ClientError::BadUi(message)) => {
                ClientError::BadUi(message.clone())
            }
            EngineError::External(error) => error.clone(),
            EngineError::ReadFalse => ClientError::BadUi("source read produced no token".to_string()),
        }
    }
}

fn rendered_diagnostic(diagnostic: &ScriptDiagnostic) -> String {
    format!(
        "{}:{}:{}: {}",
        diagnostic.location.path, diagnostic.location.line, diagnostic.location.column, diagnostic.message
    )
}

/// Byte-oriented port of the donor `ScriptLexer` token flow.
struct RawLexer {
    bytes: Vec<u8>,
    path: String,
    offset: usize,
    line: usize,
    column: usize,
    punctuations: Vec<ScriptPunctuation>,
    max_token_length: usize,
}

/// One lexer step: emitted diagnostics plus the token or failure flag.
struct LexerStep {
    emitted: Vec<ScriptDiagnostic>,
    token: Option<StoredToken>,
    failed: bool,
}

fn byte_char(byte: u8) -> char {
    byte as char
}

fn bytes_text(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| byte_char(*byte)).collect()
}

impl RawLexer {
    fn new(text: &str, path: &str) -> RawLexer {
        RawLexer {
            bytes: text.bytes().collect(),
            path: path.to_string(),
            offset: 0,
            line: 1,
            column: 1,
            punctuations: default_punctuations(),
            max_token_length: DEFAULT_SCRIPT_TOKEN_LIMIT,
        }
    }

    fn current_location(&self) -> SourceLocation {
        SourceLocation {
            path: self.path.clone(),
            line: self.line,
            column: self.column,
        }
    }

    fn at_end(&self) -> bool {
        self.offset >= self.bytes.len() || self.bytes.get(self.offset) == Some(&0)
    }

    fn peek(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.offset + ahead).copied()
    }

    fn advance(&mut self, count: usize) {
        for _ in 0..count {
            if self.offset >= self.bytes.len() {
                return;
            }
            let byte = self.bytes[self.offset];
            self.offset += 1;
            if byte == b'\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }
    }

    fn starts_with(&self, text: &str) -> bool {
        let bytes = text.as_bytes();
        self.offset + bytes.len() <= self.bytes.len() && self.bytes[self.offset..self.offset + bytes.len()] == *bytes
    }

    fn slice(&self, start: usize, end: usize) -> String {
        bytes_text(&self.bytes[start..end])
    }

    fn next_token(&mut self) -> LexerStep {
        let mut emitted = Vec::new();
        let whitespace_start = self.offset;
        let line_before = self.line;
        self.read_whitespace();
        if self.at_end() {
            return LexerStep {
                emitted,
                token: None,
                failed: false,
            };
        }
        let leading_whitespace = self.slice(whitespace_start, self.offset);
        let lines_crossed = self.line - line_before;
        let location = self.current_location();
        let byte = self.peek(0);
        let next = self.peek(1);
        if byte == Some(b'"') || byte == Some(b'\'') {
            let quote = byte_char(byte.unwrap_or(b'"'));
            return self.read_quoted(quote, &location, &leading_whitespace, lines_crossed, &mut emitted);
        }
        let is_digit = byte.is_some_and(|value| value.is_ascii_digit());
        let dot_digit = byte == Some(b'.') && next.is_some_and(|value| value.is_ascii_digit());
        if is_digit || dot_digit {
            return self.read_number(&location, &leading_whitespace, lines_crossed, &mut emitted);
        }
        if byte.is_some_and(|value| is_name_start(&value)) {
            return self.name_with_limit(&location, &leading_whitespace, lines_crossed, &mut emitted);
        }
        // Longest-match punctuation, matching the donor's per-head ordering.
        let mut best: Option<&ScriptPunctuation> = None;
        for spec in &self.punctuations {
            if self.starts_with(&spec.text)
                && best.is_none_or(|current: &ScriptPunctuation| spec.text.len() > current.text.len())
            {
                best = Some(spec);
            }
        }
        let Some(spec) = best.cloned() else {
            let diagnostic = ScriptDiagnostic {
                severity: DiagnosticSeverity::Error,
                message: "can't read token".to_string(),
                location: self.current_location(),
            };
            emitted.push(diagnostic);
            return LexerStep {
                emitted,
                token: None,
                failed: true,
            };
        };
        self.advance(spec.text.len());
        let token = ScriptToken::Punctuation {
            text: spec.text.clone(),
            value: spec.text.clone(),
            punctuation: spec.punctuation,
            location,
            leading_whitespace,
            lines_crossed,
        };
        LexerStep {
            emitted,
            token: Some(StoredToken {
                token,
                subtype: spec.punctuation,
                integer_value: 0,
                float_value: 0.0,
                unsupported: None,
            }),
            failed: false,
        }
    }

    fn read_whitespace(&mut self) {
        while !self.at_end() {
            let Some(byte) = self.peek(0) else { return };
            if byte <= 32 || byte >= 128 {
                self.advance(1);
                continue;
            }
            if self.starts_with("//") {
                self.advance(2);
                while !self.at_end() && self.peek(0) != Some(b'\n') {
                    self.advance(1);
                }
                continue;
            }
            if self.starts_with("/*") {
                self.advance(2);
                while !self.at_end() && !self.starts_with("*/") {
                    self.advance(1);
                }
                if self.starts_with("*/") {
                    self.advance(2);
                }
                continue;
            }
            return;
        }
    }

    fn read_quoted(
        &mut self,
        quote: char,
        location: &SourceLocation,
        leading_whitespace: &str,
        lines_crossed: usize,
        emitted: &mut Vec<ScriptDiagnostic>,
    ) -> LexerStep {
        let fail = |lexer: &mut RawLexer, emitted: &mut Vec<ScriptDiagnostic>, message: String| LexerStep {
            emitted: {
                emitted.push(ScriptDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    message,
                    location: lexer.current_location(),
                });
                std::mem::take(emitted)
            },
            token: None,
            failed: true,
        };
        let mut value = String::new();
        self.advance(1);
        loop {
            if value.len() + 1 >= self.max_token_length - 2 {
                let max = self.max_token_length;
                return fail(self, emitted, format!("string longer than MAX_TOKEN = {max}"));
            }
            let Some(byte) = self.peek(0) else {
                return fail(self, emitted, "missing trailing quote".to_string());
            };
            if byte == 0 {
                return fail(self, emitted, "missing trailing quote".to_string());
            }
            if byte == b'\n' {
                let shown = value.split('\0').next().unwrap_or("").to_string();
                return fail(self, emitted, format!("newline inside string {quote}{shown}"));
            }
            if byte == b'\\' {
                match self.read_escape(emitted) {
                    Ok(escaped) => {
                        value.push(escaped);
                        continue;
                    }
                    Err(message) => {
                        return LexerStep {
                            emitted: std::mem::take(emitted),
                            token: None,
                            failed: true,
                        }
                        .with_internal(message);
                    }
                }
            }
            if byte_char(byte) == quote {
                self.advance(1);
                let saved = (self.offset, self.line, self.column);
                self.read_whitespace();
                if self.peek(0).is_some_and(|next| byte_char(next) == quote) {
                    self.advance(1);
                    continue;
                }
                self.offset = saved.0;
                self.line = saved.1;
                self.column = saved.2;
                break;
            }
            value.push(byte_char(byte));
            self.advance(1);
        }
        let text = format!("{quote}{value}{quote}");
        let length = text.len();
        let token = if quote == '"' {
            ScriptToken::String {
                text,
                value,
                length,
                location: location.clone(),
                leading_whitespace: leading_whitespace.to_string(),
                lines_crossed,
            }
        } else {
            ScriptToken::Literal {
                text,
                value,
                length,
                location: location.clone(),
                leading_whitespace: leading_whitespace.to_string(),
                lines_crossed,
            }
        };
        LexerStep {
            emitted: std::mem::take(emitted),
            token: Some(StoredToken {
                token,
                subtype: length as u32,
                integer_value: 0,
                float_value: 0.0,
                unsupported: None,
            }),
            failed: false,
        }
    }

    fn read_escape(&mut self, emitted: &mut Vec<ScriptDiagnostic>) -> Result<char, String> {
        self.advance(1);
        let Some(byte) = self.peek(0) else {
            emitted.push(ScriptDiagnostic {
                severity: DiagnosticSeverity::Error,
                message: "unknown escape char".to_string(),
                location: self.current_location(),
            });
            return Ok('\0');
        };
        let simple = match byte {
            b'\\' => Some('\\'),
            b'n' => Some('\n'),
            b'r' => Some('\r'),
            b't' => Some('\t'),
            b'v' => Some('\x0b'),
            b'b' => Some('\x08'),
            b'f' => Some('\x0c'),
            b'a' => Some('\x07'),
            b'\'' => Some('\''),
            b'"' => Some('"'),
            b'?' => Some('?'),
            _ => None,
        };
        if let Some(escaped) = simple {
            self.advance(1);
            return Ok(escaped);
        }
        if byte == b'x' {
            self.advance(1);
            let start = self.offset;
            while self.peek(0).is_some_and(|value| value.is_ascii_alphanumeric()) {
                self.advance(1);
            }
            let text = self.slice(start, self.offset);
            let mut value: i64 = 0;
            for digit in text.bytes() {
                let place = if digit.is_ascii_digit() {
                    i64::from(digit - b'0')
                } else if digit.is_ascii_uppercase() {
                    i64::from(digit - b'A') + 10
                } else {
                    i64::from(digit.to_ascii_lowercase() - b'a') + 10
                };
                value = value * 16 + place;
                if value > 2_147_483_647 {
                    return Err("hex escape exceeds the source signed-int range".to_string());
                }
            }
            return Ok(self.finish_escape_value(value, emitted));
        }
        if byte.is_ascii_digit() {
            let start = self.offset;
            while self.peek(0).is_some_and(|value| value.is_ascii_digit()) {
                self.advance(1);
            }
            let text = self.slice(start, self.offset);
            let mut value: i64 = 0;
            for digit in text.bytes() {
                value = value * 10 + i64::from(digit - b'0');
                if value > 2_147_483_647 {
                    return Err("decimal escape exceeds the source signed-int range".to_string());
                }
            }
            return Ok(self.finish_escape_value(value, emitted));
        }
        emitted.push(ScriptDiagnostic {
            severity: DiagnosticSeverity::Error,
            message: "unknown escape char".to_string(),
            location: self.current_location(),
        });
        Ok('\0')
    }

    fn finish_escape_value(&mut self, value: i64, emitted: &mut Vec<ScriptDiagnostic>) -> char {
        if value > 255 {
            emitted.push(ScriptDiagnostic {
                severity: DiagnosticSeverity::Warning,
                message: "too large value in escape character".to_string(),
                location: self.current_location(),
            });
            return 255 as char;
        }
        byte_char(value as u8)
    }

    fn name_with_limit(
        &mut self,
        location: &SourceLocation,
        leading_whitespace: &str,
        lines_crossed: usize,
        emitted: &mut Vec<ScriptDiagnostic>,
    ) -> LexerStep {
        let start = self.offset;
        while self
            .peek(0)
            .is_some_and(|byte| is_name_start(&byte) || byte.is_ascii_digit())
        {
            self.advance(1);
            if self.offset - start >= self.max_token_length {
                let max = self.max_token_length;
                emitted.push(ScriptDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!("name longer than MAX_TOKEN = {max}"),
                    location: self.current_location(),
                });
                return LexerStep {
                    emitted: std::mem::take(emitted),
                    token: None,
                    failed: true,
                };
            }
        }
        let value = self.slice(start, self.offset);
        let length = value.len();
        LexerStep {
            emitted: std::mem::take(emitted),
            token: Some(StoredToken {
                token: ScriptToken::Name {
                    text: value.clone(),
                    value,
                    length,
                    location: location.clone(),
                    leading_whitespace: leading_whitespace.to_string(),
                    lines_crossed,
                },
                subtype: length as u32,
                integer_value: 0,
                float_value: 0.0,
                unsupported: None,
            }),
            failed: false,
        }
    }

    fn read_number(
        &mut self,
        location: &SourceLocation,
        leading_whitespace: &str,
        lines_crossed: usize,
        emitted: &mut Vec<ScriptDiagnostic>,
    ) -> LexerStep {
        let start = self.offset;
        let mut flags = 0u32;
        let fail_length = |lexer: &mut RawLexer, label: &str| ScriptDiagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!("{label} longer than MAX_TOKEN = {}", lexer.max_token_length),
            location: lexer.current_location(),
        };
        if self.peek(0) == Some(b'0') && matches!(self.peek(1), Some(b'x') | Some(b'X')) {
            self.advance(2);
            while self.peek(0).is_some_and(is_hex_digit) {
                self.advance(1);
                if self.offset - start >= self.max_token_length {
                    let diagnostic = fail_length(self, "hexadecimal number");
                    emitted.push(diagnostic);
                    return LexerStep {
                        emitted: std::mem::take(emitted),
                        token: None,
                        failed: true,
                    };
                }
            }
            flags |= NumberFlag::HEX;
        } else if self.peek(0) == Some(b'0') && matches!(self.peek(1), Some(b'b') | Some(b'B')) {
            self.advance(2);
            while matches!(self.peek(0), Some(b'0' | b'1')) {
                self.advance(1);
                if self.offset - start >= self.max_token_length {
                    let diagnostic = fail_length(self, "binary number");
                    emitted.push(diagnostic);
                    return LexerStep {
                        emitted: std::mem::take(emitted),
                        token: None,
                        failed: true,
                    };
                }
            }
            flags |= NumberFlag::BINARY;
        } else {
            let mut octal = self.peek(0) == Some(b'0');
            let mut dots = 0;
            loop {
                match self.peek(0) {
                    Some(b'.') => dots += 1,
                    Some(b'8' | b'9') => octal = false,
                    Some(byte) if byte.is_ascii_digit() => {}
                    _ => break,
                }
                self.advance(1);
                if self.offset - start >= self.max_token_length - 1 {
                    let diagnostic = fail_length(self, "number");
                    emitted.push(diagnostic);
                    return LexerStep {
                        emitted: std::mem::take(emitted),
                        token: None,
                        failed: true,
                    };
                }
            }
            flags |= if octal { NumberFlag::OCTAL } else { NumberFlag::DECIMAL };
            if dots > 0 {
                flags |= NumberFlag::FLOAT;
            }
        }
        let text_end = self.offset;
        for _ in 0..2 {
            match self.peek(0) {
                Some(b'l' | b'L') if flags & NumberFlag::LONG == 0 => {
                    flags |= NumberFlag::LONG;
                    self.advance(1);
                }
                Some(b'u' | b'U') if flags & NumberFlag::UNSIGNED == 0 && flags & NumberFlag::FLOAT == 0 => {
                    flags |= NumberFlag::UNSIGNED;
                    self.advance(1);
                }
                _ => {}
            }
        }
        if flags & NumberFlag::FLOAT == 0 {
            flags |= NumberFlag::INTEGER;
        }
        let text = self.slice(start, text_end);
        let values = match number_values(&text, flags) {
            Ok(values) => values,
            Err(message) => {
                return LexerStep {
                    emitted: std::mem::take(emitted),
                    token: None,
                    failed: true,
                }
                .with_internal(message);
            }
        };
        LexerStep {
            emitted: std::mem::take(emitted),
            token: Some(StoredToken {
                token: ScriptToken::Number {
                    text,
                    flags,
                    integer_value: values.0,
                    float_value: values.1,
                    location: location.clone(),
                    leading_whitespace: leading_whitespace.to_string(),
                    lines_crossed,
                },
                subtype: flags,
                integer_value: values.0,
                float_value: values.1,
                unsupported: None,
            }),
            failed: false,
        }
    }
}

impl LexerStep {
    fn with_internal(mut self, message: String) -> LexerStep {
        self.emitted.push(ScriptDiagnostic {
            severity: DiagnosticSeverity::Error,
            message: format!("internal: {message}"),
            location: SourceLocation {
                path: String::new(),
                line: 0,
                column: 0,
            },
        });
        self
    }
}

fn is_name_start(byte: &u8) -> bool {
    byte.is_ascii_alphabetic() || *byte == b'_'
}

fn is_hex_digit(byte: u8) -> bool {
    // Donor quirk: uppercase B-F are not hex digits here.
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f' | b'A')
}

/// Donor `numberValues`: wrapping u32 accumulation, float branch included.
fn number_values(text: &str, flags: u32) -> Result<(u32, f64), String> {
    if flags & NumberFlag::FLOAT != 0 {
        let bytes = text.as_bytes();
        let mut value = 0.0f64;
        let mut divisor = 0u32;
        let mut index = 0usize;
        while index < bytes.len() {
            if bytes[index] == b'.' {
                if divisor != 0 {
                    return Ok((0, value));
                }
                divisor = 10;
                index += 1;
            }
            // A trailing dot consumes the NUL in the donor's cleared token
            // buffer, contributing a -48 digit.
            let digit = bytes.get(index).map_or(0i32, |byte| i32::from(*byte)) - 48;
            if divisor != 0 {
                value += f64::from(digit) / f64::from(divisor);
                divisor = divisor.wrapping_mul(10);
            } else {
                value = value * 10.0 + f64::from(digit);
            }
            index += 1;
        }
        let integer_value = value.trunc();
        if !integer_value.is_finite() || integer_value < 0.0 || integer_value > 4_294_967_295.0 {
            return Err("NumberValue floating conversion exceeds the source unsigned-long range".to_string());
        }
        return Ok((integer_value as u32, value));
    }
    let (radix, index) = if flags & NumberFlag::HEX != 0 {
        (16u32, 2usize)
    } else if flags & NumberFlag::OCTAL != 0 {
        (8u32, 1usize)
    } else if flags & NumberFlag::BINARY != 0 {
        (2u32, 2usize)
    } else {
        (10u32, 0usize)
    };
    let mut index = index;
    let mut value = 0u32;
    let bytes = text.as_bytes();
    while index < bytes.len() {
        let code = bytes[index];
        let digit = if code.is_ascii_digit() {
            u32::from(code - b'0')
        } else if code.is_ascii_lowercase() {
            u32::from(code - b'a') + 10
        } else {
            u32::from(code - b'A') + 10
        };
        value = value.wrapping_mul(radix).wrapping_add(digit);
        index += 1;
    }
    Ok((value, f64::from(value)))
}

/// Evaluated `#if`/`#eval` value (donor `EvalValue`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct EvalValue {
    integer_value: i32,
    float_value: f64,
}

/// Donor `signedExpressionValue`: range-check into the signed-long range.
fn signed_expression_value(value: i64) -> Result<i32, String> {
    if !(-2_147_483_648..=2_147_483_647).contains(&value) {
        return Err("preprocessor operation exceeds the source signed-long range".to_string());
    }
    Ok(value as i32)
}

/// Donor `debugEvaluationValue`.
fn debug_evaluation_value(value: &EvalValue, integer_mode: bool) -> String {
    if integer_mode {
        return value.integer_value.to_string();
    }
    let number = value.float_value;
    let sign = if number < 0.0 || (number == 0.0 && number.is_sign_negative()) {
        "-"
    } else {
        ""
    };
    format!("{sign}{}", evaluation_decimal(number.abs(), 6))
}

/// Events the expression parser collects for the engine to apply in order.
enum ExprEvent {
    DebugLine(String),
    IncrementReport,
}

/// Expression failure: recorded source failure or internal range error.
enum ExprError {
    Fail { message: String, location: SourceLocation },
    Internal(String),
}

/// Donor `ExpressionParser`: precedence-climbing `#if` reduction.
struct ExpressionParser<'a> {
    tokens: &'a [StoredToken],
    integer_mode: bool,
    defined: &'a dyn Fn(&str) -> bool,
    fallback: SourceLocation,
    events: Vec<ExprEvent>,
}

struct ExprOperator {
    punctuation: u32,
    text: String,
    location: SourceLocation,
    depth: i32,
    priority: i32,
}

fn expression_precedence(punctuation: u32) -> Option<i32> {
    match Punctuation::from_u32(punctuation)? {
        Punctuation::Multiply | Punctuation::Divide | Punctuation::Modulo => Some(15),
        Punctuation::Add | Punctuation::Subtract => Some(14),
        Punctuation::RightShift | Punctuation::LeftShift => Some(13),
        Punctuation::GreaterOrEqual | Punctuation::LessOrEqual | Punctuation::Greater | Punctuation::Less => Some(12),
        Punctuation::Equal | Punctuation::NotEqual => Some(11),
        Punctuation::BinaryAnd => Some(10),
        Punctuation::BinaryXor => Some(9),
        Punctuation::BinaryOr => Some(8),
        Punctuation::LogicalAnd => Some(7),
        Punctuation::LogicalOr => Some(6),
        Punctuation::Question | Punctuation::Colon => Some(5),
        _ => None,
    }
}

impl<'a> ExpressionParser<'a> {
    fn evaluate(mut self) -> (Result<EvalValue, ExprError>, Vec<ExprEvent>) {
        let result = self.run();
        (result, self.events)
    }

    fn run(&mut self) -> Result<EvalValue, ExprError> {
        let mut values: Vec<EvalValue> = Vec::new();
        let mut operators: Vec<ExprOperator> = Vec::new();
        let mut depth = 0i32;
        let mut last_was_value = false;
        let mut negative = false;
        let mut index = 0usize;
        while index < self.tokens.len() {
            let stored = &self.tokens[index];
            let location = stored.token.location().clone();
            match &stored.token {
                ScriptToken::Name { value, text, .. } => {
                    if last_was_value || negative {
                        return Err(self.fail("syntax error in #if/#elif", &location));
                    }
                    if value != "defined" {
                        return Err(self.fail(&format!("undefined name {text} in #if/#elif"), &location));
                    }
                    index += 1;
                    let Some(next) = self.tokens.get(index) else {
                        return Err(ExprError::Internal(
                            "defined reads a null source token before checking its name".to_string(),
                        ));
                    };
                    let brace = next.token.text() == "(";
                    if brace {
                        index += 1;
                    }
                    let defined_name = match self.tokens.get(index) {
                        Some(candidate) => match &candidate.token {
                            ScriptToken::Name { value, .. } => Some(value.clone()),
                            _ => None,
                        },
                        None => None,
                    };
                    let Some(defined_name) = defined_name else {
                        return Err(self.fail("defined without name in #if/#elif", &location));
                    };
                    let scalar = f64::from(u8::from((self.defined)(&defined_name)));
                    if values.len() >= 64 {
                        return Err(self.fail("out of value space\n", &self.fallback.clone()));
                    }
                    values.push(EvalValue {
                        integer_value: scalar as i32,
                        float_value: scalar,
                    });
                    last_was_value = true;
                    if brace {
                        index += 1;
                        let closed = self
                            .tokens
                            .get(index)
                            .is_some_and(|candidate| candidate.token.text() == ")");
                        if !closed {
                            return Err(self.fail("defined without ) in #if/#elif", &location));
                        }
                    }
                }
                ScriptToken::Number { .. } => {
                    if last_was_value {
                        return Err(self.fail("syntax error in #if/#elif", &location));
                    }
                    let wrapped = stored.integer_value as i32;
                    let integer_value = if negative {
                        signed_expression_value(-(i64::from(wrapped))).map_err(ExprError::Internal)?
                    } else {
                        wrapped
                    };
                    if values.len() >= 64 {
                        return Err(self.fail("out of value space\n", &self.fallback.clone()));
                    }
                    values.push(EvalValue {
                        integer_value,
                        float_value: if negative {
                            -stored.float_value
                        } else {
                            stored.float_value
                        },
                    });
                    negative = false;
                    last_was_value = true;
                }
                ScriptToken::Punctuation { punctuation, text, .. } => {
                    if negative {
                        return Err(self.fail("misplaced minus sign in #if/#elif", &location));
                    }
                    let punctuation = *punctuation;
                    if punctuation == Punctuation::ParenthesisOpen.as_u32() {
                        depth += 1;
                        index += 1;
                        continue;
                    }
                    if punctuation == Punctuation::ParenthesisClose.as_u32() {
                        depth -= 1;
                        if depth < 0 {
                            return Err(self.fail("too many ) in #if/#elsif", &location));
                        }
                        index += 1;
                        continue;
                    }
                    if !self.integer_mode
                        && matches!(
                            Punctuation::from_u32(punctuation),
                            Some(
                                Punctuation::BinaryNot
                                    | Punctuation::Modulo
                                    | Punctuation::RightShift
                                    | Punctuation::LeftShift
                                    | Punctuation::BinaryAnd
                                    | Punctuation::BinaryOr
                                    | Punctuation::BinaryXor
                            )
                        )
                    {
                        return Err(self.fail(
                            &format!("illigal operator {text} on floating point operands\n"),
                            &location,
                        ));
                    }
                    if punctuation == Punctuation::LogicalNot.as_u32() || punctuation == Punctuation::BinaryNot.as_u32()
                    {
                        if last_was_value {
                            return Err(self.fail("! or ~ after value in #if/#elif", &location));
                        }
                    } else if punctuation == Punctuation::Subtract.as_u32() && !last_was_value {
                        negative = true;
                        index += 1;
                        continue;
                    } else if punctuation == Punctuation::Increment.as_u32()
                        || punctuation == Punctuation::Decrement.as_u32()
                    {
                        self.events.push(ExprEvent::IncrementReport);
                    } else {
                        if expression_precedence(punctuation).is_none() {
                            return Err(self.fail(&format!("invalid operator {text} in #if/#elif"), &location));
                        }
                        if !last_was_value {
                            return Err(self.fail(&format!("operator {text} after operator in #if/#elif"), &location));
                        }
                    }
                    if operators.len() >= 64 {
                        return Err(self.fail("out of operator space\n", &location));
                    }
                    let priority = if punctuation == Punctuation::LogicalNot.as_u32()
                        || punctuation == Punctuation::BinaryNot.as_u32()
                    {
                        16
                    } else if punctuation == Punctuation::Increment.as_u32()
                        || punctuation == Punctuation::Decrement.as_u32()
                    {
                        0
                    } else if let Some(priority) = expression_precedence(punctuation) {
                        priority
                    } else {
                        return Err(ExprError::Internal("source expression priority is missing".to_string()));
                    };
                    operators.push(ExprOperator {
                        punctuation,
                        text: text.clone(),
                        location,
                        depth,
                        priority,
                    });
                    last_was_value = false;
                }
                _ => {
                    return Err(self.fail(&format!("unknown {} in #if/#elif", stored.token.text()), &location));
                }
            }
            index += 1;
        }
        if !last_was_value {
            return Err(self.fail("trailing operator in #if/#elif", &self.fallback.clone()));
        }
        if depth != 0 {
            return Err(self.fail("too many ( in #if/#elif", &self.fallback.clone()));
        }
        let mut question: Option<EvalValue> = None;
        while !operators.is_empty() {
            let mut operator_index = 0usize;
            let mut value_index = 0usize;
            while operator_index + 1 < operators.len() {
                let (current_depth, current_priority) =
                    (operators[operator_index].depth, operators[operator_index].priority);
                let (next_depth, next_priority) = (
                    operators[operator_index + 1].depth,
                    operators[operator_index + 1].priority,
                );
                if current_depth > next_depth || (current_depth == next_depth && current_priority >= next_priority) {
                    break;
                }
                let current_punctuation = operators[operator_index].punctuation;
                if current_punctuation != Punctuation::LogicalNot.as_u32()
                    && current_punctuation != Punctuation::BinaryNot.as_u32()
                {
                    value_index += 1;
                    if value_index >= values.len() {
                        let location = operators[operator_index].location.clone();
                        return Err(self.fail("mising values in #if/#elif", &location));
                    }
                }
                operator_index += 1;
            }
            if value_index >= values.len() {
                return Err(ExprError::Internal(
                    "source expression reduction dereferences a missing value".to_string(),
                ));
            }
            let operator = ExprOperator {
                punctuation: operators[operator_index].punctuation,
                text: operators[operator_index].text.clone(),
                location: operators[operator_index].location.clone(),
                depth: operators[operator_index].depth,
                priority: operators[operator_index].priority,
            };
            self.events.push(ExprEvent::DebugLine(format!(
                "operator {}, value1 = {}",
                operator.text,
                debug_evaluation_value(&values[value_index], self.integer_mode)
            )));
            if value_index + 1 < values.len() {
                self.events.push(ExprEvent::DebugLine(format!(
                    "value2 = {}",
                    debug_evaluation_value(&values[value_index + 1], self.integer_mode)
                )));
            }
            match self.apply_operator(&mut values, &mut question, value_index, &operator) {
                Ok(after) => {
                    self.events.push(ExprEvent::DebugLine(format!(
                        "result value = {}",
                        debug_evaluation_value(&after, self.integer_mode)
                    )));
                    operators.remove(operator_index);
                }
                Err(ExprError::Fail { message, location }) => {
                    self.events.push(ExprEvent::DebugLine(format!(
                        "result value = {}",
                        debug_evaluation_value(&values[value_index], self.integer_mode)
                    )));
                    return Err(ExprError::Fail { message, location });
                }
                Err(internal) => return Err(internal),
            }
        }
        let Some(first) = values.first().copied() else {
            return Err(ExprError::Internal("source expression has no result value".to_string()));
        };
        Ok(first)
    }

    fn apply_operator(
        &self,
        values: &mut Vec<EvalValue>,
        question: &mut Option<EvalValue>,
        value_index: usize,
        operator: &ExprOperator,
    ) -> Result<EvalValue, ExprError> {
        let punctuation = operator.punctuation;
        if punctuation == Punctuation::LogicalNot.as_u32() {
            let current = values[value_index];
            let updated = EvalValue {
                integer_value: i32::from(current.integer_value == 0),
                float_value: f64::from(u8::from(current.float_value == 0.0)),
            };
            values[value_index] = updated;
            return Ok(updated);
        }
        if punctuation == Punctuation::BinaryNot.as_u32() {
            let current = values[value_index];
            let updated = EvalValue {
                integer_value: !current.integer_value,
                float_value: current.float_value,
            };
            values[value_index] = updated;
            return Ok(updated);
        }
        if punctuation == Punctuation::Question.as_u32() {
            if question.is_some() {
                return Err(self.fail("? after ? in #if/#elif", &operator.location));
            }
            let current = values[value_index];
            *question = Some(current);
            values.remove(value_index);
            return Ok(current);
        }
        if value_index + 1 >= values.len() {
            return Err(ExprError::Internal(
                "source expression binary reduction dereferences a missing value".to_string(),
            ));
        }
        let left = values[value_index];
        let right = values[value_index + 1];
        if punctuation == Punctuation::Colon.as_u32() {
            let Some(ask) = *question else {
                return Err(self.fail(": without ? in #if/#elif", &operator.location));
            };
            *question = None;
            let updated = if self.integer_mode {
                EvalValue {
                    integer_value: if ask.integer_value == 0 {
                        right.integer_value
                    } else {
                        left.integer_value
                    },
                    float_value: left.float_value,
                }
            } else {
                EvalValue {
                    integer_value: left.integer_value,
                    float_value: if ask.float_value == 0.0 {
                        right.float_value
                    } else {
                        left.float_value
                    },
                }
            };
            values[value_index] = updated;
            values.remove(value_index + 1);
            return Ok(updated);
        }
        if punctuation != Punctuation::Increment.as_u32() && punctuation != Punctuation::Decrement.as_u32() {
            let updated = self.binary(operator, &left, &right)?;
            values[value_index] = updated;
            values.remove(value_index + 1);
            return Ok(updated);
        }
        values.remove(value_index + 1);
        Ok(values[value_index])
    }

    fn binary(&self, operator: &ExprOperator, left: &EvalValue, right: &EvalValue) -> Result<EvalValue, ExprError> {
        let location = &operator.location;
        match Punctuation::from_u32(operator.punctuation) {
            Some(Punctuation::Multiply) => Ok(EvalValue {
                integer_value: signed_expression_value(i64::from(left.integer_value) * i64::from(right.integer_value))
                    .map_err(ExprError::Internal)?,
                float_value: left.float_value * right.float_value,
            }),
            Some(Punctuation::Divide) => {
                if right.integer_value == 0 || right.float_value == 0.0 {
                    return Err(self.fail("divide by zero in #if/#elif\n", location));
                }
                Ok(EvalValue {
                    integer_value: signed_expression_value(
                        i64::from(left.integer_value) / i64::from(right.integer_value),
                    )
                    .map_err(ExprError::Internal)?,
                    float_value: left.float_value / right.float_value,
                })
            }
            Some(Punctuation::Modulo) => {
                self.require_integer_operator(operator)?;
                if right.integer_value == 0 {
                    return Err(self.fail("divide by zero in #if/#elif\n", location));
                }
                signed_expression_value(i64::from(left.integer_value) / i64::from(right.integer_value))
                    .map_err(ExprError::Internal)?;
                Ok(EvalValue {
                    integer_value: signed_expression_value(
                        i64::from(left.integer_value) % i64::from(right.integer_value),
                    )
                    .map_err(ExprError::Internal)?,
                    float_value: left.float_value,
                })
            }
            Some(Punctuation::Add) => Ok(EvalValue {
                integer_value: signed_expression_value(i64::from(left.integer_value) + i64::from(right.integer_value))
                    .map_err(ExprError::Internal)?,
                float_value: left.float_value + right.float_value,
            }),
            Some(Punctuation::Subtract) => Ok(EvalValue {
                integer_value: signed_expression_value(i64::from(left.integer_value) - i64::from(right.integer_value))
                    .map_err(ExprError::Internal)?,
                float_value: left.float_value - right.float_value,
            }),
            Some(Punctuation::LogicalAnd) => Ok(EvalValue {
                integer_value: i32::from(left.integer_value != 0 && right.integer_value != 0),
                float_value: f64::from(u8::from(left.float_value != 0.0 && right.float_value != 0.0)),
            }),
            Some(Punctuation::LogicalOr) => Ok(EvalValue {
                integer_value: i32::from(left.integer_value != 0 || right.integer_value != 0),
                float_value: f64::from(u8::from(left.float_value != 0.0 || right.float_value != 0.0)),
            }),
            Some(Punctuation::GreaterOrEqual) => Ok(EvalValue {
                integer_value: i32::from(left.integer_value >= right.integer_value),
                float_value: f64::from(u8::from(left.float_value >= right.float_value)),
            }),
            Some(Punctuation::LessOrEqual) => Ok(EvalValue {
                integer_value: i32::from(left.integer_value <= right.integer_value),
                float_value: f64::from(u8::from(left.float_value <= right.float_value)),
            }),
            Some(Punctuation::Equal) => Ok(EvalValue {
                integer_value: i32::from(left.integer_value == right.integer_value),
                float_value: f64::from(u8::from(left.float_value == right.float_value)),
            }),
            Some(Punctuation::NotEqual) => Ok(EvalValue {
                integer_value: i32::from(left.integer_value != right.integer_value),
                float_value: f64::from(u8::from(left.float_value != right.float_value)),
            }),
            Some(Punctuation::Greater) => Ok(EvalValue {
                integer_value: i32::from(left.integer_value > right.integer_value),
                float_value: f64::from(u8::from(left.float_value > right.float_value)),
            }),
            Some(Punctuation::Less) => Ok(EvalValue {
                integer_value: i32::from(left.integer_value < right.integer_value),
                float_value: f64::from(u8::from(left.float_value < right.float_value)),
            }),
            Some(Punctuation::RightShift) => {
                self.require_integer_operator(operator)?;
                if right.integer_value < 0 || right.integer_value >= 32 {
                    return Err(ExprError::Internal(
                        "source right shift count exceeds its signed-long width".to_string(),
                    ));
                }
                Ok(EvalValue {
                    integer_value: left.integer_value >> right.integer_value,
                    float_value: left.float_value,
                })
            }
            Some(Punctuation::LeftShift) => {
                self.require_integer_operator(operator)?;
                if right.integer_value < 0 || right.integer_value >= 32 || left.integer_value < 0 {
                    return Err(ExprError::Internal(
                        "source signed left shift has an undefined operand".to_string(),
                    ));
                }
                Ok(EvalValue {
                    integer_value: signed_expression_value(
                        i64::from(left.integer_value) * (1i64 << right.integer_value),
                    )
                    .map_err(ExprError::Internal)?,
                    float_value: left.float_value,
                })
            }
            Some(Punctuation::BinaryAnd) => {
                self.require_integer_operator(operator)?;
                Ok(EvalValue {
                    integer_value: left.integer_value & right.integer_value,
                    float_value: left.float_value,
                })
            }
            Some(Punctuation::BinaryOr) => {
                self.require_integer_operator(operator)?;
                Ok(EvalValue {
                    integer_value: left.integer_value | right.integer_value,
                    float_value: left.float_value,
                })
            }
            Some(Punctuation::BinaryXor) => {
                self.require_integer_operator(operator)?;
                Ok(EvalValue {
                    integer_value: left.integer_value ^ right.integer_value,
                    float_value: left.float_value,
                })
            }
            _ => Err(self.fail(&format!("unsupported expression operator {}", operator.text), location)),
        }
    }

    fn require_integer_operator(&self, operator: &ExprOperator) -> Result<(), ExprError> {
        if !self.integer_mode {
            return Err(self.fail(
                &format!("operator {} is invalid in a floating-point expression", operator.text),
                &operator.location,
            ));
        }
        Ok(())
    }

    fn fail(&self, message: &str, _location: &SourceLocation) -> ExprError {
        // The donor `fail` records at the current lexer location; the engine
        // applies that when converting this error.
        ExprError::Fail {
            message: message.to_string(),
            location: self.fallback.clone(),
        }
    }
}

/// Engine resource limits (donor `Limits`).
#[derive(Debug, Clone)]
struct Limits {
    include_depth: usize,
    macro_expansions: usize,
    queued_tokens: usize,
    output_tokens: usize,
    source_tokens: usize,
    defines: usize,
    expression_tokens: usize,
}

impl Limits {
    fn unlimited() -> Limits {
        Limits {
            include_depth: usize::MAX,
            macro_expansions: usize::MAX,
            queued_tokens: usize::MAX,
            output_tokens: usize::MAX,
            source_tokens: usize::MAX,
            defines: usize::MAX,
            expression_tokens: usize::MAX,
        }
    }
}

/// One open source file (donor `SourceFrame`).
struct Frame {
    lexer: RawLexer,
    token_count: usize,
}

/// Conditional stack entry (donor `SourceIndent`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Indent {
    kind: IndentKind,
    skip: bool,
    frame: u64,
}

/// Conditional flavor (donor `SourceIndentType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndentKind {
    If,
    Else,
    Elif,
    Ifdef,
    Ifndef,
}

/// Macro definition (donor `PrecompDefine` data).
#[derive(Debug, Clone, PartialEq)]
struct MacroDef {
    name: String,
    fixed: bool,
    builtin: u32,
    params: Vec<String>,
    body: Vec<StoredToken>,
}

/// `#eval` form: `#` line form or `$` paren form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvalForm {
    Hash,
    Dollar,
}

/// Preprocessor engine (donor `PreprocessorEngine`).
struct Engine {
    resolver: Box<dyn IncludeResolver>,
    limits: Limits,
    now: NowCallback,
    report: Option<ReportCallback>,
    debug_eval: Option<DebugEvalCallback>,
    reported: Vec<ScriptDiagnostic>,
    failures: HashSet<String>,
    root_path: String,
    macros: HashMap<String, Vec<MacroDef>>,
    macro_count: usize,
    buckets: Vec<Vec<String>>,
    frames: HashMap<u64, Frame>,
    stack: Vec<u64>,
    next_frame_id: u64,
    queued: VecDeque<StoredToken>,
    last_output: Option<StoredToken>,
    indents: Vec<Indent>,
    skip_count: usize,
    include_path: String,
    punctuations: Option<Vec<ScriptPunctuation>>,
    expansion_count: usize,
    output_count: usize,
}

impl Engine {
    fn minimal(root_path: &str, text: &str) -> Engine {
        let mut frames = HashMap::new();
        frames.insert(
            1,
            Frame {
                lexer: RawLexer::new(text, root_path),
                token_count: 0,
            },
        );
        Engine {
            resolver: Box::new(NullIncludeResolver),
            limits: Limits::unlimited(),
            now: Box::new(system_date_time),
            report: None,
            debug_eval: None,
            reported: Vec::new(),
            failures: HashSet::new(),
            root_path: root_path.to_string(),
            macros: HashMap::new(),
            macro_count: 0,
            buckets: vec![Vec::new(); DEFINE_HASH_BUCKETS],
            frames,
            stack: vec![1],
            next_frame_id: 2,
            queued: VecDeque::new(),
            last_output: None,
            indents: Vec::new(),
            skip_count: 0,
            include_path: String::new(),
            punctuations: None,
            expansion_count: 0,
            output_count: 0,
        }
    }

    fn current_frame_id(&self) -> u64 {
        self.stack.last().copied().unwrap_or(1)
    }

    fn current_location(&self) -> SourceLocation {
        self.stack
            .last()
            .and_then(|id| self.frames.get(id))
            .map(|frame| frame.lexer.current_location())
            .unwrap_or(SourceLocation {
                path: self.root_path.clone(),
                line: 1,
                column: 1,
            })
    }

    fn current_script_filename(&self) -> String {
        self.current_location().path
    }

    fn is_active(&self) -> bool {
        self.skip_count == 0
    }

    fn record(&mut self, diagnostic: ScriptDiagnostic) {
        self.reported.push(diagnostic.clone());
        if let Some(report) = self.report.as_mut() {
            report(&diagnostic);
        }
    }

    fn warn(&mut self, message: impl Into<String>) {
        let location = self.current_location();
        self.record(ScriptDiagnostic {
            severity: DiagnosticSeverity::Warning,
            message: message.into(),
            location,
        });
    }

    fn error(&mut self, message: impl Into<String>) {
        let location = self.current_location();
        self.record(ScriptDiagnostic {
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            location,
        });
    }

    fn fail(&mut self, message: impl Into<String>) -> EngineError {
        let location = self.current_location();
        let diagnostic = ScriptDiagnostic {
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            location,
        };
        self.record(diagnostic.clone());
        self.failures.insert(rendered_diagnostic(&diagnostic));
        EngineError::Fail(diagnostic)
    }

    fn fail_with(&mut self, diagnostic: ScriptDiagnostic) -> EngineError {
        self.failures.insert(rendered_diagnostic(&diagnostic));
        EngineError::Fail(diagnostic)
    }

    fn debug_line(&mut self, text: String) {
        if let Some(debug) = self.debug_eval.as_mut() {
            debug(&text);
        }
    }

    fn macro_defined(&self, name: &str) -> bool {
        self.macros.get(name).is_some_and(|stack| !stack.is_empty())
    }

    fn macro_get(&self, name: &str) -> Option<&MacroDef> {
        self.macros.get(name).and_then(|stack| stack.last())
    }

    fn macro_prepend(&mut self, macro_def: MacroDef) {
        let bucket = define_bucket(&macro_def.name);
        self.buckets[bucket].insert(0, macro_def.name.clone());
        self.macros.entry(macro_def.name.clone()).or_default().push(macro_def);
        self.macro_count += 1;
    }

    fn macro_delete(&mut self, name: &str) {
        let Some(stack) = self.macros.get_mut(name) else {
            return;
        };
        if stack.pop().is_none() {
            return;
        }
        if stack.is_empty() {
            self.macros.remove(name);
        }
        let bucket = define_bucket(name);
        if let Some(position) = self.buckets[bucket].iter().position(|entry| entry == name) {
            self.buckets[bucket].remove(position);
        }
        self.macro_count = self.macro_count.saturating_sub(1);
    }

    fn macro_first(&self) -> Option<MacroDef> {
        for bucket in &self.buckets {
            if let Some(name) = bucket.first() {
                if let Some(found) = self.macro_get(name) {
                    return Some(found.clone());
                }
            }
        }
        None
    }

    fn macro_newest_mut(&mut self, name: &str) -> Option<&mut MacroDef> {
        self.macros.get_mut(name).and_then(|stack| stack.last_mut())
    }

    fn has_current_indent(&self) -> bool {
        let current = self.current_frame_id();
        self.indents.last().is_some_and(|indent| indent.frame == current)
    }

    fn push_indent(&mut self, kind: IndentKind, skip: bool) {
        let frame = self.current_frame_id();
        if skip {
            self.skip_count += 1;
        }
        self.indents.push(Indent { kind, skip, frame });
    }

    fn pop_indent(&mut self) -> Option<Indent> {
        let current = self.current_frame_id();
        if self.indents.last().is_some_and(|indent| indent.frame == current) {
            let indent = self.indents.pop();
            if indent.as_ref().is_some_and(|popped| popped.skip) {
                self.skip_count = self.skip_count.saturating_sub(1);
            }
            indent
        } else {
            None
        }
    }

    fn read_source_token(&mut self) -> Result<Option<StoredToken>, EngineError> {
        if let Some(stored) = self.queued.pop_front() {
            return Ok(Some(stored));
        }
        loop {
            let mut failure: Option<ScriptDiagnostic> = None;
            let frame_id = self.current_frame_id();
            let step = match self.frames.get_mut(&frame_id) {
                Some(frame) => frame.lexer.next_token(),
                None => {
                    return Err(EngineError::Internal(
                        "source script pointer does not identify a live script".to_string(),
                    ));
                }
            };
            let (mut emitted, token, failed, internal) = split_lexer_step(step);
            for diagnostic in emitted.drain(..) {
                self.record(diagnostic);
            }
            if let Some(message) = internal {
                return Err(EngineError::Internal(message));
            }
            if failed {
                failure = self.reported.last().cloned();
            }
            if let Some(stored) = token {
                let limit = self.limits.source_tokens;
                let count = self.frames.get(&frame_id).map_or(0, |frame| frame.token_count);
                if count >= limit {
                    return Err(self.fail(format!("source exceeds the {limit}-token limit")));
                }
                if let Some(frame) = self.frames.get_mut(&frame_id) {
                    frame.token_count += 1;
                }
                return Ok(Some(stored));
            }
            let at_end = self.frames.get(&frame_id).is_some_and(|frame| frame.lexer.at_end());
            if at_end {
                while self.has_current_indent() {
                    self.warn("missing #endif");
                    self.pop_indent();
                }
            }
            if self.stack.len() <= 1 {
                if let Some(diagnostic) = failure {
                    return Err(self.fail_with(diagnostic));
                }
                return Ok(None);
            }
            self.stack.pop();
            self.frames.remove(&frame_id);
        }
    }

    fn read_token(&mut self) -> Result<Option<StoredToken>, EngineError> {
        loop {
            let stored = match self.read_source_token()? {
                Some(stored) => stored,
                None => return Ok(None),
            };
            if stored.token.is_punctuation(Punctuation::Preprocessor) {
                self.directive()?;
                continue;
            }
            if stored.token.is_punctuation(Punctuation::Dollar) {
                self.dollar_directive()?;
                continue;
            }
            let mut stored = stored;
            if matches!(stored.token.kind(), "string") {
                let next = match self.read_token() {
                    Ok(next) => next,
                    Err(EngineError::ReadFalse) | Err(EngineError::Fail(_)) => None,
                    Err(other) => return Err(other),
                };
                if let Some(next) = next {
                    if matches!(next.token.kind(), "string") {
                        // The donor NUL-truncates the first closing quote
                        // before concatenating; mirror that exactly.
                        let head = stored
                            .token
                            .text()
                            .get(..stored.token.text().len().saturating_sub(1))
                            .unwrap_or("");
                        let rest = next.token.text().get(1..).unwrap_or("");
                        let combined = format!("{head}{rest}");
                        if combined.len() + 1 >= DEFAULT_SCRIPT_TOKEN_LIMIT {
                            return Err(
                                self.fail(format!("string longer than MAX_TOKEN {DEFAULT_SCRIPT_TOKEN_LIMIT}\n"))
                            );
                        }
                        let value = combined
                            .get(1..combined.len().saturating_sub(1))
                            .unwrap_or("")
                            .to_string();
                        let (location, leading, crossed) = match &stored.token {
                            ScriptToken::String {
                                location,
                                leading_whitespace,
                                lines_crossed,
                                ..
                            } => (location.clone(), leading_whitespace.clone(), *lines_crossed),
                            _ => (self.current_location(), String::new(), 0),
                        };
                        stored.token = ScriptToken::String {
                            text: combined.clone(),
                            value,
                            length: combined.len(),
                            location,
                            leading_whitespace: leading,
                            lines_crossed: crossed,
                        };
                    } else {
                        self.queued.push_front(next);
                    }
                }
            }
            if !self.is_active() {
                continue;
            }
            let is_macro = matches!(&stored.token, ScriptToken::Name { value, .. } if self.macro_defined(value));
            if is_macro {
                if !self.expand_macro_into_source(&stored)? {
                    return Ok(None);
                }
                continue;
            }
            self.last_output = Some(stored.clone());
            return Ok(Some(stored));
        }
    }

    fn next_token_internal(&mut self) -> Result<Option<StoredToken>, EngineError> {
        match self.read_token() {
            Err(EngineError::ReadFalse) => Ok(None),
            Err(other) => Err(other),
            Ok(None) => Ok(None),
            Ok(Some(stored)) => {
                if self.output_count >= self.limits.output_tokens {
                    let limit = self.limits.output_tokens;
                    return Err(self.fail(format!("preprocessor output exceeds {limit} tokens")));
                }
                self.output_count += 1;
                Ok(Some(stored))
            }
        }
    }

    fn next_record_internal(&mut self) -> Result<Option<ScriptTokenRecord>, EngineError> {
        let stored = match self.next_token_internal()? {
            Some(stored) => stored,
            None => return Ok(None),
        };
        if let Some(reason) = &stored.unsupported {
            return Err(self.fail(format!("source token profile is unsupported: {reason}")));
        }
        Ok(Some(stored.record()))
    }

    fn read_expected(&mut self) -> Result<Option<ScriptTokenRecord>, EngineError> {
        match self.next_record_internal() {
            Err(EngineError::Fail(_)) => Ok(None),
            other => other,
        }
    }

    fn read_line_token(&mut self) -> Result<Option<StoredToken>, EngineError> {
        let mut may_cross = false;
        loop {
            let stored = match self.read_source_token()? {
                Some(stored) => stored,
                None => return Ok(None),
            };
            if stored.token.lines_crossed() > usize::from(may_cross) {
                self.queued.push_front(stored);
                return Ok(None);
            }
            if stored.token.text() == "\\" {
                may_cross = true;
                continue;
            }
            return Ok(Some(stored));
        }
    }

    fn read_line(&mut self) -> Result<Vec<StoredToken>, EngineError> {
        let mut line = Vec::new();
        while let Some(stored) = self.read_line_token()? {
            line.push(stored);
        }
        Ok(line)
    }

    fn apply_expr_events(&mut self, events: Vec<ExprEvent>) {
        for event in events {
            match event {
                ExprEvent::DebugLine(text) => self.debug_line(text),
                ExprEvent::IncrementReport => self.error("++ or -- used in #if/#elif"),
            }
        }
    }

    fn print_define_hash_table(&self, write: &mut dyn FnMut(&str)) {
        for (bucket, names) in self.buckets.iter().enumerate() {
            write(&format!("{bucket:>4}:"));
            for name in names {
                write(&format!(" {name}"));
            }
            write("\n");
        }
    }
}

/// Split a lexer step into recordable diagnostics, the token, the failure
/// flag, and a possible internal (non-source) error smuggled through the
/// sentinel diagnostic (lexer paths are never empty, so the empty-path
/// marker is unambiguous).
fn split_lexer_step(step: LexerStep) -> (Vec<ScriptDiagnostic>, Option<StoredToken>, bool, Option<String>) {
    let mut emitted = Vec::new();
    let mut internal = None;
    for diagnostic in step.emitted {
        if diagnostic.location.path.is_empty() && diagnostic.location.line == 0 {
            let message = diagnostic
                .message
                .strip_prefix("internal: ")
                .unwrap_or(&diagnostic.message)
                .to_string();
            internal = Some(message);
        } else {
            emitted.push(diagnostic);
        }
    }
    (emitted, step.token, step.failed, internal)
}

impl Engine {
    fn directive(&mut self) -> Result<(), EngineError> {
        let token = self.read_source_token()?;
        let stored = match token {
            None => return Err(self.fail("found # without name")),
            Some(stored) if stored.token.lines_crossed() > 0 => {
                self.queued.push_front(stored);
                return Err(self.fail("found # at end of line"));
            }
            Some(stored) => stored,
        };
        let value = match &stored.token {
            ScriptToken::Name { value, .. } => Some(value.clone()),
            _ => None,
        };
        let Some(value) = value else {
            let text = stored.token.text().to_string();
            return Err(self.fail(format!("unknown precompiler directive {text}")));
        };
        match value.as_str() {
            "if" => self.if_directive(),
            "ifdef" => self.ifdef_directive(false),
            "ifndef" => self.ifdef_directive(true),
            "elif" => self.elif_directive(),
            "else" => self.else_directive(),
            "endif" => self.endif_directive(),
            "include" => {
                if self.is_active() {
                    self.include_directive()?;
                }
                Ok(())
            }
            "define" => {
                if self.is_active() {
                    self.define_directive()?;
                }
                Ok(())
            }
            "undef" => {
                if self.is_active() {
                    let name = self.read_line_token()?;
                    self.undefine(name.into_iter().collect())?;
                }
                Ok(())
            }
            "eval" => self.evaluate_directive(true, EvalForm::Hash),
            "evalfloat" => self.evaluate_directive(false, EvalForm::Hash),
            "line" => Err(self.fail("#line directive not supported")),
            "error" => {
                let detail = self
                    .read_source_token()?
                    .map(|token| token.token.text().to_string())
                    .unwrap_or_default();
                Err(self.fail(format!("#error directive: {detail}")))
            }
            "pragma" => {
                self.warn("#pragma directive not supported");
                self.read_line()?;
                Ok(())
            }
            _ => Err(self.fail(format!("unknown precompiler directive {value}"))),
        }
    }

    fn if_directive(&mut self) -> Result<(), EngineError> {
        let selected = self.evaluate_expression(true, false)?.integer_value != 0;
        self.push_indent(IndentKind::If, !selected);
        Ok(())
    }

    fn ifdef_directive(&mut self, negate: bool) -> Result<(), EngineError> {
        let name = self.read_line_token()?;
        let value = match &name {
            Some(stored) => match &stored.token {
                ScriptToken::Name { value, .. } => Some(value.clone()),
                _ => None,
            },
            None => None,
        };
        match (name, value) {
            (Some(_), Some(value)) => {
                let defined = self.macro_defined(&value);
                let selected = if negate { !defined } else { defined };
                self.push_indent(if negate { IndentKind::Ifndef } else { IndentKind::Ifdef }, !selected);
                Ok(())
            }
            (Some(stored), None) => {
                let text = stored.token.text().to_string();
                self.queued.push_front(stored);
                Err(self.fail(format!("expected name after #ifdef, found {text}")))
            }
            (None, _) => Err(self.fail("#ifdef without name")),
        }
    }

    fn elif_directive(&mut self) -> Result<(), EngineError> {
        match self.pop_indent() {
            None => return Err(self.fail("misplaced #elif")),
            Some(indent) if indent.kind == IndentKind::Else => {
                return Err(self.fail("misplaced #elif"));
            }
            Some(_) => {}
        }
        let selected = self.evaluate_expression(true, false)?.integer_value != 0;
        self.push_indent(IndentKind::Elif, !selected);
        Ok(())
    }

    fn else_directive(&mut self) -> Result<(), EngineError> {
        let conditional = self.pop_indent();
        match conditional {
            None => Err(self.fail("misplaced #else")),
            Some(indent) if indent.kind == IndentKind::Else => Err(self.fail("#else after #else")),
            Some(indent) => {
                self.push_indent(IndentKind::Else, !indent.skip);
                Ok(())
            }
        }
    }

    fn endif_directive(&mut self) -> Result<(), EngineError> {
        if self.pop_indent().is_none() {
            return Err(self.fail("misplaced #endif"));
        }
        Ok(())
    }

    fn define_directive(&mut self) -> Result<(), EngineError> {
        let name_token = self.read_line_token()?;
        let name = match &name_token {
            Some(stored) => match &stored.token {
                ScriptToken::Name { value, .. } => Some(value.clone()),
                _ => None,
            },
            None => None,
        };
        let (name_stored, name) = match (name_token, name) {
            (Some(stored), Some(name)) => (stored, name),
            (Some(stored), None) => {
                let text = stored.token.text().to_string();
                self.queued.push_front(stored);
                return Err(self.fail(format!("expected name after #define, found {text}")));
            }
            (None, _) => return Err(self.fail("#define without name")),
        };
        if self.macro_get(&name).is_some_and(|defined| defined.fixed) {
            return Err(self.fail(format!("can't redefine {name}")));
        }
        if self.macro_defined(&name) {
            self.warn(format!("redefinition of {name}"));
            self.queued.push_front(name_stored);
            let reread = self.read_line_token()?;
            self.undefine(reread.into_iter().collect())?;
        } else if self.macro_count >= self.limits.defines {
            let limit = self.limits.defines;
            return Err(self.fail(format!("define count exceeds {limit}")));
        }
        self.macro_prepend(MacroDef {
            name: name.clone(),
            fixed: false,
            builtin: 0,
            params: Vec::new(),
            body: Vec::new(),
        });
        let token = self.read_line_token()?;
        let Some(token) = token else {
            return Ok(());
        };
        let function_like = token.token.text() == "(" && token.token.leading_whitespace().is_empty();
        let mut token = token;
        if function_like {
            let checked = self.read_token()?;
            if checked.as_ref().is_none_or(|candidate| candidate.token.text() != ")") {
                if let Some(checked) = checked {
                    self.queued.push_front(checked);
                }
                let mut params: Vec<String> = Vec::new();
                loop {
                    let parameter = self.read_line_token()?;
                    let parameter_name = match &parameter {
                        Some(stored) => match &stored.token {
                            ScriptToken::Name { value, .. } => Some(value.clone()),
                            _ => None,
                        },
                        None => None,
                    };
                    let Some(parameter_name) = parameter_name else {
                        if parameter.is_none() {
                            return Err(self.fail("expected define parameter"));
                        }
                        return Err(self.fail("invalid define parameter"));
                    };
                    if params.iter().any(|existing| existing == &parameter_name) {
                        return Err(self.fail("two the same define parameters"));
                    }
                    params.push(parameter_name);
                    if let Some(newest) = self.macro_newest_mut(&name) {
                        newest.params.clone_from(&params);
                    }
                    let separator = self.read_line_token()?;
                    match separator {
                        None => return Err(self.fail("define parameters not terminated")),
                        Some(stored) if stored.token.text() == ")" => break,
                        Some(stored) if stored.token.text() == "," => {}
                        Some(_) => return Err(self.fail("define not terminated")),
                    }
                }
            }
            let rest = self.read_line_token()?;
            let Some(rest) = rest else {
                return Ok(());
            };
            token = rest;
        }
        let mut body: Vec<StoredToken> = Vec::new();
        loop {
            let cleared = token.clone().cleared();
            let recursive = matches!(&cleared.token, ScriptToken::Name { value, .. } if value == &name);
            if recursive {
                self.error("recursive define (removed recursion)");
            } else {
                body.push(cleared);
                if let Some(newest) = self.macro_newest_mut(&name) {
                    newest.body.clone_from(&body);
                }
            }
            let next = self.read_line_token()?;
            let Some(next) = next else {
                break;
            };
            token = next;
        }
        let misplaced = !body.is_empty()
            && (body.first().is_some_and(|first| first.token.text() == "##")
                || body.last().is_some_and(|last| last.token.text() == "##"));
        if misplaced {
            return Err(self.fail("define with misplaced ##"));
        }
        Ok(())
    }

    fn undefine(&mut self, tokens: Vec<StoredToken>) -> Result<(), EngineError> {
        let valid =
            tokens.len() == 1 && matches!(tokens.first().map(|token| &token.token), Some(ScriptToken::Name { .. }));
        if !valid {
            let mut tokens = tokens.into_iter();
            if let Some(invalid) = tokens.next() {
                let text = invalid.token.text().to_string();
                self.queued.push_front(invalid);
                return Err(self.fail(format!("expected name, found {text}")));
            }
            return Err(self.fail("undef without name"));
        }
        let name = match &tokens[0].token {
            ScriptToken::Name { value, .. } => value.clone(),
            _ => return Err(EngineError::Internal("undef validation was inconsistent".to_string())),
        };
        if self.macro_get(&name).is_some_and(|defined| defined.fixed) {
            self.warn(format!("can't undef {name}"));
            return Ok(());
        }
        self.macro_delete(&name);
        Ok(())
    }

    fn include_directive(&mut self) -> Result<(), EngineError> {
        let source_path = self.current_script_filename();
        let first = self.read_source_token()?;
        let first = match first {
            None => return Err(self.fail("#include without file name")),
            Some(stored) if stored.token.lines_crossed() > 0 => {
                return Err(self.fail("#include without file name"));
            }
            Some(stored) => stored,
        };
        let request = match &first.token {
            ScriptToken::String { value, .. } => IncludeRequest {
                kind: IncludeKind::Quoted,
                from_path: source_path,
                requested_path: value.clone(),
                include_path: if self.include_path.is_empty() {
                    None
                } else {
                    Some(self.include_path.clone())
                },
            },
            ScriptToken::Punctuation { punctuation, .. } if *punctuation == Punctuation::Less.as_u32() => {
                let mut pieces = String::new();
                let mut closed = false;
                loop {
                    let token = self.read_source_token()?;
                    let Some(token) = token else {
                        break;
                    };
                    if token.token.lines_crossed() > 0 {
                        self.queued.push_front(token);
                        break;
                    }
                    if token.token.is_punctuation(Punctuation::Greater) {
                        closed = true;
                        break;
                    }
                    pieces.push_str(token.token.text());
                }
                if !closed {
                    self.warn("#include missing trailing >");
                }
                let requested_path = format!("{}{}", self.include_path, pieces);
                if requested_path.is_empty() {
                    return Err(self.fail("#include without file name between < >"));
                }
                IncludeRequest {
                    kind: IncludeKind::System,
                    from_path: source_path,
                    requested_path,
                    include_path: None,
                }
            }
            _ => return Err(self.fail("#include without file name")),
        };
        if self.frames.len() >= self.limits.include_depth {
            let limit = self.limits.include_depth;
            return Err(self.fail(format!("include depth exceeds {limit}")));
        }
        let resolved = match self.resolver.resolve(&request) {
            Ok(resolved) => resolved,
            Err(error) => return Err(EngineError::External(error)),
        };
        let Some(included) = resolved else {
            let converted = request.requested_path.split('\0').next().unwrap_or("");
            let mut normalized = String::new();
            let mut last_was_separator = false;
            for character in converted.chars() {
                if character == '/' || character == '\\' {
                    if !last_was_separator {
                        normalized.push('/');
                    }
                    last_was_separator = true;
                } else {
                    normalized.push(character);
                    last_was_separator = false;
                }
            }
            let failed_path = if request.kind == IncludeKind::Quoted {
                format!("{}{}", self.include_path, normalized)
            } else {
                normalized
            };
            return Err(self.fail(format!("file {failed_path} not found")));
        };
        if included.path.is_empty() {
            return Err(self.fail("include resolver returned an empty canonical path"));
        }
        let folded_new = folded_command_text(&included.path).map_err(EngineError::Internal)?;
        for id in self.stack.clone() {
            let path = self
                .frames
                .get(&id)
                .map(|frame| frame.lexer.path.clone())
                .unwrap_or_default();
            let folded = folded_command_text(&path).map_err(EngineError::Internal)?;
            if folded == folded_new {
                self.error(format!("{} recursively included", included.path));
                return Ok(());
            }
        }
        let id = self.next_frame_id;
        self.next_frame_id += 1;
        self.frames.insert(
            id,
            Frame {
                lexer: RawLexer::new(&included.text, &included.path),
                token_count: 0,
            },
        );
        self.stack.push(id);
        Ok(())
    }

    fn expand_macro_into_source(&mut self, invocation: &StoredToken) -> Result<bool, EngineError> {
        let name = match &invocation.token {
            ScriptToken::Name { value, .. } => value.clone(),
            _ => return Ok(true),
        };
        let macro_def = match self.macro_get(&name) {
            Some(found) => found.clone(),
            None => return Ok(true),
        };
        self.expansion_count += 1;
        if self.expansion_count > self.limits.macro_expansions {
            let limit = self.limits.macro_expansions;
            return Err(self.fail(format!("macro expansion count exceeds {limit}")));
        }
        let expanded = if macro_def.builtin != 0 {
            self.expand_builtin(&macro_def, invocation)
        } else {
            self.expand_define(&macro_def, invocation)?
        };
        if expanded.is_empty() {
            return Ok(false);
        }
        for stored in expanded.into_iter().rev() {
            self.queued.push_front(stored);
        }
        Ok(true)
    }

    fn read_macro_arguments(&mut self, macro_def: &MacroDef) -> Result<Vec<Vec<StoredToken>>, EngineError> {
        let opening = self.read_source_token()?;
        let Some(opening) = opening else {
            return Err(self.fail(format!("define {} missing parms", macro_def.name)));
        };
        if macro_def.params.len() > MAX_DEFINE_PARAMETERS {
            return Err(self.fail(format!("define with more than {MAX_DEFINE_PARAMETERS} parameters")));
        }
        let mut arguments: Vec<Vec<StoredToken>> = vec![Vec::new(); macro_def.params.len()];
        if opening.token.text() != "(" {
            self.queued.push_front(opening);
            return Err(self.fail(format!("define {} missing parms", macro_def.name)));
        }
        let mut argument_token_count = 0usize;
        let mut depth = 0i32;
        let mut argument = 0usize;
        loop {
            if argument >= MAX_DEFINE_PARAMETERS {
                return Err(self.fail(format!("define {} with too many parms", macro_def.name)));
            }
            if argument >= macro_def.params.len() {
                let diagnostic = ScriptDiagnostic {
                    severity: DiagnosticSeverity::Warning,
                    message: format!("define {} has too many parms", macro_def.name),
                    location: self.current_location(),
                };
                self.record(diagnostic.clone());
                return Err(self.fail_with(diagnostic));
            }
            let mut last_comma = true;
            let mut done = false;
            loop {
                let token = self.read_source_token()?;
                let Some(token) = token else {
                    return Err(self.fail(format!("define {} incomplete", macro_def.name)));
                };
                if token.token.text() == "," && depth <= 0 {
                    if last_comma {
                        self.warn("too many comma's");
                    }
                    break;
                }
                last_comma = false;
                if token.token.text() == "(" {
                    depth += 1;
                    continue;
                }
                if token.token.text() == ")" {
                    depth -= 1;
                    if depth <= 0 {
                        if arguments.last().is_some_and(|last: &Vec<StoredToken>| last.is_empty()) {
                            self.warn("too few define parms");
                        }
                        done = true;
                        break;
                    }
                }
                if argument_token_count >= self.limits.queued_tokens {
                    let limit = self.limits.queued_tokens;
                    return Err(self.fail(format!("macro argument queue exceeds {limit} tokens")));
                }
                argument_token_count += 1;
                arguments[argument].push(token);
            }
            argument += 1;
            if done {
                break;
            }
        }
        Ok(arguments)
    }

    fn append_expansion(&mut self, list: &mut Vec<StoredToken>, stored: StoredToken) -> Result<(), EngineError> {
        if list.len() >= self.limits.queued_tokens {
            let limit = self.limits.queued_tokens;
            return Err(self.fail(format!("macro expansion queue exceeds {limit} tokens")));
        }
        list.push(stored);
        Ok(())
    }

    fn expand_define(
        &mut self,
        macro_def: &MacroDef,
        invocation: &StoredToken,
    ) -> Result<Vec<StoredToken>, EngineError> {
        let arguments = if macro_def.params.is_empty() {
            Vec::new()
        } else {
            self.read_macro_arguments(macro_def)?
        };
        if self.debug_eval.is_some() {
            for (index, argument) in arguments.iter().enumerate() {
                self.debug_line(format!("define parms {index}:"));
                for stored in argument {
                    self.debug_line(stored.token.text().to_string());
                }
            }
        }
        let mut expanded: Vec<StoredToken> = Vec::new();
        let mut index = 0usize;
        while index < macro_def.body.len() {
            let defined = &macro_def.body[index];
            let parameter = match &defined.token {
                ScriptToken::Name { value, .. } => macro_def.params.iter().position(|param| param == value),
                _ => None,
            };
            if let Some(slot) = parameter {
                let Some(argument) = arguments.get(slot) else {
                    return Err(EngineError::Internal(
                        "define parameter does not identify a supplied argument slot".to_string(),
                    ));
                };
                for stored in argument.clone() {
                    self.append_expansion(&mut expanded, stored)?;
                }
                index += 1;
                continue;
            }
            if defined.token.text() == "#" {
                let next = macro_def.body.get(index + 1);
                let string_parameter = match next {
                    Some(candidate) => match &candidate.token {
                        ScriptToken::Name { value, .. } => macro_def.params.iter().position(|param| param == value),
                        _ => None,
                    },
                    None => None,
                };
                let Some(slot) = string_parameter else {
                    self.warn("stringizing operator without define parameter");
                    index += 1;
                    continue;
                };
                let Some(argument) = arguments.get(slot) else {
                    return Err(EngineError::Internal(
                        "stringizing parameter does not identify an argument slot".to_string(),
                    ));
                };
                let mut text = String::from("\"");
                for stored in argument {
                    text.push_str(stored.token.text());
                }
                text.push('"');
                let invocation_location = invocation.token.location();
                let value = text.get(1..text.len().saturating_sub(1)).unwrap_or("").to_string();
                let stringized = StoredToken {
                    token: ScriptToken::String {
                        text: text.clone(),
                        value,
                        length: text.len(),
                        location: SourceLocation {
                            path: invocation_location.path.clone(),
                            line: 0,
                            column: invocation_location.column,
                        },
                        leading_whitespace: String::new(),
                        lines_crossed: 0,
                    },
                    subtype: 0,
                    integer_value: 0,
                    float_value: 0.0,
                    unsupported: Some("macro stringizing leaves subtype and numeric fields uninitialized".to_string()),
                };
                self.append_expansion(&mut expanded, stringized)?;
                index += 2;
                continue;
            }
            self.append_expansion(&mut expanded, defined.clone())?;
            index += 1;
        }
        let mut at = 0usize;
        while at < expanded.len() {
            let mergeable = expanded
                .get(at + 1)
                .is_some_and(|candidate| candidate.token.text().starts_with("##"))
                && at + 2 < expanded.len();
            if !mergeable {
                at += 1;
                continue;
            }
            let left = expanded[at].clone();
            let right = expanded[at + 2].clone();
            let merged_text = match (&left.token, &right.token) {
                (ScriptToken::Name { text: first, .. }, ScriptToken::Name { text: second, .. })
                | (ScriptToken::Name { text: first, .. }, ScriptToken::Number { text: second, .. }) => {
                    Some(format!("{first}{second}"))
                }
                (ScriptToken::String { text: first, .. }, ScriptToken::String { text: second, .. }) => Some(format!(
                    "{}{}",
                    first.get(..first.len().saturating_sub(1)).unwrap_or(""),
                    second.get(1..).unwrap_or("")
                )),
                _ => None,
            };
            let Some(merged_text) = merged_text else {
                return Err(self.fail(format!("can't merge {} with {}", left.token.text(), right.token.text())));
            };
            let merged_token = match left.token {
                ScriptToken::Name {
                    location,
                    leading_whitespace,
                    lines_crossed,
                    ..
                } => ScriptToken::Name {
                    text: merged_text.clone(),
                    value: merged_text.clone(),
                    length: merged_text.len(),
                    location,
                    leading_whitespace,
                    lines_crossed,
                },
                ScriptToken::String {
                    location,
                    leading_whitespace,
                    lines_crossed,
                    ..
                } => {
                    let value = merged_text
                        .get(1..merged_text.len().saturating_sub(1))
                        .unwrap_or("")
                        .to_string();
                    ScriptToken::String {
                        text: merged_text.clone(),
                        value,
                        length: merged_text.len(),
                        location,
                        leading_whitespace,
                        lines_crossed,
                    }
                }
                _ => left.token,
            };
            expanded[at].token = merged_token;
            expanded.remove(at + 1);
            expanded.remove(at + 1);
        }
        Ok(expanded)
    }

    fn expand_builtin(&mut self, macro_def: &MacroDef, invocation: &StoredToken) -> Vec<StoredToken> {
        let mut stored = invocation.clone();
        let location = invocation.token.location().clone();
        let (leading, crossed) = (
            invocation.token.leading_whitespace().to_string(),
            invocation.token.lines_crossed(),
        );
        match macro_def.builtin {
            1 => {
                let line = location.line;
                let flags = NumberFlag::DECIMAL | NumberFlag::INTEGER;
                stored.token = ScriptToken::Number {
                    text: line.to_string(),
                    flags,
                    integer_value: line as u32,
                    float_value: line as f64,
                    location,
                    leading_whitespace: leading,
                    lines_crossed: crossed,
                };
                stored.subtype = flags;
                stored.integer_value = line as u32;
                stored.float_value = line as f64;
            }
            2 => {
                let filename = self.current_script_filename();
                let length = filename.len();
                stored.token = ScriptToken::Name {
                    text: filename.clone(),
                    value: filename,
                    length,
                    location,
                    leading_whitespace: leading,
                    lines_crossed: crossed,
                };
                stored.subtype = length as u32;
            }
            3 => {
                let date = (self.now)();
                let months = [
                    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
                ];
                let month = months
                    .get(date.month.wrapping_sub(1) as usize)
                    .copied()
                    .unwrap_or("???");
                let text = format!("\"{month} {:>2} {}\"", date.day, date.year);
                let length = text.len();
                stored.token = ScriptToken::Name {
                    text: text.clone(),
                    value: text,
                    length,
                    location,
                    leading_whitespace: leading,
                    lines_crossed: crossed,
                };
                stored.subtype = length as u32;
            }
            4 => {
                let date = (self.now)();
                let text = format!("\"{:02}:{:02}:{:02}\"", date.hour, date.minute, date.second);
                let length = text.len();
                stored.token = ScriptToken::Name {
                    text: text.clone(),
                    value: text,
                    length,
                    location,
                    leading_whitespace: leading,
                    lines_crossed: crossed,
                };
                stored.subtype = length as u32;
            }
            _ => return Vec::new(),
        }
        vec![stored]
    }

    fn evaluate_directive(&mut self, integer_mode: bool, form: EvalForm) -> Result<(), EngineError> {
        let dollar = form == EvalForm::Dollar;
        let value = self.evaluate_expression(integer_mode, dollar)?;
        let numeric = if integer_mode {
            f64::from(value.integer_value)
        } else {
            value.float_value
        };
        let absolute = numeric.abs();
        let text = if integer_mode {
            absolute.trunc().to_string()
        } else {
            evaluation_decimal(absolute, 2)
        };
        let flags = NumberFlag::DECIMAL
            | NumberFlag::LONG
            | if integer_mode {
                NumberFlag::INTEGER
            } else {
                NumberFlag::FLOAT
            };
        let emitted = self.current_location();
        let (stored_int, stored_float, unsupported) = match form {
            EvalForm::Hash => (
                absolute.trunc() as u32,
                absolute,
                Some("#eval and #evalfloat leave numeric fields uninitialized".to_string()),
            ),
            EvalForm::Dollar => (numeric.trunc().rem_euclid(4_294_967_296.0) as u32, numeric, None),
        };
        let number = StoredToken {
            token: ScriptToken::Number {
                text: text.clone(),
                flags,
                integer_value: absolute.trunc() as u32,
                float_value: absolute,
                location: emitted.clone(),
                leading_whitespace: String::new(),
                lines_crossed: 0,
            },
            subtype: flags,
            integer_value: stored_int,
            float_value: stored_float,
            unsupported,
        };
        self.queued.push_front(number);
        if numeric < 0.0 {
            let sign = StoredToken {
                token: ScriptToken::Punctuation {
                    text: "-".to_string(),
                    value: "-".to_string(),
                    punctuation: Punctuation::Subtract.as_u32(),
                    location: emitted,
                    leading_whitespace: String::new(),
                    lines_crossed: 0,
                },
                subtype: Punctuation::Subtract.as_u32(),
                integer_value: 0,
                float_value: 0.0,
                unsupported: Some("evaluation sign token leaves numeric fields uninitialized".to_string()),
            };
            self.queued.push_front(sign);
        }
        Ok(())
    }

    fn evaluate_expression(&mut self, integer_mode: bool, dollar: bool) -> Result<EvalValue, EngineError> {
        let location = self.current_location();
        if dollar {
            let _ = self.read_source_token()?;
        }
        let first = if dollar {
            self.read_source_token()?
        } else {
            self.read_line_token()?
        };
        let Some(first) = first else {
            let message = if dollar {
                "nothing to evaluate"
            } else {
                "no value after #if/#elif"
            };
            return Err(self.fail(message));
        };
        let mut token = first;
        let mut expression: Vec<StoredToken> = Vec::new();
        let mut defined = false;
        let mut depth = 1i32;
        loop {
            match &token.token {
                ScriptToken::Name { value, .. } => {
                    if defined {
                        defined = false;
                    } else if value == "defined" {
                        defined = true;
                    } else {
                        if !self.macro_defined(value) {
                            return Err(self.fail(format!("can't evaluate {value}, not defined")));
                        }
                        if !self.expand_macro_into_source(&token)? {
                            return Err(EngineError::ReadFalse);
                        }
                        let next = if dollar {
                            self.read_source_token()?
                        } else {
                            self.read_line_token()?
                        };
                        let Some(next) = next else {
                            break;
                        };
                        token = next;
                        continue;
                    }
                }
                ScriptToken::Number { .. } | ScriptToken::Punctuation { .. } => {
                    if dollar {
                        if token.token.text().starts_with('(') {
                            depth += 1;
                        } else if token.token.text().starts_with(')') {
                            depth -= 1;
                        }
                        if depth <= 0 {
                            break;
                        }
                    }
                }
                _ => {
                    let text = token.token.text().to_string();
                    return Err(self.fail(format!("can't evaluate {text}")));
                }
            }
            self.append_expansion(&mut expression, token.clone())?;
            if expression.len() > self.limits.expression_tokens {
                let limit = self.limits.expression_tokens;
                return Err(self.fail(format!("expression exceeds {limit} tokens")));
            }
            let next = if dollar {
                self.read_source_token()?
            } else {
                self.read_line_token()?
            };
            let Some(next) = next else {
                break;
            };
            token = next;
        }
        let (result, events) = {
            let parser = ExpressionParser {
                tokens: &expression,
                integer_mode,
                defined: &|name| self.macro_defined(name),
                fallback: location,
                events: Vec::new(),
            };
            parser.evaluate()
        };
        self.apply_expr_events(events);
        let value = match result {
            Ok(value) => value,
            Err(ExprError::Fail { message, .. }) => return Err(self.fail(message)),
            Err(ExprError::Internal(message)) => return Err(EngineError::Internal(message)),
        };
        if self.debug_eval.is_some() {
            let label = if dollar { "$eval" } else { "eval" };
            self.debug_line(format!("{label}:"));
            for stored in &expression {
                self.debug_line(format!(" {}", stored.token.text()));
            }
            self.debug_line(format!(
                "{label} result: {}",
                debug_evaluation_value(&value, integer_mode)
            ));
        }
        Ok(value)
    }

    fn dollar_directive(&mut self) -> Result<(), EngineError> {
        let name = self.read_source_token()?;
        let stored = match name {
            None => return Err(self.fail("found $ without name")),
            Some(stored) if stored.token.lines_crossed() > 0 => {
                self.queued.push_front(stored);
                return Err(self.fail("found $ at end of line"));
            }
            Some(stored) => stored,
        };
        let integer_mode = matches!(&stored.token, ScriptToken::Name { value, .. } if value == "evalint");
        let float_mode = matches!(&stored.token, ScriptToken::Name { value, .. } if value == "evalfloat");
        if !integer_mode && !float_mode {
            let text = stored.token.text().to_string();
            self.queued.push_front(stored);
            return Err(self.fail(format!("unknown precompiler directive {text}")));
        }
        self.evaluate_directive(integer_mode, EvalForm::Dollar)
    }
}

/// Placeholder for an unreachable lexer state (opening token checked `None`
/// just above); avoids panics while keeping the donor control flow shape.
fn unreachable_stored() -> StoredToken {
    StoredToken {
        token: ScriptToken::Primitive {
            text: String::new(),
            value: String::new(),
            location: SourceLocation {
                path: String::new(),
                line: 1,
                column: 1,
            },
            leading_whitespace: String::new(),
            lines_crossed: 0,
        },
        subtype: 0,
        integer_value: 0,
        float_value: 0.0,
        unsupported: None,
    }
}

/// Parse one `NAME ...` definition line in a throwaway engine (donor
/// `defineFromString`), returning the parsed macro and its diagnostics.
fn parse_global_define(text: &str) -> Result<(Option<MacroDef>, Vec<ScriptDiagnostic>), EngineError> {
    let truncated = text.split('\0').next().unwrap_or("");
    let mut engine = Engine::minimal("*extern", truncated);
    match engine.define_directive() {
        Ok(()) => {
            let define = engine.macro_first();
            Ok((define, engine.reported))
        }
        Err(EngineError::ReadFalse) | Err(EngineError::Fail(_)) => Ok((None, engine.reported)),
        Err(other) => Err(other),
    }
}

impl Engine {
    fn open(
        root: ScriptSource,
        resolver: Box<dyn IncludeResolver>,
        options: ScriptPreprocessorOptions,
    ) -> Result<Engine, ClientError> {
        if root.path.is_empty() {
            return Err(ClientError::BadUi("root source path cannot be empty".to_string()));
        }
        let limits = Limits {
            include_depth: positive_limit(options.max_include_depth, usize::MAX, "maxIncludeDepth")?,
            macro_expansions: positive_limit(options.max_macro_expansions, usize::MAX, "maxMacroExpansions")?,
            queued_tokens: positive_limit(options.max_queued_tokens, usize::MAX, "maxQueuedTokens")?,
            output_tokens: positive_limit(options.max_output_tokens, usize::MAX, "maxOutputTokens")?,
            source_tokens: positive_limit(options.max_source_tokens, usize::MAX, "maxSourceTokens")?,
            defines: positive_limit(options.max_defines, usize::MAX, "maxDefines")?,
            expression_tokens: positive_limit(options.max_expression_tokens, usize::MAX, "maxExpressionTokens")?,
        };
        let mut engine = Engine::minimal(&root.path, &root.text);
        engine.resolver = resolver;
        engine.limits = limits;
        engine.now = options.now.unwrap_or_else(|| Box::new(system_date_time));
        engine.report = options.report;
        engine.debug_eval = options.debug_eval;
        if let Some(globals) = &options.globals {
            for define in globals.copy_to() {
                engine.macro_prepend(define.into_macro());
            }
        }
        if let Some(snapshot) = &options.global_defines {
            for define in &snapshot.definitions {
                engine.macro_prepend(define.clone().into_macro());
            }
        }
        if options.install_builtins {
            engine.install_builtins();
        }
        for definition in &options.initial_defines {
            let (define, diagnostics) = parse_global_define(definition).map_err(|error| error.client_error())?;
            for diagnostic in diagnostics {
                engine.record(diagnostic);
            }
            match define {
                Some(define) => engine.macro_prepend(define),
                None => {
                    let error = engine.fail("initial define is invalid");
                    return Err(error.client_error());
                }
            }
        }
        Ok(engine)
    }

    fn install_builtins(&mut self) {
        for (index, name) in ["__LINE__", "__FILE__", "__DATE__", "__TIME__"].into_iter().enumerate() {
            self.macro_prepend(MacroDef {
                name: name.to_string(),
                fixed: true,
                builtin: (index + 1) as u32,
                params: Vec::new(),
                body: Vec::new(),
            });
        }
    }

    fn add_define_internal(&mut self, text: &str) -> Result<bool, EngineError> {
        let (define, diagnostics) = parse_global_define(text)?;
        for diagnostic in diagnostics {
            self.record(diagnostic);
        }
        match define {
            Some(define) => {
                self.macro_prepend(define);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn expect_token_string(&mut self, text: &str) -> Result<bool, EngineError> {
        let record = self.read_expected()?;
        let Some(record) = record else {
            self.error(format!("couldn't find expected {text}"));
            return Ok(false);
        };
        if record.token.text() != text {
            let found = record.token.text().to_string();
            self.error(format!("expected {text}, found {found}"));
            return Ok(false);
        }
        Ok(true)
    }

    fn expect_any_token(&mut self, output: &mut ScriptTokenRecord) -> Result<bool, EngineError> {
        let record = self.read_expected()?;
        match record {
            Some(record) => {
                *output = record;
                Ok(true)
            }
            None => {
                self.error("couldn't read expected token");
                Ok(false)
            }
        }
    }

    fn expect_token_type(
        &mut self,
        token_type: u32,
        subtype: u32,
        output: &mut ScriptTokenRecord,
    ) -> Result<bool, EngineError> {
        if !self.expect_any_token(output)? {
            return Ok(false);
        }
        if output.token.type_id() != token_type {
            let found = output.token.text().to_string();
            self.error(format!(
                "expected a {}, found {found}",
                script_token_type_name(token_type)
            ));
            return Ok(false);
        }
        if token_type == ScriptTokenType::Number.as_u32() && (output.subtype & subtype) != subtype {
            let name = match script_number_subtype_name(subtype) {
                Ok(name) => name,
                Err(ClientError::BadUi(message)) => return Err(EngineError::Internal(message)),
                Err(error) => return Err(EngineError::External(error)),
            };
            let found = output.token.text().to_string();
            self.error(format!("expected {name}, found {found}"));
            return Ok(false);
        }
        if token_type == ScriptTokenType::Punctuation.as_u32() && output.subtype != subtype {
            let found = output.token.text().to_string();
            self.error(format!("found {found}"));
            return Ok(false);
        }
        Ok(true)
    }

    fn check_token_string(&mut self, text: &str) -> Result<bool, EngineError> {
        let record = self.read_expected()?;
        match record {
            None => Ok(false),
            Some(record) => {
                if record.token.text() == text {
                    Ok(true)
                } else {
                    self.queued.push_front(StoredToken::from_record(&record));
                    Ok(false)
                }
            }
        }
    }

    fn check_token_type(
        &mut self,
        token_type: u32,
        subtype: u32,
        output: &mut ScriptTokenRecord,
    ) -> Result<bool, EngineError> {
        let record = self.read_expected()?;
        match record {
            None => Ok(false),
            Some(record) => {
                if record.token.type_id() == token_type && (record.subtype & subtype) == subtype {
                    *output = record;
                    Ok(true)
                } else {
                    self.queued.push_front(StoredToken::from_record(&record));
                    Ok(false)
                }
            }
        }
    }

    fn skip_until_string(&mut self, text: &str) -> Result<bool, EngineError> {
        while let Some(record) = self.read_expected()? {
            if record.token.text() == text {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn read_line_into(&mut self, output: &mut ScriptTokenRecord) -> Result<bool, EngineError> {
        match self.read_line_token() {
            Ok(Some(stored)) => {
                *output = stored.record();
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(EngineError::Fail(_)) => Ok(false),
            Err(other) => Err(other),
        }
    }

    fn current_record_internal(&mut self) -> Result<ScriptTokenRecord, EngineError> {
        let current = match self.last_output.clone() {
            Some(current) => current,
            None => self.empty_output(),
        };
        if let Some(reason) = &current.unsupported {
            return Err(self.fail(format!("source token profile is unsupported: {reason}")));
        }
        Ok(current.record())
    }

    fn raw_token_internal(&self) -> Result<ScriptTokenRecord, EngineError> {
        let current = match self.last_output.clone() {
            Some(current) => current,
            None => self.empty_output(),
        };
        if let Some(reason) = &current.unsupported {
            // The donor throws `RangeError` here without recording; the error
            // below is likewise not a source failure.
            return Err(EngineError::Internal(format!(
                "source token profile is unsupported: {reason}"
            )));
        }
        Ok(current.record())
    }

    fn empty_output(&self) -> StoredToken {
        let mut empty = unreachable_stored();
        empty.token = ScriptToken::Primitive {
            text: String::new(),
            value: String::new(),
            location: self.current_location(),
            leading_whitespace: String::new(),
            lines_crossed: 0,
        };
        empty
    }

    fn is_source_failure(&self, error: &ClientError) -> bool {
        match error {
            ClientError::BadUi(message) => self.failures.contains(message),
            _ => false,
        }
    }
}

fn positive_limit(value: Option<usize>, fallback: usize, label: &str) -> Result<usize, ClientError> {
    match value {
        None => Ok(fallback),
        Some(limit) if limit >= 1 => Ok(limit),
        _ => Err(ClientError::BadUi(format!("{label} must be a positive integer"))),
    }
}

fn engine_result<T>(result: Result<T, EngineError>) -> Result<T, ClientError> {
    result.map_err(|error| error.client_error())
}

/// Diagnostic observer callback (donor `report` option).
pub type ReportCallback = Box<dyn FnMut(&ScriptDiagnostic)>;

/// Opt-in `DEBUG_EVAL` line observer (donor `debugEval` option).
pub type DebugEvalCallback = Box<dyn FnMut(&str)>;

/// Clock callback for `__DATE__`/`__TIME__` (donor `now` option).
pub type NowCallback = Box<dyn FnMut() -> ScriptDateTime>;

/// Preprocessor options (`ScriptPreprocessorOptions` in the donor).
///
/// The donor `memory` option has no counterpart: the engine owns plain Rust
/// structures. Limit fields are `None` for unlimited (donor default
/// `Number.MAX_SAFE_INTEGER`).
#[derive(Default)]
pub struct ScriptPreprocessorOptions {
    /// `NAME ...` definitions installed before the root source runs.
    pub initial_defines: Vec<String>,
    /// Owned clone of live globals, installed before `global_defines`.
    pub globals: Option<ScriptGlobalDefines>,
    /// Detached global snapshot, installed after `globals`.
    pub global_defines: Option<ScriptGlobalSnapshot>,
    /// Install `__LINE__`, `__FILE__`, `__DATE__` and `__TIME__`.
    pub install_builtins: bool,
    /// Clock for `__DATE__`/`__TIME__`; defaults to current UTC time.
    pub now: Option<NowCallback>,
    /// Maximum live include frames.
    pub max_include_depth: Option<usize>,
    /// Maximum macro expansions.
    pub max_macro_expansions: Option<usize>,
    /// Maximum queued/expansion/expression-list tokens.
    pub max_queued_tokens: Option<usize>,
    /// Maximum output tokens.
    pub max_output_tokens: Option<usize>,
    /// Maximum lexer tokens per source file.
    pub max_source_tokens: Option<usize>,
    /// Maximum defines.
    pub max_defines: Option<usize>,
    /// Maximum `#if`/`#eval` expression tokens.
    pub max_expression_tokens: Option<usize>,
    /// Diagnostic observer, called for every recorded diagnostic.
    pub report: Option<ReportCallback>,
    /// Opt-in `DEBUG_EVAL` line observer, one call per line.
    pub debug_eval: Option<DebugEvalCallback>,
}

/// Owned global define (snapshot counterpart of the sibling `PrecompDefine`).
#[derive(Debug, Clone, PartialEq)]
pub struct GlobalDefine {
    /// Macro name.
    pub name: String,
    /// Builtin/fixed flag; fixed macros cannot be redefined or undefined.
    pub fixed: bool,
    /// Builtin id (1 `__LINE__`, 2 `__FILE__`, 3 `__DATE__`, 4 `__TIME__`).
    pub builtin: u32,
    /// Parameter names for function-like macros.
    pub params: Vec<String>,
    /// Body tokens.
    pub body: Vec<ScriptTokenRecord>,
}

impl GlobalDefine {
    /// Convert an internal macro into owned snapshot data.
    fn from_macro(macro_def: &MacroDef) -> GlobalDefine {
        GlobalDefine {
            name: macro_def.name.clone(),
            fixed: macro_def.fixed,
            builtin: macro_def.builtin,
            params: macro_def.params.clone(),
            body: macro_def.body.iter().map(StoredToken::record).collect(),
        }
    }

    /// Convert snapshot data back into an internal macro.
    ///
    /// Stored `unsupported` markers do not survive the snapshot round trip;
    /// snapshots therefore behave as freshly parsed defines.
    fn into_macro(self) -> MacroDef {
        MacroDef {
            name: self.name,
            fixed: self.fixed,
            builtin: self.builtin,
            params: self.params,
            body: self.body.iter().map(StoredToken::from_record).collect(),
        }
    }
}

/// Detached global snapshot (`ScriptGlobalSnapshot` in the donor).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScriptGlobalSnapshot {
    /// Definitions, newest first.
    pub definitions: Vec<GlobalDefine>,
}

/// Global define owner (`ScriptGlobalDefines` in the donor).
///
/// Save/restore snapshots are omitted with the rest of the donor memory
/// surface; use [`ScriptGlobalDefines::snapshot`] for detached copies.
pub struct ScriptGlobalDefines {
    entries: Vec<MacroDef>,
    reported: Vec<ScriptDiagnostic>,
    report: Option<ReportCallback>,
}

impl ScriptGlobalDefines {
    /// Create an empty owner with an optional diagnostic observer.
    pub fn new(report: Option<ReportCallback>) -> ScriptGlobalDefines {
        ScriptGlobalDefines {
            entries: Vec::new(),
            reported: Vec::new(),
            report,
        }
    }

    /// Recorded diagnostics.
    pub fn diagnostics(&self) -> &[ScriptDiagnostic] {
        &self.reported
    }

    /// Parse and add one `NAME ...` definition line.
    pub fn add(&mut self, text: &str) -> Result<bool, ClientError> {
        let (define, diagnostics) = parse_global_define(text).map_err(|error| error.client_error())?;
        for diagnostic in diagnostics {
            self.reported.push(diagnostic.clone());
            if let Some(report) = self.report.as_mut() {
                report(&diagnostic);
            }
        }
        match define {
            Some(define) => {
                self.entries.push(define);
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Remove the newest define with this name (truncated at NUL).
    ///
    /// The donor frees without unlinking so later traversal reaches a
    /// dangling id; this port drops the entry and traversal skips it.
    pub fn remove(&mut self, name: &str) -> bool {
        let text = name.split('\0').next().unwrap_or("");
        if let Some(position) = self.entries.iter().rposition(|entry| entry.name == text) {
            self.entries.remove(position);
            true
        } else {
            false
        }
    }

    /// Detached snapshot, newest definition first.
    pub fn snapshot(&self) -> ScriptGlobalSnapshot {
        ScriptGlobalSnapshot {
            definitions: self.entries.iter().rev().map(GlobalDefine::from_macro).collect(),
        }
    }

    /// Owned copies for engine installation, newest first (donor `copyTo`).
    pub fn copy_to(&self) -> Vec<GlobalDefine> {
        self.snapshot().definitions
    }

    /// Drop all defines.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Streaming preprocessed source (`ScriptSourceReader` in the donor).
///
/// There is no `next_async`: the donor async read path collapses into sync
/// reads because [`IncludeResolver`] is sync.
pub struct ScriptSourceReader {
    engine: Engine,
}

impl ScriptSourceReader {
    /// Open a root source with an include resolver and options.
    pub fn open(
        root: ScriptSource,
        resolver: impl IncludeResolver + 'static,
        options: ScriptPreprocessorOptions,
    ) -> Result<ScriptSourceReader, ClientError> {
        Ok(ScriptSourceReader {
            engine: Engine::open(root, Box::new(resolver), options)?,
        })
    }

    /// Read the next preprocessed record, or `None` at end of source.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<ScriptTokenRecord>, ClientError> {
        engine_result(self.engine.next_record_internal())
    }

    /// Last record produced by [`ScriptSourceReader::next`].
    ///
    /// Records a failure when the token carries an uninitialized numeric
    /// profile (`#eval` output, stringizing, evaluation signs).
    pub fn current_record(&mut self) -> Result<ScriptTokenRecord, ClientError> {
        engine_result(self.engine.current_record_internal())
    }

    /// Last output token without recording a diagnostic.
    ///
    /// This merges the donor `rawToken` accessor: without a token-memory
    /// sibling it returns the current record, failing (unrecorded) on an
    /// uninitialized numeric profile.
    pub fn raw_token(&self) -> Result<ScriptTokenRecord, ClientError> {
        engine_result(self.engine.raw_token_internal())
    }

    /// Push a record back onto the source queue.
    pub fn unread(&mut self, record: ScriptTokenRecord) {
        self.engine.queued.push_front(StoredToken::from_record(&record));
    }

    /// Push the last output token back onto the source queue.
    pub fn unread_last(&mut self) {
        let stored = self
            .engine
            .last_output
            .clone()
            .unwrap_or_else(|| self.engine.empty_output());
        self.engine.queued.push_front(stored);
    }

    /// Free the source. No-op: `Drop` owns the memory; kept for donor shape.
    pub fn dispose(&mut self) {}

    /// Free the source record only. No-op; kept for donor shape.
    pub fn dispose_record_only(&mut self) {}

    /// Set the include path (a trailing `/` is appended when missing).
    pub fn set_include_path(&mut self, path: &str) {
        if path.ends_with('/') || path.ends_with('\\') {
            self.engine.include_path = path.to_string();
        } else {
            self.engine.include_path = format!("{path}/");
        }
    }

    /// Store punctuation overrides. Like the donor record field, this is
    /// write-only state: the token flow always uses the default table.
    pub fn set_punctuations(&mut self, punctuations: Option<Vec<ScriptPunctuation>>) {
        self.engine.punctuations = punctuations;
    }

    /// Parse and install one `NAME ...` definition line.
    pub fn add_define(&mut self, text: &str) -> Result<bool, ClientError> {
        engine_result(self.engine.add_define_internal(text))
    }

    /// Install the `__LINE__`/`__FILE__`/`__DATE__`/`__TIME__` builtins.
    pub fn add_builtin_defines(&mut self) {
        self.engine.install_builtins();
    }

    /// Write the donor `printDefineHashTable` listing (1024 buckets).
    pub fn print_define_hash_table(&self, write: &mut dyn FnMut(&str)) {
        self.engine.print_define_hash_table(write);
    }

    /// Expect a token with this text, recording on mismatch or end of source.
    pub fn expect_token_string(&mut self, text: &str) -> Result<bool, ClientError> {
        engine_result(self.engine.expect_token_string(text))
    }

    /// Expect any token into `output`, recording at end of source.
    pub fn expect_any_token(&mut self, output: &mut ScriptTokenRecord) -> Result<bool, ClientError> {
        engine_result(self.engine.expect_any_token(output))
    }

    /// Expect a token of this type/subtype into `output`.
    pub fn expect_token_type(
        &mut self,
        token_type: u32,
        subtype: u32,
        output: &mut ScriptTokenRecord,
    ) -> Result<bool, ClientError> {
        engine_result(self.engine.expect_token_type(token_type, subtype, output))
    }

    /// Consume a token with this text, pushing it back on mismatch.
    pub fn check_token_string(&mut self, text: &str) -> Result<bool, ClientError> {
        engine_result(self.engine.check_token_string(text))
    }

    /// Consume a token of this type/subtype, pushing it back on mismatch.
    pub fn check_token_type(
        &mut self,
        token_type: u32,
        subtype: u32,
        output: &mut ScriptTokenRecord,
    ) -> Result<bool, ClientError> {
        engine_result(self.engine.check_token_type(token_type, subtype, output))
    }

    /// Skip tokens until one with this text (consumed) or end of source.
    pub fn skip_until_string(&mut self, text: &str) -> Result<bool, ClientError> {
        engine_result(self.engine.skip_until_string(text))
    }

    /// Read one raw line token into `output` (directives bypassed).
    ///
    /// Returns `Ok(false)` at end of line/source or on a source failure, in
    /// which case `output` is left untouched (the donor copies a partial
    /// token there).
    pub fn read_line_into(&mut self, output: &mut ScriptTokenRecord) -> Result<bool, ClientError> {
        engine_result(self.engine.read_line_into(output))
    }

    /// Whether this error is a recorded source failure from this reader.
    pub fn is_source_failure(&self, error: &ClientError) -> bool {
        self.engine.is_source_failure(error)
    }

    /// Current position: root filename plus current line.
    pub fn position(&self) -> ScriptSourcePosition {
        ScriptSourcePosition {
            filename: self.engine.root_path.clone(),
            line: self.engine.current_location().line,
        }
    }

    /// Path of the file currently being read (root or include).
    pub fn current_script_filename(&self) -> String {
        self.engine.current_script_filename()
    }

    /// Recorded diagnostics.
    pub fn diagnostics(&self) -> &[ScriptDiagnostic] {
        &self.engine.reported
    }
}

/// Batch preprocessor (`ScriptPreprocessor` in the donor).
///
/// `create` drains the whole source up front; [`ScriptPreprocessor::next`]
/// then replays the collected tokens infallibly.
pub struct ScriptPreprocessor {
    tokens: Vec<ScriptToken>,
    reported: Vec<ScriptDiagnostic>,
    index: usize,
    unread_token: Option<ScriptToken>,
}

impl ScriptPreprocessor {
    /// Preprocess a root source completely.
    pub fn create(
        root: ScriptSource,
        resolver: impl IncludeResolver + 'static,
        options: ScriptPreprocessorOptions,
    ) -> Result<ScriptPreprocessor, ClientError> {
        let mut engine = Engine::open(root, Box::new(resolver), options)?;
        let mut tokens = Vec::new();
        loop {
            match engine.next_token_internal() {
                Ok(None) => break,
                Ok(Some(stored)) => tokens.push(stored.token),
                Err(error) => return Err(error.client_error()),
            }
        }
        Ok(ScriptPreprocessor {
            tokens,
            reported: engine.reported,
            index: 0,
            unread_token: None,
        })
    }

    /// Free the source. No-op: `Drop` owns the memory; kept for donor shape.
    pub fn dispose(&mut self) {}

    /// Diagnostics recorded while preprocessing.
    pub fn diagnostics(&self) -> &[ScriptDiagnostic] {
        &self.reported
    }

    /// Replay the next token, or `None` at the end.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<ScriptToken> {
        if let Some(token) = self.unread_token.take() {
            return Some(token);
        }
        if self.index < self.tokens.len() {
            let token = self.tokens[self.index].clone();
            self.index += 1;
            Some(token)
        } else {
            None
        }
    }

    /// Push one token back; fails when one is already held.
    pub fn unread(&mut self, token: ScriptToken) -> Result<(), ClientError> {
        if self.unread_token.is_some() {
            return Err(ClientError::BadUi(
                "ScriptPreprocessor can only unread one token".to_string(),
            ));
        }
        self.unread_token = Some(token);
        Ok(())
    }

    /// Rewind replay to the first token.
    pub fn reset(&mut self) {
        self.index = 0;
        self.unread_token = None;
    }

    /// All preprocessed tokens.
    pub fn all(&self) -> &[ScriptToken] {
        &self.tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn root(text: &str) -> ScriptSource {
        ScriptSource {
            path: "root.menu".to_string(),
            text: text.to_string(),
        }
    }

    fn expand_texts(text: &str) -> Vec<String> {
        let preprocessor =
            ScriptPreprocessor::create(root(text), NullIncludeResolver, ScriptPreprocessorOptions::default())
                .expect("preprocess");
        preprocessor
            .all()
            .iter()
            .map(|token| token.text().to_string())
            .collect()
    }

    fn expand_records(text: &str) -> Result<Vec<ScriptTokenRecord>, ClientError> {
        let mut reader =
            ScriptSourceReader::open(root(text), NullIncludeResolver, ScriptPreprocessorOptions::default())?;
        let mut records = Vec::new();
        while let Some(record) = reader.next()? {
            records.push(record);
        }
        Ok(records)
    }

    fn record_texts(records: &[ScriptTokenRecord]) -> Vec<String> {
        records.iter().map(|record| record.token.text().to_string()).collect()
    }

    fn fixed_options() -> ScriptPreprocessorOptions {
        ScriptPreprocessorOptions {
            install_builtins: true,
            now: Some(Box::new(|| ScriptDateTime {
                year: 2026,
                month: 1,
                day: 5,
                hour: 3,
                minute: 4,
                second: 7,
            })),
            ..ScriptPreprocessorOptions::default()
        }
    }

    #[test]
    fn plain_tokens_and_comments() {
        assert_eq!(
            expand_texts("itemDef { name \"x\" } // trailing\n/* block */ visible 1"),
            vec!["itemDef", "{", "name", "\"x\"", "}", "visible", "1"]
        );
    }

    #[test]
    fn object_like_define() {
        assert_eq!(expand_texts("#define FOO bar\nFOO"), vec!["bar"]);
    }

    #[test]
    fn function_like_define() {
        assert_eq!(expand_texts("#define ADD(a, b) a + b\nADD(1, 2)"), vec!["1", "+", "2"]);
    }

    #[test]
    fn function_like_empty_parens_call() {
        // `F()` with one parameter warns but still expands with an empty argument.
        let mut reader = ScriptSourceReader::open(
            root("#define F(a) a x\nF()"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let mut texts = Vec::new();
        while let Some(record) = reader.next().expect("next") {
            texts.push(record.token.text().to_string());
        }
        assert_eq!(texts, vec!["x"]);
        assert!(reader
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message == "too few define parms"));
    }

    #[test]
    fn stringizing_produces_quoted_token() {
        let preprocessor = ScriptPreprocessor::create(
            root("#define STR(x) #x\nSTR(hello)"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("preprocess");
        assert_eq!(preprocessor.all().len(), 1);
        match &preprocessor.all()[0] {
            ScriptToken::String {
                text, value, length, ..
            } => {
                assert_eq!(text, "\"hello\"");
                assert_eq!(value, "hello");
                assert_eq!(*length, 7);
            }
            other => panic!("expected string, got {other:?}"),
        }
        // The streaming reader rejects the uninitialized numeric profile.
        let mut reader = ScriptSourceReader::open(
            root("#define STR(x) #x\nSTR(hello)"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = reader.next().expect_err("stringized read must fail");
        assert_eq!(
            error,
            ClientError::BadUi(
                "root.menu:2:11: source token profile is unsupported: macro stringizing leaves subtype and numeric fields uninitialized".to_string()
            )
        );
        assert!(reader.is_source_failure(&error));
    }

    #[test]
    fn token_merge_names_numbers_strings() {
        assert_eq!(expand_texts("#define CAT(a, b) a ## b\nCAT(foo, bar)"), vec!["foobar"]);
        assert_eq!(expand_texts("#define CAT(a, b) a ## b\nCAT(foo, 12)"), vec!["foo12"]);
        assert_eq!(
            expand_texts("#define CAT(a, b) a ## b\nCAT(\"a\", \"b\")"),
            vec!["\"ab\""]
        );
    }

    #[test]
    fn token_merge_mismatch_fails() {
        let mut reader = ScriptSourceReader::open(
            root("#define CAT(a, b) a ## b\nCAT(;, ;)"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = reader.next().expect_err("merge must fail");
        assert!(matches!(error, ClientError::BadUi(ref message) if message.ends_with("can't merge ; with ;")));
    }

    #[test]
    fn conditionals_select_branches() {
        assert_eq!(
            expand_texts("#define FOO 1\n#ifdef FOO\nyes\n#else\nno\n#endif"),
            vec!["yes"]
        );
        assert_eq!(expand_texts("#ifdef MISSING\nyes\n#else\nno\n#endif"), vec!["no"]);
        assert_eq!(expand_texts("#ifndef MISSING\ntaken\n#endif"), vec!["taken"]);
        assert_eq!(
            expand_texts("#if 1\none\n#elif 1\ntwo\n#else\nthree\n#endif"),
            vec!["one", "two"]
        );
    }

    #[test]
    fn if_expressions() {
        assert_eq!(expand_texts("#if 1 + 2 * 3 == 7\nok\n#endif"), vec!["ok"]);
        assert_eq!(
            expand_texts("#define V 2\n#if V == 2 && defined(V)\nok\n#endif"),
            vec!["ok"]
        );
        assert_eq!(
            expand_texts("#if (10 % 3 == 1) && (7 / 2 == 3)\nok\n#endif"),
            vec!["ok"]
        );
        assert_eq!(expand_texts("#if (1 << 4) == 16\nok\n#endif"), vec!["ok"]);
        assert_eq!(expand_texts("#if 0 ? 1 : 2 == 2\nok\n#endif"), vec!["ok"]);
        assert_eq!(expand_texts("#if 0\nbad\n#else\ngood\n#endif"), vec!["good"]);
        assert!(expand_texts("#if 0\nbad\n#endif").is_empty());
    }

    #[test]
    fn eval_emits_numbers() {
        assert_eq!(expand_texts("#eval 1 + 2"), vec!["3"]);
        assert_eq!(expand_texts("#eval 0 - 5"), vec!["-", "5"]);
        assert_eq!(expand_texts("#evalfloat 1.5 + 1.5"), vec!["3.00"]);
        // Streaming reads reject the uninitialized `#eval` profile.
        let mut reader = ScriptSourceReader::open(
            root("#eval 1 + 2"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = reader.next().expect_err("eval read must fail");
        assert!(
            matches!(error, ClientError::BadUi(ref message) if message.ends_with("source token profile is unsupported: #eval and #evalfloat leave numeric fields uninitialized"))
        );
        assert!(reader.is_source_failure(&error));
    }

    #[test]
    fn builtins_expand_with_fixed_clock() {
        let preprocessor =
            ScriptPreprocessor::create(root("__LINE__"), NullIncludeResolver, fixed_options()).expect("preprocess");
        assert_eq!(preprocessor.all().len(), 1);
        match &preprocessor.all()[0] {
            ScriptToken::Number {
                text,
                flags,
                integer_value,
                float_value,
                ..
            } => {
                assert_eq!(text, "1");
                assert_eq!(*flags, NumberFlag::DECIMAL | NumberFlag::INTEGER);
                assert_eq!(*integer_value, 1);
                assert_eq!(*float_value, 1.0);
            }
            other => panic!("expected number, got {other:?}"),
        }
        let mut second = ScriptPreprocessor::create(
            root("__FILE__\n__DATE__\n__TIME__"),
            NullIncludeResolver,
            fixed_options(),
        )
        .expect("preprocess");
        let texts: Vec<String> = {
            let mut out = Vec::new();
            while let Some(token) = second.next() {
                out.push(token.text().to_string());
            }
            out
        };
        assert_eq!(texts, vec!["root.menu", "\"Jan  5 2026\"", "\"03:04:07\""]);
    }

    #[test]
    fn builtins_are_fixed() {
        let mut reader =
            ScriptSourceReader::open(root("#define __LINE__ 9"), NullIncludeResolver, fixed_options()).expect("open");
        let error = reader.next().expect_err("redefine must fail");
        assert!(matches!(error, ClientError::BadUi(ref message) if message.ends_with("can't redefine __LINE__")));
        let mut undef =
            ScriptSourceReader::open(root("#undef __FILE__\nok"), NullIncludeResolver, fixed_options()).expect("open");
        let records = {
            let mut out = Vec::new();
            while let Some(record) = undef.next().expect("next") {
                out.push(record);
            }
            out
        };
        assert_eq!(record_texts(&records), vec!["ok"]);
        assert!(undef
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message == "can't undef __FILE__"));
    }

    #[test]
    fn undef_and_redefine() {
        assert_eq!(expand_texts("#define A 1\n#undef A\nA"), vec!["A"]);
        let mut reader = ScriptSourceReader::open(
            root("#define A 1\n#define A 2\nA"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let mut texts = Vec::new();
        while let Some(record) = reader.next().expect("next") {
            texts.push(record.token.text().to_string());
        }
        assert_eq!(texts, vec!["2"]);
        assert!(reader
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message == "redefinition of A"));
    }

    #[test]
    fn empty_define_ends_stream() {
        // Donor quirk: an empty expansion returns undefined from the read.
        let mut reader = ScriptSourceReader::open(
            root("#define EMPTY\nEMPTY\nx"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        assert!(reader.next().expect("next").is_none());
    }

    #[test]
    fn quoted_include_splices_tokens() {
        let files = Rc::new(RefCell::new(HashMap::from([(
            "lib.cfg".to_string(),
            "from_lib".to_string(),
        )])));
        let captured: Rc<RefCell<Vec<IncludeRequest>>> = Rc::new(RefCell::new(Vec::new()));
        let captured_clone = captured.clone();
        let mut reader = ScriptSourceReader::open(
            root("before\n#include \"lib.cfg\"\nafter"),
            move |request: &IncludeRequest| {
                captured_clone.borrow_mut().push(request.clone());
                Ok(files.borrow().get(&request.requested_path).map(|text| ScriptSource {
                    path: request.requested_path.clone(),
                    text: text.clone(),
                }))
            },
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let mut texts = Vec::new();
        let mut filenames = Vec::new();
        while let Some(record) = reader.next().expect("next") {
            texts.push(record.token.text().to_string());
            filenames.push(reader.current_script_filename());
        }
        assert_eq!(texts, vec!["before", "from_lib", "after"]);
        assert_eq!(filenames, vec!["root.menu", "lib.cfg", "root.menu"]);
        assert_eq!(
            reader.position(),
            ScriptSourcePosition {
                filename: "root.menu".to_string(),
                line: 3
            }
        );
        let requests = captured.borrow();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].kind, IncludeKind::Quoted);
        assert_eq!(requests[0].from_path, "root.menu");
        assert_eq!(requests[0].requested_path, "lib.cfg");
        assert_eq!(requests[0].include_path, None);
    }

    #[test]
    fn system_include_and_missing_file() {
        let mut reader = ScriptSourceReader::open(
            root("#include <sys/lib.h>"),
            |request: &IncludeRequest| {
                assert_eq!(request.kind, IncludeKind::System);
                assert_eq!(request.requested_path, "sys/lib.h");
                Ok(None)
            },
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = reader.next().expect_err("missing include must fail");
        assert_eq!(
            error,
            ClientError::BadUi("root.menu:1:21: file sys/lib.h not found".to_string())
        );
        assert!(reader.is_source_failure(&error));
    }

    #[test]
    fn recursive_include_records_error_and_continues() {
        let mut reader = ScriptSourceReader::open(
            root("top\n#include \"root.menu\"\nbottom"),
            |request: &IncludeRequest| {
                Ok(Some(ScriptSource {
                    path: request.requested_path.clone(),
                    text: "inner".to_string(),
                }))
            },
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let mut texts = Vec::new();
        while let Some(record) = reader.next().expect("next") {
            texts.push(record.token.text().to_string());
        }
        assert_eq!(texts, vec!["top", "bottom"]);
        assert!(reader
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message == "root.menu recursively included"));
    }

    #[test]
    fn error_directive_and_unknowns() {
        let mut reader = ScriptSourceReader::open(
            root("#error boom"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = reader.next().expect_err("error directive must fail");
        assert_eq!(
            error,
            ClientError::BadUi("root.menu:1:12: #error directive: boom".to_string())
        );
        let mut unknown = ScriptSourceReader::open(
            root("#bogus"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = unknown.next().expect_err("unknown directive must fail");
        assert!(
            matches!(error, ClientError::BadUi(ref message) if message.ends_with("unknown precompiler directive bogus"))
        );
        let mut endif_alone = ScriptSourceReader::open(
            root("#endif"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = endif_alone.next().expect_err("misplaced endif must fail");
        assert!(matches!(error, ClientError::BadUi(ref message) if message.ends_with("misplaced #endif")));
        let mut line = ScriptSourceReader::open(
            root("#line 5"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = line.next().expect_err("line directive must fail");
        assert!(matches!(error, ClientError::BadUi(ref message) if message.ends_with("#line directive not supported")));
        let mut dollar = ScriptSourceReader::open(
            root("$bogus"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = dollar.next().expect_err("dollar directive must fail");
        assert!(
            matches!(error, ClientError::BadUi(ref message) if message.ends_with("unknown precompiler directive bogus"))
        );
    }

    #[test]
    fn dollar_eval_forms() {
        assert_eq!(expand_texts("$evalint(1 + 2 * 3)"), vec!["7"]);
        assert_eq!(expand_texts("$evalfloat(1.5 + 1.5)"), vec!["3.00"]);
        assert_eq!(expand_texts("$evalint((1 + 2) * 3)"), vec!["9"]);
        assert_eq!(expand_texts("$evalint(0 - 5)"), vec!["-", "5"]);
        // Unlike `#eval`, dollar numbers carry initialized numeric fields,
        // so streaming reads succeed; the emitted sign stays unsupported.
        let records = expand_records("$evalint(1 + 2)").expect("records");
        assert_eq!(record_texts(&records), vec!["3"]);
        let mut signed = ScriptSourceReader::open(
            root("$evalint(0 - 5)"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = signed.next().expect_err("dollar sign must fail");
        assert!(
            matches!(error, ClientError::BadUi(ref message) if message.ends_with("source token profile is unsupported: evaluation sign token leaves numeric fields uninitialized"))
        );
    }

    #[test]
    fn expression_errors() {
        let mut divide = ScriptSourceReader::open(
            root("#if 1 / 0\nx\n#endif"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = divide.next().expect_err("divide by zero must fail");
        assert!(matches!(error, ClientError::BadUi(ref message) if message.ends_with("divide by zero in #if/#elif\n")));
        let mut unknown = ScriptSourceReader::open(
            root("#if MISSING\nx\n#endif"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = unknown.next().expect_err("unknown name must fail");
        assert!(
            matches!(error, ClientError::BadUi(ref message) if message.ends_with("can't evaluate MISSING, not defined"))
        );
        let mut trailing = ScriptSourceReader::open(
            root("#if 1 +\n#endif"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = trailing.next().expect_err("trailing operator must fail");
        assert!(
            matches!(error, ClientError::BadUi(ref message) if message.ends_with("trailing operator in #if/#elif"))
        );
    }

    #[test]
    fn too_many_arguments_fails() {
        let mut reader = ScriptSourceReader::open(
            root("#define F(a) a\nF(1, 2)"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = reader.next().expect_err("extra argument must fail");
        assert!(matches!(error, ClientError::BadUi(ref message) if message.ends_with("define F has too many parms")));
        assert!(reader.is_source_failure(&error));
    }

    #[test]
    fn limits_are_enforced() {
        let options = ScriptPreprocessorOptions {
            max_output_tokens: Some(2),
            ..ScriptPreprocessorOptions::default()
        };
        let mut reader = ScriptSourceReader::open(root("a b c"), NullIncludeResolver, options).expect("open");
        assert!(reader.next().expect("a").is_some());
        assert!(reader.next().expect("b").is_some());
        let error = reader.next().expect_err("output limit must fail");
        assert!(
            matches!(error, ClientError::BadUi(ref message) if message.ends_with("preprocessor output exceeds 2 tokens"))
        );

        let options = ScriptPreprocessorOptions {
            max_macro_expansions: Some(2),
            ..ScriptPreprocessorOptions::default()
        };
        let mut reader = ScriptSourceReader::open(
            root("#define A B\n#define B C\n#define C d\nA"),
            NullIncludeResolver,
            options,
        )
        .expect("open");
        let error = reader.next().expect_err("expansion limit must fail");
        assert!(
            matches!(error, ClientError::BadUi(ref message) if message.ends_with("macro expansion count exceeds 2"))
        );

        let options = ScriptPreprocessorOptions {
            max_defines: Some(1),
            ..ScriptPreprocessorOptions::default()
        };
        let mut reader =
            ScriptSourceReader::open(root("#define A 1\n#define B 2"), NullIncludeResolver, options).expect("open");
        let error = reader.next().expect_err("define limit must fail");
        assert!(matches!(error, ClientError::BadUi(ref message) if message.ends_with("define count exceeds 1")));

        let zeroed = ScriptPreprocessorOptions {
            max_include_depth: Some(0),
            ..ScriptPreprocessorOptions::default()
        };
        let bad = match ScriptSourceReader::open(root("a"), NullIncludeResolver, zeroed) {
            Err(error) => error,
            Ok(_) => panic!("zero limit must fail"),
        };
        assert_eq!(
            bad,
            ClientError::BadUi("maxIncludeDepth must be a positive integer".to_string())
        );
    }

    #[test]
    fn include_depth_limit() {
        let options = ScriptPreprocessorOptions {
            max_include_depth: Some(1),
            ..ScriptPreprocessorOptions::default()
        };
        let mut reader = ScriptSourceReader::open(
            root("#include \"a.h\""),
            |_: &IncludeRequest| {
                Ok(Some(ScriptSource {
                    path: "a.h".to_string(),
                    text: "x".to_string(),
                }))
            },
            options,
        )
        .expect("open");
        let error = reader.next().expect_err("depth limit must fail");
        assert!(matches!(error, ClientError::BadUi(ref message) if message.ends_with("include depth exceeds 1")));
    }

    #[test]
    fn numbers_and_strings() {
        let records = expand_records("0x10 010 0b101 10L 1.5 \"a\\n\" 'lit'").expect("records");
        assert_eq!(
            record_texts(&records),
            vec!["0x10", "010", "0b101", "10", "1.5", "\"a\n\"", "'lit'"]
        );
        assert_eq!(records[0].integer_value, 16);
        assert_eq!(records[0].subtype, NumberFlag::HEX | NumberFlag::INTEGER);
        assert_eq!(records[1].integer_value, 8);
        assert_eq!(records[1].subtype, NumberFlag::OCTAL | NumberFlag::INTEGER);
        assert_eq!(records[2].integer_value, 5);
        assert_eq!(
            records[3].subtype,
            NumberFlag::DECIMAL | NumberFlag::LONG | NumberFlag::INTEGER
        );
        assert_eq!(records[3].integer_value, 10);
        assert_eq!(records[4].float_value, 1.5);
        match &records[5].token {
            ScriptToken::String { value, .. } => assert_eq!(value, "a\n"),
            other => panic!("expected string, got {other:?}"),
        }
        // Lexer-level concatenation joins adjacent strings.
        assert_eq!(expand_texts("\"a\" \"b\""), vec!["\"ab\""]);
        // Preprocessor-level merge joins strings across a directive.
        assert_eq!(expand_texts("\"a\"\n#define X 1\n\"b\""), vec!["\"ab\""]);
    }

    #[test]
    fn merged_string_overflow_fails() {
        let half = "x".repeat(511);
        let text = format!("\"{half}\"\n#define X 1\n\"{half}\"");
        let mut reader =
            ScriptSourceReader::open(root(&text), NullIncludeResolver, ScriptPreprocessorOptions::default())
                .expect("open");
        let error = reader.next().expect_err("overflow must fail");
        assert!(
            matches!(error, ClientError::BadUi(ref message) if message.ends_with("string longer than MAX_TOKEN 1024\n"))
        );
    }

    #[test]
    fn trailing_dot_number_is_internal_error() {
        // Donor quirk: `1.` consumes a phantom NUL digit and goes negative.
        let mut reader =
            ScriptSourceReader::open(root("1."), NullIncludeResolver, ScriptPreprocessorOptions::default())
                .expect("open");
        let error = reader.next().expect_err("1. must fail");
        assert_eq!(
            error,
            ClientError::BadUi("NumberValue floating conversion exceeds the source unsigned-long range".to_string())
        );
        assert!(!reader.is_source_failure(&error));
    }

    #[test]
    fn expect_check_skip_and_line_reads() {
        let mut reader = ScriptSourceReader::open(
            root("alpha 42 ;"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        assert!(!reader.check_token_string("beta").expect("check miss"));
        assert!(reader.check_token_string("alpha").expect("check hit"));
        let mut output = reader.raw_token().expect("raw");
        assert!(reader
            .expect_token_type(ScriptTokenType::Number.as_u32(), NumberFlag::INTEGER, &mut output)
            .expect("expect number"));
        assert_eq!(output.token.text(), "42");
        assert!(!reader.expect_token_string(";x").expect("expect miss"));
        assert_eq!(
            reader.diagnostics().last().map(|diagnostic| diagnostic.message.clone()),
            Some("expected ;x, found ;".to_string())
        );

        // A missed expect consumes the token, so skipping runs on fresh input.
        let mut skipper =
            ScriptSourceReader::open(root("a ; b"), NullIncludeResolver, ScriptPreprocessorOptions::default())
                .expect("open");
        assert!(skipper.skip_until_string(";").expect("skip"));
        let after = skipper.next().expect("next").expect("record");
        assert_eq!(after.token.text(), "b");
        assert!(!skipper.skip_until_string(";").expect("skip eof"));

        let mut lines = ScriptSourceReader::open(
            root("one two\nthree"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let mut output = lines.raw_token().expect("raw");
        assert!(lines.read_line_into(&mut output).expect("line one"));
        assert_eq!(output.token.text(), "one");
        assert!(lines.read_line_into(&mut output).expect("line two"));
        assert_eq!(output.token.text(), "two");
        assert!(!lines.read_line_into(&mut output).expect("line end"));
    }

    #[test]
    fn unread_round_trip() {
        let mut reader =
            ScriptSourceReader::open(root("a b"), NullIncludeResolver, ScriptPreprocessorOptions::default())
                .expect("open");
        let first = reader.next().expect("next").expect("record");
        reader.unread(first);
        let again = reader.next().expect("next").expect("record");
        assert_eq!(again.token.text(), "a");
        reader.unread_last();
        let replayed = reader.next().expect("next").expect("record");
        assert_eq!(replayed.token.text(), "a");
    }

    #[test]
    fn hash_table_lists_defines() {
        let mut reader = ScriptSourceReader::open(
            root("#define FOO 1\n#define BAR 2\nx"),
            NullIncludeResolver,
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        assert!(reader.next().expect("next").is_some());
        let mut listing = String::new();
        reader.print_define_hash_table(&mut |piece| listing.push_str(piece));
        assert_eq!(listing.lines().count(), 1024);
        for name in ["FOO", "BAR"] {
            let bucket = define_bucket(name);
            assert!(
                listing.contains(&format!("{bucket:>4}: {name}")),
                "missing {name} in bucket {bucket}"
            );
        }
    }

    #[test]
    fn debug_eval_lines_match_donor() {
        let lines: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let capture = lines.clone();
        let options = ScriptPreprocessorOptions {
            debug_eval: Some(Box::new(move |text: &str| capture.borrow_mut().push(text.to_string()))),
            ..ScriptPreprocessorOptions::default()
        };
        let mut reader = ScriptSourceReader::open(root("#eval 1 + 2"), NullIncludeResolver, options).expect("open");
        let _ = reader.next();
        assert_eq!(
            *lines.borrow(),
            vec![
                "operator +, value1 = 1".to_string(),
                "value2 = 2".to_string(),
                "result value = 3".to_string(),
                "eval:".to_string(),
                " 1".to_string(),
                " +".to_string(),
                " 2".to_string(),
                "eval result: 3".to_string(),
            ]
        );
    }

    #[test]
    fn globals_install_and_snapshot() {
        let mut globals = ScriptGlobalDefines::new(None);
        assert!(globals.add("GVAL 41").expect("add"));
        assert!(!globals.add("#define BAD").expect("add invalid"));
        let snapshot = globals.snapshot();
        assert_eq!(snapshot.definitions.len(), 1);
        assert_eq!(snapshot.definitions[0].name, "GVAL");
        let options = ScriptPreprocessorOptions {
            globals: Some(globals),
            ..ScriptPreprocessorOptions::default()
        };
        let mut reader = ScriptSourceReader::open(root("GVAL"), NullIncludeResolver, options).expect("open");
        let record = reader.next().expect("next").expect("record");
        assert_eq!(record.token.text(), "41");

        let mut owned = ScriptGlobalDefines::new(None);
        assert!(owned.add("ONE 1").expect("add"));
        assert!(owned.add("TWO 2").expect("add"));
        assert!(owned.remove("TWO"));
        assert!(!owned.remove("TWO"));
        let snapshot = owned.snapshot();
        assert_eq!(snapshot.definitions.len(), 1);
        assert_eq!(snapshot.definitions[0].name, "ONE");
        owned.clear();
        assert!(owned.snapshot().definitions.is_empty());
    }

    #[test]
    fn initial_defines_install() {
        let options = ScriptPreprocessorOptions {
            initial_defines: vec!["SEED 7".to_string()],
            ..ScriptPreprocessorOptions::default()
        };
        let preprocessor = ScriptPreprocessor::create(root("SEED"), NullIncludeResolver, options).expect("create");
        assert_eq!(preprocessor.all().len(), 1);
        assert_eq!(preprocessor.all()[0].text(), "7");
    }

    #[test]
    fn resolver_errors_are_not_source_failures() {
        let mut reader = ScriptSourceReader::open(
            root("#include \"a.h\""),
            |_: &IncludeRequest| Err(ClientError::BadUi("boom".to_string())),
            ScriptPreprocessorOptions::default(),
        )
        .expect("open");
        let error = reader.next().expect_err("resolver error must propagate");
        assert_eq!(error, ClientError::BadUi("boom".to_string()));
        assert!(!reader.is_source_failure(&error));
        assert!(reader.diagnostics().is_empty());
    }

    #[test]
    fn preprocessor_replay() {
        let mut preprocessor =
            ScriptPreprocessor::create(root("a b"), NullIncludeResolver, ScriptPreprocessorOptions::default())
                .expect("create");
        assert_eq!(preprocessor.all().len(), 2);
        let first = preprocessor.next().expect("first");
        assert_eq!(first.text(), "a");
        preprocessor.unread(first).expect("unread");
        assert_eq!(preprocessor.next().expect("replay").text(), "a");
        preprocessor.reset();
        assert_eq!(preprocessor.next().expect("restart").text(), "a");
        let held = preprocessor.next().expect("held");
        preprocessor.unread(held.clone()).expect("unread");
        assert!(preprocessor.unread(held).is_err());
    }

    #[test]
    fn evaluation_decimal_matches_donor() {
        assert_eq!(evaluation_decimal(3.0, 2), "3.00");
        assert_eq!(evaluation_decimal(1.5, 2), "1.50");
        assert_eq!(evaluation_decimal(0.125, 2), "0.12");
        assert_eq!(evaluation_decimal(0.375, 2), "0.38");
        assert_eq!(evaluation_decimal(f64::INFINITY, 2), "inf");
        assert!(evaluation_decimal(f64::NAN, 2) == "nan");
        assert_eq!(evaluation_decimal(1e300, 2).len(), 304);
        assert!(evaluation_decimal(1e300, 2).ends_with(".00"));
    }
}
