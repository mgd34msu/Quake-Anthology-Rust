//! Quake III source postures.
//!
//! Donor provenance: `src/movement/q3/postures.ts`.

use qa_core::math::{Bounds, Vec3};

use super::super::types::FixedMovementPose;
use super::types::Q3Postures;

/// Source standing bounds for a selected Q3 collision body.
pub const Q3_SOURCE_STANDING_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -15.0,
        y: -15.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 15.0,
        y: 15.0,
        z: 32.0,
    },
};

/// Source posture dimensions.
pub const Q3_SOURCE_POSTURES: Q3Postures = Q3Postures {
    standing_view_height: 26.0,
    crouched: super::types::Q3Posture {
        bounds: Bounds {
            min: Vec3 {
                x: -15.0,
                y: -15.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 15.0,
                y: 15.0,
                z: 16.0,
            },
        },
        view_height: 12.0,
    },
    dead: super::types::Q3Posture {
        bounds: Bounds {
            min: Vec3 {
                x: -15.0,
                y: -15.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 15.0,
                y: 15.0,
                z: -8.0,
            },
        },
        view_height: -16.0,
    },
    invulnerability_expanded: Bounds {
        min: Vec3 {
            x: -42.0,
            y: -42.0,
            z: -42.0,
        },
        max: Vec3 {
            x: 42.0,
            y: 42.0,
            z: 42.0,
        },
    },
};

/// `PM_CheckDuck` uses the expanded sphere only after the original client
/// overlap test admits it.
#[must_use]
pub fn q3_invulnerability_pose(expanded: bool, postures: &Q3Postures) -> FixedMovementPose {
    FixedMovementPose {
        crouched: true,
        bounds: if expanded {
            postures.invulnerability_expanded
        } else {
            postures.crouched.bounds
        },
        view_height: postures.crouched.view_height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_dimensions_match_bg_pmove() {
        assert_eq!(Q3_SOURCE_STANDING_BOUNDS.max.z, 32.0);
        assert_eq!(Q3_SOURCE_POSTURES.standing_view_height, 26.0);
        assert_eq!(Q3_SOURCE_POSTURES.crouched.view_height, 12.0);
        assert_eq!(Q3_SOURCE_POSTURES.dead.view_height, -16.0);
    }

    #[test]
    fn invulnerability_pose_selects_expansion() {
        let tight = q3_invulnerability_pose(false, &Q3_SOURCE_POSTURES);
        assert!(tight.crouched);
        assert_eq!(tight.bounds, Q3_SOURCE_POSTURES.crouched.bounds);
        assert_eq!(tight.view_height, 12.0);
        let expanded = q3_invulnerability_pose(true, &Q3_SOURCE_POSTURES);
        assert_eq!(expanded.bounds, Q3_SOURCE_POSTURES.invulnerability_expanded);
        assert_eq!(expanded.view_height, 12.0);
    }
}
