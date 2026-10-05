//! Rogue dragon (`src/content/q1/missionpacks/monsters/dragon.ts`).
//!
//! qsrc functionality reference: `quake/WinQuake/mathlib.c:155`
//! (`anglemod`, 16-bit fixed-point form).

use std::sync::Arc;

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{angle_mod, Vec3};

use crate::q1::base::animation::MonsterAi;
use crate::q1::base::projectiles::throw_gib;
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{
    dot, length, normalize, vadd, vscale, vsub, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, Q1TraceRequest, POINT,
    ZERO,
};
use crate::q1::missionpacks::rogue_weapons::launch_rogue_plasma;
use crate::q1::missionpacks::types::{velocity_angles, Q1MissionPack};
use crate::q1::Q1Error;

use super::helpers::{missile, number, HULL_BOUNDS};
use super::runtime::{mission_pack_monsters, MissionMonster, Q1MissionPackMonsters};
use super::tables::dragon::FRAMES;
use super::types::{MissionAction, MissionDie, MissionPain, MissionUse, PackMonsterDefinition};

/// Launch a dragon fireball (`launchDragonFireball`).
pub fn launch_dragon_fireball(game: &mut Q1EntityServices, owner: &ActorId, origin: Vec3, direction: Vec3) -> ActorId {
    if game.entity(owner).is_some() {
        let _ = game.update_entity(owner, |entity| entity.effects |= 2);
    }
    let shot = missile(
        game,
        owner,
        "fireball",
        "progs/fireball.mdl",
        origin,
        ZERO,
        "rogue:FireballTouch",
        6.0,
    );
    let speed = game.host.random() * 300.0 + 900.0;
    let _ = game.set_body(
        &shot,
        &BodyPatch {
            velocity: Some(vscale(direction, speed)),
            ..Default::default()
        },
    );
    let _ = game.update_entity(&shot, |entity| {
        entity.angular_velocity = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 300.0,
        };
    });
    if let Ok(body) = game.body(&shot) {
        let _ = game.set_body(
            &shot,
            &BodyPatch {
                angles: Some(velocity_angles(body.velocity)),
                ..Default::default()
            },
        );
    }
    let enemy = game
        .entity(owner)
        .and_then(|entity| entity.monster.as_ref())
        .and_then(|monster| monster.enemy.clone());
    let _ = game.update_entity(&shot, |entity| {
        entity.references.insert("enemy".to_string(), enemy);
    });
    shot
}

/// Current flight destination (`goal`).
fn goal(monster: &MissionMonster) -> Option<ActorId> {
    monster.entity.references.get("movetarget").cloned().flatten()
}

/// End an attack run (`stopAttack`).
fn stop_attack(monster: &mut MissionMonster) {
    if monster.entity.number("dragonAttacking") == 0.0 {
        return;
    }
    let time = monster.game.time;
    let skill = monster.game.options().skill;
    monster.state.attack_finished = time + monster.game.host.random() * 2.0 + 4.0 - skill as f64;
    number(monster, "dragonAttacking", 0.0);
    let destination = goal(monster);
    let end = destination
        .as_ref()
        .and_then(|destination| monster.game.body(destination).ok())
        .map(|body| body.origin)
        .unwrap_or(ZERO);
    let world = monster.game.world.clone();
    let blocked = monster
        .game
        .host
        .trace(&Q1TraceRequest {
            start: monster.origin,
            end,
            bounds: POINT,
            ignore: world,
            monsters: false,
            missile: false,
        })
        .fraction
        != 1.0;
    if blocked {
        for player in (monster.game.host.players)() {
            monster.game.message(
                Some(&player),
                "Error: Dragon cannot get to next target!\n",
                false,
                Vec::new(),
            );
        }
    }
    monster.refresh();
}

/// Check for a flame attack (`checkAttack`).
fn check_attack(monster: &mut MissionMonster) {
    monster.sync();
    let attack = monster.entity.fields.get("dragon:missile").cloned().unwrap_or_default();
    if monster.entity.number("dragonAttacking") == 1.0
        || attack.is_empty()
        || monster.state.attack_finished > monster.game.time
    {
        monster.refresh();
        return;
    }
    if monster
        .enemy
        .clone()
        .is_some_and(|enemy| monster.game.health(&enemy) < 0.0)
    {
        monster.enemy = None;
    }
    if monster.enemy.as_ref().is_some_and(|enemy| {
        monster
            .game
            .entity(enemy)
            .map(|entity| entity.movement_flags)
            .unwrap_or(0)
            & 128
            != 0
    }) {
        monster.refresh();
        return;
    }
    if monster.enemy.is_none() {
        monster.find_target();
        monster.refresh();
        return;
    }
    let Some(target) = monster.target else {
        monster.refresh();
        return;
    };
    let id = monster.entity.actor.id().clone();
    let Ok(body) = monster.game.body(&id) else {
        monster.refresh();
        return;
    };
    let basis = monster.game.make_vectors(body.angles);
    let direction = normalize(vsub(target, monster.origin));
    let world = monster.game.world.clone();
    if f64::from(dot(direction, basis.forward)) <= 0.3
        || monster
            .game
            .host
            .trace(&Q1TraceRequest {
                start: monster.origin,
                end: target,
                bounds: POINT,
                ignore: world,
                monsters: false,
                missile: false,
            })
            .fraction
            != 1.0
    {
        monster.refresh();
        return;
    }
    number(monster, "dragonAttacking", 1.0);
    monster.next_frame = if monster.distance() < 350.0 {
        "dragon_melee1"
    } else {
        attack.as_str()
    }
    .to_string();
}

/// Fly toward the destination or enemy (`move`).
fn dragon_move(monster: &mut MissionMonster, distance: f64) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    if monster.game.health(&id) < 1.0 {
        let _ = monster.game.remove(&id);
        return;
    }
    if monster.entity.number("dragonAttacking") == 0.0 {
        check_attack(monster);
    }
    let previous_enemy = monster.enemy.clone();
    let destination = goal(monster);
    let target = if monster.entity.number("dragonAttacking") == 0.0 {
        destination
            .as_ref()
            .and_then(|destination| monster.game.body(destination).ok())
            .map(|body| body.origin)
            .unwrap_or(ZERO)
    } else {
        monster.target.unwrap_or(ZERO)
    };
    if monster.entity.number("dragonAttacking") == 0.0 {
        monster.enemy = destination.clone();
    }
    let direction = vsub(target, monster.origin);
    let desired = velocity_angles(direction);
    let Ok(body) = monster.game.body(&id) else {
        monster.enemy = previous_enemy;
        monster.refresh();
        return;
    };
    let original = body.angles;
    let mut yaw = f64::from(original.y);
    let mut roll = f64::from(original.z);
    let offset = yaw - f64::from(desired.y);
    if offset != 0.0 {
        let offset = 180.0 - yaw;
        let mut left = angle_mod(f64::from(desired.y) + offset) - 180.0;
        let mut right = 180.0 - angle_mod(f64::from(desired.y) + offset);
        if left < 0.0 {
            left = 360.0;
        } else if right < 0.0 {
            right = 360.0;
        }
        monster.entity.yaw_speed = 10.0;
        monster.flush_entity();
        if right < 180.0 {
            yaw = if monster.entity.yaw_speed < right {
                yaw - monster.entity.yaw_speed
            } else {
                f64::from(desired.y)
            };
            if right > 5.0 {
                roll = (roll + 5.0).min(30.0);
            }
        } else {
            yaw = if monster.entity.yaw_speed < right {
                yaw + monster.entity.yaw_speed
            } else {
                f64::from(desired.y)
            };
            if left > 5.0 {
                roll = (roll - 5.0).max(-30.0);
            }
        }
    } else if roll != 0.0 {
        if roll < -5.0 {
            roll += 5.0;
        } else if roll < 5.0 {
            roll = 0.0;
        } else if roll > 5.0 {
            roll -= 5.0;
        }
    }
    let _ = monster.game.set_body(
        &id,
        &BodyPatch {
            angles: Some(Vec3 {
                x: original.x,
                y: yaw as f32,
                z: roll as f32,
            }),
            ..Default::default()
        },
    );
    if direction.z > 5.0 {
        let _ = monster
            .game
            .set_origin(&id, vadd(monster.origin, Vec3 { x: 0.0, y: 0.0, z: 5.0 }));
    } else if direction.z < -5.0 {
        let _ = monster
            .game
            .set_origin(&id, vsub(monster.origin, Vec3 { x: 0.0, y: 0.0, z: 5.0 }));
    }
    monster.refresh();
    let before = monster.origin;
    let owned = monster.entity.actor.clone();
    monster.game.host.walk_move(&owned, yaw, distance);
    monster.refresh();
    let after = monster.origin;
    if before.x == after.x && before.y == after.y && before.z == after.z {
        if let Some(movement_goal) = monster.entity.references.get("goalentity").cloned().flatten() {
            let owned = monster.entity.actor.clone();
            monster.game.host.move_to_goal(&owned, &movement_goal, distance, None);
        }
    }
    monster.enemy = previous_enemy;
    monster.refresh();
}

/// Breathe fire and plasma (`fire`).
fn fire(monster: &mut MissionMonster) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    let _ = monster.game.sound_simple(&id, "dragon/attack.wav");
    let Ok(body) = monster.game.body(&id) else {
        monster.refresh();
        return;
    };
    let basis = monster.game.make_vectors(body.angles);
    let origin = vadd(
        vadd(monster.origin, vscale(basis.forward, 112.0)),
        vscale(basis.up, 32.0),
    );
    let plasma = monster.game.host.random() > 0.66;
    let skill = monster.game.options().skill;
    let mut count = if plasma {
        if skill > 1 {
            2
        } else {
            1
        }
    } else {
        (monster.game.host.random() * skill as f64 + 0.5).floor() as i32 + 1
    };
    while count > 0 {
        count -= 1;
        let distortion = (monster.game.host.random() - 0.5) * 0.25;
        let direction = normalize(vsub(monster.target.unwrap_or(ZERO), origin));
        let spray = monster.game.make_vectors(direction);
        let aim = vadd(direction, vscale(spray.right, distortion));
        if plasma {
            let _ = launch_rogue_plasma(monster.game, &id, origin, aim);
        } else {
            launch_dragon_fireball(monster.game, &id, origin, aim);
        }
    }
    monster.refresh();
}

/// Tail swipe (`tail`).
fn tail(monster: &mut MissionMonster) {
    let id = monster.entity.actor.id().clone();
    let Some(enemy) = monster.enemy.clone() else {
        monster.refresh();
        return;
    };
    if !monster.game.can_damage(&enemy, &id) {
        monster.refresh();
        return;
    }
    dragon_move(monster, 10.0);
    let Some(target) = monster.target else {
        monster.refresh();
        return;
    };
    let delta = vsub(target, monster.origin);
    if f64::from(length(delta)) < 250.0 {
        monster
            .game
            .damage(&enemy, Some(&id), Some(&id), 30.0, &Q1DamageParams::default());
        let owned = monster.game.host.actors.resolve_owned(&enemy);
        let body = monster.game.host.bodies.read(&enemy);
        if let (Some(owned), Some(body)) = (owned, body) {
            let mut moved = body.clone();
            moved.velocity = Vec3 {
                x: vscale(normalize(delta), 500.0).x,
                y: vscale(normalize(delta), 500.0).y,
                z: 350.0,
            };
            let _ = monster.game.host.bodies.write(&owned, &moved);
        }
    }
    stop_attack(monster);
}

/// Gib the dragon across the room (`violentDeath`).
fn violent_death(monster: &mut MissionMonster, mut count: i32) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    while count > 0 {
        count -= 3;
        for model in ["gib1", "gib2", "gib3"] {
            let Ok(body) = monster.game.body(&id) else {
                continue;
            };
            let Ok(gib) = monster.game.create("gib", None, None) else {
                continue;
            };
            let _ = monster.game.update_entity(&gib, |entity| {
                entity.model = format!("progs/{model}.mdl");
            });
            let velocity = vscale(body.velocity, -1.25);
            let basis = monster.game.make_vectors(velocity);
            let spray_right = monster.game.host.random() * 300.0 - 150.0;
            let spray_up = monster.game.host.random() * 300.0 - 150.0;
            let _ = monster.game.set_body(
                &gib,
                &BodyPatch {
                    origin: Some(monster.origin),
                    bounds: Some(qa_core::math::Bounds {
                        min: Vec3 {
                            x: -8.0,
                            y: -8.0,
                            z: -8.0,
                        },
                        max: Vec3 { x: 8.0, y: 8.0, z: 8.0 },
                    }),
                    velocity: Some(vadd(
                        vadd(velocity, vscale(basis.right, spray_right)),
                        vscale(monster.game.basis.up, spray_up),
                    )),
                    ..Default::default()
                },
            );
            let spin = Vec3 {
                x: monster.game.host.random() as f32 * 600.0,
                y: monster.game.host.random() as f32 * 600.0,
                z: monster.game.host.random() as f32 * 600.0,
            };
            let time = monster.game.time;
            let _ = monster.game.update_entity(&gib, |entity| {
                entity.movement = Q1MoveType::Bounce;
                entity.angular_velocity = spin;
                entity.fields.insert("ltime".to_string(), time.to_string());
            });
            if let Ok(remove) = monster.game.named.action("SUB_Remove") {
                let lifetime = 10.0 + monster.game.host.random() * 10.0;
                let _ = monster.game.schedule(&gib, lifetime, &remove);
            }
            let _ = monster.game.link(&gib);
        }
    }
    monster.refresh();
}

/// Finish the death sequence (`finishDeath`).
fn finish_death(monster: &mut MissionMonster, count: i32) {
    violent_death(monster, count);
    let id = monster.entity.actor.id().clone();
    monster.entity.target = "dragondoor".to_string();
    monster.flush_entity();
    let activator = monster.entity.activator.clone();
    let _ = monster.game.use_targets(&id, activator.as_ref());
    let _ = monster.game.remove(&id);
}

/// Explode on the ground (`boom`).
fn boom(monster: &mut MissionMonster) {
    if monster.entity.number("dragonDeathState") > 2.0 {
        return;
    }
    number(monster, "dragonDeathState", 3.0);
    for model in ["drggib01", "drggib02", "drggib03"] {
        let _ = throw_gib(monster.game, monster.origin, model, -100.0);
    }
    let id = monster.entity.actor.id().clone();
    let _ = monster
        .game
        .sound(&id, "player/tornoff2.wav", Q1SoundChannel::Body, 0.0, 1.0);
    monster.next_frame = "dragon_boom2".to_string();
    monster.delay(0.1);
}

/// Fall out of the sky (`explode`).
fn explode(monster: &mut MissionMonster) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    if monster.entity.number("dragonDeathState") > 1.0 {
        monster.refresh();
        return;
    }
    let Ok(body) = monster.game.body(&id) else {
        monster.refresh();
        return;
    };
    if f64::from(length(body.velocity)) < 100.0 || monster.entity.movement_flags & 16 != 0 {
        number(monster, "dragonDeathState", 2.0);
        boom(monster);
        return;
    }
    let basis = monster.game.make_vectors(body.angles);
    let velocity = vsub(body.velocity, vscale(basis.up, 40.0));
    let _ = monster.game.set_body(
        &id,
        &BodyPatch {
            velocity: Some(velocity),
            ..Default::default()
        },
    );
    monster.entity.fields.insert(
        "dragonLastVelocity".to_string(),
        format!("{} {} {}", velocity.x, velocity.y, velocity.z),
    );
    monster.flush_entity();
    monster.refresh();
}

/// Wake the dragon (`use`).
fn use_dragon(monster: &mut MissionMonster) {
    let id = monster.entity.actor.id().clone();
    if monster.game.health(&id) < 1.0 {
        monster.entity.use_callback = None;
        monster.flush_entity();
        return;
    }
    monster.next_frame = "dragon_walk1".to_string();
    monster.delay(0.1);
}

/// Detonate a fireball (`FireballTouch`).
fn fireball_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let (id, other) = (id.clone(), other.clone());
    let owner = game.entity(&id).and_then(|entity| entity.owner.clone());
    if owner.as_ref().is_some_and(|owner| same_actor(&other, owner)) {
        return Ok(());
    }
    let dragon = owner
        .as_ref()
        .is_some_and(|owner| game.host.classname(owner) == "monster_dragon");
    let world = game.world.clone();
    game.radius_damage(
        &id,
        owner.as_ref(),
        if dragon { 90.0 } else { 30.0 },
        if dragon { owner.as_ref() } else { world.as_ref() },
        None,
        "",
    );
    game.sound(&id, "weapons/r_exp3.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    let origin = game.body(&id).map(|body| body.origin).unwrap_or(ZERO);
    game.host.emit(Q1Event::ColoredExplosion {
        origin,
        color_start: 228,
        color_length: 5,
    });
    game.remove(&id)
}

/// Crush whoever the falling dragon lands on (`dragon_squish`).
fn dragon_squish(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let (id, other) = (id.clone(), other.clone());
    let mut local: Q1MissionPackMonsters = mission_pack_monsters(&Q1MissionPack::Rogue)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Ok(mut monster) = local.require(game, &id) else {
        return Ok(());
    };
    if monster.game.host.classname(&other) == "player" {
        monster.entity.classname = "monster_dragon_dead".to_string();
        monster.flush_entity();
        monster
            .game
            .damage(&other, Some(&id), Some(&id), 200.0, &Q1DamageParams::default());
    }
    if monster
        .game
        .world
        .as_ref()
        .is_some_and(|world| same_actor(&other, world))
    {
        let _ = monster.game.set_body(
            &id,
            &BodyPatch {
                velocity: Some(ZERO),
                ..Default::default()
            },
        );
        explode(&mut monster);
    }
    monster.finish();
    *mission_pack_monsters(&Q1MissionPack::Rogue)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = local;
    Ok(())
}

/// Advance a dragon along its path (`dragon_corner_touch`).
fn dragon_corner_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let (id, other) = (id.clone(), other.clone());
    let entity = game.entity(&other).cloned();
    let current = entity
        .as_ref()
        .and_then(|entity| entity.references.get("movetarget").cloned().flatten());
    let Some(entity) = entity else { return Ok(()) };
    let Some(current) = current else { return Ok(()) };
    if !same_actor(&current, &id) || entity.classname != "monster_dragon" {
        return Ok(());
    }
    let corner_target = game.entity(&id).map(|entity| entity.target.clone()).unwrap_or_default();
    let target = game.find(&corner_target).first().cloned();
    game.update_entity(&other, |entity| {
        entity.references.insert("movetarget".to_string(), target.clone());
        entity.references.insert("goalentity".to_string(), target.clone());
        entity.target = corner_target;
    })?;
    if target.is_none() {
        panic!("dragon_corner: no target found");
    }
    Ok(())
}

/// Rogue dragon definition (`dragonDefinition`).
pub fn dragon_definition(runtime: &Q1MissionPackMonsters) -> PackMonsterDefinition {
    debug_assert!(matches!(runtime.pack, Q1MissionPack::Rogue), "dragon is Rogue-only");
    let mut actions: Vec<(&'static str, MissionAction)> = vec![
        ("dragon_stop_attack", Arc::new(stop_attack)),
        ("dragon_fireball", Arc::new(fire)),
        ("dragon_tail", Arc::new(tail)),
        ("dragon_explode", Arc::new(explode)),
        (
            "dragon_tail_touch",
            Arc::new(|monster| {
                let id = monster.entity.actor.id().clone();
                let Some(enemy) = monster.enemy.clone() else {
                    monster.refresh();
                    return;
                };
                if !monster.game.can_damage(&enemy, &id) {
                    monster.refresh();
                    return;
                }
                monster.ai(MonsterAi::Charge, 10.0);
                if let Some(target) = monster.target {
                    if f64::from(length(vsub(target, monster.origin))) <= 150.0 {
                        let damage = monster.game.host.random() * 30.0 + 30.0;
                        monster
                            .game
                            .damage(&enemy, Some(&id), Some(&id), damage, &Q1DamageParams::default());
                    }
                }
                monster.refresh();
            }),
        ),
        (
            "dragon_boom2",
            Arc::new(|monster| {
                let id = monster.entity.actor.id().clone();
                let velocity = monster.entity.vector("dragonLastVelocity");
                let _ = monster.game.set_body(
                    &id,
                    &BodyPatch {
                        velocity: Some(velocity),
                        ..Default::default()
                    },
                );
                finish_death(monster, 15);
            }),
        ),
        (
            "dragon_activate",
            Arc::new(|monster| {
                let id = monster.entity.actor.id().clone();
                let _ = monster.game.set_damageable(&id, true);
                monster.entity.aimed_damage = true;
                if let Ok(body) = monster.game.body(&id) {
                    monster.entity.ideal_yaw = f64::from(body.angles.y);
                }
                if monster.entity.yaw_speed == 0.0 {
                    monster.entity.yaw_speed = 10.0;
                }
                monster
                    .entity
                    .fields
                    .insert("view_ofs".to_string(), "0 0 25".to_string());
                monster.entity.movement_flags |= 1 | 32;
                monster.flush_entity();
                let owned = monster.entity.actor.clone();
                monster.game.host.walk_move(&owned, 0.0, 0.0);
                if !monster.entity.target.is_empty() {
                    let target = monster.entity.target.clone();
                    let destination = monster.game.find(&target).first().cloned();
                    monster
                        .entity
                        .references
                        .insert("movetarget".to_string(), destination.clone());
                    monster.entity.references.insert("goalentity".to_string(), destination);
                    monster.flush_entity();
                }
                if !monster.entity.targetname.is_empty() {
                    if let Ok(use_callback) = monster.game.named.use_callback("rogue:monster_use") {
                        monster.entity.use_callback = Some(use_callback);
                        monster.flush_entity();
                    }
                    return;
                }
                use_dragon(monster);
            }),
        ),
        (
            "dragon:dragon_walk1",
            Arc::new(|monster| {
                if monster.entity.number("dragonAttacking") != 0.0 {
                    stop_attack(monster);
                }
                monster
                    .entity
                    .fields
                    .insert("dragon:missile".to_string(), "dragon_atk_a1".to_string());
                monster.flush_entity();
                number(monster, "dragonPainSequence", 1.0);
                dragon_move(monster, 17.0);
                if monster.game.host.random() < 0.2 {
                    let id = monster.entity.actor.id().clone();
                    let _ = monster
                        .game
                        .sound(&id, "dragon/active.wav", Q1SoundChannel::Voice, 2.0, 0.6);
                }
            }),
        ),
        (
            "dragon:dragon_walk2",
            Arc::new(|monster| {
                monster
                    .entity
                    .fields
                    .insert("dragon:missile".to_string(), String::new());
                monster.flush_entity();
                dragon_move(monster, 17.0);
            }),
        ),
        (
            "dragon:dragon_walk13",
            Arc::new(|monster| {
                monster
                    .entity
                    .fields
                    .insert("dragon:missile".to_string(), String::new());
                monster.flush_entity();
                dragon_move(monster, 17.0);
                number(monster, "dragonPainSequence", 1.0);
            }),
        ),
        (
            "dragon:dragon_death1",
            Arc::new(|monster| {
                if monster.entity.number("dragonDeathState") > 0.0 {
                    return;
                }
                number(monster, "dragonDeathState", 1.0);
                monster.entity.use_callback = None;
                monster.flush_entity();
                let id = monster.entity.actor.id().clone();
                if let Ok(body) = monster.game.body(&id) {
                    let basis = monster.game.make_vectors(body.angles);
                    let _ = monster.game.set_body(
                        &id,
                        &BodyPatch {
                            velocity: Some(vsub(vscale(basis.forward, 300.0), vscale(basis.up, 40.0))),
                            ground: Some(None),
                            ..Default::default()
                        },
                    );
                }
                monster.entity.movement_flags &= !512;
                monster.flush_entity();
                let _ = monster.game.set_bounds(&id, HULL_BOUNDS);
                if let Ok(touch) = monster.game.named.touch("rogue:dragon_squish") {
                    monster.entity.touch = Some(touch);
                    monster.flush_entity();
                }
                let _ = monster
                    .game
                    .sound(&id, "dragon/death.wav", Q1SoundChannel::Voice, 0.0, 1.0);
                number(monster, "dragonAttacking", 0.0);
            }),
        ),
        ("dragon:dragon_death21", Arc::new(|monster| finish_death(monster, 39))),
    ];
    for distance in [10, 12, 17] {
        actions.push((
            match distance {
                10 => "dragon_move(10)",
                12 => "dragon_move(12)",
                _ => "dragon_move(17)",
            },
            Arc::new(move |monster: &mut MissionMonster| dragon_move(monster, distance as f64)),
        ));
    }
    for (frame, attack, pain) in [
        (3, "b", 2.0),
        (5, "c", 3.0),
        (7, "d", 4.0),
        (9, "e", 5.0),
        (11, "f", 6.0),
    ] {
        actions.push((
            match frame {
                3 => "dragon:dragon_walk3",
                5 => "dragon:dragon_walk5",
                7 => "dragon:dragon_walk7",
                9 => "dragon:dragon_walk9",
                _ => "dragon:dragon_walk11",
            },
            Arc::new(move |monster: &mut MissionMonster| {
                monster
                    .entity
                    .fields
                    .insert("dragon:missile".to_string(), format!("dragon_atk_{attack}1"));
                monster.flush_entity();
                dragon_move(monster, 17.0);
                number(monster, "dragonPainSequence", pain);
            }),
        ));
    }
    let use_: MissionUse = Arc::new(|monster, _activator| use_dragon(monster));
    let pain: MissionPain = Arc::new(|monster, _attacker, _damage| {
        if monster.state.pain_finished > monster.game.time || monster.game.host.random() >= 0.25 {
            return;
        }
        stop_attack(monster);
        let id = monster.entity.actor.id().clone();
        let _ = monster.game.sound_simple(&id, "dragon/pain.wav");
        let time = monster.game.time;
        monster.state.pain_finished = time + 2.0;
        let name = match monster.entity.number("dragonPainSequence") as i32 {
            1 => Some("A"),
            2 => Some("F"),
            3 => Some("E"),
            4 => Some("D"),
            5 => Some("C"),
            6 => Some("B"),
            _ => None,
        };
        if let Some(name) = name {
            monster.next_frame = format!("dragon_pain{name}1");
        }
    });
    let die: MissionDie = Arc::new(|monster, _attacker| {
        monster.play("dragon_death1");
    });
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::Dragon,
            kill_string: None,
            classnames: &["monster_dragon"],
            model: "dragon",
            head: None,
            health: 4000.0,
            gib_health: f64::NEG_INFINITY,
            gibs: &[],
            bounds: qa_core::math::Bounds {
                min: Vec3 {
                    x: -32.0,
                    y: -32.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 32.0,
                    y: 32.0,
                    z: 64.0,
                },
            },
            stand: "dragon_walk1",
            walk: "dragon_walk1",
            run: "dragon_walk1",
            sight: "dragon/see.wav",
            missile: None,
            melee: false,
            movement: MonsterMovement::Fly,
        })),
        base_behavior: false,
        frames: FRAMES,
        actions,
        callbacks: vec![
            (
                "FireballTouch",
                Q1CallbackHandlers {
                    touch: Some(fireball_touch),
                    ..Default::default()
                },
            ),
            (
                "dragon_squish",
                Q1CallbackHandlers {
                    touch: Some(dragon_squish),
                    ..Default::default()
                },
            ),
            (
                "dragon_corner_touch",
                Q1CallbackHandlers {
                    touch: Some(dragon_corner_touch),
                    ..Default::default()
                },
            ),
        ],
        spawn: Some(Arc::new(|monster| {
            number(monster, "dragonInRoom", 1.0);
            number(monster, "dragonInTransit", 0.0);
            number(monster, "dragonAttacking", 0.0);
            number(monster, "playerInRoom", 1.0);
            number(monster, "playerInTransit", 0.0);
            let id = monster.entity.actor.id().clone();
            monster.entity.solid = Q1Solid::Slidebox;
            monster.entity.movement = Q1MoveType::Step;
            monster.entity.yaw_speed = monster.entity.number("yaw_speed");
            monster.entity.model = "progs/dragon.mdl".to_string();
            monster.flush_entity();
            let _ = monster.game.set_bounds(&id, monster.spec.bounds);
            monster.entity.max_health = 3000.0 + 1000.0 * monster.game.options().skill as f64;
            monster.flush_entity();
            let owned = monster.entity.actor.clone();
            let _ = monster.game.host.combat.set_health(&owned, monster.entity.max_health);
            if let Ok(pain) = monster.game.named.pain("rogue:monster_pain") {
                monster.entity.pain = Some(pain);
            }
            if let Ok(die) = monster.game.named.die("rogue:monster_die") {
                monster.entity.die = Some(die);
            }
            monster.flush_entity();
            number(monster, "dragonPainSequence", 1.0);
            monster.game.total_monsters += 1;
            monster.next_frame = "dragon_activate".to_string();
            let time = monster.game.time;
            monster.delay(0.1 - time);
        })),
        start: None,
        pain,
        die,
        melee: None,
        check_attack: None,
        found: None,
        ai: None,
        use_: Some(use_),
    }
}

/// Register dragon path corners (`registerDragonCorners`).
pub fn register_dragon_corners(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.register_spawn("trigger_dragon", |game, id| game.remove(id))?;
    game.register_spawn("dragon_corner", |game, id| {
        if game.entity(id).map(|entity| entity.targetname.as_str()) == Some("") {
            panic!("dragon_corner: no targetname");
        }
        let touch = game.named.touch("rogue:dragon_corner_touch")?;
        game.update_entity(id, |entity| {
            entity.solid = Q1Solid::Trigger;
            entity.movement = Q1MoveType::None;
            entity.touch = Some(touch);
            entity.model = String::new();
        })?;
        game.set_bounds(
            id,
            qa_core::math::Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -16.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 16.0,
                },
            },
        )?;
        game.link(id)
    })
}

#[cfg(test)]
mod tests {
    use super::{dragon_definition, register_dragon_corners};
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::MissionMonsterHooks;
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};

    #[test]
    fn dragon_registers_with_fireball_touch() {
        let mut game = test_game();
        let mut runtime = Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Rogue, MissionMonsterHooks::default())
            .expect("runtime");
        let definition = dragon_definition(&runtime);
        assert_eq!(definition.spec.classnames, &["monster_dragon"]);
        assert_eq!(definition.spec.model, "dragon");
        assert_eq!(definition.spec.health, 4000.0);
        assert!(!definition.frames.is_empty());
        assert_eq!(definition.actions.len(), 12 + 3 + 5);
        assert_eq!(definition.callbacks.len(), 3);
        runtime.register(&mut game, definition).expect("register");
        register_dragon_corners(&mut game).expect("corners");
        let id = game.create("monster_dragon", None, None).expect("create");
        let monster = runtime.require(&mut game, &id).expect("require");
        assert_eq!(monster.definition.spec.model, "dragon");
    }
}
