//! Q1 mg3 capacity upgrades (`src/content/q1/addons/items/upgrades.ts`).
//!
//! `quakec_mg3/mg3_upgrades.qc` and `client.qc`. GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::campaign::BLOODY_NIGHTMARE_ACTIVE;
use crate::q1::addons::context::{
    addon_cvar, addon_player_number, addon_player_word, fround, set_addon_number, set_addon_player_number,
};
use crate::q1::addons::items::common::{finish_mg3_pickup, start_mg3_item, MG3_ITEM_PREFIX};
use crate::q1::addons::items::pickups::MG3_BLOODY_SUPER_SHOTGUN;
use crate::q1::base::provider::update_base;
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{vadd, Q1MessageArg, Q1Solid, Q1Weapon};
use crate::q1::Q1Error;
use crate::value::{arr, num, SaveReader};

/// MG3 capacity upgrade (`Mg3Upgrade`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mg3Upgrade {
    /// Extra health.
    Health,
    /// Extra shells.
    Shells,
    /// Extra nails.
    Nails,
    /// Extra rockets.
    Rockets,
    /// Extra cells.
    Cells,
}

struct UpgradeDefinition {
    upgrade: Mg3Upgrade,
    parm: &'static str,
    base: f64,
    deathmatch: f64,
    model: &'static str,
    label: &'static str,
    ammo: Option<&'static str>,
}

const UPGRADES: [UpgradeDefinition; 5] = [
    UpgradeDefinition {
        upgrade: Mg3Upgrade::Health,
        parm: "parm10",
        base: 50.0,
        deathmatch: 100.0,
        model: "item_h_player",
        label: "health",
        ammo: None,
    },
    UpgradeDefinition {
        upgrade: Mg3Upgrade::Shells,
        parm: "parm11",
        base: 50.0,
        deathmatch: 100.0,
        model: "backpackshells",
        label: "shell",
        ammo: Some("q1:ammo/shells"),
    },
    UpgradeDefinition {
        upgrade: Mg3Upgrade::Nails,
        parm: "parm12",
        base: 100.0,
        deathmatch: 200.0,
        model: "backpacknails",
        label: "nail",
        ammo: Some("q1:ammo/nails"),
    },
    UpgradeDefinition {
        upgrade: Mg3Upgrade::Rockets,
        parm: "parm13",
        base: 20.0,
        deathmatch: 100.0,
        model: "backpacker",
        label: "rocket",
        ammo: Some("q1:ammo/rockets"),
    },
    UpgradeDefinition {
        upgrade: Mg3Upgrade::Cells,
        parm: "parm14",
        base: 100.0,
        deathmatch: 200.0,
        model: "backpackcells",
        label: "cell",
        ammo: Some("q1:ammo/cells"),
    },
];

const TRAVEL_PARMS: [&str; 6] = ["parm10", "parm11", "parm12", "parm13", "parm14", "parm15"];

fn definition(upgrade: Mg3Upgrade) -> &'static UpgradeDefinition {
    UPGRADES
        .iter()
        .find(|definition| definition.upgrade == upgrade)
        .expect("mg3 upgrade")
}

fn parse_upgrade(name: &str) -> Result<Mg3Upgrade, Q1Error> {
    match name {
        "health" => Ok(Mg3Upgrade::Health),
        "shells" => Ok(Mg3Upgrade::Shells),
        "nails" => Ok(Mg3Upgrade::Nails),
        "rockets" => Ok(Mg3Upgrade::Rockets),
        "cells" => Ok(Mg3Upgrade::Cells),
        _ => Err(crate::q1::q1_error(format!("Unknown MG3 upgrade {name}"))),
    }
}

fn suffix(upgrade: Mg3Upgrade) -> &'static str {
    match upgrade {
        Mg3Upgrade::Health => "health",
        Mg3Upgrade::Shells => "shells",
        Mg3Upgrade::Nails => "nails",
        Mg3Upgrade::Rockets => "rockets",
        Mg3Upgrade::Cells => "cells",
    }
}

/// Upgrade flag bit for a map (`mg3UpgradeFlag`).
#[must_use]
pub fn mg3_upgrade_flag(map: &str) -> i32 {
    if map == "map2b" {
        return 8192;
    }
    const MAPS: [&str; 13] = [
        "map1", "map2", "map3", "map4", "map5", "map6", "map7", "map8", "secret1", "secret2", "secret3", "secret4",
        "secret5",
    ];
    match MAPS.iter().position(|candidate| *candidate == map) {
        Some(index) => 1 << index,
        None => {
            if map == "secret6" {
                16384
            } else {
                0
            }
        }
    }
}

/// Upgraded maximum capacity (`mg3UpgradedMaximum`).
#[must_use]
pub fn mg3_upgraded_maximum(base: f64, flags: f64) -> f64 {
    let mut result = base;
    #[allow(clippy::cast_possible_truncation)]
    let flags = flags as i32;
    for bit in 0..23 {
        if flags & (1 << bit) != 0 {
            result += 10.0;
        }
    }
    result
}

/// Captures upgrade travel bytes (`captureMg3UpgradeTravel`).
pub fn capture_mg3_upgrade_travel(game: &mut Q1EntityServices, player: &ActorId) -> Vec<u8> {
    let values = TRAVEL_PARMS
        .iter()
        .map(|parm| num(addon_player_number(game, player, parm).unwrap_or(0.0)))
        .collect();
    encode_checkpoint_value(&arr(values))
}

/// Restores upgrade travel bytes (`restoreMg3UpgradeTravel`).
pub fn restore_mg3_upgrade_travel(game: &mut Q1EntityServices, player: &ActorId, bytes: &[u8]) -> Result<(), Q1Error> {
    let value = decode_checkpoint_value(bytes)?;
    let reader = SaveReader::at(&value, "mg3:upgrade-travel");
    let values = reader.list(|entry| {
        let number = entry.integer(0)?;
        if number <= 8_388_607 {
            Ok(number)
        } else {
            Err(entry.fail("MG3 source parameter exceeds its flag word"))
        }
    })?;
    if values.len() != TRAVEL_PARMS.len() {
        return Err(reader.fail("expected six MG3 source travel parameters").into());
    }
    for (index, parm) in TRAVEL_PARMS.iter().enumerate() {
        let Some(value) = values.get(index) else {
            return Err(reader.fail("missing source travel parameter").into());
        };
        set_addon_player_number(game, player, parm, *value as f64)?;
    }
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    if flags & BLOODY_NIGHTMARE_ACTIVE != 0 {
        if addon_cvar(game, "skill")? != 3.0 {
            update_base(game, |state| {
                state.campaign.write_flags(flags & !BLOODY_NIGHTMARE_ACTIVE);
            })?;
        } else {
            let hammer = game
                .host
                .inventory
                .count(player, &game.weapon_item(Q1Weapon::Mg3Mjolnir))
                != 0.0;
            let bloody_super = addon_player_number(game, player, "parm15")? as i32 & MG3_BLOODY_SUPER_SHOTGUN != 0;
            for entry in game.host.inventory.entries(player) {
                if entry.item.starts_with("q1:weapon/") {
                    let owned = entry.item == game.weapon_item(Q1Weapon::Axe)
                        || entry.item == game.weapon_item(Q1Weapon::Shotgun)
                        || hammer && entry.item == game.weapon_item(Q1Weapon::Mg3Mjolnir)
                        || bloody_super && entry.item == game.weapon_item(Q1Weapon::Supershotgun);
                    let owned_actor = game
                        .player_owned(player)
                        .ok_or_else(|| crate::q1::q1_error("Missing Q1 upgrade player"))?;
                    let mut reset = entry.clone();
                    reset.count = f64::from(i32::from(owned));
                    game.host.inventory.configure(&owned_actor, &reset)?;
                }
            }
            if bloody_super {
                let owned_actor = game
                    .player_owned(player)
                    .ok_or_else(|| crate::q1::q1_error("Missing Q1 upgrade player"))?;
                game.host.inventory.configure(
                    &owned_actor,
                    &crate::contract::InventoryEntry {
                        item: game.weapon_item(Q1Weapon::Supershotgun),
                        count: 1.0,
                        capacity: 1.0,
                        count_policy: None,
                    },
                )?;
            }
            let weapon = game
                .player_ref(player)
                .map(|state| state.weapon)
                .unwrap_or(Q1Weapon::Axe);
            if weapon != Q1Weapon::Axe && weapon != Q1Weapon::Mg3Mjolnir && weapon != Q1Weapon::Shotgun {
                game.update_player(player, |state| state.weapon = Q1Weapon::Shotgun)?;
            }
        }
    }
    initialize_mg3_capacities(game, player)?;
    let owned = game
        .player_owned(player)
        .ok_or_else(|| crate::q1::q1_error("Missing Q1 upgrade player"))?;
    let weapon = game
        .player_ref(player)
        .map(|state| state.weapon)
        .unwrap_or(Q1Weapon::Axe);
    game.select_weapon(&owned, weapon)?;
    Ok(())
}

/// Resolves the capacity for an upgrade (`mg3Capacity`).
fn mg3_capacity(game: &mut Q1EntityServices, player: &ActorId, upgrade: &UpgradeDefinition) -> Result<f64, Q1Error> {
    if game.options().deathmatch != 0 {
        Ok(upgrade.deathmatch)
    } else {
        Ok(mg3_upgraded_maximum(
            upgrade.base,
            addon_player_number(game, player, upgrade.parm)?,
        ))
    }
}

/// Initializes mg3 capacities (`initializeMg3Capacities`).
pub fn initialize_mg3_capacities(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    let owned = game
        .player_owned(player)
        .ok_or_else(|| crate::q1::q1_error("Missing Q1 upgrade player"))?;
    for upgrade in UPGRADES {
        let capacity = mg3_capacity(game, player, &upgrade)?;
        match upgrade.ammo {
            None => {
                game.update_player(player, |state| state.max_health = capacity)?;
                if game.health(player) > capacity {
                    game.host.combat.set_health(&owned, capacity)?;
                }
            }
            Some(ammo) => {
                set_mg3_inventory_capacity(game, player, &ammo.to_string(), capacity)?;
                let count = game.host.inventory.count(player, &ammo.to_string()).min(capacity);
                game.host.inventory.configure(
                    &owned,
                    &crate::contract::InventoryEntry {
                        item: ammo.to_string(),
                        count,
                        capacity,
                        count_policy: None,
                    },
                )?;
            }
        }
    }
    Ok(())
}

/// Resolves the mg3 inventory capacity (`mg3InventoryCapacity`).
pub fn mg3_inventory_capacity(
    game: &mut Q1EntityServices,
    player: &ActorId,
    item: &crate::contract::ItemId,
) -> Result<Option<f64>, Q1Error> {
    let upgrade = UPGRADES.iter().find(|upgrade| upgrade.ammo == Some(item.as_str()));
    let Some(upgrade) = upgrade else {
        return Ok(None);
    };
    let saved = addon_player_word(game, player, &format!("ammo_{}_max", suffix(upgrade.upgrade)))?;
    match saved {
        None => Ok(Some(mg3_capacity(game, player, upgrade)?)),
        Some(saved) if !saved.is_finite() || saved <= 0.0 => {
            Err(crate::q1::q1_error("Invalid MG3 source ammo maximum"))
        }
        Some(saved) => Ok(Some(saved)),
    }
}

/// Writes an mg3 inventory capacity word (`setMg3InventoryCapacity`).
pub fn set_mg3_inventory_capacity(
    game: &mut Q1EntityServices,
    player: &ActorId,
    item: &crate::contract::ItemId,
    capacity: f64,
) -> Result<(), Q1Error> {
    let upgrade = UPGRADES.iter().find(|upgrade| upgrade.ammo == Some(item.as_str()));
    let Some(upgrade) = upgrade else {
        return Err(crate::q1::q1_error(format!("Missing MG3 capacity word for {item}")));
    };
    set_addon_player_number(game, player, &format!("ammo_{}_max", suffix(upgrade.upgrade)), capacity)
}

fn upgrade_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let owned = match game.player_owned(other) {
        Some(owned) => owned,
        None => return Ok(()),
    };
    let name = game
        .entity_ref(id)
        .map(|entity| entity.text("mg3.upgrade"))
        .unwrap_or_default();
    let upgrade = parse_upgrade(&name)?;
    let definition = definition(upgrade);
    let flag = game
        .entity_ref(id)
        .map(|entity| entity.number("upgrade_flag"))
        .unwrap_or(0.0) as i32;
    let flags = addon_player_number(game, other, definition.parm)? as i32;
    let collected = flags & flag != 0;
    let mut maximum = game.player_ref(other).map(|state| state.max_health).unwrap_or(0.0);
    if !collected {
        let capacity = match definition.ammo {
            None => None,
            Some(ammo) => Some(
                game.host
                    .inventory
                    .entries(other)
                    .into_iter()
                    .find(|entry| entry.item == ammo)
                    .map(|entry| entry.capacity)
                    .unwrap_or(definition.base),
            ),
        };
        set_addon_player_number(game, other, definition.parm, f64::from(flags | flag))?;
        match (definition.ammo, capacity) {
            (None, _) => {
                let grown = game
                    .player_ref(other)
                    .map(|state| fround(state.max_health + 10.0))
                    .unwrap_or(0.0);
                game.update_player(other, |state| state.max_health = grown)?;
                maximum = grown;
                let health = game.health(other);
                if health > 0.0 && health < maximum {
                    game.host.combat.set_health(&owned, (health + maximum).min(maximum))?;
                }
            }
            (Some(ammo), Some(capacity)) => {
                maximum = capacity + 10.0;
                set_mg3_inventory_capacity(game, other, &ammo.to_string(), maximum)?;
                game.host.inventory.configure(
                    &owned,
                    &crate::contract::InventoryEntry {
                        item: ammo.to_string(),
                        count: maximum,
                        capacity: maximum,
                        count_policy: None,
                    },
                )?;
            }
            _ => return Err(crate::q1::q1_error("Missing MG3 source ammo capacity")),
        }
    }
    for entry in game.host.inventory.entries(other) {
        if entry.item.starts_with("q1:ammo/") && entry.count > entry.capacity {
            let mut bounded = entry.clone();
            bounded.count = bounded.capacity;
            game.host.inventory.configure(&owned, &bounded)?;
        }
    }
    let weapon = game
        .player_ref(other)
        .map(|state| state.weapon)
        .unwrap_or(Q1Weapon::Axe);
    game.select_weapon(&owned, weapon)?;
    let mut args = vec![Q1MessageArg::Text(format!("$mg3_qc_upgrade_{}", definition.label))];
    if !collected {
        args.push(Q1MessageArg::Number(maximum));
    }
    finish_mg3_pickup(
        game,
        id,
        other,
        if collected {
            "$mg3_qc_upgrade_fail"
        } else {
            "$mg3_qc_upgrade_success"
        },
        if upgrade == Mg3Upgrade::Health {
            "player/tornoff2.wav"
        } else {
            "weapons/lock4.wav"
        },
        args,
    )
}

fn upgrade_start(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let name = game
        .entity_ref(id)
        .map(|entity| entity.text("mg3.upgrade"))
        .unwrap_or_default();
    let upgrade = parse_upgrade(&name)?;
    let flag = game
        .entity_ref(id)
        .map(|entity| entity.number("upgrade_flag"))
        .unwrap_or(0.0) as i32;
    let collected = (game.host.players)().iter().any(|player| {
        addon_player_number(game, player, definition(upgrade).parm)
            .map(|flags| flags as i32 & flag != 0)
            .unwrap_or(false)
    });
    if collected {
        crate::q1::addons::context::addon_alpha(game, id, 0.6)?;
    }
    game.update_entity(id, |entity| entity.solid = Q1Solid::Trigger)?;
    let touch = game.named.touch(&format!("{MG3_ITEM_PREFIX}upgrade_touch"))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))?;
    start_mg3_item(game, id)
}

fn spawn_upgrade(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let name = game
        .entity_ref(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    let upgrade = name
        .strip_prefix("item_upgrade_")
        .and_then(|suffix| parse_upgrade(suffix).ok());
    let Some(upgrade) = upgrade else {
        return Ok(());
    };
    let definition = definition(upgrade);
    if game
        .entity_ref(id)
        .map(|entity| entity.number("upgrade_flag"))
        .unwrap_or(0.0)
        == 0.0
    {
        let map = game.map_name.clone();
        set_addon_number(game, id, "upgrade_flag", f64::from(mg3_upgrade_flag(&map)))?;
    }
    game.update_entity(id, |entity| {
        entity
            .fields
            .insert(String::from("mg3.upgrade"), String::from(suffix(upgrade)));
        entity
            .fields
            .insert(String::from("netname"), format!("$mg3_qc_upgrade_{}", definition.label));
        entity.model = format!("progs/{}.mdl", definition.model);
    })?;
    if upgrade == Mg3Upgrade::Health {
        let origin = game.body(id)?.origin;
        game.set_origin(id, vadd(origin, Vec3 { x: 0.0, y: 0.0, z: 8.0 }))?;
    }
    game.set_bounds(
        id,
        Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: if upgrade == Mg3Upgrade::Health { -8.0 } else { 0.0 },
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: if upgrade == Mg3Upgrade::Health { 48.0 } else { 56.0 },
            },
        },
    )?;
    game.schedule(id, 0.5, &format!("{MG3_ITEM_PREFIX}upgrade_start"))
}

/// Registers mg3 capacity upgrades (`registerMg3Upgrades`).
pub fn register_mg3_upgrades(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        &format!("{MG3_ITEM_PREFIX}upgrade_touch"),
        Q1CallbackHandlers {
            touch: Some(upgrade_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{MG3_ITEM_PREFIX}upgrade_start"),
        Q1CallbackHandlers {
            action: Some(upgrade_start as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    for upgrade in [
        Mg3Upgrade::Health,
        Mg3Upgrade::Shells,
        Mg3Upgrade::Nails,
        Mg3Upgrade::Rockets,
        Mg3Upgrade::Cells,
    ] {
        game.register_spawn(&format!("item_upgrade_{}", suffix(upgrade)), spawn_upgrade)?;
    }
    Ok(())
}

/// Grants the next uncollected upgrade bit (`giveNextMg3Upgrade`).
pub fn give_next_mg3_upgrade(
    game: &mut Q1EntityServices,
    upgrade: Mg3Upgrade,
    player: &ActorId,
) -> Result<bool, Q1Error> {
    let flags = addon_player_number(game, player, definition(upgrade).parm)? as i32;
    let mut bit = 1;
    while bit <= 16384 {
        if flags & bit == 0 {
            let entity = game.create(&format!("item_upgrade_{}", suffix(upgrade)), None, None)?;
            game.update_entity(&entity, |entity| {
                entity
                    .fields
                    .insert(String::from("mg3.upgrade"), String::from(suffix(upgrade)));
            })?;
            set_addon_number(game, &entity, "upgrade_flag", f64::from(bit))?;
            upgrade_touch(game, &entity, player, None, None)?;
            return Ok(true);
        }
        bit *= 2;
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
        register_mg3_upgrades(game).expect("upgrades");
        guard
    }

    #[test]
    fn flags_and_maxima_match_donor() {
        assert_eq!(mg3_upgrade_flag("map1"), 1);
        assert_eq!(mg3_upgrade_flag("secret5"), 4096);
        assert_eq!(mg3_upgrade_flag("map2b"), 8192);
        assert_eq!(mg3_upgrade_flag("secret6"), 16384);
        assert_eq!(mg3_upgrade_flag("boss"), 0);
        assert_eq!(mg3_upgraded_maximum(50.0, 0.0), 50.0);
        assert_eq!(mg3_upgraded_maximum(50.0, 3.0), 70.0);
    }

    #[test]
    fn travel_round_trips_parms() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        set_addon_player_number(&mut game, &player, "parm11", 3.0).expect("parm");
        let bytes = capture_mg3_upgrade_travel(&mut game, &player);
        set_addon_player_number(&mut game, &player, "parm11", 0.0).expect("clear");
        restore_mg3_upgrade_travel(&mut game, &player, &bytes).expect("restore");
        assert_eq!(addon_player_number(&game, &player, "parm11"), Ok(3.0));
    }

    #[test]
    fn touch_collects_flag_once() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        let before = game
            .host
            .inventory
            .entries(&player)
            .into_iter()
            .find(|entry| entry.item == "q1:ammo/shells")
            .map(|entry| entry.capacity)
            .unwrap_or(50.0);
        assert!(give_next_mg3_upgrade(&mut game, Mg3Upgrade::Shells, &player).expect("give"));
        assert_eq!(addon_player_number(&game, &player, "parm11"), Ok(1.0));
        let capacity = mg3_inventory_capacity(&mut game, &player, &String::from("q1:ammo/shells")).expect("capacity");
        assert_eq!(capacity, Some(before + 10.0));
    }
}
