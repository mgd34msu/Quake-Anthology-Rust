//! Q1 doors, buttons, secret doors, and plats (`src/content/q1/foundation/movers.ts`).
//!
//! Original `doors.qc`/`buttons.qc`/`plats.qc` behavior: door groups,
//! key handling, button firing, secret-door sequencing, and plat
//! triggers. Door groups store member ids; the first member is the
//! master exactly like the donor's object list.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use super::entity::{move_direction, Q1MoverState};
use super::entity_services::{Q1DamageParams, Q1EntityServices};
use super::gameplay::DamageDelivery;
use super::types::{dot, overlaps, vadd, vscale, vsub, Q1MoveType, Q1Solid, ZERO};
use crate::q1::{q1_error, Q1Error};

fn schedule_fround(value: f64) -> f64 {
    f64::from(value as f32)
}

fn door_sound(sounds: i32, moving: bool) -> &'static str {
    match sounds {
        1 => {
            if moving {
                "doors/doormv1.wav"
            } else {
                "doors/drclos4.wav"
            }
        }
        2 => {
            if moving {
                "doors/hydro1.wav"
            } else {
                "doors/hydro2.wav"
            }
        }
        3 => {
            if moving {
                "doors/stndr1.wav"
            } else {
                "doors/stndr2.wav"
            }
        }
        4 => {
            if moving {
                "doors/ddoor1.wav"
            } else {
                "doors/ddoor2.wav"
            }
        }
        _ => "misc/null.wav",
    }
}

fn button_sound(sounds: i32) -> &'static str {
    if sounds >= 0 {
        [
            "buttons/airbut1.wav",
            "buttons/switch21.wav",
            "buttons/switch02.wav",
            "buttons/switch04.wav",
        ]
        .get(sounds as usize)
        .copied()
        .unwrap_or("buttons/airbut1.wav")
    } else {
        "buttons/airbut1.wav"
    }
}

fn secret_sound(sounds: i32, moving: bool) -> &'static str {
    match sounds {
        1 => {
            if moving {
                "doors/winch2.wav"
            } else {
                "doors/drclos4.wav"
            }
        }
        2 => {
            if moving {
                "doors/airdoor1.wav"
            } else {
                "doors/airdoor2.wav"
            }
        }
        _ => {
            if moving {
                "doors/basesec1.wav"
            } else {
                "doors/basesec2.wav"
            }
        }
    }
}

fn plat_sound(sounds: i32, moving: bool) -> &'static str {
    if sounds == 1 {
        if moving {
            "plats/plat1.wav"
        } else {
            "plats/plat2.wav"
        }
    } else if moving {
        "plats/medplat1.wav"
    } else {
        "plats/medplat2.wav"
    }
}

fn crush_params() -> Q1DamageParams {
    Q1DamageParams {
        weapon: None,
        delivery: DamageDelivery::default(),
        death_type: String::from("crush"),
        armor_effect: None,
    }
}

/// Send a door to the closed position (`doorDown`).
pub fn door_down(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let (sounds, max_health, pos1, speed) = {
        let entity = game.entity_ref(&id).ok_or_else(|| q1_error("Missing Q1 entity"))?;
        (entity.sounds, entity.max_health, entity.pos1, entity.speed)
    };
    game.sound_simple(&id, door_sound(sounds, true))?;
    game.update_entity(&id, |entity| entity.state = Q1MoverState::Down)?;
    if max_health > 0.0 {
        game.set_health(&id, max_health)?;
        game.set_damageable(&id, true)?;
    }
    game.calc_move(&id, pos1, speed, "door_hit_bottom")
}

/// Send a door to the open position (`doorUp`).
pub fn door_up(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if entity.state == Q1MoverState::Up {
        return Ok(());
    }
    if entity.state == Q1MoverState::Top {
        if entity.wait >= 0.0 && (entity.spawnflags & 32) == 0 {
            let due = schedule_fround(entity.number("ltime") + entity.wait);
            return game.schedule_at(&id, due, "door_go_down");
        }
        return Ok(());
    }
    game.sound_simple(&id, door_sound(entity.sounds, true))?;
    game.update_entity(&id, |entity| entity.state = Q1MoverState::Up)?;
    game.calc_move(&id, entity.pos2, entity.speed, "door_hit_top")?;
    let activator = game.entity_ref(&id).and_then(|entity| entity.activator.clone());
    game.use_targets(&id, activator.as_ref())
}

fn master_id(game: &Q1EntityServices, id: &ActorId) -> ActorId {
    game.entity_ref(id)
        .and_then(|entity| entity.door_group.first().cloned())
        .unwrap_or_else(|| id.clone())
}

fn door_group(game: &Q1EntityServices, master: &ActorId) -> Vec<ActorId> {
    match game.entity_ref(master).map(|entity| entity.door_group.clone()) {
        Some(group) if !group.is_empty() => group,
        _ => vec![master.clone()],
    }
}

fn door_use(game: &mut Q1EntityServices, id: &ActorId, activator: Option<&ActorId>) -> Result<(), Q1Error> {
    let master = master_id(game, id);
    let master_state = game
        .entity_ref(&master)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let down = (master_state.spawnflags & 32) != 0
        && (master_state.state == Q1MoverState::Up || master_state.state == Q1MoverState::Top);
    let activator = activator.cloned();
    let group = door_group(game, &master);
    for door in &group {
        let activator = activator.clone();
        game.update_entity(door, |entity| {
            entity.message.clear();
            entity.activator = activator;
        })?;
        if down {
            door_down(game, door)?;
        } else {
            door_up(game, door)?;
        }
    }
    Ok(())
}

/// Initialize a door from its map entity (`spawnDoor`).
pub fn spawn_door(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let key_sounds: &[&str] = match game.world_type {
        0 => &["doors/medtry.wav", "doors/meduse.wav"],
        1 => &["doors/runetry.wav", "doors/runeuse.wav"],
        2 => &["doors/basetry.wav", "doors/baseuse.wav"],
        _ => &[],
    };
    for path in key_sounds {
        if game.uses_id1_precaches() {
            game.precache_sound(path)?;
        }
    }
    let sounds = game.entity_ref(&id).map(|entity| entity.sounds).unwrap_or(0);
    let door_sounds: &[&str] = match sounds {
        0 => &["misc/null.wav", "misc/null.wav"],
        1 => &["doors/drclos4.wav", "doors/doormv1.wav"],
        2 => &["doors/hydro1.wav", "doors/hydro2.wav"],
        3 => &["doors/stndr1.wav", "doors/stndr2.wav"],
        4 => &["doors/ddoor1.wav", "doors/ddoor2.wav"],
        _ => &[],
    };
    for path in door_sounds {
        if game.uses_id1_precaches() {
            game.precache_sound(path)?;
        }
    }
    let body = game.body(&id)?;
    let movedir = move_direction(body.angles, None);
    game.update_entity(&id, |entity| entity.movedir = movedir)?;
    game.set_body(
        &id,
        &super::gameplay::BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    game.update_entity(&id, |entity| {
        entity.solid = Q1Solid::Bsp;
        entity.movement = Q1MoveType::Push;
        if entity.speed == 0.0 {
            entity.speed = 100.0;
        }
        if entity.wait == 0.0 {
            entity.wait = 3.0;
        }
        if entity.damage == 0.0 {
            entity.damage = 2.0;
        }
        if (entity.spawnflags & 24) != 0 {
            entity.wait = -1.0;
        }
        entity.pos1 = body.origin;
    })?;
    let entity = game.entity_ref(&id).cloned().expect("door");
    let size = vsub(body.bounds.max, body.bounds.min);
    let absolute = Vec3 {
        x: entity.movedir.x.abs(),
        y: entity.movedir.y.abs(),
        z: entity.movedir.z.abs(),
    };
    let distance = if game.options().edition == super::types::Q1Edition::Rerelease {
        f64::from(dot(absolute, size))
    } else {
        f64::from(dot(entity.movedir, size).abs())
    };
    let lip = entity.number("lip");
    let lip = if lip == 0.0 { 8.0 } else { lip };
    let pos2 = vadd(entity.pos1, vscale(entity.movedir, distance - lip));
    game.update_entity(&id, |entity| entity.pos2 = pos2)?;
    if (entity.spawnflags & 1) != 0 {
        game.update_entity(&id, |entity| {
            core::mem::swap(&mut entity.pos1, &mut entity.pos2);
        })?;
        let pos1 = game.entity_ref(&id).map(|entity| entity.pos1).expect("door");
        game.set_origin(&id, pos1)?;
    }
    let use_callback = game.named.use_callback("door_use")?;
    let blocked = game.named.blocked("door_blocked")?;
    let touch = game.named.touch("door_touch")?;
    let die = if entity.max_health > 0.0 {
        Some(game.named.die("door_killed")?)
    } else {
        None
    };
    game.update_entity(&id, |entity| {
        entity.use_callback = Some(use_callback);
        entity.blocked = Some(blocked);
        entity.touch = Some(touch);
        if entity.max_health > 0.0 {
            entity.die = die.clone();
        }
    })?;
    if entity.max_health > 0.0 {
        game.set_damageable(&id, true)?;
    }
    Ok(())
}

/// Link touching doors into groups (`linkDoors`).
pub fn link_doors(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let doors: Vec<ActorId> = game
        .entity_ids()
        .into_iter()
        .filter(|id| {
            game.entity_ref(id)
                .is_some_and(|entity| entity.classname == "func_door")
        })
        .collect();
    for (index, master) in doors.iter().enumerate() {
        let grouped = game
            .entity_ref(master)
            .is_some_and(|entity| !entity.door_group.is_empty());
        if grouped {
            continue;
        }
        let bounds = game.body(master)?.bounds;
        let mut group = vec![master.clone()];
        let mut min = bounds.min;
        let mut max = bounds.max;
        let master_flags = game.entity_ref(master).map(|entity| entity.spawnflags).unwrap_or(0);
        if (master_flags & 4) == 0 {
            let mut current = master.clone();
            for candidate in doors.iter().skip(index + 1) {
                let current_bounds = game.body(&current)?.bounds;
                let candidate_bounds = game.body(candidate)?.bounds;
                if !overlaps(&current_bounds, &candidate_bounds) {
                    continue;
                }
                if game
                    .entity_ref(candidate)
                    .is_some_and(|entity| !entity.door_group.is_empty())
                {
                    return Err(q1_error("cross connected doors"));
                }
                group.push(candidate.clone());
                current = candidate.clone();
                min = Vec3 {
                    x: min.x.min(candidate_bounds.min.x),
                    y: min.y.min(candidate_bounds.min.y),
                    z: min.z.min(candidate_bounds.min.z),
                };
                max = Vec3 {
                    x: max.x.max(candidate_bounds.max.x),
                    y: max.y.max(candidate_bounds.max.y),
                    z: max.z.max(candidate_bounds.max.z),
                };
                let candidate_state = game.entity_ref(candidate).cloned().expect("door");
                if !candidate_state.targetname.is_empty() {
                    let targetname = candidate_state.targetname.clone();
                    game.update_entity(master, |entity| entity.targetname = targetname)?;
                }
                if !candidate_state.message.is_empty() {
                    let message = candidate_state.message.clone();
                    game.update_entity(master, |entity| entity.message = message)?;
                }
                if candidate_state.max_health != 0.0 {
                    game.set_health(master, candidate_state.max_health)?;
                    game.update_entity(master, |entity| {
                        entity.max_health = candidate_state.max_health;
                    })?;
                }
            }
        }
        for door in &group {
            let group = group.clone();
            game.update_entity(door, |entity| entity.door_group = group)?;
        }
        let master_state = game.entity_ref(master).cloned().expect("door");
        if (master_state.spawnflags & (4 | 8 | 16)) != 0
            || master_state.max_health > 0.0
            || !master_state.targetname.is_empty()
        {
            continue;
        }
        let trigger = game.create("door_trigger", None, None)?;
        let trigger_bounds = qa_core::math::Bounds {
            min: vsub(
                min,
                Vec3 {
                    x: 60.0,
                    y: 60.0,
                    z: 8.0,
                },
            ),
            max: vadd(
                max,
                Vec3 {
                    x: 60.0,
                    y: 60.0,
                    z: 8.0,
                },
            ),
        };
        game.update_entity(&trigger, |entity| {
            entity.solid = Q1Solid::Trigger;
            entity.trigger_bounds = Some(trigger_bounds);
        })?;
        game.set_bounds(&trigger, trigger_bounds)?;
        let touch = game.named.touch("door_trigger_touch")?;
        let master = master.clone();
        game.update_entity(&trigger, |entity| {
            entity.owner = Some(master);
            entity.touch = Some(touch);
        })?;
    }
    Ok(())
}

/// Initialize a button from its map entity (`spawnButton`).
pub fn spawn_button(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let sounds = game.entity_ref(&id).map(|entity| entity.sounds).unwrap_or(0);
    if game.uses_id1_precaches() && (0..4).contains(&sounds) {
        let paths = [
            "buttons/airbut1.wav",
            "buttons/switch21.wav",
            "buttons/switch02.wav",
            "buttons/switch04.wav",
        ];
        game.precache_sound(paths[sounds as usize])?;
    }
    let body = game.body(&id)?;
    let movedir = move_direction(body.angles, None);
    game.update_entity(&id, |entity| {
        entity.solid = Q1Solid::Bsp;
        entity.movement = Q1MoveType::Push;
        if entity.speed == 0.0 {
            entity.speed = 40.0;
        }
        if entity.wait == 0.0 {
            entity.wait = 1.0;
        }
        entity.movedir = movedir;
        entity.pos1 = body.origin;
    })?;
    game.set_body(
        &id,
        &super::gameplay::BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    let entity = game.entity_ref(&id).cloned().expect("button");
    let lip = entity.number("lip");
    let lip = if lip == 0.0 { 4.0 } else { lip };
    let travel = f64::from(dot(entity.movedir, vsub(body.bounds.max, body.bounds.min)).abs()) - lip;
    let pos2 = vadd(body.origin, vscale(entity.movedir, travel));
    game.update_entity(&id, |entity| entity.pos2 = pos2)?;
    let use_callback = game.named.use_callback("button_use")?;
    if entity.max_health > 0.0 {
        let die = game.named.die("button_killed")?;
        game.update_entity(&id, |entity| {
            entity.use_callback = Some(use_callback);
            entity.die = Some(die);
        })?;
        game.set_damageable(&id, true)?;
    } else {
        let touch = game.named.touch("button_touch")?;
        game.update_entity(&id, |entity| {
            entity.use_callback = Some(use_callback);
            entity.touch = Some(touch);
        })?;
    }
    Ok(())
}

fn secret_shootable(targetname: &str, spawnflags: i32) -> bool {
    targetname.is_empty() || (spawnflags & 16) != 0
}

/// Initialize a secret door from its map entity (`spawnSecretDoor`).
pub fn spawn_secret_door(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let sounds = game.entity_ref(&id).map(|entity| entity.sounds).unwrap_or(0);
    let paths: &[&str] = match sounds {
        1 => &["doors/latch2.wav", "doors/winch2.wav", "doors/drclos4.wav"],
        2 => &["doors/airdoor1.wav", "doors/airdoor2.wav"],
        0 | 3 => &["doors/basesec1.wav", "doors/basesec2.wav"],
        _ => &[],
    };
    for path in paths {
        if game.uses_id1_precaches() {
            game.precache_sound(path)?;
        }
    }
    let body = game.body(&id)?;
    game.update_entity(&id, |entity| {
        entity.mangle = body.angles;
        entity.solid = Q1Solid::Bsp;
        entity.movement = Q1MoveType::Push;
        entity.speed = 50.0;
        if entity.wait == 0.0 {
            entity.wait = 5.0;
        }
        if entity.damage == 0.0 {
            entity.damage = 2.0;
        }
        entity.pos1 = body.origin;
        if entity.sounds == 0 {
            entity.sounds = 3;
        }
    })?;
    game.set_body(
        &id,
        &super::gameplay::BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    let entity = game.entity_ref(&id).cloned().expect("secret");
    game.set_damageable(&id, secret_shootable(&entity.targetname, entity.spawnflags))?;
    game.set_health(&id, 10000.0)?;
    let use_callback = game.named.use_callback("fd_secret_use")?;
    let pain = game.named.pain("fd_secret_use")?;
    let die = game.named.die("fd_secret_use")?;
    let blocked = game.named.blocked("fd_secret_blocked")?;
    let touch = game.named.touch("fd_secret_touch")?;
    game.update_entity(&id, |entity| {
        entity.use_callback = Some(use_callback);
        entity.pain = Some(pain);
        entity.die = Some(die);
        entity.blocked = Some(blocked);
        entity.touch = Some(touch);
    })?;
    let _ = entity;
    Ok(())
}

/// Initialize a plat from its map entity (`spawnPlat`).
pub fn spawn_plat(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let sounds = game.entity_ref(&id).map(|entity| entity.sounds).unwrap_or(0);
    let paths: &[&str] = match sounds {
        1 => &["plats/plat1.wav", "plats/plat2.wav"],
        0 | 2 => &["plats/medplat1.wav", "plats/medplat2.wav"],
        _ => &[],
    };
    for path in paths {
        if game.uses_id1_precaches() {
            game.precache_sound(path)?;
        }
    }
    let body = game.body(&id)?;
    let size = vsub(body.bounds.max, body.bounds.min);
    let height = {
        let entity = game.entity_ref(&id).expect("plat");
        let height = entity.number("height");
        if height == 0.0 {
            f64::from(size.z) - 8.0
        } else {
            height
        }
    };
    game.update_entity(&id, |entity| {
        entity.solid = Q1Solid::Bsp;
        entity.movement = Q1MoveType::Push;
        if entity.speed == 0.0 {
            entity.speed = 150.0;
        }
        entity.pos1 = body.origin;
        entity.pos2 = vadd(
            body.origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: -(height as f32),
            },
        );
    })?;
    game.set_body(
        &id,
        &super::gameplay::BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    let blocked = game.named.blocked("plat_crush")?;
    let use_callback = game.named.use_callback("plat_use")?;
    game.update_entity(&id, |entity| {
        entity.blocked = Some(blocked);
        entity.use_callback = Some(use_callback);
        entity.activated = entity.targetname.is_empty();
    })?;
    let activated = game.entity_ref(&id).map(|entity| entity.activated).unwrap_or(false);
    if activated {
        game.update_entity(&id, |entity| entity.state = Q1MoverState::Bottom)?;
        let pos2 = game.entity_ref(&id).map(|entity| entity.pos2).expect("plat");
        game.set_origin(&id, pos2)?;
    } else {
        game.update_entity(&id, |entity| entity.state = Q1MoverState::Up)?;
    }
    let entity = game.entity_ref(&id).cloned().expect("plat");
    let trigger = game.create("plat_trigger", None, None)?;
    let mut min = vadd(
        body.bounds.min,
        Vec3 {
            x: 25.0,
            y: 25.0,
            z: 0.0,
        },
    );
    let mut max = vsub(
        body.bounds.max,
        Vec3 {
            x: 25.0,
            y: 25.0,
            z: -8.0,
        },
    );
    min.z = max.z - ((entity.pos1.z - entity.pos2.z) as f64 + 8.0) as f32;
    if (entity.spawnflags & 1) != 0 {
        max.z = min.z + 8.0;
    }
    if size.x <= 50.0 {
        min.x = (body.bounds.min.x + body.bounds.max.x) / 2.0;
        max.x = min.x + 1.0;
    }
    if size.y <= 50.0 {
        min.y = (body.bounds.min.y + body.bounds.max.y) / 2.0;
        max.y = min.y + 1.0;
    }
    let trigger_bounds = qa_core::math::Bounds { min, max };
    game.update_entity(&trigger, |entity| {
        entity.solid = Q1Solid::Trigger;
        entity.trigger_bounds = Some(trigger_bounds);
    })?;
    game.set_bounds(&trigger, trigger_bounds)?;
    let touch = game.named.touch("plat_center_touch")?;
    game.update_entity(&trigger, |entity| {
        entity.owner = Some(id.clone());
        entity.touch = Some(touch);
    })
}

fn door_touch(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let other = other.clone();
    if !game.is_player(&other) {
        return Ok(());
    }
    let master = master_id(game, &id);
    let master_state = game
        .entity_ref(&master)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if master_state.attack_finished > game.time {
        return Ok(());
    }
    let time = game.time;
    game.update_entity(&master, |entity| entity.attack_finished = time + 2.0)?;
    game.message(Some(&other), &master_state.message, true, Vec::new());
    let entity = game.entity_ref(&id).cloned().expect("door");
    let key = if (entity.spawnflags & 8) != 0 {
        Some("q1:key/gold")
    } else if (entity.spawnflags & 16) != 0 {
        Some("q1:key/silver")
    } else {
        None
    };
    let key = match key {
        Some(key) => key,
        None => return Ok(()),
    };
    let player = match game.player_owned(&other) {
        Some(player) => player,
        None => return Ok(()),
    };
    let world_type = game.world_type;
    if !game.host.inventory.consume(&player, &key.to_string(), 1.0) {
        let kind = if key == "q1:key/gold" { "gold" } else { "silver" };
        let suffix = if world_type == 2 {
            "keycard"
        } else if world_type == 1 {
            "runekey"
        } else {
            "key"
        };
        game.message(Some(&other), &format!("$qc_need_{kind}_{suffix}"), true, Vec::new());
        let path = if world_type == 2 {
            "doors/basetry.wav"
        } else if world_type == 1 {
            "doors/runetry.wav"
        } else {
            "doors/medtry.wav"
        };
        return game.sound_simple(&id, path);
    }
    let group = door_group(game, &master);
    for door in &group {
        game.update_entity(door, |entity| entity.touch = None)?;
    }
    let path = if world_type == 2 {
        "doors/baseuse.wav"
    } else if world_type == 1 {
        "doors/runeuse.wav"
    } else {
        "doors/meduse.wav"
    };
    game.sound(&id, path, super::types::Q1SoundChannel::Item, 1.0, 1.0)?;
    door_use(game, &master, Some(&other))
}

fn button_fire(game: &mut Q1EntityServices, id: &ActorId, activator: Option<&ActorId>) -> Result<(), Q1Error> {
    let id = id.clone();
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if entity.state == Q1MoverState::Up || entity.state == Q1MoverState::Top {
        return Ok(());
    }
    let activator = activator.cloned();
    game.update_entity(&id, |entity| {
        entity.activator = activator;
        entity.state = Q1MoverState::Up;
    })?;
    game.sound_simple(&id, button_sound(entity.sounds))?;
    game.calc_move(&id, entity.pos2, entity.speed, "button_wait")
}

fn secret_fire(game: &mut Q1EntityServices, id: &ActorId, activator: Option<ActorId>) -> Result<(), Q1Error> {
    let id = id.clone();
    game.set_health(&id, 10000.0)?;
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if entity.state != Q1MoverState::Bottom || entity.move_completion.is_some() {
        return Ok(());
    }
    game.update_entity(&id, |entity| entity.message.clear())?;
    game.use_targets(&id, activator.as_ref())?;
    game.set_damageable(&id, false)?;
    game.update_entity(&id, |entity| entity.state = Q1MoverState::Up)?;
    let bounds = game.body(&id)?.bounds;
    let size = vsub(bounds.max, bounds.min);
    let basis = game.make_vectors(entity.mangle);
    let side = if (entity.spawnflags & 4) != 0 {
        basis.up
    } else {
        basis.right
    };
    let t_width = entity.number("t_width");
    let width = if t_width == 0.0 {
        f64::from(dot(side, size).abs())
    } else {
        t_width
    };
    let t_length = entity.number("t_length");
    let distance = if t_length == 0.0 {
        f64::from(dot(basis.forward, size).abs())
    } else {
        t_length
    };
    let direction = if (entity.spawnflags & 4) != 0 {
        vscale(basis.up, -width)
    } else {
        vscale(basis.right, width * f64::from(1 - (entity.spawnflags & 2)))
    };
    let dest1 = vadd(entity.pos1, direction);
    let dest2 = vadd(dest1, vscale(basis.forward, distance));
    game.update_entity(&id, |entity| {
        entity.dest1 = dest1;
        entity.dest2 = dest2;
    })?;
    game.sound_simple(
        &id,
        if entity.sounds == 1 {
            "doors/latch2.wav"
        } else {
            secret_sound(entity.sounds, false)
        },
    )?;
    game.sound_simple(&id, secret_sound(entity.sounds, true))?;
    game.calc_move(&id, dest1, entity.speed, "fd_secret_move1")
}

fn plat_down(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    game.update_entity(&id, |entity| entity.state = Q1MoverState::Down)?;
    game.sound_simple(&id, plat_sound(entity.sounds, true))?;
    game.calc_move(&id, entity.pos2, entity.speed, "plat_hit_bottom")
}

fn plat_up(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    game.update_entity(&id, |entity| entity.state = Q1MoverState::Up)?;
    game.sound_simple(&id, plat_sound(entity.sounds, true))?;
    game.calc_move(&id, entity.pos1, entity.speed, "plat_hit_top")
}

/// Register mover callbacks.
pub fn register_mover_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "SUB_CalcMoveDone",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let completion = game
                    .entity_ref(&id)
                    .and_then(|entity| entity.move_completion.clone())
                    .ok_or_else(|| q1_error("Q1 mover completion has no destination"))?;
                game.set_origin(&id, completion.destination)?;
                game.set_body(
                    &id,
                    &super::gameplay::BodyPatch {
                        velocity: Some(ZERO),
                        ..Default::default()
                    },
                )?;
                game.update_entity(&id, |entity| {
                    entity.next_think = -1.0;
                    entity.move_completion = None;
                })?;
                game.invoke_action(&id, &completion.done)
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "door_go_down",
        super::callbacks::Q1CallbackHandlers {
            action: Some(door_down),
            ..Default::default()
        },
    )?;
    game.named.register(
        "door_hit_bottom",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let sounds = game.entity_ref(&id).map(|entity| entity.sounds).unwrap_or(0);
                game.update_entity(&id, |entity| entity.state = Q1MoverState::Bottom)?;
                game.sound_simple(&id, door_sound(sounds, false))
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "door_hit_top",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                game.update_entity(&id, |entity| entity.state = Q1MoverState::Top)?;
                game.sound_simple(&id, door_sound(entity.sounds, false))?;
                if entity.wait >= 0.0 && (entity.spawnflags & 32) == 0 {
                    let due = schedule_fround(entity.number("ltime") + entity.wait);
                    game.schedule_at(&id, due, "door_go_down")?;
                }
                Ok(())
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "door_use",
        super::callbacks::Q1CallbackHandlers {
            use_callback: Some(
                |game: &mut Q1EntityServices, id: &ActorId, _other: Option<&ActorId>, activator: Option<&ActorId>| {
                    door_use(game, id, activator)
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "door_blocked",
        super::callbacks::Q1CallbackHandlers {
            blocked: Some(|game: &mut Q1EntityServices, id: &ActorId, other: &ActorId| {
                let id = id.clone();
                let other = other.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                let params = crush_params();
                game.damage(&other, Some(&id), Some(&id), entity.damage, &params);
                if entity.wait >= 0.0 {
                    if entity.state == Q1MoverState::Down {
                        door_up(game, &id)?;
                    } else {
                        door_down(game, &id)?;
                    }
                }
                Ok(())
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "door_killed",
        super::callbacks::Q1CallbackHandlers {
            die: Some(
                |game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>| {
                    let master = master_id(game, id);
                    let max_health = game.entity_ref(&master).map(|entity| entity.max_health).unwrap_or(0.0);
                    game.set_health(&master, max_health)?;
                    game.set_damageable(&master, false)?;
                    let attacker = attacker.cloned();
                    door_use(game, &master, attacker.as_ref())
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "door_touch",
        super::callbacks::Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 _normal: Option<Vec3>,
                 _surface: Option<&super::gameplay::TouchSurface>| { door_touch(game, id, other) },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "door_trigger_touch",
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
                    if game.health(&other) <= 0.0 || entity.attack_finished > game.time {
                        return Ok(());
                    }
                    let master = match entity.owner.clone() {
                        Some(owner) => owner,
                        None => return Ok(()),
                    };
                    if game.entity_ref(&master).is_none() {
                        return Ok(());
                    }
                    let time = game.time;
                    game.update_entity(&id, |entity| entity.attack_finished = time + 1.0)?;
                    door_use(game, &master, Some(&other))
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "button_use",
        super::callbacks::Q1CallbackHandlers {
            use_callback: Some(
                |game: &mut Q1EntityServices, id: &ActorId, _other: Option<&ActorId>, activator: Option<&ActorId>| {
                    button_fire(game, id, activator)
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "button_touch",
        super::callbacks::Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 _normal: Option<Vec3>,
                 _surface: Option<&super::gameplay::TouchSurface>| {
                    if game.is_player(other) {
                        button_fire(game, id, Some(other))
                    } else {
                        Ok(())
                    }
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "button_killed",
        super::callbacks::Q1CallbackHandlers {
            die: Some(
                |game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>| {
                    let id = id.clone();
                    let attacker = attacker.cloned();
                    let max_health = game.entity_ref(&id).map(|entity| entity.max_health).unwrap_or(0.0);
                    game.set_health(&id, max_health)?;
                    game.set_damageable(&id, false)?;
                    button_fire(game, &id, attacker.as_ref())
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "button_wait",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                game.update_entity(&id, |entity| entity.state = Q1MoverState::Top)?;
                if entity.wait >= 0.0 {
                    let due = schedule_fround(entity.number("ltime") + entity.wait);
                    game.schedule_at(&id, due, "button_return")?;
                }
                let activator = game.entity_ref(&id).and_then(|entity| entity.activator.clone());
                game.use_targets(&id, activator.as_ref())?;
                game.update_entity(&id, |entity| entity.frame = 1)
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "button_return",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                game.update_entity(&id, |entity| {
                    entity.state = Q1MoverState::Down;
                    entity.frame = 0;
                })?;
                if entity.max_health > 0.0 {
                    game.set_damageable(&id, true)?;
                }
                game.calc_move(&id, entity.pos1, entity.speed, "button_done")
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "button_done",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                game.update_entity(id, |entity| entity.state = Q1MoverState::Bottom)
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "fd_secret_use",
        super::callbacks::Q1CallbackHandlers {
            use_callback: Some(
                |game: &mut Q1EntityServices, id: &ActorId, _other: Option<&ActorId>, activator: Option<&ActorId>| {
                    secret_fire(game, id, activator.cloned())
                },
            ),
            pain: Some(
                |game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>, _damage: f64| {
                    secret_fire(game, id, attacker.cloned())
                },
            ),
            die: Some(
                |game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>| {
                    secret_fire(game, id, attacker.cloned())
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "fd_secret_move1",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                game.sound_simple(&id, secret_sound(entity.sounds, false))?;
                let due = schedule_fround(entity.number("ltime") + 1.0);
                game.schedule_at(&id, due, "fd_secret_move2")
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "fd_secret_move2",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                game.sound_simple(&id, secret_sound(entity.sounds, true))?;
                game.calc_move(&id, entity.dest2, entity.speed, "fd_secret_move3")
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "fd_secret_move3",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                game.update_entity(&id, |entity| entity.state = Q1MoverState::Top)?;
                game.sound_simple(&id, secret_sound(entity.sounds, false))?;
                if (entity.spawnflags & 1) == 0 {
                    let due = schedule_fround(entity.number("ltime") + entity.wait);
                    game.schedule_at(&id, due, "fd_secret_move4")?;
                }
                Ok(())
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "fd_secret_move4",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                game.sound_simple(&id, secret_sound(entity.sounds, true))?;
                game.update_entity(&id, |entity| entity.state = Q1MoverState::Down)?;
                game.calc_move(&id, entity.dest1, entity.speed, "fd_secret_move5")
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "fd_secret_move5",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                game.sound_simple(&id, secret_sound(entity.sounds, false))?;
                let due = schedule_fround(entity.number("ltime") + 1.0);
                game.schedule_at(&id, due, "fd_secret_move6")
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "fd_secret_move6",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                game.sound_simple(&id, secret_sound(entity.sounds, true))?;
                game.calc_move(&id, entity.pos1, entity.speed, "fd_secret_done")
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "fd_secret_done",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                game.update_entity(&id, |entity| entity.state = Q1MoverState::Bottom)?;
                game.set_damageable(&id, secret_shootable(&entity.targetname, entity.spawnflags))?;
                game.set_health(&id, 10000.0)?;
                game.sound_simple(&id, secret_sound(entity.sounds, false))
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "fd_secret_blocked",
        super::callbacks::Q1CallbackHandlers {
            blocked: Some(|game: &mut Q1EntityServices, id: &ActorId, other: &ActorId| {
                let id = id.clone();
                let other = other.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                if entity.attack_finished > game.time {
                    return Ok(());
                }
                let time = game.time;
                game.update_entity(&id, |entity| entity.attack_finished = time + 0.5)?;
                let params = crush_params();
                game.damage(&other, Some(&id), Some(&id), entity.damage, &params);
                Ok(())
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "fd_secret_touch",
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
                    if !game.is_player(&other) || entity.attack_finished > game.time {
                        return Ok(());
                    }
                    let time = game.time;
                    game.update_entity(&id, |entity| entity.attack_finished = time + 2.0)?;
                    game.message(Some(&other), &entity.message, true, Vec::new());
                    Ok(())
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "plat_go_down",
        super::callbacks::Q1CallbackHandlers {
            action: Some(plat_down),
            ..Default::default()
        },
    )?;
    game.named.register(
        "plat_hit_bottom",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let sounds = game.entity_ref(&id).map(|entity| entity.sounds).unwrap_or(0);
                game.update_entity(&id, |entity| entity.state = Q1MoverState::Bottom)?;
                game.sound_simple(&id, plat_sound(sounds, false))
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "plat_hit_top",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                let id = id.clone();
                let entity = game
                    .entity_ref(&id)
                    .cloned()
                    .ok_or_else(|| q1_error("Missing Q1 entity"))?;
                game.update_entity(&id, |entity| entity.state = Q1MoverState::Top)?;
                game.sound_simple(&id, plat_sound(entity.sounds, false))?;
                let due = schedule_fround(entity.number("ltime") + 3.0);
                game.schedule_at(&id, due, "plat_go_down")
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "plat_crush",
        super::callbacks::Q1CallbackHandlers {
            blocked: Some(|game: &mut Q1EntityServices, id: &ActorId, other: &ActorId| {
                let id = id.clone();
                let other = other.clone();
                let params = crush_params();
                game.damage(&other, Some(&id), Some(&id), 1.0, &params);
                let state = game
                    .entity_ref(&id)
                    .map(|entity| entity.state)
                    .unwrap_or(Q1MoverState::Bottom);
                if state == Q1MoverState::Up {
                    plat_down(game, &id)
                } else {
                    plat_up(game, &id)
                }
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "plat_use",
        super::callbacks::Q1CallbackHandlers {
            use_callback: Some(
                |game: &mut Q1EntityServices, id: &ActorId, _other: Option<&ActorId>, _activator: Option<&ActorId>| {
                    let id = id.clone();
                    if game.entity_ref(&id).is_some_and(|entity| entity.activated) {
                        return Ok(());
                    }
                    game.update_entity(&id, |entity| entity.activated = true)?;
                    plat_down(game, &id)
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "plat_center_touch",
        super::callbacks::Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 _normal: Option<Vec3>,
                 _surface: Option<&super::gameplay::TouchSurface>| {
                    let other = other.clone();
                    if !game.is_player(&other) || game.health(&other) <= 0.0 {
                        return Ok(());
                    }
                    let owner = game.entity_ref(id).and_then(|entity| entity.owner.clone());
                    let owner = match owner {
                        Some(owner) => owner,
                        None => return Ok(()),
                    };
                    let entity = match game.entity_ref(&owner).cloned() {
                        Some(entity) => entity,
                        None => return Ok(()),
                    };
                    if entity.state == Q1MoverState::Bottom {
                        return plat_up(game, &owner);
                    }
                    if entity.state == Q1MoverState::Top {
                        let due = schedule_fround(entity.number("ltime") + 1.0);
                        return game.schedule_at(&owner, due, "plat_go_down");
                    }
                    Ok(())
                },
            ),
            ..Default::default()
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::super::host::mock::{mock_host, MockEvents};
    use super::super::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};
    use super::*;

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

    #[test]
    fn spawn_door_sets_defaults_and_callbacks() {
        let (mut game, _) = game();
        let door = game.create("func_door", None, None).expect("door");
        spawn_door(&mut game, &door).expect("spawn");
        let entity = game.entity_ref(&door).cloned().expect("door");
        assert_eq!(entity.speed, 100.0);
        assert_eq!(entity.wait, 3.0);
        assert_eq!(entity.damage, 2.0);
        assert_eq!(entity.solid, Q1Solid::Bsp);
        assert_eq!(entity.movement, Q1MoveType::Push);
        assert_eq!(entity.use_callback.as_deref(), Some("door_use"));
        assert_eq!(entity.blocked.as_deref(), Some("door_blocked"));
        assert_eq!(entity.touch.as_deref(), Some("door_touch"));
    }

    #[test]
    fn link_doors_groups_and_adds_trigger() {
        let (mut game, _) = game();
        let first = game.create("func_door", None, None).expect("first");
        let second = game.create("func_door", None, None).expect("second");
        spawn_door(&mut game, &first).expect("spawn");
        spawn_door(&mut game, &second).expect("spawn");
        link_doors(&mut game).expect("link");
        assert_eq!(game.entity_ref(&first).map(|entity| entity.door_group.len()), Some(2));
        assert_eq!(game.entity_ref(&second).map(|entity| entity.door_group.len()), Some(2));
        let triggers = game
            .entity_ids()
            .iter()
            .filter(|id| {
                game.entity_ref(id)
                    .is_some_and(|entity| entity.classname == "door_trigger")
            })
            .count();
        assert_eq!(triggers, 1);
    }
}
