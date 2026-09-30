//! Q1 triggers, lights, barrels, and map spawn dispatch (`src/content/q1/foundation/spawns.ts`).
//!
//! Original `triggers.qc`/`misc.qc`/`world.qc`/`client.qc` spawn
//! functions plus the official spawn dispatcher. Unregistered
//! classnames throw with the source ordinal instead of disappearing.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use super::entity::move_direction;
use super::entity_services::{Q1DamageParams, Q1EntityServices};
use super::monsters::{path_end_time, spawn_monster};
use super::movers::{spawn_button, spawn_door, spawn_plat, spawn_secret_door};
use super::pickups::spawn_pickup;
use super::precache_world::precache_q1_world;
use super::types::{
    dot, vadd, vscale, vsub, yaw_for, Q1Edition, Q1Effect, Q1Event, Q1MoveType, Q1Powerup, Q1Solid, Q1TraceRequest,
    ZERO,
};
use crate::q1::{q1_error, Q1Error};

pub use super::movers::link_doors;

fn init_trigger(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let angles = game.body(&id).map(|body| body.angles)?;
    let movedir = if angles.x == 0.0 && angles.y == 0.0 && angles.z == 0.0 {
        ZERO
    } else {
        move_direction(angles, Some(game))
    };
    game.update_entity(&id, |entity| {
        entity.movedir = movedir;
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::None;
        entity.model.clear();
    })?;
    game.set_body(
        &id,
        &super::gameplay::BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )
}

fn spawn_multi(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    init_trigger(game, &id)?;
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let secret = entity.classname == "trigger_secret";
    let once = secret || entity.classname == "trigger_once";
    game.update_entity(&id, |entity| {
        entity.wait = if once {
            -1.0
        } else if entity.wait == 0.0 {
            0.2
        } else {
            entity.wait
        };
        if secret {
            if entity.message.is_empty() {
                entity.message = String::from("$qc_found_secret");
            }
            if entity.sounds == 0 {
                entity.sounds = 1;
            }
        }
    })?;
    game.total_secrets += i32::from(secret);
    let sounds = game.entity_ref(&id).map(|entity| entity.sounds).unwrap_or(0);
    let sound = match sounds {
        1 => Some("misc/secret.wav"),
        2 => Some("misc/talk.wav"),
        3 => Some("misc/trigger1.wav"),
        _ => None,
    };
    if game.uses_id1_precaches() && secret && (sounds == 1 || sounds == 2) {
        if let Some(sound) = sound {
            game.precache_sound(sound)?;
        }
    }
    if game.uses_id1_precaches() {
        if let Some(sound) = sound {
            game.precache_sound(sound)?;
        }
    }
    let use_callback = game.named.use_callback("multi_use")?;
    game.update_entity(&id, |entity| entity.use_callback = Some(use_callback))?;
    let entity = game.entity_ref(&id).cloned().expect("trigger");
    if entity.max_health > 0.0 {
        if (entity.spawnflags & 1) != 0 {
            return Err(q1_error("health and notouch do not make sense"));
        }
        let die = game.named.die("multi_killed")?;
        game.update_entity(&id, |entity| {
            entity.solid = Q1Solid::Bbox;
            entity.die = Some(die);
        })?;
        game.set_damageable(&id, true)?;
    } else if (entity.spawnflags & 1) == 0 {
        let touch = game.named.touch("multi_touch")?;
        game.update_entity(&id, |entity| entity.touch = Some(touch))?;
    }
    Ok(())
}

fn spawn_counter(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    game.update_entity(&id, |entity| {
        entity.model.clear();
        if entity.count == 0.0 {
            entity.count = 2.0;
        }
    })?;
    let use_callback = game.named.use_callback("counter_use")?;
    game.update_entity(&id, |entity| entity.use_callback = Some(use_callback))
}

fn spawn_teleport_trigger(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    init_trigger(game, &id)?;
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if entity.target.is_empty() {
        return Err(q1_error("trigger_teleport has no target"));
    }
    let use_callback = game.named.use_callback("teleport_use")?;
    game.update_entity(&id, |entity| entity.use_callback = Some(use_callback))?;
    if (entity.spawnflags & 2) == 0 {
        if game.uses_id1_precaches() {
            game.precache_sound("ambience/hum1.wav")?;
        }
        let bounds = game.body(&id)?.bounds;
        game.host.emit(Q1Event::Ambient {
            origin: vscale(vadd(bounds.min, bounds.max), 0.5),
            path: String::from("ambience/hum1.wav"),
            volume: 0.5,
            attenuation: 3.0,
        });
    }
    let touch = game.named.touch("teleport_touch")?;
    game.update_entity(&id, |entity| entity.touch = Some(touch))
}

fn spawn_light(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if entity.classname == "light" && entity.targetname.is_empty() {
        return game.remove(&id);
    }
    let style = entity.number("style");
    if style >= 32.0 && entity.classname != "light_fluorospark" {
        let use_callback = game.named.use_callback("light_use")?;
        game.update_entity(&id, |entity| entity.use_callback = Some(use_callback))?;
        game.host.emit(Q1Event::Lightstyle {
            style: style as i32,
            pattern: String::from(if (entity.spawnflags & 1) != 0 { "a" } else { "m" }),
        });
    }
    if game.uses_id1_precaches() && entity.classname == "light_fluoro" {
        game.precache_sound("ambience/fl_hum1.wav")?;
    }
    if game.uses_id1_precaches() && entity.classname == "light_fluorospark" {
        game.precache_sound("ambience/buzz1.wav")?;
    }
    if entity.classname == "light_fluoro" || entity.classname == "light_fluorospark" {
        let origin = game.body(&id).map(|body| body.origin)?;
        let path = if entity.classname == "light_fluoro" {
            "ambience/fl_hum1.wav"
        } else {
            "ambience/buzz1.wav"
        };
        game.host.emit(Q1Event::Ambient {
            origin,
            path: String::from(path),
            volume: 0.5,
            attenuation: 3.0,
        });
    }
    Ok(())
}

fn spawn_barrel(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let model = if entity.classname == "misc_explobox2" {
        "maps/b_exbox2.bsp"
    } else {
        "maps/b_explob.bsp"
    };
    if game.uses_id1_precaches() {
        game.precache_model(model)?;
    }
    if game.uses_id1_precaches() {
        game.precache_sound("weapons/r_exp3.wav")?;
    }
    game.update_entity(&id, |entity| {
        entity.model = String::from(model);
        entity.solid = Q1Solid::Bbox;
        entity.movement = Q1MoveType::None;
        entity.aimed_damage = true;
    })?;
    game.set_damageable(&id, true)?;
    game.set_health(&id, 20.0)?;
    let tall = entity.classname != "misc_explobox2";
    game.set_bounds(
        &id,
        Bounds {
            min: ZERO,
            max: Vec3 {
                x: 32.0,
                y: 32.0,
                z: if tall { 64.0 } else { 32.0 },
            },
        },
    )?;
    let die = game.named.die("barrel_die")?;
    game.update_entity(&id, |entity| entity.die = Some(die))?;
    let body = game.body(&id)?;
    let start = vadd(body.origin, Vec3 { x: 0.0, y: 0.0, z: 2.0 });
    game.set_body(
        &id,
        &super::gameplay::BodyPatch {
            origin: Some(start),
            ..Default::default()
        },
    )?;
    let trace = game.host.trace(&Q1TraceRequest {
        start,
        end: vadd(
            start,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: -256.0,
            },
        ),
        bounds: body.bounds,
        ignore: Some(id.clone()),
        monsters: true,
        missile: false,
    });
    if trace.fraction < 1.0 && !trace.all_solid {
        game.set_body(
            &id,
            &super::gameplay::BodyPatch {
                origin: Some(trace.end),
                ground: Some(trace.actor.clone()),
                ..Default::default()
            },
        )?;
        game.update_entity(&id, |entity| entity.movement_flags |= 512)?;
        if start.z - trace.end.z > 250.0 {
            return game.remove(&id);
        }
    }
    Ok(())
}

const WORLD_LIGHTSTYLES: [&str; 12] = [
    "m",
    "mmnmmommommnonmmonqnmmo",
    "abcdefghijklmnopqrstuvwxyzyxwvutsrqponmlkjihgfedcba",
    "mmmmmaaaaammmmmaaaaaabcdefgabcdefg",
    "mamamamamama",
    "jklmnopqrstuvwxyzyxwvutsrqponmlkj",
    "nmonqnmomnmomomno",
    "mmmaaaabcdefgmmmmaaaammmaamm",
    "mmmaaammmaaammmabcdefaaaammmmabcdefmmmaaaa",
    "aaaaaaaazzzzzzzz",
    "mmamammmmammamamaaamammma",
    "abcdefghijklmnopqrrqponmlkjihgfedcba",
];

/// Spawn one official map entity (`spawnMapActor`).
pub fn spawn_map_actor(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    if spawn_pickup(game, &id)? {
        return Ok(());
    }
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    match entity.classname.as_str() {
        "worldspawn" => {
            game.world = Some(id.clone());
            game.world_type = entity.number("worldtype") as i32;
            game.update_entity(&id, |entity| entity.solid = Q1Solid::Bsp)?;
            if game.uses_id1_precaches() {
                precache_q1_world(game)?;
            }
            for (style, pattern) in WORLD_LIGHTSTYLES.iter().enumerate() {
                game.host.emit(Q1Event::Lightstyle {
                    style: style as i32,
                    pattern: String::from(*pattern),
                });
            }
            Ok(())
        }
        "func_door" => spawn_door(game, &id),
        "func_button" => spawn_button(game, &id),
        "func_door_secret" => spawn_secret_door(game, &id),
        "func_plat" => spawn_plat(game, &id),
        "func_wall" => {
            let use_callback = game.named.use_callback("func_wall_use")?;
            game.update_entity(&id, |entity| {
                entity.solid = Q1Solid::Bsp;
                entity.movement = Q1MoveType::Push;
                entity.use_callback = Some(use_callback);
            })?;
            game.set_body(
                &id,
                &super::gameplay::BodyPatch {
                    angles: Some(ZERO),
                    ..Default::default()
                },
            )
        }
        "trigger_once" | "trigger_multiple" | "trigger_secret" => spawn_multi(game, &id),
        "trigger_counter" => spawn_counter(game, &id),
        "trigger_relay" => {
            let use_callback = game.named.use_callback("trigger_relay_use")?;
            game.update_entity(&id, |entity| entity.use_callback = Some(use_callback))
        }
        "trigger_teleport" => spawn_teleport_trigger(game, &id),
        "trigger_changelevel" => {
            init_trigger(game, &id)?;
            let map = game
                .entity_ref(&id)
                .map(|entity| entity.text("map"))
                .unwrap_or_default();
            if map.is_empty() {
                return Err(q1_error("changelevel trigger has no map"));
            }
            let touch = game.named.touch("changelevel_touch")?;
            game.update_entity(&id, |entity| entity.touch = Some(touch))
        }
        "trigger_hurt" => {
            init_trigger(game, &id)?;
            game.update_entity(&id, |entity| {
                if entity.damage == 0.0 {
                    entity.damage = 5.0;
                }
            })?;
            let touch = game.named.touch("hurt_touch")?;
            game.update_entity(&id, |entity| entity.touch = Some(touch))
        }
        "trigger_push" => {
            if game.uses_id1_precaches() {
                game.precache_sound("ambience/windfly.wav")?;
            }
            init_trigger(game, &id)?;
            game.update_entity(&id, |entity| {
                if entity.speed == 0.0 {
                    entity.speed = 1000.0;
                }
            })?;
            let touch = game.named.touch("push_touch")?;
            game.update_entity(&id, |entity| entity.touch = Some(touch))
        }
        "info_teleport_destination" => {
            let angles = game.body(&id).map(|body| body.angles)?;
            game.update_entity(&id, |entity| entity.mangle = angles)?;
            game.set_body(
                &id,
                &super::gameplay::BodyPatch {
                    angles: Some(ZERO),
                    ..Default::default()
                },
            )?;
            let origin = game.body(&id).map(|body| body.origin)?;
            game.set_origin(
                &id,
                vadd(
                    origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 27.0,
                    },
                ),
            )?;
            if game.entity_ref(&id).is_some_and(|entity| entity.targetname.is_empty()) {
                return Err(q1_error("teleport destination has no targetname"));
            }
            Ok(())
        }
        "testplayerstart"
        | "info_player_start"
        | "info_player_coop"
        | "info_player_deathmatch"
        | "info_player_start2"
        | "info_intermission"
        | "info_notnull" => Ok(()),
        "path_corner" => {
            if entity.targetname.is_empty() {
                return Err(q1_error("monster_movetarget has no targetname"));
            }
            game.update_entity(&id, |entity| entity.solid = Q1Solid::Trigger)?;
            game.set_bounds(
                &id,
                Bounds {
                    min: Vec3 {
                        x: -8.0,
                        y: -8.0,
                        z: -8.0,
                    },
                    max: Vec3 { x: 8.0, y: 8.0, z: 8.0 },
                },
            )?;
            let touch = game.named.touch("movetarget_touch")?;
            game.update_entity(&id, |entity| entity.touch = Some(touch))
        }
        "light" | "light_fluoro" | "light_fluorospark" => spawn_light(game, &id),
        "ambient_comp_hum" | "ambient_drone" => {
            let comp = entity.classname == "ambient_comp_hum";
            let path = if comp {
                "ambience/comp1.wav"
            } else {
                "ambience/drone6.wav"
            };
            if game.uses_id1_precaches() {
                game.precache_sound(path)?;
            }
            let origin = game.body(&id).map(|body| body.origin)?;
            game.host.emit(Q1Event::Ambient {
                origin,
                path: String::from(path),
                volume: if comp { 1.0 } else { 0.5 },
                attenuation: 3.0,
            });
            Ok(())
        }
        "misc_explobox" | "misc_explobox2" => spawn_barrel(game, &id),
        "monster_army" | "monster_dog" => spawn_monster(game, &id),
        other => Err(q1_error(format!(
            "Q1 official spawn not yet implemented: {other} at source entity {}",
            entity.source_ordinal.unwrap_or(-1)
        ))),
    }
}

fn multi_fire(game: &mut Q1EntityServices, id: &ActorId, activator: Option<ActorId>) -> Result<(), Q1Error> {
    let id = id.clone();
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let secret = entity.classname == "trigger_secret";
    let sound = match entity.sounds {
        1 => "misc/secret.wav",
        2 => "misc/talk.wav",
        3 => "misc/trigger1.wav",
        _ => "",
    };
    if entity.next_think > game.time || !game.is_live(&id) {
        return Ok(());
    }
    if secret {
        if !activator.as_ref().is_some_and(|activator| game.is_player(activator)) {
            return Ok(());
        }
        game.found_secrets += 1;
        game.host.emit(Q1Event::Secret {
            actor: id.clone(),
            total: game.total_secrets,
            found: game.found_secrets,
        });
        if game.options().edition == Q1Edition::Rerelease {
            game.host.emit(Q1Event::Achievement {
                player: activator.clone(),
                id: String::from("ACH_FIND_SECRET"),
            });
        }
    }
    game.update_entity(&id, |entity| entity.activator = activator.clone())?;
    game.set_damageable(&id, false)?;
    if !sound.is_empty() {
        game.sound_simple(&id, sound)?;
    }
    game.use_targets(&id, activator.as_ref())?;
    if !game.is_live(&id) {
        return Ok(());
    }
    let wait = game.entity_ref(&id).map(|entity| entity.wait).unwrap_or(0.0);
    if wait > 0.0 {
        return game.schedule(&id, wait, "multi_wait");
    }
    game.update_entity(&id, |entity| entity.touch = None)?;
    game.schedule(&id, 0.1, "SUB_Remove")
}

fn counter_use(game: &mut Q1EntityServices, id: &ActorId, activator: Option<&ActorId>) -> Result<(), Q1Error> {
    let id = id.clone();
    game.update_entity(&id, |entity| entity.count -= 1.0)?;
    let entity = game.entity_ref(&id).cloned().expect("counter");
    if entity.count < 0.0 {
        return Ok(());
    }
    if (entity.spawnflags & 1) == 0 {
        let text = if entity.count >= 4.0 {
            "$qc_more_go"
        } else if entity.count == 3.0 {
            "$qc_three_more"
        } else if entity.count == 2.0 {
            "$qc_two_more"
        } else if entity.count == 1.0 {
            "$qc_one_more"
        } else {
            "$qc_sequence_completed"
        };
        game.message(activator, text, true, Vec::new());
    }
    if entity.count != 0.0 {
        return Ok(());
    }
    game.use_targets(&id, activator)?;
    game.schedule(&id, 0.1, "SUB_Remove")
}

fn teleport_touch(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let other = other.clone();
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if !entity.targetname.is_empty() && entity.next_think < game.time {
        return Ok(());
    }
    let player = game.is_player(&other);
    if (entity.spawnflags & 1) != 0 && !player {
        return Ok(());
    }
    if game.health(&other) <= 0.0 || !player && !game.source_target(&other).slidebox {
        return Ok(());
    }
    let target = game
        .find(&entity.target)
        .first()
        .cloned()
        .ok_or_else(|| q1_error("could not find teleport target"))?;
    if game.host.actors.resolve_owned(&other).is_none() {
        return Ok(());
    }
    let body = match game.host.bodies.read(&other) {
        Some(body) => body,
        None => return Ok(()),
    };
    game.use_targets(&id, Some(&other))?;
    spawn_teleport_fog(game, body.origin)?;
    let destination = game.body(&target).map(|body| body.origin)?;
    let mangle = game.entity_ref(&target).map(|entity| entity.mangle).unwrap_or(ZERO);
    let forward = game.make_vectors(mangle).forward;
    spawn_teleport_fog(game, vadd(destination, vscale(forward, 32.0)))?;
    spawn_teledeath(game, destination, &other)?;
    game.set_body(
        &other,
        &super::gameplay::BodyPatch {
            origin: Some(destination),
            angles: Some(mangle),
            velocity: Some(if player { vscale(forward, 300.0) } else { body.velocity }),
            ground: Some(None),
            ..Default::default()
        },
    )?;
    game.link(&other)?;
    if player {
        if game.player_owned(&other).is_some() {
            let time = game.time;
            game.update_player(&other, |player| player.teleport_until = time + 0.7)?;
        }
        let lock_until = game.time + 0.7;
        game.host.emit(Q1Event::TeleportPlayer {
            player: other.clone(),
            angles: mangle,
            lock_until,
        });
    }
    Ok(())
}

fn changelevel_touch(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let other = other.clone();
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let map = entity.text("map");
    if !game.is_player(&other) {
        return Ok(());
    }
    if game.options().no_exit == Some(1) || game.options().no_exit == Some(2) && game.map_name != "start" {
        let params = Q1DamageParams {
            death_type: String::from("exit"),
            ..Default::default()
        };
        game.damage(&other, Some(&id), Some(&id), 50000.0, &params);
        return Ok(());
    }
    game.use_targets(&id, Some(&other))?;
    game.update_entity(&id, |entity| entity.touch = None)?;
    if (entity.spawnflags & 1) != 0 && game.options().deathmatch == 0 {
        game.travel(&map, Some(&other));
        return Ok(());
    }
    game.update_entity(&id, |entity| entity.activator = Some(other.clone()))?;
    game.schedule(&id, 0.1, "execute_changelevel")
}

struct Q1PathNext {
    name: String,
    target: Option<ActorId>,
    pause_until: f64,
}

fn next_path_target(game: &Q1EntityServices, corner: &ActorId) -> Result<Q1PathNext, Q1Error> {
    let corner_state = game
        .entity_ref(corner)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let target = game.find(&corner_state.target).first().cloned();
    Ok(match target {
        Some(target) => Q1PathNext {
            name: corner_state.target.clone(),
            target: Some(target),
            pause_until: 0.0,
        },
        None => Q1PathNext {
            name: String::new(),
            target: None,
            pause_until: path_end_time(game.time),
        },
    })
}

fn path_touch(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let other = other.clone();
    let actor = game.entity_ref(&other).cloned();
    let monster = actor.as_ref().and_then(|actor| actor.monster.clone());
    if actor.is_some() && game.source_path_touch(&id, &other)? {
        return Ok(());
    }
    let mut authored = None;
    if game.authored_path_follower.is_some() {
        let mut factory = game.authored_path_follower.take().expect("follower factory");
        authored = factory(&other);
        game.authored_path_follower = Some(factory);
    }
    if let Some(mut follower) = authored {
        let corner = game.entity_ref(&id).cloned().expect("corner");
        if follower.targetname != corner.targetname || follower.enemy.is_some() {
            return Ok(());
        }
        let next = next_path_target(game, &id)?;
        let target = next
            .target
            .as_ref()
            .and_then(|target| game.entity_ref(target).map(|entity| entity.actor.id().clone()));
        return (follower.advance)(game, &next.name, target, next.pause_until);
    }
    let monster = match (actor, monster) {
        (Some(_), Some(monster)) => monster,
        _ => return Ok(()),
    };
    let corner = game.entity_ref(&id).cloned().expect("corner");
    if monster.path != corner.targetname || monster.enemy.is_some() {
        return Ok(());
    }
    let next = next_path_target(game, &id)?;
    match next.target.as_ref() {
        None => {
            game.update_entity(&other, |entity| {
                if let Some(monster) = entity.monster.as_mut() {
                    monster.path = next.name.clone();
                    monster.pause_until = next.pause_until;
                }
            })?;
            if let Some(path_end) = game.entity_ref(&other).and_then(|entity| entity.path_end.clone()) {
                game.invoke_action(&other, &path_end)?;
            }
            Ok(())
        }
        Some(target) => {
            let origin = game.body(target).map(|body| body.origin)?;
            let self_origin = game.body(&other).map(|body| body.origin)?;
            let yaw = yaw_for(vsub(origin, self_origin));
            game.update_entity(&other, |entity| {
                entity.ideal_yaw = yaw;
                if let Some(monster) = entity.monster.as_mut() {
                    monster.path = next.name.clone();
                }
            })
        }
    }
}

/// Register spawn callbacks.
pub fn register_spawn_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "multi_use",
        super::callbacks::Q1CallbackHandlers {
            use_callback: Some(
                |game: &mut Q1EntityServices, id: &ActorId, _other: Option<&ActorId>, activator: Option<&ActorId>| {
                    multi_fire(game, id, activator.cloned())
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "multi_killed",
        super::callbacks::Q1CallbackHandlers {
            die: Some(
                |game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>| {
                    multi_fire(game, id, attacker.cloned())
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "multi_touch",
        super::callbacks::Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 _normal: Option<Vec3>,
                 _surface: Option<&super::gameplay::TouchSurface>| {
                    if !game.is_player(other) {
                        return Ok(());
                    }
                    let body = match game.host.bodies.read(other) {
                        Some(body) => body,
                        None => return Ok(()),
                    };
                    let movedir = game.entity_ref(id).map(|entity| entity.movedir).unwrap_or(ZERO);
                    if dot(game.make_vectors(body.angles).forward, movedir) < 0.0 {
                        return Ok(());
                    }
                    multi_fire(game, id, Some(other.clone()))
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "multi_wait",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let max_health = game.entity_ref(&id).map(|entity| entity.max_health).unwrap_or(0.0);
                if max_health > 0.0 {
                    let owned = game
                        .entity_ref(&id)
                        .map(|entity| entity.actor.clone())
                        .expect("trigger");
                    game.host.combat.set_health(&owned, max_health)?;
                    game.set_damageable(&id, true)?;
                    game.update_entity(&id, |entity| entity.solid = Q1Solid::Bbox)?;
                }
                Ok(())
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "counter_use",
        super::callbacks::Q1CallbackHandlers {
            use_callback: Some(
                |game: &mut Q1EntityServices, id: &ActorId, _other: Option<&ActorId>, activator: Option<&ActorId>| {
                    counter_use(game, id, activator)
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "teleport_use",
        super::callbacks::Q1CallbackHandlers {
            use_callback: Some(
                |game: &mut Q1EntityServices, id: &ActorId, _other: Option<&ActorId>, _activator: Option<&ActorId>| {
                    game.force_retouch = 2;
                    game.schedule(id, 0.2, "SUB_Null")
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "teleport_touch",
        super::callbacks::Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 _normal: Option<Vec3>,
                 _surface: Option<&super::gameplay::TouchSurface>| { teleport_touch(game, id, other) },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "play_teleport",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let index = 4.min(f64::from((game.host.random() * 5.0) as f32).floor() as i32);
                game.sound_simple(&id, &format!("misc/r_tele{}.wav", index + 1))?;
                game.remove(&id)
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "tdeath_touch",
        super::callbacks::Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 _normal: Option<Vec3>,
                 _surface: Option<&super::gameplay::TouchSurface>| {
                    let id = id.clone();
                    let other = other.clone();
                    let entity = game
                        .entity_ref(&id)
                        .cloned()
                        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                    let owner = match entity.owner.clone() {
                        Some(owner) if !qa_core::identity::same_actor(&other, &owner) => owner,
                        _ => return Ok(()),
                    };
                    if game.is_player(&other) {
                        let shielded = game
                            .player_ref(&other)
                            .and_then(|player| player.powerups.get(&Q1Powerup::Invulnerability).copied())
                            .unwrap_or(0.0)
                            > game.time;
                        if shielded {
                            game.update_entity(&id, |entity| {
                                entity.classname = String::from("teledeath2");
                            })?;
                        }
                        if !game.is_player(&owner) {
                            let classname = game
                                .entity_ref(&id)
                                .map(|entity| entity.classname.clone())
                                .unwrap_or_default();
                            let params = Q1DamageParams {
                                death_type: classname,
                                ..Default::default()
                            };
                            game.damage(&owner, Some(&id), Some(&id), 50000.0, &params);
                            return Ok(());
                        }
                    }
                    if game.health(&other) != 0.0 {
                        let classname = game
                            .entity_ref(&id)
                            .map(|entity| entity.classname.clone())
                            .unwrap_or_default();
                        let params = Q1DamageParams {
                            death_type: classname,
                            ..Default::default()
                        };
                        game.damage(&other, Some(&id), Some(&id), 50000.0, &params);
                    }
                    Ok(())
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "light_use",
        super::callbacks::Q1CallbackHandlers {
            use_callback: Some(
                |game: &mut Q1EntityServices, id: &ActorId, _other: Option<&ActorId>, _activator: Option<&ActorId>| {
                    game.update_entity(id, |entity| entity.spawnflags ^= 1)?;
                    let entity = game
                        .entity_ref(id)
                        .cloned()
                        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                    game.host.emit(Q1Event::Lightstyle {
                        style: entity.number("style") as i32,
                        pattern: String::from(if (entity.spawnflags & 1) != 0 { "a" } else { "m" }),
                    });
                    Ok(())
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "barrel_die",
        super::callbacks::Q1CallbackHandlers {
            die: Some(
                |game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>| {
                    let id = id.clone();
                    let attacker = attacker.cloned();
                    game.update_entity(&id, |entity| {
                        entity.classname = String::from("explo_box");
                        entity.activator = attacker;
                    })?;
                    game.set_damageable(&id, false)?;
                    game.schedule(&id, 0.3, "barrel_explode")
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "barrel_explode",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let activator = game.entity_ref(&id).and_then(|entity| entity.activator.clone());
                game.radius_damage(&id, activator.as_ref(), 160.0, None, None, "");
                game.sound_simple(&id, "weapons/r_exp3.wav")?;
                let origin = game.body(&id).map(|body| body.origin)?;
                game.effect_simple(
                    Q1Effect::Explosion,
                    vadd(
                        origin,
                        Vec3 {
                            x: 0.0,
                            y: 0.0,
                            z: 32.0,
                        },
                    ),
                );
                game.remove(&id)
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "func_wall_use",
        super::callbacks::Q1CallbackHandlers {
            use_callback: Some(
                |game: &mut Q1EntityServices, id: &ActorId, _other: Option<&ActorId>, _activator: Option<&ActorId>| {
                    game.update_entity(id, |entity| entity.frame = 1 - entity.frame)
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "trigger_relay_use",
        super::callbacks::Q1CallbackHandlers {
            use_callback: Some(
                |game: &mut Q1EntityServices, id: &ActorId, _other: Option<&ActorId>, activator: Option<&ActorId>| {
                    game.use_targets(id, activator)
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "changelevel_touch",
        super::callbacks::Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 _normal: Option<Vec3>,
                 _surface: Option<&super::gameplay::TouchSurface>| {
                    changelevel_touch(game, id, other)
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "execute_changelevel",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let entity = game
                    .entity_ref(id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                game.begin_intermission(&entity.text("map"), entity.activator.as_ref())
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hurt_touch",
        super::callbacks::Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 _normal: Option<Vec3>,
                 _surface: Option<&super::gameplay::TouchSurface>| {
                    let id = id.clone();
                    let other = other.clone();
                    let entity = game
                        .entity_ref(&id)
                        .cloned()
                        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                    if entity.solid != Q1Solid::Trigger
                        || !game
                            .host
                            .combat
                            .read(&other)
                            .is_some_and(|combat| combat.can_take_damage)
                    {
                        return Ok(());
                    }
                    game.update_entity(&id, |entity| entity.solid = Q1Solid::None)?;
                    let params = Q1DamageParams {
                        death_type: String::from("trigger"),
                        ..Default::default()
                    };
                    game.damage(&other, Some(&id), Some(&id), entity.damage, &params);
                    game.schedule(&id, 1.0, "hurt_on")
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hurt_on",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                game.update_entity(&id, |entity| entity.solid = Q1Solid::Trigger)?;
                game.link(&id)
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "push_touch",
        super::callbacks::Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 _normal: Option<Vec3>,
                 _surface: Option<&super::gameplay::TouchSurface>| {
                    let id = id.clone();
                    let other = other.clone();
                    let entity = game
                        .entity_ref(&id)
                        .cloned()
                        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                    if game.health(&other) <= 0.0
                        && game
                            .entity_ref(&other)
                            .is_some_and(|entity| entity.classname != "grenade")
                    {
                        return Ok(());
                    }
                    if game.host.actors.resolve_owned(&other).is_none() || game.host.bodies.read(&other).is_none() {
                        return Ok(());
                    }
                    game.set_body(
                        &id,
                        &super::gameplay::BodyPatch {
                            velocity: Some(vscale(entity.movedir, entity.speed * 10.0)),
                            ..Default::default()
                        },
                    )?;
                    if (entity.spawnflags & 1) != 0 {
                        game.remove(&id)?;
                    }
                    Ok(())
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "movetarget_touch",
        super::callbacks::Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 _normal: Option<Vec3>,
                 _surface: Option<&super::gameplay::TouchSurface>| { path_touch(game, id, other) },
            ),
            ..Default::default()
        },
    )?;
    Ok(())
}

/// Spawn a teleport fog marker (`spawn_tfog`).
pub fn spawn_teleport_fog(game: &mut Q1EntityServices, origin: Vec3) -> Result<ActorId, Q1Error> {
    let fog = game.create("teleport_fog", None, None)?;
    game.update_entity(&fog, |entity| entity.classname.clear())?;
    game.set_body(
        &fog,
        &super::gameplay::BodyPatch {
            origin: Some(origin),
            ..Default::default()
        },
    )?;
    game.schedule(&fog, 0.2, "play_teleport")?;
    game.effect_simple(Q1Effect::Teleport, origin);
    Ok(fog)
}

/// Spawn a teledeath volume (`spawn_tdeath`).
pub fn spawn_teledeath(game: &mut Q1EntityServices, origin: Vec3, owner: &ActorId) -> Result<ActorId, Q1Error> {
    let owner = owner.clone();
    let body = game
        .host
        .bodies
        .read(&owner)
        .ok_or_else(|| q1_error("Teledeath owner has no shared body"))?;
    let death = game.create("teledeath", None, None)?;
    game.update_entity(&death, |entity| {
        entity.solid = Q1Solid::Trigger;
        entity.owner = Some(owner);
    })?;
    game.set_body(
        &death,
        &super::gameplay::BodyPatch {
            origin: Some(origin),
            bounds: Some(Bounds {
                min: vsub(body.bounds.min, Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
                max: vadd(body.bounds.max, Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
            }),
            ..Default::default()
        },
    )?;
    let touch = game.named.touch("tdeath_touch")?;
    game.update_entity(&death, |entity| entity.touch = Some(touch))?;
    game.schedule(&death, 0.2, "SUB_Remove")?;
    game.link(&death)?;
    game.force_retouch = 2;
    Ok(death)
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::super::host::mock::{mock_host, MockEvents};
    use super::super::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};
    use super::*;
    use crate::bsp::Q1Entity;

    fn options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    fn game() -> (Q1EntityServices, std::rc::Rc<std::cell::RefCell<MockEvents>>) {
        let (host, events) = mock_host();
        (Q1EntityServices::new(host, options()).expect("game"), events)
    }

    fn source(classname: &str, properties: &[(&str, &str)]) -> Q1Entity {
        let mut owned = vec![(String::from("classname"), String::from(classname))];
        for (key, value) in properties {
            owned.push((String::from(*key), String::from(*value)));
        }
        Q1Entity { properties: owned }
    }

    #[test]
    fn counter_counts_down_and_fires() {
        let (mut game, _) = game();
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &super::super::entity_services::Q1AttachOptions::default())
            .expect("attach");
        let counter = game.create("trigger_counter", None, None).expect("counter");
        spawn_map_actor(&mut game, &counter).expect("spawn");
        game.invoke_use(&counter, "counter_use", None, Some(&player))
            .expect("use");
        assert_eq!(game.entity_ref(&counter).map(|entity| entity.count), Some(1.0));
        game.invoke_use(&counter, "counter_use", None, Some(&player))
            .expect("use");
        assert_eq!(game.entity_ref(&counter).map(|entity| entity.count), Some(0.0));
        assert_eq!(
            game.entity_ref(&counter).and_then(|entity| entity.think.clone()),
            Some(String::from("SUB_Remove"))
        );
    }

    #[test]
    fn worldspawn_sets_world_and_lightstyles() {
        let (mut game, events) = game();
        let world = source("worldspawn", &[("worldtype", "2")]);
        let id = game.create("worldspawn", Some(&world), Some(0)).expect("world");
        spawn_map_actor(&mut game, &id).expect("spawn");
        assert_eq!(game.world, Some(id));
        assert_eq!(game.world_type, 2);
        let styles = events
            .borrow()
            .events
            .iter()
            .filter_map(|event| match event {
                Q1Event::Lightstyle { style, pattern } => Some((*style, pattern.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(styles.len(), 12);
        assert_eq!(styles[0], (0, String::from("m")));
    }

    #[test]
    fn unknown_classname_reports_ordinal() {
        let (mut game, _) = game();
        let shambler = source("monster_shambler", &[]);
        let id = game
            .create("monster_shambler", Some(&shambler), Some(7))
            .expect("shambler");
        let error = spawn_map_actor(&mut game, &id).expect_err("unknown spawn");
        assert_eq!(
            error.to_string(),
            "Q1 official spawn not yet implemented: monster_shambler at source entity 7"
        );
    }

    #[test]
    fn teleport_destination_lifts_and_validates() {
        let (mut game, _) = game();
        let bare = source("info_teleport_destination", &[]);
        let id = game
            .create("info_teleport_destination", Some(&bare), Some(3))
            .expect("dest");
        assert!(spawn_map_actor(&mut game, &id).is_err());
        let named = source("info_teleport_destination", &[("targetname", "t1")]);
        let id = game
            .create("info_teleport_destination", Some(&named), Some(4))
            .expect("dest");
        spawn_map_actor(&mut game, &id).expect("spawn");
        assert_eq!(game.body(&id).map(|body| body.origin.z), Ok(27.0));
    }
}
