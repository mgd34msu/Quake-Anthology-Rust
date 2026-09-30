//! Quake III Team Arena (`src/content/q3/team-arena`).

pub mod arenas;
pub mod client_admission;
pub mod client_effects;
pub mod client_events;
pub mod client_policy;
pub mod client_spawn;
pub mod client_think;
pub mod commands;
pub mod foreign_objectives;
pub mod index;
pub mod r#match;
pub mod movement_host;
pub mod objective_placement;
pub mod server_commands;
pub mod session;
pub mod support;
pub mod team;
#[cfg(test)]
mod tests;
