//! Rerelease boss2 (`src/content/q2/rerelease/monsters/base-variants/boss2.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::math::{add3, length3, normalize3, scale3, sub3, vec3};

use super::super::boss::{boss_explode, boss_explode_think};
use super::super::common::{check_gib, monster_flash, predicted_direction, reacts_to_pain};
use super::super::tables::boss2::boss2_moves;
use super::super::tables::flashes::rerelease_flash;
use crate::q2::base::monsters::boss2::boss2_definition;
use crate::q2::base::monsters::boss_common::stop_loop;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2Edition, Q2EffectEvent, Q2PresentationEvent};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, enemy_body, health, in_front, project_flash, target_distance,
};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::perception::{check_attack_with_profile, Q2AttackChanceProfile};
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Attack machine gun (`attackMachinegun`).
fn boss2_attack_machinegun(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let n64 = context.game.require_entity(&actor).spawnflags & 8 != 0;
    context.set_move(
        if n64 {
            "boss2_move_attack_hb"
        } else {
            "boss2_move_attack_mg"
        },
        true,
    );
}

/// Fire rocket (`fireRocket`).
fn boss2_fire_rocket(context: &mut MonsterContext, predictive: bool) {
    let actor = context.actor().clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let right = angles_vectors(context.game.body_of(actor.clone()).angles).right;
    for (flash, lead, spread, lower) in [
        (78, -0.1, 0.4, true),
        (79, -0.05, 0.025, false),
        (80, 0.05, -0.025, false),
        (81, 0.1, -0.4, true),
    ] {
        let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, flash as usize), None);
        let direction = if predictive {
            predicted_direction(context, start, 750.0, false, lead)
        } else {
            let target = if lower {
                vec3(enemy.origin.x, enemy.origin.y, enemy.origin.z - 15.0)
            } else {
                enemy.origin
            };
            Some(normalize3(add3(
                normalize3(sub3(target, start)),
                scale3(right, spread as f32),
            )))
        };
        let Some(direction) = direction else {
            continue;
        };
        let fire_rocket = context.weapons.fire_rocket;
        fire_rocket(
            actor.clone(),
            &mut *context.game,
            start,
            direction,
            50.0,
            if predictive { 750.0 } else { 500.0 },
            70.0,
            50.0,
        );
        monster_flash(context, flash, start, direction);
    }
}

/// Bullet (`bullet`).
fn boss2_bullet(context: &mut MonsterContext, flash: usize) {
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, flash), None);
    let Some(direction) = predicted_direction(context, start, 0.0, true, -0.2) else {
        return;
    };
    let actor = context.actor().clone();
    let fire_bullet = context.weapons.fire_bullet;
    fire_bullet(actor, &mut *context.game, start, direction, 6.0, 4.0, 900.0, 500.0, 0);
    monster_flash(context, flash as i32, start, direction);
}

/// Fire bullet left (`boss2_firebullet_left`).
fn boss2_firebullet_left(context: &mut MonsterContext) {
    boss2_bullet(context, 73);
}

/// Fire bullet right (`boss2_firebullet_right`).
fn boss2_firebullet_right(context: &mut MonsterContext) {
    boss2_bullet(context, 133);
}

/// Machine gun (`Boss2MachineGun`).
fn boss2_machine_gun(context: &mut MonsterContext) {
    boss2_bullet(context, 73);
    boss2_bullet(context, 133);
}

/// Predictive rocket (`Boss2PredictiveRocket`).
fn boss2_predictive_rocket(context: &mut MonsterContext) {
    boss2_fire_rocket(context, true);
}

/// Gib (`gib`).
fn boss2_gib(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:explosion1-big".to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    stop_loop(context);
    let entity = context.game.require_entity_mut(&actor);
    entity.skin /= 2;
    entity.gravity_vector.z = -1.0;
    for _ in 0..2 {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_meat/tris.md2",
            500.0,
            Q2GibOptions::default(),
        );
    }
    for _ in 0..2 {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_metal/tris.md2",
            500.0,
            Q2GibOptions {
                metallic: true,
                ..Q2GibOptions::default()
            },
        );
    }
    for part in ["chest", "engine", "spine"] {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            &format!("models/monsters/boss2/gibs/{part}.md2"),
            500.0,
            Q2GibOptions {
                skinned: true,
                ..Q2GibOptions::default()
            },
        );
    }
    for part in ["chaingun", "chaingun", "cpu", "rocket", "wing", "wing"] {
        throw_gib(
            actor.clone(),
            &mut *context.game,
            &format!("models/monsters/boss2/gibs/{part}.md2"),
            500.0,
            Q2GibOptions {
                skinned: true,
                upright: true,
                ..Q2GibOptions::default()
            },
        );
    }
    for factor in [1.0, 2.0, 1.35] {
        for part in ["larm", "rarm"] {
            let piece = throw_gib(
                actor.clone(),
                &mut *context.game,
                &format!("models/monsters/boss2/gibs/{part}.md2"),
                500.0,
                Q2GibOptions {
                    skinned: true,
                    upright: true,
                    ..Q2GibOptions::default()
                },
            );
            if let Some(piece) = piece {
                let scale = context.game.require_entity(&actor).scale;
                context.game.require_entity_mut(&piece).scale = (if scale != 0.0 { scale } else { 1.0 }) * factor;
                context.game.show(piece);
            }
        }
    }
    throw_gib(
        actor.clone(),
        &mut *context.game,
        "models/monsters/boss2/gibs/head.md2",
        500.0,
        Q2GibOptions {
            skinned: true,
            metallic: true,
            head: true,
            ..Q2GibOptions::default()
        },
    );
    context.state_mut().dead = true;
    context.state_mut().gibbed = true;
}

/// Check attack (`checkAttack`).
fn rerelease_boss2_check_attack(context: &mut MonsterContext) -> bool {
    check_attack_with_profile(
        context,
        &Q2AttackChanceProfile {
            stand_ground: 0.4,
            melee: 0.8,
            near: 0.8,
            mid: 0.8,
            far: 0.0,
            strafe_scalar: 0.0,
        },
    )
}

/// Initialize (`initialize`).
fn rerelease_boss2_initialize(context: &mut MonsterContext) {
    context.state_mut().ignore_shots = true;
    if let Some(initialize) = boss2_definition().initialize.clone() {
        initialize.dispatch(context);
    }
}

/// Attack (`attack`).
fn rerelease_boss2_attack(context: &mut MonsterContext) {
    if enemy_body(context).is_none() {
        return;
    }
    let actor = context.actor().clone();
    let gun = target_distance(context) <= 125.0 || context.game.random() <= 0.6;
    let n64 = context.game.require_entity(&actor).spawnflags & 8 != 0;
    context.set_move(
        if gun {
            if n64 {
                "boss2_move_attack_hb"
            } else {
                "boss2_move_attack_pre_mg"
            }
        } else if n64 {
            "boss2_move_attack_rocket2"
        } else {
            "boss2_move_attack_rocket"
        },
        true,
    );
}

/// Pain (`pain`).
fn rerelease_boss2_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.game.require_entity(&actor).max_health;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { 1 } else { 0 };
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let damage = reaction.damage;
    context.game.sound(
        &actor,
        if damage < 10.0 {
            "bosshovr/bhvpain3.wav"
        } else if damage < 30.0 {
            "bosshovr/bhvpain1.wav"
        } else {
            "bosshovr/bhvpain2.wav"
        },
        2,
        1.0,
        0.0,
    );
    if !reacts_to_pain(context) {
        return;
    }
    context.set_move(
        if damage < 30.0 {
            "boss2_move_pain_light"
        } else {
            "boss2_move_pain_heavy"
        },
        true,
    );
}

/// Die (`die`).
fn rerelease_boss2_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if context.game.require_entity(&actor).spawnflags & 65536 != 0 {
        if check_gib(context) {
            boss2_gib(context);
            return;
        }
        if context.state().dead {
            return;
        }
    } else {
        context.game.sound(&actor, "bosshovr/bhvdeth1.wav", 2, 1.0, 0.0);
        context.state_mut().dead = true;
        context.state_mut().can_take_damage = false;
        context.game.require_entity_mut(&actor).count = 0;
        let owned = context.game.owned_of(actor.clone());
        context.game.set_combat_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: Some(false),
                ..CombatTraitChanges::default()
            },
        );
        let mut moved = context.game.body_of(actor.clone());
        moved.velocity = vec3(0.0, 0.0, 0.0);
        context.game.write_body(actor.clone(), &moved, true);
        let entity = context.game.require_entity_mut(&actor);
        entity.gravity_vector.z *= 0.3;
    }
    context.set_move("boss2_move_death", true);
}

/// Reattack machine gun (`boss2_reattack_mg`).
fn boss2_reattack_mg(context: &mut MonsterContext) {
    let enemy = context.entity().enemy.clone();
    let again = enemy.as_ref().is_some_and(|enemy| in_front(context, enemy)) && context.game.random() <= 0.7;
    if again {
        boss2_attack_machinegun(context);
    } else {
        context.set_move("boss2_move_attack_post_mg", true);
    }
}

/// Rocket (`Boss2Rocket`).
fn boss2_rocket(context: &mut MonsterContext) {
    let enemy = context.entity().enemy.clone();
    let predictive =
        enemy.as_ref().is_some_and(|enemy| context.game.host.is_player(enemy)) && context.game.random() < 0.9;
    boss2_fire_rocket(context, predictive);
}

/// N64 rocket (`Boss2Rocket64`).
fn boss2_rocket64(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let right = angles_vectors(context.game.body_of(actor.clone()).angles).right;
    let authored = context.game.require_entity(&actor).scale;
    let size = if authored != 0.0 { authored } else { 1.0 };
    let muzzle = project_flash(context, muzzle_offset(Q2Edition::Rerelease, 78), None);
    let entity = context.game.require_entity_mut(&actor);
    let count = entity.count;
    entity.count += 1;
    let mut start = vec3(muzzle.x, muzzle.y, muzzle.z + (10.0 * size) as f32);
    start = sub3(start, scale3(right, ((2 + count % 4 * 8) as f64 * size) as f32));
    let enemy_id = context.entity().enemy.clone();
    let player = enemy_id
        .as_ref()
        .is_some_and(|enemy| context.game.host.is_player(enemy));
    let target = if player && context.game.random() < 0.9 {
        add3(
            enemy.origin,
            scale3(
                enemy.velocity,
                (length3(sub3(enemy.origin, start)) as f64 / 750.0 - 0.3) as f32,
            ),
        )
    } else {
        vec3(enemy.origin.x, enemy.origin.y, enemy.origin.z - 15.0)
    };
    let direction = normalize3(sub3(target, start));
    let fire_rocket = context.weapons.fire_rocket;
    fire_rocket(actor, &mut *context.game, start, direction, 35.0, 750.0, 55.0, 35.0);
    monster_flash(context, 78, start, direction);
}

/// Hyper blaster (`Boss2HyperBlaster`).
fn boss2_hyper_blaster(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let frame = context.game.require_entity(&actor).frame;
    let id = if frame & 1 != 0 {
        rerelease_flash::BOSS2_MACHINEGUN_L2
    } else {
        rerelease_flash::BOSS2_MACHINEGUN_R2
    };
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, id as usize), None);
    let enemy_id = context.entity().enemy.clone();
    let view_height = enemy_id
        .as_ref()
        .and_then(|enemy| context.game.entity(enemy))
        .map(|target| target.view_height)
        .unwrap_or(22);
    let target = vec3(enemy.origin.x, enemy.origin.y, enemy.origin.z + view_height as f32);
    let direction = normalize3(sub3(target, start));
    let effects = if frame % 4 == 0 { 64 } else { 0 };
    let fire_blaster = context.weapons.fire_blaster;
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
    monster_flash(context, id, start, direction);
}

/// Shrink (`boss2_shrink`).
fn boss2_shrink(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.max.z = 50.0;
    context.game.write_body(actor, &moved, true);
}

/// Dead (`boss2_dead`).
fn boss2_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.require_entity(&actor).spawnflags & 65536 != 0 {
        context.state_mut().dead = false;
        context.state_mut().can_take_damage = true;
        let owned = context.game.owned_of(actor);
        context.game.set_combat_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: Some(true),
                ..CombatTraitChanges::default()
            },
        );
        return;
    }
    boss2_gib(context);
}

/// Create the rerelease boss2 definition (`rereleaseBoss2Definition`).
pub fn rerelease_boss2_definition() -> Q2MonsterDefinition {
    let mut definition = boss2_definition();
    definition.moves = boss2_moves();
    definition.yaw_speed = Some(50.0);
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks.think.insert("BossExplode_think", boss_explode_think);
    definition.source_callbacks = Some(source_callbacks);
    definition.check_attack = Some(rerelease_boss2_check_attack);
    definition.initialize = Some(MonsterHandler::Callback(rerelease_boss2_initialize));
    definition.attack = MonsterHandler::Callback(rerelease_boss2_attack);
    definition.pain = Some(rerelease_boss2_pain);
    definition.die = rerelease_boss2_die;
    for (name, handler) in [
        ("BossExplode", MonsterHandler::Callback(boss_explode)),
        ("boss2_attack_mg", MonsterHandler::Callback(boss2_attack_machinegun)),
        ("boss2_reattack_mg", MonsterHandler::Callback(boss2_reattack_mg)),
        (
            "Boss2PredictiveRocket",
            MonsterHandler::Callback(boss2_predictive_rocket),
        ),
        ("Boss2Rocket", MonsterHandler::Callback(boss2_rocket)),
        ("Boss2Rocket64", MonsterHandler::Callback(boss2_rocket64)),
        ("boss2_firebullet_left", MonsterHandler::Callback(boss2_firebullet_left)),
        (
            "boss2_firebullet_right",
            MonsterHandler::Callback(boss2_firebullet_right),
        ),
        ("Boss2MachineGun", MonsterHandler::Callback(boss2_machine_gun)),
        ("Boss2HyperBlaster", MonsterHandler::Callback(boss2_hyper_blaster)),
        ("boss2_shrink", MonsterHandler::Callback(boss2_shrink)),
        ("boss2_dead", MonsterHandler::Callback(boss2_dead)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
