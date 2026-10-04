//! Quake I shared movement kernels: math, context, lifecycle plumbing.
//!
//! Donor provenance: `src/movement/q1/common.ts` (derived from Quake
//! `sv_phys.c`, `sv_user.c` and QW `pmove.c`).

use qa_core::math::{angle_vectors, donor_angle_vectors, vec3, AngleVectors, Bounds, Vec3};
use qa_core::numeric::{Arithmetic, NumericOps};
use qa_core::time::SourceTime;

use super::super::types::{
    MovementContinuation, MovementEffect, MovementTouchContact, OrderedMovementEffect, TraceContact, TraceHit,
    TraceShape,
};
use super::types::{
    NoQ1Hooks, Q1LifecyclePhase, Q1MovementContact, Q1MovementHooks, Q1MovementOptions, Q1MovementServices,
    Q1PlayerInput, Q1State, Q1Trace, Q1TraceMove, Q1TraceQuery,
};

/// Zero vector.
pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
/// No-hit marker.
pub const NONE: TraceHit = TraceHit::None;
/// World hit marker for model zero.
pub const WORLD: TraceHit = TraceHit::World { model: 0 };

/// Bounds carried by a trace shape; points collapse to the zero box.
#[must_use]
pub fn shape_bounds(shape: &TraceShape) -> Bounds {
    match *shape {
        TraceShape::Point => Bounds { min: ZERO, max: ZERO },
        TraceShape::Box(bounds) | TraceShape::Capsule(bounds) => bounds,
    }
}

/// Source time in seconds.
#[must_use]
pub fn seconds(time: SourceTime) -> f64 {
    time.as_seconds_f64()
}

/// Profile-aware vector math, mirroring donor `MovementMath`.
#[derive(Debug, Clone, Copy)]
pub struct MovementMath {
    /// Numeric operations.
    pub n: NumericOps,
}

impl MovementMath {
    /// Bind operations.
    #[must_use]
    pub fn new(n: NumericOps) -> Self {
        Self { n }
    }

    /// Store a vector.
    #[must_use]
    pub fn vec(&self, x: f64, y: f64, z: f64) -> Vec3 {
        vec3(self.n.store(x), self.n.store(y), self.n.store(z))
    }

    /// Add two vectors.
    #[must_use]
    pub fn add(&self, a: Vec3, b: Vec3) -> Vec3 {
        self.vec(
            self.n.add(f64::from(a.x), f64::from(b.x)),
            self.n.add(f64::from(a.y), f64::from(b.y)),
            self.n.add(f64::from(a.z), f64::from(b.z)),
        )
    }

    /// Subtract two vectors.
    #[must_use]
    pub fn sub(&self, a: Vec3, b: Vec3) -> Vec3 {
        self.vec(
            self.n.sub(f64::from(a.x), f64::from(b.x)),
            self.n.sub(f64::from(a.y), f64::from(b.y)),
            self.n.sub(f64::from(a.z), f64::from(b.z)),
        )
    }

    /// Scale a vector.
    #[must_use]
    pub fn scale(&self, a: Vec3, b: f64) -> Vec3 {
        self.vec(
            self.n.mul(f64::from(a.x), b),
            self.n.mul(f64::from(a.y), b),
            self.n.mul(f64::from(a.z), b),
        )
    }

    /// Multiply-add: `a + b * c`.
    #[must_use]
    pub fn ma(&self, a: Vec3, b: f64, c: Vec3) -> Vec3 {
        self.vec(
            self.n.add(f64::from(a.x), self.n.mul(b, f64::from(c.x))),
            self.n.add(f64::from(a.y), self.n.mul(b, f64::from(c.y))),
            self.n.add(f64::from(a.z), self.n.mul(b, f64::from(c.z))),
        )
    }

    /// Dot product.
    #[must_use]
    pub fn dot(&self, a: Vec3, b: Vec3) -> f64 {
        self.n.add(
            self.n.add(
                self.n.mul(f64::from(a.x), f64::from(b.x)),
                self.n.mul(f64::from(a.y), f64::from(b.y)),
            ),
            self.n.mul(f64::from(a.z), f64::from(b.z)),
        )
    }

    /// Vector length.
    #[must_use]
    pub fn length(&self, a: Vec3) -> f64 {
        self.n.sqrt(self.dot(a, a))
    }

    /// Normalize, returning direction and length.
    #[must_use]
    pub fn normalize(&self, a: Vec3) -> (Vec3, f64) {
        let length = self.length(a);
        if length == 0.0 {
            (a, 0.0)
        } else {
            (self.scale(a, self.n.div(1.0, length)), length)
        }
    }

    /// Cross product.
    #[must_use]
    pub fn cross(&self, a: Vec3, b: Vec3) -> Vec3 {
        self.vec(
            self.n.sub(
                self.n.mul(f64::from(a.y), f64::from(b.z)),
                self.n.mul(f64::from(a.z), f64::from(b.y)),
            ),
            self.n.sub(
                self.n.mul(f64::from(a.z), f64::from(b.x)),
                self.n.mul(f64::from(a.x), f64::from(b.z)),
            ),
            self.n.sub(
                self.n.mul(f64::from(a.x), f64::from(b.y)),
                self.n.mul(f64::from(a.y), f64::from(b.x)),
            ),
        )
    }

    /// Angle vectors; donor-binary64 profiles use the donor path.
    #[must_use]
    pub fn angles(&self, value: Vec3) -> AngleVectors {
        if !matches!(self.n.profile.arithmetic, Arithmetic::DonorBinary64(_)) {
            return angle_vectors(value);
        }
        let mut forward = ZERO;
        let mut right = ZERO;
        let mut up = ZERO;
        donor_angle_vectors(value, Some(&mut forward), Some(&mut right), Some(&mut up));
        AngleVectors { forward, right, up }
    }

    /// Clip a velocity against a plane normal with the Q1 snap-to-zero.
    #[must_use]
    pub fn clip(&self, velocity: Vec3, normal: Vec3, overbounce: f64) -> Vec3 {
        super::super::clip_velocity_q1(velocity, normal, overbounce, &self.n)
    }

    /// Horizontal speed.
    #[must_use]
    pub fn horizontal(&self, a: Vec3) -> f64 {
        self.n.sqrt(self.n.add(
            self.n.mul(f64::from(a.x), f64::from(a.x)),
            self.n.mul(f64::from(a.y), f64::from(a.y)),
        ))
    }
}

/// Shared Q1 accelerate kernel. NetQuake and QuakeWorld differ only in their
/// guards and wish/axis selection; the add-speed gate, ground/air gain order,
/// and velocity apply step are one donor formula.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn q1_accelerate_apply(
    math: MovementMath,
    velocity: Vec3,
    axis: Vec3,
    wish: f64,
    speed: f64,
    accelerate: f64,
    frame_seconds: f64,
    air: bool,
) -> Vec3 {
    let n = math.n;
    let add = n.sub(wish, math.dot(velocity, axis));
    if add <= 0.0 {
        return velocity;
    }
    let gain = if air {
        n.mul(n.mul(accelerate, speed), frame_seconds)
    } else {
        n.mul(n.mul(accelerate, frame_seconds), speed)
    };
    math.ma(velocity, gain.min(add), axis)
}

/// Shared per-step movement context: contacts, effects, traces, lifecycle.
pub struct MovementContext<'s, S: Q1MovementServices, H: Q1MovementHooks = NoQ1Hooks> {
    /// Player input.
    pub input: Q1PlayerInput,
    /// Movement services.
    pub services: &'s mut S,
    /// Movement options.
    pub options: Q1MovementOptions<H>,
    /// Vector math.
    pub math: MovementMath,
    /// Recorded contacts.
    pub contacts: Vec<Q1MovementContact>,
    /// Ordered effects.
    pub effects: Vec<OrderedMovementEffect>,
    /// Removal flag.
    pub removed: bool,
    /// Substep index.
    pub substep: usize,
    source_mode: i32,
    owned_bounds: Option<Bounds>,
    mode_projected: bool,
}

impl<'s, S: Q1MovementServices, H: Q1MovementHooks> MovementContext<'s, S, H> {
    /// Build a context for one step.
    pub fn new(input: Q1PlayerInput, services: &'s mut S, options: Q1MovementOptions<H>) -> Self {
        let source_mode = match &input {
            Q1PlayerInput::Netquake(inner) => inner.state.move_type,
            Q1PlayerInput::Quakeworld(inner) => inner.state.spectator,
        };
        let math = MovementMath::new(services.numeric());
        Self {
            input,
            services,
            options,
            math,
            contacts: Vec::new(),
            effects: Vec::new(),
            removed: false,
            substep: 0,
            source_mode,
            owned_bounds: None,
            mode_projected: false,
        }
    }

    /// View height for water-level probes.
    #[must_use]
    pub fn view_height(&self) -> f64 {
        self.input
            .environment()
            .pose
            .map(|pose| pose.view_height)
            .or(self.options.view_height)
            .unwrap_or(22.0)
    }

    /// Mark the client mode as projected onto source state.
    pub fn project_client_mode(&mut self) {
        self.mode_projected = true;
    }

    /// Project the client mode onto source state when active.
    #[must_use]
    pub fn source_state(&self, state: Q1State) -> Q1State {
        if !self.mode_projected {
            return state;
        }
        match state {
            Q1State::Netquake(mut inner) => {
                inner.move_type = self.source_mode;
                Q1State::Netquake(inner)
            }
            Q1State::Quakeworld(mut inner) => {
                inner.spectator = self.source_mode;
                Q1State::Quakeworld(inner)
            }
        }
    }

    fn resumed(&mut self, state: Q1State) -> Q1State {
        match &state {
            Q1State::Netquake(inner) => self.source_mode = inner.move_type,
            Q1State::Quakeworld(inner) => self.source_mode = inner.spectator,
        }
        state
    }

    /// Apply the equipment speed multiplier.
    #[must_use]
    pub fn speed(&self, value: f64) -> f64 {
        let multiplier = self.input.environment().speed_multiplier.unwrap_or(1.0);
        if multiplier == 1.0 {
            value
        } else {
            self.math.n.mul(value, multiplier)
        }
    }

    /// Active trace shape: pose, hook override, owned bounds, or input shape.
    pub fn shape(&self) -> TraceShape {
        if let Some(pose) = self.input.environment().pose {
            return TraceShape::Box(pose.bounds);
        }
        let source = self
            .options
            .hooks
            .as_ref()
            .and_then(|hooks| hooks.shape())
            .unwrap_or_else(|| self.input.fields().shape);
        match (&source, self.owned_bounds) {
            (TraceShape::Point, _) | (_, None) => source,
            (TraceShape::Box(_), Some(bounds)) => TraceShape::Box(bounds),
            (TraceShape::Capsule(_), Some(bounds)) => TraceShape::Capsule(bounds),
        }
    }

    /// Accept or reject a client body-shape request.
    pub fn update_body_shape(&mut self, state: &Q1State) {
        let requested = self
            .input
            .environment()
            .client_outputs
            .and_then(|outputs| outputs.body_bounds);
        let Some(requested) = requested else {
            self.owned_bounds = None;
            return;
        };
        if self.input.environment().pose.is_some() {
            self.owned_bounds = None;
            return;
        }
        let source = self
            .options
            .hooks
            .as_ref()
            .and_then(|hooks| hooks.shape())
            .unwrap_or_else(|| self.input.fields().shape);
        let (TraceShape::Box(_) | TraceShape::Capsule(_)) = source else {
            panic!("A player body output requires a selected collision hull");
        };
        let previous = self.owned_bounds.unwrap_or_else(|| {
            self.input
                .fields()
                .current_bounds
                .unwrap_or_else(|| shape_bounds(&source))
        });
        let origin = match state {
            Q1State::Netquake(inner) => inner.origin,
            Q1State::Quakeworld(inner) => inner.origin,
        };
        // Same gate as movement_bounds, inlined because the clearance probe
        // needs the services borrow the closure cannot capture.
        let expands = requested.min.x < previous.min.x
            || requested.min.y < previous.min.y
            || requested.min.z < previous.min.z
            || requested.max.x > previous.max.x
            || requested.max.y > previous.max.y
            || requested.max.z > previous.max.z;
        let mut accepted = requested;
        if expands {
            let capsule = matches!(source, TraceShape::Capsule(_));
            let shape = if capsule {
                TraceShape::Capsule(requested)
            } else {
                TraceShape::Box(requested)
            };
            let trace = self.services.trace(Q1TraceQuery {
                start: origin,
                end: origin,
                shape,
                policy: Q1TraceMove::Normal,
            });
            if trace.start_solid || trace.all_solid {
                accepted = previous;
            }
        }
        self.owned_bounds = Some(accepted);
        if let Some(hooks) = self.options.hooks.as_mut() {
            hooks.body_shape(accepted);
        }
    }

    /// Bounds of the active shape.
    #[must_use]
    pub fn bounds(&self) -> Bounds {
        shape_bounds(&self.shape())
    }

    /// Run a trace with the active shape by default.
    pub fn trace(&mut self, start: Vec3, end: Vec3, shape: TraceShape, policy: Q1TraceMove) -> Q1Trace {
        self.services.trace(Q1TraceQuery {
            start,
            end,
            shape,
            policy,
        })
    }

    /// Run a trace with the active shape and normal policy.
    pub fn trace_active(&mut self, start: Vec3, end: Vec3) -> Q1Trace {
        let shape = self.shape();
        self.trace(start, end, shape, Q1TraceMove::Normal)
    }

    /// Q1 point contents with source currents collapsed to water.
    pub fn contents(&mut self, point: Vec3) -> i32 {
        let value = self.services.point_contents(point);
        if (-14..=-9).contains(&value) {
            -3
        } else {
            value
        }
    }

    /// Whether an origin is free of solid.
    pub fn position_free(&mut self, origin: Vec3) -> bool {
        let trace = self.trace_active(origin, origin);
        !trace.start_solid && !trace.all_solid
    }

    /// Record an ordered effect.
    pub fn effect(&mut self, effect: MovementEffect) {
        let sequence = self.effects.len();
        let time = self.input.frame().time;
        self.effects.push(OrderedMovementEffect {
            substep: self.substep,
            sequence,
            time,
            effect,
        });
    }

    /// Dispatch a touch contact.
    pub fn touch(&mut self, trace: Q1Trace, state: Q1State, record: bool) -> Q1State {
        if record {
            self.contacts.push(Q1MovementContact {
                target: trace.hit.clone(),
                trace: trace.clone(),
                substep: self.substep,
            });
        }
        if matches!(trace.hit, TraceHit::None) || self.removed {
            return state;
        }
        self.effect(MovementEffect::Touch {
            target: trace.hit.clone(),
            substep: self.substep,
        });
        let plane = match trace.contact {
            TraceContact::Plane(plane) => Some(plane),
            TraceContact::None => None,
        };
        let actor = self.input.actor().clone();
        let projected = self.source_state(state);
        let continuation = self.services.touch(
            MovementTouchContact {
                mover: actor,
                other: trace.hit.clone(),
                plane,
                surface: None,
            },
            projected,
        );
        match continuation {
            MovementContinuation::ActorRemoved => {
                self.removed = true;
                self.source_state_from_input()
            }
            MovementContinuation::Continue(state) => self.resumed(state),
        }
    }

    fn source_state_from_input(&self) -> Q1State {
        match &self.input {
            Q1PlayerInput::Netquake(inner) => Q1State::Netquake(inner.state.clone()),
            Q1PlayerInput::Quakeworld(inner) => Q1State::Quakeworld(inner.state.clone()),
        }
    }

    /// Link the actor through game hooks.
    pub fn link(&mut self, state: Q1State, touch_triggers: bool) -> Q1State {
        if self.removed || self.options.hooks.is_none() {
            return state;
        }
        let actor = self.input.actor().clone();
        let projected = self.source_state(state);
        let hooks = self.options.hooks.as_mut().expect("hooks present");
        match hooks.link(&actor, projected, touch_triggers) {
            MovementContinuation::ActorRemoved => {
                self.removed = true;
                self.source_state_from_input()
            }
            MovementContinuation::Continue(state) => self.resumed(state),
        }
    }

    /// Run a lifecycle phase through game hooks.
    pub fn lifecycle(&mut self, state: Q1State, phase: Q1LifecyclePhase) -> Q1State {
        if self.removed {
            return state;
        }
        if self.options.hooks.is_none() {
            if phase == Q1LifecyclePhase::BeforePhysics {
                self.update_body_shape(&state);
            }
            return state;
        }
        let input = self.input.clone();
        let projected = self.source_state(state);
        let hooks = self.options.hooks.as_mut().expect("hooks present");
        let continuation = match phase {
            Q1LifecyclePhase::BeforePhysics => hooks.before_physics(&input, projected),
            Q1LifecyclePhase::Think => {
                if hooks.has_think() {
                    hooks.think(&input, projected)
                } else {
                    return self.source_state_from_input_restore(projected);
                }
            }
            Q1LifecyclePhase::AfterPhysics => hooks.after_physics(&input, projected),
        };
        let result = match continuation {
            MovementContinuation::ActorRemoved => {
                self.removed = true;
                return self.source_state_from_input();
            }
            MovementContinuation::Continue(state) => self.resumed(state),
        };
        if phase == Q1LifecyclePhase::BeforePhysics {
            self.update_body_shape(&result);
        }
        result
    }

    fn source_state_from_input_restore(&mut self, state: Q1State) -> Q1State {
        self.resumed(state)
    }

    /// Source command for this step.
    #[must_use]
    pub fn input_command(&self) -> super::super::types::UserCommand {
        match &self.input {
            Q1PlayerInput::Netquake(input) => super::super::types::UserCommand::Q1Netquake(input.command),
            Q1PlayerInput::Quakeworld(input) => super::super::types::UserCommand::Q1Quakeworld(input.command),
        }
    }

    /// Whether a hit is BSP geometry.
    #[must_use]
    pub fn is_bsp(&self, hit: &TraceHit) -> bool {
        matches!(hit, TraceHit::World { .. }) || self.options.hooks.as_ref().is_some_and(|hooks| hooks.is_bsp(hit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::numeric::{NumericOps, Q1_DONOR_PROFILE};
    use qa_core::time::{FrameContext, FramePhase, SourceTime};

    use super::super::super::types::{
        ActorAnimationState, AnimationState, ArsenalState, MovementEnvironment, MovementExecution, MovementInputFields,
        UserCommand, WeaponState,
    };
    use super::super::types::{
        NoQ1Hooks, Q1AnimationStepInput, Q1AnimationStepResult, Q1MovementOptions, Q1MovementState, Q1WeaponStepInput,
        Q1WeaponStepResult, QwMovementState,
    };

    struct NullServices {
        ops: NumericOps,
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
                contact: TraceContact::None,
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
            -1
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
            Q1AnimationStepResult {
                animation: (*input.animation).clone(),
                effects: Vec::new(),
            }
        }
    }

    fn fixture_input(state: Q1State) -> Q1PlayerInput {
        let owner = IdentityOwner::create("q1-common").unwrap();
        let id = owner.actor(1, 0);
        let actor = owner.owned_actor(&id, ProviderId::new("q1", "test")).unwrap();
        let fields = MovementInputFields {
            actor,
            command_sequence: 1,
            frame: FrameContext {
                frame: 1,
                time: SourceTime::Seconds(0.0),
                elapsed: SourceTime::Seconds(0.05),
                phase: FramePhase::EntityPhysics,
            },
            shape: TraceShape::Box(Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            }),
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
        };
        match state {
            Q1State::Netquake(state) => Q1PlayerInput::Netquake(super::super::types::Q1MovementInput {
                fields,
                command: super::super::super::types::Q1UserCommand {
                    acknowledged_server_time_seconds: 0.0,
                    view_angles: vec3(0.0, 0.0, 0.0),
                    forward_move: 0.0,
                    side_move: 0.0,
                    up_move: 0.0,
                    buttons: 0,
                    impulse: 0,
                },
                state,
                profile: super::super::types::Q1MovementProfile {
                    id: ProviderId::new("q1", "test"),
                    clock: qa_core::time::ClockProfile::Q1Netquake {
                        minimum_frame_seconds: 0.001,
                        maximum_frame_seconds: 0.1,
                        fixed_frame_seconds: None,
                    },
                    numeric: Q1_DONOR_PROFILE,
                    edition: super::super::types::Q1Edition::Classic,
                    parameters: super::super::super::Q1MovementParameters {
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
                    edge_friction: 2.0,
                    no_clip_angle_hack: false,
                },
            }),
            Q1State::Quakeworld(_) => panic!("fixture covers netquake"),
        }
    }

    fn netquake_state() -> Q1State {
        Q1State::Netquake(Q1MovementState {
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            old_origin: vec3(0.0, 0.0, 0.0),
            angular_velocity: vec3(0.0, 0.0, 0.0),
            view_angles: vec3(0.0, 0.0, 0.0),
            punch_angles: vec3(0.0, 0.0, 0.0),
            move_type: 3,
            flags: 0,
            ground: TraceHit::None,
            water_level: 0,
            water_type: -1,
            teleport_time_seconds: 0.0,
            water_jump_direction: vec3(0.0, 0.0, 0.0),
            ideal_pitch: 0.0,
            fix_angle: false,
            health: 100.0,
        })
    }

    #[test]
    fn accelerate_kernel_gates_and_gains_like_both_donors() {
        let math = MovementMath::new(NumericOps::select(Q1_DONOR_PROFILE).unwrap());
        let axis = vec3(1.0, 0.0, 0.0);
        let resting = vec3(0.0, 0.0, 0.0);
        let ground = q1_accelerate_apply(math, resting, axis, 320.0, 320.0, 10.0, 0.05, false);
        assert!((f64::from(ground.x) - 160.0).abs() < 1e-6);
        let air = q1_accelerate_apply(math, resting, axis, 30.0, 320.0, 10.0, 0.05, true);
        assert!((f64::from(air.x) - 30.0).abs() < 1e-6);
        let fast = vec3(400.0, 0.0, 0.0);
        assert_eq!(
            q1_accelerate_apply(math, fast, axis, 320.0, 320.0, 10.0, 0.05, false),
            fast
        );
        assert_eq!(
            q1_accelerate_apply(math, fast, axis, 30.0, 320.0, 10.0, 0.05, true),
            fast
        );
    }

    #[test]
    fn math_add_scales_and_clips() {
        let math = MovementMath::new(NumericOps::select(Q1_DONOR_PROFILE).unwrap());
        assert_eq!(math.add(vec3(1.0, 2.0, 3.0), vec3(4.0, 5.0, 6.0)), vec3(5.0, 7.0, 9.0));
        assert_eq!(math.scale(vec3(1.0, 2.0, 3.0), 2.0), vec3(2.0, 4.0, 6.0));
        let clipped = math.clip(vec3(1.0, 0.05, -1.0), vec3(0.0, 0.0, 1.0), 1.0);
        assert_eq!(clipped, vec3(1.0, 0.0, 0.0));
        let (direction, length) = math.normalize(vec3(3.0, 4.0, 0.0));
        assert_eq!(length, 5.0);
        assert_eq!(direction, vec3(0.6, 0.8, 0.0));
        assert_eq!(math.horizontal(vec3(3.0, 4.0, 9.0)), 5.0);
    }

    #[test]
    fn lifecycle_without_hooks_only_updates_body_shape() {
        let mut services = NullServices {
            ops: NumericOps::select(Q1_DONOR_PROFILE).unwrap(),
        };
        let state = netquake_state();
        let mut context = MovementContext::new(
            fixture_input(state.clone()),
            &mut services,
            Q1MovementOptions::<NoQ1Hooks>::default(),
        );
        let out = context.lifecycle(state.clone(), Q1LifecyclePhase::BeforePhysics);
        assert_eq!(out, state);
        assert!(!context.removed);
    }

    #[test]
    fn touch_none_records_without_dispatch() {
        let mut services = NullServices {
            ops: NumericOps::select(Q1_DONOR_PROFILE).unwrap(),
        };
        let state = netquake_state();
        let mut context = MovementContext::new(
            fixture_input(state.clone()),
            &mut services,
            Q1MovementOptions::<NoQ1Hooks>::default(),
        );
        let trace = Q1Trace {
            fraction: 1.0,
            end: vec3(0.0, 0.0, 0.0),
            start_solid: false,
            all_solid: false,
            contact: TraceContact::None,
            hit: TraceHit::None,
            in_open: true,
            in_water: false,
            source_plane: qa_core::math::Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
            },
            surface_flags: None,
        };
        let out = context.touch(trace, state.clone(), true);
        assert_eq!(out, state);
        assert_eq!(context.contacts.len(), 1);
        assert!(context.effects.is_empty());
    }

    #[test]
    fn source_currents_collapse_to_water() {
        let mut services = NullServices {
            ops: NumericOps::select(Q1_DONOR_PROFILE).unwrap(),
        };
        let state = netquake_state();
        let mut context = MovementContext::new(
            fixture_input(state),
            &mut services,
            Q1MovementOptions::<NoQ1Hooks>::default(),
        );
        assert_eq!(context.contents(vec3(0.0, 0.0, 0.0)), -1);
        assert!(context.position_free(vec3(0.0, 0.0, 0.0)));
    }

    #[test]
    fn qw_state_shape_flows_through_input() {
        let mut services = NullServices {
            ops: NumericOps::select(Q1_DONOR_PROFILE).unwrap(),
        };
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
        let input = match fixture_input(netquake_state()) {
            Q1PlayerInput::Netquake(inner) => Q1PlayerInput::Quakeworld(super::super::types::QwMovementInput {
                fields: inner.fields,
                command: super::super::super::types::QwUserCommand {
                    milliseconds: 20,
                    angles: vec3(0.0, 0.0, 0.0),
                    forward_move: 0.0,
                    side_move: 0.0,
                    up_move: 0.0,
                    buttons: 0,
                    impulse: 0,
                },
                state: match state {
                    Q1State::Quakeworld(inner) => inner,
                    _ => panic!("qw"),
                },
                profile: super::super::types::QwMovementProfile {
                    id: ProviderId::new("q1", "test"),
                    clock: qa_core::time::ClockProfile::Q1Quakeworld {
                        maximum_command_milliseconds: 50.0,
                    },
                    numeric: Q1_DONOR_PROFILE,
                    parameters: super::super::super::Q1MovementParameters {
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
            }),
            Q1PlayerInput::Quakeworld(_) => panic!("netquake fixture"),
        };
        let context = MovementContext::new(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default());
        assert_eq!(context.view_height(), 22.0);
        let _ = UserCommand::Q1Netquake(super::super::super::types::Q1UserCommand {
            acknowledged_server_time_seconds: 0.0,
            view_angles: vec3(0.0, 0.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        });
    }
}
