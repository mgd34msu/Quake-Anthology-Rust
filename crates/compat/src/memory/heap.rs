//! Bounded native C allocations within one load-reserved module byte range.
use super::{MemoryError, ModuleMemory};

#[derive(Clone, Copy, Default)]
struct Block {
    offset: usize,
    bytes: usize,
    requested: Option<usize>,
}
pub struct Heap {
    base: u64,
    blocks: Box<[Block]>,
    count: usize,
}
impl Heap {
    pub fn load(base: u64, bytes: usize, capacity: usize) -> Result<Self, MemoryError> {
        super::extent(base, bytes)?;
        if base == 0
            || base % 16 != 0
            || bytes == 0
            || bytes % 16 != 0
            || capacity == 0
            || capacity > 65536
        {
            return Err(MemoryError);
        }
        let mut blocks = vec![Block::default(); capacity].into_boxed_slice();
        blocks[0].bytes = bytes;
        Ok(Self {
            base,
            blocks,
            count: 1,
        })
    }
    pub fn allocate(&mut self, requested: usize) -> Option<u64> {
        let bytes = requested.max(1).checked_add(15)? & !15;
        let index = self.blocks[..self.count]
            .iter()
            .position(|block| block.requested.is_none() && block.bytes >= bytes)?;
        let block = self.blocks[index];
        if block.bytes > bytes && self.count < self.blocks.len() {
            self.blocks.copy_within(index + 1..self.count, index + 2);
            self.blocks[index + 1] = Block {
                offset: block.offset + bytes,
                bytes: block.bytes - bytes,
                requested: None,
            };
            self.blocks[index].bytes = bytes;
            self.count += 1;
        }
        // At the metadata limit, consume the whole free extent rather than
        // growing metadata or failing an otherwise satisfiable allocation.
        self.blocks[index].requested = Some(requested);
        Some(self.base + block.offset as u64)
    }
    fn index(&self, address: u64) -> Result<usize, MemoryError> {
        let offset = usize::try_from(address.checked_sub(self.base).ok_or(MemoryError)?)
            .map_err(|_| MemoryError)?;
        self.blocks[..self.count]
            .binary_search_by_key(&offset, |block| block.offset)
            .ok()
            .filter(|&index| self.blocks[index].requested.is_some())
            .ok_or(MemoryError)
    }
    fn remove(&mut self, index: usize) {
        self.blocks.copy_within(index + 1..self.count, index);
        self.count -= 1;
    }
    pub fn free(&mut self, address: u64) -> Result<(), MemoryError> {
        if address == 0 {
            return Ok(());
        }
        let mut index = self.index(address)?;
        self.blocks[index].requested = None;
        if index > 0 && self.blocks[index - 1].requested.is_none() {
            self.blocks[index - 1].bytes += self.blocks[index].bytes;
            self.remove(index);
            index -= 1;
        }
        if index + 1 < self.count && self.blocks[index + 1].requested.is_none() {
            self.blocks[index].bytes += self.blocks[index + 1].bytes;
            self.remove(index + 1);
        }
        Ok(())
    }
    pub fn reallocate(
        &mut self,
        memory: &mut ModuleMemory<'_>,
        address: u64,
        bytes: usize,
    ) -> Result<u64, MemoryError> {
        if address == 0 {
            return Ok(self.allocate(bytes).unwrap_or(0));
        }
        let old = self.blocks[self.index(address)?];
        let count = old.requested.ok_or(MemoryError)?.min(bytes);
        if bytes == 0 {
            self.free(address)?;
            return Ok(0);
        }
        if bytes <= old.bytes {
            self.blocks[self.index(address)?].requested = Some(bytes);
            return Ok(address);
        }
        let Some(next) = self.allocate(bytes) else {
            return Ok(0);
        };
        if let Err(error) = memory.copy(next, address, count) {
            self.free(next)?;
            return Err(error);
        }
        self.free(address)?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_blocks_align_reuse_coalesce_and_reject_non_allocations() {
        let mut heap = Heap::load(0x1000, 128, 8).unwrap();
        let first = heap.allocate(17).unwrap();
        let second = heap.allocate(1).unwrap();
        let third = heap.allocate(31).unwrap();
        assert_eq!((first, second, third), (0x1000, 0x1020, 0x1030));
        assert_eq!(heap.free(second + 1), Err(MemoryError));
        heap.free(second).unwrap();
        assert_eq!(heap.free(second), Err(MemoryError));
        heap.free(third).unwrap();
        heap.free(first).unwrap();
        assert_eq!(heap.allocate(128), Some(first));
        assert_eq!(heap.allocate(0), None);
        heap.free(first).unwrap();
        assert_eq!(heap.allocate(usize::MAX), None);
        assert_eq!(heap.allocate(0), Some(first));
        heap.free(0).unwrap();
    }
    #[test]
    fn seeded_allocations_keep_live_ranges_disjoint_and_bytes_intact() {
        let mut heap = Heap::load(0x1000, 16384, 128).unwrap();
        let mut memory = ModuleMemory::load(0x1000, 16384, &[]).unwrap();
        let mut live = [None; 64];
        let mut seed = 1729u32;
        for _ in 0..4096 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let slot = ((seed >> 16) as usize) % live.len();
            if let Some((address, bytes)) = live[slot].take() {
                assert!(
                    memory
                        .read(address, bytes)
                        .unwrap()
                        .iter()
                        .all(|&b| b == slot as u8)
                );
                heap.free(address).unwrap();
            } else {
                let bytes = (seed as usize & 511) + 1;
                if let Some(address) = heap.allocate(bytes) {
                    for &(other, size) in live.iter().flatten() {
                        assert!(address + bytes as u64 <= other || other + size as u64 <= address);
                    }
                    memory.read_mut(address, bytes).unwrap().fill(slot as u8);
                    live[slot] = Some((address, bytes));
                }
            }
        }
        for (address, bytes) in live.into_iter().flatten() {
            assert!(
                memory
                    .read(address, bytes)
                    .unwrap()
                    .iter()
                    .all(|&b| b == memory.read(address, 1).unwrap()[0])
            );
            heap.free(address).unwrap();
        }
        assert_eq!(heap.allocate(16384), Some(0x1000));
    }
    #[test]
    fn metadata_limit_uses_existing_extent_and_reallocation_preserves_old_on_failure() {
        let base = 1 << 40;
        let mut heap = Heap::load(base, 128, 2).unwrap();
        let mut memory = ModuleMemory::load(base, 128, &[]).unwrap();
        let first = heap.allocate(17).unwrap();
        let second = heap.allocate(16).unwrap();
        assert_eq!(heap.count, 2);
        memory.write(first, b"original").unwrap();
        assert_eq!(heap.reallocate(&mut memory, first, 64).unwrap(), 0);
        assert_eq!(memory.read(first, 8).unwrap(), b"original");
        heap.free(second).unwrap();
        let moved = heap.reallocate(&mut memory, first, 64).unwrap();
        assert_ne!(moved, first);
        assert_eq!(memory.read(moved, 8).unwrap(), b"original");
        assert_eq!(heap.reallocate(&mut memory, moved, 4).unwrap(), moved);
        assert_eq!(heap.reallocate(&mut memory, moved, 0).unwrap(), 0);
        assert_eq!(heap.count, 1);
        assert_eq!(heap.allocate(128), Some(base));
    }
}
