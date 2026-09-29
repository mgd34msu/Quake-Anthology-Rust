//! Client precompiler traps over the shared source parser.
//!
//! Provenance: `src/compat/qvm/client-script-syscalls.ts` (Q3 client PC
//! traps over the shared botlib source parser). [`ScriptToken`] and
//! [`write_script_token`] are local mirrors of `script-record.ts` (owned by
//! another worker); sibling files reuse them via
//! `super::client_script_syscalls::...`. Donor promises become direct returns.

use super::client_state::{CallKind, HostCall, QvmRole, SyscallMemory};
use super::legacy_bot_abi::{CG_PC_ADD_GLOBAL_DEFINE, CG_PC_LOAD_SOURCE, UI_PC_ADD_GLOBAL_DEFINE, UI_PC_LOAD_SOURCE};
use crate::error::GuestError;

/// Byte length of `pc_token_t`.
pub const QVM_SCRIPT_TOKEN_BYTES: usize = 1040;
/// Maximum token string bytes.
pub const QVM_SCRIPT_TOKEN_STRING_MAX: usize = 1023;

/// Script token kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptTokenKind {
    /// Primitive token.
    Primitive,
    /// String token.
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

/// Token kind to `pc_token_t` type number.
#[must_use]
pub const fn token_type(kind: ScriptTokenKind) -> i32 {
    match kind {
        ScriptTokenKind::Primitive => 0,
        ScriptTokenKind::String => 1,
        ScriptTokenKind::Literal => 2,
        ScriptTokenKind::Number => 3,
        ScriptTokenKind::Name => 4,
        ScriptTokenKind::Punctuation => 5,
    }
}

/// Owned script token record.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptToken {
    /// Token text (source bytes).
    pub text: String,
    /// Token kind.
    pub kind: ScriptTokenKind,
    /// Token subtype.
    pub subtype: i32,
    /// Integer value.
    pub integer_value: i32,
    /// Float value.
    pub float_value: f32,
}

/// Write `PC_ReadToken` output, including quote stripping for string tokens.
pub fn write_script_token(memory: &mut SyscallMemory, word: i32, value: &ScriptToken) -> Result<(), GuestError> {
    let range = memory.span(word, QVM_SCRIPT_TOKEN_BYTES, 0)?;
    let mut bytes = Vec::new();
    for (index, ch) in value.text.chars().enumerate() {
        let code = ch as u32;
        if code == 0 {
            break;
        }
        if index >= QVM_SCRIPT_TOKEN_STRING_MAX || code > 255 {
            return Err(GuestError::invalid(
                "QVM pc_token_t string exceeds 1023 source bytes or needs source byte characters",
            ));
        }
        bytes.push(code as u8);
    }
    let token_type = token_type(value.kind);
    let leading_quote = token_type == 1 && bytes.first() == Some(&b'"');
    let stripped = bytes.len() - usize::from(leading_quote);
    if token_type == 1 && stripped == 0 {
        return Err(GuestError::invalid(
            "QVM pc_token_t cannot encode undefined empty StripDoubleQuotes input",
        ));
    }
    for (index, byte) in bytes.iter().enumerate() {
        memory.set(range.start + 16 + index, *byte)?;
    }
    memory.set(range.start + 16 + bytes.len(), 0)?;
    memory.write_i32(range.start, token_type)?;
    memory.write_i32(range.start + 4, value.subtype)?;
    memory.write_i32(range.start + 8, value.integer_value)?;
    memory.write_f32(range.start + 12, value.float_value)?;
    if leading_quote {
        for index in 0..bytes.len() {
            let byte = memory.get(range.start + 17 + index)?;
            memory.set(range.start + 16 + index, byte)?;
        }
    }
    if token_type == 1 && memory.get(range.start + 16 + stripped - 1)? == b'"' {
        memory.set(range.start + 16 + stripped - 1, 0)?;
    }
    Ok(())
}

/// Token-read outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenRead {
    /// Token record to publish.
    pub token: ScriptToken,
    /// Whether the read consumed a token.
    pub read: bool,
}

/// Host client-script surface used by the traps.
pub trait ClientScriptHost {
    /// Add a global define, returning whether it was new.
    fn add_define(&mut self, text: &str) -> bool;
    /// Load a source file, returning its handle (0 when unavailable).
    fn load(&mut self, filename: &str) -> i32;
    /// Free a source handle, returning whether it was live.
    fn free(&mut self, handle: i32) -> bool;
    /// Read the next token from a handle.
    fn read_token(&mut self, handle: i32) -> Option<TokenRead>;
    /// Source position by handle: filename plus line.
    fn position(&mut self, handle: i32) -> Option<(String, i32)>;
}

/// Dispatch a client-script trap. Returns `Ok(None)` when unhandled.
pub fn client_script_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    scripts: &mut dyn ClientScriptHost,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine || call.role == QvmRole::Qagame {
        return Ok(None);
    }
    let (define, load) = if call.role == QvmRole::Ui {
        (UI_PC_ADD_GLOBAL_DEFINE, UI_PC_LOAD_SOURCE)
    } else {
        (CG_PC_ADD_GLOBAL_DEFINE, CG_PC_LOAD_SOURCE)
    };
    if call.code == define {
        let text = memory.read_string(call.int(1)?)?;
        return Ok(Some(i32::from(scripts.add_define(&text))));
    }
    match call.code - load {
        0 => {
            let filename = memory.read_string(call.int(1)?)?;
            Ok(Some(scripts.load(&filename)))
        }
        1 => Ok(Some(i32::from(scripts.free(call.int(1)?)))),
        2 => {
            let handle = call.int(1)?;
            let word = call.int(2)?;
            match scripts.read_token(handle) {
                None => Ok(Some(0)),
                Some(outcome) => {
                    write_script_token(memory, word, &outcome.token)?;
                    Ok(Some(i32::from(outcome.read)))
                }
            }
        }
        3 => {
            let handle = call.int(1)?;
            let name_word = call.int(2)?;
            let line_word = call.int(3)?;
            match scripts.position(handle) {
                None => Ok(Some(0)),
                Some((filename, line)) => {
                    let truncated: String = filename.chars().take(64).collect();
                    let capacity = truncated.len() + 1;
                    memory.write_bounded_string(name_word, &truncated, capacity)?;
                    memory.span(line_word, 4, 0)?;
                    let base = memory.pointer(line_word).expect("checked span");
                    memory.write_i32(base, line)?;
                    Ok(Some(1))
                }
            }
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::super::client_state::AbiProfile;
    use super::*;

    struct FakeScripts {
        log: Vec<String>,
    }

    impl ClientScriptHost for FakeScripts {
        fn add_define(&mut self, text: &str) -> bool {
            self.log.push(format!("define {text}"));
            true
        }
        fn load(&mut self, filename: &str) -> i32 {
            self.log.push(format!("load {filename}"));
            if filename == "missing" {
                0
            } else {
                2
            }
        }
        fn free(&mut self, handle: i32) -> bool {
            handle == 2
        }
        fn read_token(&mut self, handle: i32) -> Option<TokenRead> {
            (handle == 2).then(|| TokenRead {
                token: ScriptToken {
                    text: "hello".to_string(),
                    kind: ScriptTokenKind::Name,
                    subtype: 7,
                    integer_value: 0,
                    float_value: 0.0,
                },
                read: true,
            })
        }
        fn position(&mut self, handle: i32) -> Option<(String, i32)> {
            (handle == 2).then(|| ("maps/a.script".to_string(), 41))
        }
    }

    fn cg(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Cgame, code, args, AbiProfile::Modern)
    }

    #[test]
    fn full_sequence() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(256, "MAX 4", 6).unwrap();
        memory.write_string(512, "bot.c", 6).unwrap();
        let mut scripts = FakeScripts { log: Vec::new() };
        assert_eq!(
            client_script_syscall(&cg(64, &[256]), &mut memory, &mut scripts).unwrap(),
            Some(1)
        );
        assert_eq!(
            client_script_syscall(&cg(65, &[512]), &mut memory, &mut scripts).unwrap(),
            Some(2)
        );
        assert_eq!(
            client_script_syscall(&cg(66, &[2]), &mut memory, &mut scripts).unwrap(),
            Some(1)
        );
        assert_eq!(
            client_script_syscall(&cg(66, &[9]), &mut memory, &mut scripts).unwrap(),
            Some(0)
        );
        assert_eq!(
            client_script_syscall(&cg(67, &[2, 1024]), &mut memory, &mut scripts).unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_i32(1024).unwrap(), 4);
        assert_eq!(memory.read_i32(1028).unwrap(), 7);
        assert_eq!(memory.read_string(1040).unwrap(), "hello");
        assert_eq!(
            client_script_syscall(&cg(67, &[9, 1024]), &mut memory, &mut scripts).unwrap(),
            Some(0)
        );
        assert_eq!(
            client_script_syscall(&cg(68, &[2, 256, 512]), &mut memory, &mut scripts).unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_string(256).unwrap(), "maps/a.script");
        assert_eq!(memory.read_i32(512).unwrap(), 41);
        assert_eq!(
            client_script_syscall(&cg(68, &[9, 256, 512]), &mut memory, &mut scripts).unwrap(),
            Some(0)
        );
        assert_eq!(scripts.log, vec!["define MAX 4".to_string(), "load bot.c".to_string()]);
    }

    #[test]
    fn ui_bases_and_routing() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(256, "X", 2).unwrap();
        let mut scripts = FakeScripts { log: Vec::new() };
        let define = HostCall::engine(QvmRole::Ui, 57, &[256], AbiProfile::Modern);
        assert_eq!(
            client_script_syscall(&define, &mut memory, &mut scripts).unwrap(),
            Some(1)
        );
        let load = HostCall::engine(QvmRole::Ui, 58, &[256], AbiProfile::Modern);
        assert_eq!(
            client_script_syscall(&load, &mut memory, &mut scripts).unwrap(),
            Some(2)
        );
        let game = HostCall::engine(QvmRole::Qagame, 64, &[256], AbiProfile::Modern);
        assert_eq!(client_script_syscall(&game, &mut memory, &mut scripts).unwrap(), None);
        assert_eq!(
            client_script_syscall(&cg(69, &[]), &mut memory, &mut scripts).unwrap(),
            None
        );
    }

    #[test]
    fn string_token_strips_quotes() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let token = ScriptToken {
            text: "\"quoted\"".to_string(),
            kind: ScriptTokenKind::String,
            subtype: 0,
            integer_value: 0,
            float_value: 1.5,
        };
        write_script_token(&mut memory, 128, &token).unwrap();
        assert_eq!(memory.read_i32(128).unwrap(), 1);
        assert_eq!(memory.read_f32(140).unwrap(), 1.5);
        assert_eq!(memory.read_string(144).unwrap(), "quoted");
    }

    #[test]
    fn token_write_rejects_bad_strings() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let long = ScriptToken {
            text: "x".repeat(1024),
            kind: ScriptTokenKind::Name,
            subtype: 0,
            integer_value: 0,
            float_value: 0.0,
        };
        assert!(write_script_token(&mut memory, 128, &long).is_err());
        let wide = ScriptToken {
            text: "caf\u{e9}".to_string(),
            ..long.clone()
        };
        assert!(write_script_token(&mut memory, 128, &wide).is_ok());
        let astral = ScriptToken {
            text: "\u{1f600}".to_string(),
            ..long.clone()
        };
        assert!(write_script_token(&mut memory, 128, &astral).is_err());
        let empty_quote = ScriptToken {
            text: "\"".to_string(),
            kind: ScriptTokenKind::String,
            ..long
        };
        assert!(write_script_token(&mut memory, 128, &empty_quote).is_err());
    }
}
