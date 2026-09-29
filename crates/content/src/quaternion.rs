//! MD5 quaternion arithmetic (`src/formats/q3-model/quaternion.ts`).
//!
//! Donor provenance: `src/formats/q3-model/quaternion.ts`, from q2repro
//! `common/math.c`. The donor keeps every intermediate in binary64 and
//! rounds once at each `vec3`/`vec4` construction; this port computes in
//! `f64` and casts once at the store, per the `qa-core` math discipline.

use qa_core::math::{vec3, vec4, Vec3, Vec4};

/// Build the MD5 orientation quaternion from its vector part
/// (`md5Quaternion`).
#[must_use]
pub fn md5_quaternion(value: Vec3) -> Vec4 {
    let (x, y, z) = (f64::from(value.x), f64::from(value.y), f64::from(value.z));
    let square = 1.0 - x * x - y * y - z * z;
    vec4(
        value.x,
        value.y,
        value.z,
        if square < 0.0 { 0.0 } else { -square.sqrt() as f32 },
    )
}

/// Conjugate a quaternion (`conjugateQuaternion`).
#[must_use]
pub fn conjugate_quaternion(value: Vec4) -> Vec4 {
    vec4(-value.x, -value.y, -value.z, value.w)
}

/// Multiply two quaternions (`multiplyQuaternion`).
#[must_use]
pub fn multiply_quaternion(a: Vec4, b: Vec4) -> Vec4 {
    let (ax, ay, az, aw) = (f64::from(a.x), f64::from(a.y), f64::from(a.z), f64::from(a.w));
    let (bx, by, bz, bw) = (f64::from(b.x), f64::from(b.y), f64::from(b.z), f64::from(b.w));
    vec4(
        (ax * bw + aw * bx + ay * bz - az * by) as f32,
        (ay * bw + aw * by + az * bx - ax * bz) as f32,
        (az * bw + aw * bz + ax * by - ay * bx) as f32,
        (aw * bw - ax * bx - ay * by - az * bz) as f32,
    )
}

/// Normalize a quaternion (`normalizeQuaternion`).
#[must_use]
pub fn normalize_quaternion(value: Vec4) -> Vec4 {
    let (x, y, z, w) = (
        f64::from(value.x),
        f64::from(value.y),
        f64::from(value.z),
        f64::from(value.w),
    );
    let length = (x * x + y * y + z * z + w * w).sqrt();
    if length == 0.0 {
        return value;
    }
    vec4(
        (x / length) as f32,
        (y / length) as f32,
        (z / length) as f32,
        (w / length) as f32,
    )
}

/// Rotate a position by an orientation quaternion (`rotateQuaternion`).
#[must_use]
pub fn rotate_quaternion(orientation: Vec4, position: Vec3) -> Vec3 {
    let product = multiply_quaternion(
        multiply_quaternion(orientation, vec4(position.x, position.y, position.z, 0.0)),
        conjugate_quaternion(orientation),
    );
    vec3(product.x, product.y, product.z)
}

fn rotation_coefficient(q: Vec4, selector: usize) -> f32 {
    let (x, y, z, w) = (f64::from(q.x), f64::from(q.y), f64::from(q.z), f64::from(q.w));
    let value = match selector {
        0 => 2.0 * (w * w + x * x) - 1.0,
        1 => 2.0 * (x * y - w * z),
        2 => 2.0 * (x * z + w * y),
        3 => 2.0 * (x * y + w * z),
        4 => 2.0 * (w * w + y * y) - 1.0,
        5 => 2.0 * (y * z - w * x),
        6 => 2.0 * (x * z - w * y),
        7 => 2.0 * (y * z + w * x),
        _ => 2.0 * (w * w + z * z) - 1.0,
    };
    value as f32
}

/// Rotate a vector through `Quat_ToAxis` rows (`rotateQuaternionAxis`).
#[must_use]
pub fn rotate_quaternion_axis(q: Vec4, v: Vec3) -> Vec3 {
    rotate_quaternion_rows(&quaternion_rotation_rows(q), v)
}

/// Retained `Quat_ToAxis` coefficients (`QuaternionRotationRows`).
pub type QuaternionRotationRows = [f32; 9];

/// Compute the `Quat_ToAxis` coefficient rows (`quaternionRotationRows`).
#[must_use]
pub fn quaternion_rotation_rows(q: Vec4) -> QuaternionRotationRows {
    [
        rotation_coefficient(q, 0),
        rotation_coefficient(q, 1),
        rotation_coefficient(q, 2),
        rotation_coefficient(q, 3),
        rotation_coefficient(q, 4),
        rotation_coefficient(q, 5),
        rotation_coefficient(q, 6),
        rotation_coefficient(q, 7),
        rotation_coefficient(q, 8),
    ]
}

/// Rotate a vector through retained rows (`rotateQuaternionRows`).
#[must_use]
pub fn rotate_quaternion_rows(rows: &QuaternionRotationRows, v: Vec3) -> Vec3 {
    let (x, y, z) = (f64::from(v.x), f64::from(v.y), f64::from(v.z));
    let row = |a: f32, b: f32, c: f32| (x * f64::from(a) + y * f64::from(b) + z * f64::from(c)) as f32;
    vec3(
        row(rows[0], rows[1], rows[2]),
        row(rows[3], rows[4], rows[5]),
        row(rows[6], rows[7], rows[8]),
    )
}

/// Blend two orientations (`slerpQuaternion`).
#[must_use]
pub fn slerp_quaternion(previous: Vec4, current: Vec4, back_lerp: f32) -> Vec4 {
    let back_lerp = f64::from(back_lerp);
    if back_lerp <= 0.0 {
        return current;
    }
    if back_lerp >= 1.0 {
        return previous;
    }
    let (px, py, pz, pw) = (
        f64::from(previous.x),
        f64::from(previous.y),
        f64::from(previous.z),
        f64::from(previous.w),
    );
    let (cx, cy, cz, cw) = (
        f64::from(current.x),
        f64::from(current.y),
        f64::from(current.z),
        f64::from(current.w),
    );
    let mut cosine = px * cx + py * cy + pz * cz + pw * cw;
    let sign = if cosine < 0.0 { -1.0 } else { 1.0 };
    cosine *= sign;
    let mut a = back_lerp;
    let mut b = 1.0 - back_lerp;
    if cosine <= 0.9995 {
        let sine = (1.0 - cosine * cosine).sqrt();
        let omega = sine.atan2(cosine);
        a = (back_lerp * omega).sin() / sine;
        b = ((1.0 - back_lerp) * omega).sin() / sine;
    }
    b *= sign;
    vec4(
        (a * px + b * cx) as f32,
        (a * py + b * cy) as f32,
        (a * pz + b * cz) as f32,
        (a * pw + b * cw) as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3 as make_vec3;

    #[test]
    fn quaternion_round_trip() {
        // Identity rotation: zero vector part yields -1 W.
        let identity = md5_quaternion(make_vec3(0.0, 0.0, 0.0));
        assert_eq!(identity, vec4(0.0, 0.0, 0.0, -1.0));
        let rotated = rotate_quaternion(identity, make_vec3(1.0, 2.0, 3.0));
        assert_eq!(rotated, make_vec3(1.0, 2.0, 3.0));
        assert_eq!(
            rotate_quaternion_axis(identity, make_vec3(1.0, 0.0, 0.0)),
            make_vec3(1.0, 0.0, 0.0)
        );
        let normalized = normalize_quaternion(vec4(0.0, 0.0, 0.0, -2.0));
        assert_eq!(normalized, vec4(0.0, 0.0, 0.0, -1.0));
        // Degenerate quaternions pass through.
        assert_eq!(normalize_quaternion(vec4(0.0, 0.0, 0.0, 0.0)), vec4(0.0, 0.0, 0.0, 0.0));
        // Endpoints pass through the blend.
        assert_eq!(slerp_quaternion(identity, normalized, 0.0), normalized);
        assert_eq!(slerp_quaternion(identity, normalized, 1.0), identity);
        let rows = quaternion_rotation_rows(identity);
        assert_eq!(rows[0], 1.0);
        assert_eq!(rows[4], 1.0);
        assert_eq!(rows[8], 1.0);
    }
}
