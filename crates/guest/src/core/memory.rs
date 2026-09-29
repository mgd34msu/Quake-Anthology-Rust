//! Sparse guest memory: disjoint sorted mappings over shared backings.
//!
//! Donor: `src/guest/core/memory.ts` (`SparseGuestMemory`). Only mapped
//! regions allocate host bytes; high guest addresses never become host
//! indices. Aliases share backings, write observers see alias writes, and
//! executable-byte retentions invalidate when a mapping retires or any live
//! byte changes. The donor's `bigint` offsets are `u64` with explicit
//! wrapping at the pointer width; thrown faults become [`GuestError`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::math::Vec3;

use crate::core::contracts::{
    fresh_address_space, GuestAccess, GuestAddress, GuestAllocationOptions, GuestMapOptions,
    GuestMapping, GuestMemorySnapshot, GuestPermissions, GuestSnapshotMapping, GuestWrittenRange,
    ModuleIdentity,
};
use crate::error::GuestError;

/// Code-page size for executable retention tracking.
const PAGE_BYTES: usize = 4096;

/// Wrap `value` to the pointer width.
#[must_use]
pub fn wrap_guest_pointer(value: u64, pointer_bytes: usize) -> u64 {
    if pointer_bytes >= 8 {
        value
    } else {
        value & 0xffff_ffff
    }
}

/// Signed interpretation of a wrapped pointer.
#[must_use]
pub fn signed_guest_pointer(value: u64, pointer_bytes: usize) -> i64 {
    if pointer_bytes >= 8 {
        value as i64
    } else {
        (value & 0xffff_ffff) as u32 as i32 as i64
    }
}

/// Add `displacement` to a pointer with wraparound.
#[must_use]
pub fn add_guest_pointer(value: u64, displacement: i64, pointer_bytes: usize) -> u64 {
    wrap_guest_pointer(value.wrapping_add(displacement as u64), pointer_bytes)
}

type BackingRef = Rc<RefCell<BackingData>>;

#[derive(Debug)]
struct BackingData {
    bytes: Vec<u8>,
    exposed: bool,
    first_page: u32,
    last_page: i64,
    pages: HashMap<u32, u64>,
    next_revision: u64,
}

impl BackingData {
    fn new(byte_length: usize) -> Self {
        Self {
            bytes: vec![0; byte_length],
            exposed: false,
            first_page: u32::MAX,
            last_page: -1,
            pages: HashMap::new(),
            next_revision: 1,
        }
    }

    fn track_pages(&mut self, first: u32, last: u32) -> Vec<(u32, u64)> {
        let mut tracked = Vec::new();
        for page in first..=last {
            let revision = *self.pages.entry(page).or_insert_with(|| {
                let revision = self.next_revision;
                self.next_revision += 1;
                revision
            });
            tracked.push((page, revision));
        }
        self.first_page = self.first_page.min(first);
        self.last_page = self.last_page.max(i64::from(last));
        tracked
    }
}

#[derive(Debug, Clone)]
struct Mapping {
    id: u64,
    base: u64,
    byte_length: usize,
    permissions: GuestPermissions,
    label: String,
    backing: BackingRef,
    backing_start: usize,
    active: bool,
}

impl Mapping {
    fn end(&self) -> u128 {
        u128::from(self.base) + self.byte_length as u128
    }
}

#[derive(Debug, Clone)]
struct Chunk {
    mapping_id: usize,
    offset: usize,
    byte_length: usize,
}

/// Retention guard for decoded executable bytes. Retired mappings or any
/// live byte change invalidate it.
#[derive(Debug, Clone)]
pub struct ExecutableRetention {
    mapping_id: u64,
    offset: usize,
    expected: Vec<u8>,
    pages: Vec<(BackingRef, u32, u64)>,
    checked: RefCell<Option<u64>>,
}

impl ExecutableRetention {
    /// Revalidate against live memory.
    pub fn unchanged(&self, memory: &SparseGuestMemory) -> bool {
        let Some(mapping) = memory.find_mapping(self.mapping_id) else {
            return false;
        };
        if !mapping.active {
            return false;
        }
        let backing = mapping.backing.borrow();
        if !backing.exposed && self.pages.len() == 1 {
            let revision = backing.pages.get(&self.pages[0].1).copied();
            if revision == self.checked.borrow().as_ref().copied().or(Some(self.pages[0].2))
                && *self.checked.borrow() == revision
            {
                return true;
            }
        }
        let start = mapping.backing_start + self.offset;
        if start + self.expected.len() > backing.bytes.len() {
            return false;
        }
        if backing.bytes[start..start + self.expected.len()] != self.expected[..] {
            return false;
        }
        if self.pages.len() == 1 {
            *self.checked.borrow_mut() = backing.pages.get(&self.pages[0].1).copied();
        }
        true
    }
}

/// Retention for a decoded block: entry check plus a cheap post-store check.
/// Valid only after admission, while no host callback or borrowed-memory
/// owner can run.
#[derive(Debug, Clone)]
pub struct ExecutableBlock {
    retention: ExecutableRetention,
    pages: Vec<(BackingRef, u32, u64)>,
}

impl ExecutableBlock {
    /// Full entry check; refreshes the post-store revisions on success.
    pub fn unchanged(&mut self, memory: &SparseGuestMemory) -> bool {
        if !self.retention.unchanged(memory) {
            return false;
        }
        for (backing, page, checked) in &mut self.pages {
            *checked = backing
                .borrow()
                .pages
                .get(page)
                .copied()
                .unwrap_or(*checked);
        }
        true
    }

    /// Cheap check after managed stores: all tracked page revisions match.
    #[must_use]
    pub fn after_store(&self, memory: &SparseGuestMemory) -> bool {
        let Some(mapping) = memory.find_mapping(self.retention.mapping_id) else {
            return false;
        };
        mapping.active
            && self.pages.iter().all(|(backing, page, checked)| {
                backing.borrow().pages.get(page).copied() == Some(*checked)
            })
    }
}

struct WriteObserver {
    chunks: Vec<WatchedChunk>,
    notify: Box<dyn Fn(&[GuestWrittenRange])>,
}

/// Watched backing range, resolved at observe time so later mapping edits
/// cannot redirect the watch at a replacement backing.
struct WatchedChunk {
    backing: BackingRef,
    start: usize,
    byte_length: usize,
}

fn fault(
    reason: &'static str,
    address: u64,
    length: usize,
    access: GuestAccess,
    detail: impl Into<String>,
) -> GuestError {
    GuestError::memory_fault(reason, address, length, access.label(), detail)
}

/// Sparse guest memory over disjoint sorted mappings.
pub struct SparseGuestMemory {
    module: ModuleIdentity,
    space: u64,
    pointer_bytes: usize,
    limit: u128,
    allocation_base: u64,
    mappings: Vec<Mapping>,
    next_mapping_id: u64,
    generation: u64,
    allocation_hints: HashMap<u64, (u64, usize)>,
    recent: [Option<usize>; 4],
    working_set: [Option<usize>; 8],
    next_working: usize,
    lookup_offset: usize,
    observers: HashMap<u64, WriteObserver>,
    next_observer: u64,
}

impl SparseGuestMemory {
    /// Fresh memory for `module` with `pointer_bytes` (4 or 8).
    pub fn new(
        module: ModuleIdentity,
        pointer_bytes: usize,
        allocation_base: u64,
    ) -> Result<Self, GuestError> {
        if pointer_bytes != 4 && pointer_bytes != 8 {
            return Err(GuestError::invalid("Guest pointer width must be 4 or 8 bytes"));
        }
        let limit = 1u128 << (pointer_bytes * 8);
        let memory = Self {
            module,
            space: fresh_address_space(),
            pointer_bytes,
            limit,
            allocation_base,
            mappings: Vec::new(),
            next_mapping_id: 1,
            generation: 0,
            allocation_hints: HashMap::new(),
            recent: [None, None, None, None],
            working_set: [None; 8],
            next_working: 0,
            lookup_offset: 0,
            observers: HashMap::new(),
            next_observer: 1,
        };
        memory.check_range(allocation_base, 0, "map")?;
        Ok(memory)
    }

    /// Owning module.
    #[must_use]
    pub fn module(&self) -> &ModuleIdentity {
        &self.module
    }

    /// Address-space token.
    #[must_use]
    pub const fn address_space(&self) -> u64 {
        self.space
    }

    /// Pointer width in bytes.
    #[must_use]
    pub const fn pointer_bytes(&self) -> usize {
        self.pointer_bytes
    }

    /// Whether any write observer is installed.
    #[must_use]
    pub fn has_write_observers(&self) -> bool {
        !self.observers.is_empty()
    }

    /// Decode the source's null representation before exposing an address.
    pub fn pointer(&self, raw: u64) -> Result<Option<GuestAddress>, GuestError> {
        if raw == 0 {
            return Ok(None);
        }
        self.check_range(raw, 0, "read")?;
        Ok(Some(GuestAddress::new(self.space, raw)))
    }

    /// Offset an owned address by a signed displacement.
    pub fn offset(&self, address: GuestAddress, displacement: i64) -> Result<GuestAddress, GuestError> {
        self.check_owned(address, 0, "read")?;
        let result = address.offset.wrapping_add(displacement as u64);
        // Detect wraparound past the address-space limit.
        if displacement >= 0 && result < address.offset {
            return Err(fault(
                "address-overflow",
                result,
                0,
                GuestAccess::Read,
                format!("range exceeds {}-bit address space", self.pointer_bytes * 8),
            ));
        }
        self.check_range(result, 0, "read")?;
        Ok(GuestAddress::new(self.space, result))
    }

    /// Map a fresh zero-filled range, optionally with initial bytes.
    pub fn map(&mut self, options: &GuestMapOptions) -> Result<GuestAddress, GuestError> {
        self.check_available(options.base, options.byte_length)?;
        if options
            .bytes
            .as_ref()
            .is_some_and(|bytes| bytes.len() > options.byte_length)
        {
            return Err(GuestError::invalid("Initial guest bytes exceed the mapping length"));
        }
        let backing = Rc::new(RefCell::new(BackingData::new(options.byte_length)));
        if let Some(bytes) = &options.bytes {
            backing.borrow_mut().bytes[..bytes.len()].copy_from_slice(bytes);
        }
        self.insert(
            options.base,
            options.byte_length,
            options.permissions,
            options.label.clone(),
            backing,
            0,
        );
        Ok(GuestAddress::new(self.space, options.base))
    }

    /// Allocate a fresh range at or after the allocation base.
    pub fn allocate(&mut self, options: &GuestAllocationOptions) -> Result<GuestAddress, GuestError> {
        let alignment = options.alignment;
        if alignment == 0 || alignment & (alignment - 1) != 0 {
            return Err(GuestError::invalid(
                "Guest allocation alignment must be a positive power of two",
            ));
        }
        self.check_range(self.allocation_base, options.byte_length, "map")?;
        if options.byte_length == 0 {
            return Err(GuestError::invalid(
                "Guest allocations must contain at least one byte",
            ));
        }
        let start = match self.allocation_hints.get(&alignment) {
            Some((base, hint_len)) if options.byte_length >= *hint_len => *base,
            _ => self.allocation_base,
        };
        let mut base = start.next_multiple_of(alignment);
        let length = options.byte_length as u64;
        let mut index = self.first_end_after(base);
        while index < self.mappings.len() {
            let mapping = &self.mappings[index];
            if base.saturating_add(length) <= mapping.base {
                break;
            }
            if u128::from(base) < mapping.end() {
                // A mapping ending past u64::MAX is necessarily last, so no
                // later gap can fit once alignment leaves the address space.
                let next = mapping.end().next_multiple_of(u128::from(alignment));
                if next > u128::from(u64::MAX) {
                    break;
                }
                base = next as u64;
            }
            index = self.first_end_after(base);
        }
        let address = self.map(&GuestMapOptions {
            base,
            byte_length: options.byte_length,
            permissions: options.permissions,
            label: options.label.clone(),
            bytes: None,
        })?;
        self.allocation_hints
            .insert(alignment, (base.saturating_add(length), options.byte_length));
        Ok(address)
    }

    /// Map an alias view over an already-mapped source range.
    pub fn map_alias(
        &mut self,
        base: u64,
        byte_length: usize,
        permissions: GuestPermissions,
        label: &str,
        source: GuestAddress,
    ) -> Result<GuestAddress, GuestError> {
        self.check_available(base, byte_length)?;
        let chunks = self.chunks(source, byte_length, None)?;
        let (backing, backing_start) = self.contiguous(&chunks, source, byte_length)?;
        self.insert(
            base,
            byte_length,
            permissions,
            label.to_string(),
            backing,
            backing_start,
        );
        Ok(GuestAddress::new(self.space, base))
    }

    /// Unmap a fully-mapped range.
    pub fn unmap(&mut self, address: GuestAddress, byte_length: usize) -> Result<(), GuestError> {
        self.chunks(address, byte_length, None)?;
        self.replace_range(address.offset, byte_length, None);
        Ok(())
    }

    /// Change permissions over a fully-mapped range.
    pub fn protect(
        &mut self,
        address: GuestAddress,
        byte_length: usize,
        permissions: GuestPermissions,
    ) -> Result<(), GuestError> {
        self.chunks(address, byte_length, None)?;
        self.replace_range(address.offset, byte_length, Some(permissions));
        Ok(())
    }

    /// Snapshot of the current mappings.
    #[must_use]
    pub fn mappings(&self) -> Vec<GuestMapping> {
        self.mappings
            .iter()
            .map(|mapping| GuestMapping {
                base: mapping.base,
                byte_length: mapping.byte_length,
                permissions: mapping.permissions,
                label: mapping.label.clone(),
            })
            .collect()
    }

    /// Check that a range is mapped with `access`.
    pub fn check(
        &mut self,
        address: GuestAddress,
        byte_length: usize,
        access: GuestAccess,
    ) -> Result<(), GuestError> {
        if self.single_mapping(address, byte_length, access)?.is_none() {
            self.chunks(address, byte_length, Some(access))?;
        }
        Ok(())
    }

    /// Copy a range out. The full source range is checked first.
    pub fn copy(&mut self, address: GuestAddress, byte_length: usize) -> Result<Vec<u8>, GuestError> {
        if let Some(index) = self.single_mapping(address, byte_length, GuestAccess::Read)? {
            let offset = self.lookup_offset;
            let mapping = &self.mappings[index];
            let backing = mapping.backing.borrow();
            return Ok(backing.bytes[mapping.backing_start + offset
                ..mapping.backing_start + offset + byte_length]
                .to_vec());
        }
        let chunks = self.chunks(address, byte_length, Some(GuestAccess::Read))?;
        Ok(self.copy_chunks(&chunks, byte_length))
    }

    /// Copy a checked range into existing host storage.
    pub fn copy_into(
        &mut self,
        address: GuestAddress,
        destination: &mut [u8],
        destination_offset: usize,
        byte_length: usize,
    ) -> Result<(), GuestError> {
        if destination_offset + byte_length > destination.len() {
            return Err(GuestError::invalid("Guest copy exceeds destination storage"));
        }
        if let Some(index) = self.single_mapping(address, byte_length, GuestAccess::Read)? {
            let offset = self.lookup_offset;
            let mapping = &self.mappings[index].clone();
            let backing = mapping.backing.borrow();
            destination[destination_offset..destination_offset + byte_length]
                .copy_from_slice(
                    &backing.bytes[mapping.backing_start + offset
                        ..mapping.backing_start + offset + byte_length],
                );
            return Ok(());
        }
        let chunks = self.chunks(address, byte_length, Some(GuestAccess::Read))?;
        let bytes = self.copy_chunks(&chunks, byte_length);
        destination[destination_offset..destination_offset + byte_length].copy_from_slice(&bytes);
        Ok(())
    }

    /// Find a zero terminator without probing past it.
    pub fn find_zero(&mut self, address: GuestAddress, maximum: usize) -> Result<isize, GuestError> {
        let mut consumed = 0usize;
        let mut cursor = address;
        while consumed < maximum {
            let index = match self.single_mapping(cursor, 1, GuestAccess::Read)? {
                Some(index) => index,
                None => {
                    return Err(fault(
                        "unmapped",
                        cursor.offset,
                        1,
                        GuestAccess::Read,
                        "range includes unmapped bytes",
                    ));
                }
            };
            let offset = self.lookup_offset;
            let mapping = &self.mappings[index];
            let length = (maximum - consumed).min(mapping.byte_length - offset);
            let backing = mapping.backing.borrow();
            let window = &backing.bytes
                [mapping.backing_start + offset..mapping.backing_start + offset + length];
            if let Some(terminator) = window.iter().position(|byte| *byte == 0) {
                return Ok((consumed + terminator) as isize);
            }
            consumed += length;
            if consumed < maximum {
                cursor = self.offset(address, consumed as i64)?;
            }
        }
        Ok(-1)
    }

    /// Fetch instruction bytes with execute permission.
    pub fn fetch(&mut self, address: GuestAddress, byte_length: usize) -> Result<Vec<u8>, GuestError> {
        let chunks = self.chunks(address, byte_length, Some(GuestAccess::Execute))?;
        Ok(self.copy_chunks(&chunks, byte_length))
    }

    /// Execute one live byte at this memory owner's processor IP.
    pub fn fetch_byte(&mut self, byte_offset: u64) -> Result<u8, GuestError> {
        let index = self.execute_mapping(byte_offset)?;
        let mapping = &self.mappings[index];
        let offset = (byte_offset - mapping.base) as usize;
        Ok(mapping.backing.borrow().bytes[mapping.backing_start + offset])
    }

    /// Fetch one byte, tracking a cursor for sequential decode.
    pub fn fetch_sequence_byte(&mut self, cursor: &mut FetchCursor) -> Result<u8, GuestError> {
        if cursor.generation != self.generation
            || cursor.mapping.map_or(true, |id| {
                self.find_mapping(id).map_or(true, |mapping| {
                    cursor.offset >= mapping.byte_length
                })
            })
        {
            let address = cursor
                .base
                .checked_add(cursor.consumed)
                .ok_or_else(|| {
                    fault(
                        "address-overflow",
                        cursor.base,
                        1,
                        GuestAccess::Execute,
                        "fetch sequence overflowed",
                    )
                })?;
            let byte = self.fetch_byte(address)?;
            let mapping = self.recent[GuestAccess::Execute as usize]
                .and_then(|index| self.mappings.get(index))
                .ok_or_else(|| GuestError::cpu("Guest execute mapping is missing"))?;
            cursor.mapping = Some(mapping.id);
            cursor.offset = (address - mapping.base) as usize + 1;
            cursor.generation = self.generation;
            cursor.consumed += 1;
            return Ok(byte);
        }
        let id = cursor.mapping.ok_or_else(|| GuestError::cpu("Guest execute mapping is missing"))?;
        let mapping = self.find_mapping(id).ok_or_else(|| GuestError::cpu("Guest execute mapping is missing"))?.clone();
        let byte = mapping.backing.borrow().bytes[mapping.backing_start + cursor.offset];
        cursor.offset += 1;
        cursor.consumed += 1;
        Ok(byte)
    }

    /// Cache guard for decoded bytes (at most 15). Retired mappings or any
    /// live byte change invalidate it.
    pub fn retain_executable_bytes(
        &mut self,
        byte_offset: u64,
        bytes: &[u8],
    ) -> Option<ExecutableRetention> {
        if bytes.is_empty() || bytes.len() > 15 {
            return None;
        }
        self.retain_executable_range(byte_offset, bytes)
    }

    /// Retention guard for a decoded range of any length.
    pub fn retain_executable_range(
        &mut self,
        byte_offset: u64,
        bytes: &[u8],
    ) -> Option<ExecutableRetention> {
        if bytes.is_empty() {
            return None;
        }
        let index = self.first_end_after(byte_offset);
        let mapping = self.mappings.get(index)?.clone();
        if byte_offset < mapping.base || !mapping.permissions.allows(GuestAccess::Execute) {
            return None;
        }
        let offset = (byte_offset - mapping.base) as usize;
        if offset + bytes.len() > mapping.byte_length {
            return None;
        }
        let start = mapping.backing_start + offset;
        let first = (start / PAGE_BYTES) as u32;
        let last = ((start + bytes.len() - 1) / PAGE_BYTES) as u32;
        let mut pages = Vec::new();
        {
            let mut backing = mapping.backing.borrow_mut();
            if !backing.exposed && first == last {
                pages.push((mapping.backing.clone(), first, backing.track_pages(first, last)[0].1));
            }
        }
        let retention = ExecutableRetention {
            mapping_id: mapping.id,
            offset,
            expected: bytes.to_vec(),
            pages,
            checked: RefCell::new(None),
        };
        retention.unchanged(self).then_some(retention)
    }

    /// Block retention: entry check plus cheap post-store checks.
    pub fn retain_executable_block(
        &mut self,
        byte_offset: u64,
        bytes: &[u8],
    ) -> Option<ExecutableBlock> {
        let retention = self.retain_executable_range(byte_offset, bytes)?;
        let index = self.first_end_after(byte_offset);
        let mapping = self.mappings.get(index)?.clone();
        let start = mapping.backing_start + (byte_offset - mapping.base) as usize;
        let first = (start / PAGE_BYTES) as u32;
        let last = ((start + bytes.len() - 1) / PAGE_BYTES) as u32;
        let tracked = mapping.backing.borrow_mut().track_pages(first, last);
        let pages = tracked
            .into_iter()
            .map(|(page, revision)| (mapping.backing.clone(), page, revision))
            .collect();
        Some(ExecutableBlock { retention, pages })
    }

    /// Store bytes; the full destination range is checked before the first
    /// store, and aliasing sources are snapshotted first.
    pub fn write(&mut self, address: GuestAddress, bytes: &[u8]) -> Result<(), GuestError> {
        if let Some(index) = self.single_mapping(address, bytes.len(), GuestAccess::Write)? {
            let offset = self.lookup_offset;
            let mapping = self.mappings[index].clone();
            {
                let mut backing = mapping.backing.borrow_mut();
                let start = mapping.backing_start + offset;
                backing.bytes[start..start + bytes.len()].copy_from_slice(bytes);
            }
            self.invalidate_code(&mapping, offset, bytes.len());
            let chunk = Chunk {
                mapping_id: index,
                offset,
                byte_length: bytes.len(),
            };
            return self.notify_write(std::slice::from_ref(&chunk));
        }
        let chunks = self.chunks(address, bytes.len(), Some(GuestAccess::Write))?;
        let snapshot = bytes.to_vec();
        self.commit_write(&chunks, &snapshot)
    }

    /// Copy guest bytes to guest bytes, handling overlap.
    pub fn move_bytes(
        &mut self,
        destination: GuestAddress,
        source: GuestAddress,
        byte_length: usize,
    ) -> Result<(), GuestError> {
        let snapshot = self.copy(source, byte_length)?;
        self.write(destination, &snapshot)
    }

    /// Fill a range with one byte value.
    pub fn fill(
        &mut self,
        address: GuestAddress,
        byte_length: usize,
        value: u8,
    ) -> Result<(), GuestError> {
        if let Some(index) = self.single_mapping(address, byte_length, GuestAccess::Write)? {
            let offset = self.lookup_offset;
            let mapping = self.mappings[index].clone();
            {
                let mut backing = mapping.backing.borrow_mut();
                let start = mapping.backing_start + offset;
                backing.bytes[start..start + byte_length].fill(value);
            }
            self.invalidate_code(&mapping, offset, byte_length);
            let chunk = Chunk {
                mapping_id: index,
                offset,
                byte_length,
            };
            return self.notify_write(std::slice::from_ref(&chunk));
        }
        let chunks = self.chunks(address, byte_length, Some(GuestAccess::Write))?;
        for chunk in &chunks {
            let mapping = self.mappings[chunk.mapping_id].clone();
            {
                let mut backing = mapping.backing.borrow_mut();
                let start = mapping.backing_start + chunk.offset;
                backing.bytes[start..start + chunk.byte_length].fill(value);
            }
            self.invalidate_code(&mapping, chunk.offset, chunk.byte_length);
        }
        self.notify_write(&chunks)
    }

    /// Observe committed stores to a range, including writes through
    /// aliases. Returns an observer id for [`Self::unobserve`].
    pub fn observe_writes(
        &mut self,
        address: GuestAddress,
        byte_length: usize,
        after_write: Box<dyn Fn(&[GuestWrittenRange])>,
    ) -> Result<u64, GuestError> {
        let chunks = self.chunks(address, byte_length, Some(GuestAccess::Read))?;
        let watched = chunks
            .iter()
            .map(|chunk| {
                let mapping = &self.mappings[chunk.mapping_id];
                WatchedChunk {
                    backing: Rc::clone(&mapping.backing),
                    start: mapping.backing_start + chunk.offset,
                    byte_length: chunk.byte_length,
                }
            })
            .collect();
        let id = self.next_observer;
        self.next_observer += 1;
        self.observers.insert(id, WriteObserver { chunks: watched, notify: after_write });
        Ok(id)
    }

    /// Remove a write observer.
    pub fn unobserve(&mut self, id: u64) {
        self.observers.remove(&id);
    }

    /// Scalar loads.
    pub fn read_u8(&mut self, address: GuestAddress) -> Result<u8, GuestError> {
        Ok(self.read_scalar(address, 1)?[0])
    }
    /// Scalar loads.
    pub fn read_i8(&mut self, address: GuestAddress) -> Result<i8, GuestError> {
        Ok(self.read_scalar(address, 1)?[0] as i8)
    }
    /// Scalar loads.
    pub fn read_u16(&mut self, address: GuestAddress) -> Result<u16, GuestError> {
        let bytes = self.read_scalar(address, 2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }
    /// Scalar loads.
    pub fn read_i16(&mut self, address: GuestAddress) -> Result<i16, GuestError> {
        let bytes = self.read_scalar(address, 2)?;
        Ok(i16::from_le_bytes([bytes[0], bytes[1]]))
    }
    /// Scalar loads.
    pub fn read_u32(&mut self, address: GuestAddress) -> Result<u32, GuestError> {
        let bytes = self.read_scalar(address, 4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    /// Scalar loads.
    pub fn read_i32(&mut self, address: GuestAddress) -> Result<i32, GuestError> {
        let bytes = self.read_scalar(address, 4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    /// Scalar loads.
    pub fn read_u64(&mut self, address: GuestAddress) -> Result<u64, GuestError> {
        let bytes = self.read_scalar(address, 8)?;
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes);
        Ok(u64::from_le_bytes(word))
    }
    /// Scalar loads.
    pub fn read_i64(&mut self, address: GuestAddress) -> Result<i64, GuestError> {
        let bytes = self.read_scalar(address, 8)?;
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes);
        Ok(i64::from_le_bytes(word))
    }
    /// Scalar loads.
    pub fn read_u64_words(&mut self, address: GuestAddress) -> Result<(u32, u32), GuestError> {
        let bytes = self.read_scalar(address, 8)?;
        Ok((
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        ))
    }
    /// Scalar loads.
    pub fn read_f32(&mut self, address: GuestAddress) -> Result<f32, GuestError> {
        let bytes = self.read_scalar(address, 4)?;
        Ok(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    /// Scalar loads.
    pub fn read_f32x3(&mut self, address: GuestAddress) -> Result<Vec3, GuestError> {
        let bytes = self.read_scalar(address, 12)?;
        Ok(Vec3 {
            x: f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            y: f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            z: f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        })
    }
    /// Scalar loads.
    pub fn read_f64(&mut self, address: GuestAddress) -> Result<f64, GuestError> {
        let bytes = self.read_scalar(address, 8)?;
        let mut word = [0u8; 8];
        word.copy_from_slice(&bytes);
        Ok(f64::from_le_bytes(word))
    }
    /// Decode a stored pointer, mapping null to `None`.
    pub fn read_pointer(&mut self, address: GuestAddress) -> Result<Option<GuestAddress>, GuestError> {
        let raw = if self.pointer_bytes == 4 {
            u64::from(self.read_u32(address)?)
        } else {
            self.read_u64(address)?
        };
        self.pointer(raw)
    }

    /// Scalar stores.
    pub fn write_u8(&mut self, address: GuestAddress, value: u8) -> Result<(), GuestError> {
        self.write(address, &[value])
    }
    /// Scalar stores.
    pub fn write_i8(&mut self, address: GuestAddress, value: i8) -> Result<(), GuestError> {
        self.write(address, &[value as u8])
    }
    /// Scalar stores.
    pub fn write_u16(&mut self, address: GuestAddress, value: u16) -> Result<(), GuestError> {
        self.write(address, &value.to_le_bytes())
    }
    /// Scalar stores.
    pub fn write_i16(&mut self, address: GuestAddress, value: i16) -> Result<(), GuestError> {
        self.write(address, &value.to_le_bytes())
    }
    /// Scalar stores.
    pub fn write_u32(&mut self, address: GuestAddress, value: u32) -> Result<(), GuestError> {
        self.write(address, &value.to_le_bytes())
    }
    /// Scalar stores.
    pub fn write_i32(&mut self, address: GuestAddress, value: i32) -> Result<(), GuestError> {
        self.write(address, &value.to_le_bytes())
    }
    /// Scalar stores.
    pub fn write_u64(&mut self, address: GuestAddress, value: u64) -> Result<(), GuestError> {
        self.write(address, &value.to_le_bytes())
    }
    /// Scalar stores.
    pub fn write_i64(&mut self, address: GuestAddress, value: i64) -> Result<(), GuestError> {
        self.write(address, &value.to_le_bytes())
    }
    /// Scalar stores.
    pub fn write_u64_words(
        &mut self,
        address: GuestAddress,
        low: u32,
        high: u32,
    ) -> Result<(), GuestError> {
        let mut bytes = [0u8; 8];
        bytes[..4].copy_from_slice(&low.to_le_bytes());
        bytes[4..].copy_from_slice(&high.to_le_bytes());
        self.write(address, &bytes)
    }
    /// Scalar stores.
    pub fn write_f32(&mut self, address: GuestAddress, value: f32) -> Result<(), GuestError> {
        self.write(address, &value.to_le_bytes())
    }
    /// Scalar stores.
    pub fn write_f64(&mut self, address: GuestAddress, value: f64) -> Result<(), GuestError> {
        self.write(address, &value.to_le_bytes())
    }
    /// Encode a pointer, mapping `None` to null.
    pub fn write_pointer(
        &mut self,
        address: GuestAddress,
        value: Option<GuestAddress>,
    ) -> Result<(), GuestError> {
        if let Some(target) = value {
            self.check_owned(target, 0, "write")?;
        }
        let raw = value.map_or(0, |target| target.offset);
        if self.pointer_bytes == 4 {
            self.write_u32(address, raw as u32)
        } else {
            self.write_u64(address, raw)
        }
    }

    /// Snapshot memory, preserving alias relationships between mappings.
    #[must_use]
    pub fn checkpoint(&self) -> GuestMemorySnapshot {
        let mut backings: Vec<Vec<u8>> = Vec::new();
        let mut indices: HashMap<usize, usize> = HashMap::new();
        let mut mappings = Vec::with_capacity(self.mappings.len());
        for mapping in &self.mappings {
            let key = Rc::as_ptr(&mapping.backing) as usize;
            let backing = *indices.entry(key).or_insert_with(|| {
                let index = backings.len();
                backings.push(mapping.backing.borrow().bytes.clone());
                index
            });
            mappings.push(GuestSnapshotMapping {
                base: mapping.base,
                byte_length: mapping.byte_length,
                permissions: mapping.permissions,
                label: mapping.label.clone(),
                backing,
                backing_offset: mapping.backing_start,
            });
        }
        GuestMemorySnapshot {
            module: self.module.clone(),
            pointer_bytes: self.pointer_bytes,
            allocation_base: self.allocation_base,
            backings,
            mappings,
        }
    }

    /// Restore a snapshot. Restored pointers get a fresh address-space
    /// token; raw offsets and alias relationships survive.
    pub fn restore(
        module: ModuleIdentity,
        snapshot: &GuestMemorySnapshot,
    ) -> Result<Self, GuestError> {
        if module.id != snapshot.module.id
            || module.digest != snapshot.module.digest
            || module.revision != snapshot.module.revision
        {
            return Err(GuestError::BadSave(
                "Guest snapshot belongs to a different module artifact".to_string(),
            ));
        }
        let mut memory = Self::new(module, snapshot.pointer_bytes, snapshot.allocation_base)?;
        let backings: Vec<BackingRef> = snapshot
            .backings
            .iter()
            .map(|bytes| {
                Rc::new(RefCell::new(BackingData {
                    bytes: bytes.clone(),
                    exposed: false,
                    first_page: u32::MAX,
                    last_page: -1,
                    pages: HashMap::new(),
                    next_revision: 1,
                }))
            })
            .collect();
        for mapping in &snapshot.mappings {
            memory.check_available(mapping.base, mapping.byte_length)?;
            let backing = backings.get(mapping.backing).ok_or_else(|| {
                GuestError::invalid("Invalid guest snapshot backing range")
            })?;
            if mapping.backing_offset + mapping.byte_length > backing.borrow().bytes.len() {
                return Err(GuestError::invalid("Invalid guest snapshot backing range"));
            }
            memory.insert(
                mapping.base,
                mapping.byte_length,
                mapping.permissions,
                mapping.label.clone(),
                Rc::clone(backing),
                mapping.backing_offset,
            );
        }
        Ok(memory)
    }

    fn find_mapping(&self, id: u64) -> Option<&Mapping> {
        self.mappings.iter().find(|mapping| mapping.id == id)
    }

    fn check_range(&self, base: u64, byte_length: usize, access: &'static str) -> Result<(), GuestError> {
        let access = match access {
            "read" => GuestAccess::Read,
            "write" => GuestAccess::Write,
            "execute" => GuestAccess::Execute,
            _ => GuestAccess::Read,
        };
        if base == 0 {
            return Err(fault("null-address", base, byte_length, access, "null is not a mapped address"));
        }
        if u128::from(base) >= self.limit
            || (byte_length != 0 && u128::from(base) + byte_length as u128 > self.limit)
        {
            return Err(fault(
                "address-overflow",
                base,
                byte_length,
                access,
                format!("range exceeds {}-bit address space", self.pointer_bytes * 8),
            ));
        }
        Ok(())
    }

    fn check_owned(
        &self,
        address: GuestAddress,
        byte_length: usize,
        access: &'static str,
    ) -> Result<(), GuestError> {
        if address.space != self.space {
            let access = match access {
                "write" => GuestAccess::Write,
                "execute" => GuestAccess::Execute,
                _ => GuestAccess::Read,
            };
            return Err(fault(
                "foreign-address-space",
                address.offset,
                byte_length,
                access,
                "pointer belongs to another execution owner",
            ));
        }
        self.check_range(address.offset, byte_length, access)
    }

    fn check_available(&self, base: u64, byte_length: usize) -> Result<(), GuestError> {
        self.check_range(base, byte_length, "map")?;
        if byte_length == 0 {
            return Err(fault(
                "invalid-length",
                base,
                byte_length,
                GuestAccess::Read,
                "mapping must contain at least one byte",
            ));
        }
        let end = u128::from(base) + byte_length as u128;
        if let Some(overlapping) = self.mappings.get(self.first_end_after(base)) {
            if u128::from(overlapping.base) < end {
                return Err(fault(
                    "overlap",
                    base,
                    byte_length,
                    GuestAccess::Read,
                    "mapping overlaps an existing guest range",
                ));
            }
        }
        Ok(())
    }

    fn insert(
        &mut self,
        base: u64,
        byte_length: usize,
        permissions: GuestPermissions,
        label: String,
        backing: BackingRef,
        backing_start: usize,
    ) {
        let id = self.next_mapping_id;
        self.next_mapping_id += 1;
        self.generation += 1;
        let index = self.first_end_after(base);
        self.mappings.insert(
            index,
            Mapping {
                id,
                base,
                byte_length,
                permissions,
                label,
                backing,
                backing_start,
                active: true,
            },
        );
    }

    fn first_end_after(&self, address: u64) -> usize {
        let (mut low, mut high) = (0, self.mappings.len());
        while low < high {
            let middle = (low + high) / 2;
            if self.mappings[middle].end() <= u128::from(address) {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low
    }

    fn access_slot(access: Option<GuestAccess>) -> usize {
        match access {
            Some(GuestAccess::Read) => 0,
            Some(GuestAccess::Write) => 1,
            Some(GuestAccess::Execute) => 2,
            None => 3,
        }
    }

    fn chunks(
        &mut self,
        address: GuestAddress,
        byte_length: usize,
        access: Option<GuestAccess>,
    ) -> Result<Vec<Chunk>, GuestError> {
        let label = access.map_or("read", GuestAccess::label);
        self.check_owned(address, byte_length, label)?;
        let slot = Self::access_slot(access);
        if byte_length > 0 {
            if let Some(index) = self.recent[slot] {
                if let Some(recent) = self.mappings.get(index) {
                    if recent.active
                        && address.offset >= recent.base
                        && u128::from(address.offset) + byte_length as u128 <= recent.end()
                    {
                        if let Some(access) = access {
                            if !recent.permissions.allows(access) {
                                return Err(fault(
                                    "permission",
                                    address.offset,
                                    byte_length,
                                    access,
                                    format!(
                                        "mapping '{}' permits {}",
                                        recent.label,
                                        recent.permissions.label()
                                    ),
                                ));
                            }
                        }
                        return Ok(vec![Chunk {
                            mapping_id: index,
                            offset: (address.offset - recent.base) as usize,
                            byte_length,
                        }]);
                    }
                }
            }
        }
        let mut chunks = Vec::new();
        let mut cursor = address.offset;
        let mut remaining = byte_length;
        let low = self.first_end_after(cursor);
        for index in low..self.mappings.len() {
            if remaining == 0 {
                break;
            }
            let mapping = self.mappings[index].clone();
            if u128::from(cursor) >= mapping.end() {
                continue;
            }
            if cursor < mapping.base {
                break;
            }
            if let Some(access) = access {
                if !mapping.permissions.allows(access) {
                    return Err(fault(
                        "permission",
                        cursor,
                        remaining,
                        access,
                        format!(
                            "mapping '{}' permits {}",
                            mapping.label,
                            mapping.permissions.label()
                        ),
                    ));
                }
            }
            let offset = (cursor - mapping.base) as usize;
            let length = remaining.min(mapping.byte_length - offset);
            chunks.push(Chunk {
                mapping_id: index,
                offset,
                byte_length: length,
            });
            self.remember(access, index);
            remaining -= length;
            // The range check admits ends at exactly 2^64; saturation only
            // bites when `remaining` already reached zero.
            cursor = cursor.saturating_add(length as u64);
        }
        if remaining != 0 {
            return Err(fault(
                "unmapped",
                cursor,
                remaining,
                access.unwrap_or(GuestAccess::Read),
                "range includes unmapped bytes",
            ));
        }
        Ok(chunks)
    }

    fn contiguous(
        &self,
        chunks: &[Chunk],
        address: GuestAddress,
        byte_length: usize,
    ) -> Result<(BackingRef, usize), GuestError> {
        let Some(first) = chunks.first() else {
            return Ok((Rc::new(RefCell::new(BackingData::new(0))), 0));
        };
        let mapping = &self.mappings[first.mapping_id];
        let backing = Rc::clone(&mapping.backing);
        let start = mapping.backing_start + first.offset;
        let mut consumed = 0;
        for chunk in chunks {
            let mapping = &self.mappings[chunk.mapping_id];
            if !Rc::ptr_eq(&mapping.backing, &backing)
                || mapping.backing_start + chunk.offset != start + consumed
            {
                return Err(fault(
                    "noncontiguous-borrow",
                    address.offset,
                    byte_length,
                    GuestAccess::Read,
                    "a live view requires contiguous backing storage; use checked loads or copy for fragmented ranges",
                ));
            }
            consumed += chunk.byte_length;
        }
        Ok((backing, start))
    }

    fn copy_chunks(&self, chunks: &[Chunk], byte_length: usize) -> Vec<u8> {
        let mut result = vec![0; byte_length];
        let mut consumed = 0;
        for chunk in chunks {
            let mapping = &self.mappings[chunk.mapping_id];
            let backing = mapping.backing.borrow();
            let start = mapping.backing_start + chunk.offset;
            result[consumed..consumed + chunk.byte_length]
                .copy_from_slice(&backing.bytes[start..start + chunk.byte_length]);
            consumed += chunk.byte_length;
        }
        result
    }

    #[allow(clippy::too_many_lines)]
    fn replace_range(&mut self, base: u64, byte_length: usize, permissions: Option<GuestPermissions>) {
        if byte_length == 0 {
            return;
        }
        let end = u128::from(base) + byte_length as u128;
        let first = self.first_end_after(base);
        let mut next: Vec<Mapping> = Vec::new();
        let mut last = first;
        while last < self.mappings.len() {
            if u128::from(self.mappings[last].base) >= end {
                break;
            }
            self.mappings[last].active = false;
            let mapping = self.mappings[last].clone();
            let mapping_end = mapping.end();
            let start_offset = (base.max(mapping.base) - mapping.base) as usize;
            let end_offset = (end.min(mapping_end) - u128::from(mapping.base)) as usize;
            if start_offset > 0 {
                next.push(Mapping {
                    id: self.fresh_mapping_id(),
                    base: mapping.base,
                    byte_length: start_offset,
                    backing_start: mapping.backing_start,
                    ..mapping.clone()
                });
            }
            if let Some(permissions) = permissions {
                next.push(Mapping {
                    id: self.fresh_mapping_id(),
                    base: mapping.base + start_offset as u64,
                    byte_length: end_offset - start_offset,
                    permissions,
                    backing_start: mapping.backing_start + start_offset,
                    ..mapping.clone()
                });
            }
            if end_offset < mapping.byte_length {
                next.push(Mapping {
                    id: self.fresh_mapping_id(),
                    base: mapping.base + end_offset as u64,
                    byte_length: mapping.byte_length - end_offset,
                    backing_start: mapping.backing_start + end_offset,
                    ..mapping.clone()
                });
            }
            last += 1;
        }
        if permissions.is_none() {
            let touched = self.mappings.get(first).cloned();
            let previous = if first > 0 {
                self.mappings.get(first - 1).cloned()
            } else {
                None
            };
            let gap_start = match (&touched, &previous) {
                (Some(touched), _) if touched.base < base => base,
                (_, None) => self.allocation_base,
                (_, Some(previous)) => previous.end().min(u128::from(u64::MAX)) as u64,
            };
            let lower_bound = gap_start.max(self.allocation_base);
            for hint in self.allocation_hints.values_mut() {
                if lower_bound < hint.0 {
                    hint.0 = lower_bound;
                }
            }
        }
        self.generation += 1;
        self.mappings.splice(first..last, next);
    }

    fn fresh_mapping_id(&mut self) -> u64 {
        let id = self.next_mapping_id;
        self.next_mapping_id += 1;
        id
    }

    fn remember(&mut self, access: Option<GuestAccess>, index: usize) {
        self.recent[Self::access_slot(access)] = Some(index);
        if self.working_set.contains(&Some(index)) {
            return;
        }
        self.working_set[self.next_working] = Some(index);
        self.next_working = (self.next_working + 1) & 7;
    }

    fn single_mapping(
        &mut self,
        address: GuestAddress,
        byte_length: usize,
        access: GuestAccess,
    ) -> Result<Option<usize>, GuestError> {
        if address.space == self.space && byte_length > 0 {
            let slot = Self::access_slot(Some(access));
            if let Some(index) = self.recent[slot] {
                if let Some(recent) = self.mappings.get(index).cloned() {
                    if recent.active
                        && address.offset >= recent.base
                        && u128::from(address.offset) + byte_length as u128 <= recent.end()
                    {
                        if !recent.permissions.allows(access) {
                            return Err(fault(
                                "permission",
                                address.offset,
                                byte_length,
                                access,
                                format!(
                                    "mapping '{}' permits {}",
                                    recent.label,
                                    recent.permissions.label()
                                ),
                            ));
                        }
                        self.lookup_offset = (address.offset - recent.base) as usize;
                        return Ok(Some(index));
                    }
                }
            }
            for candidate in self.working_set.into_iter().flatten() {
                if Some(candidate) == self.recent[slot] {
                    continue;
                }
                let Some(mapping) = self.mappings.get(candidate).cloned() else {
                    continue;
                };
                if !mapping.active
                    || address.offset < mapping.base
                    || u128::from(address.offset) + byte_length as u128 > mapping.end()
                {
                    continue;
                }
                if !mapping.permissions.allows(access) {
                    return Err(fault(
                        "permission",
                        address.offset,
                        byte_length,
                        access,
                        format!(
                            "mapping '{}' permits {}",
                            mapping.label,
                            mapping.permissions.label()
                        ),
                    ));
                }
                self.recent[slot] = Some(candidate);
                self.lookup_offset = (address.offset - mapping.base) as usize;
                return Ok(Some(candidate));
            }
        }
        self.check_owned(address, byte_length, access.label())?;
        if byte_length == 0 {
            return Ok(None);
        }
        let index = self.first_end_after(address.offset);
        let Some(mapping) = self.mappings.get(index).cloned() else {
            return Ok(None);
        };
        if address.offset < mapping.base || u128::from(address.offset) + byte_length as u128 > mapping.end() {
            return Ok(None);
        }
        if !mapping.permissions.allows(access) {
            return Err(fault(
                "permission",
                address.offset,
                byte_length,
                access,
                format!(
                    "mapping '{}' permits {}",
                    mapping.label,
                    mapping.permissions.label()
                ),
            ));
        }
        self.remember(Some(access), index);
        self.lookup_offset = (address.offset - mapping.base) as usize;
        Ok(Some(index))
    }

    fn execute_mapping(&mut self, byte_offset: u64) -> Result<usize, GuestError> {
        if let Some(index) = self.recent[Self::access_slot(Some(GuestAccess::Execute))] {
            if let Some(recent) = self.mappings.get(index) {
                if recent.active && byte_offset >= recent.base && u128::from(byte_offset) < recent.end() {
                    if !recent.permissions.allows(GuestAccess::Execute) {
                        return Err(fault(
                            "permission",
                            byte_offset,
                            1,
                            GuestAccess::Execute,
                            format!(
                                "mapping '{}' permits {}",
                                recent.label,
                                recent.permissions.label()
                            ),
                        ));
                    }
                    return Ok(index);
                }
            }
        }
        self.check_range(byte_offset, 1, "execute")?;
        let index = self.first_end_after(byte_offset);
        let mapping = self.mappings.get(index).cloned().ok_or_else(|| {
            fault(
                "unmapped",
                byte_offset,
                1,
                GuestAccess::Execute,
                "range includes unmapped bytes",
            )
        })?;
        if byte_offset < mapping.base {
            return Err(fault(
                "unmapped",
                byte_offset,
                1,
                GuestAccess::Execute,
                "range includes unmapped bytes",
            ));
        }
        if !mapping.permissions.allows(GuestAccess::Execute) {
            return Err(fault(
                "permission",
                byte_offset,
                1,
                GuestAccess::Execute,
                format!(
                    "mapping '{}' permits {}",
                    mapping.label,
                    mapping.permissions.label()
                ),
            ));
        }
        self.remember(Some(GuestAccess::Execute), index);
        Ok(index)
    }

    fn read_scalar(&mut self, address: GuestAddress, byte_length: usize) -> Result<Vec<u8>, GuestError> {
        if let Some(index) = self.single_mapping(address, byte_length, GuestAccess::Read)? {
            let offset = self.lookup_offset;
            let mapping = &self.mappings[index];
            let backing = mapping.backing.borrow();
            let start = mapping.backing_start + offset;
            return Ok(backing.bytes[start..start + byte_length].to_vec());
        }
        let chunks = self.chunks(address, byte_length, Some(GuestAccess::Read))?;
        Ok(self.copy_chunks(&chunks, byte_length))
    }

    fn commit_write(&mut self, chunks: &[Chunk], source: &[u8]) -> Result<(), GuestError> {
        let mut consumed = 0;
        for chunk in chunks {
            let mapping = self.mappings[chunk.mapping_id].clone();
            {
                let mut backing = mapping.backing.borrow_mut();
                let start = mapping.backing_start + chunk.offset;
                backing.bytes[start..start + chunk.byte_length]
                    .copy_from_slice(&source[consumed..consumed + chunk.byte_length]);
            }
            self.invalidate_code(&mapping, chunk.offset, chunk.byte_length);
            consumed += chunk.byte_length;
        }
        self.notify_write(chunks)
    }

    fn invalidate_code(&self, mapping: &Mapping, offset: usize, byte_length: usize) {
        if byte_length == 0 {
            return;
        }
        let mut backing = mapping.backing.borrow_mut();
        if backing.last_page < 0 {
            return;
        }
        let start = mapping.backing_start + offset;
        let first = (start / PAGE_BYTES) as u32;
        let last = ((start + byte_length - 1) / PAGE_BYTES) as u32;
        for page in first.max(backing.first_page)..=last.min(backing.last_page as u32) {
            let revision = backing.next_revision;
            backing.next_revision += 1;
            if let Some(slot) = backing.pages.get_mut(&page) {
                *slot = revision;
            }
        }
    }

    fn notify_write(&self, chunks: &[Chunk]) -> Result<(), GuestError> {
        if self.observers.is_empty() {
            return Ok(());
        }
        let mut failures: Vec<String> = Vec::new();
        let mut ids: Vec<u64> = self.observers.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let Some(observer) = self.observers.get(&id) else {
                continue;
            };
            let mut ranges = Vec::new();
            let mut displacement = 0;
            for watched in &observer.chunks {
                for written in chunks {
                    let written_mapping = &self.mappings[written.mapping_id];
                    if !Rc::ptr_eq(&watched.backing, &written_mapping.backing) {
                        continue;
                    }
                    let start = watched.start;
                    let other = written_mapping.backing_start + written.offset;
                    let first = start.max(other);
                    let last = (start + watched.byte_length).min(other + written.byte_length);
                    if first < last {
                        ranges.push(GuestWrittenRange {
                            byte_offset: displacement + first - start,
                            byte_length: last - first,
                        });
                    }
                }
                displacement += watched.byte_length;
            }
            if !ranges.is_empty() {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    (observer.notify)(&ranges);
                }))
                .is_err()
                {
                    failures.push(format!("Guest write observer {id} failed"));
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(GuestError::Callback(failures.join("; ")))
        }
    }
}

/// Cursor for sequential instruction-byte fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchCursor {
    /// Sequence base offset.
    pub base: u64,
    /// Consumed bytes.
    pub consumed: u64,
    mapping: Option<u64>,
    offset: usize,
    generation: u64,
}

impl FetchCursor {
    /// Cursor starting at `base`.
    #[must_use]
    pub const fn new(base: u64) -> Self {
        Self {
            base,
            consumed: 0,
            mapping: None,
            offset: 0,
            generation: u64::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;

    use crate::core::contracts::ContentDigest;

    fn test_module() -> ModuleIdentity {
        ModuleIdentity::new(
            ProviderId::new("test", "memory"),
            "test.so",
            ContentDigest::new("sha256", "abc"),
            "r1",
        )
    }

    #[test]
    fn map_write_read_round_trip() {
        let mut memory = SparseGuestMemory::new(test_module(), 8, 0x10000).unwrap();
        let base = memory
            .map(&GuestMapOptions::new(0x10000, 0x1000, GuestPermissions::ReadWrite))
            .unwrap();
        memory.write_u64(base, 0xdead_beef_cafe_f00d).unwrap();
        assert_eq!(memory.read_u64(base).unwrap(), 0xdead_beef_cafe_f00d);
        assert_eq!(memory.mappings().len(), 1);
    }

    #[test]
    fn aliases_share_backing_and_observers_see_writes() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let mut memory = SparseGuestMemory::new(test_module(), 4, 0x10000).unwrap();
        let base = memory
            .map(&GuestMapOptions::new(0x10000, 0x100, GuestPermissions::ReadWrite))
            .unwrap();
        let alias = memory
            .map_alias(0x20000, 0x100, GuestPermissions::ReadWrite, "alias", base)
            .unwrap();
        memory.write_u32(alias, 0x1234_5678).unwrap();
        assert_eq!(memory.read_u32(base).unwrap(), 0x1234_5678);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_clone = Rc::clone(&seen);
        let id = memory
            .observe_writes(
                base,
                0x100,
                Box::new(move |ranges| seen_clone.borrow_mut().extend_from_slice(ranges)),
            )
            .unwrap();
        memory.write_u8(memory.offset(alias, 7).unwrap(), 0xff).unwrap();
        assert_eq!(
            *seen.borrow(),
            vec![GuestWrittenRange {
                byte_offset: 7,
                byte_length: 1
            }]
        );
        memory.unobserve(id);
    }

    #[test]
    fn checkpoint_restore_preserves_aliases() {
        let mut memory = SparseGuestMemory::new(test_module(), 8, 0x10000).unwrap();
        let base = memory
            .map(&GuestMapOptions::new(0x10000, 0x10, GuestPermissions::ReadWrite))
            .unwrap();
        memory.write_u64(base, 42).unwrap();
        memory
            .map_alias(0x20000, 0x10, GuestPermissions::Read, "alias", base)
            .unwrap();
        let snapshot = memory.checkpoint();
        let mut restored = SparseGuestMemory::restore(test_module(), &snapshot).unwrap();
        let restored_base = restored.pointer(0x10000).unwrap().unwrap();
        assert_eq!(restored.read_u64(restored_base).unwrap(), 42);
        assert_eq!(restored.mappings().len(), 2);
    }

    #[test]
    fn executable_retention_invalidates_on_store() {
        let mut memory = SparseGuestMemory::new(test_module(), 8, 0x10000).unwrap();
        let base = memory
            .map(&GuestMapOptions {
                base: 0x10000,
                byte_length: 0x1000,
                permissions: GuestPermissions::ReadWriteExecute,
                label: "code".to_string(),
                bytes: None,
            })
            .unwrap();
        memory.write(base, &[0x90, 0xc3]).unwrap();
        let retention = memory.retain_executable_bytes(0x10000, &[0x90, 0xc3]).unwrap();
        assert!(retention.unchanged(&memory));
        memory.write_u8(base, 0xcc).unwrap();
        assert!(!retention.unchanged(&memory));
    }

    #[test]
    fn faults_name_reason_and_range() {
        let mut memory = SparseGuestMemory::new(test_module(), 4, 0x10000).unwrap();
        let address = GuestAddress::new(memory.address_space(), 0x5000);
        let error = memory.read_u32(address).unwrap_err();
        assert!(matches!(error, GuestError::MemoryFault { .. }));
    }
}
