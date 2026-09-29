//! Typed compatibility errors.
//!
//! Donor provenance: `src/core/info-string.ts` (`infoValueForKey`,
//! `printInfo`) and `src/content/q3/team-arena/client-admission.ts`
//! (`clientInfoValue`, `cleanClientName`, `byteBuffer`).

use thiserror::Error;

/// Errors for cross-family compatibility shims.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CompatError {
    /// An info string reached its bound.
    #[error("Info_ValueForKey: oversize infostring")]
    OversizeInfo,
    /// A value contains non-byte characters.
    #[error("{0} requires byte characters")]
    NonByteChars(String),
    /// An info-string bound is outside `1..=8192`.
    #[error("invalid source info-string bound")]
    BadInfoBound,
    /// `Info_Print` would overflow a 512-byte key/value buffer.
    #[error("Info_Print would overflow its source {0} buffer")]
    PrintOverflow(&'static str),
}
