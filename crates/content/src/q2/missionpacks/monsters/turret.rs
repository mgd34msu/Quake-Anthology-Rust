//! Rogue turret (`src/content/q2/missionpacks/monsters/turret.ts`).
//!
//! Quake II rogue/m_turret.c. ZeniMax Media, GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, add3, dot3, length3, normalize3, scale3, sub3, vec3};

use super::state::rogue_state;
use super::tables::rogue_turret::{turret_frame, turret_moves};
use super::types::mission_services;
use crate::q2::base::monsters::common::move_handler;
use crate::q2::foundation::callbacks::{Q2CallbackDefinitions, free_q2_entity};
use crate::q2::foundation::host::{
    Q2EffectEvent, Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid,
    Q2TraceRequest,
};
use crate::q2::foundation::monsters::ai::{
    angles_vectors, attack_trace_mask, enemy_body, enemy_eye, health,
    target_distance, vector_angles, visible,
};
use crate::q2::foundation::monsters::types::{
    MonsterAttackState, MonsterContext, MonsterHandler, MonsterLocomotion,
    Q2MonsterDefinition, StartMode,
};
use crate::q2::foundation::weapons::types::Mod;
use crate::q2::rerelease::monsters::common::monster_flash;
use crate::q2::support::contracts::{
    CombatTraitChanges, DeathReaction, PainReaction, TraceHit, TraceResult,
};

/// Angle mod (`anglemod`).
fn angle_mod(angle: f64) -> f64 {
    ((angle * 65536.0 / 360.0).trunc() as i64 & 65535) as f64 * 360.0 / 65536.0
}

/// Clamp (`clamp`).
fn clamp_f32(value: f32, low: f32, high: f32) -> f32 {
    value.max(low).min(high)
}

/// Target or world (`targetOrWorld`).
fn target_or_world(context: &mut MonsterContext, trace: &TraceResult) -> bool {
    match &trace.hit {
        TraceHit::None => true,
        TraceHit::World { .. } => true,
        TraceHit::Actor { actor } => {
            *actor == context.game.host.world_actor()
                || context.entity().enemy.as_ref() == Some(actor)
        }
    }
}

/// Ready (`ready`).
fn turret_ready(context: &mut MonsterContext) {
    context.set_move("turret_move_ready_gun", false);
}

/// Run (`run`).
fn turret_run(context: &mut MonsterContext) {
    if context.entity().frame < turret_frame::RUN01 {
        turret_ready(context);
    } else {
        context.set_move("turret_move_run", false);
    }
}

/// Walk (`walk`).
fn turret_walk(context: &mut MonsterContext) {
    if context.entity().frame < turret_frame::RUN01 {
        turret_ready(context);
    } else {
        context.set_move("turret_move_seek", false);
    }
}

/// Aim (`aim`).
fn turret_aim(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    if enemy.is_none()
        || enemy.as_ref().is_some_and(|enemy| *enemy == context.game.host.world_actor())
    {
        if !context.find_target() {
            return;
        }
    }
    if context.entity().frame < turret_frame::ACTIVE01 {
        turret_ready(context);
        return;
    }
    if context.entity().frame < turret_frame::RUN01 {
        return;
    }
    let enemy_id = context.entity().enemy.clone();
    let Some(enemy_body) = enemy_body(context) else {
        return;
    };
    let Some(enemy_id) = enemy_id else {
        return;
    };
    let mut end = enemy_body.origin;
    let view_height = context
        .game
        .entity(&enemy_id)
        .map(|enemy| enemy.view_height as f32)
        .unwrap_or(22.0);
    if context.state().current_move.name == "turret_move_fire_blind" {
        let target = context.state().blind_fire_target;
        end = vec3(
            target.x,
            target.y,
            target.z
                + if enemy_body.origin.z < target.z {
                    view_height + 10.0
                } else {
                    enemy_body.bounds.min.z - 10.0
                },
        );
    } else if context.game.host.is_player(&enemy_id) {
        end.z += view_height;
    }
    let body = context.game.body_of(actor.clone());
    let ideal = vector_angles(sub3(end, body.origin));
    let mut pitch = ideal.x;
    let mut yaw = ideal.y;
    let orientation = rogue_state(&mut *context.game, &actor).turret_orientation.trunc() as i32;
    match orientation {
        -1 => {
            if pitch < -90.0 {
                pitch += 360.0;
            }
            pitch = pitch.min(-5.0);
        }
        -2 => {
            if pitch > -90.0 {
                pitch -= 360.0;
            }
            pitch = clamp_f32(pitch, -355.0, -185.0);
        }
        0 => {
            if pitch < -180.0 {
                pitch += 360.0;
            }
            pitch = clamp_f32(pitch, -85.0, 85.0);
            if yaw > 180.0 {
                yaw -= 360.0;
            }
            yaw = clamp_f32(yaw, -85.0, 85.0);
        }
        90 => {
            if pitch < -180.0 {
                pitch += 360.0;
            }
            pitch = clamp_f32(pitch, -85.0, 85.0);
            if yaw > 270.0 {
                yaw -= 360.0;
            }
            yaw = clamp_f32(yaw, 5.0, 175.0);
        }
        180 => {
            if pitch < -180.0 {
                pitch += 360.0;
            }
            pitch = clamp_f32(pitch, -85.0, 85.0);
            yaw = clamp_f32(yaw, 95.0, 265.0);
        }
        270 => {
            if pitch < -180.0 {
                pitch += 360.0;
            }
            pitch = clamp_f32(pitch, -85.0, 85.0);
            if yaw < 90.0 {
                yaw += 360.0;
            }
            yaw = clamp_f32(yaw, 185.0, 355.0);
        }
        _ => {}
    }
    let mut pitch_move = pitch - body.angles.x;
    while pitch_move >= 360.0 {
        pitch_move -= 360.0;
    }
    if pitch_move >= 90.0 {
        pitch_move -= 360.0;
    }
    while pitch_move <= -360.0 {
        pitch_move += 360.0;
    }
    if pitch_move <= -90.0 {
        pitch_move += 360.0;
    }
    let mut yaw_move = yaw - body.angles.y;
    if yaw_move >= 180.0 {
        yaw_move -= 360.0;
    }
    if yaw_move <= -180.0 {
        yaw_move += 360.0;
    }
    let yaw_speed = context.state().yaw_speed as f32;
    let angles = body.angles;
    let mut moved = body;
    moved.angles.x = if pitch == angles.x {
        angles.x
    } else {
        angle_mod(f64::from(angles.x) + f64::from(clamp_f32(pitch_move, -yaw_speed, yaw_speed))) as f32
    };
    moved.angles.y = if yaw == angles.y {
        angles.y
    } else {
        angle_mod(f64::from(angles.y) + f64::from(clamp_f32(yaw_move, -yaw_speed, yaw_speed))) as f32
    };
    context.game.write_body(actor, &moved, true);
}

/// Rocket speed (`rocketSpeed`).
fn turret_rocket_speed(context: &mut MonsterContext) -> f64 {
    550.0
        + if context.game.options.skill == 2 {
            (200.0 * context.game.random()).trunc()
        } else if context.game.options.skill == 3 {
            (100.0 + 200.0 * context.game.random()).trunc()
        } else {
            0.0
        }
}

/// Fire (`fire`).
fn turret_fire_inner(context: &mut MonsterContext, blind: bool) {
    turret_aim(context);
    let actor = context.actor().clone();
    let enemy_id = context.entity().enemy.clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let Some(enemy_id) = enemy_id else {
        return;
    };
    let body = context.game.body_of(actor.clone());
    let start = body.origin;
    let target = if blind {
        context.state().blind_fire_target
    } else {
        enemy.origin
    };
    let direct = normalize3(sub3(target, start));
    if dot3(direct, angles_vectors(body.angles).forward) < 0.98 {
        return;
    }
    if !blind {
        // The source retains this draw after its fire-chance condition was removed.
        context.game.random();
    }
    let spawnflags = context.entity().spawnflags;
    let skill = context.game.options.skill;
    let speed = if spawnflags & 32 != 0 {
        turret_rocket_speed(context)
    } else if skill == 0 {
        600.0
    } else if skill == 1 {
        800.0
    } else {
        1000.0
    };
    if !blind && !visible(context, None) {
        return;
    }
    let view_height = context
        .game
        .entity(&enemy_id)
        .map(|enemy| enemy.view_height as f32)
        .unwrap_or(22.0);
    let mut end = if blind {
        let target = context.state().blind_fire_target;
        vec3(
            target.x,
            target.y,
            target.z
                + if enemy.origin.z < target.z {
                    view_height + 10.0
                } else {
                    enemy.bounds.min.z - 10.0
                },
        )
    } else {
        vec3(
            enemy.origin.x,
            enemy.origin.y,
            enemy.origin.z
                + if context.game.host.is_player(&enemy_id) {
                    view_height
                } else {
                    22.0
                },
        )
    };
    let distance = length3(sub3(end, start));
    if !blind
        && spawnflags & 80 == 0
        && distance < 512.0
        && context.game.random() + f64::from(3 - skill) * 0.1 < 0.8
    {
        end = add3(end, scale3(enemy.velocity, distance / 1000.0));
    }
    let direction = normalize3(sub3(end, start));
    let trace = if blind {
        None
    } else {
        let mask = attack_trace_mask(&*context.game);
        Some(context.game.host.trace(&Q2TraceRequest {
            start,
            end,
            bounds: None,
            ignore: Some(actor.clone()),
            mask,
            exclude: Vec::new(),
        }))
    };
    if let Some(trace) = &trace {
        if !target_or_world(context, trace) {
            return;
        }
    }
    if spawnflags & 8 != 0 {
        let fire_blaster = context.weapons.fire_blaster;
        fire_blaster(
            actor,
            &mut *context.game,
            start,
            direction,
            20.0,
            if blind { 1000.0 } else { speed },
            8,
            false,
            Mod::BLASTER,
        );
        monster_flash(context, 143, start, direction);
    } else if !blind && spawnflags & 16 != 0 {
        let fire_bullet = context.weapons.fire_bullet;
        fire_bullet(
            actor,
            &mut *context.game,
            start,
            direction,
            4.0,
            0.0,
            300.0,
            500.0,
            0,
        );
        monster_flash(context, 141, start, direction);
    } else if spawnflags & 32 != 0
        && trace.as_ref().is_none_or(|trace| f64::from(distance) * trace.fraction > 72.0)
    {
        let fire_rocket = context.weapons.fire_rocket;
        fire_rocket(
            actor,
            &mut *context.game,
            start,
            direction,
            50.0,
            speed,
            70.0,
            50.0,
        );
        monster_flash(context, 142, start, direction);
    }
}

/// Wake (`wake`).
fn turret_wake(actor: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&actor).flags & 1024 != 0 {
        return;
    }
    if !game.monsters.states.contains_key(&actor) {
        panic!("Turret wall wake has no source monster context");
    }
    if let Some(state) = game.monsters.states.get_mut(&actor) {
        state.can_take_damage = true;
    }
    let owned = game.owned_of(actor.clone());
    game.set_combat_traits(
        &owned,
        &CombatTraitChanges {
            can_take_damage: Some(true),
            ..CombatTraitChanges::default()
        },
    );
    game.set_motion_kind(actor.clone(), Q2MotionKind::Stationary);
    let mut context = MonsterContext::new(actor.clone(), game);
    context.set_move("turret_move_stand", false);
    let game = &mut *context.game;
    // stationarymonster_start consumes its initial random frame but must not count a second time.
    let frame = (game.random() * 2.0).floor() as i32;
    game.require_entity_mut(&actor).frame = frame;
    game.link_actor(actor.clone());
    game.show(actor.clone());
    game.require_entity_mut(&actor).use_ = game.source_callbacks.resolve_use(Some("monster_use"));
    let Some(start) = game.source_callbacks.resolve_think(Some("monster_start_go")) else {
        panic!("Turret wake has no source stationarymonster_start callback");
    };
    game.schedule(actor, 0.1, start);
}

/// Activate (`activate`).
fn turret_activate(
    actor: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    game.set_motion_kind(actor.clone(), Q2MotionKind::Push);
    {
        let entity = game.require_entity_mut(&actor);
        if entity.speed == 0.0 {
            entity.speed = 15.0;
        }
        entity.accel = entity.speed;
        entity.decel = entity.speed;
    }
    let body = game.body_of(actor.clone());
    let forward = if body.angles.x == 270.0 {
        vec3(0.0, 0.0, 1.0)
    } else if body.angles.x == 90.0 {
        vec3(0.0, 0.0, -1.0)
    } else if body.angles.y == 0.0 {
        vec3(1.0, 0.0, 0.0)
    } else if body.angles.y == 90.0 {
        vec3(0.0, 1.0, 0.0)
    } else if body.angles.y == 180.0 {
        vec3(-1.0, 0.0, 0.0)
    } else if body.angles.y == 270.0 {
        vec3(0.0, -1.0, 0.0)
    } else {
        vec3(0.0, 0.0, 0.0)
    };
    let destination = add3(body.origin, scale3(forward, 32.0));
    let services = mission_services(game);
    services.move_linear(&actor, game, destination, turret_wake);
    let team_chain = game.require_entity(&actor).team_chain.clone();
    if let Some(base) = team_chain {
        let speed = game.require_entity(&actor).speed;
        game.set_motion_kind(base.clone(), Q2MotionKind::Push);
        {
            let entity = game.require_entity_mut(&base);
            entity.speed = speed;
            entity.accel = speed;
            entity.decel = speed;
        }
        let origin = game.body_of(base.clone()).origin;
        let services = mission_services(game);
        let destination = add3(origin, scale3(forward, 32.0));
        services.move_linear(&base, game, destination, turret_wake);
    }
    game.sound(&actor, "world/dr_short.wav", 2, 1.0, 1.0);
}

/// Start mode (`startMode`).
fn turret_start_mode(context: &mut MonsterContext) -> StartMode {
    if context.entity().spawnflags & 128 != 0 {
        StartMode::Manual
    } else {
        StartMode::Automatic
    }
}

/// Initialize (`initialize`).
fn turret_initialize(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.entity_mut().flags |= 8192;
    context.entity_mut().gravity = 0.0;
    context.state_mut().manual_steering = true;
    context.state_mut().ignore_shots = true;
    if context.entity().spawnflags & 120 == 0 {
        context.entity_mut().spawnflags |= 8;
    }
    if context.entity().spawnflags & 64 != 0 {
        let spawnflags = context.entity().spawnflags;
        context.entity_mut().spawnflags = spawnflags & !64 | 8;
    }
    let body = context.game.body_of(actor.clone());
    let yaw = f64::from(body.angles.y).trunc();
    rogue_state(&mut *context.game, &actor).turret_orientation = yaw;
    let mut moved = body;
    match yaw as i32 {
        -1 => {
            moved.angles.x = 270.0;
            moved.angles.y = 0.0;
            moved.origin.z += 2.0;
        }
        -2 => {
            moved.angles.x = 90.0;
            moved.angles.y = 0.0;
            moved.origin.z -= 2.0;
        }
        0 => {
            moved.origin.x += 2.0;
        }
        90 => {
            moved.origin.y += 2.0;
        }
        180 => {
            moved.origin.x -= 2.0;
        }
        270 => {
            moved.origin.y -= 2.0;
        }
        _ => {}
    }
    context.game.write_body(actor.clone(), &moved, true);
    if context.entity().spawnflags & 128 != 0 && context.entity().targetname.is_empty() {
        context.game.remove_actor(actor);
    }
}

/// After spawn (`afterSpawn`).
fn turret_after_spawn(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    if context.entity().spawnflags & 128 != 0 {
        context.state_mut().can_take_damage = false;
        let owned = context.game.owned_of(actor.clone());
        context.game.set_combat_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: Some(false),
                ..CombatTraitChanges::default()
            },
        );
        context.entity_mut().use_ = Some(turret_activate);
        context.game.cancel_actor(actor.clone());
        let body = context.game.body_of(actor.clone());
        let base = context.game.create("turret_wall", BTreeMap::new());
        let orientation = if body.angles.x == 90.0 {
            -1
        } else if body.angles.x == 270.0 {
            -2
        } else {
            f64::from(body.angles.y).trunc() as i32
        };
        let bounds = match orientation {
            -1 => Bounds {
                min: vec3(-16.0, -16.0, -8.0),
                max: vec3(16.0, 16.0, 0.0),
            },
            -2 => Bounds {
                min: vec3(-16.0, -16.0, 0.0),
                max: vec3(16.0, 16.0, 8.0),
            },
            0 => Bounds {
                min: vec3(-8.0, -16.0, -16.0),
                max: vec3(0.0, 16.0, 16.0),
            },
            90 => Bounds {
                min: vec3(-16.0, -8.0, -16.0),
                max: vec3(16.0, 0.0, 16.0),
            },
            180 => Bounds {
                min: vec3(0.0, -16.0, -16.0),
                max: vec3(8.0, 16.0, 16.0),
            },
            270 => Bounds {
                min: vec3(-16.0, 0.0, -16.0),
                max: vec3(16.0, 8.0, 16.0),
            },
            _ => Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            },
        };
        let mut moved = context.game.body_of(base.clone());
        moved.origin = body.origin;
        moved.angles = body.angles;
        moved.bounds = bounds;
        context.game.write_body(base.clone(), &moved, false);
        context.game.require_entity_mut(&base).team_master = Some(actor.clone());
        context.entity_mut().team_master = Some(actor.clone());
        context.entity_mut().team_chain = Some(base.clone());
        context.game.require_entity_mut(&base).team_chain = None;
        {
            let entity = context.game.require_entity_mut(&base);
            entity.flags |= 1024;
            entity.owner = Some(actor.clone());
            entity.model = "models/monsters/turretbase/tris.md2".to_string();
        }
        context.game.set_motion_kind(base.clone(), Q2MotionKind::Push);
        context.game.set_solid(base.clone(), Q2Solid::None);
        context.game.link_actor(base.clone());
        context.game.show(base);
    }
    let spawnflags = context.entity().spawnflags;
    if spawnflags & 16 != 0 {
        context.entity_mut().skin = 1;
    } else if spawnflags & 32 != 0 {
        context.entity_mut().skin = 2;
    } else {
        context.entity_mut().spawnflags |= 8;
    }
    context.game.show(actor);
}

/// Attack (`attack`).
fn turret_attack(context: &mut MonsterContext) {
    if context.entity().frame < turret_frame::RUN01 {
        turret_ready(context);
        return;
    }
    if context.state().attack_state != MonsterAttackState::Blind {
        context.state_mut().next_frame = turret_frame::POW01;
        context.set_move("turret_move_fire", false);
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
    let random = context.game.random();
    let delay = delay + 3.4 + context.game.random() * 4.0;
    context.state_mut().blind_fire_delay = delay;
    if length3(context.state().blind_fire_target) == 0.0 || random > chance {
        return;
    }
    context.state_mut().next_frame = turret_frame::POW01;
    context.set_move("turret_move_fire_blind", false);
}

/// Check attack (`checkAttack`).
fn turret_check_attack(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    let enemy = context.entity().enemy.clone();
    let Some(eye) = enemy_eye(context) else {
        return false;
    };
    let Some(enemy_id) = enemy else {
        return false;
    };
    let origin = context.game.body_of(actor.clone()).origin;
    let start = vec3(origin.x, origin.y, origin.z + context.entity().view_height as f32);
    if health(&mut *context.game, Some(&enemy_id)) > 0.0 {
        let trace = context.game.host.trace(&Q2TraceRequest {
            start,
            end: eye,
            bounds: None,
            ignore: Some(actor.clone()),
            mask: 1 | 2 | 0x2000000 | 16 | 8,
            exclude: Vec::new(),
        });
        let enemy_solid = context.game.entity(&enemy_id).map(|enemy| enemy.solid);
        let direct = matches!(&trace.hit, TraceHit::Actor { actor } if *actor == enemy_id);
        if !direct && (enemy_solid != Some(Q2Solid::None) || trace.fraction < 1.0) {
            let blocker_monster = matches!(&trace.hit, TraceHit::Actor { actor } if context.game.host.is_monster(actor));
            let spawnflags = context.entity().spawnflags;
            if !blocker_monster
                && !visible(context, None)
                && spawnflags & 40 != 0
                && context.state().blind_fire_delay <= 10.0
                && context.game.host.now() >= context.state().attack_finished
                && context.game.host.now()
                    >= context.state().trail_time + context.state().blind_fire_delay
            {
                let target = context.state().blind_fire_target;
                let blind = context.game.host.trace(&Q2TraceRequest {
                    start,
                    end: target,
                    bounds: None,
                    ignore: Some(actor.clone()),
                    mask: 0x2000000,
                    exclude: Vec::new(),
                });
                if !blind.all_solid
                    && !blind.start_solid
                    && (blind.fraction == 1.0
                        || matches!(&blind.hit, TraceHit::Actor { actor } if *actor == enemy_id))
                {
                    context.state_mut().attack_state = MonsterAttackState::Blind;
                    let finished = context.game.host.now() + 0.5 + 2.0 * context.game.random();
                    context.state_mut().attack_finished = finished;
                    return true;
                }
            }
            return false;
        }
    }
    if context.game.host.now() < context.state().attack_finished {
        return false;
    }
    if target_distance(context) < 80.0 {
        if context.game.options.skill == 0 && (context.game.random() * 4.0).floor() as i32 != 0 {
            return false;
        }
        context.state_mut().attack_state = MonsterAttackState::Missile;
        return true;
    }
    let spawnflags = context.entity().spawnflags;
    let skill = context.game.options.skill;
    let mut chance = if spawnflags & 32 != 0 {
        0.1
    } else if spawnflags & 8 != 0 {
        0.35
    } else {
        0.5
    };
    let next = if spawnflags & 32 != 0 {
        1.8 - 0.2 * f64::from(skill)
    } else if spawnflags & 8 != 0 {
        1.2 - 0.2 * f64::from(skill)
    } else {
        0.8 - 0.1 * f64::from(skill)
    };
    chance *= if skill == 0 {
        0.5
    } else if skill > 1 {
        2.0
    } else {
        1.0
    };
    let enemy_solid_none = context
        .game
        .entity(&enemy_id)
        .is_some_and(|enemy| enemy.solid == Q2Solid::None);
    if context.game.random() < chance && visible(context, None) || enemy_solid_none {
        context.state_mut().attack_state = MonsterAttackState::Missile;
        let finished = context.game.host.now() + next;
        context.state_mut().attack_finished = finished;
        return true;
    }
    context.state_mut().attack_state = MonsterAttackState::Straight;
    false
}

/// Pain (`pain`).
fn turret_pain(_context: &mut MonsterContext, _reaction: &PainReaction) {}

/// Throw turret debris (`throwQ2Debris`).
///
/// This mirrors the foundation scenery helper until that module is ported.
fn throw_turret_debris(owner: &ActorId, game: &mut Q2GameServices, origin: Vec3, speed: f64) {
    let chunk = game.create("debris", BTreeMap::new());
    game.require_entity_mut(&chunk).model = "models/objects/debris1/tris.md2".to_string();
    let random_velocity = vec3(
        (100.0 * (game.random() * 2.0 - 1.0)) as f32,
        (100.0 * (game.random() * 2.0 - 1.0)) as f32,
        (100.0 + 100.0 * (game.random() * 2.0 - 1.0)) as f32,
    );
    let owner_velocity = game.body_of(owner.clone()).velocity;
    let mut moved = game.body_of(chunk.clone());
    moved.origin = origin;
    moved.velocity = add3(owner_velocity, scale3(random_velocity, speed as f32));
    game.write_body(chunk.clone(), &moved, false);
    game.require_entity_mut(&chunk).angular_velocity = vec3(
        (game.random() * 600.0) as f32,
        (game.random() * 600.0) as f32,
        (game.random() * 600.0) as f32,
    );
    let owned = game.owned_of(chunk.clone());
    game.create_combat(&owned, 0.0, 0.0, true);
    game.require_entity_mut(&chunk).die = Some(turret_debris_die);
    game.set_motion_kind(chunk.clone(), Q2MotionKind::Bounce);
    game.set_solid(chunk.clone(), Q2Solid::None);
    game.show(chunk.clone());
    let lifetime = 5.0 + game.random() * 5.0;
    game.schedule(chunk, lifetime, free_q2_entity);
}

/// Turret debris die (`freeQ2Entity` as `Q2Die`).
fn turret_debris_die(actor: ActorId, game: &mut Q2GameServices, _reaction: DeathReaction) {
    free_q2_entity(actor, game);
}

/// Die (`die`).
fn turret_die(context: &mut MonsterContext, _reaction: &DeathReaction) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    context.game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:plain-explosion".to_string(),
        origin: body.origin,
        direction: vec3(0.0, 0.0, 0.0),
        count: 1,
        color: 0,
    }));
    let start = add3(body.origin, angles_vectors(body.angles).forward);
    for speed in [1.0, 2.0, 1.0, 2.0] {
        throw_turret_debris(&actor, &mut *context.game, start, speed);
    }
    let team_chain = context.entity().team_chain.clone();
    if let Some(base) = team_chain {
        context.game.set_solid(base.clone(), Q2Solid::Box);
        if context.game.host.combat().read(&base).is_some() {
            let owned = context.game.owned_of(base.clone());
            context.game.set_combat_traits(
                &owned,
                &CombatTraitChanges {
                    can_take_damage: Some(false),
                    ..CombatTraitChanges::default()
                },
            );
        }
        context.game.set_motion_kind(base.clone(), Q2MotionKind::Stationary);
        context.game.link_actor(base);
    }
    // Stationary entities reach the source die callback before monster_death_use.
    if !context.entity().target.is_empty() {
        let enemy = context.entity().enemy.clone();
        let live = enemy.as_ref().is_some_and(|enemy| context.game.host.actors().is_live(enemy));
        let activator = if live { enemy } else { Some(actor.clone()) };
        let authored = context.game.require_entity(&actor).clone().authored_target();
        context.game.use_targets(&authored, activator.as_ref(), false);
    }
    context.game.remove_actor(actor);
}

/// Fire (`TurretFire`).
fn turret_fire(context: &mut MonsterContext) {
    turret_fire_inner(context, false);
}

/// Blind fire (`TurretFireBlind`).
fn turret_fire_blind(context: &mut MonsterContext) {
    turret_fire_inner(context, true);
}

/// Create the rogue turret definition (`createRogueTurretDefinition`).
pub fn create_rogue_turret_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_turret",
        "turret",
        "models/monsters/turret/tris.md2",
        240.0,
        -100.0,
        250.0,
        Bounds {
            min: vec3(-12.0, -12.0, -12.0),
            max: vec3(12.0, 12.0, 12.0),
        },
        1.0,
        "turret_move_stand",
        turret_moves(),
        move_handler("turret_move_stand"),
        MonsterHandler::Callback(turret_walk),
        MonsterHandler::Callback(turret_run),
        MonsterHandler::Callback(turret_attack),
        turret_die,
    );
    definition.yaw_speed = Some(45.0);
    definition.locomotion = Some(MonsterLocomotion::Stationary);
    definition.view_height = Some(0);
    definition.blind_fire = true;
    definition.start_mode = Some(turret_start_mode);
    definition.initialize = Some(MonsterHandler::Callback(turret_initialize));
    definition.after_spawn = Some(MonsterHandler::Callback(turret_after_spawn));
    let mut source_callbacks = Q2CallbackDefinitions::default();
    source_callbacks.think.insert("q2:rogue/turret_wake", turret_wake);
    source_callbacks.use_.insert("q2:rogue/turret_activate", turret_activate);
    definition.source_callbacks = Some(source_callbacks);
    definition.check_attack = Some(turret_check_attack);
    definition.pain = Some(turret_pain);
    definition.callbacks.insert(
        "turret_run".to_string(),
        MonsterHandler::Callback(turret_run),
    );
    definition.callbacks.insert(
        "TurretAim".to_string(),
        MonsterHandler::Callback(turret_aim),
    );
    definition.callbacks.insert(
        "TurretFire".to_string(),
        MonsterHandler::Callback(turret_fire),
    );
    definition.callbacks.insert(
        "TurretFireBlind".to_string(),
        MonsterHandler::Callback(turret_fire_blind),
    );
    definition
}
