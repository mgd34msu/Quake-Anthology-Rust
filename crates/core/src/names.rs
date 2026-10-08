use crate::primitives::NameId;
use std::cmp::Ordering;

fn compare_folded(left: &[u8], right: &[u8]) -> Ordering {
    left.iter()
        .map(u8::to_ascii_lowercase)
        .cmp(right.iter().map(u8::to_ascii_lowercase))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameMatch {
    Exact,
    Folded,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NamesError {
    Capacity,
}

pub struct NameTable {
    bytes: Box<[u8]>,
    entries: Box<[(u32, u32)]>,
    folded_entries: Box<[NameId]>,
    folded_ids: Box<[NameId]>,
}

impl NameTable {
    /// Build once at load. Exact bytes determine IDs; NameId(0) is empty.
    /// Folded groups retain their lowest exact ID as a deterministic name.
    pub fn load<'a>(names: impl IntoIterator<Item = &'a [u8]>) -> Result<Self, NamesError> {
        let mut unique: Vec<&[u8]> = vec![b""];
        unique.extend(names);
        unique.sort_unstable();
        unique.dedup();
        let count = u32::try_from(unique.len()).map_err(|_| NamesError::Capacity)?;
        let size = unique
            .iter()
            .try_fold(0u32, |sum, name| {
                sum.checked_add(u32::try_from(name.len()).ok()?)
            })
            .ok_or(NamesError::Capacity)?;

        let mut folded_entries: Vec<_> = (0..count).map(NameId).collect();
        folded_entries.sort_unstable_by(|left, right| {
            compare_folded(unique[left.0 as usize], unique[right.0 as usize])
                .then_with(|| left.0.cmp(&right.0))
        });
        let mut folded_ids = vec![NameId(0); count as usize];
        let mut representative = NameId(0);
        for (index, &id) in folded_entries.iter().enumerate() {
            if index == 0
                || compare_folded(
                    unique[folded_entries[index - 1].0 as usize],
                    unique[id.0 as usize],
                ) != Ordering::Equal
            {
                representative = id;
            }
            folded_ids[id.0 as usize] = representative;
        }
        folded_entries.dedup_by(|left, right| {
            compare_folded(unique[left.0 as usize], unique[right.0 as usize]) == Ordering::Equal
        });

        let mut bytes = Vec::with_capacity(size as usize);
        let mut entries = Vec::with_capacity(count as usize);
        for name in unique {
            entries.push((bytes.len() as u32, name.len() as u32));
            bytes.extend_from_slice(name);
        }
        Ok(Self {
            bytes: bytes.into_boxed_slice(),
            entries: entries.into_boxed_slice(),
            folded_entries: folded_entries.into_boxed_slice(),
            folded_ids: folded_ids.into_boxed_slice(),
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
                self.bytes[offset as usize..offset as usize + len as usize].cmp(name)
            })
            .ok()
            .map(|index| NameId(index as u32))
    }

    pub fn find_folded(&self, name: &[u8]) -> Option<NameId> {
        self.folded_entries
            .binary_search_by(|id| {
                let (offset, len) = self.entries[id.0 as usize];
                compare_folded(
                    &self.bytes[offset as usize..offset as usize + len as usize],
                    name,
                )
            })
            .ok()
            .map(|index| self.folded_entries[index])
    }

    /// Cached canonical exact ID for this ASCII-folded group.
    pub fn folded(&self, id: NameId) -> Option<NameId> {
        self.folded_ids.get(id.0 as usize).copied()
    }
}
