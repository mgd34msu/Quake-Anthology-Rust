//! Quake III base: world.
//!
//! Donor provenance: `src/content/q3/base/world.ts`.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Plane, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::mirrors::*;

// ---------------------------------------------------------------------------
// world.ts (+ shared/slide-move.ts)
// ---------------------------------------------------------------------------

/// Trace shape (`TraceShape`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceShape {
    /// Point trace.
    Point,
    /// Box trace.
    Box {
        /// Minimum corner.
        mins: Vec3,
        /// Maximum corner.
        maxs: Vec3,
    },
    /// Capsule trace.
    Capsule {
        /// Minimum corner.
        mins: Vec3,
        /// Maximum corner.
        maxs: Vec3,
    },
}

/// Server trace query (`ServerTraceQuery`).
#[derive(Debug, Clone, PartialEq)]
pub struct ServerTraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Trace shape.
    pub shape: TraceShape,
    /// Entity number to pass through.
    pub pass_entity_num: i32,
    /// Contents mask.
    pub mask: i32,
}

/// Trace solidity (`ServerTraceResult` solidity word).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceSolidity {
    /// Clear.
    Clear,
    /// Started solid.
    StartSolid,
    /// Entirely solid.
    AllSolid,
}

/// Trace contact (`TraceContact`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceContact {
    /// No contact.
    None,
    /// Plane contact.
    Plane {
        /// Contact plane.
        plane: Plane,
    },
}

/// Server trace result (`ServerTraceResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ServerTraceResult {
    /// Fraction traveled.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Hit entity number.
    pub entity_num: i32,
    /// Solidity.
    pub solidity: TraceSolidity,
    /// Contact.
    pub contact: TraceContact,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
}

/// Movement trace (`MovementTrace` via `slide-move.ts`).
pub type MovementTrace = ServerTraceResult;

/// Actor trace query (`ActorTraceQuery`).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorTraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Trace shape.
    pub shape: TraceShape,
    /// Actor to pass through.
    pub pass_actor: Option<ActorId>,
    /// Contents mask.
    pub mask: i32,
}

/// Actor trace hit (`ActorTraceResult` hit word).
#[derive(Debug, Clone, PartialEq)]
pub enum ActorTraceHit {
    /// No hit.
    None,
    /// World hit.
    World,
    /// Actor hit.
    Actor {
        /// Hit actor.
        actor: ActorId,
    },
}

/// Actor trace result (`ActorTraceResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorTraceResult {
    /// Fraction traveled.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Hit record.
    pub hit: ActorTraceHit,
    /// Contact.
    pub contact: TraceContact,
    /// Solidity.
    pub solidity: TraceSolidity,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
}

/// Actor spatial queries (`ActorSpatialQueries`).
pub trait ActorSpatialQueries {
    /// Actors overlapping bounds, at most `maximum`.
    fn area_actors(&self, bounds: Bounds, maximum: usize) -> Vec<ActorId>;
    /// Trace against actors.
    fn trace_actor(&self, query: &ActorTraceQuery) -> ActorTraceResult;
    /// Whether bounds contact an actor.
    fn contact_actor(&self, bounds: Bounds, actor: &ActorId, capsule: bool) -> bool;
}

/// Link state (`LinkState`).
#[derive(Debug, Clone, PartialEq)]
pub struct LinkState {
    /// Absolute bounds.
    pub absbounds: Bounds,
    /// Whether linked.
    pub linked: bool,
    /// Link count.
    pub linkcount: i32,
}

/// Source-shaped operations over the session collision and body owners
/// (`ServerWorld`). No world storage lives here.
pub trait ServerWorld {
    /// Whether bounds contact an entity.
    fn entity_contact(&self, bounds: Bounds, entity_num: i32, capsule: bool) -> bool;
    /// Trace against the world.
    fn trace(&self, query: &ServerTraceQuery) -> ServerTraceResult;
    /// Entities overlapping bounds, at most `maximum` (source default
    /// 1024).
    fn area_entities(&self, bounds: Bounds, maximum: usize) -> Vec<i32>;
    /// Contents at a point.
    fn point_contents(&self, point: Vec3, pass_entity_num: i32) -> i32;
    /// Link state for an entity number.
    fn link_state(&self, number: i32) -> Option<LinkState>;
    /// Link an entity.
    fn link(&self, entity: EntityRef);
    /// Unlink an entity number.
    fn unlink(&self, number: i32);
}

/// Combined server world plus actor queries.
pub trait Q3ServerWorld: ServerWorld + ActorSpatialQueries {}

impl<T: ServerWorld + ActorSpatialQueries> Q3ServerWorld for T {}
