//! Typed failures for development tooling.
//!
//! Donor `Error` sites become named variants so callers match on meaning
//! instead of message text; the original donor messages are preserved.

use std::io;

use thiserror::Error;

/// Failure of a tooling parse, capture, inventory, or verification operation.
#[derive(Debug, Error)]
pub enum ToolsError {
    /// Filesystem or process IO failure.
    #[error("{context}: {source}")]
    Io {
        /// What was being attempted.
        context: String,
        /// Underlying failure.
        #[source]
        source: io::Error,
    },
    /// Malformed input document or value.
    #[error("{0}")]
    Parse(String),
    /// Schema or policy violation with a stable message.
    #[error("{0}")]
    Invalid(String),
    /// External command failed.
    #[error("{0}")]
    Command(String),
    /// Camera file failure from `qa-client`.
    #[error(transparent)]
    Camera(#[from] qa_client::ClientError),
}

impl ToolsError {
    /// Build a [`ToolsError::Parse`] from parts.
    #[must_use]
    pub fn parse(message: impl Into<String>) -> Self {
        Self::Parse(message.into())
    }

    /// Build a [`ToolsError::Invalid`] from parts.
    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    /// Build a [`ToolsError::Command`] from parts.
    #[must_use]
    pub fn command(message: impl Into<String>) -> Self {
        Self::Command(message.into())
    }

    /// Attach context to an IO failure.
    #[must_use]
    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}
