//! Quake II view-angle clamping.
//!
//! Donor provenance: `src/movement/q2/view.ts` (Quake II `PM_ClampAngles`).

use qa_core::numeric::NumericOps;

use super::types::{pm_flags, SrcVec3, AXES};

fn clamp_pitch(view: &mut SrcVec3, numeric: &NumericOps) {
    if view[0] > 89.0 && view[0] < 180.0 {
        view[0] = f64::from(numeric.store(89.0));
    } else if view[0] < 271.0 && view[0] >= 180.0 {
        view[0] = f64::from(numeric.store(271.0));
    }
}

/// Classic short-encoded view angles.
pub fn classic_view_angles(view: &mut SrcVec3, angles: [i32; 3], delta: [i32; 3], flags: i32, numeric: &NumericOps) {
    if flags & pm_flags::TIME_TELEPORT != 0 {
        view[1] =
            f64::from(numeric.store(numeric.mul(numeric.add(angles[1] as f64, delta[1] as f64), 360.0 / 65536.0)));
        view[0] = f64::from(numeric.store(0.0));
        view[2] = f64::from(numeric.store(0.0));
    } else {
        for axis in AXES {
            let word = ((numeric.add(angles[axis] as f64, delta[axis] as f64) as i32) << 16) >> 16;
            view[axis] = f64::from(numeric.store(numeric.mul(word as f64, 360.0 / 65536.0)));
        }
        clamp_pitch(view, numeric);
    }
}

/// Rerelease float view angles.
pub fn rerelease_view_angles(view: &mut SrcVec3, angles: SrcVec3, delta: SrcVec3, flags: i32, numeric: &NumericOps) {
    if flags & pm_flags::TIME_TELEPORT != 0 {
        view[1] = f64::from(numeric.store(numeric.add(angles[1], delta[1])));
        view[0] = f64::from(numeric.store(0.0));
        view[2] = f64::from(numeric.store(0.0));
    } else {
        for axis in AXES {
            view[axis] = f64::from(numeric.store(numeric.add(angles[axis], delta[axis])));
        }
        clamp_pitch(view, numeric);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::numeric::{NumericOps, Q2_DONOR_PROFILE};

    fn ops() -> NumericOps {
        NumericOps::select(Q2_DONOR_PROFILE).unwrap()
    }

    #[test]
    fn classic_converts_short_words() {
        let numeric = ops();
        let mut view = [0.0, 0.0, 0.0];
        classic_view_angles(&mut view, [0, 8192, 0], [0, 0, 0], 0, &numeric);
        assert_eq!(view[1], 45.0);
    }

    #[test]
    fn teleport_snaps_pitch_and_roll() {
        let numeric = ops();
        let mut view = [10.0, 20.0, 30.0];
        classic_view_angles(&mut view, [0, 8192, 0], [0, 0, 0], pm_flags::TIME_TELEPORT, &numeric);
        assert_eq!(view, [0.0, 45.0, 0.0]);
        let mut view = [10.0, 20.0, 30.0];
        rerelease_view_angles(
            &mut view,
            [10.0, 20.0, 30.0],
            [0.0, 0.0, 0.0],
            pm_flags::TIME_TELEPORT,
            &numeric,
        );
        assert_eq!(view, [0.0, 20.0, 0.0]);
    }

    #[test]
    fn pitch_clamps_both_directions() {
        let numeric = ops();
        let mut view = [100.0, 0.0, 0.0];
        rerelease_view_angles(&mut view, [100.0, 0.0, 0.0], [0.0, 0.0, 0.0], 0, &numeric);
        assert_eq!(view[0], 89.0);
        let mut view = [200.0, 0.0, 0.0];
        rerelease_view_angles(&mut view, [200.0, 0.0, 0.0], [0.0, 0.0, 0.0], 0, &numeric);
        assert_eq!(view[0], 271.0);
    }
}
