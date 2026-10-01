//! Rerelease parasite (`src/content/q2/rerelease/monsters/base-variants/parasite.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::math::{add3, scale3, vec3};

use super::super::common::{
    blocked_check_jump, blocked_check_platform, check_gib, monster_jump_finished, reacts_to_pain, rerelease_random,
    JumpNavigation, JumpResult,
};
use super::super::tables::parasite::{parasite_frame, parasite_moves};
use super::proboscis::{proboscis_callbacks, proboscis_draw, proboscis_fire, proboscis_reset, proboscis_retract};
use crate::q2::base::monsters::parasite::parasite_definition;
use crate::q2::foundation::monsters::ai::{angles_vectors, clear_shot, corpse, health, run_ai};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::types::{MonsterAi, MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Start run (`startRun`).
fn parasite_start_run(context: &mut MonsterContext) {
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "parasite_move_stand"
        } else {
            "parasite_move_start_run"
        },
        true,
    );
}

/// Jump (`jump`).
fn parasite_jump(context: &mut MonsterContext, up: bool) {
    let actor = context.actor().clone();
    let mut moved = context.game.body_of(actor.clone());
    let axes = angles_vectors(moved.angles);
    moved.velocity = add3(
        moved.velocity,
        add3(
            scale3(axes.forward, if up { 200.0 } else { 100.0 }),
            scale3(axes.up, if up { 450.0 } else { 300.0 }),
        ),
    );
    context.game.write_body(actor, &moved, true);
}

/// Jump down (`parasite_jump_down`).
fn parasite_jump_down(context: &mut MonsterContext) {
    parasite_jump(context, false);
}

/// Jump up (`parasite_jump_up`).
fn parasite_jump_up(context: &mut MonsterContext) {
    parasite_jump(context, true);
}

/// Retract (`retract`).
fn parasite_retract(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let tip = context.game.require_entity(&actor).proboscus.clone();
    if let Some(tip) = tip {
        if context.game.require_entity(&tip).style != 2 {
            proboscis_retract(tip, &mut *context.game);
        }
    }
}

/// Run (`run`).
fn rerelease_parasite_run(context: &mut MonsterContext) {
    parasite_retract(context);
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "parasite_move_stand"
        } else {
            "parasite_move_run"
        },
        true,
    );
}

/// Initialize (`initialize`).
fn rerelease_parasite_initialize(context: &mut MonsterContext) {
    context.state_mut().yaw_speed = 30.0;
}

/// Idle (`idle`).
fn rerelease_parasite_idle(context: &mut MonsterContext) {
    if context.entity().enemy.is_none() {
        context.set_move("parasite_move_start_fidget", true);
    }
}

/// Attack (`attack`).
fn rerelease_parasite_attack(context: &mut MonsterContext) {
    if !clear_shot(context, vec3(-1.7, 0.0, 1.2)) {
        return;
    }
    parasite_retract(context);
    context.set_move("parasite_move_fire_proboscis", true);
}

/// Charge proboscis (`parasite_charge_proboscis`).
fn parasite_charge_proboscis(context: &mut MonsterContext, distance: f64) {
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let trailed = frame >= parasite_frame::BREAK01 && frame <= parasite_frame::BREAK32;
    run_ai(
        context,
        if trailed { &MonsterAi::Move } else { &MonsterAi::Charge },
        distance,
    );
    let tip = context.game.require_entity(&actor).proboscus.clone();
    let segment = tip
        .as_ref()
        .and_then(|tip| context.game.require_entity(tip).proboscus.clone());
    if let Some(segment) = segment {
        if context.game.entity(&segment).is_some() {
            proboscis_draw(segment, &mut *context.game);
        }
    }
}

/// Pain (`pain`).
fn rerelease_parasite_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.game.require_entity(&actor).max_health;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { 1 } else { 0 };
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    parasite_retract(context);
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let first = context.game.random() < 0.5;
    context.game.sound(
        &actor,
        if first {
            "parasite/parpain1.wav"
        } else {
            "parasite/parpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    if reacts_to_pain(context) {
        context.set_move("parasite_move_pain1", true);
    }
}

/// Die (`die`).
fn rerelease_parasite_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    let tip = context.game.require_entity(&actor).proboscus.clone();
    if let Some(tip) = tip {
        if context.game.require_entity(&tip).style != 2 {
            proboscis_reset(tip, &mut *context.game);
        }
    }
    if check_gib(context) {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        context.game.require_entity_mut(&actor).skin /= 2;
        let damage = reaction.pain.damage;
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/bone/tris.md2",
            damage,
            Q2GibOptions::default(),
        );
        for _ in 0..3 {
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
            "models/monsters/parasite/gibs/chest.md2",
            damage,
            Q2GibOptions {
                skinned: true,
                ..Q2GibOptions::default()
            },
        );
        for part in ["bleg", "bleg", "fleg", "fleg"] {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                &format!("models/monsters/parasite/gibs/{part}.md2"),
                damage,
                Q2GibOptions {
                    skinned: true,
                    upright: true,
                    ..Q2GibOptions::default()
                },
            );
        }
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/parasite/gibs/head.md2",
            damage,
            Q2GibOptions {
                skinned: true,
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
    context.game.sound(&actor, "parasite/pardeth1.wav", 2, 1.0, 1.0);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    context.set_move("parasite_move_death", true);
}

/// Blocked (`blocked`).
fn rerelease_parasite_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    let actor = context.actor().clone();
    let can_jump = context.game.require_entity(&actor).spawnflags & 8 == 0;
    let result = blocked_check_jump(context, distance, 256.0, 68.0, can_jump, JumpNavigation::None);
    if result != JumpResult::None {
        if result != JumpResult::Turn && context.entity().enemy.is_some() {
            context.set_move(
                if result == JumpResult::Up {
                    "parasite_move_jump_up"
                } else {
                    "parasite_move_jump_down"
                },
                true,
            );
        }
        return true;
    }
    blocked_check_platform(context, distance)
}

/// Tap (`parasite_tap`).
fn parasite_tap(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "parasite/paridle1.wav", 1, 0.75, 2.75);
}

/// Scratch (`parasite_scratch`).
fn parasite_scratch(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "parasite/paridle2.wav", 1, 0.75, 2.75);
}

/// Proboscis wait (`parasite_proboscis_wait`).
fn parasite_proboscis_wait(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    context.state_mut().next_frame = if frame == parasite_frame::DRAIN04 {
        parasite_frame::DRAIN05
    } else {
        parasite_frame::DRAIN04
    };
}

/// Proboscis pull wait (`parasite_proboscis_pull_wait`).
fn parasite_proboscis_pull_wait(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let tip = context.game.require_entity(&actor).proboscus.clone();
    let style = tip.as_ref().map(|tip| context.game.require_entity(tip).style);
    if tip.is_none() || style == Some(3) {
        context.state_mut().next_frame = parasite_frame::DRAIN14;
        return;
    }
    let frame = context.game.require_entity(&actor).frame;
    context.state_mut().next_frame = if frame == parasite_frame::DRAIN12 {
        parasite_frame::DRAIN13
    } else {
        parasite_frame::DRAIN12
    };
    if style != Some(2) {
        proboscis_retract(tip.expect("parasite tip"), &mut *context.game);
    }
}

/// Break noise (`parasite_break_noise`).
fn parasite_break_noise(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "parasite/parsrch1.wav", 2, 1.0, 1.0);
}

/// Break retract (`parasite_break_retract`).
fn parasite_break_retract(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if let Some(tip) = context.game.require_entity(&actor).proboscus.clone() {
        proboscis_retract(tip, &mut *context.game);
    }
}

/// Break sound (`parasite_break_sound`).
fn parasite_break_sound(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let first = context.game.random() < 0.5;
    context.game.sound(
        &actor,
        if first {
            "parasite/parpain1.wav"
        } else {
            "parasite/parpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
}

/// Break wait (`parasite_break_wait`).
fn parasite_break_wait(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let tip = context.game.require_entity(&actor).proboscus.clone();
    let waiting = tip
        .as_ref()
        .is_some_and(|tip| context.game.require_entity(tip).style != 3);
    if waiting {
        context.state_mut().next_frame = parasite_frame::BREAK19;
    } else if rerelease_random(context).integer_max(2) != 0 {
        context.game.sound(&actor, "parasite/paratck4.wav", 1, 1.0, 1.0);
        context.state_mut().next_frame = parasite_frame::BREAK31;
    }
}

/// Jump wait land (`parasite_jump_wait_land`).
fn parasite_jump_wait_land(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let landed = context.game.body_of(actor).ground.is_some() || monster_jump_finished(context);
    context.state_mut().next_frame = if landed { frame + 1 } else { frame };
}

/// Shrink (`parasite_shrink`).
fn parasite_shrink(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).server_flags |= 2;
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.max.z = 0.0;
    context.game.write_body(actor, &moved, true);
}

/// Dead (`parasite_dead`).
fn parasite_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.min = vec3(-16.0, -16.0, -24.0);
    moved.bounds.max = vec3(16.0, 16.0, -8.0);
    context.game.write_body(actor, &moved, true);
    corpse(context);
}

/// Create the rerelease parasite definition (`createRereleaseParasiteDefinition`).
pub fn create_rerelease_parasite_definition() -> Q2MonsterDefinition {
    let mut definition = parasite_definition();
    definition.moves = parasite_moves();
    definition.source_callbacks = Some(proboscis_callbacks());
    definition.run = MonsterHandler::Callback(parasite_start_run);
    definition.initialize = Some(MonsterHandler::Callback(rerelease_parasite_initialize));
    definition.idle = Some(MonsterHandler::Callback(rerelease_parasite_idle));
    definition.attack = MonsterHandler::Callback(rerelease_parasite_attack);
    definition
        .ai
        .insert("parasite_charge_proboscis".to_string(), parasite_charge_proboscis);
    definition.pain = Some(rerelease_parasite_pain);
    definition.die = rerelease_parasite_die;
    definition.blocked = Some(rerelease_parasite_blocked);
    for (name, handler) in [
        ("parasite_run", MonsterHandler::Callback(rerelease_parasite_run)),
        ("parasite_start_run", MonsterHandler::Callback(parasite_start_run)),
        ("parasite_tap", MonsterHandler::Callback(parasite_tap)),
        ("parasite_scratch", MonsterHandler::Callback(parasite_scratch)),
        ("parasite_fire_proboscis", MonsterHandler::Callback(proboscis_fire)),
        (
            "parasite_proboscis_wait",
            MonsterHandler::Callback(parasite_proboscis_wait),
        ),
        (
            "parasite_proboscis_pull_wait",
            MonsterHandler::Callback(parasite_proboscis_pull_wait),
        ),
        ("parasite_break_noise", MonsterHandler::Callback(parasite_break_noise)),
        (
            "parasite_break_retract",
            MonsterHandler::Callback(parasite_break_retract),
        ),
        ("parasite_break_sound", MonsterHandler::Callback(parasite_break_sound)),
        ("parasite_break_wait", MonsterHandler::Callback(parasite_break_wait)),
        ("parasite_jump_down", MonsterHandler::Callback(parasite_jump_down)),
        ("parasite_jump_up", MonsterHandler::Callback(parasite_jump_up)),
        (
            "parasite_jump_wait_land",
            MonsterHandler::Callback(parasite_jump_wait_land),
        ),
        ("parasite_shrink", MonsterHandler::Callback(parasite_shrink)),
        ("parasite_dead", MonsterHandler::Callback(parasite_dead)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
