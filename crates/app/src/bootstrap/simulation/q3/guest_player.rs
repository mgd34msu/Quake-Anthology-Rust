//! Q3 guest player UI projection.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/q3/guest-player.ts`
//! (`q3GuestPlayerUi`).
//!
//! The QVM authority exposes the same public player state as its native
//! protocol clients. Counts are donor numbers (`f64` in the UI hub); the
//! weapon-bit and ammo-index reads use the guest arrays exactly like the
//! donor, with out-of-range reads defaulting to zero.

use qa_content::contract::{
    ArmorState, InventoryEntry, ItemId, PoweredProtectionState, ProviderReference, RegularArmorState,
};
use qa_content::q3::base::shared::definitions::{stat_schema, Product};
use qa_content::q3::guest_items::{Q3GuestWeapon, BASE_Q3_GUEST_WEAPONS, STANDARD_Q3_GUEST_WEAPONS};
use qa_guest::qvm::player_record::QvmPlayerState;

use super::super::arsenal::selected::{ArsenalAmmoWarning, WeaponAmmoStatus, WeaponHudStatus};
use super::super::arsenal::weapon_status::q3_arsenal_warning;
use super::super::powerup_timers::q3_public_powerup_timers;
use super::super::types::{PlayerUi, PlayerUiItem, PlayerUiItemKind, UiAmmo};

/// Active-weapon selection (donor `active?: ItemId | null`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3GuestActiveWeapon {
    /// Derive from the player weapon word (donor `undefined`).
    FromState,
    /// No active weapon (donor `null`).
    None,
    /// Force an item (donor item id).
    Item(ItemId),
}

/// Project guest player state to the player UI snapshot.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn q3_guest_player_ui(
    state: &QvmPlayerState,
    source: ProviderReference,
    now_milliseconds: i32,
    catalog: Option<&[Q3GuestWeapon]>,
    product: Product,
    inventory: Option<&[InventoryEntry]>,
    active: Q3GuestActiveWeapon,
) -> PlayerUi {
    let definitions: &[Q3GuestWeapon] = catalog.unwrap_or(if product == Product::Missionpack {
        STANDARD_Q3_GUEST_WEAPONS.as_slice()
    } else {
        BASE_Q3_GUEST_WEAPONS.as_slice()
    });
    let schema = stat_schema(product);
    let weapons_slot = schema.weapons();
    let weapon = match &active {
        Q3GuestActiveWeapon::FromState => definitions.iter().find(|value| value.weapon == state.weapon),
        Q3GuestActiveWeapon::None => None,
        Q3GuestActiveWeapon::Item(item) => definitions.iter().find(|value| &value.item == item),
    };
    let expected = match &active {
        Q3GuestActiveWeapon::FromState => state.weapon != 0,
        Q3GuestActiveWeapon::None => false,
        Q3GuestActiveWeapon::Item(_) => true,
    };
    if expected && weapon.is_none() {
        panic!(
            "Q3 guest weapon {} is missing from its item catalog; this mod needs a qvm-items.json declaration",
            state.weapon
        );
    }
    let count = |item: &ItemId| -> f64 {
        if let Some(inventory) = inventory {
            return inventory
                .iter()
                .find(|entry| &entry.item == item)
                .map_or(0.0, |entry| entry.count);
        }
        let Some(definition) = definitions
            .iter()
            .find(|value| &value.item == item || value.ammo.as_ref() == Some(item))
        else {
            return 0.0;
        };
        if &definition.item == item {
            let owned = state.stats.get(weapons_slot).copied().unwrap_or(0);
            if owned & 1i32.wrapping_shl(definition.weapon as u32) != 0 {
                1.0
            } else {
                0.0
            }
        } else {
            f64::from(
                usize::try_from(definition.weapon)
                    .ok()
                    .and_then(|index| state.ammo.get(index).copied())
                    .unwrap_or(0),
            )
        }
    };
    let armor = state.stats.get(schema.armor()).copied().unwrap_or(0);
    PlayerUi {
        selected_arsenal: false,
        native_inventory: None,
        powerups: q3_public_powerup_timers(
            |powerup| {
                usize::try_from(powerup)
                    .ok()
                    .and_then(|index| state.powerups.get(index).copied())
                    .unwrap_or(0)
            },
            now_milliseconds,
        ),
        health: f64::from(state.stats.get(schema.health()).copied().unwrap_or(0)),
        armor: ArmorState {
            powered: PoweredProtectionState::None,
            regular: if armor == 0 {
                RegularArmorState::None
            } else {
                RegularArmorState::Q3 {
                    points: f64::from(armor),
                    protection: f64::from(0.66f32),
                }
            },
        },
        active_weapon: weapon.map(|weapon| weapon.item.clone()),
        ammo: weapon.and_then(|weapon| {
            weapon.ammo.as_ref().map(|ammo| UiAmmo {
                item: ammo.clone(),
                count: count(ammo),
            })
        }),
        inventory: inventory.map_or_else(
            || {
                definitions
                    .iter()
                    .flat_map(|value| {
                        let mut entries = vec![InventoryEntry {
                            item: value.item.clone(),
                            count: count(&value.item),
                            capacity: 1.0,
                            count_policy: None,
                        }];
                        if let Some(ammo) = &value.ammo {
                            entries.push(InventoryEntry {
                                item: ammo.clone(),
                                count: count(ammo),
                                capacity: 200.0,
                                count_policy: None,
                            });
                        }
                        entries
                    })
                    .collect()
            },
            |inventory| inventory.to_vec(),
        ),
        weapon_status: weapon.map(|weapon| WeaponHudStatus {
            source: source.clone(),
            item: weapon.item.clone(),
            label: weapon.label.clone(),
            ammo: match &weapon.ammo {
                None => WeaponAmmoStatus::Unmetered,
                Some(ammo) if count(ammo) == -1.0 => WeaponAmmoStatus::Unmetered,
                Some(ammo) => WeaponAmmoStatus::Finite {
                    item: ammo.clone(),
                    count: count(ammo),
                    has_ammo_to_start: count(ammo) > 0.0,
                    low: false,
                },
            },
        }),
        arsenal_warning: if catalog.is_none() {
            q3_arsenal_warning(product, |item| count(item) as i32)
        } else {
            ArsenalAmmoWarning::None
        },
        items: definitions
            .iter()
            .map(|value| PlayerUiItem {
                id: value.item.clone(),
                label: value.label.clone(),
                kind: PlayerUiItemKind::Weapon,
                source_ordinal: f64::from(value.weapon),
                owned: count(&value.item) > 0.0,
                has_ammo: value.ammo.as_ref().is_none_or(|ammo| count(ammo) != 0.0),
                count: value.ammo.as_ref().map(&count),
                warning_count: 0.0,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use qa_content::contract::{ContentId, ProviderReference};
    use qa_core::identity::ProviderId;
    use qa_guest::qvm::player_record::QvmPlayerState;

    use super::*;

    fn source() -> ProviderReference {
        ProviderReference {
            provider: ProviderId::new("q3", "guest"),
            content: ContentId("q3:guest".to_string()),
        }
    }

    fn state() -> QvmPlayerState {
        QvmPlayerState {
            command_time_ms: 0,
            movement_type: 0,
            bob_cycle: 0,
            movement_flags: 0,
            movement_time_ms: 0,
            origin: qa_core::math::vec3(0.0, 0.0, 0.0),
            velocity: qa_core::math::vec3(0.0, 0.0, 0.0),
            weapon_time_ms: 0,
            gravity: 800,
            speed: 320,
            delta_angle_words: [0, 0, 0],
            ground_entity_number: 1023,
            legs_timer_ms: 0,
            legs_animation: 0,
            torso_timer_ms: 0,
            torso_animation: 0,
            movement_direction: 0,
            grapple_point: qa_core::math::vec3(0.0, 0.0, 0.0),
            flags: 0,
            event_sequence: 0,
            events: [0, 0],
            event_parameters: [0, 0],
            external_event: 0,
            external_event_parameter: 0,
            external_event_time_ms: 0,
            client_number: 0,
            weapon: 2,
            weapon_state: 0,
            view_angles: qa_core::math::vec3(0.0, 0.0, 0.0),
            view_height: 26,
            damage_event: 0,
            damage_yaw: 0,
            damage_pitch: 0,
            damage_count: 0,
            stats: [100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            persistent: [0; 16],
            powerups: [0; 16],
            ammo: [0, 0, 50, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            generic1: 0,
            loop_sound: 0,
            jump_pad_entity: 0,
            ping_ms: 0,
            movement_frame_count: 0,
            jump_pad_frame: 0,
            entity_event_sequence: 0,
        }
    }

    #[test]
    fn projects_gauntlet_and_machinegun() {
        let mut state = state();
        state.stats[2] = (1 << 1) | (1 << 2);
        let ui = q3_guest_player_ui(
            &state,
            source(),
            1000,
            None,
            Product::Baseq3,
            None,
            Q3GuestActiveWeapon::FromState,
        );
        assert_eq!(ui.health, 100.0);
        assert!(matches!(ui.armor.regular, RegularArmorState::None));
        assert_eq!(ui.active_weapon.as_deref(), Some("q3:weapon/machinegun"));
        assert_eq!(ui.ammo.as_ref().map(|ammo| ammo.count), Some(50.0));
        assert!(ui
            .items
            .iter()
            .any(|item| item.owned && item.id == "q3:weapon/gauntlet"));
        assert_eq!(ui.arsenal_warning, ArsenalAmmoWarning::None);
    }

    #[test]
    fn inventory_override_and_forced_weapon() {
        let state = state();
        let shells: ItemId = "q3:ammo/shotgun".to_string();
        let inventory = vec![InventoryEntry {
            item: shells.clone(),
            count: 7.0,
            capacity: 200.0,
            count_policy: None,
        }];
        let ui = q3_guest_player_ui(
            &state,
            source(),
            1000,
            None,
            Product::Baseq3,
            Some(&inventory),
            Q3GuestActiveWeapon::Item("q3:weapon/shotgun".to_string()),
        );
        assert_eq!(ui.active_weapon.as_deref(), Some("q3:weapon/shotgun"));
        assert_eq!(ui.inventory.len(), 1);
        assert_eq!(ui.ammo.as_ref().map(|ammo| ammo.count), Some(7.0));
    }

    #[test]
    fn armor_projects_q3_layer() {
        let mut state = state();
        state.stats[3] = 50;
        let ui = q3_guest_player_ui(
            &state,
            source(),
            1000,
            None,
            Product::Baseq3,
            None,
            Q3GuestActiveWeapon::None,
        );
        assert!(matches!(
            ui.armor.regular,
            RegularArmorState::Q3 { points, .. } if points == 50.0
        ));
        assert_eq!(ui.active_weapon, None);
        assert_eq!(ui.weapon_status, None);
    }

    #[test]
    #[should_panic(expected = "missing from its item catalog")]
    fn missing_weapon_panics() {
        let mut state = state();
        state.weapon = 99;
        let _ = q3_guest_player_ui(
            &state,
            source(),
            1000,
            None,
            Product::Baseq3,
            None,
            Q3GuestActiveWeapon::FromState,
        );
    }
}
