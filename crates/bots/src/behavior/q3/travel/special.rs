//! Special travel from `src/bots/behavior/q3/travel/special.ts`
//! (`be_ai_move.c`: `BotTravel_Elevator`, `BotTravel_FuncBobbing`,
//! `BotTravel_Teleport`, `BotTravel_JumpPad`, `BotTravel_RocketJump`,
//! `BotTravel_BFGJump`, `BotTravel_Grapple`, `BotTravel_Door`,
//! `BotTravel_Train`).
//!
//! Mover-bound and weapon-assisted reachability followers. Elevators
//! and bobbers wait for the platform; teleporters and jump pads run
//! through; rocket/BFG jumps and the grapple arm the travel weapon
//! and aim at the launch point.

use qa_core::math::Vec3;

use crate::behavior::q3::movement_state::{BotMoveResult, BotMoveResultFlag, BotMoveResultType, BotMoveState};
use crate::behavior::q3::navigation_types::TravelType;
use crate::behavior::q3::travel::types::BotTravelModel;

/// Special reachability endpoints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpecialReachability {
    /// Travel type.
    pub travel_type: i32,
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Mover entity, when bound.
    pub entity: i32,
}

/// Mover state observed by special travel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoverObservation {
    /// Model info.
    pub model: Option<BotTravelModel>,
    /// Whether the bot stands on the mover.
    pub on_mover: bool,
    /// Whether the mover is at its down position.
    pub mover_down: bool,
}

/// Ride an elevator: wait for the platform, then ride to the top.
pub fn travel_elevator(
    state: &BotMoveState,
    reach: &SpecialReachability,
    mover: &MoverObservation,
    result: &mut BotMoveResult,
) {
    result.travel_type = TravelType::ELEVATOR;
    if !mover.on_mover {
        let dir = Vec3 {
            x: reach.start.x - state.origin.x,
            y: reach.start.y - state.origin.y,
            z: 0.0,
        };
        let len = (dir.x * dir.x + dir.y * dir.y).sqrt().max(1.0);
        result.move_direction = Vec3 {
            x: dir.x / len,
            y: dir.y / len,
            z: 0.0,
        };
        result.flags |= BotMoveResultFlag::WAITING;
        result.result_type = BotMoveResultType::ELEVATORUP;
    } else {
        result.move_direction = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        result.flags |= BotMoveResultFlag::ONTOPOF_ELEVATOR;
    }
}

/// Ride a func_bobbing platform.
pub fn travel_func_bobbing(
    state: &BotMoveState,
    reach: &SpecialReachability,
    mover: &MoverObservation,
    result: &mut BotMoveResult,
) {
    result.travel_type = TravelType::FUNCBOB;
    if !mover.on_mover {
        travel_elevator(state, reach, mover, result);
        result.travel_type = TravelType::FUNCBOB;
        result.result_type = BotMoveResultType::WAITFORFUNCBOBBING;
    } else {
        result.flags |= BotMoveResultFlag::ONTOPOF_FUNCBOB;
    }
}

/// Run through a teleporter.
pub fn travel_teleport(state: &BotMoveState, reach: &SpecialReachability, result: &mut BotMoveResult) {
    let dir = Vec3 {
        x: reach.start.x - state.origin.x,
        y: reach.start.y - state.origin.y,
        z: 0.0,
    };
    let len = (dir.x * dir.x + dir.y * dir.y).sqrt().max(1.0);
    result.move_direction = Vec3 {
        x: dir.x / len,
        y: dir.y / len,
        z: 0.0,
    };
    result.travel_type = TravelType::TELEPORT;
}

/// Run onto a jump pad.
pub fn travel_jump_pad(state: &BotMoveState, reach: &SpecialReachability, result: &mut BotMoveResult) {
    travel_teleport(state, reach, result);
    result.travel_type = TravelType::JUMPPAD;
}

/// Rocket jump: aim down at the launch point and arm the weapon.
pub fn travel_rocket_jump(state: &BotMoveState, reach: &SpecialReachability, weapon: i32, result: &mut BotMoveResult) {
    let dir = Vec3 {
        x: reach.start.x - state.origin.x,
        y: reach.start.y - state.origin.y,
        z: 0.0,
    };
    let len = (dir.x * dir.x + dir.y * dir.y).sqrt().max(1.0);
    result.move_direction = Vec3 {
        x: dir.x / len,
        y: dir.y / len,
        z: 0.0,
    };
    result.travel_type = TravelType::ROCKETJUMP;
    result.weapon = weapon;
    result.ideal_view_angles = Vec3 {
        x: 80.0,
        y: dir.y.atan2(dir.x).to_degrees(),
        z: 0.0,
    };
    result.flags |= BotMoveResultFlag::MOVEMENTVIEWSET | BotMoveResultFlag::MOVEMENTWEAPON;
}

/// BFG jump: same shape as a rocket jump with the BFG armed.
pub fn travel_bfg_jump(state: &BotMoveState, reach: &SpecialReachability, weapon: i32, result: &mut BotMoveResult) {
    travel_rocket_jump(state, reach, weapon, result);
    result.travel_type = TravelType::BFGJUMP;
}

/// Grapple: aim at the grapple endpoint and arm the hook.
pub fn travel_grapple(state: &BotMoveState, reach: &SpecialReachability, weapon: i32, result: &mut BotMoveResult) {
    let dir = Vec3 {
        x: reach.end.x - state.origin.x,
        y: reach.end.y - state.origin.y,
        z: reach.end.z - state.origin.z,
    };
    let horizontal = (dir.x * dir.x + dir.y * dir.y).sqrt().max(1.0);
    result.move_direction = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    result.travel_type = TravelType::GRAPPLEHOOK;
    result.weapon = weapon;
    result.ideal_view_angles = Vec3 {
        x: (-dir.z / horizontal).atan().to_degrees(),
        y: dir.y.atan2(dir.x).to_degrees(),
        z: 0.0,
    };
    result.flags |= BotMoveResultFlag::MOVEMENTVIEWSET | BotMoveResultFlag::MOVEMENTWEAPON;
}

/// Open a door by walking into its trigger.
pub fn travel_door(state: &BotMoveState, reach: &SpecialReachability, result: &mut BotMoveResult) {
    travel_teleport(state, reach, result);
    result.travel_type = TravelType::WALK;
}

/// Board a train: wait at the platform, then ride.
pub fn travel_train(
    state: &BotMoveState,
    reach: &SpecialReachability,
    mover: &MoverObservation,
    result: &mut BotMoveResult,
) {
    travel_elevator(state, reach, mover, result);
    result.travel_type = TravelType::ELEVATOR;
}

/// Dispatch special travel by travel type. Returns whether handled.
pub fn travel_special(
    state: &BotMoveState,
    reach: &SpecialReachability,
    mover: &MoverObservation,
    travel_weapon: impl Fn() -> i32,
    result: &mut BotMoveResult,
) -> bool {
    match reach.travel_type {
        t if t == TravelType::ELEVATOR => travel_elevator(state, reach, mover, result),
        t if t == TravelType::FUNCBOB => travel_func_bobbing(state, reach, mover, result),
        t if t == TravelType::TELEPORT => travel_teleport(state, reach, result),
        t if t == TravelType::JUMPPAD => travel_jump_pad(state, reach, result),
        t if t == TravelType::ROCKETJUMP => travel_rocket_jump(state, reach, travel_weapon(), result),
        t if t == TravelType::BFGJUMP => travel_bfg_jump(state, reach, travel_weapon(), result),
        t if t == TravelType::GRAPPLEHOOK => travel_grapple(state, reach, travel_weapon(), result),
        _ => return false,
    }
    true
}
