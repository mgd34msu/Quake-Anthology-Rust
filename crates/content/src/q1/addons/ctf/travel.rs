//! Q1 CTF level travel parameters (src/content/q1/addons/ctf/travel.ts).

use qa_core::identity::{ActorId, OwnedActor};

use crate::contract::{ArmorState, InventoryEntry, RegularArmorState};
use crate::q1::addons::context::set_combat_team;
use crate::q1::addons::ctf::state::{
    ctf_number, ctf_owner, ctf_set, ctf_start_map, ctf_teamplay_bits, native_grapple_enabled, number_team,
    with_ctf_services,
};
use crate::q1::addons::ctf::types::CtfFlags;
use crate::q1::base::travel::{capture_q1_travel, new_q1_travel, Q1TravelState};
use crate::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::Q1Weapon;
use crate::q1::Q1Error;
use crate::value::{num, obj, SaveReader};

/// Fresh CTF travel state (`newQ1CtfTravel`). CTF SetNewParms always
/// starts at 100, including rerelease nightmare co-op.
pub fn new_q1_ctf_travel(game: &Q1EntityServices) -> Result<Q1TravelState, Q1Error> {
    let base = new_q1_travel(game.options());
    let pregame_over = game
        .world
        .as_ref()
        .and_then(|world| game.entity_ref(world).map(|entity| entity.number("ctf.pregameOver")))
        .unwrap_or(0.0);
    let lobby = ctf_start_map(game) && pregame_over == 0.0;
    let mut inventory: Vec<InventoryEntry> = base
        .inventory
        .into_iter()
        .map(|entry| {
            if entry.item == "q1:weapon/shotgun" {
                InventoryEntry {
                    count: if lobby { 0.0 } else { 1.0 },
                    ..entry
                }
            } else if entry.item == "q1:ammo/shells" {
                InventoryEntry {
                    count: if lobby { 0.0 } else { 40.0 },
                    ..entry
                }
            } else {
                entry
            }
        })
        .collect();
    let grapple = native_grapple_enabled(game)? && !lobby && ctf_teamplay_bits(game)? & CtfFlags::DISABLE_GRAPPLE == 0;
    inventory.push(InventoryEntry {
        item: String::from("q1:ctf/weapon/grapple"),
        count: if grapple { 1.0 } else { 0.0 },
        capacity: 1.0,
        count_policy: None,
    });
    Ok(Q1TravelState {
        health: 100.0,
        max_health: 100.0,
        inventory,
        weapon: if lobby { Q1Weapon::Axe } else { Q1Weapon::Shotgun },
        extensions: Vec::new(),
        armor: ArmorState {
            regular: if lobby {
                RegularArmorState::None
            } else {
                RegularArmorState::Q1 {
                    points: 50.0,
                    absorption: 0.3,
                    item: String::from("q1:item_armor1"),
                }
            },
            powered: base.armor.powered,
        },
    })
}

/// Capture CTF travel policy (`captureQ1CtfTravel`). SetChangeParms
/// resets equipment even for living players; parm10/14/15 still travel
/// through the spawn-parameters extension.
pub fn capture_q1_ctf_travel(game: &mut Q1EntityServices, actor: &OwnedActor) -> Result<Q1TravelState, Q1Error> {
    let saved = capture_q1_travel(game, actor, None, None, true)?;
    Ok(Q1TravelState {
        extensions: saved.extensions,
        ..new_q1_ctf_travel(game)?
    })
}

/// Decode CTF travel state for admission (`decodeQ1CtfTravel`).
pub fn decode_q1_ctf_travel(game: &Q1EntityServices, travel: &Q1TravelState) -> Result<Q1TravelState, Q1Error> {
    if ctf_start_map(game) {
        Ok(Q1TravelState {
            extensions: travel.extensions.clone(),
            ..new_q1_ctf_travel(game)?
        })
    } else {
        Ok(travel.clone())
    }
}

/// Capture the spawn-parameters extension bytes (`captureTravel`).
pub fn capture_travel(game: &mut Q1EntityServices, actor: &ActorId) -> Vec<u8> {
    capture_travel_inner(game, actor).unwrap_or_else(|_| {
        encode_checkpoint_value(&obj(vec![
            ("lastTeam", num(0.0)),
            ("status", num(0.0)),
            ("access", num(0.0)),
        ]))
    })
}

fn capture_travel_inner(game: &mut Q1EntityServices, actor: &ActorId) -> Result<Vec<u8>, Q1Error> {
    let id = actor.clone();
    let observer = with_ctf_services(game, |services| services.observer(&id))?;
    let last_team = if game.health(&id) > 0.0 && (ctf_start_map(game) || observer) {
        -1.0
    } else {
        ctf_number(game, &id, "lastteam")?
    };
    Ok(encode_checkpoint_value(&obj(vec![
        ("lastTeam", num(last_team)),
        ("status", num(ctf_number(game, &id, "status")?)),
        ("access", num(ctf_number(game, &id, "access")?)),
    ])))
}

/// Restore the spawn-parameters extension bytes (`restoreTravel`).
pub fn restore_travel(game: &mut Q1EntityServices, actor: &ActorId, bytes: &[u8]) -> Result<(), Q1Error> {
    let id = actor.clone();
    let saved = decode_checkpoint_value(bytes)?;
    let reader = SaveReader::at(&saved, "q1:ctf:travel");
    let team = reader.field("lastTeam").number()? as i64;
    ctf_set(
        game,
        &id,
        "lastteam",
        if ctf_start_map(game) { 1.0 } else { team as f64 },
    )?;
    ctf_set(game, &id, "status", reader.field("status").number()?)?;
    ctf_set(game, &id, "access", reader.field("access").number()?)?;
    if let Some(color) = number_team(team) {
        if !ctf_start_map(game) {
            let owner = ctf_owner(game, &id)?;
            set_combat_team(game, &owner, Some(color.as_str()))?;
            let shade = (team - 1) as i32;
            with_ctf_services(game, |services| services.colors(&id, shade, shade))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::addons::ctf::state::{ctf_last_team, register_ctf_state};
    use crate::q1::addons::ctf::types::{CtfTeam, FakeCtfServices};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> (Q1BaseGuard, ActorId) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Ctf);
        let (services, _) = FakeCtfServices::new();
        register_ctf_state(game, Box::new(services), true, false);
        let player = attach_test_player(game);
        (guard, player)
    }

    #[test]
    fn fresh_travel_starts_at_full_health() {
        let mut game = test_game();
        let (_guard, _player) = setup(&mut game);
        let travel = new_q1_ctf_travel(&game).expect("travel");
        assert_eq!(travel.health, 100.0);
        assert_eq!(travel.weapon, Q1Weapon::Shotgun);
        assert!(travel
            .inventory
            .iter()
            .any(|entry| { entry.item == "q1:ctf/weapon/grapple" && entry.count == 1.0 }));
    }

    #[test]
    fn spawn_parameters_round_trip_last_team() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game);
        ctf_set(&mut game, &player, "lastteam", 14.0).expect("lastteam");
        game.set_health(&player, 0.0).expect("dead");
        let bytes = capture_travel(&mut game, &player);
        ctf_set(&mut game, &player, "lastteam", 0.0).expect("clear");
        restore_travel(&mut game, &player, &bytes).expect("restore");
        assert_eq!(ctf_last_team(&game, &player), Ok(Some(CtfTeam::Blue)));
    }
}
