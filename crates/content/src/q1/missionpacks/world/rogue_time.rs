//! Rogue time machine (`src/content/q1/missionpacks/world/rogue-time.ts`).
//!
//! timemach.qc entity behavior.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{
    vadd, vectors, vscale, vsub, Q1Effect, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, POINT,
};
use crate::q1::{q1_error, Q1Error};

use super::common::{later, number};

/// Tear a chunk off the dying machine (`chunk`).
fn chunk(game: &mut Q1EntityServices, explosion: &ActorId, machine: &ActorId) -> Result<(), Q1Error> {
    let body = game.body(machine)?;
    let basis = vectors(body.angles);
    let gib = game.create("time_machine_gib", None, None)?;
    game.update_entity(&gib, |gib| {
        gib.solid = Q1Solid::None;
        gib.movement = Q1MoveType::Toss;
        gib.model = "progs/timegib.mdl".to_string();
    })?;
    game.set_body(
        &gib,
        &BodyPatch {
            origin: Some(vsub(
                vadd(body.origin, vscale(basis.forward, 84.0)),
                vscale(basis.up, 136.0),
            )),
            velocity: Some(vscale(basis.up, -50.0)),
            angles: Some(body.angles),
            ..Default::default()
        },
    )?;
    game.update_entity(&gib, |gib| {
        gib.angular_velocity = Vec3 {
            x: 300.0,
            y: 300.0,
            z: 300.0,
        };
    })?;
    game.sound(explosion, "weapons/r_exp3.wav", Q1SoundChannel::Weapon, 0.0, 1.0)?;
    let origin = game.body(&gib)?.origin;
    game.effect_simple(Q1Effect::Explosion, origin);
    game.update_entity(machine, |machine| machine.frame = 1)?;
    later(game, &gib, 5.0, "SUB_Remove")?;
    game.link(&gib)
}

/// Rattle the time machine when shot (`pain`).
/// Death entry shared with the pain handler (`die: pain`).
fn time_die(game: &mut Q1EntityServices, id: &ActorId, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    time_pain(game, id, attacker, 0.0)
}

fn time_pain(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _attacker: Option<&ActorId>,
    _damage: f64,
) -> Result<(), Q1Error> {
    let health = game.health(id);
    if health > 1100.0
        && game
            .entity(id)
            .map(|entity| entity.number("pain_finished"))
            .unwrap_or(0.0)
            > game.time
    {
        return Ok(());
    }
    if game.host.random() < 0.4 {
        let time = game.time;
        game.update_entity(id, |entity| number(entity, "pain_finished", time + 2.0))?;
        let random = game.host.random();
        let body = game.body(id)?;
        let basis = vectors(body.angles);
        let explosion = game.create("time_machine_pain", None, None)?;
        let target = game.entity(id).map(|entity| entity.target.clone()).unwrap_or_default();
        let owner = id.clone();
        game.update_entity(&explosion, |explosion| {
            explosion.owner = Some(owner);
            explosion.target = target;
        })?;
        let offset = if random < 0.33 {
            vsub(vscale(basis.forward, 80.0), vscale(basis.up, 64.0))
        } else if random < 0.66 {
            vsub(vscale(basis.right, 80.0), vscale(basis.up, 24.0))
        } else {
            vsub(
                vsub(vscale(basis.forward, 64.0), vscale(basis.up, 48.0)),
                vscale(basis.right, 48.0),
            )
        };
        game.set_origin(&explosion, vadd(body.origin, offset))?;
        let fuse = 0.2 + game.host.random() * 0.3;
        later(game, &explosion, fuse, "rogue:time_boom")?;
    }
    if health < 1000.0 {
        game.update_entity(id, |entity| {
            number(entity, "pain_finished", 0.0);
            entity.pain = None;
            entity.die = None;
        })?;
        if let Some(world) = game.world.clone() {
            game.update_entity(&world, |world| number(world, "rogue:cutscene_running", 1.0))?;
        }
    }
    Ok(())
}

/// Crash the time machine into the lava (`crashTimeMachine`).
pub fn crash_time_machine(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let machine = game
        .world
        .as_ref()
        .and_then(|world| game.entity(world))
        .and_then(|world| world.references.get("rogue:theMachine").cloned().flatten())
        .filter(|machine| game.entity(machine).is_some())
        .ok_or_else(|| q1_error("Rogue time_crash requires item_time_machine"))?;
    game.set_damageable(&machine, false)?;
    game.update_entity(&machine, |machine| {
        machine.movement = Q1MoveType::Fly;
        machine.solid = Q1Solid::None;
        machine.angular_velocity = Vec3 {
            x: 15.0,
            y: 0.0,
            z: 5.0,
        };
    })?;
    game.set_body(
        &machine,
        &BodyPatch {
            velocity: Some(Vec3 {
                x: 0.0,
                y: 0.0,
                z: -50.0,
            }),
            bounds: Some(POINT),
            ..Default::default()
        },
    )?;
    later(game, &machine, 0.1, "rogue:time_fall")?;
    game.update_entity(&machine, |machine| machine.target = "timeramp".to_string())?;
    let activator = game.entity(&machine).and_then(|entity| entity.activator.clone());
    game.use_targets(&machine, activator.as_ref())
}

/// Crash from a scheduled dispatch.
fn time_crash_action(game: &mut Q1EntityServices, _id: &ActorId) -> Result<(), Q1Error> {
    crash_time_machine(game)
}

/// Crash from a use dispatch.
fn time_crash_use(
    game: &mut Q1EntityServices,
    _id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    crash_time_machine(game)
}

/// Crash from a pain dispatch.
fn time_crash_pain(
    game: &mut Q1EntityServices,
    _id: &ActorId,
    _attacker: Option<&ActorId>,
    _damage: f64,
) -> Result<(), Q1Error> {
    crash_time_machine(game)
}

/// Crash from a death dispatch.
fn time_crash_die(game: &mut Q1EntityServices, _id: &ActorId, _attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    crash_time_machine(game)
}

/// Stop shaking after a pain explosion.
fn time_stop_shake(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let activator = game.entity(id).and_then(|entity| entity.activator.clone());
    game.use_targets(id, activator.as_ref())?;
    game.remove(id)
}

/// Detonate a pain explosion on the machine.
fn time_boom(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let activator = game.entity(id).and_then(|entity| entity.activator.clone());
    game.use_targets(id, activator.as_ref())?;
    let machine = game
        .entity(id)
        .and_then(|entity| entity.owner.clone())
        .filter(|machine| game.entity(machine).is_some())
        .ok_or_else(|| q1_error("Time machine explosion lost its machine"))?;
    let (health, frame, skin) = (
        game.health(&machine),
        game.entity(&machine).map(|entity| entity.frame).unwrap_or(0),
        game.entity(&machine).map(|entity| entity.skin).unwrap_or(0),
    );
    if health < 1250.0 && frame > 0 {
        if skin < 2 {
            game.update_entity(&machine, |machine| {
                machine.frame = 2;
                machine.skin = 2;
            })?;
        }
    } else if health < 1500.0 && frame == 0 {
        chunk(game, id, &machine)?;
        game.update_entity(&machine, |machine| {
            machine.frame = 1;
            machine.skin = 1;
        })?;
    }
    game.sound(id, "weapons/r_exp3.wav", Q1SoundChannel::Weapon, 0.0, 1.0)?;
    let origin = game.body(id)?.origin;
    if game.host.random() < 0.5 {
        game.effect_simple(Q1Effect::Explosion, origin);
    } else {
        game.host.emit(Q1Event::ColoredExplosion {
            origin,
            color_start: 244,
            color_length: 3,
        });
    }
    game.update_entity(id, |entity| {
        entity.model = "progs/s_explod.spr".to_string();
        entity.frame = 0;
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::None;
        entity.touch = None;
    })?;
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(Vec3 { x: 0.0, y: 0.0, z: 0.0 }),
            ..Default::default()
        },
    )?;
    later(game, id, 0.1, "base:explosion_frame")?;
    game.link(id)?;
    let stop = game.create("time_stop_shake", None, None)?;
    let target = game.entity(id).map(|entity| entity.target.clone()).unwrap_or_default();
    game.update_entity(&stop, |stop| stop.target = target)?;
    later(game, &stop, 0.7, "rogue:time_stop_shake")
}

/// Sink the crashing machine, splashing into lava.
fn time_fall(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let body = game.body(id)?;
    if game
        .entity(id)
        .map(|entity| entity.number("pain_finished"))
        .unwrap_or(0.0)
        == 0.0
    {
        if body.origin.z < -20.0 {
            game.effect_simple(
                Q1Effect::LavaSplash,
                vadd(
                    body.origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: -80.0,
                    },
                ),
            );
            game.update_entity(id, |entity| number(entity, "pain_finished", 1.0))?;
        }
    } else if game.host.random() < 0.3 {
        game.effect_simple(Q1Effect::Explosion, body.origin);
    }
    game.set_body(
        id,
        &BodyPatch {
            velocity: Some(Vec3 {
                x: body.velocity.x,
                y: body.velocity.y,
                z: body.velocity.z - 5.0,
            }),
            ..Default::default()
        },
    )?;
    later(game, id, 0.1, "rogue:time_fall")
}

/// Spawn an `item_time_machine`.
fn spawn_time_machine(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.options().deathmatch != 0 {
        return game.remove(id);
    }
    game.update_entity(id, |entity| {
        entity.model = "progs/timemach.mdl".to_string();
        entity.solid = Q1Solid::Slidebox;
        entity.movement = Q1MoveType::Fly;
        entity.max_health = 1600.0;
        entity.movement_flags |= 32;
        entity.angular_velocity = Vec3 {
            x: 0.0,
            y: 60.0,
            z: 0.0,
        };
    })?;
    game.set_health(id, 1600.0)?;
    let pain_name = game.named.pain("rogue:time_pain")?;
    let die_name = game.named.die("rogue:time_pain")?;
    game.update_entity(id, |entity| {
        entity.pain = Some(pain_name);
        entity.die = Some(die_name);
    })?;
    game.set_damageable(id, true)?;
    if let Some(world) = game.world.clone() {
        let machine = id.clone();
        game.update_entity(&world, |world| {
            world.references.insert("rogue:theMachine".to_string(), Some(machine));
        })?;
    }
    game.set_bounds(
        id,
        qa_core::math::Bounds {
            min: Vec3 {
                x: -64.0,
                y: -64.0,
                z: -144.0,
            },
            max: Vec3 {
                x: 64.0,
                y: 64.0,
                z: 0.0,
            },
        },
    )
}

/// Spawn an `item_time_core`.
fn spawn_time_core(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.options().deathmatch != 0 {
        return game.remove(id);
    }
    game.update_entity(id, |entity| {
        entity.model = "progs/timecore.mdl".to_string();
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::Fly;
        entity.angular_velocity = Vec3 {
            x: 60.0,
            y: 60.0,
            z: 60.0,
        };
    })
}

/// Register Rogue time-machine entities (`registerRogueTime`).
pub fn register_rogue_time(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "rogue:time_pain",
        Q1CallbackHandlers {
            pain: Some(time_pain),
            die: Some(time_die),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:time_stop_shake",
        Q1CallbackHandlers {
            action: Some(time_stop_shake),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:time_boom",
        Q1CallbackHandlers {
            action: Some(time_boom),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:time_fall",
        Q1CallbackHandlers {
            action: Some(time_fall),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:time_crash",
        Q1CallbackHandlers {
            action: Some(time_crash_action),
            use_callback: Some(time_crash_use),
            pain: Some(time_crash_pain),
            die: Some(time_crash_die),
            ..Default::default()
        },
    )?;
    game.register_spawn("item_time_machine", spawn_time_machine)?;
    game.register_spawn("item_time_core", spawn_time_core)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    fn with_world(game: &mut Q1EntityServices) -> ActorId {
        let world = game.create("worldspawn", None, None).expect("world");
        game.world = Some(world.clone());
        world
    }

    #[test]
    fn time_machine_spawns_armed_and_linked() {
        let mut game = test_game();
        register_rogue_time(&mut game).expect("register");
        let world = with_world(&mut game);
        let id = game.create("item_time_machine", None, None).expect("machine");
        game.spawn_entity(&id, None).expect("spawn");
        assert_eq!(game.health(&id), 1600.0);
        assert!(game.is_damageable(&id));
        assert_eq!(
            game.entity(&world)
                .and_then(|world| world.references.get("rogue:theMachine").cloned().flatten())
                .as_ref(),
            Some(&id)
        );
    }

    #[test]
    fn beating_machine_starts_cutscene() {
        let mut game = test_game();
        register_rogue_time(&mut game).expect("register");
        let world = with_world(&mut game);
        let id = game.create("item_time_machine", None, None).expect("machine");
        game.spawn_entity(&id, None).expect("spawn");
        game.set_health(&id, 900.0).expect("health");
        game.invoke_pain(&id, None, 100.0).expect("pain");
        assert!(game.entity(&id).expect("machine").pain.is_none());
        assert_eq!(
            game.entity(&world).expect("world").number("rogue:cutscene_running"),
            1.0
        );
    }

    #[test]
    fn crash_drops_machine_into_lava() {
        let mut game = test_game();
        register_rogue_time(&mut game).expect("register");
        with_world(&mut game);
        let id = game.create("item_time_machine", None, None).expect("machine");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_action(&id, "rogue:time_crash").expect("crash");
        let machine = game.entity(&id).cloned().expect("machine");
        assert_eq!(machine.movement, Q1MoveType::Fly);
        assert_eq!(machine.target, "timeramp");
        assert_eq!(machine.think.as_deref(), Some("rogue:time_fall"));
    }
}
