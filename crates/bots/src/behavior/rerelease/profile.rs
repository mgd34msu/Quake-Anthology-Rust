//! Rerelease bot profile from `src/bots/behavior/rerelease/profile.ts`.
//!
//! Ties a brain to a slot: name, skill, appearance, roster role, and
//! per-profile RNG seed and movement speeds. Profiles validate names
//! and resolve appearance from the knowledge character entries.

use crate::behavior::rerelease::data::botdata::CharacterEntry;
use crate::behavior::rerelease::data::knowledge::BotKnowledge;
use crate::error::BotsError;

/// Profile RNG seed derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RereleaseProfileSeed {
    /// Base seed.
    pub base: i32,
    /// Slot offset.
    pub slot: i32,
}

impl RereleaseProfileSeed {
    /// Derive the per-bot seed.
    #[must_use]
    pub fn seed(self) -> i32 {
        self.base.wrapping_add(self.slot.wrapping_mul(0x9e37_79b1u32 as i32))
    }
}

/// Rerelease bot profile.
#[derive(Debug, Clone)]
pub struct RereleaseProfile {
    /// Profile name.
    pub name: String,
    /// Skill name.
    pub skill: String,
    /// Character entry, when resolved.
    pub character: Option<CharacterEntry>,
    /// Run speed.
    pub run_speed: f32,
    /// Walk speed.
    pub walk_speed: f32,
    /// Seed derivation.
    pub seed: RereleaseProfileSeed,
    /// Roster role.
    pub role: String,
}

impl RereleaseProfile {
    /// New profile.
    pub fn new(name: &str, skill: &str, seed: RereleaseProfileSeed) -> Result<Self, BotsError> {
        if name.trim().is_empty() {
            return Err(BotsError::BotScript("rerelease bot profile needs a name".to_owned()));
        }
        if skill.trim().is_empty() {
            return Err(BotsError::BotScript("rerelease bot profile needs a skill".to_owned()));
        }
        Ok(Self {
            name: name.to_owned(),
            skill: skill.to_owned(),
            character: None,
            run_speed: super::path_follow::BOT_RUN_SPEED,
            walk_speed: super::path_follow::BOT_WALK_SPEED,
            seed,
            role: String::new(),
        })
    }

    /// Resolve appearance from knowledge.
    pub fn resolve_character(&mut self, knowledge: &BotKnowledge) {
        self.character = knowledge.character(&self.name).cloned();
    }

    /// Per-bot RNG seed.
    #[must_use]
    pub fn rng_seed(&self) -> i32 {
        self.seed.seed()
    }
}

/// Roster of rerelease profiles.
#[derive(Debug, Clone, Default)]
pub struct RereleaseRoster {
    profiles: Vec<RereleaseProfile>,
}

impl RereleaseRoster {
    /// Empty roster.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a profile.
    pub fn add(&mut self, profile: RereleaseProfile) {
        self.profiles.push(profile);
    }

    /// Profiles.
    #[must_use]
    pub fn profiles(&self) -> &[RereleaseProfile] {
        &self.profiles
    }

    /// Profile count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// Whether empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// Find a profile by name.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&RereleaseProfile> {
        self.profiles.iter().find(|profile| profile.name == name)
    }
}
