//! Hand grenade timing (`src/content/q2/foundation/weapons/hand-grenade.ts`).
//!
//! Quake II `p_weapon.c` / rerelease `p_weapon.cpp` (id Software,
//! GPL-2.0-or-later). Shared hand grenade calculation, independent of
//! weapon selection.

use qa_core::math::Vec3;

use crate::q2::foundation::host::Q2Edition;

/// Hand grenade tempo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandGrenadeTempo {
    /// Edition.
    pub edition: Q2Edition,
    /// Haste.
    pub haste: bool,
    /// Quad fire.
    pub quad_fire: bool,
}

/// Hand recovery seconds (`handRecoverySeconds`).
pub fn hand_recovery_seconds(tempo: &HandGrenadeTempo) -> f64 {
    if tempo.edition == Q2Edition::Classic {
        1.0
    } else {
        (if tempo.haste { 0.5 } else { 1.0 }) * (if tempo.quad_fire { 0.5 } else { 1.0 })
    }
}

/// Hand frame seconds (`handFrameSeconds`).
pub fn hand_frame_seconds(tempo: &HandGrenadeTempo) -> f64 {
    f64::from((100.0 * hand_recovery_seconds(tempo)).trunc() as i32) / 1000.0
}

/// Hand deadline (`handDeadline`).
pub fn hand_deadline(now: f64, seconds: f64, edition: Q2Edition) -> f64 {
    if edition == Q2Edition::Classic {
        now + seconds
    } else {
        (js_round(now * 1000.0) + js_round(seconds * 1000.0)) / 1000.0
    }
}

/// Hand fuse deadline (`handFuseDeadline`).
pub fn hand_fuse_deadline(now: f64, edition: Q2Edition) -> f64 {
    hand_deadline(now, 3.2, edition)
}

/// JavaScript `Math.round` (ties round toward positive infinity).
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// Hand throw input (`HandThrowInput`).
pub struct HandThrowInput<'a> {
    /// Edition.
    pub edition: Q2Edition,
    /// Throw angles.
    pub angles: Vec3,
    /// Whether alive.
    pub alive: bool,
    /// Now.
    pub now: f64,
    /// Fuse deadline.
    pub fuse_deadline: f64,
    /// Damage multiplier.
    pub damage_multiplier: f64,
    /// Gravity.
    pub gravity: f64,
    /// Whether held.
    pub held: bool,
    /// Muzzle projection.
    pub project: &'a mut dyn FnMut(Vec3, Vec3) -> (Vec3, Vec3),
}

/// Hand projectile spec (`HandProjectileSpec`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandProjectileSpec {
    /// Start.
    pub start: Vec3,
    /// Direction.
    pub direction: Vec3,
    /// Damage.
    pub damage: f64,
    /// Speed.
    pub speed: f64,
    /// Fuse.
    pub fuse: f64,
    /// Radius.
    pub radius: f64,
    /// Whether held.
    pub held: bool,
    /// Gravity.
    pub gravity: f64,
}

/// Calculate a hand throw (`calculateHandThrow`).
pub fn calculate_hand_throw(input: HandThrowInput) -> HandProjectileSpec {
    let rerelease = input.edition == Q2Edition::Rerelease;
    let angles = if rerelease {
        Vec3 {
            x: (-62.5f32).max(input.angles.x),
            y: input.angles.y,
            z: input.angles.z,
        }
    } else {
        input.angles
    };
    let (start, direction) = (input.project)(
        angles,
        if rerelease {
            Vec3 {
                x: 2.0,
                y: 0.0,
                z: -14.0,
            }
        } else {
            Vec3 {
                x: 8.0,
                y: 8.0,
                z: -8.0,
            }
        },
    );
    let fuse = input.fuse_deadline - input.now;
    let charged_speed = 400.0 + (3.0 - fuse) * (400.0 / 3.0);
    let speed = (if rerelease {
        if input.alive {
            charged_speed.min(800.0)
        } else {
            400.0
        }
    } else {
        charged_speed
    })
    .trunc();
    HandProjectileSpec {
        start,
        direction,
        damage: 125.0 * input.damage_multiplier,
        speed,
        fuse,
        radius: 165.0,
        held: input.held,
        gravity: input.gravity,
    }
}
