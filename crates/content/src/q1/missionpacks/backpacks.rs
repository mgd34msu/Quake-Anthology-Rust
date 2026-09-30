//! Mission-pack backpacks (src/content/q1/missionpacks/backpacks.ts).

use std::collections::HashMap;

use qa_core::identity::{same_actor, ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3};

use crate::contract::ItemId;
use crate::q1::base::projectiles::{drop_backpack, BackpackDrop, BackpackExtra, BackpackLaunch, BackpackSelection};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{
    vadd, vscale, weapon_item, Q1Effect, Q1MoveType, Q1Solid, Q1SoundChannel, Q1Weapon,
};
use crate::q1::foundation::weapons::aim;
use crate::q1::Q1Error;

use super::messages::{mission_message, mission_pickup_message};
use super::player::MissionPackPlayers;
use super::types::Q1MissionPack;

/// Map a Rogue combo weapon to its base weapon (`baseWeapon`).
fn base_weapon(weapon: Q1Weapon) -> Q1Weapon {
    match weapon {
        Q1Weapon::RogueLavaNailgun => Q1Weapon::Nailgun,
        Q1Weapon::RogueLavaSupernailgun => Q1Weapon::Supernailgun,
        Q1Weapon::RogueMultiGrenade => Q1Weapon::Grenadelauncher,
        Q1Weapon::RogueMultiRocket => Q1Weapon::Rocketlauncher,
        Q1Weapon::RoguePlasma => Q1Weapon::Lightning,
        other => other,
    }
}

/// Drop a mission-pack death backpack (`dropMissionPackBackpack`).
pub fn drop_mission_pack_backpack(
    game: &mut Q1EntityServices,
    actor: &OwnedActor,
    pack: Q1MissionPack,
) -> Result<Option<ActorId>, Q1Error> {
    let id = actor.id().clone();
    let player = match game.player_ref(&id) {
        Some(player) => player.clone(),
        None => return Ok(None),
    };
    let body = match game.host.bodies.read(&id) {
        Some(body) => body,
        None => return Ok(None),
    };
    let edition = game.options().edition;
    let shells = game.host.inventory.count(&id, &ItemId::from("q1:ammo/shells"));
    let nails = game.host.inventory.count(&id, &ItemId::from("q1:ammo/nails"));
    let rockets = game.host.inventory.count(&id, &ItemId::from("q1:ammo/rockets"));
    let cells = game.host.inventory.count(&id, &ItemId::from("q1:ammo/cells"));
    if pack == Q1MissionPack::Hipnotic {
        if shells + nails + rockets + cells == 0.0 {
            return Ok(None);
        }
        let cells = if edition == crate::q1::foundation::types::Q1Edition::Rerelease
            && matches!(player.weapon, Q1Weapon::HipnoticLaser | Q1Weapon::HipnoticMjolnir)
        {
            cells.max(15.0)
        } else {
            cells
        };
        return drop_backpack(
            game,
            body.origin,
            &BackpackDrop {
                weapon: Some(player.weapon),
                shells,
                nails,
                rockets,
                cells,
                extra: Vec::new(),
                selection: Some(BackpackSelection::Rank),
                avoid_underwater_lightning: Some(true),
                owner_pickup_delay: None,
            },
            None,
        );
    }
    let lava = game.host.inventory.count(&id, &ItemId::from("rogue:ammo/lava-nails"));
    let multi = game
        .host
        .inventory
        .count(&id, &ItemId::from("rogue:ammo/multi-rockets"));
    let plasma = game.host.inventory.count(&id, &ItemId::from("rogue:ammo/plasma"));
    // Both source editions omit plasma from this early empty-backpack check.
    if shells + nails + rockets + cells + lava + multi == 0.0 {
        return Ok(None);
    }
    drop_backpack(
        game,
        body.origin,
        &BackpackDrop {
            weapon: Some(base_weapon(player.weapon)),
            shells,
            nails,
            rockets,
            cells,
            extra: vec![
                BackpackExtra {
                    item: ItemId::from("rogue:ammo/lava-nails"),
                    count: lava,
                },
                BackpackExtra {
                    item: ItemId::from("rogue:ammo/multi-rockets"),
                    count: multi,
                },
                BackpackExtra {
                    item: ItemId::from("rogue:ammo/plasma"),
                    count: plasma,
                },
            ],
            selection: None,
            avoid_underwater_lightning: None,
            owner_pickup_delay: Some(1.0),
        },
        None,
    )
}

/// Rogue tossable ammunition rule (`TossAmmo`).
struct TossAmmo {
    item: &'static str,
    amount: f64,
    selected: &'static [Q1Weapon],
    owners: &'static [Q1Weapon],
}

const TOSS_AMMO: [TossAmmo; 7] = [
    TossAmmo {
        item: "q1:ammo/shells",
        amount: 20.0,
        selected: &[Q1Weapon::Shotgun, Q1Weapon::Supershotgun],
        owners: &[Q1Weapon::Shotgun, Q1Weapon::Supershotgun],
    },
    TossAmmo {
        item: "q1:ammo/nails",
        amount: 20.0,
        selected: &[Q1Weapon::Nailgun, Q1Weapon::Supernailgun],
        owners: &[Q1Weapon::Nailgun, Q1Weapon::Supernailgun],
    },
    TossAmmo {
        item: "rogue:ammo/lava-nails",
        amount: 20.0,
        selected: &[Q1Weapon::RogueLavaNailgun, Q1Weapon::RogueLavaSupernailgun],
        owners: &[Q1Weapon::Nailgun, Q1Weapon::Supernailgun],
    },
    TossAmmo {
        item: "q1:ammo/rockets",
        amount: 10.0,
        selected: &[Q1Weapon::Grenadelauncher, Q1Weapon::Rocketlauncher],
        owners: &[Q1Weapon::Grenadelauncher, Q1Weapon::Rocketlauncher],
    },
    TossAmmo {
        item: "rogue:ammo/multi-rockets",
        amount: 10.0,
        selected: &[Q1Weapon::RogueMultiGrenade, Q1Weapon::RogueMultiRocket],
        owners: &[Q1Weapon::Grenadelauncher, Q1Weapon::Rocketlauncher],
    },
    TossAmmo {
        item: "q1:ammo/cells",
        amount: 20.0,
        selected: &[Q1Weapon::Lightning],
        owners: &[Q1Weapon::Lightning],
    },
    TossAmmo {
        item: "rogue:ammo/plasma",
        amount: 10.0,
        selected: &[Q1Weapon::RoguePlasma],
        owners: &[Q1Weapon::Lightning],
    },
];

/// Toss a Rogue ammunition backpack (`tossRogueBackpack`).
pub fn toss_rogue_backpack(game: &mut Q1EntityServices, player: &ActorId) -> Result<Option<ActorId>, Q1Error> {
    let state = match game.player_ref(player) {
        Some(state) => state.clone(),
        None => return Ok(None),
    };
    let teamplay = game.options().teamplay.unwrap_or(0);
    let ammo = game.weapon_ammo(state.weapon);
    let body = game.host.bodies.read(player);
    let Some(ammo) = ammo else {
        return Ok(None);
    };
    let Some(body) = body else {
        return Ok(None);
    };
    if teamplay < 1 || game.host.inventory.count(player, &ammo) <= 0.0 {
        return Ok(None);
    }
    let mut amounts: HashMap<ItemId, f64> = HashMap::new();
    for rule in TOSS_AMMO.iter() {
        let mut take = false;
        if rule.selected.contains(&state.weapon) {
            take = true;
        }
        if !take
            && rule
                .owners
                .iter()
                .all(|weapon| game.host.inventory.count(player, &weapon_item(*weapon)) == 0.0)
        {
            take = true;
        }
        if take {
            let item = ItemId::from(rule.item);
            let count = rule.amount.min(game.host.inventory.count(player, &item));
            game.host.inventory.consume(&state.actor, &item, count);
            amounts.insert(item, count);
        }
    }
    if amounts.values().sum::<f64>() == 0.0 {
        mission_message(game, Some(player), "$qc_no_ammo_available");
        return Ok(None);
    }
    let amount = |item: &str| amounts.get(item).copied().unwrap_or(0.0);
    let forward = game.make_vectors(state.view_angles).forward;
    let velocity = vscale(aim(game, &state.actor, forward), 500.0);
    let backpack = drop_backpack(
        game,
        body.origin,
        &BackpackDrop {
            weapon: None,
            shells: amount("q1:ammo/shells"),
            nails: amount("q1:ammo/nails"),
            rockets: amount("q1:ammo/rockets"),
            cells: amount("q1:ammo/cells"),
            extra: vec![
                BackpackExtra {
                    item: ItemId::from("rogue:ammo/lava-nails"),
                    count: amount("rogue:ammo/lava-nails"),
                },
                BackpackExtra {
                    item: ItemId::from("rogue:ammo/multi-rockets"),
                    count: amount("rogue:ammo/multi-rockets"),
                },
                BackpackExtra {
                    item: ItemId::from("rogue:ammo/plasma"),
                    count: amount("rogue:ammo/plasma"),
                },
            ],
            selection: None,
            avoid_underwater_lightning: None,
            owner_pickup_delay: Some(1.0),
        },
        Some(&BackpackLaunch {
            origin: vadd(
                body.origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 16.0,
                },
            ),
            velocity,
            movement: Q1MoveType::Bounce,
        }),
    )?;
    if let Some(backpack) = backpack.as_ref() {
        let owner = player.clone();
        game.update_entity(backpack, |entity| entity.owner = Some(owner))?;
    }
    Ok(backpack)
}

/// Rogue tossable weapon (`TossWeapon`).
struct TossWeapon {
    weapon: Q1Weapon,
    powered: Option<Q1Weapon>,
    classname: &'static str,
    model: &'static str,
    name: &'static str,
}

const TOSS_WEAPONS: [TossWeapon; 6] = [
    TossWeapon {
        weapon: Q1Weapon::Supershotgun,
        powered: None,
        classname: "weapon_supershotgun",
        model: "progs/g_shot.mdl",
        name: "$qc_double_shotgun",
    },
    TossWeapon {
        weapon: Q1Weapon::Nailgun,
        powered: Some(Q1Weapon::RogueLavaNailgun),
        classname: "weapon_nailgun",
        model: "progs/g_nail.mdl",
        name: "$qc_nailgun",
    },
    TossWeapon {
        weapon: Q1Weapon::Supernailgun,
        powered: Some(Q1Weapon::RogueLavaSupernailgun),
        classname: "weapon_supernailgun",
        model: "progs/g_nail2.mdl",
        name: "$qc_super_nailgun",
    },
    TossWeapon {
        weapon: Q1Weapon::Grenadelauncher,
        powered: Some(Q1Weapon::RogueMultiGrenade),
        classname: "weapon_grenadelauncher",
        model: "progs/g_rock.mdl",
        name: "$qc_grenade_launcher",
    },
    TossWeapon {
        weapon: Q1Weapon::Rocketlauncher,
        powered: Some(Q1Weapon::RogueMultiRocket),
        classname: "weapon_rocketlauncher",
        model: "progs/g_rock2.mdl",
        name: "$qc_rocket_launcher",
    },
    TossWeapon {
        weapon: Q1Weapon::Lightning,
        powered: Some(Q1Weapon::RoguePlasma),
        classname: "weapon_lightning",
        model: "progs/g_light.mdl",
        name: "$qc_thunderbolt",
    },
];

/// Toss the selected Rogue weapon (`tossRogueWeapon`).
pub fn toss_rogue_weapon(game: &mut Q1EntityServices, player: &ActorId) -> Result<Option<ActorId>, Q1Error> {
    let state = match game.player_ref(player) {
        Some(state) => state.clone(),
        None => return Ok(None),
    };
    let deathmatch = game.options().deathmatch;
    let teamplay = game.options().teamplay.unwrap_or(0);
    let definition = TOSS_WEAPONS
        .iter()
        .find(|candidate| candidate.weapon == state.weapon || candidate.powered == Some(state.weapon));
    let body = game.host.bodies.read(player);
    let (Some(definition), Some(body)) = (definition, body) else {
        return Ok(None);
    };
    if deathmatch != 1 || teamplay < 1 {
        return Ok(None);
    }
    let item = game.create(definition.classname, None, None)?;
    let owner = player.clone();
    let model = definition.model.to_string();
    game.update_entity(&item, |entity| {
        entity.owner = Some(owner);
        entity.model = model;
        entity.movement = Q1MoveType::Bounce;
        entity.solid = Q1Solid::Trigger;
    })?;
    let forward = game.make_vectors(state.view_angles).forward;
    let velocity = vscale(aim(game, &state.actor, forward), 500.0);
    game.set_body(
        &item,
        &BodyPatch {
            origin: Some(vadd(
                body.origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 16.0,
                },
            )),
            velocity: Some(velocity),
            bounds: Some(Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: 0.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 56.0,
                },
            }),
            ..Default::default()
        },
    )?;
    game.host
        .inventory
        .consume(&state.actor, &weapon_item(definition.weapon), 1.0);
    if let Some(powered) = definition.powered {
        game.host.inventory.consume(&state.actor, &weapon_item(powered), 1.0);
    }
    let touch = game.named.touch("rogue:tossed-weapon-touch")?;
    game.update_entity(&item, |entity| entity.touch = Some(touch))?;
    game.schedule(&item, 120.0, "SUB_Remove")?;
    game.link(&item)?;
    let best = game.choose_best(&state.actor, None)?;
    game.select_weapon(&state.actor, best)?;
    Ok(Some(item))
}

/// Tossed-weapon touch (`rogue:tossed-weapon-touch`).
fn tossed_weapon_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let state = match game.player_ref(other) {
        Some(state) => state.clone(),
        None => return Ok(()),
    };
    let entity = match game.entity_ref(id) {
        Some(entity) => entity.clone(),
        None => return Ok(()),
    };
    let definition = TOSS_WEAPONS
        .iter()
        .find(|candidate| candidate.classname == entity.classname);
    let Some(definition) = definition else {
        return Ok(());
    };
    if entity.owner.as_ref().is_some_and(|owner| same_actor(owner, other)) && entity.next_think - game.time > 119.0 {
        return Ok(());
    }
    mission_pickup_message(game, other, definition.name);
    game.sound(other, "weapons/pkup.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
    game.effect(Q1Effect::Pickup, game.body(id)?.origin, Some(other), 1);
    game.host
        .inventory
        .give(&state.actor, &weapon_item(definition.weapon), 1.0);
    game.remove(id)?;
    let rank = |weapon| {
        game.pickup_rules
            .as_ref()
            .and_then(|rules| rules.weapon_rank)
            .map(|rank| rank(weapon))
            .unwrap_or(12)
    };
    let deathmatch = game.options().deathmatch;
    if deathmatch == 0 || rank(definition.weapon) < rank(state.weapon) {
        game.select_weapon(&state.actor, definition.weapon)?;
    }
    MissionPackPlayers::for_pack(Q1MissionPack::Rogue).enable_combos(game, other)
}

/// Register Rogue toss callbacks (`registerRogueTossCallbacks`).
/// The donor takes the player services; this port calls the Rogue combo
/// view directly since touch hooks are static function pointers.
pub fn register_rogue_toss_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "rogue:tossed-weapon-touch",
        Q1CallbackHandlers {
            touch: Some(tossed_weapon_touch),
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::super::types::test_game;
    use super::*;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::foundation::entity_services::Q1AttachOptions;

    fn attached_player(game: &mut Q1EntityServices) -> (ActorId, OwnedActor) {
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
        (player, owned)
    }

    #[test]
    fn empty_packs_drop_nothing() {
        let mut game = test_game();
        let (_, owned) = attached_player(&mut game);
        assert!(drop_mission_pack_backpack(&mut game, &owned, Q1MissionPack::Hipnotic)
            .expect("drop")
            .is_none());
        assert!(drop_mission_pack_backpack(&mut game, &owned, Q1MissionPack::Rogue)
            .expect("drop")
            .is_none());
    }

    #[test]
    fn loaded_hipnotic_pack_drops() {
        let mut game = test_game();
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let (_, owned) = attached_player(&mut game);
        game.host.inventory.give(&owned, &ItemId::from("q1:ammo/shells"), 25.0);
        let pack = drop_mission_pack_backpack(&mut game, &owned, Q1MissionPack::Hipnotic).expect("drop");
        assert!(pack.is_some());
    }

    #[test]
    fn toss_gates_on_teamplay_and_callbacks_register() {
        let mut game = test_game();
        register_rogue_toss_callbacks(&mut game).expect("register");
        assert!(game.named.touch("rogue:tossed-weapon-touch").is_ok());
        let (player, _) = attached_player(&mut game);
        assert!(toss_rogue_backpack(&mut game, &player).expect("toss").is_none());
        assert!(toss_rogue_weapon(&mut game, &player).expect("toss").is_none());
    }
}
