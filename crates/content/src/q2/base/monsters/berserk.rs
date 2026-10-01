//! Berserk monster (`src/content/q2/base/monsters/berserk.ts`).
//!
//! Quake II m_berserk.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::math::vec3;

use super::common::{begin_death, damaged_skin, finish_corpse_default, move_handler, sound_handler, HUMANOID_BOUNDS};
use super::tables::berserk::berserk_moves;
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{DeathReaction, PainReaction};

/// Run (`run`).
pub fn berserk_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("berserk_move_stand", true);
    } else {
        context.set_move("berserk_move_run1", true);
    }
}

/// Melee (`melee`).
fn berserk_melee(context: &mut MonsterContext) {
    if context.game.random() < 0.5 {
        context.set_move("berserk_move_attack_spike", true);
    } else {
        context.set_move("berserk_move_attack_club", true);
    }
}

/// Pain (`pain`).
fn berserk_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let actor = context.actor().clone();
    context.game.sound(&actor, "berserk/berpain2.wav", 2, 1.0, 1.0);
    if context.game.options.skill == 3 {
        return;
    }
    if reaction.damage < 20.0 || context.game.random() < 0.5 {
        context.set_move("berserk_move_pain1", true);
    } else {
        context.set_move("berserk_move_pain2", true);
    }
}

/// Die (`die`).
fn berserk_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let animation = if reaction.pain.damage >= 50.0 {
        "berserk_move_death1"
    } else {
        "berserk_move_death2"
    };
    begin_death(context, reaction, "berserk/berdeth2.wav", animation, 2, 4);
}

/// Fidget (`berserk_fidget`).
fn berserk_fidget(context: &mut MonsterContext) {
    if context.state().stand_ground || context.game.random() > 0.15 {
        return;
    }
    context.set_move("berserk_move_stand_fidget", true);
    let actor = context.actor().clone();
    context.game.sound(&actor, "berserk/beridle1.wav", 1, 1.0, 2.0);
}

/// Spike attack (`berserk_attack_spike`).
fn berserk_attack_spike(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let damage = 15.0 + (context.game.random() * 6.0).floor();
    fire_hit(actor, &mut *context.game, vec3(80.0, 0.0, -24.0), damage, 400.0);
}

/// Club attack (`berserk_attack_club`).
fn berserk_attack_club(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let side = context.game.body_of(actor.clone()).bounds.min.x;
    let damage = 5.0 + (context.game.random() * 6.0).floor();
    fire_hit(actor, &mut *context.game, vec3(80.0, side, -4.0), damage, 400.0);
}

/// Unused source animation (`berserk_strike`).
fn berserk_strike(_context: &mut MonsterContext) {}

/// Berserk definition (`berserkDefinition`).
pub fn berserk_definition() -> Q2MonsterDefinition {
    let stand = move_handler("berserk_move_stand");
    let mut definition = Q2MonsterDefinition::new(
        "monster_berserk",
        "berserk",
        "models/monsters/berserk/tris.md2",
        240.0,
        -60.0,
        250.0,
        HUMANOID_BOUNDS,
        1.0,
        "berserk_move_stand",
        berserk_moves(),
        stand,
        move_handler("berserk_move_walk"),
        MonsterHandler::Callback(berserk_run),
        MonsterHandler::Callback(berserk_melee),
        berserk_die,
    );
    definition.melee = Some(MonsterHandler::Callback(berserk_melee));
    definition.sight = Some(sound_handler("berserk/sight.wav", 2, 1.0));
    definition.search = Some(sound_handler("berserk/bersrch1.wav", 2, 1.0));
    definition.pain = Some(berserk_pain);
    definition.callbacks = HashMap::from([
        ("berserk_stand".to_string(), move_handler("berserk_move_stand")),
        ("berserk_run".to_string(), MonsterHandler::Callback(berserk_run)),
        (
            "berserk_dead".to_string(),
            MonsterHandler::Callback(finish_corpse_default),
        ),
        ("berserk_fidget".to_string(), MonsterHandler::Callback(berserk_fidget)),
        ("berserk_swing".to_string(), sound_handler("berserk/attack.wav", 1, 1.0)),
        (
            "berserk_attack_spike".to_string(),
            MonsterHandler::Callback(berserk_attack_spike),
        ),
        (
            "berserk_attack_club".to_string(),
            MonsterHandler::Callback(berserk_attack_club),
        ),
        ("berserk_strike".to_string(), MonsterHandler::Callback(berserk_strike)),
    ]);
    definition
}
