//! Q2 landmark placement (`src/content/q2/base/player/landmarks.ts`).

use qa_core::identity::ActorId;
use qa_core::math::{add3, dot3, scale3, sub3, vec3, Bounds, Vec3};

use crate::q2::foundation::host::{Q2GameServices, Q2LandmarkCarry, Q2TraceRequest};

/// Stuck-probe side.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Side {
    /// Axis: 0 = x, 1 = y, 2 = z.
    axis: usize,
    /// Sign.
    sign: f32,
}

/// Probe sides in source order.
const SIDES: &[Side] = &[
    Side { axis: 2, sign: 1.0 },
    Side { axis: 2, sign: -1.0 },
    Side { axis: 0, sign: 1.0 },
    Side { axis: 0, sign: -1.0 },
    Side { axis: 1, sign: 1.0 },
    Side { axis: 1, sign: -1.0 },
];

/// Read a vector component.
fn component(vector: Vec3, axis: usize) -> f32 {
    match axis {
        0 => vector.x,
        1 => vector.y,
        _ => vector.z,
    }
}

/// Set a vector component (`axisSet`).
fn set_component(vector: Vec3, axis: usize, value: f32) -> Vec3 {
    match axis {
        0 => vec3(value, vector.y, vector.z),
        1 => vec3(vector.x, value, vector.z),
        _ => vec3(vector.x, vector.y, value),
    }
}

/// Rotate a landmark vector (`rotateQ2Landmark`).
pub fn rotate_q2_landmark(vector: Vec3, angles: Vec3) -> Vec3 {
    let pitch = f64::from(angles.x) * std::f64::consts::PI / 180.0;
    let roll = f64::from(angles.z) * std::f64::consts::PI / 180.0;
    let yaw = f64::from(angles.y) * std::f64::consts::PI / 180.0;
    let (vx, vy, vz) = (f64::from(vector.x), f64::from(vector.y), f64::from(vector.z));
    let x = vec3(
        vx as f32,
        (vy * pitch.cos() - vz * pitch.sin()) as f32,
        (vy * pitch.sin() + vz * pitch.cos()) as f32,
    );
    let y = vec3(
        (f64::from(x.x) * roll.cos() + f64::from(x.z) * roll.sin()) as f32,
        x.y,
        (-f64::from(x.x) * roll.sin() + f64::from(x.z) * roll.cos()) as f32,
    );
    vec3(
        (f64::from(y.x) * yaw.cos() - f64::from(y.y) * yaw.sin()) as f32,
        (f64::from(y.x) * yaw.sin() + f64::from(y.y) * yaw.cos()) as f32,
        y.z,
    )
}

/// Free a stuck player (`fixQ2StuckPlayer`).
///
/// Source p_move.cpp face probes, including its final-unsorted-candidate quirk.
pub fn fix_q2_stuck_player(actor: ActorId, game: &mut Q2GameServices, origin: Vec3, bounds: Bounds) -> Option<Vec3> {
    let trace = |game: &mut Q2GameServices, start: Vec3, shape: Bounds, end: Vec3| {
        game.host.trace(&Q2TraceRequest {
            start,
            end,
            bounds: Some(shape),
            ignore: Some(actor.clone()),
            mask: 0x2010003,
            exclude: Vec::new(),
        })
    };
    if !trace(game, origin, bounds, origin).start_solid {
        return Some(origin);
    }
    let mut good: Vec<(Vec3, f32)> = Vec::new();
    for side in SIDES {
        let facing = if side.sign < 0.0 { bounds.min } else { bounds.max };
        let mut start = set_component(
            origin,
            side.axis,
            component(origin, side.axis) + component(facing, side.axis),
        );
        let face = Bounds {
            min: set_component(bounds.min, side.axis, 0.0),
            max: set_component(bounds.max, side.axis, 0.0),
        };
        let mut hit = trace(game, start, face, start);
        let mut epsilon: Option<(usize, f32)> = None;
        if hit.start_solid {
            for axis in 0..3 {
                if axis == side.axis {
                    continue;
                }
                let positive = set_component(start, axis, component(start, axis) + 1.0);
                let first = trace(game, positive, face, positive);
                if !first.start_solid {
                    start = positive;
                    hit = first;
                    epsilon = Some((axis, 1.0));
                    break;
                }
                let negative = set_component(start, axis, component(start, axis) - 1.0);
                let second = trace(game, negative, face, negative);
                if !second.start_solid {
                    start = negative;
                    hit = second;
                    epsilon = Some((axis, -1.0));
                    break;
                }
            }
        }
        if hit.start_solid {
            continue;
        }
        let facing = if side.sign < 0.0 { bounds.max } else { bounds.min };
        let mut opposite = set_component(
            origin,
            side.axis,
            component(origin, side.axis) + component(facing, side.axis),
        );
        if let Some((axis, distance)) = epsilon {
            opposite = set_component(opposite, axis, component(opposite, axis) + distance);
        }
        hit = trace(game, start, face, opposite);
        if hit.start_solid {
            continue;
        }
        let normal = set_component(vec3(0.0, 0.0, 0.0), side.axis, side.sign);
        let delta = sub3(add3(hit.end, scale3(normal, 0.125)), opposite);
        let mut position = add3(origin, delta);
        if let Some((axis, distance)) = epsilon {
            position = set_component(position, axis, component(position, axis) + distance);
        }
        if !trace(game, position, bounds, position).start_solid {
            good.push((position, dot3(delta, delta)));
        }
    }
    if good.len() > 1 {
        let last = good.pop();
        good.sort_by(|left, right| left.1.partial_cmp(&right.1).unwrap_or(std::cmp::Ordering::Equal));
        if let Some(last) = last {
            good.push(last);
        }
    }
    good.first().map(|(origin, _)| *origin)
}

/// Landmark placement (`Q2LandmarkPlacement`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2LandmarkPlacement {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Angles.
    pub angles: Vec3,
}

/// Place a landmark arrival (`placeQ2Landmark`).
pub fn place_q2_landmark(
    actor: ActorId,
    game: &mut Q2GameServices,
    carry: &Q2LandmarkCarry,
    spawn: ActorId,
    bounds: Bounds,
) -> Option<Q2LandmarkPlacement> {
    if carry.name.is_empty() {
        return None;
    }
    let landmark = game.pick_target(&carry.name)?;
    let reference = game.body_of(landmark.clone());
    let mut origin = add3(
        rotate_q2_landmark(carry.relative_origin, reference.angles),
        reference.origin,
    );
    // This is source bit 0, also authored by the shipped rerelease BSPs.
    if game.require_entity(&landmark).spawnflags & 1 != 0 {
        origin = vec3(origin.x, origin.y, game.body_of(spawn).origin.z);
    }
    let clear = fix_q2_stuck_player(actor, game, origin, bounds)?;
    Some(Q2LandmarkPlacement {
        origin: clear,
        velocity: rotate_q2_landmark(carry.relative_velocity, reference.angles),
        angles: add3(carry.relative_view_angles, reference.angles),
    })
}
