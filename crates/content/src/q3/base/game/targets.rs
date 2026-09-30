//! Quake III base/game: targets.
//!
//! Donor provenance: `src/content/q3/base/game/targets.ts`.

use crate::value::boolean;
use crate::value::obj;
use crate::value::SaveJson;
use crate::value::SaveReader;
use qa_core::math::add3;
use qa_core::math::normalize3;
use qa_core::math::scale3;
use qa_core::math::sub3;
use qa_core::math::vec3;
use qa_core::math::Vec3;
use qa_core::numeric::qvm_float_to_int;
use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_state::*;
use crate::q3::base::game::mover::*;
use crate::q3::base::game::personal_portal::*;
use crate::q3::base::game::save_module_values::*;
use crate::q3::base::game::save_state::*;
use crate::q3::base::game::spawn::*;
use crate::q3::base::game::state::*;
use crate::q3::base::game::triggers::*;
use crate::q3::base::game::use_participant::*;
use crate::q3::base::game::utilities::*;

// ---------------------------------------------------------------------------
// targets.ts: target entities (g_target.c)
// ---------------------------------------------------------------------------

/// Locations configstring base (`CS_LOCATIONS`).
pub const CS_LOCATIONS: i32 = 608;

/// Target laser trace mask (`MASK_TARGET_LASER`).
pub(crate) const MASK_TARGET_LASER: i32 = 0x1 | 0x2000000 | 0x4000000;

/// Target laser means of death (`MOD_TARGET_LASER`).
pub(crate) const MOD_TARGET_LASER: i32 = 21;

/// Broadcast server-command target.
pub const SERVER_COMMAND_BROADCAST: i32 = -1;

/// Target location state (`TargetLocationState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetLocationState {
    /// Linked.
    pub linked: bool,
    /// Head slot.
    pub head: Option<usize>,
}

impl TargetLocationState {
    /// New state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            linked: false,
            head: None,
        }
    }

    /// Capture save state.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveJson {
        obj(vec![
            ("linked", boolean(self.linked)),
            ("head", opt_slot_to_json(self.head)),
        ])
    }

    /// Restore save state.
    pub fn restore_save_state(&mut self, value: &SaveJson, pool: &dyn EntityPool) -> Result<(), Q3GameError> {
        let reader = SaveReader::at(value, "q3.locations");
        self.linked = reader.field("linked").boolean()?;
        self.head = reader
            .field("head")
            .nullable(|entry| read_module_entity(&entry, pool))?;
        Ok(())
    }

    /// Reset.
    pub fn reset(&mut self) {
        self.linked = false;
        self.head = None;
    }
}

impl Default for TargetLocationState {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn target_owned(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    if driver.pool().entity(slot).is_none() {
        return Err(failure(
            "Target entity does not belong to its entity pool or was replaced",
        ));
    }
    Ok(())
}

pub(crate) fn target_crandom(driver: &mut dyn Q3Driver) -> Result<f32, Q3GameError> {
    let value = driver.game_crandom();
    if !value.is_finite() || value < -1.0 || value > 1.0 {
        return Err(range("Game crandom() must return a value within [-1, 1]"));
    }
    Ok(value)
}

pub(crate) fn source_float_schedule(time: i32, seconds: f32) -> i32 {
    let milliseconds = seconds * 1000.0;
    qvm_float_to_int(time as f32 + milliseconds)
}

pub(crate) fn target_sound(driver: &mut dyn Q3Driver, slot: usize, sound_index: i32) {
    let origin = driver
        .pool()
        .entity(slot)
        .map(|entity| entity.r.current_origin)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    let sound = driver.pool().temp_entity(origin, Q3EntityEvent::GeneralSound);
    if let Some(sound) = driver.pool().entity_mut(sound) {
        sound.s.event_parm = sound_index;
    }
}

pub(crate) fn move_direction_for_target(driver: &mut dyn Q3Driver, slot: usize) -> Vec3 {
    let angles = driver
        .pool()
        .entity(slot)
        .map(|entity| entity.s.angles)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    let (direction, zero) = move_direction(angles);
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.s.angles = zero;
    }
    direction
}

/// `target_give` use handler.
pub fn use_target_give(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    let player = use_client(driver, &activator)?;
    let Some(player) = player else { return Ok(()) };
    let has_client = driver
        .pool()
        .entity(player)
        .is_some_and(|entity| entity.client.is_some());
    let target = driver.pool().entity(slot).and_then(|entity| entity.target.clone());
    if !has_client {
        return Ok(());
    }
    let Some(target) = target else { return Ok(()) };
    let mut current = None;
    loop {
        current = find_entity(driver.pool(), current, EntityStringField::Targetname, Some(&target));
        let Some(found) = current else { break };
        let (has_item, target_actor) = match driver.pool().entity(found) {
            Some(entity) => (entity.item.is_some(), entity.actor.clone()),
            None => continue,
        };
        if !has_item {
            continue;
        }
        let player_actor = use_actor(driver.pool(), &Participant::Entity(player))?;
        driver.touch_item(
            found,
            player,
            &TouchContact {
                self_actor: target_actor,
                other: player_actor,
                plane: None,
                surface: None,
            },
        );
        driver.pool().set_nextthink(found, 0);
        driver.world().unlink(found);
    }
    Ok(())
}

/// `target_remove_powerups` use handler.
pub fn use_target_remove_powerups(
    driver: &mut dyn Q3Driver,
    _slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    let player = use_client(driver, &activator)?;
    let Some(player) = player else { return Ok(()) };
    let Some(client) = driver.pool().entity(player).and_then(|entity| entity.client) else {
        return Ok(());
    };
    let (red, blue, neutral, length) = {
        let client = driver
            .pool()
            .client(client)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (
            client.ps.powerups.get(Q3Powerup::Redflag as usize),
            client.ps.powerups.get(Q3Powerup::Blueflag as usize),
            client.ps.powerups.get(Q3Powerup::Neutralflag as usize),
            client.ps.powerups.len(),
        )
    };
    if red != 0 {
        driver.return_flag(Q3Team::Red);
    } else if blue != 0 {
        driver.return_flag(Q3Team::Blue);
    } else if neutral != 0 {
        driver.return_flag(Q3Team::Free);
    }
    if let Some(client) = driver.pool().client_mut(client) {
        for index in 0..length {
            client.ps.powerups.set(index, 0);
        }
    }
    Ok(())
}

/// `target_delay` think handler.
pub fn think_target_delay(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    let activation = driver.pool().entity(slot).and_then(|entity| entity.activation.clone());
    driver.use_targets(slot, activation);
    Ok(())
}

/// `target_delay` use handler.
pub fn use_target_delay(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    let (wait, random) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (entity.wait, entity.random)
    };
    let variance = random * target_crandom(driver)?;
    let seconds = wait + variance;
    let nextthink = source_float_schedule(driver.combat().time(), seconds);
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.targets.useTargetDelay.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.think = think;
        entity.activation = activator.cloned();
    }
    driver.pool().set_nextthink(slot, nextthink);
    Ok(())
}

/// `target_score` use handler.
pub fn use_target_score(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    let player = use_client(driver, &activator)?;
    if let Some(player) = player {
        let (origin, count) = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
            (entity.r.current_origin, entity.count)
        };
        driver.add_score(player, origin, count);
    }
    Ok(())
}

/// `target_print` use handler.
pub fn use_target_print(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    let player = use_client(driver, &activator)?;
    let (message, spawnflags) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (entity.message.clone(), entity.spawnflags)
    };
    let command = game_format("cp \"%s\"", &[GameFormatArg::Text(message)])?;
    if player.is_some() && spawnflags & 4 != 0 {
        driver.send_server_command(player.unwrap_or(0) as i32, &command);
        return Ok(());
    }
    if spawnflags & 3 != 0 {
        if spawnflags & 1 != 0 {
            team_command(driver, Q3Team::Red, &command);
        }
        if spawnflags & 2 != 0 {
            team_command(driver, Q3Team::Blue, &command);
        }
        return Ok(());
    }
    driver.send_server_command(SERVER_COMMAND_BROADCAST, &command);
    Ok(())
}

/// `target_speaker` use handler.
pub fn use_target_speaker(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let (spawnflags, loop_sound, noise_index) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (entity.spawnflags, entity.s.loop_sound, entity.noise_index)
    };
    if spawnflags & 3 != 0 {
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.s.loop_sound = if loop_sound != 0 { 0 } else { noise_index };
        }
        return Ok(());
    }
    if spawnflags & 8 != 0 {
        let participant = require_use_participant(activator.cloned())?;
        match &participant {
            Participant::Entity(native) => {
                driver
                    .pool()
                    .add_event(*native, Q3EntityEvent::GeneralSound, noise_index);
            }
            Participant::SharedActor(actor) => {
                let actor = actor.clone();
                driver.actor_event(&actor, Q3EntityEvent::GeneralSound, noise_index);
            }
        }
    } else if spawnflags & 4 != 0 {
        driver.pool().add_event(slot, Q3EntityEvent::GlobalSound, noise_index);
    } else {
        driver.pool().add_event(slot, Q3EntityEvent::GeneralSound, noise_index);
    }
    Ok(())
}

/// `target_push` use handler.
pub fn use_target_push(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    let player = use_client(driver, &activator)?;
    let Some(player) = player else { return Ok(()) };
    let Some(client) = driver.pool().entity(player).and_then(|entity| entity.client) else {
        return Ok(());
    };
    let (pm_type, flight) = {
        let client = driver
            .pool()
            .client(client)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (client.ps.pm_type, client.ps.powerups.get(Q3Powerup::Flight as usize))
    };
    if pm_type != Q3MoveType::Normal as i32 || flight != 0 {
        return Ok(());
    }
    let origin2 = driver
        .pool()
        .entity(slot)
        .map(|entity| entity.s.origin2)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    if let Some(client) = driver.pool().client_mut(client) {
        client.ps.velocity = origin2;
    }
    let time = driver.combat().time();
    let debounce = driver
        .pool()
        .entity(player)
        .map(|entity| entity.fly_sound_debounce_time)
        .unwrap_or(0);
    if debounce < time {
        if let Some(entity) = driver.pool().entity_mut(player) {
            entity.fly_sound_debounce_time = time.wrapping_add(1500);
        }
        let noise = driver.pool().entity(slot).map(|entity| entity.noise_index).unwrap_or(0);
        target_sound(driver, player, noise);
    }
    Ok(())
}

pub(crate) fn laser_think(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    let enemy = driver.pool().entity(slot).and_then(|entity| entity.enemy);
    if let Some(enemy) = enemy {
        let (origin, mins, maxs) = {
            let entity = driver
                .pool()
                .entity(enemy)
                .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
            (entity.s.origin, entity.r.mins, entity.r.maxs)
        };
        let self_origin = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.s.origin)
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
        let point = add3(add3(origin, scale3(mins, 0.5)), scale3(maxs, 0.5));
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.movedir = normalize3(sub3(point, self_origin));
        }
    }
    let (origin, movedir, actor, damage, activation) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (
            entity.s.origin,
            entity.movedir,
            entity.actor.clone(),
            entity.damage,
            entity.activation.clone(),
        )
    };
    let end = add3(origin, scale3(movedir, 2048.0));
    let trace = driver.spatial().trace_actor(&Q3TraceQuery {
        start: origin,
        end,
        shape: Q3TraceShape::Point,
        pass_actor: Some(actor),
        mask: MASK_TARGET_LASER,
    });
    if let Q3TraceHit::Actor(hit) = &trace.hit {
        let target = driver.participant(hit);
        if !matches!(target, Participant::Entity(0)) {
            let host = Participant::Entity(slot);
            let mut direction = movedir;
            driver.combat().damage(
                &target,
                Some(&host),
                activation.as_ref(),
                Some(&mut direction),
                Some(trace.end),
                damage,
                DamageFlags::NO_KNOCKBACK,
                MOD_TARGET_LASER,
            );
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.movedir = direction;
            }
        }
    }
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.s.origin2 = trace.end;
    }
    driver.world().link(slot);
    let nextthink = driver.combat().time().wrapping_add(100);
    driver.pool().set_nextthink(slot, nextthink);
    Ok(())
}

pub(crate) fn laser_on(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    let missing = driver
        .pool()
        .entity(slot)
        .is_some_and(|entity| entity.activation.is_none());
    if missing {
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.activation = Some(Participant::Entity(slot));
        }
    }
    laser_think(driver, slot)
}

pub(crate) fn laser_off(driver: &mut dyn Q3Driver, slot: usize) {
    driver.world().unlink(slot);
    driver.pool().set_nextthink(slot, 0);
}

/// `target_laser` use handler.
pub fn use_target_laser(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.activation = activator.cloned();
    }
    let nextthink = driver.pool().entity(slot).map(|entity| entity.nextthink).unwrap_or(0);
    if nextthink > 0 {
        laser_off(driver, slot);
        Ok(())
    } else {
        laser_on(driver, slot)
    }
}

pub(crate) fn start_target_laser(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.s.e_type = Q3EntityType::Beam as i32;
    }
    let target = driver.pool().entity(slot).and_then(|entity| entity.target.clone());
    if let Some(target) = target {
        let found = find_entity(driver.pool(), None, EntityStringField::Targetname, Some(&target));
        if found.is_none() {
            let (classname, origin) = {
                let entity = driver
                    .pool()
                    .entity(slot)
                    .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
                (entity.classname_value().map(str::to_string), entity.s.origin)
            };
            let at = driver.scratch().vtos(origin)?.read_string();
            driver.warn(&game_format(
                "%s at %s: %s is a bad target\n",
                &[
                    GameFormatArg::Text(classname),
                    GameFormatArg::Text(Some(at)),
                    GameFormatArg::Text(Some(target)),
                ],
            )?);
        }
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.enemy = found;
        }
    } else {
        move_direction_for_target(driver, slot);
    }
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.startTargetLaser.use"))?;
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.targets.startTargetLaser.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
        entity.think = think;
        if entity.damage == 0 {
            entity.damage = 1;
        }
    }
    let spawnflags = driver.pool().entity(slot).map(|entity| entity.spawnflags).unwrap_or(0);
    if spawnflags & 1 != 0 {
        laser_on(driver, slot)
    } else {
        laser_off(driver, slot);
        Ok(())
    }
}

/// `target_teleporter` use handler.
pub fn use_target_teleporter(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    let player = use_client(driver, &activator)?;
    let Some(player) = player else { return Ok(()) };
    if driver
        .pool()
        .entity(player)
        .is_some_and(|entity| entity.client.is_none())
    {
        return Ok(());
    }
    let target = driver.pool().entity(slot).and_then(|entity| entity.target.clone());
    let destination = pick_target(driver, target.as_deref())?;
    let Some(destination) = destination else {
        driver.warn("Couldn't find teleporter destination\n");
        return Ok(());
    };
    let (origin, angles) = {
        let entity = driver
            .pool()
            .entity(destination)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (entity.s.origin, entity.s.angles)
    };
    driver.teleport_player(player, origin, angles);
    Ok(())
}

/// `target_kill` use handler.
pub fn use_target_kill(
    driver: &mut dyn Q3Driver,
    _slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let activator = require_use_participant(activator.cloned())?;
    driver.combat().damage(
        &activator,
        None,
        None,
        None,
        None,
        100_000,
        DamageFlags::NO_PROTECTION,
        MOD_TELEFRAG,
    );
    Ok(())
}

/// `target_relay` use handler.
pub fn use_target_relay(
    driver: &mut dyn Q3Driver,
    slot: usize,
    _other: Option<&Participant>,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    let spawnflags = driver.pool().entity(slot).map(|entity| entity.spawnflags).unwrap_or(0);
    if spawnflags & 3 != 0 && activator.is_none() {
        return Err(failure("Team-filtered target_relay requires an activator"));
    }
    let player = match activator {
        None => None,
        Some(activator) => {
            let required = require_use_participant(Some(activator.clone()))?;
            use_client(driver, &required)?
        }
    };
    let team = match player {
        Some(player) => driver
            .pool()
            .entity(player)
            .and_then(|entity| entity.client)
            .and_then(|client| driver.pool().client(client).map(|client| client.sess.session_team)),
        None => None,
    };
    if spawnflags & 1 != 0 && player.is_some() && team.is_some() && team != Some(Q3Team::Red as i32) {
        return Ok(());
    }
    if spawnflags & 2 != 0 && player.is_some() && team.is_some() && team != Some(Q3Team::Blue as i32) {
        return Ok(());
    }
    if spawnflags & 4 != 0 {
        let target = driver.pool().entity(slot).and_then(|entity| entity.target.clone());
        let selected = pick_target(driver, target.as_deref())?;
        if let Some(selected) = selected {
            let use_callback = driver
                .pool()
                .entity(selected)
                .and_then(|entity| entity.use_callback.clone());
            if let Some(use_callback) = use_callback {
                let other = Participant::Entity(slot);
                use_callback(driver, selected, Some(&other), activator);
            }
        }
        return Ok(());
    }
    driver.use_targets(slot, activator.cloned());
    Ok(())
}

/// Link target locations (`linkTargetLocations`).
pub fn link_target_locations(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
) -> Result<(), Q3GameError> {
    if locations.borrow().linked {
        return Ok(());
    }
    locations.borrow_mut().linked = true;
    locations.borrow_mut().head = None;
    driver.set_configstring(CS_LOCATIONS, "unknown");
    let mut number = 1i32;
    let count = driver.pool().num_entities();
    for index in 0..count {
        let (classname, message) = match driver.pool().entity(index) {
            Some(entity) => (entity.classname_value().map(str::to_string), entity.message.clone()),
            None => {
                return Err(failure(
                    "Target entity does not belong to its entity pool or was replaced",
                ))
            }
        };
        let Some(classname) = classname else { continue };
        if ascii_fold(&classname) != b"target_location" {
            continue;
        }
        if let Some(entity) = driver.pool().entity_mut(index) {
            entity.health = number;
        }
        driver.set_configstring(CS_LOCATIONS + number, &message.unwrap_or_default());
        number += 1;
        let head = locations.borrow().head;
        if let Some(entity) = driver.pool().entity_mut(index) {
            entity.next_train = head;
        }
        locations.borrow_mut().head = Some(index);
    }
    Ok(())
}

pub(crate) fn speaker_sound_path(noise: &str) -> Result<String, Q3GameError> {
    if noise.contains(".wav") {
        game_format_sized("%s", &[GameFormatArg::Text(Some(noise.to_string()))], 64)
    } else {
        game_format_sized("%s.wav", &[GameFormatArg::Text(Some(noise.to_string()))], 64)
    }
}

/// `target_give` spawn handler.
pub fn spawn_target_give(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetGive.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_remove_powerups` spawn handler.
pub fn spawn_target_remove_powerups(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetRemovePowerups.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_delay` spawn handler.
pub fn spawn_target_delay(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
    variables: &SpawnVariables,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let delay = variables.float("delay", "0")?;
    let wait = if delay.present {
        delay.value
    } else {
        variables.float("wait", "1")?.value
    };
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetDelay.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.wait = if wait == 0.0 { 1.0 } else { wait };
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_score` spawn handler.
pub fn spawn_target_score(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetScore.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        if entity.count == 0 {
            entity.count = 1;
        }
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_print` spawn handler.
pub fn spawn_target_print(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetPrint.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_speaker` spawn handler.
pub fn spawn_target_speaker(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
    variables: &SpawnVariables,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let noise = variables.string("noise", "NOSOUND");
    if !noise.present {
        let origin = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.s.origin)
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
        let at = driver.scratch().vtos(origin)?.read_string();
        let message = game_format(
            "target_speaker without a noise key at %s",
            &[GameFormatArg::Text(Some(at))],
        )?;
        return Err(failure(message));
    }
    if noise.value.starts_with('*') {
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.spawnflags |= 8;
        }
    }
    let index = driver.sound_index(&speaker_sound_path(&noise.value)?);
    let (wait, random, spawnflags) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (entity.wait, entity.random, entity.spawnflags)
    };
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetSpeaker.use"))?;
    {
        let entity = driver
            .pool()
            .entity_mut(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        entity.noise_index = index;
        entity.s.e_type = Q3EntityType::Speaker as i32;
        entity.s.event_parm = index;
        entity.s.frame = qvm_float_to_int(wait * 10.0);
        entity.s.client_num = qvm_float_to_int(random * 10.0);
        if spawnflags & 1 != 0 {
            entity.s.loop_sound = index;
        }
        entity.use_callback = use_callback;
        if spawnflags & 4 != 0 {
            entity.r.sv_flags |= ServerEntityFlags::BROADCAST;
        }
        entity.s.pos.base = entity.s.origin;
    }
    driver.world().link(slot);
    Ok(())
}

/// `target_push` spawn handler.
pub fn spawn_target_push(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    if driver.pool().entity(slot).is_some_and(|entity| entity.speed == 0.0) {
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.speed = 1000.0;
        }
    }
    let moved = move_direction_for_target(driver, slot);
    let (speed, spawnflags, target, origin) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (entity.speed, entity.spawnflags, entity.target.clone(), entity.s.origin)
    };
    let noise = driver.sound_index(if spawnflags & 1 != 0 {
        "sound/world/jumppad.wav"
    } else {
        "sound/misc/windfly.wav"
    });
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.targets.spawnTargetPush.think"))?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetPush.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.s.origin2 = scale3(moved, speed);
        entity.noise_index = noise;
        entity.use_callback = use_callback;
        if target.is_some() {
            entity.r.absmin_override = Some(origin);
            entity.r.absmax_override = Some(origin);
            entity.think = think;
        }
    }
    if target.is_some() {
        let nextthink = driver.combat().time().wrapping_add(100);
        driver.pool().set_nextthink(slot, nextthink);
    }
    Ok(())
}

/// `target_push` aim think handler.
pub fn think_target_push_aim(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    let origin = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        scale3(add3(entity.r.absmin(), entity.r.absmax()), 0.5)
    };
    aim_at_target(driver, slot, origin)
}

/// `target_laser` spawn handler.
pub fn spawn_target_laser(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.targets.spawnTargetLaser.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.think = think;
    }
    let nextthink = driver.combat().time().wrapping_add(100);
    driver.pool().set_nextthink(slot, nextthink);
    Ok(())
}

/// `target_teleporter` spawn handler.
pub fn spawn_target_teleporter(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let (targetname, classname, origin) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Target entity does not belong to its entity pool or was replaced"))?;
        (
            entity.targetname.clone(),
            entity.classname_value().map(str::to_string),
            entity.s.origin,
        )
    };
    if targetname.is_none() {
        let at = driver.scratch().vtos(origin)?.read_string();
        driver.warn(&game_format(
            "untargeted %s at %s\n",
            &[GameFormatArg::Text(classname), GameFormatArg::Text(Some(at))],
        )?);
    }
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetTeleporter.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_kill` spawn handler.
pub fn spawn_target_kill(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetKill.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_location` spawn handler.
pub fn spawn_target_location(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.targets.spawnTargetLocation.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.think = think;
        set_origin(entity, entity.s.origin);
    }
    let nextthink = driver.combat().time().wrapping_add(200);
    driver.pool().set_nextthink(slot, nextthink);
    Ok(())
}

/// `target_relay` spawn handler.
pub fn spawn_target_relay(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
    slot: usize,
) -> Result<(), Q3GameError> {
    bind_target_save_callbacks(driver, locations)?;
    target_owned(driver, slot)?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.targets.spawnTargetRelay.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.use_callback = use_callback;
    }
    Ok(())
}

/// `target_position` spawn handler.
pub fn spawn_target_position(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    target_owned(driver, slot)?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        set_origin(entity, entity.s.origin);
    }
    Ok(())
}

/// Spawn table entries (`targetSpawnHandlers`).
pub fn target_spawn_handlers(locations: &Rc<RefCell<TargetLocationState>>) -> SpawnHandlerTable {
    let mut table = SpawnHandlerTable::new();
    let give = Rc::clone(locations);
    table.insert(
        "target_give",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_give(driver, &give, slot)?;
            Ok(())
        }),
    );
    let remove = Rc::clone(locations);
    table.insert(
        "target_remove_powerups",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_remove_powerups(driver, &remove, slot)?;
            Ok(())
        }),
    );
    let delay = Rc::clone(locations);
    table.insert(
        "target_delay",
        Rc::new(move |driver, _services, slot, variables| {
            spawn_target_delay(driver, &delay, slot, variables)?;
            Ok(())
        }),
    );
    let score = Rc::clone(locations);
    table.insert(
        "target_score",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_score(driver, &score, slot)?;
            Ok(())
        }),
    );
    let print = Rc::clone(locations);
    table.insert(
        "target_print",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_print(driver, &print, slot)?;
            Ok(())
        }),
    );
    let speaker = Rc::clone(locations);
    table.insert(
        "target_speaker",
        Rc::new(move |driver, _services, slot, variables| {
            spawn_target_speaker(driver, &speaker, slot, variables)?;
            Ok(())
        }),
    );
    let laser = Rc::clone(locations);
    table.insert(
        "target_laser",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_laser(driver, &laser, slot)?;
            Ok(())
        }),
    );
    let teleporter = Rc::clone(locations);
    table.insert(
        "target_teleporter",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_teleporter(driver, &teleporter, slot)?;
            Ok(())
        }),
    );
    let relay = Rc::clone(locations);
    table.insert(
        "target_relay",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_relay(driver, &relay, slot)?;
            Ok(())
        }),
    );
    table.insert(
        "target_position",
        Rc::new(|driver, _services, slot, _variables| {
            spawn_target_position(driver, slot)?;
            Ok(())
        }),
    );
    let push = Rc::clone(locations);
    table.insert(
        "target_push",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_push(driver, &push, slot)?;
            Ok(())
        }),
    );
    let kill = Rc::clone(locations);
    table.insert(
        "target_kill",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_kill(driver, &kill, slot)?;
            Ok(())
        }),
    );
    let location = Rc::clone(locations);
    table.insert(
        "target_location",
        Rc::new(move |driver, _services, slot, _variables| {
            spawn_target_location(driver, &location, slot)?;
            Ok(())
        }),
    );
    table
}

/// Bind target save callbacks (`bindTargetSaveCallbacks`).
pub fn bind_target_save_callbacks(
    driver: &mut dyn Q3Driver,
    locations: &Rc<RefCell<TargetLocationState>>,
) -> Result<(), Q3GameError> {
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetGive.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_give(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetRemovePowerups.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_remove_powerups(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.targets.useTargetDelay.think",
        Rc::new(|driver, slot| {
            or_panic(think_target_delay(driver, slot));
        }),
    )?;
    let delay_locations = Rc::clone(locations);
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetDelay.use",
        Rc::new(move |driver, slot, other, activator| {
            or_panic(use_target_delay(driver, &delay_locations, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetScore.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_score(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetPrint.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_print(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetSpeaker.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_speaker(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.targets.spawnTargetPush.think",
        Rc::new(|driver, slot| {
            or_panic(think_target_push_aim(driver, slot));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetPush.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_push(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.startTargetLaser.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_laser(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.targets.startTargetLaser.think",
        Rc::new(|driver, slot| {
            or_panic(laser_think(driver, slot));
        }),
    )?;
    let start_locations = Rc::clone(locations);
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.targets.spawnTargetLaser.think",
        Rc::new(move |driver, slot| {
            or_panic(start_target_laser(driver, &start_locations, slot));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetTeleporter.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_teleporter(driver, slot, other, activator));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetKill.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_kill(driver, slot, other, activator));
        }),
    )?;
    let link_locations = Rc::clone(locations);
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.targets.spawnTargetLocation.think",
        Rc::new(move |driver, _slot| {
            or_panic(link_target_locations(driver, &link_locations));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.targets.spawnTargetRelay.use",
        Rc::new(|driver, slot, other, activator| {
            or_panic(use_target_relay(driver, slot, other, activator));
        }),
    )?;
    Ok(())
}
