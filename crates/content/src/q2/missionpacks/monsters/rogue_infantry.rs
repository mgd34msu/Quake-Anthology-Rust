//! Rogue infantry variant (`src/content/q2/missionpacks/monsters/rogue-infantry.ts`).
//!
//! Original Rogue m_infantry.c behavior. ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, scale3};

use super::rogue_common::{
    rogue_blocked_check_shot, rogue_duck_down, rogue_duck_hold, rogue_duck_up, rogue_monster_dodge,
};
use super::tables::rogue_infantry::{infantry_frame, infantry_moves};
use crate::q2::base::monsters::common::{damaged_skin, monster_shot, HUMANOID_BOUNDS};
use crate::q2::foundation::monsters::ai::{angles_vectors, enemy_body, finish_dodge};
use crate::q2::foundation::monsters::infantry::{
    infantry_attack, infantry_callbacks, infantry_die, infantry_run, infantry_sight, infantry_stand, infantry_walk,
    machine_gun,
};
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::rerelease::monsters::common::{
    blocked_check_jump, blocked_check_platform, monster_flash, monster_jump_finished, JumpNavigation, JumpResult,
};
use crate::q2::support::contracts::{PainReaction, TraceResult};

/// Run (`run`).
fn rogue_infantry_run(context: &mut MonsterContext) {
    finish_dodge(context);
    infantry_run(context);
}

/// Whether jumping (`jumping`).
fn jumping(context: &mut MonsterContext) -> bool {
    let current = context.state().current_move.name.clone();
    current == "infantry_move_jump" || current == "infantry_move_jump2"
}

/// Whether shooting (`shooting`).
fn shooting(context: &mut MonsterContext) -> bool {
    let current = context.state().current_move.name.clone();
    current == "infantry_move_attack1" || current == "infantry_move_attack2"
}

/// Duck (`duck`).
fn rogue_infantry_duck_inner(context: &mut MonsterContext, eta: f64) {
    if jumping(context) {
        return;
    }
    if shooting(context) && context.game.options.skill != 0 {
        context.state_mut().ducked = false;
        return;
    }
    let skill = context.game.options.skill;
    let now = context.game.host.now();
    context.state_mut().duck_wait = now + eta + if skill == 0 { 1.0 } else { 0.1 * f64::from(3 - skill) };
    rogue_duck_down(context);
    context.state_mut().next_frame = infantry_frame::DUCK01;
    context.set_move("infantry_move_duck", true);
}

/// Sidestep (`sidestep`).
fn rogue_infantry_sidestep_inner(context: &mut MonsterContext) {
    if jumping(context) {
        return;
    }
    if shooting(context) && context.game.options.skill != 0 {
        context.state_mut().dodging = false;
        return;
    }
    if context.state().current_move.name != "infantry_move_run" {
        context.set_move("infantry_move_run", true);
    }
}

/// Machine gun (`machineGun`).
fn rogue_infantry_machine_gun(context: &mut MonsterContext) {
    if enemy_body(context).is_none() {
        return;
    }
    if context.entity().frame != infantry_frame::ATTAK104 {
        machine_gun(context);
        return;
    }
    let Some((start, direction)) = monster_shot(context, 26, -0.2) else {
        return;
    };
    let fire_bullet = context.weapons.fire_bullet;
    let actor = context.actor().clone();
    fire_bullet(actor, &mut *context.game, start, direction, 3.0, 4.0, 300.0, 500.0, 0);
    monster_flash(context, 26, start, direction);
}

/// Jump now (`jumpNow`).
fn jump_now(context: &mut MonsterContext, high: bool) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let axes = angles_vectors(body.angles);
    let now = context.game.host.now();
    context.entity_mut().timestamp = now;
    let mut moved = body.clone();
    moved.velocity = add3(
        body.velocity,
        add3(
            scale3(axes.forward, if high { 150.0 } else { 100.0 }),
            scale3(axes.up, if high { 400.0 } else { 300.0 }),
        ),
    );
    context.game.write_body(actor, &moved, true);
}

/// Attack (`attack`).
fn rogue_infantry_attack(context: &mut MonsterContext) {
    finish_dodge(context);
    infantry_attack(context);
}

/// Idle (`idle`).
fn rogue_infantry_idle(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "infantry/infidle1.wav", 2, 1.0, 2.0);
    context.set_move("infantry_move_fidget", true);
}

/// Pain (`pain`).
fn rogue_infantry_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    damaged_skin(context);
    let actor = context.actor().clone();
    if context.game.body_of(actor.clone()).ground.is_none() {
        return;
    }
    finish_dodge(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
    let second = ((context.game.random() * 32768.0).floor() as i64 & 1) != 0;
    context.set_move(
        if second {
            "infantry_move_pain2"
        } else {
            "infantry_move_pain1"
        },
        false,
    );
    context.game.sound(
        &actor,
        if second {
            "infantry/infpain2.wav"
        } else {
            "infantry/infpain1.wav"
        },
        2,
        1.0,
        1.0,
    );
    if context.state().ducked {
        rogue_duck_up(context);
    }
}

/// Dodge (`dodge`).
fn rogue_infantry_dodge(
    context: &mut MonsterContext,
    attacker: &ActorId,
    eta: f64,
    trace: Option<&TraceResult>,
    _direct: bool,
) {
    rogue_monster_dodge(
        context,
        attacker,
        eta,
        trace,
        Some(rogue_infantry_duck_inner),
        Some(rogue_infantry_sidestep_inner),
    );
}

/// Duck slot (`duck`).
fn rogue_infantry_duck(context: &mut MonsterContext, eta: f64) -> bool {
    rogue_infantry_duck_inner(context, eta);
    true
}

/// Blocked (`blocked`).
fn rogue_infantry_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    let chance = 0.25 + 0.05 * f64::from(context.game.options.skill);
    if rogue_blocked_check_shot(context, chance) {
        return true;
    }
    if blocked_check_jump(context, distance, 192.0, 40.0, true, JumpNavigation::None) != JumpResult::None {
        let enemy = enemy_body(context);
        if let Some(enemy) = enemy {
            finish_dodge(context);
            let actor = context.actor().clone();
            let above = enemy.origin.z > context.game.body_of(actor).origin.z;
            context.set_move(
                if above {
                    "infantry_move_jump2"
                } else {
                    "infantry_move_jump"
                },
                false,
            );
        }
        return true;
    }
    blocked_check_platform(context, distance)
}

/// Cock gun (`infantry_cock_gun`).
fn infantry_cock_gun(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "infantry/infatck3.wav", 1, 1.0, 1.0);
}

/// Fire prep (`infantry_fire_prep`).
fn infantry_fire_prep(context: &mut MonsterContext) {
    let pause = context.game.host.now() + (((context.game.random() * 32768.0).floor() as i64 & 15) + 4) as f64 * 0.1;
    context.state_mut().pause_time = pause;
}

/// Fire (`infantry_fire`).
fn infantry_fire(context: &mut MonsterContext) {
    rogue_infantry_machine_gun(context);
    let hold = context.game.host.now() < context.state().pause_time;
    context.state_mut().hold_frame = hold;
}

/// Jump now (`infantry_jump_now`).
fn infantry_jump_now(context: &mut MonsterContext) {
    jump_now(context, false);
}

/// Jump2 now (`infantry_jump2_now`).
fn infantry_jump2_now(context: &mut MonsterContext) {
    jump_now(context, true);
}

/// Jump wait land (`infantry_jump_wait_land`).
fn infantry_jump_wait_land(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let landed = context.game.body_of(actor).ground.is_some() || monster_jump_finished(context);
    let next = context.entity().frame + if landed { 1 } else { 0 };
    context.state_mut().next_frame = next;
}

/// Create the rogue infantry definition (`createRogueInfantryDefinition`).
pub fn create_rogue_infantry_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_infantry",
        "infantry",
        "models/monsters/infantry/tris.md2",
        100.0,
        -40.0,
        200.0,
        HUMANOID_BOUNDS,
        1.0,
        "infantry_move_stand",
        infantry_moves(),
        MonsterHandler::Callback(infantry_stand),
        MonsterHandler::Callback(infantry_walk),
        MonsterHandler::Callback(rogue_infantry_run),
        MonsterHandler::Callback(rogue_infantry_attack),
        infantry_die,
    );
    definition.sight = Some(MonsterHandler::Callback(infantry_sight));
    definition.idle = Some(MonsterHandler::Callback(rogue_infantry_idle));
    definition.pain = Some(rogue_infantry_pain);
    definition.dodge = Some(rogue_infantry_dodge);
    definition.duck = Some(rogue_infantry_duck);
    definition.blocked = Some(rogue_infantry_blocked);
    let mut callbacks = infantry_callbacks();
    callbacks.insert("infantry_run".to_string(), MonsterHandler::Callback(rogue_infantry_run));
    callbacks.insert(
        "InfantryMachineGun".to_string(),
        MonsterHandler::Callback(rogue_infantry_machine_gun),
    );
    callbacks.insert("monster_done_dodge".to_string(), MonsterHandler::Callback(finish_dodge));
    callbacks.insert(
        "monster_duck_down".to_string(),
        MonsterHandler::Callback(rogue_duck_down),
    );
    callbacks.insert(
        "monster_duck_hold".to_string(),
        MonsterHandler::Callback(rogue_duck_hold),
    );
    callbacks.insert("monster_duck_up".to_string(), MonsterHandler::Callback(rogue_duck_up));
    callbacks.insert(
        "infantry_cock_gun".to_string(),
        MonsterHandler::Callback(infantry_cock_gun),
    );
    callbacks.insert(
        "infantry_fire_prep".to_string(),
        MonsterHandler::Callback(infantry_fire_prep),
    );
    callbacks.insert("infantry_fire".to_string(), MonsterHandler::Callback(infantry_fire));
    callbacks.insert(
        "infantry_jump_now".to_string(),
        MonsterHandler::Callback(infantry_jump_now),
    );
    callbacks.insert(
        "infantry_jump2_now".to_string(),
        MonsterHandler::Callback(infantry_jump2_now),
    );
    callbacks.insert(
        "infantry_jump_wait_land".to_string(),
        MonsterHandler::Callback(infantry_jump_wait_land),
    );
    definition.callbacks = callbacks;
    definition
}
