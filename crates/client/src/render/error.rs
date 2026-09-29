//! Renderer-local errors.
//!
//! Donor provenance: `src/contracts/render.ts` (ownership rules) and the
//! `throw` sites across `src/render/*`. Every failure in the render module
//! tree reports [`RenderError`]; sibling crates keep their own error types.

use thiserror::Error;

/// Renderer-local failure.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum RenderError {
    /// A frame, image, or seat belongs to another renderer lifetime.
    #[error("renderer object belongs to another owner: {0}")]
    ForeignOwner(String),
    /// An image ordinal has no resident texture.
    #[error("unknown renderer image ordinal {0}")]
    UnknownImage(u32),
    /// An image ordinal is already bound to another identity.
    #[error("renderer image ordinal {0} has another identity")]
    ImageConflict(u32),
    /// Image dimensions are not positive or do not match the pixels.
    #[error("invalid image dimensions {width}x{height}: {detail}")]
    BadDimensions {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// What failed validation.
        detail: String,
    },
    /// A mipmap level is out of range.
    #[error("image level {level} out of range for image {ordinal}")]
    BadLevel {
        /// Image ordinal.
        ordinal: u32,
        /// Requested level.
        level: u32,
    },
    /// Pixel encoding does not match the resident image.
    #[error("pixel encoding mismatch for image {0}: {1}")]
    PixelMismatch(u32, String),
    /// A projection matrix violates the backend's assumption.
    #[error("projection violates backend assumption: {0}")]
    BadProjection(String),
    /// A viewport or scissor rectangle is invalid.
    #[error("invalid viewport: {0}")]
    BadViewport(String),
    /// A wire value failed to decode.
    #[error("invalid renderer wire value: {0}")]
    BadWire(String),
    /// A worker request failed or arrived out of order.
    #[error("renderer worker failure: {0}")]
    Worker(String),
    /// The backend rejected an operation.
    #[error("renderer backend failure: {0}")]
    Backend(String),
    /// A batch references arrays that do not cover its vertices.
    #[error("batch arrays do not cover vertex {index}: {detail}")]
    BadBatch {
        /// Vertex index.
        index: usize,
        /// What failed validation.
        detail: String,
    },
    /// A command arrived in the wrong state.
    #[error("renderer out of order: {0}")]
    OutOfOrder(String),
    /// A value is not finite where the pipeline requires it.
    #[error("renderer requires a finite {0}")]
    NotFinite(String),
}
