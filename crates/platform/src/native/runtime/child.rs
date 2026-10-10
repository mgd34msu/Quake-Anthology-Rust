//! Child-local C services. References into native bytes end before any foreign
//! callback; the engine still borrows only after its kernel stop boundary.
use super::{ImportResult, NativeEntry, NativeError, NativeRegion};
use crate::native::runtime::{Comparison, Function, Operation, RuntimeConfig};
use qa_core::heap::{Heap, MemoryError};
use std::sync::{Mutex, OnceLock};
#[path = "crt.rs"]
mod crt;
#[path = "msvc.rs"]
mod msvc;

struct Memory {
    base: u64,
    length: usize,
    regions: Box<[NativeRegion]>,
}
// Only mapping metadata is borrowed. Native bytes are copied through checked
// raw pointers, so neither a scan nor a callback retains a Rust byte reference.
#[derive(Clone)]
struct Cursor<'a> {
    memory: &'a Memory,
    region: usize,
    at: usize,
    end: usize,
    rights: u8,
}
impl Cursor<'_> {
    fn advance_region(&mut self) -> Result<(), NativeError> {
        if self.at < self.end {
            return Ok(());
        }
        self.region += 1;
        let region = self
            .memory
            .regions
            .get(self.region)
            .filter(|r| r.offset == self.at && r.permissions & self.rights == self.rights)
            .ok_or(NativeError::Extent)?;
        self.end = region.offset + region.length;
        Ok(())
    }
    fn validate(&self, bytes: usize) -> Result<(), NativeError> {
        let end = self
            .at
            .checked_add(bytes)
            .filter(|&end| end <= self.memory.length)
            .ok_or(NativeError::Extent)?;
        let mut cursor = self.clone();
        while cursor.at < end {
            cursor.advance_region()?;
            cursor.at = end.min(cursor.end);
        }
        Ok(())
    }
    fn read(&mut self) -> Result<u8, NativeError> {
        self.advance_region()?;
        // SAFETY: the cursor's current region is readable and contains at.
        // This scalar copy creates no borrow into shared native bytes.
        let byte = unsafe { ((self.memory.base as *const u8).add(self.at)).read() };
        self.at += 1;
        Ok(byte)
    }
    fn write(&mut self, byte: u8) -> Result<(), NativeError> {
        self.advance_region()?;
        // SAFETY: the cursor's current region is writable and contains at;
        // only the child thread accesses these bytes during this operation.
        unsafe { ((self.memory.base as *mut u8).add(self.at)).write(byte) };
        self.at += 1;
        Ok(())
    }
    fn length(&self) -> Result<usize, NativeError> {
        let start = self.at;
        let mut cursor = self.clone();
        loop {
            cursor.advance_region()?;
            while cursor.at < cursor.end {
                // SAFETY: this entire region has read permission. No native
                // byte reference is constructed or held across a callback.
                if unsafe { ((cursor.memory.base as *const u8).add(cursor.at)).read() } == 0 {
                    return Ok(cursor.at - start);
                }
                cursor.at += 1;
            }
        }
    }
}
impl Memory {
    fn cursor(&self, address: u64, rights: u8) -> Result<Cursor<'_>, NativeError> {
        let at = usize::try_from(address.checked_sub(self.base).ok_or(NativeError::Extent)?)
            .map_err(|_| NativeError::Extent)?;
        let index = self
            .regions
            .partition_point(|r| r.offset <= at)
            .checked_sub(1)
            .ok_or(NativeError::Extent)?;
        let region = &self.regions[index];
        if region.permissions & rights != rights || at - region.offset >= region.length {
            return Err(NativeError::Extent);
        }
        Ok(Cursor {
            memory: self,
            region: index,
            at,
            end: region.offset + region.length,
            rights,
        })
    }
    fn range(&self, address: u64, bytes: usize, rights: u8) -> Result<usize, NativeError> {
        let offset = usize::try_from(address.checked_sub(self.base).ok_or(NativeError::Extent)?)
            .map_err(|_| NativeError::Extent)?;
        offset
            .checked_add(bytes)
            .filter(|&end| end <= self.length)
            .ok_or(NativeError::Extent)?;
        if bytes != 0 {
            self.cursor(address, rights)?.validate(bytes)?;
        }
        Ok(offset)
    }
    fn read(&self, address: u64) -> Result<u8, NativeError> {
        self.cursor(address, 1)?.read()
    }
    fn unsigned(&self, address: u64, bytes: usize) -> Result<u64, NativeError> {
        if !matches!(bytes, 1 | 2 | 4 | 8) {
            return Err(NativeError::Extent);
        }
        self.range(address, bytes, 1)?;
        let mut value = 0;
        for i in 0..bytes {
            // SAFETY: checked readable mapping; scalar copy retains no borrow.
            value |= u64::from(unsafe { (address as *const u8).add(i).read() }) << (i * 8);
        }
        Ok(value)
    }
    fn put(&self, address: u64, bytes: usize, value: u64) -> Result<(), NativeError> {
        if !matches!(bytes, 1 | 2 | 4 | 8) {
            return Err(NativeError::Extent);
        }
        self.range(address, bytes, 2)?;
        for i in 0..bytes {
            // SAFETY: checked writable mapping; no Rust view escapes the copy.
            unsafe { (address as *mut u8).add(i).write((value >> (i * 8)) as u8) };
        }
        Ok(())
    }
    fn copy(&self, to: u64, from: u64, bytes: usize) -> Result<(), NativeError> {
        self.range(from, bytes, 1)?;
        self.range(to, bytes, 2)?;
        // SAFETY: both mapped ranges were checked, overlap is admitted, and no
        // Rust borrow into either range spans the copy.
        unsafe { std::ptr::copy(from as *const u8, to as *mut u8, bytes) };
        Ok(())
    }
    fn fill(&self, to: u64, byte: u8, bytes: usize) -> Result<(), NativeError> {
        self.range(to, bytes, 2)?;
        // SAFETY: writable mapped range, no overlapping Rust references.
        unsafe { std::ptr::write_bytes(to as *mut u8, byte, bytes) };
        Ok(())
    }
    fn swap(&self, left: u64, right: u64, bytes: usize) -> Result<(), NativeError> {
        self.range(left, bytes, 3)?;
        self.range(right, bytes, 3)?;
        if left != right {
            if left.abs_diff(right) < bytes as u64 {
                return Err(NativeError::Extent);
            }
            // SAFETY: both ranges are checked readable/writable and disjoint.
            // No Rust views into them exist during the child-local byte swap.
            unsafe { std::ptr::swap_nonoverlapping(left as *mut u8, right as *mut u8, bytes) };
        }
        Ok(())
    }
    fn length(&self, address: u64) -> Result<usize, NativeError> {
        self.cursor(address, 1)?.length()
    }
    // Caller must perform only pure byte inspection until this slice expires:
    // no mapped-byte writes, native callbacks, or publication to the parent.
    unsafe fn string_bytes(&self, address: u64) -> Result<&[u8], NativeError> {
        let length = self.length(address)?;
        // SAFETY: length scanned the entire readable span including its NUL.
        // The caller's invariant excludes all writers during this byte borrow.
        Ok(unsafe { std::slice::from_raw_parts(address as *const u8, length) })
    }
}
struct Runtime {
    memory: Memory,
    imports: Box<[Option<(&'static Function, NativeEntry)>]>,
    heap: Mutex<Option<Heap>>,
    config: Option<RuntimeConfig>,
}
static CHILD: OnceLock<Runtime> = OnceLock::new();

pub(super) fn initialize(
    base: u64,
    length: usize,
    regions: Box<[NativeRegion]>,
    imports: Box<[Option<(&'static Function, NativeEntry)>]>,
    config: Option<RuntimeConfig>,
) -> Result<(), NativeError> {
    let memory = Memory {
        base,
        length,
        regions,
    };
    let heap = config
        .filter(|c| c.heap_bytes != 0)
        .map(|c| {
            let start = c
                .base
                .checked_add(crate::native::PAGE_BYTES as u64)
                .ok_or(NativeError::Extent)?;
            memory.range(start, c.heap_bytes, 3)?;
            Heap::load(start, c.heap_bytes, 65536).map_err(|_| NativeError::Extent)
        })
        .transpose()?;
    CHILD
        .set(Runtime {
            memory,
            imports,
            heap: Mutex::new(heap),
            config,
        })
        .map_err(|_| NativeError::Protocol)
}

pub(super) fn invoke(
    ordinal: usize,
    words: [u64; 13],
    floats: [u64; 8],
) -> Result<Option<ImportResult>, NativeError> {
    let runtime = CHILD.get().ok_or(NativeError::Protocol)?;
    let Some(&(function, entry)) = runtime
        .imports
        .get(ordinal)
        .ok_or(NativeError::Protocol)?
        .as_ref()
    else {
        return Ok(None);
    };
    let arguments = entry.unpack(words, floats);
    let value = runtime.call(function.operation, arguments, entry.abi)?;
    Ok(Some(ImportResult {
        value: entry.result(value),
        kind: entry.control & 3,
    }))
}
impl Runtime {
    fn state(&self) -> Result<u64, NativeError> {
        self.config.map(|c| c.base).ok_or(NativeError::Extent)
    }
    fn allocate(&self, bytes: usize) -> Result<u64, NativeError> {
        self.heap
            .lock()
            .map_err(|_| NativeError::Protocol)?
            .as_mut()
            .ok_or(NativeError::Extent)
            .map(|heap| heap.allocate(bytes).unwrap_or(0))
    }
    fn free(&self, address: u64) -> Result<(), NativeError> {
        self.heap
            .lock()
            .map_err(|_| NativeError::Protocol)?
            .as_mut()
            .ok_or(NativeError::Extent)?
            .free(address)
            .map_err(|_| NativeError::Extent)
    }
    fn foreign(
        &self,
        address: u64,
        abi: crate::native::NativeAbi,
        kinds: &[crate::native::NativeScalar],
        result: crate::native::NativeScalar,
        arguments: &[u64],
    ) -> Result<u64, NativeError> {
        self.memory.range(address, 1, 4)?;
        let entry = NativeEntry::bind(address, abi, kinds, result)?;
        if arguments.len() != entry.argument_count() {
            return Err(NativeError::Protocol);
        }
        let mut words = [0; 13];
        words[..arguments.len()].copy_from_slice(arguments);
        let (words, floats) = entry.pack(words);
        // SAFETY: the hardware gate captures the current shared guest RSP.
        // A nested call uses untouched stack below it, preserving the red zone.
        let top = unsafe { super::x64::callback_top() };
        self.memory
            .range(top.checked_sub(112).ok_or(NativeError::Extent)?, 112, 3)?;
        // SAFETY: checked executable target and load-selected scalar ABI. All
        // mapped-byte accessors and heap locks have ended before foreign code;
        // the gate saves/restores its private frame and outer import state.
        Ok(entry.result(unsafe {
            super::x64::call(address, abi as u64, &words, top, &floats, entry.control)
        }))
    }
    fn call(
        &self,
        operation: Operation,
        a: [u64; 13],
        abi: crate::native::NativeAbi,
    ) -> Result<u64, NativeError> {
        let size = |value| usize::try_from(value).map_err(|_| NativeError::Extent);
        let m = &self.memory;
        Ok(match operation {
            Operation::Msvc(operation) => return self.msvc(operation, a),
            Operation::Crt(operation) => return self.crt(operation, a, abi),
            Operation::Math(operation, precision) => return self.math(operation, precision, a),
            Operation::Data(_) => return Err(NativeError::Unsupported),
            Operation::Copy => {
                m.copy(a[0], a[1], size(a[2])?)?;
                a[0]
            }
            Operation::Fill => {
                m.fill(a[0], a[1] as u8, size(a[2])?)?;
                a[0]
            }
            Operation::Strncpy => {
                let count = size(a[2])?;
                if count == 0 {
                    m.range(a[0], 0, 2)?;
                } else {
                    let mut to = m.cursor(a[0], 2)?;
                    to.validate(count)?;
                    let mut from = m.cursor(a[1], 1)?;
                    let mut ended = false;
                    for _ in 0..count {
                        let byte = if ended { 0 } else { from.read()? };
                        ended |= byte == 0;
                        to.write(byte)?;
                    }
                }
                a[0]
            }
            Operation::Length => m.length(a[0])? as u64,
            Operation::Compare(comparison) => {
                let string = matches!(comparison, Comparison::String);
                let prefix = matches!(comparison, Comparison::Prefix);
                if prefix && (a[0] == 0 || a[1] == 0 || a[2] > 0x10000000) {
                    return Err(NativeError::Extent);
                }
                let mut difference = 0;
                if !string && a[2] == 0 {
                    if !prefix {
                        m.range(a[0], 0, 1)?;
                        m.range(a[1], 0, 1)?;
                    }
                } else {
                    let mut left = m.cursor(a[0], 1)?;
                    let left_length = if string { left.length()? } else { 0 };
                    let mut right = m.cursor(a[1], 1)?;
                    let count = if string {
                        left_length.min(right.length()?) + 1
                    } else {
                        size(a[2])?
                    };
                    if !prefix {
                        left.validate(count)?;
                        right.validate(count)?;
                    }
                    for _ in 0..count {
                        let l = left.read()?;
                        difference = i32::from(l) - i32::from(right.read()?);
                        if difference != 0 || (prefix && l == 0) {
                            break;
                        }
                    }
                }
                difference as i64 as u64
            }
            Operation::Find(string) => {
                if !string && a[2] > 0x10000000 {
                    return Err(NativeError::Extent);
                }
                if !string && a[2] == 0 {
                    return Ok(0);
                }
                let mut cursor = m.cursor(a[0], 1)?;
                let count = if string {
                    cursor.length()? + 1
                } else {
                    size(a[2])?
                };
                cursor.validate(count)?;
                let mut result = 0;
                for index in 0..count {
                    if cursor.read()? == a[1] as u8 {
                        result = a[0] + index as u64;
                        break;
                    }
                }
                result
            }
            Operation::Malloc | Operation::Calloc | Operation::Realloc | Operation::Free => {
                let mut guard = self.heap.lock().map_err(|_| NativeError::Protocol)?;
                let heap = guard.as_mut().ok_or(NativeError::Extent)?;
                match operation {
                    Operation::Malloc => heap.allocate(size(a[0])?).unwrap_or(0),
                    Operation::Calloc => {
                        if let Some(bytes) = size(a[0])?.checked_mul(size(a[1])?) {
                            let address = heap.allocate(bytes).unwrap_or(0);
                            if address != 0 {
                                m.fill(address, 0, bytes)?;
                            }
                            address
                        } else {
                            0
                        }
                    }
                    Operation::Realloc => heap
                        .reallocate(
                            |to, from, bytes| m.copy(to, from, bytes).map_err(|_| MemoryError),
                            a[0],
                            size(a[1])?,
                        )
                        .map_err(|_| NativeError::Extent)?,
                    Operation::Free => {
                        heap.free(a[0]).map_err(|_| NativeError::Extent)?;
                        0
                    }
                    _ => return Err(NativeError::Protocol),
                }
            }
        })
    }
}
