//! Script storage for legacy menu scripts.
//!
//! Donor provenance: `src/ui/common/legacy/script/memory.ts`
//! (script storage from id Software's `botlib/l_script.c` and `l_script.h`).
//!
//! [`SourceScriptStorage`] owns one `LoadScriptFile`/`LoadScriptMemory`
//! allocation: filename bytes, buffer pointer words, an embedded token, and
//! an optional punctuation table. Buffer pointer words are offsets into the
//! block; punctuation words resolve owner-local ids to the lexer's static
//! punctuation records. Layout is the Linux i386 donor profile:
//!
//! | Range     | Field                  | Type              |
//! |-----------|------------------------|-------------------|
//! | 0..1024   | filename bytes         | NUL-terminated Latin-1 |
//! | 1024..1028 | buffer pointer        | `u32` offset      |
//! | 1028..1032 | script pointer        | `u32` offset      |
//! | 1032..1036 | end pointer           | `u32` offset      |
//! | 1036..1040 | last script pointer   | `u32` offset      |
//! | 1040..1044 | whitespace pointer    | `u32` offset      |
//! | 1044..1048 | end whitespace pointer | `u32` offset     |
//! | 1048..1052 | length                | `i32`             |
//! | 1052..1056 | line                  | `i32`             |
//! | 1056..1060 | last line             | `i32`             |
//! | 1060..1064 | token available       | `i32`             |
//! | 1064..1068 | flags                 | `i32`             |
//! | 1068..1072 | punctuations flag     | `u32` (`0`/`1`)   |
//! | 1072..1076 | punctuation table flag | `u32` (`0`/`1`)  |
//! | 1076..2144 | embedded token        | [`SOURCE_TOKEN_BYTES`] bytes |
//! | 2144..2148 | next script id        | `u32`             |
//!
//! [`SOURCE_TOKEN_BYTES`]: super::token_memory::SOURCE_TOKEN_BYTES
//!
//! # Adaptations
//!
//! * [`ScriptMemory`], [`ScriptMemoryAllocation`], [`ScriptMemoryCapture`],
//!   and [`ScriptMemoryRestore`] are re-exported from
//!   [`super::precomp_memory`], which already mirrors them; the donor
//!   `allocate(size, kind, clear)` triple folds into
//!   [`ScriptMemory::allocate`], which always returns zeroed blocks.
//! * The donor stores the `ScriptMemory` owner on the instance. This port
//!   holds it as `Box<dyn ScriptMemory>`, like [`super::precomp_memory`]'s
//!   owner, so [`SourceScriptStorage::dispose`] and
//!   [`SourceScriptStorage::set_default_punctuations`] keep their donor
//!   arities.
//! * The donor token views allocation bytes `1076..2144` through a borrow
//!   closure. Allocation sub-views are module-private in
//!   [`super::precomp_memory`], so the token is a standalone
//!   [`SourceTokenMemory`] synced into the allocation on
//!   [`SourceScriptStorage::capture_save_state`] and restored with
//!   write-back plus sync on
//!   [`SourceScriptStorage::restore_save_state`]. No donor path reads the
//!   token area except through the token or a checkpoint image, so the
//!   split is unobservable outside checkpoints, where the token round-trips
//!   through its own save state.
//! * The donor `buffer` getter returns a live subarray; [`buffer`][Self::buffer]
//!   returns a snapshot instead, and [`copy_text`][Self::copy_text] writes
//!   through the public byte accessors.
//! * `SaveReader` checkpoints become the typed [`SourceScriptSaveState`].
//!   Every donor `RangeError` (and the internal-misuse `Error`s) becomes
//!   [`ClientError::BadUi`] carrying the donor message; save-envelope
//!   validations keep their `script.storage: ` path prefix, matching
//!   `SaveReader` roots.
//! * Pointer-difference getters (`offset`, `last_offset`) return `i64` so
//!   corrupt states below the base stay representable, as in the donor's
//!   number arithmetic.
//! * The donor file performs no asynchronous reads, so this port is sync.
//!
//! [`SourceTokenMemory`]: super::token_memory::SourceTokenMemory

pub use super::precomp_memory::{ScriptMemory, ScriptMemoryAllocation, ScriptMemoryCapture, ScriptMemoryRestore};
use super::token_memory::{SourceTokenMemory, SourceTokenSaveState, SOURCE_TOKEN_BYTES};
use crate::error::ClientError;

/// Size of one `script_t` record, in bytes (donor `SOURCE_SCRIPT_BYTES`).
pub const SOURCE_SCRIPT_BYTES: usize = 2148;

/// Size of the punctuation head table, in bytes
/// (donor `SOURCE_PUNCTUATION_TABLE_BYTES`).
pub const SOURCE_PUNCTUATION_TABLE_BYTES: usize = 256 * 4;

/// Filename field length, in bytes.
const FILENAME_BYTES: usize = 1024;
/// Offset of the buffer pointer word.
const BUFFER: usize = 1024;
/// Offset of the script pointer word.
const SCRIPT_POINTER: usize = 1028;
/// Offset of the end pointer word.
const END_POINTER: usize = 1032;
/// Offset of the last script pointer word.
const LAST_SCRIPT_POINTER: usize = 1036;
/// Offset of the whitespace pointer word.
const WHITESPACE_POINTER: usize = 1040;
/// Offset of the end whitespace pointer word.
const END_WHITESPACE_POINTER: usize = 1044;
/// Offset of the length word.
const LENGTH: usize = 1048;
/// Offset of the line word.
const LINE: usize = 1052;
/// Offset of the last line word.
const LAST_LINE: usize = 1056;
/// Offset of the token available word.
const TOKEN_AVAILABLE: usize = 1060;
/// Offset of the flags word.
const FLAGS: usize = 1064;
/// Offset of the punctuations flag word.
const PUNCTUATIONS: usize = 1068;
/// Offset of the punctuation table flag word.
const PUNCTUATION_TABLE: usize = 1072;
/// Offset of the embedded token.
const TOKEN: usize = 1076;
/// Offset of the next script word.
const NEXT_SCRIPT: usize = 2144;

/// Read a little-endian `u32` word through the public byte accessors.
fn read_u32(allocation: &ScriptMemoryAllocation, offset: usize) -> Result<u32, ClientError> {
    let mut bytes = [0u8; 4];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = allocation.byte(offset + index)?;
    }
    Ok(u32::from_le_bytes(bytes))
}

/// Read a little-endian `i32` word through the public byte accessors.
fn read_i32(allocation: &ScriptMemoryAllocation, offset: usize) -> Result<i32, ClientError> {
    let mut bytes = [0u8; 4];
    for (index, slot) in bytes.iter_mut().enumerate() {
        *slot = allocation.byte(offset + index)?;
    }
    Ok(i32::from_le_bytes(bytes))
}

/// Write a little-endian `u32` word through the public byte accessors.
fn write_u32(allocation: &mut ScriptMemoryAllocation, offset: usize, value: u32) -> Result<(), ClientError> {
    for (index, byte) in value.to_le_bytes().iter().enumerate() {
        allocation.set_byte(offset + index, *byte)?;
    }
    Ok(())
}

/// Write a little-endian `i32` word through the public byte accessors.
fn write_i32(allocation: &mut ScriptMemoryAllocation, offset: usize, value: i32) -> Result<(), ClientError> {
    for (index, byte) in value.to_le_bytes().iter().enumerate() {
        allocation.set_byte(offset + index, *byte)?;
    }
    Ok(())
}

/// Checkpoint of one script storage (donor `captureSaveState` record).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceScriptSaveState {
    /// Checkpoint reference of the script allocation.
    pub allocation: u32,
    /// Checkpoint reference of the punctuation table, if published.
    pub punctuation_table: Option<u32>,
    /// Embedded token checkpoint.
    pub token: SourceTokenSaveState,
}

/// One `LoadScriptFile`/`LoadScriptMemory` allocation and its punctuation
/// table (donor `SourceScriptStorage`).
pub struct SourceScriptStorage {
    memory: Box<dyn ScriptMemory>,
    allocation: ScriptMemoryAllocation,
    punctuation_table: Option<ScriptMemoryAllocation>,
    token: SourceTokenMemory,
    disposed: bool,
}

impl std::fmt::Debug for SourceScriptStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceScriptStorage")
            .field("allocation_len", &self.allocation.len())
            .field("has_punctuation_table", &self.punctuation_table.is_some())
            .field("token", &self.token)
            .field("disposed", &self.disposed)
            .finish()
    }
}

impl SourceScriptStorage {
    /// Capture a checkpoint (donor `captureSaveState`).
    pub fn capture_save_state(
        &mut self,
        capture: &mut dyn ScriptMemoryCapture,
    ) -> Result<SourceScriptSaveState, ClientError> {
        if self.disposed {
            return Err(ClientError::BadUi(
                "Cannot checkpoint disposed script storage".to_string(),
            ));
        }
        self.sync_token_to_allocation()?;
        Ok(SourceScriptSaveState {
            allocation: capture.reference(&self.allocation),
            punctuation_table: self.punctuation_table.as_ref().map(|table| capture.reference(table)),
            token: self.token.capture_save_state()?,
        })
    }

    /// Restore a checkpoint (donor `restoreSaveState`).
    pub fn restore_save_state(
        state: &SourceScriptSaveState,
        memory: Box<dyn ScriptMemory>,
        restore: &mut dyn ScriptMemoryRestore,
    ) -> Result<Self, ClientError> {
        let allocation = restore.allocation(state.allocation)?;
        if allocation.len() < SOURCE_SCRIPT_BYTES + 1 {
            return Err(ClientError::BadUi(
                "script.storage: invalid script storage extent".to_string(),
            ));
        }
        let punctuation_table = match state.punctuation_table {
            None => None,
            Some(id) => {
                let table = restore.allocation(id)?;
                if table.len() != SOURCE_PUNCTUATION_TABLE_BYTES {
                    return Err(ClientError::BadUi(
                        "script.storage: invalid punctuation table extent".to_string(),
                    ));
                }
                Some(table)
            }
        };
        let mut token = SourceTokenMemory::new();
        token.restore_save_state(&state.token, false)?;
        let mut script = Self {
            memory,
            allocation,
            punctuation_table,
            token,
            disposed: false,
        };
        script.sync_token_to_allocation()?;
        script.buffer()?;
        script.path()?;
        Ok(script)
    }

    /// Allocate storage for `length` source bytes under `path`
    /// (donor `SourceScriptStorage.allocate`).
    pub fn allocate(length: i32, path: &str, mut memory: Box<dyn ScriptMemory>) -> Result<Self, ClientError> {
        if length < 0 || length > i32::MAX - SOURCE_SCRIPT_BYTES as i32 - 1 {
            return Err(ClientError::BadUi(
                "script allocation must fit its nonnegative source signed size".to_string(),
            ));
        }
        let mut allocation = memory.allocate(SOURCE_SCRIPT_BYTES + length as usize + 1);
        let mut written = 0;
        for (index, character) in path.chars().enumerate() {
            if index >= FILENAME_BYTES {
                return Err(ClientError::BadUi(
                    "LoadScript filename exceeds its 1024-byte source allocation".to_string(),
                ));
            }
            if character as u32 > 255 {
                return Err(ClientError::BadUi(
                    "LoadScript filename requires source byte characters".to_string(),
                ));
            }
            allocation.set_byte(index, character as u8)?;
            written = index + 1;
        }
        if written >= FILENAME_BYTES {
            return Err(ClientError::BadUi(
                "LoadScript filename exceeds its 1024-byte source allocation".to_string(),
            ));
        }
        allocation.set_byte(written, 0)?;
        write_u32(&mut allocation, BUFFER, SOURCE_SCRIPT_BYTES as u32)?;
        write_i32(&mut allocation, LENGTH, length)?;
        write_u32(&mut allocation, SCRIPT_POINTER, SOURCE_SCRIPT_BYTES as u32)?;
        write_u32(&mut allocation, LAST_SCRIPT_POINTER, SOURCE_SCRIPT_BYTES as u32)?;
        write_u32(&mut allocation, END_POINTER, SOURCE_SCRIPT_BYTES as u32 + length as u32)?;
        write_i32(&mut allocation, TOKEN_AVAILABLE, 0)?;
        write_i32(&mut allocation, LINE, 1)?;
        write_i32(&mut allocation, LAST_LINE, 1)?;
        Ok(Self {
            memory,
            allocation,
            punctuation_table: None,
            token: SourceTokenMemory::new(),
            disposed: false,
        })
    }

    /// Publish the 256-head punctuation table (donor
    /// `setDefaultPunctuations`). Allocates, clears, and populates the table
    /// before publishing its set.
    pub fn set_default_punctuations(&mut self, heads: &[u32]) -> Result<(), ClientError> {
        self.ensure_live()?;
        if heads.len() != 256 {
            return Err(ClientError::BadUi(
                "script punctuation table requires 256 heads".to_string(),
            ));
        }
        if read_u32(&self.allocation, PUNCTUATION_TABLE)? == 0 {
            self.punctuation_table = Some(self.memory.allocate(SOURCE_PUNCTUATION_TABLE_BYTES));
            write_u32(&mut self.allocation, PUNCTUATION_TABLE, 1)?;
        }
        if read_u32(&self.allocation, PUNCTUATION_TABLE)? != 1 || self.punctuation_table.is_none() {
            return Err(ClientError::BadUi(
                "script punctuation pointer does not identify its table allocation".to_string(),
            ));
        }
        let table = self.punctuation_table.as_mut().ok_or_else(|| {
            ClientError::BadUi("script punctuation pointer does not identify its table allocation".to_string())
        })?;
        table.fill(0)?;
        for (index, head) in heads.iter().enumerate() {
            write_u32(table, index * 4, *head)?;
        }
        write_u32(&mut self.allocation, PUNCTUATIONS, 1)?;
        Ok(())
    }

    /// Embedded retained token (donor `token`).
    #[must_use]
    pub fn token(&self) -> &SourceTokenMemory {
        &self.token
    }

    /// Embedded retained token for mutation (donor `token`).
    ///
    /// Writes sync into the allocation on [`Self::capture_save_state`].
    pub fn token_mut(&mut self) -> &mut SourceTokenMemory {
        &mut self.token
    }

    /// Script path (donor `path` getter).
    pub fn path(&self) -> Result<String, ClientError> {
        self.ensure_live()?;
        let mut result = String::new();
        for index in 0..FILENAME_BYTES {
            let byte = self
                .allocation
                .byte(index)
                .map_err(|_| ClientError::BadUi("script filename is outside its allocation".to_string()))?;
            if byte == 0 {
                return Ok(result);
            }
            result.push(char::from(byte));
        }
        Err(ClientError::BadUi(
            "script filename lacks its source terminator".to_string(),
        ))
    }

    /// Compatibility observation of the loaded text (donor `text` getter).
    ///
    /// Scanners retain this storage, never this copy.
    pub fn text(&self) -> Result<String, ClientError> {
        let mut text = String::new();
        for byte in self.buffer()? {
            if byte == 0 {
                break;
            }
            text.push(char::from(byte));
        }
        Ok(text)
    }

    /// Borrow the loaded source bytes, excluding the initialized NUL byte
    /// (donor `buffer` getter).
    pub fn buffer(&self) -> Result<Vec<u8>, ClientError> {
        self.ensure_live()?;
        let start = read_u32(&self.allocation, BUFFER)?;
        let end = read_u32(&self.allocation, END_POINTER)?;
        if start != SOURCE_SCRIPT_BYTES as u32 || end < start || (end as usize) >= self.allocation.len() {
            return Err(ClientError::BadUi(
                "script buffer pointers are outside their allocation".to_string(),
            ));
        }
        let bytes = self.allocation.snapshot_bytes()?;
        Ok(bytes[start as usize..end as usize].to_vec())
    }

    /// Source length word (donor `length` getter).
    pub fn length(&self) -> Result<i32, ClientError> {
        self.ensure_live()?;
        read_i32(&self.allocation, LENGTH)
    }

    /// Next script id (donor `nextScript` getter).
    pub fn next_script(&self) -> Result<u32, ClientError> {
        self.ensure_live()?;
        read_u32(&self.allocation, NEXT_SCRIPT)
    }

    /// Set the next script id (donor `nextScript` setter).
    pub fn set_next_script(&mut self, value: u32) -> Result<(), ClientError> {
        self.ensure_live()?;
        write_u32(&mut self.allocation, NEXT_SCRIPT, value)
    }

    /// Current scan offset past the record base (donor `offset` getter).
    pub fn offset(&self) -> Result<i64, ClientError> {
        self.ensure_live()?;
        Ok(i64::from(read_u32(&self.allocation, SCRIPT_POINTER)?) - SOURCE_SCRIPT_BYTES as i64)
    }

    /// Set the current scan offset (donor `offset` setter).
    pub fn set_offset(&mut self, value: i64) -> Result<(), ClientError> {
        self.ensure_live()?;
        write_u32(
            &mut self.allocation,
            SCRIPT_POINTER,
            (SOURCE_SCRIPT_BYTES as i64 + value) as u32,
        )
    }

    /// Scan offset at the previous token start (donor `lastOffset` getter).
    pub fn last_offset(&self) -> Result<i64, ClientError> {
        self.ensure_live()?;
        Ok(i64::from(read_u32(&self.allocation, LAST_SCRIPT_POINTER)?) - SOURCE_SCRIPT_BYTES as i64)
    }

    /// Lines crossed since the previous token start (donor `linesCrossed`
    /// getter).
    pub fn lines_crossed(&self) -> Result<i32, ClientError> {
        self.ensure_live()?;
        Ok(self.line()?.wrapping_sub(read_i32(&self.allocation, LAST_LINE)?))
    }

    /// Consume one whitespace byte as a signed source char (donor
    /// `nextWhitespaceChar`).
    pub fn next_whitespace_char(&mut self) -> Result<i32, ClientError> {
        self.ensure_live()?;
        let pointer = read_u32(&self.allocation, WHITESPACE_POINTER)?;
        if pointer == read_u32(&self.allocation, END_WHITESPACE_POINTER)? {
            return Ok(0);
        }
        if pointer < SOURCE_SCRIPT_BYTES as u32 || (pointer as usize) >= self.allocation.len() {
            return Err(ClientError::BadUi(
                "script whitespace pointer exceeds its source allocation".to_string(),
            ));
        }
        let byte = self.allocation.byte(pointer as usize)?;
        write_u32(&mut self.allocation, WHITESPACE_POINTER, pointer.wrapping_add(1))?;
        Ok(i32::from(byte as i8))
    }

    /// Current line (donor `line` getter).
    pub fn line(&self) -> Result<i32, ClientError> {
        self.ensure_live()?;
        read_i32(&self.allocation, LINE)
    }

    /// Set the current line (donor `line` setter).
    pub fn set_line(&mut self, value: i32) -> Result<(), ClientError> {
        self.ensure_live()?;
        write_i32(&mut self.allocation, LINE, value)
    }

    /// Lexer flags (donor `flags` getter).
    pub fn flags(&self) -> Result<i32, ClientError> {
        self.ensure_live()?;
        read_i32(&self.allocation, FLAGS)
    }

    /// Set the lexer flags (donor `flags` setter).
    pub fn set_flags(&mut self, value: i32) -> Result<(), ClientError> {
        self.ensure_live()?;
        write_i32(&mut self.allocation, FLAGS, value)
    }

    /// Whether a pushed-back token awaits reading (donor `tokenAvailable`
    /// getter).
    pub fn token_available(&self) -> Result<bool, ClientError> {
        self.ensure_live()?;
        Ok(read_i32(&self.allocation, TOKEN_AVAILABLE)? != 0)
    }

    /// Set the push-back flag (donor `tokenAvailable` setter).
    pub fn set_token_available(&mut self, value: bool) -> Result<(), ClientError> {
        self.ensure_live()?;
        write_i32(&mut self.allocation, TOKEN_AVAILABLE, i32::from(value))
    }

    /// Mark the start of a token (donor `beginToken`).
    pub fn begin_token(&mut self) -> Result<(), ClientError> {
        self.ensure_live()?;
        let pointer = read_u32(&self.allocation, SCRIPT_POINTER)?;
        write_u32(&mut self.allocation, LAST_SCRIPT_POINTER, pointer)?;
        let line = read_i32(&self.allocation, LINE)?;
        write_i32(&mut self.allocation, LAST_LINE, line)?;
        write_u32(&mut self.allocation, WHITESPACE_POINTER, pointer)?;
        Ok(())
    }

    /// Mark the end of the token's leading whitespace (donor
    /// `endWhitespace`).
    pub fn end_whitespace(&mut self) -> Result<(), ClientError> {
        self.ensure_live()?;
        let pointer = read_u32(&self.allocation, SCRIPT_POINTER)?;
        write_u32(&mut self.allocation, END_WHITESPACE_POINTER, pointer)?;
        Ok(())
    }

    /// Head punctuation id for one source byte (donor `punctuationHead`).
    pub fn punctuation_head(&self, character: i32) -> Result<u32, ClientError> {
        self.ensure_live()?;
        if read_u32(&self.allocation, PUNCTUATIONS)? != 1
            || read_u32(&self.allocation, PUNCTUATION_TABLE)? != 1
            || self.punctuation_table.is_none()
        {
            return Err(ClientError::BadUi(
                "script punctuation pointer does not identify its default table".to_string(),
            ));
        }
        if !(0..=255).contains(&character) {
            return Err(ClientError::BadUi(
                "script punctuation lookup requires a source byte".to_string(),
            ));
        }
        let table = self.punctuation_table.as_ref().ok_or_else(|| {
            ClientError::BadUi("script punctuation pointer does not identify its default table".to_string())
        })?;
        read_u32(table, character as usize * 4)
    }

    /// Copy loaded text into the buffer (donor `copyText`).
    ///
    /// Runs after both allocations and punctuation publication, like
    /// `LoadScriptMemory`.
    pub fn copy_text(&mut self, text: &str) -> Result<(), ClientError> {
        self.ensure_live()?;
        let start = read_u32(&self.allocation, BUFFER)?;
        let end = read_u32(&self.allocation, END_POINTER)?;
        if start != SOURCE_SCRIPT_BYTES as u32 || end < start || (end as usize) >= self.allocation.len() {
            return Err(ClientError::BadUi(
                "script buffer pointers are outside their allocation".to_string(),
            ));
        }
        let units: Vec<u16> = text.encode_utf16().collect();
        if units.len() != (end - start) as usize {
            return Err(ClientError::BadUi(
                "script text does not match its allocated length".to_string(),
            ));
        }
        for (index, unit) in units.iter().enumerate() {
            if *unit > 255 {
                return Err(ClientError::BadUi(format!(
                    "LoadScriptMemory input is not a source byte at {index}"
                )));
            }
            self.allocation.set_byte(start as usize + index, *unit as u8)?;
        }
        Ok(())
    }

    /// Compress the loaded buffer in place without moving the end pointer
    /// (donor `compress`, `COM_Compress`).
    pub fn compress(&mut self) -> Result<(), ClientError> {
        self.ensure_live()?;
        let start = read_u32(&self.allocation, BUFFER)? as usize;
        let mut input = start;
        let mut output = start;
        let mut newline = false;
        let mut whitespace = false;
        loop {
            let byte = self.compress_read(input)?;
            if byte == 0 {
                break;
            }
            if byte == 47 && self.compress_read(input + 1)? == 47 {
                loop {
                    let skipped = self.compress_read(input)?;
                    if skipped == 0 || skipped == 10 {
                        break;
                    }
                    input += 1;
                }
            } else if byte == 47 && self.compress_read(input + 1)? == 42 {
                while self.compress_read(input)? != 0
                    && (self.compress_read(input)? != 42 || self.compress_read(input + 1)? != 47)
                {
                    input += 1;
                }
                if self.compress_read(input)? != 0 {
                    input += 2;
                }
            } else if byte == 10 || byte == 13 {
                newline = true;
                input += 1;
            } else if byte == 32 || byte == 9 {
                whitespace = true;
                input += 1;
            } else {
                if newline {
                    self.allocation.set_byte(output, 10)?;
                    output += 1;
                    newline = false;
                    whitespace = false;
                }
                if whitespace {
                    self.allocation.set_byte(output, 32)?;
                    output += 1;
                    whitespace = false;
                }
                self.allocation.set_byte(output, self.compress_read(input)?)?;
                output += 1;
                input += 1;
                if byte == 34 {
                    loop {
                        let quoted = self.compress_read(input)?;
                        if quoted == 0 || quoted == 34 {
                            break;
                        }
                        self.allocation.set_byte(output, quoted)?;
                        output += 1;
                        input += 1;
                    }
                    if self.compress_read(input)? == 34 {
                        self.allocation.set_byte(output, 34)?;
                        output += 1;
                        input += 1;
                    }
                }
            }
        }
        self.allocation.set_byte(output, 0)?;
        write_i32(&mut self.allocation, LENGTH, (output - start) as i32)?;
        Ok(())
    }

    /// Rewind to the buffer start and clear the token (donor `reset`).
    pub fn reset(&mut self) -> Result<(), ClientError> {
        self.ensure_live()?;
        let pointer = read_u32(&self.allocation, BUFFER)?;
        write_u32(&mut self.allocation, SCRIPT_POINTER, pointer)?;
        write_u32(&mut self.allocation, LAST_SCRIPT_POINTER, pointer)?;
        write_u32(&mut self.allocation, WHITESPACE_POINTER, 0)?;
        write_u32(&mut self.allocation, END_WHITESPACE_POINTER, 0)?;
        write_i32(&mut self.allocation, TOKEN_AVAILABLE, 0)?;
        write_i32(&mut self.allocation, LINE, 1)?;
        write_i32(&mut self.allocation, LAST_LINE, 1)?;
        self.token.clear()?;
        Ok(())
    }

    /// Free the punctuation table before its containing script block (donor
    /// `dispose`, `FreeScript`). Idempotent.
    pub fn dispose(&mut self) -> Result<(), ClientError> {
        if self.disposed {
            return Ok(());
        }
        let pointer = read_u32(&self.allocation, PUNCTUATION_TABLE)?;
        if pointer != 0 {
            match self.punctuation_table.as_ref() {
                Some(table) if pointer == 1 => self.memory.free(table),
                _ => {
                    return Err(ClientError::BadUi(
                        "script punctuation pointer does not identify its table allocation".to_string(),
                    ))
                }
            }
        }
        let allocation = std::mem::replace(&mut self.allocation, ScriptMemoryAllocation::zeroed(0));
        self.memory.free(&allocation);
        self.disposed = true;
        Ok(())
    }

    /// Reject access after [`Self::dispose`] (donor `bytes` getter).
    fn ensure_live(&self) -> Result<(), ClientError> {
        if self.disposed {
            return Err(ClientError::BadUi("script storage has been freed".to_string()));
        }
        Ok(())
    }

    /// Copy the standalone token image into the allocation token area.
    fn sync_token_to_allocation(&mut self) -> Result<(), ClientError> {
        let bytes = self.token.bytes()?;
        debug_assert_eq!(bytes.len(), SOURCE_TOKEN_BYTES);
        for (index, byte) in bytes.iter().enumerate() {
            self.allocation.set_byte(TOKEN + index, *byte)?;
        }
        Ok(())
    }

    /// Bounds-checked compress read (donor `read` closure).
    fn compress_read(&self, offset: usize) -> Result<u8, ClientError> {
        self.allocation
            .byte(offset)
            .map_err(|_| ClientError::BadUi("COM_Compress read outside its script allocation".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    #[derive(Debug, Default)]
    struct Probe {
        allocs: usize,
        frees: usize,
    }

    struct ProbeMemory {
        probe: Rc<RefCell<Probe>>,
    }

    impl ScriptMemory for ProbeMemory {
        fn allocate(&mut self, size: usize) -> ScriptMemoryAllocation {
            self.probe.borrow_mut().allocs += 1;
            ScriptMemoryAllocation::zeroed(size)
        }

        fn free(&mut self, _allocation: &ScriptMemoryAllocation) {
            self.probe.borrow_mut().frees += 1;
        }
    }

    fn probe_memory() -> (Box<dyn ScriptMemory>, Rc<RefCell<Probe>>) {
        let probe = Rc::new(RefCell::new(Probe::default()));
        let memory: Box<dyn ScriptMemory> = Box::new(ProbeMemory { probe: probe.clone() });
        (memory, probe)
    }

    struct MapCapture {
        next: u32,
        map: HashMap<u32, ScriptMemoryAllocation>,
    }

    impl ScriptMemoryCapture for MapCapture {
        fn reference(&mut self, allocation: &ScriptMemoryAllocation) -> u32 {
            let id = self.next;
            self.next += 1;
            self.map.insert(id, allocation.clone());
            id
        }
    }

    struct MapRestore {
        map: HashMap<u32, ScriptMemoryAllocation>,
    }

    impl ScriptMemoryRestore for MapRestore {
        fn allocation(&mut self, id: u32) -> Result<ScriptMemoryAllocation, ClientError> {
            self.map
                .get(&id)
                .cloned()
                .ok_or_else(|| ClientError::BadUi(format!("script.storage: missing allocation {id}")))
        }
    }

    fn bad_ui(error: ClientError) -> String {
        match error {
            ClientError::BadUi(message) => message,
            other => panic!("expected BadUi, got {other:?}"),
        }
    }

    #[test]
    fn allocate_lays_out_header_words() {
        let (memory, probe) = probe_memory();
        let storage = SourceScriptStorage::allocate(5, "q3.shader", memory).unwrap();
        assert_eq!(probe.borrow().allocs, 1);
        assert_eq!(storage.length().unwrap(), 5);
        assert_eq!(storage.path().unwrap(), "q3.shader");
        assert_eq!(storage.buffer().unwrap(), vec![0, 0, 0, 0, 0]);
        assert_eq!(storage.offset().unwrap(), 0);
        assert_eq!(storage.last_offset().unwrap(), 0);
        assert_eq!(storage.line().unwrap(), 1);
        assert_eq!(storage.lines_crossed().unwrap(), 0);
        assert!(!storage.token_available().unwrap());
        assert_eq!(storage.next_script().unwrap(), 0);
        assert_eq!(storage.flags().unwrap(), 0);
    }

    #[test]
    fn allocate_rejects_bad_lengths_and_names() {
        let (memory, _) = probe_memory();
        assert_eq!(
            bad_ui(SourceScriptStorage::allocate(-1, "a", memory).unwrap_err()),
            "script allocation must fit its nonnegative source signed size"
        );
        let (memory, _) = probe_memory();
        assert_eq!(
            bad_ui(SourceScriptStorage::allocate(i32::MAX, "a", memory).unwrap_err()),
            "script allocation must fit its nonnegative source signed size"
        );
        let (memory, _) = probe_memory();
        assert_eq!(
            bad_ui(SourceScriptStorage::allocate(0, &"p".repeat(1024), memory).unwrap_err()),
            "LoadScript filename exceeds its 1024-byte source allocation"
        );
        let (memory, _) = probe_memory();
        assert_eq!(
            bad_ui(SourceScriptStorage::allocate(0, "caf\u{100}", memory).unwrap_err()),
            "LoadScript filename requires source byte characters"
        );
    }

    #[test]
    fn copy_text_round_trips_and_validates() {
        let (memory, _) = probe_memory();
        let mut storage = SourceScriptStorage::allocate(5, "m", memory).unwrap();
        storage.copy_text("hello").unwrap();
        assert_eq!(storage.text().unwrap(), "hello");
        assert_eq!(storage.buffer().unwrap(), b"hello".to_vec());
        assert_eq!(
            bad_ui(storage.copy_text("toolong").unwrap_err()),
            "script text does not match its allocated length"
        );
        assert_eq!(
            bad_ui(storage.copy_text("hell\u{100}").unwrap_err()),
            "LoadScriptMemory input is not a source byte at 4"
        );
    }

    #[test]
    fn offset_and_next_script_round_trip() {
        let (memory, _) = probe_memory();
        let mut storage = SourceScriptStorage::allocate(8, "m", memory).unwrap();
        storage.set_offset(3).unwrap();
        assert_eq!(storage.offset().unwrap(), 3);
        assert_eq!(storage.last_offset().unwrap(), 0);
        storage.set_next_script(41).unwrap();
        assert_eq!(storage.next_script().unwrap(), 41);
        storage.set_line(7).unwrap();
        assert_eq!(storage.lines_crossed().unwrap(), 6);
        storage.set_token_available(true).unwrap();
        assert!(storage.token_available().unwrap());
        storage.set_flags(-2).unwrap();
        assert_eq!(storage.flags().unwrap(), -2);
    }

    #[test]
    fn punctuation_table_requires_publication_and_bytes() {
        let (memory, _) = probe_memory();
        let mut storage = SourceScriptStorage::allocate(1, "m", memory).unwrap();
        assert_eq!(
            bad_ui(storage.punctuation_head(65).unwrap_err()),
            "script punctuation pointer does not identify its default table"
        );
        assert_eq!(
            bad_ui(storage.set_default_punctuations(&[1u32; 3]).unwrap_err()),
            "script punctuation table requires 256 heads"
        );
        let heads: Vec<u32> = (0..256).map(|index| index * 7 + 1).collect();
        storage.set_default_punctuations(&heads).unwrap();
        assert_eq!(storage.punctuation_head(65).unwrap(), 65 * 7 + 1);
        assert_eq!(storage.punctuation_head(0).unwrap(), 1);
        assert_eq!(
            bad_ui(storage.punctuation_head(256).unwrap_err()),
            "script punctuation lookup requires a source byte"
        );
        assert_eq!(
            bad_ui(storage.punctuation_head(-1).unwrap_err()),
            "script punctuation lookup requires a source byte"
        );
    }

    #[test]
    fn whitespace_span_reads_signed_bytes() {
        let (memory, _) = probe_memory();
        let mut storage = SourceScriptStorage::allocate(2, "m", memory).unwrap();
        storage.copy_text("a\u{80}").unwrap();
        storage.begin_token().unwrap();
        storage.set_offset(2).unwrap();
        storage.end_whitespace().unwrap();
        assert_eq!(storage.next_whitespace_char().unwrap(), 97);
        assert_eq!(storage.next_whitespace_char().unwrap(), -128);
        assert_eq!(storage.next_whitespace_char().unwrap(), 0);
    }

    #[test]
    fn compress_strips_comments_and_updates_length() {
        let text = "a  b//c\n d";
        let (memory, _) = probe_memory();
        let mut storage = SourceScriptStorage::allocate(text.len() as i32, "m", memory).unwrap();
        storage.copy_text(text).unwrap();
        storage.compress().unwrap();
        assert_eq!(storage.length().unwrap(), 5);
        assert_eq!(storage.text().unwrap(), "a b\nd");
        assert_eq!(storage.buffer().unwrap().len(), text.len());
    }

    #[test]
    fn reset_rewinds_and_clears_token() {
        let (memory, _) = probe_memory();
        let mut storage = SourceScriptStorage::allocate(3, "m", memory).unwrap();
        storage.copy_text("abc").unwrap();
        storage.set_offset(2).unwrap();
        storage.set_line(9).unwrap();
        storage.set_token_available(true).unwrap();
        storage.token_mut().set_token_type(4).unwrap();
        storage.reset().unwrap();
        assert_eq!(storage.offset().unwrap(), 0);
        assert_eq!(storage.last_offset().unwrap(), 0);
        assert_eq!(storage.line().unwrap(), 1);
        assert!(!storage.token_available().unwrap());
        assert_eq!(storage.token().token_type().unwrap(), 0);
        assert_eq!(storage.next_whitespace_char().unwrap(), 0);
        assert_eq!(storage.text().unwrap(), "abc");
    }

    #[test]
    fn dispose_frees_table_then_block_and_revokes_access() {
        let (memory, probe) = probe_memory();
        let mut storage = SourceScriptStorage::allocate(1, "m", memory).unwrap();
        storage.set_default_punctuations(&[0u32; 256]).unwrap();
        storage.dispose().unwrap();
        assert_eq!(probe.borrow().frees, 2);
        storage.dispose().unwrap();
        assert_eq!(probe.borrow().frees, 2);
        assert_eq!(bad_ui(storage.path().unwrap_err()), "script storage has been freed");
        let mut capture = MapCapture {
            next: 0,
            map: HashMap::new(),
        };
        assert_eq!(
            bad_ui(storage.capture_save_state(&mut capture).unwrap_err()),
            "Cannot checkpoint disposed script storage"
        );
    }

    #[test]
    fn save_state_round_trips_storage() {
        let (memory, _) = probe_memory();
        let mut storage = SourceScriptStorage::allocate(5, "maps/a", memory).unwrap();
        storage.copy_text("hi\"q\"").unwrap();
        let heads: Vec<u32> = (0..256).collect();
        storage.set_default_punctuations(&heads).unwrap();
        storage.token_mut().set_token_type(4).unwrap();
        storage.token_mut().set_subtype(11).unwrap();
        storage.set_offset(2).unwrap();
        storage.set_line(4).unwrap();
        let mut capture = MapCapture {
            next: 0,
            map: HashMap::new(),
        };
        let state = storage.capture_save_state(&mut capture).unwrap();
        assert!(state.punctuation_table.is_some());
        let (memory, _) = probe_memory();
        let mut restore = MapRestore { map: capture.map };
        let restored = SourceScriptStorage::restore_save_state(&state, memory, &mut restore).unwrap();
        assert_eq!(restored.path().unwrap(), "maps/a");
        assert_eq!(restored.text().unwrap(), "hi\"q\"");
        assert_eq!(restored.offset().unwrap(), 2);
        assert_eq!(restored.line().unwrap(), 4);
        assert_eq!(restored.punctuation_head(200).unwrap(), 200);
        assert_eq!(restored.token().token_type().unwrap(), 4);
        assert_eq!(restored.token().subtype().unwrap(), 11);
    }

    #[test]
    fn restore_rejects_bad_extents() {
        let (memory, _) = probe_memory();
        let mut storage = SourceScriptStorage::allocate(2, "m", memory).unwrap();
        let mut capture = MapCapture {
            next: 0,
            map: HashMap::new(),
        };
        let mut state = storage.capture_save_state(&mut capture).unwrap();
        state.allocation = 90;
        capture.map.insert(90, ScriptMemoryAllocation::zeroed(100));
        let (memory, _) = probe_memory();
        let mut restore = MapRestore { map: capture.map };
        assert_eq!(
            bad_ui(SourceScriptStorage::restore_save_state(&state, memory, &mut restore).unwrap_err()),
            "script.storage: invalid script storage extent"
        );
        let (memory, _) = probe_memory();
        let mut storage = SourceScriptStorage::allocate(2, "m", memory).unwrap();
        storage.set_default_punctuations(&[0u32; 256]).unwrap();
        let mut capture = MapCapture {
            next: 0,
            map: HashMap::new(),
        };
        let mut state = storage.capture_save_state(&mut capture).unwrap();
        state.punctuation_table = Some(91);
        capture.map.insert(91, ScriptMemoryAllocation::zeroed(8));
        let (memory, _) = probe_memory();
        let mut restore = MapRestore { map: capture.map };
        assert_eq!(
            bad_ui(SourceScriptStorage::restore_save_state(&state, memory, &mut restore).unwrap_err()),
            "script.storage: invalid punctuation table extent"
        );
    }
}
