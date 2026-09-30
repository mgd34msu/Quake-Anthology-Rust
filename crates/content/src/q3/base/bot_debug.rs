//! Quake III base: bot debug.
//!
//! Donor provenance: `src/content/q3/base/bot-debug.ts`.

use qa_core::math::Vec3;

// Intra-group imports: sibling modules split from the same flat port.

// ---------------------------------------------------------------------------
// bot-debug.ts
// ---------------------------------------------------------------------------

/// Bot debug polygon sink (`BotDebugPolygons`).
pub trait BotDebugPolygons {
    /// Create a debug polygon, returning its identifier.
    fn create(&mut self, color: i32, count: usize, points: &[Vec3]) -> i32;
}
