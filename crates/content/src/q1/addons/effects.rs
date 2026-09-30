//! Q1 addon effects (`src/content/q1/addons/effects.ts`).
//!
//! `quakec_mg1/misc_fx.qc`, `fog.qc`, and `quakec_mg3` variants.
//! Copyright (C) 1996-2026 id Software LLC. GPL-2.0-or-later.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::Vec3;

use crate::q1::addons::context::{
    addon_alpha, addon_emit, addon_frame_time, addon_player_reference, addon_program, fround, init_trigger,
    removed_outside_coop, require_entity, set_addon_number, set_addon_player_number, set_addon_player_reference,
    set_addon_vector, Q1AddonContext, Q1AddonEvent, Q1AddonLightningStyle, Q1AddonProgram,
};
use crate::q1::foundation::callbacks::{callback_name, Q1CallbackHandlers};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{
    length, normalize, vadd, vscale, vsub, Q1Event, Q1SoundChannel, Q1TraceRequest, POINT, ZERO,
};
use crate::q1::{q1_error, Q1Error};

fn fx_prefix(game: &Q1EntityServices) -> Result<String, Q1Error> {
    Ok(format!("{}:fx:", addon_program(game)?.as_str()))
}

fn fog_prefix(game: &Q1EntityServices) -> Result<String, Q1Error> {
    Ok(format!("{}:fog:", addon_program(game)?.as_str()))
}

fn schedule_fx(game: &mut Q1EntityServices, id: &ActorId, name: &str, delay: f64) -> Result<(), Q1Error> {
    let callback = game.named.action(&format!("{}{name}", fx_prefix(game)?))?;
    game.schedule(id, delay, &callback)
}

/// Signed unit random draw (`random`).
fn random_draw(game: &mut Q1EntityServices) -> f64 {
    fround(game.host.random() * 2.0 - 1.0)
}

fn shake_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.intermission.is_some() {
        return Ok(());
    }
    let entity = require_entity(game, id)?.clone();
    let end = entity.number("storednextthink");
    let start = end - entity.wait;
    if game.time > end {
        if (entity.spawnflags & 1) == 0 {
            game.sound_simple(id, &entity.text("noise1"))?;
        }
        for player in (game.host.players)() {
            addon_emit(game, Q1AddonEvent::ViewRoll { player, roll: 0.0 })?;
        }
        return Ok(());
    }
    let intensity = fround(
        entity.damage
            * if game.time < entity.delay {
                (game.time - start) / (entity.wait / 3.0)
            } else {
                1.0
            },
    );
    for player in (game.host.players)() {
        let x = fround(game.host.random() * intensity);
        let y = fround(random_draw(game) * intensity);
        let z = fround(game.host.random() * intensity);
        addon_emit(
            game,
            Q1AddonEvent::PunchAngle {
                player,
                angles: Vec3 {
                    x: x as f32,
                    y: y as f32,
                    z: z as f32,
                },
            },
        )?;
    }
    schedule_fx(game, id, "shake", 0.05)
}

fn shake_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    set_addon_number(game, id, "storednextthink", game.time + entity.wait)?;
    let delay = fround(game.time + entity.wait / 3.0);
    game.update_entity(id, |entity| entity.delay = delay)?;
    if (entity.spawnflags & 1) == 0 {
        game.sound_simple(id, &entity.text("noise"))?;
    }
    schedule_fx(game, id, "shake", 0.05)
}

fn spawn_trigger_screenshake(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    game.update_entity(id, |entity| {
        if entity.wait == 0.0 {
            entity.wait = 2.0;
        }
        if entity.damage == 0.0 {
            entity.damage = 3.0;
        }
        entity
            .fields
            .insert(String::from("noise"), String::from("misc/quake.wav"));
        entity
            .fields
            .insert(String::from("noise1"), String::from("misc/quakeend.wav"));
    })?;
    let use_callback = game.named.use_callback(&format!("{}shake", fx_prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn sound_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let noise = require_entity(game, id)?.text("noise");
    if noise.is_empty() {
        return Ok(());
    }
    game.sound_simple(id, &noise)
}

fn spawn_trigger_sound(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if require_entity(game, id)?.text("noise").is_empty() {
        return game.remove(id);
    }
    let use_callback = game.named.use_callback(&format!("{}sound", fx_prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn damage_lightning(game: &mut Q1EntityServices, id: &ActorId, start: Vec3, end: Vec3) -> Result<(), Q1Error> {
    // QC discards normalize's return, then assigns y from the already
    // changed x.
    let entity = require_entity(game, id)?.clone();
    let difference = vsub(end, start);
    let side = Vec3 {
        x: fround(f64::from(-difference.y) * 16.0) as f32,
        y: fround(f64::from(-difference.y) * 16.0) as f32,
        z: 0.0,
    };
    let mut hits: Vec<ActorId> = Vec::new();
    for offset in [ZERO, side, vscale(side, -1.0)] {
        let trace = game.host.trace(&Q1TraceRequest {
            start: vadd(start, offset),
            end: vadd(end, offset),
            bounds: POINT,
            ignore: Some(id.clone()),
            monsters: true,
            missile: false,
        });
        let Some(target) = trace.actor.clone() else {
            continue;
        };
        if hits.iter().any(|hit| same_actor(hit, &target)) {
            continue;
        }
        hits.push(target.clone());
        if !game
            .host
            .combat
            .read(&target)
            .is_some_and(|combat| combat.can_take_damage)
        {
            continue;
        }
        game.host.emit(Q1Event::Particles {
            origin: trace.end,
            direction: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 100.0,
            },
            color: 225,
            count: (entity.damage * 4.0) as i32,
        });
        game.damage(&target, Some(id), Some(id), entity.damage, &Q1DamageParams::default());
    }
    Ok(())
}

fn lightning_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let program = addon_program(game)?;
    let entity = require_entity(game, id)?.clone();
    let targets = game.find(&entity.target);
    let chosen = if (entity.spawnflags & 1) != 0 {
        (targets.len() as f64 * game.host.random()).floor() as i32
    } else {
        -1
    };
    for (index, target) in targets.iter().enumerate() {
        if chosen >= 0 && chosen != index as i32 {
            continue;
        }
        let reverse = (entity.spawnflags & 2) != 0;
        let start = game.body(if reverse { target } else { id })?.origin;
        let trace = game.host.trace(&Q1TraceRequest {
            start,
            end: game.body(if reverse { id } else { target })?.origin,
            bounds: POINT,
            ignore: Some(id.clone()),
            monsters: false,
            missile: false,
        });
        if program != Q1AddonProgram::Mg3 || (entity.spawnflags & 16) == 0 {
            game.host.emit(Q1Event::Sound {
                origin: None,
                actor: target.clone(),
                path: entity.text("noise"),
                channel: Q1SoundChannel::Auto,
                attenuation: 1.0,
                volume: entity.number("volume"),
            });
        }
        let style = entity.number("style");
        addon_emit(
            game,
            Q1AddonEvent::Lightning {
                actor: target.clone(),
                start,
                end: trace.end,
                style: if style == 1.0 {
                    Q1AddonLightningStyle::Style1
                } else if style == 2.0 {
                    Q1AddonLightningStyle::Style2
                } else {
                    Q1AddonLightningStyle::Style3
                },
            },
        )?;
        if entity.damage != 0.0 {
            damage_lightning(game, id, start, trace.end)?;
        }
        let target_entity = require_entity(game, target)?.clone();
        if (entity.spawnflags & 4) != 0 || target_entity.target.is_empty() {
            continue;
        }
        game.use_targets(target, activator)?;
        if (entity.spawnflags & 8) == 0 && game.is_live(target) {
            let delay = target_entity.delay;
            game.update_entity(target, |entity| entity.delay = fround(delay + 0.2))?;
            game.use_targets(target, activator)?;
            game.update_entity(target, |entity| entity.delay = delay)?;
        }
    }
    Ok(())
}

fn spawn_trigger_lightning(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let program = addon_program(game)?;
    if require_entity(game, id)?.text("noise").is_empty() {
        game.update_entity(id, |entity| {
            entity
                .fields
                .insert(String::from("noise"), String::from("misc/power.wav"));
        })?;
    }
    if require_entity(game, id)?.number("volume") == 0.0 {
        set_addon_number(game, id, "volume", 1.0)?;
    }
    if program == Q1AddonProgram::Mg3 && (require_entity(game, id)?.spawnflags & 16) != 0 {
        set_addon_number(game, id, "volume", 0.0)?;
    }
    let use_callback = game.named.use_callback(&format!("{}lightning", fx_prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn fade_targets_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    let targets = game.find(&entity.target);
    let mut count = 0;
    if !entity.activated {
        for target in &targets {
            addon_alpha(game, target, 1.0)?;
        }
        game.update_entity(id, |entity| entity.activated = true)?;
    }
    for target in &targets {
        if game.health(target) <= 0.0 {
            if require_entity(game, target)?.number("alpha") > 0.0 {
                let alpha =
                    fround(require_entity(game, target)?.number("alpha") - addon_frame_time(game)? / entity.delay);
                addon_alpha(game, target, alpha)?;
                count += 1;
            } else {
                game.remove(target)?;
            }
        }
    }
    if count > 0 {
        schedule_fx(game, id, "fade_targets", 0.0)
    } else {
        game.remove(id)
    }
}

fn fade_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    let manager = game.create("fade_manager", None, None)?;
    game.update_entity(&manager, |manager| {
        manager.target = entity.target.clone();
        manager.delay = entity.delay;
    })?;
    schedule_fx(game, &manager, "fade_targets", 0.0)
}

fn spawn_trigger_fade(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    game.update_entity(id, |entity| {
        if entity.delay == 0.0 {
            entity.delay = 1.0;
        }
    })?;
    if !game.is_live(id) {
        return Ok(());
    }
    let use_callback = game.named.use_callback(&format!("{}fade", fx_prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn freeze_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let target_name = require_entity(game, id)?.target.clone();
    for target in game.find(&target_name) {
        if require_entity(game, &target)?.number("is_frozen") == 0.0 {
            let entity = require_entity(game, &target)?.clone();
            set_addon_number(game, &target, "storednextthink", entity.next_think)?;
            let stored = callback_name(entity.think.as_ref()).unwrap_or_default();
            game.update_entity(&target, |entity| {
                entity.fields.insert(String::from("addon.storedthink"), stored);
            })?;
            set_addon_number(
                game,
                &target,
                "addon.frozenDamageable",
                if game.is_damageable(&target) { 1.0 } else { 0.0 },
            )?;
            game.set_damageable(&target, false)?;
            game.cancel(&target);
            let think = game.named.action("SUB_Null")?;
            game.update_entity(&target, |entity| entity.think = Some(think))?;
            set_addon_number(game, &target, "is_frozen", 1.0)?;
        } else {
            let entity = require_entity(game, &target)?.clone();
            let name = entity.text("addon.storedthink");
            let due = entity.number("storednextthink");
            let think = if name.is_empty() {
                None
            } else {
                Some(game.named.action(&name)?)
            };
            game.update_entity(&target, |entity| entity.think = think.clone())?;
            if let Some(think) = think {
                if due >= 0.0 {
                    game.schedule(&target, due - game.time, &think)?;
                }
            }
            set_addon_number(game, &target, "storednextthink", -1.0)?;
            set_addon_number(game, &target, "is_frozen", 0.0)?;
            let damageable = require_entity(game, &target)?.number("addon.frozenDamageable") != 0.0;
            game.set_damageable(&target, damageable)?;
        }
    }
    Ok(())
}

fn spawn_trigger_freeze(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    if !game.is_live(id) {
        return Ok(());
    }
    let use_callback = game.named.use_callback(&format!("{}freeze", fx_prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn embers_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let body = game.body(id)?;
    let entity = require_entity(game, id)?.clone();
    let size = entity.vector("size");
    let up = fround(game.host.random() * 2.0 + 2.0);
    let direction = Vec3 {
        x: fround(random_draw(game) * f64::from(body.velocity.x)) as f32,
        y: fround(random_draw(game) * f64::from(body.velocity.y)) as f32,
        z: fround(up * f64::from(body.velocity.z)) as f32,
    };
    let jitter_x = random_draw(game) as f32;
    let jitter_y = random_draw(game) as f32;
    game.host.emit(Q1Event::Particles {
        origin: Vec3 {
            x: body.origin.x + size.x * jitter_x,
            y: body.origin.y + size.y * jitter_y,
            z: body.origin.z,
        },
        direction,
        color: 234,
        count: 2,
    });
    let delay = entity.wait + entity.delay * game.host.random();
    schedule_fx(game, id, "embers", delay)
}

fn spawn_embers(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let tall = require_entity(game, id)?.classname == "particle_embers_tall";
    if length(require_entity(game, id)?.vector("size")) == 0.0 {
        set_addon_vector(
            game,
            id,
            "size",
            if tall {
                Vec3 {
                    x: 40.0,
                    y: 40.0,
                    z: 0.0,
                }
            } else {
                Vec3 {
                    x: 128.0,
                    y: 128.0,
                    z: 0.0,
                }
            },
        )?;
    }
    game.update_entity(id, |entity| {
        if entity.wait == 0.0 {
            entity.wait = 0.05;
        }
        if entity.delay == 0.0 {
            entity.delay = 0.1;
        }
    })?;
    if length(game.body(id)?.velocity) == 0.0 {
        game.set_body(
            id,
            &BodyPatch {
                velocity: Some(Vec3 {
                    x: 1.0,
                    y: 1.0,
                    z: if tall { 2.0 } else { 1.0 },
                }),
                ..Default::default()
            },
        )?;
    }
    let entity = require_entity(game, id)?.clone();
    let delay = entity.wait + entity.delay * game.host.random();
    schedule_fx(game, id, "embers", delay)
}

fn tele_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let program = addon_program(game)?;
    let entity = require_entity(game, id)?.clone();
    let dx = random_draw(game) as f32 * 10.0;
    let dy = random_draw(game) as f32 * 10.0;
    let dz = random_draw(game) as f32 * 5.0;
    let direction = normalize(Vec3 { x: dx, y: dy, z: dz });
    let distance = if program == Q1AddonProgram::Mg3 {
        entity.number("distance")
    } else {
        64.0
    };
    let origin = game.body(id)?.origin;
    game.host.emit(Q1Event::Particles {
        origin: vadd(origin, vscale(direction, distance)),
        direction: vscale(direction, distance * -0.125),
        color: 3,
        count: 3,
    });
    let delay = entity.wait + entity.delay * game.host.random();
    schedule_fx(game, id, "tele", delay)
}

fn spawn_particle_tele(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        if entity.delay == 0.0 {
            entity.delay = 0.1;
        }
    })?;
    if require_entity(game, id)?.number("distance") == 0.0 {
        set_addon_number(game, id, "distance", 64.0)?;
    }
    let entity = require_entity(game, id)?.clone();
    let delay = entity.wait + entity.delay * game.host.random();
    schedule_fx(game, id, "tele", delay)
}

fn fountain_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let body = game.body(id)?;
    let entity = require_entity(game, id)?.clone();
    let jitter_x = random_draw(game) as f32 * body.velocity.x;
    let jitter_y = random_draw(game) as f32 * body.velocity.y;
    game.host.emit(Q1Event::Particles {
        origin: body.origin,
        direction: Vec3 {
            x: jitter_x,
            y: jitter_y,
            z: body.velocity.z,
        },
        color: 13,
        count: 2,
    });
    let delay = entity.wait + entity.delay * game.host.random();
    schedule_fx(game, id, "fountain", delay)
}

fn fountain_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    schedule_fx(game, id, "fountain", 0.1)
}

fn spawn_particle_tele_fountain(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        if entity.wait == 0.0 {
            entity.wait = 0.05;
        }
        if entity.delay == 0.0 {
            entity.delay = 0.1;
        }
    })?;
    if length(game.body(id)?.velocity) == 0.0 {
        game.set_body(
            id,
            &BodyPatch {
                velocity: Some(Vec3 { x: 1.0, y: 1.0, z: 6.0 }),
                ..Default::default()
            },
        )?;
    }
    if (require_entity(game, id)?.spawnflags & 1) != 0 {
        let use_callback = game.named.use_callback(&format!("{}fountain", fx_prefix(game)?))?;
        return game.update_entity(id, |entity| entity.use_callback = Some(use_callback));
    }
    let entity = require_entity(game, id)?.clone();
    let delay = entity.wait + entity.delay * game.host.random();
    schedule_fx(game, id, "fountain", delay)
}

/// Register addon effects (`registerAddonEffects`).
pub fn register_addon_effects(context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let prefix = format!("{}:fx:", context.program().as_str());
    game.named.register(
        &format!("{prefix}shake"),
        Q1CallbackHandlers {
            action: Some(shake_action),
            use_callback: Some(shake_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_screenshake", spawn_trigger_screenshake)?;
    game.named.register(
        &format!("{prefix}sound"),
        Q1CallbackHandlers {
            use_callback: Some(sound_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_sound", spawn_trigger_sound)?;
    game.named.register(
        &format!("{prefix}lightning"),
        Q1CallbackHandlers {
            use_callback: Some(lightning_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_lightning", spawn_trigger_lightning)?;
    game.named.register(
        &format!("{prefix}fade_targets"),
        Q1CallbackHandlers {
            action: Some(fade_targets_action),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}fade"),
        Q1CallbackHandlers {
            use_callback: Some(fade_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_fade", spawn_trigger_fade)?;
    game.named.register(
        &format!("{prefix}freeze"),
        Q1CallbackHandlers {
            use_callback: Some(freeze_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_freeze", spawn_trigger_freeze)?;
    game.named.register(
        &format!("{prefix}embers"),
        Q1CallbackHandlers {
            action: Some(embers_action),
            ..Default::default()
        },
    )?;
    game.register_spawn("particle_embers", spawn_embers)?;
    game.register_spawn("particle_embers_tall", spawn_embers)?;
    game.named.register(
        &format!("{prefix}tele"),
        Q1CallbackHandlers {
            action: Some(tele_action),
            ..Default::default()
        },
    )?;
    game.register_spawn("particle_tele", spawn_particle_tele)?;
    game.named.register(
        &format!("{prefix}fountain"),
        Q1CallbackHandlers {
            action: Some(fountain_action),
            use_callback: Some(fountain_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("particle_tele_fountain", spawn_particle_tele_fountain)?;
    Ok(())
}

fn fog_info(game: &Q1EntityServices, id: &ActorId, field: &str) -> Result<Option<ActorId>, Q1Error> {
    let name = require_entity(game, id)?.text(field);
    if name.is_empty() {
        return Ok(None);
    }
    let source = game.find(&name).first().cloned();
    Ok(match source {
        Some(source)
            if game
                .entity_ref(&source)
                .is_some_and(|entity| entity.classname == "info_fog") =>
        {
            Some(source)
        }
        _ => None,
    })
}

fn set_fog(
    game: &mut Q1EntityServices,
    player: &ActorId,
    density: f64,
    color: Vec3,
    duration: f64,
) -> Result<(), Q1Error> {
    set_addon_player_number(game, player, "fog_density", density)?;
    set_addon_player_number(game, player, "fog_color_x", f64::from(color.x))?;
    set_addon_player_number(game, player, "fog_color_y", f64::from(color.y))?;
    set_addon_player_number(game, player, "fog_color_z", f64::from(color.z))?;
    addon_emit(
        game,
        Q1AddonEvent::Fog {
            player: Some(player.clone()),
            density,
            color,
            duration,
            sky_factor: 0.0,
        },
    )
}

fn fog_activate(game: &mut Q1EntityServices, id: &ActorId, player: Option<&ActorId>) -> Result<(), Q1Error> {
    let Some(player) = player else {
        return Ok(());
    };
    if !game.is_player(player) {
        return Ok(());
    }
    let previous = addon_player_reference(game, player, "fog.active")?;
    if previous.as_ref().is_some_and(|previous| same_actor(previous, id)) {
        return Ok(());
    }
    set_addon_player_reference(game, player, "fog.active", Some(id))?;
    let source = fog_info(game, id, "fog_info_entity")?;
    let Some(source) = source else {
        return Ok(());
    };
    let entity = require_entity(game, &source)?.clone();
    let density = entity.number("fog_density");
    let color = entity.vector("fog_color");
    if density != 0.0 || length(color) != 0.0 || (entity.spawnflags & 1) != 0 {
        set_fog(game, player, density, color, require_entity(game, id)?.delay)?;
    }
    Ok(())
}

fn fog_activate_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    fog_activate(game, id, Some(other))
}

fn fog_activate_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    fog_activate(game, id, other)
}

fn spawn_info_fog(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if require_entity(game, id)?.number("fog_density") == 0.0 {
        set_addon_number(game, id, "fog_density", 0.05)?;
    } else {
        let color = require_entity(game, id)?.vector("fog_color");
        if color.x > 1.0 || color.y > 1.0 || color.z > 1.0 {
            set_addon_vector(game, id, "fog_color", vscale(color, 1.0 / 255.0))?;
        }
    }
    Ok(())
}

fn spawn_trigger_fog(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.options().coop || game.options().deathmatch != 0 {
        return game.remove(id);
    }
    game.update_entity(id, |entity| {
        if entity.delay == 0.0 {
            entity.delay = 0.5;
        }
    })?;
    init_trigger(game, id)?;
    if !game.is_live(id) {
        return Ok(());
    }
    let activate = format!("{}activate", fog_prefix(game)?);
    let use_callback = game.named.use_callback(&activate)?;
    let touch = if (require_entity(game, id)?.spawnflags & 1) == 0 {
        Some(game.named.touch(&activate)?)
    } else {
        None
    };
    game.update_entity(id, |entity| {
        entity.use_callback = Some(use_callback);
        entity.touch = touch;
    })
}

fn fog_transition_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    player: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(player) {
        return Ok(());
    }
    let body = match game.host.bodies.read(player) {
        Some(body) => body,
        None => return Ok(()),
    };
    let entity = require_entity(game, id)?.clone();
    let bounds = game.body(id)?.bounds;
    let size = vsub(bounds.max, bounds.min);
    let position = vsub(body.origin, bounds.min);
    let axis = entity.number("style");
    let raw = if axis == 0.0 {
        f64::from(position.x) / f64::from(size.x)
    } else if axis == 1.0 {
        f64::from(position.y) / f64::from(size.y)
    } else {
        f64::from(position.z) / f64::from(size.z)
    };
    let tween = if raw.is_nan() { raw } else { raw.clamp(0.0, 1.0) };
    let first = fog_info(game, id, "fog_info_entity")?;
    let second = fog_info(game, id, "target")?;
    let first_density = first
        .as_ref()
        .and_then(|first| game.entity_ref(first).map(|entity| entity.number("fog_density")))
        .unwrap_or(0.0);
    let second_density = second
        .as_ref()
        .and_then(|second| game.entity_ref(second).map(|entity| entity.number("fog_density")))
        .unwrap_or(0.0);
    let first_color = first
        .as_ref()
        .and_then(|first| game.entity_ref(first).map(|entity| entity.vector("fog_color")))
        .unwrap_or(ZERO);
    let second_color = second
        .as_ref()
        .and_then(|second| game.entity_ref(second).map(|entity| entity.vector("fog_color")))
        .unwrap_or(ZERO);
    set_fog(
        game,
        player,
        (1.0 - tween) * first_density + tween * second_density,
        vadd(vscale(first_color, 1.0 - tween), vscale(second_color, tween)),
        0.0,
    )
}

fn spawn_trigger_fog_transition(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.options().coop || game.options().deathmatch != 0 {
        return game.remove(id);
    }
    let axis = require_entity(game, id)?.number("style");
    if axis != 0.0 && axis != 1.0 && axis != 2.0 {
        return Err(q1_error("Invalid style for trigger_fog_transition"));
    }
    init_trigger(game, id)?;
    if !game.is_live(id) {
        return Ok(());
    }
    let touch = game.named.touch(&format!("{}transition", fog_prefix(game)?))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))
}

/// Register addon fog (`registerAddonFog`).
pub fn register_addon_fog(context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let prefix = format!("{}:fog:", context.program().as_str());
    game.named.register(
        &format!("{prefix}activate"),
        Q1CallbackHandlers {
            touch: Some(fog_activate_touch),
            use_callback: Some(fog_activate_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("info_fog", spawn_info_fog)?;
    game.register_spawn("trigger_fog", spawn_trigger_fog)?;
    game.named.register(
        &format!("{prefix}transition"),
        Q1CallbackHandlers {
            touch: Some(fog_transition_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_fog_transition", spawn_trigger_fog_transition)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{
        addon_player_number, attach_test_player, register_test_addons, test_addon_events,
    };
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices, program: Q1AddonProgram) -> (Q1BaseGuard, Q1AddonContext) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(game, program);
        register_addon_effects(&context, game).expect("effects");
        register_addon_fog(&context, game).expect("fog");
        (guard, context)
    }

    #[test]
    fn screenshake_arms_and_finishes() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let shake = game.create("trigger_screenshake", None, None).expect("shake");
        spawn_trigger_screenshake(&mut game, &shake).expect("spawn");
        let entity = require_entity(&game, &shake).expect("entity");
        assert_eq!(entity.wait, 2.0);
        assert_eq!(entity.damage, 3.0);
        shake_use(&mut game, &shake, None, None).expect("use");
        assert!(require_entity(&game, &shake).expect("entity").think.is_some());
        game.time = 99.0;
        shake_action(&mut game, &shake).expect("finish");
    }

    #[test]
    fn lightning_defaults_and_emits() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let lightning = game.create("trigger_lightning", None, None).expect("lightning");
        spawn_trigger_lightning(&mut game, &lightning).expect("spawn");
        let entity = require_entity(&game, &lightning).expect("entity");
        assert_eq!(entity.text("noise"), "misc/power.wav");
        assert_eq!(entity.number("volume"), 1.0);
        let target = game.create("info_null", None, None).expect("target");
        game.update_entity(&target, |entity| entity.targetname = String::from("zap"))
            .expect("name");
        game.update_entity(&lightning, |entity| entity.target = String::from("zap"))
            .expect("target");
        lightning_use(&mut game, &lightning, None, None).expect("use");
        assert_eq!(test_addon_events(&game).expect("events").len(), 1);
    }

    #[test]
    fn freeze_holds_and_releases_think() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let target = game.create("info_null", None, None).expect("target");
        game.update_entity(&target, |entity| {
            entity.targetname = String::from("frozen");
            entity.think = Some(String::from("SUB_Null"));
            entity.next_think = 5.0;
        })
        .expect("think");
        let freeze = game.create("trigger_freeze", None, None).expect("freeze");
        game.update_entity(&freeze, |entity| entity.target = String::from("frozen"))
            .expect("target");
        spawn_trigger_freeze(&mut game, &freeze).expect("spawn");
        freeze_use(&mut game, &freeze, None, None).expect("freeze");
        assert_eq!(require_entity(&game, &target).expect("entity").number("is_frozen"), 1.0);
        freeze_use(&mut game, &freeze, None, None).expect("thaw");
        assert_eq!(require_entity(&game, &target).expect("entity").number("is_frozen"), 0.0);
    }

    #[test]
    fn particles_schedule_themselves() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let embers = game.create("particle_embers", None, None).expect("embers");
        spawn_embers(&mut game, &embers).expect("spawn");
        assert_eq!(
            require_entity(&game, &embers).expect("entity").vector("size"),
            Vec3 {
                x: 128.0,
                y: 128.0,
                z: 0.0
            }
        );
        embers_action(&mut game, &embers).expect("tick");
        let tele = game.create("particle_tele", None, None).expect("tele");
        spawn_particle_tele(&mut game, &tele).expect("spawn");
        assert_eq!(require_entity(&game, &tele).expect("entity").number("distance"), 64.0);
        let fountain = game.create("particle_tele_fountain", None, None).expect("fountain");
        spawn_particle_tele_fountain(&mut game, &fountain).expect("spawn");
        fountain_action(&mut game, &fountain).expect("tick");
    }

    #[test]
    fn fog_activates_and_transitions() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let info = game.create("info_fog", None, None).expect("info");
        game.update_entity(&info, |entity| entity.targetname = String::from("foggy"))
            .expect("name");
        spawn_info_fog(&mut game, &info).expect("spawn");
        assert_eq!(
            require_entity(&game, &info).expect("entity").number("fog_density"),
            fround(0.05)
        );
        let trigger = game.create("trigger_fog", None, None).expect("trigger");
        game.update_entity(&trigger, |entity| {
            entity
                .fields
                .insert(String::from("fog_info_entity"), String::from("foggy"));
        })
        .expect("info");
        spawn_trigger_fog(&mut game, &trigger).expect("spawn");
        let player = attach_test_player(&mut game);
        fog_activate_touch(&mut game, &trigger, &player, None, None).expect("touch");
        assert_eq!(addon_player_number(&game, &player, "fog_density"), Ok(fround(0.05)));
        let transition = game.create("trigger_fog_transition", None, None).expect("transition");
        assert!(spawn_trigger_fog_transition(&mut game, &transition).is_ok());
        game.update_entity(&transition, |entity| {
            entity.fields.insert(String::from("style"), String::from("9"));
        })
        .expect("style");
        assert!(spawn_trigger_fog_transition(&mut game, &transition).is_err());
    }
}
