//! Persistence: saved games, recipes, providers, and save policy.
//!
//! Donor provenance: `src/persistence/{saved-game,save-policy,recipe,
//! mods,q1,q1-source,q1-foundation,q1-quakec,q1-selection,q2-*,q3,
//! native-weapon,qvm-grapple}.ts` plus the unified-save assembly over
//! `src/persistence/save-image.ts` shared records. Wire bytes reuse the
//! [`qa_world::save`] codec and [`qa_core::binary`]; guest checkpoints
//! reuse [`qa_guest::checkpoint`]. Only error mapping is app-local.
//!
//! Out of scope by donor boundary: image codecs (content formats stay
//! deferred per `README.md`; the `save-image` module here is the unified
//! *save envelope*, which the donor names `save-image.ts`), mod callback
//! declaration shapes (owned by content mods readers; retained as
//! validated records), and native weapon loader cross-checks (owned by
//! the compat rerelease loader; structural identity is verified here).

pub mod image;
pub mod mods;
pub mod native_weapon;
pub mod policy;
pub mod q1;
pub mod q2;
pub mod q3;
pub mod qvm_grapple;
pub mod recipe;
pub mod saved_game;

use thiserror::Error;

/// Failure of a persistence operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PersistenceError {
    /// Malformed save data (carries the `path: message` detail).
    #[error("{0}")]
    BadSave(String),
    /// Save path escapes its directory or breaks slot rules.
    #[error("{0}")]
    BadPath(String),
    /// File operation failure.
    #[error("{0}")]
    Io(String),
}

impl From<qa_world::WorldError> for PersistenceError {
    fn from(error: qa_world::WorldError) -> Self {
        Self::BadSave(error.to_string())
    }
}

impl From<qa_guest::GuestError> for PersistenceError {
    fn from(error: qa_guest::GuestError) -> Self {
        Self::BadSave(error.to_string())
    }
}

impl From<qa_core::binary::BinaryError> for PersistenceError {
    fn from(error: qa_core::binary::BinaryError) -> Self {
        Self::BadSave(error.to_string())
    }
}

impl From<std::io::Error> for PersistenceError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
