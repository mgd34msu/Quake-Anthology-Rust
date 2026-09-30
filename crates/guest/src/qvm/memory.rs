//! QVM guest memory: masked pointers and `Q_strncpyz` string copies.
//!
//! Port of `src/compat/qvm/memory.ts` (`QvmMemory`, including `VM_ArgPtr` from
//! `qcommon/vm.c` and `Q_strncpyz` from `game/q_shared.c`; Copyright (C) 1999-2005
//! Id Software, Inc., GPL-2.0-or-later).
//!
//! The allocation is a nonzero power of two of at most 2^30 bytes. Guest words
//! are masked to the allocation, so only the pointer base is masked, never the
//! length. All clones share one backing through [`QvmMemory`], and every store
//! publishes through [`QvmMemoryWrites`](super::memory_writes::QvmMemoryWrites).
//! Failures use [`GuestError`](crate::error::GuestError); reads never
//! intercept, matching the donor (only setters publish).

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::GuestError;

use super::memory_writes::{QvmCommittedWrite, QvmMemoryWrites, QvmWriteRange};

/// Deferred-effect hook routing `after_publication` callbacks.
type QvmEffectHook = Option<Box<dyn FnMut(&mut dyn FnMut())>>;
/// Per-write publication hook.
type QvmPublishHook = Box<dyn FnMut(&QvmCommittedWrite) -> Result<(), GuestError>>;

/// Largest QVM allocation: 2^30 bytes.
pub const QVM_MAX_MEMORY_BYTES: usize = 0x4000_0000;

/// `Q_strncpyz` onto a live span: copy at most `size - 1` Latin-1 bytes, always
/// NUL-terminate, and pad the remainder of `size` with zeros.
fn q_strncpyz(destination: &mut [u8], source: &str, size: usize) -> Result<(), GuestError> {
    if size < 1 {
        return Err(GuestError::runtime("fatal: Q_strncpyz: destsize < 1"));
    }
    if size > destination.len() {
        return Err(GuestError::memory_fault(
            "out-of-bounds",
            0,
            size,
            "write",
            "Q_strncpyz destination exceeds its allocation",
        ));
    }
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < size - 1 {
        let byte = if index < bytes.len() { bytes[index] } else { 0 };
        destination[index] = byte;
        if byte == 0 {
            break;
        }
        index += 1;
    }
    destination[index..size].fill(0);
    Ok(())
}

#[derive(Debug)]
struct MemoryInner {
    bytes: Vec<u8>,
    writes: QvmMemoryWrites,
}

/// Live byte span over the shared allocation (a masked guest pointer).
#[derive(Debug, Clone)]
pub struct QvmSpan {
    memory: QvmMemory,
    start: usize,
    len: usize,
}

impl QvmSpan {
    /// Start offset within the allocation.
    #[must_use]
    pub fn start(&self) -> usize {
        self.start
    }

    /// Span length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the span is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Copy the span bytes out.
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        self.memory.read_bytes(self.start, self.len).unwrap_or_default()
    }

    /// Offset of the first `byte` in the span, if any.
    #[must_use]
    pub fn index_of(&self, byte: u8) -> Option<usize> {
        self.memory
            .with_bytes(|bytes| bytes[self.start..self.start + self.len].iter().position(|b| *b == byte))
    }

    /// Sub-span at `offset` with `len` bytes.
    pub fn subspan(&self, offset: usize, len: usize) -> Result<QvmSpan, GuestError> {
        if offset > self.len || len > self.len - offset {
            return Err(GuestError::memory_fault(
                "out-of-bounds",
                (self.start + offset) as u64,
                len,
                "read",
                "QVM memory span exceeds allocation or has an invalid length",
            ));
        }
        Ok(QvmSpan {
            memory: self.memory.clone(),
            start: self.start + offset,
            len,
        })
    }

    /// Scalar read/write view over the span.
    #[must_use]
    pub fn view(&self) -> QvmWritableView {
        QvmWritableView {
            memory: self.memory.clone(),
            start: self.start,
            len: self.len,
        }
    }
}

/// Scalar view over a live allocation range. Reads never intercept; writes
/// capture and publish when the owner intercepts.
#[derive(Debug, Clone)]
pub struct QvmWritableView {
    memory: QvmMemory,
    start: usize,
    len: usize,
}

impl QvmWritableView {
    /// Start offset within the allocation.
    #[must_use]
    pub fn start(&self) -> usize {
        self.start
    }

    /// View length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the view is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn index(&self, offset: usize, size: usize) -> Result<usize, GuestError> {
        if offset > self.len || size > self.len - offset {
            return Err(GuestError::memory_fault(
                "out-of-bounds",
                (self.start + offset) as u64,
                size,
                "access",
                "Offset is outside the bounds of the DataView",
            ));
        }
        Ok(self.start + offset)
    }

    /// Copy the view bytes out.
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        self.memory.read_bytes(self.start, self.len).unwrap_or_default()
    }

    /// Sub-view at `offset` with `len` bytes.
    pub fn subview(&self, offset: usize, len: usize) -> Result<QvmWritableView, GuestError> {
        let start = self.index(offset, len)?;
        Ok(QvmWritableView {
            memory: self.memory.clone(),
            start,
            len,
        })
    }

    /// Read a little-endian `i8`.
    pub fn get_i8(&self, offset: usize) -> Result<i8, GuestError> {
        Ok(self.memory.get_u8(self.index(offset, 1)?)? as i8)
    }

    /// Read a byte.
    pub fn get_u8(&self, offset: usize) -> Result<u8, GuestError> {
        self.memory.get_u8(self.index(offset, 1)?)
    }

    /// Read a little-endian `i16`.
    pub fn get_i16(&self, offset: usize) -> Result<i16, GuestError> {
        self.memory.get_i16(self.index(offset, 2)?)
    }

    /// Read a little-endian `u16`.
    pub fn get_u16(&self, offset: usize) -> Result<u16, GuestError> {
        self.memory.get_u16(self.index(offset, 2)?)
    }

    /// Read a little-endian `i32`.
    pub fn get_i32(&self, offset: usize) -> Result<i32, GuestError> {
        self.memory.get_i32(self.index(offset, 4)?)
    }

    /// Read a little-endian `u32`.
    pub fn get_u32(&self, offset: usize) -> Result<u32, GuestError> {
        self.memory.get_u32(self.index(offset, 4)?)
    }

    /// Read a little-endian `f32`.
    pub fn get_f32(&self, offset: usize) -> Result<f32, GuestError> {
        self.memory.get_f32(self.index(offset, 4)?)
    }

    /// Read a little-endian `f64`.
    pub fn get_f64(&self, offset: usize) -> Result<f64, GuestError> {
        self.memory.get_f64(self.index(offset, 8)?)
    }

    /// Read a little-endian `i64`.
    pub fn get_i64(&self, offset: usize) -> Result<i64, GuestError> {
        self.memory.get_i64(self.index(offset, 8)?)
    }

    /// Read a little-endian `u64`.
    pub fn get_u64(&self, offset: usize) -> Result<u64, GuestError> {
        self.memory.get_u64(self.index(offset, 8)?)
    }

    /// Write an `i8` with publication.
    pub fn set_i8(&self, offset: usize, value: i8) -> Result<(), GuestError> {
        let at = self.index(offset, 1)?;
        self.memory.set_u8(at, value as u8)
    }

    /// Write a byte with publication.
    pub fn set_u8(&self, offset: usize, value: u8) -> Result<(), GuestError> {
        let at = self.index(offset, 1)?;
        self.memory.set_u8(at, value)
    }

    /// Write a little-endian `i16` with publication.
    pub fn set_i16(&self, offset: usize, value: i16) -> Result<(), GuestError> {
        let at = self.index(offset, 2)?;
        self.memory.set_i16(at, value)
    }

    /// Write a little-endian `u16` with publication.
    pub fn set_u16(&self, offset: usize, value: u16) -> Result<(), GuestError> {
        let at = self.index(offset, 2)?;
        self.memory.set_u16(at, value)
    }

    /// Write a little-endian `i32` with publication.
    pub fn set_i32(&self, offset: usize, value: i32) -> Result<(), GuestError> {
        let at = self.index(offset, 4)?;
        self.memory.set_i32(at, value)
    }

    /// Write a little-endian `u32` with publication.
    pub fn set_u32(&self, offset: usize, value: u32) -> Result<(), GuestError> {
        let at = self.index(offset, 4)?;
        self.memory.set_u32(at, value)
    }

    /// Write a little-endian `f32` with publication.
    pub fn set_f32(&self, offset: usize, value: f32) -> Result<(), GuestError> {
        let at = self.index(offset, 4)?;
        self.memory.set_f32(at, value)
    }

    /// Write a little-endian `f64` with publication.
    pub fn set_f64(&self, offset: usize, value: f64) -> Result<(), GuestError> {
        let at = self.index(offset, 8)?;
        self.memory.set_f64(at, value)
    }

    /// Write a little-endian `i64` with publication.
    pub fn set_i64(&self, offset: usize, value: i64) -> Result<(), GuestError> {
        let at = self.index(offset, 8)?;
        self.memory.set_i64(at, value)
    }

    /// Write a little-endian `u64` with publication.
    pub fn set_u64(&self, offset: usize, value: u64) -> Result<(), GuestError> {
        let at = self.index(offset, 8)?;
        self.memory.set_u64(at, value)
    }

    /// Write the whole view with publication.
    pub fn write_bytes(&self, bytes: &[u8]) -> Result<(), GuestError> {
        if bytes.len() != self.len {
            return Err(GuestError::invalid("QVM view write must cover the whole view"));
        }
        self.memory.write_bytes(self.start, bytes)
    }
}

/// Borrows an interpreter allocation; pointer masking applies only to its start.
///
/// Clones share the allocation. All methods take `&self`; interior mutability
/// serializes access, and no borrow is held across host callbacks.
#[derive(Debug, Clone)]
pub struct QvmMemory {
    inner: Rc<RefCell<MemoryInner>>,
}

impl QvmMemory {
    /// Wrap `bytes`, which must be a nonzero power of two of at most 2^30 bytes.
    pub fn new(bytes: Vec<u8>) -> Result<Self, GuestError> {
        let length = bytes.len();
        if length == 0 || length > QVM_MAX_MEMORY_BYTES || !length.is_power_of_two() {
            return Err(GuestError::invalid(
                "QVM memory allocation must be a nonzero power of two at most 2^30 bytes",
            ));
        }
        Ok(Self {
            inner: Rc::new(RefCell::new(MemoryInner {
                bytes,
                writes: QvmMemoryWrites::new(),
            })),
        })
    }

    /// Allocation length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.borrow().bytes.len()
    }

    /// Whether the allocation is empty (never true; lengths are nonzero).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Mask applied to guest pointer words.
    #[must_use]
    pub fn mask(&self) -> usize {
        self.len() - 1
    }

    fn with_bytes<T>(&self, read: impl FnOnce(&[u8]) -> T) -> T {
        read(&self.inner.borrow().bytes)
    }

    /// Copy `len` bytes at `offset` out.
    pub fn read_bytes(&self, offset: usize, len: usize) -> Result<Vec<u8>, GuestError> {
        let inner = self.inner.borrow();
        inner.writes.check_range(inner.bytes.len(), offset, len)?;
        Ok(inner.bytes[offset..offset + len].to_vec())
    }

    /// Snapshot the whole allocation.
    #[must_use]
    pub fn snapshot(&self) -> Vec<u8> {
        self.inner.borrow().bytes.clone()
    }

    /// Route `after_publication` hooks through `effect` (default: run inline).
    pub fn set_effect(&self, effect: Option<Box<dyn FnMut(&mut dyn FnMut())>>) {
        self.inner.borrow_mut().writes.set_effect(effect);
    }

    /// Watch `ranges`; returns a watch id for [`Self::unobserve_writes`].
    pub fn observe_writes(
        &self,
        ranges: &[QvmWriteRange],
        publish: Box<dyn FnMut(&QvmCommittedWrite) -> Result<(), GuestError>>,
        after_publication: Option<Box<dyn FnMut(&QvmCommittedWrite) -> Result<(), GuestError>>>,
    ) -> Result<u64, GuestError> {
        let mut inner = self.inner.borrow_mut();
        let total = inner.bytes.len();
        inner.writes.observe(total, ranges, publish, after_publication)
    }

    /// Remove the watch installed by [`Self::observe_writes`].
    pub fn unobserve_writes(&self, id: u64) {
        self.inner.borrow_mut().writes.unobserve(id);
    }

    /// Whether stores currently capture.
    #[must_use]
    pub fn observes_writes(&self) -> bool {
        self.inner.borrow().writes.intercepts()
    }

    /// Fail once the allocation is retired.
    pub fn assert_live(&self) -> Result<(), GuestError> {
        self.inner.borrow().writes.assert_live()
    }

    /// Fail while a publication is in flight.
    pub fn assert_not_publishing(&self) -> Result<(), GuestError> {
        self.inner.borrow().writes.assert_not_publishing()
    }

    /// Remove every watch.
    pub fn clear_write_observers(&self) -> Result<(), GuestError> {
        self.inner.borrow_mut().writes.clear()
    }

    /// Retire the allocation: clear watches and reject later stores.
    pub fn close(&self) {
        self.inner.borrow_mut().writes.close();
    }

    /// Scalar view over `start..start+len`. Only bounds are checked, so retained
    /// views stay readable (matching the donor).
    pub fn data_view(&self, start: usize, len: usize) -> Result<QvmWritableView, GuestError> {
        let inner = self.inner.borrow();
        inner.writes.check_range(inner.bytes.len(), start, len)?;
        Ok(QvmWritableView {
            memory: self.clone(),
            start,
            len,
        })
    }

    /// Store `bytes` at `offset` with publication.
    pub fn write_bytes(&self, offset: usize, bytes: &[u8]) -> Result<(), GuestError> {
        let mut inner = self.inner.borrow_mut();
        inner.writes.assert_writable()?;
        inner.writes.check_range(inner.bytes.len(), offset, bytes.len())?;
        let captures = if inner.writes.intercepts() {
            inner.writes.before(&inner.bytes, offset, bytes.len())?
        } else {
            None
        };
        inner.bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
        let fields: &mut MemoryInner = &mut inner;
        let (writes, raw) = (&mut fields.writes, &fields.bytes);
        writes.after(raw, captures)
    }

    /// Fill `len` bytes at `offset` with `value`, with publication.
    pub fn fill_bytes(&self, offset: usize, len: usize, value: u8) -> Result<(), GuestError> {
        let mut inner = self.inner.borrow_mut();
        inner.writes.assert_writable()?;
        inner.writes.check_range(inner.bytes.len(), offset, len)?;
        let captures = if inner.writes.intercepts() {
            inner.writes.before(&inner.bytes, offset, len)?
        } else {
            None
        };
        inner.bytes[offset..offset + len].fill(value);
        let fields: &mut MemoryInner = &mut inner;
        let (writes, raw) = (&mut fields.writes, &fields.bytes);
        writes.after(raw, captures)
    }

    /// Move `len` bytes from `source` to `destination` (memmove), with publication.
    pub fn copy_bytes(&self, destination: usize, source: usize, len: usize) -> Result<(), GuestError> {
        let mut inner = self.inner.borrow_mut();
        inner.writes.assert_writable()?;
        inner.writes.check_range(inner.bytes.len(), destination, len)?;
        inner.writes.check_range(inner.bytes.len(), source, len)?;
        let captures = if inner.writes.intercepts() {
            inner.writes.before(&inner.bytes, destination, len)?
        } else {
            None
        };
        inner.bytes.copy_within(source..source + len, destination);
        let fields: &mut MemoryInner = &mut inner;
        let (writes, raw) = (&mut fields.writes, &fields.bytes);
        writes.after(raw, captures)
    }

    /// Mask a guest word to a live span. Word 0 maps to null.
    pub fn pointer(&self, word: i32) -> Result<Option<QvmSpan>, GuestError> {
        if word == 0 {
            return Ok(None);
        }
        let start = (word as u32 as usize) & self.mask();
        Ok(Some(QvmSpan {
            memory: self.clone(),
            start,
            len: self.len() - start,
        }))
    }

    /// Span of `len` bytes at masked `word` plus `relative_offset`.
    pub fn span(&self, word: i32, len: usize, relative_offset: i64) -> Result<QvmSpan, GuestError> {
        let pointer = self
            .pointer(word)?
            .ok_or_else(|| GuestError::invalid("QVM memory span requires a nonnull pointer"))?;
        let start = pointer.start as i64 + relative_offset;
        span_error(start, len, self.len())?;
        Ok(QvmSpan {
            memory: self.clone(),
            start: start as usize,
            len,
        })
    }

    /// Scalar view over `len` bytes at masked `word` plus `relative_offset`.
    pub fn view(&self, word: i32, len: usize, relative_offset: i64) -> Result<QvmWritableView, GuestError> {
        Ok(self.span(word, len, relative_offset)?.view())
    }

    /// Read a NUL-terminated Latin-1 string at masked `word`.
    pub fn read_string(&self, word: i32) -> Result<String, GuestError> {
        let pointer = self
            .pointer(word)?
            .ok_or_else(|| GuestError::invalid("QVM string requires a nonnull pointer"))?;
        let end = pointer
            .index_of(0)
            .ok_or_else(|| GuestError::invalid("QVM string has no terminator before the allocation ends"))?;
        let bytes = pointer.subspan(0, end)?.to_vec();
        Ok(bytes.iter().map(|byte| *byte as char).collect())
    }

    /// `Q_strncpyz`: copy `text` into `capacity` bytes at `word`, padding with
    /// NUL and leaving bytes beyond `capacity` untouched. A null word raises the
    /// source `Com_Error`.
    pub fn write_string(&self, word: i32, text: &str, capacity: usize) -> Result<(), GuestError> {
        let pointer = self.pointer(word)?;
        let Some(span) = pointer else {
            return Err(GuestError::runtime("fatal: Q_strncpyz: NULL dest"));
        };
        if capacity > span.len {
            return Err(GuestError::memory_fault(
                "out-of-bounds",
                span.start as u64,
                capacity,
                "write",
                "Q_strncpyz destination exceeds QVM allocation",
            ));
        }
        if capacity < 1 {
            return Err(GuestError::runtime("fatal: Q_strncpyz: destsize < 1"));
        }
        let mut inner = self.inner.borrow_mut();
        inner.writes.assert_writable()?;
        let captures = if inner.writes.intercepts() {
            inner.writes.before(&inner.bytes, span.start, capacity)?
        } else {
            None
        };
        q_strncpyz(&mut inner.bytes[span.start..span.start + capacity], text, capacity)?;
        let fields: &mut MemoryInner = &mut inner;
        let (writes, raw) = (&mut fields.writes, &fields.bytes);
        writes.after(raw, captures)
    }

    /// Raw bounded copy (`strncpy(size - 1)` plus NUL) with no source
    /// `Com_Error`: invalid pointers or sizes are range errors.
    pub fn write_bounded_string(&self, word: i32, text: &str, capacity: usize) -> Result<(), GuestError> {
        let pointer = self.pointer(word)?;
        let Some(span) = pointer else {
            return Err(GuestError::invalid("Bounded string copy exceeds QVM allocation"));
        };
        if capacity < 1 || capacity > span.len {
            return Err(GuestError::invalid("Bounded string copy exceeds QVM allocation"));
        }
        let mut inner = self.inner.borrow_mut();
        inner.writes.assert_writable()?;
        let captures = if inner.writes.intercepts() {
            inner.writes.before(&inner.bytes, span.start, capacity)?
        } else {
            None
        };
        q_strncpyz(&mut inner.bytes[span.start..span.start + capacity], text, capacity)?;
        let fields: &mut MemoryInner = &mut inner;
        let (writes, raw) = (&mut fields.writes, &fields.bytes);
        writes.after(raw, captures)
    }

    /// Read one byte.
    pub fn get_u8(&self, offset: usize) -> Result<u8, GuestError> {
        let inner = self.inner.borrow();
        inner.writes.check_range(inner.bytes.len(), offset, 1)?;
        Ok(inner.bytes[offset])
    }

    /// Read a little-endian `i16`.
    pub fn get_i16(&self, offset: usize) -> Result<i16, GuestError> {
        let inner = self.inner.borrow();
        inner.writes.check_range(inner.bytes.len(), offset, 2)?;
        Ok(i16::from_le_bytes([inner.bytes[offset], inner.bytes[offset + 1]]))
    }

    /// Read a little-endian `u16`.
    pub fn get_u16(&self, offset: usize) -> Result<u16, GuestError> {
        Ok(self.get_i16(offset)? as u16)
    }

    /// Read a little-endian `i32`.
    pub fn get_i32(&self, offset: usize) -> Result<i32, GuestError> {
        let inner = self.inner.borrow();
        inner.writes.check_range(inner.bytes.len(), offset, 4)?;
        Ok(i32::from_le_bytes([
            inner.bytes[offset],
            inner.bytes[offset + 1],
            inner.bytes[offset + 2],
            inner.bytes[offset + 3],
        ]))
    }

    /// Read a little-endian `u32`.
    pub fn get_u32(&self, offset: usize) -> Result<u32, GuestError> {
        Ok(self.get_i32(offset)? as u32)
    }

    /// Read a little-endian `f32`.
    pub fn get_f32(&self, offset: usize) -> Result<f32, GuestError> {
        Ok(f32::from_bits(self.get_u32(offset)?))
    }

    /// Read a little-endian `f64`.
    pub fn get_f64(&self, offset: usize) -> Result<f64, GuestError> {
        let inner = self.inner.borrow();
        inner.writes.check_range(inner.bytes.len(), offset, 8)?;
        let mut raw = [0u8; 8];
        raw.copy_from_slice(&inner.bytes[offset..offset + 8]);
        Ok(f64::from_le_bytes(raw))
    }

    /// Read a little-endian `i64`.
    pub fn get_i64(&self, offset: usize) -> Result<i64, GuestError> {
        let inner = self.inner.borrow();
        inner.writes.check_range(inner.bytes.len(), offset, 8)?;
        let mut raw = [0u8; 8];
        raw.copy_from_slice(&inner.bytes[offset..offset + 8]);
        Ok(i64::from_le_bytes(raw))
    }

    /// Read a little-endian `u64`.
    pub fn get_u64(&self, offset: usize) -> Result<u64, GuestError> {
        Ok(self.get_i64(offset)? as u64)
    }

    fn store(&self, offset: usize, bytes: &[u8]) -> Result<(), GuestError> {
        let mut inner = self.inner.borrow_mut();
        if !inner.writes.intercepts() {
            inner.writes.check_range(inner.bytes.len(), offset, bytes.len())?;
            inner.bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
            return Ok(());
        }
        inner.writes.assert_writable()?;
        let captures = inner.writes.before(&inner.bytes, offset, bytes.len())?;
        inner.bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
        let fields: &mut MemoryInner = &mut inner;
        let (writes, raw) = (&mut fields.writes, &fields.bytes);
        writes.after(raw, captures)
    }

    /// Write one byte with publication.
    pub fn set_u8(&self, offset: usize, value: u8) -> Result<(), GuestError> {
        self.store(offset, &[value])
    }

    /// Write a little-endian `i16` with publication.
    pub fn set_i16(&self, offset: usize, value: i16) -> Result<(), GuestError> {
        self.store(offset, &value.to_le_bytes())
    }

    /// Write a little-endian `u16` with publication.
    pub fn set_u16(&self, offset: usize, value: u16) -> Result<(), GuestError> {
        self.store(offset, &value.to_le_bytes())
    }

    /// Write a little-endian `i32` with publication.
    pub fn set_i32(&self, offset: usize, value: i32) -> Result<(), GuestError> {
        self.store(offset, &value.to_le_bytes())
    }

    /// Write a little-endian `u32` with publication.
    pub fn set_u32(&self, offset: usize, value: u32) -> Result<(), GuestError> {
        self.store(offset, &value.to_le_bytes())
    }

    /// Write a little-endian `f32` with publication.
    pub fn set_f32(&self, offset: usize, value: f32) -> Result<(), GuestError> {
        self.store(offset, &value.to_le_bytes())
    }

    /// Write a little-endian `f64` with publication.
    pub fn set_f64(&self, offset: usize, value: f64) -> Result<(), GuestError> {
        self.store(offset, &value.to_le_bytes())
    }

    /// Write a little-endian `i64` with publication.
    pub fn set_i64(&self, offset: usize, value: i64) -> Result<(), GuestError> {
        self.store(offset, &value.to_le_bytes())
    }

    /// Write a little-endian `u64` with publication.
    pub fn set_u64(&self, offset: usize, value: u64) -> Result<(), GuestError> {
        self.store(offset, &value.to_le_bytes())
    }

    fn store_raw(&self, offset: usize, bytes: &[u8]) -> Result<(), GuestError> {
        let mut inner = self.inner.borrow_mut();
        inner.writes.assert_writable()?;
        inner.writes.check_range(inner.bytes.len(), offset, bytes.len())?;
        inner.bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    /// Write one byte without publication (counter evaluation scratch stores).
    pub fn set_u8_unobserved(&self, offset: usize, value: u8) -> Result<(), GuestError> {
        self.store_raw(offset, &[value])
    }

    /// Write a little-endian `u16` without publication.
    pub fn set_u16_unobserved(&self, offset: usize, value: u16) -> Result<(), GuestError> {
        self.store_raw(offset, &value.to_le_bytes())
    }

    /// Write a little-endian `i32` without publication.
    pub fn set_i32_unobserved(&self, offset: usize, value: i32) -> Result<(), GuestError> {
        self.store_raw(offset, &value.to_le_bytes())
    }
}

fn span_error(start: i64, len: usize, total: usize) -> Result<(), GuestError> {
    if start < 0 || start as u64 > total as u64 || len > total - start as usize {
        return Err(GuestError::memory_fault(
            "out-of-bounds",
            start as u64,
            len,
            "read",
            "QVM memory span exceeds allocation or has an invalid length",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn memory() -> QvmMemory {
        QvmMemory::new(vec![0; 64]).unwrap()
    }

    #[test]
    fn rejects_non_power_of_two_lengths() {
        assert!(QvmMemory::new(vec![0; 0]).is_err());
        assert!(QvmMemory::new(vec![0; 48]).is_err());
        assert!(QvmMemory::new(vec![0; 64]).is_ok());
    }

    #[test]
    fn pointer_masks_negative_words_and_nulls_zero() {
        let memory = memory();
        assert!(memory.pointer(0).unwrap().is_none());
        let span = memory.pointer(-1).unwrap().unwrap();
        assert_eq!((span.start(), span.len()), (63, 1));
        let span = memory.pointer(64).unwrap().unwrap();
        assert_eq!((span.start(), span.len()), (0, 64));
    }

    #[test]
    fn span_rejects_overruns() {
        let memory = memory();
        assert!(memory.span(0, 1, 0).is_err());
        assert!(memory.span(60, 8, 0).is_err());
        assert!(memory.span(60, 4, 0).is_ok());
        assert!(memory.span(60, 4, -60).is_ok());
    }

    #[test]
    fn strings_round_trip_with_q_strncpyz_padding() {
        let memory = memory();
        memory.write_string(8, "hi", 8).unwrap();
        assert_eq!(memory.read_string(8).unwrap(), "hi");
        assert_eq!(memory.read_bytes(8, 8).unwrap(), vec![b'h', b'i', 0, 0, 0, 0, 0, 0]);
        memory.write_string(8, "12345678", 4).unwrap();
        assert_eq!(memory.read_string(8).unwrap(), "123");
        assert!(memory.write_string(0, "x", 4).is_err());
        assert!(memory.write_string(8, "x", 0).is_err());
    }

    #[test]
    fn bounded_strings_reject_nulls_as_range_errors() {
        let memory = memory();
        assert!(memory.write_bounded_string(0, "x", 4).is_err());
        memory.write_bounded_string(8, "ok", 4).unwrap();
        assert_eq!(memory.read_string(8).unwrap(), "ok");
    }

    #[test]
    fn scalar_views_publish_only_when_watched() {
        let memory = memory();
        let view = memory.data_view(0, 16).unwrap();
        view.set_i32(0, 0x0102_0304).unwrap();
        assert_eq!(view.get_i32(0).unwrap(), 0x0102_0304);
        assert_eq!(view.get_u16(0).unwrap(), 0x0304);
        let events: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
        let events_clone = Rc::clone(&events);
        memory
            .observe_writes(
                &[QvmWriteRange {
                    byte_offset: 0,
                    byte_length: 16,
                }],
                Box::new(move |_| {
                    *events_clone.borrow_mut() += 1;
                    Ok(())
                }),
                None,
            )
            .unwrap();
        view.set_i32(4, 42).unwrap();
        assert_eq!(*events.borrow(), 1);
        assert!(view.subview(0, 17).is_err());
    }

    #[test]
    fn copy_within_handles_overlap() {
        let memory = memory();
        memory.write_bytes(0, &[1, 2, 3, 4]).unwrap();
        memory.copy_bytes(1, 0, 3).unwrap();
        assert_eq!(memory.read_bytes(0, 4).unwrap(), vec![1, 1, 2, 3]);
    }
}
