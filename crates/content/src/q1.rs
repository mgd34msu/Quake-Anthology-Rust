//! Quake 1 game content (`src/content/q1`, `src/content/composition/q1`,
//! `src/content/composition/q1-expansion-arsenal.ts`,
//! `src/content/mods/callbacks.ts`).
//!
//! The foundation implements the id1/QuakeC gameplay on host-provided
//! session tables (actors, bodies, combat, inventory); it owns no second
//! health or ammunition store. Types referenced from out-of-scope
//! contracts, world gameplay, movement, and compat packages are defined
//! here structurally with their donor noted, since `qa-content` depends
//! only on `qa-core` and `qa-platform`.

use thiserror::Error;

/// Quake 1 content failure (donor `Error`/`RangeError` throws and save
/// boundary failures).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q1Error {
    /// Donor `Error` throw; the message preserves the donor text.
    #[error("{0}")]
    Message(String),
    /// Donor `RangeError` throw; the message preserves the donor text.
    #[error("{0}")]
    Range(String),
    /// Checkpoint value boundary failure.
    #[error(transparent)]
    Value(#[from] crate::value::ValueError),
}

/// Build a donor `Error` failure.
pub fn q1_error(message: impl Into<String>) -> Q1Error {
    Q1Error::Message(message.into())
}

/// Build a donor `RangeError` failure.
pub fn q1_range(message: impl Into<String>) -> Q1Error {
    Q1Error::Range(message.into())
}

pub mod addons;
pub mod base;
pub mod composition;
pub mod equipment;
pub mod foundation;
pub mod missionpacks;
pub mod mods_callbacks;
pub mod quakec;
