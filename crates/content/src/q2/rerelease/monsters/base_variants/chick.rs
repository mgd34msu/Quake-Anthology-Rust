//! Rerelease chick (`src/content/q2/rerelease/monsters/base-variants/chick.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::math::{add3, length3, normalize3, scale3, sub3, vec3, Vec3};

use super::super::common::{blocked_check_platform, check_gib, monster_flash, predict_aim, reacts_to_pain};
use super::super::tables::chick::chick_moves;
use crate::q2::base::monsters::chick::chick_definition;
use crate::q2::base::monsters::common::alive_enemy;
use crate::q2::foundation::host::{Q2Edition, Q2Solid, Q2TraceRequest};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, clear_shot, corpse, enemy_body, finish_dodge, health, project_flash, set_duck, target_distance,
    vector_angles, visible,
};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::missionpacks::monsters::types::mission_weapons;
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction, TraceHit};

/// Run (`run`).
fn rerelease_chick_run(context: &mut MonsterContext) {
    finish_dodge(context);
    let current = context.state().current_move.name.clone();
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "chick_move_stand"
        } else if current == "chick_move_walk" || current == "chick_move_start_run" {
            "chick_move_run"
        } else {
            "chick_move_start_run"
        },
        true,
    );
}

/// Rocket (`rocket`).
fn chick_rocket(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, 57), None);
    let right = angles_vectors(context.game.body_of(actor.clone()).angles).right;
    let manual = context.state().manual_steering;
    let target = if manual {
        context.state().blind_fire_target
    } else {
        enemy.origin
    };
    let heat = context.game.require_entity(&actor).skin > 1;
    let speed = if heat { 500.0 } else { 650.0 };
    let mut point = target;
    if !manual {
        let enemy_id = context.entity().enemy.clone();
        let view_height = enemy_id
            .as_ref()
            .and_then(|enemy| context.game.entity(enemy))
            .map(|target| target.view_height)
            .unwrap_or(22);
        if context.game.random() < 0.33 || start.z < enemy.origin.z + enemy.bounds.min.z {
            point = vec3(target.x, target.y, target.z + view_height as f32);
        } else {
            point = vec3(target.x, target.y, enemy.origin.z + enemy.bounds.min.z + 1.0);
        }
        if context.game.random() < 0.35 {
            if let Some(aim) = predict_aim(context, start, speed, false, 0.0) {
                point = aim.point;
            }
        }
    }
    let trace_end = |game: &mut crate::q2::foundation::host::Q2GameServices, end: Vec3| {
        game.host.trace(&Q2TraceRequest {
            start,
            end,
            bounds: None,
            ignore: Some(actor.clone()),
            mask: 0x4600_4003,
            exclude: Vec::new(),
        })
    };
    let mut obstruction = trace_end(&mut *context.game, point);
    if manual {
        let blocked = |trace: &crate::q2::support::contracts::TraceResult| {
            trace.start_solid || trace.all_solid || trace.fraction < 0.5
        };
        if blocked(&obstruction) {
            point = add3(target, scale3(right, -10.0));
            obstruction = trace_end(&mut *context.game, point);
        }
        if blocked(&obstruction) {
            point = add3(target, scale3(right, 10.0));
            obstruction = trace_end(&mut *context.game, point);
        }
        if blocked(&obstruction) {
            return;
        }
    } else if obstruction.fraction <= 0.5 && chick_trace_blocked(context, &obstruction) {
        return;
    }
    let direction = normalize3(sub3(point, start));
    if heat {
        let weapons = mission_weapons(&*context.game);
        weapons.fire_heat_rocket(
            actor,
            &mut *context.game,
            start,
            direction,
            50.0,
            speed,
            70.0,
            50.0,
            Some(if manual { 0.075 } else { 0.15 }),
        );
    } else {
        let fire_rocket = context.weapons.fire_rocket;
        fire_rocket(actor, &mut *context.game, start, direction, 50.0, speed, 70.0, 50.0);
    }
    monster_flash(context, 57, start, direction);
}

/// Whether a rocket trace is blocked (`rocket` obstruction check).
fn chick_trace_blocked(context: &mut MonsterContext, trace: &crate::q2::support::contracts::TraceResult) -> bool {
    match &trace.hit {
        TraceHit::World { .. } => true,
        TraceHit::Actor { actor } => context
            .game
            .entity(actor)
            .is_some_and(|target| target.solid == Q2Solid::Brush),
        TraceHit::None => false,
    }
}

/// Melee (`melee`).
fn chick_melee(context: &mut MonsterContext) {
    context.set_move("chick_move_start_slash", true);
}

/// Sight (`sight`).
fn chick_sight(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "chick/chksght1.wav", 2, 1.0, 1.0);
}

/// Attack (`attack`).
fn rerelease_chick_attack(context: &mut MonsterContext) {
    if !clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 57)) {
        return;
    }
    finish_dodge(context);
    if context.state().attack_state == MonsterAttackState::Blind {
        let delay = context.state().blind_fire_delay;
        let chance = if delay < 1.0 {
            1.0
        } else if delay < 7.5 {
            0.4
        } else {
            0.1
        };
        let choice = context.game.random();
        context.state_mut().blind_fire_delay = delay + 5.5 + context.game.random();
        if length3(context.state().blind_fire_target) == 0.0 || choice > chance {
            return;
        }
        context.state_mut().manual_steering = true;
        let now = context.game.host.now();
        context.state_mut().attack_finished = now + context.game.random() * 2.0;
    }
    context.set_move("chick_move_start_attack1", true);
}

/// Pain (`pain`).
fn rerelease_chick_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    finish_dodge(context);
    let max_health = context.game.require_entity(&actor).max_health;
    let skin = context.game.require_entity(&actor).skin;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { skin | 1 } else { skin & !1 };
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let choice = context.game.random();
    context.game.sound(
        &actor,
        if choice < 0.33 {
            "chick/chkpain1.wav"
        } else if choice < 0.66 {
            "chick/chkpain2.wav"
        } else {
            "chick/chkpain3.wav"
        },
        2,
        1.0,
        1.0,
    );
    if !reacts_to_pain(context) {
        return;
    }
    context.state_mut().manual_steering = false;
    let damage = reaction.damage;
    context.set_move(
        if damage <= 10.0 {
            "chick_move_pain1"
        } else if damage <= 25.0 {
            "chick_move_pain2"
        } else {
            "chick_move_pain3"
        },
        true,
    );
    if context.state().ducked {
        set_duck(context, false);
    }
}

/// Die (`die`).
fn rerelease_chick_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if check_gib(context) {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        context.game.require_entity_mut(&actor).skin /= 2;
        let damage = reaction.pain.damage;
        for _ in 0..2 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/bone/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
        }
        for _ in 0..3 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/sm_meat/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
        }
        for part in ["arm", "foot", "tube"] {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                &format!("models/monsters/bitch/gibs/{part}.md2"),
                damage,
                Q2GibOptions {
                    skinned: true,
                    upright: true,
                    ..Q2GibOptions::default()
                },
            );
        }
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/bitch/gibs/chest.md2",
            damage,
            Q2GibOptions {
                skinned: true,
                ..Q2GibOptions::default()
            },
        );
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/bitch/gibs/head.md2",
            damage,
            Q2GibOptions {
                skinned: true,
                head: true,
                ..Q2GibOptions::default()
            },
        );
        context.state_mut().dead = true;
        context.state_mut().gibbed = true;
        return;
    }
    if context.state().dead {
        return;
    }
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor.clone());
    context.game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    let first = context.game.random() >= 0.5;
    context.game.sound(
        &actor,
        if first {
            "chick/chkdeth1.wav"
        } else {
            "chick/chkdeth2.wav"
        },
        2,
        1.0,
        1.0,
    );
    context.set_move(
        if first {
            "chick_move_death1"
        } else {
            "chick_move_death2"
        },
        true,
    );
}

/// Duck (`duck`).
fn rerelease_chick_duck(context: &mut MonsterContext, _eta: f64) -> bool {
    let current = context.state().current_move.name.clone();
    if current == "chick_move_start_attack1" || current == "chick_move_attack1" {
        context.dispatch("monster_duck_up");
        return false;
    }
    context.set_move("chick_move_duck", true);
    true
}

/// Sidestep (`sidestep`).
fn rerelease_chick_sidestep(context: &mut MonsterContext) -> bool {
    let current = context.state().current_move.name.clone();
    if current == "chick_move_start_attack1" || current == "chick_move_attack1" || current == "chick_move_pain3" {
        return false;
    }
    if current != "chick_move_run" {
        context.set_move("chick_move_run", true);
    }
    true
}

/// Pre-attack (`Chick_PreAttack1`).
fn chick_pre_attack1(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "chick/chkatck1.wav", 2, 1.0, 1.0);
    if context.state().manual_steering {
        let target = context.state().blind_fire_target;
        let origin = context.game.body_of(actor).origin;
        context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(target, origin)).y);
    }
}

/// Slash (`ChickSlash`).
fn chick_slash(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "chick/chkatck3.wav", 1, 1.0, 1.0);
    let aim = vec3(80.0, context.game.body_of(actor.clone()).bounds.min.x, 10.0);
    let damage = 10.0 + (context.game.random() * 6.0).floor();
    let fire_hit = context.weapons.fire_hit;
    fire_hit(actor, &mut *context.game, aim, damage, 100.0);
}

/// Rerocket (`chick_rerocket`).
fn chick_rerocket(context: &mut MonsterContext) {
    if context.state().manual_steering {
        context.state_mut().manual_steering = false;
        context.set_move("chick_move_end_attack1", true);
        return;
    }
    let again = clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 57))
        && alive_enemy(context)
        && target_distance(context) > 80.0
        && visible(context, None)
        && context.game.random() <= 0.7;
    context.set_move(
        if again {
            "chick_move_attack1"
        } else {
            "chick_move_end_attack1"
        },
        true,
    );
}

/// Reslash (`chick_reslash`).
fn chick_reslash(context: &mut MonsterContext) {
    let again = alive_enemy(context) && target_distance(context) <= 80.0 && context.game.random() <= 0.9;
    context.set_move(
        if again {
            "chick_move_slash"
        } else {
            "chick_move_end_slash"
        },
        true,
    );
}

/// Shrink (`chick_shrink`).
fn chick_shrink(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).server_flags |= 2;
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.max.z = 12.0;
    context.game.write_body(actor, &moved, true);
}

/// Dead (`chick_dead`).
fn chick_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    corpse(context);
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.min = vec3(-16.0, -16.0, 0.0);
    moved.bounds.max = vec3(16.0, 16.0, 8.0);
    context.game.write_body(actor, &moved, true);
}

/// Heat after-spawn (`monster_chick_heat afterSpawn`).
fn chick_heat_after_spawn(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).skin = 2;
}

/// Create the rerelease chick definitions (`createRereleaseChickDefinitions`).
pub fn create_rerelease_chick_definitions() -> Vec<Q2MonsterDefinition> {
    let base = chick_definition();
    let mut definition = Q2MonsterDefinition::new(
        &base.classname,
        &base.kind,
        &base.model,
        175.0,
        -70.0,
        200.0,
        base.bounds,
        1.0,
        &base.initial_move,
        chick_moves(),
        base.stand.clone(),
        base.walk.clone(),
        MonsterHandler::Callback(rerelease_chick_run),
        MonsterHandler::Callback(rerelease_chick_attack),
        rerelease_chick_die,
    );
    definition.melee = Some(MonsterHandler::Callback(chick_melee));
    definition.sight = Some(MonsterHandler::Callback(chick_sight));
    definition.blind_fire = true;
    definition.pain = Some(rerelease_chick_pain);
    definition.duck = Some(rerelease_chick_duck);
    definition.sidestep = Some(rerelease_chick_sidestep);
    definition.blocked = Some(blocked_check_platform);
    definition.callbacks = base.callbacks.clone();
    for (name, handler) in [
        ("chick_run", MonsterHandler::Callback(rerelease_chick_run)),
        ("ChickRocket", MonsterHandler::Callback(chick_rocket)),
        ("Chick_PreAttack1", MonsterHandler::Callback(chick_pre_attack1)),
        ("ChickSlash", MonsterHandler::Callback(chick_slash)),
        ("chick_rerocket", MonsterHandler::Callback(chick_rerocket)),
        ("chick_reslash", MonsterHandler::Callback(chick_reslash)),
        ("chick_shrink", MonsterHandler::Callback(chick_shrink)),
        ("chick_dead", MonsterHandler::Callback(chick_dead)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    let mut heat = definition.clone();
    heat.classname = "monster_chick_heat".to_string();
    heat.after_spawn = Some(MonsterHandler::Callback(chick_heat_after_spawn));
    vec![definition, heat]
}
