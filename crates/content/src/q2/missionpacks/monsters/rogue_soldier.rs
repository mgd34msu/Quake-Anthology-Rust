//! Rogue soldier variants (`src/content/q2/missionpacks/monsters/rogue-soldier.ts`).
//!
//! Original Rogue m_soldier.c behavior. ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, add3, dot3, normalize3, scale3, sub3};

use super::rogue_common::{
    rogue_blocked_check_shot, rogue_duck_down, rogue_duck_hold, rogue_duck_up,
    rogue_monster_dodge,
};
use super::state::rogue_state;
use super::tables::rogue_soldier::{soldier_frame, soldier_moves};
use crate::q2::base::monsters::common::{
    HUMANOID_BOUNDS, finish_corpse, finish_corpse_default,
};
use crate::q2::foundation::host::Q2TraceRequest;
use crate::q2::foundation::monsters::ai::{
    angles_vectors, enemy_body, enemy_eye, finish_dodge, health, project_flash, target_distance,
    vector_angles, visible,
};
use crate::q2::foundation::monsters::soldier::{
    soldier_callbacks, soldier_die, soldier_run, soldier_stand, soldier_walk,
};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{
    MonsterContext, MonsterHandler, Q2MonsterDefinition, record_at,
};
use crate::q2::rerelease::monsters::common::{blocked_check_platform, monster_flash};
use crate::q2::support::contracts::{PainReaction, TraceHit, TraceResult};

/// Blaster flashes.
const BLASTER_FLASHES: [usize; 8] = [39, 40, 83, 86, 89, 92, 95, 98];
/// Shotgun flashes.
const SHOTGUN_FLASHES: [usize; 8] = [41, 42, 84, 87, 90, 93, 96, 99];
/// Machinegun flashes.
const MACHINEGUN_FLASHES: [usize; 8] = [43, 44, 85, 88, 91, 94, 97, 100];

/// Run (`run`).
fn rogue_soldier_run(context: &mut MonsterContext) {
    finish_dodge(context);
    soldier_run(context);
}

/// Stand (`stand`).
fn rogue_soldier_stand(context: &mut MonsterContext) {
    if context.entity().spawnflags & 8 != 0 {
        context.set_move("soldier_move_blind", true);
    } else {
        soldier_stand(context);
    }
}

/// Fire (`fire`).
fn rogue_soldier_fire(context: &mut MonsterContext, input: i32) {
    if enemy_body(context).is_none() {
        context.state_mut().hold_frame = false;
        return;
    }
    let actor = context.actor().clone();
    let skin = context.entity().skin;
    let index = input.unsigned_abs() as usize;
    let flashes = if skin < 2 {
        &BLASTER_FLASHES
    } else if skin < 4 {
        &SHOTGUN_FLASHES
    } else {
        &MACHINEGUN_FLASHES
    };
    let flash = *record_at(flashes, index);
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, flash), None);
    let body = context.game.body_of(actor.clone());
    let mut aim = angles_vectors(body.angles).forward;
    if index != 5 && index != 6 {
        let eye = enemy_eye(context);
        let Some(eye) = eye else { return };
        let direction = sub3(eye, start);
        if input < 0 && dot3(normalize3(direction), aim) < 0.9 {
            return;
        }
        let axes = angles_vectors(vector_angles(direction));
        let spread = if context.game.options.skill < 2 { 1000.0 } else { 500.0 };
        let rx = context.game.random() * 2.0 - 1.0;
        let rz = context.game.random() * 2.0 - 1.0;
        aim = normalize3(add3(
            scale3(axes.forward, 8192.0),
            add3(
                scale3(axes.right, (rx * spread) as f32),
                scale3(axes.up, (rz * spread * 0.5) as f32),
            ),
        ));
        let trace = context.game.host.trace(&Q2TraceRequest {
            start,
            end: eye,
            bounds: None,
            ignore: Some(actor),
            mask: 0x6000003,
            exclude: Vec::new(),
        });
        let enemy = context.entity().enemy.clone();
        if matches!(&trace.hit, TraceHit::Actor { actor } if Some(actor) != enemy.as_ref()) {
            return;
        }
    }
    let fire = context.weapons;
    let actor = context.actor().clone();
    if skin <= 1 {
        (fire.fire_blaster)(actor, &mut *context.game, start, aim, 5.0, 600.0, 8, false, crate::q2::foundation::weapons::types::Mod::BLASTER);
    } else if skin <= 3 {
        (fire.fire_shotgun)(
            actor,
            &mut *context.game,
            start,
            aim,
            2.0,
            1.0,
            1000.0,
            500.0,
            12,
            0,
        );
    } else {
        if !context.state().hold_frame {
            let wait = context.game.host.now()
                + (3.0 + ((context.game.random() * 32768.0).floor() as i64 % 8) as f64) * 0.1;
            context.entity_mut().wait = wait;
        }
        (fire.fire_bullet)(
            actor,
            &mut *context.game,
            start,
            aim,
            2.0,
            4.0,
            300.0,
            500.0,
            0,
        );
        let hold = context.game.host.now() < context.entity().wait;
        context.state_mut().hold_frame = hold;
    }
    monster_flash(context, flash as i32, start, aim);
}

/// Duck (`duck`).
fn rogue_soldier_duck_inner(context: &mut MonsterContext, eta: f64) {
    rogue_duck_down(context);
    let skill = context.game.options.skill;
    let simple = skill == 0 || context.game.random() > f64::from(skill) * 0.3;
    context.state_mut().next_frame = if simple {
        soldier_frame::DUCK01
    } else {
        soldier_frame::ATTAK301
    };
    context.set_move(
        if simple {
            "soldier_move_duck"
        } else {
            "soldier_move_attack3"
        },
        false,
    );
    let now = context.game.host.now();
    context.state_mut().duck_wait =
        now + eta + if skill == 0 || !simple { 1.0 } else { 0.1 * f64::from(3 - skill) };
}

/// Sidestep (`sidestep`).
fn rogue_soldier_sidestep_inner(context: &mut MonsterContext) {
    let animation = if context.entity().skin <= 3 {
        "soldier_move_attack6"
    } else {
        "soldier_move_start_run"
    };
    if context.state().current_move.name != animation {
        context.set_move(animation, true);
    }
}

/// Refire (`refire`).
fn rogue_soldier_refire(context: &mut MonsterContext, first: bool, blaster: bool) {
    if first && blaster && context.state().manual_steering {
        context.state_mut().manual_steering = false;
        return;
    }
    let enemy = context.entity().enemy.clone();
    let skin = context.entity().skin;
    if enemy.is_none() || (skin <= 1) != blaster || health(&mut *context.game, enemy.as_ref()) <= 0.0
    {
        return;
    }
    if context.game.options.skill == 3 && context.game.random() < 0.5
        || target_distance(context) < 80.0
    {
        context.state_mut().next_frame = if first {
            soldier_frame::ATTAK102
        } else {
            soldier_frame::ATTAK204
        };
    } else if blaster {
        context.state_mut().next_frame = if first {
            soldier_frame::ATTAK110
        } else {
            soldier_frame::ATTAK216
        };
    }
}

/// Attack (`attack`).
fn rogue_soldier_attack(context: &mut MonsterContext) {
    finish_dodge(context);
    if context.state().attack_state == crate::q2::foundation::monsters::types::MonsterAttackState::Blind {
        let chance = if context.state().blind_fire_delay < 1.0 {
            1.0
        } else if context.state().blind_fire_delay < 7.5 {
            0.4
        } else {
            0.1
        };
        let random = context.game.random();
        let delay = context.state().blind_fire_delay + 4.1 + context.game.random() * 3.0;
        context.state_mut().blind_fire_delay = delay;
        let point = context.state().blind_fire_target;
        if point.x == 0.0 && point.y == 0.0 && point.z == 0.0 || random > chance {
            return;
        }
        context.state_mut().manual_steering = true;
        context.set_move("soldier_move_attack1", true);
        let finished = context.game.host.now() + 1.5 + context.game.random();
        context.state_mut().attack_finished = finished;
        return;
    }
    let random = context.game.random();
    let actor = context.actor().clone();
    let blocked = rogue_state(&mut *context.game, &actor).blocked;
    let skin = context.entity().skin;
    if !blocked
        && !context.state().stand_ground
        && target_distance(context) >= 80.0
        && random < f64::from(context.game.options.skill) * 0.25
        && skin <= 3
    {
        context.set_move("soldier_move_attack6", true);
        return;
    }
    let first = skin < 4 && context.game.random() < 0.5;
    context.set_move(
        if skin < 4 {
            if first {
                "soldier_move_attack1"
            } else {
                "soldier_move_attack2"
            }
        } else {
            "soldier_move_attack4"
        },
        false,
    );
}

/// Sight (`sight`).
fn rogue_soldier_sight(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let path = if context.game.random() < 0.5 {
        "soldier/solsght1.wav"
    } else {
        "soldier/solsrch1.wav"
    };
    context.game.sound(&actor, path, 2, 1.0, 1.0);
    let enemy = context.entity().enemy.clone();
    let skin = context.entity().skin;
    if context.game.options.skill > 0
        && enemy.is_some()
        && target_distance(context) >= 80.0
        && context.game.random() > 0.75
        && skin <= 3
    {
        context.set_move("soldier_move_attack6", true);
    }
}

/// Pain (`pain`).
fn rogue_soldier_pain(context: &mut MonsterContext, _reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.entity().max_health;
    if health(&mut *context.game, Some(&actor)) < max_health / 2.0 {
        context.entity_mut().skin |= 1;
    }
    finish_dodge(context);
    context.state_mut().charging = false;
    context.state_mut().manual_steering = false;
    let airborne = context.game.body_of(actor).velocity.z > 100.0;
    if context.game.host.now() < context.state().pain_time {
        let current = context.state().current_move.name.clone();
        if airborne
            && (current == "soldier_move_pain1"
                || current == "soldier_move_pain2"
                || current == "soldier_move_pain3")
        {
            if context.state().ducked {
                rogue_duck_up(context);
            }
            context.set_move("soldier_move_pain4", true);
        }
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let skin = context.entity().skin;
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if skin | 1 == 1 {
            "soldier/solpain2.wav"
        } else if skin | 1 == 3 {
            "soldier/solpain1.wav"
        } else {
            "soldier/solpain3.wav"
        },
        2,
        1.0,
        1.0,
    );
    if airborne {
        if context.state().ducked {
            rogue_duck_up(context);
        }
        context.set_move("soldier_move_pain4", true);
        return;
    }
    if context.game.options.skill == 3 {
        return;
    }
    let random = context.game.random();
    context.set_move(
        if random < 0.33 {
            "soldier_move_pain1"
        } else if random < 0.66 {
            "soldier_move_pain2"
        } else {
            "soldier_move_pain3"
        },
        false,
    );
    if context.state().ducked {
        rogue_duck_up(context);
    }
}

/// Dodge (`dodge`).
fn rogue_soldier_dodge(
    context: &mut MonsterContext,
    attacker: &ActorId,
    eta: f64,
    trace: Option<&TraceResult>,
    _direct: bool,
) {
    rogue_monster_dodge(
        context,
        attacker,
        eta,
        trace,
        Some(rogue_soldier_duck_inner),
        Some(rogue_soldier_sidestep_inner),
    );
}

/// Duck slot (`duck`).
fn rogue_soldier_duck(context: &mut MonsterContext, eta: f64) -> bool {
    rogue_soldier_duck_inner(context, eta);
    true
}

/// Blocked (`blocked`).
fn rogue_soldier_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    if context.state().dodging || context.state().ducked {
        return false;
    }
    let chance = 0.25 + 0.05 * f64::from(context.game.options.skill);
    rogue_blocked_check_shot(context, chance) || blocked_check_platform(context, distance)
}

/// Fire callbacks.
fn soldier_fire1(context: &mut MonsterContext) {
    rogue_soldier_fire(context, 0);
}
fn soldier_fire2(context: &mut MonsterContext) {
    rogue_soldier_fire(context, 1);
}
fn soldier_fire3(context: &mut MonsterContext) {
    rogue_duck_down(context);
    rogue_soldier_fire(context, 2);
}
fn soldier_fire4(context: &mut MonsterContext) {
    rogue_soldier_fire(context, 3);
}
fn soldier_fire6(context: &mut MonsterContext) {
    rogue_soldier_fire(context, 5);
}
fn soldier_fire7(context: &mut MonsterContext) {
    rogue_soldier_fire(context, 6);
}
fn soldier_fire8(context: &mut MonsterContext) {
    rogue_soldier_fire(context, -7);
}

/// Fire while running (`soldier_fire_run`).
fn soldier_fire_run(context: &mut MonsterContext) {
    if context.entity().skin <= 1
        && context.entity().enemy.is_some()
        && visible(context, None)
    {
        rogue_soldier_fire(context, 0);
    }
}

/// Refire callbacks.
fn soldier_attack1_refire1(context: &mut MonsterContext) {
    rogue_soldier_refire(context, true, true);
}
fn soldier_attack1_refire2(context: &mut MonsterContext) {
    rogue_soldier_refire(context, true, false);
}
fn soldier_attack2_refire1(context: &mut MonsterContext) {
    rogue_soldier_refire(context, false, true);
}
fn soldier_attack2_refire2(context: &mut MonsterContext) {
    rogue_soldier_refire(context, false, false);
}

/// Attack3 refire (`soldier_attack3_refire`).
fn soldier_attack3_refire(context: &mut MonsterContext) {
    if context.game.host.now() + 0.4 < context.state().duck_wait {
        context.state_mut().next_frame = soldier_frame::ATTAK303;
    }
}

/// Attack6 refire (`soldier_attack6_refire`).
fn soldier_attack6_refire(context: &mut MonsterContext) {
    finish_dodge(context);
    context.state_mut().charging = false;
    let enemy = context.entity().enemy.clone();
    let skill = context.game.options.skill;
    if enemy.is_some()
        && health(&mut *context.game, enemy.as_ref()) > 0.0
        && target_distance(context) >= 80.0
        && (skill == 3 || context.game.random() < 0.25 * f64::from(skill))
    {
        context.state_mut().next_frame = soldier_frame::RUNS03;
    }
}

/// Dead (`soldier_dead2`).
fn soldier_dead2(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    let start = Vec3 {
        x: origin.x,
        y: origin.y,
        z: origin.z + 1.0,
    };
    let bounds = Bounds {
        min: Vec3 {
            x: -32.0,
            y: -32.0,
            z: -24.0,
        },
        max: Vec3 {
            x: 32.0,
            y: 32.0,
            z: -8.0,
        },
    };
    let trace = context.game.host.trace(&Q2TraceRequest {
        start,
        end: start,
        bounds: Some(bounds),
        ignore: Some(actor),
        mask: 3,
        exclude: Vec::new(),
    });
    if trace.start_solid || trace.all_solid {
        finish_corpse_default(context);
    } else {
        finish_corpse(context, bounds);
    }
}

/// Stop charge (`soldier_stop_charge`).
fn soldier_stop_charge(context: &mut MonsterContext) {
    context.state_mut().charging = false;
}

/// Create rogue soldier definitions (`createRogueSoldierDefinitions`).
pub fn create_rogue_soldier_definitions() -> Vec<Q2MonsterDefinition> {
    [
        "monster_soldier_light",
        "monster_soldier",
        "monster_soldier_ss",
    ]
    .into_iter()
    .map(|classname| {
        let mut definition = Q2MonsterDefinition::new(
            classname,
            "soldier",
            "models/monsters/soldier/tris.md2",
            if classname == "monster_soldier_light" {
                20.0
            } else if classname == "monster_soldier" {
                30.0
            } else {
                40.0
            },
            -30.0,
            100.0,
            HUMANOID_BOUNDS,
            1.0,
            "soldier_move_stand1",
            soldier_moves(),
            MonsterHandler::Callback(rogue_soldier_stand),
            MonsterHandler::Callback(soldier_walk),
            MonsterHandler::Callback(rogue_soldier_run),
            MonsterHandler::Callback(rogue_soldier_attack),
            soldier_die,
        );
        definition.blind_fire = classname == "monster_soldier_light";
        definition.initialize = Some(MonsterHandler::Callback(rogue_soldier_stand));
        definition.sight = Some(MonsterHandler::Callback(rogue_soldier_sight));
        definition.pain = Some(rogue_soldier_pain);
        definition.dodge = Some(rogue_soldier_dodge);
        definition.duck = Some(rogue_soldier_duck);
        definition.blocked = Some(rogue_soldier_blocked);
        let mut callbacks = soldier_callbacks();
        callbacks.insert(
            "soldier_run".to_string(),
            MonsterHandler::Callback(rogue_soldier_run),
        );
        callbacks.insert(
            "monster_done_dodge".to_string(),
            MonsterHandler::Callback(finish_dodge),
        );
        callbacks.insert(
            "monster_duck_down".to_string(),
            MonsterHandler::Callback(rogue_duck_down),
        );
        callbacks.insert(
            "monster_duck_hold".to_string(),
            MonsterHandler::Callback(rogue_duck_hold),
        );
        callbacks.insert(
            "monster_duck_up".to_string(),
            MonsterHandler::Callback(rogue_duck_up),
        );
        callbacks.insert(
            "soldier_stop_charge".to_string(),
            MonsterHandler::Callback(soldier_stop_charge),
        );
        callbacks.insert(
            "soldier_fire1".to_string(),
            MonsterHandler::Callback(soldier_fire1),
        );
        callbacks.insert(
            "soldier_fire2".to_string(),
            MonsterHandler::Callback(soldier_fire2),
        );
        callbacks.insert(
            "soldier_fire3".to_string(),
            MonsterHandler::Callback(soldier_fire3),
        );
        callbacks.insert(
            "soldier_fire4".to_string(),
            MonsterHandler::Callback(soldier_fire4),
        );
        callbacks.insert(
            "soldier_fire6".to_string(),
            MonsterHandler::Callback(soldier_fire6),
        );
        callbacks.insert(
            "soldier_fire7".to_string(),
            MonsterHandler::Callback(soldier_fire7),
        );
        callbacks.insert(
            "soldier_fire8".to_string(),
            MonsterHandler::Callback(soldier_fire8),
        );
        callbacks.insert(
            "soldier_fire_run".to_string(),
            MonsterHandler::Callback(soldier_fire_run),
        );
        callbacks.insert(
            "soldier_attack1_refire1".to_string(),
            MonsterHandler::Callback(soldier_attack1_refire1),
        );
        callbacks.insert(
            "soldier_attack1_refire2".to_string(),
            MonsterHandler::Callback(soldier_attack1_refire2),
        );
        callbacks.insert(
            "soldier_attack2_refire1".to_string(),
            MonsterHandler::Callback(soldier_attack2_refire1),
        );
        callbacks.insert(
            "soldier_attack2_refire2".to_string(),
            MonsterHandler::Callback(soldier_attack2_refire2),
        );
        callbacks.insert(
            "soldier_attack3_refire".to_string(),
            MonsterHandler::Callback(soldier_attack3_refire),
        );
        callbacks.insert(
            "soldier_attack6_refire".to_string(),
            MonsterHandler::Callback(soldier_attack6_refire),
        );
        callbacks.insert(
            "soldier_dead2".to_string(),
            MonsterHandler::Callback(soldier_dead2),
        );
        definition.callbacks = callbacks;
        definition
    })
    .collect()
}
