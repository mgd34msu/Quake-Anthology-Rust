//! Q1 mg3 item impulses (`src/content/q1/addons/items/commands.ts`).
//!
//! `quakec_mg3/weapons.qc` equipment branches of `ImpulseCommands`.
//! GPL-2.0-or-later.

use qa_core::identity::ActorId;

use crate::contract::{InventoryEntry, ItemId};
use crate::q1::addons::context::{
    addon_cheat_arsenal, addon_cvar, addon_player_number, addon_program, set_addon_player_number, Q1AddonCheatCategory,
    Q1AddonProgram,
};
use crate::q1::addons::items::pickups::{MG3_BLOODY_SHOTGUN, MG3_BLOODY_SUPER_SHOTGUN};
use crate::q1::addons::items::upgrades::{give_next_mg3_upgrade, set_mg3_inventory_capacity, Mg3Upgrade};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{Q1Weapon, WEAPONS};
use crate::q1::Q1Error;

/// Sets an inventory count (`count`).
fn set_count(
    game: &mut Q1EntityServices,
    actor: &ActorId,
    player: &qa_core::identity::OwnedActor,
    item: ItemId,
    amount: f64,
    capacity: f64,
) -> Result<(), Q1Error> {
    let previous = game
        .host
        .inventory
        .entries(actor)
        .into_iter()
        .find(|entry| entry.item == item);
    let entry = match previous {
        Some(mut found) => {
            found.count = amount;
            found
        }
        None => InventoryEntry {
            item,
            count: amount,
            capacity,
            count_policy: None,
        },
    };
    game.host.inventory.configure(player, &entry)
}

/// Restocks ammunition (`restock`).
fn restock(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let owned = game
        .player_owned(actor)
        .ok_or_else(|| crate::q1::q1_error("Missing Q1 restock player"))?;
    if addon_cheat_arsenal(game, actor, Q1AddonCheatCategory::Ammo)? {
        return Ok(());
    }
    set_count(game, actor, &owned, String::from("q1:ammo/shells"), 100.0, 100.0)?;
    set_count(game, actor, &owned, String::from("q1:ammo/nails"), 200.0, 200.0)?;
    set_count(game, actor, &owned, String::from("q1:ammo/rockets"), 100.0, 100.0)?;
    set_count(game, actor, &owned, String::from("q1:ammo/cells"), 200.0, 100.0)
}

/// Checks firing ammunition (`enoughAmmo`).
fn enough_ammo(game: &mut Q1EntityServices, actor: &ActorId, weapon: Q1Weapon) -> bool {
    if weapon == Q1Weapon::Axe || weapon == Q1Weapon::Mg3Mjolnir {
        return true;
    }
    let minimum = if weapon == Q1Weapon::Supershotgun || weapon == Q1Weapon::Supernailgun {
        2.0
    } else {
        1.0
    };
    match game.weapon_ammo(weapon) {
        None => true,
        Some(item) => game.host.inventory.count(actor, &item) >= minimum,
    }
}

/// Resolves the melee weapon (`melee`).
fn melee_weapon(game: &mut Q1EntityServices, actor: &ActorId) -> Q1Weapon {
    if game
        .host
        .inventory
        .count(actor, &game.weapon_item(Q1Weapon::Mg3Mjolnir))
        != 0.0
    {
        Q1Weapon::Mg3Mjolnir
    } else {
        Q1Weapon::Axe
    }
}

/// Cycles weapons (`cycle`).
fn cycle_weapon(game: &mut Q1EntityServices, actor: &ActorId, reverse: bool) -> Result<(), Q1Error> {
    let order = [
        melee_weapon(game, actor),
        Q1Weapon::Shotgun,
        Q1Weapon::Supershotgun,
        Q1Weapon::Nailgun,
        Q1Weapon::Supernailgun,
        Q1Weapon::Grenadelauncher,
        Q1Weapon::Rocketlauncher,
        Q1Weapon::Lightning,
        Q1Weapon::Mg3Laser,
    ];
    let current = game
        .player_ref(actor)
        .map(|state| state.weapon)
        .unwrap_or(Q1Weapon::Axe);
    let mut index = if current == Q1Weapon::Axe || current == Q1Weapon::Mg3Mjolnir {
        0
    } else {
        match order.iter().position(|weapon| *weapon == current) {
            Some(index) => index as i32,
            None => return Ok(()),
        }
    };
    let owned = game
        .player_owned(actor)
        .ok_or_else(|| crate::q1::q1_error("Missing Q1 cycle player"))?;
    for _ in 0..10 {
        index = (index + (if reverse { -1 } else { 1 }) + order.len() as i32) % order.len() as i32;
        let weapon = order[index as usize];
        if enough_ammo(game, actor, weapon) && game.select_weapon(&owned, weapon)? {
            return Ok(());
        }
    }
    Ok(())
}

/// Handles an mg3 item impulse (`handleMg3ItemImpulse`). Called after
/// the source attack_finished gate; returns whether it was handled.
pub fn handle_mg3_item_impulse(
    game: &mut Q1EntityServices,
    actor: &ActorId,
    impulse: i32,
    developer_message: &mut dyn FnMut(&str),
) -> Result<bool, Q1Error> {
    if addon_program(game)? != Q1AddonProgram::Mg3 {
        return Ok(false);
    }
    let state = match game.player_ref(actor).cloned() {
        Some(state) => state,
        None => return Ok(false),
    };
    if impulse == 1 || impulse == 225 {
        let weapon = if impulse == 1 {
            melee_weapon(game, actor)
        } else {
            Q1Weapon::Mg3Laser
        };
        if game.host.inventory.count(actor, &game.weapon_item(weapon)) == 0.0 {
            game.message(Some(actor), "$qc_no_weapon", false, Vec::new());
        } else if !enough_ammo(game, actor, weapon) {
            game.message(Some(actor), "$qc_not_enough_ammo", false, Vec::new());
        } else {
            game.select_weapon(&state.actor, weapon)?;
        }
        return Ok(true);
    }
    if impulse == 10 || impulse == 12 {
        cycle_weapon(game, actor, impulse == 12)?;
        return Ok(true);
    }
    if impulse == 9 || impulse == 99 {
        if (game.options().deathmatch != 0 || game.options().coop) && addon_cvar(game, "sv_cheats")? == 0.0 {
            return Ok(true);
        }
        restock(game, actor)?;
        let selected = addon_cheat_arsenal(game, actor, Q1AddonCheatCategory::Weapons)?;
        let owned = game
            .player_owned(actor)
            .ok_or_else(|| crate::q1::q1_error("Missing Q1 cheat player"))?;
        if !selected {
            for weapon in WEAPONS {
                let item = game.weapon_item(Q1Weapon::from(weapon));
                set_count(game, actor, &owned, item, 1.0, 1.0)?;
            }
            let item = game.weapon_item(Q1Weapon::Mg3Laser);
            set_count(game, actor, &owned, item, 1.0, 1.0)?;
        }
        if impulse != 99 {
            set_count(game, actor, &owned, String::from("q1:key/silver"), 1.0, 1.0)?;
            set_count(game, actor, &owned, String::from("q1:key/gold"), 1.0, 1.0)?;
        }
        if !selected {
            game.select_weapon(&owned, Q1Weapon::Rocketlauncher)?;
        }
        return Ok(true);
    }
    if impulse == 100 {
        developer_message("Resetting to defaults\n");
        game.update_player(actor, |state| state.max_health = 100.0)?;
        let owned = game
            .player_owned(actor)
            .ok_or_else(|| crate::q1::q1_error("Missing Q1 reset player"))?;
        game.host.combat.set_health(&owned, 100.0)?;
        for (item, capacity) in [
            ("q1:ammo/shells", 100.0),
            ("q1:ammo/nails", 200.0),
            ("q1:ammo/rockets", 100.0),
            ("q1:ammo/cells", 100.0),
        ] {
            set_mg3_inventory_capacity(game, actor, &item.to_string(), capacity)?;
            let count = game.host.inventory.count(actor, &item.to_string());
            game.host.inventory.configure(
                &owned,
                &InventoryEntry {
                    item: item.to_string(),
                    count,
                    capacity,
                    count_policy: None,
                },
            )?;
        }
        return Ok(true);
    }
    if (111..=115).contains(&impulse) {
        let upgrade = [
            Mg3Upgrade::Health,
            Mg3Upgrade::Shells,
            Mg3Upgrade::Nails,
            Mg3Upgrade::Rockets,
            Mg3Upgrade::Cells,
        ][(impulse - 111) as usize];
        give_next_mg3_upgrade(game, upgrade, actor)?;
        return Ok(true);
    }
    if impulse == 118 {
        let owned = game
            .player_owned(actor)
            .ok_or_else(|| crate::q1::q1_error("Missing Q1 mjolnir player"))?;
        let item = game.weapon_item(Q1Weapon::Mg3Mjolnir);
        set_count(game, actor, &owned, item, 1.0, 1.0)?;
        return Ok(true);
    }
    if impulse == 122 {
        developer_message("$m_inf_ammo");
        let enabled = addon_player_number(game, actor, "infiniteammo")? == 0.0;
        set_addon_player_number(game, actor, "infiniteammo", f64::from(i32::from(enabled)))?;
        if enabled {
            restock(game, actor)?;
        }
        return Ok(true);
    }
    if impulse == 227 || impulse == 228 {
        let flag = if impulse == 227 {
            MG3_BLOODY_SHOTGUN
        } else {
            MG3_BLOODY_SUPER_SHOTGUN
        };
        let flags = addon_player_number(game, actor, "parm15")? as i32;
        let enabled = flags & flag == 0;
        set_addon_player_number(game, actor, "parm15", f64::from(flags ^ flag))?;
        developer_message(&format!(
            "{} 'bloody {}' upgrade\n",
            if enabled { "activated" } else { "deactivated" },
            if impulse == 227 { "shotgun" } else { "Super Shotgun" }
        ));
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> Q1BaseGuard {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Mg3);
        crate::q1::addons::items::common::register_mg3_item_callbacks(game).expect("callbacks");
        crate::q1::addons::items::upgrades::register_mg3_upgrades(game).expect("upgrades");
        guard
    }

    #[test]
    fn impulse_9_restocks_and_arms() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        let mut messages = Vec::new();
        assert!(
            handle_mg3_item_impulse(&mut game, &player, 9, &mut |text| messages.push(text.to_string()))
                .expect("impulse")
        );
        assert_eq!(
            game.host.inventory.count(&player, &String::from("q1:ammo/shells")),
            100.0
        );
        assert!(
            game.host
                .inventory
                .count(&player, &game.weapon_item(Q1Weapon::Mg3Laser))
                != 0.0
        );
    }

    #[test]
    fn upgrade_impulses_grant_bits() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        let mut messages = Vec::new();
        assert!(
            handle_mg3_item_impulse(&mut game, &player, 112, &mut |text| messages.push(text.to_string()))
                .expect("impulse")
        );
        assert_eq!(addon_player_number(&game, &player, "parm11"), Ok(1.0));
    }

    #[test]
    fn bloody_and_infinite_toggles_flip() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        let mut messages = Vec::new();
        assert!(
            handle_mg3_item_impulse(&mut game, &player, 227, &mut |text| messages.push(text.to_string()))
                .expect("impulse")
        );
        assert_eq!(addon_player_number(&game, &player, "parm15"), Ok(1.0));
        assert!(
            handle_mg3_item_impulse(&mut game, &player, 122, &mut |text| messages.push(text.to_string()))
                .expect("impulse")
        );
        assert_eq!(addon_player_number(&game, &player, "infiniteammo"), Ok(1.0));
        assert_eq!(handle_mg3_item_impulse(&mut game, &player, 200, &mut |_| {}), Ok(false));
    }
}
