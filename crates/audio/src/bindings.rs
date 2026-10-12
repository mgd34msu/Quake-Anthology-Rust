use crate::Bank;
use qa_core::{
    primitives::{NameId, RuleSetId, SoundId},
    text::FixedText,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundBindingError {
    Ordinal,
    Sound,
    Text,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SoundNames {
    literal_prefixes: bool,
    full_path: bool,
}
impl SoundNames {
    fn for_rules(rules: RuleSetId) -> Self {
        Self {
            literal_prefixes: matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld),
            full_path: rules == RuleSetId::Quake3,
        }
    }
}

pub fn same_sound_codec(first: RuleSetId, second: RuleSetId) -> bool {
    SoundNames::for_rules(first) == SoundNames::for_rules(second)
}

/// Native resource names select player-relative or explicit file paths.
pub fn sound_path(
    name: &[u8],
    rules: RuleSetId,
) -> Result<Option<FixedText<8198>>, SoundBindingError> {
    if name.is_empty() {
        return Ok(None);
    }
    let policy = SoundNames::for_rules(rules);
    let mut path = FixedText::default();
    if policy.literal_prefixes {
        // Quake's loader treats both prefix bytes as literal filenames.
        path.append_bytes(b"sound/")
            .map_err(|_| SoundBindingError::Text)?;
        path.append_bytes(name)
            .map_err(|_| SoundBindingError::Text)?;
    } else if name.starts_with(b"*") {
        return Ok(None);
    } else if policy.full_path {
        path.append_bytes(name)
            .map_err(|_| SoundBindingError::Text)?;
    } else if let Some(explicit) = name.strip_prefix(b"#") {
        path.append_bytes(explicit)
            .map_err(|_| SoundBindingError::Text)?;
    } else {
        path.append_bytes(b"sound/")
            .map_err(|_| SoundBindingError::Text)?;
        path.append_bytes(name)
            .map_err(|_| SoundBindingError::Text)?;
    }
    Ok(Some(path))
}

/// The inverse filename codec uses the same policy as native resource binding.
/// Q1/QW cannot represent paths outside their literal sound/ namespace.
pub fn native_sound_name(rules: RuleSetId, bank: &Bank, sound: SoundId) -> Option<FixedText<8198>> {
    let path = bank.name(sound)?;
    let policy = SoundNames::for_rules(rules);
    let mut name = FixedText::default();
    if policy.full_path {
        name.append_bytes(path).ok()?;
    } else if policy.literal_prefixes {
        name.append_bytes(path.strip_prefix(b"sound/")?).ok()?;
    } else if let Some(relative) = path.strip_prefix(b"sound/")
        && !relative.starts_with(b"#")
        && !relative.starts_with(b"*")
    {
        name.append_bytes(relative).ok()?;
    } else {
        name.append_bytes(b"#").ok()?;
        name.append_bytes(path).ok()?;
    }
    Some(name)
}

/// One numeric binding table for a module or a received CLIENT namespace.
/// Registration resolves names; emission only reads numeric forward/inverse rows.
pub struct SoundBindings {
    forward: Box<[Option<SoundId>]>,
    inverse: Box<[Option<u32>]>,
    registration_keys: Box<[Option<u32>]>,
    valid_zero: bool,
}
impl SoundBindings {
    pub fn load(ordinals: usize) -> Self {
        Self::load_reserved(ordinals, 0, false)
    }
    /// Engine sound handles have a real default sample at zero. Configstring
    /// ordinals keep the default load() domain, where zero means no sound.
    pub fn load_with_zero(ordinals: usize) -> Self {
        Self::load_reserved(ordinals, 0, true)
    }
    pub(crate) fn load_reserved(ordinals: usize, sounds: usize, valid_zero: bool) -> Self {
        Self {
            forward: vec![None; ordinals].into_boxed_slice(),
            inverse: vec![None; sounds].into_boxed_slice(),
            registration_keys: vec![None; if valid_zero { sounds } else { 0 }].into_boxed_slice(),
            valid_zero,
        }
    }
    /// Cold bank attachment reserves the inverse once and clears old handles.
    pub fn attach(&mut self, bank: &Bank) {
        self.clear();
        if self.inverse.len() != bank.len() {
            self.inverse = vec![None; bank.len()].into_boxed_slice();
        }
        if self.valid_zero && self.registration_keys.len() != bank.capacity() {
            self.registration_keys = vec![None; bank.capacity()].into_boxed_slice();
        }
    }
    pub fn clear(&mut self) {
        self.forward.fill(None);
        self.inverse.fill(None);
        self.registration_keys.fill(None);
    }
    pub fn sound(&self, ordinal: u32) -> Option<SoundId> {
        self.forward.get(ordinal as usize).copied().flatten()
    }
    pub fn ordinal(&self, sound: SoundId) -> Option<u32> {
        self.inverse.get(sound.0 as usize).copied().flatten()
    }
    /// Duplicate registrations retain the lowest ordinal admitted by the domain.
    /// Replacing that row recomputes its fallback only during registration.
    pub fn set(&mut self, ordinal: u32, sound: Option<SoundId>) -> Result<(), SoundBindingError> {
        let index = ordinal as usize;
        let old = *self.forward.get(index).ok_or(SoundBindingError::Ordinal)?;
        let sound = if ordinal == 0 && !self.valid_zero {
            None
        } else {
            sound
        };
        if !self.inverse.is_empty()
            && sound.is_some_and(|sound| sound.0 as usize >= self.inverse.len())
        {
            return Err(SoundBindingError::Sound);
        }
        if old == sound {
            return Ok(());
        }
        for key in self.registration_keys.iter_mut() {
            if *key == Some(ordinal) {
                *key = None;
            }
        }
        self.forward[index] = sound;
        if let Some(old) = old
            && let Some(inverse) = self.inverse.get_mut(old.0 as usize)
            && *inverse == Some(ordinal)
        {
            *inverse = self
                .forward
                .iter()
                .enumerate()
                .skip(usize::from(!self.valid_zero))
                .find_map(|(index, &bound)| (bound == Some(old)).then_some(index as u32));
        }
        if let Some(sound) = sound
            && let Some(inverse) = self.inverse.get_mut(sound.0 as usize)
            && inverse.is_none_or(|old| ordinal < old)
        {
            *inverse = Some(ordinal);
        }
        Ok(())
    }
    /// Cold registration allocates the next native ordinal from the same
    /// forward/inverse rows. A failed PCM attempt can retain its numeric row.
    pub fn register(&mut self, sound: SoundId) -> Result<u32, SoundBindingError> {
        self.register_key(None, sound)
    }
    /// Distinct native folded names can share one canonical prepared sample.
    pub fn register_named(
        &mut self,
        name: NameId,
        sound: SoundId,
    ) -> Result<u32, SoundBindingError> {
        self.register_key(Some(name), sound)
    }
    pub fn named(&self, name: NameId) -> Option<u32> {
        self.registration_keys
            .get(name.0 as usize)
            .copied()
            .flatten()
    }
    /// Change only an admitted native row, preserving its allocated ordinal.
    pub fn replace_named(&mut self, name: NameId, sound: SoundId) -> Result<(), SoundBindingError> {
        let ordinal = self.named(name).ok_or(SoundBindingError::Ordinal)?;
        self.set(ordinal, Some(sound))?;
        self.registration_keys[name.0 as usize] = Some(ordinal);
        Ok(())
    }
    fn register_key(
        &mut self,
        name: Option<NameId>,
        sound: SoundId,
    ) -> Result<u32, SoundBindingError> {
        let inverse = self
            .inverse
            .get(sound.0 as usize)
            .ok_or(SoundBindingError::Sound)?;
        let existing = match name {
            Some(name) => *self
                .registration_keys
                .get(name.0 as usize)
                .ok_or(SoundBindingError::Sound)?,
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
            .ok_or(SoundBindingError::Ordinal)?;
        self.set(ordinal, Some(sound))?;
        if let Some(name) = name {
            self.registration_keys[name.0 as usize] = Some(ordinal);
        }
        Ok(ordinal)
    }
    /// A destination can preserve a load-bound primary native registration
    /// even when a secondary namespace already registered the same sample.
    pub fn prefer(&mut self, ordinal: u32) {
        if let Some(sound) = self.sound(ordinal)
            && let Some(inverse) = self.inverse.get_mut(sound.0 as usize)
        {
            *inverse = Some(ordinal);
        }
    }
    pub fn bind_name(
        &mut self,
        bank: Option<&Bank>,
        ordinal: u32,
        name: &[u8],
        rules: RuleSetId,
    ) -> Result<(), SoundBindingError> {
        match sound_path(name, rules)? {
            Some(path) => self.bind_path(bank, ordinal, path.as_bytes()),
            None => self.set(ordinal, None),
        }
    }
    /// Register an already decoded VFS path through the same numeric bindings.
    pub fn bind_path(
        &mut self,
        bank: Option<&Bank>,
        ordinal: u32,
        path: &[u8],
    ) -> Result<(), SoundBindingError> {
        let sound = bank.and_then(|bank| bank.find_bytes(path));
        self.set(ordinal, sound)
    }
}
