//! Q2 base mover entities (`src/content/q2/base/entities/movers.ts`).
//!
//! Quake II g_func.c platforms, secret doors and supporting brush entities.

use std::collections::BTreeMap;

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{add3, dot3, scale3, sub3, vec3, Vec3};

use crate::contract::{ArmorState, PoweredProtectionState, RegularArmorState};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::checkpoint::restore_q2_actor;
use crate::q2::foundation::fields::{integer_field, number_field};
use crate::q2::foundation::host::{
    Q2Die, Q2EffectEvent, Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop, Q2Use,
};
use crate::q2::foundation::motion::{
    base_linear_motion_callbacks, capture_linear_motion, linear_move_destination, linear_move_to,
    restore_linear_motion, LinearMotionScope, Q2LinearMotionCheckpoint,
};
use crate::q2::foundation::scenery::kill_q2_box;
use crate::q2::foundation::weapons::vectors::angle_vectors;
use crate::q2::support::contracts::{CombatState, DeathReaction, TouchContact};

use super::types::Q2BaseEntityHooks;

/// Platform phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2PlatformPhase {
    /// At the top.
    Top,
    /// At the bottom.
    Bottom,
    /// Moving up.
    Up,
    /// Moving down.
    Down,
}

/// Platform state (`Q2PlatformState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2PlatformState {
    /// Top position.
    pub top: Vec3,
    /// Bottom position.
    pub bottom: Vec3,
    /// Phase.
    pub phase: Q2PlatformPhase,
}

/// Secret door state (`SecretState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2SecretState {
    /// First slide position.
    pub first: Vec3,
    /// Second slide position.
    pub second: Vec3,
    /// Home position.
    pub home: Vec3,
    /// Whether shootable.
    pub shootable: bool,
    /// Blocked-message throttle.
    pub blocked_time: f64,
    /// Talk-message throttle.
    pub message_time: f64,
}

/// Platform checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlatformEntry {
    /// Platform actor.
    pub actor: SavedActorId,
    /// Platform state.
    pub state: Q2PlatformState,
}

/// Secret checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SecretEntry {
    /// Secret actor.
    pub actor: SavedActorId,
    /// Secret state.
    pub state: Q2SecretState,
}

/// Base movers checkpoint (`Q2BaseMoversCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2BaseMoversCheckpoint {
    /// Platforms.
    pub platforms: Vec<Q2PlatformEntry>,
    /// Secrets.
    pub secrets: Vec<Q2SecretEntry>,
    /// Linear moves.
    pub linear: Q2LinearMotionCheckpoint,
}

/// Platform traversal (`traversal` result).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2PlatformTraversal {
    /// Whether locked.
    pub locked: bool,
    /// Linear destination.
    pub destination: Option<Vec3>,
}

/// Read registered hooks.
fn hooks(game: &Q2GameServices) -> Q2BaseEntityHooks {
    game.base_entities
        .hooks
        .expect("Q2 base entity hooks are not registered")
}

/// Require platform state.
fn platform_state(game: &Q2GameServices, actor: &ActorId) -> Q2PlatformState {
    game.base_entities
        .platforms
        .get(actor)
        .copied()
        .expect("Platform callback without platform state")
}

/// Require secret state.
fn secret_state(game: &Q2GameServices, actor: &ActorId) -> Q2SecretState {
    game.base_entities
        .secrets
        .get(actor)
        .copied()
        .expect("Secret door callback without source state")
}

/// Emit a looping platform sound (`loop`).
fn emit_loop(actor: ActorId, game: &mut Q2GameServices, path: &str, start: bool) {
    let origin = game.body_of(actor.clone()).origin;
    game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(actor),
        origin,
        path: path.to_string(),
        channel: 2,
        volume: 1.0,
        attenuation: 3.0,
        reliable: false,
        loop_: if start { Q2SoundLoop::Start } else { Q2SoundLoop::Stop },
        loop_owner: None,
    }));
}

/// Open or close area portals (`portals`).
fn set_portals(actor: ActorId, game: &mut Q2GameServices, open: bool) {
    let target = game.require_entity(&actor).target.clone();
    for portal in game.targets(&target) {
        if game.require_entity(&portal).classname == "func_areaportal" {
            let style = integer_field(&game.require_entity(&portal).spawn.clone(), "style", 0);
            game.host.set_area_portal(style, open);
        }
    }
}

/// Crush an obstruction (`crush`).
fn crush(actor: ActorId, game: &mut Q2GameServices, other: ActorId) -> bool {
    let zero = vec3(0.0, 0.0, 0.0);
    let body = match game.host.bodies().read(&other) {
        Some(body) => body,
        None => return false,
    };
    if !game.host.is_player(&other) && !game.host.is_monster(&other) {
        let can_take = game
            .host
            .combat()
            .read(&other)
            .is_some_and(|state| state.can_take_damage);
        if can_take {
            game.damage(
                other.clone(),
                actor.clone(),
                Some(actor.clone()),
                100000.0,
                1.0,
                zero,
                body.origin,
                zero,
                20,
                0,
                None,
            );
        }
        if game.entity(&other).is_some() {
            game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
                effect: "q2:explosion1".to_string(),
                origin: body.origin,
                direction: zero,
                count: 1,
                color: 0,
            }));
            game.remove_actor(other);
        }
        return false;
    }
    let damage = game.require_entity(&actor).damage;
    game.damage(
        other,
        actor.clone(),
        Some(actor),
        damage,
        1.0,
        zero,
        body.origin,
        zero,
        20,
        0,
        None,
    );
    true
}

/// Platform use (`platformUse`).
fn plat_use(actor: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    if game.require_entity(&actor).think.is_some() {
        return;
    }
    plat_go_down(actor, game);
}

/// Platform blocked (`platformBlocked`).
fn plat_blocked(actor: ActorId, game: &mut Q2GameServices, other: ActorId) {
    if !crush(actor.clone(), game, other) {
        return;
    }
    match platform_state(game, &actor).phase {
        Q2PlatformPhase::Up => plat_go_down(actor, game),
        Q2PlatformPhase::Down => plat_go_up(actor, game),
        _ => {}
    }
}

/// Platform center touch (`Touch_Plat_Center`).
fn touch_plat_center(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let other = contact.other.clone();
    if !game.host.is_player(&other) {
        return;
    }
    let health = game.host.combat().read(&other).map_or(0.0, |state| state.health);
    if health <= 0.0 {
        return;
    }
    let enemy = game.require_entity(&actor).enemy.clone();
    let Some(platform) = enemy else { return };
    if game.entity(&platform).is_none() {
        return;
    }
    match platform_state(game, &platform).phase {
        Q2PlatformPhase::Bottom => plat_go_up(platform, game),
        Q2PlatformPhase::Top => game.schedule(platform, 1.0, plat_go_down as _),
        _ => {}
    }
}

/// Secret-door done (`door_secret_done`).
fn door_secret_done(actor: ActorId, game: &mut Q2GameServices) {
    if secret_state(game, &actor).shootable {
        let owned = game.owned_of(actor.clone());
        game.host.combat().set_health(&owned, 0.0);
        game.set_combat_traits(
            &owned,
            &crate::q2::support::contracts::CombatTraitChanges {
                can_take_damage: Some(true),
                mass: None,
                invulnerable: None,
                team: None,
                no_knockback: None,
            },
        );
    }
    set_portals(actor, game, false);
}

/// Secret-door move home (`door_secret_move6`).
fn door_secret_move6(actor: ActorId, game: &mut Q2GameServices) {
    let home = secret_state(game, &actor).home;
    linear_move_to(game, LinearMotionScope::Base, actor, home, door_secret_done as _);
}

/// Secret-door return pause (`door_secret_move5`).
fn door_secret_move5(actor: ActorId, game: &mut Q2GameServices) {
    game.schedule(actor, 1.0, door_secret_move6 as _);
}

/// Secret-door return (`door_secret_move4`).
fn door_secret_move4(actor: ActorId, game: &mut Q2GameServices) {
    let first = secret_state(game, &actor).first;
    linear_move_to(game, LinearMotionScope::Base, actor, first, door_secret_move5 as _);
}

/// Secret-door opened (`door_secret_move3`).
fn door_secret_move3(actor: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&actor).wait == -1.0 {
        return;
    }
    let wait = game.require_entity(&actor).wait;
    game.schedule(actor, wait, door_secret_move4 as _);
}

/// Secret-door second slide (`door_secret_move2`).
fn door_secret_move2(actor: ActorId, game: &mut Q2GameServices) {
    let second = secret_state(game, &actor).second;
    linear_move_to(game, LinearMotionScope::Base, actor, second, door_secret_move3 as _);
}

/// Secret-door pause (`door_secret_move1`).
fn door_secret_move1(actor: ActorId, game: &mut Q2GameServices) {
    game.schedule(actor, 1.0, door_secret_move2 as _);
}

/// Secret-door use (`secretUse`).
fn door_secret_use(actor: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let origin = game.body_of(actor.clone()).origin;
    let state = secret_state(game, &actor);
    if origin.x != state.home.x || origin.y != state.home.y || origin.z != state.home.z {
        return;
    }
    linear_move_to(
        game,
        LinearMotionScope::Base,
        actor.clone(),
        state.first,
        door_secret_move1 as _,
    );
    set_portals(actor, game, true);
}

/// Secret-door die (`secretDie`).
fn door_secret_die(actor: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    let owned = game.owned_of(actor.clone());
    game.set_combat_traits(
        &owned,
        &crate::q2::support::contracts::CombatTraitChanges {
            can_take_damage: Some(false),
            mass: None,
            invulnerable: None,
            team: None,
            no_knockback: None,
        },
    );
    let use_fn: Option<Q2Use> = game.require_entity(&actor).use_;
    if let Some(use_fn) = use_fn {
        let attacker = reaction.pain.attacker.clone();
        use_fn(actor, game, attacker.clone(), attacker);
    }
}

/// Secret-door touch (`secretTouch`).
fn door_secret_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let other = contact.other.clone();
    let state = secret_state(game, &actor);
    let now = game.host.now();
    if !game.host.is_player(&other) || state.message_time > now {
        return;
    }
    if let Some(entry) = game.base_entities.secrets.get_mut(&actor) {
        entry.message_time = now + 5.0;
    }
    let message = game.require_entity(&actor).message.clone();
    game.host.emit(Q2PresentationEvent::CenterPrint {
        actor: other,
        text: message,
        instant: false,
        duration_seconds: None,
    });
    game.sound(&actor, "misc/talk1.wav", 0, 1.0, 1.0);
}

/// Secret-door blocked (`secretBlocked`).
fn door_secret_blocked(actor: ActorId, game: &mut Q2GameServices, other: ActorId) {
    if !game.host.is_player(&other) && !game.host.is_monster(&other) {
        crush(actor, game, other);
        return;
    }
    let state = secret_state(game, &actor);
    let now = game.host.now();
    if state.blocked_time > now {
        return;
    }
    if let Some(entry) = game.base_entities.secrets.get_mut(&actor) {
        entry.blocked_time = now + 0.5;
    }
    crush(actor, game, other);
}

/// Platform arrival sound (`platformSound`).
fn platform_sound(actor: ActorId, game: &mut Q2GameServices, start: bool) {
    game.sound(
        &actor,
        if start {
            "plats/pt1_strt.wav"
        } else {
            "plats/pt1_end.wav"
        },
        2,
        1.0,
        3.0,
    );
    emit_loop(actor, game, "plats/pt1_mid.wav", start);
}

/// Platform hit bottom (`plat_hit_bottom`).
fn plat_hit_bottom(actor: ActorId, game: &mut Q2GameServices) {
    if let Some(entry) = game.base_entities.platforms.get_mut(&actor) {
        entry.phase = Q2PlatformPhase::Bottom;
    }
    platform_sound(actor, game, false);
}

/// Platform hit top (`plat_hit_top`).
fn plat_hit_top(actor: ActorId, game: &mut Q2GameServices) {
    if let Some(entry) = game.base_entities.platforms.get_mut(&actor) {
        entry.phase = Q2PlatformPhase::Top;
    }
    platform_sound(actor.clone(), game, false);
    game.schedule(actor, 3.0, plat_go_down as _);
}

/// Platform go down (`plat_go_down`).
fn plat_go_down(actor: ActorId, game: &mut Q2GameServices) {
    let bottom = {
        let entry = game
            .base_entities
            .platforms
            .get_mut(&actor)
            .expect("Platform callback without platform state");
        entry.phase = Q2PlatformPhase::Down;
        entry.bottom
    };
    platform_sound(actor.clone(), game, true);
    linear_move_to(game, LinearMotionScope::Base, actor, bottom, plat_hit_bottom as _);
}

/// Platform go up (`plat_go_up`).
fn plat_go_up(actor: ActorId, game: &mut Q2GameServices) {
    let top = {
        let entry = game
            .base_entities
            .platforms
            .get_mut(&actor)
            .expect("Platform callback without platform state");
        entry.phase = Q2PlatformPhase::Up;
        entry.top
    };
    platform_sound(actor.clone(), game, true);
    linear_move_to(game, LinearMotionScope::Base, actor, top, plat_hit_top as _);
}

/// Spawn a platform (`spawnPlatform`).
fn spawn_platform(actor: ActorId, game: &mut Q2GameServices) {
    let zero = vec3(0.0, 0.0, 0.0);
    let mut moved = game.body_of(actor.clone());
    moved.angles = zero;
    game.write_body(actor.clone(), &moved, false);
    game.set_solid(actor.clone(), Q2Solid::Brush);
    game.set_motion_kind(actor.clone(), Q2MotionKind::Push);
    {
        let entity = game.require_entity_mut(&actor);
        entity.speed = if entity.speed == 0.0 { 20.0 } else { entity.speed * 0.1 };
        entity.accel = if entity.accel == 0.0 { 5.0 } else { entity.accel * 0.1 };
        entity.decel = if entity.decel == 0.0 { 5.0 } else { entity.decel * 0.1 };
        if entity.damage == 0.0 {
            entity.damage = 2.0;
        }
    }
    let body = game.body_of(actor.clone());
    let spawn = game.require_entity(&actor).spawn.clone();
    let lip = {
        let lip = number_field(&spawn, "lip", 0.0);
        if lip == 0.0 {
            8.0
        } else {
            lip
        }
    };
    let height = {
        let height = number_field(&spawn, "height", 0.0);
        if height == 0.0 {
            f64::from(body.bounds.max.z - body.bounds.min.z) - lip
        } else {
            height
        }
    };
    let top = body.origin;
    let bottom = vec3(top.x, top.y, top.z - height as f32);
    let has_targetname = !game.require_entity(&actor).targetname.is_empty();
    game.base_entities.platforms.insert(
        actor.clone(),
        Q2PlatformState {
            top,
            bottom,
            phase: if has_targetname {
                Q2PlatformPhase::Up
            } else {
                Q2PlatformPhase::Bottom
            },
        },
    );
    {
        let entity = game.require_entity_mut(&actor);
        entity.use_ = Some(plat_use as _);
        entity.blocked = Some(plat_blocked as _);
    }
    let trigger = game.create("plat_trigger", BTreeMap::new());
    game.require_entity_mut(&trigger).enemy = Some(actor.clone());
    game.require_entity_mut(&trigger).visible = false;
    let mut min_x = body.bounds.min.x + 25.0;
    let mut max_x = body.bounds.max.x - 25.0;
    let mut min_y = body.bounds.min.y + 25.0;
    let mut max_y = body.bounds.max.y - 25.0;
    let min_z = body.bounds.max.z + 8.0 - (height + lip) as f32;
    if max_x - min_x <= 0.0 {
        min_x = (body.bounds.min.x + body.bounds.max.x) * 0.5;
        max_x = min_x + 1.0;
    }
    if max_y - min_y <= 0.0 {
        min_y = (body.bounds.min.y + body.bounds.max.y) * 0.5;
        max_y = min_y + 1.0;
    }
    let low_ceiling = game.require_entity(&actor).spawnflags & 1 != 0;
    let mut trigger_moved = game.body_of(trigger.clone());
    trigger_moved.bounds.min = vec3(min_x, min_y, min_z);
    trigger_moved.bounds.max = vec3(
        max_x,
        max_y,
        if low_ceiling {
            min_z + 8.0
        } else {
            body.bounds.max.z + 8.0
        },
    );
    game.write_body(trigger.clone(), &trigger_moved, false);
    game.require_entity_mut(&trigger).touch = Some(touch_plat_center as _);
    game.set_solid(trigger, Q2Solid::Trigger);
    if !has_targetname {
        let mut moved = game.body_of(actor.clone());
        moved.origin = bottom;
        game.write_body(actor.clone(), &moved, true);
    }
    game.show(actor);
}

/// Spawn a secret door (`spawnSecret`).
fn spawn_secret(actor: ActorId, game: &mut Q2GameServices) {
    let zero = vec3(0.0, 0.0, 0.0);
    let axes = angle_vectors(game.body_of(actor.clone()).angles);
    let mut moved = game.body_of(actor.clone());
    moved.angles = zero;
    game.write_body(actor.clone(), &moved, false);
    game.set_solid(actor.clone(), Q2Solid::Brush);
    game.set_motion_kind(actor.clone(), Q2MotionKind::Push);
    let body = game.body_of(actor.clone());
    let size = sub3(body.bounds.max, body.bounds.min);
    let entity = game.require_entity(&actor);
    let (spawnflags, max_health) = (entity.spawnflags, entity.max_health);
    let sideways = spawnflags & 4 != 0;
    let slide = if sideways { axes.up } else { axes.right };
    let width = dot3(slide, size).abs();
    let length = dot3(axes.forward, size).abs();
    let first = if sideways {
        add3(body.origin, scale3(slide, -width))
    } else {
        add3(
            body.origin,
            scale3(slide, f64::from(1 - (spawnflags & 2)) as f32 * width),
        )
    };
    let second = add3(first, scale3(axes.forward, length));
    {
        let entity = game.require_entity_mut(&actor);
        if entity.damage == 0.0 {
            entity.damage = 2.0;
        }
        if entity.wait == 0.0 {
            entity.wait = 5.0;
        }
        entity.speed = 50.0;
        entity.accel = 50.0;
        entity.decel = 50.0;
    }
    let entity = game.require_entity(&actor);
    let shootable = entity.targetname.is_empty() || entity.spawnflags & 1 != 0;
    let (targetname_empty, message_empty) = (entity.targetname.is_empty(), entity.message.is_empty());
    let owned = entity.actor.clone();
    if shootable || max_health != 0.0 {
        game.host.combat().create(
            &owned,
            &CombatState {
                health: if shootable { 0.0 } else { max_health },
                armor: ArmorState {
                    regular: RegularArmorState::None,
                    powered: PoweredProtectionState::None,
                },
                mass: 0.0,
                can_take_damage: true,
                invulnerable: false,
                no_knockback: false,
                team: None,
            },
        );
    }
    game.base_entities.secrets.insert(
        actor.clone(),
        Q2SecretState {
            first,
            second,
            home: zero,
            shootable,
            blocked_time: 0.0,
            message_time: 0.0,
        },
    );
    {
        let entity = game.require_entity_mut(&actor);
        entity.use_ = Some(door_secret_use as Q2Use);
        if shootable || max_health != 0.0 {
            entity.die = Some(door_secret_die as Q2Die);
        } else if !targetname_empty && !message_empty {
            entity.touch = Some(door_secret_touch as _);
        }
        entity.blocked = Some(door_secret_blocked as _);
    }
    game.show(actor);
}

/// Elevator init (`elevatorInit`).
fn trigger_elevator_init(actor: ActorId, game: &mut Q2GameServices) {
    let target = game.require_entity(&actor).target.clone();
    let train = game.pick_target(&target);
    let valid = train
        .as_ref()
        .is_some_and(|train| game.require_entity(train).classname == "func_train");
    if !valid {
        game.host
            .diagnostic(&format!("trigger_elevator has invalid train {target}"));
        return;
    }
    let entity = game.require_entity_mut(&actor);
    entity.enemy = train;
    entity.visible = false;
    entity.use_ = Some(trigger_elevator_use as _);
}

/// Elevator use (`elevatorUse`).
fn trigger_elevator_use(
    actor: ActorId,
    game: &mut Q2GameServices,
    other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let enemy = game.require_entity(&actor).enemy.clone();
    let Some(train) = enemy else { return };
    if game.entity(&train).is_none() {
        return;
    }
    if game.require_entity(&train).next_think.is_some() {
        return;
    }
    let path = other
        .as_ref()
        .and_then(|other| game.entity(other))
        .and_then(|entity| entity.spawn.values.get("pathtarget").cloned())
        .unwrap_or_default();
    let corner = game.pick_target(&path);
    let Some(corner) = corner else {
        game.host
            .diagnostic(&format!("trigger_elevator used with invalid pathtarget {path}"));
        return;
    };
    hooks(game).movers.resume_train_at(train, game, corner);
}

/// Conveyor use (`conveyorUse`).
fn func_conveyor_use(actor: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let entity = game.require_entity_mut(&actor);
    if entity.spawnflags & 1 != 0 {
        entity.speed = 0.0;
        entity.spawnflags &= !1;
    } else {
        entity.speed = f64::from(entity.count);
        entity.spawnflags |= 1;
    }
    if entity.spawnflags & 2 == 0 {
        entity.count = 0;
    }
}

/// Killbox use (`killboxUse`).
fn func_killbox_use(actor: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    kill_q2_box(game, actor);
}

/// Object touch (`objectTouch`).
fn func_object_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let other = contact.other.clone();
    let zero = vec3(0.0, 0.0, 0.0);
    match contact.plane {
        Some(plane) if plane.normal.z >= 1.0 => {}
        _ => return,
    }
    let can_take = game
        .host
        .combat()
        .read(&other)
        .is_some_and(|state| state.can_take_damage);
    if !can_take {
        return;
    }
    let damage = game.require_entity(&actor).damage;
    let origin = game.body_of(actor.clone()).origin;
    game.damage(
        other,
        actor.clone(),
        Some(actor),
        damage,
        1.0,
        zero,
        origin,
        zero,
        20,
        0,
        None,
    );
}

/// Object release (`objectRelease`).
fn func_object_release(actor: ActorId, game: &mut Q2GameServices) {
    game.set_motion_kind(actor.clone(), Q2MotionKind::Toss);
    game.require_entity_mut(&actor).touch = Some(func_object_touch as _);
}

/// Object use (`objectUse`).
fn func_object_use(actor: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    game.require_entity_mut(&actor).visible = true;
    game.require_entity_mut(&actor).use_ = None;
    game.set_solid(actor.clone(), Q2Solid::Brush);
    let moved = game.body_of(actor.clone());
    game.write_body(actor.clone(), &moved, true);
    kill_q2_box(game, actor.clone());
    game.show(actor.clone());
    func_object_release(actor, game);
}

/// Base mover callbacks (`Q2BaseMoverEntities[callbacks]`).
pub fn mover_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = base_linear_motion_callbacks();
    callbacks.think.insert("plat_hit_bottom", plat_hit_bottom as _);
    callbacks.think.insert("plat_hit_top", plat_hit_top as _);
    callbacks.think.insert("plat_go_down", plat_go_down as _);
    callbacks.think.insert("plat_go_up", plat_go_up as _);
    callbacks.think.insert("door_secret_done", door_secret_done as _);
    callbacks.think.insert("door_secret_move6", door_secret_move6 as _);
    callbacks.think.insert("door_secret_move5", door_secret_move5 as _);
    callbacks.think.insert("door_secret_move4", door_secret_move4 as _);
    callbacks.think.insert("door_secret_move3", door_secret_move3 as _);
    callbacks.think.insert("door_secret_move2", door_secret_move2 as _);
    callbacks.think.insert("door_secret_move1", door_secret_move1 as _);
    callbacks
        .think
        .insert("trigger_elevator_init", trigger_elevator_init as _);
    callbacks.think.insert("func_object_release", func_object_release as _);
    callbacks.use_.insert("plat_use", plat_use as _);
    callbacks.use_.insert("door_secret_use", door_secret_use as _);
    callbacks.use_.insert("trigger_elevator_use", trigger_elevator_use as _);
    callbacks.use_.insert("func_conveyor_use", func_conveyor_use as _);
    callbacks.use_.insert("func_killbox_use", func_killbox_use as _);
    callbacks.use_.insert("func_object_use", func_object_use as _);
    callbacks.touch.insert("Touch_Plat_Center", touch_plat_center as _);
    callbacks.touch.insert("door_secret_touch", door_secret_touch as _);
    callbacks.touch.insert("func_object_touch", func_object_touch as _);
    callbacks.die.insert("door_secret_die", door_secret_die as _);
    callbacks.blocked.insert("plat_blocked", plat_blocked as _);
    callbacks
        .blocked
        .insert("door_secret_blocked", door_secret_blocked as _);
    callbacks
}

/// Capture base movers (`Q2BaseMoverEntities[capture]`).
pub fn capture_movers(game: &mut Q2GameServices) -> Q2BaseMoversCheckpoint {
    let mut platforms = Vec::new();
    let mut secrets = Vec::new();
    let actors: Vec<ActorId> = game.entities.keys().cloned().collect();
    for actor in actors {
        let saved = SavedActorId::from(&actor);
        if let Some(state) = game.base_entities.platforms.get(&actor).copied() {
            platforms.push(Q2PlatformEntry { actor: saved, state });
        }
        if let Some(state) = game.base_entities.secrets.get(&actor).copied() {
            secrets.push(Q2SecretEntry { actor: saved, state });
        }
    }
    let linear = capture_linear_motion(game, LinearMotionScope::Base);
    Q2BaseMoversCheckpoint {
        platforms,
        secrets,
        linear,
    }
}

/// Restore base movers (`Q2BaseMoverEntities[restore]`).
pub fn restore_movers(game: &mut Q2GameServices, checkpoint: &Q2BaseMoversCheckpoint) {
    game.base_entities.platforms = std::collections::HashMap::new();
    game.base_entities.secrets = std::collections::HashMap::new();
    for saved in &checkpoint.platforms {
        let actor = restore_q2_actor(game, saved.actor).id().clone();
        if game.entity(&actor).is_none() {
            panic!("Missing saved Q2 base mover");
        }
        game.base_entities.platforms.insert(actor, saved.state);
    }
    for saved in &checkpoint.secrets {
        let actor = restore_q2_actor(game, saved.actor).id().clone();
        if game.entity(&actor).is_none() {
            panic!("Missing saved Q2 base mover");
        }
        game.base_entities.secrets.insert(actor, saved.state);
    }
    restore_linear_motion(game, LinearMotionScope::Base, &checkpoint.linear);
}

/// Read platform state (`Q2BaseMoverEntities[platformState]`).
pub fn mover_platform_state(game: &Q2GameServices, actor: &ActorId) -> Option<Q2PlatformState> {
    game.base_entities.platforms.get(actor).copied()
}

/// Read mover traversal (`Q2BaseMoverEntities[traversal]`).
pub fn mover_traversal(game: &mut Q2GameServices, actor: &ActorId) -> Option<Q2PlatformTraversal> {
    let platform = game.base_entities.platforms.get(actor).copied();
    let destination = linear_move_destination(game, LinearMotionScope::Base, actor);
    if platform.is_none() && !game.base_entities.secrets.contains_key(actor) {
        return None;
    }
    let entity = game.require_entity(actor);
    let locked = platform.is_some_and(|platform| platform.phase == Q2PlatformPhase::Up)
        && !entity.targetname.is_empty()
        && destination.is_none()
        && entity.think.is_none();
    Some(Q2PlatformTraversal { locked, destination })
}

/// Spawn a base mover entity (`Q2BaseMoverEntities[spawn]`).
pub fn spawn_mover(actor: ActorId, game: &mut Q2GameServices) -> bool {
    let classname = game.require_entity(&actor).classname.clone();
    match classname.as_str() {
        "func_plat" => {
            spawn_platform(actor, game);
            true
        }
        "func_door_secret" => {
            spawn_secret(actor, game);
            true
        }
        "trigger_elevator" => {
            let frame_seconds = game.host.frame_seconds();
            game.schedule(actor, frame_seconds, trigger_elevator_init as _);
            true
        }
        "func_conveyor" => {
            {
                let entity = game.require_entity_mut(&actor);
                if entity.speed == 0.0 {
                    entity.speed = 100.0;
                }
                if entity.spawnflags & 1 == 0 {
                    entity.count = entity.speed as i32;
                    entity.speed = 0.0;
                }
            }
            game.set_solid(actor.clone(), Q2Solid::Brush);
            game.show(actor.clone());
            game.require_entity_mut(&actor).use_ = Some(func_conveyor_use as _);
            true
        }
        "func_killbox" => {
            game.require_entity_mut(&actor).visible = false;
            game.set_solid(actor.clone(), Q2Solid::None);
            game.require_entity_mut(&actor).use_ = Some(func_killbox_use as _);
            true
        }
        "func_object" => {
            game.set_solid(actor.clone(), Q2Solid::Brush);
            game.set_motion_kind(actor.clone(), Q2MotionKind::Push);
            {
                let entity = game.require_entity_mut(&actor);
                if entity.damage == 0.0 {
                    entity.damage = 100.0;
                }
            }
            let original = game.body_of(actor.clone()).bounds;
            let unit = vec3(1.0, 1.0, 1.0);
            let shrunk_min = add3(original.min, unit);
            let shrunk_max = sub3(original.max, unit);
            let spawnflags = game.require_entity(&actor).spawnflags;
            if spawnflags == 0 {
                let delay = 2.0 * game.host.frame_seconds();
                game.schedule(actor.clone(), delay, func_object_release as _);
            } else {
                game.require_entity_mut(&actor).visible = false;
                game.set_solid(actor.clone(), Q2Solid::None);
                game.require_entity_mut(&actor).use_ = Some(func_object_use as _);
            }
            {
                let entity = game.require_entity_mut(&actor);
                if entity.spawnflags & 2 != 0 {
                    entity.effects |= 0x1000;
                }
                if entity.spawnflags & 4 != 0 {
                    entity.effects |= 0x2000;
                }
                entity.clip_mask = 0x2010003;
            }
            let mut moved = game.body_of(actor.clone());
            moved.bounds.min = shrunk_min;
            moved.bounds.max = shrunk_max;
            game.write_body(actor.clone(), &moved, true);
            game.show(actor);
            true
        }
        _ => false,
    }
}
