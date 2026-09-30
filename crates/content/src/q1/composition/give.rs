//! Q1 source give command (`src/content/composition/q1/give.ts`).
//!
//! Host give syntax plus shared named/category grants, using the
//! selected inventory and real item touches.

use qa_core::identity::ActorId;
use qa_core::numeric::native_atoi;

use crate::contract::PickupSelection;
use crate::contract::{InventoryEntry, ItemId, PickupAmmoGrant, PickupSupplyOffer, PickupWeaponGrant};
use crate::q1::composition::types::Q1CompositionServices;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::TouchContact;
use crate::q1::foundation::pickups::give_pickup;
use crate::q1::foundation::types::{Q1Solid, Q1Weapon, WEAPONS};
use crate::q1::{q1_error, Q1Error};

/// Configure an inventory entry (`configure`).
fn configure(
    game: &mut Q1EntityServices,
    actor: &ActorId,
    item: &ItemId,
    count: f64,
    capacity: f64,
) -> Result<(), Q1Error> {
    let owned = game
        .host
        .actors
        .resolve_owned(actor)
        .ok_or_else(|| q1_error("Q1 give requires an admitted player"))?;
    let previous = game
        .host
        .inventory
        .entries(actor)
        .into_iter()
        .find(|entry| entry.item == *item);
    let entry = InventoryEntry {
        item: item.clone(),
        count: count.max(0.0),
        capacity: capacity
            .max(previous.as_ref().map_or(0.0, |entry| entry.capacity))
            .max(count),
        count_policy: previous.and_then(|entry| entry.count_policy),
    };
    game.host.inventory.configure(&owned, &entry)
}

/// Grant ammunition, mapping through pickup admission (`ammoCount`).
/// An unaccepted preview falls back to the direct grant, matching the
/// donor's undefined-preview path.
fn ammo_count(
    game: &mut Q1EntityServices,
    services: &mut dyn Q1CompositionServices,
    actor: &ActorId,
    item: &ItemId,
    count: f64,
) -> Result<(), Q1Error> {
    if game.pickup_admission.is_none() && services.give_selected_item(actor, &[item.clone(), count.to_string()]) {
        return Ok(());
    }
    let mapped = game.pickup_admission.as_ref().map(|admission| {
        admission.preview(
            actor,
            &PickupSupplyOffer::Ammo(PickupAmmoGrant {
                item: item.clone(),
                amount: count.max(1.0),
            }),
        )
    });
    match mapped {
        Some(mapped) if mapped.accepted => {
            for receipt in &mapped.ammo {
                configure(game, actor, &receipt.item, count, count)?;
            }
            Ok(())
        }
        _ => configure(game, actor, item, count, count),
    }
}

/// Ordered weapon roster: base weapons first, then registered extras
/// in donor id order for determinism.
fn roster(game: &Q1EntityServices) -> Vec<Q1Weapon> {
    let mut weapons: Vec<Q1Weapon> = WEAPONS.iter().map(|weapon| Q1Weapon::from(*weapon)).collect();
    let mut extras: Vec<Q1Weapon> = game
        .registered_weapons
        .keys()
        .copied()
        .filter(|weapon| !weapons.contains(weapon))
        .collect();
    extras.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    weapons.extend(extras);
    weapons
}

/// Normalize a weapon name for matching (`normalize`).
fn normalize(name: &str) -> String {
    name.chars().filter(|cell| *cell != ' ' && *cell != '_').collect()
}

/// Grant items by host give syntax (`giveQ1`).
pub fn give_q1(
    game: &mut Q1EntityServices,
    services: &mut dyn Q1CompositionServices,
    actor: &ActorId,
    args: &[String],
) -> Result<(), Q1Error> {
    let player = game
        .player_ref(actor)
        .cloned()
        .ok_or_else(|| q1_error("Q1 give requires an admitted player"))?;
    let input = args
        .first()
        .map(|arg| arg.to_lowercase())
        .ok_or_else(|| q1_error("Usage: give <all|weapons|ammo|health|armor|keys|item> [amount]"))?;
    let amount = args
        .get(1)
        .map(|arg| native_atoi(arg).map_err(|error| q1_error(error.to_string())))
        .transpose()?;
    let all = input == "all";
    let weapons = roster(game);
    if all || input == "health" || input == "h" {
        let health = amount.map_or_else(
            || {
                if input == "h" {
                    0.0
                } else {
                    player.max_health
                }
            },
            f64::from,
        );
        game.host.combat.set_health(&player.actor, health)?;
        if !all {
            return Ok(());
        }
    }
    if all || input == "armor" || input == "a" {
        let points = amount.map_or_else(|| if input == "a" { 0.0 } else { 200.0 }, f64::from);
        game.host.combat.set_regular_armor(
            &player.actor,
            &if points <= 0.0 {
                crate::contract::RegularArmorState::None
            } else {
                crate::contract::RegularArmorState::Q1 {
                    points,
                    absorption: if points > 150.0 {
                        0.8
                    } else if points > 100.0 {
                        0.6
                    } else {
                        0.3
                    },
                    item: ItemId::from(if points > 150.0 {
                        "q1:item_armorInv"
                    } else if points > 100.0 {
                        "q1:item_armor2"
                    } else {
                        "q1:item_armor1"
                    }),
                }
            },
        )?;
        if !all {
            return Ok(());
        }
    }
    if all || input == "weapons" {
        if !services.cheat_arsenal(
            actor,
            Some(crate::q1::composition::types::Q1CompositionCheatCategory::Weapons),
        ) {
            for weapon in &weapons {
                let item = game.weapon_item(*weapon);
                configure(game, actor, &item, 1.0, 1.0)?;
            }
        }
        if !all {
            return Ok(());
        }
    }
    if all || input == "ammo" {
        if !services.cheat_arsenal(
            actor,
            Some(crate::q1::composition::types::Q1CompositionCheatCategory::Ammo),
        ) {
            let mut granted: Vec<ItemId> = Vec::new();
            for weapon in &weapons {
                if let Some(ammo) = game.weapon_ammo(*weapon) {
                    if !granted.contains(&ammo) {
                        granted.push(ammo);
                    }
                }
            }
            for item in &granted {
                let count = if item.contains("nails") { 200.0 } else { 100.0 };
                configure(game, actor, item, count, count)?;
            }
        }
        if !all {
            return Ok(());
        }
    }
    if all || input == "keys" {
        configure(game, actor, &String::from("q1:key/silver"), 1.0, 1.0)?;
        configure(game, actor, &String::from("q1:key/gold"), 1.0, 1.0)?;
        return Ok(());
    }
    if input == "items" {
        for item in ["quad", "pent", "ring", "suit"] {
            give_q1(game, services, actor, &[String::from(item)])?;
        }
        return Ok(());
    }
    let shorthand = match input.as_str() {
        "s" => Some("q1:ammo/shells"),
        "n" => Some("q1:ammo/nails"),
        "r" => Some("q1:ammo/rockets"),
        "c" => Some("q1:ammo/cells"),
        "l" => Some("rogue:ammo/lava-nails"),
        "m" => Some("rogue:ammo/multi-rockets"),
        "p" => Some("rogue:ammo/plasma"),
        _ => None,
    };
    if let Some(shorthand) = shorthand {
        if !weapons
            .iter()
            .any(|weapon| game.weapon_ammo(*weapon).as_deref() == Some(shorthand))
        {
            return Err(q1_error(format!("Ammo {input} is unavailable in this arsenal")));
        }
        return ammo_count(
            game,
            services,
            actor,
            &String::from(shorthand),
            amount.map_or(0.0, f64::from),
        );
    }
    let hipnotic = game.registered_weapons.contains_key(&Q1Weapon::HipnoticLaser);
    let numeric_tail = args.last().is_some_and(|last| {
        let digits = last.strip_prefix('-').unwrap_or(last);
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    });
    let named_input = (if args.len() > 1 && numeric_tail {
        &args[..args.len() - 1]
    } else {
        args
    })
    .join(" ")
    .to_lowercase();
    let numbered = if hipnotic && input == "6a" {
        Some(Q1Weapon::HipnoticProximity)
    } else if hipnotic && input == "9" {
        Some(Q1Weapon::HipnoticLaser)
    } else if hipnotic && input == "0" {
        Some(Q1Weapon::HipnoticMjolnir)
    } else if input.len() == 1 && ("2"..="8").contains(&input.as_str()) {
        WEAPONS
            .get(input.as_bytes()[0] as usize - b'0' as usize - 1)
            .map(|weapon| Q1Weapon::from(*weapon))
    } else {
        None
    };
    let weapon = weapons.iter().copied().find(|weapon| {
        Some(*weapon) == numbered
            || normalize(weapon.as_str()) == normalize(&named_input)
            || normalize(game.weapon_item(*weapon).as_str()) == normalize(&named_input)
            || normalize(format!("weapon_{}", weapon.as_str()).as_str()) == normalize(&named_input)
    });
    if let Some(weapon) = weapon {
        let item = game.weapon_item(weapon);
        if game.pickup_admission.is_none() && services.give_selected_item(actor, std::slice::from_ref(&item)) {
            return Ok(());
        }
        if game.pickup_admission.is_none() {
            return configure(game, actor, &item, 1.0, 1.0);
        }
        if let Some(admission) = game.pickup_admission.as_ref() {
            admission.weapon(
                &player.actor,
                &PickupWeaponGrant { item, ammo: Vec::new() },
                PickupSelection::Never,
            );
        }
        return Ok(());
    }
    let mut ammo_items: Vec<ItemId> = Vec::new();
    for weapon in &weapons {
        if let Some(ammo) = game.weapon_ammo(*weapon) {
            if !ammo_items.contains(&ammo) {
                ammo_items.push(ammo);
            }
        }
    }
    if let Some(ammo) = ammo_items
        .into_iter()
        .find(|item| item == &input || item.rsplit('/').next() == Some(input.as_str()))
    {
        let count = amount.map_or_else(|| game.host.inventory.count(actor, &ammo) + 20.0, f64::from);
        return ammo_count(game, services, actor, &ammo, count);
    }
    let classname = match input.as_str() {
        "quad" => "item_artifact_super_damage",
        "pent" => "item_artifact_invulnerability",
        "ring" => "item_artifact_invisibility",
        "suit" => "item_artifact_envirosuit",
        _ => input.as_str(),
    };
    if services.give_selected_item(actor, args) {
        return Ok(());
    }
    if !classname.starts_with("item_") && !classname.starts_with("weapon_") {
        return Err(q1_error(format!("Unknown Q1 item: {}", args.join(" "))));
    }
    let item = game.create(classname, None, None)?;
    let outcome = (|| {
        if give_pickup(game, &item, actor)? {
            return Ok(());
        }
        game.spawn_entity(&item, None)?;
        let touch = game.entity_ref(&item).and_then(|entity| entity.touch.clone());
        if !game.is_live(&item) || touch.is_none() {
            return Err(q1_error(format!("Item is not giveable: {classname}")));
        }
        game.cancel(&item);
        game.update_entity(&item, |entity| entity.solid = Q1Solid::Trigger)?;
        let owned = game
            .host
            .actors
            .resolve_owned(&item)
            .ok_or_else(|| q1_error(format!("Item is not giveable: {classname}")))?;
        game.fire_touch(&TouchContact {
            self_actor: owned,
            other: actor.clone(),
            plane: None,
            surface: None,
        })
    })();
    if game.is_live(&item) {
        game.remove(&item)?;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::attach_test_player;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::composition::types::FakeCompositionServices;
    use crate::q1::missionpacks::types::test_game;

    /// Fresh game; callers register base and attach players on the
    /// final binding since registries key by game address.
    fn setup() -> (Q1EntityServices, FakeCompositionServices) {
        (test_game(), FakeCompositionServices::new())
    }

    fn armed(game: &mut Q1EntityServices) -> (Q1BaseGuard, ActorId) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        let player = attach_test_player(game);
        (guard, player)
    }

    #[test]
    fn give_all_grants_everything() {
        let (mut game, mut services) = setup();
        let (_guard, player) = armed(&mut game);
        give_q1(&mut game, &mut services, &player, &[String::from("all")]).expect("give");
        assert_eq!(game.host.combat.read(&player).expect("combat").health, 100.0);
        assert_eq!(
            game.host.inventory.count(&player, &String::from("q1:weapon/lightning")),
            1.0
        );
        assert_eq!(
            game.host.inventory.count(&player, &String::from("q1:ammo/nails")),
            200.0
        );
        assert_eq!(game.host.inventory.count(&player, &String::from("q1:key/silver")), 1.0);
    }

    #[test]
    fn give_health_and_armor() {
        let (mut game, mut services) = setup();
        let (_guard, player) = armed(&mut game);
        give_q1(
            &mut game,
            &mut services,
            &player,
            &[String::from("health"), String::from("250")],
        )
        .expect("give");
        assert_eq!(game.host.combat.read(&player).expect("combat").health, 250.0);
        give_q1(&mut game, &mut services, &player, &[String::from("a")]).expect("give");
        assert!(matches!(
            game.host.combat.read(&player).expect("combat").armor.regular,
            crate::contract::RegularArmorState::None
        ));
    }

    #[test]
    fn give_named_weapon_and_shorthand() {
        let (mut game, mut services) = setup();
        let (_guard, player) = armed(&mut game);
        give_q1(&mut game, &mut services, &player, &[String::from("supershotgun")]).expect("give");
        assert_eq!(
            game.host
                .inventory
                .count(&player, &String::from("q1:weapon/supershotgun")),
            1.0
        );
        give_q1(&mut game, &mut services, &player, &[String::from("s")]).expect("give");
        assert_eq!(game.host.inventory.count(&player, &String::from("q1:ammo/shells")), 0.0);
        give_q1(&mut game, &mut services, &player, &[String::from("shells")]).expect("give");
        assert_eq!(
            game.host.inventory.count(&player, &String::from("q1:ammo/shells")),
            20.0
        );
    }

    #[test]
    fn give_rejects_unknown() {
        let (mut game, mut services) = setup();
        let (_guard, player) = armed(&mut game);
        let error = give_q1(&mut game, &mut services, &player, &[String::from("toaster")]).expect_err("unknown");
        assert_eq!(error.to_string(), "Unknown Q1 item: toaster");
        let error = give_q1(&mut game, &mut services, &player, &[]).expect_err("usage");
        assert!(error.to_string().starts_with("Usage: give"));
    }
}
