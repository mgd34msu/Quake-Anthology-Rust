//! Quake III view-angle update.
//!
//! Donor provenance: `src/movement/q3/view.ts` (`PM_UpdateViewAngles`;
/// callers supply the source movement enum).
use qa_core::math::Vec3;

/// Updated view angles plus adjusted delta.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3ViewAngles {
    /// View angles.
    pub angles: Vec3,
    /// Adjusted delta.
    pub delta: Vec3,
}

/// `PM_UpdateViewAngles`; callers supply the source movement enum.
#[must_use]
pub fn q3_view_angles(
    command: Vec3,
    delta: Vec3,
    previous: Vec3,
    health: f64,
    movement_type: i32,
    intermission: &[i32],
) -> Q3ViewAngles {
    if intermission.contains(&movement_type) || (movement_type != 2 && health <= 0.0) {
        return Q3ViewAngles {
            angles: previous,
            delta,
        };
    }
    let command_x = command.x as i32;
    let mut delta_x = delta.x as i32;
    let mut pitch = (command_x.wrapping_add(delta_x) << 16) >> 16;
    if pitch > 16000 {
        delta_x = 16000 - command_x;
        pitch = 16000;
    } else if pitch < -16000 {
        delta_x = -16000 - command_x;
        pitch = -16000;
    }
    let yaw = ((command.y as i32).wrapping_add(delta.y as i32) << 16) >> 16;
    let roll = ((command.z as i32).wrapping_add(delta.z as i32) << 16) >> 16;
    let scale = 360.0 / 65536.0;
    Q3ViewAngles {
        angles: Vec3 {
            x: pitch as f32 * scale,
            y: yaw as f32 * scale,
            z: roll as f32 * scale,
        },
        delta: Vec3 {
            x: delta_x as f32,
            y: delta.y,
            z: delta.z,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;

    #[test]
    fn intermission_and_death_keep_angles() {
        let previous = vec3(10.0, 20.0, 30.0);
        let out = q3_view_angles(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), previous, 100.0, 5, &[5, 6]);
        assert_eq!((out.angles, out.delta), (previous, vec3(0.0, 0.0, 0.0)));
        let out = q3_view_angles(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), previous, 0.0, 0, &[5, 6]);
        assert_eq!(out.angles, previous);
    }

    #[test]
    fn words_convert_to_degrees() {
        let out = q3_view_angles(
            vec3(0.0, 8192.0, 0.0),
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 0.0),
            100.0,
            0,
            &[5, 6],
        );
        assert_eq!(out.angles.y, 45.0);
    }

    #[test]
    fn pitch_clamps_and_rewrites_delta() {
        let out = q3_view_angles(
            vec3(0.0, 0.0, 0.0),
            vec3(20000.0, 0.0, 0.0),
            vec3(0.0, 0.0, 0.0),
            100.0,
            0,
            &[5, 6],
        );
        assert_eq!(out.delta.x, 16000.0);
        assert_eq!(out.angles.x, 16000.0 * (360.0 / 65536.0));
        let out = q3_view_angles(
            vec3(0.0, 0.0, 0.0),
            vec3(-20000.0, 0.0, 0.0),
            vec3(0.0, 0.0, 0.0),
            100.0,
            0,
            &[5, 6],
        );
        assert_eq!(out.delta.x, -16000.0);
    }
}
