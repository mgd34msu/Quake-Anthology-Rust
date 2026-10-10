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
        let Some(next) = self.allocate(bytes) else {
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
