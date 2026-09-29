//! Source and indent storage for legacy menu scripts.
//!
//! Donor provenance: `src/ui/common/legacy/script/source-memory.ts`
//! (source and indent storage from id Software's `botlib/l_precomp.c/h`).
//!
//! [`SourceRecord`] owns one `source_t` record: filename and include path
//! bytes, punctuation/script/token/hash/indent/skip pointer words, and an
//! embedded token. Conditional-directive nesting lives in chained indent
//! levels keyed by owner-local ids (`0` ends a chain). A heap record
//! occupies exactly [`SOURCE_RECORD_BYTES`] little-endian bytes (Linux i386
//! layout):
//!
//! | Range      | Field             | Type                |
//! |------------|-------------------|---------------------|
//! | 0..1024    | filename bytes    | NUL-terminated Latin-1 |
//! | 1024..2048 | include path bytes | NUL-terminated Latin-1 |
//! | 2048..2052 | punctuations flag | `u32` (`0`/`1`)   |
//! | 2052..2056 | script stack head | `u32` id          |
//! | 2056..2060 | tokens head       | `u32` id          |
//! | 2060..2064 | padding           | untouched         |
//! | 2064..2068 | define hash flag  | `u32` (`0`/`1`)   |
//! | 2068..2072 | indent stack head | `u32` id          |
//! | 2072..2076 | skip count        | `i32`             |
//! | 2076..3144 | embedded token    | [`SOURCE_TOKEN_BYTES`] bytes |
//!
//! [`SOURCE_TOKEN_BYTES`]: super::token_memory::SOURCE_TOKEN_BYTES
//!
//! # Adaptations
//!
//! * The donor stores the `ScriptMemory` owner on the record and the hash
//!   and indent backings. Rust cannot hold that aliasing borrow, so the
//!   owner arrives per call as `Option<&mut dyn ScriptMemory>` and the
//!   record keeps only an `owns_heap` flag captured at construction. Callers
//!   must pass the same owner (or lack of one) they constructed the record
//!   with; heap operations with `None` fail with the donor's
//!   `requires memory owner` messages.
//! * Allocation word helpers are module-private in `super::precomp_memory`,
//!   so word access goes through the public byte accessors with the same
//!   little-endian layout.
//! * Allocation sub-views are module-private in `super::precomp_memory`, so
//!   the heap record's embedded token uses a separate zeroed allocation
//!   instead of viewing record bytes `2076..3144`. No donor path reads the
//!   token area directly (paths cover `0..2048`, words cover `2048..2076`),
//!   so the split is unobservable outside checkpoints, where the token
//!   round-trips through its own save state with write-back instead of the
//!   donor's heap verify.
//! * `SaveReader` checkpoints become the typed [`SourceRecordSaveState`]
//!   and friends. Every donor `RangeError` (and the internal-misuse
//!   `Error`s) becomes [`ClientError::BadUi`] carrying the donor message.
//! * [`ScriptPunctuation`] is imported from `super::preprocessor` because
//!   the `super::lexer` port is still a stub; it moves to `super::lexer` on
//!   integration.

use std::collections::BTreeMap;

use super::precomp_memory::{
    local_token, PrecompToken, PrecompTokenSaveState, ScriptMemory, ScriptMemoryAllocation, ScriptMemoryCapture,
    ScriptMemoryRestore,
};
use super::preprocessor::ScriptPunctuation;
use crate::error::ClientError;

/// Size of one `source_t` record, in bytes (donor `SOURCE_RECORD_BYTES`).
pub const SOURCE_RECORD_BYTES: usize = 3144;

/// Number of define-hash buckets (donor `SOURCE_DEFINE_HASH_BUCKETS`).
/// The hash allocation holds one `u32` head id per bucket.
pub const SOURCE_DEFINE_HASH_BUCKETS: usize = 1024;

/// Size of one indent level record, in bytes.
const SOURCE_INDENT_BYTES: usize = 16;
/// Path characters copied by one path store (donor `SOURCE_PATH_BYTES`).
const SOURCE_PATH_BYTES: usize = 64;
/// Offset of the filename bytes.
const SOURCE_FILENAME: usize = 0;
/// Offset of the include path bytes.
const SOURCE_INCLUDE_PATH: usize = 1024;
/// Offset of the punctuations flag word.
const SOURCE_PUNCTUATIONS: usize = 2048;
/// Offset of the script stack head word.
const SOURCE_SCRIPT_STACK: usize = 2052;
/// Offset of the tokens head word.
const SOURCE_TOKENS: usize = 2056;
/// Offset of the define hash flag word.
const SOURCE_DEFINE_HASH: usize = 2064;
/// Offset of the indent stack head word.
const SOURCE_INDENT_STACK: usize = 2068;
/// Offset of the skip count word.
const SOURCE_SKIP: usize = 2072;
/// Path field length scanned for a terminator.
const PATH_SCAN_BYTES: usize = 1024;

/// Word validation failure (donor engine `DataView` range error equivalent).
fn field_error() -> ClientError {
    ClientError::BadUi("script allocation field is outside its extent".to_string())
}

/// Read a little-endian `u32` word through the public byte accessors.
fn read_u32(allocation: &ScriptMemoryAllocation, offset: usize) -> Result<u32, ClientError> {
    let mut bytes = [0u8; 4];
    for (index, slot) in bytes.iter_mut().enumerate() {
        let at = offset.checked_add(index).ok_or_else(field_error)?;
        *slot = allocation.byte(at).map_err(|_| field_error())?;
    }
    Ok(u32::from_le_bytes(bytes))
}

/// Read a little-endian `i32` word through the public byte accessors.
fn read_i32(allocation: &ScriptMemoryAllocation, offset: usize) -> Result<i32, ClientError> {
    let mut bytes = [0u8; 4];
    for (index, slot) in bytes.iter_mut().enumerate() {
        let at = offset.checked_add(index).ok_or_else(field_error)?;
        *slot = allocation.byte(at).map_err(|_| field_error())?;
    }
    Ok(i32::from_le_bytes(bytes))
}

/// Write a little-endian `u32` word through the public byte accessors.
fn write_u32(allocation: &mut ScriptMemoryAllocation, offset: usize, value: u32) -> Result<(), ClientError> {
    for (index, byte) in value.to_le_bytes().iter().enumerate() {
        let at = offset.checked_add(index).ok_or_else(field_error)?;
        allocation.set_byte(at, *byte).map_err(|_| field_error())?;
    }
    Ok(())
}

/// Write a little-endian `i32` word through the public byte accessors.
fn write_i32(allocation: &mut ScriptMemoryAllocation, offset: usize, value: i32) -> Result<(), ClientError> {
    for (index, byte) in value.to_le_bytes().iter().enumerate() {
        let at = offset.checked_add(index).ok_or_else(field_error)?;
        allocation.set_byte(at, *byte).map_err(|_| field_error())?;
    }
    Ok(())
}

/// Conditional directive tracked by one indent level (donor `SourceIndentType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceIndentType {
    /// `#if` level.
    If = 1,
    /// `#else` level.
    Else = 2,
    /// `#elif` level.
    Elif = 4,
    /// `#ifdef` level.
    Ifdef = 8,
    /// `#ifndef` level.
    Ifndef = 16,
}

impl SourceIndentType {
    /// Numeric directive value stored in indent words.
    #[must_use]
    pub fn value(self) -> i32 {
        self as i32
    }

    /// Decode a stored directive value (donor indent `type` getter).
    pub fn from_value(value: i32) -> Result<Self, ClientError> {
        match value {
            1 => Ok(Self::If),
            2 => Ok(Self::Else),
            4 => Ok(Self::Elif),
            8 => Ok(Self::Ifdef),
            16 => Ok(Self::Ifndef),
            _ => Err(ClientError::BadUi(
                "source indent has an unsupported directive type".to_string(),
            )),
        }
    }
}

/// Record storage lifetime (donor `"heap" | "stack"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceLifetime {
    /// Heap record allocation owned by the script memory.
    Heap,
    /// Stack-managed record without a heap allocation.
    Stack,
}

/// Indent level removed by [`SourceRecord::pop_indent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoppedIndent {
    /// Directive of the removed level.
    pub indent_type: SourceIndentType,
    /// Skip count the level contributed.
    pub skip: i32,
}

/// Storage for one indent level (donor `IndentBacking`).
#[derive(Debug)]
enum IndentBacking {
    /// Heap words: `type` (`i32` at 0), `skip` (`i32` at 4),
    /// `script` (`u32` at 8), `next` (`u32` at 12).
    Heap { allocation: ScriptMemoryAllocation },
    /// Field storage for records without a memory owner.
    Managed {
        indent_type: i32,
        skip: i32,
        script: u32,
        next: u32,
    },
}

/// One conditional-directive indent level (donor `SourceIndent`).
#[derive(Debug)]
struct SourceIndent {
    /// Owner-local identity.
    id: u32,
    backing: IndentBacking,
}

impl SourceIndent {
    /// Directive of this level (donor `type` getter).
    fn indent_type(&self) -> Result<SourceIndentType, ClientError> {
        let value = match &self.backing {
            IndentBacking::Heap { allocation } => read_i32(allocation, 0)?,
            IndentBacking::Managed { indent_type, .. } => *indent_type,
        };
        SourceIndentType::from_value(value)
    }

    /// Set the directive of this level (donor `type` setter).
    fn set_type(&mut self, value: SourceIndentType) -> Result<(), ClientError> {
        match &mut self.backing {
            IndentBacking::Heap { allocation } => write_i32(allocation, 0, value.value()),
            IndentBacking::Managed { indent_type, .. } => {
                *indent_type = value.value();
                Ok(())
            }
        }
    }

    /// Skip count of this level (donor `skip` getter).
    fn skip(&self) -> Result<i32, ClientError> {
        match &self.backing {
            IndentBacking::Heap { allocation } => read_i32(allocation, 4),
            IndentBacking::Managed { skip, .. } => Ok(*skip),
        }
    }

    /// Set the skip count of this level (donor `skip` setter).
    fn set_skip(&mut self, value: i32) -> Result<(), ClientError> {
        match &mut self.backing {
            IndentBacking::Heap { allocation } => write_i32(allocation, 4, value),
            IndentBacking::Managed { skip, .. } => {
                *skip = value;
                Ok(())
            }
        }
    }

    /// Script id this level belongs to (donor `script` getter).
    fn script(&self) -> Result<u32, ClientError> {
        match &self.backing {
            IndentBacking::Heap { allocation } => read_u32(allocation, 8),
            IndentBacking::Managed { script, .. } => Ok(*script),
        }
    }

    /// Set the owning script id (donor `script` setter).
    fn set_script(&mut self, value: u32) -> Result<(), ClientError> {
        match &mut self.backing {
            IndentBacking::Heap { allocation } => write_u32(allocation, 8, value),
            IndentBacking::Managed { script, .. } => {
                *script = value;
                Ok(())
            }
        }
    }

    /// Next indent id in the stack (donor `next` getter).
    fn next(&self) -> Result<u32, ClientError> {
        match &self.backing {
            IndentBacking::Heap { allocation } => read_u32(allocation, 12),
            IndentBacking::Managed { next, .. } => Ok(*next),
        }
    }

    /// Set the next indent id (donor `next` setter).
    fn set_next(&mut self, value: u32) -> Result<(), ClientError> {
        match &mut self.backing {
            IndentBacking::Heap { allocation } => write_u32(allocation, 12, value),
            IndentBacking::Managed { next, .. } => {
                *next = value;
                Ok(())
            }
        }
    }

    /// Release a heap indent allocation (donor `free`).
    fn free(&self, memory: Option<&mut dyn ScriptMemory>) -> Result<(), ClientError> {
        if let IndentBacking::Heap { allocation } = &self.backing {
            let memory = memory.ok_or_else(|| ClientError::BadUi("heap indent requires memory owner".to_string()))?;
            memory.free(allocation);
        }
        Ok(())
    }

    /// Capture a checkpoint (donor `captureSaveState`).
    fn capture_save_state(&self, capture: &mut dyn ScriptMemoryCapture) -> SourceIndentBackingSave {
        match &self.backing {
            IndentBacking::Heap { allocation } => SourceIndentBackingSave::Heap {
                allocation: capture.reference(allocation),
            },
            IndentBacking::Managed {
                indent_type,
                skip,
                script,
                next,
            } => SourceIndentBackingSave::Managed {
                indent_type: *indent_type,
                skip: *skip,
                script: *script,
                next: *next,
            },
        }
    }

    /// Restore a checkpoint (donor `restoreSaveState`).
    fn restore_save_state(
        id: u32,
        state: &SourceIndentBackingSave,
        owns_heap: bool,
        restore: &mut dyn ScriptMemoryRestore,
    ) -> Result<Self, ClientError> {
        match state {
            SourceIndentBackingSave::Managed {
                indent_type,
                skip,
                script,
                next,
            } => Ok(Self {
                id,
                backing: IndentBacking::Managed {
                    indent_type: *indent_type,
                    skip: *skip,
                    script: *script,
                    next: *next,
                },
            }),
            SourceIndentBackingSave::Heap { allocation } => {
                if !owns_heap {
                    return Err(ClientError::BadUi("heap indent requires memory owner".to_string()));
                }
                let allocation = restore.allocation(*allocation)?;
                if allocation.len() != SOURCE_INDENT_BYTES {
                    return Err(ClientError::BadUi("invalid indent extent".to_string()));
                }
                Ok(Self {
                    id,
                    backing: IndentBacking::Heap { allocation },
                })
            }
        }
    }
}

/// Checkpoint of one indent backing (donor `SourceIndent` save state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceIndentBackingSave {
    /// Heap indent allocation checkpoint reference.
    Heap {
        /// Checkpoint reference of the indent allocation.
        allocation: u32,
    },
    /// Managed indent field values.
    Managed {
        /// Stored directive value.
        indent_type: i32,
        /// Stored skip count.
        skip: i32,
        /// Stored owning script id.
        script: u32,
        /// Stored next indent id.
        next: u32,
    },
}

/// Checkpoint of one indent level with its owner-local identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIndentSaveState {
    /// Owner-local indent identity.
    pub id: u32,
    /// Backing checkpoint.
    pub state: SourceIndentBackingSave,
}

/// Checkpoint of the record backing (donor `SourceRecord` backing save state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceBackingSave {
    /// Heap record allocation checkpoint reference.
    Heap {
        /// Checkpoint reference of the record allocation.
        allocation: u32,
    },
    /// Managed record field values.
    Managed {
        /// Filename.
        filename: String,
        /// Include path.
        include_path: String,
        /// Script stack head id.
        script: u32,
        /// Tokens head id.
        tokens: u32,
        /// Indent stack head id.
        indent: u32,
        /// Skip count.
        skip: i32,
    },
}

/// Checkpoint of one define-hash bucket head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceHashHeadSave {
    /// Bucket index.
    pub bucket: u32,
    /// Head define id.
    pub id: u32,
}

/// Checkpoint of the define-hash table (donor `SourceRecord` hash save state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceHashSave {
    /// Heap hash allocation checkpoint reference.
    Heap {
        /// Checkpoint reference of the hash allocation.
        allocation: u32,
    },
    /// Managed bucket heads.
    Managed {
        /// Nonzero bucket heads.
        heads: Vec<SourceHashHeadSave>,
    },
}

/// Checkpoint of one source record (donor `SourceRecord.captureSaveState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRecordSaveState {
    /// Record backing checkpoint.
    pub backing: SourceBackingSave,
    /// Define-hash checkpoint.
    pub hash: SourceHashSave,
    /// Next indent identity.
    pub next_indent: u32,
    /// Live indent checkpoints.
    pub indents: Vec<SourceIndentSaveState>,
    /// Embedded token checkpoint.
    pub token: PrecompTokenSaveState,
    /// Borrowed punctuation table, if any.
    pub punctuations: Option<Vec<ScriptPunctuation>>,
}

/// Storage for one source record (donor `SourceBacking`).
#[derive(Debug)]
enum SourceBacking {
    /// Heap record allocation.
    Heap { allocation: ScriptMemoryAllocation },
    /// Field storage for stack records.
    Managed {
        filename: String,
        include_path: String,
        script: u32,
        tokens: u32,
        indent: u32,
        skip: i32,
    },
}

/// Storage for the define-hash table (donor `HashBacking`).
#[derive(Debug)]
enum HashBacking {
    /// Heap allocation with one `u32` head id per bucket.
    Heap { allocation: ScriptMemoryAllocation },
    /// Bucket heads for records without a memory owner.
    Managed { heads: BTreeMap<u32, u32> },
}

/// One preprocessor source record (donor `SourceRecord`).
///
/// Pointer words resolve owner-local ids. Hash heads, script/indent links
/// and skip are read from the source allocation at every reached consumer.
#[derive(Debug)]
pub struct SourceRecord {
    /// Embedded token of the record.
    pub token: PrecompToken,
    backing: SourceBacking,
    hash: HashBacking,
    indents: BTreeMap<u32, SourceIndent>,
    next_indent: u32,
    borrowed_punctuations: Option<Vec<ScriptPunctuation>>,
    owns_heap: bool,
}

impl SourceRecord {
    /// Create a record for `filename` on script `script` (donor constructor).
    ///
    /// `Some` memory allocates the define-hash table, and with
    /// [`SourceLifetime::Heap`] the record itself; `None` builds a fully
    /// managed record. Later calls must pass the same owner (or lack of one).
    pub fn new(
        filename: &str,
        script: u32,
        memory: Option<&mut dyn ScriptMemory>,
        lifetime: SourceLifetime,
    ) -> Result<Self, ClientError> {
        let owns_heap = memory.is_some();
        let mut record = Self {
            token: local_token(),
            backing: SourceBacking::Managed {
                filename: String::new(),
                include_path: String::new(),
                script: 0,
                tokens: 0,
                indent: 0,
                skip: 0,
            },
            hash: HashBacking::Managed { heads: BTreeMap::new() },
            indents: BTreeMap::new(),
            next_indent: 1,
            borrowed_punctuations: None,
            owns_heap,
        };
        if let Some(memory) = memory {
            if lifetime == SourceLifetime::Heap {
                let mut allocation = memory.allocate(SOURCE_RECORD_BYTES);
                allocation.fill(0)?;
                record.backing = SourceBacking::Heap { allocation };
                record.copy_path(SOURCE_FILENAME, filename)?;
                record.set_script(script)?;
            } else {
                record.backing = SourceBacking::Managed {
                    filename: filename.to_string(),
                    include_path: String::new(),
                    script,
                    tokens: 0,
                    indent: 0,
                    skip: 0,
                };
            }
            let allocation = memory.allocate(SOURCE_DEFINE_HASH_BUCKETS * 4);
            record.hash = HashBacking::Heap { allocation };
            if matches!(record.backing, SourceBacking::Heap { .. }) {
                record.write_word_u32(SOURCE_DEFINE_HASH, 1)?;
            }
        } else {
            record.backing = SourceBacking::Managed {
                filename: filename.to_string(),
                include_path: String::new(),
                script,
                tokens: 0,
                indent: 0,
                skip: 0,
            };
        }
        Ok(record)
    }

    /// Capture a checkpoint (donor `captureSaveState`).
    pub fn capture_save_state(
        &self,
        capture: &mut dyn ScriptMemoryCapture,
    ) -> Result<SourceRecordSaveState, ClientError> {
        let backing = match &self.backing {
            SourceBacking::Heap { allocation } => SourceBackingSave::Heap {
                allocation: capture.reference(allocation),
            },
            SourceBacking::Managed {
                filename,
                include_path,
                script,
                tokens,
                indent,
                skip,
            } => SourceBackingSave::Managed {
                filename: filename.clone(),
                include_path: include_path.clone(),
                script: *script,
                tokens: *tokens,
                indent: *indent,
                skip: *skip,
            },
        };
        let hash = match &self.hash {
            HashBacking::Heap { allocation } => SourceHashSave::Heap {
                allocation: capture.reference(allocation),
            },
            HashBacking::Managed { heads } => SourceHashSave::Managed {
                heads: heads
                    .iter()
                    .map(|(bucket, id)| SourceHashHeadSave {
                        bucket: *bucket,
                        id: *id,
                    })
                    .collect(),
            },
        };
        let mut indents = Vec::with_capacity(self.indents.len());
        for indent in self.indents.values() {
            indents.push(SourceIndentSaveState {
                id: indent.id,
                state: indent.capture_save_state(capture),
            });
        }
        Ok(SourceRecordSaveState {
            backing,
            hash,
            next_indent: self.next_indent,
            indents,
            token: self.token.capture_save_state()?,
            punctuations: self.borrowed_punctuations.clone(),
        })
    }

    /// Restore a checkpoint (donor `restoreSaveState`).
    ///
    /// `memory` only records heap ownership for later calls; allocations
    /// resolve through `restore`.
    pub fn restore_save_state(
        state: &SourceRecordSaveState,
        memory: Option<&mut dyn ScriptMemory>,
        restore: &mut dyn ScriptMemoryRestore,
    ) -> Result<Self, ClientError> {
        let owns_heap = memory.is_some();
        if state.next_indent < 1 {
            return Err(ClientError::BadUi(
                "script.source.nextIndent: expected an integer in range".to_string(),
            ));
        }
        let backing = match &state.backing {
            SourceBackingSave::Heap { allocation } => {
                let allocation = restore.allocation(*allocation)?;
                if allocation.len() != SOURCE_RECORD_BYTES {
                    return Err(ClientError::BadUi("invalid source extent".to_string()));
                }
                SourceBacking::Heap { allocation }
            }
            SourceBackingSave::Managed {
                filename,
                include_path,
                script,
                tokens,
                indent,
                skip,
            } => SourceBacking::Managed {
                filename: filename.clone(),
                include_path: include_path.clone(),
                script: *script,
                tokens: *tokens,
                indent: *indent,
                skip: *skip,
            },
        };
        let hash = match &state.hash {
            SourceHashSave::Heap { allocation } => {
                if !owns_heap {
                    return Err(ClientError::BadUi("heap hash requires memory owner".to_string()));
                }
                let allocation = restore.allocation(*allocation)?;
                if allocation.len() != SOURCE_DEFINE_HASH_BUCKETS * 4 {
                    return Err(ClientError::BadUi("invalid define hash extent".to_string()));
                }
                HashBacking::Heap { allocation }
            }
            SourceHashSave::Managed { heads } => {
                let mut map = BTreeMap::new();
                for head in heads {
                    if head.bucket as usize >= SOURCE_DEFINE_HASH_BUCKETS || map.contains_key(&head.bucket) {
                        return Err(ClientError::BadUi("invalid hash bucket".to_string()));
                    }
                    map.insert(head.bucket, head.id);
                }
                HashBacking::Managed { heads: map }
            }
        };
        let mut indents = BTreeMap::new();
        for entry in &state.indents {
            if entry.id == 0 || entry.id >= state.next_indent || indents.contains_key(&entry.id) {
                return Err(ClientError::BadUi("invalid indent identity".to_string()));
            }
            indents.insert(
                entry.id,
                SourceIndent::restore_save_state(entry.id, &entry.state, owns_heap, restore)?,
            );
        }
        let mut token = local_token();
        token.restore_save_state(&state.token, false)?;
        Ok(Self {
            token,
            backing,
            hash,
            indents,
            next_indent: state.next_indent,
            borrowed_punctuations: state.punctuations.clone(),
            owns_heap,
        })
    }

    /// Record filename (donor `filename` getter).
    pub fn filename(&self) -> Result<String, ClientError> {
        match &self.backing {
            SourceBacking::Heap { .. } => self.read_path(SOURCE_FILENAME),
            SourceBacking::Managed { filename, .. } => Ok(filename.clone()),
        }
    }

    /// Borrow a punctuation table, or clear the borrow (donor `setPunctuations`).
    pub fn set_punctuations(&mut self, punctuations: Option<&[ScriptPunctuation]>) -> Result<(), ClientError> {
        if matches!(self.backing, SourceBacking::Heap { .. }) {
            self.write_word_u32(SOURCE_PUNCTUATIONS, u32::from(punctuations.is_some()))?;
        }
        self.borrowed_punctuations = punctuations.map(<[ScriptPunctuation]>::to_vec);
        Ok(())
    }

    /// Borrowed punctuation table, if any (donor `punctuations` getter).
    pub fn punctuations(&self) -> Result<Option<&[ScriptPunctuation]>, ClientError> {
        if matches!(self.backing, SourceBacking::Managed { .. }) {
            return Ok(self.borrowed_punctuations.as_deref());
        }
        let pointer = self.read_word_u32(SOURCE_PUNCTUATIONS)?;
        if pointer == 0 {
            return Ok(None);
        }
        match &self.borrowed_punctuations {
            Some(table) if pointer == 1 => Ok(Some(table)),
            _ => Err(ClientError::BadUi(
                "source punctuation pointer does not identify its borrowed table".to_string(),
            )),
        }
    }

    /// Tokens head id (donor `tokens` getter).
    pub fn tokens(&self) -> Result<u32, ClientError> {
        match &self.backing {
            SourceBacking::Heap { .. } => self.read_word_u32(SOURCE_TOKENS),
            SourceBacking::Managed { tokens, .. } => Ok(*tokens),
        }
    }

    /// Set the tokens head id (donor `tokens` setter).
    pub fn set_tokens(&mut self, value: u32) -> Result<(), ClientError> {
        if matches!(self.backing, SourceBacking::Heap { .. }) {
            return self.write_word_u32(SOURCE_TOKENS, value);
        }
        if let SourceBacking::Managed { tokens, .. } = &mut self.backing {
            *tokens = value;
        }
        Ok(())
    }

    /// Include path (donor `includePath` getter).
    pub fn include_path(&self) -> Result<String, ClientError> {
        match &self.backing {
            SourceBacking::Heap { .. } => self.read_path(SOURCE_INCLUDE_PATH),
            SourceBacking::Managed { include_path, .. } => Ok(include_path.clone()),
        }
    }

    /// Set the include path, appending a separator (donor `setIncludePath`).
    pub fn set_include_path(&mut self, path: &str) -> Result<(), ClientError> {
        if let SourceBacking::Managed { include_path, .. } = &mut self.backing {
            let cut = match path.find('\0') {
                Some(end) => &path[..end],
                None => path,
            };
            let units: Vec<u16> = cut.encode_utf16().collect();
            let end = units.len().min(SOURCE_PATH_BYTES);
            let mut value = String::from_utf16_lossy(&units[..end]);
            if !value.ends_with('/') && !value.ends_with('\\') {
                value.push('/');
            }
            *include_path = value;
            return Ok(());
        }
        self.copy_path(SOURCE_INCLUDE_PATH, path)?;
        let value = self.include_path()?;
        if !value.ends_with('/') && !value.ends_with('\\') {
            let length = value.chars().count();
            if length + 1 >= PATH_SCAN_BYTES {
                return Err(ClientError::BadUi(
                    "source include path exceeds its allocation".to_string(),
                ));
            }
            let allocation = self.heap_allocation_mut()?;
            allocation
                .set_byte(SOURCE_INCLUDE_PATH + length, b'/')
                .map_err(|_| field_error())?;
            allocation
                .set_byte(SOURCE_INCLUDE_PATH + length + 1, 0)
                .map_err(|_| field_error())?;
        }
        Ok(())
    }

    /// Script stack head id (donor `script` getter).
    pub fn script(&self) -> Result<u32, ClientError> {
        match &self.backing {
            SourceBacking::Heap { .. } => self.read_word_u32(SOURCE_SCRIPT_STACK),
            SourceBacking::Managed { script, .. } => Ok(*script),
        }
    }

    /// Set the script stack head id (donor `script` setter).
    pub fn set_script(&mut self, value: u32) -> Result<(), ClientError> {
        if matches!(self.backing, SourceBacking::Heap { .. }) {
            return self.write_word_u32(SOURCE_SCRIPT_STACK, value);
        }
        if let SourceBacking::Managed { script, .. } = &mut self.backing {
            *script = value;
        }
        Ok(())
    }

    /// Skip count (donor `skip` getter).
    pub fn skip(&self) -> Result<i32, ClientError> {
        match &self.backing {
            SourceBacking::Heap { .. } => self.read_word_i32(SOURCE_SKIP),
            SourceBacking::Managed { skip, .. } => Ok(*skip),
        }
    }

    /// Set the skip count (donor `skip` setter).
    pub fn set_skip(&mut self, value: i32) -> Result<(), ClientError> {
        if matches!(self.backing, SourceBacking::Heap { .. }) {
            return self.write_word_i32(SOURCE_SKIP, value);
        }
        if let SourceBacking::Managed { skip, .. } = &mut self.backing {
            *skip = value;
        }
        Ok(())
    }

    /// Head define id of one hash bucket (donor `hashHead`).
    pub fn hash_head(&self, bucket: u32) -> Result<u32, ClientError> {
        match &self.hash {
            HashBacking::Managed { heads } => Ok(heads.get(&bucket).copied().unwrap_or(0)),
            HashBacking::Heap { .. } => {
                self.check_hash_pointer()?;
                let offset = (bucket as usize).checked_mul(4).ok_or_else(field_error)?;
                let allocation = self.hash_allocation()?;
                read_u32(allocation, offset)
            }
        }
    }

    /// Set the head define id of one hash bucket (donor `setHashHead`).
    /// A zero id clears a managed bucket.
    pub fn set_hash_head(&mut self, bucket: u32, id: u32) -> Result<(), ClientError> {
        if let HashBacking::Managed { heads } = &mut self.hash {
            if id == 0 {
                heads.remove(&bucket);
            } else {
                heads.insert(bucket, id);
            }
            return Ok(());
        }
        self.check_hash_pointer()?;
        let offset = (bucket as usize).checked_mul(4).ok_or_else(field_error)?;
        let allocation = self.hash_allocation_mut()?;
        write_u32(allocation, offset, id)
    }

    /// Whether the top indent belongs to the current script
    /// (donor `hasCurrentIndent`).
    pub fn has_current_indent(&self) -> Result<bool, ClientError> {
        match self.top_indent()? {
            Some(indent) => Ok(indent.script()? == self.script()?),
            None => Ok(false),
        }
    }

    /// Push one indent level for the current script (donor `pushIndent`).
    pub fn push_indent(
        &mut self,
        memory: Option<&mut dyn ScriptMemory>,
        indent_type: SourceIndentType,
        skip: bool,
    ) -> Result<(), ClientError> {
        let backing = if self.owns_heap {
            let memory = memory.ok_or_else(|| ClientError::BadUi("heap indent requires memory owner".to_string()))?;
            IndentBacking::Heap {
                allocation: memory.allocate(SOURCE_INDENT_BYTES),
            }
        } else {
            IndentBacking::Managed {
                indent_type: 0,
                skip: 0,
                script: 0,
                next: 0,
            }
        };
        let id = self.next_indent;
        self.next_indent = self
            .next_indent
            .checked_add(1)
            .ok_or_else(|| ClientError::BadUi("source indent count exceeds its range".to_string()))?;
        let mut indent = SourceIndent { id, backing };
        indent.set_type(indent_type)?;
        indent.set_script(self.script()?)?;
        indent.set_skip(i32::from(skip))?;
        self.set_skip(self.skip()? + i32::from(skip))?;
        indent.set_next(self.indent_head()?)?;
        self.indents.insert(id, indent);
        self.set_indent_head(id)?;
        Ok(())
    }

    /// Pop the top indent when it belongs to the current script
    /// (donor `popIndent`). Returns `None` for an empty or foreign stack.
    pub fn pop_indent(&mut self, memory: Option<&mut dyn ScriptMemory>) -> Result<Option<PoppedIndent>, ClientError> {
        let id = self.indent_head()?;
        if id == 0 {
            return Ok(None);
        }
        let indent = self.indents.get(&id).ok_or_else(|| {
            ClientError::BadUi("source indent pointer does not identify a live allocation".to_string())
        })?;
        if indent.script()? != self.script()? {
            return Ok(None);
        }
        let popped = PoppedIndent {
            indent_type: indent.indent_type()?,
            skip: indent.skip()?,
        };
        let next = indent.next()?;
        if matches!(indent.backing, IndentBacking::Heap { .. }) && memory.is_none() {
            return Err(ClientError::BadUi("heap indent requires memory owner".to_string()));
        }
        self.set_indent_head(next)?;
        self.set_skip(self.skip()? - popped.skip)?;
        let indent = self.indents.remove(&id).ok_or_else(|| {
            ClientError::BadUi("source indent pointer does not identify a live allocation".to_string())
        })?;
        indent.free(memory)?;
        Ok(Some(popped))
    }

    /// Release every indent level without adjusting skip (donor `freeIndents`).
    pub fn free_indents(&mut self, memory: Option<&mut dyn ScriptMemory>) -> Result<(), ClientError> {
        let mut cursor = self.indent_head()?;
        while cursor != 0 {
            let indent = self.indents.get(&cursor).ok_or_else(|| {
                ClientError::BadUi("source indent pointer does not identify a live allocation".to_string())
            })?;
            if matches!(indent.backing, IndentBacking::Heap { .. }) && memory.is_none() {
                return Err(ClientError::BadUi("heap indent requires memory owner".to_string()));
            }
            cursor = indent.next()?;
        }
        let mut memory = memory;
        loop {
            let id = self.indent_head()?;
            if id == 0 {
                return Ok(());
            }
            let indent = self.indents.remove(&id).ok_or_else(|| {
                ClientError::BadUi("source indent pointer does not identify a live allocation".to_string())
            })?;
            let next = indent.next()?;
            self.set_indent_head(next)?;
            match memory.as_mut() {
                Some(slot) => indent.free(Some(&mut **slot))?,
                None => indent.free(None)?,
            }
        }
    }

    /// Release the define-hash allocation (donor `freeHash`).
    /// Managed hashes and already-cleared pointers are a no-op.
    pub fn free_hash(&mut self, memory: Option<&mut dyn ScriptMemory>) -> Result<(), ClientError> {
        if !matches!(self.hash, HashBacking::Heap { .. }) {
            return Ok(());
        }
        if matches!(self.backing, SourceBacking::Heap { .. }) {
            match self.read_word_u32(SOURCE_DEFINE_HASH)? {
                0 => return Ok(()),
                1 => {}
                _ => {
                    return Err(ClientError::BadUi(
                        "source define hash pointer does not identify its allocation".to_string(),
                    ));
                }
            }
        }
        let memory = memory.ok_or_else(|| ClientError::BadUi("heap hash requires memory owner".to_string()))?;
        if let HashBacking::Heap { allocation } = &self.hash {
            memory.free(allocation);
        }
        Ok(())
    }

    /// Release the record allocation (donor `freeRecord`).
    pub fn free_record(&mut self, memory: Option<&mut dyn ScriptMemory>) -> Result<(), ClientError> {
        if !matches!(self.backing, SourceBacking::Heap { .. }) || !self.owns_heap {
            return Ok(());
        }
        let memory =
            memory.ok_or_else(|| ClientError::BadUi("heap source record requires memory owner".to_string()))?;
        if let SourceBacking::Heap { allocation } = &self.backing {
            memory.free(allocation);
        }
        Ok(())
    }

    /// Indent stack head id (donor private `indent` getter).
    fn indent_head(&self) -> Result<u32, ClientError> {
        match &self.backing {
            SourceBacking::Heap { .. } => self.read_word_u32(SOURCE_INDENT_STACK),
            SourceBacking::Managed { indent, .. } => Ok(*indent),
        }
    }

    /// Set the indent stack head id (donor private `indent` setter).
    fn set_indent_head(&mut self, value: u32) -> Result<(), ClientError> {
        if matches!(self.backing, SourceBacking::Heap { .. }) {
            return self.write_word_u32(SOURCE_INDENT_STACK, value);
        }
        if let SourceBacking::Managed { indent, .. } = &mut self.backing {
            *indent = value;
        }
        Ok(())
    }

    /// Top indent level, if any (donor private `topIndent` getter).
    fn top_indent(&self) -> Result<Option<&SourceIndent>, ClientError> {
        let id = self.indent_head()?;
        if id == 0 {
            return Ok(None);
        }
        self.indents
            .get(&id)
            .map(Some)
            .ok_or_else(|| ClientError::BadUi("source indent pointer does not identify a live allocation".to_string()))
    }

    /// Heap record allocation (donor private `view` owner).
    fn heap_allocation(&self) -> Result<&ScriptMemoryAllocation, ClientError> {
        match &self.backing {
            SourceBacking::Heap { allocation } => Ok(allocation),
            SourceBacking::Managed { .. } => Err(ClientError::BadUi("stack source has no heap record".to_string())),
        }
    }

    /// Mutable heap record allocation.
    fn heap_allocation_mut(&mut self) -> Result<&mut ScriptMemoryAllocation, ClientError> {
        match &mut self.backing {
            SourceBacking::Heap { allocation } => Ok(allocation),
            SourceBacking::Managed { .. } => Err(ClientError::BadUi("stack source has no heap record".to_string())),
        }
    }

    /// Read a `u32` word from the heap record.
    fn read_word_u32(&self, offset: usize) -> Result<u32, ClientError> {
        read_u32(self.heap_allocation()?, offset)
    }

    /// Write a `u32` word to the heap record.
    fn write_word_u32(&mut self, offset: usize, value: u32) -> Result<(), ClientError> {
        let allocation = self.heap_allocation_mut()?;
        write_u32(allocation, offset, value)
    }

    /// Read an `i32` word from the heap record.
    fn read_word_i32(&self, offset: usize) -> Result<i32, ClientError> {
        read_i32(self.heap_allocation()?, offset)
    }

    /// Write an `i32` word to the heap record.
    fn write_word_i32(&mut self, offset: usize, value: i32) -> Result<(), ClientError> {
        let allocation = self.heap_allocation_mut()?;
        write_i32(allocation, offset, value)
    }

    /// Heap hash allocation (donor private `hashView` owner).
    fn hash_allocation(&self) -> Result<&ScriptMemoryAllocation, ClientError> {
        match &self.hash {
            HashBacking::Heap { allocation } => Ok(allocation),
            HashBacking::Managed { .. } => Err(ClientError::BadUi(
                "diagnostic source has no define hash allocation".to_string(),
            )),
        }
    }

    /// Mutable heap hash allocation.
    fn hash_allocation_mut(&mut self) -> Result<&mut ScriptMemoryAllocation, ClientError> {
        match &mut self.hash {
            HashBacking::Heap { allocation } => Ok(allocation),
            HashBacking::Managed { .. } => Err(ClientError::BadUi(
                "diagnostic source has no define hash allocation".to_string(),
            )),
        }
    }

    /// Validate the heap define-hash pointer word (donor `hashView` guard).
    fn check_hash_pointer(&self) -> Result<(), ClientError> {
        if matches!(self.backing, SourceBacking::Heap { .. }) && self.read_word_u32(SOURCE_DEFINE_HASH)? != 1 {
            return Err(ClientError::BadUi(
                "source define hash pointer does not identify its allocation".to_string(),
            ));
        }
        Ok(())
    }

    /// Store a path field (donor private `copyPath`).
    fn copy_path(&mut self, offset: usize, path: &str) -> Result<(), ClientError> {
        let units: Vec<u16> = path.encode_utf16().collect();
        let allocation = match &mut self.backing {
            SourceBacking::Heap { allocation } => allocation,
            SourceBacking::Managed { .. } => {
                return Err(ClientError::BadUi("stack source has no heap path".to_string()));
            }
        };
        let mut ended = false;
        for index in 0..SOURCE_PATH_BYTES {
            let character = if ended || index >= units.len() {
                0
            } else {
                u32::from(units[index])
            };
            if character > 255 {
                return Err(ClientError::BadUi(
                    "source path requires source byte characters".to_string(),
                ));
            }
            allocation
                .set_byte(offset + index, character as u8)
                .map_err(|_| field_error())?;
            if character == 0 {
                ended = true;
            }
        }
        Ok(())
    }

    /// Read a path field (donor private `readPath`).
    fn read_path(&self, offset: usize) -> Result<String, ClientError> {
        let allocation = match &self.backing {
            SourceBacking::Heap { allocation } => allocation,
            SourceBacking::Managed { .. } => {
                return Err(ClientError::BadUi("stack source has no heap path".to_string()));
            }
        };
        let mut text = String::new();
        for index in 0..PATH_SCAN_BYTES {
            let character = allocation.byte(offset + index).map_err(|_| field_error())?;
            if character == 0 {
                return Ok(text);
            }
            text.push(character as char);
        }
        Err(ClientError::BadUi("source path lacks its terminator".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeMemory {
        allocated: Vec<usize>,
        freed: usize,
    }

    impl FakeMemory {
        fn new() -> Self {
            Self {
                allocated: Vec::new(),
                freed: 0,
            }
        }
    }

    impl ScriptMemory for FakeMemory {
        fn allocate(&mut self, size: usize) -> ScriptMemoryAllocation {
            self.allocated.push(size);
            ScriptMemoryAllocation::zeroed(size)
        }

        fn free(&mut self, _allocation: &ScriptMemoryAllocation) {
            self.freed += 1;
        }
    }

    struct FakeCheckpoint {
        images: HashMap<u32, Vec<u8>>,
        next: u32,
    }

    impl FakeCheckpoint {
        fn new() -> Self {
            Self {
                images: HashMap::new(),
                next: 0,
            }
        }
    }

    impl ScriptMemoryCapture for FakeCheckpoint {
        fn reference(&mut self, allocation: &ScriptMemoryAllocation) -> u32 {
            let id = self.next;
            self.next += 1;
            self.images.insert(id, allocation.snapshot_bytes().expect("snapshot"));
            id
        }
    }

    impl ScriptMemoryRestore for FakeCheckpoint {
        fn allocation(&mut self, id: u32) -> Result<ScriptMemoryAllocation, ClientError> {
            self.images
                .get(&id)
                .cloned()
                .map(ScriptMemoryAllocation::from_bytes)
                .ok_or_else(|| ClientError::BadUi(format!("unknown allocation {id}")))
        }
    }

    fn managed_save() -> SourceRecordSaveState {
        let record = SourceRecord::new("save", 3, None, SourceLifetime::Stack).expect("managed");
        let mut checkpoint = FakeCheckpoint::new();
        record.capture_save_state(&mut checkpoint).expect("capture managed")
    }

    fn bad_ui(error: ClientError) -> String {
        match error {
            ClientError::BadUi(message) => message,
            other => panic!("expected BadUi, got {other:?}"),
        }
    }

    #[test]
    fn record_constants_match_donor_layout() {
        assert_eq!(SOURCE_RECORD_BYTES, 3144);
        assert_eq!(SOURCE_DEFINE_HASH_BUCKETS, 1024);
        assert_eq!(SOURCE_DEFINE_HASH_BUCKETS * 4, 4096);
    }

    #[test]
    fn managed_record_starts_with_donor_defaults() {
        let record = SourceRecord::new("maps/test", 7, None, SourceLifetime::Stack).expect("new");
        assert_eq!(record.filename().expect("filename"), "maps/test");
        assert_eq!(record.script().expect("script"), 7);
        assert_eq!(record.tokens().expect("tokens"), 0);
        assert_eq!(record.skip().expect("skip"), 0);
        assert_eq!(record.include_path().expect("include"), "");
        assert!(record.punctuations().expect("punct").is_none());
        assert!(!record.has_current_indent().expect("indent"));
        assert_eq!(record.hash_head(9).expect("hash"), 0);
    }

    #[test]
    fn heap_record_allocates_record_and_hash() {
        let mut memory = FakeMemory::new();
        let record = SourceRecord::new("heap", 5, Some(&mut memory), SourceLifetime::Heap).expect("new");
        assert_eq!(record.filename().expect("filename"), "heap");
        assert_eq!(record.script().expect("script"), 5);
        assert_eq!(record.tokens().expect("tokens"), 0);
        assert_eq!(record.skip().expect("skip"), 0);
        assert_eq!(memory.allocated, vec![SOURCE_RECORD_BYTES, 4096]);
    }

    #[test]
    fn stack_lifetime_with_memory_keeps_managed_backing_and_heap_hash() {
        let mut memory = FakeMemory::new();
        let mut record = SourceRecord::new("stack", 2, Some(&mut memory), SourceLifetime::Stack).expect("new");
        assert_eq!(record.filename().expect("filename"), "stack");
        assert_eq!(memory.allocated, vec![4096]);
        record.set_hash_head(4, 11).expect("set head");
        assert_eq!(record.hash_head(4).expect("head"), 11);
    }

    #[test]
    fn heap_filename_keeps_latin1_but_rejects_wider_chars() {
        let mut memory = FakeMemory::new();
        let record =
            SourceRecord::new("caf\u{e9}", 1, Some(&mut memory), SourceLifetime::Heap).expect("latin1 filename");
        assert_eq!(record.filename().expect("filename"), "caf\u{e9}");
        let mut memory = FakeMemory::new();
        let error = SourceRecord::new("caf\u{20ac}", 1, Some(&mut memory), SourceLifetime::Heap)
            .expect_err("wide filename fails");
        assert_eq!(bad_ui(error), "source path requires source byte characters");
    }

    #[test]
    fn managed_include_path_rules_match_donor() {
        let mut record = SourceRecord::new("x", 0, None, SourceLifetime::Stack).expect("new");
        record.set_include_path("maps").expect("set");
        assert_eq!(record.include_path().expect("include"), "maps/");
        record.set_include_path("maps/").expect("set");
        assert_eq!(record.include_path().expect("include"), "maps/");
        record.set_include_path("a\\b\\").expect("set");
        assert_eq!(record.include_path().expect("include"), "a\\b\\");
        record.set_include_path("cut\0rest").expect("set");
        assert_eq!(record.include_path().expect("include"), "cut/");
        let long = "p".repeat(100);
        record.set_include_path(&long).expect("set");
        assert_eq!(record.include_path().expect("include"), format!("{}/", "p".repeat(64)));
    }

    #[test]
    fn heap_include_path_appends_separator() {
        let mut memory = FakeMemory::new();
        let mut record = SourceRecord::new("x", 0, Some(&mut memory), SourceLifetime::Heap).expect("new");
        record.set_include_path("maps").expect("set");
        assert_eq!(record.include_path().expect("include"), "maps/");
        record.set_include_path("maps/").expect("set");
        assert_eq!(record.include_path().expect("include"), "maps/");
    }

    #[test]
    fn word_fields_round_trip_on_both_backings() {
        let mut managed = SourceRecord::new("x", 0, None, SourceLifetime::Stack).expect("new");
        managed.set_script(9).expect("script");
        managed.set_tokens(10).expect("tokens");
        managed.set_skip(-3).expect("skip");
        assert_eq!(managed.script().expect("script"), 9);
        assert_eq!(managed.tokens().expect("tokens"), 10);
        assert_eq!(managed.skip().expect("skip"), -3);

        let mut memory = FakeMemory::new();
        let mut heap = SourceRecord::new("x", 0, Some(&mut memory), SourceLifetime::Heap).expect("new");
        heap.set_script(9).expect("script");
        heap.set_tokens(10).expect("tokens");
        heap.set_skip(-3).expect("skip");
        assert_eq!(heap.script().expect("script"), 9);
        assert_eq!(heap.tokens().expect("tokens"), 10);
        assert_eq!(heap.skip().expect("skip"), -3);
    }

    #[test]
    fn punctuation_borrow_round_trip_on_both_backings() {
        let table = vec![
            ScriptPunctuation {
                text: "&&".to_string(),
                punctuation: 42,
            },
            ScriptPunctuation {
                text: "##".to_string(),
                punctuation: 43,
            },
        ];
        let mut managed = SourceRecord::new("x", 0, None, SourceLifetime::Stack).expect("new");
        managed.set_punctuations(Some(&table)).expect("set");
        assert_eq!(managed.punctuations().expect("get"), Some(table.as_slice()));
        managed.set_punctuations(None).expect("clear");
        assert!(managed.punctuations().expect("get").is_none());

        let mut memory = FakeMemory::new();
        let mut heap = SourceRecord::new("x", 0, Some(&mut memory), SourceLifetime::Heap).expect("new");
        assert!(heap.punctuations().expect("fresh").is_none());
        heap.set_punctuations(Some(&table)).expect("set");
        assert_eq!(heap.punctuations().expect("get"), Some(table.as_slice()));
        heap.set_punctuations(None).expect("clear");
        assert!(heap.punctuations().expect("get").is_none());
    }

    #[test]
    fn managed_hash_heads_set_and_clear() {
        let mut record = SourceRecord::new("x", 0, None, SourceLifetime::Stack).expect("new");
        record.set_hash_head(7, 21).expect("set");
        assert_eq!(record.hash_head(7).expect("head"), 21);
        record.set_hash_head(7, 0).expect("clear");
        assert_eq!(record.hash_head(7).expect("head"), 0);
    }

    #[test]
    fn heap_hash_heads_round_trip() {
        let mut memory = FakeMemory::new();
        let mut record = SourceRecord::new("x", 0, Some(&mut memory), SourceLifetime::Heap).expect("new");
        assert_eq!(record.hash_head(1023).expect("fresh"), 0);
        record.set_hash_head(1023, 77).expect("set");
        assert_eq!(record.hash_head(1023).expect("head"), 77);
    }

    #[test]
    fn managed_indents_push_pop_and_nest() {
        let mut record = SourceRecord::new("x", 4, None, SourceLifetime::Stack).expect("new");
        assert!(record.pop_indent(None).expect("pop empty").is_none());
        record.push_indent(None, SourceIndentType::If, true).expect("push if");
        assert!(record.has_current_indent().expect("current"));
        assert_eq!(record.skip().expect("skip"), 1);
        record
            .push_indent(None, SourceIndentType::Else, false)
            .expect("push else");
        assert_eq!(record.skip().expect("skip"), 1);
        let popped = record.pop_indent(None).expect("pop").expect("some");
        assert_eq!(popped.indent_type, SourceIndentType::Else);
        assert_eq!(popped.skip, 0);
        let popped = record.pop_indent(None).expect("pop").expect("some");
        assert_eq!(popped.indent_type, SourceIndentType::If);
        assert_eq!(popped.skip, 1);
        assert_eq!(record.skip().expect("skip"), 0);
        assert!(!record.has_current_indent().expect("current"));
    }

    #[test]
    fn indents_ignore_foreign_scripts() {
        let mut record = SourceRecord::new("x", 4, None, SourceLifetime::Stack).expect("new");
        record.push_indent(None, SourceIndentType::Ifdef, true).expect("push");
        record.set_script(9).expect("switch script");
        assert!(!record.has_current_indent().expect("current"));
        assert!(record.pop_indent(None).expect("pop").is_none());
        assert_eq!(record.skip().expect("skip"), 1);
        record.set_script(4).expect("switch back");
        assert!(record.has_current_indent().expect("current"));
    }

    #[test]
    fn free_indents_drains_without_touching_skip() {
        let mut record = SourceRecord::new("x", 1, None, SourceLifetime::Stack).expect("new");
        record.push_indent(None, SourceIndentType::If, true).expect("push");
        record.push_indent(None, SourceIndentType::Elif, true).expect("push");
        record.free_indents(None).expect("free");
        assert!(!record.has_current_indent().expect("current"));
        assert_eq!(record.skip().expect("skip"), 2);
        assert!(record.pop_indent(None).expect("pop").is_none());
    }

    #[test]
    fn heap_indents_allocate_and_free() {
        let mut memory = FakeMemory::new();
        let mut record = SourceRecord::new("x", 1, Some(&mut memory), SourceLifetime::Heap).expect("new");
        record
            .push_indent(Some(&mut memory), SourceIndentType::Ifndef, false)
            .expect("push");
        assert_eq!(memory.allocated, vec![SOURCE_RECORD_BYTES, 4096, 16]);
        let popped = record.pop_indent(Some(&mut memory)).expect("pop").expect("some");
        assert_eq!(popped.indent_type, SourceIndentType::Ifndef);
        assert_eq!(memory.freed, 1);
    }

    #[test]
    fn free_hash_and_record_release_heap_allocations() {
        let mut memory = FakeMemory::new();
        let mut record = SourceRecord::new("x", 1, Some(&mut memory), SourceLifetime::Heap).expect("new");
        record.free_hash(Some(&mut memory)).expect("free hash");
        record.free_record(Some(&mut memory)).expect("free record");
        assert_eq!(memory.freed, 2);

        let mut managed = SourceRecord::new("x", 1, None, SourceLifetime::Stack).expect("new");
        managed.free_hash(None).expect("managed hash");
        managed.free_record(None).expect("managed record");
    }

    #[test]
    fn managed_checkpoint_round_trip_preserves_state() {
        let mut record = SourceRecord::new("round", 6, None, SourceLifetime::Stack).expect("new");
        record.set_tokens(12).expect("tokens");
        record.set_skip(2).expect("skip");
        record.set_include_path("inc").expect("include");
        record.set_hash_head(3, 30).expect("hash");
        record.push_indent(None, SourceIndentType::If, true).expect("push");
        record.token.token.write_string("tok").expect("token string");
        let table = vec![ScriptPunctuation {
            text: "...".to_string(),
            punctuation: 7,
        }];
        record.set_punctuations(Some(&table)).expect("punct");
        let mut checkpoint = FakeCheckpoint::new();
        let save = record.capture_save_state(&mut checkpoint).expect("capture");

        let mut restore = FakeCheckpoint {
            images: checkpoint.images,
            next: checkpoint.next,
        };
        let restored = SourceRecord::restore_save_state(&save, None, &mut restore).expect("restore");
        assert_eq!(restored.filename().expect("filename"), "round");
        assert_eq!(restored.script().expect("script"), 6);
        assert_eq!(restored.tokens().expect("tokens"), 12);
        assert_eq!(restored.skip().expect("skip"), 3);
        assert_eq!(restored.include_path().expect("include"), "inc/");
        assert_eq!(restored.hash_head(3).expect("hash"), 30);
        assert!(restored.has_current_indent().expect("current"));
        assert_eq!(restored.token.token.string().expect("token"), "tok");
        assert_eq!(restored.punctuations().expect("punct"), Some(table.as_slice()));
        let mut checkpoint = FakeCheckpoint::new();
        let again = restored.capture_save_state(&mut checkpoint).expect("recapture");
        assert_eq!(save, again);
    }

    #[test]
    fn heap_checkpoint_round_trip_preserves_state() {
        let mut memory = FakeMemory::new();
        let mut record = SourceRecord::new("heap-round", 8, Some(&mut memory), SourceLifetime::Heap).expect("new");
        record.set_tokens(15).expect("tokens");
        record.set_include_path("inc").expect("include");
        record.set_hash_head(5, 50).expect("hash");
        record
            .push_indent(Some(&mut memory), SourceIndentType::Else, false)
            .expect("push");
        let mut checkpoint = FakeCheckpoint::new();
        let save = record.capture_save_state(&mut checkpoint).expect("capture");
        assert!(matches!(save.backing, SourceBackingSave::Heap { .. }));
        assert!(matches!(save.hash, SourceHashSave::Heap { .. }));

        let mut memory = FakeMemory::new();
        let mut restore = FakeCheckpoint {
            images: checkpoint.images,
            next: checkpoint.next,
        };
        let restored = SourceRecord::restore_save_state(&save, Some(&mut memory), &mut restore).expect("restore");
        assert_eq!(restored.filename().expect("filename"), "heap-round");
        assert_eq!(restored.tokens().expect("tokens"), 15);
        assert_eq!(restored.hash_head(5).expect("hash"), 50);
        assert!(restored.has_current_indent().expect("current"));
    }

    #[test]
    fn restore_rejects_bad_extents() {
        let mut checkpoint = FakeCheckpoint::new();
        checkpoint.images.insert(0, vec![0u8; 10]);
        let mut save = managed_save();
        save.backing = SourceBackingSave::Heap { allocation: 0 };
        let error = SourceRecord::restore_save_state(&save, None, &mut checkpoint).expect_err("record");
        assert_eq!(bad_ui(error), "invalid source extent");

        let mut checkpoint = FakeCheckpoint::new();
        checkpoint.images.insert(0, vec![0u8; 8]);
        let mut save = managed_save();
        let mut memory = FakeMemory::new();
        save.hash = SourceHashSave::Heap { allocation: 0 };
        let error = SourceRecord::restore_save_state(&save, Some(&mut memory), &mut checkpoint).expect_err("hash");
        assert_eq!(bad_ui(error), "invalid define hash extent");

        let mut checkpoint = FakeCheckpoint::new();
        checkpoint.images.insert(0, vec![0u8; 8]);
        let mut save = managed_save();
        let mut memory = FakeMemory::new();
        save.next_indent = 2;
        save.indents = vec![SourceIndentSaveState {
            id: 1,
            state: SourceIndentBackingSave::Heap { allocation: 0 },
        }];
        let error = SourceRecord::restore_save_state(&save, Some(&mut memory), &mut checkpoint).expect_err("indent");
        assert_eq!(bad_ui(error), "invalid indent extent");
    }

    #[test]
    fn restore_rejects_heap_state_without_owner() {
        let mut checkpoint = FakeCheckpoint::new();
        checkpoint.images.insert(0, vec![0u8; 4096]);
        let mut save = managed_save();
        save.hash = SourceHashSave::Heap { allocation: 0 };
        let error = SourceRecord::restore_save_state(&save, None, &mut checkpoint).expect_err("hash");
        assert_eq!(bad_ui(error), "heap hash requires memory owner");

        let mut checkpoint = FakeCheckpoint::new();
        checkpoint.images.insert(0, vec![0u8; 16]);
        let mut save = managed_save();
        save.next_indent = 2;
        save.indents = vec![SourceIndentSaveState {
            id: 1,
            state: SourceIndentBackingSave::Heap { allocation: 0 },
        }];
        let error = SourceRecord::restore_save_state(&save, None, &mut checkpoint).expect_err("indent");
        assert_eq!(bad_ui(error), "heap indent requires memory owner");
    }

    #[test]
    fn restore_rejects_bad_buckets_identities_and_counter() {
        let mut save = managed_save();
        save.hash = SourceHashSave::Managed {
            heads: vec![SourceHashHeadSave { bucket: 1024, id: 1 }],
        };
        let mut checkpoint = FakeCheckpoint::new();
        let error = SourceRecord::restore_save_state(&save, None, &mut checkpoint).expect_err("bucket");
        assert_eq!(bad_ui(error), "invalid hash bucket");

        let mut save = managed_save();
        save.hash = SourceHashSave::Managed {
            heads: vec![
                SourceHashHeadSave { bucket: 2, id: 1 },
                SourceHashHeadSave { bucket: 2, id: 3 },
            ],
        };
        let mut checkpoint = FakeCheckpoint::new();
        let error = SourceRecord::restore_save_state(&save, None, &mut checkpoint).expect_err("dup");
        assert_eq!(bad_ui(error), "invalid hash bucket");

        let mut save = managed_save();
        save.next_indent = 2;
        save.indents = vec![SourceIndentSaveState {
            id: 2,
            state: SourceIndentBackingSave::Managed {
                indent_type: 1,
                skip: 0,
                script: 3,
                next: 0,
            },
        }];
        let mut checkpoint = FakeCheckpoint::new();
        let error = SourceRecord::restore_save_state(&save, None, &mut checkpoint).expect_err("identity");
        assert_eq!(bad_ui(error), "invalid indent identity");

        let mut save = managed_save();
        save.next_indent = 0;
        let mut checkpoint = FakeCheckpoint::new();
        let error = SourceRecord::restore_save_state(&save, None, &mut checkpoint).expect_err("counter");
        assert_eq!(bad_ui(error), "script.source.nextIndent: expected an integer in range");
    }

    #[test]
    fn bad_indent_type_fails_on_read_and_missing_link_fails_on_lookup() {
        let mut save = managed_save();
        save.next_indent = 2;
        save.indents = vec![SourceIndentSaveState {
            id: 1,
            state: SourceIndentBackingSave::Managed {
                indent_type: 3,
                skip: 1,
                script: 3,
                next: 0,
            },
        }];
        if let SourceBackingSave::Managed { indent, .. } = &mut save.backing {
            *indent = 1;
        } else {
            panic!("expected managed backing");
        }
        let mut checkpoint = FakeCheckpoint::new();
        let mut restored = SourceRecord::restore_save_state(&save, None, &mut checkpoint).expect("restore");
        let error = restored.pop_indent(None).expect_err("bad type");
        assert_eq!(bad_ui(error), "source indent has an unsupported directive type");

        let mut save = managed_save();
        save.next_indent = 8;
        if let SourceBackingSave::Managed { indent, .. } = &mut save.backing {
            *indent = 7;
        } else {
            panic!("expected managed backing");
        }
        let mut checkpoint = FakeCheckpoint::new();
        let restored = SourceRecord::restore_save_state(&save, None, &mut checkpoint).expect("restore");
        let error = restored.has_current_indent().expect_err("missing link");
        assert_eq!(
            bad_ui(error),
            "source indent pointer does not identify a live allocation"
        );
    }

    #[test]
    fn indent_type_values_match_donor() {
        assert_eq!(SourceIndentType::If.value(), 1);
        assert_eq!(SourceIndentType::Else.value(), 2);
        assert_eq!(SourceIndentType::Elif.value(), 4);
        assert_eq!(SourceIndentType::Ifdef.value(), 8);
        assert_eq!(SourceIndentType::Ifndef.value(), 16);
        assert_eq!(SourceIndentType::from_value(8).expect("ifdef"), SourceIndentType::Ifdef);
        assert!(SourceIndentType::from_value(3).is_err());
    }
}
