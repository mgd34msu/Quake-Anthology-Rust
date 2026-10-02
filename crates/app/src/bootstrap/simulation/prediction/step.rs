//! One-command movement prediction over copied player state.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/prediction/step.ts`
//! (`copyMovementState`, `copyPredictionSnapshot`, `predictMovementCommand`).

use std::cell::RefCell;
use std::rc::Rc;

use qa_content::q3::base::shared::definitions::Product as ContentProduct;
use qa_content::q3::foundation::arsenal::{
    step_q3_arsenal, Q3ArsenalControls, Q3ArsenalRuntimeState, WeaponStepInput as Q3WeaponStepInput,
};
use qa_content::q3::foundation::character::{step_q3_character_animation, AnimationStepInput};
use qa_core::identity::OwnedActor;
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::{NumericError, NumericOps};
use qa_core::time::{FrameContext, FramePhase, SourceTime};
use qa_net::common::commands::ArsenalIntent;
use qa_world::movement::q1::netquake::create_q1_movement_provider;
use qa_world::movement::q1::quakeworld::create_qw_movement_provider;
use qa_world::movement::q1::types::{Q1AnimationStepInput, Q1AnimationStepResult};
use qa_world::movement::q1::types::{
    Q1MovementHooks, Q1MovementInput, Q1MovementOptions, Q1MovementServices, Q1PlayerInput, Q1State, Q1Trace,
    Q1TraceQuery, Q1WeaponStepInput, Q1WeaponStepResult, QwMovementInput,
};
use qa_world::movement::q2::rerelease::Q2RereleaseMovementContext;
use qa_world::movement::q2::types::{
    Q2ContentsQuery, Q2MovementInput, Q2MovementProfile, Q2MovementServices, Q2RereleaseMovementInput,
    Q2RereleaseMovementResult, Q2State, Q2TouchContact, Q2Trace, Q2TraceQuery,
};
use qa_world::movement::q2::{create_q2_classic_movement_provider, create_q2_rerelease_movement_provider};
use qa_world::movement::q3::animation::{q3_source_animation, q3_source_torso};
use qa_world::movement::q3::constants::move_flags::{RESPAWNED, USE_ITEM_HELD};
use qa_world::movement::q3::postures::Q3_SOURCE_POSTURES;
use qa_world::movement::q3::provider::{create_q3_movement_provider, Q3TracePolicy};
use qa_world::movement::q3::types::{
    Q3AnimationRequest, Q3AnimationStepResult, Q3HookContext, Q3MovementHooks, Q3MovementInput, Q3MovementServices,
    Q3Postures, Q3Product, Q3Trace, Q3TraceQuery, Q3WeaponPhaseResult,
};
use qa_world::movement::types::{
    ActorAnimationState, AnimationState, ArsenalState, MovementContinuation, MovementDialect, MovementEffect,
    MovementEnvironment, MovementError, MovementExecution, MovementInputFields, MovementOutcome, MovementResultFields,
    MovementTouchContact, OrderedMovementEffect, Q3UserCommand, TraceHit, TraceShape, UserCommand, WeaponState,
};

use super::super::arsenal::selected::MovementState;
use super::super::arsenal_intent::{resolve_q3_arsenal_controls, ArsenalIntentError};
use super::super::q3_commands::{relative_movement_command, CommandAngleSpace};
use super::super::shared_quakeworld_movement::create_shared_quake_world_movement;
use super::types::{
    MovementPredictionProfile, MovementPredictionSnapshot, MovementProbeOptions, PredictionCommand, PredictionContact,
    PredictionSceneHandle, PredictionStepOptions,
};
use qa_core::identity::ActorId;

/// Errors predicting one movement command.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PredictError {
    /// Command and state do not match the selected profile.
    #[error("Prediction command {command} and state {state} do not match selected {profile}")]
    DialectMismatch {
        /// Command dialect.
        command: String,
        /// State dialect.
        state: String,
        /// Profile dialect.
        profile: String,
    },
    /// Prediction removed the authoritative actor.
    #[error("Private movement prediction cannot remove an authoritative actor")]
    ActorRemoved,
    /// Arsenal intent without a selected arsenal owner.
    #[error("Prediction has no selected arsenal owner for this intent")]
    MissingArsenalOwner,
    /// Q3 weapon step without the snapshot's private arsenal runtime.
    #[error("Q3 prediction needs the snapshot's private arsenal runtime")]
    MissingQ3Runtime,
    /// Initial snapshot does not match the selected movement.
    #[error("Prediction snapshot does not match selected movement")]
    InitialSnapshotMismatch,
    /// Snapshot changed movement family.
    #[error("Prediction snapshot changed movement family")]
    SnapshotFamilyChanged,
    /// Command needs a finite clock and integer sequence.
    #[error("Prediction command needs a finite clock and integer sequence")]
    InvalidCommandClock,
    /// Command does not match the selected movement.
    #[error("Prediction command does not match selected movement")]
    CommandDialectMismatch,
    /// Arsenal intent does not fit the selected Q3 product.
    #[error(transparent)]
    Intent(#[from] ArsenalIntentError),
    /// Q3 arsenal step failed.
    #[error(transparent)]
    Arsenal(#[from] qa_content::q3::foundation::arsenal::ArsenalError),
    /// Q3 character animation failed.
    #[error(transparent)]
    Character(#[from] qa_content::q3::foundation::character::CharacterError),
    /// Q3 source animation failed.
    #[error("{0}")]
    Animation(MovementError),
    /// Movement provider failed.
    #[error("{0}")]
    Movement(#[from] MovementError),
    /// Numeric profile needs an unavailable backend.
    #[error(transparent)]
    Numeric(#[from] NumericError),
}

/// Deep copy of a movement state.
pub fn copy_movement_state(state: &MovementState) -> MovementState {
    state.clone()
}

/// Deep copy of a prediction snapshot.
pub fn copy_prediction_snapshot(snapshot: &MovementPredictionSnapshot) -> MovementPredictionSnapshot {
    snapshot.clone()
}

/// One predicted command: the replayed snapshot plus its ordered effects.
#[derive(Debug, Clone, PartialEq)]
pub struct PredictedCommand {
    /// Replayed player snapshot.
    pub player: MovementPredictionSnapshot,
    /// Ordered effects.
    pub effects: Vec<OrderedMovementEffect>,
}

type RuntimeCell = Rc<RefCell<Option<Q3ArsenalRuntimeState>>>;
type ErrorCell = Rc<RefCell<Option<PredictError>>>;

fn fail(error: &ErrorCell, failure: PredictError) {
    if error.borrow().is_none() {
        *error.borrow_mut() = Some(failure);
    }
}

fn content_product(product: ContentProduct) -> Q3Product {
    match product {
        ContentProduct::Baseq3 => Q3Product::BaseQ3,
        ContentProduct::Missionpack => Q3Product::MissionPack,
    }
}

/// Q3 weapon stepping shared by the Q1 services, the Q3 hooks, and the
/// Q2-with-Q3-arsenal follow-up step.
#[derive(Clone)]
struct WeaponDriver {
    movement_only: bool,
    intent: Option<ArsenalIntent>,
    environment: MovementEnvironment,
    gauntlet_hit: bool,
    runtime: RuntimeCell,
    error: ErrorCell,
}

impl WeaponDriver {
    fn step(
        &self,
        actor: &OwnedActor,
        command: &UserCommand,
        frame: &FrameContext,
        arsenal: &ArsenalState,
        animation: &ActorAnimationState,
    ) -> (ArsenalState, ActorAnimationState, Vec<MovementEffect>) {
        // Native Q1/Q2 client prediction does not execute server weapon gamecode.
        if self.movement_only || !matches!(arsenal.state, WeaponState::Q3 { .. }) {
            return (arsenal.clone(), animation.clone(), Vec::new());
        }
        let runtime = self.runtime.borrow().clone();
        let Some(runtime) = runtime else {
            fail(&self.error, PredictError::MissingQ3Runtime);
            return (arsenal.clone(), animation.clone(), Vec::new());
        };
        let controls = match resolve_q3_arsenal_controls(arsenal, self.intent.as_ref(), command, runtime.product) {
            Ok(controls) => controls,
            Err(error) => {
                fail(&self.error, PredictError::Intent(error));
                return (arsenal.clone(), animation.clone(), Vec::new());
            }
        };
        let controls = match command {
            UserCommand::Q3(command) => Q3ArsenalControls {
                use_holdable: command.buttons & 4 != 0,
                ..controls
            },
            _ => controls,
        };
        let input = Q3WeaponStepInput {
            actor: actor.clone(),
            command: *command,
            frame: *frame,
            arsenal: arsenal.clone(),
            animation: animation.clone(),
            environment: self.environment,
            gauntlet_hit: self.gauntlet_hit,
        };
        match step_q3_arsenal(&input, &runtime, &controls, None) {
            Ok(step) => {
                *self.runtime.borrow_mut() = Some(step.runtime.clone());
                (step.arsenal, step.animation, step.effects)
            }
            Err(error) => {
                fail(&self.error, PredictError::Arsenal(error));
                (arsenal.clone(), animation.clone(), Vec::new())
            }
        }
    }
}

/// Q3 character animation stepping for the Q1 services.
#[derive(Clone)]
struct AnimationDriver {
    movement_only: bool,
    source_health: f64,
    runtime: RuntimeCell,
    error: ErrorCell,
}

impl AnimationDriver {
    fn step(&self, input: &Q1AnimationStepInput<'_>) -> Q1AnimationStepResult {
        let passthrough = Q1AnimationStepResult {
            animation: input.animation.clone(),
            effects: Vec::new(),
        };
        if self.movement_only || !matches!(input.animation.state, AnimationState::Q3 { .. }) {
            return passthrough;
        }
        let runtime = self.runtime.borrow().clone();
        let product = content_product(
            runtime
                .as_ref()
                .map(|runtime| runtime.product)
                .unwrap_or(ContentProduct::Baseq3),
        );
        let event_sequence = runtime.map(|runtime| runtime.event_sequence).unwrap_or(0);
        let input = AnimationStepInput {
            frame: *input.frame,
            animation: input.animation.clone(),
            locomotion: input.locomotion,
            backwards: input.backwards,
            force: input.force,
        };
        match step_q3_character_animation(&input, product, self.source_health <= 0.0, event_sequence) {
            Ok(step) => Q1AnimationStepResult {
                animation: step.animation,
                effects: step.effects,
            },
            Err(error) => {
                fail(&self.error, PredictError::Character(error));
                passthrough
            }
        }
    }
}

#[derive(Clone)]
struct PredictionQ1Hooks {
    is_brush: super::types::PredictionBrushFn,
}

impl Q1MovementHooks for PredictionQ1Hooks {
    fn link(&mut self, _actor: &OwnedActor, state: Q1State, _touch_triggers: bool) -> MovementContinuation<Q1State> {
        MovementContinuation::Continue(state)
    }

    fn is_bsp(&self, hit: &TraceHit) -> bool {
        (self.is_brush)(hit)
    }

    fn before_physics(&mut self, _input: &Q1PlayerInput, state: Q1State) -> MovementContinuation<Q1State> {
        MovementContinuation::Continue(state)
    }

    fn after_physics(&mut self, _input: &Q1PlayerInput, state: Q1State) -> MovementContinuation<Q1State> {
        MovementContinuation::Continue(state)
    }
}

struct PredictionQ1Services {
    scene: PredictionSceneHandle,
    numeric: NumericOps,
    weapons: WeaponDriver,
    animation: AnimationDriver,
}

impl Q1MovementServices for PredictionQ1Services {
    fn numeric(&self) -> NumericOps {
        self.numeric
    }

    fn trace(&mut self, query: Q1TraceQuery) -> Q1Trace {
        self.scene.borrow_mut().trace_q1(query)
    }

    fn point_contents(&mut self, point: Vec3) -> i32 {
        self.scene.borrow_mut().point_contents_q1(point)
    }

    fn touch(&mut self, _contact: MovementTouchContact, state: Q1State) -> MovementContinuation<Q1State> {
        MovementContinuation::Continue(state)
    }

    fn weapon_step(&mut self, input: Q1WeaponStepInput<'_>, _state: &Q1State) -> Q1WeaponStepResult {
        let (arsenal, animation, effects) =
            self.weapons
                .step(input.actor, input.command, input.frame, input.arsenal, input.animation);
        Q1WeaponStepResult {
            continuation: None,
            arsenal,
            animation,
            effects,
        }
    }

    fn animation_step(&mut self, input: Q1AnimationStepInput<'_>) -> Q1AnimationStepResult {
        self.animation.step(&input)
    }
}

struct PredictionQ2Services {
    scene: PredictionSceneHandle,
    numeric: NumericOps,
}

impl Q2MovementServices for PredictionQ2Services {
    fn numeric(&self) -> NumericOps {
        self.numeric
    }

    fn trace(&mut self, query: Q2TraceQuery) -> Q2Trace {
        self.scene.borrow_mut().trace_q2(query)
    }

    fn point_contents(&mut self, query: Q2ContentsQuery) -> (i32, i32) {
        self.scene.borrow_mut().point_contents_q2(query)
    }

    fn touch(&mut self, _contact: Q2TouchContact, state: Q2State) -> MovementContinuation<Q2State> {
        MovementContinuation::Continue(state)
    }
}

struct PredictionQ3Services {
    scene: PredictionSceneHandle,
    numeric: NumericOps,
}

impl Q3MovementServices for PredictionQ3Services {
    fn numeric(&self) -> NumericOps {
        self.numeric
    }

    fn trace(&mut self, query: Q3TraceQuery) -> Q3Trace {
        self.scene.borrow_mut().trace_q3(query)
    }

    fn point_contents(&mut self, point: Vec3, pass_actor: &ActorId) -> i32 {
        self.scene.borrow_mut().point_contents_q3(point, pass_actor)
    }
}

#[derive(Clone)]
struct PredictionQ3Hooks {
    movement_only: bool,
    weapons: WeaponDriver,
    actor: OwnedActor,
}

impl PredictionQ3Hooks {
    fn passthrough(context: &Q3HookContext) -> Q3AnimationStepResult {
        Q3AnimationStepResult {
            animation: context.animation.clone(),
            effects: Vec::new(),
        }
    }
}

impl Q3MovementHooks for PredictionQ3Hooks {
    fn firing(&mut self, context: &Q3HookContext) -> bool {
        context.command.buttons & 1 != 0 && context.motion.health > 0.0
    }

    fn animation(&mut self, request: Q3AnimationRequest, context: &Q3HookContext) -> Q3AnimationStepResult {
        if !self.movement_only && matches!(context.animation.state, AnimationState::Q3 { .. }) {
            match q3_source_animation(request, context) {
                Ok(step) => step,
                Err(error) => {
                    fail(&self.weapons.error, PredictError::Animation(error));
                    Self::passthrough(context)
                }
            }
        } else {
            Self::passthrough(context)
        }
    }

    fn weapon(&mut self, context: &Q3HookContext) -> Q3WeaponPhaseResult {
        let mut command = context.input.command;
        command.buttons = context.command.buttons;
        command.weapon = context.command.weapon;
        let (arsenal, animation, effects) = self.weapons.step(
            &self.actor,
            &UserCommand::Q3(command),
            &context.frame,
            &context.arsenal,
            &context.animation,
        );
        let movement_flags = match self.weapons.runtime.borrow().clone() {
            None => context.motion.pm_flags,
            Some(runtime) => {
                (context.motion.pm_flags & !(RESPAWNED | USE_ITEM_HELD))
                    | if runtime.respawned { RESPAWNED } else { 0 }
                    | if runtime.use_item_held { USE_ITEM_HELD } else { 0 }
            }
        };
        Q3WeaponPhaseResult {
            arsenal,
            animation,
            effects,
            continuation: None,
            movement_flags,
        }
    }

    fn torso(&mut self, context: &Q3HookContext) -> Q3AnimationStepResult {
        if !self.movement_only && matches!(context.animation.state, AnimationState::Q3 { .. }) {
            match q3_source_torso(11, context, true) {
                Ok(step) => step,
                Err(error) => {
                    fail(&self.weapons.error, PredictError::Animation(error));
                    Self::passthrough(context)
                }
            }
        } else {
            Self::passthrough(context)
        }
    }
}

fn state_dialect(state: &MovementState) -> MovementDialect {
    match state {
        MovementState::Q1Netquake(_) => MovementDialect::Q1Netquake,
        MovementState::Q1Quakeworld(_) => MovementDialect::Q1Quakeworld,
        MovementState::Q2Classic(_) => MovementDialect::Q2Classic,
        MovementState::Q2Rerelease(_) => MovementDialect::Q2Rerelease,
        MovementState::Q3(_) => MovementDialect::Q3,
    }
}

fn check_error(error: &ErrorCell) -> Result<(), PredictError> {
    if let Some(failure) = error.borrow_mut().take() {
        return Err(failure);
    }
    Ok(())
}

fn finish<Contact>(
    source: &MovementPredictionSnapshot,
    entry: &PredictionCommand,
    movement_only: bool,
    fields: MovementResultFields<Contact>,
    runtime: &RuntimeCell,
) -> PredictedCommand {
    let (arsenal, animation) = if movement_only {
        (source.arsenal.clone(), source.animation.clone())
    } else {
        (fields.arsenal.clone(), fields.animation.clone())
    };
    PredictedCommand {
        player: MovementPredictionSnapshot {
            sequence: entry.sequence,
            command_time_milliseconds: entry.time_milliseconds,
            state: source.state.clone(),
            arsenal,
            animation,
            environment: source.environment,
            bounds: fields.bounds,
            view_angles: fields.view_angles,
            view_height: fields.view_height,
            view_offset: source.view_offset,
            contact: Some(PredictionContact {
                ground: fields.ground.clone(),
                water_level: fields.water_level,
                water_type: fields.water_type,
            }),
            q3_arsenal: runtime.borrow().clone(),
        },
        effects: fields.effects.clone(),
    }
}

/// One command operates only on copied player state and read-only collision
/// queries.
pub fn predict_movement_command(
    configuration: &MovementProbeOptions,
    snapshot: &MovementPredictionSnapshot,
    entry: &PredictionCommand,
    options: &PredictionStepOptions,
    shape: TraceShape,
) -> Result<PredictedCommand, PredictError> {
    let source = copy_prediction_snapshot(snapshot);
    let profile = configuration.profile.clone();
    let mut command = entry.command;
    if entry.angle_space == Some(CommandAngleSpace::Absolute) {
        command = relative_movement_command(entry, &source.state).command;
    }
    if !configuration.movement_only
        && matches!(source.arsenal.state, WeaponState::Q3 { .. })
        && source.q3_arsenal.is_some()
    {
        let runtime = source.q3_arsenal.clone().unwrap();
        let controls = resolve_q3_arsenal_controls(&source.arsenal, entry.arsenal.as_ref(), &command, runtime.product)?;
        if matches!(command, UserCommand::Q3(_)) && entry.arsenal.is_some() {
            let UserCommand::Q3(current) = command else {
                unreachable!("q3 command checked above");
            };
            command = UserCommand::Q3(Q3UserCommand {
                weapon: controls.requested_weapon,
                buttons: (current.buttons & !4) | if controls.use_holdable { 4 } else { 0 },
                ..current
            });
        }
    } else if !configuration.movement_only && entry.arsenal.is_some() {
        return Err(PredictError::MissingArsenalOwner);
    }
    let environment = if matches!(source.state, MovementState::Q3(_)) {
        MovementEnvironment {
            gravity_multiplier: 1.0,
            ..source.environment
        }
    } else {
        source.environment
    };
    let duration = match command {
        UserCommand::Q1Netquake(_) | UserCommand::Q3(_) => {
            (entry.time_milliseconds - source.command_time_milliseconds).max(0.0)
        }
        UserCommand::Q1Quakeworld(command) => f64::from(command.milliseconds),
        UserCommand::Q2Classic(command) => f64::from(command.milliseconds),
        UserCommand::Q2Rerelease(command) => f64::from(command.milliseconds),
    };
    let source_seconds = matches!(
        profile,
        MovementPredictionProfile::Q1Netquake(_) | MovementPredictionProfile::Q1Quakeworld(_)
    );
    let frame = FrameContext {
        frame: entry.sequence as i32,
        phase: FramePhase::ClientCommand,
        time: if source_seconds {
            SourceTime::Seconds((entry.time_milliseconds / 1000.0) as f32)
        } else {
            SourceTime::Milliseconds(entry.time_milliseconds as i32)
        },
        elapsed: if source_seconds {
            SourceTime::Seconds((duration / 1000.0) as f32)
        } else {
            SourceTime::Milliseconds(duration as i32)
        },
    };
    let runtime: RuntimeCell = Rc::new(RefCell::new(source.q3_arsenal.clone()));
    let error: ErrorCell = Rc::new(RefCell::new(None));
    let numeric = NumericOps::select(profile.numeric())?;
    let weapons = WeaponDriver {
        movement_only: configuration.movement_only,
        intent: entry.arsenal.clone(),
        environment,
        gauntlet_hit: options.gauntlet_hit,
        runtime: runtime.clone(),
        error: error.clone(),
    };
    let animation = AnimationDriver {
        movement_only: configuration.movement_only,
        source_health: source.environment.health,
        runtime: runtime.clone(),
        error: error.clone(),
    };
    let fields = MovementInputFields {
        actor: configuration.actor.clone(),
        command_sequence: entry.sequence as i32,
        frame,
        shape,
        current_bounds: Some(source.bounds),
        environment,
        arsenal: source.arsenal.clone(),
        animation: source.animation.clone(),
        execution: MovementExecution::Prediction,
    };
    let q1_options = || Q1MovementOptions {
        view_height: Some(configuration.standing_view_height),
        hooks: Some(PredictionQ1Hooks {
            is_brush: configuration.is_brush.clone(),
        }),
        ..Default::default()
    };
    match (&profile, &source.state, command) {
        (
            MovementPredictionProfile::Q1Netquake(profile),
            MovementState::Q1Netquake(state),
            UserCommand::Q1Netquake(command),
        ) => {
            let provider = create_q1_movement_provider(profile.id.clone(), q1_options());
            let input = Q1MovementInput {
                fields,
                command,
                state: state.clone(),
                profile: profile.clone(),
            };
            let mut services = PredictionQ1Services {
                scene: options.scene.clone(),
                numeric,
                weapons,
                animation,
            };
            match provider.move_step(input, &mut services)? {
                MovementOutcome::Active { fields: result, state } => {
                    check_error(&error)?;
                    let mut predicted = finish(&source, entry, configuration.movement_only, result, &runtime);
                    predicted.player.state = MovementState::Q1Netquake(state);
                    predicted.player.environment = environment;
                    Ok(predicted)
                }
                MovementOutcome::ActorRemoved { .. } => Err(PredictError::ActorRemoved),
            }
        }
        (
            MovementPredictionProfile::Q1Quakeworld(profile),
            MovementState::Q1Quakeworld(state),
            UserCommand::Q1Quakeworld(command),
        ) => {
            let shared = environment
                .client_outputs
                .as_ref()
                .is_some_and(|outputs| outputs.stance.is_some());
            let input = QwMovementInput {
                fields: MovementInputFields {
                    shape: if shared { TraceShape::Box(source.bounds) } else { shape },
                    ..fields
                },
                command,
                state: state.clone(),
                profile: profile.clone(),
            };
            let mut services = PredictionQ1Services {
                scene: options.scene.clone(),
                numeric,
                weapons,
                animation,
            };
            let outcome = if shared {
                let postures = configuration.postures.postures(
                    &source.animation.state,
                    configuration.standing_bounds,
                    configuration.standing_view_height,
                );
                create_shared_quake_world_movement(
                    profile.id.clone(),
                    q1_options(),
                    configuration.standing_bounds,
                    &postures,
                    None::<fn(Bounds, f64)>,
                )
                .move_step(input, &mut services)?
            } else {
                create_qw_movement_provider(profile.id.clone(), q1_options()).move_step(input, &mut services)?
            };
            match outcome {
                MovementOutcome::Active { fields: result, state } => {
                    check_error(&error)?;
                    let mut predicted = finish(&source, entry, configuration.movement_only, result, &runtime);
                    predicted.player.state = MovementState::Q1Quakeworld(state);
                    predicted.player.environment = environment;
                    Ok(predicted)
                }
                MovementOutcome::ActorRemoved { .. } => Err(PredictError::ActorRemoved),
            }
        }
        (
            MovementPredictionProfile::Q2Classic(profile),
            MovementState::Q2Classic(state),
            UserCommand::Q2Classic(command),
        ) => {
            let provider = create_q2_classic_movement_provider(profile.id.clone());
            let input = Q2MovementInput {
                fields,
                command,
                state: *state,
                profile: Q2MovementProfile {
                    snap_initial: false,
                    ..profile.clone()
                },
            };
            let mut services = PredictionQ2Services {
                scene: options.scene.clone(),
                numeric,
            };
            match provider.move_step(input, &mut services)? {
                MovementOutcome::Active {
                    fields: mut result,
                    state,
                } => {
                    check_error(&error)?;
                    if matches!(result.arsenal.state, WeaponState::Q3 { .. }) {
                        let (arsenal, animation, effects) = weapons.step(
                            &configuration.actor,
                            &UserCommand::Q2Classic(command),
                            &frame,
                            &result.arsenal,
                            &result.animation,
                        );
                        let previous = result.effects.len();
                        result
                            .effects
                            .extend(
                                effects
                                    .into_iter()
                                    .enumerate()
                                    .map(|(index, effect)| OrderedMovementEffect {
                                        effect,
                                        substep: 0,
                                        sequence: previous + index,
                                        time: frame.time,
                                    }),
                            );
                        result.arsenal = arsenal;
                        result.animation = animation;
                    }
                    check_error(&error)?;
                    let mut predicted = finish(&source, entry, configuration.movement_only, result, &runtime);
                    predicted.player.state = MovementState::Q2Classic(state);
                    predicted.player.environment = environment;
                    Ok(predicted)
                }
                MovementOutcome::ActorRemoved { .. } => Err(PredictError::ActorRemoved),
            }
        }
        (
            MovementPredictionProfile::Q2Rerelease(profile),
            MovementState::Q2Rerelease(state),
            UserCommand::Q2Rerelease(command),
        ) => {
            let context = options.rerelease_movement.replace(Q2RereleaseMovementContext::new());
            let mut provider = create_q2_rerelease_movement_provider(profile.id.clone(), context);
            let input = Q2RereleaseMovementInput {
                fields,
                command,
                state: *state,
                profile: profile.clone(),
                view_offset: source.view_offset,
                snap_initial: options.first_command,
            };
            let mut services = PredictionQ2Services {
                scene: options.scene.clone(),
                numeric,
            };
            let outcome = provider.move_step(input, &mut services);
            options.rerelease_movement.replace(provider.context);
            match outcome? {
                Q2RereleaseMovementResult::Active {
                    fields: mut result,
                    state,
                    ..
                } => {
                    check_error(&error)?;
                    if matches!(result.arsenal.state, WeaponState::Q3 { .. }) {
                        let (arsenal, animation, effects) = weapons.step(
                            &configuration.actor,
                            &UserCommand::Q2Rerelease(command),
                            &frame,
                            &result.arsenal,
                            &result.animation,
                        );
                        let previous = result.effects.len();
                        result
                            .effects
                            .extend(
                                effects
                                    .into_iter()
                                    .enumerate()
                                    .map(|(index, effect)| OrderedMovementEffect {
                                        effect,
                                        substep: 0,
                                        sequence: previous + index,
                                        time: frame.time,
                                    }),
                            );
                        result.arsenal = arsenal;
                        result.animation = animation;
                    }
                    check_error(&error)?;
                    let mut predicted = finish(&source, entry, configuration.movement_only, result, &runtime);
                    predicted.player.state = MovementState::Q2Rerelease(state);
                    predicted.player.environment = environment;
                    Ok(predicted)
                }
                Q2RereleaseMovementResult::ActorRemoved { .. } => Err(PredictError::ActorRemoved),
            }
        }
        (MovementPredictionProfile::Q3(profile), MovementState::Q3(state), UserCommand::Q3(command)) => {
            let standing_bounds = configuration.standing_bounds;
            let standing_view_height = configuration.standing_view_height;
            let postures = Rc::new(move |input: &Q3MovementInput| -> Q3Postures {
                if matches!(input.fields.animation.state, AnimationState::Q3 { .. }) {
                    Q3_SOURCE_POSTURES
                } else {
                    Q3Postures {
                        standing_view_height,
                        crouched: qa_world::movement::q3::types::Q3Posture {
                            bounds: Bounds {
                                min: standing_bounds.min,
                                max: Vec3 {
                                    z: 4.0,
                                    ..standing_bounds.max
                                },
                            },
                            view_height: -2.0,
                        },
                        dead: qa_world::movement::q3::types::Q3Posture {
                            bounds: Bounds {
                                min: standing_bounds.min,
                                max: Vec3 {
                                    z: -8.0,
                                    ..standing_bounds.max
                                },
                            },
                            view_height: -16.0,
                        },
                        invulnerability_expanded: Q3_SOURCE_POSTURES.invulnerability_expanded,
                    }
                }
            });
            let trace_policy = options.trace_mask.map(|mask| {
                Rc::new(move |_input: &Q3MovementInput| Q3TracePolicy {
                    contents_mask: mask,
                    curves: true,
                    player_curve_clip: true,
                }) as Rc<dyn Fn(&Q3MovementInput) -> Q3TracePolicy>
            });
            let hooks = PredictionQ3Hooks {
                movement_only: configuration.movement_only,
                weapons,
                actor: configuration.actor.clone(),
            };
            let provider = create_q3_movement_provider(qa_world::movement::q3::provider::Q3MovementProviderOptions {
                id: profile.id.clone(),
                hooks,
                postures,
                trace_policy,
                diagnostics: None,
            });
            let input = Q3MovementInput {
                fields,
                command,
                state: state.clone(),
                profile: qa_world::movement::q3::types::Q3MovementProfile {
                    fixed_milliseconds: options.fixed_milliseconds,
                    no_footsteps: options.no_footsteps,
                    ..profile.clone()
                },
            };
            let mut services = PredictionQ3Services {
                scene: options.scene.clone(),
                numeric,
            };
            match provider.move_step(input, &mut services)? {
                MovementOutcome::Active { fields: result, state } => {
                    check_error(&error)?;
                    let mut predicted = finish(&source, entry, configuration.movement_only, result, &runtime);
                    predicted.player.state = MovementState::Q3(state);
                    predicted.player.environment = environment;
                    Ok(predicted)
                }
                MovementOutcome::ActorRemoved { .. } => Err(PredictError::ActorRemoved),
            }
        }
        _ => Err(PredictError::DialectMismatch {
            command: format!("{:?}", command.dialect()),
            state: format!("{:?}", state_dialect(&source.state)),
            profile: format!("{:?}", profile.dialect()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;
    use qa_core::math::vec3;
    use qa_world::movement::q1::types::{QwMovementProfile, QwMovementState};
    use qa_world::movement::q2::types::Q2RereleaseMovementProfile;
    use qa_world::movement::types::{ActorAnimationState, AnimationState, ArsenalState, QwUserCommand, WeaponState};
    use qa_world::movement::Q1MovementParameters;

    use super::super::test_support::{empty_scene, standing_bounds, stub_postures, test_actor, test_recipe};
    use super::*;

    fn qw_probe() -> MovementProbeOptions {
        MovementProbeOptions {
            movement_only: false,
            actor: test_actor(),
            recipe: test_recipe(),
            profile: MovementPredictionProfile::Q1Quakeworld(QwMovementProfile {
                id: ProviderId::new("sim", "test"),
                clock: qa_core::time::ClockProfile::Q1Quakeworld {
                    maximum_command_milliseconds: 50.0,
                },
                numeric: qa_core::numeric::Q1_DONOR_PROFILE,
                parameters: Q1MovementParameters {
                    gravity: 800.0,
                    stop_speed: 100.0,
                    max_speed: 320.0,
                    spectator_max_speed: 500.0,
                    accelerate: 10.0,
                    air_accelerate: 1.0,
                    water_accelerate: 10.0,
                    friction: 4.0,
                    water_friction: 4.0,
                    entity_gravity: 1.0,
                },
            }),
            standing_bounds: standing_bounds(),
            standing_view_height: 22.0,
            scene: empty_scene(),
            is_brush: Rc::new(|_| false),
            postures: stub_postures(),
        }
    }

    fn qw_snapshot() -> MovementPredictionSnapshot {
        MovementPredictionSnapshot {
            sequence: 0,
            command_time_milliseconds: 0.0,
            state: MovementState::Q1Quakeworld(QwMovementState {
                origin: vec3(0.0, 0.0, 0.0),
                velocity: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
                old_buttons: 0,
                water_jump_time_seconds: 0.0,
                dead: false,
                spectator: 0,
                ground: TraceHit::None,
            }),
            arsenal: ArsenalState {
                provider: ProviderId::new("sim", "test"),
                active_weapon: None,
                state: WeaponState::Q1 {
                    frame: 0,
                    attack_finished_seconds: 0.0,
                    source_weapon: 0,
                },
                ammo: Vec::new(),
            },
            animation: ActorAnimationState {
                provider: ProviderId::new("sim", "test"),
                state: AnimationState::Q1 {
                    frame: 0,
                    next_frame_seconds: 0.0,
                },
            },
            environment: MovementEnvironment::default(),
            bounds: standing_bounds(),
            view_angles: vec3(0.0, 0.0, 0.0),
            view_height: 22.0,
            view_offset: vec3(0.0, 0.0, 22.0),
            contact: None,
            q3_arsenal: None,
        }
    }

    fn step_options(scene: PredictionSceneHandle) -> PredictionStepOptions {
        PredictionStepOptions {
            scene,
            rerelease_movement: Rc::new(RefCell::new(Q2RereleaseMovementContext::new())),
            fixed_milliseconds: None,
            no_footsteps: false,
            gauntlet_hit: false,
            trace_mask: None,
            first_command: true,
        }
    }

    #[test]
    fn copies_roundtrip() {
        let snapshot = qw_snapshot();
        assert_eq!(copy_prediction_snapshot(&snapshot), snapshot);
        assert_eq!(copy_movement_state(&snapshot.state), snapshot.state);
    }

    #[test]
    fn rejects_dialect_mismatches() {
        let options = qw_probe();
        let snapshot = qw_snapshot();
        let entry = PredictionCommand {
            angle_space: None,
            sequence: 1,
            time_milliseconds: 50.0,
            command: UserCommand::Q3(Q3UserCommand {
                server_time_milliseconds: 50,
                angle_words: [0, 0, 0],
                buttons: 0,
                weapon: 1,
                forward_move: 0,
                right_move: 0,
                up_move: 0,
            }),
            arsenal: None,
        };
        let error = predict_movement_command(
            &options,
            &snapshot,
            &entry,
            &step_options(options.scene.clone()),
            TraceShape::Box(standing_bounds()),
        )
        .unwrap_err();
        assert!(matches!(error, PredictError::DialectMismatch { .. }));
    }

    #[test]
    fn rejects_intent_without_an_arsenal_owner() {
        let options = qw_probe();
        let snapshot = qw_snapshot();
        let entry = PredictionCommand {
            angle_space: None,
            sequence: 1,
            time_milliseconds: 50.0,
            command: UserCommand::Q1Quakeworld(QwUserCommand {
                milliseconds: 50,
                angles: vec3(0.0, 0.0, 0.0),
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0,
                impulse: 0,
            }),
            arsenal: Some(ArsenalIntent {
                provider: "sim:test".to_string(),
                weapon: None,
                use_holdable: false,
            }),
        };
        let error = predict_movement_command(
            &options,
            &snapshot,
            &entry,
            &step_options(options.scene.clone()),
            TraceShape::Box(standing_bounds()),
        )
        .unwrap_err();
        assert_eq!(error, PredictError::MissingArsenalOwner);
    }

    #[test]
    fn predicts_a_quakeworld_command() {
        let options = qw_probe();
        let snapshot = qw_snapshot();
        let entry = PredictionCommand {
            angle_space: None,
            sequence: 1,
            time_milliseconds: 50.0,
            command: UserCommand::Q1Quakeworld(QwUserCommand {
                milliseconds: 50,
                angles: vec3(0.0, 0.0, 0.0),
                forward_move: 320.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0,
                impulse: 0,
            }),
            arsenal: None,
        };
        let predicted = predict_movement_command(
            &options,
            &snapshot,
            &entry,
            &step_options(options.scene.clone()),
            TraceShape::Box(standing_bounds()),
        )
        .unwrap();
        assert_eq!(predicted.player.sequence, 1);
        assert_eq!(predicted.player.command_time_milliseconds, 50.0);
        assert!(predicted.player.contact.is_some());
        match predicted.player.state {
            MovementState::Q1Quakeworld(state) => {
                assert!(state.origin.x != 0.0 || state.origin.y != 0.0 || state.velocity.x != 0.0);
            }
            _ => panic!("family changed"),
        }
    }

    #[test]
    fn rerelease_profile_reports_through_probe_options() {
        let profile = MovementPredictionProfile::Q2Rerelease(Q2RereleaseMovementProfile {
            id: ProviderId::new("sim", "test"),
            clock: qa_core::time::ClockProfile::Q2Rerelease {
                frame_milliseconds: 16.0,
            },
            numeric: qa_core::numeric::Q2_DONOR_PROFILE,
            air_accelerate: 0.0,
            n64_physics: false,
        });
        assert_eq!(profile.dialect(), MovementDialect::Q2Rerelease);
    }
}
