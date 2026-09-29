//! Geometry obstruction gain.
//!
//! Donor provenance: `src/audio/geometry.ts`
//! (`geometryTransmission`).

use qa_core::math::Vec3;

/// Geometry trace hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioGeometryHit {
    /// Hit fraction.
    pub fraction: f64,
    /// Started inside solid.
    pub start_solid: bool,
    /// Entirely inside solid.
    pub all_solid: bool,
}

/// Geometry trace query.
pub type AudioGeometryTrace = Box<dyn FnMut(Vec3, Vec3) -> AudioGeometryHit>;

/// Obstruction gain: clear paths keep source gain; a solid boundary
/// transmits half amplitude, halving again per 64 units of thickness.
pub fn geometry_transmission(start: Vec3, end: Vec3, trace: &mut AudioGeometryTrace) -> f64 {
    let distance = (f64::from(end.x) - f64::from(start.x))
        .hypot(f64::from(end.y) - f64::from(start.y))
        .hypot(f64::from(end.z) - f64::from(start.z));
    if distance == 0.0 {
        return 1.0;
    }
    let forward = trace(start, end);
    if forward.fraction == 1.0 && !forward.start_solid && !forward.all_solid {
        return 1.0;
    }
    let reverse = trace(end, start);
    let thickness = if forward.all_solid || reverse.all_solid {
        distance
    } else {
        0.0f64.max(
            distance
                * (1.0
                    - if forward.start_solid { 0.0 } else { forward.fraction }
                    - if reverse.start_solid { 0.0 } else { reverse.fraction }),
        )
    };
    0.5f64.powf(1.0 + thickness / 64.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;

    #[test]
    fn clear_paths_keep_gain() {
        let mut clear: AudioGeometryTrace = Box::new(|_, _| AudioGeometryHit {
            fraction: 1.0,
            start_solid: false,
            all_solid: false,
        });
        assert_eq!(
            geometry_transmission(vec3(0.0, 0.0, 0.0), vec3(100.0, 0.0, 0.0), &mut clear),
            1.0
        );
        assert_eq!(
            geometry_transmission(vec3(1.0, 1.0, 1.0), vec3(1.0, 1.0, 1.0), &mut clear),
            1.0
        );
        let mut solid: AudioGeometryTrace = Box::new(|_, _| AudioGeometryHit {
            fraction: 0.0,
            start_solid: true,
            all_solid: true,
        });
        let gain = geometry_transmission(vec3(0.0, 0.0, 0.0), vec3(64.0, 0.0, 0.0), &mut solid);
        assert!((gain - 0.25).abs() < 1e-12, "{gain}");
    }
}
