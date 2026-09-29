//! Retained `token_t` storage for legacy menu scripts.
//!
//! Donor provenance: `src/ui/common/legacy/script/token-memory.ts`
//! (`token_t` from id Software's `botlib/l_script.h` and `l_script.c`).
//!
//! A token occupies exactly [`SOURCE_TOKEN_BYTES`] little-endian bytes
//! (Linux i386 layout):
//!
//! | Range     | Field            | Type                |
//! |-----------|------------------|---------------------|
//! | 0..1024   | string bytes     | NUL-terminated Latin-1 |
//! | 1024..1028 | type            | `i32`               |
//! | 1028..1032 | subtype         | `i32`               |
//! | 1032..1036 | integer value   | `u32`               |
//! | 1036..1046 | float value     | x87 80-bit extended |
//! | 1046..1048 | ABI padding     | untouched           |
//! | 1048..1052 | whitespace start | `u32`              |
//! | 1052..1056 | whitespace end  | `u32`               |
//! | 1056..1060 | line            | `i32`               |
//! | 1060..1064 | lines crossed   | `i32`               |
//! | 1064..1068 | next token id   | `u32`               |
//!
//! # Adaptations
//!
//! * The donor borrow closure (`() => Uint8Array`) becomes shared ownership:
//!   [`SourceTokenMemory`] views a [`SharedScriptBytes`] store through a
//!   base/len window. Standalone tokens ([`SourceTokenMemory::new`]) own a
//!   fresh zeroed window; [`SourceTokenMemory::view`] shares an allocation
//!   so token writes are visible through the owning allocation and back,
//!   matching the donor aliasing.
//! * The donor's global `WeakMap` snapshot, keyed by token object identity,
//!   has no value-semantics equivalent. The snapshot is instance-local and
//!   write-back matches on token value; [`SourceTokenMemory::copy_from`]
//!   carries the snapshot along like the donor, so the lexer's
//!   push-back/retained round trip keeps its whitespace and link words.
//! * `SaveReader` checkpoints become the typed [`SourceTokenSaveState`].
//!   Every donor `RangeError` becomes [`ClientError::BadUi`] carrying the
//!   donor message; save-envelope validations keep their `script.token: `
//!   path prefix, matching `SaveReader` roots.
//! * The donor file performs no asynchronous reads, so this port injects no
//!   callbacks.
//! * [`ScriptToken`], [`ScriptTokenRecord`], and [`SourceLocation`] are owned
//!   by `super::lexer`, matching the donor's imports.

use std::cell::RefCell;
use std::rc::Rc;

use super::lexer::{ScriptToken, ScriptTokenRecord, SourceLocation};
use crate::error::ClientError;

/// Size of one retained `token_t` record, in bytes (donor `SOURCE_TOKEN_BYTES`).
pub const SOURCE_TOKEN_BYTES: usize = 1068;

/// Length of the inline string field, in bytes.
const STRING_BYTES: usize = 1024;
/// Offset of the `i32` token type word.
const TYPE: usize = 1024;
/// Offset of the `i32` token subtype word.
const SUBTYPE: usize = 1028;
/// Offset of the `u32` integer value word.
const INTEGER: usize = 1032;
/// Offset of the 10-byte x87 extended float field.
const FLOAT: usize = 1036;
/// Offset of the `u32` whitespace-start pointer word.
const WHITESPACE: usize = 1048;
/// Offset of the `u32` whitespace-end pointer word.
const END_WHITESPACE: usize = 1052;
/// Offset of the `i32` line word.
const LINE: usize = 1056;
/// Offset of the `i32` lines-crossed word.
const LINES_CROSSED: usize = 1060;
/// Offset of the `u32` next-token link word.
const NEXT: usize = 1064;

/// Shared backing store for retained script bytes.
///
/// Donor `Uint8Array` values have reference identity: a token view and its
/// owning allocation observe each other's writes. This store provides that
/// identity; [`SourceTokenMemory`] and the precompiler allocation mirrors
/// hold clones of one store for the same underlying bytes.
#[derive(Debug, Clone)]
pub(crate) struct SharedScriptBytes {
    inner: Rc<RefCell<Vec<u8>>>,
}

impl SharedScriptBytes {
    /// Allocate a zeroed store of `len` bytes.
    pub(crate) fn zeroed(len: usize) -> Self {
        Self {
            inner: Rc::new(RefCell::new(vec![0; len])),
        }
    }

    /// Wrap an existing byte vector.
    pub(crate) fn from_vec(bytes: Vec<u8>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(bytes)),
        }
    }

    /// Run `read` against the live bytes.
    pub(crate) fn with_bytes<R>(&self, read: impl FnOnce(&[u8]) -> Result<R, ClientError>) -> Result<R, ClientError> {
        let borrowed = self
            .inner
            .try_borrow()
            .map_err(|_| ClientError::BadUi("script token storage is already borrowed".to_string()))?;
        read(&borrowed)
    }

    /// Run `write` against the live bytes.
    pub(crate) fn with_bytes_mut<R>(
        &self,
        write: impl FnOnce(&mut [u8]) -> Result<R, ClientError>,
    ) -> Result<R, ClientError> {
        let mut borrowed = self
            .inner
            .try_borrow_mut()
            .map_err(|_| ClientError::BadUi("script token storage is already borrowed".to_string()))?;
        write(&mut borrowed)
    }
}

/// File position and leading trivia for decoding one retained token
/// (donor `SourceTokenContext`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTokenContext {
    /// Source path carried into decoded token locations.
    pub path: String,
    /// 1-based column carried into decoded token locations.
    pub column: i32,
    /// Leading whitespace text carried into decoded tokens.
    pub leading_whitespace: String,
}






/// Checkpoint of one retained token (donor `captureSaveState` record).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTokenSaveState {
    /// Full 1068-byte record image, including padding and link words.
    pub bytes: Vec<u8>,
    /// Typed text extent preserving escaped NUL payloads, if any.
    pub text_extent: Option<usize>,
}

/// Last decoded record plus the byte image it was decoded from.
#[derive(Debug, Clone)]
struct TokenSnapshot {
    /// Byte image at decode time.
    bytes: [u8; SOURCE_TOKEN_BYTES],
    /// Typed text extent at decode time.
    text_extent: Option<usize>,
    /// Decoded record.
    record: ScriptTokenRecord,
    /// Context the record was decoded with.
    context: SourceTokenContext,
}

/// Retained `token_t` storage: a view over [`SharedScriptBytes`] with no
/// allocation ownership of its own (donor `SourceTokenMemory`).
#[derive(Debug, Clone)]
pub struct SourceTokenMemory {
    store: SharedScriptBytes,
    base: usize,
    len: usize,
    text_extent: Option<usize>,
    snapshot: Option<TokenSnapshot>,
}

/// Donor `tokenType`: type word for a token kind.
fn token_type_of(token: &ScriptToken) -> i32 {
    match token {
        ScriptToken::Primitive { .. } => 0,
        ScriptToken::String { .. } => 1,
        ScriptToken::Literal { .. } => 2,
        ScriptToken::Number { .. } => 3,
        ScriptToken::Name { .. } => 4,
        ScriptToken::Punctuation { .. } => 5,
    }
}

/// Donor `tokenSubtype`: default subtype word for a fresh token.
fn token_subtype_of(token: &ScriptToken) -> i32 {
    match token {
        ScriptToken::Primitive { .. } => 0,
        ScriptToken::String { length, .. } | ScriptToken::Literal { length, .. } | ScriptToken::Name { length, .. } => {
            *length as i32
        }
        ScriptToken::Number { flags, .. } => *flags as i32,
        ScriptToken::Punctuation { punctuation, .. } => *punctuation as i32,
    }
}

/// Donor `text.slice(1, -1)`: strip one surrounding quote pair.
fn strip_quotes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 2 {
        return String::new();
    }
    chars[1..chars.len() - 1].iter().collect()
}

/// Span validation failure (donor `bytes` getter range error).
fn span_error() -> ClientError {
    ClientError::BadUi("token_t requires exactly 1068 source bytes".to_string())
}

/// Read a little-endian `i32` word from a validated span.
fn read_i32(span: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([span[offset], span[offset + 1], span[offset + 2], span[offset + 3]])
}

/// Read a little-endian `u32` word from a validated span.
fn read_u32(span: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([span[offset], span[offset + 1], span[offset + 2], span[offset + 3]])
}

/// Write a little-endian `i32` word into a validated span.
fn write_i32(span: &mut [u8], offset: usize, value: i32) {
    span[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Write a little-endian `u32` word into a validated span.
fn write_u32(span: &mut [u8], offset: usize, value: u32) {
    span[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Encode a binary64 value as a 10-byte x87 extended field (donor `writeFloat`).
fn encode_f64_extended(value: f64) -> [u8; 10] {
    let bits = value.to_bits();
    let sign = ((bits >> 63) as u16) << 15;
    let exponent = ((bits >> 52) & 0x7ff) as u16;
    let fraction = bits & 0x000f_ffff_ffff_ffff;
    let (significand, extended_exponent) = if exponent == 0 && fraction == 0 {
        (0_u64, 0_u16)
    } else if exponent == 0 {
        let highest_bit = 63 - fraction.leading_zeros();
        let significand = fraction << (63 - highest_bit);
        let extended = highest_bit as i32 - 1074 + 16383;
        (significand, extended as u16)
    } else {
        let significand = (fraction | 0x0010_0000_0000_0000) << 11;
        let extended = if exponent == 0x7ff {
            0x7fff
        } else {
            i32::from(exponent) - 1023 + 16383
        };
        (significand, extended as u16)
    };
    let mut field = [0_u8; 10];
    field[0..8].copy_from_slice(&significand.to_le_bytes());
    field[8..10].copy_from_slice(&(sign | extended_exponent).to_le_bytes());
    field
}

/// Decode a 10-byte x87 extended field into the supported binary64 profile
/// (donor `readFloat`).
fn decode_f64_extended(field: &[u8; 10]) -> Result<f64, ClientError> {
    let mut significand_bytes = [0_u8; 8];
    significand_bytes.copy_from_slice(&field[0..8]);
    let significand = u64::from_le_bytes(significand_bytes);
    let word = u16::from_le_bytes([field[8], field[9]]);
    let exponent = word & 0x7fff;
    let negative = word & 0x8000 != 0;
    if exponent == 0 && significand == 0 {
        return Ok(if negative { -0.0 } else { 0.0 });
    }
    if exponent == 0x7fff && significand == 0x8000_0000_0000_0000 {
        return Ok(if negative { f64::NEG_INFINITY } else { f64::INFINITY });
    }
    if exponent == 0x7fff && significand > 0x8000_0000_0000_0000 {
        return Ok(f64::NAN);
    }
    let power = i32::from(exponent) - 16383;
    let fraction = significand as f64 / 9_223_372_036_854_775_808.0;
    let magnitude = if power < -1022 {
        fraction * 2f64.powi(power + 1022) * 2f64.powi(-1022)
    } else {
        fraction * 2f64.powi(power)
    };
    let value = if negative { -magnitude } else { magnitude };
    if encode_f64_extended(value) != *field {
        return Err(ClientError::BadUi(
            "token_t long double exceeds the supported binary64 numeric profile".to_string(),
        ));
    }
    Ok(value)
}

impl SourceTokenMemory {
    /// Create a standalone zeroed token (donor constructor over fresh bytes).
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: SharedScriptBytes::zeroed(SOURCE_TOKEN_BYTES),
            base: 0,
            len: SOURCE_TOKEN_BYTES,
            text_extent: None,
            snapshot: None,
        }
    }

    /// Create a standalone token from a full record image.
    #[must_use]
    pub fn from_bytes(bytes: [u8; SOURCE_TOKEN_BYTES]) -> Self {
        Self {
            store: SharedScriptBytes::from_vec(bytes.to_vec()),
            base: 0,
            len: SOURCE_TOKEN_BYTES,
            text_extent: None,
            snapshot: None,
        }
    }

    /// View `len` bytes at `base` of shared storage (donor constructor over
    /// a borrowed array or subarray). The window is validated lazily on
    /// every access, like the donor `bytes` getter.
    pub(crate) fn view(store: &SharedScriptBytes, base: usize, len: usize) -> Self {
        Self {
            store: store.clone(),
            base,
            len,
            text_extent: None,
            snapshot: None,
        }
    }

    /// Copy of the live record bytes (donor `bytes` getter).
    pub fn bytes(&self) -> Result<Vec<u8>, ClientError> {
        self.with_span(|span| Ok(span.to_vec()))
    }

    /// Zero every byte and drop the text extent and snapshot (donor `clear`).
    pub fn clear(&mut self) -> Result<(), ClientError> {
        self.with_span_mut(|span| {
            span.fill(0);
            Ok(())
        })?;
        self.text_extent = None;
        self.snapshot = None;
        Ok(())
    }

    /// Copy bytes, text extent, and snapshot from another token, retaining
    /// padding, pointer words, and bytes after NUL (donor `copyFrom`).
    pub fn copy_from(&mut self, other: &Self) -> Result<(), ClientError> {
        let bytes = other.span_copy()?;
        self.set_span(&bytes)?;
        self.text_extent = other.text_extent;
        self.snapshot = other.snapshot.clone();
        Ok(())
    }

    /// Token type word (donor `type` getter).
    pub fn token_type(&self) -> Result<i32, ClientError> {
        self.with_span(|span| Ok(read_i32(span, TYPE)))
    }

    /// Set the token type word (donor `type` setter).
    pub fn set_token_type(&mut self, value: i32) -> Result<(), ClientError> {
        self.with_span_mut(|span| {
            write_i32(span, TYPE, value);
            Ok(())
        })
    }

    /// Token subtype word (donor `subtype` getter).
    pub fn subtype(&self) -> Result<i32, ClientError> {
        self.with_span(|span| Ok(read_i32(span, SUBTYPE)))
    }

    /// Set the token subtype word (donor `subtype` setter).
    pub fn set_subtype(&mut self, value: i32) -> Result<(), ClientError> {
        self.with_span_mut(|span| {
            write_i32(span, SUBTYPE, value);
            Ok(())
        })
    }

    /// Integer value word (donor `integerValue` getter).
    pub fn integer_value(&self) -> Result<u32, ClientError> {
        self.with_span(|span| Ok(read_u32(span, INTEGER)))
    }

    /// Set the integer value word (donor `integerValue` setter).
    pub fn set_integer_value(&mut self, value: u32) -> Result<(), ClientError> {
        self.with_span_mut(|span| {
            write_u32(span, INTEGER, value);
            Ok(())
        })
    }

    /// Float value in the supported binary64 profile
    /// (donor `floatValue` getter).
    pub fn float_value(&self) -> Result<f64, ClientError> {
        self.with_span(|span| {
            let mut field = [0_u8; 10];
            field.copy_from_slice(&span[FLOAT..FLOAT + 10]);
            decode_f64_extended(&field)
        })
    }

    /// Encode a binary64 value into the x87 field, leaving the two ABI
    /// padding bytes untouched (donor `floatValue` setter).
    pub fn set_float_value(&mut self, value: f64) -> Result<(), ClientError> {
        let field = encode_f64_extended(value);
        self.with_span_mut(|span| {
            span[FLOAT..FLOAT + 10].copy_from_slice(&field);
            Ok(())
        })
    }

    /// Whitespace-start pointer word (donor `whitespaceStart` getter).
    pub fn whitespace_start(&self) -> Result<u32, ClientError> {
        self.with_span(|span| Ok(read_u32(span, WHITESPACE)))
    }

    /// Set the whitespace-start pointer word (donor `whitespaceStart` setter).
    pub fn set_whitespace_start(&mut self, value: u32) -> Result<(), ClientError> {
        self.with_span_mut(|span| {
            write_u32(span, WHITESPACE, value);
            Ok(())
        })
    }

    /// Whitespace-end pointer word (donor `whitespaceEnd` getter).
    pub fn whitespace_end(&self) -> Result<u32, ClientError> {
        self.with_span(|span| Ok(read_u32(span, END_WHITESPACE)))
    }

    /// Set the whitespace-end pointer word (donor `whitespaceEnd` setter).
    pub fn set_whitespace_end(&mut self, value: u32) -> Result<(), ClientError> {
        self.with_span_mut(|span| {
            write_u32(span, END_WHITESPACE, value);
            Ok(())
        })
    }

    /// Line word (donor `line` getter).
    pub fn line(&self) -> Result<i32, ClientError> {
        self.with_span(|span| Ok(read_i32(span, LINE)))
    }

    /// Set the line word (donor `line` setter).
    pub fn set_line(&mut self, value: i32) -> Result<(), ClientError> {
        self.with_span_mut(|span| {
            write_i32(span, LINE, value);
            Ok(())
        })
    }

    /// Lines-crossed word (donor `linesCrossed` getter).
    pub fn lines_crossed(&self) -> Result<i32, ClientError> {
        self.with_span(|span| Ok(read_i32(span, LINES_CROSSED)))
    }

    /// Set the lines-crossed word (donor `linesCrossed` setter).
    pub fn set_lines_crossed(&mut self, value: i32) -> Result<(), ClientError> {
        self.with_span_mut(|span| {
            write_i32(span, LINES_CROSSED, value);
            Ok(())
        })
    }

    /// Whether any whitespace was skipped before the token: the end pointer
    /// is past the start pointer (donor `whitespaceBefore` getter).
    pub fn whitespace_before(&self) -> Result<bool, ClientError> {
        self.with_span(|span| Ok(read_u32(span, END_WHITESPACE) > read_u32(span, WHITESPACE)))
    }

    /// Clear the whitespace pointers and lines crossed
    /// (donor `clearWhitespace`).
    pub fn clear_whitespace(&mut self) -> Result<(), ClientError> {
        self.with_span_mut(|span| {
            write_u32(span, WHITESPACE, 0);
            write_u32(span, END_WHITESPACE, 0);
            write_i32(span, LINES_CROSSED, 0);
            Ok(())
        })
    }

    /// Next-token link word (donor `next` getter).
    pub fn next(&self) -> Result<u32, ClientError> {
        self.with_span(|span| Ok(read_u32(span, NEXT)))
    }

    /// Set the next-token link word (donor `next` setter).
    pub fn set_next(&mut self, value: u32) -> Result<(), ClientError> {
        self.with_span_mut(|span| {
            write_u32(span, NEXT, value);
            Ok(())
        })
    }

    /// NUL-terminated Latin-1 string field (donor `string` getter).
    pub fn string(&self) -> Result<String, ClientError> {
        self.with_span(|span| {
            let mut text = String::new();
            for byte in span.iter().take(STRING_BYTES) {
                if *byte == 0 {
                    return Ok(text);
                }
                text.push(*byte as char);
            }
            Err(ClientError::BadUi(
                "token_t string lacks a terminator within its 1024-byte field".to_string(),
            ))
        })
    }

    /// Write a NUL-terminated string, recording a typed text extent when it
    /// carries escaped NUL payloads (donor `writeString`).
    pub fn write_string(&mut self, text: &str) -> Result<(), ClientError> {
        let units: Vec<u16> = text.encode_utf16().collect();
        if units.len() >= STRING_BYTES {
            return Err(ClientError::BadUi(
                "token_t string exceeds its 1024-byte field".to_string(),
            ));
        }
        for unit in &units {
            if *unit > 255 {
                return Err(ClientError::BadUi("token_t string requires source bytes".to_string()));
            }
        }
        self.with_span_mut(|span| {
            for (index, unit) in units.iter().enumerate() {
                span[index] = *unit as u8;
            }
            span[units.len()] = 0;
            Ok(())
        })?;
        self.text_extent = if text.contains('\0') { Some(units.len()) } else { None };
        Ok(())
    }

    /// Write one string-field byte (donor `setStringByte`).
    pub fn set_string_byte(&mut self, index: usize, value: u8) -> Result<(), ClientError> {
        if index >= STRING_BYTES {
            return Err(ClientError::BadUi(
                "token_t string byte is outside its field".to_string(),
            ));
        }
        self.with_span_mut(|span| {
            span[index] = value;
            Ok(())
        })
    }

    /// Write a whole record, restoring the snapshotted byte image (padding,
    /// pointer words, bytes after NUL) when this token previously decoded
    /// the same token value, else clearing first (donor `writeRecord`).
    pub fn write_record(&mut self, record: &ScriptTokenRecord) -> Result<(), ClientError> {
        let cached = match &self.snapshot {
            Some(previous) if previous.record.token == record.token => Some((previous.bytes, previous.text_extent)),
            _ => None,
        };
        match cached {
            Some((bytes, extent)) => {
                self.set_span(&bytes)?;
                self.text_extent = extent;
            }
            None => self.clear()?,
        }
        self.write_string(record.token.text())?;
        self.set_token_type(token_type_of(&record.token))?;
        self.set_subtype(record.subtype as i32)?;
        self.set_integer_value(record.integer_value)?;
        self.set_float_value(record.float_value)?;
        self.set_line(record.token.location().line as i32)?;
        self.set_lines_crossed(record.token.lines_crossed() as i32)?;
        Ok(())
    }

    /// Write a token, reusing its snapshotted record when this token
    /// previously decoded the same token value (donor `writeToken`).
    pub fn write_token(&mut self, token: &ScriptToken) -> Result<(), ClientError> {
        let cached = match &self.snapshot {
            Some(previous) if previous.record.token == *token => Some(previous.record.clone()),
            _ => None,
        };
        if let Some(record) = cached {
            return self.write_record(&record);
        }
        let (integer_value, float_value) = match token {
            ScriptToken::Number {
                integer_value,
                float_value,
                ..
            } => (*integer_value, *float_value),
            _ => (0, 0.0),
        };
        let record = ScriptTokenRecord {
            token: token.clone(),
            subtype: token_subtype_of(token) as u32,
            integer_value,
            float_value,
        };
        self.write_record(&record)
    }

    /// Decode the retained bytes into a typed record (donor `readRecord`).
    ///
    /// `text_extent` carries the typed text length for payloads with escaped
    /// NULs; when it exceeds the NUL-truncated length it is retained. A
    /// repeated decode with an unchanged context and byte image returns the
    /// snapshotted record.
    pub fn read_record(
        &mut self,
        context: &SourceTokenContext,
        text_extent: Option<usize>,
    ) -> Result<ScriptTokenRecord, ClientError> {
        if let Some(extent) = text_extent {
            if extent >= STRING_BYTES {
                return Err(ClientError::BadUi(
                    "typed token text extent exceeds its source string field".to_string(),
                ));
            }
            let truncated = self.string()?.chars().count();
            self.text_extent = if truncated < extent { Some(extent) } else { None };
        }
        if let Some(previous) = &self.snapshot {
            if previous.context == *context && self.snapshot_matches(previous)? {
                return Ok(previous.record.clone());
            }
        }
        let mut text = self.string()?;
        if let Some(extent) = self.text_extent {
            if extent > text.chars().count() {
                if extent >= STRING_BYTES {
                    return Err(ClientError::BadUi(
                        "typed token text extent exceeds its source string field".to_string(),
                    ));
                }
                text = self.with_span(|span| {
                    let mut rebuilt = String::new();
                    for index in 0..extent {
                        let byte = span.get(index).copied().ok_or_else(|| {
                            ClientError::BadUi("typed token text exceeds its source allocation".to_string())
                        })?;
                        rebuilt.push(byte as char);
                    }
                    Ok(rebuilt)
                })?;
            }
        }
        let location = SourceLocation {
            path: context.path.clone(),
            line: self.line()? as usize,
            column: context.column as usize,
        };
        let leading_whitespace = context.leading_whitespace.clone();
        let lines_crossed = self.lines_crossed()? as usize;
        let subtype = self.subtype()?;
        let integer_value = self.integer_value()?;
        let float_value = self.float_value()?;
        let token = match self.token_type()? {
            0 => ScriptToken::Primitive {
                text: text.clone(),
                value: text.clone(),
                location: location.clone(),
                leading_whitespace: leading_whitespace.clone(),
                lines_crossed,
            },
            1 => ScriptToken::String {
                text: text.clone(),
                value: strip_quotes(&text),
                length: text.chars().count(),
                location: location.clone(),
                leading_whitespace: leading_whitespace.clone(),
                lines_crossed,
            },
            2 => ScriptToken::Literal {
                text: text.clone(),
                value: strip_quotes(&text),
                length: text.chars().count(),
                location: location.clone(),
                leading_whitespace: leading_whitespace.clone(),
                lines_crossed,
            },
            3 => ScriptToken::Number {
                text: text.clone(),
                flags: subtype as u32,
                integer_value,
                float_value,
                location: location.clone(),
                leading_whitespace: leading_whitespace.clone(),
                lines_crossed,
            },
            4 => ScriptToken::Name {
                text: text.clone(),
                value: text.clone(),
                length: text.chars().count(),
                location: location.clone(),
                leading_whitespace: leading_whitespace.clone(),
                lines_crossed,
            },
            5 => ScriptToken::Punctuation {
                text: text.clone(),
                value: text.clone(),
                punctuation: subtype as u32,
                location: location.clone(),
                leading_whitespace: leading_whitespace.clone(),
                lines_crossed,
            },
            _ => {
                return Err(ClientError::BadUi("token_t has no completed typed token".to_string()));
            }
        };
        let record = ScriptTokenRecord {
            token,
            subtype: subtype as u32,
            integer_value,
            float_value,
        };
        let bytes = self.span_copy()?;
        self.snapshot = Some(TokenSnapshot {
            bytes,
            text_extent: self.text_extent,
            record: record.clone(),
            context: context.clone(),
        });
        Ok(record)
    }

    /// Capture a checkpoint of the record image and text extent
    /// (donor `captureSaveState`).
    pub fn capture_save_state(&self) -> Result<SourceTokenSaveState, ClientError> {
        Ok(SourceTokenSaveState {
            bytes: self.bytes()?,
            text_extent: self.text_extent,
        })
    }

    /// Restore a checkpoint. With `verify_bytes`, the stored image is
    /// compared against the live bytes instead of being written back, so the
    /// caller can confirm the owning allocation was restored underneath
    /// (donor `restoreSaveState`).
    pub fn restore_save_state(&mut self, state: &SourceTokenSaveState, verify_bytes: bool) -> Result<(), ClientError> {
        if state.bytes.len() != SOURCE_TOKEN_BYTES || state.text_extent.is_some_and(|extent| extent > STRING_BYTES) {
            return Err(ClientError::BadUi("script.token: invalid token extent".to_string()));
        }
        if verify_bytes {
            let current = self.bytes()?;
            if current != state.bytes {
                return Err(ClientError::BadUi(
                    "script.token: token bytes disagree with restored allocation".to_string(),
                ));
            }
        } else {
            self.with_span_mut(|span| {
                span.copy_from_slice(&state.bytes);
                Ok(())
            })?;
        }
        self.text_extent = state.text_extent;
        self.snapshot = None;
        Ok(())
    }

    /// Run `read` against the validated record window.
    fn with_span<R>(&self, read: impl FnOnce(&[u8]) -> Result<R, ClientError>) -> Result<R, ClientError> {
        if self.len != SOURCE_TOKEN_BYTES {
            return Err(span_error());
        }
        self.store.with_bytes(|bytes| {
            let end = self.base.checked_add(SOURCE_TOKEN_BYTES).ok_or_else(span_error)?;
            let span = bytes.get(self.base..end).ok_or_else(span_error)?;
            read(span)
        })
    }

    /// Run `write` against the validated record window.
    fn with_span_mut<R>(&mut self, write: impl FnOnce(&mut [u8]) -> Result<R, ClientError>) -> Result<R, ClientError> {
        if self.len != SOURCE_TOKEN_BYTES {
            return Err(span_error());
        }
        let (base, len) = (self.base, self.len);
        self.store.with_bytes_mut(|bytes| {
            let end = base.checked_add(len).ok_or_else(span_error)?;
            let span = bytes.get_mut(base..end).ok_or_else(span_error)?;
            write(span)
        })
    }

    /// Copy the validated record window.
    fn span_copy(&self) -> Result<[u8; SOURCE_TOKEN_BYTES], ClientError> {
        self.with_span(|span| {
            let mut bytes = [0_u8; SOURCE_TOKEN_BYTES];
            bytes.copy_from_slice(span);
            Ok(bytes)
        })
    }

    /// Overwrite the validated record window.
    fn set_span(&mut self, bytes: &[u8; SOURCE_TOKEN_BYTES]) -> Result<(), ClientError> {
        self.with_span_mut(|span| {
            span.copy_from_slice(bytes);
            Ok(())
        })
    }

    /// Whether the live window still matches a snapshot (donor `sameValue`).
    fn snapshot_matches(&self, snapshot: &TokenSnapshot) -> Result<bool, ClientError> {
        if self.text_extent != snapshot.text_extent {
            return Ok(false);
        }
        self.with_span(|span| Ok(span == snapshot.bytes.as_slice()))
    }
}

impl Default for SourceTokenMemory {
    /// Standalone zeroed token.
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Context fixture: path, column, and leading whitespace travel into
    /// decoded token locations.
    fn context() -> SourceTokenContext {
        SourceTokenContext {
            path: "maps/test.cfg".to_string(),
            column: 7,
            leading_whitespace: "  ".to_string(),
        }
    }

    /// Token base fixture at the given line.
    fn base(text: &str, line: i32) -> TokenBase {
        TokenBase {
            text: text.to_string(),
            location: SourceLocation {
                path: "maps/test.cfg".to_string(),
                line,
                column: 7,
            },
            leading_whitespace: "  ".to_string(),
            lines_crossed: 2,
        }
    }

    /// Assert a `BadUi` message.
    fn assert_bad_ui(result: Result<(), ClientError>, message: &str) {
        match result {
            Err(ClientError::BadUi(actual)) => assert_eq!(actual, message),
            other => panic!("expected BadUi({message:?}), got {other:?}"),
        }
    }

    #[test]
    fn token_bytes_is_1068() {
        assert_eq!(SOURCE_TOKEN_BYTES, 1068);
        let token = SourceTokenMemory::new();
        assert_eq!(token.bytes().unwrap().len(), SOURCE_TOKEN_BYTES);
    }

    #[test]
    fn fresh_token_is_zeroed() {
        let token = SourceTokenMemory::new();
        assert_eq!(token.token_type().unwrap(), 0);
        assert_eq!(token.subtype().unwrap(), 0);
        assert_eq!(token.integer_value().unwrap(), 0);
        assert_eq!(token.float_value().unwrap(), 0.0);
        assert_eq!(token.whitespace_start().unwrap(), 0);
        assert_eq!(token.whitespace_end().unwrap(), 0);
        assert_eq!(token.line().unwrap(), 0);
        assert_eq!(token.lines_crossed().unwrap(), 0);
        assert_eq!(token.next().unwrap(), 0);
        assert!(!token.whitespace_before().unwrap());
        assert_eq!(token.string().unwrap(), "");
        assert!(token.bytes().unwrap().iter().all(|byte| *byte == 0));
    }

    #[test]
    fn typed_fields_round_trip() {
        let mut token = SourceTokenMemory::new();
        token.set_token_type(5).unwrap();
        token.set_subtype(-3).unwrap();
        token.set_integer_value(0xdead_beef).unwrap();
        token.set_whitespace_start(100).unwrap();
        token.set_whitespace_end(140).unwrap();
        token.set_line(42).unwrap();
        token.set_lines_crossed(3).unwrap();
        token.set_next(9).unwrap();
        assert_eq!(token.token_type().unwrap(), 5);
        assert_eq!(token.subtype().unwrap(), -3);
        assert_eq!(token.integer_value().unwrap(), 0xdead_beef);
        assert_eq!(token.whitespace_start().unwrap(), 100);
        assert_eq!(token.whitespace_end().unwrap(), 140);
        assert_eq!(token.line().unwrap(), 42);
        assert_eq!(token.lines_crossed().unwrap(), 3);
        assert_eq!(token.next().unwrap(), 9);
        assert!(token.whitespace_before().unwrap());
        token.clear_whitespace().unwrap();
        assert_eq!(token.whitespace_start().unwrap(), 0);
        assert_eq!(token.whitespace_end().unwrap(), 0);
        assert_eq!(token.lines_crossed().unwrap(), 0);
        assert!(!token.whitespace_before().unwrap());
    }

    #[test]
    fn whitespace_before_compares_pointers() {
        let mut token = SourceTokenMemory::new();
        token.set_whitespace_start(50).unwrap();
        token.set_whitespace_end(50).unwrap();
        assert!(!token.whitespace_before().unwrap());
        // A wrapped subtraction would report whitespace here; the donor
        // compares in float64, so an end before the start means none.
        token.set_whitespace_start(60).unwrap();
        token.set_whitespace_end(50).unwrap();
        assert!(!token.whitespace_before().unwrap());
    }

    #[test]
    fn string_write_and_read() {
        let mut token = SourceTokenMemory::new();
        token.write_string("hello").unwrap();
        assert_eq!(token.string().unwrap(), "hello");
        let bytes = token.bytes().unwrap();
        assert_eq!(&bytes[0..6], b"hello\0");
        // Latin-1 bytes above 127 survive the round trip.
        token.write_string("caf\u{e9}").unwrap();
        assert_eq!(token.string().unwrap(), "caf\u{e9}");
        assert_eq!(token.bytes().unwrap()[3], 0xe9);
    }

    #[test]
    fn string_rejects_long_and_wide_text() {
        let mut token = SourceTokenMemory::new();
        let longest = "x".repeat(1023);
        token.write_string(&longest).unwrap();
        assert_eq!(token.string().unwrap(), longest);
        let too_long = "x".repeat(1024);
        assert_bad_ui(
            token.write_string(&too_long),
            "token_t string exceeds its 1024-byte field",
        );
        assert_bad_ui(
            token.write_string("price \u{20ac}"),
            "token_t string requires source bytes",
        );
        assert_bad_ui(token.write_string("\u{1f600}"), "token_t string requires source bytes");
    }

    #[test]
    fn set_string_byte_bounds() {
        let mut token = SourceTokenMemory::new();
        token.set_string_byte(0, b'A').unwrap();
        token.set_string_byte(1, 0).unwrap();
        token.set_string_byte(1023, b'z').unwrap();
        assert_eq!(token.string().unwrap(), "A");
        assert_bad_ui(
            token.set_string_byte(1024, 0),
            "token_t string byte is outside its field",
        );
    }

    #[test]
    fn string_requires_terminator() {
        let mut token = SourceTokenMemory::new();
        for index in 0..1024 {
            token.set_string_byte(index, b'q').unwrap();
        }
        match token.string() {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "token_t string lacks a terminator within its 1024-byte field")
            }
            other => panic!("expected missing-terminator error, got {other:?}"),
        }
    }

    #[test]
    fn float_round_trips() {
        let mut token = SourceTokenMemory::new();
        for value in [
            0.0,
            -0.0,
            1.0,
            -1.5,
            123_456.789,
            5e-324,
            f64::MIN_POSITIVE,
            2.225_073_858_507_201_4e-308,
            1e308,
            f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ] {
            token.set_float_value(value).unwrap();
            assert_eq!(
                token.float_value().unwrap().to_bits(),
                value.to_bits(),
                "float round trip for {value}"
            );
        }
        token.set_float_value(f64::NAN).unwrap();
        assert!(token.float_value().unwrap().is_nan());
        // The two ABI padding bytes after the 10-byte field stay untouched.
        let mut token = SourceTokenMemory::new();
        token.set_float_value(1.5).unwrap();
        let bytes = token.bytes().unwrap();
        assert_eq!(&bytes[1046..1048], &[0, 0]);
    }

    #[test]
    fn float_rejects_non_binary64_long_double() {
        // Unnormalized significand: decodes to 0.5, which re-encodes to a
        // different image, so the profile check must fail.
        let mut bytes = [0_u8; SOURCE_TOKEN_BYTES];
        bytes[1036..1044].copy_from_slice(&0x4000_0000_0000_0000_u64.to_le_bytes());
        bytes[1044..1046].copy_from_slice(&0x3fff_u16.to_le_bytes());
        let token = SourceTokenMemory::from_bytes(bytes);
        match token.float_value() {
            Err(ClientError::BadUi(message)) => assert_eq!(
                message,
                "token_t long double exceeds the supported binary64 numeric profile"
            ),
            other => panic!("expected profile error, got {other:?}"),
        }
    }

    #[test]
    fn records_round_trip_all_kinds() {
        let cases: Vec<(ScriptToken, i32, u32, f64)> = vec![
            (
                ScriptToken::Primitive {
                    base: base("primitive-word", 11),
                    value: "primitive-word".to_string(),
                },
                0,
                0,
                0.0,
            ),
            (
                ScriptToken::String {
                    base: base("\"hi\"", 12),
                    value: "hi".to_string(),
                    length: 4,
                },
                4,
                0,
                0.0,
            ),
            (
                ScriptToken::Literal {
                    base: base("'a'", 13),
                    value: "a".to_string(),
                    length: 3,
                },
                3,
                0,
                0.0,
            ),
            (
                ScriptToken::Number {
                    base: base("0x10", 14),
                    flags: 0x1100,
                    integer_value: 16,
                    float_value: 16.0,
                },
                0x1100,
                16,
                16.0,
            ),
            (
                ScriptToken::Name {
                    base: base("someName", 15),
                    value: "someName".to_string(),
                    length: 8,
                },
                8,
                0,
                0.0,
            ),
            (
                ScriptToken::Punctuation {
                    base: base("==", 16),
                    value: "==".to_string(),
                    punctuation: 9,
                },
                9,
                0,
                0.0,
            ),
        ];
        for (token, subtype, integer_value, float_value) in cases {
            let expected_line = token.base().location.line;
            let record = ScriptTokenRecord {
                token: token.clone(),
                subtype,
                integer_value,
                float_value,
            };
            let mut memory = SourceTokenMemory::new();
            memory.write_record(&record).unwrap();
            let decoded = memory.read_record(&context(), None).unwrap();
            assert_eq!(decoded.subtype, subtype, "subtype for {}", token.kind_name());
            assert_eq!(decoded.integer_value, integer_value);
            assert_eq!(decoded.float_value.to_bits(), float_value.to_bits());
            assert_eq!(decoded.token, token, "token for {}", token.kind_name());
            // Positions: the line travels with the record while the path and
            // column come from the decode context.
            assert_eq!(decoded.token.base().location.line, expected_line);
            assert_eq!(decoded.token.base().location.path, "maps/test.cfg");
            assert_eq!(decoded.token.base().location.column, 7);
            assert_eq!(decoded.token.base().leading_whitespace, "  ");
            assert_eq!(decoded.token.base().lines_crossed, 2);
        }
    }

    #[test]
    fn write_token_derives_fresh_record() {
        let mut memory = SourceTokenMemory::new();
        let number = ScriptToken::Number {
            base: base("12.5", 3),
            flags: 0x0808,
            integer_value: 12,
            float_value: 12.5,
        };
        memory.write_token(&number).unwrap();
        let decoded = memory.read_record(&context(), None).unwrap();
        assert_eq!(decoded.subtype, 0x0808);
        assert_eq!(decoded.integer_value, 12);
        assert_eq!(decoded.float_value, 12.5);

        let mut memory = SourceTokenMemory::new();
        let name = ScriptToken::Name {
            base: base("target", 4),
            value: "target".to_string(),
            length: 6,
        };
        memory.write_token(&name).unwrap();
        let decoded = memory.read_record(&context(), None).unwrap();
        assert_eq!(decoded.subtype, 6);
        assert_eq!(decoded.integer_value, 0);
        assert_eq!(decoded.float_value, 0.0);
    }

    #[test]
    fn write_back_restores_snapshot_bytes() {
        let token = ScriptToken::Name {
            base: base("keep", 9),
            value: "keep".to_string(),
            length: 4,
        };
        let record = ScriptTokenRecord {
            token: token.clone(),
            subtype: 4,
            integer_value: 0,
            float_value: 0.0,
        };
        let mut memory = SourceTokenMemory::new();
        memory.write_record(&record).unwrap();
        memory.set_whitespace_start(11).unwrap();
        memory.set_whitespace_end(22).unwrap();
        memory.set_next(33).unwrap();
        let decoded = memory.read_record(&context(), None).unwrap();
        assert_eq!(decoded.token, token);
        // Clobber the live bytes, then write the token back: the snapshot
        // image (whitespace pointers and link word included) is restored.
        memory.write_string("changed").unwrap();
        memory.set_next(0).unwrap();
        memory.write_token(&token).unwrap();
        assert_eq!(memory.string().unwrap(), "keep");
        assert_eq!(memory.whitespace_start().unwrap(), 11);
        assert_eq!(memory.whitespace_end().unwrap(), 22);
        assert_eq!(memory.next().unwrap(), 33);
    }

    #[test]
    fn repeated_decode_returns_snapshot() {
        let mut memory = SourceTokenMemory::new();
        memory.write_string("name").unwrap();
        memory.set_token_type(4).unwrap();
        memory.set_subtype(4).unwrap();
        memory.set_line(5).unwrap();
        let first = memory.read_record(&context(), None).unwrap();
        let second = memory.read_record(&context(), None).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn read_record_rejects_unknown_type() {
        let mut memory = SourceTokenMemory::new();
        memory.write_string("??").unwrap();
        memory.set_token_type(99).unwrap();
        match memory.read_record(&context(), None) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "token_t has no completed typed token");
            }
            other => panic!("expected untyped-token error, got {other:?}"),
        }
    }

    #[test]
    fn embedded_nul_round_trip() {
        let mut memory = SourceTokenMemory::new();
        memory.write_string("a\0b").unwrap();
        memory.set_token_type(4).unwrap();
        memory.set_line(6).unwrap();
        let decoded = memory.read_record(&context(), None).unwrap();
        assert_eq!(decoded.token.text(), "a\0b");
        assert_eq!(
            decoded.token,
            ScriptToken::Name {
                base: TokenBase {
                    text: "a\0b".to_string(),
                    location: SourceLocation {
                        path: "maps/test.cfg".to_string(),
                        line: 6,
                        column: 7,
                    },
                    leading_whitespace: "  ".to_string(),
                    lines_crossed: 0,
                },
                value: "a\0b".to_string(),
                length: 3,
            }
        );
        // An explicit extent shorter than the NUL-truncated text is dropped.
        let decoded = memory.read_record(&context(), Some(1)).unwrap();
        assert_eq!(decoded.token.text(), "a");
    }

    #[test]
    fn text_extent_param_validated() {
        let mut memory = SourceTokenMemory::new();
        memory.write_string("x").unwrap();
        memory.set_token_type(4).unwrap();
        match memory.read_record(&context(), Some(1024)) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "typed token text extent exceeds its source string field")
            }
            other => panic!("expected extent error, got {other:?}"),
        }
    }

    #[test]
    fn save_state_round_trip() {
        let mut memory = SourceTokenMemory::new();
        memory.write_string("saved").unwrap();
        memory.set_token_type(4).unwrap();
        memory.set_subtype(5).unwrap();
        memory.set_line(21).unwrap();
        memory.set_next(4).unwrap();
        let state = memory.capture_save_state().unwrap();
        assert_eq!(state.bytes.len(), SOURCE_TOKEN_BYTES);
        assert_eq!(state.text_extent, None);

        memory.clear().unwrap();
        assert_eq!(memory.string().unwrap(), "");
        memory.restore_save_state(&state, false).unwrap();
        assert_eq!(memory.string().unwrap(), "saved");
        assert_eq!(memory.token_type().unwrap(), 4);
        assert_eq!(memory.next().unwrap(), 4);

        // Verification compares instead of writing.
        memory.restore_save_state(&state, true).unwrap();
        let mut tampered = state.clone();
        tampered.bytes[0] = b'X';
        match memory.restore_save_state(&tampered, true) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "script.token: token bytes disagree with restored allocation")
            }
            other => panic!("expected allocation-mismatch error, got {other:?}"),
        }
    }

    #[test]
    fn save_state_validates_extent() {
        let mut memory = SourceTokenMemory::new();
        let short = SourceTokenSaveState {
            bytes: vec![0; 100],
            text_extent: None,
        };
        assert_bad_ui(
            memory.restore_save_state(&short, false),
            "script.token: invalid token extent",
        );
        let wide = SourceTokenSaveState {
            bytes: vec![0; SOURCE_TOKEN_BYTES],
            text_extent: Some(1025),
        };
        assert_bad_ui(
            memory.restore_save_state(&wide, false),
            "script.token: invalid token extent",
        );
        // The donor bound is `extent > 1024`, so 1024 itself restores.
        let edge = SourceTokenSaveState {
            bytes: vec![0; SOURCE_TOKEN_BYTES],
            text_extent: Some(1024),
        };
        memory.restore_save_state(&edge, false).unwrap();
    }

    #[test]
    fn copy_from_duplicates_bytes_and_state() {
        let mut source = SourceTokenMemory::new();
        source.write_string("shared").unwrap();
        source.set_token_type(4).unwrap();
        source.set_subtype(6).unwrap();
        source.set_next(12).unwrap();
        let mut copy = SourceTokenMemory::new();
        copy.copy_from(&source).unwrap();
        assert_eq!(copy.bytes().unwrap(), source.bytes().unwrap());
        assert_eq!(copy.string().unwrap(), "shared");
        assert_eq!(copy.next().unwrap(), 12);
        // Later writes to the source do not alias the copy.
        source.write_string("other").unwrap();
        assert_eq!(copy.string().unwrap(), "shared");
    }

    #[test]
    fn bad_span_fails_accessors() {
        let store = SharedScriptBytes::from_vec(vec![0; 100]);
        let token = SourceTokenMemory::view(&store, 0, 100);
        match token.token_type() {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "token_t requires exactly 1068 source bytes");
            }
            other => panic!("expected span error, got {other:?}"),
        }
        let store = SharedScriptBytes::zeroed(SOURCE_TOKEN_BYTES + 10);
        let token = SourceTokenMemory::view(&store, 11, SOURCE_TOKEN_BYTES);
        match token.token_type() {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "token_t requires exactly 1068 source bytes");
            }
            other => panic!("expected span error, got {other:?}"),
        }
    }
}
