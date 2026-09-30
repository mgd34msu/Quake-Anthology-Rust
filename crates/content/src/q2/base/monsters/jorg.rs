//! Jorg monster (`src/content/q2/base/monsters/jorg.ts`).
//!
//! Quake II m_boss31.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::math::{Bounds, Vec3};

use super::boss_common::{boss_check_attack, boss_explode, stop_loop};
use super::common::{damaged_skin, monster_loop_sound, monster_muzzle, monster_shot, move_handler, sound_handler};
use super::makron::makron_toss;
use super::tables::boss31::{boss31_frame, boss31_moves};
use crate::q2::foundation::monsters::ai::visible;
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Run (`run`).
fn jorg_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("jorg_move_stand", false);
    } else {
        context.set_move("jorg_move_run", false);
    }
}

/// Initialize (`initialize`).
pub(crate) fn jorg_initialize(context: &mut MonsterContext) {
    context.entity_mut().model2 = "models/monsters/boss3/jorg/tris.md2".to_string();
}

/// Check attack (`checkAttack`).
fn jorg_check_attack(context: &mut MonsterContext) -> bool {
    boss_check_attack(context, false, false)
}

/// Search (`search`).
fn jorg_search(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let r = context.game.random();
    context.game.sound(
        &actor,
        if r <= 0.3 {
            "boss3/bs3srch1.wav"
        } else if r <= 0.6 {
            "boss3/bs3srch2.wav"
        } else {
            "boss3/bs3srch3.wav"
        },
        2,
        1.0,
        1.0,
    );
}

/// Attack (`attack`).
fn jorg_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.random() <= 0.75 {
        context.game.sound(&actor, "boss3/bs3atck1.wav", 2, 1.0, 1.0);
        monster_loop_sound(context, "boss3/w_loop.wav");
        context.set_move("jorg_move_start_attack1", false);
    } else {
        context.game.sound(&actor, "boss3/bs3atck2.wav", 2, 1.0, 1.0);
        context.set_move("jorg_move_attack2", false);
    }
}

/// Pain (`pain`).
fn jorg_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    stop_loop(context);
    if context.game.host.now() < context.state().pain_time
        || reaction.damage <= 40.0 && context.game.random() <= 0.6
    {
        return;
    }
    let frame = context.entity().frame;
    if frame >= boss31_frame::ATTAK101
        && frame <= boss31_frame::ATTAK108
        && context.game.random() <= 0.005
    {
        return;
    }
    if frame >= boss31_frame::ATTAK109
        && frame <= boss31_frame::ATTAK114
        && context.game.random() <= 0.00005
    {
        return;
    }
    if frame >= boss31_frame::ATTAK201
        && frame <= boss31_frame::ATTAK208
        && context.game.random() <= 0.005
    {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
    if reaction.damage > 100.0 && context.game.random() > 0.3 {
        return;
    }
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if reaction.damage <= 50.0 {
            "boss3/bs3pain1.wav"
        } else if reaction.damage <= 100.0 {
            "boss3/bs3pain2.wav"
        } else {
            "boss3/bs3pain3.wav"
        },
        2,
        1.0,
        1.0,
    );
    context.set_move(
        if reaction.damage <= 50.0 {
            "jorg_move_pain1"
        } else if reaction.damage <= 100.0 {
            "jorg_move_pain2"
        } else {
            "jorg_move_pain3"
        },
        false,
    );
}

/// Die (`die`).
fn jorg_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "boss3/bs3deth1.wav", 2, 1.0, 1.0);
    stop_loop(context);
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
    context.set_move("jorg_move_death", false);
}

/// Dead (`jorg_dead`).
///
/// The original jorg_dead body is excluded by `#if 0`.
fn jorg_dead(_context: &mut MonsterContext) {}

/// Makron toss (`MakronToss`).
fn jorg_makron_toss(context: &mut MonsterContext) {
    makron_toss(context);
}

/// Reattack (`jorg_reattack1`).
fn jorg_reattack1(context: &mut MonsterContext) {
    if visible(context, None) && context.game.random() < 0.9 {
        context.set_move("jorg_move_attack1", false);
        return;
    }
    stop_loop(context);
    context.set_move("jorg_move_end_attack1", false);
}

/// BFG (`jorgBFG`).
fn jorg_bfg(context: &mut MonsterContext) {
    let Some((start, direction)) = monster_shot(context, 132, 0.0) else {
        return;
    };
    let actor = context.actor().clone();
    context.game.sound(&actor, "boss3/bs3atck2.wav", 2, 1.0, 1.0);
    let fire_bfg = context.weapons.fire_bfg;
    fire_bfg(actor, &mut *context.game, start, direction, 50.0, 300.0, 200.0);
    monster_muzzle(context, 132, direction, start);
}

/// Fire bullets (`jorg_firebullet`).
fn jorg_firebullet(context: &mut MonsterContext) {
    let fire_bullet = context.weapons.fire_bullet;
    for flash in [120usize, 126] {
        let Some((start, direction)) = monster_shot(context, flash, -0.2) else {
            return;
        };
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
        monster_muzzle(context, flash as i32, direction, start);
    }
}

/// Create a jorg definition (`createJorgDefinition`).
pub fn create_jorg_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_jorg",
        "jorg",
        "models/monsters/boss3/rider/tris.md2",
        3000.0,
        -2000.0,
        1000.0,
        Bounds {
            min: Vec3 { x: -80.0, y: -80.0, z: 0.0 },
            max: Vec3 { x: 80.0, y: 80.0, z: 140.0 },
        },
        1.0,
        "jorg_move_stand",
        boss31_moves(),
        move_handler("jorg_move_stand"),
        move_handler("jorg_move_walk"),
        MonsterHandler::Callback(jorg_run),
        MonsterHandler::Callback(jorg_attack),
        jorg_die,
    );
    definition.search = Some(MonsterHandler::Callback(jorg_search));
    definition.pain = Some(jorg_pain);
    definition.check_attack = Some(jorg_check_attack);
    definition.initialize = Some(MonsterHandler::Callback(jorg_initialize));
    definition.callbacks = HashMap::from([
        ("jorg_run".to_string(), MonsterHandler::Callback(jorg_run)),
        (
            "jorg_attack1".to_string(),
            move_handler("jorg_move_attack1"),
        ),
        (
            "BossExplode".to_string(),
            MonsterHandler::Callback(boss_explode),
        ),
        (
            "jorg_idle".to_string(),
            sound_handler("boss3/bs3idle1.wav", 2, 1.0),
        ),
        (
            "jorg_step_left".to_string(),
            sound_handler("boss3/step1.wav", 4, 1.0),
        ),
        (
            "jorg_step_right".to_string(),
            sound_handler("boss3/step2.wav", 4, 1.0),
        ),
        (
            "jorg_death_hit".to_string(),
            sound_handler("boss3/d_hit.wav", 4, 1.0),
        ),
        (
            "jorg_dead".to_string(),
            MonsterHandler::Callback(jorg_dead),
        ),
        (
            "MakronToss".to_string(),
            MonsterHandler::Callback(jorg_makron_toss),
        ),
        (
            "jorg_reattack1".to_string(),
            MonsterHandler::Callback(jorg_reattack1),
        ),
        (
            "jorgBFG".to_string(),
            MonsterHandler::Callback(jorg_bfg),
        ),
        (
            "jorg_firebullet".to_string(),
            MonsterHandler::Callback(jorg_firebullet),
        ),
    ]);
    definition
}
