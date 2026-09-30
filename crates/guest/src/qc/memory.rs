//! Port of `src/compat/qc/memory.ts` (source-owned QC word storage and
//! byte-addressed edicts, GPL-2.0-or-later): [`QcWords`] word buffers,
//! [`QcEntityMemory`] edict storage, and the [`QcStrings`] byte arena.
//!
//! The donor shares one `Uint8Array` between overlapping `QcWords` views;
//! Rust aliasing rules forbid that, so entity words are read through
//! methods on [`QcEntityMemory`] plus short-lived byte slices instead of
//! persistent view objects. Semantics (offsets, validation, checkpoint
//! bytes) match the donor exactly.

use qa_core::binary::{BinaryReader, BinaryWriter};
use qa_core::math::{vec3, Vec3};

use super::program::qc_byte_string;
use crate::error::GuestError;

/// Magic heading a [`QcStrings`] snapshot (`QCS1` little-endian).
pub const QC_STRINGS_MAGIC: u32 = 0x3153_4351;

/// Word-addressed QC storage over an owned little-endian byte buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcWords {
    bytes: Vec<u8>,
}

impl QcWords {
    /// Wrap `bytes`; the length must be a whole number of words.
    pub fn new(bytes: Vec<u8>) -> Result<Self, GuestError> {
        if !bytes.len().is_multiple_of(4) {
            return Err(GuestError::invalid("QC storage must contain whole words"));
        }
        Ok(Self { bytes })
    }

    /// Zeroed storage of `words` words.
    #[must_use]
    pub fn zeroed(words: usize) -> Self {
        Self {
            bytes: vec![0; words * 4],
        }
    }

    /// Word count.
    #[must_use]
    pub fn len_words(&self) -> usize {
        self.bytes.len() / 4
    }

    /// Whether the storage holds no words.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Raw little-endian bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Mutable raw bytes.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    fn check(&self, word: usize, count: usize) -> Result<usize, GuestError> {
        let fault = || {
            GuestError::memory_fault(
                "word-range",
                (word as u64).saturating_mul(4),
                count.saturating_mul(4),
                "read",
                format!("word {word} outside {}-word storage", self.len_words()),
            )
        };
        let end = word.checked_add(count).ok_or_else(fault)?;
        if end > self.len_words() {
            return Err(fault());
        }
        Ok(word * 4)
    }

    /// Read a signed word.
    #[inline]
    pub fn int(&self, word: usize) -> Result<i32, GuestError> {
        let at = self.check(word, 1)?;
        Ok(i32::from_le_bytes([
            self.bytes[at],
            self.bytes[at + 1],
            self.bytes[at + 2],
            self.bytes[at + 3],
        ]))
    }

    /// Read a float word.
    #[inline]
    pub fn float(&self, word: usize) -> Result<f32, GuestError> {
        let at = self.check(word, 1)?;
        Ok(f32::from_le_bytes([
            self.bytes[at],
            self.bytes[at + 1],
            self.bytes[at + 2],
            self.bytes[at + 3],
        ]))
    }

    /// Write a signed word.
    #[inline]
    pub fn set_int(&mut self, word: usize, value: i32) -> Result<(), GuestError> {
        let at = self.check(word, 1)?;
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write a float word.
    #[inline]
    pub fn set_float(&mut self, word: usize, value: f32) -> Result<(), GuestError> {
        self.set_int(word, value.to_bits() as i32)
    }

    /// Read a three-word vector.
    pub fn vector(&self, word: usize) -> Result<Vec3, GuestError> {
        Ok(vec3(self.float(word)?, self.float(word + 1)?, self.float(word + 2)?))
    }

    /// Write a three-word vector.
    pub fn set_vector(&mut self, word: usize, value: Vec3) -> Result<(), GuestError> {
        self.set_float(word, value.x)?;
        self.set_float(word + 1, value.y)?;
        self.set_float(word + 2, value.z)
    }

    /// Copy `count` words forward within this buffer, matching the
    /// interpreter's sequential stores including deliberate overlap.
    pub fn copy_within(&mut self, source: usize, destination: usize, count: usize) -> Result<(), GuestError> {
        self.check(source, count)?;
        self.check(destination, count)?;
        for index in 0..count {
            let value = self.int(source + index)?;
            self.set_int(destination + index, value)?;
        }
        Ok(())
    }

    /// Copy `count` words from another buffer, sequentially like the donor.
    pub fn copy_from_words(
        &mut self,
        from: &QcWords,
        source: usize,
        destination: usize,
        count: usize,
    ) -> Result<(), GuestError> {
        for index in 0..count {
            let value = from.int(source + index)?;
            self.set_int(destination + index, value)?;
        }
        Ok(())
    }

    /// Copy `count` words from a raw little-endian byte slice.
    pub fn copy_from_bytes(
        &mut self,
        from: &[u8],
        source_word: usize,
        destination: usize,
        count: usize,
    ) -> Result<(), GuestError> {
        for index in 0..count {
            let at = source_word
                .checked_add(index)
                .and_then(|word| word.checked_mul(4))
                .filter(|at| at + 4 <= from.len())
                .ok_or_else(|| {
                    GuestError::memory_fault(
                        "word-range",
                        (source_word as u64).saturating_mul(4),
                        4,
                        "read",
                        "entity word outside variable storage",
                    )
                })?;
            let value = i32::from_le_bytes([from[at], from[at + 1], from[at + 2], from[at + 3]]);
            self.set_int(destination + index, value)?;
        }
        Ok(())
    }
}

/// Edict record geometry: fixed prefix plus trailing QC variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QcEntityLayout {
    /// Full record length in bytes.
    pub stride_bytes: usize,
    /// Byte offset of the QC variable words within a record.
    pub variables_offset_bytes: usize,
    /// QC variable word count.
    pub field_words: usize,
}

/// Byte-addressed edict storage: `capacity` stride-sized records with a
/// live prefix of `count` slots.
#[derive(Debug, Clone)]
pub struct QcEntityMemory {
    layout: QcEntityLayout,
    bytes: Vec<u8>,
    capacity: usize,
    count: usize,
}

impl QcEntityMemory {
    /// Allocate zeroed storage for `capacity` slots with `count` live.
    pub fn new(layout: QcEntityLayout, capacity: usize, count: usize) -> Result<Self, GuestError> {
        if capacity < 1
            || count < 1
            || count > capacity
            || layout.stride_bytes == 0
            || !layout.stride_bytes.is_multiple_of(4)
            || !layout.variables_offset_bytes.is_multiple_of(4)
            || layout.field_words == 0
            || layout.variables_offset_bytes + layout.field_words * 4 > layout.stride_bytes
            || (capacity as u128) * (layout.stride_bytes as u128) > 0x7fff_ffff
        {
            return Err(GuestError::invalid("Invalid QC entity layout/capacity"));
        }
        Ok(Self {
            bytes: vec![0; capacity * layout.stride_bytes],
            layout,
            capacity,
            count,
        })
    }

    /// Record geometry.
    #[must_use]
    pub const fn layout(&self) -> QcEntityLayout {
        self.layout
    }

    /// Allocated slot capacity.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Live slot count.
    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }

    /// Raw record bytes (all slots, including prefixes).
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Resize the live prefix.
    pub fn set_count(&mut self, count: usize) -> Result<(), GuestError> {
        if count < 1 || count > self.capacity {
            return Err(GuestError::invalid("Invalid QC entity count"));
        }
        self.count = count;
        Ok(())
    }

    /// Byte reference of a live slot.
    pub fn reference(&self, slot: u32) -> Result<i32, GuestError> {
        self.check_slot(slot)?;
        Ok(slot as i32 * self.layout.stride_bytes as i32)
    }

    /// Slot addressed by a byte reference.
    pub fn slot(&self, reference: i32) -> Result<u32, GuestError> {
        if reference < 0 || !(reference as usize).is_multiple_of(self.layout.stride_bytes) {
            return Err(self.entity_error(format!("invalid entity reference {reference}")));
        }
        let slot = reference as usize / self.layout.stride_bytes;
        let Ok(slot) = u32::try_from(slot) else {
            return Err(self.entity_error(format!("invalid entity reference {reference}")));
        };
        self.check_slot(slot)?;
        Ok(slot)
    }

    fn check_slot(&self, slot: u32) -> Result<(), GuestError> {
        if slot as usize >= self.count {
            return Err(self.entity_error(format!("invalid entity slot {slot}")));
        }
        Ok(())
    }

    fn entity_error(&self, detail: String) -> GuestError {
        GuestError::memory_fault("entity-range", 0, 0, "read", detail)
    }

    fn field_range(&self, slot: u32) -> Result<(usize, usize), GuestError> {
        self.check_slot(slot)?;
        let start = slot as usize * self.layout.stride_bytes + self.layout.variables_offset_bytes;
        Ok((start, start + self.layout.field_words * 4))
    }

    /// Variable bytes of a live slot.
    pub fn field_bytes(&self, slot: u32) -> Result<&[u8], GuestError> {
        let (start, end) = self.field_range(slot)?;
        Ok(&self.bytes[start..end])
    }

    /// Mutable variable bytes of a live slot.
    pub fn field_bytes_mut(&mut self, slot: u32) -> Result<&mut [u8], GuestError> {
        let (start, end) = self.field_range(slot)?;
        Ok(&mut self.bytes[start..end])
    }

    /// Complete record bytes (prefix plus variables) of a live slot.
    pub fn record_bytes(&self, slot: u32) -> Result<&[u8], GuestError> {
        self.check_slot(slot)?;
        let start = slot as usize * self.layout.stride_bytes;
        Ok(&self.bytes[start..start + self.layout.stride_bytes])
    }

    /// Zero the variable words of a slot.
    pub fn clear_slot(&mut self, slot: u32) -> Result<(), GuestError> {
        self.field_bytes_mut(slot)?.fill(0);
        Ok(())
    }

    /// Read a variable word of a slot.
    pub fn slot_int(&self, slot: u32, word: usize) -> Result<i32, GuestError> {
        let fields = self.field_bytes(slot)?;
        let at = word.checked_mul(4).filter(|at| at + 4 <= fields.len()).ok_or_else(|| {
            GuestError::memory_fault(
                "word-range",
                (word as u64).saturating_mul(4),
                4,
                "read",
                "entity word outside variable storage",
            )
        })?;
        Ok(i32::from_le_bytes([
            fields[at],
            fields[at + 1],
            fields[at + 2],
            fields[at + 3],
        ]))
    }

    /// Read a variable float of a slot.
    pub fn slot_float(&self, slot: u32, word: usize) -> Result<f32, GuestError> {
        Ok(f32::from_bits(self.slot_int(slot, word)? as u32))
    }

    /// Write a variable word of a slot.
    pub fn set_slot_int(&mut self, slot: u32, word: usize, value: i32) -> Result<(), GuestError> {
        let field_words = self.layout.field_words;
        let fields = self.field_bytes_mut(slot)?;
        let at = word
            .checked_mul(4)
            .filter(|at| at + 4 <= field_words * 4)
            .ok_or_else(|| {
                GuestError::memory_fault(
                    "word-range",
                    (word as u64).saturating_mul(4),
                    4,
                    "write",
                    "entity word outside variable storage",
                )
            })?;
        fields[at..at + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Write a variable float of a slot.
    pub fn set_slot_float(&mut self, slot: u32, word: usize, value: f32) -> Result<(), GuestError> {
        self.set_slot_int(slot, word, value.to_bits() as i32)
    }

    /// Read a variable vector of a slot.
    pub fn slot_vector(&self, slot: u32, word: usize) -> Result<Vec3, GuestError> {
        Ok(vec3(
            self.slot_float(slot, word)?,
            self.slot_float(slot, word + 1)?,
            self.slot_float(slot, word + 2)?,
        ))
    }

    /// Write a variable vector of a slot.
    pub fn set_slot_vector(&mut self, slot: u32, word: usize, value: Vec3) -> Result<(), GuestError> {
        self.set_slot_float(slot, word, value.x)?;
        self.set_slot_float(slot, word + 1, value.y)?;
        self.set_slot_float(slot, word + 2, value.z)
    }

    /// Byte pointer to an entity variable word (`Address` opcode).
    pub fn pointer(&self, reference: i32, field: i32) -> Result<i32, GuestError> {
        self.slot(reference)?;
        if field < 0 || field as usize >= self.layout.field_words {
            return Err(self.entity_error(format!("invalid field offset {field}")));
        }
        Ok(reference + self.layout.variables_offset_bytes as i32 + field * 4)
    }

    /// Resolve a byte pointer to its slot and variable word.
    pub fn resolve_pointer(&self, pointer: i32, words: usize) -> Result<(u32, usize), GuestError> {
        if pointer < 0 || pointer % 4 != 0 {
            return Err(self.entity_error(format!("invalid entity pointer {pointer}")));
        }
        let pointer = pointer as usize;
        let slot = pointer / self.layout.stride_bytes;
        let offset = pointer % self.layout.stride_bytes;
        if offset < self.layout.variables_offset_bytes {
            return Err(self.entity_error(format!("pointer {pointer} does not address entity variables")));
        }
        let word = (offset - self.layout.variables_offset_bytes) / 4;
        let Ok(slot) = u32::try_from(slot) else {
            return Err(self.entity_error(format!("pointer {pointer} does not address entity variables")));
        };
        if word + words > self.layout.field_words {
            return Err(self.entity_error(format!("pointer {pointer} does not address entity variables")));
        }
        self.check_slot(slot)?;
        Ok((slot, word))
    }

    /// Restore record bytes and the live count from a checkpoint.
    pub fn restore(&mut self, bytes: &[u8], count: usize) -> Result<(), GuestError> {
        if bytes.len() != self.bytes.len() {
            return Err(GuestError::invalid("entity checkpoint capacity mismatch"));
        }
        self.set_count(count)?;
        self.bytes.copy_from_slice(bytes);
        Ok(())
    }
}

/// One engine-owned string buffer inside the arena.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EngineString {
    name: String,
    offset: usize,
    capacity: usize,
}

/// A byte arena that keeps string aliases live. QuakeWorld engine buffers
/// use the source's negative IDs.
#[derive(Debug, Clone)]
pub struct QcStrings {
    arena: Vec<u8>,
    used: usize,
    engines: Vec<EngineString>,
    quakeworld: bool,
}

impl QcStrings {
    /// Seed the arena with the program string table.
    pub fn new(program_strings: &[u8], quakeworld: bool) -> Self {
        let mut arena = vec![0; program_strings.len().max(4096)];
        arena[..program_strings.len()].copy_from_slice(program_strings);
        Self {
            arena,
            used: program_strings.len(),
            engines: Vec::new(),
            quakeworld,
        }
    }

    /// Whether QuakeWorld negative engine IDs are active.
    #[must_use]
    pub const fn is_quakeworld(&self) -> bool {
        self.quakeworld
    }

    fn reserve(&mut self, length: usize) -> Result<usize, GuestError> {
        let offset = self.used;
        let required = offset
            .checked_add(length)
            .ok_or_else(|| GuestError::invalid("string arena overflow"))?;
        if required > 0x7fff_ffff {
            return Err(GuestError::invalid("string arena overflow"));
        }
        if required > self.arena.len() {
            self.arena.resize(required.max(self.arena.len() * 2), 0);
        }
        self.used = required;
        Ok(offset)
    }

    fn encode(&mut self, offset: usize, capacity: usize, text: &str) -> Result<(), GuestError> {
        let bytes: Vec<u8> = text
            .chars()
            .map(|char| {
                let code = char as u32;
                if code == 0 || code > 255 {
                    Err(GuestError::invalid("QC strings require non-null byte characters"))
                } else {
                    Ok(code as u8)
                }
            })
            .collect::<Result<_, _>>()?;
        if bytes.len() + 1 > capacity {
            return Err(GuestError::invalid("engine string exceeds its buffer"));
        }
        self.arena[offset..offset + bytes.len()].copy_from_slice(&bytes);
        self.arena[offset + bytes.len()] = 0;
        Ok(())
    }

    /// Allocate a program string, returning its arena offset.
    pub fn allocate(&mut self, text: &str) -> Result<i32, GuestError> {
        let offset = self.reserve(text.len() + 1)?;
        self.encode(offset, text.len() + 1, text)?;
        Ok(offset as i32)
    }

    /// Write a named engine buffer, returning its reference (negative on
    /// QuakeWorld, arena offset otherwise).
    pub fn set_engine(&mut self, name: &str, text: &str, capacity: usize) -> Result<i32, GuestError> {
        let found = self.engines.iter().position(|entry| entry.name == name);
        let index = if let Some(index) = found {
            index
        } else {
            if capacity < 1 {
                return Err(GuestError::invalid("Invalid engine string capacity"));
            }
            if self.quakeworld && self.engines.len() >= 1023 {
                return Err(GuestError::invalid("MAX_PRSTR"));
            }
            let offset = self.reserve(capacity)?;
            self.engines.push(EngineString {
                name: name.to_string(),
                offset,
                capacity,
            });
            self.engines.len() - 1
        };
        let (offset, buffer) = {
            let entry = &self.engines[index];
            (entry.offset, entry.capacity)
        };
        self.encode(offset, buffer, text)?;
        Ok(if self.quakeworld {
            -(index as i32) - 1
        } else {
            offset as i32
        })
    }

    /// Read the string at a reference (negative IDs resolve engine buffers).
    pub fn get(&self, reference: i32) -> Result<String, GuestError> {
        let offset = if reference < 0 {
            if !self.quakeworld {
                return Err(GuestError::invalid(format!("invalid engine string {reference}")));
            }
            let index = (-(reference as i64) - 1) as usize;
            self.engines
                .get(index)
                .map(|entry| entry.offset)
                .ok_or_else(|| GuestError::invalid(format!("invalid engine string {reference}")))?
        } else {
            reference as usize
        };
        qc_byte_string(&self.arena[..self.used], offset as i64).map_err(|_| {
            GuestError::memory_fault(
                "string-range",
                offset as u64,
                1,
                "read",
                format!("invalid string offset {offset}"),
            )
        })
    }

    /// Snapshot the arena and engine buffers.
    pub fn snapshot(&self) -> Result<Vec<u8>, GuestError> {
        let mut writer =
            BinaryWriter::new(16 + self.used + self.engines.iter().map(|entry| 12 + entry.name.len()).sum::<usize>());
        let write = |writer: &mut BinaryWriter| -> Result<(), qa_core::binary::BinaryError> {
            writer.u32(QC_STRINGS_MAGIC)?;
            writer.u32(u32::from(self.quakeworld))?;
            writer.u32(self.used as u32)?;
            writer.bytes(&self.arena[..self.used])?;
            writer.u32(self.engines.len() as u32)?;
            for entry in &self.engines {
                writer.u32(entry.offset as u32)?;
                writer.u32(entry.capacity as u32)?;
                writer.u32(entry.name.len() as u32)?;
                writer.bytes(entry.name.as_bytes())?;
            }
            Ok(())
        };
        write(&mut writer).map_err(|error| GuestError::invalid(error.to_string()))?;
        Ok(writer.finish())
    }

    /// Restore a snapshot, replacing the arena and engine buffers.
    pub fn restore(&mut self, bytes: &[u8]) -> Result<(), GuestError> {
        let mut reader = BinaryReader::new(bytes, "QC strings checkpoint");
        let invalid = |detail: &str| GuestError::invalid(format!("QC strings checkpoint: {detail}"));
        let magic = reader.u32().map_err(|_| invalid("truncated"))?;
        let quakeworld = reader.u32().map_err(|_| invalid("truncated"))?;
        if magic != QC_STRINGS_MAGIC || quakeworld != u32::from(self.quakeworld) {
            return Err(invalid("incompatible string checkpoint"));
        }
        let used = reader.u32().map_err(|_| invalid("truncated"))? as usize;
        let arena = reader.bytes(used).map_err(|_| invalid("truncated"))?;
        let count = reader.u32().map_err(|_| invalid("truncated"))?;
        let mut entries = Vec::new();
        for _ in 0..count {
            let offset = reader.u32().map_err(|_| invalid("truncated"))? as usize;
            let capacity = reader.u32().map_err(|_| invalid("truncated"))? as usize;
            let name_length = reader.u32().map_err(|_| invalid("truncated"))? as usize;
            let name = reader
                .fixed_byte_string(name_length)
                .map_err(|_| invalid("invalid engine string checkpoint"))?;
            if capacity < 1 || offset + capacity > arena.len() {
                return Err(invalid("invalid engine string checkpoint"));
            }
            entries.push(EngineString { name, offset, capacity });
        }
        if reader.remaining() != 0 {
            return Err(invalid("trailing string checkpoint bytes"));
        }
        self.used = arena.len();
        self.arena = arena;
        self.engines = entries;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_reject_partial_storage() {
        assert!(QcWords::new(vec![0; 3]).is_err());
        let mut words = QcWords::zeroed(4);
        assert_eq!(words.len_words(), 4);
        words.set_int(0, -7).unwrap();
        words.set_float(1, 1.5).unwrap();
        assert_eq!(words.int(0).unwrap(), -7);
        assert_eq!(words.float(1).unwrap(), 1.5);
        assert!(words.int(4).is_err());
        assert!(words.set_int(usize::MAX, 1).is_err());
        words.set_vector(1, vec3(1.0, 2.0, 3.0)).unwrap();
        assert_eq!(words.vector(1).unwrap(), vec3(1.0, 2.0, 3.0));
    }

    #[test]
    fn copy_within_matches_sequential_overlap() {
        let mut words = QcWords::zeroed(4);
        for index in 0..4 {
            words.set_int(index, index as i32 + 1).unwrap();
        }
        words.copy_within(0, 1, 3).unwrap();
        assert_eq!(words.int(0).unwrap(), 1);
        assert_eq!(words.int(1).unwrap(), 1);
        assert_eq!(words.int(2).unwrap(), 1);
        assert_eq!(words.int(3).unwrap(), 1);
    }

    #[test]
    fn entities_validate_slots_and_pointers() {
        let layout = QcEntityLayout {
            stride_bytes: 16,
            variables_offset_bytes: 8,
            field_words: 2,
        };
        let mut entities = QcEntityMemory::new(layout, 4, 2).unwrap();
        assert_eq!(entities.reference(1).unwrap(), 16);
        assert_eq!(entities.slot(16).unwrap(), 1);
        assert!(entities.reference(2).is_err());
        assert!(entities.slot(7).is_err());
        entities.set_slot_int(1, 0, 42).unwrap();
        assert_eq!(entities.slot_int(1, 0).unwrap(), 42);
        assert_eq!(entities.pointer(16, 1).unwrap(), 28);
        assert_eq!(entities.resolve_pointer(28, 1).unwrap(), (1, 1));
        assert!(entities.resolve_pointer(16, 1).is_err());
        assert!(entities.resolve_pointer(27, 1).is_err());
        assert_eq!(entities.record_bytes(1).unwrap().len(), 16);
        entities.set_count(4).unwrap();
        assert!(entities.set_count(0).is_err());
    }

    #[test]
    fn entities_reject_bad_layout() {
        let bad = QcEntityLayout {
            stride_bytes: 12,
            variables_offset_bytes: 8,
            field_words: 2,
        };
        assert!(QcEntityMemory::new(bad, 4, 1).is_err());
    }

    #[test]
    fn strings_keep_aliases_live() {
        let mut strings = QcStrings::new(b"\0quake\0", false);
        assert_eq!(strings.get(1).unwrap(), "quake");
        let offset = strings.allocate("hello").unwrap();
        assert_eq!(strings.get(offset).unwrap(), "hello");
        assert_eq!(strings.get(1).unwrap(), "quake");
        let engine = strings.set_engine("temp", "v1", 16).unwrap();
        assert_eq!(strings.get(engine).unwrap(), "v1");
        strings.set_engine("temp", "v2", 16).unwrap();
        assert_eq!(strings.get(engine).unwrap(), "v2");
        assert!(strings.allocate("has\0nul").is_err());
        assert!(strings.get(9999).is_err());
    }

    #[test]
    fn quakeworld_engine_ids_are_negative() {
        let mut strings = QcStrings::new(b"\0", true);
        let first = strings.set_engine("a", "x", 8).unwrap();
        let second = strings.set_engine("b", "y", 8).unwrap();
        assert_eq!((first, second), (-1, -2));
        assert_eq!(strings.get(-2).unwrap(), "y");
        assert!(strings.get(-3).is_err());
        let snapshot = strings.snapshot().unwrap();
        assert_eq!(&snapshot[..4], &QC_STRINGS_MAGIC.to_le_bytes());
        let mut restored = QcStrings::new(b"\0", true);
        restored.restore(&snapshot).unwrap();
        assert_eq!(restored.get(-1).unwrap(), "x");
        assert_eq!(restored.get(-2).unwrap(), "y");
    }

    #[test]
    fn string_snapshot_rejects_mismatches() {
        let strings = QcStrings::new(b"\0", false);
        let snapshot = strings.snapshot().unwrap();
        let mut qw = QcStrings::new(b"\0", true);
        assert!(qw.restore(&snapshot).is_err());
        let mut same = QcStrings::new(b"\0", false);
        let mut trailing = snapshot.clone();
        trailing.push(0);
        assert!(same.restore(&trailing).is_err());
        assert!(same.restore(&snapshot[..8]).is_err());
    }
}
