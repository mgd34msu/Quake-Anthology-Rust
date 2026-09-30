//! Q1 mg3 pickups (`src/content/q1/addons/items/pickups.ts`).
//!
//! `quakec_mg3/mg3_items.qc` and `items.qc`. GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::contract::{
    PickupAmmoGrant, PickupMapKind, PickupResource, PickupSelection, PickupWeaponGrant, ProtectionChannel,
    RegularArmorState,
};
use crate::q1::addons::campaign::{BLOODY_NIGHTMARE_ACTIVE, BLOODY_NIGHTMARE_DISCOVERED, BLOODY_NIGHTMARE_NEWGAME};
use crate::q1::addons::context::{
    addon_broadcast, addon_player_number, addon_set_cvar, set_addon_number, set_addon_player_number,
};
use crate::q1::addons::items::common::{finish_mg3_pickup, start_mg3_item, MG3_ITEM_PREFIX};
use crate::q1::base::provider::{campaign_set_skill, update_base};
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::pickups::{spawn_pickup, touch_q1_pickup, Q1TouchTake};
use crate::q1::foundation::types::{vadd, vscale, Q1AutoSwitch, Q1Effect, Q1Event, Q1Solid, Q1SoundChannel, Q1Weapon};
use crate::q1::Q1Error;

/// Bloody shotgun flag (`MG3_BLOODY_SHOTGUN`).
pub const MG3_BLOODY_SHOTGUN: i32 = 1;
/// Bloody super shotgun flag (`MG3_BLOODY_SUPER_SHOTGUN`).
pub const MG3_BLOODY_SUPER_SHOTGUN: i32 = 2;
/// Hugged-tette flag (`TETTE`).
const TETTE: i32 = 512;

/// MG3 weapon rank (`mg3WeaponRank`).
#[must_use]
pub fn mg3_weapon_rank(weapon: Q1Weapon) -> i32 {
    match weapon {
        Q1Weapon::Lightning => 1,
        Q1Weapon::Rocketlauncher => 2,
        Q1Weapon::Mg3Laser => 3,
        Q1Weapon::Supernailgun => 4,
        Q1Weapon::Grenadelauncher => 5,
        Q1Weapon::Supershotgun => 6,
        Q1Weapon::Nailgun => 7,
        _ => 8,
    }
}

/// Grants an mg3 weapon (`weaponTouch`).
fn weapon_touch(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId) -> Result<(), Q1Error> {
    let state = match game.player_ref(other).cloned() {
        Some(state) => state,
        None => return Ok(()),
    };
    let weapon = if game
        .entity_ref(id)
        .is_some_and(|entity| entity.classname == "weapon_mjolnir")
    {
        Q1Weapon::Mg3Mjolnir
    } else {
        Q1Weapon::Mg3Laser
    };
    let (coop, deathmatch) = (game.options().coop, game.options().deathmatch);
    let leave = coop || [2, 3, 5].contains(&deathmatch);
    let item = game.weapon_item(weapon);
    let admitted = game
        .pickup_admission
        .as_ref()
        .is_some_and(|admission| admission.maps(PickupMapKind::Weapons, &item));
    let owned = if admitted {
        game.pickup_admission
            .as_ref()
            .is_some_and(|admission| admission.owns(other, &item))
    } else {
        game.host.inventory.count(other, &item) != 0.0
    };
    if leave && owned {
        return Ok(());
    }
    if admitted {
        let selection = if state.auto_switch == Q1AutoSwitch::Always || state.auto_switch == Q1AutoSwitch::New && !owned
        {
            if deathmatch == 0 {
                PickupSelection::Always
            } else {
                PickupSelection::Better
            }
        } else {
            PickupSelection::Never
        };
        let taken = game.pickup_admission.as_ref().is_some_and(|admission| {
            admission.weapon(
                &state.actor,
                &PickupWeaponGrant {
                    item: item.clone(),
                    ammo: vec![PickupAmmoGrant {
                        item: String::from("q1:ammo/cells"),
                        amount: 30.0,
                    }],
                },
                selection,
            )
        });
        if !taken {
            return Ok(());
        }
    } else {
        game.host
            .inventory
            .give(&state.actor, &String::from("q1:ammo/cells"), 30.0);
        game.host.inventory.configure(
            &state.actor,
            &crate::contract::InventoryEntry {
                item: game.weapon_item(weapon),
                count: 1.0,
                capacity: 1.0,
                count_policy: None,
            },
        )?;
        if (state.auto_switch == Q1AutoSwitch::Always || state.auto_switch == Q1AutoSwitch::New && !owned)
            && (deathmatch == 0 || mg3_weapon_rank(weapon) < mg3_weapon_rank(state.weapon))
        {
            game.select_weapon(&state.actor, weapon)?;
        }
        game.select_weapon(&state.actor, state.weapon)?;
    }
    let netname = game
        .entity_ref(id)
        .map(|entity| entity.text("netname"))
        .unwrap_or_default();
    game.message(
        Some(other),
        "$qc_got_item",
        true,
        vec![crate::q1::foundation::types::Q1MessageArg::Text(netname)],
    );
    game.sound(other, "weapons/pkup.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
    let origin = game.body(id)?.origin;
    game.effect(Q1Effect::Pickup, origin, Some(other), 1);
    game.update_entity(id, |entity| entity.activator = Some(other.clone()))?;
    game.use_targets(id, Some(other))?;
    if !game.is_live(id) {
        return Ok(());
    }
    if leave {
        game.update_entity(id, |entity| entity.target = String::new())?;
        return Ok(());
    }
    game.update_entity(id, |entity| {
        entity.model = String::new();
        entity.solid = Q1Solid::None;
    })?;
    game.link(id)?;
    let wait = game.entity_ref(id).map(|entity| entity.wait).unwrap_or(0.0);
    let respawn = if deathmatch != 0 && deathmatch != 2 { 30.0 } else { wait };
    if respawn > 0.0 {
        game.schedule(id, respawn, "SUB_regen")
    } else {
        game.cancel(id);
        Ok(())
    }
}

fn mg3_weapon_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    weapon_touch(game, id, other)
}

fn ring_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    normal: Option<Vec3>,
    surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) {
        return Ok(());
    }
    let teleport = game.named.touch_handler("mg3:trigger:silent_teleport")?;
    teleport(game, id, other, normal, surface)?;
    if let Some(body) = game.host.bodies.read(other) {
        game.effect(Q1Effect::Teleport, body.origin, None, 1);
    }
    Ok(())
}

fn spawn_ring(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let classname = game
        .entity_ref(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    let (model, label, height) = if classname == "item_draught_insight" {
        ("gold_ring", "$mg3_qc_ring_of_insight", -2048.0)
    } else {
        ("onyx_ring", "$mg3_qc_ring_of_oblivion", 2048.0)
    };
    game.update_entity(id, |entity| {
        entity.model = format!("progs/{model}.mdl");
        entity.fields.insert(String::from("netname"), String::from(label));
    })?;
    game.set_bounds(
        id,
        Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 32.0,
            },
        },
    )?;
    if game.entity_ref(id).map(|entity| entity.number("height")).unwrap_or(0.0) == 0.0 {
        set_addon_number(game, id, "height", height)?;
    }
    let teleport = game
        .entity_ref(id)
        .is_some_and(|entity| entity.spawnflags & 1 != 0 && !entity.target.is_empty());
    let touch = if teleport {
        game.named.touch("teleport_touch")?
    } else {
        game.named.touch(&format!("{MG3_ITEM_PREFIX}ring_touch"))?
    };
    game.update_entity(id, |entity| entity.touch = Some(touch))?;
    let fresh = game.entity_ref(id).is_some_and(|entity| {
        entity.spawnflags & crate::q1::addons::items::common::MG3_SPAWNED_ITEM == 0 && entity.count == 0.0
    });
    if fresh {
        game.update_entity(id, |entity| entity.count = 1.0)?;
        let origin = vadd(game.body(id)?.origin, Vec3 { x: 0.0, y: 0.0, z: 8.0 });
        game.host.emit(Q1Event::Ambient {
            origin,
            path: String::from("ambience/hum1.wav"),
            volume: 0.7,
            attenuation: 3.0,
        });
        let particles = game.create("particle_tele", None, None)?;
        set_addon_number(game, &particles, "distance", 48.0)?;
        game.set_origin(&particles, origin)?;
        game.spawn_entity(&particles, None)?;
    }
    start_mg3_item(game, id)
}

struct ShardTake {
    id: ActorId,
    other: ActorId,
}

impl Q1TouchTake for ShardTake {
    fn original(&mut self, game: &mut Q1EntityServices) -> Result<bool, Q1Error> {
        let armor = game
            .host
            .combat
            .read(&self.other)
            .map(|combat| combat.armor.regular.clone());
        if matches!(armor, Some(RegularArmorState::Source { .. })) {
            return Ok(false);
        }
        let owned = game
            .player_owned(&self.other)
            .ok_or_else(|| crate::q1::q1_error("Missing Q1 shard player"))?;
        if let Some(RegularArmorState::Q1 { absorption, .. }) = &armor {
            if *absorption < 0.3 {
                let mut raised = armor.clone().expect("armor");
                if let RegularArmorState::Q1 { absorption, .. } = &mut raised {
                    *absorption = 0.3;
                }
                game.host.combat.set_regular_armor(&owned, &raised)?;
            }
        }
        let points = match &armor {
            None | Some(RegularArmorState::None) => 0.0,
            Some(RegularArmorState::Q1 { points, .. }) => *points,
            Some(_) => 0.0,
        };
        if points >= 200.0 {
            return Ok(false);
        }
        let (absorption, item) = match &armor {
            Some(RegularArmorState::Q1 { absorption, item, .. }) => (absorption.max(0.3), item.clone()),
            _ => (0.3, String::from("q1:item_armor1")),
        };
        game.host.combat.set_regular_armor(
            &owned,
            &RegularArmorState::Q1 {
                points: (points + 5.0).min(200.0),
                absorption,
                item,
            },
        )?;
        Ok(true)
    }

    fn complete(&mut self, game: &mut Q1EntityServices, taken: bool) -> Result<(), Q1Error> {
        if taken {
            finish_mg3_pickup(
                game,
                &self.id,
                &self.other,
                "$mg3_qc_armor_shard_touch",
                "items/armor1.wav",
                Vec::new(),
            )?;
        }
        Ok(())
    }
}

fn shard_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    if game.player_ref(other).is_none() || game.health(other) <= 0.0 {
        return Ok(());
    }
    touch_q1_pickup(
        game,
        id,
        other,
        String::from("q1:item_armor_shard"),
        Some(PickupResource::Protection {
            channel: ProtectionChannel::Regular,
        }),
        ShardTake {
            id: id.clone(),
            other: other.clone(),
        },
        None,
    )
}

fn spawn_shard(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.model = String::from("progs/armorshard.mdl");
    })?;
    let touch = game.named.touch(&format!("{MG3_ITEM_PREFIX}shard_touch"))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))?;
    game.set_bounds(
        id,
        Bounds {
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
        },
    )?;
    start_mg3_item(game, id)
}

fn mjolnir_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    normal: Option<Vec3>,
    surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    weapon_touch(game, id, other)?;
    let teleport = game.named.touch_handler("mg3:trigger:silent_teleport")?;
    teleport(game, id, other, normal, surface)
}

fn spawn_mg3_weapon(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let mjolnir = game
        .entity_ref(id)
        .is_some_and(|entity| entity.classname == "weapon_mjolnir");
    let (model, label) = if mjolnir {
        ("g_hammer", "$mg3_qc_hammer")
    } else {
        ("g_laserg", "$qc_laser_cannon")
    };
    game.update_entity(id, |entity| {
        entity.model = format!("progs/{model}.mdl");
        entity.fields.insert(String::from("netname"), String::from(label));
    })?;
    let teleport = mjolnir && game.entity_ref(id).is_some_and(|entity| entity.spawnflags & 128 != 0);
    if teleport && game.entity_ref(id).map(|entity| entity.number("height")).unwrap_or(0.0) == 0.0 {
        set_addon_number(game, id, "height", -2048.0)?;
    }
    let touch = game.named.touch(&format!(
        "{MG3_ITEM_PREFIX}{}",
        if teleport { "mjolnir_touch" } else { "weapon_touch" }
    ))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))?;
    game.set_bounds(
        id,
        Bounds {
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
        },
    )?;
    start_mg3_item(game, id)
}

fn head_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) {
        return Ok(());
    }
    game.host.emit(Q1Event::Achievement {
        player: None,
        id: String::from("ACH_FIND_MG3_SECRET"),
    });
    addon_broadcast(game, "$mg3_selected_bloody_nightmare");
    update_base(game, |state| {
        let flags = state.campaign.read_flags();
        state
            .campaign
            .write_flags(flags | BLOODY_NIGHTMARE_ACTIVE | BLOODY_NIGHTMARE_DISCOVERED);
    })?;
    campaign_set_skill(game, 3)?;
    addon_set_cvar(game, "skill", "3")?;
    finish_mg3_pickup(game, id, other, "", "player/tornoff2.wav", Vec::new())
}

fn hug_tette(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    update_base(game, |state| {
        let flags = state.campaign.read_flags();
        state.campaign.write_flags(flags | TETTE);
    })?;
    let netname = game
        .entity_ref(id)
        .map(|entity| entity.text("netname"))
        .unwrap_or_default();
    finish_mg3_pickup(
        game,
        id,
        other,
        "$qc_got_item",
        "misc/secret.wav",
        vec![crate::q1::foundation::types::Q1MessageArg::Text(netname)],
    )
}

fn spawn_head(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    if flags & BLOODY_NIGHTMARE_ACTIVE != 0 && (flags & BLOODY_NIGHTMARE_NEWGAME == 0 || flags & TETTE != 0) {
        game.update_entity(id, |entity| {
            entity.classname = String::from("item_health");
            entity.spawnflags = 2;
        })?;
        let angles = game.body(id)?.angles;
        let basis = game.make_vectors(angles);
        let origin = game.body(id)?.origin;
        game.set_origin(
            id,
            vadd(vadd(origin, vscale(basis.right, 16.0)), vscale(basis.forward, -16.0)),
        )?;
        spawn_pickup(game, id)?;
        return Ok(());
    }
    let bunny = flags & BLOODY_NIGHTMARE_ACTIVE != 0;
    game.update_entity(id, |entity| {
        entity.model = String::from(if bunny {
            "progs/g_bunny.mdl"
        } else {
            "progs/item_h_hellkn.mdl"
        });
        entity.fields.insert(
            String::from("netname"),
            String::from(if bunny { "$mg3_qc_newgameplus_item" } else { "" }),
        );
    })?;
    let touch = game.named.touch(&format!(
        "{MG3_ITEM_PREFIX}{}",
        if bunny { "hug_tette" } else { "head_touch" }
    ))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))?;
    game.set_bounds(
        id,
        Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: -16.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 40.0,
            },
        },
    )?;
    start_mg3_item(game, id)
}

fn bloody_start(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    if flags & BLOODY_NIGHTMARE_NEWGAME == 0 {
        game.remove(id)
    } else {
        start_mg3_item(game, id)
    }
}

fn bloody_touch(
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
    let flag = if game
        .entity_ref(id)
        .is_some_and(|entity| entity.classname == "weapon_bloody_sg")
    {
        MG3_BLOODY_SHOTGUN
    } else {
        MG3_BLOODY_SUPER_SHOTGUN
    };
    let parm = addon_player_number(game, other, "parm15")? as i32;
    set_addon_player_number(game, other, "parm15", f64::from(parm | flag))?;
    game.host.inventory.give(&owned, &String::from("q1:ammo/shells"), 30.0);
    let weapon = if flag == MG3_BLOODY_SHOTGUN {
        Q1Weapon::Shotgun
    } else {
        Q1Weapon::Supershotgun
    };
    game.host.inventory.configure(
        &owned,
        &crate::contract::InventoryEntry {
            item: game.weapon_item(weapon),
            count: 1.0,
            capacity: 1.0,
            count_policy: None,
        },
    )?;
    game.select_weapon(&owned, weapon)?;
    finish_mg3_pickup(
        game,
        id,
        other,
        "$mg3_map2_secret_weapon",
        "weapons/pkup.wav",
        Vec::new(),
    )
}

fn spawn_bloody(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let model = if game
        .entity_ref(id)
        .is_some_and(|entity| entity.classname == "weapon_bloody_sg")
    {
        "g_bloodshot"
    } else {
        "g_bloodshot2"
    };
    game.update_entity(id, |entity| {
        entity.model = format!("progs/{model}.mdl");
    })?;
    let touch = game.named.touch(&format!("{MG3_ITEM_PREFIX}bloody_touch"))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))?;
    game.set_bounds(
        id,
        Bounds {
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
        },
    )?;
    game.schedule(id, 0.5, &format!("{MG3_ITEM_PREFIX}bloody_start"))
}

/// Registers mg3 pickups (`registerMg3Pickups`).
pub fn register_mg3_pickups(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        &format!("{MG3_ITEM_PREFIX}ring_touch"),
        Q1CallbackHandlers {
            touch: Some(ring_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("item_draught_insight", spawn_ring)?;
    game.register_spawn("item_draught_stupor", spawn_ring)?;
    game.named.register(
        &format!("{MG3_ITEM_PREFIX}shard_touch"),
        Q1CallbackHandlers {
            touch: Some(shard_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("item_armor_shard", spawn_shard)?;
    game.named.register(
        &format!("{MG3_ITEM_PREFIX}weapon_touch"),
        Q1CallbackHandlers {
            touch: Some(mg3_weapon_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{MG3_ITEM_PREFIX}mjolnir_touch"),
        Q1CallbackHandlers {
            touch: Some(mjolnir_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("weapon_laser_gun", spawn_mg3_weapon)?;
    game.register_spawn("weapon_mjolnir", spawn_mg3_weapon)?;
    game.named.register(
        &format!("{MG3_ITEM_PREFIX}head_touch"),
        Q1CallbackHandlers {
            touch: Some(head_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{MG3_ITEM_PREFIX}hug_tette"),
        Q1CallbackHandlers {
            touch: Some(hug_tette as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("item_head_hellknight", spawn_head)?;
    game.named.register(
        &format!("{MG3_ITEM_PREFIX}bloody_start"),
        Q1CallbackHandlers {
            action: Some(bloody_start as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{MG3_ITEM_PREFIX}bloody_touch"),
        Q1CallbackHandlers {
            touch: Some(bloody_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("weapon_bloody_sg", spawn_bloody)?;
    game.register_spawn("weapon_bloody_ssg", spawn_bloody)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> Q1BaseGuard {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(game, Q1AddonProgram::Mg3);
        crate::q1::addons::items::common::register_mg3_item_callbacks(game).expect("callbacks");
        crate::q1::addons::effects::register_addon_effects(&context, game).expect("effects");
        register_mg3_pickups(game).expect("pickups");
        guard
    }

    #[test]
    fn ranks_prefer_lightning_over_shotguns() {
        assert_eq!(mg3_weapon_rank(Q1Weapon::Lightning), 1);
        assert_eq!(mg3_weapon_rank(Q1Weapon::Mg3Laser), 3);
        assert_eq!(mg3_weapon_rank(Q1Weapon::Axe), 8);
    }

    #[test]
    fn bloody_touch_sets_parm_flag() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        let sg = game.create("weapon_bloody_sg", None, None).expect("sg");
        game.spawn_entity(&sg, None).expect("spawn");
        bloody_touch(&mut game, &sg, &player, None, None).expect("touch");
        assert_eq!(addon_player_number(&game, &player, "parm15"), Ok(1.0));
        assert!(game.entity_ref(&sg).is_none());
    }

    #[test]
    fn rings_spawn_with_height_defaults() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let ring = game.create("item_draught_insight", None, None).expect("ring");
        game.spawn_entity(&ring, None).expect("spawn");
        let entity = game.entity_ref(&ring).expect("entity");
        assert_eq!(entity.model, "progs/gold_ring.mdl");
        assert_eq!(entity.number("height"), -2048.0);
        assert_eq!(entity.count, 1.0);
    }
}
