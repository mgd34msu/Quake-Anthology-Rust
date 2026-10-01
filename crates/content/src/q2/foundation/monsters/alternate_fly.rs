//! Alternate fly steering (`src/content/q2/foundation/monsters/alternate-fly.ts`).
//!
//! Quake II rerelease `m_move.cpp` `SV_alternate_flystep` and `q_vec3.h`
//! (id Software, GPL-2.0-or-later).
//!
//! The donor rounds every scalar through `Math.fround`; with `f32`
//! vectors each component operation rounds identically. Scalar
//! intermediates that the donor keeps in binary64 (`1 - t` factors,
//! speeds) stay `f32` here because every use rounds again; trigonometry
//! uses `f32` methods under the workspace's 1-ulp policy.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use super::types::{MonsterContext, MonsterState};
use crate::q2::foundation::host::{Q2GameServices, Q2TraceRequest};
use crate::q2::support::contracts::{BodyState, TraceResult};
use crate::q2::support::misc::Q2RereleaseRandomSource;

const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
const UP: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 1.0 };
const FIT_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -8.0,
        y: -8.0,
        z: -8.0,
    },
    max: Vec3 { x: 8.0, y: 8.0, z: 8.0 },
};
const SOLID_MASK: i32 = 1 | 2 | 0x20000;
const PI: f32 = std::f32::consts::PI;

fn add(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn scale(a: Vec3, b: f32) -> Vec3 {
    Vec3 {
        x: a.x * b,
        y: a.y * b,
        z: a.z * b,
    }
}

fn scaled(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x * b.x,
        y: a.y * b.y,
        z: a.z * b.z,
    }
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn nonzero(v: Vec3) -> bool {
    v.x != 0.0 || v.y != 0.0 || v.z != 0.0
}

fn nan(v: Vec3) -> bool {
    v.x.is_nan() || v.y.is_nan() || v.z.is_nan()
}

fn normal(v: Vec3) -> (Vec3, f32) {
    let length = dot(v, v).sqrt();
    (if length == 0.0 { v } else { scale(v, 1.0 / length) }, length)
}

fn yaw(v: Vec3) -> f32 {
    if v.x == 0.0 {
        return if v.y == 0.0 {
            0.0
        } else if v.y > 0.0 {
            90.0
        } else {
            270.0
        };
    }
    let angle = v.y.atan2(v.x) * (180.0 / PI);
    if angle < 0.0 {
        angle + 360.0
    } else {
        angle
    }
}

fn pitch(v: Vec3) -> f32 {
    if v.x == 0.0 && v.y == 0.0 {
        return if v.z > 0.0 { -90.0 } else { -270.0 };
    }
    let forward = (v.x * v.x + v.y * v.y).sqrt();
    let angle = v.z.atan2(forward) * (180.0 / PI);
    -(if angle < 0.0 { angle + 360.0 } else { angle })
}

fn slerp(from: Vec3, to: Vec3, t: f32) -> Vec3 {
    let product = dot(from, to);
    let (a, b) = if !(product.abs() > 0.9995) {
        let angle = product.acos();
        let sine = angle.sin();
        (((1.0 - t) * angle).sin() / sine, (t * angle).sin() / sine)
    } else {
        (1.0 - t, t)
    };
    add(scale(from, a), scale(to, b))
}

/// Alternate fly steering input (`Q2AlternateFlyInput`).
pub struct AlternateFlyInput<'a> {
    /// Monster state (steering fields are read and written).
    pub state: &'a mut MonsterState,
    /// Monster body.
    pub body: BodyState,
    /// Enemy body.
    pub enemy: Option<BodyState>,
    /// Goal point.
    pub goal: Option<Vec3>,
    /// Entity flags.
    pub flags: i64,
    /// Now.
    pub now: f64,
    /// Frame seconds.
    pub frame_seconds: f64,
}

/// Alternate fly steering services (`Q2AlternateFlyServices`).
pub trait AlternateFlyServices {
    /// Rerelease random stream.
    fn random(&mut self) -> &mut dyn Q2RereleaseRandomSource;
    /// Trace.
    fn trace(&mut self, start: Vec3, end: Vec3, bounds: Option<Bounds>, mask: i32) -> TraceResult;
    /// Point contents.
    fn point_contents(&mut self, point: Vec3) -> i32;
    /// Visible enemy check.
    fn visible_enemy(&mut self) -> bool;
}

/// Alternate fly steering result (`Q2AlternateFlyResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlternateFlyResult {
    /// Fall back to the standard fly step.
    Fallback,
    /// Steered velocity and optional pitch.
    Steered {
        /// Steering velocity.
        velocity: Vec3,
        /// New pitch.
        pitch: Option<f32>,
    },
}

fn ideal_hover(input: &mut AlternateFlyInput, services: &mut dyn AlternateFlyServices) -> Vec3 {
    let (medic, combat_point, sound_target, hint_path, pathing, fly_above, fly_buzzard, fly_min, fly_max);
    {
        let state = &mut *input.state;
        medic = state.medic;
        combat_point = state.combat_point;
        sound_target = state.sound_target.is_some();
        hint_path = state.hint_path;
        pathing = state.pathing.is_some();
        fly_above = state.fly_above;
        fly_buzzard = state.fly_buzzard;
        fly_min = state.fly_min_distance;
        fly_max = state.fly_max_distance;
    }
    if input.enemy.is_none() && !medic || combat_point || sound_target || hint_path || pathing {
        return ZERO;
    }
    let random = services.random();
    let theta = random.float_max(f64::from(2.0 * PI));
    let phi = if fly_above {
        (0.7 + random.float_max(f64::from(0.3))).acos()
    } else if fly_buzzard || medic {
        random.float_unit().acos()
    } else {
        (random.float_range(-1.0, 1.0) * 0.06).acos()
    };
    scale(
        Vec3 {
            x: phi.sin() * theta.cos(),
            y: phi.sin() * theta.sin(),
            z: phi.cos(),
        },
        random.float_range(fly_min, fly_max),
    )
}

/// Steer alternate flying (`steerQ2AlternateFly`).
///
/// Source AI only changes steering velocity and pitch. The shared
/// physics step remains responsible for moving the body, gravity,
/// impacts and relinking.
pub fn steer_alternate_fly(
    input: &mut AlternateFlyInput,
    services: &mut dyn AlternateFlyServices,
) -> AlternateFlyResult {
    let now = input.now;
    if input.flags & 2 != 0 && input.state.water_level < 3 {
        return AlternateFlyResult::Steered {
            velocity: input.body.velocity,
            pitch: None,
        };
    }
    let reposition = input.state.fly_position_time <= now
        || input.enemy.is_some() && input.state.fly_pinned && !services.visible_enemy();
    if reposition {
        input.state.fly_pinned = false;
        let position_time =
            ((now * 1000.0 + 0.5).floor() + services.random().time_milliseconds(3000, 10000) as f64) / 1000.0;
        input.state.fly_position_time = position_time;
        let ideal = ideal_hover(input, services);
        input.state.fly_ideal_position = ideal;
    }
    let (direction, mut current_speed) = normal(input.body.velocity);
    if nan(direction) {
        return AlternateFlyResult::Fallback;
    }
    let (target, target_velocity) = if let Some(pathing) = input.state.pathing {
        (
            if pathing.traversal_pending {
                pathing.second_move_point
            } else {
                pathing.first_move_point
            },
            ZERO,
        )
    } else if input.enemy.is_some()
        && !input.state.combat_point
        && input.state.sound_target.is_none()
        && !input.state.lost_sight
    {
        let enemy = input.enemy.clone().expect("enemy");
        (enemy.origin, enemy.velocity)
    } else if let Some(goal) = input.goal {
        (goal, ZERO)
    } else {
        if current_speed > 0.0 {
            current_speed = 0.0f32.max(current_speed - input.state.fly_acceleration as f32);
        } else if current_speed < 0.0 {
            current_speed = 0.0f32.min(current_speed + input.state.fly_acceleration as f32);
        }
        let (_, initial_length) = normal(input.body.velocity);
        return AlternateFlyResult::Steered {
            velocity: if current_speed == initial_length {
                input.body.velocity
            } else {
                scale(direction, current_speed)
            },
            pitch: None,
        };
    };
    let mut wanted_position = if input.state.fly_pinned {
        input.state.fly_ideal_position
    } else if input.state.pathing.is_some()
        || input.state.combat_point
        || input.state.sound_target.is_some()
        || input.state.lost_sight
    {
        target
    } else {
        add(
            add(target, scale(target_velocity, 0.25)),
            input.state.fly_ideal_position,
        )
    };
    let fit = services.trace(target, wanted_position, Some(FIT_BOUNDS), SOLID_MASK);
    if !fit.all_solid {
        wanted_position = fit.end;
    }
    let mut difference = sub(wanted_position, input.body.origin);
    if difference.z > input.body.bounds.min.z && difference.z < input.body.bounds.max.z {
        difference.z = 0.0;
    }
    let (mut wanted_direction, wanted_length) = normal(difference);
    if !input.state.manual_steering {
        input.state.ideal_yaw = f64::from(yaw(normal(sub(target, input.body.origin)).0));
    }
    let obstruction = services.trace(
        input.body.origin,
        add(
            input.body.origin,
            scale(wanted_direction, input.state.fly_acceleration as f32),
        ),
        Some(input.body.bounds),
        SOLID_MASK,
    );
    let angle = input.body.angles.y * ((PI * 2.0) / 360.0);
    let (sine, cosine) = angle.sin_cos();
    let forward = Vec3 {
        x: cosine,
        y: sine,
        z: -0.0,
    };
    let right = Vec3 {
        x: sine,
        y: -cosine,
        z: -0.0,
    };
    if obstruction.fraction < f64::from(0.25) {
        let visible_position = |height: f32, end_height: f32, services: &mut dyn AlternateFlyServices| {
            let start = add(
                input.body.origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: height,
                },
            );
            services.trace(start, wanted_position, None, SOLID_MASK).fraction == 1.0
                && services
                    .trace(
                        input.body.origin,
                        add(
                            input.body.origin,
                            Vec3 {
                                x: 0.0,
                                y: 0.0,
                                z: end_height,
                            },
                        ),
                        Some(input.body.bounds),
                        SOLID_MASK,
                    )
                    .fraction
                    == 1.0
        };
        let bottom_visible = visible_position(
            input.body.bounds.min.z,
            input.body.bounds.min.z - input.state.fly_acceleration as f32,
            services,
        );
        let top_visible = visible_position(
            input.body.bounds.max.z,
            input.body.bounds.max.z + input.state.fly_acceleration as f32,
            services,
        );
        if bottom_visible == top_visible {
            let front = add(input.body.origin, scaled(forward, input.body.bounds.max));
            let side = scaled(right, input.body.bounds.max);
            let left_visible = services
                .trace(sub(front, side), wanted_position, None, SOLID_MASK)
                .fraction
                == 1.0;
            let right_visible = services
                .trace(add(front, side), wanted_position, None, SOLID_MASK)
                .fraction
                == 1.0;
            wanted_direction = if left_visible != right_visible {
                if right_visible {
                    add(wanted_direction, right)
                } else {
                    sub(wanted_direction, right)
                }
            } else {
                obstruction
                    .q2()
                    .map(|fields| fields.source_plane.normal)
                    .unwrap_or(ZERO)
            };
        } else if top_visible {
            wanted_direction = add(wanted_direction, UP);
        } else {
            wanted_direction = sub(wanted_direction, UP);
        }
        wanted_direction = normal(wanted_direction).0;
    }
    let direct = input.state.fly_thrusters && !input.state.fly_pinned
        || input.state.pathing.is_some()
        || input.state.combat_point
        || input.state.lost_sight;
    let turn_factor = if direct && dot(direction, wanted_direction) > 0.0 {
        0.45
    } else {
        1.0f32.min(0.84 + 0.08 * (current_speed / input.state.fly_speed as f32))
    };
    let mut final_direction = if nonzero(direction) {
        direction
    } else {
        wanted_direction
    };
    if nan(final_direction) {
        return AlternateFlyResult::Fallback;
    }
    let swimming = input.flags & 2 != 0;
    let flying = input.flags & 1 != 0;
    let avoid_water = swimming || flying && input.state.water_level < 3;
    let water_ahead = avoid_water
        && services.point_contents(add(input.body.origin, scale(wanted_direction, current_speed))) & 32 != 0;
    let bad_direction = if swimming {
        !water_ahead
    } else {
        flying && input.state.water_level < 3 && water_ahead
    };
    if bad_direction && input.state.fly_recovery_time < now {
        input.state.fly_recovery_direction = normal(Vec3 {
            x: services.random().float_range(-1.0, 1.0),
            y: services.random().float_range(-1.0, 1.0),
            z: services.random().float_range(-1.0, 1.0),
        })
        .0;
        input.state.fly_recovery_time = ((now * 1000.0 + 0.5).floor() + 1000.0) / 1000.0;
    }
    if bad_direction {
        wanted_direction = input.state.fly_recovery_direction;
    }
    if nonzero(direction) && turn_factor > 0.0 {
        final_direction = normal(slerp(direction, wanted_direction, 1.0 - turn_factor)).0;
    }
    let mut speed_factor = if input.enemy.is_none() || direct {
        1.0
    } else if dot(forward, wanted_direction) < -0.25 && nonzero(direction) {
        0.0
    } else {
        1.0f32.min(wanted_length / input.state.fly_speed as f32)
    };
    if bad_direction {
        speed_factor = -speed_factor;
    }
    let mut acceleration = input.state.fly_acceleration as f32;
    if dot(final_direction, wanted_direction) < 0.25 {
        acceleration *= 2.0;
    }
    let wanted_speed = if input.state.manual_steering {
        0.0
    } else {
        input.state.fly_speed as f32 * speed_factor
    };
    if current_speed > wanted_speed {
        current_speed = wanted_speed.max(current_speed - acceleration);
    } else if current_speed < wanted_speed {
        current_speed = wanted_speed.min(current_speed + acceleration);
    }
    if nan(final_direction) || current_speed.is_nan() {
        return AlternateFlyResult::Fallback;
    }
    let mut new_pitch = 0.0f32;
    if input.enemy.is_some() && (input.state.fly_buzzard || input.state.medic) {
        let mut desired = -pitch(normal(sub(input.body.origin, target)).0);
        if desired - input.body.angles.x > 180.0 {
            desired -= 360.0;
        }
        if desired - input.body.angles.x < -180.0 {
            desired += 360.0;
        }
        new_pitch = input.body.angles.x + input.frame_seconds as f32 * 4.0 * (desired - input.body.angles.x);
    }
    AlternateFlyResult::Steered {
        velocity: scale(final_direction, current_speed),
        pitch: Some(new_pitch),
    }
}

/// Run one alternate fly step (`alternateFlyStep`).
pub fn alternate_fly_step(context: &mut MonsterContext) -> bool {
    let actor = context.actor().clone();
    let body = context.game.body_of(actor.clone());
    if context.game.host.rerelease_random().is_none() {
        panic!("Rerelease alternate flying requires the shared rerelease random stream");
    }
    let enemy_actor = context.entity().enemy.clone();
    let enemy = enemy_actor
        .as_ref()
        .and_then(|enemy| context.game.host.bodies().read(enemy));
    let goal = context
        .entity()
        .goal
        .clone()
        .and_then(|goal| context.game.host.bodies().read(&goal).map(|goal_body| goal_body.origin));
    let flags = context.entity().flags;
    let now = context.game.now();
    let frame_seconds = context.game.host.frame_seconds();
    let view_height = context.entity().view_height;
    // The steering input borrows the monster state while the services
    // borrow the game, so the state travels outside the arena for the
    // call. The services only touch the engine and the entity map, never
    // monster state, so the temporary removal is unobservable.
    let mut state = context.game.monsters.states.remove(&actor).unwrap_or_else(|| {
        panic!("Missing Q2 source monster context {}", actor.slot());
    });
    let mut input = AlternateFlyInput {
        state: &mut state,
        body,
        enemy,
        goal,
        flags,
        now,
        frame_seconds,
    };
    let mut services = ContextFlyServices {
        game: &mut *context.game,
        actor: actor.clone(),
        view_height,
        enemy_actor,
    };
    let result = steer_alternate_fly(&mut input, &mut services);
    let game = services.game;
    game.monsters.states.insert(actor.clone(), state);
    match result {
        AlternateFlyResult::Fallback => false,
        AlternateFlyResult::Steered { velocity, pitch } => {
            let mut body = game.body_of(actor.clone());
            body.velocity = velocity;
            if let Some(pitch) = pitch {
                body.angles.x = pitch;
            }
            game.write_body(actor, &body, false);
            true
        }
    }
}

/// Context-backed fly services.
struct ContextFlyServices<'a> {
    game: &'a mut Q2GameServices,
    actor: ActorId,
    view_height: i32,
    enemy_actor: Option<ActorId>,
}

impl AlternateFlyServices for ContextFlyServices<'_> {
    fn random(&mut self) -> &mut dyn Q2RereleaseRandomSource {
        self.game.host.rerelease_random().expect("rerelease random stream")
    }

    fn trace(&mut self, start: Vec3, end: Vec3, bounds: Option<Bounds>, mask: i32) -> TraceResult {
        self.game.host.trace(&Q2TraceRequest {
            start,
            end,
            bounds,
            mask,
            ignore: Some(self.actor.clone()),
            exclude: Vec::new(),
        })
    }

    fn point_contents(&mut self, point: Vec3) -> i32 {
        self.game.host.point_contents(point)
    }

    fn visible_enemy(&mut self) -> bool {
        let Some(enemy_actor) = self.enemy_actor.as_ref() else {
            return false;
        };
        let Some(enemy_body) = self.game.host.bodies().read(enemy_actor) else {
            return false;
        };
        let origin = self.game.body_of(self.actor.clone()).origin;
        let start = Vec3 {
            x: origin.x,
            y: origin.y,
            z: origin.z + self.view_height as f32,
        };
        let height = self
            .game
            .entity(enemy_actor)
            .map(|entity| entity.view_height)
            .unwrap_or(22);
        let end = Vec3 {
            x: enemy_body.origin.x,
            y: enemy_body.origin.y,
            z: enemy_body.origin.z + height as f32,
        };
        self.game
            .host
            .trace(&Q2TraceRequest {
                start,
                end,
                bounds: None,
                ignore: Some(self.actor.clone()),
                mask: 1 | 8 | 16,
                exclude: Vec::new(),
            })
            .fraction
            == 1.0
    }
}
