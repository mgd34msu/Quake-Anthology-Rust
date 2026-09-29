//! Q3 travel followers from `src/bots/behavior/q3/travel/index.ts`.

pub mod controller;
pub mod ground;
pub mod routing;
pub mod special;
pub mod types;

pub use controller::{avoid_spots, check_blocked, move_to_goal, note_blocked_reach, TravelOutcome, TravelStep};
pub use ground::{gap_distance, travel_ground, GroundReachability};
pub use routing::{air_control, jump_speed, move_in_direction, movement_view_target, predict_visible_position};
pub use special::{travel_special, MoverObservation, SpecialReachability};
pub use types::{
    BotMovementStop, BotTravelModel, BotTravelModelKind, BotTravelPredictionResult, BotTravelVariables,
    TravelWeaponMode,
};
