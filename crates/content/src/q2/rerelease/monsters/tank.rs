//! Rerelease tank (`src/content/q2/rerelease/monsters/tank.ts`).
//!
//! ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, length3, normalize3, scale3, sub3, vec3, Vec3};

use super::common::{
    blocked_check_platform, chainfist, check_gib, monster_flash, predict_aim, predicted_direction, reacts_to_pain,
};
use super::tables::tank::{tank_frame, tank_moves};
use crate::q2::base::monsters::tank::tank_definition;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::number_field;
use crate::q2::foundation::host::{
    Q2Edition, Q2EffectEvent, Q2Mode, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SpawnFn, Q2TraceRequest,
    SpawnModule,
};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, clear_shot, corpse, enemy_body, enemy_eye, health, project_flash, vector_angles, visible,
};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::monsters::muzzle::muzzle_offset;
use crate::q2::foundation::monsters::types::{MonsterAttackState, MonsterContext, MonsterHandler, Q2MonsterDefinition};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::missionpacks::monsters::types::mission_weapons;
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, PainReaction, TraceHit};

/// Blind aim (`blindAim`).
fn tank_blind_aim(context: &mut MonsterContext, start: Vec3, target: Vec3, right: Vec3) -> Option<Vec3> {
    let actor = context.actor().clone();
    for side in [0.0, -20.0, 20.0] {
        let end = add3(target, scale3(right, side));
        let trace = context.game.host.trace(&Q2TraceRequest {
            start,
            end,
            bounds: None,
            ignore: Some(actor.clone()),
            mask: 0x4600_4003,
            exclude: Vec::new(),
        });
        if !trace.start_solid && !trace.all_solid && trace.fraction >= 0.5 {
            return Some(normalize3(sub3(end, start)));
        }
    }
    None
}

/// Initialize (`initialize`).
fn rerelease_tank_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.game.require_entity(&actor).spawnflags & 8 == 0 {
        return;
    }
    let spawn = context.game.require_entity(&actor).spawn.clone();
    let max_health = 1500.0 * number_field(&spawn, "health_multiplier", 1.0);
    context.game.require_entity_mut(&actor).max_health = max_health;
    let owned = context.game.owned_of(actor.clone());
    context.game.host.combat().set_health(&owned, max_health);
    if number_field(&spawn, "scale", 0.0) != 0.0 {
        return;
    }
    context.game.require_entity_mut(&actor).scale = 1.5;
    context.state_mut().scale = 1.5;
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.min = vec3(-48.0, -48.0, -24.0);
    moved.bounds.max = vec3(48.0, 48.0, 108.0);
    context.game.write_body(actor.clone(), &moved, true);
    context.state_mut().normal_height = 108.0;
    context.game.require_entity_mut(&actor).view_height = 100;
    let owned = context.game.owned_of(actor);
    context.game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            mass: Some(750.0),
            ..CombatTraitChanges::default()
        },
    );
}

/// Commander initialize (`monster_tank_commander initialize`).
fn tank_commander_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    rerelease_tank_initialize(context);
    let entity = context.game.require_entity_mut(&actor);
    entity.skin = 2;
    entity.count = 1;
}

/// Attack (`attack`).
fn rerelease_tank_attack(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let enemy_id = context.entity().enemy.clone();
    if health(&mut *context.game, enemy_id.as_ref()) <= 0.0 {
        context.state_mut().brutal = false;
        context.set_move("tank_move_attack_strike", true);
        return;
    }
    if context.state().attack_state == MonsterAttackState::Blind {
        let delay = context.state().blind_fire_delay;
        let chance = if delay < 1.0 {
            1.0
        } else if delay < 7.5 {
            0.4
        } else {
            0.1
        };
        let choice = context.game.random();
        context.state_mut().blind_fire_delay = delay + 5.2 + context.game.random() * 3.0;
        if length3(context.state().blind_fire_target) == 0.0 || choice > chance {
            return;
        }
        let rocket = clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 23));
        let blaster = clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 1));
        if !rocket && !blaster {
            return;
        }
        context.state_mut().manual_steering = true;
        let fire_rocket = if rocket && blaster {
            context.game.random() < 0.5
        } else {
            rocket
        };
        if fire_rocket {
            context.set_move("tank_move_attack_fire_rocket", true);
        } else {
            context.set_move("tank_move_attack_blast", true);
            context.state_mut().next_frame = tank_frame::ATTAK108;
        }
        let now = context.game.host.now();
        let attack_finished = now + 3.0 + context.game.random() * 2.0;
        context.state_mut().attack_finished = attack_finished;
        context.state_mut().pain_time = now + 5.0;
        return;
    }
    let range = length3(sub3(enemy.origin, context.game.body_of(actor).origin));
    let choice = context.game.random();
    if range <= 250.0 {
        let enemy_id = context.entity().enemy.clone();
        let tesla = enemy_id
            .as_ref()
            .and_then(|id| context.game.entity(id))
            .is_some_and(|target| target.classname == "tesla_mine");
        let machinegun = !tesla && clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 8));
        if machinegun && choice < if range <= 125.0 { 0.5 } else { 0.25 } {
            context.set_move("tank_move_attack_chain", true);
            return;
        }
    } else {
        let machinegun = clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 8));
        let rocket = clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 23));
        if machinegun && choice < 0.33 {
            context.set_move("tank_move_attack_chain", true);
            return;
        }
        if rocket && choice < 0.66 {
            let now = context.game.host.now();
            context.state_mut().pain_time = now + 5.0;
            context.set_move("tank_move_attack_pre_rocket", true);
            return;
        }
    }
    if clear_shot(context, muzzle_offset(Q2Edition::Rerelease, 1)) {
        context.set_move("tank_move_attack_blast", true);
    }
}

/// Pain (`pain`).
fn rerelease_tank_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    let actor = context.actor().clone();
    let max_health = context.game.require_entity(&actor).max_health;
    let skin = context.game.require_entity(&actor).skin;
    let bloodied = health(&mut *context.game, Some(&actor)) < max_health / 2.0;
    context.game.require_entity_mut(&actor).skin = if bloodied { skin | 1 } else { skin & !1 };
    let damage = reaction.damage;
    let chained = chainfist(context);
    if !chained && damage <= 10.0 || context.game.host.now() < context.state().pain_time {
        return;
    }
    if !chainfist(context) {
        if damage <= 30.0 && context.game.random() > 0.2 {
            return;
        }
        let frame = context.game.require_entity(&actor).frame;
        if (frame >= tank_frame::ATTAK301 && frame <= tank_frame::ATTAK330)
            || (frame >= tank_frame::ATTAK101 && frame <= tank_frame::ATTAK116)
        {
            return;
        }
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    let count = context.game.require_entity(&actor).count;
    context.game.sound(
        &actor,
        if count != 0 {
            "tank/pain.wav"
        } else {
            "tank/tnkpain2.wav"
        },
        2,
        1.0,
        1.0,
    );
    if !reacts_to_pain(context) {
        return;
    }
    context.state_mut().manual_steering = false;
    context.set_move(
        if damage <= 30.0 {
            "tank_move_pain1"
        } else if damage <= 60.0 {
            "tank_move_pain2"
        } else {
            "tank_move_pain3"
        },
        true,
    );
}

/// Die (`die`).
fn rerelease_tank_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    if check_gib(context) {
        context.game.sound(&actor, "misc/udeath.wav", 2, 1.0, 1.0);
        let entity = context.game.require_entity_mut(&actor);
        entity.skin /= 2;
        let damage = reaction.pain.damage;
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/objects/gibs/sm_meat/tris.md2",
            damage,
            Q2GibOptions::default(),
        );
        for _ in 0..3 {
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
                ..Q2GibOptions::default()
            },
        );
        for part in ["foot", "thigh"] {
            for _ in 0..2 {
                throw_gib(
                    actor.clone(),
                    &mut *context.game,
                    &format!("models/monsters/tank/gibs/{part}.md2"),
                    damage,
                    Q2GibOptions {
                        skinned: true,
                        metallic: true,
                        ..Q2GibOptions::default()
                    },
                );
            }
        }
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/tank/gibs/chest.md2",
            damage,
            Q2GibOptions {
                skinned: true,
                ..Q2GibOptions::default()
            },
        );
        throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/tank/gibs/head.md2",
            damage,
            Q2GibOptions {
                skinned: true,
                head: true,
                ..Q2GibOptions::default()
            },
        );
        if context.game.require_entity(&actor).style == 0 {
            throw_gib(
                actor.clone(),
                &mut *context.game,
                "models/monsters/tank/gibs/barm.md2",
                damage,
                Q2GibOptions {
                    skinned: true,
                    upright: true,
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
    if context.game.require_entity(&actor).style == 0 {
        context.game.require_entity_mut(&actor).style = 1;
        let body = context.game.body_of(actor.clone());
        let axes = angles_vectors(body.angles);
        let arm = throw_gib(
            actor.clone(),
            &mut *context.game,
            "models/monsters/tank/gibs/barm.md2",
            reaction.pain.damage,
            Q2GibOptions {
                skinned: true,
                upright: true,
                ..Q2GibOptions::default()
            },
        );
        if let Some(arm) = arm {
            let spin = vec3(
                (context.game.random() * 2.0 - 1.0) as f32 * 15.0,
                (context.game.random() * 2.0 - 1.0) as f32 * 15.0,
                180.0,
            );
            let arm_entity = context.game.require_entity_mut(&arm);
            arm_entity.angular_velocity = spin;
            arm_entity.skin /= 2;
            let mut moved = context.game.body_of(arm.clone());
            moved.origin = add3(body.origin, add3(scale3(axes.right, -16.0), scale3(axes.up, 23.0)));
            moved.velocity = add3(scale3(axes.up, 100.0), scale3(axes.right, -120.0));
            moved.angles = vec3(body.angles.x, body.angles.y, -90.0);
            context.game.write_body(arm, &moved, true);
        }
    }
    context.game.sound(&actor, "tank/death.wav", 2, 1.0, 1.0);
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
    context.set_move("tank_move_death", true);
}

/// Blind check (`tank_blind_check`).
fn tank_blind_check(context: &mut MonsterContext) {
    if context.state().manual_steering {
        let actor = context.actor().clone();
        let target = context.state().blind_fire_target;
        let origin = context.game.body_of(actor).origin;
        context.state_mut().ideal_yaw = vector_angles(sub3(target, origin)).y.into();
    }
}

/// Dead (`tank_dead`).
fn tank_dead(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    corpse(context);
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.min = vec3(-16.0, -16.0, -16.0);
    moved.bounds.max = vec3(16.0, 16.0, 0.0);
    context.game.write_body(actor, &moved, true);
}

/// Shrink (`tank_shrink`).
fn tank_shrink(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.require_entity_mut(&actor).server_flags |= 2;
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds.max.z = 0.0;
    context.game.write_body(actor, &moved, true);
}

/// Reattack blaster (`tank_reattack_blaster`).
fn tank_reattack_blaster(context: &mut MonsterContext) {
    if context.state().manual_steering {
        context.state_mut().manual_steering = false;
        context.set_move("tank_move_attack_post_blast", true);
        return;
    }
    let enemy = context.entity().enemy.clone();
    let again =
        visible(context, None) && health(&mut *context.game, enemy.as_ref()) > 0.0 && context.game.random() <= 0.6;
    context.set_move(
        if again {
            "tank_move_reattack_blast"
        } else {
            "tank_move_attack_post_blast"
        },
        true,
    );
}

/// Refire rocket (`tank_refire_rocket`).
fn tank_refire_rocket(context: &mut MonsterContext) {
    if context.state().manual_steering {
        context.state_mut().manual_steering = false;
        context.set_move("tank_move_attack_post_rocket", true);
        return;
    }
    let enemy = context.entity().enemy.clone();
    let again =
        health(&mut *context.game, enemy.as_ref()) > 0.0 && visible(context, None) && context.game.random() <= 0.4;
    context.set_move(
        if again {
            "tank_move_attack_fire_rocket"
        } else {
            "tank_move_attack_post_rocket"
        },
        true,
    );
}

/// Blaster (`TankBlaster`).
fn tank_blaster(context: &mut MonsterContext) {
    if enemy_body(context).is_none() {
        return;
    }
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let id = if frame == tank_frame::ATTAK110 {
        1
    } else if frame == tank_frame::ATTAK113 {
        2
    } else {
        3
    };
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, id as usize), None);
    let direction = if context.state().manual_steering {
        let target = context.state().blind_fire_target;
        let right = angles_vectors(context.game.body_of(actor.clone()).angles).right;
        tank_blind_aim(context, start, target, right)
    } else {
        predicted_direction(context, start, 0.0, false, 0.0)
    };
    let Some(direction) = direction else {
        return;
    };
    let fire_blaster = context.weapons.fire_blaster;
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
    monster_flash(context, id, start, direction);
}

/// Rocket (`TankRocket`).
fn tank_rocket(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let frame = context.game.require_entity(&actor).frame;
    let id = if frame == tank_frame::ATTAK324 {
        23
    } else if frame == tank_frame::ATTAK327 {
        24
    } else {
        25
    };
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, id as usize), None);
    let heat = context.game.require_entity(&actor).spawnflags & 16 != 0;
    let authored_speed = context.game.require_entity(&actor).speed;
    let speed = if authored_speed != 0.0 {
        authored_speed
    } else if heat {
        500.0
    } else {
        650.0
    };
    let manual = context.state().manual_steering;
    let mut target = if manual {
        context.state().blind_fire_target
    } else if context.game.random() < 0.66 || start.z < enemy.origin.z + enemy.bounds.min.z {
        enemy_eye(context).unwrap_or(enemy.origin)
    } else {
        vec3(
            enemy.origin.x,
            enemy.origin.y,
            enemy.origin.z + enemy.bounds.min.z + 1.0,
        )
    };
    if !manual {
        let skill = f64::from(context.game.options.skill);
        if context.game.random() < 0.2 + (3.0 - skill) * 0.15 {
            if let Some(aim) = predict_aim(context, start, speed, false, 0.0) {
                target = aim.point;
            }
        }
    }
    let direction = if manual {
        let right = angles_vectors(context.game.body_of(actor.clone()).angles).right;
        tank_blind_aim(context, start, target, right)
    } else {
        Some(normalize3(sub3(target, start)))
    };
    let Some(direction) = direction else {
        return;
    };
    if !manual {
        let trace = context.game.host.trace(&Q2TraceRequest {
            start,
            end: target,
            bounds: None,
            ignore: Some(actor.clone()),
            mask: 0x4600_4003,
            exclude: Vec::new(),
        });
        if trace.fraction <= 0.5 && matches!(trace.hit, TraceHit::World { .. }) {
            return;
        }
    }
    if heat {
        let authored_accel = context.game.require_entity(&actor).accel;
        let weapons = mission_weapons(&*context.game);
        weapons.fire_heat_rocket(
            actor,
            &mut *context.game,
            start,
            direction,
            50.0,
            speed,
            70.0,
            50.0,
            Some(if authored_accel != 0.0 { authored_accel } else { 0.075 }),
        );
    } else {
        let fire_rocket = context.weapons.fire_rocket;
        fire_rocket(actor, &mut *context.game, start, direction, 50.0, speed, 70.0, 50.0);
    }
    monster_flash(context, id, start, direction);
}

/// Machine gun (`TankMachineGun`).
fn tank_machine_gun(context: &mut MonsterContext) {
    let Some(eye) = enemy_eye(context) else {
        return;
    };
    let actor = context.actor().clone();
    let frame = context.game.require_entity(&actor).frame;
    let id = 4 + frame - tank_frame::ATTAK406;
    let start = project_flash(context, muzzle_offset(Q2Edition::Rerelease, id as usize), None);
    let angles = context.game.body_of(actor.clone()).angles;
    let pitch = vector_angles(sub3(eye, start)).x;
    let yaw = if frame <= tank_frame::ATTAK415 {
        f64::from(angles.y) - 8.0 * f64::from(frame - tank_frame::ATTAK411)
    } else {
        f64::from(angles.y) + 8.0 * f64::from(frame - tank_frame::ATTAK419)
    };
    let direction = angles_vectors(vec3(pitch, yaw as f32, 0.0)).forward;
    let fire_bullet = context.weapons.fire_bullet;
    fire_bullet(actor, &mut *context.game, start, direction, 20.0, 4.0, 300.0, 500.0, 0);
    monster_flash(context, id, start, direction);
}

/// Create the rerelease tank definitions (`createRereleaseTankDefinitions`).
pub fn create_rerelease_tank_definitions() -> Vec<Q2MonsterDefinition> {
    let mut definition = tank_definition();
    definition.moves = tank_moves();
    definition.blind_fire = true;
    definition.initialize = Some(MonsterHandler::Callback(rerelease_tank_initialize));
    definition.blocked = Some(blocked_check_platform);
    definition.attack = MonsterHandler::Callback(rerelease_tank_attack);
    definition.pain = Some(rerelease_tank_pain);
    definition.die = rerelease_tank_die;
    for (name, handler) in [
        ("tank_blind_check", MonsterHandler::Callback(tank_blind_check)),
        ("tank_dead", MonsterHandler::Callback(tank_dead)),
        ("tank_shrink", MonsterHandler::Callback(tank_shrink)),
        ("tank_reattack_blaster", MonsterHandler::Callback(tank_reattack_blaster)),
        ("tank_refire_rocket", MonsterHandler::Callback(tank_refire_rocket)),
        ("TankBlaster", MonsterHandler::Callback(tank_blaster)),
        ("TankRocket", MonsterHandler::Callback(tank_rocket)),
        ("TankMachineGun", MonsterHandler::Callback(tank_machine_gun)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    let mut commander = definition.clone();
    commander.classname = "monster_tank_commander".to_string();
    commander.health = 1000.0;
    commander.gib_health = -225.0;
    commander.initialize = Some(MonsterHandler::Callback(tank_commander_initialize));
    vec![definition, commander]
}

/// Tank stand think (`thinkTankStand`).
fn think_tank_stand(actor: ActorId, game: &mut crate::q2::foundation::host::Q2GameServices) {
    let frame = game.require_entity(&actor).frame;
    game.require_entity_mut(&actor).frame = if frame == tank_frame::STAND30 {
        tank_frame::STAND01
    } else {
        frame + 1
    };
    game.show(actor.clone());
    game.schedule(actor, 0.1, think_tank_stand);
}

/// Tank stand use (`useTankStand`).
fn use_tank_stand(
    actor: ActorId,
    game: &mut crate::q2::foundation::host::Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let origin = game.body_of(actor.clone()).origin;
    game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:boss-teleport".to_string(),
        origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    game.remove_actor(actor);
}

/// Tank stand spawn (`rereleaseTankStandModule spawn`).
fn spawn_tank_stand(actor: ActorId, game: &mut crate::q2::foundation::host::Q2GameServices) -> bool {
    if game.require_entity(&actor).classname != "monster_tank_stand" {
        return false;
    }
    if game.options.mode == Q2Mode::Deathmatch {
        game.remove_actor(actor);
        return true;
    }
    let spawn = game.require_entity(&actor).spawn.clone();
    let authored = number_field(&spawn, "scale", 0.0);
    let scale = if authored != 0.0 { authored } else { 1.5 };
    let entity = game.require_entity_mut(&actor);
    entity.scale = scale;
    entity.model = "models/monsters/tank/tris.md2".to_string();
    entity.frame = tank_frame::STAND01;
    entity.skin = 2;
    let mut moved = game.body_of(actor.clone());
    moved.bounds.min = scale3(vec3(-32.0, -32.0, -16.0), scale as f32);
    moved.bounds.max = scale3(vec3(32.0, 32.0, 64.0), scale as f32);
    game.write_body(actor.clone(), &moved, true);
    game.set_motion_kind(actor.clone(), Q2MotionKind::Step);
    game.set_solid(actor.clone(), crate::q2::foundation::host::Q2Solid::Box);
    game.show(actor.clone());
    game.require_entity_mut(&actor).use_ = Some(use_tank_stand);
    game.schedule(actor, 0.1, think_tank_stand);
    true
}

/// Tank stand item name (unused).
fn tank_stand_item_name(_classname: &str) -> Option<String> {
    None
}

/// Create the rerelease tank stand module (`rereleaseTankStandModule`).
pub fn rerelease_tank_stand_module() -> SpawnModule {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks
        .think
        .insert("rerelease.tank_stand.Think_TankStand", think_tank_stand);
    callbacks.use_.insert("rerelease.tank_stand.Use_Boss3", use_tank_stand);
    let spawn: Q2SpawnFn = spawn_tank_stand;
    SpawnModule {
        spawn,
        item_name: tank_stand_item_name,
        callbacks,
    }
}
