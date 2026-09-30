//! Flipper monster (`src/content/q2/base/monsters/flipper.ts`).
//!
//! Quake II m_flipper.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::math::{Bounds, Vec3, vec3};

use super::common::{
    begin_death, damaged_skin, finish_corpse_default, move_handler, sound_handler, standard_gib,
};
use super::tables::flipper::flipper_moves;
use crate::q2::foundation::monsters::types::{
    MonsterContext, MonsterHandler, MonsterLocomotion, Q2MonsterDefinition,
};
use crate::q2::support::contracts::{DeathReaction, PainReaction};

/// Pain (`pain`).
fn flipper_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
    let first = ((context.game.random() * 2.0).floor() + 1.0) as i32 % 2 == 0;
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if first {
            "flipper/flppain1.wav"
        } else {
            "flipper/flppain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    context.set_move(
        if first {
            "flipper_move_pain1"
        } else {
            "flipper_move_pain2"
        },
        false,
    );
}

/// Die (`die`).
fn flipper_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    if standard_gib(
        context,
        reaction,
        2,
        2,
        "models/objects/gibs/sm_meat/tris.md2",
        1.0,
    ) {
        return;
    }
    begin_death(context, reaction, "flipper/flpdeth1.wav", "flipper_move_death", 2, 4);
}

/// Bite (`flipper_bite`).
fn flipper_bite(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    fire_hit(actor, &mut *context.game, vec3(80.0, 0.0, 0.0), 5.0, 0.0);
}

/// Flipper definition (`flipperDefinition`).
pub fn flipper_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_flipper",
        "flipper",
        "models/monsters/flipper/tris.md2",
        50.0,
        -30.0,
        100.0,
        Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: 0.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 32.0,
            },
        },
        1.0,
        "flipper_move_stand",
        flipper_moves(),
        move_handler("flipper_move_stand"),
        move_handler("flipper_move_walk"),
        move_handler("flipper_move_start_run"),
        move_handler("flipper_move_attack"),
        flipper_die,
    );
    definition.locomotion = Some(MonsterLocomotion::Swim);
    definition.melee = Some(move_handler("flipper_move_attack"));
    definition.sight = Some(sound_handler("flipper/flpsght1.wav", 2, 1.0));
    definition.pain = Some(flipper_pain);
    definition.callbacks = HashMap::from([
        (
            "flipper_run".to_string(),
            move_handler("flipper_move_run_start"),
        ),
        (
            "flipper_run_loop".to_string(),
            move_handler("flipper_move_run_loop"),
        ),
        (
            "flipper_dead".to_string(),
            MonsterHandler::Callback(finish_corpse_default),
        ),
        (
            "flipper_preattack".to_string(),
            sound_handler("flipper/flpatck1.wav", 1, 1.0),
        ),
        (
            "flipper_bite".to_string(),
            MonsterHandler::Callback(flipper_bite),
        ),
    ]);
    definition
}
