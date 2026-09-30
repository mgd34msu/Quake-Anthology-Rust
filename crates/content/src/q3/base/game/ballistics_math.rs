//! Quake III base/game: ballistics math.
//!
//! Donor provenance: `src/content/q3/base/game/ballistics-math.ts`.

use qa_core::math::{add3, cross3, dot3, normalize3, normalize3_or_zero, perpendicular_vector, scale3, sub3, Vec3};
use qa_core::numeric::{q_crandom, qvm_float_to_int};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::entities::SimRandom;

// ---------------------------------------------------------------------------
// Ballistics (ballistics-math.ts).
// ---------------------------------------------------------------------------

/// Bullet attack frame (`BallisticAttack`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BallisticAttack {
    /// Muzzle origin.
    pub muzzle: Vec3,
    /// Forward direction.
    pub forward: Vec3,
    /// Right direction.
    pub right: Vec3,
    /// Up direction.
    pub up: Vec3,
}

/// Randomized bullet endpoint (`q3BulletEndpoint`).
#[must_use]
pub fn q3_bullet_endpoint(attack: &BallisticAttack, spread: f32, random: &mut dyn SimRandom) -> Vec3 {
    let angle = random.random_value() * std::f32::consts::PI * 2.0;
    let vertical = ((f64::from(angle).sin() as f32) * random.crandom_value()) * spread * 16.0;
    let horizontal = ((f64::from(angle).cos() as f32) * random.crandom_value()) * spread * 16.0;
    add3(
        add3(
            add3(attack.muzzle, scale3(attack.forward, 131072.0)),
            scale3(attack.right, horizontal),
        ),
        scale3(attack.up, vertical),
    )
}

/// Deterministic shotgun pellet endpoints (`q3ShotgunEndpoints`).
#[must_use]
pub fn q3_shotgun_endpoints(origin: Vec3, direction: Vec3, initial_seed: i32) -> Vec<Vec3> {
    let forward = normalize3_or_zero(direction);
    let right = perpendicular_vector(forward);
    let up = cross3(forward, right);
    let mut seed = initial_seed;
    let mut ends = Vec::with_capacity(11);
    for _ in 0..11 {
        let r = q_crandom(seed);
        let u = q_crandom(r.seed);
        seed = u.seed;
        let horizontal = ((r.value * 700.0) as f32) * 16.0;
        let vertical = ((u.value * 700.0) as f32) * 16.0;
        ends.push(add3(
            add3(add3(origin, scale3(forward, 131072.0)), scale3(right, horizontal)),
            scale3(up, vertical),
        ));
    }
    ends
}

/// Missile parameters (`Q3MissileParameters`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3MissileParameters {
    /// Missile speed.
    pub speed: f32,
    /// Lifetime milliseconds.
    pub duration: f32,
    /// Affected by gravity.
    pub gravity: bool,
    /// Direct damage.
    pub direct: f32,
    /// Splash damage.
    pub splash: f32,
    /// Splash radius.
    pub radius: f32,
    /// Direct means of death.
    pub method: i32,
    /// Splash means of death.
    pub splash_method: i32,
}

/// Missile parameters by weapon (`q3MissileParameters`).
#[must_use]
pub fn q3_missile_parameters(weapon: i32) -> Q3MissileParameters {
    match weapon {
        4 => Q3MissileParameters {
            speed: 700.0,
            duration: 2500.0,
            gravity: true,
            direct: 100.0,
            splash: 100.0,
            radius: 150.0,
            method: 4,
            splash_method: 5,
        },
        5 => Q3MissileParameters {
            speed: 900.0,
            duration: 15000.0,
            gravity: false,
            direct: 100.0,
            splash: 100.0,
            radius: 120.0,
            method: 6,
            splash_method: 7,
        },
        8 => Q3MissileParameters {
            speed: 2000.0,
            duration: 10000.0,
            gravity: false,
            direct: 20.0,
            splash: 15.0,
            radius: 20.0,
            method: 8,
            splash_method: 9,
        },
        9 => Q3MissileParameters {
            speed: 2000.0,
            duration: 10000.0,
            gravity: false,
            direct: 100.0,
            splash: 100.0,
            radius: 120.0,
            method: 12,
            splash_method: 13,
        },
        _ => panic!("No Q3 missile parameters for weapon {weapon}"),
    }
}

/// Randomized nail velocity (`q3NailVelocity`).
#[must_use]
pub fn q3_nail_velocity(start: Vec3, forward: Vec3, right: Vec3, up: Vec3, random: &mut dyn SimRandom) -> Vec3 {
    let angle = random.random_value() * std::f32::consts::PI * 2.0;
    let vertical = ((f64::from(angle).sin() as f32) * random.crandom_value()) * 500.0 * 16.0;
    let horizontal = ((f64::from(angle).cos() as f32) * random.crandom_value()) * 500.0 * 16.0;
    let end = add3(
        add3(add3(start, scale3(forward, 8192.0 * 16.0)), scale3(right, horizontal)),
        scale3(up, vertical),
    );
    let direction = normalize3(sub3(end, start));
    let speed = 555.0 + random.random_value() * 1800.0;
    scale3(direction, speed)
}

/// Reflected bounce velocity (`q3BounceVelocity`).
#[must_use]
pub fn q3_bounce_velocity(velocity: Vec3, normal: Vec3, half: bool) -> Vec3 {
    let delta = add3(velocity, scale3(normal, -2.0 * dot3(velocity, normal)));
    if half {
        scale3(delta, 0.65)
    } else {
        delta
    }
}

/// Missile impact time milliseconds (`q3MissileHitTime`).
#[must_use]
pub fn q3_missile_hit_time(previous: i32, time: i32, fraction: f32) -> i32 {
    let elapsed = time.wrapping_sub(previous) as f32;
    qvm_float_to_int(previous as f32 + elapsed * fraction)
}
