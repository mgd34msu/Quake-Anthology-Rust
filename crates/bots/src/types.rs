//! Navigation graph vocabulary ported from
//! `src/bots/navigation/types.ts`: travel modes, profiles, nodes, edges,
//! graphs, traversal admission, entity state, world seams, routes, and
//! metadata cost queries.

use std::collections::HashSet;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::aas::AasAsset;
use crate::content::ContentDigest;
use crate::movement_contract::MovementProfile;
use crate::nav::KexNavigationAsset;
use crate::scene::{BodyShape, SceneQueries, TracePolicy, WorldKind};

/// Traversal capability of an edge or profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TravelMode {
    /// Walk.
    Walk,
    /// Crouch.
    Crouch,
    /// Jump.
    Jump,
    /// Drop.
    Drop,
    /// Swim.
    Swim,
    /// Water jump.
    WaterJump,
    /// Ladder.
    Ladder,
    /// Teleport.
    Teleport,
    /// Mover ride.
    Mover,
    /// Jump pad.
    JumpPad,
    /// Rocket jump.
    RocketJump,
    /// BFG jump.
    BfgJump,
    /// Grapple.
    Grapple,
    /// Double jump.
    DoubleJump,
    /// Ramp jump.
    RampJump,
    /// Strafe jump.
    StrafeJump,
    /// Unsupported source travel type.
    Unknown,
}

impl TravelMode {
    /// Donor mode name.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            TravelMode::Walk => "walk",
            TravelMode::Crouch => "crouch",
            TravelMode::Jump => "jump",
            TravelMode::Drop => "drop",
            TravelMode::Swim => "swim",
            TravelMode::WaterJump => "water-jump",
            TravelMode::Ladder => "ladder",
            TravelMode::Teleport => "teleport",
            TravelMode::Mover => "mover",
            TravelMode::JumpPad => "jump-pad",
            TravelMode::RocketJump => "rocket-jump",
            TravelMode::BfgJump => "bfg-jump",
            TravelMode::Grapple => "grapple",
            TravelMode::DoubleJump => "double-jump",
            TravelMode::RampJump => "ramp-jump",
            TravelMode::StrafeJump => "strafe-jump",
            TravelMode::Unknown => "unknown",
        }
    }
}

/// Team side for team-filtered traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Team {
    /// Red team.
    Red,
    /// Blue team.
    Blue,
}

/// Map identity: name, BSP family, and content digest.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NavigationMapIdentity {
    /// Map name.
    pub name: String,
    /// BSP family.
    pub format: WorldKind,
    /// Content digest.
    pub digest: ContentDigest,
}

/// Character traversal profile.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationProfile {
    /// Selected movement.
    pub movement: MovementProfile,
    /// Standing collision body.
    pub shape: BodyShape,
    /// Crouched collision body.
    pub crouched_shape: Option<BodyShape>,
    /// Collision policy; must follow the selected movement.
    pub policy: TracePolicy,
    /// Admitted travel modes.
    pub capabilities: HashSet<TravelMode>,
    /// Maximum step height.
    pub maximum_step: f64,
    /// Minimum walkable floor normal Z.
    pub minimum_floor_normal: f64,
    /// Maximum survivable drop.
    pub maximum_drop: f64,
    /// Team side, when traversal is team-filtered.
    pub team: Option<Team>,
    /// Monster traversal rules.
    pub monster: bool,
}

/// Source record behind a node or edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NavigationSource {
    /// AAS area, with an optional source reachability index.
    Aas {
        /// Source area.
        area: i32,
        /// Source reachability index.
        reachability: Option<i32>,
    },
    /// Kex node, with an optional source link index.
    Kex {
        /// NAV2 or NAV3 generation.
        generation: KexGeneration,
        /// Source node.
        node: i32,
        /// Source link index.
        link: Option<i32>,
    },
    /// Constructed sample with an optional surface or leaf source.
    Constructed {
        /// Source surface.
        surface: Option<i32>,
        /// Source leaf.
        leaf: Option<i32>,
    },
}

/// Kex navigation generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KexGeneration {
    /// NAV2 (Quake I rerelease).
    Nav2,
    /// NAV3 (Quake II rerelease).
    Nav3,
}

/// Navigation node.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationNode {
    /// Node id.
    pub id: i32,
    /// Traversal origin.
    pub origin: Vec3,
    /// Node bounds.
    pub bounds: Bounds,
    /// Node radius (Kex sources).
    pub radius: f64,
    /// Medium contents at the origin.
    pub contents: i32,
    /// Source flags.
    pub flags: i32,
    /// Source presence bits.
    pub presence: i32,
    /// Source cluster, when the asset clusters nodes.
    pub source_cluster: Option<i32>,
    /// Source record.
    pub source: NavigationSource,
}

/// Traversal funnel hint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraversalHint {
    /// Staging funnel point.
    pub funnel: Vec3,
    /// Traversal start.
    pub start: Vec3,
    /// Traversal end.
    pub end: Vec3,
    /// Ladder plane point, when the traversal climbs a ladder.
    pub ladder_plane: Option<Vec3>,
}

/// Entity binding behind a mover/teleport edge.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationEntityBinding {
    /// Source model ordinal.
    pub model: Option<i32>,
    /// Binding bounds.
    pub bounds: Bounds,
    /// Raw source words.
    pub raw: Vec<i32>,
}

/// Navigation edge.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationEdge {
    /// Edge id.
    pub id: i32,
    /// Origin node.
    pub from: i32,
    /// Destination node.
    pub to: i32,
    /// Travel mode.
    pub mode: TravelMode,
    /// Traversal start.
    pub start: Vec3,
    /// Traversal end.
    pub end: Vec3,
    /// Estimated travel seconds.
    pub travel_seconds: f64,
    /// Source travel type.
    pub source_travel_type: i32,
    /// Source flags.
    pub source_flags: i32,
    /// Traversal hint.
    pub hint: Option<TraversalHint>,
    /// Entity binding.
    pub entity: Option<NavigationEntityBinding>,
    /// Source record.
    pub source: NavigationSource,
}

/// Parsed navigation asset behind a graph.
#[derive(Debug, Clone, PartialEq)]
pub enum NavigationAsset {
    /// AAS asset.
    Aas(Box<AasAsset>),
    /// Kex NAV2/NAV3 asset.
    Kex(KexNavigationAsset),
}

/// Navigation graph: nodes, directed edges, structural clusters, and the
/// rejected source records with their reasons.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationGraph {
    /// Map identity.
    pub map: NavigationMapIdentity,
    /// Traversal profile.
    pub profile: NavigationProfile,
    /// Source asset, when the graph was built from one.
    pub asset: Option<NavigationAsset>,
    /// Nodes.
    pub nodes: Vec<NavigationNode>,
    /// Directed edges.
    pub edges: Vec<NavigationEdge>,
    /// Structural directed components. Route publication separately
    /// requires movement admission.
    pub clusters: Vec<Vec<i32>>,
    /// Rejected source records.
    pub rejected: Vec<RejectedSource>,
}

/// Rejected source record with its reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedSource {
    /// Source record.
    pub source: NavigationSource,
    /// Rejection reason.
    pub reason: String,
}

/// One movement-admitted traversal.
#[derive(Debug, Clone, PartialEq)]
pub struct TraversalRequest {
    /// Traversal start.
    pub from: Vec3,
    /// Traversal end.
    pub to: Vec3,
    /// Travel mode.
    pub mode: TravelMode,
    /// Traversal hint.
    pub hint: Option<TraversalHint>,
    /// Entity binding.
    pub entity: Option<NavigationEntityBinding>,
}

/// Traversal admission verdict.
#[derive(Debug, Clone, PartialEq)]
pub enum TraversalAdmission {
    /// Admitted with a duration and trajectory.
    Admitted {
        /// Traversal seconds.
        seconds: f64,
        /// Movement trajectory including both endpoints.
        trajectory: Vec<Vec3>,
    },
    /// Refused with a reason.
    Refused {
        /// Refusal reason.
        reason: String,
    },
}

impl TraversalAdmission {
    /// Admission verdict.
    #[must_use]
    pub fn admitted(&self) -> bool {
        matches!(self, TraversalAdmission::Admitted { .. })
    }
}

/// Detached route-prediction session. Carries movement state between
/// successful segments; discard the session after failure.
pub trait NavigationRoutePrediction {
    /// Admit one traversal against the session's detached state.
    fn admit(&mut self, request: &TraversalRequest) -> Result<TraversalAdmission, crate::error::BotsError>;
}

/// Live elevator state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElevatorState {
    /// Current origin.
    pub origin: Vec3,
    /// Bottom endpoint.
    pub bottom: Vec3,
    /// Top endpoint.
    pub top: Vec3,
    /// Travel phase.
    pub phase: ElevatorPhase,
}

/// Elevator travel phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ElevatorPhase {
    /// At the bottom.
    Bottom,
    /// Moving up.
    Up,
    /// At the top.
    Top,
    /// Moving down.
    Down,
}

/// Train stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrainStop {
    /// Stop id.
    pub id: i32,
    /// Stop origin.
    pub origin: Vec3,
    /// Next stop id.
    pub next: Option<i32>,
    /// Wait seconds at this stop.
    pub wait: f64,
    /// Corner teleport rather than a boarding stop.
    pub teleport: bool,
}

/// Live train state.
#[derive(Debug, Clone, PartialEq)]
pub struct TrainState {
    /// Current origin.
    pub origin: Vec3,
    /// Running.
    pub running: bool,
    /// Route stops.
    pub stops: Vec<TrainStop>,
}

/// Live entity state behind a binding.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationEntityState {
    /// Entity actor.
    pub actor: ActorId,
    /// Enabled for traversal.
    pub enabled: bool,
    /// Locked against traversal.
    pub locked: bool,
    /// Entity bounds.
    pub bounds: Bounds,
    /// Entity velocity.
    pub velocity: Vec3,
    /// Current destination, when the mover has one.
    pub destination: Option<Vec3>,
    /// Elevator state, when the entity is an elevator.
    pub elevator: Option<ElevatorState>,
    /// Train state, when the entity is a train.
    pub train: Option<TrainState>,
}

/// Shared world state for navigation. Prediction must use the selected
/// movement provider without committing actors.
pub trait NavigationWorld {
    /// Shared collision queries.
    fn scene(&self) -> &dyn SceneQueries;
    /// Actor ignored by collision queries.
    fn pass_actor(&self) -> Option<ActorId>;
    /// Revision; changes whenever collision, movers, hazards, or
    /// traversal availability change.
    fn revision(&self) -> i64;
    /// Admit one traversal.
    fn admit(&self, request: &TraversalRequest, profile: &NavigationProfile) -> TraversalAdmission;
    /// Begin a detached route-prediction session.
    fn begin_route(&self, profile: &NavigationProfile) -> Box<dyn NavigationRoutePrediction>;
    /// Resolve an entity binding to live state.
    fn entity(&self, binding: &NavigationEntityBinding) -> Option<NavigationEntityState>;
    /// Hazard test over bounds.
    fn hazard(&self, bounds: &Bounds) -> bool;
}

/// Published route.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationRoute {
    /// Map identity.
    pub map: NavigationMapIdentity,
    /// Node path.
    pub nodes: Vec<i32>,
    /// Edge path.
    pub edges: Vec<NavigationEdge>,
    /// Movement-admitted walking points.
    pub points: Vec<Vec3>,
    /// Total travel seconds.
    pub travel_seconds: f64,
    /// Runtime generation that admitted the route.
    pub generation: i64,
}

/// Route outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum NavigationRouteResult {
    /// Admitted route.
    Route {
        /// Route.
        route: NavigationRoute,
    },
    /// No admissible route.
    Unreachable {
        /// Reason.
        reason: String,
    },
}

/// Metadata cost query; a null origin excludes the approach within the
/// first area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavigationEstimateQuery {
    /// Start node.
    pub start_node: i32,
    /// Goal node.
    pub goal_node: i32,
    /// Start origin, when the approach within the first area counts.
    pub origin: Option<Vec3>,
    /// Travel-flag mask.
    pub travel_flags: Option<i32>,
}

/// Metadata cost outcome. Source centiseconds are estimates, never a
/// movement-admitted trajectory or duration.
#[derive(Debug, Clone, PartialEq)]
pub enum NavigationEstimateResult {
    /// Estimated cost with the first edge of the cheapest tail.
    Estimate {
        /// Travel time in centiseconds, at least one.
        travel_time: i32,
        /// First edge, when the query carried an origin.
        first_edge: Option<NavigationEdge>,
    },
    /// No tail under the mask.
    Unreachable,
}
