//! Heat-seeking chick variant (`src/content/q2/missionpacks/monsters/chick-heat.ts`).
//!
//! Quake II xatrix/m_chick.c heat-seeking variant. GPL-2.0-or-later.

use crate::q2::base::monsters::chick::chick_definition;
use crate::q2::base::monsters::common::{damaged_skin, monster_muzzle, monster_shot};
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::PainReaction;

use super::types::mission_weapons;

/// Initialize (`initialize`).
fn chick_heat_initialize(context: &mut MonsterContext) {
    context.entity_mut().skin = 3;
}

/// Pain (`pain`).
fn chick_heat_pain(context: &mut MonsterContext, reaction: &PainReaction) {
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

/// Rocket (`ChickRocket`).
fn chick_heat_rocket(context: &mut MonsterContext) {
    let Some((start, direction)) = monster_shot(context, 57, 0.0) else {
        return;
    };
    let actor = context.actor().clone();
    if context.entity().skin > 1 {
        let weapons = mission_weapons(context.game);
        weapons.fire_heat_rocket(
            actor,
            &mut *context.game,
            start,
            direction,
            50.0,
            500.0,
            70.0,
            50.0,
            None,
        );
    } else {
        let fire_rocket = context.weapons.fire_rocket;
        fire_rocket(actor, &mut *context.game, start, direction, 50.0, 500.0, 70.0, 50.0);
    }
    monster_muzzle(context, 57, direction, start);
}

/// Create a heat-seeking chick definition (`createChickHeatDefinition`).
pub fn create_chick_heat_definition() -> Q2MonsterDefinition {
    let mut definition = chick_definition();
    definition.classname = "monster_chick_heat".to_string();
    definition.initialize = Some(MonsterHandler::Callback(chick_heat_initialize));
    definition.pain = Some(chick_heat_pain);
    definition
        .callbacks
        .insert("ChickRocket".to_string(), MonsterHandler::Callback(chick_heat_rocket));
    definition
}
