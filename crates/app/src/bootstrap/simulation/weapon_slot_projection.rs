//! Weapon slot presentation projection.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/weapon-slot-projection.ts`
//! (`projectWeaponSlot`).
//!
//! The outgoing weapon remains visible through its own drop animation.

use qa_content::contract::{ItemId, ProviderReference};
use qa_core::identity::ProviderId;

use super::arsenal::selected::{WeaponAmmoStatus, WeaponHudStatus};
use super::types::{PlayerUi, PlayerUiItem, SimulationPresentation};
use super::weapon_slot::{WeaponReference, WeaponSlotState};

/// Equipment-owned weapon projection feeding the slot projection.
#[derive(Debug, Clone, PartialEq)]
pub struct EquipmentWeaponProjection {
    /// Owning source.
    pub source: ProviderReference,
    /// Owned weapon.
    pub weapon: WeaponReference,
    /// HUD item row.
    pub item: PlayerUiItem,
    /// View model presentation.
    pub model: Option<SimulationPresentation>,
}

/// Projected weapon slot state.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponSlotProjection {
    /// Visible active weapon.
    pub active: Option<WeaponReference>,
    /// Visible pending weapon.
    pub pending: Option<WeaponReference>,
    /// Projected HUD.
    pub ui: PlayerUi,
    /// Visible view model.
    pub model: Option<SimulationPresentation>,
}

/// Primary arsenal projection: the slot projection it feeds.
pub type PrimaryWeaponProjection = WeaponSlotProjection;

/// One source-owned weapon projection.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceWeaponProjection {
    /// Owning provider.
    pub provider: ProviderId,
    /// Source active weapon.
    pub active: Option<WeaponReference>,
    /// Source pending weapon.
    pub pending: Option<WeaponReference>,
    /// Source HUD item rows.
    pub items: Vec<PlayerUiItem>,
    /// Source weapon status.
    pub weapon_status: Option<WeaponHudStatus>,
    /// Source ammo readout.
    pub ammo: Option<super::types::UiAmmo>,
    /// Source view model.
    pub model: Option<SimulationPresentation>,
}

/// Project the visible weapon slot over primary, equipment, and sources.
pub fn project_weapon_slot(
    state: &WeaponSlotState,
    primary: &PrimaryWeaponProjection,
    equipment: Option<&EquipmentWeaponProjection>,
    sources: &[SourceWeaponProjection],
) -> WeaponSlotProjection {
    let mut projections: Vec<SourceWeaponProjection> = sources.to_vec();
    if let Some(equipment) = equipment {
        projections.push(SourceWeaponProjection {
            provider: equipment.weapon.provider.clone(),
            active: Some(equipment.weapon.clone()),
            pending: None,
            items: vec![equipment.item.clone()],
            model: equipment.model.clone(),
            ammo: None,
            weapon_status: Some(WeaponHudStatus {
                source: equipment.source.clone(),
                item: equipment.weapon.item.clone(),
                label: equipment.item.label.clone(),
                ammo: WeaponAmmoStatus::Unmetered,
            }),
        });
    }
    let provider = match state {
        WeaponSlotState::Active { provider } => provider,
        WeaponSlotState::Switching { from, .. } | WeaponSlotState::Activating { from, .. } => from,
    };
    let visible = projections.iter().find(|source| &source.provider == provider);
    // Donor Map semantics: later rows overwrite by id, keeping first position.
    let mut order: Vec<ItemId> = Vec::new();
    let mut supplied: std::collections::HashMap<ItemId, PlayerUiItem> = std::collections::HashMap::new();
    for source in &projections {
        for item in &source.items {
            if !supplied.contains_key(&item.id) {
                order.push(item.id.clone());
            }
            supplied.insert(item.id.clone(), item.clone());
        }
    }
    let mut items: Vec<PlayerUiItem> = primary
        .ui
        .items
        .iter()
        .filter(|item| !supplied.contains_key(&item.id))
        .cloned()
        .collect();
    for id in &order {
        if let Some(item) = supplied.get(id) {
            items.push(item.clone());
        }
    }
    let pending = match state {
        WeaponSlotState::Active { .. } => match visible {
            None => primary.pending.clone(),
            Some(visible) => visible.pending.clone(),
        },
        WeaponSlotState::Switching { next, .. } | WeaponSlotState::Activating { next, .. } => Some(next.clone()),
    };
    match visible {
        None => WeaponSlotProjection {
            active: primary.active.clone(),
            pending,
            ui: PlayerUi {
                items,
                ..primary.ui.clone()
            },
            model: primary.model.clone(),
        },
        Some(visible) => WeaponSlotProjection {
            active: visible.active.clone(),
            pending,
            ui: PlayerUi {
                items,
                weapon_status: visible.weapon_status.clone(),
                active_weapon: visible.active.as_ref().map(|active| active.item.clone()),
                ammo: visible.ammo.clone(),
                ..primary.ui.clone()
            },
            model: visible.model.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use qa_content::contract::{ArmorState, ContentId, PoweredProtectionState, ProviderReference, RegularArmorState};

    use super::super::arsenal::selected::ArsenalAmmoWarning;
    use super::super::types::{PlayerUiItemKind, UiAmmo};
    use super::*;

    fn provider(namespace: &str, name: &str) -> ProviderId {
        ProviderId::new(namespace, name)
    }

    fn weapon(namespace: &str, name: &str, item: &str) -> WeaponReference {
        WeaponReference {
            provider: provider(namespace, name),
            item: item.to_string(),
        }
    }

    fn item(id: &str, label: &str) -> PlayerUiItem {
        PlayerUiItem {
            id: id.to_string(),
            label: label.to_string(),
            kind: PlayerUiItemKind::Weapon,
            source_ordinal: 0.0,
            owned: true,
            has_ammo: true,
            count: None,
            warning_count: 0.0,
        }
    }

    fn ui(items: Vec<PlayerUiItem>) -> PlayerUi {
        PlayerUi {
            selected_arsenal: true,
            native_inventory: None,
            powerups: Vec::new(),
            weapon_status: None,
            arsenal_warning: ArsenalAmmoWarning::None,
            health: 100.0,
            armor: ArmorState {
                regular: RegularArmorState::None,
                powered: PoweredProtectionState::None,
            },
            active_weapon: None,
            ammo: None,
            inventory: Vec::new(),
            items,
        }
    }

    fn primary() -> PrimaryWeaponProjection {
        WeaponSlotProjection {
            active: Some(weapon("q2", "base", "q2:weapon_blaster")),
            pending: None,
            ui: ui(vec![item("q2:weapon_blaster", "Blaster")]),
            model: None,
        }
    }

    #[test]
    fn active_primary_shows_through_without_sources() {
        let primary = primary();
        let state = WeaponSlotState::Active {
            provider: provider("q2", "base"),
        };
        let projected = project_weapon_slot(&state, &primary, None, &[]);
        assert_eq!(projected.active, primary.active);
        assert_eq!(projected.ui.items, primary.ui.items);
        assert!(projected.model.is_none());
    }

    #[test]
    fn visible_source_replaces_hud_rows() {
        let primary = primary();
        let state = WeaponSlotState::Active {
            provider: provider("q2", "ctf"),
        };
        let sources = vec![SourceWeaponProjection {
            provider: provider("q2", "ctf"),
            active: Some(weapon("q2", "ctf", "q2:weapon_grapple")),
            pending: None,
            items: vec![item("q2:weapon_grapple", "Grapple")],
            weapon_status: None,
            ammo: Some(UiAmmo {
                item: "q2:ammo_cells".to_string(),
                count: 50.0,
            }),
            model: None,
        }];
        let projected = project_weapon_slot(&state, &primary, None, &sources);
        assert_eq!(projected.active, Some(weapon("q2", "ctf", "q2:weapon_grapple")));
        assert_eq!(
            projected
                .ui
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["q2:weapon_blaster", "q2:weapon_grapple"]
        );
        assert_eq!(projected.ui.active_weapon, Some("q2:weapon_grapple".to_string()));
        assert_eq!(projected.ui.ammo.expect("ammo").count, 50.0);
    }

    #[test]
    fn switching_keeps_outgoing_visible_with_next_pending() {
        let primary = primary();
        let state = WeaponSlotState::Switching {
            from: provider("q2", "base"),
            next: weapon("q2", "ctf", "q2:weapon_grapple"),
        };
        let projected = project_weapon_slot(&state, &primary, None, &[]);
        assert_eq!(projected.active, primary.active);
        assert_eq!(projected.pending, Some(weapon("q2", "ctf", "q2:weapon_grapple")));
    }

    #[test]
    fn equipment_source_is_unmetered() {
        let primary = primary();
        let state = WeaponSlotState::Active {
            provider: provider("q2", "rogue"),
        };
        let equipment = EquipmentWeaponProjection {
            source: ProviderReference {
                provider: provider("q2", "rogue"),
                content: ContentId("q2:re:rogue:1".to_string()),
            },
            weapon: weapon("q2", "rogue", "q2:weapon_hook"),
            item: item("q2:weapon_hook", "Hook"),
            model: None,
        };
        let projected = project_weapon_slot(&state, &primary, Some(&equipment), &[]);
        assert_eq!(projected.active, Some(weapon("q2", "rogue", "q2:weapon_hook")));
        assert_eq!(
            projected.ui.weapon_status.expect("status").ammo,
            WeaponAmmoStatus::Unmetered
        );
        assert!(projected.ui.ammo.is_none());
    }
}
