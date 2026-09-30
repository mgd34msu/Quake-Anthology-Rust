//! Quake III foundation: held weapons.
//!
//! Donor provenance: `src/content/q3/foundation/held-weapons.ts`.

use crate::contract::ModelTransform;
use qa_core::math::Vec3;

// Intra-group imports: sibling modules split from the same flat port.

// ---------------------------------------------------------------------------
// held-weapons.ts: shared tag registration.
// ---------------------------------------------------------------------------

/// Native weapon hand grip in tag_weapon coordinates (`Q3_WEAPON_HAND_GRIP`).
pub const Q3_WEAPON_HAND_GRIP: ModelTransform = ModelTransform {
    origin: Vec3 {
        x: -2.9841071642362156f64 as f32,
        y: -0.7671715473899474f64 as f32,
        z: -2.208833547738882f64 as f32,
    },
    axis: [
        Vec3 { x: 1.0, y: 0.0, z: 0.0 },
        Vec3 { x: 0.0, y: 1.0, z: 0.0 },
        Vec3 { x: 0.0, y: 0.0, z: 1.0 },
    ],
    scale: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
};

#[cfg(test)]
mod tests {
    use crate::q3::foundation::character::{Q3_CHARACTER_BOUNDS, Q3_CHARACTER_VIEW_HEIGHT};
    use qa_core::math::vec3;

    use super::*;
    #[test]
    fn weapon_hand_grip_matches_sarge_registration() {
        assert_eq!(Q3_WEAPON_HAND_GRIP.origin.x, -2.9841071642362156f64 as f32);
        assert_eq!(Q3_WEAPON_HAND_GRIP.axis[0], vec3(1.0, 0.0, 0.0));
        assert_eq!(Q3_WEAPON_HAND_GRIP.scale, vec3(1.0, 1.0, 1.0));
        assert_eq!(Q3_CHARACTER_VIEW_HEIGHT, 26);
        assert_eq!(Q3_CHARACTER_BOUNDS.min, vec3(-15.0, -15.0, -24.0));
    }
}
