//! Team Arena UI memory: `UI_Alloc`, string pools, and typed pointers.
//!
//! Donor provenance: `src/ui/common/legacy/team-arena/memory.ts`
//! (translated from id Software's `code/ui/ui_shared.c` `UI_Alloc`,
//! `UI_InitMemory`, `String_Alloc`, and `String_Report`). Fully synchronous
//! port; every donor item is present under Rust case conventions.
//!
//! Assumed sibling imports (donor-derived, committed in this checkout):
//! `super::super::runtime::{UiItemDefinition, UiMenuDefinition,
//! UiModelReference, UiScript, UiShaderReference, UiSoundReference}`.
//! The donor's `gameFormat` call in `report` is inlined with `format!`
//! (`%.1f`/`%i` only); no content-crate dependency.
//!
//! All failures surface as [`ClientError::BadUi`] carrying the donor's exact
//! throw message.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::super::menu::{
    UiItemDefinition, UiMenuDefinition, UiModelReference, UiScript, UiShaderReference, UiSoundReference,
};
use crate::ClientError;

/// Number of static menu records (`64`).
const MENU_COUNT: usize = 64;
/// Bytes per static menu record (`644`).
const MENU_RECORD_BYTES: usize = 644;
/// String hash buckets (`2048`).
const STRING_BUCKET_COUNT: usize = 2048;
/// Main pool bytes for the `ui` module (1 MiB).
const UI_MEMORY_BYTES: usize = 1024 * 1024;
/// String pool bytes for the `ui` module (384 KiB).
const UI_STRING_BYTES: usize = 384 * 1024;
/// Main and string pool bytes for the `cgame` module (128 KiB).
const CGAME_POOL_BYTES: usize = 128 * 1024;
/// Largest accepted `UI_Alloc` size (`0x7fffffff - 15`).
const MAX_ALLOC_SIZE: usize = 0x7fff_ffff - 15;

/// Wrap a donor throw message as [`ClientError::BadUi`].
fn bad_ui(message: &str) -> ClientError {
    ClientError::BadUi(message.to_string())
}

/// A char pointer into retained byte storage (`UiStringReference`).
///
/// `String_Init` does not invalidate or clear it. The byte pool is shared
/// (`Rc`); reads validate overlapping typed pointers like the donor.
#[derive(Debug, Clone)]
pub struct UiStringReference {
    /// Shared byte pool.
    bytes: Rc<RefCell<Vec<u8>>>,
    /// Byte offset of the NUL-terminated string.
    pub offset: usize,
    /// Typed-pointer table of the owning pool, if any.
    pointers: Option<Rc<RefCell<HashMap<usize, UiMemoryPointer>>>>,
}

impl UiStringReference {
    /// Borrow a string at `offset` in shared `bytes` (`constructor`).
    pub(crate) fn new(
        bytes: Rc<RefCell<Vec<u8>>>,
        offset: usize,
        pointers: Option<Rc<RefCell<HashMap<usize, UiMemoryPointer>>>>,
    ) -> Result<Self, ClientError> {
        if offset >= bytes.borrow().len() {
            return Err(bad_ui("UI string pointer is outside its byte storage"));
        }
        Ok(Self {
            bytes,
            offset,
            pointers,
        })
    }

    /// The shared empty string (infallible; `literal("")`).
    fn empty() -> Self {
        Self {
            bytes: Rc::new(RefCell::new(vec![0])),
            offset: 0,
            pointers: None,
        }
    }

    /// Read the NUL-terminated byte string (`read`).
    ///
    /// Bytes map 1:1 onto Unicode scalar values (`String.fromCharCode`).
    pub fn read(&self) -> Result<String, ClientError> {
        let bytes = self.bytes.borrow();
        if self.offset >= bytes.len() {
            return Err(bad_ui("UI string pointer is outside its byte storage"));
        }
        let mut result = String::new();
        for index in self.offset..bytes.len() {
            if let Some(pointers) = &self.pointers {
                let table = pointers.borrow();
                let start = index.saturating_sub(3);
                for word in start..=index {
                    if let Some(pointer) = table.get(&word) {
                        let resolved = matches!(pointer, UiMemoryPointer::Resource { handle: Some(_), .. });
                        if !resolved {
                            return Err(bad_ui("UI string byte read requires QVM pointer address bits"));
                        }
                    }
                }
            }
            let byte = bytes
                .get(index)
                .copied()
                .ok_or_else(|| bad_ui("UI string byte is outside its storage"))?;
            if byte == 0 {
                return Ok(result);
            }
            result.push(byte as char);
        }
        Err(bad_ui("UI string reads beyond its retained byte storage"))
    }

    /// Copy `text` (truncated at NUL) into fresh NUL-terminated storage (`literal`).
    pub fn literal(text: &str) -> Result<Self, ClientError> {
        let end = text.find('\0').unwrap_or(text.len());
        let truncated = &text[..end];
        for ch in truncated.chars() {
            if ch as u32 > 255 {
                return Err(bad_ui("UI string requires source bytes"));
            }
        }
        let mut bytes = Vec::with_capacity(truncated.len() + 1);
        for ch in truncated.chars() {
            bytes.push(ch as u8);
        }
        bytes.push(0);
        Ok(Self {
            bytes: Rc::new(RefCell::new(bytes)),
            offset: 0,
            pointers: None,
        })
    }
}

/// A typed pointer slot (donor-private `UiMemoryPointer`, crate-visible for pool sharing).
///
/// Variants keep donor-faithful inline storage; the table holds few entries
/// and definitions are shared by clone-on-write handles upstream.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum UiMemoryPointer {
    /// String pointer.
    String(UiStringReference),
    /// Nested allocation pointer.
    Allocation(UiMemoryAllocation),
    /// Script pointer.
    Script(UiScript),
    /// Item pointer.
    Item(UiItemDefinition),
    /// Menu pointer.
    Menu(UiMenuDefinition),
    /// Shader/model/sound reference plus numeric handle.
    Resource {
        /// Referenced asset.
        value: UiResourceReference,
        /// Numeric handle (`None` before registration).
        handle: Option<i32>,
    },
}

/// A physical `UI_Alloc` span (`UiMemoryAllocation`).
///
/// Typed pointers deliberately have no fabricated QVM address bits; the
/// numeric bytes stay zero while a typed pointer is installed (except a
/// resolved resource handle). Clones share the pool.
#[derive(Debug, Clone)]
pub struct UiMemoryAllocation {
    /// Shared byte pool.
    bytes: Rc<RefCell<Vec<u8>>>,
    /// Shared typed-pointer table keyed by absolute pool offset.
    pointers: Rc<RefCell<HashMap<usize, UiMemoryPointer>>>,
    /// Absolute base offset in the pool.
    pub offset: usize,
    /// Span length in bytes.
    pub size: usize,
}

impl UiMemoryAllocation {
    /// Borrow `size` bytes at `offset` of a shared pool (`constructor`).
    pub(crate) fn new(
        bytes: Rc<RefCell<Vec<u8>>>,
        pointers: Rc<RefCell<HashMap<usize, UiMemoryPointer>>>,
        offset: usize,
        size: usize,
    ) -> Result<Self, ClientError> {
        let end = offset
            .checked_add(size)
            .ok_or_else(|| bad_ui("UI_Alloc borrow is outside the physical memory pool"))?;
        if end > bytes.borrow().len() {
            return Err(bad_ui("UI_Alloc borrow is outside the physical memory pool"));
        }
        Ok(Self {
            bytes,
            pointers,
            offset,
            size,
        })
    }

    /// Fresh zeroed private pool of `size` bytes (`zeroed`).
    #[must_use]
    pub fn zeroed(size: usize) -> Self {
        Self {
            bytes: Rc::new(RefCell::new(vec![0; size])),
            pointers: Rc::new(RefCell::new(HashMap::new())),
            offset: 0,
            size,
        }
    }

    /// Narrowing view at relative `offset` with `size` bytes (`subrecord`).
    pub fn subrecord(&self, offset: usize, size: usize) -> Result<Self, ClientError> {
        let end = offset
            .checked_add(size)
            .ok_or_else(|| bad_ui("UI record view is outside its retained storage"))?;
        if end > self.size {
            return Err(bad_ui("UI record view is outside its retained storage"));
        }
        Ok(Self {
            bytes: Rc::clone(&self.bytes),
            pointers: Rc::clone(&self.pointers),
            offset: self.offset + offset,
            size,
        })
    }

    /// Reinterpret this span with `size` bytes from its base (`dereference`).
    ///
    /// Bounds check against the pool, not against this span, like the donor.
    pub fn dereference(&self, size: usize) -> Result<Self, ClientError> {
        Self::new(Rc::clone(&self.bytes), Rc::clone(&self.pointers), self.offset, size)
    }

    /// Zero the span and discard overlapping pointers (`clear`).
    pub fn clear(&self) {
        self.discard_pointers(0, self.size);
        let mut bytes = self.bytes.borrow_mut();
        let end = self.offset + self.size;
        if let Some(span) = bytes.get_mut(self.offset..end) {
            span.fill(0);
        }
    }

    /// Read a little-endian `i32` at relative `offset` (`getInt32`).
    pub fn get_int32(&self, offset: usize) -> Result<i32, ClientError> {
        self.check_numeric(offset)?;
        let bytes = self.bytes.borrow();
        let base = self.offset + offset;
        let raw: [u8; 4] = bytes
            .get(base..base + 4)
            .and_then(|span| span.try_into().ok())
            .ok_or_else(|| bad_ui("UI_Alloc field is outside the borrowed record"))?;
        Ok(i32::from_le_bytes(raw))
    }

    /// Read a little-endian `f32` at relative `offset` (`getFloat32`).
    pub fn get_float32(&self, offset: usize) -> Result<f32, ClientError> {
        self.check_numeric(offset)?;
        let bytes = self.bytes.borrow();
        let base = self.offset + offset;
        let raw: [u8; 4] = bytes
            .get(base..base + 4)
            .and_then(|span| span.try_into().ok())
            .ok_or_else(|| bad_ui("UI_Alloc field is outside the borrowed record"))?;
        Ok(f32::from_le_bytes(raw))
    }

    /// Write a little-endian `i32`, discarding overlapping pointers (`setInt32`).
    pub fn set_int32(&self, offset: usize, value: i32) -> Result<(), ClientError> {
        self.check_word(offset)?;
        self.discard_pointers(offset, 4);
        let mut bytes = self.bytes.borrow_mut();
        let base = self.offset + offset;
        let raw = value.to_le_bytes();
        if let Some(span) = bytes.get_mut(base..base + 4) {
            span.copy_from_slice(&raw);
            Ok(())
        } else {
            Err(bad_ui("UI_Alloc field is outside the borrowed record"))
        }
    }

    /// Write a little-endian `f32`, discarding overlapping pointers (`setFloat32`).
    pub fn set_float32(&self, offset: usize, value: f32) -> Result<(), ClientError> {
        self.check_word(offset)?;
        self.discard_pointers(offset, 4);
        let mut bytes = self.bytes.borrow_mut();
        let base = self.offset + offset;
        let raw = value.to_le_bytes();
        if let Some(span) = bytes.get_mut(base..base + 4) {
            span.copy_from_slice(&raw);
            Ok(())
        } else {
            Err(bad_ui("UI_Alloc field is outside the borrowed record"))
        }
    }

    /// Read the string behind the pointer at `offset` (`getString`).
    pub fn get_string(&self, offset: usize) -> Result<Option<String>, ClientError> {
        match self.get_string_reference(offset)? {
            None => Ok(None),
            Some(reference) => Ok(Some(reference.read()?)),
        }
    }

    /// Read the string reference behind the pointer at `offset` (`getStringReference`).
    pub fn get_string_reference(&self, offset: usize) -> Result<Option<UiStringReference>, ClientError> {
        match self.pointer(offset)? {
            None => Ok(None),
            Some(UiMemoryPointer::String(reference)) => Ok(Some(reference)),
            Some(_) => Err(bad_ui("UI_Alloc string read aliases a non-string pointer")),
        }
    }

    /// Install a string pointer: text, reference, or null (`setString`).
    pub fn set_string(&self, offset: usize, value: Option<UiStringValue>) -> Result<(), ClientError> {
        match value {
            None => self.set_pointer(offset, None),
            Some(UiStringValue::Text(text)) => {
                let reference = UiStringReference::literal(&text)?;
                self.set_pointer(offset, Some(UiMemoryPointer::String(reference)))
            }
            Some(UiStringValue::Reference(reference)) => {
                self.set_pointer(offset, Some(UiMemoryPointer::String(reference)))
            }
        }
    }

    /// A string reference rooted at this span's base (`stringReference`).
    pub fn string_reference(&self) -> Result<UiStringReference, ClientError> {
        UiStringReference::new(Rc::clone(&self.bytes), self.offset, Some(Rc::clone(&self.pointers)))
    }

    /// Live view over `count` string-pointer slots from `offset` (`stringArray`).
    pub fn string_array(&self, offset: usize, count: usize) -> Result<UiStringArray, ClientError> {
        for index in 0..count {
            let slot = offset
                .checked_add(
                    index
                        .checked_mul(4)
                        .ok_or_else(|| bad_ui("UI_Alloc field is outside the borrowed record"))?,
                )
                .ok_or_else(|| bad_ui("UI_Alloc field is outside the borrowed record"))?;
            self.check_word(slot)?;
        }
        Ok(UiStringArray {
            allocation: self.clone(),
            offset,
            count,
        })
    }

    /// Copy NUL-truncated `text` plus terminator into this span (`writeString`).
    pub fn write_string(&self, text: &str) -> Result<(), ClientError> {
        let normalized = UiStringReference::literal(text)?.read()?;
        let length = normalized.chars().count();
        if length >= self.size {
            return Err(bad_ui("UI string copy exceeds its allocation"));
        }
        self.discard_pointers(0, length + 1);
        let mut bytes = self.bytes.borrow_mut();
        for (index, ch) in normalized.chars().enumerate() {
            if let Some(slot) = bytes.get_mut(self.offset + index) {
                *slot = ch as u8;
            } else {
                return Err(bad_ui("UI string copy exceeds its allocation"));
            }
        }
        if let Some(slot) = bytes.get_mut(self.offset + length) {
            *slot = 0;
        } else {
            return Err(bad_ui("UI string copy exceeds its allocation"));
        }
        Ok(())
    }

    /// Install a nested allocation pointer (`setAllocationPointer`).
    pub fn set_allocation_pointer(&self, offset: usize, value: Option<UiMemoryAllocation>) -> Result<(), ClientError> {
        self.set_pointer(offset, value.map(UiMemoryPointer::Allocation))
    }

    /// Whether the slot holds no typed pointer and zero bytes (`isNullPointer`).
    pub fn is_null_pointer(&self, offset: usize) -> Result<bool, ClientError> {
        self.check_word(offset)?;
        let pointer = self.pointers.borrow().get(&(self.offset + offset)).cloned();
        if pointer.is_none() || matches!(pointer, Some(UiMemoryPointer::Resource { handle: Some(_), .. })) {
            self.check_numeric(offset)?;
            let bytes = self.bytes.borrow();
            let base = self.offset + offset;
            let raw: [u8; 4] = bytes
                .get(base..base + 4)
                .and_then(|span| span.try_into().ok())
                .ok_or_else(|| bad_ui("UI_Alloc field is outside the borrowed record"))?;
            return Ok(u32::from_le_bytes(raw) == 0);
        }
        Ok(false)
    }

    /// Read the nested allocation pointer at `offset` (`getAllocationPointer`).
    pub fn get_allocation_pointer(&self, offset: usize) -> Result<Option<UiMemoryAllocation>, ClientError> {
        match self.pointer(offset)? {
            None => Ok(None),
            Some(UiMemoryPointer::Allocation(allocation)) => Ok(Some(allocation)),
            Some(_) => Err(bad_ui("UI allocation read aliases a different typed pointer")),
        }
    }

    /// Read the script pointer at `offset` (`getScript`).
    pub fn get_script(&self, offset: usize) -> Result<Option<UiScript>, ClientError> {
        match self.pointer(offset)? {
            None => Ok(None),
            Some(UiMemoryPointer::Script(script)) => Ok(Some(script)),
            Some(_) => Err(bad_ui("UI script read aliases a different typed pointer")),
        }
    }

    /// Install a script pointer (`setScript`).
    pub fn set_script(&self, offset: usize, value: Option<UiScript>) -> Result<(), ClientError> {
        self.set_pointer(offset, value.map(UiMemoryPointer::Script))
    }

    /// Read the item pointer at `offset` (`getItem`).
    pub fn get_item(&self, offset: usize) -> Result<Option<UiItemDefinition>, ClientError> {
        match self.pointer(offset)? {
            None => Ok(None),
            Some(UiMemoryPointer::Item(item)) => Ok(Some(item)),
            Some(_) => Err(bad_ui("UI item read aliases a different typed pointer")),
        }
    }

    /// Install an item pointer (`setItem`).
    pub fn set_item(&self, offset: usize, value: Option<UiItemDefinition>) -> Result<(), ClientError> {
        self.set_pointer(offset, value.map(UiMemoryPointer::Item))
    }

    /// Read the menu pointer at `offset` (`getMenu`).
    pub fn get_menu(&self, offset: usize) -> Result<Option<UiMenuDefinition>, ClientError> {
        match self.pointer(offset)? {
            None => Ok(None),
            Some(UiMemoryPointer::Menu(menu)) => Ok(Some(menu)),
            Some(_) => Err(bad_ui("UI menu read aliases a different typed pointer")),
        }
    }

    /// Install a menu pointer (`setMenu`).
    pub fn set_menu(&self, offset: usize, value: Option<UiMenuDefinition>) -> Result<(), ClientError> {
        self.set_pointer(offset, value.map(UiMemoryPointer::Menu))
    }

    /// Read the asset reference behind the resource slot (`getResource`).
    pub fn get_resource(&self, offset: usize) -> Result<Option<UiResourceReference>, ClientError> {
        self.check_word(offset)?;
        let pointer = self.pointers.borrow().get(&(self.offset + offset)).cloned();
        match pointer {
            Some(UiMemoryPointer::Resource { value, .. }) => Ok(Some(value)),
            None => {
                self.check_numeric(offset)?;
                Ok(None)
            }
            Some(_) => Err(bad_ui("UI resource handle read aliases a different typed pointer")),
        }
    }

    /// Read the numeric handle of the resource slot (`getResourceHandle`).
    ///
    /// Unregistered resources yield `None`; empty slots yield their `i32`
    /// bytes (normally zero) wrapped in `Some`, like the donor.
    pub fn get_resource_handle(&self, offset: usize) -> Result<Option<i32>, ClientError> {
        self.check_word(offset)?;
        let pointer = self.pointers.borrow().get(&(self.offset + offset)).cloned();
        if matches!(pointer, Some(UiMemoryPointer::Resource { handle: None, .. })) {
            return Ok(None);
        }
        Ok(Some(self.get_int32(offset)?))
    }

    /// Install an asset reference plus numeric handle (`setResource`).
    pub fn set_resource(
        &self,
        offset: usize,
        value: UiResourceReference,
        handle: Option<i32>,
    ) -> Result<(), ClientError> {
        self.set_int32(offset, handle.unwrap_or(0))?;
        self.pointers
            .borrow_mut()
            .insert(self.offset + offset, UiMemoryPointer::Resource { value, handle });
        Ok(())
    }

    /// Resolve a typed pointer, rejecting stray nonzero bytes (`pointer`).
    fn pointer(&self, offset: usize) -> Result<Option<UiMemoryPointer>, ClientError> {
        self.check_word(offset)?;
        let pointer = self.pointers.borrow().get(&(self.offset + offset)).cloned();
        if pointer.is_none() || matches!(pointer, Some(UiMemoryPointer::Resource { handle: Some(_), .. })) {
            self.check_numeric(offset)?;
            let bytes = self.bytes.borrow();
            let base = self.offset + offset;
            let raw: [u8; 4] = bytes
                .get(base..base + 4)
                .and_then(|span| span.try_into().ok())
                .ok_or_else(|| bad_ui("UI_Alloc field is outside the borrowed record"))?;
            if u32::from_le_bytes(raw) != 0 {
                return Err(bad_ui("UI pointer read contains non-pointer bytes"));
            }
            return Ok(None);
        }
        Ok(pointer)
    }

    /// Zero the slot bytes, then install or remove a typed pointer (`setPointer`).
    fn set_pointer(&self, offset: usize, pointer: Option<UiMemoryPointer>) -> Result<(), ClientError> {
        self.set_int32(offset, 0)?;
        if let Some(pointer) = pointer {
            self.pointers.borrow_mut().insert(self.offset + offset, pointer);
        }
        Ok(())
    }

    /// Reject a 4-byte field outside this span (`checkWord`).
    fn check_word(&self, offset: usize) -> Result<(), ClientError> {
        let end = offset
            .checked_add(4)
            .ok_or_else(|| bad_ui("UI_Alloc field is outside the borrowed record"))?;
        if end > self.size {
            return Err(bad_ui("UI_Alloc field is outside the borrowed record"));
        }
        Ok(())
    }

    /// Reject numeric access overlapping a typed pointer (`checkNumeric`).
    fn check_numeric(&self, offset: usize) -> Result<(), ClientError> {
        self.check_word(offset)?;
        let table = self.pointers.borrow();
        let base = self.offset + offset;
        let start = base.saturating_sub(3);
        for key in start..base + 4 {
            if let Some(pointer) = table.get(&key) {
                let resolved = matches!(pointer, UiMemoryPointer::Resource { handle: Some(_), .. });
                if !resolved {
                    return Err(bad_ui("UI_Alloc numeric read requires QVM pointer address bits"));
                }
            }
        }
        Ok(())
    }

    /// Drop pointers overlapping `[offset, offset + size)` (`discardPointers`).
    fn discard_pointers(&self, offset: usize, size: usize) {
        if size == 0 {
            return;
        }
        let base = self.offset + offset;
        let start = base.saturating_sub(3);
        let mut table = self.pointers.borrow_mut();
        for key in start..base + size {
            table.remove(&key);
        }
    }
}

/// A `setString` value: owned text or a retained reference.
///
/// Covers the donor's `string | UiStringReference | undefined` parameter
/// (with `None` for null).
#[derive(Debug, Clone)]
pub enum UiStringValue {
    /// Text copied through [`UiStringReference::literal`].
    Text(String),
    /// Retained reference installed directly.
    Reference(UiStringReference),
}

impl From<&str> for UiStringValue {
    /// Copy from a string slice.
    fn from(value: &str) -> Self {
        UiStringValue::Text(value.to_string())
    }
}

impl From<String> for UiStringValue {
    /// Copy from an owned string.
    fn from(value: String) -> Self {
        UiStringValue::Text(value)
    }
}

impl From<UiStringReference> for UiStringValue {
    /// Install a retained reference.
    fn from(value: UiStringReference) -> Self {
        UiStringValue::Reference(value)
    }
}

/// A shader, model, or sound asset reference.
///
/// Covers the donor's inline `UiShaderReference | UiModelReference |
/// UiSoundReference` resource union.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiResourceReference {
    /// Shader (picture) asset.
    Shader(UiShaderReference),
    /// Model asset.
    Model(UiModelReference),
    /// Sound asset.
    Sound(UiSoundReference),
}

/// Live view over consecutive string-pointer slots (`stringArray` result).
///
/// Reads and writes go through the backing allocation, matching the donor's
/// getter/setter array.
#[derive(Debug, Clone)]
pub struct UiStringArray {
    /// Backing allocation (shared pool).
    allocation: UiMemoryAllocation,
    /// Relative byte offset of slot zero.
    offset: usize,
    /// Slot count.
    count: usize,
}

impl UiStringArray {
    /// Slot count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether there are no slots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Relative byte offset of slot zero.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Read slot `index` (`null` becomes `None`).
    pub fn get(&self, index: usize) -> Result<Option<String>, ClientError> {
        if index >= self.count {
            return Err(bad_ui("UI_Alloc field is outside the borrowed record"));
        }
        let slot = self
            .offset
            .checked_add(
                index
                    .checked_mul(4)
                    .ok_or_else(|| bad_ui("UI_Alloc field is outside the borrowed record"))?,
            )
            .ok_or_else(|| bad_ui("UI_Alloc field is outside the borrowed record"))?;
        self.allocation.get_string(slot)
    }

    /// Write slot `index` (`None` clears it).
    pub fn set(&self, index: usize, value: Option<&str>) -> Result<(), ClientError> {
        if index >= self.count {
            return Err(bad_ui("UI_Alloc field is outside the borrowed record"));
        }
        let slot = self
            .offset
            .checked_add(
                index
                    .checked_mul(4)
                    .ok_or_else(|| bad_ui("UI_Alloc field is outside the borrowed record"))?,
            )
            .ok_or_else(|| bad_ui("UI_Alloc field is outside the borrowed record"))?;
        self.allocation
            .set_string(slot, value.map(|text| UiStringValue::Text(text.to_string())))
    }
}

/// UI memory module selecting pool sizes (`"ui" | "cgame"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamArenaUiModule {
    /// Menu VM: 1 MiB main pool, 384 KiB string pool.
    Ui,
    /// Cgame VM: 128 KiB main and string pools.
    Cgame,
}

impl TeamArenaUiModule {
    /// Donor module name.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            TeamArenaUiModule::Ui => "ui",
            TeamArenaUiModule::Cgame => "cgame",
        }
    }
}

impl Default for TeamArenaUiModule {
    /// The donor's default module (`"ui"`).
    fn default() -> Self {
        TeamArenaUiModule::Ui
    }
}

/// Team Arena UI memory pools (`TeamArenaUiMemory`).
///
/// Source QVM layout: `stringDef_t` has two 32-bit pointers (next at `+0`,
/// string at `+4`). Managed heap usage is separate.
pub struct TeamArenaUiMemory {
    /// QVM profile (always `"qvm32"`).
    pub profile: &'static str,
    /// Module selecting pool sizes.
    pub module: TeamArenaUiModule,
    /// Main `UI_Alloc` pool.
    bytes: Rc<RefCell<Vec<u8>>>,
    /// Typed-pointer table for the main pool.
    pointers: Rc<RefCell<HashMap<usize, UiMemoryPointer>>>,
    /// Static menu records (`64 * 644` bytes, private pool).
    menus: UiMemoryAllocation,
    /// String-hash bucket heads (`2048`).
    buckets: Vec<Option<UiMemoryAllocation>>,
    /// String character pool.
    strings: Rc<RefCell<Vec<u8>>>,
    /// Shared empty-string reference.
    empty_string: UiStringReference,
    /// String pool high-water mark.
    string_point: usize,
    /// Main pool high-water mark (16-byte aligned).
    allocation_point: usize,
    /// Set by a failed `UI_Alloc`, cleared by `initialize_memory`.
    exhausted: bool,
    /// Print sink (`Com_Printf`).
    print: Box<dyn FnMut(&str)>,
}

impl std::fmt::Debug for TeamArenaUiMemory {
    /// Debug without pool contents or the print sink.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TeamArenaUiMemory")
            .field("profile", &self.profile)
            .field("module", &self.module)
            .field("string_point", &self.string_point)
            .field("allocation_point", &self.allocation_point)
            .field("exhausted", &self.exhausted)
            .finish_non_exhaustive()
    }
}

impl TeamArenaUiMemory {
    /// Fresh `ui`-module memory (`constructor` with default module).
    pub fn new(print: impl FnMut(&str) + 'static) -> Self {
        Self::with_module(print, TeamArenaUiModule::Ui)
    }

    /// Fresh memory for `module` (`constructor`).
    pub fn with_module(print: impl FnMut(&str) + 'static, module: TeamArenaUiModule) -> Self {
        let (memory_len, string_len) = match module {
            TeamArenaUiModule::Ui => (UI_MEMORY_BYTES, UI_STRING_BYTES),
            TeamArenaUiModule::Cgame => (CGAME_POOL_BYTES, CGAME_POOL_BYTES),
        };
        Self {
            profile: "qvm32",
            module,
            bytes: Rc::new(RefCell::new(vec![0; memory_len])),
            pointers: Rc::new(RefCell::new(HashMap::new())),
            menus: UiMemoryAllocation::zeroed(MENU_COUNT * MENU_RECORD_BYTES),
            buckets: vec![None; STRING_BUCKET_COUNT],
            strings: Rc::new(RefCell::new(vec![0; string_len])),
            empty_string: UiStringReference::empty(),
            string_point: 0,
            allocation_point: 0,
            exhausted: false,
            print: Box::new(print),
        }
    }

    /// String pool bytes used (`stringBytes`).
    #[must_use]
    pub fn string_bytes(&self) -> usize {
        self.string_point
    }

    /// Main pool bytes allocated (`allocatedBytes`).
    #[must_use]
    pub fn allocated_bytes(&self) -> usize {
        self.allocation_point
    }

    /// Whether the last `UI_Alloc` ran out of memory (`outOfMemory`).
    #[must_use]
    pub fn out_of_memory(&self) -> bool {
        self.exhausted
    }

    /// Reset the allocation high-water mark (`initializeMemory`).
    pub fn initialize_memory(&mut self) {
        self.allocation_point = 0;
        self.exhausted = false;
    }

    /// Borrow `size` bytes at absolute `offset` of the main pool (`borrow`).
    pub fn borrow(&self, offset: usize, size: usize) -> Result<UiMemoryAllocation, ClientError> {
        UiMemoryAllocation::new(Rc::clone(&self.bytes), Rc::clone(&self.pointers), offset, size)
    }

    /// The 644-byte static record for menu `index` (`menuRecord`).
    pub fn menu_record(&self, index: usize) -> Result<UiMemoryAllocation, ClientError> {
        if index >= MENU_COUNT {
            return Err(bad_ui("UI menu index exceeds the source static array"));
        }
        self.menus.subrecord(index * MENU_RECORD_BYTES, MENU_RECORD_BYTES)
    }

    /// Print pool usage (`report`, `String_Report`).
    pub fn report(&mut self) {
        (self.print)("Memory/String Pool Info\n");
        (self.print)("----------------\n");
        let strings_len = self.strings.borrow().len();
        let string_percent = (self.string_point as f32) / (strings_len as f32) * 100.0;
        (self.print)(&format!(
            "String Pool is {:.1}% full, {} bytes out of {} used.\n",
            string_percent, self.string_point, strings_len
        ));
        let memory_len = self.bytes.borrow().len();
        let memory_percent = (self.allocation_point as f32) / (memory_len as f32) * 100.0;
        (self.print)(&format!(
            "Memory Pool is {:.1}% full, {} bytes out of {} used.\n",
            memory_percent, self.allocation_point, memory_len
        ));
    }

    /// Storage part of `String_Init` (`initializeStrings`).
    ///
    /// The product controller owns menu and binding resets.
    pub fn initialize_strings(&mut self) {
        self.buckets.fill(None);
        self.string_point = 0;
        self.initialize_memory();
    }

    /// Allocate `size` bytes, 16-byte aligned (`allocate`, `UI_Alloc`).
    ///
    /// Returns `None` (and prints) on exhaustion; errors only on sizes
    /// outside the source range.
    pub fn allocate(&mut self, size: usize) -> Result<Option<usize>, ClientError> {
        if size > MAX_ALLOC_SIZE {
            return Err(bad_ui("UI_Alloc size is outside the source allocation range"));
        }
        let pool_len = self.bytes.borrow().len();
        let overflows = self.allocation_point.checked_add(size).is_none_or(|end| end > pool_len);
        if overflows {
            self.exhausted = true;
            (self.print)("UI_Alloc: Failure. Out of memory!\n");
            return Ok(None);
        }
        let offset = self.allocation_point;
        self.allocation_point += (size + 15) & !15;
        Ok(Some(offset))
    }

    /// Intern `input` and read it back (`stringAlloc`).
    pub fn string_alloc(&mut self, input: Option<&str>) -> Result<Option<String>, ClientError> {
        match self.string_alloc_reference(input)? {
            None => Ok(None),
            Some(reference) => Ok(Some(reference.read()?)),
        }
    }

    /// Intern `input` in the string pool (`stringAllocReference`).
    ///
    /// Hashing folds ASCII case (with the donor's signed-byte weights) but
    /// equality stays exact. Tail linking follows the source, which tracks
    /// the penultimate node and replaces the previous tail on collision
    /// chains longer than one.
    pub fn string_alloc_reference(&mut self, input: Option<&str>) -> Result<Option<UiStringReference>, ClientError> {
        let input = match input {
            None => return Ok(None),
            Some(text) => text,
        };
        let end = input.find('\0').unwrap_or(input.len());
        let text = &input[..end];
        if text.is_empty() {
            return Ok(Some(self.empty_string.clone()));
        }
        let mut hash: i32 = 0;
        for (index, ch) in text.chars().enumerate() {
            let code = ch as u32;
            if code > 255 {
                return Err(bad_ui("String_Alloc requires source byte strings"));
            }
            let mut byte = code as i32;
            if (65..=90).contains(&byte) {
                byte += 32;
            }
            if byte >= 128 {
                byte -= 256;
            }
            let factor = (index as i32).wrapping_add(119);
            hash = hash.wrapping_add(byte.wrapping_mul(factor));
        }
        let bucket = (hash & 2047) as usize;
        if bucket >= self.buckets.len() {
            return Err(bad_ui("String_Alloc hash exceeds source table"));
        }
        let first = self.buckets.get(bucket).cloned().flatten();
        let mut current = first.clone();
        while let Some(node) = current {
            let reference = node
                .get_string_reference(4)?
                .ok_or_else(|| bad_ui("String_Alloc compares a NULL stringDef_t string"))?;
            if reference.read()? == text {
                return Ok(Some(reference));
            }
            current = node.get_allocation_pointer(0)?;
        }
        let text_len = text.chars().count();
        let strings_len = self.strings.borrow().len();
        let total = text_len
            .checked_add(self.string_point)
            .and_then(|sum| sum.checked_add(1));
        if total.is_none_or(|sum| sum >= strings_len) {
            return Ok(None);
        }
        let reference = UiStringReference::new(Rc::clone(&self.strings), self.string_point, None)?;
        {
            let mut pool = self.strings.borrow_mut();
            for (index, ch) in text.chars().enumerate() {
                match pool.get_mut(self.string_point + index) {
                    Some(slot) => *slot = ch as u8,
                    None => return Err(bad_ui("UI string pointer is outside its byte storage")),
                }
            }
            match pool.get_mut(self.string_point + text_len) {
                Some(slot) => *slot = 0,
                None => return Err(bad_ui("UI string pointer is outside its byte storage")),
            }
        }
        self.string_point += text_len + 1;
        let mut last = first.clone();
        let mut tail = first;
        loop {
            let node = match tail.clone() {
                None => break,
                Some(node) => node,
            };
            let next = node.get_allocation_pointer(0)?;
            if next.is_none() {
                break;
            }
            last = tail.clone();
            tail = next;
        }
        let offset = match self.allocate(8)? {
            None => return Err(bad_ui("String_Alloc dereferences a failed UI_Alloc stringDef_t")),
            Some(offset) => offset,
        };
        let allocation = self.borrow(offset, 8)?;
        allocation.set_allocation_pointer(0, None)?;
        allocation.set_string(4, Some(UiStringValue::Reference(reference.clone())))?;
        if let Some(penultimate) = last {
            penultimate.set_allocation_pointer(0, Some(allocation))?;
        } else {
            self.buckets[bucket] = Some(allocation);
        }
        Ok(Some(reference))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::common::legacy::runtime::{
        SourceLocation, UiItemBehavior, UiItemDefinition, UiMenuDefinition, UiModelReference, UiRect, UiScript,
        UiShaderReference, UiSoundReference, UiWindowDefinition,
    };
    use qa_core::math::Vec4;

    fn bad_ui_message(error: ClientError) -> String {
        match error {
            ClientError::BadUi(message) => message,
            other => panic!("expected BadUi, got {other:?}"),
        }
    }

    fn capture_memory(module: TeamArenaUiModule) -> (TeamArenaUiMemory, Rc<RefCell<Vec<String>>>) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&log);
        let memory = TeamArenaUiMemory::with_module(move |text: &str| sink.borrow_mut().push(text.to_string()), module);
        (memory, log)
    }

    fn test_window() -> UiWindowDefinition {
        let color = Vec4 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        };
        UiWindowDefinition {
            rect: UiRect::default(),
            client_rect: UiRect::default(),
            rect_effects: UiRect::default(),
            rect_effects2: UiRect::default(),
            name: None,
            group: None,
            cinematic: None,
            style: 0,
            border: 0,
            owner_draw: 0,
            owner_draw_flags: 0,
            border_size: 0.0,
            flags: 0,
            next_time: 0,
            offset_time: 0,
            cinematic_handle: -1,
            fore_color: color,
            back_color: color,
            border_color: color,
            outline_color: color,
            background: None,
            background_handle: None,
        }
    }

    fn test_location() -> SourceLocation {
        SourceLocation {
            path: "test.menu".to_string(),
            line: 1,
            column: 1,
        }
    }

    fn test_item(text: &str) -> UiItemDefinition {
        UiItemDefinition {
            location: test_location(),
            allocation_offset: None,
            window: test_window(),
            type_code: 1,
            parent: None,
            text_rect: UiRect::default(),
            behavior: UiItemBehavior::Button,
            alignment: 0,
            text_alignment: 0,
            text_align_x: 0.0,
            text_align_y: 0.0,
            text_scale: 1.0,
            text_style: 0,
            text: Some(text.to_string()),
            asset: None,
            asset_handle: None,
            mouse_enter_text: None,
            mouse_exit_text: None,
            mouse_enter: None,
            mouse_exit: None,
            action: None,
            on_focus: None,
            leave_focus: None,
            cvar: None,
            cvar_test: None,
            cvar_rule: None,
            cvar_flags: 0,
            cvar_script: None,
            focus_sound: None,
            focus_sound_handle: None,
            color_ranges: Vec::new(),
            special: 0.0,
            cursor_position: 0,
        }
    }

    fn test_menu() -> UiMenuDefinition {
        let color = Vec4 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
            w: 1.0,
        };
        UiMenuDefinition {
            location: test_location(),
            source_index: 0,
            window: test_window(),
            font: None,
            full_screen: 1,
            cursor_item: -1,
            font_index: 0,
            fade_cycle: 0,
            fade_clamp: 0.0,
            fade_amount: 0.0,
            on_open: None,
            on_close: None,
            on_escape: None,
            sound_loop: None,
            focus_color: color,
            disable_color: color,
            items: Vec::new(),
        }
    }

    #[test]
    fn string_literal_round_trips_and_truncates_at_nul() {
        assert_eq!(UiStringReference::literal("hello").unwrap().read().unwrap(), "hello");
        assert_eq!(UiStringReference::literal("").unwrap().read().unwrap(), "");
        assert_eq!(UiStringReference::literal("ab\0cd").unwrap().read().unwrap(), "ab");
        assert_eq!(
            UiStringReference::literal("caf\u{e9}").unwrap().read().unwrap(),
            "caf\u{e9}"
        );
    }

    #[test]
    fn string_literal_rejects_non_byte_chars() {
        let error = UiStringReference::literal("caf\u{20ac}").unwrap_err();
        assert_eq!(bad_ui_message(error), "UI string requires source bytes");
    }

    #[test]
    fn string_read_requires_nul_terminator() {
        let allocation = UiMemoryAllocation::zeroed(4);
        allocation.set_int32(0, 0x4443_4241).unwrap();
        let error = allocation.string_reference().unwrap().read().unwrap_err();
        assert_eq!(
            bad_ui_message(error),
            "UI string reads beyond its retained byte storage"
        );
    }

    #[test]
    fn string_reference_rejects_bad_offset() {
        let bytes = Rc::new(RefCell::new(vec![0, 0]));
        let error = UiStringReference::new(bytes, 2, None).unwrap_err();
        assert_eq!(bad_ui_message(error), "UI string pointer is outside its byte storage");
    }

    #[test]
    fn string_read_rejects_pointer_overlap_but_allows_resolved_handles() {
        let allocation = UiMemoryAllocation::zeroed(8);
        allocation.set_string(0, Some("hi".into())).unwrap();
        let error = allocation.string_reference().unwrap().read().unwrap_err();
        assert_eq!(
            bad_ui_message(error),
            "UI string byte read requires QVM pointer address bits"
        );

        let resolved = UiMemoryAllocation::zeroed(8);
        resolved
            .set_resource(
                0,
                UiResourceReference::Shader(UiShaderReference {
                    path: Some("pics/top".to_string()),
                }),
                Some(7),
            )
            .unwrap();
        assert_eq!(resolved.string_reference().unwrap().read().unwrap(), "\u{7}");

        let pending = UiMemoryAllocation::zeroed(8);
        pending
            .set_resource(0, UiResourceReference::Sound(UiSoundReference { path: None }), None)
            .unwrap();
        let error = pending.string_reference().unwrap().read().unwrap_err();
        assert_eq!(
            bad_ui_message(error),
            "UI string byte read requires QVM pointer address bits"
        );
    }

    #[test]
    fn allocation_subrecord_and_dereference_use_exact_offsets() {
        let (memory, _) = capture_memory(TeamArenaUiModule::Ui);
        let span = memory.borrow(100, 32).unwrap();
        assert_eq!(span.offset, 100);
        assert_eq!(span.size, 32);
        let inner = span.subrecord(8, 16).unwrap();
        assert_eq!(inner.offset, 108);
        assert_eq!(inner.size, 16);
        let grown = span.dereference(64).unwrap();
        assert_eq!(grown.offset, 100);
        assert_eq!(grown.size, 64);

        let error = span.subrecord(30, 4).unwrap_err();
        assert_eq!(bad_ui_message(error), "UI record view is outside its retained storage");
        let error = memory.borrow(1024 * 1024 - 4, 8).unwrap_err();
        assert_eq!(
            bad_ui_message(error),
            "UI_Alloc borrow is outside the physical memory pool"
        );
        let error = span.dereference(1024 * 1024).unwrap_err();
        assert_eq!(
            bad_ui_message(error),
            "UI_Alloc borrow is outside the physical memory pool"
        );
    }

    #[test]
    fn allocation_clear_zeroes_and_discards_pointers() {
        let allocation = UiMemoryAllocation::zeroed(8);
        allocation.set_int32(0, 0x1234_5678).unwrap();
        allocation.set_string(4, Some("x".into())).unwrap();
        allocation.clear();
        assert_eq!(allocation.get_int32(0).unwrap(), 0);
        assert!(allocation.get_string(4).unwrap().is_none());
        assert!(allocation.is_null_pointer(0).unwrap());
    }

    #[test]
    fn int_and_float_round_trip_with_word_errors() {
        let allocation = UiMemoryAllocation::zeroed(8);
        allocation.set_int32(0, -42).unwrap();
        allocation.set_float32(4, 1.5).unwrap();
        assert_eq!(allocation.get_int32(0).unwrap(), -42);
        assert_eq!(allocation.get_float32(4).unwrap(), 1.5);

        let error = allocation.get_int32(5).unwrap_err();
        assert_eq!(bad_ui_message(error), "UI_Alloc field is outside the borrowed record");
        let error = allocation.set_float32(8, 0.0).unwrap_err();
        assert_eq!(bad_ui_message(error), "UI_Alloc field is outside the borrowed record");
    }

    #[test]
    fn numeric_read_rejects_pointer_overlap() {
        let allocation = UiMemoryAllocation::zeroed(8);
        allocation.set_string(0, Some("hi".into())).unwrap();
        let error = allocation.get_int32(0).unwrap_err();
        assert_eq!(
            bad_ui_message(error),
            "UI_Alloc numeric read requires QVM pointer address bits"
        );
        allocation.set_int32(0, 11).unwrap();
        assert_eq!(allocation.get_int32(0).unwrap(), 11);
    }

    #[test]
    fn string_get_set_round_trip_and_alias_error() {
        let allocation = UiMemoryAllocation::zeroed(8);
        assert!(allocation.get_string(0).unwrap().is_none());
        allocation.set_string(0, Some("hello".into())).unwrap();
        assert_eq!(allocation.get_string(0).unwrap().as_deref(), Some("hello"));
        let reference = UiStringReference::literal("kept").unwrap();
        allocation
            .set_string(0, Some(UiStringValue::Reference(reference)))
            .unwrap();
        assert_eq!(allocation.get_string(0).unwrap().as_deref(), Some("kept"));
        allocation.set_string(0, None).unwrap();
        assert!(allocation.get_string(0).unwrap().is_none());

        allocation.set_script(0, Some(UiScript::from_text("quit"))).unwrap();
        let error = allocation.get_string_reference(0).unwrap_err();
        assert_eq!(
            bad_ui_message(error),
            "UI_Alloc string read aliases a non-string pointer"
        );
    }

    #[test]
    fn write_string_copies_with_nul_and_rejects_overflow() {
        let allocation = UiMemoryAllocation::zeroed(8);
        allocation.set_string(0, Some("stale".into())).unwrap();
        allocation.write_string("abc").unwrap();
        assert_eq!(allocation.get_int32(4).unwrap(), 0);
        assert_eq!(allocation.string_reference().unwrap().read().unwrap(), "abc");
        let error = allocation.write_string("12345678").unwrap_err();
        assert_eq!(bad_ui_message(error), "UI string copy exceeds its allocation");
    }

    #[test]
    fn allocation_pointer_null_check_and_stray_bytes() {
        let allocation = UiMemoryAllocation::zeroed(8);
        assert!(allocation.is_null_pointer(0).unwrap());
        assert!(allocation.get_allocation_pointer(0).unwrap().is_none());

        let child = UiMemoryAllocation::zeroed(4);
        allocation.set_allocation_pointer(0, Some(child)).unwrap();
        assert!(!allocation.is_null_pointer(0).unwrap());
        assert!(allocation.get_allocation_pointer(0).unwrap().is_some());

        allocation.set_script(0, Some(UiScript::from_text("x"))).unwrap();
        let error = allocation.get_allocation_pointer(0).unwrap_err();
        assert_eq!(
            bad_ui_message(error),
            "UI allocation read aliases a different typed pointer"
        );

        let stray = UiMemoryAllocation::zeroed(8);
        stray.set_int32(0, 5).unwrap();
        let error = stray.get_allocation_pointer(0).unwrap_err();
        assert_eq!(bad_ui_message(error), "UI pointer read contains non-pointer bytes");
    }

    #[test]
    fn script_item_menu_pointers_round_trip_with_alias_errors() {
        let allocation = UiMemoryAllocation::zeroed(12);
        let script = UiScript::from_text("uiScript start");
        allocation.set_script(0, Some(script.clone())).unwrap();
        assert_eq!(allocation.get_script(0).unwrap(), Some(script));
        allocation.set_script(0, None).unwrap();
        assert!(allocation.get_script(0).unwrap().is_none());

        let item = test_item("ok");
        allocation.set_item(4, Some(item.clone())).unwrap();
        assert_eq!(allocation.get_item(4).unwrap(), Some(item));

        let menu = test_menu();
        allocation.set_menu(8, Some(menu.clone())).unwrap();
        assert_eq!(allocation.get_menu(8).unwrap(), Some(menu));

        let error = allocation.get_script(4).unwrap_err();
        assert_eq!(
            bad_ui_message(error),
            "UI script read aliases a different typed pointer"
        );
        let error = allocation.get_item(8).unwrap_err();
        assert_eq!(bad_ui_message(error), "UI item read aliases a different typed pointer");
        let error = allocation.get_menu(4).unwrap_err();
        assert_eq!(bad_ui_message(error), "UI menu read aliases a different typed pointer");
    }

    #[test]
    fn resource_slots_round_trip_with_handle_states() {
        let allocation = UiMemoryAllocation::zeroed(12);
        assert!(allocation.get_resource(0).unwrap().is_none());
        assert_eq!(allocation.get_resource_handle(0).unwrap(), Some(0));

        let shader = UiResourceReference::Shader(UiShaderReference {
            path: Some("pics/top".to_string()),
        });
        allocation.set_resource(0, shader.clone(), Some(3)).unwrap();
        assert_eq!(allocation.get_resource(0).unwrap(), Some(shader));
        assert_eq!(allocation.get_resource_handle(0).unwrap(), Some(3));
        assert!(!allocation.is_null_pointer(0).unwrap());

        let model = UiResourceReference::Model(UiModelReference { path: None });
        allocation.set_resource(4, model.clone(), None).unwrap();
        assert_eq!(allocation.get_resource(4).unwrap(), Some(model));
        assert_eq!(allocation.get_resource_handle(4).unwrap(), None);

        allocation.set_string(8, Some("s".into())).unwrap();
        let error = allocation.get_resource(8).unwrap_err();
        assert_eq!(
            bad_ui_message(error),
            "UI resource handle read aliases a different typed pointer"
        );
    }

    #[test]
    fn string_array_is_a_live_slot_view() {
        let allocation = UiMemoryAllocation::zeroed(12);
        let array = allocation.string_array(0, 3).unwrap();
        assert_eq!(array.len(), 3);
        assert_eq!(array.offset(), 0);
        assert!(!array.is_empty());
        assert_eq!(array.get(0).unwrap(), None);
        array.set(1, Some("mid")).unwrap();
        assert_eq!(array.get(1).unwrap().as_deref(), Some("mid"));
        assert_eq!(allocation.get_string(4).unwrap().as_deref(), Some("mid"));
        allocation.set_string(8, Some("tail".into())).unwrap();
        assert_eq!(array.get(2).unwrap().as_deref(), Some("tail"));
        array.set(1, None).unwrap();
        assert_eq!(array.get(1).unwrap(), None);

        let error = array.get(3).unwrap_err();
        assert_eq!(bad_ui_message(error), "UI_Alloc field is outside the borrowed record");
        let error = allocation.string_array(8, 2).unwrap_err();
        assert_eq!(bad_ui_message(error), "UI_Alloc field is outside the borrowed record");
        assert!(allocation.string_array(0, 0).unwrap().is_empty());
    }

    #[test]
    fn menu_records_use_exact_static_offsets() {
        let (memory, _) = capture_memory(TeamArenaUiModule::Ui);
        let first = memory.menu_record(0).unwrap();
        assert_eq!(first.offset, 0);
        assert_eq!(first.size, 644);
        let last = memory.menu_record(63).unwrap();
        assert_eq!(last.offset, 63 * 644);
        assert_eq!(last.size, 644);
        assert_eq!(63 * 644, 40_572);
        let error = memory.menu_record(64).unwrap_err();
        assert_eq!(bad_ui_message(error), "UI menu index exceeds the source static array");
    }

    #[test]
    fn report_prints_exact_pool_lines() {
        let (mut memory, log) = capture_memory(TeamArenaUiModule::Ui);
        assert_eq!(memory.profile, "qvm32");
        memory.report();
        assert_eq!(
            log.borrow().as_slice(),
            [
                "Memory/String Pool Info\n",
                "----------------\n",
                "String Pool is 0.0% full, 0 bytes out of 393216 used.\n",
                "Memory Pool is 0.0% full, 0 bytes out of 1048576 used.\n",
            ]
        );

        let (mut cgame, cgame_log) = capture_memory(TeamArenaUiModule::Cgame);
        cgame.report();
        assert_eq!(
            cgame_log.borrow().as_slice(),
            [
                "Memory/String Pool Info\n",
                "----------------\n",
                "String Pool is 0.0% full, 0 bytes out of 131072 used.\n",
                "Memory Pool is 0.0% full, 0 bytes out of 131072 used.\n",
            ]
        );

        memory.string_alloc(Some("test")).unwrap();
        memory.allocate(10_486).unwrap();
        memory.report();
        let lines = log.borrow();
        assert_eq!(lines.len(), 8);
        assert_eq!(lines[6], "String Pool is 0.0% full, 5 bytes out of 393216 used.\n");
        assert_eq!(lines[7], "Memory Pool is 1.0% full, 10512 bytes out of 1048576 used.\n");
    }

    #[test]
    fn allocate_aligns_and_reports_out_of_memory() {
        let (mut memory, log) = capture_memory(TeamArenaUiModule::Ui);
        assert_eq!(memory.allocate(1).unwrap(), Some(0));
        assert_eq!(memory.allocated_bytes(), 16);
        assert_eq!(memory.allocate(16).unwrap(), Some(16));
        assert_eq!(memory.allocate(17).unwrap(), Some(32));
        assert_eq!(memory.allocated_bytes(), 64);
        assert!(!memory.out_of_memory());

        let error = memory.allocate(0x7fff_ffff - 14).unwrap_err();
        assert_eq!(
            bad_ui_message(error),
            "UI_Alloc size is outside the source allocation range"
        );

        memory.allocation_point = 1024 * 1024 - 4;
        assert_eq!(memory.allocate(8).unwrap(), None);
        assert!(memory.out_of_memory());
        assert_eq!(
            log.borrow().last().map(String::as_str),
            Some("UI_Alloc: Failure. Out of memory!\n")
        );
        memory.initialize_memory();
        assert!(!memory.out_of_memory());
        assert_eq!(memory.allocated_bytes(), 0);
    }

    #[test]
    fn initialize_strings_resets_buckets_and_points() {
        let (mut memory, _) = capture_memory(TeamArenaUiModule::Ui);
        memory.string_alloc(Some("persist")).unwrap();
        assert!(memory.string_bytes() > 0);
        assert!(memory.allocated_bytes() > 0);
        memory.initialize_strings();
        assert_eq!(memory.string_bytes(), 0);
        assert_eq!(memory.allocated_bytes(), 0);
        assert!(!memory.out_of_memory());
        assert!(memory.buckets.iter().all(Option::is_none));
        assert_eq!(
            memory.string_alloc(Some("persist")).unwrap().as_deref(),
            Some("persist")
        );
    }

    #[test]
    fn string_alloc_interns_dedups_and_hashes_case_insensitively() {
        let (mut memory, _) = capture_memory(TeamArenaUiModule::Ui);
        assert_eq!(memory.string_alloc(None).unwrap(), None);
        assert_eq!(memory.string_alloc(Some("")).unwrap().as_deref(), Some(""));
        assert_eq!(memory.string_alloc(Some("a\0b")).unwrap().as_deref(), Some("a"));

        let first = memory.string_alloc_reference(Some("test")).unwrap().unwrap();
        assert_eq!(first.offset, 2);
        assert_eq!(memory.string_bytes(), 7);
        let again = memory.string_alloc_reference(Some("test")).unwrap().unwrap();
        assert_eq!(again.offset, first.offset);
        assert_eq!(memory.string_bytes(), 7);

        let upper = memory.string_alloc_reference(Some("TEST")).unwrap().unwrap();
        assert_ne!(upper.offset, first.offset);
        assert_eq!(memory.string_alloc(Some("TEST")).unwrap().as_deref(), Some("TEST"));

        let head = memory.buckets[743].clone().expect("bucket 743 holds test");
        assert_eq!(head.get_string(4).unwrap().as_deref(), Some("test"));
        let next = head.get_allocation_pointer(0).unwrap().expect("chain holds TEST");
        assert_eq!(next.offset, 32);
        assert_eq!(next.get_string(4).unwrap().as_deref(), Some("TEST"));
        assert!(next.get_allocation_pointer(0).unwrap().is_none());
    }

    #[test]
    fn string_alloc_third_collision_replaces_tail_like_source() {
        let (mut memory, _) = capture_memory(TeamArenaUiModule::Ui);
        memory.string_alloc(Some("test")).unwrap();
        memory.string_alloc(Some("TEST")).unwrap();
        memory.string_alloc(Some("Test")).unwrap();
        let head = memory.buckets[743].clone().expect("bucket 743 holds test");
        let next = head.get_allocation_pointer(0).unwrap().expect("chain tail");
        assert_eq!(next.get_string(4).unwrap().as_deref(), Some("Test"));
        assert_eq!(memory.string_alloc(Some("test")).unwrap().as_deref(), Some("test"));
        assert_eq!(memory.string_alloc(Some("Test")).unwrap().as_deref(), Some("Test"));
    }

    #[test]
    fn string_alloc_rejects_non_byte_chars() {
        let (mut memory, _) = capture_memory(TeamArenaUiModule::Ui);
        let error = memory.string_alloc(Some("bad\u{2019}")).unwrap_err();
        assert_eq!(bad_ui_message(error), "String_Alloc requires source byte strings");
    }

    #[test]
    fn string_alloc_reports_null_stringdef_and_pool_outcomes() {
        let (mut memory, _) = capture_memory(TeamArenaUiModule::Ui);
        let bad = memory.borrow(0, 8).unwrap();
        for bucket in memory.buckets.iter_mut() {
            *bucket = Some(bad.clone());
        }
        let error = memory.string_alloc_reference(Some("test")).unwrap_err();
        assert_eq!(bad_ui_message(error), "String_Alloc compares a NULL stringDef_t string");

        let (mut starved, _) = capture_memory(TeamArenaUiModule::Ui);
        starved.string_point = 384 * 1024 - 2;
        assert_eq!(starved.string_alloc(Some("toolong")).unwrap(), None);

        let (mut exhausted, _) = capture_memory(TeamArenaUiModule::Ui);
        exhausted.allocation_point = 1024 * 1024 - 4;
        let error = exhausted.string_alloc_reference(Some("fresh")).unwrap_err();
        assert_eq!(
            bad_ui_message(error),
            "String_Alloc dereferences a failed UI_Alloc stringDef_t"
        );
    }
}
