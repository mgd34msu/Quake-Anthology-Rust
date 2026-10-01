//! Q2 environmental triggers (`src/content/q2/base/entities/triggers.ts`).
//!
//! Quake II g_trigger.c environmental triggers.

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{scale3, vec3, Vec3};

use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::checkpoint::restore_q2_actor;
use crate::q2::foundation::entity_services::js_round;
use crate::q2::foundation::fields::{integer_field, movedir, number_field};
use crate::q2::foundation::host::{Q2Edition, Q2GameServices, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop};
use crate::q2::support::contracts::TouchContact;

use super::types::Q2BaseEntityHooks;

/// Wind-sound throttle checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WindTimeEntry {
    /// Throttled actor.
    pub actor: SavedActorId,
    /// Throttle expiry.
    pub until: f64,
}

/// Write velocity and ground (`velocity`).
fn set_velocity(game: &mut Q2GameServices, actor: ActorId, value: Vec3, ground: Option<Option<ActorId>>) {
    let owned = match game.host.actors().resolve_owned(&actor) {
        Some(owned) => owned,
        None => return,
    };
    let mut body = match game.host.bodies().read(&actor) {
        Some(body) => body,
        None => return,
    };
    body.velocity = value;
    if let Some(ground) = ground {
        body.ground = ground;
    }
    game.host.bodies().write(&owned, &body);
    if game.entity(&actor).is_some() {
        let motion = game.require_entity(&actor).motion;
        game.set_motion_kind(actor, motion);
    }
}

/// Shared trigger setup (`init`).
fn init_trigger(actor: ActorId, game: &mut Q2GameServices) {
    let angles = game.body_of(actor.clone()).angles;
    let movedir = movedir(angles);
    {
        let entity = game.require_entity_mut(&actor);
        entity.visible = false;
        entity.movedir = movedir;
    }
    let mut moved = game.body_of(actor.clone());
    moved.angles = vec3(0.0, 0.0, 0.0);
    game.write_body(actor.clone(), &moved, false);
    game.set_solid(actor, Q2Solid::Trigger);
}

/// Read registered hooks.
fn hooks(game: &Q2GameServices) -> Q2BaseEntityHooks {
    game.base_entities
        .hooks
        .expect("Q2 base entity hooks are not registered")
}

/// Trigger push touch (`pushTouch`).
fn trigger_push_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let other = contact.other.clone();
    let is_grenade = game.entity(&other).is_some_and(|entity| entity.classname == "grenade");
    let health = game.host.combat().read(&other).map_or(0.0, |state| state.health);
    if is_grenade || health > 0.0 {
        let entity = game.require_entity(&actor);
        let push = scale3(entity.movedir, (entity.speed * 10.0) as f32);
        set_velocity(game, other.clone(), push, None);
        if game.host.is_player(&other) {
            (hooks(game).player_push)(other.clone(), push);
            let now = game.host.now();
            let throttled = game.base_entities.wind_times.get(&other).copied().unwrap_or(0.0);
            if throttled < now {
                game.base_entities.wind_times.insert(other.clone(), now + 1.5);
                if let Some(body) = game.host.bodies().read(&other) {
                    game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
                        actor: Some(other.clone()),
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
    if game.require_entity(&actor).spawnflags & 1 != 0 {
        game.remove_actor(actor);
    }
}

/// Trigger hurt touch (`hurtTouch`).
fn trigger_hurt_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let other = contact.other.clone();
    let body = match game.host.bodies().read(&other) {
        Some(body) => body,
        None => return,
    };
    let can_take = game
        .host
        .combat()
        .read(&other)
        .is_some_and(|state| state.can_take_damage);
    let now = game.host.now();
    let entity = game.require_entity(&actor);
    if !can_take || entity.timestamp > now {
        return;
    }
    let (spawnflags, damage) = (entity.spawnflags, entity.damage);
    let frame_seconds = game.host.frame_seconds();
    game.require_entity_mut(&actor).timestamp = now + if spawnflags & 16 != 0 { 1.0 } else { frame_seconds };
    if spawnflags & 4 == 0 && (js_round(now / frame_seconds) as i64) % 10 == 0 {
        game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(other.clone()),
            origin: body.origin,
            path: "world/electro.wav".to_string(),
            channel: 0,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Once,
            loop_owner: None,
        }));
    }
    let zero = vec3(0.0, 0.0, 0.0);
    game.damage(
        other,
        actor.clone(),
        Some(actor),
        damage,
        damage,
        zero,
        body.origin,
        zero,
        31,
        if spawnflags & 8 != 0 { 32 } else { 0 },
        None,
    );
}

/// Trigger gravity touch (`gravityTouch`).
fn trigger_gravity_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let other = contact.other.clone();
    let gravity = game.require_entity(&actor).gravity;
    if game.entity(&other).is_some() {
        let motion = {
            let entity = game.require_entity_mut(&other);
            entity.gravity = gravity;
            entity.motion
        };
        game.set_motion_kind(other.clone(), motion);
    }
    (hooks(game).set_actor_gravity)(other, gravity);
}

/// Trigger monster-jump touch (`jumpTouch`).
fn trigger_monsterjump_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let other = contact.other.clone();
    let body = match game.host.bodies().read(&other) {
        Some(body) => body,
        None => return,
    };
    if !game.host.is_monster(&other) {
        return;
    }
    if let Some(other_entity) = game.entity(&other) {
        if other_entity.flags & 3 != 0 || other_entity.server_flags & 8 != 0 {
            return;
        }
    }
    let entity = game.require_entity(&actor);
    let velocity = vec3(
        entity.movedir.x * entity.speed as f32,
        entity.movedir.y * entity.speed as f32,
        if body.ground.is_none() {
            body.velocity.z
        } else {
            entity.movedir.z
        },
    );
    set_velocity(game, other, velocity, Some(None));
}

/// Trigger hurt use (`hurtUse`).
fn trigger_hurt_use(actor: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let entity = game.require_entity_mut(&actor);
    entity.solid = if entity.solid == Q2Solid::None {
        Q2Solid::Trigger
    } else {
        Q2Solid::None
    };
    let solid = entity.solid;
    if entity.spawnflags & 2 == 0 {
        entity.use_ = None;
    }
    game.set_solid(actor, solid);
}

/// Trigger callbacks (`Q2EnvironmentalTriggers[callbacks]`).
pub fn trigger_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.touch.insert("trigger_push_touch", trigger_push_touch as _);
    callbacks.touch.insert("hurt_touch", trigger_hurt_touch as _);
    callbacks
        .touch
        .insert("trigger_gravity_touch", trigger_gravity_touch as _);
    callbacks
        .touch
        .insert("trigger_monsterjump_touch", trigger_monsterjump_touch as _);
    callbacks.use_.insert("hurt_use", trigger_hurt_use as _);
    callbacks
}

/// Capture wind-sound throttles (`Q2EnvironmentalTriggers[capture]`).
pub fn capture_triggers(game: &mut Q2GameServices) -> Vec<Q2WindTimeEntry> {
    let mut entries = Vec::new();
    let actors: Vec<ActorId> = game.base_entities.wind_times.keys().cloned().collect();
    for actor in actors {
        if !game.host.actors().is_live(&actor) {
            continue;
        }
        if let Some(until) = game.base_entities.wind_times.get(&actor).copied() {
            entries.push(Q2WindTimeEntry {
                actor: SavedActorId::from(&actor),
                until,
            });
        }
    }
    entries
}

/// Restore wind-sound throttles (`Q2EnvironmentalTriggers[restore]`).
pub fn restore_triggers(game: &mut Q2GameServices, checkpoint: &[Q2WindTimeEntry]) {
    game.base_entities.wind_times = std::collections::HashMap::new();
    for entry in checkpoint {
        let actor = restore_q2_actor(game, entry.actor).id().clone();
        game.base_entities.wind_times.insert(actor, entry.until);
    }
}

/// Spawn a trigger entity (`Q2EnvironmentalTriggers[spawn]`).
pub fn spawn_trigger(actor: ActorId, game: &mut Q2GameServices) -> bool {
    let classname = game.require_entity(&actor).classname.clone();
    match classname.as_str() {
        "trigger_push" => {
            init_trigger(actor.clone(), game);
            let entity = game.require_entity_mut(&actor);
            if entity.speed == 0.0 {
                entity.speed = 1000.0;
            }
            entity.touch = Some(trigger_push_touch as _);
            true
        }
        "trigger_hurt" => {
            init_trigger(actor.clone(), game);
            {
                let entity = game.require_entity_mut(&actor);
                if entity.damage == 0.0 {
                    entity.damage = 5.0;
                }
                entity.timestamp = 0.0;
            }
            let spawnflags = game.require_entity(&actor).spawnflags;
            if spawnflags & 1 != 0 {
                game.set_solid(actor.clone(), Q2Solid::None);
            }
            let entity = game.require_entity_mut(&actor);
            if entity.spawnflags & 2 != 0 {
                entity.use_ = Some(trigger_hurt_use as _);
            }
            entity.touch = Some(trigger_hurt_touch as _);
            true
        }
        "trigger_gravity" => {
            let has_gravity = game.require_entity(&actor).spawn.values.contains_key("gravity");
            if !has_gravity {
                game.host.diagnostic("trigger_gravity without gravity set");
                game.remove_actor(actor);
                return true;
            }
            init_trigger(actor.clone(), game);
            let gravity = if game.options.edition == Q2Edition::Classic {
                f64::from(integer_field(&game.require_entity(&actor).spawn.clone(), "gravity", 0))
            } else {
                number_field(&game.require_entity(&actor).spawn.clone(), "gravity", 0.0)
            };
            let entity = game.require_entity_mut(&actor);
            entity.gravity = gravity;
            entity.touch = Some(trigger_gravity_touch as _);
            true
        }
        "trigger_monsterjump" => {
            {
                let entity = game.require_entity_mut(&actor);
                if entity.speed == 0.0 {
                    entity.speed = 200.0;
                }
            }
            let angles = game.body_of(actor.clone()).angles;
            if angles.y == 0.0 {
                let mut moved = game.body_of(actor.clone());
                moved.angles = vec3(angles.x, 360.0, angles.z);
                game.write_body(actor.clone(), &moved, false);
            }
            init_trigger(actor.clone(), game);
            let height = number_field(&game.require_entity(&actor).spawn.clone(), "height", 0.0);
            let entity = game.require_entity_mut(&actor);
            entity.movedir.z = (if height == 0.0 { 200.0 } else { height }) as f32;
            entity.touch = Some(trigger_monsterjump_touch as _);
            true
        }
        _ => false,
    }
}
