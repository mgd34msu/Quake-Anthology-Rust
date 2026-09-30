//! Makron monster (`src/content/q2/base/monsters/makron.ts`).
//!
//! Quake II m_boss32.c. id Software, GPL-2.0-or-later.

use std::collections::{BTreeMap, HashMap};

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, normalize3, scale3, sub3, vec3};

use super::boss_common::{boss_check_attack, stop_loop};
use super::common::{damaged_skin, finish_corpse, move_handler, monster_muzzle, monster_shot, sound_handler};
use super::tables::boss32::{boss32_frame, boss32_moves};
use crate::q2::foundation::host::{
    Q2GameServices, Q2MotionKind, Q2Solid, Q2SoundEvent, Q2SoundLoop, Q2PresentationEvent,
};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, enemy_eye, health, project_flash, vector_angles,
};
use crate::q2::foundation::monsters::gibs::{Q2GibOptions, throw_gib};
use crate::q2::foundation::monsters::monster_spawn;
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Run (`run`).
fn makron_run(context: &mut MonsterContext) {
    if context.state().stand_ground {
        context.set_move("makron_move_stand", false);
    } else {
        context.set_move("makron_move_run", false);
    }
}

/// Makron spawn think (`q2:base/MakronSpawn`).
pub fn makron_spawn_think(actor: ActorId, game: &mut Q2GameServices) {
    if !monster_spawn(actor.clone(), game) {
        panic!("Makron definition must be registered before Jorg");
    }
    let player = game.monsters.perception.sight_client.clone();
    let target = player.as_ref().and_then(|player| game.host.bodies().read(player));
    let Some(target) = target else { return };
    let mut body = game.body_of(actor.clone());
    let direction = sub3(target.origin, body.origin);
    body.angles.y = vector_angles(direction).y;
    let flat = scale3(normalize3(direction), 400.0);
    body.velocity = vec3(flat.x, flat.y, 200.0);
    body.ground = None;
    game.write_body(actor, &body, true);
}

/// Makron torso think (`q2:base/makron_torso_think`).
pub fn makron_torso_think(actor: ActorId, game: &mut Q2GameServices) {
    let frame = game.require_entity(&actor).frame + 1;
    game.require_entity_mut(&actor).frame = if frame >= 365 { 346 } else { frame };
    game.show(actor.clone());
    game.schedule(actor, 0.1, makron_torso_think);
}

/// Attach makron spawn callbacks (`withMakronSpawnCallbacks`).
pub fn with_makron_spawn_callbacks(mut definition: Q2MonsterDefinition) -> Q2MonsterDefinition {
    let mut callbacks = definition.source_callbacks.take().unwrap_or_default();
    callbacks.think.insert("q2:base/MakronSpawn", makron_spawn_think);
    callbacks.think.insert("q2:base/makron_torso_think", makron_torso_think);
    definition.source_callbacks = Some(callbacks);
    definition
}

/// Toss a makron (`makronToss`).
pub fn makron_toss(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let child = context.game.create("monster_makron", BTreeMap::new());
    let target = context.entity().target.clone();
    context.game.require_entity_mut(&child).target = target;
    let origin = context.game.body_of(actor).origin;
    let mut body = context.game.body_of(child.clone());
    body.origin = origin;
    context.game.write_body(child.clone(), &body, false);
    context.game.schedule(child, 0.8, makron_spawn_think);
}

/// Attack (`attack`).
fn makron_attack(context: &mut MonsterContext) {
    let r = context.game.random();
    context.set_move(
        if r <= 0.3 {
            "makron_move_attack3"
        } else if r <= 0.6 {
            "makron_move_attack4"
        } else {
            "makron_move_attack5"
        },
        false,
    );
}

/// Pain (`pain`).
fn makron_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time
        || reaction.damage <= 25.0 && context.game.random() < 0.2
    {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if context.game.options.skill == 3 {
        return;
    }
    let (animation, path) = if reaction.damage <= 40.0 {
        ("makron_move_pain4", "makron/pain3.wav")
    } else if reaction.damage <= 110.0 {
        ("makron_move_pain5", "makron/pain2.wav")
    } else {
        // The original dangling else belongs to the first random test,
        // inside damage <= 150.
        if reaction.damage > 150.0 {
            return;
        }
        if context.game.random() > 0.45 && context.game.random() > 0.35 {
            return;
        }
        ("makron_move_pain6", "makron/pain1.wav")
    };
    let actor = context.actor().clone();
    context.game.sound(&actor, path, 2, 1.0, 0.0);
    context.set_move(animation, false);
}

/// Die (`die`).
fn makron_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    stop_loop(context);
    let actor = context.actor().clone();
    let gib_health = context.state().gib_health;
    if health(&mut *context.game, Some(&actor)) <= gib_health {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        let damage = reaction.pain.damage;
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_meat/tris.md2",
            damage,
            Q2GibOptions::default(),
        );
        for _ in 0..4 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/sm_metal/tris.md2",
                damage,
                Q2GibOptions {
                    metallic: true,
                    ..Q2GibOptions::default()
                },
            );
        }
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/gear/tris.md2",
            damage,
            Q2GibOptions {
                metallic: true,
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
    context.game.sound(&actor, "makron/death.wav", 2, 1.0, 0.0);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor.clone());
    context.game.host.combat().set_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    let torso = context.game.create("makron_torso", BTreeMap::new());
    let body = context.game.body_of(actor);
    {
        let torso_entity = context.game.require_entity_mut(&torso);
        torso_entity.model = "models/monsters/boss3/rider/tris.md2".to_string();
        torso_entity.frame = 346;
    }
    let mut torso_body = context.game.body_of(torso.clone());
    torso_body.origin = vec3(body.origin.x, body.origin.y - 84.0, body.origin.z);
    torso_body.angles = body.angles;
    torso_body.bounds = Bounds {
        min: Vec3 { x: -8.0, y: -8.0, z: 0.0 },
        max: Vec3 { x: 8.0, y: 8.0, z: 8.0 },
    };
    torso_body.velocity = vec3(0.0, 0.0, 0.0);
    context.game.write_body(torso.clone(), &torso_body, true);
    context.game.set_motion_kind(torso.clone(), Q2MotionKind::Stationary);
    context.game.set_solid(torso.clone(), Q2Solid::None);
    context.game.show(torso.clone());
    let torso_origin = context.game.body_of(torso.clone()).origin;
    context.game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(torso.clone()),
        origin: torso_origin,
        path: "makron/spine.wav".to_string(),
        channel: 0,
        volume: 1.0,
        attenuation: 1.0,
        reliable: false,
        loop_: Q2SoundLoop::Start,
        loop_owner: None,
    }));
    context.game.schedule(torso, 0.2, makron_torso_think);
    context.set_move("makron_move_death2", false);
}

/// Dead (`makron_dead`).
fn makron_dead(context: &mut MonsterContext) {
    finish_corpse(
        context,
        Bounds {
            min: Vec3 { x: -60.0, y: -60.0, z: 0.0 },
            max: Vec3 { x: 60.0, y: 60.0, z: 72.0 },
        },
    );
}

/// Taunt (`makron_taunt`).
fn makron_taunt(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let r = context.game.random();
    context.game.sound(
        &actor,
        if r <= 0.3 {
            "makron/voice4.wav"
        } else if r <= 0.6 {
            "makron/voice3.wav"
        } else {
            "makron/voice.wav"
        },
        0,
        1.0,
        0.0,
    );
}

/// BFG (`makronBFG`).
fn makron_bfg(context: &mut MonsterContext) {
    let Some((start, direction)) = monster_shot(context, 101, 0.0) else {
        return;
    };
    let actor = context.actor().clone();
    context.game.sound(&actor, "makron/bfg_fire.wav", 2, 1.0, 1.0);
    let fire_bfg = context.weapons.fire_bfg;
    fire_bfg(actor, &mut *context.game, start, direction, 50.0, 300.0, 300.0);
    monster_muzzle(context, 101, direction, start);
}

/// Save location (`MakronSaveloc`).
fn makron_save_loc(context: &mut MonsterContext) {
    if let Some(eye) = enemy_eye(context) {
        context.state_mut().blind_fire_target = eye;
    }
}

/// Railgun (`MakronRailgun`).
fn makron_railgun(context: &mut MonsterContext) {
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, 119), None);
    let target = context.state().blind_fire_target;
    let direction = normalize3(sub3(target, start));
    let fire_rail = context.weapons.fire_rail;
    let actor = context.actor().clone();
    fire_rail(actor, &mut *context.game, start, direction, 50.0, 100.0);
    monster_muzzle(context, 119, direction, start);
}

/// Hyperblaster (`MakronHyperblaster`).
fn makron_hyperblaster(context: &mut MonsterContext) {
    let frame = context.entity().frame;
    let flash = 102 + frame - boss32_frame::ATTAK405;
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, flash as usize), None);
    let eye = enemy_eye(context);
    let actor = context.actor().clone();
    let yaw = context.game.body_of(actor.clone()).angles.y;
    let pitch = eye.map(|eye| vector_angles(sub3(eye, start)).x).unwrap_or(0.0);
    let sweep = if frame <= boss32_frame::ATTAK413 {
        yaw - 10.0 * (frame - boss32_frame::ATTAK413) as f32
    } else {
        yaw + 10.0 * (frame - boss32_frame::ATTAK421) as f32
    };
    let direction = angles_vectors(vec3(pitch, sweep, 0.0)).forward;
    let fire_blaster = context.weapons.fire_blaster;
    fire_blaster(actor, &mut *context.game, start, direction, 15.0, 1000.0, 8, false, Mod::BLASTER);
    monster_muzzle(context, 102, direction, start);
}

/// Check attack (`checkAttack`).
fn makron_check_attack(context: &mut MonsterContext) -> bool {
    boss_check_attack(context, false, false)
}

/// Makron definition (`makronDefinition`).
pub fn makron_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_makron",
        "makron",
        "models/monsters/boss3/rider/tris.md2",
        3000.0,
        -2000.0,
        500.0,
        Bounds {
            min: Vec3 { x: -30.0, y: -30.0, z: 0.0 },
            max: Vec3 { x: 30.0, y: 30.0, z: 90.0 },
        },
        1.0,
        "makron_move_sight",
        boss32_moves(),
        move_handler("makron_move_stand"),
        move_handler("makron_move_walk"),
        MonsterHandler::Callback(makron_run),
        MonsterHandler::Callback(makron_attack),
        makron_die,
    );
    definition.sight = Some(move_handler("makron_move_sight"));
    definition.pain = Some(makron_pain);
    definition.check_attack = Some(makron_check_attack);
    definition.callbacks = HashMap::from([
        (
            "makron_run".to_string(),
            MonsterHandler::Callback(makron_run),
        ),
        (
            "makron_dead".to_string(),
            MonsterHandler::Callback(makron_dead),
        ),
        (
            "makron_step_left".to_string(),
            sound_handler("makron/step1.wav", 4, 1.0),
        ),
        (
            "makron_step_right".to_string(),
            sound_handler("makron/step2.wav", 4, 1.0),
        ),
        (
            "makron_popup".to_string(),
            sound_handler("makron/popup.wav", 4, 0.0),
        ),
        (
            "makron_hit".to_string(),
            sound_handler("makron/bhit.wav", 0, 0.0),
        ),
        (
            "makron_brainsplorch".to_string(),
            sound_handler("makron/brain1.wav", 2, 1.0),
        ),
        (
            "makron_prerailgun".to_string(),
            sound_handler("makron/rail_up.wav", 1, 1.0),
        ),
        (
            "makron_taunt".to_string(),
            MonsterHandler::Callback(makron_taunt),
        ),
        (
            "makronBFG".to_string(),
            MonsterHandler::Callback(makron_bfg),
        ),
        (
            "MakronSaveloc".to_string(),
            MonsterHandler::Callback(makron_save_loc),
        ),
        (
            "MakronRailgun".to_string(),
            MonsterHandler::Callback(makron_railgun),
        ),
        (
            "MakronHyperblaster".to_string(),
            MonsterHandler::Callback(makron_hyperblaster),
        ),
    ]);
    definition
}
