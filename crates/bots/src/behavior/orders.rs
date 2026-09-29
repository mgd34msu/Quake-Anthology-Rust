//! Scripted bot orders from `src/bots/behavior/orders.ts`.
//!
//! Orders are the observation-boundary goal contract: a move-to-point goal
//! or a follow-entity goal guarded by an entity generation so a bot never
//! follows a recycled slot. Status values match the shipped QuakeC
//! `BOT_GOAL_*` contract (0 idle/error, 1 reached, 2 in progress).

use qa_core::math::Vec3;

/// Entity reference bound to an observation generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotOrderEntity {
    /// Observation entity number.
    pub number: i32,
    /// Generation of the observed entity.
    pub generation: i32,
}

/// Scripted bot goal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BotOrder {
    /// Move to a world point.
    Point {
        /// Destination.
        point: Vec3,
    },
    /// Follow an observed entity.
    Follow {
        /// Entity to follow.
        entity: BotOrderEntity,
    },
}

/// Scripted goal progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotOrderProgress {
    /// Driving toward the goal.
    InProgress,
    /// Goal reached.
    Success,
    /// Goal failed or was invalidated.
    Error,
}

/// Scripted goal plus its progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotOrderState {
    /// Ordered goal.
    pub order: BotOrder,
    /// Current progress.
    pub progress: BotOrderProgress,
}

/// Goal status on the shipped QuakeC `BOT_GOAL_*` contract.
pub type BotGoalStatus = u8;

/// Idle or failed status.
pub const BOT_GOAL_NONE: BotGoalStatus = 0;
/// Reached status.
pub const BOT_GOAL_REACHED: BotGoalStatus = 1;
/// In-progress status.
pub const BOT_GOAL_ACTIVE: BotGoalStatus = 2;

/// Status of a scripted goal: none/error maps to 0, success to 1,
///
/// in-progress to 2.
#[must_use]
pub fn bot_order_status(state: Option<&BotOrderState>) -> BotGoalStatus {
    match state {
        None => BOT_GOAL_NONE,
        Some(state) => match state.progress {
            BotOrderProgress::Error => BOT_GOAL_NONE,
            BotOrderProgress::Success => BOT_GOAL_REACHED,
            BotOrderProgress::InProgress => BOT_GOAL_ACTIVE,
        },
    }
}

/// Whether the scripted goal still drives the bot. Follow goals stay
/// active once issued; point goals complete on arrival.
#[must_use]
pub fn bot_order_active(state: Option<&BotOrderState>) -> bool {
    match state {
        None => false,
        Some(state) => {
            state.progress != BotOrderProgress::Error
                && (state.progress == BotOrderProgress::InProgress || matches!(state.order, BotOrder::Follow { .. }))
        }
    }
}

/// Point orders within 8 units are the same order; follow orders match
/// on entity number and generation.
#[must_use]
pub fn same_bot_order(left: &BotOrder, right: &BotOrder) -> bool {
    match (left, right) {
        (BotOrder::Point { point: a }, BotOrder::Point { point: b }) => {
            let dx = f64::from(a.x - b.x);
            let dy = f64::from(a.y - b.y);
            let dz = f64::from(a.z - b.z);
            dx.hypot(dy).hypot(dz) < 8.0
        }
        (BotOrder::Follow { entity: a }, BotOrder::Follow { entity: b }) => {
            a.number == b.number && a.generation == b.generation
        }
        _ => false,
    }
}
