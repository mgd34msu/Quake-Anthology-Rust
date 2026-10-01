//! Q2 rerelease triggers (`src/content/q2/rerelease/triggers.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::{add3, scale3, vec3};

use super::killbox::kill_q2_rerelease_box;
use super::players::{rerelease_extra, Q2RereleasePlayers};
use super::rerelease_hooks;
use super::types::Q2RereleaseHooks;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::{movedir, number_field};
use crate::q2::foundation::host::{
    Q2EffectEvent, Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop,
    Q2TraceRequest,
};
use crate::q2::support::contracts::TouchContact;

/// Initialize a trigger volume (`init`).
fn init_trigger(entity: &ActorId, game: &mut Q2GameServices) {
    let angles = game.body_of(entity.clone()).angles;
    let record = game.require_entity_mut(entity);
    record.visible = false;
    record.server_flags |= 1;
    record.movedir = movedir(angles);
    let mut body = game.body_of(entity.clone());
    body.angles = vec3(0.0, 0.0, 0.0);
    game.write_body(entity.clone(), &body, false);
    game.set_solid(entity.clone(), Q2Solid::Trigger);
}

/// Toggle a trigger (`toggleUse`).
fn toggle_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let solid = if game.require_entity(&entity).solid == Q2Solid::None {
        Q2Solid::Trigger
    } else {
        Q2Solid::None
    };
    game.set_solid(entity.clone(), solid);
    game.link_actor(entity);
}

/// Fire a killbox (`killboxUse`).
fn killbox_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    game.rerelease.deadly_kill_box = game.require_entity(&entity).spawnflags & 2 != 0;
    game.set_solid(entity.clone(), Q2Solid::Trigger);
    game.link_actor(entity.clone());
    let exact = game.require_entity(&entity).spawnflags & 4 != 0;
    kill_q2_rerelease_box(entity.clone(), game, false, exact);
    game.set_solid(entity.clone(), Q2Solid::None);
    game.link_actor(entity);
    game.rerelease.deadly_kill_box = false;
}

/// Toggle a hurt trigger (`hurtUse`).
fn hurt_use(entity: ActorId, game: &mut Q2GameServices, other: Option<ActorId>, activator: Option<ActorId>) {
    toggle_use(entity.clone(), game, other, activator);
    if game.require_entity(&entity).spawnflags & 2 == 0 {
        game.require_entity_mut(&entity).use_ = None;
    }
}

/// Set world gravity (`worldGravityUse`).
fn world_gravity_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let gravity = game.require_entity(&entity).gravity;
    (rerelease_hooks(game).set_world_gravity)(game, gravity);
}

/// Schedule a sound effect (`soundFxUse`).
fn sound_fx_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let delay = game.require_entity(&entity).delay;
    game.schedule(entity, delay, sound_fx_think);
}

/// Play a sound effect (`soundFxThink`).
fn sound_fx_think(entity: ActorId, game: &mut Q2GameServices) {
    let record = game.require_entity(&entity).clone();
    game.sound(&entity, &record.noise, 2, record.volume, record.attenuation);
}

/// Push touch (`pushTouch`).
fn push_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let hooks = rerelease_hooks(game);
    if game.require_entity(&entity).spawnflags & 16 != 0
        && !(hooks.clip_trigger)(entity.clone(), contact.other.clone(), game)
    {
        return;
    }
    let owner = game.host.actors().resolve_owned(&contact.other);
    let body = game.host.bodies().read(&contact.other);
    let classname = game.entity(&contact.other).map(|target| target.classname.clone());
    let healthy = game
        .host
        .combat()
        .read(&contact.other)
        .map(|combat| combat.health)
        .unwrap_or(0.0)
        > 0.0;
    if let (Some(owner), Some(body)) = (owner.as_ref(), body.as_ref()) {
        if classname.as_deref() == Some("grenade") || healthy {
            let record = game.require_entity(&entity).clone();
            let velocity = scale3(record.movedir, (record.speed * 10.0) as f32);
            let mut moved = body.clone();
            moved.velocity = velocity;
            game.host.bodies().write(owner, &moved);
            if classname.is_some() {
                let motion = game.require_entity(&contact.other).motion;
                game.set_motion_kind(contact.other.clone(), motion);
            }
            if game.host.is_player(&contact.other) {
                if let Some(state) = game.players.states.get_mut(&contact.other) {
                    state.old_velocity = velocity;
                }
                (hooks.push_player)(contact.other.clone(), game, velocity);
                let noisy = game.require_entity(&entity).spawnflags & 4 == 0;
                let now = game.now();
                let wind_time = rerelease_extra(game, &contact.other).wind_sound_time;
                if noisy && wind_time < now {
                    game.rerelease
                        .states
                        .get_mut(&contact.other)
                        .expect("Q2 rerelease player is not admitted")
                        .wind_sound_time = now + 1.5;
                    game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
                        actor: Some(contact.other.clone()),
                        origin: body.origin,
                        path: "misc/windfly.wav".to_string(),
                        channel: 0,
                        volume: 1.0,
                        attenuation: 1.0,
                        reliable: false,
                        loop_: Q2SoundLoop::Once,
                        loop_owner: None,
                    }));
                }
            }
        }
    }
    if game.require_entity(&entity).spawnflags & 1 != 0 {
        game.remove_actor(entity);
    }
}

/// Push active phase (`pushActive`).
fn push_active(entity: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&entity).delay > game.now() {
        let body = game.body_of(entity.clone());
        let mut origin = add3(body.origin, scale3(add3(body.bounds.min, body.bounds.max), 0.5));
        let speed = game.require_entity(&entity).speed;
        for index in 0..10 {
            origin = add3(
                origin,
                vec3(0.0, 0.0, (speed * 0.01 * (index as f64 + game.random())) as f32),
            );
            let color = 0x74 + (game.random() * 8.0).floor() as i32;
            game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
                effect: "q2:tunnel-sparks".to_string(),
                origin,
                direction: vec3(0.0, 0.0, 0.0),
                count: 1,
                color,
            }));
        }
        game.schedule(entity, 0.1, push_active);
        return;
    }
    let wait = game.require_entity(&entity).wait;
    game.require_entity_mut(&entity).touch = None;
    let now = game.now();
    game.require_entity_mut(&entity).delay = now + 0.1 + wait;
    game.schedule(entity, 0.1, push_inactive);
}

/// Push inactive phase (`pushInactive`).
fn push_inactive(entity: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&entity).delay > game.now() {
        game.schedule(entity, 0.1, push_inactive);
        return;
    }
    let wait = game.require_entity(&entity).wait;
    game.require_entity_mut(&entity).touch = Some(push_touch);
    let now = game.now();
    game.require_entity_mut(&entity).delay = now + 0.1 + wait;
    game.schedule(entity, 0.1, push_active);
}

/// Hurt touch (`hurtTouch`).
fn hurt_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let Some(body) = game.host.bodies().read(&contact.other) else {
        return;
    };
    let player = game.host.is_player(&contact.other);
    let monster = game.host.is_monster(&contact.other);
    let damageable = game
        .host
        .combat()
        .read(&contact.other)
        .map(|combat| combat.can_take_damage)
        .unwrap_or(false);
    let target = game.entity(&contact.other).cloned();
    if !damageable
        || !player
            && !monster
            && !target.as_ref().map(|target| target.damageable_target).unwrap_or(false)
            && target.as_ref().map(|target| target.classname.as_str()) != Some("misc_explobox")
    {
        return;
    }
    let flags = game.require_entity(&entity).spawnflags;
    if flags & 32 != 0 && player || flags & 64 != 0 && monster || game.require_entity(&entity).timestamp > game.now() {
        return;
    }
    if flags & 128 != 0 && !(rerelease_hooks(game).clip_trigger)(entity.clone(), contact.other.clone(), game) {
        return;
    }
    let now = game.now();
    game.require_entity_mut(&entity).timestamp = now + if flags & 16 != 0 { 1.0 } else { 0.1 };
    if flags & 4 == 0 && game.rerelease.trigger_sound_times.get(&entity).copied().unwrap_or(0.0) < now {
        game.rerelease.trigger_sound_times.insert(entity.clone(), now + 1.0);
        let noise = game.require_entity(&entity).noise.clone();
        game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(contact.other.clone()),
            origin: body.origin,
            path: noise,
            channel: 0,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Once,
            loop_owner: None,
        }));
    }
    let damage = game.require_entity(&entity).damage;
    game.damage(
        contact.other,
        entity.clone(),
        Some(entity),
        damage,
        damage,
        vec3(0.0, 0.0, 0.0),
        body.origin,
        vec3(0.0, 0.0, 0.0),
        31,
        if flags & 8 != 0 { 32 } else { 0 },
        None,
    );
}

/// Gravity touch (`gravityTouch`).
fn gravity_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let gravity = game.require_entity(&entity).gravity;
    if game.require_entity(&entity).spawnflags & 4 != 0
        && !(rerelease_hooks(game).clip_trigger)(entity, contact.other.clone(), game)
    {
        return;
    }
    if game.entity(&contact.other).is_some() {
        game.require_entity_mut(&contact.other).gravity = gravity;
        let motion = game.require_entity(&contact.other).motion;
        game.set_motion_kind(contact.other.clone(), motion);
    }
    (rerelease_hooks(game).set_actor_gravity)(contact.other, game, gravity);
}

/// Monster jump touch (`monsterJumpTouch`).
fn monster_jump_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let target = game.entity(&contact.other).cloned();
    let body = game.host.bodies().read(&contact.other);
    let (Some(target), Some(body)) = (target, body) else {
        return;
    };
    if !game.host.is_monster(&contact.other) || target.flags & 3 != 0 || target.server_flags & 2 != 0 {
        return;
    }
    if game.require_entity(&entity).spawnflags & 4 != 0
        && !(rerelease_hooks(game).clip_trigger)(entity.clone(), contact.other.clone(), game)
    {
        return;
    }
    let grounded = body.ground.is_some()
        || game
            .host
            .trace(&Q2TraceRequest {
                start: body.origin,
                end: add3(body.origin, vec3(0.0, 0.0, -0.25)),
                bounds: Some(body.bounds),
                ignore: Some(target.actor.id().clone()),
                mask: target.clip_mask,
                exclude: Vec::new(),
            })
            .fraction
            < 1.0;
    let record = game.require_entity(&entity).clone();
    let mut moved = body.clone();
    moved.velocity = vec3(
        (f64::from(record.movedir.x) * record.speed) as f32,
        (f64::from(record.movedir.y) * record.speed) as f32,
        if grounded { record.movedir.z } else { body.velocity.z },
    );
    moved.ground = None;
    game.write_body(contact.other.clone(), &moved, false);
    game.set_motion_kind(contact.other, target.motion);
}

/// Rerelease trigger callbacks (`Q2RereleaseTriggers::callbacks`).
pub fn rerelease_trigger_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("rr.trigger_push_active", push_active);
    callbacks.think.insert("rr.trigger_push_inactive", push_inactive);
    callbacks.think.insert("rr.update_target_soundfx", sound_fx_think);
    callbacks.touch.insert("rr.trigger_push_touch", push_touch);
    callbacks.touch.insert("rr.hurt_touch", hurt_touch);
    callbacks.touch.insert("rr.trigger_gravity_touch", gravity_touch);
    callbacks
        .touch
        .insert("rr.trigger_monsterjump_touch", monster_jump_touch);
    callbacks.use_.insert("rr.trigger_gravity_use", toggle_use);
    callbacks.use_.insert("rr.hurt_use", hurt_use);
    callbacks.use_.insert("rr.use_target_gravity", world_gravity_use);
    callbacks.use_.insert("rr.use_target_soundfx", sound_fx_use);
    callbacks.use_.insert("rr.use_killbox", killbox_use);
    callbacks
}

/// Rerelease trigger spawn (`Q2RereleaseTriggers::spawn`).
pub fn rerelease_trigger_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    spawn_trigger(entity, game)
}

/// Rerelease triggers (`Q2RereleaseTriggers`).
#[derive(Debug, Clone, Copy)]
pub struct Q2RereleaseTriggers {
    /// Rerelease players.
    pub players: Q2RereleasePlayers,
    /// Rerelease hooks.
    pub hooks: Q2RereleaseHooks,
}

impl Q2RereleaseTriggers {
    /// Read the trigger callbacks.
    pub fn callbacks(&self) -> Q2CallbackDefinitions {
        rerelease_trigger_callbacks()
    }

    /// Spawn trigger entities.
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        let _ = self;
        spawn_trigger(entity, game)
    }
}

/// Spawn trigger entities.
fn spawn_trigger(entity: ActorId, game: &mut Q2GameServices) -> bool {
    let classname = game.require_entity(&entity).classname.clone();
    match classname.as_str() {
        "func_killbox" => {
            let record = game.require_entity_mut(&entity);
            record.visible = false;
            record.server_flags |= 1;
            record.use_ = Some(killbox_use);
            game.set_solid(entity, Q2Solid::None);
            true
        }
        "trigger_push" => {
            init_trigger(&entity, game);
            {
                let record = game.require_entity_mut(&entity);
                if record.speed == 0.0 {
                    record.speed = 1000.0;
                }
                record.touch = Some(push_touch);
            }
            if game.require_entity(&entity).spawnflags & 2 != 0 {
                let wait = {
                    let record = game.require_entity_mut(&entity);
                    if record.wait == 0.0 {
                        record.wait = 10.0;
                    }
                    record.wait
                };
                let now = game.now();
                game.require_entity_mut(&entity).delay = now + 0.1 + wait;
                game.schedule(entity.clone(), 0.1, push_active);
            }
            if !game.require_entity(&entity).targetname.is_empty() {
                game.require_entity_mut(&entity).use_ = Some(toggle_use);
                if game.require_entity(&entity).spawnflags & 8 != 0 {
                    game.set_solid(entity.clone(), Q2Solid::None);
                }
            } else if game.require_entity(&entity).spawnflags & 8 != 0 {
                let record = game.require_entity_mut(&entity);
                record.server_flags = 0;
                record.touch = None;
                record.visible = true;
                game.set_solid(entity.clone(), Q2Solid::Brush);
                game.set_motion_kind(entity.clone(), Q2MotionKind::Push);
            }
            game.link_actor(entity);
            true
        }
        "trigger_hurt" => {
            init_trigger(&entity, game);
            {
                let record = game.require_entity_mut(&entity);
                if record.damage == 0.0 {
                    record.damage = 5.0;
                }
                record.touch = Some(hurt_touch);
                record.noise = "world/electro.wav".to_string();
            }
            if game.require_entity(&entity).spawnflags & 1 != 0 {
                game.set_solid(entity.clone(), Q2Solid::None);
            }
            if game.require_entity(&entity).spawnflags & 2 != 0 {
                game.require_entity_mut(&entity).use_ = Some(hurt_use);
            }
            game.link_actor(entity);
            true
        }
        "trigger_gravity" => {
            if !game.require_entity(&entity).spawn.values.contains_key("gravity") {
                game.host.diagnostic("trigger_gravity: no gravity set");
                game.remove_actor(entity);
                return true;
            }
            init_trigger(&entity, game);
            let gravity = {
                let record = game.require_entity(&entity);
                number_field(&record.spawn, "gravity", 0.0)
            };
            {
                let record = game.require_entity_mut(&entity);
                record.gravity = gravity;
                record.touch = Some(gravity_touch);
            }
            if game.require_entity(&entity).spawnflags & 3 != 0 {
                game.require_entity_mut(&entity).use_ = Some(toggle_use);
            }
            if game.require_entity(&entity).spawnflags & 2 != 0 {
                game.set_solid(entity.clone(), Q2Solid::None);
            }
            game.link_actor(entity);
            true
        }
        "trigger_monsterjump" => {
            if game.body_of(entity.clone()).angles.y == 0.0 {
                let mut body = game.body_of(entity.clone());
                body.angles.y = 360.0;
                game.write_body(entity.clone(), &body, false);
            }
            init_trigger(&entity, game);
            let height = {
                let record = game.require_entity(&entity);
                number_field(&record.spawn, "height", 0.0)
            };
            {
                let record = game.require_entity_mut(&entity);
                if record.speed == 0.0 {
                    record.speed = 200.0;
                }
                record.movedir.z = if height == 0.0 { 200.0 } else { height as f32 };
                record.touch = Some(monster_jump_touch);
            }
            if game.require_entity(&entity).spawnflags & 3 != 0 {
                game.require_entity_mut(&entity).use_ = Some(toggle_use);
            }
            if game.require_entity(&entity).spawnflags & 2 != 0 {
                game.set_solid(entity.clone(), Q2Solid::None);
            }
            game.link_actor(entity);
            true
        }
        "target_gravity" => {
            let gravity = {
                let record = game.require_entity(&entity);
                number_field(&record.spawn, "gravity", 0.0)
            };
            let record = game.require_entity_mut(&entity);
            record.gravity = gravity;
            record.use_ = Some(world_gravity_use);
            true
        }
        "target_soundfx" => {
            {
                let record = game.require_entity_mut(&entity);
                if record.volume == 0.0 {
                    record.volume = 1.0;
                }
                record.attenuation = if record.attenuation == -1.0 {
                    0.0
                } else if record.attenuation == 0.0 {
                    1.0
                } else {
                    record.attenuation
                };
            }
            let noise = {
                let record = game.require_entity(&entity);
                number_field(&record.spawn, "noise", 0.0) as i32
            };
            let sound = match noise {
                1 => Some("world/x_alarm.wav"),
                2 => Some("world/flyby1.wav"),
                4 => Some("world/amb12.wav"),
                5 => Some("world/amb17.wav"),
                7 => Some("world/bigpump2.wav"),
                _ => None,
            };
            match sound {
                Some(sound) => {
                    let record = game.require_entity_mut(&entity);
                    record.noise = sound.to_string();
                    record.use_ = Some(sound_fx_use);
                    true
                }
                None => {
                    let key = game
                        .require_entity(&entity)
                        .spawn
                        .values
                        .get("noise")
                        .cloned()
                        .unwrap_or_else(|| "0".to_string());
                    game.host.diagnostic(&format!("target_soundfx: unknown noise {key}"));
                    true
                }
            }
        }
        _ => false,
    }
}
