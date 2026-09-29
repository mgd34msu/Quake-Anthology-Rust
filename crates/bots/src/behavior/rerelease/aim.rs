//! Rerelease aim tracker from `src/bots/behavior/rerelease/aim.ts`.
//!
//! The `aiming.*` block run as a spring-damper on pitch and yaw with
//! substepped semi-implicit Euler. `max_acceleration` clamps angular
//! speed; `velocity_offset` leads or lags the target; the modifier
//! window multiplies the spring past `modifier.max_angle`.

use qa_core::math::Vec3;

use crate::behavior::rerelease::data::botdata::BotAimingSettings;
use crate::behavior::rerelease::math::{angle_delta, angle_mod, bvec_ma, vector_to_angles};

/// Aim tracker state.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BotAimStateT {
    /// Pitch.
    pub pitch: f32,
    /// Yaw.
    pub yaw: f32,
    /// Pitch velocity.
    pub pitch_velocity: f32,
    /// Yaw velocity.
    pub yaw_velocity: f32,
    /// Modifier window expiry; `<= 0` when idle.
    pub modifier_until: f32,
}

/// New aim state.
#[must_use]
pub fn new_aim_state(pitch: f32, yaw: f32) -> BotAimStateT {
    BotAimStateT {
        pitch,
        yaw,
        pitch_velocity: 0.0,
        yaw_velocity: 0.0,
        modifier_until: 0.0,
    }
}

/// Lead point with `velocity_offset` applied.
#[must_use]
pub fn aim_lead_point(target: Vec3, target_velocity: Vec3, settings: &BotAimingSettings) -> Vec3 {
    if settings.velocity_offset == 0.0 {
        target
    } else {
        bvec_ma(target, settings.velocity_offset, target_velocity)
    }
}

/// Advance the tracker toward `ideal_dir`; returns pitch and yaw.
pub fn aim_step(
    state: &mut BotAimStateT,
    ideal_dir: Vec3,
    settings: &BotAimingSettings,
    dt: f32,
    now: f32,
) -> (f32, f32) {
    if dt <= 0.0 {
        return (state.pitch, state.yaw);
    }
    let (ideal_pitch, ideal_yaw) = vector_to_angles(ideal_dir);
    let pitch_error = angle_delta(state.pitch, ideal_pitch);
    let yaw_error = angle_delta(state.yaw, ideal_yaw);
    let worst = pitch_error.abs().max(yaw_error.abs());
    if settings.modifier_apply_time > 0.0 && settings.modifier_max_angle > 0.0 && worst > settings.modifier_max_angle {
        state.modifier_until = now + settings.modifier_apply_time;
    }
    let modified = state.modifier_until > now;
    let max_speed = settings.max_acceleration * if modified { settings.modifier_accel_scalar } else { 1.0 };
    let stiffness = settings.spring_stiffness * if modified { settings.modifier_spring_scalar } else { 1.0 };
    let damping = settings.damping
        * if modified {
            settings.modifier_damping_scalar
        } else {
            1.0
        };
    let omega = stiffness.max(1.0).sqrt();
    let substeps = ((dt * omega / 0.25).ceil() as usize).clamp(1, 64);
    let h = dt / substeps as f32;
    let tuning = AxisTuning {
        stiffness,
        damping,
        max_speed,
        h,
        substeps,
    };
    let (pitch, pitch_velocity) = advance_axis(state.pitch, state.pitch_velocity, pitch_error, tuning);
    let (yaw, yaw_velocity) = advance_axis(state.yaw, state.yaw_velocity, yaw_error, tuning);
    state.pitch = pitch.clamp(-80.0, 80.0);
    state.pitch_velocity = if state.pitch == pitch { pitch_velocity } else { 0.0 };
    state.yaw = angle_mod(yaw);
    state.yaw_velocity = yaw_velocity;
    (state.pitch, state.yaw)
}

/// Spring-damper tuning shared by both aim axes.
#[derive(Debug, Clone, Copy)]
struct AxisTuning {
    stiffness: f32,
    damping: f32,
    max_speed: f32,
    h: f32,
    substeps: usize,
}

fn advance_axis(angle: f32, velocity: f32, error_to: f32, tuning: AxisTuning) -> (f32, f32) {
    let AxisTuning {
        stiffness,
        damping,
        max_speed,
        h,
        substeps,
    } = tuning;
    let mut a = angle;
    let mut v = velocity;
    for _ in 0..substeps {
        let error = error_to - (a - angle);
        let accel = stiffness * error - damping * v;
        v += accel * h;
        if max_speed > 0.0 {
            v = v.clamp(-max_speed, max_speed);
        }
        a += v * h;
    }
    (a, v)
}

/// Remaining angular error from `ideal_dir` in degrees.
#[must_use]
pub fn aim_error(state: &BotAimStateT, ideal_dir: Vec3) -> f32 {
    let (ideal_pitch, ideal_yaw) = vector_to_angles(ideal_dir);
    angle_delta(state.pitch, ideal_pitch)
        .abs()
        .max(angle_delta(state.yaw, ideal_yaw).abs())
}
