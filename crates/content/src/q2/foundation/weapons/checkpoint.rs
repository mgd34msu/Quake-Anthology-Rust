//! Weapon checkpoint data (`src/content/q2/foundation/weapons/checkpoint.ts`).
//!
//! Data structs plus capture/restore helpers. Byte framing lives with
//! the `qa-app` persistence codecs.

use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;

use super::types::{Q2NoiseRecord, Q2WeaponInput, Q2WeaponState};
use crate::q2::foundation::checkpoint::restore_q2_actor;
use crate::q2::foundation::host::Q2GameServices;
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

/// Save a noise record (`saveNoise`).
fn save_q2_noise(noise: &Q2NoiseRecord) -> Q2NoiseCheckpoint {
    Q2NoiseCheckpoint {
        actor: SavedActorId::from(&noise.actor),
        origin: noise.origin,
        time: noise.time,
        secondary: noise.secondary,
    }
}

/// Restore a noise record (`restoreNoise`).
fn restore_q2_noise(game: &mut Q2GameServices, noise: &Q2NoiseCheckpoint) -> Q2NoiseRecord {
    let actor = match game.host.actors().resolve_saved(noise.actor) {
        Some(owned) => owned.id().clone(),
        None => game.host.actors().reference_saved(noise.actor),
    };
    Q2NoiseRecord {
        actor,
        origin: noise.origin,
        time: noise.time,
        secondary: noise.secondary,
    }
}

/// Capture weapon state (`Q2Weapons::capture`).
pub fn capture_q2_weapons(game: &mut Q2GameServices) -> Q2WeaponsCheckpoint {
    let mut silencer_charges = Vec::new();
    for (actor, charges) in &game.weapons.silencer_charges {
        if game.host.actors().is_live(actor) {
            silencer_charges.push(Q2SilencerEntry {
                actor: SavedActorId::from(actor),
                charges: *charges,
            });
        }
    }
    let mut registered: Vec<String> = game.weapons.definitions.keys().cloned().collect();
    registered.sort();
    let mut states = Vec::new();
    for (actor, state) in &game.weapons.states {
        if game.host.actors().is_live(actor) {
            states.push(Q2WeaponStateEntry {
                actor: SavedActorId::from(actor),
                state: state.clone(),
            });
        }
    }
    let mut inputs = Vec::new();
    for (actor, input) in &game.weapons.inputs {
        if game.host.actors().is_live(actor) {
            inputs.push(Q2WeaponInputEntry {
                actor: SavedActorId::from(actor),
                input: input.clone(),
            });
        }
    }
    let mut noises = Vec::new();
    for (actor, records) in &game.weapons.noises {
        if game.host.actors().is_live(actor) {
            noises.push(Q2WeaponNoiseEntry {
                actor: SavedActorId::from(actor),
                primary: records.primary.as_ref().map(save_q2_noise),
                secondary: records.secondary.as_ref().map(save_q2_noise),
            });
        }
    }
    let mut blaster_causes = Vec::new();
    for (actor, means_of_death) in &game.weapons.blaster_causes {
        if game.host.actors().is_live(actor) {
            blaster_causes.push(Q2BlasterCauseEntry {
                actor: SavedActorId::from(actor),
                means_of_death: *means_of_death,
            });
        }
    }
    Q2WeaponsCheckpoint {
        format_version: 2,
        silencer_charges,
        source_rules: game.weapons.source_rules.unwrap_or_default(),
        registered,
        fallback_order: game.weapons.fallback_order.clone(),
        states,
        inputs,
        noises,
        sound_entity: game.weapons.sound_entity.as_ref().map(save_q2_noise),
        sound2_entity: game.weapons.sound2_entity.as_ref().map(save_q2_noise),
        blaster_causes,
    }
}

/// Restore weapon state (`Q2Weapons::restore`).
///
/// The donor's release wiring (`trackActors`) is arena-central in the
/// port (`WeaponRuntime::on_actor_released`), so restore only rebuilds
/// the maps.
pub fn restore_q2_weapons(game: &mut Q2GameServices, checkpoint: &Q2WeaponsCheckpoint) {
    if checkpoint.source_rules != game.weapons.source_rules.unwrap_or_default() {
        panic!("Q2 weapon checkpoint source rules differ from the selected game module");
    }
    if checkpoint.registered.len() != game.weapons.definitions.len()
        || checkpoint
            .registered
            .iter()
            .any(|name| !game.weapons.definitions.contains_key(name))
    {
        panic!("Q2 weapon checkpoint arsenal differs from the selected source modules");
    }
    game.weapons.states.clear();
    game.weapons.inputs.clear();
    game.weapons.noises.clear();
    game.weapons.blaster_causes.clear();
    game.weapons.silencer_charges.clear();
    game.weapons.fallback_order = checkpoint.fallback_order.clone();
    if let Some(order) = checkpoint.fallback_order.clone() {
        for name in &order {
            super::player::weapon_definition(game, name);
        }
    }
    for saved in &checkpoint.states {
        let owned = restore_q2_actor(game, saved.actor);
        for name in [&saved.state.weapon, &saved.state.pending, &saved.state.last_weapon]
            .into_iter()
            .flatten()
        {
            super::player::weapon_definition(game, name);
        }
        super::player::bind_player_weapon(game, &owned, saved.state.clone());
    }
    for saved in &checkpoint.silencer_charges {
        let owned = restore_q2_actor(game, saved.actor);
        super::ballistics::grant_silencer(game, owned.id().clone(), saved.charges);
    }
    for saved in &checkpoint.inputs {
        let owned = restore_q2_actor(game, saved.actor);
        game.weapons.inputs.insert(owned.id().clone(), saved.input.clone());
    }
    for saved in &checkpoint.noises {
        let owned = restore_q2_actor(game, saved.actor);
        let primary = saved.primary.as_ref().map(|noise| restore_q2_noise(game, noise));
        let secondary = saved.secondary.as_ref().map(|noise| restore_q2_noise(game, noise));
        game.weapons
            .noises
            .insert(owned.id().clone(), super::WeaponNoises { primary, secondary });
    }
    game.weapons.sound_entity = checkpoint
        .sound_entity
        .as_ref()
        .map(|noise| restore_q2_noise(game, noise));
    game.weapons.sound2_entity = checkpoint
        .sound2_entity
        .as_ref()
        .map(|noise| restore_q2_noise(game, noise));
    for saved in &checkpoint.blaster_causes {
        let owned = restore_q2_actor(game, saved.actor);
        game.weapons
            .blaster_causes
            .insert(owned.id().clone(), saved.means_of_death);
    }
    game.source_callbacks
        .register(&super::ballistics::q2_ballistics_callbacks());
}
