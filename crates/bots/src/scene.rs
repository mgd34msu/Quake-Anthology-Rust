//! Navigation-visible scene contracts projected from
//! `src/contracts/scene.ts`: trace/policy/shape/target vocabulary, the
//! [`SceneQueries`] trait, and the decoded-world geometry subset that
//! construction and AAS reachability read (models, faces, planes, edges,
//! vertices, surfaces, leaves, entities). Full scene ownership lives with
//! the world provider; this module pins the shapes navigation consumes.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Plane, Vec3};
use qa_core::numeric::NumericProfile;

/// BSP family of a decoded world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorldKind {
    /// Quake I BSP.
    Q1Bsp,
    /// Quake II BSP.
    Q2Bsp,
    /// Quake III BSP.
    Q3Bsp,
}

/// Contiguous record span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IndexRange {
    /// First record.
    pub first: usize,
    /// Record count.
    pub count: usize,
}

/// Collision body of a trace query.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceShape {
    /// Point trace.
    Point,
    /// Axis-aligned box trace.
    Box {
        /// Body bounds.
        bounds: Bounds,
    },
    /// Capsule trace.
    Capsule {
        /// Body bounds.
        bounds: Bounds,
    },
}

/// Non-point collision body carried by a navigation profile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BodyShape {
    /// Axis-aligned box body.
    Box(Bounds),
    /// Capsule body.
    Capsule(Bounds),
}

impl BodyShape {
    /// Body bounds.
    #[must_use]
    pub fn bounds(&self) -> Bounds {
        match *self {
            BodyShape::Box(bounds) | BodyShape::Capsule(bounds) => bounds,
        }
    }
}

impl From<BodyShape> for TraceShape {
    fn from(shape: BodyShape) -> Self {
        match shape {
            BodyShape::Box(bounds) => TraceShape::Box { bounds },
            BodyShape::Capsule(bounds) => TraceShape::Capsule { bounds },
        }
    }
}

/// Quake I move rule for traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1MoveRule {
    /// Normal movement.
    Normal,
    /// Monsters do not block.
    NoMonsters,
    /// Missile movement.
    Missile,
}

/// Quake II leaf-contents selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LeafContents {
    /// Stored leaf contents.
    Stored,
    /// Merged leaf contents.
    Merged,
}

/// Collision family policy of a trace or contents query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TracePolicy {
    /// Quake I hull policy.
    Q1 {
        /// Move rule.
        move_rule: Q1MoveRule,
        /// Hull override.
        hull: Option<i32>,
    },
    /// Quake II brush policy.
    Q2 {
        /// Contents mask.
        contents_mask: i32,
        /// Leaf-contents selection.
        leaf_contents: LeafContents,
    },
    /// Quake III brush policy.
    Q3 {
        /// Contents mask.
        contents_mask: i32,
        /// Curve collision enabled.
        curves: bool,
        /// Player curve clipping enabled.
        player_curve_clip: bool,
    },
}

impl TracePolicy {
    /// Policy family.
    #[must_use]
    pub fn family(&self) -> WorldKind {
        match self {
            TracePolicy::Q1 { .. } => WorldKind::Q1Bsp,
            TracePolicy::Q2 { .. } => WorldKind::Q2Bsp,
            TracePolicy::Q3 { .. } => WorldKind::Q3Bsp,
        }
    }
}

/// Trace target: the static world or one posed brush model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QueryTarget {
    /// Static world.
    World,
    /// Brush model with an explicit pose.
    Model {
        /// Model index.
        model: i32,
        /// Model origin.
        origin: Vec3,
        /// Model angles.
        angles: Vec3,
    },
}

/// Trace query against shared collision.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceQuery {
    /// Sweep start.
    pub start: Vec3,
    /// Sweep end.
    pub end: Vec3,
    /// Sweep body.
    pub shape: TraceShape,
    /// Sweep target.
    pub target: QueryTarget,
    /// Collision policy.
    pub policy: TracePolicy,
    /// Selected numeric profile.
    pub numeric: NumericProfile,
    /// Actor to ignore.
    pub pass_actor: Option<ActorId>,
}

/// Trace impact plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceContact {
    /// No contact.
    None,
    /// Impact plane.
    Plane {
        /// Contact plane.
        plane: Plane,
    },
}

/// Trace hit record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceHit {
    /// No hit.
    None,
    /// Static world model hit.
    World {
        /// Model index.
        model: i32,
    },
    /// Actor hit.
    Actor {
        /// Hit actor.
        actor: ActorId,
    },
}

/// BSP plane with its source type tags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BspPlane {
    /// Plane normal.
    pub normal: Vec3,
    /// Plane distance.
    pub distance: f32,
    /// Source plane type.
    pub plane_type: i32,
    /// Source sign bits.
    pub signbits: i32,
}

/// Minimal Quake II surface record: navigation reads only the flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2SurfaceInfo {
    /// Surface name.
    pub name: String,
    /// Surface flags.
    pub flags: i32,
}

/// Runner-up Quake II impact: the second enter plane beside the primary
/// surface (`secondary` on donor Quake II traces).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SecondaryImpact {
    /// Runner-up plane.
    pub plane: BspPlane,
    /// Primary surface retained beside it.
    pub surface: Option<Q2SurfaceInfo>,
}

/// Per-family trace detail. Source ABI records retain the stored plane
/// even when contact is none.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceDetail {
    /// Quake I result.
    Q1 {
        /// Ended in open space.
        in_open: bool,
        /// Ended in water.
        in_water: bool,
        /// Stored source plane.
        source_plane: Plane,
        /// Optional surface flags.
        surface_flags: Option<i32>,
        /// Native contents. Native hull traces always carry it; adapted
        /// results leave it empty and readers fall back to the hit record.
        contents: Option<i32>,
    },
    /// Quake II result.
    Q2 {
        /// Contents at the end position.
        contents: i32,
        /// Hit surface.
        surface: Option<Q2SurfaceInfo>,
        /// Stored source plane.
        source_plane: BspPlane,
        /// Runner-up impact; adapted results always clear it.
        secondary: Option<Q2SecondaryImpact>,
    },
    /// Quake III result.
    Q3 {
        /// Contents at the end position.
        contents: i32,
        /// Hit surface flags.
        surface_flags: i32,
        /// Stored source plane.
        source_plane: BspPlane,
    },
}

/// Trace result.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceResult {
    /// Fraction of the sweep completed.
    pub fraction: f64,
    /// Sweep end position.
    pub end: Vec3,
    /// Start position was solid.
    pub start_solid: bool,
    /// Entire sweep was solid.
    pub all_solid: bool,
    /// Impact contact.
    pub contact: TraceContact,
    /// Hit record.
    pub hit: TraceHit,
    /// Per-family detail.
    pub detail: TraceDetail,
}

/// Point-contents query against shared collision.
#[derive(Debug, Clone, PartialEq)]
pub struct PointContentsQuery {
    /// Sample point.
    pub point: Vec3,
    /// Sample target.
    pub target: QueryTarget,
    /// Collision policy.
    pub policy: TracePolicy,
    /// Selected numeric profile.
    pub numeric: NumericProfile,
    /// Actor to ignore.
    pub pass_actor: Option<ActorId>,
}

/// Point-contents result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointContentsResult {
    /// Quake I contents.
    Q1 {
        /// Raw contents value.
        contents: i32,
    },
    /// Quake II stored and merged contents.
    Q2 {
        /// Stored contents.
        stored: i32,
        /// Merged contents.
        merged: i32,
    },
    /// Quake III contents.
    Q3 {
        /// Raw contents value.
        contents: i32,
    },
}

/// Box-leaf query result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafQueryResult {
    /// Touched leaves.
    pub leaves: Vec<i32>,
    /// Top node, when the query stayed within one subtree.
    pub topnode: Option<i32>,
    /// Limit overflow.
    pub overflow: bool,
}

/// Visibility set selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VisibilityKind {
    /// Potentially visible set.
    Pvs,
    /// Potentially hearable set.
    Phs,
}

/// Shared collision queries. Navigation uses [`SceneQueries::trace`] and
/// [`SceneQueries::point_contents`]; the remaining methods complete the
/// donor contract for providers.
pub trait SceneQueries {
    /// Sweep a body through collision.
    fn trace(&self, query: &TraceQuery) -> TraceResult;
    /// Sample contents at a point.
    fn point_contents(&self, query: &PointContentsQuery) -> PointContentsResult;
    /// Enumerate leaves touched by bounds.
    fn box_leaves(&self, bounds: &Bounds, limit: usize) -> LeafQueryResult;
    /// Area connectivity.
    fn areas_connected(&self, first: i32, second: i32) -> bool;
    /// Cluster visibility.
    fn cluster_visible(&self, from: i32, to: i32, kind: VisibilityKind) -> bool;
}

/// Unwrap a scene query result with the family's `expect` message
/// (`"<family> scene <what>"`). The native fallible cores stay split per
/// family; only the infallible trait wrappers share this helper.
pub fn scene_expect<T, E: std::fmt::Debug>(result: Result<T, E>, family: WorldKind, what: &str) -> T {
    let message = format!("{} scene {what}", family.scene_tag());
    result.expect(&message)
}

/// BSP edge between two vertices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BspEdge {
    /// Endpoint vertex indexes.
    pub vertices: [i32; 2],
}

/// BSP face: plane, winding side, and surface-edge span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BspFace {
    /// Plane index.
    pub plane: usize,
    /// Winding faces the plane back side.
    pub back: bool,
    /// Surface-edge span.
    pub edges: IndexRange,
}

/// Quake I world model: navigation reads bounds and the face span.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1WorldModel {
    /// Model bounds.
    pub bounds: Bounds,
    /// Face span.
    pub faces: IndexRange,
}

/// Quake I leaf: navigation reads bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1Leaf {
    /// Leaf bounds.
    pub bounds: Bounds,
}

/// Quake I world geometry subset consumed by navigation.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1WorldGeometry {
    /// Entity text.
    pub entities: String,
    /// Planes.
    pub planes: Vec<BspPlane>,
    /// Vertices.
    pub vertices: Vec<Vec3>,
    /// Edges.
    pub edges: Vec<BspEdge>,
    /// Signed surface-edge references.
    pub surface_edges: Vec<i32>,
    /// Leaves.
    pub leaves: Vec<Q1Leaf>,
    /// Faces.
    pub faces: Vec<BspFace>,
    /// Models; index zero is the world model.
    pub models: Vec<Q1WorldModel>,
}

/// Quake II world model: navigation reads bounds and the face span.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2WorldModel {
    /// Model bounds.
    pub bounds: Bounds,
    /// Face span.
    pub faces: IndexRange,
}

/// Quake II leaf: navigation reads bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2Leaf {
    /// Leaf bounds.
    pub bounds: Bounds,
}

/// Quake II world geometry subset consumed by navigation.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WorldGeometry {
    /// Entity text.
    pub entities: String,
    /// Planes.
    pub planes: Vec<BspPlane>,
    /// Vertices.
    pub vertices: Vec<Vec3>,
    /// Edges.
    pub edges: Vec<BspEdge>,
    /// Signed surface-edge references.
    pub surface_edges: Vec<i32>,
    /// Leaves.
    pub leaves: Vec<Q2Leaf>,
    /// Faces.
    pub faces: Vec<BspFace>,
    /// Models; index zero is the world model.
    pub models: Vec<Q2WorldModel>,
}

/// Quake III surface kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3SurfaceKind {
    /// Planar surface.
    Planar,
    /// Triangle soup.
    Triangles,
    /// Flare (never walkable).
    Flare,
    /// Bezier patch.
    Patch,
}

/// Quake III vertex: navigation reads position and normal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3BspVertex {
    /// Position.
    pub position: Vec3,
    /// Authored normal.
    pub normal: Vec3,
}

/// Quake III surface: navigation reads kind, spans, and patch dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q3BspSurface {
    /// Surface kind.
    pub kind: Q3SurfaceKind,
    /// Vertex span.
    pub vertices: IndexRange,
    /// Index span.
    pub indices: IndexRange,
    /// Patch width in vertices.
    pub width: usize,
    /// Patch height in vertices.
    pub height: usize,
}

/// Quake III model: navigation reads bounds and the surface span.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3BspModel {
    /// Model bounds.
    pub bounds: Bounds,
    /// Surface span.
    pub surfaces: IndexRange,
}

/// Quake III leaf: navigation reads bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3BspLeaf {
    /// Leaf bounds.
    pub bounds: Bounds,
}

/// Quake III world geometry subset consumed by navigation.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3WorldGeometry {
    /// Entity text.
    pub entities: String,
    /// Models; index zero is the world model.
    pub models: Vec<Q3BspModel>,
    /// Vertices.
    pub vertices: Vec<Q3BspVertex>,
    /// Indexes into surface vertex spans.
    pub indices: Vec<i32>,
    /// Surfaces.
    pub surfaces: Vec<Q3BspSurface>,
    /// Leaves.
    pub leaves: Vec<Q3BspLeaf>,
}

/// Decoded world geometry subset consumed by navigation.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodedWorld {
    /// Quake I geometry.
    Q1(Q1WorldGeometry),
    /// Quake II geometry.
    Q2(Q2WorldGeometry),
    /// Quake III geometry.
    Q3(Q3WorldGeometry),
}

impl WorldKind {
    /// Donor format name.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            WorldKind::Q1Bsp => "q1-bsp",
            WorldKind::Q2Bsp => "q2-bsp",
            WorldKind::Q3Bsp => "q3-bsp",
        }
    }

    /// Short tag used by the infallible scene-query wrappers (`q1`, `q2`, `q3`).
    #[must_use]
    pub fn scene_tag(self) -> &'static str {
        match self {
            WorldKind::Q1Bsp => "q1",
            WorldKind::Q2Bsp => "q2",
            WorldKind::Q3Bsp => "q3",
        }
    }
}

impl DecodedWorld {
    /// World family.
    #[must_use]
    pub fn kind(&self) -> WorldKind {
        match self {
            DecodedWorld::Q1(_) => WorldKind::Q1Bsp,
            DecodedWorld::Q2(_) => WorldKind::Q2Bsp,
            DecodedWorld::Q3(_) => WorldKind::Q3Bsp,
        }
    }

    /// Entity text.
    #[must_use]
    pub fn entities(&self) -> &str {
        match self {
            DecodedWorld::Q1(world) => &world.entities,
            DecodedWorld::Q2(world) => &world.entities,
            DecodedWorld::Q3(world) => &world.entities,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panic_text(result: Result<(), Box<dyn std::any::Any + Send>>) -> String {
        match result {
            Ok(()) => panic!("expected a panic"),
            Err(payload) => payload
                .downcast::<String>()
                .map(|text| text.as_str().to_owned())
                .unwrap_or_else(|_| panic!("expected a string panic payload")),
        }
    }

    #[test]
    fn scene_tags_match_wrapper_prefixes() {
        assert_eq!(WorldKind::Q1Bsp.scene_tag(), "q1");
        assert_eq!(WorldKind::Q2Bsp.scene_tag(), "q2");
        assert_eq!(WorldKind::Q3Bsp.scene_tag(), "q3");
    }

    #[test]
    #[allow(clippy::unnecessary_literal_unwrap)]
    fn scene_expect_matches_literal_expects() {
        assert_eq!(scene_expect::<u8, String>(Ok(7), WorldKind::Q1Bsp, "trace"), 7);
        for (family, tag) in [
            (WorldKind::Q1Bsp, "q1"),
            (WorldKind::Q2Bsp, "q2"),
            (WorldKind::Q3Bsp, "q3"),
        ] {
            for what in ["trace", "contents", "leaves", "areas", "visibility"] {
                let message = format!("{tag} scene {what}");
                let literal = panic_text(std::panic::catch_unwind(|| {
                    Err::<u8, String>("boom".to_owned()).expect(&message);
                }));
                let shared = panic_text(std::panic::catch_unwind(|| {
                    scene_expect::<u8, String>(Err("boom".to_owned()), family, what);
                }));
                assert_eq!(shared, literal);
                assert!(shared.starts_with(&message));
            }
        }
    }
}
