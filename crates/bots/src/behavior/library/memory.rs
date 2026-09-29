//! Bot memory manager from `src/bots/behavior/library/memory.ts`
//! (`be_memory.c`: `GetMemory`/`FreeMemory`, `GetHunkMemory`).
//!
//! The donor threads explicit allocations through the bot libraries so
//! checkpoints can capture every library-owned byte. This port keeps the
//! same shape: typed heap/hunk allocations with provenance, a capture
//! that snapshots live bytes, and a restore that replays them.

use crate::error::BotsError;

/// Allocation arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotMemoryKind {
    /// Long-lived heap block.
    Heap,
    /// Map-lifetime hunk block.
    Hunk,
}

/// Allocation provenance for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotMemoryProvenance {
    /// Source file.
    pub file: String,
    /// Source line.
    pub line: u32,
    /// Allocation label.
    pub label: String,
}

/// Handle to a bot memory allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotMemoryAllocation {
    id: u32,
}

impl BotMemoryAllocation {
    /// Null allocation handle.
    #[must_use]
    pub const fn null() -> Self {
        Self { id: 0 }
    }

    /// Whether this is the null handle.
    #[must_use]
    pub const fn is_null(self) -> bool {
        self.id == 0
    }
}

#[derive(Debug, Clone)]
struct AllocationRecord {
    kind: BotMemoryKind,
    bytes: Vec<u8>,
    provenance: Option<BotMemoryProvenance>,
}

/// Checkpoint image of live bot memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotMemoryCheckpoint {
    /// Live allocations in id order.
    pub allocations: Vec<BotMemoryCheckpointAllocation>,
}

/// One checkpointed allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotMemoryCheckpointAllocation {
    /// Arena.
    pub kind: BotMemoryKind,
    /// Allocation bytes.
    pub bytes: Vec<u8>,
    /// Provenance, when recorded.
    pub provenance: Option<BotMemoryProvenance>,
}

/// Capture handle resolving allocations to checkpoint references.
#[derive(Debug, Clone)]
pub struct BotMemoryCapture {
    image: BotMemoryCheckpoint,
    ids: Vec<u32>,
}

impl BotMemoryCapture {
    /// Checkpoint image.
    #[must_use]
    pub fn image(&self) -> &BotMemoryCheckpoint {
        &self.image
    }

    /// Reference for an allocation, or `None` for foreign handles.
    #[must_use]
    pub fn reference(&self, allocation: BotMemoryAllocation) -> Option<usize> {
        self.ids.iter().position(|id| *id == allocation.id)
    }
}

/// Restore handle resolving checkpoint references to allocations.
#[derive(Debug, Clone)]
pub struct BotMemoryRestore {
    ids: Vec<u32>,
}

impl BotMemoryRestore {
    /// Allocation for a checkpoint reference.
    #[must_use]
    pub fn allocation(&self, reference: usize) -> Option<BotMemoryAllocation> {
        self.ids.get(reference).map(|id| BotMemoryAllocation { id: *id })
    }
}

/// Bot memory manager.
#[derive(Debug, Default)]
pub struct BotMemory {
    records: std::collections::HashMap<u32, AllocationRecord>,
    next_id: u32,
    disposed: bool,
}

impl BotMemory {
    /// Empty manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: std::collections::HashMap::new(),
            next_id: 1,
            disposed: false,
        }
    }

    fn check_live(&self) -> Result<(), BotsError> {
        if self.disposed {
            return Err(BotsError::BotLibraryShutdown);
        }
        Ok(())
    }

    /// Allocate `size` bytes, optionally zeroed.
    pub fn allocate(
        &mut self,
        size: usize,
        kind: BotMemoryKind,
        clear: bool,
        provenance: Option<BotMemoryProvenance>,
    ) -> Result<BotMemoryAllocation, BotsError> {
        self.check_live()?;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let bytes = if clear { vec![0u8; size] } else { vec![0u8; size] };
        self.records.insert(
            id,
            AllocationRecord {
                kind,
                bytes,
                provenance,
            },
        );
        Ok(BotMemoryAllocation { id })
    }

    /// Read allocation bytes.
    pub fn bytes(&self, allocation: BotMemoryAllocation) -> Result<&[u8], BotsError> {
        self.check_live()?;
        self.records
            .get(&allocation.id)
            .map(|record| record.bytes.as_slice())
            .ok_or(BotsError::BotMemoryFree)
    }

    /// Write allocation bytes.
    pub fn bytes_mut(&mut self, allocation: BotMemoryAllocation) -> Result<&mut Vec<u8>, BotsError> {
        self.check_live()?;
        self.records
            .get_mut(&allocation.id)
            .map(|record| &mut record.bytes)
            .ok_or(BotsError::BotMemoryFree)
    }

    /// Byte size of an allocation, or 0 for null/foreign handles.
    #[must_use]
    pub fn memory_byte_size(&self, allocation: BotMemoryAllocation) -> usize {
        self.records.get(&allocation.id).map_or(0, |record| record.bytes.len())
    }

    /// Free an allocation; null frees nothing.
    pub fn free(&mut self, allocation: BotMemoryAllocation) -> Result<(), BotsError> {
        self.check_live()?;
        if allocation.is_null() {
            return Ok(());
        }
        self.records
            .remove(&allocation.id)
            .map(|_| ())
            .ok_or(BotsError::BotMemoryFree)
    }

    /// Release all hunk allocations (map change).
    pub fn reset_hunk(&mut self) {
        self.records.retain(|_, record| record.kind != BotMemoryKind::Hunk);
    }

    /// Dispose the manager; further use fails.
    pub fn dispose(&mut self) {
        self.records.clear();
        self.disposed = true;
    }

    /// Live allocation count.
    #[must_use]
    pub fn live_allocations(&self) -> usize {
        self.records.len()
    }

    /// Live allocated bytes.
    #[must_use]
    pub fn allocated_bytes(&self) -> usize {
        self.records.values().map(|record| record.bytes.len()).sum()
    }

    /// Available bytes for new allocations.
    #[must_use]
    pub fn available_memory(&self) -> usize {
        usize::MAX - self.allocated_bytes()
    }

    /// Snapshot live allocations in id order.
    pub fn checkpoint(&self) -> BotMemoryCapture {
        let mut ids: Vec<u32> = self.records.keys().copied().collect();
        ids.sort_unstable();
        let allocations = ids
            .iter()
            .map(|id| {
                let record = &self.records[id];
                BotMemoryCheckpointAllocation {
                    kind: record.kind,
                    bytes: record.bytes.clone(),
                    provenance: record.provenance.clone(),
                }
            })
            .collect();
        BotMemoryCapture {
            image: BotMemoryCheckpoint { allocations },
            ids,
        }
    }

    /// Replay a checkpoint image into fresh allocations.
    pub fn restore(&mut self, image: &BotMemoryCheckpoint) -> BotMemoryRestore {
        let mut ids = Vec::with_capacity(image.allocations.len());
        for allocation in &image.allocations {
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1).max(1);
            self.records.insert(
                id,
                AllocationRecord {
                    kind: allocation.kind,
                    bytes: allocation.bytes.clone(),
                    provenance: allocation.provenance.clone(),
                },
            );
            ids.push(id);
        }
        BotMemoryRestore { ids }
    }
}
