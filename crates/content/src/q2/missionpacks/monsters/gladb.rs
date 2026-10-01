//! Gladb monster (`src/content/q2/missionpacks/monsters/gladb.ts`).
//!
//! Quake II xatrix/m_gladb.c. ZeniMax Media, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::math::{Bounds, Vec3, normalize3, sub3, vec3};

use super::power_armor::{PowerArmorKind, monster_power_armor, restore_monster_power_armor};
use super::tables::xatrix_gladb::gladb_moves;
use super::types::mission_weapons;
use crate::q2::base::monsters::common::{begin_death, damaged_skin, finish_corpse_default, move_handler, sound_handler};
use crate::q2::foundation::monsters::ai::{enemy_eye, project_flash, target_distance};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{DeathReaction, PainReaction};

/// Run (`run`).
fn gladb_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("gladb_move_stand", true);
    } else {
        context.set_move("gladb_move_run", true);
    }
}

/// Fire plasma (`fire`).
fn gladb_fire(context: &mut MonsterContext) {
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, 61), None);
    let target = context.state().blind_fire_target;
    let direction = normalize3(sub3(target, start));
    let weapons = mission_weapons(context.game);
    let actor = context.actor().clone();
    weapons.fire_plasma(actor, &mut *context.game, start, direction, 100.0, 725.0, 60.0, 60.0);
}

/// Initialize (`initialize`).
fn gladb_initialize(context: &mut MonsterContext) {
    monster_power_armor(context, PowerArmorKind::Shield, 400.0);
}

/// Attack (`attack`).
fn gladb_attack(context: &mut MonsterContext) {
    let eye = enemy_eye(context);
    let Some(eye) = eye else { return };
    if target_distance(context) <= 112.0 {
        return;
    }
    let actor = context.actor().clone();
    context.game.sound(&actor, "weapons/plasshot.wav", 1, 1.0, 1.0);
    context.state_mut().blind_fire_target = eye;
    context.set_move("gladb_move_attack_gun", true);
}

/// Pain (`pain`).
fn gladb_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    damaged_skin(context);
    let actor = context.actor().clone();
    let airborne = context.game.body_of(actor).velocity.z > 100.0;
    if context.game.host.now() < context.state().pain_time {
        if airborne && context.state().current_move.name == "gladb_move_pain" {
            context.set_move("gladb_move_pain_air", true);
        }
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let actor = context.actor().clone();
    let path = if context.game.random() < 0.5 {
        "gladiator/pain.wav"
    } else {
        "gladiator/gldpain2.wav"
    };
    context.game.sound(&actor, path, 2, 1.0, 1.0);
    context.set_move(
        if airborne {
            "gladb_move_pain_air"
        } else {
            "gladb_move_pain"
        },
        false,
    );
}

/// Die (`die`).
fn gladb_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    begin_death(context, reaction, "gladiator/glddeth2.wav", "gladb_move_death", 2, 4);
}

/// Melee (`GladbMelee`).
fn gladb_melee(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let side = context.game.body_of(actor.clone()).bounds.min.x;
    let damage = 20.0 + (context.game.random() * 5.0).floor();
    let hit = fire_hit(
        actor.clone(),
        &mut *context.game,
        vec3(80.0, side, -4.0),
        damage,
        300.0,
    );
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

/// Gun check (`gladbGun_check`).
fn gladb_gun_check(context: &mut MonsterContext) {
    if context.game.options.skill == 3 {
        gladb_fire(context);
    }
}

/// Create a gladb definition (`createGladbDefinition`).
pub fn create_gladb_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_gladb",
        "gladb",
        "models/monsters/gladb/tris.md2",
        800.0,
        -175.0,
        350.0,
        Bounds {
            min: Vec3 { x: -32.0, y: -32.0, z: -24.0 },
            max: Vec3 { x: 32.0, y: 32.0, z: 64.0 },
        },
        1.0,
        "gladb_move_stand",
        gladb_moves(),
        move_handler("gladb_move_stand"),
        move_handler("gladb_move_walk"),
        MonsterHandler::Callback(gladb_run),
        MonsterHandler::Callback(gladb_attack),
        gladb_die,
    );
    definition.melee = Some(move_handler("gladb_move_attack_melee"));
    definition.sight = Some(sound_handler("gladiator/sight.wav", 2, 1.0));
    definition.idle = Some(sound_handler("gladiator/gldidle1.wav", 2, 2.0));
    definition.search = Some(sound_handler("gladiator/gldsrch1.wav", 2, 1.0));
    definition.pain = Some(gladb_pain);
    definition.initialize = Some(MonsterHandler::Callback(gladb_initialize));
    definition.restore = Some(MonsterHandler::Callback(restore_monster_power_armor));
    definition.callbacks = HashMap::from([
        (
            "gladb_run".to_string(),
            MonsterHandler::Callback(gladb_run),
        ),
        (
            "gladb_dead".to_string(),
            MonsterHandler::Callback(finish_corpse_default),
        ),
        (
            "gladb_cleaver_swing".to_string(),
            sound_handler("gladiator/melee1.wav", 1, 1.0),
        ),
        (
            "GladbMelee".to_string(),
            MonsterHandler::Callback(gladb_melee),
        ),
        ("gladbGun".to_string(), MonsterHandler::Callback(gladb_fire)),
        (
            "gladbGun_check".to_string(),
            MonsterHandler::Callback(gladb_gun_check),
        ),
    ]);
    definition
}
