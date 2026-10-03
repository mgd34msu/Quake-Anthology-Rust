//! Player locomotion snapshots, movement provider assembly, and detached prediction.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/player-movement.ts`
//! (`LocomotionPlayer`, `capturePlayerLocomotion`, `playerLocomotionMatches`, `playerTracePolicy`,
//! `playerStandingBounds`, `playerPostures`, `locomotionTemplate`, `playerCrouchedBounds`,
//! `selectedMovementProfile`, `playerMovementEnvironment`, `PlayerMovementHooks`,
//! `createPlayerMovementProvider`, `MovementPredictionPlayer`, `PlayerMovementPrediction`,
//! `createPlayerMovementPrediction`, `movementObservation`).
//!
//! Missing siblings (no Rust home in this worktree; bridged below, unify post-merge):
//! the collision backends behind prediction traces (`qa_world::collision`
//! query bodies belong to the collision lane; services answer empty-world
//! until then, matching the `SharedSceneQueries` seam).

use std::cell::RefCell;
use std::rc::Rc;

use qa_bots::movement::NavigationPrediction;
use qa_bots::movement_contract::{
    MovementExecution as BotsExecution, MovementInput as BotsInput, MovementKind, MovementProfile as BotsProfile,
    MovementProvider as BotsProvider, MovementResult as BotsResult, MovementServices as BotsServices,
    MovementState as BotsState,
};
use qa_bots::scene::{LeafContents, Q1MoveRule, TracePolicy};
use qa_bots::types::{TravelMode, TraversalRequest};
use qa_content::contract::{ExecutableRecipe, GameFamily};
use qa_content::q3::base::shared::definitions::Product;
use qa_content::q3::foundation::arsenal::Q3ArsenalRuntimeState;
use qa_content::q3::foundation::movement_hooks::{
    create_q3_source_movement_hooks, Q3ArsenalRuntimeAccess, Q3Execution, Q3HookCommand,
    Q3HookContext as ContentHookContext, Q3HookInput, Q3MotionWork, Q3MovementHooks as ContentHooks,
    Q3SourceMovementHooks,
};
use qa_core::identity::{ActorId, ClientId, OwnedActor, ProviderId};
use qa_core::math::{Bounds, Plane, Vec3};
use qa_core::numeric::{NumericOps, NumericProfile};
use qa_core::time::{FrameContext, FramePhase, SourceTime};
use qa_world::collision::q1::CONTENTS_EMPTY;
use qa_world::hull::BspPlane;
use qa_world::movement::q1::netquake::{create_q1_movement_provider, Q1MovementProvider};
use qa_world::movement::q1::quakeworld::{create_qw_movement_provider, QwMovementProvider};
use qa_world::movement::q1::types::{
    NoQ1Hooks, Q1AnimationStepInput, Q1AnimationStepResult, Q1MovementInput, Q1MovementOptions, Q1MovementProfile,
    Q1MovementServices, Q1State, Q1Trace, Q1TraceQuery, Q1WeaponStepInput, Q1WeaponStepResult, QwMovementInput,
    QwMovementProfile,
};
use qa_world::movement::q2::rerelease::Q2RereleaseMovementContext;
use qa_world::movement::q2::types::{
    Q2ContentsQuery, Q2MovementInput, Q2MovementProfile, Q2MovementServices, Q2MovementState, Q2RereleaseMovementInput,
    Q2RereleaseMovementProfile, Q2RereleaseMovementResult, Q2State, Q2TouchContact, Q2Trace, Q2TracePlane,
    Q2TraceQuery,
};
use qa_world::movement::q2::{
    create_q2_classic_movement_provider, create_q2_rerelease_movement_provider, Q2ClassicMovementProvider,
    Q2RereleaseMovementProvider,
};
use qa_world::movement::q3::animation::{q3_source_animation, q3_source_torso};
use qa_world::movement::q3::constants::{entity_event, player_animation};
use qa_world::movement::q3::postures::{Q3_SOURCE_POSTURES, Q3_SOURCE_STANDING_BOUNDS};
use qa_world::movement::q3::provider::{
    create_q3_movement_provider, Q3MovementProvider, Q3MovementProviderOptions, Q3TracePolicy,
};
use qa_world::movement::q3::types::{
    Q3AnimationRequest, Q3AnimationStepResult, Q3HookContext as WorldHookContext, Q3MovementHooks as WorldHooks,
    Q3MovementInput, Q3MovementProfile, Q3MovementServices, Q3MovementState, Q3Postures, Q3Trace, Q3TraceQuery,
    Q3WeaponPhaseResult,
};
use qa_world::movement::types::{
    ActorAnimationState, AnimationState, ArsenalState, MovementContinuation, MovementEffect, MovementEnvironment,
    MovementExecution as WorldExecution, MovementInputFields, MovementOutcome, MovementTouchContact,
    OrderedMovementEffect, Q1UserCommand, Q2RereleaseUserCommand, Q2UserCommand, Q3UserCommand, QwUserCommand,
    TraceContact, TraceHit as WorldHit, TraceShape, UserCommand, WeaponState,
};
use qa_world::movement::Q1MovementParameters;

use super::player_input_application::{MovementProfile, MovementState};
use super::runtime::{
    movement_origin, movement_profile, provider_family, provider_text, ClientMovementOptions, MovementPlayer,
    Q2MovementConfig, SharedSimulation,
};
use super::shared_quakeworld_movement::{create_shared_quake_world_movement, SharedQuakeWorldMovement};
use super::types::QuakeCSourceKind;

/// Relocate a movement state to a detached prediction origin/velocity.
fn relocated(state: &MovementState, origin: Vec3, velocity: Vec3) -> MovementState {
    match state {
        MovementState::Q2Classic(inner) => MovementState::Q2Classic(Q2MovementState {
            origin_eighths: [
                f64::from(origin.x).mul_add(8.0, 0.0).trunc() as i32,
                f64::from(origin.y).mul_add(8.0, 0.0).trunc() as i32,
                f64::from(origin.z).mul_add(8.0, 0.0).trunc() as i32,
            ],
            velocity_eighths: [
                f64::from(velocity.x).mul_add(8.0, 0.0).trunc() as i32,
                f64::from(velocity.y).mul_add(8.0, 0.0).trunc() as i32,
                f64::from(velocity.z).mul_add(8.0, 0.0).trunc() as i32,
            ],
            ..*inner
        }),
        MovementState::Q1Netquake(inner) => {
            let mut next = inner.clone();
            next.origin = origin;
            next.velocity = velocity;
            MovementState::Q1Netquake(next)
        }
        MovementState::Q1Quakeworld(inner) => {
            let mut next = inner.clone();
            next.origin = origin;
            next.velocity = velocity;
            MovementState::Q1Quakeworld(next)
        }
        MovementState::Q2Rerelease(inner) => {
            let mut next = *inner;
            next.origin = origin;
            next.velocity = velocity;
            MovementState::Q2Rerelease(next)
        }
        MovementState::Q3(inner) => {
            let mut next = inner.clone();
            next.origin = origin;
            next.velocity = velocity;
            MovementState::Q3(next)
        }
    }
}

/// Locomotion slice of a movement player (donor `LocomotionPlayer`).
#[derive(Debug, Clone, PartialEq)]
pub struct LocomotionPlayer {
    /// Selected movement profile.
    pub profile: MovementProfile,
    /// Standing collision bounds.
    pub standing_bounds: Bounds,
    /// Source movement options, when a source client owns movement.
    pub source_movement: Option<ClientMovementOptions>,
    /// Character family.
    pub character: GameFamily,
    /// World gravity.
    pub world_gravity: f64,
    /// Host Q2 movement tuning.
    pub q2_movement_config: Option<Q2MovementConfig>,
    /// Flight flag.
    pub flight: bool,
}

impl From<&MovementPlayer> for LocomotionPlayer {
    fn from(player: &MovementPlayer) -> Self {
        Self {
            profile: player.profile.clone(),
            standing_bounds: player.standing_bounds,
            source_movement: player.source_movement.clone(),
            character: player.character,
            world_gravity: player.world_gravity,
            q2_movement_config: player.q2_movement_config,
            flight: player.flight,
        }
    }
}

/// Snapshot the locomotion slice (donor `capturePlayerLocomotion`).
pub fn capture_player_locomotion(player: &LocomotionPlayer) -> LocomotionPlayer {
    player.clone()
}

/// Whether two locomotion snapshots match on the compared fields.
pub fn player_locomotion_matches(player: &LocomotionPlayer, previous: &LocomotionPlayer) -> bool {
    player.flight == previous.flight
        && player.profile == previous.profile
        && player.character == previous.character
        && player.standing_bounds == previous.standing_bounds
        && player.world_gravity == previous.world_gravity
        && player.q2_movement_config.map(|config| config.air_accelerate)
            == previous.q2_movement_config.map(|config| config.air_accelerate)
        && player.q2_movement_config.map(|config| config.n64_physics)
            == previous.q2_movement_config.map(|config| config.n64_physics)
        && player.source_movement.as_ref().and_then(|source| source.trace_mask)
            == previous.source_movement.as_ref().and_then(|source| source.trace_mask)
        && player.source_movement.as_ref().and_then(|source| source.fixed_msec)
            == previous.source_movement.as_ref().and_then(|source| source.fixed_msec)
        && player.source_movement.as_ref().and_then(|source| source.no_footsteps)
            == previous.source_movement.as_ref().and_then(|source| source.no_footsteps)
}

/// Trace policy for a locomotion slice (donor `playerTracePolicy`).
pub fn player_trace_policy(player: &LocomotionPlayer) -> TracePolicy {
    match &player.profile {
        MovementProfile::Q1Netquake(_) | MovementProfile::Q1Quakeworld(_) => TracePolicy::Q1 {
            move_rule: Q1MoveRule::Normal,
            hull: None,
        },
        MovementProfile::Q2Classic(_) => TracePolicy::Q2 {
            contents_mask: 0x0201_0003,
            leaf_contents: LeafContents::Stored,
        },
        MovementProfile::Q2Rerelease(_) => TracePolicy::Q2 {
            contents_mask: 0x0201_0003,
            leaf_contents: LeafContents::Merged,
        },
        MovementProfile::Q3(_) => TracePolicy::Q3 {
            contents_mask: player
                .source_movement
                .as_ref()
                .and_then(|source| source.trace_mask)
                .unwrap_or(0x0201_0001),
            curves: true,
            player_curve_clip: true,
        },
    }
}

/// Standing bounds for a character family (donor `playerStandingBounds`).
pub fn player_standing_bounds(character: GameFamily) -> Bounds {
    if character == GameFamily::Q3 {
        Q3_SOURCE_STANDING_BOUNDS
    } else {
        Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 32.0,
            },
        }
    }
}
use qa_world::movement::q3::provider::{Q3PosturesFn, Q3TracePolicyFn};
use qa_world::movement::q3::types::Q3Posture;

/// Character postures for a player slice (donor `playerPostures`).
pub fn player_postures(character: GameFamily, standing_bounds: Bounds, view_height: f64) -> Q3Postures {
    if character == GameFamily::Q3 {
        Q3_SOURCE_POSTURES
    } else {
        Q3Postures {
            standing_view_height: view_height,
            crouched: Q3Posture {
                bounds: Bounds {
                    min: standing_bounds.min,
                    max: Vec3 {
                        x: standing_bounds.max.x,
                        y: standing_bounds.max.y,
                        z: 4.0,
                    },
                },
                view_height: -2.0,
            },
            dead: Q3Posture {
                bounds: Bounds {
                    min: standing_bounds.min,
                    max: Vec3 {
                        x: standing_bounds.max.x,
                        y: standing_bounds.max.y,
                        z: -8.0,
                    },
                },
                view_height: -16.0,
            },
            invulnerability_expanded: Q3_SOURCE_POSTURES.invulnerability_expanded,
        }
    }
}

/// Locomotion template for a recipe (donor `locomotionTemplate`).
pub fn locomotion_template(recipe: &ExecutableRecipe) -> LocomotionPlayer {
    let character = provider_family(&provider_text(&recipe.character.definition.provider))
        .expect("locomotion template character family");
    let profile = movement_profile(recipe).expect("locomotion template movement profile");
    LocomotionPlayer {
        standing_bounds: player_standing_bounds(character),
        profile,
        source_movement: None,
        character,
        world_gravity: 800.0,
        q2_movement_config: None,
        flight: false,
    }
}

/// Crouched bounds for a locomotion slice (donor `playerCrouchedBounds`).
pub fn player_crouched_bounds(player: &LocomotionPlayer) -> Bounds {
    player_postures(player.character, player.standing_bounds, 0.0)
        .crouched
        .bounds
}

/// Selected movement profile with host tuning applied (donor `selectedMovementProfile`).
pub fn selected_movement_profile(
    player: &LocomotionPlayer,
    profile_override: Option<&MovementProfile>,
) -> MovementProfile {
    let profile = profile_override.unwrap_or(&player.profile);
    match profile {
        MovementProfile::Q1Netquake(inner) => MovementProfile::Q1Netquake(Q1MovementProfile {
            parameters: Q1MovementParameters {
                gravity: player.world_gravity,
                ..inner.parameters
            },
            ..inner.clone()
        }),
        MovementProfile::Q1Quakeworld(inner) => MovementProfile::Q1Quakeworld(QwMovementProfile {
            parameters: Q1MovementParameters {
                gravity: player.world_gravity,
                ..inner.parameters
            },
            ..inner.clone()
        }),
        MovementProfile::Q3(inner) => match &player.source_movement {
            Some(source) => MovementProfile::Q3(Q3MovementProfile {
                fixed_milliseconds: source.fixed_msec,
                no_footsteps: source.no_footsteps.unwrap_or(inner.no_footsteps),
                ..inner.clone()
            }),
            None => profile.clone(),
        },
        MovementProfile::Q2Classic(inner) => match &player.q2_movement_config {
            Some(config) => MovementProfile::Q2Classic(Q2MovementProfile {
                air_accelerate: config.air_accelerate,
                ..inner.clone()
            }),
            None => profile.clone(),
        },
        MovementProfile::Q2Rerelease(inner) => match &player.q2_movement_config {
            Some(config) => MovementProfile::Q2Rerelease(Q2RereleaseMovementProfile {
                air_accelerate: config.air_accelerate,
                n64_physics: config.n64_physics,
                ..inner.clone()
            }),
            None => profile.clone(),
        },
    }
}

/// Player fields read by `player_movement_environment` (donor parameter picks).
pub trait MovementEnvironmentPlayer {
    /// Source environment override.
    fn source_environment(&self) -> &Option<MovementEnvironment>;
    /// Gravity multiplier.
    fn gravity_multiplier(&self) -> f64;
    /// Whether the selected state is Q3.
    fn state_is_q3(&self) -> bool;
    /// Flight flag.
    fn flight(&self) -> bool;
    /// Movement speed multiplier.
    fn movement_speed_multiplier(&self) -> f64;
}

impl MovementEnvironmentPlayer for MovementPlayer {
    fn source_environment(&self) -> &Option<MovementEnvironment> {
        &self.source_environment
    }

    fn gravity_multiplier(&self) -> f64 {
        self.gravity_multiplier
    }

    fn state_is_q3(&self) -> bool {
        matches!(self.state, MovementState::Q3(_))
    }

    fn flight(&self) -> bool {
        self.flight
    }

    fn movement_speed_multiplier(&self) -> f64 {
        self.movement_speed_multiplier
    }
}

/// Movement environment for a player and combat health (donor `playerMovementEnvironment`).
pub fn player_movement_environment(
    player: &impl MovementEnvironmentPlayer,
    health: f64,
    invulnerable: bool,
) -> MovementEnvironment {
    let speed_multiplier = if player.movement_speed_multiplier() == 1.0 {
        None
    } else {
        Some(player.movement_speed_multiplier())
    };
    match player.source_environment() {
        None => MovementEnvironment {
            speed_multiplier,
            health,
            flight: player.flight() && health > 0.0,
            haste: false,
            invulnerable,
            gravity_multiplier: if player.state_is_q3() {
                1.0
            } else {
                player.gravity_multiplier()
            },
            ..MovementEnvironment::default()
        },
        Some(source) => MovementEnvironment {
            speed_multiplier,
            health,
            flight: (source.flight || player.flight()) && health > 0.0,
            invulnerable: source.invulnerable || invulnerable,
            gravity_multiplier: if player.state_is_q3() {
                1.0
            } else {
                source.gravity_multiplier
            },
            ..*source
        },
    }
}

/// Movement provider hooks (donor `PlayerMovementHooks`).
pub struct PlayerMovementHooks<H> {
    /// Use the native QuakeWorld provider instead of the shared one.
    pub native_quake_world: bool,
    /// Posture publisher for the shared QuakeWorld provider.
    pub publish_posture: Option<Rc<dyn Fn(Bounds, f64)>>,
    /// Q1 movement options.
    pub q1: Q1MovementOptions<NoQ1Hooks>,
    /// Q2 rerelease movement context.
    pub q2: Q2RereleaseMovementContext,
    /// Q3 movement hooks.
    pub q3: H,
}

/// Selected player movement provider (donor `MovementProvider` union).
pub enum PlayerMovementProvider<H> {
    /// NetQuake provider.
    Q1Netquake(Q1MovementProvider<NoQ1Hooks>),
    /// Native QuakeWorld provider.
    Q1QuakeworldNative(QwMovementProvider<NoQ1Hooks>),
    /// Shared QuakeWorld provider.
    Q1QuakeworldShared(SharedQuakeWorldMovement<NoQ1Hooks>),
    /// Classic Q2 provider.
    Q2Classic(Q2ClassicMovementProvider),
    /// Rerelease Q2 provider (step borrows mutably through the handle).
    Q2Rerelease(RefCell<Q2RereleaseMovementProvider>),
    /// Q3 provider.
    Q3(Q3MovementProvider<H>),
}

/// Build the movement provider for a locomotion slice (donor `createPlayerMovementProvider`).
pub fn create_player_movement_provider<H: WorldHooks + Clone>(
    player: &LocomotionPlayer,
    view_height: f64,
    hooks: PlayerMovementHooks<H>,
) -> PlayerMovementProvider<H> {
    match &player.profile {
        MovementProfile::Q1Netquake(profile) => {
            PlayerMovementProvider::Q1Netquake(create_q1_movement_provider(profile.id.clone(), hooks.q1))
        }
        MovementProfile::Q1Quakeworld(profile) => {
            if hooks.native_quake_world {
                PlayerMovementProvider::Q1QuakeworldNative(create_qw_movement_provider(profile.id.clone(), hooks.q1))
            } else {
                let postures = player_postures(player.character, player.standing_bounds, 22.0);
                let publish = hooks
                    .publish_posture
                    .clone()
                    .map(|publish| move |bounds: Bounds, height: f64| publish(bounds, height));
                PlayerMovementProvider::Q1QuakeworldShared(create_shared_quake_world_movement(
                    profile.id.clone(),
                    hooks.q1,
                    player.standing_bounds,
                    &postures,
                    publish,
                ))
            }
        }
        MovementProfile::Q2Classic(profile) => {
            PlayerMovementProvider::Q2Classic(create_q2_classic_movement_provider(profile.id.clone()))
        }
        MovementProfile::Q2Rerelease(profile) => PlayerMovementProvider::Q2Rerelease(RefCell::new(
            create_q2_rerelease_movement_provider(profile.id.clone(), hooks.q2),
        )),
        MovementProfile::Q3(profile) => {
            let character = player.character;
            let standing_bounds = player.standing_bounds;
            let postures: Q3PosturesFn = Rc::new(move |_| player_postures(character, standing_bounds, view_height));
            let trace_policy: Option<Q3TracePolicyFn> = player.source_movement.as_ref().map(|source| {
                let contents_mask = source.trace_mask.unwrap_or(0x0201_0001);
                Rc::new(move |_: &Q3MovementInput| Q3TracePolicy {
                    curves: true,
                    player_curve_clip: true,
                    contents_mask,
                }) as Q3TracePolicyFn
            });
            PlayerMovementProvider::Q3(create_q3_movement_provider(Q3MovementProviderOptions {
                id: profile.id.clone(),
                hooks: hooks.q3,
                postures,
                trace_policy,
                diagnostics: None,
            }))
        }
    }
}

/// Prediction player snapshot (donor `MovementPredictionPlayer`).
#[derive(Debug, Clone)]
pub struct MovementPredictionPlayer {
    /// Owning client.
    pub client: ClientId,
    /// Owning actor handle.
    pub actor: OwnedActor,
    /// Selected movement profile.
    pub profile: MovementProfile,
    /// Standing collision bounds.
    pub standing_bounds: Bounds,
    /// Collision bounds.
    pub bounds: Bounds,
    /// Source movement options, when a source client owns movement.
    pub source_movement: Option<ClientMovementOptions>,
    /// Character family.
    pub character: GameFamily,
    /// World gravity.
    pub world_gravity: f64,
    /// Host Q2 movement tuning.
    pub q2_movement_config: Option<Q2MovementConfig>,
    /// Flight flag.
    pub flight: bool,
    /// Source environment override.
    pub source_environment: Option<MovementEnvironment>,
    /// Gravity multiplier.
    pub gravity_multiplier: f64,
    /// Movement speed multiplier.
    pub movement_speed_multiplier: f64,
    /// Movement state.
    pub state: MovementState,
    /// Arsenal snapshot.
    pub arsenal: ArsenalState,
    /// Animation snapshot.
    pub animation: ActorAnimationState,
    /// View height.
    pub view_height: f64,
    /// Selected numeric profile (donor `services.numeric`).
    pub numeric: NumericProfile,
    /// Detached Q3 arsenal runtime, when the player owns one.
    pub q3_arsenal: Option<Q3ArsenalRuntimeState>,
}

impl MovementPredictionPlayer {
    /// Snapshot a movement player for prediction.
    pub fn from_player(
        player: &MovementPlayer,
        numeric: NumericProfile,
        q3_arsenal: Option<Q3ArsenalRuntimeState>,
    ) -> Self {
        Self {
            client: player.client.clone(),
            actor: player.actor.clone(),
            profile: player.profile.clone(),
            standing_bounds: player.standing_bounds,
            bounds: player.bounds,
            source_movement: player.source_movement.clone(),
            character: player.character,
            world_gravity: player.world_gravity,
            q2_movement_config: player.q2_movement_config,
            flight: player.flight,
            source_environment: player.source_environment,
            gravity_multiplier: player.gravity_multiplier,
            movement_speed_multiplier: player.movement_speed_multiplier,
            state: player.state.clone(),
            arsenal: player.arsenal.clone(),
            animation: player.animation.clone(),
            view_height: player.view_height,
            numeric,
            q3_arsenal,
        }
    }

    /// Project the locomotion slice.
    pub fn locomotion(&self) -> LocomotionPlayer {
        LocomotionPlayer {
            profile: self.profile.clone(),
            standing_bounds: self.standing_bounds,
            source_movement: self.source_movement.clone(),
            character: self.character,
            world_gravity: self.world_gravity,
            q2_movement_config: self.q2_movement_config,
            flight: self.flight,
        }
    }
}

impl MovementEnvironmentPlayer for MovementPredictionPlayer {
    fn source_environment(&self) -> &Option<MovementEnvironment> {
        &self.source_environment
    }

    fn gravity_multiplier(&self) -> f64 {
        self.gravity_multiplier
    }

    fn state_is_q3(&self) -> bool {
        matches!(self.state, MovementState::Q3(_))
    }

    fn flight(&self) -> bool {
        self.flight
    }

    fn movement_speed_multiplier(&self) -> f64 {
        self.movement_speed_multiplier
    }
}

/// Detached Q3 arsenal storage behind source movement hooks.
#[derive(Debug, Clone)]
struct PredictionArsenalAccess {
    /// Shared runtime (prediction writes stay detached from live actors).
    runtime: Rc<RefCell<Q3ArsenalRuntimeState>>,
}

impl Q3ArsenalRuntimeAccess for PredictionArsenalAccess {
    fn read(&self, _actor: &OwnedActor, _execution: Q3Execution) -> Q3ArsenalRuntimeState {
        self.runtime.borrow().clone()
    }

    fn write(&self, _actor: &OwnedActor, _execution: Q3Execution, state: Q3ArsenalRuntimeState) {
        *self.runtime.borrow_mut() = state;
    }

    fn gauntlet_hit(&self, _context: &ContentHookContext) -> bool {
        false
    }
}

/// Project a world hook context onto the content hook fields.
fn content_hook_context(context: &WorldHookContext) -> ContentHookContext {
    ContentHookContext {
        input: Q3HookInput {
            actor: context.input.fields.actor.clone(),
            execution: match context.input.fields.execution {
                WorldExecution::Authoritative => Q3Execution::Authoritative,
                WorldExecution::Prediction => Q3Execution::Prediction,
            },
            command: UserCommand::Q3(context.input.command),
            environment: context.input.fields.environment,
        },
        motion: Q3MotionWork {
            pm_type: context.motion.pm_type,
            pm_flags: context.motion.pm_flags,
            event_sequence: context.motion.event_sequence,
            product: context.input.profile.product,
        },
        command: Q3HookCommand {
            buttons: context.command.buttons,
            weapon: context.command.weapon,
        },
        frame: context.frame,
        arsenal: context.arsenal.clone(),
        animation: context.animation.clone(),
    }
}

/// Source Q3 hooks over detached arsenal storage (donor `createQ3SourceMovementHooks` branch).
pub struct SourceQ3Hooks {
    /// Content hooks (rebuilt on clone; storage stays shared).
    hooks: Q3SourceMovementHooks<PredictionArsenalAccess>,
    /// Detached storage (clones share the runtime).
    access: PredictionArsenalAccess,
}

impl SourceQ3Hooks {
    /// Wrap detached arsenal storage.
    pub fn new(runtime: Q3ArsenalRuntimeState) -> Self {
        let access = PredictionArsenalAccess {
            runtime: Rc::new(RefCell::new(runtime)),
        };
        Self {
            hooks: create_q3_source_movement_hooks(access.clone()),
            access,
        }
    }
}

impl Clone for SourceQ3Hooks {
    fn clone(&self) -> Self {
        let access = self.access.clone();
        Self {
            hooks: create_q3_source_movement_hooks(access.clone()),
            access,
        }
    }
}

impl WorldHooks for SourceQ3Hooks {
    fn firing(&mut self, context: &WorldHookContext) -> bool {
        self.hooks.firing(&content_hook_context(context))
    }

    fn animation(&mut self, request: Q3AnimationRequest, context: &WorldHookContext) -> Q3AnimationStepResult {
        self.hooks
            .animation(&request, &content_hook_context(context))
            .unwrap_or(Q3AnimationStepResult {
                animation: context.animation.clone(),
                effects: Vec::new(),
            })
    }

    fn weapon(&mut self, context: &WorldHookContext) -> Q3WeaponPhaseResult {
        self.hooks
            .weapon(&content_hook_context(context))
            .map(|step| Q3WeaponPhaseResult {
                arsenal: step.arsenal,
                animation: step.animation,
                effects: step.effects,
                continuation: None,
                movement_flags: step.movement_flags,
            })
            .unwrap_or(Q3WeaponPhaseResult {
                arsenal: context.arsenal.clone(),
                animation: context.animation.clone(),
                effects: Vec::new(),
                continuation: None,
                movement_flags: context.motion.pm_flags,
            })
    }

    fn torso(&mut self, context: &WorldHookContext) -> Q3AnimationStepResult {
        self.hooks
            .torso(&content_hook_context(context))
            .unwrap_or(Q3AnimationStepResult {
                animation: context.animation.clone(),
                effects: Vec::new(),
            })
    }
}

/// Detached Q3 hooks without an arsenal owner (donor inline-hooks branch).
#[derive(Debug, Clone, Default)]
pub struct DetachedQ3Hooks;

impl WorldHooks for DetachedQ3Hooks {
    fn firing(&mut self, _context: &WorldHookContext) -> bool {
        false
    }

    fn animation(&mut self, request: Q3AnimationRequest, context: &WorldHookContext) -> Q3AnimationStepResult {
        let identity = || Q3AnimationStepResult {
            animation: context.animation.clone(),
            effects: Vec::new(),
        };
        if matches!(context.animation.state, AnimationState::Q3 { .. }) {
            q3_source_animation(request, context).unwrap_or_else(|_| identity())
        } else {
            identity()
        }
    }

    fn weapon(&mut self, context: &WorldHookContext) -> Q3WeaponPhaseResult {
        Q3WeaponPhaseResult {
            arsenal: context.arsenal.clone(),
            animation: context.animation.clone(),
            effects: Vec::new(),
            continuation: None,
            movement_flags: context.motion.pm_flags,
        }
    }

    fn torso(&mut self, context: &WorldHookContext) -> Q3AnimationStepResult {
        let identity = || Q3AnimationStepResult {
            animation: context.animation.clone(),
            effects: Vec::new(),
        };
        if matches!(context.animation.state, AnimationState::Q3 { .. }) {
            q3_source_torso(player_animation::TORSO_STAND, context, true).unwrap_or_else(|_| identity())
        } else {
            identity()
        }
    }
}

/// Q3 hooks selected for prediction (donor `q3Hooks` union).
#[derive(Clone)]
pub enum Q3PredictionHooks {
    /// Source hooks over detached arsenal storage.
    Source(SourceQ3Hooks),
    /// Detached hooks without an arsenal owner.
    Detached(DetachedQ3Hooks),
}

impl WorldHooks for Q3PredictionHooks {
    fn firing(&mut self, context: &WorldHookContext) -> bool {
        match self {
            Q3PredictionHooks::Source(hooks) => hooks.firing(context),
            Q3PredictionHooks::Detached(hooks) => hooks.firing(context),
        }
    }

    fn animation(&mut self, request: Q3AnimationRequest, context: &WorldHookContext) -> Q3AnimationStepResult {
        match self {
            Q3PredictionHooks::Source(hooks) => hooks.animation(request, context),
            Q3PredictionHooks::Detached(hooks) => hooks.animation(request, context),
        }
    }

    fn weapon(&mut self, context: &WorldHookContext) -> Q3WeaponPhaseResult {
        match self {
            Q3PredictionHooks::Source(hooks) => hooks.weapon(context),
            Q3PredictionHooks::Detached(hooks) => hooks.weapon(context),
        }
    }

    fn torso(&mut self, context: &WorldHookContext) -> Q3AnimationStepResult {
        match self {
            Q3PredictionHooks::Source(hooks) => hooks.torso(context),
            Q3PredictionHooks::Detached(hooks) => hooks.torso(context),
        }
    }
}

/// Up normal for empty-trace planes.
fn up_normal() -> Vec3 {
    Vec3 { x: 0.0, y: 0.0, z: 1.0 }
}

/// Empty-world Q1 trace (collision-lane seam).
fn empty_q1_trace(query: &Q1TraceQuery) -> Q1Trace {
    Q1Trace {
        fraction: 1.0,
        end: query.end,
        start_solid: false,
        all_solid: false,
        contact: TraceContact::None,
        hit: WorldHit::None,
        in_open: true,
        in_water: false,
        source_plane: Plane {
            normal: up_normal(),
            distance: 0.0,
        },
        surface_flags: None,
    }
}

/// Empty-world Q2 trace (collision-lane seam).
fn empty_q2_trace(query: &Q2TraceQuery) -> Q2Trace {
    Q2Trace {
        fraction: 1.0,
        end: query.end,
        start_solid: false,
        all_solid: false,
        contact: TraceContact::None,
        hit: WorldHit::None,
        contents: CONTENTS_EMPTY,
        surface: None,
        source_plane: Q2TracePlane {
            normal: up_normal(),
            dist: 0.0,
            plane_type: 2,
            signbits: 0,
        },
        secondary: None,
    }
}

/// Empty-world Q3 trace (collision-lane seam).
fn empty_q3_trace(query: &Q3TraceQuery) -> Q3Trace {
    Q3Trace {
        fraction: 1.0,
        end: query.end,
        start_solid: false,
        all_solid: false,
        contact: TraceContact::None,
        hit: WorldHit::None,
        contents: CONTENTS_EMPTY,
        surface_flags: 0,
        source_plane: BspPlane {
            normal: up_normal(),
            distance: 0.0,
            plane_type: 2,
            signbits: 0,
        },
    }
}

/// Detached Q1 services: identity touch/weapon/animation, empty-world traces.
struct PredictionQ1Services {
    /// Numeric operations for the selected profile.
    numeric: NumericOps,
}

impl Q1MovementServices for PredictionQ1Services {
    fn numeric(&self) -> NumericOps {
        self.numeric
    }

    fn trace(&mut self, query: Q1TraceQuery) -> Q1Trace {
        empty_q1_trace(&query)
    }

    fn point_contents(&mut self, _point: Vec3) -> i32 {
        CONTENTS_EMPTY
    }

    fn touch(&mut self, _contact: MovementTouchContact, state: Q1State) -> MovementContinuation<Q1State> {
        MovementContinuation::Continue(state)
    }

    fn weapon_step(&mut self, input: Q1WeaponStepInput<'_>, _state: &Q1State) -> Q1WeaponStepResult {
        Q1WeaponStepResult {
            continuation: None,
            arsenal: input.arsenal.clone(),
            animation: input.animation.clone(),
            effects: Vec::new(),
        }
    }

    fn animation_step(&mut self, input: Q1AnimationStepInput<'_>) -> Q1AnimationStepResult {
        Q1AnimationStepResult {
            animation: input.animation.clone(),
            effects: Vec::new(),
        }
    }
}

/// Detached Q2 services: continuing touch, empty-world traces.
struct PredictionQ2Services {
    /// Numeric operations for the selected profile.
    numeric: NumericOps,
}

impl Q2MovementServices for PredictionQ2Services {
    fn numeric(&self) -> NumericOps {
        self.numeric
    }

    fn trace(&mut self, query: Q2TraceQuery) -> Q2Trace {
        empty_q2_trace(&query)
    }

    fn point_contents(&mut self, _query: Q2ContentsQuery) -> (i32, i32) {
        (CONTENTS_EMPTY, CONTENTS_EMPTY)
    }

    fn touch(&mut self, _contact: Q2TouchContact, state: Q2State) -> MovementContinuation<Q2State> {
        MovementContinuation::Continue(state)
    }
}

/// Detached Q3 services: empty-world traces.
struct PredictionQ3Services {
    /// Numeric operations for the selected profile.
    numeric: NumericOps,
}

impl Q3MovementServices for PredictionQ3Services {
    fn numeric(&self) -> NumericOps {
        self.numeric
    }

    fn trace(&mut self, query: Q3TraceQuery) -> Q3Trace {
        empty_q3_trace(&query)
    }

    fn point_contents(&mut self, _point: Vec3, _pass_actor: &ActorId) -> i32 {
        CONTENTS_EMPTY
    }
}
use qa_bots::scene::TraceHit as BotsHit;

/// Provider family of a movement profile.
fn movement_kind(profile: &MovementProfile) -> MovementKind {
    match profile {
        MovementProfile::Q1Netquake(_) => MovementKind::Q1Netquake,
        MovementProfile::Q1Quakeworld(_) => MovementKind::Q1Quakeworld,
        MovementProfile::Q2Classic(_) => MovementKind::Q2Classic,
        MovementProfile::Q2Rerelease(_) => MovementKind::Q2Rerelease,
        MovementProfile::Q3(_) => MovementKind::Q3,
    }
}

/// Provider identity of a movement profile.
fn profile_id(profile: &MovementProfile) -> &ProviderId {
    match profile {
        MovementProfile::Q1Netquake(inner) => &inner.id,
        MovementProfile::Q1Quakeworld(inner) => &inner.id,
        MovementProfile::Q2Classic(inner) => &inner.id,
        MovementProfile::Q2Rerelease(inner) => &inner.id,
        MovementProfile::Q3(inner) => &inner.id,
    }
}

/// Navigation-visible origin state for a full movement state.
fn bots_state_for_state(state: &MovementState) -> BotsState {
    match state {
        MovementState::Q1Netquake(inner) => BotsState::Q1Netquake { origin: inner.origin },
        MovementState::Q1Quakeworld(inner) => BotsState::Q1Quakeworld { origin: inner.origin },
        MovementState::Q2Classic(inner) => BotsState::Q2Classic {
            origin_eighths: inner.origin_eighths,
        },
        MovementState::Q2Rerelease(inner) => BotsState::Q2Rerelease { origin: inner.origin },
        MovementState::Q3(inner) => BotsState::Q3 { origin: inner.origin },
    }
}

/// Navigation-visible ground contact for a world hit.
fn bots_ground(ground: &WorldHit) -> BotsHit {
    match ground {
        WorldHit::None => BotsHit::None,
        WorldHit::World { model } => BotsHit::World { model: *model as i32 },
        WorldHit::Actor { actor } => BotsHit::Actor { actor: actor.clone() },
    }
}

/// Rich movement step behind one prediction command (donor active `MovementResult`).
#[derive(Debug, Clone)]
pub struct PredictedMovementStep {
    /// Full movement state.
    pub state: MovementState,
    /// Ground contact.
    pub ground: WorldHit,
    /// Water level.
    pub water_level: i32,
    /// Water contents type.
    pub water_type: i32,
    /// Ordered effects.
    pub effects: Vec<OrderedMovementEffect>,
}

/// Pending prediction slot shared by input building and stepping.
struct PredictionPending {
    /// Current full state.
    state: MovementState,
    /// Current bounds.
    bounds: Bounds,
    /// Shared input fields for the pending command.
    fields: Option<MovementInputFields>,
    /// Pending user command.
    command: Option<UserCommand>,
    /// Last rich step.
    last: PredictedMovementStep,
}

/// Detached player movement prediction (donor `PlayerMovementPrediction`).
pub struct PlayerMovementPrediction {
    /// Selected provider.
    provider: PlayerMovementProvider<Q3PredictionHooks>,
    /// Navigation services.
    services: BotsServices,
    /// Pending slot.
    pending: RefCell<PredictionPending>,
    /// Movement environment.
    environment: MovementEnvironment,
    /// Selected profile.
    profile: MovementProfile,
    /// Step length in milliseconds.
    milliseconds: i32,
    /// Crouched flag.
    crouched: bool,
    /// Crouched bounds.
    crouched_bounds: Bounds,
    /// Native QuakeWorld flag.
    native_quake_world: bool,
    /// Moving actor.
    actor: OwnedActor,
    /// Player collision bounds.
    bounds: Bounds,
    /// Player standing bounds.
    standing_bounds: Bounds,
    /// Player view height.
    view_height: f64,
    /// Arsenal snapshot (identity weapon steps keep it constant).
    arsenal: ArsenalState,
    /// Animation snapshot (identity animation steps keep it constant).
    animation: ActorAnimationState,
    /// Live Q3 weapon override, when a Q3 entity is bound.
    entity_weapon: Option<i32>,
    /// Baseline Q3 command time, when the baseline is Q3.
    q3_command_time: Option<i32>,
    /// Simulation handle (live clock reads).
    simulation: SharedSimulation,
    /// Numeric operations for the selected profile.
    numeric_ops: NumericOps,
}

impl PlayerMovementPrediction {
    /// Last rich movement step.
    pub fn last_step(&self) -> PredictedMovementStep {
        self.pending.borrow().last.clone()
    }

    /// Build the next prediction input for a traversal request.
    fn build_input(
        &self,
        previous: Option<&BotsResult>,
        command_index: usize,
        request: &TraversalRequest,
    ) -> BotsInput {
        if matches!(previous, Some(BotsResult::ActorRemoved)) {
            panic!("Movement prediction removed its actor");
        }
        let mut pending = self.pending.borrow_mut();
        // Command move from the traversal request (donor navigation-driver math,
        // relocated here because the Rust driver passes the request through).
        let at = if previous.is_some() {
            movement_origin(&pending.state)
        } else {
            request.from
        };
        let dx = f64::from(request.to.x) - f64::from(at.x);
        let dy = f64::from(request.to.y) - f64::from(at.y);
        let distance = dx.hypot(dy);
        let scale = if distance == 0.0 {
            0.0
        } else {
            400.0_f64.min(distance * 20.0) / distance
        };
        let (command_x, command_y) = (dx * scale, dy * scale);
        let command_z: f64 = match request.mode {
            TravelMode::Jump | TravelMode::WaterJump | TravelMode::Ladder => 400.0,
            TravelMode::Crouch => -400.0,
            _ => 0.0,
        };
        let state = pending.state.clone();
        let yaw = command_y.atan2(command_x);
        let horizontal = 400.0_f64.min(command_x.hypot(command_y));
        let up = command_z.clamp(-400.0, 400.0);
        let angles = Vec3 {
            x: 0.0,
            y: (yaw * 180.0 / std::f64::consts::PI) as f32,
            z: 0.0,
        };
        let words = [
            0,
            (yaw * 65536.0 / (std::f64::consts::PI * 2.0)).round() as i32 & 65535,
            0,
        ];
        let base_ms = match self.q3_command_time {
            Some(baseline) => f64::from(baseline),
            None => self.simulation.time_seconds() * 1000.0,
        };
        let time = base_ms + (command_index as f64 + 1.0) * f64::from(self.milliseconds);
        let bounds = if matches!(self.profile, MovementProfile::Q1Quakeworld(_)) && !self.native_quake_world {
            if previous.is_some() {
                pending.bounds
            } else if self.crouched {
                self.crouched_bounds
            } else {
                self.bounds
            }
        } else {
            self.standing_bounds
        };
        let frame = FrameContext {
            frame: command_index as i32,
            time: SourceTime::Milliseconds(time as i32),
            elapsed: SourceTime::Milliseconds(self.milliseconds),
            phase: FramePhase::ClientCommand,
        };
        let fields = MovementInputFields {
            actor: self.actor.clone(),
            command_sequence: command_index as i32,
            frame,
            shape: TraceShape::Box(bounds),
            current_bounds: None,
            environment: self.environment,
            arsenal: self.arsenal.clone(),
            animation: self.animation.clone(),
            execution: WorldExecution::Prediction,
        };
        let command = match (&self.profile, &state) {
            (MovementProfile::Q1Netquake(_), MovementState::Q1Netquake(_)) => UserCommand::Q1Netquake(Q1UserCommand {
                acknowledged_server_time_seconds: time / 1000.0,
                view_angles: angles,
                forward_move: horizontal,
                side_move: 0.0,
                up_move: up,
                buttons: if up > 0.0 { 2 } else { 0 },
                impulse: 0,
            }),
            (MovementProfile::Q1Quakeworld(_), MovementState::Q1Quakeworld(_)) => {
                UserCommand::Q1Quakeworld(QwUserCommand {
                    milliseconds: self.milliseconds,
                    angles,
                    forward_move: horizontal,
                    side_move: 0.0,
                    up_move: up,
                    buttons: if up > 0.0 { 2 } else { 0 },
                    impulse: 0,
                })
            }
            (MovementProfile::Q2Classic(_), MovementState::Q2Classic(q2)) => UserCommand::Q2Classic(Q2UserCommand {
                milliseconds: self.milliseconds,
                angle_shorts: [
                    words[0] - q2.delta_angle_shorts[0],
                    words[1] - q2.delta_angle_shorts[1],
                    words[2] - q2.delta_angle_shorts[2],
                ],
                forward_move: horizontal,
                side_move: 0.0,
                up_move: up,
                buttons: 0,
                impulse: 0,
                light_level: 0,
            }),
            (MovementProfile::Q2Rerelease(_), MovementState::Q2Rerelease(q2)) => {
                UserCommand::Q2Rerelease(Q2RereleaseUserCommand {
                    milliseconds: self.milliseconds,
                    angles: Vec3 {
                        x: -q2.delta_angles.x,
                        y: angles.y - q2.delta_angles.y,
                        z: -q2.delta_angles.z,
                    },
                    forward_move: horizontal,
                    side_move: 0.0,
                    buttons: if up > 0.0 {
                        8
                    } else if up < 0.0 {
                        16
                    } else {
                        0
                    },
                    server_frame: command_index as i32,
                })
            }
            (MovementProfile::Q3(_), MovementState::Q3(q3)) => {
                let weapon = self.entity_weapon.unwrap_or(match &self.arsenal.state {
                    WeaponState::Q3 { source_weapon, .. } => *source_weapon,
                    _ => 0,
                });
                UserCommand::Q3(Q3UserCommand {
                    server_time_milliseconds: time as i32,
                    angle_words: [
                        -q3.delta_angle_words[0],
                        (words[1] - q3.delta_angle_words[1]) & 65535,
                        -q3.delta_angle_words[2],
                    ],
                    buttons: 0,
                    weapon,
                    forward_move: (horizontal * 127.0 / 400.0).round() as i32,
                    right_move: 0,
                    up_move: (up * 127.0 / 400.0).round() as i32,
                })
            }
            _ => panic!("Player state differs from selected movement"),
        };
        pending.fields = Some(fields.clone());
        pending.command = Some(command);
        pending.bounds = bounds;
        BotsInput {
            kind: movement_kind(&self.profile),
            profile: BotsProfile {
                kind: movement_kind(&self.profile),
                id: profile_id(&self.profile).clone(),
                numeric: self.services.numeric,
            },
            frame: fields.frame,
            execution: BotsExecution::Prediction,
        }
    }

    /// Commit an active provider outcome.
    fn commit<Contact>(
        &self,
        pending: &mut PredictionPending,
        state: MovementState,
        fields: qa_world::movement::types::MovementResultFields<Contact>,
    ) -> BotsResult {
        pending.state = state.clone();
        pending.bounds = fields.bounds;
        pending.last = PredictedMovementStep {
            state: state.clone(),
            ground: fields.ground.clone(),
            water_level: fields.water_level,
            water_type: fields.water_type,
            effects: fields.effects.clone(),
        };
        BotsResult::Active {
            state: bots_state_for_state(&state),
            ground: bots_ground(&fields.ground),
            water_level: fields.water_level,
        }
    }

    /// Step the pending command through the selected provider.
    fn advance_step(&self, _input: &BotsInput, _services: &BotsServices) -> BotsResult {
        let mut pending = self.pending.borrow_mut();
        let fields = pending
            .fields
            .take()
            .expect("prediction input builds the pending fields before advance");
        let command = pending
            .command
            .take()
            .expect("prediction input builds the pending command before advance");
        let state = pending.state.clone();
        match (&self.provider, &self.profile, state, command) {
            (
                PlayerMovementProvider::Q1Netquake(provider),
                MovementProfile::Q1Netquake(profile),
                MovementState::Q1Netquake(state),
                UserCommand::Q1Netquake(command),
            ) => {
                let mut services = PredictionQ1Services {
                    numeric: self.numeric_ops,
                };
                let input = Q1MovementInput {
                    fields,
                    command,
                    state,
                    profile: profile.clone(),
                };
                match provider
                    .move_step(input, &mut services)
                    .expect("player movement prediction step")
                {
                    MovementOutcome::Active { fields, state } => {
                        self.commit(&mut pending, MovementState::Q1Netquake(state), fields)
                    }
                    MovementOutcome::ActorRemoved { .. } => BotsResult::ActorRemoved,
                }
            }
            (
                PlayerMovementProvider::Q1QuakeworldNative(provider),
                MovementProfile::Q1Quakeworld(profile),
                MovementState::Q1Quakeworld(state),
                UserCommand::Q1Quakeworld(command),
            ) => {
                let mut services = PredictionQ1Services {
                    numeric: self.numeric_ops,
                };
                let input = QwMovementInput {
                    fields,
                    command,
                    state,
                    profile: profile.clone(),
                };
                match provider
                    .move_step(input, &mut services)
                    .expect("player movement prediction step")
                {
                    MovementOutcome::Active { fields, state } => {
                        self.commit(&mut pending, MovementState::Q1Quakeworld(state), fields)
                    }
                    MovementOutcome::ActorRemoved { .. } => BotsResult::ActorRemoved,
                }
            }
            (
                PlayerMovementProvider::Q1QuakeworldShared(provider),
                MovementProfile::Q1Quakeworld(profile),
                MovementState::Q1Quakeworld(state),
                UserCommand::Q1Quakeworld(command),
            ) => {
                let mut services = PredictionQ1Services {
                    numeric: self.numeric_ops,
                };
                let input = QwMovementInput {
                    fields,
                    command,
                    state,
                    profile: profile.clone(),
                };
                match provider
                    .move_step(input, &mut services)
                    .expect("player movement prediction step")
                {
                    MovementOutcome::Active { fields, state } => {
                        self.commit(&mut pending, MovementState::Q1Quakeworld(state), fields)
                    }
                    MovementOutcome::ActorRemoved { .. } => BotsResult::ActorRemoved,
                }
            }
            (
                PlayerMovementProvider::Q2Classic(provider),
                MovementProfile::Q2Classic(profile),
                MovementState::Q2Classic(state),
                UserCommand::Q2Classic(command),
            ) => {
                let mut services = PredictionQ2Services {
                    numeric: self.numeric_ops,
                };
                let input = Q2MovementInput {
                    fields,
                    command,
                    state,
                    profile: profile.clone(),
                };
                match provider
                    .move_step(input, &mut services)
                    .expect("player movement prediction step")
                {
                    MovementOutcome::Active { fields, state } => {
                        self.commit(&mut pending, MovementState::Q2Classic(state), fields)
                    }
                    MovementOutcome::ActorRemoved { .. } => BotsResult::ActorRemoved,
                }
            }
            (
                PlayerMovementProvider::Q2Rerelease(provider),
                MovementProfile::Q2Rerelease(profile),
                MovementState::Q2Rerelease(state),
                UserCommand::Q2Rerelease(command),
            ) => {
                let mut services = PredictionQ2Services {
                    numeric: self.numeric_ops,
                };
                let input = Q2RereleaseMovementInput {
                    fields,
                    command,
                    state,
                    profile: profile.clone(),
                    view_offset: Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: self.view_height as f32,
                    },
                    snap_initial: true,
                };
                match provider
                    .borrow_mut()
                    .move_step(input, &mut services)
                    .expect("player movement prediction step")
                {
                    Q2RereleaseMovementResult::Active { fields, state, .. } => {
                        self.commit(&mut pending, MovementState::Q2Rerelease(state), fields)
                    }
                    Q2RereleaseMovementResult::ActorRemoved { .. } => BotsResult::ActorRemoved,
                }
            }
            (
                PlayerMovementProvider::Q3(provider),
                MovementProfile::Q3(profile),
                MovementState::Q3(state),
                UserCommand::Q3(command),
            ) => {
                let mut services = PredictionQ3Services {
                    numeric: self.numeric_ops,
                };
                let input = Q3MovementInput {
                    fields,
                    command,
                    state,
                    profile: profile.clone(),
                };
                match provider
                    .move_step(input, &mut services)
                    .expect("player movement prediction step")
                {
                    MovementOutcome::Active { fields, state } => {
                        self.commit(&mut pending, MovementState::Q3(state), fields)
                    }
                    MovementOutcome::ActorRemoved { .. } => BotsResult::ActorRemoved,
                }
            }
            _ => panic!("Player state differs from selected movement"),
        }
    }
}

impl BotsProvider for PlayerMovementPrediction {
    fn kind(&self) -> MovementKind {
        movement_kind(&self.profile)
    }

    fn id(&self) -> &ProviderId {
        profile_id(&self.profile)
    }

    fn advance(&self, input: &BotsInput, services: &BotsServices) -> BotsResult {
        self.advance_step(input, services)
    }
}

impl NavigationPrediction for PlayerMovementPrediction {
    fn provider(&self) -> &dyn BotsProvider {
        self
    }

    fn services(&self) -> &BotsServices {
        &self.services
    }

    fn input(&self, previous: Option<&BotsResult>, command_index: usize, request: &TraversalRequest) -> BotsInput {
        self.build_input(previous, command_index, request)
    }
}

/// Q3 source baseline for prediction.
struct Q3PredictionBaseline {
    /// Live Q3 movement state, when a Q3 entity is bound.
    movement: Option<Q3MovementState>,
    /// Merged Q3 arsenal runtime, when the player owns Q3 arsenal.
    arsenal: Option<Q3ArsenalRuntimeState>,
    /// Live Q3 weapon override, when a Q3 entity is bound.
    weapon: Option<i32>,
}

/// Resolve the Q3 source baseline.
///
/// Implements the donor null-source path (donor player-movement.ts:117-126):
/// with no bound Q3 entity, prediction falls back to the player snapshot
/// state, the player arsenal (or a fresh `Baseq3` runtime), and the arsenal
/// source weapon (donor player-movement.ts:163). Live-source reads
/// (`readQ3MovementState`/`readQ3ArsenalRuntime` over the entity pool and
/// records) need the q3 lane's native source host: `SharedSimulation`
/// holds the opaque `Q3SourceRuntime` seam (`simulation/runtime.rs:4154`,
/// exposed through `with_q3_source` at `simulation/runtime.rs:7774` with
/// the bound product only), so no pool, records, or level time is
/// reachable here; extend this resolver when that seam lands.
fn q3_prediction_baseline(
    _simulation: &SharedSimulation,
    _player: &MovementPredictionPlayer,
    _profile: &MovementProfile,
) -> Q3PredictionBaseline {
    Q3PredictionBaseline {
        movement: None,
        arsenal: None,
        weapon: None,
    }
}

/// Spawn a fresh Q3 arsenal runtime for detached prediction.
fn spawn_arsenal_runtime(product: Product, max_health: f64) -> Q3ArsenalRuntimeState {
    qa_content::q3::foundation::arsenal::q3_spawn_arsenal_runtime(product, max_health, 0)
}

/// Whether the bound QuakeC source is native QuakeWorld (assembly zone: C7's
/// `quakec_source` accessor lands with reconstruction; adapt below).
fn is_native_quake_world(simulation: &SharedSimulation) -> bool {
    matches!(
        simulation.quakec_source().as_ref().map(|source| source.kind()),
        Some(QuakeCSourceKind::Quakeworld)
    )
}

/// Create detached player movement prediction (donor `createPlayerMovementPrediction`).
pub fn create_player_movement_prediction(
    simulation: &SharedSimulation,
    player: &MovementPredictionPlayer,
    origin: Vec3,
    velocity: Vec3,
    milliseconds: i32,
    crouched: bool,
    selected_profile: Option<&MovementProfile>,
) -> PlayerMovementPrediction {
    let locomotion = player.locomotion();
    let profile = selected_movement_profile(&locomotion, selected_profile);
    let q3 = q3_prediction_baseline(simulation, player, &profile);
    let (health, invulnerable) = simulation
        .combat_state(player.actor.id())
        .map(|combat| (combat.health, combat.invulnerable))
        .or_else(|| {
            player
                .source_environment
                .as_ref()
                .map(|env| (env.health, env.invulnerable))
        })
        .expect("Player has no combat state during movement prediction");
    let environment = player_movement_environment(player, health, invulnerable);
    let baseline = match (&q3.movement, &profile) {
        (Some(movement), MovementProfile::Q3(_)) => MovementState::Q3(movement.clone()),
        _ => player.state.clone(),
    };
    let mut initial = relocated(&baseline, origin, velocity);
    if let MovementState::Q3(inner) = &mut initial {
        inner.movement_flags = (inner.movement_flags & !2) | if crouched { 1 } else { 0 };
    }
    let q3_hooks = if matches!(player.arsenal.state, WeaponState::Q3 { .. }) {
        let runtime = q3
            .arsenal
            .or_else(|| player.q3_arsenal.clone())
            .unwrap_or_else(|| spawn_arsenal_runtime(Product::Baseq3, 100.0));
        Q3PredictionHooks::Source(SourceQ3Hooks::new(runtime))
    } else {
        Q3PredictionHooks::Detached(DetachedQ3Hooks)
    };
    let native_quake_world = is_native_quake_world(simulation);
    let provider = create_player_movement_provider(
        &locomotion,
        player.view_height,
        PlayerMovementHooks {
            native_quake_world,
            publish_posture: None,
            q1: Q1MovementOptions {
                view_height: Some(player.view_height),
                ..Default::default()
            },
            q2: Q2RereleaseMovementContext::new(),
            q3: q3_hooks,
        },
    );
    let crouched_bounds = player_crouched_bounds(&locomotion);
    let bounds = if matches!(profile, MovementProfile::Q1Quakeworld(_)) && !native_quake_world {
        if crouched {
            crouched_bounds
        } else {
            player.bounds
        }
    } else {
        player.standing_bounds
    };
    let q3_command_time = match &baseline {
        MovementState::Q3(inner) => Some(inner.command_time_milliseconds),
        _ => None,
    };
    let last = PredictedMovementStep {
        state: initial.clone(),
        ground: WorldHit::None,
        water_level: 0,
        water_type: 0,
        effects: Vec::new(),
    };
    PlayerMovementPrediction {
        provider,
        services: BotsServices {
            numeric: player.numeric,
        },
        pending: RefCell::new(PredictionPending {
            state: initial,
            bounds,
            fields: None,
            command: None,
            last,
        }),
        environment,
        profile,
        milliseconds,
        crouched,
        crouched_bounds,
        native_quake_world,
        actor: player.actor.clone(),
        bounds: player.bounds,
        standing_bounds: player.standing_bounds,
        view_height: player.view_height,
        arsenal: player.arsenal.clone(),
        animation: player.animation.clone(),
        entity_weapon: q3.weapon,
        q3_command_time,
        simulation: simulation.clone(),
        numeric_ops: NumericOps::select(player.numeric).expect("player movement numeric profile"),
    }
}

/// Medium of a movement observation (donor `medium`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementMedium {
    /// Dry ground.
    Dry,
    /// Water.
    Water,
    /// Slime.
    Slime,
    /// Lava.
    Lava,
}

/// Movement observation for a rich step (donor `movementObservation`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementObservation {
    /// State origin.
    pub origin: Vec3,
    /// Grounded flag.
    pub grounded: bool,
    /// Medium at the origin.
    pub medium: MovementMedium,
    /// Damaging fall flag.
    pub damaging_fall: bool,
}

/// Observe a rich movement step (donor `movementObservation`).
pub fn movement_observation(step: &PredictedMovementStep) -> MovementObservation {
    let medium = if step.water_level == 0 {
        MovementMedium::Dry
    } else {
        match &step.state {
            MovementState::Q1Netquake(_) | MovementState::Q1Quakeworld(_) => {
                if step.water_type == -4 {
                    MovementMedium::Slime
                } else if step.water_type == -5 {
                    MovementMedium::Lava
                } else {
                    MovementMedium::Water
                }
            }
            _ => {
                if step.water_type & 16 != 0 {
                    MovementMedium::Slime
                } else if step.water_type & 8 != 0 {
                    MovementMedium::Lava
                } else {
                    MovementMedium::Water
                }
            }
        }
    };
    MovementObservation {
        origin: movement_origin(&step.state),
        grounded: !matches!(step.ground, WorldHit::None),
        medium,
        damaging_fall: step.effects.iter().any(|ordered| {
            matches!(&ordered.effect, MovementEffect::Event(event)
                if event.event == entity_event::FALL_MEDIUM || event.event == entity_event::FALL_FAR)
        }),
    }
}
