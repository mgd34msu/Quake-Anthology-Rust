//! Q3 guest weapon catalogs (`src/content/q3/guest-items.ts`).

use std::collections::HashSet;
use std::sync::LazyLock;

use qa_guest::error::GuestError;
use qa_guest::qvm::artifacts::{known_qvm_artifacts, QvmProduct};
use qa_guest::qvm::game_data::{ProfileReader, ProfileValue, QvmArtifact, QvmModule};
use qa_guest::qvm::game_inventory::QvmInventoryProfile;
use qa_guest::qvm::item_catalog::{
    parse_qvm_item_layout, read_qvm_item_catalog, QvmCatalogItem, QvmCatalogRecord, QvmItemAddress, QvmItemCount,
    QvmItemFields, QvmItemLayout, QvmSourceItemCatalog,
};
use qa_guest::qvm::item_storage::QvmItemStorage;
use qa_world::combat::ItemId;

use super::base::shared::definitions::{ItemType, Product};
use super::base::shared::items::item_list;
use super::equipment::threewave_grapple_profile::THREEWAVE_GRAPPLE_DIGEST;
use super::foundation::arsenal::{q3_weapon_item, Q3_WEAPON_ITEMS};
use crate::mounts::MountedContent;
use crate::value::{parse_save_json, SaveJson, SaveReader};

/// Q3 guest weapon row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3GuestWeapon {
    /// Source weapon number.
    pub weapon: i32,
    /// Weapon item.
    pub item: ItemId,
    /// Ammo item, if any.
    pub ammo: Option<ItemId>,
    /// Pickup label.
    pub label: String,
}

/// Declared weapon selection value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3GuestSelection {
    /// Source value.
    pub value: i32,
    /// Weapon item.
    pub item: ItemId,
}

/// Standard (mission-pack) guest weapons with source pickup labels.
pub static STANDARD_Q3_GUEST_WEAPONS: LazyLock<Vec<Q3GuestWeapon>> = LazyLock::new(|| {
    let source_items = item_list(Product::Missionpack);
    Q3_WEAPON_ITEMS
        .iter()
        .map(|weapon| {
            let label = source_items
                .iter()
                .find(|item| item.item_type() == ItemType::ItWeapon && item.tag() == weapon.weapon as i32)
                .and_then(|item| item.pickup_name)
                .unwrap_or(weapon.item.as_str())
                .to_string();
            Q3GuestWeapon {
                weapon: weapon.weapon as i32,
                item: weapon.item.clone(),
                ammo: weapon.ammo.clone(),
                label,
            }
        })
        .collect()
});

/// Base-game guest weapons (source weapons 1 through 10).
pub static BASE_Q3_GUEST_WEAPONS: LazyLock<Vec<Q3GuestWeapon>> = LazyLock::new(|| {
    STANDARD_Q3_GUEST_WEAPONS
        .iter()
        .filter(|weapon| weapon.weapon <= 10)
        .cloned()
        .collect()
});

/// Threewave 1.7 `bg_itemlist` layout: original initialized records,
/// excluding its empty terminal record.
fn threewave_layout() -> QvmItemLayout {
    QvmItemLayout {
        address: QvmItemAddress::Direct(5356),
        count: QvmItemCount::Direct(49),
        live_source: false,
        stride: 52,
        fields: QvmItemFields {
            class_name: 0,
            pickup_name: 28,
            item_type: 36,
            tag: 40,
        },
        weapon_type: 1,
        ammo_type: 2,
    }
}

fn guest_item_id(module_digest: &str, class_name: &str) -> ItemId {
    format!("q3:guest/{module_digest}/{class_name}")
}

fn save_json_to_profile(value: &SaveJson) -> ProfileValue {
    match value {
        SaveJson::Null => ProfileValue::Null,
        SaveJson::Bool(value) => ProfileValue::Bool(*value),
        SaveJson::Number(value) => {
            if value.fract() == 0.0 && *value >= i64::MIN as f64 && *value <= i64::MAX as f64 {
                ProfileValue::Int(*value as i64)
            } else {
                ProfileValue::Float(*value)
            }
        }
        SaveJson::BigInt(value) => i64::try_from(*value).map_or(ProfileValue::Float(*value as f64), ProfileValue::Int),
        SaveJson::Bytes(value) => ProfileValue::Bytes(value.clone()),
        SaveJson::String(value) => ProfileValue::Str(value.clone()),
        SaveJson::Array(items) => ProfileValue::Array(items.iter().map(save_json_to_profile).collect()),
        SaveJson::Object(members) => ProfileValue::Record(
            members
                .iter()
                .map(|(key, value)| (key.clone(), save_json_to_profile(value)))
                .collect(),
        ),
    }
}

/// Guest weapons for an artifact: declared layouts and `qvm-items.json`
/// first, then the known-product fallback.
pub fn q3_guest_weapons(
    artifact: &QvmArtifact,
    mounts: &MountedContent,
    declared_layout: Option<QvmItemLayout>,
    private_inventory: bool,
) -> Result<Vec<Q3GuestWeapon>, GuestError> {
    let mut layout =
        declared_layout.or_else(|| (artifact.module.digest == THREEWAVE_GRAPPLE_DIGEST).then(threewave_layout));
    if declared_layout.is_none() {
        let opened = mounts
            .open("qvm-items.json", |_| true)
            .map_err(|error| GuestError::invalid(error.to_string()))?;
        if let Some(opened) = opened {
            let text = String::from_utf8_lossy(&opened.bytes);
            let document = parse_save_json(&text).map_err(|error| GuestError::invalid(error.to_string()))?;
            let reader = SaveReader::at(&document, "qvm-items.json");
            reader
                .field("version")
                .literal_i64(1)
                .map_err(|error| GuestError::invalid(error.to_string()))?;
            let digest = reader
                .field("artifactDigest")
                .string()
                .map_err(|error| GuestError::invalid(error.to_string()))?;
            if digest != artifact.module.digest {
                return Err(GuestError::invalid(
                    reader
                        .fail("item declaration belongs to different qagame bytes")
                        .to_string(),
                ));
            }
            let items = document
                .get("items")
                .map_or(ProfileValue::Undefined, save_json_to_profile);
            layout = Some(parse_qvm_item_layout(&ProfileReader::new(&items))?);
        }
    }
    let Some(layout) = layout else {
        let missionpack = known_qvm_artifacts()
            .iter()
            .find(|entry| entry.digest == artifact.module.digest)
            .is_some_and(|entry| entry.product == QvmProduct::Missionpack);
        return Ok(if missionpack {
            STANDARD_Q3_GUEST_WEAPONS.clone()
        } else {
            BASE_Q3_GUEST_WEAPONS.clone()
        });
    };
    if layout.live_source {
        if declared_layout.is_none() {
            return Err(GuestError::invalid(
                "Live QVM catalogs require the complete declared primary interface",
            ));
        }
        return Ok(Vec::new());
    }
    let entries = read_qvm_item_catalog(&artifact.image.initialized_data, &layout, private_inventory)?;
    Ok(catalog_weapons(&artifact.module.digest, &layout, &entries, None))
}

fn catalog_weapons(
    module_digest: &str,
    layout: &QvmItemLayout,
    entries: &[QvmCatalogItem],
    selection: Option<&[Q3GuestSelection]>,
) -> Vec<Q3GuestWeapon> {
    let source_items = item_list(Product::Missionpack);
    entries
        .iter()
        .enumerate()
        .filter(|(index, item)| {
            item.item_type == layout.weapon_type
                && entries[..*index]
                    .iter()
                    .all(|other| !(other.item_type == layout.weapon_type && other.tag == item.tag))
        })
        .map(|(_, item)| {
            let original = source_items.iter().find(|entry| {
                entry.item_type() == ItemType::ItWeapon && entry.class_name == Some(item.class_name.as_str())
            });
            let canonical = original.and_then(|entry| q3_weapon_item(entry.tag()));
            let ammo = entries
                .iter()
                .find(|entry| entry.item_type == layout.ammo_type && entry.tag == item.tag);
            let canonical_ammo = ammo.and_then(|ammo| {
                source_items
                    .iter()
                    .find(|entry| {
                        entry.item_type() == ItemType::ItAmmo && entry.class_name == Some(ammo.class_name.as_str())
                    })
                    .and_then(|entry| q3_weapon_item(entry.tag()))
                    .and_then(|weapon| weapon.ammo.clone())
            });
            Q3GuestWeapon {
                weapon: item.tag,
                item: selection
                    .and_then(|selection| selection.iter().find(|value| value.value == item.tag))
                    .map(|value| value.item.clone())
                    .or_else(|| canonical.map(|weapon| weapon.item.clone()))
                    .unwrap_or_else(|| guest_item_id(module_digest, &item.class_name)),
                ammo: ammo.map(|ammo| {
                    canonical_ammo
                        .clone()
                        .unwrap_or_else(|| guest_item_id(module_digest, &ammo.class_name))
                }),
                label: item.pickup_name.clone(),
            }
        })
        .collect()
}

fn validate_weapons(
    weapons: &[Q3GuestWeapon],
    selection: Option<&[Q3GuestSelection]>,
    private_items: Option<&HashSet<ItemId>>,
) -> Result<(), GuestError> {
    for weapon in weapons {
        if weapon.weapon < 1 || private_items.is_none() && weapon.weapon > 15 {
            return Err(GuestError::invalid(
                "QVM item cannot be represented by its declared inventory storage",
            ));
        }
        if selection.is_some_and(|selection| {
            !selection
                .iter()
                .any(|value| value.value == weapon.weapon && value.item == weapon.item)
        }) {
            return Err(GuestError::invalid(
                "QVM weapon has no declared original selection value",
            ));
        }
        if private_items.is_some_and(|private| {
            !private.contains(&weapon.item) || weapon.ammo.as_ref().is_some_and(|ammo| !private.contains(ammo))
        }) {
            return Err(GuestError::invalid(
                "QVM item has no declared original inventory storage",
            ));
        }
    }
    Ok(())
}

/// One live original module owns discovery; preparation never executes a
/// second game initialization.
pub struct Q3GuestCatalog {
    module_digest: String,
    layout: Option<QvmItemLayout>,
    table: Option<QvmSourceItemCatalog>,
    closed: bool,
    previous: Option<Vec<QvmCatalogRecord>>,
    weapons: Vec<Q3GuestWeapon>,
    selection: Option<Vec<Q3GuestSelection>>,
    private_items: Option<HashSet<ItemId>>,
}

impl Q3GuestCatalog {
    /// Build a guest catalog over a module's item table.
    pub fn new(
        artifact: &QvmArtifact,
        module: &QvmModule,
        layout: Option<QvmItemLayout>,
        initial: Vec<Q3GuestWeapon>,
        selection: Option<Vec<Q3GuestSelection>>,
        inventory: Option<&QvmInventoryProfile>,
    ) -> Result<Self, GuestError> {
        if !module.module_id().same_module(&artifact.module) {
            return Err(GuestError::invalid(
                "QVM item catalog belongs to a different original module",
            ));
        }
        let table = layout
            .map(|layout| QvmSourceItemCatalog::new(module.memory(), layout, artifact.image.initialized_data.clone()));
        let private_items = match inventory {
            Some(QvmInventoryProfile::Private(profile)) => Some(
                profile
                    .storage
                    .iter()
                    .flat_map(|storage| match storage {
                        QvmItemStorage::Counter { item, .. } => vec![item.clone()],
                        QvmItemStorage::Bits { items, .. } => items.iter().map(|item| item.item.clone()).collect(),
                    })
                    .collect(),
            ),
            _ => None,
        };
        validate_weapons(&initial, selection.as_deref(), private_items.as_ref())?;
        Ok(Self {
            module_digest: artifact.module.digest.clone(),
            layout,
            table,
            closed: false,
            previous: None,
            weapons: initial,
            selection,
            private_items,
        })
    }

    /// Read the source records.
    pub fn records(&self) -> Result<Vec<QvmCatalogRecord>, GuestError> {
        if self.closed {
            return Err(GuestError::invalid("QVM item catalog owner is retired"));
        }
        let Some(table) = self.table.as_ref() else {
            return Err(GuestError::invalid("QVM source has no declared item table"));
        };
        table.records()
    }

    /// Read the guest weapons, refreshing live tables on change.
    pub fn weapons(&mut self) -> Result<Vec<Q3GuestWeapon>, GuestError> {
        if self.closed {
            return Err(GuestError::invalid("QVM item catalog owner is retired"));
        }
        let (Some(table), Some(layout)) = (self.table.as_ref(), self.layout) else {
            return Ok(self.weapons.clone());
        };
        if !layout.live_source {
            return Ok(self.weapons.clone());
        }
        let entries = table.records()?;
        if self.previous.as_ref() != Some(&entries) {
            let items = entries
                .iter()
                .map(|entry| QvmCatalogItem {
                    class_name: entry.class_name.clone(),
                    pickup_name: entry.pickup_name.clone(),
                    item_type: entry.item_type,
                    tag: entry.tag,
                })
                .collect::<Vec<_>>();
            let weapons = catalog_weapons(&self.module_digest, &layout, &items, self.selection.as_deref());
            validate_weapons(&weapons, self.selection.as_deref(), self.private_items.as_ref())?;
            self.previous = Some(entries);
            self.weapons = weapons;
        }
        Ok(self.weapons.clone())
    }

    /// Drop cached records.
    pub fn reset(&mut self) {
        if let Some(table) = self.table.as_ref() {
            table.reset();
        }
        self.previous = None;
    }

    /// Retire the catalog.
    pub fn close(&mut self) {
        if let Some(table) = self.table.as_ref() {
            table.close();
        }
        self.previous = None;
        self.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use qa_guest::qvm::game_data::QvmRole;

    use super::*;
    use crate::contract::{create_content_id, ContentIdentity, GameFamily};
    use crate::q3::test_support::{fixture_artifact, mount_loose_dir};

    fn stock_artifact() -> QvmArtifact {
        fixture_artifact(
            "sha256:57c52bf22e4f528c064f8af1553a7103723bab0a02276bb11eed944bf829b219",
            QvmRole::Qagame,
            64,
            &[],
        )
    }

    fn mounted(name: &str, files: &[(&str, &str)]) -> MountedContent {
        let root = std::env::temp_dir().join(format!("qa-q3-guest-items-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for (name, text) in files {
            std::fs::write(root.join(name), text).unwrap();
        }
        let content = create_content_id(&ContentIdentity {
            family: GameFamily::Q3,
            edition: "classic".to_string(),
            package: "baseq3".to_string(),
            revision: "v1".to_string(),
        })
        .unwrap();
        mount_loose_dir(&root, &content)
    }

    #[test]
    fn standard_and_base_weapons_carry_source_labels() {
        assert_eq!(STANDARD_Q3_GUEST_WEAPONS.len(), 13);
        assert_eq!(BASE_Q3_GUEST_WEAPONS.len(), 10);
        let machinegun = STANDARD_Q3_GUEST_WEAPONS
            .iter()
            .find(|weapon| weapon.weapon == 2)
            .unwrap();
        assert_eq!(machinegun.label, "Machinegun");
        assert_eq!(machinegun.ammo.as_deref(), Some("q3:ammo/machinegun"));
        let gauntlet = STANDARD_Q3_GUEST_WEAPONS
            .iter()
            .find(|weapon| weapon.weapon == 1)
            .unwrap();
        assert_eq!(gauntlet.ammo, None);
    }

    #[test]
    fn guest_weapons_fall_back_by_known_product() {
        let mounts = mounted("fallback", &[]);
        let stock = q3_guest_weapons(&stock_artifact(), &mounts, None, false).unwrap();
        assert_eq!(stock.len(), 10);
        let pack = fixture_artifact(
            "sha256:da041f17f296feeaf8269eabc9062cefdecddfd24ff4d84eb291902e527d1d8a",
            QvmRole::Qagame,
            64,
            &[],
        );
        let missionpack = q3_guest_weapons(&pack, &mounts, None, false).unwrap();
        assert_eq!(missionpack.len(), 13);
    }

    #[test]
    fn guest_weapons_read_declared_layouts() {
        let declaration = r#"{"version": 1, "artifactDigest": "sha256:57c52bf22e4f528c064f8af1553a7103723bab0a02276bb11eed944bf829b219",
          "items": {"address": 64, "count": 2, "stride": 52,
            "fields": {"className": 0, "pickupName": 28, "type": 36, "tag": 40}, "weaponType": 1, "ammoType": 2}}"#;
        let mounts = mounted("declared", &[("qvm-items.json", declaration)]);
        let mut artifact = stock_artifact();
        let mut data = vec![0u8; 512];
        data[64..68].copy_from_slice(&128i32.to_le_bytes());
        data[92..96].copy_from_slice(&160i32.to_le_bytes());
        data[100..104].copy_from_slice(&1i32.to_le_bytes());
        data[104..108].copy_from_slice(&2i32.to_le_bytes());
        data[128..128 + 18].copy_from_slice(b"weapon_machinegun\0");
        data[160..160 + 11].copy_from_slice(b"Machinegun\0");
        artifact.image.initialized_data = data;
        let weapons = q3_guest_weapons(&artifact, &mounts, None, false).unwrap();
        assert_eq!(weapons.len(), 1);
        assert_eq!(weapons[0].weapon, 2);
        assert_eq!(weapons[0].item, "q3:weapon/machinegun");
        assert_eq!(weapons[0].label, "Machinegun");
    }

    #[test]
    fn guest_weapons_reject_mismatched_and_live_declarations() {
        let mounts = mounted(
            "mismatch",
            &[(
                "qvm-items.json",
                r#"{"version": 1, "artifactDigest": "sha256:other", "items": {}}"#,
            )],
        );
        assert!(q3_guest_weapons(&stock_artifact(), &mounts, None, false).is_err());
        let live = QvmItemLayout {
            address: QvmItemAddress::Direct(64),
            count: QvmItemCount::Direct(1),
            live_source: true,
            stride: 52,
            fields: QvmItemFields {
                class_name: 0,
                pickup_name: 28,
                item_type: 36,
                tag: 40,
            },
            weapon_type: 1,
            ammo_type: 2,
        };
        let mounts = mounted("live", &[]);
        assert!(q3_guest_weapons(&stock_artifact(), &mounts, Some(live), false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn catalog_validates_ownership_and_storage() {
        let artifact = stock_artifact();
        let module = QvmModule::new(artifact.clone(), None, None).unwrap();
        let initial = BASE_Q3_GUEST_WEAPONS.clone();
        let mut catalog = Q3GuestCatalog::new(&artifact, &module, None, initial.clone(), None, None).unwrap();
        assert_eq!(catalog.weapons().unwrap(), initial);
        assert!(catalog.records().is_err());
        catalog.close();
        assert!(catalog.weapons().is_err());
        let foreign = fixture_artifact("sha256:other", QvmRole::Qagame, 64, &[]);
        let foreign_module = QvmModule::new(foreign, None, None).unwrap();
        assert!(Q3GuestCatalog::new(&artifact, &foreign_module, None, initial, None, None).is_err());
    }
}
