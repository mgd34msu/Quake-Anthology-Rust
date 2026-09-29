//! Q3 navigation query words from `src/bots/behavior/q3/navigation-types.ts`
//! (`be_aas.h`, `be_aas_route.c`).
//!
//! Travel types index the travel-flag table; team bits ride above the
//! mask. This module also defines the navigation query shapes (route,
//! predict, alternative goals) and the `BotNavigation` trait the
//! selected navigation provider implements.

use qa_core::math::{Bounds, Vec3};

use crate::behavior::library::goals::BotGoal;
use crate::behavior::q3::movement_state::BotMoveResult;

/// Source travel types from `be_aas.h`. Values 1-19 index the travel-flag
/// table; the mask and team bits ride above them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TravelType;

impl TravelType {
    /// Invalid travel.
    pub const INVALID: i32 = 1;
    /// Walk.
    pub const WALK: i32 = 2;
    /// Crouch.
    pub const CROUCH: i32 = 3;
    /// Barrier jump.
    pub const BARRIERJUMP: i32 = 4;
    /// Jump.
    pub const JUMP: i32 = 5;
    /// Ladder.
    pub const LADDER: i32 = 6;
    /// Walk off a ledge.
    pub const WALKOFFLEDGE: i32 = 7;
    /// Swim.
    pub const SWIM: i32 = 8;
    /// Water jump.
    pub const WATERJUMP: i32 = 9;
    /// Teleport.
    pub const TELEPORT: i32 = 10;
    /// Elevator.
    pub const ELEVATOR: i32 = 11;
    /// Rocket jump.
    pub const ROCKETJUMP: i32 = 12;
    /// BFG jump.
    pub const BFGJUMP: i32 = 13;
    /// Grapple hook.
    pub const GRAPPLEHOOK: i32 = 14;
    /// Double jump.
    pub const DOUBLEJUMP: i32 = 15;
    /// Ramp jump.
    pub const RAMPJUMP: i32 = 16;
    /// Strafe jump.
    pub const STRAFEJUMP: i32 = 17;
    /// Jump pad.
    pub const JUMPPAD: i32 = 18;
    /// Func bobbing.
    pub const FUNCBOB: i32 = 19;
    /// Travel-type mask.
    pub const MASK: i32 = 0x00ff_ffff;
    /// Not-team-1 flag.
    pub const NOTTEAM1: i32 = 1 << 24;
    /// Not-team-2 flag.
    pub const NOTTEAM2: i32 = 2 << 24;
}

/// Travel flags (`tfl` bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TravelFlags;

impl TravelFlags {
    /// Invalid.
    pub const INVALID: i32 = 0x0000_0001;
    /// Walk.
    pub const WALK: i32 = 0x0000_0002;
    /// Crouch.
    pub const CROUCH: i32 = 0x0000_0004;
    /// Barrier jump.
    pub const BARRIERJUMP: i32 = 0x0000_0008;
    /// Jump.
    pub const JUMP: i32 = 0x0000_0010;
    /// Ladder.
    pub const LADDER: i32 = 0x0000_0020;
    /// Walk off ledge.
    pub const WALKOFFLEDGE: i32 = 0x0000_0080;
    /// Swim.
    pub const SWIM: i32 = 0x0000_0100;
    /// Water jump.
    pub const WATERJUMP: i32 = 0x0000_0200;
    /// Teleport.
    pub const TELEPORT: i32 = 0x0000_0400;
    /// Elevator.
    pub const ELEVATOR: i32 = 0x0000_0800;
    /// Rocket jump.
    pub const ROCKETJUMP: i32 = 0x0000_1000;
    /// BFG jump.
    pub const BFGJUMP: i32 = 0x0000_2000;
    /// Grapple hook.
    pub const GRAPPLEHOOK: i32 = 0x0000_4000;
    /// Double jump.
    pub const DOUBLEJUMP: i32 = 0x0000_8000;
    /// Ramp jump.
    pub const RAMPJUMP: i32 = 0x0001_0000;
    /// Strafe jump.
    pub const STRAFEJUMP: i32 = 0x0002_0000;
    /// Jump pad.
    pub const JUMPPAD: i32 = 0x0004_0000;
    /// Air.
    pub const AIR: i32 = 0x0008_0000;
    /// Water.
    pub const WATER: i32 = 0x0010_0000;
    /// Slime.
    pub const SLIME: i32 = 0x0020_0000;
    /// Lava.
    pub const LAVA: i32 = 0x0040_0000;
    /// Do not enter.
    pub const DONOTENTER: i32 = 0x0080_0000;
    /// Func bob.
    pub const FUNCBOB: i32 = 0x0100_0000;
    /// Flight.
    pub const FLIGHT: i32 = 0x0200_0000;
    /// Bridge.
    pub const BRIDGE: i32 = 0x0400_0000;
    /// Not team 1.
    pub const NOTTEAM1: i32 = 0x0800_0000;
    /// Not team 2.
    pub const NOTTEAM2: i32 = 0x1000_0000;
    /// Default bot travel flags.
    pub const DEFAULT: i32 = 0x011c_0fbe;
}

/// Travel flag for a travel type. The source tests zero `tfl`, so
/// reachability team bits are ignored.
#[must_use]
pub fn travel_flag_for_type(travel_type: i32) -> i32 {
    const TABLE: [i32; 20] = [
        TravelFlags::INVALID,
        TravelFlags::INVALID,
        TravelFlags::WALK,
        TravelFlags::CROUCH,
        TravelFlags::BARRIERJUMP,
        TravelFlags::JUMP,
        TravelFlags::LADDER,
        TravelFlags::WALKOFFLEDGE,
        TravelFlags::SWIM,
        TravelFlags::WATERJUMP,
        TravelFlags::TELEPORT,
        TravelFlags::ELEVATOR,
        TravelFlags::ROCKETJUMP,
        TravelFlags::BFGJUMP,
        TravelFlags::GRAPPLEHOOK,
        TravelFlags::DOUBLEJUMP,
        TravelFlags::RAMPJUMP,
        TravelFlags::STRAFEJUMP,
        TravelFlags::JUMPPAD,
        TravelFlags::FUNCBOB,
    ];
    let index = (travel_type & TravelType::MASK) as usize;
    TABLE.get(index).copied().unwrap_or(TravelFlags::INVALID)
}

/// Route query (`AAS_AreaRouteToGoal` inputs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteQuery {
    /// Start area.
    pub area: i32,
    /// Start origin (required: the source reads it for intercluster queries).
    pub origin: Vec3,
    /// Goal area.
    pub goal_area: i32,
    /// Travel flags.
    pub travel_flags: i32,
}

/// Area travel-time query: origin may be null for same-cluster times.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AreaTravelTimeQuery {
    /// Start area.
    pub area: i32,
    /// Start origin, or `None`.
    pub origin: Option<Vec3>,
    /// Goal area.
    pub goal_area: i32,
    /// Travel flags.
    pub travel_flags: i32,
}

/// Alternative route goal kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AlternativeRouteType;

impl AlternativeRouteType {
    /// All portals.
    pub const ALL: i32 = 1;
    /// Cluster portals.
    pub const CLUSTER_PORTALS: i32 = 2;
    /// View portals.
    pub const VIEW_PORTALS: i32 = 4;
}

/// Alternative route query (`AAS_AlternativeRouteGoals`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlternativeRouteQuery {
    /// Start origin.
    pub start: Vec3,
    /// Start area.
    pub start_area: i32,
    /// Goal origin (unused by the source; coordinates never read).
    pub goal: Vec3,
    /// Goal area.
    pub goal_area: i32,
    /// Travel flags.
    pub travel_flags: i32,
    /// Maximum goals (checked after publishing each goal).
    pub maximum_goals: i32,
    /// Route type bits.
    pub route_type: i32,
}

/// One alternative route goal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlternativeGoal {
    /// Goal origin.
    pub origin: Vec3,
    /// Area (0 is the source fallback).
    pub area: i32,
    /// Travel time from start.
    pub start_travel_time: i32,
    /// Travel time to goal.
    pub goal_travel_time: i32,
    /// Extra travel time.
    pub extra_travel_time: i32,
}

/// Route result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteResult {
    /// Route found.
    Found {
        /// Travel time in 10ms units.
        travel_time: i32,
        /// Next reachability number.
        next_reachability: i32,
    },
    /// Unreachable.
    Unreachable,
}

/// Route prediction stop events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RouteStopEvent;

impl RouteStopEvent {
    /// None.
    pub const NONE: i32 = 0;
    /// No route.
    pub const NO_ROUTE: i32 = 1;
    /// Use travel type.
    pub const USE_TRAVEL_TYPE: i32 = 2;
    /// Enter contents.
    pub const ENTER_CONTENTS: i32 = 4;
    /// Enter area.
    pub const ENTER_AREA: i32 = 8;
}

/// Route prediction query (`AAS_PredictRoute`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PredictRouteQuery {
    /// Start area.
    pub area: i32,
    /// Start origin.
    pub origin: Vec3,
    /// Goal area.
    pub goal_area: i32,
    /// Travel flags.
    pub travel_flags: i32,
    /// Maximum areas to walk.
    pub maximum_areas: i32,
    /// Maximum time.
    pub maximum_time: i32,
    /// Stop event mask.
    pub stop_event: i32,
    /// Stop contents.
    pub stop_contents: i32,
    /// Stop travel flags.
    pub stop_travel_flags: i32,
    /// Stop area.
    pub stop_area: i32,
}

/// Predicted route (`aas_predictroute_t` plus the return value).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PredictedRoute {
    /// Whether prediction succeeded.
    pub succeeded: bool,
    /// Stop event raised.
    pub stop_event: i32,
    /// End area.
    pub end_area: i32,
    /// End contents.
    pub end_contents: i32,
    /// End travel flags.
    pub end_travel_flags: i32,
    /// End position.
    pub end_position: Vec3,
    /// Predicted time.
    pub time: i32,
}

/// Navigation area summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotNavigationArea {
    /// Contents.
    pub contents: i32,
    /// Flags.
    pub flags: i32,
    /// Presence type.
    pub presence_type: i32,
    /// Cluster.
    pub cluster: i32,
    /// Reachable area count.
    pub reachable_area_count: i32,
}

/// Client-movement prediction query for AAS generation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotMovementPrediction {
    /// Client slot to predict as.
    pub entity_num: i32,
    /// Prediction origin.
    pub origin: Vec3,
    /// Presence bounds selector (2 = standing, 4 = crouched).
    pub presence: i32,
    /// Start grounded.
    pub on_ground: bool,
    /// Initial velocity.
    pub velocity: Vec3,
    /// Command velocity.
    pub command_move: Vec3,
    /// Frames applying the command velocity.
    pub command_frames: i32,
    /// Maximum simulated frames.
    pub max_frames: i32,
    /// Seconds per frame.
    pub frame_time: f32,
    /// Stop-event mask.
    pub stop_events: i32,
    /// Area stop target.
    pub stop_area: i32,
    /// Emit visualization lines.
    pub visualize: bool,
}

/// Navigation provider: every operation queries the selected shared
/// navigation and movement providers.
pub trait BotNavigation {
    /// Whether navigation data is ready.
    fn ready(&self) -> bool;
    /// Area containing an origin.
    fn point_area(&self, origin: Vec3) -> i32;
    /// Reachability area for an origin and client.
    fn reachability_area(&self, origin: Vec3, client: i32) -> i32;
    /// Fuzzy reachability area for an origin.
    fn fuzzy_point_reachability_area(&self, origin: Vec3) -> i32;
    /// Area summary.
    fn area(&self, number: i32) -> BotNavigationArea;
    /// Areas crossed by a segment.
    fn trace_areas(&self, start: Vec3, end: Vec3, maximum: usize) -> Vec<(i32, Vec3)>;
    /// Areas overlapping bounds.
    fn bbox_areas(&self, bounds: &Bounds) -> Vec<i32>;
    /// Enable or disable an area.
    fn set_area_enabled(&mut self, area: i32, enabled: bool);
    /// Travel time between areas in 10ms units.
    fn area_travel_time_to_goal(&mut self, query: &AreaTravelTimeQuery) -> i32;
    /// Route from an area to a goal area.
    fn route(&mut self, query: &RouteQuery) -> RouteResult;
    /// Predict a route with stop events.
    fn predict_route(&mut self, query: &PredictRouteQuery) -> PredictedRoute;
    /// Alternative route goals.
    fn alternative_route_goals(&mut self, query: &AlternativeRouteQuery) -> Vec<AlternativeGoal>;
    /// Drive movement toward a goal.
    fn move_to_goal(&mut self, result: &mut BotMoveResult, move_state: i32, goal: &BotGoal, travel_flags: i32);
    /// Drive movement in a direction.
    fn move_in_direction(&mut self, move_state: i32, direction: Vec3, speed: f32, move_type: i32) -> bool;
    /// Movement view target with lookahead.
    fn movement_view_target(&self, move_state: i32, goal: &BotGoal, travel_flags: i32, look_ahead: f32)
        -> Option<Vec3>;
    /// Visible position prediction toward a goal.
    fn predict_visible_position(&self, origin: Vec3, area: i32, goal: &BotGoal, travel_flags: i32) -> Option<Vec3>;
    /// Whether an origin is in swimming contents.
    fn swimming(&self, origin: Vec3) -> bool;
    /// Presence bounds for standing (2) or crouched (4).
    fn presence_bounds(&self, presence: i32) -> Bounds;
}
