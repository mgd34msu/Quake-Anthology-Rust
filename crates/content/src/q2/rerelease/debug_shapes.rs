//! Q2 rerelease debug shapes (`src/content/q2/rerelease/debug-shapes.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::math::Vec4;

use crate::q2::support::misc::{debug_shape_lines, DebugShape};

use super::types::Q2RereleaseEvent;

/// Convert float seconds to unsigned milliseconds (`rereleaseDebugLifetime`).
///
/// Matches `PF_Draw_*`: single-precision milliseconds truncated toward
/// zero, then wrapped to 32 bits, including negative wrapping.
pub fn rerelease_debug_lifetime(seconds: f64) -> u32 {
    if !seconds.is_finite() {
        panic!("Invalid debug lifetime");
    }
    let milliseconds = f64::from((seconds as f32) * 1000.0).trunc();
    milliseconds.rem_euclid(4_294_967_296.0) as u32
}

/// Build a debug-shape event (`q2DebugShape`).
pub fn q2_debug_shape(shape: &DebugShape, color: Vec4, lifetime_seconds: f64, depth_test: bool) -> Q2RereleaseEvent {
    Q2RereleaseEvent::DebugShapes {
        lines: debug_shape_lines(shape, color, depth_test),
        lifetime_milliseconds: rerelease_debug_lifetime(lifetime_seconds),
    }
}
