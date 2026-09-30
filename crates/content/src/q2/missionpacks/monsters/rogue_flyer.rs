//! Rogue flyer (`src/content/q2/missionpacks/monsters/flyer.ts`).
//!
//! Original Rogue m_flyer.c behavior. ZeniMax Media, GPL-2.0-or-later.

use qa_core::math::{Bounds, Vec3, add3, scale3, sub3, vec3};

use super::rogue_common::{monster_mass, rogue_blocked_check_shot};
use super::state::RogueFlyerNext;
use super::tables::rogue_flyer::flyer_moves;
use crate::q2::base::monsters::common::{monster_explode, monster_loop_sound, move_handler};
use crate::q2::base::monsters::flyer::{flyer_definition, flyer_pain};
use crate::q2::foundation::host::{Q2EffectEvent, Q2PresentationEvent};
use crate::q2::foundation::monsters::ai::{enemy_body, target_distance};
use crate::q2::foundation::monsters::types::{
    MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition,
};
use crate::q2::support::contracts::PainReaction;

/// Run (`run`).
fn rogue_flyer_run(context: &mut MonsterContext) {
    if monster_mass(context) > 50.0 {
        context.set_move("flyer_move_kamikaze", true);
    } else if context.state().stand_ground {
        context.set_move("flyer_move_stand", true);
    } else {
        context.set_move("flyer_move_run", true);
    }
}

/// Return a slot to the carrier commander (`returnSlot`).
fn return_slot(context: &mut MonsterContext) {
    let commander = context.state().commander.clone();
    let commander_entity =
        commander.as_ref().and_then(|commander| context.game.entity(commander).cloned());
    if let Some(commander_entity) = commander_entity {
        if commander_entity.classname == "monster_carrier" {
            let id = commander_entity.actor.id().clone();
            if let Some(commander_state) = context.game.monsters.states.get_mut(&id) {
                commander_state.monster_slots += 1;
            }
        }
    }
}

/// Kamikaze explode (`kamikazeExplode`).
fn kamikaze_explode(context: &mut MonsterContext) {
    return_slot(context);
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    let body = context.game.body_of(actor.clone());
    if let (Some(enemy), Some(enemy_body)) = (enemy, enemy_body(context)) {
        context.game.damage(
            enemy,
            actor.clone(),
            Some(actor),
            50.0,
            50.0,
            sub3(enemy_body.origin, body.origin),
            body.origin,
            vec3(0.0, 0.0, 0.0),
            0,
            1,
            None,
        );
    }
    monster_explode(context, "flyer/flydeth1.wav");
}

/// Kamikaze check (`kamikazeCheck`).
fn kamikaze_check(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if !context.game.host.actors().is_live(&actor) {
        return;
    }
    let enemy = context.entity().enemy.clone();
    match enemy {
        Some(enemy) if context.game.host.actors().is_live(&enemy) => {
            context.entity_mut().goal = Some(enemy);
        }
        _ => {
            kamikaze_explode(context);
            return;
        }
    }
    if target_distance(context) < 90.0 {
        kamikaze_explode(context);
    }
}

/// Stand (`stand`).
fn rogue_flyer_stand(context: &mut MonsterContext) {
    if monster_mass(context) > 50.0 {
        rogue_flyer_run(context);
    } else {
        context.set_move("flyer_move_stand", true);
    }
}

/// Walk (`walk`).
fn rogue_flyer_walk(context: &mut MonsterContext) {
    if monster_mass(context) > 50.0 {
        rogue_flyer_run(context);
    } else {
        context.set_move("flyer_move_walk", true);
    }
}

/// Melee (`melee`).
fn rogue_flyer_melee(context: &mut MonsterContext) {
    if monster_mass(context) > 50.0 {
        rogue_flyer_run(context);
    } else {
        context.set_move("flyer_move_start_melee", true);
    }
}

/// Attack (`attack`).
fn rogue_flyer_attack(context: &mut MonsterContext) {
    if monster_mass(context) > 50.0 {
        rogue_flyer_run(context);
        return;
    }
    let skill = context.game.options.skill;
    let chance = if skill == 0 {
        0.0
    } else {
        1.0 - 0.5 / f64::from(skill)
    };
    if context.game.random() > chance {
        context.state_mut().attack_state = MonsterAttackState::Straight;
        context.set_move("flyer_move_attack2", true);
        return;
    }
    if context.game.random() <= 0.5 {
        let lefty = !context.state().lefty;
        context.state_mut().lefty = lefty;
    }
    context.state_mut().attack_state = MonsterAttackState::Sliding;
    context.set_move("flyer_move_attack3", true);
}

/// Pain (`pain`).
fn rogue_flyer_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    if monster_mass(context) == 50.0 {
        flyer_pain(context, reaction);
    }
}

/// Blocked (`blocked`).
fn rogue_flyer_blocked(context: &mut MonsterContext, _distance: f64) -> bool {
    if monster_mass(context) != 100.0 {
        let chance = 0.25 + 0.05 * f64::from(context.game.options.skill);
        return rogue_blocked_check_shot(context, chance);
    }
    kamikaze_check(context);
    let actor = context.actor().clone();
    if context.game.host.actors().is_live(&actor) {
        return_slot(context);
        let body = context.game.body_of(actor.clone());
        context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
            effect: "q2:rocket-explosion".to_string(),
            origin: add3(body.origin, scale3(body.velocity, -0.02)),
            direction: vec3(0.0, 0.0, 0.0),
            count: 1,
            color: 0,
        }));
        context.game.remove_actor(actor);
    }
    true
}

/// Set start (`flyer_setstart`).
fn flyer_setstart(context: &mut MonsterContext) {
    context.game.mission_monsters.flyer_next_move = RogueFlyerNext::Run;
    context.set_move("flyer_move_start", true);
}

/// Next move (`flyer_nextmove`).
fn flyer_nextmove(context: &mut MonsterContext) {
    if context.game.mission_monsters.flyer_next_move == RogueFlyerNext::Run {
        context.set_move("flyer_move_run", true);
    }
}

/// Kamikaze initialize (`initialize`).
fn kamikaze_initialize(context: &mut MonsterContext) {
    context.entity_mut().effects |= 16;
    // The kamikaze spawn does not apply the base flyer's jail5 correction.
    monster_loop_sound(context, "flyer/flyidle1.wav");
}

/// Create rogue flyer definitions (`createRogueFlyerDefinitions`).
pub fn create_rogue_flyer_definitions() -> Vec<Q2MonsterDefinition> {
    let mut definition = flyer_definition();
    definition.bounds = Bounds {
        min: Vec3 { x: -16.0, y: -16.0, z: -24.0 },
        max: Vec3 { x: 16.0, y: 16.0, z: 16.0 },
    };
    definition.moves = flyer_moves();
    definition.run = MonsterHandler::Callback(rogue_flyer_run);
    definition.stand = MonsterHandler::Callback(rogue_flyer_stand);
    definition.walk = MonsterHandler::Callback(rogue_flyer_walk);
    definition.melee = Some(MonsterHandler::Callback(rogue_flyer_melee));
    definition.attack = MonsterHandler::Callback(rogue_flyer_attack);
    definition.pain = Some(rogue_flyer_pain);
    definition.blocked = Some(rogue_flyer_blocked);
    definition.callbacks.insert(
        "flyer_run".to_string(),
        MonsterHandler::Callback(rogue_flyer_run),
    );
    definition.callbacks.insert(
        "flyer_kamikaze".to_string(),
        move_handler("flyer_move_kamikaze"),
    );
    definition.callbacks.insert(
        "flyer_kamikaze_check".to_string(),
        MonsterHandler::Callback(kamikaze_check),
    );
    definition.callbacks.insert(
        "flyer_setstart".to_string(),
        MonsterHandler::Callback(flyer_setstart),
    );
    definition.callbacks.insert(
        "flyer_nextmove".to_string(),
        MonsterHandler::Callback(flyer_nextmove),
    );
    let mut kamikaze = definition.clone();
    kamikaze.classname = "monster_kamikaze".to_string();
    kamikaze.kind = "kamikaze".to_string();
    kamikaze.mass = 100.0;
    kamikaze.initialize = Some(MonsterHandler::Callback(kamikaze_initialize));
    vec![definition, kamikaze]
}
