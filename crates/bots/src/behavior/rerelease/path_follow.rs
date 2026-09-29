//! Rerelease path following from `src/bots/behavior/rerelease/path-follow.ts`.
//!
//! Turns a point list into forward/side/jump for one frame in the
//! bot's own view frame, so a bot looking at an enemy strafes around
//! corners instead of turning away. Handles trains, lifts, ledges,
//! swimming, corner slowdown, lateral slide cancel, crouch/ladder
//! posture, and distance-over-time stuck detection.

use qa_core::math::Vec3;

use crate::behavior::rerelease::data::botdata::BotMovementSettings;
use crate::behavior::rerelease::math::{angle_vectors, bvec_distance, bvec_distance_2d, bvec_normalized, bvec_sub};
use crate::behavior::rerelease::nav::{
    nav_link_is_entity, nav_link_is_jump, steer_direction, BotTransportStep, NavGraphLinkT, NavLinkType, NavPathT,
};
use crate::behavior::rerelease::rng::{random_chance, BotRandomT};

/// Run speed units per second.
pub const BOT_RUN_SPEED: f32 = 320.0;
/// Walk speed units per second.
pub const BOT_WALK_SPEED: f32 = 160.0;
/// Steering point reach radius.
pub const BOT_POINT_REACHED: f32 = 32.0;
/// Traversal point reach radius.
pub const BOT_TRAVERSAL_REACHED: f32 = 20.0;
/// Corner slowdown turn threshold degrees.
pub const CORNER_TURN_DEGREES: f32 = 60.0;
/// Corner slowdown distance.
pub const CORNER_SLOW_DISTANCE: f32 = 96.0;
/// Lift board radius.
pub const LIFT_BOARD_RADIUS: f32 = 48.0;
/// Lift wait seconds.
pub const LIFT_WAIT_SECONDS: f32 = 4.0;
/// Ledge overshoot past a landing shadow.
pub const LEDGE_OVERSHOOT: f32 = 128.0;
/// Swim air reserve seconds.
pub const SWIM_AIR_RESERVE: f32 = 5.0;
/// Swim surface push share.
pub const SWIM_SURFACE_PUSH: f32 = 0.7;
/// Stuck progress distance.
pub const STUCK_PROGRESS_DISTANCE: f32 = 24.0;

/// Path follower state.
#[derive(Debug, Clone, PartialEq)]
pub struct BotPathStateT {
    /// Current path.
    pub path: Option<NavPathT>,
    /// Steering point index.
    pub index: usize,
    /// Stuck timer origin.
    pub stuck_origin: Vec3,
    /// Stuck timer start.
    pub stuck_since: f32,
    /// Consecutive stuck trips.
    pub stuck_count: i32,
    /// Lift wait start (-1 when idle).
    pub lift_wait_since: f32,
    /// Lift wait height.
    pub lift_wait_z: f32,
    /// Next jump allowed time.
    pub jump_ready_at: f32,
    /// Plan time.
    pub planned_at: f32,
}

/// New path state.
#[must_use]
pub fn new_path_state() -> BotPathStateT {
    BotPathStateT {
        path: None,
        index: 0,
        stuck_origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        stuck_since: 0.0,
        stuck_count: 0,
        jump_ready_at: 0.0,
        planned_at: 0.0,
        lift_wait_since: -1.0,
        lift_wait_z: 0.0,
    }
}

/// Set a path, skipping points already walked past.
pub fn set_path(state: &mut BotPathStateT, path: Option<NavPathT>, origin: Vec3, now: f32) {
    state.path = path;
    state.index = 0;
    if let Some(path) = state.path.as_ref() {
        let mut index = 0;
        while index + 1 < path.points.len() {
            let here = path.points[index];
            let next = path.points[index + 1];
            let back = (here.x - origin.x) * (next.x - origin.x) + (here.y - origin.y) * (next.y - origin.y);
            if back >= 0.0 {
                break;
            }
            if (here.z - origin.z).abs() > 64.0 {
                break;
            }
            let next_link = path.links.get(index + 1).and_then(|link| *link);
            if let Some(link) = next_link {
                if nav_link_is_entity(link.link_type) || nav_link_is_jump(link.link_type) {
                    break;
                }
            }
            index += 1;
        }
        state.index = index;
    }
    state.stuck_origin = origin;
    state.stuck_since = now;
    state.stuck_count = 0;
    state.planned_at = now;
}

/// Clear the path.
pub fn clear_path(state: &mut BotPathStateT) {
    state.path = None;
    state.index = 0;
    state.stuck_count = 0;
    state.lift_wait_since = -1.0;
}

/// Path status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotPathStatus;

impl BotPathStatus {
    /// No path.
    pub const NO_PATH: i32 = 0;
    /// Moving.
    pub const MOVING: i32 = 1;
    /// Arrived.
    pub const ARRIVED: i32 = 2;
    /// Stuck.
    pub const STUCK: i32 = 3;
}

/// Move output for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotMoveOutputT {
    /// Status.
    pub status: i32,
    /// Forward move.
    pub forwardmove: f32,
    /// Side move.
    pub sidemove: f32,
    /// Up move (swim/posture).
    pub upmove: f32,
    /// Jump.
    pub jump: bool,
    /// Riding a lift/train.
    pub riding: bool,
    /// Steering target.
    pub target: Option<Vec3>,
    /// Traversed link.
    pub link: Option<NavGraphLinkT>,
}

fn idle() -> BotMoveOutputT {
    BotMoveOutputT {
        status: BotPathStatus::NO_PATH,
        forwardmove: 0.0,
        sidemove: 0.0,
        upmove: 0.0,
        jump: false,
        riding: false,
        target: None,
        link: None,
    }
}

/// Follow input for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotFollowInputT {
    /// Origin.
    pub origin: Vec3,
    /// View yaw.
    pub yaw: f32,
    /// On ground.
    pub on_ground: bool,
    /// Water level.
    pub water_level: i32,
    /// Air seconds, when known.
    pub air_seconds: Option<f32>,
    /// Air above, when known.
    pub air_above: Option<bool>,
    /// Velocity, when known.
    pub velocity: Option<Vec3>,
    /// Now.
    pub now: f32,
    /// Stuck time seconds.
    pub stuck_time: f32,
    /// Run speed.
    pub run_speed: f32,
    /// Walk speed.
    pub walk_speed: f32,
}

/// One frame of path following.
pub fn follow_path(
    state: &mut BotPathStateT,
    input: &BotFollowInputT,
    movement: &BotMovementSettings,
    transport: &mut dyn FnMut(&NavGraphLinkT, Vec3) -> Option<BotTransportStep>,
) -> BotMoveOutputT {
    let idle = idle();
    let Some(path) = state.path.clone() else {
        return idle;
    };
    if path.points.is_empty() {
        return idle;
    }
    // Retire reached points.
    while state.index < path.points.len() {
        let point = path.points[state.index];
        let link = path.links.get(state.index).and_then(|link| *link);
        if state.index > 0 {
            if let Some(arriving) = path.links.get(state.index - 1).and_then(|link| *link) {
                if arriving.link_type == NavLinkType::Train {
                    match transport(&arriving, input.origin) {
                        Some(BotTransportStep::Move { approach, .. }) if !approach => {}
                        _ => break,
                    }
                }
            }
        }
        let tolerance = if link.is_some_and(|link| link.link_type != NavLinkType::Walk) {
            BOT_TRAVERSAL_REACHED
        } else {
            BOT_POINT_REACHED
        };
        if bvec_distance_2d(input.origin, point) <= tolerance && (input.origin.z - point.z).abs() <= 64.0 {
            state.index += 1;
            state.stuck_origin = input.origin;
            state.stuck_since = input.now;
            state.stuck_count = 0;
            continue;
        }
        break;
    }
    if state.index >= path.points.len() {
        return BotMoveOutputT {
            status: BotPathStatus::ARRIVED,
            target: None,
            link: None,
            ..idle
        };
    }
    let mut target = path.points[state.index];
    let link = path.links.get(state.index).and_then(|link| *link);
    let previous_transport = if state.index > 0 {
        path.links.get(state.index - 1).and_then(|link| *link)
    } else {
        None
    };
    let train = link
        .filter(|link| link.link_type == NavLinkType::Train)
        .or_else(|| previous_transport.filter(|link| link.link_type == NavLinkType::Train));
    if let Some(train) = train {
        match transport(&train, input.origin) {
            None | Some(BotTransportStep::Unavailable) => {
                return BotMoveOutputT {
                    status: BotPathStatus::STUCK,
                    target: Some(target),
                    link: Some(train),
                    ..idle
                };
            }
            Some(BotTransportStep::Wait) | Some(BotTransportStep::Ride) => {
                if state.lift_wait_since < 0.0 || bvec_distance(input.origin, state.stuck_origin) > 8.0 {
                    state.lift_wait_since = input.now;
                    state.stuck_origin = input.origin;
                }
                if input.now - state.lift_wait_since > LIFT_WAIT_SECONDS {
                    return BotMoveOutputT {
                        status: BotPathStatus::STUCK,
                        target: Some(target),
                        link: Some(train),
                        ..idle
                    };
                }
                state.stuck_since = input.now;
                let riding = matches!(transport(&train, input.origin), Some(BotTransportStep::Ride));
                return BotMoveOutputT {
                    status: BotPathStatus::MOVING,
                    riding,
                    target: Some(target),
                    link: Some(train),
                    ..idle
                };
            }
            Some(BotTransportStep::Move {
                target: step_target, ..
            }) => {
                target = step_target;
                state.lift_wait_since = -1.0;
            }
        }
    }
    // Walk-off-ledge overshoot while still above the landing.
    if link.is_some_and(|link| link.link_type == NavLinkType::WalkOffLedge)
        && input.on_ground
        && input.origin.z - target.z > 64.0
    {
        let prev = if state.index > 0 {
            path.points[state.index - 1]
        } else {
            input.origin
        };
        let mut ax = target.x - prev.x;
        let mut ay = target.y - prev.y;
        let mut length = (ax * ax + ay * ay).sqrt();
        if length < 1.0 {
            if let Some(velocity) = input.velocity {
                ax = velocity.x;
                ay = velocity.y;
                length = (ax * ax + ay * ay).sqrt();
            }
        }
        if length >= 1.0 {
            target = Vec3 {
                x: target.x + ax / length * LEDGE_OVERSHOOT,
                y: target.y + ay / length * LEDGE_OVERSHOOT,
                z: target.z,
            };
        }
    }
    // Lift riding: stand still aboard until near the end height.
    let prev_link = previous_transport;
    let lift = link
        .filter(|link| link.link_type == NavLinkType::Elevator)
        .or_else(|| prev_link.filter(|link| link.link_type == NavLinkType::Elevator));
    if let Some(lift) = lift {
        if let Some(ride) = lift.traversal.filter(|_| input.on_ground) {
            let below_end = ride.end.z - input.origin.z > 64.0;
            if below_end && bvec_distance_2d(input.origin, ride.start) <= LIFT_BOARD_RADIUS {
                if state.lift_wait_since < 0.0 || (input.origin.z - state.lift_wait_z).abs() > 8.0 {
                    state.lift_wait_since = input.now;
                    state.lift_wait_z = input.origin.z;
                }
                if input.now - state.lift_wait_since <= LIFT_WAIT_SECONDS {
                    state.stuck_origin = input.origin;
                    state.stuck_since = input.now;
                    return BotMoveOutputT {
                        status: BotPathStatus::MOVING,
                        riding: true,
                        target: Some(target),
                        link,
                        ..idle
                    };
                }
            } else {
                state.lift_wait_since = -1.0;
            }
        } else {
            state.lift_wait_since = -1.0;
        }
    } else {
        state.lift_wait_since = -1.0;
    }
    // Progress check.
    if bvec_distance(input.origin, state.stuck_origin) > STUCK_PROGRESS_DISTANCE {
        state.stuck_origin = input.origin;
        state.stuck_since = input.now;
    } else if input.now - state.stuck_since >= input.stuck_time {
        state.stuck_count += 1;
        state.stuck_origin = input.origin;
        state.stuck_since = input.now;
        return BotMoveOutputT {
            status: BotPathStatus::STUCK,
            target: Some(target),
            link,
            ..idle
        };
    }
    let speed = if movement.walk_only {
        input.walk_speed
    } else {
        input.run_speed
    };
    let (forward, right, _) = angle_vectors(0.0, input.yaw, 0.0);
    let swimming = input.water_level >= 2;
    let (dir, mut upmove) = if swimming {
        let dx = target.x - input.origin.x;
        let dy = target.y - input.origin.y;
        let dz = target.z - input.origin.z;
        let len = (dx * dx + dy * dy + dz * dz).sqrt();
        let dir = if len > 0.0 {
            Vec3 {
                x: dx / len,
                y: dy / len,
                z: dz / len,
            }
        } else {
            Vec3 { x: 0.0, y: 0.0, z: 0.0 }
        };
        let mut upmove = (dir.z * speed).clamp(-speed, speed);
        if input.air_seconds.is_some_and(|air| air < SWIM_AIR_RESERVE) && input.air_above == Some(true) && dir.z > -0.5
        {
            upmove = upmove.max(SWIM_SURFACE_PUSH * speed);
        }
        (
            Vec3 {
                x: dir.x,
                y: dir.y,
                z: 0.0,
            },
            upmove,
        )
    } else {
        (steer_direction(input.origin, target), 0.0)
    };
    // Corner slowdown.
    let mut wish_speed = speed;
    if let Some(next) = path.points.get(state.index + 1) {
        if bvec_distance_2d(input.origin, target) < CORNER_SLOW_DISTANCE {
            let (ax, ay) = (target.x - input.origin.x, target.y - input.origin.y);
            let (bx, by) = (next.x - target.x, next.y - target.y);
            let (la, lb) = ((ax * ax + ay * ay).sqrt(), (bx * bx + by * by).sqrt());
            if la > 1.0 && lb > 1.0 {
                let cos = (ax * bx + ay * by) / (la * lb);
                if cos < CORNER_TURN_DEGREES.to_radians().cos() {
                    wish_speed = wish_speed.min(input.walk_speed);
                }
            }
        }
    }
    // Lateral slide cancel.
    let mut wx = dir.x * wish_speed;
    let mut wy = dir.y * wish_speed;
    if let Some(velocity) = input.velocity {
        let along = velocity.x * dir.x + velocity.y * dir.y;
        wx -= velocity.x - along * dir.x;
        wy -= velocity.y - along * dir.y;
        let length = (wx * wx + wy * wy).sqrt();
        if length > speed {
            wx *= speed / length;
            wy *= speed / length;
        }
    }
    let forwardmove = (wx * forward.x + wy * forward.y).clamp(-speed, speed);
    let sidemove = (wx * right.x + wy * right.y).clamp(-speed, speed);
    let posture = link.or(prev_link);
    if posture.is_some_and(|posture| posture.link_type == NavLinkType::Crouch) {
        upmove = -speed;
    }
    if posture.is_some_and(|posture| posture.link_type == NavLinkType::Ladder) {
        upmove = ((target.z - input.origin.z) * 4.0).clamp(-speed, speed);
    }
    let mut jump = false;
    if input.on_ground {
        let needs_jump = link.is_some_and(|link| nav_link_is_jump(link.link_type));
        let step_up = target.z - input.origin.z > 24.0 && bvec_distance_2d(input.origin, target) < 96.0;
        if needs_jump || step_up && posture.is_none_or(|posture| posture.link_type != NavLinkType::Ladder) {
            jump = true;
        }
    }
    BotMoveOutputT {
        status: BotPathStatus::MOVING,
        forwardmove,
        sidemove,
        upmove,
        jump,
        riding: false,
        target: Some(target),
        link,
    }
}

/// Combat jump roll.
pub fn roll_combat_jump(
    state: &mut BotPathStateT,
    movement: &BotMovementSettings,
    rng: &mut dyn BotRandomT,
    now: f32,
    on_ground: bool,
) -> bool {
    if !movement.allow_jumping_in_combat || !on_ground || now < state.jump_ready_at {
        return false;
    }
    state.jump_ready_at = now + movement.jump_cooldown;
    random_chance(rng, movement.jump_chance)
}

/// Straight-line steering with no path.
#[must_use]
pub fn steer_direct(
    origin: Vec3,
    yaw: f32,
    target: Vec3,
    walk_only: bool,
    run_speed: f32,
    walk_speed: f32,
) -> (f32, f32) {
    let dir = steer_direction(origin, target);
    let speed = if walk_only { walk_speed } else { run_speed };
    let (forward, right, _) = angle_vectors(0.0, yaw, 0.0);
    (
        ((dir.x * forward.x + dir.y * forward.y) * speed).clamp(-speed, speed),
        ((dir.x * right.x + dir.y * right.y) * speed).clamp(-speed, speed),
    )
}

/// Normalized flat direction helper (tests).
#[must_use]
pub fn flat_direction(delta: Vec3) -> Vec3 {
    bvec_normalized(Vec3 {
        x: delta.x,
        y: delta.y,
        z: 0.0,
    })
}

/// 2D subtraction helper (tests).
#[must_use]
pub fn delta_2d(from: Vec3, to: Vec3) -> Vec3 {
    let delta = bvec_sub(to, from);
    Vec3 {
        x: delta.x,
        y: delta.y,
        z: 0.0,
    }
}
