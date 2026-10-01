//! Insane monster (`src/content/q2/base/monsters/insane.ts`).
//!
//! Quake II m_insane.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::math::{Bounds, Vec3};

use super::common::{finish_corpse, move_handler, sound_handler, standard_gib, HUMANOID_BOUNDS};
use super::tables::insane::{insane_frame, insane_moves};
use crate::q2::foundation::monsters::ai::health;
use crate::q2::foundation::monsters::types::{
    record_at, MonsterContext, MonsterHandler, MonsterLocomotion, Q2MonsterDefinition,
};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Stand (`stand`).
fn insane_stand(context: &mut MonsterContext) {
    if context.entity().spawnflags & 8 != 0 {
        context.state_mut().stand_ground = true;
        context.set_move("insane_move_cross", true);
        return;
    }
    if context.entity().spawnflags & 20 == 20 {
        context.set_move("insane_move_down", true);
        return;
    }
    if context.game.random() < 0.5 {
        context.set_move("insane_move_stand_normal", true);
    } else {
        context.set_move("insane_move_stand_insane", true);
    }
}

/// Walk or run (`walking`).
fn insane_walking(context: &mut MonsterContext, running: bool) {
    if context.entity().spawnflags & 16 != 0 && context.entity().frame == insane_frame::CR_PAIN10 {
        context.set_move("insane_move_down", true);
        return;
    }
    if context.entity().spawnflags & 4 != 0 {
        context.set_move(
            if running {
                "insane_move_runcrawl"
            } else {
                "insane_move_crawl"
            },
            false,
        );
        return;
    }
    let normal = context.game.random() <= 0.5;
    context.set_move(
        if running {
            if normal {
                "insane_move_run_normal"
            } else {
                "insane_move_run_insane"
            }
        } else if normal {
            "insane_move_walk_normal"
        } else {
            "insane_move_walk_insane"
        },
        false,
    );
}

/// Walk (`insane_walk`).
fn insane_walk(context: &mut MonsterContext) {
    insane_walking(context, false);
}

/// Run (`insane_run`).
fn insane_run(context: &mut MonsterContext) {
    insane_walking(context, true);
}

/// Dead (`dead`).
fn insane_dead(context: &mut MonsterContext) {
    if context.entity().spawnflags & 8 == 0 {
        finish_corpse(context, super::common::CORPSE_BOUNDS);
        return;
    }
    {
        let entity = context.entity_mut();
        entity.flags |= 1;
        entity.server_flags |= 2;
    }
    context.state_mut().corpse = true;
    let actor = context.actor().clone();
    context.game.link_actor(actor.clone());
    context.game.cancel_actor(actor);
}

/// Whether crawling (`crawling`).
fn insane_crawling(context: &mut MonsterContext) -> bool {
    let frame = context.entity().frame;
    frame >= insane_frame::CRAWL1 && frame <= insane_frame::CRAWL9
        || frame >= insane_frame::STAND99 && frame <= insane_frame::STAND160
}

/// Initialize (`initialize`).
fn insane_initialize(context: &mut MonsterContext) {
    context.state_mut().good_guy = true;
    if context.entity().spawnflags & 16 != 0 {
        context.state_mut().stand_ground = true;
    }
    if context.entity().spawnflags & 8 != 0 {
        context.state_mut().locomotion = MonsterLocomotion::Fly;
        context.entity_mut().flags |= 1 | 2048;
        let actor = context.actor().clone();
        let mut body = context.game.body_of(actor.clone());
        body.bounds = Bounds {
            min: Vec3 {
                x: -16.0,
                y: 0.0,
                z: 0.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 8.0,
                z: 32.0,
            },
        };
        context.game.write_body(actor, &body, false);
    }
}

/// After spawn (`afterSpawn`).
fn insane_after_spawn(context: &mut MonsterContext) {
    if context.entity().spawnflags & 8 == 0 {
        let skin = (context.game.random() * 3.0).floor() as i32;
        context.entity_mut().skin = skin;
    }
}

/// Pain (`pain`).
fn insane_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let variant = 1 + (context.game.random() * 2.0).floor() as i32;
    let actor = context.actor().clone();
    let hp = health(&mut *context.game, Some(&actor));
    let band = if hp < 25.0 {
        25
    } else if hp < 50.0 {
        50
    } else if hp < 75.0 {
        75
    } else {
        100
    };
    context
        .game
        .sound(&actor, &format!("player/male/pain{band}_{variant}.wav"), 2, 1.0, 2.0);
    if context.game.options.skill == 3 {
        return;
    }
    if context.entity().spawnflags & 8 != 0 {
        context.set_move("insane_move_struggle_cross", true);
    } else if insane_crawling(context) {
        context.set_move("insane_move_crawl_pain", true);
    } else {
        context.set_move("insane_move_stand_pain", true);
    }
}

/// Die (`die`).
fn insane_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    if standard_gib(context, reaction, 2, 4, "models/objects/gibs/head2/tris.md2", 2.0) || context.state().dead {
        return;
    }
    let variant = 1 + (context.game.random() * 4.0).floor() as i32;
    let actor = context.actor().clone();
    context
        .game
        .sound(&actor, &format!("player/male/death{variant}.wav"), 2, 1.0, 2.0);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    if context.entity().spawnflags & 8 != 0 {
        insane_dead(context);
    } else if insane_crawling(context) {
        context.set_move("insane_move_crawl_death", true);
    } else {
        context.set_move("insane_move_stand_death", true);
    }
}

/// Scream (`insane_scream`).
fn insane_scream(context: &mut MonsterContext) {
    let variant = record_at(
        &[1, 2, 3, 4, 6, 8, 9, 10],
        (context.game.random() * 8.0).floor() as usize,
    );
    let actor = context.actor().clone();
    context
        .game
        .sound(&actor, &format!("insane/insane{variant}.wav"), 2, 1.0, 2.0);
}

/// Cross (`insane_cross`).
fn insane_cross(context: &mut MonsterContext) {
    if context.game.random() < 0.8 {
        context.set_move("insane_move_cross", true);
    } else {
        context.set_move("insane_move_struggle_cross", true);
    }
}

/// Check down (`insane_checkdown`).
fn insane_checkdown(context: &mut MonsterContext) {
    if context.entity().spawnflags & 32 != 0 || context.game.random() >= 0.3 {
        return;
    }
    if context.game.random() < 0.5 {
        context.set_move("insane_move_uptodown", true);
    } else {
        context.set_move("insane_move_jumpdown", true);
    }
}

/// Check up (`insane_checkup`).
fn insane_checkup(context: &mut MonsterContext) {
    if context.entity().spawnflags & 20 != 20 && context.game.random() < 0.5 {
        context.set_move("insane_move_downtoup", true);
    }
}

/// Insane definition (`insaneDefinition`).
pub fn insane_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "misc_insane",
        "insane",
        "models/monsters/insane/tris.md2",
        100.0,
        -50.0,
        300.0,
        HUMANOID_BOUNDS,
        1.0,
        "insane_move_stand_normal",
        insane_moves(),
        MonsterHandler::Callback(insane_stand),
        MonsterHandler::Callback(insane_walk),
        MonsterHandler::Callback(insane_run),
        MonsterHandler::Callback(insane_stand),
        insane_die,
    );
    definition.pain = Some(insane_pain);
    definition.initialize = Some(MonsterHandler::Callback(insane_initialize));
    definition.after_spawn = Some(MonsterHandler::Callback(insane_after_spawn));
    definition.callbacks = HashMap::from([
        ("insane_stand".to_string(), MonsterHandler::Callback(insane_stand)),
        ("insane_walk".to_string(), MonsterHandler::Callback(insane_walk)),
        ("insane_run".to_string(), MonsterHandler::Callback(insane_run)),
        ("insane_dead".to_string(), MonsterHandler::Callback(insane_dead)),
        ("insane_onground".to_string(), move_handler("insane_move_down")),
        ("insane_fist".to_string(), sound_handler("insane/insane11.wav", 2, 2.0)),
        ("insane_shake".to_string(), sound_handler("insane/insane5.wav", 2, 2.0)),
        ("insane_moan".to_string(), sound_handler("insane/insane7.wav", 2, 2.0)),
        ("insane_scream".to_string(), MonsterHandler::Callback(insane_scream)),
        ("insane_cross".to_string(), MonsterHandler::Callback(insane_cross)),
        (
            "insane_checkdown".to_string(),
            MonsterHandler::Callback(insane_checkdown),
        ),
        ("insane_checkup".to_string(), MonsterHandler::Callback(insane_checkup)),
    ]);
    definition
}
