//! Typed failures for the guest game-logic and native-guest VM crate.

use thiserror::Error;

/// Failure of a guest module, field, save, memory, CPU, or image operation.
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
    /// Guest memory fault: `reason` at `address` (`length` bytes, `access`).
    #[error("Guest memory fault ({reason} {access} at 0x{address:x}, {length} bytes): {detail}")]
    MemoryFault {
        /// Machine-readable fault reason.
        reason: &'static str,
        /// Faulting guest offset.
        address: u64,
        /// Requested length in bytes.
        length: usize,
        /// Requested access.
        access: &'static str,
        /// Human-readable detail.
        detail: String,
    },
    /// Invalid guest argument (out of range, wrong width, bad alignment).
    #[error("{0}")]
    InvalidArgument(String),
    /// Guest CPU fault (bad register use, snapshot mismatch, decode error).
    #[error("{0}")]
    Cpu(String),
    /// Unsupported guest instruction or image feature.
    #[error("{0}")]
    Unsupported(String),
    /// Malformed executable image.
    #[error("Invalid {image} image: {detail}")]
    BadImage {
        /// Image format label.
        image: &'static str,
        /// Human-readable detail.
        detail: String,
    },
    /// Host callback table error.
    #[error("{0}")]
    Callback(String),
    /// ABI marshalling error.
    #[error("{0}")]
    Abi(String),
    /// Guest runtime (libc/kernel) service error.
    #[error("{0}")]
    Runtime(String),
}

impl GuestError {
    /// Build a memory fault.
    #[must_use]
    pub fn memory_fault(
        reason: &'static str,
        address: u64,
        length: usize,
        access: &'static str,
        detail: impl Into<String>,
    ) -> Self {
        Self::MemoryFault {
            reason,
            address,
            length,
            access,
            detail: detail.into(),
        }
    }

    /// Build an invalid-argument error.
    #[must_use]
    pub fn invalid(detail: impl Into<String>) -> Self {
        Self::InvalidArgument(detail.into())
    }

    /// Build a CPU error.
    #[must_use]
    pub fn cpu(detail: impl Into<String>) -> Self {
        Self::Cpu(detail.into())
    }

    /// Build an unsupported-feature error.
    #[must_use]
    pub fn unsupported(detail: impl Into<String>) -> Self {
        Self::Unsupported(detail.into())
    }

    /// Build a malformed-image error.
    #[must_use]
    pub fn bad_image(image: &'static str, detail: impl Into<String>) -> Self {
        Self::BadImage {
            image,
            detail: detail.into(),
        }
    }

    /// Build a callback-table error.
    #[must_use]
    pub fn callback(detail: impl Into<String>) -> Self {
        Self::Callback(detail.into())
    }

    /// Build an ABI error.
    #[must_use]
    pub fn abi(detail: impl Into<String>) -> Self {
        Self::Abi(detail.into())
    }

    /// Build a runtime-service error.
    #[must_use]
    pub fn runtime(detail: impl Into<String>) -> Self {
        Self::Runtime(detail.into())
    }
}

impl From<qa_world::WorldError> for GuestError {
    fn from(error: qa_world::WorldError) -> Self {
        Self::BadSave(error.to_string())
    }
}
