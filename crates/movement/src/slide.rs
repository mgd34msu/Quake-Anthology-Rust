//! Shared bounded plane clipping; native differences are selected as rules data.
use crate::physics::Step;
use qa_core::{
    math,
    primitives::{MovementRules, Vec3},
};

pub(crate) fn clip(velocity: Vec3, normal: Vec3, rules: MovementRules) -> Vec3 {
    let dot = velocity.dot(normal);
    let backoff = if rules == MovementRules::Quake3 {
        if dot < 0.0 { dot * 1.001 } else { dot / 1.001 }
    } else {
        dot * if matches!(
            rules,
            MovementRules::Quake2 | MovementRules::Quake2Rerelease
        ) {
            1.01
        } else {
            1.0
        }
    };
    let mut out = velocity - normal * backoff;
    if rules != MovementRules::Quake3 {
        for value in &mut out.0 {
            if *value > -0.1 && *value < 0.1 {
                *value = 0.0;
            }
        }
    }
    out
}
fn legacy_slide(step: &mut Step<'_>) -> u8 {
    let rules = step.parameters.rules;
    let primal = step.player.body.velocity;
    let mut original = primal;
    let mut planes = [Vec3::default(); 5];
    let mut count = 0;
    let mut remaining = step.dt;
    let mut blocked = 0;
    for _ in 0..4 {
        if step.nq() && step.player.body.velocity == Vec3::default() {
            break;
        }
        let start = step.player.body.position;
        let end = start + step.player.body.velocity * remaining;
        let tr = step.trace(start, end);
        if tr.all_solid || step.qw() && tr.start_solid {
            if step.classic() || step.rr() {
                step.player.body.velocity.0[2] = 0.0;
            } else {
                step.player.body.velocity = Vec3::default();
            }
            return 3;
        }
        if tr.fraction > 0.0 {
            step.player.body.position = tr.end;
            count = 0;
            if step.nq() {
                original = step.player.body.velocity;
            }
        }
        if tr.fraction == 1.0 {
            break;
        }
        step.contact(&tr);
        if tr.plane.normal.0[2] > 0.7 {
            blocked |= 1;
            if step.nq() && tr.brush_solid {
                step.player.movement.grounded = true;
                step.player.movement.ground = tr.entity;
                step.player.movement.ground_normal = tr.plane.normal;
                step.player.movement.ground_surface = tr.surface;
            }
        }
        if tr.plane.normal.0[2] == 0.0 {
            blocked |= 2;
        }
        remaining -= remaining * tr.fraction;
        if count == 5 {
            step.player.body.velocity = Vec3::default();
            return 3;
        }
        if step.rr()
            && planes[..count]
                .iter()
                .any(|&p| p.dot(tr.plane.normal) > 0.99)
        {
            step.player.body.position.0[0] += tr.plane.normal.0[0] * 0.01;
            step.player.body.position.0[1] += tr.plane.normal.0[1] * 0.01;
            continue;
        }
        planes[count] = tr.plane.normal;
        count += 1;
        let mut accepted = false;
        let mut candidate = step.player.body.velocity;
        for i in 0..count {
            candidate = clip(
                if step.classic() || step.rr() {
                    candidate
                } else {
                    original
                },
                planes[i],
                rules,
            );
            if (0..count).all(|j| i == j || candidate.dot(planes[j]) >= 0.0) {
                accepted = true;
                break;
            }
        }
        if !accepted {
            if count != 2 {
                step.player.body.velocity = Vec3::default();
                return 7;
            }
            let direction = math::cross(planes[0], planes[1]);
            candidate = direction
                * direction.dot(if step.nq() {
                    step.player.body.velocity
                } else {
                    candidate
                });
        }
        step.player.body.velocity = candidate;
        if candidate.dot(primal) <= 0.0 {
            step.player.body.velocity = Vec3::default();
            break;
        }
    }
    if step.qw() && step.player.movement.water_jump_seconds != 0.0
        || !step.nq() && !step.qw() && step.player.movement.remaining_ms != 0
    {
        step.player.body.velocity = primal;
    }
    blocked
}
fn arena_slide(step: &mut Step<'_>, gravity: bool) -> u8 {
    let mut primal = step.player.body.velocity;
    let mut end_velocity = primal;
    if gravity {
        end_velocity.0[2] -= step.parameters.gravity * step.dt;
        step.player.body.velocity.0[2] = (step.player.body.velocity.0[2] + end_velocity.0[2]) * 0.5;
        primal.0[2] = end_velocity.0[2];
        if step.ground_plane {
            step.player.body.velocity = clip(
                step.player.body.velocity,
                step.ground_normal,
                MovementRules::Quake3,
            );
        }
    }
    let mut planes = [Vec3::default(); 5];
    let mut count = 0;
    if step.ground_plane {
        planes[count] = step.ground_normal;
        count += 1;
    }
    planes[count] = math::normalized(step.player.body.velocity);
    count += 1;
    let mut remaining = step.dt;
    let mut blocked = 0;
    for _ in 0..4 {
        let start = step.player.body.position;
        let tr = step.trace(start, start + step.player.body.velocity * remaining);
        if tr.all_solid {
            step.player.body.velocity.0[2] = 0.0;
            return 3;
        }
        if tr.fraction > 0.0 {
            step.player.body.position = tr.end;
        }
        if tr.fraction == 1.0 {
            break;
        }
        blocked |= 2;
        step.contact(&tr);
        remaining -= remaining * tr.fraction;
        if count == 5 {
            step.player.body.velocity = Vec3::default();
            return 3;
        }
        let normal = tr.plane.normal;
        if planes[..count].iter().any(|&p| normal.dot(p) > 0.99) {
            step.player.body.velocity = step.player.body.velocity + normal;
            continue;
        }
        planes[count] = normal;
        count += 1;
        for i in 0..count {
            if step.player.body.velocity.dot(planes[i]) >= 0.1 {
                continue;
            }
            let mut candidate = clip(step.player.body.velocity, planes[i], MovementRules::Quake3);
            let mut end_candidate = clip(end_velocity, planes[i], MovementRules::Quake3);
            for j in 0..count {
                if j == i || candidate.dot(planes[j]) >= 0.1 {
                    continue;
                }
                candidate = clip(candidate, planes[j], MovementRules::Quake3);
                end_candidate = clip(end_candidate, planes[j], MovementRules::Quake3);
                if candidate.dot(planes[i]) >= 0.0 {
                    continue;
                }
                let direction = math::normalized(math::cross(planes[i], planes[j]));
                candidate = direction * direction.dot(step.player.body.velocity);
                end_candidate = direction * direction.dot(end_velocity);
                if (0..count).any(|k| k != i && k != j && candidate.dot(planes[k]) < 0.1) {
                    step.player.body.velocity = Vec3::default();
                    return 3;
                }
            }
            step.player.body.velocity = candidate;
            end_velocity = end_candidate;
            break;
        }
    }
    if gravity {
        step.player.body.velocity = end_velocity;
    }
    if step.player.movement.remaining_ms != 0 {
        step.player.body.velocity = primal;
    }
    blocked
}
pub(crate) fn slide(step: &mut Step<'_>, gravity: bool) -> u8 {
    if step.arena() {
        arena_slide(step, gravity)
    } else {
        legacy_slide(step)
    }
}
fn horizontal_distance(a: Vec3, b: Vec3) -> f32 {
    let delta = a - b;
    delta.0[0] * delta.0[0] + delta.0[1] * delta.0[1]
}

pub(crate) fn step_slide(step: &mut Step<'_>, gravity: bool) {
    let start = step.player.body.position;
    let start_velocity = step.player.body.velocity;
    let old_ground = step.player.movement.grounded;
    if step.nq() {
        step.player.movement.grounded = false;
        step.player.movement.ground = None;
    }
    if step.qw() && old_ground {
        step.player.body.velocity.0[2] = 0.0;
        let tr = step.trace(start, start + step.player.body.velocity * step.dt);
        if tr.fraction == 1.0 {
            step.player.body.position = tr.end;
            return;
        }
    }
    let blocked = slide(step, gravity);
    if step.parameters.no_step || step.qw() && !old_ground {
        return;
    }
    if step.nq() && (blocked & 2 == 0 || !old_ground && step.player.movement.water_level == 0) {
        return;
    }
    if step.arena() && blocked == 0 {
        return;
    }
    if step.arena() {
        let ground = step.trace(start, start + Vec3([0.0, 0.0, -18.0]));
        if step.player.body.velocity.0[2] > 0.0
            && (ground.fraction == 1.0 || ground.plane.normal.0[2] < 0.7)
        {
            return;
        }
    }
    let down = step.player.body.position;
    let down_velocity = step.player.body.velocity;
    let up = start + Vec3([0.0, 0.0, 18.0]);
    let raised = if step.classic() {
        step.trace(up, up)
    } else {
        step.trace(start, up)
    };
    if raised.all_solid {
        return;
    }
    step.player.body.position = if step.classic() { up } else { raised.end };
    step.player.body.velocity = start_velocity;
    if step.nq() {
        step.player.body.velocity.0[2] = 0.0;
    }
    let height = step.player.body.position.0[2] - start.0[2];
    slide(step, gravity);
    let mut drop = step.player.body.position;
    drop.0[2] -= if step.nq() {
        18.0 - start_velocity.0[2] * step.dt
    } else if step.classic() || step.qw() {
        18.0
    } else {
        height
    };
    let dropped = step.trace(step.player.body.position, drop);
    if !dropped.all_solid {
        step.player.body.position = dropped.end;
    }
    if step.arena() {
        if dropped.fraction < 1.0 {
            step.player.body.velocity = clip(
                step.player.body.velocity,
                dropped.plane.normal,
                MovementRules::Quake3,
            );
        }
        return;
    }
    if dropped.plane.normal.0[2] < 0.7
        || !step.nq()
            && horizontal_distance(down, start)
                > horizontal_distance(step.player.body.position, start)
    {
        step.player.body.position = down;
        step.player.body.velocity = down_velocity;
    } else if !step.nq() {
        step.player.body.velocity.0[2] = down_velocity.0[2];
    }
}
