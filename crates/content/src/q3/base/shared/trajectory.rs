//! Quake III base/shared: trajectory.
//!
//! Donor provenance: `src/content/q3/base/shared/trajectory.ts`.

use qa_core::math::{add3, scale3, vec3, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::records::Q3BaseError;
use crate::q3::base::shared::definitions::*;

// ---------------------------------------------------------------------------
// shared/trajectory.ts
// ---------------------------------------------------------------------------

/// Trajectory type (`TrajectoryType`, `trType_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum TrajectoryType {
    /// Stationary.
    TrStationary = 0,
    /// Interpolated.
    TrInterpolate = 1,
    /// Linear.
    TrLinear = 2,
    /// Linear with stop.
    TrLinearStop = 3,
    /// Sine.
    TrSine = 4,
    /// Gravity.
    TrGravity = 5,
}

impl TrajectoryType {
    /// Convert a raw integer tag, if it names a trajectory type.
    ///
    /// Unknown tags are rejected here; the donor reports them from
    /// `BG_EvaluateTrajectory`/`BG_EvaluateTrajectoryDelta` with a drop
    /// error, which [`Q3BaseError::Drop`] preserves at this boundary.
    pub fn from_i32(value: i32) -> Result<Self, Q3BaseError> {
        match value {
            0 => Ok(Self::TrStationary),
            1 => Ok(Self::TrInterpolate),
            2 => Ok(Self::TrLinear),
            3 => Ok(Self::TrLinearStop),
            4 => Ok(Self::TrSine),
            5 => Ok(Self::TrGravity),
            _ => Err(Q3BaseError::Drop(format!(
                "BG_EvaluateTrajectory: unknown trType: {value}"
            ))),
        }
    }
}

/// Trajectory record (`Trajectory`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trajectory {
    /// Trajectory type.
    pub trajectory_type: TrajectoryType,
    /// Start time in milliseconds.
    pub time: i32,
    /// Duration in milliseconds.
    pub duration: i32,
    /// Base position.
    pub base: Vec3,
    /// Delta (velocity or amplitude).
    pub delta: Vec3,
}

impl Default for Trajectory {
    fn default() -> Self {
        Self::zero(TrajectoryType::TrStationary)
    }
}

impl Trajectory {
    /// Zero trajectory of one type.
    #[must_use]
    pub fn zero(trajectory_type: TrajectoryType) -> Self {
        Self {
            trajectory_type,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 0.0),
        }
    }
}

pub(crate) fn trajectory_seconds(milliseconds: i32) -> f32 {
    (milliseconds as f32) * 0.001
}

pub(crate) fn trajectory_periodic_radians(tr: &Trajectory, at_time: i32) -> f32 {
    let fraction = at_time.wrapping_sub(tr.time) as f32 / (tr.duration as f32);
    (fraction * std::f32::consts::PI) * 2.0
}

/// Evaluate a trajectory at a millisecond time (`BG_EvaluateTrajectory`).
///
/// Sine phases use `f32` trigonometry, matching the C source's `sinf`.
#[must_use]
pub fn evaluate_trajectory(tr: &Trajectory, at_time: i32) -> Vec3 {
    match tr.trajectory_type {
        TrajectoryType::TrStationary | TrajectoryType::TrInterpolate => tr.base,
        TrajectoryType::TrLinear => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(tr.time));
            add3(tr.base, scale3(tr.delta, delta_time))
        }
        TrajectoryType::TrSine => {
            let phase = trajectory_periodic_radians(tr, at_time).sin();
            add3(tr.base, scale3(tr.delta, phase))
        }
        TrajectoryType::TrLinearStop => {
            let end = tr.time.wrapping_add(tr.duration);
            let time = if at_time > end { end } else { at_time };
            let delta_time = trajectory_seconds(time.wrapping_sub(tr.time)).max(0.0);
            add3(tr.base, scale3(tr.delta, delta_time))
        }
        TrajectoryType::TrGravity => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(tr.time));
            let result = add3(tr.base, scale3(tr.delta, delta_time));
            let fall = (0.5 * (DEFAULT_GRAVITY as f32) * delta_time) * delta_time;
            vec3(result.x, result.y, result.z - fall)
        }
    }
}

/// Evaluate a trajectory velocity at a millisecond time
/// (`BG_EvaluateTrajectoryDelta`).
#[must_use]
pub fn evaluate_trajectory_delta(tr: &Trajectory, at_time: i32) -> Vec3 {
    match tr.trajectory_type {
        TrajectoryType::TrStationary | TrajectoryType::TrInterpolate => vec3(0.0, 0.0, 0.0),
        TrajectoryType::TrLinear => tr.delta,
        TrajectoryType::TrSine => {
            // The source uses a half-amplitude cosine, independent of duration.
            let phase = trajectory_periodic_radians(tr, at_time).cos() * 0.5;
            scale3(tr.delta, phase)
        }
        TrajectoryType::TrLinearStop => {
            if at_time > tr.time.wrapping_add(tr.duration) {
                vec3(0.0, 0.0, 0.0)
            } else {
                tr.delta
            }
        }
        TrajectoryType::TrGravity => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(tr.time));
            vec3(
                tr.delta.x,
                tr.delta.y,
                tr.delta.z - (DEFAULT_GRAVITY as f32) * delta_time,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::base::records::Q3BaseError;
    use qa_core::math::vec3;

    fn linear_fixture() -> Trajectory {
        Trajectory {
            trajectory_type: TrajectoryType::TrLinear,
            time: 1000,
            duration: 0,
            base: vec3(1.0, 2.0, 3.0),
            delta: vec3(100.0, 0.0, -50.0),
        }
    }

    #[test]
    fn linear_trajectory_scales_by_seconds() {
        let at = evaluate_trajectory(&linear_fixture(), 1500);
        assert_eq!(at, vec3(51.0, 2.0, -22.0));
        assert_eq!(
            evaluate_trajectory_delta(&linear_fixture(), 9999),
            vec3(100.0, 0.0, -50.0)
        );
    }

    #[test]
    fn gravity_trajectory_falls_quadratically() {
        let tr = Trajectory {
            trajectory_type: TrajectoryType::TrGravity,
            ..linear_fixture()
        };
        let at = evaluate_trajectory(&tr, 2000);
        assert_eq!(at.x, 101.0);
        assert_eq!(at.z, 3.0 - 50.0 - 400.0);
        let delta = evaluate_trajectory_delta(&tr, 2000);
        assert_eq!(delta, vec3(100.0, 0.0, -850.0));
    }

    #[test]
    fn sine_and_stop_trajectories_match_source() {
        let head_sine = Trajectory {
            trajectory_type: TrajectoryType::TrSine,
            time: 0,
            duration: 1000,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 10.0),
        };
        assert_eq!(evaluate_trajectory(&head_sine, 0), vec3(0.0, 0.0, 0.0));
        let head_stop = Trajectory {
            trajectory_type: TrajectoryType::TrLinearStop,
            time: 0,
            duration: 100,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(10.0, 0.0, 0.0),
        };
        assert_eq!(evaluate_trajectory(&head_stop, 50), vec3(0.5, 0.0, 0.0));
        assert_eq!(evaluate_trajectory(&head_stop, 5000), vec3(1.0, 0.0, 0.0));
        assert_eq!(evaluate_trajectory_delta(&head_stop, 5000), vec3(0.0, 0.0, 0.0));
        let stationary = Trajectory::zero(TrajectoryType::TrStationary);
        assert_eq!(evaluate_trajectory(&stationary, 1234), vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn trajectories_evaluate() {
        let linear = Trajectory {
            trajectory_type: TrajectoryType::TrLinear,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(1000.0, 0.0, 0.0),
        };
        assert_eq!(evaluate_trajectory(&linear, 1000), vec3(1000.0, 0.0, 0.0));
        assert_eq!(evaluate_trajectory_delta(&linear, 500), vec3(1000.0, 0.0, 0.0));
        let gravity = Trajectory {
            trajectory_type: TrajectoryType::TrGravity,
            ..linear
        };
        assert_eq!(evaluate_trajectory(&gravity, 1000), vec3(1000.0, 0.0, -400.0));
        let sine = Trajectory {
            trajectory_type: TrajectoryType::TrSine,
            time: 0,
            duration: 1000,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(10.0, 0.0, 0.0),
        };
        let at_quarter = evaluate_trajectory(&sine, 250);
        assert!((at_quarter.x - 10.0).abs() < 0.001);
        let stop = Trajectory {
            trajectory_type: TrajectoryType::TrLinearStop,
            duration: 500,
            ..linear
        };
        assert_eq!(evaluate_trajectory(&stop, 1000), vec3(500.0, 0.0, 0.0));
        assert_eq!(evaluate_trajectory_delta(&stop, 1000), vec3(0.0, 0.0, 0.0));
        assert!(TrajectoryType::from_i32(9).is_err());
    }

    #[test]
    fn unknown_trajectory_tag_is_a_drop_error() {
        assert!(TrajectoryType::from_i32(5).is_ok());
        let error = TrajectoryType::from_i32(99).unwrap_err();
        assert!(matches!(error, Q3BaseError::Drop(_)));
    }

    #[test]
    fn trajectory_evaluation() {
        let linear = Trajectory {
            trajectory_type: TrajectoryType::TrLinear,
            time: 0,
            duration: 0,
            base: vec3(1.0, 2.0, 3.0),
            delta: vec3(10.0, 0.0, 0.0),
        };
        assert_eq!(evaluate_trajectory(&linear, 1000), vec3(11.0, 2.0, 3.0));
        assert_eq!(evaluate_trajectory_delta(&linear, 500), vec3(10.0, 0.0, 0.0));
        let gravity = Trajectory {
            trajectory_type: TrajectoryType::TrGravity,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 0.0),
        };
        let fallen = evaluate_trajectory(&gravity, 1000);
        assert!((fallen.z + 400.0).abs() < 0.01);
        let stop = Trajectory {
            trajectory_type: TrajectoryType::TrLinearStop,
            time: 0,
            duration: 500,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(10.0, 0.0, 0.0),
        };
        assert_eq!(evaluate_trajectory(&stop, 5000).x, 5.0);
        assert_eq!(evaluate_trajectory_delta(&stop, 5000), vec3(0.0, 0.0, 0.0));
    }
}
