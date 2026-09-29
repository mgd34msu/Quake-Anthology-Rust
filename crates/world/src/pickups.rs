//! Pickup grant previews ported from `src/world/gameplay/pickups.ts`. A
//! source-resolved grant plan previews against admitted entries; eligibility
//! and missing-entry admission remain source-owned.

use std::collections::HashMap;

use crate::combat::ItemId;
use crate::inventory::{inventory_give, GiveTransition, InventoryEntry};
use crate::WorldError;

/// One ammo grant.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupAmmoGrant {
    /// Destination item.
    pub item: ItemId,
    /// Granted amount.
    pub amount: f64,
}

/// One grant receipt.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupAmmoReceipt {
    /// Destination item.
    pub item: ItemId,
    /// Count before the grant.
    pub before: f64,
    /// Count delta.
    pub given: f64,
}

/// Ammo-plan acceptance rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmmoAcceptance {
    /// Any positive grant accepts.
    Positive,
    /// Any nonzero grant accepts.
    Nonzero,
}

/// Weapon side of an ammo plan.
#[derive(Debug, Clone, PartialEq)]
pub enum AmmoWeapons {
    /// Grant weapons after acceptance.
    Grant {
        /// Weapon grants.
        grants: Vec<PickupAmmoGrant>,
    },
    /// Share ammo receipts with these weapons.
    SharedAmmo {
        /// Weapon items.
        items: Vec<ItemId>,
    },
}

/// Source-resolved grant plan.
#[derive(Debug, Clone, PartialEq)]
pub enum PickupGrantPlan {
    /// Weapon pickup: always accepted.
    Weapon {
        /// Weapon grants.
        weapons: Vec<PickupAmmoGrant>,
        /// Ammo grants.
        ammo: Vec<PickupAmmoGrant>,
    },
    /// Ammo pickup: accepted when ammo lands.
    Ammo {
        /// Acceptance rule.
        acceptance: AmmoAcceptance,
        /// Ammo grants.
        ammo: Vec<PickupAmmoGrant>,
        /// Weapon side.
        weapons: AmmoWeapons,
    },
}

/// Grant preview.
#[derive(Debug, Clone, PartialEq)]
pub struct PickupSupplyPreview {
    /// Whether the pickup is accepted.
    pub accepted: bool,
    /// Weapon receipts.
    pub weapons: Vec<PickupAmmoReceipt>,
    /// Ammo receipts.
    pub ammo: Vec<PickupAmmoReceipt>,
}

/// Preview pickup grants against admitted entries.
pub fn preview_pickup_grants(
    inventory: &[InventoryEntry],
    plan: &PickupGrantPlan,
) -> Result<PickupSupplyPreview, WorldError> {
    let weapon_items: Vec<ItemId> = match plan {
        PickupGrantPlan::Weapon { weapons, .. } => weapons.iter().map(|grant| grant.item.clone()).collect(),
        PickupGrantPlan::Ammo { weapons, .. } => match weapons {
            AmmoWeapons::Grant { grants } => grants.iter().map(|grant| grant.item.clone()).collect(),
            AmmoWeapons::SharedAmmo { items } => items.clone(),
        },
    };
    let ammo_items: Vec<ItemId> = match plan {
        PickupGrantPlan::Weapon { ammo, .. } | PickupGrantPlan::Ammo { ammo, .. } => {
            ammo.iter().map(|grant| grant.item.clone()).collect()
        }
    };
    let mut entries: HashMap<ItemId, InventoryEntry> = HashMap::new();
    for entry in inventory {
        entries.insert(entry.item.clone(), entry.clone());
    }
    for item in weapon_items.iter().chain(ammo_items.iter()) {
        if !entries.contains_key(item) {
            return Err(WorldError::PickupAdmission(item.clone()));
        }
    }
    if let PickupGrantPlan::Ammo {
        weapons: AmmoWeapons::SharedAmmo { items },
        ammo,
        ..
    } = plan
    {
        for item in items {
            if !ammo.iter().any(|grant| &grant.item == item) {
                return Err(WorldError::PickupPlan(format!(
                    "Shared weapon {item} has no ammo grant"
                )));
            }
        }
    }
    let mut give = |grant: &PickupAmmoGrant| -> Result<PickupAmmoReceipt, WorldError> {
        let entry = entries
            .get(&grant.item)
            .cloned()
            .ok_or_else(|| WorldError::PickupAdmission(grant.item.clone()))?;
        let before = entry.count;
        match inventory_give(&entry, grant.amount)? {
            GiveTransition::Unchanged => Ok(PickupAmmoReceipt {
                item: grant.item.clone(),
                before,
                given: 0.0,
            }),
            GiveTransition::Write { entry, given } => {
                entries.insert(grant.item.clone(), entry);
                Ok(PickupAmmoReceipt {
                    item: grant.item.clone(),
                    before,
                    given,
                })
            }
        }
    };
    match plan {
        PickupGrantPlan::Weapon { weapons, ammo } => {
            let mut receipts = Vec::with_capacity(weapons.len());
            for grant in weapons {
                receipts.push(give(grant)?);
            }
            let mut ammo_receipts = Vec::with_capacity(ammo.len());
            for grant in ammo {
                ammo_receipts.push(give(grant)?);
            }
            Ok(PickupSupplyPreview {
                accepted: true,
                weapons: receipts,
                ammo: ammo_receipts,
            })
        }
        PickupGrantPlan::Ammo {
            acceptance,
            ammo,
            weapons,
        } => {
            let mut ammo_receipts = Vec::with_capacity(ammo.len());
            for grant in ammo {
                ammo_receipts.push(give(grant)?);
            }
            let accepted = ammo_receipts.iter().any(|receipt| match acceptance {
                AmmoAcceptance::Positive => receipt.given > 0.0,
                AmmoAcceptance::Nonzero => receipt.given != 0.0,
            });
            let weapon_receipts = if !accepted {
                Vec::new()
            } else {
                match weapons {
                    AmmoWeapons::Grant { grants } => {
                        let mut receipts = Vec::with_capacity(grants.len());
                        for grant in grants {
                            if let Some(receipt) = ammo_receipts.iter().find(|receipt| receipt.item == grant.item) {
                                receipts.push(receipt.clone());
                            } else {
                                receipts.push(give(grant)?);
                            }
                        }
                        receipts
                    }
                    AmmoWeapons::SharedAmmo { items } => ammo_receipts
                        .iter()
                        .filter(|receipt| items.contains(&receipt.item))
                        .cloned()
                        .collect(),
                }
            };
            Ok(PickupSupplyPreview {
                accepted,
                weapons: weapon_receipts,
                ammo: ammo_receipts,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(item: &str, count: f64, capacity: f64) -> InventoryEntry {
        InventoryEntry {
            item: item.to_string(),
            count,
            capacity,
            count_policy: None,
        }
    }

    fn grant(item: &str, amount: f64) -> PickupAmmoGrant {
        PickupAmmoGrant {
            item: item.to_string(),
            amount,
        }
    }

    #[test]
    fn weapon_plans_always_accept() {
        let inventory = vec![entry("q3:shotgun", 0.0, 1.0), entry("q3:shells", 0.0, 10.0)];
        let preview = preview_pickup_grants(
            &inventory,
            &PickupGrantPlan::Weapon {
                weapons: vec![grant("q3:shotgun", 1.0)],
                ammo: vec![grant("q3:shells", 8.0)],
            },
        )
        .unwrap();
        assert!(preview.accepted);
        assert_eq!(preview.weapons[0].given, 1.0);
        assert_eq!(preview.ammo[0].given, 8.0);
    }

    #[test]
    fn ammo_plans_accept_only_when_ammo_lands() {
        let full = vec![entry("q3:shells", 10.0, 10.0)];
        let preview = preview_pickup_grants(
            &full,
            &PickupGrantPlan::Ammo {
                acceptance: AmmoAcceptance::Positive,
                ammo: vec![grant("q3:shells", 5.0)],
                weapons: AmmoWeapons::Grant { grants: vec![] },
            },
        )
        .unwrap();
        assert!(!preview.accepted);
        assert!(preview.weapons.is_empty());
        let room = vec![entry("q3:shells", 5.0, 10.0)];
        let preview = preview_pickup_grants(
            &room,
            &PickupGrantPlan::Ammo {
                acceptance: AmmoAcceptance::Positive,
                ammo: vec![grant("q3:shells", 5.0)],
                weapons: AmmoWeapons::Grant { grants: vec![] },
            },
        )
        .unwrap();
        assert!(preview.accepted);
    }

    #[test]
    fn unadmitted_destinations_and_shared_plans_fail() {
        let inventory = vec![entry("q3:shells", 0.0, 10.0)];
        let missing = preview_pickup_grants(
            &inventory,
            &PickupGrantPlan::Weapon {
                weapons: vec![grant("q3:shotgun", 1.0)],
                ammo: vec![],
            },
        );
        assert_eq!(missing, Err(WorldError::PickupAdmission("q3:shotgun".to_string())));
        let shared = preview_pickup_grants(
            &inventory,
            &PickupGrantPlan::Ammo {
                acceptance: AmmoAcceptance::Positive,
                ammo: vec![grant("q3:shells", 5.0)],
                weapons: AmmoWeapons::SharedAmmo {
                    items: vec!["q3:shotgun".to_string()],
                },
            },
        );
        assert!(matches!(shared, Err(WorldError::PickupAdmission(_))));
    }
}
