//! Quake II shared vector math.
//!
//! Donor provenance: `src/movement/q2/math.ts` (from Quake II `q_shared.c`
//! and rerelease `q_vec3.h`).

use qa_core::numeric::{Arithmetic, NumericOps};

use super::types::{SrcVec3, SrcVec4, AXES};

/// Math edition selecting the angle-radians path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2MathEdition {
    /// Classic `Math.PI * 2 / 360`.
    Classic,
    /// Rerelease numeric-operator path.
    Rerelease,
}

/// Profile-aware Q2 vector math, mirroring donor `createMovementMath`.
#[derive(Debug, Clone, Copy)]
pub struct Q2Math {
    /// Numeric operations.
    pub n: NumericOps,
    /// Math edition.
    pub edition: Q2MathEdition,
}

impl Q2Math {
    /// Bind operations for an edition.
    #[must_use]
    pub fn new(n: NumericOps, edition: Q2MathEdition) -> Self {
        Self { n, edition }
    }

    fn scalar(&self, value: f64) -> f64 {
        if matches!(self.n.profile.arithmetic, Arithmetic::DonorBinary64(_)) {
            value
        } else {
            f64::from(self.n.store(value))
        }
    }

    /// Store a source vector.
    #[must_use]
    pub fn vec3(&self, x: f64, y: f64, z: f64) -> SrcVec3 {
        [
            f64::from(self.n.store(x)),
            f64::from(self.n.store(y)),
            f64::from(self.n.store(z)),
        ]
    }

    /// Dot product.
    #[must_use]
    pub fn dot(&self, a: SrcVec3, b: SrcVec3) -> f64 {
        self.n.add(
            self.n.add(self.n.mul(a[0], b[0]), self.n.mul(a[1], b[1])),
            self.n.mul(a[2], b[2]),
        )
    }

    /// Copy a vector.
    pub fn copy(&self, a: SrcVec3, b: &mut SrcVec3) {
        for i in AXES {
            b[i] = f64::from(self.n.store(a[i]));
        }
    }

    /// Clear a vector.
    pub fn clear(&self, a: &mut SrcVec3) {
        a[0] = 0.0;
        a[1] = 0.0;
        a[2] = 0.0;
    }

    /// Multiply-add into `out`.
    pub fn ma(&self, a: SrcVec3, scale: f64, b: SrcVec3, out: &mut SrcVec3) {
        for i in AXES {
            out[i] = f64::from(self.n.store(self.n.add(a[i], self.n.mul(scale, b[i]))));
        }
    }

    /// Scale into `out`.
    pub fn scale_into(&self, a: SrcVec3, scale: f64, out: &mut SrcVec3) {
        for i in AXES {
            out[i] = f64::from(self.n.store(self.n.mul(a[i], scale)));
        }
    }

    /// Vector length.
    #[must_use]
    pub fn length(&self, a: SrcVec3) -> f64 {
        self.n.sqrt(self.dot(a, a))
    }

    /// Normalize in place, returning the length.
    pub fn normalize(&self, a: &mut SrcVec3) -> f64 {
        let length = self.length(*a);
        if length != 0.0 {
            let scale = self.n.div(1.0, length);
            let copy = *a;
            self.scale_into(copy, scale, a);
        }
        length
    }

    /// Cross product into `out`.
    pub fn cross_into(&self, a: SrcVec3, b: SrcVec3, out: &mut SrcVec3) {
        let result = self.vec3(
            self.n.sub(self.n.mul(a[1], b[2]), self.n.mul(a[2], b[1])),
            self.n.sub(self.n.mul(a[2], b[0]), self.n.mul(a[0], b[2])),
            self.n.sub(self.n.mul(a[0], b[1]), self.n.mul(a[1], b[0])),
        );
        self.copy(result, out);
    }

    /// Angle vectors into forward/right/up.
    pub fn angle_vectors(&self, angles: SrcVec3, forward: &mut SrcVec3, right: &mut SrcVec3, up: &mut SrcVec3) {
        let radians = match self.edition {
            Q2MathEdition::Classic => std::f64::consts::PI * 2.0 / 360.0,
            Q2MathEdition::Rerelease => self
                .n
                .div(self.n.mul(f64::from(self.n.store(std::f64::consts::PI)), 2.0), 360.0),
        };
        let yaw = self.scalar(angles[1] * radians);
        let pitch = self.scalar(angles[0] * radians);
        let roll = self.scalar(angles[2] * radians);
        let sy = self.scalar(yaw.sin());
        let cy = self.scalar(yaw.cos());
        let sp = self.scalar(pitch.sin());
        let cp = self.scalar(pitch.cos());
        let sr = self.scalar(roll.sin());
        let cr = self.scalar(roll.cos());
        forward[0] = f64::from(self.n.store(self.n.mul(cp, cy)));
        forward[1] = f64::from(self.n.store(self.n.mul(cp, sy)));
        forward[2] = f64::from(self.n.store(-sp));
        right[0] = f64::from(
            self.n
                .store(self.n.add(self.n.mul(self.n.mul(-sr, sp), cy), self.n.mul(cr, sy))),
        );
        right[1] = f64::from(
            self.n
                .store(self.n.sub(self.n.mul(self.n.mul(-sr, sp), sy), self.n.mul(cr, cy))),
        );
        right[2] = f64::from(self.n.store(self.n.mul(-sr, cp)));
        up[0] = f64::from(
            self.n
                .store(self.n.add(self.n.mul(self.n.mul(cr, sp), cy), self.n.mul(sr, sy))),
        );
        up[1] = f64::from(
            self.n
                .store(self.n.sub(self.n.mul(self.n.mul(cr, sp), sy), self.n.mul(sr, cy))),
        );
        up[2] = f64::from(self.n.store(self.n.mul(cr, cp)));
    }

    /// Add two vectors.
    #[must_use]
    pub fn add(&self, a: SrcVec3, b: SrcVec3) -> SrcVec3 {
        self.vec3(self.n.add(a[0], b[0]), self.n.add(a[1], b[1]), self.n.add(a[2], b[2]))
    }

    /// Subtract two vectors.
    #[must_use]
    pub fn sub(&self, a: SrcVec3, b: SrcVec3) -> SrcVec3 {
        self.vec3(self.n.sub(a[0], b[0]), self.n.sub(a[1], b[1]), self.n.sub(a[2], b[2]))
    }

    /// Scale a vector.
    #[must_use]
    pub fn muls(&self, a: SrcVec3, scale: f64) -> SrcVec3 {
        self.vec3(
            self.n.mul(a[0], scale),
            self.n.mul(a[1], scale),
            self.n.mul(a[2], scale),
        )
    }

    /// Scale in place.
    pub fn mul_eq(&self, a: &mut SrcVec3, scale: f64) {
        let copy = *a;
        self.scale_into(copy, scale, a);
    }

    /// Add-assign.
    pub fn add_eq(&self, a: &mut SrcVec3, b: SrcVec3) {
        for i in AXES {
            a[i] = f64::from(self.n.store(self.n.add(a[i], b[i])));
        }
    }

    /// Cross product.
    #[must_use]
    pub fn cross(&self, a: SrcVec3, b: SrcVec3) -> SrcVec3 {
        let mut out = self.vec3(0.0, 0.0, 0.0);
        self.cross_into(a, b, &mut out);
        out
    }

    /// Squared length.
    #[must_use]
    pub fn length_squared(&self, a: SrcVec3) -> f64 {
        self.dot(a, a)
    }

    /// Slide-clip a velocity against a plane normal with the 0.1 dead zone.
    #[must_use]
    pub fn slide_clip_velocity(&self, a: SrcVec3, normal: SrcVec3, overbounce: f64) -> SrcVec3 {
        let backoff = self.n.mul(self.dot(a, normal), self.scalar(overbounce));
        let mut out = self.vec3(0.0, 0.0, 0.0);
        for i in AXES {
            out[i] = self.n.sub(a[i], self.n.mul(normal[i], backoff));
            if out[i] > -self.scalar(0.1) && out[i] < self.scalar(0.1) {
                out[i] = 0.0;
            }
        }
        out
    }

    /// Screen-blend accumulation (`G_AddBlend`).
    pub fn add_blend(&self, r: f64, g: f64, b: f64, a: f64, blend: &mut SrcVec4) {
        if a <= 0.0 {
            return;
        }
        let alpha = self.n.add(blend[3], self.n.mul(self.n.sub(1.0, blend[3]), a));
        let fraction = self.n.div(blend[3], alpha);
        blend[0] = self
            .n
            .add(self.n.mul(blend[0], fraction), self.n.mul(r, self.n.sub(1.0, fraction)));
        blend[1] = self
            .n
            .add(self.n.mul(blend[1], fraction), self.n.mul(g, self.n.sub(1.0, fraction)));
        blend[2] = self
            .n
            .add(self.n.mul(blend[2], fraction), self.n.mul(b, self.n.sub(1.0, fraction)));
        blend[3] = alpha;
    }

    /// Short-to-angle conversion.
    #[must_use]
    pub fn short2angle(&self, value: f64) -> f64 {
        self.n.mul(value, 360.0 / 65536.0)
    }

    /// Clamp a value.
    #[must_use]
    pub fn clamp(&self, value: f64, min: f64, max: f64) -> f64 {
        value.clamp(min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::numeric::{NumericOps, Q2_DONOR_PROFILE};

    fn math() -> Q2Math {
        Q2Math::new(NumericOps::select(Q2_DONOR_PROFILE).unwrap(), Q2MathEdition::Classic)
    }

    #[test]
    fn vector_ops_follow_source_order() {
        let math = math();
        assert_eq!(math.add([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]), [5.0, 7.0, 9.0]);
        assert_eq!(math.sub([4.0, 5.0, 6.0], [1.0, 2.0, 3.0]), [3.0, 3.0, 3.0]);
        assert_eq!(math.muls([1.0, 2.0, 3.0], 2.0), [2.0, 4.0, 6.0]);
        assert_eq!(math.dot([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]), 32.0);
        assert_eq!(math.length([3.0, 4.0, 0.0]), 5.0);
        assert_eq!(math.cross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
    }

    #[test]
    fn normalize_reports_length_and_handles_zero() {
        let math = math();
        let mut value = [3.0, 4.0, 0.0];
        assert_eq!(math.normalize(&mut value), 5.0);
        let mut zero = [0.0, 0.0, 0.0];
        assert_eq!(math.normalize(&mut zero), 0.0);
        assert_eq!(zero, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn slide_clip_applies_dead_zone() {
        let math = math();
        let clipped = math.slide_clip_velocity([0.05, 0.2, 1.0], [0.0, 0.0, 1.0], 1.0);
        assert_eq!(clipped, [0.0, 0.2, 0.0]);
    }

    #[test]
    fn angles_and_blend_match_source() {
        let math = math();
        let (mut forward, mut right, mut up) = ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
        math.angle_vectors([0.0, 90.0, 0.0], &mut forward, &mut right, &mut up);
        assert!((forward[0]).abs() < 1e-6);
        assert!((forward[1] - 1.0).abs() < 1e-6);
        let mut blend = [0.0, 0.0, 0.0, 0.0];
        math.add_blend(1.0, 0.0, 0.0, 0.5, &mut blend);
        assert_eq!(blend[3], 0.5);
        assert_eq!(math.short2angle(8192.0), 45.0);
        assert_eq!(math.clamp(500.0, -200.0, 200.0), 200.0);
    }

    #[test]
    fn rerelease_edition_matches_classic_angles() {
        let math = Q2Math::new(NumericOps::select(Q2_DONOR_PROFILE).unwrap(), Q2MathEdition::Rerelease);
        let (mut forward, mut right, mut up) = ([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]);
        math.angle_vectors([0.0, 0.0, 0.0], &mut forward, &mut right, &mut up);
        assert!((forward[0] - 1.0).abs() < 1e-6);
    }
}
