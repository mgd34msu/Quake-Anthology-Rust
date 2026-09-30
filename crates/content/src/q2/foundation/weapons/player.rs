//! Player weapons (`src/content/q2/foundation/weapons/player.ts`).
//!
//! Adapted from id Software's Quake II `g_weapon.c`, `p_weapon.c` and
//! rerelease `g_weapon.cpp`, `p_weapon.cpp` (GPL-2.0-or-later). All
//! damage is admitted by the session combat authority.

use std::collections::HashMap;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Vec3, add3, length3, normalize3, scale3, sub3, vec3};

use super::ballistics::{
    NoiseKind, check_dodge, fire_bfg, fire_blaster, fire_bullet, fire_grenade, fire_hit,
    fire_rail, fire_rocket, fire_shotgun, player_noise, register_ballistics_callbacks,
};
use super::checkpoint::{
    Q2BlasterCauseEntry, Q2NoiseCheckpoint, Q2SilencerEntry, Q2WeaponInputEntry, Q2WeaponNoiseEntry,
    Q2WeaponStateEntry, Q2WeaponsCheckpoint,
};
use super::damage::q2_weapon_damage_multiplier;
use super::generic_frame::{
    ClassicFrameHooks, GenericFrameState, Q2ClassicFrameInput, Q2GenericDefinition,
    Q2RereleaseFrameInput, RereleaseFrameHooks, millisecond_sum, step_q2_classic_frame,
    step_q2_rerelease_frame,
};
use super::hand_grenade::{
    HandGrenadeTempo, HandProjectileSpec, HandThrowInput, calculate_hand_throw, hand_deadline,
    hand_fuse_deadline, hand_recovery_seconds,
};
use super::presentation::{
    Q2AnimationRateInput, Q2PowerupSoundInput, q2_attack_frames, q2_powerup_sound,
    q2_reverse_frames, q2_weapon_animation_rate, set_q2_weapon_recoil,
};
use super::projection::{Q2ActorView, project_q2_actor};
use super::types::{
    LagToken, Mod, PLAYER_CONTENTS, Q2GrenadeAdjustment, Q2HandReservation, Q2NoiseRecord,
    Q2WeaponDefinition, Q2WeaponEvent, Q2WeaponInput, Q2WeaponName, Q2WeaponOwner, Q2WeaponPhase,
    Q2WeaponState, PlayerAnimationPriority, PrimaryHandoff, WeaponBeamEffect, WeaponEngine,
    WeaponHand,
};
use super::vectors::angle_vectors;
use crate::contract::ItemId;
use crate::q2::foundation::checkpoint::{restore_q2_actor, save_q2_actor};
use crate::q2::foundation::host::{
    Q2Edition, Q2GameServices, Q2Mode, Q2PresentationEvent, Q2SoundEvent, Q2SoundLoop,
    Q2TraceRequest,
};
use crate::q2::support::contracts::TraceHit;

/// Weapon context (`Q2WeaponContext`).
///
/// An owned snapshot: the donor hands extensions live references, but
/// the arena lends the weapon state separately, so the context carries
/// clones and the state travels as an explicit parameter.
#[derive(Debug, Clone)]
pub struct Q2WeaponContext {
    /// Owner.
    pub owner: Q2WeaponOwner,
    /// Input snapshot.
    pub input: Q2WeaponInput,
    /// Definition snapshot.
    pub definition: Q2WeaponDefinition,
    /// Now.
    pub now: f64,
    /// Whether rerelease.
    pub rerelease: bool,
    /// Whether silenced.
    pub silenced: bool,
}

/// Weapon selection rule (`Q2WeaponSelectionExtension`).
#[derive(Debug, Clone, Copy)]
pub struct Q2WeaponSelection {
    /// Requested weapon.
    pub requested: bool,
    /// Choose the extension weapon.
    pub choose: fn(&Q2WeaponOwner, &mut Q2GameServices, &Q2WeaponState) -> bool,
}

/// Weapon extension (`Q2WeaponExtension`).
pub trait Q2WeaponExtension {
    /// Extension definition.
    fn definition(&self) -> &Q2WeaponDefinition;
    /// Fire.
    fn fire(&mut self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState);
    /// Think, reporting whether the extension handled the frame.
    ///
    /// The donor checks for a think hook and returns; `false` is the
    /// missing hook, `true` a handled frame.
    fn think(
        &mut self,
        _context: &Q2WeaponContext,
        _game: &mut Q2GameServices,
        _state: &mut Q2WeaponState,
    ) -> bool {
        false
    }
    /// Throw a held grenade, reporting whether the extension handled it.
    fn held(
        &mut self,
        _context: &Q2WeaponContext,
        _game: &mut Q2GameServices,
        _state: &mut Q2WeaponState,
        _held: bool,
    ) -> bool {
        false
    }
    /// Selection rule.
    fn selection(&self) -> Option<Q2WeaponSelection> {
        None
    }
}

/// CTF weapon hooks (`Q2WeaponSourceRules["ctf"]`).
#[derive(Debug, Clone, Copy)]
pub struct CtfWeaponHooks {
    /// Haste check.
    pub haste: fn(&Q2WeaponContext, &mut Q2GameServices) -> bool,
    /// Strength sound check.
    pub strength_sound: fn(&Q2WeaponContext, &mut Q2GameServices) -> bool,
    /// Haste sound.
    pub haste_sound: fn(&Q2WeaponContext, &mut Q2GameServices),
}

/// LMCTF weapon hooks (`Q2WeaponSourceRules["lmctf"]`).
#[derive(Debug, Clone, Copy)]
pub struct LmctfWeaponHooks {
    /// Post-native think, reporting whether to run the frame again.
    ///
    /// The donor passes a repeat callback the hook may invoke; the
    /// bundled hook invokes it at most once, so a boolean carries the
    /// same decision.
    pub post_native_think: fn(&Q2WeaponContext, &mut Q2GameServices) -> bool,
}

/// Match hooks.
#[derive(Debug, Clone, Copy, Default)]
pub struct WeaponMatchHooks {
    /// CTF hooks.
    pub ctf: Option<CtfWeaponHooks>,
    /// LMCTF hooks.
    pub lmctf: Option<LmctfWeaponHooks>,
}

/// Weapon source rules (`Q2WeaponSourceRules`).
#[derive(Debug, Clone, Copy)]
pub struct Q2WeaponSourceRules {
    /// Rules kind.
    pub kind: super::WeaponSourceRules,
    /// CTF hooks.
    pub ctf: Option<CtfWeaponHooks>,
    /// LMCTF hooks.
    pub lmctf: Option<LmctfWeaponHooks>,
}

/// Thrown weapon definition (`Q2ThrowDefinition`).
#[derive(Debug, Clone)]
pub struct Q2ThrowDefinition {
    /// Sound frame.
    pub sound_frame: i32,
    /// Hold frame.
    pub hold_frame: i32,
    /// Fire frame.
    pub fire_frame: i32,
    /// Cock sound.
    pub cock_sound: String,
    /// Hold sound.
    pub hold_sound: String,
    /// Whether the throw explodes.
    pub explode: bool,
    /// Whether to wrap before the pause.
    pub wrap_before_pause: bool,
    /// Whether to release held grenades.
    pub release_held: bool,
    /// Fire the throw.
    pub fire: fn(&Q2WeaponContext, &mut Q2GameServices, &mut Q2WeaponState, bool),
}

/// Emit a weapon event through the session engine (`hooks.emit`).
fn weapon_emit(game: &mut Q2GameServices, event: &Q2WeaponEvent) {
    let mut engine = game.weapons.engine.take();
    if let Some(engine) = engine.as_mut() {
        engine.emit(event);
    }
    game.weapons.engine = engine;
}

/// Report ammo changes (`hooks.ammoChanged`).
fn ammo_changed(game: &mut Q2GameServices, actor: &ActorId, ammo: &ItemId) {
    let mut engine = game.weapons.engine.take();
    if let Some(engine) = engine.as_mut() {
        engine.ammo_changed(actor, ammo);
    }
    game.weapons.engine = engine;
}

/// Adjust a firing interval (`hooks.firingInterval`).
fn firing_interval(game: &mut Q2GameServices, actor: &ActorId, seconds: f64) -> f64 {
    let mut engine = game.weapons.engine.take();
    let interval = engine
        .as_mut()
        .map(|engine| engine.firing_interval(actor, seconds))
        .unwrap_or(seconds);
    game.weapons.engine = engine;
    interval
}

/// Run the weapon state dance.
///
/// The state travels outside the arena while hooks run so frame
/// callbacks observe the same live state the donor steps.
fn with_weapon_state<R>(
    game: &mut Q2GameServices,
    owner: &ActorId,
    f: impl FnOnce(&mut Q2GameServices, &mut Q2WeaponState) -> R,
) -> R {
    let mut state = game.weapons.states.remove(owner).unwrap_or_else(|| Q2WeaponState::new(None));
    let out = f(game, &mut state);
    game.weapons.states.insert(owner.clone(), state);
    out
}

/// Require a weapon definition (`require`).
fn require_definition(game: &Q2GameServices, name: Option<&Q2WeaponName>) -> Q2WeaponDefinition {
    let name = name.unwrap_or_else(|| panic!("Player has no readied Q2 weapon"));
    game.weapons.definitions.get(name).cloned().unwrap_or_else(|| {
        panic!("Unknown Q2 weapon {name}");
    })
}

/// Read ammo (`ammo`).
fn read_ammo(game: &mut Q2GameServices, owner: &ActorId, definition: &Q2WeaponDefinition) -> f64 {
    match definition.ammo.as_ref() {
        None => 1.0,
        Some(ammo) => game.host.inventory().count(owner, ammo),
    }
}

/// Register a weapon extension (`registerExtension`).
pub fn register_weapon_extension(
    game: &mut Q2GameServices,
    extension: Box<dyn Q2WeaponExtension>,
) {
    register_ballistics_callbacks(game);
    let name = extension.definition().name.clone();
    game.weapons.definitions.insert(name.clone(), extension.definition().clone());
    game.weapons.extensions.insert(name, extension);
}

/// Set the fallback weapon order (`setFallbackOrder`).
pub fn set_fallback_order(game: &mut Q2GameServices, order: Vec<Q2WeaponName>) {
    if order.is_empty() {
        panic!("Q2 weapon fallback order must select at least one weapon");
    }
    game.weapons.fallback_order = Some(order);
}

/// Set source rules (`setSourceRules`).
pub fn set_weapon_source_rules(game: &mut Q2GameServices, rules: &Q2WeaponSourceRules) {
    game.weapons.source_rules = Some(rules.kind);
    game.weapons.match_hooks.ctf = rules.ctf;
    game.weapons.match_hooks.lmctf = rules.lmctf;
}

/// Registered weapon names (`registeredNames`).
pub fn registered_weapon_names(game: &Q2GameServices) -> Vec<Q2WeaponName> {
    let mut names: Vec<Q2WeaponName> = game.weapons.definitions.keys().cloned().collect();
    names.sort();
    names
}

/// Source rules kind (`sourceRules`).
pub fn weapon_source_rules(game: &Q2GameServices) -> super::WeaponSourceRules {
    game.weapons.source_rules.unwrap_or_default()
}

/// Report weapon noise (`playerNoise`).
fn weapon_noise(context: &Q2WeaponContext, game: &mut Q2GameServices, kind: NoiseKind) {
    let origin = game.body_of(context.owner.actor.id().clone()).origin;
    player_noise(game, context.owner.actor.id(), origin, kind);
}

/// Report weapon impact noise (`impactNoise`).
fn impact_noise(context: &Q2WeaponContext, game: &mut Q2GameServices, origin: Vec3) {
    player_noise(game, context.owner.actor.id(), origin, NoiseKind::Impact);
}

/// Consume ammo (`consume`).
fn use_ammo(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    use_ammo_count(context, game, state, f64::from(context.definition.quantity));
}

/// Consume counted ammo (`consume(context, count)`).
fn use_ammo_count(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    count: f64,
) {
    if context.input.infinite_ammo {
        return;
    }
    let Some(ammo) = context.definition.ammo.clone() else { return };
    if state.hand_reservation != Q2HandReservation::None {
        return;
    }
    let owner = context.owner.actor.id().clone();
    let before = game.host.inventory().count(&owner, &ammo);
    let owned = game.owned_of(owner.clone());
    if !game.host.inventory().consume(&owned, &ammo, count) {
        return;
    }
    if context.rerelease
        && before > f64::from(context.definition.warning)
        && game.host.inventory().count(&owner, &ammo) <= f64::from(context.definition.warning)
    {
        game.sound(&owner, "weapons/lowammo.wav", 0, 1.0, 1.0);
    }
    ammo_changed(game, &owner, &ammo);
}

/// Player animation (`animation`).
pub fn animate_player(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    priority: PlayerAnimationPriority,
    first: i32,
    last: i32,
) {
    if !context.input.animate_player {
        return;
    }
    weapon_emit(game, &Q2WeaponEvent::PlayerAnimation {
        actor: context.owner.actor.id().clone(),
        priority,
        first,
        last,
        reset_time: context.rerelease,
    });
}

/// Attack animation (`attackAnimation`).
pub fn attack_animation(context: &Q2WeaponContext, game: &mut Q2GameServices, offset: i32) {
    let (first, last) = q2_attack_frames(context.input.ducked, offset);
    animate_player(context, game, PlayerAnimationPriority::Attack, first, last);
}

/// Reverse animation (`reverseAnimation`).
fn reverse_animation(context: &Q2WeaponContext, game: &mut Q2GameServices) {
    let (first, last) = q2_reverse_frames(context.input.ducked);
    animate_player(context, game, PlayerAnimationPriority::Reverse, first, last);
}

/// Powerup sound (`powerupSound`).
fn powerup_sound(context: &Q2WeaponContext, game: &mut Q2GameServices) {
    if context.rerelease && context.silenced {
        return;
    }
    let path = q2_powerup_sound(&Q2PowerupSoundInput {
        quad_until: context.input.quad_until,
        double_until: context.input.double_until,
        rerelease: context.rerelease,
        now: context.now,
    });
    if let Some(path) = path {
        let owner = context.owner.actor.id().clone();
        game.sound(&owner, path, 1, 1.0, 1.0);
    }
    if let Some(ctf) = game.weapons.match_hooks.ctf {
        if (ctf.strength_sound)(context, game) {
            let owner = context.owner.actor.id().clone();
            game.sound(&owner, "ctf/tech2x.wav", 1, 1.0, 1.0);
        }
        if (ctf.haste)(context, game) {
            (ctf.haste_sound)(context, game);
        }
    }
}

/// Animation frame time (`animationTime`).
fn animation_time(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
) -> f64 {
    let rate = q2_weapon_animation_rate(&Q2AnimationRateInput {
        quick_switch: context.input.quick_switch,
        frame_seconds: game.host.frame_seconds(),
        phase: state.phase,
        frame: state.frame,
        quad_fire_until: context.input.quad_fire_until,
        haste: context.input.haste,
        now: context.now,
    });
    let native = 1.0 / f64::from(rate);
    let interval = if context.rerelease {
        firing_interval(game, context.owner.actor.id(), native)
    } else {
        native
    };
    state.gun_rate = if interval == native { f64::from(rate) } else { 1.0 / interval };
    interval
}

/// Change the weapon (`changeWeapon`).
fn change_weapon(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    state.last_weapon.clone_from(&state.weapon);
    state.weapon.clone_from(&state.pending);
    state.pending = None;
    state.phase = Q2WeaponPhase::Activating;
    state.frame = 0;
    state.think_time = context.now;
    state.fire_finished = 0.0;
    state.fire_buffered = false;
    present(context, game, state);
}

/// Prepare a drop (`prepareDrop`).
fn prepare_drop(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    if context.input.holster {
        state.primary_handoff = PrimaryHandoff::Holstering;
    }
}

/// Project a muzzle (`project`).
fn project_weapon(
    owner: &Q2WeaponOwner,
    game: &mut Q2GameServices,
    input: &Q2WeaponInput,
    angles: Vec3,
    offset: Vec3,
) -> (Vec3, Vec3) {
    project_q2_actor(
        owner.actor.id(),
        game,
        &Q2ActorView {
            hand: input.hand,
            view_height: owner.view_height,
            players_collide: input.players_collide,
        },
        angles,
        offset,
    )
}

/// Present the view weapon (`present`).
fn present(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    let definition = state.weapon.as_ref().and_then(|weapon| game.weapons.definitions.get(weapon));
    let (kick_origin, kick_angles) =
        super::presentation::q2_weapon_recoil(state, game.options.edition, game.host.now());
    weapon_emit(game, &Q2WeaponEvent::ViewWeapon {
        actor: context.owner.actor.id().clone(),
        weapon: state.weapon.clone(),
        model: if state.primary_handoff == PrimaryHandoff::Holstered {
            String::new()
        } else {
            state.view_model.clone().or_else(|| definition.map(|definition| definition.view_model.clone())).unwrap_or_default()
        },
        player_model: definition.map(|definition| definition.player_model).unwrap_or(0),
        frame: state.frame,
        skin: state.view_skin,
        rate: state.gun_rate,
        kick_origin,
        kick_angles,
    });
}

/// Fire the readied weapon (`fire`).
fn fire_weapon(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    if game.weapons.extensions.contains_key(&context.definition.name) {
        let name = context.definition.name.clone();
        let mut extension =
            game.weapons.extensions.remove(&name).expect("weapon extension");
        extension.fire(context, game, state);
        game.weapons.extensions.insert(name, extension);
        return;
    }
    match context.definition.name.as_str() {
        "blaster" => {
            let damage =
                if context.rerelease || game.options.mode == Q2Mode::Deathmatch { 15.0 } else { 10.0 };
            fire_blaster_weapon(context, game, state, vec3(0.0, 0.0, 0.0), damage, false, 8);
            if !context.rerelease {
                state.frame += 1;
            }
        }
        "hyperblaster" => fire_hyperblaster(context, game, state),
        "machinegun" => fire_machinegun(context, game, state),
        "chaingun" => fire_chaingun(context, game, state),
        "shotgun" => fire_shotgun_weapon(context, game, state, false),
        "supershotgun" => fire_shotgun_weapon(context, game, state, true),
        "grenadelauncher" => fire_grenade_launcher(context, game, state),
        "rocketlauncher" => fire_rocket_launcher(context, game, state),
        "railgun" => fire_railgun(context, game, state),
        "bfg" => fire_bfg_weapon(context, game, state),
        "grenades" => throw_grenade_launch(context, game, state, false),
        name => panic!("Q2 weapon has no source fire callback: {name}"),
    }
}

/// Frame hooks over live arena state (`genericFrameHooks`).
struct FrameHooks<'a> {
    /// Game services.
    game: &'a mut Q2GameServices,
    /// Live weapon state.
    state: &'a mut Q2WeaponState,
    /// Weapon context.
    context: Q2WeaponContext,
}

impl FrameHooks<'_> {
    /// Context for a fire callback, with buffered input when set.
    fn firing_context(&mut self, buffered: bool) -> Q2WeaponContext {
        let mut context = self.context.clone();
        if buffered {
            context.input.attack = true;
            context.now += self.game.host.frame_seconds();
        }
        context
    }
}

impl ClassicFrameHooks for FrameHooks<'_> {
    fn random(&mut self) -> f64 {
        self.game.random()
    }

    fn ammo(&mut self) -> f64 {
        let owner = self.context.owner.actor.id().clone();
        let definition = self.context.definition.clone();
        read_ammo(self.game, &owner, &definition)
    }

    fn no_ammo(&mut self) {
        let context = self.context.clone();
        no_ammo(&context, self.game, self.state, true);
    }

    fn fire(&mut self, buffered: bool) {
        let context = self.firing_context(buffered);
        fire_weapon(&context, self.game, self.state);
    }

    fn change_weapon(&mut self) {
        let context = self.context.clone();
        change_weapon(&context, self.game, self.state);
    }

    fn reverse_animation(&mut self) {
        let context = self.context.clone();
        reverse_animation(&context, self.game);
    }

    fn attack_animation(&mut self) {
        let context = self.context.clone();
        attack_animation(&context, self.game, 1);
    }

    fn powerup_sound(&mut self) {
        let context = self.context.clone();
        powerup_sound(&context, self.game);
    }
}

impl RereleaseFrameHooks for FrameHooks<'_> {
    fn animation_time(&mut self) -> f64 {
        let context = self.context.clone();
        animation_time(&context, self.game, self.state)
    }

    fn prepare_drop(&mut self) {
        let context = self.context.clone();
        prepare_drop(&context, self.game, self.state);
    }
}

/// Report no ammo (`noAmmo`).
fn no_ammo(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    sound: bool,
) {
    let owner = context.owner.actor.id().clone();
    if sound && context.now >= state.empty_sound_time {
        game.sound(&owner, "weapons/noammo.wav", if context.rerelease { 1 } else { 2 }, 1.0, 1.0);
        state.empty_sound_time = context.now + 1.0;
    }
    state.pending.clone_from(&fallback_weapon(context, game));
}

/// Fallback weapon scan (`noAmmo` order).
fn fallback_weapon(context: &Q2WeaponContext, game: &mut Q2GameServices) -> Option<Q2WeaponName> {
    if let Some(order) = game.weapons.fallback_order.clone() {
        for name in &order {
            if let Some(found) = fallback_candidate(context, game, name) {
                return Some(found);
            }
        }
        return None;
    }
    let order: &[&str] = if context.rerelease {
                &[
                    "railgun",
                    "hyperblaster",
                    "chaingun",
                    "machinegun",
                    "supershotgun",
                    "shotgun",
                    "rocketlauncher",
                    "grenadelauncher",
                    "blaster",
                ]
            } else {
                &[
                    "railgun",
                    "hyperblaster",
                    "chaingun",
                    "machinegun",
                    "supershotgun",
                    "shotgun",
                    "blaster",
                ]
            };
    for name in order {
        if let Some(found) = fallback_candidate(context, game, name) {
            return Some(found);
        }
    }
    None
}

/// One fallback candidate.
fn fallback_candidate(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    name: &str,
) -> Option<Q2WeaponName> {
    let definition = require_definition(game, Some(&name.to_string()));
    let owner = context.owner.actor.id();
    if name != "blaster" && game.host.inventory().count(owner, &definition.item) == 0.0 {
        return None;
    }
    if definition.ammo.as_ref().is_some_and(|ammo| {
        game.host.inventory().count(owner, ammo) < f64::from(definition.quantity)
    }) {
        return None;
    }
    Some(name.to_string())
}

/// Thrown grenade definition.
fn grenade_throw() -> Q2ThrowDefinition {
    Q2ThrowDefinition {
        sound_frame: 2,
        hold_frame: 12,
        fire_frame: 15,
        cock_sound: "weapons/hgrent1a.wav".to_string(),
        hold_sound: "weapons/hgrenc1b.wav".to_string(),
        explode: true,
        wrap_before_pause: true,
        release_held: true,
        fire: grenade_throw_fire,
    }
}

/// Thrown trap definition.
fn trap_throw() -> Q2ThrowDefinition {
    Q2ThrowDefinition {
        sound_frame: 5,
        hold_frame: 9,
        fire_frame: 11,
        cock_sound: String::new(),
        hold_sound: String::new(),
        explode: false,
        wrap_before_pause: false,
        release_held: false,
        fire: trap_throw_fire,
    }
}

/// Run thrown frames (`thrownFrames`).
fn thrown_frames(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    throw: &Q2ThrowDefinition,
) {
    let owner = context.owner.actor.id().clone();
    if state.phase == Q2WeaponPhase::Dropping {
        if state.frame == context.definition.deactivate_last {
            change_weapon(context, game, state);
        } else {
            state.frame += 1;
        }
        present(context, game, state);
        return;
    }
    if state.phase == Q2WeaponPhase::Activating {
        if state.frame == context.definition.activate_last {
            state.phase = Q2WeaponPhase::Ready;
            state.frame = throw.hold_frame;
        } else {
            state.frame += 1;
        }
        present(context, game, state);
        return;
    }
    if state.phase == Q2WeaponPhase::Ready {
        if (context.input.attack || state.latched_attack) && read_ammo(game, &owner, &context.definition) > 0.0 {
            state.latched_attack = false;
            if throw.release_held && state.hand_reservation != Q2HandReservation::None {
                return;
            }
            state.phase = Q2WeaponPhase::Firing;
            state.frame = 1;
            state.grenade_time = 0.0;
        } else if (state.frame == throw.fire_frame || state.frame == throw.hold_frame)
            && read_ammo(game, &owner, &context.definition) <= 0.0
        {
            if context.rerelease {
                no_ammo(context, game, state, true);
            } else if state.empty_sound_time < game.host.now() {
                game.sound(&owner, "weapons/noammo.wav", 1, 1.0, 1.0);
                state.empty_sound_time = game.host.now() + 1.0;
            }
        } else {
            if state.frame == throw.fire_frame + 1 {
                state.frame = throw.hold_frame;
            } else {
                state.frame += 1;
            }
            if context.input.attack {
                state.latched_attack = true;
            }
        }
        present(context, game, state);
        return;
    }
    if state.phase != Q2WeaponPhase::Firing {
        present(context, game, state);
        return;
    }
    if state.frame == throw.sound_frame {
        game.sound(&owner, &throw.cock_sound, 0, 1.0, 1.0);
    }
    if state.frame == throw.hold_frame + 1 {
        state.grenade_time = game.host.now();
    }
    if state.hand_reservation == Q2HandReservation::Infinite && state.frame != throw.hold_frame + 1
    {
        state.frame = throw.hold_frame;
        present(context, game, state);
        return;
    }
    if state.hand_reservation == Q2HandReservation::Finite && state.frame == throw.hold_frame + 1
    {
        throw_holding(context, game, state, throw);
        present(context, game, state);
        return;
    }
    if state.frame == throw.fire_frame {
        if state.hand_reservation == Q2HandReservation::None {
            (throw.fire)(context, game, state, false);
        } else {
            throw_holding(context, game, state, throw);
        }
        present(context, game, state);
        return;
    }
    if throw.wrap_before_pause && state.frame == throw.fire_frame + 1 {
        state.phase = Q2WeaponPhase::Ready;
        state.frame = throw.hold_frame;
        if !context.input.attack {
            state.latched_attack = false;
        }
        present(context, game, state);
        return;
    }
    state.frame += 1;
    if state.frame == throw.fire_frame + 2 && !throw.explode {
        state.phase = Q2WeaponPhase::Ready;
        state.frame = throw.hold_frame;
    }
    present(context, game, state);
}

/// Prime a finite hold (`primeFiniteHold`).
fn prime_finite_hold(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    throw: &Q2ThrowDefinition,
) {
    let owner = context.owner.actor.id().clone();
    let owned = game.owned_of(owner.clone());
    let Some(ammo) = context.definition.ammo.clone() else { return };
    if !game.host.inventory().consume(&owned, &ammo, 1.0) {
        return;
    }
    ammo_changed(game, &owner, &ammo);
    let haste = context.input.haste || game.weapons.match_hooks.ctf.is_some_and(|ctf| (ctf.haste)(context, game));
    let recovery = hand_recovery_seconds(&HandGrenadeTempo {
        edition: game.options.edition,
        haste,
        quad_fire: context.input.quad_fire_until > game.host.now(),
    });
    let fuse = hand_deadline(context.now, 3.0, game.options.edition);
    state.hand_reservation = Q2HandReservation::Finite;
    state.grenade_time = context.now;
    state.grenade_finished = fuse;
    state.grenade_blew_up = false;
    game.sound(&owner, &throw.hold_sound, 1, 1.0, 1.0);
    let _ = recovery;
}

/// Prime an infinite hold (`primeInfiniteHold`).
fn prime_infinite_hold(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    throw: &Q2ThrowDefinition,
) {
    let owner = context.owner.actor.id().clone();
    state.hand_reservation = Q2HandReservation::Infinite;
    state.grenade_time = context.now;
    state.grenade_finished = hand_deadline(context.now, 3.0, game.options.edition);
    state.grenade_blew_up = false;
    game.sound(&owner, &throw.hold_sound, 1, 1.0, 1.0);
}

/// Release a finite hold (`releaseFiniteHold`).
fn release_finite_hold(game: &mut Q2GameServices, owner: &ActorId, state: &mut Q2WeaponState) {
    state.hand_reservation = Q2HandReservation::None;
    state.grenade_time = 0.0;
    state.grenade_finished = 0.0;
    state.grenade_blew_up = false;
    game.sound(owner, "weapons/hgrenc1b.wav", 1, 1.0, 1.0);
}

/// Release an infinite hold (`releaseInfiniteHold`).
fn release_infinite_hold(
    game: &mut Q2GameServices,
    owner: &ActorId,
    state: &mut Q2WeaponState,
    armed: bool,
) {
    state.hand_reservation = Q2HandReservation::None;
    state.grenade_time = 0.0;
    state.grenade_finished = if armed { game.host.now() } else { 0.0 };
    state.grenade_blew_up = false;
    game.sound(owner, "weapons/hgrenc1b.wav", 1, 1.0, 1.0);
}

/// Throw while holding (`throwHolding`).
fn throw_holding(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    throw: &Q2ThrowDefinition,
) {
    let owner = context.owner.actor.id().clone();
    if state.grenade_blew_up {
        return;
    }
    if state.hand_reservation == Q2HandReservation::Finite {
        let armed = game.host.now() >= state.grenade_finished;
        if armed {
            state.grenade_blew_up = true;
        } else if !context.input.attack {
            release_finite_hold(game, &owner, state);
        }
        (throw.fire)(context, game, state, !armed);
        if armed && throw.release_held {
            release_finite_hold(game, &owner, state);
        }
        return;
    }
    let armed = context.input.attack && game.host.now() >= state.grenade_time + 5.0;
    if !context.input.attack || armed {
        (throw.fire)(context, game, state, true);
        release_infinite_hold(game, &owner, state, armed);
    }
}

/// Keep attacking (`continuesAttack`).
fn continues_attack(context: &Q2WeaponContext, state: &Q2WeaponState) -> bool {
    state.primary_handoff == PrimaryHandoff::Active && context.input.attack
}

/// Symmetric random spread (`game.host.random() * 2 - 1`).
fn spread_random(game: &mut Q2GameServices) -> f64 {
    game.random() * 2.0 - 1.0
}

/// Set the weapon loop sound (`setLoop`).
fn set_loop(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    path: &str,
) {
    if state.loop_sound == path {
        return;
    }
    let owner = context.owner.actor.id().clone();
    let origin = game.body_of(owner.clone()).origin;
    if !state.loop_sound.is_empty() {
        let stop = state.loop_sound.clone();
        game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(owner.clone()),
            origin,
            path: stop,
            channel: 1,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Stop,
            loop_owner: None,
        }));
    }
    state.loop_sound = path.to_string();
    if !path.is_empty() {
        game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(owner),
            origin,
            path: path.to_string(),
            channel: 1,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Start,
            loop_owner: None,
        }));
    }
}

/// Emit a muzzle flash (`flash`).
fn muzzle_flash(context: &Q2WeaponContext, game: &mut Q2GameServices, flash: i32) {
    weapon_emit(
        game,
        &Q2WeaponEvent::Muzzleflash {
            actor: context.owner.actor.id().clone(),
            flash,
            silenced: context.silenced,
        },
    );
}

/// Damage multiplier (`multiplier`).
fn damage_multiplier(context: &Q2WeaponContext, game: &mut Q2GameServices) -> f64 {
    q2_weapon_damage_multiplier(context.owner.actor.id(), &context.input, context.now, game)
}

/// Begin lag compensation (`hooks.lagCompensation.begin`).
fn lag_begin(
    game: &mut Q2GameServices,
    owner: &ActorId,
    start: Vec3,
    direction: Vec3,
) -> Option<LagToken> {
    let mut engine = game.weapons.engine.take();
    let token = engine
        .as_mut()
        .and_then(|engine| engine.lag_begin(owner, start, direction));
    game.weapons.engine = engine;
    token
}

/// End lag compensation.
fn lag_end(game: &mut Q2GameServices, token: Option<LagToken>) {
    let Some(token) = token else { return };
    let mut engine = game.weapons.engine.take();
    if let Some(engine) = engine.as_mut() {
        engine.lag_end(token);
    }
    game.weapons.engine = engine;
}

/// Fire a blaster bolt (`blaster`).
#[allow(clippy::too_many_arguments)]
fn fire_blaster_weapon(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    offset: Vec3,
    damage: f64,
    hyper: bool,
    effects: i64,
) {
    let (start, direction) = project_weapon(
        &context.owner,
        game,
        &context.input,
        context.input.angles,
        add3(vec3(24.0, 8.0, -8.0), offset),
    );
    let kick_angles = if hyper && context.rerelease {
        vec3(
            (spread_random(game) * 0.7) as f32,
            (spread_random(game) * 0.7) as f32,
            (spread_random(game) * 0.7) as f32,
        )
    } else {
        vec3(-1.0, 0.0, 0.0)
    };
    let forward = angle_vectors(context.input.angles).forward;
    set_q2_weapon_recoil(
        state,
        game.options.edition,
        context.now,
        scale3(forward, -2.0),
        kick_angles,
        None,
    );
    let multiplier = damage_multiplier(context, game);
    let owner = context.owner.actor.id().clone();
    fire_blaster(
        owner.clone(),
        game,
        start,
        direction,
        damage * multiplier,
        if context.rerelease && !hyper {
            1500.0
        } else {
            1000.0
        },
        effects,
        hyper,
        if hyper {
            Mod::HYPERBLASTER
        } else {
            Mod::BLASTER
        },
    );
    muzzle_flash(context, game, if hyper { 14 } else { 0 });
    player_noise(game, &owner, start, NoiseKind::Weapon);
}

/// Fire the hyperblaster (`hyperblaster`).
fn fire_hyperblaster(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
) {
    let owner = context.owner.actor.id().clone();
    let damage = if game.options.mode == Q2Mode::Deathmatch {
        15.0
    } else {
        20.0
    };
    if context.rerelease {
        state.frame = if state.frame > 20 { 6 } else { state.frame + 1 };
        if state.frame == 12 {
            if read_ammo(game, &owner, &context.definition) > 0.0
                && continues_attack(context, state)
            {
                state.frame = 6;
            } else {
                game.sound(&owner, "weapons/hyprbd1a.wav", 0, 1.0, 1.0);
            }
        }
        let spinning = (6..=11).contains(&state.frame);
        set_loop(
            context,
            game,
            state,
            if spinning {
                "weapons/hyprbl1a.wav"
            } else {
                ""
            },
        );
        if continues_attack(context, state) && spinning {
            if read_ammo(game, &owner, &context.definition) < 1.0 {
                no_ammo(context, game, state, true);
                return;
            }
            let rotation =
                f64::from(state.frame - 5) * 2.0 * std::f64::consts::PI / 6.0;
            let offset = vec3(
                (-4.0 * rotation.sin()) as f32,
                (4.0 * rotation.cos()) as f32,
                0.0,
            );
            fire_blaster_weapon(
                context,
                game,
                state,
                offset,
                damage,
                true,
                if state.frame % 4 == 0 { 64 } else { 0 },
            );
            powerup_sound(context, game);
            use_ammo(context, game, state);
            let offset_frame = (game.random() + 0.25).trunc() as i32;
            attack_animation(context, game, offset_frame);
        }
        return;
    }
    set_loop(context, game, state, "weapons/hyprbl1a.wav");
    if !continues_attack(context, state) {
        state.frame += 1;
    } else {
        if read_ammo(game, &owner, &context.definition) < 1.0 {
            no_ammo(context, game, state, true);
        } else {
            let rotation =
                f64::from(state.frame - 5) * 2.0 * std::f64::consts::PI / 6.0;
            let offset = vec3(
                (-4.0 * rotation.sin()) as f32,
                0.0,
                (4.0 * rotation.cos()) as f32,
            );
            fire_blaster_weapon(
                context,
                game,
                state,
                offset,
                damage,
                true,
                if state.frame == 6 || state.frame == 9 {
                    64
                } else {
                    0
                },
            );
            use_ammo(context, game, state);
            attack_animation(context, game, 1);
        }
        state.frame += 1;
        if state.frame == 12 && read_ammo(game, &owner, &context.definition) > 0.0 {
            state.frame = 6;
        }
    }
    if state.frame == 12 {
        game.sound(&owner, "weapons/hyprbd1a.wav", 0, 1.0, 1.0);
        set_loop(context, game, state, "");
    }
}

/// Fire the machinegun (`machinegun`).
fn fire_machinegun(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
) {
    let owner = context.owner.actor.id().clone();
    if !continues_attack(context, state) {
        state.machinegun_shots = 0;
        if context.rerelease {
            state.frame = 6;
        } else {
            state.frame += 1;
        }
        return;
    }
    state.frame = if state.frame == 4 { 5 } else { 4 };
    if read_ammo(game, &owner, &context.definition) < 1.0 {
        state.frame = 6;
        no_ammo(context, game, state, true);
        return;
    }
    let (kick_origin, kick_angles) = if context.rerelease {
        (
            vec3(
                (spread_random(game) * 0.35) as f32,
                (spread_random(game) * 0.35) as f32,
                (spread_random(game) * 0.35) as f32,
            ),
            vec3(
                (spread_random(game) * 0.7) as f32,
                (spread_random(game) * 0.7) as f32,
                (spread_random(game) * 0.7) as f32,
            ),
        )
    } else {
        let oy = spread_random(game) * 0.35;
        let ay = spread_random(game) * 0.7;
        let oz = spread_random(game) * 0.35;
        let az = spread_random(game) * 0.7;
        let ox = spread_random(game) * 0.35;
        let angles = vec3(-1.5 * state.machinegun_shots as f32, ay as f32, az as f32);
        if game.options.mode != Q2Mode::Deathmatch {
            state.machinegun_shots = (state.machinegun_shots + 1).min(9);
        }
        (vec3(ox as f32, oy as f32, oz as f32), angles)
    };
    set_q2_weapon_recoil(
        state,
        game.options.edition,
        context.now,
        kick_origin,
        kick_angles,
        None,
    );
    let angles = if context.rerelease {
        context.input.angles
    } else {
        add3(context.input.angles, kick_angles)
    };
    let offset = if context.rerelease {
        vec3(0.0, 0.0, -8.0)
    } else {
        vec3(0.0, 8.0, -8.0)
    };
    let (start, direction) =
        project_weapon(&context.owner, game, &context.input, angles, offset);
    let token = if context.rerelease {
        lag_begin(game, &owner, start, direction)
    } else {
        None
    };
    let multiplier = damage_multiplier(context, game);
    fire_bullet(
        owner.clone(),
        game,
        start,
        direction,
        8.0 * multiplier,
        2.0 * multiplier,
        300.0,
        500.0,
        Mod::MACHINEGUN,
    );
    lag_end(game, token);
    if context.rerelease {
        powerup_sound(context, game);
    }
    muzzle_flash(context, game, 1);
    player_noise(game, &owner, start, NoiseKind::Weapon);
    use_ammo(context, game, state);
    let offset_frame = (game.random() + 0.25).trunc() as i32;
    attack_animation(context, game, offset_frame);
}

/// Fire the chaingun (`chaingun`).
fn fire_chaingun(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
) {
    let owner = context.owner.actor.id().clone();
    if context.rerelease && state.frame > 31 {
        state.frame = 5;
        game.sound(&owner, "weapons/chngnu1a.wav", 0, 1.0, 2.0);
    } else {
        if !context.rerelease && state.frame == 5 {
            game.sound(&owner, "weapons/chngnu1a.wav", 0, 1.0, 2.0);
        }
        if state.frame == 14 && !continues_attack(context, state) {
            state.frame = 32;
            set_loop(context, game, state, "");
            return;
        }
        if state.frame == 21
            && continues_attack(context, state)
            && read_ammo(game, &owner, &context.definition) > 0.0
        {
            state.frame = 15;
        } else {
            state.frame += 1;
        }
    }
    if state.frame == 22 {
        set_loop(context, game, state, "");
        game.sound(&owner, "weapons/chngnd1a.wav", 0, 1.0, 2.0);
    } else if !context.rerelease {
        set_loop(context, game, state, "weapons/chngnl1a.wav");
    }
    if context.rerelease && (state.frame < 5 || state.frame > 21) {
        return;
    }
    if context.rerelease {
        set_loop(context, game, state, "weapons/chngnl1a.wav");
    }
    attack_animation(context, game, state.frame & 1);
    let want = if state.frame <= 9 {
        1
    } else if state.frame <= 14 {
        if continues_attack(context, state) {
            2
        } else {
            1
        }
    } else {
        3
    };
    let shots = read_ammo(game, &owner, &context.definition).min(f64::from(want));
    if shots == 0.0 {
        no_ammo(context, game, state, true);
        return;
    }
    let shots = shots.ceil() as i32;
    let (kick_origin, kick_angles) = if context.rerelease {
        let factor = 0.5 + f64::from(shots) * 0.15;
        (
            vec3(
                (spread_random(game) * 0.35) as f32,
                (spread_random(game) * 0.35) as f32,
                (spread_random(game) * 0.35) as f32,
            ),
            vec3(
                (spread_random(game) * factor) as f32,
                (spread_random(game) * factor) as f32,
                (spread_random(game) * factor) as f32,
            ),
        )
    } else {
        let ox = spread_random(game) * 0.35;
        let ax = spread_random(game) * 0.7;
        let oy = spread_random(game) * 0.35;
        let ay = spread_random(game) * 0.7;
        let oz = spread_random(game) * 0.35;
        let az = spread_random(game) * 0.7;
        (
            vec3(ox as f32, oy as f32, oz as f32),
            vec3(ax as f32, ay as f32, az as f32),
        )
    };
    set_q2_weapon_recoil(
        state,
        game.options.edition,
        context.now,
        kick_origin,
        kick_angles,
        None,
    );
    let initial = project_weapon(
        &context.owner,
        game,
        &context.input,
        context.input.angles,
        vec3(0.0, 0.0, -8.0),
    );
    let token = if context.rerelease {
        lag_begin(game, &owner, initial.0, initial.1)
    } else {
        None
    };
    let multiplier = damage_multiplier(context, game);
    let deathmatch = game.options.mode == Q2Mode::Deathmatch;
    let mut noise_origin = initial.0;
    for _ in 0..shots {
        let side = (if context.rerelease { 0.0 } else { 7.0 }) + game.random() * 4.0;
        let up = game.random() * 4.0 - 8.0;
        let (start, direction) = project_weapon(
            &context.owner,
            game,
            &context.input,
            context.input.angles,
            vec3(0.0, side as f32, up as f32),
        );
        noise_origin = start;
        fire_bullet(
            owner.clone(),
            game,
            start,
            direction,
            (if deathmatch { 6.0 } else { 8.0 }) * multiplier,
            2.0 * multiplier,
            300.0,
            500.0,
            Mod::CHAINGUN,
        );
    }
    lag_end(game, token);
    if context.rerelease {
        powerup_sound(context, game);
    }
    muzzle_flash(context, game, 3 + shots - 1);
    player_noise(game, &owner, noise_origin, NoiseKind::Weapon);
    use_ammo_count(context, game, state, f64::from(shots));
}

/// Fire a shotgun (`shotgun`).
fn fire_shotgun_weapon(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    super_shotgun: bool,
) {
    let owner = context.owner.actor.id().clone();
    if !super_shotgun && state.frame == 9 {
        if !context.rerelease {
            state.frame += 1;
        }
        return;
    }
    let offset = if context.rerelease {
        vec3(0.0, 0.0, -8.0)
    } else {
        vec3(0.0, 8.0, -8.0)
    };
    let (start, direction) = project_weapon(
        &context.owner,
        game,
        &context.input,
        context.input.angles,
        offset,
    );
    let forward = angle_vectors(context.input.angles).forward;
    set_q2_weapon_recoil(
        state,
        game.options.edition,
        context.now,
        scale3(forward, -2.0),
        vec3(-2.0, 0.0, 0.0),
        None,
    );
    let token = if context.rerelease {
        lag_begin(game, &owner, start, direction)
    } else {
        None
    };
    let multiplier = damage_multiplier(context, game);
    let mut noise_origin = start;
    if super_shotgun {
        for yaw in [-5.0f32, 5.0f32] {
            let angles = vec3(
                context.input.angles.x,
                context.input.angles.y + yaw,
                context.input.angles.z,
            );
            let (pellet_start, pellet_direction) = if context.rerelease {
                project_weapon(
                    &context.owner,
                    game,
                    &context.input,
                    angles,
                    vec3(0.0, 0.0, -8.0),
                )
            } else {
                (start, angle_vectors(angles).forward)
            };
            noise_origin = pellet_start;
            fire_shotgun(
                owner.clone(),
                game,
                pellet_start,
                pellet_direction,
                6.0 * multiplier,
                12.0 * multiplier,
                1000.0,
                500.0,
                10,
                Mod::SUPERSHOTGUN,
            );
        }
    } else {
        fire_shotgun(
            owner.clone(),
            game,
            start,
            direction,
            4.0 * multiplier,
            8.0 * multiplier,
            500.0,
            500.0,
            12,
            Mod::SHOTGUN,
        );
    }
    lag_end(game, token);
    muzzle_flash(context, game, if super_shotgun { 13 } else { 2 });
    if !context.rerelease {
        state.frame += 1;
    }
    player_noise(game, &owner, noise_origin, NoiseKind::Weapon);
    use_ammo(context, game, state);
}

/// Fire the grenade launcher (`grenadeLauncher`).
fn fire_grenade_launcher(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
) {
    let owner = context.owner.actor.id().clone();
    let angles = if context.rerelease {
        vec3(
            (-62.5f32).max(context.input.angles.x),
            context.input.angles.y,
            context.input.angles.z,
        )
    } else {
        context.input.angles
    };
    let offset = if context.rerelease {
        vec3(8.0, 0.0, -8.0)
    } else {
        vec3(8.0, 8.0, -8.0)
    };
    let (start, direction) =
        project_weapon(&context.owner, game, &context.input, angles, offset);
    let forward = angle_vectors(context.input.angles).forward;
    set_q2_weapon_recoil(
        state,
        game.options.edition,
        context.now,
        scale3(forward, -2.0),
        vec3(-1.0, 0.0, 0.0),
        None,
    );
    let multiplier = damage_multiplier(context, game);
    let adjustment = if context.rerelease {
        let right =
            (-0.9999999403953552 + game.random() * 1.9999999403953552) * 10.0;
        let up = 200.0 + (-0.9999999403953552 + game.random() * 1.9999999403953552) * 10.0;
        Some(Q2GrenadeAdjustment {
            right,
            up,
            gravity: context.input.gravity,
        })
    } else {
        None
    };
    fire_grenade(
        owner.clone(),
        game,
        start,
        direction,
        120.0 * multiplier,
        600.0,
        2.5,
        160.0,
        false,
        false,
        false,
        adjustment,
    );
    muzzle_flash(context, game, 8);
    if !context.rerelease {
        state.frame += 1;
    }
    player_noise(game, &owner, start, NoiseKind::Weapon);
    use_ammo(context, game, state);
}

/// Fire the rocket launcher (`rocketLauncher`).
fn fire_rocket_launcher(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
) {
    let owner = context.owner.actor.id().clone();
    let damage = 100.0 + (game.random() * 20.0).floor();
    let (start, direction) = project_weapon(
        &context.owner,
        game,
        &context.input,
        context.input.angles,
        vec3(8.0, 8.0, -8.0),
    );
    let forward = angle_vectors(context.input.angles).forward;
    set_q2_weapon_recoil(
        state,
        game.options.edition,
        context.now,
        scale3(forward, -2.0),
        vec3(-1.0, 0.0, 0.0),
        None,
    );
    let multiplier = damage_multiplier(context, game);
    fire_rocket(
        owner.clone(),
        game,
        start,
        direction,
        damage * multiplier,
        650.0,
        120.0,
        120.0 * multiplier,
    );
    muzzle_flash(context, game, 7);
    if !context.rerelease {
        state.frame += 1;
    }
    player_noise(game, &owner, start, NoiseKind::Weapon);
    use_ammo(context, game, state);
}

/// Fire the railgun (`railgun`).
fn fire_railgun(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
) {
    let owner = context.owner.actor.id().clone();
    let (start, direction) = project_weapon(
        &context.owner,
        game,
        &context.input,
        context.input.angles,
        vec3(0.0, 7.0, -8.0),
    );
    let deathmatch = game.options.mode == Q2Mode::Deathmatch;
    let forward = angle_vectors(context.input.angles).forward;
    set_q2_weapon_recoil(
        state,
        game.options.edition,
        context.now,
        scale3(forward, -3.0),
        vec3(-3.0, 0.0, 0.0),
        None,
    );
    let token = if context.rerelease {
        lag_begin(game, &owner, start, direction)
    } else {
        None
    };
    let multiplier = damage_multiplier(context, game);
    let damage = if deathmatch {
        100.0
    } else if context.rerelease {
        125.0
    } else {
        150.0
    } * multiplier;
    let kick = if deathmatch {
        200.0
    } else if context.rerelease {
        225.0
    } else {
        250.0
    } * multiplier;
    fire_rail(owner.clone(), game, start, direction, damage, kick);
    lag_end(game, token);
    muzzle_flash(context, game, 6);
    if !context.rerelease {
        state.frame += 1;
    }
    player_noise(game, &owner, start, NoiseKind::Weapon);
    use_ammo(context, game, state);
}

/// Fire the BFG (`bfg`).
fn fire_bfg_weapon(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
) {
    let owner = context.owner.actor.id().clone();
    if state.frame == 9 {
        muzzle_flash(context, game, 12);
        if !context.rerelease {
            state.frame += 1;
        }
        // Classic C reads an uninitialized stack vector here; keep the
        // donor's explicit zero vector.
        let origin = if context.rerelease {
            game.body_of(owner.clone()).origin
        } else {
            vec3(0.0, 0.0, 0.0)
        };
        player_noise(game, &owner, origin, NoiseKind::Weapon);
        return;
    }
    if read_ammo(game, &owner, &context.definition) < 50.0 {
        if !context.rerelease {
            state.frame += 1;
        }
        return;
    }
    let (start, direction) = project_weapon(
        &context.owner,
        game,
        &context.input,
        context.input.angles,
        vec3(8.0, 8.0, -8.0),
    );
    let multiplier = damage_multiplier(context, game);
    let damage = if game.options.mode == Q2Mode::Deathmatch {
        200.0
    } else {
        500.0
    } * multiplier;
    fire_bfg(owner.clone(), game, start, direction, damage, 400.0, 1000.0);
    let forward = angle_vectors(context.input.angles).forward;
    let duration = if context.rerelease {
        0.6 - game.host.frame_seconds()
    } else {
        0.5
    };
    set_q2_weapon_recoil(
        state,
        game.options.edition,
        context.now,
        scale3(forward, -2.0),
        vec3(
            if context.rerelease { -20.0 } else { -40.0 },
            0.0,
            (spread_random(game) * 8.0) as f32,
        ),
        Some(duration),
    );
    if context.rerelease {
        muzzle_flash(context, game, 19);
    } else {
        state.frame += 1;
    }
    player_noise(game, &owner, start, NoiseKind::Weapon);
    use_ammo(context, game, state);
}

/// Throw a hand grenade (`throwGrenade`).
fn throw_grenade_launch(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    held: bool,
) {
    if context.definition.name == "grenades"
        && state.hand_reservation == Q2HandReservation::None
    {
        return;
    }
    let owner = context.owner.actor.id().clone();
    let alive = game
        .host
        .combat()
        .read(&owner)
        .map(|record| record.health)
        .unwrap_or(0.0)
        > 0.0;
    let multiplier = damage_multiplier(context, game);
    let edition = game.options.edition;
    let throw_owner = context.owner.clone();
    let throw_input = context.input.clone();
    let spec = {
        let mut project = |angles: Vec3, offset: Vec3| {
            project_weapon(&throw_owner, game, &throw_input, angles, offset)
        };
        calculate_hand_throw(HandThrowInput {
            edition,
            angles: context.input.angles,
            alive,
            now: context.now,
            fuse_deadline: state.grenade_time,
            damage_multiplier: multiplier,
            gravity: context.input.gravity,
            held,
            project: &mut project,
        })
    };
    if context.definition.name == "grenades" {
        state.hand_reservation = Q2HandReservation::None;
    }
    state.grenade_time = if context.rerelease {
        0.0
    } else {
        context.now + firing_interval(game, &owner, 1.0)
    };
    fire_grenade(
        owner.clone(),
        game,
        spec.start,
        spec.direction,
        spec.damage,
        spec.speed,
        spec.fuse,
        spec.radius,
        true,
        spec.held,
        false,
        None,
    );
    if context.definition.name != "grenades" {
        use_ammo_count(context, game, state, 1.0);
    }
    if !context.rerelease && alive {
        if context.input.ducked {
            animate_player(context, game, PlayerAnimationPriority::Attack, 159, 162);
        } else {
            animate_player(context, game, PlayerAnimationPriority::Reverse, 119, 112);
        }
    }
}

/// Fire a thrown grenade (`grenadeThrow.fire`).
fn grenade_throw_fire(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    held: bool,
) {
    throw_grenade_launch(context, game, state, held);
}

/// Fire a thrown trap (`trapThrow.fire`).
///
/// The missionpack trap and tesla extensions register their own throw
/// definitions; this default shares the hand-grenade throw until they do.
fn trap_throw_fire(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    held: bool,
) {
    throw_grenade_launch(context, game, state, held);
}