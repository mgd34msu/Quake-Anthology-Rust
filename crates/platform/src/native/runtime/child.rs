//! Child-local C services. References into native bytes end before any foreign
//! callback; the engine still borrows only after its kernel stop boundary.
use super::{ImportResult, NativeEntry, NativeError, NativeRegion};
use crate::native::runtime::{Function, Operation, RuntimeConfig};
use qa_core::heap::{Heap, MemoryError};
use std::sync::{Mutex, OnceLock};

struct Memory {
    base: u64,
    length: usize,
    regions: Box<[NativeRegion]>,
}
impl Memory {
    fn range(&self, address: u64, bytes: usize, rights: u8) -> Result<usize, NativeError> {
        let offset = usize::try_from(address.checked_sub(self.base).ok_or(NativeError::Extent)?)
            .map_err(|_| NativeError::Extent)?;
        let end = offset
            .checked_add(bytes)
            .filter(|&end| end <= self.length)
            .ok_or(NativeError::Extent)?;
        let mut at = offset;
        while at < end {
            let region = self
                .regions
                .partition_point(|r| r.offset <= at)
                .checked_sub(1)
                .and_then(|i| self.regions.get(i))
                .ok_or(NativeError::Extent)?;
            if region.permissions & rights != rights || at - region.offset >= region.length {
                return Err(NativeError::Extent);
            }
            at = end.min(region.offset + region.length);
        }
        Ok(offset)
    }
    fn read(&self, address: u64) -> Result<u8, NativeError> {
        self.range(address, 1, 1)?;
        // SAFETY: checked readable child-owned mapping. Only this child thread
        // executes; no reference escapes into a native callback or the parent.
        Ok(unsafe { (address as *const u8).read() })
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
    fn length(&self, address: u64) -> Result<usize, NativeError> {
        let mut count = 0;
        while self.read(
            address
                .checked_add(count as u64)
                .ok_or(NativeError::Extent)?,
        )? != 0
        {
            count += 1;
        }
        Ok(count)
    }
}
struct Runtime {
    memory: Memory,
    imports: Box<[Option<(&'static Function, NativeEntry)>]>,
    heap: Mutex<Option<Heap>>,
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
    let value = runtime.call(function.operation, arguments)?;
    Ok(Some(ImportResult {
        value: entry.result(value),
        kind: entry.control & 3,
    }))
}
impl Runtime {
    fn call(&self, operation: Operation, a: [u64; 13]) -> Result<u64, NativeError> {
        let size = |value| usize::try_from(value).map_err(|_| NativeError::Extent);
        let m = &self.memory;
        let x = f64::from_bits(a[0]);
        let y = f64::from_bits(a[1]);
        Ok(match operation {
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
                m.range(a[0], count, 2)?;
                let mut ended = false;
                for i in 0..count {
                    let byte = if ended {
                        0
                    } else {
                        m.read(a[1].checked_add(i as u64).ok_or(NativeError::Extent)?)?
                    };
                    ended |= byte == 0;
                    m.fill(a[0] + i as u64, byte, 1)?;
                }
                a[0]
            }
            Operation::Length => m.length(a[0])? as u64,
            Operation::CompareString | Operation::CompareMemory => {
                let string = matches!(operation, Operation::CompareString);
                let count = if string {
                    m.length(a[0])?.min(m.length(a[1])?) + 1
                } else {
                    size(a[2])?
                };
                m.range(a[0], count, 1)?;
                m.range(a[1], count, 1)?;
                let mut difference = 0;
                for i in 0..count {
                    difference =
                        i32::from(m.read(a[0] + i as u64)?) - i32::from(m.read(a[1] + i as u64)?);
                    if difference != 0 {
                        break;
                    }
                }
                difference as i64 as u64
            }
            Operation::Sin => x.sin().to_bits(),
            Operation::Cos => x.cos().to_bits(),
            Operation::Atan2 => x.atan2(y).to_bits(),
            Operation::Sqrt => x.sqrt().to_bits(),
            Operation::Floor => x.floor().to_bits(),
            Operation::Ceil => x.ceil().to_bits(),
            Operation::Acos => x.acos().to_bits(),
            Operation::Absolute => x.abs().to_bits(),
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
