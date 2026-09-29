//! QuakeWorld player movement.
//!
//! Donor provenance: `src/movement/q1/quakeworld.ts` (ported from
//! QuakeWorld `client/pmove.c` and `server/sv_user.c`).

use qa_core::identity::ProviderId;
use qa_core::math::Vec3;
use qa_core::time::ClockProfile;

use super::super::client_outputs::{
    client_movement_mode, client_movement_type, client_stance_command,
};
use super::super::swept_body::{
    CollisionOriginalVelocity, CollisionPolicy, CreaseVelocity, SweepStop, SweptBodyServices,
    SweptBodyState, sweep_body,
};
use super::super::types::{
    MovementContinuation, MovementDialect, MovementError, MovementExecution,
    MovementInputContinuation, MovementOutcome, QwUserCommand, TraceHit, UserCommand,
};
use super::common::{MovementContext, NONE, ZERO};
use super::result::finish_quakeworld;
use super::types::{
    NoQ1Hooks, Q1_CONTENTS_EMPTY, Q1_CONTENTS_SLIME, Q1_CONTENTS_SOLID, Q1_CONTENTS_WATER,
    Q1_STEP_HEIGHT, Q1LifecyclePhase, Q1MovementHooks, Q1MovementOptions, Q1MovementServices,
    Q1State, Q1Trace, QwMovementInput, QwMovementResult, QwMovementState,
};

/// Split a command into slices of at most `maximum_milliseconds`, mirroring
/// donor `quakeWorldCommandSlices`. Later slices clear the impulse.
pub fn quake_world_command_slices(
    command: &QwUserCommand,
    maximum_milliseconds: f64,
) -> Result<Vec<QwUserCommand>, MovementError> {
    if maximum_milliseconds < 1.0 {
        return Err(MovementError::Range(
            "QuakeWorld command interval must be positive",
        ));
    }
    check_milliseconds(command.milliseconds)?;
    if f64::from(command.milliseconds) > maximum_milliseconds {
        let milliseconds = command.milliseconds / 2;
        let mut out = quake_world_command_slices(
            &QwUserCommand { milliseconds, ..*command },
            maximum_milliseconds,
        )?;
        out.extend(quake_world_command_slices(
            &QwUserCommand {
                milliseconds,
                impulse: 0,
                ..*command
            },
            maximum_milliseconds,
        )?);
        Ok(out)
    } else {
        Ok(vec![*command])
    }
}

fn check_milliseconds(milliseconds: i32) -> Result<(), MovementError> {
    if milliseconds < 0 || milliseconds > 255 {
        return Err(MovementError::Range(
            "QuakeWorld command milliseconds must fit its source byte",
        ));
    }
    Ok(())
}

struct QuakeWorldMove<'s, S: Q1MovementServices, H: Q1MovementHooks> {
    state: QwMovementState,
    context: MovementContext<'s, S, H>,
    frame_seconds: f64,
    forward: Vec3,
    right: Vec3,
    water_level: i32,
    water_type: i32,
    touched: Vec<TraceHit>,
    command: QwUserCommand,
}

impl<'s, S: Q1MovementServices, H: Q1MovementHooks> QuakeWorldMove<'s, S, H> {
    fn new(input: QwMovementInput, services: &'s mut S, options: Q1MovementOptions<H>) -> Self {
        let command = input.command;
        let state = input.state.clone();
        Self {
            state,
            context: MovementContext::new(
                super::types::Q1PlayerInput::Quakeworld(input),
                services,
                options,
            ),
            frame_seconds: 0.0,
            forward: ZERO,
            right: ZERO,
            water_level: 0,
            water_type: Q1_CONTENTS_EMPTY,
            touched: Vec::new(),
            command,
        }
    }

    fn set_state(&mut self, state: Q1State) -> Result<(), MovementError> {
        let Q1State::Quakeworld(state) = state else {
            return Err(MovementError::Contract(
                "QuakeWorld callback changed the movement provider during a command",
            ));
        };
        self.state = state;
        let environment = self.context.input.environment().clone();
        if let Some(mode) = client_movement_mode(environment.client_outputs.as_ref(), environment.health)
        {
            self.context.project_client_mode();
            self.state.spectator = client_movement_type(MovementDialect::Q1Quakeworld, mode);
        }
        Ok(())
    }

    fn record(&mut self, trace: &Q1Trace) {
        self.context.contacts.push(super::types::Q1MovementContact {
            trace: trace.clone(),
            target: trace.hit.clone(),
            substep: self.context.substep,
        });
    }

    fn fly_move(&mut self) -> i32 {
        struct Sweep<'a, 's, S: Q1MovementServices, H: Q1MovementHooks> {
            mover: &'a mut QuakeWorldMove<'s, S, H>,
            blocked: i32,
        }

        impl<S: Q1MovementServices, H: Q1MovementHooks> SweptBodyServices for Sweep<'_, '_, S, H> {
            type Trace = Q1Trace;

            fn read(&mut self) -> Option<SweptBodyState> {
                Some(SweptBodyState {
                    origin: self.mover.state.origin,
                    velocity: self.mover.state.velocity,
                })
            }
            fn write_origin(&mut self, origin: Vec3) {
                self.mover.state.origin = origin;
            }
            fn write_velocity(&mut self, velocity: Vec3) {
                self.mover.state.velocity = velocity;
            }
            fn trace(&mut self, start: Vec3, end: Vec3) -> Q1Trace {
                self.mover.context.trace_active(start, end)
            }
            fn normal(&mut self, trace: &Q1Trace) -> Vec3 {
                trace.source_plane.normal
            }
            fn impact(&mut self, trace: &Q1Trace, normal: Vec3) {
                self.mover.record(trace);
                if normal.z > 0.7 {
                    self.blocked |= 1;
                }
                if normal.z == 0.0 {
                    self.blocked |= 2;
                }
            }
            fn stop_when_still(&self) -> bool {
                false
            }
            fn collision_policy(&self) -> CollisionPolicy {
                CollisionPolicy {
                    stop_on_start_solid: true,
                    original_velocity: CollisionOriginalVelocity::Initial,
                    crease_velocity: CreaseVelocity::LastCandidate,
                    ..CollisionPolicy::default()
                }
            }
            fn same_plane(&mut self, _first: Vec3, _second: Vec3) -> bool {
                false
            }
            fn advance(&mut self, origin: Vec3, time: f64, velocity: Vec3) -> Vec3 {
                self.mover.context.math.ma(origin, time, velocity)
            }
            fn remaining(&mut self, time: f64, fraction: f64) -> f64 {
                let n = self.mover.context.math.n;
                n.sub(time, n.mul(time, fraction))
            }
            fn clip(&mut self, velocity: Vec3, normal: Vec3) -> Vec3 {
                self.mover.context.math.clip(velocity, normal, 1.0)
            }
            fn dot(&mut self, first: Vec3, second: Vec3) -> f64 {
                self.mover.context.math.dot(first, second)
            }
            fn cross(&mut self, first: Vec3, second: Vec3) -> Vec3 {
                self.mover.context.math.cross(first, second)
            }
            fn scale(&mut self, vector: Vec3, amount: f64) -> Vec3 {
                self.mover.context.math.scale(vector, amount)
            }
        }

        let primal = self.state.velocity;
        let mut sweep = Sweep { mover: self, blocked: 0 };
        let frame_seconds = sweep.mover.frame_seconds;
        let stop = sweep_body(&mut sweep, frame_seconds);
        let blocked = sweep.blocked;
        if stop == SweepStop::Solid {
            return 3;
        }
        if sweep.mover.state.water_jump_time_seconds != 0.0 {
            sweep.mover.state.velocity = primal;
        }
        blocked
    }

    fn ground_move(&mut self) {
        self.state.velocity = self.context.math.vec(
            f64::from(self.state.velocity.x),
            f64::from(self.state.velocity.y),
            0.0,
        );
        if self.state.velocity.x == 0.0 && self.state.velocity.y == 0.0 {
            return;
        }
        let destination = self.context.math.ma(
            self.state.origin,
            self.frame_seconds,
            self.state.velocity,
        );
        let trace = self.context.trace_active(self.state.origin, destination);
        if trace.fraction == 1.0 {
            self.state.origin = trace.end;
            return;
        }
        let original = self.state.origin;
        let original_velocity = self.state.velocity;
        self.fly_move();
        let down = self.state.origin;
        let down_velocity = self.state.velocity;
        self.state.origin = original;
        self.state.velocity = original_velocity;
        let up = self.context.math.add(
            self.state.origin,
            self.context.math.vec(0.0, 0.0, Q1_STEP_HEIGHT),
        );
        let trace = self.context.trace_active(self.state.origin, up);
        if !trace.start_solid && !trace.all_solid {
            self.state.origin = trace.end;
        }
        self.fly_move();
        let down_target = self.context.math.add(
            self.state.origin,
            self.context.math.vec(0.0, 0.0, -Q1_STEP_HEIGHT),
        );
        let trace = self.context.trace_active(self.state.origin, down_target);
        let mut use_down = trace.source_plane.normal.z < 0.7;
        if !use_down {
            if !trace.start_solid && !trace.all_solid {
                self.state.origin = trace.end;
            }
            let n = self.context.math.n;
            let down_delta = self.context.math.sub(down, original);
            let up_delta = self.context.math.sub(self.state.origin, original);
            let down_distance = n.add(
                n.mul(f64::from(down_delta.x), f64::from(down_delta.x)),
                n.mul(f64::from(down_delta.y), f64::from(down_delta.y)),
            );
            let up_distance = n.add(
                n.mul(f64::from(up_delta.x), f64::from(up_delta.x)),
                n.mul(f64::from(up_delta.y), f64::from(up_delta.y)),
            );
            use_down = down_distance > up_distance;
        }
        if use_down {
            self.state.origin = down;
            self.state.velocity = down_velocity;
        } else {
            self.state.velocity = self.context.math.vec(
                f64::from(self.state.velocity.x),
                f64::from(self.state.velocity.y),
                f64::from(down_velocity.z),
            );
        }
    }

    fn friction(&mut self) {
        if self.state.water_jump_time_seconds != 0.0 {
            return;
        }
        let parameters = self.qw_parameters();
        let speed = self.context.math.length(self.state.velocity);
        if speed < 1.0 {
            self.state.velocity = self.context.math.vec(
                0.0,
                0.0,
                f64::from(self.state.velocity.z),
            );
            return;
        }
        let mut friction = parameters.friction;
        if !matches!(self.state.ground, TraceHit::None) {
            let n = self.context.math.n;
            let origin = self.state.origin;
            let velocity = self.state.velocity;
            let bounds = self.context.bounds();
            let start = self.context.math.vec(
                n.add(
                    f64::from(origin.x),
                    n.mul(n.div(f64::from(velocity.x), speed), 16.0),
                ),
                n.add(
                    f64::from(origin.y),
                    n.mul(n.div(f64::from(velocity.y), speed), 16.0),
                ),
                n.add(f64::from(origin.z), f64::from(bounds.min.z)),
            );
            // QW uses the player hull here; NetQuake uses a point trace.
            let end = self.context.math.add(start, self.context.math.vec(0.0, 0.0, -34.0));
            if self.context.trace_active(start, end).fraction == 1.0 {
                friction = n.mul(friction, 2.0);
            }
        }
        let n = self.context.math.n;
        let mut drop = 0.0;
        if self.water_level >= 2 {
            drop = n.mul(
                n.mul(n.mul(speed, parameters.water_friction), self.water_level as f64),
                self.frame_seconds,
            );
        } else if !matches!(self.state.ground, TraceHit::None) {
            drop = n.mul(
                n.mul(speed.max(parameters.stop_speed), friction),
                self.frame_seconds,
            );
        }
        self.state.velocity = self.context.math.scale(
            self.state.velocity,
            n.div((0.0f64).max(n.sub(speed, drop)), speed),
        );
    }

    fn qw_parameters(&self) -> crate::movement::Q1MovementParameters {
        match &self.context.input {
            super::types::Q1PlayerInput::Quakeworld(input) => input.profile.parameters,
            _ => panic!("QuakeWorld mover with NetQuake input"),
        }
    }

    fn accelerate(&mut self, direction: Vec3, speed: f64, acceleration: f64, air: bool) {
        if self.state.dead || self.state.water_jump_time_seconds != 0.0 {
            return;
        }
        let n = self.context.math.n;
        let wish = if air { speed.min(30.0) } else { speed };
        let add = n.sub(wish, self.context.math.dot(self.state.velocity, direction));
        if add <= 0.0 {
            return;
        }
        let amount = if air {
            n.mul(n.mul(acceleration, speed), self.frame_seconds)
        } else {
            n.mul(n.mul(acceleration, self.frame_seconds), speed)
        }
        .min(add);
        self.state.velocity = self.context.math.ma(self.state.velocity, amount, direction);
    }

    fn wish_velocity(&mut self) -> Vec3 {
        let n = self.context.math.n;
        let command = self.command;
        let forward = self.context.speed(command.forward_move);
        let side = self.context.speed(command.side_move);
        self.context.math.vec(
            n.add(
                n.mul(f64::from(self.forward.x), forward),
                n.mul(f64::from(self.right.x), side),
            ),
            n.add(
                n.mul(f64::from(self.forward.y), forward),
                n.mul(f64::from(self.right.y), side),
            ),
            n.add(
                n.mul(f64::from(self.forward.z), forward),
                n.mul(f64::from(self.right.z), side),
            ),
        )
    }

    fn water_move(&mut self) {
        let parameters = self.qw_parameters();
        let n = self.context.math.n;
        let command = self.command;
        let mut wish = self.wish_velocity();
        let up = if command.forward_move == 0.0 && command.side_move == 0.0 && command.up_move == 0.0
        {
            -60.0
        } else {
            self.context.speed(command.up_move)
        };
        wish = self.context.math.vec(f64::from(wish.x), f64::from(wish.y), n.add(f64::from(wish.z), up));
        let normalized = self.context.math.normalize(wish);
        let max = self.context.speed(parameters.max_speed);
        let speed = n.mul(normalized.1.min(max), 0.7);
        self.accelerate(normalized.0, speed, parameters.water_accelerate, false);
        let destination = self.context.math.ma(
            self.state.origin,
            self.frame_seconds,
            self.state.velocity,
        );
        let start = self.context.math.add(
            destination,
            self.context.math.vec(0.0, 0.0, Q1_STEP_HEIGHT + 1.0),
        );
        let trace = self.context.trace_active(start, destination);
        if !trace.start_solid && !trace.all_solid {
            self.state.origin = trace.end;
            return;
        }
        self.fly_move();
    }

    fn air_move(&mut self) {
        let parameters = self.qw_parameters();
        let n = self.context.math.n;
        self.forward = self.context.math.normalize(self.context.math.vec(
            f64::from(self.forward.x),
            f64::from(self.forward.y),
            0.0,
        )).0;
        self.right = self.context.math.normalize(self.context.math.vec(
            f64::from(self.right.x),
            f64::from(self.right.y),
            0.0,
        )).0;
        let wish = self.wish_velocity();
        let normalized = self.context.math.normalize(wish);
        let max = self.context.speed(parameters.max_speed);
        let speed = normalized.1.min(max);
        let multiplier = self.context.input.environment().gravity_multiplier;
        let gravity = n.mul(
            n.mul(n.mul(parameters.entity_gravity, multiplier), parameters.gravity),
            self.frame_seconds,
        );
        if !matches!(self.state.ground, TraceHit::None) {
            self.state.velocity = self.context.math.vec(
                f64::from(self.state.velocity.x),
                f64::from(self.state.velocity.y),
                0.0,
            );
            self.accelerate(normalized.0, speed, parameters.accelerate, false);
            self.state.velocity = self.context.math.vec(
                f64::from(self.state.velocity.x),
                f64::from(self.state.velocity.y),
                n.sub(f64::from(self.state.velocity.z), gravity),
            );
            self.ground_move();
        } else {
            // Original pmove.c uses accelerate here; airaccelerate is transmitted but unused.
            self.accelerate(normalized.0, speed, parameters.accelerate, true);
            self.state.velocity = self.context.math.vec(
                f64::from(self.state.velocity.x),
                f64::from(self.state.velocity.y),
                n.sub(f64::from(self.state.velocity.z), gravity),
            );
            self.fly_move();
        }
    }

    fn categorize(&mut self) {
        let n = self.context.math.n;
        if f64::from(self.state.velocity.z) > 180.0 {
            self.state.ground = NONE;
        } else {
            let down = self.context.math.add(
                self.state.origin,
                self.context.math.vec(0.0, 0.0, -1.0),
            );
            let trace = self.context.trace_active(self.state.origin, down);
            self.state.ground = if trace.source_plane.normal.z < 0.7 {
                NONE
            } else {
                trace.hit.clone()
            };
            if !matches!(self.state.ground, TraceHit::None) {
                self.state.water_jump_time_seconds = 0.0;
                if !trace.start_solid && !trace.all_solid {
                    self.state.origin = trace.end;
                }
            }
            if matches!(trace.hit, TraceHit::Actor { .. }) {
                self.record(&trace);
            }
        }
        self.water_level = 0;
        self.water_type = Q1_CONTENTS_EMPTY;
        let bounds = self.context.bounds();
        let origin = self.state.origin;
        let mut point = self.context.math.vec(
            f64::from(origin.x),
            f64::from(origin.y),
            n.add(n.add(f64::from(origin.z), f64::from(bounds.min.z)), 1.0),
        );
        if self.context.contents(point) > Q1_CONTENTS_WATER {
            return;
        }
        let contents = self.context.contents(point);
        self.water_type = contents;
        self.water_level = 1;
        point = self.context.math.vec(
            f64::from(point.x),
            f64::from(point.y),
            n.add(
                f64::from(origin.z),
                n.mul(n.add(f64::from(bounds.min.z), f64::from(bounds.max.z)), 0.5),
            ),
        );
        if self.context.contents(point) > Q1_CONTENTS_WATER {
            return;
        }
        self.water_level = 2;
        let height = self.context.view_height();
        point = self.context.math.vec(
            f64::from(point.x),
            f64::from(point.y),
            n.add(f64::from(origin.z), height),
        );
        if self.context.contents(point) <= Q1_CONTENTS_WATER {
            self.water_level = 3;
        }
    }

    fn jump(&mut self) {
        if self.state.dead {
            self.state.old_buttons |= 2;
            return;
        }
        if self.state.water_jump_time_seconds != 0.0 {
            let n = self.context.math.n;
            self.state.water_jump_time_seconds = (0.0f64).max(n.sub(
                self.state.water_jump_time_seconds,
                self.frame_seconds,
            ));
            return;
        }
        if self.water_level >= 2 {
            self.state.ground = NONE;
            let vertical = if self.water_type == Q1_CONTENTS_WATER {
                100.0
            } else if self.water_type == Q1_CONTENTS_SLIME {
                80.0
            } else {
                50.0
            };
            self.state.velocity = self.context.math.vec(
                f64::from(self.state.velocity.x),
                f64::from(self.state.velocity.y),
                vertical,
            );
            return;
        }
        if matches!(self.state.ground, TraceHit::None) || self.state.old_buttons & 2 != 0 {
            return;
        }
        self.state.ground = NONE;
        let n = self.context.math.n;
        self.state.velocity = self.context.math.vec(
            f64::from(self.state.velocity.x),
            f64::from(self.state.velocity.y),
            n.add(f64::from(self.state.velocity.z), 270.0),
        );
        self.state.old_buttons |= 2;
    }

    fn check_water_jump(&mut self) {
        if self.state.water_jump_time_seconds != 0.0 || f64::from(self.state.velocity.z) < -180.0 {
            return;
        }
        let forward = self.context.math.normalize(self.context.math.vec(
            f64::from(self.forward.x),
            f64::from(self.forward.y),
            0.0,
        )).0;
        let spot = self.context.math.add(
            self.context.math.ma(self.state.origin, 24.0, forward),
            self.context.math.vec(0.0, 0.0, 8.0),
        );
        let high = self.context.math.add(spot, self.context.math.vec(0.0, 0.0, 24.0));
        if self.context.contents(spot) != Q1_CONTENTS_SOLID
            || self.context.contents(high) != Q1_CONTENTS_EMPTY
        {
            return;
        }
        let velocity = self.context.math.scale(forward, 50.0);
        self.state.velocity = self.context.math.vec(
            f64::from(velocity.x),
            f64::from(velocity.y),
            310.0,
        );
        self.state.water_jump_time_seconds = 2.0;
        self.state.old_buttons |= 2;
    }

    fn nudge(&mut self) {
        let base = self.state.origin;
        // Source loops use the unsnapped base, including the initial zero-offset try.
        for z in [0.0, -1.0, 1.0] {
            for x in [0.0, -1.0, 1.0] {
                for y in [0.0, -1.0, 1.0] {
                    self.state.origin = self.context.math.add(
                        base,
                        self.context.math.vec(x / 8.0, y / 8.0, z / 8.0),
                    );
                    if self.context.position_free(self.state.origin) {
                        return;
                    }
                }
            }
        }
        self.state.origin = base;
    }

    fn spectator_move(&mut self, collide: bool) {
        let parameters = self.qw_parameters();
        let n = self.context.math.n;
        let speed = self.context.math.length(self.state.velocity);
        if speed < 1.0 {
            self.state.velocity = ZERO;
        } else {
            let friction = n.mul(parameters.friction, 1.5);
            let drop = n.mul(
                n.mul(speed.max(parameters.stop_speed), friction),
                self.frame_seconds,
            );
            self.state.velocity = self.context.math.scale(
                self.state.velocity,
                n.div((0.0f64).max(n.sub(speed, drop)), speed),
            );
        }
        self.forward = self.context.math.normalize(self.forward).0;
        self.right = self.context.math.normalize(self.right).0;
        let mut wish = self.wish_velocity();
        wish = self.context.math.vec(
            f64::from(wish.x),
            f64::from(wish.y),
            n.add(f64::from(wish.z), self.context.speed(f64::from(self.command.up_move))),
        );
        let normalized = self.context.math.normalize(wish);
        let cap = if collide {
            self.context.speed(parameters.max_speed)
        } else {
            self.context.speed(parameters.spectator_max_speed)
        };
        let wish_speed = normalized.1.min(cap);
        let add = n.sub(wish_speed, self.context.math.dot(self.state.velocity, normalized.0));
        // This early return also skips origin integration in original SpectatorMove.
        if add <= 0.0 && !collide {
            return;
        }
        let acceleration =
            (0.0f64).max(n.mul(n.mul(parameters.accelerate, self.frame_seconds), wish_speed).min(add));
        self.state.velocity = self.context.math.ma(self.state.velocity, acceleration, normalized.0);
        if collide {
            self.fly_move();
        } else {
            self.state.origin = self.context.math.ma(
                self.state.origin,
                self.frame_seconds,
                self.state.velocity,
            );
        }
    }

    fn already_touched(&self, hit: &TraceHit) -> bool {
        self.touched.iter().any(|prior| match (prior, hit) {
            (TraceHit::World { model: a }, TraceHit::World { model: b }) => a == b,
            (TraceHit::Actor { actor: a }, TraceHit::Actor { actor: b }) => a == b,
            _ => false,
        })
    }

    fn step(&mut self, command: QwUserCommand) -> Result<(), MovementError> {
        let authoritative = self.context.input.fields().execution == MovementExecution::Authoritative;
        if !authoritative || self.context.services.input_application().is_none() {
            return self.step_physics(command);
        }
        let mut frame = self.context.input.frame().clone();
        frame.elapsed = qa_core::time::SourceTime::Milliseconds(command.milliseconds);
        let before = self
            .context
            .services
            .input_application()
            .expect("input application present")
            .begin(
                UserCommand::Q1Quakeworld(command),
                &frame,
                Q1State::Quakeworld(self.state.clone()),
            );
        match before {
            MovementInputContinuation::ActorRemoved => {
                self.context.removed = true;
            }
            MovementInputContinuation::Continue { state, command } => {
                let UserCommand::Q1Quakeworld(command) = command else {
                    self.context.services.input_application()
                        .expect("input application present")
                        .end(Q1State::Quakeworld(self.state.clone()), true);
                    return Err(MovementError::Contract("Input output changed command dialect"));
                };
                self.set_state(state)?;
                if let Err(error) = self.step_physics(command) {
                    self.context.services.input_application()
                        .expect("input application present")
                        .end(Q1State::Quakeworld(self.state.clone()), true);
                    return Err(error);
                }
            }
        }
        let after = self
            .context
            .services
            .input_application()
            .expect("input application present")
            .end(Q1State::Quakeworld(self.state.clone()), false);
        match after {
            MovementContinuation::ActorRemoved => self.context.removed = true,
            MovementContinuation::Continue(state) => self.set_state(state)?,
        }
        Ok(())
    }

    fn step_physics(&mut self, mut command: QwUserCommand) -> Result<(), MovementError> {
        self.command = command;
        let n = self.context.math.n;
        self.frame_seconds = n.mul(f64::from(command.milliseconds), 0.001);
        // Lifecycle hooks observe the sliced command and frame.
        let original = self.context.input.clone();
        if let super::types::Q1PlayerInput::Quakeworld(inner) = &mut self.context.input {
            inner.command = command;
            inner.fields.frame.elapsed = qa_core::time::SourceTime::Milliseconds(command.milliseconds);
        }
        let state = self.context.lifecycle(
            Q1State::Quakeworld(self.state.clone()),
            Q1LifecyclePhase::BeforePhysics,
        );
        self.context.input = original;
        self.set_state(state)?;
        if self.context.removed {
            return Ok(());
        }
        let stance = self.context.input.environment().client_outputs.and_then(|outputs| outputs.stance);
        let effective = client_stance_command(UserCommand::Q1Quakeworld(command), stance)
            .map_err(|error| MovementError::Contract(error.0))?;
        let UserCommand::Q1Quakeworld(effective) = effective else {
            return Err(MovementError::Contract("Client stance changed movement dialect"));
        };
        command = effective;
        self.command = command;
        // CL_PredictUsercmd seeds pmove.angles before PlayerMove computes its axes.
        if self.context.input.fields().execution == MovementExecution::Prediction {
            self.state.angles = self.context.math.vec(
                f64::from(command.angles.x),
                f64::from(command.angles.y),
                f64::from(command.angles.z),
            );
        }
        let axes = self.context.math.angles(self.state.angles);
        self.forward = axes.forward;
        self.right = axes.right;
        let environment = self.context.input.environment().clone();
        if let Some(mode) = client_movement_mode(environment.client_outputs.as_ref(), environment.health)
        {
            if mode == super::super::types::ModClientMovementMode::Freeze {
                self.state.velocity = ZERO;
                return Ok(());
            }
        }
        if self.state.spectator != 0 {
            self.spectator_move(false);
            return Ok(());
        }
        let contact_start = self.context.contacts.len();
        let has_pose = environment.pose.is_some();
        if !has_pose {
            self.nudge();
        }
        self.state.angles = self.context.math.vec(
            f64::from(command.angles.x),
            f64::from(command.angles.y),
            f64::from(command.angles.z),
        );
        self.categorize();
        if has_pose {
            self.state.velocity = ZERO;
        } else if environment.flight && environment.health > 0.0 {
            self.state.ground = NONE;
            self.state.water_jump_time_seconds = 0.0;
            self.spectator_move(true);
        } else {
            if self.water_level == 2 {
                self.check_water_jump();
            }
            if f64::from(self.state.velocity.z) < 0.0 {
                self.state.water_jump_time_seconds = 0.0;
            }
            if command.buttons & 2 != 0 {
                self.jump();
            } else {
                self.state.old_buttons &= !2;
            }
            self.friction();
            if self.water_level >= 2 {
                self.water_move();
            } else {
                self.air_move();
            }
        }
        self.categorize();
        if let Some(hooks) = self.context.options.hooks.as_mut() {
            hooks.qw_state(self.water_level, self.water_type);
        }
        let state = self.context.link(Q1State::Quakeworld(self.state.clone()), true);
        self.set_state(state)?;
        // QW records impacts during PMove, then invokes each target once after linking.
        let contacts = self.context.contacts[contact_start..].to_vec();
        for contact in contacts {
            if self.context.removed {
                break;
            }
            if matches!(contact.target, TraceHit::None) || self.already_touched(&contact.target) {
                continue;
            }
            self.touched.push(contact.target.clone());
            let state =
                self.context.touch(contact.trace.clone(), Q1State::Quakeworld(self.state.clone()), false);
            self.set_state(state)?;
        }
        Ok(())
    }

    fn run_command(&mut self, command: QwUserCommand) -> Result<(), MovementError> {
        if self.context.removed {
            return Ok(());
        }
        let maximum = match &self.context.input {
            super::types::Q1PlayerInput::Quakeworld(input) => match input.profile.clock {
                ClockProfile::Q1Quakeworld {
                    maximum_command_milliseconds,
                } => maximum_command_milliseconds,
                _ => {
                    return Err(MovementError::Contract(
                        "QuakeWorld movement needs a QuakeWorld command clock",
                    ));
                }
            },
            _ => {
                return Err(MovementError::Contract(
                    "QuakeWorld movement needs a QuakeWorld command clock",
                ));
            }
        };
        for slice in quake_world_command_slices(&command, maximum)? {
            if self.context.removed {
                break;
            }
            self.step(slice)?;
            self.context.substep += 1;
        }
        Ok(())
    }

    fn run(&mut self) -> Result<QwMovementResult, MovementError> {
        check_milliseconds(self.command.milliseconds)?;
        let command = self.command;
        self.run_command(command)?;
        let state = self.context.lifecycle(
            Q1State::Quakeworld(self.state.clone()),
            Q1LifecyclePhase::AfterPhysics,
        );
        self.set_state(state)?;
        if self.context.removed {
            return Ok(MovementOutcome::ActorRemoved {
                actor: self.context.input.actor().id().clone(),
                command_sequence: self.context.input.fields().command_sequence,
                effects: std::mem::take(&mut self.context.effects),
            });
        }
        finish_quakeworld(
            &mut self.context,
            self.state.clone(),
            self.state.angles,
            self.state.ground.clone(),
            self.water_level,
            self.water_type,
        )
    }
}

/// Run a QuakeWorld step.
pub fn move_quake_world<S: Q1MovementServices, H: Q1MovementHooks>(
    input: QwMovementInput,
    services: &mut S,
    options: Q1MovementOptions<H>,
) -> Result<QwMovementResult, MovementError> {
    QuakeWorldMove::new(input, services, options).run()
}

/// QuakeWorld movement provider.
#[derive(Debug, Clone)]
pub struct QwMovementProvider<H = NoQ1Hooks> {
    /// Provider identity.
    pub id: ProviderId,
    /// Movement options.
    pub options: Q1MovementOptions<H>,
}

/// Build a QuakeWorld movement provider.
pub fn create_qw_movement_provider<H>(
    id: ProviderId,
    options: Q1MovementOptions<H>,
) -> QwMovementProvider<H> {
    QwMovementProvider { id, options }
}

impl<H: Q1MovementHooks + Clone> QwMovementProvider<H> {
    /// Run a QuakeWorld step.
    pub fn move_step<S: Q1MovementServices>(
        &self,
        input: QwMovementInput,
        services: &mut S,
    ) -> Result<QwMovementResult, MovementError> {
        move_quake_world(input, services, self.options.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{Bounds, vec3};
    use qa_core::numeric::{NumericOps, Q1_DONOR_PROFILE};
    use qa_core::time::{FrameContext, FramePhase, SourceTime};

    use super::super::types::{
        Q1AnimationStepInput, Q1AnimationStepResult, Q1MovementProfile, Q1TraceQuery,
        Q1WeaponStepInput, Q1WeaponStepResult, QwMovementProfile,
    };
    use super::super::super::types::{
        ActorAnimationState, AnimationState, ArsenalState, MovementEnvironment, MovementInputFields,
        MovementTouchContact, TraceContact, TraceShape, WeaponState,
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
            Q1_CONTENTS_EMPTY
        }
        fn touch(
            &mut self,
            _contact: MovementTouchContact,
            state: Q1State,
        ) -> MovementContinuation<Q1State> {
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

    fn fixture(milliseconds: i32) -> (QwMovementInput, NullServices) {
        let owner = IdentityOwner::create("q1-qw").unwrap();
        let id = owner.actor(1, 0);
        let actor = owner.owned_actor(&id, ProviderId::new("q1", "test")).unwrap();
        let input = QwMovementInput {
            fields: MovementInputFields {
                actor,
                command_sequence: 1,
                frame: FrameContext {
                    frame: 1,
                    time: SourceTime::Milliseconds(1000),
                    elapsed: SourceTime::Milliseconds(milliseconds),
                    phase: FramePhase::ClientCommand,
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
            },
            command: QwUserCommand {
                milliseconds,
                angles: vec3(0.0, 90.0, 0.0),
                forward_move: 400.0,
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
                parameters: crate::movement::Q1MovementParameters {
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
        };
        (input, services)
    }

    #[test]
    fn slices_split_long_commands_and_clear_impulse() {
        let (input, _) = fixture(100);
        let slices = quake_world_command_slices(&input.command, 50.0).unwrap();
        assert_eq!(slices.len(), 2);
        assert_eq!(slices[0].milliseconds, 50);
        assert_eq!(slices[1].milliseconds, 50);
        let (mut input, _) = fixture(100);
        input.command.impulse = 9;
        let slices = quake_world_command_slices(&input.command, 50.0).unwrap();
        assert_eq!(slices[0].impulse, 9);
        assert_eq!(slices[1].impulse, 0);
        assert!(quake_world_command_slices(&input.command, 0.0).is_err());
        let (mut input, _) = fixture(100);
        input.command.milliseconds = 300;
        assert!(quake_world_command_slices(&input.command, 50.0).is_err());
        let _ = Q1MovementProfile {
            id: ProviderId::new("q1", "test"),
            clock: ClockProfile::Q1Netquake {
                minimum_frame_seconds: 0.0,
                maximum_frame_seconds: 0.1,
                fixed_frame_seconds: None,
            },
            numeric: Q1_DONOR_PROFILE,
            edition: super::super::types::Q1Edition::Classic,
            parameters: crate::movement::Q1MovementParameters {
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
        };
    }

    #[test]
    fn step_moves_and_applies_gravity() {
        let (input, mut services) = fixture(30);
        let result =
            move_quake_world(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default())
                .unwrap();
        match result {
            MovementOutcome::Active { state, .. } => {
                assert!(state.velocity.z < 0.0);
                assert!(state.origin.y > 0.0);
            }
            MovementOutcome::ActorRemoved { .. } => panic!("unexpected removal"),
        }
    }

    #[test]
    fn long_command_runs_multiple_slices() {
        let (input, mut services) = fixture(100);
        let result =
            move_quake_world(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default())
                .unwrap();
        assert!(!result.removed());
    }

    #[test]
    fn invalid_milliseconds_rejected() {
        let (mut input, mut services) = fixture(30);
        input.command.milliseconds = 300;
        assert!(
            move_quake_world(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default())
                .is_err()
        );
    }

    #[test]
    fn spectator_integrates_freely() {
        let (mut input, mut services) = fixture(30);
        input.state.spectator = 1;
        input.state.velocity = vec3(0.0, 100.0, 0.0);
        // Zero wish skips spectator integration in the source; keep a
        // forward command so the wish accelerates the body.
        input.command.forward_move = 100.0;
        let result =
            move_quake_world(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default())
                .unwrap();
        match result {
            MovementOutcome::Active { state, .. } => {
                assert!(state.origin.y > 0.0);
            }
            MovementOutcome::ActorRemoved { .. } => panic!("unexpected removal"),
        }
    }
}
