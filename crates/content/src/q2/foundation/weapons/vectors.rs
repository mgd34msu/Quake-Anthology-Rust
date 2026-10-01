//! Weapon vectors (`src/content/q2/foundation/weapons/vectors.ts`).

use qa_core::math::Vec3;

/// Angle axes (`AngleVectors`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AngleAxes {
    /// Forward axis.
    pub forward: Vec3,
    /// Right axis.
    pub right: Vec3,
    /// Up axis.
    pub up: Vec3,
}

/// Angle vectors (`angleVectors`).
pub fn angle_vectors(angles: Vec3) -> AngleAxes {
    let pitch = f64::from(angles.x) * std::f64::consts::PI / 180.0;
    let yaw = f64::from(angles.y) * std::f64::consts::PI / 180.0;
    let roll = f64::from(angles.z) * std::f64::consts::PI / 180.0;
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    let (sr, cr) = roll.sin_cos();
    AngleAxes {
        forward: Vec3 {
            x: (cp * cy) as f32,
            y: (cp * sy) as f32,
            z: (-sp) as f32,
        },
        right: Vec3 {
            x: (-sr * sp * cy + cr * sy) as f32,
            y: (-sr * sp * sy - cr * cy) as f32,
            z: (-sr * cp) as f32,
        },
        up: Vec3 {
            x: (cr * sp * cy + sr * sy) as f32,
            y: (cr * sp * sy - sr * cy) as f32,
            z: (cr * cp) as f32,
        },
    }
}

/// Vector angles (`vectorAngles`).
pub fn vector_angles(direction: Vec3) -> Vec3 {
    if direction.x == 0.0 && direction.y == 0.0 {
        return Vec3 {
            x: if direction.z > 0.0 { -90.0 } else { -270.0 },
            y: 0.0,
            z: 0.0,
        };
    }
    let mut yaw = f64::from(direction.y).atan2(f64::from(direction.x)) * 180.0 / std::f64::consts::PI;
    if yaw < 0.0 {
        yaw += 360.0;
    }
    let mut pitch = f64::from(direction.z).atan2(f64::from(direction.x).hypot(f64::from(direction.y))) * 180.0
        / std::f64::consts::PI;
    if pitch < 0.0 {
        pitch += 360.0;
    }
    Vec3 {
        x: -pitch as f32,
        y: yaw as f32,
        z: 0.0,
    }
}

/// Angle interpolation (`lerpAngle`).
pub fn lerp_angle(from: f64, to: f64, fraction: f64) -> f64 {
    let delta = to - from;
    from + (if delta > 180.0 {
        delta - 360.0
    } else if delta < -180.0 {
        delta + 360.0
    } else {
        delta
    }) * fraction
}
