//! Rogue jumping monsters (`src/content/q2/missionpacks/monsters/rogue-jumpers.ts`).
//!
//! Original Rogue Berserk, Mutant and Parasite movement callbacks.
//! GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, scale3};

use super::rogue_common::{
    rogue_blocked_check_shot, rogue_monster_dodge, rogue_parasite_drain_trace,
};
use super::state::rogue_state;
use super::tables::rogue_berserk::berserk_moves;
use super::tables::rogue_mutant::mutant_moves;
use super::tables::rogue_parasite::parasite_moves;
use crate::q2::base::monsters::berserk::{berserk_definition, berserk_run};
use crate::q2::base::monsters::common::damaged_skin;
use crate::q2::base::monsters::mutant::{MutantSource, create_mutant_definition};
use crate::q2::base::monsters::parasite::parasite_definition;
use crate::q2::foundation::monsters::ai::{angles_vectors, enemy_body, finish_dodge};
use crate::q2::foundation::monsters::perception::default_check_attack;
use crate::q2::foundation::monsters::types::{
    MonsterContext, MonsterHandler, Q2MonsterDefinition,
};
use crate::q2::rerelease::monsters::common::{
    JumpNavigation, JumpResult, blocked_check_jump, blocked_check_platform,
    monster_jump_finished,
};
use crate::q2::support::contracts::{PainReaction, TraceHit, TraceResult};

/// Jump impulse (`jumpImpulse`).
fn jump_impulse(
    context: &mut MonsterContext,
    forward_speed: f64,
    up_speed: f64,
    timed: bool,
) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let axes = angles_vectors(body.angles);
    if timed {
        let now = context.game.host.now();
        context.entity_mut().timestamp = now;
    }
    let mut moved = body;
    moved.velocity = add3(
        moved.velocity,
        add3(
            scale3(axes.forward, forward_speed as f32),
            scale3(axes.up, up_speed as f32),
        ),
    );
    context.game.write_body(actor, &moved, true);
}

/// Jump wait (`jumpWait`).
fn jump_wait(context: &mut MonsterContext, timed: bool) {
    let actor = context.actor().clone();
    let landed = context.game.body_of(actor).ground.is_some()
        || timed && monster_jump_finished(context);
    let next = context.entity().frame + if landed { 1 } else { 0 };
    context.state_mut().next_frame = next;
}

/// Jump (`jump`).
fn jump(context: &mut MonsterContext, up: &str, down: &str, dodge: bool) {
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    if dodge {
        finish_dodge(context);
    }
    let actor = context.actor().clone();
    let above = enemy.origin.z > context.game.body_of(actor).origin.z;
    context.set_move(if above { up } else { down }, false);
}

/// Berserk run (`berserkRun`).
fn rogue_berserk_run(context: &mut MonsterContext) {
    finish_dodge(context);
    berserk_run(context);
}

/// Berserk melee (`berserkMelee`).
fn rogue_berserk_melee(context: &mut MonsterContext) {
    finish_dodge(context);
    let spike = (context.game.random() * 32768.0).floor() as i64 & 1 == 0;
    context.set_move(
        if spike {
            "berserk_move_attack_spike"
        } else {
            "berserk_move_attack_club"
        },
        false,
    );
}

/// Berserk sidestep (`berserkSidestep`).
fn rogue_berserk_sidestep(context: &mut MonsterContext) {
    let current = context.state().current_move.name.clone();
    if current == "berserk_move_jump"
        || current == "berserk_move_jump2"
        || current == "berserk_move_run1"
    {
        return;
    }
    context.set_move("berserk_move_run1", false);
}

/// Berserk dodge (`dodge`).
fn rogue_berserk_dodge(
    context: &mut MonsterContext,
    attacker: &ActorId,
    eta: f64,
    trace: Option<&TraceResult>,
    _direct: bool,
) {
    rogue_monster_dodge(context, attacker, eta, trace, None, Some(rogue_berserk_sidestep));
}

/// Berserk blocked (`blocked`).
fn rogue_berserk_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    if blocked_check_jump(context, distance, 256.0, 40.0, true, JumpNavigation::None)
        != JumpResult::None
    {
        jump(context, "berserk_move_jump2", "berserk_move_jump", true);
        return true;
    }
    blocked_check_platform(context, distance)
}

/// Berserk pain (`pain`).
fn rogue_berserk_pain(context: &mut MonsterContext, reaction: &PainReaction) {
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
    finish_dodge(context);
    let light = reaction.damage < 20.0 || context.game.random() < 0.5;
    context.set_move(
        if light {
            "berserk_move_pain1"
        } else {
            "berserk_move_pain2"
        },
        false,
    );
}

/// Berserk jump now (`berserk_jump_now`).
fn berserk_jump_now(context: &mut MonsterContext) {
    jump_impulse(context, 100.0, 300.0, true);
}

/// Berserk second jump now (`berserk_jump2_now`).
fn berserk_jump2_now(context: &mut MonsterContext) {
    jump_impulse(context, 150.0, 400.0, true);
}

/// Berserk jump wait land (`berserk_jump_wait_land`).
fn berserk_jump_wait_land(context: &mut MonsterContext) {
    jump_wait(context, true);
}

/// Mutant blocked (`blocked`).
fn rogue_mutant_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    if blocked_check_jump(context, distance, 256.0, 68.0, true, JumpNavigation::None)
        != JumpResult::None
    {
        jump(context, "mutant_move_jump_up", "mutant_move_jump_down", false);
        return true;
    }
    blocked_check_platform(context, distance)
}

/// Mutant jump up (`mutant_jump_up`).
fn mutant_jump_up(context: &mut MonsterContext) {
    jump_impulse(context, 200.0, 450.0, false);
}

/// Mutant jump down (`mutant_jump_down`).
fn mutant_jump_down(context: &mut MonsterContext) {
    jump_impulse(context, 100.0, 300.0, false);
}

/// Mutant jump wait land (`mutant_jump_wait_land`).
fn mutant_jump_wait_land(context: &mut MonsterContext) {
    jump_wait(context, false);
}

/// Parasite blocked (`blocked`).
fn rogue_parasite_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    let chance = 0.25 + 0.05 * f64::from(context.game.options.skill);
    if rogue_blocked_check_shot(context, chance) {
        return true;
    }
    if blocked_check_jump(context, distance, 256.0, 68.0, true, JumpNavigation::None)
        != JumpResult::None
    {
        jump(context, "parasite_move_jump_up", "parasite_move_jump_down", false);
        return true;
    }
    blocked_check_platform(context, distance)
}

/// Parasite check attack (`checkAttack`).
fn rogue_parasite_check_attack(context: &mut MonsterContext) -> bool {
    if !default_check_attack(context) {
        return false;
    }
    let Some(trace) = rogue_parasite_drain_trace(context) else {
        return false;
    };
    let enemy = context.entity().enemy.clone();
    let drain = matches!(&trace.hit, TraceHit::Actor { actor } if Some(actor) == enemy.as_ref());
    if !drain {
        let actor = context.actor().clone();
        rogue_state(&mut *context.game, &actor).blocked = true;
        context.attack();
        rogue_state(&mut *context.game, &actor).blocked = false;
    }
    // The original C falls off its successful trace branch; retain its accepted M_CheckAttack result.
    true
}

/// Parasite jump up (`parasite_jump_up`).
fn parasite_jump_up(context: &mut MonsterContext) {
    jump_impulse(context, 200.0, 450.0, true);
}

/// Parasite jump down (`parasite_jump_down`).
fn parasite_jump_down(context: &mut MonsterContext) {
    jump_impulse(context, 100.0, 300.0, true);
}

/// Parasite jump wait land (`parasite_jump_wait_land`).
fn parasite_jump_wait_land(context: &mut MonsterContext) {
    jump_wait(context, true);
}

/// Create rogue jumping monsters (`createRogueJumpingMonsters`).
pub fn create_rogue_jumping_monsters() -> Vec<Q2MonsterDefinition> {
    let mut berserk = berserk_definition();
    berserk.moves = berserk_moves();
    berserk.run = MonsterHandler::Callback(rogue_berserk_run);
    berserk.melee = Some(MonsterHandler::Callback(rogue_berserk_melee));
    berserk.attack = MonsterHandler::Callback(rogue_berserk_melee);
    berserk.dodge = Some(rogue_berserk_dodge);
    berserk.blocked = Some(rogue_berserk_blocked);
    berserk.pain = Some(rogue_berserk_pain);
    berserk.callbacks.insert(
        "berserk_run".to_string(),
        MonsterHandler::Callback(rogue_berserk_run),
    );
    berserk.callbacks.insert(
        "monster_done_dodge".to_string(),
        MonsterHandler::Callback(finish_dodge),
    );
    berserk.callbacks.insert(
        "berserk_jump_now".to_string(),
        MonsterHandler::Callback(berserk_jump_now),
    );
    berserk.callbacks.insert(
        "berserk_jump2_now".to_string(),
        MonsterHandler::Callback(berserk_jump2_now),
    );
    berserk.callbacks.insert(
        "berserk_jump_wait_land".to_string(),
        MonsterHandler::Callback(berserk_jump_wait_land),
    );

    let mut mutant = create_mutant_definition(MutantSource::Rogue);
    mutant.moves = mutant_moves();
    mutant.blocked = Some(rogue_mutant_blocked);
    mutant.callbacks.insert(
        "mutant_jump_up".to_string(),
        MonsterHandler::Callback(mutant_jump_up),
    );
    mutant.callbacks.insert(
        "mutant_jump_down".to_string(),
        MonsterHandler::Callback(mutant_jump_down),
    );
    mutant.callbacks.insert(
        "mutant_jump_wait_land".to_string(),
        MonsterHandler::Callback(mutant_jump_wait_land),
    );

    let mut parasite = parasite_definition();
    parasite.moves = parasite_moves();
    parasite.blocked = Some(rogue_parasite_blocked);
    parasite.check_attack = Some(rogue_parasite_check_attack);
    parasite.callbacks.insert(
        "parasite_jump_up".to_string(),
        MonsterHandler::Callback(parasite_jump_up),
    );
    parasite.callbacks.insert(
        "parasite_jump_down".to_string(),
        MonsterHandler::Callback(parasite_jump_down),
    );
    parasite.callbacks.insert(
        "parasite_jump_wait_land".to_string(),
        MonsterHandler::Callback(parasite_jump_wait_land),
    );

    vec![berserk, mutant, parasite]
}
