//! Q1 campaign addon root (`src/content/q1/addons`).
//!
//! Rerelease addon words for the `dopa`, `mg1`, `mg3`, and `ctf`
//! programs. The context owns per-game addon words, player references,
//! and frame ticks; every other module here registers its spawns and
//! named callbacks against it.

pub mod base_triggers;
pub mod brushes;
pub mod campaign;
pub mod commands;
pub mod context;
pub mod corpses;
pub mod ctf;
pub mod effects;
pub mod field_triggers;
pub mod horde;
pub mod items;
pub mod lights;
pub mod monsters;
pub mod rope;
pub mod travel;
pub mod triggers;
