//! Rogue earthquakes, buzzsaws, and lightning trails
//! (`src/content/q1/missionpacks/world/rogue-hazards.ts`).
//!
//! earthq.qc / buzzsaw.qc / lightnin.qc entity behavior.

use qa_core::identity::{ActorId, same_actor};
use qa_core::math::Vec3;

use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{
    POINT, Q1BeamStyle, Q1Effect, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, Q1TraceRequest,
    ZERO, normalize, vadd, vscale, vsub,
};
use crate::q1::{Q1Error, q1_error};

use super::common::{later, number, trigger};

/// Shake a grounded actor (`rogueEarthquake`).
pub fn rogue_earthquake(
    game: &mut Q1EntityServices,
    actor: &ActorId,
    intensity: f64,
) -> Result<(), Q1Error> {
    let owned = game.host.actors.resolve_owned(actor);
    let body = game.host.bodies.read(actor);
    let (Some(owned), Some(body)) = (owned, body) else {
        return Ok(());
    };
    if body.ground.is_none() {
        return Ok(());
    }
    let mut next = body.clone();
    next.velocity = vadd(
        next.velocity,
        Vec3 {
            x: (game.host.random() * intensity * 2.0 - intensity) as f32,
            y: (game.host.random() * intensity * 2.0 - intensity) as f32,
            z: (game.host.random() * intensity * 2.0 - intensity) as f32,
        },
    );
    game.host.bodies.write(&owned, &next)?;
    Ok(())
}

/// Stop an earthquake, scheduling the next one.
fn quake_stop(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if let Some(world) = game.world.clone() {
        game.update_entity(&world, |world| {
            number(world, "rogue:earthquake_active", 0.0)
        })?;
    }
    let (spawnflags, wait) = game
        .entity(id)
        .map(|entity| (entity.spawnflags, entity.wait))
        .unwrap_or((0, 0.0));
    later(
        game,
        id,
        if spawnflags & 1 != 0 {
            game.host.random() * wait
        } else {
            wait
        },
        "rogue:quake_start",
    )
}

/// Rumble while an earthquake runs.
fn quake_rumble(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game
        .entity(id)
        .map(|entity| entity.attack_finished)
        .unwrap_or(0.0)
        < game.time
    {
        return quake_stop(game, id);
    }
    game.sound(id, "equake/rumble.wav", Q1SoundChannel::Voice, 0.0, 1.0)?;
    later(game, id, 1.0, "rogue:quake_rumble")
}

/// Start a buzzsaw flying or standing.
fn saw_start(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let touch_name = game.named.touch("rogue:saw_touch")?;
    game.update_entity(id, |entity| {
        entity.touch = Some(touch_name);
        entity.use_callback = None;
    })?;
    let target = game
        .entity(id)
        .map(|entity| entity.target.clone())
        .unwrap_or_default();
    let goal = game.find(&target).first().cloned();
    game.update_entity(id, |entity| {
        entity
            .references
            .insert("goalentity".to_string(), goal.clone());
        entity.references.insert("movetarget".to_string(), goal);
    })?;
    later(
        game,
        id,
        0.1,
        if target.is_empty() {
            "rogue:saw_stand"
        } else {
            "rogue:saw_fly"
        },
    )
}

/// Start a buzzsaw from a scheduled dispatch.
fn saw_start_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    saw_start(game, id, None, None)
}

/// Fire one lightning-trail segment.
fn trail_fire(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game
        .entity(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default()
        != "ltrail_end"
    {
        game.sound_simple(id, "weapons/lhit.wav")?;
        let target = game
            .entity(id)
            .map(|entity| entity.target.clone())
            .unwrap_or_default();
        let target = game
            .find(&target)
            .first()
            .cloned()
            .or_else(|| game.world.clone());
        let Some(target) = target.filter(|target| game.entity(target).is_some()) else {
            return Err(q1_error("Lightning trail requires worldspawn"));
        };
        let start = game.body(id)?.origin;
        let end = game.body(&target)?.origin;
        game.host.emit(Q1Event::Beam {
            style: Q1BeamStyle::Lightning2,
            actor: id.clone(),
            start,
            end,
        });
        let side = Vec3 {
            x: -(end.y - start.y) * 16.0,
            y: -(end.y - start.y) * 16.0,
            z: 0.0,
        };
        let mut hit: Vec<ActorId> = Vec::new();
        for offset in [ZERO, side, vscale(side, -1.0)] {
            let trace = game.host.trace(&Q1TraceRequest {
                start: vadd(start, offset),
                end: vadd(end, offset),
                bounds: POINT,
                ignore: Some(id.clone()),
                monsters: true,
                missile: false,
            });
            let Some(actor) = trace.actor.clone() else {
                continue;
            };
            if hit.iter().any(|prior| same_actor(prior, &actor)) {
                continue;
            }
            hit.push(actor.clone());
            if game
                .host
                .combat
                .read(&actor)
                .is_some_and(|combat| combat.can_take_damage)
            {
                let ammo = game
                    .entity(id)
                    .map(|entity| entity.number("currentammo"))
                    .unwrap_or(0.0);
                game.host.emit(Q1Event::Particles {
                    origin: trace.end,
                    direction: Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 100.0,
                    },
                    color: 225,
                    count: (ammo * 4.0) as i32,
                });
                let id_copy = id.clone();
                game.damage(
                    &actor,
                    Some(&id_copy),
                    Some(&id_copy),
                    ammo,
                    &crate::q1::foundation::entity_services::Q1DamageParams::default(),
                );
            }
        }
    }
    let (items, frags) = game
        .entity(id)
        .map(|entity| (entity.number("items"), entity.number("frags")))
        .unwrap_or((0.0, 0.0));
    if items < game.time {
        return later(game, id, frags, "rogue:ltrail_chain");
    }
    later(game, id, 0.05, "rogue:ltrail_fire")
}

/// Start an earthquake.
fn quake_start(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if let Some(world) = game.world.clone() {
        game.update_entity(&world, |world| {
            number(world, "rogue:earthquake_active", 1.0)
        })?;
    }
    let (spawnflags, delay) = game
        .entity(id)
        .map(|entity| (entity.spawnflags, entity.delay))
        .unwrap_or((0, 0.0));
    let time = game.time;
    game.update_entity(id, |entity| {
        entity.attack_finished = time
            + if spawnflags & 1 != 0 {
                game.host.random() * delay
            } else {
                delay
            };
    })?;
    quake_rumble(game, id)
}

/// Spawn an `earthquake`.
fn spawn_earthquake(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        if entity.delay == 0.0 {
            entity.delay = 20.0;
        }
        if entity.wait == 0.0 {
            entity.wait = 60.0;
        }
        if entity.number("weapon") == 0.0 {
            number(entity, "weapon", 40.0);
        }
    })?;
    if let Some(world) = game.world.clone() {
        let weapon = game
            .entity(id)
            .map(|entity| entity.number("weapon"))
            .unwrap_or(0.0);
        game.update_entity(&world, |world| {
            number(world, "rogue:earthquake_active", 0.0);
            number(world, "rogue:earthquake_intensity", weapon * 0.5);
        })?;
    }
    game.set_bounds(id, POINT)?;
    later(game, id, 1.0, "rogue:quake_stop")
}

/// Toggle an earthquake field.
fn earthquake_field_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.delay = if entity.delay == 0.0 { 1.0 } else { 0.0 }
    })
}

/// Shake players inside an earthquake field.
fn earthquake_field_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if game.entity(id).map(|entity| entity.delay).unwrap_or(0.0) == 0.0 {
        return Ok(());
    }
    if game
        .entity(id)
        .map(|entity| entity.attack_finished)
        .unwrap_or(0.0)
        < game.time
    {
        game.sound(id, "equake/rumble.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
        let time = game.time;
        game.update_entity(id, |entity| entity.attack_finished = time + 1.0)?;
    }
    if game.is_player(other) {
        let weapon = game
            .entity(id)
            .map(|entity| entity.number("weapon"))
            .unwrap_or(0.0);
        return rogue_earthquake(game, other, weapon);
    }
    Ok(())
}

/// Spawn a `trigger_earthquake`.
fn spawn_earthquake_field(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        let weapon = entity.number("weapon");
        number(
            entity,
            "weapon",
            (if weapon == 0.0 { 40.0 } else { weapon }) * 0.5,
        );
        entity.delay = if entity.targetname.is_empty() {
            1.0
        } else {
            0.0
        };
    })?;
    let touch_name = game.named.touch("rogue:earthquake_field")?;
    game.update_entity(id, |entity| entity.touch = Some(touch_name))?;
    if !game
        .entity(id)
        .map(|entity| entity.targetname.clone())
        .unwrap_or_default()
        .is_empty()
    {
        let use_name = game.named.use_callback("rogue:earthquake_field")?;
        game.update_entity(id, |entity| entity.use_callback = Some(use_name))?;
    }
    trigger(game, id)
}

/// Kill the earthquake when touched.
fn earthquake_kill_touch(
    game: &mut Q1EntityServices,
    _id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) {
        return Ok(());
    }
    let quake = game.entity_ids().into_iter().find(|id| {
        game.entity(id)
            .is_some_and(|entity| entity.classname == "earthquake")
    });
    if let Some(quake) = quake {
        if let Some(world) = game.world.clone() {
            game.update_entity(&world, |world| {
                number(world, "rogue:earthquake_active", 0.0)
            })?;
        }
        game.remove(&quake)?;
    }
    Ok(())
}

/// Spawn a `trigger_earthquake_kill`.
fn spawn_earthquake_kill(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let touch_name = game.named.touch("rogue:earthquake_kill")?;
    game.update_entity(id, |entity| entity.touch = Some(touch_name))?;
    trigger(game, id)
}

/// Step a buzzsaw, flying toward its goal when flagged.
fn saw_step(game: &mut Q1EntityServices, id: &ActorId, flying: bool) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.frame = if flying { 1 } else { 0 })?;
    if game
        .entity(id)
        .map(|entity| entity.number("pain_finished"))
        .unwrap_or(0.0)
        < game.time
    {
        game.host.emit(Q1Event::Sound {
            origin: None,
            actor: id.clone(),
            path: "buzz/buzz1.wav".to_string(),
            channel: Q1SoundChannel::Voice,
            attenuation: 1.0,
            volume: 0.2,
        });
        let time = game.time;
        game.update_entity(id, |entity| number(entity, "pain_finished", time + 1.0))?;
    }
    let body = game.body(id)?;
    if flying {
        let goal = game
            .entity(id)
            .and_then(|entity| entity.references.get("goalentity").cloned().flatten())
            .and_then(|goal| game.entity(&goal).map(|_| goal))
            .or_else(|| game.world.clone());
        let Some(goal) = goal.filter(|goal| game.entity(goal).is_some()) else {
            return Err(q1_error("Buzzsaw requires worldspawn"));
        };
        let speed = game.entity(id).map(|entity| entity.speed).unwrap_or(0.0);
        game.set_origin(
            id,
            vadd(
                body.origin,
                vscale(
                    normalize(vsub(game.body(&goal)?.origin, body.origin)),
                    speed,
                ),
            ),
        )?;
    }
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(Vec3 {
                x: body.angles.x - 60.0,
                y: body.angles.y,
                z: body.angles.z,
            }),
            ..Default::default()
        },
    )?;
    game.update_entity(id, |entity| {
        entity.angular_velocity = Vec3 {
            x: 60.0,
            y: entity.angular_velocity.y,
            z: entity.angular_velocity.z,
        };
    })?;
    later(
        game,
        id,
        0.1,
        if flying {
            "rogue:saw_fly"
        } else {
            "rogue:saw_stand"
        },
    )
}

fn saw_fly(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    saw_step(game, id, true)
}

fn saw_stand(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    saw_step(game, id, false)
}

/// Slice and throw whoever touches a buzzsaw.
fn saw_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let flags = game
        .entity(other)
        .map(|entity| entity.movement_flags)
        .unwrap_or(0);
    if !game.is_player(other) && flags & 32 == 0 {
        return Ok(());
    }
    if game
        .entity(id)
        .map(|entity| entity.attack_finished)
        .unwrap_or(0.0)
        < game.time
    {
        game.sound(id, "buzz/buzz.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        let time = game.time;
        game.update_entity(id, |entity| entity.attack_finished = time + 2.0)?;
    }
    let ammo = game
        .entity(id)
        .map(|entity| entity.number("currentammo"))
        .unwrap_or(0.0);
    let id_copy = id.clone();
    let other_copy = other.clone();
    game.damage(
        &other_copy,
        Some(&id_copy),
        Some(&id_copy),
        ammo,
        &crate::q1::foundation::entity_services::Q1DamageParams::default(),
    );
    let goal = game
        .entity(id)
        .and_then(|entity| entity.references.get("goalentity").cloned().flatten())
        .and_then(|goal| game.entity(&goal).map(|_| goal))
        .or_else(|| game.world.clone());
    let body = game.host.bodies.read(other);
    let owned = game.host.actors.resolve_owned(other);
    let (Some(goal), Some(body), Some(owned)) = (goal, body, owned) else {
        return Ok(());
    };
    if game.entity(&goal).is_none() {
        return Ok(());
    }
    let origin = game.body(id)?.origin;
    let direction = vscale(normalize(vsub(game.body(&goal)?.origin, origin)), 200.0);
    game.effect(Q1Effect::MeatSpray, origin, Some(other), 1);
    let mut next = body.clone();
    next.velocity = Vec3 {
        x: direction.x,
        y: direction.y,
        z: 200.0,
    };
    game.host.bodies.write(&owned, &next)?;
    Ok(())
}

/// Spawn a `buzzsaw`.
fn spawn_buzzsaw(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.model = "progs/buzzsaw.mdl".to_string();
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::Fly;
    })?;
    game.set_damageable(id, false)?;
    let yaw = game.body(id)?.angles.y;
    if yaw == 0.0 || yaw == 180.0 {
        game.set_bounds(
            id,
            qa_core::math::Bounds {
                min: Vec3 {
                    x: -18.0,
                    y: 0.0,
                    z: -18.0,
                },
                max: Vec3 {
                    x: 18.0,
                    y: 0.0,
                    z: 18.0,
                },
            },
        )?;
    } else if yaw == 90.0 || yaw == 270.0 {
        game.set_bounds(
            id,
            qa_core::math::Bounds {
                min: Vec3 {
                    x: 0.0,
                    y: -18.0,
                    z: -18.0,
                },
                max: Vec3 {
                    x: 0.0,
                    y: 18.0,
                    z: 18.0,
                },
            },
        )?;
    } else {
        return Err(q1_error("Buzzsaw: Not at 90 degree angle!"));
    }
    game.update_entity(id, |entity| {
        if entity.speed == 0.0 {
            entity.speed = 10.0;
        }
        if entity.number("currentammo") == 0.0 {
            number(entity, "currentammo", 10.0);
        }
    })?;
    let time = game.time + game.host.random() * 2.0;
    game.update_entity(id, |entity| number(entity, "pain_finished", time))?;
    if game
        .entity(id)
        .map(|entity| entity.targetname.clone())
        .unwrap_or_default()
        .is_empty()
    {
        return later(game, id, 0.2, "rogue:saw_start");
    }
    let use_name = game.named.use_callback("rogue:saw_start")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))
}

/// Fire trail targets, then go idle.
fn ltrail_chain(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let activator = game.entity(id).and_then(|entity| entity.activator.clone());
    game.use_targets(id, activator.as_ref())?;
    let null = game.named.action("SUB_Null")?;
    game.update_entity(id, |entity| entity.think = Some(null))
}

/// Toggle or trigger a lightning trail.
fn ltrail_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let activator = activator.cloned();
    game.update_entity(id, |entity| entity.activator = activator)?;
    let (spawnflags, classname) = game
        .entity(id)
        .map(|entity| (entity.spawnflags, entity.classname.clone()))
        .unwrap_or((0, String::new()));
    if spawnflags & 1 != 0 {
        let other_class = other
            .and_then(|other| game.entity(other))
            .map(|entity| entity.classname.clone());
        if other_class.as_deref() != Some("ltrail_end") {
            if spawnflags & 2 != 0 {
                game.update_entity(id, |entity| entity.spawnflags -= 2)?;
                return Ok(());
            }
            game.update_entity(id, |entity| entity.spawnflags += 2)?;
        } else if spawnflags & 2 == 0 {
            return Ok(());
        }
    }
    if classname == "ltrail_end" {
        let frags = game
            .entity(id)
            .map(|entity| entity.number("frags"))
            .unwrap_or(0.0);
        return later(game, id, frags, "rogue:ltrail_chain");
    }
    let weapon = game
        .entity(id)
        .map(|entity| entity.number("weapon"))
        .unwrap_or(0.0);
    let time = game.time;
    game.update_entity(id, |entity| number(entity, "items", time + weapon))?;
    trail_fire(game, id)?;
    if classname == "ltrail_start" {
        let time = game.time;
        game.update_entity(id, |entity| number(entity, "ltrailLastUsed", time))?;
    }
    Ok(())
}

/// Spawn a lightning-trail node.
fn spawn_ltrail_variant(
    classname: &'static str,
) -> fn(&mut Q1EntityServices, &ActorId) -> Result<(), Q1Error> {
    match classname {
        "ltrail_relay" => spawn_ltrail_relay,
        "ltrail_end" => spawn_ltrail_end,
        _ => spawn_ltrail_start,
    }
}

fn ltrail_variant(
    game: &mut Q1EntityServices,
    id: &ActorId,
    classname: &str,
) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.movement = Q1MoveType::None;
        entity.solid = Q1Solid::Bbox;
        if entity.number("currentammo") == 0.0 {
            number(entity, "currentammo", 25.0);
        }
        if entity.number("weapon") == 0.0 {
            number(entity, "weapon", 0.3);
        }
        if entity.number("frags") == 0.0 {
            number(entity, "frags", 0.3);
        }
    })?;
    let use_name = game.named.use_callback("rogue:ltrail_use")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))?;
    if classname == "ltrail_start" {
        let time = game.time;
        game.update_entity(id, |entity| number(entity, "ltrailLastUsed", time))?;
        if game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0) & 2 != 0 {
            let time = game.time;
            game.update_entity(id, |entity| number(entity, "items", time + 99_999_999.0))?;
            later(game, id, 0.1, "rogue:ltrail_fire")?;
        }
    }
    Ok(())
}

fn spawn_ltrail_start(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    ltrail_variant(game, id, "ltrail_start")
}
fn spawn_ltrail_relay(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    ltrail_variant(game, id, "ltrail_relay")
}
fn spawn_ltrail_end(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    ltrail_variant(game, id, "ltrail_end")
}

/// Register Rogue hazard entities (`registerRogueHazards`).
pub fn register_rogue_hazards(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "rogue:quake_stop",
        Q1CallbackHandlers {
            action: Some(quake_stop),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:quake_rumble",
        Q1CallbackHandlers {
            action: Some(quake_rumble),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:quake_start",
        Q1CallbackHandlers {
            action: Some(quake_start),
            ..Default::default()
        },
    )?;
    game.register_spawn("earthquake", spawn_earthquake)?;
    game.named.register(
        "rogue:earthquake_field",
        Q1CallbackHandlers {
            use_callback: Some(earthquake_field_use),
            touch: Some(earthquake_field_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_earthquake", spawn_earthquake_field)?;
    game.named.register(
        "rogue:earthquake_kill",
        Q1CallbackHandlers {
            touch: Some(earthquake_kill_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_earthquake_kill", spawn_earthquake_kill)?;
    game.named.register(
        "rogue:saw_fly",
        Q1CallbackHandlers {
            action: Some(saw_fly),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:saw_stand",
        Q1CallbackHandlers {
            action: Some(saw_stand),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:saw_start",
        Q1CallbackHandlers {
            action: Some(saw_start_action),
            use_callback: Some(saw_start),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:saw_touch",
        Q1CallbackHandlers {
            touch: Some(saw_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("buzzsaw", spawn_buzzsaw)?;
    game.named.register(
        "rogue:ltrail_chain",
        Q1CallbackHandlers {
            action: Some(ltrail_chain),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:ltrail_fire",
        Q1CallbackHandlers {
            action: Some(trail_fire),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:ltrail_use",
        Q1CallbackHandlers {
            use_callback: Some(ltrail_use),
            ..Default::default()
        },
    )?;
    for classname in ["ltrail_start", "ltrail_relay", "ltrail_end"] {
        game.register_spawn(classname, spawn_ltrail_variant(classname))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    fn with_world(game: &mut Q1EntityServices) -> ActorId {
        let world = game.create("worldspawn", None, None).expect("world");
        game.world = Some(world.clone());
        world
    }

    #[test]
    fn quake_runs_until_its_timer_expires() {
        let mut game = test_game();
        register_rogue_hazards(&mut game).expect("register");
        let world = with_world(&mut game);
        let id = game.create("earthquake", None, None).expect("quake");
        game.spawn_entity(&id, None).expect("spawn");
        assert_eq!(
            game.entity(&world)
                .expect("world")
                .number("rogue:earthquake_intensity"),
            20.0
        );
        game.invoke_action(&id, "rogue:quake_start").expect("start");
        assert_eq!(
            game.entity(&world)
                .expect("world")
                .number("rogue:earthquake_active"),
            1.0
        );
        game.time = 100.0;
        game.invoke_action(&id, "rogue:quake_rumble")
            .expect("rumble");
        assert_eq!(
            game.entity(&world)
                .expect("world")
                .number("rogue:earthquake_active"),
            0.0
        );
        assert_eq!(
            game.entity(&id).expect("quake").think.as_deref(),
            Some("rogue:quake_start")
        );
    }

    #[test]
    fn buzzsaw_flies_toward_its_goal() {
        let mut game = test_game();
        register_rogue_hazards(&mut game).expect("register");
        let corner = game.create("path_corner", None, None).expect("corner");
        game.update_entity(&corner, |entity| entity.targetname = "p1".to_string())
            .expect("targetname");
        game.set_origin(
            &corner,
            Vec3 {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
        )
        .expect("corner origin");
        let id = game.create("buzzsaw", None, None).expect("saw");
        game.set_body(
            &id,
            &BodyPatch {
                angles: Some(Vec3 {
                    x: 0.0,
                    y: 90.0,
                    z: 0.0,
                }),
                ..Default::default()
            },
        )
        .expect("angles");
        game.update_entity(&id, |entity| {
            entity.target = "p1".to_string();
            entity.targetname = "saw1".to_string();
        })
        .expect("target");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_use(&id, "rogue:saw_start", None, None)
            .expect("start");
        assert_eq!(
            game.entity(&id).expect("saw").think.as_deref(),
            Some("rogue:saw_fly")
        );
        game.invoke_action(&id, "rogue:saw_fly").expect("fly");
        assert_eq!(f64::from(game.body(&id).expect("body").origin.x), 10.0);
    }

    #[test]
    fn lightning_trail_fires_then_chains() {
        let mut game = test_game();
        register_rogue_hazards(&mut game).expect("register");
        let end = game.create("ltrail_end", None, None).expect("end");
        game.update_entity(&end, |entity| entity.targetname = "e1".to_string())
            .expect("targetname");
        game.spawn_entity(&end, None).expect("spawn end");
        let id = game.create("ltrail_start", None, None).expect("start");
        game.update_entity(&id, |entity| entity.target = "e1".to_string())
            .expect("target");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_use(&id, "rogue:ltrail_use", None, None)
            .expect("use");
        game.time = 1.0;
        game.invoke_action(&id, "rogue:ltrail_fire").expect("fire");
        assert_eq!(
            game.entity(&id).expect("trail").think.as_deref(),
            Some("rogue:ltrail_chain")
        );
        game.invoke_action(&id, "rogue:ltrail_chain")
            .expect("chain");
        assert_eq!(
            game.entity(&id).expect("trail").think.as_deref(),
            Some("SUB_Null")
        );
    }
}
