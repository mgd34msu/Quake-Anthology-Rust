//! Rerelease floater (`src/content/q2/rerelease/monsters/base-variants/floater.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::math::{sub3, vec3};

use super::super::common::{monster_flash, reacts_to_pain, rerelease_random};
use super::super::tables::float::float_moves;
use crate::q2::base::monsters::common::{monster_loop_sound, monster_shot};
use crate::q2::base::monsters::floater::floater_definition;
use crate::q2::foundation::host::{Q2EffectEvent, Q2PresentationEvent};
use crate::q2::foundation::monsters::ai::{enemy_body, health, project_flash};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::types::{MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::support::contracts::{DeathReaction, PainReaction};

/// Stand (`stand`).
fn rerelease_floater_stand(context: &mut MonsterContext) {
    let current = context.state().current_move.name.clone();
    let disguised = current == "floater_move_disguise";
    let first = context.game.random() <= 0.5;
    context.set_move(
        if disguised {
            "floater_move_disguise"
        } else if first {
            "floater_move_stand1"
        } else {
            "floater_move_stand2"
        },
        true,
    );
}

/// Run (`run`).
fn rerelease_floater_run(context: &mut MonsterContext) {
    let current = context.state().current_move.name.clone();
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if current == "floater_move_disguise" {
            "floater_move_pop"
        } else if stand_ground {
            "floater_move_stand1"
        } else {
            "floater_move_run"
        },
        true,
    );
}

/// Initialize (`initialize`).
fn rerelease_floater_initialize(context: &mut MonsterContext) {
    let state = context.state_mut();
    state.alternate_fly = true;
    state.fly_thrusters = false;
    state.fly_acceleration = 10.0;
    state.fly_speed = 100.0;
    state.fly_min_distance = 20.0;
    state.fly_max_distance = 200.0;
    monster_loop_sound(context, "floater/fltsrch1.wav");
    let actor = context.actor().clone();
    if context.game.require_entity(&actor).spawnflags & 8 != 0 {
        context.set_move("floater_move_disguise", true);
    } else {
        rerelease_floater_stand(context);
    }
}

/// Attack (`attack`).
fn rerelease_floater_attack(context: &mut MonsterContext) {
    if context.game.random() > 0.5 {
        context.state_mut().attack_state = MonsterAttackState::Straight;
        context.set_move("floater_move_attack1", true);
        return;
    }
    if context.game.random() <= 0.5 {
        let lefty = context.state().lefty;
        context.state_mut().lefty = !lefty;
    }
    context.state_mut().attack_state = MonsterAttackState::Sliding;
    context.set_move("floater_move_attack1a", true);
}

/// Pain (`pain`).
fn rerelease_floater_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.game.require_entity(&actor).max_health;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { 1 } else { 0 };
    let current = context.state().current_move.name.clone();
    if context.game.host.now() < context.state().pain_time
        || current == "floater_move_disguise"
        || current == "floater_move_pop"
    {
        return;
    }
    let first = rerelease_random(context).integer_max(3) == 0;
    context.game.sound(
        &actor,
        if first {
            "floater/fltpain1.wav"
        } else {
            "floater/fltpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if !reacts_to_pain(context) {
        return;
    }
    context.set_move(
        if first {
            "floater_move_pain1"
        } else {
            "floater_move_pain2"
        },
        true,
    );
}

/// Die (`die`).
fn rerelease_floater_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "floater/fltdeth1.wav", 2, 1.0, 1.0);
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
    for _ in 0..3 {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_meat/tris.md2",
            55.0,
            Q2GibOptions::default(),
        );
    }
    for part in ["piece", "gun", "base"] {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            &format!("models/monsters/float/gibs/{part}.md2"),
            55.0,
            Q2GibOptions {
                skinned: true,
                ..Q2GibOptions::default()
            },
        );
    }
    throw_gib(
        actor.clone(),
        &mut *context.game,
        "models/monsters/float/gibs/jar.md2",
        55.0,
        Q2GibOptions {
            skinned: true,
            head: true,
            ..Q2GibOptions::default()
        },
    );
    context.state_mut().dead = true;
    context.state_mut().gibbed = true;
}

/// Fire blaster (`floater_fire_blaster`).
fn floater_fire_blaster(context: &mut MonsterContext) {
    let Some((start, direction)) = monster_shot(context, 82, 0.0) else {
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
    monster_flash(context, 82, start, direction);
}

/// Wham (`floater_wham`).
fn floater_wham(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "floater/fltatck3.wav", 1, 1.0, 1.0);
    let damage = f64::from(rerelease_random(context).integer_range(5, 11));
    let fire_hit = context.weapons.fire_hit;
    if !fire_hit(actor, &mut *context.game, vec3(80.0, 0.0, 0.0), damage, -50.0) {
        let now = context.game.host.now();
        context.state_mut().melee_time = now + 3.0;
    }
}

/// Zap (`floater_zap`).
fn floater_zap(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let Some(enemy_id) = context.entity().enemy.clone() else {
        return;
    };
    let direction = sub3(enemy.origin, context.game.body_of(actor.clone()).origin);
    let origin = project_flash(context, vec3(18.5, -0.9, 10.0), None);
    context.game.sound(&actor, "floater/fltatck2.wav", 1, 1.0, 1.0);
    context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:splash".to_string(),
        origin,
        direction,
        count: 32,
        color: 1,
    }));
    let damage = f64::from(rerelease_random(context).integer_range(5, 11));
    context.game.damage(
        enemy_id,
        actor.clone(),
        Some(actor),
        damage,
        -10.0,
        direction,
        enemy.origin,
        vec3(0.0, 0.0, 0.0),
        4,
        0,
        None,
    );
}

/// Create the rerelease floater definition (`rereleaseFloaterDefinition`).
pub fn rerelease_floater_definition() -> Q2MonsterDefinition {
    let mut definition = floater_definition();
    definition.moves = float_moves();
    definition.bounds.min = vec3(-24.0, -24.0, -24.0);
    definition.bounds.max = vec3(24.0, 24.0, 48.0);
    definition.stand = MonsterHandler::Callback(rerelease_floater_stand);
    definition.run = MonsterHandler::Callback(rerelease_floater_run);
    definition.initialize = Some(MonsterHandler::Callback(rerelease_floater_initialize));
    definition.attack = MonsterHandler::Callback(rerelease_floater_attack);
    definition.pain = Some(rerelease_floater_pain);
    definition.die = rerelease_floater_die;
    for (name, handler) in [
        ("floater_run", MonsterHandler::Callback(rerelease_floater_run)),
        ("floater_fire_blaster", MonsterHandler::Callback(floater_fire_blaster)),
        ("floater_wham", MonsterHandler::Callback(floater_wham)),
        ("floater_zap", MonsterHandler::Callback(floater_zap)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
