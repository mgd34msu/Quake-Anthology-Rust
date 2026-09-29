//! Behavior-owned shapes consumed by navigation, mirrored from
//! `src/bots/behavior/q3/navigation-types.ts` (`TravelType`,
//! `BotMovementPrediction`), `src/bots/behavior/q3/travel/types.ts`
//! (`BotTravelPredictionResult`), and `src/bots/behavior/prediction.ts`
//! (`BotMovementStop`). Canonical behavior definitions live with the
//! behavior port; this module pins the navigation-visible subset.

use qa_core::math::Vec3;

/// Source travel types from `be_aas.h`. Values 1-19 index the travel-flag
/// table; the mask and team bits ride above them.
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

/// Client-movement prediction outcome. AAS generation reads the end
/// state, frame count, stop event, and end area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotTravelPredictionResult {
    /// Predicted end position.
    pub end: Vec3,
    /// Predicted end velocity.
    pub velocity: Vec3,
    /// Simulated frames.
    pub frames: i32,
    /// Stop-event bits.
    pub stop_event: i32,
    /// End area, when the predictor resolved one.
    pub end_area: Option<i32>,
}

/// Classified movement stop: event bits, stop origin, and stop area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotMovementStop {
    /// Stop-event bits.
    pub events: i32,
    /// Stop origin.
    pub origin: Vec3,
    /// Stop area.
    pub area: Option<i32>,
}
