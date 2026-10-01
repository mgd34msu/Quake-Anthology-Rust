//! Name-to-slot index with source lookup order: a matching name must occur
//! before the first free slot.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/core/resource-name-index.ts`.

use std::collections::{HashMap, HashSet};

use thiserror::Error;

#[derive(Debug, Clone, Default)]
struct NameSlots {
    slots: HashSet<usize>,
    first: usize,
}

/// Index errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ResourceNameIndexError {
    /// Capacity must be a positive integer.
    #[error("Invalid resource capacity")]
    InvalidCapacity,
}

/// Maps resource names to slots; slot 0 is the empty-name sentinel.
#[derive(Debug, Clone)]
pub struct ResourceNameIndex {
    values: Vec<String>,
    reserved: HashSet<usize>,
    names: HashMap<String, NameSlots>,
    free: usize,
}

impl ResourceNameIndex {
    /// Build an index with `capacity` slots (slot 0 reserved as sentinel).
    pub fn new(capacity: usize, reserved: &[usize]) -> Result<Self, ResourceNameIndexError> {
        if capacity < 1 {
            return Err(ResourceNameIndexError::InvalidCapacity);
        }
        let mut index = Self {
            values: vec![String::new(); capacity],
            reserved: reserved.iter().copied().collect(),
            names: HashMap::new(),
            free: 1,
        };
        index.advance();
        Ok(index)
    }

    /// Clear every slot and name.
    pub fn clear(&mut self) {
        self.values.iter_mut().for_each(|value| value.clear());
        self.names.clear();
        self.free = 1;
        self.advance();
    }

    /// Assign `value` to `index`; out-of-range and reserved slots are ignored.
    pub fn set(&mut self, index: usize, value: impl Into<String>) {
        if index == 0 || index >= self.values.len() || self.reserved.contains(&index) {
            return;
        }
        let value = value.into();
        let previous = self.values[index].clone();
        if previous == value {
            return;
        }
        if !previous.is_empty() {
            let remove = match self.names.get_mut(&previous) {
                Some(entry) => {
                    entry.slots.remove(&index);
                    if entry.slots.is_empty() {
                        true
                    } else {
                        if entry.first == index {
                            entry.first = self.values.len();
                            for slot in entry.slots.iter() {
                                entry.first = entry.first.min(*slot);
                            }
                        }
                        false
                    }
                }
                None => false,
            };
            if remove {
                self.names.remove(&previous);
            }
        }
        self.values[index] = value.clone();
        if value.is_empty() {
            self.free = self.free.min(index);
        } else {
            let entry = self.names.entry(value).or_insert_with(|| NameSlots {
                slots: HashSet::new(),
                first: index,
            });
            entry.slots.insert(index);
            entry.first = entry.first.min(index);
            if index == self.free {
                self.advance();
            }
        }
    }

    /// Resolve `name`: the empty name maps to slot 0, a stored name to its
    /// first slot when it precedes the first free slot, otherwise the first
    /// free slot, or `None` when the index is full.
    pub fn find(&self, name: &str) -> Option<usize> {
        if name.is_empty() {
            return Some(0);
        }
        if let Some(entry) = self.names.get(name) {
            if entry.first < self.free {
                return Some(entry.first);
            }
        }
        if self.free < self.values.len() {
            Some(self.free)
        } else {
            None
        }
    }

    fn advance(&mut self) {
        while self.free < self.values.len()
            && (self.reserved.contains(&self.free) || !self.values[self.free].is_empty())
        {
            self.free += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_capacity() {
        assert_eq!(
            ResourceNameIndex::new(0, &[]).unwrap_err(),
            ResourceNameIndexError::InvalidCapacity
        );
    }

    #[test]
    fn empty_name_maps_to_slot_zero() {
        let index = ResourceNameIndex::new(4, &[]).expect("capacity");
        assert_eq!(index.find(""), Some(0));
    }

    #[test]
    fn find_returns_stored_name_before_free() {
        let mut index = ResourceNameIndex::new(4, &[]).expect("capacity");
        index.set(1, "player");
        assert_eq!(index.find("player"), Some(1));
        assert_eq!(index.find("unknown"), Some(2));
    }

    #[test]
    fn full_index_returns_none() {
        let mut index = ResourceNameIndex::new(2, &[]).expect("capacity");
        index.set(1, "only");
        assert_eq!(index.find("missing"), None);
        assert_eq!(index.find("only"), Some(1));
    }

    #[test]
    fn clearing_frees_slots() {
        let mut index = ResourceNameIndex::new(3, &[]).expect("capacity");
        index.set(1, "a");
        index.set(2, "b");
        index.clear();
        assert_eq!(index.find("a"), Some(1));
        assert_eq!(index.find(""), Some(0));
    }

    #[test]
    fn duplicate_names_track_first_slot() {
        let mut index = ResourceNameIndex::new(5, &[]).expect("capacity");
        index.set(2, "dup");
        index.set(1, "dup");
        assert_eq!(index.find("dup"), Some(1));
        index.set(1, "");
        assert_eq!(index.find("dup"), Some(1));
        index.set(1, "other");
        assert_eq!(index.find("dup"), Some(2));
    }

    #[test]
    fn reserved_slots_are_skipped() {
        let mut index = ResourceNameIndex::new(4, &[1]).expect("capacity");
        index.set(1, "ignored");
        assert_eq!(index.find("new"), Some(2));
        index.set(2, "kept");
        assert_eq!(index.find("new"), Some(3));
    }
}
