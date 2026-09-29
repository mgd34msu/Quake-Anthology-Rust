//! UI CD-key traps.
//!
//! Provenance: `src/compat/qvm/ui-key-syscalls.ts` (UI CD-key traps from id
//! Software `client/cl_ui.c`). The donor's async export invocation becomes a
//! synchronous host query, and [`validate_q3_cd_key`] is a local mirror of
//! `src/core/q3-cd-key.ts`.

use super::client_state::{CallKind, HostCall, QvmRole, SyscallMemory};
use super::legacy_bot_abi::{UI_GET_CDKEY, UI_SET_CDKEY, UI_VERIFY_CDKEY};
use crate::error::GuestError;

/// CD-key bytes excluding the terminator.
pub const QVM_CD_KEY_BYTES: usize = 16;
/// CD-key destination span including the terminator.
pub const QVM_CD_KEY_SPAN: usize = 17;

/// Host CD-key surface used by the traps.
pub trait UiKeyHost {
    /// Result of the `UI_HASUNIQUECDKEY` export query.
    fn has_unique_cd_key(&mut self) -> i32;
    /// Active game directory.
    fn game_directory(&mut self) -> String;
    /// Read the 17-byte UI key record.
    fn read_cd_key(&mut self, unique: i32, directory: &str) -> [u8; QVM_CD_KEY_SPAN];
    /// Write a 16-byte UI key.
    fn write_cd_key(&mut self, unique: i32, directory: &str, key: &[u8; QVM_CD_KEY_BYTES]);
}

/// Validate a CD key against an optional two-character checksum.
///
/// Keys hold 16 characters from `237ABCDGHJLPRSTW` (case-insensitive); the
/// checksum is the lowercase two-digit hex of the byte sum.
pub fn validate_q3_cd_key(key: &str, checksum: Option<&str>) -> bool {
    fn command_text(input: &str) -> Option<String> {
        let end = input.find('\0').unwrap_or(input.len());
        let text = &input[..end];
        if text.chars().any(|ch| ch as u32 > 255) {
            return None;
        }
        Some(text.to_string())
    }
    let (Some(key), checksum) = (command_text(key), checksum.map(command_text)) else {
        return false;
    };
    let checksum = match checksum {
        None => None,
        Some(None) => return false,
        Some(Some(text)) => Some(text),
    };
    if key.len() != QVM_CD_KEY_BYTES || checksum.as_ref().is_some_and(|text| text.len() != 2) {
        return false;
    }
    let mut sum = 0u32;
    for mut byte in key.bytes() {
        if byte.is_ascii_lowercase() {
            byte -= 32;
        }
        if !"237ABCDGHJLPRSTW".bytes().any(|allowed| allowed == byte) {
            return false;
        }
        sum = (sum + u32::from(byte)) & 255;
    }
    checksum.is_none_or(|text| text.to_lowercase() == format!("{sum:02x}"))
}

/// Dispatch a UI CD-key trap. Returns `Ok(None)` when unhandled.
pub fn ui_key_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    services: &mut dyn UiKeyHost,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine || call.role != QvmRole::Ui {
        return Ok(None);
    }
    match call.code {
        UI_GET_CDKEY | UI_SET_CDKEY => {
            let word = call.int(1)?;
            let unique = services.has_unique_cd_key();
            let directory = services.game_directory();
            if call.code == UI_GET_CDKEY {
                let range = memory.span(word, QVM_CD_KEY_SPAN, 0)?;
                let record = services.read_cd_key(unique, &directory);
                memory.write_bytes(range.start, &record)?;
            } else {
                let range = memory.span(word, QVM_CD_KEY_BYTES, 0)?;
                let mut key = [0u8; QVM_CD_KEY_BYTES];
                key.copy_from_slice(memory.read_bytes(range.start, QVM_CD_KEY_BYTES)?);
                services.write_cd_key(unique, &directory, &key);
            }
            Ok(Some(0))
        }
        UI_VERIFY_CDKEY => {
            let key = memory.read_string(call.int(1)?)?;
            if key.len() != QVM_CD_KEY_BYTES {
                return Ok(Some(0));
            }
            let checksum_word = call.int(2)?;
            let checksum = if memory.pointer(checksum_word).is_none() {
                None
            } else {
                Some(memory.read_string(checksum_word)?)
            };
            Ok(Some(i32::from(validate_q3_cd_key(&key, checksum.as_deref()))))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::super::client_state::AbiProfile;
    use super::*;

    struct FakeKeys {
        log: Vec<String>,
    }

    impl UiKeyHost for FakeKeys {
        fn has_unique_cd_key(&mut self) -> i32 {
            1
        }
        fn game_directory(&mut self) -> String {
            "baseq3".to_string()
        }
        fn read_cd_key(&mut self, unique: i32, directory: &str) -> [u8; QVM_CD_KEY_SPAN] {
            self.log.push(format!("read {unique} {directory}"));
            *b"237ABCDGHJLPRSTW\0"
        }
        fn write_cd_key(&mut self, unique: i32, directory: &str, key: &[u8; QVM_CD_KEY_BYTES]) {
            self.log.push(format!("write {unique} {directory} {}", key.len()));
        }
    }

    fn call(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Ui, code, args, AbiProfile::Modern)
    }

    #[test]
    fn get_cdkey_writes_record() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut keys = FakeKeys { log: Vec::new() };
        assert_eq!(
            ui_key_syscall(&call(UI_GET_CDKEY, &[256]), &mut memory, &mut keys).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_bytes(256, 17).unwrap(), b"237ABCDGHJLPRSTW\0");
        assert_eq!(keys.log, vec!["read 1 baseq3".to_string()]);
    }

    #[test]
    fn set_cdkey_passes_bytes() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_bytes(256, b"237ABCDGHJLPRSTW").unwrap();
        let mut keys = FakeKeys { log: Vec::new() };
        assert_eq!(
            ui_key_syscall(&call(UI_SET_CDKEY, &[256]), &mut memory, &mut keys).unwrap(),
            Some(0)
        );
        assert_eq!(keys.log, vec!["write 1 baseq3 16".to_string()]);
    }

    fn checksum(key: &str) -> String {
        let sum = key
            .bytes()
            .map(|mut byte| {
                if byte.is_ascii_lowercase() {
                    byte -= 32;
                }
                u32::from(byte)
            })
            .sum::<u32>()
            & 255;
        format!("{sum:02x}")
    }

    #[test]
    fn verify_cdkey() {
        let key = "237ABCDGHJLPRSTW";
        let sum = checksum(key);
        assert!(validate_q3_cd_key(key, None));
        assert!(validate_q3_cd_key(key, Some(&sum)));
        assert!(validate_q3_cd_key(&key.to_lowercase(), Some(&sum.to_uppercase())));
        assert!(!validate_q3_cd_key(key, Some("00")));
        assert!(!validate_q3_cd_key("237ABCDGHJLPRSTWX1", None));
        assert!(!validate_q3_cd_key("short", None));
        assert!(!validate_q3_cd_key("!!!!!!!!!!!!!!!!", None));
        assert!(!validate_q3_cd_key(key, Some("xyz")));
    }

    #[test]
    fn verify_trap_handles_length_and_null_checksum() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_string(256, "237ABCDGHJLPRSTW", 17).unwrap();
        memory.write_string(512, "short", 6).unwrap();
        let mut keys = FakeKeys { log: Vec::new() };
        assert_eq!(
            ui_key_syscall(&call(UI_VERIFY_CDKEY, &[256, 0]), &mut memory, &mut keys).unwrap(),
            Some(1)
        );
        assert_eq!(
            ui_key_syscall(&call(UI_VERIFY_CDKEY, &[512, 0]), &mut memory, &mut keys).unwrap(),
            Some(0)
        );
        let sum = checksum("237ABCDGHJLPRSTW");
        memory.write_string(768, &sum, 3).unwrap();
        assert_eq!(
            ui_key_syscall(&call(UI_VERIFY_CDKEY, &[256, 768]), &mut memory, &mut keys).unwrap(),
            Some(1)
        );
        memory.write_string(768, "zz", 3).unwrap();
        assert_eq!(
            ui_key_syscall(&call(UI_VERIFY_CDKEY, &[256, 768]), &mut memory, &mut keys).unwrap(),
            Some(0)
        );
    }

    #[test]
    fn routing() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut keys = FakeKeys { log: Vec::new() };
        let other = HostCall::engine(QvmRole::Cgame, UI_GET_CDKEY, &[256], AbiProfile::Modern);
        assert_eq!(ui_key_syscall(&other, &mut memory, &mut keys).unwrap(), None);
        assert_eq!(ui_key_syscall(&call(999, &[]), &mut memory, &mut keys).unwrap(), None);
    }
}
