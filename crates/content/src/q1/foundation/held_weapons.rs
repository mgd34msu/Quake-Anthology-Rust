//! Q1 held-weapon models (`src/content/q1/foundation/held-weapons.ts`).
//!
//! player.qc stand1/axstnd1 use one carried firearm and one axe for
//! the complete Q1 arsenal.

use qa_core::math::Vec3;

use crate::contract::{HeldWeaponModel, HeldWeaponPart, ModelTransform};

fn digests() -> Vec<String> {
    vec![
        String::from("sha256:10cecfe08d312ff17c63529e280b976c50018f683f541cb03b2048c7a681ebf9"),
        String::from("sha256:7bd9988aa264d27cea670bbf280d9101d712cf27d87dbe774ac41d363bdf01d2"),
    ]
}

fn gun() -> HeldWeaponModel {
    HeldWeaponModel {
        digest: None,
        path: String::from("progs/player.mdl"),
        reference_frame: 12.0,
        grip: ModelTransform {
            origin: Vec3 {
                x: 3.1924999,
                y: -5.9476357,
                z: 10.002536,
            },
            axis: [
                Vec3 {
                    x: 0.7190016,
                    y: 0.6357764,
                    z: -0.28075826,
                },
                Vec3 {
                    x: -0.6084284,
                    y: 0.7710461,
                    z: 0.18789083,
                },
                Vec3 {
                    x: 0.33593407,
                    y: 0.035727482,
                    z: 0.94120777,
                },
            ],
            scale: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
        },
        fallback: None,
        part: Some(HeldWeaponPart {
            digests: digests(),
            vertices: [
                40.0, 41.0, 42.0, 100.0, 101.0, 102.0, 116.0, 117.0, 118.0, 142.0, 143.0, 144.0, 145.0, 146.0, 147.0,
                148.0, 156.0, 157.0, 158.0, 174.0, 175.0, 177.0, 178.0, 179.0, 185.0, 186.0, 187.0, 188.0, 189.0,
                190.0, 191.0, 194.0, 195.0, 196.0, 199.0, 200.0, 201.0, 202.0,
            ]
            .to_vec(),
        }),
    }
}

fn axe() -> HeldWeaponModel {
    HeldWeaponModel {
        digest: None,
        path: String::from("progs/player.mdl"),
        reference_frame: 17.0,
        grip: ModelTransform {
            origin: Vec3 {
                x: -1.0801829,
                y: -16.939234,
                z: 3.4784603,
            },
            axis: [
                Vec3 {
                    x: 0.85296005,
                    y: 0.34326276,
                    z: 0.39322984,
                },
                Vec3 {
                    x: -0.49903303,
                    y: 0.3153751,
                    z: 0.8071583,
                },
                Vec3 {
                    x: 0.15305252,
                    y: -0.8847085,
                    z: 0.44030184,
                },
            ],
            scale: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
        },
        fallback: None,
        part: Some(HeldWeaponPart {
            digests: digests(),
            vertices: [
                9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 20.0, 21.0, 22.0, 23.0, 24.0,
            ]
            .to_vec(),
        }),
    }
}

/// Held-weapon model for a view model (`q1HeldWeapon`).
#[must_use]
pub fn q1_held_weapon(view_model: &str) -> Option<HeldWeaponModel> {
    if view_model.starts_with("progs/v_") && view_model.ends_with(".mdl") {
        Some(if view_model == "progs/v_axe.mdl" { axe() } else { gun() })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_weapons_select_by_view_model() {
        assert_eq!(
            q1_held_weapon("progs/v_axe.mdl").map(|model| model.reference_frame),
            Some(17.0)
        );
        assert_eq!(
            q1_held_weapon("progs/v_shot.mdl").map(|model| model.reference_frame),
            Some(12.0)
        );
        assert_eq!(q1_held_weapon("progs/player.mdl"), None);
    }
}
