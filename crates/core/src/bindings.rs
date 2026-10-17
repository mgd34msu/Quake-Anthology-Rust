//! Load-sized ordinal bindings with a cached numeric inverse.
use crate::primitives::{NameId, SoundId};

pub trait BindingId: Copy + Eq {
    fn index(self) -> usize;
}
impl BindingId for NameId {
    fn index(self) -> usize {
        self.0 as usize
    }
}
impl BindingId for SoundId {
    fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingError {
    Ordinal,
    Identity,
}

#[derive(Clone)]
pub struct OrdinalBindings<K> {
    forward: Box<[Option<K>]>,
    inverse: Box<[Option<u32>]>,
    registration_keys: Box<[Option<u32>]>,
    valid_zero: bool,
}
impl<K: BindingId> OrdinalBindings<K> {
    pub fn load(
        ordinals: usize,
        identities: usize,
        registration_keys: usize,
        valid_zero: bool,
    ) -> Self {
        Self {
            forward: vec![None; ordinals].into_boxed_slice(),
            inverse: vec![None; identities].into_boxed_slice(),
            registration_keys: vec![None; registration_keys].into_boxed_slice(),
            valid_zero,
        }
    }
    /// Cold attachment replaces capacities only when the owning arena changes.
    pub fn configure(&mut self, identities: usize, registration_keys: usize) {
        self.clear();
        if self.inverse.len() != identities {
            self.inverse = vec![None; identities].into_boxed_slice();
        }
        if self.registration_keys.len() != registration_keys {
            self.registration_keys = vec![None; registration_keys].into_boxed_slice();
        }
    }
    pub fn capacity(&self) -> usize {
        self.forward.len()
    }
    pub fn valid_zero(&self) -> bool {
        self.valid_zero
    }
    pub fn clear(&mut self) {
        self.forward.fill(None);
        self.inverse.fill(None);
        self.registration_keys.fill(None);
    }
    pub fn get(&self, ordinal: u32) -> Option<K> {
        self.forward.get(ordinal as usize).copied().flatten()
    }
    pub fn ordinal(&self, identity: K) -> Option<u32> {
        self.inverse.get(identity.index()).copied().flatten()
    }
    /// Duplicate identities normally retain their lowest admitted ordinal.
    /// Replacing that row repairs the inverse during registration only.
    pub fn set(&mut self, ordinal: u32, identity: Option<K>) -> Result<(), BindingError> {
        let index = ordinal as usize;
        let old = *self.forward.get(index).ok_or(BindingError::Ordinal)?;
        let identity = if ordinal == 0 && !self.valid_zero {
            None
        } else {
            identity
        };
        if !self.inverse.is_empty()
            && identity.is_some_and(|identity| identity.index() >= self.inverse.len())
        {
            return Err(BindingError::Identity);
        }
        if old == identity {
            return Ok(());
        }
        for key in self.registration_keys.iter_mut() {
            if *key == Some(ordinal) {
                *key = None;
            }
        }
        self.forward[index] = identity;
        if let Some(old) = old
            && let Some(inverse) = self.inverse.get_mut(old.index())
            && *inverse == Some(ordinal)
        {
            *inverse = self
                .forward
                .iter()
                .enumerate()
                .skip(usize::from(!self.valid_zero))
                .find_map(|(index, &bound)| (bound == Some(old)).then_some(index as u32));
        }
        if let Some(identity) = identity
            && let Some(inverse) = self.inverse.get_mut(identity.index())
            && inverse.is_none_or(|old| ordinal < old)
        {
            *inverse = Some(ordinal);
        }
        Ok(())
    }
    pub fn named(&self, name: NameId) -> Option<u32> {
        self.registration_keys.get(name.index()).copied().flatten()
    }
    pub fn replace_named(&mut self, name: NameId, identity: K) -> Result<(), BindingError> {
        let ordinal = self.named(name).ok_or(BindingError::Ordinal)?;
        self.set(ordinal, Some(identity))?;
        self.registration_keys[name.index()] = Some(ordinal);
        Ok(())
    }
    /// Registration is cold; delivery only reads forward and inverse entries.
    pub fn register(&mut self, name: Option<NameId>, identity: K) -> Result<u32, BindingError> {
        let inverse = self
            .inverse
            .get(identity.index())
            .ok_or(BindingError::Identity)?;
        let existing = match name {
            Some(name) => *self
                .registration_keys
                .get(name.index())
                .ok_or(BindingError::Identity)?,
            None => *inverse,
        };
        if let Some(ordinal) = existing {
            return Ok(ordinal);
        }
        let ordinal = self
            .forward
            .iter()
            .enumerate()
            .skip(usize::from(!self.valid_zero))
            .find_map(|(index, bound)| bound.is_none().then_some(index))
            .and_then(|index| u32::try_from(index).ok())
            .ok_or(BindingError::Ordinal)?;
        self.set(ordinal, Some(identity))?;
        if let Some(name) = name {
            self.registration_keys[name.index()] = Some(ordinal);
        }
        Ok(ordinal)
    }
    /// A primary native registration can override a secondary inverse choice.
    pub fn prefer(&mut self, ordinal: u32) {
        if let Some(identity) = self.get(ordinal)
            && let Some(inverse) = self.inverse.get_mut(identity.index())
        {
            *inverse = Some(ordinal);
        }
    }
}
