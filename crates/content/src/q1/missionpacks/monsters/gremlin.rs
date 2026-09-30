//! Hipnotic gremlin (`src/content/q1/missionpacks/monsters/gremlin.ts`).

use std::sync::Arc;

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::contract::{InventoryCountPolicy, InventoryEntry, SourceCounterArithmetic};
use crate::q1::base::projectiles::{spawn_meat_spray, throw_gib, throw_head};
use crate::q1::base::species::{MonsterMovement, MonsterSpecies};
use crate::q1::foundation::callbacks::{callback_name, Q1CallbackHandlers};
use crate::q1::foundation::entity::{Q1AttackState, Q1MonsterSpecies};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{
    dot, normalize, vadd, vscale, vsub, Q1MoveType, Q1Powerup, Q1SoundChannel, Q1TraceRequest, POINT, ZERO,
};
use crate::q1::missionpacks::types::Q1MissionPack;
use crate::q1::Q1Error;

use super::gremlin_ai::{gremlin_run, gremlin_stand, gremlin_walk};
use super::gremlin_weapons::{
    gremlin_drop_backpack, gremlin_fire_laser, gremlin_fire_lightning, gremlin_fire_nail, gremlin_steal,
    gremlin_weapon_attack,
};
use super::helpers::{number, radius_actors, HULL_BOUNDS};
use super::runtime::{mission_pack_monsters, MissionMonster, Q1MissionPackMonsters};
use super::tables::hipgrem::FRAMES;
use super::types::{leaked_name, MissionAction, MissionCheckAttack, MissionDie, MissionPain, PackMonsterDefinition};

/// Gib the gremlin (`gib`).
fn gib(monster: &mut MissionMonster, damage: f64) {
    let id = monster.entity.actor.id().clone();
    let _ = monster.game.sound_simple(&id, "player/udeath.wav");
    let _ = throw_head(monster.game, &id, "h_grem", damage);
    for _ in 0..3 {
        let _ = throw_gib(monster.game, monster.origin, "gib1", damage);
    }
    monster.refresh();
}

/// Resume hunting the previous enemy (`resume`).
fn resume(monster: &mut MissionMonster) {
    if let Some(old) = monster.state.old_enemy.clone() {
        if monster.game.health(&old) > 0.0 {
            monster.enemy = Some(old.clone());
            monster.entity.references.insert("goalentity".to_string(), Some(old));
            monster.flush_entity();
            monster.next_frame = monster.spec.run.to_string();
            let time = monster.game.time;
            monster.state.attack_finished = time + 1.0;
            monster.delay(0.1);
            return;
        }
    }
    let frame = if monster.state.path.is_empty() {
        "gremlin_stand1"
    } else {
        "gremlin_walk1"
    };
    monster.play(frame);
}

/// Claw the enemy (`melee`).
fn melee(monster: &mut MissionMonster, side: f64) {
    monster.face();
    let id = monster.entity.actor.id().clone();
    let Some(enemy) = monster.enemy.clone() else {
        monster.refresh();
        return;
    };
    if monster.distance() > 100.0 || !monster.game.can_damage(&enemy, &id) {
        monster.refresh();
        return;
    }
    let _ = monster
        .game
        .sound(&id, "grem/attack.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    let damage = 10.0 + 5.0 * monster.game.host.random();
    monster
        .game
        .damage(&enemy, Some(&id), Some(&id), damage, &Q1DamageParams::default());
    if let Ok(body) = monster.game.body(&id) {
        let basis = monster.game.make_vectors(body.angles);
        let origin = vadd(monster.origin, vscale(basis.forward, 16.0));
        let _ = spawn_meat_spray(monster.game, &id, origin, vscale(basis.right, side));
    }
    monster.refresh();
}

/// Split a fed gremlin into two (`split`).
fn split(monster: &mut MissionMonster) {
    monster.sync();
    if monster.runtime.spawned_gremlins >= monster.runtime.authored_gremlins * 2 {
        monster.refresh();
        return;
    }
    let id = monster.entity.actor.id().clone();
    let Ok(body) = monster.game.body(&id) else {
        monster.refresh();
        return;
    };
    let mut angles = body.angles;
    let mut position = monster.origin;
    let mut found = false;
    for _ in 0..10 {
        position = vadd(monster.origin, vscale(monster.game.make_vectors(angles).forward, 80.0));
        let mut proceed = true;
        for actor in radius_actors(monster.game, position, 35.0) {
            if monster.game.health(&actor) > 0.0
                && (monster
                    .game
                    .entity(&actor)
                    .map(|entity| entity.movement_flags)
                    .unwrap_or(0)
                    & 32
                    != 0
                    || monster.game.is_player(&actor))
            {
                proceed = false;
            }
        }
        let clear = |monster: &mut MissionMonster, end: Vec3| {
            monster
                .game
                .host
                .trace(&Q1TraceRequest {
                    start: monster.origin,
                    end,
                    bounds: POINT,
                    ignore: Some(id.clone()),
                    monsters: true,
                    missile: false,
                })
                .fraction
                == 1.0
        };
        if clear(monster, position)
            && proceed
            && clear(
                monster,
                vsub(
                    position,
                    Vec3 {
                        x: 40.0,
                        y: 40.0,
                        z: 0.0,
                    },
                ),
            )
            && clear(
                monster,
                vadd(
                    position,
                    Vec3 {
                        x: 40.0,
                        y: 40.0,
                        z: 0.0,
                    },
                ),
            )
            && clear(
                monster,
                vadd(
                    position,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 64.0,
                    },
                ),
            )
            && !clear(
                monster,
                vsub(
                    position,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 64.0,
                    },
                ),
            )
        {
            found = true;
            break;
        }
        angles = Vec3 {
            x: angles.x,
            y: angles.y + 36.0,
            z: angles.z,
        };
    }
    if !found {
        monster.refresh();
        return;
    }
    monster.runtime.spawned_gremlins += 1;
    let Ok(child) = monster.game.clone_entity(&id) else {
        monster.refresh();
        return;
    };
    let _ = monster.game.update_entity(&child, |entity| {
        entity.solid = crate::q1::foundation::types::Q1Solid::Slidebox;
        entity.movement = Q1MoveType::Step;
        entity.model = "progs/grem.mdl".to_string();
    });
    let _ = monster.game.set_bounds(&child, HULL_BOUNDS);
    let health = 100.0_f64.max(monster.game.health(&id));
    let owned = monster.entity.actor.clone();
    let child_owned = monster.game.host.actors.resolve_owned(&child);
    let _ = monster.game.host.combat.set_health(&owned, health / 2.0);
    if let Some(child_owned) = &child_owned {
        let _ = monster.game.host.combat.set_health(child_owned, health / 2.0);
    }
    if let Some(child_owned) = &child_owned {
        for mut entry in monster.game.host.inventory.entries(&child) {
            if entry.item.starts_with("q1:weapon/") {
                entry.count = 0.0;
                let _ = monster.game.host.inventory.configure(child_owned, &entry);
            }
        }
    }
    monster.game.total_monsters += 1;
    let total = monster.game.total_monsters;
    monster
        .game
        .host
        .emit(crate::q1::foundation::types::Q1Event::MonsterTotal { total });
    let _ = monster.game.set_origin(&child, position);
    let game = &mut *monster.game;
    let runtime = &mut *monster.runtime;
    let Ok(mut controller) = runtime.require(game, &child) else {
        return;
    };
    number(&mut controller, "stoleweapon", 0.0);
    controller.play("gremlin_spawn1");
    controller.enemy = None;
    number(&mut controller, "gorging", 0.0);
    controller.finish();
}

/// Damage a gorging victim, honouring protection (`gorgeDamage`).
fn gorge_damage(monster: &mut MissionMonster, target: &ActorId, damage: f64) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    let victim = monster.game.entity(target).cloned();
    let owned = monster.game.host.actors.resolve_owned(target);
    let combat = monster.game.host.combat.read(target);
    let (Some(owned), Some(combat)) = (owned, combat) else {
        monster.refresh();
        return;
    };
    if victim.as_ref().map(|entity| entity.movement_flags).unwrap_or(0) & 64 != 0 {
        monster.refresh();
        return;
    }
    let invulnerable_until = monster
        .game
        .player_ref(target)
        .and_then(|player| player.powerups.get(&Q1Powerup::Invulnerability).copied())
        .or_else(|| victim.as_ref().map(|entity| entity.number("invincible_finished")))
        .unwrap_or(0.0);
    if invulnerable_until >= monster.game.time {
        if monster.entity.number("invincible_sound") < monster.game.time {
            let _ = monster
                .game
                .sound(target, "items/protect3.wav", Q1SoundChannel::Item, 1.0, 1.0);
            let time = monster.game.time;
            number(monster, "invincible_sound", time + 2.0);
        }
        monster.refresh();
        return;
    }
    if monster.game.options().teamplay == Some(1)
        && combat.team.is_some()
        && combat.team == monster.game.host.combat.read(&id).and_then(|combat| combat.team)
    {
        monster.refresh();
        return;
    }
    let _ = monster
        .game
        .host
        .combat
        .set_health(&owned, f64::from((combat.health - damage) as f32));
    monster.refresh();
}

/// Feed on a corpse (`gorge`).
fn gorge(monster: &mut MissionMonster, side: f64) {
    monster.sync();
    let id = monster.entity.actor.id().clone();
    let Some(target) = monster.enemy.clone() else {
        monster.refresh();
        return;
    };
    let _ = monster
        .game
        .sound(&id, "demon/dhit2.wav", Q1SoundChannel::Weapon, 1.0, 1.0);
    let damage = 7.0 + 5.0 * monster.game.host.random();
    gorge_damage(monster, &target, damage);
    if let Ok(body) = monster.game.body(&id) {
        let basis = monster.game.make_vectors(body.angles);
        let origin = vadd(monster.origin, vscale(basis.forward, 16.0));
        let _ = spawn_meat_spray(monster.game, &id, origin, vscale(basis.right, side));
    }
    if monster.game.health(&target) >= -200.0 {
        monster.refresh();
        return;
    }
    if let Some(victim) = monster.game.entity(&target).cloned() {
        if victim.number("gorging") == 0.0 {
            let _ = monster.game.update_entity(&target, |entity| {
                entity.fields.insert("gorging".to_string(), "1".to_string());
            });
            let _ = monster.game.sound_simple(&id, "player/udeath.wav");
            let head = match victim.classname.as_str() {
                "monster_ogre" => "h_ogre",
                "monster_knight" => "h_knight",
                "monster_shambler" => "h_shams",
                "monster_demon1" => "h_demon",
                "monster_wizard" => "h_wizard",
                "monster_zombie" => "h_zombie",
                "monster_dog" => "h_dog",
                "monster_hell_knight" => "h_hellkn",
                "monster_enforcer" => "h_mega",
                "monster_army" => "h_guard",
                "monster_shalrath" => "h_shal",
                "monster_gremlin" => "h_grem",
                "monster_scourge" => "h_scourg",
                "monster_fish" => "gib1",
                _ => "h_player",
            };
            let _ = throw_head(monster.game, &target, head, -15.0);
            let amount = 150.0 + 100.0 * monster.game.host.random();
            let health = monster.game.health(&id);
            if health > 0.0 && health < monster.entity.max_health {
                let owned = monster.entity.actor.clone();
                let _ = monster
                    .game
                    .host
                    .combat
                    .set_health(&owned, monster.entity.max_health.min(health + amount.ceil()));
            }
            split(monster);
        }
    }
    monster.enemy = None;
    number(monster, "gorging", 0.0);
    monster.play("gremlin_look1");
}

/// Choose a melee attack (`meleeAttack`).
fn melee_attack(monster: &mut MissionMonster) {
    if monster.entity.number("gorging") != 0.0 {
        monster.play("gremlin_gorge1");
        return;
    }
    if monster.entity.number("stoleweapon") == 1.0 {
        panic!("gremlin meleeing with stolen weapon");
    }
    let enemy = monster.enemy.clone();
    if enemy.as_ref().is_some_and(|enemy| monster.game.is_player(enemy))
        && monster.game.host.random() < 0.4
        && gremlin_steal(monster)
    {
        return;
    }
    let rolled = monster.game.host.random();
    monster.play(if rolled < 0.3 {
        "gremlin_claw1"
    } else if rolled < 0.6 {
        "gremlin_lunge1"
    } else {
        "gremlin_claw1"
    });
}

/// Choose a missile attack (`missileAttack`).
fn missile_attack(monster: &mut MissionMonster) {
    if monster.entity.number("stoleweapon") != 0.0 {
        if gremlin_weapon_attack(monster) {
            return;
        }
        if monster.game.host.random() < 0.1 && monster.entity.movement_flags & 512 != 0 {
            monster.play("gremlin_jump1");
            return;
        }
    }
    if monster.entity.movement_flags & 512 != 0 {
        monster.play("gremlin_jump1");
    }
}

/// Land a gremlin leap (`Gremlin_JumpTouch`).
fn gremlin_jump_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let mut local: Q1MissionPackMonsters = mission_pack_monsters(&Q1MissionPack::Hipnotic)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Ok(mut monster) = local.require(game, &id) else {
        return Ok(());
    };
    if monster.game.health(&id) <= 0.0 {
        return Ok(());
    }
    if !monster.game.host.check_bottom(&id) {
        if monster.entity.movement_flags & 512 != 0 {
            monster.entity.touch = None;
            monster.flush_entity();
            monster.next_frame = "gremlin_jump1".to_string();
            monster.delay(0.1);
        }
        monster.finish();
        *mission_pack_monsters(&Q1MissionPack::Hipnotic)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = local;
        return Ok(());
    }
    monster.entity.touch = None;
    monster.flush_entity();
    monster.next_frame = "gremlin_jump12".to_string();
    monster.delay(0.1);
    monster.finish();
    *mission_pack_monsters(&Q1MissionPack::Hipnotic)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = local;
    Ok(())
}

/// Land a gremlin death flip (`Gremlin_FlipTouch`).
fn gremlin_flip_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let mut local: Q1MissionPackMonsters = mission_pack_monsters(&Q1MissionPack::Hipnotic)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let Ok(mut monster) = local.require(game, &id) else {
        return Ok(());
    };
    if !monster.game.host.check_bottom(&id) && monster.entity.movement_flags & 512 != 0 {
        monster.entity.touch = None;
        monster.flush_entity();
        monster.next_frame = "gremlin_flip1".to_string();
        monster.delay(0.1);
    }
    monster.entity.touch = None;
    monster.flush_entity();
    monster.next_frame = "gremlin_flip8".to_string();
    monster.delay(0.1);
    monster.finish();
    *mission_pack_monsters(&Q1MissionPack::Hipnotic)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = local;
    Ok(())
}

/// Hipnotic gremlin definition (`gremlinDefinition`).
pub fn gremlin_definition(runtime: &Q1MissionPackMonsters) -> PackMonsterDefinition {
    debug_assert!(
        matches!(runtime.pack, Q1MissionPack::Hipnotic),
        "gremlin is Hipnotic-only"
    );
    let mut actions: Vec<(&'static str, MissionAction)> = vec![
        ("Gremlin_MeleeAttack", Arc::new(melee_attack)),
        ("Gremlin_MissileAttack", Arc::new(missile_attack)),
        ("Gremlin_FireLightningGun", Arc::new(gremlin_fire_lightning)),
        ("GremlinDropBackpack", Arc::new(gremlin_drop_backpack)),
        ("gremlin_gib", Arc::new(|monster| gib(monster, -35.0))),
        (
            "hipgrem:gremlin_stand1",
            Arc::new(|monster| {
                gremlin_stand(monster);
                monster.delay(0.2);
            }),
        ),
        (
            "hipgrem:gremlin_jump5",
            Arc::new(|monster| {
                monster.face();
                let id = monster.entity.actor.id().clone();
                if monster.entity.movement_flags & 512 == 0 {
                    monster.play("gremlin_run1");
                    return;
                }
                if let Ok(touch) = monster.game.named.touch("hipnotic:Gremlin_JumpTouch") {
                    monster.entity.touch = Some(touch);
                    monster.flush_entity();
                }
                if let Ok(body) = monster.game.body(&id) {
                    let basis = monster.game.make_vectors(body.angles);
                    let origin = vadd(monster.origin, Vec3 { x: 0.0, y: 0.0, z: 1.0 });
                    let _ = monster.game.set_body(
                        &id,
                        &BodyPatch {
                            origin: Some(origin),
                            velocity: Some(vadd(
                                vscale(basis.forward, 300.0),
                                Vec3 {
                                    x: 0.0,
                                    y: 0.0,
                                    z: 300.0,
                                },
                            )),
                            ..Default::default()
                        },
                    );
                }
                monster.entity.movement_flags -= 512;
                monster.flush_entity();
            }),
        ),
        ("hipgrem:gremlin_jump11", Arc::new(|monster| monster.delay(3.0))),
        (
            "hipgrem:gremlin_shot1",
            Arc::new(|monster| {
                monster.entity.effects |= 2;
                monster.flush_entity();
            }),
        ),
        (
            "hipgrem:gremlin_nail1",
            Arc::new(|monster| {
                monster.entity.effects |= 2;
                monster.flush_entity();
                gremlin_fire_nail(monster);
            }),
        ),
        (
            "hipgrem:gremlin_laser1",
            Arc::new(|monster| {
                monster.entity.effects |= 2;
                monster.flush_entity();
                gremlin_fire_laser(monster);
            }),
        ),
        ("hipgrem:gremlin_look1", Arc::new(|monster| monster.delay(0.2))),
        ("hipgrem:gremlin_look9", Arc::new(resume)),
        (
            "hipgrem:gremlin_glook20",
            Arc::new(|monster| {
                gremlin_drop_backpack(monster);
                number(monster, "stoleweapon", 0.0);
                resume(monster);
            }),
        ),
        (
            "hipgrem:gremlin_spawn1",
            Arc::new(|monster| {
                monster.delay(0.3);
                number(monster, "gremlin:pain-disabled", 1.0);
            }),
        ),
        ("hipgrem:gremlin_spawn2", Arc::new(|monster| monster.delay(0.3))),
        (
            "hipgrem:gremlin_spawn6",
            Arc::new(|monster| number(monster, "gremlin:pain-disabled", 0.0)),
        ),
        (
            "hipgrem:gremlin_flip1",
            Arc::new(|monster| {
                monster.face();
                let id = monster.entity.actor.id().clone();
                if let Ok(body) = monster.game.body(&id) {
                    let basis = monster.game.make_vectors(body.angles);
                    let _ = monster.game.set_body(
                        &id,
                        &BodyPatch {
                            origin: Some(vadd(monster.origin, Vec3 { x: 0.0, y: 0.0, z: 1.0 })),
                            velocity: Some(vsub(
                                Vec3 {
                                    x: 0.0,
                                    y: 0.0,
                                    z: 350.0,
                                },
                                vscale(basis.forward, 200.0),
                            )),
                            ..Default::default()
                        },
                    );
                }
                monster.entity.movement_flags &= !512;
                monster.flush_entity();
                let _ = monster.game.sound_simple(&id, "grem/death.wav");
            }),
        ),
        (
            "hipgrem:gremlin_flip6",
            Arc::new(|monster| {
                if let Ok(touch) = monster.game.named.touch("hipnotic:Gremlin_FlipTouch") {
                    monster.entity.touch = Some(touch);
                    monster.flush_entity();
                }
            }),
        ),
    ];
    for distance in [2, 4] {
        actions.push((
            leaked_name(format!("ai_back({distance})")),
            Arc::new(move |monster: &mut MissionMonster| {
                let id = monster.entity.actor.id().clone();
                let owned = monster.entity.actor.clone();
                let yaw = monster
                    .game
                    .body(&id)
                    .map(|body| f64::from(body.angles.y))
                    .unwrap_or(0.0);
                monster.game.host.walk_move(&owned, yaw + 180.0, distance as f64);
                monster.refresh();
            }),
        ));
    }
    for distance in [0, 8, 12, 16] {
        actions.push((
            leaked_name(format!("gremlin_run({distance})")),
            Arc::new(move |monster: &mut MissionMonster| gremlin_run(monster, distance as f64)),
        ));
    }
    actions.push(("gremlin_walk(8)", Arc::new(|monster| gremlin_walk(monster, 8.0))));
    for side in [0.0, 200.0] {
        actions.push((
            leaked_name(format!("Gremlin_Melee({side})")),
            Arc::new(move |monster: &mut MissionMonster| melee(monster, side)),
        ));
    }
    for side in [-200.0, 200.0] {
        actions.push((
            leaked_name(format!("Gremlin_Gorge({side})")),
            Arc::new(move |monster: &mut MissionMonster| gorge(monster, side)),
        ));
    }
    for offset in [-4, 4] {
        actions.push((
            leaked_name(format!("Gremlin_FireNailGun({offset})")),
            Arc::new(gremlin_fire_nail),
        ));
        actions.push((
            leaked_name(format!("Gremlin_FireLaserGun({offset})")),
            Arc::new(gremlin_fire_laser),
        ));
    }
    let spawn: MissionAction = Arc::new(|monster| {
        monster.runtime.authored_gremlins += 1;
        monster.entity.fields.insert("yaw_speed".to_string(), "40".to_string());
        monster.flush_entity();
        monster.spawn_default();
        monster.entity.max_health = 101.0;
        monster.flush_entity();
        let id = monster.entity.actor.id().clone();
        if !monster.game.host.inventory.has(&id) {
            let owned = monster.entity.actor.clone();
            let entries: Vec<InventoryEntry> = ["shells", "nails", "rockets", "cells"]
                .into_iter()
                .map(|ammo| InventoryEntry {
                    item: format!("q1:ammo/{ammo}"),
                    count: 0.0,
                    capacity: 1_000_000.0,
                    count_policy: Some(InventoryCountPolicy::SourceCounter(SourceCounterArithmetic::Binary32)),
                })
                .collect();
            let _ = monster.game.host.inventory.create(&owned, &entries);
        }
        monster.refresh();
    });
    let check_attack: MissionCheckAttack = Arc::new(|monster| {
        if monster.game.time < monster.state.attack_finished {
            return false;
        }
        if monster.distance() <= 90.0 && monster.entity.number("stoleweapon") == 0.0 {
            monster.entity.attack_state = Q1AttackState::Melee;
            monster.flush_entity();
            return true;
        }
        if monster.game.host.random() < 0.03 + monster.entity.number("stoleweapon") {
            monster.entity.attack_state = Q1AttackState::Missile;
            monster.flush_entity();
            return true;
        }
        false
    });
    let pain: MissionPain = Arc::new(|monster, attacker, _damage| {
        if monster.entity.number("gremlin:pain-disabled") != 0.0 {
            return;
        }
        if monster.game.host.random() < 0.8 {
            number(monster, "gorging", 0.0);
            monster.enemy = attacker.cloned();
            if let Some(attacker) = attacker {
                monster.found(attacker);
            }
        }
        if callback_name(monster.entity.touch.as_ref()).as_deref() == Some("hipnotic:Gremlin_JumpTouch")
            || monster.state.pain_finished > monster.game.time
        {
            return;
        }
        let time = monster.game.time;
        monster.state.pain_finished = time + 1.0;
        let rolled = monster.game.host.random();
        let id = monster.entity.actor.id().clone();
        let _ = monster.game.sound_simple(
            &id,
            if rolled < 0.33 {
                "grem/pain1.wav"
            } else if rolled < 0.66 {
                "grem/pain2.wav"
            } else {
                "grem/pain3.wav"
            },
        );
        monster.play(if monster.entity.number("stoleweapon") != 0.0 {
            "gremlin_gunpain1"
        } else {
            "gremlin_pain1"
        });
    });
    let die: MissionDie = Arc::new(|monster, attacker| {
        monster.sync();
        let id = monster.entity.actor.id().clone();
        if monster.game.host.inventory.entries(&id).iter().any(|entry| {
            entry.item.starts_with("q1:weapon/")
                && entry.item != "q1:weapon/axe"
                && entry.item != "q1:weapon/shotgun"
                && entry.item != "q1:weapon/hipnotic:mjolnir"
                && entry.count > 0.0
        }) {
            gremlin_drop_backpack(monster);
            number(monster, "stoleweapon", 0.0);
        }
        let facing = monster
            .game
            .body(&id)
            .map(|body| {
                let basis = monster.game.make_vectors(body.angles);
                let attacker_origin = attacker
                    .and_then(|attacker| monster.game.host.bodies.read(attacker))
                    .map(|body| body.origin)
                    .unwrap_or(ZERO);
                dot(normalize(vsub(attacker_origin, monster.origin)), basis.forward)
            })
            .unwrap_or(0.0);
        let health = monster.game.health(&id);
        if health < -35.0 {
            gib(monster, health);
            return;
        }
        if facing > 0.7 && monster.game.host.random() < 0.5 && monster.entity.movement_flags & 512 != 0 {
            monster.play("gremlin_flip1");
            return;
        }
        monster.play("gremlin_die1");
    });
    PackMonsterDefinition {
        spec: Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::Gremlin,
            kill_string: None,
            classnames: &["monster_gremlin"],
            model: "grem",
            head: Some("h_grem"),
            health: 100.0,
            gib_health: -35.0,
            gibs: &["gib1", "gib1", "gib1"],
            bounds: HULL_BOUNDS,
            stand: "gremlin_stand1",
            walk: "gremlin_walk1",
            run: "gremlin_run1",
            sight: "grem/sight1.wav",
            missile: Some("Gremlin_MissileAttack"),
            melee: true,
            movement: MonsterMovement::Walk,
        })),
        base_behavior: false,
        frames: FRAMES,
        actions,
        callbacks: vec![
            (
                "Gremlin_JumpTouch",
                Q1CallbackHandlers {
                    touch: Some(gremlin_jump_touch),
                    ..Default::default()
                },
            ),
            (
                "Gremlin_FlipTouch",
                Q1CallbackHandlers {
                    touch: Some(gremlin_flip_touch),
                    ..Default::default()
                },
            ),
        ],
        spawn: Some(spawn),
        start: None,
        pain,
        die,
        melee: Some(Arc::new(melee_attack)),
        check_attack: Some(check_attack),
        found: None,
        ai: None,
        use_: None,
    }
}

#[cfg(test)]
mod tests {
    use super::gremlin_definition;
    use crate::q1::missionpacks::monsters::runtime::Q1MissionPackMonsters;
    use crate::q1::missionpacks::monsters::types::MissionMonsterHooks;
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};

    #[test]
    fn gremlin_registers_with_jump_callbacks() {
        let mut game = test_game();
        let mut runtime =
            Q1MissionPackMonsters::new(&mut game, Q1MissionPack::Hipnotic, MissionMonsterHooks::default())
                .expect("runtime");
        let definition = gremlin_definition(&runtime);
        assert_eq!(definition.spec.classnames, &["monster_gremlin"]);
        assert_eq!(definition.spec.model, "grem");
        assert_eq!(definition.spec.health, 100.0);
        assert_eq!(definition.spec.missile, Some("Gremlin_MissileAttack"));
        assert!(!definition.frames.is_empty());
        assert_eq!(definition.actions.len(), 19 + 2 + 4 + 1 + 2 + 2 + 4);
        assert_eq!(definition.callbacks.len(), 2);
        runtime.register(&mut game, definition).expect("register");
        let id = game.create("monster_gremlin", None, None).expect("create");
        let monster = runtime.require(&mut game, &id).expect("require");
        assert_eq!(monster.definition.spec.model, "grem");
    }
}
