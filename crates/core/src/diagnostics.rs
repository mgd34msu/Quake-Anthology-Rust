//! Session diagnostics: structured records plus host-abort errors.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/core/diagnostics.ts`.

use thiserror::Error;

use crate::identity::{ProviderId, SessionId};
use crate::time::SourceTime;

/// Severity of a diagnostic record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticSeverity {
    /// Debug record.
    Debug,
    /// Informational record.
    Info,
    /// Warning record.
    Warning,
    /// Error record.
    Error,
}

/// One structured diagnostic record.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// Severity.
    pub severity: DiagnosticSeverity,
    /// Stable machine-readable code.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Owning session.
    pub session: SessionId,
    /// Optional provider that produced the record.
    pub provider: Option<ProviderId>,
    /// Optional source time of the record.
    pub time: Option<SourceTime>,
}

/// Sink receiving diagnostics; returns `()` like the donor's `undefined`.
pub type DiagnosticSink = Box<dyn Fn(Diagnostic) + Send + Sync>;

/// Host abort: unwinds the current synchronous frame; fatal errors end its owner.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{kind:?}: {message}")]
pub struct EngineError {
    /// Abort kind.
    pub kind: EngineErrorKind,
    /// Human-readable message.
    pub message: String,
}

/// Abort kind for [`EngineError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineErrorKind {
    /// Disconnect from the current server.
    Disconnect,
    /// Drop to console.
    Drop,
    /// Fatal error.
    Fatal,
}

impl EngineError {
    /// Build an abort of `kind` with `message`.
    pub fn new(kind: EngineErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// Session-bound diagnostics view; stamps every record with its session.
pub struct SessionDiagnostics {
    /// Owning session.
    pub session: SessionId,
    sink: DiagnosticSink,
}

impl SessionDiagnostics {
    /// Bind `sink` to `session`.
    pub fn new(session: SessionId, sink: DiagnosticSink) -> Self {
        Self { session, sink }
    }

    /// Emit a record; the session is filled in from this view.
    pub fn emit(
        &self,
        severity: DiagnosticSeverity,
        code: impl Into<String>,
        message: impl Into<String>,
        provider: Option<ProviderId>,
        time: Option<SourceTime>,
    ) {
        (self.sink)(Diagnostic {
            severity,
            code: code.into(),
            message: message.into(),
            session: self.session.clone(),
            provider,
            time,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::IdentityOwner;
    use std::sync::{Arc, Mutex};

    #[test]
    fn engine_error_carries_kind() {
        let error = EngineError::new(EngineErrorKind::Drop, "dropped");
        assert_eq!(error.kind, EngineErrorKind::Drop);
        assert_eq!(format!("{error}"), "Drop: dropped");
    }

    #[test]
    fn session_view_stamps_session() {
        let owner = IdentityOwner::create("diag-test").expect("owner");
        let seen: Arc<Mutex<Vec<Diagnostic>>> = Arc::new(Mutex::new(Vec::new()));
        let capture = Arc::clone(&seen);
        let view = SessionDiagnostics::new(
            owner.session().clone(),
            Box::new(move |d| capture.lock().expect("lock").push(d)),
        );
        view.emit(DiagnosticSeverity::Warning, "net.lag", "slow", None, None);
        let seen = seen.lock().expect("lock");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].session, *owner.session());
        assert_eq!(seen[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(seen[0].code, "net.lag");
        assert!(seen[0].provider.is_none());
    }
}
