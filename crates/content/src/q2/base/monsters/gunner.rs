//! Gunner monster (`src/content/q2/base/monsters/gunner.ts`).
//!
//! Quake II m_gunner.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::identity::ActorId;

use super::common::{
    HUMANOID_BOUNDS, alive_enemy, begin_death, damaged_skin, finish_corpse_default, forward_shot,
    monster_muzzle, monster_shot, move_handler, sound_handler,
};
use super::tables::gunner::{gunner_frame, gunner_moves};
use crate::q2::foundation::monsters::ai::{set_duck, target_distance, visible};
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{DeathReaction, PainReaction, TraceResult};

/// Run (`run`).
pub fn gunner_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("gunner_move_stand", false);
    } else {
        context.set_move("gunner_move_run", false);
    }
}

/// Throw a grenade (`grenade`).
fn gunner_grenade(context: &mut MonsterContext) {
    let frame = context.entity().frame;
    let flash = if frame == gunner_frame::ATTAK105 {
        53usize
    } else if frame == gunner_frame::ATTAK108 {
        54
    } else if frame == gunner_frame::ATTAK111 {
        55
    } else {
        56
    };
    let (start, direction) = forward_shot(context, flash);
    let fire_grenade = context.weapons.fire_grenade;
    let actor = context.actor().clone();
    fire_grenade(
        actor,
        &mut *context.game,
        start,
        direction,
        50.0,
        600.0,
        2.5,
        90.0,
        false,
        false,
        true,
        None,
    );
    monster_muzzle(context, flash as i32, direction, start);
}

/// Attack (`attack`).
fn gunner_attack(context: &mut MonsterContext) {
    if target_distance(context) < 80.0 {
        context.set_move("gunner_move_attack_chain", false);
    } else if context.game.random() <= 0.5 {
        context.set_move("gunner_move_attack_grenade", false);
    } else {
        context.set_move("gunner_move_attack_chain", false);
    }
}

/// Pain (`pain`).
fn gunner_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let actor = context.actor().clone();
    let path = if context.game.random() < 0.5 {
        "gunner/gunpain2.wav"
    } else {
        "gunner/gunpain1.wav"
    };
    context.game.sound(&actor, path, 2, 1.0, 1.0);
    if context.game.options.skill == 3 {
        return;
    }
    context.set_move(
        if reaction.damage <= 10.0 {
            "gunner_move_pain3"
        } else if reaction.damage <= 25.0 {
            "gunner_move_pain2"
        } else {
            "gunner_move_pain1"
        },
        false,
    );
}

/// Die (`die`).
fn gunner_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    begin_death(context, reaction, "gunner/death1.wav", "gunner_move_death", 2, 4);
}

/// Dodge (`dodge`).
fn gunner_dodge(
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
    context.set_move("gunner_move_duck", false);
}

/// Fidget (`gunner_fidget`).
fn gunner_fidget(context: &mut MonsterContext) {
    if !context.state().stand_ground && context.game.random() <= 0.05 {
        context.set_move("gunner_move_fidget", false);
    }
}

/// Duck down (`gunner_duck_down`).
fn gunner_duck_down(context: &mut MonsterContext) {
    if context.state().ducked {
        return;
    }
    if context.game.options.skill >= 2 && context.game.random() > 0.5 {
        gunner_grenade(context);
    }
    set_duck(context, true);
    let now = context.game.host.now();
    context.state_mut().pause_time = now + 1.0;
}

/// Duck hold (`gunner_duck_hold`).
fn gunner_duck_hold(context: &mut MonsterContext) {
    let hold = context.game.host.now() < context.state().pause_time;
    context.state_mut().hold_frame = hold;
}

/// Duck up (`gunner_duck_up`).
fn gunner_duck_up(context: &mut MonsterContext) {
    set_duck(context, false);
}

/// Chain fire (`GunnerFire`).
fn gunner_fire(context: &mut MonsterContext) {
    let flash = 45 + context.entity().frame - gunner_frame::ATTAK216;
    let Some((start, direction)) = monster_shot(context, flash as usize, -0.2) else {
        return;
    };
    let fire_bullet = context.weapons.fire_bullet;
    let actor = context.actor().clone();
    fire_bullet(
        actor,
        &mut *context.game,
        start,
        direction,
        3.0,
        4.0,
        300.0,
        500.0,
        0,
    );
    monster_muzzle(context, flash, direction, start);
}

/// Refire chain (`gunner_refire_chain`).
fn gunner_refire_chain(context: &mut MonsterContext) {
    if alive_enemy(context) && visible(context, None) && context.game.random() <= 0.5 {
        context.set_move("gunner_move_fire_chain", false);
    } else {
        context.set_move("gunner_move_endfire_chain", false);
    }
}

/// Gunner definition (`gunnerDefinition`).
pub fn gunner_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_gunner",
        "gunner",
        "models/monsters/gunner/tris.md2",
        175.0,
        -70.0,
        200.0,
        HUMANOID_BOUNDS,
        1.0,
        "gunner_move_stand",
        gunner_moves(),
        move_handler("gunner_move_stand"),
        move_handler("gunner_move_walk"),
        MonsterHandler::Callback(gunner_run),
        MonsterHandler::Callback(gunner_attack),
        gunner_die,
    );
    definition.sight = Some(sound_handler("gunner/sight1.wav", 2, 1.0));
    definition.search = Some(sound_handler("gunner/gunsrch1.wav", 2, 1.0));
    definition.pain = Some(gunner_pain);
    definition.dodge = Some(gunner_dodge);
    definition.callbacks = HashMap::from([
        (
            "gunner_stand".to_string(),
            move_handler("gunner_move_stand"),
        ),
        (
            "gunner_run".to_string(),
            MonsterHandler::Callback(gunner_run),
        ),
        (
            "gunner_dead".to_string(),
            MonsterHandler::Callback(finish_corpse_default),
        ),
        (
            "gunner_idlesound".to_string(),
            sound_handler("gunner/gunidle1.wav", 2, 2.0),
        ),
        (
            "gunner_opengun".to_string(),
            sound_handler("gunner/gunatck1.wav", 2, 2.0),
        ),
        (
            "gunner_fidget".to_string(),
            MonsterHandler::Callback(gunner_fidget),
        ),
        (
            "gunner_duck_down".to_string(),
            MonsterHandler::Callback(gunner_duck_down),
        ),
        (
            "gunner_duck_hold".to_string(),
            MonsterHandler::Callback(gunner_duck_hold),
        ),
        (
            "gunner_duck_up".to_string(),
            MonsterHandler::Callback(gunner_duck_up),
        ),
        (
            "GunnerGrenade".to_string(),
            MonsterHandler::Callback(gunner_grenade),
        ),
        (
            "GunnerFire".to_string(),
            MonsterHandler::Callback(gunner_fire),
        ),
        (
            "gunner_fire_chain".to_string(),
            move_handler("gunner_move_fire_chain"),
        ),
        (
            "gunner_refire_chain".to_string(),
            MonsterHandler::Callback(gunner_refire_chain),
        ),
    ]);
    definition
}
