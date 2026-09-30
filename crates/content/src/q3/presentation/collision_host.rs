//! Quake III presentation: collision host.
//!
//! Donor provenance: `src/content/q3/presentation/collision-host.ts`.
//!
//! Host surface over the out-of-scope collision donor (`world/collision/q3/world.ts`).

use qa_core::math::Vec3;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::world::*;

// ---------------------------------------------------------------------------
// collision-host.ts (unified from mirrors_present_client)
// ---------------------------------------------------------------------------

/// Trace query (`TraceQuery`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceQuery {
    /// Start.
    pub start: Vec3,
    /// End.
    pub end: Vec3,
    /// Shape.
    pub shape: TraceShape,
    /// Mask.
    pub mask: i32,
    /// Model index.
    pub model_index: Option<i32>,
}

/// Trace result (`TraceResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceResult {
    /// Fraction.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Solidity.
    pub solidity: TraceSolidity,
    /// Contact.
    pub contact: TraceContact,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
}

/// Collision world (`CollisionWorld`).
pub trait CollisionWorld {
    /// Trace the world.
    fn trace(&mut self, query: &TraceQuery) -> TraceResult;
    /// Point contents.
    fn point_contents(&mut self, point: Vec3) -> i32;
    /// Transformed model trace.
    fn transformed_trace(&mut self, query: &TraceQuery, model_index: i32, origin: Vec3, angles: Vec3) -> TraceResult;
    /// Transformed point contents.
    fn transformed_point_contents(&mut self, point: Vec3, model_index: i32, origin: Vec3, angles: Vec3) -> i32;
    /// Box-model trace (`createBoxModel(...).transformedTrace`).
    fn box_trace(&mut self, mins: Vec3, maxs: Vec3, query: &TraceQuery, origin: Vec3) -> TraceResult;
}
