//! Quake I monster AI rules ported from `src/movement/q1/monsters.ts`
//! (`WinQuake/sv_move.c`, `PF_changeyaw`/`PF_walkmove` from `pr_cmds.c`) and
//! the movement constants in `src/movement/q1/types.ts`. Pure decision
//! helpers: angle stepping, chase-direction order, fly/swim depth adjust,
//! range tests, and think-gating predicates. Scene traces and actor writes
//! stay with the owning provider; this module pins the donor numbers.

use qa_core::math::{Bounds, Vec3};

/// Ground flag bit shared with [`crate::movement::q1_flag::ONGROUND`].
pub const Q1_FLAG_ONGROUND: i32 = 512;
/// Fly flag bit.
pub const Q1_FLAG_FLY: i32 = 1;
/// Swim flag bit.
pub const Q1_FLAG_SWIM: i32 = 2;
/// Partial-ground flag bit (`monsters.ts`).
pub const Q1_FLAG_PARTIALGROUND: i32 = 1024;
/// Empty contents.
pub const Q1_CONTENTS_EMPTY: i32 = -1;
/// Solid contents.
pub const Q1_CONTENTS_SOLID: i32 = -2;
/// Water contents.
pub const Q1_CONTENTS_WATER: i32 = -3;
/// Slime contents.
pub const Q1_CONTENTS_SLIME: i32 = -4;
/// Lava contents.
pub const Q1_CONTENTS_LAVA: i32 = -5;
/// Step height in world units.
pub const Q1_STEP_HEIGHT: f64 = 18.0;
/// Fly/swim descent trigger: enemy this far below.
pub const Q1_FLY_DESCEND_DZ: f64 = 40.0;
/// Fly/swim ascent trigger: enemy this far above (dz below this).
pub const Q1_FLY_ASCEND_DZ: f64 = 30.0;
/// Fly/swim vertical correction per move.
pub const Q1_FLY_DZ_STEP: f64 = 8.0;
/// Chase axis dead zone in world units.
pub const Q1_CHASE_AXIS_DEADZONE: f64 = 10.0;
/// Step-direction yaw clamp in degrees.
pub const Q1_STEP_YAW_CLAMP: f64 = 45.0;
/// Turnaround sweep span in degrees.
pub const Q1_SWEEP_MAX_YAW: i32 = 315;
/// Diagonal chase constant for southwest. The donor notes this is 215 in
/// both the released source and the donor (not the symmetric 225).
pub const Q1_CHASE_DIAGONAL_SOUTHWEST: i32 = 215;

/// Wrap an angle to `[0, 360)` through the donor's 16-bit yaw encoding
/// (`angle * 65536 / 360` truncated to `u16`, scaled back).
#[must_use]
pub fn q1_angle_mod(angle: f64) -> f64 {
    let encoded = (angle * (65536.0 / 360.0)).trunc() as i64 & 0xffff;
    360.0 / 65536.0 * encoded as f64
}

/// One yaw step toward `ideal`, limited to `yaw_speed` degrees. Mirrors
/// `changeYaw`: the shortest arc across the 180-degree seam wins.
#[must_use]
pub fn q1_change_yaw_step(current: f64, ideal: f64, yaw_speed: f64) -> f64 {
    let current = q1_angle_mod(current);
    if current == ideal {
        return current;
    }
    let mut delta = ideal - current;
    if ideal > current {
        if delta >= 180.0 {
            delta -= 360.0;
        }
    } else if delta <= -180.0 {
        delta += 360.0;
    }
    let step = if delta > 0.0 {
        delta.min(yaw_speed)
    } else {
        delta.max(-yaw_speed)
    };
    q1_angle_mod(current + step)
}

/// Whether `walkMove`/`moveToGoal` may run: grounded, flying, or swimming.
#[must_use]
pub fn q1_can_ground_move(flags: i32) -> bool {
    flags & (Q1_FLAG_ONGROUND | Q1_FLAG_FLY | Q1_FLAG_SWIM) != 0
}

/// Fly/swim vertical correction for the signed height above the enemy
/// (`origin.z - enemy.z`): descend 8 when more than 40 above, ascend 8
/// when less than 30 above, else hold depth.
#[must_use]
pub fn q1_fly_depth_adjust(dz: f64) -> f64 {
    if dz > Q1_FLY_DESCEND_DZ {
        -Q1_FLY_DZ_STEP
    } else if dz < Q1_FLY_ASCEND_DZ {
        Q1_FLY_DZ_STEP
    } else {
        0.0
    }
}

/// Range contact test (`closeEnough`): target bounds expanded by `distance`
/// on every axis must intersect the mover bounds.
#[must_use]
pub fn q1_close_enough(mover: &Bounds, target: &Bounds, distance: f64) -> bool {
    let distance = distance as f32;
    target.min.x <= mover.max.x + distance
        && target.max.x >= mover.min.x - distance
        && target.min.y <= mover.max.y + distance
        && target.max.y >= mover.min.y - distance
        && target.min.z <= mover.max.z + distance
        && target.max.z >= mover.min.z - distance
}

/// Chase axis picks from the enemy offset with a 10-unit dead zone:
/// east 0, west 180, north 90, south 270, or -1 when inside the zone.
#[must_use]
pub fn q1_chase_axes(origin: &Vec3, enemy: &Vec3) -> (i32, i32) {
    let dx = f64::from(enemy.x - origin.x);
    let dy = f64::from(enemy.y - origin.y);
    let first = if dx > Q1_CHASE_AXIS_DEADZONE {
        0
    } else if dx < -Q1_CHASE_AXIS_DEADZONE {
        180
    } else {
        -1
    };
    let second = if dy < -Q1_CHASE_AXIS_DEADZONE {
        270
    } else if dy > Q1_CHASE_AXIS_DEADZONE {
        90
    } else {
        -1
    };
    (first, second)
}

/// Diagonal chase direction for two set axes. Southwest is the donor's
/// asymmetric 215-degree constant.
#[must_use]
pub fn q1_chase_diagonal(first: i32, second: i32) -> i32 {
    if first == 0 {
        if second == 90 {
            45
        } else {
            315
        }
    } else if second == 90 {
        135
    } else {
        Q1_CHASE_DIAGONAL_SOUTHWEST
    }
}

/// Whether `moveToGoal` falls through to `newChaseDirection`: a 1-in-4
/// random roll or a failed step along the ideal yaw.
#[must_use]
pub fn q1_retarget_chase(random: i32, stepped: bool) -> bool {
    random & 3 == 1 || !stepped
}

/// Whether a successful `stepDirection` keeps its move: the donor reverts
/// moves whose yaw error lands strictly between 45 and 315 degrees.
#[must_use]
pub fn q1_step_keeps_move(angle_yaw: f64, ideal_yaw: f64) -> bool {
    let delta = angle_yaw - ideal_yaw;
    !(delta > Q1_STEP_YAW_CLAMP && delta < 360.0 - Q1_STEP_YAW_CLAMP)
}

/// Turnaround direction for an old quantized direction.
#[must_use]
pub fn q1_turnaround(old_direction: f64) -> f64 {
    q1_angle_mod(old_direction - 180.0)
}

/// Quantize an ideal yaw to a 45-degree chase direction.
#[must_use]
pub fn q1_quantize_chase_direction(ideal_yaw: f64) -> f64 {
    q1_angle_mod((ideal_yaw / 45.0).trunc() * 45.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;

    #[test]
    fn angle_mod_matches_donor_encoding() {
        assert_eq!(q1_angle_mod(0.0), 0.0);
        assert_eq!(q1_angle_mod(360.0), 0.0);
        assert_eq!(q1_angle_mod(90.0), 90.0);
        assert!((q1_angle_mod(45.0) - 45.0).abs() < 0.01);
        assert_eq!(q1_angle_mod(-90.0), 270.0);
    }

    #[test]
    fn change_yaw_takes_shortest_arc_and_caps_speed() {
        assert_eq!(q1_change_yaw_step(0.0, 0.0, 10.0), 0.0);
        assert_eq!(q1_change_yaw_step(0.0, 90.0, 10.0), q1_angle_mod(10.0));
        assert_eq!(q1_change_yaw_step(0.0, 90.0, 100.0), 90.0);
        assert!((q1_change_yaw_step(350.0, 10.0, 45.0) - 10.0).abs() < 0.01);
        assert!((q1_change_yaw_step(10.0, 350.0, 45.0) - 350.0).abs() < 0.01);
        assert!((q1_change_yaw_step(0.0, 180.0, 10.0) - 350.0).abs() < 0.01);
    }

    #[test]
    fn ground_move_requires_contact_or_flight() {
        assert!(q1_can_ground_move(Q1_FLAG_ONGROUND));
        assert!(q1_can_ground_move(Q1_FLAG_FLY));
        assert!(q1_can_ground_move(Q1_FLAG_SWIM));
        assert!(!q1_can_ground_move(0));
        assert!(!q1_can_ground_move(Q1_FLAG_PARTIALGROUND));
    }

    #[test]
    fn fly_depth_adjust_uses_donor_thresholds() {
        assert_eq!(q1_fly_depth_adjust(41.0), -8.0);
        assert_eq!(q1_fly_depth_adjust(40.0), 0.0);
        assert_eq!(q1_fly_depth_adjust(30.0), 0.0);
        assert_eq!(q1_fly_depth_adjust(29.0), 8.0);
    }

    #[test]
    fn chase_axes_and_diagonals_match_donor() {
        let origin = vec3(0.0, 0.0, 0.0);
        assert_eq!(q1_chase_axes(&origin, &vec3(20.0, 30.0, 0.0)), (0, 90));
        assert_eq!(q1_chase_axes(&origin, &vec3(-20.0, -30.0, 0.0)), (180, 270));
        assert_eq!(q1_chase_axes(&origin, &vec3(5.0, 5.0, 0.0)), (-1, -1));
        assert_eq!(q1_chase_diagonal(0, 90), 45);
        assert_eq!(q1_chase_diagonal(0, 270), 315);
        assert_eq!(q1_chase_diagonal(180, 90), 135);
        assert_eq!(q1_chase_diagonal(180, 270), 215);
    }

    #[test]
    fn chase_gating_and_step_revert_match_donor() {
        assert!(q1_retarget_chase(1, true));
        assert!(!q1_retarget_chase(0, true));
        assert!(q1_retarget_chase(0, false));
        assert!(q1_step_keeps_move(10.0, 0.0));
        assert!(!q1_step_keeps_move(100.0, 0.0));
        assert!(q1_step_keeps_move(320.0, 0.0));
        assert_eq!(q1_turnaround(0.0), 180.0);
        assert_eq!(q1_quantize_chase_direction(100.0), 90.0);
    }

    #[test]
    fn close_enough_expands_target_bounds() {
        let mover = Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(32.0, 32.0, 56.0),
        };
        let near = Bounds {
            min: vec3(40.0, 0.0, 0.0),
            max: vec3(72.0, 32.0, 56.0),
        };
        assert!(!q1_close_enough(&mover, &near, 4.0));
        assert!(q1_close_enough(&mover, &near, 8.0));
    }
}
