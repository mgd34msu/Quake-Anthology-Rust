//! Selected-movement player state and command dispatch.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/players.ts`
//! (`NetQuakeClientBinding`, `PlayerMovementHost`, `providerFamily`,
//! `providerTiming`, `movementProfile`, `movementOrigin`,
//! `movementVelocity`, `MovementPlayer`).
//!
//! # Sibling homes
//!
//! - [`player_movement`](super::player_movement)
//!   (`simulation/player-movement.ts` port): [`player_standing_bounds`]
//!   delegates to the canonical selector; the remaining small selectors
//!   keep module-local shapes over [`MovementPlayer`] while dialect
//!   dispatch calls the `qa-world` movement entry points directly.
//! - [`SharedSimulation`](super::runtime::SharedSimulation)
//!   (`simulation/runtime.ts` port): [`PlayerMovementHost`] is the narrow
//!   host seam.

use std::cell::RefCell;
use std::rc::Rc;

use qa_content::contract::{ExecutableRecipe, GameFamily, ProviderTiming};
use qa_core::identity::{ActorId, ClientId, OwnedActor};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::{NumericOps, NumericProfile};
use qa_core::time::FrameContext;
use qa_net::common::commands::{ActorCommand, UserCommand as NetUserCommand};
use qa_world::movement::q1::types::{
    Q1MovementOptions, Q1MovementProfile, Q1MovementState, QwMovementProfile, QwMovementState,
};
use qa_world::movement::q2::rerelease::Q2RereleaseMovementContext;
use qa_world::movement::types::{
    ActorAnimationState, ArsenalState, MovementDialect, TraceHit, TraceShape, UserCommand as WorldUserCommand,
};

use super::player_input_application::{MovementProfile, MovementState};
use super::player_jump::movement_jumped;
use super::types::PlayerView;

/// NetQuake client binding behind [`PlayerMovementHost::net_quake`].
pub trait NetQuakeClientBinding {
    /// Jump authority.
    fn jump_authority(&self) -> NetQuakeJumpAuthority;
    /// Feed a NetQuake command.
    fn input(&self, command: &qa_world::movement::types::Q1UserCommand);
    /// Pre-physics hook.
    fn before_physics(&self, frame: &FrameContext);
    /// Think hook.
    fn think(&self, frame: &FrameContext);
    /// Post-physics hook.
    fn after_physics(&self, frame: &FrameContext);
}

/// NetQuake jump authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetQuakeJumpAuthority {
    /// Selected movement decides jumps.
    SelectedMovement,
    /// Source gamecode decides jumps.
    SourceGamecode,
}

/// QuakeWorld client hooks behind [`PlayerMovementHost::quake_world`].
pub trait QuakeWorldClientHooks {
    /// Read projected state.
    fn read(&self, state: &QwMovementState) -> QwMovementState;
    /// Write projected state.
    fn write(&self, state: &QwMovementState);
    /// Pre-physics hook.
    fn before_physics(&self, command: &qa_world::movement::types::QwUserCommand, frame: &FrameContext);
    /// Water update.
    fn water(&self, level: i32, water_type: i32);
    /// Project the profile.
    fn profile(&self, profile: &QwMovementProfile) -> QwMovementProfile;
}

/// Movement command view for source-client hooks.
#[derive(Debug, Clone)]
pub struct PlayerMovementCommand {
    /// Moving actor.
    pub actor: OwnedActor,
    /// World command.
    pub command: WorldUserCommand,
    /// Frame context.
    pub frame: FrameContext,
}

/// Source client projection behind [`PlayerMovementHost::source_client`].
pub trait PlayerSourceClient {
    /// Project movement state.
    fn project_state(&self, state: &MovementState) -> MovementState;
    /// Pre-movement projection.
    fn before_movement(
        &self,
        command: &PlayerMovementCommand,
        source_command: Option<&qa_world::movement::types::QwUserCommand>,
    ) -> qa_content::q3::team_arena::movement_host::ClientMovementOptions;
    /// Post-movement hook.
    fn after_movement(
        &self,
        player: &MovementPlayer,
        frame: &FrameContext,
        source_command: Option<&qa_world::movement::types::QwUserCommand>,
    );
}

/// Narrow actor registry view.
pub trait PlayerActors {
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Assert ownership.
    fn assert_owned(&self, actor: &OwnedActor);
}

/// Narrow body table view.
pub trait PlayerBodyView {
    /// Body origin.
    fn origin(&self) -> Vec3;
    /// Body velocity.
    fn velocity(&self) -> Vec3;
    /// Body angles.
    fn angles(&self) -> Vec3;
    /// Body bounds.
    fn bounds(&self) -> Bounds;
    /// Ground actor.
    fn ground(&self) -> Option<ActorId>;
}

/// Narrow body table.
pub trait PlayerBodies {
    /// Read a body.
    fn read(&self, actor: &ActorId) -> Option<PlayerBody>;
    /// Write a body.
    fn write(&self, actor: &OwnedActor, body: PlayerBody);
    /// Link a body.
    fn link(&self, actor: &OwnedActor);
}

/// Owned body snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerBody {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Bounds.
    pub bounds: Vec3,
    /// Hull bounds.
    pub hull: Bounds,
    /// Ground actor.
    pub ground: Option<ActorId>,
}

impl PlayerBodyView for PlayerBody {
    fn origin(&self) -> Vec3 {
        self.origin
    }

    fn velocity(&self) -> Vec3 {
        self.velocity
    }

    fn angles(&self) -> Vec3 {
        self.angles
    }

    fn bounds(&self) -> Bounds {
        self.hull
    }

    fn ground(&self) -> Option<ActorId> {
        self.ground.clone()
    }
}

/// Narrow combat view.
pub trait PlayerCombat {
    /// Read combat health.
    fn health(&self, actor: &ActorId) -> Option<f64>;
    /// Whether combat is invulnerable.
    fn invulnerable(&self, _actor: &ActorId) -> bool {
        false
    }
}

/// Movement player host services.
pub trait PlayerMovementHost {
    /// Client movement outputs.
    fn client_outputs(&self) -> Option<qa_world::movement::types::ModClientMovementOutputs> {
        None
    }
    /// Input applications manager.
    fn input_applications(&self) -> Option<Rc<dyn PlayerInputApplications>> {
        None
    }
    /// Accepted mod-client command.
    fn accepted_input(&self) -> Option<super::mod_client_checkpoint::ModClientCommand> {
        None
    }
    /// Source client projection.
    fn source_client(&self) -> Option<Rc<dyn PlayerSourceClient>> {
        None
    }
    /// Q2 movement config override.
    fn q2_movement_config(&self) -> Option<Q2MovementConfig> {
        None
    }
    /// NetQuake binding.
    fn net_quake(&self) -> Option<Rc<dyn NetQuakeClientBinding>> {
        None
    }
    /// QuakeWorld hooks.
    fn quake_world(&self) -> Option<Rc<dyn QuakeWorldClientHooks>> {
        None
    }
    /// Actor registry.
    fn actors(&self) -> Rc<dyn PlayerActors>;
    /// Body table.
    fn bodies(&self) -> Rc<dyn PlayerBodies>;
    /// Combat view.
    fn combat(&self) -> Rc<dyn PlayerCombat>;
    /// Scene queries.
    fn scene(&self) -> super::prediction::types::PredictionSceneHandle;
    /// Rerelease movement context.
    fn rerelease_movement(&self) -> Rc<RefCell<Q2RereleaseMovementContext>>;
    /// Touch triggers.
    fn touch_triggers(&self, actor: &OwnedActor);
    /// Whether an actor is a brush.
    fn is_brush(&self, actor: &ActorId) -> bool;
    /// World actor.
    fn world_actor(&self) -> Option<ActorId>;
    /// Speed multiplier.
    fn speed_multiplier(&self, _actor: &OwnedActor) -> f64 {
        1.0
    }
    /// Fixed pose override.
    fn fixed_pose(&self, _actor: &OwnedActor) -> Option<qa_world::movement::types::FixedMovementPose> {
        None
    }
    /// Source punch angles.
    fn source_punch(&self, _actor: &ActorId) -> Option<Vec3> {
        None
    }
    /// Whether the player gibbed.
    fn gibbed(&self) -> bool {
        false
    }
    /// Jump or swim action.
    fn jump(&self, actor: &OwnedActor, action: PlayerJumpAction);
    /// Q1 touch dispatch.
    fn touch_q1(
        &self,
        contact: qa_world::movement::types::MovementTouchContact,
        state: qa_world::movement::q1::types::Q1State,
    ) -> qa_world::movement::types::MovementContinuation<qa_world::movement::q1::types::Q1State>;
    /// Q1 weapon step.
    fn weapon_step_q1(
        &self,
        input: qa_world::movement::q1::types::Q1WeaponStepInput<'_>,
        state: &qa_world::movement::q1::types::Q1State,
    ) -> qa_world::movement::q1::types::Q1WeaponStepResult;
    /// Q1 animation step.
    fn animation_step_q1(
        &self,
        input: qa_world::movement::q1::types::Q1AnimationStepInput<'_>,
    ) -> qa_world::movement::q1::types::Q1AnimationStepResult;
    /// Q2 touch dispatch.
    fn touch_q2(
        &self,
        contact: qa_world::movement::q2::types::Q2TouchContact,
        state: qa_world::movement::q2::types::Q2State,
    ) -> qa_world::movement::types::MovementContinuation<qa_world::movement::q2::types::Q2State>;
    /// Q3 movement hooks.
    fn q3_hooks(&self) -> Rc<RefCell<dyn qa_world::movement::q3::types::Q3MovementHooks>>;
}

/// Player jump action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerJumpAction {
    /// Jump.
    Jump,
    /// Swim.
    Swim,
}

/// Q2 movement config override.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2MovementConfig {
    /// Air accelerate.
    pub air_accelerate: f64,
    /// N64 physics.
    pub n64_physics: bool,
}

/// Provider family of a source provider id.
pub fn provider_family(provider: &qa_core::identity::ProviderId) -> GameFamily {
    if provider.namespace == "q1" {
        GameFamily::Q1
    } else if provider.namespace == "q2" {
        GameFamily::Q2
    } else if provider.namespace == "q3" {
        GameFamily::Q3
    } else {
        panic!(
            "Unknown source provider family: {}:{}",
            provider.namespace, provider.name
        );
    }
}

/// Timing entry for a provider.
pub fn provider_timing(recipe: &ExecutableRecipe, provider: &qa_core::identity::ProviderId) -> ProviderTiming {
    recipe
        .timing
        .iter()
        .find(|value| &value.provider == provider)
        .cloned()
        .unwrap_or_else(|| panic!("Recipe has no timing for {}:{}", provider.namespace, provider.name))
}

/// Movement profile selected by a recipe.
pub fn movement_profile(recipe: &ExecutableRecipe) -> MovementProfile {
    let timing = provider_timing(recipe, &recipe.movement.provider);
    match &timing.clock {
        qa_core::time::ClockProfile::Q1Netquake { .. } => MovementProfile::Q1Netquake(Q1MovementProfile {
            id: timing.provider.clone(),
            clock: timing.clock,
            numeric: timing.numeric,
            edition: if recipe.movement.content.0.contains(":rerelease:") {
                qa_world::movement::q1::types::Q1Edition::Rerelease
            } else {
                qa_world::movement::q1::types::Q1Edition::Classic
            },
            parameters: qa_world::movement::Q1MovementParameters {
                gravity: 800.0,
                stop_speed: 100.0,
                max_speed: 320.0,
                spectator_max_speed: 500.0,
                accelerate: 10.0,
                air_accelerate: 10.0,
                water_accelerate: 10.0,
                friction: 4.0,
                water_friction: 4.0,
                entity_gravity: 1.0,
            },
            edge_friction: 2.0,
            no_clip_angle_hack: false,
        }),
        qa_core::time::ClockProfile::Q1Quakeworld { .. } => MovementProfile::Q1Quakeworld(QwMovementProfile {
            id: timing.provider.clone(),
            clock: timing.clock,
            numeric: timing.numeric,
            parameters: qa_world::movement::Q1MovementParameters {
                gravity: 800.0,
                stop_speed: 100.0,
                max_speed: 320.0,
                spectator_max_speed: 500.0,
                accelerate: 10.0,
                air_accelerate: 0.7,
                water_accelerate: 10.0,
                friction: 4.0,
                water_friction: 4.0,
                entity_gravity: 1.0,
            },
        }),
        qa_core::time::ClockProfile::Q2Classic => {
            MovementProfile::Q2Classic(qa_world::movement::q2::types::Q2MovementProfile {
                id: timing.provider.clone(),
                clock: timing.clock,
                numeric: timing.numeric,
                air_accelerate: 0.0,
                snap_initial: true,
                strafejump_hack: false,
            })
        }
        qa_core::time::ClockProfile::Q2Rerelease { .. } => {
            MovementProfile::Q2Rerelease(qa_world::movement::q2::types::Q2RereleaseMovementProfile {
                id: timing.provider.clone(),
                clock: timing.clock,
                numeric: timing.numeric,
                air_accelerate: 0.0,
                n64_physics: false,
            })
        }
        qa_core::time::ClockProfile::Q3 {
            fixed_movement_milliseconds,
            ..
        } => MovementProfile::Q3(qa_world::movement::q3::types::Q3MovementProfile {
            id: timing.provider.clone(),
            clock: timing.clock,
            numeric: timing.numeric,
            product: qa_world::movement::q3::types::Q3Product::BaseQ3,
            fixed_milliseconds: fixed_movement_milliseconds.map(|ms| ms as i32),
            no_footsteps: false,
        }),
    }
}

/// Origin of a movement state.
#[must_use]
pub fn movement_origin(state: &MovementState) -> Vec3 {
    match state {
        MovementState::Q2Classic(state) => qa_core::math::vec3(
            state.origin_eighths[0] as f32 / 8.0,
            state.origin_eighths[1] as f32 / 8.0,
            state.origin_eighths[2] as f32 / 8.0,
        ),
        MovementState::Q1Netquake(state) => state.origin,
        MovementState::Q1Quakeworld(state) => state.origin,
        MovementState::Q2Rerelease(state) => state.origin,
        MovementState::Q3(state) => state.origin,
    }
}

/// Velocity of a movement state.
#[must_use]
pub fn movement_velocity(state: &MovementState) -> Vec3 {
    match state {
        MovementState::Q2Classic(state) => qa_core::math::vec3(
            state.velocity_eighths[0] as f32 / 8.0,
            state.velocity_eighths[1] as f32 / 8.0,
            state.velocity_eighths[2] as f32 / 8.0,
        ),
        MovementState::Q1Netquake(state) => state.velocity,
        MovementState::Q1Quakeworld(state) => state.velocity,
        MovementState::Q2Rerelease(state) => state.velocity,
        MovementState::Q3(state) => state.velocity,
    }
}

/// Standing bounds for a character family.
#[must_use]
pub fn player_standing_bounds(character: GameFamily) -> Bounds {
    super::player_movement::player_standing_bounds(character)
}

/// Crouched bounds for a player.
#[must_use]
pub fn player_crouched_bounds(character: GameFamily, standing: Bounds) -> Bounds {
    match character {
        GameFamily::Q3 => qa_world::movement::q3::postures::Q3_SOURCE_POSTURES.crouched.bounds,
        _ => Bounds {
            min: standing.min,
            max: qa_core::math::vec3(standing.max.x, standing.max.y, 4.0),
        },
    }
}

/// Mutable player core shared with movement hooks.
#[derive(Debug, Clone)]
pub struct PlayerCore {
    /// Movement state.
    pub state: MovementState,
    /// Arsenal state.
    pub arsenal: ArsenalState,
    /// Animation state.
    pub animation: ActorAnimationState,
    /// View angles.
    pub view_angles: Vec3,
    /// Command angles.
    pub command_angles: Vec3,
    /// View height.
    pub view_height: f64,
    /// Body bounds.
    pub bounds: Bounds,
    /// Ground contact.
    pub ground: TraceHit,
    /// Water level.
    pub water_level: i32,
    /// Water type.
    pub water_type: i32,
    /// Intermission lock.
    pub intermission: bool,
    /// Cutscene pose.
    pub cutscene: Option<PlayerCutscene>,
    /// Fixed pose active.
    pub fixed_pose_active: bool,
    /// NetQuake body-shape base.
    pub body_shape_base: Option<Bounds>,
    /// Gravity multiplier.
    pub gravity_multiplier: f64,
    /// Flight enabled.
    pub flight: bool,
    /// World gravity.
    pub world_gravity: f64,
    /// Current buttons.
    pub buttons: i32,
    /// Previous buttons.
    pub previous_buttons: i32,
    /// Last command sequence.
    pub last_sequence: i64,
    /// Pending NetQuake command.
    pub net_quake_command: Option<qa_world::movement::types::Q1UserCommand>,
    /// Open NetQuake input application.
    pub net_quake_application: Option<PlayerInputApplication>,
    /// NetQuake slice command.
    pub net_quake_slice_command: Option<WorldUserCommand>,
    /// Whether the NetQuake slice is open.
    pub net_quake_slice_open: bool,
    /// Parent application invocation.
    pub application_parent: Option<u64>,
    /// Last weapon time in seconds.
    pub last_weapon_seconds: f64,
    /// Pending arsenal intent.
    pub arsenal_intent: Option<qa_net::common::commands::ArsenalIntent>,
    /// Source movement options.
    pub source_movement: Option<qa_content::q3::team_arena::movement_host::ClientMovementOptions>,
    /// Source environment override.
    pub source_environment: Option<qa_world::movement::types::MovementEnvironment>,
}

/// Cutscene pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerCutscene {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// View offset.
    pub view_offset: Vec3,
}

/// Selected-movement player.
pub struct MovementPlayer {
    /// Owning actor.
    pub actor: OwnedActor,
    /// Client id.
    pub client: ClientId,
    /// Executable recipe.
    pub recipe: ExecutableRecipe,
    host: Rc<dyn PlayerMovementHost>,
    /// Character family.
    pub character: GameFamily,
    /// Standing bounds.
    pub standing_bounds: Bounds,
    /// Movement profile.
    pub profile: MovementProfile,
    /// Numeric operations.
    pub numeric: NumericOps,
    core: Rc<RefCell<PlayerCore>>,
}

impl MovementPlayer {
    /// Create a player at an origin with view angles.
    pub fn new(
        actor: OwnedActor,
        client: ClientId,
        recipe: ExecutableRecipe,
        host: Rc<dyn PlayerMovementHost>,
        origin: Vec3,
        angles: Vec3,
        arsenal: ArsenalState,
    ) -> Self {
        let character = provider_family(&recipe.character.definition.provider);
        let standing_bounds = player_standing_bounds(character);
        let view_height = if character == GameFamily::Q3 { 26.0 } else { 22.0 };
        let profile = movement_profile(&recipe);
        let numeric = NumericOps::select(profile_numeric(&profile)).expect("numeric profile");
        let animation_state = match character {
            GameFamily::Q1 => qa_world::movement::types::AnimationState::Q1 {
                frame: 12,
                next_frame_seconds: 0.0,
            },
            GameFamily::Q2 => qa_world::movement::types::AnimationState::Q2 {
                frame: 0,
                end_frame: 39,
                priority: 0,
                duck: false,
                run: false,
            },
            GameFamily::Q3 => qa_world::movement::types::AnimationState::Q3 {
                legs: 22,
                torso: 11,
                legs_timer_milliseconds: 0,
                torso_timer_milliseconds: 0,
            },
        };
        let state = initial_movement_state(&profile, origin, angles, view_height);
        let core = PlayerCore {
            state,
            arsenal,
            animation: ActorAnimationState {
                provider: recipe.character.definition.provider.clone(),
                state: animation_state,
            },
            view_angles: angles,
            command_angles: angles,
            view_height,
            bounds: standing_bounds,
            ground: TraceHit::None,
            water_level: 0,
            water_type: 0,
            intermission: false,
            cutscene: None,
            fixed_pose_active: false,
            body_shape_base: None,
            gravity_multiplier: 1.0,
            flight: false,
            world_gravity: 800.0,
            buttons: 0,
            previous_buttons: 0,
            last_sequence: -1,
            net_quake_command: None,
            net_quake_application: None,
            net_quake_slice_command: None,
            net_quake_slice_open: false,
            application_parent: None,
            last_weapon_seconds: f64::NEG_INFINITY,
            arsenal_intent: None,
            source_movement: None,
            source_environment: None,
        };
        Self {
            actor,
            client,
            recipe,
            host,
            character,
            standing_bounds,
            profile,
            numeric,
            core: Rc::new(RefCell::new(core)),
        }
    }

    /// Borrow the mutable core.
    #[must_use]
    pub fn core(&self) -> Rc<RefCell<PlayerCore>> {
        self.core.clone()
    }

    /// Prediction profile with QuakeWorld projection.
    #[must_use]
    pub fn prediction_profile(&self) -> MovementProfile {
        let profile = selected_movement_profile(self);
        match &profile {
            MovementProfile::Q1Quakeworld(qw) => self
                .host
                .quake_world()
                .map(|hooks| MovementProfile::Q1Quakeworld(hooks.profile(qw)))
                .unwrap_or(profile),
            _ => profile,
        }
    }

    /// Movement speed multiplier.
    #[must_use]
    pub fn movement_speed_multiplier(&self) -> f64 {
        self.host.speed_multiplier(&self.actor)
    }

    /// Prediction environment.
    #[must_use]
    pub fn prediction_environment(&self) -> qa_world::movement::types::MovementEnvironment {
        let health = self.host.combat().health(self.actor.id()).expect("combat state");
        let invulnerable = self.host.combat().invulnerable(self.actor.id());
        let mut env = player_movement_environment(
            self.source_environment(),
            self.gravity_multiplier(),
            &self.state(),
            self.flight() && health > 0.0,
            health,
            invulnerable,
            self.movement_speed_multiplier(),
        );
        if let Some(pose) = self.fixed_pose_update() {
            env.pose = Some(pose);
        }
        env.client_outputs = Some(self.client_movement_outputs());
        env
    }

    /// Q2 movement config.
    #[must_use]
    pub fn q2_movement_config(&self) -> Option<Q2MovementConfig> {
        self.host.q2_movement_config()
    }

    /// Read a core field.
    fn core_get<T: Clone>(&self, select: impl FnOnce(&PlayerCore) -> T) -> T {
        select(&self.core.borrow())
    }

    /// Current movement state.
    #[must_use]
    pub fn state(&self) -> MovementState {
        self.core_get(|core| core.state.clone())
    }

    /// Current arsenal.
    #[must_use]
    pub fn arsenal(&self) -> ArsenalState {
        self.core_get(|core| core.arsenal.clone())
    }

    /// Current animation.
    #[must_use]
    pub fn animation(&self) -> ActorAnimationState {
        self.core_get(|core| core.animation.clone())
    }

    /// Current view angles.
    #[must_use]
    pub fn view_angles(&self) -> Vec3 {
        self.core_get(|core| core.view_angles)
    }

    /// Current bounds.
    #[must_use]
    pub fn bounds(&self) -> Bounds {
        self.core_get(|core| core.bounds)
    }

    /// Current view height.
    #[must_use]
    pub fn view_height(&self) -> f64 {
        self.core_get(|core| core.view_height)
    }

    /// Current ground.
    #[must_use]
    pub fn ground(&self) -> TraceHit {
        self.core_get(|core| core.ground.clone())
    }

    /// Flight flag.
    #[must_use]
    pub fn flight(&self) -> bool {
        self.core_get(|core| core.flight)
    }

    /// Gravity multiplier.
    #[must_use]
    pub fn gravity_multiplier(&self) -> f64 {
        self.core_get(|core| core.gravity_multiplier)
    }

    /// World gravity.
    #[must_use]
    pub fn world_gravity(&self) -> f64 {
        self.core_get(|core| core.world_gravity)
    }

    /// Source environment override.
    #[must_use]
    pub fn source_environment(&self) -> Option<qa_world::movement::types::MovementEnvironment> {
        self.core_get(|core| core.source_environment)
    }

    /// Last sequence.
    #[must_use]
    pub fn last_sequence(&self) -> i64 {
        self.core_get(|core| core.last_sequence)
    }
}

/// Input application scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerInputScope {
    /// Movement slice.
    MovementSlice,
    /// Client command.
    ClientCommand,
}

/// Command angle space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerAngleSpace {
    /// Absolute aim.
    Absolute,
    /// Source-relative aim.
    SourceRelative,
}

/// Open input application.
#[derive(Debug, Clone)]
pub struct PlayerInputApplication {
    /// Projected command.
    pub command: WorldUserCommand,
    /// Projected arsenal.
    pub arsenal: Option<qa_net::common::commands::ArsenalIntent>,
    /// Invocation id.
    pub invocation: u64,
}

/// Narrow input applications manager.
pub trait PlayerInputApplications {
    /// Whether applications are active.
    fn active(&self) -> bool;
    /// Begin an application.
    #[allow(clippy::too_many_arguments)]
    fn begin(
        &self,
        actor: &ActorId,
        client: &ClientId,
        scope: PlayerInputScope,
        command: &WorldUserCommand,
        angle_space: PlayerAngleSpace,
        absolute_aim: Vec3,
        frame: &FrameContext,
        accepted: Option<super::mod_client_checkpoint::ModClientCommand>,
        arsenal: Option<qa_net::common::commands::ArsenalIntent>,
        parent_invocation: Option<u64>,
        aim: &dyn Fn(Vec3, &WorldUserCommand) -> WorldUserCommand,
    ) -> Option<PlayerInputApplication>;
    /// Finish an application.
    fn finish(&self, application: Option<&PlayerInputApplication>, failed: bool);
}

/// Numeric profile of a movement profile.
fn profile_numeric(profile: &MovementProfile) -> NumericProfile {
    match profile {
        MovementProfile::Q1Netquake(profile) => profile.numeric,
        MovementProfile::Q1Quakeworld(profile) => profile.numeric,
        MovementProfile::Q2Classic(profile) => profile.numeric,
        MovementProfile::Q2Rerelease(profile) => profile.numeric,
        MovementProfile::Q3(profile) => profile.numeric,
    }
}

/// Initial movement state for a profile.
fn initial_movement_state(profile: &MovementProfile, origin: Vec3, angles: Vec3, view_height: f64) -> MovementState {
    let zero = qa_core::math::vec3(0.0, 0.0, 0.0);
    match profile {
        MovementProfile::Q1Netquake(_) => MovementState::Q1Netquake(Q1MovementState {
            origin,
            velocity: zero,
            angles,
            old_origin: origin,
            angular_velocity: zero,
            view_angles: angles,
            punch_angles: zero,
            move_type: 3,
            flags: 4096,
            ground: TraceHit::None,
            water_level: 0,
            water_type: -1,
            teleport_time_seconds: 0.0,
            water_jump_direction: zero,
            ideal_pitch: 0.0,
            fix_angle: false,
            health: 100.0,
        }),
        MovementProfile::Q1Quakeworld(_) => MovementState::Q1Quakeworld(QwMovementState {
            origin,
            velocity: zero,
            angles,
            old_buttons: 0,
            water_jump_time_seconds: 0.0,
            dead: false,
            spectator: 0,
            ground: TraceHit::None,
        }),
        MovementProfile::Q2Classic(_) => MovementState::Q2Classic(qa_world::movement::q2::types::Q2MovementState {
            move_type: 0,
            origin_eighths: eighths(origin),
            velocity_eighths: [0, 0, 0],
            flags: 0,
            time_eight_milliseconds: 0,
            gravity: 800.0,
            delta_angle_shorts: [0, 0, 0],
        }),
        MovementProfile::Q2Rerelease(_) => {
            MovementState::Q2Rerelease(qa_world::movement::q2::types::Q2RereleaseMovementState {
                move_type: 0,
                origin,
                velocity: zero,
                flags: 0,
                time_milliseconds: 0,
                gravity: 800.0,
                delta_angles: zero,
                view_height,
            })
        }
        MovementProfile::Q3(_) => MovementState::Q3(qa_world::movement::q3::types::Q3MovementState {
            command_time_milliseconds: 0,
            movement_type: 0,
            bob_cycle: 0,
            movement_flags: 0,
            movement_time_milliseconds: 0,
            origin,
            velocity: zero,
            gravity: 800.0,
            speed: 320.0,
            delta_angle_words: [0, 0, 0],
            movement_direction: 0,
            grapple_point: zero,
            flags: 0,
            view_angles: angles,
            view_height,
            ground: TraceHit::None,
            predictable_event_sequence: 0,
            jump_pad: None,
            movement_frame: 0,
            jump_pad_frame: 0,
        }),
    }
}

/// Eighths of a position.
fn eighths(value: Vec3) -> [i32; 3] {
    [
        (value.x * 8.0).trunc() as i32,
        (value.y * 8.0).trunc() as i32,
        (value.z * 8.0).trunc() as i32,
    ]
}

/// Selected movement profile with live overrides.
#[must_use]
pub fn selected_movement_profile(player: &MovementPlayer) -> MovementProfile {
    let mut profile = player.profile.clone();
    match &mut profile {
        MovementProfile::Q1Netquake(selected) => {
            selected.parameters.gravity = player.world_gravity();
        }
        MovementProfile::Q1Quakeworld(selected) => {
            selected.parameters.gravity = player.world_gravity();
        }
        MovementProfile::Q3(selected) => {
            if let Some(source) = player.core_get(|core| core.source_movement) {
                selected.fixed_milliseconds = source.fixed_msec;
                selected.no_footsteps = source.no_footsteps;
            }
        }
        MovementProfile::Q2Classic(selected) => {
            if let Some(q2) = player.q2_movement_config() {
                selected.air_accelerate = q2.air_accelerate;
            }
        }
        MovementProfile::Q2Rerelease(selected) => {
            if let Some(q2) = player.q2_movement_config() {
                selected.air_accelerate = q2.air_accelerate;
                selected.n64_physics = q2.n64_physics;
            }
        }
    }
    profile
}

/// Movement environment for a player and combat state.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn player_movement_environment(
    source: Option<qa_world::movement::types::MovementEnvironment>,
    gravity_multiplier: f64,
    state: &MovementState,
    flight: bool,
    health: f64,
    invulnerable: bool,
    speed_multiplier: f64,
) -> qa_world::movement::types::MovementEnvironment {
    let speed = if speed_multiplier == 1.0 {
        None
    } else {
        Some(speed_multiplier)
    };
    let gravity = if matches!(state, MovementState::Q3(_)) {
        1.0
    } else {
        gravity_multiplier
    };
    match source {
        None => qa_world::movement::types::MovementEnvironment {
            client_outputs: None,
            speed_multiplier: speed,
            pose: None,
            health,
            flight,
            haste: false,
            invulnerable,
            gravity_multiplier: gravity,
        },
        Some(source) => qa_world::movement::types::MovementEnvironment {
            speed_multiplier: speed,
            flight: (source.flight || flight) && health > 0.0,
            health,
            invulnerable: source.invulnerable || invulnerable,
            gravity_multiplier: if matches!(state, MovementState::Q3(_)) {
                1.0
            } else {
                source.gravity_multiplier
            },
            ..source
        },
    }
}

/// Movement continuation after a commit.
#[derive(Debug, Clone)]
pub enum PlayerContinuation {
    /// Continue with state.
    Continue(MovementState),
    /// Actor removed.
    ActorRemoved,
}

impl MovementPlayer {
    /// Set flight.
    pub fn set_flight(&self, enabled: bool) -> bool {
        let health = self.host.combat().health(self.actor.id()).unwrap_or(0.0);
        let flight = enabled && health > 0.0;
        let mut core = self.core.borrow_mut();
        core.flight = flight;
        match &mut core.state {
            MovementState::Q1Netquake(state) => {
                state.move_type = if flight {
                    5
                } else if state.move_type == 5 {
                    3
                } else {
                    state.move_type
                };
                if flight {
                    state.flags &= !512;
                    state.ground = TraceHit::None;
                }
            }
            MovementState::Q1Quakeworld(state) => {
                if flight {
                    state.spectator = 0;
                    state.ground = TraceHit::None;
                    state.water_jump_time_seconds = 0.0;
                }
            }
            MovementState::Q2Classic(state) => {
                if flight {
                    state.move_type = 0;
                    state.flags &= !4;
                }
            }
            MovementState::Q2Rerelease(state) => {
                if flight {
                    state.move_type = 0;
                    state.flags &= !4;
                }
            }
            MovementState::Q3(state) => {
                if flight {
                    state.movement_type = 0;
                    state.ground = TraceHit::None;
                }
            }
        }
        if flight {
            core.ground = TraceHit::None;
            if let Some(mut body) = self.host.bodies().read(self.actor.id()) {
                body.ground = None;
                self.host.bodies().write(&self.actor, body);
            }
        }
        flight
    }

    /// Set the source view roll.
    pub fn set_source_view_roll(&self, roll: f32) {
        self.host.actors().assert_owned(&self.actor);
        let mut core = self.core.borrow_mut();
        core.view_angles.z = roll;
        let z = core.view_angles.z;
        match &mut core.state {
            MovementState::Q1Netquake(state) => {
                state.view_angles.z = z;
            }
            MovementState::Q3(state) => {
                state.view_angles.z = z;
            }
            MovementState::Q1Quakeworld(state) => {
                state.angles.z = z;
            }
            _ => {}
        }
    }

    /// Player view.
    pub fn view(&self) -> PlayerView {
        let body = self.host.bodies().read(self.actor.id()).expect("live body");
        PlayerView {
            client_view_offset_delta: None,
            blend: None,
            damage_blend: None,
            origin: body.origin,
            angles: self.view_angles(),
            view_height: self.view_height(),
            kick_angles: None,
            field_of_view: None,
            foreign_character_death: false,
            pitch_drift: None,
        }
    }

    /// Read projected movement state.
    pub fn read_state(&self) -> MovementState {
        read_projected_state(&self.host, &self.actor, &self.core)
    }
}

/// Read projected state with explicit handles.
fn read_projected_state(
    host: &Rc<dyn PlayerMovementHost>,
    actor: &OwnedActor,
    core: &Rc<RefCell<PlayerCore>>,
) -> MovementState {
    let state = read_body_state(host, actor, core);
    host.source_client()
        .map_or(state.clone(), |source| source.project_state(&state))
}

/// Read movement state from the live body with explicit handles.
fn read_body_state(
    host: &Rc<dyn PlayerMovementHost>,
    actor: &OwnedActor,
    core: &Rc<RefCell<PlayerCore>>,
) -> MovementState {
    let body = host.bodies().read(actor.id()).expect("live body");
    let world = host.world_actor();
    let ground = match &body.ground {
        None => TraceHit::None,
        Some(actor) => {
            if world.as_ref() == Some(actor) {
                TraceHit::World { model: 0 }
            } else {
                TraceHit::Actor { actor: actor.clone() }
            }
        }
    };
    core.borrow_mut().ground = ground.clone();
    let health = host.combat().health(actor.id()).unwrap_or(0.0);
    let state = core.borrow().state.clone();
    match state {
        MovementState::Q1Netquake(mut current) => {
            current.origin = body.origin;
            current.velocity = body.velocity;
            current.angles = body.angles;
            current.ground = ground.clone();
            if matches!(ground, TraceHit::None) {
                current.flags &= !512;
            } else {
                current.flags |= 512;
            }
            current.health = health;
            MovementState::Q1Netquake(current)
        }
        MovementState::Q1Quakeworld(mut current) => {
            current.origin = body.origin;
            current.velocity = body.velocity;
            current.angles = body.angles;
            current.ground = ground;
            current.dead = health <= 0.0;
            MovementState::Q1Quakeworld(current)
        }
        MovementState::Q2Classic(mut current) => {
            current.origin_eighths = eighths(body.origin);
            current.velocity_eighths = eighths(body.velocity);
            if health <= 0.0 {
                current.move_type = if host.gibbed() { 3 } else { 2 };
            }
            MovementState::Q2Classic(current)
        }
        MovementState::Q2Rerelease(mut current) => {
            current.origin = body.origin;
            current.velocity = body.velocity;
            if health <= 0.0 {
                current.move_type = if host.gibbed() { 5 } else { 4 };
            }
            MovementState::Q2Rerelease(current)
        }
        MovementState::Q3(mut current) => {
            current.origin = body.origin;
            current.velocity = body.velocity;
            current.ground = ground;
            if health <= 0.0 {
                current.movement_type = 3;
            }
            MovementState::Q3(current)
        }
    }
}

impl MovementPlayer {
    /// Commit movement state to the body.
    pub fn commit(&self, state: MovementState, link: bool, triggers: bool) -> PlayerContinuation {
        Self::commit_with(&self.host, &self.actor, &self.core, state, link, triggers)
    }

    /// Commit with explicit handles for hook adapters.
    pub fn commit_with(
        host: &Rc<dyn PlayerMovementHost>,
        actor: &OwnedActor,
        core: &Rc<RefCell<PlayerCore>>,
        state: MovementState,
        link: bool,
        triggers: bool,
    ) -> PlayerContinuation {
        if !host.actors().is_live(actor.id()) {
            return PlayerContinuation::ActorRemoved;
        }
        {
            let mut core = core.borrow_mut();
            core.state = state.clone();
            match &state {
                MovementState::Q1Netquake(nq) => {
                    core.ground = if nq.flags & 512 != 0 {
                        nq.ground.clone()
                    } else {
                        TraceHit::None
                    };
                    core.water_level = nq.water_level;
                    core.water_type = nq.water_type;
                    core.view_angles = nq.view_angles;
                }
                MovementState::Q1Quakeworld(qw) => {
                    core.ground = qw.ground.clone();
                }
                MovementState::Q3(q3) => {
                    core.ground = q3.ground.clone();
                }
                _ => {}
            }
        }
        let body = host.bodies().read(actor.id()).expect("live body");
        if matches!(state, MovementState::Q1Netquake(_)) && host.net_quake().is_some() {
            core.borrow_mut().bounds = body.hull;
        }
        let angles = match &state {
            MovementState::Q1Netquake(nq) => nq.angles,
            MovementState::Q1Quakeworld(qw) => qw.angles,
            _ => core.borrow().view_angles,
        };
        if matches!(state, MovementState::Q1Quakeworld(_)) && host.quake_world().is_some() {
            if let MovementState::Q1Quakeworld(qw) = &state {
                host.quake_world().expect("qw").write(qw);
            }
            core.borrow_mut().bounds = body.hull;
        } else {
            let (ground, bounds) = {
                let borrowed = core.borrow();
                (borrowed.ground.clone(), borrowed.bounds)
            };
            let mut next = body.clone();
            next.origin = movement_origin(&state);
            next.velocity = movement_velocity(&state);
            next.angles = angles;
            next.hull = bounds;
            next.ground = match &ground {
                TraceHit::Actor { actor } => Some(actor.clone()),
                TraceHit::World { .. } => host.world_actor(),
                TraceHit::None => None,
            };
            host.bodies().write(actor, next);
        }
        if link {
            host.bodies().link(actor);
        }
        if triggers {
            host.touch_triggers(actor);
        }
        if host.actors().is_live(actor.id()) {
            PlayerContinuation::Continue(read_projected_state(host, actor, core))
        } else {
            PlayerContinuation::ActorRemoved
        }
    }

    /// Client movement outputs with body-shape tracking.
    fn client_movement_outputs(&self) -> qa_world::movement::types::ModClientMovementOutputs {
        let outputs = self.host.client_outputs().unwrap_or_default();
        let is_nq = matches!(self.profile, MovementProfile::Q1Netquake(_));
        let is_qw = matches!(self.profile, MovementProfile::Q1Quakeworld(_)) && self.host.quake_world().is_some();
        if !is_nq && !is_qw {
            return outputs;
        }
        if outputs.body_bounds.is_some() {
            let mut core = self.core.borrow_mut();
            if core.body_shape_base.is_none() {
                core.body_shape_base = Some(core.bounds);
            }
            return outputs;
        }
        let base = self.core.borrow().body_shape_base;
        let Some(base) = base else {
            return outputs;
        };
        let current = self.bounds();
        if current.min == base.min && current.max == base.max {
            self.core.borrow_mut().body_shape_base = None;
        }
        qa_world::movement::types::ModClientMovementOutputs {
            body_bounds: Some(base),
            ..outputs
        }
    }

    /// Apply the fixed pose, returning it when active.
    fn fixed_pose_update(&self) -> Option<qa_world::movement::types::FixedMovementPose> {
        let pose = self.host.fixed_pose(&self.actor);
        if pose.is_none() && self.core.borrow().fixed_pose_active {
            let bounds = match self.profile {
                MovementProfile::Q1Quakeworld(_) => player_crouched_bounds(self.character, self.standing_bounds),
                _ => self.standing_bounds,
            };
            let view_height = if self.character == GameFamily::Q3 { 26.0 } else { 22.0 };
            let mut core = self.core.borrow_mut();
            core.bounds = bounds;
            core.view_height = view_height;
            if let Some(mut body) = self.host.bodies().read(self.actor.id()) {
                body.hull = bounds;
                self.host.bodies().write(&self.actor, body);
            }
        }
        if let Some(pose) = pose {
            let mut core = self.core.borrow_mut();
            core.bounds = pose.bounds;
            core.view_height = pose.view_height;
            if let Some(mut body) = self.host.bodies().read(self.actor.id()) {
                body.hull = pose.bounds;
                self.host.bodies().write(&self.actor, body);
            }
        }
        self.core.borrow_mut().fixed_pose_active = pose.is_some();
        pose
    }
}

/// Q1 service adapter over the player host.
struct PlayerQ1Services {
    host: Rc<dyn PlayerMovementHost>,
    numeric: NumericOps,
    slice: Option<PlayerSliceApplication>,
}

impl qa_world::movement::q1::types::Q1MovementServices for PlayerQ1Services {
    fn numeric(&self) -> NumericOps {
        self.numeric
    }

    fn trace(&mut self, query: qa_world::movement::q1::types::Q1TraceQuery) -> qa_world::movement::q1::types::Q1Trace {
        self.host.scene().borrow_mut().trace_q1(query)
    }

    fn point_contents(&mut self, point: Vec3) -> i32 {
        self.host.scene().borrow_mut().point_contents_q1(point)
    }

    fn touch(
        &mut self,
        contact: qa_world::movement::types::MovementTouchContact,
        state: qa_world::movement::q1::types::Q1State,
    ) -> qa_world::movement::types::MovementContinuation<qa_world::movement::q1::types::Q1State> {
        self.host.touch_q1(contact, state)
    }

    fn weapon_step(
        &mut self,
        input: qa_world::movement::q1::types::Q1WeaponStepInput<'_>,
        state: &qa_world::movement::q1::types::Q1State,
    ) -> qa_world::movement::q1::types::Q1WeaponStepResult {
        self.host.weapon_step_q1(input, state)
    }

    fn animation_step(
        &mut self,
        input: qa_world::movement::q1::types::Q1AnimationStepInput<'_>,
    ) -> qa_world::movement::q1::types::Q1AnimationStepResult {
        self.host.animation_step_q1(input)
    }

    fn input_application(&mut self) -> Option<&mut dyn qa_world::movement::q1::types::Q1InputApplication> {
        None
    }
}

/// Q1 hook adapter closing over the player.
#[derive(Clone)]
struct PlayerQ1Hooks {
    player: PlayerHookTarget,
    /// Whether the NetQuake think hook is active.
    think_netquake: bool,
    /// Whether the source consumed the jump.
    source_jump: bool,
}

/// Shared hook target.
#[derive(Clone)]
struct PlayerHookTarget {
    actor: OwnedActor,
    client: ClientId,
    host: Rc<dyn PlayerMovementHost>,
    numeric: NumericOps,
    core: Rc<RefCell<PlayerCore>>,
}

impl qa_world::movement::q1::types::Q1MovementHooks for PlayerQ1Hooks {
    fn qw_state(&mut self, water_level: i32, water_type: i32) {
        if let Some(qw) = self.player.host.quake_world() {
            qw.water(water_level, water_type);
        }
    }

    fn shape(&self) -> Option<TraceShape> {
        self.player.host.net_quake()?;
        let body = self
            .player
            .host
            .bodies()
            .read(self.player.actor.id())
            .expect("live body");
        Some(TraceShape::Box(body.hull))
    }

    fn body_shape(&mut self, bounds: Bounds) {
        self.player.core.borrow_mut().bounds = bounds;
        if let Some(mut body) = self.player.host.bodies().read(self.player.actor.id()) {
            body.hull = bounds;
            self.player.host.bodies().write(&self.player.actor, body);
        }
    }

    fn link(
        &mut self,
        actor: &OwnedActor,
        state: qa_world::movement::q1::types::Q1State,
        touch_triggers: bool,
    ) -> qa_world::movement::types::MovementContinuation<qa_world::movement::q1::types::Q1State> {
        let _ = actor;
        let movement = q1_movement_state(&state);
        match MovementPlayer::commit_with(
            &self.player.host,
            &self.player.actor,
            &self.player.core,
            movement,
            true,
            touch_triggers,
        ) {
            PlayerContinuation::Continue(_) => qa_world::movement::types::MovementContinuation::Continue(state),
            PlayerContinuation::ActorRemoved => qa_world::movement::types::MovementContinuation::ActorRemoved,
        }
    }

    fn is_bsp(&self, hit: &TraceHit) -> bool {
        match hit {
            TraceHit::World { .. } => true,
            TraceHit::Actor { actor } => self.player.host.is_brush(actor),
            TraceHit::None => false,
        }
    }

    fn before_physics(
        &mut self,
        input: &qa_world::movement::q1::types::Q1PlayerInput,
        state: qa_world::movement::q1::types::Q1State,
    ) -> qa_world::movement::types::MovementContinuation<qa_world::movement::q1::types::Q1State> {
        let movement = q1_movement_state(&state);
        let committed = MovementPlayer::commit_with(
            &self.player.host,
            &self.player.actor,
            &self.player.core,
            movement,
            false,
            false,
        );
        if matches!(committed, PlayerContinuation::ActorRemoved) {
            return qa_world::movement::types::MovementContinuation::ActorRemoved;
        }
        match input {
            qa_world::movement::q1::types::Q1PlayerInput::Netquake(input) => {
                if let Some(binding) = self.player.host.net_quake() {
                    binding.before_physics(&input.fields.frame);
                }
            }
            qa_world::movement::q1::types::Q1PlayerInput::Quakeworld(input) => {
                if let Some(qw) = self.player.host.quake_world() {
                    qw.before_physics(&input.command, &input.fields.frame);
                }
            }
        }
        if self.player.host.actors().is_live(self.player.actor.id()) {
            qa_world::movement::types::MovementContinuation::Continue(state)
        } else {
            qa_world::movement::types::MovementContinuation::ActorRemoved
        }
    }

    fn think(
        &mut self,
        input: &qa_world::movement::q1::types::Q1PlayerInput,
        state: qa_world::movement::q1::types::Q1State,
    ) -> qa_world::movement::types::MovementContinuation<qa_world::movement::q1::types::Q1State> {
        let movement = q1_movement_state(&state);
        let committed = MovementPlayer::commit_with(
            &self.player.host,
            &self.player.actor,
            &self.player.core,
            movement,
            false,
            false,
        );
        if matches!(committed, PlayerContinuation::ActorRemoved) {
            return qa_world::movement::types::MovementContinuation::ActorRemoved;
        }
        if let qa_world::movement::q1::types::Q1PlayerInput::Netquake(input) = input {
            if let Some(binding) = self.player.host.net_quake() {
                binding.think(&input.fields.frame);
            }
        }
        if self.player.host.actors().is_live(self.player.actor.id()) {
            qa_world::movement::types::MovementContinuation::Continue(state)
        } else {
            qa_world::movement::types::MovementContinuation::ActorRemoved
        }
    }

    fn has_think(&self) -> bool {
        self.think_netquake
    }

    fn after_physics(
        &mut self,
        input: &qa_world::movement::q1::types::Q1PlayerInput,
        state: qa_world::movement::q1::types::Q1State,
    ) -> qa_world::movement::types::MovementContinuation<qa_world::movement::q1::types::Q1State> {
        let movement = q1_movement_state(&state);
        let committed = MovementPlayer::commit_with(
            &self.player.host,
            &self.player.actor,
            &self.player.core,
            movement,
            false,
            false,
        );
        if matches!(committed, PlayerContinuation::ActorRemoved) {
            return qa_world::movement::types::MovementContinuation::ActorRemoved;
        }
        if let qa_world::movement::q1::types::Q1PlayerInput::Netquake(input) = input {
            if let Some(binding) = self.player.host.net_quake() {
                binding.after_physics(&input.fields.frame);
            }
        }
        if self.player.host.actors().is_live(self.player.actor.id()) {
            qa_world::movement::types::MovementContinuation::Continue(state)
        } else {
            qa_world::movement::types::MovementContinuation::ActorRemoved
        }
    }

    fn player_action(
        &mut self,
        actor: &OwnedActor,
        action: qa_world::movement::q1::types::Q1PlayerActionKind,
        _state: &qa_world::movement::q1::types::Q1MovementState,
    ) {
        let jump = matches!(action, qa_world::movement::q1::types::Q1PlayerActionKind::Jump);
        if !jump || !self.source_jump {
            self.player.host.jump(
                actor,
                if jump {
                    PlayerJumpAction::Jump
                } else {
                    PlayerJumpAction::Swim
                },
            );
        }
    }
}

/// Wrap a Q1 state in its movement state.
fn q1_movement_state(state: &qa_world::movement::q1::types::Q1State) -> MovementState {
    match state {
        qa_world::movement::q1::types::Q1State::Netquake(state) => MovementState::Q1Netquake(state.clone()),
        qa_world::movement::q1::types::Q1State::Quakeworld(state) => MovementState::Q1Quakeworld(state.clone()),
    }
}

/// Q2 service adapter over the player host.
struct PlayerQ2Services {
    host: Rc<dyn PlayerMovementHost>,
    numeric: NumericOps,
    slice: Option<PlayerSliceApplication>,
}

impl qa_world::movement::q2::types::Q2MovementServices for PlayerQ2Services {
    fn numeric(&self) -> NumericOps {
        self.numeric
    }

    fn trace(&mut self, query: qa_world::movement::q2::types::Q2TraceQuery) -> qa_world::movement::q2::types::Q2Trace {
        self.host.scene().borrow_mut().trace_q2(query)
    }

    fn point_contents(&mut self, query: qa_world::movement::q2::types::Q2ContentsQuery) -> (i32, i32) {
        self.host.scene().borrow_mut().point_contents_q2(query)
    }

    fn touch(
        &mut self,
        contact: qa_world::movement::q2::types::Q2TouchContact,
        state: qa_world::movement::q2::types::Q2State,
    ) -> qa_world::movement::types::MovementContinuation<qa_world::movement::q2::types::Q2State> {
        self.host.touch_q2(contact, state)
    }

    fn input_application(&mut self) -> Option<&mut dyn qa_world::movement::q2::types::Q2InputApplication> {
        None
    }
}

/// Q3 service adapter over the player host.
struct PlayerQ3Services {
    host: Rc<dyn PlayerMovementHost>,
    numeric: NumericOps,
    slice: Option<PlayerSliceApplication>,
}

impl qa_world::movement::q3::types::Q3MovementServices for PlayerQ3Services {
    fn numeric(&self) -> NumericOps {
        self.numeric
    }

    fn trace(&mut self, query: qa_world::movement::q3::types::Q3TraceQuery) -> qa_world::movement::q3::types::Q3Trace {
        self.host.scene().borrow_mut().trace_q3(query)
    }

    fn point_contents(&mut self, point: Vec3, pass_actor: &ActorId) -> i32 {
        self.host.scene().borrow_mut().point_contents_q3(point, pass_actor)
    }

    fn input_application(&mut self) -> Option<&mut dyn qa_world::movement::q3::types::Q3InputApplication> {
        None
    }
}

/// Q3 hook adapter closing over the player.
struct PlayerQ3Hooks {
    player: PlayerHookTarget,
}

impl qa_world::movement::q3::types::Q3MovementHooks for PlayerQ3Hooks {
    fn firing(&mut self, context: &qa_world::movement::q3::types::Q3HookContext) -> bool {
        self.player.host.q3_hooks().borrow_mut().firing(context)
    }

    fn animation(
        &mut self,
        request: qa_world::movement::q3::types::Q3AnimationRequest,
        context: &qa_world::movement::q3::types::Q3HookContext,
    ) -> qa_world::movement::q3::types::Q3AnimationStepResult {
        self.player.host.q3_hooks().borrow_mut().animation(request, context)
    }

    fn weapon(
        &mut self,
        context: &qa_world::movement::q3::types::Q3HookContext,
    ) -> qa_world::movement::q3::types::Q3WeaponPhaseResult {
        self.player.core.borrow_mut().view_angles = context.motion.viewangles;
        self.player.host.q3_hooks().borrow_mut().weapon(context)
    }

    fn torso(
        &mut self,
        context: &qa_world::movement::q3::types::Q3HookContext,
    ) -> qa_world::movement::q3::types::Q3AnimationStepResult {
        self.player.host.q3_hooks().borrow_mut().torso(context)
    }
}

/// Player move result across dialects.
#[derive(Debug, Clone)]
pub enum PlayerMoveResult {
    /// NetQuake result.
    Q1Netquake(qa_world::movement::q1::types::Q1MovementResult),
    /// QuakeWorld result.
    Q1Quakeworld(qa_world::movement::q1::types::QwMovementResult),
    /// Quake II classic result.
    Q2Classic(qa_world::movement::q2::types::Q2MovementResult),
    /// Quake II rerelease result.
    Q2Rerelease(qa_world::movement::q2::types::Q2RereleaseMovementResult),
    /// Quake III result.
    Q3(qa_world::movement::q3::types::Q3MovementResult),
}

impl PlayerMoveResult {
    /// Result dialect.
    #[must_use]
    pub fn dialect(&self) -> MovementDialect {
        match self {
            Self::Q1Netquake(_) => MovementDialect::Q1Netquake,
            Self::Q1Quakeworld(_) => MovementDialect::Q1Quakeworld,
            Self::Q2Classic(_) => MovementDialect::Q2Classic,
            Self::Q2Rerelease(_) => MovementDialect::Q2Rerelease,
            Self::Q3(_) => MovementDialect::Q3,
        }
    }

    /// Whether the actor was removed.
    #[must_use]
    pub fn is_removed(&self) -> bool {
        match self {
            Self::Q1Netquake(result) => result.removed(),
            Self::Q1Quakeworld(result) => result.removed(),
            Self::Q2Classic(result) => result.removed(),
            Self::Q2Rerelease(result) => result.removed(),
            Self::Q3(result) => result.removed(),
        }
    }
}

impl MovementPlayer {
    /// Accept an arsenal intent.
    fn accept_arsenal_intent(&self, intent: Option<qa_net::common::commands::ArsenalIntent>) {
        let mut core = self.core.borrow_mut();
        let pending = core.arsenal_intent.clone();
        if let Some(pending) = pending {
            if pending.weapon.is_some()
                && (intent.is_none() || intent.as_ref().is_some_and(|next| next.provider == pending.provider))
            {
                core.arsenal_intent = if intent.is_none() {
                    Some(qa_net::common::commands::ArsenalIntent {
                        weapon: None,
                        use_holdable: false,
                        ..pending
                    })
                } else {
                    intent
                };
                return;
            }
        }
        core.arsenal_intent = intent;
    }

    /// Receive a NetQuake client command.
    pub fn receive_net_quake(&self, input: &ActorCommand) {
        let NetUserCommand::Q1Netquake {
            buttons,
            impulse,
            view_angles,
            ..
        } = &input.command
        else {
            panic!("NetQuake client requires a NetQuake command");
        };
        if (input.sequence as i64) <= self.last_sequence() {
            return;
        }
        let world = net_to_world_q1(&input.command);
        let mut core = self.core.borrow_mut();
        let merged_impulse = (*impulse as i32) | core.net_quake_command.as_ref().map_or(0, |cmd| cmd.impulse);
        let mut command = world;
        command.impulse = merged_impulse;
        core.net_quake_command = Some(command);
        core.previous_buttons = core.buttons;
        core.buttons = *buttons as i32;
        core.command_angles = qa_core::math::vec3(view_angles[0] as f32, view_angles[1] as f32, view_angles[2] as f32);
        core.view_angles = core.command_angles;
        core.last_sequence = input.sequence as i64;
        let pending = core.arsenal_intent.clone();
        if pending.is_some() {
            core.arsenal_intent = input.arsenal.clone().or(pending);
        } else {
            core.arsenal_intent = input.arsenal.clone();
        }
    }

    /// Q1 hook target.
    fn hook_target(&self) -> PlayerHookTarget {
        PlayerHookTarget {
            actor: self.actor.clone(),
            client: self.client.clone(),
            host: self.host.clone(),
            numeric: self.numeric,
            core: self.core.clone(),
        }
    }

    /// NetQuake movement options.
    fn net_quake_options(&self, think_netquake: bool, source_jump: bool) -> Q1MovementOptions<PlayerQ1Hooks> {
        let authority = self.host.net_quake().map(|binding| match binding.jump_authority() {
            NetQuakeJumpAuthority::SelectedMovement => qa_world::movement::q1::types::Q1JumpAuthority::SelectedMovement,
            NetQuakeJumpAuthority::SourceGamecode => qa_world::movement::q1::types::Q1JumpAuthority::SourceGamecode,
        });
        Q1MovementOptions {
            source_punch_angles: self.host.source_punch(self.actor.id()),
            view_height: Some(self.view_height()),
            max_velocity: None,
            no_step: false,
            ideal_pitch_scale: None,
            roll_speed: None,
            roll_angle: None,
            solid: None,
            jump_authority: authority.unwrap_or(qa_world::movement::q1::types::Q1JumpAuthority::SelectedMovement),
            fix_angle_roll: qa_world::movement::q1::types::Q1FixAngleRoll::default(),
            hooks: Some(PlayerQ1Hooks {
                player: self.hook_target(),
                think_netquake,
                source_jump,
            }),
        }
    }

    /// Build NetQuake movement input.
    fn net_quake_input(&self, frame: &FrameContext) -> qa_world::movement::q1::types::Q1MovementInput {
        let fixed = self.fixed_pose_update();
        let state = self.read_state();
        let profile = selected_movement_profile(self);
        let health = self.host.combat().health(self.actor.id()).expect("combat");
        let (MovementState::Q1Netquake(state), MovementProfile::Q1Netquake(profile)) = (state, profile) else {
            panic!("Missing NetQuake movement state");
        };
        let command = self.core.borrow().net_quake_command.unwrap_or_else(|| {
            let angles = self.view_angles();
            qa_world::movement::types::Q1UserCommand {
                acknowledged_server_time_seconds: 0.0,
                view_angles: angles,
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0,
                impulse: 0,
            }
        });
        let mut environment = player_movement_environment(
            self.source_environment(),
            self.gravity_multiplier(),
            &MovementState::Q1Netquake(state.clone()),
            self.flight() && health > 0.0,
            health,
            self.host.combat().invulnerable(self.actor.id()),
            self.movement_speed_multiplier(),
        );
        environment.pose = fixed;
        environment.client_outputs = Some(self.client_movement_outputs());
        qa_world::movement::q1::types::Q1MovementInput {
            fields: qa_world::movement::types::MovementInputFields {
                actor: self.actor.clone(),
                command_sequence: self.last_sequence() as i32,
                frame: *frame,
                shape: TraceShape::Box(self.bounds()),
                current_bounds: Some(self.bounds()),
                environment,
                arsenal: self.arsenal(),
                animation: self.animation(),
                execution: qa_world::movement::types::MovementExecution::Authoritative,
            },
            command,
            state,
            profile,
        }
    }
}

/// Convert a NetQuake net command to its world twin.
fn net_to_world_q1(command: &NetUserCommand) -> qa_world::movement::types::Q1UserCommand {
    let NetUserCommand::Q1Netquake {
        acknowledged_server_time_seconds,
        view_angles,
        forward_move,
        side_move,
        up_move,
        buttons,
        impulse,
    } = command
    else {
        panic!("NetQuake client requires a NetQuake command");
    };
    qa_world::movement::types::Q1UserCommand {
        acknowledged_server_time_seconds: *acknowledged_server_time_seconds,
        view_angles: qa_core::math::vec3(view_angles[0] as f32, view_angles[1] as f32, view_angles[2] as f32),
        forward_move: *forward_move,
        side_move: *side_move,
        up_move: *up_move,
        buttons: *buttons as i32,
        impulse: *impulse as i32,
    }
}

impl MovementPlayer {
    /// Prepare NetQuake movement for a frame.
    pub fn prepare_net_quake(&self, frame: &FrameContext) {
        let input = self.net_quake_input(frame);
        let actor = input.fields.actor.id().clone();
        let binding = self.host.net_quake();
        let mut services = self.q1_services();
        let think = self.read_state();
        let jumped = matches!(think, MovementState::Q1Netquake(_));
        let _ = jumped;
        let prepared = qa_world::movement::q1::netquake::prepare_netquake(
            input,
            &mut services,
            self.net_quake_options(true, false),
        )
        .expect("netquake prepare");
        if let Some(binding) = binding {
            binding.input(&self.core.borrow().net_quake_command.expect("received command"));
        }
        if self
            .commit(MovementState::Q1Netquake(prepared), true, false)
            .is_removed_player()
        {
            return;
        }
        let _ = actor;
    }

    /// Run NetQuake physics for a frame.
    pub fn physics_net_quake(&self, frame: &FrameContext, source_jump: bool) -> PlayerMoveResult {
        let input = self.net_quake_input(frame);
        let mut services = self.q1_services();
        let output = qa_world::movement::q1::netquake::physics_netquake(
            input,
            &mut services,
            self.net_quake_options(true, source_jump),
        )
        .expect("netquake physics");
        let result = PlayerMoveResult::Q1Netquake(output);
        self.accept(&result);
        result
    }

    /// Accept a move result into the core.
    fn accept(&self, result: &PlayerMoveResult) {
        if result.is_removed() {
            return;
        }
        let mut core = self.core.borrow_mut();
        core.last_weapon_seconds = f64::NEG_INFINITY;
        match result {
            PlayerMoveResult::Q1Netquake(qa_world::movement::types::MovementOutcome::Active { fields, state }) => {
                core.state = MovementState::Q1Netquake(state.clone());
                core.view_angles = fields.view_angles;
                core.view_height = fields.view_height;
                core.bounds = fields.bounds;
                core.ground = fields.ground.clone();
                core.water_level = fields.water_level;
                core.water_type = fields.water_type;
                core.arsenal = fields.arsenal.clone();
                core.animation = fields.animation.clone();
                if core.net_quake_command.is_some() {
                    core.net_quake_command = None;
                }
            }
            PlayerMoveResult::Q1Quakeworld(qa_world::movement::types::MovementOutcome::Active { fields, state }) => {
                core.state = MovementState::Q1Quakeworld(state.clone());
                core.view_angles = fields.view_angles;
                core.view_height = fields.view_height;
                core.bounds = fields.bounds;
                core.ground = fields.ground.clone();
                core.water_level = fields.water_level;
                core.water_type = fields.water_type;
                core.arsenal = fields.arsenal.clone();
                core.animation = fields.animation.clone();
            }
            PlayerMoveResult::Q2Classic(qa_world::movement::types::MovementOutcome::Active { fields, state }) => {
                core.state = MovementState::Q2Classic(*state);
                core.view_angles = fields.view_angles;
                core.view_height = fields.view_height;
                core.bounds = fields.bounds;
                core.ground = fields.ground.clone();
                core.water_level = fields.water_level;
                core.water_type = fields.water_type;
                core.arsenal = fields.arsenal.clone();
                core.animation = fields.animation.clone();
            }
            PlayerMoveResult::Q2Rerelease(qa_world::movement::q2::types::Q2RereleaseMovementResult::Active {
                fields,
                state,
                ..
            }) => {
                core.state = MovementState::Q2Rerelease(*state);
                core.view_angles = fields.view_angles;
                core.view_height = fields.view_height;
                core.bounds = fields.bounds;
                core.ground = fields.ground.clone();
                core.water_level = fields.water_level;
                core.water_type = fields.water_type;
                core.arsenal = fields.arsenal.clone();
                core.animation = fields.animation.clone();
            }
            PlayerMoveResult::Q3(qa_world::movement::types::MovementOutcome::Active { fields, state }) => {
                core.state = MovementState::Q3(state.clone());
                core.view_angles = fields.view_angles;
                core.view_height = fields.view_height;
                core.bounds = fields.bounds;
                core.ground = fields.ground.clone();
                core.water_level = fields.water_level;
                core.water_type = fields.water_type;
                core.arsenal = fields.arsenal.clone();
                core.animation = fields.animation.clone();
            }
            _ => {}
        }
    }

    /// Begin an input application slice.
    fn begin_input_application(
        &self,
        command: &WorldUserCommand,
        arsenal: Option<qa_net::common::commands::ArsenalIntent>,
        frame: &FrameContext,
        health: f64,
    ) -> Vec<PlayerInputApplication> {
        let applications = self.host.input_applications();
        let Some(manager) = applications else {
            return Vec::new();
        };
        if !manager.active() {
            return Vec::new();
        }
        let numeric = self.numeric;
        let state = self.read_state();
        let health_copy = health;
        let aim = move |absolute: Vec3, current: &WorldUserCommand| {
            let current_aim =
                super::player_input_application::movement_application_aim(current, &state, health_copy, &numeric)
                    .unwrap_or(absolute);
            let _ = current_aim;
            apply_absolute_aim(current, &state, absolute)
        };
        let accepted = self.host.accepted_input();
        let first = manager.begin(
            self.actor.id(),
            &self.client,
            PlayerInputScope::MovementSlice,
            command,
            PlayerAngleSpace::Absolute,
            absolute_aim(command, &self.read_state()),
            frame,
            accepted.clone(),
            arsenal.clone(),
            None,
            &aim,
        );
        let mut applied = Vec::new();
        if let Some(first) = first {
            if let Some(second) = manager.begin(
                self.actor.id(),
                &self.client,
                PlayerInputScope::ClientCommand,
                &first.command,
                PlayerAngleSpace::SourceRelative,
                absolute_aim(&first.command, &self.read_state()),
                frame,
                accepted,
                first.arsenal.clone(),
                Some(first.invocation),
                &aim,
            ) {
                applied.push(first);
                applied.push(second);
            }
        }
        applied
    }

    /// Finish input applications.
    fn finish_input_applications(&self, applied: &[PlayerInputApplication], result: &PlayerMoveResult) {
        let Some(manager) = self.host.input_applications() else {
            return;
        };
        let failed = result.is_removed();
        for application in applied {
            manager.finish(Some(application), failed);
        }
    }

    /// Run one command through the selected provider.
    pub fn move_command(&self, command: WorldUserCommand, frame: &FrameContext) -> PlayerMoveResult {
        let state = self.read_state();
        wrong_family(&command, &state);
        let health = self.host.combat().health(self.actor.id()).expect("combat");
        let fixed = self.fixed_pose_update();
        let applied = self.begin_input_application(&command, self.core.borrow().arsenal_intent.clone(), frame, health);
        let (command, _arsenal) = applied
            .last()
            .map(|application| (application.command, application.arsenal.clone()))
            .unwrap_or((command, None));
        let mut environment = player_movement_environment(
            self.source_environment(),
            self.gravity_multiplier(),
            &state,
            self.flight() && health > 0.0,
            health,
            self.host.combat().invulnerable(self.actor.id()),
            self.movement_speed_multiplier(),
        );
        environment.pose = fixed;
        environment.client_outputs = Some(self.client_movement_outputs());
        let source_movement = self.host.source_client().map(|source| {
            let qw_command = match &command {
                WorldUserCommand::Q1Quakeworld(command) => Some(*command),
                _ => None,
            };
            source.before_movement(
                &PlayerMovementCommand {
                    actor: self.actor.clone(),
                    command,
                    frame: *frame,
                },
                qw_command.as_ref(),
            )
        });
        self.core.borrow_mut().source_movement = source_movement;
        let result = self.dispatch_move(command, state, environment, frame);
        let output = self.after_move(&command, result, frame);
        self.finish_input_applications(&applied, &output);
        output
    }

    /// Dispatch to the dialect provider.
    fn dispatch_move(
        &self,
        command: WorldUserCommand,
        state: MovementState,
        environment: qa_world::movement::types::MovementEnvironment,
        frame: &FrameContext,
    ) -> PlayerMoveResult {
        let profile = selected_movement_profile(self);
        let fields = qa_world::movement::types::MovementInputFields {
            actor: self.actor.clone(),
            command_sequence: self.last_sequence() as i32,
            frame: *frame,
            shape: TraceShape::Box(self.bounds()),
            current_bounds: Some(self.bounds()),
            environment,
            arsenal: self.arsenal(),
            animation: self.animation(),
            execution: qa_world::movement::types::MovementExecution::Authoritative,
        };
        match (command, state, profile) {
            (
                WorldUserCommand::Q1Netquake(command),
                MovementState::Q1Netquake(state),
                MovementProfile::Q1Netquake(profile),
            ) => {
                let slice = self.open_slice(
                    &WorldUserCommand::Q1Netquake(command),
                    frame,
                    &MovementState::Q1Netquake(state.clone()),
                );
                let mut services = self.q1_services();
                services.slice = slice;
                let output = qa_world::movement::q1::netquake::physics_netquake(
                    qa_world::movement::q1::types::Q1MovementInput {
                        fields,
                        command,
                        state,
                        profile,
                    },
                    &mut services,
                    self.net_quake_options(false, false),
                )
                .expect("netquake move");
                PlayerMoveResult::Q1Netquake(output)
            }
            (
                WorldUserCommand::Q1Quakeworld(command),
                MovementState::Q1Quakeworld(state),
                MovementProfile::Q1Quakeworld(profile),
            ) => {
                let slice = self.open_slice(
                    &WorldUserCommand::Q1Quakeworld(command),
                    frame,
                    &MovementState::Q1Quakeworld(state.clone()),
                );
                let mut services = self.q1_services();
                services.slice = slice;
                let options = self.net_quake_options(false, false);
                let output = if self.host.quake_world().is_some() {
                    qa_world::movement::q1::quakeworld::create_qw_movement_provider(
                        self.recipe.movement.provider.clone(),
                        options,
                    )
                    .move_step(
                        qa_world::movement::q1::types::QwMovementInput {
                            fields,
                            command,
                            state,
                            profile,
                        },
                        &mut services,
                    )
                    .expect("qw move")
                } else {
                    super::shared_quakeworld_movement::create_shared_quake_world_movement(
                        self.recipe.movement.provider.clone(),
                        options,
                        self.standing_bounds,
                        &default_postures(self.character, self.view_height()),
                        None::<fn(Bounds, f64)>,
                    )
                    .move_step(
                        qa_world::movement::q1::types::QwMovementInput {
                            fields,
                            command,
                            state,
                            profile,
                        },
                        &mut services,
                    )
                    .expect("shared qw move")
                };
                PlayerMoveResult::Q1Quakeworld(output)
            }
            (
                WorldUserCommand::Q2Classic(command),
                MovementState::Q2Classic(state),
                MovementProfile::Q2Classic(profile),
            ) => {
                let slice = self.open_slice(
                    &WorldUserCommand::Q2Classic(command),
                    frame,
                    &MovementState::Q2Classic(state),
                );
                let mut services = self.q2_services();
                services.slice = slice;
                let input = qa_world::movement::q2::types::Q2MovementInput {
                    fields,
                    command,
                    state,
                    profile,
                };
                let output = qa_world::movement::q2::move_q2_classic(input.clone(), &mut services).expect("q2 move");
                let output = qa_world::movement::q2::apply_q2_movement_contacts(&input, &mut services, output)
                    .expect("q2 contacts");
                PlayerMoveResult::Q2Classic(output)
            }
            (
                WorldUserCommand::Q2Rerelease(command),
                MovementState::Q2Rerelease(state),
                MovementProfile::Q2Rerelease(profile),
            ) => {
                let context = self.host.rerelease_movement();
                let slice = self.open_slice(
                    &WorldUserCommand::Q2Rerelease(command),
                    frame,
                    &MovementState::Q2Rerelease(state),
                );
                let mut services = self.q2_services();
                services.slice = slice;
                let output = qa_world::movement::q2::move_q2_rerelease(
                    qa_world::movement::q2::types::Q2RereleaseMovementInput {
                        fields,
                        command,
                        state,
                        profile,
                        view_offset: qa_core::math::vec3(0.0, 0.0, self.view_height() as f32),
                        snap_initial: false,
                    },
                    &mut services,
                    &mut context.borrow_mut(),
                )
                .expect("q2r move");
                PlayerMoveResult::Q2Rerelease(output)
            }
            (WorldUserCommand::Q3(command), MovementState::Q3(state), MovementProfile::Q3(profile)) => {
                let slice = self.open_slice(&WorldUserCommand::Q3(command), frame, &MovementState::Q3(state.clone()));
                let mut services = self.q3_services();
                services.slice = slice;
                let output = qa_world::movement::q3::provider::move_q3(
                    qa_world::movement::q3::types::Q3MovementInput {
                        fields,
                        command,
                        state,
                        profile,
                    },
                    &mut services,
                    qa_world::movement::q3::provider::Q3MovementProviderOptions {
                        id: self.recipe.movement.provider.clone(),
                        hooks: PlayerQ3Hooks {
                            player: self.hook_target(),
                        },
                        postures: std::rc::Rc::new(|_| qa_world::movement::q3::postures::Q3_SOURCE_POSTURES),
                        trace_policy: None,
                        diagnostics: None,
                    },
                )
                .expect("q3 move");
                PlayerMoveResult::Q3(output)
            }
            _ => panic!("Command and movement state belong to different providers"),
        }
    }

    /// Post-move commit, events, and source hooks.
    fn after_move(
        &self,
        command: &WorldUserCommand,
        result: PlayerMoveResult,
        frame: &FrameContext,
    ) -> PlayerMoveResult {
        let state = match &result {
            PlayerMoveResult::Q1Netquake(qa_world::movement::types::MovementOutcome::Active { state, .. }) => {
                Some(MovementState::Q1Netquake(state.clone()))
            }
            PlayerMoveResult::Q1Quakeworld(qa_world::movement::types::MovementOutcome::Active { state, .. }) => {
                Some(MovementState::Q1Quakeworld(state.clone()))
            }
            PlayerMoveResult::Q2Classic(qa_world::movement::types::MovementOutcome::Active { state, .. }) => {
                Some(MovementState::Q2Classic(*state))
            }
            PlayerMoveResult::Q2Rerelease(qa_world::movement::q2::types::Q2RereleaseMovementResult::Active {
                state,
                ..
            }) => Some(MovementState::Q2Rerelease(*state)),
            PlayerMoveResult::Q3(qa_world::movement::types::MovementOutcome::Active { state, .. }) => {
                Some(MovementState::Q3(state.clone()))
            }
            _ => None,
        };
        if let Some(state) = state {
            let _ = self.commit(state, true, true);
        }
        let jump_command = self.jump_command(command, &result);
        if let Some(jump) = jump_command {
            let jumped = movement_jumped(jump.old_buttons, &self.ground(), &jump.command, Some(jump.result));
            let _ = jumped;
        }
        let _ = frame;
        self.accept(&result);
        if let Some(source) = self.host.source_client() {
            source.after_movement(self, frame, None);
        }
        result
    }
}

/// Jump evidence for the character animation.
struct PlayerJumpEvidence {
    old_buttons: Option<i32>,
    command: WorldUserCommand,
    result: super::player_jump::MovementJumpResult,
}

impl MovementPlayer {
    /// Build jump evidence for a command and result.
    fn jump_command(&self, command: &WorldUserCommand, result: &PlayerMoveResult) -> Option<PlayerJumpEvidence> {
        let state = self.state();
        match (command, &state, result) {
            (WorldUserCommand::Q1Netquake(command), MovementState::Q1Netquake(_), PlayerMoveResult::Q1Netquake(_)) => {
                Some(PlayerJumpEvidence {
                    old_buttons: None,
                    command: WorldUserCommand::Q1Netquake(*command),
                    result: super::player_jump::MovementJumpResult::Q1Netquake,
                })
            }
            (
                WorldUserCommand::Q1Quakeworld(command),
                MovementState::Q1Quakeworld(before),
                PlayerMoveResult::Q1Quakeworld(qa_world::movement::types::MovementOutcome::Active { state, .. }),
            ) => Some(PlayerJumpEvidence {
                old_buttons: Some(before.old_buttons),
                command: WorldUserCommand::Q1Quakeworld(*command),
                result: super::player_jump::MovementJumpResult::Q1Quakeworld {
                    old_buttons: state.old_buttons,
                    velocity_z: state.velocity.z,
                    dead: state.dead,
                },
            }),
            (
                WorldUserCommand::Q2Classic(_),
                MovementState::Q2Classic(_),
                PlayerMoveResult::Q2Classic(qa_world::movement::types::MovementOutcome::Active { fields, .. }),
            ) => Some(PlayerJumpEvidence {
                old_buttons: None,
                command: *command,
                result: super::player_jump::MovementJumpResult::Q2Classic {
                    ground: fields.ground.clone(),
                    water_level: fields.water_level,
                },
            }),
            (
                WorldUserCommand::Q2Rerelease(_),
                MovementState::Q2Rerelease(_),
                PlayerMoveResult::Q2Rerelease(qa_world::movement::q2::types::Q2RereleaseMovementResult::Active {
                    fields,
                    state,
                    ..
                }),
            ) => Some(PlayerJumpEvidence {
                old_buttons: None,
                command: *command,
                result: super::player_jump::MovementJumpResult::Q2Rerelease {
                    jump_sound: fields
                        .effects
                        .iter()
                        .any(|effect| matches!(effect.effect, qa_world::movement::types::MovementEffect::Event(_))),
                    flags: state.flags,
                },
            }),
            (
                WorldUserCommand::Q3(_),
                MovementState::Q3(_),
                PlayerMoveResult::Q3(qa_world::movement::types::MovementOutcome::Active { fields, .. }),
            ) => Some(PlayerJumpEvidence {
                old_buttons: None,
                command: *command,
                result: super::player_jump::MovementJumpResult::Q3 {
                    effects: fields.effects.iter().map(|effect| effect.effect.clone()).collect(),
                },
            }),
            _ => None,
        }
    }
}

/// Whether a continuation removed the player.
trait ContinuationRemoved {
    /// True when removed.
    fn is_removed_player(&self) -> bool;
}

impl ContinuationRemoved for PlayerContinuation {
    fn is_removed_player(&self) -> bool {
        matches!(self, Self::ActorRemoved)
    }
}

/// Rewrite a command to an absolute aim.
fn apply_absolute_aim(command: &WorldUserCommand, state: &MovementState, absolute: Vec3) -> WorldUserCommand {
    match (command, state) {
        (WorldUserCommand::Q1Netquake(command), _) => {
            let mut next = *command;
            next.view_angles = absolute;
            WorldUserCommand::Q1Netquake(next)
        }
        (WorldUserCommand::Q1Quakeworld(command), _) => {
            let mut next = *command;
            next.angles = absolute;
            WorldUserCommand::Q1Quakeworld(next)
        }
        (WorldUserCommand::Q2Classic(command), MovementState::Q2Classic(state)) => {
            let mut next = *command;
            for axis in 0..3 {
                let target = [absolute.x, absolute.y, absolute.z][axis] * 65536.0 / 360.0;
                next.angle_shorts[axis] = (target.trunc() as i32 - state.delta_angle_shorts[axis]) << 16 >> 16;
            }
            WorldUserCommand::Q2Classic(next)
        }
        (WorldUserCommand::Q2Rerelease(command), MovementState::Q2Rerelease(state)) => {
            let mut next = *command;
            next.angles = qa_core::math::vec3(
                absolute.x - state.delta_angles.x,
                absolute.y - state.delta_angles.y,
                absolute.z - state.delta_angles.z,
            );
            WorldUserCommand::Q2Rerelease(next)
        }
        (WorldUserCommand::Q3(command), MovementState::Q3(state)) => {
            let mut next = *command;
            for axis in 0..3 {
                let target = [absolute.x, absolute.y, absolute.z][axis] * 65536.0 / 360.0;
                next.angle_words[axis] = (target.trunc() as i32 - state.delta_angle_words[axis]) << 16 >> 16;
            }
            WorldUserCommand::Q3(next)
        }
        _ => *command,
    }
}

/// Absolute aim of a command and state.
fn absolute_aim(command: &WorldUserCommand, state: &MovementState) -> Vec3 {
    match (command, state) {
        (WorldUserCommand::Q1Netquake(command), MovementState::Q1Netquake(state)) => {
            if state.fix_angle {
                state.view_angles
            } else {
                command.view_angles
            }
        }
        (WorldUserCommand::Q1Quakeworld(command), _) => command.angles,
        (WorldUserCommand::Q2Classic(command), MovementState::Q2Classic(state)) => {
            let angle = |word: i32, delta: i32| (((word + delta) << 16 >> 16) as f32) * 360.0 / 65536.0;
            qa_core::math::vec3(
                angle(command.angle_shorts[0], state.delta_angle_shorts[0]),
                angle(command.angle_shorts[1], state.delta_angle_shorts[1]),
                angle(command.angle_shorts[2], state.delta_angle_shorts[2]),
            )
        }
        (WorldUserCommand::Q2Rerelease(command), MovementState::Q2Rerelease(state)) => qa_core::math::vec3(
            command.angles.x + state.delta_angles.x,
            command.angles.y + state.delta_angles.y,
            command.angles.z + state.delta_angles.z,
        ),
        (WorldUserCommand::Q3(command), MovementState::Q3(state)) => {
            let angle = |word: i32, delta: i32| (((word + delta) << 16 >> 16) as f32) * 360.0 / 65536.0;
            qa_core::math::vec3(
                angle(command.angle_words[0], state.delta_angle_words[0]),
                angle(command.angle_words[1], state.delta_angle_words[1]),
                angle(command.angle_words[2], state.delta_angle_words[2]),
            )
        }
        _ => qa_core::math::vec3(0.0, 0.0, 0.0),
    }
}

/// Panic when command and state belong to different providers.
fn wrong_family(command: &WorldUserCommand, state: &MovementState) {
    let same = matches!(
        (command, state),
        (WorldUserCommand::Q1Netquake(_), MovementState::Q1Netquake(_))
            | (WorldUserCommand::Q1Quakeworld(_), MovementState::Q1Quakeworld(_))
            | (WorldUserCommand::Q2Classic(_), MovementState::Q2Classic(_))
            | (WorldUserCommand::Q2Rerelease(_), MovementState::Q2Rerelease(_))
            | (WorldUserCommand::Q3(_), MovementState::Q3(_))
    );
    if !same {
        panic!("Command and movement state belong to different providers");
    }
}

/// Default postures for shared QuakeWorld movement.
fn default_postures(character: GameFamily, view_height: f64) -> qa_world::movement::q3::types::Q3Postures {
    if character == GameFamily::Q3 {
        qa_world::movement::q3::postures::Q3_SOURCE_POSTURES
    } else {
        qa_world::movement::q3::types::Q3Postures {
            standing_view_height: view_height,
            ..qa_world::movement::q3::postures::Q3_SOURCE_POSTURES
        }
    }
}

/// Input application slice over the manager.
struct PlayerSliceApplication {
    manager: Rc<dyn PlayerInputApplications>,
    application: Option<PlayerInputApplication>,
}

impl PlayerSliceApplication {
    /// Open a slice.
    fn open(
        target: PlayerHookTarget,
        manager: Rc<dyn PlayerInputApplications>,
        command: &WorldUserCommand,
        frame: &FrameContext,
        state: MovementState,
        arsenal: Option<qa_net::common::commands::ArsenalIntent>,
    ) -> Option<Self> {
        let numeric = target.numeric;
        let aim_state = state.clone();
        let health = target.host.combat().health(target.actor.id()).unwrap_or(0.0);
        let aim = move |absolute: Vec3, current: &WorldUserCommand| {
            let _ = super::player_input_application::movement_application_aim(current, &aim_state, health, &numeric);
            apply_absolute_aim(current, &aim_state, absolute)
        };
        let application = manager.begin(
            target.actor.id(),
            &target.client,
            PlayerInputScope::MovementSlice,
            command,
            PlayerAngleSpace::Absolute,
            absolute_aim(command, &state),
            frame,
            target.host.accepted_input(),
            arsenal.clone(),
            target.core.borrow().application_parent,
            &aim,
        );
        application.map(|application| Self {
            manager,
            application: Some(application),
        })
    }
}

impl qa_world::movement::q1::types::Q1InputApplication for PlayerSliceApplication {
    fn begin(
        &mut self,
        command: WorldUserCommand,
        frame: &FrameContext,
        state: qa_world::movement::q1::types::Q1State,
    ) -> qa_world::movement::types::MovementInputContinuation<qa_world::movement::q1::types::Q1State> {
        let _ = frame;
        qa_world::movement::types::MovementInputContinuation::Continue {
            state,
            command: self
                .application
                .as_ref()
                .map(|application| application.command)
                .unwrap_or(command),
        }
    }

    fn end(
        &mut self,
        state: qa_world::movement::q1::types::Q1State,
        failed: bool,
    ) -> qa_world::movement::types::MovementContinuation<qa_world::movement::q1::types::Q1State> {
        self.manager.finish(self.application.as_ref(), failed);
        qa_world::movement::types::MovementContinuation::Continue(state)
    }
}

impl qa_world::movement::q2::types::Q2InputApplication for PlayerSliceApplication {
    fn begin(
        &mut self,
        command: WorldUserCommand,
        frame: &FrameContext,
        state: qa_world::movement::q2::types::Q2State,
    ) -> qa_world::movement::types::MovementInputContinuation<qa_world::movement::q2::types::Q2State> {
        let _ = frame;
        qa_world::movement::types::MovementInputContinuation::Continue {
            state,
            command: self
                .application
                .as_ref()
                .map(|application| application.command)
                .unwrap_or(command),
        }
    }

    fn end(
        &mut self,
        state: qa_world::movement::q2::types::Q2State,
        failed: bool,
        _posture: Option<(Bounds, f64)>,
    ) -> qa_world::movement::types::MovementContinuation<qa_world::movement::q2::types::Q2State> {
        self.manager.finish(self.application.as_ref(), failed);
        qa_world::movement::types::MovementContinuation::Continue(state)
    }
}

impl qa_world::movement::q3::types::Q3InputApplication for PlayerSliceApplication {
    fn begin(
        &mut self,
        command: WorldUserCommand,
        frame: &FrameContext,
        state: qa_world::movement::q3::types::Q3MovementState,
    ) -> qa_world::movement::types::MovementInputContinuation<qa_world::movement::q3::types::Q3MovementState> {
        let _ = frame;
        qa_world::movement::types::MovementInputContinuation::Continue {
            state,
            command: self
                .application
                .as_ref()
                .map(|application| application.command)
                .unwrap_or(command),
        }
    }

    fn end(
        &mut self,
        state: qa_world::movement::q3::types::Q3MovementState,
        failed: bool,
    ) -> qa_world::movement::types::MovementContinuation<qa_world::movement::q3::types::Q3MovementState> {
        self.manager.finish(self.application.as_ref(), failed);
        qa_world::movement::types::MovementContinuation::Continue(state)
    }
}

impl MovementPlayer {
    /// Service bundle with an optional slice.
    fn q1_services(&self) -> PlayerQ1Services {
        PlayerQ1Services {
            host: self.host.clone(),
            numeric: self.numeric,
            slice: None,
        }
    }

    /// Service bundle with an optional slice.
    fn q2_services(&self) -> PlayerQ2Services {
        PlayerQ2Services {
            host: self.host.clone(),
            numeric: self.numeric,
            slice: None,
        }
    }

    /// Service bundle with an optional slice.
    fn q3_services(&self) -> PlayerQ3Services {
        PlayerQ3Services {
            host: self.host.clone(),
            numeric: self.numeric,
            slice: None,
        }
    }

    /// Open a movement slice for a command.
    fn open_slice(
        &self,
        command: &WorldUserCommand,
        frame: &FrameContext,
        state: &MovementState,
    ) -> Option<PlayerSliceApplication> {
        let manager = self.host.input_applications()?;
        if !manager.active() {
            return None;
        }
        PlayerSliceApplication::open(
            self.hook_target(),
            manager,
            command,
            frame,
            state.clone(),
            self.core.borrow().arsenal_intent.clone(),
        )
    }

    /// Finish NetQuake input applications.
    pub fn finish_net_quake_input(&self, failed: bool) {
        let mut core = self.core.borrow_mut();
        let mut failed = failed;
        if core.net_quake_slice_open {
            core.net_quake_slice_open = false;
            if let Some(manager) = self.host.input_applications() {
                manager.finish(None, failed);
            }
        }
        if let Some(manager) = self.host.input_applications() {
            manager.finish(core.net_quake_application.as_ref(), failed);
            let _ = manager;
        }
        core.net_quake_application = None;
        core.net_quake_slice_command = None;
        core.application_parent = None;
        let _ = &mut failed;
    }

    /// Run an actor command through the selected provider.
    pub fn move_input(
        &self,
        input: &ActorCommand,
        frame: &FrameContext,
        source_command: Option<&qa_world::movement::types::QwUserCommand>,
    ) -> PlayerMoveResult {
        if !self.host.input_applications().is_some_and(|manager| manager.active()) {
            return self.move_command_actor(input, frame, source_command.cloned());
        }
        self.accept_arsenal_intent(input.arsenal.clone());
        let world = net_to_world(&input.command);
        let result = self.with_input_command(
            world,
            frame,
            |command, arsenal| {
                let mut moved = input.clone();
                moved.command = world_to_net(command);
                if arsenal.is_some() {
                    moved.arsenal = arsenal;
                }
                self.move_command_actor(&moved, frame, source_command.cloned())
            },
            || removed_result(&self.profile, self.actor.id(), input.sequence),
            self.core.borrow().arsenal_intent.clone(),
        );
        self.refresh_input_result(result)
    }

    /// Run an operation under a client-command application.
    pub fn with_input_command<R>(
        &self,
        command: WorldUserCommand,
        frame: &FrameContext,
        operation: impl FnOnce(&WorldUserCommand, Option<qa_net::common::commands::ArsenalIntent>) -> R,
        removed: impl FnOnce() -> R,
        arsenal: Option<qa_net::common::commands::ArsenalIntent>,
    ) -> R {
        let manager = self.host.input_applications();
        let Some(manager) = manager else {
            return operation(&command, arsenal);
        };
        if !manager.active() {
            return operation(&command, arsenal);
        }
        let previous = self.core.borrow().application_parent;
        let state = self.read_state();
        let child_frame = super::player_input_application::movement_application_frame(
            &command,
            &state,
            &self.prediction_profile_player(),
            *frame,
        )
        .unwrap_or(*frame);
        let numeric = self.numeric;
        let aim_state = state.clone();
        let health = self.host.combat().health(self.actor.id()).unwrap_or(0.0);
        let aim = |absolute: Vec3, current: &WorldUserCommand| {
            let _ = super::player_input_application::movement_application_aim(current, &aim_state, health, &numeric);
            apply_absolute_aim(current, &aim_state, absolute)
        };
        let application = manager.begin(
            self.actor.id(),
            &self.client,
            PlayerInputScope::ClientCommand,
            &command,
            PlayerAngleSpace::SourceRelative,
            absolute_aim(&command, &state),
            &child_frame,
            self.host.accepted_input(),
            arsenal,
            previous,
            &aim,
        );
        self.core.borrow_mut().application_parent = application.as_ref().map(|application| application.invocation);
        let live = self.host.actors().is_live(self.actor.id());
        let output = if !live {
            manager.finish(application.as_ref(), true);
            self.core.borrow_mut().application_parent = previous;
            return removed();
        } else {
            let (command, arsenal) = application.as_ref().map_or((command, None), |application| {
                (application.command, application.arsenal.clone())
            });
            operation(&command, arsenal)
        };
        manager.finish(application.as_ref(), false);
        self.core.borrow_mut().application_parent = previous;
        output
    }

    /// Prediction profile for input applications.
    fn prediction_profile_player(&self) -> super::player_input_application::MovementProfile {
        self.prediction_profile()
    }

    /// Refresh a move result from the live body.
    fn refresh_input_result(&self, result: PlayerMoveResult) -> PlayerMoveResult {
        if result.is_removed() {
            return result;
        }
        let state = self.read_state();
        let body = self.host.bodies().read(self.actor.id()).expect("live body");
        {
            let mut core = self.core.borrow_mut();
            core.state = state.clone();
            core.bounds = body.hull;
        }
        let same = matches!(
            (&result, &state),
            (PlayerMoveResult::Q1Netquake(_), MovementState::Q1Netquake(_))
                | (PlayerMoveResult::Q1Quakeworld(_), MovementState::Q1Quakeworld(_))
                | (PlayerMoveResult::Q2Classic(_), MovementState::Q2Classic(_))
                | (PlayerMoveResult::Q2Rerelease(_), MovementState::Q2Rerelease(_))
                | (PlayerMoveResult::Q3(_), MovementState::Q3(_))
        );
        if same {
            self.accept(&result);
            result
        } else {
            panic!("Command and movement state belong to different providers");
        }
    }

    /// Run an actor command without the outer application.
    fn move_command_actor(
        &self,
        input: &ActorCommand,
        frame: &FrameContext,
        _source_command: Option<qa_world::movement::types::QwUserCommand>,
    ) -> PlayerMoveResult {
        if input.actor != *self.actor.id() {
            panic!("Movement command belongs to another actor");
        }
        self.core.borrow_mut().last_sequence = input.sequence as i64;
        self.move_command(net_to_world(&input.command), frame)
    }
}

/// Removed-actor result for a profile.
fn removed_result(profile: &MovementProfile, actor: &ActorId, sequence: u64) -> PlayerMoveResult {
    let sequence = sequence as i32;
    let actor = actor.clone();
    match profile {
        MovementProfile::Q1Netquake(_) => {
            PlayerMoveResult::Q1Netquake(qa_world::movement::types::MovementOutcome::ActorRemoved {
                actor,
                command_sequence: sequence,
                effects: Vec::new(),
            })
        }
        MovementProfile::Q1Quakeworld(_) => {
            PlayerMoveResult::Q1Quakeworld(qa_world::movement::types::MovementOutcome::ActorRemoved {
                actor,
                command_sequence: sequence,
                effects: Vec::new(),
            })
        }
        MovementProfile::Q2Classic(_) => {
            PlayerMoveResult::Q2Classic(qa_world::movement::types::MovementOutcome::ActorRemoved {
                actor,
                command_sequence: sequence,
                effects: Vec::new(),
            })
        }
        MovementProfile::Q2Rerelease(_) => {
            PlayerMoveResult::Q2Rerelease(qa_world::movement::q2::types::Q2RereleaseMovementResult::ActorRemoved {
                actor,
                command_sequence: sequence,
                effects: Vec::new(),
            })
        }
        MovementProfile::Q3(_) => PlayerMoveResult::Q3(qa_world::movement::types::MovementOutcome::ActorRemoved {
            actor,
            command_sequence: sequence,
            effects: Vec::new(),
        }),
    }
}

/// Convert a net command to its world twin.
fn net_to_world(command: &NetUserCommand) -> WorldUserCommand {
    match command {
        NetUserCommand::Q1Netquake { .. } => WorldUserCommand::Q1Netquake(net_to_world_q1(command)),
        NetUserCommand::Q1Quakeworld {
            milliseconds,
            angles,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
        } => WorldUserCommand::Q1Quakeworld(qa_world::movement::types::QwUserCommand {
            milliseconds: *milliseconds as i32,
            angles: qa_core::math::vec3(angles[0] as f32, angles[1] as f32, angles[2] as f32),
            forward_move: *forward_move,
            side_move: *side_move,
            up_move: *up_move,
            buttons: *buttons as i32,
            impulse: *impulse as i32,
        }),
        NetUserCommand::Q2Classic {
            milliseconds,
            angle_shorts,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
            light_level,
        } => WorldUserCommand::Q2Classic(qa_world::movement::types::Q2UserCommand {
            milliseconds: *milliseconds as i32,
            angle_shorts: [angle_shorts[0] as i32, angle_shorts[1] as i32, angle_shorts[2] as i32],
            forward_move: *forward_move,
            side_move: *side_move,
            up_move: *up_move,
            buttons: *buttons as i32,
            impulse: *impulse as i32,
            light_level: *light_level as i32,
        }),
        NetUserCommand::Q2Rerelease {
            milliseconds,
            angles,
            forward_move,
            side_move,
            buttons,
            server_frame,
        } => WorldUserCommand::Q2Rerelease(qa_world::movement::types::Q2RereleaseUserCommand {
            milliseconds: *milliseconds as i32,
            angles: qa_core::math::vec3(angles[0] as f32, angles[1] as f32, angles[2] as f32),
            forward_move: *forward_move,
            side_move: *side_move,
            buttons: *buttons as i32,
            server_frame: *server_frame as i32,
        }),
        NetUserCommand::Q3 {
            server_time_milliseconds,
            angle_words,
            buttons,
            weapon,
            forward_move,
            right_move,
            up_move,
        } => WorldUserCommand::Q3(qa_world::movement::types::Q3UserCommand {
            server_time_milliseconds: *server_time_milliseconds as i32,
            angle_words: [angle_words[0] as i32, angle_words[1] as i32, angle_words[2] as i32],
            buttons: *buttons as i32,
            weapon: *weapon as i32,
            forward_move: *forward_move as i32,
            right_move: *right_move as i32,
            up_move: *up_move as i32,
        }),
    }
}

/// Convert a world command to its net twin.
fn world_to_net(command: &WorldUserCommand) -> NetUserCommand {
    match command {
        WorldUserCommand::Q1Netquake(command) => NetUserCommand::Q1Netquake {
            acknowledged_server_time_seconds: command.acknowledged_server_time_seconds,
            view_angles: [
                command.view_angles.x as f64,
                command.view_angles.y as f64,
                command.view_angles.z as f64,
            ],
            forward_move: command.forward_move,
            side_move: command.side_move,
            up_move: command.up_move,
            buttons: command.buttons as f64,
            impulse: command.impulse as f64,
        },
        WorldUserCommand::Q1Quakeworld(command) => NetUserCommand::Q1Quakeworld {
            milliseconds: command.milliseconds as f64,
            angles: [
                command.angles.x as f64,
                command.angles.y as f64,
                command.angles.z as f64,
            ],
            forward_move: command.forward_move,
            side_move: command.side_move,
            up_move: command.up_move,
            buttons: command.buttons as f64,
            impulse: command.impulse as f64,
        },
        WorldUserCommand::Q2Classic(command) => NetUserCommand::Q2Classic {
            milliseconds: command.milliseconds as f64,
            angle_shorts: [
                command.angle_shorts[0] as f64,
                command.angle_shorts[1] as f64,
                command.angle_shorts[2] as f64,
            ],
            forward_move: command.forward_move,
            side_move: command.side_move,
            up_move: command.up_move,
            buttons: command.buttons as f64,
            impulse: command.impulse as f64,
            light_level: command.light_level as f64,
        },
        WorldUserCommand::Q2Rerelease(command) => NetUserCommand::Q2Rerelease {
            milliseconds: command.milliseconds as f64,
            angles: [
                command.angles.x as f64,
                command.angles.y as f64,
                command.angles.z as f64,
            ],
            forward_move: command.forward_move,
            side_move: command.side_move,
            buttons: command.buttons as f64,
            server_frame: command.server_frame as f64,
        },
        WorldUserCommand::Q3(command) => NetUserCommand::Q3 {
            server_time_milliseconds: command.server_time_milliseconds as f64,
            angle_words: [
                command.angle_words[0] as f64,
                command.angle_words[1] as f64,
                command.angle_words[2] as f64,
            ],
            buttons: command.buttons as f64,
            weapon: command.weapon as f64,
            forward_move: command.forward_move as f64,
            right_move: command.right_move as f64,
            up_move: command.up_move as f64,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use qa_core::identity::ProviderId;

    fn provider(namespace: &str, name: &str) -> ProviderId {
        ProviderId {
            namespace: namespace.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn provider_family_matches_namespace() {
        assert_eq!(provider_family(&provider("q1", "netquake")), GameFamily::Q1);
        assert_eq!(provider_family(&provider("q2", "classic")), GameFamily::Q2);
        assert_eq!(provider_family(&provider("q3", "arena")), GameFamily::Q3);
    }

    #[test]
    fn q3_standing_bounds_use_source() {
        let bounds = player_standing_bounds(GameFamily::Q3);
        assert_eq!(bounds.min.x, -15.0);
        assert_eq!(bounds.max.z, 32.0);
        let classic = player_standing_bounds(GameFamily::Q1);
        assert_eq!(classic.min.x, -16.0);
    }

    #[test]
    fn crouched_bounds_clamp_z() {
        let standing = player_standing_bounds(GameFamily::Q1);
        let crouched = player_crouched_bounds(GameFamily::Q1, standing);
        assert_eq!(crouched.min.z, -24.0);
        assert_eq!(crouched.max.z, 4.0);
    }

    #[test]
    fn origin_velocity_follow_state() {
        let state = MovementState::Q1Netquake(Q1MovementState {
            origin: qa_core::math::vec3(1.0, 2.0, 3.0),
            velocity: qa_core::math::vec3(4.0, 5.0, 6.0),
            angles: qa_core::math::vec3(0.0, 0.0, 0.0),
            old_origin: qa_core::math::vec3(0.0, 0.0, 0.0),
            angular_velocity: qa_core::math::vec3(0.0, 0.0, 0.0),
            view_angles: qa_core::math::vec3(0.0, 0.0, 0.0),
            punch_angles: qa_core::math::vec3(0.0, 0.0, 0.0),
            move_type: 3,
            flags: 0,
            ground: TraceHit::None,
            water_level: 0,
            water_type: -1,
            teleport_time_seconds: 0.0,
            water_jump_direction: qa_core::math::vec3(0.0, 0.0, 0.0),
            ideal_pitch: 0.0,
            fix_angle: false,
            health: 100.0,
        });
        assert_eq!(movement_origin(&state).x, 1.0);
        assert_eq!(movement_velocity(&state).z, 6.0);
    }

    #[test]
    fn fixed_quake_aim_wins() {
        let command = WorldUserCommand::Q1Netquake(qa_world::movement::types::Q1UserCommand {
            acknowledged_server_time_seconds: 0.0,
            view_angles: qa_core::math::vec3(0.0, 0.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        });
        let state = MovementState::Q1Netquake(Q1MovementState {
            origin: qa_core::math::vec3(0.0, 0.0, 0.0),
            velocity: qa_core::math::vec3(0.0, 0.0, 0.0),
            angles: qa_core::math::vec3(0.0, 0.0, 0.0),
            old_origin: qa_core::math::vec3(0.0, 0.0, 0.0),
            angular_velocity: qa_core::math::vec3(0.0, 0.0, 0.0),
            view_angles: qa_core::math::vec3(10.0, 20.0, 30.0),
            punch_angles: qa_core::math::vec3(0.0, 0.0, 0.0),
            move_type: 3,
            flags: 0,
            ground: TraceHit::None,
            water_level: 0,
            water_type: -1,
            teleport_time_seconds: 0.0,
            water_jump_direction: qa_core::math::vec3(0.0, 0.0, 0.0),
            ideal_pitch: 0.0,
            fix_angle: true,
            health: 100.0,
        });
        assert_eq!(absolute_aim(&command, &state).y, 20.0);
    }
}
