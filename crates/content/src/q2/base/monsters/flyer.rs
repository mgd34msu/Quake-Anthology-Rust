//! Flyer monster (`src/content/q2/base/monsters/flyer.ts`).
//!
//! Quake II m_flyer.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::math::vec3;

use super::common::{
    HUMANOID_BOUNDS, damaged_skin, monster_explode, monster_loop_sound, monster_muzzle,
    monster_shot, move_handler, sound_handler,
};
use super::tables::flyer::{flyer_frame, flyer_moves};
use crate::q2::foundation::monsters::ai::target_distance;
use crate::q2::foundation::monsters::types::{
    MonsterContext, MonsterFlyerNext, MonsterHandler, MonsterLocomotion, Q2MonsterDefinition,
};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::support::contracts::{DeathReaction, PainReaction};

/// Run (`run`).
fn flyer_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("flyer_move_stand", false);
    } else {
        context.set_move("flyer_move_run", false);
    }
}

/// Fire a blaster bolt (`fire`).
fn flyer_fire(context: &mut MonsterContext, flash: usize) {
    let Some((start, direction)) = monster_shot(context, flash, 0.0) else {
        return;
    };
    let frame = context.entity().frame;
    let effect = if frame == flyer_frame::ATTAK204
        || frame == flyer_frame::ATTAK207
        || frame == flyer_frame::ATTAK210
    {
        64
    } else {
        0
    };
    let fire_blaster = context.weapons.fire_blaster;
    let actor = context.actor().clone();
    fire_blaster(
        actor,
        &mut *context.game,
        start,
        direction,
        1.0,
        1000.0,
        effect,
        false,
        Mod::BLASTER,
    );
    monster_muzzle(context, flash as i32, direction, start);
}

/// Slash (`slash`).
fn flyer_slash(context: &mut MonsterContext, right: bool) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let bounds = context.game.body_of(actor.clone()).bounds;
    fire_hit(
        actor.clone(),
        &mut *context.game,
        vec3(80.0, if right { bounds.max.x } else { bounds.min.x }, 0.0),
        5.0,
        0.0,
    );
    context.game.sound(&actor, "flyer/flyatck2.wav", 1, 1.0, 1.0);
}

/// Initialize (`initialize`).
fn flyer_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.options.map_name.to_lowercase() == "jail5"
        && context.game.body_of(actor).origin.z == -104.0
    {
        let target = context.entity().target.clone();
        let entity = context.entity_mut();
        entity.targetname = target;
        entity.target = String::new();
    }
    monster_loop_sound(context, "flyer/flyidle1.wav");
}

/// Pain (`pain`).
pub fn flyer_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
    let n = (context.game.random() * 3.0).floor() as i32;
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if n == 1 {
            "flyer/flypain2.wav"
        } else {
            "flyer/flypain1.wav"
        },
        2,
        1.0,
        1.0,
    );
    context.set_move(
        if n == 0 {
            "flyer_move_pain1"
        } else if n == 1 {
            "flyer_move_pain2"
        } else {
            "flyer_move_pain3"
        },
        false,
    );
}

/// Die (`die`).
fn flyer_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    monster_explode(context, "flyer/flydeth1.wav");
}

/// Fire left (`flyer_fireleft`).
fn flyer_fire_left(context: &mut MonsterContext) {
    flyer_fire(context, 58);
}

/// Fire right (`flyer_fireright`).
fn flyer_fire_right(context: &mut MonsterContext) {
    flyer_fire(context, 59);
}

/// Slash left (`flyer_slash_left`).
fn flyer_slash_left(context: &mut MonsterContext) {
    flyer_slash(context, false);
}

/// Slash right (`flyer_slash_right`).
fn flyer_slash_right(context: &mut MonsterContext) {
    flyer_slash(context, true);
}

/// Check melee (`flyer_check_melee`).
fn flyer_check_melee(context: &mut MonsterContext) {
    if target_distance(context) < 80.0 && context.game.random() <= 0.8 {
        context.set_move("flyer_move_loop_melee", false);
    } else {
        context.set_move("flyer_move_end_melee", false);
    }
}

/// Set start (`flyer_setstart`).
fn flyer_set_start(context: &mut MonsterContext) {
    context.game.monsters.flyer_next = Some(MonsterFlyerNext::Run);
    context.set_move("flyer_move_start", false);
}

/// Next move (`flyer_nextmove`).
fn flyer_next_move(context: &mut MonsterContext) {
    let Some(next) = context.game.monsters.flyer_next else {
        return;
    };
    context.set_move(
        match next {
            MonsterFlyerNext::Melee => "flyer_move_start_melee",
            MonsterFlyerNext::Attack => "flyer_move_attack2",
            MonsterFlyerNext::Run => "flyer_move_run",
        },
        false,
    );
}

/// Flyer definition (`flyerDefinition`).
pub fn flyer_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_flyer",
        "flyer",
        "models/monsters/flyer/tris.md2",
        50.0,
        0.0,
        50.0,
        HUMANOID_BOUNDS,
        1.0,
        "flyer_move_stand",
        flyer_moves(),
        move_handler("flyer_move_stand"),
        move_handler("flyer_move_walk"),
        MonsterHandler::Callback(flyer_run),
        move_handler("flyer_move_attack2"),
        flyer_die,
    );
    definition.locomotion = Some(MonsterLocomotion::Fly);
    definition.melee = Some(move_handler("flyer_move_start_melee"));
    definition.sight = Some(sound_handler("flyer/flysght1.wav", 2, 1.0));
    definition.idle = Some(sound_handler("flyer/flysrch1.wav", 2, 2.0));
    definition.pain = Some(flyer_pain);
    definition.initialize = Some(MonsterHandler::Callback(flyer_initialize));
    definition.callbacks = HashMap::from([
        ("flyer_run".to_string(), MonsterHandler::Callback(flyer_run)),
        (
            "flyer_pop_blades".to_string(),
            sound_handler("flyer/flyatck1.wav", 2, 1.0),
        ),
        (
            "flyer_loop_melee".to_string(),
            move_handler("flyer_move_loop_melee"),
        ),
        (
            "flyer_fireleft".to_string(),
            MonsterHandler::Callback(flyer_fire_left),
        ),
        (
            "flyer_fireright".to_string(),
            MonsterHandler::Callback(flyer_fire_right),
        ),
        (
            "flyer_slash_left".to_string(),
            MonsterHandler::Callback(flyer_slash_left),
        ),
        (
            "flyer_slash_right".to_string(),
            MonsterHandler::Callback(flyer_slash_right),
        ),
        (
            "flyer_check_melee".to_string(),
            MonsterHandler::Callback(flyer_check_melee),
        ),
        (
            "flyer_setstart".to_string(),
            MonsterHandler::Callback(flyer_set_start),
        ),
        (
            "flyer_nextmove".to_string(),
            MonsterHandler::Callback(flyer_next_move),
        ),
    ]);
    definition
}
