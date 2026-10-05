//! Rerelease brain math from `src/bots/behavior/rerelease/math.ts`.
//!
//! Game-agnostic vector and angle arithmetic. Pitch is negative
//! looking up; angles are (pitch, yaw, roll) in degrees.

use qa_core::math::{angle_mod_rerelease, Vec3};

/// Zero vector.
#[must_use]
pub fn bvec() -> Vec3 {
    Vec3 { x: 0.0, y: 0.0, z: 0.0 }
}

/// Vector with components.
#[must_use]
pub fn bvec3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

/// Add vectors.
#[must_use]
pub fn bvec_add(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

/// Subtract vectors.
#[must_use]
pub fn bvec_sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

/// Scale a vector.
#[must_use]
pub fn bvec_scale(v: Vec3, s: f32) -> Vec3 {
    Vec3 {
        x: v.x * s,
        y: v.y * s,
        z: v.z * s,
    }
}

/// Multiply-add: `base + scale * dir`.
#[must_use]
pub fn bvec_ma(base: Vec3, scale: f32, dir: Vec3) -> Vec3 {
    Vec3 {
        x: base.x + dir.x * scale,
        y: base.y + dir.y * scale,
        z: base.z + dir.z * scale,
    }
}

/// Dot product.
#[must_use]
pub fn bvec_dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// Length.
#[must_use]
pub fn bvec_length(v: Vec3) -> f32 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

/// 2D length.
#[must_use]
pub fn bvec_length_2d(v: Vec3) -> f32 {
    (v.x * v.x + v.y * v.y).sqrt()
}

/// Distance.
#[must_use]
pub fn bvec_distance(a: Vec3, b: Vec3) -> f32 {
    bvec_length(bvec_sub(a, b))
}

/// 2D distance.
#[must_use]
pub fn bvec_distance_2d(a: Vec3, b: Vec3) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

/// Unit-length copy; zero stays zero.
#[must_use]
pub fn bvec_normalized(v: Vec3) -> Vec3 {
    let len = bvec_length(v);
    if len == 0.0 {
        bvec()
    } else {
        Vec3 {
            x: v.x / len,
            y: v.y / len,
            z: v.z / len,
        }
    }
}

/// Wrap an angle into `[0, 360)` with the rerelease `fmod` form.
#[must_use]
pub fn angle_mod(a: f32) -> f32 {
    angle_mod_rerelease(f64::from(a)) as f32
}

/// Shortest signed rotation from `from` to `to`, in `(-180, 180]`.
#[must_use]
pub fn angle_delta(from: f32, to: f32) -> f32 {
    let mut d = angle_mod(to - from);
    if d > 180.0 {
        d -= 360.0;
    }
    d
}

/// Pitch/yaw to look along `dir`.
#[must_use]
pub fn vector_to_angles(dir: Vec3) -> (f32, f32) {
    if dir.x == 0.0 && dir.y == 0.0 {
        return (if dir.z > 0.0 { -90.0 } else { 90.0 }, 0.0);
    }
    let yaw = angle_mod(dir.y.atan2(dir.x).to_degrees());
    let forward = (dir.x * dir.x + dir.y * dir.y).sqrt();
    let pitch = -dir.z.atan2(forward).to_degrees();
    (pitch, yaw)
}

/// Forward/right/up basis for pitch/yaw/roll.
#[must_use]
pub fn angle_vectors(pitch: f32, yaw: f32, roll: f32) -> (Vec3, Vec3, Vec3) {
    let (sy, cy) = yaw.to_radians().sin_cos();
    let (sp, cp) = pitch.to_radians().sin_cos();
    let (sr, cr) = roll.to_radians().sin_cos();
    (
        Vec3 {
            x: cp * cy,
            y: cp * sy,
            z: -sp,
        },
        Vec3 {
            x: -sr * sp * cy + cr * sy,
            y: -sr * sp * sy - cr * cy,
            z: -sr * cp,
        },
        Vec3 {
            x: cr * sp * cy + sr * sy,
            y: cr * sp * sy - sr * cy,
            z: cr * cp,
        },
    )
}

/// Angle in degrees between `dir` and the view direction.
#[must_use]
pub fn angle_between(pitch: f32, yaw: f32, dir: Vec3) -> f32 {
    let (forward, _, _) = angle_vectors(pitch, yaw, 0.0);
    let d = bvec_normalized(dir);
    bvec_dot(forward, d).clamp(-1.0, 1.0).acos().to_degrees()
}

#[cfg(test)]
mod tests {
    use super::angle_mod;

    /// Rerelease `anglemod` is the `fmod` form: fractional inputs survive.
    #[test]
    fn rerelease_angle_mod_uses_fmod() {
        assert_eq!(angle_mod(720.5), 0.5);
        assert_eq!(angle_mod(-90.0), 270.0);
        assert_eq!(angle_mod(360.0), 0.0);
        assert_eq!(angle_mod(0.0), 0.0);
    }
}
