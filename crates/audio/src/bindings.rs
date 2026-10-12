use crate::Bank;
use qa_core::{
    primitives::{RuleSetId, SoundId},
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
}
impl SoundBindings {
    pub fn load(ordinals: usize) -> Self {
        Self {
            forward: vec![None; ordinals].into_boxed_slice(),
            inverse: Box::new([]),
        }
    }
    /// Cold bank attachment reserves the inverse once and clears old handles.
    pub fn attach(&mut self, bank: &Bank) {
        self.clear();
        if self.inverse.len() != bank.len() {
            self.inverse = vec![None; bank.len()].into_boxed_slice();
        }
    }
    pub fn clear(&mut self) {
        self.forward.fill(None);
        self.inverse.fill(None);
    }
    pub fn sound(&self, ordinal: u32) -> Option<SoundId> {
        self.forward.get(ordinal as usize).copied().flatten()
    }
    pub fn ordinal(&self, sound: SoundId) -> Option<u32> {
        self.inverse.get(sound.0 as usize).copied().flatten()
    }
    /// Duplicate registrations retain the lowest nonzero native ordinal.
    /// Replacing that row recomputes its fallback only during registration.
    pub fn set(&mut self, ordinal: u32, sound: Option<SoundId>) -> Result<(), SoundBindingError> {
        let index = ordinal as usize;
        let old = *self.forward.get(index).ok_or(SoundBindingError::Ordinal)?;
        let sound = if ordinal == 0 { None } else { sound };
        if !self.inverse.is_empty()
            && sound.is_some_and(|sound| sound.0 as usize >= self.inverse.len())
        {
            return Err(SoundBindingError::Sound);
        }
        if old == sound {
            return Ok(());
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
                .skip(1)
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
        let sound = match bank {
            Some(bank) => {
                bank.find(std::str::from_utf8(path).map_err(|_| SoundBindingError::Text)?)
            }
            None => None,
        };
        self.set(ordinal, sound)
    }
}
