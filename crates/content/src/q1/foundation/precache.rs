//! Q1 precache registry (`src/content/q1/foundation/precache.ts`).
//!
//! WinQuake SV_SpawnServer and PF_precache_model/PF_precache_sound.
//! GPL-2.0-or-later.

use std::collections::HashMap;

use super::types::{Q1PrecachePhase, Q1PrecacheTables};
use crate::q1::{q1_error, Q1Error};

/// Ordered-name precache registry (`Q1PrecacheRegistry`). Ordered names
/// project source declarations; mounted content still owns resource
/// bytes.
#[derive(Debug, Default)]
pub struct Q1PrecacheRegistry {
    phase: Q1PrecachePhase,
    models: Vec<String>,
    sounds: Vec<String>,
    model_indices: HashMap<String, usize>,
    sound_indices: HashMap<String, usize>,
}

impl Q1PrecacheRegistry {
    /// Fresh loading registry with the source empty string in slot
    /// zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            phase: Q1PrecachePhase::Loading,
            models: vec![String::new()],
            sounds: vec![String::new()],
            model_indices: HashMap::from([(String::new(), 0)]),
            sound_indices: HashMap::from([(String::new(), 0)]),
        }
    }

    /// Registry phase.
    #[must_use]
    pub fn phase(&self) -> Q1PrecachePhase {
        self.phase
    }

    /// Declared models.
    #[must_use]
    pub fn models(&self) -> &[String] {
        &self.models
    }

    /// Declared sounds.
    #[must_use]
    pub fn sounds(&self) -> &[String] {
        &self.sounds
    }

    /// Declare world and inline models before source spawn declarations
    /// (`beginWorld`).
    pub fn begin_world(&mut self, path: &str, inline_models: i32) -> Result<(), Q1Error> {
        if self.phase != Q1PrecachePhase::Loading || self.models.len() != 1 || self.sounds.len() != 1 {
            return Err(q1_error("Q1 world precaches must precede source spawn declarations"));
        }
        if inline_models < 0 {
            return Err(q1_error("Invalid Q1 inline model count"));
        }
        self.model(path)?;
        for ordinal in 1..=inline_models {
            self.model(&format!("*{ordinal}"))?;
        }
        Ok(())
    }

    /// Declare a model, returning the path.
    pub fn model(&mut self, path: &str) -> Result<String, Q1Error> {
        Self::declare(self.phase, path, &mut self.models, &mut self.model_indices)
    }

    /// Declare a sound, returning the path.
    pub fn sound(&mut self, path: &str) -> Result<String, Q1Error> {
        Self::declare(self.phase, path, &mut self.sounds, &mut self.sound_indices)
    }

    /// Seal the registry after spawn.
    pub fn freeze(&mut self) {
        self.phase = Q1PrecachePhase::Frozen;
    }

    /// Restore tables into a fresh registry.
    pub fn restore(&mut self, tables: &Q1PrecacheTables) -> Result<(), Q1Error> {
        if self.phase != Q1PrecachePhase::Loading || self.models.len() != 1 || self.sounds.len() != 1 {
            return Err(q1_error("Restore Q1 precaches into a fresh registry"));
        }
        let models = Self::validate(&tables.models)?;
        let sounds = Self::validate(&tables.sounds)?;
        self.models = tables.models.clone();
        self.sounds = tables.sounds.clone();
        self.model_indices = models;
        self.sound_indices = sounds;
        self.phase = tables.phase;
        Ok(())
    }

    fn validate(names: &[String]) -> Result<HashMap<String, usize>, Q1Error> {
        if names.first().map(String::as_str) != Some("") {
            return Err(q1_error("Saved Q1 precache slot zero must be empty"));
        }
        let mut indices = HashMap::new();
        for (index, path) in names.iter().enumerate() {
            let leading = path.chars().next().map(|char| char as u32).unwrap_or(0);
            if indices.contains_key(path) || (index != 0 && (path.is_empty() || leading <= 32 || path.contains('\0'))) {
                return Err(q1_error("Invalid saved Q1 precache declaration"));
            }
            indices.insert(path.clone(), index);
        }
        Ok(indices)
    }

    fn declare(
        phase: Q1PrecachePhase,
        path: &str,
        names: &mut Vec<String>,
        indices: &mut HashMap<String, usize>,
    ) -> Result<String, Q1Error> {
        if phase != Q1PrecachePhase::Loading {
            return Err(q1_error("Q1 precache can only be done in spawn functions"));
        }
        // Donor `charCodeAt(0) <= 32` compares UTF-16 code units; the
        // ASCII control/space range matches on scalar values.
        let leading = path.chars().next().map(|char| char as u32).unwrap_or(0);
        if path.is_empty() || leading <= 32 || path.contains('\0') {
            return Err(q1_error("Invalid Q1 precache string"));
        }
        if !indices.contains_key(path) {
            indices.insert(path.to_string(), names.len());
            names.push(path.to_string());
        }
        Ok(path.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_precaches_precede_declarations() {
        let mut registry = Q1PrecacheRegistry::new();
        registry.begin_world("maps/e1m1.bsp", 2).expect("world");
        assert_eq!(registry.models(), &["", "maps/e1m1.bsp", "*1", "*2"]);
        assert!(registry.begin_world("maps/e1m2.bsp", 0).is_err());
        assert_eq!(registry.model("progs/player.mdl").expect("model"), "progs/player.mdl");
        registry.freeze();
        assert!(registry.sound("misc/talk.wav").is_err());
    }
}
