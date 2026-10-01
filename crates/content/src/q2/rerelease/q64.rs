//! Q2 rerelease Q64 entities (`src/content/q2/rerelease/q64/index.ts`).
//!
//! Rerelease g_func.cpp / g_target.cpp Q64 entities run on the shared
//! source scheduler.
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{add3, angle_mod, dot3, length3, normalize3, scale3, sub3, vec3, Vec3};

use super::players::{rerelease_end_of_unit, rerelease_finish_camera, rerelease_move_to_camera, Q2RereleasePlayers};
use super::rerelease_hooks;
use super::types::{Q2RereleaseEvent, Q2RereleaseHooks};
use super::view::q2_rerelease_dummy_animation;
use crate::q2::base::player::index::Q2Intermission;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::{number_field, vector_field};
use crate::q2::foundation::host::{Q2GameServices, Q2Mode, Q2MotionKind, Q2PresentationEvent, Q2Solid};
use crate::q2::foundation::weapons::vectors::{angle_vectors, vector_angles};

/// Turn toward an ideal angle (`turn`).
fn turn_angle(current: f64, ideal: f64, speed: f64) -> f64 {
    let current = angle_mod(current);
    let mut delta = ideal - current;
    if ideal > current && delta >= 180.0 {
        delta -= 360.0;
    } else if ideal <= current && delta <= -180.0 {
        delta += 360.0;
    }
    angle_mod(current + (-speed).max(speed.min(delta)))
}

/// Aim a camera at its path target (`lookAt`).
fn look_at(entity: &ActorId, game: &mut Q2GameServices, origin: Vec3, previous: Vec3) -> Vec3 {
    let name = game
        .require_entity(entity)
        .spawn
        .values
        .get("pathtarget")
        .cloned()
        .unwrap_or_default();
    let Some(target) = game.targets(&name).first().cloned() else {
        return previous;
    };
    let delta = sub3(game.body_of(target).origin, origin);
    let planar = (f64::from(delta.x).powi(2) + f64::from(delta.y).powi(2)).sqrt();
    if planar == 0.0 {
        vec3(if delta.z > 0.0 { -90.0 } else { 90.0 }, 0.0, 0.0)
    } else {
        vec3(
            (-f64::from(delta.z).atan2(planar) * 180.0 / std::f64::consts::PI) as f32,
            (f64::from(delta.y).atan2(f64::from(delta.x)) * 180.0 / std::f64::consts::PI) as f32,
            0.0,
        )
    }
}

/// Spinning think (`spinningThink`).
fn spinning_think(entity: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&entity).timestamp <= game.now() {
        let (decel, speed) = {
            let record = game.require_entity(&entity);
            (record.decel, record.speed)
        };
        let now = game.now();
        let mut component = decel + game.random() * (speed - decel);
        if game.random() < 0.5 {
            component = -component;
        }
        let x = component;
        component = decel + game.random() * (speed - decel);
        if game.random() < 0.5 {
            component = -component;
        }
        let y = component;
        component = decel + game.random() * (speed - decel);
        if game.random() < 0.5 {
            component = -component;
        }
        let z = component;
        let timestamp = now + 1.0 + game.random() * 5.0;
        let record = game.require_entity_mut(&entity);
        record.timestamp = timestamp;
        record.movedir = vec3(x as f32, y as f32, z as f32);
    }
    let (accel, movedir, mut angular) = {
        let record = game.require_entity(&entity);
        (record.accel, record.movedir, record.angular_velocity)
    };
    let step = |current: f32, wanted: f32| {
        if current < wanted {
            wanted.min(current + accel as f32)
        } else {
            wanted.max(current - accel as f32)
        }
    };
    angular = vec3(
        step(angular.x, movedir.x),
        step(angular.y, movedir.y),
        step(angular.z, movedir.z),
    );
    game.require_entity_mut(&entity).angular_velocity = angular;
    game.set_motion_kind(entity.clone(), Q2MotionKind::Push);
    let frame = game.host.frame_seconds();
    game.schedule(entity, frame, spinning_think);
}

/// Eye setup (`eyeSetup`).
fn eye_setup(entity: ActorId, game: &mut Q2GameServices) {
    if !game.rerelease.q64_eyes.contains_key(&entity) {
        panic!("func_eye source state missing");
    }
    let name = game
        .require_entity(&entity)
        .spawn
        .values
        .get("pathtarget")
        .cloned()
        .unwrap_or_default();
    match game.pick_target(&name) {
        None => game.host.diagnostic("func_eye: bad target"),
        Some(target) => {
            let position = sub3(game.body_of(target).origin, game.body_of(entity.clone()).origin);
            game.rerelease
                .q64_eyes
                .get_mut(&entity)
                .expect("func_eye source state missing")
                .eye_position = position;
        }
    }
    let position = game
        .rerelease
        .q64_eyes
        .get(&entity)
        .expect("func_eye source state missing")
        .eye_position;
    game.require_entity_mut(&entity).movedir = normalize3(position);
    game.schedule(entity, 0.1, eye_think);
}

/// Eye think (`eyeThink`).
fn eye_think(entity: ActorId, game: &mut Q2GameServices) {
    if !game.rerelease.q64_eyes.contains_key(&entity) {
        panic!("func_eye source state missing");
    }
    let body = game.body_of(entity.clone());
    let mut closest: Option<ActorId> = None;
    let mut closest_distance = f64::INFINITY;
    for actor in game.host.players() {
        let connected = game
            .players
            .states
            .get(&actor)
            .map(|player| player.connected)
            .unwrap_or(false);
        let target = game.host.bodies().read(&actor);
        let Some(target) = target else {
            continue;
        };
        if !connected {
            continue;
        }
        let direction = sub3(target.origin, body.origin);
        let distance = f64::from(length3(direction));
        let record = game.require_entity(&entity);
        let cone = game
            .rerelease
            .q64_eyes
            .get(&entity)
            .expect("func_eye source state missing")
            .vision_cone;
        if f64::from(dot3(normalize3(direction), record.movedir)) < cone
            || distance >= record.damage_radius
            || distance >= closest_distance
        {
            continue;
        }
        closest = Some(actor);
        closest_distance = distance;
    }
    game.require_entity_mut(&entity).enemy = closest.clone();
    let mut wanted = body.angles;
    let target = closest.as_ref().and_then(|actor| game.host.bodies().read(actor));
    if let (Some(closest), Some(target)) = (closest.clone(), target) {
        if game.require_entity(&entity).spawnflags & 0x20000 == 0 {
            let authored = game.require_entity(&entity).authored_target();
            game.use_targets(&authored, Some(&closest), false);
            game.require_entity_mut(&entity).spawnflags |= 0x20000;
        }
        if !game.host.actors().is_live(&entity) {
            return;
        }
        let offset = game
            .rerelease
            .q64_eyes
            .get(&entity)
            .expect("func_eye source state missing")
            .eye_position;
        let axes = angle_vectors(body.angles);
        let eye = add3(
            body.origin,
            add3(
                add3(scale3(axes.forward, offset.x), scale3(axes.right, offset.y)),
                scale3(axes.up, offset.z),
            ),
        );
        wanted = vector_angles(normalize3(sub3(target.origin, eye)));
        let wait = game.require_entity(&entity).wait;
        let timestamp = game.now() + wait;
        let record = game.require_entity_mut(&entity);
        record.frame = 2;
        record.timestamp = timestamp;
    } else if game.require_entity(&entity).timestamp <= game.now() {
        let neutral = game
            .rerelease
            .q64_eyes
            .get(&entity)
            .expect("func_eye source state missing")
            .neutral_angles;
        wanted = neutral;
        let record = game.require_entity_mut(&entity);
        record.frame = 0;
    }
    let speed = game.require_entity(&entity).speed;
    let mut moved = game.body_of(entity.clone());
    moved.angles = vec3(
        turn_angle(f64::from(body.angles.x), f64::from(wanted.x), speed) as f32,
        turn_angle(f64::from(body.angles.y), f64::from(wanted.y), speed) as f32,
        body.angles.z,
    );
    game.write_body(entity.clone(), &moved, true);
    game.show(entity.clone());
    let frame = game.host.frame_seconds();
    game.schedule(entity, frame, eye_think);
}

/// Camera use (`cameraUse`).
fn camera_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    let music = {
        let record = game.require_entity(&entity);
        number_field(&record.spawn, "sounds", 0.0)
    };
    if music != 0.0 {
        game.host_emit(Q2PresentationEvent::Music {
            track: music.to_string(),
        });
    }
    if game.require_entity(&entity).target.is_empty() {
        return;
    }
    let target_name = game.require_entity(&entity).target.clone();
    let Some(target) = game.pick_target(&target_name) else {
        return;
    };
    let origin = game.body_of(entity.clone()).origin;
    {
        let record = game.require_entity_mut(&entity);
        record.goal = Some(target.clone());
        record.activator = activator.clone();
    }
    let source = activator.as_ref().and_then(|actor| game.entity(actor).cloned());
    if let Some(source) = source {
        if game.players.states.contains_key(source.actor.id()) {
            let body = game.body_of(source.actor.id().clone());
            let dummy = game.create("target_camera_dummy", std::collections::BTreeMap::new());
            {
                let record = game.require_entity_mut(&dummy);
                record.owner = Some(source.actor.id().clone());
                record.clip_mask = source.clip_mask;
                record.model = source.model.clone();
                record.model2 = source.model2.clone();
                record.skin = source.skin;
                record.frame = source.frame;
                record.render_flags = 1;
            }
            game.write_body(dummy.clone(), &body, false);
            game.set_solid(dummy.clone(), Q2Solid::Box);
            game.set_motion_kind(dummy.clone(), Q2MotionKind::Step);
            game.link_actor(dummy.clone());
            game.show(dummy.clone());
            game.rerelease.q64_dummies.insert(
                dummy.clone(),
                super::checkpoint::Q2RereleaseQ64DummyState {
                    fade_remaining: 0.0,
                    fade_duration: 0.0,
                    fading: false,
                },
            );
            game.schedule(dummy.clone(), 0.1, dummy_think);
            game.require_entity_mut(&entity).enemy = Some(dummy);
        }
    }
    let distance = f64::from(length3(sub3(game.body_of(target).origin, origin)));
    let speed = game.require_entity(&entity).speed;
    let angles = look_at(&entity, game, origin, vec3(0.0, 0.0, 0.0));
    game.rerelease.q64_cameras.insert(
        entity.clone(),
        super::checkpoint::Q2RereleaseQ64CameraState {
            remaining: distance,
            distance,
            speed,
            angles,
        },
    );
    let started = game.now();
    game.players.intermission = Q2Intermission::Intermission {
        map: String::new(),
        landmark: None,
        started,
        exit: false,
    };
    rerelease_move_to_camera(game, origin, angles, true);
    let hackflags = {
        let record = game.require_entity(&entity);
        number_field(&record.spawn, "hackflags", 0.0) as i32
    };
    if hackflags & 128 != 0 {
        rerelease_end_of_unit(game);
    }
    let wait = game.require_entity(&entity).wait;
    game.schedule(entity, wait, camera_think);
}

/// Camera think (`cameraThink`).
fn camera_think(entity: ActorId, game: &mut Q2GameServices) {
    if !game.rerelease.q64_cameras.contains_key(&entity) {
        panic!("target_camera source state missing");
    }
    let hackflags = {
        let record = game.require_entity(&entity);
        number_field(&record.spawn, "hackflags", 0.0) as i32
    };
    let skip = hackflags & 64 != 0
        && game.now() > 2.0
        && game
            .players
            .states
            .values()
            .any(|player| player.connected && player.buttons != 0);
    let goal = game.require_entity(&entity).goal.clone();
    let target = goal.as_ref().and_then(|actor| game.entity(actor).cloned());
    if skip || target.is_none() {
        if !game.require_entity(&entity).killtarget.is_empty() {
            if let Some(enemy) = game.require_entity(&entity).enemy.clone() {
                if game.entity(&enemy).is_some() {
                    game.remove_actor(enemy);
                }
            }
            game.players.intermission = Q2Intermission::Playing;
            game.rerelease.intermission_camera_set = true;
            let killtarget = game.require_entity(&entity).killtarget.clone();
            let activator = game.require_entity(&entity).activator.clone();
            for destination in game.targets(&killtarget) {
                if let Some(use_) = game.require_entity(&destination).use_ {
                    use_(destination, game, Some(entity.clone()), activator.clone());
                }
            }
            rerelease_finish_camera(game);
        }
        game.cancel_actor(entity);
        return;
    }
    let target = target.expect("camera target checked");
    let frame = game.host.frame_seconds();
    {
        let state = game
            .rerelease
            .q64_cameras
            .get_mut(&entity)
            .expect("target_camera source state missing");
        state.remaining -= state.speed * frame * 0.8;
    }
    let remaining = game
        .rerelease
        .q64_cameras
        .get(&entity)
        .expect("target_camera source state missing")
        .remaining;
    if remaining <= 0.0 {
        let target_record = game.require_entity(target.actor.id()).clone();
        if number_field(&target_record.spawn, "hackflags", 0.0) as i32 & 2 != 0 {
            let enemy = game.require_entity(&entity).enemy.clone();
            let fade = enemy
                .as_ref()
                .is_some_and(|actor| game.rerelease.q64_dummies.contains_key(actor));
            if let Some(enemy) = enemy {
                if fade {
                    game.host_emit(Q2PresentationEvent::EntityEvent {
                        actor: enemy.clone(),
                        event: 6,
                    });
                    if let Some(state) = game.rerelease.q64_dummies.get_mut(&enemy) {
                        state.fading = true;
                        state.fade_remaining = target_record.wait;
                        state.fade_duration = target_record.wait;
                    }
                }
            }
        }
        let target_origin = game.body_of(target.actor.id().clone()).origin;
        let mut moved = game.body_of(entity.clone());
        moved.origin = target_origin;
        game.write_body(entity.clone(), &moved, true);
        let next = if target_record.target.is_empty() {
            None
        } else {
            game.pick_target(&target_record.target)
        };
        game.require_entity_mut(&entity).goal = next.clone();
        if let Some(next) = next {
            let speed = game.require_entity(&next).speed;
            let distance = f64::from(length3(sub3(
                game.body_of(next).origin,
                game.body_of(entity.clone()).origin,
            )));
            let state = game
                .rerelease
                .q64_cameras
                .get_mut(&entity)
                .expect("target_camera source state missing");
            state.speed = if speed == 0.0 { 55.0 } else { speed };
            state.distance = distance;
            state.remaining = distance;
        }
        game.schedule(entity, target_record.wait, camera_think);
        return;
    }
    let distance = game
        .rerelease
        .q64_cameras
        .get(&entity)
        .expect("target_camera source state missing")
        .distance;
    let fraction = 1.0 - remaining / distance;
    let origin = add3(
        game.body_of(entity.clone()).origin,
        scale3(
            sub3(
                game.body_of(target.actor.id().clone()).origin,
                game.body_of(entity.clone()).origin,
            ),
            fraction as f32,
        ),
    );
    if let Some(enemy) = game.require_entity(&entity).enemy.clone() {
        if game
            .rerelease
            .q64_dummies
            .get(&enemy)
            .map(|state| state.fading)
            .unwrap_or(false)
        {
            (rerelease_hooks(game).emit)(
                game,
                Q2RereleaseEvent::Alpha {
                    actor: enemy,
                    alpha: (1.0f64 / 255.0).max(fraction),
                },
            );
        }
    }
    let angles = look_at(
        &entity,
        game,
        origin,
        game.rerelease
            .q64_cameras
            .get(&entity)
            .expect("target_camera source state missing")
            .angles,
    );
    game.rerelease
        .q64_cameras
        .get_mut(&entity)
        .expect("target_camera source state missing")
        .angles = angles;
    rerelease_move_to_camera(game, origin, angles, false);
    game.schedule(entity, frame, camera_think);
}

/// Dummy think (`dummyThink`).
fn dummy_think(entity: ActorId, game: &mut Q2GameServices) {
    let owner = game.require_entity(&entity).owner.clone();
    let source = owner.as_ref().and_then(|actor| game.entity(actor).cloned());
    if source.is_none() || !game.rerelease.q64_dummies.contains_key(&entity) {
        game.remove_actor(entity);
        return;
    }
    let source = source.expect("dummy source checked");
    q2_rerelease_dummy_animation(entity.clone(), source.actor.id().clone(), game);
    game.show(entity.clone());
    let fading = game
        .rerelease
        .q64_dummies
        .get(&entity)
        .expect("dummy state checked")
        .fading;
    if fading {
        let state = game
            .rerelease
            .q64_dummies
            .get_mut(&entity)
            .expect("dummy state checked");
        state.fade_remaining = 0.0f64.max(state.fade_remaining - 0.1);
        let alpha = if state.fade_duration == 0.0 {
            0.0
        } else {
            state.fade_remaining / state.fade_duration
        };
        (rerelease_hooks(game).emit)(
            game,
            Q2RereleaseEvent::Alpha {
                actor: entity.clone(),
                alpha: (1.0f64 / 255.0).max(alpha),
            },
        );
    }
    game.schedule(entity, 0.1, dummy_think);
}

/// Rerelease Q64 callbacks (`Q2RereleaseQ64::callbacks`).
pub fn rerelease_q64_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("rr.func_eye_setup", eye_setup);
    callbacks.think.insert("rr.func_eye_think", eye_think);
    callbacks.think.insert("rr.func_spinning_think", spinning_think);
    callbacks.think.insert("rr.update_target_camera", camera_think);
    callbacks.think.insert("rr.target_camera_dummy_think", dummy_think);
    callbacks.use_.insert("rr.use_target_camera", camera_use);
    callbacks
}

/// Rerelease Q64 spawn (`Q2RereleaseQ64::spawn`).
pub fn rerelease_q64_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    spawn_q64(entity, game)
}

/// Rerelease Q64 entities (`Q2RereleaseQ64`).
#[derive(Debug, Clone, Copy)]
pub struct Q2RereleaseQ64 {
    /// Rerelease players.
    pub players: Q2RereleasePlayers,
    /// Rerelease hooks.
    pub hooks: Q2RereleaseHooks,
}

impl Q2RereleaseQ64 {
    /// Read the Q64 callbacks.
    pub fn callbacks(&self) -> Q2CallbackDefinitions {
        rerelease_q64_callbacks()
    }

    /// Spawn Q64 entities.
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        let _ = self;
        spawn_q64(entity, game)
    }
}

/// Spawn Q64 entities.
fn spawn_q64(entity: ActorId, game: &mut Q2GameServices) -> bool {
    let classname = game.require_entity(&entity).classname.clone();
    if classname == "target_camera" {
        if game.options.mode == Q2Mode::Deathmatch {
            game.remove_actor(entity);
        } else {
            let record = game.require_entity_mut(&entity);
            record.visible = false;
            record.server_flags |= 1;
            record.use_ = Some(camera_use);
        }
        return true;
    }
    if classname != "func_eye" && classname != "func_spinning" {
        return false;
    }
    game.set_solid(entity.clone(), Q2Solid::Brush);
    game.set_motion_kind(entity.clone(), Q2MotionKind::Push);
    if classname == "func_spinning" {
        {
            let record = game.require_entity_mut(&entity);
            if record.speed == 0.0 {
                record.speed = 100.0;
            }
            if record.damage == 0.0 {
                record.damage = 2.0;
            }
            record.timestamp = 0.0;
        }
        let frame = game.host.frame_seconds();
        game.schedule(entity.clone(), frame, spinning_think);
    } else {
        let radius = {
            let record = game.require_entity(&entity);
            number_field(&record.spawn, "radius", 0.0)
        };
        let frame = game.host.frame_seconds();
        {
            let record = game.require_entity_mut(&entity);
            record.damage_radius = if radius == 0.0 { 512.0 } else { radius };
            record.speed = if record.speed == 0.0 { 45.0 } else { record.speed } * frame;
            record.wait = 1.0;
        }
        let (eye_position, vision_cone) = {
            let record = game.require_entity(&entity);
            (
                vector_field(&record.spawn, "eye_position"),
                number_field(&record.spawn, "vision_cone", 0.0),
            )
        };
        let neutral = game.body_of(entity.clone()).angles;
        game.rerelease.q64_eyes.insert(
            entity.clone(),
            super::checkpoint::Q2RereleaseQ64EyeState {
                neutral_angles: neutral,
                eye_position,
                vision_cone: if vision_cone == 0.0 { 0.5 } else { vision_cone },
            },
        );
        let pathtarget = game
            .require_entity(&entity)
            .spawn
            .values
            .get("pathtarget")
            .cloned()
            .unwrap_or_default();
        if !pathtarget.is_empty() {
            game.schedule(entity.clone(), 0.1, eye_setup);
        } else {
            let state = game
                .rerelease
                .q64_eyes
                .get(&entity)
                .copied()
                .expect("func_eye source state missing");
            let axes = angle_vectors(state.neutral_angles);
            game.require_entity_mut(&entity).movedir = axes.forward;
            let point = state.eye_position;
            let position = add3(
                add3(scale3(axes.forward, point.x), scale3(axes.right, point.y)),
                scale3(axes.up, point.z),
            );
            game.rerelease
                .q64_eyes
                .get_mut(&entity)
                .expect("func_eye source state missing")
                .eye_position = position;
            game.schedule(entity.clone(), 0.1, eye_think);
        }
    }
    game.link_actor(entity);
    true
}

/// Capture Q64 state (`capture`).
pub fn q64_capture(game: &Q2GameServices) -> super::checkpoint::Q2RereleaseQ64Checkpoint {
    let mut eyes: Vec<ActorId> = game.rerelease.q64_eyes.keys().cloned().collect();
    super::sort_rerelease_actors(&mut eyes);
    let mut cameras: Vec<ActorId> = game.rerelease.q64_cameras.keys().cloned().collect();
    super::sort_rerelease_actors(&mut cameras);
    let mut dummies: Vec<ActorId> = game.rerelease.q64_dummies.keys().cloned().collect();
    super::sort_rerelease_actors(&mut dummies);
    super::checkpoint::Q2RereleaseQ64Checkpoint {
        eyes: eyes
            .into_iter()
            .map(|actor| super::checkpoint::Q2RereleaseQ64EyeCheckpoint {
                actor: SavedActorId::from(&actor),
                state: game
                    .rerelease
                    .q64_eyes
                    .get(&actor)
                    .copied()
                    .expect("Q64 eye is missing"),
            })
            .collect(),
        cameras: cameras
            .into_iter()
            .map(|actor| super::checkpoint::Q2RereleaseQ64CameraCheckpoint {
                actor: SavedActorId::from(&actor),
                state: game
                    .rerelease
                    .q64_cameras
                    .get(&actor)
                    .copied()
                    .expect("Q64 camera is missing"),
            })
            .collect(),
        dummies: dummies
            .into_iter()
            .map(|actor| super::checkpoint::Q2RereleaseQ64DummyCheckpoint {
                actor: SavedActorId::from(&actor),
                state: game
                    .rerelease
                    .q64_dummies
                    .get(&actor)
                    .copied()
                    .expect("Q64 dummy is missing"),
            })
            .collect(),
    }
}

/// Restore Q64 state (`restore`).
pub fn q64_restore(game: &mut Q2GameServices, checkpoint: &super::checkpoint::Q2RereleaseQ64Checkpoint) {
    use crate::q2::foundation::checkpoint::restore_q2_actor;
    game.rerelease.q64_eyes.clear();
    for entry in &checkpoint.eyes {
        let actor = restore_q2_actor(game, entry.actor).id().clone();
        game.rerelease.q64_eyes.insert(actor, entry.state);
    }
    game.rerelease.q64_cameras.clear();
    for entry in &checkpoint.cameras {
        let actor = restore_q2_actor(game, entry.actor).id().clone();
        game.rerelease.q64_cameras.insert(actor, entry.state);
    }
    game.rerelease.q64_dummies.clear();
    for entry in &checkpoint.dummies {
        let actor = restore_q2_actor(game, entry.actor).id().clone();
        game.rerelease.q64_dummies.insert(actor, entry.state);
    }
}
