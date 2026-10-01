//! Q2 held weapon models (`src/content/q2/foundation/held-weapons.ts`).

use qa_core::math::Vec3;

use crate::contract::{HeldWeaponModel, ModelTransform};
use crate::q2::foundation::weapons::definitions::base_weapons;
use crate::q2::missionpacks::weapons::definitions::{
    rogue_weapon_definitions, xatrix_weapon_definitions,
};

/// Shared male blaster grip (`MALE_BLASTER_GRIP`).
const MALE_BLASTER_GRIP: ModelTransform = ModelTransform {
    origin: Vec3 {
        x: (-2.5316378672917685_f64 as f32),
        y: (-9.27009121576945_f64 as f32),
        z: (4.795252025127411_f64 as f32),
    },
    axis: [
        Vec3 {
            x: (0.9701912999153137_f64 as f32),
            y: (-0.16778743267059326_f64 as f32),
            z: (-0.17486034333705902_f64 as f32),
        },
        Vec3 {
            x: (0.16538193821907043_f64 as f32),
            y: (0.9858221411705017_f64 as f32),
            z: (-0.02834496460855007_f64 as f32),
        },
        Vec3 {
            x: (0.17713716626167297_f64 as f32),
            y: (-0.0014186727348715067_f64 as f32),
            z: (0.9841850996017456_f64 as f32),
        },
    ],
    scale: Vec3 {
        x: 1.0,
        y: 1.0,
        z: 1.0,
    },
};

/// Native male held-weapon model names by item (`nativeModels`).
const NATIVE_MODELS: &[(&str, &str)] = &[
    ("q2:weapon_blaster", "w_blaster"),
    ("q2:weapon_shotgun", "w_shotgun"),
    ("q2:weapon_supershotgun", "w_sshotgun"),
    ("q2:weapon_machinegun", "w_machinegun"),
    ("q2:weapon_chaingun", "w_chaingun"),
    ("q2:ammo_grenades", "a_grenades"),
    ("q2:weapon_grenadelauncher", "w_glauncher"),
    ("q2:weapon_rocketlauncher", "w_rlauncher"),
    ("q2:weapon_hyperblaster", "w_hyperblaster"),
    ("q2:weapon_railgun", "w_railgun"),
    ("q2:weapon_bfg", "w_bfg"),
    ("q2:ammo_trap", "a_trap"),
    ("q2:weapon_boomer", "w_ripper"),
    ("q2:weapon_phalanx", "w_phalanx"),
    ("q2:ammo_tesla", "a_tesla"),
    ("q2:weapon_proxlauncher", "w_plauncher"),
    ("q2:weapon_chainfist", "w_chainfist"),
    ("q2:weapon_disintegrator", "w_disrupt"),
    ("q2:weapon_etf_rifle", "w_etfrifle"),
    ("q2:weapon_plasmabeam", "w_plasma"),
];

/// Resolve a Q2 held weapon model by view model or item (`q2HeldWeapon`).
pub fn q2_held_weapon(view_model: &str, item: Option<&str>) -> Option<HeldWeaponModel> {
    let mut weapons: Vec<(String, String)> = base_weapons()
        .into_iter()
        .map(|weapon| (weapon.definition.view_model, weapon.definition.item))
        .collect();
    for weapon in xatrix_weapon_definitions()
        .into_iter()
        .chain(rogue_weapon_definitions())
    {
        weapons.push((weapon.view_model, weapon.item));
    }
    let found = match item {
        Some(item) => weapons.iter().find(|(_, candidate)| candidate == item),
        None => weapons.iter().find(|(candidate, _)| candidate == view_model),
    };
    let name = match found {
        Some((_, item)) => NATIVE_MODELS
            .iter()
            .find(|(candidate, _)| candidate == item)
            .map(|(_, name)| *name),
        None if view_model == "models/weapons/grapple/tris.md2" => Some("w_grapple"),
        None => None,
    }?;
    Some(HeldWeaponModel {
        digest: None,
        path: format!("players/male/{name}.md2"),
        reference_frame: 0.0,
        grip: MALE_BLASTER_GRIP,
        fallback: Some("players/male/weapon.md2".to_string()),
        part: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_by_view_model_item_and_grapple() {
        let by_view = q2_held_weapon("models/weapons/v_blast/tris.md2", None).expect("blaster");
        assert_eq!(by_view.path, "players/male/w_blaster.md2");
        let by_item = q2_held_weapon("", Some("q2:weapon_railgun")).expect("railgun");
        assert_eq!(by_item.path, "players/male/w_railgun.md2");
        let rogue = q2_held_weapon("", Some("q2:weapon_chainfist")).expect("chainfist");
        assert_eq!(rogue.path, "players/male/w_chainfist.md2");
        let grapple =
            q2_held_weapon("models/weapons/grapple/tris.md2", None).expect("grapple");
        assert_eq!(grapple.path, "players/male/w_grapple.md2");
        assert!(q2_held_weapon("models/weapons/v_unknown/tris.md2", None).is_none());
    }
}
