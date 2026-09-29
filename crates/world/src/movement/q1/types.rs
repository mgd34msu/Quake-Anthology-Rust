//! Quake I movement state, input, option, and service shapes.
//!
//! Donor provenance: `src/movement/q1/types.ts`.

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{Bounds, Plane, Vec3};
use qa_core::numeric::NumericProfile;
use qa_core::time::{ClockProfile, FrameContext};

use super::super::Q1MovementParameters;
use super::super::types::{
    ActorAnimationState, ArsenalState, FixedMovementPose, LocomotionAnimation, MovementContinuation,
    MovementEffect, MovementEnvironment, MovementError, MovementInputContinuation,
    MovementInputFields, MovementOutcome, MovementTouchContact, Q1UserCommand, QwUserCommand,
    TraceContact, TraceHit, TraceShape, UserCommand,
};

/// No movement.
pub const Q1_MOVE_NONE: i32 = 0;
/// Player walk.
pub const Q1_MOVE_WALK: i32 = 3;
/// Step movement.
pub const Q1_MOVE_STEP: i32 = 4;
/// Fly movement.
pub const Q1_MOVE_FLY: i32 = 5;
/// Toss movement.
pub const Q1_MOVE_TOSS: i32 = 6;
/// Pusher movement.
pub const Q1_MOVE_PUSH: i32 = 7;
/// Noclip movement.
pub const Q1_MOVE_NOCLIP: i32 = 8;
/// Fly missile.
pub const Q1_MOVE_FLYMISSILE: i32 = 9;
/// Bounce movement.
pub const Q1_MOVE_BOUNCE: i32 = 10;
/// Gib movement.
pub const Q1_MOVE_GIB: i32 = 11;
/// Flying flag.
pub const Q1_FLAG_FLY: i32 = 1;
/// Swimming flag.
pub const Q1_FLAG_SWIM: i32 = 2;
/// On-ground flag.
pub const Q1_FLAG_ONGROUND: i32 = 512;
/// Water-jump flag.
pub const Q1_FLAG_WATERJUMP: i32 = 2048;
/// Jump-released flag.
pub const Q1_FLAG_JUMPRELEASED: i32 = 4096;
/// Empty contents.
pub const Q1_CONTENTS_EMPTY: i32 = -1;
/// Solid contents.
pub const Q1_CONTENTS_SOLID: i32 = -2;
/// Water contents.
pub const Q1_CONTENTS_WATER: i32 = -3;
/// Slime contents.
pub const Q1_CONTENTS_SLIME: i32 = -4;
/// Lava contents.
pub const Q1_CONTENTS_LAVA: i32 = -5;
/// Step height.
pub const Q1_STEP_HEIGHT: f64 = 18.0;

/// Quake edition selecting NetQuake behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1Edition {
    /// Original Quake.
    Classic,
    /// Rerelease (gib bounce).
    Rerelease,
    /// Quake 64 (needs its own qualified profile).
    Quake64,
}

/// NetQuake movement state, mirroring donor `Q1MovementState`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1MovementState {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Last known unstuck origin.
    pub old_origin: Vec3,
    /// Angular velocity.
    pub angular_velocity: Vec3,
    /// View angles.
    pub view_angles: Vec3,
    /// Punch angles.
    pub punch_angles: Vec3,
    /// Movement type word.
    pub move_type: i32,
    /// Entity flags.
    pub flags: i32,
    /// Ground hit.
    pub ground: TraceHit,
    /// Water level 0-3.
    pub water_level: i32,
    /// Water contents type.
    pub water_type: i32,
    /// Teleport/water-jump expiry in seconds.
    pub teleport_time_seconds: f64,
    /// Water-jump push direction.
    pub water_jump_direction: Vec3,
    /// Ideal pitch for slopes.
    pub ideal_pitch: f64,
    /// Fix-angle flag.
    pub fix_angle: bool,
    /// Health.
    pub health: f64,
}

/// QuakeWorld movement state, mirroring donor `QwMovementState`.
#[derive(Debug, Clone, PartialEq)]
pub struct QwMovementState {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Previous button word.
    pub old_buttons: i32,
    /// Water-jump time remaining in seconds.
    pub water_jump_time_seconds: f64,
    /// Dead flag.
    pub dead: bool,
    /// Spectator word.
    pub spectator: i32,
    /// Ground hit.
    pub ground: TraceHit,
}

/// Either Quake I movement state.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1State {
    /// NetQuake state.
    Netquake(Q1MovementState),
    /// QuakeWorld state.
    Quakeworld(QwMovementState),
}

/// NetQuake movement profile, mirroring donor `Q1MovementProfile`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1MovementProfile {
    /// Provider identity.
    pub id: ProviderId,
    /// Frame clock (must be NetQuake).
    pub clock: ClockProfile,
    /// Numeric profile.
    pub numeric: NumericProfile,
    /// Quake edition.
    pub edition: Q1Edition,
    /// Movement tuning.
    pub parameters: Q1MovementParameters,
    /// Edge friction multiplier.
    pub edge_friction: f64,
    /// Alternate noclip angle handling.
    pub no_clip_angle_hack: bool,
}

/// QuakeWorld movement profile, mirroring donor `QwMovementProfile`.
#[derive(Debug, Clone, PartialEq)]
pub struct QwMovementProfile {
    /// Provider identity.
    pub id: ProviderId,
    /// Frame clock (must be QuakeWorld).
    pub clock: ClockProfile,
    /// Numeric profile.
    pub numeric: NumericProfile,
    /// Movement tuning.
    pub parameters: Q1MovementParameters,
}

/// NetQuake movement input, mirroring donor `Q1MovementInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1MovementInput {
    /// Shared input fields.
    pub fields: MovementInputFields,
    /// Source command.
    pub command: Q1UserCommand,
    /// Current state.
    pub state: Q1MovementState,
    /// Movement profile.
    pub profile: Q1MovementProfile,
}

/// QuakeWorld movement input, mirroring donor `QwMovementInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct QwMovementInput {
    /// Shared input fields.
    pub fields: MovementInputFields,
    /// Source command.
    pub command: QwUserCommand,
    /// Current state.
    pub state: QwMovementState,
    /// Movement profile.
    pub profile: QwMovementProfile,
}

/// Either Quake I player input.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1PlayerInput {
    /// NetQuake input.
    Netquake(Q1MovementInput),
    /// QuakeWorld input.
    Quakeworld(QwMovementInput),
}

impl Q1PlayerInput {
    /// Shared input fields.
    #[must_use]
    pub fn fields(&self) -> &MovementInputFields {
        match self {
            Q1PlayerInput::Netquake(input) => &input.fields,
            Q1PlayerInput::Quakeworld(input) => &input.fields,
        }
    }

    /// Movement environment.
    #[must_use]
    pub fn environment(&self) -> &MovementEnvironment {
        &self.fields().environment
    }

    /// Frame clock.
    #[must_use]
    pub fn frame(&self) -> &FrameContext {
        &self.fields().frame
    }

    /// Moving actor.
    #[must_use]
    pub fn actor(&self) -> &OwnedActor {
        &self.fields().actor
    }
}

/// Q1 trace policy selector, mirroring donor `move`.
pub use crate::collision::Q1Move as Q1TraceMove;

/// Q1 trace query.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1TraceQuery {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Trace shape.
    pub shape: TraceShape,
    /// Trace policy selector.
    pub policy: Q1TraceMove,
}

/// Q1 trace result, mirroring donor `TraceResult` with `kind: "q1"`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Trace {
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
    /// Started in open space.
    pub in_open: bool,
    /// Started in water.
    pub in_water: bool,
    /// Stored source plane (present even when contact is none).
    pub source_plane: Plane,
    /// Optional surface flags.
    pub surface_flags: Option<i32>,
}

/// Q1 movement contact.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1MovementContact {
    /// Contact target.
    pub target: TraceHit,
    /// Contact trace.
    pub trace: Q1Trace,
    /// Substep index.
    pub substep: usize,
}

/// NetQuake movement result.
pub type Q1MovementResult = MovementOutcome<Q1MovementState, Q1MovementContact>;
/// QuakeWorld movement result.
pub type QwMovementResult = MovementOutcome<QwMovementState, Q1MovementContact>;

impl super::super::swept_body::SweptBodyTrace for Q1Trace {
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

/// Q1 solidity selector, mirroring donor `solid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1Solid {
    /// Not solid.
    Not,
    /// Trigger volume.
    Trigger,
    /// Box solid.
    Box,
    /// Slide box solid.
    SlideBox,
    /// BSP solid.
    Bsp,
    /// Corpse solid.
    Corpse,
}

/// Jump initiation authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Q1JumpAuthority {
    /// Official movement owns initiation.
    #[default]
    SelectedMovement,
    /// Source gamecode owns initiation.
    SourceGamecode,
}

/// Fix-angle roll behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Q1FixAngleRoll {
    /// Original C writes roll during fixangle.
    #[default]
    Source,
    /// Preserve roll (donor stuck-roll fix).
    Preserve,
}

/// Q1 movement sound selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1MovementSound {
    /// Water splash.
    WaterSplash,
    /// Monster landing thud.
    DemonLand,
}

impl Q1MovementSound {
    /// Source sound path.
    #[must_use]
    pub fn path(&self) -> &'static str {
        match self {
            Q1MovementSound::WaterSplash => "misc/h2ohit1.wav",
            Q1MovementSound::DemonLand => "demon/dland2.wav",
        }
    }
}

/// Q1 player action selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1PlayerActionKind {
    /// Jump action.
    Jump,
    /// Swim action.
    Swim,
}

/// Lifecycle phase for game hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1LifecyclePhase {
    /// PlayerPreThink, after client think, before physics.
    BeforePhysics,
    /// Think callback.
    Think,
    /// After physics.
    AfterPhysics,
}

/// Source callbacks publish state before invoking game code and return its
/// changes. Mirrors donor `Q1MovementHooks`.
pub trait Q1MovementHooks {
    /// Publish QuakeWorld water state.
    fn qw_state(&mut self, _water_level: i32, _water_type: i32) {}
    /// Override the trace shape.
    fn shape(&self) -> Option<TraceShape> {
        None
    }
    /// Observe accepted body bounds.
    fn body_shape(&mut self, _bounds: Bounds) {}
    /// Link the actor, touching triggers on request.
    fn link(
        &mut self,
        actor: &OwnedActor,
        state: Q1State,
        touch_triggers: bool,
    ) -> MovementContinuation<Q1State>;
    /// Whether a hit is BSP geometry.
    fn is_bsp(&self, _hit: &TraceHit) -> bool {
        false
    }
    /// Run before physics.
    fn before_physics(
        &mut self,
        input: &Q1PlayerInput,
        state: Q1State,
    ) -> MovementContinuation<Q1State>;
    /// Run think; default keeps state (no think hook).
    fn think(&mut self, _input: &Q1PlayerInput, state: Q1State) -> MovementContinuation<Q1State> {
        MovementContinuation::Continue(state)
    }
    /// Whether a think hook is installed.
    fn has_think(&self) -> bool {
        false
    }
    /// Run after physics.
    fn after_physics(
        &mut self,
        input: &Q1PlayerInput,
        state: Q1State,
    ) -> MovementContinuation<Q1State>;
    /// Play a movement sound.
    fn sound(&mut self, _actor: &OwnedActor, _sound: Q1MovementSound, _state: &Q1MovementState) {}
    /// Observe a player jump/swim action.
    fn player_action(
        &mut self,
        _actor: &OwnedActor,
        _action: Q1PlayerActionKind,
        _state: &Q1MovementState,
    ) {
    }
}

/// No-op hooks for headless runs.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoQ1Hooks;

impl Q1MovementHooks for NoQ1Hooks {
    fn link(
        &mut self,
        _actor: &OwnedActor,
        state: Q1State,
        _touch_triggers: bool,
    ) -> MovementContinuation<Q1State> {
        MovementContinuation::Continue(state)
    }

    fn before_physics(
        &mut self,
        _input: &Q1PlayerInput,
        state: Q1State,
    ) -> MovementContinuation<Q1State> {
        MovementContinuation::Continue(state)
    }

    fn after_physics(
        &mut self,
        _input: &Q1PlayerInput,
        state: Q1State,
    ) -> MovementContinuation<Q1State> {
        MovementContinuation::Continue(state)
    }
}

/// Q1 movement options, mirroring donor `Q1MovementOptions`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1MovementOptions<H = NoQ1Hooks> {
    /// Forced punch angles (source punch override).
    pub source_punch_angles: Option<Vec3>,
    /// View height override.
    pub view_height: Option<f64>,
    /// Maximum velocity component.
    pub max_velocity: Option<f64>,
    /// Disable step-up.
    pub no_step: bool,
    /// Ideal-pitch scale.
    pub ideal_pitch_scale: Option<f64>,
    /// Roll speed.
    pub roll_speed: Option<f64>,
    /// Roll angle.
    pub roll_angle: Option<f64>,
    /// Player solidity selector.
    pub solid: Option<Q1Solid>,
    /// Jump initiation authority.
    pub jump_authority: Q1JumpAuthority,
    /// Fix-angle roll behavior.
    pub fix_angle_roll: Q1FixAngleRoll,
    /// Game hooks.
    pub hooks: Option<H>,
}

impl<H> Default for Q1MovementOptions<H> {
    fn default() -> Self {
        Self {
            source_punch_angles: None,
            view_height: None,
            max_velocity: None,
            no_step: false,
            ideal_pitch_scale: None,
            roll_speed: None,
            roll_angle: None,
            solid: None,
            jump_authority: Q1JumpAuthority::default(),
            fix_angle_roll: Q1FixAngleRoll::default(),
            hooks: None,
        }
    }
}

/// Weapon-step input for the Q1 weapon owner.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1WeaponStepInput<'a> {
    /// Moving actor.
    pub actor: &'a OwnedActor,
    /// Source command.
    pub command: &'a UserCommand,
    /// Frame clock.
    pub frame: &'a FrameContext,
    /// Arsenal snapshot.
    pub arsenal: &'a ArsenalState,
    /// Animation snapshot.
    pub animation: &'a ActorAnimationState,
    /// Movement environment.
    pub environment: &'a MovementEnvironment,
    /// Gauntlet hit flag (always false for Q1).
    pub gauntlet_hit: bool,
}

/// Weapon-step result from the Q1 weapon owner.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1WeaponStepResult {
    /// Optional continuation.
    pub continuation: Option<MovementContinuation<Q1State>>,
    /// Updated arsenal.
    pub arsenal: ArsenalState,
    /// Updated animation.
    pub animation: ActorAnimationState,
    /// Emitted effects.
    pub effects: Vec<MovementEffect>,
}

/// Animation-step input for the Q1 animation owner.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1AnimationStepInput<'a> {
    /// Moving actor.
    pub actor: &'a OwnedActor,
    /// Frame clock.
    pub frame: &'a FrameContext,
    /// Animation snapshot.
    pub animation: &'a ActorAnimationState,
    /// Selected locomotion.
    pub locomotion: LocomotionAnimation,
    /// Backwards flag.
    pub backwards: bool,
    /// Force flag (always false for Q1).
    pub force: bool,
}

/// Animation-step result from the Q1 animation owner.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1AnimationStepResult {
    /// Updated animation.
    pub animation: ActorAnimationState,
    /// Emitted effects.
    pub effects: Vec<MovementEffect>,
}

/// Input application boundary for authoritative Q1 steps.
pub trait Q1InputApplication {
    /// Begin input application for a command.
    fn begin(
        &mut self,
        command: UserCommand,
        frame: &FrameContext,
        state: Q1State,
    ) -> MovementInputContinuation<Q1State>;
    /// End input application.
    fn end(&mut self, state: Q1State, failed: bool) -> MovementContinuation<Q1State>;
}

/// Movement services for Quake I steps, mirroring donor `MovementServices`.
pub trait Q1MovementServices {
    /// Numeric operations for the selected profile.
    fn numeric(&self) -> qa_core::numeric::NumericOps;
    /// Run a Q1 trace.
    fn trace(&mut self, query: Q1TraceQuery) -> Q1Trace;
    /// Q1-translated point contents (source currents collapsed to water).
    fn point_contents(&mut self, point: Vec3) -> i32;
    /// Dispatch a touch contact.
    fn touch(
        &mut self,
        contact: MovementTouchContact,
        state: Q1State,
    ) -> MovementContinuation<Q1State>;
    /// Run the weapon owner.
    fn weapon_step(&mut self, input: Q1WeaponStepInput<'_>, state: &Q1State) -> Q1WeaponStepResult;
    /// Run the animation owner.
    fn animation_step(&mut self, input: Q1AnimationStepInput<'_>) -> Q1AnimationStepResult;
    /// Optional authoritative input application.
    fn input_application(&mut self) -> Option<&mut dyn Q1InputApplication> {
        None
    }
}

/// Q1 physics entity snapshot for pusher transactions, mirroring donor
/// `Q1PhysicsEntity`.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1PhysicsEntity {
    /// Owning actor.
    pub actor: OwnedActor,
    /// Movement state.
    pub state: Q1MovementState,
    /// Local bounds.
    pub bounds: Bounds,
    /// Absolute bounds from the last source link.
    pub absolute_bounds: Bounds,
    /// Solidity.
    pub solid: Q1Solid,
    /// Local pusher time in seconds.
    pub local_time_seconds: f64,
    /// Next think time in seconds.
    pub next_think_seconds: f64,
}

/// Pusher transaction result, mirroring donor `Q1PusherResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1PusherResult {
    /// Pusher actor.
    pub actor: ActorId,
    /// Transaction status.
    pub status: Q1PusherStatus,
    /// Moved actors.
    pub moved: Vec<ActorId>,
}

/// Pusher transaction status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1PusherStatus {
    /// Pusher moved.
    Moved,
    /// Pusher blocked.
    Blocked,
    /// Actor removed mid-transaction.
    ActorRemoved,
}

/// Pusher services, mirroring donor `Q1PusherServices`.
pub trait Q1PusherServices {
    /// Numeric operations.
    fn numeric(&self) -> qa_core::numeric::NumericOps;
    /// Read an entity snapshot.
    fn read(&mut self, actor: &ActorId) -> Option<Q1PhysicsEntity>;
    /// Candidate actors in source slot order.
    fn candidates(&mut self) -> Vec<ActorId>;
    /// Write an entity snapshot.
    fn write(&mut self, entity: Q1PhysicsEntity);
    /// Link an actor.
    fn link(&mut self, actor: &OwnedActor, touch_triggers: bool);
    /// Toggle collision for one actor.
    fn collision_enabled(&mut self, actor: &OwnedActor, enabled: bool);
    /// Test an entity's position.
    fn test_position(&mut self, entity: &Q1PhysicsEntity) -> TraceHit;
    /// Push an entity by a displacement.
    fn push(
        &mut self,
        entity: &Q1PhysicsEntity,
        displacement: Vec3,
    ) -> (Option<Q1PhysicsEntity>, Q1Trace);
    /// Blocked callback.
    fn blocked(&mut self, pusher: &OwnedActor, obstacle: &ActorId);
    /// Think callback.
    fn think(&mut self, _pusher: &OwnedActor) {}
}

/// Fixed pose accessor used by shared result code.
pub fn fixed_pose(environment: &MovementEnvironment) -> Option<FixedMovementPose> {
    environment.pose
}

/// Movement error for dialect changes.
pub fn dialect_error() -> MovementError {
    MovementError::Contract("Source client output changed movement dialect")
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    #[test]
    fn move_and_flag_words_match_source() {
        assert_eq!(Q1_MOVE_WALK, 3);
        assert_eq!(Q1_MOVE_NOCLIP, 8);
        assert_eq!(Q1_FLAG_ONGROUND, 512);
        assert_eq!(Q1_FLAG_WATERJUMP, 2048);
        assert_eq!(Q1_CONTENTS_WATER, -3);
        assert_eq!(Q1_STEP_HEIGHT, 18.0);
    }

    #[test]
    fn sound_paths_match_source() {
        assert_eq!(Q1MovementSound::WaterSplash.path(), "misc/h2ohit1.wav");
        assert_eq!(Q1MovementSound::DemonLand.path(), "demon/dland2.wav");
    }

    #[test]
    fn default_options_disable_extensions() {
        let options: Q1MovementOptions = Q1MovementOptions::default();
        assert!(!options.no_step);
        assert_eq!(options.jump_authority, Q1JumpAuthority::SelectedMovement);
        assert_eq!(options.fix_angle_roll, Q1FixAngleRoll::Source);
        assert!(options.hooks.is_none());
    }

    #[test]
    fn no_hooks_continue_link_and_lifecycle() {
        let owner = IdentityOwner::create("q1-types").unwrap();
        let id = owner.actor(1, 0);
        let actor = owner.owned_actor(&id, ProviderId::new("q1", "test")).unwrap();
        let mut hooks = NoQ1Hooks;
        assert!(!hooks.has_think());
        assert!(hooks.shape().is_none());
        assert!(!hooks.is_bsp(&TraceHit::None));
        let state = Q1State::Quakeworld(QwMovementState {
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            old_buttons: 0,
            water_jump_time_seconds: 0.0,
            dead: false,
            spectator: 0,
            ground: TraceHit::None,
        });
        assert!(matches!(
            hooks.link(&actor, state.clone(), true),
            MovementContinuation::Continue(_)
        ));
    }
}
