//! Hipnotic counters, keys, and triggers
//! (`src/content/q1/missionpacks/world/hipnotic-triggers.ts`).
//!
//! hipcount.qc / hiptrig.qc / hip_brk.qc entity behavior.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::contract::ItemId;
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::types::ZERO;
use crate::q1::{Q1Error, q1_error};

use super::common::{brush, later, number, trigger};

/// Stop a counter, parking retriggerable counters (`counterOff`).
fn counter_off(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let (cnt, spawnflags) = game
        .entity(id)
        .map(|entity| (entity.number("cnt"), entity.spawnflags))
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if cnt != 0.0 && spawnflags & 32 != 0 {
        return game.update_entity(id, |entity| number(entity, "aflag", 1.0));
    }
    let start = game.named.use_callback("hip:counter_start")?;
    game.update_entity(id, |entity| {
        entity.use_callback = Some(start);
        number(entity, "aflag", 0.0);
    })?;
    game.cancel(id);
    Ok(())
}

/// Advance a counter and fire its targets (`counterTick`).
fn counter_tick(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (cnt, count, spawnflags) = game
        .entity(id)
        .map(|entity| (entity.number("cnt"), entity.count, entity.spawnflags))
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let ticked = cnt + 1.0;
    let state = if spawnflags & 16 != 0 {
        (game.host.random() * count).floor() + 1.0
    } else {
        ticked
    };
    game.update_entity(id, |entity| {
        number(entity, "cnt", ticked);
        number(entity, "counter_state", state);
    })?;
    let activator = game.entity(id).and_then(|entity| entity.activator.clone());
    game.use_targets(id, activator.as_ref())?;
    if !game.is_live(id) {
        return Ok(());
    }
    let wait = game.entity(id).map(|entity| entity.wait).unwrap_or(0.0);
    later(game, id, wait, "hip:counter_tick")?;
    if spawnflags & 4 != 0 {
        counter_off(game, id, None, None)?;
    }
    if ticked >= count {
        game.update_entity(id, |entity| number(entity, "cnt", 0.0))?;
        let (aflag, spawnflags) = game
            .entity(id)
            .map(|entity| (entity.number("aflag"), entity.spawnflags))
            .unwrap_or((0.0, 0));
        if aflag != 0.0 || spawnflags & 2 == 0 {
            if spawnflags & 1 != 0 {
                return counter_off(game, id, None, None);
            }
            return game.remove(id);
        }
    }
    Ok(())
}

/// Start (or restart) a counter (`counterStart`).
fn counter_start(
    game: &mut Q1EntityServices,
    id: &ActorId,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let (spawnflags, delay) = game
        .entity(id)
        .map(|entity| (entity.spawnflags, entity.delay))
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let activator = activator.cloned();
    let stop = if spawnflags & 1 != 0 {
        Some(game.named.use_callback("hip:counter_stop")?)
    } else {
        None
    };
    game.update_entity(id, |entity| {
        entity.activator = activator;
        number(entity, "aflag", 0.0);
        entity.use_callback = stop;
        if spawnflags & 8 != 0 {
            number(entity, "cnt", 0.0);
            number(entity, "counter_state", 0.0);
        }
    })?;
    if delay != 0.0 {
        return later(game, id, delay, "hip:counter_tick");
    }
    counter_tick(game, id)
}

/// Start a counter from a use dispatch.
fn counter_start_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    counter_start(game, id, activator)
}

/// Start a counter from a scheduled action dispatch.
fn counter_start_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    counter_start(game, id, None)
}

/// Fire when the using counter reaches this count (`onCount`).
fn on_count(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let count = other
        .and_then(|other| game.entity(other))
        .map(|counter| {
            if counter.classname == "func_counter" {
                counter.number("counter_state")
            } else {
                0.0
            }
        })
        .unwrap_or(0.0);
    let wanted = game.entity(id).map(|entity| entity.count).unwrap_or(0.0);
    if count == wanted {
        return game.use_targets(id, other);
    }
    Ok(())
}

/// Consume a key and fire (`useKey`).
fn use_key(
    game: &mut Q1EntityServices,
    id: &ActorId,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let Some(activator) = activator.cloned() else {
        return Ok(());
    };
    let (attack_finished, spawnflags, message) = game
        .entity(id)
        .map(|entity| {
            (
                entity.attack_finished,
                entity.spawnflags,
                entity.message.clone(),
            )
        })
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if !game.is_player(&activator) || attack_finished > game.time {
        return Ok(());
    }
    let time = game.time;
    game.update_entity(id, |entity| entity.attack_finished = time + 2.0)?;
    let gold = spawnflags & 1 != 0;
    let key = if gold { "q1:key/gold" } else { "q1:key/silver" };
    let owned = game.host.actors.resolve_owned(&activator);
    let sound = if game.world_type == 2 {
        "base"
    } else if game.world_type == 1 {
        "rune"
    } else {
        "med"
    };
    let consumed = owned
        .as_ref()
        .is_some_and(|owned| game.host.inventory.consume(owned, &ItemId::from(key), 1.0));
    if !consumed {
        let fallback = format!(
            "$qc_need_{}_{}",
            if gold { "gold" } else { "silver" },
            if game.world_type == 2 {
                "keycard"
            } else if game.world_type == 1 {
                "runekey"
            } else {
                "key"
            }
        );
        let text = if message.is_empty() {
            fallback
        } else {
            message
        };
        game.message(Some(&activator), &text, true, Vec::new());
        return game.sound_simple(id, &format!("doors/{sound}try.wav"));
    }
    game.update_entity(id, |entity| {
        entity.touch = None;
        entity.use_callback = None;
        entity.message.clear();
    })?;
    later(game, id, 0.1, "SUB_Remove")?;
    game.sound_simple(id, &format!("doors/{sound}use.wav"))?;
    game.use_targets(id, Some(&activator))
}

/// Fire a key trigger from a use dispatch.
fn key_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    use_key(game, id, activator)
}

/// Fire a key trigger from a touch dispatch.
fn key_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    use_key(game, id, Some(other))
}

/// Hide a victim class, then remove this trigger (`hip:remove_touch`).
fn remove_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
    let player = game.is_player(other);
    let classname = game.host.classname(other);
    if player && spawnflags & 2 == 0 || classname.starts_with("monster_") && spawnflags & 1 == 0 {
        return Ok(());
    }
    if game.entity(other).is_some() {
        game.update_entity(other, |victim| {
            victim.touch = None;
            victim.model.clear();
        })?;
    }
    game.remove(id)
}

/// Apply trigger gravity to players (`hip:set_gravity`).
fn set_gravity_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) {
        return Ok(());
    }
    let gravity = game
        .entity(id)
        .map(|entity| entity.number("gravity"))
        .unwrap_or(0.0);
    game.set_gravity(other, if gravity == -1.0 { 1.0 } else { gravity })
}

/// Fire when a decoy walks through (`hip:decoy_trigger`).
fn decoy_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if game.host.classname(other) != "monster_decoy" {
        return Ok(());
    }
    game.update_entity(id, |entity| entity.touch = None)?;
    later(game, id, 0.1, "SUB_Remove")?;
    let other = other.clone();
    game.use_targets(id, Some(&other))
}

/// Push players downstream with turbulence (`hip:waterfall`).
fn waterfall_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) {
        return Ok(());
    }
    let owned = game.host.actors.resolve_owned(other);
    let body = game.host.bodies.read(other);
    let (Some(owned), Some(body)) = (owned, body) else {
        return Ok(());
    };
    let (movedir, count) = game
        .entity(id)
        .map(|entity| (entity.movedir, entity.count))
        .unwrap_or((ZERO, 0.0));
    let velocity = Vec3 {
        x: f64::from(body.velocity.x) + f64::from(movedir.x),
        y: f64::from(body.velocity.y) + f64::from(movedir.y),
        z: f64::from(body.velocity.z) + f64::from(movedir.z),
    };
    let turbulence = || game.host.random() - 0.5;
    let jitter_x = count * turbulence();
    let jitter_y = count * turbulence();
    let mut next = body.clone();
    next.velocity = Vec3 {
        x: (velocity.x + jitter_x) as f32,
        y: (velocity.y + jitter_y) as f32,
        z: velocity.z as f32,
    };
    game.host.bodies.write(&owned, &next)?;
    Ok(())
}

/// Reset threshold health on pain (`hip:threshold_pain`).
fn threshold_pain(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _attacker: Option<&ActorId>,
    _damage: f64,
) -> Result<(), Q1Error> {
    let max_health = game
        .entity(id)
        .map(|entity| entity.max_health)
        .unwrap_or(0.0);
    game.set_health(id, max_health)
}

/// Fire threshold targets on death (`hip:threshold_die`).
fn threshold_die(
    game: &mut Q1EntityServices,
    id: &ActorId,
    attacker: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let (max_health, spawnflags) = game
        .entity(id)
        .map(|entity| (entity.max_health, entity.spawnflags))
        .unwrap_or((0.0, 0));
    game.set_health(id, max_health)?;
    game.set_damageable(id, false)?;
    let attacker = attacker.cloned();
    game.use_targets(id, attacker.as_ref())?;
    game.set_damageable(id, true)?;
    if spawnflags & 1 == 0 {
        return game.remove(id);
    }
    Ok(())
}

/// Remove a breakaway wall on use.
fn breakaway_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    game.remove(id)
}

/// Spawn a `func_counter`.
fn spawn_counter(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let start = game.named.use_callback("hip:counter_start")?;
    let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
    game.update_entity(id, |entity| {
        if entity.wait == 0.0 {
            entity.wait = 1.0;
        }
        entity.count = entity.count.floor();
        if entity.count <= 0.0 {
            entity.count = 10.0;
        }
        number(entity, "cnt", 0.0);
        number(entity, "counter_state", 0.0);
        entity.use_callback = Some(start);
    })?;
    if spawnflags & 64 != 0 {
        return later(game, id, 0.1, "hip:counter_start");
    }
    Ok(())
}

/// Spawn a `func_oncount`.
fn spawn_oncount(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let use_name = game.named.use_callback("hip:oncount")?;
    game.update_entity(id, |entity| {
        entity.count = entity.count.floor();
        if entity.count <= 0.0 {
            entity.count = 1.0;
        }
        entity.use_callback = Some(use_name);
    })
}

/// Spawn a `trigger_usekey`.
fn spawn_usekey(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    trigger(game, id)?;
    let use_name = game.named.use_callback("hip:key")?;
    let touch_name = game.named.touch("hip:key")?;
    game.update_entity(id, |entity| {
        entity.use_callback = Some(use_name);
        entity.touch = Some(touch_name);
    })
}

/// Spawn a `trigger_remove`.
fn spawn_remove(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    trigger(game, id)?;
    let touch_name = game.named.touch("hip:remove_touch")?;
    game.update_entity(id, |entity| entity.touch = Some(touch_name))
}

/// Spawn a `trigger_setgravity`.
fn spawn_setgravity(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    trigger(game, id)?;
    game.update_entity(id, |entity| {
        let gravity = entity.number("gravity");
        number(
            entity,
            "gravity",
            if gravity == 0.0 {
                -1.0
            } else {
                (gravity - 1.0) / 100.0
            },
        );
    })?;
    let touch_name = game.named.touch("hip:set_gravity")?;
    game.update_entity(id, |entity| entity.touch = Some(touch_name))
}

/// Spawn a `trigger_command`.
fn spawn_command(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let use_name = game.named.use_callback("hip:oncount")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))
}

/// Spawn a `trigger_decoy_use`.
fn spawn_decoy_use(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.options().deathmatch != 0 {
        return game.remove(id);
    }
    trigger(game, id)?;
    let touch_name = game.named.touch("hip:decoy_trigger")?;
    game.update_entity(id, |entity| entity.touch = Some(touch_name))
}

/// Spawn a `trigger_waterfall`.
fn spawn_waterfall(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    trigger(game, id)?;
    game.update_entity(id, |entity| {
        if entity.count == 0.0 {
            entity.count = 100.0;
        }
        let speed = if entity.speed == 0.0 {
            50.0
        } else {
            entity.speed
        };
        entity.movedir = Vec3 {
            x: (f64::from(entity.movedir.x) * speed) as f32,
            y: (f64::from(entity.movedir.y) * speed) as f32,
            z: (f64::from(entity.movedir.z) * speed) as f32,
        };
    })?;
    let touch_name = game.named.touch("hip:waterfall")?;
    game.update_entity(id, |entity| entity.touch = Some(touch_name))
}

/// Spawn a `trigger_damagethreshold`.
fn spawn_threshold(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    brush(game, id)?;
    let pain_name = game.named.pain("hip:threshold_pain")?;
    let die_name = game.named.die("hip:threshold_die")?;
    let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
    game.update_entity(id, |entity| {
        if spawnflags & 2 != 0 {
            entity.model.clear();
        }
        if entity.max_health == 0.0 {
            entity.max_health = 60.0;
        }
        entity.pain = Some(pain_name);
        entity.die = Some(die_name);
    })?;
    let max_health = game
        .entity(id)
        .map(|entity| entity.max_health)
        .unwrap_or(0.0);
    game.set_health(id, max_health)?;
    game.set_damageable(id, true)
}

/// Spawn a `func_breakawaywall`.
fn spawn_breakaway(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    brush(game, id)?;
    let use_name = game.named.use_callback("hip:breakaway")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )
}

/// Register Hipnotic trigger entities (`registerHipnoticTriggers`).
pub fn register_hipnotic_triggers(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "hip:counter_tick",
        Q1CallbackHandlers {
            action: Some(counter_tick),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:counter_start",
        Q1CallbackHandlers {
            use_callback: Some(counter_start_use),
            action: Some(counter_start_action),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:counter_stop",
        Q1CallbackHandlers {
            use_callback: Some(counter_off),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:oncount",
        Q1CallbackHandlers {
            use_callback: Some(on_count),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_counter", spawn_counter)?;
    game.register_spawn("func_oncount", spawn_oncount)?;
    game.named.register(
        "hip:key",
        Q1CallbackHandlers {
            use_callback: Some(key_use),
            touch: Some(key_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_usekey", spawn_usekey)?;
    game.named.register(
        "hip:remove_touch",
        Q1CallbackHandlers {
            touch: Some(remove_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_remove", spawn_remove)?;
    game.named.register(
        "hip:set_gravity",
        Q1CallbackHandlers {
            touch: Some(set_gravity_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_setgravity", spawn_setgravity)?;
    game.register_spawn("trigger_command", spawn_command)?;
    game.named.register(
        "hip:decoy_trigger",
        Q1CallbackHandlers {
            touch: Some(decoy_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_decoy_use", spawn_decoy_use)?;
    game.named.register(
        "hip:waterfall",
        Q1CallbackHandlers {
            touch: Some(waterfall_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_waterfall", spawn_waterfall)?;
    game.named.register(
        "hip:threshold_pain",
        Q1CallbackHandlers {
            pain: Some(threshold_pain),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:threshold_die",
        Q1CallbackHandlers {
            die: Some(threshold_die),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_damagethreshold", spawn_threshold)?;
    game.named.register(
        "hip:breakaway",
        Q1CallbackHandlers {
            use_callback: Some(breakaway_use),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_breakawaywall", spawn_breakaway)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    #[test]
    fn counter_counts_down_then_removes() {
        let mut game = test_game();
        register_hipnotic_triggers(&mut game).expect("register");
        let id = game.create("func_counter", None, None).expect("counter");
        game.update_entity(&id, |entity| entity.count = 2.0)
            .expect("count");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_use(&id, "hip:counter_start", None, None)
            .expect("start");
        assert_eq!(game.entity(&id).expect("entity").number("cnt"), 1.0);
        assert_eq!(
            game.entity(&id).expect("entity").number("counter_state"),
            1.0
        );
        game.invoke_action(&id, "hip:counter_tick").expect("tick");
        assert!(game.entity(&id).is_none());
    }

    #[test]
    fn usekey_consumes_gold_key_and_fires() {
        fn mark_used(
            game: &mut Q1EntityServices,
            id: &ActorId,
            _: Option<&ActorId>,
            _: Option<&ActorId>,
        ) -> Result<(), Q1Error> {
            game.update_entity(id, |entity| number(entity, "used", 1.0))
        }

        let mut game = test_game();
        register_hipnotic_triggers(&mut game).expect("register");
        game.named
            .register(
                "test:mark_used",
                Q1CallbackHandlers {
                    use_callback: Some(mark_used),
                    ..Default::default()
                },
            )
            .expect("register mark");
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.host
            .inventory
            .give(&owned, &ItemId::from("q1:key/gold"), 1.0);
        let watch = player.clone();
        game.host.players = Box::new(move || vec![watch.clone()]);
        let target = game.create("target", None, None).expect("target");
        let use_name = game.named.use_callback("test:mark_used").expect("use name");
        game.update_entity(&target, |entity| {
            entity.targetname = "door".to_string();
            entity.use_callback = Some(use_name);
        })
        .expect("target");
        let key = game.create("trigger_usekey", None, None).expect("key");
        game.update_entity(&key, |entity| {
            entity.spawnflags = 1;
            entity.target = "door".to_string();
        })
        .expect("flags");
        game.spawn_entity(&key, None).expect("spawn");
        game.invoke_touch(&key, &player, None, None).expect("touch");
        assert_eq!(
            game.host
                .inventory
                .count(&player, &ItemId::from("q1:key/gold")),
            0.0
        );
        assert_eq!(game.entity(&target).expect("target").number("used"), 1.0);
    }

    #[test]
    fn waterfall_pushes_players_downstream() {
        let mut game = test_game();
        register_hipnotic_triggers(&mut game).expect("register");
        let player = game.create("player", None, None).expect("player");
        let watch = player.clone();
        game.host.players = Box::new(move || vec![watch.clone()]);
        let fall = game.create("trigger_waterfall", None, None).expect("fall");
        game.spawn_entity(&fall, None).expect("spawn");
        game.invoke_touch(&fall, &player, None, None)
            .expect("touch");
        let body = game.host.bodies.read(&player).expect("body");
        assert!(f64::from(body.velocity.x).abs() > 0.0 || f64::from(body.velocity.y).abs() > 0.0);
    }
}
