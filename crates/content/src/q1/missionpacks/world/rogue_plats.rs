//! Rogue plats, elevators, and buttons
//! (`src/content/q1/missionpacks/world/rogue-plats.ts`).
//!
//! newplats.qc / elevatr.qc entity behavior.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity::{move_direction, Q1MoverState};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::{dot, vadd, vscale, vsub, Q1MoveType, Q1Solid, ZERO};
use crate::q1::Q1Error;

use super::common::{brush, number};

/// Move a plat up or down (`move`).
fn plat_move(game: &mut Q1EntityServices, id: &ActorId, up: bool) -> Result<(), Q1Error> {
    let noise = game.entity(id).map(|entity| entity.text("noise")).unwrap_or_default();
    game.sound_simple(id, &noise)?;
    let destination = game
        .entity(id)
        .map(|entity| if up { entity.pos1 } else { entity.pos2 })
        .unwrap_or(ZERO);
    let speed = game.entity(id).map(|entity| entity.speed).unwrap_or(0.0);
    game.update_entity(id, |entity| {
        entity.state = if up { Q1MoverState::Up } else { Q1MoverState::Down };
    })?;
    let done = game
        .named
        .action(if up { "rogue:plat_top" } else { "rogue:plat_bottom" })?;
    game.calc_move(id, destination, speed, &done)
}

/// Send an elevator to its target floor (`elevatorGo`).
fn elevator_go(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let noise = game.entity(id).map(|entity| entity.text("noise")).unwrap_or_default();
    game.sound_simple(id, &noise)?;
    let time = game.time;
    game.update_entity(id, |entity| {
        entity.state = Q1MoverState::Up;
        number(entity, "elevatorLastUse", time);
    })?;
    let (pos2, height, floor, speed) = game
        .entity(id)
        .map(|entity| {
            (
                entity.pos2,
                entity.number("height"),
                entity.number("elevatorToFloor"),
                entity.speed,
            )
        })
        .unwrap_or((ZERO, 0.0, 0.0, 0.0));
    let done = game.named.action("rogue:elevator_stop")?;
    game.calc_move(
        id,
        Vec3 {
            x: pos2.x,
            y: pos2.y,
            z: (f64::from(pos2.z) + height * floor) as f32,
        },
        speed,
        &done,
    )
}

/// Fire an elevator button (`buttonFire`).
fn button_fire(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let state = game
        .entity(id)
        .map(|entity| entity.state)
        .unwrap_or(Q1MoverState::Bottom);
    if state == Q1MoverState::Up || state == Q1MoverState::Top {
        return Ok(());
    }
    let noise = game.entity(id).map(|entity| entity.text("noise")).unwrap_or_default();
    game.sound_simple(id, &noise)?;
    let (pos2, speed) = game
        .entity(id)
        .map(|entity| (entity.pos2, entity.speed))
        .unwrap_or((ZERO, 0.0));
    game.update_entity(id, |entity| entity.state = Q1MoverState::Up)?;
    let done = game.named.action("rogue:elvbutton_wait")?;
    game.calc_move(id, pos2, speed, &done)
}

/// Settle a plat at the top or bottom.
fn plat_settle(game: &mut Q1EntityServices, id: &ActorId, up: bool) -> Result<(), Q1Error> {
    let noise1 = game.entity(id).map(|entity| entity.text("noise1")).unwrap_or_default();
    game.sound_simple(id, &noise1)?;
    game.update_entity(id, |entity| {
        entity.state = if up { Q1MoverState::Top } else { Q1MoverState::Bottom };
    })?;
    let (spawnflags, ltime, health) = game
        .entity(id)
        .map(|entity| (entity.spawnflags, entity.number("ltime"), game.health(id)))
        .unwrap_or((0, 0.0, 0.0));
    if spawnflags & 1 != 0 && !up {
        let done = game.named.action("rogue:plat_up")?;
        return game.schedule_at(id, ltime + health, &done);
    }
    if spawnflags & 16 == 0 {
        return Ok(());
    }
    let time = game.time;
    game.update_entity(id, |entity| number(entity, "plat2LastMove", time))?;
    if game
        .entity(id)
        .map(|entity| entity.number("plat2Called"))
        .unwrap_or(0.0)
        == 1.0
    {
        game.update_entity(id, |entity| {
            number(entity, "plat2Called", 0.0);
            number(entity, "plat2LastMove", 0.0);
        })?;
        let done = game
            .named
            .action(if up { "rogue:plat_down" } else { "rogue:plat_up" })?;
        return game.schedule_at(id, ltime + 1.5, &done);
    }
    if up != (spawnflags & 8 != 0) {
        game.update_entity(id, |entity| number(entity, "plat2Called", 0.0))?;
        let delay = game.entity(id).map(|entity| entity.delay).unwrap_or(0.0);
        let done = game
            .named
            .action(if up { "rogue:plat_down" } else { "rogue:plat_up" })?;
        return game.schedule_at(id, ltime + delay, &done);
    }
    Ok(())
}

fn plat_up(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    plat_move(game, id, true)
}
fn plat_down(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    plat_move(game, id, false)
}
fn plat_top(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    plat_settle(game, id, true)
}
fn plat_bottom(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    plat_settle(game, id, false)
}

/// Crush whoever blocks a plat, reversing it.
fn plat_blocked(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId) -> Result<(), Q1Error> {
    let id_copy = id.clone();
    let other = other.clone();
    game.damage(
        &other,
        Some(&id_copy),
        Some(&id_copy),
        1.0,
        &crate::q1::foundation::entity_services::Q1DamageParams::default(),
    );
    let state = game
        .entity(id)
        .map(|entity| entity.state)
        .unwrap_or(Q1MoverState::Bottom);
    if state != Q1MoverState::Up && state != Q1MoverState::Down {
        return Err(crate::q1::q1_error("plat_new_crush: bad self.state"));
    }
    plat_move(game, id, state == Q1MoverState::Down)
}

/// Toggle a plat from a use dispatch.
fn plat_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let (spawnflags, state) = game
        .entity(id)
        .map(|entity| (entity.spawnflags, entity.state))
        .unwrap_or((0, Q1MoverState::Bottom));
    if spawnflags & 1 != 0 {
        if state == Q1MoverState::Top {
            return plat_move(game, id, false);
        }
        return Ok(());
    }
    if state == Q1MoverState::Top {
        return plat_move(game, id, false);
    }
    if state == Q1MoverState::Bottom {
        return plat_move(game, id, true);
    }
    Ok(())
}

/// Settle an elevator at its floor.
fn elevator_stop(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let floor = game
        .entity(id)
        .map(|entity| entity.number("elevatorToFloor"))
        .unwrap_or(0.0);
    let noise1 = game.entity(id).map(|entity| entity.text("noise1")).unwrap_or_default();
    game.update_entity(id, |entity| number(entity, "elevatorOnFloor", floor))?;
    game.sound_simple(id, &noise1)?;
    let time = game.time;
    game.update_entity(id, |entity| {
        entity.state = Q1MoverState::Bottom;
        number(entity, "elevatorLastUse", time);
    })
}

/// Return an elevator to its floor after being blocked.
fn elevator_blocked(game: &mut Q1EntityServices, id: &ActorId, _other: &ActorId) -> Result<(), Q1Error> {
    let floor = game
        .entity(id)
        .map(|entity| entity.number("elevatorOnFloor"))
        .unwrap_or(0.0);
    game.update_entity(id, |entity| number(entity, "elevatorToFloor", floor))?;
    elevator_go(game, id)
}

/// Pick a floor from a button press.
fn elevator_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    if game
        .entity(id)
        .map(|entity| entity.number("elevatorLastUse"))
        .unwrap_or(0.0)
        + 2.0
        > game.time
    {
        return Ok(());
    }
    let time = game.time;
    game.update_entity(id, |entity| number(entity, "elevatorLastUse", time))?;
    let direction = game
        .world
        .as_ref()
        .and_then(|world| game.entity(world))
        .map(|world| world.number("rogue:elvButnDir"))
        .unwrap_or(0.0);
    if direction == 0.0 {
        return Ok(());
    }
    let button = other
        .and_then(|other| game.entity(other).map(|_| other.clone()))
        .or_else(|| game.world.clone());
    let Some(button) = button else {
        return Ok(());
    };
    let body = game.body(id)?;
    let other_body = game.body(&button)?;
    let position = f64::from(body.origin.z) + f64::from(body.bounds.min.z + body.bounds.max.z) / 2.0;
    let button_position =
        f64::from(other_body.origin.z) + f64::from(other_body.bounds.min.z + other_body.bounds.max.z) / 2.0;
    let (height, floor, count) = game
        .entity(id)
        .map(|entity| {
            (
                entity.number("height"),
                entity.number("elevatorOnFloor"),
                entity.number("cnt"),
            )
        })
        .unwrap_or((0.0, 0.0, 0.0));
    if position > button_position {
        let floor = floor - ((position - button_position) / height).ceil();
        game.update_entity(id, |entity| number(entity, "elevatorToFloor", floor))?;
    } else if button_position - position > height {
        let floor = floor + ((button_position - position) / height).floor();
        game.update_entity(id, |entity| number(entity, "elevatorToFloor", floor))?;
    } else if direction == -1.0 && floor > 0.0 {
        game.update_entity(id, |entity| number(entity, "elevatorToFloor", floor - 1.0))?;
    } else if direction == 1.0 && floor < count - 1.0 {
        game.update_entity(id, |entity| number(entity, "elevatorToFloor", floor + 1.0))?;
    } else {
        return Ok(());
    }
    elevator_go(game, id)
}

/// Enable a disabled plat2.
fn plat2_enable(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        number(entity, "plat2Disabled", 0.0);
        entity.use_callback = None;
    })
}

/// Steer a plat2 from its center trigger.
fn plat2_center(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) || game.health(other) <= 0.0 {
        return Ok(());
    }
    let plat = game.entity(id).and_then(|trigger| trigger.owner.clone());
    let body = game.host.bodies.read(other);
    let (Some(plat), Some(body)) = (plat, body) else {
        return Ok(());
    };
    let (last_move, disabled) = game
        .entity(&plat)
        .map(|plat| (plat.number("plat2LastMove"), plat.number("plat2Disabled")))
        .unwrap_or((0.0, 0.0));
    if last_move + 2.0 > game.time || disabled != 0.0 {
        return Ok(());
    }
    let pending = game.entity(&plat).map(|plat| plat.number("plat2GoTo")).unwrap_or(0.0);
    if pending > 0.0 {
        if game.entity(&plat).map(|plat| plat.number("plat2GoTime")).unwrap_or(0.0) < game.time {
            plat_move(game, &plat, pending == 1.0)?;
            game.update_entity(&plat, |plat| number(plat, "plat2GoTo", 0.0))?;
        }
        return Ok(());
    }
    let state = game
        .entity(&plat)
        .map(|entity| entity.state)
        .unwrap_or(Q1MoverState::Bottom);
    if state == Q1MoverState::Up || state == Q1MoverState::Down {
        return Ok(());
    }
    let platform = game.body(&plat)?;
    let center = f64::from(platform.origin.z) + f64::from(platform.bounds.min.z + platform.bounds.max.z) / 2.0;
    let height = game.entity(&plat).map(|plat| plat.number("height")).unwrap_or(0.0);
    let same_level = if state == Q1MoverState::Top {
        center <= f64::from(body.origin.z)
    } else {
        f64::from(body.origin.z) - center <= height
    };
    let time = game.time;
    game.update_entity(&plat, |plat| {
        number(plat, "plat2Called", if same_level { 0.0 } else { 1.0 });
        number(plat, "plat2GoTime", time + if same_level { 0.5 } else { 0.1 });
        number(plat, "plat2GoTo", if state == Q1MoverState::Bottom { 1.0 } else { 2.0 });
    })
}

/// Spawn a `func_new_plat` in toggle, elevator, or plat2 mode.
fn spawn_new_plat(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    brush(game, id)?;
    game.update_entity(id, |entity| {
        if entity.speed == 0.0 {
            entity.speed = 150.0;
        }
        if entity.sounds == 0 {
            entity.sounds = 2;
        }
    })?;
    let sounds = game.entity(id).map(|entity| entity.sounds).unwrap_or(0);
    if sounds == 1 || sounds == 2 {
        game.update_entity(id, |entity| {
            entity.fields.insert(
                "noise".to_string(),
                if sounds == 1 {
                    "plats/plat1.wav"
                } else {
                    "plats/medplat1.wav"
                }
                .to_string(),
            );
            entity.fields.insert(
                "noise1".to_string(),
                if sounds == 1 {
                    "plats/plat2.wav"
                } else {
                    "plats/medplat2.wav"
                }
                .to_string(),
            );
        })?;
    }
    let body = game.body(id)?;
    let stored = game.entity(id).map(|entity| entity.number("height")).unwrap_or(0.0);
    let mut negative = stored < 0.0;
    let mut height = stored.abs();
    if height == 0.0 {
        negative = true;
        height = f64::from(body.bounds.max.z - body.bounds.min.z) - 8.0;
    }
    game.update_entity(id, |entity| {
        number(entity, "height", height);
        entity.pos1 = body.origin;
        entity.pos2 = Vec3 {
            x: body.origin.x,
            y: body.origin.y,
            z: (f64::from(body.origin.z) - height) as f32,
        };
    })?;
    let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
    if spawnflags & 3 != 0 {
        let use_name = game.named.use_callback("rogue:plat_use")?;
        let blocked_name = game.named.blocked("rogue:plat_blocked")?;
        game.update_entity(id, |entity| {
            entity.use_callback = Some(use_name);
            entity.blocked = Some(blocked_name);
            entity.state = if negative {
                Q1MoverState::Bottom
            } else {
                Q1MoverState::Top
            };
        })?;
        if negative {
            let pos2 = game.entity(id).map(|entity| entity.pos2).unwrap_or(ZERO);
            game.set_origin(id, pos2)?;
        }
        if spawnflags & 1 != 0 && game.health(id) == 0.0 {
            game.set_health(id, 5.0)?;
        }
    } else if spawnflags & 4 != 0 {
        let count = game.entity(id).map(|entity| entity.number("cnt")).unwrap_or(0.0);
        let floor = if spawnflags & 8 != 0 { count - 1.0 } else { 0.0 };
        game.update_entity(id, |entity| {
            number(entity, "elevatorOnFloor", floor);
            number(entity, "elevatorToFloor", 0.0);
            number(entity, "elevatorLastUse", 0.0);
        })?;
        if spawnflags & 8 != 0 {
            game.update_entity(id, |entity| {
                entity.pos2 = Vec3 {
                    x: body.origin.x,
                    y: body.origin.y,
                    z: (f64::from(body.origin.z) - height * (count - 1.0)) as f32,
                };
            })?;
        } else {
            game.update_entity(id, |entity| {
                entity.pos1 = Vec3 {
                    x: body.origin.x,
                    y: body.origin.y,
                    z: (f64::from(body.origin.z) + height * (count - 1.0)) as f32,
                };
                entity.pos2 = body.origin;
            })?;
        }
        let use_name = game.named.use_callback("rogue:elevator_use")?;
        let blocked_name = game.named.blocked("rogue:elevator_blocked")?;
        game.update_entity(id, |entity| {
            entity.use_callback = Some(use_name);
            entity.blocked = Some(blocked_name);
        })?;
    } else if spawnflags & 16 != 0 {
        let trigger = game.create("rogue_plat2_trigger", None, None)?;
        let owner = id.clone();
        game.update_entity(&trigger, |trigger| {
            trigger.owner = Some(owner);
            trigger.solid = Q1Solid::Trigger;
            trigger.movement = Q1MoveType::None;
        })?;
        let touch_name = game.named.touch("rogue:plat2_center")?;
        game.update_entity(&trigger, |trigger| trigger.touch = Some(touch_name))?;
        let mut min = vadd(
            body.bounds.min,
            Vec3 {
                x: 25.0,
                y: 25.0,
                z: 0.0,
            },
        );
        let mut max = vsub(
            body.bounds.max,
            Vec3 {
                x: 25.0,
                y: 25.0,
                z: -8.0,
            },
        );
        min.z = (f64::from(max.z) - height - 8.0) as f32;
        if spawnflags & 1 != 0 {
            max.z = (f64::from(min.z) + 8.0) as f32;
        }
        if body.bounds.max.x - body.bounds.min.x <= 50.0 {
            min.x = (body.bounds.min.x + body.bounds.max.x) / 2.0;
            max.x = min.x + 1.0;
        }
        if body.bounds.max.y - body.bounds.min.y <= 50.0 {
            min.y = (body.bounds.min.y + body.bounds.max.y) / 2.0;
            max.y = min.y + 1.0;
        }
        game.set_bounds(&trigger, qa_core::math::Bounds { min, max })?;
        game.update_entity(id, |entity| {
            for key in ["plat2Called", "plat2LastMove", "plat2GoTo", "plat2GoTime"] {
                number(entity, key, 0.0);
            }
            if entity.delay == 0.0 {
                entity.delay = 3.0;
            }
        })?;
        let blocked_name = game.named.blocked("rogue:plat_blocked")?;
        game.update_entity(id, |entity| entity.blocked = Some(blocked_name))?;
        if negative {
            game.update_entity(id, |entity| {
                entity.state = Q1MoverState::Bottom;
                entity.spawnflags = 16;
            })?;
            let pos2 = game.entity(id).map(|entity| entity.pos2).unwrap_or(ZERO);
            game.set_origin(id, pos2)?;
        } else {
            game.update_entity(id, |entity| {
                entity.spawnflags |= 8;
                entity.state = Q1MoverState::Top;
            })?;
        }
        if !game
            .entity(id)
            .map(|entity| entity.targetname.clone())
            .unwrap_or_default()
            .is_empty()
        {
            let use_name = game.named.use_callback("rogue:plat2_enable")?;
            game.update_entity(id, |entity| {
                number(entity, "plat2Disabled", 1.0);
                entity.use_callback = Some(use_name);
            })?;
        }
    }
    Ok(())
}

/// Hold an elevator button down, then fire its targets.
fn elvbutton_wait(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if let Some(world) = game.world.clone() {
        let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
        let direction = if spawnflags & 1 != 0 { -1.0 } else { 1.0 };
        game.update_entity(&world, |world| number(world, "rogue:elvButnDir", direction))?;
    }
    game.update_entity(id, |entity| entity.state = Q1MoverState::Top)?;
    let (ltime, wait) = game
        .entity(id)
        .map(|entity| (entity.number("ltime"), entity.wait))
        .unwrap_or((0.0, 0.0));
    let done = game.named.action("rogue:elvbutton_return")?;
    game.schedule_at(id, ltime + wait, &done)?;
    let activator = game.entity(id).and_then(|entity| entity.activator.clone());
    game.use_targets(id, activator.as_ref())?;
    game.update_entity(id, |entity| entity.frame = 1)
}

/// Settle an elevator button at the bottom.
fn elvbutton_done(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.state = Q1MoverState::Bottom)
}

/// Return an elevator button to rest.
fn elvbutton_return(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.state = Q1MoverState::Down;
        entity.frame = 0;
    })?;
    if game.health(id) != 0.0 {
        game.set_damageable(id, true)?;
    }
    let (pos1, speed) = game
        .entity(id)
        .map(|entity| (entity.pos1, entity.speed))
        .unwrap_or((ZERO, 0.0));
    let done = game.named.action("rogue:elvbutton_done")?;
    game.calc_move(id, pos1, speed, &done)
}

/// Fire an elevator button from a use dispatch.
fn elvbutton_fire_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let activator = activator.cloned();
    game.update_entity(id, |entity| entity.activator = activator)?;
    button_fire(game, id)
}

/// Fire an elevator button from a touch dispatch.
fn elvbutton_fire_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) {
        return Ok(());
    }
    let other = other.clone();
    game.update_entity(id, |entity| entity.activator = Some(other))?;
    button_fire(game, id)
}

/// Fire a shootable elevator button on death.
fn elvbutton_fire_die(game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    let attacker = attacker.cloned();
    game.update_entity(id, |entity| entity.activator = attacker)?;
    let max_health = game.entity(id).map(|entity| entity.max_health).unwrap_or(0.0);
    game.set_health(id, max_health)?;
    game.set_damageable(id, false)?;
    button_fire(game, id)
}

/// Spawn a `func_elvtr_button`.
fn spawn_elvtr_button(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    const SOUNDS: [&str; 4] = [
        "buttons/airbut1.wav",
        "buttons/switch21.wav",
        "buttons/switch02.wav",
        "buttons/switch04.wav",
    ];
    let sounds = game.entity(id).map(|entity| entity.sounds).unwrap_or(0);
    if sounds >= 0 {
        if let Some(sound) = SOUNDS.get(sounds as usize) {
            game.update_entity(id, |entity| {
                entity.fields.insert("noise".to_string(), sound.to_string());
            })?;
        }
    }
    let angles = game.body(id)?.angles;
    let movedir = move_direction(angles, None);
    game.update_entity(id, |entity| entity.movedir = movedir)?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    game.update_entity(id, |entity| {
        entity.movement = Q1MoveType::Push;
        entity.solid = Q1Solid::Bsp;
    })?;
    let use_name = game.named.use_callback("rogue:elvbutton_fire")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))?;
    if game.health(id) != 0.0 {
        let max_health = game.health(id);
        let die_name = game.named.die("rogue:elvbutton_fire")?;
        game.update_entity(id, |entity| {
            entity.max_health = max_health;
            entity.die = Some(die_name);
        })?;
        game.set_damageable(id, true)?;
    } else {
        let touch_name = game.named.touch("rogue:elvbutton_fire")?;
        game.update_entity(id, |entity| entity.touch = Some(touch_name))?;
    }
    game.update_entity(id, |entity| {
        if entity.speed == 0.0 {
            entity.speed = 40.0;
        }
        if entity.wait == 0.0 {
            entity.wait = 1.0;
        }
        entity.state = Q1MoverState::Bottom;
    })?;
    let body = game.body(id)?;
    let (movedir, lip) = game
        .entity(id)
        .map(|entity| (entity.movedir, entity.number("lip")))
        .unwrap_or((ZERO, 0.0));
    let travel =
        f64::from(dot(movedir, vsub(body.bounds.max, body.bounds.min)).abs()) - if lip == 0.0 { 4.0 } else { lip };
    game.update_entity(id, |entity| {
        entity.pos1 = body.origin;
        entity.pos2 = vadd(entity.pos1, vscale(movedir, travel));
    })
}

/// Register Rogue plat entities (`registerRoguePlats`).
pub fn register_rogue_plats(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "rogue:plat_up",
        Q1CallbackHandlers {
            action: Some(plat_up),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:plat_down",
        Q1CallbackHandlers {
            action: Some(plat_down),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:plat_top",
        Q1CallbackHandlers {
            action: Some(plat_top),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:plat_bottom",
        Q1CallbackHandlers {
            action: Some(plat_bottom),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:plat_blocked",
        Q1CallbackHandlers {
            blocked: Some(plat_blocked),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:plat_use",
        Q1CallbackHandlers {
            use_callback: Some(plat_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:elevator_stop",
        Q1CallbackHandlers {
            action: Some(elevator_stop),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:elevator_blocked",
        Q1CallbackHandlers {
            blocked: Some(elevator_blocked),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:elevator_use",
        Q1CallbackHandlers {
            use_callback: Some(elevator_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:plat2_enable",
        Q1CallbackHandlers {
            use_callback: Some(plat2_enable),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:plat2_center",
        Q1CallbackHandlers {
            touch: Some(plat2_center),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_new_plat", spawn_new_plat)?;
    game.named.register(
        "rogue:elvbutton_wait",
        Q1CallbackHandlers {
            action: Some(elvbutton_wait),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:elvbutton_done",
        Q1CallbackHandlers {
            action: Some(elvbutton_done),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:elvbutton_return",
        Q1CallbackHandlers {
            action: Some(elvbutton_return),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:elvbutton_fire",
        Q1CallbackHandlers {
            use_callback: Some(elvbutton_fire_use),
            touch: Some(elvbutton_fire_touch),
            die: Some(elvbutton_fire_die),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_elvtr_button", spawn_elvtr_button)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    fn register_for_test(game: &mut Q1EntityServices) {
        register_rogue_plats(game).expect("register");
    }

    fn tall_bounds() -> qa_core::math::Bounds {
        qa_core::math::Bounds {
            min: Vec3 {
                x: -32.0,
                y: -32.0,
                z: -28.0,
            },
            max: Vec3 {
                x: 32.0,
                y: 32.0,
                z: 28.0,
            },
        }
    }

    #[test]
    fn plat_use_moves_top_plat_down() {
        let mut game = test_game();
        register_for_test(&mut game);
        let id = game.create("func_new_plat", None, None).expect("plat");
        game.set_body(
            &id,
            &BodyPatch {
                bounds: Some(tall_bounds()),
                ..Default::default()
            },
        )
        .expect("bounds");
        game.update_entity(&id, |entity| entity.spawnflags = 2).expect("flags");
        game.update_entity(&id, |entity| number(entity, "height", 64.0))
            .expect("height");
        game.spawn_entity(&id, None).expect("spawn");
        assert_eq!(game.entity(&id).expect("plat").state, Q1MoverState::Top);
        game.invoke_use(&id, "rogue:plat_use", None, None).expect("use");
        assert_eq!(game.entity(&id).expect("plat").state, Q1MoverState::Down);
        assert!(game.entity(&id).expect("plat").move_completion.is_some());
    }

    #[test]
    fn elevator_use_selects_floor_above_button() {
        let mut game = test_game();
        register_for_test(&mut game);
        let world = game.create("worldspawn", None, None).expect("world");
        game.world = Some(world.clone());
        game.update_entity(&world, |world| number(world, "rogue:elvButnDir", 1.0))
            .expect("dir");
        let id = game.create("func_new_plat", None, None).expect("elevator");
        game.set_body(
            &id,
            &BodyPatch {
                bounds: Some(tall_bounds()),
                ..Default::default()
            },
        )
        .expect("bounds");
        game.update_entity(&id, |entity| {
            entity.spawnflags = 4;
            number(entity, "cnt", 3.0);
        })
        .expect("flags");
        game.spawn_entity(&id, None).expect("spawn");
        let button = game.create("func_elvtr_button", None, None).expect("button");
        game.set_origin(
            &button,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 60.0,
            },
        )
        .expect("button origin");
        game.time = 3.0;
        game.invoke_use(&id, "rogue:elevator_use", Some(&button), None)
            .expect("use");
        assert_eq!(game.entity(&id).expect("elevator").number("elevatorToFloor"), 1.0);
    }

    #[test]
    fn elvbutton_fire_moves_and_arms_return() {
        let mut game = test_game();
        register_for_test(&mut game);
        let id = game.create("func_elvtr_button", None, None).expect("button");
        game.set_body(
            &id,
            &BodyPatch {
                bounds: Some(tall_bounds()),
                ..Default::default()
            },
        )
        .expect("bounds");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_use(&id, "rogue:elvbutton_fire", None, None).expect("fire");
        assert_eq!(game.entity(&id).expect("button").state, Q1MoverState::Up);
    }
}
