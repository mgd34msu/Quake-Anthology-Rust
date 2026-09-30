//! Level travel state (`src/content/q1/base/travel.ts`).
//!
//! client.qc SetNewParms/SetChangeParms/DecodeLevelParms.
//! GPL-2.0-or-later.
//!
//! Player travel extensions (`Q1PlayerExtension::capture_travel` /
//! `restore_travel`) are registered on the game, whose extension table
//! has no read accessor in the frozen foundation. Capture therefore
//! reports no extensions and admission reports the donor
//! missing-extension failure for any carried extension id, exactly as
//! the donor does when no extension defines those hooks. Wiring the
//! extension loop needs a foundation accessor; see the lane report.

use qa_core::identity::OwnedActor;

use crate::contract::{ArmorState, InventoryEntry, ItemId, PoweredProtectionState, RegularArmorState};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{weapon_item, Q1Edition, Q1FoundationOptions, Q1Weapon, WEAPONS};
use crate::q1::{q1_error, Q1Error};

/// Captured player travel extension (`Q1TravelState["extensions"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1TravelExtension {
    /// Extension id.
    pub id: String,
    /// Captured bytes.
    pub bytes: Vec<u8>,
}

/// Player state carried across a level change (`Q1TravelState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1TravelState {
    /// Health.
    pub health: f64,
    /// Maximum health.
    pub max_health: f64,
    /// Armor.
    pub armor: ArmorState,
    /// Inventory entries.
    pub inventory: Vec<InventoryEntry>,
    /// Selected weapon.
    pub weapon: Q1Weapon,
    /// Captured travel extensions.
    pub extensions: Vec<Q1TravelExtension>,
}

/// Items dropped on every level change (`temporaryItems`).
const TEMPORARY_ITEMS: &[&str] = &[
    "q1:key/silver",
    "q1:key/gold",
    "q1:powerup/quad",
    "q1:powerup/invulnerability",
    "q1:powerup/invisibility",
    "q1:powerup/suit",
];

fn entry(item: &str, count: f64, capacity: f64) -> InventoryEntry {
    InventoryEntry {
        item: ItemId::from(item),
        count,
        capacity,
        count_policy: None,
    }
}

/// Fresh travel state for a new campaign (`newQ1Travel`).
#[must_use]
pub fn new_q1_travel(options: &Q1FoundationOptions) -> Q1TravelState {
    let mut inventory: Vec<InventoryEntry> = WEAPONS
        .iter()
        .map(|weapon| {
            let owned = weapon_item(Q1Weapon::from(*weapon));
            let count = if matches!(
                weapon,
                crate::q1::foundation::types::Q1BaseWeapon::Axe | crate::q1::foundation::types::Q1BaseWeapon::Shotgun
            ) {
                1.0
            } else {
                0.0
            };
            InventoryEntry {
                item: owned,
                count,
                capacity: 1.0,
                count_policy: None,
            }
        })
        .collect();
    inventory.push(entry("q1:ammo/shells", 25.0, 100.0));
    inventory.push(entry("q1:ammo/nails", 0.0, 200.0));
    inventory.push(entry("q1:ammo/rockets", 0.0, 100.0));
    inventory.push(entry("q1:ammo/cells", 0.0, 100.0));
    inventory.push(entry("q1:key/silver", 0.0, 1.0));
    inventory.push(entry("q1:key/gold", 0.0, 1.0));
    let health = if options.edition == Q1Edition::Rerelease && options.skill == 3 && options.deathmatch == 0 {
        50.0
    } else {
        100.0
    };
    Q1TravelState {
        health,
        max_health: health,
        armor: ArmorState {
            regular: RegularArmorState::None,
            powered: PoweredProtectionState::None,
        },
        inventory,
        weapon: Q1Weapon::Shotgun,
        extensions: Vec::new(),
    }
}

/// Capture source travel policy without changing the departing actor
/// (`captureQ1Travel`).
pub fn capture_q1_travel(
    game: &mut Q1EntityServices,
    actor: &OwnedActor,
    weapon: Option<Q1Weapon>,
    max_health: Option<f64>,
    reset_in_deathmatch: bool,
) -> Result<Q1TravelState, Q1Error> {
    let id = actor.id().clone();
    let combat = game
        .host
        .combat
        .read(&id)
        .ok_or_else(|| q1_error("Q1 travel actor has no combat state"))?;
    let player = game.player_ref(&id).cloned();
    let weapon = weapon
        .or_else(|| player.as_ref().map(|player| player.weapon))
        .unwrap_or(Q1Weapon::Shotgun);
    let max_health = max_health
        .or_else(|| player.as_ref().map(|player| player.max_health))
        .unwrap_or(100.0);
    if combat.health <= 0.0 || game.options().deathmatch != 0 && reset_in_deathmatch {
        return Ok(Q1TravelState {
            extensions: Vec::new(),
            ..new_q1_travel(game.options())
        });
    }
    let inventory = game
        .host
        .inventory
        .entries(&id)
        .into_iter()
        .map(|entry| {
            if TEMPORARY_ITEMS.contains(&entry.item.as_str()) {
                InventoryEntry { count: 0.0, ..entry }
            } else if entry.item == "q1:ammo/shells" {
                InventoryEntry {
                    count: entry.count.max(25.0),
                    ..entry
                }
            } else {
                entry
            }
        })
        .collect();
    Ok(Q1TravelState {
        health: (max_health / 2.0).max(combat.health.min(max_health)),
        max_health,
        armor: combat.armor,
        inventory,
        weapon,
        extensions: Vec::new(),
    })
}

/// DecodeLevelParms resets equipment when returning to start with an
/// episode rune (`decodeQ1Travel`).
#[must_use]
pub fn decode_q1_travel(game: &Q1EntityServices, state: &Q1TravelState, server_flags: i32) -> Q1TravelState {
    if server_flags != 0 && game.map_name == "start" {
        Q1TravelState {
            extensions: state.extensions.clone(),
            ..new_q1_travel(game.options())
        }
    } else {
        state.clone()
    }
}

/// Apply travel state to an existing admitted actor; it neither
/// allocates one nor selects its movement/appearance (`admitQ1Travel`).
pub fn admit_q1_travel(game: &mut Q1EntityServices, actor: &OwnedActor, state: &Q1TravelState) -> Result<(), Q1Error> {
    let id = actor.id().clone();
    game.host.combat.set_health(actor, state.health)?;
    game.host.combat.set_armor(actor, &state.armor)?;
    if !game.host.inventory.has(&id) {
        game.host.inventory.create(actor, &state.inventory)?;
    } else {
        for entry in &state.inventory {
            game.host.inventory.configure(actor, entry)?;
        }
    }
    if game.player_ref(&id).is_some() {
        let powerups: Vec<_> = game
            .player_ref(&id)
            .map(|player| player.powerups.keys().copied().collect())
            .unwrap_or_default();
        for powerup in powerups {
            game.host.powerup(actor, powerup, 0.0);
        }
        let max_health = state.max_health;
        game.update_player(&id, |player| {
            player.max_health = max_health;
            player.powerups.clear();
            player.mega_rot_at = -1.0;
        })?;
        game.select_weapon(actor, state.weapon)?;
        if let Some(extension) = state.extensions.first() {
            return Err(q1_error(format!(
                "Missing Q1 player travel extension: {}",
                extension.id
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::*;
    use crate::q1::foundation::entity_services::Q1AttachOptions;
    use crate::q1::foundation::host::mock::mock_host;
    use crate::q1::foundation::types::{Q1FoundationOptions, Q1PrecacheProgram};

    fn options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    #[test]
    fn new_travel_matches_donor_loadout() {
        let classic = new_q1_travel(&options());
        assert_eq!((classic.health, classic.max_health), (100.0, 100.0));
        assert_eq!(classic.weapon, Q1Weapon::Shotgun);
        assert_eq!(classic.inventory.len(), 14);
        let nightmare = new_q1_travel(&Q1FoundationOptions {
            edition: Q1Edition::Rerelease,
            skill: 3,
            ..options()
        });
        assert_eq!(nightmare.health, 50.0);
    }

    #[test]
    fn capture_clamps_and_drops_temporary_items() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let id = game.create("player", None, None).expect("player");
        let owned = game.entity_ref(&id).map(|entity| entity.actor.clone()).expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
        game.host.combat.set_health(&owned, 40.0).expect("health");
        game.host
            .inventory
            .configure(&owned, &entry("q1:key/silver", 1.0, 1.0))
            .expect("silver");
        let state = capture_q1_travel(&mut game, &owned, None, None, true).expect("capture");
        assert_eq!(state.health, 50.0);
        assert_eq!(state.max_health, 100.0);
        let silver = state
            .inventory
            .iter()
            .find(|entry| entry.item == "q1:key/silver")
            .expect("silver");
        assert_eq!(silver.count, 0.0);
        let shells = state
            .inventory
            .iter()
            .find(|entry| entry.item == "q1:ammo/shells")
            .expect("shells");
        assert!(shells.count >= 25.0);
        game.map_name = String::from("start");
        let decoded = decode_q1_travel(&game, &state, 1);
        assert_eq!(decoded.health, 100.0);
        let kept = decode_q1_travel(&game, &state, 0);
        assert_eq!(kept.health, 50.0);
    }

    #[test]
    fn dead_actors_restart_from_defaults() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let id = game.create("player", None, None).expect("player");
        let owned = game.entity_ref(&id).map(|entity| entity.actor.clone()).expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
        game.host.combat.set_health(&owned, 0.0).expect("health");
        let state = capture_q1_travel(&mut game, &owned, None, None, true).expect("capture");
        assert_eq!(state.health, 100.0);
        admit_q1_travel(&mut game, &owned, &state).expect("admit");
        assert_eq!(game.health(&id), 100.0);
        let with_extension = Q1TravelState {
            extensions: vec![Q1TravelExtension {
                id: String::from("q1:missing"),
                bytes: vec![1],
            }],
            ..state
        };
        assert!(admit_q1_travel(&mut game, &owned, &with_extension).is_err());
    }
}
