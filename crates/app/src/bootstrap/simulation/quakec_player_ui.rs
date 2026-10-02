//! QuakeC weapon HUD bindings.
//!
//! Provenance: `src/app/bootstrap/simulation/quakec-player-ui.ts`.

use qa_content::contract::ItemId;

use super::arsenal::selected::{ArsenalAmmoWarning, WeaponHudStatus};
use super::types::{PlayerUiItem, PlayerUiItemKind, UiAmmo};

/// Display binding for one source weapon bit.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCWeaponUiBinding {
    /// Weapon item.
    pub item: ItemId,
    /// Display label.
    pub label: String,
    /// Source `items` bit.
    pub bit: i32,
    /// Source impulse.
    pub impulse: i32,
}

/// Weapon HUD rows selected by the source. The source selects its displayed
/// ammo category in `items` and reports `current_ammo` directly.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCWeaponUi {
    /// Active weapon item.
    pub active_weapon: Option<ItemId>,
    /// Displayed ammo.
    pub ammo: Option<UiAmmo>,
    /// Weapon rows.
    pub items: Vec<PlayerUiItem>,
    /// Weapon status (the source owns none).
    pub weapon_status: Option<WeaponHudStatus>,
    /// Arsenal warning (the source owns none).
    pub arsenal_warning: ArsenalAmmoWarning,
}

/// Ammo category bits in donor selection order.
const AMMUNITION: [(i32, &str); 4] = [
    (256, "q1:ammo/shells"),
    (512, "q1:ammo/nails"),
    (1024, "q1:ammo/rockets"),
    (2048, "q1:ammo/cells"),
];

/// Build the weapon HUD from the source `items`/`weapon` words.
#[must_use]
pub fn quake_c_weapon_ui(
    items: i32,
    weapon: i32,
    current_ammo: f64,
    bindings: &[QuakeCWeaponUiBinding],
) -> QuakeCWeaponUi {
    let active = bindings.iter().position(|binding| binding.bit == weapon);
    let category = AMMUNITION
        .iter()
        .find(|(bit, _)| items & *bit != 0)
        .map(|(_, item)| (*item).to_string());
    let ammo = category.map(|item| UiAmmo {
        item,
        count: current_ammo,
    });
    QuakeCWeaponUi {
        active_weapon: active.map(|index| bindings[index].item.clone()),
        ammo: ammo.clone(),
        weapon_status: None,
        arsenal_warning: ArsenalAmmoWarning::None,
        items: bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| {
                let is_active = Some(index) == active;
                PlayerUiItem {
                    id: binding.item.clone(),
                    label: binding.label.clone(),
                    kind: PlayerUiItemKind::Weapon,
                    source_ordinal: f64::from(binding.impulse),
                    owned: items & binding.bit != 0,
                    has_ammo: !is_active || ammo.as_ref().is_none_or(|ammo| ammo.count > 0.0),
                    count: if is_active {
                        ammo.as_ref().map(|ammo| ammo.count)
                    } else {
                        None
                    },
                    warning_count: 0.0,
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bindings() -> Vec<QuakeCWeaponUiBinding> {
        vec![
            QuakeCWeaponUiBinding {
                item: "q1:weapon/shotgun".to_string(),
                label: "Shotgun".to_string(),
                bit: 2,
                impulse: 3,
            },
            QuakeCWeaponUiBinding {
                item: "q1:weapon/nailgun".to_string(),
                label: "Nailgun".to_string(),
                bit: 4,
                impulse: 4,
            },
        ]
    }

    #[test]
    fn selects_active_weapon_and_ammo_category() {
        let ui = quake_c_weapon_ui(2 | 256, 2, 12.0, &bindings());
        assert_eq!(ui.active_weapon.as_deref(), Some("q1:weapon/shotgun"));
        assert_eq!(
            ui.ammo,
            Some(UiAmmo {
                item: "q1:ammo/shells".to_string(),
                count: 12.0,
            })
        );
        assert_eq!(ui.items.len(), 2);
        assert!(ui.items[0].owned);
        assert!(!ui.items[1].owned);
        assert_eq!(ui.items[0].count, Some(12.0));
        assert_eq!(ui.items[1].count, None);
        assert_eq!(ui.weapon_status, None);
        assert_eq!(ui.arsenal_warning, ArsenalAmmoWarning::None);
    }

    #[test]
    fn no_ammo_category_without_bits() {
        let ui = quake_c_weapon_ui(2, 2, 0.0, &bindings());
        assert_eq!(ui.ammo, None);
        assert!(ui.items[0].has_ammo);
    }

    #[test]
    fn empty_active_weapon_has_no_ammo() {
        let ui = quake_c_weapon_ui(2 | 512, 2, 0.0, &bindings());
        assert!(!ui.items[0].has_ammo);
        assert!(ui.items[1].has_ammo);
    }

    #[test]
    fn unknown_weapon_bit_has_no_active_item() {
        let ui = quake_c_weapon_ui(0, 64, 5.0, &bindings());
        assert_eq!(ui.active_weapon, None);
        assert!(ui.items.iter().all(|item| item.count.is_none()));
    }
}
