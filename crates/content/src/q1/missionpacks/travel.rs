//! Mission-pack travel (src/content/q1/missionpacks/travel.ts).

use std::collections::HashSet;

use qa_core::identity::OwnedActor;

use crate::contract::{ArmorState, InventoryEntry, ItemId, RegularArmorState};
use crate::q1::base::travel::{admit_q1_travel, capture_q1_travel, new_q1_travel, Q1TravelState};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{weapon_item, Q1Edition, Q1Weapon};
use crate::q1::Q1Error;

use super::types::{Q1MissionPack, MISSION_WEAPONS};

/// Pack inventory entries (`packInventory`).
fn pack_inventory(game: &Q1EntityServices, pack: Q1MissionPack) -> Vec<InventoryEntry> {
    let prefix = pack.as_str();
    let mut inventory: Vec<InventoryEntry> = MISSION_WEAPONS
        .iter()
        .filter(|weapon| weapon.id.as_str().starts_with(prefix))
        .map(|weapon| InventoryEntry {
            item: weapon_item(Q1Weapon::from(weapon.id)),
            count: 0.0,
            capacity: 1.0,
            count_policy: None,
        })
        .collect();
    if pack == Q1MissionPack::Rogue {
        let deathmatch = game.options().deathmatch;
        let teamplay = game.options().teamplay.unwrap_or(0);
        for (item, capacity) in [
            ("rogue:ammo/lava-nails", 200.0),
            ("rogue:ammo/multi-rockets", 100.0),
            ("rogue:ammo/plasma", 100.0),
            ("rogue:artifact/vengeance", 1.0),
        ] {
            inventory.push(InventoryEntry {
                item: ItemId::from(item),
                count: 0.0,
                capacity,
                count_policy: None,
            });
        }
        inventory.push(InventoryEntry {
            item: weapon_item(Q1Weapon::RogueGrapple),
            count: if deathmatch != 0 && teamplay >= 4 { 1.0 } else { 0.0 },
            capacity: 1.0,
            count_policy: None,
        });
    }
    inventory
}

/// Fresh mission-pack travel state (`newMissionPackTravel`).
pub fn new_mission_pack_travel(game: &mut Q1EntityServices, pack: Q1MissionPack) -> Q1TravelState {
    let base = new_q1_travel(game.options());
    let mut inventory = base.inventory.clone();
    inventory.extend(pack_inventory(game, pack));
    let deathmatch = game.options().deathmatch;
    let teamplay = game.options().teamplay.unwrap_or(0);
    if pack == Q1MissionPack::Rogue && deathmatch != 0 && teamplay >= 4 {
        Q1TravelState {
            inventory,
            armor: ArmorState {
                regular: RegularArmorState::Q1 {
                    points: 50.0,
                    absorption: 0.3,
                    item: ItemId::from("q1:armor/green"),
                },
                ..base.armor
            },
            ..base
        }
    } else {
        Q1TravelState { inventory, ..base }
    }
}

/// Capture mission-pack travel state (`captureMissionPackTravel`).
pub fn capture_mission_pack_travel(
    game: &mut Q1EntityServices,
    actor: &OwnedActor,
    pack: Q1MissionPack,
) -> Result<Q1TravelState, Q1Error> {
    let edition = game.options().edition;
    let deathmatch = game.options().deathmatch;
    let teamplay = game.options().teamplay.unwrap_or(0);
    if game.health(actor.id()) <= 0.0
        || edition == Q1Edition::Rerelease && deathmatch != 0
        || pack == Q1MissionPack::Rogue && teamplay >= 4
    {
        return Ok(new_mission_pack_travel(game, pack));
    }
    let player = game.player_ref(actor.id()).cloned();
    let weapon = player.as_ref().map(|player| player.weapon).unwrap_or(Q1Weapon::Shotgun);
    let max_health = if edition == Q1Edition::Classic {
        100.0
    } else {
        player.as_ref().map(|player| player.max_health).unwrap_or(100.0)
    };
    capture_q1_travel(
        game,
        actor,
        Some(weapon),
        Some(max_health),
        edition == Q1Edition::Rerelease,
    )
}

/// Decode mission-pack travel state (`decodeMissionPackTravel`).
pub fn decode_mission_pack_travel(
    game: &mut Q1EntityServices,
    state: &Q1TravelState,
    server_flags: i32,
    pack: Q1MissionPack,
) -> Q1TravelState {
    let deathmatch = game.options().deathmatch;
    let teamplay = game.options().teamplay.unwrap_or(0);
    if pack == Q1MissionPack::Hipnotic && matches!(game.map_name.as_str(), "start" | "hip1m1" | "hip2m1" | "hip3m1")
        || pack == Q1MissionPack::Rogue
            && (server_flags != 0 && game.map_name == "start" || deathmatch == 0 && game.map_name == "r2m1")
    {
        return new_mission_pack_travel(game, pack);
    }
    if pack == Q1MissionPack::Rogue && state.weapon == Q1Weapon::RogueGrapple && teamplay < 4 {
        Q1TravelState {
            weapon: Q1Weapon::Axe,
            ..state.clone()
        }
    } else {
        state.clone()
    }
}

/// Admit mission-pack travel state (`admitMissionPackTravel`).
pub fn admit_mission_pack_travel(
    game: &mut Q1EntityServices,
    actor: &OwnedActor,
    state: &Q1TravelState,
    pack: Q1MissionPack,
) -> Result<(), Q1Error> {
    let supplied: HashSet<&ItemId> = state.inventory.iter().map(|entry| &entry.item).collect();
    let mut inventory: Vec<InventoryEntry> = pack_inventory(game, pack)
        .into_iter()
        .filter(|entry| !supplied.contains(&entry.item))
        .collect();
    inventory.extend(state.inventory.clone());
    admit_q1_travel(
        game,
        actor,
        &Q1TravelState {
            inventory,
            ..state.clone()
        },
    )?;
    game.set_gravity(actor.id(), 1.0)
}

#[cfg(test)]
mod tests {
    use super::super::types::test_game;
    use super::*;
    use crate::q1::foundation::entity_services::Q1AttachOptions;

    fn attached(game: &mut Q1EntityServices) -> OwnedActor {
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
        owned
    }

    #[test]
    fn fresh_travel_carries_pack_entries() {
        let mut game = test_game();
        let state = new_mission_pack_travel(&mut game, Q1MissionPack::Hipnotic);
        assert!(state
            .inventory
            .iter()
            .any(|entry| entry.item == weapon_item(Q1Weapon::HipnoticLaser)));
        assert!(!state
            .inventory
            .iter()
            .any(|entry| entry.item == weapon_item(Q1Weapon::RoguePlasma)));
        let state = new_mission_pack_travel(&mut game, Q1MissionPack::Rogue);
        assert!(state.inventory.iter().any(|entry| entry.item == "rogue:ammo/plasma"));
    }

    #[test]
    fn dead_actors_capture_fresh() {
        let mut game = test_game();
        let owned = attached(&mut game);
        game.set_health(owned.id(), 0.0).expect("kill");
        let state = capture_mission_pack_travel(&mut game, &owned, Q1MissionPack::Rogue).expect("capture");
        assert_eq!(state.weapon, Q1Weapon::Shotgun);
    }

    #[test]
    fn decode_resets_episode_starts() {
        let mut game = test_game();
        game.map_name = String::from("hip2m1");
        let state = new_mission_pack_travel(&mut game, Q1MissionPack::Hipnotic);
        let decoded = decode_mission_pack_travel(&mut game, &state, 1, Q1MissionPack::Hipnotic);
        assert_eq!(decoded.weapon, Q1Weapon::Shotgun);
    }

    #[test]
    fn admit_merges_pack_entries() {
        let mut game = test_game();
        game.host.set_gravity = Some(Box::new(|_, _| {}));
        let owned = attached(&mut game);
        let state = new_q1_travel(game.options());
        admit_mission_pack_travel(&mut game, &owned, &state, Q1MissionPack::Hipnotic).expect("admit");
        assert!(game.host.inventory.has(owned.id()));
    }
}
