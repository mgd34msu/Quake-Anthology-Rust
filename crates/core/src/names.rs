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
pub fn path_byte(byte: u8) -> u8 {
    fold(if byte == b'\\' { b'/' } else { byte })
}
/// Native COM_DefaultExtension ignores dots before the final directory
/// separator. Callers retain the source's backslash interpretation explicitly.
pub fn default_extension<const N: usize>(
    path: &mut crate::text::FixedText<N>,
    extension: &[u8],
    backslash_separator: bool,
) -> std::fmt::Result {
    let component = path
        .as_bytes()
        .rsplit(|&byte| byte == b'/' || backslash_separator && byte == b'\\')
        .next()
        .unwrap_or_default();
    if component.contains(&b'.') {
        Ok(())
    } else {
        path.append_bytes(extension)
    }
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
        let mut table = Self::allocate(capacity, bytes)?;
        for name in unique {
            table.intern(name)?;
        }
        Ok(table)
    }
    fn allocate(capacity: usize, bytes: usize) -> Result<Self, NamesError> {
        if capacity == 0 || capacity > u32::MAX as usize || bytes > u32::MAX as usize {
            return Err(NamesError::Capacity);
        }
        let buckets = capacity
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or(NamesError::Capacity)?;
        Ok(Self {
            bytes: vec![0; bytes].into_boxed_slice(),
            entries: vec![(0, 0); capacity].into_boxed_slice(),
            exact_entries: vec![NameId(0); capacity].into_boxed_slice(),
            folded_ids: vec![NameId(0); capacity].into_boxed_slice(),
            path_ids: vec![u32::MAX; capacity].into_boxed_slice(),
            buckets: vec![u32::MAX; buckets].into_boxed_slice(),
            len: 0,
            used: 0,
        })
    }
    /// Expand the load reservation without reassigning existing identities.
    /// Call only before module callbacks; runtime intern never grows storage.
    pub fn reserve(
        &mut self,
        additional_names: usize,
        additional_bytes: usize,
    ) -> Result<(), NamesError> {
        if additional_names == 0 && additional_bytes == 0 {
            return Ok(());
        }
        let capacity = self
            .capacity()
            .checked_add(additional_names)
            .ok_or(NamesError::Capacity)?;
        let bytes = self
            .bytes
            .len()
            .checked_add(additional_bytes)
            .ok_or(NamesError::Capacity)?;
        let mut reserved = Self::allocate(capacity, bytes)?;
        reserved.bytes[..self.used].copy_from_slice(&self.bytes[..self.used]);
        reserved.entries[..self.len].copy_from_slice(&self.entries[..self.len]);
        reserved.exact_entries[..self.len].copy_from_slice(&self.exact_entries[..self.len]);
        reserved.folded_ids[..self.len].copy_from_slice(&self.folded_ids[..self.len]);
        reserved.path_ids[..self.len].copy_from_slice(&self.path_ids[..self.len]);
        reserved.len = self.len;
        reserved.used = self.used;
        for index in 0..reserved.len {
            if reserved.folded_ids[index] == NameId(index as u32) {
                let (offset, length) = reserved.entries[index];
                let bytes = &reserved.bytes[offset as usize..(offset + length) as usize];
                let (bucket, _) = reserved.folded_bucket(bytes.iter().copied().map(fold));
                reserved.buckets[bucket] = index as u32;
            }
        }
        *self = reserved;
        Ok(())
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn capacity(&self) -> usize {
        self.entries.len()
    }
    pub fn byte_capacity(&self) -> usize {
        self.bytes.len()
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
        self.find_path_bytes(name.as_bytes())
    }
    /// Native filenames remain byte strings; only ASCII and separators fold.
    pub fn find_path_bytes(&self, name: &[u8]) -> Option<NameId> {
        let group = self.folded_bucket(name.iter().copied().map(path_byte)).1?;
        let id = self.path_ids[group.0 as usize];
        (id != u32::MAX).then_some(NameId(id))
    }
    pub fn folded(&self, id: NameId) -> Option<NameId> {
        ((id.0 as usize) < self.len).then(|| self.folded_ids[id.0 as usize])
    }
    fn check_capacity(
        &self,
        additional_names: usize,
        additional_bytes: usize,
    ) -> Result<usize, NamesError> {
        self.len
            .checked_add(additional_names)
            .filter(|&count| count <= self.entries.len())
            .ok_or(NamesError::Capacity)?;
        self.used
            .checked_add(additional_bytes)
            .filter(|&count| count <= self.bytes.len())
            .ok_or(NamesError::Capacity)
    }
    /// Check a cold batch against this table's remaining reservation without
    /// mutating it or allocating. Exact repeats consume no additional rows.
    /// Intern the checked batch before admitting unrelated names.
    pub fn check_intern_all<'a>(
        &self,
        names: impl Iterator<Item = &'a [u8]> + Clone,
    ) -> Result<(), NamesError> {
        let mut additional_names = 0usize;
        let mut additional_bytes = 0usize;
        for (index, name) in names.clone().enumerate() {
            if self.find(name).is_some() || names.clone().take(index).any(|earlier| earlier == name)
            {
                continue;
            }
            additional_names = additional_names
                .checked_add(1)
                .ok_or(NamesError::Capacity)?;
            additional_bytes = additional_bytes
                .checked_add(name.len())
                .ok_or(NamesError::Capacity)?;
        }
        self.check_capacity(additional_names, additional_bytes)
            .map(|_| ())
    }
    pub fn intern(&mut self, name: &[u8]) -> Result<NameId, NamesError> {
        if let Some(id) = self.find(name) {
            return Ok(id);
        }
        let end = self.check_capacity(1, name.len())?;
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
        self.intern_path_bytes(name.as_bytes())
    }
    pub fn intern_path_bytes(&mut self, name: &[u8]) -> Result<NameId, NamesError> {
        if let Some(id) = self.find_path_bytes(name) {
            return Ok(id);
        }
        let canonical: Vec<_> = name.iter().copied().map(path_byte).collect();
        self.intern(&canonical)
    }
}
