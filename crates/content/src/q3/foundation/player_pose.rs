//! Quake III foundation: player pose.
//!
//! Donor provenance: `src/content/q3/foundation/player-pose.ts`.

use qa_core::math::{dot3, length3, normalize3, sub3, vec3, Axis, Vec3};
use qa_core::numeric::qvm_float_to_int;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::animation::*;
use qa_world::movement::q3::constants::player_animation;
use thiserror::Error;

// ---------------------------------------------------------------------------
// player-pose.ts: CG_SwingAngles, CG_PlayerAngles, CG_AddPainTwitch.
// ---------------------------------------------------------------------------

/// Player pose failure (donor `RangeError` and `CommonError("drop")`
/// throws).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlayerPoseError {
    /// Out-of-range value (donor `RangeError`).
    #[error("{0}")]
    Range(String),
    /// Dropped movement angle (donor `CommonError` with `"drop"`).
    #[error("drop: {0}")]
    Drop(String),
}

fn range(message: impl Into<String>) -> PlayerPoseError {
    PlayerPoseError::Range(message.into())
}

pub(crate) const PAIN_TWITCH_TIME: i32 = 200;

pub(crate) const DEAD_ENTITY_FLAG: i32 = 1;

pub(crate) const MOVEMENT_OFFSETS: [f32; 8] = [0.0, 22.0, 45.0, -22.0, 0.0, 22.0, -45.0, -22.0];

/// Lerp frame with swing state (`PoseLerpFrame`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoseLerpFrame {
    /// Frame interpolation.
    pub lerp: LerpFrame,
    /// Current yaw.
    pub yaw_angle: f32,
    /// Yaw in motion.
    pub yawing: bool,
    /// Current pitch.
    pub pitch_angle: f32,
    /// Pitch in motion.
    pub pitching: bool,
}

/// Player pose state (`PlayerPoseState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerPoseState {
    /// Legs frame.
    pub legs: PoseLerpFrame,
    /// Torso frame.
    pub torso: PoseLerpFrame,
    /// Last pain time.
    pub pain_time: i32,
    /// Pain direction toggle.
    pub pain_direction: bool,
}

pub(crate) fn create_pose_lerp_frame() -> PoseLerpFrame {
    PoseLerpFrame {
        lerp: create_lerp_frame(),
        yaw_angle: 0.0,
        yawing: false,
        pitch_angle: 0.0,
        pitching: false,
    }
}

/// Fresh pose state (`createPlayerPoseState`).
#[must_use]
pub fn create_player_pose_state() -> PlayerPoseState {
    PlayerPoseState {
        legs: create_pose_lerp_frame(),
        torso: create_pose_lerp_frame(),
        pain_time: 0,
        pain_direction: false,
    }
}

/// Swing input (`SwingAnglesInput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwingAnglesInput {
    /// Destination angle.
    pub destination: f32,
    /// Tolerance that starts swinging.
    pub swing_tolerance: f32,
    /// Tolerance that clamps.
    pub clamp_tolerance: f32,
    /// Degrees per millisecond factor.
    pub speed: f32,
    /// Frame time in milliseconds.
    pub frame_time_ms: i32,
    /// Current angle.
    pub angle: f32,
    /// Already swinging.
    pub swinging: bool,
}

/// Swing result (`SwingAnglesResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwingAnglesResult {
    /// New angle.
    pub angle: f32,
    /// Still swinging.
    pub swinging: bool,
}

/// Pain twitch input (`PainTwitchInput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PainTwitchInput {
    /// Clock in milliseconds.
    pub time_ms: i32,
    /// Pain time.
    pub pain_time: i32,
    /// Pain direction.
    pub pain_direction: bool,
}

/// Pose entity snapshot (`PoseEntityState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoseEntityState {
    /// Entity flags.
    pub e_flags: i32,
    /// Velocity.
    pub velocity: Vec3,
    /// Movement direction.
    pub movement_direction: f32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
}

/// Pose calculation input (`CalculatePlayerPoseInput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalculatePlayerPoseInput {
    /// Entity snapshot.
    pub entity: PoseEntityState,
    /// Fixed legs.
    pub fixed_legs: bool,
    /// Fixed torso.
    pub fixed_torso: bool,
    /// Interpolated angles.
    pub lerp_angles: Vec3,
    /// Clock in milliseconds.
    pub time_ms: i32,
    /// Frame time in milliseconds.
    pub frame_time_ms: i32,
    /// Swing speed.
    pub swing_speed: f32,
}

/// Computed hierarchical axes (`PlayerPose`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerPose {
    /// Legs axis.
    pub legs: Axis,
    /// Torso axis.
    pub torso: Axis,
    /// Head axis.
    pub head: Axis,
}

pub(crate) fn angle_subtract(first: f32, second: f32) -> f32 {
    let mut angle = first - second;
    while angle > 180.0 {
        angle -= 360.0;
    }
    while angle < -180.0 {
        angle += 360.0;
    }
    angle
}

/// Swing one angle toward its destination (`swingAngles`).
pub fn swing_angles(input: &SwingAnglesInput) -> Result<SwingAnglesResult, PlayerPoseError> {
    if !input.destination.is_finite() {
        return Err(range("swing destination must be a finite float32 value"));
    }
    if !input.swing_tolerance.is_finite() {
        return Err(range("swing tolerance must be a finite float32 value"));
    }
    if !input.clamp_tolerance.is_finite() {
        return Err(range("clamp tolerance must be a finite float32 value"));
    }
    if !input.speed.is_finite() {
        return Err(range("swing speed must be a finite float32 value"));
    }
    if !input.angle.is_finite() {
        return Err(range("swing angle must be a finite float32 value"));
    }
    if input.frame_time_ms < 0 {
        return Err(range("frame time must be a non-negative int32 millisecond value"));
    }
    let mut angle = input.angle;
    let mut swinging = input.swinging;
    if input.swing_tolerance < 0.0 || input.clamp_tolerance < 1.0 || input.speed < 0.0 {
        return Err(range("swing tolerances and speed are outside source ranges"));
    }
    if !swinging {
        let swing = angle_subtract(angle, input.destination);
        if swing > input.swing_tolerance || swing < -input.swing_tolerance {
            swinging = true;
        }
    }
    if !swinging {
        return Ok(SwingAnglesResult { angle, swinging });
    }
    let swing = angle_subtract(input.destination, angle);
    let distance = swing.abs();
    let scale = if distance < input.swing_tolerance * 0.5 {
        0.5
    } else if distance < input.swing_tolerance {
        1.0
    } else {
        2.0
    };
    if swing >= 0.0 {
        let mut step = input.frame_time_ms as f32 * scale * input.speed;
        if step >= swing {
            step = swing;
            swinging = false;
        }
        angle = qvm_angle_mod(angle + step);
    } else {
        let mut step = input.frame_time_ms as f32 * scale * -input.speed;
        if step <= swing {
            step = swing;
            swinging = false;
        }
        angle = qvm_angle_mod(angle + step);
    }
    let swing = angle_subtract(input.destination, angle);
    if swing > input.clamp_tolerance {
        angle = qvm_angle_mod(input.destination - (input.clamp_tolerance - 1.0));
    } else if swing < -input.clamp_tolerance {
        angle = qvm_angle_mod(input.destination + (input.clamp_tolerance - 1.0));
    }
    Ok(SwingAnglesResult { angle, swinging })
}

/// Add the decaying pain roll (`addPainTwitch`).
#[must_use]
pub fn add_pain_twitch(torso_angles: Vec3, input: &PainTwitchInput) -> Vec3 {
    let elapsed = input.time_ms.wrapping_sub(input.pain_time);
    if elapsed >= PAIN_TWITCH_TIME {
        return vec3(torso_angles.x, torso_angles.y, torso_angles.z);
    }
    let fraction = 1.0 - elapsed as f32 / PAIN_TWITCH_TIME as f32;
    let roll = 20.0 * fraction;
    vec3(
        torso_angles.x,
        torso_angles.y,
        if input.pain_direction {
            torso_angles.z + roll
        } else {
            torso_angles.z - roll
        },
    )
}

pub(crate) fn subtract_angles(first: Vec3, second: Vec3) -> Vec3 {
    vec3(
        angle_subtract(first.x, second.x),
        angle_subtract(first.y, second.y),
        angle_subtract(first.z, second.z),
    )
}

pub(crate) fn movement_offset(entity: &PoseEntityState) -> Result<f32, PlayerPoseError> {
    if entity.e_flags & DEAD_ENTITY_FLAG != 0 {
        return Ok(0.0);
    }
    let direction = qvm_float_to_int(entity.movement_direction);
    if direction < 0 || direction as usize >= MOVEMENT_OFFSETS.len() {
        return Err(PlayerPoseError::Drop("Bad player movement angle".to_string()));
    }
    MOVEMENT_OFFSETS
        .get(direction as usize)
        .copied()
        .ok_or_else(|| range(format!("missing player movement offset {direction}")))
}

pub(crate) fn update_yaw(
    state: &mut PoseLerpFrame,
    destination: f32,
    tolerance: f32,
    input: &CalculatePlayerPoseInput,
) -> Result<f32, PlayerPoseError> {
    let result = swing_angles(&SwingAnglesInput {
        destination,
        swing_tolerance: tolerance,
        clamp_tolerance: 90.0,
        speed: input.swing_speed,
        frame_time_ms: input.frame_time_ms,
        angle: state.yaw_angle,
        swinging: state.yawing,
    })?;
    state.yaw_angle = result.angle;
    state.yawing = result.swinging;
    Ok(result.angle)
}

/// Compute hierarchical axes while updating swing state (`calculatePlayerPose`).
pub fn calculate_player_pose(
    state: &mut PlayerPoseState,
    input: &CalculatePlayerPoseInput,
) -> Result<PlayerPose, PlayerPoseError> {
    if input.frame_time_ms < 0 {
        return Err(range("frame time must be a non-negative int32 millisecond value"));
    }
    if !input.swing_speed.is_finite() {
        return Err(range("swing speed must be a finite float32 value"));
    }
    if input.swing_speed < 0.0 {
        return Err(range("swing speed must be non-negative"));
    }
    let head_angles = vec3(
        input.lerp_angles.x,
        qvm_angle_mod(input.lerp_angles.y),
        input.lerp_angles.z,
    );
    if input.entity.legs_anim & !ANIMATION_TOGGLE_BIT != player_animation::LEGS_IDLE
        || input.entity.torso_anim & !ANIMATION_TOGGLE_BIT != player_animation::TORSO_STAND
    {
        state.torso.yawing = true;
        state.torso.pitching = true;
        state.legs.yawing = true;
    }

    let offset = movement_offset(&input.entity)?;
    let legs_destination = head_angles.y + offset;
    let torso_destination = head_angles.y + 0.25 * offset;
    let torso_yaw = update_yaw(&mut state.torso, torso_destination, 25.0, input)?;
    let legs_yaw = update_yaw(&mut state.legs, legs_destination, 40.0, input)?;
    let mut torso_angles = vec3(0.0, torso_yaw, 0.0);
    let mut legs_angles = vec3(0.0, legs_yaw, 0.0);

    let pitch_destination = if head_angles.x > 180.0 {
        (-360.0 + head_angles.x) * 0.75
    } else {
        head_angles.x * 0.75
    };
    let pitch = swing_angles(&SwingAnglesInput {
        destination: pitch_destination,
        swing_tolerance: 15.0,
        clamp_tolerance: 30.0,
        speed: 0.1,
        frame_time_ms: input.frame_time_ms,
        angle: state.torso.pitch_angle,
        swinging: state.torso.pitching,
    })?;
    state.torso.pitch_angle = pitch.angle;
    state.torso.pitching = pitch.swinging;
    torso_angles = vec3(pitch.angle, torso_angles.y, torso_angles.z);

    if input.fixed_torso {
        torso_angles = vec3(0.0, torso_angles.y, torso_angles.z);
    }

    let speed = length3(input.entity.velocity);
    if speed != 0.0 {
        let velocity = normalize3(input.entity.velocity);
        let lean_speed = speed * 0.05;
        let legs_axis = qvm_angles_to_axis(legs_angles);
        let side = lean_speed * dot3(velocity, legs_axis[1]);
        let forward = lean_speed * dot3(velocity, legs_axis[0]);
        legs_angles = vec3(legs_angles.x + forward, legs_angles.y, legs_angles.z - side);
    }

    if input.fixed_legs {
        legs_angles = vec3(0.0, torso_angles.y, 0.0);
    }
    torso_angles = add_pain_twitch(
        torso_angles,
        &PainTwitchInput {
            time_ms: input.time_ms,
            pain_time: state.pain_time,
            pain_direction: state.pain_direction,
        },
    );
    let head_local = subtract_angles(head_angles, torso_angles);
    let torso_local = subtract_angles(torso_angles, legs_angles);
    Ok(PlayerPose {
        legs: qvm_angles_to_axis(legs_angles),
        torso: qvm_angles_to_axis(torso_local),
        head: qvm_angles_to_axis(head_local),
    })
}

// ---------------------------------------------------------------------------
// QVM math profile (`src/core/qvm-math.ts`).
// ---------------------------------------------------------------------------

pub(crate) const QVM_ANGLE_SCALE: f32 = (65536.0f64 / 360.0) as f32;

pub(crate) const QVM_ANGLE_UNSCALE: f32 = (360.0f64 / 65536.0) as f32;

pub(crate) const QVM_ANGLE_RADIANS: f32 = (std::f64::consts::PI * 2.0 / 360.0) as f32;

/// QVM `AngleMod` (`qvmAngleMod`).
#[must_use]
pub fn qvm_angle_mod(angle: f32) -> f32 {
    let scaled = angle * QVM_ANGLE_SCALE;
    f64::from(qvm_float_to_int(scaled) & 65535) as f32 * QVM_ANGLE_UNSCALE
}

/// Forward/right/up vectors (`AngleVectors`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmAngleVectors {
    /// Forward direction.
    pub forward: Vec3,
    /// Right direction.
    pub right: Vec3,
    /// Up direction.
    pub up: Vec3,
}

/// QVM angle vectors (`qvmAngleVectors`).
#[must_use]
pub fn qvm_angle_vectors(angles: Vec3) -> QvmAngleVectors {
    let yaw = angles.y * QVM_ANGLE_RADIANS;
    let pitch = angles.x * QVM_ANGLE_RADIANS;
    let roll = angles.z * QVM_ANGLE_RADIANS;
    let sy = f64::from(yaw).sin() as f32;
    let cy = f64::from(yaw).cos() as f32;
    let sp = f64::from(pitch).sin() as f32;
    let cp = f64::from(pitch).cos() as f32;
    let sr = f64::from(roll).sin() as f32;
    let cr = f64::from(roll).cos() as f32;
    QvmAngleVectors {
        forward: vec3(cp * cy, cp * sy, -sp),
        right: vec3((-sr * sp) * cy + -cr * -sy, (-sr * sp) * sy + -cr * cy, -sr * cp),
        up: vec3((cr * sp) * cy + -sr * -sy, (cr * sp) * sy + -sr * cy, cr * cp),
    }
}

/// QVM angles-to-axis (`qvmAnglesToAxis`).
#[must_use]
pub fn qvm_angles_to_axis(angles: Vec3) -> Axis {
    let vectors = qvm_angle_vectors(angles);
    [vectors.forward, sub3(vec3(0.0, 0.0, 0.0), vectors.right), vectors.up]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swing_angles_pain_twitch_and_pose() {
        let held = swing_angles(&SwingAnglesInput {
            destination: 10.0,
            swing_tolerance: 30.0,
            clamp_tolerance: 90.0,
            speed: 0.1,
            frame_time_ms: 16,
            angle: 0.0,
            swinging: false,
        })
        .unwrap();
        assert!(!held.swinging);
        assert_eq!(held.angle, 0.0);

        let moving = swing_angles(&SwingAnglesInput {
            destination: 100.0,
            swing_tolerance: 10.0,
            clamp_tolerance: 90.0,
            speed: 0.5,
            frame_time_ms: 10,
            angle: 0.0,
            swinging: false,
        })
        .unwrap();
        assert!(moving.swinging);
        assert!(moving.angle > 0.0);

        assert!(swing_angles(&SwingAnglesInput {
            destination: 0.0,
            swing_tolerance: 10.0,
            clamp_tolerance: 0.5,
            speed: 0.1,
            frame_time_ms: 16,
            angle: 0.0,
            swinging: false,
        })
        .is_err());

        let twitch = add_pain_twitch(
            vec3(1.0, 2.0, 3.0),
            &PainTwitchInput {
                time_ms: 100,
                pain_time: 0,
                pain_direction: true,
            },
        );
        assert!(twitch.z > 3.0);
        let settled = add_pain_twitch(
            vec3(1.0, 2.0, 3.0),
            &PainTwitchInput {
                time_ms: 500,
                pain_time: 0,
                pain_direction: true,
            },
        );
        assert_eq!(settled.z, 3.0);

        let mut pose = create_player_pose_state();
        let result = calculate_player_pose(
            &mut pose,
            &CalculatePlayerPoseInput {
                entity: PoseEntityState {
                    e_flags: 0,
                    velocity: vec3(100.0, 0.0, 0.0),
                    movement_direction: 1.0,
                    legs_anim: player_animation::LEGS_IDLE,
                    torso_anim: player_animation::TORSO_STAND,
                },
                fixed_legs: false,
                fixed_torso: false,
                lerp_angles: vec3(0.0, 90.0, 0.0),
                time_ms: 1000,
                frame_time_ms: 16,
                swing_speed: 0.2,
            },
        )
        .unwrap();
        assert!(pose.legs.yawing);
        assert_eq!(result.legs.len(), 3);

        let mut pose = create_player_pose_state();
        assert!(calculate_player_pose(
            &mut pose,
            &CalculatePlayerPoseInput {
                entity: PoseEntityState {
                    e_flags: 0,
                    velocity: vec3(0.0, 0.0, 0.0),
                    movement_direction: 9.0,
                    legs_anim: 0,
                    torso_anim: 0,
                },
                fixed_legs: false,
                fixed_torso: false,
                lerp_angles: vec3(0.0, 0.0, 0.0),
                time_ms: 0,
                frame_time_ms: 16,
                swing_speed: 0.2,
            },
        )
        .is_err());
    }
}
