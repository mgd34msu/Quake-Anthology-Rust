//! Rogue tank and chick variants (`src/content/q2/missionpacks/monsters/rogue-arsenal.ts`).
//!
//! Original Rogue Tank and Iron Maiden weapon and dodge behavior.
//! GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, length3, normalize3, scale3, sub3, vec3, Vec3};

use super::rogue_common::{
    rogue_blocked_check_shot, rogue_duck_down, rogue_duck_hold, rogue_duck_up, rogue_monster_dodge, source_trace_world,
};
use super::tables::rogue_chick::{chick_frame, chick_moves};
use crate::q2::base::monsters::chick::{chick_definition, chick_run};
use crate::q2::base::monsters::common::{alive_enemy, damaged_skin};
use crate::q2::base::monsters::tables::tank::tank_frame;
use crate::q2::base::monsters::tank::{tank_attack, tank_definition, tank_machine_gun};
use crate::q2::foundation::host::Q2TraceRequest;
use crate::q2::foundation::monsters::ai::{
    angles_vectors, enemy_body, finish_dodge, health, project_flash, target_distance, visible,
};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::rerelease::monsters::common::{blocked_check_platform, monster_flash};
use crate::q2::support::contracts::{PainReaction, TraceHit, TraceResult};

/// Rogue rocket (`rocket`).
fn rogue_rocket(context: &mut MonsterContext, flash: usize, head_chance: f64, blind_offset: f64) {
    let actor = context.actor().clone();
    let enemy = enemy_body(context);
    let enemy_id = context.entity().enemy.clone();
    let (Some(enemy), Some(enemy_id)) = (enemy, enemy_id) else {
        return;
    };
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, flash), None);
    let right = angles_vectors(context.game.body_of(actor.clone()).angles).right;
    let speed = 500.0 + 100.0 * f64::from(context.game.options.skill);
    let blind = context.state().manual_steering;
    let target = if blind {
        context.state().blind_fire_target
    } else {
        enemy.origin
    };
    let mut point = if blind {
        target
    } else {
        let view_height = context
            .game
            .entities
            .get(&enemy_id)
            .map(|entity| entity.view_height)
            .unwrap_or(22);
        let head = context.game.random() < head_chance || start.z < enemy.origin.z + enemy.bounds.min.z;
        vec3(
            target.x,
            target.y,
            if head {
                target.z + view_height as f32
            } else {
                enemy.origin.z + enemy.bounds.min.z
            },
        )
    };
    let skill = context.game.options.skill;
    if !blind && context.game.random() < 0.2 + f64::from(3 - skill) * 0.15 {
        let flight = f64::from(length3(sub3(point, start))) / speed;
        point = add3(point, scale3(enemy.velocity, flight as f32));
    }
    let trace_to = |game: &mut crate::q2::foundation::host::Q2GameServices, end: Vec3| -> TraceResult {
        game.host.trace(&Q2TraceRequest {
            start,
            end,
            bounds: None,
            ignore: Some(actor.clone()),
            mask: 0x6000003,
            exclude: Vec::new(),
        })
    };
    let mut trace = trace_to(&mut *context.game, point);
    if blind {
        if trace.start_solid || trace.all_solid || trace.fraction < 0.5 {
            point = add3(target, scale3(right, -(blind_offset as f32)));
            trace = trace_to(&mut *context.game, point);
            if trace.start_solid || trace.all_solid || trace.fraction < 0.5 {
                point = add3(target, scale3(right, blind_offset as f32));
                trace = trace_to(&mut *context.game, point);
                if trace.start_solid || trace.all_solid || trace.fraction < 0.5 {
                    return;
                }
            }
        }
    } else {
        // Both source callbacks perform this second trace, then use the
        // Chick flash even for a Tank.
        trace = trace_to(&mut *context.game, point);
        let hit_enemy = matches!(&trace.hit, TraceHit::Actor { actor } if *actor == enemy_id);
        if !(hit_enemy || source_trace_world(&mut *context.game, &trace)) {
            return;
        }
        let hit_player = matches!(&trace.hit, TraceHit::Actor { actor } if context.game.host.is_player(actor));
        if !(trace.fraction > 0.5 || hit_player) {
            return;
        }
    }
    let direction = normalize3(sub3(point, start));
    let fire_rocket = context.weapons.fire_rocket;
    fire_rocket(actor, &mut *context.game, start, direction, 50.0, speed, 70.0, 50.0);
    monster_flash(context, if blind { flash as i32 } else { 57 }, start, direction);
}

/// Blind chance (`blindChance`).
fn blind_chance(context: &mut MonsterContext) -> f64 {
    if context.state().blind_fire_delay < 1.0 {
        1.0
    } else if context.state().blind_fire_delay < 7.5 {
        0.4
    } else {
        0.1
    }
}

/// Zero target (`zeroTarget`).
fn zero_target(context: &mut MonsterContext) -> bool {
    let point = context.state().blind_fire_target;
    point.x == 0.0 && point.y == 0.0 && point.z == 0.0
}

/// Rogue blocked (`blocked`).
fn rogue_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    let chance = 0.25 + 0.05 * f64::from(context.game.options.skill);
    rogue_blocked_check_shot(context, chance) || blocked_check_platform(context, distance)
}

/// Rogue tank pain (`pain`).
fn rogue_tank_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.entity().max_health;
    if health(&mut *context.game, Some(&actor)) < max_health / 2.0 {
        context.entity_mut().skin |= 1;
    }
    if reaction.damage <= 10.0
        || context.game.host.now() < context.state().pain_time
        || reaction.damage <= 30.0 && context.game.random() > 0.2
    {
        return;
    }
    let frame = context.entity().frame;
    if context.game.options.skill >= 2
        && (frame >= tank_frame::ATTAK301 && frame <= tank_frame::ATTAK330
            || frame >= tank_frame::ATTAK101 && frame <= tank_frame::ATTAK116)
    {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    context.game.sound(&actor, "tank/tnkpain2.wav", 2, 1.0, 1.0);
    if context.game.options.skill == 3 {
        return;
    }
    context.state_mut().manual_steering = false;
    context.set_move(
        if reaction.damage <= 30.0 {
            "tank_move_pain1"
        } else if reaction.damage <= 60.0 {
            "tank_move_pain2"
        } else {
            "tank_move_pain3"
        },
        false,
    );
}

/// Rogue tank attack (`attack`).
fn rogue_tank_attack(context: &mut MonsterContext) {
    let enemy = context.entity().enemy.clone();
    let Some(enemy) = enemy else { return };
    if !context.game.host.actors().is_live(&enemy) {
        return;
    }
    if health(&mut *context.game, Some(&enemy)) < 0.0 {
        context.state_mut().brutal = false;
        context.set_move("tank_move_attack_strike", true);
        return;
    }
    if context.state().attack_state != MonsterAttackState::Blind {
        tank_attack(context);
        return;
    }
    let chance = blind_chance(context);
    let random = context.game.random();
    let delay = context.state().blind_fire_delay + 5.2 + context.game.random() * 3.0;
    context.state_mut().blind_fire_delay = delay;
    if zero_target(context) || random > chance {
        return;
    }
    context.state_mut().manual_steering = true;
    context.set_move("tank_move_attack_fire_rocket", true);
    let now = context.game.host.now();
    let attack_finished = now + 3.0 + 2.0 * context.game.random();
    context.state_mut().attack_finished = attack_finished;
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 5.0;
}

/// Rogue tank rocket (`TankRocket`).
fn rogue_tank_rocket(context: &mut MonsterContext) {
    let frame = context.entity().frame;
    let flash = if frame == tank_frame::ATTAK324 {
        23usize
    } else if frame == tank_frame::ATTAK327 {
        24
    } else {
        25
    };
    rogue_rocket(context, flash, 0.66, 20.0);
}

/// Rogue tank machine gun (`TankMachineGun`).
fn rogue_tank_machine_gun(context: &mut MonsterContext) {
    if enemy_body(context).is_none() {
        return;
    }
    tank_machine_gun(context);
}

/// Rogue tank refire (`tank_refire_rocket`).
fn rogue_tank_refire_rocket(context: &mut MonsterContext) {
    if context.state().manual_steering {
        context.state_mut().manual_steering = false;
        context.set_move("tank_move_attack_post_rocket", true);
        return;
    }
    let elite = context.game.options.skill >= 2;
    let acquire = elite && alive_enemy(context) && visible(context, None) && context.game.random() <= 0.4;
    context.set_move(
        if acquire {
            "tank_move_attack_fire_rocket"
        } else {
            "tank_move_attack_post_rocket"
        },
        false,
    );
}

/// Rogue tank initialize (`initialize`).
fn rogue_tank_initialize(context: &mut MonsterContext) {
    context.state_mut().ignore_shots = true;
}

/// Rogue commander initialize (`initialize`).
fn rogue_commander_initialize(context: &mut MonsterContext) {
    context.state_mut().ignore_shots = true;
    context.entity_mut().skin = 2;
}

/// Rogue chick run (`chickRun`).
fn rogue_chick_run(context: &mut MonsterContext) {
    finish_dodge(context);
    chick_run(context);
}

/// Whether the chick is shooting (`chickShooting`).
fn chick_shooting(context: &mut MonsterContext) -> bool {
    let current = context.state().current_move.name.clone();
    current == "chick_move_start_attack1" || current == "chick_move_attack1"
}

/// Rogue chick duck (`duck`).
fn rogue_chick_duck_inner(context: &mut MonsterContext, eta: f64) {
    if chick_shooting(context) && context.game.options.skill != 0 {
        context.state_mut().ducked = false;
        return;
    }
    let skill = context.game.options.skill;
    let now = context.game.host.now();
    context.state_mut().duck_wait = now + eta + if skill == 0 { 1.0 } else { 0.1 * f64::from(3 - skill) };
    rogue_duck_down(context);
    context.state_mut().next_frame = chick_frame::DUCK01;
    context.set_move("chick_move_duck", true);
}

/// Rogue chick sidestep (`sidestep`).
fn rogue_chick_sidestep_inner(context: &mut MonsterContext) {
    if chick_shooting(context) && context.game.options.skill != 0 {
        context.state_mut().dodging = false;
        return;
    }
    if context.state().current_move.name != "chick_move_run" {
        context.set_move("chick_move_run", true);
    }
}

/// Rogue chick attack (`attack`).
fn rogue_chick_attack(context: &mut MonsterContext) {
    finish_dodge(context);
    if context.state().attack_state == MonsterAttackState::Blind {
        let chance = blind_chance(context);
        let random = context.game.random();
        let delay = context.state().blind_fire_delay + 5.5 + context.game.random();
        context.state_mut().blind_fire_delay = delay;
        if zero_target(context) || random > chance {
            return;
        }
        context.state_mut().manual_steering = true;
        context.set_move("chick_move_start_attack1", true);
        let now = context.game.host.now();
        let attack_finished = now + 2.0 * context.game.random();
        context.state_mut().attack_finished = attack_finished;
        return;
    }
    context.set_move("chick_move_start_attack1", true);
}

/// Rogue chick pain (`pain`).
fn rogue_chick_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    finish_dodge(context);
    damaged_skin(context);
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let random = context.game.random();
    let actor = context.actor().clone();
    context.game.sound(
        &actor,
        if random < 0.33 {
            "chick/chkpain1.wav"
        } else if random < 0.66 {
            "chick/chkpain2.wav"
        } else {
            "chick/chkpain3.wav"
        },
        2,
        1.0,
        1.0,
    );
    if context.game.options.skill == 3 {
        return;
    }
    context.state_mut().manual_steering = false;
    context.set_move(
        if reaction.damage <= 10.0 {
            "chick_move_pain1"
        } else if reaction.damage <= 25.0 {
            "chick_move_pain2"
        } else {
            "chick_move_pain3"
        },
        false,
    );
    if context.state().ducked {
        rogue_duck_up(context);
    }
}

/// Rogue chick dodge (`dodge`).
fn rogue_chick_dodge(
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
        Some(rogue_chick_duck_inner),
        Some(rogue_chick_sidestep_inner),
    );
}

/// Rogue chick duck slot (`duck`).
fn rogue_chick_duck(context: &mut MonsterContext, eta: f64) -> bool {
    rogue_chick_duck_inner(context, eta);
    true
}

/// Rogue chick sidestep slot (`sidestep`).
fn rogue_chick_sidestep(context: &mut MonsterContext) -> bool {
    rogue_chick_sidestep_inner(context);
    true
}

/// Rogue chick rocket (`ChickRocket`).
fn rogue_chick_rocket(context: &mut MonsterContext) {
    rogue_rocket(context, 57, 0.33, 10.0);
}

/// Rogue chick rerocket (`chick_rerocket`).
fn rogue_chick_rerocket(context: &mut MonsterContext) {
    if context.state().manual_steering {
        context.state_mut().manual_steering = false;
        context.set_move("chick_move_end_attack1", true);
        return;
    }
    let skill = context.game.options.skill;
    let acquire = alive_enemy(context)
        && target_distance(context) >= 80.0
        && visible(context, None)
        && context.game.random() <= 0.6 + 0.05 * f64::from(skill);
    context.set_move(
        if acquire {
            "chick_move_attack1"
        } else {
            "chick_move_end_attack1"
        },
        false,
    );
}

/// Create rogue arsenal monsters (`createRogueArsenalMonsters`).
pub fn create_rogue_arsenal_monsters() -> Vec<Q2MonsterDefinition> {
    let mut tank = tank_definition();
    tank.blind_fire = true;
    tank.blocked = Some(rogue_blocked);
    tank.initialize = Some(MonsterHandler::Callback(rogue_tank_initialize));
    tank.pain = Some(rogue_tank_pain);
    tank.attack = MonsterHandler::Callback(rogue_tank_attack);
    tank.callbacks
        .insert("TankRocket".to_string(), MonsterHandler::Callback(rogue_tank_rocket));
    tank.callbacks.insert(
        "TankMachineGun".to_string(),
        MonsterHandler::Callback(rogue_tank_machine_gun),
    );
    tank.callbacks.insert(
        "tank_refire_rocket".to_string(),
        MonsterHandler::Callback(rogue_tank_refire_rocket),
    );

    let mut commander = tank.clone();
    commander.classname = "monster_tank_commander".to_string();
    commander.health = 1000.0;
    commander.gib_health = -225.0;
    commander.initialize = Some(MonsterHandler::Callback(rogue_commander_initialize));

    let mut chick = chick_definition();
    chick.model = "models/monsters/bitch2/tris.md2".to_string();
    chick.moves = chick_moves();
    chick.run = MonsterHandler::Callback(rogue_chick_run);
    chick.blocked = Some(rogue_blocked);
    chick.blind_fire = true;
    chick.attack = MonsterHandler::Callback(rogue_chick_attack);
    chick.pain = Some(rogue_chick_pain);
    chick.dodge = Some(rogue_chick_dodge);
    chick.duck = Some(rogue_chick_duck);
    chick.sidestep = Some(rogue_chick_sidestep);
    chick
        .callbacks
        .insert("chick_run".to_string(), MonsterHandler::Callback(rogue_chick_run));
    chick.callbacks.insert(
        "monster_duck_down".to_string(),
        MonsterHandler::Callback(rogue_duck_down),
    );
    chick.callbacks.insert(
        "monster_duck_hold".to_string(),
        MonsterHandler::Callback(rogue_duck_hold),
    );
    chick
        .callbacks
        .insert("monster_duck_up".to_string(), MonsterHandler::Callback(rogue_duck_up));
    chick
        .callbacks
        .insert("ChickRocket".to_string(), MonsterHandler::Callback(rogue_chick_rocket));
    chick.callbacks.insert(
        "chick_rerocket".to_string(),
        MonsterHandler::Callback(rogue_chick_rerocket),
    );

    vec![tank, commander, chick]
}
