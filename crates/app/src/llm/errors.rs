//! LLM error surface: only settings-boundary messages may reach the UI.
//!
//! Donor provenance: `src/llm/errors.ts` (`LlmSettingsError`,
//! `LlmHttpError`). Provider bodies and headers are never reflected;
//! [`LlmError::http`] maps a status to the donor's fixed advice text.

use thiserror::Error;

/// An LLM failure with a UI-safe message.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LlmError {
    /// A settings, transport, or protocol failure with a fixed message.
    #[error("{0}")]
    Settings(String),
    /// An HTTP status failure with the donor's status-specific advice.
    #[error("{message}")]
    Http {
        /// HTTP status code.
        status: u16,
        /// Rendered `LLM service returned HTTP {status}. {advice}` text.
        message: String,
    },
    /// A fetch that exceeded its deadline.
    #[error("LLM request timed out. Try a shorter request or check the service.")]
    Timeout,
}

impl LlmError {
    /// Build a settings-surface error from a fixed message.
    #[must_use]
    pub fn settings(message: impl Into<String>) -> Self {
        Self::Settings(message.into())
    }

    /// Build the donor's HTTP status error with its fixed advice text.
    #[must_use]
    pub fn http(status: u16) -> Self {
        let advice = if status == 401 || status == 403 {
            "Check the selected provider's credential or sign in again."
        } else if status == 429 {
            "The service limit was reached. Try again later."
        } else if status == 400 || status == 404 {
            "Check the model and service settings."
        } else {
            "Try again later."
        };
        Self::Http {
            status,
            message: format!("LLM service returned HTTP {status}. {advice}"),
        }
    }

    /// HTTP status when this is an [`LlmError::Http`] error.
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Http { status, .. } => Some(*status),
            Self::Settings(_) | Self::Timeout => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_advice_matches_donor_statuses() {
        assert_eq!(
            LlmError::http(401).to_string(),
            "LLM service returned HTTP 401. Check the selected provider's credential or sign in again."
        );
        assert_eq!(
            LlmError::http(429).to_string(),
            "LLM service returned HTTP 429. The service limit was reached. Try again later."
        );
        assert_eq!(
            LlmError::http(404).to_string(),
            "LLM service returned HTTP 404. Check the model and service settings."
        );
        assert_eq!(
            LlmError::http(500).to_string(),
            "LLM service returned HTTP 500. Try again later."
        );
        assert_eq!(LlmError::http(403).status(), Some(403));
        assert_eq!(LlmError::settings("x").status(), None);
    }
}
