//! Typed failures for the guest game-logic crate.

use thiserror::Error;

/// Failure of a guest module, field, or save operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GuestError {
    /// Unknown entity field.
    #[error("Unknown entity field: {0}")]
    UnknownField(String),
    /// Field value has the wrong type.
    #[error("Field {0} has the wrong value type")]
    FieldType(String),
    /// Unknown game module.
    #[error("Unknown game module: {0}")]
    UnknownModule(String),
    /// Duplicate game module.
    #[error("Duplicate game module: {0}")]
    DuplicateModule(String),
    /// Callback dispatch exceeded its depth.
    #[error("Callback dispatch exceeded its depth")]
    DispatchDepth,
    /// Dispatch while the registry is closed.
    #[error("Guest registry is closed")]
    RegistryClosed,
    /// Invalid guest save image.
    #[error("{0}")]
    BadSave(String),
}

impl From<qa_world::WorldError> for GuestError {
    fn from(error: qa_world::WorldError) -> Self {
        Self::BadSave(error.to_string())
    }
}
