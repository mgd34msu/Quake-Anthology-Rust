//! Medic monster (`src/content/q2/base/monsters/medic.ts`).
//!
//! Quake II m_medic.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, add3, length3, scale3, sub3, vec3};

use super::common::{
    begin_death, damaged_skin, finish_corpse_default, monster_muzzle, monster_shot, move_handler,
    sound_handler,
};
use super::tables::medic::{medic_frame, medic_moves};
use crate::q2::foundation::host::{
    Q2MonsterBeam, Q2PresentationEvent, Q2SoundEvent, Q2SoundLoop, Q2TraceRequest,
};
use crate::q2::foundation::monsters::ai::{
    MASK_SHOT, angles_vectors, health, project_flash, set_duck, vector_angles, visible,
};
use crate::q2::foundation::monsters::perception::{default_check_attack, found_target};
use crate::q2::foundation::monsters::types::{
    MonsterContext, MonsterHandler, Q2MonsterDefinition, record_at,
};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::support::contracts::{DeathReaction, PainReaction, TraceHit, TraceResult};

/// Cable offsets (`cableOffsets`).
const CABLE_OFFSETS: [Vec3; 10] = [
    Vec3 { x: 45.0, y: -9.2, z: 15.5 },
    Vec3 { x: 48.4, y: -9.7, z: 15.2 },
    Vec3 { x: 47.8, y: -9.8, z: 15.8 },
    Vec3 { x: 47.3, y: -9.3, z: 14.3 },
    Vec3 { x: 45.4, y: -10.1, z: 13.1 },
    Vec3 { x: 41.9, y: -12.7, z: 12.0 },
    Vec3 { x: 37.8, y: -15.8, z: 11.2 },
    Vec3 { x: 34.3, y: -18.4, z: 10.7 },
    Vec3 { x: 32.7, y: -19.7, z: 10.4 },
    Vec3 { x: 32.7, y: -19.7, z: 10.4 },
];

/// Find a patient (`patient`).
fn patient(context: &mut MonsterContext) -> Option<ActorId> {
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    let mut best: Option<ActorId> = None;
    let mut best_health = 0.0;
    for candidate in context.game.host.nearby(origin, 1024.0) {
        let entity = context.game.entities.get(&candidate).cloned();
        let good_guy = context.game.monsters.states.get(&candidate).is_some_and(|state| state.good_guy);
        let Some(entity) = entity else { continue };
        if candidate == actor
            || entity.server_flags & 4 == 0
            || good_guy
            || entity.owner.is_some()
            || health(&mut *context.game, Some(&candidate)) > 0.0
            || entity.next_think.is_some()
            || !visible(context, Some(&candidate))
        {
            continue;
        }
        if best.is_none() || entity.max_health > best_health {
            best_health = entity.max_health;
            best = Some(candidate);
        }
    }
    best
}

/// Acquire a patient (`acquire`).
fn acquire(context: &mut MonsterContext, preserve_enemy: bool) -> bool {
    let Some(target) = patient(context) else {
        return false;
    };
    if preserve_enemy {
        let enemy = context.entity().enemy.clone();
        context.state_mut().old_enemy = enemy;
    }
    let actor = context.actor().clone();
    context.entity_mut().enemy = Some(target.clone());
    context.game.require_entity_mut(&target).owner = Some(actor);
    context.state_mut().medic = true;
    found_target(context);
    true
}

/// Run (`run`).
fn medic_run(context: &mut MonsterContext) {
    if !context.state().medic && acquire(context, true) {
        return;
    }
    if context.state().stand_ground {
        context.set_move("medic_move_stand", true);
    } else {
        context.set_move("medic_move_run", true);
    }
}

/// Idle (`idle`).
fn medic_idle(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "medic/idle.wav", 2, 1.0, 2.0);
    acquire(context, false);
}

/// Attack (`attack`).
fn medic_attack(context: &mut MonsterContext) {
    if context.state().medic {
        context.set_move("medic_move_attackCable", true);
    } else {
        context.set_move("medic_move_attackBlaster", true);
    }
}

/// Search (`search`).
fn medic_search(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "medic/medsrch1.wav", 2, 1.0, 2.0);
    if context.state().old_enemy.is_none() {
        acquire(context, true);
    }
}

/// Check attack (`checkAttack`).
fn medic_check_attack(context: &mut MonsterContext) -> bool {
    if context.state().medic {
        medic_attack(context);
        return true;
    }
    default_check_attack(context)
}

/// Pain (`pain`).
fn medic_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
    let first = context.game.random() < 0.5;
    context.set_move(
        if first {
            "medic_move_pain1"
        } else {
            "medic_move_pain2"
        },
        false,
    );
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if first {
            "medic/medpain1.wav"
        } else {
            "medic/medpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
}

/// Die (`die`).
fn medic_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    if let Some(enemy) = enemy {
        let owned = context.game.entities.get(&enemy).and_then(|target| target.owner.clone());
        if owned.as_ref() == Some(&actor) {
            context.game.require_entity_mut(&enemy).owner = None;
        }
    }
    begin_death(context, reaction, "medic/meddeth1.wav", "medic_move_death", 2, 4);
}

/// Dodge (`dodge`).
fn medic_dodge(
    context: &mut MonsterContext,
    attacker: &ActorId,
    _eta: f64,
    _trace: Option<&TraceResult>,
    _direct: bool,
) {
    if context.game.random() > 0.25 {
        return;
    }
    if context.entity().enemy.is_none() {
        context.entity_mut().enemy = Some(attacker.clone());
    }
    context.set_move("medic_move_duck", true);
}

/// Duck down (`medic_duck_down`).
fn medic_duck_down(context: &mut MonsterContext) {
    if context.state().ducked {
        return;
    }
    set_duck(context, true);
    let now = context.game.host.now();
    context.state_mut().pause_time = now + 1.0;
}

/// Duck hold (`medic_duck_hold`).
fn medic_duck_hold(context: &mut MonsterContext) {
    let hold = context.game.host.now() < context.state().pause_time;
    context.state_mut().hold_frame = hold;
}

/// Duck up (`medic_duck_up`).
fn medic_duck_up(context: &mut MonsterContext) {
    set_duck(context, false);
}

/// Hook retract (`medic_hook_retract`).
fn medic_hook_retract(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "medic/medatck5.wav", 1, 1.0, 1.0);
    let enemy = context.entity().enemy.clone();
    if let Some(enemy) = enemy {
        if let Some(state) = context.game.monsters.states.get_mut(&enemy) {
            state.resurrecting = false;
        }
    }
}

/// Continue (`medic_continue`).
fn medic_continue(context: &mut MonsterContext) {
    if visible(context, None) && context.game.random() <= 0.95 {
        context.set_move("medic_move_attackHyperBlaster", true);
    }
}

/// Fire blaster (`medic_fire_blaster`).
fn medic_fire_blaster(context: &mut MonsterContext) {
    let Some((start, direction)) = monster_shot(context, 60, 0.0) else {
        return;
    };
    let frame = context.entity().frame;
    let effects = if frame == medic_frame::ATTACK9 || frame == medic_frame::ATTACK12 {
        8
    } else if frame == medic_frame::ATTACK19
        || frame == medic_frame::ATTACK22
        || frame == medic_frame::ATTACK25
        || frame == medic_frame::ATTACK28
    {
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
        2.0,
        1000.0,
        effects,
        false,
        Mod::BLASTER,
    );
    monster_muzzle(context, 60, direction, start);
}

/// Cable attack (`medic_cable_attack`).
fn medic_cable_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    let Some(enemy) = enemy else { return };
    if !context.game.entities.contains_key(&enemy) {
        return;
    }
    let frame = context.entity().frame;
    let start = project_flash(
        context,
        *record_at(&CABLE_OFFSETS, (frame - medic_frame::ATTACK42) as usize),
        None,
    );
    let body = context.game.body_of(enemy.clone());
    let direction = sub3(start, body.origin);
    if length3(direction) > 256.0 {
        return;
    }
    let mut pitch = vector_angles(direction).x;
    if pitch < -180.0 {
        pitch += 360.0;
    }
    if pitch.abs() > 45.0 {
        return;
    }
    let trace = context.game.host.trace(&Q2TraceRequest {
        start,
        end: body.origin,
        bounds: None,
        ignore: Some(actor.clone()),
        mask: MASK_SHOT,
        exclude: Vec::new(),
    });
    if trace.fraction != 1.0
        && !matches!(&trace.hit, TraceHit::Actor { actor: hit } if *hit == enemy)
    {
        return;
    }
    if frame == medic_frame::ATTACK43 {
        context.game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(enemy.clone()),
            origin: body.origin,
            path: "medic/medatck3.wav".to_string(),
            channel: 0,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Once,
            loop_owner: None,
        }));
        if let Some(state) = context.game.monsters.states.get_mut(&enemy) {
            state.resurrecting = true;
        }
    } else if frame == medic_frame::ATTACK50 {
        {
            let target = context.game.require_entity_mut(&enemy);
            target.spawnflags = 0;
            target.target = String::new();
            target.targetname = String::new();
            target.combat_target = String::new();
            target.death_target = String::new();
            target.owner = Some(actor.clone());
        }
        crate::q2::foundation::monsters::respawn_monster(&mut *context.game, enemy.clone());
        context.game.require_entity_mut(&enemy).owner = None;
        let think = context.game.require_entity(&enemy).think;
        if think.is_some() {
            let now = context.game.host.now();
            context.game.require_entity_mut(&enemy).next_think = Some(now);
            let think = think.expect("medic revive think");
            think(enemy.clone(), &mut *context.game);
        }
        if let Some(state) = context.game.monsters.states.get_mut(&enemy) {
            state.resurrecting = true;
        }
        let old_enemy = context.state().old_enemy.clone();
        if old_enemy.as_ref().is_some_and(|old| context.game.host.is_player(old)) {
            let old = old_enemy.expect("medic old enemy");
            context.game.require_entity_mut(&enemy).enemy = Some(old);
            let mut revived = MonsterContext::new(enemy.clone(), &mut *context.game);
            found_target(&mut revived);
        }
    } else if frame == medic_frame::ATTACK44 {
        context.game.sound(&actor, "medic/medatck4.wav", 1, 1.0, 1.0);
    }
    let current = context.game.body_of(enemy.clone());
    let end = vec3(
        current.origin.x,
        current.origin.y,
        current.origin.z + (current.bounds.min.z + current.bounds.max.z) / 2.0,
    );
    let self_angles = context.game.body_of(actor.clone()).angles;
    let beam_start = add3(start, scale3(angles_vectors(self_angles).forward, 8.0));
    context.game.host_emit(Q2PresentationEvent::MonsterBeam {
        effect: Q2MonsterBeam::Medic,
        actor,
        start: beam_start,
        end,
    });
}

/// Create a medic definition (`createMedicDefinition`).
pub fn create_medic_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_medic",
        "medic",
        "models/monsters/medic/tris.md2",
        300.0,
        -130.0,
        400.0,
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
        "medic_move_stand",
        medic_moves(),
        move_handler("medic_move_stand"),
        move_handler("medic_move_walk"),
        MonsterHandler::Callback(medic_run),
        MonsterHandler::Callback(medic_attack),
        medic_die,
    );
    definition.idle = Some(MonsterHandler::Callback(medic_idle));
    definition.sight = Some(sound_handler("medic/medsght1.wav", 2, 1.0));
    definition.search = Some(MonsterHandler::Callback(medic_search));
    definition.pain = Some(medic_pain);
    definition.dodge = Some(medic_dodge);
    definition.check_attack = Some(medic_check_attack);
    definition.callbacks = HashMap::from([
        ("medic_idle".to_string(), MonsterHandler::Callback(medic_idle)),
        ("medic_run".to_string(), MonsterHandler::Callback(medic_run)),
        (
            "medic_dead".to_string(),
            MonsterHandler::Callback(finish_corpse_default),
        ),
        (
            "medic_duck_down".to_string(),
            MonsterHandler::Callback(medic_duck_down),
        ),
        (
            "medic_duck_hold".to_string(),
            MonsterHandler::Callback(medic_duck_hold),
        ),
        (
            "medic_duck_up".to_string(),
            MonsterHandler::Callback(medic_duck_up),
        ),
        (
            "medic_hook_launch".to_string(),
            sound_handler("medic/medatck2.wav", 1, 1.0),
        ),
        (
            "medic_hook_retract".to_string(),
            MonsterHandler::Callback(medic_hook_retract),
        ),
        (
            "medic_continue".to_string(),
            MonsterHandler::Callback(medic_continue),
        ),
        (
            "medic_fire_blaster".to_string(),
            MonsterHandler::Callback(medic_fire_blaster),
        ),
        (
            "medic_cable_attack".to_string(),
            MonsterHandler::Callback(medic_cable_attack),
        ),
    ]);
    definition
}
