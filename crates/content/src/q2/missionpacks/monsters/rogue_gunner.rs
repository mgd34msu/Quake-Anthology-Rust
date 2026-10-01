//! Rogue gunner (`src/content/q2/missionpacks/monsters/rogue-gunner.ts`).
//!
//! Original Rogue m_gunner.c weapon, jump and dodge behavior.
//! GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, length3, normalize3, scale3, sub3};

use super::rogue_common::{
    rogue_blocked_check_shot, rogue_duck_down, rogue_duck_hold, rogue_duck_up, rogue_monster_dodge,
};
use super::state::rogue_state;
use super::tables::rogue_gunner::{gunner_frame, gunner_moves};
use crate::q2::base::monsters::common::{damaged_skin, monster_shot};
use crate::q2::base::monsters::gunner::{gunner_definition, gunner_run};
use crate::q2::foundation::host::Q2TraceRequest;
use crate::q2::foundation::monsters::ai::{
    angles_vectors, enemy_body, finish_dodge, project_flash, target_distance, vector_angles, visible,
};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::rerelease::monsters::common::{
    blocked_check_jump, blocked_check_platform, monster_flash, monster_jump_finished, JumpNavigation, JumpResult,
};
use crate::q2::support::contracts::{PainReaction, TraceHit, TraceResult};

/// Grenade check (`grenadeCheck`).
fn grenade_check(context: &mut MonsterContext) -> bool {
    let Some(enemy) = enemy_body(context) else {
        return false;
    };
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let manual = context.state().manual_steering;
    if manual {
        let target = context.state().blind_fire_target;
        let height = context.entity().view_height as f32;
        if body.origin.z + height < target.z {
            return false;
        }
    } else if body.origin.z + body.bounds.max.z <= enemy.origin.z + enemy.bounds.min.z {
        return false;
    }
    let target = if manual {
        context.state().blind_fire_target
    } else {
        enemy.origin
    };
    if length3(sub3(body.origin, target)) < 100.0 {
        return false;
    }
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, 53), None);
    let trace = context.game.host.trace(&Q2TraceRequest {
        start,
        end: target,
        bounds: None,
        ignore: Some(actor),
        mask: 0x6000003,
        exclude: Vec::new(),
    });
    let enemy_id = context.entity().enemy.clone();
    trace.fraction == 1.0 || matches!(&trace.hit, TraceHit::Actor { actor } if Some(actor) == enemy_id.as_ref())
}

/// Grenade (`grenade`).
fn gunner_grenade(context: &mut MonsterContext) {
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let _ = enemy;
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let blind = context.state().manual_steering;
    let frame = context.entity().frame;
    let index = if frame == gunner_frame::ATTAK105 {
        0
    } else if frame == gunner_frame::ATTAK108 {
        1
    } else if frame == gunner_frame::ATTAK111 {
        2
    } else {
        3
    };
    if index == 3 {
        context.state_mut().manual_steering = false;
    }
    let blind_target = blind && !visible(context, None);
    let point = if blind_target {
        context.state().blind_fire_target
    } else {
        body.origin
    };
    if blind_target && point.x == 0.0 && point.y == 0.0 && point.z == 0.0 {
        return;
    }
    let flash = 53 + index;
    let edition = context.game.options.edition;
    let axes = angles_vectors(body.angles);
    let start = project_flash(context, muzzle_offset(edition, flash as usize), None);
    let mut aim = sub3(point, body.origin);
    let distance = length3(aim);
    if distance > 512.0 && aim.z < 64.0 && aim.z > -64.0 {
        aim.z += distance - 512.0;
    }
    let pitch = normalize3(aim).z.clamp(-0.5, 0.4);
    let direction = add3(
        add3(axes.forward, scale3(axes.right, 0.02 + index as f32 * 0.03)),
        scale3(axes.up, pitch),
    );
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
        None,
    );
    monster_flash(context, flash, start, direction);
}

/// Duck down (`duckDown`).
fn gunner_duck_down(context: &mut MonsterContext) {
    context.state_mut().ducked = true;
    if context.game.options.skill >= 2 && context.game.random() > 0.5 {
        gunner_grenade(context);
    }
    rogue_duck_down(context);
}

/// Jumping (`jumping`).
fn gunner_jumping(context: &mut MonsterContext) -> bool {
    let current = context.state().current_move.name.clone();
    current == "gunner_move_jump" || current == "gunner_move_jump2"
}

/// Shooting (`shooting`).
fn gunner_shooting(context: &mut MonsterContext) -> bool {
    let current = context.state().current_move.name.clone();
    current == "gunner_move_attack_chain"
        || current == "gunner_move_fire_chain"
        || current == "gunner_move_attack_grenade"
}

/// Duck (`duck`).
fn gunner_duck(context: &mut MonsterContext, eta: f64) {
    if gunner_jumping(context) {
        return;
    }
    if gunner_shooting(context) && context.game.options.skill != 0 {
        context.state_mut().ducked = false;
        return;
    }
    let skill = context.game.options.skill;
    let wait = context.game.host.now() + eta + if skill == 0 { 1.0 } else { 0.1 * f64::from(3 - skill) };
    context.state_mut().duck_wait = wait;
    gunner_duck_down(context);
    context.state_mut().next_frame = gunner_frame::DUCK01;
    context.set_move("gunner_move_duck", true);
}

/// Sidestep (`sidestep`).
fn gunner_sidestep(context: &mut MonsterContext) {
    if gunner_jumping(context) {
        return;
    }
    if gunner_shooting(context) && context.game.options.skill != 0 {
        context.state_mut().dodging = false;
        return;
    }
    if context.state().current_move.name != "gunner_move_run" {
        context.set_move("gunner_move_run", true);
    }
}

/// Run (`run`).
fn rogue_gunner_run(context: &mut MonsterContext) {
    finish_dodge(context);
    gunner_run(context);
}

/// Jump now (`jumpNow`).
fn gunner_jump_now_inner(context: &mut MonsterContext, up: bool) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let axes = angles_vectors(body.angles);
    let now = context.game.host.now();
    context.entity_mut().timestamp = now;
    let mut moved = body;
    moved.velocity = add3(
        moved.velocity,
        add3(
            scale3(axes.forward, if up { 150.0 } else { 100.0 }),
            scale3(axes.up, if up { 400.0 } else { 300.0 }),
        ),
    );
    context.game.write_body(actor, &moved, true);
}

/// Attack (`attack`).
fn rogue_gunner_attack(context: &mut MonsterContext) {
    finish_dodge(context);
    if context.state().attack_state == MonsterAttackState::Blind {
        let delay = context.state().blind_fire_delay;
        let chance = if delay < 1.0 {
            1.0
        } else if delay < 7.5 {
            0.4
        } else {
            0.1
        };
        let random = context.game.random();
        let delay = delay + 4.1 + context.game.random() * 3.0;
        context.state_mut().blind_fire_delay = delay;
        let target = context.state().blind_fire_target;
        if target.x == 0.0 && target.y == 0.0 && target.z == 0.0 || random > chance {
            return;
        }
        context.state_mut().manual_steering = true;
        if grenade_check(context) {
            context.set_move("gunner_move_attack_grenade", true);
            let finished = context.game.host.now() + 2.0 * context.game.random();
            context.state_mut().attack_finished = finished;
        }
        context.state_mut().manual_steering = false;
        return;
    }
    let actor = context.actor().clone();
    let bad_area = rogue_state(&mut *context.game, &actor).bad_area.is_some();
    if target_distance(context) < 80.0 || bad_area {
        context.set_move("gunner_move_attack_chain", true);
    } else if context.game.random() <= 0.5 && grenade_check(context) {
        context.set_move("gunner_move_attack_grenade", true);
    } else {
        context.set_move("gunner_move_attack_chain", true);
    }
}

/// Pain (`pain`).
fn rogue_gunner_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    damaged_skin(context);
    finish_dodge(context);
    let actor = context.actor().clone();
    if context.game.body_of(actor).ground.is_none() || context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let actor = context.actor().clone();
    let second = (context.game.random() * 32768.0).floor() as i64 & 1 != 0;
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
    if context.game.options.skill == 3 {
        return;
    }
    context.set_move(
        if reaction.damage <= 10.0 {
            "gunner_move_pain3"
        } else if reaction.damage <= 25.0 {
            "gunner_move_pain2"
        } else {
            "gunner_move_pain1"
        },
        false,
    );
    context.state_mut().manual_steering = false;
    if context.state().ducked {
        rogue_duck_up(context);
    }
}

/// Dodge (`dodge`).
fn rogue_gunner_dodge(
    context: &mut MonsterContext,
    attacker: &ActorId,
    eta: f64,
    trace: Option<&TraceResult>,
    _direct: bool,
) {
    rogue_monster_dodge(context, attacker, eta, trace, Some(gunner_duck), Some(gunner_sidestep));
}

/// Duck slot (`duck`).
fn rogue_gunner_duck(context: &mut MonsterContext, eta: f64) -> bool {
    gunner_duck(context, eta);
    true
}

/// Blocked (`blocked`).
fn rogue_gunner_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    let chance = 0.25 + 0.05 * f64::from(context.game.options.skill);
    if rogue_blocked_check_shot(context, chance) || blocked_check_platform(context, distance) {
        return true;
    }
    if blocked_check_jump(context, distance, 192.0, 40.0, true, JumpNavigation::None) == JumpResult::None {
        return false;
    }
    if let Some(enemy) = enemy_body(context) {
        finish_dodge(context);
        let actor = context.actor().clone();
        let above = enemy.origin.z > context.game.body_of(actor).origin.z;
        context.set_move(if above { "gunner_move_jump2" } else { "gunner_move_jump" }, false);
    }
    true
}

/// Chain fire (`GunnerFire`).
fn gunner_fire(context: &mut MonsterContext) {
    if enemy_body(context).is_none() {
        return;
    }
    let flash = 45 + context.entity().frame - gunner_frame::ATTAK216;
    let Some((start, direction)) = monster_shot(context, flash as usize, -0.2) else {
        return;
    };
    let fire_bullet = context.weapons.fire_bullet;
    let actor = context.actor().clone();
    fire_bullet(actor, &mut *context.game, start, direction, 3.0, 4.0, 300.0, 500.0, 0);
    monster_flash(context, flash, start, direction);
}

/// Blind check (`gunner_blind_check`).
fn gunner_blind_check(context: &mut MonsterContext) {
    if context.state().manual_steering {
        let target = context.state().blind_fire_target;
        let actor = context.actor().clone();
        let origin = context.game.body_of(actor).origin;
        context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(target, origin)).y);
    }
}

/// Jump now (`gunner_jump_now`).
fn gunner_jump_now(context: &mut MonsterContext) {
    gunner_jump_now_inner(context, false);
}

/// Second jump now (`gunner_jump2_now`).
fn gunner_jump2_now(context: &mut MonsterContext) {
    gunner_jump_now_inner(context, true);
}

/// Jump wait land (`gunner_jump_wait_land`).
fn gunner_jump_wait_land(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let landed = context.game.body_of(actor).ground.is_some() || monster_jump_finished(context);
    let next = context.entity().frame + if landed { 1 } else { 0 };
    context.state_mut().next_frame = next;
}

/// Create the rogue gunner definition (`createRogueGunnerDefinition`).
pub fn create_rogue_gunner_definition() -> Q2MonsterDefinition {
    let mut definition = gunner_definition();
    definition.moves = gunner_moves();
    definition.blind_fire = true;
    definition.run = MonsterHandler::Callback(rogue_gunner_run);
    definition.attack = MonsterHandler::Callback(rogue_gunner_attack);
    definition.pain = Some(rogue_gunner_pain);
    definition.dodge = Some(rogue_gunner_dodge);
    definition.duck = Some(rogue_gunner_duck);
    definition.blocked = Some(rogue_gunner_blocked);
    definition
        .callbacks
        .insert("gunner_run".to_string(), MonsterHandler::Callback(rogue_gunner_run));
    definition
        .callbacks
        .insert("monster_done_dodge".to_string(), MonsterHandler::Callback(finish_dodge));
    definition.callbacks.insert(
        "gunner_duck_down".to_string(),
        MonsterHandler::Callback(gunner_duck_down),
    );
    definition.callbacks.insert(
        "monster_duck_hold".to_string(),
        MonsterHandler::Callback(rogue_duck_hold),
    );
    definition
        .callbacks
        .insert("monster_duck_up".to_string(), MonsterHandler::Callback(rogue_duck_up));
    definition
        .callbacks
        .insert("GunnerGrenade".to_string(), MonsterHandler::Callback(gunner_grenade));
    definition
        .callbacks
        .insert("GunnerFire".to_string(), MonsterHandler::Callback(gunner_fire));
    definition.callbacks.insert(
        "gunner_blind_check".to_string(),
        MonsterHandler::Callback(gunner_blind_check),
    );
    definition
        .callbacks
        .insert("gunner_jump_now".to_string(), MonsterHandler::Callback(gunner_jump_now));
    definition.callbacks.insert(
        "gunner_jump2_now".to_string(),
        MonsterHandler::Callback(gunner_jump2_now),
    );
    definition.callbacks.insert(
        "gunner_jump_wait_land".to_string(),
        MonsterHandler::Callback(gunner_jump_wait_land),
    );
    definition
}
