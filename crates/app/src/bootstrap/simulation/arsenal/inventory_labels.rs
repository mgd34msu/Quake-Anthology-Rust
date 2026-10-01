//! Selected-ammunition display labels.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/arsenal/inventory-labels.ts`
//! (`selectedAmmoLabel`).

use qa_content::contract::ItemId;
use qa_content::q2::foundation::items::q2_item_pickup_name;
use qa_content::q2::missionpacks::items::q2_mission_weapon_display_name;
use qa_content::q3::base::shared::definitions::{ItemType, Product};
use qa_content::q3::base::shared::items::item_list;
use qa_content::q3::foundation::arsenal::Q3_WEAPON_ITEMS;

use super::selected::ArsenalFamily;

/// Errors resolving a selected-ammunition label.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InventoryLabelError {
    /// Ammunition has no source catalog label.
    #[error("Selected ammunition lacks a source catalog label: {item}")]
    MissingLabel {
        /// Ammunition item.
        item: ItemId,
    },
}

fn q1_ammo_label(item: &str) -> Option<&'static str> {
    match item {
        "q1:ammo/shells" => Some("Shells"),
        "q1:ammo/nails" => Some("Nails"),
        "q1:ammo/rockets" => Some("Rockets"),
        "q1:ammo/cells" => Some("Cells"),
        "rogue:ammo/lava-nails" => Some("Lava Nails"),
        "rogue:ammo/multi-rockets" => Some("Multi Rockets"),
        "rogue:ammo/plasma" => Some("Plasma"),
        _ => None,
    }
}

/// Display label for selected ammunition, from the owning source catalog.
pub fn selected_ammo_label(family: ArsenalFamily, item: &ItemId) -> Result<String, InventoryLabelError> {
    let label = match family {
        ArsenalFamily::Q1 => q1_ammo_label(item).map(str::to_string),
        ArsenalFamily::Q2 => q2_item_pickup_name(item.get(3..).unwrap_or(""))
            .or_else(|| q2_mission_weapon_display_name(item.get(8..).unwrap_or(""))),
        ArsenalFamily::Q3 => {
            let weapon = Q3_WEAPON_ITEMS
                .iter()
                .find(|weapon| weapon.ammo.as_deref() == Some(item.as_str()));
            let tag = weapon.map(|weapon| weapon.weapon as i32);
            item_list(Product::Missionpack)
                .iter()
                .find(|value| value.item_type() == ItemType::ItAmmo && Some(value.tag()) == tag)
                .and_then(|value| value.pickup_name)
                .map(str::to_string)
        }
    };
    label.ok_or_else(|| InventoryLabelError::MissingLabel { item: item.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q1_labels_come_from_the_fixed_table() {
        assert_eq!(
            selected_ammo_label(ArsenalFamily::Q1, &"q1:ammo/shells".to_string()).unwrap(),
            "Shells"
        );
        assert_eq!(
            selected_ammo_label(ArsenalFamily::Q1, &"rogue:ammo/plasma".to_string()).unwrap(),
            "Plasma"
        );
        assert_eq!(
            selected_ammo_label(ArsenalFamily::Q1, &"q1:ammo/bullets".to_string()),
            Err(InventoryLabelError::MissingLabel {
                item: "q1:ammo/bullets".to_string()
            })
        );
    }

    #[test]
    fn q2_labels_come_from_the_source_catalogs() {
        let label = selected_ammo_label(ArsenalFamily::Q2, &"q2:ammo_shells".to_string()).unwrap();
        assert!(!label.is_empty());
        assert_eq!(
            selected_ammo_label(ArsenalFamily::Q2, &"q2:ammo_nope".to_string()),
            Err(InventoryLabelError::MissingLabel {
                item: "q2:ammo_nope".to_string()
            })
        );
    }

    #[test]
    fn q3_labels_come_from_the_missionpack_item_list() {
        let label = selected_ammo_label(ArsenalFamily::Q3, &"q3:ammo/railgun".to_string()).unwrap();
        assert!(!label.is_empty());
        assert_eq!(
            selected_ammo_label(ArsenalFamily::Q3, &"q3:ammo/nope".to_string()),
            Err(InventoryLabelError::MissingLabel {
                item: "q3:ammo/nope".to_string()
            })
        );
    }
}
