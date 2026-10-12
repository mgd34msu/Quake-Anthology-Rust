use crate::Bank;
use qa_core::{primitives::SoundId, text::FixedText};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundBindingError {
    Ordinal,
    Sound,
    Text,
}

/// Native resource names select player-relative or explicit file paths.
pub fn sound_path(name: &[u8]) -> Result<Option<FixedText<8198>>, SoundBindingError> {
    if name.is_empty() || name.starts_with(b"*") {
        return Ok(None);
    }
    let mut path = FixedText::default();
    if let Some(explicit) = name.strip_prefix(b"#") {
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
        self.forward.fill(None);
        if self.inverse.len() != bank.len() {
            self.inverse = vec![None; bank.len()].into_boxed_slice();
        } else {
            self.inverse.fill(None);
        }
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
    pub fn bind_name(
        &mut self,
        bank: Option<&Bank>,
        ordinal: u32,
        name: &[u8],
    ) -> Result<(), SoundBindingError> {
        let sound = match bank.zip(sound_path(name)?) {
            Some((bank, path)) => bank
                .find(std::str::from_utf8(path.as_bytes()).map_err(|_| SoundBindingError::Text)?),
            None => None,
        };
        self.set(ordinal, sound)
    }
}
