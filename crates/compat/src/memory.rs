//! One module byte view over owned VM storage or stopped native backing.
mod heap;
pub use heap::Heap;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryError;

enum Bytes<'a> {
    Owned(Box<[u8]>),
    Borrowed(&'a mut [u8]),
}
impl AsRef<[u8]> for Bytes<'_> {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Borrowed(bytes) => bytes,
        }
    }
}
impl AsMut<[u8]> for Bytes<'_> {
    fn as_mut(&mut self) -> &mut [u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Borrowed(bytes) => bytes,
        }
    }
}
pub struct ModuleMemory<'a> {
    base: u64,
    bytes: Bytes<'a>,
}
fn extent(base: u64, length: usize) -> Result<(), MemoryError> {
    if length > 512 * 1024 * 1024 || base.checked_add(length as u64).is_none() {
        return Err(MemoryError);
    }
    Ok(())
}
impl ModuleMemory<'static> {
    pub fn load(base: u64, length: usize, initialized: &[u8]) -> Result<Self, MemoryError> {
        extent(base, length)?;
        if initialized.len() > length {
            return Err(MemoryError);
        }
        let mut bytes = vec![0; length].into_boxed_slice();
        bytes[..initialized.len()].copy_from_slice(initialized);
        Ok(Self {
            base,
            bytes: Bytes::Owned(bytes),
        })
    }
}
impl<'a> ModuleMemory<'a> {
    /// The lifetime prevents this view escaping the owner's parked borrow.
    pub fn borrow(base: u64, bytes: &'a mut [u8]) -> Result<Self, MemoryError> {
        extent(base, bytes.len())?;
        Ok(Self {
            base,
            bytes: Bytes::Borrowed(bytes),
        })
    }
    pub fn len(&self) -> usize {
        self.bytes.as_ref().len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.as_ref().is_empty()
    }
    fn range(&self, address: u64, length: usize) -> Result<std::ops::Range<usize>, MemoryError> {
        let start = usize::try_from(address.checked_sub(self.base).ok_or(MemoryError)?)
            .map_err(|_| MemoryError)?;
        let end = start
            .checked_add(length)
            .filter(|&end| end <= self.len())
            .ok_or(MemoryError)?;
        Ok(start..end)
    }
    pub fn read(&self, address: u64, length: usize) -> Result<&[u8], MemoryError> {
        Ok(&self.bytes.as_ref()[self.range(address, length)?])
    }
    pub fn read_mut(&mut self, address: u64, length: usize) -> Result<&mut [u8], MemoryError> {
        let range = self.range(address, length)?;
        Ok(&mut self.bytes.as_mut()[range])
    }
    pub fn copy(&mut self, to: u64, from: u64, length: usize) -> Result<(), MemoryError> {
        let source = self.range(from, length)?;
        let target = self.range(to, length)?;
        self.bytes.as_mut().copy_within(source, target.start);
        Ok(())
    }
    pub fn write(&mut self, address: u64, bytes: &[u8]) -> Result<(), MemoryError> {
        let range = self.range(address, bytes.len())?;
        self.bytes.as_mut()[range].copy_from_slice(bytes);
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
        let tail = &self.bytes.as_ref()[start..];
        let end = tail.iter().position(|&b| b == 0).ok_or(MemoryError)?;
        Ok(&tail[..end])
    }
}
