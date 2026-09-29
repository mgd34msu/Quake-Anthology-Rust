//! QVM `pc_token_t` record writer.
//!
//! Provenance: `src/compat/qvm/script-record.ts` (port of id Software's
//! `game/q_shared.h:pc_token_t`, `botlib/l_precomp.c:PC_ReadTokenHandle`, and
//! `botlib/l_script.c:StripDoubleQuotes`).
//!
//! [`ScriptTokenKind`] mirrors `src/ui/common/legacy/script/lexer.ts`
//! (`ScriptToken` kinds) and [`SourceTokenMemory`] mirrors
//! `src/ui/common/legacy/script/token-memory.ts` (1068-byte `token_t`).

use crate::error::GuestError;

/// `pc_token_t` size in bytes.
pub const QVM_SCRIPT_TOKEN_BYTES: usize = 1040;

/// `token_t` allocation size in bytes.
pub const SOURCE_TOKEN_BYTES: usize = 1068;

/// Script token kind (source `TT_*` type tags).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptTokenKind {
    /// Primitive token.
    Primitive,
    /// Quoted string token.
    String,
    /// Literal token.
    Literal,
    /// Number token.
    Number,
    /// Name token.
    Name,
    /// Punctuation token.
    Punctuation,
}

impl ScriptTokenKind {
    /// Source type tag.
    #[must_use]
    pub const fn token_type(self) -> i32 {
        match self {
            Self::Primitive => 0,
            Self::String => 1,
            Self::Literal => 2,
            Self::Number => 3,
            Self::Name => 4,
            Self::Punctuation => 5,
        }
    }
}

/// Token value accepted by [`write_qvm_script_token`].
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptTokenInput {
    /// Token text (Latin-1 bytes).
    pub text: String,
    /// Source type tag.
    pub token_type: i32,
    /// Source subtype.
    pub subtype: i32,
    /// Exactly represented source integer.
    pub integer_value: i64,
    /// Source float value.
    pub float_value: f32,
}

impl ScriptTokenInput {
    /// Build from a lexer token kind.
    #[must_use]
    pub fn record(
        kind: ScriptTokenKind,
        text: &str,
        subtype: i32,
        integer_value: i64,
        float_value: f32,
    ) -> Self {
        Self {
            text: text.to_string(),
            token_type: kind.token_type(),
            subtype,
            integer_value,
            float_value,
        }
    }

    /// Build from live token memory.
    pub fn from_memory(memory: &SourceTokenMemory) -> Result<Self, GuestError> {
        Ok(Self {
            text: memory.text(),
            token_type: memory.token_type(),
            subtype: memory.subtype(),
            integer_value: i64::from(memory.integer_value()),
            float_value: memory.float_value(),
        })
    }
}

/// Live 1068-byte `token_t` allocation (mirror of `SourceTokenMemory`).
#[derive(Debug, Clone)]
pub struct SourceTokenMemory {
    bytes: Vec<u8>,
}

impl SourceTokenMemory {
    /// Wrap exactly 1068 source bytes.
    pub fn wrap(bytes: Vec<u8>) -> Result<Self, GuestError> {
        if bytes.len() != SOURCE_TOKEN_BYTES {
            return Err(GuestError::invalid("token_t requires exactly 1068 source bytes"));
        }
        Ok(Self { bytes })
    }

    /// Token string (NUL-terminated Latin-1 prefix of the text extent).
    #[must_use]
    pub fn text(&self) -> String {
        self.bytes[..1024]
            .iter()
            .take_while(|byte| **byte != 0)
            .map(|byte| *byte as char)
            .collect()
    }

    /// Source type tag.
    #[must_use]
    pub fn token_type(&self) -> i32 {
        i32::from_le_bytes(self.bytes[1024..1028].try_into().unwrap_or([0; 4]))
    }

    /// Source subtype.
    #[must_use]
    pub fn subtype(&self) -> i32 {
        i32::from_le_bytes(self.bytes[1028..1032].try_into().unwrap_or([0; 4]))
    }

    /// Source integer value.
    #[must_use]
    pub fn integer_value(&self) -> u32 {
        u32::from_le_bytes(self.bytes[1032..1036].try_into().unwrap_or([0; 4]))
    }

    /// Source float value.
    #[must_use]
    pub fn float_value(&self) -> f32 {
        f32::from_le_bytes(self.bytes[1036..1040].try_into().unwrap_or([0; 4]))
    }
}

/// Measure the source string length, enforcing byte-string rules.
fn string_length(text: &str) -> Result<usize, GuestError> {
    for (index, byte) in text.bytes().enumerate() {
        if byte == 0 {
            return Ok(index);
        }
        if index >= 1023 {
            return Err(GuestError::invalid("QVM pc_token_t string exceeds 1023 source bytes"));
        }
    }
    if !text.is_ascii() {
        return Err(GuestError::invalid("QVM pc_token_t requires source byte characters"));
    }
    Ok(text.len())
}

/// Write `PC_ReadToken` output, including its defined partial false result
/// and `StripDoubleQuotes` handling of quoted strings.
pub fn write_qvm_script_token(bytes: &mut [u8], value: &ScriptTokenInput) -> Result<(), GuestError> {
    if bytes.len() < QVM_SCRIPT_TOKEN_BYTES {
        return Err(GuestError::invalid(format!(
            "QVM pc_token_t record requires {QVM_SCRIPT_TOKEN_BYTES} bytes, received {}",
            bytes.len()
        )));
    }
    let length = string_length(&value.text)?;
    if value.integer_value < -(1 << 53) || value.integer_value > (1 << 53) {
        return Err(GuestError::invalid(
            "QVM pc_token_t requires an exactly represented source integer",
        ));
    }
    let text = value.text.as_bytes();
    let leading_quote = value.token_type == 1 && text.first() == Some(&b'"');
    let stripped = length - usize::from(leading_quote);
    if value.token_type == 1 && stripped == 0 {
        return Err(GuestError::invalid(
            "QVM pc_token_t cannot encode undefined empty StripDoubleQuotes input",
        ));
    }
    bytes[16..16 + length].copy_from_slice(&text[..length]);
    bytes[16 + length] = 0;
    bytes[0..4].copy_from_slice(&value.token_type.to_le_bytes());
    bytes[4..8].copy_from_slice(&value.subtype.to_le_bytes());
    bytes[8..12].copy_from_slice(&(value.integer_value as i32).to_le_bytes());
    bytes[12..16].copy_from_slice(&value.float_value.to_le_bytes());
    if leading_quote {
        bytes.copy_within(17..17 + length, 16);
    }
    if value.token_type == 1 && bytes[16 + stripped - 1] == b'"' {
        bytes[16 + stripped - 1] = 0;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_token_writes_c_string_and_scalars() {
        let mut bytes = vec![0u8; QVM_SCRIPT_TOKEN_BYTES];
        let value = ScriptTokenInput::record(ScriptTokenKind::Name, "player", 3, 42, 1.5);
        write_qvm_script_token(&mut bytes, &value).unwrap();
        assert_eq!(&bytes[16..23], b"player\0");
        assert_eq!(i32::from_le_bytes(bytes[0..4].try_into().unwrap()), 4);
        assert_eq!(i32::from_le_bytes(bytes[4..8].try_into().unwrap()), 3);
        assert_eq!(i32::from_le_bytes(bytes[8..12].try_into().unwrap()), 42);
        assert_eq!(f32::from_le_bytes(bytes[12..16].try_into().unwrap()), 1.5);
    }

    #[test]
    fn quoted_string_strips_double_quotes() {
        let mut bytes = vec![0u8; QVM_SCRIPT_TOKEN_BYTES];
        let value = ScriptTokenInput::record(ScriptTokenKind::String, "\"hi\"", 0, 0, 0.0);
        write_qvm_script_token(&mut bytes, &value).unwrap();
        assert_eq!(&bytes[16..19], b"hi\0");
        let empty = ScriptTokenInput::record(ScriptTokenKind::String, "\"", 0, 0, 0.0);
        assert!(write_qvm_script_token(&mut bytes, &empty).is_err());
    }

    #[test]
    fn oversized_and_non_byte_strings_rejected() {
        let mut bytes = vec![0u8; QVM_SCRIPT_TOKEN_BYTES];
        let long = "x".repeat(1024);
        let value = ScriptTokenInput::record(ScriptTokenKind::Name, &long, 0, 0, 0.0);
        assert!(write_qvm_script_token(&mut bytes, &value).is_err());
        let wide = ScriptTokenInput::record(ScriptTokenKind::Name, "caf\u{e9}", 0, 0, 0.0);
        assert!(write_qvm_script_token(&mut bytes, &wide).is_err());
        let mut short = vec![0u8; 16];
        let value = ScriptTokenInput::record(ScriptTokenKind::Name, "ok", 0, 0, 0.0);
        assert!(write_qvm_script_token(&mut short, &value).is_err());
    }

    #[test]
    fn token_memory_feeds_writer() {
        let mut raw = vec![0u8; SOURCE_TOKEN_BYTES];
        raw[..4].copy_from_slice(b"name");
        raw[1024..1028].copy_from_slice(&4i32.to_le_bytes());
        raw[1028..1032].copy_from_slice(&2i32.to_le_bytes());
        raw[1032..1036].copy_from_slice(&9u32.to_le_bytes());
        raw[1036..1040].copy_from_slice(&2.5f32.to_le_bytes());
        let memory = SourceTokenMemory::wrap(raw).unwrap();
        assert_eq!(memory.text(), "name");
        assert_eq!(memory.token_type(), 4);
        assert_eq!(memory.integer_value(), 9);
        let value = ScriptTokenInput::from_memory(&memory).unwrap();
        let mut bytes = vec![0u8; QVM_SCRIPT_TOKEN_BYTES];
        write_qvm_script_token(&mut bytes, &value).unwrap();
        assert_eq!(&bytes[16..21], b"name\0");
        assert!(SourceTokenMemory::wrap(vec![0u8; 8]).is_err());
    }
}
