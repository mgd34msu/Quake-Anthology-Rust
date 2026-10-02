//! Vector and angle routines ported from `src/core/math.ts` (Q3 `q_math.c`,
//! Q1/Q2 `mathlib`) and `src/contracts/math.ts`.
//!
//! The unqualified helpers retain the Q3 donor's operation sequence: every
//! intermediate that the donor passes through `Math.fround` is an `f32`
//! operation here, and intermediates the donor keeps in binary64 (`1/x`
//! scale factors, `length3` roots, trigonometric calls, Q1/Q2 donor paths)
//! are computed in `f64` and rounded once at the store.

use thiserror::Error;

use crate::numeric::float_to_wrapped_i32;

/// Error for math helpers with integer preconditions.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MathError {
    /// `clamp_short` / `q_log2` need a signed 32-bit integer.
    #[error("q_math integer argument must be a signed 32-bit integer")]
    IntArgument,
    /// `Q_log2` never terminates for negative source integers.
    #[error("Q_log2 does not terminate for negative source integers")]
    NegativeLog,
    /// Native float-to-byte conversion left `0..=255`.
    #[error("ColorBytes3 native float-to-byte conversion is outside its defined range")]
    ColorRange,
    /// `FloorDivMod` needs a positive denominator.
    #[error("FloorDivMod: bad denominator")]
    BadDenominator,
}

/// Two-component single-precision vector.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vec2 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
}

/// Three-component single-precision vector.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vec3 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
}

/// Four-component single-precision vector.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vec4 {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
    /// W component.
    pub w: f32,
}

/// Axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// Minimum corner.
    pub min: Vec3,
    /// Maximum corner.
    pub max: Vec3,
}

/// Plane in Hessian form (`dot(normal, p) = distance`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
    /// Unit normal.
    pub normal: Vec3,
    /// Distance from the origin along the normal.
    pub distance: f32,
}

/// Quake axes are forward, left, up.
pub type Axis = [Vec3; 3];

/// Orthonormal basis derived from Euler angles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AngleVectors {
    /// Forward direction.
    pub forward: Vec3,
    /// Right direction.
    pub right: Vec3,
    /// Up direction.
    pub up: Vec3,
}

/// Column-major 4x4 matrix for column vectors, matching OpenGL.
pub type Mat4 = [f32; 16];

type Mat3 = [Vec3; 3];

/// Degrees-to-radians factor (`Math.PI * 2 / 360`).
const DEGREES_TO_RADIANS: f64 = std::f64::consts::PI * 2.0 / 360.0;

/// Build a 2-vector.
#[must_use]
pub fn vec2(x: f32, y: f32) -> Vec2 {
    Vec2 { x, y }
}

/// Build a 3-vector.
#[must_use]
pub fn vec3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

/// Build a 4-vector.
#[must_use]
pub fn vec4(x: f32, y: f32, z: f32, w: f32) -> Vec4 {
    Vec4 { x, y, z, w }
}

/// Component-wise addition.
#[must_use]
pub fn add3(a: Vec3, b: Vec3) -> Vec3 {
    vec3(a.x + b.x, a.y + b.y, a.z + b.z)
}

/// Component-wise subtraction.
#[must_use]
pub fn sub3(a: Vec3, b: Vec3) -> Vec3 {
    vec3(a.x - b.x, a.y - b.y, a.z - b.z)
}

/// Scale every component.
#[must_use]
pub fn scale3(value: Vec3, scale: f32) -> Vec3 {
    vec3(value.x * scale, value.y * scale, value.z * scale)
}

/// Dot product with the donor's ordered rounding: each product, then each
/// ordered addition, rounds to binary32.
#[must_use]
pub fn dot3(a: Vec3, b: Vec3) -> f32 {
    (a.x * b.x + a.y * b.y) + a.z * b.z
}

/// Cross product with per-product binary32 rounding.
#[must_use]
pub fn cross3(a: Vec3, b: Vec3) -> Vec3 {
    vec3(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

/// Euclidean length: binary64 root of the binary32 dot product, stored once.
#[must_use]
pub fn length3(value: Vec3) -> f32 {
    f64::from(dot3(value, value)).sqrt() as f32
}

/// Normalize; a zero vector keeps its input (matching `VectorNormalize`).
#[must_use]
pub fn normalize3(value: Vec3) -> Vec3 {
    let length = length3(value);
    if length == 0.0 {
        vec3(value.x, value.y, value.z)
    } else {
        scale3(value, (1.0 / f64::from(length)) as f32)
    }
}

/// Normalize; a zero vector clears its output (matching `VectorNormalize2`).
#[must_use]
pub fn normalize3_or_zero(value: Vec3) -> Vec3 {
    let length = length3(value);
    if length == 0.0 {
        vec3(0.0, 0.0, 0.0)
    } else {
        scale3(value, (1.0 / f64::from(length)) as f32)
    }
}

/// Clamp a signed 32-bit integer to the `i16` range.
pub fn clamp_short(value: i32) -> i32 {
    value.clamp(-32_768, 32_767)
}

/// `Q_log2`: position of the highest set bit; zero maps to zero.
pub fn q_log2(value: i32) -> Result<i32, MathError> {
    if value < 0 {
        return Err(MathError::NegativeLog);
    }
    let mut integer = value;
    let mut answer = 0;
    while {
        integer >>= 1;
        integer
    } != 0
    {
        answer += 1;
    }
    Ok(answer)
}

/// Normalize a color by its ordered maximum; returns the maximum.
pub fn normalize_color(input: Vec3, output: &mut Vec3) -> f32 {
    let mut maximum = input.x;
    if input.y > maximum {
        maximum = input.y;
    }
    if input.z > maximum {
        maximum = input.z;
    }
    if maximum == 0.0 {
        output.x = 0.0;
        output.y = 0.0;
        output.z = 0.0;
    } else {
        output.x = input.x / maximum;
        output.y = input.y / maximum;
        output.z = input.z / maximum;
    }
    maximum
}

fn native_color_byte(value: f32) -> Result<u8, MathError> {
    let integer = (value * 255.0).trunc();
    if !integer.is_finite() || integer < 0.0 || integer > 255.0 {
        return Err(MathError::ColorRange);
    }
    Ok(integer as u8)
}

/// Native little-endian `ColorBytes3`; the caller supplies the retained
/// fourth byte of `storage`.
pub fn color_bytes3(red: f32, green: f32, blue: f32, storage: &mut [u8; 4]) -> Result<u32, MathError> {
    storage[0] = native_color_byte(red)?;
    storage[1] = native_color_byte(green)?;
    storage[2] = native_color_byte(blue)?;
    Ok(u32::from_le_bytes(*storage))
}

/// Donor `Math.min` over binary64 inputs, including NaN and signed-zero edges.
#[must_use]
pub fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        -0.0
    } else {
        a.min(b)
    }
}

/// Donor `Math.max` over binary64 inputs, including NaN and signed-zero edges.
#[must_use]
pub fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        0.0
    } else {
        a.max(b)
    }
}

/// Donor `Math.min` over binary32 inputs.
#[must_use]
pub fn js_min_f32(a: f32, b: f32) -> f32 {
    js_min(f64::from(a), f64::from(b)) as f32
}

/// Donor `Math.max` over binary32 inputs.
#[must_use]
pub fn js_max_f32(a: f32, b: f32) -> f32 {
    js_max(f64::from(a), f64::from(b)) as f32
}

/// Linear interpolation between two points.
#[must_use]
pub fn lerp3(from: Vec3, to: Vec3, fraction: f32) -> Vec3 {
    vec3(
        from.x + fraction * (to.x - from.x),
        from.y + fraction * (to.y - from.y),
        from.z + fraction * (to.z - from.z),
    )
}

/// Derive forward/right/up from Euler angles (`x` = pitch, `y` = yaw,
/// `z` = roll, in degrees). Trigonometric calls evaluate in binary64 and
/// store once, matching the donor.
#[must_use]
pub fn angle_vectors(angles: Vec3) -> AngleVectors {
    let yaw = (f64::from(angles.y) * DEGREES_TO_RADIANS) as f32;
    let pitch = (f64::from(angles.x) * DEGREES_TO_RADIANS) as f32;
    let roll = (f64::from(angles.z) * DEGREES_TO_RADIANS) as f32;
    let sy = f64::from(yaw).sin() as f32;
    let cy = f64::from(yaw).cos() as f32;
    let sp = f64::from(pitch).sin() as f32;
    let cp = f64::from(pitch).cos() as f32;
    let sr = f64::from(roll).sin() as f32;
    let cr = f64::from(roll).cos() as f32;
    AngleVectors {
        forward: vec3(cp * cy, cp * sy, -sp),
        right: vec3((-sr * sp) * cy + -cr * -sy, (-sr * sp) * sy + -cr * cy, -sr * cp),
        up: vec3((cr * sp) * cy + -sr * -sy, (cr * sp) * sy + -sr * cy, cr * cp),
    }
}

/// Build the forward/left/up axis triple from Euler angles.
#[must_use]
pub fn angles_to_axis(angles: Vec3) -> Axis {
    let vectors = angle_vectors(angles);
    [vectors.forward, scale3(vectors.right, -1.0), vectors.up]
}

/// Convert a direction to Euler angles (`-pitch`, `yaw`, `0`).
#[must_use]
pub fn vector_to_angles(value: Vec3) -> Vec3 {
    let qvm_pi = std::f32::consts::PI;
    let (yaw, pitch) = if value.y == 0.0 && value.x == 0.0 {
        (0.0, if value.z > 0.0 { 90.0 } else { 270.0 })
    } else {
        let mut yaw = if value.x != 0.0 {
            let yaw_radians = f64::from(value.y).atan2(f64::from(value.x)) as f32;
            (f64::from(yaw_radians * 180.0) / f64::from(qvm_pi)) as f32
        } else if value.y > 0.0 {
            90.0
        } else {
            270.0
        };
        if yaw < 0.0 {
            yaw += 360.0;
        }
        let forward = (f64::from(value.x * value.x + value.y * value.y).sqrt()) as f32;
        let pitch_radians = f64::from(value.z).atan2(f64::from(forward)) as f32;
        let mut pitch = (f64::from(pitch_radians * 180.0) / f64::from(qvm_pi)) as f32;
        if pitch < 0.0 {
            pitch += 360.0;
        }
        (yaw, pitch)
    };
    vec3(-pitch, yaw, 0.0)
}

/// Reduce an angle to `[0, 360)` via the donor's 16-bit fixed-point path.
#[must_use]
pub fn angle_mod(angle: f64) -> f64 {
    (360.0 / 65_536.0) * f64::from(float_to_wrapped_i32(angle * (65_536.0 / 360.0)) & 65_535)
}

/// Normalize an angle to `[0, 360)`.
#[must_use]
pub fn angle_normalize360(angle: f64) -> f64 {
    angle_mod(angle)
}

/// Normalize an angle to `(-180, 180]`.
#[must_use]
pub fn angle_normalize180(angle: f64) -> f64 {
    let normalized = angle_normalize360(angle);
    if normalized > 180.0 {
        normalized - 360.0
    } else {
        normalized
    }
}

/// Shortest signed difference between two angles.
#[must_use]
pub fn angle_delta(angle1: f64, angle2: f64) -> f64 {
    angle_normalize180(angle1 - angle2)
}

/// Interpolate between angles along the shortest path.
#[must_use]
pub fn lerp_angle(from: f64, to: f64, fraction: f64) -> f64 {
    let mut end = to;
    if end - from > 180.0 {
        end -= 360.0;
    }
    if end - from < -180.0 {
        end += 360.0;
    }
    from + fraction * (end - from)
}

/// Project a point onto a plane through the origin. The normal is expected
/// to be nonzero, matching `q_math.c`.
#[must_use]
pub fn project_point_on_plane(point: Vec3, normal: Vec3) -> Vec3 {
    let inverse = (1.0 / f64::from(dot3(normal, normal))) as f32;
    let distance = dot3(normal, point) * inverse;
    let scaled = scale3(normal, inverse);
    sub3(point, scale3(scaled, distance))
}

/// Find a vector perpendicular to a normalized source.
#[must_use]
pub fn perpendicular_vector(source: Vec3) -> Vec3 {
    let mut axis = vec3(1.0, 0.0, 0.0);
    let mut minimum = 1.0;
    if source.x.abs() < minimum {
        minimum = source.x.abs();
        axis = vec3(1.0, 0.0, 0.0);
    }
    if source.y.abs() < minimum {
        minimum = source.y.abs();
        axis = vec3(0.0, 1.0, 0.0);
    }
    if source.z.abs() < minimum {
        axis = vec3(0.0, 0.0, 1.0);
    }
    normalize3(project_point_on_plane(axis, source))
}

/// Rotate a point around a normalized direction by degrees.
#[must_use]
pub fn rotate_point_around_vector(direction: Vec3, point: Vec3, degrees: f64) -> Vec3 {
    let radial = perpendicular_vector(direction);
    let vertical = cross3(radial, direction);
    let radians = ((f64::from(degrees as f32) * std::f64::consts::PI) / 180.0) as f32;
    let cosine = f64::from(radians).cos() as f32;
    let sine = f64::from(radians).sin() as f32;
    let basis: Mat3 = [
        vec3(radial.x, vertical.x, direction.x),
        vec3(radial.y, vertical.y, direction.y),
        vec3(radial.z, vertical.z, direction.z),
    ];
    let z_rotation: Mat3 = [vec3(cosine, sine, 0.0), vec3(-sine, cosine, 0.0), vec3(0.0, 0.0, 1.0)];
    let inverse: Mat3 = [radial, vertical, direction];
    let rotation = multiply_mat3(multiply_mat3(basis, z_rotation), inverse);
    vec3(
        dot3(rotation[0], point),
        dot3(rotation[1], point),
        dot3(rotation[2], point),
    )
}

fn multiply_mat3(a: Mat3, b: Mat3) -> Mat3 {
    let x = vec3(b[0].x, b[1].x, b[2].x);
    let y = vec3(b[0].y, b[1].y, b[2].y);
    let z = vec3(b[0].z, b[1].z, b[2].z);
    [
        vec3(dot3(a[0], x), dot3(a[0], y), dot3(a[0], z)),
        vec3(dot3(a[1], x), dot3(a[1], y), dot3(a[1], z)),
        vec3(dot3(a[2], x), dot3(a[2], y), dot3(a[2], z)),
    ]
}

/// Empty bounds for accumulation.
#[must_use]
pub fn empty_bounds() -> Bounds {
    Bounds {
        min: vec3(99_999.0, 99_999.0, 99_999.0),
        max: vec3(-99_999.0, -99_999.0, -99_999.0),
    }
}

/// Grow bounds to include a point.
#[must_use]
pub fn add_point_to_bounds(bounds: Bounds, point: Vec3) -> Bounds {
    Bounds {
        min: vec3(
            if point.x < bounds.min.x { point.x } else { bounds.min.x },
            if point.y < bounds.min.y { point.y } else { bounds.min.y },
            if point.z < bounds.min.z { point.z } else { bounds.min.z },
        ),
        max: vec3(
            if point.x > bounds.max.x { point.x } else { bounds.max.x },
            if point.y > bounds.max.y { point.y } else { bounds.max.y },
            if point.z > bounds.max.z { point.z } else { bounds.max.z },
        ),
    }
}

/// Radius of the sphere centered at the origin that contains the bounds.
#[must_use]
pub fn radius_from_bounds(bounds: Bounds) -> f32 {
    length3(vec3(
        bounds.min.x.abs().max(bounds.max.x.abs()),
        bounds.min.y.abs().max(bounds.max.y.abs()),
        bounds.min.z.abs().max(bounds.max.z.abs()),
    ))
}

/// Plane through three points, or `None` for a degenerate triangle.
/// Clockwise points produce an outward normal.
#[must_use]
pub fn plane_from_points(a: Vec3, b: Vec3, c: Vec3) -> Option<Plane> {
    let first_edge = sub3(b, a);
    let second_edge = sub3(c, a);
    let cross = cross3(second_edge, first_edge);
    let length = length3(cross);
    if length == 0.0 {
        return None;
    }
    let normal = scale3(cross, (1.0 / f64::from(length)) as f32);
    Some(Plane {
        normal,
        distance: dot3(a, normal),
    })
}

/// Signed distance from a point to a plane.
#[must_use]
pub fn distance_to_plane(plane: Plane, point: Vec3) -> f32 {
    dot3(plane.normal, point) - plane.distance
}

/// Quake front/back bitmask: 1 front, 2 back, or 3 crossing.
#[must_use]
pub fn box_on_plane_side(bounds: Bounds, plane: Plane) -> u32 {
    let front_corner = vec3(
        if plane.normal.x < 0.0 {
            bounds.min.x
        } else {
            bounds.max.x
        },
        if plane.normal.y < 0.0 {
            bounds.min.y
        } else {
            bounds.max.y
        },
        if plane.normal.z < 0.0 {
            bounds.min.z
        } else {
            bounds.max.z
        },
    );
    let back_corner = vec3(
        if plane.normal.x < 0.0 {
            bounds.max.x
        } else {
            bounds.min.x
        },
        if plane.normal.y < 0.0 {
            bounds.max.y
        } else {
            bounds.min.y
        },
        if plane.normal.z < 0.0 {
            bounds.max.z
        } else {
            bounds.min.z
        },
    );
    let mut sides = 0;
    if distance_to_plane(plane, front_corner) >= 0.0 {
        sides = 1;
    }
    if distance_to_plane(plane, back_corner) < 0.0 {
        sides |= 2;
    }
    sides
}

/// Identity matrix.
#[must_use]
pub fn identity_mat4() -> Mat4 {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

/// Column-major matrix multiplication.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn multiply_mat4(a: Mat4, b: Mat4) -> Mat4 {
    [
        a[0] * b[0] + a[4] * b[1] + a[8] * b[2] + a[12] * b[3],
        a[1] * b[0] + a[5] * b[1] + a[9] * b[2] + a[13] * b[3],
        a[2] * b[0] + a[6] * b[1] + a[10] * b[2] + a[14] * b[3],
        a[3] * b[0] + a[7] * b[1] + a[11] * b[2] + a[15] * b[3],
        a[0] * b[4] + a[4] * b[5] + a[8] * b[6] + a[12] * b[7],
        a[1] * b[4] + a[5] * b[5] + a[9] * b[6] + a[13] * b[7],
        a[2] * b[4] + a[6] * b[5] + a[10] * b[6] + a[14] * b[7],
        a[3] * b[4] + a[7] * b[5] + a[11] * b[6] + a[15] * b[7],
        a[0] * b[8] + a[4] * b[9] + a[8] * b[10] + a[12] * b[11],
        a[1] * b[8] + a[5] * b[9] + a[9] * b[10] + a[13] * b[11],
        a[2] * b[8] + a[6] * b[9] + a[10] * b[10] + a[14] * b[11],
        a[3] * b[8] + a[7] * b[9] + a[11] * b[10] + a[15] * b[11],
        a[0] * b[12] + a[4] * b[13] + a[8] * b[14] + a[12] * b[15],
        a[1] * b[12] + a[5] * b[13] + a[9] * b[14] + a[13] * b[15],
        a[2] * b[12] + a[6] * b[13] + a[10] * b[14] + a[14] * b[15],
        a[3] * b[12] + a[7] * b[13] + a[11] * b[14] + a[15] * b[15],
    ]
}

/// Right-handed OpenGL perspective matrix with NDC depth `[-1, 1]`.
#[must_use]
pub fn perspective_mat4(vertical_fov_degrees: f64, aspect_ratio: f64, near: f64, far: f64) -> Mat4 {
    let focal_length = 1.0 / (vertical_fov_degrees * DEGREES_TO_RADIANS / 2.0).tan();
    let depth_scale = 1.0 / (near - far);
    [
        (focal_length / aspect_ratio) as f32,
        0.0,
        0.0,
        0.0,
        0.0,
        focal_length as f32,
        0.0,
        0.0,
        0.0,
        0.0,
        ((far + near) * depth_scale) as f32,
        -1.0,
        0.0,
        0.0,
        (2.0 * far * near * depth_scale) as f32,
        0.0,
    ]
}

/// Right-handed world-to-view matrix.
#[must_use]
pub fn look_at_mat4(eye: Vec3, target: Vec3, up: Vec3) -> Mat4 {
    let backward = normalize3(sub3(eye, target));
    let right = normalize3(cross3(up, backward));
    let camera_up = cross3(backward, right);
    [
        right.x,
        camera_up.x,
        backward.x,
        0.0,
        right.y,
        camera_up.y,
        backward.y,
        0.0,
        right.z,
        camera_up.z,
        backward.z,
        0.0,
        -dot3(right, eye),
        -dot3(camera_up, eye),
        -dot3(backward, eye),
        1.0,
    ]
}

/// Transform a column vector by a matrix.
#[must_use]
pub fn transform_vec4(matrix: Mat4, value: Vec4) -> Vec4 {
    vec4(
        matrix[0] * value.x + matrix[4] * value.y + matrix[8] * value.z + matrix[12] * value.w,
        matrix[1] * value.x + matrix[5] * value.y + matrix[9] * value.z + matrix[13] * value.w,
        matrix[2] * value.x + matrix[6] * value.y + matrix[10] * value.z + matrix[14] * value.w,
        matrix[3] * value.x + matrix[7] * value.y + matrix[11] * value.z + matrix[15] * value.w,
    )
}

/// Q1/Q2 donor angle vectors: trigonometric intermediates stay in binary64
/// with binary32 stores. Each output is optional, matching the donor's
/// nullable vector arguments.
pub fn donor_angle_vectors(angles: Vec3, forward: Option<&mut Vec3>, right: Option<&mut Vec3>, up: Option<&mut Vec3>) {
    let yaw = f64::from(angles.y) * DEGREES_TO_RADIANS;
    let sy = yaw.sin();
    let cy = yaw.cos();
    let pitch = f64::from(angles.x) * DEGREES_TO_RADIANS;
    let sp = pitch.sin();
    let cp = pitch.cos();
    let roll = f64::from(angles.z) * DEGREES_TO_RADIANS;
    let sr = roll.sin();
    let cr = roll.cos();
    if let Some(out) = forward {
        out.x = (cp * cy) as f32;
        out.y = (cp * sy) as f32;
        out.z = (-sp) as f32;
    }
    if let Some(out) = right {
        out.x = (-sr * sp * cy + -cr * -sy) as f32;
        out.y = (-sr * sp * sy + -cr * cy) as f32;
        out.z = (-sr * cp) as f32;
    }
    if let Some(out) = up {
        out.x = (cr * sp * cy + -sr * -sy) as f32;
        out.y = (cr * sp * sy + -sr * cy) as f32;
        out.z = (cr * cp) as f32;
    }
}

/// Q1/Q2 donor rotation concatenation: binary64 arithmetic, binary32 stores.
pub fn donor_concat_rotations(first: Mat3, second: Mat3, output: &mut [Vec3; 3]) {
    for (row, target) in first.iter().zip(output.iter_mut()) {
        target.x = (f64::from(row.x) * f64::from(second[0].x)
            + f64::from(row.y) * f64::from(second[1].x)
            + f64::from(row.z) * f64::from(second[2].x)) as f32;
        target.y = (f64::from(row.x) * f64::from(second[0].y)
            + f64::from(row.y) * f64::from(second[1].y)
            + f64::from(row.z) * f64::from(second[2].y)) as f32;
        target.z = (f64::from(row.x) * f64::from(second[0].z)
            + f64::from(row.y) * f64::from(second[1].z)
            + f64::from(row.z) * f64::from(second[2].z)) as f32;
    }
}

fn donor_dot(a: Vec3, b: Vec3) -> f64 {
    f64::from(a.x) * f64::from(b.x) + f64::from(a.y) * f64::from(b.y) + f64::from(a.z) * f64::from(b.z)
}

fn donor_cross(a: Vec3, b: Vec3, output: &mut Vec3) {
    output.x = (f64::from(a.y) * f64::from(b.z) - f64::from(a.z) * f64::from(b.y)) as f32;
    output.y = (f64::from(a.z) * f64::from(b.x) - f64::from(a.x) * f64::from(b.z)) as f32;
    output.z = (f64::from(a.x) * f64::from(b.y) - f64::from(a.y) * f64::from(b.x)) as f32;
}

fn donor_normalize_in_place(value: &mut Vec3) {
    let length = donor_dot(*value, *value).sqrt();
    if length != 0.0 {
        let scale = 1.0 / length;
        value.x = (f64::from(value.x) * scale) as f32;
        value.y = (f64::from(value.y) * scale) as f32;
        value.z = (f64::from(value.z) * scale) as f32;
    }
}

fn donor_project_point_on_plane(output: &mut Vec3, point: Vec3, normal: Vec3) {
    let inverse = 1.0 / donor_dot(normal, normal);
    let distance = donor_dot(normal, point) * inverse;
    let scaled = vec3(
        (f64::from(normal.x) * inverse) as f32,
        (f64::from(normal.y) * inverse) as f32,
        (f64::from(normal.z) * inverse) as f32,
    );
    output.x = (f64::from(point.x) - distance * f64::from(scaled.x)) as f32;
    output.y = (f64::from(point.y) - distance * f64::from(scaled.y)) as f32;
    output.z = (f64::from(point.z) - distance * f64::from(scaled.z)) as f32;
}

fn donor_perpendicular_vector(output: &mut Vec3, source: Vec3) {
    let mut axis = 0;
    let mut minimum = 1.0;
    for (index, component) in [source.x, source.y, source.z].iter().enumerate() {
        if f64::from(component.abs()) < minimum {
            minimum = f64::from(component.abs());
            axis = index;
        }
    }
    let mut temporary = vec3(0.0, 0.0, 0.0);
    match axis {
        0 => temporary.x = 1.0,
        1 => temporary.y = 1.0,
        _ => temporary.z = 1.0,
    }
    donor_project_point_on_plane(output, temporary, source);
    donor_normalize_in_place(output);
}

/// Q1/Q2 donor point rotation around a normalized direction.
pub fn donor_rotate_point_around_vector(output: &mut Vec3, direction: Vec3, point: Vec3, degrees: f64) {
    let forward = vec3(direction.x, direction.y, direction.z);
    let mut radial = vec3(0.0, 0.0, 0.0);
    let mut vertical = vec3(0.0, 0.0, 0.0);
    donor_perpendicular_vector(&mut radial, direction);
    donor_cross(radial, forward, &mut vertical);
    let basis: Mat3 = [
        vec3(radial.x, vertical.x, forward.x),
        vec3(radial.y, vertical.y, forward.y),
        vec3(radial.z, vertical.z, forward.z),
    ];
    let radians = degrees * std::f64::consts::PI / 180.0;
    let cosine = radians.cos();
    let sine = radians.sin();
    let rotation: Mat3 = [
        vec3(cosine as f32, sine as f32, 0.0),
        vec3((-sine) as f32, cosine as f32, 0.0),
        vec3(0.0, 0.0, 1.0),
    ];
    let mut temporary = [vec3(0.0, 0.0, 0.0); 3];
    let mut result = [vec3(0.0, 0.0, 0.0); 3];
    donor_concat_rotations(basis, rotation, &mut temporary);
    donor_concat_rotations(temporary, [radial, vertical, forward], &mut result);
    output.x = donor_dot(result[0], point) as f32;
    output.y = donor_dot(result[1], point) as f32;
    output.z = donor_dot(result[2], point) as f32;
}

/// Quake software-renderer floor division, including negative numerators.
pub fn floor_div_mod(numerator: i32, denominator: i32) -> Result<(i32, i32), MathError> {
    if denominator <= 0 {
        return Err(MathError::BadDenominator);
    }
    if numerator >= 0 {
        let quotient = numerator / denominator;
        Ok((quotient, numerator - quotient * denominator))
    } else {
        let positive = numerator.checked_neg().unwrap_or(i32::MIN);
        let scaled = if positive == i32::MIN {
            2_147_483_648i64 / i64::from(denominator)
        } else {
            i64::from(positive) / i64::from(denominator)
        };
        let mut quotient = -scaled;
        let mut remainder = if positive == i32::MIN {
            2_147_483_648i64 - scaled * i64::from(denominator)
        } else {
            i64::from(positive) - scaled * i64::from(denominator)
        };
        if remainder != 0 {
            quotient -= 1;
            remainder = i64::from(denominator) - remainder;
        }
        Ok((quotient as i32, remainder as i32))
    }
}

/// Greatest common divisor.
#[must_use]
pub fn greatest_common_divisor(first: i32, second: i32) -> i32 {
    if first > second {
        if second == 0 {
            first
        } else {
            greatest_common_divisor(second, first % second)
        }
    } else if first == 0 {
        second
    } else {
        greatest_common_divisor(first, second % first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot3_keeps_donor_operation_order() {
        // 16777216 + 1 rounds back to 16777216 in binary32, so the ordered
        // sum cancels to zero on the binary32 path.
        let a = vec3(16_777_216.0, 1.0, -16_777_216.0);
        let b = vec3(1.0, 1.0, 1.0);
        assert_eq!(dot3(a, b), 0.0);
        assert_eq!(dot3(vec3(1.0, 2.0, 3.0), vec3(4.0, -5.0, 6.0)), 12.0);
    }

    #[test]
    fn normalize3_preserves_and_clears() {
        let zero = vec3(0.0, 0.0, 0.0);
        assert_eq!(normalize3(zero), zero);
        assert_eq!(normalize3_or_zero(zero), zero);
        let unit = normalize3(vec3(0.0, 3.0, 4.0));
        assert_eq!(unit, vec3(0.0, 0.6, 0.8));
    }

    #[test]
    fn integer_helpers_match_donor() {
        assert_eq!(clamp_short(40_000), 32_767);
        assert_eq!(clamp_short(-40_000), -32_768);
        assert_eq!(clamp_short(7), 7);
        assert_eq!(q_log2(0), Ok(0));
        assert_eq!(q_log2(1), Ok(0));
        assert_eq!(q_log2(16), Ok(4));
        assert_eq!(q_log2(-1), Err(MathError::NegativeLog));
        assert_eq!(floor_div_mod(7, 3), Ok((2, 1)));
        assert_eq!(floor_div_mod(-7, 3), Ok((-3, 2)));
        assert_eq!(floor_div_mod(-6, 3), Ok((-2, 0)));
        assert_eq!(floor_div_mod(1, 0), Err(MathError::BadDenominator));
        assert_eq!(greatest_common_divisor(54, 24), 6);
    }

    #[test]
    fn angle_helpers_match_donor() {
        assert_eq!(angle_mod(720.5), 91.0 * (360.0 / 65_536.0));
        assert!((angle_mod(-90.0) - 270.0).abs() < 0.01);
        assert!((angle_normalize180(270.0) + 90.0).abs() < 0.01);
        assert!((angle_delta(10.0, 350.0) - 20.0).abs() < 0.01);
        assert!((lerp_angle(350.0, 10.0, 0.5) - 360.0).abs() < 1e-9);
    }

    #[test]
    fn angle_vectors_round_trip() {
        let vectors = angle_vectors(vec3(0.0, 0.0, 0.0));
        assert_eq!(vectors.forward, vec3(1.0, 0.0, 0.0));
        let back = vector_to_angles(vec3(1.0, 0.0, 0.0));
        assert_eq!(back, vec3(-0.0, 0.0, 0.0));
        let up = vector_to_angles(vec3(0.0, 0.0, 1.0));
        assert_eq!(up, vec3(-90.0, 0.0, 0.0));
    }

    #[test]
    fn color_and_bounds_match_donor() {
        let mut output = vec3(0.0, 0.0, 0.0);
        assert_eq!(normalize_color(vec3(2.0, 4.0, 1.0), &mut output), 4.0);
        assert_eq!(output, vec3(0.5, 1.0, 0.25));
        let mut storage = [0x11, 0x22, 0x33, 0x44u8];
        let word = color_bytes3(1.0, 0.0, 0.5, &mut storage).unwrap();
        assert_eq!(storage, [255, 0, 127, 0x44]);
        assert_eq!(word, u32::from_le_bytes([255, 0, 127, 0x44]));
        assert!(color_bytes3(2.0, 0.0, 0.0, &mut storage).is_err());

        let bounds = add_point_to_bounds(
            add_point_to_bounds(empty_bounds(), vec3(1.0, 2.0, 3.0)),
            vec3(-4.0, 0.0, 0.0),
        );
        assert_eq!(bounds.min, vec3(-4.0, 0.0, 0.0));
        assert_eq!(bounds.max, vec3(1.0, 2.0, 3.0));
        assert!(radius_from_bounds(bounds) > 5.0);
    }

    #[test]
    fn planes_and_sides_match_donor() {
        let plane = plane_from_points(vec3(0.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0)).unwrap();
        assert_eq!(plane.normal, vec3(0.0, 0.0, 1.0));
        assert!(plane_from_points(vec3(0.0, 0.0, 0.0), vec3(1.0, 1.0, 1.0), vec3(2.0, 2.0, 2.0)).is_none());
        let bounds = Bounds {
            min: vec3(-1.0, -1.0, -1.0),
            max: vec3(1.0, 1.0, 1.0),
        };
        assert_eq!(box_on_plane_side(bounds, plane), 3);
        let above = Plane {
            normal: vec3(0.0, 0.0, 1.0),
            distance: 5.0,
        };
        assert_eq!(box_on_plane_side(bounds, above), 2);
    }

    #[test]
    fn matrices_match_donor() {
        let identity = identity_mat4();
        assert_eq!(multiply_mat4(identity, identity), identity);
        let moved = transform_vec4(identity, vec4(1.0, 2.0, 3.0, 1.0));
        assert_eq!(moved, vec4(1.0, 2.0, 3.0, 1.0));
        let view = look_at_mat4(vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0));
        assert_eq!(
            transform_vec4(view, vec4(0.0, 0.0, 0.0, 1.0)),
            vec4(0.0, 0.0, -1.0, 1.0)
        );
        let projection = perspective_mat4(90.0, 1.0, 1.0, 10.0);
        assert!((f64::from(projection[0]) - 1.0).abs() < 1e-6);
        assert_eq!(projection[11], -1.0);
    }

    #[test]
    fn donor_paths_match_donor() {
        let mut forward = vec3(0.0, 0.0, 0.0);
        donor_angle_vectors(vec3(0.0, 90.0, 0.0), Some(&mut forward), None, None);
        assert!(forward.x.abs() < 1e-6);
        assert_eq!(forward.y, 1.0);
        assert_eq!(forward.z, -0.0);
        let mut output = vec3(0.0, 0.0, 0.0);
        donor_rotate_point_around_vector(&mut output, vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0), 90.0);
        assert!(output.x.abs() < 1e-5);
        assert!((output.y - 1.0).abs() < 1e-5);
        let rotated = rotate_point_around_vector(vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0), 90.0);
        assert!(rotated.x.abs() < 1e-5);
        assert!((rotated.y - 1.0).abs() < 1e-5);
        assert_eq!(
            project_point_on_plane(vec3(1.0, 1.0, 1.0), vec3(0.0, 0.0, 1.0)),
            vec3(1.0, 1.0, 0.0)
        );
    }

    #[test]
    fn js_min_max_match_donor() {
        assert_eq!(js_min(1.0, 2.0), 1.0);
        assert_eq!(js_max(1.0, 2.0), 2.0);
        assert!(js_min(f64::NAN, 1.0).is_nan());
        assert!(js_min(1.0, f64::NAN).is_nan());
        assert!(js_max(f64::NAN, 1.0).is_nan());
        assert_eq!(js_min(0.0, -0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(js_min(-0.0, 0.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(js_max(0.0, -0.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(js_max(-0.0, 0.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(js_min_f32(0.0, -0.0).to_bits(), (-0.0f32).to_bits());
        assert_eq!(js_max_f32(-0.0, 0.0).to_bits(), 0.0f32.to_bits());
    }
}
