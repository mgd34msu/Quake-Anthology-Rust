//! Travel types from `src/bots/behavior/q3/travel/types.ts`
//! (`be_ai_move.c` travel half).
//!
//! Movement prediction results plus the mover model view the travel
//! controller reads: elevators, bobbing platforms, doors, and trains.

use qa_core::math::{Bounds, Vec3};

/// Client-movement prediction outcome. AAS generation reads the end
/// state, frame count, stop event, and end area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotTravelPredictionResult {
    /// Predicted end position.
    pub end: Vec3,
    /// Predicted end velocity.
    pub velocity: Vec3,
    /// Simulated frames.
    pub frames: i32,
    /// Stop-event bits.
    pub stop_event: i32,
    /// End area, when the predictor resolved one.
    pub end_area: Option<i32>,
}

/// Classified movement stop: event bits, stop origin, and stop area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotMovementStop {
    /// Stop-event bits.
    pub events: i32,
    /// Stop origin.
    pub origin: Vec3,
    /// Stop area.
    pub area: Option<i32>,
}

/// Mover model observed by travel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotTravelModel {
    /// Entity number.
    pub entity: i32,
    /// Origin.
    pub origin: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Mover kind.
    pub kind: BotTravelModelKind,
}

/// Mover kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotTravelModelKind {
    /// Elevator.
    Elevator,
    /// Bobbing platform.
    Bobbing,
    /// Door.
    Door,
    /// Train.
    Train,
    /// Static.
    Static,
}

/// Travel weapon modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TravelWeaponMode {
    /// Rocket jump.
    RocketJump,
    /// BFG jump.
    BfgJump,
    /// Grapple.
    Grapple,
}

/// Movement variables read by travel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotTravelVariables {
    /// Maximum step height.
    pub sv_max_step: f32,
    /// Maximum barrier height.
    pub sv_max_barrier: f32,
    /// Gravity.
    pub sv_gravity: f32,
    /// Rocket launcher weapon index.
    pub rocket_launcher_index: i32,
    /// BFG index.
    pub bfg_index: i32,
    /// Grapple index.
    pub grapple_index: i32,
    /// Missile entity type.
    pub missile_entity_type: i32,
    /// Offhand grapple.
    pub offhand_grapple: bool,
}

impl Default for BotTravelVariables {
    fn default() -> Self {
        Self {
            sv_max_step: 18.0,
            sv_max_barrier: 32.0,
            sv_gravity: 800.0,
            rocket_launcher_index: 5,
            bfg_index: 9,
            grapple_index: 10,
            missile_entity_type: 2,
            offhand_grapple: false,
        }
    }
}
