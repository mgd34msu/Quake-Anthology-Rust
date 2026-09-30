//! Black widow (`src/content/q2/missionpacks/monsters/widow.ts`).
//!
//! Original Rogue m_widow.c. ZeniMax Media, GPL-2.0-or-later.

use qa_core::math::{Bounds, Vec3, normalize3, sub3, vec3};

use super::power_armor::{PowerArmorKind, monster_power_armor};
use super::rogue_common::rogue_blocked_check_shot;
use super::spawn::rogue_spawn_callbacks;
use super::state::rogue_state;
use super::tables::rogue_widow::{widow_frame, widow_moves};
use super::types::{mission_services, mission_weapons};
use super::widow_common::{
    widow_clear_powerups, widow_power_think, widow_powerups, widow_project,
    widow_restore_armor, widow_slots, widow_slots_left, widow_summon,
};
use super::widow_death::{spawn_widow_legs, widow_debris_callbacks, widow_effect};
use crate::q2::base::monsters::common::{damaged_skin, finish_corpse, move_handler};
use crate::q2::foundation::host::{Q2Solid, Q2TraceRequest};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, enemy_body, enemy_eye, health, project_flash,
    target_distance, vector_angles,
};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{
    MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition,
    record_at,
};
use crate::q2::rerelease::monsters::common::{monster_flash, predicted_direction};
use crate::q2::support::contracts::{DeathReaction, PainReaction, TraceHit};

/// Sweep angles (`sweepAngles`).
const SWEEP_ANGLES: [f64; 9] = [32.0, 26.0, 20.0, 10.0, 0.0, -6.5, -13.0, -27.0, -41.0];

/// Target angle (`targetAngle`).
fn widow_target_angle(context: &mut MonsterContext) -> f64 {
    let Some(enemy) = enemy_body(context) else {
        return 0.0;
    };
    let actor = context.actor().clone();
    let body = context.game.body_of(actor);
    let mut angle = f64::from(body.angles.y) - f64::from(vector_angles(sub3(body.origin, enemy.origin)).y);
    if angle < 0.0 {
        angle += 360.0;
    }
    angle - 180.0
}

/// Torso (`torso`).
fn widow_torso(context: &mut MonsterContext) -> i32 {
    let angle = widow_target_angle(context);
    if angle >= 105.0 || angle <= -75.0 {
        context.set_move(
            if angle >= 105.0 {
                "widow_move_attack_post_blaster_r"
            } else {
                "widow_move_attack_post_blaster_l"
            },
            false,
        );
        context.state_mut().manual_steering = false;
        return 0;
    }
    for index in 0..17 {
        if angle >= 95.0 - f64::from(index) * 10.0 {
            return widow_frame::FIRED03 + index;
        }
    }
    widow_frame::FIRED20
}

/// Run (`run`).
fn widow_run(context: &mut MonsterContext) {
    context.state_mut().hold_frame = false;
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "widow_move_stand"
        } else {
            "widow_move_run"
        },
        false,
    );
}

/// Rail move (`railMove`).
fn widow_rail_move(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "gladiator/railgun.wav", 1, 1.0, 1.0);
    context.set_move("widow_move_attack_pre_rail", false);
}

/// Blaster (`blaster`).
fn widow_blaster(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let enemy = enemy_body(context);
    let eye = enemy_eye(context);
    let (Some(enemy), Some(_)) = (enemy, eye) else {
        return;
    };
    context.game.mission_monsters.widow_shots_fired += 1;
    let effect = if context.game.mission_monsters.widow_shots_fired % 4 == 0 {
        8
    } else {
        0
    };
    let frame = context.entity().frame;
    if frame >= widow_frame::SPAWN05 && frame <= widow_frame::SPAWN13 {
        let index = frame - widow_frame::SPAWN05;
        let flash = 156 + index;
        let edition = context.game.options.edition;
        let start = project_flash(context, muzzle_offset(edition, flash as usize), None);
        let body = context.game.body_of(actor.clone());
        let pitch = vector_angles(sub3(enemy.origin, start)).x;
        let sweep = *record_at(&SWEEP_ANGLES, index as usize);
        let aimed = vec3(body.angles.x + pitch, body.angles.y - sweep as f32, body.angles.z);
        let direction = angles_vectors(aimed).forward;
        let weapons = mission_weapons(&*context.game);
        let damage = 10.0 * f64::from(context.game.mission_monsters.widow_damage_multiplier);
        weapons.fire_blaster2(actor, &mut *context.game, start, direction, damage, 1000.0, effect);
        monster_flash(context, flash, start, direction);
        return;
    }
    if frame >= widow_frame::FIRED02A && frame <= widow_frame::FIRED20 {
        context.state_mut().manual_steering = true;
        let torso = widow_torso(context);
        let frame = context.entity().frame;
        context.state_mut().next_frame = if torso == 0 { frame } else { torso };
        let flash = if frame == widow_frame::FIRED02A {
            175
        } else {
            165 + frame - widow_frame::FIRED03
        };
        let edition = context.game.options.edition;
        let start = project_flash(context, muzzle_offset(edition, flash as usize), None);
        let offset = context.game.random() * 0.1 - 0.05;
        let aim = predicted_direction(context, start, 1000.0, true, offset);
        let Some(aim) = aim else {
            return;
        };
        let angles = vector_angles(aim);
        let yaw = context.game.body_of(actor.clone()).angles.y;
        let mut aim_angle = 100.0 - 10.0 * f64::from(flash - 165);
        if aim_angle <= 0.0 {
            aim_angle += 360.0;
        }
        let mut enemy_angle = f64::from(yaw) - f64::from(angles.y);
        if enemy_angle <= 0.0 {
            enemy_angle += 360.0;
        }
        let error = aim_angle - enemy_angle;
        let direction = angles_vectors(vec3(
            angles.x,
            if error > 15.0 {
                yaw - aim_angle as f32 + 15.0
            } else if error < -15.0 {
                yaw - aim_angle as f32 - 15.0
            } else {
                angles.y
            },
            angles.z,
        ))
        .forward;
        let weapons = mission_weapons(&*context.game);
        let damage = 10.0 * f64::from(context.game.mission_monsters.widow_damage_multiplier);
        weapons.fire_blaster2(actor, &mut *context.game, start, direction, damage, 1000.0, effect);
        monster_flash(context, flash, start, direction);
        return;
    }
    if frame >= widow_frame::RUN01 && frame <= widow_frame::RUN08 {
        let flash = 183 + frame - widow_frame::RUN01;
        let edition = context.game.options.edition;
        let start = project_flash(context, muzzle_offset(edition, flash as usize), None);
        let eye = enemy_eye(context).unwrap_or(enemy.origin);
        let direction = sub3(eye, start);
        let weapons = mission_weapons(&*context.game);
        let damage = 10.0 * f64::from(context.game.mission_monsters.widow_damage_multiplier);
        weapons.fire_blaster2(actor, &mut *context.game, start, direction, damage, 1000.0, effect);
        monster_flash(context, flash, start, direction);
    }
}

/// Check attack (`checkAttack`).
fn widow_check_attack(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    let enemy = enemy_body(context);
    let eye = enemy_eye(context);
    let enemy_id = context.entity().enemy.clone();
    let (Some(_), Some(_), Some(enemy_id)) = (enemy, eye, enemy_id) else {
        return false;
    };
    widow_powerups(context);
    let frame = context.entity().frame;
    if context.state().current_move.name == "widow_move_run"
        && (frame >= widow_frame::WALK04 && frame <= widow_frame::WALK08
            || frame == widow_frame::WALK12)
    {
        return false;
    }
    let distance = target_distance(context);
    if context.game.random() < 0.8 && widow_slots_left(context) >= 2 && distance > 150.0 {
        rogue_state(&mut *context.game, &actor).blocked = true;
        context.state_mut().attack_state = MonsterAttackState::Missile;
        return true;
    }
    if health(&mut *context.game, Some(&enemy_id)) > 0.0 {
        let origin = context.game.body_of(actor.clone()).origin;
        let start = vec3(origin.x, origin.y, origin.z + context.entity().view_height as f32);
        let eye = enemy_eye(context).unwrap_or(origin);
        let trace = context.game.host.trace(&Q2TraceRequest {
            start,
            end: eye,
            bounds: None,
            ignore: Some(actor.clone()),
            mask: 1 | 0x2000000 | 8 | 16,
            exclude: Vec::new(),
        });
        if !matches!(&trace.hit, TraceHit::Actor { actor } if *actor == enemy_id) {
            if context.game.host.is_player(&enemy_id) && widow_slots_left(context) >= 2 {
                context.state_mut().attack_state = MonsterAttackState::Blind;
                return true;
            }
            let solid_none = context
                .game
                .entity(&enemy_id)
                .is_some_and(|enemy| enemy.solid == Q2Solid::None);
            if !solid_none || trace.fraction < 1.0 {
                return false;
            }
        }
    }
    let enemy = enemy_body(context).map(|enemy| enemy.origin).unwrap_or(origin_of(context));
    let origin = context.game.body_of(actor).origin;
    context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(enemy, origin)).y);
    if distance <= 100.0 {
        if context.game.options.skill == 0 && (context.game.random() * 4.0).floor() as i32 != 0 {
            return false;
        }
        context.state_mut().attack_state = MonsterAttackState::Melee;
        return true;
    }
    if context.game.host.now() < context.state().attack_finished {
        return false;
    }
    let stand_ground = context.state().stand_ground;
    let chance = if stand_ground {
        0.4
    } else if distance < 80.0 {
        0.8
    } else if distance < 500.0 {
        0.7
    } else if distance < 1000.0 {
        0.6
    } else {
        0.5
    };
    if context.game.random() < chance
        || context.game.entity(&enemy_id).is_some_and(|enemy| enemy.solid == Q2Solid::None)
    {
        context.state_mut().attack_state = MonsterAttackState::Missile;
        return true;
    }
    false
}

/// Origin helper for the widow check attack.
fn origin_of(context: &mut MonsterContext) -> Vec3 {
    let actor = context.actor().clone();
    context.game.body_of(actor).origin
}

/// Attack (`attack`).
fn widow_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let blocked = rogue_state(&mut *context.game, &actor).blocked;
    let anger = context.state().target_anger;
    context.state_mut().move_target = None;
    rogue_state(&mut *context.game, &actor).blocked = false;
    context.state_mut().target_anger = false;
    if enemy_body(context).is_none() {
        return;
    }
    if mission_services(&*context.game).bad_area(&actor) {
        if context.game.random() < 0.1 || context.game.host.now() < context.entity().timestamp {
            context.set_move("widow_move_attack_pre_blaster", false);
        } else {
            widow_rail_move(context);
        }
        return;
    }
    let frame = context.entity().frame;
    let rail_frames = frame == widow_frame::WALK13
        || frame >= widow_frame::WALK01 && frame <= widow_frame::WALK03;
    let blaster_frames = frame >= widow_frame::WALK09 && frame <= widow_frame::WALK12;
    widow_slots(context);
    if (context.state().attack_state == MonsterAttackState::Blind || blocked)
        && widow_slots_left(context) >= 2
    {
        context.set_move("widow_move_spawn", false);
        return;
    }
    if target_distance(context) > 300.0 && !anger && context.game.random() < 0.5 && !blocked {
        context.set_move("widow_move_run_attack", false);
        return;
    }
    if blaster_frames {
        if widow_slots_left(context) >= 2 {
            context.set_move("widow_move_spawn", false);
            return;
        }
        if context.state().pause_time + 2.0 <= context.game.host.now() {
            context.set_move("widow_move_attack_pre_blaster", false);
            return;
        }
    }
    if rail_frames && context.game.host.now() >= context.entity().timestamp {
        widow_rail_move(context);
        return;
    }
    if blaster_frames || rail_frames {
        return;
    }
    let luck = context.game.random();
    if widow_slots_left(context) >= 2 {
        if luck <= 0.4 && context.state().pause_time + 2.0 <= context.game.host.now() {
            context.set_move("widow_move_attack_pre_blaster", false);
        } else if luck <= 0.7 && context.game.host.now() >= context.entity().timestamp {
            widow_rail_move(context);
        } else {
            context.set_move("widow_move_spawn", false);
        }
        return;
    }
    if context.game.host.now() < context.entity().timestamp {
        context.set_move("widow_move_attack_pre_blaster", false);
        return;
    }
    if luck <= 0.5 || context.game.host.now() + 2.0 >= context.state().pause_time {
        widow_rail_move(context);
    } else {
        context.set_move("widow_move_attack_pre_blaster", false);
    }
}

/// Stand (`stand`).
fn widow_stand(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "widow/laugh.wav", 2, 1.0, 1.0);
    context.set_move("widow_move_stand", false);
}

/// Sight (`sight`).
fn widow_sight(context: &mut MonsterContext) {
    context.state_mut().pause_time = 0.0;
}

/// Initialize (`initialize`).
fn widow_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let skill = f64::from(context.game.options.skill);
    let coop = if context.game.options.mode == crate::q2::foundation::host::Q2Mode::Coop {
        500.0 * skill
    } else {
        0.0
    };
    let max_health = 2000.0 + 1000.0 * skill + coop;
    context.entity_mut().max_health = max_health;
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_health(&owned, max_health);
    context.entity_mut().laser_immune = true;
    context.state_mut().ignore_shots = true;
    context.entity_mut().prethink = Some(widow_power_think);
    widow_slots(context);
    context.game.mission_monsters.widow_damage_multiplier = 1;
    if context.game.options.skill == 3 {
        monster_power_armor(context, PowerArmorKind::Shield, 500.0);
    }
}

/// Blocked (`blocked`).
fn widow_blocked(context: &mut MonsterContext, _distance: f64) -> bool {
    if context.state().current_move.name == "widow_move_run_attack" {
        context.state_mut().target_anger = true;
        if widow_check_attack(context) {
            widow_attack(context);
        } else {
            widow_run(context);
        }
        return true;
    }
    let chance = 0.25 + 0.05 * f64::from(context.game.options.skill);
    rogue_blocked_check_shot(context, chance)
}

/// Pain (`pain`).
fn widow_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    let skill = context.game.options.skill;
    if skill == 3 || context.game.host.now() < context.state().pain_time {
        return;
    }
    if context.state().pause_time == 100000000.0 {
        context.state_mut().pause_time = 0.0;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 5.0;
    let actor = context.actor().clone();
    if reaction.damage < 15.0 {
        context.game.sound(&actor, "widow/bw1pain1.wav", 2, 1.0, 0.0);
        return;
    }
    if context.game.random()
        < if reaction.damage < 75.0 {
            0.6 - 0.2 * f64::from(skill)
        } else {
            0.75 - 0.1 * f64::from(skill)
        }
    {
        context.state_mut().manual_steering = false;
        context.set_move(
            if reaction.damage < 75.0 {
                "widow_move_pain_light"
            } else {
                "widow_move_pain_heavy"
            },
            false,
        );
    }
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if reaction.damage < 75.0 {
            "widow/bw1pain2.wav"
        } else {
            "widow/bw1pain3.wav"
        },
        2,
        1.0,
        0.0,
    );
}

/// Die (`die`).
fn widow_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    let actor = context.actor().clone();
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = false;
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &crate::q2::support::contracts::CombatTraitChanges {
            can_take_damage: Some(false),
            ..crate::q2::support::contracts::CombatTraitChanges::default()
        },
    );
    context.entity_mut().count = 0;
    widow_clear_powerups(context);
    context.set_move("widow_move_death", false);
}

/// Step (`widow_step`).
fn widow_step(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "widow/bwstep3.wav", 4, 1.0, 1.0);
}

/// Step shoot (`widow_stepshoot`).
fn widow_stepshoot(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "widow/bwstep3.wav", 4, 1.0, 1.0);
    widow_blaster(context);
}

/// Start run 5 (`widow_start_run_5`).
fn widow_start_run_5(context: &mut MonsterContext) {
    context.set_move("widow_move_run", false);
    context.state_mut().next_frame = widow_frame::WALK05;
}

/// Start run 10 (`widow_start_run_10`).
fn widow_start_run_10(context: &mut MonsterContext) {
    context.set_move("widow_move_run", false);
    context.state_mut().next_frame = widow_frame::WALK10;
}

/// Start run 12 (`widow_start_run_12`).
fn widow_start_run_12(context: &mut MonsterContext) {
    context.set_move("widow_move_run", false);
    context.state_mut().next_frame = widow_frame::WALK12;
}

/// Attack blaster (`widow_attack_blaster`).
fn widow_attack_blaster(context: &mut MonsterContext) {
    let pause = context.game.host.now() + 1.0 + 2.0 * context.game.random();
    context.state_mut().pause_time = pause;
    context.set_move("widow_move_attack_blaster", false);
    let torso = widow_torso(context);
    context.state_mut().next_frame = torso;
}

/// Reattack blaster (`widow_reattack_blaster`).
fn widow_reattack_blaster(context: &mut MonsterContext) {
    widow_blaster(context);
    let current = context.state().current_move.name.clone();
    if current == "widow_move_attack_post_blaster_r"
        || current == "widow_move_attack_post_blaster_l"
        || context.state().pause_time >= context.game.host.now()
    {
        return;
    }
    context.state_mut().manual_steering = false;
    context.set_move("widow_move_attack_post_blaster", false);
}

/// Save loc (`WidowSaveLoc`).
fn widow_save_loc(context: &mut MonsterContext) {
    if let Some(eye) = enemy_eye(context) {
        context.entity_mut().pos1 = eye;
    }
}

/// Rail (`WidowRail`).
fn widow_rail(context: &mut MonsterContext) {
    let current = context.state().current_move.name.clone();
    let flash = if current == "widow_move_attack_rail_l" {
        154
    } else if current == "widow_move_attack_rail_r" {
        155
    } else {
        150
    };
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, flash as usize), None);
    let pos1 = context.entity().pos1;
    let direction = normalize3(sub3(pos1, start));
    let fire_rail = context.weapons.fire_rail;
    let actor = context.actor().clone();
    let damage = 50.0 * f64::from(context.game.mission_monsters.widow_damage_multiplier);
    fire_rail(actor, &mut *context.game, start, direction, damage, 100.0);
    monster_flash(context, flash, start, direction);
    let timestamp = context.game.host.now() + 3.0;
    context.entity_mut().timestamp = timestamp;
}

/// Start rail (`widow_start_rail`).
fn widow_start_rail(context: &mut MonsterContext) {
    context.state_mut().manual_steering = true;
}

/// Rail done (`widow_rail_done`).
fn widow_rail_done(context: &mut MonsterContext) {
    context.state_mut().manual_steering = false;
}

/// Attack rail (`widow_attack_rail`).
fn widow_attack_rail(context: &mut MonsterContext) {
    let angle = widow_target_angle(context);
    context.set_move(
        if angle < -15.0 {
            "widow_move_attack_rail_l"
        } else if angle > 15.0 {
            "widow_move_attack_rail_r"
        } else {
            "widow_move_attack_rail"
        },
        false,
    );
}

/// Start spawn (`widow_start_spawn`).
fn widow_start_spawn(context: &mut MonsterContext) {
    context.state_mut().manual_steering = true;
}

/// Done spawn (`widow_done_spawn`).
fn widow_done_spawn(context: &mut MonsterContext) {
    context.state_mut().manual_steering = false;
}

/// Ready spawn (`widow_ready_spawn`).
fn widow_ready_spawn(context: &mut MonsterContext) {
    widow_blaster(context);
    widow_summon(context, false, true);
}

/// Spawn check (`widow_spawn_check`).
fn widow_spawn_check(context: &mut MonsterContext) {
    widow_blaster(context);
    widow_summon(context, false, false);
}

/// Attack kick (`widow_attack_kick`).
fn widow_attack_kick(context: &mut MonsterContext) {
    let enemy = enemy_body(context);
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let damage = 50.0 + (context.game.random() * 6.0).floor();
    let kick = if enemy.as_ref().is_some_and(|enemy| enemy.ground.is_none()) {
        250.0
    } else {
        500.0
    };
    fire_hit(actor, &mut *context.game, vec3(100.0, 0.0, 4.0), damage, kick);
}

/// Spawn out start (`spawn_out_start`).
fn widow_spawn_out_start(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let wait = context.game.host.now() + 2.0;
    context.entity_mut().wait = wait;
    let first = widow_project(&actor, &mut *context.game, vec3(12.58, -43.71, 68.88));
    widow_effect(&mut *context.game, first, "q2:widowbeamout", 20001);
    let second = widow_project(&actor, &mut *context.game, vec3(3.43, 58.72, 68.41));
    widow_effect(&mut *context.game, second, "q2:widowbeamout", 20002);
    context.game.sound(&actor, "misc/bwidowbeamout.wav", 2, 1.0, 1.0);
}

/// Spawn out do (`spawn_out_do`).
fn widow_spawn_out_do(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let first = widow_project(&actor, &mut *context.game, vec3(12.58, -43.71, 68.88));
    widow_effect(&mut *context.game, first, "q2:widowsplash", 1);
    let second = widow_project(&actor, &mut *context.game, vec3(3.43, 58.72, 68.41));
    widow_effect(&mut *context.game, second, "q2:widowsplash", 1);
    let origin = context.game.body_of(actor.clone()).origin;
    widow_effect(
        &mut *context.game,
        vec3(origin.x, origin.y, origin.z + 36.0),
        "q2:bosstport",
        1,
    );
    spawn_widow_legs(&actor, &mut *context.game);
    context.game.remove_actor(actor);
}

/// Dead (`widow_dead`).
fn widow_dead(context: &mut MonsterContext) {
    finish_corpse(
        context,
        Bounds {
            min: vec3(-56.0, -56.0, 0.0),
            max: vec3(56.0, 56.0, 80.0),
        },
    );
}

/// Create the widow definition (`createWidowDefinition`).
pub fn create_widow_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_widow",
        "widow",
        "models/monsters/blackwidow/tris.md2",
        2000.0,
        -5000.0,
        1500.0,
        Bounds {
            min: vec3(-40.0, -40.0, 0.0),
            max: vec3(40.0, 40.0, 144.0),
        },
        1.0,
        "widow_move_stand",
        widow_moves(),
        MonsterHandler::Callback(widow_stand),
        move_handler("widow_move_walk"),
        MonsterHandler::Callback(widow_run),
        MonsterHandler::Callback(widow_attack),
        widow_die,
    );
    definition.yaw_speed = Some(30.0);
    definition.sight = Some(MonsterHandler::Callback(widow_sight));
    definition.melee = Some(move_handler("widow_move_attack_kick"));
    definition.check_attack = Some(widow_check_attack);
    let mut source_callbacks = widow_debris_callbacks();
    for (key, think) in rogue_spawn_callbacks().think {
        source_callbacks.think.insert(key, think);
    }
    source_callbacks.think.insert("q2:rogue/widow_powerups", widow_power_think);
    definition.source_callbacks = Some(source_callbacks);
    definition.initialize = Some(MonsterHandler::Callback(widow_initialize));
    definition.restore = Some(MonsterHandler::Callback(widow_restore_armor));
    definition.blocked = Some(widow_blocked);
    definition.pain = Some(widow_pain);
    for (name, handler) in [
        ("widow_run", MonsterHandler::Callback(widow_run)),
        ("WidowBlaster", MonsterHandler::Callback(widow_blaster)),
        ("widow_step", MonsterHandler::Callback(widow_step)),
        ("widow_stepshoot", MonsterHandler::Callback(widow_stepshoot)),
        ("widow_start_run_5", MonsterHandler::Callback(widow_start_run_5)),
        ("widow_start_run_10", MonsterHandler::Callback(widow_start_run_10)),
        ("widow_start_run_12", MonsterHandler::Callback(widow_start_run_12)),
        ("widow_attack_blaster", MonsterHandler::Callback(widow_attack_blaster)),
        ("widow_reattack_blaster", MonsterHandler::Callback(widow_reattack_blaster)),
        ("WidowSaveLoc", MonsterHandler::Callback(widow_save_loc)),
        ("WidowRail", MonsterHandler::Callback(widow_rail)),
        ("widow_start_rail", MonsterHandler::Callback(widow_start_rail)),
        ("widow_rail_done", MonsterHandler::Callback(widow_rail_done)),
        ("widow_attack_rail", MonsterHandler::Callback(widow_attack_rail)),
        ("widow_start_spawn", MonsterHandler::Callback(widow_start_spawn)),
        ("widow_done_spawn", MonsterHandler::Callback(widow_done_spawn)),
        ("widow_ready_spawn", MonsterHandler::Callback(widow_ready_spawn)),
        ("widow_spawn_check", MonsterHandler::Callback(widow_spawn_check)),
        ("widow_attack_kick", MonsterHandler::Callback(widow_attack_kick)),
        ("spawn_out_start", MonsterHandler::Callback(widow_spawn_out_start)),
        ("spawn_out_do", MonsterHandler::Callback(widow_spawn_out_do)),
        ("widow_dead", MonsterHandler::Callback(widow_dead)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
