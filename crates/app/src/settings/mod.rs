//! Settings: seat configuration, restart flow, and server settings.
//!
//! Donor provenance: `src/settings/{config,restart}.ts` and
//! `src/settings/server/*`. Document parsing validates with the donor's
//! messages; file writes stay atomic under the writable root; server
//! setting definitions, ranges, and profile handling match the donor
//! collections. Input-device tuning types that the donor imports from
//! unported input/platform modules (`GamepadTuning`, `MouseTuning`,
//! `ControllerSelection`) live here as document-schema projections; key
//! codes, bindings, and axes reuse [`qa_client::input`].

pub mod config;
pub mod json;
pub mod restart;
pub mod server;

use thiserror::Error;

/// Failure of a settings operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SettingsError {
    /// A document value failed validation (carries the donor message).
    #[error("{0}")]
    BadValue(String),
    /// A path escapes the writable directory.
    #[error("{0}")]
    BadPath(String),
    /// A file operation failed.
    #[error("{0}")]
    Io(String),
    /// Command text failed validation.
    #[error("{0}")]
    Command(String),
    /// A cvar operation failed.
    #[error("{0}")]
    Cvar(String),
    /// A console operation failed.
    #[error("{0}")]
    Console(String),
}

impl From<qa_core::cmd::CmdError> for SettingsError {
    fn from(error: qa_core::cmd::CmdError) -> Self {
        Self::Command(error.to_string())
    }
}

impl From<qa_core::cvar::CvarError> for SettingsError {
    fn from(error: qa_core::cvar::CvarError) -> Self {
        Self::Cvar(error.to_string())
    }
}

impl From<crate::console::ConsoleError> for SettingsError {
    fn from(error: crate::console::ConsoleError) -> Self {
        Self::Console(error.to_string())
    }
}

impl From<std::io::Error> for SettingsError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
