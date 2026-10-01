//! Tank monster (`src/content/q2/base/monsters/tank.ts`).
//!
//! Quake II m_tank.c. id Software, GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::math::{sub3, vec3, Bounds, Vec3};

use super::common::{alive_enemy, finish_corpse, monster_muzzle, monster_shot, move_handler, sound_handler};
use super::tables::tank::{tank_frame, tank_moves};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, enemy_eye, health, project_flash, target_distance, vector_angles, visible,
};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction};

/// Run (`run`).
fn tank_run(context: &mut MonsterContext) {
    let enemy = context.entity().enemy.clone();
    let brutal = enemy.as_ref().is_some_and(|enemy| context.game.host.is_player(enemy));
    context.state_mut().brutal = brutal;
    if context.state().stand_ground {
        context.set_move("tank_move_stand", true);
        return;
    }
    let current = context.state().current_move.name.clone();
    if current == "tank_move_walk" || current == "tank_move_start_run" {
        context.set_move("tank_move_run", true);
    } else {
        context.set_move("tank_move_start_run", true);
    }
}

/// Attack (`attack`).
pub(crate) fn tank_attack(context: &mut MonsterContext) {
    let enemy = context.entity().enemy.clone();
    if health(&mut *context.game, enemy.as_ref()) < 0.0 {
        context.state_mut().brutal = false;
        context.set_move("tank_move_attack_strike", true);
        return;
    }
    let range = target_distance(context);
    let r = context.game.random();
    if range <= 125.0 {
        context.set_move(
            if r < 0.4 {
                "tank_move_attack_chain"
            } else {
                "tank_move_attack_blast"
            },
            false,
        );
    } else if range <= 250.0 {
        context.set_move(
            if r < 0.5 {
                "tank_move_attack_chain"
            } else {
                "tank_move_attack_blast"
            },
            false,
        );
    } else if r < 0.33 {
        context.set_move("tank_move_attack_chain", true);
    } else if r < 0.66 {
        let now = context.game.host.now();
        context.state_mut().pain_time = now + 5.0;
        context.set_move("tank_move_attack_pre_rocket", true);
    } else {
        context.set_move("tank_move_attack_blast", true);
    }
}

/// Pain (`pain`).
fn tank_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.entity().max_health;
    if health(&mut *context.game, Some(&actor)) < max_health / 2.0 {
        context.entity_mut().skin |= 1;
    }
    if reaction.damage <= 10.0 || context.game.host.now() < context.state().pain_time {
        return;
    }
    if reaction.damage <= 30.0 && context.game.random() > 0.2 {
        return;
    }
    let frame = context.entity().frame;
    if context.game.options.skill >= 2
        && ((tank_frame::ATTAK301..=tank_frame::ATTAK330).contains(&frame)
            || (tank_frame::ATTAK101..=tank_frame::ATTAK116).contains(&frame))
    {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    context.game.sound(&actor, "tank/tnkpain2.wav", 2, 1.0, 1.0);
    if context.game.options.skill == 3 {
        return;
    }
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

/// Die (`die`).
fn tank_die(context: &mut MonsterContext, reaction: &DeathReaction) {
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
            "models/objects/gibs/chest/tris.md2",
            damage,
            Q2GibOptions::default(),
        );
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
    context.game.sound(&actor, "tank/death.wav", 2, 1.0, 1.0);
    context.state_mut().dead = true;
    context.state_mut().can_take_damage = true;
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    context.set_move("tank_move_death", true);
}

/// Dead (`tank_dead`).
fn tank_dead(context: &mut MonsterContext) {
    finish_corpse(
        context,
        Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: -16.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 0.0,
            },
        },
    );
}

/// Post strike (`tank_poststrike`).
fn tank_poststrike(context: &mut MonsterContext) {
    context.entity_mut().enemy = None;
    tank_run(context);
}

/// Reattack blaster (`tank_reattack_blaster`).
fn tank_reattack_blaster(context: &mut MonsterContext) {
    if context.game.options.skill >= 2 && visible(context, None) && alive_enemy(context) && context.game.random() <= 0.6
    {
        context.set_move("tank_move_reattack_blast", true);
    } else {
        context.set_move("tank_move_attack_post_blast", true);
    }
}

/// Refire rocket (`tank_refire_rocket`).
fn tank_refire_rocket(context: &mut MonsterContext) {
    if context.game.options.skill >= 2 && alive_enemy(context) && visible(context, None) && context.game.random() <= 0.4
    {
        context.set_move("tank_move_attack_fire_rocket", true);
    } else {
        context.set_move("tank_move_attack_post_rocket", true);
    }
}

/// Blaster (`TankBlaster`).
fn tank_blaster(context: &mut MonsterContext) {
    let frame = context.entity().frame;
    let flash = if frame == tank_frame::ATTAK110 {
        1usize
    } else if frame == tank_frame::ATTAK113 {
        2
    } else {
        3
    };
    let Some((start, direction)) = monster_shot(context, flash, 0.0) else {
        return;
    };
    let fire_blaster = context.weapons.fire_blaster;
    let actor = context.actor().clone();
    fire_blaster(
        actor,
        &mut *context.game,
        start,
        direction,
        30.0,
        800.0,
        8,
        false,
        Mod::BLASTER,
    );
    monster_muzzle(context, flash as i32, direction, start);
}

/// Rocket (`TankRocket`).
fn tank_rocket(context: &mut MonsterContext) {
    let frame = context.entity().frame;
    let flash = if frame == tank_frame::ATTAK324 {
        23usize
    } else if frame == tank_frame::ATTAK327 {
        24
    } else {
        25
    };
    let Some((start, direction)) = monster_shot(context, flash, 0.0) else {
        return;
    };
    let fire_rocket = context.weapons.fire_rocket;
    let actor = context.actor().clone();
    fire_rocket(actor, &mut *context.game, start, direction, 50.0, 550.0, 70.0, 50.0);
    monster_muzzle(context, flash as i32, direction, start);
}

/// Machine gun (`TankMachineGun`).
pub(crate) fn tank_machine_gun(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let frame = context.entity().frame;
    let flash = 4 + frame - tank_frame::ATTAK406;
    let edition = context.game.options.edition;
    let start = project_flash(context, muzzle_offset(edition, flash as usize), None);
    let eye = enemy_eye(context);
    let angles = context.game.body_of(actor).angles;
    let pitch = eye.map(|eye| vector_angles(sub3(eye, start)).x).unwrap_or(0.0);
    let yaw = if frame <= tank_frame::ATTAK415 {
        angles.y - 8.0 * (frame - tank_frame::ATTAK411) as f32
    } else {
        angles.y + 8.0 * (frame - tank_frame::ATTAK419) as f32
    };
    let direction = angles_vectors(vec3(pitch, yaw, 0.0)).forward;
    let fire_bullet = context.weapons.fire_bullet;
    let actor = context.actor().clone();
    fire_bullet(actor, &mut *context.game, start, direction, 20.0, 4.0, 300.0, 500.0, 0);
    monster_muzzle(context, flash, direction, start);
}

/// Tank definition (`tankDefinition`).
pub fn tank_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_tank",
        "tank",
        "models/monsters/tank/tris.md2",
        750.0,
        -200.0,
        500.0,
        Bounds {
            min: Vec3 {
                x: -32.0,
                y: -32.0,
                z: -16.0,
            },
            max: Vec3 {
                x: 32.0,
                y: 32.0,
                z: 72.0,
            },
        },
        1.0,
        "tank_move_stand",
        tank_moves(),
        move_handler("tank_move_stand"),
        move_handler("tank_move_walk"),
        MonsterHandler::Callback(tank_run),
        MonsterHandler::Callback(tank_attack),
        tank_die,
    );
    definition.sight = Some(sound_handler("tank/sight1.wav", 2, 1.0));
    definition.idle = Some(sound_handler("tank/tnkidle1.wav", 2, 2.0));
    definition.pain = Some(tank_pain);
    definition.callbacks = HashMap::from([
        ("tank_stand".to_string(), move_handler("tank_move_stand")),
        ("tank_walk".to_string(), move_handler("tank_move_walk")),
        ("tank_run".to_string(), MonsterHandler::Callback(tank_run)),
        ("tank_footstep".to_string(), sound_handler("tank/step.wav", 4, 1.0)),
        ("tank_thud".to_string(), sound_handler("tank/tnkdeth2.wav", 4, 1.0)),
        ("tank_windup".to_string(), sound_handler("tank/tnkatck4.wav", 1, 1.0)),
        ("TankStrike".to_string(), sound_handler("tank/tnkatck5.wav", 1, 1.0)),
        ("tank_dead".to_string(), MonsterHandler::Callback(tank_dead)),
        ("tank_poststrike".to_string(), MonsterHandler::Callback(tank_poststrike)),
        (
            "tank_doattack_rocket".to_string(),
            move_handler("tank_move_attack_fire_rocket"),
        ),
        (
            "tank_reattack_blaster".to_string(),
            MonsterHandler::Callback(tank_reattack_blaster),
        ),
        (
            "tank_refire_rocket".to_string(),
            MonsterHandler::Callback(tank_refire_rocket),
        ),
        ("TankBlaster".to_string(), MonsterHandler::Callback(tank_blaster)),
        ("TankRocket".to_string(), MonsterHandler::Callback(tank_rocket)),
        ("TankMachineGun".to_string(), MonsterHandler::Callback(tank_machine_gun)),
    ]);
    definition
}

/// Tank commander initialize (`initialize`).
fn tank_commander_initialize(context: &mut MonsterContext) {
    context.entity_mut().skin = 2;
}

/// Tank commander definition (`tankCommanderDefinition`).
pub fn tank_commander_definition() -> Q2MonsterDefinition {
    let mut definition = tank_definition();
    definition.classname = "monster_tank_commander".to_string();
    definition.health = 1000.0;
    definition.gib_health = -225.0;
    definition.initialize = Some(MonsterHandler::Callback(tank_commander_initialize));
    definition
}
