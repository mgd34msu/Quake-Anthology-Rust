//! Q2 turret entities (`src/content/q2/base/entities/turrets.ts`).
//!
//! Quake II g_turret.c. Turret drivers reuse the permanent infantry
//! AI/death runner.

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{add3, length3, scale3, sub3, vec3, Vec3};

use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::checkpoint::{restore_q2_actor, save_q2_actor};
use crate::q2::foundation::fields::number_field;
use crate::q2::foundation::host::{
    Q2Die, Q2GameServices, Q2Mode, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop,
};
use crate::q2::foundation::monsters::ai::visible;
use crate::q2::foundation::monsters::infantry::infantry_stand;
use crate::q2::foundation::monsters::types::MonsterContext;
use crate::q2::foundation::weapons::vectors::{angle_vectors, vector_angles};
use crate::q2::support::contracts::DeathReaction;

use super::types::Q2BaseEntityHooks;

/// Turret breach state (`Breach`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2BreachState {
    /// Goal angles.
    pub goal: Vec3,
    /// Muzzle offset.
    pub muzzle: Vec3,
    /// Maximum pitch.
    pub pitch_max: f64,
    /// Minimum pitch.
    pub pitch_min: f64,
    /// Minimum yaw.
    pub yaw_min: f64,
    /// Maximum yaw.
    pub yaw_max: f64,
}

/// Turret driver state (`Driver`).
#[derive(Debug, Clone)]
pub struct Q2DriverState {
    /// Bound monster death callback.
    pub monster_die: Q2Die,
    /// Breach actor.
    pub breach: Option<ActorId>,
    /// Orbit radius.
    pub radius: f64,
    /// Yaw offset.
    pub yaw_offset: f64,
    /// Height offset.
    pub height: f64,
}

/// Breach checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2BreachEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// State.
    pub state: Q2BreachState,
}

/// Driver checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2DriverEntry {
    /// Actor.
    pub actor: SavedActorId,
    /// Breach actor.
    pub breach: Option<SavedActorId>,
    /// Orbit radius.
    pub radius: f64,
    /// Yaw offset.
    pub yaw_offset: f64,
    /// Height offset.
    pub height: f64,
    /// Monster death callback name.
    pub monster_die: String,
}

/// Turrets checkpoint (`Q2TurretsCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TurretsCheckpoint {
    /// Breaches.
    pub breaches: Vec<Q2BreachEntry>,
    /// Drivers.
    pub drivers: Vec<Q2DriverEntry>,
}

/// Snap to turret eighths (`snapQ2TurretEighth`).
pub fn snap_q2_turret_eighth(value: f64) -> f64 {
    (value * 8.0 + if value > 0.0 { 0.5 } else { -0.5 }).trunc() * 0.125
}

/// Normalize an angle to 0..=360 (`normalizeAngle`).
fn normalize_angle(mut value: f64) -> f64 {
    while value > 360.0 {
        value -= 360.0;
    }
    while value < 0.0 {
        value += 360.0;
    }
    value
}

/// Wrap an angle to -180..=180 (`shortAngle`).
fn short_angle(value: f64) -> f64 {
    if value < -180.0 {
        value + 360.0
    } else if value > 180.0 {
        value - 360.0
    } else {
        value
    }
}

/// Read registered hooks.
fn hooks(game: &Q2GameServices) -> Q2BaseEntityHooks {
    game.base_entities
        .hooks
        .expect("Q2 base entity hooks are not registered")
}

/// Team members in chain order (`team`).
fn team(game: &Q2GameServices, actor: &ActorId) -> Vec<ActorId> {
    let entity = game.require_entity(actor);
    if let Some(master) = entity.team_master.clone() {
        if game.entity(&master).is_some() {
            let mut members = Vec::new();
            let mut member = Some(master);
            while let Some(current) = member {
                member = game.require_entity(&current).team_chain.clone();
                members.push(current);
            }
            return members;
        }
    }
    match entity.spawn.values.get("team").cloned() {
        None => vec![actor.clone()],
        Some(team) => {
            let mut members: Vec<ActorId> = game
                .entities
                .values()
                .filter(|member| member.spawn.values.get("team") == Some(&team))
                .map(|member| member.actor.id().clone())
                .collect();
            members.sort_by_key(|member| (member.slot(), member.generation()));
            members
        }
    }
}

/// Turret blocked (`turretBlocked`).
fn turret_blocked(actor: ActorId, game: &mut Q2GameServices, other: ActorId) {
    let zero = vec3(0.0, 0.0, 0.0);
    let body = match game.host.bodies().read(&other) {
        Some(body) => body,
        None => return,
    };
    let can_take = game
        .host
        .combat()
        .read(&other)
        .is_some_and(|state| state.can_take_damage);
    if !can_take {
        return;
    }
    let master = team(game, &actor).first().cloned().unwrap_or(actor.clone());
    let entity = game.require_entity(&master);
    let attacker = entity.owner.clone().unwrap_or(master);
    let damage = entity.damage;
    game.damage(
        other,
        actor,
        Some(attacker),
        damage,
        10.0,
        zero,
        body.origin,
        zero,
        20,
        0,
        None,
    );
}

/// Shared turret brush setup (`blocked`).
fn init_brush(actor: ActorId, game: &mut Q2GameServices) {
    game.require_entity_mut(&actor).blocked = Some(turret_blocked as _);
    game.set_solid(actor.clone(), Q2Solid::Brush);
    game.set_motion_kind(actor.clone(), Q2MotionKind::Push);
    game.show(actor);
}

/// Fire the breach (`fire`).
fn fire_breach(actor: ActorId, game: &mut Q2GameServices, state: Q2BreachState, driver: ActorId) {
    let body = game.body_of(actor.clone());
    let axes = angle_vectors(body.angles);
    let start = add3(
        add3(
            add3(body.origin, scale3(axes.forward, state.muzzle.x)),
            scale3(axes.right, state.muzzle.y),
        ),
        scale3(axes.up, state.muzzle.z),
    );
    let damage = (100.0 + game.host.random() * 50.0).trunc();
    let speed = 550.0 + 50.0 * f64::from(game.options.skill);
    (hooks(game).fire_rocket)(driver, game, start, axes.forward, damage, speed, 150.0, damage);
    game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(actor),
        origin: start,
        path: "weapons/rocklf1a.wav".to_string(),
        channel: 1,
        volume: 1.0,
        attenuation: 1.0,
        reliable: false,
        loop_: Q2SoundLoop::Once,
        loop_owner: None,
    }));
}

/// Breach think (`breachThink`).
fn turret_breach_think(actor: ActorId, game: &mut Q2GameServices) {
    let state = match game.base_entities.breaches.get(&actor).copied() {
        Some(state) => state,
        None => return,
    };
    let body = game.body_of(actor.clone());
    let frame = game.host.frame_seconds();
    let mut pitch = normalize_angle(f64::from(state.goal.x));
    let mut yaw = normalize_angle(f64::from(state.goal.y));
    if pitch > 180.0 {
        pitch -= 360.0;
    }
    pitch = pitch.clamp(state.pitch_min, state.pitch_max);
    if yaw < state.yaw_min || yaw > state.yaw_max {
        let min = short_angle((state.yaw_min - yaw).abs()).abs();
        let max = short_angle((state.yaw_max - yaw).abs()).abs();
        yaw = if min < max { state.yaw_min } else { state.yaw_max };
    }
    let speed = game.require_entity(&actor).speed;
    if let Some(entry) = game.base_entities.breaches.get_mut(&actor) {
        entry.goal = vec3(pitch as f32, yaw as f32, state.goal.z);
    }
    let clamp = |delta: f64| short_angle(delta).clamp(-speed * frame, speed * frame) / frame;
    let angular = vec3(
        clamp(pitch - normalize_angle(f64::from(body.angles.x))) as f32,
        clamp(yaw - normalize_angle(f64::from(body.angles.y))) as f32,
        0.0,
    );
    game.require_entity_mut(&actor).angular_velocity = angular;
    game.set_motion_kind(actor.clone(), Q2MotionKind::Push);
    for member in team(game, &actor) {
        {
            let entity = game.require_entity_mut(&member);
            entity.angular_velocity = vec3(entity.angular_velocity.x, angular.y, entity.angular_velocity.z);
        }
        game.set_motion_kind(member, Q2MotionKind::Push);
    }
    let owner = game.require_entity(&actor).owner.clone();
    let driver = owner
        .as_ref()
        .and_then(|owner| game.base_entities.drivers.get(owner).cloned());
    if let (Some(owner), Some(driver)) = (owner, driver) {
        let driver_body = game.body_of(owner.clone());
        let radians = (f64::from(body.angles.y) + driver.yaw_offset) * std::f64::consts::PI / 180.0;
        let target = vec3(
            snap_q2_turret_eighth(f64::from(body.origin.x) + radians.cos() * driver.radius) as f32,
            snap_q2_turret_eighth(f64::from(body.origin.y) + radians.sin() * driver.radius) as f32,
            snap_q2_turret_eighth(
                f64::from(body.origin.z)
                    + driver.radius * (f64::from(body.angles.x) * std::f64::consts::PI / 180.0).tan()
                    + driver.height,
            ) as f32,
        );
        {
            let entity = game.require_entity_mut(&owner);
            entity.angular_velocity = vec3(angular.x, angular.y, entity.angular_velocity.z);
        }
        let mut moved = game.body_of(owner.clone());
        moved.velocity = scale3(sub3(target, driver_body.origin), (1.0 / frame) as f32);
        game.write_body(owner.clone(), &moved, false);
        game.set_motion_kind(owner, Q2MotionKind::Push);
        if game.require_entity(&actor).spawnflags & 65536 != 0 {
            let state = game
                .base_entities
                .breaches
                .get(&actor)
                .copied()
                .expect("Turret breach state vanished");
            let owner = game.require_entity(&actor).owner.clone();
            let Some(owner) = owner else {
                game.schedule(actor, frame, turret_breach_think as _);
                return;
            };
            fire_breach(actor.clone(), game, state, owner);
            game.require_entity_mut(&actor).spawnflags &= !65536;
        }
    }
    game.schedule(actor, frame, turret_breach_think as _);
}

/// Driver think (`driverThink`).
fn turret_driver_think(actor: ActorId, game: &mut Q2GameServices) {
    let driver = game.base_entities.drivers.get(&actor).cloned();
    let turret = driver.and_then(|driver| driver.breach.clone());
    let Some(turret) = turret else { return };
    if game.entity(&turret).is_none() {
        return;
    }
    let frame_seconds = game.host.frame_seconds();
    game.schedule(actor.clone(), frame_seconds, turret_driver_think as _);
    let enemy = game.require_entity(&actor).enemy.clone();
    if let Some(enemy) = enemy {
        let dead = !game.host.actors().is_live(&enemy)
            || game.host.combat().read(&enemy).map_or(0.0, |state| state.health) <= 0.0;
        if dead {
            game.require_entity_mut(&actor).enemy = None;
        }
    }
    let mut context = MonsterContext::new(actor.clone(), game);
    if context.entity().enemy.is_none() {
        if !context.find_target() {
            return;
        }
        let now = context.game.host.now();
        context.state_mut().trail_time = now;
        context.state_mut().lost_sight = false;
    } else if visible(&mut context, None) {
        if context.state().lost_sight {
            let now = context.game.host.now();
            context.state_mut().trail_time = now;
            context.state_mut().lost_sight = false;
        }
    } else {
        context.state_mut().lost_sight = true;
        return;
    }
    let game = context.game;
    let enemy = game.require_entity(&actor).enemy.clone();
    let body = enemy.as_ref().and_then(|enemy| game.host.bodies().read(enemy));
    let breach = game.base_entities.breaches.get(&turret).copied();
    let (Some(enemy), Some(enemy_body), Some(_breach)) = (enemy, body, breach) else {
        return;
    };
    let view_height = game.entity(&enemy).map_or(22, |entity| entity.view_height);
    let turret_origin = game.body_of(turret.clone()).origin;
    let goal = vector_angles(sub3(
        add3(enemy_body.origin, vec3(0.0, 0.0, view_height as f32)),
        turret_origin,
    ));
    if let Some(entry) = game.base_entities.breaches.get_mut(&turret) {
        entry.goal = goal;
    }
    let mut context = MonsterContext::new(actor.clone(), game);
    let now = context.game.host.now();
    if now < context.state().attack_finished {
        return;
    }
    let reaction_time = 3.0 - f64::from(context.game.options.skill);
    if now - context.state().trail_time < reaction_time {
        return;
    }
    context.state_mut().attack_finished = now + reaction_time + 1.0;
    let game = context.game;
    game.require_entity_mut(&turret).spawnflags |= 65536;
}

/// Breach init (`breachInit`).
fn turret_breach_finish_init(actor: ActorId, game: &mut Q2GameServices) {
    if !game.base_entities.breaches.contains_key(&actor) {
        panic!("Turret breach initialization without source state");
    }
    let target = game.require_entity(&actor).target.clone();
    let muzzle = game.pick_target(&target);
    match muzzle {
        None => game
            .host
            .diagnostic(&format!("turret_breach missing muzzle target {target}")),
        Some(muzzle) => {
            let delta = sub3(game.body_of(muzzle.clone()).origin, game.body_of(actor.clone()).origin);
            if let Some(entry) = game.base_entities.breaches.get_mut(&actor) {
                entry.muzzle = delta;
            }
            game.remove_actor(muzzle);
        }
    }
    let master = team(game, &actor).first().cloned().unwrap_or(actor.clone());
    let damage = game.require_entity(&actor).damage;
    game.require_entity_mut(&master).damage = damage;
    turret_breach_think(actor, game);
}

/// Driver die (`driverDie`).
fn turret_driver_die(actor: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    let state = game
        .base_entities
        .drivers
        .get(&actor)
        .cloned()
        .expect("Turret driver death without source state");
    if let Some(turret) = state.breach.clone() {
        if game.entity(&turret).is_some() {
            if let Some(entry) = game.base_entities.breaches.get_mut(&turret) {
                entry.goal = vec3(0.0, entry.goal.y, entry.goal.z);
            }
            game.require_entity_mut(&turret).owner = None;
            let master = team(game, &turret).first().cloned().unwrap_or(turret.clone());
            game.require_entity_mut(&master).owner = None;
            let team_chain = game.require_entity(&actor).team_chain.clone();
            for member in team(game, &turret) {
                if game.require_entity(&member).team_chain == Some(actor.clone()) {
                    game.require_entity_mut(&member).team_chain = team_chain.clone();
                    break;
                }
            }
        }
    }
    if let Some(entry) = game.base_entities.drivers.get_mut(&actor) {
        entry.breach = None;
    }
    {
        let entity = game.require_entity_mut(&actor);
        entity.team_master = None;
        entity.team_chain = None;
        entity.flags &= !1024;
        entity.angular_velocity = vec3(0.0, 0.0, 0.0);
    }
    game.set_motion_kind(actor.clone(), Q2MotionKind::Step);
    (state.monster_die)(actor.clone(), game, reaction);
    let context = MonsterContext::new(actor.clone(), game);
    let gibbed = context.state().gibbed;
    let game = context.game;
    if !gibbed && game.host.actors().is_live(&actor) {
        (hooks(game).resume_monster)(actor, game);
    }
}

/// Driver link (`driverLink`).
fn turret_driver_link(actor: ActorId, game: &mut Q2GameServices) {
    if !game.base_entities.drivers.contains_key(&actor) {
        panic!("Turret driver link without source state");
    }
    let target = game.require_entity(&actor).target.clone();
    let breach = game.pick_target(&target);
    let valid = breach
        .as_ref()
        .is_some_and(|breach| game.base_entities.breaches.contains_key(breach));
    if !valid {
        game.host
            .diagnostic(&format!("turret_driver has invalid breach {target}"));
        return;
    }
    let breach = breach.expect("Turret driver breach vanished");
    if let Some(entry) = game.base_entities.drivers.get_mut(&actor) {
        entry.breach = Some(breach.clone());
    }
    game.require_entity_mut(&breach).owner = Some(actor.clone());
    let members = team(game, &breach);
    let master = members.first().cloned().unwrap_or(breach.clone());
    let last = members.last().cloned().unwrap_or(breach.clone());
    game.require_entity_mut(&master).owner = Some(actor.clone());
    game.require_entity_mut(&master).team_master = Some(master.clone());
    game.require_entity_mut(&last).team_chain = Some(actor.clone());
    {
        let entity = game.require_entity_mut(&actor);
        entity.team_master = Some(master);
        entity.team_chain = None;
    }
    let body = game.body_of(actor.clone());
    let target_body = game.body_of(breach);
    let delta = sub3(body.origin, target_body.origin);
    let radius = length3(vec3(delta.x, delta.y, 0.0));
    let yaw_offset = normalize_angle(f64::from(vector_angles(delta).y));
    if let Some(entry) = game.base_entities.drivers.get_mut(&actor) {
        entry.radius = f64::from(radius);
        entry.yaw_offset = yaw_offset;
        entry.height = f64::from(delta.z);
    }
    let mut moved = game.body_of(actor.clone());
    moved.angles = target_body.angles;
    game.write_body(actor.clone(), &moved, false);
    game.require_entity_mut(&actor).flags |= 1024;
    let frame_seconds = game.host.frame_seconds();
    game.schedule(actor, frame_seconds, turret_driver_think as _);
}

/// Turret callbacks (`Q2TurretEntities[callbacks]`).
pub fn turret_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("turret_breach_think", turret_breach_think as _);
    callbacks
        .think
        .insert("turret_breach_finish_init", turret_breach_finish_init as _);
    callbacks.think.insert("turret_driver_think", turret_driver_think as _);
    callbacks.think.insert("turret_driver_link", turret_driver_link as _);
    callbacks.die.insert("turret_driver_die", turret_driver_die as _);
    callbacks.blocked.insert("turret_blocked", turret_blocked as _);
    callbacks
}

/// Capture turrets (`Q2TurretEntities[capture]`).
pub fn capture_turrets(game: &mut Q2GameServices) -> Q2TurretsCheckpoint {
    let mut breaches = Vec::new();
    let mut drivers = Vec::new();
    let actors: Vec<ActorId> = game.entities.keys().cloned().collect();
    for actor in actors {
        let saved = SavedActorId::from(&actor);
        if let Some(state) = game.base_entities.breaches.get(&actor).copied() {
            breaches.push(Q2BreachEntry {
                actor: saved.clone(),
                state,
            });
        }
        if let Some(state) = game.base_entities.drivers.get(&actor).cloned() {
            let monster_die = game
                .source_callbacks
                .die_name(Some(state.monster_die))
                .unwrap_or_else(|| panic!("Turret driver has no named monster death callback"))
                .to_string();
            drivers.push(Q2DriverEntry {
                actor: saved,
                monster_die,
                radius: state.radius,
                yaw_offset: state.yaw_offset,
                height: state.height,
                breach: save_q2_actor(state.breach.as_ref()),
            });
        }
    }
    Q2TurretsCheckpoint { breaches, drivers }
}

/// Restore turrets (`Q2TurretEntities[restore]`).
pub fn restore_turrets(game: &mut Q2GameServices, checkpoint: &Q2TurretsCheckpoint) {
    game.base_entities.breaches = std::collections::HashMap::new();
    game.base_entities.drivers = std::collections::HashMap::new();
    for saved in &checkpoint.breaches {
        let actor = restore_q2_actor(game, saved.actor.clone()).id().clone();
        if game.entity(&actor).is_none() {
            panic!("Missing saved Q2 turret entity");
        }
        game.base_entities.breaches.insert(actor, saved.state);
    }
    for saved in &checkpoint.drivers {
        let actor = restore_q2_actor(game, saved.actor.clone()).id().clone();
        if game.entity(&actor).is_none() {
            panic!("Missing saved Q2 turret entity");
        }
        let admitted = (hooks(game).monster_context)(game, &actor);
        let monster_die = game.source_callbacks.resolve_die(Some(&saved.monster_die));
        let (true, Some(monster_die)) = (admitted, monster_die) else {
            panic!("Restore Q2 turret drivers after their monster state");
        };
        let breach = saved
            .breach
            .clone()
            .and_then(|breach| game.host.actors().resolve_saved(breach).map(|owned| owned.id().clone()));
        game.base_entities.drivers.insert(
            actor,
            Q2DriverState {
                monster_die,
                breach,
                radius: saved.radius,
                yaw_offset: saved.yaw_offset,
                height: saved.height,
            },
        );
    }
}

/// Spawn a turret entity (`Q2TurretEntities[spawn]`).
pub fn spawn_turret(actor: ActorId, game: &mut Q2GameServices) -> bool {
    let classname = game.require_entity(&actor).classname.clone();
    match classname.as_str() {
        "turret_base" => {
            init_brush(actor, game);
            true
        }
        "turret_breach" => {
            init_brush(actor.clone(), game);
            {
                let entity = game.require_entity_mut(&actor);
                if entity.speed == 0.0 {
                    entity.speed = 50.0;
                }
                if entity.damage == 0.0 {
                    entity.damage = 10.0;
                }
            }
            let spawn = game.require_entity(&actor).spawn.clone();
            let yaw = game.body_of(actor.clone()).angles.y;
            let min_pitch = number_field(&spawn, "minpitch", 0.0);
            let max_pitch = number_field(&spawn, "maxpitch", 0.0);
            let min_yaw = number_field(&spawn, "minyaw", 0.0);
            let max_yaw = number_field(&spawn, "maxyaw", 0.0);
            game.base_entities.breaches.insert(
                actor.clone(),
                Q2BreachState {
                    goal: vec3(0.0, yaw, 0.0),
                    muzzle: vec3(0.0, 0.0, 0.0),
                    pitch_max: -(if min_pitch == 0.0 { -30.0 } else { min_pitch }),
                    pitch_min: -(if max_pitch == 0.0 { 30.0 } else { max_pitch }),
                    yaw_min: min_yaw,
                    yaw_max: if max_yaw == 0.0 { 360.0 } else { max_yaw },
                },
            );
            let frame_seconds = game.host.frame_seconds();
            game.schedule(actor, frame_seconds, turret_breach_finish_init as _);
            true
        }
        "turret_driver" => {
            if game.options.mode == Q2Mode::Deathmatch {
                game.remove_actor(actor);
                return true;
            }
            let mut context = (hooks(game).turret_driver)(actor.clone(), game);
            context.state_mut().gib_health = 0.0;
            context.state_mut().stand_ground = true;
            context.state_mut().ducked = true;
            infantry_stand(&mut context);
            let game = context.game;
            let monster_die = game.require_entity(&actor).die;
            let Some(monster_die) = monster_die else {
                panic!("Turret driver admission did not bind infantry death");
            };
            game.base_entities.drivers.insert(
                actor.clone(),
                Q2DriverState {
                    monster_die,
                    breach: None,
                    radius: 0.0,
                    yaw_offset: 0.0,
                    height: 0.0,
                },
            );
            {
                let entity = game.require_entity_mut(&actor);
                entity.flags |= 2048;
                entity.server_flags |= 4;
                entity.render_flags |= 64;
                entity.view_height = 24;
                entity.frame = 0;
                entity.die = Some(turret_driver_die as _);
            }
            game.set_motion_kind(actor.clone(), Q2MotionKind::Push);
            game.set_solid(actor.clone(), Q2Solid::Box);
            game.show(actor.clone());
            let frame_seconds = game.host.frame_seconds();
            game.schedule(actor, frame_seconds, turret_driver_link as _);
            true
        }
        _ => false,
    }
}
