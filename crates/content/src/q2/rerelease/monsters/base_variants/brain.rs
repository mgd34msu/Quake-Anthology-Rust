//! Rerelease brain (`src/content/q2/rerelease/monsters/base-variants/brain.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, length3, scale3, sub3, vec3, Vec3};

use super::super::beam::{fire_monster_beam, free_monster_beam, update_monster_beam};
use super::super::common::{check_gib, predicted_direction, reacts_to_pain};
use super::super::tables::brain::{brain_frame, brain_moves};
use crate::contract::PoweredProtectionState;
use crate::q2::base::monsters::brain::brain_definition;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::number_field;
use crate::q2::foundation::host::{Q2GameServices, Q2MonsterBeam, Q2PresentationEvent, Q2TraceRequest};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, corpse, enemy_body, health, project_flash, set_duck, target_distance, vector_angles, visible,
};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::missionpacks::monsters::power_armor::{
    monster_power_armor, restore_monster_power_armor, PowerArmorKind,
};
use crate::q2::missionpacks::monsters::xatrix_variants::{XATRIX_BRAIN_LEFT_EYE, XATRIX_BRAIN_RIGHT_EYE};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction, TraceHit};

/// Screen (`screen`).
fn brain_screen(context: &mut MonsterContext, active: bool) {
    let actor = context.actor().clone();
    let cells = context
        .game
        .host
        .inventory()
        .count(&actor, &"q2:monster-power".to_string());
    let owned = context.game.owned_of(actor);
    let protection = if active {
        PoweredProtectionState::Screen { cells }
    } else {
        PoweredProtectionState::None
    };
    context.game.host.combat().set_powered_protection(&owned, &protection);
}

/// Run (`run`).
fn rerelease_brain_run(context: &mut MonsterContext) {
    brain_screen(context, true);
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "brain_move_stand"
        } else {
            "brain_move_run"
        },
        true,
    );
}

/// Tongue allowed (`tongueAllowed`).
fn brain_tongue_allowed(start: Vec3, end: Vec3) -> bool {
    let direction = sub3(start, end);
    if length3(direction) > 512.0 {
        return false;
    }
    let mut pitch = vector_angles(direction).x;
    if pitch < -180.0 {
        pitch += 360.0;
    }
    pitch.abs() <= 30.0
}

/// Hit (`hit`).
fn brain_hit(context: &mut MonsterContext, right: bool) {
    let actor = context.actor().clone();
    let bounds = context.game.body_of(actor.clone()).bounds;
    let aim = vec3(80.0, if right { bounds.max.x } else { bounds.min.x }, 8.0);
    let damage = 15.0 + (context.game.random() * 5.0).floor();
    let fire_hit = context.weapons.fire_hit;
    if fire_hit(actor.clone(), &mut *context.game, aim, damage, 40.0) {
        context.game.sound(&actor, "brain/melee3.wav", 1, 1.0, 1.0);
    } else {
        let now = context.game.host.now();
        context.state_mut().melee_time = now + 3.0;
    }
}

/// Hit right (`brain_hit_right`).
fn brain_hit_right(context: &mut MonsterContext) {
    brain_hit(context, true);
}

/// Hit left (`brain_hit_left`).
fn brain_hit_left(context: &mut MonsterContext) {
    brain_hit(context, false);
}

/// Eye update (`eye`).
fn brain_eye_update(tip: ActorId, game: &mut Q2GameServices, positions: &[Vec3], update: bool) {
    let owner = game.require_entity(&tip).owner.clone();
    let Some(owner) = owner.filter(|owner| game.monsters.states.contains_key(owner)) else {
        free_monster_beam(tip, game);
        return;
    };
    let frame = game.require_entity(&owner).frame;
    let index = usize::try_from(frame - brain_frame::WALK101)
        .ok()
        .and_then(|index| positions.get(index));
    let Some(position) = index else {
        free_monster_beam(tip, game);
        return;
    };
    let mut context = MonsterContext::new(owner, game);
    let body = context.game.body_of(context.actor().clone());
    let axes = angles_vectors(body.angles);
    let start = add3(
        add3(
            add3(body.origin, scale3(axes.right, position.x)),
            scale3(axes.forward, position.y),
        ),
        scale3(axes.up, position.z),
    );
    let offset = 0.1 + context.game.random() * 0.1;
    let Some(direction) = predicted_direction(&mut context, start, 0.0, false, offset) else {
        return;
    };
    let game = &mut *context.game;
    let mut moved = game.body_of(tip.clone());
    moved.origin = start;
    game.write_body(tip.clone(), &moved, true);
    game.require_entity_mut(&tip).movedir = direction;
    if update {
        update_monster_beam(&tip, game, false);
    }
}

/// Right eye update (`rightEye`).
fn brain_right_eye_update(tip: ActorId, game: &mut Q2GameServices) {
    brain_eye_update(tip, game, &XATRIX_BRAIN_RIGHT_EYE, false);
}

/// Left eye update (`leftEye`).
fn brain_left_eye_update(tip: ActorId, game: &mut Q2GameServices) {
    brain_eye_update(tip, game, &XATRIX_BRAIN_LEFT_EYE, true);
}

/// Sight (`sight`).
fn brain_sight(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "brain/brnsght1.wav", 2, 1.0, 1.0);
}

/// Search (`search`).
fn brain_search(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "brain/brnsrch1.wav", 2, 1.0, 1.0);
}

/// Idle (`idle`).
fn brain_idle(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "brain/brnlens1.wav", 0, 1.0, 2.0);
    context.set_move("brain_move_idle", true);
}

/// Melee (`melee`).
fn brain_melee(context: &mut MonsterContext) {
    let first = context.game.random() <= 0.5;
    context.set_move(
        if first {
            "brain_move_attack1"
        } else {
            "brain_move_attack2"
        },
        true,
    );
}

/// Initialize (`initialize`).
fn rerelease_brain_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let spawn = context.game.require_entity(&actor).spawn.clone();
    let armor_type = number_field(&spawn, "power_armor_type", 1.0);
    let cells = number_field(&spawn, "power_armor_power", 100.0);
    monster_power_armor(
        context,
        if armor_type == 2.0 {
            PowerArmorKind::Shield
        } else {
            PowerArmorKind::Screen
        },
        cells,
    );
    if armor_type == 0.0 {
        brain_screen(context, false);
    }
}

/// Attack (`attack`).
fn rerelease_brain_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if target_distance(context) <= 440.0 {
        if context.game.random() < 0.5 {
            context.set_move("brain_move_attack3", true);
            return;
        }
        if context.game.require_entity(&actor).spawnflags & 8 == 0 {
            context.set_move("brain_move_attack4", true);
        }
        return;
    }
    if context.game.require_entity(&actor).spawnflags & 8 == 0 {
        context.set_move("brain_move_attack4", true);
    }
}

/// Duck (`duck`).
fn rerelease_brain_duck(context: &mut MonsterContext, _eta: f64) -> bool {
    context.set_move("brain_move_duck", true);
    true
}

/// Pain (`pain`).
fn rerelease_brain_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.game.require_entity(&actor).max_health;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { 1 } else { 0 };
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let choice = context.game.random();
    context.game.sound(
        &actor,
        if choice < 0.33 || choice >= 0.66 {
            "brain/brnpain1.wav"
        } else {
            "brain/brnpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    if !reacts_to_pain(context) {
        return;
    }
    context.set_move(
        if choice < 0.33 {
            "brain_move_pain1"
        } else if choice < 0.66 {
            "brain_move_pain2"
        } else {
            "brain_move_pain3"
        },
        true,
    );
    if context.state().ducked {
        set_duck(context, false);
    }
}

/// Die (`die`).
fn rerelease_brain_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).effects = 0;
    brain_screen(context, false);
    if check_gib(context) {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        context.game.require_entity_mut(&actor).skin /= 2;
        let beams = [
            context.game.require_entity(&actor).beam.clone(),
            context.game.require_entity(&actor).beam2.clone(),
        ];
        for beam in beams.into_iter().flatten() {
            if context.game.entity(&beam).is_some() {
                free_monster_beam(beam, &mut *context.game);
            }
        }
        let damage = reaction.pain.damage;
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/bone/tris.md2",
            damage,
            Q2GibOptions::default(),
        );
        for _ in 0..2 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/sm_meat/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
        }
        for part in ["arm", "arm", "boot", "door", "door"] {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                &format!("models/monsters/brain/gibs/{part}.md2"),
                damage,
                Q2GibOptions {
                    skinned: true,
                    upright: true,
                    ..Q2GibOptions::default()
                },
            );
        }
        for part in ["pelvis", "chest"] {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                &format!("models/monsters/brain/gibs/{part}.md2"),
                damage,
                Q2GibOptions {
                    skinned: true,
                    ..Q2GibOptions::default()
                },
            );
        }
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/brain/gibs/head.md2",
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
    context.game.sound(&actor, "brain/brndeth1.wav", 2, 1.0, 1.0);
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
    let first = context.game.random() <= 0.5;
    context.set_move(
        if first {
            "brain_move_death1"
        } else {
            "brain_move_death2"
        },
        true,
    );
}

/// Chest open (`brain_chest_open`).
fn brain_chest_open(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).count = 0;
    brain_screen(context, false);
    context.game.sound(&actor, "brain/brnatck1.wav", 4, 1.0, 1.0);
}

/// Tentacle attack (`brain_tentacle_attack`).
fn brain_tentacle_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let damage = 10.0 + (context.game.random() * 5.0).floor();
    let fire_hit = context.weapons.fire_hit;
    if fire_hit(actor.clone(), &mut *context.game, vec3(80.0, 0.0, 8.0), damage, -600.0) {
        context.game.require_entity_mut(&actor).count = 1;
    } else {
        let now = context.game.host.now();
        context.state_mut().melee_time = now + 3.0;
    }
    context.game.sound(&actor, "brain/brnatck3.wav", 1, 1.0, 1.0);
}

/// Chest closed (`brain_chest_closed`).
fn brain_chest_closed(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    brain_screen(context, true);
    if context.game.require_entity(&actor).count != 0 {
        context.game.require_entity_mut(&actor).count = 0;
        context.set_move("brain_move_attack1", true);
    }
}

/// Tongue attack (`brain_tounge_attack`).
fn brain_tongue_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let Some(enemy_id) = context.entity().enemy.clone() else {
        return;
    };
    let start = project_flash(context, vec3(24.0, 0.0, 16.0), None);
    let ends = [
        enemy.origin,
        vec3(
            enemy.origin.x,
            enemy.origin.y,
            enemy.origin.z + enemy.bounds.max.z - 8.0,
        ),
        vec3(
            enemy.origin.x,
            enemy.origin.y,
            enemy.origin.z + enemy.bounds.min.z + 8.0,
        ),
    ];
    if !ends.iter().any(|end| brain_tongue_allowed(start, *end)) {
        return;
    }
    let end = enemy.origin;
    let trace = context.game.host.trace(&Q2TraceRequest {
        start,
        end,
        bounds: None,
        ignore: Some(actor.clone()),
        mask: 0x4600_4003,
        exclude: Vec::new(),
    });
    if !matches!(&trace.hit, TraceHit::Actor { actor: hit } if hit == &enemy_id) {
        return;
    }
    context.game.sound(&actor, "brain/brnatck3.wav", 1, 1.0, 1.0);
    context.game.host_emit(Q2PresentationEvent::MonsterBeam {
        effect: Q2MonsterBeam::Parasite,
        actor: actor.clone(),
        start,
        end,
    });
    context.game.damage(
        enemy_id.clone(),
        actor.clone(),
        Some(actor.clone()),
        5.0,
        0.0,
        sub3(start, end),
        end,
        vec3(0.0, 0.0, 0.0),
        8,
        36,
        None,
    );
    let body = context.game.body_of(actor.clone());
    let mut moved = body.clone();
    moved.origin.z += 1.0;
    context.game.write_body(actor.clone(), &moved, true);
    if let Some(target) = context.game.host.actors().resolve_owned(&enemy_id) {
        let mut stunned = enemy.clone();
        stunned.velocity = scale3(angles_vectors(body.angles).forward, -1200.0);
        context.game.host.bodies().write(&target, &stunned);
    }
}

/// Laser beam (`brain_laserbeam`).
fn brain_laserbeam(context: &mut MonsterContext) {
    fire_monster_beam(context, 1.0, false, brain_right_eye_update);
    fire_monster_beam(context, 1.0, true, brain_left_eye_update);
}

/// Laser beam reattack (`brain_laserbeam_reattack`).
fn brain_laserbeam_reattack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    if context.game.random() < 0.5 && visible(context, None) && health(&mut *context.game, enemy.as_ref()) > 0.0 {
        context.game.require_entity_mut(&actor).frame = brain_frame::WALK101;
    }
}

/// Shrink (`brain_shrink`).
fn brain_shrink(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).server_flags |= 2;
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.max.z = 0.0;
    context.game.write_body(actor, &moved, true);
}

/// Create the rerelease brain definition (`createRereleaseBrainDefinition`).
pub fn create_rerelease_brain_definition() -> Q2MonsterDefinition {
    let base = brain_definition();
    let mut definition = Q2MonsterDefinition::new(
        &base.classname,
        &base.kind,
        &base.model,
        300.0,
        -150.0,
        400.0,
        base.bounds.clone(),
        1.0,
        &base.initial_move,
        brain_moves(),
        base.stand.clone(),
        base.walk.clone(),
        MonsterHandler::Callback(rerelease_brain_run),
        base.attack.clone(),
        rerelease_brain_die,
    );
    definition.sight = Some(MonsterHandler::Callback(brain_sight));
    definition.search = Some(MonsterHandler::Callback(brain_search));
    definition.idle = Some(MonsterHandler::Callback(brain_idle));
    definition.melee = Some(MonsterHandler::Callback(brain_melee));
    definition.has_ranged_attack = true;
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks
        .think
        .insert("rerelease.brain.right_eye_update", brain_right_eye_update);
    source_callbacks
        .think
        .insert("rerelease.brain.left_eye_update", brain_left_eye_update);
    source_callbacks.think.insert("beam_think", free_monster_beam);
    definition.source_callbacks = Some(source_callbacks);
    definition.initialize = Some(MonsterHandler::Callback(rerelease_brain_initialize));
    definition.restore = Some(MonsterHandler::Callback(restore_monster_power_armor));
    definition.attack = MonsterHandler::Callback(rerelease_brain_attack);
    definition.duck = Some(rerelease_brain_duck);
    definition.pain = Some(rerelease_brain_pain);
    definition.callbacks = base.callbacks.clone();
    for (name, handler) in [
        ("brain_run", MonsterHandler::Callback(rerelease_brain_run)),
        ("brain_dead", MonsterHandler::Callback(corpse)),
        ("brain_hit_right", MonsterHandler::Callback(brain_hit_right)),
        ("brain_hit_left", MonsterHandler::Callback(brain_hit_left)),
        ("brain_chest_open", MonsterHandler::Callback(brain_chest_open)),
        ("brain_tentacle_attack", MonsterHandler::Callback(brain_tentacle_attack)),
        ("brain_chest_closed", MonsterHandler::Callback(brain_chest_closed)),
        ("brain_tounge_attack", MonsterHandler::Callback(brain_tongue_attack)),
        ("brain_laserbeam", MonsterHandler::Callback(brain_laserbeam)),
        (
            "brain_laserbeam_reattack",
            MonsterHandler::Callback(brain_laserbeam_reattack),
        ),
        ("brain_shrink", MonsterHandler::Callback(brain_shrink)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
