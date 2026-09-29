//! Source mover-bounds matching, ported from
//! `src/bots/navigation/entity-binding.ts`.

use qa_core::math::{Bounds, Vec3};

/// NAV2 names the source linked box, including link padding, at an
/// authored mover endpoint.
#[must_use]
pub fn source_mover_bounds_match(authored: &Bounds, linked: &Bounds, origin: Vec3, endpoint: Vec3) -> bool {
    f64::from(authored.min.x) == f64::from(linked.min.x) + f64::from(endpoint.x) - f64::from(origin.x)
        && f64::from(authored.max.x) == f64::from(linked.max.x) + f64::from(endpoint.x) - f64::from(origin.x)
        && f64::from(authored.min.y) == f64::from(linked.min.y) + f64::from(endpoint.y) - f64::from(origin.y)
        && f64::from(authored.max.y) == f64::from(linked.max.y) + f64::from(endpoint.y) - f64::from(origin.y)
        && f64::from(authored.min.z) == f64::from(linked.min.z) + f64::from(endpoint.z) - f64::from(origin.z)
        && f64::from(authored.max.z) == f64::from(linked.max.z) + f64::from(endpoint.z) - f64::from(origin.z)
}
