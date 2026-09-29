//! Typed application failures: option parsing, startup, and the host loop.

use thiserror::Error;

/// Failure of an application-layer operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AppError {
    /// An option flag is unknown.
    #[error("Unknown option: {0}")]
    UnknownOption(String),
    /// An option is missing its value.
    #[error("Missing value for {0}")]
    MissingValue(String),
    /// An option value is out of range or malformed.
    #[error("{0}")]
    BadValue(String),
    /// Two options conflict.
    #[error("{0}")]
    ConflictingOptions(String),
    /// A `+command` startup argument is malformed.
    #[error("{0}")]
    BadStartupCommand(String),
    /// A map name is malformed.
    #[error("Invalid map name: {0}")]
    BadMapName(String),
    /// A startup requirement is unmet (missing content, closed server).
    #[error("{0}")]
    Startup(String),
    /// The host loop was stepped after it finished.
    #[error("Application has finished")]
    Finished,
    /// The weapon-behavior tool is not ported yet.
    #[error("weapon-behavior tool is not ported yet")]
    ToolUnavailable,
    /// World failure during startup or the host loop.
    #[error("World error: {0}")]
    World(String),
    /// Client failure during startup or the host loop.
    #[error("Client error: {0}")]
    Client(String),
    /// Network failure during startup or the host loop.
    #[error("Network error: {0}")]
    Net(String),
    /// Content path failure during startup.
    #[error("Content error: {0}")]
    Content(String),
    /// Console failure.
    #[error("Console error: {0}")]
    Console(String),
    /// Settings failure.
    #[error("Settings error: {0}")]
    Settings(String),
    /// Debug failure.
    #[error("Debug error: {0}")]
    Debug(String),
    /// Persistence failure.
    #[error("Persistence error: {0}")]
    Persistence(String),
    /// Guest VM failure during startup or the host loop.
    #[error("Guest error: {0}")]
    Guest(String),
}

impl From<qa_world::WorldError> for AppError {
    fn from(error: qa_world::WorldError) -> Self {
        Self::World(error.to_string())
    }
}

impl From<qa_client::ClientError> for AppError {
    fn from(error: qa_client::ClientError) -> Self {
        Self::Client(error.to_string())
    }
}

impl From<qa_guest::GuestError> for AppError {
    fn from(error: qa_guest::GuestError) -> Self {
        Self::Guest(error.to_string())
    }
}

impl From<qa_core::cmd::CmdError> for AppError {
    fn from(error: qa_core::cmd::CmdError) -> Self {
        Self::BadStartupCommand(error.to_string())
    }
}

impl From<crate::console::ConsoleError> for AppError {
    fn from(error: crate::console::ConsoleError) -> Self {
        Self::Console(error.to_string())
    }
}

impl From<crate::settings::SettingsError> for AppError {
    fn from(error: crate::settings::SettingsError) -> Self {
        Self::Settings(error.to_string())
    }
}

impl From<crate::debug::DebugError> for AppError {
    fn from(error: crate::debug::DebugError) -> Self {
        Self::Debug(error.to_string())
    }
}

impl From<crate::persistence::PersistenceError> for AppError {
    fn from(error: crate::persistence::PersistenceError) -> Self {
        Self::Persistence(error.to_string())
    }
}
