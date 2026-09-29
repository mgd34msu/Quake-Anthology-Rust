//! Console core: line buffer, edit fields, dispatch, log, history, discovery.
//!
//! Donor provenance: `src/console/{buffer,field,commands,log,session,
//! metrics,discovery,dedicated,source-field}.ts`. Command text uses
//! [`qa_core::cmd`] and variable storage uses
//! [`qa_core::cvar::CvarRegistry`]; nothing here duplicates those cores.
//!
//! Explicitly out of scope (recorded, not ported):
//!
//! - `src/console/draw.ts`: display rendering of the console overlay; it
//!   needs the text atlas and renderer backends that stay headless here.
//! - `src/console/llm*.ts` and `src/llm/*`: developer-tool LLM edges,
//!   deferred with the rest of `llm/` (external API clients, not game
//!   behavior).
//!
//! Dispatch here is synchronous and single-seat: the donor's buffered
//! `CommandBuffer` (`wait`, multi-frame inserts, script callbacks) and its
//! async host queue stay out. Builtin command names, log behavior, history
//! depth, and discovery output match the donor.

pub mod buffer;
pub mod commands;
pub mod dedicated;
pub mod discovery;
pub mod field;
pub mod log;
pub mod metrics;
pub mod session;
pub mod source_field;

use thiserror::Error;

/// Failure of a console operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConsoleError {
    /// A console width or capacity is out of range.
    #[error("Invalid console width")]
    BadWidth,
    /// A console has no current row (unreachable invariant).
    #[error("Console has no current row")]
    NoCurrentRow,
    /// A scroll or count argument is out of range.
    #[error("Invalid console count")]
    BadCount,
    /// A camera name or path is malformed.
    #[error("{0}")]
    BadCamera(String),
    /// A field operation violated its byte bounds.
    #[error("{0}")]
    BadField(String),
    /// A console log write failed or raced a close.
    #[error("{0}")]
    BadLog(String),
    /// A dedicated-console read failed or raced a close.
    #[error("{0}")]
    BadDedicated(String),
    /// A discovery query or page is malformed.
    #[error("{0}")]
    BadDiscovery(String),
    /// A console command failed (usage errors print instead).
    #[error("{0}")]
    BadCommand(String),
    /// Command text failed validation.
    #[error("{0}")]
    Command(String),
    /// A cvar operation failed.
    #[error("{0}")]
    Cvar(String),
    /// A settings operation failed.
    #[error("{0}")]
    Settings(String),
    /// A capture operation failed.
    #[error("{0}")]
    Capture(String),
}

impl From<qa_core::cmd::CmdError> for ConsoleError {
    fn from(error: qa_core::cmd::CmdError) -> Self {
        Self::Command(error.to_string())
    }
}

impl From<qa_core::cvar::CvarError> for ConsoleError {
    fn from(error: qa_core::cvar::CvarError) -> Self {
        Self::Cvar(error.to_string())
    }
}
