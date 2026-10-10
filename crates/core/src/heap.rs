//! Bounded native C allocations within one load-reserved module byte range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryError;

pub fn extent(base: u64, length: usize) -> Result<(), MemoryError> {
    if length > 512 * 1024 * 1024 || base.checked_add(length as u64).is_none() {
        return Err(MemoryError);
    }
    Ok(())
}

#[derive(Clone, Copy, Default)]
struct Block {
    offset: usize,
    bytes: usize,
    requested: Option<usize>,
    tag: Option<i32>,
}
pub struct Heap {
    base: u64,
    blocks: Box<[Block]>,
    count: usize,
}
impl Heap {
    pub fn load(base: u64, bytes: usize, capacity: usize) -> Result<Self, MemoryError> {
        extent(base, bytes)?;
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
        self.allocate_with_tag(requested, None)
    }
    pub fn allocate_tagged(&mut self, requested: usize, tag: i32) -> Option<u64> {
        self.allocate_with_tag(requested, Some(tag))
    }
    fn allocate_with_tag(&mut self, requested: usize, tag: Option<i32>) -> Option<u64> {
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
                tag: None,
            };
            self.blocks[index].bytes = bytes;
            self.count += 1;
        }
        // At the metadata limit, consume the whole free extent rather than
        // growing metadata or failing an otherwise satisfiable allocation.
        self.blocks[index].requested = Some(requested);
        self.blocks[index].tag = tag;
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
    fn coalesce(&mut self, begin: usize, end: usize) {
        let mut write = begin;
        for read in begin..end {
            let block = self.blocks[read];
            if write > begin
                && self.blocks[write - 1].requested.is_none()
                && block.requested.is_none()
            {
                self.blocks[write - 1].bytes += block.bytes;
            } else {
                self.blocks[write] = block;
                write += 1;
            }
        }
        if write != end {
            self.blocks.copy_within(end..self.count, write);
            self.count -= end - write;
        }
    }
    pub fn free(&mut self, address: u64) -> Result<(), MemoryError> {
        if address == 0 {
            return Ok(());
        }
        let index = self.index(address)?;
        self.blocks[index].requested = None;
        self.blocks[index].tag = None;
        self.coalesce(index.saturating_sub(1), (index + 2).min(self.count));
        Ok(())
    }
    /// Bulk native game/level frees leave ordinary CRT allocations untouched,
    /// including when the native tag itself is zero.
    pub fn free_tag(&mut self, tag: i32) -> usize {
        let mut freed = 0;
        for block in &mut self.blocks[..self.count] {
            if block.requested.is_some() && block.tag == Some(tag) {
                block.requested = None;
                block.tag = None;
                freed += 1;
            }
        }
        if freed != 0 {
            self.coalesce(0, self.count);
        }
        freed
    }
    pub fn reallocate(
        &mut self,
        mut copy: impl FnMut(u64, u64, usize) -> Result<(), MemoryError>,
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
        let Some(next) = self.allocate_with_tag(bytes, old.tag) else {
            return Ok(0);
        };
        if let Err(error) = copy(next, address, count) {
            self.free(next)?;
            return Err(error);
        }
        self.free(address)?;
        Ok(next)
    }
}
