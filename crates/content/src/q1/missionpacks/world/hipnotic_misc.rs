//! Hipnotic sounds, explosions, rubble, and earthquakes
//! (`src/content/q1/missionpacks/world/hipnotic-misc.ts`).
//!
//! hipmisc.qc / hip_expl.qc / hiprubbl.qc / hipquake.qc entity behavior.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{vadd, Q1Effect, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, POINT, ZERO};
use crate::q1::{q1_error, Q1Error};

use super::common::{later, number};

/// Emit an entity sound with stored volume and speed attenuation (`sound`).
fn sound(game: &mut Q1EntityServices, id: &ActorId, path: &str, channel: Q1SoundChannel) {
    let (volume, attenuation) = game
        .entity(id)
        .map(|entity| (entity.number_or("volume", 1.0), entity.speed))
        .unwrap_or((1.0, 1.0));
    let id = id.clone();
    game.host.emit(Q1Event::Sound {
        origin: None,
        actor: id,
        path: path.to_string(),
        channel,
        attenuation,
        volume,
    });
}

/// Play (or toggle) a sound entity (`playSound`).
fn play_sound(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (mut path, spawnflags, impulse) = game
        .entity(id)
        .map(|entity| (entity.text("noise"), entity.spawnflags, entity.number("impulse")))
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if spawnflags & 1 != 0 {
        let active = game
            .entity(id)
            .map(|entity| entity.number("sound_state"))
            .unwrap_or(0.0)
            == 0.0;
        game.update_entity(id, |entity| {
            number(entity, "sound_state", if active { 1.0 } else { 0.0 })
        })?;
        if !active {
            path = "misc/null.wav".to_string();
        }
    }
    let channel = match impulse as i32 {
        0 => Q1SoundChannel::Auto,
        1 => Q1SoundChannel::Weapon,
        2 => Q1SoundChannel::Voice,
        3 => Q1SoundChannel::Item,
        4 => Q1SoundChannel::Body,
        5 => Q1SoundChannel::Raw(5),
        6 => Q1SoundChannel::Raw(6),
        _ => Q1SoundChannel::Raw(7),
    };
    sound(game, id, &path, channel);
    Ok(())
}

/// Play a sound entity from a use dispatch.
fn play_sound_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    play_sound(game, id)
}

/// Replay a periodic sound entity from a scheduled dispatch.
fn play_sound_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (delay, wait) = game
        .entity(id)
        .map(|entity| (entity.delay, entity.wait))
        .unwrap_or((0.0, 0.0));
    let first = delay.max(wait * game.host.random());
    later(game, id, first, "hip:play_sound")?;
    play_sound(game, id)
}

/// Configure a sound entity, scheduling periodic replays (`soundSpawn`).
fn sound_spawn(game: &mut Q1EntityServices, id: &ActorId, periodic: bool) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        if entity.number("volume") == 0.0 {
            number(entity, "volume", 1.0);
        }
        entity.speed = if entity.speed == 0.0 {
            1.0
        } else if entity.speed == -1.0 {
            0.0
        } else {
            entity.speed
        };
        if entity.spawnflags & 1 != 0 && entity.number("impulse") == 0.0 {
            number(entity, "impulse", 7.0);
        }
    })?;
    let use_name = game.named.use_callback("hip:play_sound")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))?;
    if periodic {
        game.update_entity(id, |entity| {
            if entity.wait == 0.0 {
                entity.wait = 20.0;
            }
            if entity.delay == 0.0 {
                entity.delay = 2.0;
            }
        })?;
        let (delay, wait) = game
            .entity(id)
            .map(|entity| (entity.delay, entity.wait))
            .unwrap_or((2.0, 20.0));
        let first = delay.max(wait * game.host.random());
        later(game, id, first, "hip:play_sound")?;
    }
    Ok(())
}

/// Spawn a triggered or periodic sound entity.
fn spawn_sound(thunder: bool, triggered: bool) -> fn(&mut Q1EntityServices, &ActorId) -> Result<(), Q1Error> {
    if thunder && triggered {
        spawn_random_thunder_triggered
    } else if thunder {
        spawn_random_thunder
    } else if triggered {
        spawn_play_sound_triggered
    } else {
        spawn_play_sound
    }
}

fn spawn_play_sound(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    sound_spawn(game, id, true)
}

fn spawn_play_sound_triggered(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    sound_spawn(game, id, false)
}

fn spawn_random_thunder(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity
            .fields
            .insert("noise".to_string(), "ambience/thunder1.wav".to_string());
    })?;
    sound_spawn(game, id, true)?;
    game.update_entity(id, |entity| number(entity, "impulse", 6.0))
}

fn spawn_random_thunder_triggered(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity
            .fields
            .insert("noise".to_string(), "ambience/thunder1.wav".to_string());
    })?;
    sound_spawn(game, id, false)?;
    game.update_entity(id, |entity| number(entity, "impulse", 6.0))
}

/// Turn an entity into an explosion sprite (`becomeExplosion`).
fn become_explosion(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
        entity.touch = None;
        entity.model = "progs/s_explod.spr".to_string();
        entity.frame = 0;
    })?;
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(ZERO),
            ..Default::default()
        },
    )?;
    game.link(id)?;
    later(game, id, 0.1, "base:explosion_frame")
}

/// Detonate a single explosion (`explode`).
fn explode(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (activator, damage, owner, spawnflags) = game
        .entity(id)
        .map(|entity| {
            (
                entity.activator.clone(),
                entity.damage,
                entity.owner.clone(),
                entity.spawnflags,
            )
        })
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    game.use_targets(id, activator.as_ref())?;
    sound(
        game,
        id,
        if damage < 120.0 {
            "misc/shortexp.wav"
        } else {
            "misc/longexpl.wav"
        },
        Q1SoundChannel::Auto,
    );
    let id_copy = id.clone();
    game.radius_damage(&id_copy, owner.as_ref(), damage, Some(&id_copy), None, "");
    if spawnflags & 1 != 0 {
        let origin = game.body(id)?.origin;
        game.effect_simple(Q1Effect::Explosion, origin);
    }
    become_explosion(game, id)
}

/// Detonate a single explosion from a use dispatch, honoring delay.
fn explode_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let activator = activator.cloned();
    game.update_entity(id, |entity| entity.activator = activator)?;
    let delay = game.entity(id).map(|entity| entity.delay).unwrap_or(0.0);
    if delay == 0.0 {
        return explode(game, id);
    }
    game.update_entity(id, |entity| entity.delay = 0.0)?;
    later(game, id, delay, "hip:explode")
}

/// Scatter one random explosion inside the volume (`multiExplode`).
fn multi_explode(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let wait = game.entity(id).map(|entity| entity.wait).unwrap_or(0.0);
    later(game, id, wait, "hip:multi_explode")?;
    let state = game
        .entity(id)
        .map(|entity| entity.number("explosion_state"))
        .unwrap_or(0.0);
    if state == 0.0 {
        let duration = game.entity(id).map(|entity| entity.number("duration")).unwrap_or(0.0);
        let time = game.time;
        game.update_entity(id, |entity| {
            number(entity, "explosion_state", 1.0);
            number(entity, "duration", time + duration);
        })?;
        let activator = game.entity(id).and_then(|entity| entity.activator.clone());
        game.use_targets(id, activator.as_ref())?;
    }
    let duration = game.entity(id).map(|entity| entity.number("duration")).unwrap_or(0.0);
    if game.time > duration {
        return game.remove(id);
    }
    let body = game.body(id)?;
    let min = vadd(body.origin, body.bounds.min);
    let max = vadd(body.origin, body.bounds.max);
    let explosion = game.create("hip_explosion", None, None)?;
    let (owner, damage, spawnflags, volume, speed) = game
        .entity(id)
        .map(|entity| {
            (
                entity.owner.clone(),
                entity.damage,
                entity.spawnflags,
                entity.number("volume"),
                entity.speed,
            )
        })
        .unwrap_or((None, 0.0, 0, 0.0, 1.0));
    game.update_entity(&explosion, |entity| {
        entity.owner = owner.clone();
        entity.damage = damage;
    })?;
    let jitter = Vec3 {
        x: game.host.random() as f32,
        y: game.host.random() as f32,
        z: game.host.random() as f32,
    };
    game.set_origin(
        &explosion,
        Vec3 {
            x: (f64::from(min.x) + f64::from(jitter.x) * f64::from(max.x - min.x)) as f32,
            y: (f64::from(min.y) + f64::from(jitter.y) * f64::from(max.y - min.y)) as f32,
            z: (f64::from(min.z) + f64::from(jitter.z) * f64::from(max.z - min.z)) as f32,
        },
    )?;
    let boom = explosion.clone();
    game.host.emit(Q1Event::Sound {
        origin: None,
        actor: boom,
        path: "misc/shortexp.wav".to_string(),
        channel: Q1SoundChannel::Voice,
        attenuation: speed,
        volume,
    });
    let id_copy = id.clone();
    game.radius_damage(&explosion, owner.as_ref(), damage, Some(&id_copy), None, "");
    if spawnflags & 1 != 0 {
        let origin = game.body(&explosion)?.origin;
        game.effect_simple(Q1Effect::Explosion, origin);
    }
    become_explosion(game, &explosion)
}

/// Scatter explosions from a use dispatch, honoring delay.
fn multi_explode_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let activator = activator.cloned();
    game.update_entity(id, |entity| entity.activator = activator)?;
    let delay = game.entity(id).map(|entity| entity.delay).unwrap_or(0.0);
    if delay == 0.0 {
        return multi_explode(game, id);
    }
    game.update_entity(id, |entity| entity.delay = 0.0)?;
    later(game, id, delay, "hip:multi_explode")
}

/// Spawn a `func_exploder` or `func_multi_exploder`.
fn spawn_exploder(multi: bool) -> fn(&mut Q1EntityServices, &ActorId) -> Result<(), Q1Error> {
    if multi {
        spawn_multi_exploder
    } else {
        spawn_single_exploder
    }
}

fn spawn_single_exploder(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_exploder_inner(game, id, false)
}

fn spawn_multi_exploder(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_exploder_inner(game, id, true)
}

fn spawn_exploder_inner(game: &mut Q1EntityServices, id: &ActorId, multi: bool) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.damage = if entity.damage == 0.0 {
            120.0
        } else {
            entity.damage.max(0.0)
        };
        if entity.speed == 0.0 {
            entity.speed = 1.0;
        }
        if entity.number("volume") == 0.0 {
            number(entity, "volume", if multi { 0.5 } else { 1.0 });
        }
        if multi {
            entity.model.clear();
            entity.movement = Q1MoveType::None;
            if entity.wait == 0.0 {
                entity.wait = 0.25;
            }
            if entity.number("duration") == 0.0 {
                number(entity, "duration", 1.0);
            }
            number(entity, "explosion_state", 0.0);
        }
    })?;
    let use_name = game
        .named
        .use_callback(if multi { "hip:multi_explode" } else { "hip:explode" })?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))
}

/// Damage whoever touches flying rubble (`hip:rubble_touch`).
fn rubble_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let (ltime, pausetime) = game
        .entity(id)
        .map(|entity| (entity.number("ltime"), entity.number("pausetime")))
        .unwrap_or((0.0, 0.0));
    if ltime < pausetime
        || !game
            .host
            .combat
            .read(other)
            .is_some_and(|combat| combat.can_take_damage)
    {
        return Ok(());
    }
    let (owner, id_copy) = (game.entity(id).and_then(|entity| entity.owner.clone()), id.clone());
    game.damage(
        other,
        Some(&id_copy),
        owner.as_ref(),
        10.0,
        &crate::q1::foundation::entity_services::Q1DamageParams::default(),
    );
    game.sound(id, "zombie/z_hit.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    game.update_entity(id, |entity| number(entity, "pausetime", ltime + 0.1))
}

/// Throw rubble pieces (`hip:rubble_use`).
fn rubble_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let (count, origin) = game
        .entity(id)
        .map(|entity| (entity.count, game.body(id)))
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let origin = origin?.origin;
    let pieces = 1.max(count as i32);
    for _ in 0..pieces {
        let stored = game.entity(id).map(|entity| entity.number("cnt")).unwrap_or(0.0);
        let which = if stored == 0.0 {
            (1.0 + 3.0 * game.host.random()).floor()
        } else {
            stored
        };
        let piece = game.create("hip_rubble", None, None)?;
        let model = if which == 1.0 {
            "progs/rubble1.mdl"
        } else if which == 2.0 {
            "progs/rubble3.mdl"
        } else {
            "progs/rubble2.mdl"
        };
        let velocity = Vec3 {
            x: (70.0 * (game.host.random() * 2.0 - 1.0)) as f32,
            y: (70.0 * (game.host.random() * 2.0 - 1.0)) as f32,
            z: (140.0 + 70.0 * game.host.random()) as f32,
        };
        let spin = Vec3 {
            x: (game.host.random() * 600.0) as f32,
            y: (game.host.random() * 600.0) as f32,
            z: (game.host.random() * 600.0) as f32,
        };
        game.update_entity(&piece, |piece| {
            piece.model = model.to_string();
            piece.movement = Q1MoveType::Bounce;
            piece.solid = Q1Solid::Bbox;
            piece.angular_velocity = spin;
        })?;
        game.set_body(
            &piece,
            &BodyPatch {
                origin: Some(origin),
                velocity: Some(velocity),
                bounds: Some(POINT),
                ..Default::default()
            },
        )?;
        let touch_name = game.named.touch("hip:rubble_touch")?;
        let time = game.time;
        game.update_entity(&piece, |piece| {
            piece.touch = Some(touch_name);
            number(piece, "ltime", time);
        })?;
        game.update_entity(id, |entity| number(entity, "pausetime", time))?;
        let lifetime = 13.0 + game.host.random() * 10.0;
        later(game, &piece, lifetime, "SUB_Remove")?;
        game.link(&piece)?;
    }
    Ok(())
}

/// Start an earthquake on use (`hip:earthquake`).
fn earthquake_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let world = game.world.clone();
    let Some(world) = world else {
        return Ok(());
    };
    let (current, damage) = (
        game.entity(&world)
            .map(|world| world.number("hip:earthquake"))
            .unwrap_or(0.0),
        game.entity(id).map(|entity| entity.damage).unwrap_or(0.0),
    );
    let until = current.max(game.time + damage);
    game.update_entity(&world, |world| number(world, "hip:earthquake", until))
}

/// Spawn a looping teleport effect.
fn teleport_effect_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let origin = game.body(id)?.origin;
    game.effect_simple(Q1Effect::Teleport, origin);
    game.sound_simple(id, "misc/r_tele1.wav")
}

/// Spawn a looping ambient sound emitter.
fn spawn_ambient(path: &'static str) -> fn(&mut Q1EntityServices, &ActorId) -> Result<(), Q1Error> {
    match path {
        "humming" => spawn_ambient_humming,
        "rushing" => spawn_ambient_rushing,
        "runwater" => spawn_ambient_running_water,
        "fanblow" => spawn_ambient_fan,
        "waterfal" => spawn_ambient_waterfall,
        _ => spawn_ambient_riftpower,
    }
}

fn ambient_emit(game: &mut Q1EntityServices, id: &ActorId, path: &str) -> Result<(), Q1Error> {
    let origin = game.body(id)?.origin;
    let stored = game.entity(id).map(|entity| entity.number("volume")).unwrap_or(0.0);
    game.host.emit(Q1Event::Ambient {
        origin,
        path: format!("ambient/{path}.wav"),
        volume: if stored == 0.0 { 0.5 } else { stored },
        attenuation: 3.0,
    });
    Ok(())
}

fn spawn_ambient_humming(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    ambient_emit(game, id, "humming")
}
fn spawn_ambient_rushing(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    ambient_emit(game, id, "rushing")
}
fn spawn_ambient_running_water(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    ambient_emit(game, id, "runwater")
}
fn spawn_ambient_fan(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    ambient_emit(game, id, "fanblow")
}
fn spawn_ambient_waterfall(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    ambient_emit(game, id, "waterfal")
}
fn spawn_ambient_riftpower(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    ambient_emit(game, id, "riftpowr")
}

/// Spawn an `effect_teleport`.
fn spawn_teleport_effect(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let use_name = game.named.use_callback("hip:teleport_effect")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))
}

/// Spawn an `info_command`.
fn spawn_command(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let message = game.entity(id).map(|entity| entity.message.clone()).unwrap_or_default();
    if message.is_empty() {
        return Ok(());
    }
    game.host.emit(Q1Event::ServerCommand { text: message });
    Ok(())
}

/// Spawn a `func_rubble` variant with its fixed piece kind.
fn spawn_rubble_variant(index: i32) -> fn(&mut Q1EntityServices, &ActorId) -> Result<(), Q1Error> {
    match index {
        1 => spawn_rubble1,
        2 => spawn_rubble2,
        3 => spawn_rubble3,
        _ => spawn_rubble,
    }
}

fn rubble_variant(game: &mut Q1EntityServices, id: &ActorId, index: i32) -> Result<(), Q1Error> {
    let use_name = game.named.use_callback("hip:rubble_use")?;
    game.update_entity(id, |entity| {
        number(entity, "cnt", f64::from(index));
        entity.use_callback = Some(use_name);
    })
}

fn spawn_rubble(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    rubble_variant(game, id, 0)
}
fn spawn_rubble1(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    rubble_variant(game, id, 1)
}
fn spawn_rubble2(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    rubble_variant(game, id, 2)
}
fn spawn_rubble3(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    rubble_variant(game, id, 3)
}

/// Spawn a `func_earthquake`.
fn spawn_earthquake(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        if entity.damage == 0.0 {
            entity.damage = 0.8;
        }
    })?;
    let use_name = game.named.use_callback("hip:earthquake")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))?;
    if let Some(world) = game.world.clone() {
        game.update_entity(&world, |world| number(world, "hip:quakeactive", 0.0))?;
    }
    Ok(())
}

/// Fan explosions out over a volume (`multiExplosion`).
#[allow(clippy::too_many_arguments)]
pub fn multi_explosion(
    game: &mut Q1EntityServices,
    _source: &ActorId,
    origin: Vec3,
    radius: f64,
    damage: f64,
    duration: f64,
    pause: f64,
    volume: f64,
) -> Result<ActorId, Q1Error> {
    let entity = game.create("hip_multi_explosion", None, None)?;
    let owner = game.world.clone();
    game.update_entity(&entity, |entity| {
        entity.damage = damage;
        entity.wait = pause;
        entity.owner = owner;
        number(entity, "duration", duration);
        number(entity, "volume", volume);
    })?;
    let radius = radius as f32;
    game.set_body(
        &entity,
        &BodyPatch {
            origin: Some(origin),
            bounds: Some(qa_core::math::Bounds {
                min: Vec3 {
                    x: -radius,
                    y: -radius,
                    z: -radius,
                },
                max: Vec3 {
                    x: radius,
                    y: radius,
                    z: radius,
                },
            }),
            ..Default::default()
        },
    )?;
    multi_explode(game, &entity)?;
    Ok(entity)
}

/// Shake a grounded actor during a Hipnotic earthquake (`earthquakeAfterPhysics`).
pub fn earthquake_after_physics(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let world = game.world.clone();
    let Some(world) = world else {
        return Ok(());
    };
    let owned = game.host.actors.resolve_owned(actor);
    let body = game.host.bodies.read(actor);
    let (Some(owned), Some(body)) = (owned, body) else {
        return Ok(());
    };
    let (until, active) = game
        .entity(&world)
        .map(|world| (world.number("hip:earthquake"), world.number("hip:quakeactive")))
        .unwrap_or((0.0, 0.0));
    if until > game.time {
        if active == 0.0 {
            game.sound(actor, "misc/quake.wav", Q1SoundChannel::Voice, 0.0, 1.0)?;
            game.update_entity(&world, |world| number(world, "hip:quakeactive", 1.0))?;
        }
        if body.ground.is_some() {
            let mut next = body.clone();
            next.velocity = vadd(
                next.velocity,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: (game.host.random() * 150.0) as f32,
                },
            );
            game.host.bodies.write(&owned, &next)?;
        }
    } else if active == 1.0 {
        game.sound(actor, "misc/quakeend.wav", Q1SoundChannel::Voice, 0.0, 1.0)?;
        game.update_entity(&world, |world| number(world, "hip:quakeactive", 0.0))?;
    }
    Ok(())
}

/// Register Hipnotic misc entities (`registerHipnoticMisc`).
pub fn register_hipnotic_misc(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "hip:play_sound",
        Q1CallbackHandlers {
            use_callback: Some(play_sound_use),
            action: Some(play_sound_action),
            ..Default::default()
        },
    )?;
    for (classname, thunder, triggered) in [
        ("play_sound", false, false),
        ("play_sound_triggered", false, true),
        ("random_thunder", true, false),
        ("random_thunder_triggered", true, true),
    ] {
        game.register_spawn(classname, spawn_sound(thunder, triggered))?;
    }
    for (classname, path) in [
        ("ambient_humming", "humming"),
        ("ambient_rushing", "rushing"),
        ("ambient_running_water", "runwater"),
        ("ambient_fan_blowing", "fanblow"),
        ("ambient_waterfall", "waterfal"),
        ("ambient_riftpower", "riftpowr"),
    ] {
        game.register_spawn(classname, spawn_ambient(path))?;
    }
    game.register_spawn("info_command", spawn_command)?;
    game.named.register(
        "hip:teleport_effect",
        Q1CallbackHandlers {
            use_callback: Some(teleport_effect_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("effect_teleport", spawn_teleport_effect)?;
    game.named.register(
        "hip:explode",
        Q1CallbackHandlers {
            action: Some(explode),
            use_callback: Some(explode_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:multi_explode",
        Q1CallbackHandlers {
            action: Some(multi_explode),
            use_callback: Some(multi_explode_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_exploder", spawn_exploder(false))?;
    game.register_spawn("func_multi_exploder", spawn_exploder(true))?;
    game.named.register(
        "hip:rubble_touch",
        Q1CallbackHandlers {
            touch: Some(rubble_touch),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:rubble_use",
        Q1CallbackHandlers {
            use_callback: Some(rubble_use),
            ..Default::default()
        },
    )?;
    for (classname, index) in [
        ("func_rubble", 0),
        ("func_rubble1", 1),
        ("func_rubble2", 2),
        ("func_rubble3", 3),
    ] {
        game.register_spawn(classname, spawn_rubble_variant(index))?;
    }
    game.named.register(
        "hip:earthquake",
        Q1CallbackHandlers {
            use_callback: Some(earthquake_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_earthquake", spawn_earthquake)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    fn null_action(_game: &mut Q1EntityServices, _id: &ActorId) -> Result<(), Q1Error> {
        Ok(())
    }

    fn register_for_test(game: &mut Q1EntityServices) {
        register_hipnotic_misc(game).expect("register");
        game.named
            .register(
                "base:explosion_frame",
                Q1CallbackHandlers {
                    action: Some(null_action),
                    ..Default::default()
                },
            )
            .expect("explosion frame");
    }

    #[test]
    fn exploder_use_becomes_explosion() {
        let mut game = test_game();
        register_for_test(&mut game);
        let id = game.create("func_exploder", None, None).expect("exploder");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_use(&id, "hip:explode", None, None).expect("use");
        let entity = game.entity(&id).cloned().expect("entity");
        assert_eq!(entity.model, "progs/s_explod.spr");
        assert_eq!(entity.think.as_deref(), Some("base:explosion_frame"));
    }

    #[test]
    fn earthquake_shakes_grounded_actors() {
        let mut game = test_game();
        register_for_test(&mut game);
        let world = game.create("worldspawn", None, None).expect("world");
        game.world = Some(world.clone());
        let until = game.time + 5.0;
        game.update_entity(&world, |world| number(world, "hip:earthquake", until))
            .expect("shake");
        let player = game.create("player", None, None).expect("player");
        game.set_body(
            &player,
            &BodyPatch {
                ground: Some(Some(world.clone())),
                ..Default::default()
            },
        )
        .expect("ground");
        earthquake_after_physics(&mut game, &player).expect("shake");
        assert_eq!(game.entity(&world).expect("world").number("hip:quakeactive"), 1.0);
        assert!(f64::from(game.host.bodies.read(&player).expect("body").velocity.z) > 0.0);
    }

    #[test]
    fn rubble_use_throws_numbered_pieces() {
        let mut game = test_game();
        register_for_test(&mut game);
        let id = game.create("func_rubble1", None, None).expect("rubble");
        game.update_entity(&id, |entity| entity.count = 2.0).expect("count");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_use(&id, "hip:rubble_use", None, None).expect("use");
        let pieces: Vec<ActorId> = game
            .entity_ids()
            .into_iter()
            .filter(|id| game.entity(id).is_some_and(|entity| entity.classname == "hip_rubble"))
            .collect();
        assert_eq!(pieces.len(), 2);
        for piece in pieces {
            assert_eq!(game.entity(&piece).expect("piece").model, "progs/rubble1.mdl");
        }
    }
}
