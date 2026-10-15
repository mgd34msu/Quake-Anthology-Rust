//! Child-local C services. References into native bytes end before any foreign
//! callback; the engine borrows only while holding a received kernel notification.
use super::{ImportResult, NativeEntry, NativeError, NativeRegion};
use crate::native::runtime::{Comparison, Function, Operation, RuntimeConfig};
use qa_core::heap::{Heap, MemoryError};
use qa_core::random::{Algorithm, Random};
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicUsize, Ordering},
};
#[path = "crt.rs"]
mod crt;
#[path = "format.rs"]
mod format;
pub(super) use format::Capture;
#[path = "kernel.rs"]
mod kernel;
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
        let index =
            crate::native::region_at(&self.regions, at, rights).ok_or(NativeError::Extent)?;
        let region = &self.regions[index];
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
        // Native CRT calls admit null pointers for an empty copy. No bytes
        // are accessed, and Rust's pointer-copy preconditions need not apply.
        if bytes == 0 {
            return Ok(());
        }
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
    imports: Box<[ImportBinding]>,
    heap: Mutex<Option<Heap>>,
    config: Option<RuntimeConfig>,
    kernel: Mutex<kernel::State>,
    filter_depth: AtomicUsize,
    random: Mutex<[Random; 2]>,
}
// NativeAbi's source calling convention selects the admitted CRT profile.
// Streams are child-private, and their algorithms live only in core.
const RANDOM_ALGORITHMS: [Algorithm; 2] = [Algorithm::GlibcType3, Algorithm::MsvcRand];
static CHILD: OnceLock<Runtime> = OnceLock::new();

pub(super) enum ImportBinding {
    Engine(NativeEntry, Option<crate::native::NativeListFilter>),
    Formatted(NativeEntry, crate::native::NativeFormat),
    Runtime(&'static Function, NativeEntry),
    Trap,
}

pub(super) fn initialize(
    base: u64,
    length: usize,
    regions: Box<[NativeRegion]>,
    imports: Box<[ImportBinding]>,
    config: Option<RuntimeConfig>,
) -> Result<(), NativeError> {
    let memory = Memory {
        base,
        length,
        regions,
    };
    for binding in &imports {
        if let ImportBinding::Formatted(_, format) = binding {
            memory.range(format.buffer, usize::from(format.capacity), 3)?;
        }
        if let ImportBinding::Engine(_, Some(filter)) = binding {
            memory.range(
                filter.buffer,
                filter
                    .byte_length_for_width(size_of::<usize>() as u8)
                    .ok_or(NativeError::Extent)?,
                3,
            )?;
        }
    }
    if let Some(teb) = config.and_then(|c| c.teb) {
        memory.range(teb, crate::native::runtime::THREAD_BYTES, 3)?;
        let stack = memory.regions.last().ok_or(NativeError::Extent)?;
        for (offset, value) in [
            (0, u64::MAX),
            (8, base + (stack.offset + stack.length) as u64),
            (16, base + stack.offset as u64),
            (0x30, teb),
            (0x40, 1),
            (0x48, 1),
            (0x58, teb + crate::native::runtime::STATIC_TLS_OFFSET as u64),
            (0x60, teb + 0x2000),
            (0x1780, teb + 0x5000),
        ] {
            memory.put(teb + offset, 8, value)?;
        }
    }
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
            kernel: Mutex::new(kernel::State::default()),
            filter_depth: AtomicUsize::new(0),
            random: Mutex::new(
                RANDOM_ALGORITHMS.map(|algorithm| Random::with_algorithm(1, algorithm)),
            ),
        })
        .map_err(|_| NativeError::Protocol)
}

/// The sole foreign execution gate serves outer exports, host-requested reentry
/// and child-local CRT callbacks. Only mapping metadata survives this call.
pub(super) fn foreign_call(
    address: u64,
    abi: u8,
    words: &[u64; 13],
    floats: &[u64; 8],
    control: u64,
    top: Option<u64>,
) -> Result<u64, NativeError> {
    let runtime = CHILD.get().ok_or(NativeError::Protocol)?;
    if abi > 1 || size_of::<usize>() == 4 && abi != 0 || control & !0xff03 != 0 || control >> 8 > 8
    {
        return Err(NativeError::Protocol);
    }
    runtime.memory.range(address, 1, 4)?;
    // SAFETY: hardware captured the suspended guest's stack in private import
    // state. Reentry uses untouched stack below it, preserving its red zone.
    let top = top.unwrap_or_else(|| unsafe { super::hardware::callback_top() });
    runtime
        .memory
        .range(top.checked_sub(112).ok_or(NativeError::Extent)?, 112, 3)?;
    // SAFETY: the target and bounded guest stack are checked, the ABI/control
    // descriptor is validated, and no guest bytes or heap locks are borrowed.
    // Both hardware widths preserve the outer controller/import and FP state.
    let value =
        unsafe { super::hardware::call(address, u64::from(abi), words, top, floats, control) };
    if let Some((ordinal, address)) = super::child_trap() {
        return Err(NativeError::ImportTrap { ordinal, address });
    }
    Ok(value)
}

pub(super) fn invoke(
    ordinal: usize,
    words: [u64; 13],
    floats: [u64; 8],
    capture: Capture,
) -> Result<Option<ImportResult>, NativeError> {
    let runtime = CHILD.get().ok_or(NativeError::Protocol)?;
    let (function, entry) = match runtime.imports.get(ordinal).ok_or(NativeError::Protocol)? {
        ImportBinding::Engine(entry, filter) => {
            return filter.map_or(Ok(None), |filter| {
                runtime.filtered(ordinal, *entry, filter, words, floats)
            });
        }
        ImportBinding::Formatted(entry, format) => {
            let formatted = || {
                let mut arguments = entry.unpack(words, floats);
                let cursor = format::Arguments::direct(
                    &runtime.memory,
                    entry.abi,
                    usize::from(format.format) + 1,
                    words,
                    floats,
                    capture,
                )?;
                format::write(
                    &runtime.memory,
                    format.buffer,
                    usize::from(format.capacity),
                    arguments[usize::from(format.format)],
                    cursor,
                    false,
                )?;
                arguments[usize::from(format.format)] = format.buffer;
                let (words, floats) = entry.pack(arguments);
                Ok(Some(super::host_import(ordinal as u64, words, floats, 1)))
            };
            return formatted().map_err(|_: NativeError| NativeError::ImportTrap {
                ordinal,
                address: 0,
            });
        }
        ImportBinding::Trap => {
            return Err(NativeError::ImportTrap {
                ordinal,
                address: 0,
            });
        }
        ImportBinding::Runtime(function, entry) => (*function, *entry),
    };
    let arguments = entry.unpack(words, floats);
    let value = if let Operation::Format { sized, list } = function.operation {
        let format_index = if sized { 2 } else { 1 };
        let capacity = if sized {
            usize::try_from(arguments[1]).map_err(|_| NativeError::Extent)?
        } else {
            format::MAX_OUTPUT + 1
        };
        let cursor = if list {
            format::Arguments::list(&runtime.memory, entry.abi, arguments[format_index + 1])?
        } else {
            format::Arguments::direct(
                &runtime.memory,
                entry.abi,
                format_index + 1,
                words,
                floats,
                capture,
            )?
        };
        format::write(
            &runtime.memory,
            arguments[0],
            capacity,
            arguments[format_index],
            cursor,
            sized,
        )? as u64
    } else {
        runtime.call(function.operation, arguments, entry.abi)?
    };
    Ok(Some(ImportResult {
        value: entry.result(value),
        kind: entry.control & 3,
    }))
}
impl Runtime {
    fn filtered(
        &self,
        ordinal: usize,
        entry: NativeEntry,
        filter: crate::native::NativeListFilter,
        mut words: [u64; 13],
        floats: [u64; 8],
    ) -> Result<Option<ImportResult>, NativeError> {
        use crate::native::NativeScalar::{U32, Word};
        words = entry.unpack(words, floats);
        let pointer_bytes = size_of::<usize>();
        let callback = words[usize::from(filter.callback)];
        if callback == 0 {
            return Ok(None);
        }
        self.memory.range(callback, 1, 4)?;
        let output = words[usize::from(filter.list)];
        let limit =
            usize::try_from(words[usize::from(filter.limit)]).map_err(|_| NativeError::Extent)?;
        if output != 0 && limit != 0 {
            self.memory.range(
                output,
                limit
                    .checked_mul(pointer_bytes)
                    .ok_or(NativeError::Extent)?,
                3,
            )?;
        }
        let data = words[usize::from(filter.data)];
        let depth = self.filter_depth.fetch_add(1, Ordering::Relaxed);
        struct Bank<'a>(&'a AtomicUsize);
        impl Drop for Bank<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::Relaxed);
            }
        }
        let _bank = Bank(&self.filter_depth);
        if depth >= usize::from(filter.depth) {
            return Err(NativeError::Extent);
        }
        let scratch =
            filter.buffer + depth as u64 * u64::from(filter.capacity) * pointer_bytes as u64;
        words[usize::from(filter.list)] = scratch;
        words[usize::from(filter.limit)] = u64::from(filter.capacity);
        words[usize::from(filter.callback)] = 0;
        words[usize::from(filter.data)] = 0;
        let (words, floats) = entry.pack(words);
        let result = super::host_import(ordinal as u64, words, floats, 1);
        if result.value > u64::from(filter.capacity) {
            return Err(NativeError::Protocol);
        }
        let mut kept = 0usize;
        for index in 0..result.value {
            if limit != 0 && kept == limit {
                break;
            }
            let entity = self
                .memory
                .unsigned(scratch + index * pointer_bytes as u64, pointer_bytes)?;
            // All byte reads and the engine's area borrow end before the native
            // callback. Nested imports reuse the one kernel-parked shared slot;
            // nested filtered calls claim another fixed bank.
            let decision =
                self.foreign(callback, entry.abi, &[Word, Word], U32, &[entity, data])? as u32;
            if decision & !filter.end == filter.keep {
                if output != 0 && limit != 0 {
                    self.memory.put(
                        output + kept as u64 * pointer_bytes as u64,
                        pointer_bytes,
                        entity,
                    )?;
                }
                kept += 1;
            }
            if decision & filter.end != 0 {
                break;
            }
        }
        Ok(Some(ImportResult {
            value: kept as u64,
            kind: result.kind,
        }))
    }

    fn wall_millis(&self) -> Result<i64, NativeError> {
        Ok(self.memory.unsigned(
            self.state()? + crate::native::runtime::TIME_OFFSET as u64 + 8,
            8,
        )? as i64)
    }
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
        let entry = NativeEntry::bind(address, abi, size_of::<usize>() as u8, kinds, result)?;
        if arguments.len() != entry.argument_count() {
            return Err(NativeError::Protocol);
        }
        let mut words = [0; 13];
        words[..arguments.len()].copy_from_slice(arguments);
        let (words, floats) = entry.pack(words);
        foreign_call(address, abi as u8, &words, &floats, entry.control, None)
            .map(|value| entry.result(value))
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
            Operation::Kernel(operation) => return self.kernel(operation, a),
            Operation::Crt(operation) => return self.crt(operation, a, abi),
            Operation::Math(operation, precision) => return self.math(operation, precision, a),
            Operation::Random { seed } => {
                let index = abi as usize;
                let mut streams = self.random.lock().map_err(|_| NativeError::Protocol)?;
                let stream = &mut streams[index];
                if seed {
                    *stream = Random::with_algorithm(a[0] as u32, RANDOM_ALGORITHMS[index]);
                    0
                } else {
                    u64::from(stream.next_u32())
                }
            }
            Operation::Data(_) | Operation::Format { .. } => return Err(NativeError::Unsupported),
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
            Operation::Malloc
            | Operation::Calloc
            | Operation::Realloc
            | Operation::Free
            | Operation::TagMalloc
            | Operation::FreeTags => {
                let mut guard = self.heap.lock().map_err(|_| NativeError::Protocol)?;
                let heap = guard.as_mut().ok_or(NativeError::Extent)?;
                match operation {
                    Operation::Malloc => heap.allocate(size(a[0])?).unwrap_or(0),
                    Operation::TagMalloc => {
                        let bytes = size(a[0])?;
                        let address = heap
                            .allocate_tagged(bytes, a[1] as i32)
                            .ok_or(NativeError::Extent)?;
                        m.fill(address, 0, bytes)?;
                        address
                    }
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
                    Operation::FreeTags => {
                        heap.free_tag(a[0] as i32);
                        0
                    }
                    _ => return Err(NativeError::Protocol),
                }
            }
        })
    }
}
