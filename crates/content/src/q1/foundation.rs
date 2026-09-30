//! Q1 foundation root (`src/content/q1/foundation`, barrel `index.ts`).
//!
//! Renames against the donor barrel: `Q1_PROVIDER` is
//! [`types::q1_provider`], `ammoItem` is
//! [`entity_services::ammo_item`], `Q1FoundationHost` lives in [`host`],
//! and `Q1Foundation` is [`entity_services::Q1EntityServices`] (the
//! donor subclass adds no Rust-visible surface).

pub mod callbacks;
pub mod checkpoint;
pub mod entity;
pub mod entity_services;
pub mod extensions;
pub mod gameplay;
pub mod held_weapons;
pub mod host;
pub mod monsters;
pub mod movers;
pub mod pickups;
pub mod precache;
pub mod precache_world;
pub mod runtime;
pub mod shambler_damage;
pub mod spawns;
pub mod text;
pub mod types;
pub mod weapon_names;
pub mod weapons;

pub use callbacks::{callback_name, Q1CallbackHandlers, Q1StateExtension};
pub use checkpoint::{
    save_q1_actor, Q1EntitySourceState, Q1FoundationCheckpoint, Q1SavedCallbacks, Q1SavedEntity, Q1SavedPlayer,
};
pub use entity::{
    move_direction, parse_vector, source_angles, Q1Actor, Q1Monster, Q1MonsterSpecies, Q1Move, Q1_MONSTER_SPECIES,
};
pub use entity_services::{ammo_item, Q1EntityServices};
pub use extensions::{Q1PickupRules, Q1PlayerExtension, Q1WeaponDefinition, Q1WeaponRules};
pub use host::Q1FoundationHost;
pub use pickups::{observe_q1_supply, spawn_pickup};
pub use runtime::Q1SpawnReport;
pub use spawns::{link_doors, spawn_map_actor, spawn_teledeath, spawn_teleport_fog};
pub use types::{
    is_q1_base_weapon, q1_provider, weapon_item, Q1BaseWeapon, Q1Basis, Q1Event, Q1FoundationOptions, Q1MoveType,
    Q1PlayerState, Q1Powerup, Q1Presentation, Q1Solid, Q1SoundChannel, Q1Trace, Q1TraceRequest, Q1Weapon,
    PLAYER_BOUNDS, Q1_POWERUP_IDS, Q1_WEAPON_IDS, WEAPONS,
};
pub use weapons::{best_weapon, fire_base_weapon, fire_bullets, fire_weapon, weapon_model};
