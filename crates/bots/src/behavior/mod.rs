//! Bot behavior from `src/bots/behavior/*`.
//!
//! Scripted orders, the source bot director, the shared population,
//! movement projection, the botlib service libraries, the Q3 game AI,
//! and the rerelease brain. Navigation-visible names from the former
//! navigation-subset pin keep working through re-exports.

pub mod assets;
pub mod director;
pub mod library;
pub mod orders;
pub mod population;
pub mod prediction;
pub mod q3;
pub mod rerelease;

pub use assets::{load_bot_asset_files, normalize_bot_path, BotAssetFiles, BotSourceFiles};
pub use director::{SourceBotDirector, SourceBotDirectorHost, SourceBotRosterEntry};
pub use orders::{
    bot_order_active, bot_order_status, same_bot_order, BotGoalStatus, BotOrder, BotOrderEntity, BotOrderProgress,
    BotOrderState, BOT_GOAL_ACTIVE, BOT_GOAL_NONE, BOT_GOAL_REACHED,
};
pub use population::{BotActorCommand, BotFrame, PopulationCommandSource, SharedBotPopulation};
pub use prediction::{project_bot_movement, BotMovementProjection};
pub use q3::navigation_types::{travel_flag_for_type, BotMovementPrediction, TravelFlags, TravelType};
pub use q3::travel::types::{BotMovementStop, BotTravelPredictionResult};
