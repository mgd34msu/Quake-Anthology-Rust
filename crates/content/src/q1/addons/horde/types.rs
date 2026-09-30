//! Q1 horde services (`src/content/q1/addons/horde/types.ts`).

use qa_core::identity::ActorId;

/// Engine services for the horde (`Q1HordeServices`).
pub trait Q1HordeServices: Send {
    /// Source deadflag, distinct from shared health during death animations.
    fn dead_flag(&mut self, player: &ActorId) -> i32;
    /// Whether the player is untargetable.
    fn no_target(&mut self, player: &ActorId) -> bool;
    /// Whether the player is a bot.
    fn is_bot(&mut self, player: &ActorId) -> bool;
    /// Respawns a teammate.
    fn respawn_teammate(&mut self, player: &ActorId);
    /// Adds score to a player.
    fn add_score(&mut self, player: &ActorId, delta: f64);
    /// Restarts the map with source reset flags.
    fn restart_session(&mut self, map: &str, starting_server_flags: i32);
}
