//! Q2 grapple weapons (`src/content/q2/equipment/grapple-weapon.ts`).
//!
//! CTF g_ctf.c/g_ctf.cpp and LMCTF p_weapon.c weapon-slot continuations
//! (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::{scale3, Vec3};

use crate::q2::foundation::host::{Q2Edition, Q2GameServices};
use crate::q2::foundation::weapons::generic_frame::{
    step_q2_classic_frame, step_q2_rerelease_frame, ClassicFrameHooks, Q2ClassicFrameInput, Q2GenericDefinition,
    Q2GenericFrameState, Q2RereleaseFrameInput, RereleaseFrameHooks,
};
use crate::q2::foundation::weapons::types::{Q2WeaponDefinition, Q2WeaponPhase};
use crate::q2::foundation::weapons::vectors::angle_vectors;

use super::ctf_grapple::Q2CtfGrappleEquipment;
use super::grapple_services::{CtfGrapplePhase, CtfGrappleState, LmctfGrappleState};
use super::lmctf_grapple::LmctfGrappleEquipment;

/// CTF grapple weapon definition (`Q2_CTF_GRAPPLE`).
pub fn q2_ctf_grapple() -> Q2WeaponDefinition {
    Q2WeaponDefinition {
        name: "grapple".to_string(),
        classname: "weapon_grapple".to_string(),
        item: "q2:weapon_grapple".to_string(),
        ammo: None,
        quantity: 0,
        warning: 0,
        view_model: "models/weapons/grapple/tris.md2".to_string(),
        world_model: String::new(),
        player_model: 12,
        activate_last: 5,
        fire_last: 9,
        idle_last: 31,
        deactivate_last: 36,
        pauses: vec![10, 18, 27],
        fires: vec![6],
        repeating: false,
    }
}

/// Rerelease CTF grapple weapon definition (`Q2_RERELEASE_CTF_GRAPPLE`).
pub fn q2_rerelease_ctf_grapple() -> Q2WeaponDefinition {
    let mut definition = q2_ctf_grapple();
    definition.fire_last = 10;
    definition
}

/// LMCTF grapple weapon definition (`LMCTF_GRAPPLE`).
pub fn lmctf_grapple() -> Q2WeaponDefinition {
    Q2WeaponDefinition {
        name: "lmctf:hook".to_string(),
        item: "q2:weapon_hook".to_string(),
        classname: "weapon_hook".to_string(),
        ammo: None,
        quantity: 0,
        warning: 0,
        view_model: "models/weapons/v_hook/tris.md2".to_string(),
        world_model: "models/objects/debris2/tris.md2".to_string(),
        player_model: 11,
        activate_last: 9,
        fire_last: 13,
        idle_last: 34,
        deactivate_last: 38,
        pauses: vec![14, 18, 26, 30],
        fires: vec![8, 9, 10, 11],
        repeating: true,
    }
}

/// Grapple weapon step input (`GrappleWeaponInput`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrappleWeaponInput {
    /// Attack held.
    pub attack: bool,
    /// Change requested.
    pub change_requested: bool,
    /// Holster held.
    pub holster: bool,
    /// Latched holster.
    pub latched_holster: bool,
}

/// CTF grapple weapon actions (`stepCtfGrappleWeapon` hooks).
pub trait CtfGrappleWeaponActions {
    /// Reset the grapple.
    fn reset(&mut self);
    /// Prepare a drop.
    fn prepare_drop(&mut self);
    /// Step the generic frame.
    fn generic(&mut self, animation: &mut Q2GenericFrameState);
}

/// Step the CTF grapple weapon (`stepCtfGrappleWeapon`).
pub fn step_ctf_grapple_weapon(
    state: &mut Q2GenericFrameState,
    source: &CtfGrappleState,
    input: &GrappleWeaponInput,
    edition: Q2Edition,
    actions: &mut dyn CtfGrappleWeaponActions,
) {
    let rerelease = edition == Q2Edition::Rerelease;
    let held = input.attack || rerelease && input.holster;
    if held && state.phase == Q2WeaponPhase::Firing && source.grapple.is_some() {
        state.frame = if rerelease { 6 } else { 9 };
    }
    if !held && source.grapple.is_some() {
        actions.reset();
        if state.phase == Q2WeaponPhase::Firing {
            state.phase = Q2WeaponPhase::Ready;
        }
    }
    if (input.change_requested || rerelease && (input.holster || input.latched_holster))
        && source.grapple_state != CtfGrapplePhase::Fly
        && state.phase == Q2WeaponPhase::Firing
    {
        actions.prepare_drop();
        state.phase = Q2WeaponPhase::Dropping;
        state.frame = 32;
    }
    let before = state.phase;
    actions.generic(state);
    if rerelease && held && state.phase == Q2WeaponPhase::Firing && source.grapple.is_some() {
        state.frame = 6;
    }
    if before == Q2WeaponPhase::Activating
        && state.phase == Q2WeaponPhase::Ready
        && source.grapple_state != CtfGrapplePhase::Fly
    {
        state.frame = if held {
            5
        } else if rerelease {
            6
        } else {
            9
        };
        state.phase = Q2WeaponPhase::Firing;
    }
}

/// LMCTF grapple weapon actions (`stepLmctfGrappleWeapon` hooks).
pub trait LmctfGrappleWeaponActions {
    /// Abort the grapple.
    fn abort(&mut self);
    /// Step the generic frame.
    fn generic(&mut self, animation: &mut Q2GenericFrameState);
}

/// Step the LMCTF grapple weapon (`stepLmctfGrappleWeapon`).
pub fn step_lmctf_grapple_weapon(
    state: &mut Q2GenericFrameState,
    source: &LmctfGrappleState,
    input: &GrappleWeaponInput,
    actions: &mut dyn LmctfGrappleWeaponActions,
) {
    if state.phase == Q2WeaponPhase::Activating {
        state.frame += 1;
    }
    if input.change_requested && state.phase != Q2WeaponPhase::Dropping {
        state.phase = Q2WeaponPhase::Dropping;
        state.frame = 36;
        return;
    }
    if !input.attack && !state.latched_attack && !source.hook_held {
        actions.abort();
    }
    actions.generic(state);
}

/// Grapple weapon presentation (`GrappleWeaponPresentation`).
#[derive(Debug, Clone, Copy)]
pub struct GrappleWeaponPresentation {
    /// Apply view kick.
    pub kick: fn(Vec3, f64),
    /// Play the attack animation.
    pub attack_animation: fn(),
    /// Play the reverse animation.
    pub reverse_animation: fn(),
    /// Play the powerup sound.
    pub powerup_sound: fn(),
    /// Read the animation frame time.
    pub animation_time: fn(&Q2GenericFrameState) -> f64,
}

/// Fire the CTF grapple weapon (`fireCtfGrappleWeapon`).
pub fn fire_ctf_grapple_weapon(
    actor: ActorId,
    game: &mut Q2GameServices,
    core: Q2CtfGrappleEquipment,
    state: &mut Q2GenericFrameState,
    edition: Q2Edition,
) -> Option<(Vec3, f64)> {
    if core.state_snapshot(game, actor.clone()).grapple_state != CtfGrapplePhase::Fly {
        if edition == Q2Edition::Classic {
            state.frame += 1;
        }
        return None;
    }
    let kick = if edition == Q2Edition::Classic {
        let pose = (core.hooks.pose)(actor.clone(), game);
        Some((scale3(angle_vectors(pose.angles).forward, -2.0), -1.0))
    } else {
        None
    };
    core.fire_from_pose(actor.clone(), game);
    if edition == Q2Edition::Classic && game.host.actors().is_live(&actor) {
        state.frame += 1;
    }
    kick
}

/// Fire the LMCTF grapple weapon (`fireLmctfGrappleWeapon`).
pub fn fire_lmctf_grapple_weapon(
    actor: ActorId,
    game: &mut Q2GameServices,
    core: LmctfGrappleEquipment,
    state: &mut Q2GenericFrameState,
) -> Option<(Vec3, f64)> {
    state.source_firing = core.state_snapshot(game, actor.clone()).hook_state == 0;
    let kick = if state.source_firing {
        let pose = (core.hooks.pose)(actor.clone(), game);
        Some((scale3(angle_vectors(pose.angles).forward, -2.0), -1.0))
    } else {
        None
    };
    core.fire(actor, game);
    kick
}

/// Release the LMCTF grapple weapon (`releaseLmctfGrappleWeapon`).
pub fn release_lmctf_grapple_weapon(state: &mut Q2GenericFrameState) {
    if state.phase == Q2WeaponPhase::Firing {
        state.phase = Q2WeaponPhase::Ready;
    }
}

/// Whether the CTF grapple weapon should reset (`ctfGrappleWeaponShouldReset`).
pub fn ctf_grapple_weapon_should_reset(state: &Q2GenericFrameState, selected: bool, change_requested: bool) -> bool {
    selected && !change_requested && state.phase != Q2WeaponPhase::Firing && state.phase != Q2WeaponPhase::Activating
}

/// Grapple weapon source (`GrappleWeaponSource`).
#[derive(Debug, Clone, Copy)]
pub enum GrappleWeaponSource {
    /// CTF source.
    Ctf {
        /// Equipment core.
        core: Q2CtfGrappleEquipment,
        /// Edition.
        edition: Q2Edition,
    },
    /// LMCTF source.
    Lmctf {
        /// Equipment core.
        core: LmctfGrappleEquipment,
    },
}

/// Grapple weapon handoff (`GrappleWeaponState["handoff"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrappleHandoff {
    /// Active.
    Active,
    /// Holstering.
    Holstering,
    /// Holstered.
    Holstered,
}

/// Grapple weapon state (`GrappleWeaponState`).
#[derive(Debug, Clone, PartialEq)]
pub struct GrappleWeaponState {
    /// Frame animation.
    pub animation: Q2GenericFrameState,
    /// Handoff.
    pub handoff: GrappleHandoff,
}

/// Create grapple weapon state (`createGrappleWeaponState`).
pub fn create_grapple_weapon_state() -> GrappleWeaponState {
    GrappleWeaponState {
        animation: Q2GenericFrameState {
            phase: Q2WeaponPhase::Activating,
            frame: 0,
            latched_attack: false,
            source_firing: false,
            think_time: 0.0,
            fire_finished: 0.0,
            fire_buffered: false,
            last_firing_time: 0.0,
        },
        handoff: GrappleHandoff::Holstered,
    }
}

/// Grapple weapon step input (`step` input).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrappleStepInput {
    /// Attack held.
    pub attack: bool,
    /// Now.
    pub now: f64,
    /// Frame seconds.
    pub frame_seconds: f64,
    /// Instant switch.
    pub instant_switch: bool,
    /// Holster held.
    pub holster: bool,
    /// Weapon thunk.
    pub weapon_thunk: bool,
    /// Latched holster.
    pub latched_holster: bool,
}

/// Grapple weapon slot adapter (`Q2GrappleWeapon`).
///
/// The session owns the weapon state; the adapter borrows it per call.
#[derive(Debug, Clone)]
pub struct Q2GrappleWeapon {
    /// Owning actor.
    pub actor: ActorId,
    /// Weapon source.
    pub source: GrappleWeaponSource,
    /// Presentation.
    pub presentation: GrappleWeaponPresentation,
}

impl Q2GrappleWeapon {
    /// Read the weapon definition (`definition`).
    pub fn definition(&self) -> Q2WeaponDefinition {
        match self.source {
            GrappleWeaponSource::Lmctf { .. } => lmctf_grapple(),
            GrappleWeaponSource::Ctf { edition, .. } => {
                if edition == Q2Edition::Rerelease {
                    q2_rerelease_ctf_grapple()
                } else {
                    q2_ctf_grapple()
                }
            }
        }
    }

    /// Holster the weapon (`holster`).
    pub fn holster(&self, state: &mut GrappleWeaponState) {
        if state.handoff == GrappleHandoff::Active {
            state.handoff = GrappleHandoff::Holstering;
        }
    }

    /// Whether the weapon is holstered (`isHolstered`).
    pub fn is_holstered(&self, state: &GrappleWeaponState) -> bool {
        state.handoff == GrappleHandoff::Holstered
    }

    /// Handle a release (`released`).
    pub fn released(&self, state: &mut GrappleWeaponState) {
        if matches!(self.source, GrappleWeaponSource::Lmctf { .. }) {
            release_lmctf_grapple_weapon(&mut state.animation);
        }
    }

    /// Resume the weapon (`resume`).
    pub fn resume(&self, state: &mut GrappleWeaponState) {
        state.handoff = GrappleHandoff::Active;
        state.animation.phase = Q2WeaponPhase::Activating;
        state.animation.frame = 0;
    }

    /// Run source player-end work (`playerFrame`).
    pub fn player_frame(&self, state: &mut GrappleWeaponState, game: &mut Q2GameServices) {
        match self.source {
            GrappleWeaponSource::Ctf { core, .. } => {
                if ctf_grapple_weapon_should_reset(
                    &state.animation,
                    state.handoff == GrappleHandoff::Active,
                    state.handoff == GrappleHandoff::Holstering,
                ) {
                    core.reset(self.actor.clone(), game);
                } else {
                    core.player_frame(self.actor.clone(), game, true);
                }
            }
            GrappleWeaponSource::Lmctf { core } => {
                if core.state_snapshot(game, self.actor.clone()).hook_state != 0 {
                    core.fire(self.actor.clone(), game);
                }
            }
        }
    }

    /// Step the weapon (`step`).
    pub fn step(&self, state: &mut GrappleWeaponState, game: &mut Q2GameServices, input: &GrappleStepInput) {
        if self.is_holstered(state) {
            return;
        }
        let frame_input = Q2RereleaseFrameInput {
            attack: input.attack,
            change_requested: state.handoff == GrappleHandoff::Holstering,
            now: input.now,
            frame_seconds: input.frame_seconds,
            instant_switch: input.instant_switch,
            holster: input.holster,
            weapon_thunk: input.weapon_thunk,
        };
        let step_input = GrappleWeaponInput {
            attack: frame_input.attack,
            change_requested: frame_input.change_requested,
            holster: frame_input.holster,
            latched_holster: input.latched_holster,
        };
        let definition = Q2GenericDefinition::from(&self.definition());
        let mut driver = GrappleStepDriver {
            game,
            actor: self.actor.clone(),
            source: self.source,
            presentation: self.presentation,
            definition,
            frame_input,
            handoff: &mut state.handoff,
        };
        let animation = &mut state.animation;
        match self.source {
            GrappleWeaponSource::Ctf { core, edition } => {
                let source = core.state_snapshot(&mut *driver.game, driver.actor.clone());
                step_ctf_grapple_weapon(animation, &source, &step_input, edition, &mut driver);
            }
            GrappleWeaponSource::Lmctf { core } => {
                let source = core.state_snapshot(&mut *driver.game, driver.actor.clone());
                step_lmctf_grapple_weapon(animation, &source, &step_input, &mut driver);
            }
        }
    }
}

/// Step driver implementing the grapple weapon actions.
struct GrappleStepDriver<'a> {
    /// Game services.
    game: &'a mut Q2GameServices,
    /// Owning actor.
    actor: ActorId,
    /// Weapon source.
    source: GrappleWeaponSource,
    /// Presentation.
    presentation: GrappleWeaponPresentation,
    /// Generic definition.
    definition: Q2GenericDefinition,
    /// Frame input.
    frame_input: Q2RereleaseFrameInput,
    /// Handoff.
    handoff: &'a mut GrappleHandoff,
}

impl GrappleStepDriver<'_> {
    /// Step the generic frame (`generic`).
    fn generic(&mut self, animation: &mut Q2GenericFrameState) {
        let mut hooks = GrappleFrameHooks {
            game: &mut *self.game,
            animation,
            handoff: &mut *self.handoff,
            actor: self.actor.clone(),
            source: self.source,
            presentation: self.presentation,
        };
        match self.source {
            GrappleWeaponSource::Ctf {
                edition: Q2Edition::Rerelease,
                ..
            } => {
                step_q2_rerelease_frame(&self.definition, &self.frame_input, &mut hooks);
            }
            GrappleWeaponSource::Ctf { .. } => {
                let before = hooks.animation.phase;
                let classic = Q2ClassicFrameInput {
                    attack: self.frame_input.attack,
                    change_requested: self.frame_input.change_requested,
                };
                step_q2_classic_frame(&self.definition, &classic, &mut hooks);
                if hooks.animation.phase != Q2WeaponPhase::Firing
                    && before == hooks.animation.phase
                    && *hooks.handoff != GrappleHandoff::Holstered
                {
                    step_q2_classic_frame(&self.definition, &classic, &mut hooks);
                }
            }
            GrappleWeaponSource::Lmctf { .. } => {
                hooks.animation.source_firing = false;
                let classic = Q2ClassicFrameInput {
                    attack: self.frame_input.attack,
                    change_requested: self.frame_input.change_requested,
                };
                step_q2_classic_frame(&self.definition, &classic, &mut hooks);
            }
        }
    }
}

impl CtfGrappleWeaponActions for GrappleStepDriver<'_> {
    fn reset(&mut self) {
        let GrappleWeaponSource::Ctf { core, .. } = self.source else {
            panic!("Q2 CTF grapple weapon actions require a CTF source");
        };
        core.reset(self.actor.clone(), self.game);
    }

    fn prepare_drop(&mut self) {
        if *self.handoff == GrappleHandoff::Active {
            *self.handoff = GrappleHandoff::Holstering;
        }
    }

    fn generic(&mut self, animation: &mut Q2GenericFrameState) {
        GrappleStepDriver::generic(self, animation);
    }
}

impl LmctfGrappleWeaponActions for GrappleStepDriver<'_> {
    fn abort(&mut self) {
        let GrappleWeaponSource::Lmctf { core } = self.source else {
            panic!("Q2 LMCTF grapple weapon actions require an LMCTF source");
        };
        core.abort(self.actor.clone(), self.game);
    }

    fn generic(&mut self, animation: &mut Q2GenericFrameState) {
        GrappleStepDriver::generic(self, animation);
    }
}

/// Frame hooks over the grapple weapon state.
struct GrappleFrameHooks<'a> {
    /// Game services.
    game: &'a mut Q2GameServices,
    /// Frame animation.
    animation: &'a mut Q2GenericFrameState,
    /// Handoff.
    handoff: &'a mut GrappleHandoff,
    /// Owning actor.
    actor: ActorId,
    /// Weapon source.
    source: GrappleWeaponSource,
    /// Presentation.
    presentation: GrappleWeaponPresentation,
}

impl ClassicFrameHooks for GrappleFrameHooks<'_> {
    fn frame_state(&mut self) -> &mut dyn crate::q2::foundation::weapons::generic_frame::GenericFrameState {
        self.animation
    }

    fn random(&mut self) -> f64 {
        self.game.random()
    }

    fn ammo(&mut self) -> f64 {
        0.0
    }

    fn no_ammo(&mut self) {
        panic!("An ammunition-free grapple cannot run out of ammo");
    }

    fn fire(&mut self, _buffered: bool) {
        match self.source {
            GrappleWeaponSource::Ctf { core, edition } => {
                if let Some((origin, pitch)) =
                    fire_ctf_grapple_weapon(self.actor.clone(), self.game, core, self.animation, edition)
                {
                    (self.presentation.kick)(origin, pitch);
                }
            }
            GrappleWeaponSource::Lmctf { core } => {
                if let Some((origin, pitch)) =
                    fire_lmctf_grapple_weapon(self.actor.clone(), self.game, core, self.animation)
                {
                    (self.presentation.kick)(origin, pitch);
                }
            }
        }
    }

    fn change_weapon(&mut self) {
        *self.handoff = GrappleHandoff::Holstered;
    }

    fn reverse_animation(&mut self) {
        (self.presentation.reverse_animation)();
    }

    fn attack_animation(&mut self) {
        (self.presentation.attack_animation)();
    }

    fn powerup_sound(&mut self) {
        (self.presentation.powerup_sound)();
    }
}

impl RereleaseFrameHooks for GrappleFrameHooks<'_> {
    fn animation_time(&mut self) -> f64 {
        (self.presentation.animation_time)(self.animation)
    }

    fn prepare_drop(&mut self) {
        if *self.handoff == GrappleHandoff::Active {
            *self.handoff = GrappleHandoff::Holstering;
        }
    }
}
