//! Q1 campaign addons (`src/content/q1/addons/campaign.ts`).
//!
//! `quakec_mg1/items_runes.qc`, map-specific words, and
//! `quakec_mg3/items_runes.qc`. Copyright (C) 1996-2026 id Software
//! LLC. GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::{
    addon_alpha, addon_broadcast, addon_cvar, addon_emit, addon_program, init_trigger, require_entity,
    set_addon_number, Q1AddonContext, Q1AddonEvent, Q1AddonProgram,
};
use crate::q1::base::provider::update_base;
use crate::q1::base::rules::{
    Q1FinaleDecision, Q1IntermissionResult, Q1IntermissionRule, Q1SourceFinale, Q1SpawnDecision,
};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity::Q1MoverState;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyPatch, DamageDelivery, TouchSurface};
use crate::q1::foundation::movers::{door_up, spawn_button};
use crate::q1::foundation::types::{
    vadd, Q1Effect, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, Q1TraceRequest, ZERO,
};
use crate::q1::{q1_error, Q1Error};

/// All Dimension of the Machine sigil bits.
pub const MG1_ALL_SIGILS: i32 = 31;
/// Bit shift of the last collected sigil in the campaign flags.
pub const MG1_LAST_SIGIL_SHIFT: u32 = 6;
/// Honey rune mask.
pub const MG3_RUNE_MASK: i32 = 15;
/// Bloody nightmare active flag.
pub const BLOODY_NIGHTMARE_ACTIVE: i32 = 64;
/// Bloody nightmare discovered flag.
pub const BLOODY_NIGHTMARE_DISCOVERED: i32 = 128;
/// Bloody nightmare new-game flag.
pub const BLOODY_NIGHTMARE_NEWGAME: i32 = 256;

/// Last collected sigil bits (`mg1LastSigil`).
#[must_use]
pub fn mg1_last_sigil(flags: i32) -> i32 {
    (((flags as u32) >> MG1_LAST_SIGIL_SHIFT) & (MG1_ALL_SIGILS as u32)) as i32
}

/// Clear the last collected sigil bits (`mg1ClearLastSigil`).
#[must_use]
pub fn mg1_clear_last_sigil(flags: i32) -> i32 {
    flags & !((MG1_ALL_SIGILS as u32) << MG1_LAST_SIGIL_SHIFT) as i32
}

/// Count collected Honey runes (`mg3RuneCount`).
#[must_use]
pub fn mg3_rune_count(flags: i32) -> i32 {
    [1, 2, 4, 8].iter().filter(|bit| (flags & **bit) != 0).count() as i32
}

fn prefix(game: &Q1EntityServices) -> Result<String, Q1Error> {
    Ok(format!("{}:campaign:", addon_program(game)?.as_str()))
}

fn schedule_action(game: &mut Q1EntityServices, id: &ActorId, name: &str, delay: f64) -> Result<(), Q1Error> {
    let callback = game.named.action(name)?;
    game.schedule(id, delay, &callback)
}

fn gib_monster_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let world = game
        .world
        .clone()
        .ok_or_else(|| q1_error("Horde exit requires the source world actor"))?;
    game.damage(id, Some(&world), Some(&world), 4000.0, &Q1DamageParams::default());
    Ok(())
}

fn campaign_touch(game: &mut Q1EntityServices, trigger: &ActorId, _player: &ActorId) -> Result<(), Q1Error> {
    let endtext = require_entity(game, trigger)?.text("endtext");
    if let Some(world) = game.world.clone() {
        game.update_entity(&world, |entity| {
            entity.fields.insert("addon.intermissiontext".to_string(), endtext);
        })?;
    }
    if game.map_name == "mgend" {
        update_base(game, |state| state.campaign.write_flags(0))?;
    }
    Ok(())
}

fn campaign_begin(game: &mut Q1EntityServices, map: &str, _cause: Option<&ActorId>) -> Result<(), Q1Error> {
    let program = addon_program(game)?;
    // The Rust base state retains no official-campaign reader, so the
    // rule follows the donor default (`officialCampaign !== false`).
    let complete = if game.map_name == "e5end" {
        Some("E5END")
    } else if game.map_name == "mgend" {
        Some("MGEND")
    } else if program == Q1AddonProgram::Mg3 && game.map_name == "boss" && (map == "start" || map == "map1") {
        Some("MG3")
    } else {
        None
    };
    if let Some(complete) = complete {
        game.host.emit(Q1Event::Achievement {
            player: None,
            id: format!("ACH_COMPLETE_{complete}"),
        });
        if game.options().skill == 3 {
            game.host.emit(Q1Event::Achievement {
                player: None,
                id: format!("ACH_COMPLETE_{complete}_NIGHTMARE"),
            });
        }
    }
    if game.map_name == "e5m6" && map == "e5sm2" {
        game.host.emit(Q1Event::Achievement {
            player: None,
            id: String::from("ACH_FIND_E5M8"),
        });
    }
    if game.map_name == "mge1m1" && map == "mge1m3" {
        game.host.emit(Q1Event::Achievement {
            player: None,
            id: String::from("ACH_FIND_MGE1M3"),
        });
    }
    if program != Q1AddonProgram::Mg3 {
        let manager = game
            .entities
            .iter()
            .find(|(_, entity)| entity.classname == "horde_manager")
            .map(|(id, _)| id.clone());
        if let Some(manager) = manager {
            game.cancel(&manager);
            let gib = format!("{}:campaign:gib_monster", program.as_str());
            let monsters: Vec<ActorId> = game
                .entities
                .iter()
                .filter(|(_, entity)| entity.text("category") == "monster")
                .map(|(id, _)| id.clone())
                .collect();
            for monster in &monsters {
                let delay = 0.2 + game.host.random() * 1.8;
                schedule_action(game, monster, &gib, delay)?;
            }
        }
    }
    Ok(())
}

fn campaign_finale(game: &mut Q1EntityServices, stage: i32, _next_map: &str) -> Result<Q1FinaleDecision, Q1Error> {
    let program = addon_program(game)?;
    if stage == 2 {
        let registered = update_base(game, |state| state.registered)?;
        let map = game.map_name.clone();
        let text = if map == "e1m7" {
            if registered {
                String::from("$qc_finale_e1")
            } else {
                String::from("$qc_finale_e1_shareware")
            }
        } else if map == "e2m6" {
            String::from("$qc_finale_e2")
        } else if map == "e3m6" {
            String::from("$qc_finale_e3")
        } else if map == "e4m7" {
            String::from("$qc_finale_e4")
        } else if let Some(world) = game.world.clone() {
            let entity = require_entity(game, &world)?.clone();
            let endtext = entity.text("endtext");
            if !endtext.is_empty() {
                endtext
            } else {
                entity.text("addon.intermissiontext")
            }
        } else {
            String::new()
        };
        if let Some(world) = game.world.clone() {
            game.update_entity(&world, |entity| {
                entity
                    .fields
                    .insert(String::from("addon.intermissiontext"), String::new());
            })?;
        }
        return Ok(if text.is_empty() {
            Q1FinaleDecision::Delegate
        } else {
            Q1FinaleDecision::Finale(Q1SourceFinale::Finale { text, track: 2 })
        });
    }
    if stage == 3 {
        let (registered, flags) = update_base(game, |state| (state.registered, state.campaign.read_flags()))?;
        if !registered {
            return Ok(Q1FinaleDecision::Finale(Q1SourceFinale::SellScreen));
        }
        if program != Q1AddonProgram::Mg3 && (flags & MG1_ALL_SIGILS) == MG1_ALL_SIGILS {
            return Ok(Q1FinaleDecision::Finale(Q1SourceFinale::Finale {
                text: String::from("$qc_mg1_endtext_all_runes"),
                track: 2,
            }));
        }
    }
    Ok(Q1FinaleDecision::Delegate)
}

fn campaign_travel(game: &mut Q1EntityServices, map: &str, cause: Option<&ActorId>) -> Result<bool, Q1Error> {
    let program = addon_program(game)?;
    if addon_cvar(game, "samelevel")? != 0.0 {
        let current = game.map_name.clone();
        game.travel(&current, cause);
        return Ok(true);
    }
    if program != Q1AddonProgram::Mg3 && addon_cvar(game, "horde")? != 0.0 {
        let next = if game.map_name == "horde1" {
            "horde2"
        } else if game.map_name == "horde2" {
            "horde3"
        } else if game.map_name == "horde3" {
            "horde4"
        } else {
            "horde1"
        };
        game.travel(next, cause);
        return Ok(true);
    }
    if program == Q1AddonProgram::Mg3
        && game.map_name == "hub"
        && map == "secret2"
        && (update_base(game, |state| state.campaign.read_flags())?
            & (BLOODY_NIGHTMARE_ACTIVE | BLOODY_NIGHTMARE_NEWGAME))
            == (BLOODY_NIGHTMARE_ACTIVE | BLOODY_NIGHTMARE_NEWGAME)
    {
        game.travel("boss2", cause);
        return Ok(true);
    }
    if map == "start" && !game.options().coop && game.options().deathmatch == 0 {
        game.host.emit(Q1Event::ServerCommand {
            text: String::from("menu_credits\ndisconnect\n"),
        });
        return Ok(true);
    }
    Ok(false)
}

fn sigil_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let program = addon_program(game)?;
    let entity = require_entity(game, id)?.clone();
    if entity.solid != Q1Solid::Trigger || !game.is_player(other) || game.health(other) <= 0.0 {
        return Ok(());
    }
    addon_broadcast(game, &entity.text("netname"));
    if game.host.actors.resolve_owned(other).is_some() {
        game.sound(other, "misc/runekey.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
    }
    let origin = game.body(id)?.origin;
    game.effect(Q1Effect::Pickup, origin, Some(other), 1);
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::None;
        entity.model.clear();
    })?;
    game.link(id)?;
    let bits = if program == Q1AddonProgram::Mg3 {
        require_entity(game, id)?.number("style") as i32
    } else {
        require_entity(game, id)?.spawnflags & MG1_ALL_SIGILS
    };
    update_base(game, |state| {
        let flags = state.campaign.read_flags();
        state.campaign.write_flags(
            flags
                | bits
                | (if program == Q1AddonProgram::Mg3 {
                    0
                } else {
                    bits << MG1_LAST_SIGIL_SHIFT
                }),
        );
    })?;
    if program != Q1AddonProgram::Mg3 && addon_cvar(game, "horde")? != 0.0 && (bits & 2) != 0 {
        let until = game.time + 10.0;
        let players = (game.host.players)();
        for player in &players {
            crate::q1::addons::context::set_addon_player_number(game, player, "hunger_time", until)?;
        }
    }
    let player = other.clone();
    addon_emit(game, Q1AddonEvent::RuneCollected { player, bits, program })?;
    game.use_targets(id, Some(other))
}

fn drop_item_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let body = game.body(id)?;
    let start = vadd(body.origin, Vec3 { x: 0.0, y: 0.0, z: 6.0 });
    let floor = game.host.trace(&Q1TraceRequest {
        start,
        end: vadd(
            start,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: -256.0,
            },
        ),
        bounds: body.bounds,
        ignore: Some(id.clone()),
        monsters: true,
        missile: false,
    });
    if floor.all_solid || floor.fraction == 1.0 {
        return game.remove(id);
    }
    game.set_body(
        id,
        &BodyPatch {
            origin: Some(floor.end),
            velocity: Some(ZERO),
            ground: Some(floor.actor.clone()),
            ..Default::default()
        },
    )?;
    game.link(id)
}

fn use_targets_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let activator = require_entity(game, id)?.activator.clone();
    game.use_targets(id, activator.as_ref())
}

fn indicator_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    addon_alpha(game, id, 1.0)?;
    game.use_targets(id, activator)
}

fn hub_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) {
        return Ok(());
    }
    if matches!(game.options().no_exit, Some(1)) || (game.options().no_exit == Some(2) && game.map_name != "start") {
        game.damage(
            other,
            Some(id),
            Some(id),
            50000.0,
            &Q1DamageParams {
                weapon: None,
                delivery: DamageDelivery::Direct,
                death_type: String::from("exit"),
                armor_effect: None,
            },
        );
        return Ok(());
    }
    let rules = update_base(game, |state| state.level_rules.clone())?;
    rules.changelevel_touched(game, id, other)?;
    if let Some(player_exited) = update_base(game, |state| state.player_exited)? {
        player_exited(other.clone());
    }
    game.use_targets(id, Some(other))?;
    let same = update_base(game, |state| state.same_level)?.is_some_and(|same| same());
    let entity = require_entity(game, id)?.clone();
    let map = if same {
        game.map_name.clone()
    } else {
        entity.text("map")
    };
    if (entity.spawnflags & 1) != 0 && game.options().deathmatch == 0 && entity.text("endtext").is_empty() {
        return rules.travel_to(game, &map, Some(other));
    }
    game.update_entity(id, |entity| {
        entity.touch = None;
        entity.activator = Some(other.clone());
    })?;
    schedule_action(game, id, &format!("{}hub_changelevel", prefix(game)?), 0.1)
}

fn hub_changelevel_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    let mut rules = update_base(game, |state| state.level_rules.clone())?;
    rules.begin(game, &entity.text("map"), entity.activator.as_ref())?;
    let moved = rules.clone();
    update_base(game, |state| state.level_rules = moved)?;
    if (entity.spawnflags & 1) != 0 {
        let mut rules = update_base(game, |state| state.level_rules.clone())?;
        let result = rules.advance_finale(game, game.time)?;
        update_base(game, |state| state.level_rules = rules)?;
        match result {
            Q1IntermissionResult::Finale { text, track } => {
                addon_emit(game, Q1AddonEvent::Music { track, loop_track: 3 })?;
                game.host.emit(Q1Event::Finale { text, stage: 2 });
            }
            Q1IntermissionResult::SellScreen => addon_emit(game, Q1AddonEvent::SellScreen)?,
            Q1IntermissionResult::Waiting | Q1IntermissionResult::Travel { .. } => {}
        }
    }
    Ok(())
}

fn electrode_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    normal: Option<Vec3>,
    surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) || game.health(other) <= 0.0 {
        return Ok(());
    }
    let button = game.named.touch_handler("button_touch")?;
    let surface = surface.cloned();
    button(game, id, other, normal, surface.as_ref())?;
    game.update_entity(id, |entity| entity.touch = None)?;
    let count = require_entity(game, id)?.number("cnt");
    let targets: Vec<ActorId> = game
        .entities
        .iter()
        .filter(|(_, entity)| entity.classname == "mge2m2_electrode_target" && entity.number("cnt") == count)
        .map(|(id, _)| id.clone())
        .collect();
    for target in &targets {
        schedule_action(game, target, "SUB_Remove", 0.1)?;
    }
    Ok(())
}

fn egg_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let target = require_entity(game, id)?.target.clone();
    for door in game.find(&target) {
        if require_entity(game, &door)?.classname != "func_door" {
            continue;
        }
        let entity = require_entity(game, &door)?.clone();
        let movedir = if entity.fields.contains_key("dest2") {
            entity.vector("dest2")
        } else {
            entity.dest2
        };
        let origin = game.body(&door)?.origin;
        let pos2 = vadd(origin, movedir);
        game.update_entity(&door, |entity| {
            entity.movedir = movedir;
            entity.pos1 = origin;
            entity.pos2 = pos2;
            entity.state = Q1MoverState::Bottom;
            entity.speed = 500.0;
            entity
                .fields
                .insert(String::from("noise1"), String::from("doors/drclos4.wav"));
            entity
                .fields
                .insert(String::from("noise2"), String::from("doors/doormv1.wav"));
        })?;
        door_up(game, &door)?;
    }
    Ok(())
}

fn fix_pickup(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let enable = require_entity(game, id)?.activated;
    game.update_entity(id, |entity| entity.activated = true)?;
    let runes: Vec<ActorId> = game
        .entities
        .iter()
        .filter(|(_, entity)| entity.classname == "item_sigil")
        .map(|(id, _)| id.clone())
        .collect();
    for rune in &runes {
        if enable {
            let touch = game.named.touch(&format!("{}sigil_touch", prefix(game)?))?;
            game.update_entity(rune, |entity| entity.touch = Some(touch))?;
        } else {
            game.update_entity(rune, |entity| entity.touch = None)?;
        }
    }
    Ok(())
}

fn fix_pickup_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    fix_pickup(game, id)
}

fn fix_pickup_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    fix_pickup(game, id)
}

fn spawn_item_sigil(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let program = addon_program(game)?;
    let mission3 = program == Q1AddonProgram::Mg3;
    let entity = require_entity(game, id)?.clone();
    let spawned = if mission3 { entity.spawnflags & 128 } else { 0 };
    let mut spawnflags = entity.spawnflags;
    if spawnflags == 0 {
        spawnflags = 1;
    }
    let bits: &[i32] = if mission3 { &[1, 2, 4, 8] } else { &[1, 2, 4, 8, 16, 32] };
    let found = bits.iter().enumerate().find(|(_, bit)| (spawnflags & **bit) != 0);
    let (index, bit) = found
        .map(|(index, bit)| (index, *bit))
        .ok_or_else(|| q1_error(format!("{} item_sigil has no source rune model", program.as_str())))?;
    let touch = game
        .named
        .touch(&format!("{}:campaign:sigil_touch", program.as_str()))?;
    game.update_entity(id, |entity| {
        entity.spawnflags = bit | spawned;
        entity.model = if mission3 {
            format!("progs/end{}.mdl", index + 1)
        } else {
            format!("progs/mg1_rune{}.mdl", index + 1)
        };
        entity.fields.insert(
            String::from("netname"),
            if mission3 {
                format!("$mg3_qc_rune{}", index + 1)
            } else {
                format!("$qc_mg1_pickup_rune{}", index + 1)
            },
        );
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::Toss;
        entity.touch = Some(touch);
    })?;
    set_addon_number(game, id, "style", f64::from(bit))?;
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
    schedule_action(game, id, &format!("{}:campaign:drop_item", program.as_str()), 0.2)
}

fn spawn_misc_rune_indicator(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let program = addon_program(game)?;
    let entity = require_entity(game, id)?.clone();
    let active = (entity.spawnflags & 64) != 0;
    let mut spawnflags = entity.spawnflags & !64;
    if spawnflags == 0 {
        spawnflags = 1;
    }
    let bits = [1, 2, 4, 8, 16, 32];
    let found = bits.iter().enumerate().find(|(_, bit)| (spawnflags & **bit) != 0);
    let (index, bit) = found
        .map(|(index, bit)| (index, *bit))
        .ok_or_else(|| q1_error("misc_rune_indicator has no rune model"))?;
    let use_callback = game
        .named
        .use_callback(&format!("{}:campaign:indicator_use", program.as_str()))?;
    game.update_entity(id, |entity| {
        entity.spawnflags = bit;
        entity.model = format!("progs/mg1_rune{}.mdl", index + 1);
        entity.use_callback = Some(use_callback);
    })?;
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    if (flags & bit) != 0 || active {
        schedule_action(game, id, &format!("{}:campaign:use_targets", program.as_str()), 0.2)
    } else {
        addon_alpha(game, id, 0.2)
    }
}

fn spawn_func_bossgate(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let program = addon_program(game)?;
    let entity = require_entity(game, id)?.clone();
    let inverse = (entity.spawnflags & 64) != 0;
    let mut spawnflags = entity.spawnflags & !64;
    if spawnflags == 0 {
        spawnflags = 15;
    }
    spawnflags &= MG1_ALL_SIGILS;
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    let complete = (flags & spawnflags) == spawnflags;
    if complete != inverse {
        return game.remove(id);
    }
    let use_callback = game.named.use_callback("func_wall_use")?;
    game.update_entity(id, |entity| {
        entity.spawnflags = spawnflags;
        entity.solid = Q1Solid::Bsp;
        entity.movement = Q1MoveType::Push;
        entity.use_callback = Some(use_callback);
    })?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    let entity = require_entity(game, id)?.clone();
    if !entity.target.is_empty() || !entity.killtarget.is_empty() {
        schedule_action(game, id, &format!("{}:campaign:use_targets", program.as_str()), 0.2)?;
    }
    Ok(())
}

fn spawn_info_player_start_hub(_game: &mut Q1EntityServices, _id: &ActorId) -> Result<(), Q1Error> {
    Ok(())
}

fn spawn_trigger_changelevel(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if require_entity(game, id)?.text("map").is_empty() {
        return Err(q1_error("changelevel trigger doesn't have map"));
    }
    init_trigger(game, id)?;
    if game.is_live(id) {
        let touch = game.named.touch(&format!("{}hub_touch", prefix(game)?))?;
        game.update_entity(id, |entity| entity.touch = Some(touch))?;
    }
    Ok(())
}

fn select_mg3_start(game: &mut Q1EntityServices, _force_spawn: bool) -> Result<Q1SpawnDecision, Q1Error> {
    if game.options().coop
        || game.options().deathmatch != 0
        || game
            .entities
            .values()
            .any(|entity| entity.classname == "testplayerstart")
    {
        return Ok(Q1SpawnDecision::Delegate);
    }
    let point = game
        .entities
        .iter()
        .find(|(_, entity)| entity.classname == "info_player_start")
        .map(|(id, _)| id.clone())
        .ok_or_else(|| q1_error("PutClientInServer: no info_player_start on level"))?;
    Ok(Q1SpawnDecision::Use(point))
}

fn select_hub(game: &mut Q1EntityServices, _force_spawn: bool) -> Result<Q1SpawnDecision, Q1Error> {
    if game.options().coop
        || game.options().deathmatch != 0
        || game
            .entities
            .values()
            .any(|entity| entity.classname == "testplayerstart")
    {
        return Ok(Q1SpawnDecision::Delegate);
    }
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    let pickup = mg1_last_sigil(flags);
    if pickup == 0 {
        return Ok(Q1SpawnDecision::Delegate);
    }
    let bits = [1, 2, 4, 8, 16, 32];
    let index = bits.iter().position(|bit| (pickup & *bit) != 0).unwrap_or(0);
    let point = game
        .entities
        .iter()
        .find(|(_, entity)| entity.text("netname") == format!("start_{}", index + 1))
        .map(|(id, entity)| (id.clone(), entity.classname.clone()));
    match point {
        Some((id, classname)) if classname == "info_player_start_hub" => {
            update_base(game, |state| {
                state.campaign.write_flags(mg1_clear_last_sigil(flags));
            })?;
            Ok(Q1SpawnDecision::Use(id))
        }
        _ => Ok(Q1SpawnDecision::Delegate),
    }
}

fn spawn_hub_trigger_changelevel(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    if (flags & MG1_ALL_SIGILS) != MG1_ALL_SIGILS {
        return game.remove(id);
    }
    if require_entity(game, id)?.text("map").is_empty() {
        return Err(q1_error("hub_trigger_changelevel is missing its map"));
    }
    init_trigger(game, id)?;
    if game.is_live(id) {
        let touch = game.named.touch(&format!("{}hub_touch", prefix(game)?))?;
        game.update_entity(id, |entity| entity.touch = Some(touch))?;
    }
    Ok(())
}

fn spawn_mge2m2_electrode_target(_game: &mut Q1EntityServices, _id: &ActorId) -> Result<(), Q1Error> {
    Ok(())
}

fn spawn_mge2m2_electrode_button(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_button(game, id)?;
    let touch = game.named.touch(&format!("{}electrode_touch", prefix(game)?))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))
}

fn spawn_mge2m2_rune_egg_opener(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let use_callback = game.named.use_callback(&format!("{}egg_use", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn spawn_mge2m2_rune_pickup_fixer(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let use_callback = game.named.use_callback(&format!("{}fix_pickup", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))?;
    schedule_action(game, id, &format!("{}fix_pickup", prefix(game)?), 0.8)
}

/// Register campaign addons (`registerCampaignAddons`). The base content
/// must already be registered on the game.
pub fn register_campaign_addons(context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let program = context.program();
    let prefix = format!("{}:campaign:", program.as_str());
    game.named.register(
        &format!("{prefix}gib_monster"),
        Q1CallbackHandlers {
            action: Some(gib_monster_action),
            ..Default::default()
        },
    )?;
    update_base(game, |state| {
        state.level_rules.register_intermission_rule(Q1IntermissionRule {
            id: format!("{}:campaign", program.as_str()),
            touch: Some(campaign_touch),
            begin: Some(campaign_begin),
            finale: Some(campaign_finale),
            travel: Some(campaign_travel),
        })
    })??;
    game.named.register(
        &format!("{prefix}sigil_touch"),
        Q1CallbackHandlers {
            touch: Some(sigil_touch),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}drop_item"),
        Q1CallbackHandlers {
            action: Some(drop_item_action),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}use_targets"),
        Q1CallbackHandlers {
            action: Some(use_targets_action),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}indicator_use"),
        Q1CallbackHandlers {
            use_callback: Some(indicator_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}hub_touch"),
        Q1CallbackHandlers {
            touch: Some(hub_touch),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}hub_changelevel"),
        Q1CallbackHandlers {
            action: Some(hub_changelevel_action),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}electrode_touch"),
        Q1CallbackHandlers {
            touch: Some(electrode_touch),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}egg_use"),
        Q1CallbackHandlers {
            use_callback: Some(egg_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}fix_pickup"),
        Q1CallbackHandlers {
            action: Some(fix_pickup_action),
            use_callback: Some(fix_pickup_use),
            ..Default::default()
        },
    )?;
    game.replace_spawn("item_sigil", spawn_item_sigil)?;
    game.register_spawn("misc_rune_indicator", spawn_misc_rune_indicator)?;
    game.replace_spawn("func_bossgate", spawn_func_bossgate)?;
    game.register_spawn("info_player_start_hub", spawn_info_player_start_hub)?;
    game.replace_spawn("trigger_changelevel", spawn_trigger_changelevel)?;
    if program == Q1AddonProgram::Mg3 {
        update_base(game, |state| {
            state.spawn_selector.register_selection("mg3:start", select_mg3_start)
        })??;
    } else {
        update_base(game, |state| {
            state
                .spawn_selector
                .register_selection(&format!("{}:hub", program.as_str()), select_hub)
        })??;
        game.register_spawn("hub_trigger_changelevel", spawn_hub_trigger_changelevel)?;
        game.register_spawn("mge2m2_electrode_target", spawn_mge2m2_electrode_target)?;
        game.register_spawn("mge2m2_electrode_button", spawn_mge2m2_electrode_button)?;
        game.register_spawn("mge2m2_rune_egg_opener", spawn_mge2m2_rune_egg_opener)?;
        game.register_spawn("mge2m2_rune_pickup_fixer", spawn_mge2m2_rune_pickup_fixer)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{
        add_frame_tick, attach_test_player, register_test_addons, test_addon_events, update_addons,
    };
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices, program: Q1AddonProgram) -> (Q1BaseGuard, Q1AddonContext) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(game, program);
        register_campaign_addons(&context, game).expect("campaign");
        (guard, context)
    }

    #[test]
    fn sigil_and_rune_helpers_match_donor() {
        assert_eq!(MG1_ALL_SIGILS, 31);
        assert_eq!(MG1_LAST_SIGIL_SHIFT, 6);
        assert_eq!(MG3_RUNE_MASK, 15);
        assert_eq!(BLOODY_NIGHTMARE_ACTIVE, 64);
        assert_eq!(BLOODY_NIGHTMARE_DISCOVERED, 128);
        assert_eq!(BLOODY_NIGHTMARE_NEWGAME, 256);
        assert_eq!(mg1_last_sigil((5 << 6) | 3), 5);
        assert_eq!(mg1_clear_last_sigil((5 << 6) | 3), 3);
        assert_eq!(mg3_rune_count(0b1011), 3);
        assert_eq!(mg3_rune_count(0), 0);
    }

    #[test]
    fn sigil_spawn_assigns_models() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let sigil = game.create("item_sigil", None, None).expect("sigil");
        game.update_entity(&sigil, |entity| entity.spawnflags = 2)
            .expect("flags");
        spawn_item_sigil(&mut game, &sigil).expect("spawn");
        let entity = require_entity(&game, &sigil).expect("entity");
        assert_eq!(entity.model, "progs/mg1_rune2.mdl");
        assert_eq!(entity.text("netname"), "$qc_mg1_pickup_rune2");
        assert_eq!(entity.number("style"), 2.0);
        assert_eq!(entity.solid, Q1Solid::Trigger);

        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg3);
        let sigil = game.create("item_sigil", None, None).expect("sigil");
        spawn_item_sigil(&mut game, &sigil).expect("spawn");
        let entity = require_entity(&game, &sigil).expect("entity");
        assert_eq!(entity.model, "progs/end1.mdl");
        assert_eq!(entity.text("netname"), "$mg3_qc_rune1");
        assert_eq!(entity.number("style"), 1.0);
    }

    #[test]
    fn sigil_touch_writes_flags_and_emits() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let player = attach_test_player(&mut game);
        let sigil = game.create("item_sigil", None, None).expect("sigil");
        game.update_entity(&sigil, |entity| entity.spawnflags = 4)
            .expect("flags");
        spawn_item_sigil(&mut game, &sigil).expect("spawn");
        sigil_touch(&mut game, &sigil, &player, None, None).expect("touch");
        let flags = update_base(&game, |state| state.campaign.read_flags()).expect("flags");
        assert_eq!(flags & MG1_ALL_SIGILS, 4);
        assert_eq!(mg1_last_sigil(flags), 4);
        assert_eq!(
            test_addon_events(&game).expect("events"),
            vec![Q1AddonEvent::RuneCollected {
                player: player.clone(),
                bits: 4,
                program: Q1AddonProgram::Mg1,
            }]
        );
    }

    #[test]
    fn finale_matches_episode_text() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        game.map_name = String::from("e1m7");
        assert_eq!(
            campaign_finale(&mut game, 2, "e1m8"),
            Ok(Q1FinaleDecision::Finale(Q1SourceFinale::Finale {
                text: String::from("$qc_finale_e1"),
                track: 2,
            }))
        );
        update_base(&game, |state| state.campaign.write_flags(MG1_ALL_SIGILS)).expect("flags");
        assert_eq!(
            campaign_finale(&mut game, 3, "start"),
            Ok(Q1FinaleDecision::Finale(Q1SourceFinale::Finale {
                text: String::from("$qc_mg1_endtext_all_runes"),
                track: 2,
            }))
        );
    }

    #[test]
    fn travel_handles_start_and_horde_cycle() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        assert_eq!(campaign_travel(&mut game, "start", None), Ok(true));
        assert_eq!(campaign_travel(&mut game, "e1m2", None), Ok(false));

        let mut game = test_game();
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(&mut game, Q1AddonProgram::Mg1);
        update_addons(&game, |state| {
            state.services.set_cvar("horde", "1");
            state.services.set_cvar("samelevel", "0");
        })
        .expect("cvars");
        register_campaign_addons(&context, &mut game).expect("campaign");
        game.map_name = String::from("horde2");
        assert_eq!(campaign_travel(&mut game, "horde3", None), Ok(true));
    }

    #[test]
    fn changelevel_requires_a_map() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg3);
        let trigger = game.create("trigger_changelevel", None, None).expect("trigger");
        assert!(spawn_trigger_changelevel(&mut game, &trigger).is_err());
        game.update_entity(&trigger, |entity| {
            entity.fields.insert(String::from("map"), String::from("e1m2"));
        })
        .expect("map");
        spawn_trigger_changelevel(&mut game, &trigger).expect("spawn");
        assert!(require_entity(&game, &trigger).expect("entity").touch.is_some());
    }

    #[test]
    fn bossgate_opens_with_all_sigils() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        update_base(&game, |state| state.campaign.write_flags(15)).expect("flags");
        let gate = game.create("func_bossgate", None, None).expect("gate");
        game.update_entity(&gate, |entity| entity.spawnflags = 15)
            .expect("flags");
        spawn_func_bossgate(&mut game, &gate).expect("spawn");
        assert!(game.entity_ref(&gate).is_none());
    }

    #[test]
    fn frame_tick_helper_marks_entity() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let id = game.create("info_null", None, None).expect("entity");
        add_frame_tick(&mut game, &id, "mg1:campaign:use_targets").expect("tick");
        assert_eq!(
            require_entity(&game, &id).expect("entity").text("addon.frameTick"),
            "mg1:campaign:use_targets"
        );
    }
}
