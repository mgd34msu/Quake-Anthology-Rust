//! Soldier callbacks (`src/content/q2/foundation/monsters/soldier.ts`).
//!
//! id Software `game/m_soldier.c` and `rerelease/m_soldier.cpp`
//! (GPL-2.0-or-later).

use std::collections::HashMap;

use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, vec3};

use super::ai::{
    angles_vectors, clear_shot, corpse, enemy_body, enemy_eye, finish_dodge, health, prone_shot,
    project_flash, set_duck, target_distance, vector_angles, visible,
};
use super::frames::{SoldierFrames, rerelease_soldier};
use super::gibs::{Q2GibOptions, throw_gib, throw_head};
use super::muzzle::{self, muzzle_offset};
use super::types::{MonsterContext, MonsterHandler, MonsterWeapon, record_at};
use crate::q2::foundation::host::Q2Edition;
use crate::q2::support::contracts::{AttackCause, DeathReaction, PainReaction};

/// Edition frame table (`frames`).
fn frames(context: &MonsterContext) -> SoldierFrames {
    SoldierFrames::new(context.game.options.edition == Q2Edition::Classic)
}

/// Whether the enemy is alive (`aliveEnemy`).
fn alive_enemy(context: &mut MonsterContext) -> bool {
    let enemy = context.entity().enemy.clone();
    enemy_body(context).is_some() && health(context.game, enemy.as_ref()) > 0.0
}

/// Whether the shotgun needs cocking (`needsCock`).
fn needs_cock(context: &MonsterContext) -> bool {
    context.state().weapon == MonsterWeapon::Shotgun && !context.state().cocked
}

/// Whether the enemy is in melee range (`meleeRange`).
fn melee_range(context: &mut MonsterContext) -> bool {
    target_distance(context)
        <= if context.game.options.edition == Q2Edition::Classic { 80.0 } else { 20.0 }
}

/// Whether to refire (`refire`).
fn refire(context: &mut MonsterContext, force: bool) -> bool {
    if context.game.options.edition == Q2Edition::Classic {
        context.game.options.skill == 3 && context.game.random() < 0.5 || melee_range(context)
    } else {
        (force || context.game.random() < 0.5) && visible(context, None) || melee_range(context)
    }
}

/// Soldier stand (`soldierStand`).
pub fn soldier_stand(context: &mut MonsterContext) {
    if context.game.options.edition == Q2Edition::Rerelease {
        if (context.entity().spawnflags & 8) != 0 {
            context.set_move("soldier_move_blind", true);
            return;
        }
        let r = context.game.random();
        let current = context.state().current_move.name.clone();
        context.set_move(
            if current != "soldier_move_stand1" || r < 0.6 {
                "soldier_move_stand1"
            } else if r < 0.8 {
                "soldier_move_stand2"
            } else {
                "soldier_move_stand3"
            },
            true,
        );
        return;
    }
    let current = context.state().current_move.name.clone();
    if current == "soldier_move_stand3" || context.game.random() < 0.8 {
        context.set_move("soldier_move_stand1", true);
    } else {
        context.set_move("soldier_move_stand3", true);
    }
}

/// Soldier walk (`soldierWalk`).
pub fn soldier_walk(context: &mut MonsterContext) {
    if context.game.random() < 0.5 {
        context.set_move("soldier_move_walk1", true);
    } else {
        context.set_move("soldier_move_walk2", true);
    }
}

/// Soldier run (`soldierRun`).
pub fn soldier_run(context: &mut MonsterContext) {
    if context.game.options.edition == Q2Edition::Rerelease {
        finish_dodge(context);
    }
    if context.state().stand_ground {
        context.set_move("soldier_move_stand1", true);
        return;
    }
    let movement = context.state().current_move.name.clone();
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    if movement == "soldier_move_walk1"
        || movement == "soldier_move_walk2"
        || movement == "soldier_move_start_run"
        || rerelease && movement == "soldier_move_run"
    {
        context.set_move("soldier_move_run", true);
    } else {
        context.set_move("soldier_move_start_run", true);
    }
}

/// Soldier attack (`soldierAttack`).
pub fn soldier_attack(context: &mut MonsterContext) {
    if context.game.options.edition == Q2Edition::Classic {
        if context.state().weapon == MonsterWeapon::Machinegun {
            context.set_move("soldier_move_attack4", true);
        } else if context.game.random() < 0.5 {
            context.set_move("soldier_move_attack1", true);
        } else {
            context.set_move("soldier_move_attack2", true);
        }
        return;
    }
    finish_dodge(context);
    if context.state().attack_state == super::types::MonsterAttackState::Blind {
        let chance = if context.state().blind_fire_delay < 1.0 {
            1.0
        } else if context.state().blind_fire_delay < 7.5 {
            0.4
        } else {
            0.1
        };
        let r = context.game.random();
        let delay = context.state().blind_fire_delay + 4.1 + context.game.random() * 3.0;
        context.state_mut().blind_fire_delay = delay;
        if f64::from(length3(context.state().blind_fire_target)) == 0.0 || r > chance {
            return;
        }
        context.state_mut().manual_steering = true;
        context.set_move("soldier_move_attack1", true);
        let finished = context.game.host.now() + 1.5 + context.game.random();
        context.state_mut().attack_finished = finished;
        return;
    }
    let r = context.game.random();
    if !context.state().stand_ground
        && r < 0.25
        && context.state().weapon != MonsterWeapon::Machinegun
        && target_distance(context) >= 220.0
    {
        context.set_move("soldier_move_attack6", true);
        return;
    }
    if context.state().weapon != MonsterWeapon::Machinegun {
        let close = context.state().weapon == MonsterWeapon::Shotgun
            && target_distance(context) <= 286.0;
        let first = !close
            && clear_shot(context, muzzle_offset(Q2Edition::Rerelease, muzzle::SOLDIER_BLASTER_1));
        let second =
            clear_shot(context, muzzle_offset(Q2Edition::Rerelease, muzzle::SOLDIER_BLASTER_2));
        if first && (!second || context.game.random() < 0.5) {
            context.set_move("soldier_move_attack1", true);
        } else if second {
            context.set_move("soldier_move_attack2", true);
        }
    } else if clear_shot(context, muzzle_offset(Q2Edition::Rerelease, muzzle::SOLDIER_MACHINEGUN_4))
    {
        context.set_move("soldier_move_attack4", true);
    }
}

/// Soldier sight (`soldierSight`).
pub fn soldier_sight(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let path =
        if context.game.random() < 0.5 { "soldier/solsght1.wav" } else { "soldier/solsrch1.wav" };
    context.game.sound(&actor, path, 2, 1.0, 1.0);
    if context.game.options.edition == Q2Edition::Classic {
        if context.game.options.skill > 0
            && target_distance(context) >= 500.0
            && context.game.random() > 0.5
        {
            context.set_move("soldier_move_attack6", true);
        }
    } else if visible(context, None)
        && target_distance(context) >= 440.0
        && context.game.random() > 0.75
    {
        if context.state().weapon != MonsterWeapon::Machinegun {
            context.set_move("soldier_move_attack6", true);
        } else if clear_shot(
            context,
            muzzle_offset(Q2Edition::Rerelease, muzzle::SOLDIER_MACHINEGUN_4),
        ) {
            context.set_move("soldier_move_attack4", true);
        }
    }
}

/// Soldier pain (`soldierPain`).
pub fn soldier_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    let actor = context.actor().clone();
    if health(context.game, Some(&actor)) < context.entity().max_health / 2.0 {
        context.entity_mut().skin |= 1;
    } else if rerelease {
        context.entity_mut().skin &= !1;
    }
    if rerelease {
        finish_dodge(context);
        context.state_mut().charging = false;
        context.state_mut().manual_steering = false;
    }
    let actor = context.actor().clone();
    let velocity = context.game.body_of(actor.clone()).velocity;
    if context.game.host.now() < context.state().pain_time {
        if velocity.z > 100.0
            && ["soldier_move_pain1", "soldier_move_pain2", "soldier_move_pain3"]
                .contains(&context.state().current_move.name.as_str())
        {
            if rerelease {
                set_duck(context, false);
            }
            context.set_move("soldier_move_pain4", true);
        }
        return;
    }
    let pain = context.game.host.now() + 3.0;
    context.state_mut().pain_time = pain;
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        match context.state().weapon {
            MonsterWeapon::Blaster => "soldier/solpain2.wav",
            MonsterWeapon::Shotgun => "soldier/solpain1.wav",
            MonsterWeapon::Machinegun => "soldier/solpain3.wav",
        },
        2,
        1.0,
        1.0,
    );
    if velocity.z > 100.0 {
        if rerelease {
            set_duck(context, false);
        }
        context.set_move("soldier_move_pain4", true);
        return;
    }
    let cause = context.entity().last_attack.as_ref().map(|attack| attack.cause.clone());
    if rerelease && (context.state().ducked || context.state().combat_point)
        || context.game.options.skill == 3
            && !(rerelease && matches!(cause, Some(AttackCause::Q2 { means_of_death: 41, .. })))
    {
        return;
    }
    let r = context.game.random();
    context.set_move(
        if r < 0.33 {
            "soldier_move_pain1"
        } else if r < 0.66 {
            "soldier_move_pain2"
        } else {
            "soldier_move_pain3"
        },
        true,
    );
    if rerelease {
        set_duck(context, false);
    }
}

/// Blaster flash numbers (`blasterFlash`).
const BLASTER_FLASH: [usize; 9] = [39, 40, 83, 86, 89, 92, 95, 98, muzzle::SOLDIER_BLASTER_9];
/// Shotgun flash numbers (`shotgunFlash`).
const SHOTGUN_FLASH: [usize; 9] = [41, 42, 84, 87, 90, 93, 96, 99, muzzle::SOLDIER_SHOTGUN_9];
/// Machine gun flash numbers (`machinegunFlash`).
const MACHINEGUN_FLASH: [usize; 9] =
    [43, 44, 85, 88, 91, 94, 97, 100, muzzle::SOLDIER_MACHINEGUN_9];

/// Fire the soldier weapon (`fire`).
fn fire(context: &mut MonsterContext, flash_number: usize, angle_limited: bool) {
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    let table = match context.state().weapon {
        MonsterWeapon::Blaster => &BLASTER_FLASH,
        MonsterWeapon::Shotgun => &SHOTGUN_FLASH,
        MonsterWeapon::Machinegun => &MACHINEGUN_FLASH,
    };
    let flash = *record_at(table, flash_number);
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, flash), None);
    let actor = context.actor().clone();
    let mut aim = angles_vectors(context.game.body_of(actor.clone()).angles).forward;
    if flash_number == 5 || flash_number == 6 {
        if rerelease && (context.entity().spawnflags & 65536) != 0 {
            return;
        }
    } else {
        let Some(enemy) = enemy_eye(context) else {
            context.state_mut().hold_frame = false;
            return;
        };
        let end = if context.state().attack_state == super::types::MonsterAttackState::Blind {
            let enemy_actor = context.entity().enemy.clone();
            let height = enemy_actor
                .as_ref()
                .and_then(|enemy| context.game.entity(enemy))
                .map(|target| target.view_height)
                .unwrap_or(22);
            let target = context.state().blind_fire_target;
            vec3(target.x, target.y, target.z + height as f32)
        } else {
            enemy
        };
        let direction = sub3(end, start);
        if rerelease && angle_limited && f64::from(dot3(normalize3(direction), aim)) < 0.5 {
            let hold = context.game.host.now() < context.state().fire_wait;
            context.state_mut().hold_frame = hold;
            return;
        }
        let basis = angles_vectors(vector_angles(direction));
        let r = (context.game.random() * 2.0 - 1.0) * 1000.0;
        let u = (context.game.random() * 2.0 - 1.0) * 500.0;
        aim = normalize3(add3(
            scale3(basis.forward, 8192.0),
            add3(scale3(basis.right, r as f32), scale3(basis.up, u as f32)),
        ));
    }
    let actor = context.actor().clone();
    match context.state().weapon {
        MonsterWeapon::Blaster => {
            (context.weapons.fire_blaster)(
                actor.clone(),
                context.game,
                start,
                aim,
                5.0,
                600.0,
                8,
                false,
                crate::q2::foundation::weapons::types::Mod::BLASTER,
            );
        }
        MonsterWeapon::Shotgun => {
            (context.weapons.fire_shotgun)(
                actor.clone(),
                context.game,
                start,
                aim,
                2.0,
                1.0,
                if rerelease { 1500.0 } else { 1000.0 },
                if rerelease { 750.0 } else { 500.0 },
                if rerelease { 9 } else { 12 },
                0,
            );
            if rerelease {
                context.state_mut().cocked = false;
            }
        }
        MonsterWeapon::Machinegun => {
            if !context.state().hold_frame {
                if rerelease {
                    let wait = context.game.host.now() + 0.3 + context.game.random() * 0.8;
                    context.state_mut().fire_wait = wait;
                } else {
                    let pause = context.game.host.now()
                        + (3.0 + (context.game.random() * 8.0).floor()) * 0.1;
                    context.state_mut().pause_time = pause;
                }
            }
            (context.weapons.fire_bullet)(
                actor.clone(),
                context.game,
                start,
                aim,
                2.0,
                4.0,
                300.0,
                500.0,
                0,
            );
            let hold = context.game.host.now()
                < if rerelease {
                    context.state().fire_wait
                } else {
                    context.state().pause_time
                };
            context.state_mut().hold_frame = hold;
        }
    }
    context.game.host_emit(crate::q2::foundation::host::Q2PresentationEvent::MonsterMuzzleflash {
        actor,
        flash: flash as i32,
        origin: start,
        direction: aim,
    });
}

/// Soldier death (`soldierDie`).
pub fn soldier_die(context: &mut MonsterContext, reaction: &DeathReaction) {
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
            context.entity_mut().skin /= 2;
            let damage = reaction.pain.damage;
            let actor = context.actor().clone();
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
                "models/objects/gibs/bone2/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
            throw_gib(
                actor.clone(),
                context.game,
                "models/objects/gibs/bone/tris.md2",
                damage,
                Q2GibOptions::default(),
            );
            throw_gib(
                actor.clone(),
                context.game,
                "models/monsters/soldier/gibs/arm.md2",
                damage,
                Q2GibOptions { skinned: true, ..Q2GibOptions::default() },
            );
            throw_gib(
                actor.clone(),
                context.game,
                "models/monsters/soldier/gibs/gun.md2",
                damage,
                Q2GibOptions { skinned: true, upright: true, ..Q2GibOptions::default() },
            );
            throw_gib(
                actor.clone(),
                context.game,
                "models/monsters/soldier/gibs/chest.md2",
                damage,
                Q2GibOptions { skinned: true, ..Q2GibOptions::default() },
            );
            throw_gib(
                actor,
                context.game,
                "models/monsters/soldier/gibs/head.md2",
                damage,
                Q2GibOptions { head: true, skinned: true, ..Q2GibOptions::default() },
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
    let owned = context.game.owned_of(actor.clone());
    context.game.set_combat_traits(&owned, &crate::q2::support::contracts::CombatTraitChanges {
        can_take_damage: Some(true),
        ..crate::q2::support::contracts::CombatTraitChanges::default()
    });
    context.game.sound(
        &actor,
        match context.state().weapon {
            MonsterWeapon::Blaster => "soldier/soldeth2.wav",
            MonsterWeapon::Shotgun => "soldier/soldeth1.wav",
            MonsterWeapon::Machinegun => "soldier/soldeth3.wav",
        },
        2,
        1.0,
        1.0,
    );
    let body = context.game.body_of(actor);
    let view_height = context.entity().view_height;
    if (f64::from(body.origin.z) + f64::from(view_height) - f64::from(reaction.point.z)).abs() <= 4.0
        && (!rerelease || body.velocity.z < 65.0)
    {
        context.set_move("soldier_move_death3", true);
        return;
    }
    if rerelease
        && (context.state().current_move.name == "soldier_move_trip"
            || context.state().current_move.name == "soldier_move_attack5")
    {
        context.set_move("soldier_move_death4", true);
        context.state_mut().next_frame = rerelease_soldier::DEATH413;
        death_shrink(context);
        return;
    }
    let fast =
        !(rerelease && body.velocity.z <= 65.0 && f64::from(length3(body.velocity)) <= 150.0);
    let n = (context.game.random() * if fast { 5.0 } else { 4.0 }).floor() as usize;
    let movement = record_at(
        &[
            "soldier_move_death1",
            "soldier_move_death2",
            "soldier_move_death4",
            "soldier_move_death5",
            "soldier_move_death6",
        ],
        n,
    );
    context.set_move(movement, true);
}

/// Soldier sidestep (`soldierSidestep`).
pub fn soldier_sidestep(context: &mut MonsterContext) -> bool {
    let name = context.state().current_move.name.clone();
    if name == "soldier_move_trip" || name == "soldier_move_attack5" || name == "soldier_move_pain4"
    {
        return false;
    }
    if context.state().weapon != MonsterWeapon::Machinegun {
        if name != "soldier_move_attack6" {
            context.set_move("soldier_move_attack6", true);
        }
    } else if name != "soldier_move_start_run" && name != "soldier_move_run" {
        context.set_move("soldier_move_start_run", true);
    }
    true
}

/// Soldier duck (`soldierDuck`).
pub fn soldier_duck(context: &mut MonsterContext, _eta_seconds: f64) -> bool {
    context.state_mut().hold_frame = false;
    let attack6 = context.state().current_move.name == "soldier_move_attack6";
    let duck = needs_cock(context) || context.game.random() < 0.5;
    context.set_move(
        if attack6 {
            "soldier_move_trip"
        } else if duck {
            "soldier_move_duck"
        } else {
            "soldier_move_attack3"
        },
        true,
    );
    true
}

/// Expansion hyper ripper (`hyperRipper`).
///
/// Expansion-only callbacks retain their source skin guards in shared
/// base moves.
fn hyper_ripper(context: &mut MonsterContext, flash: usize, limited: bool) {
    if context.entity().skin >= 6 && context.state().weapon != MonsterWeapon::Machinegun {
        fire(context, flash, limited);
    }
}

/// Expansion laser sound (`laserSound`).
fn laser_sound(context: &mut MonsterContext, start: bool) {
    if context.entity().skin >= 6 && context.state().weapon == MonsterWeapon::Machinegun {
        let actor = context.actor().clone();
        let origin = context.game.body_of(actor.clone()).origin;
        context.game.host_emit(crate::q2::foundation::host::Q2PresentationEvent::Sound(
            crate::q2::foundation::host::Q2SoundEvent {
                actor: Some(actor),
                origin,
                path: "weapons/laser2.wav".to_string(),
                channel: 1,
                volume: 1.0,
                attenuation: 1.0,
                reliable: false,
                loop_: if start {
                    crate::q2::foundation::host::Q2SoundLoop::Start
                } else {
                    crate::q2::foundation::host::Q2SoundLoop::Stop
                },
                loop_owner: None,
            },
        ));
    }
}

/// Soldier idle (`soldier_idle`).
fn idle(context: &mut MonsterContext) {
    if context.game.random() > 0.8 {
        let actor = context.actor().clone();
        context.game.sound(&actor, "soldier/solidle1.wav", 2, 1.0, 2.0);
    }
}

/// Soldier cock (`soldier_cock`).
fn cock(context: &mut MonsterContext) {
    let table = frames(context);
    let attenuated = context.entity().frame == table.stand322();
    let actor = context.actor().clone();
    context.game.sound(&actor, "infantry/infatck3.wav", 1, 1.0, if attenuated { 2.0 } else { 1.0 });
    context.state_mut().cocked = true;
}

/// Walk1 random (`soldier_walk1_random`).
fn walk1_random(context: &mut MonsterContext) {
    if context.game.random() > 0.1 {
        let next = frames(context).walk101();
        context.state_mut().next_frame = next;
    }
}

/// Fire callbacks.
fn fire1(context: &mut MonsterContext) {
    fire(context, 0, false);
}
/// Fire 2.
fn fire2(context: &mut MonsterContext) {
    fire(context, 1, false);
}
/// Fire 3.
fn fire3(context: &mut MonsterContext) {
    if context.game.options.edition == Q2Edition::Classic {
        soldier_duck_down(context);
    }
    fire(context, 2, false);
}
/// Fire 4.
fn fire4(context: &mut MonsterContext) {
    fire(context, 3, false);
}
/// Fire 5.
fn fire5(context: &mut MonsterContext) {
    fire(context, 8, true);
}
/// Fire 6.
fn fire6(context: &mut MonsterContext) {
    fire(context, 5, false);
    if context.game.options.edition == Q2Edition::Rerelease && needs_cock(context) {
        let next = frames(context).death126();
        context.state_mut().next_frame = next;
    }
}
/// Fire 7.
fn fire7(context: &mut MonsterContext) {
    fire(context, 6, false);
}
/// Fire 8.
fn fire8(context: &mut MonsterContext) {
    fire(context, 7, true);
}

/// Attack1 refire 1 (`soldier_attack1_refire1`).
fn attack1_refire1(context: &mut MonsterContext) {
    if context.game.options.edition == Q2Edition::Rerelease
        && context.state().weapon == MonsterWeapon::Blaster
    {
        let next = frames(context).attak110();
        context.state_mut().next_frame = next;
    }
    if context.state().manual_steering {
        context.state_mut().manual_steering = false;
        return;
    }
    if context.state().weapon != MonsterWeapon::Blaster || !alive_enemy(context) {
        return;
    }
    let table = frames(context);
    let next = if refire(context, false) { table.attak102() } else { table.attak110() };
    context.state_mut().next_frame = next;
}

/// Attack1 refire 2 (`soldier_attack1_refire2`).
fn attack1_refire2(context: &mut MonsterContext) {
    if context.state().weapon != MonsterWeapon::Blaster
        && alive_enemy(context)
        && refire(context, context.state().force_refire)
    {
        let next = frames(context).attak102();
        context.state_mut().next_frame = next;
        context.state_mut().force_refire = false;
    }
}

/// Attack1 shotgun check (`soldier_attack1_shotgun_check`).
fn attack1_shotgun_check(context: &mut MonsterContext) {
    if needs_cock(context) {
        let next = frames(context).attak106();
        context.state_mut().next_frame = next;
        context.state_mut().force_refire = true;
    }
}

/// Attack2 refire 1 (`soldier_attack2_refire1`).
fn attack2_refire1(context: &mut MonsterContext) {
    if context.game.options.edition == Q2Edition::Rerelease
        && context.state().weapon == MonsterWeapon::Blaster
    {
        let next = frames(context).attak216();
        context.state_mut().next_frame = next;
    }
    if context.state().weapon != MonsterWeapon::Blaster || !alive_enemy(context) {
        return;
    }
    if refire(context, false) {
        let next = frames(context).attak204();
        context.state_mut().next_frame = next;
    } else if context.game.options.edition == Q2Edition::Classic {
        let next = frames(context).attak216();
        context.state_mut().next_frame = next;
    }
}

/// Attack2 refire 2 (`soldier_attack2_refire2`).
fn attack2_refire2(context: &mut MonsterContext) {
    if context.state().weapon != MonsterWeapon::Blaster
        && alive_enemy(context)
        && refire(context, context.state().force_refire)
    {
        let next = frames(context).attak204();
        context.state_mut().next_frame = next;
        context.state_mut().force_refire = false;
    }
}

/// Attack2 shotgun check (`soldier_attack2_shotgun_check`).
fn attack2_shotgun_check(context: &mut MonsterContext) {
    if needs_cock(context) {
        let next = frames(context).attak210();
        context.state_mut().next_frame = next;
        context.state_mut().force_refire = true;
    }
}

/// Attack3 refire (`soldier_attack3_refire`).
fn attack3_refire(context: &mut MonsterContext) {
    if context.game.options.edition == Q2Edition::Rerelease && needs_cock(context) {
        let hold = context.game.host.now() < context.state().duck_wait;
        context.state_mut().hold_frame = hold;
    } else if context.game.host.now() + 0.4
        < if context.game.options.edition == Q2Edition::Classic {
            context.state().pause_time
        } else {
            context.state().duck_wait
        }
    {
        let next = frames(context).attak303();
        context.state_mut().next_frame = next;
    }
}

/// Attack6 refire (`soldier_attack6_refire`).
fn attack6_refire(context: &mut MonsterContext) {
    if alive_enemy(context)
        && target_distance(context) >= 500.0
        && context.game.options.skill == 3
    {
        let next = frames(context).runs03();
        context.state_mut().next_frame = next;
    }
}

/// Attack6 refire 1 (`soldier_attack6_refire1`).
fn attack6_refire1(context: &mut MonsterContext) {
    finish_dodge(context);
    context.state_mut().charging = false;
    if context.entity().enemy.is_none() || context.state().weapon != MonsterWeapon::Blaster {
        return;
    }
    if !alive_enemy(context) || target_distance(context) < 440.0 || !visible(context, None) {
        soldier_run(context);
        return;
    }
    if context.game.random() < 0.25 {
        let next = frames(context).runs03();
        context.state_mut().next_frame = next;
    } else {
        soldier_run(context);
    }
}

/// Attack6 refire 2 (`soldier_attack6_refire2`).
fn attack6_refire2(context: &mut MonsterContext) {
    finish_dodge(context);
    context.state_mut().charging = false;
    if context.entity().enemy.is_none()
        || context.state().weapon == MonsterWeapon::Blaster
        || !alive_enemy(context)
        || !context.state().force_refire && target_distance(context) < 440.0
        || !visible(context, None)
    {
        return;
    }
    if context.state().force_refire || context.game.random() < 0.25 {
        let next = frames(context).runs03();
        context.state_mut().next_frame = next;
        context.state_mut().force_refire = false;
    }
}

/// Attack6 shotgun check (`soldier_attack6_shotgun_check`).
fn attack6_shotgun_check(context: &mut MonsterContext) {
    if needs_cock(context) {
        let next = frames(context).runs09();
        context.state_mut().next_frame = next;
        context.state_mut().force_refire = true;
    }
}

/// Duck down (`soldier_duck_down`).
fn soldier_duck_down(context: &mut MonsterContext) {
    if context.state().ducked {
        return;
    }
    set_duck(context, true);
    let pause = context.game.host.now() + 1.0;
    context.state_mut().pause_time = pause;
}

/// Duck hold (`soldier_duck_hold`).
fn soldier_duck_hold(context: &mut MonsterContext) {
    let hold = context.game.host.now() < context.state().pause_time;
    context.state_mut().hold_frame = hold;
}

/// Duck up (`soldier_duck_up`).
fn soldier_duck_up(context: &mut MonsterContext) {
    set_duck(context, false);
}

/// Start charge (`soldier_start_charge`).
fn start_charge(context: &mut MonsterContext) {
    context.state_mut().charging = true;
}

/// Blind (`soldier_blind`).
fn blind(context: &mut MonsterContext) {
    context.set_move("soldier_move_blind", true);
}

/// Blind check (`soldier_blind_check`).
fn blind_check(context: &mut MonsterContext) {
    if context.state().manual_steering {
        let actor = context.actor().clone();
        let origin = context.game.body_of(actor).origin;
        let yaw =
            f64::from(vector_angles(sub3(context.state().blind_fire_target, origin)).y);
        context.state_mut().ideal_yaw = yaw;
    }
}

/// Stand up (`soldier_stand_up`).
fn stand_up(context: &mut MonsterContext) {
    context.set_move("soldier_move_trip", false);
    context.state_mut().next_frame = rerelease_soldier::RUNT08;
}

/// Check prone (`monster_check_prone`).
fn check_prone(context: &mut MonsterContext) {
    if !needs_cock(context) && prone_shot(context) {
        context.set_move("soldier_move_attack5", false);
    }
}

/// Death shrink (`soldier_death_shrink`).
fn death_shrink(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    context.entity_mut().server_flags |= 2;
    let mut moved = body;
    moved.bounds.max.z = 0.0;
    context.game.write_body(actor, &moved, true);
}

/// Hyper laser sound start.
fn hyper_laser_sound_start(context: &mut MonsterContext) {
    laser_sound(context, true);
}

/// Hyper laser sound end.
fn hyper_laser_sound_end(context: &mut MonsterContext) {
    laser_sound(context, false);
}

/// Hyper ripper 1.
fn hyperripper1(context: &mut MonsterContext) {
    fire(context, 0, false);
}

/// Hyper ripper 2.
fn hyperripper2(context: &mut MonsterContext) {
    if context.state().weapon != MonsterWeapon::Machinegun {
        fire(context, 1, false);
    }
}

/// Hyper ripper 3.
fn hyperripper3(context: &mut MonsterContext) {
    hyper_ripper(context, 2, false);
}

/// Hyper ripper 5.
fn hyperripper5(context: &mut MonsterContext) {
    hyper_ripper(context, 8, true);
}

/// Hyper ripper 8.
fn hyperripper8(context: &mut MonsterContext) {
    hyper_ripper(context, 7, true);
}

/// Hyper refire 1.
fn hyper_refire1(context: &mut MonsterContext) {
    if context.state().weapon == MonsterWeapon::Shotgun
        && context.game.random() < 0.7
        && visible(context, None)
    {
        let frame = frames(context).attak103();
        context.entity_mut().frame = frame;
    }
}

/// Hyper refire 2.
fn hyper_refire2(context: &mut MonsterContext) {
    if context.state().weapon == MonsterWeapon::Shotgun
        && context.game.random() < 0.7
        && visible(context, None)
    {
        let frame = frames(context).attak205();
        context.entity_mut().frame = frame;
    }
}

/// Named soldier callbacks (`soldierCallbacks`).
pub fn soldier_callbacks() -> HashMap<String, MonsterHandler> {
    HashMap::from([
        ("soldier_stand".to_string(), MonsterHandler::Callback(soldier_stand)),
        ("soldier_run".to_string(), MonsterHandler::Callback(soldier_run)),
        ("soldier_idle".to_string(), MonsterHandler::Callback(idle)),
        ("soldier_cock".to_string(), MonsterHandler::Callback(cock)),
        ("soldier_walk1_random".to_string(), MonsterHandler::Callback(walk1_random)),
        ("soldier_fire1".to_string(), MonsterHandler::Callback(fire1)),
        ("soldier_fire2".to_string(), MonsterHandler::Callback(fire2)),
        ("soldier_fire3".to_string(), MonsterHandler::Callback(fire3)),
        ("soldier_fire4".to_string(), MonsterHandler::Callback(fire4)),
        ("soldier_fire5".to_string(), MonsterHandler::Callback(fire5)),
        ("soldier_fire6".to_string(), MonsterHandler::Callback(fire6)),
        ("soldier_fire7".to_string(), MonsterHandler::Callback(fire7)),
        ("soldier_fire8".to_string(), MonsterHandler::Callback(fire8)),
        ("soldier_attack1_refire1".to_string(), MonsterHandler::Callback(attack1_refire1)),
        ("soldier_attack1_refire2".to_string(), MonsterHandler::Callback(attack1_refire2)),
        ("soldier_attack1_shotgun_check".to_string(), MonsterHandler::Callback(attack1_shotgun_check)),
        ("soldier_attack2_refire1".to_string(), MonsterHandler::Callback(attack2_refire1)),
        ("soldier_attack2_refire2".to_string(), MonsterHandler::Callback(attack2_refire2)),
        ("soldier_attack2_shotgun_check".to_string(), MonsterHandler::Callback(attack2_shotgun_check)),
        ("soldier_attack3_refire".to_string(), MonsterHandler::Callback(attack3_refire)),
        ("soldier_attack6_refire".to_string(), MonsterHandler::Callback(attack6_refire)),
        ("soldier_attack6_refire1".to_string(), MonsterHandler::Callback(attack6_refire1)),
        ("soldier_attack6_refire2".to_string(), MonsterHandler::Callback(attack6_refire2)),
        ("soldier_attack6_shotgun_check".to_string(), MonsterHandler::Callback(attack6_shotgun_check)),
        ("soldier_duck_down".to_string(), MonsterHandler::Callback(soldier_duck_down)),
        ("soldier_duck_hold".to_string(), MonsterHandler::Callback(soldier_duck_hold)),
        ("soldier_duck_up".to_string(), MonsterHandler::Callback(soldier_duck_up)),
        ("soldier_start_charge".to_string(), MonsterHandler::Callback(start_charge)),
        ("soldier_blind".to_string(), MonsterHandler::Callback(blind)),
        ("soldier_blind_check".to_string(), MonsterHandler::Callback(blind_check)),
        ("soldier_stand_up".to_string(), MonsterHandler::Callback(stand_up)),
        ("monster_check_prone".to_string(), MonsterHandler::Callback(check_prone)),
        ("soldier_dead".to_string(), MonsterHandler::Callback(corpse)),
        ("soldier_death_shrink".to_string(), MonsterHandler::Callback(death_shrink)),
        ("soldierh_hyper_laser_sound_start".to_string(), MonsterHandler::Callback(hyper_laser_sound_start)),
        ("soldierh_hyper_laser_sound_end".to_string(), MonsterHandler::Callback(hyper_laser_sound_end)),
        ("soldierh_hyperripper1".to_string(), MonsterHandler::Callback(hyperripper1)),
        ("soldierh_hyperripper2".to_string(), MonsterHandler::Callback(hyperripper2)),
        ("soldierh_hyperripper3".to_string(), MonsterHandler::Callback(hyperripper3)),
        ("soldierh_hyperripper5".to_string(), MonsterHandler::Callback(hyperripper5)),
        ("soldierh_hyperripper8".to_string(), MonsterHandler::Callback(hyperripper8)),
        ("soldierh_hyper_refire1".to_string(), MonsterHandler::Callback(hyper_refire1)),
        ("soldierh_hyper_refire2".to_string(), MonsterHandler::Callback(hyper_refire2)),
    ])
}
