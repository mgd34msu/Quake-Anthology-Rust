//! Quake III motion work state, options, and service shapes.
//!
//! Donor provenance: `src/movement/q3/types.ts`, plus Q3 state/input/result
//! mirrors from `src/contracts/movement.ts` and the `kind: "q3"` trace shape
//! from `src/contracts/scene.ts`.

use crate::hull::BspPlane;
use qa_core::identity::{ActorId, ProviderId};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::{NumericOps, NumericProfile};
use qa_core::time::{ClockProfile, FrameContext};

use super::super::types::{
    ActorAnimationState, AnimationState, ArsenalState, FixedMovementPose, MovementContinuation,
    MovementEffect, MovementInputContinuation, MovementInputFields, MovementOutcome, Q3UserCommand,
    TraceContact, TraceHit, UserCommand, WeaponState,
};

/// Q3 product selecting mission-pack behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3Product {
    /// Base game.
    BaseQ3,
    /// Mission pack.
    MissionPack,
}

/// Locomotion command decoded from a user command, mirroring donor
/// `Q3Command`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3Command {
    /// Server time in milliseconds.
    pub server_time: i32,
    /// Word-encoded angles.
    pub angles: Vec3,
    /// Button bitmask.
    pub buttons: i32,
    /// Requested weapon.
    pub weapon: i32,
    /// Forward impulse.
    pub forwardmove: i32,
    /// Right impulse.
    pub rightmove: i32,
    /// Vertical impulse.
    pub upmove: i32,
}

/// Mutable locomotion work state; only locomotion-owned words from
/// `playerState_t` live here. Mirrors donor `Q3Motion`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3Motion {
    /// Command time in milliseconds.
    pub command_time: i32,
    /// Movement type word.
    pub pm_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Movement flags.
    pub pm_flags: i32,
    /// Movement timer in milliseconds.
    pub pm_time: i32,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Gravity.
    pub gravity: f32,
    /// Speed.
    pub speed: f32,
    /// Delta angles.
    pub delta_angles: Vec3,
    /// Ground hit.
    pub ground: TraceHit,
    /// Movement direction.
    pub movement_dir: i32,
    /// Grapple point.
    pub grapple_point: Vec3,
    /// Entity flags.
    pub e_flags: i32,
    /// View angles.
    pub viewangles: Vec3,
    /// View height.
    pub viewheight: f64,
    /// Pmove frame count.
    pub pmove_framecount: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Moving actor.
    pub actor: ActorId,
    /// Health.
    pub health: f64,
    /// Flight granted.
    pub flight: bool,
    /// Invulnerability granted.
    pub invulnerable: bool,
    /// Product selector.
    pub product: Q3Product,
}

/// Animation request at a locomotion call site, mirroring donor
/// `Q3AnimationRequest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3AnimationRequest {
    /// Set legs animation.
    Legs {
        /// Animation word.
        animation: i32,
        /// Force flag.
        force: bool,
    },
    /// Set legs timer.
    LegsTimer {
        /// Milliseconds.
        milliseconds: i32,
    },
    /// Drop timers.
    DropTimers,
    /// Gesture.
    Gesture,
}

/// Hook context at a locomotion call site, mirroring donor `Q3HookContext`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3HookContext {
    /// Movement input.
    pub input: Box<Q3MovementInput>,
    /// Motion snapshot.
    pub motion: Q3Motion,
    /// Movement state.
    pub state: Q3MovementState,
    /// Active command.
    pub command: Q3Command,
    /// Frame clock.
    pub frame: FrameContext,
    /// Arsenal snapshot.
    pub arsenal: ArsenalState,
    /// Animation snapshot.
    pub animation: ActorAnimationState,
}

/// Animation-step result from the animation owner.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3AnimationStepResult {
    /// Updated animation.
    pub animation: ActorAnimationState,
    /// Emitted effects.
    pub effects: Vec<MovementEffect>,
}

/// Weapon-phase result from the weapon owner, mirroring donor
/// `Q3WeaponPhaseResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3WeaponPhaseResult {
    /// Updated arsenal.
    pub arsenal: ArsenalState,
    /// Updated animation.
    pub animation: ActorAnimationState,
    /// Emitted effects.
    pub effects: Vec<MovementEffect>,
    /// Optional continuation.
    pub continuation: Option<MovementContinuation<Q3MovementState>>,
    /// Movement flags selected by the weapon step.
    pub movement_flags: i32,
}

/// Selected arsenal and character adapters running at the original `PM_*`
/// call sites. Mirrors donor `Q3MovementHooks`.
pub trait Q3MovementHooks {
    /// Whether the weapon is firing.
    fn firing(&mut self, context: &Q3HookContext) -> bool;
    /// Run an animation request.
    fn animation(
        &mut self,
        request: Q3AnimationRequest,
        context: &Q3HookContext,
    ) -> Q3AnimationStepResult;
    /// Run the weapon phase.
    fn weapon(&mut self, context: &Q3HookContext) -> Q3WeaponPhaseResult;
    /// Run the torso phase.
    fn torso(&mut self, context: &Q3HookContext) -> Q3AnimationStepResult;
}

/// Trace callback for locomotion, mirroring donor
/// `Q3MovementTraceFunction`.
pub type Q3MovementTraceFn<'a> =
    dyn FnMut(Vec3, Vec3, Bounds, ActorId, i32) -> Q3Trace + 'a;

/// Point-contents callback for locomotion.
pub type Q3PointContentsFn<'a> = dyn FnMut(Vec3, ActorId) -> i32 + 'a;

/// Per-step locomotion callbacks at the original `PM_*` call sites.
///
/// Callbacks that read or write locomotion state (events bump the event
/// sequence, the weapon phase resumes hook state) receive the live motion;
/// touch and diagnostics observers do not need it.
pub trait Q3MotionDriver {
    /// Begin a substep; false stops the command.
    fn begin_step(
        &mut self,
        motion: &mut Q3Motion,
        command: &mut Q3Command,
        msec: i32,
        substep: usize,
    ) -> bool;
    /// End a substep; false stops the command.
    fn end_step(&mut self, _motion: &mut Q3Motion) -> bool {
        true
    }
    /// Emit a locomotion event.
    fn event(&mut self, event: i32, motion: &mut Q3Motion);
    /// Run an animation request.
    fn animation(&mut self, request: Q3AnimationRequest, motion: &mut Q3Motion);
    /// Run the weapon phase; false stops the command.
    fn weapon(&mut self, motion: &mut Q3Motion) -> bool;
    /// Run the torso phase.
    fn torso(&mut self, motion: &mut Q3Motion);
    /// Whether the weapon is firing.
    fn firing(&mut self, motion: &mut Q3Motion) -> bool;
    /// Observe a touch contact.
    fn contact(&mut self, trace: &Q3Trace);
    /// Diagnostics output.
    fn debug(&mut self, _message: &str) {}
}

/// Locomotion options, mirroring donor `Q3MotionOptions`.
pub struct Q3MotionOptions<'a> {
    /// Fixed pose override.
    pub pose: Option<FixedMovementPose>,
    /// Trace callback.
    pub trace: Box<Q3MovementTraceFn<'a>>,
    /// Point-contents callback.
    pub point_contents: Box<Q3PointContentsFn<'a>>,
    /// Standing bounds.
    pub standing_bounds: Bounds,
    /// Current bounds.
    pub current_bounds: Option<Bounds>,
    /// Requested client body bounds.
    pub body_bounds: Option<Bounds>,
    /// Postures.
    pub postures: Q3Postures,
    /// Trace mask.
    pub trace_mask: i32,
    /// Fixed step in milliseconds (null selects 66).
    pub fixed_msec: Option<i32>,
    /// Suppress footsteps.
    pub no_footsteps: bool,
    /// Per-step callbacks.
    pub driver: &'a mut dyn Q3MotionDriver,
}

/// Locomotion result, mirroring donor `Q3MotionResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3MotionResult {
    /// Touched actor traces.
    pub contacts: Vec<Q3Trace>,
    /// Accepted bounds.
    pub bounds: Bounds,
    /// Water level.
    pub waterlevel: i32,
    /// Water type.
    pub watertype: i32,
    /// Horizontal speed.
    pub xyspeed: f32,
}

/// Posture dimensions, mirroring donor `Q3Postures`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3Postures {
    /// Standing view height.
    pub standing_view_height: f64,
    /// Crouched posture.
    pub crouched: Q3Posture,
    /// Dead posture.
    pub dead: Q3Posture,
    /// Invulnerability-expanded bounds.
    pub invulnerability_expanded: Bounds,
}

/// One posture: bounds plus view height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3Posture {
    /// Body bounds.
    pub bounds: Bounds,
    /// View height.
    pub view_height: f64,
}

/// Quake III movement state, mirroring donor `Q3MovementState`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3MovementState {
    /// Command time in milliseconds.
    pub command_time_milliseconds: i32,
    /// Movement type word.
    pub movement_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Movement flags.
    pub movement_flags: i32,
    /// Movement timer in milliseconds.
    pub movement_time_milliseconds: i32,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Gravity.
    pub gravity: f64,
    /// Speed.
    pub speed: f64,
    /// Delta angle words.
    pub delta_angle_words: [i32; 3],
    /// Movement direction.
    pub movement_direction: i32,
    /// Grapple point.
    pub grapple_point: Vec3,
    /// Entity flags.
    pub flags: i32,
    /// View angles.
    pub view_angles: Vec3,
    /// View height.
    pub view_height: f64,
    /// Ground hit.
    pub ground: TraceHit,
    /// Predictable event sequence.
    pub predictable_event_sequence: i32,
    /// Active jump pad.
    pub jump_pad: Option<ActorId>,
    /// Movement frame.
    pub movement_frame: i32,
    /// Jump-pad frame.
    pub jump_pad_frame: i32,
}

/// Quake III movement profile, mirroring donor `Q3MovementProfile`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3MovementProfile {
    /// Provider identity.
    pub id: ProviderId,
    /// Frame clock.
    pub clock: ClockProfile,
    /// Numeric profile (must select binary32 arithmetic).
    pub numeric: NumericProfile,
    /// Product selector.
    pub product: Q3Product,
    /// Fixed step in milliseconds.
    pub fixed_milliseconds: Option<i32>,
    /// Suppress footsteps.
    pub no_footsteps: bool,
}

/// Quake III movement input, mirroring donor `Q3MovementInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3MovementInput {
    /// Shared input fields.
    pub fields: MovementInputFields,
    /// Source command.
    pub command: Q3UserCommand,
    /// Current state.
    pub state: Q3MovementState,
    /// Movement profile.
    pub profile: Q3MovementProfile,
}

/// Q3 scene trace, mirroring donor `TraceResult` with `kind: "q3"`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3Trace {
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
    /// Surface flags.
    pub surface_flags: i32,
    /// Stored source plane.
    pub source_plane: BspPlane,
}

impl super::super::swept_body::SweptBodyTrace for Q3Trace {
    fn fraction(&self) -> f64 {
        self.fraction
    }
    fn end(&self) -> Vec3 {
        self.end
    }
    fn all_solid(&self) -> bool {
        self.all_solid
    }
    fn start_solid(&self) -> bool {
        self.start_solid
    }
}

/// Q3 trace query for movement services.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3TraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Trace bounds.
    pub bounds: Bounds,
    /// Point trace flag.
    pub point: bool,
    /// Passing actor.
    pub pass_actor: ActorId,
    /// Contents mask.
    pub mask: i32,
    /// BSP curves enabled.
    pub curves: bool,
    /// Player curve clipping enabled.
    pub player_curve_clip: bool,
}

/// Input application boundary for authoritative Q3 steps.
pub trait Q3InputApplication {
    /// Begin input application for a command.
    fn begin(
        &mut self,
        command: UserCommand,
        frame: &FrameContext,
        state: Q3MovementState,
    ) -> MovementInputContinuation<Q3MovementState>;
    /// End input application.
    fn end(
        &mut self,
        state: Q3MovementState,
        failed: bool,
    ) -> MovementContinuation<Q3MovementState>;
}

/// Movement services for Quake III steps.
pub trait Q3MovementServices {
    /// Numeric operations for the selected profile.
    fn numeric(&self) -> NumericOps;
    /// Run a Q3 trace.
    fn trace(&mut self, query: Q3TraceQuery) -> Q3Trace;
    /// Q3 point contents.
    fn point_contents(&mut self, point: Vec3, pass_actor: &ActorId) -> i32;
    /// Optional authoritative input application.
    fn input_application(&mut self) -> Option<&mut dyn Q3InputApplication> {
        None
    }
}

/// Q3 movement contact.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3MovementContact {
    /// Contact target.
    pub target: TraceHit,
    /// Contact trace.
    pub trace: Q3Trace,
    /// Substep index.
    pub substep: usize,
}

/// Quake III movement result.
pub type Q3MovementResult = MovementOutcome<Q3MovementState, Q3MovementContact>;

/// Q3 animation state view for the animation owner.
#[must_use]
pub fn q3_animation_state(animation: &ActorAnimationState) -> Option<(i32, i32, i32, i32)> {
    match animation.state {
        AnimationState::Q3 {
            legs,
            torso,
            legs_timer_milliseconds,
            torso_timer_milliseconds,
        } => Some((legs, torso, legs_timer_milliseconds, torso_timer_milliseconds)),
        _ => None,
    }
}

/// Q3 weapon state view for the weapon owner.
#[must_use]
pub fn q3_weapon_state(arsenal: &ArsenalState) -> Option<(i32, i32, i32)> {
    match &arsenal.state {
        WeaponState::Q3 {
            source_weapon,
            state,
            time_milliseconds,
        } => Some((*source_weapon, *state, *time_milliseconds)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;

    #[test]
    fn postures_carry_view_heights() {
        let postures = Q3Postures {
            standing_view_height: 26.0,
            crouched: Q3Posture {
                bounds: Bounds {
                    min: vec3(-15.0, -15.0, -24.0),
                    max: vec3(15.0, 15.0, 16.0),
                },
                view_height: 12.0,
            },
            dead: Q3Posture {
                bounds: Bounds {
                    min: vec3(-15.0, -15.0, -24.0),
                    max: vec3(15.0, 15.0, -8.0),
                },
                view_height: -16.0,
            },
            invulnerability_expanded: Bounds {
                min: vec3(-42.0, -42.0, -42.0),
                max: vec3(42.0, 42.0, 42.0),
            },
        };
        assert_eq!(postures.standing_view_height, 26.0);
        assert_eq!(postures.dead.view_height, -16.0);
    }

    #[test]
    fn animation_requests_cover_locomotion_sites() {
        assert_ne!(
            Q3AnimationRequest::Legs {
                animation: 15,
                force: true
            },
            Q3AnimationRequest::Gesture
        );
        assert_eq!(
            Q3AnimationRequest::LegsTimer { milliseconds: 130 },
            Q3AnimationRequest::LegsTimer { milliseconds: 130 }
        );
        assert_eq!(
            Q3AnimationRequest::DropTimers,
            Q3AnimationRequest::DropTimers
        );
    }

    #[test]
    fn trace_reports_sweep_view() {
        use super::super::super::swept_body::SweptBodyTrace;
        let trace = Q3Trace {
            fraction: 0.5,
            end: vec3(1.0, 2.0, 3.0),
            start_solid: false,
            all_solid: false,
            contact: TraceContact::None,
            hit: TraceHit::None,
            contents: 0,
            surface_flags: 0,
            source_plane: BspPlane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
                plane_type: 0,
                signbits: 0,
            },
        };
        assert_eq!(trace.fraction(), 0.5);
        assert_eq!(trace.end(), vec3(1.0, 2.0, 3.0));
    }

    #[test]
    fn state_views_reject_foreign_families() {
        let animation = ActorAnimationState {
            provider: ProviderId::new("q3", "test"),
            state: AnimationState::Q1 {
                frame: 0,
                next_frame_seconds: 0.0,
            },
        };
        assert_eq!(q3_animation_state(&animation), None);
        let arsenal = ArsenalState {
            provider: ProviderId::new("q3", "test"),
            active_weapon: None,
            state: WeaponState::Q1 {
                frame: 0,
                attack_finished_seconds: 0.0,
                source_weapon: 0,
            },
            ammo: Vec::new(),
        };
        assert_eq!(q3_weapon_state(&arsenal), None);
    }
}
