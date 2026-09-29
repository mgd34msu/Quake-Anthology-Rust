//! Travel routing from `src/bots/behavior/q3/travel/routing.ts`
//! (`be_ai_move.c`: `BotMoveInDirection`, `BotMovementViewTarget`,
//! `BotPredictVisiblePosition`, `BotReachabilityArea`).
//!
//! Direction movement and view-target helpers shared by the travel
//! controller and the game AI: steering, lookahead views, and visible
//! position prediction along a route.

use qa_core::math::Vec3;

use crate::behavior::library::goals::BotGoal;
use crate::behavior::q3::movement_state::{BotMoveResult, BotMoveState};

/// Move in a direction at a speed (`BotMoveInDirection`).
pub fn move_in_direction(state: &BotMoveState, direction: Vec3, speed: f32, result: &mut BotMoveResult) -> bool {
    let len = (direction.x * direction.x + direction.y * direction.y).sqrt();
    if len < 0.001 {
        return false;
    }
    result.move_direction = Vec3 {
        x: direction.x / len,
        y: direction.y / len,
        z: direction.z,
    };
    result.ideal_view_angles = Vec3 {
        x: 0.0,
        y: direction.y.atan2(direction.x).to_degrees(),
        z: 0.0,
    };
    let _ = (state, speed);
    true
}

/// View target with lookahead toward a goal (`BotMovementViewTarget`).
#[must_use]
pub fn movement_view_target(origin: Vec3, goal: &BotGoal, look_ahead: f32) -> Option<Vec3> {
    let dir = Vec3 {
        x: goal.origin.x - origin.x,
        y: goal.origin.y - origin.y,
        z: goal.origin.z - origin.z,
    };
    let len = (dir.x * dir.x + dir.y * dir.y + dir.z * dir.z).sqrt();
    if len < 1.0 {
        return None;
    }
    let ahead = look_ahead.min(len);
    Some(Vec3 {
        x: origin.x + dir.x / len * ahead,
        y: origin.y + dir.y / len * ahead,
        z: origin.z + dir.z / len * ahead,
    })
}

/// Predict a visible position toward a goal (`BotPredictVisiblePosition`).
#[must_use]
pub fn predict_visible_position(origin: Vec3, goal: &BotGoal, visible: &dyn Fn(Vec3, Vec3) -> bool) -> Option<Vec3> {
    let mut best: Option<Vec3> = None;
    for fraction in [1.0, 0.75, 0.5, 0.25] {
        let probe = Vec3 {
            x: origin.x + (goal.origin.x - origin.x) * fraction,
            y: origin.y + (goal.origin.y - origin.y) * fraction,
            z: origin.z + (goal.origin.z - origin.z) * fraction,
        };
        if visible(origin, probe) {
            best = Some(probe);
            break;
        }
    }
    best
}

/// Jump launch velocity for a start/end pair under gravity
/// (`BotJumpSpeed`): returns the horizontal speed for the arc.
#[must_use]
pub fn jump_speed(start: Vec3, end: Vec3, initial_vertical_velocity: f32, gravity: f32) -> Option<f32> {
    let dz = end.z - start.z;
    let gravity = gravity.max(1.0);
    // Time to apex and fall: solve for the arc duration.
    let discriminant = initial_vertical_velocity * initial_vertical_velocity - 2.0 * gravity * dz;
    if discriminant < 0.0 {
        return None;
    }
    let time = (initial_vertical_velocity + discriminant.sqrt()) / gravity;
    if time <= 0.0 {
        return None;
    }
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let horizontal = (dx * dx + dy * dy).sqrt();
    Some(horizontal / time)
}

/// Air control steering toward a goal (`BotAirControl`).
#[must_use]
pub fn air_control(origin: Vec3, velocity: Vec3, goal: Vec3) -> (bool, Vec3, f32) {
    let dir = Vec3 {
        x: goal.x - origin.x,
        y: goal.y - origin.y,
        z: 0.0,
    };
    let len = (dir.x * dir.x + dir.y * dir.y).sqrt();
    if len < 1.0 {
        return (false, velocity, 0.0);
    }
    (
        true,
        Vec3 {
            x: dir.x / len,
            y: dir.y / len,
            z: 0.0,
        },
        400.0,
    )
}
