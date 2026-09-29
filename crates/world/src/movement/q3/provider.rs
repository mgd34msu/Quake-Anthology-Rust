//! Quake III movement provider: input application, locomotion, effects.
//!
//! Donor provenance: `src/movement/q3/provider.ts` (`move`,
//! `createQ3MovementProvider`, `q3Command`). The provider projects the
//! source state into locomotion work state, runs [`move_player`](super::pmove::move_player)
//! with a driver that applies input callbacks, stance projection, hooks,
//! and diagnostics at the original call sites, then folds the result back
//! into a movement outcome.

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::identity::ProviderId;
use qa_core::math::{vec3, Bounds};
use qa_core::numeric::Arithmetic;
use qa_core::time::{FrameContext, SourceTime};

use super::super::client_outputs::{client_movement_mode, client_movement_type, client_stance_command};
use super::super::types::{
    ActorAnimationState, ArsenalState, MovementContinuation, MovementDialect, MovementEffect, MovementError,
    MovementExecution, MovementInputContinuation, MovementOutcome, MovementResultFields, OrderedMovementEffect,
    PredictableMovementEvent, Q3UserCommand, TraceHit, TraceShape, UserCommand,
};
use super::constants::{move_flags as F, move_type};
use super::pmove::move_player;
use super::types::{
    Q3AnimationRequest, Q3AnimationStepResult, Q3Command, Q3HookContext, Q3Motion, Q3MotionDriver, Q3MotionOptions,
    Q3MovementContact, Q3MovementHooks, Q3MovementInput, Q3MovementResult, Q3MovementServices, Q3MovementState,
    Q3Postures, Q3Trace, Q3TraceQuery, Q3WeaponPhaseResult,
};

/// Decode a source command into locomotion words.
#[must_use]
pub fn q3_command(command: &Q3UserCommand) -> Q3Command {
    Q3Command {
        server_time: command.server_time_milliseconds,
        angles: vec3(
            command.angle_words[0] as f32,
            command.angle_words[1] as f32,
            command.angle_words[2] as f32,
        ),
        buttons: command.buttons,
        weapon: command.weapon,
        forwardmove: command.forward_move,
        rightmove: command.right_move,
        upmove: command.up_move,
    }
}

/// Q3 trace policy: curve handling plus the contents mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3TracePolicy {
    /// BSP curves enabled.
    pub curves: bool,
    /// Player curve clipping enabled.
    pub player_curve_clip: bool,
    /// Contents mask.
    pub contents_mask: i32,
}

/// Locomotion diagnostics sink.
#[derive(Clone)]
pub struct Q3Diagnostics {
    /// Print level (zero disables output).
    pub level: i32,
    /// Print callback.
    pub print: Rc<RefCell<dyn FnMut(&str)>>,
}

/// Null movement hooks: nothing fires, phases pass snapshots through.
#[derive(Debug, Clone, Default)]
pub struct NoQ3Hooks;

impl Q3MovementHooks for NoQ3Hooks {
    fn firing(&mut self, _context: &Q3HookContext) -> bool {
        false
    }

    fn animation(&mut self, _request: Q3AnimationRequest, context: &Q3HookContext) -> Q3AnimationStepResult {
        Q3AnimationStepResult {
            animation: context.animation.clone(),
            effects: Vec::new(),
        }
    }

    fn weapon(&mut self, context: &Q3HookContext) -> Q3WeaponPhaseResult {
        Q3WeaponPhaseResult {
            arsenal: context.arsenal.clone(),
            animation: context.animation.clone(),
            effects: Vec::new(),
            continuation: None,
            movement_flags: context.motion.pm_flags,
        }
    }

    fn torso(&mut self, context: &Q3HookContext) -> Q3AnimationStepResult {
        Q3AnimationStepResult {
            animation: context.animation.clone(),
            effects: Vec::new(),
        }
    }
}

/// Quake III movement provider options.
#[derive(Clone)]
pub struct Q3MovementProviderOptions<H = NoQ3Hooks> {
    /// Provider identity.
    pub id: ProviderId,
    /// Selected arsenal and character adapters.
    pub hooks: H,
    /// Character postures for the step.
    pub postures: Rc<dyn Fn(&Q3MovementInput) -> Q3Postures>,
    /// Trace policy override (default selects the spectator-aware mask).
    pub trace_policy: Option<Rc<dyn Fn(&Q3MovementInput) -> Q3TracePolicy>>,
    /// Locomotion diagnostics.
    pub diagnostics: Option<Q3Diagnostics>,
}

/// Quake III movement provider.
#[derive(Clone)]
pub struct Q3MovementProvider<H = NoQ3Hooks> {
    /// Provider identity.
    pub id: ProviderId,
    /// Movement options.
    pub options: Q3MovementProviderOptions<H>,
}

/// Build a Quake III movement provider.
pub fn create_q3_movement_provider<H>(options: Q3MovementProviderOptions<H>) -> Q3MovementProvider<H> {
    Q3MovementProvider {
        id: options.id.clone(),
        options,
    }
}

impl<H: Q3MovementHooks + Clone> Q3MovementProvider<H> {
    /// Run a Quake III step.
    pub fn move_step(
        &self,
        input: Q3MovementInput,
        services: &mut dyn Q3MovementServices,
    ) -> Result<Q3MovementResult, MovementError> {
        move_q3(input, services, self.options.clone())
    }
}

fn movement_speed(speed: f64, multiplier: f64) -> f64 {
    if multiplier == 1.0 {
        speed
    } else {
        f64::from((speed as f32 * multiplier as f32).trunc() as i32)
    }
}

struct ProviderDriver<'a, H> {
    id: ProviderId,
    hooks: H,
    input: Q3MovementInput,
    command: Q3Command,
    frame: FrameContext,
    source_state: Q3MovementState,
    mode_projected: bool,
    arsenal: ArsenalState,
    animation: ActorAnimationState,
    effects: Vec<OrderedMovementEffect>,
    substep: usize,
    removed: bool,
    application_open: bool,
    has_application: bool,
    failure: Option<MovementError>,
    services: Rc<RefCell<&'a mut dyn Q3MovementServices>>,
    diagnostics_count: i32,
    diagnostics: Option<Q3Diagnostics>,
}

impl<H: Q3MovementHooks> ProviderDriver<'_, H> {
    fn output_mode(&self) -> Option<super::super::types::ModClientMovementMode> {
        client_movement_mode(
            self.input.fields.environment.client_outputs.as_ref(),
            self.input.fields.environment.health,
        )
    }

    fn movement_state(&self, motion: &Q3Motion) -> Q3MovementState {
        let mut state = self.source_state.clone();
        state.command_time_milliseconds = motion.command_time;
        if !self.mode_projected {
            state.movement_type = motion.pm_type;
        }
        state.bob_cycle = motion.bob_cycle;
        state.movement_flags = motion.pm_flags;
        state.movement_time_milliseconds = motion.pm_time;
        state.origin = motion.origin;
        state.velocity = motion.velocity;
        state.delta_angle_words = [
            motion.delta_angles.x as i32,
            motion.delta_angles.y as i32,
            motion.delta_angles.z as i32,
        ];
        state.ground = motion.ground.clone();
        state.movement_direction = motion.movement_dir;
        state.flags = motion.e_flags;
        state.view_angles = motion.viewangles;
        state.view_height = motion.viewheight;
        state.movement_frame = motion.pmove_framecount;
        state.predictable_event_sequence = motion.event_sequence;
        state
    }

    fn resume(&mut self, state: Q3MovementState, motion: &mut Q3Motion) {
        self.source_state = state.clone();
        let mode = self.output_mode();
        self.mode_projected = mode.is_some();
        motion.command_time = state.command_time_milliseconds;
        motion.pm_type = mode.map_or(state.movement_type, |mode| {
            client_movement_type(MovementDialect::Q3, mode)
        });
        motion.bob_cycle = state.bob_cycle;
        motion.pm_flags = state.movement_flags;
        motion.pm_time = state.movement_time_milliseconds;
        motion.origin = state.origin;
        motion.velocity = state.velocity;
        motion.ground = state.ground.clone();
        motion.delta_angles = vec3(
            state.delta_angle_words[0] as f32,
            state.delta_angle_words[1] as f32,
            state.delta_angle_words[2] as f32,
        );
        motion.movement_dir = state.movement_direction;
        motion.e_flags = state.flags;
        motion.viewangles = state.view_angles;
        motion.viewheight = state.view_height;
        motion.pmove_framecount = state.movement_frame;
        motion.event_sequence = state.predictable_event_sequence;
        motion.grapple_point = state.grapple_point;
        motion.gravity = (state.gravity * self.input.fields.environment.gravity_multiplier).trunc() as f32;
        motion.speed = movement_speed(
            state.speed,
            self.input.fields.environment.speed_multiplier.unwrap_or(1.0),
        ) as f32;
    }

    fn context(&self, motion: &Q3Motion) -> Q3HookContext {
        Q3HookContext {
            input: Box::new(self.input.clone()),
            motion: motion.clone(),
            state: self.movement_state(motion),
            command: self.command,
            frame: self.frame.clone(),
            arsenal: self.arsenal.clone(),
            animation: self.animation.clone(),
        }
    }

    fn append(&mut self, effect: MovementEffect, motion: &mut Q3Motion) {
        let ordered = match effect {
            MovementEffect::Event(mut event) => {
                event.sequence = motion.event_sequence;
                motion.event_sequence = motion.event_sequence.wrapping_add(1);
                MovementEffect::Event(event)
            }
            effect => effect,
        };
        let sequence = self.effects.len();
        self.effects.push(OrderedMovementEffect {
            substep: self.substep,
            sequence,
            time: self.frame.time,
            effect: ordered,
        });
    }

    fn continued_user_command(&self, active_command: &Q3Command) -> Q3UserCommand {
        Q3UserCommand {
            server_time_milliseconds: active_command.server_time,
            angle_words: [
                active_command.angles.x as i32,
                active_command.angles.y as i32,
                active_command.angles.z as i32,
            ],
            buttons: active_command.buttons,
            weapon: active_command.weapon,
            forward_move: active_command.forwardmove,
            right_move: active_command.rightmove,
            up_move: active_command.upmove,
        }
    }

    fn close_application(&mut self, motion: &Q3Motion, failed: bool) -> MovementContinuation<Q3MovementState> {
        let state = self.movement_state(motion);
        self.services
            .borrow_mut()
            .input_application()
            .map(|application| application.end(state, failed))
            .expect("application present")
    }
}

impl<H: Q3MovementHooks> Q3MotionDriver for ProviderDriver<'_, H> {
    fn begin_step(&mut self, motion: &mut Q3Motion, active_command: &mut Q3Command, msec: i32, substep: usize) -> bool {
        if self.removed || self.failure.is_some() {
            return false;
        }
        self.command = *active_command;
        self.substep = substep;
        self.frame = FrameContext {
            time: SourceTime::Milliseconds(active_command.server_time),
            elapsed: SourceTime::Milliseconds(msec),
            ..self.input.fields.frame.clone()
        };
        if self.has_application {
            self.application_open = true;
            let rebuilt = self.continued_user_command(active_command);
            let state = self.movement_state(motion);
            let before = self
                .services
                .borrow_mut()
                .input_application()
                .map(|application| application.begin(UserCommand::Q3(rebuilt), &self.frame, state))
                .expect("application present");
            match before {
                MovementInputContinuation::ActorRemoved => {
                    self.removed = true;
                    return false;
                }
                MovementInputContinuation::Continue { state, command } => {
                    let UserCommand::Q3(command) = command else {
                        self.failure = Some(MovementError::Contract("Input output changed command dialect"));
                        return false;
                    };
                    *active_command = q3_command(&command);
                    self.resume(state, motion);
                }
            }
        }
        let rebuilt = self.continued_user_command(active_command);
        let stance = self
            .input
            .fields
            .environment
            .client_outputs
            .as_ref()
            .and_then(|outputs| outputs.stance);
        match client_stance_command(UserCommand::Q3(rebuilt), stance) {
            Ok(UserCommand::Q3(effective)) => *active_command = q3_command(&effective),
            _ => {
                self.failure = Some(MovementError::Contract("Client stance changed movement dialect"));
                return false;
            }
        }
        if let Some(mode) = self.output_mode() {
            self.mode_projected = true;
            motion.pm_type = client_movement_type(MovementDialect::Q3, mode);
        }
        if self.diagnostics.is_some() {
            self.diagnostics_count = self.diagnostics_count.wrapping_add(1);
        }
        true
    }

    fn end_step(&mut self, motion: &mut Q3Motion) -> bool {
        if !self.has_application {
            return true;
        }
        self.application_open = false;
        match self.close_application(motion, false) {
            MovementContinuation::ActorRemoved => {
                self.removed = true;
                false
            }
            MovementContinuation::Continue(state) => {
                self.resume(state, motion);
                true
            }
        }
    }

    fn event(&mut self, event: i32, motion: &mut Q3Motion) {
        self.append(
            MovementEffect::Event(PredictableMovementEvent {
                provider: self.id.clone(),
                sequence: motion.event_sequence,
                event,
                parameter: 0,
            }),
            motion,
        );
    }

    fn animation(&mut self, request: Q3AnimationRequest, motion: &mut Q3Motion) {
        let update = self.hooks.animation(request, &self.context(motion));
        self.animation = update.animation;
        for effect in update.effects {
            self.append(effect, motion);
        }
    }

    fn weapon(&mut self, motion: &mut Q3Motion) -> bool {
        let update = self.hooks.weapon(&self.context(motion));
        self.arsenal = update.arsenal;
        self.animation = update.animation;
        motion.pm_flags = update.movement_flags;
        for effect in update.effects {
            self.append(effect, motion);
        }
        match update.continuation {
            Some(MovementContinuation::ActorRemoved) => {
                self.removed = true;
                false
            }
            Some(MovementContinuation::Continue(state)) => {
                self.resume(state, motion);
                let weapon_flags = F::RESPAWNED | F::USE_ITEM_HELD;
                motion.pm_flags = (motion.pm_flags & !weapon_flags) | (update.movement_flags & weapon_flags);
                true
            }
            None => true,
        }
    }

    fn torso(&mut self, motion: &mut Q3Motion) {
        let update = self.hooks.torso(&self.context(motion));
        self.animation = update.animation;
        for effect in update.effects {
            self.append(effect, motion);
        }
    }

    fn firing(&mut self, motion: &mut Q3Motion) -> bool {
        self.hooks.firing(&self.context(motion))
    }

    fn contact(&mut self, trace: &Q3Trace) {
        if !matches!(trace.hit, TraceHit::None) {
            let substep = self.substep;
            let sequence = self.effects.len();
            let time = self.frame.time;
            // Touch effects never resequence; append without touching motion.
            self.effects.push(OrderedMovementEffect {
                substep,
                sequence,
                time,
                effect: MovementEffect::Touch {
                    target: trace.hit.clone(),
                    substep,
                },
            });
        }
    }

    fn debug(&mut self, message: &str) {
        if let Some(diagnostics) = self.diagnostics.as_ref() {
            if diagnostics.level != 0 {
                let count = self.diagnostics_count;
                let mut print = diagnostics.print.borrow_mut();
                print(&format!("{count}:{message}\n"));
            }
        }
    }
}

/// Run a Quake III step.
pub fn move_q3<H: Q3MovementHooks>(
    input: Q3MovementInput,
    services: &mut dyn Q3MovementServices,
    options: Q3MovementProviderOptions<H>,
) -> Result<Q3MovementResult, MovementError> {
    if !matches!(input.profile.numeric.arithmetic, Arithmetic::Binary32EachOp)
        || !matches!(services.numeric().profile.arithmetic, Arithmetic::Binary32EachOp)
    {
        return Err(MovementError::Contract(
            "The Q3 movement implementation requires binary32 operations",
        ));
    }
    let environment = input.fields.environment.clone();
    let initial_mode = client_movement_mode(environment.client_outputs.as_ref(), environment.health);
    let multiplier = environment.speed_multiplier.unwrap_or(1.0);
    let source = input.state.clone();
    let mut motion = Q3Motion {
        command_time: source.command_time_milliseconds,
        pm_type: initial_mode.map_or(source.movement_type, |mode| {
            client_movement_type(MovementDialect::Q3, mode)
        }),
        bob_cycle: source.bob_cycle,
        pm_flags: source.movement_flags,
        pm_time: source.movement_time_milliseconds,
        origin: source.origin,
        velocity: source.velocity,
        gravity: (source.gravity * environment.gravity_multiplier).trunc() as f32,
        speed: movement_speed(source.speed, multiplier) as f32,
        delta_angles: vec3(
            source.delta_angle_words[0] as f32,
            source.delta_angle_words[1] as f32,
            source.delta_angle_words[2] as f32,
        ),
        ground: source.ground.clone(),
        movement_dir: source.movement_direction,
        grapple_point: source.grapple_point,
        e_flags: source.flags,
        viewangles: source.view_angles,
        viewheight: source.view_height,
        pmove_framecount: source.movement_frame,
        event_sequence: source.predictable_event_sequence,
        actor: input.fields.actor.id().clone(),
        health: environment.health,
        flight: environment.flight,
        invulnerable: environment.invulnerable,
        product: input.profile.product,
    };
    let standing: Bounds = input.fields.shape.bounds().unwrap_or(Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(0.0, 0.0, 0.0),
    });
    let point_shape = matches!(input.fields.shape, TraceShape::Point);
    let selected_policy = options.trace_policy.as_ref().map_or(
        Q3TracePolicy {
            curves: true,
            player_curve_clip: true,
            contents_mask: 1
                | 0x10000
                | if source.movement_type == move_type::SPECTATOR {
                    0
                } else {
                    0x2000000
                },
        },
        |policy| policy(&input),
    );
    let shared: Rc<RefCell<&mut dyn Q3MovementServices>> = Rc::new(RefCell::new(services));
    let has_application =
        input.fields.execution == MovementExecution::Authoritative && shared.borrow_mut().input_application().is_some();
    let mut driver = ProviderDriver {
        id: options.id.clone(),
        hooks: options.hooks,
        command: q3_command(&input.command),
        frame: input.fields.frame.clone(),
        source_state: source,
        mode_projected: initial_mode.is_some(),
        arsenal: input.fields.arsenal.clone(),
        animation: input.fields.animation.clone(),
        effects: Vec::new(),
        substep: 0,
        removed: false,
        application_open: false,
        has_application,
        services: Rc::clone(&shared),
        diagnostics_count: 0,
        diagnostics: options.diagnostics.clone(),
        failure: None,
        input: input.clone(),
    };
    let postures = (options.postures)(&input);
    let command = q3_command(&input.command);
    let result = {
        let mut motion_options = Q3MotionOptions {
            pose: environment.pose,
            trace: Box::new({
                let shared = Rc::clone(&shared);
                let policy = selected_policy;
                move |start, end, bounds, pass_actor, mask| {
                    shared.borrow_mut().trace(Q3TraceQuery {
                        start,
                        end,
                        bounds,
                        point: point_shape,
                        pass_actor,
                        mask,
                        curves: policy.curves,
                        player_curve_clip: policy.player_curve_clip,
                    })
                }
            }),
            point_contents: Box::new({
                let shared = Rc::clone(&shared);
                move |point, pass_actor| shared.borrow_mut().point_contents(point, &pass_actor)
            }),
            standing_bounds: standing,
            current_bounds: input.fields.current_bounds,
            body_bounds: environment.client_outputs.and_then(|outputs| outputs.body_bounds),
            postures,
            trace_mask: selected_policy.contents_mask,
            fixed_msec: input.profile.fixed_milliseconds,
            no_footsteps: input.profile.no_footsteps,
            driver: &mut driver,
        };
        move_player(&mut motion, &command, &mut motion_options)
    };
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            if driver.application_open {
                driver.application_open = false;
                let _ = driver.close_application(&motion, true);
            }
            return Err(error);
        }
    };
    if let Some(failure) = driver.failure.take() {
        if driver.application_open {
            driver.application_open = false;
            let _ = driver.close_application(&motion, true);
        }
        return Err(failure);
    }
    if driver.application_open {
        driver.application_open = false;
        let _ = driver.close_application(&motion, false);
    }
    if driver.removed {
        return Ok(MovementOutcome::ActorRemoved {
            actor: input.fields.actor.id().clone(),
            command_sequence: input.fields.command_sequence,
            effects: driver.effects,
        });
    }
    let substep = driver.substep;
    let state = driver.movement_state(&motion);
    Ok(MovementOutcome::Active {
        fields: MovementResultFields {
            actor: input.fields.actor.id().clone(),
            command_sequence: input.fields.command_sequence,
            bounds: result.bounds,
            view_angles: motion.viewangles,
            view_height: motion.viewheight,
            ground: motion.ground.clone(),
            water_level: result.waterlevel,
            water_type: result.watertype,
            horizontal_speed: f64::from(result.xyspeed),
            contacts: result
                .contacts
                .into_iter()
                .map(|trace| Q3MovementContact {
                    target: trace.hit.clone(),
                    trace,
                    substep,
                })
                .collect(),
            effects: driver.effects,
            arsenal: driver.arsenal,
            animation: driver.animation,
        },
        state,
    })
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::Bounds;
    use qa_core::numeric::{NumericOps, Q3_BINARY32_PROFILE};
    use qa_core::time::{ClockProfile, FrameContext, FramePhase, SourceTime};

    use super::super::super::types::ModClientMovementOutputs;
    use super::super::super::types::{
        ActorAnimationState, AnimationState, ArsenalState, MovementEnvironment, MovementInputFields, WeaponState,
    };
    use super::*;

    struct NullServices {
        numeric: NumericOps,
    }

    impl Q3MovementServices for NullServices {
        fn numeric(&self) -> NumericOps {
            self.numeric
        }

        fn trace(&mut self, query: Q3TraceQuery) -> Q3Trace {
            Q3Trace {
                fraction: 1.0,
                end: query.end,
                start_solid: false,
                all_solid: false,
                contact: super::super::super::types::TraceContact::None,
                hit: TraceHit::None,
                contents: 0,
                surface_flags: 0,
                source_plane: crate::hull::BspPlane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                    plane_type: 0,
                    signbits: 0,
                },
            }
        }

        fn point_contents(&mut self, _point: qa_core::math::Vec3, _pass_actor: &qa_core::identity::ActorId) -> i32 {
            0
        }
    }

    fn movement_input(owner: &IdentityOwner, provider: &ProviderId) -> Q3MovementInput {
        let actor = owner.actor(1, 0);
        let owned = owner.owned_actor(&actor, provider.clone()).unwrap();
        Q3MovementInput {
            fields: MovementInputFields {
                actor: owned,
                command_sequence: 7,
                frame: FrameContext {
                    frame: 1,
                    time: SourceTime::Milliseconds(50),
                    elapsed: SourceTime::Milliseconds(8),
                    phase: FramePhase::EntityPhysics,
                },
                shape: TraceShape::Box(Bounds {
                    min: vec3(-15.0, -15.0, -24.0),
                    max: vec3(15.0, 15.0, 32.0),
                }),
                current_bounds: None,
                environment: MovementEnvironment::default(),
                arsenal: ArsenalState {
                    provider: provider.clone(),
                    active_weapon: None,
                    state: WeaponState::Q3 {
                        source_weapon: 1,
                        state: 0,
                        time_milliseconds: 0,
                    },
                    ammo: Vec::new(),
                },
                animation: ActorAnimationState {
                    provider: provider.clone(),
                    state: AnimationState::Q3 {
                        legs: 0,
                        torso: 0,
                        legs_timer_milliseconds: 0,
                        torso_timer_milliseconds: 0,
                    },
                },
                execution: MovementExecution::Authoritative,
            },
            command: Q3UserCommand {
                server_time_milliseconds: 50,
                angle_words: [0, 0, 0],
                buttons: 0,
                weapon: 1,
                forward_move: 127,
                right_move: 0,
                up_move: 0,
            },
            state: Q3MovementState {
                command_time_milliseconds: 0,
                movement_type: move_type::NORMAL,
                bob_cycle: 0,
                movement_flags: 0,
                movement_time_milliseconds: 0,
                origin: vec3(0.0, 0.0, 0.0),
                velocity: vec3(0.0, 0.0, 0.0),
                gravity: 800.0,
                speed: 320.0,
                delta_angle_words: [0, 0, 0],
                movement_direction: 0,
                grapple_point: vec3(0.0, 0.0, 0.0),
                flags: 0,
                view_angles: vec3(0.0, 0.0, 0.0),
                view_height: 26.0,
                ground: TraceHit::None,
                predictable_event_sequence: 0,
                jump_pad: None,
                movement_frame: 0,
                jump_pad_frame: 0,
            },
            profile: super::super::types::Q3MovementProfile {
                id: provider.clone(),
                clock: ClockProfile::Q3 {
                    server_frame_milliseconds: 16.0,
                    fixed_movement_milliseconds: None,
                },
                numeric: Q3_BINARY32_PROFILE,
                product: super::super::types::Q3Product::BaseQ3,
                fixed_milliseconds: None,
                no_footsteps: false,
            },
        }
    }

    fn test_options(owner: &ProviderId) -> Q3MovementProviderOptions<NoQ3Hooks> {
        Q3MovementProviderOptions {
            id: owner.clone(),
            hooks: NoQ3Hooks,
            postures: Rc::new(|_| super::super::postures::Q3_SOURCE_POSTURES),
            trace_policy: None,
            diagnostics: None,
        }
    }

    #[test]
    fn steps_through_locomotion() {
        let owner = IdentityOwner::create("q3-provider").unwrap();
        let provider = ProviderId::new("q3", "test");
        let input = movement_input(&owner, &provider);
        let options = test_options(&provider);
        let engine = create_q3_movement_provider(options);
        let mut services = NullServices {
            numeric: NumericOps::select(Q3_BINARY32_PROFILE).unwrap(),
        };
        let output = engine.move_step(input, &mut services).unwrap();
        match output {
            MovementOutcome::Active { fields, state } => {
                assert_eq!(fields.command_sequence, 7);
                assert_eq!(state.command_time_milliseconds, 50);
                assert!(state.velocity.x > 0.0);
                assert!(fields.horizontal_speed > 0.0);
            }
            MovementOutcome::ActorRemoved { .. } => panic!("unexpected removal"),
        }
    }

    #[test]
    fn rejects_non_binary32_numeric() {
        let owner = IdentityOwner::create("q3-provider").unwrap();
        let provider = ProviderId::new("q3", "test");
        let mut input = movement_input(&owner, &provider);
        input.profile.numeric = qa_core::numeric::Q1_DONOR_PROFILE;
        let options = test_options(&provider);
        let engine = create_q3_movement_provider(options);
        let mut services = NullServices {
            numeric: NumericOps::select(Q3_BINARY32_PROFILE).unwrap(),
        };
        assert!(matches!(
            engine.move_step(input, &mut services),
            Err(MovementError::Contract(_))
        ));
    }

    #[test]
    fn custom_trace_policy_masks_queries() {
        let owner = IdentityOwner::create("q3-provider").unwrap();
        let provider = ProviderId::new("q3", "test");
        let seen = Rc::new(RefCell::new(Vec::new()));
        struct PolicyServices {
            inner: NullServices,
            seen: Rc<RefCell<Vec<(i32, bool, bool)>>>,
        }
        impl Q3MovementServices for PolicyServices {
            fn numeric(&self) -> NumericOps {
                self.inner.numeric
            }
            fn trace(&mut self, query: Q3TraceQuery) -> Q3Trace {
                self.seen
                    .borrow_mut()
                    .push((query.mask, query.curves, query.player_curve_clip));
                self.inner.trace(query)
            }
            fn point_contents(&mut self, point: qa_core::math::Vec3, pass_actor: &qa_core::identity::ActorId) -> i32 {
                self.inner.point_contents(point, pass_actor)
            }
        }
        let mut input = movement_input(&owner, &provider);
        input.state.movement_type = move_type::SPECTATOR;
        let options = Q3MovementProviderOptions {
            id: provider.clone(),
            hooks: NoQ3Hooks,
            postures: Rc::new(|_| super::super::postures::Q3_SOURCE_POSTURES),
            trace_policy: Some(Rc::new(|_: &Q3MovementInput| Q3TracePolicy {
                curves: false,
                player_curve_clip: false,
                contents_mask: 0x10000,
            })),
            diagnostics: None,
        };
        let engine = create_q3_movement_provider(options);
        let mut services = PolicyServices {
            inner: NullServices {
                numeric: NumericOps::select(Q3_BINARY32_PROFILE).unwrap(),
            },
            seen: Rc::clone(&seen),
        };
        let output = engine.move_step(input, &mut services).unwrap();
        assert!(matches!(output, MovementOutcome::Active { .. }));
        let seen = seen.borrow();
        assert!(!seen.is_empty());
        assert!(seen
            .iter()
            .all(|(mask, curves, clip)| *mask == 0x10000 && !curves && !clip));
    }

    #[test]
    fn diagnostics_do_not_change_physics() {
        let owner = IdentityOwner::create("q3-provider").unwrap();
        let provider = ProviderId::new("q3", "test");
        let lines = Rc::new(RefCell::new(Vec::new()));
        let print_lines = Rc::clone(&lines);
        let run = |level: i32| {
            let print_lines = Rc::clone(&print_lines);
            let input = movement_input(&owner, &provider);
            let options = Q3MovementProviderOptions {
                id: provider.clone(),
                hooks: NoQ3Hooks,
                postures: Rc::new(|_| super::super::postures::Q3_SOURCE_POSTURES),
                trace_policy: None,
                diagnostics: Some(Q3Diagnostics {
                    level,
                    print: Rc::new(RefCell::new(move |message: &str| {
                        print_lines.borrow_mut().push(message.to_string());
                    })),
                }),
            };
            let engine = create_q3_movement_provider(options);
            let mut services = NullServices {
                numeric: NumericOps::select(Q3_BINARY32_PROFILE).unwrap(),
            };
            engine.move_step(input, &mut services).unwrap()
        };
        let quiet = run(0);
        assert!(lines.borrow().is_empty());
        let loud = run(1);
        assert_eq!(quiet, loud);
    }

    #[test]
    fn decodes_source_commands() {
        let command = Q3UserCommand {
            server_time_milliseconds: 120,
            angle_words: [1, 2, 3],
            buttons: 5,
            weapon: 2,
            forward_move: 10,
            right_move: -10,
            up_move: 0,
        };
        let decoded = q3_command(&command);
        assert_eq!(decoded.server_time, 120);
        assert_eq!(decoded.angles, vec3(1.0, 2.0, 3.0));
        assert_eq!(decoded.forwardmove, 10);
    }

    #[test]
    fn client_mode_projects_movement_type() {
        let owner = IdentityOwner::create("q3-provider").unwrap();
        let provider = ProviderId::new("q3", "test");
        let mut input = movement_input(&owner, &provider);
        input.fields.environment.client_outputs = Some(ModClientMovementOutputs {
            view_offset: None,
            mode: Some(super::super::super::types::ModClientMovementMode::Noclip),
            stance: None,
            body_bounds: None,
        });
        let options = test_options(&provider);
        let engine = create_q3_movement_provider(options);
        let mut services = NullServices {
            numeric: NumericOps::select(Q3_BINARY32_PROFILE).unwrap(),
        };
        let output = engine.move_step(input, &mut services).unwrap();
        match output {
            MovementOutcome::Active { state, .. } => {
                // The projected mode keeps the source movement type word.
                assert_eq!(state.movement_type, move_type::NORMAL);
            }
            MovementOutcome::ActorRemoved { .. } => panic!("unexpected removal"),
        }
    }
}
