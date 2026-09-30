//! Q1 source impulse commands (`src/content/composition/q1/commands.ts`).
//!
//! `weapons.qc` `W_ChangeWeapon`, `CycleWeaponCommand` and base
//! `ImpulseCommands`. GPL-2.0-or-later.

use qa_core::identity::ActorId;

use crate::contract::{InventoryEntry, ItemId, RegularArmorState};
use crate::q1::base::provider::{campaign_read_flags, campaign_write_flags};
use crate::q1::composition::types::{Q1CompositionEvent, Q1CompositionServices, Q1SourceProgram};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{Q1Edition, Q1Powerup, Q1Weapon, WEAPONS};
use crate::q1::Q1Error;

/// Whether the player can afford one shot (`ammunition`).
fn ammunition(game: &Q1EntityServices, player: &ActorId, weapon: Q1Weapon) -> bool {
    let ammo = game.weapon_ammo(weapon);
    let minimum = if weapon == Q1Weapon::Supershotgun || weapon == Q1Weapon::Supernailgun {
        2.0
    } else {
        1.0
    };
    ammo.is_none()
        || game
            .host
            .inventory
            .count(player, ammo.as_ref().unwrap_or(&String::new()))
            >= minimum
}

/// Select by impulse or cycle (`q1WeaponImpulse`).
pub fn q1_weapon_impulse(
    game: &mut Q1EntityServices,
    player: &crate::q1::foundation::types::Q1PlayerState,
    impulse: i32,
) -> Result<bool, Q1Error> {
    if (1..=8).contains(&impulse) {
        let base = WEAPONS[(impulse - 1) as usize];
        let weapon = Q1Weapon::from(base);
        if game.host.inventory.count(player.actor.id(), &game.weapon_item(weapon)) == 0.0 {
            let text = if game.options().edition == Q1Edition::Classic {
                "no weapon.\n"
            } else {
                "$qc_no_weapon"
            };
            game.message(Some(player.actor.id()), text, false, Vec::new());
        } else if !ammunition(game, player.actor.id(), weapon) {
            let text = if game.options().edition == Q1Edition::Classic {
                "not enough ammo.\n"
            } else {
                "$qc_not_enough_ammo"
            };
            game.message(Some(player.actor.id()), text, false, Vec::new());
        } else {
            game.select_weapon(&player.actor, weapon)?;
        }
        return Ok(true);
    }
    if impulse == 10 || impulse == 12 {
        let index = WEAPONS
            .iter()
            .position(|weapon| Q1Weapon::from(*weapon) == player.weapon)
            .map_or(-1, |index| index as i32);
        let direction = if impulse == 10 { 1 } else { -1 };
        let length = WEAPONS.len() as i32;
        for offset in 1..=length {
            let weapon = WEAPONS[((index + direction * offset + length * 2) % length) as usize];
            let weapon = Q1Weapon::from(weapon);
            if ammunition(game, player.actor.id(), weapon) && game.select_weapon(&player.actor, weapon)? {
                break;
            }
        }
        return Ok(true);
    }
    Ok(false)
}

/// Write an inventory count (`setCount`).
fn set_count(
    game: &mut Q1EntityServices,
    player: &crate::q1::foundation::types::Q1PlayerState,
    item: &ItemId,
    count: f64,
    capacity: f64,
) -> Result<(), Q1Error> {
    let previous = game
        .host
        .inventory
        .entries(player.actor.id())
        .into_iter()
        .find(|entry| entry.item == *item);
    let entry = previous.map_or(
        InventoryEntry {
            item: item.clone(),
            count,
            capacity,
            count_policy: None,
        },
        |entry| InventoryEntry { count, ..entry },
    );
    game.host.inventory.configure(&player.actor, &entry)
}

/// Base source impulses (`baseQ1Impulse`).
pub fn base_q1_impulse(
    game: &mut Q1EntityServices,
    services: &mut dyn Q1CompositionServices,
    program: Q1SourceProgram,
    player: &crate::q1::foundation::types::Q1PlayerState,
    impulse: i32,
) -> Result<bool, Q1Error> {
    if impulse == 11 {
        let flags = campaign_read_flags(game)?;
        campaign_write_flags(game, f64::from(flags).mul_add(2.0, 1.0) as f32 as i32)?;
        return Ok(true);
    }
    if impulse == 9 {
        if (game.options().deathmatch != 0 || game.options().coop)
            && (game.options().edition == Q1Edition::Classic || services.cvar("sv_cheats") == 0.0)
        {
            return Ok(true);
        }
        let foreign_arsenal = services.cheat_arsenal(player.actor.id(), None);
        if !foreign_arsenal {
            for base in WEAPONS {
                let item = game.weapon_item(Q1Weapon::from(base));
                set_count(game, player, &item, 1.0, 1.0)?;
            }
            set_count(game, player, &String::from("q1:ammo/shells"), 100.0, 100.0)?;
            set_count(game, player, &String::from("q1:ammo/nails"), 200.0, 200.0)?;
            set_count(game, player, &String::from("q1:ammo/rockets"), 100.0, 100.0)?;
            set_count(game, player, &String::from("q1:ammo/cells"), 200.0, 100.0)?;
        }
        set_count(game, player, &String::from("q1:key/silver"), 1.0, 1.0)?;
        set_count(game, player, &String::from("q1:key/gold"), 1.0, 1.0)?;
        if program == Q1SourceProgram::Ctf {
            set_count(game, player, &String::from("q1:ctf/weapon/grapple"), 1.0, 1.0)?;
        }
        if game.options().edition == Q1Edition::Rerelease
            && (program == Q1SourceProgram::Id1 || program == Q1SourceProgram::Ctf)
        {
            game.host.combat.set_regular_armor(
                &player.actor,
                &RegularArmorState::Q1 {
                    points: 200.0,
                    absorption: 0.8,
                    item: String::from("q1:item_armorInv"),
                },
            )?;
        }
        if !foreign_arsenal {
            let item = game.weapon_item(Q1Weapon::Rocketlauncher);
            services.select_weapon(player.actor.id(), &item);
        }
        return Ok(true);
    }
    if impulse == 255 {
        let source_cheats = game.options().edition == Q1Edition::Rerelease
            && (program == Q1SourceProgram::Id1 || program == Q1SourceProgram::Ctf);
        if (source_cheats && services.cvar("sv_cheats") == 0.0)
            || (!source_cheats && (game.options().deathmatch != 0 || game.options().coop))
        {
            return Ok(true);
        }
        game.give_powerup(player.actor.id(), Q1Powerup::Quad, 30.0)?;
        services.emit(Q1CompositionEvent::DeveloperMessage {
            text: String::from("quad cheat\n"),
        });
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::attach_test_player;
    use crate::q1::base::provider::{campaign_read_flags, Q1BaseGuard, Q1BaseOptions};
    use crate::q1::composition::types::FakeCompositionServices;
    use crate::q1::missionpacks::types::test_game;

    /// Fresh game; callers register base and attach players on the
    /// final binding since registries key by game address.
    fn setup() -> Q1EntityServices {
        test_game()
    }

    fn armed(game: &mut Q1EntityServices) -> (Q1BaseGuard, ActorId) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        let player = attach_test_player(game);
        (guard, player)
    }

    fn grant(game: &mut Q1EntityServices, player: &ActorId, item: &str, count: f64) {
        let owned = game.host.actors.resolve_owned(player).expect("owned");
        game.host
            .inventory
            .configure(
                &owned,
                &InventoryEntry {
                    item: ItemId::from(item),
                    count,
                    capacity: count,
                    count_policy: None,
                },
            )
            .expect("grant");
    }

    #[test]
    fn weapon_impulse_selects_owned_weapon() {
        let mut game = setup();
        let (_guard, player) = armed(&mut game);
        grant(&mut game, &player, "q1:weapon/grenadelauncher", 1.0);
        grant(&mut game, &player, "q1:ammo/rockets", 5.0);
        let state = game.player_ref(&player).cloned().expect("player");
        assert!(q1_weapon_impulse(&mut game, &state, 6).expect("impulse"));
        assert_eq!(
            game.player_ref(&player).expect("player").weapon,
            Q1Weapon::Grenadelauncher
        );
    }

    #[test]
    fn weapon_impulse_consumes_without_ammo() {
        let mut game = setup();
        let (_guard, player) = armed(&mut game);
        grant(&mut game, &player, "q1:weapon/grenadelauncher", 1.0);
        let state = game.player_ref(&player).cloned().expect("player");
        assert!(q1_weapon_impulse(&mut game, &state, 6).expect("impulse"));
        assert_ne!(
            game.player_ref(&player).expect("player").weapon,
            Q1Weapon::Grenadelauncher
        );
    }

    #[test]
    fn weapon_impulse_cycles() {
        let mut game = setup();
        let (_guard, player) = armed(&mut game);
        grant(&mut game, &player, "q1:weapon/shotgun", 1.0);
        grant(&mut game, &player, "q1:ammo/shells", 5.0);
        let state = game.player_ref(&player).cloned().expect("player");
        assert!(q1_weapon_impulse(&mut game, &state, 10).expect("cycle"));
        assert!(q1_weapon_impulse(&mut game, &state, 12).expect("cycle"));
        assert!(!q1_weapon_impulse(&mut game, &state, 13).expect("other"));
    }

    #[test]
    fn base_impulse_eleven_doubles_flags() {
        let mut game = setup();
        let (_guard, player) = armed(&mut game);
        let mut services = FakeCompositionServices::new();
        let state = game.player_ref(&player).cloned().expect("player");
        assert!(base_q1_impulse(&mut game, &mut services, Q1SourceProgram::Id1, &state, 11).expect("impulse"));
        assert_eq!(campaign_read_flags(&game).expect("flags"), 1);
    }

    #[test]
    fn base_impulse_nine_grants_arsenal() {
        let mut game = setup();
        let (_guard, player) = armed(&mut game);
        let mut services = FakeCompositionServices::new();
        let state = game.player_ref(&player).cloned().expect("player");
        assert!(base_q1_impulse(&mut game, &mut services, Q1SourceProgram::Id1, &state, 9).expect("impulse"));
        assert_eq!(
            game.host.inventory.count(&player, &String::from("q1:weapon/lightning")),
            1.0
        );
        assert_eq!(
            game.host.inventory.count(&player, &String::from("q1:ammo/cells")),
            200.0
        );
        assert_eq!(game.host.inventory.count(&player, &String::from("q1:key/gold")), 1.0);
        assert!(services
            .selections
            .iter()
            .any(|(actor, item)| actor == &player && item == "q1:weapon/rocketlauncher"));
    }

    #[test]
    fn base_impulse_quad_cheat_emits() {
        let mut game = setup();
        let (_guard, player) = armed(&mut game);
        let mut services = FakeCompositionServices::new();
        let state = game.player_ref(&player).cloned().expect("player");
        assert!(base_q1_impulse(&mut game, &mut services, Q1SourceProgram::Id1, &state, 255).expect("impulse"));
        assert!(game
            .player_ref(&player)
            .expect("player")
            .powerups
            .contains_key(&Q1Powerup::Quad));
        assert!(matches!(
            services.events.as_slice(),
            [Q1CompositionEvent::DeveloperMessage { text }] if text == "quad cheat\n"
        ));
        assert!(!base_q1_impulse(&mut game, &mut services, Q1SourceProgram::Id1, &state, 7).expect("other"));
    }
}
