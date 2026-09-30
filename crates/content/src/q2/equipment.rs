//! Q2 equipment (`src/content/q2/equipment`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

pub mod ctf_grapple;
pub mod grapple_services;
pub mod grapple_weapon;
pub mod hand_grenades;
pub mod lmctf_grapple;

use std::collections::HashMap;

use qa_core::identity::ActorId;

use crate::q2::foundation::host::Q2GameServices;

pub use ctf_grapple::{
    ctf_actor_released, ctf_grapple_callbacks, ctf_grapple_can_damage, ctf_grapple_settings,
    default_ctf_grapple_settings, CtfGrappleSettings, Q2CtfGrappleEquipment,
};
pub use grapple_services::{
    capture_ctf_grapple, capture_lmctf_grapple, grapple_body, grapple_velocity,
    restore_ctf_grapple, restore_lmctf_grapple, CtfGrappleCheckpoint, CtfGrapplePhase,
    CtfGrappleState, GrappleAnchor, GrappleCableEvent, GrappleHand, GrappleHooks, GrappleNoise,
    GrapplePose, LmctfGrappleCheckpoint, LmctfGrappleState,
};
pub use grapple_weapon::{
    create_grapple_weapon_state, ctf_grapple_weapon_should_reset, fire_ctf_grapple_weapon,
    fire_lmctf_grapple_weapon, lmctf_grapple, q2_ctf_grapple, q2_rerelease_ctf_grapple,
    release_lmctf_grapple_weapon, step_ctf_grapple_weapon, step_lmctf_grapple_weapon,
    CtfGrappleWeaponActions, GrappleHandoff, GrappleStepInput, GrappleWeaponInput,
    GrappleWeaponPresentation, GrappleWeaponSource, GrappleWeaponState, LmctfGrappleWeaponActions,
    Q2GrappleWeapon,
};
pub use hand_grenades::{
    default_hand_grenade_loadout, HandGrenadeActorCheckpoint, HandGrenadeCheckpoint,
    HandGrenadeEquipmentInput, HandGrenadeEquipmentState, HandGrenadeLoadout,
    Q2HandGrenadeEquipment, HAND_GRENADE_AMMO,
};
pub use lmctf_grapple::{
    default_lmctf_grapple_policy, lmctf_actor_released, lmctf_can_attach, lmctf_can_damage,
    lmctf_grapple_callbacks, lmctf_player_hit, lmctf_released, LmctfGrappleEquipment,
    LmctfGrapplePolicy,
};

/// Arena runtime state for this module.
#[derive(Debug, Default)]
pub struct EquipmentRuntime {
    /// Bound CTF equipment.
    pub ctf: Option<Q2CtfGrappleEquipment>,
    /// CTF grapple states by owner.
    pub ctf_states: HashMap<ActorId, CtfGrappleState>,
    /// Bound LMCTF equipment.
    pub lmctf: Option<LmctfGrappleEquipment>,
    /// LMCTF grapple states by owner.
    pub lmctf_states: HashMap<ActorId, LmctfGrappleState>,
    /// Hand grenade states by owner.
    pub grenades: HashMap<ActorId, HandGrenadeEquipmentState>,
    /// Active hand grenade steps.
    pub grenade_steps: i32,
}

/// Resolve the bound CTF handle.
pub(crate) fn ctf_handle(game: &Q2GameServices) -> Q2CtfGrappleEquipment {
    game.equipment
        .ctf
        .expect("Q2 CTF grapple equipment is not bound")
}

/// Read or create CTF owner state.
pub(crate) fn ctf_state_mut(game: &mut Q2GameServices, actor: ActorId) -> &mut CtfGrappleState {
    game.equipment.ctf_states.entry(actor).or_default()
}

/// Resolve the bound LMCTF handle.
pub(crate) fn lmctf_handle(game: &Q2GameServices) -> LmctfGrappleEquipment {
    game.equipment
        .lmctf
        .expect("Q2 LMCTF grapple equipment is not bound")
}

/// Read or create LMCTF owner state.
pub(crate) fn lmctf_state_mut(game: &mut Q2GameServices, actor: ActorId) -> &mut LmctfGrappleState {
    game.equipment.lmctf_states.entry(actor).or_default()
}

/// Release fan-out for equipment owners and hooks (`onRelease`).
pub fn equipment_actor_released(game: &mut Q2GameServices, actor: &ActorId) {
    if game.equipment.ctf.is_some() {
        ctf_actor_released(game, actor);
    } else {
        game.equipment.ctf_states.remove(actor);
    }
    if game.equipment.lmctf.is_some() {
        lmctf_actor_released(game, actor);
    } else {
        game.equipment.lmctf_states.remove(actor);
    }
    game.equipment.grenades.remove(actor);
}
