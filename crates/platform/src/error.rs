//! Platform error type shared by every native binding in this crate.

use std::fmt;
use std::io;

/// Failure modes for dynamically loaded system libraries and native sockets.
///
/// [`Error::Unavailable`] is the honest absence signal: the named library (or
/// provider) could not be loaded on this host. Callers must treat it as
/// "feature not present", never as corruption.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A named native library (or one of its required symbols) is absent.
    #[error("native library `{library}` is unavailable: {detail}")]
    Unavailable {
        /// Donor [`NativeLibrary`](crate::native_libraries::NativeLibrary) name,
        /// e.g. `"sdl2"`, `"vorbisfile"`, or a pseudo-library such as `"ipx"`.
        library: String,
        /// Human-readable cause, including every candidate path attempted.
        detail: String,
    },
    /// The host ABI cannot implement this operation at all.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// Caller input violates the native contract (empty strings, NUL bytes,
    /// misaligned buffers, unknown enum values).
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// A numeric value is outside the range the native API accepts.
    #[error("out of range: {0}")]
    OutOfRange(String),
    /// A loaded native API reported a runtime failure.
    #[error("{operation} failed: {detail}")]
    Native {
        /// Operation name, e.g. `"SDL_CreateWindow"` or `"ov_read"`.
        operation: String,
        /// Backend-supplied detail (SDL error string, errno name, ...).
        detail: String,
    },
    /// A typed native-code failure such as [`FreeTypeError`](crate::freetype::FreeTypeError).
    #[error("{operation} failed with native error {code}")]
    Coded {
        /// Operation name, e.g. `"ov_fopen"`.
        operation: String,
        /// Backend error code.
        code: i64,
    },
    /// Use of a closed handle (window, device, decoder, file).
    #[error("{0} is closed")]
    Closed(String),
    /// Several cleanup steps failed; mirrors donor `AggregateError` handling.
    #[error("{message} ({count} failures)")]
    Aggregate {
        /// What was being cleaned up.
        message: String,
        /// Number of contributing failures.
        count: usize,
        /// Contributing failures, in order.
        #[source]
        errors: Box<AggregateList>,
    },
    /// Underlying OS I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

/// Ordered list of failures behind [`Error::Aggregate`].
#[derive(Debug)]
pub struct AggregateList(pub Vec<Error>);

impl fmt::Display for AggregateList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for AggregateList {}

impl Error {
    /// Absent-library error that always names the library.
    pub fn unavailable(library: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::Unavailable {
            library: library.into(),
            detail: detail.into(),
        }
    }

    /// Native API failure with backend detail.
    pub fn native(operation: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::Native {
            operation: operation.into(),
            detail: detail.into(),
        }
    }

    /// Combine cleanup failures; an empty list is a logic error at the call site.
    pub fn aggregate(message: impl Into<String>, errors: Vec<Error>) -> Self {
        debug_assert!(!errors.is_empty(), "aggregate requires at least one error");
        Self::Aggregate {
            message: message.into(),
            count: errors.len(),
            errors: Box::new(AggregateList(errors)),
        }
    }

    /// True only for the honest absence signal.
    #[must_use]
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
    }
}

/// Short alias used across the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;
