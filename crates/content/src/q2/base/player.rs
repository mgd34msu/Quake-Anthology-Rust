//! Q2 base players (`src/content/q2/base/player`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

pub mod character;
pub mod checkpoint;
pub mod commands;
pub mod environment;
pub mod index;
pub mod landmarks;
pub mod obituary;
pub mod resources;
pub mod spawns;
pub mod types;
pub mod view;

use std::collections::HashMap;

use qa_core::identity::ActorId;

use crate::q2::foundation::items::Q2ItemModule;

pub use index::{
    Q2ConnectionResult, Q2Intermission, Q2PlayerAdmission, Q2Players, create_q2_players,
    player_callbacks, player_hooks, player_items, spawn_player,
};
pub use types::{
    Q2BodyChanges, Q2CharacterContext, Q2CharacterWeapon, Q2PlayerCarry, Q2PlayerContext,
    Q2PlayerEvent, Q2PlayerHooks, Q2PlayerMovement, Q2PlayerMovementChange, Q2PlayerRules,
    Q2PlayerSpawnChange, Q2PlayerState, Q2PlayerView, Q2ScoreRow, create_q2_player_rules,
};

/// Arena runtime state for this module.
#[derive(Debug, Default)]
pub struct PlayerRuntime {
    /// Admitted player states by actor.
    pub states: HashMap<ActorId, types::Q2PlayerState>,
    /// Provider hooks.
    pub hooks: Option<types::Q2PlayerHooks>,
    /// Item module.
    pub items: Option<Q2ItemModule>,
    /// Player rules.
    pub rules: types::Q2PlayerRules,
    /// Intermission state.
    pub intermission: index::Q2Intermission,
    /// Corpse queue index.
    pub corpse_index: i32,
    /// Death animation cycle.
    pub death_animation: i32,
    /// Pain animation cycle.
    pub pain_animation: i32,
}
