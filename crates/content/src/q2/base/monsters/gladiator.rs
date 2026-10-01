//! Gladiator monster (`src/content/q2/base/monsters/gladiator.ts`).
//!
//! Quake II m_gladiator.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::math::{normalize3, sub3, vec3, Bounds, Vec3};

use super::common::{begin_death, damaged_skin, finish_corpse_default, monster_muzzle, move_handler, sound_handler};
use super::tables::gladiator::gladiator_moves;
use crate::q2::foundation::monsters::ai::{enemy_eye, project_flash, target_distance};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{DeathReaction, PainReaction};

/// Run (`run`).
fn gladiator_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("gladiator_move_stand", true);
    } else {
        context.set_move("gladiator_move_run", true);
    }
}

/// Attack (`attack`).
fn gladiator_attack(context: &mut MonsterContext) {
    if target_distance(context) <= 112.0 {
        return;
    }
    let Some(eye) = enemy_eye(context) else { return };
    context.state_mut().blind_fire_target = eye;
    let actor = context.actor().clone();
    context.game.sound(&actor, "gladiator/railgun.wav", 1, 1.0, 1.0);
    context.set_move("gladiator_move_attack_gun", true);
}

/// Pain (`pain`).
fn gladiator_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    damaged_skin(context);
    let actor = context.actor().clone();
    let airborne = context.game.body_of(actor.clone()).velocity.z > 100.0;
    if context.game.host.now() < context.state().pain_time {
        if airborne && context.state().current_move.name == "gladiator_move_pain" {
            context.set_move("gladiator_move_pain_air", true);
        }
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let path = if context.game.random() < 0.5 {
        "gladiator/pain.wav"
    } else {
        "gladiator/gldpain2.wav"
    };
    context.game.sound(&actor, path, 2, 1.0, 1.0);
    if context.game.options.skill == 3 {
        return;
    }
    context.set_move(
        if airborne {
            "gladiator_move_pain_air"
        } else {
            "gladiator_move_pain"
        },
        false,
    );
}

/// Die (`die`).
fn gladiator_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    begin_death(
        context,
        reaction,
        "gladiator/glddeth2.wav",
        "gladiator_move_death",
        2,
        4,
    );
}

/// Cleaver melee (`GaldiatorMelee`).
fn gladiator_melee_hit(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let side = context.game.body_of(actor.clone()).bounds.min.x;
    let damage = 20.0 + (context.game.random() * 5.0).floor();
    let hit = fire_hit(actor.clone(), &mut *context.game, vec3(80.0, side, -4.0), damage, 300.0);
    context.game.sound(
        &actor,
        if hit {
            "gladiator/melee2.wav"
        } else {
            "gladiator/melee3.wav"
        },
        0,
        1.0,
        1.0,
    );
}

/// Railgun attack (`GladiatorGun`).
fn gladiator_gun(context: &mut MonsterContext) {
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, 61), None);
    let target = context.state().blind_fire_target;
    let direction = normalize3(sub3(target, start));
    let fire_rail = context.weapons.fire_rail;
    let actor = context.actor().clone();
    fire_rail(actor, &mut *context.game, start, direction, 50.0, 100.0);
    monster_muzzle(context, 61, direction, start);
}

/// Gladiator definition (`gladiatorDefinition`).
pub fn gladiator_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_gladiator",
        "gladiator",
        "models/monsters/gladiatr/tris.md2",
        400.0,
        -175.0,
        400.0,
        Bounds {
            min: Vec3 {
                x: -32.0,
                y: -32.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 32.0,
                y: 32.0,
                z: 64.0,
            },
        },
        1.0,
        "gladiator_move_stand",
        gladiator_moves(),
        move_handler("gladiator_move_stand"),
        move_handler("gladiator_move_walk"),
        MonsterHandler::Callback(gladiator_run),
        MonsterHandler::Callback(gladiator_attack),
        gladiator_die,
    );
    definition.melee = Some(move_handler("gladiator_move_attack_melee"));
    definition.sight = Some(sound_handler("gladiator/sight.wav", 2, 1.0));
    definition.idle = Some(sound_handler("gladiator/gldidle1.wav", 2, 2.0));
    definition.search = Some(sound_handler("gladiator/gldsrch1.wav", 2, 2.0));
    definition.pain = Some(gladiator_pain);
    definition.callbacks = HashMap::from([
        ("gladiator_run".to_string(), MonsterHandler::Callback(gladiator_run)),
        (
            "gladiator_dead".to_string(),
            MonsterHandler::Callback(finish_corpse_default),
        ),
        (
            "gladiator_cleaver_swing".to_string(),
            sound_handler("gladiator/melee1.wav", 1, 1.0),
        ),
        (
            "GaldiatorMelee".to_string(),
            MonsterHandler::Callback(gladiator_melee_hit),
        ),
        ("GladiatorGun".to_string(), MonsterHandler::Callback(gladiator_gun)),
    ]);
    definition
}
