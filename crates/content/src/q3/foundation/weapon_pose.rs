//! Quake III foundation: weapon pose.
//!
//! Donor provenance: `src/content/q3/foundation/weapon-pose.ts`.

use qa_core::math::{add3, vec3, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::animation_config::*;
use crate::q3::foundation::mirrors::*;

// ---------------------------------------------------------------------------
// weapon-pose.ts: CG_CalculateWeaponPosition, CG_MapTorsoToWeaponFrame,
// CG_MachinegunSpinAngle.
// ---------------------------------------------------------------------------

/// Weapon view motion (`Q3WeaponViewMotion`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3WeaponViewMotion {
    /// View origin.
    pub origin: Vec3,
    /// View angles.
    pub angles: Vec3,
    /// Clock in milliseconds.
    pub time_ms: i32,
    /// Horizontal speed.
    pub horizontal_speed: f32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Bob fraction sine.
    pub bob_fraction_sine: f32,
    /// Land time.
    pub land_time: i32,
    /// Land change.
    pub land_change: f32,
}

/// Weapon view pose (`q3WeaponViewPose`).
#[must_use]
pub fn q3_weapon_view_pose(input: &Q3WeaponViewMotion) -> (Vec3, Vec3) {
    let scale = if input.bob_cycle & 1 != 0 {
        -input.horizontal_speed
    } else {
        input.horizontal_speed
    };
    let roll = scale * input.bob_fraction_sine * 0.005;
    let yaw = scale * input.bob_fraction_sine * 0.01;
    let pitch = input.horizontal_speed * input.bob_fraction_sine * 0.005;
    let mut origin = input.origin;
    let mut angles = add3(input.angles, vec3(pitch, yaw, roll));
    let delta = input.time_ms.wrapping_sub(input.land_time);
    if delta < 150 {
        origin = add3(origin, vec3(0.0, 0.0, input.land_change * 0.25 * delta as f32 / 150.0));
    } else if delta < 450 {
        origin = add3(
            origin,
            vec3(
                0.0,
                0.0,
                input.land_change * 0.25 * 450_i32.wrapping_sub(delta) as f32 / 300.0,
            ),
        );
    }
    let drift = (input.horizontal_speed + 40.0) * f64::from(input.time_ms as f32 * 0.001).sin() as f32 * 0.01;
    angles = add3(angles, vec3(drift, drift, drift));
    (origin, angles)
}

/// Map a torso frame to its weapon frame (`q3TorsoWeaponFrame`).
pub fn q3_torso_weapon_frame(config: &PlayerAnimationConfig, frame: i32) -> Result<i32, Q3FoundationError> {
    for index in [
        Q3PlayerAnimation::TORSO_DROP,
        Q3PlayerAnimation::TORSO_ATTACK,
        Q3PlayerAnimation::TORSO_ATTACK2,
    ] {
        let animation = config.animations[index as usize]
            .ok_or_else(|| failed(format!("Missing Q3 weapon torso animation {index}")))?;
        let drop = index == Q3PlayerAnimation::TORSO_DROP;
        let width = if drop { 9 } else { 6 };
        if frame >= animation.first_frame && frame < animation.first_frame + width {
            return Ok(frame - animation.first_frame + if drop { 6 } else { 1 });
        }
    }
    Ok(0)
}

/// Machinegun barrel spin state (`Q3WeaponBarrel`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Q3WeaponBarrel {
    time: i32,
    angle: f32,
    spinning: bool,
}

/// Barrel step result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3BarrelStep {
    /// Barrel angle.
    pub angle: f32,
    /// Just stopped.
    pub stopped: bool,
}

impl Q3WeaponBarrel {
    /// Advance the barrel (`step`).
    pub fn step(&mut self, time_ms: i32, firing: bool) -> Q3BarrelStep {
        let mut delta = time_ms.wrapping_sub(self.time);
        let angle = if self.spinning {
            self.angle + delta as f32 * 0.9
        } else {
            if delta > 1000 {
                delta = 1000;
            }
            let speed = 0.5 * (0.9 + 1000_i32.wrapping_sub(delta) as f32 / 1000.0);
            self.angle + delta as f32 * speed
        };
        let stopped = self.spinning && !firing;
        if self.spinning != firing {
            self.time = time_ms;
            self.angle = qvm_angle_mod(angle);
            self.spinning = firing;
        }
        Q3BarrelStep { angle, stopped }
    }
}
