//! Rerelease makron (`src/content/q2/rerelease/monsters/base-variants/makron.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, normalize3, scale3, sub3, vec3};

use super::super::common::{chainfist, check_gib, monster_flash, reacts_to_pain};
use super::super::tables::boss32::{boss32_frame, boss32_moves};
use crate::q2::base::monsters::boss_common::stop_loop;
use crate::q2::base::monsters::makron::makron_definition;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2Edition, Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2SoundEvent,
    Q2SoundLoop,
};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, corpse, enemy_eye, health, project_flash, vector_angles,
};
use crate::q2::foundation::monsters::gibs::{Q2GibOptions, throw_gib};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::perception::{
    Q2AttackChanceProfile, check_attack_with_profile, found_target,
};
use crate::q2::foundation::monsters::types::{
    MonsterContext, MonsterHandler, Q2MonsterDefinition,
};
use crate::q2::foundation::monsters::monster_spawn;
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::support::contracts::{
    CombatTraitChanges, DeathReaction, PainReaction,
};

/// Torso think (`torsoThink`).
fn makron_torso_think(torso: ActorId, game: &mut Q2GameServices) {
    let frame = game.require_entity(&torso).frame + 1;
    game.require_entity_mut(&torso).frame = if frame >= 365 { 346 } else { frame };
    let angles = game.body_of(torso.clone()).angles;
    if angles.x > 0.0 {
        let mut moved = game.body_of(torso.clone());
        moved.angles.x = 0.0f32.max(angles.x - 15.0);
        game.write_body(torso.clone(), &moved, true);
    }
    game.show(torso.clone());
    game.schedule(torso, 0.1, makron_torso_think);
}

/// Spawn torso (`spawnTorso`).
fn makron_spawn_torso(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let Some(torso) = throw_gib(
        actor.clone(),
        &mut *context.game,
        "models/monsters/boss3/rider/tris.md2",
        0.0,
        Q2GibOptions::default(),
    ) else {
        return;
    };
    let body = context.game.body_of(actor);
    let axes = angles_vectors(body.angles);
    let height = body.bounds.max.z - context.game.body_of(torso.clone()).bounds.max.z;
    let entity = context.game.require_entity_mut(&torso);
    entity.frame = 346;
    entity.skin = 1;
    entity.effects = 2;
    entity.angular_velocity = vec3(0.0, 0.0, 0.0);
    entity.sound = "makron/spine.wav".to_string();
    let mut moved = context.game.body_of(torso.clone());
    moved.origin = add3(
        vec3(body.origin.x, body.origin.y, body.origin.z + height - 15.0),
        scale3(axes.forward, -10.0),
    );
    moved.angles = vec3(body.angles.x, body.angles.y, 90.0);
    moved.velocity = add3(
        moved.velocity,
        add3(scale3(axes.up, 120.0), scale3(axes.forward, -120.0)),
    );
    context.game.write_body(torso.clone(), &moved, true);
    context.game.set_motion_kind(torso.clone(), Q2MotionKind::Toss);
    context.game.show(torso.clone());
    let origin = context.game.body_of(torso.clone()).origin;
    context.game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(torso.clone()),
        origin,
        path: "makron/spine.wav".to_string(),
        channel: 0,
        volume: 1.0,
        attenuation: 1.0,
        reliable: false,
        loop_: Q2SoundLoop::Start,
        loop_owner: None,
    }));
    context.game.schedule(torso, 0.1, makron_torso_think);
}

/// Toss the makron (`tossRereleaseMakron`).
pub fn toss_rerelease_makron(context: &mut MonsterContext) -> ActorId {
    let actor = context.actor().clone();
    let child = context.game.create("monster_makron", std::collections::BTreeMap::new());
    let entity = context.game.require_entity(&actor);
    let (target, enemy) = (entity.target.clone(), entity.enemy.clone());
    let child_entity = context.game.require_entity_mut(&child);
    child_entity.target = target;
    child_entity.enemy = enemy;
    let mut moved = context.game.body_of(child.clone());
    moved.origin = context.game.body_of(actor).origin;
    context.game.write_body(child.clone(), &moved, false);
    if !monster_spawn(child.clone(), &mut *context.game) {
        panic!("Makron must be registered before Jorg");
    }
    if let Some(think) = context.game.require_entity(&child).think {
        think(child.clone(), &mut *context.game);
    }
    let child_enemy = context.game.require_entity(&child).enemy.clone();
    let current = context.game.monsters.perception.sight_client.clone();
    let enemy = if child_enemy.is_some() && health(&mut *context.game, child_enemy.as_ref()) > 0.0 {
        child_enemy
    } else {
        current
    };
    let target = enemy.as_ref().and_then(|enemy| context.game.host.bodies().read(enemy));
    if target.is_none() || !context.game.monsters.states.contains_key(&child) {
        return child;
    }
    let target = target.expect("makron target");
    let body = context.game.body_of(child.clone());
    let difference = sub3(target.origin, body.origin);
    let mut moved = body.clone();
    moved.angles.y = vector_angles(difference).y;
    let flat = scale3(normalize3(difference), 400.0);
    moved.velocity = vec3(flat.x, flat.y, 200.0);
    moved.ground = None;
    context.game.write_body(child.clone(), &moved, true);
    context.game.require_entity_mut(&child).enemy = enemy;
    let mut resumed = MonsterContext::new(child.clone(), &mut *context.game);
    found_target(&mut resumed);
    resumed.set_move("makron_move_sight", true);
    resumed.game.require_entity_mut(&child).frame = boss32_frame::ACTIVE01;
    resumed.state_mut().next_frame = boss32_frame::ACTIVE01;
    child
}

/// Initialize (`initialize`).
fn rerelease_makron_initialize(context: &mut MonsterContext) {
    context.state_mut().ignore_shots = true;
}

/// Check attack (`checkAttack`).
fn rerelease_makron_check_attack(context: &mut MonsterContext) -> bool {
    check_attack_with_profile(
        context,
        &Q2AttackChanceProfile {
            stand_ground: 0.4,
            melee: 0.8,
            near: 0.4,
            mid: 0.2,
            far: 0.0,
            strafe_scalar: 0.0,
        },
    )
}

/// Pain (`pain`).
fn rerelease_makron_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.game.require_entity(&actor).max_health;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { 1 } else { 0 };
    let damage = reaction.damage;
    if context.state().current_move.name == "makron_move_sight"
        || context.game.host.now() < context.state().pain_time
        || !chainfist(context) && damage <= 25.0 && context.game.random() < 0.2
    {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let mut heavy = false;
    if damage <= 40.0 {
        context.game.sound(&actor, "makron/pain3.wav", 2, 1.0, 0.0);
    } else if damage <= 110.0 {
        context.game.sound(&actor, "makron/pain2.wav", 2, 1.0, 0.0);
    } else if context.game.random() <= if damage <= 150.0 { 0.45 } else { 0.35 } {
        heavy = true;
        context.game.sound(&actor, "makron/pain1.wav", 2, 1.0, 0.0);
    }
    if !reacts_to_pain(context) {
        return;
    }
    if damage <= 40.0 {
        context.set_move("makron_move_pain4", true);
    } else if damage <= 110.0 {
        context.set_move("makron_move_pain5", true);
    } else if heavy {
        context.set_move("makron_move_pain6", true);
    }
}

/// Die (`die`).
fn rerelease_makron_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    stop_loop(context);
    context.game.require_entity_mut(&actor).sound = String::new();
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
    context.game.require_entity_mut(&actor).server_flags |= 2;
    let owned = context.game.owned_of(actor.clone());
    context.game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    context.set_move("makron_move_death2", true);
    makron_spawn_torso(context);
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.min = vec3(-60.0, -60.0, 0.0);
    moved.bounds.max = vec3(60.0, 60.0, 48.0);
    context.game.write_body(actor, &moved, true);
}

/// Dead (`makron_dead`).
fn makron_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    corpse(context);
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.min = vec3(-60.0, -60.0, 0.0);
    moved.bounds.max = vec3(60.0, 60.0, 24.0);
    context.game.write_body(actor, &moved, true);
}

/// Save location (`MakronSaveloc`).
fn makron_saveloc(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if let Some(eye) = enemy_eye(context) {
        context.game.require_entity_mut(&actor).pos1 = eye;
    }
}

/// Railgun (`MakronRailgun`).
fn makron_railgun(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, 119), None);
    let pos1 = context.game.require_entity(&actor).pos1;
    let direction = normalize3(sub3(pos1, start));
    let fire_rail = context.weapons.fire_rail;
    fire_rail(actor, &mut *context.game, start, direction, 50.0, 100.0);
    monster_flash(context, 119, start, direction);
}

/// Hyperblaster (`MakronHyperblaster`).
fn makron_hyperblaster(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let flash = 102 + frame - boss32_frame::ATTAK405;
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, flash as usize), None);
    let eye = enemy_eye(context);
    let yaw = context.game.body_of(actor).angles.y;
    let pitch = eye.map(|eye| vector_angles(sub3(eye, start)).x).unwrap_or(0.0);
    let sweep = if frame <= boss32_frame::ATTAK413 {
        f64::from(yaw) - 10.0 * f64::from(frame - boss32_frame::ATTAK413)
    } else {
        f64::from(yaw) + 10.0 * f64::from(frame - boss32_frame::ATTAK421)
    };
    let direction = angles_vectors(vec3(pitch, sweep as f32, 0.0)).forward;
    let actor = context.actor().clone();
    let fire_blaster = context.weapons.fire_blaster;
    fire_blaster(actor, &mut *context.game, start, direction, 15.0, 1000.0, 8, false, Mod::BLASTER);
    monster_flash(context, flash, start, direction);
}

/// Create the rerelease makron definition (`rereleaseMakronDefinition`).
pub fn rerelease_makron_definition() -> Q2MonsterDefinition {
    let mut definition = makron_definition();
    definition.moves = boss32_moves();
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks.think.insert("rerelease.makron.makron_torso_think", makron_torso_think);
    definition.source_callbacks = Some(source_callbacks);
    definition.initialize = Some(MonsterHandler::Callback(rerelease_makron_initialize));
    definition.check_attack = Some(rerelease_makron_check_attack);
    definition.pain = Some(rerelease_makron_pain);
    definition.die = rerelease_makron_die;
    for (name, handler) in [
        ("makron_dead", MonsterHandler::Callback(makron_dead)),
        (
            "makron_spawn_torso",
            MonsterHandler::Callback(makron_spawn_torso),
        ),
        ("MakronSaveloc", MonsterHandler::Callback(makron_saveloc)),
        ("MakronRailgun", MonsterHandler::Callback(makron_railgun)),
        (
            "MakronHyperblaster",
            MonsterHandler::Callback(makron_hyperblaster),
        ),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
