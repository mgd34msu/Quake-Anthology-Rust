//! Q2 weapons barrel (`src/content/q2/foundation/weapons/index.ts`).
//!
//! Pure re-export barrel: the donor index only re-exports. The donor's
//! `Q2Weapons`/`Q2Ballistics` classes become arena state ([`WeaponRuntime`])
//! plus the free functions re-exported below; `Q2_BASE_WEAPONS` is
//! [`definitions::base_weapons`], `MOD` is [`types::Mod`], and
//! `Q2WeaponHooks` splits into [`types::WeaponEngine`] plus arena calls.
//! The session registry lookup stays at [`player::weapon_definition`] so
//! the barrel name keeps the static [`definitions::weapon_definition`].
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use std::collections::HashMap;

use qa_core::identity::ActorId;

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

pub use ballistics::{
    bfg_explode, bfg_think, bfg_touch, blaster_touch, capture_projectiles, check_dodge, fire_bfg, fire_blaster,
    fire_bullet, fire_grenade, fire_hand_grenade, fire_hit, fire_rail, fire_rocket, fire_shotgun, grant_silencer,
    grenade_explode, grenade_touch, grenade_update, q2_ballistics_callbacks, register_ballistics_callbacks,
    reset_silencer, restore_projectiles, rocket_touch, silencer_shots, weapon_player_noise,
    weapon_player_noise_for_actor, NoiseKind, Q2HandGrenadeLaunch, Q2ProjectileContact,
};
pub use checkpoint::{Q2NoiseCheckpoint, Q2WeaponsCheckpoint};
pub use definitions::{base_weapons, weapon_definition, weapon_from_classname};
pub use player::{
    animate_player, attack_animation, bind_player_weapon, can_drop_weapon, definition_from_classname, is_holstered,
    register_weapon_extension, registered_weapon_definitions, registered_weapon_names, request_holster, request_weapon,
    resume_primary, set_fallback_order, set_weapon_source_rules, tick_player_weapon, weapon_ammo, weapon_ammo_changed,
    weapon_can_target, weapon_consume, weapon_consume_count, weapon_consume_infinite, weapon_continues_attack,
    weapon_emit, weapon_firing_interval, weapon_flash, weapon_generic_classic, weapon_generic_rerelease, weapon_kick,
    weapon_lag_begin, weapon_lag_end, weapon_multiplier, weapon_no_ammo, weapon_powerup_sound, weapon_project,
    weapon_set_loop, weapon_source_rules, weapon_throw_classic, weapon_throw_rerelease, Q2ThrowDefinition,
    Q2WeaponContext, Q2WeaponExtension, Q2WeaponSelection, Q2WeaponSourceRules,
};
pub use types::{
    Mod, Q2BaseWeaponDefinition, Q2BaseWeaponName, Q2GrenadeAdjustment, Q2NoiseRecord, Q2WeaponDefinition,
    Q2WeaponEvent, Q2WeaponInput, Q2WeaponName, Q2WeaponOwner, Q2WeaponPhase, Q2WeaponState, WeaponEngine,
    PROJECTILE_MASK, SHOT_MASK, WATER_MASK,
};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_weapons_cover_source_arsenal() {
        let weapons = base_weapons();
        assert!(!weapons.is_empty());
        let names: Vec<&str> = weapons.iter().map(|weapon| weapon.definition.name.as_str()).collect();
        assert!(names.contains(&"blaster"));
        assert!(names.contains(&"shotgun"));
        assert!(weapon_from_classname("weapon_shotgun").is_some());
        assert!(weapon_from_classname("weapon_bfg").is_some());
        assert!(weapon_from_classname("weapon_unknown").is_none());
        let blaster = weapon_definition(Q2BaseWeaponName::Blaster);
        assert_eq!(blaster.definition.name, "blaster");
    }

    #[test]
    fn means_of_death_and_masks_match_source() {
        assert_eq!(Mod::BLASTER, 1);
        assert_eq!(Mod::HIT, 32);
        assert_eq!(SHOT_MASK, 1 | 2 | 0x2000000 | 0x4000000);
        assert_eq!(WATER_MASK, 8 | 16 | 32);
        assert_eq!(SHOT_MASK & WATER_MASK, 0);
        assert_eq!(PROJECTILE_MASK & SHOT_MASK, SHOT_MASK);
    }

    #[test]
    fn selection_and_ballistics_surface_is_reachable() {
        assert_eq!(
            [
                Q2WeaponSelection::Selected,
                Q2WeaponSelection::Current,
                Q2WeaponSelection::NotOwned,
                Q2WeaponSelection::NoAmmo,
                Q2WeaponSelection::NotEnoughAmmo,
            ]
            .len(),
            5
        );
        let callbacks = q2_ballistics_callbacks();
        assert!(callbacks.think.contains_key("grenade_think"));
        assert!(callbacks.touch.contains_key("rocket_touch"));
    }
}
