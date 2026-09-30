//! Q2 base targets (`src/content/q2/base/entities/targets.ts`).
//!
//! Remaining Quake II g_target.c targets.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, add3, normalize3, scale3, sub3, vec3};

use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::{integer_field, movedir};
use crate::q2::foundation::host::{
    Q2BeamEvent, Q2Edition, Q2EffectEvent, Q2GameServices, Q2Mode, Q2PresentationEvent,
    Q2SpawnFields, Q2TraceRequest,
};
use crate::q2::foundation::scenery::kill_q2_box;
use crate::q2::support::contracts::{TraceContact, TraceHit};

use super::types::Q2BaseEntityHooks;

/// Laser direction flag (`0x80000000`).
const LASER_DIRTY: i32 = i32::MIN;

/// Read registered hooks.
fn hooks(game: &Q2GameServices) -> Q2BaseEntityHooks {
    game.base_entities
        .hooks
        .expect("Q2 base entity hooks are not registered")
}

/// Target laser think (`q2TargetLaserThink`).
fn target_laser_think(actor: ActorId, game: &mut Q2GameServices) {
    let zero = vec3(0.0, 0.0, 0.0);
    let count = if game.require_entity(&actor).spawnflags & LASER_DIRTY != 0 {
        8
    } else {
        4
    };
    let origin = game.body_of(actor.clone()).origin;
    let enemy = game.require_entity(&actor).enemy.clone();
    let target_body = enemy
        .as_ref()
        .and_then(|enemy| game.host.bodies().read(enemy));
    if let Some(target) = target_body {
        let center = scale3(
            add3(target.bounds.min, target.bounds.max),
            0.5,
        );
        let direction = normalize3(sub3(add3(target.origin, center), origin));
        let entity = game.require_entity_mut(&actor);
        if direction.x != entity.movedir.x
            || direction.y != entity.movedir.y
            || direction.z != entity.movedir.z
        {
            entity.spawnflags |= LASER_DIRTY;
        }
        entity.movedir = direction;
    }
    let movedir = game.require_entity(&actor).movedir;
    let end = add3(origin, scale3(movedir, 2048.0));
    let mut start = origin;
    let mut terminal: Vec3;
    let mut ignore = Some(actor.clone());
    let mut passed: Vec<ActorId> = Vec::new();
    loop {
        let trace = game.host.trace(&Q2TraceRequest {
            start,
            end,
            bounds: None,
            ignore: ignore.clone(),
            mask: 0x6000001,
            exclude: passed.clone(),
        });
        terminal = trace.end;
        let hit = match &trace.hit {
            TraceHit::Actor { actor } => Some(actor.clone()),
            _ => None,
        };
        if let Some(hit_actor) = hit.clone() {
            let can_take = game
                .host
                .combat()
                .read(&hit_actor)
                .is_some_and(|state| state.can_take_damage);
            let immune = game
                .entity(&hit_actor)
                .is_some_and(|entity| entity.laser_immune);
            if can_take && !immune {
                let self_id = game.require_entity(&actor).actor.id().clone();
                let activator = game.require_entity(&actor).activator.clone();
                let damage = game.require_entity(&actor).damage;
                game.damage(
                    hit_actor.clone(),
                    self_id,
                    activator,
                    damage,
                    1.0,
                    movedir,
                    trace.end,
                    zero,
                    30,
                    4,
                    None,
                );
            }
        }
        let stop = match hit.clone() {
            None => true,
            Some(hit_actor) => {
                !game.host.is_monster(&hit_actor) && !game.host.is_player(&hit_actor)
            }
        };
        if stop {
            let spawnflags = game.require_entity(&actor).spawnflags;
            if trace.fraction < 1.0 && spawnflags & LASER_DIRTY != 0 {
                game.require_entity_mut(&actor).spawnflags &= !LASER_DIRTY;
                let direction = match &trace.contact {
                    TraceContact::Plane { plane } => plane.normal,
                    _ => zero,
                };
                let color = game.require_entity(&actor).skin & 255;
                game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
                    effect: "q2:laser-sparks".to_string(),
                    origin: trace.end,
                    direction,
                    count,
                    color,
                }));
            }
            break;
        }
        if let Some(hit_actor) = hit {
            passed.push(hit_actor.clone());
            ignore = Some(hit_actor);
            start = trace.end;
        }
    }
    let entity = game.require_entity(&actor);
    let (frame, skin) = (entity.frame, entity.skin);
    game.host.emit(Q2PresentationEvent::Beam(Q2BeamEvent {
        actor: actor.clone(),
        start: origin,
        end: terminal,
        width: f64::from(frame),
        color: skin,
        visible: true,
    }));
    let frame_seconds = game.host.frame_seconds();
    game.schedule(actor, frame_seconds, target_laser_think as _);
}

/// Target laser on (`laserOn`).
fn target_laser_on(actor: ActorId, game: &mut Q2GameServices) {
    {
        let entity = game.require_entity_mut(&actor);
        if entity.activator.is_none() {
            let id = entity.actor.id().clone();
            entity.activator = Some(id);
        }
        entity.spawnflags |= LASER_DIRTY | 1;
        entity.visible = true;
    }
    target_laser_think(actor, game);
}

/// Target laser off (`laserOff`).
fn target_laser_off(actor: ActorId, game: &mut Q2GameServices) {
    let origin = game.body_of(actor.clone()).origin;
    {
        let entity = game.require_entity_mut(&actor);
        entity.spawnflags &= !1;
        entity.visible = false;
    }
    game.cancel_actor(actor.clone());
    game.host.emit(Q2PresentationEvent::Visibility {
        actor: actor.clone(),
        visible: false,
    });
    let entity = game.require_entity(&actor);
    let (frame, skin) = (entity.frame, entity.skin);
    game.host.emit(Q2PresentationEvent::Beam(Q2BeamEvent {
        actor,
        start: origin,
        end: origin,
        width: f64::from(frame),
        color: skin,
        visible: false,
    }));
}

/// Target laser use (`laserUse`).
fn target_laser_use(
    actor: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    activator: Option<ActorId>,
) {
    game.require_entity_mut(&actor).activator = activator;
    if game.require_entity(&actor).spawnflags & 1 != 0 {
        target_laser_off(actor, game);
    } else {
        target_laser_on(actor, game);
    }
}

/// Target laser start (`laserStart`).
fn target_laser_start(actor: ActorId, game: &mut Q2GameServices) {
    let spawnflags = game.require_entity(&actor).spawnflags;
    let skin = if spawnflags & 2 != 0 {
        0xf2f2f0f0u32 as i32
    } else if spawnflags & 4 != 0 {
        0xd0d1d2d3u32 as i32
    } else if spawnflags & 8 != 0 {
        0xf3f3f1f1u32 as i32
    } else if spawnflags & 16 != 0 {
        0xdcdddedfu32 as i32
    } else if spawnflags & 32 != 0 {
        0xe0e1e2e3u32 as i32
    } else {
        0
    };
    {
        let entity = game.require_entity_mut(&actor);
        entity.frame = if spawnflags & 64 != 0 { 16 } else { 4 };
        entity.render_flags |= 0xa0;
        entity.skin = skin;
    }
    if game.require_entity(&actor).enemy.is_none() {
        let target = game.require_entity(&actor).target.clone();
        if !target.is_empty() {
            let enemy = game.targets(&target).first().cloned();
            if enemy.is_none() {
                game.host
                    .diagnostic(&format!("target_laser has missing target {target}"));
            }
            game.require_entity_mut(&actor).enemy = enemy;
        } else {
            let angles = game.body_of(actor.clone()).angles;
            let dir = movedir(angles);
            game.require_entity_mut(&actor).movedir = dir;
            let mut moved = game.body_of(actor.clone());
            moved.angles = vec3(0.0, 0.0, 0.0);
            game.write_body(actor.clone(), &moved, false);
        }
    }
    {
        let entity = game.require_entity_mut(&actor);
        if entity.damage == 0.0 {
            entity.damage = 1.0;
        }
    }
    let mut moved = game.body_of(actor.clone());
    moved.bounds.min = vec3(-8.0, -8.0, -8.0);
    moved.bounds.max = vec3(8.0, 8.0, 8.0);
    game.write_body(actor.clone(), &moved, true);
    game.require_entity_mut(&actor).use_ = Some(target_laser_use as _);
    if game.require_entity(&actor).spawnflags & 1 != 0 {
        target_laser_on(actor, game);
    } else {
        target_laser_off(actor, game);
    }
}

/// Target lightramp think (`rampThink`).
fn target_lightramp_think(actor: ActorId, game: &mut Q2GameServices) {
    let enemy = game.require_entity(&actor).enemy.clone();
    let Some(enemy) = enemy else { return };
    if game.entity(&enemy).is_none() {
        return;
    }
    let entity = game.require_entity(&actor);
    let (timestamp, speed, spawnflags) = (entity.timestamp, entity.speed, entity.spawnflags);
    let (from, to) = (entity.movedir.x, entity.movedir.y);
    let now = game.host.now();
    let elapsed = now - timestamp;
    let level = (97.0 + f64::from(from) + elapsed / speed * f64::from(to - from)).trunc() as i32;
    let style = integer_field(&game.require_entity(&enemy).spawn.clone(), "style", 0);
    let pattern = char::from_u32((level & 255) as u32)
        .unwrap_or('\0')
        .to_string();
    game.host.emit(Q2PresentationEvent::LightStyle { style, pattern });
    if elapsed < speed {
        let frame_seconds = game.host.frame_seconds();
        game.schedule(actor, frame_seconds, target_lightramp_think as _);
    } else if spawnflags & 1 != 0 {
        let entity = game.require_entity_mut(&actor);
        entity.movedir = vec3(to, from, entity.movedir.z);
    }
}

/// Target lightramp use (`rampUse`).
fn target_lightramp_use(
    actor: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let enemy = game.require_entity(&actor).enemy.clone();
    let missing = match enemy.as_ref() {
        None => true,
        Some(enemy) => game.entity(enemy).is_none(),
    };
    if missing {
        let target = game.require_entity(&actor).target.clone();
        for candidate in game.targets(&target) {
            let classname = game.require_entity(&candidate).classname.clone();
            if classname == "light" {
                game.require_entity_mut(&actor).enemy = Some(candidate);
            } else {
                game.host.diagnostic(&format!(
                    "target_lightramp target {classname} is not a light"
                ));
            }
        }
        if game.require_entity(&actor).enemy.is_none() {
            game.host
                .diagnostic(&format!("target_lightramp missing target {target}"));
            game.remove_actor(actor);
            return;
        }
    }
    let now = game.host.now();
    game.require_entity_mut(&actor).timestamp = now;
    target_lightramp_think(actor, game);
}

/// Target temp-entity use (`Use_Target_Tent`).
fn use_target_tent(
    actor: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let zero = vec3(0.0, 0.0, 0.0);
    let style = integer_field(&game.require_entity(&actor).spawn.clone(), "style", 0);
    let origin = game.body_of(actor).origin;
    game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: format!("q2:temp-{}", style & 255),
        origin,
        direction: zero,
        count: 1,
        color: 0,
    }));
}

/// Target spawner use (`spawnerUse`).
fn use_target_spawner(
    actor: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let body = game.body_of(actor.clone());
    let target = game.require_entity(&actor).target.clone();
    let mut values = BTreeMap::new();
    values.insert("classname".to_string(), target.clone());
    values.insert(
        "origin".to_string(),
        format!("{} {} {}", body.origin.x, body.origin.y, body.origin.z),
    );
    values.insert(
        "angles".to_string(),
        format!("{} {} {}", body.angles.x, body.angles.y, body.angles.z),
    );
    let spawned = game.spawn(Q2SpawnFields {
        ordinal: -1,
        classname: target,
        values,
    });
    if !game.host.actors().is_live(&spawned) {
        return;
    }
    if let Some(owned) = game.host.actors().resolve_owned(&spawned) {
        game.host.bodies().unlink(&owned);
    }
    kill_q2_box(game, spawned.clone());
    game.link_actor(spawned.clone());
    let entity = game.require_entity(&actor);
    let (speed, movedir) = (entity.speed, entity.movedir);
    if speed != 0.0 {
        let mut moved = game.body_of(spawned.clone());
        moved.velocity = movedir;
        game.write_body(spawned.clone(), &moved, false);
        let motion = game.require_entity(&spawned).motion;
        game.set_motion_kind(spawned, motion);
    }
}

/// Target blaster use (`blasterUse`).
fn use_target_blaster(
    actor: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let entity = game.require_entity(&actor);
    let effects = if game.options.edition == Q2Edition::Classic {
        8
    } else if entity.spawnflags & 2 != 0 {
        0
    } else if entity.spawnflags & 1 != 0 {
        64
    } else {
        8
    };
    let (movedir, damage, speed) = (entity.movedir, entity.damage, entity.speed);
    let origin = game.body_of(actor.clone()).origin;
    (hooks(game).fire_blaster)(
        actor.clone(),
        game,
        origin,
        movedir,
        damage,
        speed,
        effects,
        false,
        33,
    );
    game.sound(&actor, "weapons/laser2.wav", 2, 1.0, 1.0);
}

/// Cross-level trigger use (`crosslevelUse`).
fn target_crosslevel_trigger_use(
    actor: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let spawnflags = game.require_entity(&actor).spawnflags;
    game.counters.server_flags |= spawnflags;
    game.remove_actor(actor);
}

/// Cross-level target think (`crosslevelThink`).
fn target_crosslevel_target_think(actor: ActorId, game: &mut Q2GameServices) {
    let entity = game.require_entity(&actor);
    let spawnflags = entity.spawnflags;
    if spawnflags == (game.counters.server_flags & 255 & spawnflags) {
        let authored = entity.authored_target();
        let id = entity.actor.id().clone();
        game.use_targets(&authored, Some(&id), false);
        game.remove_actor(actor);
    }
}

/// Earthquake think (`quakeThink`).
fn target_earthquake_think(actor: ActorId, game: &mut Q2GameServices) {
    let entity = game.require_entity(&actor);
    let (wait, speed, timestamp) = (entity.wait, entity.speed, entity.timestamp);
    let now = game.host.now();
    if wait < now {
        game.sound(&actor, "world/quake.wav", 0, 1.0, 0.0);
        game.require_entity_mut(&actor).wait = game.host.now() + 0.5;
    }
    for player in game.host.players() {
        let body = game.host.bodies().read(&player);
        let owned = game.host.actors().resolve_owned(&player);
        let (Some(body), Some(owned)) = (body, owned) else {
            continue;
        };
        if body.ground.is_none() {
            continue;
        }
        let mass = game
            .host
            .combat()
            .read(&player)
            .map_or(200.0, |state| state.mass);
        let mut moved = body.clone();
        moved.ground = None;
        moved.velocity = vec3(
            body.velocity.x + ((game.host.random() * 2.0 - 1.0) * 150.0) as f32,
            body.velocity.y + ((game.host.random() * 2.0 - 1.0) * 150.0) as f32,
            (speed * (100.0 / mass)) as f32,
        );
        game.host.bodies().write(&owned, &moved);
    }
    if game.host.now() < timestamp {
        let frame_seconds = game.host.frame_seconds();
        game.schedule(actor, frame_seconds, target_earthquake_think as _);
    }
}

/// Earthquake use (`quakeUse`).
fn target_earthquake_use(
    actor: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    activator: Option<ActorId>,
) {
    let count = game.require_entity(&actor).count;
    let now = game.host.now();
    {
        let entity = game.require_entity_mut(&actor);
        entity.timestamp = now + f64::from(count);
        entity.wait = 0.0;
        entity.activator = activator;
    }
    let frame_seconds = game.host.frame_seconds();
    game.schedule(actor, frame_seconds, target_earthquake_think as _);
}

/// Base target callbacks (`Q2BaseTargets[callbacks]`).
pub fn target_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("target_laser_think", target_laser_think as _);
    callbacks.think.insert("target_laser_start", target_laser_start as _);
    callbacks.think.insert("target_laser_on", target_laser_on as _);
    callbacks.think.insert("target_laser_off", target_laser_off as _);
    callbacks
        .think
        .insert("target_lightramp_think", target_lightramp_think as _);
    callbacks.think.insert(
        "target_crosslevel_target_think",
        target_crosslevel_target_think as _,
    );
    callbacks
        .think
        .insert("target_earthquake_think", target_earthquake_think as _);
    callbacks.use_.insert("target_laser_use", target_laser_use as _);
    callbacks
        .use_
        .insert("target_lightramp_use", target_lightramp_use as _);
    callbacks.use_.insert("Use_Target_Tent", use_target_tent as _);
    callbacks.use_.insert("use_target_spawner", use_target_spawner as _);
    callbacks.use_.insert("use_target_blaster", use_target_blaster as _);
    callbacks.use_.insert(
        "target_crosslevel_trigger_use",
        target_crosslevel_trigger_use as _,
    );
    callbacks
        .use_
        .insert("target_earthquake_use", target_earthquake_use as _);
    callbacks
}

/// Spawn a base target entity (`Q2BaseTargets[spawn]`).
pub fn spawn_target(actor: ActorId, game: &mut Q2GameServices) -> bool {
    let classname = game.require_entity(&actor).classname.clone();
    match classname.as_str() {
        "target_temp_entity" => {
            game.require_entity_mut(&actor).use_ = Some(use_target_tent as _);
            true
        }
        "target_spawner" => {
            game.require_entity_mut(&actor).visible = false;
            if game.require_entity(&actor).speed != 0.0 {
                let angles = game.body_of(actor.clone()).angles;
                let speed = game.require_entity(&actor).speed;
                let dir = scale3(movedir(angles), speed as f32);
                game.require_entity_mut(&actor).movedir = dir;
                let mut moved = game.body_of(actor.clone());
                moved.angles = vec3(0.0, 0.0, 0.0);
                game.write_body(actor.clone(), &moved, false);
            }
            game.require_entity_mut(&actor).use_ = Some(use_target_spawner as _);
            true
        }
        "target_blaster" => {
            {
                let entity = game.require_entity_mut(&actor);
                entity.visible = false;
                if entity.damage == 0.0 {
                    entity.damage = 15.0;
                }
                if entity.speed == 0.0 {
                    entity.speed = 1000.0;
                }
            }
            let angles = game.body_of(actor.clone()).angles;
            let dir = movedir(angles);
            game.require_entity_mut(&actor).movedir = dir;
            let mut moved = game.body_of(actor.clone());
            moved.angles = vec3(0.0, 0.0, 0.0);
            game.write_body(actor.clone(), &moved, false);
            game.require_entity_mut(&actor).use_ = Some(use_target_blaster as _);
            true
        }
        "target_crosslevel_trigger" => {
            let entity = game.require_entity_mut(&actor);
            entity.visible = false;
            entity.use_ = Some(target_crosslevel_trigger_use as _);
            true
        }
        "target_crosslevel_target" => {
            {
                let entity = game.require_entity_mut(&actor);
                entity.visible = false;
                if entity.delay == 0.0 {
                    entity.delay = 1.0;
                }
            }
            let delay = game.require_entity(&actor).delay;
            game.schedule(actor, delay, target_crosslevel_target_think as _);
            true
        }
        "target_laser" => {
            game.schedule(actor, 1.0, target_laser_start as _);
            true
        }
        "target_lightramp" => {
            let entity = game.require_entity(&actor);
            let message: Vec<char> = entity.message.chars().collect();
            let valid = message.len() == 2
                && message[0].is_ascii_lowercase()
                && message[1].is_ascii_lowercase()
                && message[0] != message[1]
                && !entity.target.is_empty()
                && game.options.mode != Q2Mode::Deathmatch;
            if !valid {
                if game.options.mode != Q2Mode::Deathmatch {
                    let message = entity.message.clone();
                    game.host
                        .diagnostic(&format!("Invalid target_lightramp {message}"));
                }
                game.remove_actor(actor);
                return true;
            }
            let movedir = vec3(
                (message[0] as u32 - 97) as f32,
                (message[1] as u32 - 97) as f32,
                0.0,
            );
            let entity = game.require_entity_mut(&actor);
            entity.movedir = movedir;
            entity.timestamp = 0.0;
            entity.use_ = Some(target_lightramp_use as _);
            true
        }
        "target_earthquake" => {
            let entity = game.require_entity_mut(&actor);
            entity.visible = false;
            if entity.count == 0 {
                entity.count = 5;
            }
            if entity.speed == 0.0 {
                entity.speed = 200.0;
            }
            entity.timestamp = 0.0;
            entity.wait = 0.0;
            entity.use_ = Some(target_earthquake_use as _);
            true
        }
        _ => false,
    }
}
