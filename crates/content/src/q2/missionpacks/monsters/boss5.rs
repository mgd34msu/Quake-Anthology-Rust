//! Boss5 monster (`src/content/q2/missionpacks/monsters/boss5.ts`).
//!
//! Quake II xatrix/m_boss5.c. ZeniMax Media, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::math::{Bounds, Vec3, normalize3, sub3, vec3};

use super::power_armor::{PowerArmorKind, monster_power_armor, restore_monster_power_armor};
use super::tables::xatrix_boss5::{boss5_frame, boss5_moves};
use crate::q2::base::monsters::boss_common::boss_explode;
use crate::q2::base::monsters::common::{
    damaged_skin, finish_corpse, monster_muzzle, monster_shot, move_handler, sound_handler,
};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, enemy_eye, project_flash, target_distance, visible,
};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Run (`run`).
fn boss5_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("boss5_move_stand", false);
    } else {
        context.set_move("boss5_move_run", false);
    }
}

/// Initialize (`initialize`).
fn boss5_initialize(context: &mut MonsterContext) {
    monster_power_armor(context, PowerArmorKind::Shield, 400.0);
}

/// Search (`search`).
fn boss5_search(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let path = if context.game.random() < 0.5 {
        "bosstank/btkunqv1.wav"
    } else {
        "bosstank/btkunqv2.wav"
    };
    context.game.sound(&actor, path, 2, 1.0, 1.0);
}

/// Attack (`attack`).
fn boss5_attack(context: &mut MonsterContext) {
    if target_distance(context) <= 160.0 || context.game.random() < 0.3 {
        context.set_move("boss5_move_attack1", false);
    } else {
        context.set_move("boss5_move_attack2", false);
    }
}

/// Pain (`pain`).
fn boss5_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time
        || reaction.damage <= 25.0 && context.game.random() < 0.2
    {
        return;
    }
    let frame = context.entity().frame;
    if context.game.options.skill >= 2
        && frame >= boss5_frame::ATTAK2_1
        && frame <= boss5_frame::ATTAK2_14
    {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if reaction.damage <= 10.0 {
            "bosstank/btkpain1.wav"
        } else if reaction.damage <= 25.0 {
            "bosstank/btkpain3.wav"
        } else {
            "bosstank/btkpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    context.set_move(
        if reaction.damage <= 10.0 {
            "boss5_move_pain1"
        } else if reaction.damage <= 25.0 {
            "boss5_move_pain2"
        } else {
            "boss5_move_pain3"
        },
        false,
    );
}

/// Die (`die`).
fn boss5_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "bosstank/btkdeth1.wav", 2, 1.0, 1.0);
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
    context.set_move("boss5_move_death", false);
}

/// Dead (`boss5_dead`).
fn boss5_dead(context: &mut MonsterContext) {
    finish_corpse(
        context,
        Bounds {
            min: Vec3 { x: -60.0, y: -60.0, z: 0.0 },
            max: Vec3 { x: 60.0, y: 60.0, z: 72.0 },
        },
    );
}

/// Reattack (`boss5_reattack1`).
fn boss5_reattack1(context: &mut MonsterContext) {
    if visible(context, None) && context.game.random() < 0.9 {
        context.set_move("boss5_move_attack1", false);
    } else {
        context.set_move("boss5_move_end_attack1", false);
    }
}

/// Rocket (`boss5Rocket`).
fn boss5_rocket(context: &mut MonsterContext) {
    let frame = context.entity().frame;
    let flash = if frame == boss5_frame::ATTAK2_8 {
        70usize
    } else if frame == boss5_frame::ATTAK2_11 {
        71
    } else {
        72
    };
    let Some((start, direction)) = monster_shot(context, flash, 0.0) else {
        return;
    };
    let fire_rocket = context.weapons.fire_rocket;
    let actor = context.actor().clone();
    fire_rocket(
        actor,
        &mut *context.game,
        start,
        direction,
        50.0,
        500.0,
        70.0,
        50.0,
    );
    monster_muzzle(context, flash as i32, direction, start);
}

/// Machine gun (`boss5MachineGun`).
fn boss5_machine_gun(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let frame = context.entity().frame;
    let flash = 64 + frame - boss5_frame::ATTAK1_1;
    let body = context.game.body_of(actor);
    let edition = context.game.options.edition;
    let start = project_flash(
        context,
        muzzle_offset(edition, flash as usize),
        Some(vec3(0.0, body.angles.y, 0.0)),
    );
    let direction = enemy_eye(context)
        .map(|eye| normalize3(sub3(eye, start)))
        .unwrap_or_else(|| angles_vectors(vec3(0.0, body.angles.y, 0.0)).forward);
    let fire_bullet = context.weapons.fire_bullet;
    let actor = context.actor().clone();
    fire_bullet(
        actor,
        &mut *context.game,
        start,
        direction,
        6.0,
        4.0,
        300.0,
        500.0,
        0,
    );
    monster_muzzle(context, flash, direction, start);
}

/// Boss5 definition (`boss5Definition`).
pub fn boss5_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_boss5",
        "boss5",
        "models/monsters/boss5/tris.md2",
        1500.0,
        -500.0,
        800.0,
        Bounds {
            min: Vec3 { x: -64.0, y: -64.0, z: 0.0 },
            max: Vec3 { x: 64.0, y: 64.0, z: 112.0 },
        },
        1.0,
        "boss5_move_stand",
        boss5_moves(),
        move_handler("boss5_move_stand"),
        move_handler("boss5_move_forward"),
        MonsterHandler::Callback(boss5_run),
        MonsterHandler::Callback(boss5_attack),
        boss5_die,
    );
    definition.search = Some(MonsterHandler::Callback(boss5_search));
    definition.pain = Some(boss5_pain);
    definition.initialize = Some(MonsterHandler::Callback(boss5_initialize));
    definition.restore = Some(MonsterHandler::Callback(restore_monster_power_armor));
    definition.callbacks = HashMap::from([
        ("boss5_run".to_string(), MonsterHandler::Callback(boss5_run)),
        (
            "BossExplode2".to_string(),
            MonsterHandler::Callback(boss_explode),
        ),
        (
            "TreadSound2".to_string(),
            sound_handler("bosstank/btkengn1.wav", 2, 1.0),
        ),
        (
            "boss5_dead".to_string(),
            MonsterHandler::Callback(boss5_dead),
        ),
        (
            "boss5_reattack1".to_string(),
            MonsterHandler::Callback(boss5_reattack1),
        ),
        (
            "boss5Rocket".to_string(),
            MonsterHandler::Callback(boss5_rocket),
        ),
        (
            "boss5MachineGun".to_string(),
            MonsterHandler::Callback(boss5_machine_gun),
        ),
    ]);
    definition
}
