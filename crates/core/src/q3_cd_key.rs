//! Port of Quake-Anthology-TS `src/core/q3-cd-key.ts`
//!
//! Quake III CD-key byte state from id Software `common.c`, `cl_main.c` and
//! `cl_ui.c`. The donor's `async` storage becomes synchronous closures so the
//! state stays dependency-free: file reads take a `load_text` closure and
//! writes take a `dump` closure, each generic over the caller's store error.
//! The donor constructor's cvar handle is borrowed at the `write_ui` call
//! instead, the only method that marks archive flags.

use thiserror::Error;

use crate::cmd::{source_command_text, CmdError};
use crate::cvar::{flags, CvarRegistry};

/// Characters a CD key may contain after ASCII uppercasing.
const KEY_ALPHABET: &str = "237ABCDGHJLPRSTW";

/// Loose key file name.
const KEY_FILE_NAME: &str = "q3key";

/// Build variant selecting the initial key bytes (donor `build` parameter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Q3CdKeyBuild {
    /// Client: 32 spaces.
    #[default]
    Client,
    /// Dedicated: `123456789` prefix.
    Dedicated,
}

/// Which 16-byte key half a file write targets (donor `offset: 0 | 16`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3KeySlot {
    /// Base key at byte offset 0.
    Base,
    /// Mod key at byte offset 16.
    Game,
}

impl Q3KeySlot {
    /// Byte offset of the slot.
    fn offset(self) -> usize {
        match self {
            Q3KeySlot::Base => 0,
            Q3KeySlot::Game => 16,
        }
    }

    /// Slot for a donor `0 | 16` file offset, or [`None`] when invalid.
    #[must_use]
    pub fn from_offset(offset: u32) -> Option<Self> {
        match offset {
            0 => Some(Q3KeySlot::Base),
            16 => Some(Q3KeySlot::Game),
            _ => None,
        }
    }
}

/// VM-facing UI key write sink (donor `readUi` `writes` parameter).
pub trait Q3CdKeyUiWrites {
    /// Copy key bytes.
    fn copy(&mut self, bytes: &[u8]);
    /// Set one byte.
    fn set_byte(&mut self, offset: usize, value: u8);
}

/// CD-key byte failures (donor `RangeError`s).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3CdKeyError {
    /// Appended key bytes overflow the 34-byte store.
    #[error("CD key storage overflow")]
    StorageOverflow,
    /// Authorization destination needs 33 bytes.
    #[error("CD key authorization requires 33 bytes")]
    AuthorizationTooShort,
    /// UI destination needs 17 bytes.
    #[error("CD key UI destination requires 17 bytes")]
    UiDestinationTooShort,
    /// UI source needs 16 bytes.
    #[error("CD key UI source requires 16 bytes")]
    UiSourceTooShort,
    /// Game directory must be source bytes.
    #[error(transparent)]
    CommandText(#[from] CmdError),
}

/// Storage-or-key failure from a key-file operation.
#[derive(Debug, Error)]
pub enum Q3CdKeyFileError<E> {
    /// Backing store failure.
    #[error(transparent)]
    Storage(E),
    /// Key byte failure.
    #[error(transparent)]
    Key(#[from] Q3CdKeyError),
}

/// Validate a CD key with an optional two-digit hex checksum.
#[must_use]
pub fn validate_q3_cd_key(key: &str, checksum: Option<&str>) -> bool {
    if key.chars().any(|character| character as u32 > 255) {
        return false;
    }
    if checksum.is_some_and(|checksum| checksum.chars().any(|character| character as u32 > 255)) {
        return false;
    }
    let Ok(key) = source_command_text(key) else {
        return false;
    };
    let checksum = match checksum {
        None => None,
        Some(checksum) => match source_command_text(checksum) {
            Ok(checksum) => Some(checksum),
            Err(_) => return false,
        },
    };
    if key.chars().count() != 16 || checksum.as_ref().is_some_and(|checksum| checksum.chars().count() != 2) {
        return false;
    }
    let mut sum: u8 = 0;
    for mut character in key.chars() {
        if character.is_ascii_lowercase() {
            character = (character as u8 - 32) as char;
        }
        if !KEY_ALPHABET.contains(character) {
            return false;
        }
        sum = sum.wrapping_add(character as u8);
    }
    match checksum {
        None => true,
        Some(checksum) => checksum.to_lowercase() == format!("{sum:02x}"),
    }
}

/// Quake III CD-key state (donor `Q3CdKeyState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3CdKeyState {
    bytes: [u8; 34],
    build: Q3CdKeyBuild,
}

impl Q3CdKeyState {
    /// Fresh state for a build variant.
    #[must_use]
    pub fn new(build: Q3CdKeyBuild) -> Self {
        let mut bytes = [0u8; 34];
        match build {
            Q3CdKeyBuild::Client => bytes[0..32].fill(32),
            Q3CdKeyBuild::Dedicated => {
                for (index, byte) in bytes.iter_mut().take(9).enumerate() {
                    *byte = 49 + index as u8;
                }
            }
        }
        Self { bytes, build }
    }

    /// Key bytes from the loose key file, or [`None`] when absent or invalid.
    fn file_bytes<E>(&self, load_text: &dyn Fn(&str) -> Result<Option<String>, E>) -> Result<Option<[u8; 33]>, E> {
        let Some(text) = load_text(KEY_FILE_NAME)? else {
            return Ok(None);
        };
        let units: Vec<u16> = text.encode_utf16().take(16).collect();
        if self.build == Q3CdKeyBuild::Client
            && !String::from_utf16(&units).is_ok_and(|key| validate_q3_cd_key(&key, None))
        {
            return Ok(None);
        }
        let mut result = [0u8; 33];
        for (index, unit) in units.iter().enumerate() {
            let byte = (unit & 0xFF) as u8;
            if byte == 0 {
                break;
            }
            result[index] = byte;
        }
        Ok(Some(result))
    }

    /// Read the base key file.
    pub fn read_file<E>(&mut self, load_text: &dyn Fn(&str) -> Result<Option<String>, E>) -> Result<(), E> {
        match self.file_bytes(load_text)? {
            None => {
                self.bytes[0..16].fill(32);
                self.bytes[16] = 0;
            }
            Some(key) => self.bytes[0..17].copy_from_slice(&key[0..17]),
        }
        Ok(())
    }

    /// Append the mod key file.
    pub fn append_file<E>(
        &mut self,
        load_text: &dyn Fn(&str) -> Result<Option<String>, E>,
    ) -> Result<(), Q3CdKeyFileError<E>> {
        let key = self.file_bytes(load_text).map_err(Q3CdKeyFileError::Storage)?;
        let Some(key) = key else {
            self.bytes[16..32].fill(32);
            self.bytes[32] = 0;
            return Ok(());
        };
        let end = self.bytes[16..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|offset| offset + 16);
        let length = key.iter().position(|byte| *byte == 0).map_or(33, |offset| offset + 1);
        match end {
            None => Err(Q3CdKeyFileError::Key(Q3CdKeyError::StorageOverflow)),
            Some(end) if end + length > self.bytes.len() => Err(Q3CdKeyFileError::Key(Q3CdKeyError::StorageOverflow)),
            Some(end) => {
                self.bytes[end..end + length].copy_from_slice(&key[..length]);
                Ok(())
            }
        }
    }

    /// Write a validated key half back to its key file; invalid halves are skipped.
    pub fn write_file<E>(&self, dump: &dyn Fn(&str, &str) -> Result<(), E>, slot: Q3KeySlot) -> Result<(), E> {
        let offset = slot.offset();
        let key: String = self.bytes[offset..offset + 16]
            .iter()
            .map(|byte| *byte as char)
            .collect();
        if !validate_q3_cd_key(&key, None) {
            return Ok(());
        }
        dump(
            KEY_FILE_NAME,
            &format!(
                "{key}\n// generated by quake, do not modify\r\n// Do not give this file to ANYONE.\r\n// id Software and Activision will NOT ask you to send this file to them.\r\n"
            ),
        )?;
        Ok(())
    }

    /// Fill the 33-byte authorization key.
    pub fn read_authorization(&self, destination: &mut [u8]) -> Result<(), Q3CdKeyError> {
        if destination.len() < 33 {
            return Err(Q3CdKeyError::AuthorizationTooShort);
        }
        destination[..32].copy_from_slice(&self.bytes[..32]);
        destination[32] = 0;
        Ok(())
    }

    /// Byte offset of the UI key half.
    fn ui_offset(&self, unique: i32, game_directory: &str) -> Result<usize, Q3CdKeyError> {
        let directory = source_command_text(game_directory)?;
        Ok(if unique == 1 && !directory.is_empty() { 16 } else { 0 })
    }

    /// Read the UI key half into `destination`, or through `writes` when present.
    pub fn read_ui(
        &self,
        unique: i32,
        game_directory: &str,
        destination: &mut [u8],
        writes: Option<&mut dyn Q3CdKeyUiWrites>,
    ) -> Result<(), Q3CdKeyError> {
        if destination.len() < 17 {
            return Err(Q3CdKeyError::UiDestinationTooShort);
        }
        let offset = self.ui_offset(unique, game_directory)?;
        let source = &self.bytes[offset..offset + 16];
        match writes {
            None => {
                destination[..16].copy_from_slice(source);
                destination[16] = 0;
            }
            Some(writes) => {
                writes.copy(source);
                writes.set_byte(16, 0);
            }
        }
        Ok(())
    }

    /// Write the UI key half from `source`, marking archive flags on `cvars`.
    pub fn write_ui(
        &mut self,
        unique: i32,
        game_directory: &str,
        source: &[u8],
        cvars: &mut CvarRegistry,
    ) -> Result<(), Q3CdKeyError> {
        if source.len() < 16 {
            return Err(Q3CdKeyError::UiSourceTooShort);
        }
        let offset = self.ui_offset(unique, game_directory)?;
        self.bytes[offset..offset + 16].copy_from_slice(&source[..16]);
        if offset == 16 {
            self.bytes[32] = 0;
        }
        cvars.mark_modified_flags(flags::ARCHIVE);
        Ok(())
    }
}

impl Default for Q3CdKeyState {
    /// Client state (donor default `build`).
    fn default() -> Self {
        Self::new(Q3CdKeyBuild::Client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::Dialect;
    use std::cell::RefCell;
    use std::convert::Infallible;

    const VALID_BASE: &str = "AAAAAAAAAAAAAAAA";
    const VALID_MOD: &str = "LLLLLLLLLLLLLLLL";

    fn cvars() -> CvarRegistry {
        CvarRegistry::new(Dialect::Q3)
    }

    #[test]
    fn validate_accepts_keys_and_checksums() {
        assert!(validate_q3_cd_key(VALID_BASE, None));
        assert!(validate_q3_cd_key("aaaaaaaaaaaaaaaa", None));
        assert!(validate_q3_cd_key(VALID_BASE, Some("10")));
        assert!(validate_q3_cd_key(VALID_MOD, Some("c0")));
        assert!(validate_q3_cd_key(VALID_MOD, Some("C0")));
    }

    #[test]
    fn validate_rejects_shape_and_charset() {
        assert!(!validate_q3_cd_key("AAAAAAAAAAAAAAA", None));
        assert!(!validate_q3_cd_key("AAAAAAAAAAAAAAAAA", None));
        assert!(!validate_q3_cd_key("AAAA\0AAAAAAAAAAA", None));
        assert!(!validate_q3_cd_key("AAAAAAAAAAAAAAAZ", None));
        assert!(!validate_q3_cd_key("AAAAAAAAAAAAAAA0", None));
        assert!(!validate_q3_cd_key("AAAAAAAAAAAAAAAé", None));
        assert!(!validate_q3_cd_key("AAAAAAAAAAAAAAA€", None));
        assert!(!validate_q3_cd_key("                ", None));
        assert!(!validate_q3_cd_key(VALID_BASE, Some("11")));
        assert!(!validate_q3_cd_key(VALID_BASE, Some("1")));
        assert!(!validate_q3_cd_key(VALID_BASE, Some("100")));
    }

    #[test]
    fn constructor_fills_build_bytes() {
        let client = Q3CdKeyState::new(Q3CdKeyBuild::Client);
        assert_eq!(&client.bytes[0..32], &[32u8; 32]);
        assert_eq!(&client.bytes[32..34], &[0u8; 2]);
        let dedicated = Q3CdKeyState::new(Q3CdKeyBuild::Dedicated);
        assert_eq!(&dedicated.bytes[0..9], b"123456789");
        assert_eq!(&dedicated.bytes[9], &0);
        assert_eq!(Q3CdKeyState::default().build, Q3CdKeyBuild::Client);
    }

    #[test]
    fn read_file_loads_valid_base_and_blanks_invalid() {
        let mut state = Q3CdKeyState::default();
        let load = |_: &str| -> Result<Option<String>, Infallible> { Ok(Some(format!("{VALID_BASE}\n"))) };
        state.read_file(&load).expect("read valid");
        assert_eq!(&state.bytes[0..16], VALID_BASE.as_bytes());
        assert_eq!(state.bytes[16], 0);

        let mut state = Q3CdKeyState::default();
        let load = |_: &str| -> Result<Option<String>, Infallible> { Ok(Some("bogus-key".to_owned())) };
        state.read_file(&load).expect("read invalid");
        assert_eq!(&state.bytes[0..16], &[32u8; 16]);
        assert_eq!(state.bytes[16], 0);

        let mut state = Q3CdKeyState::default();
        let load = |_: &str| -> Result<Option<String>, Infallible> { Ok(None) };
        state.read_file(&load).expect("read missing");
        assert_eq!(&state.bytes[0..16], &[32u8; 16]);
        assert_eq!(state.bytes[16], 0);
    }

    #[test]
    fn read_file_dedicated_skips_validation() {
        let mut state = Q3CdKeyState::new(Q3CdKeyBuild::Dedicated);
        let load = |_: &str| -> Result<Option<String>, Infallible> { Ok(Some("hello".to_owned())) };
        state.read_file(&load).expect("read dedicated");
        assert_eq!(&state.bytes[0..5], b"hello");
        assert_eq!(state.bytes[5], 0);
    }

    #[test]
    fn append_file_appends_mod_and_blanks_missing() {
        let mut state = Q3CdKeyState::default();
        let base = |_: &str| -> Result<Option<String>, Infallible> { Ok(Some(VALID_BASE.to_owned())) };
        state.read_file(&base).expect("read base");
        let game = |_: &str| -> Result<Option<String>, Infallible> { Ok(Some(VALID_MOD.to_owned())) };
        state.append_file(&game).expect("append mod");
        assert_eq!(&state.bytes[0..16], VALID_BASE.as_bytes());
        assert_eq!(&state.bytes[16..32], VALID_MOD.as_bytes());
        assert_eq!(state.bytes[32], 0);

        let mut state = Q3CdKeyState::default();
        state.read_file(&base).expect("read base");
        let missing = |_: &str| -> Result<Option<String>, Infallible> { Ok(None) };
        state.append_file(&missing).expect("append missing");
        assert_eq!(&state.bytes[16..32], &[32u8; 16]);
        assert_eq!(state.bytes[32], 0);
    }

    #[test]
    fn append_file_rejects_overflow() {
        let mut state = Q3CdKeyState::default();
        let mut registry = cvars();
        state
            .write_ui(1, "missionpack", &[b'X'; 16], &mut registry)
            .expect("write mod half");
        let game = |_: &str| -> Result<Option<String>, Infallible> { Ok(Some(VALID_MOD.to_owned())) };
        let error = state.append_file(&game).expect_err("overflow");
        assert!(matches!(error, Q3CdKeyFileError::Key(Q3CdKeyError::StorageOverflow)));
    }

    #[test]
    fn write_file_dumps_valid_halves_and_skips_invalid() {
        let dumped = RefCell::new(Vec::new());
        let dump = |name: &str, contents: &str| -> Result<(), Infallible> {
            dumped.borrow_mut().push((name.to_owned(), contents.to_owned()));
            Ok(())
        };
        let mut state = Q3CdKeyState::default();
        state.write_file(&dump, Q3KeySlot::Base).expect("skip invalid");
        assert!(dumped.borrow().is_empty());

        let mut registry = cvars();
        state
            .write_ui(0, "", VALID_BASE.as_bytes(), &mut registry)
            .expect("write base");
        state.write_file(&dump, Q3KeySlot::Base).expect("dump base");
        let dumped = dumped.borrow();
        assert_eq!(dumped.len(), 1);
        assert_eq!(dumped[0].0, "q3key");
        assert_eq!(
            dumped[0].1,
            format!(
                "{VALID_BASE}\n// generated by quake, do not modify\r\n// Do not give this file to ANYONE.\r\n// id Software and Activision will NOT ask you to send this file to them.\r\n"
            )
        );
        assert_eq!(Q3KeySlot::from_offset(0), Some(Q3KeySlot::Base));
        assert_eq!(Q3KeySlot::from_offset(16), Some(Q3KeySlot::Game));
        assert_eq!(Q3KeySlot::from_offset(8), None);
    }

    #[test]
    fn read_authorization_fills_key_and_rejects_short() {
        let mut state = Q3CdKeyState::default();
        let load = |_: &str| -> Result<Option<String>, Infallible> { Ok(Some(VALID_BASE.to_owned())) };
        state.read_file(&load).expect("read base");
        let mut destination = [0xFFu8; 33];
        state.read_authorization(&mut destination).expect("authorize");
        assert_eq!(&destination[0..16], VALID_BASE.as_bytes());
        assert_eq!(destination[32], 0);
        let mut short = [0u8; 32];
        assert_eq!(
            state.read_authorization(&mut short),
            Err(Q3CdKeyError::AuthorizationTooShort)
        );
    }

    struct Sink {
        copied: Vec<u8>,
        bytes: [u8; 17],
    }

    impl Q3CdKeyUiWrites for Sink {
        fn copy(&mut self, bytes: &[u8]) {
            self.copied = bytes.to_vec();
        }

        fn set_byte(&mut self, offset: usize, value: u8) {
            self.bytes[offset] = value;
        }
    }

    #[test]
    fn read_ui_selects_half_and_honors_sink() {
        let mut state = Q3CdKeyState::default();
        let base = |_: &str| -> Result<Option<String>, Infallible> { Ok(Some(VALID_BASE.to_owned())) };
        state.read_file(&base).expect("read base");
        let game = |_: &str| -> Result<Option<String>, Infallible> { Ok(Some(VALID_MOD.to_owned())) };
        state.append_file(&game).expect("append mod");

        let mut destination = [0xFFu8; 17];
        state.read_ui(0, "", &mut destination, None).expect("read base half");
        assert_eq!(&destination[0..16], VALID_BASE.as_bytes());
        assert_eq!(destination[16], 0);

        state
            .read_ui(1, "missionpack", &mut destination, None)
            .expect("read mod half");
        assert_eq!(&destination[0..16], VALID_MOD.as_bytes());
        state.read_ui(1, "", &mut destination, None).expect("read base half");
        assert_eq!(&destination[0..16], VALID_BASE.as_bytes());

        let mut sink = Sink {
            copied: Vec::new(),
            bytes: [0xFFu8; 17],
        };
        let mut untouched = [0xFFu8; 17];
        state
            .read_ui(1, "missionpack", &mut untouched, Some(&mut sink))
            .expect("read through sink");
        assert_eq!(sink.copied, VALID_MOD.as_bytes());
        assert_eq!(sink.bytes[16], 0);
        assert_eq!(untouched, [0xFFu8; 17]);

        let mut short = [0u8; 5];
        assert_eq!(
            state.read_ui(0, "", &mut short, None),
            Err(Q3CdKeyError::UiDestinationTooShort)
        );
    }

    #[test]
    fn write_ui_stores_half_and_marks_archive() {
        let mut state = Q3CdKeyState::default();
        let mut registry = cvars();
        state
            .write_ui(1, "missionpack", VALID_MOD.as_bytes(), &mut registry)
            .expect("write mod half");
        assert_eq!(&state.bytes[16..32], VALID_MOD.as_bytes());
        assert_eq!(state.bytes[32], 0);
        assert_eq!(registry.take_modified_flags(), flags::ARCHIVE);

        state
            .write_ui(0, "", VALID_BASE.as_bytes(), &mut registry)
            .expect("write base half");
        assert_eq!(&state.bytes[0..16], VALID_BASE.as_bytes());

        assert_eq!(
            state.write_ui(0, "", &[0u8; 8], &mut registry),
            Err(Q3CdKeyError::UiSourceTooShort)
        );
        assert!(matches!(
            state.write_ui(0, "€", VALID_BASE.as_bytes(), &mut registry),
            Err(Q3CdKeyError::CommandText(_))
        ));
    }
}
