//! Q2 weapons (`src/content/q2/foundation/weapons`).
//!
//! The donor's `Q2Ballistics`/`Q2Weapons` classes become arena state
//! ([`WeaponRuntime`]) plus free functions.
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use std::collections::HashMap;

use qa_core::identity::ActorId;

use self::types::{Q2NoiseRecord, Q2WeaponDefinition, Q2WeaponInput, Q2WeaponState, WeaponEngine};

pub mod ballistics;
pub mod checkpoint;
pub mod damage;
pub mod definitions;
pub mod generic_frame;
pub mod hand_action;
pub mod hand_grenade;
pub mod player;
pub mod presentation;
pub mod projection;
pub mod turn;
pub mod types;
pub mod vectors;

/// Per-actor noise records.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WeaponNoises {
    /// Primary noise.
    pub primary: Option<Q2NoiseRecord>,
    /// Secondary noise.
    pub secondary: Option<Q2NoiseRecord>,
}

/// Weapon source rules (`Q2WeaponSourceRules["kind"]`).
///
/// CTF haste and LMCTF rune frames dispatch to the match runtimes; the
/// donor's hook closures become arena calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WeaponSourceRules {
    /// Base rules.
    #[default]
    Base,
    /// CTF rules.
    Ctf,
    /// LMCTF rules.
    Lmctf,
}

/// Weapon arena runtime.
pub struct WeaponRuntime {
    /// Blaster causes by projectile.
    pub blaster_causes: HashMap<ActorId, i32>,
    /// Silencer charges by actor.
    pub silencer_charges: HashMap<ActorId, i64>,
    /// Weapon states by actor.
    pub states: HashMap<ActorId, Q2WeaponState>,
    /// Weapon inputs by actor.
    pub inputs: HashMap<ActorId, Q2WeaponInput>,
    /// Noise records by actor.
    pub noises: HashMap<ActorId, WeaponNoises>,
    /// Primary sound entity.
    pub sound_entity: Option<Q2NoiseRecord>,
    /// Secondary sound entity.
    pub sound2_entity: Option<Q2NoiseRecord>,
    /// Session engine.
    pub engine: Option<Box<dyn WeaponEngine>>,
    /// Source rules.
    pub source_rules: Option<WeaponSourceRules>,
    /// Registered weapon definitions by name.
    pub definitions: HashMap<String, Q2WeaponDefinition>,
    /// Fallback weapon order.
    pub fallback_order: Option<Vec<String>>,
    /// Weapon extensions by name.
    pub extensions: HashMap<String, Box<dyn player::Q2WeaponExtension>>,
    /// Match hooks.
    pub match_hooks: player::WeaponMatchHooks,
}

impl Default for WeaponRuntime {
    fn default() -> Self {
        let mut definitions = HashMap::new();
        for definition in definitions::base_weapons() {
            definitions.insert(definition.definition.name.clone(), definition.definition.clone());
        }
        Self {
            blaster_causes: HashMap::new(),
            silencer_charges: HashMap::new(),
            states: HashMap::new(),
            inputs: HashMap::new(),
            noises: HashMap::new(),
            sound_entity: None,
            sound2_entity: None,
            engine: None,
            source_rules: None,
            definitions,
            fallback_order: None,
            extensions: HashMap::new(),
            match_hooks: player::WeaponMatchHooks::default(),
        }
    }
}

impl WeaponRuntime {
    /// Drop weapon state after an actor release.
    pub fn on_actor_released(&mut self, actor: &ActorId) {
        self.states.remove(actor);
        self.inputs.remove(actor);
        self.blaster_causes.remove(actor);
        self.silencer_charges.remove(actor);
        self.noises.remove(actor);
        if self.sound_entity.as_ref().is_some_and(|record| &record.actor == actor) {
            self.sound_entity = None;
        }
        if self.sound2_entity.as_ref().is_some_and(|record| &record.actor == actor) {
            self.sound2_entity = None;
        }
    }
}
