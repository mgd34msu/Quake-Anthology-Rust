//! Q1 grunt and rottweiler AI (`src/content/q1/foundation/monsters.ts`).
//!
//! Original `ai.qc`/`fight.qc` behavior for the two foundation
//! species: model-frame sequences, target acquisition, melee/missile
//! attacks, pain/death, gibbing, and patrols. Monster state lives on
//! the entity; frame handlers clone it, run the donor logic against
//! the game, then write it back, which keeps the donor's synchronous
//! recursion visible within one frame.
//!
//! `throw_gib`/`throw_head` are defined here (donor
//! `src/content/q1/base/projectiles.ts`) so the foundation does not
//! grow a second copy; the base projectile module re-exports them.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::contract::gib_impulse_coefficients;
use crate::monsters::monster_target_eligible;

use super::entity::{Q1AttackState, Q1Monster, Q1MonsterMode, Q1MonsterSpecies};
use super::entity_services::{Q1DamageParams, Q1EntityServices};
use super::types::{
    dot, length, normalize, vadd, vscale, vsub, yaw_for, Q1Effect, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel,
    Q1TraceRequest, POINT, ZERO,
};
use super::weapons::fire_bullets;
use crate::q1::{q1_error, Q1Error};

const ARMY_WALK: [f64; 24] = [
    1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 4.0, 4.0, 2.0, 2.0, 2.0, 1.0, 0.0, 1.0, 1.0, 1.0, 3.0, 3.0, 3.0, 3.0, 2.0, 1.0, 1.0,
    1.0,
];
const ARMY_RUN: [f64; 8] = [11.0, 15.0, 10.0, 10.0, 8.0, 15.0, 10.0, 8.0];
const DOG_RUN: [f64; 12] = [16.0, 32.0, 32.0, 20.0, 64.0, 32.0, 16.0, 32.0, 32.0, 20.0, 64.0, 32.0];

fn stationary(count: usize) -> Vec<f64> {
    vec![0.0; count]
}

fn set_sequence(monster: &mut Q1Monster, mode: Q1MonsterMode, first_frame: i32, sequence: Vec<f64>) {
    monster.mode = mode;
    monster.first_frame = first_frame;
    monster.sequence = sequence;
    monster.frame_index = 0;
}

fn stand(monster: &mut Q1Monster) {
    if monster.species == Q1MonsterSpecies::Army {
        set_sequence(monster, Q1MonsterMode::Stand, 0, stationary(8));
    } else {
        set_sequence(monster, Q1MonsterMode::Stand, 69, stationary(9));
    }
}

fn walk(monster: &mut Q1Monster) {
    if monster.species == Q1MonsterSpecies::Army {
        set_sequence(monster, Q1MonsterMode::Walk, 90, ARMY_WALK.to_vec());
    } else {
        set_sequence(monster, Q1MonsterMode::Walk, 78, vec![8.0; 8]);
    }
}

fn run(monster: &mut Q1Monster) {
    if monster.species == Q1MonsterSpecies::Army {
        set_sequence(monster, Q1MonsterMode::Run, 73, ARMY_RUN.to_vec());
    } else {
        set_sequence(monster, Q1MonsterMode::Run, 48, DOG_RUN.to_vec());
    }
}

/// Patrol-route end time (`pathEndTime`).
#[must_use]
pub fn path_end_time(time: f64) -> f64 {
    f64::from((f64::from(time as f32) + 999999.0) as f32)
}

/// Load a monster or fail with the donor message.
fn require_monster(game: &Q1EntityServices, id: &ActorId) -> Result<Q1Monster, Q1Error> {
    let entity = game.entity_ref(id).ok_or_else(|| q1_error("Missing Q1 entity"))?;
    entity
        .monster
        .clone()
        .ok_or_else(|| q1_error(format!("Missing saved monster state: {}", entity.classname)))
}

/// Run donor logic against cloned monster state, then write it back.
fn with_monster(
    game: &mut Q1EntityServices,
    id: &ActorId,
    run: impl FnOnce(&mut Q1EntityServices, &ActorId, &mut Q1Monster) -> Result<(), Q1Error>,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let mut monster = require_monster(game, &id)?;
    run(game, &id, &mut monster)?;
    game.update_entity(&id, |entity| entity.monster = Some(monster))
}

/// Retarget a patrolling monster (`setMonsterRoute`).
pub fn set_monster_route(
    game: &mut Q1EntityServices,
    id: &ActorId,
    goal: Option<&ActorId>,
    pause_until: f64,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let mut monster = require_monster(game, &id)?;
    monster.pause_until = pause_until;
    let prefix = game
        .entity_ref(&id)
        .and_then(|entity| entity.fields.get("source.monsterCallbackPrefix").cloned());
    if let Some(prefix) = prefix {
        game.invoke_action(&id, &format!("{prefix}:monster_route"))?;
    } else if goal.is_none() || pause_until > game.time {
        stand(&mut monster);
    } else if monster.mode == Q1MonsterMode::Stand {
        walk(&mut monster);
    }
    if let Some(target) = goal.and_then(|goal| game.host.bodies.read(goal)) {
        let origin = game.body(&id).map(|body| body.origin)?;
        let yaw = yaw_for(vsub(target.origin, origin));
        game.update_entity(&id, |entity| entity.ideal_yaw = yaw)?;
    }
    game.update_entity(&id, |entity| entity.monster = Some(monster))
}

fn face(game: &mut Q1EntityServices, id: &ActorId, destination: Vec3) -> Result<f64, Q1Error> {
    let id = id.clone();
    let body = game.body(&id)?;
    let ideal = yaw_for(vsub(destination, body.origin));
    let mut shift = ideal - f64::from(body.angles.y);
    if shift > 180.0 {
        shift -= 360.0;
    }
    if shift < -180.0 {
        shift += 360.0;
    }
    let entity = game.entity_ref(&id).cloned().expect("monster");
    let speed = entity.number("yaw_speed");
    let speed = if speed == 0.0 { 20.0 } else { speed };
    game.update_entity(&id, |entity| entity.ideal_yaw = ideal)?;
    let yaw = (f64::from(body.angles.y) + (-speed).max(speed.min(shift)) + 360.0) % 360.0;
    game.set_body(
        &id,
        &super::gameplay::BodyPatch {
            angles: Some(Vec3 {
                x: body.angles.x,
                y: yaw as f32,
                z: body.angles.z,
            }),
            ..Default::default()
        },
    )?;
    Ok(yaw)
}

fn visible(game: &mut Q1EntityServices, id: &ActorId, target: &ActorId) -> bool {
    let body = match game.host.bodies.read(target) {
        Some(body) => body,
        None => return false,
    };
    let observed = match game.monster_target(target) {
        Some(observed) => observed,
        None => return false,
    };
    let origin = match game.body(id) {
        Ok(body) => body.origin,
        Err(_) => return false,
    };
    let trace = game.host.trace(&Q1TraceRequest {
        start: vadd(
            origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 25.0,
            },
        ),
        end: vadd(
            body.origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: observed.view_height as f32,
            },
        ),
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: false,
        missile: false,
    });
    trace.fraction == 1.0 && !(trace.in_open && trace.in_water)
}

fn found_target(
    game: &mut Q1EntityServices,
    id: &ActorId,
    monster: &mut Q1Monster,
    target: &ActorId,
) -> Result<(), Q1Error> {
    monster.enemy = Some(target.clone());
    monster.search_until = game.time + 5.0;
    monster.attack_finished = game.time + 1.0;
    run(monster);
    if let Some(mission) = game.monster_missions.get_mut(id) {
        mission.found_target();
    }
    game.sight_entity = Some(id.clone());
    game.sight_time = game.time;
    let sight = if monster.species == Q1MonsterSpecies::Army {
        "soldier/sight1.wav"
    } else {
        "dog/dsight.wav"
    };
    game.sound_simple(id, sight)
}

fn find_target(game: &mut Q1EntityServices, id: &ActorId, monster: &mut Q1Monster) -> Result<bool, Q1Error> {
    let entity = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let candidate = if game.sight_entity.is_some() && game.sight_time >= game.time - 0.1 && (entity.spawnflags & 3) == 0
    {
        game.sight_entity
            .as_ref()
            .and_then(|sight| game.entity_ref(sight))
            .and_then(|sight| sight.monster.as_ref())
            .and_then(|sight| sight.enemy.clone())
    } else {
        game.host.check_client(&entity.actor)
    };
    let candidate = match candidate {
        Some(candidate) => candidate,
        None => return Ok(false),
    };
    if game.health(&candidate) <= 0.0 {
        return Ok(false);
    }
    let observed = match game.monster_target(&candidate) {
        Some(observed) => observed,
        None => return Ok(false),
    };
    if observed.invisible || observed.notarget {
        return Ok(false);
    }
    let target = match game.host.bodies.read(&candidate) {
        Some(target) => target,
        None => return Ok(false),
    };
    let origin = game.body(id).map(|body| body.origin)?;
    let delta = vsub(target.origin, origin);
    let distance = length(delta);
    if distance >= 1000.0 || !visible(game, id, &candidate) {
        return Ok(false);
    }
    let angles = game.body(id).map(|body| body.angles)?;
    let in_front = dot(normalize(delta), game.make_vectors(angles).forward) > 0.3;
    if distance >= 500.0 && !in_front
        || (120.0..500.0).contains(&distance)
            && observed.hostile_until.is_none_or(|until| until < game.time)
            && !in_front
    {
        return Ok(false);
    }
    found_target(game, id, monster, &candidate)?;
    Ok(true)
}

fn try_attack(game: &mut Q1EntityServices, id: &ActorId, monster: &mut Q1Monster) -> Result<bool, Q1Error> {
    let enemy = match monster.enemy.clone() {
        Some(enemy) => enemy,
        None => return Ok(false),
    };
    let target = match game.host.bodies.read(&enemy) {
        Some(target) => target,
        None => return Ok(false),
    };
    let body = game.body(id)?;
    let delta = vsub(target.origin, body.origin);
    let distance = length(delta);
    if monster.species == Q1MonsterSpecies::Dog {
        if distance < 120.0 {
            game.update_entity(id, |entity| entity.attack_state = Q1AttackState::Melee)?;
            return Ok(true);
        }
        let vertical = body.origin.z + body.bounds.min.z
            <= target.origin.z + target.bounds.min.z + (target.bounds.max.z - target.bounds.min.z) * 0.75
            && body.origin.z + body.bounds.max.z
                >= target.origin.z + target.bounds.min.z + (target.bounds.max.z - target.bounds.min.z) * 0.25;
        let horizontal = delta.x.hypot(delta.y);
        if vertical && (80.0..=150.0).contains(&horizontal) {
            game.update_entity(id, |entity| entity.attack_state = Q1AttackState::Missile)?;
            return Ok(true);
        }
        return Ok(false);
    }
    if game.time < monster.attack_finished || distance >= 1000.0 {
        return Ok(false);
    }
    let observed = match game.monster_target(&enemy) {
        Some(observed) => observed,
        None => return Ok(false),
    };
    let trace = game.host.trace(&Q1TraceRequest {
        start: vadd(
            body.origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 25.0,
            },
        ),
        end: vadd(
            target.origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: observed.view_height as f32,
            },
        ),
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: true,
        missile: false,
    });
    if !trace.actor.as_ref().is_some_and(|actor| same_actor(actor, &enemy)) || trace.in_open && trace.in_water {
        return Ok(false);
    }
    let chance = if distance < 120.0 {
        0.9
    } else if distance < 500.0 {
        0.4
    } else {
        0.05
    };
    if game.host.random() >= chance {
        return Ok(false);
    }
    set_sequence(monster, Q1MonsterMode::Attack, 81, stationary(9));
    monster.attack_finished = game.time + 1.0 + game.host.random();
    monster.refired = false;
    game.host.random();
    Ok(true)
}

fn monster_frame_inner(game: &mut Q1EntityServices, id: &ActorId, monster: &mut Q1Monster) -> Result<(), Q1Error> {
    let id = id.clone();
    let mode = monster.mode;
    let index = monster.frame_index;
    let distance = monster.sequence.get(index).copied().unwrap_or(0.0);
    game.update_entity(&id, |entity| entity.frame = monster.first_frame + index as i32)?;
    if (mode == Q1MonsterMode::Walk || mode == Q1MonsterMode::Run) && index == 0 && game.host.random() < 0.2 {
        let idle = if monster.species == Q1MonsterSpecies::Army {
            "soldier/idle.wav"
        } else {
            "dog/idle.wav"
        };
        game.sound(&id, idle, Q1SoundChannel::Voice, 2.0, 1.0)?;
    }
    if mode == Q1MonsterMode::Death {
        if monster.species == Q1MonsterSpecies::Army && index == 2 && !monster.death_drop {
            game.update_entity(&id, |entity| entity.solid = Q1Solid::None)?;
            game.link(&id)?;
            let origin = game.body(&id).map(|body| body.origin)?;
            game.drop_shells(origin)?;
            monster.death_drop = true;
        }
        if distance != 0.0 {
            let owned = game
                .entity_ref(&id)
                .map(|entity| entity.actor.clone())
                .expect("monster");
            let yaw = game.body(&id).map(|body| body.angles.y)? as f64;
            game.host.walk_move(&owned, yaw, distance);
        }
    } else if mode == Q1MonsterMode::Pain {
        if distance != 0.0 {
            let owned = game
                .entity_ref(&id)
                .map(|entity| entity.actor.clone())
                .expect("monster");
            let yaw = game.body(&id).map(|body| body.angles.y)? as f64;
            game.host.walk_move(&owned, yaw, distance);
        }
    } else if mode == Q1MonsterMode::Stand || mode == Q1MonsterMode::Walk {
        if find_target(game, &id, monster)? {
            return game.schedule(&id, 0.1, "monster_frame");
        }
        if mode == Q1MonsterMode::Stand
            && game.time >= monster.pause_until
            && game
                .monster_missions
                .get(&id)
                .is_some_and(|mission| mission.route().is_some())
        {
            walk(monster);
        }
        if mode == Q1MonsterMode::Walk && game.time >= monster.pause_until {
            let target = match game.monster_missions.get(&id) {
                Some(mission) => mission.route(),
                None => game
                    .find(&monster.path)
                    .first()
                    .and_then(|target| game.entity_ref(target))
                    .map(|entity| entity.actor.id().clone()),
            };
            match target {
                None => stand(monster),
                Some(target) => {
                    let owned = game
                        .entity_ref(&id)
                        .map(|entity| entity.actor.clone())
                        .expect("monster");
                    game.host.move_to_goal(&owned, &target, distance, None);
                }
            }
        }
    } else {
        let enemy = monster.enemy.clone();
        let eligible = match enemy.as_ref() {
            Some(enemy) => {
                let observed = game.monster_target(enemy);
                monster_target_eligible(game.health(enemy), observed.as_ref())
            }
            None => false,
        };
        if !eligible {
            game.update_entity(&id, |entity| entity.attack_state = Q1AttackState::Straight)?;
            let old_enemy = monster.old_enemy.clone();
            let old_eligible = match old_enemy.as_ref() {
                Some(old) => {
                    let observed = game.monster_target(old);
                    monster_target_eligible(game.health(old), observed.as_ref())
                }
                None => false,
            };
            if old_eligible {
                monster.enemy = old_enemy;
                monster.old_enemy = None;
                run(monster);
                return monster_frame_inner(game, &id, monster);
            }
            monster.enemy = None;
            monster.old_enemy = None;
            if game
                .monster_missions
                .get(&id)
                .is_some_and(|mission| mission.route().is_some())
                || !monster.path.is_empty()
            {
                walk(monster);
            } else {
                stand(monster);
            }
            return game.schedule(&id, 0.1, "monster_frame");
        }
        let enemy = enemy.expect("eligible enemy");
        let target = match game.host.bodies.read(&enemy) {
            Some(target) => target,
            None => return Ok(()),
        };
        if mode == Q1MonsterMode::Run {
            let combat_route = game.monster_missions.get(&id).map(|mission| mission.combat_route());
            if let Some(goal) = combat_route.as_ref().and_then(|route| route.goal.clone()) {
                let owned = game
                    .entity_ref(&id)
                    .map(|entity| entity.actor.clone())
                    .expect("monster");
                game.host
                    .move_to_goal(&owned, &goal, distance, Some(super::host::Q1GoalMode::Contact));
                monster.frame_index = (monster.frame_index + 1) % monster.sequence.len();
                return game.schedule(&id, 0.1, "monster_frame");
            }
            let seen = visible(game, &id, &enemy);
            if seen {
                monster.search_until = game.time + 5.0;
            }
            if game.options().coop && monster.search_until < game.time && find_target(game, &id, monster)? {
                return game.schedule(&id, 0.1, "monster_frame");
            }
            let attack_state = game
                .entity_ref(&id)
                .map(|entity| entity.attack_state)
                .unwrap_or(Q1AttackState::Straight);
            if attack_state != Q1AttackState::Straight {
                let yaw = face(game, &id, target.origin)?;
                let self_origin = game.body(&id).map(|body| body.origin)?;
                let ideal = yaw_for(vsub(target.origin, self_origin));
                let delta = (yaw - ideal + 360.0) % 360.0;
                if delta <= 45.0 || delta >= 315.0 {
                    if attack_state == Q1AttackState::Melee {
                        set_sequence(monster, Q1MonsterMode::Attack, 0, vec![10.0; 8]);
                    } else {
                        set_sequence(monster, Q1MonsterMode::Leap, 60, stationary(9));
                    }
                    game.update_entity(&id, |entity| {
                        entity.attack_state = Q1AttackState::Straight;
                    })?;
                    return monster_frame_inner(game, &id, monster);
                }
                monster.frame_index = (monster.frame_index + 1) % monster.sequence.len();
                return game.schedule(&id, 0.1, "monster_frame");
            }
            if seen && try_attack(game, &id, monster)? {
                if monster.mode != mode {
                    return monster_frame_inner(game, &id, monster);
                }
                monster.frame_index = (monster.frame_index + 1) % monster.sequence.len();
                return game.schedule(&id, 0.1, "monster_frame");
            }
            if !combat_route.is_some_and(|route| route.stand_ground) {
                let owned = game
                    .entity_ref(&id)
                    .map(|entity| entity.actor.clone())
                    .expect("monster");
                game.host.move_to_goal(&owned, &enemy, distance, None);
            }
        } else if mode == Q1MonsterMode::Attack {
            face(game, &id, target.origin)?;
            if monster.species == Q1MonsterSpecies::Army {
                if index == 4 {
                    game.sound(&id, "soldier/sattck1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
                    let origin = game.body(&id).map(|body| body.origin)?;
                    let direction = normalize(vsub(vsub(target.origin, vscale(target.velocity, 0.2)), origin));
                    let view_angles = game
                        .entity_ref(&id)
                        .map(|entity| entity.vector("v_angle"))
                        .unwrap_or(ZERO);
                    let owned = game
                        .entity_ref(&id)
                        .map(|entity| entity.actor.clone())
                        .expect("monster");
                    fire_bullets(game, &owned, direction, view_angles, 4, 0.1, 0.1, None);
                    let flash = game.body(&id).map(|body| body.origin)?;
                    game.effect(Q1Effect::Muzzleflash, flash, Some(&id), 1);
                }
                if index == 6 && game.options().skill == 3 && !monster.refired && visible(game, &id, &enemy) {
                    monster.refired = true;
                    monster.frame_index = 0;
                    return game.schedule(&id, 0.1, "monster_frame");
                }
            } else {
                let owned = game
                    .entity_ref(&id)
                    .map(|entity| entity.actor.clone())
                    .expect("monster");
                game.host.move_to_goal(&owned, &enemy, distance, None);
                if index == 3 {
                    game.sound_simple(&id, "dog/dattack1.wav")?;
                    let self_origin = game.body(&id).map(|body| body.origin)?;
                    if game.can_damage(&enemy, &id) && length(vsub(target.origin, self_origin)) <= 100.0 {
                        let amount = (game.host.random() + game.host.random() + game.host.random()) * 8.0;
                        game.damage(&enemy, Some(&id), Some(&id), amount, &Q1DamageParams::default());
                    }
                }
            }
        } else if mode == Q1MonsterMode::Leap {
            if index < 2 {
                face(game, &id, target.origin)?;
            }
            if index == 1 {
                let body = game.body(&id)?;
                game.update_entity(&id, |entity| entity.movement = Q1MoveType::Toss)?;
                let forward = game.make_vectors(body.angles).forward;
                game.set_body(
                    &id,
                    &super::gameplay::BodyPatch {
                        origin: Some(vadd(body.origin, Vec3 { x: 0.0, y: 0.0, z: 1.0 })),
                        velocity: Some(vadd(
                            vscale(forward, 300.0),
                            Vec3 {
                                x: 0.0,
                                y: 0.0,
                                z: 200.0,
                            },
                        )),
                        ground: Some(None),
                        ..Default::default()
                    },
                )?;
                let touch = game.named.touch("Dog_JumpTouch")?;
                game.update_entity(&id, |entity| entity.touch = Some(touch))?;
            }
        }
    }
    if monster.mode == mode {
        if index + 1 < monster.sequence.len() {
            monster.frame_index += 1;
        } else if mode == Q1MonsterMode::Death {
            return Ok(());
        } else if mode == Q1MonsterMode::Leap {
            monster.frame_index = monster.sequence.len() - 1;
        } else if mode == Q1MonsterMode::Attack || mode == Q1MonsterMode::Pain {
            run(monster);
        } else {
            monster.frame_index = 0;
        }
    }
    game.schedule(&id, 0.1, "monster_frame")
}

fn gib(game: &mut Q1EntityServices, id: &ActorId, monster: &Q1Monster) -> Result<(), Q1Error> {
    let id = id.clone();
    let health = game.health(&id);
    game.sound_simple(&id, "player/udeath.wav")?;
    if monster.species == Q1MonsterSpecies::Army {
        throw_head(game, &id, "h_guard", health)?;
    }
    let models = if monster.species == Q1MonsterSpecies::Army {
        ["gib1", "gib2", "gib3"]
    } else {
        ["gib3", "gib3", "gib3"]
    };
    for model in models {
        let origin = game.body(&id).map(|body| body.origin)?;
        throw_gib(game, origin, model, health)?;
    }
    if monster.species == Q1MonsterSpecies::Dog {
        throw_head(game, &id, "h_dog", health)?;
    }
    Ok(())
}

/// Spawn a grunt or rottweiler (`spawnMonster`).
pub fn spawn_monster(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let entity = game
        .entity_ref(&id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let species = if entity.classname == "monster_army" {
        Q1MonsterSpecies::Army
    } else {
        Q1MonsterSpecies::Dog
    };
    if game.uses_id1_precaches() {
        if game.options().deathmatch != 0 {
            return game.remove(&id);
        }
        if species == Q1MonsterSpecies::Army {
            for model in [
                "progs/soldier.mdl",
                "progs/h_guard.mdl",
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
            ] {
                game.precache_model(model)?;
            }
            for sound in [
                "soldier/death1.wav",
                "soldier/idle.wav",
                "soldier/pain1.wav",
                "soldier/pain2.wav",
                "soldier/sattck1.wav",
                "soldier/sight1.wav",
                "player/udeath.wav",
            ] {
                game.precache_sound(sound)?;
            }
        } else {
            for model in ["progs/h_dog.mdl", "progs/dog.mdl"] {
                game.precache_model(model)?;
            }
            for sound in [
                "dog/dattack1.wav",
                "dog/ddeath.wav",
                "dog/dpain1.wav",
                "dog/dsight.wav",
                "dog/idle.wav",
            ] {
                game.precache_sound(sound)?;
            }
        }
    }
    let mut monster = Q1Monster {
        species,
        mode: Q1MonsterMode::Stand,
        frame_index: 0,
        sequence: Vec::new(),
        first_frame: 0,
        enemy: None,
        old_enemy: None,
        path: entity.target.clone(),
        pause_until: 0.0,
        attack_finished: 0.0,
        pain_finished: 0.0,
        search_until: 0.0,
        death_drop: false,
        refired: false,
    };
    stand(&mut monster);
    let model = if species == Q1MonsterSpecies::Army {
        "progs/soldier.mdl"
    } else {
        "progs/dog.mdl"
    };
    let max_health = if species == Q1MonsterSpecies::Army { 30.0 } else { 25.0 };
    let use_callback = game.named.use_callback("monster_use")?;
    let pain = game.named.pain("monster_pain")?;
    let die = game.named.die("monster_die")?;
    let path_end = game.named.action("monster_path_end")?;
    game.update_entity(&id, |entity| {
        entity.monster = Some(monster);
        entity.solid = Q1Solid::Slidebox;
        entity.movement = Q1MoveType::Step;
        entity.aimed_damage = true;
        entity.path_end = Some(path_end);
        entity.model = String::from(model);
        entity.max_health = max_health;
        entity.use_callback = Some(use_callback);
        entity.pain = Some(pain);
        entity.die = Some(die);
    })?;
    game.set_health(&id, max_health)?;
    let half = if species == Q1MonsterSpecies::Army { 16.0 } else { 32.0 };
    game.set_bounds(
        &id,
        Bounds {
            min: Vec3 {
                x: -half,
                y: -half,
                z: -24.0,
            },
            max: Vec3 {
                x: half,
                y: half,
                z: 40.0,
            },
        },
    )?;
    if let Some(mission) = game.monster_missions.get_mut(&id) {
        mission.spawned();
    } else {
        game.total_monsters += 1;
    }
    let delay = 0.1 + game.host.random() * 0.5;
    game.schedule(&id, delay, "walkmonster_start_go")
}

fn dog_leap_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    monster: &mut Q1Monster,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let other = other.clone();
    if game.health(&id) <= 0.0 {
        return Ok(());
    }
    let velocity = game.body(&id).map(|body| body.velocity)?;
    if game
        .host
        .combat
        .read(&other)
        .is_some_and(|combat| combat.can_take_damage)
        && length(velocity) > 300.0
    {
        let amount = 10.0 + 10.0 * game.host.random();
        game.damage(&other, Some(&id), Some(&id), amount, &Q1DamageParams::default());
    }
    if game.host.check_bottom(&id) {
        game.update_entity(&id, |entity| {
            entity.touch = None;
            entity.movement = Q1MoveType::Step;
        })?;
        run(monster);
        game.schedule(&id, 0.1, "monster_frame")?;
    }
    Ok(())
}

fn retaliate(
    game: &mut Q1EntityServices,
    id: &ActorId,
    monster: &mut Q1Monster,
    attacker: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let attacker = attacker.cloned();
    let entity = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if attacker.is_none()
        || attacker.as_ref().is_some_and(|attacker| same_actor(attacker, id))
        || attacker.as_ref().is_some_and(|attacker| {
            game.entity_ref(attacker)
                .is_some_and(|actor| actor.classname == entity.classname)
        }) && monster.species != Q1MonsterSpecies::Army
    {
        return Ok(());
    }
    let attacker = attacker.expect("attacker");
    if monster.enemy.as_ref().is_some_and(|enemy| game.is_player(enemy)) {
        monster.old_enemy = monster.enemy.clone();
    }
    if monster.enemy.is_none()
        || monster
            .enemy
            .as_ref()
            .is_some_and(|enemy| !same_actor(enemy, &attacker))
    {
        found_target(game, id, monster, &attacker)?;
    }
    Ok(())
}

fn monster_use(game: &mut Q1EntityServices, id: &ActorId, activator: Option<&ActorId>) -> Result<(), Q1Error> {
    if game
        .monster_missions
        .get_mut(id)
        .is_some_and(|mission| mission.r#use(activator))
    {
        return Ok(());
    }
    with_monster(game, id, |game, id, monster| {
        let id = id.clone();
        let activator = activator.cloned();
        if monster.enemy.is_some()
            || game.health(&id) <= 0.0
            || !activator.as_ref().is_some_and(|activator| game.is_player(activator))
            || activator.is_none()
        {
            return Ok(());
        }
        let activator = activator.expect("player activator");
        let observed = match game.monster_target(&activator) {
            Some(observed) => observed,
            None => return Ok(()),
        };
        if observed.invisible || observed.notarget {
            return Ok(());
        }
        monster.enemy = Some(activator);
        game.schedule(&id, 0.1, "monster_found_target")
    })
}

fn monster_pain(
    game: &mut Q1EntityServices,
    id: &ActorId,
    attacker: Option<&ActorId>,
    monster: &mut Q1Monster,
) -> Result<(), Q1Error> {
    retaliate(game, id, monster, attacker)?;
    if monster.species == Q1MonsterSpecies::Army {
        if monster.pain_finished > game.time {
            return Ok(());
        }
        let random = game.host.random();
        monster.pain_finished = game.time + if random < 0.2 { 0.6 } else { 1.1 };
        if random < 0.2 {
            set_sequence(monster, Q1MonsterMode::Pain, 40, vec![0.0, 0.0, 0.0, 0.0, 0.0, -1.0]);
        } else if random < 0.6 {
            set_sequence(
                monster,
                Q1MonsterMode::Pain,
                46,
                vec![0.0, 13.0, 9.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -2.0, 0.0, 0.0],
            );
        } else {
            set_sequence(
                monster,
                Q1MonsterMode::Pain,
                60,
                vec![0.0, -1.0, 0.0, 0.0, 1.0, 1.0, 0.0, -1.0, 4.0, 3.0, 6.0, 8.0, 0.0],
            );
        }
        game.sound_simple(
            id,
            if random < 0.2 {
                "soldier/pain1.wav"
            } else {
                "soldier/pain2.wav"
            },
        )?;
        if game.options().skill == 3 {
            monster.pain_finished = game.time + 5.0;
        }
    } else {
        game.sound_simple(id, "dog/dpain1.wav")?;
        if game.host.random() > 0.5 {
            set_sequence(monster, Q1MonsterMode::Pain, 26, stationary(6));
        } else {
            set_sequence(
                monster,
                Q1MonsterMode::Pain,
                32,
                vec![
                    0.0, 0.0, -4.0, -12.0, -12.0, -2.0, 0.0, -4.0, 0.0, -10.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                ],
            );
        }
    }
    monster_frame_inner(game, id, monster)
}

fn monster_die(
    game: &mut Q1EntityServices,
    id: &ActorId,
    attacker: Option<&ActorId>,
    monster: &mut Q1Monster,
) -> Result<(), Q1Error> {
    let id = id.clone();
    let attacker = attacker.cloned();
    let species = monster.species;
    game.update_entity(&id, |entity| {
        entity.touch = None;
    })?;
    game.set_damageable(&id, false)?;
    if game.monster_missions.get_mut(&id).is_none() {
        game.killed_monsters += 1;
        game.host.emit(Q1Event::MonsterKilled {
            actor: id.clone(),
            total: game.total_monsters,
            found: game.killed_monsters,
        });
        let cause = monster.enemy.clone().or(attacker.clone());
        game.use_targets(&id, cause.as_ref())?;
    } else if let Some(mission) = game.monster_missions.get_mut(&id) {
        let cause = monster.enemy.clone().or(attacker.clone());
        mission.killed(cause.as_ref());
    }
    if game.health(&id) < -35.0 {
        return gib(game, &id, monster);
    }
    if species == Q1MonsterSpecies::Dog {
        game.update_entity(&id, |entity| entity.solid = Q1Solid::None)?;
        game.sound_simple(&id, "dog/ddeath.wav")?;
        set_sequence(
            monster,
            Q1MonsterMode::Death,
            if game.host.random() > 0.5 { 8 } else { 17 },
            stationary(9),
        );
    } else {
        game.sound_simple(&id, "soldier/death1.wav")?;
        if game.host.random() < 0.5 {
            set_sequence(monster, Q1MonsterMode::Death, 8, stationary(10));
        } else {
            set_sequence(
                monster,
                Q1MonsterMode::Death,
                18,
                vec![0.0, -5.0, -4.0, -13.0, -3.0, -4.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            );
        }
    }
    game.link(&id)?;
    monster_frame_inner(game, &id, monster)
}

fn monster_start(game: &mut Q1EntityServices, id: &ActorId, monster: &mut Q1Monster) -> Result<(), Q1Error> {
    let id = id.clone();
    let body = game.body(&id)?;
    let start = vadd(body.origin, Vec3 { x: 0.0, y: 0.0, z: 1.0 });
    game.set_body(
        &id,
        &super::gameplay::BodyPatch {
            origin: Some(start),
            ..Default::default()
        },
    )?;
    let floor = game.host.trace(&Q1TraceRequest {
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
    if floor.fraction < 1.0 && !floor.all_solid {
        game.set_body(
            &id,
            &super::gameplay::BodyPatch {
                origin: Some(floor.end),
                ground: Some(floor.actor.clone()),
                ..Default::default()
            },
        )?;
        game.update_entity(&id, |entity| entity.movement_flags |= 512)?;
    }
    let owned = game
        .entity_ref(&id)
        .map(|entity| entity.actor.clone())
        .expect("monster");
    game.host.walk_move(&owned, 0.0, 0.0);
    game.update_entity(&id, |entity| entity.movement_flags |= 32)?;
    let yaw_speed = game
        .entity_ref(&id)
        .map(|entity| entity.number("yaw_speed"))
        .unwrap_or(0.0);
    game.update_entity(&id, |entity| {
        entity.ideal_yaw = f64::from(body.angles.y);
        entity.yaw_speed = if yaw_speed == 0.0 { 20.0 } else { yaw_speed };
    })?;
    game.set_damageable(&id, true)?;
    game.link(&id)?;
    let patrol = if !game.monster_missions.contains_key(&id) {
        !monster.path.is_empty()
            && game
                .find(&monster.path)
                .first()
                .and_then(|corner| game.entity_ref(corner))
                .is_some_and(|corner| corner.classname == "path_corner")
    } else {
        game.monster_missions
            .get(&id)
            .is_some_and(|mission| mission.route().is_some())
    };
    if patrol {
        walk(monster);
    }
    if let Some(mission) = game.monster_missions.get_mut(&id) {
        mission.started();
    }
    let delay = 0.1 + game.host.random() * 0.5;
    game.schedule(&id, delay, "monster_frame")
}

/// Register monster callbacks.
pub fn register_monster_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "monster_frame",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| with_monster(game, id, monster_frame_inner)),
            ..Default::default()
        },
    )?;
    game.named.register(
        "monster_path_end",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                with_monster(game, id, |game, id, monster| {
                    monster.pause_until = path_end_time(game.time);
                    stand(monster);
                    let first_frame = monster.first_frame;
                    game.update_entity(id, |entity| entity.frame = first_frame)
                })
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "monster_found_target",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| {
                with_monster(game, id, |game, id, monster| {
                    let enemy = match monster.enemy.clone() {
                        Some(enemy) => enemy,
                        None => return Ok(()),
                    };
                    found_target(game, id, monster, &enemy)?;
                    monster_frame_inner(game, id, monster)
                })
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "monster_use",
        super::callbacks::Q1CallbackHandlers {
            use_callback: Some(
                |game: &mut Q1EntityServices, id: &ActorId, _other: Option<&ActorId>, activator: Option<&ActorId>| {
                    monster_use(game, id, activator)
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "monster_pain",
        super::callbacks::Q1CallbackHandlers {
            pain: Some(
                |game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>, _damage: f64| {
                    let attacker = attacker.cloned();
                    with_monster(game, id, |game, id, monster| {
                        monster_pain(game, id, attacker.as_ref(), monster)
                    })
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "monster_die",
        super::callbacks::Q1CallbackHandlers {
            die: Some(
                |game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>| {
                    let attacker = attacker.cloned();
                    with_monster(game, id, |game, id, monster| {
                        monster_die(game, id, attacker.as_ref(), monster)
                    })
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "Dog_JumpTouch",
        super::callbacks::Q1CallbackHandlers {
            touch: Some(
                |game: &mut Q1EntityServices,
                 id: &ActorId,
                 other: &ActorId,
                 _normal: Option<Vec3>,
                 _surface: Option<&super::gameplay::TouchSurface>| {
                    let other = other.clone();
                    with_monster(game, id, |game, id, monster| dog_leap_touch(game, id, &other, monster))
                },
            ),
            ..Default::default()
        },
    )?;
    game.named.register(
        "walkmonster_start_go",
        super::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| with_monster(game, id, monster_start)),
            ..Default::default()
        },
    )?;
    Ok(())
}

fn damage_velocity(game: &mut Q1EntityServices, damage: f64) -> Vec3 {
    let scale = if damage > -50.0 {
        0.7
    } else if damage > -200.0 {
        2.0
    } else {
        10.0
    };
    let random_x = game.host.random();
    let random_y = game.host.random();
    let random_z = game.host.random();
    let coefficients = gib_impulse_coefficients(random_x, random_y, random_z);
    vscale(
        Vec3 {
            x: coefficients[0] as f32,
            y: coefficients[1] as f32,
            z: coefficients[2] as f32,
        },
        scale,
    )
}

/// Throw a bouncing gib (`throwGib`, donor `base/projectiles.ts`).
pub fn throw_gib(game: &mut Q1EntityServices, origin: Vec3, model: &str, damage: f64) -> Result<ActorId, Q1Error> {
    let gib = game.create("gib", None, None)?;
    game.update_entity(&gib, |entity| {
        entity.model = format!("progs/{model}.mdl");
        entity.movement = Q1MoveType::Bounce;
    })?;
    let velocity = damage_velocity(game, damage);
    game.set_body(
        &gib,
        &super::gameplay::BodyPatch {
            origin: Some(origin),
            velocity: Some(velocity),
            bounds: Some(POINT),
            ..Default::default()
        },
    )?;
    let spin = Vec3 {
        x: (game.host.random() * 600.0) as f32,
        y: (game.host.random() * 600.0) as f32,
        z: (game.host.random() * 600.0) as f32,
    };
    game.update_entity(&gib, |entity| entity.angular_velocity = spin)?;
    let lifetime = 10.0 + game.host.random() * 10.0;
    game.schedule(&gib, lifetime, "SUB_Remove")?;
    game.link(&gib)?;
    Ok(gib)
}

/// Turn a dying entity into a bouncing head (`throwHead`, donor
/// `base/projectiles.ts`).
pub fn throw_head(game: &mut Q1EntityServices, id: &ActorId, model: &str, damage: f64) -> Result<(), Q1Error> {
    let id = id.clone();
    game.cancel(&id);
    game.update_entity(&id, |entity| {
        entity.model = format!("progs/{model}.mdl");
        entity.frame = 0;
        entity.movement = Q1MoveType::Bounce;
        entity.solid = Q1Solid::None;
    })?;
    game.set_damageable(&id, false)?;
    let origin = game.body(&id).map(|body| body.origin)?;
    let velocity = damage_velocity(game, damage);
    game.set_body(
        &id,
        &super::gameplay::BodyPatch {
            origin: Some(vadd(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: -24.0,
                },
            )),
            velocity: Some(velocity),
            bounds: Some(Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: 0.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 56.0,
                },
            }),
            ground: Some(None),
            ..Default::default()
        },
    )?;
    let spin = (game.host.random() * 2.0 - 1.0) * 600.0;
    game.update_entity(&id, |entity| {
        entity.angular_velocity = Vec3 {
            x: 0.0,
            y: spin as f32,
            z: 0.0,
        };
    })?;
    game.link(&id)
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
    fn path_end_time_rounds_like_fround() {
        assert_eq!(path_end_time(1.5), f64::from((f64::from(1.5f32) + 999999.0) as f32));
        assert!(path_end_time(0.0) > 999998.0);
    }

    #[test]
    fn sequences_match_donor_tables() {
        assert_eq!(ARMY_WALK.len(), 24);
        assert_eq!(ARMY_RUN, [11.0, 15.0, 10.0, 10.0, 8.0, 15.0, 10.0, 8.0]);
        assert_eq!(DOG_RUN.len(), 12);
        assert_eq!(DOG_RUN[0..4].to_vec(), vec![16.0, 32.0, 32.0, 20.0]);
    }

    #[test]
    fn spawn_monster_sets_species_tables() {
        let (mut game, _) = game();
        let army = game.create("monster_army", None, None).expect("army");
        spawn_monster(&mut game, &army).expect("spawn");
        let entity = game.entity_ref(&army).cloned().expect("army");
        assert_eq!(entity.model, "progs/soldier.mdl");
        assert_eq!(entity.max_health, 30.0);
        assert_eq!(game.health(&army), 30.0);
        let monster = entity.monster.expect("monster");
        assert_eq!(monster.species, Q1MonsterSpecies::Army);
        assert_eq!(monster.mode, Q1MonsterMode::Stand);
        assert_eq!((monster.first_frame, monster.sequence.len()), (0, 8));
        let dog = game.create("monster_dog", None, None).expect("dog");
        spawn_monster(&mut game, &dog).expect("spawn");
        let entity = game.entity_ref(&dog).cloned().expect("dog");
        assert_eq!(entity.model, "progs/dog.mdl");
        assert_eq!(entity.max_health, 25.0);
        let monster = entity.monster.expect("monster");
        assert_eq!(monster.species, Q1MonsterSpecies::Dog);
        assert_eq!((monster.first_frame, monster.sequence.len()), (69, 9));
        assert_eq!(game.total_monsters, 2);
    }

    #[test]
    fn set_monster_route_stands_without_goal() {
        let (mut game, _) = game();
        let army = game.create("monster_army", None, None).expect("army");
        spawn_monster(&mut game, &army).expect("spawn");
        set_monster_route(&mut game, &army, None, 0.0).expect("route");
        let monster = game
            .entity_ref(&army)
            .and_then(|entity| entity.monster.clone())
            .expect("monster");
        assert_eq!(monster.mode, Q1MonsterMode::Stand);
        let goal = game.create("info_notnull", None, None).expect("goal");
        set_monster_route(&mut game, &army, Some(&goal), 0.0).expect("route");
        let monster = game
            .entity_ref(&army)
            .and_then(|entity| entity.monster.clone())
            .expect("monster");
        assert_eq!(monster.mode, Q1MonsterMode::Walk);
    }
}
