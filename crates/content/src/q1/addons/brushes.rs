//! Q1 addon brushes (`src/content/q1/addons/brushes.ts`).
//!
//! MG1/MG3 `func_bob.qc`, `func_toss.qc`, `rotate.qc`, `misc.qc`,
//! `misc_model.qc`. Copyright (C) 1996-2026 id Software LLC.
//! GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::addons::context::{
    add_frame_tick, addon_alpha, addon_frame_time, addon_is_monster, addon_program, fround, remove_frame_tick,
    require_entity, set_addon_number, set_addon_vector, Q1AddonContext,
};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{length, normalize, vadd, vscale, vsub, Q1Effect, Q1MoveType, Q1Solid, ZERO};
use crate::q1::{q1_error, Q1Error};

fn prefix(game: &Q1EntityServices) -> Result<String, Q1Error> {
    Ok(format!("{}:brush:", addon_program(game)?.as_str()))
}

fn schedule_brush(game: &mut Q1EntityServices, id: &ActorId, name: &str, delay: f64) -> Result<(), Q1Error> {
    let callback = game.named.action(&format!("{}{name}", prefix(game)?))?;
    game.schedule(id, delay, &callback)
}

fn schedule_brush_at(game: &mut Q1EntityServices, id: &ActorId, name: &str, due: f64) -> Result<(), Q1Error> {
    let callback = game.named.action(&format!("{}{name}", prefix(game)?))?;
    game.schedule_at(id, due, &callback)
}

fn invoke_stored_use(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let name = require_entity(game, id)?.use_callback.clone();
    match name {
        Some(name) => game.invoke_use(id, &name, None, None),
        None => Ok(()),
    }
}

fn push_brush(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::Bsp;
        entity.movement = Q1MoveType::Push;
    })?;
    game.link(id)
}

fn bob_position(game: &mut Q1EntityServices, id: &ActorId, angle: f64) -> Result<Vec3, Q1Error> {
    let count = fround(fround(require_entity(game, id)?.count + angle) % 360.0);
    game.update_entity(id, |entity| entity.count = count)?;
    let entity = require_entity(game, id)?.clone();
    Ok(vadd(
        vscale(entity.vector("dest"), count.sin()),
        vscale(entity.vector("dest2"), count.cos()),
    ))
}

fn bob_tick(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let step = fround(f64::from(require_entity(game, id)?.angular_velocity.x) * addon_frame_time(game)?);
    let position = bob_position(game, id, step)?;
    game.set_origin(id, position)
}

fn bob_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let step = fround(f64::from(require_entity(game, id)?.angular_velocity.x) * 0.05);
    let position = bob_position(game, id, step)?;
    let origin = game.body(id)?.origin;
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(vscale(vsub(position, origin), 20.0)),
            ..Default::default()
        },
    )?;
    schedule_brush_at(
        game,
        id,
        "bob_think",
        fround(require_entity(game, id)?.number("ltime") + 0.05),
    )
}

fn bob_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    let solid = entity.solid != Q1Solid::None;
    if entity.activated {
        if solid {
            game.set_body(
                id,
                &BodyPatch {
                    velocity: Some(ZERO),
                    ..Default::default()
                },
            )?;
            game.cancel(id);
        } else {
            remove_frame_tick(game, id)?;
        }
    } else if solid {
        game.invoke_action(id, &format!("{}bob_think", prefix(game)?))?;
    } else {
        add_frame_tick(game, id, &format!("{}bob_tick", prefix(game)?))?;
    }
    game.update_entity(id, |entity| entity.activated = !entity.activated)
}

fn bob_blocked(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    if entity.attack_finished > game.time {
        return Ok(());
    }
    game.damage(other, Some(id), Some(id), entity.damage, &Q1DamageParams::default());
    let attack_finished = fround(game.time + 0.5);
    game.update_entity(id, |entity| entity.attack_finished = attack_finished)
}

fn spawn_func_bob(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if length(require_entity(game, id)?.vector("dest")) == 0.0 {
        set_addon_vector(
            game,
            id,
            "dest",
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 64.0,
            },
        )?;
    }
    game.update_entity(id, |entity| {
        if entity.wait == 0.0 {
            entity.wait = 10.0;
        }
        if entity.damage == 0.0 {
            entity.damage = 1.0;
        }
    })?;
    let entity = require_entity(game, id)?.clone();
    game.update_entity(id, |entity| {
        entity.angular_velocity = Vec3 {
            x: fround(360.0 / entity.wait) as f32,
            y: 0.0,
            z: 0.0,
        };
        entity.count = fround(360.0 * entity.delay);
    })?;
    let use_callback = game.named.use_callback(&format!("{}bob_use", prefix(game)?))?;
    let blocked = game.named.blocked(&format!("{}bob_blocked", prefix(game)?))?;
    game.update_entity(id, |entity| {
        entity.use_callback = Some(use_callback);
        entity.blocked = Some(blocked);
        entity.solid = if (entity.spawnflags & 1) != 0 {
            Q1Solid::None
        } else {
            Q1Solid::Bsp
        };
        entity.movement = if entity.solid == Q1Solid::None {
            Q1MoveType::None
        } else {
            Q1MoveType::Push
        };
    })?;
    if (entity.spawnflags & 2) != 0 {
        invoke_stored_use(game, id)?;
    }
    Ok(())
}

fn toss_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if length(game.body(id)?.velocity) < 0.01 {
        let entity = require_entity(game, id)?.clone();
        if (entity.spawnflags & 2) != 0
            && ((entity.spawnflags & 8) == 0 || length(vsub(entity.vector("oldorigin"), game.body(id)?.origin)) > 64.0)
        {
            return game.remove(id);
        }
        game.set_body(
            id,
            &BodyPatch {
                velocity: Some(ZERO),
                ..Default::default()
            },
        )?;
        return push_brush(game, id);
    }
    schedule_brush(game, id, "toss_think", 0.5)
}

fn toss_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    let body = game.body(id)?;
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::Bbox;
        entity.movement = if (entity.spawnflags & 4) != 0 {
            Q1MoveType::Toss
        } else {
            Q1MoveType::Bounce
        };
    })?;
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(entity.movedir),
            bounds: Some(qa_core::math::Bounds {
                min: vadd(body.bounds.min, Vec3 { x: 4.0, y: 4.0, z: 0.0 }),
                max: body.bounds.max,
            }),
            ..Default::default()
        },
    )?;
    if !entity.text("noise").is_empty() {
        game.sound_simple(id, &entity.text("noise"))?;
    }
    game.link(id)?;
    schedule_brush(game, id, "toss_think", 0.25)
}

fn toss_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.use_callback = None)?;
    if require_entity(game, id)?.delay != 0.0 {
        let due = fround(require_entity(game, id)?.number("ltime") + require_entity(game, id)?.delay);
        schedule_brush_at(game, id, "toss", due)
    } else {
        game.invoke_action(id, &format!("{}toss", prefix(game)?))
    }
}

fn toss_cascade(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    let bounds = game.body(id)?.bounds;
    let position = vscale(vadd(bounds.min, bounds.max), 0.5);
    let mut nearest = 16384.0f64;
    for (other_id, other) in game.entities.iter() {
        if other.text("netname") != "_toss_origin" || other.targetname != entity.targetname {
            continue;
        }
        let other_bounds = game.body(other_id)?.bounds;
        let center = vscale(vadd(other_bounds.min, other_bounds.max), 0.5);
        let distance = fround(f64::from(length(vsub(position, center))) / other.speed);
        nearest = nearest.min(distance);
    }
    if nearest < 16384.0 {
        game.update_entity(id, |entity| entity.delay = nearest)?;
    }
    Ok(())
}

fn spawn_func_toss(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    push_brush(game, id)?;
    let origin = game.body(id)?.origin;
    set_addon_vector(game, id, "oldorigin", origin)?;
    let movedir = require_entity(game, id)?.vector("movedir");
    game.update_entity(id, |entity| {
        entity.movedir = if length(movedir) == 0.0 {
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 200.0,
            }
        } else {
            movedir
        };
    })?;
    let variance = require_entity(game, id)?.vector("dest");
    if length(variance) != 0.0 {
        let x = (game.host.random() * 2.0 - 1.0) * f64::from(variance.x);
        let y = (game.host.random() * 2.0 - 1.0) * f64::from(variance.y);
        let z = (game.host.random() * 2.0 - 1.0) * f64::from(variance.z);
        game.update_entity(id, |entity| {
            entity.movedir = vadd(
                entity.movedir,
                Vec3 {
                    x: x as f32,
                    y: y as f32,
                    z: z as f32,
                },
            );
        })?;
    }
    let use_callback = game.named.use_callback(&format!("{}toss_use", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))?;
    if (require_entity(game, id)?.spawnflags & 1) != 0 {
        game.update_entity(id, |entity| {
            entity
                .fields
                .insert(String::from("netname"), String::from("_toss_origin"));
            if entity.speed == 0.0 {
                entity.speed = 200.0;
            }
        })?;
        return Ok(());
    }
    if require_entity(game, id)?.delay == 0.0 {
        let due = fround(require_entity(game, id)?.number("ltime") + 0.2);
        schedule_brush_at(game, id, "toss_cascade", due)?;
    }
    Ok(())
}

fn shatter_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let movedir = require_entity(game, id)?.movedir;
    game.update_entity(id, |entity| {
        entity.use_callback = None;
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::Toss;
    })?;
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(movedir),
            ..Default::default()
        },
    )?;
    game.link(id)
}

fn spawn_func_shatter(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    push_brush(game, id)?;
    let body = game.body(id)?;
    let pos1 = vadd(body.origin, vscale(vadd(body.bounds.min, body.bounds.max), 0.5));
    game.update_entity(id, |entity| entity.pos1 = pos1)?;
    let mut pos2 = require_entity(game, id)?.vector("pos2");
    if length(pos2) == 0.0 {
        pos2 = vsub(
            body.origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 100.0,
            },
        );
        game.update_entity(id, |entity| entity.pos2 = pos2)?;
    }
    let difference = vsub(pos1, pos2);
    let distance = f64::from(length(difference));
    let speed = require_entity(game, id)?.speed;
    game.update_entity(id, |entity| {
        entity.speed = fround(fround(if speed == 0.0 { 200.0 } else { speed } * 10000.0) / fround(distance * distance));
        if entity.wait == 0.0 {
            entity.wait = 10.0;
        }
    })?;
    let entity = require_entity(game, id)?.clone();
    let x = (game.host.random() * 2.0 - 1.0) * entity.wait;
    let y = (game.host.random() * 2.0 - 1.0) * entity.wait;
    let z = (game.host.random() * 2.0 - 1.0) * entity.wait;
    game.update_entity(id, |entity| {
        entity.movedir = vadd(
            vscale(normalize(difference), entity.speed),
            Vec3 {
                x: x as f32,
                y: y as f32,
                z: z as f32,
            },
        );
    })?;
    let use_callback = game.named.use_callback(&format!("{}shatter", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn debris_fade(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    let alpha = fround(entity.number_or("alpha", 1.0) - fround(fround(1.0 / entity.wait) * addon_frame_time(game)?));
    addon_alpha(game, id, alpha)?;
    if alpha < 0.0 {
        addon_alpha(game, id, 1.0)?;
        let origin = require_entity(game, id)?.vector("oldorigin");
        game.set_origin(id, origin)?;
        game.update_entity(id, |entity| entity.movement = Q1MoveType::Toss)?;
        let x = (game.host.random() * 2.0 - 1.0) * 8.0;
        let y = (game.host.random() * 2.0 - 1.0) * 8.0;
        game.set_body(
            id,
            &BodyPatch {
                velocity: Some(Vec3 {
                    x: x as f32,
                    y: y as f32,
                    z: 0.0,
                }),
                ..Default::default()
            },
        )?;
        return schedule_brush(game, id, "debris_fade", entity.delay);
    }
    schedule_brush(game, id, "debris_fade", 0.01)
}

fn debris_wake(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.use_callback = None;
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::Toss;
    })?;
    addon_alpha(game, id, 1.0)?;
    let velocity = game.body(id)?.velocity;
    let x = (game.host.random() * 2.0 - 1.0) * 8.0;
    let y = (game.host.random() * 2.0 - 1.0) * 8.0;
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(Vec3 {
                x: x as f32,
                y: y as f32,
                z: velocity.z,
            }),
            ..Default::default()
        },
    )?;
    game.link(id)?;
    schedule_brush(game, id, "debris_fade", require_entity(game, id)?.delay)
}

fn debris_wait_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let due = fround(require_entity(game, id)?.number("ltime") + game.host.random());
    schedule_brush_at(game, id, "debris_wake", due)
}

fn spawn_func_debris(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    push_brush(game, id)?;
    let origin = game.body(id)?.origin;
    set_addon_vector(game, id, "oldorigin", origin)?;
    game.update_entity(id, |entity| {
        if entity.delay == 0.0 {
            entity.delay = 1.5;
        }
        if entity.wait == 0.0 {
            entity.wait = 0.1;
        }
        entity.movedir = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 200.0,
        };
    })?;
    let use_callback = game.named.use_callback(&format!("{}debris_wait", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))?;
    addon_alpha(game, id, 1.0)
}

fn explode_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let body = game.body(id)?;
    let position = vscale(vadd(body.bounds.min, body.bounds.max), 0.5);
    game.update_entity(id, |entity| entity.model.clear())?;
    game.set_origin(id, position)?;
    game.radius_damage(id, Some(id), 160.0, None, None, "");
    game.sound_simple(id, "weapons/r_exp3.wav")?;
    game.effect_simple(Q1Effect::Explosion, position);
    let activator = require_entity(game, id)?.activator.clone();
    game.use_targets(id, activator.as_ref())?;
    game.remove(id)
}

fn explode_die(game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    game.set_damageable(id, false)?;
    game.update_entity(id, |entity| entity.activator = attacker.cloned())?;
    let due = fround(require_entity(game, id)?.number("ltime") + 0.15);
    schedule_brush_at(game, id, "explode", due)
}

fn spawn_func_explode(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    push_brush(game, id)?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    let actor = require_entity(game, id)?.actor.clone();
    game.host.combat.set_health(&actor, 20.0)?;
    game.set_damageable(id, true)?;
    game.update_entity(id, |entity| entity.aimed_damage = true)?;
    let die = game.named.die(&format!("{}explode_die", prefix(game)?))?;
    game.update_entity(id, |entity| entity.die = Some(die))
}

fn hurt_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.activated = !entity.activated;
        entity.attack_finished = 0.0;
    })
}

fn hurt_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    if !entity.activated
        || entity.attack_finished > game.time
        || game.health(other) <= 0.0
        || !game
            .host
            .combat
            .read(other)
            .is_some_and(|combat| combat.can_take_damage)
    {
        return Ok(());
    }
    if !game.is_player(other) && !addon_is_monster(game, other)? {
        return Ok(());
    }
    let world = game.world.clone();
    game.damage(
        other,
        Some(id),
        world.as_ref(),
        entity.damage,
        &Q1DamageParams::default(),
    );
    let attack_finished = fround(game.time + entity.wait);
    game.update_entity(id, |entity| entity.attack_finished = attack_finished)
}

fn spawn_func_hurt(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    push_brush(game, id)?;
    game.update_entity(id, |entity| {
        if entity.damage == 0.0 {
            entity.damage = 10.0;
        }
        if entity.wait == 0.0 {
            entity.wait = 0.2;
        }
        entity.activated = (entity.spawnflags & 1) != 0;
    })?;
    let use_callback = game.named.use_callback(&format!("{}hurt_use", prefix(game)?))?;
    let touch = game.named.touch(&format!("{}hurt_touch", prefix(game)?))?;
    game.update_entity(id, |entity| {
        entity.use_callback = Some(use_callback);
        entity.touch = Some(touch);
    })
}

fn spawn_func_fade(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    // The official function unconditionally delegates to func_wall
    // before its obsolete fade code.
    let use_callback = game.named.use_callback("func_wall_use")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))?;
    push_brush(game, id)
}

fn model_loop(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.frame += 1)?;
    let entity = require_entity(game, id)?.clone();
    if f64::from(entity.frame) == entity.number("cnt") {
        game.update_entity(id, |entity| entity.frame = entity.count as i32)?;
    }
    schedule_brush(game, id, "model_loop", 0.1)
}

fn model_once(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.frame += 1)?;
    if f64::from(require_entity(game, id)?.frame) < require_entity(game, id)?.number("cnt") {
        schedule_brush(game, id, "model_once", 0.1)?;
    }
    Ok(())
}

fn model_use_once(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let count = require_entity(game, id)?.count;
    game.update_entity(id, |entity| entity.frame = count as i32)?;
    schedule_brush(game, id, "model_once", 0.1)
}

fn model_use_loop(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.spawnflags ^= 4)?;
    if (require_entity(game, id)?.spawnflags & 4) == 0 {
        game.invoke_action(id, &format!("{}model_loop", prefix(game)?))
    } else {
        game.cancel(id);
        Ok(())
    }
}

fn spawn_misc_model(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if require_entity(game, id)?.model.is_empty() {
        return Err(q1_error("misc_model requires a model"));
    }
    let frame = require_entity(game, id)?.number("frame") as i32;
    game.update_entity(id, |entity| {
        entity.frame = frame;
        entity.solid = Q1Solid::None;
    })?;
    if require_entity(game, id)?.spawnflags == 0 {
        return Ok(());
    }
    let entity = require_entity(game, id)?.clone();
    if entity.number("cnt") < f64::from(entity.frame) || entity.targetname.is_empty() {
        return Err(q1_error("misc_model requires a valid frame range and targetname"));
    }
    game.update_entity(id, |entity| entity.count = f64::from(entity.frame))?;
    if (entity.spawnflags & 2) != 0 {
        let use_callback = game.named.use_callback(&format!("{}model_use_once", prefix(game)?))?;
        game.update_entity(id, |entity| entity.use_callback = Some(use_callback))?;
    } else if (entity.spawnflags & 5) != 0 {
        let use_callback = game.named.use_callback(&format!("{}model_use_loop", prefix(game)?))?;
        game.update_entity(id, |entity| {
            entity.use_callback = Some(use_callback);
            entity.spawnflags ^= 4;
        })?;
        invoke_stored_use(game, id)?;
    }
    Ok(())
}

fn spawn_info_rotate_axis(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.remove(id)
}

fn rotate_brush(game: &mut Q1EntityServices, id: &ActorId, scale: f64) -> Result<(), Q1Error> {
    let body = game.body(id)?;
    let step = fround(addon_frame_time(game)? * scale);
    let angles = vadd(body.angles, vscale(require_entity(game, id)?.angular_velocity, step));
    let angle = |value: f64| {
        if value > 360.0 {
            value - ((value - 360.0) / 360.0).ceil() * 360.0
        } else if value < 0.0 {
            value + (-value / 360.0).ceil() * 360.0
        } else {
            value
        }
    };
    // Source ModAngles mistakenly tests x for the negative-z
    // adjustment.
    let z = f64::from(angles.z);
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(Vec3 {
                x: angle(f64::from(angles.x)) as f32,
                y: angle(f64::from(angles.y)) as f32,
                z: (if z > 360.0 { angle(z) } else { z }) as f32,
            }),
            ..Default::default()
        },
    )
}

fn rotate_tick(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    rotate_brush(game, id, 1.0)
}

fn rotate_tween(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let speed = fround(
        require_entity(game, id)?.speed
            + fround(require_entity(game, id)?.number("distance") * addon_frame_time(game)?),
    )
    .clamp(0.0, 1.0);
    game.update_entity(id, |entity| entity.speed = speed)?;
    if speed == 0.0 {
        set_addon_number(game, id, "rotate.state", 0.0)?;
        return remove_frame_tick(game, id);
    }
    if speed == 1.0 {
        set_addon_number(game, id, "rotate.state", 2.0)?;
        let tick = format!("{}rotate_tick", prefix(game)?);
        game.update_entity(id, |entity| {
            entity.fields.insert(String::from("addon.frameTick"), tick);
        })?;
        return rotate_brush(game, id, 1.0);
    }
    rotate_brush(game, id, speed)
}

fn rotate_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    let state = entity.number("rotate.state");
    if entity.delay <= 0.0 {
        set_addon_number(game, id, "rotate.state", if state == 0.0 { 2.0 } else { 0.0 })?;
        return if state == 0.0 {
            add_frame_tick(game, id, &format!("{}rotate_tick", prefix(game)?))
        } else {
            remove_frame_tick(game, id)
        };
    }
    let tween = format!("{}rotate_tween", prefix(game)?);
    game.update_entity(id, |entity| {
        entity.fields.insert(String::from("addon.frameTick"), tween);
    })?;
    set_addon_number(
        game,
        id,
        "rotate.state",
        if state == 0.0 || state == 3.0 { 1.0 } else { 3.0 },
    )?;
    let distance =
        require_entity(game, id)?.number("distance").abs() * if state == 0.0 || state == 3.0 { 1.0 } else { -1.0 };
    set_addon_number(game, id, "distance", distance)?;
    if state == 0.0 {
        add_frame_tick(game, id, &format!("{}rotate_tween", prefix(game)?))?;
    }
    Ok(())
}

fn spawn_rotate_object_continuously(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let avelocity = require_entity(game, id)?.vector("avelocity");
    game.update_entity(id, |entity| {
        entity.angular_velocity = if length(avelocity) == 0.0 {
            Vec3 {
                x: 0.0,
                y: 30.0,
                z: 0.0,
            }
        } else {
            avelocity
        };
    })?;
    if require_entity(game, id)?.delay > 0.0 {
        let delay = require_entity(game, id)?.delay;
        set_addon_number(game, id, "distance", 1.0 / delay)?;
    }
    game.update_entity(id, |entity| {
        entity.solid = if (entity.spawnflags & 4) != 0 {
            Q1Solid::Bsp
        } else {
            Q1Solid::None
        };
        entity.movement = if entity.solid == Q1Solid::Bsp {
            Q1MoveType::Push
        } else {
            Q1MoveType::None
        };
    })?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    let pos = require_entity(game, id)?.vector("pos2");
    if length(pos) != 0.0 {
        game.set_origin(id, pos)?;
    }
    let use_callback = game.named.use_callback(&format!("{}rotate_use", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))?;
    let off = (require_entity(game, id)?.spawnflags & 1) != 0;
    set_addon_number(game, id, "rotate.state", if off { 0.0 } else { 2.0 })?;
    if !off {
        game.update_entity(id, |entity| entity.speed = 1.0)?;
        add_frame_tick(game, id, &format!("{}rotate_tick", prefix(game)?))?;
    }
    Ok(())
}

fn breakable_stop(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(ZERO),
            ..Default::default()
        },
    )
}

fn breakable_pain(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _attacker: Option<&ActorId>,
    _damage: f64,
) -> Result<(), Q1Error> {
    let actor = require_entity(game, id)?.actor.clone();
    game.host.combat.set_health(&actor, 10000.0)?;
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(Vec3 {
                x: 0.0,
                y: 0.0,
                z: -20.0,
            }),
            ..Default::default()
        },
    )?;
    let due = fround(require_entity(game, id)?.number("ltime") + 1.0);
    schedule_brush_at(game, id, "breakable_stop", due)
}

fn breakable_die(game: &mut Q1EntityServices, id: &ActorId, _attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    game.remove(id)
}

fn spawn_func_breakable(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    push_brush(game, id)?;
    let actor = require_entity(game, id)?.actor.clone();
    game.host.combat.set_health(&actor, 10000.0)?;
    game.update_entity(id, |entity| entity.max_health = 10000.0)?;
    game.set_damageable(id, true)?;
    game.update_entity(id, |entity| entity.aimed_damage = false)?;
    let pain = game.named.pain(&format!("{}breakable_pain", prefix(game)?))?;
    let die = game.named.die(&format!("{}breakable_die", prefix(game)?))?;
    game.update_entity(id, |entity| {
        entity.pain = Some(pain);
        entity.die = Some(die);
    })
}

/// Register addon brushes (`registerAddonBrushes`).
pub fn register_addon_brushes(context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let prefix = format!("{}:brush:", context.program().as_str());
    game.named.register(
        &format!("{prefix}bob_tick"),
        Q1CallbackHandlers {
            action: Some(bob_tick),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}bob_think"),
        Q1CallbackHandlers {
            action: Some(bob_think),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}bob_use"),
        Q1CallbackHandlers {
            use_callback: Some(bob_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}bob_blocked"),
        Q1CallbackHandlers {
            blocked: Some(bob_blocked),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_bob", spawn_func_bob)?;
    game.named.register(
        &format!("{prefix}toss_think"),
        Q1CallbackHandlers {
            action: Some(toss_think),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}toss"),
        Q1CallbackHandlers {
            action: Some(toss_action),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}toss_use"),
        Q1CallbackHandlers {
            use_callback: Some(toss_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}toss_cascade"),
        Q1CallbackHandlers {
            action: Some(toss_cascade),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_toss", spawn_func_toss)?;
    game.named.register(
        &format!("{prefix}shatter"),
        Q1CallbackHandlers {
            use_callback: Some(shatter_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_shatter", spawn_func_shatter)?;
    game.named.register(
        &format!("{prefix}debris_fade"),
        Q1CallbackHandlers {
            action: Some(debris_fade),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}debris_wake"),
        Q1CallbackHandlers {
            action: Some(debris_wake),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}debris_wait"),
        Q1CallbackHandlers {
            use_callback: Some(debris_wait_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_debris", spawn_func_debris)?;
    game.named.register(
        &format!("{prefix}explode"),
        Q1CallbackHandlers {
            action: Some(explode_action),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}explode_die"),
        Q1CallbackHandlers {
            die: Some(explode_die),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_explode", spawn_func_explode)?;
    game.named.register(
        &format!("{prefix}hurt_use"),
        Q1CallbackHandlers {
            use_callback: Some(hurt_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}hurt_touch"),
        Q1CallbackHandlers {
            touch: Some(hurt_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_hurt", spawn_func_hurt)?;
    game.register_spawn("func_fade", spawn_func_fade)?;
    game.named.register(
        &format!("{prefix}model_loop"),
        Q1CallbackHandlers {
            action: Some(model_loop),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}model_once"),
        Q1CallbackHandlers {
            action: Some(model_once),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}model_use_once"),
        Q1CallbackHandlers {
            use_callback: Some(model_use_once),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}model_use_loop"),
        Q1CallbackHandlers {
            use_callback: Some(model_use_loop),
            ..Default::default()
        },
    )?;
    game.register_spawn("misc_model", spawn_misc_model)?;
    game.register_spawn("info_rotate_axis", spawn_info_rotate_axis)?;
    game.named.register(
        &format!("{prefix}rotate_tick"),
        Q1CallbackHandlers {
            action: Some(rotate_tick),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}rotate_tween"),
        Q1CallbackHandlers {
            action: Some(rotate_tween),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}rotate_use"),
        Q1CallbackHandlers {
            use_callback: Some(rotate_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("rotate_object_continuously", spawn_rotate_object_continuously)?;
    if context.program() == crate::q1::addons::context::Q1AddonProgram::Mg3 {
        game.named.register(
            &format!("{prefix}breakable_stop"),
            Q1CallbackHandlers {
                action: Some(breakable_stop),
                ..Default::default()
            },
        )?;
        game.named.register(
            &format!("{prefix}breakable_pain"),
            Q1CallbackHandlers {
                pain: Some(breakable_pain),
                ..Default::default()
            },
        )?;
        game.named.register(
            &format!("{prefix}breakable_die"),
            Q1CallbackHandlers {
                die: Some(breakable_die),
                ..Default::default()
            },
        )?;
        game.register_spawn("func_breakable", spawn_func_breakable)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices, program: Q1AddonProgram) -> (Q1BaseGuard, Q1AddonContext) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(game, program);
        register_addon_brushes(&context, game).expect("brushes");
        (guard, context)
    }

    #[test]
    fn bob_defaults_and_ticks() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let bob = game.create("func_bob", None, None).expect("bob");
        spawn_func_bob(&mut game, &bob).expect("spawn");
        let entity = require_entity(&game, &bob).expect("entity");
        assert_eq!(
            entity.vector("dest"),
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 64.0
            }
        );
        assert_eq!(entity.wait, 10.0);
        assert_eq!(entity.solid, Q1Solid::Bsp);
        bob_use(&mut game, &bob, None, None).expect("use");
        assert!(require_entity(&game, &bob).expect("entity").activated);
    }

    #[test]
    fn toss_arms_and_throws() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let toss = game.create("func_toss", None, None).expect("toss");
        spawn_func_toss(&mut game, &toss).expect("spawn");
        assert_eq!(
            require_entity(&game, &toss).expect("entity").movedir,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 200.0
            }
        );
        toss_use(&mut game, &toss, None, None).expect("use");
        assert_eq!(require_entity(&game, &toss).expect("entity").solid, Q1Solid::Bbox);
    }

    #[test]
    fn explode_arms_and_detonates() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let explode = game.create("func_explode", None, None).expect("explode");
        spawn_func_explode(&mut game, &explode).expect("spawn");
        assert!(game.is_damageable(&explode));
        let player = attach_test_player(&mut game);
        explode_die(&mut game, &explode, Some(&player)).expect("die");
        assert!(!game.is_damageable(&explode));
    }

    #[test]
    fn hurt_toggles_and_burns() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let hurt = game.create("func_hurt", None, None).expect("hurt");
        game.update_entity(&hurt, |entity| entity.spawnflags = 1)
            .expect("flags");
        spawn_func_hurt(&mut game, &hurt).expect("spawn");
        assert!(require_entity(&game, &hurt).expect("entity").activated);
        let player = attach_test_player(&mut game);
        hurt_touch(&mut game, &hurt, &player, None, None).expect("touch");
        assert_eq!(
            require_entity(&game, &hurt).expect("entity").attack_finished,
            fround(0.2)
        );
        hurt_use(&mut game, &hurt, None, None).expect("use");
        assert!(!require_entity(&game, &hurt).expect("entity").activated);
    }

    #[test]
    fn models_validate_and_rotate_spins() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let invalid = game.create("misc_model", None, None).expect("invalid");
        assert!(spawn_misc_model(&mut game, &invalid).is_err());
        let model = game.create("misc_model", None, None).expect("model");
        game.update_entity(&model, |entity| entity.model = String::from("progs/model.mdl"))
            .expect("model");
        spawn_misc_model(&mut game, &model).expect("spawn");
        let rotate = game.create("rotate_object_continuously", None, None).expect("rotate");
        spawn_rotate_object_continuously(&mut game, &rotate).expect("spawn");
        assert_eq!(
            require_entity(&game, &rotate).expect("entity").number("rotate.state"),
            2.0
        );
        rotate_tick(&mut game, &rotate).expect("tick");
    }

    #[test]
    fn honey_breakable_absorbs_pain() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg3);
        let breakable = game.create("func_breakable", None, None).expect("breakable");
        spawn_func_breakable(&mut game, &breakable).expect("spawn");
        assert_eq!(require_entity(&game, &breakable).expect("entity").max_health, 10000.0);
        breakable_pain(&mut game, &breakable, None, 10.0).expect("pain");
        assert_eq!(game.health(&breakable), 10000.0);
    }
}
