//! Deterministic movement kernels ported from `src/movement`: Q1 clip and
//! friction/accelerate helpers (`q1/common.ts`, `q1/netquake.ts`), Q2 slide
//! clip (`q2/math.ts`), Q3 clip velocity and step/friction rules
//! (`q3/slide-move.ts`, `q3/move.ts`), and the shared body-shape gate
//! (`body-shape.ts`). Values come from the TS donor; game providers own
//! the remaining Pmove call sites.

use qa_core::math::{dot3, vec3, Bounds, Vec3};
use qa_core::numeric::NumericOps;

/// Quake I movement types.
pub mod q1_move {
    /// No movement.
    pub const NONE: i32 = 0;
    /// Player walk.
    pub const WALK: i32 = 3;
    /// Step movement.
    pub const STEP: i32 = 4;
    /// Fly movement.
    pub const FLY: i32 = 5;
    /// Toss movement.
    pub const TOSS: i32 = 6;
    /// Pusher movement.
    pub const PUSH: i32 = 7;
    /// Noclip movement.
    pub const NOCLIP: i32 = 8;
    /// Fly missile.
    pub const FLYMISSILE: i32 = 9;
    /// Bounce movement.
    pub const BOUNCE: i32 = 10;
    /// Gib movement.
    pub const GIB: i32 = 11;
}

/// Quake I entity flags.
pub mod q1_flag {
    /// Flying.
    pub const FLY: i32 = 1;
    /// Swimming.
    pub const SWIM: i32 = 2;
    /// On ground.
    pub const ONGROUND: i32 = 512;
    /// Water jump in progress.
    pub const WATERJUMP: i32 = 2048;
    /// Jump button released.
    pub const JUMPRELEASED: i32 = 4096;
}

/// Quake I movement tuning. Values come from the game provider or its
/// cvars; this crate defines no defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1MovementParameters {
    /// World gravity.
    pub gravity: f64,
    /// Stop speed for friction control.
    pub stop_speed: f64,
    /// Maximum ground speed.
    pub max_speed: f64,
    /// Maximum spectator speed.
    pub spectator_max_speed: f64,
    /// Ground acceleration.
    pub accelerate: f64,
    /// Air acceleration.
    pub air_accelerate: f64,
    /// Water acceleration.
    pub water_accelerate: f64,
    /// Ground friction.
    pub friction: f64,
    /// Water friction.
    pub water_friction: f64,
    /// Entity gravity scale.
    pub entity_gravity: f64,
}

/// Quake III movement types (`bg_public.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Q3MoveType {
    /// Normal movement.
    Normal = 0,
    /// Noclip.
    Noclip = 1,
    /// Spectator.
    Spectator = 2,
    /// Dead.
    Dead = 3,
    /// Frozen.
    Freeze = 4,
    /// Intermission.
    Intermission = 5,
    /// Single-player intermission.
    SpIntermission = 6,
}

/// Quake III movement flags.
pub mod q3_flag {
    /// Ducked.
    pub const DUCKED: u32 = 1;
    /// Jump held.
    pub const JUMP_HELD: u32 = 2;
    /// Backwards jump.
    pub const BACKWARDS_JUMP: u32 = 8;
    /// Backwards run.
    pub const BACKWARDS_RUN: u32 = 16;
    /// Landing timer.
    pub const TIME_LAND: u32 = 32;
    /// Knockback timer.
    pub const TIME_KNOCKBACK: u32 = 64;
    /// Water-jump timer.
    pub const TIME_WATERJUMP: u32 = 256;
    /// Respawned this frame.
    pub const RESPAWNED: u32 = 512;
    /// All movement timers.
    pub const ALL_TIMES: u32 = TIME_WATERJUMP | TIME_LAND | TIME_KNOCKBACK;
}

/// Quake III step and fall events used by movement.
pub mod q3_event {
    /// Four-unit step.
    pub const STEP_4: u32 = 6;
    /// Eight-unit step.
    pub const STEP_8: u32 = 7;
    /// Twelve-unit step.
    pub const STEP_12: u32 = 8;
    /// Sixteen-unit step.
    pub const STEP_16: u32 = 9;
    /// Short fall.
    pub const FALL_SHORT: u32 = 10;
    /// Medium fall.
    pub const FALL_MEDIUM: u32 = 11;
    /// Far fall.
    pub const FALL_FAR: u32 = 12;
    /// Jump.
    pub const JUMP: u32 = 14;
}

/// Maximum slide-move bumps.
pub const MAX_BUMPS: usize = 4;
/// Maximum slide-move planes before stopping.
pub const MAX_PLANES: usize = 5;
/// Q3 step height.
pub const Q3_STEP_HEIGHT: f32 = 18.0;
/// Q3 walkable ground normal.
pub const Q3_MIN_GROUND_NORMAL: f32 = 0.7;
/// Q3 duplicate-plane dot threshold.
pub const Q3_DUPLICATE_PLANE_DOT: f32 = 0.99;
/// Q3 plane-enter threshold.
pub const Q3_ENTER_THRESHOLD: f32 = 0.1;

/// Q3 `PM_ClipVelocity`: binary32 each-op with the `1.001` overbounce.
#[must_use]
pub fn clip_velocity_q3(velocity: Vec3, normal: Vec3, overbounce: f32) -> Vec3 {
    let dot = dot3(velocity, normal);
    let backoff = if dot < 0.0 { dot * overbounce } else { dot / overbounce };
    vec3(
        velocity.x - normal.x * backoff,
        velocity.y - normal.y * backoff,
        velocity.z - normal.z * backoff,
    )
}

/// Q3 clip with the source default overbounce.
#[must_use]
pub fn clip_velocity_q3_default(velocity: Vec3, normal: Vec3) -> Vec3 {
    clip_velocity_q3(velocity, normal, 1.001)
}

/// Q1 `ClipVelocity`: profile arithmetic with the `0.1` snap-to-zero.
#[must_use]
pub fn clip_velocity_q1(velocity: Vec3, normal: Vec3, overbounce: f64, ops: &NumericOps) -> Vec3 {
    let dot = ops.add(
        ops.add(
            ops.mul(f64::from(velocity.x), f64::from(normal.x)),
            ops.mul(f64::from(velocity.y), f64::from(normal.y)),
        ),
        ops.mul(f64::from(velocity.z), f64::from(normal.z)),
    );
    let backoff = ops.mul(dot, overbounce);
    let snap = |value: f32| {
        if value.abs() < 0.1 {
            0.0
        } else {
            value
        }
    };
    vec3(
        snap(ops.store(ops.sub(f64::from(velocity.x), ops.mul(f64::from(normal.x), backoff)))),
        snap(ops.store(ops.sub(f64::from(velocity.y), ops.mul(f64::from(normal.y), backoff)))),
        snap(ops.store(ops.sub(f64::from(velocity.z), ops.mul(f64::from(normal.z), backoff)))),
    )
}

/// Q2 `SlideClipVelocity`: profile arithmetic with the `0.1` dead zone.
#[must_use]
pub fn slide_clip_q2(velocity: Vec3, normal: Vec3, overbounce: f64, ops: &NumericOps) -> Vec3 {
    let dot = ops.add(
        ops.add(
            ops.mul(f64::from(velocity.x), f64::from(normal.x)),
            ops.mul(f64::from(velocity.y), f64::from(normal.y)),
        ),
        ops.mul(f64::from(velocity.z), f64::from(normal.z)),
    );
    let backoff = ops.mul(dot, overbounce);
    let clean = |value: f32| {
        if value > -0.1 && value < 0.1 {
            0.0
        } else {
            value
        }
    };
    vec3(
        clean(ops.store(ops.sub(f64::from(velocity.x), ops.mul(f64::from(normal.x), backoff)))),
        clean(ops.store(ops.sub(f64::from(velocity.y), ops.mul(f64::from(normal.y), backoff)))),
        clean(ops.store(ops.sub(f64::from(velocity.z), ops.mul(f64::from(normal.z), backoff)))),
    )
}

/// A requested local hull expands only after the selected source collision
/// query accepts it.
#[must_use]
pub fn movement_bounds(previous: &Bounds, requested: &Bounds, clear: &dyn Fn(&Bounds) -> bool) -> Bounds {
    let expands = requested.min.x < previous.min.x
        || requested.min.y < previous.min.y
        || requested.min.z < previous.min.z
        || requested.max.x > previous.max.x
        || requested.max.y > previous.max.y
        || requested.max.z > previous.max.z;
    if expands && !clear(requested) {
        *previous
    } else {
        *requested
    }
}

/// Q3 movement-timer countdown. Clears every timer when the step covers the
/// remaining time.
pub fn drop_q3_movement_timers(pm_time: &mut i32, pm_flags: &mut u32, milliseconds: i32) {
    if *pm_time != 0 {
        if milliseconds >= *pm_time {
            *pm_flags &= !q3_flag::ALL_TIMES;
            *pm_time = 0;
        } else {
            *pm_time -= milliseconds;
        }
    }
}

/// Q3 step event for a vertical step delta. Steps of two units or less are
/// silent.
#[must_use]
pub fn q3_step_event(delta_z: f32) -> Option<u32> {
    if delta_z <= 2.0 {
        return None;
    }
    Some(if delta_z < 7.0 {
        q3_event::STEP_4
    } else if delta_z < 11.0 {
        q3_event::STEP_8
    } else if delta_z < 15.0 {
        q3_event::STEP_12
    } else {
        q3_event::STEP_16
    })
}

/// Q1 ground friction scale: `new_speed / speed` with the donor's control
/// speed and operation order.
#[must_use]
pub fn q1_friction_scale(speed: f64, stop_speed: f64, friction: f64, frame_seconds: f64, ops: &NumericOps) -> f64 {
    if speed <= 0.0 {
        return 0.0;
    }
    let control = speed.max(stop_speed);
    let drop = ops.mul(ops.mul(frame_seconds, control), friction);
    (speed - drop).max(0.0) / speed
}

/// Q1 accelerate speed gain. Ground and air differ only in multiply order,
/// which the profile preserves.
#[must_use]
pub fn q1_accelerate_gain(
    current_speed: f64,
    wish_speed: f64,
    accelerate: f64,
    frame_seconds: f64,
    air: bool,
    ops: &NumericOps,
) -> f64 {
    let add = wish_speed - current_speed;
    if add <= 0.0 {
        return 0.0;
    }
    let gain = if air {
        ops.mul(ops.mul(accelerate, wish_speed), frame_seconds)
    } else {
        ops.mul(ops.mul(accelerate, frame_seconds), wish_speed)
    };
    add.min(gain)
}

/// Q1 gravity drop for one frame: `entity * multiplier * gravity * dt`.
#[must_use]
pub fn q1_gravity_drop(
    parameters: &Q1MovementParameters,
    gravity_multiplier: f64,
    frame_seconds: f64,
    ops: &NumericOps,
) -> f64 {
    ops.mul(
        ops.mul(
            ops.mul(parameters.entity_gravity, gravity_multiplier),
            parameters.gravity,
        ),
        frame_seconds,
    )
}

/// Q3 friction over one frame, with the donor's surface and state gates.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn q3_friction(
    velocity: Vec3,
    walking: bool,
    water_level: i32,
    slick: bool,
    knockback_time: bool,
    flight: bool,
    spectator: bool,
    frame_time: f32,
) -> Vec3 {
    let planar = walking;
    let speed = if planar {
        (f64::from(velocity.x * velocity.x + velocity.y * velocity.y).sqrt()) as f32
    } else {
        (f64::from(dot3(velocity, velocity)).sqrt()) as f32
    };
    if speed < 1.0 {
        return vec3(0.0, 0.0, velocity.z);
    }
    let mut drop = 0.0_f32;
    if water_level <= 1 && walking && !slick && !knockback_time {
        drop += speed.max(100.0) * 6.0 * frame_time;
    }
    if water_level > 0 {
        drop += speed * water_level as f32 * frame_time;
    }
    if flight {
        drop += speed * 3.0 * frame_time;
    }
    if spectator {
        drop += speed * 5.0 * frame_time;
    }
    let scale = (speed - drop).max(0.0) / speed;
    vec3(velocity.x * scale, velocity.y * scale, velocity.z * scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::numeric::{Q1_DONOR_PROFILE, Q3_BINARY32_PROFILE};

    fn q1_ops() -> NumericOps {
        NumericOps::select(Q1_DONOR_PROFILE).unwrap()
    }

    #[test]
    fn q3_clip_removes_into_plane_motion() {
        let clipped = clip_velocity_q3_default(vec3(3.0, 4.0, -5.0), vec3(0.0, 0.0, 1.0));
        assert_eq!(clipped.x, 3.0);
        assert_eq!(clipped.y, 4.0);
        assert!(clipped.z.abs() < 0.01);
        let parallel = clip_velocity_q3_default(vec3(3.0, 4.0, 0.0), vec3(0.0, 0.0, 1.0));
        assert_eq!(parallel, vec3(3.0, 4.0, 0.0));
    }

    #[test]
    fn q1_clip_snaps_near_zero_components() {
        let clipped = clip_velocity_q1(vec3(1.0, 0.05, -1.0), vec3(0.0, 0.0, 1.0), 1.0, &q1_ops());
        assert_eq!(clipped, vec3(1.0, 0.0, 0.0));
    }

    #[test]
    fn q2_slide_clip_applies_the_dead_zone() {
        let clipped = slide_clip_q2(vec3(0.05, 0.2, 0.0), vec3(0.0, 0.0, 1.0), 1.0, &q1_ops());
        assert_eq!(clipped, vec3(0.0, 0.2, 0.0));
    }

    #[test]
    fn body_shape_expansion_requires_clearance() {
        let previous = Bounds {
            min: vec3(-16.0, -16.0, -24.0),
            max: vec3(16.0, 16.0, 32.0),
        };
        let requested = Bounds {
            min: vec3(-16.0, -16.0, -24.0),
            max: vec3(16.0, 16.0, 48.0),
        };
        assert_eq!(movement_bounds(&previous, &requested, &|_| false), previous);
        assert_eq!(movement_bounds(&previous, &requested, &|_| true), requested);
        let shrink = Bounds {
            min: vec3(-16.0, -16.0, -24.0),
            max: vec3(16.0, 16.0, 16.0),
        };
        assert_eq!(movement_bounds(&previous, &shrink, &|_| false), shrink);
    }

    #[test]
    fn q3_timers_and_steps_follow_source_thresholds() {
        let mut time = 100;
        let mut flags = q3_flag::TIME_LAND | q3_flag::DUCKED;
        drop_q3_movement_timers(&mut time, &mut flags, 30);
        assert_eq!((time, flags), (70, q3_flag::TIME_LAND | q3_flag::DUCKED));
        drop_q3_movement_timers(&mut time, &mut flags, 70);
        assert_eq!((time, flags), (0, q3_flag::DUCKED));
        assert_eq!(q3_step_event(1.5), None);
        assert_eq!(q3_step_event(5.0), Some(q3_event::STEP_4));
        assert_eq!(q3_step_event(9.0), Some(q3_event::STEP_8));
        assert_eq!(q3_step_event(13.0), Some(q3_event::STEP_12));
        assert_eq!(q3_step_event(16.0), Some(q3_event::STEP_16));
    }

    #[test]
    fn q1_friction_accelerate_and_gravity_are_deterministic() {
        let ops = q1_ops();
        let scale = q1_friction_scale(300.0, 100.0, 4.0, 0.05, &ops);
        assert!((scale - 0.8).abs() < 1e-9);
        let gain = q1_accelerate_gain(200.0, 320.0, 10.0, 0.05, false, &ops);
        assert!((gain - 120.0).abs() < 1e-9);
        assert_eq!(q1_accelerate_gain(400.0, 320.0, 10.0, 0.05, false, &ops), 0.0);
        let parameters = Q1MovementParameters {
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
        };
        assert!((q1_gravity_drop(&parameters, 1.0, 0.05, &ops) - 40.0).abs() < 1e-9);
    }

    #[test]
    fn q3_friction_stops_slow_bodies_and_scales_fast_ones() {
        let slow = q3_friction(vec3(0.5, 0.0, -10.0), true, 0, false, false, false, false, 0.008);
        assert_eq!(slow, vec3(0.0, 0.0, -10.0));
        let fast = q3_friction(vec3(300.0, 0.0, 0.0), true, 0, false, false, false, false, 0.008);
        assert!(fast.x < 300.0 && fast.x > 280.0);
        let slick = q3_friction(vec3(300.0, 0.0, 0.0), true, 0, true, false, false, false, 0.008);
        assert_eq!(slick.x, 300.0);
        let _ = Q3_BINARY32_PROFILE;
    }
}
