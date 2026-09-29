//! Quake II movement representations of source ABI fields.
//!
//! Donor provenance: `src/movement/q2/types.ts` (from Quake II `q_shared.h`
//! and rerelease `game.h`).
//!
//! Contents and mask words reuse [`crate::collision::q2`]; only the current
//! flags and enums the collision module does not carry are defined here.

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::{NumericOps, NumericProfile};
use qa_core::time::{ClockProfile, FrameContext};

pub use crate::collision::q2::{
    CONTENTS_LADDER, CONTENTS_LAVA, CONTENTS_MONSTER, CONTENTS_PLAYER, CONTENTS_PLAYERCLIP, CONTENTS_SLIME,
    CONTENTS_SOLID, CONTENTS_WATER, CONTENTS_WINDOW, MASK_CLASSIC_PLAYERSOLID, MASK_DEADSOLID, MASK_PLAYERSOLID,
    MASK_SOLID, MASK_WATER, MAXTOUCH, MIN_STEP_NORMAL, STEPSIZE, STOP_EPSILON, SURF_SLICK,
};
pub use crate::collision::LeafContents;

/// Current flags the collision module does not carry.
pub const CONTENTS_CURRENT_0: i32 = 1 << 18;
/// +Y current.
pub const CONTENTS_CURRENT_90: i32 = 1 << 19;
/// -X current.
pub const CONTENTS_CURRENT_180: i32 = 1 << 20;
/// -Y current.
pub const CONTENTS_CURRENT_270: i32 = 1 << 21;
/// +Z current.
pub const CONTENTS_CURRENT_UP: i32 = 1 << 22;
/// -Z current.
pub const CONTENTS_CURRENT_DOWN: i32 = 1 << 23;
/// Water-jump suppression.
pub const CONTENTS_NO_WATERJUMP: i32 = 1 << 13;
/// No contents.
pub const CONTENTS_NONE: i32 = 0;
/// Any current.
pub const MASK_CURRENT: i32 = CONTENTS_CURRENT_0
    | CONTENTS_CURRENT_90
    | CONTENTS_CURRENT_180
    | CONTENTS_CURRENT_270
    | CONTENTS_CURRENT_UP
    | CONTENTS_CURRENT_DOWN;

/// Source tuple vector (donor `Vec3`).
pub type SrcVec3 = [f64; 3];
/// Source four-vector (donor `Vec4`).
pub type SrcVec4 = [f64; 4];
/// Axis indices.
pub const AXES: [usize; 3] = [0, 1, 2];
/// Pitch axis.
pub const PITCH: usize = 0;
/// Yaw axis.
pub const YAW: usize = 1;
/// Roll axis.
pub const ROLL: usize = 2;

/// Index a source tuple with the donor's range error.
pub fn element(values: &[f64], index: usize) -> f64 {
    *values
        .get(index)
        .unwrap_or_else(|| panic!("Quake II movement index {index} outside {}", values.len()))
}

/// Classic pmove types (`PmTypeT`).
pub mod pm_type {
    /// Normal.
    pub const NORMAL: i32 = 0;
    /// Spectator.
    pub const SPECTATOR: i32 = 1;
    /// Dead.
    pub const DEAD: i32 = 2;
    /// Gib.
    pub const GIB: i32 = 3;
    /// Freeze.
    pub const FREEZE: i32 = 4;
}

/// Rerelease pmove types (`KexPmTypeT`).
pub mod kex_pm_type {
    /// Normal.
    pub const NORMAL: i32 = 0;
    /// Grapple.
    pub const GRAPPLE: i32 = 1;
    /// Noclip.
    pub const NOCLIP: i32 = 2;
    /// Spectator.
    pub const SPECTATOR: i32 = 3;
    /// Dead.
    pub const DEAD: i32 = 4;
    /// Gib.
    pub const GIB: i32 = 5;
    /// Freeze.
    pub const FREEZE: i32 = 6;
}

/// Pmove flags (`PmflagsT`).
pub mod pm_flags {
    /// No flags.
    pub const NONE: i32 = 0;
    /// Ducked.
    pub const DUCKED: i32 = 1;
    /// Jump held.
    pub const JUMP_HELD: i32 = 2;
    /// On ground.
    pub const ON_GROUND: i32 = 4;
    /// Water-jump timer.
    pub const TIME_WATERJUMP: i32 = 8;
    /// Landing timer.
    pub const TIME_LAND: i32 = 16;
    /// Teleport timer.
    pub const TIME_TELEPORT: i32 = 32;
    /// No positional prediction.
    pub const NO_POSITIONAL_PREDICTION: i32 = 64;
    /// On ladder.
    pub const ON_LADDER: i32 = 128;
    /// No angular prediction.
    pub const NO_ANGULAR_PREDICTION: i32 = 256;
    /// Ignore player collision.
    pub const IGNORE_PLAYER_COLLISION: i32 = 512;
    /// Trick-jump timer.
    pub const TIME_TRICK: i32 = 1024;
}

/// Command buttons (`ButtonT`).
pub mod button {
    /// No button.
    pub const NONE: i32 = 0;
    /// Attack.
    pub const ATTACK: i32 = 1;
    /// Use.
    pub const USE: i32 = 2;
    /// Holster.
    pub const HOLSTER: i32 = 4;
    /// Jump.
    pub const JUMP: i32 = 8;
    /// Crouch.
    pub const CROUCH: i32 = 16;
    /// Any.
    pub const ANY: i32 = 128;
}

/// Render flags (`RefdefFlagsT`).
pub mod refdef_flags {
    /// No flags.
    pub const NONE: i32 = 0;
    /// Underwater.
    pub const UNDERWATER: i32 = 1;
}

/// Water levels (`WaterLevelT`).
pub mod water_level {
    /// Dry.
    pub const NONE: i32 = 0;
    /// Feet.
    pub const FEET: i32 = 1;
    /// Waist.
    pub const WAIST: i32 = 2;
    /// Under.
    pub const UNDER: i32 = 3;
}

/// Stuck-object results (`StuckResultT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StuckResult {
    /// Position free.
    GoodPosition,
    /// Position fixed.
    Fixed,
    /// No good position.
    NoGoodPosition,
}

/// Entity touched by a Q2 trace (never none).
pub type MovementEntity = super::super::types::TraceHit;

/// Source clip plane (`CplaneT`).
#[derive(Debug, Clone, PartialEq)]
pub struct CPlane {
    /// Plane normal.
    pub normal: SrcVec3,
    /// Plane distance.
    pub dist: f64,
    /// Plane type.
    pub plane_type: i32,
    /// Sign bits.
    pub signbits: i32,
}

/// Zeroed source plane.
#[must_use]
pub fn plane() -> CPlane {
    CPlane {
        normal: [0.0, 0.0, 0.0],
        dist: 0.0,
        plane_type: 0,
        signbits: 0,
    }
}

/// Source surface (`CsurfaceT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CSurface {
    /// Surface name.
    pub name: String,
    /// Surface flags.
    pub flags: i32,
    /// Surface value.
    pub value: i32,
    /// Surface material.
    pub material: String,
}

/// Source trace (`TraceT`). The donor's `source` back-reference is the
/// native scene trace; adapter-built traces carry it, synthetic test traces
/// leave it empty and rebuild contacts from fields instead.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceT {
    /// Entire sweep inside solid.
    pub allsolid: bool,
    /// Sweep start inside solid.
    pub startsolid: bool,
    /// Travel fraction consumed.
    pub fraction: f64,
    /// Trace end position.
    pub endpos: SrcVec3,
    /// Impact plane.
    pub plane: CPlane,
    /// Impact surface.
    pub surface: Option<CSurface>,
    /// Contents at the impact.
    pub contents: i32,
    /// Hit entity.
    pub ent: Option<MovementEntity>,
    /// Secondary plane (rerelease brush detail).
    pub plane2: CPlane,
    /// Secondary surface.
    pub surface2: Option<CSurface>,
    /// Native scene trace (donor `source`).
    pub native: Option<Q2Trace>,
}

impl super::super::swept_body::SweptBodyTrace for TraceT {
    fn fraction(&self) -> f64 {
        self.fraction
    }
    fn end(&self) -> Vec3 {
        Vec3 {
            x: self.endpos[0] as f32,
            y: self.endpos[1] as f32,
            z: self.endpos[2] as f32,
        }
    }
    fn all_solid(&self) -> bool {
        self.allsolid
    }
    fn start_solid(&self) -> bool {
        self.startsolid
    }
}

/// Classic pmove state words (`pm.s`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicPmoveState {
    /// Pmove type word.
    pub pm_type: i32,
    /// Origin in eighths.
    pub origin: [i32; 3],
    /// Velocity in eighths.
    pub velocity: [i32; 3],
    /// Pmove flags.
    pub pm_flags: i32,
    /// Pmove timer in 8ms units.
    pub pm_time: i32,
    /// Gravity.
    pub gravity: f64,
    /// Delta angles.
    pub delta_angles: [i32; 3],
}

/// Classic pmove command words (`pm.cmd`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicPmoveCmd {
    /// Command length in milliseconds.
    pub msec: i32,
    /// Short-encoded angles.
    pub angles: [i32; 3],
    /// Forward impulse.
    pub forwardmove: f64,
    /// Side impulse.
    pub sidemove: f64,
    /// Vertical impulse.
    pub upmove: f64,
    /// Button bitmask.
    pub buttons: i32,
    /// Impulse byte.
    pub impulse: i32,
    /// Light level.
    pub lightlevel: i32,
}

/// Classic pmove block (`ClassicPmove`). Trace callbacks are boxed so tests
/// inject fake worlds without a scene backend.
pub struct ClassicPmove<'a> {
    /// State words.
    pub s: ClassicPmoveState,
    /// Command words.
    pub cmd: ClassicPmoveCmd,
    /// Snap the initial position.
    pub snapinitial: bool,
    /// Touch count.
    pub numtouch: usize,
    /// Touched entities.
    pub touchents: Vec<MovementEntity>,
    /// Touch traces.
    pub touchtraces: Vec<TraceT>,
    /// View angles.
    pub viewangles: SrcVec3,
    /// View height.
    pub viewheight: f64,
    /// Hull minimums.
    pub mins: SrcVec3,
    /// Hull maximums.
    pub maxs: SrcVec3,
    /// Ground entity.
    pub groundentity: Option<MovementEntity>,
    /// Water type.
    pub watertype: i32,
    /// Water level.
    pub waterlevel: i32,
    /// Trace callback.
    pub trace: Box<dyn FnMut(SrcVec3, SrcVec3, SrcVec3, SrcVec3) -> TraceT + 'a>,
    /// Point-contents callback.
    pub pointcontents: Box<dyn FnMut(SrcVec3) -> i32 + 'a>,
    /// Selected character dimensions.
    pub character_bounds: Bounds,
    /// Requested client body bounds.
    pub body_bounds: Option<Bounds>,
    /// Previously accepted bounds.
    pub previous_bounds: Option<Bounds>,
}

/// Rerelease touch list (`KexTouchListT`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KexTouchList {
    /// Touch count.
    pub num: usize,
    /// Touch traces.
    pub traces: Vec<TraceT>,
}

/// Rerelease pmove state words.
#[derive(Debug, Clone, PartialEq)]
pub struct KexPmoveState {
    /// Pmove type word.
    pub pm_type: i32,
    /// Origin.
    pub origin: SrcVec3,
    /// Velocity.
    pub velocity: SrcVec3,
    /// Pmove flags.
    pub pm_flags: i32,
    /// Pmove timer in milliseconds.
    pub pm_time: i32,
    /// Gravity.
    pub gravity: f64,
    /// Delta angles.
    pub delta_angles: SrcVec3,
    /// View height.
    pub viewheight: f64,
}

/// Rerelease pmove command words.
#[derive(Debug, Clone, PartialEq)]
pub struct KexPmoveCmd {
    /// Command length in milliseconds.
    pub msec: i32,
    /// View angles.
    pub angles: SrcVec3,
    /// Forward impulse.
    pub forwardmove: f64,
    /// Side impulse.
    pub sidemove: f64,
    /// Button bitmask.
    pub buttons: i32,
    /// Server frame.
    pub server_frame: i32,
}

/// Rerelease pmove block (`KexPmoveT`).
pub struct KexPmove<'a> {
    /// State words.
    pub s: KexPmoveState,
    /// Command words.
    pub cmd: KexPmoveCmd,
    /// Snap the initial position.
    pub snapinitial: bool,
    /// Touch list.
    pub touch: KexTouchList,
    /// View angles.
    pub viewangles: SrcVec3,
    /// Hull minimums.
    pub mins: SrcVec3,
    /// Hull maximums.
    pub maxs: SrcVec3,
    /// Ground entity.
    pub groundentity: Option<MovementEntity>,
    /// Ground plane.
    pub groundplane: CPlane,
    /// Water type.
    pub watertype: i32,
    /// Water level.
    pub waterlevel: i32,
    /// Moving player.
    pub player: Option<MovementEntity>,
    /// Entity trace callback.
    pub trace: Box<dyn FnMut(SrcVec3, SrcVec3, SrcVec3, SrcVec3, Option<MovementEntity>, i32) -> TraceT + 'a>,
    /// World-only clip callback.
    pub clip: Box<dyn FnMut(SrcVec3, SrcVec3, SrcVec3, SrcVec3, i32) -> TraceT + 'a>,
    /// Point-contents callback.
    pub pointcontents: Box<dyn FnMut(SrcVec3) -> i32 + 'a>,
    /// View offset.
    pub viewoffset: SrcVec3,
    /// Screen blend.
    pub screen_blend: SrcVec4,
    /// Render flags.
    pub rdflags: i32,
    /// Jump sound flag.
    pub jump_sound: bool,
    /// Step-clip flag.
    pub step_clip: bool,
    /// Impact delta.
    pub impact_delta: f64,
    /// Selected character dimensions.
    pub character_bounds: Bounds,
    /// Requested client body bounds.
    pub body_bounds: Option<Bounds>,
    /// Previously accepted bounds.
    pub previous_bounds: Option<Bounds>,
}

/// Rerelease pmove configuration (`PmConfigT`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PmConfig {
    /// Air acceleration override (0 selects the classic path).
    pub airaccel: f64,
    /// Nintendo 64 physics variant.
    pub n64_physics: bool,
}

/// Default pmove configuration.
pub const PM_CONFIG_DEFAULT: PmConfig = PmConfig {
    airaccel: 0.0,
    n64_physics: false,
};

use super::super::types::{
    MovementContinuation, MovementInputContinuation, MovementInputFields, MovementOutcome, MovementTouchContact,
    Q2RereleaseUserCommand, Q2UserCommand, TouchSurface, TraceContact, TraceHit, UserCommand,
};

/// Quake II classic movement state, mirroring donor `Q2MovementState`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2MovementState {
    /// Pmove type word.
    pub move_type: i32,
    /// Origin in eighths.
    pub origin_eighths: [i32; 3],
    /// Velocity in eighths.
    pub velocity_eighths: [i32; 3],
    /// Pmove flags.
    pub flags: i32,
    /// Timer in 8ms units.
    pub time_eight_milliseconds: i32,
    /// Gravity.
    pub gravity: f64,
    /// Delta angle shorts.
    pub delta_angle_shorts: [i32; 3],
}

/// Quake II rerelease movement state, mirroring donor
/// `Q2RereleaseMovementState`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseMovementState {
    /// Pmove type word.
    pub move_type: i32,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Pmove flags.
    pub flags: i32,
    /// Timer in milliseconds.
    pub time_milliseconds: i32,
    /// Gravity.
    pub gravity: f64,
    /// Delta angles.
    pub delta_angles: Vec3,
    /// View height.
    pub view_height: f64,
}

/// Either Quake II movement state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q2State {
    /// Classic state.
    Classic(Q2MovementState),
    /// Rerelease state.
    Rerelease(Q2RereleaseMovementState),
}

/// Quake II classic movement profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MovementProfile {
    /// Provider identity.
    pub id: ProviderId,
    /// Frame clock.
    pub clock: ClockProfile,
    /// Numeric profile.
    pub numeric: NumericProfile,
    /// Strafe-jump landing-timer hack.
    pub strafejump_hack: bool,
    /// Air acceleration override.
    pub air_accelerate: f64,
    /// Snap the initial position.
    pub snap_initial: bool,
}

/// Quake II rerelease movement profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseMovementProfile {
    /// Provider identity.
    pub id: ProviderId,
    /// Frame clock.
    pub clock: ClockProfile,
    /// Numeric profile.
    pub numeric: NumericProfile,
    /// Air acceleration override.
    pub air_accelerate: f64,
    /// Nintendo 64 physics variant.
    pub n64_physics: bool,
}

/// Quake II classic movement input.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MovementInput {
    /// Shared input fields.
    pub fields: MovementInputFields,
    /// Source command.
    pub command: Q2UserCommand,
    /// Current state.
    pub state: Q2MovementState,
    /// Movement profile.
    pub profile: Q2MovementProfile,
}

/// Quake II rerelease movement input.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseMovementInput {
    /// Shared input fields.
    pub fields: MovementInputFields,
    /// Source command.
    pub command: Q2RereleaseUserCommand,
    /// Current state.
    pub state: Q2RereleaseMovementState,
    /// Movement profile.
    pub profile: Q2RereleaseMovementProfile,
    /// Previous camera offset.
    pub view_offset: Vec3,
    /// Snap the initial position.
    pub snap_initial: bool,
}

/// Q2 trace plane with donor value types.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2TracePlane {
    /// Plane normal.
    pub normal: Vec3,
    /// Plane distance.
    pub dist: f64,
    /// Plane type.
    pub plane_type: i32,
    /// Sign bits.
    pub signbits: i32,
}

/// Q2 surface detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2Surface {
    /// Surface name.
    pub name: String,
    /// Surface flags.
    pub flags: i32,
    /// Surface value.
    pub value: i32,
    /// Surface material.
    pub material: String,
}

impl From<&CSurface> for Q2Surface {
    fn from(surface: &CSurface) -> Self {
        Self {
            name: surface.name.clone(),
            flags: surface.flags,
            value: surface.value,
            material: surface.material.clone(),
        }
    }
}

impl From<&Q2Surface> for CSurface {
    fn from(surface: &Q2Surface) -> Self {
        Self {
            name: surface.name.clone(),
            flags: surface.flags,
            value: surface.value,
            material: surface.material.clone(),
        }
    }
}

/// Q2 scene trace, mirroring donor `TraceResult` with `kind: "q2"`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2Trace {
    /// Travel fraction consumed.
    pub fraction: f64,
    /// Trace end position.
    pub end: Vec3,
    /// Start inside solid.
    pub start_solid: bool,
    /// Entire sweep inside solid.
    pub all_solid: bool,
    /// Contact surface.
    pub contact: TraceContact,
    /// Hit target.
    pub hit: TraceHit,
    /// Contents at the impact.
    pub contents: i32,
    /// Impact surface.
    pub surface: Option<Q2Surface>,
    /// Stored source plane.
    pub source_plane: Q2TracePlane,
    /// Secondary plane/surface detail.
    pub secondary: Option<(Q2TracePlane, Option<Q2Surface>)>,
}

/// Q2 trace query for movement services.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Hull minimums (source tuples).
    pub mins: SrcVec3,
    /// Hull maximums (source tuples).
    pub maxs: SrcVec3,
    /// Point trace flag (input shape is a point).
    pub point: bool,
    /// Contents mask.
    pub mask: i32,
    /// World-only clip flag.
    pub world_only: bool,
    /// Leaf-contents selection.
    pub leaf: LeafContents,
}

/// Q2 point-contents query.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ContentsQuery {
    /// Sample point.
    pub point: Vec3,
    /// Leaf-contents selection.
    pub leaf: LeafContents,
}

/// Q2 touch contact, mirroring donor `MovementTouchContact` with the
/// rerelease source-trace detail.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TouchContact {
    /// Moving actor.
    pub mover: OwnedActor,
    /// Touch target (never none).
    pub other: TraceHit,
    /// Shared contact view.
    pub shared: MovementTouchContact,
    /// Rerelease source trace (inverted, entity omitted).
    pub source_trace: Option<Q2SourceTrace>,
}

/// Rerelease source-trace detail for touch dispatch.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SourceTrace {
    /// Source trace.
    pub trace: Q2Trace,
    /// Inverted flag.
    pub inverted: bool,
}

impl Q2TouchContact {
    /// Build a contact from a trace.
    pub fn from_trace(
        mover: OwnedActor,
        other: TraceHit,
        plane: Option<qa_core::math::Plane>,
        surface: Option<TouchSurface>,
        source_trace: Option<Q2SourceTrace>,
    ) -> Self {
        Self {
            shared: MovementTouchContact {
                mover: mover.clone(),
                other: other.clone(),
                plane,
                surface,
            },
            mover,
            other,
            source_trace,
        }
    }
}

/// Input application boundary for authoritative Q2 steps.
pub trait Q2InputApplication {
    /// Begin input application for a command.
    fn begin(
        &mut self,
        command: UserCommand,
        frame: &FrameContext,
        state: Q2State,
    ) -> MovementInputContinuation<Q2State>;
    /// End input application with the accepted posture.
    fn end(&mut self, state: Q2State, failed: bool, posture: Option<(Bounds, f64)>) -> MovementContinuation<Q2State>;
}

/// Movement services for Quake II steps.
pub trait Q2MovementServices {
    /// Numeric operations for the selected profile.
    fn numeric(&self) -> NumericOps;
    /// Run a Q2 trace.
    fn trace(&mut self, query: Q2TraceQuery) -> Q2Trace;
    /// Q2 point contents as (stored, merged).
    fn point_contents(&mut self, query: Q2ContentsQuery) -> (i32, i32);
    /// Dispatch a touch contact.
    fn touch(&mut self, contact: Q2TouchContact, state: Q2State) -> MovementContinuation<Q2State>;
    /// Optional authoritative input application.
    fn input_application(&mut self) -> Option<&mut dyn Q2InputApplication> {
        None
    }
}

/// Q2 movement contact.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MovementContact {
    /// Contact target.
    pub target: TraceHit,
    /// Contact trace.
    pub trace: Q2Trace,
    /// Substep index.
    pub substep: usize,
}

/// Classic movement result.
pub type Q2MovementResult = MovementOutcome<Q2MovementState, Q2MovementContact>;

/// Rerelease presentation fields, mirroring donor
/// `Q2RereleaseMovementPresentation`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleasePresentation {
    /// Screen blend.
    pub screen_blend: qa_core::math::Vec4,
    /// Render flags.
    pub render_flags: i32,
    /// Jump sound flag.
    pub jump_sound: bool,
    /// Step-clip flag.
    pub step_clip: bool,
    /// Impact delta.
    pub impact_delta: f64,
}

/// Rerelease movement result.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2RereleaseMovementResult {
    /// Active result with presentation.
    Active {
        /// Result fields.
        fields: super::super::types::MovementResultFields<Q2MovementContact>,
        /// Final state.
        state: Q2RereleaseMovementState,
        /// Presentation.
        presentation: Q2RereleasePresentation,
    },
    /// The actor was removed mid-step.
    ActorRemoved {
        /// Moving actor.
        actor: ActorId,
        /// Command sequence number.
        command_sequence: i32,
        /// Ordered effects.
        effects: Vec<super::super::types::OrderedMovementEffect>,
    },
}

impl Q2RereleaseMovementResult {
    /// True when the actor was removed mid-step.
    #[must_use]
    pub fn removed(&self) -> bool {
        matches!(self, Q2RereleaseMovementResult::ActorRemoved { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;

    #[test]
    fn contents_and_masks_match_source() {
        assert_eq!(CONTENTS_SOLID, 1);
        assert_eq!(CONTENTS_WATER, 32);
        assert_eq!(MASK_SOLID, 3);
        assert_eq!(MASK_PLAYERSOLID, MASK_DEADSOLID | CONTENTS_MONSTER | CONTENTS_PLAYER);
        assert_eq!(MASK_WATER, CONTENTS_WATER | CONTENTS_LAVA | CONTENTS_SLIME);
        assert_eq!(MASK_CURRENT & CONTENTS_CURRENT_UP, CONTENTS_CURRENT_UP);
        assert_eq!(STEPSIZE, 18.0);
        assert_eq!(MAXTOUCH, 32);
        assert_eq!(PITCH, 0);
    }

    #[test]
    fn pmove_enumerations_match_source() {
        assert_eq!(pm_type::NORMAL, 0);
        assert_eq!(pm_type::FREEZE, 4);
        assert_eq!(kex_pm_type::GRAPPLE, 1);
        assert_eq!(kex_pm_type::FREEZE, 6);
        assert_eq!(pm_flags::DUCKED, 1);
        assert_eq!(pm_flags::TIME_TRICK, 1024);
        assert_eq!(button::CROUCH, 16);
        assert_eq!(water_level::UNDER, 3);
        assert_eq!(refdef_flags::UNDERWATER, 1);
    }

    #[test]
    fn zero_plane_and_surface_round_trip() {
        let zero = plane();
        assert_eq!(zero.normal, [0.0, 0.0, 0.0]);
        let surface = CSurface {
            name: "rock".to_string(),
            flags: 2,
            value: 0,
            material: "stone".to_string(),
        };
        let converted = Q2Surface::from(&surface);
        assert_eq!(CSurface::from(&converted), surface);
    }

    #[test]
    fn trace_reports_sweep_view() {
        use super::super::super::swept_body::SweptBodyTrace;
        let trace = TraceT {
            allsolid: false,
            startsolid: false,
            fraction: 0.5,
            endpos: [1.0, 2.0, 3.0],
            plane: plane(),
            surface: None,
            contents: 0,
            ent: None,
            plane2: plane(),
            surface2: None,
            native: None,
        };
        assert_eq!(trace.fraction(), 0.5);
        assert_eq!(trace.end(), vec3(1.0, 2.0, 3.0));
        assert!(!trace.all_solid());
    }

    #[test]
    fn stuck_results_cover_source() {
        assert_ne!(StuckResult::GoodPosition, StuckResult::NoGoodPosition);
        assert_eq!(PM_CONFIG_DEFAULT.airaccel, 0.0);
        assert!(!PM_CONFIG_DEFAULT.n64_physics);
    }
}
