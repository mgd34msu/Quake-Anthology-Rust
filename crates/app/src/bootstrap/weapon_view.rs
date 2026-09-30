//! Weapon view origin and camera.
//!
//! Donor: `src/app/bootstrap/weapon-view.ts` (`weaponViewOrigin`,
//! `weaponViewCamera`). `Rect`/`SceneCamera` reuse `qa_client::view`,
//! `Vec3` reuses `qa_core::math`, `GameFamily` reuses
//! `qa_content::contract`. Only the three `SimulationPresentation`
//! fields the donor reads are absorbed (see `WeaponViewSource`).

use qa_client::view::{Rect, SceneCamera};
use qa_content::contract::GameFamily;
use qa_core::math::Vec3;
use thiserror::Error;

/// Weapon view failure.
///
/// The donor never throws; this enum exists for module convention.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[allow(dead_code)]
pub enum WeaponViewError {
    /// Unreachable (donor is infallible).
    #[error("unreachable weapon view failure")]
    Unreachable,
}

/// Minimal weapon-view source: the only `SimulationPresentation` fields
/// the donor reads (`viewWeapon`, `family`, `origin`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponViewSource {
    /// Whether this is a view weapon.
    pub view_weapon: bool,
    /// Game family.
    pub family: GameFamily,
    /// World origin.
    pub origin: Vec3,
}

/// Q1 `V_CalcRefdef`'s default viewsize 100 gun calibration precedes
/// camera punch.
#[must_use]
pub fn weapon_view_origin(source: &WeaponViewSource) -> Vec3 {
    if source.view_weapon && source.family == GameFamily::Q1 {
        Vec3 { x: source.origin.x, y: source.origin.y, z: source.origin.z + 2.0 }
    } else {
        source.origin
    }
}

/// Keep horizontal scale and the world viewport; move the weapon
/// projection above the active HUD.
#[must_use]
pub fn weapon_view_camera(camera: &SceneCamera, occupied: &[Rect]) -> SceneCamera {
    let area = camera.viewport;
    let (ax, ay, aw, ah) = (f64::from(area.x), f64::from(area.y), f64::from(area.width), f64::from(area.height));
    let mut bottom = ay + ah;
    for rect in occupied {
        let (rx, ry, rw, rh) = (
            f64::from(rect.x),
            f64::from(rect.y),
            f64::from(rect.width),
            f64::from(rect.height),
        );
        if rect.width > 0 && rect.height > 0 && rx < ax + aw && rx + rw > ax && ry + rh > ay + ah / 2.0
        {
            bottom = bottom.min(ay.max(ry));
        }
    }
    if bottom == ay + ah || bottom <= ay {
        return *camera;
    }
    let offset = (ay + ah - bottom) / ah;
    // Equivalent to a shorter viewport with the same horizontal FOV,
    // expressed in the full seat viewport.
    let p = camera.projection;
    let mut projection = p;
    projection[1] = (f64::from(p[1]) + offset * f64::from(p[3])) as f32;
    projection[5] = (f64::from(p[5]) + offset * f64::from(p[7])) as f32;
    projection[9] = (f64::from(p[9]) + offset * f64::from(p[11])) as f32;
    projection[13] = (f64::from(p[13]) + offset * f64::from(p[15])) as f32;
    SceneCamera { projection, ..*camera }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::view::CameraClip;
    use qa_core::math::vec3;

    fn camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0],
            viewport: Rect { x: 0, y: 0, width: 640, height: 480 },
            clip: CameraClip::None,
        }
    }

    #[test]
    fn q1_view_weapon_lifts_two_units() {
        let source = WeaponViewSource { view_weapon: true, family: GameFamily::Q1, origin: vec3(1.0, 2.0, 3.0) };
        assert_eq!(weapon_view_origin(&source), vec3(1.0, 2.0, 5.0));
    }

    #[test]
    fn other_presentations_pass_through() {
        for (view_weapon, family) in [(false, GameFamily::Q1), (true, GameFamily::Q2), (true, GameFamily::Q3)] {
            let source = WeaponViewSource { view_weapon, family, origin: vec3(1.0, 2.0, 3.0) };
            assert_eq!(weapon_view_origin(&source), vec3(1.0, 2.0, 3.0));
        }
    }

    #[test]
    fn camera_passes_through_without_occlusion() {
        let camera = camera();
        assert_eq!(weapon_view_camera(&camera, &[]), camera);
        // Upper-half HUD does not count.
        let hud = Rect { x: 0, y: 0, width: 640, height: 100 };
        assert_eq!(weapon_view_camera(&camera, &[hud]), camera);
        // Degenerate rects are ignored.
        let flat = Rect { x: 0, y: 400, width: 0, height: 80 };
        assert_eq!(weapon_view_camera(&camera, &[flat]), camera);
    }

    #[test]
    fn camera_shifts_above_lower_hud() {
        let camera = camera();
        let hud = Rect { x: 0, y: 400, width: 640, height: 80 };
        let shifted = weapon_view_camera(&camera, &[hud]);
        let offset = 80.0 / 480.0;
        assert!((shifted.projection[1] - (2.0 + offset as f32 * 4.0)).abs() < 1e-6);
        assert!((shifted.projection[13] - (14.0 + offset as f32 * 16.0)).abs() < 1e-5);
        assert_eq!(shifted.viewport, camera.viewport);
        assert_eq!(shifted.projection[0], 1.0);
    }
}
