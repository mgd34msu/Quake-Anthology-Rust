//! Quake III base/game: triggers.
//!
//! Donor provenance: `src/content/q3/base/game/triggers.ts`.

use qa_core::math::add3;
use qa_core::math::dot3;
use qa_core::math::scale3;
use qa_core::math::sub3;
use qa_core::math::vec3;
use qa_core::math::Vec3;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::combat::DamageFlags;
use crate::q3::base::game::format::{game_format, GameFormatArgument};
use crate::q3::base::game::mover::*;
use crate::q3::base::game::spawn::*;
use crate::q3::base::game::state::*;
use crate::q3::base::game::state::{failure, range, touch_jump_pad, Q3Driver, Q3GameError};
use crate::q3::base::game::use_participant::*;
use crate::q3::base::game::utilities::*;
use crate::q3::base::shared::definitions::{EntityEvent, EntityType, MoveType, Team};
use crate::q3::base::shared::entity_shared::ServerEntityFlags;

// ---------------------------------------------------------------------------
// triggers.ts: trigger entities (g_trigger.c, BG_TouchJumpPad)
// ---------------------------------------------------------------------------

/// Frame time (`FRAMETIME`).
pub(crate) const FRAMETIME: i32 = 100;

/// Trigger contents (`CONTENTS_TRIGGER`; renamed to avoid the portal constant).
pub(crate) const TRIGGER_CONTENTS: i32 = 0x40000000;

/// Trigger-hurt means of death (`MOD_TRIGGER_HURT`).
pub(crate) const MOD_TRIGGER_HURT: i32 = 22;

pub(crate) fn trigger_owned(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    if driver.pool().entity(slot).is_none() {
        return Err(failure(
            "Trigger entity does not belong to its entity pool or was replaced",
        ));
    }
    Ok(())
}

pub(crate) fn source_float_to_int(value: f32) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        value.trunc() as i32
    } else {
        -2_147_483_648
    }
}

pub(crate) fn source_schedule(time: i32, wait: f32, random: f32, crandom: f32) -> i32 {
    let seconds = wait + random * crandom;
    let milliseconds = 1000.0 * seconds;
    source_float_to_int(time as f32 + milliseconds)
}

pub(crate) fn checked_crandom(driver: &mut dyn Q3Driver) -> Result<f32, Q3GameError> {
    let value = driver.game_crandom();
    if !value.is_finite() || value < -1.0 || value > 1.0 {
        return Err(range("Game crandom() must return a value within [-1, 1]"));
    }
    Ok(value)
}

/// Shared AimAtTarget math (`aimAtTarget`).
pub fn aim_at_target(driver: &mut dyn Q3Driver, slot: usize, origin: Vec3) -> Result<(), Q3GameError> {
    let target = driver.pool().entity(slot).and_then(|entity| entity.target.clone());
    let found = pick_target(driver, target.as_deref())?;
    let Some(found) = found else {
        driver.pool().free_entity(slot);
        return Ok(());
    };
    let target_origin = driver
        .pool()
        .entity(found)
        .map(|entity| entity.s.origin)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    let height = target_origin.z - origin.z;
    let gravity = driver.gravity();
    let time = (height / (0.5 * gravity)).sqrt();
    if time == 0.0 {
        driver.pool().free_entity(slot);
        return Ok(());
    }
    let offset = sub3(target_origin, origin);
    let horizontal = vec3(offset.x, offset.y, 0.0);
    let distance = dot3(horizontal, horizontal).sqrt();
    let direction = if distance == 0.0 {
        horizontal
    } else {
        scale3(horizontal, 1.0 / distance)
    };
    let forward = distance / time;
    let velocity = scale3(direction, forward);
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.s.origin2 = vec3(velocity.x, velocity.y, time * gravity);
    }
    Ok(())
}

pub(crate) fn init_trigger(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    let angles = driver
        .pool()
        .entity(slot)
        .map(|entity| entity.s.angles)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    if angles.x != 0.0 || angles.y != 0.0 || angles.z != 0.0 {
        let (direction, zero) = move_direction(angles);
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.movedir = direction;
            entity.s.angles = zero;
        }
    }
    let model = driver.pool().entity(slot).and_then(|entity| entity.model.clone());
    driver.set_brush_model(slot, model.as_deref());
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.r.contents = TRIGGER_CONTENTS;
        entity.r.sv_flags = ServerEntityFlags::Noclient as i32;
    }
    Ok(())
}

pub(crate) fn multi_wait(driver: &mut dyn Q3Driver, slot: usize) {
    driver.pool().set_nextthink(slot, 0);
}

pub(crate) fn multi_trigger(
    driver: &mut dyn Q3Driver,
    slot: usize,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    if let Some(activator) = activator {
        require_use_participant(Some(activator.clone()))?;
    }
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.activation = activator.cloned();
    }
    let nextthink = driver.pool().entity(slot).map(|entity| entity.nextthink).unwrap_or(0);
    if nextthink != 0 {
        return Ok(());
    }
    let Some(activator) = activator else {
        return Err(failure("trigger_multiple requires an activator"));
    };
    let player = use_client(driver, activator)?;
    let team = match player {
        Some(player) => driver
            .pool()
            .entity(player)
            .and_then(|entity| entity.client)
            .and_then(|client| driver.pool().client(client).map(|client| client.sess.session_team)),
        None => None,
    };
    let (spawnflags, wait, random) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Trigger entity does not belong to its entity pool or was replaced"))?;
        (entity.spawnflags, entity.wait, entity.random)
    };
    if team.is_some() {
        if spawnflags & 1 != 0 && team != Some(Team::TeamRed as i32) {
            return Ok(());
        }
        if spawnflags & 2 != 0 && team != Some(Team::TeamBlue as i32) {
            return Ok(());
        }
    }
    driver.use_targets(slot, Some(activator.clone()));
    if wait > 0.0 {
        let think = driver
            .pool()
            .callbacks()
            .think
            .resolve(Some("q3.base.game.triggers.multiTrigger.think"))?;
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.think = think;
        }
        let nextthink = source_schedule(driver.combat().time(), wait, random, checked_crandom(driver)?);
        driver.pool().set_nextthink(slot, nextthink);
    } else {
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.touch = None;
        }
        let think = driver
            .pool()
            .callbacks()
            .think
            .resolve(Some("q3.base.game.triggers.multiTrigger.free"))?;
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.think = think;
        }
        let nextthink = driver.combat().time().wrapping_add(FRAMETIME);
        driver.pool().set_nextthink(slot, nextthink);
    }
    Ok(())
}

/// `trigger_multiple` spawn handler.
pub fn spawn_trigger_multiple(
    driver: &mut dyn Q3Driver,
    slot: usize,
    variables: &SpawnVariables,
) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    trigger_owned(driver, slot)?;
    let wait = variables.float("wait", "0.5")?.value;
    let mut random = variables.float("random", "0")?.value;
    if random >= wait && wait >= 0.0 {
        random = wait - FRAMETIME as f32;
        driver.warn("trigger_multiple has random >= wait\n");
    }
    let touch = driver
        .pool()
        .callbacks()
        .touch
        .resolve(Some("q3.base.game.triggers.spawnTriggerMultiple.touch"))?;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.triggers.spawnTriggerMultiple.use"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.wait = wait;
        entity.random = random;
        entity.touch = touch;
        entity.use_callback = use_callback;
    }
    init_trigger(driver, slot)?;
    driver.world().link(slot);
    Ok(())
}

/// `trigger_always` spawn handler.
pub fn spawn_trigger_always(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    trigger_owned(driver, slot)?;
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.triggers.spawnTriggerAlways.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.think = think;
    }
    let nextthink = driver.combat().time().wrapping_add(300);
    driver.pool().set_nextthink(slot, nextthink);
    Ok(())
}

/// `trigger_push` spawn handler.
pub fn spawn_trigger_push(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    trigger_owned(driver, slot)?;
    init_trigger(driver, slot)?;
    driver.sound_index("sound/world/jumppad.wav");
    let touch = driver
        .pool()
        .callbacks()
        .touch
        .resolve(Some("q3.base.game.triggers.spawnTriggerPush.touch"))?;
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.triggers.spawnTriggerPush.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.r.sv_flags &= !(ServerEntityFlags::Noclient as i32);
        entity.s.e_type = EntityType::EtPushTrigger as i32;
        entity.touch = touch;
        entity.think = think;
    }
    let nextthink = driver.combat().time().wrapping_add(FRAMETIME);
    driver.pool().set_nextthink(slot, nextthink);
    driver.world().link(slot);
    Ok(())
}

/// `trigger_teleport` spawn handler.
pub fn spawn_trigger_teleport(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    trigger_owned(driver, slot)?;
    init_trigger(driver, slot)?;
    driver.sound_index("sound/world/jumppad.wav");
    let touch = driver
        .pool()
        .callbacks()
        .touch
        .resolve(Some("q3.base.game.triggers.spawnTriggerTeleport.touch"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        if entity.spawnflags & 1 != 0 {
            entity.r.sv_flags |= ServerEntityFlags::Noclient as i32;
        } else {
            entity.r.sv_flags &= !(ServerEntityFlags::Noclient as i32);
        }
        entity.s.e_type = EntityType::EtTeleportTrigger as i32;
        entity.touch = touch;
    }
    driver.world().link(slot);
    Ok(())
}

pub(crate) fn sound_at(driver: &mut dyn Q3Driver, participant: &Participant, sound: i32) {
    let origin = match participant {
        Participant::Entity(slot) => driver.pool().entity(*slot).map(|entity| entity.r.current_origin),
        Participant::SharedActor(actor) => driver.actor_origin(actor),
    };
    let Some(origin) = origin else { return };
    let event = driver.pool().temp_entity(origin, EntityEvent::EvGeneralSound);
    if let Some(event) = driver.pool().entity_mut(event) {
        event.s.event_parm = sound;
    }
}

/// `trigger_hurt` spawn handler.
pub fn spawn_trigger_hurt(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    trigger_owned(driver, slot)?;
    init_trigger(driver, slot)?;
    let noise = driver.sound_index("sound/world/electro.wav");
    let touch = driver
        .pool()
        .callbacks()
        .touch
        .resolve(Some("q3.base.game.triggers.spawnTriggerHurt.touch"))?;
    let use_callback = if driver
        .pool()
        .entity(slot)
        .is_some_and(|entity| entity.spawnflags & 2 != 0)
    {
        driver
            .pool()
            .callbacks()
            .use_callbacks
            .resolve(Some("q3.base.game.triggers.spawnTriggerHurt.use"))?
    } else {
        None
    };
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.noise_index = noise;
        entity.touch = touch;
        if entity.damage == 0 {
            entity.damage = 5;
        }
        entity.r.contents = TRIGGER_CONTENTS;
        if use_callback.is_some() {
            entity.use_callback = use_callback;
        }
    }
    if driver
        .pool()
        .entity(slot)
        .is_some_and(|entity| entity.spawnflags & 1 == 0)
    {
        driver.world().link(slot);
    }
    Ok(())
}

pub(crate) fn timer_think(driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
    let (activation, wait, random) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Trigger entity does not belong to its entity pool or was replaced"))?;
        (entity.activation.clone(), entity.wait, entity.random)
    };
    driver.use_targets(slot, activation);
    let nextthink = source_schedule(driver.combat().time(), wait, random, checked_crandom(driver)?);
    driver.pool().set_nextthink(slot, nextthink);
    Ok(())
}

/// `func_timer` spawn handler.
pub fn spawn_func_timer(driver: &mut dyn Q3Driver, slot: usize, variables: &SpawnVariables) -> Result<(), Q3GameError> {
    bind_trigger_save_callbacks(driver)?;
    trigger_owned(driver, slot)?;
    let mut random = variables.float("random", "1")?.value;
    let wait = variables.float("wait", "1")?.value;
    let use_callback = driver
        .pool()
        .callbacks()
        .use_callbacks
        .resolve(Some("q3.base.game.triggers.spawnFuncTimer.use"))?;
    let think = driver
        .pool()
        .callbacks()
        .think
        .resolve(Some("q3.base.game.triggers.spawnFuncTimer.think"))?;
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.random = random;
        entity.wait = wait;
        entity.use_callback = use_callback;
        entity.think = think;
    }
    if random >= wait {
        random = wait - FRAMETIME as f32;
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.random = random;
        }
        let origin = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.s.origin)
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
        let at = driver.scratch().vtos(origin)?.read_string();
        driver.warn(&game_format(
            "func_timer at %s has random >= wait\n",
            &[GameFormatArgument::Text(at)],
        ));
    }
    if driver
        .pool()
        .entity(slot)
        .is_some_and(|entity| entity.spawnflags & 1 != 0)
    {
        let nextthink = driver.combat().time().wrapping_add(FRAMETIME);
        driver.pool().set_nextthink(slot, nextthink);
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.activation = Some(Participant::Entity(slot));
        }
    }
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.r.sv_flags = ServerEntityFlags::Noclient as i32;
    }
    Ok(())
}

pub(crate) fn touch_jump_pad_trigger(driver: &mut dyn Q3Driver, slot: usize, other: usize) -> Result<(), Q3GameError> {
    let Some(client) = driver.pool().entity(other).and_then(|entity| entity.client) else {
        return Ok(());
    };
    let pad = driver
        .pool()
        .entity(slot)
        .map(|entity| entity.s.clone())
        .unwrap_or_default();
    if let Some(client) = driver.pool().client_mut(client) {
        touch_jump_pad(&mut client.ps, &pad);
    }
    Ok(())
}

pub(crate) fn touch_teleport(driver: &mut dyn Q3Driver, slot: usize, other: usize) -> Result<(), Q3GameError> {
    let Some(client) = driver.pool().entity(other).and_then(|entity| entity.client) else {
        return Ok(());
    };
    let (pm_type, team) = {
        let client = driver
            .pool()
            .client(client)
            .ok_or_else(|| failure("Trigger entity does not belong to its entity pool or was replaced"))?;
        (client.ps.pm_type, client.sess.session_team)
    };
    let (spawnflags, target) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Trigger entity does not belong to its entity pool or was replaced"))?;
        (entity.spawnflags, entity.target.clone())
    };
    if pm_type == MoveType::PmDead as i32 {
        return Ok(());
    }
    if spawnflags & 1 != 0 && team != Team::TeamSpectator as i32 {
        return Ok(());
    }
    let destination = pick_target(driver, target.as_deref())?;
    let Some(destination) = destination else {
        driver.warn("Couldn't find teleporter destination\n");
        return Ok(());
    };
    let (origin, angles) = {
        let entity = driver
            .pool()
            .entity(destination)
            .ok_or_else(|| failure("Trigger entity does not belong to its entity pool or was replaced"))?;
        (entity.s.origin, entity.s.angles)
    };
    driver.teleport_player(other, origin, angles);
    Ok(())
}

pub(crate) fn touch_hurt(driver: &mut dyn Q3Driver, slot: usize, other: &Participant) -> Result<(), Q3GameError> {
    let damageable = match other {
        Participant::Entity(other) => driver.pool().entity(*other).is_some_and(|entity| entity.takedamage),
        Participant::SharedActor(actor) => driver
            .combat()
            .actor_combat_state(actor)
            .is_some_and(|state| state.can_take_damage),
    };
    let time = driver.combat().time();
    let timestamp = driver.pool().entity(slot).map(|entity| entity.timestamp).unwrap_or(0);
    if !damageable || timestamp > time {
        return Ok(());
    }
    let (spawnflags, noise_index, damage) = {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Trigger entity does not belong to its entity pool or was replaced"))?;
        (entity.spawnflags, entity.noise_index, entity.damage)
    };
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.timestamp = time.wrapping_add(if spawnflags & 16 != 0 { 1000 } else { FRAMETIME });
    }
    if spawnflags & 4 == 0 {
        sound_at(driver, other, noise_index);
    }
    let flags = if spawnflags & 8 != 0 {
        DamageFlags::NO_PROTECTION
    } else {
        0
    };
    let host = Participant::Entity(slot);
    driver.combat().damage(
        other,
        Some(&host),
        Some(&host),
        None,
        None,
        damage,
        flags,
        MOD_TRIGGER_HURT,
    );
    Ok(())
}

/// Spawn table entries (`triggerSpawnHandlers`).
#[must_use]
pub fn trigger_spawn_handlers() -> SpawnHandlerTable {
    let mut table = SpawnHandlerTable::new();
    table.insert(
        "trigger_multiple",
        Rc::new(|driver, _services, slot, variables| {
            spawn_trigger_multiple(driver, slot, variables)?;
            Ok(())
        }),
    );
    table.insert(
        "trigger_always",
        Rc::new(|driver, _services, slot, _variables| {
            spawn_trigger_always(driver, slot)?;
            Ok(())
        }),
    );
    table.insert(
        "trigger_push",
        Rc::new(|driver, _services, slot, _variables| {
            spawn_trigger_push(driver, slot)?;
            Ok(())
        }),
    );
    table.insert(
        "trigger_teleport",
        Rc::new(|driver, _services, slot, _variables| {
            spawn_trigger_teleport(driver, slot)?;
            Ok(())
        }),
    );
    table.insert(
        "trigger_hurt",
        Rc::new(|driver, _services, slot, _variables| {
            spawn_trigger_hurt(driver, slot)?;
            Ok(())
        }),
    );
    table.insert(
        "func_timer",
        Rc::new(|driver, _services, slot, variables| {
            spawn_func_timer(driver, slot, variables)?;
            Ok(())
        }),
    );
    table
}

/// Bind trigger save callbacks (`bindTriggerSaveCallbacks`).
pub fn bind_trigger_save_callbacks(driver: &mut dyn Q3Driver) -> Result<(), Q3GameError> {
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.triggers.multiTrigger.think",
        Rc::new(|driver, slot| {
            multi_wait(driver, slot);
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.triggers.multiTrigger.free",
        Rc::new(|driver, slot| {
            driver.pool().free_entity(slot);
        }),
    )?;
    driver.pool().callbacks_mut().touch.intern(
        "q3.base.game.triggers.spawnTriggerMultiple.touch",
        Rc::new(|driver, slot, other, _contact| {
            if let Participant::Entity(other) = other {
                let has_client = driver
                    .pool()
                    .entity(*other)
                    .is_some_and(|entity| entity.client.is_some());
                if has_client {
                    or_panic(multi_trigger(driver, slot, Some(&Participant::Entity(*other))));
                }
            }
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.triggers.spawnTriggerMultiple.use",
        Rc::new(|driver, slot, _other, activator| {
            or_panic(multi_trigger(driver, slot, activator));
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.triggers.spawnTriggerAlways.think",
        Rc::new(|driver, slot| {
            let activator = Participant::Entity(slot);
            driver.use_targets(slot, Some(activator));
            driver.pool().free_entity(slot);
        }),
    )?;
    driver.pool().callbacks_mut().touch.intern(
        "q3.base.game.triggers.spawnTriggerPush.touch",
        Rc::new(|driver, slot, other, _contact| {
            if let Participant::Entity(other) = other {
                let has_client = driver
                    .pool()
                    .entity(*other)
                    .is_some_and(|entity| entity.client.is_some());
                if has_client {
                    or_panic(touch_jump_pad_trigger(driver, slot, *other));
                }
            }
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.triggers.spawnTriggerPush.think",
        Rc::new(|driver, slot| {
            let origin = driver
                .pool()
                .entity(slot)
                .map(|entity| scale3(add3(entity.r.absmin(), entity.r.absmax()), 0.5))
                .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
            or_panic(aim_at_target(driver, slot, origin));
        }),
    )?;
    driver.pool().callbacks_mut().touch.intern(
        "q3.base.game.triggers.spawnTriggerTeleport.touch",
        Rc::new(|driver, slot, other, _contact| {
            if let Participant::Entity(other) = other {
                or_panic(touch_teleport(driver, slot, *other));
            }
        }),
    )?;
    driver.pool().callbacks_mut().touch.intern(
        "q3.base.game.triggers.spawnTriggerHurt.touch",
        Rc::new(|driver, slot, other, _contact| {
            or_panic(touch_hurt(driver, slot, other));
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.triggers.spawnTriggerHurt.use",
        Rc::new(|driver, slot, _other, _activator| {
            let linked = driver.pool().entity(slot).is_some_and(|entity| entity.r.linked);
            if linked {
                driver.world().unlink(slot);
            } else {
                driver.world().link(slot);
            }
        }),
    )?;
    driver.pool().callbacks_mut().use_callbacks.intern(
        "q3.base.game.triggers.spawnFuncTimer.use",
        Rc::new(|driver, slot, _other, activator| {
            or_panic(use_func_timer(driver, slot, activator));
        }),
    )?;
    driver.pool().callbacks_mut().think.intern(
        "q3.base.game.triggers.spawnFuncTimer.think",
        Rc::new(|driver, slot| {
            or_panic(timer_think(driver, slot));
        }),
    )?;
    Ok(())
}

pub(crate) fn use_func_timer(
    driver: &mut dyn Q3Driver,
    slot: usize,
    activator: Option<&Participant>,
) -> Result<(), Q3GameError> {
    if let Some(activator) = activator {
        require_use_participant(Some(activator.clone()))?;
    }
    if let Some(entity) = driver.pool().entity_mut(slot) {
        entity.activation = activator.cloned();
    }
    let nextthink = driver.pool().entity(slot).map(|entity| entity.nextthink).unwrap_or(0);
    if nextthink != 0 {
        driver.pool().set_nextthink(slot, 0);
        Ok(())
    } else {
        timer_think(driver, slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::q3::base::game::state::test_support::*;

    use qa_core::math::vec3;

    use crate::q3::base::shared::definitions::Product;

    #[test]
    fn triggers_aim_fire_and_schedule() {
        let owner = test_owner();
        let mut driver = StubDriver::new(&owner, Product::Baseq3);
        bind_trigger_save_callbacks(&mut driver).unwrap();
        let table = trigger_spawn_handlers();
        assert!(table.get("trigger_multiple").is_some());
        assert!(table.get("func_timer").is_some());
        let pad = driver.pool.spawn_entity().unwrap();
        let dest = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[pad].target = Some("dest".to_string());
        driver.pool.entities[dest].targetname = Some("dest".to_string());
        driver.pool.entities[dest].s.origin = vec3(100.0, 0.0, 200.0);
        aim_at_target(&mut driver, pad, vec3(0.0, 0.0, 0.0)).unwrap();
        let velocity = driver.pool.entities[pad].s.origin2;
        assert!(velocity.x > 0.0);
        assert!(velocity.z > 0.0);
        let multi = driver.pool.spawn_entity().unwrap();
        let variables = SpawnVariables::new(vec![SpawnPair {
            key: "wait".to_string(),
            value: "2".to_string(),
        }])
        .unwrap();
        spawn_trigger_multiple(&mut driver, multi, &variables).unwrap();
        assert_eq!(driver.pool.entities[multi].wait, 2.0);
        assert!(driver.world.linked.contains(&multi));
        let player = driver.pool.spawn_entity().unwrap();
        driver.pool.entities[player].client = Some(0);
        let activator = Participant::Entity(player);
        multi_trigger(&mut driver, multi, Some(&activator)).unwrap();
        assert_eq!(driver.use_targets_calls.len(), 1);
        assert!(driver.pool.entities[multi].nextthink > 1000);
        let hurt = driver.pool.spawn_entity().unwrap();
        spawn_trigger_hurt(&mut driver, hurt).unwrap();
        assert_eq!(driver.pool.entities[hurt].damage, 5);
        driver.pool.entities[player].takedamage = true;
        touch_hurt(&mut driver, hurt, &activator).unwrap();
        assert_eq!(driver.combat.calls.len(), 1);
        assert_eq!(driver.combat.calls[0].method, MOD_TRIGGER_HURT);
        assert!(driver.pool.entities[hurt].timestamp > 1000);
        let timer = driver.pool.spawn_entity().unwrap();
        let timer_vars = SpawnVariables::new(Vec::new()).unwrap();
        spawn_func_timer(&mut driver, timer, &timer_vars).unwrap();
        use_func_timer(&mut driver, timer, Some(&activator)).unwrap();
        assert_eq!(driver.use_targets_calls.len(), 2);
    }
}
