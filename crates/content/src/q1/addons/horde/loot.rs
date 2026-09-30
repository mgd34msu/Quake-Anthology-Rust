//! Q1 horde loot (`src/content/q1/addons/horde/loot.ts`).
//!
//! `quakec_mg1/horde.qc` item and key functions. GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::contract::{PickupResource, ProtectionChannel, RegularArmorState};
use crate::q1::addons::context::{
    addon_alpha, addon_broadcast, addon_cvar, fround, set_addon_number, set_addon_player_number,
};
use crate::q1::addons::horde::{horde_change_keys, horde_is_bot, horde_living_players, horde_manager, horde_schedule};
use crate::q1::base::provider::update_base;
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::pickups::{touch_q1_pickup, Q1TouchTake};
use crate::q1::foundation::types::{vadd, vsub, Q1Effect, Q1MessageArg, Q1MoveType, Q1Solid, Q1SoundChannel, ZERO};
use crate::q1::Q1Error;

struct HordeAmmo {
    item: &'static str,
    classname: &'static str,
    model: &'static str,
    label: &'static str,
    amount: f64,
    capacity: f64,
}

fn ammo_definition(kind: i32, big: bool) -> HordeAmmo {
    if kind == 1 {
        HordeAmmo {
            item: "q1:ammo/shells",
            classname: "item_shells",
            model: "shell",
            label: "$qc_shells",
            amount: if big { 40.0 } else { 20.0 },
            capacity: 100.0,
        }
    } else if kind == 2 {
        HordeAmmo {
            item: "q1:ammo/nails",
            classname: "item_spikes",
            model: "nail",
            label: "$qc_nails",
            amount: if big { 50.0 } else { 25.0 },
            capacity: 200.0,
        }
    } else if kind == 3 {
        HordeAmmo {
            item: "q1:ammo/rockets",
            classname: "item_rockets",
            model: "rock",
            label: "$qc_rockets",
            amount: if big { 10.0 } else { 5.0 },
            capacity: 100.0,
        }
    } else {
        HordeAmmo {
            item: "q1:ammo/cells",
            classname: "item_cells",
            model: "batt",
            label: "$qc_cells",
            amount: if big { 12.0 } else { 6.0 },
            capacity: 100.0,
        }
    }
}

/// Shared loot take feedback (`taken`).
fn loot_taken(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId, sound: &str) -> Result<(), Q1Error> {
    if let Some(actor) = game.host.actors.resolve_owned(other) {
        game.sound(actor.id(), sound, Q1SoundChannel::Item, 1.0, 1.0)?;
    }
    if !game.is_live(id) || !game.host.actors.is_live(other) {
        return Ok(());
    }
    let origin = game.body(id)?.origin;
    game.effect(Q1Effect::Pickup, origin, Some(other), 1);
    if !game.is_live(id) || !game.host.actors.is_live(other) {
        return Ok(());
    }
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::None;
        entity.model = String::new();
    })?;
    game.link(id)
}

fn ammo_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) || game.health(other) <= 0.0 {
        return Ok(());
    }
    let definition = ammo_definition(
        game.entity_ref(id).map(|entity| entity.number("weapon")).unwrap_or(0.0) as i32,
        false,
    );
    if game.host.inventory.count(other, &definition.item.to_string()) >= definition.capacity {
        return Ok(());
    }
    if let Some(state) = game.player_ref(other).cloned() {
        let best = game.choose_best(&state.actor, None)?;
        if state.weapon == best {
            game.select_weapon(&state.actor, best)?;
        }
    }
    game.message(
        Some(other),
        "$qc_got_item",
        false,
        vec![Q1MessageArg::Text(definition.label.to_string())],
    );
    loot_taken(game, id, other, "weapons/lock4.wav")?;
    let amount = game.entity_ref(id).map(|entity| entity.number("aflag")).unwrap_or(0.0);
    for player in horde_living_players(game) {
        if let Some(actor) = game.host.actors.resolve_owned(&player) {
            game.host.inventory.give(&actor, &definition.item.to_string(), amount);
        }
    }
    if let Some(owner) = game.entity_ref(id).and_then(|entity| entity.owner.clone()) {
        if game.entity_ref(&owner).is_some() {
            horde_schedule(game, &owner, "ammo", 20.0)?;
        }
    }
    game.remove(id)
}

fn health_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let state = match game.player_ref(other).cloned() {
        Some(state) => state,
        None => return Ok(()),
    };
    let health = game.health(other);
    if health <= 0.0 || health >= state.max_health {
        return Ok(());
    }
    if addon_cvar(game, "horde")? != 0.0 && update_base(game, |state| state.campaign.read_flags())? & 2 != 0 {
        set_addon_player_number(game, other, "hunger_time", game.time + 10.0)?;
    }
    let amount = game
        .entity_ref(id)
        .map(|entity| entity.number("healamount"))
        .unwrap_or(0.0);
    game.host
        .combat
        .set_health(&state.actor, (health + amount).min(state.max_health))?;
    game.message(
        Some(other),
        "$qc_item_health",
        false,
        vec![Q1MessageArg::Number(amount)],
    );
    let noise = game
        .entity_ref(id)
        .map(|entity| entity.text("noise"))
        .unwrap_or_default();
    loot_taken(game, id, other, &noise)?;
    let owner = game.entity_ref(id).and_then(|entity| entity.owner.clone());
    if let Some(owner) = owner {
        game.update_entity(&owner, |entity| entity.wait = 0.0)?;
    }
    game.remove(id)
}

struct ArmorTake {
    id: ActorId,
    other: ActorId,
}

impl Q1TouchTake for ArmorTake {
    fn original(&mut self, game: &mut Q1EntityServices) -> Result<bool, Q1Error> {
        let classname = game
            .entity_ref(&self.id)
            .map(|entity| entity.classname.clone())
            .unwrap_or_default();
        let absorption = if classname == "item_armor1" {
            0.3
        } else if classname == "item_armor2" {
            0.6
        } else {
            0.8
        };
        let points = if classname == "item_armor1" {
            100.0
        } else if classname == "item_armor2" {
            150.0
        } else {
            200.0
        };
        let armor = game
            .host
            .combat
            .read(&self.other)
            .map(|combat| combat.armor.regular.clone());
        if matches!(armor, Some(RegularArmorState::Source { .. })) {
            return Ok(false);
        }
        let protection = match &armor {
            None | Some(RegularArmorState::None) => 0.0,
            Some(RegularArmorState::Q1 { points, absorption, .. }) => points * absorption,
            Some(RegularArmorState::Q2 {
                points,
                normal_protection,
                ..
            }) => points * normal_protection,
            Some(RegularArmorState::Q3 { points, protection, .. }) => points * protection,
            Some(RegularArmorState::Source { .. }) => 0.0,
        };
        if protection >= absorption * points {
            return Ok(false);
        }
        let owned = game
            .player_owned(&self.other)
            .ok_or_else(|| crate::q1::q1_error("Missing Q1 armor player"))?;
        game.host.combat.set_regular_armor(
            &owned,
            &RegularArmorState::Q1 {
                points,
                absorption,
                item: format!("q1:{classname}"),
            },
        )?;
        Ok(true)
    }

    fn complete(&mut self, game: &mut Q1EntityServices, taken: bool) -> Result<(), Q1Error> {
        if !taken {
            return Ok(());
        }
        game.message(Some(&self.other), "$qc_item_armor", false, Vec::new());
        if !game.is_live(&self.id) || !game.host.actors.is_live(&self.other) {
            return Ok(());
        }
        loot_taken(game, &self.id, &self.other, "items/armor1.wav")?;
        if !game.is_live(&self.id) {
            return Ok(());
        }
        let owner = game.entity_ref(&self.id).and_then(|entity| entity.owner.clone());
        if let Some(owner) = owner {
            game.update_entity(&owner, |entity| entity.wait = 0.0)?;
        }
        game.remove(&self.id)
    }
}

fn armor_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) || game.health(other) <= 0.0 {
        return Ok(());
    }
    if game.host.actors.resolve_owned(other).is_none() {
        return Ok(());
    }
    let item = game
        .entity_ref(id)
        .map(|entity| format!("q1:{}", entity.classname))
        .unwrap_or_default();
    touch_q1_pickup(
        game,
        id,
        other,
        item,
        Some(PickupResource::Protection {
            channel: ProtectionChannel::Regular,
        }),
        ArmorTake {
            id: id.clone(),
            other: other.clone(),
        },
        None,
    )
}

fn key_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) || game.health(other) <= 0.0 || horde_is_bot(game, other)? {
        return Ok(());
    }
    let netname = game
        .entity_ref(id)
        .map(|entity| entity.text("netname"))
        .unwrap_or_default();
    game.message(Some(other), "$qc_got_item", false, vec![Q1MessageArg::Text(netname)]);
    let noise = game
        .entity_ref(id)
        .map(|entity| entity.text("noise"))
        .unwrap_or_default();
    loot_taken(game, id, other, &noise)?;
    let gold = game
        .entity_ref(id)
        .map(|entity| entity.text("horde.key"))
        .unwrap_or_default()
        == "gold";
    horde_change_keys(game, if gold { "gold" } else { "silver" }, 1)?;
    if let Some(manager) = horde_manager(game) {
        if game
            .entity_ref(&manager)
            .map(|entity| entity.number("key_spawned"))
            .unwrap_or(0.0)
            != 0.0
        {
            game.update_entity(&manager, |entity| entity.wait = 1.0)?;
            horde_schedule(game, &manager, "countdown", 0.0)?;
        }
    }
    game.remove(id)
}

fn ammo_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let big = game.host.random() * 4.0 <= 1.0;
    let roll = game.host.random() * 20.0;
    let kind = if roll <= 7.0 {
        1
    } else if roll <= 14.0 {
        2
    } else if roll <= 17.0 {
        3
    } else {
        4
    };
    let definition = ammo_definition(kind, big);
    let item = game.create(definition.classname, None, None)?;
    let offset: f32 = if big { 16.0 } else { 12.0 };
    let position = vsub(
        game.body(id)?.origin,
        Vec3 {
            x: offset,
            y: offset,
            z: 0.0,
        },
    );
    game.update_entity(&item, |entity| {
        entity.model = format!("maps/b_{}{}.bsp", definition.model, i32::from(big));
        entity
            .fields
            .insert(String::from("netname"), definition.label.to_string());
    })?;
    set_addon_number(game, &item, "weapon", f64::from(kind))?;
    set_addon_number(game, &item, "aflag", definition.amount)?;
    let touch = game.named.touch("mg1:horde:ammo_touch")?;
    game.update_entity(&item, |entity| {
        entity.owner = Some(id.clone());
        entity.movement = Q1MoveType::Toss;
        entity.solid = Q1Solid::Trigger;
        entity.movement_flags = 256;
        entity.touch = Some(touch);
    })?;
    game.update_entity(id, |entity| entity.wait = 1.0)?;
    game.set_body(
        &item,
        &BodyPatch {
            origin: Some(vadd(position, Vec3 { x: 0.0, y: 0.0, z: 1.0 })),
            bounds: Some(Bounds {
                min: ZERO,
                max: Vec3 {
                    x: 32.0,
                    y: 32.0,
                    z: 56.0,
                },
            }),
            ..Default::default()
        },
    )?;
    game.link(&item)?;
    game.effect(
        Q1Effect::Teleport,
        vadd(position, Vec3 { x: 0.0, y: 0.0, z: 8.0 }),
        None,
        1,
    );
    Ok(())
}

fn item_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let health = game.host.random() * 6.0 < 5.0;
    let item = game.create(if health { "item_health" } else { "item_armor1" }, None, None)?;
    let position = if health {
        vsub(
            game.body(id)?.origin,
            Vec3 {
                x: 16.0,
                y: 16.0,
                z: 0.0,
            },
        )
    } else {
        game.body(id)?.origin
    };
    game.update_entity(&item, |entity| {
        entity.model = String::from(if health { "maps/b_bh25.bsp" } else { "progs/armor.mdl" });
        entity
            .fields
            .insert(String::from("noise"), String::from("items/health1.wav"));
    })?;
    set_addon_number(game, &item, "healamount", 25.0)?;
    let touch = game.named.touch(if health {
        "mg1:horde:health_touch"
    } else {
        "mg1:horde:armor_touch"
    })?;
    game.update_entity(&item, |entity| {
        entity.touch = Some(touch);
        entity.owner = Some(id.clone());
        entity.movement = Q1MoveType::Toss;
        entity.solid = Q1Solid::Trigger;
        entity.movement_flags = 256;
    })?;
    game.update_entity(id, |entity| entity.wait = 1.0)?;
    game.set_body(
        &item,
        &BodyPatch {
            origin: Some(vadd(position, Vec3 { x: 0.0, y: 0.0, z: 1.0 })),
            bounds: Some(if health {
                Bounds {
                    min: ZERO,
                    max: Vec3 {
                        x: 32.0,
                        y: 32.0,
                        z: 56.0,
                    },
                }
            } else {
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
                }
            }),
            ..Default::default()
        },
    )?;
    game.link(&item)?;
    game.effect(
        Q1Effect::Teleport,
        vadd(position, Vec3 { x: 0.0, y: 0.0, z: 8.0 }),
        None,
        1,
    );
    Ok(())
}

fn spawn_key(game: &mut Q1EntityServices, id: &ActorId, gold: bool) -> Result<(), Q1Error> {
    let name = if gold { "gold" } else { "silver" };
    let metal = game.world_type == 1;
    let base = game.world_type == 2;
    let label = format!(
        "{name}_{}",
        if metal {
            "runekey"
        } else if base {
            "keycard"
        } else {
            "key"
        }
    );
    let item = game.create(if gold { "item_key2" } else { "item_key1" }, None, None)?;
    game.update_entity(&item, |entity| {
        entity.fields.insert(String::from("horde.key"), name.to_string());
        entity.fields.insert(String::from("netname"), format!("$qc_{label}"));
        entity.fields.insert(
            String::from("noise"),
            String::from(if metal {
                "misc/runekey.wav"
            } else if base {
                "misc/basekey.wav"
            } else {
                "misc/medkey.wav"
            }),
        );
        entity.model = format!(
            "progs/{}_{}_key.mdl",
            if metal {
                "m"
            } else if base {
                "b"
            } else {
                "w"
            },
            if gold { "g" } else { "s" }
        );
        entity.target = String::from("horde_manager");
        entity.movement = Q1MoveType::Toss;
        entity.solid = Q1Solid::Trigger;
        entity.movement_flags = 256;
        if !metal {
            entity.effects = 4;
        }
    })?;
    let touch = game.named.touch("mg1:horde:key_touch")?;
    game.update_entity(&item, |entity| entity.touch = Some(touch))?;
    game.set_body(
        &item,
        &BodyPatch {
            origin: Some(vadd(
                game.body(id)?.origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 32.0,
                },
            )),
            velocity: Some(Vec3 {
                x: 0.0,
                y: 0.0,
                z: 255.0,
            }),
            bounds: Some(Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -25.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 32.0,
                },
            }),
            ..Default::default()
        },
    )?;
    game.link(&item)?;
    addon_broadcast(game, &format!("$qc_horde_{label}_appears"));
    let origin = game.body(&item)?.origin;
    game.effect(Q1Effect::Teleport, origin, None, 1);
    if let Some(manager) = horde_manager(game) {
        set_addon_number(game, &manager, "key_spawned", 1.0)?;
    }
    Ok(())
}

fn silver_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_key(game, id, false)
}

fn gold_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_key(game, id, true)
}

fn powerup_fade(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let alpha = game.entity_ref(id).map(|entity| entity.number("alpha")).unwrap_or(0.0);
    if alpha <= 0.0 {
        return game.remove(id);
    }
    addon_alpha(game, id, fround(alpha - 0.25 * game.frame_seconds))?;
    horde_schedule(game, id, "powerup_fade", 0.0)
}

fn powerup_wait(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.body(id)?.velocity.z < 0.0 {
        return game.remove(id);
    }
    addon_alpha(game, id, 1.0)?;
    horde_schedule(game, id, "powerup_fade", 0.0)
}

fn spawn_horde_ammo(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let delay = 10.0 + game.host.random() * 3.0;
    horde_schedule(game, id, "ammo", delay)
}

fn spawn_horde_wait(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.wait = 0.0)
}

/// Registers horde loot (`registerHordeLoot`).
pub fn register_horde_loot(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "mg1:horde:ammo_touch",
        Q1CallbackHandlers {
            touch: Some(ammo_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:health_touch",
        Q1CallbackHandlers {
            touch: Some(health_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:armor_touch",
        Q1CallbackHandlers {
            touch: Some(armor_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:key_touch",
        Q1CallbackHandlers {
            touch: Some(key_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:ammo",
        Q1CallbackHandlers {
            action: Some(ammo_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:item",
        Q1CallbackHandlers {
            action: Some(item_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:silver",
        Q1CallbackHandlers {
            action: Some(silver_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:gold",
        Q1CallbackHandlers {
            action: Some(gold_action as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:powerup_fade",
        Q1CallbackHandlers {
            action: Some(powerup_fade as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mg1:horde:powerup_wait",
        Q1CallbackHandlers {
            action: Some(powerup_wait as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.register_spawn("info_horde_ammo", spawn_horde_ammo)?;
    game.register_spawn("info_horde_item", spawn_horde_wait)?;
    game.register_spawn("info_horde_key", spawn_horde_wait)
}

/// Spawns a horde powerup (`spawnHordePowerup`).
pub fn spawn_horde_powerup(game: &mut Q1EntityServices, dead: &ActorId) -> Result<(), Q1Error> {
    let manager = match horde_manager(game) {
        Some(manager) => manager,
        None => return Ok(()),
    };
    let chance = game
        .entity_ref(&manager)
        .map(|entity| entity.number("powerup_chance"))
        .unwrap_or(0.0);
    let chance = if chance == 0.0 { 0.025 } else { chance };
    if game.host.random() >= chance {
        return set_addon_number(game, &manager, "powerup_chance", chance + 0.025);
    }
    set_addon_number(game, &manager, "powerup_chance", 0.025)?;
    let invulnerable = game.host.random() < 0.25;
    let powerup = game.create(
        if invulnerable {
            "item_artifact_invulnerability"
        } else {
            "item_artifact_super_damage"
        },
        None,
        None,
    )?;
    game.spawn_entity(&powerup, None)?;
    game.update_entity(&powerup, |entity| {
        entity.movement_flags = 256;
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::Bounce;
    })?;
    game.set_body(
        &powerup,
        &BodyPatch {
            origin: Some(game.body(dead)?.origin),
            velocity: Some(Vec3 {
                x: 0.0,
                y: 0.0,
                z: 300.0,
            }),
            bounds: Some(Bounds {
                min: Vec3 {
                    x: -12.0,
                    y: -12.0,
                    z: -12.0,
                },
                max: Vec3 {
                    x: 12.0,
                    y: 12.0,
                    z: 12.0,
                },
            }),
            ..Default::default()
        },
    )?;
    game.link(&powerup)?;
    horde_schedule(game, &powerup, "powerup_wait", 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::addons::horde::register_q1_horde;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> Q1BaseGuard {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Mg1);
        register_q1_horde(game, Box::new(TestHordeServices)).expect("horde");
        guard
    }

    struct TestHordeServices;

    impl crate::q1::addons::horde::types::Q1HordeServices for TestHordeServices {
        fn dead_flag(&mut self, _player: &ActorId) -> i32 {
            0
        }
        fn no_target(&mut self, _player: &ActorId) -> bool {
            false
        }
        fn is_bot(&mut self, _player: &ActorId) -> bool {
            false
        }
        fn respawn_teammate(&mut self, _player: &ActorId) {}
        fn add_score(&mut self, _player: &ActorId, _delta: f64) {}
        fn restart_session(&mut self, _map: &str, _starting_server_flags: i32) {}
    }

    #[test]
    fn ammo_definitions_match_donor() {
        let shells = ammo_definition(1, true);
        assert_eq!(shells.amount, 40.0);
        assert_eq!(shells.capacity, 100.0);
        let cells = ammo_definition(4, false);
        assert_eq!(cells.amount, 6.0);
    }

    #[test]
    fn ammo_spawns_from_caches() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let cache = game.create("info_horde_ammo", None, None).expect("cache");
        game.spawn_entity(&cache, None).expect("spawn");
        ammo_action(&mut game, &cache).expect("ammo");
        let items = game
            .entity_ids()
            .into_iter()
            .filter(|id| {
                game.entity_ref(id).is_some_and(|entity| {
                    entity.classname.starts_with("item_")
                        && game
                            .entity_ref(id)
                            .is_some_and(|entity| entity.owner == Some(cache.clone()))
                })
            })
            .count();
        assert_eq!(items, 1);
        assert_eq!(game.entity_ref(&cache).expect("cache").wait, 1.0);
    }

    #[test]
    fn health_heals_living_players() {
        let mut game = test_game();
        let _guard = setup(&mut game);
        let player = attach_test_player(&mut game);
        game.set_health(&player, 50.0).expect("hurt");
        let pack = game.create("item_health", None, None).expect("pack");
        set_addon_number(&mut game, &pack, "healamount", 25.0).expect("amount");
        health_touch(&mut game, &pack, &player, None, None).expect("touch");
        assert_eq!(game.health(&player), 75.0);
        assert!(game.entity_ref(&pack).is_none());
    }
}
