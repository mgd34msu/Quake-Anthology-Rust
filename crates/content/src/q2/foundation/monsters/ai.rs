//! Walking-monster movement and perception (`src/content/q2/foundation/monsters/ai.ts`).
//!
//! Quake II `g_ai.c` / `m_move.c` and the rerelease game DLL
//! (id Software, GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, add3, dot3, length3, normalize3, scale3, sub3, vec3};

use super::alternate_fly::alternate_fly_step;
use super::types::{
    DeadThink, MonsterAi, MonsterAttackState, MonsterContext, MonsterLocomotion, SourceCombatMode,
};
use crate::q2::foundation::host::{
    Q2Edition, Q2GameServices, Q2Mode, Q2MotionKind, Q2TraceRequest,
};
use crate::q2::support::contracts::{BodyState, TraceHit, TraceResult};

/// Monster solid mask (`MASK_MONSTERSOLID`).
pub const MASK_MONSTER_SOLID: i32 = 1 | 2 | 0x20000 | 0x2000000;
/// Shot mask (`MASK_SHOT`).
pub const MASK_SHOT: i32 = 1 | 2 | 8 | 16 | 0x2000000;
/// Opaque mask (`MASK_OPAQUE`).
pub const MASK_OPAQUE: i32 = 1 | 8 | 16;
/// Water mask (`MASK_WATER`).
pub const MASK_WATER: i32 = 8 | 16 | 32;
/// Notarget flag (`FL_NOTARGET`).
pub const FL_NOTARGET: i32 = 32;

/// Monster solid mask with the rerelease bit (`monsterSolidMask`).
pub fn monster_solid_mask(game: &Q2GameServices) -> i32 {
    MASK_MONSTER_SOLID | if game.options.edition == Q2Edition::Rerelease { 0x40000000 } else { 0 }
}

/// Attack trace mask (`attackTraceMask`).
pub fn attack_trace_mask(game: &Q2GameServices) -> i32 {
    MASK_SHOT | if game.options.edition == Q2Edition::Rerelease { 0x40000000 } else { 0 }
}

/// Direction axes from Euler angles (`anglesVectors`).
pub fn angles_vectors(angles: Vec3) -> qa_core::math::AngleVectors {
    let yaw = f64::from(angles.y) * std::f64::consts::PI / 180.0;
    let pitch = f64::from(angles.x) * std::f64::consts::PI / 180.0;
    let roll = f64::from(angles.z) * std::f64::consts::PI / 180.0;
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let (sr, cr) = roll.sin_cos();
    qa_core::math::AngleVectors {
        forward: vec3((cp * cy) as f32, (cp * sy) as f32, (-sp) as f32),
        right: vec3(
            (-sr * sp * cy + cr * sy) as f32,
            (-sr * sp * sy - cr * cy) as f32,
            (-sr * cp) as f32,
        ),
        up: vec3(
            (cr * sp * cy + sr * sy) as f32,
            (cr * sp * sy - sr * cy) as f32,
            (cr * cp) as f32,
        ),
    }
}

/// Euler angles facing a direction (`vectorAngles`).
pub fn vector_angles(direction: Vec3) -> Vec3 {
    let yaw = if direction.x == 0.0 && direction.y == 0.0 {
        0.0
    } else {
        f64::from(direction.y).atan2(f64::from(direction.x)) * 180.0 / std::f64::consts::PI
    };
    let pitch = f64::from(direction.z).atan2(f64::from(length3(vec3(direction.x, direction.y, 0.0))))
        * 180.0
        / std::f64::consts::PI;
    vec3((-pitch) as f32, (if yaw < 0.0 { yaw + 360.0 } else { yaw }) as f32, 0.0)
}

/// Combat health (`health`).
pub fn health(game: &mut Q2GameServices, actor: Option<&ActorId>) -> f64 {
    let Some(actor) = actor else { return 0.0 };
    game.host.combat().read(actor).map(|state| state.health).unwrap_or(0.0)
}

/// Enemy body (`enemyBody`).
pub fn enemy_body(context: &mut MonsterContext) -> Option<BodyState> {
    let enemy = context.entity().enemy.clone();
    let enemy = enemy.as_ref()?;
    context.game.host.bodies().read(enemy)
}

/// Enemy eye position (`enemyEye`).
pub fn enemy_eye(context: &mut MonsterContext) -> Option<Vec3> {
    let body = enemy_body(context)?;
    let enemy = context.entity().enemy.clone();
    let observed = context.game.monster_target(enemy.as_ref())?;
    Some(vec3(body.origin.x, body.origin.y, body.origin.z + observed.view_height as f32))
}

/// Distance to the enemy (`targetDistance`).
pub fn target_distance(context: &mut MonsterContext) -> f64 {
    let Some(enemy) = enemy_body(context) else { return f64::INFINITY };
    let actor = context.actor().clone();
    let body = context.game.body_of(actor);
    if context.game.options.edition == Q2Edition::Classic {
        return f64::from(length3(sub3(enemy.origin, body.origin)));
    }
    let axis = |a: f32, amin: f32, amax: f32, b: f32, bmin: f32, bmax: f32| -> f64 {
        let (a, amin, amax, b, bmin, bmax) =
            (f64::from(a), f64::from(amin), f64::from(amax), f64::from(b), f64::from(bmin), f64::from(bmax));
        0.0f64.max(b + bmin - a - amax).max(a + amin - b - bmax)
    };
    axis(body.origin.x, body.bounds.min.x, body.bounds.max.x, enemy.origin.x, enemy.bounds.min.x, enemy.bounds.max.x)
        .hypot(axis(
            body.origin.y, body.bounds.min.y, body.bounds.max.y, enemy.origin.y, enemy.bounds.min.y,
            enemy.bounds.max.y,
        ))
        .hypot(axis(
            body.origin.z, body.bounds.min.z, body.bounds.max.z, enemy.origin.z, enemy.bounds.min.z,
            enemy.bounds.max.z,
        ))
}

/// Whether an actor is visible (`visible`).
///
/// `None` selects the current enemy, matching the donor default.
pub fn visible(context: &mut MonsterContext, actor: Option<&ActorId>) -> bool {
    let target_actor = match actor {
        Some(actor) => actor.clone(),
        None => {
            let Some(enemy) = context.entity().enemy.clone() else { return false };
            enemy
        }
    };
    let Some(target) = context.game.host.bodies().read(&target_actor) else { return false };
    let self_actor = context.actor().clone();
    let origin = context.game.body_of(self_actor.clone()).origin;
    let view_height = context.entity().view_height;
    let start = vec3(origin.x, origin.y, origin.z + view_height as f32);
    let Some(observed) = context.game.monster_target(Some(&target_actor)) else { return false };
    let end = vec3(target.origin.x, target.origin.y, target.origin.z + observed.view_height as f32);
    context.game.host.trace(&Q2TraceRequest {
        start,
        end,
        bounds: None,
        ignore: Some(self_actor),
        mask: MASK_OPAQUE,
        exclude: Vec::new(),
    })
    .fraction
        == 1.0
}

/// Whether an actor is in front (`inFront`).
pub fn in_front(context: &mut MonsterContext, actor: &ActorId) -> bool {
    let Some(body) = context.game.host.bodies().read(actor) else { return false };
    let self_actor = context.actor().clone();
    let origin = context.game.body_of(self_actor);
    f64::from(dot3(
        angles_vectors(origin.angles).forward,
        normalize3(sub3(body.origin, origin.origin)),
    )) > 0.3
}

/// Project a muzzle flash offset (`projectFlash`).
pub fn project_flash(context: &mut MonsterContext, offset: Vec3, angles: Option<Vec3>) -> Vec3 {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor);
    let angles = angles.unwrap_or(body.angles);
    let basis = angles_vectors(angles);
    let size = if context.game.options.edition == Q2Edition::Rerelease {
        context.entity().scale as f32
    } else {
        1.0
    };
    add3(
        body.origin,
        add3(
            scale3(basis.forward, offset.x * size),
            add3(scale3(basis.right, offset.y * size), vec3(0.0, 0.0, offset.z * size)),
        ),
    )
}

/// Whether a muzzle offset has a clear shot (`clearShot`).
pub fn clear_shot(context: &mut MonsterContext, offset: Vec3) -> bool {
    let Some(eye) = enemy_eye(context) else { return false };
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    let actor = context.actor().clone();
    let angles = context.game.body_of(actor.clone()).angles;
    let ideal_yaw = context.state().ideal_yaw;
    let start = project_flash(
        context,
        offset,
        if rerelease { Some(vec3(angles.x, ideal_yaw as f32, 0.0)) } else { Some(angles) },
    );
    let blind = rerelease
        && (context.state().attack_state == MonsterAttackState::Blind
            || context.state().manual_steering
            || context.state().lost_sight);
    let mask = if rerelease { 0x42000003 | 0x4000 } else { MASK_SHOT };
    let end = if blind { context.state().blind_fire_target } else { eye };
    if clear_shot_attempt(context, start, mask, end, rerelease) {
        return true;
    }
    let enemy = enemy_body(context).map(|body| body.origin);
    let Some(origin) = enemy else { return false };
    rerelease && !blind && clear_shot_attempt(context, start, mask, origin, rerelease)
}

/// One clear-shot trace (`clear` in `clearShot`).
fn clear_shot_attempt(
    context: &mut MonsterContext,
    start: Vec3,
    mask: i32,
    end: Vec3,
    rerelease: bool,
) -> bool {
    let self_actor = context.actor().clone();
    let trace = context.game.host.trace(&Q2TraceRequest {
        start,
        end,
        bounds: None,
        ignore: Some(self_actor),
        mask,
        exclude: Vec::new(),
    });
    let hit_enemy = match &trace.hit {
        TraceHit::Actor { actor } => {
            let enemy = context.entity().enemy.clone();
            Some(actor) == enemy.as_ref() || {
                let is_player = context.game.host.is_player(actor);
                rerelease && is_player
            }
        }
        _ => false,
    };
    hit_enemy
        || if rerelease {
            trace.fraction > 0.8 && !trace.start_solid
        } else {
            trace.fraction == 1.0
        }
}

/// Resolve a trace ground actor (`traceGroundActor`).
pub fn trace_ground_actor(trace: &TraceResult, game: &mut Q2GameServices) -> Option<ActorId> {
    match &trace.hit {
        TraceHit::Actor { actor } => Some(actor.clone()),
        TraceHit::World { .. } => Some(game.host.world_actor()),
        TraceHit::None => None,
    }
}

/// Turn toward the ideal yaw (`changeYaw`).
pub fn change_yaw(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let current = (f64::from(body.angles.y) % 360.0 + 360.0) % 360.0;
    let mut movement = context.state().ideal_yaw - current;
    if movement > 180.0 {
        movement -= 360.0;
    }
    if movement < -180.0 {
        movement += 360.0;
    }
    let yaw_speed = context.state().yaw_speed;
    let speed = yaw_speed
        * if context.game.options.edition == Q2Edition::Rerelease {
            context.game.host.frame_seconds() * 10.0
        } else {
            1.0
        };
    movement = movement.clamp(-speed, speed);
    let mut moved = body;
    moved.angles.y = (((current + movement) % 360.0 + 360.0) % 360.0) as f32;
    context.game.write_body(actor, &moved, false);
}

/// Face the enemy (`faceEnemy`).
pub fn face_enemy(context: &mut MonsterContext) {
    let target = if context.state().manual_steering {
        Some(context.state().blind_fire_target)
    } else {
        enemy_body(context).map(|body| body.origin)
    };
    let Some(target) = target else { return };
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor).origin;
    context.state_mut().ideal_yaw = f64::from(vector_angles(sub3(target, origin)).y);
    change_yaw(context);
}

/// Check for floor support (`checkBottom`).
pub fn check_bottom(context: &mut MonsterContext, origin: Vec3) -> bool {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let minimum = add3(origin, body.bounds.min);
    let maximum = add3(origin, body.bounds.max);
    let ceiling = context.entity().gravity_vector.z > 0.0;
    let direction = if ceiling { 1.0 } else { -1.0 };
    let support = if ceiling { maximum.z } else { minimum.z };
    let corners = [
        vec3(minimum.x, minimum.y, support + direction),
        vec3(minimum.x, maximum.y, support + direction),
        vec3(maximum.x, minimum.y, support + direction),
        vec3(maximum.x, maximum.y, support + direction),
    ];
    if corners.iter().all(|point| context.game.host.point_contents(*point) == 1) {
        return true;
    }
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    let center =
        vec3((minimum.x + maximum.x) * 0.5, (minimum.y + maximum.y) * 0.5, support);
    let start = if rerelease { vec3(origin.x, origin.y, support) } else { center };
    let mask = monster_solid_mask(context.game);
    let middle = context.game.host.trace(&Q2TraceRequest {
        start,
        end: vec3(start.x, start.y, start.z + direction * 36.0),
        bounds: if rerelease {
            Some(Bounds {
                min: vec3(body.bounds.min.x, body.bounds.min.y, 0.0),
                max: vec3(body.bounds.max.x, body.bounds.max.y, 0.0),
            })
        } else {
            None
        },
        ignore: Some(actor.clone()),
        mask,
        exclude: Vec::new(),
    });
    if middle.fraction == 1.0 {
        return false;
    }
    if rerelease && (context.entity().spawnflags & 131072) != 0 {
        return true;
    }
    let quadrant = vec3((maximum.x - minimum.x) * 0.25, (maximum.y - minimum.y) * 0.25, 0.0);
    for corner in corners {
        let point = if rerelease {
            vec3(
                center.x + if corner.x == minimum.x { -quadrant.x } else { quadrant.x },
                center.y + if corner.y == minimum.y { -quadrant.y } else { quadrant.y },
                support,
            )
        } else {
            vec3(corner.x, corner.y, support)
        };
        let mask = monster_solid_mask(context.game);
        let trace = context.game.host.trace(&Q2TraceRequest {
            start: point,
            end: vec3(point.x, point.y, point.z + direction * 36.0),
            bounds: if rerelease {
                Some(Bounds { min: scale3(quadrant, -1.0), max: quadrant })
            } else {
                None
            },
            ignore: Some(actor.clone()),
            mask,
            exclude: Vec::new(),
        });
        if trace.fraction == 1.0 || f64::from(trace.end.z - middle.end.z) * f64::from(direction) > 18.0 {
            return false;
        }
    }
    true
}

/// Step toward a yaw (`walkMove`).
pub fn walk_move(
    context: &mut MonsterContext,
    yaw: f64,
    distance: f64,
    commit: bool,
    relink: bool,
) -> bool {
    let moved = source_move_step(context, yaw, distance, commit, relink);
    if commit && relink {
        context.consume_source_blocked();
    }
    moved
}

/// One source movement step (`sourceMoveStep`).
fn source_move_step(
    context: &mut MonsterContext,
    yaw: f64,
    distance: f64,
    commit: bool,
    relink: bool,
) -> bool {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let locomotion = context.state().locomotion;
    if locomotion == MonsterLocomotion::Stationary {
        return false;
    }
    if body.ground.is_none() && locomotion == MonsterLocomotion::Walk {
        return false;
    }
    let radians = yaw * std::f64::consts::PI / 180.0;
    let step = context.before_source_move(vec3(
        (radians.cos() * distance) as f32,
        (radians.sin() * distance) as f32,
        0.0,
    ));
    let displacement = match step {
        super::types::SourceMoveOutcome::Handled => return true,
        super::types::SourceMoveOutcome::Move { displacement } => displacement,
    };
    let destination = add3(body.origin, displacement);
    if locomotion == MonsterLocomotion::Fly || locomotion == MonsterLocomotion::Swim {
        return fly_move_step(context, &body, destination, commit, relink);
    }
    let gravity = context.entity().gravity_vector;
    let ceiling = gravity.z > 0.0;
    let start = add3(destination, scale3(gravity, -18.0));
    let end = add3(start, scale3(gravity, 36.0));
    let mask = monster_solid_mask(context.game);
    let mut trace = context.game.host.trace(&Q2TraceRequest {
        start,
        end,
        bounds: Some(body.bounds),
        ignore: Some(actor.clone()),
        mask,
        exclude: Vec::new(),
    });
    if trace.all_solid {
        return false;
    }
    if trace.start_solid {
        let retry = if context.game.options.edition == Q2Edition::Classic {
            vec3(start.x, start.y, start.z - 18.0)
        } else {
            destination
        };
        let mask = monster_solid_mask(context.game);
        trace = context.game.host.trace(&Q2TraceRequest {
            start: retry,
            end,
            bounds: Some(body.bounds),
            ignore: Some(actor.clone()),
            mask,
            exclude: Vec::new(),
        });
        if trace.start_solid || trace.all_solid {
            return false;
        }
    }
    let support_offset = if ceiling { body.bounds.max.z - 1.0 } else { body.bounds.min.z + 1.0 };
    let feet = vec3(body.origin.x, body.origin.y, body.origin.z + support_offset);
    if context.game.host.point_contents(feet) & MASK_WATER == 0
        && context.game.host.point_contents(vec3(trace.end.x, trace.end.y, trace.end.z + support_offset))
            & MASK_WATER
            != 0
    {
        return false;
    }
    if trace.fraction == 1.0 {
        if (context.entity().flags & 256) == 0 {
            return false;
        }
        if commit {
            let mut moved = context.game.body_of(actor.clone());
            moved.origin = destination;
            moved.ground = None;
            context.game.write_body(actor.clone(), &moved, relink);
            if relink {
                touch_triggers(context);
            }
        }
        return true;
    }
    if !context.accepts_source_ground_move(trace.end) {
        return false;
    }
    if !check_bottom(context, trace.end) {
        if (context.entity().flags & 256) == 0 {
            return false;
        }
        if commit {
            let mut moved = context.game.body_of(actor.clone());
            moved.origin = trace.end;
            context.game.write_body(actor.clone(), &moved, relink);
            if relink {
                touch_triggers(context);
            }
        }
        return true;
    }
    if commit {
        context.entity_mut().flags &= !256;
        let end = trace.end;
        let mut moved = context.game.body_of(actor.clone());
        moved.origin = end;
        let game = &mut *context.game;
        moved.ground = trace_ground_actor(&trace, game);
        game.write_body(actor.clone(), &moved, relink);
        if relink {
            let owned = game.owned_of(actor);
            game.host.touch_triggers(&owned);
        }
    }
    true
}

/// Fly/swim movement step.
fn fly_move_step(
    context: &mut MonsterContext,
    body: &BodyState,
    destination: Vec3,
    commit: bool,
    relink: bool,
) -> bool {
    let actor = context.actor().clone();
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    if rerelease && context.state().alternate_fly && alternate_fly_step(context) {
        return true;
    }
    let wet = context.game.host.point_contents(vec3(
        body.origin.x,
        body.origin.y,
        body.origin.z + body.bounds.min.z + 1.0,
    )) & MASK_WATER
        != 0;
    let deep = context.game.host.point_contents(vec3(
        body.origin.x,
        body.origin.y,
        body.origin.z + body.bounds.min.z + 27.0,
    )) & MASK_WATER
        != 0;
    for attempt in 0..2 {
        let mut end = destination;
        if attempt == 0 && context.entity().enemy.is_some() {
            if context.entity().goal.is_none() {
                let enemy = context.entity().enemy.clone();
                context.entity_mut().goal = enemy;
            }
            let goal = context.entity().goal.clone();
            let goal_body =
                goal.as_ref().and_then(|goal| context.game.host.bodies().read(goal));
            if let Some(goal_body) = goal_body {
                let dz = f64::from(body.origin.z - goal_body.origin.z);
                let stride = if context.game.options.edition == Q2Edition::Rerelease {
                    context.game.host.frame_seconds() * 80.0
                } else {
                    8.0
                };
                let goal_actor = goal.clone().expect("goal body");
                if context.game.host.is_player(&goal_actor) {
                    if dz > 40.0 {
                        end.z -= stride as f32;
                    }
                    let swim_shallow =
                        context.state().locomotion == MonsterLocomotion::Swim && !deep;
                    if dz < 30.0 && !swim_shallow {
                        end.z += stride as f32;
                    }
                } else {
                    end.z += (if dz > stride {
                        -stride
                    } else if dz > 0.0 {
                        -dz
                    } else if dz < -stride {
                        stride
                    } else {
                        dz
                    }) as f32;
                }
            }
        }
        let mask = monster_solid_mask(context.game);
        let trace = context.game.host.trace(&Q2TraceRequest {
            start: body.origin,
            end,
            bounds: Some(body.bounds),
            ignore: Some(actor.clone()),
            mask,
            exclude: Vec::new(),
        });
        let enters_water = context.game.host.point_contents(vec3(
            trace.end.x,
            trace.end.y,
            trace.end.z + body.bounds.min.z + 1.0,
        )) & MASK_WATER
            != 0;
        let locomotion = context.state().locomotion;
        if locomotion == MonsterLocomotion::Fly && !wet && enters_water
            || locomotion == MonsterLocomotion::Swim && !deep && !enters_water
        {
            return false;
        }
        if trace.fraction == 1.0 && !trace.start_solid && !trace.all_solid {
            if commit {
                let mut moved = context.game.body_of(actor.clone());
                moved.origin = trace.end;
                context.game.write_body(actor.clone(), &moved, relink);
                if relink {
                    touch_triggers(context);
                }
            }
            return true;
        }
        if context.entity().enemy.is_none() {
            break;
        }
    }
    false
}

/// Touch triggers for the context actor.
fn touch_triggers(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let owned = context.game.owned_of(actor);
    context.game.host.touch_triggers(&owned);
}

/// Whether an actor is live.
fn is_live(context: &mut MonsterContext, actor: &ActorId) -> bool {
    context.game.host.actors().is_live(actor)
}

/// Step in a direction (`stepDirection`).
pub fn step_direction(context: &mut MonsterContext, yaw: f64, distance: f64) -> bool {
    context.state_mut().ideal_yaw = yaw;
    change_yaw(context);
    let actor = context.actor().clone();
    let previous = context.game.body_of(actor.clone());
    if !walk_move(context, yaw, distance, true, false) {
        context.game.link_actor(actor.clone());
        touch_triggers(context);
        return !is_live(context, &actor);
    }
    context.consume_source_blocked();
    if !is_live(context, &actor) {
        return true;
    }
    let ideal_yaw = context.state().ideal_yaw;
    let yaw_now = f64::from(context.game.body_of(actor.clone()).angles.y);
    let delta = ((yaw_now - ideal_yaw) % 360.0 + 360.0) % 360.0;
    let rogue_widow = context.source_combat_rules() == SourceCombatMode::Rogue
        && context.entity().classname.starts_with("monster_widow");
    if delta > 45.0 && delta < 315.0 && !rogue_widow {
        let mut moved = context.game.body_of(actor.clone());
        moved.origin = previous.origin;
        context.game.write_body(actor.clone(), &moved, false);
    }
    context.game.link_actor(actor.clone());
    touch_triggers(context);
    true
}

/// Chase-direction sweep (`chaseDirection`).
///
/// `SV_NewChaseDir`'s diagonal/cardinal sweep preserves source RNG and
/// yaw order.
pub fn chase_direction(context: &mut MonsterContext, goal: Vec3, distance: f64) -> bool {
    let old = (context.state().ideal_yaw / 45.0).trunc() * 45.0;
    let turnaround = (old - 180.0 + 360.0) % 360.0;
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    let delta = sub3(goal, origin);
    let mut x = if delta.x > 10.0 {
        0.0
    } else if delta.x < -10.0 {
        180.0
    } else {
        -1.0
    };
    let mut y = if delta.y < -10.0 {
        270.0
    } else if delta.y > 10.0 {
        90.0
    } else {
        -1.0
    };
    if x != -1.0 && y != -1.0 {
        let diagonal = if x == 0.0 {
            if y == 90.0 { 45.0 } else { 315.0 }
        } else if y == 90.0 {
            135.0
        } else {
            215.0
        };
        if diagonal != turnaround && step_direction(context, diagonal, distance) {
            return true;
        }
    }
    let rogue = context.source_combat_rules() == SourceCombatMode::Rogue;
    let direction_roll = (context.game.random() * 4.0).floor() as i32;
    if (if rogue { direction_roll & 1 != 0 } else { direction_roll != 0 })
        || delta.y.abs() > delta.x.abs()
    {
        std::mem::swap(&mut x, &mut y);
    }
    if x != -1.0 && x != turnaround && step_direction(context, x, distance) {
        return true;
    }
    if y != -1.0 && y != turnaround && step_direction(context, y, distance) {
        return true;
    }
    let actor = context.actor().clone();
    if rogue
        && is_live(context, &actor)
        && health(context.game, Some(&actor)) > 0.0
        && context.blocked(distance)
    {
        return true;
    }
    if old != -1.0 && step_direction(context, old, distance) {
        return true;
    }
    let descending = (context.game.random() * 2.0).floor() as i32 == 0;
    for index in 0..8 {
        let yaw = if descending { 315.0 - index as f64 * 45.0 } else { index as f64 * 45.0 };
        if yaw != turnaround && step_direction(context, yaw, distance) {
            return true;
        }
    }
    if turnaround != -1.0 && step_direction(context, turnaround, distance) {
        return true;
    }
    context.state_mut().ideal_yaw = old;
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor).origin;
    if !check_bottom(context, origin) {
        context.entity_mut().flags |= 256;
    }
    false
}

/// Run frame AI (`runAi`).
pub fn run_ai(context: &mut MonsterContext, ai: &MonsterAi, distance: f64) {
    let rogue = context.source_combat_rules() == SourceCombatMode::Rogue;
    let extended =
        context.game.options.edition == Q2Edition::Rerelease || rogue;
    match ai {
        MonsterAi::None => {}
        MonsterAi::Turn => {
            if distance != 0.0 {
                let actor = context.actor().clone();
                let yaw = f64::from(context.game.body_of(actor).angles.y);
                walk_move(context, yaw, distance, true, true);
            }
            let actor = context.actor().clone();
            if !is_live(context, &actor) {
                return;
            }
            if context.find_target() {
                return;
            }
            let manual = context.state().manual_steering;
            if context.game.options.edition == Q2Edition::Classic || !manual {
                change_yaw(context);
            }
        }
        MonsterAi::Move | MonsterAi::SoldierMove => {
            let actor = context.actor().clone();
            let yaw = f64::from(context.game.body_of(actor.clone()).angles.y);
            walk_move(context, yaw, distance, true, true);
            if !is_live(context, &actor) {
                return;
            }
            if matches!(ai, MonsterAi::SoldierMove) && !prone_shot(context) {
                context.dispatch("soldier_stand_up");
            }
        }
        MonsterAi::Charge => run_ai_charge(context, distance, rogue, extended),
        MonsterAi::Stand => run_ai_stand(context, distance, rogue),
        MonsterAi::Walk => {
            context.move_to_goal(distance);
            let actor = context.actor().clone();
            if !is_live(context, &actor) {
                return;
            }
            if context.find_target() {
                return;
            }
            let now = context.game.host.now();
            if context.state().has_search && now > context.state().idle_time {
                if context.state().idle_time != 0.0 {
                    context.search();
                    let idle = context.game.host.now() + 15.0 + context.game.random() * 15.0;
                    context.state_mut().idle_time = idle;
                } else {
                    let idle = context.game.host.now() + context.game.random() * 15.0;
                    context.state_mut().idle_time = idle;
                }
            }
        }
        MonsterAi::Run => run_ai_run(context, distance, rogue, extended),
        MonsterAi::Source(name) => {
            panic!("Named source AI {name} runs through the move dispatch, not run_ai");
        }
    }
}

/// Charge AI.
fn run_ai_charge(context: &mut MonsterContext, distance: f64, rogue: bool, extended: bool) {
    let enemy = enemy_body(context);
    if extended && enemy.is_none() {
        return;
    }
    if rogue && context.game.options.edition == Q2Edition::Classic {
        if let Some(enemy) = enemy.as_ref() {
            if visible(context, None) {
                context.state_mut().blind_fire_target = enemy.origin;
            }
        }
    }
    if !context.state().manual_steering {
        face_enemy(context);
    } else {
        change_yaw(context);
    }
    if context.game.options.edition == Q2Edition::Rerelease {
        if let Some(enemy) = enemy.as_ref() {
            if visible(context, None) {
                context.state_mut().blind_fire_target =
                    add3(enemy.origin, scale3(enemy.velocity, -0.1));
            }
        }
    }
    if distance != 0.0 {
        if extended && context.state().charging {
            context.move_to_goal(distance);
            return;
        }
        let actor = context.actor().clone();
        let yaw = f64::from(context.game.body_of(actor).angles.y);
        if extended && context.state().attack_state == MonsterAttackState::Sliding {
            let enemy_actor = context.entity().enemy.clone();
            let tesla = rogue
                && context.game.options.edition == Q2Edition::Classic
                && enemy_actor.as_ref().is_some_and(|enemy| {
                    context.game.entity(enemy).is_some_and(|target| target.classname == "tesla")
                });
            let side = if tesla {
                0.0
            } else if context.state().lefty {
                90.0
            } else {
                -90.0
            };
            let sideways = if context.game.options.edition == Q2Edition::Classic {
                distance
            } else {
                distance * context.state().current_move.sidestep_scale
            };
            let ideal = context.state().ideal_yaw;
            if !walk_move(context, ideal + side, sideways, true, true) {
                let lefty = context.state().lefty;
                context.state_mut().lefty = !lefty;
                let ideal = context.state().ideal_yaw;
                walk_move(context, ideal - side, sideways, true, true);
            }
        } else {
            walk_move(context, yaw, distance, true, true);
        }
    }
    if context.game.options.edition == Q2Edition::Rerelease && target_distance(context) <= 50.0 {
        change_yaw(context);
    }
}

/// Stand AI.
fn run_ai_stand(context: &mut MonsterContext, distance: f64, rogue: bool) {
    if distance != 0.0 {
        let actor = context.actor().clone();
        let yaw = f64::from(context.game.body_of(actor.clone()).angles.y);
        walk_move(context, yaw, distance, true, true);
        if !is_live(context, &actor) {
            return;
        }
    } else {
        let actor = context.actor().clone();
        if !is_live(context, &actor) {
            return;
        }
    }
    if context.state().stand_ground {
        if context.entity().enemy.is_some() {
            let enemy = enemy_body(context);
            if let Some(enemy) = enemy.as_ref() {
                let actor = context.actor().clone();
                let origin = context.game.body_of(actor).origin;
                context.state_mut().ideal_yaw =
                    f64::from(vector_angles(sub3(enemy.origin, origin)).y);
            }
            let actor = context.actor().clone();
            let yaw = f64::from(context.game.body_of(actor).angles.y);
            let ideal = context.state().ideal_yaw;
            if yaw != ideal && context.state().temporary_stand_ground {
                context.state_mut().stand_ground = false;
                context.state_mut().temporary_stand_ground = false;
                context.run();
            }
            let manual = context.state().manual_steering;
            if !rogue || !manual {
                change_yaw(context);
            }
            let attacking = context.check_attack(0.0);
            if rogue {
                let target = enemy_body(context);
                if target.is_some() && visible(context, None) {
                    let origin = target.map(|body| body.origin).unwrap_or(vec3(0.0, 0.0, 0.0));
                    let now = context.game.host.now();
                    context.state_mut().lost_sight = false;
                    context.state_mut().last_sighting = origin;
                    context.state_mut().blind_fire_target = origin;
                    context.state_mut().trail_time = now;
                    context.state_mut().blind_fire_delay = 0.0;
                } else if !attacking {
                    context.find_target();
                }
            }
        } else {
            context.find_target();
        }
        return;
    }
    if context.find_target() {
        return;
    }
    if context.game.host.now() > context.state().pause_time {
        context.walk();
        return;
    }
    let spawnflags = context.entity().spawnflags;
    if context.state().has_idle && (spawnflags & 1) == 0 && context.game.host.now() > context.state().idle_time {
        if context.state().idle_time != 0.0 {
            context.idle();
            let idle = context.game.host.now() + 15.0 + context.game.random() * 15.0;
            context.state_mut().idle_time = idle;
        } else {
            let idle = context.game.host.now() + context.game.random() * 15.0;
            context.state_mut().idle_time = idle;
        }
    }
}

/// Run AI.
fn run_ai_run(context: &mut MonsterContext, distance: f64, rogue: bool, extended: bool) {
    if context.state().combat_point {
        context.move_to_goal(distance);
        return;
    }
    if rogue && context.game.options.edition == Q2Edition::Classic {
        context.state_mut().ducked = false;
        let actor = context.actor().clone();
        let body = context.game.body_of(actor.clone());
        if f64::from(body.bounds.max.z) != context.state().normal_height {
            context.state_mut().can_take_damage = true;
            let duck = context.game.host.now() + 0.5;
            context.state_mut().next_duck_time = duck;
            let owned = context.game.owned_of(actor.clone());
            context.game.set_combat_traits(&owned, &crate::q2::support::contracts::CombatTraitChanges {
                can_take_damage: Some(true),
                ..crate::q2::support::contracts::CombatTraitChanges::default()
            });
            let mut moved = context.game.body_of(actor.clone());
            let normal = context.state().normal_height;
            moved.bounds.max.z = normal as f32;
            context.game.write_body(actor, &moved, true);
        }
    }
    if context.run_hint_path(distance) {
        return;
    }
    let mut already_moved = false;
    if context.state().sound_target.is_some() {
        let actor = context.actor().clone();
        let origin = context.game.body_of(actor).origin;
        let target = context.state().sound_target.as_ref().map(|sound| sound.origin).unwrap_or(origin);
        let close = if context.game.options.edition == Q2Edition::Classic { 64.0 } else { 32.0 };
        if f64::from(length3(sub3(origin, target))) < close {
            context.state_mut().stand_ground = true;
            context.state_mut().temporary_stand_ground = true;
            context.stand();
            return;
        }
        context.move_to_goal(distance);
        already_moved = extended;
        let actor = context.actor().clone();
        if !is_live(context, &actor) {
            return;
        }
        if !context.find_target() {
            return;
        }
    }
    let attacking = context.check_attack(distance);
    if extended {
        if !visible(context, None) && context.state().attack_state == MonsterAttackState::Sliding {
            context.state_mut().attack_state = MonsterAttackState::Straight;
        }
        if context.state().dodging {
            context.state_mut().attack_state = MonsterAttackState::Sliding;
        }
    } else if attacking {
        return;
    }
    if context.state().attack_state == MonsterAttackState::Sliding {
        if !already_moved {
            slide(context, distance);
        }
        if !attacking && context.state().attack_state == MonsterAttackState::Sliding {
            return;
        }
    } else if context.state().charging && !context.state().manual_steering {
        face_enemy(context);
    }
    if attacking {
        if distance != 0.0
            && !already_moved
            && context.state().attack_state == MonsterAttackState::Straight
            && !context.state().stand_ground
        {
            context.move_to_goal(distance);
        }
        return;
    }
    if context.state().stand_ground {
        return;
    }
    if !visible(context, None) && context.check_lost_hint_path() {
        return;
    }
    if !visible(context, None) && context.game.options.mode == Q2Mode::Coop && context.find_target() {
        return;
    }
    if !already_moved {
        context.move_to_goal(distance);
    }
    let actor = context.actor().clone();
    if !is_live(context, &actor) {
        return;
    }
    if context.state().search_time != 0.0 && context.game.host.now() > context.state().search_time + 20.0 {
        context.state_mut().search_time = 0.0;
    }
}

/// Strafe slide.
fn slide(context: &mut MonsterContext, distance: f64) {
    if !context.state().manual_steering {
        face_enemy(context);
    }
    let side = if context.state().lefty { 90.0 } else { -90.0 };
    let amount = if context.game.options.edition == Q2Edition::Rerelease
        && context.state().locomotion != MonsterLocomotion::Fly
    {
        distance.min(8.0 / (context.game.host.frame_seconds() * 100.0))
    } else {
        distance
    };
    let ideal = context.state().ideal_yaw;
    if walk_move(context, ideal + side, amount, true, true) {
        return;
    }
    let rogue = context.source_combat_rules() == SourceCombatMode::Rogue;
    if (context.game.options.edition == Q2Edition::Rerelease || rogue) && context.state().dodging {
        finish_dodge(context);
        context.state_mut().attack_state = MonsterAttackState::Straight;
        return;
    }
    let lefty = context.state().lefty;
    context.state_mut().lefty = !lefty;
    let ideal = context.state().ideal_yaw;
    if !walk_move(context, ideal - side, amount, true, true)
        && (context.game.options.edition == Q2Edition::Rerelease || rogue)
    {
        context.state_mut().attack_state = MonsterAttackState::Straight;
    }
}

/// Whether a prone soldier can shoot (`proneShot`).
pub fn prone_shot(context: &mut MonsterContext) -> bool {
    let enemy = enemy_body(context);
    let Some(enemy) = enemy else { return false };
    let enemy_actor = context.entity().enemy.clone();
    if health(context.game, enemy_actor.as_ref()) <= 0.0 {
        return false;
    }
    let actor = context.actor().clone();
    let body = context.game.body_of(actor);
    let difference = sub3(enemy.origin, body.origin);
    f64::from(dot3(
        angles_vectors(body.angles).forward,
        normalize3(vec3(difference.x, difference.y, 0.0)),
    )) >= 0.8
}

/// Duck or stand (`setDuck`).
pub fn set_duck(context: &mut MonsterContext, down: bool) {
    if !down && !context.state().ducked {
        return;
    }
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    context.state_mut().ducked = down;
    let normal = context.state().normal_height;
    let mut moved = body;
    moved.bounds.max.z = (normal - if down { 32.0 } else { 0.0 }) as f32;
    context.game.write_body(actor, &moved, true);
}

/// Finish a dodge (`finishDodge`).
pub fn finish_dodge(context: &mut MonsterContext) {
    context.state_mut().dodging = false;
    if context.state().attack_state == MonsterAttackState::Sliding {
        context.state_mut().attack_state = MonsterAttackState::Straight;
    }
}

/// Become a corpse (`corpse`).
pub fn corpse(context: &mut MonsterContext) {
    context.state_mut().corpse = true;
    context.entity_mut().server_flags |= 2;
    let actor = context.actor().clone();
    let mut moved = context.game.body_of(actor.clone());
    moved.bounds = Bounds {
        min: vec3(-16.0, -16.0, -24.0),
        max: vec3(16.0, 16.0, -8.0),
    };
    context.game.write_body(actor.clone(), &moved, true);
    context.game.set_motion_kind(actor.clone(), Q2MotionKind::Toss);
    context.game.cancel_actor(actor);
    if context.game.options.edition == Q2Edition::Classic {
        if context.state().kind == "infantry" {
            fly_check(context);
        }
    } else {
        context.schedule(0.1, DeadThink::MonsterDeadThink);
    }
}

/// Toggle corpse flies (`setFlies`).
fn set_flies(context: &mut MonsterContext, on: bool) {
    if on {
        context.entity_mut().effects |= 0x4000;
    } else {
        context.entity_mut().effects &= !0x4000;
    }
    let actor = context.actor().clone();
    let origin = context.game.body_of(actor.clone()).origin;
    context.game.host_emit(crate::q2::foundation::host::Q2PresentationEvent::Sound(
        crate::q2::foundation::host::Q2SoundEvent {
            actor: Some(actor.clone()),
            origin,
            path: "infantry/inflies1.wav".to_string(),
            channel: 0,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: if on {
                crate::q2::foundation::host::Q2SoundLoop::Start
            } else {
                crate::q2::foundation::host::Q2SoundLoop::Stop
            },
            loop_owner: None,
        },
    ));
    context.game.show(actor);
}

/// Dead think (`monsterDeadThink`).
pub fn monster_dead_think(context: &mut MonsterContext) {
    if context.state().kind == "infantry" {
        let flies_time = context.state().flies_time;
        if flies_time.is_none() {
            let flies = context.game.host.now() + 5.0 + context.game.random() * 10.0;
            context.state_mut().flies_time = Some(flies);
        } else if flies_time.is_some_and(|time| time < context.game.host.now()) {
            let on = (context.entity().effects & 0x4000) == 0;
            set_flies(context, on);
            context.state_mut().flies_time =
                Some(if on { context.game.host.now() + 60.0 } else { f64::MAX });
        }
    }
    let last_frame = context.state().current_move.last_frame;
    if context.entity().frame != last_frame {
        context.entity_mut().frame += 1;
    }
    let actor = context.actor().clone();
    context.game.show(actor);
    context.schedule(0.1, DeadThink::MonsterDeadThink);
}

/// Whether a corpse is wet (`wetCorpse`).
fn wet_corpse(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor);
    context.game.host.point_contents(vec3(
        body.origin.x,
        body.origin.y,
        body.origin.z + body.bounds.min.z + 1.0,
    )) & MASK_WATER
        != 0
}

/// Turn corpse flies on (`fliesOn`).
pub fn flies_on(context: &mut MonsterContext) {
    if wet_corpse(context) {
        return;
    }
    set_flies(context, true);
    context.schedule(60.0, DeadThink::FliesOn);
}

/// Turn corpse flies off (`fliesOff`).
pub fn flies_off(context: &mut MonsterContext) {
    set_flies(context, false);
    let actor = context.actor().clone();
    context.game.cancel_actor(actor);
}

/// Maybe schedule corpse flies (`flyCheck`).
pub fn fly_check(context: &mut MonsterContext) {
    if wet_corpse(context) || context.game.random() > 0.5 {
        return;
    }
    let delay = 5.0 + 10.0 * context.game.random();
    context.schedule(delay, DeadThink::FliesOn);
}

/// Body origin or zero (`bodyOf`).
pub fn body_of(game: &mut Q2GameServices, actor: Option<&ActorId>) -> Vec3 {
    let Some(actor) = actor else { return vec3(0.0, 0.0, 0.0) };
    game.body_of(actor.clone()).origin
}
