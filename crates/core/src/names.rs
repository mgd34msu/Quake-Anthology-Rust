use crate::primitives::NameId;
use std::cmp::Ordering;

fn compare(left: &[u8], right: &[u8]) -> Ordering {
    left.iter()
        .map(u8::to_ascii_lowercase)
        .cmp(right.iter().map(u8::to_ascii_lowercase))
}

#[derive(Debug, PartialEq, Eq)]
pub enum NamesError {
    Capacity,
}

pub struct NameTable {
    bytes: Box<[u8]>,
    entries: Box<[(u32, u32)]>,
    compare: fn(&[u8], &[u8]) -> Ordering,
}

impl NameTable {
    /// Build once at map load. NameId(0) denotes the empty name.
    pub fn load<'a>(names: impl IntoIterator<Item = &'a [u8]>) -> Result<Self, NamesError> {
        Self::build(names, compare)
    }

    /// Original Q1 field/function names require byte-exact lookup.
    pub fn load_exact<'a>(names: impl IntoIterator<Item = &'a [u8]>) -> Result<Self, NamesError> {
        Self::build(names, <[u8]>::cmp)
    }

    fn build<'a>(
        names: impl IntoIterator<Item = &'a [u8]>,
        compare: fn(&[u8], &[u8]) -> Ordering,
    ) -> Result<Self, NamesError> {
        let mut unique: Vec<&[u8]> = vec![b""];
        unique.extend(names);
        unique.sort_by(|left, right| compare(left, right));
        unique.dedup_by(|left, right| compare(left, right) == Ordering::Equal);
        let count = u32::try_from(unique.len()).map_err(|_| NamesError::Capacity)?;
        let size = unique
            .iter()
            .try_fold(0u32, |sum, name| {
                sum.checked_add(u32::try_from(name.len()).ok()?)
            })
            .ok_or(NamesError::Capacity)?;
        let mut bytes = Vec::with_capacity(size as usize);
        let mut entries = Vec::with_capacity(count as usize);
        for name in unique {
            entries.push((bytes.len() as u32, name.len() as u32));
            bytes.extend_from_slice(name);
        }
        Ok(Self {
            bytes: bytes.into_boxed_slice(),
            entries: entries.into_boxed_slice(),
            compare,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, id: NameId) -> Option<&[u8]> {
        self.entries
            .get(id.0 as usize)
            .map(|&(offset, len)| &self.bytes[offset as usize..offset as usize + len as usize])
    }

    pub fn find(&self, name: &[u8]) -> Option<NameId> {
        self.entries
            .binary_search_by(|&(offset, len)| {
                (self.compare)(
                    &self.bytes[offset as usize..offset as usize + len as usize],
                    name,
                )
            })
            .ok()
            .map(|index| NameId(index as u32))
    }
}
