//! Rogue stalker (`src/content/q2/missionpacks/monsters/stalker.ts`).
//!
//! Quake II rogue/m_stalker.c. ZeniMax Media, GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, length3, normalize3, scale3, sub3, vec3, Bounds, Vec3};

use super::rogue_common::rogue_blocked_check_shot;
use super::tables::rogue_stalker::{stalker_frame, stalker_moves};
use super::types::{mission_services, mission_weapons};
use crate::q2::base::monsters::common::{
    alive_enemy, begin_death, damaged_skin, finish_corpse, move_handler, sound_handler,
};
use crate::q2::foundation::host::Q2MotionKind;
use crate::q2::foundation::host::Q2TraceRequest;
use crate::q2::foundation::monsters::ai::{
    angles_vectors, attack_trace_mask, change_yaw, enemy_body, finish_dodge, health, monster_solid_mask, project_flash,
    vector_angles, visible,
};
use crate::q2::foundation::monsters::perception::found_target;
use crate::q2::foundation::monsters::types::{
    MonsterAttackState, MonsterContext, MonsterHandler, MonsterSpawner, Q2MonsterDefinition,
};
use crate::q2::rerelease::monsters::common::{
    blocked_check_jump, blocked_check_platform, monster_flash, monster_jump_finished, JumpNavigation, JumpResult,
};
use crate::q2::support::contracts::{DeathReaction, PainReaction, TraceContact, TraceFamily, TraceHit, TraceResult};

/// Ceiling (`ceiling`).
fn stalker_ceiling(context: &mut MonsterContext) -> bool {
    context.entity().gravity_vector.z > 0.0
}

/// Solid (`solid`).
fn trace_solid(trace: &TraceResult) -> bool {
    match &trace.family {
        TraceFamily::Q1 { .. } => false,
        TraceFamily::Q2(fields) => fields.contents & 1 != 0,
        TraceFamily::Q3 { contents, .. } => contents & 1 != 0,
    }
}

/// World (`world`).
fn trace_world(context: &mut MonsterContext, trace: &TraceResult) -> bool {
    match &trace.hit {
        TraceHit::None => true,
        TraceHit::World { .. } => true,
        TraceHit::Actor { actor } => *actor == context.game.host.world_actor(),
    }
}

/// Stalker transition check (`stalkerTransitionOK`).
pub fn stalker_transition_ok(context: &mut MonsterContext, spawned_by_widow: bool) -> bool {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let on_ceiling = stalker_ceiling(context);
    let margin = if on_ceiling {
        body.bounds.min.z - 8.0
    } else {
        body.bounds.max.z + 8.0
    };
    let end = vec3(
        body.origin.x,
        body.origin.y,
        body.origin.z
            + if on_ceiling {
                -384.0
            } else if spawned_by_widow {
                256.0
            } else {
                180.0
            },
    );
    let mask = monster_solid_mask(&*context.game);
    let trace = context.game.host.trace(&Q2TraceRequest {
        start: body.origin,
        end,
        bounds: Some(body.bounds),
        ignore: Some(actor.clone()),
        mask,
        exclude: Vec::new(),
    });
    if trace.fraction == 1.0 || !trace_solid(&trace) || !trace_world(context, &trace) {
        let normal_z = match &trace.contact {
            TraceContact::Plane { plane } => plane.normal.z,
            TraceContact::None => 0.0,
        };
        if on_ceiling {
            if normal_z < 0.9 {
                return false;
            }
        } else if normal_z > -0.9 {
            return false;
        }
    }
    let end_height = trace.end.z + margin;
    let corners = [
        (body.bounds.min.x, body.bounds.min.y),
        (body.bounds.max.x, body.bounds.min.y),
        (body.bounds.max.x, body.bounds.max.y),
        (body.bounds.min.x, body.bounds.max.y),
    ];
    for (cx, cy) in corners {
        let start = vec3(
            body.origin.x + cx - if cx < 0.0 { 1.0 } else { -1.0 },
            body.origin.y + cy - if cy < 0.0 { 1.0 } else { -1.0 },
            body.origin.z,
        );
        let corner_trace = context.game.host.trace(&Q2TraceRequest {
            start,
            end: vec3(start.x, start.y, end_height),
            bounds: None,
            ignore: Some(actor.clone()),
            mask,
            exclude: Vec::new(),
        });
        if corner_trace.fraction == 1.0
            || !trace_solid(&corner_trace)
            || !trace_world(context, &corner_trace)
            || (end_height - corner_trace.end.z).trunc().abs() > 8.0
        {
            return false;
        }
    }
    true
}

/// Stalker jump angles (`stalkerJumpAngles`).
///
/// The original solver deliberately uses FAUX_GRAVITY rather than sv_gravity.
pub fn stalker_jump_angles(start: Vec3, end: Vec3, velocity: f64) -> (f64, f64) {
    let delta = sub3(end, start);
    let horizontal = f64::hypot(f64::from(delta.x), f64::from(delta.y));
    let vertical = f64::from(delta.z).abs();
    let distance = f64::hypot(horizontal, vertical);
    let angle = if vertical == 0.0 {
        0.0
    } else {
        (vertical / horizontal).atan() * if delta.z > 0.0 { -1.0 } else { 1.0 }
    };
    let first = (distance * 800.0 * angle.cos().powi(2) / velocity.powi(2) - angle.sin()).asin();
    ((first - angle) / 2.0, (std::f64::consts::PI - first - angle) / 2.0)
}

/// Transition (`transition`).
fn stalker_transition(context: &mut MonsterContext) -> bool {
    let widow = context.state().spawned_by == MonsterSpawner::Widow;
    stalker_transition_ok(context, widow)
}

/// Stand (`stand`).
fn stalker_stand(context: &mut MonsterContext) {
    let idle = context.game.random() < 0.25;
    context.set_move(
        if idle {
            "stalker_move_stand"
        } else {
            "stalker_move_idle2"
        },
        false,
    );
}

/// Run (`run`).
fn stalker_run(context: &mut MonsterContext) {
    let stand_ground = context.state().stand_ground;
    context.set_move(
        if stand_ground {
            "stalker_move_stand"
        } else {
            "stalker_move_run"
        },
        false,
    );
}

/// Detach (`detach`).
fn stalker_detach(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let roll = body.angles.z + 180.0;
    context.entity_mut().gravity_vector.z = -1.0;
    let mut moved = body;
    moved.angles.z = if roll > 360.0 { roll - 360.0 } else { roll };
    moved.ground = None;
    context.game.write_body(actor, &moved, true);
}

/// Jump straight up (`jumpStraightUp`).
fn stalker_jump_straightup(context: &mut MonsterContext) {
    if context.state().dead {
        return;
    }
    if stalker_ceiling(context) {
        if stalker_transition(context) {
            stalker_detach(context);
        }
        return;
    }
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    if body.ground.is_none() {
        return;
    }
    let gravity_z = context.entity().gravity_vector.z;
    let mut moved = body;
    moved.velocity = vec3(
        moved.velocity.x + (context.game.random() * 10.0 - 5.0) as f32,
        moved.velocity.y + (context.game.random() * 10.0 - 5.0) as f32,
        moved.velocity.z - 400.0 * gravity_z,
    );
    context.game.write_body(actor.clone(), &moved, true);
    if stalker_transition(context) {
        context.entity_mut().gravity_vector.z = 1.0;
        let mut moved = context.game.body_of(actor.clone());
        moved.angles.z = 180.0;
        moved.ground = None;
        context.game.write_body(actor, &moved, true);
    }
}

/// Pounce (`pounce`).
fn stalker_pounce(context: &mut MonsterContext, destination: Vec3) -> bool {
    let actor = context.actor().clone();
    let enemy_id = context.entity().enemy.clone();
    let body = context.game.body_of(actor.clone());
    let Some(enemy) = enemy_body(context) else {
        return false;
    };
    let Some(enemy_id) = enemy_id else {
        return false;
    };
    if stalker_ceiling(context) || enemy.ground.is_none() || context.game.host.point_contents(destination) & 56 != 0 {
        return false;
    }
    let enemy_state = context.game.monsters.states.get(&enemy_id).cloned();
    let in_water = match enemy_state {
        Some(state) => state.water_level > 0,
        None => {
            context.game.host.point_contents(vec3(
                enemy.origin.x,
                enemy.origin.y,
                enemy.origin.z + enemy.bounds.min.z + 1.0,
            )) & 56
                != 0
        }
    };
    if in_water {
        return false;
    }
    // Original Rogue queries the target's local bounds, without adding its origin.
    for (cx, cy) in [
        (enemy.bounds.min.x, enemy.bounds.min.y),
        (enemy.bounds.max.x, enemy.bounds.min.y),
        (enemy.bounds.max.x, enemy.bounds.max.y),
        (enemy.bounds.min.x, enemy.bounds.max.y),
    ] {
        if context
            .game
            .host
            .point_contents(vec3(cx, cy, enemy.bounds.min.z - 0.25))
            & 3
            == 0
        {
            return false;
        }
    }
    let delta = sub3(destination, body.origin);
    let angles = vector_angles(delta);
    if (f64::from(angles.y) - f64::from(body.angles.y)).trunc().abs() > 45.0 {
        return false;
    }
    context.state_mut().ideal_yaw = f64::from(angles.y);
    change_yaw(context);
    if length3(delta) > 450.0 {
        return false;
    }
    let mut high = delta.z >= 32.0;
    let target = vec3(
        destination.x,
        destination.y,
        destination.z + if high { 32.0 } else { 0.0 },
    );
    let mask = monster_solid_mask(&*context.game);
    let trace = context.game.host.trace(&Q2TraceRequest {
        start: body.origin,
        end: destination,
        bounds: None,
        ignore: Some(actor.clone()),
        mask,
        exclude: Vec::new(),
    });
    if trace.fraction < 1.0 && !matches!(&trace.hit, TraceHit::Actor { actor } if *actor == enemy_id) {
        high = true;
    }
    let mut velocity = 400.1;
    let mut jump = (f64::NAN, f64::NAN);
    while velocity <= 800.0 {
        jump = stalker_jump_angles(body.origin, target, velocity);
        if !jump.0.is_nan() || !jump.1.is_nan() {
            break;
        }
        velocity += 200.0;
    }
    let chosen = if !high && !jump.0.is_nan() { jump.0 } else { jump.1 };
    if chosen.is_nan() {
        return false;
    }
    let body = context.game.body_of(actor.clone());
    let forward = normalize3(angles_vectors(body.angles).forward);
    let gravity = mission_services(&*context.game).gravity();
    let mut moved = body;
    let flat = scale3(forward, (velocity * chosen.cos()) as f32);
    moved.velocity = vec3(flat.x, flat.y, (velocity * chosen.sin() + 0.5 * gravity * 0.1) as f32);
    context.game.write_body(actor, &moved, true);
    true
}

/// Shoot (`shoot`).
fn stalker_shoot(context: &mut MonsterContext) {
    if !alive_enemy(context) {
        return;
    }
    let actor = context.actor().clone();
    let enemy_id = context.entity().enemy.clone();
    let Some(enemy) = enemy_body(context) else {
        return;
    };
    let Some(enemy_id) = enemy_id else {
        return;
    };
    let body = context.game.body_of(actor.clone());
    if body.ground.is_some() && context.game.random() < 0.33 {
        let distance = length3(sub3(enemy.origin, body.origin));
        if distance > 256.0 || context.game.random() < 0.5 {
            stalker_pounce(context, enemy.origin);
        } else {
            stalker_jump_straightup(context);
        }
    }
    let start = project_flash(context, vec3(24.0, 0.0, 6.0), None);
    let enemy = enemy_body(context).unwrap_or(enemy);
    let mut direction = sub3(enemy.origin, start);
    let mut end = enemy.origin;
    if context.game.random() < 0.2 + 0.1 * f64::from(context.game.options.skill) {
        end = add3(enemy.origin, scale3(enemy.velocity, length3(direction) / 1000.0));
        direction = sub3(end, start);
    }
    let mask = attack_trace_mask(&*context.game);
    let trace = context.game.host.trace(&Q2TraceRequest {
        start,
        end,
        bounds: None,
        ignore: Some(actor.clone()),
        mask,
        exclude: Vec::new(),
    });
    if trace_world(context, &trace) || matches!(&trace.hit, TraceHit::Actor { actor } if *actor == enemy_id) {
        let weapons = mission_weapons(&*context.game);
        weapons.fire_blaster2(actor, &mut *context.game, start, direction, 15.0, 800.0, 8);
        monster_flash(context, 144, start, direction);
    }
}

/// Reactivate (`reactivate`).
fn stalker_reactivate(context: &mut MonsterContext) {
    context.state_mut().stand_ground = false;
    context.set_move("stalker_move_false_death_end", true);
}

/// Jump impulse (`jumpImpulse`).
fn stalker_jump_impulse(context: &mut MonsterContext, forward_speed: f64, up_speed: f64) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let axes = angles_vectors(body.angles);
    let now = context.game.host.now();
    context.entity_mut().timestamp = now;
    let mut moved = body;
    moved.velocity = add3(
        add3(moved.velocity, scale3(axes.forward, forward_speed as f32)),
        scale3(axes.up, up_speed as f32),
    );
    context.game.write_body(actor, &moved, true);
}

/// Idle (`idle`).
fn stalker_idle(context: &mut MonsterContext) {
    let first = context.game.random() < 0.35;
    context.set_move(
        if first {
            "stalker_move_idle"
        } else {
            "stalker_move_idle2"
        },
        false,
    );
}

/// Initialize (`initialize`).
fn stalker_initialize(context: &mut MonsterContext) {
    if context.entity().spawnflags & 8 != 0 {
        context.entity_mut().gravity_vector.z = 1.0;
        let actor = context.actor().clone();
        let mut moved = context.game.body_of(actor.clone());
        moved.angles.z = 180.0;
        context.game.write_body(actor, &moved, true);
    }
}

/// Attack (`attack`).
fn stalker_attack(context: &mut MonsterContext) {
    if !alive_enemy(context) {
        return;
    }
    let skill = f64::from(context.game.options.skill);
    if context.game.random() > 1.0 - 0.5 / skill {
        context.state_mut().attack_state = MonsterAttackState::Straight;
    } else {
        if context.game.random() <= 0.5 {
            let lefty = !context.state().lefty;
            context.state_mut().lefty = lefty;
        }
        context.state_mut().attack_state = MonsterAttackState::Sliding;
    }
    context.set_move("stalker_move_shoot", true);
}

/// Melee (`melee`).
fn stalker_melee(context: &mut MonsterContext) {
    if !alive_enemy(context) {
        return;
    }
    let first = context.game.random() < 0.5;
    context.set_move(
        if first {
            "stalker_move_swing_l"
        } else {
            "stalker_move_swing_r"
        },
        false,
    );
}

/// Pain (`pain`).
fn stalker_pain(context: &mut MonsterContext, reaction: &PainReaction) {
    if context.state().dead {
        return;
    }
    damaged_skin(context);
    let actor = context.actor().clone();
    let current = context.state().current_move.name.clone();
    if context.game.options.skill == 3
        || context.game.body_of(actor.clone()).ground.is_none()
        || current == "stalker_move_false_death_end"
        || current == "stalker_move_false_death_start"
    {
        return;
    }
    if current == "stalker_move_false_death" {
        stalker_reactivate(context);
        return;
    }
    let hp = health(&mut *context.game, Some(&actor));
    let max_health = context.entity().max_health;
    if hp > 0.0
        && hp < max_health / 4.0
        && context.game.random() < 0.2 * f64::from(context.game.options.skill)
        && (!stalker_ceiling(context) || stalker_transition(context))
    {
        context.entity_mut().gravity_vector = vec3(0.0, 0.0, -1.0);
        let mut moved = context.game.body_of(actor);
        moved.angles.z = 0.0;
        // The donor re-reads the body after clearing gravity; the port reuses it.
        let actor = context.actor().clone();
        context.game.write_body(actor, &moved, true);
        context.state_mut().stand_ground = true;
        context.set_move("stalker_move_false_death_start", true);
        return;
    }
    if context.game.host.now() < context.state().pain_time {
        return;
    }
    let now = context.game.host.now();
    context.state_mut().pain_time = now + 3.0;
    if reaction.damage > 10.0 {
        let actor = context.actor().clone();
        let grounded = context.game.body_of(actor).ground.is_some();
        let jump = grounded && context.game.random() < 0.5;
        context.set_move(
            if jump {
                "stalker_move_jump_straightup"
            } else {
                "stalker_move_pain"
            },
            false,
        );
        let actor = context.actor().clone();
        context.game.sound(&actor, "stalker/pain.wav", 1, 1.0, 1.0);
    }
}

/// Die (`die`).
fn stalker_die(context: &mut MonsterContext, reaction: &DeathReaction) {
    let actor = context.actor().clone();
    context.game.set_motion_kind(actor.clone(), Q2MotionKind::Toss);
    let mut moved = context.game.body_of(actor.clone());
    moved.angles.z = 0.0;
    context.game.write_body(actor, &moved, true);
    context.entity_mut().gravity_vector = vec3(0.0, 0.0, -1.0);
    begin_death(context, reaction, "stalker/death.wav", "stalker_move_death", 2, 4);
}

/// Dodge (`dodge`).
fn stalker_dodge(
    context: &mut MonsterContext,
    attacker: &ActorId,
    eta: f64,
    _trace: Option<&TraceResult>,
    _direct: bool,
) {
    let actor = context.actor().clone();
    if context.game.body_of(actor.clone()).ground.is_none() || health(&mut *context.game, Some(&actor)) <= 0.0 {
        return;
    }
    if context.entity().enemy.is_none() {
        context.entity_mut().enemy = Some(attacker.clone());
        found_target(context);
        return;
    }
    if eta < 0.1 || eta > 5.0 {
        return;
    }
    context.set_move("stalker_move_jump_straightup", true);
}

/// Blocked (`blocked`).
fn stalker_blocked(context: &mut MonsterContext, distance: f64) -> bool {
    let Some(enemy) = enemy_body(context) else {
        return false;
    };
    if !alive_enemy(context) {
        return false;
    }
    let chance = 0.25 + 0.05 * f64::from(context.game.options.skill);
    if rogue_blocked_check_shot(context, chance) {
        return true;
    }
    if stalker_ceiling(context) {
        if !stalker_transition(context) {
            return false;
        }
        stalker_detach(context);
        return true;
    }
    if visible(context, None) {
        stalker_pounce(context, enemy.origin);
        return true;
    }
    if blocked_check_jump(context, distance, 256.0, 68.0, true, JumpNavigation::None) != JumpResult::None {
        let actor = context.actor().clone();
        let above = enemy.origin.z >= context.game.body_of(actor).origin.z;
        context.set_move(
            if above {
                "stalker_move_jump_up"
            } else {
                "stalker_move_jump_down"
            },
            false,
        );
        return true;
    }
    blocked_check_platform(context, distance)
}

/// Idle noise (`stalker_idle_noise`).
fn stalker_idle_noise(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    context.game.sound(&actor, "stalker/idle.wav", 1, 0.5, 2.0);
}

/// Heal (`stalker_heal`).
fn stalker_heal(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let skill = context.game.options.skill;
    let hp = health(&mut *context.game, Some(&actor))
        + f64::from(if skill == 2 {
            2
        } else if skill == 3 {
            3
        } else {
            1
        });
    let max_health = context.entity().max_health;
    if hp > max_health / 2.0 {
        context.entity_mut().skin = 0;
    }
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_health(&owned, hp.min(max_health));
    if hp >= max_health {
        stalker_reactivate(context);
    }
}

/// Second shoot attack (`stalker_shoot_attack2`).
fn stalker_shoot_attack2(context: &mut MonsterContext) {
    if context.game.random() < 0.4 + 0.1 * f64::from(context.game.options.skill) {
        stalker_shoot(context);
    }
}

/// Swing attack (`stalker_swing_attack`).
fn stalker_swing_attack(context: &mut MonsterContext) {
    let fire_hit = context.weapons.fire_hit;
    let actor = context.actor().clone();
    let damage = 5.0 + (context.game.random() * 5.0).floor();
    if fire_hit(actor.clone(), &mut *context.game, vec3(80.0, 0.0, 0.0), damage, 50.0) {
        let path = if context.entity().frame < stalker_frame::ATTACK08 {
            "stalker/melee2.wav"
        } else {
            "stalker/melee1.wav"
        };
        context.game.sound(&actor, path, 1, 1.0, 1.0);
    }
}

/// Jump wait land (`stalker_jump_wait_land`).
fn stalker_jump_wait_land(context: &mut MonsterContext) {
    let skill = context.game.options.skill;
    if context.game.random() < 0.3 + 0.1 * f64::from(skill)
        && context.game.host.now() >= context.state().attack_finished
    {
        let finished = context.game.host.now() + 0.3;
        context.state_mut().attack_finished = finished;
        stalker_shoot(context);
    }
    let actor = context.actor().clone();
    if context.game.body_of(actor.clone()).ground.is_none() {
        context.entity_mut().gravity = 1.3;
        let frame = context.entity().frame;
        context.state_mut().next_frame = frame;
        if monster_jump_finished(context) {
            context.entity_mut().gravity = 1.0;
            let frame = context.entity().frame;
            context.state_mut().next_frame = frame + 1;
        }
    } else {
        context.entity_mut().gravity = 1.0;
        let frame = context.entity().frame;
        context.state_mut().next_frame = frame + 1;
    }
}

/// Jump up (`stalker_jump_up`).
fn stalker_jump_up(context: &mut MonsterContext) {
    stalker_jump_impulse(context, 200.0, 450.0);
}

/// Jump down (`stalker_jump_down`).
fn stalker_jump_down(context: &mut MonsterContext) {
    stalker_jump_impulse(context, 100.0, 300.0);
}

/// Dead (`stalker_dead`).
fn stalker_dead(context: &mut MonsterContext) {
    finish_corpse(
        context,
        Bounds {
            min: vec3(-28.0, -28.0, -18.0),
            max: vec3(28.0, 28.0, -4.0),
        },
    );
}

/// Create the stalker definition (`createStalkerDefinition`).
pub fn create_stalker_definition() -> Q2MonsterDefinition {
    let mut definition = Q2MonsterDefinition::new(
        "monster_stalker",
        "stalker",
        "models/monsters/stalker/tris.md2",
        250.0,
        -50.0,
        250.0,
        Bounds {
            min: vec3(-28.0, -28.0, -18.0),
            max: vec3(28.0, 28.0, 18.0),
        },
        1.0,
        "stalker_move_stand",
        stalker_moves(),
        MonsterHandler::Callback(stalker_stand),
        move_handler("stalker_move_walk"),
        MonsterHandler::Callback(stalker_run),
        MonsterHandler::Callback(stalker_attack),
        stalker_die,
    );
    definition.initialize = Some(MonsterHandler::Callback(stalker_initialize));
    definition.idle = Some(MonsterHandler::Callback(stalker_idle));
    definition.sight = Some(sound_handler("stalker/sight.wav", 1, 1.0));
    definition.melee = Some(MonsterHandler::Callback(stalker_melee));
    definition.pain = Some(stalker_pain);
    definition.dodge = Some(stalker_dodge);
    definition.blocked = Some(stalker_blocked);
    for (name, handler) in [
        ("stalker_stand", MonsterHandler::Callback(stalker_stand)),
        ("stalker_walk", move_handler("stalker_move_walk")),
        ("stalker_run", MonsterHandler::Callback(stalker_run)),
        ("stalker_idle_noise", MonsterHandler::Callback(stalker_idle_noise)),
        ("stalker_false_death", move_handler("stalker_move_false_death")),
        ("stalker_heal", MonsterHandler::Callback(stalker_heal)),
        ("stalker_shoot_attack", MonsterHandler::Callback(stalker_shoot)),
        ("stalker_shoot_attack2", MonsterHandler::Callback(stalker_shoot_attack2)),
        ("stalker_swing_attack", MonsterHandler::Callback(stalker_swing_attack)),
        (
            "stalker_jump_straightup",
            MonsterHandler::Callback(stalker_jump_straightup),
        ),
        (
            "stalker_jump_wait_land",
            MonsterHandler::Callback(stalker_jump_wait_land),
        ),
        ("stalker_jump_up", MonsterHandler::Callback(stalker_jump_up)),
        ("stalker_jump_down", MonsterHandler::Callback(stalker_jump_down)),
        ("monster_done_dodge", MonsterHandler::Callback(finish_dodge)),
        ("stalker_dead", MonsterHandler::Callback(stalker_dead)),
    ] {
        definition.callbacks.insert(name.to_string(), handler);
    }
    definition
}
