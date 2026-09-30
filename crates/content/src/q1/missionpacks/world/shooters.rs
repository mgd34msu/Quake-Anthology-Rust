//! Mission-pack projectile traps (`src/content/q1/missionpacks/world/shooters.ts`).
//!
//! Hipnotic/Rogue misc.qc projectile traps.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::Vec3;

use crate::q1::base::projectiles::{create_missile, launch_laser, launch_spike, SpikeKind};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity::move_direction;
use crate::q1::foundation::entity::Q1ProjectileKind;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, TouchSurface};
use crate::q1::foundation::host::Q1Contents;
use crate::q1::foundation::types::{normalize, vadd, vscale, vsub, Q1Effect, Q1MoveType, Q1SoundChannel, ZERO};
use crate::q1::missionpacks::monsters::dragon::launch_dragon_fireball;
use crate::q1::missionpacks::rogue_weapons::launch_rogue_lava_spike;
use crate::q1::missionpacks::types::Q1MissionPack;
use crate::q1::Q1Error;

use super::common::{later, number, trigger};

/// Impact a Hipnotic laser bolt (`hip:shooter_laser_touch`).
fn shooter_laser_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let owner = game.entity(id).and_then(|entity| entity.owner.clone());
    if owner.as_ref().is_some_and(|owner| same_actor(other, owner)) {
        return Ok(());
    }
    let body = game.body(id)?;
    if game.host.contents(body.origin) == Q1Contents::Sky {
        return game.remove(id);
    }
    let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
    if spawnflags & 16 == 0 {
        game.sound(id, "enforcer/enfstop.wav", Q1SoundChannel::Weapon, 3.0, 1.0)?;
    }
    let origin = vsub(body.origin, vscale(normalize(body.velocity), 8.0));
    if game.health(other) != 0.0 {
        game.effect(Q1Effect::Blood, origin, Some(other), 15);
        let id_copy = id.clone();
        let other = other.clone();
        game.damage(
            &other,
            Some(&id_copy),
            owner.as_ref(),
            15.0,
            &crate::q1::foundation::entity_services::Q1DamageParams::default(),
        );
    } else {
        game.effect_simple(Q1Effect::Gunshot, origin);
    }
    game.remove(id)
}

/// Fire a Rogue shooter trap.
fn rogue_fire(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let origin = game.body(id)?.origin;
    let (spawnflags, movedir) = game
        .entity(id)
        .map(|entity| (entity.spawnflags, entity.movedir))
        .unwrap_or((0, ZERO));
    if spawnflags & 1 != 0 || spawnflags == 0 {
        game.sound_simple(id, "weapons/spike2.wav")?;
        launch_spike(
            game,
            Some(id),
            origin,
            vscale(movedir, 500.0),
            if spawnflags & 1 != 0 {
                SpikeKind::Superspike
            } else {
                SpikeKind::Spike
            },
        )?;
    } else if spawnflags & 2 != 0 {
        game.sound_simple(id, "enforcer/enfire.wav")?;
        launch_laser(game, Some(id), origin, movedir)?;
    } else if spawnflags & 32 != 0 {
        launch_dragon_fireball(game, id, origin, movedir);
    } else {
        game.sound_simple(id, "weapons/spike2.wav")?;
        let powered = spawnflags & 8 != 0 || (spawnflags & 16 != 0 && game.options().skill > 1);
        let spike = launch_rogue_lava_spike(game, id, origin, movedir, powered)?;
        game.set_body(
            &spike,
            &BodyPatch {
                velocity: Some(vscale(movedir, 500.0)),
                ..Default::default()
            },
        )?;
    }
    Ok(())
}

/// Fire a Hipnotic shooter trap.
fn hipnotic_fire(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let origin = game.body(id)?.origin;
    let (spawnflags, movedir) = game
        .entity(id)
        .map(|entity| (entity.spawnflags, entity.movedir))
        .unwrap_or((0, ZERO));
    let audible = spawnflags & 16 == 0;
    if spawnflags & 2 != 0 {
        if audible {
            game.sound_simple(id, "enforcer/enfire.wav")?;
        }
        let laser = launch_laser(game, Some(id), origin, movedir)?;
        let touch_name = game.named.touch("hip:shooter_laser_touch")?;
        game.update_entity(&laser, |laser| {
            laser.movement = Q1MoveType::Fly;
            laser.spawnflags = spawnflags;
            laser.touch = Some(touch_name);
        })?;
    } else if spawnflags & 4 != 0 {
        if audible {
            game.sound_simple(id, "misc/spike.wav")?;
        }
        let ball = create_missile(
            game,
            Some(id),
            "lavaball",
            "lavarock",
            origin,
            vscale(movedir, 300.0),
            5.0,
        )?;
        let touch_name = game.named.touch("projectile_touch")?;
        game.update_entity(&ball, |ball| {
            ball.projectile = Some(Q1ProjectileKind::Rocket);
            ball.touch = Some(touch_name);
            ball.angular_velocity = Vec3 {
                x: 0.0,
                y: 0.0,
                z: 400.0,
            };
        })?;
        game.set_bounds(
            &ball,
            qa_core::math::Bounds {
                min: Vec3 {
                    x: -4.0,
                    y: -4.0,
                    z: -4.0,
                },
                max: Vec3 { x: 4.0, y: 4.0, z: 4.0 },
            },
        )?;
    } else if spawnflags & 8 != 0 {
        if audible {
            game.sound_simple(id, "weapons/sgun1.wav")?;
        }
        game.sound(id, "weapons/sgun1.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        let rocket = create_missile(
            game,
            Some(id),
            "missile",
            "missile",
            vadd(origin, vscale(movedir, 8.0)),
            vscale(movedir, 1000.0),
            5.0,
        )?;
        let touch_name = game.named.touch("projectile_touch")?;
        game.update_entity(&rocket, |rocket| {
            rocket.projectile = Some(Q1ProjectileKind::Rocket);
            rocket.touch = Some(touch_name);
        })?;
    } else {
        if audible {
            game.sound_simple(id, "weapons/spike2.wav")?;
        }
        launch_spike(
            game,
            Some(id),
            origin,
            vscale(movedir, 500.0),
            if spawnflags & 1 != 0 {
                SpikeKind::Superspike
            } else {
                SpikeKind::Spike
            },
        )?;
    }
    Ok(())
}

/// Fire a shooter trap from a use dispatch (pack chosen at registration).
fn shooter_fire_rogue(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    rogue_fire(game, id)
}

/// Fire a shooter trap from a use dispatch (pack chosen at registration).
fn shooter_fire_hipnotic(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    hipnotic_fire(game, id)
}

/// Run a Rogue shooter's idle cycle.
fn shooter_think_rogue(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    rogue_fire(game, id)?;
    let wait = game.entity(id).map(|entity| entity.wait).unwrap_or(0.0);
    later(game, id, wait, "mission:shooter_think")
}

/// Run a Hipnotic shooter's idle cycle.
fn shooter_think_hipnotic(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game
        .entity(id)
        .map(|entity| entity.number("shooter_state"))
        .unwrap_or(0.0)
        != 0.0
    {
        hipnotic_fire(game, id)?;
    }
    let wait = game.entity(id).map(|entity| entity.wait).unwrap_or(0.0);
    later(game, id, wait, "mission:shooter_think")
}

/// Toggle a switched shooter.
fn shooter_switch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let state = game
        .entity(id)
        .map(|entity| entity.number("shooter_state"))
        .unwrap_or(0.0);
    game.update_entity(id, |entity| number(entity, "shooter_state", 1.0 - state))
}

/// Spawn a shooter trap.
fn spawn_shooter(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let angles = game.body(id)?.angles;
    let movedir = move_direction(angles, Some(game));
    game.update_entity(id, |entity| entity.movedir = movedir)?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    let use_name = game.named.use_callback("mission:shooter_fire")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))?;
    let classname = game
        .entity(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    if classname == "trap_spikeshooter" {
        return Ok(());
    }
    game.update_entity(id, |entity| {
        if entity.wait == 0.0 {
            entity.wait = 1.0;
        }
    })?;
    let state = if classname == "trap_shooter" {
        1.0
    } else {
        game.entity(id).map(|entity| entity.number("state")).unwrap_or(0.0)
    };
    game.update_entity(id, |entity| number(entity, "shooter_state", state))?;
    if classname == "trap_switched_shooter" {
        let switch_name = game.named.use_callback("mission:shooter_switch")?;
        game.update_entity(id, |entity| entity.use_callback = Some(switch_name))?;
    }
    let (nextthink, wait, ltime) = game
        .entity(id)
        .map(|entity| (entity.number("nextthink"), entity.wait, entity.number("ltime")))
        .unwrap_or((0.0, 0.0, 0.0));
    later(game, id, nextthink + wait + ltime - game.time, "mission:shooter_think")
}

/// Toggle a Rogue push trigger.
fn rogue_push_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| entity.spawnflags ^= 4)
}

/// Fling grenades and actors out of a Rogue push trigger.
fn rogue_push_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0) & 4 == 0 {
        return Ok(());
    }
    let grenade = ["grenade", "MiniGrenade", "MultiGrenade"].contains(&game.host.classname(other).as_str());
    if grenade || game.health(other) > 0.0 {
        let owned = game.host.actors.resolve_owned(other);
        let body = game.host.bodies.read(other);
        if let (Some(owned), Some(body)) = (owned, body) {
            let (movedir, speed) = game
                .entity(id)
                .map(|entity| (entity.movedir, entity.speed))
                .unwrap_or((ZERO, 0.0));
            let mut next = body.clone();
            next.velocity = vscale(movedir, speed * 10.0);
            game.host.bodies.write(&owned, &next)?;
            if !grenade && game.is_player(other) {
                let state = game.entity_ids().into_iter().find(|id| {
                    game.entity(id).is_some_and(|entity| {
                        entity.classname == "rogue_team_state"
                            && entity.owner.as_ref().is_some_and(|owner| same_actor(owner, other))
                    })
                });
                if let Some(state) = state {
                    let fly_sound = game
                        .entity(&state)
                        .map(|entity| entity.number("fly_sound"))
                        .unwrap_or(0.0);
                    if fly_sound < game.time {
                        let time = game.time;
                        game.update_entity(&state, |entity| number(entity, "fly_sound", time + 1.5))?;
                        game.sound_simple(other, "ambience/windfly.wav")?;
                    }
                }
            }
        }
    }
    if game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0) & 1 != 0 {
        return game.remove(id);
    }
    Ok(())
}

/// Spawn a Rogue `trigger_push`.
fn spawn_trigger_push(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0) & 2 != 0 {
        let use_name = game.named.use_callback("rogue:push")?;
        game.update_entity(id, |entity| entity.use_callback = Some(use_name))?;
    } else {
        game.update_entity(id, |entity| entity.spawnflags += 4)?;
    }
    let touch_name = game.named.touch("rogue:push")?;
    game.update_entity(id, |entity| {
        entity.touch = Some(touch_name);
        if entity.speed == 0.0 {
            entity.speed = 1000.0;
        }
    })?;
    trigger(game, id)
}

/// Register mission-pack shooter traps (`registerMissionShooters`).
pub fn register_mission_shooters(game: &mut Q1EntityServices, pack: Q1MissionPack) -> Result<(), Q1Error> {
    game.named.register(
        "hip:shooter_laser_touch",
        Q1CallbackHandlers {
            touch: Some(shooter_laser_touch),
            ..Default::default()
        },
    )?;
    let rogue = pack == Q1MissionPack::Rogue;
    game.named.register(
        "mission:shooter_fire",
        Q1CallbackHandlers {
            use_callback: Some(if rogue {
                shooter_fire_rogue
            } else {
                shooter_fire_hipnotic
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mission:shooter_think",
        Q1CallbackHandlers {
            action: Some(if rogue {
                shooter_think_rogue
            } else {
                shooter_think_hipnotic
            }),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mission:shooter_switch",
        Q1CallbackHandlers {
            use_callback: Some(shooter_switch),
            ..Default::default()
        },
    )?;
    game.replace_spawn("trap_spikeshooter", spawn_shooter)?;
    game.replace_spawn("trap_shooter", spawn_shooter)?;
    if pack == Q1MissionPack::Hipnotic {
        game.register_spawn("trap_switched_shooter", spawn_shooter)?;
        return Ok(());
    }
    game.named.register(
        "rogue:push",
        Q1CallbackHandlers {
            use_callback: Some(rogue_push_use),
            touch: Some(rogue_push_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("trigger_push", spawn_trigger_push)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn register_for_test(game: &mut Q1EntityServices, pack: Q1MissionPack) -> Q1BaseGuard {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_mission_shooters(game, pack).expect("register");
        guard
    }

    #[test]
    fn hipnotic_shooter_think_fires_spikes() {
        let mut game = test_game();
        let _guard = register_for_test(&mut game, Q1MissionPack::Hipnotic);
        let id = game.create("trap_shooter", None, None).expect("shooter");
        game.spawn_entity(&id, None).expect("spawn");
        game.invoke_action(&id, "mission:shooter_think").expect("think");
        let spikes = game
            .entity_ids()
            .into_iter()
            .filter(|id| game.entity(id).is_some_and(|entity| entity.classname == "spike"));
        assert_eq!(spikes.count(), 1);
    }

    #[test]
    fn switched_shooter_toggles_on_use() {
        let mut game = test_game();
        let _guard = register_for_test(&mut game, Q1MissionPack::Hipnotic);
        let id = game.create("trap_switched_shooter", None, None).expect("shooter");
        game.spawn_entity(&id, None).expect("spawn");
        assert_eq!(
            game.entity(&id).expect("shooter").use_callback.as_deref(),
            Some("mission:shooter_switch")
        );
        game.invoke_use(&id, "mission:shooter_switch", None, None)
            .expect("switch");
        assert_eq!(game.entity(&id).expect("shooter").number("shooter_state"), 1.0);
    }

    #[test]
    fn rogue_push_flings_healthy_actors() {
        let mut game = test_game();
        let _guard = register_for_test(&mut game, Q1MissionPack::Rogue);
        let id = game.create("trigger_push", None, None).expect("push");
        game.spawn_entity(&id, None).expect("spawn");
        assert_eq!(game.entity(&id).expect("push").spawnflags & 4, 4);
        let victim = game.create("monster_army", None, None).expect("victim");
        game.set_health(&victim, 100.0).expect("health");
        game.invoke_touch(&id, &victim, None, None).expect("touch");
        let velocity = game.host.bodies.read(&victim).expect("body").velocity;
        assert_eq!(f64::from(velocity.x), 10000.0);
    }
}
