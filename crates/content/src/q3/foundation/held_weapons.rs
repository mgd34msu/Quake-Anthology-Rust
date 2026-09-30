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
