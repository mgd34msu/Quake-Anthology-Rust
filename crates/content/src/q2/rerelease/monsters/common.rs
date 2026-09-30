//! Rerelease shared monster routines (`src/content/q2/rerelease/monsters/common.ts`).

use qa_core::math::{Vec3, add3, dot3, length3, normalize3, scale3, sub3, vec3};

use crate::q2::foundation::host::Q2Edition;
use crate::q2::foundation::monsters::ai::{
    angles_vectors, attack_trace_mask, change_yaw, enemy_body, finish_dodge, health,
    monster_solid_mask, vector_angles,
};
use crate::q2::foundation::monsters::types::{
    MonsterContext, PlatformPhase,
};
use crate::q2::support::contracts::{AttackCause, TraceContact, TraceHit, TraceResult};
use crate::q2::support::misc::Q2RereleaseRandomSource;

/// Rerelease RNG (`rereleaseRandom`).
pub fn rerelease_random<'x, 'a>(
    context: &'x mut MonsterContext<'a>,
) -> &'x mut dyn Q2RereleaseRandomSource {
    context
        .game
        .host
        .rerelease_random()
        .unwrap_or_else(|| panic!("Rerelease monster source requires the session random stream"))
}

/// Trace contents (`traceContents`).
fn trace_contents(trace: &TraceResult) -> i32 {
    trace.q2().map(|fields| fields.contents).unwrap_or(0)
}

/// Contact normal or zero.
fn contact_normal(trace: &TraceResult) -> Vec3 {
    match &trace.contact {
        TraceContact::Plane { plane } => plane.normal,
        TraceContact::None => vec3(0.0, 0.0, 0.0),
    }
}

/// Calculate a lobbed pitch (`calculatePitchToFire`).
#[allow(clippy::too_many_arguments)]
pub fn calculate_pitch_to_fire(
    context: &mut MonsterContext,
    target: Vec3,
    start: Vec3,
    aim: Vec3,
    speed: f64,
    seconds: f64,
    mortar: bool,
    destroy_on_touch: bool,
) -> Option<Vec3> {
    let angles = vector_angles(aim);
    let gravity = context.game.host.gravity() as f32;
    let mut best_pitch = 0.0f32;
    let mut best_distance = f32::INFINITY;
    for pitch in [-80.0f32, -70.0, -60.0, -50.0, -40.0, -30.0, -20.0, -10.0, -5.0] {
        if mortar && pitch >= -30.0 {
            break;
        }
        let mut velocity = scale3(
            angles_vectors(vec3(angles.x, angles.y, pitch)).forward,
            speed as f32,
        );
        let mut origin = start;
        let mut remaining = seconds;
        while remaining > 0.0 {
            remaining -= 0.1;
            velocity.z -= gravity * 0.1;
            let trace = context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
                start: origin,
                end: add3(origin, scale3(velocity, 0.1)),
                bounds: None,
                ignore: None,
                mask: 3 | 0x2000000 | 0x4000000 | 0x40000000,
                exclude: Vec::new(),
            });
            origin = trace.end;
            if trace.fraction >= 1.0 {
                continue;
            }
            let surface_flags = trace
                .q2()
                .and_then(|fields| fields.surface.as_ref())
                .map(|surface| surface.flags)
                .unwrap_or(0);
            if trace.q2().is_some() && surface_flags & 4 != 0 {
                break;
            }
            let normal = contact_normal(&trace);
            origin = add3(origin, normal);
            velocity = sub3(velocity, scale3(normal, dot3(velocity, normal) * 1.6));
            let delta = sub3(origin, target);
            let distance = dot3(delta, delta);
            let enemy = context.entity().enemy.clone();
            let hit_enemy = matches!(&trace.hit, TraceHit::Actor { actor } if Some(actor) == enemy.as_ref());
            let hit_player = matches!(&trace.hit, TraceHit::Actor { actor } if context.game.host.is_player(actor));
            if hit_enemy || hit_player
                || normal.z >= 0.7 && distance < 128.0 * 128.0 && distance < best_distance
            {
                best_pitch = pitch;
                best_distance = distance;
            }
            if destroy_on_touch || trace_contents(&trace) & (0x2000000 | 0x4000000 | 0x40000000) != 0
            {
                break;
            }
        }
    }
    if best_distance.is_finite() {
        Some(angles_vectors(vec3(angles.x, angles.y, best_pitch)).forward)
    } else {
        None
    }
}

/// Whether the last attack was a chainfist (`chainfist`).
pub fn chainfist(context: &mut MonsterContext) -> bool {
    matches!(
        context.entity().last_attack.as_ref().map(|attack| &attack.cause),
        Some(AttackCause::Q2 { means_of_death: 40, .. })
    )
}

/// Whether pain reacts (`reactsToPain`).
pub fn reacts_to_pain(context: &mut MonsterContext) -> bool {
    !context.state().ducked
        && !context.state().combat_point
        && (context.game.options.skill < 3 || chainfist(context))
}

/// Whether to gib (`checkGib`).
pub fn check_gib(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    let gib_health = context.state().gib_health;
    if health(&mut *context.game, Some(&actor)) <= gib_health {
        return true;
    }
    context.state().dead
        && matches!(
            context.entity().last_attack.as_ref().map(|attack| &attack.cause),
            Some(AttackCause::Q2 { means_of_death: 20, .. })
        )
}

/// Emit a monster muzzle flash (`monsterFlash`).
pub fn monster_flash(context: &mut MonsterContext, flash: i32, origin: Vec3, direction: Vec3) {
    crate::q2::base::monsters::common::monster_muzzle(context, flash, direction, origin);
}

/// Predicted aim (`predictAim`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PredictedAim {
    /// Direction.
    pub direction: Vec3,
    /// Point.
    pub point: Vec3,
}

/// Predict aim (`predictAim`).
pub fn predict_aim(
    context: &mut MonsterContext,
    start: Vec3,
    speed: f64,
    eye: bool,
    offset: f64,
) -> Option<PredictedAim> {
    let enemy = enemy_body(context)?;
    let actor = context.actor().clone();
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    let enemy_id = context.entity().enemy.clone();
    let view_height = enemy_id
        .as_ref()
        .and_then(|enemy| context.game.entities.get(enemy))
        .map(|entity| entity.view_height)
        .unwrap_or(22);
    let mut aimed_eye = eye;
    let mut direction = sub3(
        vec3(
            enemy.origin.x,
            enemy.origin.y,
            enemy.origin.z + if aimed_eye { view_height as f32 } else { 0.0 },
        ),
        start,
    );
    if rerelease {
        let mask = attack_trace_mask(context.game);
        let trace = context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
            start,
            end: add3(start, direction),
            bounds: None,
            ignore: Some(actor),
            mask,
            exclude: Vec::new(),
        });
        let hit_enemy = matches!(&trace.hit, TraceHit::Actor { actor } if Some(actor) == enemy_id.as_ref());
        if !hit_enemy {
            aimed_eye = !aimed_eye;
            direction = sub3(
                vec3(
                    enemy.origin.x,
                    enemy.origin.y,
                    enemy.origin.z + if aimed_eye { view_height as f32 } else { 0.0 },
                ),
                start,
            );
        }
    }
    let time = if rerelease && speed == 0.0 {
        0.0
    } else {
        f64::from(length3(direction)) / speed
    };
    let mut target = add3(enemy.origin, scale3(enemy.velocity, (time - offset) as f32));
    if rerelease {
        let facing = dot3(
            normalize3(direction),
            normalize3(sub3(target, start)),
        );
        let blocked = context
            .game
            .host
            .trace(&crate::q2::foundation::host::Q2TraceRequest {
                start,
                end: target,
                bounds: None,
                ignore: None,
                mask: 3,
                exclude: Vec::new(),
            })
            .fraction
            < 0.9;
        if facing < 0.0 || blocked {
            target = enemy.origin;
        }
    }
    let point = vec3(
        target.x,
        target.y,
        target.z + if aimed_eye { view_height as f32 } else { 0.0 },
    );
    Some(PredictedAim {
        direction: normalize3(sub3(point, start)),
        point,
    })
}

/// Predicted direction (`predictedDirection`).
pub fn predicted_direction(
    context: &mut MonsterContext,
    start: Vec3,
    speed: f64,
    eye: bool,
    offset: f64,
) -> Option<Vec3> {
    predict_aim(context, start, speed, eye, offset).map(|aim| aim.direction)
}

/// Blocked platform check (`blockedCheckPlatform`).
pub fn blocked_check_platform(context: &mut MonsterContext, distance: f64) -> bool {
    let enemy = enemy_body(context);
    let Some(enemy) = enemy else { return false };
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    let above = enemy.origin.z + enemy.bounds.min.z >= body.origin.z + body.bounds.max.z;
    let below = enemy.origin.z + enemy.bounds.max.z <= body.origin.z + body.bounds.min.z;
    if !above && !below {
        return false;
    }
    let mut platform = body.ground.clone().filter(|ground| {
        context
            .game
            .entities
            .get(ground)
            .is_some_and(|entity| entity.classname.starts_with("func_plat"))
    });
    if platform.is_none() {
        let forward = angles_vectors(body.angles).forward;
        let start = add3(body.origin, scale3(forward, distance as f32));
        let mask = monster_solid_mask(context.game);
        let trace = context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
            start,
            end: add3(start, vec3(0.0, 0.0, -384.0)),
            bounds: None,
            ignore: Some(actor.clone()),
            mask,
            exclude: Vec::new(),
        });
        if trace.fraction < 1.0 && !trace.all_solid && !trace.start_solid {
            if let TraceHit::Actor { actor: hit } = &trace.hit {
                platform = Some(hit.clone());
            }
        }
    }
    let Some(platform) = platform else { return false };
    let platform_entity = context.game.entities.get(&platform).cloned();
    let Some(platform_entity) = platform_entity else { return false };
    if !platform_entity.classname.starts_with("func_plat") {
        return false;
    }
    let Some(use_callback) = platform_entity.use_ else {
        return false;
    };
    let phase = context.platform_state(&platform);
    let standing = body.ground.as_ref() == Some(&platform);
    let trigger = if above {
        standing && phase == Some(PlatformPhase::Bottom)
            || !standing && phase == Some(PlatformPhase::Top)
    } else {
        standing && phase == Some(PlatformPhase::Top)
            || !standing && phase == Some(PlatformPhase::Bottom)
    };
    if trigger {
        use_callback(platform, &mut *context.game, Some(actor.clone()), Some(actor));
        return true;
    }
    false
}

/// Jump finished (`monsterJumpFinished`).
pub fn monster_jump_finished(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    if context.game.options.edition == Q2Edition::Classic {
        return context.game.host.now() - context.entity().timestamp > 3.0;
    }
    let body = context.game.body_of(actor.clone());
    let forward = angles_vectors(body.angles).forward;
    let aligned = vec3(
        body.velocity.x * forward.x,
        body.velocity.y * forward.y,
        body.velocity.z * forward.z,
    );
    if length3(aligned) < 150.0 {
        let boosted = scale3(forward, 150.0);
        let mut moved = body;
        moved.velocity = vec3(boosted.x, boosted.y, moved.velocity.z);
        context.game.write_body(actor, &moved, true);
    }
    context.state().jump_time < context.game.host.now()
}

/// Jump navigation (`JumpNavigation`).
#[derive(Debug, Clone, PartialEq)]
pub enum JumpNavigation {
    /// No navigation.
    None,
    /// Path traversal.
    Path {
        /// Traversal pending.
        traversal_pending: bool,
        /// First point.
        first: Vec3,
        /// Second point.
        second: Vec3,
    },
}

/// Jump result (`JumpResult`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JumpResult {
    /// No jump.
    None,
    /// Jump up.
    Up,
    /// Jump down.
    Down,
    /// Turn first.
    Turn,
}

/// Blocked jump check (`blockedCheckJump`).
pub fn blocked_check_jump(
    context: &mut MonsterContext,
    _distance: f64,
    drop_height: f64,
    jump_height: f64,
    can_jump: bool,
    navigation: JumpNavigation,
) -> JumpResult {
    let enemy = enemy_body(context);
    let Some(enemy) = enemy else { return JumpResult::None };
    let rerelease = context.game.options.edition == Q2Edition::Rerelease;
    let actor = context.actor().clone();
    let body = context.game.body_of(actor);
    if rerelease && (!can_jump || context.state().jump_time > context.game.host.now()) {
        return JumpResult::None;
    }
    if rerelease {
        if let JumpNavigation::Path {
            traversal_pending,
            first,
            second,
        } = navigation
        {
            if !traversal_pending {
                return JumpResult::None;
            }
            let mut ideal = vector_angles(normalize3(sub3(first, second))).y + 180.0;
            if ideal > 360.0 {
                ideal -= 360.0;
            }
            context.state_mut().ideal_yaw = f64::from(ideal);
            let delta = ((body.angles.y - ideal) % 360.0 + 360.0) % 360.0;
            if delta > 45.0 && delta < 315.0 {
                change_yaw(context);
                return JumpResult::Turn;
            }
            finish_dodge(context);
            let now = context.game.host.now();
            context.state_mut().jump_time = now + 3.0;
            return if second.z > first.z {
                JumpResult::Up
            } else {
                JumpResult::Down
            };
        }
    }
    let min_z = f64::from(body.origin.z) + f64::from(body.bounds.min.z);
    let enemy_min = f64::from(enemy.origin.z) + f64::from(enemy.bounds.min.z);
    let forward = angles_vectors(body.angles).forward;
    let ahead = add3(body.origin, scale3(forward, 48.0));
    let step = if rerelease { 18.0 } else { 16.0 };
    let down = enemy_min < min_z - step;
    let up = enemy_min > min_z + step;
    if down && drop_height != 0.0 {
        let mask = monster_solid_mask(context.game);
        let blocked = context
            .game
            .host
            .trace(&crate::q2::foundation::host::Q2TraceRequest {
                start: body.origin,
                end: ahead,
                bounds: Some(body.bounds),
                ignore: Some(context.actor().clone()),
                mask,
                exclude: Vec::new(),
            })
            .fraction
            < 1.0;
        if blocked {
            return JumpResult::None;
        }
        let base = if rerelease {
            min_z
        } else {
            f64::from(body.bounds.min.z)
        };
        let end = vec3(ahead.x, ahead.y, (base - drop_height - 1.0) as f32);
        let mask = monster_solid_mask(context.game);
        let trace = context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
            start: ahead,
            end,
            bounds: None,
            ignore: Some(context.actor().clone()),
            mask: mask | 56,
            exclude: Vec::new(),
        });
        if trace.fraction == 1.0 || trace.all_solid || trace.start_solid {
            return JumpResult::None;
        }
        if rerelease && trace_contents(&trace) & 32 != 0 {
            let mask = monster_solid_mask(context.game);
            let deep = context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
                start: trace.end,
                end,
                bounds: None,
                ignore: Some(context.actor().clone()),
                mask,
                exclude: Vec::new(),
            });
            let probe = vec3(
                deep.end.x,
                deep.end.y,
                deep.end.z + body.bounds.min.z + 49.0,
            );
            if context.game.host.point_contents(probe) & 56 != 0 {
                return JumpResult::None;
            }
        }
        let solid = if rerelease { 35 } else { 3 };
        let plane_ok = matches!(&trace.contact, TraceContact::Plane { plane } if plane.normal.z >= 0.9);
        if min_z - f64::from(trace.end.z) < 24.0
            || trace_contents(&trace) & solid == 0
            || enemy_min - f64::from(trace.end.z) > 32.0
            || !plane_ok
        {
            return JumpResult::None;
        }
        if rerelease {
            finish_dodge(context);
            let now = context.game.host.now();
            context.state_mut().jump_time = now + 3.0;
        }
        return JumpResult::Down;
    }
    if up && jump_height != 0.0 {
        let mask = monster_solid_mask(context.game);
        let trace = context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
            start: vec3(
                ahead.x,
                ahead.y,
                body.origin.z + body.bounds.max.z + jump_height as f32,
            ),
            end: ahead,
            bounds: None,
            ignore: Some(context.actor().clone()),
            mask: mask | 56,
            exclude: Vec::new(),
        });
        let solid = if rerelease { 35 } else { 3 };
        if trace.fraction == 1.0
            || trace.all_solid
            || trace.start_solid
            || f64::from(trace.end.z) - min_z > jump_height
            || trace_contents(&trace) & solid == 0
        {
            return JumpResult::None;
        }
        let mask = monster_solid_mask(context.game);
        let wall = context.game.host.trace(&crate::q2::foundation::host::Q2TraceRequest {
            start: body.origin,
            end: add3(body.origin, scale3(forward, 64.0)),
            bounds: None,
            ignore: Some(context.actor().clone()),
            mask,
            exclude: Vec::new(),
        });
        if wall.fraction < 1.0 && !wall.all_solid && !wall.start_solid {
            if let TraceContact::Plane { plane } = &wall.contact {
                let mut ideal = vector_angles(plane.normal).y + 180.0;
                if ideal > 360.0 {
                    ideal -= 360.0;
                }
                context.state_mut().ideal_yaw = f64::from(ideal);
                change_yaw(context);
            }
        }
        if rerelease {
            finish_dodge(context);
            let now = context.game.host.now();
            context.state_mut().jump_time = now + 3.0;
        }
        return JumpResult::Up;
    }
    JumpResult::None
}
