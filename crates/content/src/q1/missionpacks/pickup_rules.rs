//! Mission-pack pickup rules (src/content/q1/missionpacks/pickup-rules.ts).

use qa_core::identity::ActorId;

use crate::contract::ItemId;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::extensions::Q1PickupRules;
use crate::q1::foundation::types::{Q1AutoSwitch, Q1Edition, Q1Weapon};
use crate::q1::{Q1Error, q1_error};

use super::items::hipnotic_weapon_rank;
use super::player::MissionPackPlayers;
use super::types::Q1MissionPack;

/// Rogue weapon rank (`rogueRank`).
fn rogue_rank(weapon: Q1Weapon) -> i32 {
    const ORDER: [Q1Weapon; 11] = [
        Q1Weapon::RoguePlasma,
        Q1Weapon::Lightning,
        Q1Weapon::RogueMultiRocket,
        Q1Weapon::Rocketlauncher,
        Q1Weapon::RogueLavaSupernailgun,
        Q1Weapon::Supernailgun,
        Q1Weapon::RogueMultiGrenade,
        Q1Weapon::Grenadelauncher,
        Q1Weapon::RogueLavaNailgun,
        Q1Weapon::Supershotgun,
        Q1Weapon::Nailgun,
    ];
    ORDER
        .iter()
        .position(|candidate| *candidate == weapon)
        .map(|index| index as i32 + 1)
        .unwrap_or(12)
}

/// Whether picked-up weapons stay in the world (`weaponLeave`).
fn mission_weapon_leave(game: &mut Q1EntityServices) -> Result<bool, Q1Error> {
    let edition = game.options().edition;
    let deathmatch = game.options().deathmatch;
    let coop = game.options().coop;
    Ok(coop
        || deathmatch == 2
        || edition == Q1Edition::Rerelease && (deathmatch == 3 || deathmatch == 5))
}

/// Hipnotic automatic switch decision (`autoSwitch`).
fn hipnotic_auto_switch(
    game: &mut Q1EntityServices,
    player: &ActorId,
    was_owned: bool,
) -> Result<bool, Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    let edition = game.options().edition;
    Ok(edition == Q1Edition::Classic
        || state.auto_switch == Q1AutoSwitch::Always
        || state.auto_switch == Q1AutoSwitch::New && !was_owned)
}

/// Rogue automatic switch decision (`autoSwitch`).
fn rogue_auto_switch(
    game: &mut Q1EntityServices,
    player: &ActorId,
    was_owned: bool,
) -> Result<bool, Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    if state.weapon == Q1Weapon::RogueGrapple && state.attack_held {
        return Ok(false);
    }
    let edition = game.options().edition;
    Ok(edition == Q1Edition::Classic
        || state.auto_switch == Q1AutoSwitch::Always
        || state.auto_switch == Q1AutoSwitch::New && !was_owned)
}

/// Hipnotic weapon grant (`weaponGranted`).
fn hipnotic_weapon_granted(
    _game: &mut Q1EntityServices,
    _player: &ActorId,
    weapon: Q1Weapon,
) -> Result<Q1Weapon, Q1Error> {
    Ok(weapon)
}

/// Rogue weapon grant with combo upgrades (`weaponGranted`).
fn rogue_weapon_granted(
    game: &mut Q1EntityServices,
    player: &ActorId,
    weapon: Q1Weapon,
) -> Result<Q1Weapon, Q1Error> {
    MissionPackPlayers::for_pack(Q1MissionPack::Rogue).enable_combos(game, player)?;
    let count = |item: &str| game.host.inventory.count(player, &ItemId::from(item));
    Ok(match weapon {
        Q1Weapon::Lightning if count("rogue:ammo/plasma") > 0.0 => Q1Weapon::RoguePlasma,
        Q1Weapon::Rocketlauncher if count("rogue:ammo/multi-rockets") > 0.0 => {
            Q1Weapon::RogueMultiRocket
        }
        Q1Weapon::Grenadelauncher if count("rogue:ammo/multi-rockets") > 0.0 => {
            Q1Weapon::RogueMultiGrenade
        }
        Q1Weapon::Supernailgun if count("rogue:ammo/lava-nails") > 1.0 => {
            Q1Weapon::RogueLavaSupernailgun
        }
        Q1Weapon::Nailgun if count("rogue:ammo/lava-nails") > 0.0 => Q1Weapon::RogueLavaNailgun,
        other => other,
    })
}

/// Mission-pack respawn interval (`respawn`).
fn mission_respawn(
    game: &mut Q1EntityServices,
    entity: &ActorId,
    default_seconds: f64,
) -> Result<f64, Q1Error> {
    let edition = game.options().edition;
    let deathmatch = game.options().deathmatch;
    let classname = game
        .entity_ref(entity)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    if edition == Q1Edition::Classic
        && deathmatch != 1
        && (classname.starts_with("item_armor")
            || classname.starts_with("weapon_")
            || matches!(
                classname.as_str(),
                "item_shells" | "item_spikes" | "item_rockets" | "item_cells" | "item_weapon"
            ))
    {
        return Ok(-1.0);
    }
    Ok(default_seconds)
}

/// Register mission-pack pickup rules (`registerMissionPackPickupRules`).
/// The donor takes the player services; this port calls the Rogue combo
/// view directly since rules hooks are static function pointers.
pub fn register_mission_pack_pickup_rules(
    game: &mut Q1EntityServices,
    pack: Q1MissionPack,
) -> Result<(), Q1Error> {
    game.register_pickup_rules(Q1PickupRules {
        id: format!("q1:{}:pickups", pack.as_str()),
        weapon_leave: Some(mission_weapon_leave),
        weapon_granted: Some(if pack == Q1MissionPack::Hipnotic {
            hipnotic_weapon_granted
        } else {
            rogue_weapon_granted
        }),
        weapon_ammo_grant: None,
        weapon_rank: Some(if pack == Q1MissionPack::Hipnotic {
            hipnotic_weapon_rank
        } else {
            rogue_rank
        }),
        auto_switch: Some(if pack == Q1MissionPack::Hipnotic {
            hipnotic_auto_switch
        } else {
            rogue_auto_switch
        }),
        respawn: Some(mission_respawn),
    })
}

#[cfg(test)]
mod tests {
    use super::super::types::test_game;
    use super::*;
    use crate::q1::foundation::entity_services::Q1AttachOptions;

    fn attached_player(game: &mut Q1EntityServices) -> ActorId {
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default())
            .expect("attach");
        player
    }

    #[test]
    fn ranks_match_donor_tables() {
        assert_eq!(rogue_rank(Q1Weapon::RoguePlasma), 1);
        assert_eq!(rogue_rank(Q1Weapon::Nailgun), 11);
        assert_eq!(rogue_rank(Q1Weapon::Axe), 12);
        let mut game = test_game();
        register_mission_pack_pickup_rules(&mut game, Q1MissionPack::Hipnotic).expect("register");
        let rank = game
            .pickup_rules
            .as_ref()
            .and_then(|rules| rules.weapon_rank)
            .expect("rank");
        assert_eq!(rank(Q1Weapon::HipnoticLaser), 3);
    }

    #[test]
    fn rogue_grant_upgrades_with_combo_ammo() {
        let mut game = test_game();
        register_mission_pack_pickup_rules(&mut game, Q1MissionPack::Rogue).expect("register");
        let player = attached_player(&mut game);
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.host
            .inventory
            .give(&owned, &ItemId::from("rogue:ammo/plasma"), 10.0);
        let granted = game
            .pickup_rules
            .as_ref()
            .and_then(|rules| rules.weapon_granted)
            .expect("granted");
        assert_eq!(
            granted(&mut game, &player, Q1Weapon::Lightning).expect("grant"),
            Q1Weapon::RoguePlasma
        );
        assert_eq!(
            granted(&mut game, &player, Q1Weapon::Shotgun).expect("grant"),
            Q1Weapon::Shotgun
        );
    }

    #[test]
    fn grapple_held_blocks_auto_switch() {
        let mut game = test_game();
        register_mission_pack_pickup_rules(&mut game, Q1MissionPack::Rogue).expect("register");
        let player = attached_player(&mut game);
        game.update_player(&player, |state| {
            state.weapon = Q1Weapon::RogueGrapple;
            state.attack_held = true;
        })
        .expect("held");
        let auto = game
            .pickup_rules
            .as_ref()
            .and_then(|rules| rules.auto_switch)
            .expect("auto");
        assert!(!auto(&mut game, &player, false).expect("switch"));
    }
}
