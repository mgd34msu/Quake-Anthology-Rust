use crate::primitives::NameId;
use std::cmp::Ordering;

fn fold(byte: u8) -> u8 {
    byte.to_ascii_lowercase()
}
pub fn compare_folded(left: &[u8], right: &[u8]) -> Ordering {
    // Q_stricmp orders its ASCII letters as uppercase, including their
    // position relative to punctuation; equality uses the same folded groups.
    left.iter()
        .copied()
        .map(|byte| byte.to_ascii_uppercase())
        .cmp(right.iter().copied().map(|byte| byte.to_ascii_uppercase()))
}
fn path_byte(byte: u8) -> u8 {
    fold(if byte == b'\\' { b'/' } else { byte })
}
/// The sole renderer path conversion; other exact names never use this rule.
pub fn canonical_path(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii() {
                char::from(path_byte(c as u8))
            } else {
                c
            }
        })
        .collect()
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

impl std::fmt::Display for NamesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("name storage capacity")
    }
}
impl std::error::Error for NamesError {}

/// Exact identities remain stable. Runtime registrations use capacity reserved
/// at load; bucket keys only accelerate lookup and never identify content.
pub struct NameTable {
    bytes: Box<[u8]>,
    entries: Box<[(u32, u32)]>,
    exact_entries: Box<[NameId]>,
    folded_ids: Box<[NameId]>,
    path_ids: Box<[u32]>,
    buckets: Box<[u32]>,
    len: usize,
    used: usize,
}
impl NameTable {
    pub fn load<'a>(names: impl IntoIterator<Item = &'a [u8]>) -> Result<Self, NamesError> {
        Self::load_reserved(names, 0, 0)
    }
    pub fn load_reserved<'a>(
        names: impl IntoIterator<Item = &'a [u8]>,
        extra_names: usize,
        extra_bytes: usize,
    ) -> Result<Self, NamesError> {
        let mut unique: Vec<&[u8]> = vec![b""];
        unique.extend(names);
        unique.sort_unstable();
        unique.dedup();
        let capacity = unique
            .len()
            .checked_add(extra_names)
            .filter(|&n| n <= u32::MAX as usize)
            .ok_or(NamesError::Capacity)?;
        let bytes = unique
            .iter()
            .try_fold(extra_bytes, |sum, name| sum.checked_add(name.len()))
            .filter(|&n| n <= u32::MAX as usize)
            .ok_or(NamesError::Capacity)?;
        let buckets = capacity
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or(NamesError::Capacity)?;
        let mut table = Self {
            bytes: vec![0; bytes].into_boxed_slice(),
            entries: vec![(0, 0); capacity].into_boxed_slice(),
            exact_entries: vec![NameId(0); capacity].into_boxed_slice(),
            folded_ids: vec![NameId(0); capacity].into_boxed_slice(),
            path_ids: vec![u32::MAX; capacity].into_boxed_slice(),
            buckets: vec![u32::MAX; buckets].into_boxed_slice(),
            len: 0,
            used: 0,
        };
        for name in unique {
            table.intern(name)?;
        }
        Ok(table)
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn capacity(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn get(&self, id: NameId) -> Option<&[u8]> {
        if id.0 as usize >= self.len {
            return None;
        }
        let (offset, len) = self.entries[id.0 as usize];
        Some(&self.bytes[offset as usize..(offset + len) as usize])
    }
    pub fn find(&self, name: &[u8]) -> Option<NameId> {
        self.exact_entries[..self.len]
            .binary_search_by(|id| {
                let (offset, len) = self.entries[id.0 as usize];
                self.bytes[offset as usize..(offset + len) as usize].cmp(name)
            })
            .ok()
            .map(|index| self.exact_entries[index])
    }
    fn folded_bucket(&self, bytes: impl Iterator<Item = u8> + Clone) -> (usize, Option<NameId>) {
        let mut hash = 2166136261u32;
        for byte in bytes.clone() {
            hash = (hash ^ u32::from(byte)).wrapping_mul(16777619);
        }
        let mask = self.buckets.len() - 1;
        let mut bucket = hash as usize & mask;
        loop {
            let id = self.buckets[bucket];
            if id == u32::MAX {
                return (bucket, None);
            }
            let (offset, len) = self.entries[id as usize];
            if self.bytes[offset as usize..(offset + len) as usize]
                .iter()
                .copied()
                .map(fold)
                .eq(bytes.clone())
            {
                return (bucket, Some(NameId(id)));
            }
            bucket = (bucket + 1) & mask;
        }
    }
    pub fn find_folded(&self, name: &[u8]) -> Option<NameId> {
        self.folded_bucket(name.iter().copied().map(fold)).1
    }
    /// Allocation-free path lookup; registration interns canonical bytes once.
    pub fn find_path(&self, name: &str) -> Option<NameId> {
        let group = self
            .folded_bucket(name.as_bytes().iter().copied().map(path_byte))
            .1?;
        let id = self.path_ids[group.0 as usize];
        (id != u32::MAX).then_some(NameId(id))
    }
    pub fn folded(&self, id: NameId) -> Option<NameId> {
        ((id.0 as usize) < self.len).then(|| self.folded_ids[id.0 as usize])
    }
    pub fn intern(&mut self, name: &[u8]) -> Result<NameId, NamesError> {
        if let Some(id) = self.find(name) {
            return Ok(id);
        }
        let end = self
            .used
            .checked_add(name.len())
            .filter(|&n| n <= self.bytes.len())
            .ok_or(NamesError::Capacity)?;
        if self.len == self.entries.len() {
            return Err(NamesError::Capacity);
        }
        let at = self.exact_entries[..self.len]
            .binary_search_by(|id| {
                let (offset, len) = self.entries[id.0 as usize];
                self.bytes[offset as usize..(offset + len) as usize].cmp(name)
            })
            .unwrap_or_else(|at| at);
        let (bucket, folded) = self.folded_bucket(name.iter().copied().map(fold));
        let id = NameId(self.len as u32);
        self.bytes[self.used..end].copy_from_slice(name);
        self.entries[self.len] = (self.used as u32, name.len() as u32);
        self.folded_ids[self.len] = folded.unwrap_or(id);
        if name.iter().copied().all(|byte| byte == path_byte(byte)) {
            self.path_ids[folded.unwrap_or(id).0 as usize] = id.0;
        }
        self.exact_entries.copy_within(at..self.len, at + 1);
        self.exact_entries[at] = id;
        if folded.is_none() {
            self.buckets[bucket] = id.0;
        }
        self.len += 1;
        self.used = end;
        Ok(id)
    }
    /// Cold registration is permitted to allocate; repeated lookup does not.
    pub fn intern_path(&mut self, name: &str) -> Result<NameId, NamesError> {
        if let Some(id) = self.find_path(name) {
            return Ok(id);
        }
        let canonical = canonical_path(name);
        self.intern(canonical.as_bytes())
    }
}
