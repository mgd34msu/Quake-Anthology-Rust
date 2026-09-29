//! Quake II player dimensions and posture heights.
//!
//! Donor provenance: `src/movement/q2/dimensions.ts`.

use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::NumericOps;

use super::super::types::MovementError;

/// Source player bounds.
pub const Q2_PLAYER_BOUNDS: Bounds = Bounds {
    min: Vec3 { x: -16.0, y: -16.0, z: -24.0 },
    max: Vec3 { x: 16.0, y: 16.0, z: 32.0 },
};

/// Source posture heights relative to the selected character's standing body.
#[must_use]
pub fn character_height(bounds: &Bounds, source_height: f64, numeric: &NumericOps) -> f64 {
    numeric.add(
        f64::from(bounds.min.z),
        numeric.mul(
            numeric.add(source_height, 24.0) / 56.0,
            numeric.sub(f64::from(bounds.max.z), f64::from(bounds.min.z)),
        ),
    )
}

/// Standing bounds from explicit minima/maxima.
#[must_use]
pub fn standing_bounds() -> Bounds {
    Q2_PLAYER_BOUNDS
}

/// Body-bounds acceptance gate with a one-shot clearance probe. Same rule as
/// the shared hull gate; the `FnOnce` probe keeps the pmove borrow.
pub fn accept_body_bounds(
    previous: &Bounds,
    requested: &Bounds,
    clear: impl FnOnce(&Bounds) -> bool,
) -> Bounds {
    let expands = requested.min.x < previous.min.x
        || requested.min.y < previous.min.y
        || requested.min.z < previous.min.z
        || requested.max.x > previous.max.x
        || requested.max.y > previous.max.y
        || requested.max.z > previous.max.z;
    if expands && !clear(requested) {
        *previous
    } else {
        *requested
    }
}

/// Validate a Q2 usercmd duration byte.
pub fn command_duration(milliseconds: i32) -> Result<(), MovementError> {
    if milliseconds < 0 || milliseconds > 255 {
        return Err(MovementError::Range(
            "Quake II usercmd duration must fit its source byte",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;
    use qa_core::numeric::{NumericOps, Q2_DONOR_PROFILE};

    #[test]
    fn standard_heights_match_source() {
        let numeric = NumericOps::select(Q2_DONOR_PROFILE).unwrap();
        assert_eq!(character_height(&Q2_PLAYER_BOUNDS, 22.0, &numeric), 22.0);
        assert_eq!(character_height(&Q2_PLAYER_BOUNDS, -24.0, &numeric), -24.0);
        assert_eq!(character_height(&Q2_PLAYER_BOUNDS, 32.0, &numeric), 32.0);
        assert_eq!(character_height(&Q2_PLAYER_BOUNDS, 4.0, &numeric), 4.0);
        assert_eq!(character_height(&Q2_PLAYER_BOUNDS, -2.0, &numeric), -2.0);
    }

    #[test]
    fn scaled_characters_keep_proportions() {
        let numeric = NumericOps::select(Q2_DONOR_PROFILE).unwrap();
        let tall = Bounds {
            min: vec3(-16.0, -16.0, -48.0),
            max: vec3(16.0, 16.0, 64.0),
        };
        assert_eq!(character_height(&tall, -24.0, &numeric), -48.0);
        assert_eq!(character_height(&tall, 32.0, &numeric), 64.0);
        assert_eq!(standing_bounds(), Q2_PLAYER_BOUNDS);
    }

    #[test]
    fn acceptance_gate_matches_shared_rule() {
        let previous = Q2_PLAYER_BOUNDS;
        let requested = Bounds {
            min: vec3(-16.0, -16.0, -24.0),
            max: vec3(16.0, 16.0, 48.0),
        };
        assert_eq!(accept_body_bounds(&previous, &requested, |_| false), previous);
        assert_eq!(accept_body_bounds(&previous, &requested, |_| true), requested);
    }

    #[test]
    fn durations_fit_the_source_byte() {
        assert!(command_duration(0).is_ok());
        assert!(command_duration(255).is_ok());
        assert!(command_duration(256).is_err());
        assert!(command_duration(-1).is_err());
    }
}
