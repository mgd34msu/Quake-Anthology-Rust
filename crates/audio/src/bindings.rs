use crate::Bank;
use qa_core::{
    bindings::{BindingError, OrdinalBindings},
    primitives::{NameId, RuleSetId, SoundId},
    text::FixedText,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundBindingError {
    Ordinal,
    Sound,
    Text,
}

impl From<BindingError> for SoundBindingError {
    fn from(error: BindingError) -> Self {
        match error {
            BindingError::Ordinal => Self::Ordinal,
            BindingError::Identity => Self::Sound,
        }
    }
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
    bindings: OrdinalBindings<SoundId>,
}
impl SoundBindings {
    pub fn load(ordinals: usize) -> Self {
        Self::load_reserved(ordinals, 0, false)
    }
    /// Engine sound handles admit a real default at zero; wire ordinals do not.
    pub fn load_with_zero(ordinals: usize) -> Self {
        Self::load_reserved(ordinals, 0, true)
    }
    pub(crate) fn load_reserved(ordinals: usize, sounds: usize, valid_zero: bool) -> Self {
        Self {
            bindings: OrdinalBindings::load(
                ordinals,
                sounds,
                if valid_zero { sounds } else { 0 },
                valid_zero,
            ),
        }
    }
    pub fn attach(&mut self, bank: &Bank) {
        self.bindings.configure(
            bank.capacity(),
            if self.bindings.valid_zero() {
                bank.capacity()
            } else {
                0
            },
        );
    }
    pub fn clear(&mut self) {
        self.bindings.clear();
    }
    pub fn sound(&self, ordinal: u32) -> Option<SoundId> {
        self.bindings.get(ordinal)
    }
    pub fn ordinal(&self, sound: SoundId) -> Option<u32> {
        self.bindings.ordinal(sound)
    }
    pub fn set(&mut self, ordinal: u32, sound: Option<SoundId>) -> Result<(), SoundBindingError> {
        self.bindings.set(ordinal, sound).map_err(Into::into)
    }
    pub fn register(&mut self, sound: SoundId) -> Result<u32, SoundBindingError> {
        self.bindings.register(None, sound).map_err(Into::into)
    }
    pub fn register_named(
        &mut self,
        name: NameId,
        sound: SoundId,
    ) -> Result<u32, SoundBindingError> {
        self.bindings
            .register(Some(name), sound)
            .map_err(Into::into)
    }
    pub fn named(&self, name: NameId) -> Option<u32> {
        self.bindings.named(name)
    }
    pub fn replace_named(&mut self, name: NameId, sound: SoundId) -> Result<(), SoundBindingError> {
        self.bindings.replace_named(name, sound).map_err(Into::into)
    }
    pub fn prefer(&mut self, ordinal: u32) {
        self.bindings.prefer(ordinal);
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
