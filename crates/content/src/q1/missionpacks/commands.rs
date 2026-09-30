//! Mission-pack impulse commands (src/content/q1/missionpacks/commands.ts).

use std::rc::Rc;

use qa_core::identity::ActorId;

use crate::contract::{InventoryEntry, ItemId};
use crate::q1::base::provider::{campaign_read_flags, campaign_write_flags};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::types::{weapon_item, Q1Edition, Q1Powerup, Q1Weapon, WEAPONS};
use crate::q1::Q1Error;

use super::messages::mission_message;
use super::player::MissionPackPlayers;
use super::types::{fround, Q1MissionPack, MISSION_WEAPONS};

/// Cheat arsenal category (`"weapons" | "ammo"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheatArsenalCategory {
    /// Weapons.
    Weapons,
    /// Ammunition.
    Ammo,
}

/// Mission-pack command options (`MissionPackCommandOptions`).
#[derive(Clone, Default)]
#[allow(clippy::type_complexity)]
pub struct MissionPackCommandOptions {
    /// Session cheat-arsenal override.
    pub cheat_arsenal: Option<Rc<dyn Fn(&ActorId, CheatArsenalCategory) -> bool>>,
    /// Session cheat permission probe.
    pub cheats_allowed: Option<Rc<dyn Fn() -> bool>>,
    /// Developer log sink.
    pub developer_message: Option<Rc<dyn Fn(&str)>>,
}

impl std::fmt::Debug for MissionPackCommandOptions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MissionPackCommandOptions")
            .finish_non_exhaustive()
    }
}

/// Set an inventory entry count (`setCount`).
fn set_count(
    game: &mut Q1EntityServices,
    actor: &qa_core::identity::OwnedActor,
    item: &ItemId,
    count: f64,
    capacity: f64,
) -> Result<(), Q1Error> {
    let id = actor.id().clone();
    match game
        .host
        .inventory
        .entries(&id)
        .into_iter()
        .find(|candidate| candidate.item == *item)
    {
        Some(mut entry) => {
            entry.count = count;
            game.host.inventory.configure(actor, &entry)
        }
        None => game.host.inventory.configure(
            actor,
            &InventoryEntry {
                item: item.clone(),
                count,
                capacity,
                count_policy: None,
            },
        ),
    }
}

/// Handle a mission-pack impulse (`missionPackCommand`).
/// The donor takes the base content handle; this port reaches the campaign
/// through the base free functions since there is no `Q1Base` handle.
pub fn mission_pack_command(
    game: &mut Q1EntityServices,
    players: &MissionPackPlayers,
    player: &ActorId,
    pack: Q1MissionPack,
    impulse: i32,
    options: &MissionPackCommandOptions,
) -> Result<bool, Q1Error> {
    let state = match game.player_ref(player) {
        Some(state) => state.clone(),
        None => return Ok(false),
    };
    let edition = game.options().edition;
    let deathmatch = game.options().deathmatch;
    let coop = game.options().coop;
    let multiplayer = deathmatch != 0 || coop;
    if impulse == 9 {
        if multiplayer
            && (edition == Q1Edition::Classic
                || !(options
                    .cheats_allowed
                    .as_ref()
                    .map(|allowed| allowed())
                    .unwrap_or(false)))
        {
            return Ok(true);
        }
        let selected_weapons = options
            .cheat_arsenal
            .as_ref()
            .map(|arsenal| arsenal(player, CheatArsenalCategory::Weapons))
            .unwrap_or(false);
        if !selected_weapons {
            for weapon in WEAPONS.iter().map(|weapon| Q1Weapon::from(*weapon)) {
                set_count(game, &state.actor, &weapon_item(weapon), 1.0, 1.0)?;
            }
            let prefix = pack.as_str();
            for weapon in MISSION_WEAPONS
                .iter()
                .filter(|weapon| weapon.id.as_str().starts_with(prefix))
            {
                set_count(game, &state.actor, &weapon_item(Q1Weapon::from(weapon.id)), 1.0, 1.0)?;
            }
        }
        set_count(game, &state.actor, &ItemId::from("q1:key/silver"), 1.0, 1.0)?;
        set_count(game, &state.actor, &ItemId::from("q1:key/gold"), 1.0, 1.0)?;
        if !(options
            .cheat_arsenal
            .as_ref()
            .map(|arsenal| arsenal(player, CheatArsenalCategory::Ammo))
            .unwrap_or(false))
        {
            set_count(game, &state.actor, &ItemId::from("q1:ammo/shells"), 100.0, 100.0)?;
            set_count(game, &state.actor, &ItemId::from("q1:ammo/nails"), 200.0, 200.0)?;
            set_count(game, &state.actor, &ItemId::from("q1:ammo/rockets"), 100.0, 100.0)?;
            set_count(game, &state.actor, &ItemId::from("q1:ammo/cells"), 200.0, 100.0)?;
            if pack == Q1MissionPack::Rogue {
                set_count(game, &state.actor, &ItemId::from("rogue:ammo/lava-nails"), 200.0, 200.0)?;
                set_count(
                    game,
                    &state.actor,
                    &ItemId::from("rogue:ammo/multi-rockets"),
                    100.0,
                    100.0,
                )?;
                set_count(game, &state.actor, &ItemId::from("rogue:ammo/plasma"), 100.0, 100.0)?;
            }
        }
        if !selected_weapons {
            game.select_weapon(&state.actor, Q1Weapon::Rocketlauncher)?;
        }
        return Ok(true);
    }
    if impulse == 11 {
        let flags = campaign_read_flags(game)?;
        campaign_write_flags(game, fround(f64::from(flags) * 2.0 + 1.0) as i32)?;
        return Ok(true);
    }
    if impulse == 255 || pack == Q1MissionPack::Hipnotic && (impulse == 200 || impulse == 201) {
        if multiplayer {
            return Ok(true);
        }
        players.powerup(
            game,
            player,
            if impulse == 200 {
                Q1Powerup::HipnoticWetsuit
            } else if impulse == 201 {
                Q1Powerup::HipnoticEmpathy
            } else {
                Q1Powerup::Quad
            },
            30.0,
        )?;
        let key = if impulse == 200 {
            "$qc_wetsuit_cheat"
        } else if impulse == 201 {
            "$qc_empathy_cheat"
        } else {
            "$qc_quad_cheat"
        };
        if pack == Q1MissionPack::Rogue {
            if let Some(developer_message) = options.developer_message.as_ref() {
                developer_message(if edition == Q1Edition::Classic {
                    "quad cheat\n"
                } else {
                    key
                });
            }
        } else {
            mission_message(game, None, key);
        }
        return Ok(true);
    }
    if pack != Q1MissionPack::Hipnotic {
        return Ok(false);
    }
    if impulse == 205 {
        if multiplayer {
            return Ok(true);
        }
        mission_message(game, None, "$qc_genocide_cheat");
        let Some(world) = game.world.clone() else {
            return Ok(true);
        };
        let targets: Vec<ActorId> = game
            .entities
            .values()
            .filter(|entity| entity.monster.is_some())
            .map(|entity| entity.actor.id().clone())
            .collect();
        for target in &targets {
            if game.health(target) > 0.0 {
                let amount = game.health(target) + 10.0;
                let _ = game.damage(target, Some(&world), Some(&world), amount, &Q1DamageParams::default());
            }
        }
        return Ok(true);
    }
    if impulse == 202 || impulse == 203 {
        let mut ordinal = 0;
        for id in game.entity_ids() {
            if Some(&id) == game.world.as_ref() {
                continue;
            }
            ordinal += 1;
            if impulse == 203 && game.health(&id) <= 0.0 {
                continue;
            }
            let (classname, origin) = match game.entity_ref(&id) {
                Some(entity) => (entity.classname.clone(), game.body(&id)?.origin),
                None => continue,
            };
            if let Some(developer_message) = options.developer_message.as_ref() {
                if impulse == 202 {
                    developer_message(&format!("{ordinal} {classname}\n"));
                } else {
                    developer_message(&format!(
                        "{ordinal} {classname} '{} {} {}'\n--------------------\n",
                        origin.x, origin.y, origin.z
                    ));
                }
            }
        }
        return Ok(true);
    }
    if impulse == 206 {
        if let Some(world) = game.world.clone() {
            let next = 1.0
                - game
                    .entity_ref(&world)
                    .map(|world| world.number("hipnotic:dump-coordinates"))
                    .unwrap_or(0.0);
            game.update_entity(&world, |world| {
                world
                    .fields
                    .insert("hipnotic:dump-coordinates".to_string(), next.to_string());
            })?;
            if next == 1.0 {
                mission_message(game, None, "$qc_dump_player_loc");
            }
        }
        return Ok(true);
    }
    Ok(false)
}

/// Dump player coordinates while the debug flag is set
/// (`dumpMissionPackCoordinates`).
pub fn dump_mission_pack_coordinates(game: &mut Q1EntityServices, player: &ActorId) {
    let state = match game.player_ref(player) {
        Some(state) => state.clone(),
        None => return,
    };
    let dumping = game
        .world
        .as_ref()
        .and_then(|world| game.entity_ref(world))
        .map(|world| world.number("hipnotic:dump-coordinates"))
        != Some(1.0);
    if dumping || game.time < state.attack_finished {
        return;
    }
    let body = game
        .host
        .check_client(&state.actor)
        .as_ref()
        .and_then(|client| game.host.bodies.read(client));
    let Some(body) = body else {
        return;
    };
    game.message(
        None,
        &format!("Player: '{} {} {}'\n", body.origin.x, body.origin.y, body.origin.z),
        false,
        Vec::new(),
    );
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::super::types::test_game;
    use super::*;
    use crate::q1::foundation::entity_services::Q1AttachOptions;

    fn attached_player(game: &mut Q1EntityServices) -> ActorId {
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
        player
    }

    #[test]
    fn cheat_arsenal_grants_weapons_and_ammo() {
        let mut game = test_game();
        let players = MissionPackPlayers::for_pack(Q1MissionPack::Hipnotic);
        let player = attached_player(&mut game);
        let options = MissionPackCommandOptions::default();
        assert!(
            mission_pack_command(&mut game, &players, &player, Q1MissionPack::Hipnotic, 9, &options).expect("impulse")
        );
        assert_eq!(
            game.host
                .inventory
                .count(&player, &weapon_item(Q1Weapon::Rocketlauncher)),
            1.0
        );
        assert_eq!(
            game.host.inventory.count(&player, &ItemId::from("q1:ammo/shells")),
            100.0
        );
        assert_eq!(
            game.player_ref(&player).expect("state").weapon,
            Q1Weapon::Rocketlauncher
        );
    }

    #[test]
    fn power_cheat_grants_timed_quad() {
        let mut game = test_game();
        let players = MissionPackPlayers::for_pack(Q1MissionPack::Rogue);
        let player = attached_player(&mut game);
        let options = MissionPackCommandOptions::default();
        assert!(
            mission_pack_command(&mut game, &players, &player, Q1MissionPack::Rogue, 255, &options).expect("impulse")
        );
        assert!(game
            .player_ref(&player)
            .expect("state")
            .powerups
            .contains_key(&Q1Powerup::Quad));
        assert!(
            !mission_pack_command(&mut game, &players, &player, Q1MissionPack::Rogue, 206, &options).expect("impulse")
        );
    }

    #[test]
    fn entity_dump_lists_world_skipped_entities() {
        let mut game = test_game();
        let players = MissionPackPlayers::for_pack(Q1MissionPack::Hipnotic);
        let player = attached_player(&mut game);
        let logged: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&logged);
        let options = MissionPackCommandOptions {
            developer_message: Some(Rc::new(move |text: &str| sink.borrow_mut().push(text.to_string()))),
            ..Default::default()
        };
        assert!(
            mission_pack_command(&mut game, &players, &player, Q1MissionPack::Hipnotic, 202, &options)
                .expect("impulse")
        );
        assert!(logged.borrow().iter().any(|line| line.contains("player")));
        dump_mission_pack_coordinates(&mut game, &player);
    }
}
