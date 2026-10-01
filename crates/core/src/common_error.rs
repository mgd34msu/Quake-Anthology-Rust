//! Common error control flow ported from id Software's `common.c` `Com_Error`.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/core/common-error.ts`.

use thiserror::Error;

/// Error code selecting how the host unwinds the current frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommonErrorCode {
    /// Server disconnect (client returns to a menu/lobby state).
    ServerDisconnect,
    /// Drop to console without a full shutdown.
    Drop,
    /// Disconnect from the current server.
    Disconnect,
    /// The full game media is required.
    NeedCd,
    /// Fatal error; shut the host down.
    Fatal,
}

/// Host control-flow error carrying a [`CommonErrorCode`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{code:?}: {message}")]
pub struct CommonError {
    /// How the host must unwind.
    pub code: CommonErrorCode,
    /// Human-readable message.
    pub message: String,
}

impl CommonError {
    /// Build an error for `code` with `message`.
    pub fn new(code: CommonErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carries_code_and_message() {
        let error = CommonError::new(CommonErrorCode::Fatal, "boom");
        assert_eq!(error.code, CommonErrorCode::Fatal);
        assert_eq!(error.message, "boom");
        assert_eq!(format!("{error}"), "Fatal: boom");
    }

    #[test]
    fn all_codes_round_trip() {
        for code in [
            CommonErrorCode::ServerDisconnect,
            CommonErrorCode::Drop,
            CommonErrorCode::Disconnect,
            CommonErrorCode::NeedCd,
            CommonErrorCode::Fatal,
        ] {
            assert_eq!(CommonError::new(code, "m").code, code);
        }
    }
}
