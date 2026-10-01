//! Rerelease flipper (`src/content/q2/rerelease/monsters/base-variants/flipper.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::math::vec3;

use super::super::common::{check_gib, reacts_to_pain, rerelease_random};
use super::super::tables::flipper::flipper_moves;
use crate::q2::base::monsters::flipper::flipper_definition;
use crate::q2::foundation::monsters::ai::{corpse, health};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Initialize (`initialize`).
fn rerelease_flipper_initialize(context: &mut MonsterContext) {
    let state = context.state_mut();
    state.alternate_fly = true;
    state.fly_thrusters = false;
    state.fly_acceleration = 30.0;
    state.fly_speed = 110.0;
    state.fly_min_distance = 10.0;
    state.fly_max_distance = 10.0;
}

/// Pain (`pain`).
fn rerelease_flipper_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.game.require_entity(&actor).max_health;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { 1 } else { 0 };
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let first = rerelease_random(context).integer_max(2) != 0;
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
    if !reacts_to_pain(context) {
        return;
    }
    context.set_move(
        if first {
            "flipper_move_pain1"
        } else {
            "flipper_move_pain2"
        },
        true,
    );
}

/// Die (`die`).
fn rerelease_flipper_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if check_gib(context) {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        let damage = reaction.pain.damage;
        for _ in 0..2 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/bone/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
        }
        for _ in 0..2 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/sm_meat/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
        }
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/head2/tris.md2",
            damage,
            Q2GibOptions {
                head: true,
                ..Q2GibOptions::default()
            },
        );
        context.state_mut().dead = true;
        context.state_mut().gibbed = true;
        return;
    }
    if context.state().dead {
        return;
    }
    context.game.sound(&actor, "flipper/flpdeth1.wav", 2, 1.0, 1.0);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    context.game.require_entity_mut(&actor).server_flags |= 2;
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    context.set_move("flipper_move_death", true);
}

/// Dead (`flipper_dead`).
fn flipper_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    corpse(context);
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.min = vec3(-16.0, -16.0, -8.0);
    moved.bounds.max = vec3(16.0, 16.0, 8.0);
    context.game.write_body(actor, &moved, true);
}

/// Create the rerelease flipper definition (`rereleaseFlipperDefinition`).
pub fn rerelease_flipper_definition() -> Q2MonsterDefinition {
    let mut definition = flipper_definition();
    definition.moves = flipper_moves();
    definition.bounds.min = vec3(-16.0, -16.0, -8.0);
    definition.bounds.max = vec3(16.0, 16.0, 20.0);
    definition.initialize = Some(MonsterHandler::Callback(rerelease_flipper_initialize));
    definition.pain = Some(rerelease_flipper_pain);
    definition.die = rerelease_flipper_die;
    definition
        .callbacks
        .insert("flipper_dead".to_string(), MonsterHandler::Callback(flipper_dead));
    definition
}
