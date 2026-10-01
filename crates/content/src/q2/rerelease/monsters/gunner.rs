//! Rerelease gunner (`src/content/q2/rerelease/monsters/gunner.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::math::{add3, length3, normalize3, scale3, sub3};

use super::common::{
    blocked_check_jump, blocked_check_platform, calculate_pitch_to_fire, check_gib, monster_flash,
    monster_jump_finished, predicted_direction, reacts_to_pain, JumpNavigation, JumpResult,
};
use super::tables::flashes::rerelease_flash;
use super::tables::gunner::{gunner_frame, gunner_moves};
use crate::q2::base::monsters::gunner::gunner_definition;
use crate::q2::foundation::host::Q2Edition;
use crate::q2::foundation::monsters::ai::{
    angles_vectors, clear_shot, corpse, enemy_body, finish_dodge, health, project_flash, set_duck, target_distance,
    vector_angles, visible,
};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::foundation::weapons::types::Q2GrenadeAdjustment;
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Run (`run`).
fn rerelease_gunner_run(context: &mut MonsterContext) {
    finish_dodge(context);
    let base = gunner_definition();
    base.run.dispatch(context);
}

/// Jumping (`jumping`).
fn gunner_jumping(context: &mut MonsterContext) -> bool {
    let current = context.state().current_move.name.clone();
    current == "gunner_move_jump" || current == "gunner_move_jump2"
}

/// Shooting (`shooting`).
fn gunner_shooting(context: &mut MonsterContext) -> bool {
    let current = context.state().current_move.name.clone();
    [
        "gunner_move_attack_chain",
        "gunner_move_fire_chain",
        "gunner_move_attack_grenade",
        "gunner_move_attack_grenade2",
    ]
    .contains(&current.as_str())
}

/// Grenade check (`grenadeCheck`).
fn gunner_grenade_check(context: &mut MonsterContext) -> bool {
    let Some(enemy) = enemy_body(context) else {
        return false;
    };
    if !clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 53)) {
        return false;
    }
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, 53), None);
    let target = if context.state().manual_steering {
        context.state().blind_fire_target
    } else {
        enemy.origin
    };
    let delta = sub3(target, start);
    length3(delta) >= 100.0
        && calculate_pitch_to_fire(context, target, start, normalize3(delta), 600.0, 2.5, false, false).is_some()
}

/// Grenade (`grenade`).
fn gunner_grenade(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let blind = context.state().manual_steering;
    let current = context.game.require_entity(&actor).frame;
    let (spread, mut id) = if current == gunner_frame::ATTAK105 || current == gunner_frame::ATTAK309 {
        (-0.1, 53)
    } else if current == gunner_frame::ATTAK108 || current == gunner_frame::ATTAK312 {
        (-0.05, 54)
    } else if current == gunner_frame::ATTAK111 || current == gunner_frame::ATTAK315 {
        (0.05, 55)
    } else {
        context.state_mut().manual_steering = false;
        (0.1, 56)
    };
    if current >= gunner_frame::ATTAK301 && current <= gunner_frame::ATTAK324 {
        id = rerelease_flash::GUNNER_GRENADE2_1 + 56 - id;
    }
    let seen = visible(context, None);
    let target = if blind && !seen {
        context.state().blind_fire_target
    } else {
        enemy.origin
    };
    if blind && !seen && length3(target) == 0.0 {
        return;
    }
    let body = context.game.body_of(actor.clone());
    let axes = angles_vectors(body.angles);
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, id as usize), None);
    let mut delta = sub3(target, body.origin);
    let distance = length3(delta);
    if distance > 512.0 && delta.z < 64.0 && delta.z > -64.0 {
        delta.z += (distance - 512.0) as f32;
    }
    let pitch = (-0.5f64).max(0.4f64.min(f64::from(normalize3(delta).z)));
    let aim = add3(
        add3(axes.forward, scale3(axes.right, spread as f32)),
        scale3(axes.up, pitch as f32),
    );
    let predicted = calculate_pitch_to_fire(context, target, start, aim, 600.0, 2.5, false, false);
    let right = (context.game.random() * 2.0 - 1.0) * 10.0;
    let up = if predicted.is_none() {
        200.0 + (context.game.random() * 2.0 - 1.0) * 10.0
    } else {
        context.game.random() * 10.0
    };
    let direction = predicted.unwrap_or(aim);
    let gravity = context.game.host.gravity();
    let fire_grenade = context.weapons.fire_grenade;
    fire_grenade(
        actor,
        &mut *context.game,
        start,
        direction,
        50.0,
        600.0,
        2.5,
        90.0,
        false,
        false,
        true,
        Some(Q2GrenadeAdjustment { right, up, gravity }),
    );
    monster_flash(context, id, start, direction);
}

/// Jump (`jump`).
fn gunner_jump(context: &mut MonsterContext, up: bool) {
    let actor = context.actor().clone();
    let mut moved = context.game.body_of(actor.clone());
    let axes = angles_vectors(moved.angles);
    moved.velocity = add3(
        moved.velocity,
        add3(
            scale3(axes.forward, if up { 150.0 } else { 100.0 }),
            scale3(axes.up, if up { 400.0 } else { 300.0 }),
        ),
    );
    context.game.write_body(actor, &moved, true);
}

/// Jump now (`gunner_jump_now`).
fn gunner_jump_now(context: &mut MonsterContext) {
    gunner_jump(context, false);
}

/// Jump2 now (`gunner_jump2_now`).
fn gunner_jump2_now(context: &mut MonsterContext) {
    gunner_jump(context, true);
}

/// Sight (`sight`).
fn gunner_sight(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "gunner/sight1.wav", 2, 1.0, 1.0);
}

/// Search (`search`).
fn gunner_search(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "gunner/gunsrch1.wav", 2, 1.0, 1.0);
}

/// Attack (`attack`).
fn rerelease_gunner_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    finish_dodge(context);
    if context.state().attack_state == MonsterAttackState::Blind {
        if context.game.require_entity(&actor).timestamp > context.game.host.now() {
            return;
        }
        let delay = context.state().blind_fire_delay;
        let chance = if delay < 1.0 {
            1.0
        } else if delay < 7.5 {
            0.4
        } else {
            0.1
        };
        let choice = context.game.random();
        context.state_mut().blind_fire_delay = delay + 4.1 + context.game.random() * 3.0;
        if length3(context.state().blind_fire_target) == 0.0 || choice > chance {
            return;
        }
        context.state_mut().manual_steering = true;
        if gunner_grenade_check(context) {
            let second = context.game.random() < 0.5;
            context.set_move(
                if second {
                    "gunner_move_attack_grenade2"
                } else {
                    "gunner_move_attack_grenade"
                },
                true,
            );
            let now = context.game.host.now();
            context.state_mut().attack_finished = now + context.game.random() * 2.0;
        } else {
            context.state_mut().manual_steering = false;
        }
        let now = context.game.host.now();
        context.game.require_entity_mut(&actor).timestamp = now + 2.0 + context.game.random();
        return;
    }
    let timestamp = context.game.require_entity(&actor).timestamp;
    if timestamp > context.game.host.now()
        || target_distance(context) <= 175.0 && clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 45))
    {
        context.set_move("gunner_move_attack_chain", true);
        return;
    }
    let timestamp = context.game.require_entity(&actor).timestamp;
    if timestamp <= context.game.host.now() && context.game.random() <= 0.5 && gunner_grenade_check(context) {
        let second = context.game.random() < 0.5;
        context.set_move(
            if second {
                "gunner_move_attack_grenade2"
            } else {
                "gunner_move_attack_grenade"
            },
            true,
        );
        let now = context.game.host.now();
        context.game.require_entity_mut(&actor).timestamp = now + 2.0 + context.game.random();
    } else if clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 45)) {
        context.set_move("gunner_move_attack_chain", true);
    }
}

/// Pain (`pain`).
fn rerelease_gunner_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    finish_dodge(context);
    let max_health = context.game.require_entity(&actor).max_health;
    let skin = context.game.require_entity(&actor).skin;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { skin | 1 } else { skin & !1 };
    if gunner_jumping(context) || context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let second = context.game.random() < 0.5;
    context.game.sound(
        &actor,
        if second {
            "gunner/gunpain2.wav"
        } else {
            "gunner/gunpain1.wav"
        },
        2,
        1.0,
        1.0,
    );
    if !reacts_to_pain(context) {
        return;
    }
    let damage = reaction.damage;
    context.set_move(
        if damage <= 10.0 {
            "gunner_move_pain3"
        } else if damage <= 25.0 {
            "gunner_move_pain2"
        } else {
            "gunner_move_pain1"
        },
        true,
    );
    context.state_mut().manual_steering = false;
    if context.state().ducked {
        set_duck(context, false);
    }
}

/// Die (`die`).
fn rerelease_gunner_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if check_gib(context) {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        context.game.require_entity_mut(&actor).skin /= 2;
        let damage = reaction.pain.damage;
        for _ in 0..2 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/bone/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/objects/gibs/sm_meat/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
        }
        for part in ["chest", "garm", "gun", "foot", "head"] {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                &format!("models/monsters/gunner/gibs/{part}.md2"),
                damage,
                Q2GibOptions {
                    skinned: true,
                    upright: part == "garm" || part == "gun",
                    head: part == "head",
                    ..Q2GibOptions::default()
                },
            );
        }
        context.state_mut().dead = true;
        context.state_mut().gibbed = true;
        return;
    }
    if context.state().dead {
        return;
    }
    context.game.sound(&actor, "gunner/death1.wav", 2, 1.0, 1.0);
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
    context.set_move("gunner_move_death", true);
}

/// Duck (`duck`).
fn rerelease_gunner_duck(context: &mut MonsterContext, _eta: f64) -> bool {
    if gunner_jumping(context) {
        return false;
    }
    if gunner_shooting(context) {
        set_duck(context, false);
        return false;
    }
    if context.game.random() > 0.5 {
        gunner_grenade(context);
    }
    context.set_move("gunner_move_duck", true);
    true
}

/// Sidestep (`sidestep`).
fn rerelease_gunner_sidestep(context: &mut MonsterContext) -> bool {
    if gunner_jumping(context) || gunner_shooting(context) || context.state().current_move.name == "gunner_move_pain1" {
        return false;
    }
    if context.state().current_move.name != "gunner_move_run" {
        context.set_move("gunner_move_run", true);
    }
    true
}

/// Blocked (`blocked`).
fn rerelease_gunner_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    if blocked_check_platform(context, distance) {
        return true;
    }
    let actor = context.actor().clone();
    let can_jump = context.game.require_entity(&actor).spawnflags & 8 == 0;
    let result = blocked_check_jump(context, distance, 192.0, 40.0, can_jump, JumpNavigation::None);
    if result == JumpResult::None {
        return false;
    }
    if result != JumpResult::Turn && enemy_body(context).is_some() {
        finish_dodge(context);
        context.set_move(
            if result == JumpResult::Up {
                "gunner_move_jump2"
            } else {
                "gunner_move_jump"
            },
            true,
        );
    }
    true
}

/// Fidget (`gunner_fidget`).
fn gunner_fidget(context: &mut MonsterContext) {
    if !context.state().stand_ground && context.entity().enemy.is_none() && context.game.random() <= 0.05 {
        context.set_move("gunner_move_fidget", true);
    }
}

/// Shrink (`gunner_shrink`).
fn gunner_shrink(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).server_flags |= 2;
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.max.z = -4.0;
    context.game.write_body(actor, &moved, true);
}

/// Run and shoot (`gunner_runandshoot`).
fn gunner_run_and_shoot(context: &mut MonsterContext) {
    context.set_move("gunner_move_runandshoot", true);
}

/// Blind check (`gunner_blind_check`).
fn gunner_blind_check(context: &mut MonsterContext) {
    if context.state().manual_steering {
        let actor = context.actor().clone();
        let target = context.state().blind_fire_target;
        let origin = context.game.body_of(actor).origin;
        context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(target, origin)).y);
    }
}

/// Fire (`GunnerFire`).
fn gunner_fire(context: &mut MonsterContext) {
    if enemy_body(context).is_none() {
        return;
    }
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let id = 45 + frame - gunner_frame::ATTAK216;
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, id as usize), None);
    let Some(direction) = predicted_direction(context, start, 0.0, true, -0.2) else {
        return;
    };
    let fire_bullet = context.weapons.fire_bullet;
    fire_bullet(actor, &mut *context.game, start, direction, 3.0, 4.0, 300.0, 500.0, 0);
    monster_flash(context, id, start, direction);
}

/// Refire chain (`gunner_refire_chain`).
fn gunner_refire_chain(context: &mut MonsterContext) {
    let enemy = context.entity().enemy.clone();
    let again =
        health(&mut *context.game, enemy.as_ref()) > 0.0 && visible(context, None) && context.game.random() <= 0.5;
    context.set_move(
        if again {
            "gunner_move_fire_chain"
        } else {
            "gunner_move_endfire_chain"
        },
        false,
    );
}

/// Jump wait land (`gunner_jump_wait_land`).
fn gunner_jump_wait_land(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let landed = context.game.body_of(actor).ground.is_some() || monster_jump_finished(context);
    context.state_mut().next_frame = frame + if landed { 1 } else { 0 };
}

/// Create the rerelease gunner definition (`rereleaseGunnerDefinition`).
pub fn rerelease_gunner_definition() -> Q2MonsterDefinition {
    let base = gunner_definition();
    let mut definition = Q2MonsterDefinition::new(
        "monster_gunner",
        "gunner",
        "models/monsters/gunner/tris.md2",
        175.0,
        -70.0,
        200.0,
        base.bounds.clone(),
        f64::from(1.15f32),
        "gunner_move_stand",
        gunner_moves(),
        base.stand.clone(),
        base.walk.clone(),
        MonsterHandler::Callback(rerelease_gunner_run),
        base.attack.clone(),
        rerelease_gunner_die,
    );
    definition.blind_fire = true;
    definition.attack = MonsterHandler::Callback(rerelease_gunner_attack);
    definition.sight = Some(MonsterHandler::Callback(gunner_sight));
    definition.search = Some(MonsterHandler::Callback(gunner_search));
    definition.pain = Some(rerelease_gunner_pain);
    definition.duck = Some(rerelease_gunner_duck);
    definition.sidestep = Some(rerelease_gunner_sidestep);
    definition.blocked = Some(rerelease_gunner_blocked);
    definition.callbacks = base.callbacks.clone();
    for (name, handler) in [
        ("gunner_run", MonsterHandler::Callback(rerelease_gunner_run)),
        ("gunner_dead", MonsterHandler::Callback(corpse)),
        ("GunnerGrenade", MonsterHandler::Callback(gunner_grenade)),
        ("gunner_fidget", MonsterHandler::Callback(gunner_fidget)),
        ("gunner_shrink", MonsterHandler::Callback(gunner_shrink)),
        ("gunner_runandshoot", MonsterHandler::Callback(gunner_run_and_shoot)),
        ("gunner_blind_check", MonsterHandler::Callback(gunner_blind_check)),
        ("GunnerFire", MonsterHandler::Callback(gunner_fire)),
        ("gunner_refire_chain", MonsterHandler::Callback(gunner_refire_chain)),
        ("gunner_jump_now", MonsterHandler::Callback(gunner_jump_now)),
        ("gunner_jump2_now", MonsterHandler::Callback(gunner_jump2_now)),
        ("gunner_jump_wait_land", MonsterHandler::Callback(gunner_jump_wait_land)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
