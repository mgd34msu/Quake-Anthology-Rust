//! Mission-pack weapon selection (src/content/q1/missionpacks/selection.ts).

use qa_core::identity::ActorId;

use crate::contract::ItemId;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{weapon_item, Q1Edition, Q1PlayerState, Q1Weapon};
use crate::q1::{q1_error, Q1Error};

use super::messages::mission_message;
use super::types::Q1MissionPack;

/// Hipnotic cycle order (`hipnoticCycle`).
const HIPNOTIC_CYCLE: [Q1Weapon; 11] = [
    Q1Weapon::Axe,
    Q1Weapon::Shotgun,
    Q1Weapon::Supershotgun,
    Q1Weapon::Nailgun,
    Q1Weapon::Supernailgun,
    Q1Weapon::Grenadelauncher,
    Q1Weapon::HipnoticProximity,
    Q1Weapon::Rocketlauncher,
    Q1Weapon::Lightning,
    Q1Weapon::HipnoticLaser,
    Q1Weapon::HipnoticMjolnir,
];

/// Rogue cycle order (`rogueCycle`).
const ROGUE_CYCLE: [Q1Weapon; 14] = [
    Q1Weapon::Axe,
    Q1Weapon::RogueGrapple,
    Q1Weapon::Shotgun,
    Q1Weapon::Supershotgun,
    Q1Weapon::Nailgun,
    Q1Weapon::RogueLavaNailgun,
    Q1Weapon::Supernailgun,
    Q1Weapon::RogueLavaSupernailgun,
    Q1Weapon::Grenadelauncher,
    Q1Weapon::RogueMultiGrenade,
    Q1Weapon::Rocketlauncher,
    Q1Weapon::RogueMultiRocket,
    Q1Weapon::Lightning,
    Q1Weapon::RoguePlasma,
];

/// Ammunition required to select a weapon (`countRequired`).
fn count_required(weapon: Q1Weapon) -> f64 {
    if matches!(
        weapon,
        Q1Weapon::Supershotgun | Q1Weapon::Supernailgun | Q1Weapon::RogueLavaSupernailgun
    ) {
        2.0
    } else {
        1.0
    }
}

/// Whether a player owns a weapon (`owns`).
fn owns(game: &Q1EntityServices, player: &ActorId, weapon: Q1Weapon) -> bool {
    game.host.inventory.count(player, &weapon_item(weapon)) > 0.0
}

/// Whether a player has ammunition for a weapon (`hasAmmo`).
fn has_ammo(game: &Q1EntityServices, player: &ActorId, weapon: Q1Weapon, required: f64) -> bool {
    match game.weapon_ammo(weapon) {
        None => true,
        Some(ammo) => game.host.inventory.count(player, &ammo) >= required,
    }
}

/// Cycle weapons forward or backward (`cycle`).
fn cycle(
    game: &mut Q1EntityServices,
    state: &Q1PlayerState,
    pack: Q1MissionPack,
    reverse: bool,
) -> Result<bool, Q1Error> {
    let order: &[Q1Weapon] = if pack == Q1MissionPack::Hipnotic {
        &HIPNOTIC_CYCLE
    } else {
        &ROGUE_CYCLE
    };
    let player = state.actor.id();
    let current = order
        .iter()
        .position(|weapon| *weapon == state.weapon)
        .map(|index| index as i32)
        .unwrap_or(-1);
    let deathmatch = game.options().deathmatch;
    let teamplay = game.options().teamplay.unwrap_or(0);
    for step in 1..=order.len() as i32 {
        let delta = if reverse { -step } else { step };
        let next = order[((current + delta + order.len() as i32 * 2) % order.len() as i32) as usize];
        if next == Q1Weapon::RogueGrapple && !(deathmatch != 0 && teamplay >= 4) {
            continue;
        }
        let required = if reverse && next == Q1Weapon::RogueLavaNailgun {
            2.0
        } else {
            count_required(next)
        };
        if owns(game, player, next) && has_ammo(game, player, next, required) {
            return game.select_weapon(&state.actor, next);
        }
    }
    Ok(false)
}

/// Rogue impulse selection (`rogueSelected`).
fn rogue_selected(game: &mut Q1EntityServices, state: &Q1PlayerState, impulse: i32) -> Option<Q1Weapon> {
    let player = state.actor.id();
    let deathmatch = game.options().deathmatch;
    let teamplay = game.options().teamplay.unwrap_or(0);
    let edition = game.options().edition;
    let ctf = deathmatch != 0 && teamplay >= 4;
    let paired = |base: Q1Weapon, powered: Q1Weapon| {
        if owns(game, player, powered) && (state.weapon == base || !has_ammo(game, player, base, count_required(base)))
        {
            powered
        } else {
            base
        }
    };
    match impulse {
        1 => Some(if ctf && state.weapon == Q1Weapon::Axe {
            Q1Weapon::RogueGrapple
        } else {
            Q1Weapon::Axe
        }),
        2 => Some(Q1Weapon::Shotgun),
        3 => Some(Q1Weapon::Supershotgun),
        4 => Some(paired(Q1Weapon::Nailgun, Q1Weapon::RogueLavaNailgun)),
        5 => Some(paired(Q1Weapon::Supernailgun, Q1Weapon::RogueLavaSupernailgun)),
        6 => Some(paired(Q1Weapon::Grenadelauncher, Q1Weapon::RogueMultiGrenade)),
        7 => Some(paired(Q1Weapon::Rocketlauncher, Q1Weapon::RogueMultiRocket)),
        8 => Some(paired(Q1Weapon::Lightning, Q1Weapon::RoguePlasma)),
        22 => {
            if ctf {
                Some(Q1Weapon::RogueGrapple)
            } else {
                None
            }
        }
        60 => Some(Q1Weapon::RogueLavaNailgun),
        61 => Some(Q1Weapon::RogueLavaSupernailgun),
        62 => Some(Q1Weapon::RogueMultiGrenade),
        63 => Some(Q1Weapon::RogueMultiRocket),
        64 => Some(Q1Weapon::RoguePlasma),
        65 => {
            if edition == Q1Edition::Rerelease {
                Some(Q1Weapon::Nailgun)
            } else {
                None
            }
        }
        66 => {
            if edition == Q1Edition::Rerelease {
                Some(Q1Weapon::Supernailgun)
            } else {
                None
            }
        }
        67 => {
            if edition == Q1Edition::Rerelease {
                Some(Q1Weapon::Grenadelauncher)
            } else {
                None
            }
        }
        68 => {
            if edition == Q1Edition::Rerelease {
                Some(Q1Weapon::Rocketlauncher)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Hipnotic impulse selection (`hipnoticSelected`).
fn hipnotic_selected(game: &Q1EntityServices, state: &Q1PlayerState, impulse: i32) -> Option<Q1Weapon> {
    let edition = game.options().edition;
    match impulse {
        1 => Some(Q1Weapon::Axe),
        2 => Some(Q1Weapon::Shotgun),
        3 => Some(Q1Weapon::Supershotgun),
        4 => Some(Q1Weapon::Nailgun),
        5 => Some(Q1Weapon::Supernailgun),
        6 => Some(if state.weapon == Q1Weapon::Grenadelauncher {
            Q1Weapon::HipnoticProximity
        } else {
            Q1Weapon::Grenadelauncher
        }),
        7 => Some(Q1Weapon::Rocketlauncher),
        8 => Some(Q1Weapon::Lightning),
        225 => Some(Q1Weapon::HipnoticLaser),
        226 => Some(Q1Weapon::HipnoticMjolnir),
        227 => {
            if edition == Q1Edition::Rerelease {
                Some(Q1Weapon::HipnoticProximity)
            } else {
                None
            }
        }
        228 => {
            if edition == Q1Edition::Rerelease {
                Some(Q1Weapon::Grenadelauncher)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Handle a mission-pack weapon impulse (`missionWeaponImpulse`).
pub fn mission_weapon_impulse(
    game: &mut Q1EntityServices,
    player: &ActorId,
    pack: Q1MissionPack,
    impulse: i32,
) -> Result<bool, Q1Error> {
    let state = game
        .player_ref(player)
        .cloned()
        .ok_or_else(|| q1_error("Player has no Q1 weapon state"))?;
    if impulse == 10 || impulse == 12 {
        cycle(game, &state, pack, impulse == 12)?;
        return Ok(true);
    }
    let selected = if pack == Q1MissionPack::Hipnotic {
        hipnotic_selected(game, &state, impulse)
    } else {
        rogue_selected(game, &state, impulse)
    };
    if selected.is_none() {
        return Ok(false);
    }
    let mut selected = selected.expect("selected");
    if pack == Q1MissionPack::Hipnotic && selected == Q1Weapon::Grenadelauncher && !owns(game, player, selected) {
        selected = Q1Weapon::HipnoticProximity;
    }
    if !owns(game, player, selected) {
        mission_message(game, Some(player), "$qc_no_weapon");
        return Ok(true);
    }
    let required = if pack == Q1MissionPack::Rogue && impulse == 61 {
        1.0
    } else {
        count_required(selected)
    };
    let rogue_alias = pack == Q1MissionPack::Rogue
        && (65..=68).contains(&impulse)
        && state.weapon.as_str().starts_with("rogue:")
        && state.weapon != Q1Weapon::RogueGrapple;
    let alias_ammo = if rogue_alias {
        Some(game.host.inventory.count(
            player,
            &ItemId::from(if impulse <= 66 {
                "rogue:ammo/lava-nails"
            } else {
                "rogue:ammo/multi-rockets"
            }),
        ))
    } else {
        None
    };
    let denied = match alias_ammo {
        None => !has_ammo(game, player, selected, required),
        Some(ammo) => ammo < required,
    };
    if denied {
        mission_message(game, Some(player), "$qc_not_enough_ammo");
        return Ok(true);
    }
    if pack == Q1MissionPack::Rogue && selected != state.weapon {
        let old = state.weapon;
        let key = if old == Q1Weapon::RogueLavaNailgun || old == Q1Weapon::RogueLavaSupernailgun {
            if selected == Q1Weapon::Nailgun || selected == Q1Weapon::Supernailgun {
                "$qc_normal_nails"
            } else {
                ""
            }
        } else if old == Q1Weapon::RogueMultiGrenade {
            if selected == Q1Weapon::Grenadelauncher {
                "$qc_normal_grenades"
            } else {
                ""
            }
        } else if old == Q1Weapon::RogueMultiRocket {
            if selected == Q1Weapon::Rocketlauncher {
                "$qc_normal_rockets"
            } else {
                ""
            }
        } else if old == Q1Weapon::RoguePlasma {
            if selected == Q1Weapon::Lightning {
                "$qc_lightning_gun"
            } else {
                ""
            }
        } else if selected == Q1Weapon::RogueLavaNailgun || selected == Q1Weapon::RogueLavaSupernailgun {
            "$qc_lava_nails"
        } else if selected == Q1Weapon::RogueMultiGrenade {
            "$qc_multi_gl"
        } else if selected == Q1Weapon::RogueMultiRocket {
            "$qc_multi_rl"
        } else if selected == Q1Weapon::RoguePlasma {
            "$qc_plasma_gun"
        } else {
            ""
        };
        if !key.is_empty() {
            mission_message(game, Some(player), key);
        }
    }
    game.select_weapon(&state.actor, selected)?;
    Ok(true)
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
        game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
        player
    }

    #[test]
    fn unknown_impulses_reject() {
        let mut game = test_game();
        let player = attached_player(&mut game);
        assert!(!mission_weapon_impulse(&mut game, &player, Q1MissionPack::Hipnotic, 99).expect("impulse"));
        assert!(mission_weapon_impulse(&mut game, &player, Q1MissionPack::Rogue, 10).expect("cycle"));
    }

    #[test]
    fn owned_weapons_select() {
        let mut game = test_game();
        let player = attached_player(&mut game);
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.host.inventory.give(&owned, &weapon_item(Q1Weapon::Shotgun), 1.0);
        game.host.inventory.give(&owned, &ItemId::from("q1:ammo/shells"), 10.0);
        assert!(mission_weapon_impulse(&mut game, &player, Q1MissionPack::Hipnotic, 2).expect("impulse"));
        assert_eq!(game.player_ref(&player).expect("state").weapon, Q1Weapon::Shotgun);
    }

    #[test]
    fn rogue_pairs_prefer_powered() {
        let mut game = test_game();
        super::super::arsenal::register_mission_pack_arsenal(&mut game, Q1MissionPack::Rogue).expect("arsenal");
        let player = attached_player(&mut game);
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.host.inventory.give(&owned, &weapon_item(Q1Weapon::Nailgun), 1.0);
        game.host
            .inventory
            .give(&owned, &weapon_item(Q1Weapon::RogueLavaNailgun), 1.0);
        game.host
            .inventory
            .give(&owned, &ItemId::from("rogue:ammo/lava-nails"), 10.0);
        assert!(mission_weapon_impulse(&mut game, &player, Q1MissionPack::Rogue, 4).expect("impulse"));
        assert_eq!(
            game.player_ref(&player).expect("state").weapon,
            Q1Weapon::RogueLavaNailgun
        );
    }
}
