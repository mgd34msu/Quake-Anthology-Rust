//! Travel controller from `src/bots/behavior/q3/travel/controller.ts`
//! (`be_ai_move.c`: `BotMoveToGoal`, `BotAvoidSpots`, `BotCheckBlocked`,
//! `BotCheckBarrierJump`, `BotOnMover`, `BotMoverDown`).
//!
//! `BotMoveToGoal` resolves the next reachability on the route and
//! dispatches to the ground or special follower; arrival within the
//! goal bounds (or the entity touch radius) reports success. Avoid
//! spots steer around timed hazards and blocked reachabilities feed
//! the avoid-reach list.

use qa_core::math::Vec3;

use crate::behavior::library::goals::{touching_goal, BotGoal};
use crate::behavior::q3::movement_state::{BotMoveResult, BotMoveResultFlag, BotMoveState};
use crate::behavior::q3::navigation_types::TravelType;
use crate::behavior::q3::travel::ground::{travel_ground, GroundReachability};
use crate::behavior::q3::travel::special::{travel_special, MoverObservation, SpecialReachability};

/// Reachability resolved for one move step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TravelStep {
    /// Travel type.
    pub travel_type: i32,
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Reachability number.
    pub number: i32,
    /// Mover entity, when bound.
    pub entity: i32,
    /// Travel time.
    pub travel_time: i32,
}

/// Move-to-goal outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TravelOutcome {
    /// Moving toward the goal.
    Moving,
    /// Goal reached.
    Arrived,
    /// No route.
    NoRoute,
    /// Blocked.
    Blocked,
}

/// Steer around avoid spots: returns a diverted direction when the
/// straight line crosses a spot.
#[must_use]
pub fn avoid_spots(state: &BotMoveState, direction: Vec3, spots: &[(Vec3, f32)]) -> Option<Vec3> {
    for (origin, radius) in spots {
        let to_spot = Vec3 {
            x: origin.x - state.origin.x,
            y: origin.y - state.origin.y,
            z: 0.0,
        };
        let along = to_spot.x * direction.x + to_spot.y * direction.y;
        if along > 0.0 && along < radius + 64.0 {
            let side = -to_spot.y * direction.x + to_spot.x * direction.y;
            let sign = if side >= 0.0 { 1.0 } else { -1.0 };
            let diverted = Vec3 {
                x: direction.x - direction.y * sign * 0.75,
                y: direction.y + direction.x * sign * 0.75,
                z: 0.0,
            };
            let len = (diverted.x * diverted.x + diverted.y * diverted.y).sqrt().max(1.0);
            return Some(Vec3 {
                x: diverted.x / len,
                y: diverted.y / len,
                z: 0.0,
            });
        }
    }
    None
}

/// Check whether movement is blocked: reports the blocking entity.
pub fn check_blocked(blocked: bool, block_entity: i32, result: &mut BotMoveResult) {
    result.blocked = blocked;
    result.block_entity = if blocked { block_entity } else { 0 };
}

/// Drive one move step toward a goal (`BotMoveToGoal`).
///
/// The caller supplies the next routed reachability (or `None` for a
/// straight steering fallback); this controller dispatches followers,
/// applies avoid spots, and classifies arrival.
pub fn move_to_goal(
    state: &mut BotMoveState,
    goal: &BotGoal,
    step: Option<TravelStep>,
    mover: &MoverObservation,
    travel_weapon: impl Fn() -> i32,
    avoid: &[(Vec3, f32)],
    time: f32,
    result: &mut BotMoveResult,
) -> TravelOutcome {
    *result = BotMoveResult::default();
    if touching_goal(state.origin, goal) {
        return TravelOutcome::Arrived;
    }
    let Some(step) = step else {
        // Straight steering fallback when no reachability resolved.
        let dir = Vec3 {
            x: goal.origin.x - state.origin.x,
            y: goal.origin.y - state.origin.y,
            z: 0.0,
        };
        let len = (dir.x * dir.x + dir.y * dir.y).sqrt();
        if len < 1.0 {
            return TravelOutcome::Arrived;
        }
        result.move_direction = Vec3 {
            x: dir.x / len,
            y: dir.y / len,
            z: 0.0,
        };
        result.travel_type = TravelType::WALK;
        return TravelOutcome::Moving;
    };
    if state
        .avoid_reach
        .iter()
        .any(|reach| *reach == step.number && step.number != 0)
    {
        result.failure = true;
        return TravelOutcome::Blocked;
    }
    let ground = GroundReachability {
        travel_type: step.travel_type,
        start: step.start,
        end: step.end,
        travel_time: step.travel_time,
        entity: step.entity,
    };
    if !travel_ground(state, &ground, result) {
        let special = SpecialReachability {
            travel_type: step.travel_type,
            start: step.start,
            end: step.end,
            entity: step.entity,
        };
        if !travel_special(state, &special, mover, travel_weapon, result) {
            result.failure = true;
            return TravelOutcome::NoRoute;
        }
    }
    if let Some(diverted) = avoid_spots(state, result.move_direction, avoid) {
        result.move_direction = diverted;
        result.flags |= BotMoveResultFlag::BLOCKEDBYAVOIDSPOT;
    }
    let _ = time;
    TravelOutcome::Moving
}

/// Record a blocked reachability in the avoid-reach list.
pub fn note_blocked_reach(state: &mut BotMoveState, number: i32, time: f32) {
    for index in 0..state.avoid_reach.len() {
        if state.avoid_reach[index] == 0 || state.avoid_reach_times[index] < time {
            state.avoid_reach[index] = number;
            state.avoid_reach_times[index] = time + 2.0;
            state.avoid_reach_tries[index] += 1;
            return;
        }
    }
}
