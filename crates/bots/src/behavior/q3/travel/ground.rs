//! Ground travel from `src/bots/behavior/q3/travel/ground.ts`
//! (`be_ai_move.c`: `BotTravel_Walk`, `BotTravel_Crouch`,
//! `BotTravel_Jump`, `BotTravel_Ladder`, `BotTravel_Swim`,
//! `BotTravel_WalkOffLedge`, `BotTravel_BarrierJump`).
//!
//! Reachability followers for ground movement: each computes a move
//! direction, speed, and view target from the reachability endpoints
//! and the bot's current origin.

use qa_core::math::Vec3;

use crate::behavior::q3::movement_state::{BotMoveResult, BotMoveResultFlag, BotMoveState};
use crate::behavior::q3::navigation_types::TravelType;

/// Reachability endpoints for ground travel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundReachability {
    /// Travel type.
    pub travel_type: i32,
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Travel time.
    pub travel_time: i32,
    /// Entity number, when mover-bound.
    pub entity: i32,
}

/// Walk toward the reachability end at run speed.
pub fn travel_walk(state: &BotMoveState, reach: &GroundReachability, result: &mut BotMoveResult) {
    let dir = Vec3 {
        x: reach.end.x - state.origin.x,
        y: reach.end.y - state.origin.y,
        z: 0.0,
    };
    let len = (dir.x * dir.x + dir.y * dir.y).sqrt().max(1.0);
    result.move_direction = Vec3 {
        x: dir.x / len,
        y: dir.y / len,
        z: 0.0,
    };
    result.travel_type = TravelType::WALK;
    result.ideal_view_angles = Vec3 {
        x: 0.0,
        y: dir.y.atan2(dir.x).to_degrees(),
        z: 0.0,
    };
    result.flags |= BotMoveResultFlag::MOVEMENTVIEWSET;
}

/// Crouch toward the reachability end.
pub fn travel_crouch(state: &BotMoveState, reach: &GroundReachability, result: &mut BotMoveResult) {
    travel_walk(state, reach, result);
    result.travel_type = TravelType::CROUCH;
}

/// Jump toward the reachability end.
pub fn travel_jump(state: &BotMoveState, reach: &GroundReachability, result: &mut BotMoveResult) {
    travel_walk(state, reach, result);
    result.travel_type = TravelType::JUMP;
}

/// Climb a ladder: push into the wall and up.
pub fn travel_ladder(state: &BotMoveState, reach: &GroundReachability, result: &mut BotMoveResult) {
    let dir = Vec3 {
        x: reach.end.x - state.origin.x,
        y: reach.end.y - state.origin.y,
        z: reach.end.z - state.origin.z,
    };
    let len = (dir.x * dir.x + dir.y * dir.y + dir.z * dir.z).sqrt().max(1.0);
    result.move_direction = Vec3 {
        x: dir.x / len,
        y: dir.y / len,
        z: dir.z / len,
    };
    result.travel_type = TravelType::LADDER;
}

/// Swim toward the reachability end.
pub fn travel_swim(state: &BotMoveState, reach: &GroundReachability, result: &mut BotMoveResult) {
    travel_ladder(state, reach, result);
    result.travel_type = TravelType::SWIM;
    result.flags |= BotMoveResultFlag::SWIMVIEW;
}

/// Walk off a ledge toward the landing.
pub fn travel_walk_off_ledge(state: &BotMoveState, reach: &GroundReachability, result: &mut BotMoveResult) {
    travel_walk(state, reach, result);
    result.travel_type = TravelType::WALKOFFLEDGE;
}

/// Barrier jump: run at the barrier and jump at the edge.
pub fn travel_barrier_jump(state: &BotMoveState, reach: &GroundReachability, result: &mut BotMoveResult) {
    travel_walk(state, reach, result);
    result.travel_type = TravelType::BARRIERJUMP;
}

/// Dispatch ground travel by travel type. Returns whether handled.
pub fn travel_ground(state: &BotMoveState, reach: &GroundReachability, result: &mut BotMoveResult) -> bool {
    match reach.travel_type {
        t if t == TravelType::WALK => travel_walk(state, reach, result),
        t if t == TravelType::CROUCH => travel_crouch(state, reach, result),
        t if t == TravelType::JUMP => travel_jump(state, reach, result),
        t if t == TravelType::LADDER => travel_ladder(state, reach, result),
        t if t == TravelType::SWIM => travel_swim(state, reach, result),
        t if t == TravelType::WALKOFFLEDGE => travel_walk_off_ledge(state, reach, result),
        t if t == TravelType::BARRIERJUMP => travel_barrier_jump(state, reach, result),
        _ => return false,
    }
    true
}

/// Gap distance along a direction before a drop (`GapDistance`).
#[must_use]
pub fn gap_distance(origin: Vec3, direction: Vec3, ground_height: impl Fn(Vec3) -> Option<f32>) -> f32 {
    let mut distance = 0.0f32;
    while distance < 128.0 {
        let probe = Vec3 {
            x: origin.x + direction.x * distance,
            y: origin.y + direction.y * distance,
            z: origin.z,
        };
        match ground_height(probe) {
            Some(height) if origin.z - height < 48.0 => distance += 8.0,
            _ => break,
        }
    }
    distance
}
