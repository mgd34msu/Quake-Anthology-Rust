//! NetQuake player movement.
//!
//! Donor provenance: `src/movement/q1/netquake.ts` (ported from
//! WinQuake `sv_user.c`, `sv_phys.c` and rerelease donor extensions).

use qa_core::identity::ProviderId;
use qa_core::math::Vec3;
use qa_core::time::ClockProfile;

use super::super::client_outputs::{client_movement_mode, client_movement_type};
use super::super::swept_body::{sweep_body, SweepStop, SweptBodyServices, SweptBodyState};
use super::super::types::{
    MovementContinuation, MovementDialect, MovementError, MovementExecution, MovementInputContinuation,
    MovementOutcome, TraceHit, TraceShape, UserCommand,
};
use super::common::{seconds, MovementContext, NONE, ZERO};
use super::player_actions::{q1_check_water_jump_in, q1_player_jump, Q1JumpAction};
use super::result::finish_netquake;
use super::types::{
    NoQ1Hooks, Q1Edition, Q1FixAngleRoll, Q1JumpAuthority, Q1LifecyclePhase, Q1MovementHooks, Q1MovementInput,
    Q1MovementOptions, Q1MovementResult, Q1MovementServices, Q1MovementSound, Q1MovementState, Q1PlayerInput, Q1Solid,
    Q1State, Q1Trace, Q1TraceMove, Q1_CONTENTS_EMPTY, Q1_CONTENTS_WATER, Q1_FLAG_FLY, Q1_FLAG_JUMPRELEASED,
    Q1_FLAG_ONGROUND, Q1_FLAG_SWIM, Q1_FLAG_WATERJUMP, Q1_MOVE_BOUNCE, Q1_MOVE_FLY, Q1_MOVE_FLYMISSILE, Q1_MOVE_GIB,
    Q1_MOVE_NOCLIP, Q1_MOVE_NONE, Q1_MOVE_STEP, Q1_MOVE_TOSS, Q1_MOVE_WALK, Q1_STEP_HEIGHT,
};
use super::water_transition::q1_water_transition;

/// Fly-move outcome.
#[derive(Debug, Clone, PartialEq)]
struct FlyResult {
    blocked: i32,
    step_trace: Option<Q1Trace>,
}

struct NetQuakeMove<'s, S: Q1MovementServices, H: Q1MovementHooks> {
    state: Q1MovementState,
    context: MovementContext<'s, S, H>,
    frame_seconds: f64,
    time_seconds: f64,
}

impl<'s, S: Q1MovementServices, H: Q1MovementHooks> NetQuakeMove<'s, S, H> {
    fn new(input: Q1MovementInput, services: &'s mut S, options: Q1MovementOptions<H>) -> Result<Self, MovementError> {
        if input.profile.edition == Q1Edition::Quake64 {
            return Err(MovementError::Contract(
                "Quake64 movement requires a qualified source physics profile",
            ));
        }
        if !matches!(input.profile.clock, ClockProfile::Q1Netquake { .. }) {
            return Err(MovementError::Contract(
                "NetQuake movement requires a NetQuake frame clock",
            ));
        }
        let mut state = input.state.clone();
        if input.fields.environment.flight && input.fields.environment.health > 0.0 && state.move_type == Q1_MOVE_WALK {
            state.move_type = Q1_MOVE_FLY;
        }
        let mut mover = Self {
            state,
            context: MovementContext::new(Q1PlayerInput::Netquake(input), services, options),
            frame_seconds: 0.0,
            time_seconds: 0.0,
        };
        mover.apply_client_mode();
        // The shared frame owner already applies host minimum/maximum/fixed frame rules.
        mover.frame_seconds = seconds(mover.context.input.frame().elapsed);
        mover.time_seconds = seconds(mover.context.input.frame().time);
        if !mover.frame_seconds.is_finite() || mover.frame_seconds < 0.0 {
            return Err(MovementError::Range("Invalid NetQuake frame interval"));
        }
        Ok(mover)
    }

    fn apply_client_mode(&mut self) {
        let environment = self.context.input.environment().clone();
        if let Some(mode) = client_movement_mode(environment.client_outputs.as_ref(), environment.health) {
            self.context.project_client_mode();
            self.state.move_type = client_movement_type(MovementDialect::Q1Netquake, mode);
        }
    }

    fn set_state(&mut self, state: Q1State) -> Result<(), MovementError> {
        let Q1State::Netquake(state) = state else {
            return Err(MovementError::Contract(
                "NetQuake callback changed the movement provider during a frame",
            ));
        };
        self.state = state;
        self.apply_client_mode();
        let environment = self.context.input.environment().clone();
        if environment.flight && environment.health > 0.0 && self.state.move_type == Q1_MOVE_WALK {
            self.state.move_type = Q1_MOVE_FLY;
        }
        Ok(())
    }

    fn link(&mut self, touch_triggers: bool) -> Result<(), MovementError> {
        let state = self.context.link(Q1State::Netquake(self.state.clone()), touch_triggers);
        self.set_state(state)
    }

    fn impact(&mut self, trace: Q1Trace) -> Result<(), MovementError> {
        let state = self.context.touch(trace, Q1State::Netquake(self.state.clone()), true);
        self.set_state(state)
    }

    fn think(&mut self) -> Result<bool, MovementError> {
        let state = self
            .context
            .lifecycle(Q1State::Netquake(self.state.clone()), Q1LifecyclePhase::Think);
        self.set_state(state)?;
        Ok(!self.context.removed)
    }

    fn velocity_bounds(&mut self) {
        let maximum = self.context.options.max_velocity.unwrap_or(2000.0);
        let n = self.context.math.n;
        let coordinate = |value: f32| {
            if f64::from(n.store(f64::from(value))).is_finite() {
                f64::from(value)
            } else {
                0.0
            }
        };
        let velocity = |value: f32| coordinate(value).clamp(-maximum, maximum);
        let origin = self.state.origin;
        let velocity_v = self.state.velocity;
        self.state.origin = self
            .context
            .math
            .vec(coordinate(origin.x), coordinate(origin.y), coordinate(origin.z));
        self.state.velocity =
            self.context
                .math
                .vec(velocity(velocity_v.x), velocity(velocity_v.y), velocity(velocity_v.z));
    }

    fn gravity(&mut self) {
        let parameters = match &self.context.input {
            Q1PlayerInput::Netquake(input) => input.profile.parameters,
            Q1PlayerInput::Quakeworld(_) => panic!("NetQuake mover with QuakeWorld input"),
        };
        let entity_gravity = if parameters.entity_gravity == 0.0 {
            1.0
        } else {
            parameters.entity_gravity
        };
        let n = self.context.math.n;
        let multiplier = self.context.input.environment().gravity_multiplier;
        let gravity = n.mul(
            n.mul(n.mul(entity_gravity, multiplier), parameters.gravity),
            self.frame_seconds,
        );
        let velocity = self.state.velocity;
        self.state.velocity = self.context.math.vec(
            f64::from(velocity.x),
            f64::from(velocity.y),
            n.sub(f64::from(velocity.z), gravity),
        );
    }

    fn fly_move(&mut self, time: f64) -> FlyResult {
        struct Sweep<'a, 's, S: Q1MovementServices, H: Q1MovementHooks> {
            mover: &'a mut NetQuakeMove<'s, S, H>,
            blocked: i32,
            step_trace: Option<Q1Trace>,
        }

        impl<S: Q1MovementServices, H: Q1MovementHooks> SweptBodyServices for Sweep<'_, '_, S, H> {
            type Trace = Q1Trace;

            fn read(&mut self) -> Option<SweptBodyState> {
                if self.mover.context.removed {
                    return None;
                }
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
                if matches!(trace.hit, TraceHit::None) {
                    panic!("NetQuake slide trace blocked without a hit");
                }
                trace.source_plane.normal
            }
            fn impact(&mut self, trace: &Q1Trace, normal: Vec3) {
                if normal.z > 0.7 {
                    self.blocked |= 1;
                    if self.mover.context.is_bsp(&trace.hit) {
                        self.mover.state.flags |= Q1_FLAG_ONGROUND;
                        self.mover.state.ground = trace.hit.clone();
                    }
                }
                if normal.z == 0.0 {
                    self.blocked |= 2;
                    self.step_trace = Some(trace.clone());
                }
                self.mover
                    .impact(trace.clone())
                    .expect("NetQuake callback changed the movement provider during a frame");
            }
            fn stop_when_still(&self) -> bool {
                true
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

        let mut sweep = Sweep {
            mover: self,
            blocked: 0,
            step_trace: None,
        };
        let stop = sweep_body(&mut sweep, time);
        let mut blocked = sweep.blocked;
        if stop == SweepStop::Solid || stop == SweepStop::PlaneLimit {
            blocked = 3;
        } else if stop == SweepStop::CreaseBlocked {
            blocked = 7;
        }
        let step_trace = sweep.step_trace;
        FlyResult { blocked, step_trace }
    }

    fn push_entity(&mut self, push: Vec3) -> Result<Q1Trace, MovementError> {
        let policy = if self.state.move_type == Q1_MOVE_FLYMISSILE {
            Q1TraceMove::Missile
        } else if matches!(self.context.options.solid, Some(Q1Solid::Not) | Some(Q1Solid::Trigger)) {
            Q1TraceMove::NoMonsters
        } else {
            Q1TraceMove::Normal
        };
        let shape = self.context.shape();
        let trace = self.context.trace(
            self.state.origin,
            self.context.math.add(self.state.origin, push),
            shape,
            policy,
        );
        self.state.origin = trace.end;
        self.link(true)?;
        if !self.context.removed && !matches!(trace.hit, TraceHit::None) {
            self.impact(trace.clone())?;
        }
        Ok(trace)
    }

    fn check_stuck(&mut self) -> Result<(), MovementError> {
        if self.context.position_free(self.state.origin) {
            self.state.old_origin = self.state.origin;
            return Ok(());
        }
        let original = self.state.origin;
        self.state.origin = self.state.old_origin;
        if self.context.position_free(self.state.origin) {
            return self.link(true);
        }
        for z in 0..18 {
            for x in -1..=1 {
                for y in -1..=1 {
                    let offset = self.context.math.vec(x as f64, y as f64, z as f64);
                    self.state.origin = self.context.math.add(original, offset);
                    if self.context.position_free(self.state.origin) {
                        return self.link(true);
                    }
                }
            }
        }
        self.state.origin = original;
        Ok(())
    }

    fn check_water(&mut self) -> bool {
        let bounds = self.context.bounds();
        let origin = self.state.origin;
        let n = self.context.math.n;
        let mut point = self.context.math.vec(
            f64::from(origin.x),
            f64::from(origin.y),
            n.add(n.add(f64::from(origin.z), f64::from(bounds.min.z)), 1.0),
        );
        self.state.water_level = 0;
        self.state.water_type = Q1_CONTENTS_EMPTY;
        let contents = self.context.contents(point);
        if contents <= Q1_CONTENTS_WATER {
            self.state.water_type = contents;
            self.state.water_level = 1;
            point = self.context.math.vec(
                f64::from(point.x),
                f64::from(point.y),
                n.add(
                    f64::from(origin.z),
                    n.mul(n.add(f64::from(bounds.min.z), f64::from(bounds.max.z)), 0.5),
                ),
            );
            if self.context.contents(point) <= Q1_CONTENTS_WATER {
                self.state.water_level = 2;
                let height = self.context.view_height();
                point = self.context.math.vec(
                    f64::from(point.x),
                    f64::from(point.y),
                    n.add(f64::from(origin.z), height),
                );
                if self.context.contents(point) <= Q1_CONTENTS_WATER {
                    self.state.water_level = 3;
                }
            }
        }
        self.state.water_level > 1
    }

    fn wall_friction(&mut self, trace: &Q1Trace) {
        let direction = self.context.math.angles(self.state.view_angles).forward;
        let n = self.context.math.n;
        let d = n.add(self.context.math.dot(trace.source_plane.normal, direction), 0.5);
        if d >= 0.0 {
            return;
        }
        let into = self.context.math.scale(
            trace.source_plane.normal,
            self.context.math.dot(trace.source_plane.normal, self.state.velocity),
        );
        let side = self.context.math.sub(self.state.velocity, into);
        let velocity = self.state.velocity;
        self.state.velocity = self.context.math.vec(
            n.mul(f64::from(side.x), n.add(1.0, d)),
            n.mul(f64::from(side.y), n.add(1.0, d)),
            f64::from(velocity.z),
        );
    }

    fn try_unstick(&mut self, old_velocity: Vec3) -> Result<FlyResult, MovementError> {
        let original = self.state.origin;
        let directions = [
            (2.0, 0.0),
            (0.0, 2.0),
            (-2.0, 0.0),
            (0.0, -2.0),
            (2.0, 2.0),
            (-2.0, 2.0),
            (2.0, -2.0),
            (-2.0, -2.0),
        ];
        for (x, y) in directions {
            let push = self.context.math.vec(x, y, 0.0);
            self.push_entity(push)?;
            if self.context.removed {
                return Ok(FlyResult {
                    blocked: 7,
                    step_trace: None,
                });
            }
            self.state.velocity = self
                .context
                .math
                .vec(f64::from(old_velocity.x), f64::from(old_velocity.y), 0.0);
            let clip = self.fly_move(0.1);
            if self.context.removed
                || (f64::from(original.y) - f64::from(self.state.origin.y)).abs() > 4.0
                || (f64::from(original.x) - f64::from(self.state.origin.x)).abs() > 4.0
            {
                return Ok(clip);
            }
            self.state.origin = original;
        }
        self.state.velocity = ZERO;
        Ok(FlyResult {
            blocked: 7,
            step_trace: None,
        })
    }

    fn walk_move(&mut self) -> Result<(), MovementError> {
        let old_on_ground = self.state.flags & Q1_FLAG_ONGROUND != 0;
        self.state.flags &= !Q1_FLAG_ONGROUND;
        let original = self.state.origin;
        let old_velocity = self.state.velocity;
        let clip = self.fly_move(self.frame_seconds);
        if self.context.removed
            || clip.blocked & 2 == 0
            || (!old_on_ground && self.state.water_level == 0)
            || self.state.move_type != Q1_MOVE_WALK
            || self.context.options.no_step
            || self.state.flags & Q1_FLAG_WATERJUMP != 0
        {
            return Ok(());
        }
        let no_step_origin = self.state.origin;
        let no_step_velocity = self.state.velocity;
        self.state.origin = original;
        let up = self.context.math.vec(0.0, 0.0, Q1_STEP_HEIGHT);
        self.push_entity(up)?;
        if self.context.removed {
            return Ok(());
        }
        self.state.velocity = self
            .context
            .math
            .vec(f64::from(old_velocity.x), f64::from(old_velocity.y), 0.0);
        let mut clip = self.fly_move(self.frame_seconds);
        if self.context.removed {
            return Ok(());
        }
        if clip.blocked != 0
            && (f64::from(original.y) - f64::from(self.state.origin.y)).abs() < 0.03125
            && (f64::from(original.x) - f64::from(self.state.origin.x)).abs() < 0.03125
        {
            let unstick = self.try_unstick(old_velocity)?;
            clip = FlyResult {
                blocked: unstick.blocked,
                step_trace: clip.step_trace,
            };
        }
        if self.context.removed {
            return Ok(());
        }
        if clip.blocked & 2 != 0 {
            if let Some(step_trace) = clip.step_trace.clone() {
                self.wall_friction(&step_trace);
            }
        }
        let n = self.context.math.n;
        let down = self.context.math.vec(
            0.0,
            0.0,
            n.add(-Q1_STEP_HEIGHT, n.mul(f64::from(old_velocity.z), self.frame_seconds)),
        );
        let trace = self.push_entity(down)?;
        if self.context.removed {
            return Ok(());
        }
        if trace.source_plane.normal.z > 0.7 {
            // WinQuake checks the player's SOLID_BSP here, not the hit's solid type.
            if self.context.options.solid == Some(Q1Solid::Bsp) {
                self.state.flags |= Q1_FLAG_ONGROUND;
                self.state.ground = trace.hit;
            }
        } else {
            self.state.origin = no_step_origin;
            self.state.velocity = no_step_velocity;
        }
        Ok(())
    }

    fn friction(&mut self) {
        let speed = self.context.math.horizontal(self.state.velocity);
        if speed == 0.0 {
            return;
        }
        let parameters = match &self.context.input {
            Q1PlayerInput::Netquake(input) => input.profile.parameters,
            Q1PlayerInput::Quakeworld(_) => panic!("NetQuake mover with QuakeWorld input"),
        };
        let edge_friction = match &self.context.input {
            Q1PlayerInput::Netquake(input) => input.profile.edge_friction,
            Q1PlayerInput::Quakeworld(_) => panic!("NetQuake mover with QuakeWorld input"),
        };
        let n = self.context.math.n;
        let origin = self.state.origin;
        let velocity = self.state.velocity;
        let bounds = self.context.bounds();
        let start = self.context.math.vec(
            n.add(f64::from(origin.x), n.mul(n.div(f64::from(velocity.x), speed), 16.0)),
            n.add(f64::from(origin.y), n.mul(n.div(f64::from(velocity.y), speed), 16.0)),
            n.add(f64::from(origin.z), f64::from(bounds.min.z)),
        );
        let end = self.context.math.add(start, self.context.math.vec(0.0, 0.0, -34.0));
        let trace = self
            .context
            .trace(start, end, TraceShape::Point, Q1TraceMove::NoMonsters);
        let friction = if trace.fraction == 1.0 {
            n.mul(parameters.friction, edge_friction)
        } else {
            parameters.friction
        };
        let new_speed = (0.0f64).max(n.sub(
            speed,
            n.mul(n.mul(self.frame_seconds, speed.max(parameters.stop_speed)), friction),
        ));
        self.state.velocity = self.context.math.scale(self.state.velocity, n.div(new_speed, speed));
    }

    fn accelerate(&mut self, direction: Vec3, speed: f64, air: bool) {
        let parameters = match &self.context.input {
            Q1PlayerInput::Netquake(input) => input.profile.parameters,
            Q1PlayerInput::Quakeworld(_) => panic!("NetQuake mover with QuakeWorld input"),
        };
        let n = self.context.math.n;
        let wish = if air {
            self.context.math.length(direction).min(30.0)
        } else {
            speed
        };
        let axis = if air {
            self.context.math.normalize(direction).0
        } else {
            direction
        };
        let add = n.sub(wish, self.context.math.dot(self.state.velocity, axis));
        if add <= 0.0 {
            return;
        }
        let acceleration = if air {
            n.mul(n.mul(parameters.accelerate, speed), self.frame_seconds)
        } else {
            n.mul(n.mul(parameters.accelerate, self.frame_seconds), speed)
        }
        .min(add);
        self.state.velocity = self.context.math.ma(self.state.velocity, acceleration, axis);
    }

    fn water_move(&mut self) {
        let (parameters, command) = match &self.context.input {
            Q1PlayerInput::Netquake(input) => (input.profile.parameters, input.command),
            Q1PlayerInput::Quakeworld(_) => panic!("NetQuake mover with QuakeWorld input"),
        };
        let n = self.context.math.n;
        let axes = self.context.math.angles(self.state.view_angles);
        let forward = self.context.speed(command.forward_move);
        let side = self.context.speed(command.side_move);
        let mut wish = self.context.math.vec(
            n.add(
                n.mul(f64::from(axes.forward.x), forward),
                n.mul(f64::from(axes.right.x), side),
            ),
            n.add(
                n.mul(f64::from(axes.forward.y), forward),
                n.mul(f64::from(axes.right.y), side),
            ),
            n.add(
                n.mul(f64::from(axes.forward.z), forward),
                n.mul(f64::from(axes.right.z), side),
            ),
        );
        let up = if command.forward_move == 0.0 && command.side_move == 0.0 && command.up_move == 0.0 {
            -60.0
        } else {
            self.context.speed(command.up_move)
        };
        wish = self
            .context
            .math
            .vec(f64::from(wish.x), f64::from(wish.y), n.add(f64::from(wish.z), up));
        let mut wish_speed = self.context.math.length(wish);
        let max = self.context.speed(parameters.max_speed);
        if wish_speed > max {
            wish = self.context.math.scale(wish, n.div(max, wish_speed));
            wish_speed = max;
        }
        wish_speed = n.mul(wish_speed, 0.7);
        let speed = self.context.math.length(self.state.velocity);
        let mut new_speed = 0.0;
        if speed != 0.0 {
            new_speed = (0.0f64).max(n.sub(speed, n.mul(n.mul(self.frame_seconds, speed), parameters.friction)));
            self.state.velocity = self.context.math.scale(self.state.velocity, n.div(new_speed, speed));
        }
        if wish_speed == 0.0 {
            return;
        }
        let add = n.sub(wish_speed, new_speed);
        if add <= 0.0 {
            return;
        }
        let acceleration = n
            .mul(n.mul(parameters.accelerate, wish_speed), self.frame_seconds)
            .min(add);
        let direction = self.context.math.normalize(wish).0;
        self.state.velocity = self.context.math.ma(self.state.velocity, acceleration, direction);
    }

    fn air_move(&mut self) {
        let (parameters, command) = match &self.context.input {
            Q1PlayerInput::Netquake(input) => (input.profile.parameters, input.command),
            Q1PlayerInput::Quakeworld(_) => panic!("NetQuake mover with QuakeWorld input"),
        };
        let n = self.context.math.n;
        let axes = self.context.math.angles(self.state.angles);
        let forward_move = if self.time_seconds < self.state.teleport_time_seconds && command.forward_move < 0.0 {
            0.0
        } else {
            command.forward_move
        };
        let forward = self.context.speed(forward_move);
        let side = self.context.speed(command.side_move);
        let mut wish = self.context.math.vec(
            n.add(
                n.mul(f64::from(axes.forward.x), forward),
                n.mul(f64::from(axes.right.x), side),
            ),
            n.add(
                n.mul(f64::from(axes.forward.y), forward),
                n.mul(f64::from(axes.right.y), side),
            ),
            if self.state.move_type == Q1_MOVE_WALK {
                0.0
            } else {
                self.context.speed(command.up_move)
            },
        );
        let normalized = self.context.math.normalize(wish);
        let mut wish_speed = normalized.1;
        let max = self.context.speed(parameters.max_speed);
        if wish_speed > max {
            wish = self.context.math.scale(wish, n.div(max, wish_speed));
            wish_speed = max;
        }
        if self.state.move_type == Q1_MOVE_NOCLIP {
            self.state.velocity = wish;
        } else if self.state.flags & Q1_FLAG_ONGROUND != 0 {
            self.friction();
            self.accelerate(normalized.0, wish_speed, false);
        } else {
            self.accelerate(wish, wish_speed, true);
        }
    }

    fn alternate_noclip(&mut self) {
        let command = match &self.context.input {
            Q1PlayerInput::Netquake(input) => input.command,
            Q1PlayerInput::Quakeworld(_) => panic!("NetQuake mover with QuakeWorld input"),
        };
        let parameters = match &self.context.input {
            Q1PlayerInput::Netquake(input) => input.profile.parameters,
            Q1PlayerInput::Quakeworld(_) => panic!("NetQuake mover with QuakeWorld input"),
        };
        let n = self.context.math.n;
        let axes = self.context.math.angles(self.state.view_angles);
        let forward = self.context.speed(command.forward_move);
        let side = self.context.speed(command.side_move);
        let wish = self.context.math.vec(
            n.add(
                n.mul(f64::from(axes.forward.x), forward),
                n.mul(f64::from(axes.right.x), side),
            ),
            n.add(
                n.mul(f64::from(axes.forward.y), forward),
                n.mul(f64::from(axes.right.y), side),
            ),
            n.add(
                n.add(
                    n.mul(f64::from(axes.forward.z), forward),
                    n.mul(f64::from(axes.right.z), side),
                ),
                n.mul(self.context.speed(command.up_move), 2.0),
            ),
        );
        let normalized = self.context.math.normalize(wish);
        let max = self.context.speed(parameters.max_speed);
        self.state.velocity = if normalized.1 > max {
            self.context.math.scale(normalized.0, max)
        } else {
            wish
        };
    }

    fn client_think(&mut self) {
        if self.state.move_type == Q1_MOVE_NONE {
            return;
        }
        if let Some(source_punch) = self.context.options.source_punch_angles {
            self.state.punch_angles = source_punch;
        } else {
            let n = self.context.math.n;
            let punch = self.context.math.normalize(self.state.punch_angles);
            let length = (0.0f64).max(n.sub(punch.1, n.mul(10.0, self.frame_seconds)));
            self.state.punch_angles = self.context.math.scale(punch.0, length);
        }
        if self.state.health <= 0.0 {
            return;
        }
        let angles = self.context.math.add(self.state.view_angles, self.state.punch_angles);
        let right = self.context.math.angles(self.state.angles).right;
        let side = self.context.math.dot(self.state.velocity, right);
        let sign = if side < 0.0 { -1.0 } else { 1.0 };
        let n = self.context.math.n;
        let roll_speed = self.context.options.roll_speed.unwrap_or(200.0);
        let roll_angle = self.context.options.roll_angle.unwrap_or(2.0);
        let roll = n.mul(
            n.mul(
                if side.abs() < roll_speed {
                    n.div(n.mul(side.abs(), roll_angle), roll_speed)
                } else {
                    roll_angle
                },
                sign,
            ),
            4.0,
        );
        if !self.state.fix_angle {
            self.state.angles = self
                .context
                .math
                .vec(n.div(-f64::from(angles.x), 3.0), f64::from(angles.y), roll);
        } else if self.context.options.fix_angle_roll == Q1FixAngleRoll::Source {
            self.state.angles =
                self.context
                    .math
                    .vec(f64::from(self.state.angles.x), f64::from(self.state.angles.y), roll);
        }
        if self.state.flags & Q1_FLAG_WATERJUMP != 0 {
            if self.time_seconds > self.state.teleport_time_seconds || self.state.water_level == 0 {
                self.state.flags &= !Q1_FLAG_WATERJUMP;
                self.state.teleport_time_seconds = 0.0;
            }
            let direction = self.state.water_jump_direction;
            self.state.velocity = self.context.math.vec(
                f64::from(direction.x),
                f64::from(direction.y),
                f64::from(self.state.velocity.z),
            );
            return;
        }
        let no_clip_hack = match &self.context.input {
            Q1PlayerInput::Netquake(input) => input.profile.no_clip_angle_hack,
            Q1PlayerInput::Quakeworld(_) => false,
        };
        if self.state.move_type == Q1_MOVE_NOCLIP && no_clip_hack {
            self.alternate_noclip();
        } else if self.state.water_level >= 2 && self.state.move_type != Q1_MOVE_NOCLIP {
            self.water_move();
        } else {
            self.air_move();
        }
    }

    fn player_actions(&mut self) -> Result<(), MovementError> {
        if self.context.options.jump_authority == Q1JumpAuthority::SourceGamecode
            || self.state.health <= 0.0
            || self.context.view_height() == 0.0
            || self.state.move_type != Q1_MOVE_WALK
        {
            return Ok(());
        }
        if self.state.water_level == 2 {
            let state = self.state.clone();
            let checked = q1_check_water_jump_in(&mut self.context, &state, self.time_seconds);
            self.set_state(Q1State::Netquake(checked))?;
        }
        let buttons = match &self.context.input {
            Q1PlayerInput::Netquake(input) => input.command.buttons,
            Q1PlayerInput::Quakeworld(_) => 0,
        };
        if buttons & 2 != 0 {
            let result = {
                let services: &S = self.context.services;
                q1_player_jump(&self.state, services)
            };
            let action = result.action;
            self.set_state(Q1State::Netquake(result.state))?;
            if action != Q1JumpAction::None {
                if let Some(hooks) = self.context.options.hooks.as_mut() {
                    let actor = self.context.input.actor().clone();
                    let state = self.state.clone();
                    hooks.player_action(
                        &actor,
                        match action {
                            Q1JumpAction::Jump => super::types::Q1PlayerActionKind::Jump,
                            Q1JumpAction::Swim => super::types::Q1PlayerActionKind::Swim,
                            Q1JumpAction::None => unreachable!(),
                        },
                        &state,
                    );
                }
            }
        } else {
            self.state.flags |= Q1_FLAG_JUMPRELEASED;
        }
        Ok(())
    }

    fn ideal_pitch(&mut self) {
        if self.state.flags & Q1_FLAG_ONGROUND == 0 {
            return;
        }
        let n = self.context.math.n;
        let radians = n.div(
            n.mul(n.mul(f64::from(self.state.angles.y), std::f64::consts::PI), 2.0),
            360.0,
        );
        let sine = radians.sin();
        let cosine = radians.cos();
        let origin = self.state.origin;
        let height = self.context.view_height();
        let mut heights: Vec<f64> = Vec::new();
        for i in 0..6 {
            let top = self.context.math.vec(
                n.add(f64::from(origin.x), n.mul(n.mul(cosine, i as f64 + 3.0), 12.0)),
                n.add(f64::from(origin.y), n.mul(n.mul(sine, i as f64 + 3.0), 12.0)),
                n.add(f64::from(origin.z), height),
            );
            let bottom = self.context.math.add(top, self.context.math.vec(0.0, 0.0, -160.0));
            let trace = self
                .context
                .trace(top, bottom, TraceShape::Point, Q1TraceMove::NoMonsters);
            if trace.all_solid || trace.fraction == 1.0 {
                return;
            }
            heights.push(f64::from(n.store(n.add(
                f64::from(top.z),
                n.mul(trace.fraction, n.sub(f64::from(bottom.z), f64::from(top.z))),
            ))));
        }
        let Some(first) = heights.first().copied() else {
            return;
        };
        let mut direction = 0.0;
        let mut steps = 0;
        let mut previous = first;
        for height in heights.iter().skip(1) {
            let step = n.sub(*height, previous);
            previous = *height;
            if step > -0.1 && step < 0.1 {
                continue;
            }
            if direction != 0.0 && n.sub(step, direction).abs() > 0.1 {
                return;
            }
            steps += 1;
            direction = step;
        }
        if direction == 0.0 {
            self.state.ideal_pitch = 0.0;
        } else if steps >= 2 {
            let scale = self.context.options.ideal_pitch_scale.unwrap_or(0.8);
            self.state.ideal_pitch = f64::from(n.store(n.mul(-direction, scale)));
        }
    }

    fn water_transition(&mut self) {
        let contents = self.context.contents(self.state.origin);
        let transition = q1_water_transition(self.state.water_type, contents);
        if transition.splash {
            if let Some(hooks) = self.context.options.hooks.as_mut() {
                let actor = self.context.input.actor().clone();
                let state = self.state.clone();
                hooks.sound(&actor, Q1MovementSound::WaterSplash, &state);
            }
        }
        self.state.water_type = transition.water_type;
        self.state.water_level = transition.water_level;
    }

    fn toss(&mut self) -> Result<(), MovementError> {
        if self.state.flags & Q1_FLAG_ONGROUND != 0 {
            return Ok(());
        }
        self.velocity_bounds();
        if self.state.move_type != Q1_MOVE_FLY && self.state.move_type != Q1_MOVE_FLYMISSILE {
            self.gravity();
        }
        self.state.angles = self
            .context
            .math
            .ma(self.state.angles, self.frame_seconds, self.state.angular_velocity);
        let displacement = self.context.math.scale(self.state.velocity, self.frame_seconds);
        let trace = self.push_entity(displacement)?;
        if trace.fraction == 1.0 || self.context.removed {
            return Ok(());
        }
        let edition = match &self.context.input {
            Q1PlayerInput::Netquake(input) => input.profile.edition,
            Q1PlayerInput::Quakeworld(_) => Q1Edition::Classic,
        };
        let bounces = self.state.move_type == Q1_MOVE_BOUNCE
            || (self.state.move_type == Q1_MOVE_GIB && edition == Q1Edition::Rerelease);
        self.state.velocity = self.context.math.clip(
            self.state.velocity,
            trace.source_plane.normal,
            if bounces { 1.5 } else { 1.0 },
        );
        if trace.source_plane.normal.z > 0.7 && (f64::from(self.state.velocity.z) < 60.0 || !bounces) {
            self.state.flags |= Q1_FLAG_ONGROUND;
            self.state.ground = trace.hit;
            self.state.velocity = ZERO;
            self.state.angular_velocity = ZERO;
        }
        self.water_transition();
        Ok(())
    }

    fn prepare(&mut self) -> Result<Q1MovementState, MovementError> {
        let view = match &self.context.input {
            Q1PlayerInput::Netquake(input) => input.command.view_angles,
            Q1PlayerInput::Quakeworld(_) => panic!("NetQuake mover with QuakeWorld input"),
        };
        self.state.view_angles = self
            .context
            .math
            .vec(f64::from(view.x), f64::from(view.y), f64::from(view.z));
        self.client_think();
        let state = self.context.source_state(Q1State::Netquake(self.state.clone()));
        let Q1State::Netquake(state) = state else {
            return Err(MovementError::Contract("NetQuake output changed movement dialect"));
        };
        Ok(state)
    }

    fn physics_step(&mut self) -> Result<Q1MovementResult, MovementError> {
        let state = self
            .context
            .lifecycle(Q1State::Netquake(self.state.clone()), Q1LifecyclePhase::BeforePhysics);
        self.set_state(state)?;
        let has_pose = self.context.input.environment().pose.is_some();
        if !self.context.removed && !has_pose {
            self.player_actions()?;
        }
        // Preserve the existing extension paths; the branch-local order below
        // is SV_Physics_Client's classic set.
        let extension_think = self.state.move_type == Q1_MOVE_STEP
            || self.state.move_type == Q1_MOVE_FLYMISSILE
            || self.state.move_type == Q1_MOVE_GIB;
        if !self.context.removed && extension_think {
            self.think()?;
        }
        if !self.context.removed {
            self.velocity_bounds();
            if has_pose {
                if !extension_think {
                    if !self.think()? {
                        // fall through to link/after-physics
                    } else {
                        self.state.velocity = ZERO;
                        self.check_water();
                    }
                } else if !self.context.removed {
                    self.state.velocity = ZERO;
                    self.check_water();
                }
            } else {
                match self.state.move_type {
                    Q1_MOVE_NONE => {
                        if !extension_think {
                            self.think()?;
                        }
                    }
                    Q1_MOVE_WALK => {
                        let proceed = if extension_think {
                            !self.context.removed
                        } else {
                            self.think()?
                        };
                        if proceed {
                            if !self.check_water() && self.state.flags & Q1_FLAG_WATERJUMP == 0 {
                                self.gravity();
                            }
                            self.check_stuck()?;
                            if !self.context.removed {
                                self.walk_move()?;
                            }
                        }
                    }
                    Q1_MOVE_FLY => {
                        let proceed = if extension_think {
                            !self.context.removed
                        } else {
                            self.think()?
                        };
                        if proceed {
                            self.fly_move(self.frame_seconds);
                        }
                    }
                    Q1_MOVE_NOCLIP => {
                        let proceed = if extension_think {
                            !self.context.removed
                        } else {
                            self.think()?
                        };
                        if proceed {
                            self.state.origin =
                                self.context
                                    .math
                                    .ma(self.state.origin, self.frame_seconds, self.state.velocity);
                        }
                    }
                    Q1_MOVE_TOSS | Q1_MOVE_BOUNCE => {
                        let proceed = if extension_think {
                            !self.context.removed
                        } else {
                            self.think()?
                        };
                        if proceed {
                            self.toss()?;
                        }
                    }
                    Q1_MOVE_FLYMISSILE => {
                        self.toss()?;
                    }
                    Q1_MOVE_GIB => {
                        let edition = match &self.context.input {
                            Q1PlayerInput::Netquake(input) => input.profile.edition,
                            Q1PlayerInput::Quakeworld(_) => Q1Edition::Classic,
                        };
                        if edition != Q1Edition::Rerelease {
                            return Err(MovementError::Contract(
                                "MOVETYPE_GIB requires Quake rerelease behavior",
                            ));
                        }
                        self.toss()?;
                    }
                    Q1_MOVE_STEP => {
                        if self.state.flags & (Q1_FLAG_ONGROUND | Q1_FLAG_FLY | Q1_FLAG_SWIM) == 0 {
                            let parameters = match &self.context.input {
                                Q1PlayerInput::Netquake(input) => input.profile.parameters,
                                Q1PlayerInput::Quakeworld(_) => {
                                    panic!("NetQuake mover with QuakeWorld input")
                                }
                            };
                            let n = self.context.math.n;
                            let hit_sound = f64::from(self.state.velocity.z) < n.mul(parameters.gravity, -0.1);
                            self.gravity();
                            self.velocity_bounds();
                            self.fly_move(self.frame_seconds);
                            self.link(true)?;
                            if !self.context.removed && hit_sound && self.state.flags & Q1_FLAG_ONGROUND != 0 {
                                if let Some(hooks) = self.context.options.hooks.as_mut() {
                                    let actor = self.context.input.actor().clone();
                                    let state = self.state.clone();
                                    hooks.sound(&actor, Q1MovementSound::DemonLand, &state);
                                }
                            }
                        }
                        if !self.context.removed {
                            self.water_transition();
                        }
                    }
                    other => {
                        return Err(MovementError::Contract(Box::leak(
                            format!("Unsupported NetQuake player movetype {other}").into_boxed_str(),
                        )));
                    }
                }
            }
        }
        if !self.context.removed {
            self.link(true)?;
            let state = self
                .context
                .lifecycle(Q1State::Netquake(self.state.clone()), Q1LifecyclePhase::AfterPhysics);
            self.set_state(state)?;
        }
        if self.context.removed {
            return Ok(MovementOutcome::ActorRemoved {
                actor: self.context.input.actor().id().clone(),
                command_sequence: self.context.input.fields().command_sequence,
                effects: std::mem::take(&mut self.context.effects),
            });
        }
        self.ideal_pitch();
        let ground = if self.state.flags & Q1_FLAG_ONGROUND != 0 {
            self.state.ground.clone()
        } else {
            NONE
        };
        finish_netquake(
            &mut self.context,
            self.state.clone(),
            self.state.view_angles,
            ground,
            self.state.water_level,
            self.state.water_type,
        )
    }
}

/// Prepare a NetQuake step (client think).
pub fn prepare_netquake<S: Q1MovementServices, H: Q1MovementHooks>(
    input: Q1MovementInput,
    services: &mut S,
    options: Q1MovementOptions<H>,
) -> Result<Q1MovementState, MovementError> {
    NetQuakeMove::new(input, services, options)?.prepare()
}

/// Run NetQuake physics for one step.
pub fn physics_netquake<S: Q1MovementServices, H: Q1MovementHooks>(
    input: Q1MovementInput,
    services: &mut S,
    options: Q1MovementOptions<H>,
) -> Result<Q1MovementResult, MovementError> {
    NetQuakeMove::new(input, services, options)?.physics_step()
}

/// Run a full NetQuake step with input application when authoritative.
pub fn move_netquake<S: Q1MovementServices, H: Q1MovementHooks>(
    input: Q1MovementInput,
    services: &mut S,
    options: Q1MovementOptions<H>,
) -> Result<Q1MovementResult, MovementError> {
    let authoritative = input.fields.execution == MovementExecution::Authoritative;
    let has_application = authoritative && services.input_application().is_some();
    if !has_application {
        // One mover runs prepare then physics; prepare emits no contacts or
        // effects, so sharing the step context matches the donor's two
        // movers while keeping hooks borrowed once.
        let mut mover = NetQuakeMove::new(input, services, options)?;
        let prepared = mover.prepare()?;
        mover.state = prepared;
        return mover.physics_step();
    }
    let mut current = input.clone();
    let frame = current.fields.frame.clone();
    let before = services
        .input_application()
        .map(|application| {
            application.begin(
                UserCommand::Q1Netquake(current.command),
                &frame,
                Q1State::Netquake(current.state.clone()),
            )
        })
        .expect("input application present");
    let fail = |services: &mut S, current: &Q1MovementState| {
        services
            .input_application()
            .expect("input application present")
            .end(Q1State::Netquake(current.clone()), true);
    };
    let result = match before {
        MovementInputContinuation::ActorRemoved => MovementOutcome::ActorRemoved {
            actor: input.fields.actor.id().clone(),
            command_sequence: input.fields.command_sequence,
            effects: Vec::new(),
        },
        MovementInputContinuation::Continue { state, command } => {
            let (UserCommand::Q1Netquake(command), Q1State::Netquake(state)) = (command, state) else {
                fail(services, &current.state);
                return Err(MovementError::Contract("Input output changed NetQuake dialect"));
            };
            current = Q1MovementInput {
                command,
                state,
                ..current.clone()
            };
            let mut mover = match NetQuakeMove::new(current.clone(), services, options) {
                Ok(mover) => mover,
                Err(error) => {
                    fail(services, &current.state);
                    return Err(error);
                }
            };
            let prepared = match mover.prepare() {
                Ok(prepared) => prepared,
                Err(error) => {
                    fail(services, &current.state);
                    return Err(error);
                }
            };
            mover.state = prepared;
            match mover.physics_step() {
                Ok(result) => result,
                Err(error) => {
                    fail(services, &current.state);
                    return Err(error);
                }
            }
        }
    };
    let end_state = match &result {
        MovementOutcome::Active { state, .. } => Q1State::Netquake(state.clone()),
        MovementOutcome::ActorRemoved { .. } => Q1State::Netquake(current.state.clone()),
    };
    let after = services
        .input_application()
        .expect("input application present")
        .end(end_state, false);
    match (after, result) {
        (MovementContinuation::ActorRemoved, result) => Ok(MovementOutcome::ActorRemoved {
            actor: input.fields.actor.id().clone(),
            command_sequence: input.fields.command_sequence,
            effects: match result {
                MovementOutcome::Active { fields, .. } => fields.effects,
                MovementOutcome::ActorRemoved { effects, .. } => effects,
            },
        }),
        (
            MovementContinuation::Continue(_),
            MovementOutcome::ActorRemoved {
                actor,
                command_sequence,
                effects,
            },
        ) => Ok(MovementOutcome::ActorRemoved {
            actor,
            command_sequence,
            effects,
        }),
        (MovementContinuation::Continue(state), MovementOutcome::Active { fields, .. }) => {
            let Q1State::Netquake(state) = state else {
                return Err(MovementError::Contract("Input completion changed NetQuake dialect"));
            };
            Ok(MovementOutcome::Active { fields, state })
        }
    }
}

/// NetQuake movement provider.
#[derive(Debug, Clone)]
pub struct Q1MovementProvider<H = NoQ1Hooks> {
    /// Provider identity.
    pub id: ProviderId,
    /// Movement options.
    pub options: Q1MovementOptions<H>,
}

/// Build a NetQuake movement provider.
pub fn create_q1_movement_provider<H>(id: ProviderId, options: Q1MovementOptions<H>) -> Q1MovementProvider<H> {
    Q1MovementProvider { id, options }
}

impl<H: Q1MovementHooks + Clone> Q1MovementProvider<H> {
    /// Run a NetQuake step.
    pub fn move_step<S: Q1MovementServices>(
        &self,
        input: Q1MovementInput,
        services: &mut S,
    ) -> Result<Q1MovementResult, MovementError> {
        move_netquake(input, services, self.options.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec3, Bounds};
    use qa_core::numeric::{NumericOps, Q1_DONOR_PROFILE};
    use qa_core::time::{FrameContext, FramePhase, SourceTime};

    use super::super::super::types::{
        ActorAnimationState, AnimationState, ArsenalState, MovementEnvironment, MovementInputFields,
        MovementTouchContact, Q1UserCommand, TraceContact, WeaponState,
    };
    use super::super::types::{
        Q1AnimationStepInput, Q1AnimationStepResult, Q1TraceQuery, Q1WeaponStepInput, Q1WeaponStepResult,
    };

    struct NullServices {
        ops: NumericOps,
        contents: i32,
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
            self.contents
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

    fn fixture() -> (Q1MovementInput, NullServices) {
        let owner = IdentityOwner::create("q1-netquake").unwrap();
        let id = owner.actor(1, 0);
        let actor = owner.owned_actor(&id, ProviderId::new("q1", "test")).unwrap();
        let input = Q1MovementInput {
            fields: MovementInputFields {
                actor,
                command_sequence: 1,
                frame: FrameContext {
                    frame: 1,
                    time: SourceTime::Seconds(1.0),
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
            },
            command: Q1UserCommand {
                acknowledged_server_time_seconds: 0.0,
                view_angles: vec3(0.0, 90.0, 0.0),
                forward_move: 400.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0,
                impulse: 0,
            },
            state: Q1MovementState {
                origin: vec3(0.0, 0.0, 100.0),
                velocity: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 90.0, 0.0),
                old_origin: vec3(0.0, 0.0, 100.0),
                angular_velocity: vec3(0.0, 0.0, 0.0),
                view_angles: vec3(0.0, 0.0, 0.0),
                punch_angles: vec3(0.0, 0.0, 0.0),
                move_type: Q1_MOVE_WALK,
                flags: Q1_FLAG_ONGROUND | Q1_FLAG_JUMPRELEASED,
                ground: TraceHit::World { model: 0 },
                water_level: 0,
                water_type: Q1_CONTENTS_EMPTY,
                teleport_time_seconds: 0.0,
                water_jump_direction: vec3(0.0, 0.0, 0.0),
                ideal_pitch: 0.0,
                fix_angle: false,
                health: 100.0,
            },
            profile: super::super::types::Q1MovementProfile {
                id: ProviderId::new("q1", "test"),
                clock: ClockProfile::Q1Netquake {
                    minimum_frame_seconds: 0.001,
                    maximum_frame_seconds: 0.1,
                    fixed_frame_seconds: None,
                },
                numeric: Q1_DONOR_PROFILE,
                edition: Q1Edition::Classic,
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
            },
        };
        let services = NullServices {
            ops: NumericOps::select(Q1_DONOR_PROFILE).unwrap(),
            contents: Q1_CONTENTS_EMPTY,
        };
        (input, services)
    }

    #[test]
    fn prepare_applies_command_view_angles() {
        let (input, mut services) = fixture();
        let state = prepare_netquake(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default()).unwrap();
        assert_eq!(state.view_angles, vec3(0.0, 90.0, 0.0));
    }

    #[test]
    fn walk_step_applies_gravity_and_wish() {
        let (input, mut services) = fixture();
        let result = move_netquake(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default()).unwrap();
        match result {
            MovementOutcome::Active { fields, state } => {
                assert!(state.velocity.y > 0.0);
                assert!(fields.horizontal_speed > 0.0);
                assert_eq!(fields.command_sequence, 1);
            }
            MovementOutcome::ActorRemoved { .. } => panic!("unexpected removal"),
        }
    }

    #[test]
    fn jump_button_launches_grounded_player() {
        let (mut input, mut services) = fixture();
        input.command.buttons = 2;
        let result = move_netquake(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default()).unwrap();
        match result {
            MovementOutcome::Active { state, .. } => {
                assert_eq!(state.flags & Q1_FLAG_ONGROUND, 0);
                assert!(state.velocity.z > 200.0);
            }
            MovementOutcome::ActorRemoved { .. } => panic!("unexpected removal"),
        }
    }

    #[test]
    fn noclip_integrates_without_gravity() {
        let (mut input, mut services) = fixture();
        input.state.move_type = Q1_MOVE_NOCLIP;
        // Noclip velocity follows the wish directly; forward 400 at yaw 90
        // caps to max speed 320 along +Y.
        let result = move_netquake(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default()).unwrap();
        match result {
            MovementOutcome::Active { state, .. } => {
                assert_eq!(state.velocity.y, 320.0);
                assert_eq!(state.origin.y, 16.0);
                assert_eq!(state.origin.z, 100.0);
            }
            MovementOutcome::ActorRemoved { .. } => panic!("unexpected removal"),
        }
        let _ = Q1UserCommand {
            acknowledged_server_time_seconds: 0.0,
            view_angles: vec3(0.0, 0.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        };
    }

    #[test]
    fn invalid_profiles_are_rejected() {
        let (mut input, mut services) = fixture();
        input.profile.edition = Q1Edition::Quake64;
        assert!(move_netquake(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default()).is_err());
        let (mut input, mut services) = fixture();
        input.profile.clock = ClockProfile::Q1Quakeworld {
            maximum_command_milliseconds: 50.0,
        };
        assert!(move_netquake(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default()).is_err());
        let (mut input, mut services) = fixture();
        input.fields.frame.elapsed = SourceTime::Seconds(-1.0);
        assert!(move_netquake(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default()).is_err());
    }

    #[test]
    fn gib_requires_rerelease() {
        let (mut input, mut services) = fixture();
        input.state.move_type = Q1_MOVE_GIB;
        assert!(move_netquake(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default()).is_err());
        let (mut input, mut services) = fixture();
        input.state.move_type = Q1_MOVE_GIB;
        input.profile.edition = Q1Edition::Rerelease;
        assert!(move_netquake(input, &mut services, Q1MovementOptions::<NoQ1Hooks>::default()).is_ok());
    }
}
