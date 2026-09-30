//! Floater monster (`src/content/q2/base/monsters/floater.ts`).
//!
//! Quake II m_float.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::math::{Bounds, Vec3, sub3, vec3};

use super::common::{
    damaged_skin, finish_corpse_default, monster_explode, monster_loop_sound, monster_muzzle,
    monster_shot, move_handler, sound_handler,
};
use super::tables::float::{float_frame, float_moves};
use crate::q2::foundation::host::{Q2EffectEvent, Q2PresentationEvent};
use crate::q2::foundation::monsters::ai::{enemy_body, project_flash};
use crate::q2::foundation::monsters::types::{
    MonsterContext, MonsterHandler, MonsterLocomotion, Q2MonsterDefinition,
};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::support::contracts::{DeathReaction, PainReaction};

/// Stand (`stand`).
fn floater_stand(context: &mut MonsterContext) {
    if context.game.random() <= 0.5 {
        context.set_move("floater_move_stand1", false);
    } else {
        context.set_move("floater_move_stand2", false);
    }
}

/// Run (`run`).
fn floater_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("floater_move_stand1", false);
    } else {
        context.set_move("floater_move_run", false);
    }
}

/// Melee (`melee`).
fn floater_melee(context: &mut MonsterContext) {
    if context.game.random() < 0.5 {
        context.set_move("floater_move_attack3", false);
    } else {
        context.set_move("floater_move_attack2", false);
    }
}

/// Initialize (`initialize`).
fn floater_initialize(context: &mut MonsterContext) {
    monster_loop_sound(context, "floater/fltsrch1.wav");
    floater_stand(context);
}

/// Pain (`pain`).
fn floater_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
    let first = ((context.game.random() * 3.0).floor() + 1.0) as i32 % 3 == 0;
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if first {
            "floater/fltpain1.wav"
        } else {
            "floater/fltpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    context.set_move(
        if first {
            "floater_move_pain1"
        } else {
            "floater_move_pain2"
        },
        false,
    );
}

/// Die (`die`).
fn floater_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    monster_explode(context, "floater/fltdeth1.wav");
}

/// Fire blaster (`floater_fire_blaster`).
fn floater_fire_blaster(context: &mut MonsterContext) {
    let Some((start, direction)) = monster_shot(context, 82, 0.0) else {
        return;
    };
    let frame = context.entity().frame;
    let effects = if frame == float_frame::ATTAK104 || frame == float_frame::ATTAK107 {
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
        effects,
        false,
        Mod::BLASTER,
    );
    monster_muzzle(context, 82, direction, start);
}

/// Wham (`floater_wham`).
fn floater_wham(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    context.game.sound(&actor, "floater/fltatck3.wav", 1, 1.0, 1.0);
    let damage = 5.0 + (context.game.random() * 6.0).floor();
    fire_hit(actor, &mut *context.game, vec3(80.0, 0.0, 0.0), damage, -50.0);
}

/// Zap (`floater_zap`).
fn floater_zap(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let enemy_body = enemy_body(context);
    let enemy = context.entity().enemy.clone();
    let (Some(enemy_body), Some(enemy)) = (enemy_body, enemy) else {
        return;
    };
    let origin = project_flash(context, vec3(18.5, -0.9, 10.0), None);
    let self_origin = context.game.body_of(actor.clone()).origin;
    let direction = sub3(enemy_body.origin, self_origin);
    context.game.sound(&actor, "floater/fltatck2.wav", 1, 1.0, 1.0);
    context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:splash".to_string(),
        origin,
        direction,
        count: 32,
        color: 1,
    }));
    let damage = 5.0 + (context.game.random() * 6.0).floor();
    context.game.damage(
        enemy,
        actor.clone(),
        Some(actor),
        damage,
        -10.0,
        direction,
        enemy_body.origin,
        vec3(0.0, 0.0, 0.0),
        0,
        4,
        None,
    );
}

/// Floater definition (`floaterDefinition`).
pub fn floater_definition() -> Q2MonsterDefinition {
    // Original activate has 30 rows for 31 frames and is never selected
    // by the game.
    let moves: Vec<_> = float_moves()
        .into_iter()
        .filter(|animation| animation.name != "floater_move_activate")
        .collect();
    let mut definition = Q2MonsterDefinition::new(
        "monster_floater",
        "floater",
        "models/monsters/float/tris.md2",
        200.0,
        -80.0,
        300.0,
        Bounds {
            min: Vec3 {
                x: -24.0,
                y: -24.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 24.0,
                y: 24.0,
                z: 32.0,
            },
        },
        1.0,
        "floater_move_stand1",
        moves,
        MonsterHandler::Callback(floater_stand),
        move_handler("floater_move_walk"),
        MonsterHandler::Callback(floater_run),
        move_handler("floater_move_attack1"),
        floater_die,
    );
    definition.locomotion = Some(MonsterLocomotion::Fly);
    definition.melee = Some(MonsterHandler::Callback(floater_melee));
    definition.sight = Some(sound_handler("floater/fltsght1.wav", 2, 1.0));
    definition.idle = Some(sound_handler("floater/fltidle1.wav", 2, 2.0));
    definition.pain = Some(floater_pain);
    definition.initialize = Some(MonsterHandler::Callback(floater_initialize));
    definition.callbacks = HashMap::from([
        (
            "floater_run".to_string(),
            MonsterHandler::Callback(floater_run),
        ),
        (
            "floater_dead".to_string(),
            MonsterHandler::Callback(finish_corpse_default),
        ),
        (
            "floater_fire_blaster".to_string(),
            MonsterHandler::Callback(floater_fire_blaster),
        ),
        (
            "floater_wham".to_string(),
            MonsterHandler::Callback(floater_wham),
        ),
        (
            "floater_zap".to_string(),
            MonsterHandler::Callback(floater_zap),
        ),
    ]);
    definition
}
