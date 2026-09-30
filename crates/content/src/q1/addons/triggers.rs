//! Q1 addon triggers (`src/content/q1/addons/triggers.ts`).
//!
//! `quakec_mg1/triggers.qc` and `quakec_mg3/mg3_triggers.qc`,
//! `mg3_sacrifice_triggers.qc`. Copyright (C) 1996-2026 id Software LLC.
//! GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::campaign::{
    mg3_rune_count, BLOODY_NIGHTMARE_ACTIVE, BLOODY_NIGHTMARE_DISCOVERED, BLOODY_NIGHTMARE_NEWGAME,
};
use crate::q1::addons::context::{
    addon_broadcast, addon_emit, addon_player_number, addon_program, addon_set_cvar, fround, init_trigger,
    removed_outside_coop, require_entity, set_addon_number, set_addon_player_number, Q1AddonContext, Q1AddonEvent,
    Q1AddonProgram,
};
use crate::q1::base::provider::update_base;
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity::Q1MoverState;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyPatch, TouchContact, TouchSurface};
use crate::q1::foundation::movers::{door_down, door_up, spawn_button};
use crate::q1::foundation::types::{overlaps, vadd, Q1Effect, Q1MoveType, Q1Powerup, Q1Solid, ZERO};
use crate::q1::{q1_error, Q1Error};

fn prefix(game: &Q1EntityServices) -> Result<String, Q1Error> {
    Ok(format!("{}:trigger:", addon_program(game)?.as_str()))
}

fn schedule_trigger(game: &mut Q1EntityServices, id: &ActorId, name: &str, delay: f64) -> Result<(), Q1Error> {
    let callback = game.named.action(&format!("{}{name}", prefix(game)?))?;
    game.schedule(id, delay, &callback)
}

fn fire_targets(game: &mut Q1EntityServices, id: &ActorId, activator: Option<&ActorId>) -> Result<(), Q1Error> {
    let activator = activator.cloned();
    game.update_entity(id, |entity| entity.activator = activator.clone())?;
    game.use_targets(id, activator.as_ref())
}

fn targets_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let activator = require_entity(game, id)?.activator.clone();
    fire_targets(game, id, activator.as_ref())
}

fn counter_message(
    game: &mut Q1EntityServices,
    id: &ActorId,
    activator: Option<&ActorId>,
    sacrifice: bool,
    force: bool,
) -> Result<(), Q1Error> {
    let Some(activator) = activator else {
        return Ok(());
    };
    let entity = require_entity(game, id)?.clone();
    if !game.is_player(activator) || (!force && (entity.spawnflags & 1) != 0) {
        return Ok(());
    }
    let count = entity.count;
    let text = if sacrifice {
        if count == 0.0 {
            String::from("$mg3_qc_sacricie_count_complete")
        } else if (1.0..=8.0).contains(&count) {
            format!("$mg3_qc_sacricie_count_{}_more", count as i32)
        } else {
            String::from("$mg3_qc_sacricie_count_more")
        }
    } else if count == 0.0 {
        String::from("$qc_sequence_completed")
    } else if count == 1.0 {
        String::from("$qc_one_more")
    } else if count == 2.0 {
        String::from("$qc_two_more")
    } else if count == 3.0 {
        String::from("$qc_three_more")
    } else {
        String::from("$qc_more_go")
    };
    addon_broadcast(game, &text);
    Ok(())
}

fn counter_reset_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let count = require_entity(game, id)?.number("cnt");
    game.update_entity(id, |entity| entity.count = count)
}

fn counter_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    activator: Option<&ActorId>,
    timed: bool,
    sacrifice: bool,
) -> Result<(), Q1Error> {
    let count = fround(require_entity(game, id)?.count - 1.0);
    game.update_entity(id, |entity| entity.count = count)?;
    if count < 0.0 {
        return Ok(());
    }
    if timed {
        schedule_trigger(game, id, "counter_reset", require_entity(game, id)?.delay)?;
    }
    counter_message(game, id, activator, sacrifice, false)?;
    if count != 0.0 {
        return Ok(());
    }
    if timed {
        game.update_entity(id, |entity| entity.delay = 0.0)?;
        game.cancel(id);
    } else if !sacrifice && (require_entity(game, id)?.spawnflags & 2) != 0 {
        let wait = require_entity(game, id)?.wait;
        game.update_entity(id, |entity| entity.count = wait)?;
    }
    let multi_use = game.named.use_handler("multi_use")?;
    multi_use(game, id, None, activator)?;
    if timed {
        game.remove(id)?;
    }
    Ok(())
}

fn counter_use_plain(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    counter_use(game, id, activator, false, false)
}

fn counter_use_timed(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    counter_use(game, id, activator, true, false)
}

fn sacrifice_counter_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    counter_use(game, id, activator, false, true)
}

fn spawn_trigger_counter_timed(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    game.update_entity(id, |entity| {
        entity.wait = -1.0;
        if entity.count == 0.0 {
            entity.count = 2.0;
        }
        if entity.delay == 0.0 {
            entity.delay = 2.0;
        }
    })?;
    let count = require_entity(game, id)?.count;
    set_addon_number(game, id, "cnt", count)?;
    let use_callback = game.named.use_callback(&format!("{}counter_timed", prefix(game)?))?;
    let think = game.named.action(&format!("{}counter_reset", prefix(game)?))?;
    game.update_entity(id, |entity| {
        entity.use_callback = Some(use_callback);
        entity.think = Some(think);
    })
}

fn spawn_trigger_counter(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    game.update_entity(id, |entity| {
        if entity.count == 0.0 {
            entity.count = 2.0;
        }
        if entity.wait == 0.0 {
            entity.wait = entity.count;
        }
    })?;
    let use_callback = game.named.use_callback(&format!("{}counter", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn repeater_tick(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let activator = require_entity(game, id)?.activator.clone();
    fire_targets(game, id, activator.as_ref())?;
    if !game.is_live(id) {
        return Ok(());
    }
    let entity = require_entity(game, id)?.clone();
    let delay = entity.wait + entity.number("pausetime") * game.host.random();
    schedule_trigger(game, id, "repeater_tick", delay)
}

fn repeater_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.activator = activator.cloned();
        entity.spawnflags ^= 1;
    })?;
    if (require_entity(game, id)?.spawnflags & 1) == 0 {
        game.cancel(id);
        return Ok(());
    }
    let entity = require_entity(game, id)?.clone();
    let delay = entity.wait + entity.number("pausetime") * game.host.random();
    schedule_trigger(game, id, "repeater_tick", delay)
}

fn spawn_trigger_repeater(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    game.update_entity(id, |entity| {
        if entity.wait == 0.0 {
            entity.wait = 1.0;
        }
    })?;
    if (require_entity(game, id)?.spawnflags & 1) != 0 {
        let entity = require_entity(game, id)?.clone();
        let delay = entity.wait + entity.number("pausetime") * game.host.random();
        schedule_trigger(game, id, "repeater_tick", delay)?;
    }
    if !game.is_live(id) {
        return Ok(());
    }
    let use_callback = game.named.use_callback(&format!("{}repeater_use", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn multitouch_empty(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.wait = 0.0)?;
    if (require_entity(game, id)?.spawnflags & 32) == 0 {
        let activator = require_entity(game, id)?.activator.clone();
        fire_targets(game, id, activator.as_ref())?;
    }
    Ok(())
}

fn multitouch_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) || game.health(other) <= 0.0 {
        return Ok(());
    }
    if require_entity(game, id)?.wait == 0.0 {
        game.update_entity(id, |entity| entity.wait = 1.0)?;
        if (require_entity(game, id)?.spawnflags & 16) == 0 {
            fire_targets(game, id, Some(other))?;
        }
    }
    if game.is_live(id) {
        schedule_trigger(game, id, "multitouch_empty", 0.2)?;
    }
    Ok(())
}

fn spawn_trigger_multitouch(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    init_trigger(game, id)?;
    if !game.is_live(id) {
        return Ok(());
    }
    game.update_entity(id, |entity| entity.wait = 0.0)?;
    let touch = game.named.touch(&format!("{}multitouch_touch", prefix(game)?))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))
}

fn explosion_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.delay = 0.0)?;
    let activator = require_entity(game, id)?.activator.clone();
    fire_targets(game, id, activator.as_ref())?;
    if !game.is_live(id) {
        return Ok(());
    }
    let entity = require_entity(game, id)?.clone();
    if (entity.spawnflags & 1) == 0 {
        game.radius_damage(id, entity.owner.as_ref(), 120.0, Some(id), None, "");
    }
    let origin = game.body(id)?.origin;
    if addon_program(game)? == Q1AddonProgram::Mg3 && (entity.spawnflags & 2) != 0 {
        addon_emit(
            game,
            Q1AddonEvent::ColoredExplosion {
                origin,
                color_start: 244,
                color_length: 3,
            },
        )?;
    } else {
        game.effect_simple(Q1Effect::Explosion, origin);
    }
    game.remove(id)
}

fn explosion_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.activator = activator.cloned())?;
    if require_entity(game, id)?.delay > 0.0 {
        let delay = require_entity(game, id)?.delay;
        schedule_trigger(game, id, "explosion", delay)
    } else {
        game.invoke_action(id, &format!("{}explosion", prefix(game)?))
    }
}

fn spawn_trigger_explosion(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    if !game.is_live(id) {
        return Ok(());
    }
    let use_callback = game.named.use_callback(&format!("{}explosion", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn change_target_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    let targets: Vec<ActorId> = game
        .entities
        .iter()
        .filter(|(_, target)| target.target == entity.target)
        .map(|(id, _)| id.clone())
        .collect();
    for target in &targets {
        game.update_entity(target, |target| target.target = entity.killtarget.clone())?;
    }
    Ok(())
}

fn spawn_trigger_changetarget(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    let entity = require_entity(game, id)?.clone();
    if entity.target.is_empty() || entity.killtarget.is_empty() {
        return Err(q1_error("trigger_changetarget requires target and killtarget"));
    }
    if !game.is_live(id) {
        return Ok(());
    }
    let use_callback = game.named.use_callback(&format!("{}change_target", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn cleanup_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let corpses: Vec<ActorId> = game
        .entities
        .iter()
        .filter(|(id, target)| target.monster.is_some() && game.health(id) <= 0.0)
        .map(|(id, _)| id.clone())
        .collect();
    for corpse in &corpses {
        game.remove(corpse)?;
    }
    game.remove(id)
}

fn spawn_trigger_cleanup_corpses(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if !game.options().coop {
        return game.remove(id);
    }
    let use_callback = game.named.use_callback(&format!("{}cleanup", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn spawn_trigger_always(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    schedule_trigger(game, id, "targets", 0.1)
}

fn rune_relay_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let required = require_entity(game, id)?.spawnflags & 15;
    if (update_base(game, |state| state.campaign.read_flags())? & required) != required {
        return Ok(());
    }
    game.update_entity(id, |entity| entity.activator = activator.cloned())?;
    schedule_trigger(game, id, "targets", 0.1)
}

fn rune_counter_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    if f64::from(mg3_rune_count(flags)) >= require_entity(game, id)?.count {
        fire_targets(game, id, activator)?;
    }
    Ok(())
}

fn bn_relay_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    let spawnflags = require_entity(game, id)?.spawnflags;
    for (yes, no, bit) in [
        (1, 8, BLOODY_NIGHTMARE_ACTIVE),
        (2, 16, BLOODY_NIGHTMARE_NEWGAME),
        (4, 32, BLOODY_NIGHTMARE_DISCOVERED),
    ] {
        if ((spawnflags & yes) != 0 && (flags & bit) == 0) || ((spawnflags & no) != 0 && (flags & bit) != 0) {
            return Ok(());
        }
    }
    fire_targets(game, id, activator)
}

fn spawn_trigger_rune_relay(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    if !game.is_live(id) {
        return Ok(());
    }
    let use_callback = game.named.use_callback(&format!("{}rune_relay", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn spawn_trigger_rune_counter(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    game.update_entity(id, |entity| {
        if entity.count == 0.0 {
            entity.count = 2.0;
        }
    })?;
    if !game.is_live(id) {
        return Ok(());
    }
    let use_callback = game.named.use_callback(&format!("{}rune_counter", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn spawn_trigger_bloodynightmare_relay(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let use_callback = game.named.use_callback(&format!("{}bn_relay", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn spawn_trigger_sacrifice_counter(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    game.update_entity(id, |entity| {
        if entity.count == 0.0 {
            entity.count = 2.0;
        }
        if entity.wait == 0.0 {
            entity.wait = entity.count;
        }
    })?;
    if !game.is_live(id) {
        return Ok(());
    }
    let use_callback = game
        .named
        .use_callback(&format!("{}sacrifice_counter", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn sacrifice_empty(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.wait = 0.0)
}

fn sacrifice_check_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) || game.health(other) <= 0.0 {
        return Ok(());
    }
    if require_entity(game, id)?.wait == 0.0 {
        game.update_entity(id, |entity| entity.wait = 1.0)?;
        let target = require_entity(game, id)?.target.clone();
        if let Some(counter) = game.find(&target).first().cloned() {
            if require_entity(game, &counter)?.count > 0.0 {
                counter_message(game, &counter, Some(other), true, true)?;
            }
        }
    }
    schedule_trigger(game, id, "sacrifice_empty", 0.2)
}

fn spawn_trigger_check_sacrifices(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    init_trigger(game, id)?;
    if !game.is_live(id) {
        return Ok(());
    }
    game.update_entity(id, |entity| entity.wait = 0.0)?;
    let touch = game.named.touch(&format!("{}sacrifice_check", prefix(game)?))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))
}

fn door_relay_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    for target in game.find(&entity.target) {
        let target_entity = require_entity(game, &target)?.clone();
        let top = matches!(target_entity.state, Q1MoverState::Top | Q1MoverState::Up);
        if (entity.spawnflags & if top { 1 } else { 2 }) != 0 {
            continue;
        }
        if target_entity.classname == "func_door" {
            if top {
                door_down(game, &target)?;
            } else {
                door_up(game, &target)?;
            }
        } else if target_entity.classname == "func_button" {
            if top {
                game.invoke_action(&target, "button_return")?;
            } else {
                game.invoke_use(&target, "button_use", None, activator)?;
            }
        }
    }
    Ok(())
}

fn door_group_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    for member in game.entities.values() {
        if member.text("category") != entity.text("category") {
            continue;
        }
        let top = matches!(member.state, Q1MoverState::Top | Q1MoverState::Up);
        let goal = member.number("goal_state");
        if (!top && goal == 0.0) || (top && goal == 1.0) {
            return Ok(());
        }
    }
    fire_targets(game, id, activator)
}

fn spawn_trigger_door_relay(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    if require_entity(game, id)?.target.is_empty() {
        game.remove(id)?;
    }
    if !game.is_live(id) {
        return Ok(());
    }
    let use_callback = game.named.use_callback(&format!("{}door_relay", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn spawn_trigger_doorgroup_relay(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    let entity = require_entity(game, id)?.clone();
    if entity.target.is_empty() || entity.text("category").is_empty() {
        game.remove(id)?;
    }
    if !game.is_live(id) {
        return Ok(());
    }
    let use_callback = game.named.use_callback(&format!("{}door_group", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn lore_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) || game.time <= addon_player_number(game, other, "lore_active")? {
        return Ok(());
    }
    set_addon_player_number(game, other, "lore_active", game.time + 0.5)?;
    let message = require_entity(game, id)?.message.clone();
    game.message_simple(Some(other), &message);
    Ok(())
}

fn spawn_trigger_lore(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    if game.map_name == "hub" {
        for bit in [1, 2, 4] {
            let number = if bit == 1 {
                1
            } else if bit == 2 {
                2
            } else {
                3
            };
            let hint = format!("$mg3_hub_rune{number}_hint");
            if (flags & bit) != 0 && require_entity(game, id)?.message == hint {
                game.update_entity(id, |entity| entity.message = format!("{hint}_complete"))?;
            }
        }
    }
    if game.map_name == "map4"
        && require_entity(game, id)?.message == "$mg3_hint_bloody_nightmare_new"
        && (flags & 448) != 0
    {
        return game.remove(id);
    }
    init_trigger(game, id)?;
    if !game.is_live(id) {
        return Ok(());
    }
    let touch = game.named.touch(&format!("{}lore", prefix(game)?))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))
}

fn music_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let entity = require_entity(game, id)?.clone();
    let track = entity.number("style") as i32;
    if entity.sounds == track {
        return Ok(());
    }
    addon_emit(
        game,
        Q1AddonEvent::Music {
            track,
            loop_track: track,
        },
    )?;
    let music: Vec<ActorId> = game
        .entities
        .iter()
        .filter(|(_, entity)| entity.classname == "trigger_music")
        .map(|(id, _)| id.clone())
        .collect();
    for id in &music {
        game.update_entity(id, |entity| entity.sounds = track)?;
    }
    Ok(())
}

fn spawn_trigger_music(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    let sounds = game
        .world
        .clone()
        .and_then(|world| game.entity_ref(&world).map(|entity| entity.sounds))
        .unwrap_or(0);
    game.update_entity(id, |entity| entity.sounds = sounds)?;
    if require_entity(game, id)?.targetname.is_empty() {
        game.remove(id)?;
    }
    if !game.is_live(id) {
        return Ok(());
    }
    if require_entity(game, id)?.number("style") == 0.0 {
        set_addon_number(game, id, "style", 3.0)?;
    }
    let use_callback = game.named.use_callback(&format!("{}music", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn heal_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let player = match game.player_ref(other).cloned() {
        Some(player) => player,
        None => return Ok(()),
    };
    let entity = require_entity(game, id)?.clone();
    if entity.attack_finished > game.time {
        return Ok(());
    }
    let health = game.health(other);
    if health > 0.0 && health < player.max_health {
        game.host
            .combat
            .set_health(&player.actor, (health + entity.damage).min(player.max_health))?;
    }
    let attack_finished = fround(game.time + entity.wait);
    game.update_entity(id, |entity| entity.attack_finished = attack_finished)
}

fn spawn_trigger_heal(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        if entity.damage == 0.0 {
            entity.damage = 1.0;
        }
        if entity.wait == 0.0 {
            entity.wait = 0.1;
        }
    })?;
    init_trigger(game, id)?;
    if !game.is_live(id) {
        return Ok(());
    }
    let touch = game.named.touch(&format!("{}heal", prefix(game)?))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))
}

fn quad_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let player = match game.player_ref(other).cloned() {
        Some(player) => player,
        None => return Ok(()),
    };
    if player.powerups.get(&Q1Powerup::Quad).copied().unwrap_or(0.0) <= game.time {
        game.sound(
            other,
            "items/damage.wav",
            crate::q1::foundation::types::Q1SoundChannel::Item,
            1.0,
            1.0,
        )?;
    }
    let expires = fround(game.time + 0.1);
    game.update_player(other, |state| {
        state.powerups.insert(Q1Powerup::Quad, expires);
    })?;
    set_addon_player_number(game, other, "super_time", 2.0)?;
    game.host.powerup(&player.actor, Q1Powerup::Quad, expires);
    game.link(id)
}

fn spawn_trigger_quad(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    init_trigger(game, id)?;
    if !game.is_live(id) {
        return Ok(());
    }
    let touch = game.named.touch(&format!("{}quad", prefix(game)?))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))
}

fn kill_monster_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let target_name = require_entity(game, id)?.target.clone();
    for target in game.find(&target_name) {
        if require_entity(game, &target)?.monster.is_some() && game.health(&target) > 0.0 {
            let inflictor = activator.cloned().unwrap_or_else(|| id.clone());
            let health = game.health(&target);
            game.damage(
                &target,
                Some(&inflictor),
                activator,
                health * 2.0,
                &Q1DamageParams::default(),
            );
        }
    }
    Ok(())
}

fn spawn_trigger_relay_killmonster(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    let entity = require_entity(game, id)?.clone();
    if entity.target.is_empty() || entity.targetname.is_empty() {
        game.remove(id)?;
    }
    if !game.is_live(id) {
        return Ok(());
    }
    let use_callback = game.named.use_callback(&format!("{}kill_monster", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn silent_teleport_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) {
        return Ok(());
    }
    let player = match game.host.actors.resolve_owned(other) {
        Some(player) => player,
        None => return Ok(()),
    };
    let body = match game.host.bodies.read(other) {
        Some(body) => body,
        None => return Ok(()),
    };
    let height = require_entity(game, id)?.number("height");
    let destination = vadd(
        body.origin,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: (height + if height < 0.0 { 1.0 } else { 0.0 }) as f32,
        },
    );
    let death = game.create("teledeath", None, None)?;
    game.update_entity(&death, |entity| {
        entity.owner = Some(other.clone());
        entity.solid = Q1Solid::Trigger;
    })?;
    game.set_body(
        &death,
        &BodyPatch {
            origin: Some(destination),
            bounds: Some(Bounds {
                min: vadd(
                    body.bounds.min,
                    Vec3 {
                        x: -1.0,
                        y: -1.0,
                        z: -1.0,
                    },
                ),
                max: vadd(body.bounds.max, Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
            }),
            ..Default::default()
        },
    )?;
    let touch = game.named.touch("tdeath_touch")?;
    game.update_entity(&death, |entity| entity.touch = Some(touch))?;
    game.link(&death)?;
    let bounds = game.body(&death)?.bounds;
    let area = Bounds {
        min: vadd(destination, bounds.min),
        max: vadd(destination, bounds.max),
    };
    let death_owned = require_entity(game, &death)?.actor.clone();
    for victim in game.host.actors.observations() {
        let state = game.host.bodies.read(&victim.id);
        if state.is_some_and(|state| {
            overlaps(
                &area,
                &Bounds {
                    min: vadd(state.origin, state.bounds.min),
                    max: vadd(state.origin, state.bounds.max),
                },
            )
        }) {
            game.fire_touch(&TouchContact {
                self_actor: death_owned.clone(),
                other: victim.id.clone(),
                plane: None,
                surface: None,
            })?;
        }
    }
    let remove = game.named.action("SUB_Remove")?;
    game.schedule(&death, 0.2, &remove)?;
    let mut updated = body.clone();
    updated.origin = destination;
    game.host.bodies.write(&player, &updated)?;
    game.host.bodies.link(&player)?;
    Ok(())
}

fn spawn_trigger_teleport_silent(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if require_entity(game, id)?.number("height") == 0.0 {
        set_addon_number(game, id, "height", -2048.0)?;
    }
    init_trigger(game, id)?;
    if !game.is_live(id) {
        return Ok(());
    }
    let touch = game.named.touch(&format!("{}silent_teleport", prefix(game)?))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))
}

fn cutscene_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) {
        return Ok(());
    }
    game.update_entity(id, |entity| entity.touch = None)?;
    let camera = game
        .entities
        .iter()
        .find(|(_, entity)| entity.classname == "info_intermission")
        .map(|(id, _)| id.clone())
        .ok_or_else(|| q1_error("trigger_cutscene has no intermission camera"))?;
    let position = game.body(&camera)?.origin;
    let angles = require_entity(game, &camera)?.vector("mangle");
    for player_id in (game.host.players)() {
        let actor = match game.host.actors.resolve_owned(&player_id) {
            Some(actor) => actor,
            None => continue,
        };
        let body = match game.host.bodies.read(&player_id) {
            Some(body) => body,
            None => continue,
        };
        let mut updated = body.clone();
        updated.origin = position;
        updated.angles = angles;
        updated.velocity = ZERO;
        game.host.bodies.write(&actor, &updated)?;
        game.host.bodies.link(&actor)?;
        if game.entity_ref(&player_id).is_some() {
            game.update_entity(&player_id, |entity| {
                entity.solid = Q1Solid::None;
                entity.movement = Q1MoveType::None;
                entity.model.clear();
            })?;
        }
    }
    addon_emit(
        game,
        Q1AddonEvent::Cutscene {
            camera: position,
            angles,
        },
    )
}

fn spawn_trigger_cutscene(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    init_trigger(game, id)?;
    if !game.is_live(id) {
        return Ok(());
    }
    let touch = game.named.touch(&format!("{}cutscene", prefix(game)?))?;
    game.update_entity(id, |entity| entity.touch = Some(touch))
}

fn spawn_func_axe_button(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let actor = require_entity(game, id)?.actor.clone();
    game.update_entity(id, |entity| entity.max_health = 1.0)?;
    game.host.combat.set_health(&actor, 1.0)?;
    spawn_button(game, id)
}

fn set_skill_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let Some(activator) = activator else {
        return Ok(());
    };
    if !game.is_player(activator) {
        return Ok(());
    }
    let message = require_entity(game, id)?.message.clone();
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    if message != "4" {
        update_base(game, |state| {
            state.campaign.write_flags(flags & !BLOODY_NIGHTMARE_ACTIVE);
        })?;
    }
    let skill = if message == "0" {
        Some(0)
    } else if message == "1" {
        Some(1)
    } else if message == "2" {
        Some(2)
    } else if message == "3" || message == "4" {
        Some(3)
    } else {
        None
    };
    let Some(skill) = skill else {
        return Ok(());
    };
    if message == "4" {
        update_base(game, |state| {
            let flags = state.campaign.read_flags();
            state.campaign.write_flags(flags | BLOODY_NIGHTMARE_ACTIVE);
        })?;
    }
    addon_broadcast(
        game,
        if message == "4" {
            "$mg3_selected_bloody_nightmare"
        } else if skill == 0 {
            "$mg3_hub_selected_easy"
        } else if skill == 1 {
            "$mg3_hub_selected_normal"
        } else if skill == 2 {
            "$mg3_hub_selected_hard"
        } else {
            "$mg3_hub_selected_nightmare"
        },
    );
    update_base(game, |state| state.campaign.set_skill(skill))?;
    addon_set_cvar(game, "skill", &skill.to_string())
}

fn spawn_trigger_relay_setskill(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let use_callback = game.named.use_callback(&format!("{}set_skill", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn health_relay_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    fire_targets(game, id, activator)
}

fn spawn_trigger_health_relay(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, true)? {
        return Ok(());
    }
    if game.health(id) == 0.0 {
        let actor = require_entity(game, id)?.actor.clone();
        game.host.combat.set_health(&actor, 0.5)?;
    }
    if !game.is_live(id) {
        return Ok(());
    }
    let use_callback = game.named.use_callback(&format!("{}health_relay", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

fn explode_repeatedly(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if require_entity(game, id)?.delay > 0.0 {
        let delay = require_entity(game, id)?.delay;
        game.update_entity(id, |entity| entity.delay = 0.0)?;
        return schedule_trigger(game, id, "explosion_repeater", delay);
    }
    let previous = require_entity(game, id)?
        .references
        .get("explosion.child")
        .cloned()
        .flatten();
    if previous.as_ref().is_some_and(|previous| {
        game.entity_ref(previous)
            .is_some_and(|entity| entity.classname == "spawned_explosion")
    }) {
        if let Some(previous) = previous {
            game.remove(&previous)?;
        }
    }
    let child = game.create("spawned_explosion", None, None)?;
    game.update_entity(&child, |entity| entity.owner = Some(id.clone()))?;
    game.update_entity(id, |entity| {
        entity
            .references
            .insert(String::from("explosion.child"), Some(child.clone()));
    })?;
    let origin = game.body(id)?.origin;
    game.set_origin(&child, origin)?;
    game.invoke_action(&child, &format!("{}explosion", prefix(game)?))?;
    let entity = require_entity(game, id)?.clone();
    if (entity.spawnflags & 4) != 0 {
        let count = entity.count - 1.0;
        game.update_entity(id, |entity| entity.count = count)?;
        if count == 0.0 {
            return game.remove(id);
        }
    }
    let entity = require_entity(game, id)?.clone();
    let delay = entity.wait + game.host.random() * entity.number("pausetime");
    schedule_trigger(game, id, "explosion_repeater", delay)
}

fn explosion_repeater_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    explode_repeatedly(game, id)
}

fn explosion_repeater_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    explode_repeatedly(game, id)
}

fn spawn_trigger_explosion_repeater(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        if entity.wait == 0.0 {
            entity.wait = 0.8;
        }
    })?;
    let use_callback = game
        .named
        .use_callback(&format!("{}explosion_repeater", prefix(game)?))?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_callback))
}

/// Register addon triggers (`registerAddonTriggers`).
pub fn register_addon_triggers(context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let prefix = format!("{}:trigger:", context.program().as_str());
    game.named.register(
        &format!("{prefix}targets"),
        Q1CallbackHandlers {
            action: Some(targets_action),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}counter_reset"),
        Q1CallbackHandlers {
            action: Some(counter_reset_action),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}counter"),
        Q1CallbackHandlers {
            use_callback: Some(counter_use_plain),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}counter_timed"),
        Q1CallbackHandlers {
            use_callback: Some(counter_use_timed),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}sacrifice_counter"),
        Q1CallbackHandlers {
            use_callback: Some(sacrifice_counter_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_counter_timed", spawn_trigger_counter_timed)?;
    game.register_spawn("trigger_counter", spawn_trigger_counter)?;
    game.named.register(
        &format!("{prefix}repeater_tick"),
        Q1CallbackHandlers {
            action: Some(repeater_tick),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}repeater_use"),
        Q1CallbackHandlers {
            use_callback: Some(repeater_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_repeater", spawn_trigger_repeater)?;
    game.named.register(
        &format!("{prefix}multitouch_empty"),
        Q1CallbackHandlers {
            action: Some(multitouch_empty),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}multitouch_touch"),
        Q1CallbackHandlers {
            touch: Some(multitouch_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_multitouch", spawn_trigger_multitouch)?;
    game.named.register(
        &format!("{prefix}explosion"),
        Q1CallbackHandlers {
            action: Some(explosion_action),
            use_callback: Some(explosion_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_explosion", spawn_trigger_explosion)?;
    game.named.register(
        &format!("{prefix}change_target"),
        Q1CallbackHandlers {
            use_callback: Some(change_target_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_changetarget", spawn_trigger_changetarget)?;
    game.named.register(
        &format!("{prefix}cleanup"),
        Q1CallbackHandlers {
            use_callback: Some(cleanup_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_cleanup_corpses", spawn_trigger_cleanup_corpses)?;
    if context.program() != Q1AddonProgram::Mg3 {
        game.register_spawn("mge2m2_cleanup_corpses", spawn_trigger_cleanup_corpses)?;
        return Ok(());
    }
    game.register_spawn("trigger_always", spawn_trigger_always)?;
    game.named.register(
        &format!("{prefix}rune_relay"),
        Q1CallbackHandlers {
            use_callback: Some(rune_relay_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}rune_counter"),
        Q1CallbackHandlers {
            use_callback: Some(rune_counter_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}bn_relay"),
        Q1CallbackHandlers {
            use_callback: Some(bn_relay_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_rune_relay", spawn_trigger_rune_relay)?;
    game.register_spawn("trigger_rune_counter", spawn_trigger_rune_counter)?;
    game.register_spawn("trigger_bloodynightmare_relay", spawn_trigger_bloodynightmare_relay)?;
    game.register_spawn("trigger_sacrifice_counter", spawn_trigger_sacrifice_counter)?;
    game.named.register(
        &format!("{prefix}sacrifice_empty"),
        Q1CallbackHandlers {
            action: Some(sacrifice_empty),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}sacrifice_check"),
        Q1CallbackHandlers {
            touch: Some(sacrifice_check_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_check_sacrifices", spawn_trigger_check_sacrifices)?;
    game.named.register(
        &format!("{prefix}door_relay"),
        Q1CallbackHandlers {
            use_callback: Some(door_relay_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{prefix}door_group"),
        Q1CallbackHandlers {
            use_callback: Some(door_group_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_door_relay", spawn_trigger_door_relay)?;
    game.register_spawn("trigger_doorgroup_relay", spawn_trigger_doorgroup_relay)?;
    game.named.register(
        &format!("{prefix}lore"),
        Q1CallbackHandlers {
            touch: Some(lore_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_lore", spawn_trigger_lore)?;
    game.named.register(
        &format!("{prefix}music"),
        Q1CallbackHandlers {
            use_callback: Some(music_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_music", spawn_trigger_music)?;
    game.named.register(
        &format!("{prefix}heal"),
        Q1CallbackHandlers {
            touch: Some(heal_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_heal", spawn_trigger_heal)?;
    game.named.register(
        &format!("{prefix}quad"),
        Q1CallbackHandlers {
            touch: Some(quad_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_quad", spawn_trigger_quad)?;
    game.named.register(
        &format!("{prefix}kill_monster"),
        Q1CallbackHandlers {
            use_callback: Some(kill_monster_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_relay_killmonster", spawn_trigger_relay_killmonster)?;
    game.named.register(
        &format!("{prefix}silent_teleport"),
        Q1CallbackHandlers {
            touch: Some(silent_teleport_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_teleport_silent", spawn_trigger_teleport_silent)?;
    game.named.register(
        &format!("{prefix}cutscene"),
        Q1CallbackHandlers {
            touch: Some(cutscene_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_cutscene", spawn_trigger_cutscene)?;
    game.register_spawn("func_axe_button", spawn_func_axe_button)?;
    game.named.register(
        &format!("{prefix}set_skill"),
        Q1CallbackHandlers {
            use_callback: Some(set_skill_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_relay_setskill", spawn_trigger_relay_setskill)?;
    game.named.register(
        &format!("{prefix}health_relay"),
        Q1CallbackHandlers {
            use_callback: Some(health_relay_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_health_relay", spawn_trigger_health_relay)?;
    game.named.register(
        &format!("{prefix}explosion_repeater"),
        Q1CallbackHandlers {
            action: Some(explosion_repeater_action),
            use_callback: Some(explosion_repeater_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_explosion_repeater", spawn_trigger_explosion_repeater)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{addon_cvar, attach_test_player, register_test_addons, test_addon_events};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices, program: Q1AddonProgram) -> (Q1BaseGuard, Q1AddonContext) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        let context = register_test_addons(game, program);
        register_addon_triggers(&context, game).expect("triggers");
        (guard, context)
    }

    #[test]
    fn counter_counts_down_and_fires() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let counter = game.create("trigger_counter", None, None).expect("counter");
        spawn_trigger_counter(&mut game, &counter).expect("spawn");
        assert_eq!(require_entity(&game, &counter).expect("entity").count, 2.0);
        let player = attach_test_player(&mut game);
        counter_use_plain(&mut game, &counter, None, Some(&player)).expect("use");
        assert_eq!(require_entity(&game, &counter).expect("entity").count, 1.0);
        counter_use_plain(&mut game, &counter, None, Some(&player)).expect("use");
        assert!(game.entity_ref(&counter).is_some());
    }

    #[test]
    fn changetarget_rewrites_targets() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg1);
        let relay = game.create("trigger_changetarget", None, None).expect("relay");
        game.update_entity(&relay, |entity| {
            entity.target = String::from("old");
            entity.killtarget = String::from("new");
        })
        .expect("targets");
        spawn_trigger_changetarget(&mut game, &relay).expect("spawn");
        let victim = game.create("info_null", None, None).expect("victim");
        game.update_entity(&victim, |entity| entity.target = String::from("old"))
            .expect("target");
        change_target_use(&mut game, &relay, None, None).expect("use");
        assert_eq!(require_entity(&game, &victim).expect("entity").target, "new");
        let invalid = game.create("trigger_changetarget", None, None).expect("invalid");
        assert!(spawn_trigger_changetarget(&mut game, &invalid).is_err());
    }

    #[test]
    fn explosion_removes_itself() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg3);
        let explosion = game.create("trigger_explosion", None, None).expect("explosion");
        spawn_trigger_explosion(&mut game, &explosion).expect("spawn");
        explosion_use(&mut game, &explosion, None, None).expect("use");
        assert!(game.entity_ref(&explosion).is_none());
    }

    #[test]
    fn rune_relays_gate_on_campaign_flags() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg3);
        update_base(&game, |state| state.campaign.write_flags(3)).expect("flags");
        let relay = game.create("trigger_rune_relay", None, None).expect("relay");
        game.update_entity(&relay, |entity| entity.spawnflags = 1)
            .expect("flags");
        spawn_trigger_rune_relay(&mut game, &relay).expect("spawn");
        rune_relay_use(&mut game, &relay, None, None).expect("use");
        assert!(require_entity(&game, &relay).expect("entity").think.is_some());
        let gated = game.create("trigger_rune_relay", None, None).expect("gated");
        game.update_entity(&gated, |entity| entity.spawnflags = 4)
            .expect("flags");
        spawn_trigger_rune_relay(&mut game, &gated).expect("spawn");
        rune_relay_use(&mut game, &gated, None, None).expect("use");
        assert!(require_entity(&game, &gated).expect("entity").think.is_none());
    }

    #[test]
    fn heal_and_quad_touch_players() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg3);
        let heal = game.create("trigger_heal", None, None).expect("heal");
        spawn_trigger_heal(&mut game, &heal).expect("spawn");
        let player = attach_test_player(&mut game);
        game.set_health(&player, 40.0).expect("wound");
        heal_touch(&mut game, &heal, &player, None, None).expect("touch");
        assert_eq!(game.health(&player), 41.0);
        let quad = game.create("trigger_quad", None, None).expect("quad");
        spawn_trigger_quad(&mut game, &quad).expect("spawn");
        quad_touch(&mut game, &quad, &player, None, None).expect("touch");
        assert!(game
            .player_ref(&player)
            .expect("player")
            .powerups
            .contains_key(&Q1Powerup::Quad));
    }

    #[test]
    fn set_skill_updates_campaign_and_cvar() {
        let mut game = test_game();
        let (_guard, _context) = setup(&mut game, Q1AddonProgram::Mg3);
        let relay = game.create("trigger_relay_setskill", None, None).expect("relay");
        game.update_entity(&relay, |entity| entity.message = String::from("2"))
            .expect("message");
        spawn_trigger_relay_setskill(&mut game, &relay).expect("spawn");
        let player = attach_test_player(&mut game);
        set_skill_use(&mut game, &relay, None, Some(&player)).expect("use");
        let campaign = update_base(&game, |state| state.campaign.read_flags()).expect("flags");
        assert_eq!(campaign & BLOODY_NIGHTMARE_ACTIVE, 0);
        assert_eq!(addon_cvar(&game, "skill"), Ok(2.0));
        assert!(test_addon_events(&game).expect("events").is_empty());
    }
}
