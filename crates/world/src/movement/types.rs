//! Shared movement-local mirrors of donor contracts.
//!
//! Donor provenance: `src/contracts/movement.ts` (state, input, effect,
//! continuation shapes), `src/contracts/protocol.ts` (user-command shapes),
//! `src/contracts/scene.ts` (trace hit/contact/shape shapes),
//! `src/contracts/mod-client-outputs.ts` (client movement outputs),
//! `src/contracts/common.ts` (`CommandDialect`), `src/contracts/time.ts`
//! (re-exported from `qa_core::time`), `src/contracts/numeric.ts`
//! (re-exported from `qa_core::numeric`).
//!
//! These types keep the movement kernels headless: engine I/O (scene traces,
//! touch dispatch, weapon/animation owners, input application) arrives
//! through the per-family service traits, never through a concrete backend.
//! Arithmetic selection, vectors, identity, and clocks are reused from
//! `qa_core`; only movement-specific shapes live here.

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{Bounds, Plane, Vec3, Vec4};
use qa_core::time::{FrameContext, SourceTime};

/// Item handle (`namespace:name`), mirroring donor `gameplay.ts`.
pub type ItemId = String;

/// One ammo counter, mirroring donor `InventoryEntry`.
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryEntry {
    /// Item handle.
    pub item: ItemId,
    /// Current count.
    pub count: f64,
}

/// Command and movement dialect, mirroring donor `CommandDialect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementDialect {
    /// NetQuake.
    Q1Netquake,
    /// QuakeWorld.
    Q1Quakeworld,
    /// Quake II classic.
    Q2Classic,
    /// Quake II rerelease.
    Q2Rerelease,
    /// Quake III.
    Q3,
}

/// NetQuake user command, mirroring donor `Q1UserCommand`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1UserCommand {
    /// Acknowledged server time in seconds.
    pub acknowledged_server_time_seconds: f64,
    /// Requested view angles.
    pub view_angles: Vec3,
    /// Forward impulse.
    pub forward_move: f64,
    /// Side impulse.
    pub side_move: f64,
    /// Vertical impulse.
    pub up_move: f64,
    /// Button bitmask.
    pub buttons: i32,
    /// Impulse byte.
    pub impulse: i32,
}

/// QuakeWorld user command, mirroring donor `QwUserCommand`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QwUserCommand {
    /// Command length in milliseconds (source byte).
    pub milliseconds: i32,
    /// Requested angles.
    pub angles: Vec3,
    /// Forward impulse.
    pub forward_move: f64,
    /// Side impulse.
    pub side_move: f64,
    /// Vertical impulse.
    pub up_move: f64,
    /// Button bitmask.
    pub buttons: i32,
    /// Impulse byte.
    pub impulse: i32,
}

/// Quake II classic user command, mirroring donor `Q2UserCommand`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2UserCommand {
    /// Command length in milliseconds (source byte).
    pub milliseconds: i32,
    /// Short-encoded view angles.
    pub angle_shorts: [i32; 3],
    /// Forward impulse.
    pub forward_move: f64,
    /// Side impulse.
    pub side_move: f64,
    /// Vertical impulse.
    pub up_move: f64,
    /// Button bitmask.
    pub buttons: i32,
    /// Impulse byte.
    pub impulse: i32,
    /// Light level byte.
    pub light_level: i32,
}

/// Quake II rerelease user command, mirroring donor `Q2RereleaseUserCommand`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseUserCommand {
    /// Command length in milliseconds (source byte).
    pub milliseconds: i32,
    /// View angles.
    pub angles: Vec3,
    /// Forward impulse.
    pub forward_move: f64,
    /// Side impulse.
    pub side_move: f64,
    /// Button bitmask.
    pub buttons: i32,
    /// Server frame number.
    pub server_frame: i32,
}

/// Quake III user command, mirroring donor `Q3UserCommand`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3UserCommand {
    /// Server time in milliseconds.
    pub server_time_milliseconds: i32,
    /// Word-encoded view angles.
    pub angle_words: [i32; 3],
    /// Button bitmask.
    pub buttons: i32,
    /// Requested weapon.
    pub weapon: i32,
    /// Forward impulse (signed byte).
    pub forward_move: i32,
    /// Right impulse (signed byte).
    pub right_move: i32,
    /// Vertical impulse (signed byte).
    pub up_move: i32,
}

/// Any source user command, mirroring donor `UserCommand`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UserCommand {
    /// NetQuake command.
    Q1Netquake(Q1UserCommand),
    /// QuakeWorld command.
    Q1Quakeworld(QwUserCommand),
    /// Quake II classic command.
    Q2Classic(Q2UserCommand),
    /// Quake II rerelease command.
    Q2Rerelease(Q2RereleaseUserCommand),
    /// Quake III command.
    Q3(Q3UserCommand),
}

impl UserCommand {
    /// Dialect of this command.
    #[must_use]
    pub fn dialect(&self) -> MovementDialect {
        match self {
            UserCommand::Q1Netquake(_) => MovementDialect::Q1Netquake,
            UserCommand::Q1Quakeworld(_) => MovementDialect::Q1Quakeworld,
            UserCommand::Q2Classic(_) => MovementDialect::Q2Classic,
            UserCommand::Q2Rerelease(_) => MovementDialect::Q2Rerelease,
            UserCommand::Q3(_) => MovementDialect::Q3,
        }
    }

    /// Button bitmask carried by the command (stock `button0/1/2`).
    #[must_use]
    pub fn buttons(&self) -> i32 {
        match self {
            UserCommand::Q1Netquake(command) => command.buttons,
            UserCommand::Q1Quakeworld(command) => command.buttons,
            UserCommand::Q2Classic(command) => command.buttons,
            UserCommand::Q2Rerelease(command) => command.buttons,
            UserCommand::Q3(command) => command.buttons,
        }
    }
}

/// Trace shape, mirroring donor `TraceShape`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceShape {
    /// Point trace.
    Point,
    /// Axis-aligned box trace.
    Box(Bounds),
    /// Capsule trace.
    Capsule(Bounds),
}

impl TraceShape {
    /// Bounds carried by the shape, if any.
    #[must_use]
    pub fn bounds(&self) -> Option<Bounds> {
        match *self {
            TraceShape::Point => None,
            TraceShape::Box(bounds) | TraceShape::Capsule(bounds) => Some(bounds),
        }
    }
}

/// Trace hit target, mirroring donor `TraceHit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceHit {
    /// No hit.
    None,
    /// World geometry hit.
    World {
        /// Model index.
        model: u32,
    },
    /// Actor hit.
    Actor {
        /// Hit actor.
        actor: ActorId,
    },
}

/// Trace contact surface, mirroring donor `TraceContact`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceContact {
    /// No contact plane.
    None,
    /// Contact plane.
    Plane(Plane),
}

/// Semantic client movement mode, mirroring donor `ModClientMovementMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModClientMovementMode {
    /// Normal movement.
    Normal,
    /// Noclip movement.
    Noclip,
    /// Frozen.
    Freeze,
}

/// Semantic client movement outputs, mirroring donor
/// `ModClientMovementOutputs`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ModClientMovementOutputs {
    /// Requested view offset.
    pub view_offset: Option<Vec3>,
    /// Requested movement mode.
    pub mode: Option<ModClientMovementMode>,
    /// Requested crouch stance.
    pub stance: Option<bool>,
    /// Requested body bounds.
    pub body_bounds: Option<Bounds>,
}

/// Fixed locomotion pose, mirroring donor `FixedMovementPose`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixedMovementPose {
    /// Crouched flag.
    pub crouched: bool,
    /// Body bounds.
    pub bounds: Bounds,
    /// View height.
    pub view_height: f64,
}

/// Per-frame movement environment, mirroring donor `MovementEnvironment`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementEnvironment {
    /// Client outputs, when a mod client drives them.
    pub client_outputs: Option<ModClientMovementOutputs>,
    /// Equipment speed multiplier.
    pub speed_multiplier: Option<f64>,
    /// Fixed pose override.
    pub pose: Option<FixedMovementPose>,
    /// Current health.
    pub health: f64,
    /// Flight granted.
    pub flight: bool,
    /// Haste granted.
    pub haste: bool,
    /// Invulnerability granted.
    pub invulnerable: bool,
    /// Gravity multiplier.
    pub gravity_multiplier: f64,
}

impl Default for MovementEnvironment {
    fn default() -> Self {
        Self {
            client_outputs: None,
            speed_multiplier: None,
            pose: None,
            health: 100.0,
            flight: false,
            haste: false,
            invulnerable: false,
            gravity_multiplier: 1.0,
        }
    }
}

/// Predictable movement event, mirroring donor `PredictableMovementEvent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredictableMovementEvent {
    /// Emitting provider.
    pub provider: ProviderId,
    /// Event sequence number.
    pub sequence: i32,
    /// Source event number.
    pub event: i32,
    /// Event parameter.
    pub parameter: i32,
}

/// Weapon state by family, mirroring donor `WeaponState`.
#[derive(Debug, Clone, PartialEq)]
pub enum WeaponState {
    /// Quake I weapon frame state.
    Q1 {
        /// Current frame.
        frame: i32,
        /// Attack-finished time in seconds.
        attack_finished_seconds: f64,
        /// Source weapon number.
        source_weapon: i32,
    },
    /// Quake II weapon frame state.
    Q2 {
        /// Gun frame.
        gun_frame: i32,
        /// Weapon state word.
        state: i32,
        /// Pending weapon.
        pending_weapon: Option<ItemId>,
        /// Machinegun shot count.
        machinegun_shots: i32,
        /// Grenade timer.
        grenade_time: SourceTime,
        /// Grenade blew up flag.
        grenade_blew_up: bool,
    },
    /// Quake III weapon state.
    Q3 {
        /// Source weapon number.
        source_weapon: i32,
        /// Weapon state word.
        state: i32,
        /// Weapon timer in milliseconds.
        time_milliseconds: i32,
    },
}

/// Arsenal snapshot, mirroring donor `ArsenalState`.
#[derive(Debug, Clone, PartialEq)]
pub struct ArsenalState {
    /// Owning provider.
    pub provider: ProviderId,
    /// Active weapon.
    pub active_weapon: Option<ItemId>,
    /// Weapon state.
    pub state: WeaponState,
    /// Ammo counters.
    pub ammo: Vec<InventoryEntry>,
}

/// Animation state by family, mirroring donor `AnimationState`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnimationState {
    /// Quake I animation frame state.
    Q1 {
        /// Current frame.
        frame: i32,
        /// Next frame time in seconds.
        next_frame_seconds: f64,
    },
    /// Quake II animation frame state.
    Q2 {
        /// Current frame.
        frame: i32,
        /// End frame.
        end_frame: i32,
        /// Animation priority.
        priority: i32,
        /// Duck flag.
        duck: bool,
        /// Run flag.
        run: bool,
    },
    /// Quake III animation state.
    Q3 {
        /// Legs animation word.
        legs: i32,
        /// Torso animation word.
        torso: i32,
        /// Legs timer in milliseconds.
        legs_timer_milliseconds: i32,
        /// Torso timer in milliseconds.
        torso_timer_milliseconds: i32,
    },
}

/// Animation snapshot, mirroring donor `ActorAnimationState`.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorAnimationState {
    /// Owning provider.
    pub provider: ProviderId,
    /// Animation state.
    pub state: AnimationState,
}

/// Locomotion selector for the animation owner, mirroring donor
/// `LocomotionAnimation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocomotionAnimation {
    /// Standing still.
    Idle,
    /// Walking.
    Walk,
    /// Running.
    Run,
    /// Moving backwards.
    Backward,
    /// Crouched.
    Crouch,
    /// Airborne.
    Jump,
    /// Landing.
    Land,
    /// Swimming.
    Swim,
}

/// One movement effect, mirroring donor `MovementEffect`.
#[derive(Debug, Clone, PartialEq)]
pub enum MovementEffect {
    /// Predictable event emission.
    Event(PredictableMovementEvent),
    /// Weapon state transition.
    Weapon {
        /// Owning provider.
        provider: ProviderId,
        /// State before the step.
        before: WeaponState,
        /// State after the step.
        after: WeaponState,
    },
    /// Weapon selection transition.
    WeaponSelection {
        /// Owning provider.
        provider: ProviderId,
        /// Selection before the step.
        before: Option<ItemId>,
        /// Selection after the step.
        after: Option<ItemId>,
    },
    /// Ammo counter transition.
    Ammo {
        /// Item handle.
        item: ItemId,
        /// Count before the step.
        before: f64,
        /// Count after the step.
        after: f64,
    },
    /// Animation transition.
    Animation {
        /// Owning provider.
        provider: ProviderId,
        /// State before the step.
        before: AnimationState,
        /// State after the step.
        after: AnimationState,
    },
    /// Touch contact.
    Touch {
        /// Touch target (never none).
        target: TraceHit,
        /// Substep index.
        substep: usize,
    },
}

/// Movement effect with source ordering, mirroring donor
/// `OrderedMovementEffect`.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderedMovementEffect {
    /// Substep index.
    pub substep: usize,
    /// Sequence within the command.
    pub sequence: usize,
    /// Effect time.
    pub time: SourceTime,
    /// The effect.
    pub effect: MovementEffect,
}

/// How a movement step executes, mirroring donor `execution`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementExecution {
    /// Server-authoritative step.
    Authoritative,
    /// Client prediction step.
    Prediction,
}

/// Continuation after a synchronous game callback, mirroring donor
/// `MovementContinuation`.
#[derive(Debug, Clone, PartialEq)]
pub enum MovementContinuation<S> {
    /// Continue with updated state.
    Continue(S),
    /// The actor was removed.
    ActorRemoved,
}

/// Continuation after input application begins, mirroring donor
/// `MovementInputContinuation`.
#[derive(Debug, Clone, PartialEq)]
pub enum MovementInputContinuation<S> {
    /// Continue with updated state and command.
    Continue {
        /// Updated state.
        state: S,
        /// Updated command.
        command: UserCommand,
    },
    /// The actor was removed.
    ActorRemoved,
}

/// Touch contact passed to the touch owner, mirroring donor
/// `MovementTouchContact` (Q2 rerelease source-trace detail is carried by
/// the family trace instead).
#[derive(Debug, Clone, PartialEq)]
pub struct MovementTouchContact {
    /// Moving actor.
    pub mover: OwnedActor,
    /// Touch target (never none).
    pub other: TraceHit,
    /// Contact plane, when the trace struck one.
    pub plane: Option<Plane>,
    /// Native surface detail.
    pub surface: Option<TouchSurface>,
}

/// Native surface detail for a touch contact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchSurface {
    /// Surface name.
    pub name: String,
    /// Native surface flags.
    pub native_flags: i32,
    /// Native surface value.
    pub native_value: i32,
}

/// Screen-blend presentation is a four-component vector.
pub type ScreenBlend = Vec4;

/// Shared movement-input fields, mirroring donor `MovementInputFields`.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementInputFields {
    /// Moving actor.
    pub actor: OwnedActor,
    /// Command sequence number.
    pub command_sequence: i32,
    /// Frame clock.
    pub frame: FrameContext,
    /// Collision shape from the selected character body.
    pub shape: TraceShape,
    /// Last accepted local hull.
    pub current_bounds: Option<Bounds>,
    /// Movement environment.
    pub environment: MovementEnvironment,
    /// Arsenal snapshot.
    pub arsenal: ArsenalState,
    /// Animation snapshot.
    pub animation: ActorAnimationState,
    /// Execution mode.
    pub execution: MovementExecution,
}

/// Shared movement-result fields, mirroring donor `MovementResultFields`.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementResultFields<T> {
    /// Moving actor.
    pub actor: ActorId,
    /// Command sequence number.
    pub command_sequence: i32,
    /// Accepted body bounds.
    pub bounds: Bounds,
    /// Final view angles.
    pub view_angles: Vec3,
    /// Final view height.
    pub view_height: f64,
    /// Final ground hit.
    pub ground: TraceHit,
    /// Final water level.
    pub water_level: i32,
    /// Final water type.
    pub water_type: i32,
    /// Horizontal speed.
    pub horizontal_speed: f64,
    /// Contacts recorded during the step.
    pub contacts: Vec<T>,
    /// Ordered effects.
    pub effects: Vec<OrderedMovementEffect>,
    /// Arsenal snapshot.
    pub arsenal: ArsenalState,
    /// Animation snapshot.
    pub animation: ActorAnimationState,
}

/// Movement outcome: active state or synchronous removal, mirroring donor
/// `MovementOutcome`.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum MovementOutcome<State, Contact> {
    /// Active result with state to write back.
    Active {
        /// Result fields.
        fields: MovementResultFields<Contact>,
        /// Final state.
        state: State,
    },
    /// The actor was removed mid-step.
    ActorRemoved {
        /// Moving actor.
        actor: ActorId,
        /// Command sequence number.
        command_sequence: i32,
        /// Ordered effects.
        effects: Vec<OrderedMovementEffect>,
    },
}

impl<State, Contact> MovementOutcome<State, Contact> {
    /// True when the actor was removed mid-step.
    #[must_use]
    pub fn removed(&self) -> bool {
        matches!(self, MovementOutcome::ActorRemoved { .. })
    }
}

/// Movement contract violation (donor `Error`) or out-of-range source value
/// (donor `RangeError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MovementError {
    /// Contract violation with a static explanation.
    Contract(&'static str),
    /// Out-of-range source value with a static explanation.
    Range(&'static str),
}

impl std::fmt::Display for MovementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MovementError::Contract(message) | MovementError::Range(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for MovementError {}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;

    #[test]
    fn command_dialects_cover_every_family() {
        let q1 = UserCommand::Q1Netquake(Q1UserCommand {
            acknowledged_server_time_seconds: 0.0,
            view_angles: vec3(0.0, 0.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        });
        assert_eq!(q1.dialect(), MovementDialect::Q1Netquake);
        let qw = UserCommand::Q1Quakeworld(QwUserCommand {
            milliseconds: 10,
            angles: vec3(0.0, 0.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        });
        assert_eq!(qw.dialect(), MovementDialect::Q1Quakeworld);
        let q3 = UserCommand::Q3(Q3UserCommand {
            server_time_milliseconds: 100,
            angle_words: [0, 0, 0],
            buttons: 0,
            weapon: 1,
            forward_move: 0,
            right_move: 0,
            up_move: 0,
        });
        assert_eq!(q3.dialect(), MovementDialect::Q3);
    }

    #[test]
    fn trace_shapes_report_bounds_only_for_volumes() {
        let bounds = Bounds {
            min: vec3(-16.0, -16.0, -24.0),
            max: vec3(16.0, 16.0, 32.0),
        };
        assert_eq!(TraceShape::Point.bounds(), None);
        assert_eq!(TraceShape::Box(bounds).bounds(), Some(bounds));
        assert_eq!(TraceShape::Capsule(bounds).bounds(), Some(bounds));
    }

    #[test]
    fn default_environment_matches_healthy_grounded_actor() {
        let environment = MovementEnvironment::default();
        assert_eq!(environment.health, 100.0);
        assert!(!environment.flight);
        assert_eq!(environment.gravity_multiplier, 1.0);
        assert!(environment.pose.is_none());
    }

    #[test]
    fn outcome_reports_removal() {
        let removed: MovementOutcome<u8, u8> = MovementOutcome::ActorRemoved {
            actor: qa_core::identity::IdentityOwner::create("t").unwrap().actor(1, 0),
            command_sequence: 3,
            effects: Vec::new(),
        };
        assert!(removed.removed());
    }
}
