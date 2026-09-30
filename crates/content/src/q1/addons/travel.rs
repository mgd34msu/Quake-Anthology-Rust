//! Q1 addon travel (`src/content/q1/addons/travel.ts`).
//!
//! `quakec_mg1/client.qc` and `quakec_mg3/client.qc` source level
//! parameters. GPL-2.0-or-later.

use qa_core::identity::OwnedActor;

use crate::contract::InventoryEntry;
use crate::q1::addons::campaign::{BLOODY_NIGHTMARE_ACTIVE, BLOODY_NIGHTMARE_DISCOVERED};
use crate::q1::addons::context::{addon_cvar, Q1AddonContext, Q1AddonProgram};
use crate::q1::base::provider::update_base;
use crate::q1::base::travel::{admit_q1_travel, capture_q1_travel, new_q1_travel, Q1TravelState};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::Q1Error;

/// Fresh addon travel state for a new campaign (`newQ1AddonTravel`).
#[must_use]
pub fn new_q1_addon_travel(context: &Q1AddonContext, game: &Q1EntityServices) -> Q1TravelState {
    let state = new_q1_travel(game.options());
    if context.program() != Q1AddonProgram::Mg3 || game.options().deathmatch != 0 {
        return state;
    }
    Q1TravelState {
        health: 50.0,
        max_health: 50.0,
        inventory: state
            .inventory
            .into_iter()
            .map(|entry| {
                let capacity = if entry.item == "q1:ammo/shells" {
                    50.0
                } else if entry.item == "q1:ammo/nails" {
                    100.0
                } else if entry.item == "q1:ammo/rockets" {
                    20.0
                } else {
                    entry.capacity
                };
                InventoryEntry { capacity, ..entry }
            })
            .collect(),
        ..state
    }
}

/// Capture addon travel policy without changing the departing actor
/// (`captureQ1AddonTravel`).
pub fn capture_q1_addon_travel(
    context: &Q1AddonContext,
    game: &mut Q1EntityServices,
    actor: &OwnedActor,
) -> Result<Q1TravelState, Q1Error> {
    let state = capture_q1_travel(game, actor, None, None, true)?;
    // boss_end changes next-level parameters without changing the
    // departing player's health.
    if context.program() == Q1AddonProgram::Mg3
        && game.world.as_ref().is_some_and(|world| {
            game.entity_ref(world)
                .is_some_and(|entity| entity.number("mg3.finalNewGameTravel") == 1.0)
        })
    {
        return Ok(Q1TravelState {
            health: 50.0,
            max_health: 50.0,
            ..state
        });
    }
    if game.health(actor.id()) <= 0.0 || game.options().deathmatch != 0 || game.world_type == 3 {
        return Ok(Q1TravelState {
            extensions: state.extensions.clone(),
            ..new_q1_addon_travel(context, game)
        });
    }
    Ok(state)
}

/// Decode addon travel state for admission (`decodeQ1AddonTravel`).
pub fn decode_q1_addon_travel(
    context: &Q1AddonContext,
    game: &mut Q1EntityServices,
    state: &Q1TravelState,
) -> Result<Q1TravelState, Q1Error> {
    if context.program() == Q1AddonProgram::Mg3 && game.map_name == "boss2" && game.options().skill == 3 {
        update_base(game, |state| {
            let flags = state.campaign.read_flags();
            state
                .campaign
                .write_flags(flags | BLOODY_NIGHTMARE_ACTIVE | BLOODY_NIGHTMARE_DISCOVERED);
        })?;
    }
    if game.map_name == "start"
        || game.world_type == 3
        || (context.program() != Q1AddonProgram::Mg3 && addon_cvar(game, "horde")? != 0.0)
    {
        Ok(Q1TravelState {
            extensions: state.extensions.clone(),
            ..new_q1_addon_travel(context, game)
        })
    } else {
        Ok(state.clone())
    }
}

/// Apply addon travel state to an admitted actor (`admitQ1AddonTravel`).
/// The existing admission applies shared inventory, then source travel
/// extensions restore addon words.
pub fn admit_q1_addon_travel(
    context: &Q1AddonContext,
    game: &mut Q1EntityServices,
    actor: &OwnedActor,
    state: &Q1TravelState,
) -> Result<(), Q1Error> {
    let decoded = decode_q1_addon_travel(context, game, state)?;
    admit_q1_travel(game, actor, &decoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn nightmare_game() -> Q1EntityServices {
        use qa_core::identity::ProviderId;

        use crate::q1::foundation::host::mock::mock_host;
        use crate::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};

        let (host, _) = mock_host();
        Q1EntityServices::new(
            host,
            Q1FoundationOptions {
                provider: None,
                precache_program: Some(Q1PrecacheProgram::Id1),
                edition: Q1Edition::Classic,
                physics_edition: None,
                skill: 3,
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
            },
        )
        .expect("game")
    }

    #[test]
    fn new_travel_matches_program_policy() {
        let mut game = test_game();
        let context = register_test_addons(&mut game, Q1AddonProgram::Mg3);
        let state = new_q1_addon_travel(&context, &game);
        assert_eq!(state.health, 50.0);
        assert_eq!(state.max_health, 50.0);
        let capacity = |item: &str| {
            state
                .inventory
                .iter()
                .find(|entry| entry.item == item)
                .map(|entry| entry.capacity)
        };
        assert_eq!(capacity("q1:ammo/shells"), Some(50.0));
        assert_eq!(capacity("q1:ammo/nails"), Some(100.0));
        assert_eq!(capacity("q1:ammo/rockets"), Some(20.0));
        assert_eq!(capacity("q1:ammo/cells"), Some(100.0));

        let context = Q1AddonContext::new(Q1AddonProgram::Mg1);
        let state = new_q1_addon_travel(&context, &game);
        assert_eq!(state.health, 100.0);
    }

    #[test]
    fn capture_resets_dead_players_and_keeps_extensions() {
        let mut game = test_game();
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(&mut game, Q1AddonProgram::Mg3);
        let player = attach_test_player(&mut game);
        game.set_health(&player, 0.0).expect("kill");
        let owned = game.host.actors.resolve_owned(&player).expect("owned");
        let state = capture_q1_addon_travel(&context, &mut game, &owned).expect("capture");
        assert_eq!(state.health, 50.0);
        assert_eq!(state.max_health, 50.0);
        assert!(state.extensions.is_empty());
    }

    #[test]
    fn capture_applies_final_new_game_health() {
        let mut game = test_game();
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(&mut game, Q1AddonProgram::Mg3);
        let player = attach_test_player(&mut game);
        let owned = game.host.actors.resolve_owned(&player).expect("owned");
        let world = game.create("worldspawn", None, None).expect("world");
        game.world = Some(world.clone());
        game.update_entity(&world, |entity| {
            entity
                .fields
                .insert(String::from("mg3.finalNewGameTravel"), String::from("1"));
        })
        .expect("flag");
        let state = capture_q1_addon_travel(&context, &mut game, &owned).expect("capture");
        assert_eq!(state.health, 50.0);
        assert_eq!(state.max_health, 50.0);
    }

    #[test]
    fn decode_resets_on_start_and_marks_bloody_nightmare() {
        let mut game = nightmare_game();
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(&mut game, Q1AddonProgram::Mg3);
        game.map_name = String::from("boss2");
        let state = new_q1_addon_travel(&context, &game);
        decode_q1_addon_travel(&context, &mut game, &state).expect("decode");
        let flags = update_base(&game, |state| state.campaign.read_flags()).expect("flags");
        assert_eq!(
            flags & (BLOODY_NIGHTMARE_ACTIVE | BLOODY_NIGHTMARE_DISCOVERED),
            BLOODY_NIGHTMARE_ACTIVE | BLOODY_NIGHTMARE_DISCOVERED
        );

        game.map_name = String::from("start");
        let mut custom = state.clone();
        custom.health = 12.0;
        let decoded = decode_q1_addon_travel(&context, &mut game, &custom).expect("decode");
        assert_eq!(decoded.health, 50.0);
    }

    #[test]
    fn admit_applies_decoded_travel() {
        let mut game = test_game();
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(&mut game, Q1AddonProgram::Mg1);
        let player = attach_test_player(&mut game);
        let owned = game.host.actors.resolve_owned(&player).expect("owned");
        let mut state = new_q1_addon_travel(&context, &game);
        state.health = 73.0;
        admit_q1_addon_travel(&context, &mut game, &owned, &state).expect("admit");
        assert_eq!(game.health(&player), 73.0);
    }
}
