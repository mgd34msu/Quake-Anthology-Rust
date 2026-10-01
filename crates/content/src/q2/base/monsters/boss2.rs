//! Boss2 monster (`src/content/q2/base/monsters/boss2.ts`).
//!
//! Quake II m_boss2.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::math::{Bounds, Vec3};

use super::boss_common::{boss_check_attack, boss_explode};
use super::common::{
    damaged_skin, finish_corpse, monster_loop_sound, monster_muzzle, monster_shot, move_handler, sound_handler,
};
use super::tables::boss2::boss2_moves;
use crate::q2::foundation::monsters::ai::{in_front, target_distance};
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, MonsterLocomotion, Q2MonsterDefinition};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Run (`run`).
fn boss2_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("boss2_move_stand", true);
    } else {
        context.set_move("boss2_move_run", true);
    }
}

/// Attack (`attack`).
fn boss2_attack(context: &mut MonsterContext) {
    if target_distance(context) <= 125.0 || context.game.random() <= 0.6 {
        context.set_move("boss2_move_attack_pre_mg", true);
    } else {
        context.set_move("boss2_move_attack_rocket", true);
    }
}

/// Check attack (`checkAttack`).
fn boss2_check_attack(context: &mut MonsterContext) -> bool {
    boss_check_attack(context, true, false)
}

/// Initialize (`initialize`).
fn boss2_initialize(context: &mut MonsterContext) {
    context.entity_mut().laser_immune = true;
    monster_loop_sound(context, "bosshovr/bhvengn1.wav");
}

/// Pain (`pain`).
fn boss2_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let path = if reaction.damage < 10.0 {
        "bosshovr/bhvpain3.wav"
    } else if reaction.damage < 30.0 {
        "bosshovr/bhvpain1.wav"
    } else {
        "bosshovr/bhvpain2.wav"
    };
    let actor = context.actor().clone();
    context.game.sound(&actor, path, 2, 1.0, 0.0);
    context.set_move(
        if reaction.damage < 30.0 {
            "boss2_move_pain_light"
        } else {
            "boss2_move_pain_heavy"
        },
        false,
    );
}

/// Die (`die`).
fn boss2_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "bosshovr/bhvdeth1.wav", 2, 1.0, 0.0);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = false;
    context.entity_mut().count = 0;
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(false),
            ..CombatTraitChanges::default()
        },
    );
    context.set_move("boss2_move_death", true);
}

/// Dead (`boss2_dead`).
fn boss2_dead(context: &mut MonsterContext) {
    finish_corpse(
        context,
        Bounds {
            min: Vec3 {
                x: -56.0,
                y: -56.0,
                z: 0.0,
            },
            max: Vec3 {
                x: 56.0,
                y: 56.0,
                z: 80.0,
            },
        },
    );
}

/// Reattack (`boss2_reattack_mg`).
fn boss2_reattack_mg(context: &mut MonsterContext) {
    let enemy = context.entity().enemy.clone();
    let again = enemy
        .as_ref()
        .is_some_and(|enemy| in_front(context, enemy) && context.game.random() <= 0.7);
    context.set_move(
        if again {
            "boss2_move_attack_mg"
        } else {
            "boss2_move_attack_post_mg"
        },
        false,
    );
}

/// Machine gun (`Boss2MachineGun`).
fn boss2_machine_gun(context: &mut MonsterContext) {
    let fire_bullet = context.weapons.fire_bullet;
    for flash in [73usize, 133] {
        let Some((start, direction)) = monster_shot(context, flash, -0.2) else {
            return;
        };
        let actor = context.actor().clone();
        fire_bullet(actor, &mut *context.game, start, direction, 6.0, 4.0, 300.0, 500.0, 0);
        monster_muzzle(context, flash as i32, direction, start);
    }
}

/// Rockets (`Boss2Rocket`).
fn boss2_rocket(context: &mut MonsterContext) {
    let fire_rocket = context.weapons.fire_rocket;
    for flash in [78usize, 79, 80, 81] {
        let Some((start, direction)) = monster_shot(context, flash, 0.0) else {
            return;
        };
        let actor = context.actor().clone();
        fire_rocket(actor, &mut *context.game, start, direction, 50.0, 500.0, 70.0, 50.0);
        monster_muzzle(context, flash as i32, direction, start);
    }
}

/// Boss2 definition (`boss2Definition`).
pub fn boss2_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_boss2",
        "boss2",
        "models/monsters/boss2/tris.md2",
        2000.0,
        -200.0,
        1000.0,
        Bounds {
            min: Vec3 {
                x: -56.0,
                y: -56.0,
                z: 0.0,
            },
            max: Vec3 {
                x: 56.0,
                y: 56.0,
                z: 80.0,
            },
        },
        1.0,
        "boss2_move_stand",
        boss2_moves(),
        move_handler("boss2_move_stand"),
        move_handler("boss2_move_walk"),
        MonsterHandler::Callback(boss2_run),
        MonsterHandler::Callback(boss2_attack),
        boss2_die,
    );
    definition.locomotion = Some(MonsterLocomotion::Fly);
    definition.search = Some(sound_handler("bosshovr/bhvunqv1.wav", 2, 1.0));
    definition.pain = Some(boss2_pain);
    definition.check_attack = Some(boss2_check_attack);
    definition.initialize = Some(MonsterHandler::Callback(boss2_initialize));
    definition.callbacks = HashMap::from([
        ("boss2_run".to_string(), MonsterHandler::Callback(boss2_run)),
        ("boss2_attack_mg".to_string(), move_handler("boss2_move_attack_mg")),
        ("BossExplode".to_string(), MonsterHandler::Callback(boss_explode)),
        ("boss2_dead".to_string(), MonsterHandler::Callback(boss2_dead)),
        (
            "boss2_reattack_mg".to_string(),
            MonsterHandler::Callback(boss2_reattack_mg),
        ),
        (
            "Boss2MachineGun".to_string(),
            MonsterHandler::Callback(boss2_machine_gun),
        ),
        ("Boss2Rocket".to_string(), MonsterHandler::Callback(boss2_rocket)),
    ]);
    definition
}
