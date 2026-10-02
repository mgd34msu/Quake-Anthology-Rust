//! Simulation: actors, bodies, spatial queries, collision, movement,
//! gameplay, and sessions. Headless: never depends on client or net.

pub mod actor_execution;
pub mod ai;
pub mod body;
pub mod client;
pub mod clocks;
pub mod collision;
pub mod combat;
pub mod hull;
pub mod inventory;
pub mod movement;
pub mod movers;
pub mod pickups;
pub mod registry;
pub mod save;
pub mod scheduler;
pub mod server;
pub mod session;
pub mod spatial;
pub mod spawn;
pub mod timers;
pub mod triggers;

mod error;

pub use error::WorldError;
// Re-exports mirror `src/world/session/index.ts`.
pub use clocks::SourceClock;
pub use session::{EngineSession, ResourceScope, SessionClient, SessionMode, SessionResource, WorldLifetime};
