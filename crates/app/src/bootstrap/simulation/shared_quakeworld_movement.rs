//! Shared QuakeWorld movement: stance and character posture over unchanged physics.
//!
//! Port of donor `src/app/bootstrap/simulation/shared-quakeworld-movement.ts`
//! (`createSharedQuakeWorldMovement`).
//!
//! The Rust movement framework is per-family instead of the donor provider
//! union, so this module is a [`SharedQuakeWorldMovement`] step wrapper around
//! the QuakeWorld provider rather than a provider object. Two adaptations
//! follow from that shape, both confined to mid-step hook paths:
//!
//! * Hooks cannot trace (the services live outside the hooks trait), so the
//!   `beforePhysics` posture re-evaluation shrinks immediately but defers
//!   expansion and stand-up to the next pre-step posture, which has services.
//!   When the inner hooks leave the command and state untouched the
//!   re-evaluation is an exact no-op, matching the donor.
//! * `viewHeight` is an options snapshot taken at delegation time instead of
//!   the donor's live getter, so mid-step posture changes reach physics on
//!   the following step.

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::identity::{OwnedActor, ProviderId};
use qa_core::math::{Bounds, Vec3};
use qa_world::collision::Q1Move;
use qa_world::movement::client_outputs::client_stance_command;
use qa_world::movement::movement_bounds;
use qa_world::movement::q1::quakeworld::create_qw_movement_provider;
use qa_world::movement::q1::types::{
    Q1AnimationStepInput, Q1AnimationStepResult, Q1MovementHooks, Q1MovementOptions, Q1MovementServices,
    Q1MovementSound, Q1MovementState, Q1PlayerActionKind, Q1PlayerInput, Q1State, Q1TraceQuery, QwMovementInput,
    QwMovementResult,
};
use qa_world::movement::q3::types::{Q3Posture, Q3Postures};
use qa_world::movement::types::{
    LocomotionAnimation, MovementContinuation, MovementError, QwUserCommand, TraceHit, TraceShape, UserCommand,
};

/// Posture geometry: standing hull and height plus the crouched posture.
#[derive(Debug, Clone, Copy)]
struct PostureGeometry {
    standing_bounds: Bounds,
    standing_view_height: f64,
    crouched: Q3Posture,
    capsule: bool,
}

/// Live posture state for one step.
#[derive(Debug, Clone, Copy)]
struct PostureState {
    bounds: Bounds,
    view_height: f64,
    crouched: bool,
    published: bool,
}

/// Posture inputs. The donor reads the outer step environment here, not the
/// mid-step hook input, so the pre-step values are shared with the hooks.
#[derive(Debug, Clone, Copy)]
struct PostureInputs {
    flight: bool,
    stance: Option<bool>,
    body_bounds: Option<Bounds>,
}

struct PostureTracker {
    geometry: PostureGeometry,
    state: PostureState,
    publish: Option<Rc<dyn Fn(Bounds, f64)>>,
}

impl PostureTracker {
    fn new(
        bounds: Bounds,
        options_view_height: Option<f64>,
        geometry: PostureGeometry,
        publish: Option<Rc<dyn Fn(Bounds, f64)>>,
    ) -> Self {
        Self {
            geometry,
            state: PostureState {
                bounds,
                view_height: options_view_height.unwrap_or(geometry.standing_view_height),
                crouched: bounds.max.z < geometry.standing_bounds.max.z,
                published: false,
            },
            publish,
        }
    }

    /// Donor `posture()`: stance request, stand-up clearance, hull easing,
    /// height selection, and change publication. The trace closure reports
    /// `(start_solid, all_solid)` for hull bounds at the step origin.
    fn apply(&mut self, up_move: f64, inputs: &PostureInputs, trace: &mut dyn FnMut(&Bounds) -> (bool, bool)) {
        let requested = !inputs.flight && up_move < 0.0;
        let mut crouched = requested;
        if !requested && self.state.bounds.max.z < self.geometry.standing_bounds.max.z {
            let (start_solid, all_solid) = trace(&self.geometry.standing_bounds);
            crouched = start_solid || all_solid;
        }
        let desired = inputs.body_bounds.unwrap_or(if crouched {
            self.geometry.crouched.bounds
        } else {
            self.geometry.standing_bounds
        });
        let previous = self.state.bounds;
        let probe = RefCell::new(trace);
        let next = movement_bounds(&previous, &desired, &|bounds| {
            let (start_solid, all_solid) = (*probe.borrow_mut())(bounds);
            !start_solid && !all_solid
        });
        if next != desired {
            crouched = previous.max.z < self.geometry.standing_bounds.max.z;
        }
        let height = if crouched {
            self.geometry.crouched.view_height
        } else {
            self.geometry.standing_view_height
        };
        if !self.state.published || self.state.bounds != next || self.state.view_height != height {
            self.state.bounds = next;
            self.state.view_height = height;
            self.state.published = true;
            if let Some(publish) = &self.publish {
                publish(next, height);
            }
        }
        self.state.crouched = crouched;
    }

    /// Hook-path posture without services: shrinks apply immediately while
    /// expansion and stand-up wait for the next pre-step posture.
    fn apply_cached(&mut self, up_move: f64, inputs: &PostureInputs) {
        self.apply(up_move, inputs, &mut |_| (true, false));
    }
}

fn effective_up_move(command: &QwUserCommand, stance: Option<bool>) -> f64 {
    let wrapped = UserCommand::Q1Quakeworld(*command);
    match client_stance_command(wrapped, stance) {
        Ok(UserCommand::Q1Quakeworld(effective)) => effective.up_move,
        _ => command.up_move,
    }
}

fn synthesize_jump_command(command: &QwUserCommand) -> QwUserCommand {
    if command.up_move > 0.0 {
        let mut next = *command;
        next.buttons |= 2;
        next
    } else {
        *command
    }
}

fn trace_shape(capsule: bool, bounds: Bounds) -> TraceShape {
    if capsule {
        TraceShape::Capsule(bounds)
    } else {
        TraceShape::Box(bounds)
    }
}

#[derive(Clone)]
struct SharedPostureHooks<H> {
    inner: Option<H>,
    tracker: Rc<RefCell<PostureTracker>>,
    inputs: PostureInputs,
}

impl<H: Q1MovementHooks + Clone> Q1MovementHooks for SharedPostureHooks<H> {
    fn qw_state(&mut self, water_level: i32, water_type: i32) {
        if let Some(inner) = self.inner.as_mut() {
            inner.qw_state(water_level, water_type);
        }
    }

    fn shape(&self) -> Option<TraceShape> {
        let tracker = self.tracker.borrow();
        Some(trace_shape(tracker.geometry.capsule, tracker.state.bounds))
    }

    fn body_shape(&mut self, bounds: Bounds) {
        {
            let mut tracker = self.tracker.borrow_mut();
            tracker.state.bounds = bounds;
            if let Some(publish) = &tracker.publish {
                publish(bounds, tracker.state.view_height);
            }
        }
        if let Some(inner) = self.inner.as_mut() {
            inner.body_shape(bounds);
        }
    }

    fn link(&mut self, actor: &OwnedActor, state: Q1State, touch_triggers: bool) -> MovementContinuation<Q1State> {
        if let Some(inner) = self.inner.as_mut() {
            inner.link(actor, state, touch_triggers)
        } else {
            MovementContinuation::Continue(state)
        }
    }

    fn is_bsp(&self, hit: &TraceHit) -> bool {
        self.inner
            .as_ref()
            .map_or(matches!(hit, TraceHit::World { .. }), |inner| inner.is_bsp(hit))
    }

    fn before_physics(&mut self, input: &Q1PlayerInput, state: Q1State) -> MovementContinuation<Q1State> {
        let result = if let Some(inner) = self.inner.as_mut() {
            inner.before_physics(input, state)
        } else {
            MovementContinuation::Continue(state)
        };
        if let (Q1PlayerInput::Quakeworld(next), MovementContinuation::Continue(Q1State::Quakeworld(_))) =
            (input, &result)
        {
            let up_move = effective_up_move(&next.command, self.inputs.stance);
            self.tracker.borrow_mut().apply_cached(up_move, &self.inputs);
        }
        result
    }

    fn think(&mut self, input: &Q1PlayerInput, state: Q1State) -> MovementContinuation<Q1State> {
        if let Some(inner) = self.inner.as_mut() {
            inner.think(input, state)
        } else {
            MovementContinuation::Continue(state)
        }
    }

    fn has_think(&self) -> bool {
        self.inner.as_ref().is_some_and(Q1MovementHooks::has_think)
    }

    fn after_physics(&mut self, input: &Q1PlayerInput, state: Q1State) -> MovementContinuation<Q1State> {
        if let Some(inner) = self.inner.as_mut() {
            inner.after_physics(input, state)
        } else {
            MovementContinuation::Continue(state)
        }
    }

    fn sound(&mut self, actor: &OwnedActor, sound: Q1MovementSound, state: &Q1MovementState) {
        if let Some(inner) = self.inner.as_mut() {
            inner.sound(actor, sound, state);
        }
    }

    fn player_action(&mut self, actor: &OwnedActor, action: Q1PlayerActionKind, state: &Q1MovementState) {
        if let Some(inner) = self.inner.as_mut() {
            inner.player_action(actor, action, state);
        }
    }
}

/// Services wrapper forcing the crouch locomotion while crouched.
struct CrouchAnimationServices<'a, S> {
    inner: &'a mut S,
    tracker: Rc<RefCell<PostureTracker>>,
}

impl<S: Q1MovementServices> Q1MovementServices for CrouchAnimationServices<'_, S> {
    fn numeric(&self) -> qa_core::numeric::NumericOps {
        self.inner.numeric()
    }

    fn trace(&mut self, query: Q1TraceQuery) -> qa_world::movement::q1::types::Q1Trace {
        self.inner.trace(query)
    }

    fn point_contents(&mut self, point: Vec3) -> i32 {
        self.inner.point_contents(point)
    }

    fn touch(
        &mut self,
        contact: qa_world::movement::types::MovementTouchContact,
        state: Q1State,
    ) -> MovementContinuation<Q1State> {
        self.inner.touch(contact, state)
    }

    fn weapon_step(
        &mut self,
        input: qa_world::movement::q1::types::Q1WeaponStepInput<'_>,
        state: &Q1State,
    ) -> qa_world::movement::q1::types::Q1WeaponStepResult {
        self.inner.weapon_step(input, state)
    }

    fn animation_step(&mut self, input: Q1AnimationStepInput<'_>) -> Q1AnimationStepResult {
        if self.tracker.borrow().state.crouched {
            self.inner.animation_step(Q1AnimationStepInput {
                locomotion: LocomotionAnimation::Crouch,
                ..input
            })
        } else {
            self.inner.animation_step(input)
        }
    }

    fn input_application(&mut self) -> Option<&mut dyn qa_world::movement::q1::types::Q1InputApplication> {
        self.inner.input_application()
    }
}

/// Shared controls and character stance around the unchanged QuakeWorld physics.
pub struct SharedQuakeWorldMovement<H> {
    id: ProviderId,
    options: Q1MovementOptions<H>,
    standing_bounds: Bounds,
    standing_view_height: f64,
    crouched: Q3Posture,
    publish: Option<Rc<dyn Fn(Bounds, f64)>>,
}

/// Shared controls and character stance surround the unchanged QuakeWorld physics.
pub fn create_shared_quake_world_movement<H>(
    id: ProviderId,
    options: Q1MovementOptions<H>,
    standing_bounds: Bounds,
    postures: &Q3Postures,
    publish_posture: Option<impl Fn(Bounds, f64) + 'static>,
) -> SharedQuakeWorldMovement<H> {
    SharedQuakeWorldMovement {
        id,
        options,
        standing_bounds,
        standing_view_height: postures.standing_view_height,
        crouched: postures.crouched,
        publish: publish_posture.map(|publish| Rc::new(publish) as Rc<dyn Fn(Bounds, f64)>),
    }
}

impl<H: Q1MovementHooks + Clone> SharedQuakeWorldMovement<H> {
    /// Run a shared QuakeWorld step.
    pub fn move_step<S: Q1MovementServices>(
        &self,
        mut input: QwMovementInput,
        services: &mut S,
    ) -> Result<QwMovementResult, MovementError> {
        if input.fields.environment.pose.is_some() || input.state.dead || input.state.spectator != 0 {
            return create_qw_movement_provider(self.id.clone(), self.options.clone()).move_step(input, services);
        }
        let capsule = matches!(input.fields.shape, TraceShape::Capsule(_));
        let bounds = input
            .fields
            .current_bounds
            .unwrap_or_else(|| input.fields.shape.bounds().unwrap_or(self.standing_bounds));
        let geometry = PostureGeometry {
            standing_bounds: self.standing_bounds,
            standing_view_height: self.standing_view_height,
            crouched: self.crouched,
            capsule,
        };
        let inputs = PostureInputs {
            flight: input.fields.environment.flight,
            stance: input
                .fields
                .environment
                .client_outputs
                .as_ref()
                .and_then(|outputs| outputs.stance),
            body_bounds: input
                .fields
                .environment
                .client_outputs
                .as_ref()
                .and_then(|outputs| outputs.body_bounds),
        };
        let tracker = Rc::new(RefCell::new(PostureTracker::new(
            bounds,
            self.options.view_height,
            geometry,
            self.publish.clone(),
        )));
        let origin = input.state.origin;
        let up_move = effective_up_move(&input.command, inputs.stance);
        {
            let mut trace = |hull: &Bounds| {
                let hit = services.trace(Q1TraceQuery {
                    start: origin,
                    end: origin,
                    shape: trace_shape(capsule, *hull),
                    policy: Q1Move::Normal,
                });
                (hit.start_solid, hit.all_solid)
            };
            tracker.borrow_mut().apply(up_move, &inputs, &mut trace);
        }
        let (step_bounds, view_height) = {
            let tracker = tracker.borrow();
            (tracker.state.bounds, tracker.state.view_height)
        };
        let options = Q1MovementOptions {
            source_punch_angles: self.options.source_punch_angles,
            view_height: Some(view_height),
            max_velocity: self.options.max_velocity,
            no_step: self.options.no_step,
            ideal_pitch_scale: self.options.ideal_pitch_scale,
            roll_speed: self.options.roll_speed,
            roll_angle: self.options.roll_angle,
            solid: self.options.solid,
            jump_authority: self.options.jump_authority,
            fix_angle_roll: self.options.fix_angle_roll,
            hooks: Some(SharedPostureHooks {
                inner: self.options.hooks.clone(),
                tracker: tracker.clone(),
                inputs,
            }),
        };
        input.command = synthesize_jump_command(&input.command);
        input.fields.current_bounds = Some(step_bounds);
        input.fields.shape = trace_shape(capsule, step_bounds);
        let mut wrapped = CrouchAnimationServices {
            inner: services,
            tracker,
        };
        create_qw_movement_provider(self.id.clone(), options).move_step(input, &mut wrapped)
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use qa_core::numeric::{NumericOps, Q1_DONOR_PROFILE};
    use qa_core::time::{ClockProfile, FrameContext, FramePhase, SourceTime};
    use qa_world::movement::q1::types::{
        Q1Trace, Q1WeaponStepInput, Q1WeaponStepResult, QwMovementProfile, QwMovementState,
    };
    use qa_world::movement::types::{
        ActorAnimationState, AnimationState, ArsenalState, FixedMovementPose, MovementEnvironment, MovementExecution,
        MovementInputFields, MovementTouchContact, WeaponState,
    };
    use qa_world::movement::Q1MovementParameters;

    use super::*;

    struct NullServices {
        ops: NumericOps,
        locomotion: Rc<RefCell<Vec<LocomotionAnimation>>>,
    }

    impl Q1MovementServices for NullServices {
        fn numeric(&self) -> NumericOps {
            self.ops
        }

        fn trace(&mut self, query: Q1TraceQuery) -> Q1Trace {
            Q1Trace {
                fraction: 1.0,
                end: query.end,
                start_solid: false,
                all_solid: false,
                contact: qa_world::movement::types::TraceContact::None,
                hit: TraceHit::None,
                in_open: true,
                in_water: false,
                source_plane: qa_core::math::Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                },
                surface_flags: None,
            }
        }

        fn point_contents(&mut self, _point: Vec3) -> i32 {
            0
        }

        fn touch(&mut self, _contact: MovementTouchContact, state: Q1State) -> MovementContinuation<Q1State> {
            MovementContinuation::Continue(state)
        }

        fn weapon_step(&mut self, input: Q1WeaponStepInput<'_>, _state: &Q1State) -> Q1WeaponStepResult {
            Q1WeaponStepResult {
                continuation: None,
                arsenal: (*input.arsenal).clone(),
                animation: (*input.animation).clone(),
                effects: Vec::new(),
            }
        }

        fn animation_step(&mut self, input: Q1AnimationStepInput<'_>) -> Q1AnimationStepResult {
            self.locomotion.borrow_mut().push(input.locomotion);
            Q1AnimationStepResult {
                animation: (*input.animation).clone(),
                effects: Vec::new(),
            }
        }
    }

    fn standing() -> Bounds {
        Bounds {
            min: vec3(-16.0, -16.0, -24.0),
            max: vec3(16.0, 16.0, 32.0),
        }
    }

    fn postures() -> Q3Postures {
        Q3Postures {
            standing_view_height: 22.0,
            crouched: Q3Posture {
                bounds: Bounds {
                    min: vec3(-16.0, -16.0, -24.0),
                    max: vec3(16.0, 16.0, 16.0),
                },
                view_height: 12.0,
            },
            dead: Q3Posture {
                bounds: Bounds {
                    min: vec3(-16.0, -16.0, -24.0),
                    max: vec3(16.0, 16.0, -8.0),
                },
                view_height: 8.0,
            },
            invulnerability_expanded: standing(),
        }
    }

    fn fixture() -> (QwMovementInput, NullServices) {
        let owner = IdentityOwner::create("shared-qw").unwrap();
        let id = owner.actor(1, 0);
        let actor = owner.owned_actor(&id, ProviderId::new("q1", "test")).unwrap();
        let input = QwMovementInput {
            fields: MovementInputFields {
                actor,
                command_sequence: 1,
                frame: FrameContext {
                    frame: 1,
                    time: SourceTime::Milliseconds(1000),
                    elapsed: SourceTime::Milliseconds(8),
                    phase: FramePhase::ClientCommand,
                },
                shape: TraceShape::Box(standing()),
                current_bounds: None,
                environment: MovementEnvironment::default(),
                arsenal: ArsenalState {
                    provider: ProviderId::new("q1", "test"),
                    active_weapon: None,
                    state: WeaponState::Q1 {
                        frame: 0,
                        attack_finished_seconds: 0.0,
                        source_weapon: 0,
                    },
                    ammo: Vec::new(),
                },
                animation: ActorAnimationState {
                    provider: ProviderId::new("q1", "test"),
                    state: AnimationState::Q1 {
                        frame: 0,
                        next_frame_seconds: 0.0,
                    },
                },
                execution: MovementExecution::Authoritative,
            },
            command: QwUserCommand {
                milliseconds: 8,
                angles: vec3(0.0, 90.0, 0.0),
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0,
                impulse: 0,
            },
            state: QwMovementState {
                origin: vec3(0.0, 0.0, 100.0),
                velocity: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 90.0, 0.0),
                old_buttons: 0,
                water_jump_time_seconds: 0.0,
                dead: false,
                spectator: 0,
                ground: TraceHit::World { model: 0 },
            },
            profile: QwMovementProfile {
                id: ProviderId::new("q1", "test"),
                clock: ClockProfile::Q1Quakeworld {
                    maximum_command_milliseconds: 50.0,
                },
                numeric: Q1_DONOR_PROFILE,
                parameters: Q1MovementParameters {
                    gravity: 800.0,
                    stop_speed: 100.0,
                    max_speed: 320.0,
                    spectator_max_speed: 500.0,
                    accelerate: 10.0,
                    air_accelerate: 1.0,
                    water_accelerate: 4.0,
                    friction: 4.0,
                    water_friction: 2.0,
                    entity_gravity: 1.0,
                },
            },
        };
        let services = NullServices {
            ops: NumericOps::select(Q1_DONOR_PROFILE).unwrap(),
            locomotion: Rc::new(RefCell::new(Vec::new())),
        };
        (input, services)
    }

    fn movement() -> SharedQuakeWorldMovement<qa_world::movement::q1::types::NoQ1Hooks> {
        create_shared_quake_world_movement(
            ProviderId::new("sim", "shared-qw"),
            Q1MovementOptions::default(),
            standing(),
            &postures(),
            None::<fn(Bounds, f64)>,
        )
    }

    #[test]
    fn pose_dead_and_spectator_skip_posture() {
        let mover = movement();
        for mutate in [
            |input: &mut QwMovementInput| {
                input.fields.environment.pose = Some(FixedMovementPose {
                    crouched: false,
                    bounds: standing(),
                    view_height: 22.0,
                });
            },
            |input: &mut QwMovementInput| input.state.dead = true,
            |input: &mut QwMovementInput| input.state.spectator = 1,
        ] {
            let (mut input, mut services) = fixture();
            mutate(&mut input);
            assert!(mover.move_step(input, &mut services).is_ok());
        }
    }

    #[test]
    fn stance_crouch_publishes_and_forces_crouch_locomotion() {
        let published = Rc::new(RefCell::new(Vec::new()));
        let sink = published.clone();
        let mover = create_shared_quake_world_movement(
            ProviderId::new("sim", "shared-qw"),
            Q1MovementOptions::<qa_world::movement::q1::types::NoQ1Hooks>::default(),
            standing(),
            &postures(),
            Some(move |bounds: Bounds, height: f64| sink.borrow_mut().push((bounds, height))),
        );
        let (mut input, mut services) = fixture();
        input.fields.environment.client_outputs = Some(qa_world::movement::types::ModClientMovementOutputs {
            view_offset: None,
            mode: None,
            stance: Some(true),
            body_bounds: None,
        });
        let locomotion = services.locomotion.clone();
        assert!(mover.move_step(input, &mut services).is_ok());
        let published = published.borrow();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].0, postures().crouched.bounds);
        assert_eq!(published[0].1, 12.0);
        assert!(locomotion
            .borrow()
            .iter()
            .all(|step| *step == LocomotionAnimation::Crouch));
        assert!(!locomotion.borrow().is_empty());
    }

    #[test]
    fn jump_command_synthesizes_jump_button() {
        assert_eq!(
            synthesize_jump_command(&QwUserCommand {
                milliseconds: 8,
                angles: vec3(0.0, 0.0, 0.0),
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 10.0,
                buttons: 0,
                impulse: 0,
            })
            .buttons,
            2
        );
        assert_eq!(
            synthesize_jump_command(&QwUserCommand {
                milliseconds: 8,
                angles: vec3(0.0, 0.0, 0.0),
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 1,
                impulse: 0,
            })
            .buttons,
            1
        );
    }

    #[test]
    fn stand_up_requires_clearance() {
        let geometry = PostureGeometry {
            standing_bounds: standing(),
            standing_view_height: 22.0,
            crouched: postures().crouched,
            capsule: false,
        };
        let inputs = PostureInputs {
            flight: false,
            stance: None,
            body_bounds: None,
        };
        let mut blocked = PostureTracker::new(postures().crouched.bounds, None, geometry, None);
        blocked.apply(0.0, &inputs, &mut |_| (true, false));
        assert!(blocked.state.crouched);
        let mut clear = PostureTracker::new(postures().crouched.bounds, None, geometry, None);
        clear.apply(0.0, &inputs, &mut |_| (false, false));
        assert!(!clear.state.crouched);
        assert_eq!(clear.state.bounds, standing());
        assert_eq!(clear.state.view_height, 22.0);
    }

    #[test]
    fn hook_posture_shrinks_but_defers_expansion() {
        let geometry = PostureGeometry {
            standing_bounds: standing(),
            standing_view_height: 22.0,
            crouched: postures().crouched,
            capsule: false,
        };
        let crouch_inputs = PostureInputs {
            flight: false,
            stance: Some(true),
            body_bounds: None,
        };
        let mut tracker = PostureTracker::new(standing(), None, geometry, None);
        tracker.apply_cached(-5.0, &crouch_inputs);
        assert!(tracker.state.crouched);
        assert_eq!(tracker.state.bounds, postures().crouched.bounds);
        let stand_inputs = PostureInputs {
            flight: false,
            stance: None,
            body_bounds: None,
        };
        tracker.apply_cached(0.0, &stand_inputs);
        assert!(tracker.state.crouched);
        assert_eq!(tracker.state.bounds, postures().crouched.bounds);
    }
}
