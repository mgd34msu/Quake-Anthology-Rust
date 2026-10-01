//! Player weapons (`src/content/q2/foundation/weapons/player.ts`).
//!
//! Adapted from id Software's Quake II `g_weapon.c`, `p_weapon.c` and
//! rerelease `g_weapon.cpp`, `p_weapon.cpp` (GPL-2.0-or-later). All
//! damage is admitted by the session combat authority.

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{add3, scale3, vec3, Vec3};

use super::super::items::{add_player_power_cells, flush_player_power_cells};
use super::ballistics::{
    fire_bfg, fire_blaster, fire_bullet, fire_grenade, fire_rail, fire_rocket, fire_shotgun, player_noise,
    register_ballistics_callbacks, NoiseKind,
};
use super::damage::q2_weapon_damage_multiplier;
use super::generic_frame::{
    millisecond_sum, step_q2_classic_frame, step_q2_rerelease_frame, ClassicFrameHooks, GenericFrameState,
    Q2ClassicFrameInput, Q2GenericDefinition, Q2RereleaseFrameInput, RereleaseFrameHooks,
};
use super::hand_grenade::{
    calculate_hand_throw, hand_fuse_deadline, hand_recovery_seconds, HandGrenadeTempo, HandThrowInput,
};
use super::presentation::{
    q2_attack_frames, q2_powerup_sound, q2_reverse_frames, q2_weapon_animation_rate, set_q2_weapon_recoil,
    Q2AnimationRateInput, Q2PowerupSoundInput,
};
use super::projection::{project_q2_actor, Q2ActorView};
use super::types::{
    LagToken, Mod, PlayerAnimationPriority, PrimaryHandoff, Q2GrenadeAdjustment, Q2HandReservation, Q2WeaponDefinition,
    Q2WeaponEvent, Q2WeaponInput, Q2WeaponName, Q2WeaponOwner, Q2WeaponPhase, Q2WeaponState,
};
use super::vectors::angle_vectors;
use crate::contract::{InventoryEntry, ItemId};
use crate::q2::foundation::host::{Q2Edition, Q2GameServices, Q2Mode, Q2PresentationEvent, Q2SoundEvent, Q2SoundLoop};

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
#[derive(Debug, Clone)]
pub struct Q2WeaponSelectionRule {
    /// Requested weapon.
    pub requested: Q2WeaponName,
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
    fn think(&mut self, _context: &Q2WeaponContext, _game: &mut Q2GameServices, _state: &mut Q2WeaponState) -> bool {
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
    fn selection(&self) -> Option<Q2WeaponSelectionRule> {
        None
    }
    /// Whether the extension implements the held hook.
    fn has_held(&self) -> bool {
        false
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

/// Report ammo changes (`hooks.ammoChanged`).
fn ammo_changed(game: &mut Q2GameServices, actor: &ActorId, ammo: &ItemId) {
    let mut engine = game.weapons.engine.take();
    if let Some(engine) = engine.as_mut() {
        engine.ammo_changed(actor, ammo);
    }
    game.weapons.engine = engine;
}

/// Report ammo changes (`hooks.ammoChanged`, shared with equipment).
pub fn weapon_ammo_changed(game: &mut Q2GameServices, actor: &ActorId, ammo: &ItemId) {
    ammo_changed(game, actor, ammo);
}

/// Adjust a firing interval (`hooks.firingInterval`, shared with equipment).
pub fn weapon_firing_interval(game: &mut Q2GameServices, actor: &ActorId, seconds: f64) -> f64 {
    firing_interval(game, actor, seconds)
}

/// Project a muzzle (`project`, shared with weapon extensions).
pub fn weapon_project(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    offset: Vec3,
    angles: Option<Vec3>,
) -> (Vec3, Vec3) {
    project_weapon(
        &context.owner,
        game,
        &context.input,
        angles.unwrap_or(context.input.angles),
        offset,
    )
}

/// Apply view kick (`kick`, shared with weapon extensions).
pub fn weapon_kick(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    origin: Vec3,
    angles: Vec3,
) {
    set_q2_weapon_recoil(state, game.options.edition, context.now, origin, angles, None);
}

/// Emit a muzzle flash (`flash`, shared with weapon extensions).
pub fn weapon_flash(context: &Q2WeaponContext, game: &mut Q2GameServices, flash: i32) {
    muzzle_flash(context, game, flash);
}

/// Read ammo (`ammo`, shared with weapon extensions).
pub fn weapon_ammo(context: &Q2WeaponContext, game: &mut Q2GameServices) -> f64 {
    read_ammo(game, context.owner.actor.id(), &context.definition)
}

/// Damage multiplier (`multiplier`, shared with weapon extensions).
pub fn weapon_multiplier(context: &Q2WeaponContext, game: &mut Q2GameServices) -> f64 {
    damage_multiplier(context, game)
}

/// Handle missing ammo (`noAmmo`, shared with weapon extensions).
pub fn weapon_no_ammo(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState, sound: bool) {
    no_ammo(context, game, state, sound);
}

/// Consume ammo (`consume`, shared with weapon extensions).
pub fn weapon_consume(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    use_ammo(context, game, state);
}

/// Consume counted ammo (`consume(context, count)`, shared with weapon extensions).
pub fn weapon_consume_count(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    count: f64,
) {
    use_ammo_count(context, game, state, count);
}

/// Consume counted ammo with an explicit infinite flag (`consume(context, count, infinite)`, shared with weapon extensions).
pub fn weapon_consume_infinite(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    count: f64,
    infinite: bool,
) {
    consume_ammo(context, game, state, count, infinite);
}

/// Whether the attack continues (`continuesAttack`, shared with weapon extensions).
pub fn weapon_continues_attack(context: &Q2WeaponContext, state: &Q2WeaponState) -> bool {
    continues_attack(context, state)
}

/// Set the loop sound (`setLoop`, shared with weapon extensions).
pub fn weapon_set_loop(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState, path: &str) {
    set_loop(context, game, state, path);
}

/// Play the powerup sound (`powerupSound`, shared with weapon extensions).
pub fn weapon_powerup_sound(context: &Q2WeaponContext, game: &mut Q2GameServices) {
    powerup_sound(context, game);
}

/// Step the classic frame (`genericClassic`, shared with weapon extensions).
pub fn weapon_generic_classic(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    generic_classic(context, game, state);
}

/// Step the rerelease frame (`genericRerelease`, shared with weapon extensions).
pub fn weapon_generic_rerelease(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    generic_rerelease(context, game, state);
}

/// Step a classic throw (`throwClassic`, shared with weapon extensions).
pub fn weapon_throw_classic(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    throwing: Option<&Q2ThrowDefinition>,
) {
    throw_classic(context, game, state, throwing);
}

/// Step a rerelease throw (`throwRerelease`, shared with weapon extensions).
pub fn weapon_throw_rerelease(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    throwing: Option<&Q2ThrowDefinition>,
) {
    throw_rerelease(context, game, state, throwing);
}

/// Emit a weapon event (`hooks.emit`, shared with weapon extensions).
pub fn weapon_emit(game: &mut Q2GameServices, event: &Q2WeaponEvent) {
    let mut engine = game.weapons.engine.take();
    if let Some(engine) = engine.as_mut() {
        engine.emit(event);
    }
    game.weapons.engine = engine;
}

/// Begin lag compensation (`lagCompensation.begin`, shared with weapon extensions).
pub fn weapon_lag_begin(game: &mut Q2GameServices, owner: &ActorId, start: Vec3, direction: Vec3) -> Option<LagToken> {
    lag_begin(game, owner, start, direction)
}

/// End lag compensation (shared with weapon extensions).
pub fn weapon_lag_end(game: &mut Q2GameServices, token: Option<LagToken>) {
    lag_end(game, token);
}

/// Target eligibility (`hooks.canTarget`, shared with weapon extensions).
pub fn weapon_can_target(game: &mut Q2GameServices, attacker: Option<&ActorId>, target: &ActorId) -> bool {
    let mut engine = game.weapons.engine.take();
    let allowed = engine
        .as_mut()
        .map(|engine| engine.can_target(attacker, target))
        .unwrap_or(attacker != Some(target));
    game.weapons.engine = engine;
    allowed
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
    let mut state = game
        .weapons
        .states
        .remove(owner)
        .unwrap_or_else(|| Q2WeaponState::new(None));
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
pub fn register_weapon_extension(game: &mut Q2GameServices, extension: Box<dyn Q2WeaponExtension>) {
    register_ballistics_callbacks(game);
    let name = extension.definition().name.clone();
    game.weapons
        .definitions
        .insert(name.clone(), extension.definition().clone());
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

/// Consume ammo (`consume`).
fn use_ammo(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    use_ammo_count(context, game, state, f64::from(context.definition.quantity));
}

/// Consume counted ammo (`consume(context, count)`).
fn use_ammo_count(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState, count: f64) {
    consume_ammo(context, game, state, count, true);
}

/// Consume counted ammo with an explicit infinite flag (`consume(context, count, infinite)`).
fn consume_ammo(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    count: f64,
    infinite: bool,
) {
    if infinite && context.input.infinite_ammo {
        return;
    }
    let Some(ammo) = context.definition.ammo.clone() else {
        return;
    };
    if state.hand_reservation != Q2HandReservation::None {
        return;
    }
    let owner = context.owner.actor.id().clone();
    if ammo == "q2:ammo_cells" {
        flush_player_power_cells(game, &owner);
    }
    let before = game.host.inventory().count(&owner, &ammo);
    let owned = game.owned_of(owner.clone());
    if !game.host.inventory().consume(&owned, &ammo, count) {
        return;
    }
    if ammo == "q2:ammo_cells" {
        add_player_power_cells(game, &owner, -count);
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
    weapon_emit(
        game,
        &Q2WeaponEvent::PlayerAnimation {
            actor: context.owner.actor.id().clone(),
            priority,
            first,
            last,
            reset_time: context.rerelease,
        },
    );
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
fn animation_time(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) -> f64 {
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
    state.gun_rate = if interval == native {
        f64::from(rate)
    } else {
        1.0 / interval
    };
    interval
}

/// Prepare a drop (`prepareDrop`).
fn prepare_drop(context: &Q2WeaponContext, _game: &mut Q2GameServices, state: &mut Q2WeaponState) {
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
fn present(owner: &Q2WeaponOwner, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    let definition = state
        .weapon
        .as_ref()
        .and_then(|weapon| game.weapons.definitions.get(weapon));
    let (kick_origin, kick_angles) =
        super::presentation::q2_weapon_recoil(state, game.options.edition, game.host.now());
    weapon_emit(
        game,
        &Q2WeaponEvent::ViewWeapon {
            actor: owner.actor.id().clone(),
            weapon: state.weapon.clone(),
            model: if state.primary_handoff == PrimaryHandoff::Holstered {
                String::new()
            } else {
                state
                    .view_model
                    .clone()
                    .or_else(|| definition.map(|definition| definition.view_model.clone()))
                    .unwrap_or_default()
            },
            player_model: definition.map(|definition| definition.player_model).unwrap_or(0),
            frame: state.frame,
            skin: state.view_skin,
            rate: state.gun_rate,
            kick_origin,
            kick_angles,
        },
    );
}

/// Fire the readied weapon (`fire`).
fn fire_weapon(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    if game.weapons.extensions.contains_key(&context.definition.name) {
        let name = context.definition.name.clone();
        let mut extension = game.weapons.extensions.remove(&name).expect("weapon extension");
        extension.fire(context, game, state);
        game.weapons.extensions.insert(name, extension);
        return;
    }
    match context.definition.name.as_str() {
        "blaster" => {
            let damage = if context.rerelease || game.options.mode == Q2Mode::Deathmatch {
                15.0
            } else {
                10.0
            };
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
        change_weapon(&context.owner, self.game, self.state, &context.input);
    }

    fn frame_state(&mut self) -> &mut dyn GenericFrameState {
        self.state
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
fn no_ammo(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState, sound: bool) {
    let owner = context.owner.actor.id().clone();
    if sound && context.now >= state.empty_sound_time {
        game.sound(
            &owner,
            "weapons/noammo.wav",
            if context.rerelease { 1 } else { 2 },
            1.0,
            1.0,
        );
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
fn fallback_candidate(context: &Q2WeaponContext, game: &mut Q2GameServices, name: &str) -> Option<Q2WeaponName> {
    let definition = require_definition(game, Some(&name.to_string()));
    let owner = context.owner.actor.id();
    if name != "blaster" && game.host.inventory().count(owner, &definition.item) == 0.0 {
        return None;
    }
    if definition
        .ammo
        .as_ref()
        .is_some_and(|ammo| game.host.inventory().count(owner, ammo) < f64::from(definition.quantity))
    {
        return None;
    }
    Some(name.to_string())
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
fn set_loop(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState, path: &str) {
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
fn lag_begin(game: &mut Q2GameServices, owner: &ActorId, start: Vec3, direction: Vec3) -> Option<LagToken> {
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
        if context.rerelease && !hyper { 1500.0 } else { 1000.0 },
        effects,
        hyper,
        if hyper { Mod::HYPERBLASTER } else { Mod::BLASTER },
    );
    muzzle_flash(context, game, if hyper { 14 } else { 0 });
    player_noise(game, &owner, start, NoiseKind::Weapon);
}

/// Fire the hyperblaster (`hyperblaster`).
fn fire_hyperblaster(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    let owner = context.owner.actor.id().clone();
    let damage = if game.options.mode == Q2Mode::Deathmatch {
        15.0
    } else {
        20.0
    };
    if context.rerelease {
        state.frame = if state.frame > 20 { 6 } else { state.frame + 1 };
        if state.frame == 12 {
            if read_ammo(game, &owner, &context.definition) > 0.0 && continues_attack(context, state) {
                state.frame = 6;
            } else {
                game.sound(&owner, "weapons/hyprbd1a.wav", 0, 1.0, 1.0);
            }
        }
        let spinning = (6..=11).contains(&state.frame);
        set_loop(context, game, state, if spinning { "weapons/hyprbl1a.wav" } else { "" });
        if continues_attack(context, state) && spinning {
            if read_ammo(game, &owner, &context.definition) < 1.0 {
                no_ammo(context, game, state, true);
                return;
            }
            let rotation = f64::from(state.frame - 5) * 2.0 * std::f64::consts::PI / 6.0;
            let offset = vec3((-4.0 * rotation.sin()) as f32, (4.0 * rotation.cos()) as f32, 0.0);
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
            let rotation = f64::from(state.frame - 5) * 2.0 * std::f64::consts::PI / 6.0;
            let offset = vec3((-4.0 * rotation.sin()) as f32, 0.0, (4.0 * rotation.cos()) as f32);
            fire_blaster_weapon(
                context,
                game,
                state,
                offset,
                damage,
                true,
                if state.frame == 6 || state.frame == 9 { 64 } else { 0 },
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
fn fire_machinegun(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
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
    set_q2_weapon_recoil(state, game.options.edition, context.now, kick_origin, kick_angles, None);
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
    let (start, direction) = project_weapon(&context.owner, game, &context.input, angles, offset);
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
fn fire_chaingun(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
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
        if state.frame == 21 && continues_attack(context, state) && read_ammo(game, &owner, &context.definition) > 0.0 {
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
    set_q2_weapon_recoil(state, game.options.edition, context.now, kick_origin, kick_angles, None);
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
    let (start, direction) = project_weapon(&context.owner, game, &context.input, context.input.angles, offset);
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
                project_weapon(&context.owner, game, &context.input, angles, vec3(0.0, 0.0, -8.0))
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
fn fire_grenade_launcher(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
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
    let (start, direction) = project_weapon(&context.owner, game, &context.input, angles, offset);
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
        let right = (-0.9999999403953552 + game.random() * 1.9999999403953552) * 10.0;
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
fn fire_rocket_launcher(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
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
fn fire_railgun(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
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
fn fire_bfg_weapon(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
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
fn throw_grenade_launch(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState, held: bool) {
    if context.definition.name == "grenades" && state.hand_reservation == Q2HandReservation::None {
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
        let mut project = |angles: Vec3, offset: Vec3| project_weapon(&throw_owner, game, &throw_input, angles, offset);
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

/// Weapon selection (`Q2WeaponSelection`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2WeaponSelection {
    /// Selected.
    Selected,
    /// Already current.
    Current,
    /// Not owned.
    NotOwned,
    /// No ammo.
    NoAmmo,
    /// Not enough ammo.
    NotEnoughAmmo,
}

/// Look up a weapon definition (`definition`).
pub fn weapon_definition(game: &Q2GameServices, name: &str) -> Q2WeaponDefinition {
    game.weapons
        .definitions
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("Q2 weapon is not registered: {name}"))
}

/// Look up a weapon definition by classname (`definitionFromClassname`).
pub fn definition_from_classname(game: &Q2GameServices, classname: &str) -> Option<Q2WeaponDefinition> {
    game.weapons
        .definitions
        .values()
        .find(|definition| definition.classname == classname)
        .cloned()
}

/// Registered weapon definitions (`registeredDefinitions`), sorted by name.
pub fn registered_weapon_definitions(game: &Q2GameServices) -> Vec<Q2WeaponDefinition> {
    let mut definitions: Vec<Q2WeaponDefinition> = game.weapons.definitions.values().cloned().collect();
    definitions.sort_by(|left, right| left.name.cmp(&right.name));
    definitions
}

/// Bind weapon state to an actor (`bind`).
pub fn bind_player_weapon(game: &mut Q2GameServices, owner: &OwnedActor, state: Q2WeaponState) {
    game.host.actors().assert_owned(owner);
    if game.weapons.states.contains_key(owner.id()) {
        panic!("Q2 weapon state already bound to actor");
    }
    super::ballistics::reset_silencer(game, owner.id());
    game.weapons.states.insert(owner.id().clone(), state);
}

/// Build an owner handle with live view height.
fn weapon_owner(game: &Q2GameServices, owner: &OwnedActor) -> Q2WeaponOwner {
    Q2WeaponOwner {
        actor: owner.clone(),
        view_height: f64::from(game.require_entity(owner.id()).view_height),
    }
}

/// Build a weapon context (`context`).
fn weapon_context(
    game: &mut Q2GameServices,
    owner: &Q2WeaponOwner,
    state: &Q2WeaponState,
    input: &Q2WeaponInput,
    silenced: bool,
) -> Option<Q2WeaponContext> {
    let weapon = state.weapon.as_ref()?;
    let definition = game.weapons.definitions.get(weapon)?.clone();
    Some(Q2WeaponContext {
        owner: owner.clone(),
        input: input.clone(),
        definition,
        now: game.host.now(),
        rerelease: game.options.edition == Q2Edition::Rerelease,
        silenced,
    })
}

/// Request a weapon (`requestWeapon`).
pub fn request_weapon(
    game: &mut Q2GameServices,
    owner: &OwnedActor,
    name: &str,
    allow_empty: bool,
) -> Q2WeaponSelection {
    let snapshot = game
        .weapons
        .states
        .get(owner.id())
        .cloned()
        .expect("Q2 player weapon state is not bound");
    let mut name = name.to_string();
    let rules: Vec<(String, Q2WeaponSelectionRule)> = game
        .weapons
        .extensions
        .iter()
        .filter_map(|(candidate, extension)| extension.selection().map(|rule| (candidate.clone(), rule)))
        .collect();
    let holder = weapon_owner(game, owner);
    for (candidate, rule) in &rules {
        if rule.requested == name && (rule.choose)(&holder, game, &snapshot) {
            name.clone_from(candidate);
            break;
        }
    }
    let definition = weapon_definition(game, &name);
    if snapshot.weapon.as_deref() == Some(name.as_str()) {
        return Q2WeaponSelection::Current;
    }
    if game.host.inventory().count(owner.id(), &definition.item) < 1.0 {
        return Q2WeaponSelection::NotOwned;
    }
    if !allow_empty && definition.ammo.is_some() && definition.ammo.as_deref() != Some(definition.item.as_str()) {
        let ammo = definition.ammo.clone().expect("weapon ammo");
        let count = game.host.inventory().count(owner.id(), &ammo);
        if count == 0.0 {
            return Q2WeaponSelection::NoAmmo;
        }
        if count < f64::from(definition.quantity) {
            return Q2WeaponSelection::NotEnoughAmmo;
        }
    }
    game.weapons.states.get_mut(owner.id()).expect("weapon state").pending = Some(name);
    Q2WeaponSelection::Selected
}

/// Request a holster (`requestHolster`).
pub fn request_holster(game: &mut Q2GameServices, owner: &OwnedActor) {
    let state = game
        .weapons
        .states
        .get_mut(owner.id())
        .expect("Q2 player weapon state is not bound");
    if state.primary_handoff != PrimaryHandoff::Active {
        return;
    }
    state.primary_handoff = if state.weapon.is_none() {
        PrimaryHandoff::Holstered
    } else {
        PrimaryHandoff::Holstering
    };
    state.pending = None;
}

/// Whether the weapon is holstered (`isHolstered`).
pub fn is_holstered(game: &Q2GameServices, owner: &ActorId) -> bool {
    game.weapons
        .states
        .get(owner)
        .expect("Q2 player weapon state is not bound")
        .primary_handoff
        == PrimaryHandoff::Holstered
}

/// Resume the primary weapon (`resumePrimary`).
pub fn resume_primary(game: &mut Q2GameServices, owner: &OwnedActor, input: &Q2WeaponInput, name: Option<&str>) {
    if !game.weapons.states.contains_key(owner.id()) {
        panic!("Q2 player weapon state is not bound");
    }
    with_weapon_state(game, owner.id(), |game, state| {
        resume_primary_inner(game, owner, input, name, state);
    });
}

/// Resume the primary weapon with live state (`resumePrimary`).
fn resume_primary_inner(
    game: &mut Q2GameServices,
    owner: &OwnedActor,
    input: &Q2WeaponInput,
    name: Option<&str>,
    state: &mut Q2WeaponState,
) {
    if state.primary_handoff == PrimaryHandoff::Active {
        return;
    }
    if state.primary_handoff != PrimaryHandoff::Holstered {
        panic!("Q2 primary must finish holstering before it resumes");
    }
    let holder = weapon_owner(game, owner);
    let requested = name.map(str::to_string).or_else(|| state.weapon.clone());
    let definition = requested
        .as_ref()
        .and_then(|requested| game.weapons.definitions.get(requested).cloned());
    let alive = game
        .host
        .combat()
        .read(owner.id())
        .map(|combat| combat.health)
        .unwrap_or(0.0)
        > 0.0;
    state.pending = None;
    if alive {
        let ready = match definition {
            Some(definition)
                if (definition.name == "blaster"
                    || game.host.inventory().count(owner.id(), &definition.item) > 0.0)
                    && definition.ammo.as_ref().is_none_or(|ammo| {
                        game.host.inventory().count(owner.id(), ammo) >= f64::from(definition.quantity)
                    }) =>
            {
                state.pending = Some(definition.name);
                true
            }
            _ => false,
        };
        if !ready {
            match weapon_context(
                game,
                &holder,
                state,
                input,
                super::ballistics::silencer_shots(game, owner.id()) > 0,
            ) {
                Some(context) => no_ammo(&context, game, state, false),
                None => state.pending = Some("blaster".to_string()),
            }
        }
    }
    change_weapon(&holder, game, state, input);
}

/// Whether a weapon can be dropped (`canDrop`).
pub fn can_drop_weapon(game: &mut Q2GameServices, owner: &OwnedActor, name: &str) -> bool {
    if game.deathmatch_flags() & 4 != 0 {
        return false;
    }
    let definition = weapon_definition(game, name);
    let state = game
        .weapons
        .states
        .get(owner.id())
        .expect("Q2 player weapon state is not bound");
    let count = game.host.inventory().count(owner.id(), &definition.item);
    count > 0.0 && !((state.weapon.as_deref() == Some(name) || state.pending.as_deref() == Some(name)) && count == 1.0)
}

/// Run the weapon turn (`tick`).
pub fn tick_player_weapon(game: &mut Q2GameServices, owner: &OwnedActor, input: Q2WeaponInput) {
    game.weapons.inputs.insert(owner.id().clone(), input.clone());
    if !game.weapons.states.contains_key(owner.id()) {
        panic!("Q2 player weapon state is not bound");
    }
    with_weapon_state(game, owner.id(), |game, state| {
        tick_inner(game, owner, &input, state);
    });
}

/// Run the weapon turn with live state (`tick`).
fn tick_inner(game: &mut Q2GameServices, owner: &OwnedActor, input: &Q2WeaponInput, state: &mut Q2WeaponState) {
    let now = game.host.now();
    state.latched_attack |= input.latched_attack;
    if input.spectator {
        return;
    }
    let holder = weapon_owner(game, owner);
    if game
        .host
        .combat()
        .read(owner.id())
        .map(|combat| combat.health)
        .unwrap_or(0.0)
        < 1.0
    {
        if state.grenade_time != 0.0
            && (if state.weapon.as_deref() == Some("grenades") {
                state.hand_reservation != Q2HandReservation::None
            } else {
                game.options.edition == Q2Edition::Rerelease
            })
        {
            let silenced = super::ballistics::silencer_shots(game, owner.id()) > 0;
            if let Some(context) = weapon_context(game, &holder, state, input, silenced) {
                if !context.rerelease {
                    state.grenade_time = now;
                }
                let held = context.rerelease;
                fire_held(&context, game, state, held);
            }
        }
        state.pending = None;
        change_weapon(&holder, game, state, input);
        present(&holder, game, state);
        return;
    }
    if state.primary_handoff == PrimaryHandoff::Holstered {
        present(&holder, game, state);
        return;
    }
    if state.weapon.is_none() {
        if state.pending.is_some() {
            change_weapon(&holder, game, state, input);
        }
        present(&holder, game, state);
        return;
    }
    let classic_silenced = super::ballistics::silencer_shots(game, owner.id()) > 0;
    run_weapon_think(game, &holder, state, input, classic_silenced);
    if game.weapons.source_rules == Some(super::WeaponSourceRules::Lmctf) {
        let silenced = super::ballistics::silencer_shots(game, owner.id()) > 0;
        if let Some(context) = weapon_context(game, &holder, state, input, silenced) {
            let post = game
                .weapons
                .match_hooks
                .lmctf
                .expect("lmctf weapon hooks")
                .post_native_think;
            if post(&context, game) {
                run_weapon_think(game, &holder, state, input, classic_silenced);
            }
        }
    }
    if game.options.edition == Q2Edition::Rerelease && game.host.frame_seconds() > 0.033 {
        let silenced = super::ballistics::silencer_shots(game, owner.id()) > 0;
        if let Some(context) = weapon_context(game, &holder, state, input, silenced) {
            let interval = animation_time(&context, game, state);
            if interval < game.host.frame_seconds() {
                let mut remaining = (millisecond_sum(now, game.host.frame_seconds()) * 1000.0).round() as i64
                    - (state.think_time * 1000.0).round() as i64;
                while remaining > 0 {
                    state.think_time = millisecond_sum(state.think_time, -interval);
                    state.fire_finished = millisecond_sum(state.fire_finished, -interval);
                    run_weapon_think(game, &holder, state, input, classic_silenced);
                    remaining -= (interval * 1000.0).round() as i64;
                }
            }
        }
    } else if game.options.edition == Q2Edition::Classic && input.quad_fire_until > now {
        run_weapon_think(game, &holder, state, input, classic_silenced);
    }
    present(&holder, game, state);
}

/// Run one weapon think (`tick` run closure).
fn run_weapon_think(
    game: &mut Q2GameServices,
    owner: &Q2WeaponOwner,
    state: &mut Q2WeaponState,
    input: &Q2WeaponInput,
    classic_silenced: bool,
) {
    let silenced = if game.options.edition == Q2Edition::Classic {
        classic_silenced
    } else {
        super::ballistics::silencer_shots(game, owner.actor.id()) > 0
    };
    let Some(context) = weapon_context(game, owner, state, input, silenced) else {
        return;
    };
    if state.primary_handoff == PrimaryHandoff::Holstered {
        return;
    }
    if extension_think(game, &context.definition.name.clone(), &context, state) {
        return;
    }
    if state.weapon.as_deref() == Some("grenades") {
        if context.rerelease {
            throw_rerelease(&context, game, state, None);
        } else {
            throw_classic(&context, game, state, None);
        }
        return;
    }
    if context.rerelease {
        generic_rerelease(&context, game, state);
    } else {
        generic_classic(&context, game, state);
    }
}

/// Run an extension think hook, reporting whether it handled the frame.
fn extension_think(
    game: &mut Q2GameServices,
    name: &str,
    context: &Q2WeaponContext,
    state: &mut Q2WeaponState,
) -> bool {
    if !game.weapons.extensions.contains_key(name) {
        return false;
    }
    let mut extension = game.weapons.extensions.remove(name).expect("weapon extension");
    let handled = extension.think(context, game, state);
    game.weapons.extensions.insert(name.to_string(), extension);
    handled
}

/// Whether an extension implements the held hook.
fn extension_has_held(game: &Q2GameServices, name: &str) -> bool {
    game.weapons
        .extensions
        .get(name)
        .is_some_and(|extension| extension.has_held())
}

/// Fire a held throw (`fireHeld`).
fn fire_held(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState, held: bool) {
    let name = context.definition.name.clone();
    if game.weapons.extensions.contains_key(&name) {
        let mut extension = game.weapons.extensions.remove(&name).expect("weapon extension");
        let handled = extension.held(context, game, state, held);
        game.weapons.extensions.insert(name, extension);
        if handled {
            return;
        }
    }
    throw_grenade_launch(context, game, state, held);
}

/// Change the weapon (`changeWeapon`).
fn change_weapon(owner: &Q2WeaponOwner, game: &mut Q2GameServices, state: &mut Q2WeaponState, input: &Q2WeaponInput) {
    let rerelease = game.options.edition == Q2Edition::Rerelease;
    let actor = owner.actor.id().clone();
    let health = game
        .host
        .combat()
        .read(&actor)
        .map(|combat| combat.health)
        .unwrap_or(0.0);
    if state.primary_handoff == PrimaryHandoff::Active
        && rerelease
        && health > 0.0
        && !input.instant_switch
        && input.holster
    {
        return;
    }
    if state.grenade_time != 0.0 {
        let flush = if state.weapon.as_deref() == Some("grenades") {
            state.hand_reservation != Q2HandReservation::None
        } else {
            rerelease
                || state.primary_handoff == PrimaryHandoff::Holstering
                    && state
                        .weapon
                        .as_ref()
                        .is_some_and(|weapon| extension_has_held(game, weapon))
        };
        if flush {
            let silenced = super::ballistics::silencer_shots(game, &actor) > 0;
            if let Some(context) = weapon_context(game, owner, state, input, silenced) {
                if !context.rerelease {
                    state.grenade_time = context.now;
                }
                fire_held(&context, game, state, false);
            }
        }
    }
    cancel_hand_preparation(owner, game, state);
    state.grenade_time = 0.0;
    if state.primary_handoff == PrimaryHandoff::Holstering && health > 0.0 {
        state.primary_handoff = PrimaryHandoff::Holstered;
        state.pending = None;
        state.latched_attack = false;
        state.fire_buffered = false;
        let shell = shell_context(game, owner, state, input);
        set_loop(&shell, game, state, "");
        return;
    }
    state.primary_handoff = PrimaryHandoff::Active;
    if state.weapon.is_some() && state.pending.is_some() && state.pending != state.weapon && rerelease {
        game.sound(&actor, "weapons/change.wav", 1, 1.0, 1.0);
    }
    state.last_weapon.clone_from(&state.weapon);
    state.weapon.clone_from(&state.pending);
    state.pending = None;
    state.machinegun_shots = 0;
    state.view_model = None;
    state.view_skin = 0;
    let shell = shell_context(game, owner, state, input);
    set_loop(&shell, game, state, "");
    if state.weapon.is_none() {
        return;
    }
    state.phase = Q2WeaponPhase::Activating;
    state.frame = 0;
    let silenced = super::ballistics::silencer_shots(game, &actor) > 0;
    let Some(context) = weapon_context(game, owner, state, input, silenced) else {
        return;
    };
    let (first, last) = if input.ducked { (169, 172) } else { (62, 65) };
    animate_player(&context, game, PlayerAnimationPriority::Pain, first, last);
    if rerelease && input.instant_switch {
        if extension_think(game, &context.definition.name.clone(), &context, state) {
            return;
        }
        if state.weapon.as_deref() == Some("grenades") {
            throw_rerelease(&context, game, state, None);
        } else {
            generic_rerelease(&context, game, state);
        }
    }
}

/// Build a shell context for sound/animation helpers (`changeWeapon` helper).
fn shell_context(
    game: &mut Q2GameServices,
    owner: &Q2WeaponOwner,
    state: &Q2WeaponState,
    input: &Q2WeaponInput,
) -> Q2WeaponContext {
    let definition = state
        .weapon
        .as_ref()
        .and_then(|weapon| game.weapons.definitions.get(weapon))
        .or_else(|| {
            state
                .pending
                .as_ref()
                .and_then(|pending| game.weapons.definitions.get(pending))
        })
        .or_else(|| game.weapons.definitions.get("blaster"))
        .expect("weapon definition")
        .clone();
    Q2WeaponContext {
        owner: owner.clone(),
        input: input.clone(),
        definition,
        now: game.host.now(),
        rerelease: game.options.edition == Q2Edition::Rerelease,
        silenced: false,
    }
}

/// Reserve a hand grenade (`reserveHandGrenade`).
fn reserve_hand_grenade(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) -> bool {
    if state.hand_reservation != Q2HandReservation::None {
        return false;
    }
    let owner = context.owner.actor.id().clone();
    if context.rerelease {
        if context.input.infinite_ammo {
            state.hand_reservation = Q2HandReservation::Infinite;
            return true;
        }
    } else if game.deathmatch_flags() & 8192 != 0 {
        state.hand_reservation = Q2HandReservation::Infinite;
        return true;
    }
    let before = read_ammo(game, &owner, &context.definition);
    let owned = game.owned_of(owner.clone());
    if !game
        .host
        .inventory()
        .consume(&owned, &"q2:ammo_grenades".to_string(), 1.0)
    {
        return false;
    }
    state.hand_reservation = Q2HandReservation::Finite;
    if context.rerelease
        && before > f64::from(context.definition.warning)
        && read_ammo(game, &owner, &context.definition) <= f64::from(context.definition.warning)
    {
        game.sound(&owner, "weapons/lowammo.wav", 0, 1.0, 1.0);
    }
    ammo_changed(game, &owner, &"q2:ammo_grenades".to_string());
    true
}

/// Cancel hand preparation (`cancelHandPreparation`).
fn cancel_hand_preparation(owner: &Q2WeaponOwner, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    let reservation = std::mem::replace(&mut state.hand_reservation, Q2HandReservation::None);
    if reservation == Q2HandReservation::Finite
        && state.grenade_time == 0.0
        && game.host.actors().is_live(owner.actor.id())
    {
        let entry = game
            .host
            .inventory()
            .entries(owner.actor.id())
            .into_iter()
            .find(|entry| entry.item == "q2:ammo_grenades")
            .expect("Reserved hand grenade lost its canonical inventory entry");
        let count = entry.count + 1.0;
        game.host
            .inventory()
            .configure(&owner.actor, &InventoryEntry { count, ..entry });
        ammo_changed(game, owner.actor.id(), &"q2:ammo_grenades".to_string());
    }
}

/// Run the classic generic think (`genericClassic`).
fn generic_classic(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    let phase = state.phase;
    generic_classic_frame(context, game, state);
    if game.weapons.source_rules != Some(super::WeaponSourceRules::Ctf) {
        return;
    }
    let grapple = state.weapon.as_deref() == Some("grapple");
    if grapple && state.phase == Q2WeaponPhase::Firing {
        return;
    }
    let ctf = game.weapons.match_hooks.ctf.expect("ctf weapon hooks");
    if ((ctf.haste)(context, game) || grapple) && phase == state.phase {
        generic_classic_frame(context, game, state);
    }
}

/// Run one classic generic frame (`genericClassicFrame`).
fn generic_classic_frame(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    if state.primary_handoff == PrimaryHandoff::Holstered {
        return;
    }
    if game.weapons.source_rules == Some(super::WeaponSourceRules::Lmctf) {
        state.source_firing = false;
    }
    let definition = Q2GenericDefinition::from(&context.definition);
    let input = Q2ClassicFrameInput {
        attack: context.input.attack,
        change_requested: state.pending.is_some() || state.primary_handoff == PrimaryHandoff::Holstering,
    };
    let mut hooks = FrameHooks {
        game,
        state,
        context: context.clone(),
    };
    step_q2_classic_frame(&definition, &input, &mut hooks);
}

/// Run the rerelease generic think (`genericRerelease`).
fn generic_rerelease(context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
    if state.primary_handoff == PrimaryHandoff::Holstered {
        return;
    }
    let definition = Q2GenericDefinition::from(&context.definition);
    let input = Q2RereleaseFrameInput {
        attack: context.input.attack,
        change_requested: state.pending.is_some() || state.primary_handoff == PrimaryHandoff::Holstering,
        now: context.now,
        frame_seconds: game.host.frame_seconds(),
        instant_switch: context.input.instant_switch,
        holster: context.input.holster,
        weapon_thunk: context.input.weapon_thunk,
    };
    let mut hooks = FrameHooks {
        game,
        state,
        context: context.clone(),
    };
    step_q2_rerelease_frame(&definition, &input, &mut hooks);
}

/// Run the classic throw think (`throwClassic`).
fn throw_classic(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    throwing: Option<&Q2ThrowDefinition>,
) {
    let owner = context.owner.clone();
    let input = context.input.clone();
    let definition = context.definition.clone();
    let idle_first = definition.fire_last + 1;
    if state.primary_handoff == PrimaryHandoff::Holstering {
        change_weapon(&owner, game, state, &input);
        return;
    }
    if state.pending.is_some() && state.phase == Q2WeaponPhase::Ready {
        change_weapon(&owner, game, state, &input);
        return;
    }
    if state.phase == Q2WeaponPhase::Activating {
        state.phase = Q2WeaponPhase::Ready;
        state.frame = idle_first;
        return;
    }
    if state.phase == Q2WeaponPhase::Ready {
        if state.latched_attack || input.attack {
            state.latched_attack = false;
            if throwing.is_none() {
                if reserve_hand_grenade(context, game, state) {
                    state.frame = 1;
                    state.phase = Q2WeaponPhase::Firing;
                    state.grenade_time = 0.0;
                } else {
                    no_ammo(context, game, state, true);
                }
            } else if read_ammo(game, owner.actor.id(), &definition) > 0.0 {
                state.frame = 1;
                state.phase = Q2WeaponPhase::Firing;
                state.grenade_time = 0.0;
            } else {
                no_ammo(context, game, state, true);
            }
            return;
        }
        if throwing.is_some_and(|throwing| throwing.wrap_before_pause) && state.frame == definition.idle_last {
            state.frame = idle_first;
            return;
        }
        if definition.pauses.contains(&state.frame) && (game.random() * 16.0).floor() as i64 != 0 {
            return;
        }
        state.frame += 1;
        if state.frame > definition.idle_last {
            state.frame = idle_first;
        }
        return;
    }
    if state.phase != Q2WeaponPhase::Firing {
        return;
    }
    let actor = owner.actor.id().clone();
    if state.frame == throwing.map(|throwing| throwing.sound_frame).unwrap_or(5) {
        let sound = throwing
            .map(|throwing| throwing.cock_sound.clone())
            .unwrap_or_else(|| "weapons/hgrena1b.wav".to_string());
        game.sound(&actor, &sound, 1, 1.0, 1.0);
    }
    if state.frame == throwing.map(|throwing| throwing.hold_frame).unwrap_or(11) {
        if state.grenade_time == 0.0 {
            state.grenade_time = hand_fuse_deadline(context.now, Q2Edition::Classic);
            let sound = throwing
                .map(|throwing| throwing.hold_sound.clone())
                .unwrap_or_else(|| "weapons/hgrenc1b.wav".to_string());
            set_loop(context, game, state, &sound);
        }
        if throwing.is_none_or(|throwing| throwing.explode)
            && !state.grenade_blew_up
            && context.now >= state.grenade_time
        {
            set_loop(context, game, state, "");
            match throwing {
                None => throw_grenade_launch(context, game, state, true),
                Some(throwing) => (throwing.fire)(context, game, state, true),
            }
            if throwing.is_none()
                && (state.weapon.as_deref() != Some(definition.name.as_str()) || !game.host.actors().is_live(&actor))
            {
                return;
            }
            state.grenade_blew_up = true;
        }
        if input.attack {
            return;
        }
        if state.grenade_blew_up {
            if context.now >= state.grenade_time {
                state.frame = definition.fire_last;
                state.grenade_blew_up = false;
            } else {
                return;
            }
        }
    }
    if state.frame == throwing.map(|throwing| throwing.fire_frame).unwrap_or(12) {
        set_loop(context, game, state, "");
        match throwing {
            None => throw_grenade_launch(context, game, state, false),
            Some(throwing) => (throwing.fire)(context, game, state, throwing.release_held),
        }
        if throwing.is_none()
            && (state.weapon.as_deref() != Some(definition.name.as_str()) || !game.host.actors().is_live(&actor))
        {
            return;
        }
    }
    if state.frame == definition.fire_last && context.now < state.grenade_time {
        return;
    }
    state.frame += 1;
    if state.frame == idle_first {
        state.grenade_time = 0.0;
        state.phase = Q2WeaponPhase::Ready;
    }
}

/// Run the rerelease throw think (`throwRerelease`).
fn throw_rerelease(
    context: &Q2WeaponContext,
    game: &mut Q2GameServices,
    state: &mut Q2WeaponState,
    throwing: Option<&Q2ThrowDefinition>,
) {
    let owner = context.owner.clone();
    let input = context.input.clone();
    let definition = context.definition.clone();
    let fire_last = definition.fire_last;
    let idle_first = fire_last + 1;
    let idle_last = definition.idle_last;
    let idle_ready = if throwing.is_none() { idle_last + 1 } else { idle_first };
    let sound_frame = throwing.map(|throwing| throwing.sound_frame).unwrap_or(5);
    let hold_frame = throwing.map(|throwing| throwing.hold_frame).unwrap_or(11);
    let cock_sound = throwing
        .map(|throwing| throwing.cock_sound.as_str())
        .unwrap_or("weapons/hgrena1b.wav");
    let hold_sound = throwing
        .map(|throwing| throwing.hold_sound.as_str())
        .unwrap_or("weapons/hgrenc1b.wav");
    let explodes = throwing.is_none_or(|throwing| throwing.explode);
    if state.primary_handoff == PrimaryHandoff::Holstering {
        if state.think_time <= context.now || input.instant_switch {
            change_weapon(&owner, game, state, &input);
        }
        return;
    }
    if state.pending.is_some() && state.phase == Q2WeaponPhase::Ready {
        if state.think_time <= context.now {
            change_weapon(&owner, game, state, &input);
            let interval = animation_time(context, game, state);
            state.think_time = millisecond_sum(context.now, interval);
        }
        return;
    }
    if state.phase == Q2WeaponPhase::Activating {
        if state.think_time <= context.now {
            state.phase = Q2WeaponPhase::Ready;
            state.frame = idle_ready;
            let interval = animation_time(context, game, state);
            state.think_time = millisecond_sum(context.now, interval);
            state.fire_finished = millisecond_sum(context.now, interval);
        }
        return;
    }
    if state.phase == Q2WeaponPhase::Ready {
        if (state.fire_buffered || state.latched_attack || input.attack) && state.fire_finished <= context.now {
            state.latched_attack = false;
            let primed = if throwing.is_none() {
                reserve_hand_grenade(context, game, state)
            } else {
                read_ammo(game, owner.actor.id(), &definition) > 0.0
            };
            if primed {
                state.frame = if throwing.is_none() { 2 } else { 1 };
                state.phase = Q2WeaponPhase::Firing;
                state.grenade_time = 0.0;
                let interval = animation_time(context, game, state);
                state.think_time = millisecond_sum(context.now, interval);
            } else {
                no_ammo(context, game, state, true);
            }
        } else if state.think_time <= context.now {
            let interval = animation_time(context, game, state);
            state.think_time = millisecond_sum(context.now, interval);
            if state.frame >= idle_last {
                state.frame = idle_first;
            } else if !definition.pauses.contains(&state.frame) || (game.random() * 16.0).floor() as i64 == 0 {
                state.frame += 1;
            }
        }
        return;
    }
    if state.phase != Q2WeaponPhase::Firing {
        return;
    }
    state.last_firing_time = millisecond_sum(context.now, 2.5);
    if state.think_time > context.now {
        return;
    }
    if state.frame == sound_frame && !cock_sound.is_empty() {
        game.sound(owner.actor.id(), cock_sound, 1, 1.0, 1.0);
    }
    let wait = firing_interval(
        game,
        owner.actor.id(),
        hand_recovery_seconds(&HandGrenadeTempo {
            edition: Q2Edition::Rerelease,
            haste: input.haste,
            quad_fire: input.quad_fire_until > context.now,
        }),
    );
    if state.frame == hold_frame {
        if state.grenade_time == 0.0 && state.grenade_finished == 0.0 {
            state.grenade_time = hand_fuse_deadline(context.now, Q2Edition::Rerelease);
        }
        if !state.grenade_blew_up && !hold_sound.is_empty() {
            set_loop(context, game, state, hold_sound);
        }
        if explodes && !state.grenade_blew_up && context.now >= state.grenade_time {
            powerup_sound(context, game);
            set_loop(context, game, state, "");
            match throwing {
                None => throw_grenade_launch(context, game, state, true),
                Some(throwing) => (throwing.fire)(context, game, state, true),
            }
            let actor = owner.actor.id().clone();
            if throwing.is_none()
                && (state.weapon.as_deref() != Some(definition.name.as_str()) || !game.host.actors().is_live(&actor))
            {
                return;
            }
            state.grenade_blew_up = true;
            state.grenade_finished = millisecond_sum(context.now, wait);
        }
        if input.attack {
            state.think_time = millisecond_sum(context.now, 0.001);
            return;
        }
        if state.grenade_blew_up {
            if context.now >= state.grenade_finished {
                state.frame = fire_last;
                state.grenade_blew_up = false;
                let interval = animation_time(context, game, state);
                state.think_time = millisecond_sum(context.now, interval);
            } else {
                return;
            }
        } else {
            state.frame += 1;
            powerup_sound(context, game);
            set_loop(context, game, state, "");
            match throwing {
                None => throw_grenade_launch(context, game, state, false),
                Some(throwing) => (throwing.fire)(context, game, state, false),
            }
            let actor = owner.actor.id().clone();
            if throwing.is_none()
                && (state.weapon.as_deref() != Some(definition.name.as_str()) || !game.host.actors().is_live(&actor))
            {
                return;
            }
            state.grenade_finished = millisecond_sum(context.now, wait);
            let (first, last) = if input.ducked { (159, 162) } else { (119, 112) };
            // The donor passes (119, 112) for standing, preserving the reversed range.
            animate_player(
                context,
                game,
                if input.ducked {
                    PlayerAnimationPriority::Attack
                } else {
                    PlayerAnimationPriority::Reverse
                },
                first,
                last,
            );
        }
    }
    let interval = animation_time(context, game, state);
    state.think_time = millisecond_sum(context.now, interval);
    if state.frame == fire_last && context.now < state.grenade_finished {
        return;
    }
    state.frame += 1;
    if state.frame == idle_first {
        state.grenade_finished = 0.0;
        state.phase = Q2WeaponPhase::Ready;
        state.fire_buffered = false;
        let interval = animation_time(context, game, state);
        state.fire_finished = millisecond_sum(context.now, interval);
        state.frame = idle_ready;
        if read_ammo(game, owner.actor.id(), &definition) == 0.0 {
            no_ammo(context, game, state, false);
            change_weapon(&owner, game, state, &input);
        }
    }
}
