//! Quake III base/game: grapple.
//!
//! Donor provenance: `src/content/q3/base/game/grapple.ts`.

use qa_core::math::{add3, length3, normalize3, scale3, sub3, vec3, Bounds, Vec3};

// Intra-group imports: sibling modules split from the same flat port.

// ---------------------------------------------------------------------------
// Grapple (grapple.ts).
// ---------------------------------------------------------------------------

/// Grapple missile speed (`Q3_GRAPPLE_SPEED`).
pub const Q3_GRAPPLE_SPEED: f32 = 800.0;

/// Grapple missile lifetime milliseconds (`Q3_GRAPPLE_LIFETIME`).
pub const Q3_GRAPPLE_LIFETIME: i32 = 10000;

/// Grapple think interval milliseconds (`Q3_GRAPPLE_THINK_INTERVAL`).
pub const Q3_GRAPPLE_THINK_INTERVAL: i32 = 100;

/// Grapple cable endpoints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrappleCable {
    /// Cable start at the shooter.
    pub start: Vec3,
    /// Cable end at the grapple point.
    pub end: Vec3,
}

/// Grapple pull velocity (`q3GrappleVelocity`).
#[must_use]
pub fn q3_grapple_velocity(origin: Vec3, point: Vec3, forward: Vec3) -> Vec3 {
    let pull = sub3(add3(point, scale3(forward, -16.0)), origin);
    let distance = length3(pull);
    scale3(
        normalize3(pull),
        if distance <= 100.0 {
            10.0 * distance
        } else {
            Q3_GRAPPLE_SPEED
        },
    )
}

/// Grapple aim target at a body's center (`q3GrappleTarget`).
#[must_use]
pub fn q3_grapple_target(origin: Vec3, bounds: &Bounds) -> Vec3 {
    add3(origin, scale3(add3(bounds.min, bounds.max), 0.5))
}

/// Grapple cable placement (`q3GrappleCable`).
#[must_use]
pub fn q3_grapple_cable(origin: Vec3, up: Vec3, point: Vec3, view_height: f32) -> Option<GrappleCable> {
    let start = add3(add3(origin, vec3(0.0, 0.0, view_height)), scale3(up, -6.0));
    if length3(sub3(start, point)) < 64.0 {
        return None;
    }
    Some(GrappleCable { start, end: point })
}
