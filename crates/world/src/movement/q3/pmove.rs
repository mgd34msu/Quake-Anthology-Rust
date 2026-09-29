//! Quake III player movement (`PmoveSingle`, `Pmove` from `bg_pmove.c`).
//!
//! Donor provenance: `src/movement/q3/move.ts` (ported from id Software
//! `code/game/bg_pmove.c`). Named `pmove` because `move` is a strict Rust
//! keyword.

use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, vec3, AngleVectors, Bounds, Vec3};
use qa_core::numeric::qvm_float_to_int;

use super::super::types::{MovementError, TraceContact, TraceHit};
use super::constants::{command_buttons as B, entity_event, move_flags as F, move_type, player_animation as A};
use super::postures::q3_invulnerability_pose;
use super::slide_move::{clip_velocity, slide_move, step_slide_move, SlideMoveContext, SlideSink};
use super::types::{Q3AnimationRequest, Q3Command, Q3Motion, Q3MotionDriver, Q3MotionOptions, Q3MotionResult, Q3Trace};
use super::view::q3_view_angles;

const ALL_TIMES: i32 = F::TIME_WATERJUMP | F::TIME_LAND | F::TIME_KNOCKBACK;
const MASK_WATER: i32 = 32 | 16 | 8;
const CONTENTS_BODY: i32 = 0x2000000;
const SURF_SLICK: i32 = 2;
const SURF_NODAMAGE: i32 = 1;
const SURF_METALSTEPS: i32 = 0x1000;
const SURF_NOSTEPS: i32 = 0x2000;
const ZERO_BOUNDS: Bounds = Bounds {
    min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
    max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
};
const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

/// Drop locomotion timers.
pub fn drop_q3_movement_timers(motion: &mut Q3Motion, milliseconds: i32) {
    if motion.pm_time != 0 {
        if milliseconds >= motion.pm_time {
            motion.pm_flags &= !ALL_TIMES;
            motion.pm_time = 0;
        } else {
            motion.pm_time -= milliseconds;
        }
    }
}

/// x87 fistp / C rint semantics used by the source engine's `Sys_SnapVector`.
#[must_use]
pub fn snap(value: f32) -> f32 {
    let floor = value.floor();
    let fraction = value - floor;
    if fraction < 0.5 {
        floor
    } else if fraction > 0.5 {
        floor + 1.0
    } else if floor % 2.0 == 0.0 {
        floor
    } else {
        floor + 1.0
    }
}

/// Update view angles from the command.
pub fn update_view_angles(motion: &mut Q3Motion, command: &Q3Command) {
    let value = q3_view_angles(
        command.angles,
        motion.delta_angles,
        motion.viewangles,
        motion.health,
        motion.pm_type,
        &[move_type::INTERMISSION, move_type::SPINTERMISSION],
    );
    motion.viewangles = value.angles;
    motion.delta_angles = value.delta;
}

/// QVM each-op angle vectors, mirroring donor `qvmAngleVectors`.
#[must_use]
pub fn qvm_angle_vectors(angles: Vec3) -> AngleVectors {
    let radians = std::f32::consts::PI * 2.0 / 360.0;
    let yaw = angles.y * radians;
    let pitch = angles.x * radians;
    let roll = angles.z * radians;
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let (sr, cr) = roll.sin_cos();
    AngleVectors {
        forward: vec3(cp * cy, cp * sy, -sp),
        right: vec3((-sr * sp) * cy + -cr * -sy, (-sr * sp) * sy + -cr * cy, -sr * cp),
        up: vec3((cr * sp) * cy + -sr * -sy, (cr * sp) * sy + -sr * cy, cr * cp),
    }
}

/// Grapple pull velocity.
/// Donor provenance: `src/content/q3/base/game/grapple.ts`
/// (`q3GrappleVelocity`, `Q3_GRAPPLE_SPEED`).
#[must_use]
pub fn q3_grapple_velocity(origin: Vec3, point: Vec3, forward: Vec3) -> Vec3 {
    let pull = sub3(add3(point, scale3(forward, -16.0)), origin);
    let distance = length3(pull);
    scale3(
        normalize3(pull),
        if distance <= 100.0 { 10.0 * distance } else { 800.0 },
    )
}

struct MoveStep<'s, 'c, 'o, 'x> {
    state: &'s mut Q3Motion,
    cmd: &'c mut Q3Command,
    options: &'o mut Q3MotionOptions<'x>,
    contacts: Vec<Q3Trace>,
    bounds: Bounds,
    ground_normal: Option<Vec3>,
    ground_surface_flags: i32,
    walking: bool,
    impact_speed: f32,
    waterlevel: i32,
    watertype: i32,
    xyspeed: f32,
    forward: Vec3,
    right: Vec3,
    frame_time: f32,
    msec: i32,
    previous_origin: Vec3,
    previous_velocity: Vec3,
    mask: i32,
}

impl<'s, 'c, 'o, 'x> MoveStep<'s, 'c, 'o, 'x> {
    fn new(state: &'s mut Q3Motion, cmd: &'c mut Q3Command, options: &'o mut Q3MotionOptions<'x>) -> Self {
        let bounds = options.current_bounds.unwrap_or(options.standing_bounds);
        let msec = (cmd.server_time - state.command_time).clamp(1, 200);
        let frame_time = (msec as f32) * 0.001;
        let previous_origin = state.origin;
        let previous_velocity = state.velocity;
        let base = options.trace_mask;
        let mask = if state.health <= 0.0 {
            base & !CONTENTS_BODY
        } else {
            base
        };
        Self {
            state,
            cmd,
            options,
            contacts: Vec::new(),
            bounds,
            ground_normal: None,
            ground_surface_flags: 0,
            walking: false,
            impact_speed: 0.0,
            waterlevel: 0,
            watertype: 0,
            xyspeed: 0.0,
            forward: ZERO,
            right: ZERO,
            frame_time,
            msec,
            previous_origin,
            previous_velocity,
            mask,
        }
    }

    fn event(&mut self, event: i32) {
        self.options.driver.event(event, &mut *self.state);
    }

    fn debug(&mut self, message: &str) {
        self.options.driver.debug(message);
    }

    fn touch(&mut self, trace: &Q3Trace) {
        let TraceHit::Actor { actor } = &trace.hit else {
            return;
        };
        if self.contacts.len() >= 32 {
            return;
        }
        if self.contacts.iter().any(|previous| match &previous.hit {
            TraceHit::Actor { actor: other } => qa_core::identity::same_actor(other, actor),
            _ => false,
        }) {
            return;
        }
        self.contacts.push(trace.clone());
        self.options.driver.contact(trace);
    }

    fn test(&mut self, start: Vec3, end: Vec3) -> Q3Trace {
        let actor = self.state.actor.clone();
        let bounds = self.bounds;
        let mask = self.mask;
        (self.options.trace)(start, end, bounds, actor, mask)
    }

    fn contents(&mut self, point: Vec3) -> i32 {
        let actor = self.state.actor.clone();
        (self.options.point_contents)(point, actor)
    }

    fn legs(&mut self, animation: i32, force: bool) {
        self.options
            .driver
            .animation(Q3AnimationRequest::Legs { animation, force }, &mut *self.state);
    }

    fn jump_animation(&mut self) {
        if self.cmd.forwardmove >= 0 {
            self.legs(A::LEGS_JUMP, true);
            self.state.pm_flags &= !F::BACKWARDS_JUMP;
        } else {
            self.legs(A::LEGS_JUMPB, true);
            self.state.pm_flags |= F::BACKWARDS_JUMP;
        }
    }

    fn slide(&mut self, gravity: bool) -> bool {
        struct Sink<'a> {
            contacts: &'a mut Vec<Q3Trace>,
            driver: &'a mut dyn Q3MotionDriver,
        }

        impl SlideSink for Sink<'_> {
            fn touch(&mut self, trace: &Q3Trace) {
                let TraceHit::Actor { actor } = &trace.hit else {
                    return;
                };
                if self.contacts.len() >= 32 {
                    return;
                }
                if self.contacts.iter().any(|previous| match &previous.hit {
                    TraceHit::Actor { actor: other } => qa_core::identity::same_actor(other, actor),
                    _ => false,
                }) {
                    return;
                }
                self.contacts.push(trace.clone());
                self.driver.contact(trace);
            }

            fn event(&mut self, event: i32, motion: &mut Q3Motion) {
                self.driver.event(event, motion);
            }

            fn debug(&mut self, message: &str) {
                self.driver.debug(message);
            }
        }

        let mut sink = Sink {
            contacts: &mut self.contacts,
            driver: &mut *self.options.driver,
        };
        let mut context = SlideMoveContext {
            motion: &mut *self.state,
            frame_time: self.frame_time,
            bounds: self.bounds,
            mask: self.mask,
            trace: &mut *self.options.trace,
            ground_normal: self.ground_normal,
            impact_speed: self.impact_speed,
            sink: &mut sink,
        };
        let out = slide_move(&mut context, gravity);
        self.impact_speed = context.impact_speed;
        out
    }

    fn step_slide(&mut self, gravity: bool) {
        struct Sink<'a> {
            contacts: &'a mut Vec<Q3Trace>,
            driver: &'a mut dyn Q3MotionDriver,
        }

        impl SlideSink for Sink<'_> {
            fn touch(&mut self, trace: &Q3Trace) {
                let TraceHit::Actor { actor } = &trace.hit else {
                    return;
                };
                if self.contacts.len() >= 32 {
                    return;
                }
                if self.contacts.iter().any(|previous| match &previous.hit {
                    TraceHit::Actor { actor: other } => qa_core::identity::same_actor(other, actor),
                    _ => false,
                }) {
                    return;
                }
                self.contacts.push(trace.clone());
                self.driver.contact(trace);
            }

            fn event(&mut self, event: i32, motion: &mut Q3Motion) {
                self.driver.event(event, motion);
            }

            fn debug(&mut self, message: &str) {
                self.driver.debug(message);
            }
        }

        let mut sink = Sink {
            contacts: &mut self.contacts,
            driver: &mut *self.options.driver,
        };
        let mut context = SlideMoveContext {
            motion: &mut *self.state,
            frame_time: self.frame_time,
            bounds: self.bounds,
            mask: self.mask,
            trace: &mut *self.options.trace,
            ground_normal: self.ground_normal,
            impact_speed: self.impact_speed,
            sink: &mut sink,
        };
        step_slide_move(&mut context, gravity);
        self.impact_speed = context.impact_speed;
    }

    fn friction(&mut self) {
        let velocity = self.state.velocity;
        let speed = length3(if self.walking {
            vec3(velocity.x, velocity.y, 0.0)
        } else {
            velocity
        });
        if speed < 1.0 {
            self.state.velocity = vec3(0.0, 0.0, velocity.z);
            return;
        }
        let mut drop = 0.0;
        if self.waterlevel <= 1
            && self.walking
            && self.ground_surface_flags & SURF_SLICK == 0
            && self.state.pm_flags & F::TIME_KNOCKBACK == 0
        {
            drop = speed.max(100.0) * 6.0 * self.frame_time;
        }
        if self.waterlevel != 0 {
            drop += speed * self.waterlevel as f32 * self.frame_time;
        }
        if self.state.flight {
            drop += speed * 3.0 * self.frame_time;
        }
        if self.state.pm_type == move_type::SPECTATOR {
            drop += speed * 5.0 * self.frame_time;
        }
        self.state.velocity = scale3(velocity, 0.0f32.max(speed - drop) / speed);
    }

    fn accelerate(&mut self, direction: Vec3, speed: f32, acceleration: f32) {
        let current = dot3(self.state.velocity, direction);
        let add = speed - current;
        if add <= 0.0 {
            return;
        }
        let amount = ((acceleration * self.frame_time) * speed).min(add);
        self.state.velocity = add3(self.state.velocity, scale3(direction, amount));
    }

    fn command_scale(&self) -> f32 {
        let (f, r, u) = (
            self.cmd.forwardmove as f32,
            self.cmd.rightmove as f32,
            self.cmd.upmove as f32,
        );
        let max = f.abs().max(r.abs()).max(u.abs());
        let total = (f * f + r * r + u * u).sqrt();
        if max == 0.0 {
            0.0
        } else {
            (self.state.speed * max) / (127.0 * total)
        }
    }

    fn movement_direction(&mut self) {
        let (f, r) = (self.cmd.forwardmove, self.cmd.rightmove);
        if f != 0 || r != 0 {
            self.state.movement_dir = if f > 0 {
                if r < 0 {
                    1
                } else if r > 0 {
                    7
                } else {
                    0
                }
            } else if f < 0 {
                if r < 0 {
                    3
                } else if r > 0 {
                    5
                } else {
                    4
                }
            } else if r < 0 {
                2
            } else {
                6
            };
        } else if self.state.movement_dir == 2 {
            self.state.movement_dir = 1;
        } else if self.state.movement_dir == 6 {
            self.state.movement_dir = 7;
        }
    }

    fn check_jump(&mut self) -> bool {
        if self.state.pm_flags & F::RESPAWNED != 0 || self.cmd.upmove < 10 {
            return false;
        }
        if self.state.pm_flags & F::JUMP_HELD != 0 {
            self.cmd.upmove = 0;
            return false;
        }
        self.ground_normal = None;
        self.walking = false;
        self.state.pm_flags |= F::JUMP_HELD;
        self.state.ground = TraceHit::None;
        self.state.velocity = vec3(self.state.velocity.x, self.state.velocity.y, 270.0);
        self.event(entity_event::JUMP);
        self.jump_animation();
        true
    }

    fn check_water_jump(&mut self) -> bool {
        if self.state.pm_time != 0 || self.waterlevel != 2 {
            return false;
        }
        let flat = normalize3(vec3(self.forward.x, self.forward.y, 0.0));
        let point = add3(self.state.origin, scale3(flat, 30.0));
        if self.contents(vec3(point.x, point.y, point.z + 4.0)) & 1 == 0
            || self.contents(vec3(point.x, point.y, point.z + 20.0)) != 0
        {
            return false;
        }
        let velocity = scale3(self.forward, 200.0);
        self.state.velocity = vec3(velocity.x, velocity.y, 350.0);
        self.state.pm_flags |= F::TIME_WATERJUMP;
        self.state.pm_time = 2000;
        true
    }

    fn water_jump_move(&mut self) {
        self.step_slide(true);
        self.state.velocity = vec3(
            self.state.velocity.x,
            self.state.velocity.y,
            self.state.velocity.z - self.state.gravity * self.frame_time,
        );
        if self.state.velocity.z < 0.0 {
            self.state.pm_flags &= !ALL_TIMES;
            self.state.pm_time = 0;
        }
    }

    fn wish_velocity(&self, scale: f32) -> Vec3 {
        let component = |forward: f32, right: f32| {
            (scale * forward) * self.cmd.forwardmove as f32 + (scale * right) * self.cmd.rightmove as f32
        };
        vec3(
            component(self.forward.x, self.right.x),
            component(self.forward.y, self.right.y),
            component(self.forward.z, self.right.z) + scale * self.cmd.upmove as f32,
        )
    }

    fn water_move(&mut self) {
        if self.check_water_jump() {
            self.water_jump_move();
            return;
        }
        self.friction();
        let scale = self.command_scale();
        let wish = if scale == 0.0 {
            vec3(0.0, 0.0, -60.0)
        } else {
            self.wish_velocity(scale)
        };
        let speed = length3(wish).min(self.state.speed * 0.5);
        let direction = normalize3(wish);
        self.accelerate(direction, speed, 4.0);
        if let Some(ground) = self.ground_normal {
            if dot3(self.state.velocity, ground) < 0.0 {
                let speed = length3(self.state.velocity);
                self.state.velocity = scale3(normalize3(clip_velocity(self.state.velocity, ground)), speed);
            }
        }
        self.slide(false);
    }

    fn fly_move(&mut self) {
        self.friction();
        let scale = self.command_scale();
        let wish = self.wish_velocity(scale);
        let speed = length3(wish);
        let direction = normalize3(wish);
        self.accelerate(direction, speed, 8.0);
        self.step_slide(false);
    }

    fn air_move(&mut self) {
        self.friction();
        let scale = self.command_scale();
        self.movement_direction();
        self.forward = normalize3(vec3(self.forward.x, self.forward.y, 0.0));
        self.right = normalize3(vec3(self.right.x, self.right.y, 0.0));
        let wish = add3(
            scale3(self.forward, self.cmd.forwardmove as f32),
            scale3(self.right, self.cmd.rightmove as f32),
        );
        let direction = normalize3(wish);
        self.accelerate(direction, length3(wish) * scale, 1.0);
        if let Some(ground) = self.ground_normal {
            self.state.velocity = clip_velocity(self.state.velocity, ground);
        }
        self.step_slide(true);
    }

    fn grapple_move(&mut self) {
        self.state.velocity = q3_grapple_velocity(self.state.origin, self.state.grapple_point, self.forward);
        self.ground_normal = None;
    }

    fn walk_move(&mut self) {
        let Some(normal) = self.ground_normal else {
            panic!("Walking requires a ground plane");
        };
        if self.waterlevel > 2 && dot3(self.forward, normal) > 0.0 {
            self.water_move();
            return;
        }
        if self.check_jump() {
            if self.waterlevel > 1 {
                self.water_move();
            } else {
                self.air_move();
            }
            return;
        }
        self.friction();
        let scale = self.command_scale();
        self.movement_direction();
        self.forward = normalize3(clip_velocity(vec3(self.forward.x, self.forward.y, 0.0), normal));
        self.right = normalize3(clip_velocity(vec3(self.right.x, self.right.y, 0.0), normal));
        let wish = add3(
            scale3(self.forward, self.cmd.forwardmove as f32),
            scale3(self.right, self.cmd.rightmove as f32),
        );
        let mut wish_speed = length3(wish) * scale;
        if self.state.pm_flags & F::DUCKED != 0 {
            wish_speed = wish_speed.min(self.state.speed * 0.25);
        }
        if self.waterlevel != 0 {
            let water_scale = 1.0 - 0.5 * (self.waterlevel as f32 / 3.0);
            wish_speed = wish_speed.min(self.state.speed * water_scale);
        }
        let sliding = self.ground_surface_flags & SURF_SLICK != 0 || self.state.pm_flags & F::TIME_KNOCKBACK != 0;
        let direction = normalize3(wish);
        self.accelerate(direction, wish_speed, if sliding { 1.0 } else { 10.0 });
        if sliding {
            self.state.velocity = vec3(
                self.state.velocity.x,
                self.state.velocity.y,
                self.state.velocity.z - self.state.gravity * self.frame_time,
            );
        }
        let speed = length3(self.state.velocity);
        self.state.velocity = scale3(normalize3(clip_velocity(self.state.velocity, normal)), speed);
        if self.state.velocity.x != 0.0 || self.state.velocity.y != 0.0 {
            self.step_slide(false);
        }
    }

    fn noclip_move(&mut self) {
        self.state.viewheight = self.options.postures.standing_view_height;
        let speed = length3(self.state.velocity);
        self.state.velocity = if speed < 1.0 {
            ZERO
        } else {
            scale3(
                self.state.velocity,
                0.0f32.max(speed - (100.0f32.max(speed) * 9.0) * self.frame_time) / speed,
            )
        };
        let wish = self.wish_velocity(1.0);
        let length = length3(wish);
        let direction = normalize3(wish);
        self.accelerate(direction, length * self.command_scale(), 10.0);
        self.state.origin = add3(self.state.origin, scale3(self.state.velocity, self.frame_time));
    }

    fn footstep_for_surface(&self) -> i32 {
        if self.ground_surface_flags & SURF_NOSTEPS != 0 {
            0
        } else if self.ground_surface_flags & SURF_METALSTEPS != 0 {
            entity_event::FOOTSTEP_METAL
        } else {
            entity_event::FOOTSTEP
        }
    }

    fn crash_land(&mut self) {
        self.legs(
            if self.state.pm_flags & F::BACKWARDS_JUMP != 0 {
                A::LEGS_LANDB
            } else {
                A::LEGS_LAND
            },
            true,
        );
        self.options
            .driver
            .animation(Q3AnimationRequest::LegsTimer { milliseconds: 130 }, &mut *self.state);
        let dist = self.state.origin.z - self.previous_origin.z;
        let velocity = self.previous_velocity.z;
        let acceleration = qvm_float_to_int(-self.state.gravity) as f32;
        let a = acceleration / 2.0;
        let discriminant = velocity * velocity - (4.0 * a) * -dist;
        if discriminant < 0.0 {
            return;
        }
        let t = (-velocity - discriminant.sqrt()) / (2.0 * a);
        let mut delta = velocity + t * acceleration;
        delta = (delta * delta) * 0.0001;
        if self.state.pm_flags & F::DUCKED != 0 {
            delta *= 2.0;
        }
        if self.waterlevel == 3 {
            return;
        }
        if self.waterlevel == 2 {
            delta *= 0.25;
        }
        if self.waterlevel == 1 {
            delta *= 0.5;
        }
        if delta < 1.0 {
            return;
        }
        if self.ground_surface_flags & SURF_NODAMAGE == 0 {
            if delta > 60.0 {
                self.event(entity_event::FALL_FAR);
            } else if delta > 40.0 {
                if self.state.health > 0.0 {
                    self.event(entity_event::FALL_MEDIUM);
                }
            } else if delta > 7.0 {
                self.event(entity_event::FALL_SHORT);
            } else {
                let footstep = self.footstep_for_surface();
                self.event(footstep);
            }
        }
        self.state.bob_cycle = 0;
    }

    fn ground_trace(&mut self) {
        let origin = self.state.origin;
        let down = vec3(origin.x, origin.y, origin.z - 0.25);
        let mut trace = self.test(origin, down);
        if trace.all_solid {
            self.debug("allsolid");
            let mut corrected = false;
            'unstick: for i in -1..=1 {
                for j in -1..=1 {
                    for k in -1..=1 {
                        let point = add3(origin, vec3(i as f32, j as f32, k as f32));
                        if !self.test(point, point).all_solid {
                            trace = self.test(origin, down);
                            corrected = true;
                            break 'unstick;
                        }
                    }
                }
            }
            if !corrected {
                self.leave_ground();
                return;
            }
        }
        self.ground_surface_flags = trace.surface_flags;
        if trace.fraction == 1.0 {
            if !matches!(self.state.ground, TraceHit::None) {
                self.debug("lift");
                let origin = self.state.origin;
                if self.test(origin, vec3(origin.x, origin.y, origin.z - 64.0)).fraction == 1.0 {
                    self.jump_animation();
                }
            }
            self.leave_ground();
            return;
        }
        // An unresolved all-solid trace has no normal in the collision contract.
        let TraceContact::Plane(plane) = trace.contact else {
            self.leave_ground();
            return;
        };
        let normal = plane.normal;
        if self.state.velocity.z > 0.0 && dot3(self.state.velocity, normal) > 10.0 {
            self.debug("kickoff");
            self.jump_animation();
            self.leave_ground();
            return;
        }
        self.ground_normal = Some(normal);
        if normal.z < 0.7 {
            self.debug("steep");
            self.state.ground = TraceHit::None;
            self.walking = false;
            return;
        }
        self.walking = true;
        if self.state.pm_flags & F::TIME_WATERJUMP != 0 {
            self.state.pm_flags &= !(F::TIME_WATERJUMP | F::TIME_LAND);
            self.state.pm_time = 0;
        }
        if matches!(self.state.ground, TraceHit::None) {
            self.debug("Land");
            self.crash_land();
            if self.previous_velocity.z < -200.0 {
                self.state.pm_flags |= F::TIME_LAND;
                self.state.pm_time = 250;
            }
        }
        self.state.ground = trace.hit.clone();
        self.touch(&trace);
    }

    fn leave_ground(&mut self) {
        self.state.ground = TraceHit::None;
        self.ground_normal = None;
        self.walking = false;
    }

    fn set_water_level(&mut self) {
        self.waterlevel = 0;
        self.watertype = 0;
        let origin = self.state.origin;
        let base = self.options.standing_bounds.min.z;
        let contents = self.contents(vec3(origin.x, origin.y, origin.z + base + 1.0));
        if contents & MASK_WATER == 0 {
            return;
        }
        self.watertype = contents;
        self.waterlevel = 1;
        let sample2 = self.state.viewheight as f32 - base;
        let sample1 = f64::from(sample2 / 2.0).trunc() as i32 as f32;
        if self.contents(vec3(origin.x, origin.y, origin.z + base + sample1)) & MASK_WATER == 0 {
            return;
        }
        self.waterlevel = 2;
        if self.contents(vec3(origin.x, origin.y, origin.z + base + sample2)) & MASK_WATER != 0 {
            self.waterlevel = 3;
        }
    }

    fn check_duck(&mut self) {
        let standing = self.options.standing_bounds;
        let previous_bounds = self.bounds;
        let previous_duck = self.state.pm_flags & F::DUCKED;
        let postures = self.options.postures;
        if self.state.invulnerable || self.options.pose.is_some() {
            let pose = self
                .options
                .pose
                .unwrap_or_else(|| q3_invulnerability_pose(self.state.pm_flags & F::INVULEXPAND != 0, &postures));
            self.bounds = pose.bounds;
            if pose.crouched {
                self.state.pm_flags |= F::DUCKED;
            } else {
                self.state.pm_flags &= !F::DUCKED;
            }
            self.state.viewheight = pose.view_height;
            return;
        }
        self.state.pm_flags &= !F::INVULEXPAND;
        if self.state.pm_type == move_type::DEAD {
            self.bounds = postures.dead.bounds;
            self.state.viewheight = postures.dead.view_height;
            return;
        }
        if self.cmd.upmove < 0 {
            self.state.pm_flags |= F::DUCKED;
        } else if self.state.pm_flags & F::DUCKED != 0 {
            self.bounds = self.options.body_bounds.unwrap_or(standing);
            if !self.test(self.state.origin, self.state.origin).all_solid {
                self.state.pm_flags &= !F::DUCKED;
            }
        }
        if self.state.pm_flags & F::DUCKED != 0 {
            self.bounds = postures.crouched.bounds;
            self.state.viewheight = postures.crouched.view_height;
        } else {
            self.bounds = standing;
            self.state.viewheight = postures.standing_view_height;
        }
        let requested = self.options.body_bounds.unwrap_or(self.bounds);
        let origin = self.state.origin;
        let actor = self.state.actor.clone();
        let mask = self.mask;
        let trace = &mut self.options.trace;
        let accepted = super::super::q2::dimensions::accept_body_bounds(&previous_bounds, &requested, |bounds| {
            !trace(origin, origin, *bounds, actor.clone(), mask).all_solid
        });
        self.bounds = accepted;
        if accepted != requested {
            self.state.pm_flags = (self.state.pm_flags & !F::DUCKED) | previous_duck;
            self.state.viewheight = if previous_duck != 0 {
                postures.crouched.view_height
            } else {
                postures.standing_view_height
            };
        }
    }

    fn footsteps(&mut self) {
        let velocity = self.state.velocity;
        self.xyspeed = (velocity.x * velocity.x + velocity.y * velocity.y).sqrt();
        if matches!(self.state.ground, TraceHit::None) {
            if self.state.invulnerable {
                self.legs(A::LEGS_IDLECR, false);
            }
            if self.waterlevel > 1 {
                self.legs(A::LEGS_SWIM, false);
            }
            return;
        }
        if self.cmd.forwardmove == 0 && self.cmd.rightmove == 0 {
            if self.xyspeed < 5.0 {
                self.state.bob_cycle = 0;
                self.legs(
                    if self.state.pm_flags & F::DUCKED != 0 {
                        A::LEGS_IDLECR
                    } else {
                        A::LEGS_IDLE
                    },
                    false,
                );
            }
            return;
        }
        let backwards = self.state.pm_flags & F::BACKWARDS_RUN != 0;
        let (bob, footstep) = if self.state.pm_flags & F::DUCKED != 0 {
            self.legs(if backwards { A::LEGS_BACKCR } else { A::LEGS_WALKCR }, false);
            (0.5f32, false)
        } else if self.cmd.buttons & B::WALKING == 0 {
            self.legs(if backwards { A::LEGS_BACK } else { A::LEGS_RUN }, false);
            (0.4f32, true)
        } else {
            self.legs(if backwards { A::LEGS_BACKWALK } else { A::LEGS_WALK }, false);
            (0.3f32, false)
        };
        let old = self.state.bob_cycle;
        self.state.bob_cycle = ((old as f32 + bob * self.msec as f32).trunc() as i32) & 255;
        if ((old + 64) ^ (self.state.bob_cycle + 64)) & 128 != 0 {
            if self.waterlevel == 0 && footstep && !self.options.no_footsteps {
                let footstep = self.footstep_for_surface();
                self.event(footstep);
            } else if self.waterlevel == 1 {
                self.event(entity_event::FOOTSPLASH);
            } else if self.waterlevel == 2 {
                self.event(entity_event::SWIM);
            }
        }
    }

    fn water_events(&mut self, previous: i32) {
        if previous == 0 && self.waterlevel != 0 {
            self.event(entity_event::WATER_TOUCH);
        }
        if previous != 0 && self.waterlevel == 0 {
            self.event(entity_event::WATER_LEAVE);
        }
        if previous != 3 && self.waterlevel == 3 {
            self.event(entity_event::WATER_UNDER);
        }
        if previous == 3 && self.waterlevel != 3 {
            self.event(entity_event::WATER_CLEAR);
        }
    }

    fn drop_timers(&mut self) {
        drop_q3_movement_timers(&mut *self.state, self.msec);
        self.options
            .driver
            .animation(Q3AnimationRequest::DropTimers, &mut *self.state);
    }

    fn run(&mut self) {
        if self.cmd.forwardmove.abs() > 64 || self.cmd.rightmove.abs() > 64 {
            self.cmd.buttons &= !B::WALKING;
        }
        if self.cmd.buttons & B::TALK != 0 {
            self.state.e_flags |= 0x1000;
        } else {
            self.state.e_flags &= !0x1000;
        }
        let firing = self.state.pm_flags & F::RESPAWNED == 0
            && self.state.pm_type != move_type::INTERMISSION
            && self.cmd.buttons & B::ATTACK != 0
            && self.options.driver.firing(&mut *self.state);
        if firing {
            self.state.e_flags |= 0x100;
        } else {
            self.state.e_flags &= !0x100;
        }
        if self.state.health > 0.0 && self.cmd.buttons & (B::ATTACK | B::USE_HOLDABLE) == 0 {
            self.state.pm_flags &= !F::RESPAWNED;
        }
        if self.cmd.buttons & B::TALK != 0 {
            self.cmd.buttons = B::TALK;
            self.cmd.forwardmove = 0;
            self.cmd.rightmove = 0;
            self.cmd.upmove = 0;
        }
        self.state.command_time = self.cmd.server_time;
        let cmd = *self.cmd;
        update_view_angles(self.state, &cmd);
        let axes = qvm_angle_vectors(self.state.viewangles);
        self.forward = axes.forward;
        self.right = axes.right;
        if self.cmd.upmove < 10 {
            self.state.pm_flags &= !F::JUMP_HELD;
        }
        if self.cmd.forwardmove < 0 {
            self.state.pm_flags |= F::BACKWARDS_RUN;
        } else if self.cmd.forwardmove > 0 || (self.cmd.forwardmove == 0 && self.cmd.rightmove != 0) {
            self.state.pm_flags &= !F::BACKWARDS_RUN;
        }
        if self.state.pm_type >= move_type::DEAD {
            self.cmd.forwardmove = 0;
            self.cmd.rightmove = 0;
            self.cmd.upmove = 0;
        }
        if self.state.pm_type == move_type::SPECTATOR {
            self.check_duck();
            self.fly_move();
            self.drop_timers();
            return;
        }
        if self.state.pm_type == move_type::NOCLIP {
            self.noclip_move();
            self.drop_timers();
            return;
        }
        if self.state.pm_type == move_type::FREEZE
            || self.state.pm_type == move_type::INTERMISSION
            || self.state.pm_type == move_type::SPINTERMISSION
        {
            return;
        }
        self.set_water_level();
        let previous_waterlevel = self.waterlevel;
        self.check_duck();
        self.ground_trace();
        if self.state.pm_type == move_type::DEAD && self.walking {
            let speed = length3(self.state.velocity) - 20.0;
            self.state.velocity = if speed <= 0.0 {
                ZERO
            } else {
                scale3(normalize3(self.state.velocity), speed)
            };
        }
        self.drop_timers();
        if self.options.pose.is_some()
            || (self.state.product == super::types::Q3Product::MissionPack && self.state.invulnerable)
        {
            self.cmd.forwardmove = 0;
            self.cmd.rightmove = 0;
            self.cmd.upmove = 0;
            self.state.velocity = ZERO;
        } else if self.state.flight {
            self.fly_move();
        } else if self.state.pm_flags & F::GRAPPLE_PULL != 0 {
            self.grapple_move();
            self.air_move();
        } else if self.state.pm_flags & F::TIME_WATERJUMP != 0 {
            self.water_jump_move();
        } else if self.waterlevel > 1 {
            self.water_move();
        } else if self.walking {
            self.walk_move();
        } else {
            self.air_move();
        }
        self.options
            .driver
            .animation(Q3AnimationRequest::Gesture, &mut *self.state);
        self.ground_trace();
        self.set_water_level();
        if !self.options.driver.weapon(&mut *self.state) {
            return;
        }
        self.options.driver.torso(&mut *self.state);
        self.footsteps();
        self.water_events(previous_waterlevel);
        self.state.velocity = vec3(
            snap(self.state.velocity.x),
            snap(self.state.velocity.y),
            snap(self.state.velocity.z),
        );
    }
}

/// Mutates only the supplied state; command input remains owned by the
/// caller. Mirrors donor `movePlayer`.
pub fn move_player(
    state: &mut Q3Motion,
    command: &Q3Command,
    options: &mut Q3MotionOptions<'_>,
) -> Result<Q3MotionResult, MovementError> {
    for axis in [command.forwardmove, command.rightmove, command.upmove] {
        if !(-128..=127).contains(&axis) {
            return Err(MovementError::Range("Command movement must be a signed byte"));
        }
    }
    let fixed = options.fixed_msec.unwrap_or(66);
    if fixed < 1 {
        return Err(MovementError::Range(
            "Movement step must be a positive signed 32-bit integer",
        ));
    }
    let mut result = Q3MotionResult {
        contacts: Vec::new(),
        bounds: ZERO_BOUNDS,
        waterlevel: 0,
        watertype: 0,
        xyspeed: 0.0,
    };
    let final_time = command.server_time;
    if final_time < state.command_time {
        return Ok(result);
    }
    if final_time > state.command_time + 1000 {
        state.command_time = final_time - 1000;
    }
    state.pmove_framecount = (state.pmove_framecount + 1) & 63;
    let mut cmd = *command;
    let mut substep = 0;
    while state.command_time != final_time {
        cmd.server_time = state.command_time + (final_time - state.command_time).min(fixed);
        let msec = (cmd.server_time - state.command_time).clamp(1, 200);
        let index = substep;
        substep += 1;
        if !options.driver.begin_step(state, &mut cmd, msec, index) {
            break;
        }
        result = {
            let mut step = MoveStep::new(&mut *state, &mut cmd, &mut *options);
            if substep > 1 {
                step.bounds = result.bounds;
            }
            step.run();
            Q3MotionResult {
                contacts: step.contacts,
                bounds: step.bounds,
                waterlevel: step.waterlevel,
                watertype: step.watertype,
                xyspeed: step.xyspeed,
            }
        };
        if !options.driver.end_step(state) {
            break;
        }
        if state.pm_flags & F::JUMP_HELD != 0 {
            cmd.upmove = 20;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hull::BspPlane;
    use qa_core::identity::{ActorId, IdentityOwner, ProviderId};

    use super::super::postures::Q3_SOURCE_POSTURES;

    struct NullDriver {
        events: Vec<i32>,
    }

    impl Q3MotionDriver for NullDriver {
        fn begin_step(
            &mut self,
            _motion: &mut Q3Motion,
            _command: &mut Q3Command,
            _msec: i32,
            _substep: usize,
        ) -> bool {
            true
        }
        fn event(&mut self, event: i32, _motion: &mut Q3Motion) {
            self.events.push(event);
        }
        fn animation(&mut self, _request: Q3AnimationRequest, _motion: &mut Q3Motion) {}
        fn weapon(&mut self, _motion: &mut Q3Motion) -> bool {
            true
        }
        fn torso(&mut self, _motion: &mut Q3Motion) {}
        fn firing(&mut self, _motion: &mut Q3Motion) -> bool {
            false
        }
        fn contact(&mut self, _trace: &Q3Trace) {}
    }

    fn motion() -> Q3Motion {
        let owner = IdentityOwner::create("q3-pmove").unwrap();
        let id = owner.actor(1, 0);
        let _ = owner.owned_actor(&id, ProviderId::new("q3", "test")).unwrap();
        Q3Motion {
            command_time: 0,
            pm_type: move_type::NORMAL,
            bob_cycle: 0,
            pm_flags: 0,
            pm_time: 0,
            origin: vec3(0.0, 0.0, 100.0),
            velocity: vec3(0.0, 0.0, 0.0),
            gravity: 800.0,
            speed: 320.0,
            delta_angles: vec3(0.0, 0.0, 0.0),
            ground: TraceHit::None,
            movement_dir: 0,
            grapple_point: vec3(0.0, 0.0, 0.0),
            e_flags: 0,
            viewangles: vec3(0.0, 0.0, 0.0),
            viewheight: 26.0,
            pmove_framecount: 0,
            event_sequence: 0,
            actor: id,
            health: 100.0,
            flight: false,
            invulnerable: false,
            product: super::super::types::Q3Product::BaseQ3,
        }
    }

    fn open_trace(end: Vec3) -> Q3Trace {
        Q3Trace {
            fraction: 1.0,
            end,
            start_solid: false,
            all_solid: false,
            contact: TraceContact::None,
            hit: TraceHit::None,
            contents: 0,
            surface_flags: 0,
            source_plane: BspPlane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
                plane_type: 0,
                signbits: 0,
            },
        }
    }

    fn command() -> Q3Command {
        Q3Command {
            server_time: 50,
            angles: vec3(0.0, 0.0, 0.0),
            buttons: 0,
            weapon: 2,
            forwardmove: 100,
            rightmove: 0,
            upmove: 0,
        }
    }

    #[test]
    fn open_air_move_accelerates_and_falls() {
        let mut state = motion();
        let mut driver = NullDriver { events: Vec::new() };
        let mut options = Q3MotionOptions {
            pose: None,
            trace: Box::new(|_s: Vec3, e: Vec3, _b: Bounds, _a: ActorId, _m: i32| open_trace(e)),
            point_contents: Box::new(|_p: Vec3, _a: ActorId| 0),
            standing_bounds: Bounds {
                min: vec3(-15.0, -15.0, -24.0),
                max: vec3(15.0, 15.0, 32.0),
            },
            current_bounds: None,
            body_bounds: None,
            postures: Q3_SOURCE_POSTURES,
            trace_mask: 1 | 0x10000 | 0x2000000,
            fixed_msec: None,
            no_footsteps: false,
            driver: &mut driver,
        };
        let result = move_player(&mut state, &command(), &mut options).unwrap();
        assert!(state.velocity.x > 0.0);
        assert!(state.velocity.z < 0.0);
        assert_eq!(state.command_time, 50);
        assert!(result.xyspeed > 0.0);
    }

    #[test]
    fn stale_commands_return_empty_result() {
        let mut state = motion();
        state.command_time = 100;
        let mut driver = NullDriver { events: Vec::new() };
        let mut options = Q3MotionOptions {
            pose: None,
            trace: Box::new(|_s: Vec3, e: Vec3, _b: Bounds, _a: ActorId, _m: i32| open_trace(e)),
            point_contents: Box::new(|_p: Vec3, _a: ActorId| 0),
            standing_bounds: Bounds {
                min: vec3(-15.0, -15.0, -24.0),
                max: vec3(15.0, 15.0, 32.0),
            },
            current_bounds: None,
            body_bounds: None,
            postures: Q3_SOURCE_POSTURES,
            trace_mask: 1,
            fixed_msec: None,
            no_footsteps: false,
            driver: &mut driver,
        };
        let result = move_player(&mut state, &command(), &mut options).unwrap();
        assert!(result.contacts.is_empty());
        assert_eq!(state.command_time, 100);
    }

    #[test]
    fn invalid_commands_are_rejected() {
        let mut state = motion();
        let mut driver = NullDriver { events: Vec::new() };
        let mut options = Q3MotionOptions {
            pose: None,
            trace: Box::new(|_s: Vec3, e: Vec3, _b: Bounds, _a: ActorId, _m: i32| open_trace(e)),
            point_contents: Box::new(|_p: Vec3, _a: ActorId| 0),
            standing_bounds: Bounds {
                min: vec3(-15.0, -15.0, -24.0),
                max: vec3(15.0, 15.0, 32.0),
            },
            current_bounds: None,
            body_bounds: None,
            postures: Q3_SOURCE_POSTURES,
            trace_mask: 1,
            fixed_msec: Some(0),
            no_footsteps: false,
            driver: &mut driver,
        };
        let mut bad = command();
        bad.forwardmove = 200;
        assert!(move_player(&mut state, &bad, &mut options).is_err());
        assert!(move_player(&mut state, &command(), &mut options).is_err());
    }

    #[test]
    fn snap_rounds_half_to_even() {
        assert_eq!(snap(2.5), 2.0);
        assert_eq!(snap(3.5), 4.0);
        assert_eq!(snap(2.4), 2.0);
        assert_eq!(snap(2.6), 3.0);
    }

    #[test]
    fn timers_drop_with_milliseconds() {
        let mut state = motion();
        state.pm_flags = F::TIME_LAND;
        state.pm_time = 100;
        drop_q3_movement_timers(&mut state, 50);
        assert_eq!(state.pm_time, 50);
        drop_q3_movement_timers(&mut state, 50);
        assert_eq!((state.pm_time, state.pm_flags & F::TIME_LAND), (0, 0));
    }

    #[test]
    fn grapple_pulls_toward_point() {
        let velocity = q3_grapple_velocity(vec3(0.0, 0.0, 0.0), vec3(1000.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0));
        assert!((velocity.x - 800.0).abs() < 0.001, "{}", velocity.x);
        let near = q3_grapple_velocity(vec3(0.0, 0.0, 0.0), vec3(50.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0));
        assert!(near.x < 800.0 && near.x > 0.0);
    }
}
