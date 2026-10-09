//! Owned module bytes. Engine views borrow this backing rather than a copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryError;

pub struct ModuleMemory {
    base: u64,
    bytes: Box<[u8]>,
}
impl ModuleMemory {
    pub fn load(base: u64, length: usize, initialized: &[u8]) -> Result<Self, MemoryError> {
        if initialized.len() > length
            || length > 512 * 1024 * 1024
            || base.checked_add(length as u64).is_none()
        {
            return Err(MemoryError);
        }
        let mut bytes = vec![0; length].into_boxed_slice();
        bytes[..initialized.len()].copy_from_slice(initialized);
        Ok(Self { base, bytes })
    }
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
    fn range(&self, address: u64, length: usize) -> Result<std::ops::Range<usize>, MemoryError> {
        let start = usize::try_from(address.checked_sub(self.base).ok_or(MemoryError)?)
            .map_err(|_| MemoryError)?;
        let end = start
            .checked_add(length)
            .filter(|&end| end <= self.bytes.len())
            .ok_or(MemoryError)?;
        Ok(start..end)
    }
    pub fn read(&self, address: u64, length: usize) -> Result<&[u8], MemoryError> {
        Ok(&self.bytes[self.range(address, length)?])
    }
    pub fn write(&mut self, address: u64, bytes: &[u8]) -> Result<(), MemoryError> {
        let range = self.range(address, bytes.len())?;
        self.bytes[range].copy_from_slice(bytes);
        Ok(())
    }
    pub fn read_word(&self, address: u64) -> Result<i32, MemoryError> {
        let b = self.read(address, 4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    pub fn write_word(&mut self, address: u64, word: i32) -> Result<(), MemoryError> {
        self.write(address, &word.to_le_bytes())
    }
    pub fn cstring(&self, address: u64) -> Result<&[u8], MemoryError> {
        let start = self.range(address, 0)?.start;
        let tail = &self.bytes[start..];
        let end = tail.iter().position(|&b| b == 0).ok_or(MemoryError)?;
        Ok(&tail[..end])
    }
}
