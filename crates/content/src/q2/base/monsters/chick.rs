//! Chick monster (`src/content/q2/base/monsters/chick.ts`).
//!
//! Quake II m_chick.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, vec3};

use super::common::{
    alive_enemy, begin_death, damaged_skin, finish_corpse_default, monster_muzzle, monster_shot,
    move_handler, sound_handler, standard_gib,
};
use super::tables::chick::chick_moves;
use crate::q2::foundation::monsters::ai::{set_duck, target_distance, visible};
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{DeathReaction, PainReaction, TraceResult};

/// Run (`run`).
pub(crate) fn chick_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("chick_move_stand", false);
        return;
    }
    let current = context.state().current_move.name.clone();
    if current == "chick_move_walk" || current == "chick_move_start_run" {
        context.set_move("chick_move_run", false);
    } else {
        context.set_move("chick_move_start_run", false);
    }
}

/// Pain (`pain`).
fn chick_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let r = context.game.random();
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if r < 0.33 {
            "chick/chkpain1.wav"
        } else if r < 0.66 {
            "chick/chkpain2.wav"
        } else {
            "chick/chkpain3.wav"
        },
        2,
        1.0,
        1.0,
    );
    if context.game.options.skill == 3 {
        return;
    }
    context.set_move(
        if reaction.damage <= 10.0 {
            "chick_move_pain1"
        } else if reaction.damage <= 25.0 {
            "chick_move_pain2"
        } else {
            "chick_move_pain3"
        },
        false,
    );
}

/// Die (`die`).
fn chick_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    if standard_gib(
        context,
        reaction,
        2,
        4,
        "models/objects/gibs/head2/tris.md2",
        1.0,
    ) || context.state().dead
    {
        return;
    }
    let first = context.game.random() < 0.5;
    begin_death(
        context,
        reaction,
        if first {
            "chick/chkdeth1.wav"
        } else {
            "chick/chkdeth2.wav"
        },
        if first {
            "chick_move_death1"
        } else {
            "chick_move_death2"
        },
        2,
        4,
    );
}

/// Dodge (`dodge`).
fn chick_dodge(
    context: &mut MonsterContext,
    attacker: &ActorId,
    _eta: f64,
    _trace: Option<&TraceResult>,
    _direct: bool,
) {
    if context.game.random() > 0.25 {
        return;
    }
    if context.entity().enemy.is_none() {
        context.entity_mut().enemy = Some(attacker.clone());
    }
    context.set_move("chick_move_duck", false);
}

/// Moan (`ChickMoan`).
fn chick_moan(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let path = if context.game.random() < 0.5 {
        "chick/chkidle1.wav"
    } else {
        "chick/chkidle2.wav"
    };
    context.game.sound(&actor, path, 2, 1.0, 2.0);
}

/// Fidget (`chick_fidget`).
fn chick_fidget(context: &mut MonsterContext) {
    if !context.state().stand_ground && context.game.random() <= 0.3 {
        context.set_move("chick_move_fidget", false);
    }
}

/// Duck down (`chick_duck_down`).
fn chick_duck_down(context: &mut MonsterContext) {
    if context.state().ducked {
        return;
    }
    set_duck(context, true);
    let now = context.game.host.now();
    context.state_mut().pause_time = now + 1.0;
}

/// Duck hold (`chick_duck_hold`).
fn chick_duck_hold(context: &mut MonsterContext) {
    let hold = context.game.host.now() < context.state().pause_time;
    context.state_mut().hold_frame = hold;
}

/// Duck up (`chick_duck_up`).
fn chick_duck_up(context: &mut MonsterContext) {
    set_duck(context, false);
}

/// Rocket (`ChickRocket`).
fn chick_rocket(context: &mut MonsterContext) {
    let Some((start, direction)) = monster_shot(context, 57, 0.0) else {
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
    monster_muzzle(context, 57, direction, start);
}

/// Slash (`ChickSlash`).
fn chick_slash(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    context.game.sound(&actor, "chick/chkatck3.wav", 1, 1.0, 1.0);
    let side = context.game.body_of(actor.clone()).bounds.min.x;
    let damage = 10.0 + (context.game.random() * 6.0).floor();
    fire_hit(
        actor,
        &mut *context.game,
        vec3(80.0, side, 10.0),
        damage,
        100.0,
    );
}

/// Rerocket (`chick_rerocket`).
fn chick_rerocket(context: &mut MonsterContext) {
    if alive_enemy(context)
        && target_distance(context) >= 80.0
        && visible(context, None)
        && context.game.random() <= 0.6
    {
        context.set_move("chick_move_attack1", false);
    } else {
        context.set_move("chick_move_end_attack1", false);
    }
}

/// Reslash (`chick_reslash`).
fn chick_reslash(context: &mut MonsterContext) {
    if alive_enemy(context) && target_distance(context) < 80.0 && context.game.random() <= 0.9 {
        context.set_move("chick_move_slash", false);
    } else {
        context.set_move("chick_move_end_slash", false);
    }
}

/// Chick definition (`chickDefinition`).
pub fn chick_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_chick",
        "chick",
        "models/monsters/bitch/tris.md2",
        175.0,
        -70.0,
        200.0,
        Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: 0.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 56.0,
            },
        },
        1.0,
        "chick_move_stand",
        chick_moves(),
        move_handler("chick_move_stand"),
        move_handler("chick_move_walk"),
        MonsterHandler::Callback(chick_run),
        move_handler("chick_move_start_attack1"),
        chick_die,
    );
    definition.melee = Some(move_handler("chick_move_start_slash"));
    definition.sight = Some(sound_handler("chick/chksght1.wav", 2, 1.0));
    definition.pain = Some(chick_pain);
    definition.dodge = Some(chick_dodge);
    definition.callbacks = HashMap::from([
        (
            "chick_stand".to_string(),
            move_handler("chick_move_stand"),
        ),
        ("chick_run".to_string(), MonsterHandler::Callback(chick_run)),
        (
            "chick_dead".to_string(),
            MonsterHandler::Callback(finish_corpse_default),
        ),
        (
            "ChickMoan".to_string(),
            MonsterHandler::Callback(chick_moan),
        ),
        (
            "chick_fidget".to_string(),
            MonsterHandler::Callback(chick_fidget),
        ),
        (
            "chick_duck_down".to_string(),
            MonsterHandler::Callback(chick_duck_down),
        ),
        (
            "chick_duck_hold".to_string(),
            MonsterHandler::Callback(chick_duck_hold),
        ),
        (
            "chick_duck_up".to_string(),
            MonsterHandler::Callback(chick_duck_up),
        ),
        (
            "Chick_PreAttack1".to_string(),
            sound_handler("chick/chkatck1.wav", 2, 1.0),
        ),
        (
            "ChickReload".to_string(),
            sound_handler("chick/chkatck5.wav", 2, 1.0),
        ),
        (
            "ChickRocket".to_string(),
            MonsterHandler::Callback(chick_rocket),
        ),
        (
            "ChickSlash".to_string(),
            MonsterHandler::Callback(chick_slash),
        ),
        (
            "chick_attack1".to_string(),
            move_handler("chick_move_attack1"),
        ),
        (
            "chick_slash".to_string(),
            move_handler("chick_move_slash"),
        ),
        (
            "chick_rerocket".to_string(),
            MonsterHandler::Callback(chick_rerocket),
        ),
        (
            "chick_reslash".to_string(),
            MonsterHandler::Callback(chick_reslash),
        ),
    ]);
    definition
}
