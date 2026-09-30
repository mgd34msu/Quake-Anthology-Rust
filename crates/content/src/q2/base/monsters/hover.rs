//! Hover monster (`src/content/q2/base/monsters/hover.ts`).
//!
//! Quake II m_hover.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, vec3};

use super::common::{
    alive_enemy, begin_death, damaged_skin, monster_loop_sound, monster_muzzle, monster_shot,
    move_handler, sound_handler, standard_gib,
};
use super::tables::hover::{hover_frame, hover_moves};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2EffectEvent, Q2GameServices, Q2MotionKind, Q2PresentationEvent};
use crate::q2::foundation::monsters::ai::visible;
use crate::q2::foundation::monsters::types::{
    MonsterContext, MonsterHandler, MonsterLocomotion, Q2MonsterDefinition,
};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::support::contracts::{DeathReaction, PainReaction};

/// Run (`run`).
fn hover_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("hover_move_stand", false);
    } else {
        context.set_move("hover_move_run", false);
    }
}

/// Search (`search`).
fn hover_search(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let path = if context.game.random() < 0.5 {
        "hover/hovsrch1.wav"
    } else {
        "hover/hovsrch2.wav"
    };
    context.game.sound(&actor, path, 2, 1.0, 1.0);
}

/// Initialize (`initialize`).
fn hover_initialize(context: &mut MonsterContext) {
    monster_loop_sound(context, "hover/hovidle1.wav");
}

/// Pain (`pain`).
fn hover_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
    let light = reaction.damage <= 25.0;
    let first = !light || context.game.random() < 0.5;
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if first {
            "hover/hovpain1.wav"
        } else {
            "hover/hovpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    context.set_move(
        if !light {
            "hover_move_pain1"
        } else if first {
            "hover_move_pain3"
        } else {
            "hover_move_pain2"
        },
        false,
    );
}

/// Die (`die`).
fn hover_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    if standard_gib(
        context,
        reaction,
        2,
        2,
        "models/objects/gibs/sm_meat/tris.md2",
        1.0,
    ) || context.state().dead
    {
        return;
    }
    let path = if context.game.random() < 0.5 {
        "hover/hovdeth1.wav"
    } else {
        "hover/hovdeth2.wav"
    };
    begin_death(context, reaction, path, "hover_move_death1", 2, 4);
}

/// Reattack (`hover_reattack`).
fn hover_reattack(context: &mut MonsterContext) {
    if alive_enemy(context) && visible(context, None) && context.game.random() <= 0.6 {
        context.set_move("hover_move_attack1", false);
    } else {
        context.set_move("hover_move_end_attack", false);
    }
}

/// Fire blaster (`hover_fire_blaster`).
fn hover_fire_blaster(context: &mut MonsterContext) {
    let Some((start, direction)) = monster_shot(context, 62, 0.0) else {
        return;
    };
    let effects = if context.entity().frame == hover_frame::ATTAK104 {
        64
    } else {
        0
    };
    let fire_blaster = context.weapons.fire_blaster;
    let actor = context.actor().clone();
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
    monster_muzzle(context, 62, direction, start);
}

/// Settle the corpse, then explode on landing or timeout (`hover_dead`).
fn hover_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.state_mut().corpse = true;
    let mut body = context.game.body_of(actor.clone());
    body.bounds = Bounds {
        min: Vec3 {
            x: -16.0,
            y: -16.0,
            z: -24.0,
        },
        max: Vec3 {
            x: 16.0,
            y: 16.0,
            z: -8.0,
        },
    };
    context.game.write_body(actor.clone(), &body, true);
    context.game.set_motion_kind(actor.clone(), Q2MotionKind::Toss);
    // The donor captures the deadline in a think closure; the port keeps
    // it in the entity timestamp like the original C timeout fields.
    let deadline = context.game.host.now() + 15.0;
    context.entity_mut().timestamp = deadline;
    context.game.schedule(actor, 0.1, hover_dead_think);
}

/// Corpse think (`hover_dead` think).
pub fn hover_dead_think(actor: ActorId, game: &mut Q2GameServices) {
    let deadline = game.require_entity(&actor).timestamp;
    if game.body_of(actor.clone()).ground.is_none() && game.host.now() < deadline {
        game.schedule(actor, 0.1, hover_dead_think);
        return;
    }
    let origin = game.body_of(actor.clone()).origin;
    game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:explosion1".to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    game.remove_actor(actor);
}

/// Hover source callbacks.
pub fn hover_source_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("q2:hover_dead", hover_dead_think);
    callbacks
}

/// Hover definition (`hoverDefinition`).
pub fn hover_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_hover",
        "hover",
        "models/monsters/hover/tris.md2",
        240.0,
        -100.0,
        150.0,
        Bounds {
            min: Vec3 {
                x: -24.0,
                y: -24.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 24.0,
                y: 24.0,
                z: 32.0,
            },
        },
        1.0,
        "hover_move_stand",
        hover_moves(),
        move_handler("hover_move_stand"),
        move_handler("hover_move_walk"),
        MonsterHandler::Callback(hover_run),
        move_handler("hover_move_start_attack"),
        hover_die,
    );
    definition.locomotion = Some(MonsterLocomotion::Fly);
    definition.sight = Some(sound_handler("hover/hovsght1.wav", 2, 1.0));
    definition.search = Some(MonsterHandler::Callback(hover_search));
    definition.pain = Some(hover_pain);
    definition.initialize = Some(MonsterHandler::Callback(hover_initialize));
    definition.source_callbacks = Some(hover_source_callbacks());
    definition.callbacks = HashMap::from([
        ("hover_run".to_string(), MonsterHandler::Callback(hover_run)),
        (
            "hover_attack".to_string(),
            move_handler("hover_move_attack1"),
        ),
        (
            "hover_reattack".to_string(),
            MonsterHandler::Callback(hover_reattack),
        ),
        (
            "hover_fire_blaster".to_string(),
            MonsterHandler::Callback(hover_fire_blaster),
        ),
        (
            "hover_dead".to_string(),
            MonsterHandler::Callback(hover_dead),
        ),
    ]);
    definition
}
