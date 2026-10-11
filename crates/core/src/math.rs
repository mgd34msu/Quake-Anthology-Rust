use crate::primitives::Vec3;
use std::ops::{Add, Div, Mul, Neg, Sub};

mod directions;
pub use directions::DIRECTIONS;

/// Native coarse normal encoding, preserving table order and strict ties.
pub fn direction_to_byte(direction: Vec3) -> u8 {
    let mut best = 0;
    let mut best_dot = 0.0;
    for (index, normal) in DIRECTIONS.iter().enumerate() {
        let dot = direction.dot(*normal);
        if dot > best_dot {
            best_dot = dot;
            best = index as u8;
        }
    }
    best
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i] + rhs.0[i]))
    }
}
impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i] - rhs.0[i]))
    }
}
impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        Self(self.0.map(|v| v * rhs))
    }
}
impl Div<f32> for Vec3 {
    type Output = Self;
    fn div(self, rhs: f32) -> Self {
        Self(self.0.map(|v| v / rhs))
    }
}
impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self(self.0.map(|v| -v))
    }
}

pub fn difference(a: Vec3, b: Vec3) -> Vec3 {
    a - b
}
pub fn length(v: Vec3) -> f32 {
    v.dot(v).sqrt()
}
pub fn normalize(v: &mut Vec3) -> f32 {
    let len = length(*v);
    if len != 0.0 {
        *v = *v * (1.0 / len);
    }
    len
}
pub fn normalized(mut v: Vec3) -> Vec3 {
    normalize(&mut v);
    v
}
/// Output normalization clears zero components, as VectorNormalize2 does.
pub fn normalized_or_zero(mut v: Vec3) -> Vec3 {
    if normalize(&mut v) == 0.0 {
        Vec3::default()
    } else {
        v
    }
}

/// Native reciprocal-square-root estimate, shared by stage evaluation and guests.
pub fn normalized_fast(v: Vec3) -> Vec3 {
    let x = v.dot(v);
    let y = f32::from_bits(0x5f3759df - (x.to_bits() >> 1));
    v * (y * (1.5 - (x * 0.5 * y * y)))
}

pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3([
        a.0[1] * b.0[2] - a.0[2] * b.0[1],
        a.0[2] * b.0[0] - a.0[0] * b.0[2],
        a.0[0] * b.0[1] - a.0[1] * b.0[0],
    ])
}

/// Preserve each caller's native arithmetic order before integer narrowing.
#[derive(Clone, Copy)]
pub enum AngleShortForm {
    MultiplyDivide,
    Factored,
    Double,
}

pub fn angle_to_short(angle: f32, form: AngleShortForm) -> i32 {
    match form {
        AngleShortForm::MultiplyDivide => (angle * 65536.0 / 360.0) as i32,
        AngleShortForm::Factored => (angle * (65536.0 / 360.0)) as i32,
        AngleShortForm::Double => (f64::from(angle) * (65536.0 / 360.0)) as i32,
    }
}

pub fn short_to_angle(angle: i32) -> f32 {
    angle as f32 * (360.0 / 65536.0)
}

#[derive(Clone, Copy)]
pub enum AngleByteForm {
    Float,
    Integral,
}

/// Byte narrowing is performed by the native field writer, after these operations.
pub fn angle_to_byte(angle: f32, form: AngleByteForm) -> i32 {
    match form {
        AngleByteForm::Float => (angle * 256.0 / 360.0) as i32,
        AngleByteForm::Integral => (angle as i32).wrapping_mul(256) / 360,
    }
}

pub fn byte_to_angle(angle: i32) -> f32 {
    angle as f32 * (360.0 / 256.0)
}

/// Native Q2 signed 16-bit coordinates and velocities in eighth units.
pub fn narrow_eighth(value: f32) -> f32 {
    f32::from((value * 8.0) as i32 as i16) * 0.125
}

/// Origin first, then each axis in order; do not regroup the products.
pub fn transform_point(origin: Vec3, axes: [Vec3; 3], local: Vec3) -> Vec3 {
    Vec3(std::array::from_fn(|i| {
        origin.0[i]
            + axes[0].0[i] * local.0[0]
            + axes[1].0[i] * local.0[1]
            + axes[2].0[i] * local.0[2]
    }))
}

/// A quaternion rotation, shared by skeletal file conversion and deformation.
pub fn rotate_quaternion(q: [f32; 4], p: Vec3) -> Vec3 {
    let dot = q[0] * p.0[0] + q[1] * p.0[1] + q[2] * p.0[2];
    let scalar = q[3] * q[3] - q[0] * q[0] - q[1] * q[1] - q[2] * q[2];
    Vec3([
        scalar * p.0[0] + 2.0 * (q[0] * dot + q[3] * (q[1] * p.0[2] - q[2] * p.0[1])),
        scalar * p.0[1] + 2.0 * (q[1] * dot + q[3] * (q[2] * p.0[0] - q[0] * p.0[2])),
        scalar * p.0[2] + 2.0 * (q[2] * dot + q[3] * (q[0] * p.0[1] - q[1] * p.0[0])),
    ])
}

#[derive(Clone, Copy, Debug)]
pub struct AngleBasis {
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
}

/// mathlib.c and Q2 q_shared.c multiply by an unsuffixed M_PI expression,
/// storing the result into a float before calling sin(double)/cos(double).
pub fn radians_from_degrees(angles: Vec3) -> Vec3 {
    Vec3(
        angles
            .0
            .map(|v| (f64::from(v) * (std::f64::consts::TAU / 360.0)) as f32),
    )
}

/// Q3's q_shared.h defines M_PI with an f suffix. Convert at that boundary;
/// the basis below remains the single engine implementation.
pub fn radians_from_degrees_f32(angles: Vec3) -> Vec3 {
    angles * (std::f32::consts::TAU / 360.0)
}

pub fn angle_vectors(angles: Vec3) -> AngleBasis {
    angle_vectors_radians(radians_from_degrees(angles))
}

pub fn angle_vectors_radians(angles: Vec3) -> AngleBasis {
    let sy = f64::from(angles.0[1]).sin() as f32;
    let cy = f64::from(angles.0[1]).cos() as f32;
    let sp = f64::from(angles.0[0]).sin() as f32;
    let cp = f64::from(angles.0[0]).cos() as f32;
    let sr = f64::from(angles.0[2]).sin() as f32;
    let cr = f64::from(angles.0[2]).cos() as f32;
    AngleBasis {
        forward: Vec3([cp * cy, cp * sy, -sp]),
        right: Vec3([
            -sr * sp * cy + -cr * -sy,
            -sr * sp * sy + -cr * cy,
            -sr * cp,
        ]),
        up: Vec3([cr * sp * cy + -sr * -sy, cr * sp * sy + -sr * cy, cr * cp]),
    }
}

pub fn anglemod(angle: f32) -> f32 {
    short_to_angle(angle_to_short(angle, AngleShortForm::Double) & 65535)
}
