//! Infantry callbacks (`src/content/q2/foundation/monsters/infantry.ts`).
//!
//! Quake II `game/m_infantry.c` and `rerelease/m_infantry.cpp`
//! (id Software, GPL-2.0-or-later).

use std::collections::HashMap;

use qa_core::math::{Vec3, add3, dot3, normalize3, scale3, sub3, vec3};

use super::ai::{
    angles_vectors, clear_shot, corpse, enemy_body, finish_dodge, health, project_flash, set_duck,
    target_distance, walk_move,
};
use super::frames::{InfantryFrames, classic_infantry, rerelease_infantry};
use super::gibs::{Q2GibOptions, throw_gib, throw_head};
use super::muzzle::{self, muzzle_offset};
use super::types::{MonsterAttackState, MonsterContext, MonsterHandler, record_at};
use crate::q2::foundation::host::{Q2Edition, Q2TraceRequest};
use crate::q2::support::contracts::{AttackCause, DeathReaction, PainReaction, TraceHit};

/// Edition frame table (`frames`).
fn frames(context: &MonsterContext) -> InfantryFrames {
    InfantryFrames::new(context.game.options.edition == Q2Edition::Classic)
}

/// Whether the infantry can run (`canRun`).
fn can_run(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    let yaw = f64::from(context.game.body_of(actor).angles.y);
    walk_move(context, yaw, 8.0, false, true)
}

/// Whether the infantry is jumping (`jumping`).
fn jumping(context: &MonsterContext) -> bool {
    let name = context.state().current_move.name.clone();
    name == "infantry_move_jump" || name == "infantry_move_jump2"
}

/// Infantry stand (`infantryStand`).
pub fn infantry_stand(context: &mut MonsterContext) {
    context.set_move("infantry_move_stand", true);
}

/// Infantry walk (`infantryWalk`).
pub fn infantry_walk(context: &mut MonsterContext) {
    context.set_move("infantry_move_walk", true);
}

/// Infantry run (`infantryRun`).
pub fn infantry_run(context: &mut MonsterContext) {
    if context.game.options.edition == Q2Edition::Rerelease {
        finish_dodge(context);
    }
    if context.state().stand_ground {
        context.set_move("infantry_move_stand", true);
    } else {
        context.set_move("infantry_move_run", true);
    }
}

/// Infantry sight (`infantrySight`).
pub fn infantry_sight(context: &mut MonsterContext) {
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    let actor = context.actor().clone();
    let path = if !rerelease || context.game.random() < 0.5 {
        "infantry/infsght1.wav"
    } else {
        "infantry/infsrch1.wav"
    };
    context.game.sound(&actor, path, if rerelease { 2 } else { 4 }, 1.0, 1.0);
}

/// Infantry pain (`infantryPain`).
pub fn infantry_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    let actor = context.actor().clone();
    if health(context.game, Some(&actor)) < context.entity().max_health / 2.0 {
        context.entity_mut().skin = 1;
    } else if rerelease {
        context.entity_mut().skin = 0;
    }
    if rerelease && jumping(context) {
        return;
    }
    if rerelease {
        finish_dodge(context);
    }
    if context.game.host.now() < context.state().pain_time {
        if rerelease && context.game.random() < 0.33 {
            if let Some(attacker) = reaction.attacker.clone() {
                let frame = context.game.host.frame_seconds();
                context.dodge(attacker, frame, None, false);
            }
        }
        return;
    }
    let pain = context.game.host.now() + 3.0;
    context.state_mut().pain_time = pain;
    if !rerelease && context.game.options.skill == 3 {
        return;
    }
    let n = (context.game.random() * 2.0).floor() as i32;
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if n == 0 { "infantry/infpain1.wav" } else { "infantry/infpain2.wav" },
        2,
        1.0,
        1.0,
    );
    let cause = context.entity().last_attack.as_ref().map(|attack| attack.cause.clone());
    let excluded = context.game.options.skill == 3
        && !matches!(cause, Some(AttackCause::Q2 { means_of_death: 41, .. }));
    if rerelease && (context.state().ducked || context.state().combat_point || excluded) {
        if context.game.random() < 0.33 {
            if let Some(attacker) = reaction.attacker.clone() {
                let frame = context.game.host.frame_seconds();
                context.dodge(attacker, frame, None, false);
            }
        }
        return;
    }
    context.set_move(if n == 0 { "infantry_move_pain1" } else { "infantry_move_pain2" }, true);
    if rerelease {
        set_duck(context, false);
    }
}

/// Death aim offsets (`deathAim`).
const DEATH_AIM: [Vec3; 12] = [
    Vec3 { x: 0.0, y: 5.0, z: 0.0 },
    Vec3 { x: 10.0, y: 15.0, z: 0.0 },
    Vec3 { x: 20.0, y: 25.0, z: 0.0 },
    Vec3 { x: 25.0, y: 35.0, z: 0.0 },
    Vec3 { x: 30.0, y: 40.0, z: 0.0 },
    Vec3 { x: 30.0, y: 45.0, z: 0.0 },
    Vec3 { x: 25.0, y: 50.0, z: 0.0 },
    Vec3 { x: 20.0, y: 40.0, z: 0.0 },
    Vec3 { x: 15.0, y: 35.0, z: 0.0 },
    Vec3 { x: 40.0, y: 35.0, z: 0.0 },
    Vec3 { x: 70.0, y: 35.0, z: 0.0 },
    Vec3 { x: 90.0, y: 35.0, z: 0.0 },
];

/// Fire the machine gun (`machineGun`).
pub(crate) fn machine_gun(context: &mut MonsterContext) {
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    let table = frames(context);
    let enemy = enemy_body(context);
    if rerelease && enemy.is_none() {
        return;
    }
    let frame = context.entity().frame;
    let running = rerelease
        && frame >= rerelease_infantry::RUN201
        && frame <= rerelease_infantry::RUN208;
    let normal = if rerelease {
        frame == rerelease_infantry::ATTAK103
            || frame == rerelease_infantry::ATTAK311
            || frame == rerelease_infantry::ATTAK416
            || running
    } else {
        frame == classic_infantry::ATTAK111
    };
    // The rerelease expression MZ2_14 + (frame - MZ2_14) intentionally resolves to frame.
    let flash = if normal {
        if running {
            frame
        } else if rerelease && frame == rerelease_infantry::ATTAK416 {
            muzzle::INFANTRY_MACHINEGUN_22 as i32
        } else {
            26
        }
    } else {
        27 + frame - table.death211()
    };
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, flash as usize), None);
    let actor = context.actor().clone();
    let mut forward = angles_vectors(context.game.body_of(actor.clone()).angles).forward;
    if normal && enemy.is_some() {
        let enemy_actor = context.entity().enemy.clone();
        let observed = context.game.monster_target(enemy_actor.as_ref());
        let Some(observed) = observed else { return };
        let height = observed.view_height as f32;
        let enemy = enemy.expect("enemy body");
        if !rerelease {
            forward = normalize3(sub3(
                add3(
                    add3(enemy.origin, scale3(enemy.velocity, -0.2)),
                    vec3(0.0, 0.0, height),
                ),
                start,
            ));
        } else {
            let eye = add3(enemy.origin, vec3(0.0, 0.0, height));
            let trace = context.game.host.trace(&Q2TraceRequest {
                start,
                end: eye,
                bounds: None,
                ignore: Some(actor.clone()),
                mask: 0x46004003,
                exclude: Vec::new(),
            });
            let use_eye = matches!(&trace.hit, TraceHit::Actor { actor: hit } if Some(hit) == enemy_actor.as_ref());
            let target = if use_eye { eye } else { enemy.origin };
            let mut predicted = add3(enemy.origin, scale3(enemy.velocity, 0.2));
            let diverging = f64::from(dot3(
                normalize3(sub3(target, start)),
                normalize3(sub3(predicted, start)),
            )) < 0.0;
            let blocked = context.game.host.trace(&Q2TraceRequest {
                start,
                end: predicted,
                bounds: None,
                ignore: None,
                mask: 3,
                exclude: Vec::new(),
            })
            .fraction
                < 0.9;
            if diverging || blocked {
                predicted = enemy.origin;
            }
            if use_eye {
                predicted.z += height;
            }
            forward = normalize3(sub3(predicted, start));
        }
    } else if !normal {
        let actor = context.actor().clone();
        let angles = context.game.body_of(actor).angles;
        forward = angles_vectors(sub3(angles, *record_at(&DEATH_AIM, (flash - 27) as usize))).forward;
    }
    let actor = context.actor().clone();
    (context.weapons.fire_bullet)(actor.clone(), context.game, start, forward, 3.0, 4.0, 300.0, 500.0, 0);
    context.game.host_emit(crate::q2::foundation::host::Q2PresentationEvent::MonsterMuzzleflash {
        actor,
        flash,
        origin: start,
        direction: forward,
    });
}

/// Infantry attack (`infantryAttack`).
pub fn infantry_attack(context: &mut MonsterContext) {
    if context.game.options.edition == Q2Edition::Classic {
        if target_distance(context) < 80.0 {
            context.set_move("infantry_move_attack2", true);
        } else {
            context.set_move("infantry_move_attack1", true);
        }
        return;
    }
    finish_dodge(context);
    if target_distance(context) <= 20.0 && context.state().melee_time <= context.game.host.now() {
        context.set_move("infantry_move_attack2", true);
        return;
    }
    if clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 26)) {
        if context.state().cocked {
            context.set_move("infantry_move_attack1", true);
        } else {
            if context.game.random() <= 0.1 {
                context.set_move("infantry_move_attack5", true);
            } else {
                context.set_move("infantry_move_attack3", true);
            }
            context.state_mut().next_frame = rerelease_infantry::ATTAK405;
        }
    }
}

/// Infantry death (`infantryDie`).
pub fn infantry_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    let actor = context.actor().clone();
    let cause = context.entity().last_attack.as_ref().map(|attack| attack.cause.clone());
    if health(context.game, Some(&actor)) <= context.state().gib_health
        || rerelease
            && context.state().dead
            && matches!(cause, Some(AttackCause::Q2 { means_of_death: 20, .. }))
    {
        let actor = context.actor().clone();
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        if rerelease {
            let head = if context.state().current_move.name == "infantry_move_death3" {
                "models/monsters/infantry/gibs/head.md2"
            } else {
                "models/objects/gibs/sm_meat/tris.md2"
            };
            context.entity_mut().skin /= 2;
            let damage = reaction.pain.damage;
            let actor = context.actor().clone();
            throw_gib(actor.clone(), context.game, "models/objects/gibs/bone/tris.md2", damage, Q2GibOptions::default());
            for _ in 0..3 {
                throw_gib(
                    actor.clone(),
                    context.game,
                    "models/objects/gibs/sm_meat/tris.md2",
                    damage,
                    Q2GibOptions::default(),
                );
            }
            throw_gib(
                actor.clone(),
                context.game,
                "models/monsters/infantry/gibs/chest.md2",
                damage,
                Q2GibOptions { skinned: true, ..Q2GibOptions::default() },
            );
            throw_gib(
                actor.clone(),
                context.game,
                "models/monsters/infantry/gibs/gun.md2",
                damage,
                Q2GibOptions { skinned: true, upright: true, ..Q2GibOptions::default() },
            );
            for _ in 0..2 {
                throw_gib(
                    actor.clone(),
                    context.game,
                    "models/monsters/infantry/gibs/foot.md2",
                    damage,
                    Q2GibOptions { skinned: true, ..Q2GibOptions::default() },
                );
            }
            for _ in 0..2 {
                throw_gib(
                    actor.clone(),
                    context.game,
                    "models/monsters/infantry/gibs/arm.md2",
                    damage,
                    Q2GibOptions { skinned: true, ..Q2GibOptions::default() },
                );
            }
            throw_gib(
                actor,
                context.game,
                head,
                damage,
                Q2GibOptions { skinned: true, head: true, ..Q2GibOptions::default() },
            );
        } else {
            let damage = reaction.pain.damage;
            let actor = context.actor().clone();
            for _ in 0..2 {
                throw_gib(
                    actor.clone(),
                    context.game,
                    "models/objects/gibs/bone/tris.md2",
                    damage,
                    Q2GibOptions::default(),
                );
            }
            for _ in 0..4 {
                throw_gib(
                    actor.clone(),
                    context.game,
                    "models/objects/gibs/sm_meat/tris.md2",
                    damage,
                    Q2GibOptions::default(),
                );
            }
            throw_head(actor, context.game, "models/objects/gibs/head2/tris.md2", damage);
        }
        context.state_mut().dead = true;
        context.state_mut().gibbed = true;
        return;
    }
    if context.state().dead {
        return;
    }
    context.state_mut().dead = true;
    let actor = context.actor().clone();
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(&owned, &crate::q2::support::contracts::CombatTraitChanges {
        can_take_damage: Some(true),
        ..crate::q2::support::contracts::CombatTraitChanges::default()
    });
    let n = (context.game.random() * 3.0).floor() as usize;
    let movement =
        record_at(&["infantry_move_death1", "infantry_move_death2", "infantry_move_death3"], n);
    context.set_move(movement, true);
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if n == 1 { "infantry/infdeth1.wav" } else { "infantry/infdeth2.wav" },
        2,
        1.0,
        1.0,
    );
    if rerelease && n != 2 && context.game.random() <= 0.25 {
        let damage = reaction.pain.damage;
        let actor = context.actor().clone();
        let head = throw_gib(
            actor.clone(),
            context.game,
            "models/monsters/infantry/gibs/head.md2",
            damage,
            Q2GibOptions::default(),
        );
        if let Some(head) = head {
            let body = context.game.body_of(actor.clone());
            let inflictor = reaction.inflictor.clone();
            let inflictor_origin =
                inflictor.as_ref().and_then(|inflictor| context.game.host.bodies().read(inflictor));
            let direction = scale3(
                normalize3(sub3(body.origin, inflictor_origin.map(|body| body.origin).unwrap_or(body.origin))),
                100.0,
            );
            let spin = context.game.require_entity(&head).angular_velocity;
            context.game.require_entity_mut(&head).angular_velocity = scale3(spin, 0.15);
            let mut moved = context.game.body_of(head.clone());
            moved.origin = vec3(body.origin.x, body.origin.y, body.origin.z + 32.0);
            moved.angles = body.angles;
            moved.velocity = vec3(direction.x, direction.y, 200.0);
            context.game.write_body(head.clone(), &moved, true);
            context.game.set_motion_kind(head, crate::q2::foundation::host::Q2MotionKind::Toss);
        }
    }
}

/// Infantry duck (`infantryDuck`).
pub fn infantry_duck(context: &mut MonsterContext, _eta_seconds: f64) -> bool {
    if jumping(context) {
        return false;
    }
    if context.entity().frame == rerelease_infantry::ATTAK103
        || context.entity().frame == rerelease_infantry::ATTAK315
        || context.state().current_move.name == "infantry_move_attack2"
    {
        set_duck(context, false);
        return false;
    }
    context.set_move("infantry_move_duck", true);
    true
}

/// Infantry sidestep (`infantrySidestep`).
pub fn infantry_sidestep(context: &mut MonsterContext) -> bool {
    if jumping(context) {
        return false;
    }
    if context.state().current_move.name == "infantry_move_run" {
        return true;
    }
    let frame = context.entity().frame;
    if context.state().current_move.name != "infantry_move_attack4"
        && context.state().next_move.as_ref().is_none_or(|next| next.name != "infantry_move_attack4")
        && !context.state().cocked
        && (frame == rerelease_infantry::ATTAK103
            || frame == rerelease_infantry::ATTAK311
            || frame == rerelease_infantry::ATTAK416)
    {
        let wait = context.state().fire_wait + 0.3 + context.game.random() * 0.3;
        context.state_mut().fire_wait = wait;
        context.set_move("infantry_move_attack4", false);
    }
    true
}

/// Infantry fire (`fire`).
fn infantry_fire(context: &mut MonsterContext) {
    machine_gun(context);
    if context.game.options.edition == Q2Edition::Classic {
        let hold = context.game.host.now() < context.state().pause_time;
        context.state_mut().hold_frame = hold;
        return;
    }
    context.state_mut().cocked = false;
    if context.state().current_move.name == "infantry_move_attack4" {
        if context.game.host.now() >= context.state().fire_wait {
            finish_dodge(context);
            context.set_move("infantry_move_attack1", false);
            context.state_mut().next_frame = rerelease_infantry::ATTAK114;
        } else if !can_run(context) {
            context.set_move("infantry_move_attack1", false);
            context.state_mut().next_frame = rerelease_infantry::ATTAK103;
            finish_dodge(context);
            context.state_mut().attack_state = MonsterAttackState::Straight;
        }
    } else {
        let frame = context.entity().frame;
        if frame >= rerelease_infantry::ATTAK101 && frame <= rerelease_infantry::ATTAK115
            || frame >= rerelease_infantry::ATTAK301 && frame <= rerelease_infantry::ATTAK315
            || frame >= rerelease_infantry::ATTAK401 && frame <= rerelease_infantry::ATTAK424
        {
            let hold = context.game.host.now() < context.state().fire_wait;
            context.state_mut().hold_frame = hold;
            if !context.state().hold_frame && frame == rerelease_infantry::ATTAK416 {
                context.state_mut().next_frame = rerelease_infantry::ATTAK420;
            }
        }
    }
}

/// Infantry jump (`jump`).
fn infantry_jump(context: &mut MonsterContext, high: bool) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let basis = angles_vectors(body.angles);
    let mut moved = body.clone();
    moved.velocity = add3(
        body.velocity,
        add3(
            scale3(basis.forward, if high { 150.0 } else { 100.0 }),
            scale3(basis.up, if high { 400.0 } else { 300.0 }),
        ),
    );
    moved.ground = None;
    context.game.write_body(actor.clone(), &moved, true);
    context.game.set_motion_kind(actor, crate::q2::foundation::host::Q2MotionKind::Step);
}

/// Cock the gun (`infantry_cock_gun`).
fn cock_gun(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "infantry/infatck3.wav", 1, 1.0, 1.0);
    if context.game.options.edition == Q2Edition::Classic {
        let pause = context.game.host.now()
            + ((context.game.random() * 16.0).floor() + 10.0) * 0.1;
        context.state_mut().pause_time = pause;
    } else {
        context.state_mut().cocked = true;
    }
}

/// Set the fire time (`infantry_set_firetime`).
fn set_firetime(context: &mut MonsterContext) {
    let wait = context.game.host.now() + 0.7 + context.game.random() * 1.3;
    context.state_mut().fire_wait = wait;
    if !context.state().stand_ground
        && context.entity().enemy.is_some()
        && target_distance(context) >= 330.0
        && can_run(context)
    {
        context.set_move("infantry_move_attack4", false);
    }
}

/// Swing (`infantry_swing`).
fn swing(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "infantry/infatck2.wav", 1, 1.0, 1.0);
}

/// Smack (`infantry_smack`).
fn smack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let damage = 5.0 + (context.game.random() * 5.0).floor();
    if (context.weapons.fire_hit)(actor.clone(), context.game, vec3(80.0, 0.0, 0.0), damage, 50.0) {
        context.game.sound(&actor, "infantry/melee2.wav", 1, 1.0, 1.0);
    } else if context.game.options.edition == Q2Edition::Rerelease {
        let melee = context.game.host.now() + 1.5;
        context.state_mut().melee_time = melee;
    }
}

/// Duck down (`infantry_duck_down`).
fn duck_down(context: &mut MonsterContext) {
    if context.state().ducked {
        return;
    }
    set_duck(context, true);
    let pause = context.game.host.now() + 1.0;
    context.state_mut().pause_time = pause;
}

/// Duck hold (`infantry_duck_hold`).
fn duck_hold(context: &mut MonsterContext) {
    let hold = context.game.host.now() < context.state().pause_time;
    context.state_mut().hold_frame = hold;
}

/// Duck up (`infantry_duck_up`).
fn duck_up(context: &mut MonsterContext) {
    set_duck(context, false);
}

/// Shrink (`infantry_shrink`).
fn shrink(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    context.entity_mut().server_flags |= 2;
    let mut moved = body;
    moved.bounds.max.z = 0.0;
    context.game.write_body(actor, &moved, true);
}

/// Attack4 refire (`infantry_attack4_refire`).
fn attack4_refire(context: &mut MonsterContext) {
    if context.game.host.now() >= context.state().fire_wait {
        finish_dodge(context);
        context.set_move("infantry_move_attack1", false);
        context.state_mut().next_frame = rerelease_infantry::ATTAK114;
    } else if context.state().stand_ground
        || context.entity().enemy.is_some()
            && (target_distance(context) < 330.0 || !can_run(context))
    {
        context.set_move("infantry_move_attack1", false);
        context.state_mut().next_frame = rerelease_infantry::ATTAK103;
        finish_dodge(context);
        context.state_mut().attack_state = MonsterAttackState::Straight;
    } else {
        context.state_mut().next_frame = rerelease_infantry::RUN201;
    }
    infantry_fire(context);
}

/// Jump now (`infantry_jump_now`).
fn jump_now(context: &mut MonsterContext) {
    infantry_jump(context, false);
}

/// High jump now (`infantry_jump2_now`).
fn jump2_now(context: &mut MonsterContext) {
    infantry_jump(context, true);
}

/// Wait for landing (`infantry_jump_wait_land`).
fn jump_wait_land(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    if body.ground.is_some() {
        let next = context.entity().frame + 1;
        context.state_mut().next_frame = next;
        return;
    }
    let next = context.entity().frame;
    context.state_mut().next_frame = next;
    let forward = angles_vectors(body.angles).forward;
    let speed = (f64::from(body.velocity.x * forward.x))
        .hypot(f64::from(body.velocity.y * forward.y))
        .hypot(f64::from(body.velocity.z * forward.z));
    if speed < 150.0 {
        let mut moved = context.game.body_of(actor.clone());
        let flat = scale3(forward, 150.0);
        moved.velocity = vec3(flat.x, flat.y, body.velocity.z);
        context.game.write_body(actor.clone(), &moved, true);
        context.game.set_motion_kind(actor, crate::q2::foundation::host::Q2MotionKind::Step);
    }
    if context.state().jump_time < context.game.host.now() {
        let next = context.entity().frame + 1;
        context.state_mut().next_frame = next;
    }
}

/// Named infantry callbacks (`infantryCallbacks`).
pub fn infantry_callbacks() -> HashMap<String, MonsterHandler> {
    HashMap::from([
        ("infantry_stand".to_string(), MonsterHandler::Callback(infantry_stand)),
        ("infantry_run".to_string(), MonsterHandler::Callback(infantry_run)),
        ("InfantryMachineGun".to_string(), MonsterHandler::Callback(machine_gun)),
        ("infantry_fire".to_string(), MonsterHandler::Callback(infantry_fire)),
        ("infantry_dead".to_string(), MonsterHandler::Callback(corpse)),
        ("infantry_cock_gun".to_string(), MonsterHandler::Callback(cock_gun)),
        ("infantry_set_firetime".to_string(), MonsterHandler::Callback(set_firetime)),
        ("infantry_swing".to_string(), MonsterHandler::Callback(swing)),
        ("infantry_smack".to_string(), MonsterHandler::Callback(smack)),
        ("infantry_duck_down".to_string(), MonsterHandler::Callback(duck_down)),
        ("infantry_duck_hold".to_string(), MonsterHandler::Callback(duck_hold)),
        ("infantry_duck_up".to_string(), MonsterHandler::Callback(duck_up)),
        ("infantry_shrink".to_string(), MonsterHandler::Callback(shrink)),
        ("infantry_attack4_refire".to_string(), MonsterHandler::Callback(attack4_refire)),
        ("infantry_jump_now".to_string(), MonsterHandler::Callback(jump_now)),
        ("infantry_jump2_now".to_string(), MonsterHandler::Callback(jump2_now)),
        ("infantry_jump_wait_land".to_string(), MonsterHandler::Callback(jump_wait_land)),
    ])
}
