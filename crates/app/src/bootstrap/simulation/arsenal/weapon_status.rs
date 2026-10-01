//! Per-family weapon HUD status and aggregate Q3 ammo warnings.
//!
//! Port of donor `src/app/bootstrap/simulation/arsenal/weapon-status.ts`
//! (`q1WeaponStatus`, `q2WeaponStatus`, `q3WeaponStatus`, `q3ArsenalWarning`,
//! plus the `q1WeaponDisplayName` re-export).

use qa_content::contract::{ItemId, ProviderReference};
use qa_content::q1::foundation::entity_services::{Q1EntityServices, Q1WeaponPurpose};
use qa_content::q1::foundation::types::Q1Weapon;
pub use qa_content::q1::foundation::weapon_names::q1_weapon_display_name;
use qa_content::q2::foundation::items::q2_base_weapon_display_name;
use qa_content::q2::foundation::weapons::types::Q2WeaponDefinition;
use qa_content::q3::base::shared::definitions::{Product, Weapon};
use qa_content::q3::base::shared::items::{item_list, ItemKind};
use qa_content::q3::foundation::arsenal::Q3_WEAPON_ITEMS;
use qa_core::identity::ActorId;

use super::selected::{ArsenalAmmoWarning, WeaponAmmoStatus, WeaponHudStatus};

/// Q1 services read by weapon status. The donor calls these `game` methods
/// directly; the trait keeps HUD reads testable without a full foundation.
pub trait Q1WeaponStatusSource {
    /// Weapon item for the selected weapon.
    fn status_weapon_item(&self, weapon: Q1Weapon) -> ItemId;
    /// Ammo item for the selected weapon, when metered.
    fn status_weapon_ammo(&self, weapon: Q1Weapon) -> Option<ItemId>;
    /// Inventory count of an item.
    fn status_ammo_count(&self, actor: &ActorId, item: &ItemId) -> f64;
    /// Whether the weapon can start firing.
    fn status_can_fire(&mut self, actor: &ActorId, weapon: Q1Weapon) -> bool;
}

impl Q1WeaponStatusSource for Q1EntityServices {
    fn status_weapon_item(&self, weapon: Q1Weapon) -> ItemId {
        self.weapon_item(weapon)
    }

    fn status_weapon_ammo(&self, weapon: Q1Weapon) -> Option<ItemId> {
        self.weapon_ammo(weapon)
    }

    fn status_ammo_count(&self, actor: &ActorId, item: &ItemId) -> f64 {
        self.host.inventory.count(actor, item)
    }

    fn status_can_fire(&mut self, actor: &ActorId, weapon: Q1Weapon) -> bool {
        self.weapon_available(actor, weapon, Q1WeaponPurpose::Fire, None)
            .unwrap_or(false)
    }
}

fn display_name(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut boundary = true;
    for ch in value.chars() {
        let ch = if ch == '_' || ch == '-' { ' ' } else { ch };
        if ch.is_alphanumeric() {
            if boundary {
                out.extend(ch.to_uppercase());
            } else {
                out.push(ch);
            }
            boundary = false;
        } else {
            out.push(ch);
            boundary = true;
        }
    }
    out
}

/// Reads the active Q1 weapon HUD row. The donor takes the player record; only
/// the actor handle and selected weapon are read, so callers pass both.
pub fn q1_weapon_status(
    game: &mut impl Q1WeaponStatusSource,
    actor: &ActorId,
    weapon: Q1Weapon,
    source: ProviderReference,
) -> WeaponHudStatus {
    let item = game.status_weapon_item(weapon);
    let ammo = game.status_weapon_ammo(weapon);
    WeaponHudStatus {
        source,
        item,
        label: q1_weapon_display_name(weapon),
        ammo: match ammo {
            None => WeaponAmmoStatus::Unmetered,
            Some(ammo) => {
                let count = game.status_ammo_count(actor, &ammo);
                let has_ammo_to_start = game.status_can_fire(actor, weapon);
                WeaponAmmoStatus::Finite {
                    item: ammo,
                    count,
                    has_ammo_to_start,
                    low: false,
                }
            }
        },
    }
}

/// Reads the active Q2 weapon HUD row, if a weapon is selected.
pub fn q2_weapon_status(
    definition: Option<&Q2WeaponDefinition>,
    count: impl Fn(&ItemId) -> i32,
    source: ProviderReference,
) -> Option<WeaponHudStatus> {
    let definition = definition?;
    let label = q2_base_weapon_display_name(&definition.item).unwrap_or_else(|| display_name(&definition.name));
    Some(WeaponHudStatus {
        source,
        item: definition.item.clone(),
        label,
        ammo: match &definition.ammo {
            None => WeaponAmmoStatus::Unmetered,
            Some(ammo) => {
                let held = count(ammo);
                WeaponAmmoStatus::Finite {
                    item: ammo.clone(),
                    count: f64::from(held),
                    has_ammo_to_start: held >= definition.quantity,
                    low: held <= definition.warning,
                }
            }
        },
    })
}

/// Reads the active Q3 weapon HUD row, if the item belongs to the product.
pub fn q3_weapon_status(
    item: Option<&ItemId>,
    product: Product,
    count: impl Fn(&ItemId) -> i32,
    source: ProviderReference,
) -> Option<WeaponHudStatus> {
    let item = item?;
    let definition = Q3_WEAPON_ITEMS.iter().find(|entry| {
        &entry.item == item
            && (product == Product::Missionpack || entry.weapon as i32 <= Weapon::WpGrapplingHook as i32)
    })?;
    let label = item_list(product)
        .iter()
        .find(|row| matches!(row.kind, ItemKind::Weapon(w) if w as i32 == definition.weapon as i32))
        .and_then(|row| row.pickup_name)
        .map(str::to_string)
        .unwrap_or_else(|| display_name(item.strip_prefix("q3:weapon/").unwrap_or(item)));
    Some(WeaponHudStatus {
        source,
        item: definition.item.clone(),
        label,
        ammo: match &definition.ammo {
            None => WeaponAmmoStatus::Unmetered,
            Some(ammo) => {
                let held = count(ammo);
                if held == -1 {
                    WeaponAmmoStatus::Unmetered
                } else {
                    WeaponAmmoStatus::Finite {
                        item: ammo.clone(),
                        count: f64::from(held),
                        has_ammo_to_start: held > 0,
                        low: false,
                    }
                }
            }
        },
    })
}

/// `CG_CheckAmmo`'s aggregate five-second estimate, independent of the active weapon.
pub fn q3_arsenal_warning(product: Product, count: impl Fn(&ItemId) -> i32) -> ArsenalAmmoWarning {
    let mut total: i32 = 0;
    for entry in Q3_WEAPON_ITEMS.iter() {
        let weapon = entry.weapon as i32;
        if weapon < Weapon::WpMachinegun as i32
            || (product == Product::Baseq3 && weapon > Weapon::WpGrapplingHook as i32)
            || count(&entry.item) <= 0
        {
            continue;
        }
        let slow = matches!(
            entry.weapon,
            Weapon::WpRocketLauncher | Weapon::WpGrenadeLauncher | Weapon::WpRailgun | Weapon::WpShotgun
        ) || (product == Product::Missionpack && entry.weapon == Weapon::WpProxLauncher);
        let rounds = match &entry.ammo {
            None => -1,
            Some(ammo) => count(ammo),
        };
        total = total.wrapping_add(rounds.wrapping_mul(if slow { 1000 } else { 200 }));
        if total >= 5000 {
            return ArsenalAmmoWarning::None;
        }
    }
    if total == 0 {
        ArsenalAmmoWarning::Empty
    } else {
        ArsenalAmmoWarning::Low
    }
}

#[cfg(test)]
mod tests {
    use qa_content::contract::{ContentId, ProviderReference};
    use qa_core::identity::{IdentityOwner, ProviderId};

    use super::*;

    fn source() -> ProviderReference {
        ProviderReference {
            provider: ProviderId::new("sim", "test"),
            content: ContentId("q1:id1:quake:1".to_string()),
        }
    }

    struct FakeQ1 {
        item: ItemId,
        ammo: Option<ItemId>,
        count: f64,
        can_fire: bool,
    }

    impl Q1WeaponStatusSource for FakeQ1 {
        fn status_weapon_item(&self, _weapon: Q1Weapon) -> ItemId {
            self.item.clone()
        }

        fn status_weapon_ammo(&self, _weapon: Q1Weapon) -> Option<ItemId> {
            self.ammo.clone()
        }

        fn status_ammo_count(&self, _actor: &ActorId, _item: &ItemId) -> f64 {
            self.count
        }

        fn status_can_fire(&mut self, _actor: &ActorId, _weapon: Q1Weapon) -> bool {
            self.can_fire
        }
    }

    fn q2_definition(item: &str, name: &str, ammo: Option<&str>) -> Q2WeaponDefinition {
        Q2WeaponDefinition {
            name: name.to_string(),
            item: item.to_string(),
            classname: "weapon_test".to_string(),
            ammo: ammo.map(str::to_string),
            quantity: 1,
            warning: 10,
            view_model: String::new(),
            world_model: String::new(),
            player_model: 0,
            activate_last: 0,
            fire_last: 0,
            idle_last: 0,
            deactivate_last: 0,
            pauses: Vec::new(),
            fires: Vec::new(),
            repeating: false,
        }
    }

    #[test]
    fn q1_metered_and_unmetered_rows() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut game = FakeQ1 {
            item: "q1:weapon/shotgun".to_string(),
            ammo: Some("q1:ammo/shells".to_string()),
            count: 12.0,
            can_fire: true,
        };
        let status = q1_weapon_status(&mut game, &actor, Q1Weapon::Shotgun, source());
        assert_eq!(status.item, "q1:weapon/shotgun");
        assert!(!status.label.is_empty());
        assert!(matches!(
            status.ammo,
            WeaponAmmoStatus::Finite { count, has_ammo_to_start: true, low: false, .. }
            if count == 12.0
        ));
        game.ammo = None;
        let status = q1_weapon_status(&mut game, &actor, Q1Weapon::Axe, source());
        assert!(matches!(status.ammo, WeaponAmmoStatus::Unmetered));
    }

    #[test]
    fn q2_thresholds_and_display_fallback() {
        let definition = q2_definition("q2:weapon/test_gun", "test_gun", Some("q2:ammo/test"));
        let status = q2_weapon_status(Some(&definition), |_| 5, source()).unwrap();
        assert_eq!(status.label, "Test Gun");
        assert!(matches!(
            status.ammo,
            WeaponAmmoStatus::Finite {
                has_ammo_to_start: true,
                low: true,
                ..
            }
        ));
        let status = q2_weapon_status(Some(&definition), |_| 50, source()).unwrap();
        assert!(matches!(
            status.ammo,
            WeaponAmmoStatus::Finite {
                has_ammo_to_start: true,
                low: false,
                ..
            }
        ));
        let status = q2_weapon_status(Some(&definition), |_| 0, source()).unwrap();
        assert!(matches!(
            status.ammo,
            WeaponAmmoStatus::Finite {
                has_ammo_to_start: false,
                low: true,
                ..
            }
        ));
        assert!(q2_weapon_status(None, |_| 5, source()).is_none());
        let unmetered = q2_definition("q2:weapon/blaster", "blaster", None);
        let status = q2_weapon_status(Some(&unmetered), |_| 0, source()).unwrap();
        assert!(matches!(status.ammo, WeaponAmmoStatus::Unmetered));
    }

    #[test]
    fn q3_product_gating_and_unmetered_marker() {
        let item = "q3:weapon/machinegun".to_string();
        let status = q3_weapon_status(Some(&item), Product::Baseq3, |_| 100, source()).unwrap();
        assert!(matches!(
            status.ammo,
            WeaponAmmoStatus::Finite {
                has_ammo_to_start: true,
                ..
            }
        ));
        let status = q3_weapon_status(Some(&item), Product::Baseq3, |_| -1, source()).unwrap();
        assert!(matches!(status.ammo, WeaponAmmoStatus::Unmetered));
        let prox = "q3:weapon/proxlauncher".to_string();
        assert!(q3_weapon_status(Some(&prox), Product::Baseq3, |_| 10, source()).is_none());
        assert!(q3_weapon_status(Some(&prox), Product::Missionpack, |_| 10, source()).is_some());
        let missing = "q3:weapon/nope".to_string();
        assert!(q3_weapon_status(Some(&missing), Product::Baseq3, |_| 10, source()).is_none());
        assert!(q3_weapon_status(None, Product::Baseq3, |_| 10, source()).is_none());
    }

    #[test]
    fn q3_warning_thresholds() {
        assert_eq!(q3_arsenal_warning(Product::Baseq3, |_| 0), ArsenalAmmoWarning::Empty);
        assert_eq!(q3_arsenal_warning(Product::Baseq3, |_| 100), ArsenalAmmoWarning::None);
        let warning = q3_arsenal_warning(Product::Baseq3, |item| {
            i32::from(item == "q3:weapon/machinegun" || item == "q3:ammo/machinegun")
        });
        assert_eq!(warning, ArsenalAmmoWarning::Low);
    }
}
