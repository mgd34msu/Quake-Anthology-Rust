//! Hipnotic rotating entities, doors, trains, and clocks
//! (`src/content/q1/missionpacks/world/hipnotic-rotate.ts`).
//!
//! hiprot.qc / hipclock.qc entity behavior.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity::move_direction;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{length, vadd, vscale, vsub, Q1MoveType, Q1Solid, ZERO};
use crate::q1::{q1_error, Q1Error};

use super::common::{later, number, target_event, vector};
use super::rotate_targets::{
    damage_on_targets, link_rotate_targets, normalize_angles, rotate_targets, rotate_targets_final, set_target_origin,
};

/// Spin a continuous rotator (`continuousThink`).
fn continuous_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (ltime, state, count, spin) = game
        .entity(id)
        .map(|entity| {
            (
                entity.number("ltime"),
                entity.number("rotate_state"),
                entity.count,
                entity.number("cnt"),
            )
        })
        .unwrap_or((0.0, 0.0, 0.0, 0.0));
    let mut elapsed = game.time - ltime;
    let time = game.time;
    game.update_entity(id, |entity| number(entity, "ltime", time))?;
    if state == 2.0 {
        let count = 1.0f64.min(count + spin * elapsed);
        game.update_entity(id, |entity| entity.count = count)?;
        elapsed *= count;
    } else if state == 3.0 {
        let count = count - spin * elapsed;
        game.update_entity(id, |entity| entity.count = count)?;
        if count < 0.0 {
            rotate_targets_final(game, id)?;
            game.update_entity(id, |entity| number(entity, "rotate_state", 1.0))?;
            game.cancel(id);
            return Ok(());
        }
        elapsed *= count;
    }
    let body = game.body(id)?;
    let rotate = game.entity(id).map(|entity| entity.vector("rotate")).unwrap_or(ZERO);
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(normalize_angles(vadd(body.angles, vscale(rotate, elapsed)))),
            ..Default::default()
        },
    )?;
    rotate_targets(game, id)?;
    later(game, id, 0.02, "hip:rotate_entity")
}

/// Toggle a continuous rotator (`continuousUse`).
fn continuous_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let (state, spawnflags, speed) = game
        .entity(id)
        .map(|entity| (entity.number("rotate_state"), entity.spawnflags, entity.speed))
        .unwrap_or((0.0, 0, 0.0));
    game.update_entity(id, |entity| entity.frame = 1 - entity.frame)?;
    if state == 0.0 {
        if spawnflags & 1 != 0 {
            if speed != 0.0 {
                game.update_entity(id, |entity| {
                    entity.count = 1.0;
                    number(entity, "rotate_state", 3.0);
                })?;
            } else {
                game.update_entity(id, |entity| number(entity, "rotate_state", 1.0))?;
                game.cancel(id);
            }
        }
    } else if state == 1.0 {
        let time = game.time;
        game.update_entity(id, |entity| {
            number(entity, "ltime", time);
            entity.count = 0.0;
            number(entity, "rotate_state", if speed != 0.0 { 2.0 } else { 0.0 });
        })?;
        later(game, id, 0.02, "hip:rotate_entity")?;
    } else if state == 2.0 {
        if spawnflags & 1 != 0 {
            game.update_entity(id, |entity| number(entity, "rotate_state", 3.0))?;
        }
    } else {
        game.update_entity(id, |entity| number(entity, "rotate_state", 2.0))?;
    }
    Ok(())
}

/// Reverse one rotating door (`reverseDoor`).
fn reverse_door(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (state, dest1, dest2, speed, endtime, noise2) = game
        .entity(id)
        .map(|entity| {
            (
                entity.number("rotate_state"),
                entity.dest1,
                entity.dest2,
                entity.speed,
                entity.number("endtime"),
                entity.text("noise2"),
            )
        })
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    game.update_entity(id, |entity| entity.frame = 1 - entity.frame)?;
    let closing = state == 7.0;
    let (start, destination) = if closing { (dest1, dest2) } else { (dest2, dest1) };
    let time = game.time;
    game.update_entity(id, |entity| {
        vector(entity, "dest", destination);
        number(entity, "rotate_state", if closing { 6.0 } else { 7.0 });
        vector(entity, "rotate", vscale(vsub(destination, start), 1.0 / speed));
        number(entity, "endtime", time + speed - (endtime - time));
        number(entity, "ltime", time);
    })?;
    game.sound_simple(id, &noise2)?;
    later(game, id, 0.02, "hip:rotate_door")
}

/// Reverse a door group (`reverseGroup`).
fn reverse_group(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let group = game.entity(id).map(|entity| entity.text("group")).unwrap_or_default();
    if group.is_empty() {
        return reverse_door(game, id);
    }
    let members: Vec<ActorId> = game
        .entity_ids()
        .into_iter()
        .filter(|member| {
            game.entity(member).is_some_and(|entity| entity.text("group") == group) && game.is_live(member)
        })
        .collect();
    for member in members {
        reverse_door(game, &member)?;
    }
    Ok(())
}

/// Stop a rotate train at a path corner (`trainStop`).
fn train_stop(game: &mut Q1EntityServices, id: &ActorId, wait: bool) -> Result<(), Q1Error> {
    let goal = game
        .entity(id)
        .and_then(|entity| entity.references.get("goalentity").cloned().flatten())
        .and_then(|goal| game.entity(&goal).map(|_| goal))
        .ok_or_else(|| q1_error("rotate_train: missing goal"))?;
    let (noise, spawnflags, goal_wait) = game
        .entity(&goal)
        .map(|goal| (goal.text("noise"), goal.spawnflags, goal.wait))
        .unwrap_or_default();
    game.update_entity(id, |entity| {
        number(entity, "rotate_state", if wait { 0.0 } else { 2.0 })
    })?;
    if noise.is_empty() {
        let fallback = game.entity(id).map(|entity| entity.text("noise")).unwrap_or_default();
        game.sound_simple(id, &fallback)?;
    } else {
        game.sound_simple(id, &noise)?;
    }
    if spawnflags & 2 != 0 {
        let final_angle = game
            .entity(id)
            .map(|entity| entity.vector("finalangle"))
            .unwrap_or(ZERO);
        game.update_entity(id, |entity| vector(entity, "rotate", ZERO))?;
        game.set_body(
            id,
            &BodyPatch {
                angles: Some(final_angle),
                ..Default::default()
            },
        )?;
    }
    if spawnflags & 8 != 0 {
        game.update_entity(id, |entity| vector(entity, "rotate", ZERO))?;
    }
    if wait {
        let ltime = game.entity(id).map(|entity| entity.number("ltime")).unwrap_or(0.0);
        game.update_entity(id, |entity| number(entity, "endtime", ltime + goal_wait))?;
    } else {
        game.update_entity(id, |entity| entity.damage = 0.0)?;
    }
    game.update_entity(id, |entity| {
        entity
            .fields
            .insert("think1".to_string(), "hip:rotate_train_next".to_string());
    })
}

/// Advance a rotate train to its next path corner (`trainNext`).
fn train_next(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| number(entity, "rotate_state", 4.0))?;
    let path = game.entity(id).map(|entity| entity.text("path")).unwrap_or_default();
    let current = game
        .entity(id)
        .and_then(|entity| entity.references.get("goalentity").cloned().flatten())
        .filter(|current| game.entity(current).is_some());
    let target = game.find(&path).first().cloned();
    let (Some(current), Some(target)) = (current, target) else {
        return Err(q1_error("rotate_train_next: next target is not path_rotate"));
    };
    if game
        .entity(&target)
        .map(|target| target.classname.clone())
        .unwrap_or_default()
        != "path_rotate"
    {
        return Err(q1_error("rotate_train_next: next target is not path_rotate"));
    }
    let noise1 = game
        .entity(&current)
        .map(|current| current.text("noise1"))
        .unwrap_or_default();
    if !noise1.is_empty() {
        game.update_entity(id, |entity| {
            entity.fields.insert("noise1".to_string(), noise1);
        })?;
    }
    let start_sound = game.entity(id).map(|entity| entity.text("noise1")).unwrap_or_default();
    game.sound_simple(id, &start_sound)?;
    let next_path = game
        .entity(&target)
        .map(|target| target.target.clone())
        .unwrap_or_default();
    game.update_entity(id, |entity| {
        entity.references.insert("goalentity".to_string(), Some(target.clone()));
        entity.fields.insert("path".to_string(), next_path);
    })?;
    if game
        .entity(id)
        .map(|entity| entity.text("path"))
        .unwrap_or_default()
        .is_empty()
    {
        return Err(q1_error("rotate_train_next: no next target"));
    }
    let target_flags = game.entity(&target).map(|target| target.spawnflags).unwrap_or(0);
    let target_wait = game.entity(&target).map(|target| target.wait).unwrap_or(0.0);
    let think1 = if target_flags & 4 != 0 {
        "hip:rotate_train_stop"
    } else if target_wait != 0.0 {
        "hip:rotate_train_wait"
    } else {
        "hip:rotate_train_next"
    };
    game.update_entity(id, |entity| {
        entity.fields.insert("think1".to_string(), think1.to_string());
    })?;
    let (event, message) = game
        .entity(&current)
        .map(|current| (current.text("event"), current.message.clone()))
        .unwrap_or_default();
    if !event.is_empty() {
        target_event(game, id, &event, &message)?;
    }
    let current_flags = game.entity(&current).map(|current| current.spawnflags).unwrap_or(0);
    if current_flags & 2 != 0 {
        let final_angle = game
            .entity(id)
            .map(|entity| entity.vector("finalangle"))
            .unwrap_or(ZERO);
        game.update_entity(id, |entity| vector(entity, "rotate", ZERO))?;
        game.set_body(
            id,
            &BodyPatch {
                angles: Some(final_angle),
                ..Default::default()
            },
        )?;
    }
    if current_flags & 1 != 0 {
        let rotate = game
            .entity(&current)
            .map(|current| current.vector("rotate"))
            .unwrap_or(ZERO);
        game.update_entity(id, |entity| vector(entity, "rotate", rotate))?;
    }
    if current_flags & 16 != 0 {
        let damage = game.entity(&current).map(|current| current.damage).unwrap_or(0.0);
        game.update_entity(id, |entity| entity.damage = damage)?;
    }
    if current_flags & 64 != 0 {
        let damage = game.entity(&current).map(|current| current.damage).unwrap_or(0.0);
        damage_on_targets(game, id, damage)?;
    }
    let target_body = game.body(&target)?;
    let body = game.body(id)?;
    let current_speed = game.entity(&current).map(|current| current.speed).unwrap_or(0.0);
    let ltime = game.entity(id).map(|entity| entity.number("ltime")).unwrap_or(0.0);
    let time = game.time;
    if current_speed == -1.0 {
        game.set_origin(id, target_body.origin)?;
        game.update_entity(id, |entity| number(entity, "endtime", ltime + 0.01))?;
        set_target_origin(game, id)?;
        if target_flags & 2 != 0 {
            game.set_body(
                id,
                &BodyPatch {
                    angles: Some(target_body.angles),
                    ..Default::default()
                },
            )?;
        }
        game.update_entity(id, |entity| {
            number(entity, "duration", 1.0);
            number(entity, "cnt", time);
            entity.dest2 = ZERO;
            entity.dest1 = target_body.origin;
            vector(entity, "finaldest", target_body.origin);
        })?;
        return Ok(());
    }
    game.update_entity(id, |entity| {
        number(entity, "rotate_state", 1.0);
        vector(entity, "finaldest", target_body.origin);
    })?;
    let delta = vsub(target_body.origin, body.origin);
    let distance = f64::from(length(delta));
    if distance == 0.0 {
        game.set_body(
            id,
            &BodyPatch {
                velocity: Some(ZERO),
                ..Default::default()
            },
        )?;
        game.update_entity(id, |entity| {
            number(entity, "endtime", ltime + 0.1);
            number(entity, "duration", 1.0);
            number(entity, "cnt", time);
            entity.dest2 = ZERO;
            entity.dest1 = body.origin;
        })?;
        return Ok(());
    }
    if current_flags & 32 == 0 && current_speed > 0.0 {
        game.update_entity(id, |entity| entity.speed = current_speed)?;
    }
    let speed = game.entity(id).map(|entity| entity.speed).unwrap_or(0.0);
    if current_flags & 32 == 0 && speed == 0.0 {
        return Err(q1_error("rotate_train: no speed defined"));
    }
    let travel = if current_flags & 32 != 0 {
        current_speed
    } else {
        distance / speed
    };
    if travel < 0.1 {
        let angles = if target_flags & 2 != 0 {
            Some(target_body.angles)
        } else {
            None
        };
        game.set_body(
            id,
            &BodyPatch {
                velocity: Some(ZERO),
                angles,
                ..Default::default()
            },
        )?;
        return game.update_entity(id, |entity| number(entity, "endtime", ltime + 0.1));
    }
    let inverse = 1.0 / travel;
    if target_flags & 2 != 0 {
        let angles = game.body(id)?.angles;
        game.update_entity(id, |entity| {
            vector(entity, "finalangle", normalize_angles(target_body.angles));
            vector(entity, "rotate", vscale(vsub(target_body.angles, angles), inverse));
        })?;
    }
    game.update_entity(id, |entity| number(entity, "endtime", ltime + travel))?;
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(vscale(delta, inverse)),
            ..Default::default()
        },
    )?;
    game.update_entity(id, |entity| {
        number(entity, "duration", inverse);
        number(entity, "cnt", time);
        entity.dest2 = delta;
        entity.dest1 = body.origin;
    })
}

/// Run a rotating door toward its destination.
fn rotate_door_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (ltime, endtime) = game
        .entity(id)
        .map(|entity| (entity.number("ltime"), entity.number("endtime")))
        .unwrap_or((0.0, 0.0));
    let elapsed = game.time - ltime;
    let time = game.time;
    game.update_entity(id, |entity| number(entity, "ltime", time))?;
    let moving = time < endtime;
    let body = game.body(id)?;
    let (rotate, dest) = game
        .entity(id)
        .map(|entity| (entity.vector("rotate"), entity.vector("dest")))
        .unwrap_or((ZERO, ZERO));
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(if moving {
                vadd(body.angles, vscale(rotate, elapsed))
            } else {
                dest
            }),
            ..Default::default()
        },
    )?;
    rotate_targets(game, id)?;
    later(
        game,
        id,
        0.01,
        if moving {
            "hip:rotate_door"
        } else {
            "hip:rotate_door_done"
        },
    )
}

/// Start a rotating door moving.
fn rotate_door_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let (state, cnt, dest1, dest2, speed, noise2) = game
        .entity(id)
        .map(|entity| {
            (
                entity.number("rotate_state"),
                entity.number("cnt"),
                entity.dest1,
                entity.dest2,
                entity.speed,
                entity.text("noise2"),
            )
        })
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if state != 4.0 && state != 5.0 {
        return Ok(());
    }
    if cnt == 0.0 {
        game.update_entity(id, |entity| number(entity, "cnt", 1.0))?;
        link_rotate_targets(game, id)?;
    }
    game.update_entity(id, |entity| entity.frame = 1 - entity.frame)?;
    let (destination, start) = if state == 4.0 { (dest2, dest1) } else { (dest1, dest2) };
    let time = game.time;
    game.update_entity(id, |entity| {
        vector(entity, "dest", destination);
        number(entity, "rotate_state", if state == 4.0 { 6.0 } else { 7.0 });
        vector(entity, "rotate", vscale(vsub(destination, start), 1.0 / speed));
        number(entity, "endtime", time + speed);
        number(entity, "ltime", time);
    })?;
    game.sound_simple(id, &noise2)?;
    later(game, id, 0.01, "hip:rotate_door")
}

/// Finish a rotating door move.
fn rotate_door_done(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (state, spawnflags, dest, noise3) = game
        .entity(id)
        .map(|entity| {
            (
                entity.number("rotate_state"),
                entity.spawnflags,
                entity.vector("dest"),
                entity.text("noise3"),
            )
        })
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let time = game.time;
    game.update_entity(id, |entity| {
        number(entity, "ltime", time);
        entity.frame = 1 - entity.frame;
    })?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(dest),
            ..Default::default()
        },
    )?;
    if state == 6.0 {
        game.update_entity(id, |entity| number(entity, "rotate_state", 5.0))?;
    } else if spawnflags & 1 != 0 {
        return reverse_group(game, id);
    } else {
        game.update_entity(id, |entity| number(entity, "rotate_state", 4.0))?;
    }
    game.sound_simple(id, &noise3)?;
    rotate_targets_final(game, id)
}

/// Damage whoever a moving wall touches.
fn movewall_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let owner = game.entity(id).and_then(|entity| entity.owner.clone());
    let Some(owner) = owner else {
        return Ok(());
    };
    let attack_finished = game.entity(&owner).map(|owner| owner.attack_finished).unwrap_or(0.0);
    if game.time < attack_finished {
        return Ok(());
    }
    let (damage, owner_damage) = (
        game.entity(id).map(|entity| entity.damage).unwrap_or(0.0),
        game.entity(&owner).map(|owner| owner.damage).unwrap_or(0.0),
    );
    let damage = if damage == 0.0 { owner_damage } else { damage };
    if damage != 0.0 {
        let id_copy = id.clone();
        let other = other.clone();
        game.damage(
            &other,
            Some(&id_copy),
            Some(&owner),
            damage,
            &crate::q1::foundation::entity_services::Q1DamageParams::default(),
        );
        let time = game.time;
        game.update_entity(&owner, |owner| owner.attack_finished = time + 0.5)?;
    }
    Ok(())
}

/// Damage whoever blocks a moving wall, reversing doors.
fn movewall_blocked(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId) -> Result<(), Q1Error> {
    let owner = game.entity(id).and_then(|entity| entity.owner.clone());
    let Some(owner) = owner else {
        return Ok(());
    };
    let attack_finished = game.entity(&owner).map(|owner| owner.attack_finished).unwrap_or(0.0);
    if game.time < attack_finished {
        return Ok(());
    }
    let time = game.time;
    game.update_entity(&owner, |owner| owner.attack_finished = time + 0.5)?;
    if game
        .entity(&owner)
        .map(|owner| owner.classname.clone())
        .unwrap_or_default()
        == "func_rotate_door"
    {
        reverse_group(game, &owner)?;
    }
    let (damage, owner_damage) = (
        game.entity(id).map(|entity| entity.damage).unwrap_or(0.0),
        game.entity(&owner).map(|owner| owner.damage).unwrap_or(0.0),
    );
    let damage = if damage == 0.0 { owner_damage } else { damage };
    if damage != 0.0 {
        let id_copy = id.clone();
        let other = other.clone();
        game.damage(
            &other,
            Some(&id_copy),
            Some(&owner),
            damage,
            &crate::q1::foundation::entity_services::Q1DamageParams::default(),
        );
    }
    Ok(())
}

/// Keep a moving wall thinking.
fn movewall_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let time = game.time;
    game.update_entity(id, |entity| number(entity, "ltime", time))?;
    later(game, id, 0.02, "hip:movewall")
}

/// Find a rotate train's first corner (`hip:rotate_train_find`).
fn rotate_train_find(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| number(entity, "rotate_state", 3.0))?;
    link_rotate_targets(game, id)?;
    let path = game.entity(id).map(|entity| entity.text("path")).unwrap_or_default();
    let target = game.find(&path).first().cloned();
    let Some(target) = target else {
        return Err(q1_error("rotate_train_find: next target is not path_rotate"));
    };
    if game
        .entity(&target)
        .map(|target| target.classname.clone())
        .unwrap_or_default()
        != "path_rotate"
    {
        return Err(q1_error("rotate_train_find: next target is not path_rotate"));
    }
    let target_id = target.clone();
    game.update_entity(id, |entity| {
        entity.references.insert("goalentity".to_string(), Some(target_id));
    })?;
    let target_body = game.body(&target)?;
    let target_flags = game.entity(&target).map(|target| target.spawnflags).unwrap_or(0);
    if target_flags & 2 != 0 {
        game.set_body(
            id,
            &BodyPatch {
                angles: Some(target_body.angles),
                ..Default::default()
            },
        )?;
        game.update_entity(id, |entity| {
            vector(entity, "finalangle", normalize_angles(target_body.angles))
        })?;
    }
    let next_path = game
        .entity(&target)
        .map(|target| target.target.clone())
        .unwrap_or_default();
    game.update_entity(id, |entity| {
        entity.fields.insert("path".to_string(), next_path);
    })?;
    game.set_origin(id, target_body.origin)?;
    set_target_origin(game, id)?;
    rotate_targets_final(game, id)?;
    let (targetname, ltime) = game
        .entity(id)
        .map(|entity| (entity.targetname.clone(), entity.number("ltime")))
        .unwrap_or_default();
    let time = game.time;
    let origin = game.body(id)?.origin;
    game.update_entity(id, |entity| {
        entity
            .fields
            .insert("think1".to_string(), "hip:rotate_train_next".to_string());
        number(entity, "endtime", if targetname.is_empty() { ltime + 0.1 } else { 0.0 });
        number(entity, "duration", 1.0);
        number(entity, "cnt", time);
        entity.dest2 = ZERO;
        entity.dest1 = origin;
    })
}

/// Step a rotate train along its path.
fn rotate_train_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (ltime, endtime, state) = game
        .entity(id)
        .map(|entity| {
            (
                entity.number("ltime"),
                entity.number("endtime"),
                entity.number("rotate_state"),
            )
        })
        .unwrap_or((0.0, 0.0, 0.0));
    let elapsed = game.time - ltime;
    let time = game.time;
    game.update_entity(id, |entity| number(entity, "ltime", time))?;
    if endtime != 0.0 && time >= endtime {
        game.update_entity(id, |entity| number(entity, "endtime", 0.0))?;
        if state == 1.0 {
            let final_dest = game.entity(id).map(|entity| entity.vector("finaldest")).unwrap_or(ZERO);
            game.set_body(
                id,
                &BodyPatch {
                    origin: Some(final_dest),
                    velocity: Some(ZERO),
                    ..Default::default()
                },
            )?;
        }
        let think1 = game.entity(id).map(|entity| entity.text("think1")).unwrap_or_default();
        if !think1.is_empty() {
            game.invoke_action(id, &think1)?;
        }
    } else {
        let (dest1, dest2, cnt, duration) = game
            .entity(id)
            .map(|entity| {
                (
                    entity.dest1,
                    entity.dest2,
                    entity.number("cnt"),
                    entity.number("duration"),
                )
            })
            .unwrap_or((ZERO, ZERO, 0.0, 0.0));
        game.set_origin(id, vadd(dest1, vscale(dest2, 1.0f64.min((time - cnt) * duration))))?;
    }
    let body = game.body(id)?;
    let rotate = game.entity(id).map(|entity| entity.vector("rotate")).unwrap_or(ZERO);
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(normalize_angles(vadd(body.angles, vscale(rotate, elapsed)))),
            ..Default::default()
        },
    )?;
    rotate_targets(game, id)?;
    later(game, id, 0.02, "hip:rotate_train")
}

/// Poke a stopped rotate train into its pending leg.
fn rotate_train_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let velocity = game.body(id)?.velocity;
    let think1 = game.entity(id).map(|entity| entity.text("think1")).unwrap_or_default();
    if think1 == "hip:rotate_train_find" || velocity.x != 0.0 || velocity.y != 0.0 || velocity.z != 0.0 {
        return Ok(());
    }
    if think1.is_empty() {
        return Ok(());
    }
    game.invoke_action(id, &think1)
}

/// Stop a rotate train at a corner action.
fn rotate_train_wait(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    train_stop(game, id, true)
}

/// Stop a rotate train for good action.
fn rotate_train_stop(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    train_stop(game, id, false)
}

/// Tick a clock hand and fire its midnight event (`hip:clock`).
fn clock_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (cnt, count, ltime, movedir, event) = game
        .entity(id)
        .map(|entity| {
            (
                entity.number("cnt"),
                entity.count,
                entity.number("ltime"),
                entity.movedir,
                entity.text("event"),
            )
        })
        .unwrap_or((0.0, 1.0, 0.0, ZERO, String::new()));
    let position = (game.time + cnt) / count;
    let angle = 360.0 * (position - position.floor());
    if !event.is_empty() && ltime > angle {
        target_event(game, id, &event, "")?;
    }
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(vscale(movedir, angle)),
            ..Default::default()
        },
    )?;
    rotate_targets_final(game, id)?;
    game.update_entity(id, |entity| number(entity, "ltime", angle))?;
    later(game, id, 1.0, "hip:clock")
}

/// Link clock targets, then start ticking.
fn clock_first(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    link_rotate_targets(game, id)?;
    game.invoke_action(id, "hip:clock")
}

/// Spawn an `info_rotate`.
fn spawn_info_rotate(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    later(game, id, 2.0, "SUB_Remove")
}

/// Spawn a `path_rotate`.
fn spawn_path_rotate(_game: &mut Q1EntityServices, _id: &ActorId) -> Result<(), Q1Error> {
    Ok(())
}

/// Spawn a `rotate_object`.
fn spawn_rotate_object(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
    })
}

/// Link a continuous rotator, starting it when flagged.
fn rotate_first(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    link_rotate_targets(game, id)?;
    let use_name = game.named.use_callback("hip:rotate_entity")?;
    let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
    game.update_entity(id, |entity| {
        entity.use_callback = Some(use_name);
        number(entity, "rotate_state", if spawnflags & 2 != 0 { 0.0 } else { 1.0 });
    })?;
    if spawnflags & 2 != 0 {
        let time = game.time;
        game.update_entity(id, |entity| number(entity, "ltime", time))?;
        later(game, id, 0.02, "hip:rotate_entity")?;
    }
    Ok(())
}

/// Spawn a `func_rotate_entity`.
fn spawn_rotate_entity(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let speed = game.entity(id).map(|entity| entity.speed).unwrap_or(0.0);
    let time = game.time;
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
        if speed != 0.0 {
            number(entity, "cnt", 1.0 / speed);
        }
        number(entity, "ltime", time);
    })?;
    later(game, id, 0.1, "hip:rotate_first")
}

/// Spawn a `func_rotate_door`.
fn spawn_rotate_door(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game
        .entity(id)
        .map(|entity| entity.target.clone())
        .unwrap_or_default()
        .is_empty()
    {
        return Err(q1_error("rotate_door without target"));
    }
    let angles = game.body(id)?.angles;
    game.update_entity(id, |entity| {
        entity.dest1 = ZERO;
        entity.dest2 = angles;
        if entity.speed == 0.0 {
            entity.speed = 2.0;
        }
        entity.damage = if entity.damage == 0.0 {
            2.0
        } else {
            entity.damage.max(0.0)
        };
        number(entity, "cnt", 0.0);
        number(entity, "rotate_state", 4.0);
        if entity.sounds == 0 {
            entity.sounds = 1;
        }
        let sounds = entity.sounds;
        entity.fields.insert(
            "noise2".to_string(),
            if sounds == 2 {
                "doors/airdoor1.wav"
            } else if sounds == 3 {
                "doors/basesec1.wav"
            } else {
                "doors/winch2.wav"
            }
            .to_string(),
        );
        entity.fields.insert(
            "noise3".to_string(),
            if sounds == 2 {
                "doors/airdoor2.wav"
            } else if sounds == 3 {
                "doors/basesec2.wav"
            } else {
                "doors/drclos4.wav"
            }
            .to_string(),
        );
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
    })?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    let use_name = game.named.use_callback("hip:rotate_door")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))
}

/// Spawn a `func_movewall`.
fn spawn_movewall(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
    game.update_entity(id, |entity| {
        entity.movement = Q1MoveType::Push;
        entity.solid = if spawnflags & 4 != 0 {
            Q1Solid::None
        } else {
            Q1Solid::Bsp
        };
    })?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    let solid = game.entity(id).map(|entity| entity.solid).unwrap_or(Q1Solid::None);
    if solid == Q1Solid::Bsp {
        let blocked_name = game.named.blocked("hip:movewall")?;
        game.update_entity(id, |entity| entity.blocked = Some(blocked_name))?;
    }
    if spawnflags & 2 != 0 {
        let touch_name = game.named.touch("hip:movewall")?;
        game.update_entity(id, |entity| entity.touch = Some(touch_name))?;
    }
    let time = game.time;
    game.update_entity(id, |entity| {
        if spawnflags & 1 == 0 {
            entity.model.clear();
        }
        number(entity, "ltime", time);
    })?;
    later(game, id, 0.02, "hip:movewall")
}

/// Spawn a `func_rotate_train`.
fn spawn_rotate_train(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game
        .entity(id)
        .map(|entity| entity.target.clone())
        .unwrap_or_default()
        .is_empty()
    {
        return Err(q1_error("rotate_train without target"));
    }
    let sounds = game.entity(id).map(|entity| entity.sounds).unwrap_or(0);
    let origin = game.body(id)?.origin;
    let time = game.time;
    game.update_entity(id, |entity| {
        if entity.speed == 0.0 {
            entity.speed = 100.0;
        }
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::Step;
        if entity.text("noise").is_empty() {
            entity.fields.insert(
                "noise".to_string(),
                if sounds == 1 {
                    "plats/train2.wav"
                } else {
                    "misc/null.wav"
                }
                .to_string(),
            );
        }
        if entity.text("noise1").is_empty() {
            entity.fields.insert(
                "noise1".to_string(),
                if sounds == 1 {
                    "plats/train1.wav"
                } else {
                    "misc/null.wav"
                }
                .to_string(),
            );
        }
    })?;
    let use_name = game.named.use_callback("hip:rotate_train")?;
    game.update_entity(id, |entity| {
        entity.use_callback = Some(use_name);
        entity
            .fields
            .insert("think1".to_string(), "hip:rotate_train_find".to_string());
        number(entity, "rotate_state", 3.0);
        number(entity, "ltime", time);
        number(entity, "endtime", time + 0.1);
        number(entity, "duration", 1.0);
        number(entity, "cnt", 0.1);
        entity.dest2 = ZERO;
        entity.dest1 = origin;
    })?;
    later(game, id, 0.1, "hip:rotate_train")
}

/// Spawn a `func_clock`.
fn spawn_clock(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let angles = game.body(id)?.angles;
    let direction = move_direction(angles, None);
    game.update_entity(id, |entity| {
        entity.movedir = Vec3 {
            x: -direction.y,
            y: -direction.z,
            z: -direction.x,
        };
        if entity.count == 0.0 {
            entity.count = 60.0;
        }
        let count = entity.count;
        number(entity, "cnt", entity.number("cnt") * count / 12.0);
    })?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    let time = game.time;
    game.update_entity(id, |entity| number(entity, "ltime", time))?;
    later(game, id, 0.1, "hip:clock_first")
}

/// Register Hipnotic rotation entities (`registerHipnoticRotation`).
pub fn register_hipnotic_rotation(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.register_spawn("info_rotate", spawn_info_rotate)?;
    game.register_spawn("path_rotate", spawn_path_rotate)?;
    game.register_spawn("rotate_object", spawn_rotate_object)?;
    game.named.register(
        "hip:rotate_entity",
        Q1CallbackHandlers {
            action: Some(continuous_think),
            use_callback: Some(continuous_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:rotate_first",
        Q1CallbackHandlers {
            action: Some(rotate_first),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_rotate_entity", spawn_rotate_entity)?;
    game.named.register(
        "hip:rotate_door",
        Q1CallbackHandlers {
            action: Some(rotate_door_action),
            use_callback: Some(rotate_door_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:rotate_door_done",
        Q1CallbackHandlers {
            action: Some(rotate_door_done),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_rotate_door", spawn_rotate_door)?;
    game.named.register(
        "hip:movewall",
        Q1CallbackHandlers {
            action: Some(movewall_action),
            touch: Some(movewall_touch),
            blocked: Some(movewall_blocked),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_movewall", spawn_movewall)?;
    game.named.register(
        "hip:rotate_train_next",
        Q1CallbackHandlers {
            action: Some(train_next),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:rotate_train_wait",
        Q1CallbackHandlers {
            action: Some(rotate_train_wait),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:rotate_train_stop",
        Q1CallbackHandlers {
            action: Some(rotate_train_stop),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:rotate_train_find",
        Q1CallbackHandlers {
            action: Some(rotate_train_find),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:rotate_train",
        Q1CallbackHandlers {
            action: Some(rotate_train_action),
            use_callback: Some(rotate_train_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_rotate_train", spawn_rotate_train)?;
    game.named.register(
        "hip:clock",
        Q1CallbackHandlers {
            action: Some(clock_action),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:clock_first",
        Q1CallbackHandlers {
            action: Some(clock_first),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_clock", spawn_clock)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    #[test]
    fn rotate_first_starts_spinning_when_flagged() {
        let mut game = test_game();
        register_hipnotic_rotation(&mut game).expect("register");
        let id = game.create("func_rotate_entity", None, None).expect("rotator");
        game.update_entity(&id, |entity| {
            entity.spawnflags = 2;
            vector(
                entity,
                "rotate",
                Vec3 {
                    x: 0.0,
                    y: 90.0,
                    z: 0.0,
                },
            );
        })
        .expect("flags");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_action(&id, "hip:rotate_first").expect("first");
        assert_eq!(game.entity(&id).expect("entity").number("rotate_state"), 0.0);
        assert_eq!(
            game.entity(&id).expect("entity").think.as_deref(),
            Some("hip:rotate_entity")
        );
        game.time = 0.02;
        game.invoke_action(&id, "hip:rotate_entity").expect("think");
        let yaw = f64::from(game.body(&id).expect("body").angles.y);
        assert!((yaw - 1.8).abs() < 0.01, "yaw {yaw}");
    }

    #[test]
    fn rotate_door_opens_then_settles() {
        let mut game = test_game();
        register_hipnotic_rotation(&mut game).expect("register");
        let id = game.create("func_rotate_door", None, None).expect("door");
        game.update_entity(&id, |entity| entity.target = "t1".to_string())
            .expect("target");
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
        game.spawn_entity(&id, None).expect("spawn");
        assert_eq!(game.entity(&id).expect("door").number("rotate_state"), 4.0);
        game.invoke_use(&id, "hip:rotate_door", None, None).expect("use");
        assert_eq!(game.entity(&id).expect("door").number("rotate_state"), 6.0);
        let endtime = game.entity(&id).expect("door").number("endtime");
        game.time = endtime + 1.0;
        game.invoke_action(&id, "hip:rotate_door").expect("move");
        assert_eq!(
            game.entity(&id).expect("door").think.as_deref(),
            Some("hip:rotate_door_done")
        );
        game.invoke_action(&id, "hip:rotate_door_done").expect("done");
        assert_eq!(game.entity(&id).expect("door").number("rotate_state"), 5.0);
        assert_eq!(
            game.body(&id).expect("body").angles,
            Vec3 {
                x: 0.0,
                y: 90.0,
                z: 0.0
            }
        );
    }

    #[test]
    fn clock_ticks_and_reschedules() {
        let mut game = test_game();
        register_hipnotic_rotation(&mut game).expect("register");
        let id = game.create("func_clock", None, None).expect("clock");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_action(&id, "hip:clock_first").expect("first");
        assert_eq!(game.entity(&id).expect("clock").think.as_deref(), Some("hip:clock"));
        game.time = 1.0;
        game.invoke_action(&id, "hip:clock").expect("tick");
        assert!(game.entity(&id).expect("clock").number("ltime") > 0.0);
        assert_eq!(game.entity(&id).expect("clock").think.as_deref(), Some("hip:clock"));
    }
}
