//! Q2-local support shims for out-of-scope donor modules.
//!
//! The `src/content/q2` donor imports a small surface from sibling trees
//! (`src/contracts`, `src/core`, `src/world`, `src/movement`,
//! `src/persistence`, `src/debug`, `src/text`) that other lanes own and
//! that `qa-content` cannot depend on. Each item here mirrors its donor
//! shape so the Q2 port builds on merged crates plus these modules only.
//! Everything already ported to `qa-core` or shared `qa-content` modules
//! is reused, never redefined.

pub mod contracts;
pub mod misc;
pub mod movement;
pub mod tables;
