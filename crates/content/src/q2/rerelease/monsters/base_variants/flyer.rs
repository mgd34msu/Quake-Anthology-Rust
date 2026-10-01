//! Rerelease flyer (`src/content/q2/rerelease/monsters/base-variants/flyer.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, normalize3, scale3, sub3, vec3};

use super::super::common::{monster_flash, reacts_to_pain, rerelease_random};
use super::super::tables::flyer::flyer_moves;
use crate::q2::base::monsters::common::{monster_loop_sound, monster_shot, move_handler, sound_handler};
use crate::q2::base::monsters::flyer::flyer_definition;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2EffectEvent, Q2GameServices, Q2PresentationEvent};
use crate::q2::foundation::monsters::ai::{enemy_body, health, target_distance, visible};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::types::{
    MonsterAttackState, MonsterContext, MonsterHandler, MonsterLocomotion, Q2MonsterDefinition,
};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::support::contracts::{DeathReaction, PainReaction, TouchContact};

/// Flight (`flight`).
fn flyer_flight(context: &mut MonsterContext, melee: bool) {
    let state = context.state_mut();
    if melee {
        state.fly_pinned = false;
        state.fly_position_time = 0.0;
    }
    state.fly_thrusters = melee;
    state.fly_acceleration = if melee { 20.0 } else { 15.0 };
    state.fly_speed = if melee { 210.0 } else { 165.0 };
    state.fly_min_distance = if melee { 0.0 } else { 45.0 };
    state.fly_max_distance = if melee { 10.0 } else { 200.0 };
}

/// Run (`run`).
fn rerelease_flyer_run(context: &mut MonsterContext) {
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "flyer_move_stand"
        } else {
            "flyer_move_run"
        },
        true,
    );
}

/// Fire (`fire`).
fn flyer_fire(context: &mut MonsterContext, flash: usize) {
    let Some((start, direction)) = monster_shot(context, flash, 0.0) else {
        return;
    };
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let effects = if frame % 4 == 0 { 64 } else { 0 };
    let fire_blaster = context.weapons.fire_blaster;
    fire_blaster(
        actor,
        &mut *context.game,
        start,
        direction,
        1.0,
        1000.0,
        effects,
        false,
        Mod::BLASTER,
    );
    monster_flash(context, flash as i32, start, direction);
}

/// Fire left (`flyer_fireleft`).
fn flyer_fire_left(context: &mut MonsterContext) {
    flyer_fire(context, 58);
}

/// Fire right (`flyer_fireright`).
fn flyer_fire_right(context: &mut MonsterContext) {
    flyer_fire(context, 59);
}

/// Slash (`slash`).
fn flyer_slash(context: &mut MonsterContext, right: bool) {
    let actor = context.actor().clone();
    let bounds = context.game.body_of(actor.clone()).bounds;
    let aim = vec3(80.0, if right { bounds.max.x } else { bounds.min.x }, 0.0);
    let fire_hit = context.weapons.fire_hit;
    if !fire_hit(actor.clone(), &mut *context.game, aim, 5.0, 0.0) {
        let now = context.game.host.now();
        context.state_mut().melee_time = now + 1.5;
    }
    context.game.sound(&actor, "flyer/flyatck2.wav", 1, 1.0, 1.0);
}

/// Slash left (`flyer_slash_left`).
fn flyer_slash_left(context: &mut MonsterContext) {
    flyer_slash(context, false);
}

/// Slash right (`flyer_slash_right`).
fn flyer_slash_right(context: &mut MonsterContext) {
    flyer_slash(context, true);
}

/// Touch (`touch`).
fn flyer_touch(actor: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if !game.monsters.states.contains_key(&actor) || !game.monsters.states.contains_key(&contact.other) {
        return;
    }
    let other = game.monsters.states.get(&contact.other).expect("flyer touch other");
    if !other.alternate_fly || other.locomotion != MonsterLocomotion::Fly {
        return;
    }
    let now = game.host.now();
    if game.monsters.states.get(&actor).expect("flyer touch self").duck_wait >= now {
        return;
    }
    let state = game.monsters.states.get_mut(&actor).expect("flyer touch self");
    state.duck_wait = now + 1.0;
    state.fly_thrusters = false;
    let origin = game.body_of(actor.clone()).origin;
    let other_origin = game.body_of(contact.other.clone()).origin;
    let direction = normalize3(sub3(origin, other_origin));
    let mut moved = game.body_of(actor.clone());
    moved.velocity = scale3(direction, 500.0);
    game.write_body(actor, &moved, true);
    game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:splash".to_string(),
        origin,
        direction,
        count: 32,
        color: 1,
    }));
}

/// Initialize (`initialize`).
fn rerelease_flyer_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let state = context.state_mut();
    state.alternate_fly = true;
    state.fly_buzzard = true;
    context.game.require_entity_mut(&actor).touch = Some(flyer_touch);
    flyer_flight(context, false);
    monster_loop_sound(context, "flyer/flyidle1.wav");
}

/// Attack (`attack`).
fn rerelease_flyer_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let enemy = enemy_body(context);
    let range = target_distance(context);
    context.state_mut().attack_state = MonsterAttackState::Straight;
    if enemy.is_some() && visible(context, None) && range <= 225.0 && {
        let threshold = range / 225.0 * 0.35;
        context.game.random() > threshold
    } {
        context.set_move("flyer_move_start_melee", true);
        flyer_flight(context, true);
    } else {
        context.set_move("flyer_move_attack2", true);
    }
    if !context.state().fly_pinned
        && rerelease_random(context).integer_max(2) != 0
        && enemy.is_some()
        && visible(context, None)
    {
        context.state_mut().fly_pinned = true;
        let position_time = context.state().fly_position_time;
        context.state_mut().fly_position_time = position_time + 1.7;
        let body = context.game.body_of(actor);
        let random = context.game.random();
        let reposition = rerelease_random(context).integer_max(2) != 0;
        let enemy_origin = enemy.expect("flyer attack enemy").origin;
        let ideal = if reposition {
            add3(body.origin, scale3(body.velocity, random as f32))
        } else {
            add3(context.state().fly_ideal_position, enemy_origin)
        };
        context.state_mut().fly_ideal_position = ideal;
    }
}

/// Melee (`melee`).
fn rerelease_flyer_melee(context: &mut MonsterContext) {
    context.set_move("flyer_move_start_melee", true);
    flyer_flight(context, true);
}

/// Pain (`pain`).
fn rerelease_flyer_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.game.require_entity(&actor).max_health;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { 1 } else { 0 };
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let choice = rerelease_random(context).integer_max(3);
    context.game.sound(
        &actor,
        if choice == 1 {
            "flyer/flypain2.wav"
        } else {
            "flyer/flypain1.wav"
        },
        2,
        1.0,
        1.0,
    );
    if !reacts_to_pain(context) {
        return;
    }
    flyer_flight(context, false);
    context.set_move(&format!("flyer_move_pain{}", choice + 1), true);
}

/// Die (`die`).
fn rerelease_flyer_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "flyer/flydeth1.wav", 2, 1.0, 1.0);
    let origin = context.game.body_of(actor.clone()).origin;
    context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:explosion1".to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    context.game.require_entity_mut(&actor).skin /= 2;
    for _ in 0..2 {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_metal/tris.md2",
            55.0,
            Q2GibOptions::default(),
        );
    }
    for _ in 0..2 {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_meat/tris.md2",
            55.0,
            Q2GibOptions::default(),
        );
    }
    throw_gib(
        actor.clone(),
        &mut *context.game,
        "models/monsters/flyer/gibs/base.md2",
        55.0,
        Q2GibOptions {
            skinned: true,
            ..Q2GibOptions::default()
        },
    );
    for part in ["gun", "wing"] {
        for _ in 0..2 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                &format!("models/monsters/flyer/gibs/{part}.md2"),
                55.0,
                Q2GibOptions {
                    skinned: true,
                    ..Q2GibOptions::default()
                },
            );
        }
    }
    throw_gib(
        actor.clone(),
        &mut *context.game,
        "models/monsters/flyer/gibs/head.md2",
        55.0,
        Q2GibOptions {
            skinned: true,
            head: true,
            ..Q2GibOptions::default()
        },
    );
    context.game.require_entity_mut(&actor).touch = None;
    context.state_mut().dead = true;
    context.state_mut().gibbed = true;
}

/// Check melee (`flyer_check_melee`).
fn flyer_check_melee(context: &mut MonsterContext) {
    if target_distance(context) <= 80.0 && context.state().melee_time <= context.game.host.now() {
        context.set_move("flyer_move_loop_melee", true);
        return;
    }
    context.set_move("flyer_move_end_melee", true);
    flyer_flight(context, false);
}

/// Create the rerelease flyer definition (`createRereleaseFlyerDefinition`).
pub fn create_rerelease_flyer_definition() -> Q2MonsterDefinition {
    let mut definition = flyer_definition();
    definition.moves = flyer_moves()
        .into_iter()
        .filter(|animation| animation.name != "flyer_move_kamikaze")
        .collect();
    definition.bounds.min = vec3(-16.0, -16.0, -24.0);
    definition.bounds.max = vec3(16.0, 16.0, 16.0);
    definition.view_height = Some(12);
    definition.run = MonsterHandler::Callback(rerelease_flyer_run);
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks.touch.insert("flyer_touch", flyer_touch);
    definition.source_callbacks = Some(source_callbacks);
    definition.initialize = Some(MonsterHandler::Callback(rerelease_flyer_initialize));
    definition.attack = MonsterHandler::Callback(rerelease_flyer_attack);
    definition.melee = Some(MonsterHandler::Callback(rerelease_flyer_melee));
    definition.pain = Some(rerelease_flyer_pain);
    definition.die = rerelease_flyer_die;
    definition.callbacks.clear();
    for (name, handler) in [
        ("flyer_run", MonsterHandler::Callback(rerelease_flyer_run)),
        ("flyer_pop_blades", sound_handler("flyer/flyatck1.wav", 2, 1.0)),
        ("flyer_loop_melee", move_handler("flyer_move_loop_melee")),
        ("flyer_fireleft", MonsterHandler::Callback(flyer_fire_left)),
        ("flyer_fireright", MonsterHandler::Callback(flyer_fire_right)),
        ("flyer_slash_left", MonsterHandler::Callback(flyer_slash_left)),
        ("flyer_slash_right", MonsterHandler::Callback(flyer_slash_right)),
        ("flyer_check_melee", MonsterHandler::Callback(flyer_check_melee)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
