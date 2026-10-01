//! Weapon checkpoint data (`src/content/q2/foundation/weapons/checkpoint.ts`).
//!
//! Data structs plus capture/restore helpers. Byte framing lives with
//! the `qa-app` persistence codecs.

use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;

use super::types::{Q2WeaponInput, Q2WeaponState};
use crate::q2::foundation::weapons::WeaponSourceRules;

/// Noise checkpoint (`Q2NoiseCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2NoiseCheckpoint {
    /// Noisy actor.
    pub actor: SavedActorId,
    /// Noise origin.
    pub origin: Vec3,
    /// Noise time.
    pub time: f64,
    /// Whether secondary.
    pub secondary: bool,
}

/// Weapon state checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WeaponStateEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// State.
    pub state: Q2WeaponState,
}

/// Weapon input checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WeaponInputEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// Input.
    pub input: Q2WeaponInput,
}

/// Noise checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WeaponNoiseEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// Primary noise.
    pub primary: Option<Q2NoiseCheckpoint>,
    /// Secondary noise.
    pub secondary: Option<Q2NoiseCheckpoint>,
}

/// Silencer checkpoint entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2SilencerEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// Charges.
    pub charges: i64,
}

/// Blaster cause checkpoint entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2BlasterCauseEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// Means of death.
    pub means_of_death: i32,
}

/// Weapons checkpoint (`Q2WeaponsCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WeaponsCheckpoint {
    /// Format version (always 2).
    pub format_version: u32,
    /// Silencer charges.
    pub silencer_charges: Vec<Q2SilencerEntry>,
    /// Source rules kind.
    pub source_rules: WeaponSourceRules,
    /// Registered weapon names.
    pub registered: Vec<String>,
    /// Fallback weapon order.
    pub fallback_order: Option<Vec<String>>,
    /// Weapon states.
    pub states: Vec<Q2WeaponStateEntry>,
    /// Weapon inputs.
    pub inputs: Vec<Q2WeaponInputEntry>,
    /// Noises.
    pub noises: Vec<Q2WeaponNoiseEntry>,
    /// Primary sound entity.
    pub sound_entity: Option<Q2NoiseCheckpoint>,
    /// Secondary sound entity.
    pub sound2_entity: Option<Q2NoiseCheckpoint>,
    /// Blaster causes.
    pub blaster_causes: Vec<Q2BlasterCauseEntry>,
}
