//! Rerelease shambler (`src/content/q2/rerelease/monsters/shambler.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::math::{add3, scale3, vec3, Vec3};

use super::common::{chainfist, check_gib, predicted_direction, reacts_to_pain};
use super::tables::shambler::{shambler_frame, shambler_moves};
use crate::q2::base::monsters::common::{move_handler, sound_handler};
use crate::q2::foundation::host::{Q2BeamEvent, Q2PresentationEvent, Q2TraceRequest};
use crate::q2::foundation::monsters::ai::{
    clear_shot, corpse, enemy_body, enemy_eye, health, project_flash, run_ai, target_distance,
};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::types::{MonsterAi, MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Left hand lightning offsets (`leftHand`).
const LEFT_HAND: [Vec3; 5] = [
    Vec3 {
        x: 44.0,
        y: 36.0,
        z: 25.0,
    },
    Vec3 {
        x: 10.0,
        y: 44.0,
        z: 57.0,
    },
    Vec3 {
        x: -1.0,
        y: 40.0,
        z: 70.0,
    },
    Vec3 {
        x: -10.0,
        y: 34.0,
        z: 75.0,
    },
    Vec3 {
        x: 7.4,
        y: 24.0,
        z: 89.0,
    },
];

/// Right hand lightning offsets (`rightHand`).
const RIGHT_HAND: [Vec3; 5] = [
    Vec3 {
        x: 28.0,
        y: -38.0,
        z: 25.0,
    },
    Vec3 {
        x: 31.0,
        y: -7.0,
        z: 70.0,
    },
    Vec3 {
        x: 20.0,
        y: 0.0,
        z: 80.0,
    },
    Vec3 {
        x: 16.0,
        y: 1.2,
        z: 81.0,
    },
    Vec3 {
        x: 27.0,
        y: -11.0,
        z: 83.0,
    },
];

/// Run (`run`).
fn shambler_run(context: &mut MonsterContext) {
    let brutal = context
        .entity()
        .enemy
        .clone()
        .is_some_and(|enemy| context.game.host.is_player(&enemy));
    context.state_mut().brutal = brutal;
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "shambler_move_stand"
        } else {
            "shambler_move_run"
        },
        true,
    );
}

/// Clear the lightning beam (`clearBeam`).
fn shambler_clear_beam(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let beams = [context.entity().beam.clone(), context.entity().beam2.clone()];
    for beam in beams.into_iter().flatten() {
        if context.game.entity(&beam).is_none() {
            continue;
        }
        let origin = context.game.body_of(beam.clone()).origin;
        let end = context.game.require_entity(&beam).pos2;
        context.game.host_emit(Q2PresentationEvent::Beam(Q2BeamEvent {
            actor: beam.clone(),
            start: origin,
            end,
            width: 2.0,
            color: 0xffff_ffff_u32 as i32,
            visible: false,
        }));
        context.game.remove_actor(beam);
    }
    let entity = context.game.require_entity_mut(&actor);
    entity.beam = None;
    entity.beam2 = None;
}

/// Update the lightning beam (`lightningUpdate`).
fn shambler_lightning_update(context: &mut MonsterContext) {
    let frame = context.entity().frame - shambler_frame::MAGIC01;
    let Ok(index) = usize::try_from(frame) else {
        return;
    };
    let (Some(left), Some(right)) = (LEFT_HAND.get(index), RIGHT_HAND.get(index)) else {
        return shambler_clear_beam(context);
    };
    let beam = context.entity().beam.clone();
    let Some(beam) = beam.filter(|beam| context.game.entity(beam).is_some()) else {
        return;
    };
    let start = project_flash(context, *left, None);
    let end = project_flash(context, *right, None);
    let mut moved = context.game.body_of(beam.clone());
    moved.origin = start;
    context.game.write_body(beam.clone(), &moved, true);
    context.game.require_entity_mut(&beam).pos2 = end;
    context.game.host_emit(Q2PresentationEvent::Beam(Q2BeamEvent {
        actor: beam,
        start,
        end,
        width: 2.0,
        color: 0xffff_ffff_u32 as i32,
        visible: true,
    }));
}

/// Claw attack (`claw`).
fn shambler_claw(context: &mut MonsterContext, smash: bool) {
    let actor = context.actor().clone();
    let Some(enemy) = context.entity().enemy.clone() else {
        return;
    };
    run_ai(context, &MonsterAi::Charge, if smash { 0.0 } else { 10.0 });
    if !context.game.can_damage(&enemy, &actor) {
        return;
    }
    let aim = Vec3 {
        x: 80.0,
        y: context.game.body_of(actor.clone()).bounds.min.x,
        z: -4.0,
    };
    let damage = (if smash { 110.0 } else { 70.0 }) + (context.game.random() * 10.0).floor();
    let fire_hit = context.weapons.fire_hit;
    if fire_hit(
        actor.clone(),
        &mut *context.game,
        aim,
        damage,
        if smash { 120.0 } else { 80.0 },
    ) {
        context.game.sound(&actor, "shambler/smack.wav", 1, 1.0, 1.0);
    }
}

/// Smash claw (`sham_smash10`).
fn shambler_smash(context: &mut MonsterContext) {
    shambler_claw(context, true);
}

/// Claw (`ShamClaw`).
fn shambler_chop(context: &mut MonsterContext) {
    shambler_claw(context, false);
}

/// Melee (`melee`).
fn shambler_melee(context: &mut MonsterContext) {
    let chance = context.game.random();
    let actor = context.actor().clone();
    let full_health = health(&mut *context.game, Some(&actor)) == 600.0;
    context.set_move(
        if chance > 0.6 || full_health {
            "shambler_attack_smash"
        } else if chance > 0.3 {
            "shambler_attack_swingl"
        } else {
            "shambler_attack_swingr"
        },
        true,
    );
}

/// Pain (`pain`).
fn shambler_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    if context.game.host.now() < context.entity().timestamp {
        return;
    }
    let now = context.game.host.now();
    context.game.require_entity_mut(&actor).timestamp = now + 0.001;
    context.game.sound(&actor, "shambler/shurt2.wav", 0, 1.0, 1.0);
    let damage = reaction.damage;
    if !chainfist(context) && damage <= 30.0 && context.game.random() > 0.2 {
        return;
    }
    let frame = context.entity().frame;
    let attacking = (shambler_frame::SMASH01..=shambler_frame::SMASH12).contains(&frame)
        || (shambler_frame::SWINGL01..=shambler_frame::SWINGL09).contains(&frame)
        || (shambler_frame::SWINGR01..=shambler_frame::SWINGR09).contains(&frame);
    if context.game.options.skill >= 2 && attacking {
        return;
    }
    if !reacts_to_pain(context) || context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 2.0;
    context.set_move("shambler_move_pain", true);
}

/// Die (`die`).
fn shambler_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    shambler_clear_beam(context);
    if check_gib(context) {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        let damage = reaction.pain.damage;
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_meat/tris.md2",
            damage,
            Q2GibOptions::default(),
        );
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/chest/tris.md2",
            damage,
            Q2GibOptions::default(),
        );
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/head2/tris.md2",
            damage,
            Q2GibOptions {
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
    context.game.sound(&actor, "shambler/sdeath.wav", 2, 1.0, 1.0);
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
    context.set_move("shambler_move_death", true);
}

/// Maybe idle (`shambler_maybe_idle`).
fn shambler_maybe_idle(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.random() > 0.8 {
        context.game.sound(&actor, "shambler/sidle.wav", 2, 1.0, 2.0);
    }
}

/// Wind up the lightning (`shambler_windup`).
fn shambler_windup(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "shambler/sattck1.wav", 1, 1.0, 1.0);
    let beam = context.game.create("shambler_lightning", BTreeMap::new());
    context.game.require_entity_mut(&actor).beam = Some(beam.clone());
    let beam_entity = context.game.require_entity_mut(&beam);
    beam_entity.owner = Some(actor);
    beam_entity.model = "models/proj/lightning/tris.md2".to_string();
    beam_entity.render_flags |= 128;
    shambler_lightning_update(context);
}

/// Save the target location (`ShamblerSaveLoc`).
fn shambler_save_loc(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let Some(eye) = enemy_eye(context) else {
        return;
    };
    context.game.require_entity_mut(&actor).pos1 = eye;
    context.state_mut().next_frame = shambler_frame::MAGIC09;
    context.game.sound(&actor, "shambler/sboom.wav", 1, 1.0, 1.0);
    shambler_lightning_update(context);
}

/// Cast lightning (`ShamblerCastLightning`).
fn shambler_cast_lightning(context: &mut MonsterContext) {
    if enemy_body(context).is_none() {
        return;
    }
    let mut offset = vec3(0.0, 0.0, 48.0);
    for index in 0..8 {
        let candidate = vec3(0.0, 0.0, 48.0 - index as f32 * 4.0);
        if clear_shot(context, candidate) {
            offset = candidate;
            break;
        }
    }
    let start = project_flash(context, offset, None);
    let spawnflags = context.entity().spawnflags;
    let Some(direction) = predicted_direction(context, start, 0.0, false, if spawnflags & 1 != 0 { 0.0 } else { 0.1 })
    else {
        return;
    };
    let actor = context.actor().clone();
    let trace = context.game.host.trace(&Q2TraceRequest {
        start,
        end: add3(start, scale3(direction, 8192.0)),
        bounds: None,
        ignore: Some(actor.clone()),
        mask: 0x4600_401b,
        exclude: Vec::new(),
    });
    context.game.host_emit(Q2PresentationEvent::Beam(Q2BeamEvent {
        actor: actor.clone(),
        start,
        end: trace.end,
        width: 2.0,
        color: 0xffff_ffff_u32 as i32,
        visible: true,
    }));
    let damage = 8.0 + (context.game.random() * 4.0).floor();
    let fire_bullet = context.weapons.fire_bullet;
    fire_bullet(actor, &mut *context.game, start, direction, damage, 15.0, 0.0, 0.0, 45);
}

/// Left swing follow-up (`sham_swingl9`).
fn shambler_swingl9(context: &mut MonsterContext) {
    run_ai(context, &MonsterAi::Charge, 8.0);
    if context.game.random() < 0.5 && enemy_body(context).is_some() && target_distance(context) < 80.0 {
        context.set_move("shambler_attack_swingr", true);
    }
}

/// Right swing follow-up (`sham_swingr9`).
fn shambler_swingr9(context: &mut MonsterContext) {
    run_ai(context, &MonsterAi::Charge, 1.0);
    run_ai(context, &MonsterAi::Charge, 10.0);
    if context.game.random() < 0.5 && enemy_body(context).is_some() && target_distance(context) < 80.0 {
        context.set_move("shambler_attack_swingl", true);
    }
}

/// Shrink the corpse (`shambler_shrink`).
fn shambler_shrink(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let mut moved = context.game.body_of(actor.clone());
    context.game.require_entity_mut(&actor).server_flags |= 2;
    moved.bounds.max.z = 0.0;
    context.game.write_body(actor, &moved, true);
}

/// Dead (`shambler_dead`).
fn shambler_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    corpse(context);
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.min = vec3(-16.0, -16.0, -24.0);
    moved.bounds.max = vec3(16.0, 16.0, 0.0);
    context.game.write_body(actor, &moved, true);
}

/// Create the shambler definition (`shamblerDefinition`).
pub fn shambler_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_shambler",
        "shambler",
        "models/monsters/shambler/tris.md2",
        600.0,
        -60.0,
        500.0,
        qa_core::math::Bounds {
            min: vec3(-32.0, -32.0, -24.0),
            max: vec3(32.0, 32.0, 64.0),
        },
        1.0,
        "shambler_move_stand",
        shambler_moves(),
        move_handler("shambler_move_stand"),
        move_handler("shambler_move_walk"),
        MonsterHandler::Callback(shambler_run),
        move_handler("shambler_attack_magic"),
        shambler_die,
    );
    definition.sight = Some(sound_handler("shambler/ssight.wav", 2, 1.0));
    definition.idle = Some(sound_handler("shambler/sidle.wav", 2, 2.0));
    definition.melee = Some(MonsterHandler::Callback(shambler_melee));
    definition.pain = Some(shambler_pain);
    for (name, handler) in [
        ("shambler_run", MonsterHandler::Callback(shambler_run)),
        ("shambler_maybe_idle", MonsterHandler::Callback(shambler_maybe_idle)),
        ("shambler_windup", MonsterHandler::Callback(shambler_windup)),
        (
            "shambler_lightning_update",
            MonsterHandler::Callback(shambler_lightning_update),
        ),
        ("ShamblerSaveLoc", MonsterHandler::Callback(shambler_save_loc)),
        (
            "ShamblerCastLightning",
            MonsterHandler::Callback(shambler_cast_lightning),
        ),
        ("shambler_melee1", sound_handler("shambler/melee1.wav", 1, 1.0)),
        ("shambler_melee2", sound_handler("shambler/melee2.wav", 1, 1.0)),
        ("sham_smash10", MonsterHandler::Callback(shambler_smash)),
        ("ShamClaw", MonsterHandler::Callback(shambler_chop)),
        ("sham_swingl9", MonsterHandler::Callback(shambler_swingl9)),
        ("sham_swingr9", MonsterHandler::Callback(shambler_swingr9)),
        ("shambler_shrink", MonsterHandler::Callback(shambler_shrink)),
        ("shambler_dead", MonsterHandler::Callback(shambler_dead)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
