//! Token and define ownership for the legacy menu script precompiler.
//!
//! Donor provenance: `src/ui/common/legacy/script/precomp-memory.ts`
//! (token and define ownership from id Software's `botlib/l_precomp.c/h`).
//!
//! [`PrecompMemory`] owns retained tokens and defines by numeric identity.
//! Link words (`next`, `tokens`, `parms`) hold owner-local ids; `0` ends a
//! chain. A define occupies [`SOURCE_DEFINE_BYTES`] header bytes plus its
//! NUL-terminated name (Linux i386 layout, little-endian):
//!
//! | Range  | Field     | Type  |
//! |--------|-----------|-------|
//! | 0..4   | name pointer (byte offset, always 32) | `u32` |
//! | 4..8   | flags     | `i32` |
//! | 8..12  | builtin   | `i32` |
//! | 12..16 | numparms  | `i32` |
//! | 16..20 | parms head id | `u32` |
//! | 20..24 | tokens head id | `u32` |
//! | 24..28 | next define id | `u32` |
//! | 28..32 | hashnext define id | `u32` |
//! | 32..   | name bytes plus NUL | Latin-1 |
//!
//! # Adaptations
//!
//! * The donor `PrecompDefine.owner` back-reference is unrepresentable with
//!   Rust borrows. [`PrecompDefine::parameter_index`] takes the owner
//!   explicitly, and [`PrecompMemory::copy_define`] takes the source heap
//!   explicitly (both donor call sites copy across heaps).
//!   [`PrecompMemory::duplicate_define`] covers same-heap copies, which
//!   cannot pass one heap as both `&mut self` and `&PrecompMemory`.
//! * [`PrecompMemory::token`] and [`PrecompMemory::define`] return owned
//!   clones sharing the same byte store (donor reference semantics), so
//!   callers can hold tokens and defines across `&mut` heap calls. Byte and
//!   word writes through a clone stay visible to the owner, while `context`
//!   and `unsupported` are per-handle; that matches the donor flows, which
//!   only ever mutate those on owned handles (fields, parameters, fresh
//!   locals), never on heap-fetched entries.
//! * The donor `chain` generator becomes the [`PrecompTokenChain`] iterator
//!   with the same laziness and cycle guard.
//! * `SaveReader` checkpoints become typed save structs
//!   ([`PrecompMemorySaveState`] and friends). Every donor `RangeError`
//!   becomes [`ClientError::BadUi`] carrying the donor message;
//!   save-envelope validations keep their `script.heap: ` and
//!   `script.precompToken: ` path prefixes, matching `SaveReader` roots.
//! * [`ScriptMemory`], [`ScriptMemoryAllocation`], [`ScriptMemoryCapture`],
//!   and [`ScriptMemoryRestore`] mirror
//!   `src/ui/common/legacy/script/memory.ts`, whose Rust port is in flight;
//!   they move to `super::memory` on integration.
//! * Fresh allocations are zeroed. The donor `allocate(..., clear: false)`
//!   paths always initialize before reading (define creation writes every
//!   header word; token copies overwrite all bytes), so zeroing is
//!   unobservable. Define names are validated before allocating; the donor
//!   orphans one block when a name fails validation, and that leak is not
//!   reproduced.
//! * The donor file performs no asynchronous reads. Memory ownership and
//!   checkpointing arrive as injected synchronous callbacks: the
//!   [`ScriptMemory`], [`ScriptMemoryCapture`], and [`ScriptMemoryRestore`]
//!   traits (the donor interfaces are already synchronous).

use std::collections::BTreeMap;

use super::token_memory::{
    SharedScriptBytes, SourceTokenContext, SourceTokenMemory, SourceTokenSaveState, SOURCE_TOKEN_BYTES,
};
use crate::error::ClientError;

/// Size of one retained `define_t` header, in bytes
/// (donor `SOURCE_DEFINE_BYTES`). The allocation extends past the header
/// with the NUL-terminated name.
pub const SOURCE_DEFINE_BYTES: usize = 32;

/// Offset of the `u32` name-pointer word (byte offset, always 32).
const DEFINE_NAME: usize = 0;
/// Offset of the `i32` flags word.
const DEFINE_FLAGS: usize = 4;
/// Offset of the `i32` builtin word.
const DEFINE_BUILTIN: usize = 8;
/// Offset of the `i32` numparms word.
const DEFINE_NUMPARMS: usize = 12;
/// Offset of the `u32` parms-chain head word.
const DEFINE_PARMS: usize = 16;
/// Offset of the `u32` tokens-chain head word.
const DEFINE_TOKENS: usize = 20;
/// Offset of the `u32` next-define link word.
const DEFINE_NEXT: usize = 24;
/// Offset of the `u32` hashnext-define link word.
const DEFINE_HASHNEXT: usize = 28;

/// View validation failure: the window reaches outside its store.
fn allocation_view_error() -> ClientError {
    ClientError::BadUi("script allocation view is outside its storage".to_string())
}

/// Byte index validation failure.
fn allocation_byte_error() -> ClientError {
    ClientError::BadUi("script allocation byte is outside its extent".to_string())
}

/// Word validation failure (donor engine `DataView` range error equivalent).
fn allocation_field_error() -> ClientError {
    ClientError::BadUi("script allocation field is outside its extent".to_string())
}

/// One heap block with shared byte identity (donor `ScriptMemoryAllocation`,
/// memory mirror).
///
/// Clones share the same bytes, so a [`PrecompToken`] and its allocation
/// (or two handles from [`PrecompMemory::token`]) observe each other's
/// writes, matching the donor `Uint8Array` aliasing.
#[derive(Debug, Clone)]
pub struct ScriptMemoryAllocation {
    store: SharedScriptBytes,
    base: usize,
    len: usize,
}

impl ScriptMemoryAllocation {
    /// Allocate a zeroed block of `size` bytes.
    #[must_use]
    pub fn zeroed(size: usize) -> Self {
        Self {
            store: SharedScriptBytes::zeroed(size),
            base: 0,
            len: size,
        }
    }

    /// Wrap an existing byte vector as one block.
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let len = bytes.len();
        Self {
            store: SharedScriptBytes::from_vec(bytes),
            base: 0,
            len,
        }
    }

    /// Block length, in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the block is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Read one byte (donor `allocation.bytes[index]`).
    pub fn byte(&self, index: usize) -> Result<u8, ClientError> {
        self.with_view(|view| view.get(index).copied().ok_or_else(allocation_byte_error))
    }

    /// Write one byte.
    pub fn set_byte(&mut self, index: usize, value: u8) -> Result<(), ClientError> {
        self.with_view_mut(|view| {
            *view.get_mut(index).ok_or_else(allocation_byte_error)? = value;
            Ok(())
        })
    }

    /// Fill the whole block (donor `bytes.fill`).
    pub fn fill(&mut self, value: u8) -> Result<(), ClientError> {
        self.with_view_mut(|view| {
            view.fill(value);
            Ok(())
        })
    }

    /// Copy of the live block bytes (donor `bytes.slice()`).
    pub fn snapshot_bytes(&self) -> Result<Vec<u8>, ClientError> {
        self.with_view(|view| Ok(view.to_vec()))
    }

    /// Copy bytes into the block prefix (donor `bytes.set`).
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        if bytes.len() > self.len {
            return Err(ClientError::BadUi(
                "script allocation write exceeds its extent".to_string(),
            ));
        }
        self.with_view_mut(|view| {
            view[..bytes.len()].copy_from_slice(bytes);
            Ok(())
        })
    }

    /// Fill one byte range (donor header clear in define creation).
    fn fill_range(&mut self, start: usize, end: usize, value: u8) -> Result<(), ClientError> {
        self.with_view_mut(|view| {
            let range = view.get_mut(start..end).ok_or_else(allocation_field_error)?;
            range.fill(value);
            Ok(())
        })
    }

    /// Read a little-endian `u32` word.
    fn read_u32(&self, offset: usize) -> Result<u32, ClientError> {
        self.with_view(|view| {
            let end = offset.checked_add(4).ok_or_else(allocation_field_error)?;
            let field = view.get(offset..end).ok_or_else(allocation_field_error)?;
            Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
        })
    }

    /// Read a little-endian `i32` word.
    fn read_i32(&self, offset: usize) -> Result<i32, ClientError> {
        self.with_view(|view| {
            let end = offset.checked_add(4).ok_or_else(allocation_field_error)?;
            let field = view.get(offset..end).ok_or_else(allocation_field_error)?;
            Ok(i32::from_le_bytes([field[0], field[1], field[2], field[3]]))
        })
    }

    /// Write a little-endian `u32` word.
    fn write_u32(&mut self, offset: usize, value: u32) -> Result<(), ClientError> {
        self.with_view_mut(|view| {
            let end = offset.checked_add(4).ok_or_else(allocation_field_error)?;
            let field = view.get_mut(offset..end).ok_or_else(allocation_field_error)?;
            field.copy_from_slice(&value.to_le_bytes());
            Ok(())
        })
    }

    /// Write a little-endian `i32` word.
    fn write_i32(&mut self, offset: usize, value: i32) -> Result<(), ClientError> {
        self.with_view_mut(|view| {
            let end = offset.checked_add(4).ok_or_else(allocation_field_error)?;
            let field = view.get_mut(offset..end).ok_or_else(allocation_field_error)?;
            field.copy_from_slice(&value.to_le_bytes());
            Ok(())
        })
    }

    /// Run `read` against the validated block window.
    fn with_view<R>(&self, read: impl FnOnce(&[u8]) -> Result<R, ClientError>) -> Result<R, ClientError> {
        let (base, len) = (self.base, self.len);
        self.store.with_bytes(|bytes| {
            let end = base.checked_add(len).ok_or_else(allocation_view_error)?;
            let view = bytes.get(base..end).ok_or_else(allocation_view_error)?;
            read(view)
        })
    }

    /// Run `write` against the validated block window.
    fn with_view_mut<R>(&mut self, write: impl FnOnce(&mut [u8]) -> Result<R, ClientError>) -> Result<R, ClientError> {
        let (base, len) = (self.base, self.len);
        self.store.with_bytes_mut(|bytes| {
            let end = base.checked_add(len).ok_or_else(allocation_view_error)?;
            let view = bytes.get_mut(base..end).ok_or_else(allocation_view_error)?;
            write(view)
        })
    }
}

/// Heap block owner supplied by the runtime composition
/// (donor `ScriptMemory`, memory mirror).
///
/// The donor `kind` argument is always `"heap"` from this module and the
/// `clear` flag is folded into [`ScriptMemoryAllocation::zeroed`].
pub trait ScriptMemory {
    /// Allocate a zeroed block of `size` bytes.
    fn allocate(&mut self, size: usize) -> ScriptMemoryAllocation;

    /// Release a block previously returned by [`ScriptMemory::allocate`].
    /// Shared views keep their bytes alive until dropped.
    fn free(&mut self, allocation: &ScriptMemoryAllocation);
}

/// Checkpoint writer mapping live allocations to stable references
/// (donor `ScriptMemoryCapture`, memory mirror).
pub trait ScriptMemoryCapture {
    /// Record an allocation and return its checkpoint reference.
    fn reference(&mut self, allocation: &ScriptMemoryAllocation) -> u32;
}

/// Checkpoint reader resolving stable references back to allocations
/// (donor `ScriptMemoryRestore`, memory mirror).
pub trait ScriptMemoryRestore {
    /// Resolve a checkpoint reference into a live allocation.
    fn allocation(&mut self, id: u32) -> Result<ScriptMemoryAllocation, ClientError>;
}

/// One owned precompiler token: retained bytes plus decode context
/// (donor `PrecompToken`).
#[derive(Debug, Clone)]
pub struct PrecompToken {
    /// Owner-local identity (`0` for stack-local tokens).
    pub id: u32,
    /// Owning block; shares bytes with [`PrecompToken::token`].
    pub allocation: ScriptMemoryAllocation,
    /// Retained token bytes viewing the allocation.
    pub token: SourceTokenMemory,
    /// Decode position and leading trivia.
    pub context: SourceTokenContext,
    /// Reason the token falls outside the supported source profile, if any.
    pub unsupported: Option<String>,
}

impl PrecompToken {
    /// Wrap an allocation, viewing its bytes as a retained token.
    /// The window is validated lazily on access, like the donor.
    pub fn new(id: u32, allocation: ScriptMemoryAllocation) -> Self {
        let token = SourceTokenMemory::view(&allocation.store, allocation.base, allocation.len);
        Self {
            id,
            allocation,
            token,
            context: SourceTokenContext {
                path: String::new(),
                column: 1,
                leading_whitespace: String::new(),
            },
            unsupported: None,
        }
    }

    /// Copy retained bytes, context, and profile flag from another token
    /// (donor `copyFrom`).
    pub fn copy_from(&mut self, other: &Self) -> Result<(), ClientError> {
        self.token.copy_from(&other.token)?;
        self.context = other.context.clone();
        self.unsupported = other.unsupported.clone();
        Ok(())
    }

    /// Clear the whitespace pointers and leading trivia, keeping the path
    /// and column (donor `clearWhitespace`).
    pub fn clear_whitespace(&mut self) -> Result<(), ClientError> {
        self.token.set_whitespace_start(0)?;
        self.token.set_whitespace_end(0)?;
        self.token.set_lines_crossed(0)?;
        self.context.leading_whitespace.clear();
        Ok(())
    }

    /// Capture a checkpoint (donor `captureSaveState`).
    pub fn capture_save_state(&self) -> Result<PrecompTokenSaveState, ClientError> {
        Ok(PrecompTokenSaveState {
            token: self.token.capture_save_state()?,
            context: self.context.clone(),
            unsupported: self.unsupported.clone(),
        })
    }

    /// Restore a checkpoint; with `verify_bytes` the stored image is
    /// compared against the live bytes instead of being written back
    /// (donor `restoreSaveState`).
    pub fn restore_save_state(&mut self, state: &PrecompTokenSaveState, verify_bytes: bool) -> Result<(), ClientError> {
        self.token.restore_save_state(&state.token, verify_bytes)?;
        if state.context.column < 0 {
            return Err(ClientError::BadUi(
                "script.precompToken.context.column: expected an integer in range".to_string(),
            ));
        }
        self.context = state.context.clone();
        self.unsupported = state.unsupported.clone();
        Ok(())
    }
}

/// Stack-local token over fresh bytes (donor `localToken`).
#[must_use]
pub fn local_token() -> PrecompToken {
    PrecompToken::new(0, ScriptMemoryAllocation::zeroed(SOURCE_TOKEN_BYTES))
}

/// Checkpoint of one precompiler token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecompTokenSaveState {
    /// Retained token checkpoint.
    pub token: SourceTokenSaveState,
    /// Decode context.
    pub context: SourceTokenContext,
    /// Profile flag.
    pub unsupported: Option<String>,
}

/// One owned precompiler define: header words plus name bytes
/// (donor `PrecompDefine`).
///
/// Unlike the donor, this holds no owner back-reference; methods that walk
/// token chains take the owner explicitly.
#[derive(Debug, Clone)]
pub struct PrecompDefine {
    /// Owner-local identity.
    pub id: u32,
    /// Owning block: header words plus name bytes.
    pub allocation: ScriptMemoryAllocation,
}

impl PrecompDefine {
    /// Wrap an allocation as a define. Header words are validated lazily
    /// on access, like the donor.
    pub fn new(id: u32, allocation: ScriptMemoryAllocation) -> Self {
        Self { id, allocation }
    }

    /// NUL-terminated Latin-1 name (donor `name` getter).
    pub fn name(&self) -> Result<String, ClientError> {
        let start = self.allocation.read_u32(DEFINE_NAME)? as usize;
        if start >= self.allocation.len() {
            return Err(ClientError::BadUi(
                "define name pointer is outside its allocation".to_string(),
            ));
        }
        let mut name = String::new();
        for index in start..self.allocation.len() {
            let byte = self.allocation.byte(index)?;
            if byte == 0 {
                return Ok(name);
            }
            name.push(byte as char);
        }
        Err(ClientError::BadUi(
            "define name has no terminator in its allocation".to_string(),
        ))
    }

    /// Flags word (donor `flags` getter).
    pub fn flags(&self) -> Result<i32, ClientError> {
        self.allocation.read_i32(DEFINE_FLAGS)
    }

    /// Set the flags word (donor `flags` setter).
    pub fn set_flags(&mut self, value: i32) -> Result<(), ClientError> {
        self.allocation.write_i32(DEFINE_FLAGS, value)
    }

    /// Builtin word (donor `builtin` getter).
    pub fn builtin(&self) -> Result<i32, ClientError> {
        self.allocation.read_i32(DEFINE_BUILTIN)
    }

    /// Set the builtin word (donor `builtin` setter).
    pub fn set_builtin(&mut self, value: i32) -> Result<(), ClientError> {
        self.allocation.write_i32(DEFINE_BUILTIN, value)
    }

    /// Parameter count word (donor `numparms` getter).
    pub fn numparms(&self) -> Result<i32, ClientError> {
        self.allocation.read_i32(DEFINE_NUMPARMS)
    }

    /// Set the parameter count word (donor `numparms` setter).
    pub fn set_numparms(&mut self, value: i32) -> Result<(), ClientError> {
        self.allocation.write_i32(DEFINE_NUMPARMS, value)
    }

    /// Parameters-chain head id (donor `parms` getter).
    pub fn parms(&self) -> Result<u32, ClientError> {
        self.allocation.read_u32(DEFINE_PARMS)
    }

    /// Set the parameters-chain head id (donor `parms` setter).
    pub fn set_parms(&mut self, value: u32) -> Result<(), ClientError> {
        self.allocation.write_u32(DEFINE_PARMS, value)
    }

    /// Replacement-tokens-chain head id (donor `tokens` getter).
    pub fn tokens(&self) -> Result<u32, ClientError> {
        self.allocation.read_u32(DEFINE_TOKENS)
    }

    /// Set the replacement-tokens-chain head id (donor `tokens` setter).
    pub fn set_tokens(&mut self, value: u32) -> Result<(), ClientError> {
        self.allocation.write_u32(DEFINE_TOKENS, value)
    }

    /// Next-define link id (donor `next` getter).
    pub fn next(&self) -> Result<u32, ClientError> {
        self.allocation.read_u32(DEFINE_NEXT)
    }

    /// Set the next-define link id (donor `next` setter).
    pub fn set_next(&mut self, value: u32) -> Result<(), ClientError> {
        self.allocation.write_u32(DEFINE_NEXT, value)
    }

    /// Hash-chain link id (donor `hashnext` getter).
    pub fn hashnext(&self) -> Result<u32, ClientError> {
        self.allocation.read_u32(DEFINE_HASHNEXT)
    }

    /// Set the hash-chain link id (donor `hashnext` setter).
    pub fn set_hashnext(&mut self, value: u32) -> Result<(), ClientError> {
        self.allocation.write_u32(DEFINE_HASHNEXT, value)
    }

    /// Whether the fixed flag bit is set (donor `fixed` getter).
    pub fn fixed(&self) -> Result<bool, ClientError> {
        Ok(self.flags()? & 1 != 0)
    }

    /// Index of a parameter by name, or `-1` (donor `parameterIndex`).
    /// The owner arrives explicitly because defines hold no back-reference.
    pub fn parameter_index(&self, owner: &PrecompMemory, name: &str) -> Result<i32, ClientError> {
        for (index, parameter) in owner.chain(self.parms()?).enumerate() {
            if parameter?.token.string()? == name {
                return Ok(index as i32);
            }
        }
        Ok(-1)
    }
}

/// Token and define owner: maps retain identities while every list consumer
/// reads link words from the actual allocations (donor `PrecompMemory`).
pub struct PrecompMemory {
    tokens: BTreeMap<u32, PrecompToken>,
    defines: BTreeMap<u32, PrecompDefine>,
    next_token: u32,
    next_define: u32,
    memory: Option<Box<dyn ScriptMemory>>,
}

impl PrecompMemory {
    /// Create an owner over an optional heap block owner. Without one,
    /// allocations are fresh zeroed blocks and frees are no-ops
    /// (donor constructor with `memory | undefined`).
    pub fn new(memory: Option<Box<dyn ScriptMemory>>) -> Self {
        Self {
            tokens: BTreeMap::new(),
            defines: BTreeMap::new(),
            next_token: 1,
            next_define: 1,
            memory,
        }
    }

    /// Deep-copy a token into a fresh allocation with a cleared link word
    /// (donor `copyToken`).
    pub fn copy_token(&mut self, token: &PrecompToken) -> Result<PrecompToken, ClientError> {
        let id = self.claim_token_id()?;
        let mut copied = PrecompToken::new(id, self.allocate(SOURCE_TOKEN_BYTES));
        copied.copy_from(token)?;
        copied.token.set_next(0)?;
        self.tokens.insert(id, copied.clone());
        Ok(copied)
    }

    /// Resolve a live token id (donor `token`).
    pub fn token(&self, id: u32) -> Result<PrecompToken, ClientError> {
        self.tokens
            .get(&id)
            .cloned()
            .ok_or_else(|| ClientError::BadUi("source token pointer does not identify a live token".to_string()))
    }

    /// Walk a `next`-linked chain from `head` (donor `chain`).
    pub fn chain(&self, head: u32) -> PrecompTokenChain<'_> {
        PrecompTokenChain {
            owner: self,
            next: head,
            remaining: self.tokens.len().saturating_add(1),
        }
    }

    /// Release a token's block and drop its identity (donor `freeToken`).
    pub fn free_token(&mut self, token: &PrecompToken) {
        if let Some(memory) = self.memory.as_mut() {
            memory.free(&token.allocation);
        }
        self.tokens.remove(&token.id);
    }

    /// Release a whole `next`-linked chain (donor `freeTokens`).
    pub fn free_tokens(&mut self, head: u32) -> Result<(), ClientError> {
        let mut id = head;
        while id != 0 {
            let token = self.token(id)?;
            id = token.token.next()?;
            self.free_token(&token);
        }
        Ok(())
    }

    /// Allocate a define for `name`, clearing the header when `clear` is set
    /// (donor `allocateDefine`).
    pub fn allocate_define(&mut self, name: &str, clear: bool) -> Result<PrecompDefine, ClientError> {
        self.create_define(name, clear)
    }

    /// Resolve a live define id (donor `define`).
    pub fn define(&self, id: u32) -> Result<PrecompDefine, ClientError> {
        self.defines
            .get(&id)
            .cloned()
            .ok_or_else(|| ClientError::BadUi("source define pointer does not identify a live definition".to_string()))
    }

    /// Deep-copy a define from another owner, with fresh token and parameter
    /// chains and cleared `next`/`hashnext` links (donor `copyDefine`).
    pub fn copy_define(&mut self, source: &PrecompMemory, define_id: u32) -> Result<PrecompDefine, ClientError> {
        let origin = source.define(define_id)?;
        let tokens = source.chain(origin.tokens()?).collect::<Result<Vec<_>, _>>()?;
        let parms = source.chain(origin.parms()?).collect::<Result<Vec<_>, _>>()?;
        self.import_define(&origin, &tokens, &parms)
    }

    /// Deep-copy a define within this owner (donor `copyDefine` with the
    /// same owner on both sides).
    pub fn duplicate_define(&mut self, define_id: u32) -> Result<PrecompDefine, ClientError> {
        let origin = self.define(define_id)?;
        let tokens = self.chain(origin.tokens()?).collect::<Result<Vec<_>, _>>()?;
        let parms = self.chain(origin.parms()?).collect::<Result<Vec<_>, _>>()?;
        self.import_define(&origin, &tokens, &parms)
    }

    /// Release a define's chains and block, and drop its identity
    /// (donor `freeDefine`).
    pub fn free_define(&mut self, define: &PrecompDefine) -> Result<(), ClientError> {
        let parms = define.parms()?;
        let tokens = define.tokens()?;
        self.free_tokens(parms)?;
        self.free_tokens(tokens)?;
        if let Some(memory) = self.memory.as_mut() {
            memory.free(&define.allocation);
        }
        self.defines.remove(&define.id);
        Ok(())
    }

    /// Capture a checkpoint (donor `captureSaveState`).
    pub fn capture_save_state(
        &self,
        capture: &mut dyn ScriptMemoryCapture,
    ) -> Result<PrecompMemorySaveState, ClientError> {
        let mut tokens = Vec::with_capacity(self.tokens.len());
        for token in self.tokens.values() {
            tokens.push(PrecompTokenSaveEntry {
                id: token.id,
                allocation: capture.reference(&token.allocation),
                state: token.capture_save_state()?,
            });
        }
        let mut defines = Vec::with_capacity(self.defines.len());
        for define in self.defines.values() {
            defines.push(PrecompDefineSaveEntry {
                id: define.id,
                allocation: capture.reference(&define.allocation),
            });
        }
        Ok(PrecompMemorySaveState {
            next_token: self.next_token,
            next_define: self.next_define,
            tokens,
            defines,
        })
    }

    /// Restore a checkpoint (donor `restoreSaveState`).
    pub fn restore_save_state(
        &mut self,
        state: &PrecompMemorySaveState,
        restore: &mut dyn ScriptMemoryRestore,
    ) -> Result<(), ClientError> {
        if state.next_token < 1 {
            return Err(ClientError::BadUi(
                "script.heap.nextToken: expected an integer in range".to_string(),
            ));
        }
        if state.next_define < 1 {
            return Err(ClientError::BadUi(
                "script.heap.nextDefine: expected an integer in range".to_string(),
            ));
        }
        self.next_token = state.next_token;
        self.next_define = state.next_define;
        self.tokens.clear();
        self.defines.clear();
        for (index, entry) in state.tokens.iter().enumerate() {
            if entry.id < 1 {
                return Err(ClientError::BadUi(format!(
                    "script.heap.tokens[{index}].id: expected an integer in range"
                )));
            }
            if entry.id >= self.next_token || self.tokens.contains_key(&entry.id) {
                return Err(ClientError::BadUi(format!(
                    "script.heap.tokens[{index}]: invalid token identity"
                )));
            }
            let allocation = restore.allocation(entry.allocation)?;
            let mut token = PrecompToken::new(entry.id, allocation);
            token.restore_save_state(&entry.state, true)?;
            self.tokens.insert(entry.id, token);
        }
        for (index, entry) in state.defines.iter().enumerate() {
            if entry.id < 1 {
                return Err(ClientError::BadUi(format!(
                    "script.heap.defines[{index}].id: expected an integer in range"
                )));
            }
            if entry.id >= self.next_define || self.defines.contains_key(&entry.id) {
                return Err(ClientError::BadUi(format!(
                    "script.heap.defines[{index}]: invalid define identity"
                )));
            }
            let allocation = restore.allocation(entry.allocation)?;
            if allocation.len() < SOURCE_DEFINE_BYTES + 1 {
                return Err(ClientError::BadUi(format!(
                    "script.heap.defines[{index}]: invalid define extent"
                )));
            }
            self.defines.insert(entry.id, PrecompDefine::new(entry.id, allocation));
        }
        Ok(())
    }

    /// Allocate a define block, publish its name pointer and name
    /// (donor `createDefine`).
    fn create_define(&mut self, name: &str, clear: bool) -> Result<PrecompDefine, ClientError> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut narrow = Vec::with_capacity(units.len());
        for unit in &units {
            if *unit > 255 {
                return Err(ClientError::BadUi("define name must contain source bytes".to_string()));
            }
            narrow.push(*unit as u8);
        }
        let mut allocation = self.allocate(SOURCE_DEFINE_BYTES + narrow.len() + 1);
        if clear {
            allocation.fill_range(0, SOURCE_DEFINE_BYTES, 0)?;
        }
        allocation.write_u32(DEFINE_NAME, SOURCE_DEFINE_BYTES as u32)?;
        if SOURCE_DEFINE_BYTES + narrow.len() >= allocation.len() {
            return Err(ClientError::BadUi(
                "copied define name exceeds its reached allocation".to_string(),
            ));
        }
        for (index, byte) in narrow.iter().enumerate() {
            allocation.set_byte(SOURCE_DEFINE_BYTES + index, *byte)?;
        }
        allocation.set_byte(SOURCE_DEFINE_BYTES + narrow.len(), 0)?;
        let id = self.claim_define_id()?;
        let define = PrecompDefine::new(id, allocation);
        self.defines.insert(id, define.clone());
        Ok(define)
    }

    /// Import a copied define body with fresh chains (donor `copyDefine` tail).
    fn import_define(
        &mut self,
        origin: &PrecompDefine,
        tokens: &[PrecompToken],
        parms: &[PrecompToken],
    ) -> Result<PrecompDefine, ClientError> {
        let mut copied = self.create_define(&origin.name()?, false)?;
        copied.set_flags(origin.flags()?)?;
        copied.set_builtin(origin.builtin()?)?;
        copied.set_numparms(origin.numparms()?)?;
        copied.set_next(0)?;
        copied.set_hashnext(0)?;
        copied.set_tokens(0)?;
        let mut last: Option<PrecompToken> = None;
        for token in tokens {
            let next = self.copy_token(token)?;
            match last.as_mut() {
                None => copied.set_tokens(next.id)?,
                Some(previous) => previous.token.set_next(next.id)?,
            }
            last = Some(next);
        }
        copied.set_parms(0)?;
        last = None;
        for token in parms {
            let next = self.copy_token(token)?;
            match last.as_mut() {
                None => copied.set_parms(next.id)?,
                Some(previous) => previous.token.set_next(next.id)?,
            }
            last = Some(next);
        }
        Ok(copied)
    }

    /// Claim the next token identity.
    fn claim_token_id(&mut self) -> Result<u32, ClientError> {
        let id = self.next_token;
        self.next_token = self
            .next_token
            .checked_add(1)
            .ok_or_else(|| ClientError::BadUi("script.heap: token identity overflow".to_string()))?;
        Ok(id)
    }

    /// Claim the next define identity.
    fn claim_define_id(&mut self) -> Result<u32, ClientError> {
        let id = self.next_define;
        self.next_define = self
            .next_define
            .checked_add(1)
            .ok_or_else(|| ClientError::BadUi("script.heap: define identity overflow".to_string()))?;
        Ok(id)
    }

    /// Allocate a block from the heap owner, or a fresh zeroed block without
    /// one (donor `allocate`).
    fn allocate(&mut self, size: usize) -> ScriptMemoryAllocation {
        match self.memory.as_mut() {
            Some(memory) => memory.allocate(size),
            None => ScriptMemoryAllocation::zeroed(size),
        }
    }
}

impl Default for PrecompMemory {
    /// Owner without a heap block owner.
    fn default() -> Self {
        Self::new(None)
    }
}

/// Lazy `next`-linked chain walker (donor `chain` generator).
pub struct PrecompTokenChain<'a> {
    owner: &'a PrecompMemory,
    next: u32,
    remaining: usize,
}

impl<'a> Iterator for PrecompTokenChain<'a> {
    type Item = Result<PrecompToken, ClientError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == 0 {
            return None;
        }
        self.remaining = self.remaining.saturating_sub(1);
        if self.remaining == 0 {
            return Some(Err(ClientError::BadUi(
                "source token chain contains a cycle".to_string(),
            )));
        }
        let token = match self.owner.token(self.next) {
            Ok(token) => token,
            Err(error) => return Some(Err(error)),
        };
        match token.token.next() {
            Ok(id) => self.next = id,
            Err(error) => return Some(Err(error)),
        }
        Some(Ok(token))
    }
}

/// Checkpoint of one owned token identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecompTokenSaveEntry {
    /// Token identity.
    pub id: u32,
    /// Checkpoint reference of the owning block.
    pub allocation: u32,
    /// Token checkpoint.
    pub state: PrecompTokenSaveState,
}

/// Checkpoint of one owned define identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecompDefineSaveEntry {
    /// Define identity.
    pub id: u32,
    /// Checkpoint reference of the owning block.
    pub allocation: u32,
}

/// Checkpoint of the token and define owner (donor `captureSaveState` record).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecompMemorySaveState {
    /// Next token identity.
    pub next_token: u32,
    /// Next define identity.
    pub next_define: u32,
    /// Owned tokens in identity order.
    pub tokens: Vec<PrecompTokenSaveEntry>,
    /// Owned defines in identity order.
    pub defines: Vec<PrecompDefineSaveEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Heap block owner recording allocate/free calls.
    #[derive(Default)]
    struct FakeMemory {
        allocated: Vec<usize>,
        freed: usize,
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

    /// Checkpoint writer snapshotting allocation bytes by index.
    #[derive(Default)]
    struct FakeCapture {
        blobs: Vec<Vec<u8>>,
    }

    impl ScriptMemoryCapture for FakeCapture {
        fn reference(&mut self, allocation: &ScriptMemoryAllocation) -> u32 {
            let id = self.blobs.len() as u32;
            self.blobs.push(allocation.snapshot_bytes().expect("test allocation"));
            id
        }
    }

    /// Checkpoint reader restoring snapshotted bytes by index.
    struct FakeRestore {
        blobs: Vec<Vec<u8>>,
    }

    impl ScriptMemoryRestore for FakeRestore {
        fn allocation(&mut self, id: u32) -> Result<ScriptMemoryAllocation, ClientError> {
            self.blobs
                .get(id as usize)
                .cloned()
                .map(ScriptMemoryAllocation::from_bytes)
                .ok_or_else(|| ClientError::BadUi(format!("unknown test allocation {id}")))
        }
    }

    /// Copy a stack-local token carrying `text` into the heap.
    fn heap_token(heap: &mut PrecompMemory, text: &str) -> PrecompToken {
        let mut local = local_token();
        local.token.write_string(text).unwrap();
        heap.copy_token(&local).unwrap()
    }

    /// Link `ids` into a `next` chain in order.
    fn link_chain(heap: &PrecompMemory, ids: &[u32]) {
        for pair in ids.windows(2) {
            heap.token(pair[0]).unwrap().token.set_next(pair[1]).unwrap();
        }
    }

    #[test]
    fn define_bytes_is_32() {
        assert_eq!(SOURCE_DEFINE_BYTES, 32);
    }

    #[test]
    fn local_token_defaults() {
        let token = local_token();
        assert_eq!(token.id, 0);
        assert_eq!(token.context.path, "");
        assert_eq!(token.context.column, 1);
        assert_eq!(token.context.leading_whitespace, "");
        assert_eq!(token.unsupported, None);
        assert_eq!(token.allocation.len(), SOURCE_TOKEN_BYTES);
        assert_eq!(token.token.string().unwrap(), "");
    }

    #[test]
    fn allocation_views_share_storage() {
        let allocation = ScriptMemoryAllocation::zeroed(SOURCE_TOKEN_BYTES);
        let mut token = PrecompToken::new(1, allocation.clone());
        token.token.write_string("aliased").unwrap();
        token.token.set_next(5).unwrap();
        // Writes through the token view are visible through the allocation.
        assert_eq!(allocation.byte(0).unwrap(), b'a');
        assert_eq!(allocation.byte(7).unwrap(), 0);
        // Writes through the allocation are visible through the token.
        let mut writer = allocation.clone();
        writer.set_byte(0, b'b').unwrap();
        assert_eq!(token.token.string().unwrap(), "bliased");
        // Handle clones keep observing each other.
        let clone = token.clone();
        token.token.set_next(9).unwrap();
        assert_eq!(clone.token.next().unwrap(), 9);
    }

    #[test]
    fn allocation_byte_helpers() {
        let mut allocation = ScriptMemoryAllocation::zeroed(8);
        assert_eq!(allocation.len(), 8);
        assert!(!allocation.is_empty());
        allocation.fill(0xab).unwrap();
        assert_eq!(allocation.snapshot_bytes().unwrap(), vec![0xab; 8]);
        allocation.write_bytes(&[1, 2, 3]).unwrap();
        assert_eq!(allocation.byte(2).unwrap(), 3);
        assert_eq!(allocation.byte(3).unwrap(), 0xab);
        match allocation.write_bytes(&[0; 9]) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "script allocation write exceeds its extent");
            }
            other => panic!("expected overrun error, got {other:?}"),
        }
        match allocation.byte(8) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "script allocation byte is outside its extent");
            }
            other => panic!("expected byte-range error, got {other:?}"),
        }
    }

    #[test]
    fn precomp_token_copy_and_clear_whitespace() {
        let mut source = local_token();
        source.token.write_string("held").unwrap();
        source.token.set_whitespace_start(4).unwrap();
        source.token.set_whitespace_end(9).unwrap();
        source.token.set_lines_crossed(1).unwrap();
        source.context = SourceTokenContext {
            path: "inc.cfg".to_string(),
            column: 3,
            leading_whitespace: " ".to_string(),
        };
        source.unsupported = Some("fixed point".to_string());

        let mut copied = local_token();
        copied.copy_from(&source).unwrap();
        assert_eq!(copied.token.string().unwrap(), "held");
        assert_eq!(copied.context.leading_whitespace, " ");
        assert_eq!(copied.unsupported.as_deref(), Some("fixed point"));

        copied.clear_whitespace().unwrap();
        assert_eq!(copied.token.whitespace_start().unwrap(), 0);
        assert_eq!(copied.token.whitespace_end().unwrap(), 0);
        assert_eq!(copied.token.lines_crossed().unwrap(), 0);
        assert_eq!(copied.context.path, "inc.cfg");
        assert_eq!(copied.context.column, 3);
        assert_eq!(copied.context.leading_whitespace, "");
    }

    #[test]
    fn precomp_token_save_round_trip() {
        let mut token = local_token();
        token.token.write_string("kept").unwrap();
        token.context.column = 12;
        token.unsupported = Some("wide".to_string());
        let state = token.capture_save_state().unwrap();

        let mut revived = local_token();
        revived.restore_save_state(&state, false).unwrap();
        assert_eq!(revived.token.string().unwrap(), "kept");
        assert_eq!(revived.context.column, 12);
        assert_eq!(revived.unsupported.as_deref(), Some("wide"));

        // Verification compares against the live allocation bytes.
        revived.restore_save_state(&state, true).unwrap();
        let mut bad = state.clone();
        bad.context.column = -1;
        match revived.restore_save_state(&bad, true) {
            Err(ClientError::BadUi(message)) => assert_eq!(
                message,
                "script.precompToken.context.column: expected an integer in range"
            ),
            other => panic!("expected column-range error, got {other:?}"),
        }
    }

    #[test]
    fn define_header_round_trip() {
        let mut heap = PrecompMemory::default();
        let mut define = heap.allocate_define("FOO", true).unwrap();
        assert_eq!(define.id, 1);
        assert_eq!(define.name().unwrap(), "FOO");
        assert_eq!(define.flags().unwrap(), 0);
        assert_eq!(define.builtin().unwrap(), 0);
        assert_eq!(define.numparms().unwrap(), 0);
        assert_eq!(define.parms().unwrap(), 0);
        assert_eq!(define.tokens().unwrap(), 0);
        assert_eq!(define.next().unwrap(), 0);
        assert_eq!(define.hashnext().unwrap(), 0);
        assert!(!define.fixed().unwrap());

        define.set_flags(3).unwrap();
        define.set_builtin(7).unwrap();
        define.set_numparms(2).unwrap();
        define.set_parms(11).unwrap();
        define.set_tokens(12).unwrap();
        define.set_next(13).unwrap();
        define.set_hashnext(14).unwrap();
        assert_eq!(define.flags().unwrap(), 3);
        assert_eq!(define.builtin().unwrap(), 7);
        assert_eq!(define.numparms().unwrap(), 2);
        assert!(define.fixed().unwrap());
        assert_eq!(define.parms().unwrap(), 11);
        assert_eq!(define.tokens().unwrap(), 12);
        assert_eq!(define.next().unwrap(), 13);
        assert_eq!(define.hashnext().unwrap(), 14);

        // Words persist through the owner handle: clones share the block.
        assert_eq!(heap.define(1).unwrap().flags().unwrap(), 3);
        let second = heap.allocate_define("BAR", true).unwrap();
        assert_eq!(second.id, 2);
    }

    #[test]
    fn define_name_validation() {
        let mut heap = PrecompMemory::default();
        match heap.allocate_define("wide \u{20ac}", true) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "define name must contain source bytes");
            }
            other => panic!("expected name-bytes error, got {other:?}"),
        }
        let mut define = heap.allocate_define("OK", true).unwrap();
        define.allocation.write_u32(0, 9999).unwrap();
        match define.name() {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "define name pointer is outside its allocation");
            }
            other => panic!("expected name-pointer error, got {other:?}"),
        }
        let mut define = heap.allocate_define("TAIL", true).unwrap();
        let len = define.allocation.len();
        for index in SOURCE_DEFINE_BYTES..len {
            define.allocation.set_byte(index, b't').unwrap();
        }
        match define.name() {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "define name has no terminator in its allocation");
            }
            other => panic!("expected name-terminator error, got {other:?}"),
        }
    }

    #[test]
    fn parameter_index_finds_and_misses() {
        let mut heap = PrecompMemory::default();
        let first = heap_token(&mut heap, "alpha").id;
        let second = heap_token(&mut heap, "beta").id;
        let third = heap_token(&mut heap, "gamma").id;
        link_chain(&heap, &[first, second, third]);
        let mut define = heap.allocate_define("MACRO", true).unwrap();
        define.set_parms(first).unwrap();
        assert_eq!(define.parameter_index(&heap, "alpha").unwrap(), 0);
        assert_eq!(define.parameter_index(&heap, "beta").unwrap(), 1);
        assert_eq!(define.parameter_index(&heap, "gamma").unwrap(), 2);
        assert_eq!(define.parameter_index(&heap, "missing").unwrap(), -1);
    }

    #[test]
    fn token_lifecycle() {
        let mut heap = PrecompMemory::default();
        let mut local = local_token();
        local.token.write_string("one").unwrap();
        local.token.set_next(77).unwrap();
        let first = heap.copy_token(&local).unwrap();
        assert_eq!(first.id, 1);
        // Copies reset the link word.
        assert_eq!(first.token.next().unwrap(), 0);
        assert_eq!(first.token.string().unwrap(), "one");
        let second = heap.copy_token(&local).unwrap();
        assert_eq!(second.id, 2);

        match heap.token(99) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "source token pointer does not identify a live token")
            }
            other => panic!("expected unknown-token error, got {other:?}"),
        }

        link_chain(&heap, &[first.id, second.id]);
        let texts: Vec<String> = heap
            .chain(first.id)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .map(|token| token.token.string().unwrap())
            .collect();
        assert_eq!(texts, vec!["one".to_string(), "one".to_string()]);
        assert!(heap.chain(0).next().is_none());

        // Closing the loop trips the cycle guard instead of looping.
        heap.token(second.id).unwrap().token.set_next(first.id).unwrap();
        match heap.chain(first.id).collect::<Result<Vec<_>, _>>() {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "source token chain contains a cycle");
            }
            other => panic!("expected cycle error, got {other:?}"),
        }

        heap.free_token(&first);
        match heap.token(first.id) {
            Err(ClientError::BadUi(_)) => {}
            other => panic!("expected freed-token error, got {other:?}"),
        }
    }

    #[test]
    fn free_tokens_releases_chain() {
        let mut heap = PrecompMemory::default();
        let first = heap_token(&mut heap, "a").id;
        let second = heap_token(&mut heap, "b").id;
        link_chain(&heap, &[first, second]);
        heap.free_tokens(first).unwrap();
        assert!(heap.token(first).is_err());
        assert!(heap.token(second).is_err());
        match heap.free_tokens(42) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "source token pointer does not identify a live token")
            }
            other => panic!("expected unknown-head error, got {other:?}"),
        }
    }

    #[test]
    fn define_lifecycle() {
        let mut heap = PrecompMemory::default();
        let define = heap.allocate_define("LIFE", true).unwrap();
        assert_eq!(heap.define(define.id).unwrap().name().unwrap(), "LIFE");
        match heap.define(99) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "source define pointer does not identify a live definition")
            }
            other => panic!("expected unknown-define error, got {other:?}"),
        }
        let body = heap_token(&mut heap, "x").id;
        let mut owned = heap.define(define.id).unwrap();
        owned.set_tokens(body).unwrap();
        heap.free_define(&owned).unwrap();
        assert!(heap.define(define.id).is_err());
        assert!(heap.token(body).is_err());
    }

    /// Build a source define with two replacement tokens and one parameter.
    fn source_fixture(heap: &mut PrecompMemory) -> PrecompDefine {
        let first = heap_token(heap, "1").id;
        let second = heap_token(heap, "+").id;
        link_chain(heap, &[first, second]);
        let parm = heap_token(heap, "p").id;
        let mut define = heap.allocate_define("ADD", true).unwrap();
        define.set_flags(5).unwrap();
        define.set_builtin(6).unwrap();
        define.set_numparms(1).unwrap();
        define.set_next(41).unwrap();
        define.set_hashnext(42).unwrap();
        define.set_tokens(first).unwrap();
        define.set_parms(parm).unwrap();
        define
    }

    #[test]
    fn copy_define_across_heaps() {
        let mut source = PrecompMemory::default();
        let origin = source_fixture(&mut source);
        let mut dest = PrecompMemory::default();
        let copied = dest.copy_define(&source, origin.id).unwrap();

        assert_eq!(copied.name().unwrap(), "ADD");
        assert_eq!(copied.flags().unwrap(), 5);
        assert_eq!(copied.builtin().unwrap(), 6);
        assert_eq!(copied.numparms().unwrap(), 1);
        assert_eq!(copied.next().unwrap(), 0);
        assert_eq!(copied.hashnext().unwrap(), 0);

        let texts: Vec<String> = dest
            .chain(copied.tokens().unwrap())
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .map(|token| token.token.string().unwrap())
            .collect();
        assert_eq!(texts, vec!["1".to_string(), "+".to_string()]);
        let parms: Vec<String> = dest
            .chain(copied.parms().unwrap())
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .iter()
            .map(|token| token.token.string().unwrap())
            .collect();
        assert_eq!(parms, vec!["p".to_string()]);
        // Chains are deep copies: later writes to the destination chain do
        // not alias the source heap.
        dest.token(copied.tokens().unwrap())
            .unwrap()
            .token
            .write_string("2")
            .unwrap();
        assert_eq!(
            source.token(origin.tokens().unwrap()).unwrap().token.string().unwrap(),
            "1"
        );
        assert_eq!(source.chain(origin.tokens().unwrap()).count(), 2);
    }

    #[test]
    fn duplicate_define_same_heap() {
        let mut heap = PrecompMemory::default();
        let origin = source_fixture(&mut heap);
        let copied = heap.duplicate_define(origin.id).unwrap();
        assert_ne!(copied.id, origin.id);
        assert_eq!(copied.name().unwrap(), "ADD");
        assert_eq!(copied.flags().unwrap(), 5);
        assert_eq!(heap.chain(copied.tokens().unwrap()).count(), 2);
        assert_eq!(heap.chain(copied.parms().unwrap()).count(), 1);
    }

    #[test]
    fn save_round_trip() {
        let mut heap = PrecompMemory::default();
        // Build the local token fully before copying it into the heap, as
        // the donor flows do: context travels with the copy.
        let mut local = local_token();
        local.token.write_string("held").unwrap();
        local.context = SourceTokenContext {
            path: "m.inc".to_string(),
            column: 9,
            leading_whitespace: "\n".to_string(),
        };
        local.unsupported = Some("profile".to_string());
        let held = heap.copy_token(&local).unwrap();
        assert_eq!(held.id, 1);
        let _ = source_fixture(&mut heap);

        let mut capture = FakeCapture::default();
        let state = heap.capture_save_state(&mut capture).unwrap();
        assert_eq!(state.next_token, 5);
        assert_eq!(state.next_define, 2);
        assert_eq!(state.tokens.len(), 4);
        assert_eq!(state.defines.len(), 1);

        let mut revived = PrecompMemory::default();
        let mut restore = FakeRestore { blobs: capture.blobs };
        revived.restore_save_state(&state, &mut restore).unwrap();
        assert_eq!(revived.token(1).unwrap().token.string().unwrap(), "held");
        assert_eq!(revived.token(1).unwrap().context.column, 9);
        assert_eq!(revived.token(1).unwrap().unsupported.as_deref(), Some("profile"));
        assert_eq!(revived.define(1).unwrap().name().unwrap(), "ADD");
        assert_eq!(revived.chain(revived.define(1).unwrap().tokens().unwrap()).count(), 2);
        // Identities keep counting past the restored counters.
        let extra = heap_token(&mut revived, "extra");
        assert_eq!(extra.id, 5);
        let extra_define = revived.allocate_define("EXTRA", true).unwrap();
        assert_eq!(extra_define.id, 2);
    }

    #[test]
    fn save_rejects_bad_state() {
        let mut heap = PrecompMemory::default();
        let _ = heap_token(&mut heap, "a");
        let mut capture = FakeCapture::default();
        let state = heap.capture_save_state(&mut capture).unwrap();
        let blobs = capture.blobs.clone();

        let mut revived = PrecompMemory::default();
        let mut bad = state.clone();
        bad.next_token = 0;
        let mut restore = FakeRestore { blobs: blobs.clone() };
        match revived.restore_save_state(&bad, &mut restore) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "script.heap.nextToken: expected an integer in range")
            }
            other => panic!("expected counter-range error, got {other:?}"),
        }

        let mut bad = state.clone();
        bad.tokens.push(bad.tokens[0].clone());
        let mut restore = FakeRestore { blobs: blobs.clone() };
        match revived.restore_save_state(&bad, &mut restore) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "script.heap.tokens[1]: invalid token identity");
            }
            other => panic!("expected token-identity error, got {other:?}"),
        }

        let mut bad = state.clone();
        bad.tokens[0].id = 99;
        let mut restore = FakeRestore { blobs: blobs.clone() };
        match revived.restore_save_state(&bad, &mut restore) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "script.heap.tokens[0]: invalid token identity");
            }
            other => panic!("expected token-identity error, got {other:?}"),
        }

        let mut short_blobs = blobs.clone();
        short_blobs.push(vec![0; 10]);
        let short_id = (short_blobs.len() - 1) as u32;
        let mut bad = state.clone();
        bad.defines.push(PrecompDefineSaveEntry {
            id: 1,
            allocation: short_id,
        });
        bad.next_define = 2;
        let mut restore = FakeRestore { blobs: short_blobs };
        match revived.restore_save_state(&bad, &mut restore) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "script.heap.defines[0]: invalid define extent");
            }
            other => panic!("expected define-extent error, got {other:?}"),
        }

        let mut bad = state;
        bad.tokens[0].allocation = 99;
        let mut restore = FakeRestore { blobs };
        match revived.restore_save_state(&bad, &mut restore) {
            Err(ClientError::BadUi(message)) => {
                assert_eq!(message, "unknown test allocation 99");
            }
            other => panic!("expected restore error, got {other:?}"),
        }
    }

    /// Heap block owner sharing its log with the test.
    struct SharedMemory {
        log: std::rc::Rc<std::cell::RefCell<FakeMemory>>,
    }

    impl ScriptMemory for SharedMemory {
        fn allocate(&mut self, size: usize) -> ScriptMemoryAllocation {
            self.log.borrow_mut().allocate(size)
        }

        fn free(&mut self, allocation: &ScriptMemoryAllocation) {
            self.log.borrow_mut().free(allocation);
        }
    }

    #[test]
    fn memory_hooks_called() {
        let log = std::rc::Rc::new(std::cell::RefCell::new(FakeMemory::default()));
        let owner = SharedMemory { log: log.clone() };
        let mut heap = PrecompMemory::new(Some(Box::new(owner)));
        let token = heap_token(&mut heap, "owned");
        let define = heap.allocate_define("OWNED", true).unwrap();
        assert_eq!(log.borrow().allocated, vec![SOURCE_TOKEN_BYTES, 32 + 5 + 1]);
        heap.free_token(&token);
        heap.free_define(&define).unwrap();
        assert_eq!(log.borrow().freed, 2);
    }
}
