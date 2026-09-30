//! Parasite monster (`src/content/q2/base/monsters/parasite.ts`).
//!
//! Quake II m_parasite.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::math::{Bounds, Vec3, length3, sub3, vec3};

use super::common::{
    begin_death, damaged_skin, finish_corpse_default, move_handler, sound_handler,
};
use super::tables::parasite::{parasite_frame, parasite_moves};
use crate::q2::foundation::host::{
    Q2MonsterBeam, Q2PresentationEvent, Q2SoundEvent, Q2SoundLoop, Q2TraceRequest,
};
use crate::q2::foundation::monsters::ai::{
    MASK_SHOT, enemy_body, project_flash, vector_angles,
};
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{DeathReaction, PainReaction, TraceHit};

/// Run (`run`).
fn parasite_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("parasite_move_stand", false);
    } else {
        context.set_move("parasite_move_run", false);
    }
}

/// Start run (`startRun`).
fn parasite_start_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("parasite_move_stand", false);
    } else {
        context.set_move("parasite_move_start_run", false);
    }
}

/// Whether a drain can reach from start to end (`parasiteDrainReachable`).
pub fn parasite_drain_reachable(start: Vec3, end: Vec3) -> bool {
    let direction = sub3(start, end);
    if length3(direction) > 256.0 {
        return false;
    }
    let mut pitch = vector_angles(direction).x;
    if pitch < -180.0 {
        pitch += 360.0;
    }
    pitch.abs() <= 30.0
}

/// Pain (`pain`).
fn parasite_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
    let actor = context.actor().clone();
    let path = if context.game.random() < 0.5 {
        "parasite/parpain1.wav"
    } else {
        "parasite/parpain2.wav"
    };
    context.game.sound(&actor, path, 2, 1.0, 1.0);
    context.set_move("parasite_move_pain1", false);
}

/// Die (`die`).
fn parasite_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    begin_death(context, reaction, "parasite/pardeth1.wav", "parasite_move_death", 2, 4);
}

/// Refidget (`parasite_refidget`).
fn parasite_refidget(context: &mut MonsterContext) {
    if context.game.random() <= 0.8 {
        context.set_move("parasite_move_fidget", false);
    } else {
        context.set_move("parasite_move_end_fidget", false);
    }
}

/// Drain attack (`parasite_drain_attack`).
fn parasite_drain_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let enemy_body = enemy_body(context);
    let enemy = context.entity().enemy.clone();
    let (Some(enemy_body), Some(enemy)) = (enemy_body, enemy) else {
        return;
    };
    let start = project_flash(context, vec3(24.0, 0.0, 6.0), None);
    let top = vec3(
        enemy_body.origin.x,
        enemy_body.origin.y,
        enemy_body.origin.z + enemy_body.bounds.max.z - 8.0,
    );
    let bottom = vec3(
        enemy_body.origin.x,
        enemy_body.origin.y,
        enemy_body.origin.z + enemy_body.bounds.min.z + 8.0,
    );
    if !parasite_drain_reachable(start, enemy_body.origin)
        && !parasite_drain_reachable(start, top)
        && !parasite_drain_reachable(start, bottom)
    {
        return;
    }
    // The source restores the target origin after checking top/bottom reach.
    let end = enemy_body.origin;
    let trace = context.game.host.trace(&Q2TraceRequest {
        start,
        end,
        bounds: None,
        ignore: Some(actor.clone()),
        mask: MASK_SHOT,
        exclude: Vec::new(),
    });
    if !matches!(&trace.hit, TraceHit::Actor { actor: hit } if *hit == enemy) {
        return;
    }
    let frame = context.entity().frame;
    let first = frame == parasite_frame::DRAIN03;
    if first {
        context.game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(enemy.clone()),
            origin: end,
            path: "parasite/paratck2.wav".to_string(),
            channel: 0,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Once,
            loop_owner: None,
        }));
    } else if frame == parasite_frame::DRAIN04 {
        context.game.sound(&actor, "parasite/paratck3.wav", 1, 1.0, 1.0);
    }
    context.game.host_emit(Q2PresentationEvent::MonsterBeam {
        effect: Q2MonsterBeam::Parasite,
        actor: actor.clone(),
        start,
        end,
    });
    context.game.damage(
        enemy,
        actor.clone(),
        Some(actor),
        if first { 5.0 } else { 2.0 },
        0.0,
        sub3(start, end),
        end,
        vec3(0.0, 0.0, 0.0),
        0,
        8,
        None,
    );
}

/// Parasite definition (`parasiteDefinition`).
pub fn parasite_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_parasite",
        "parasite",
        "models/monsters/parasite/tris.md2",
        175.0,
        -50.0,
        250.0,
        Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 24.0,
            },
        },
        1.0,
        "parasite_move_stand",
        parasite_moves(),
        move_handler("parasite_move_stand"),
        move_handler("parasite_move_start_walk"),
        MonsterHandler::Callback(parasite_start_run),
        move_handler("parasite_move_drain"),
        parasite_die,
    );
    definition.sight = Some(sound_handler("parasite/parsght1.wav", 1, 1.0));
    definition.idle = Some(move_handler("parasite_move_start_fidget"));
    definition.pain = Some(parasite_pain);
    definition.callbacks = HashMap::from([
        (
            "parasite_stand".to_string(),
            move_handler("parasite_move_stand"),
        ),
        (
            "parasite_walk".to_string(),
            move_handler("parasite_move_walk"),
        ),
        (
            "parasite_run".to_string(),
            MonsterHandler::Callback(parasite_run),
        ),
        (
            "parasite_start_run".to_string(),
            MonsterHandler::Callback(parasite_start_run),
        ),
        (
            "parasite_dead".to_string(),
            MonsterHandler::Callback(finish_corpse_default),
        ),
        (
            "parasite_launch".to_string(),
            sound_handler("parasite/paratck1.wav", 1, 1.0),
        ),
        (
            "parasite_reel_in".to_string(),
            sound_handler("parasite/paratck4.wav", 1, 1.0),
        ),
        (
            "parasite_tap".to_string(),
            sound_handler("parasite/paridle1.wav", 1, 2.0),
        ),
        (
            "parasite_scratch".to_string(),
            sound_handler("parasite/paridle2.wav", 1, 2.0),
        ),
        (
            "parasite_search".to_string(),
            sound_handler("parasite/parsrch1.wav", 1, 2.0),
        ),
        (
            "parasite_do_fidget".to_string(),
            move_handler("parasite_move_fidget"),
        ),
        (
            "parasite_refidget".to_string(),
            MonsterHandler::Callback(parasite_refidget),
        ),
        (
            "parasite_drain_attack".to_string(),
            MonsterHandler::Callback(parasite_drain_attack),
        ),
    ]);
    definition
}
