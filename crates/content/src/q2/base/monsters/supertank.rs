//! Supertank monster (`src/content/q2/base/monsters/supertank.ts`).
//!
//! Quake II m_supertank.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::math::{Bounds, Vec3, normalize3, sub3, vec3};

use super::boss_common::boss_explode;
use super::common::{
    damaged_skin, finish_corpse, monster_muzzle, monster_shot, move_handler, sound_handler,
};
use super::tables::supertank::{supertank_frame, supertank_moves};
use crate::q2::foundation::monsters::ai::{angles_vectors, enemy_eye, target_distance, visible};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Run (`run`).
fn supertank_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("supertank_move_stand", true);
    } else {
        context.set_move("supertank_move_run", true);
    }
}

/// Attack (`attack`).
fn supertank_attack(context: &mut MonsterContext) {
    if target_distance(context) <= 160.0 || context.game.random() < 0.3 {
        context.set_move("supertank_move_attack1", true);
    } else {
        context.set_move("supertank_move_attack2", true);
    }
}

/// Search (`search`).
fn supertank_search(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let path = if context.game.random() < 0.5 {
        "bosstank/btkunqv1.wav"
    } else {
        "bosstank/btkunqv2.wav"
    };
    context.game.sound(&actor, path, 2, 1.0, 1.0);
}

/// Pain (`pain`).
fn supertank_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time
        || reaction.damage <= 25.0 && context.game.random() < 0.2
    {
        return;
    }
    let frame = context.entity().frame;
    if context.game.options.skill >= 2
        && frame >= supertank_frame::ATTAK2_1
        && frame <= supertank_frame::ATTAK2_14
    {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
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
            "supertank_move_pain1"
        } else if reaction.damage <= 25.0 {
            "supertank_move_pain2"
        } else {
            "supertank_move_pain3"
        },
        false,
    );
}

/// Die (`die`).
fn supertank_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
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
    context.set_move("supertank_move_death", true);
}

/// Dead (`supertank_dead`).
fn supertank_dead(context: &mut MonsterContext) {
    finish_corpse(
        context,
        Bounds {
            min: Vec3 {
                x: -60.0,
                y: -60.0,
                z: 0.0,
            },
            max: Vec3 {
                x: 60.0,
                y: 60.0,
                z: 72.0,
            },
        },
    );
}

/// Reattack (`supertank_reattack1`).
fn supertank_reattack1(context: &mut MonsterContext) {
    if visible(context, None) && context.game.random() < 0.9 {
        context.set_move("supertank_move_attack1", true);
    } else {
        context.set_move("supertank_move_end_attack1", true);
    }
}

/// Rocket (`supertankRocket`).
fn supertank_rocket(context: &mut MonsterContext) {
    let frame = context.entity().frame;
    let flash = if frame == supertank_frame::ATTAK2_8 {
        70usize
    } else if frame == supertank_frame::ATTAK2_11 {
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

/// Machine gun (`supertankMachineGun`).
fn supertank_machine_gun(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let frame = context.entity().frame;
    let flash = 64 + frame - supertank_frame::ATTAK1_1;
    let body = context.game.body_of(actor);
    // Source ignores body pitch for these six gun offsets.
    let axes = angles_vectors(vec3(0.0, body.angles.y, 0.0));
    let offset = muzzle_offset(context.game.options.edition, flash as usize);
    let start = vec3(
        body.origin.x + axes.forward.x * offset.x + axes.right.x * offset.y,
        body.origin.y + axes.forward.y * offset.x + axes.right.y * offset.y,
        body.origin.z + offset.z,
    );
    let direction = enemy_eye(context)
        .map(|eye| normalize3(sub3(eye, start)))
        .unwrap_or(axes.forward);
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

/// Supertank definition (`supertankDefinition`).
pub fn supertank_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_supertank",
        "supertank",
        "models/monsters/boss1/tris.md2",
        1500.0,
        -500.0,
        800.0,
        Bounds {
            min: Vec3 {
                x: -64.0,
                y: -64.0,
                z: 0.0,
            },
            max: Vec3 {
                x: 64.0,
                y: 64.0,
                z: 112.0,
            },
        },
        1.0,
        "supertank_move_stand",
        supertank_moves(),
        move_handler("supertank_move_stand"),
        move_handler("supertank_move_forward"),
        MonsterHandler::Callback(supertank_run),
        MonsterHandler::Callback(supertank_attack),
        supertank_die,
    );
    definition.search = Some(MonsterHandler::Callback(supertank_search));
    definition.pain = Some(supertank_pain);
    definition.callbacks = HashMap::from([
        (
            "supertank_run".to_string(),
            MonsterHandler::Callback(supertank_run),
        ),
        (
            "TreadSound".to_string(),
            sound_handler("bosstank/btkengn1.wav", 4, 1.0),
        ),
        (
            "BossExplode".to_string(),
            MonsterHandler::Callback(boss_explode),
        ),
        (
            "supertank_dead".to_string(),
            MonsterHandler::Callback(supertank_dead),
        ),
        (
            "supertank_reattack1".to_string(),
            MonsterHandler::Callback(supertank_reattack1),
        ),
        (
            "supertankRocket".to_string(),
            MonsterHandler::Callback(supertank_rocket),
        ),
        (
            "supertankMachineGun".to_string(),
            MonsterHandler::Callback(supertank_machine_gun),
        ),
    ]);
    definition
}
