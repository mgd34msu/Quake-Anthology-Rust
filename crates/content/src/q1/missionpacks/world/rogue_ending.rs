//! Rogue ending cinema sequence (`src/content/q1/missionpacks/world/rogue-ending.ts`).
//!
//! ending.qc actor and camera sequence.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::Vec3;

use crate::q1::base::projectiles::create_missile;
use crate::q1::base::provider::{level_begin_cutscene, official_campaign_flag};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity::Q1ProjectileKind;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::host::Q1CutsceneControl;
use crate::q1::foundation::spawns::spawn_teleport_fog;
use crate::q1::foundation::types::{
    vadd, vscale, vsub, yaw_for, Q1Edition, Q1Event, Q1MoveType, Q1Solid, PLAYER_BOUNDS, ZERO,
};
use crate::q1::foundation::weapons::aim;
use crate::q1::missionpacks::types::velocity_angles;
use crate::q1::missionpacks::world::finale_text::mission_finale_text;
use crate::q1::{q1_error, Q1Error};

use super::campaign::start_finale_timer;
use super::common::{later, number, vector};
use super::with_missionpack_hooks;

/// Resolve the end-sequence time machine (`machine`).
fn machine(game: &Q1EntityServices) -> Result<ActorId, Q1Error> {
    game.world
        .as_ref()
        .and_then(|world| game.entity(world))
        .and_then(|world| world.references.get("rogue:theMachine").cloned().flatten())
        .filter(|machine| game.entity(machine).is_some())
        .ok_or_else(|| q1_error("End sequence time machine is missing"))
}

/// Clear lightning trails and the time core (`removeStuff`).
fn remove_stuff(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let trails: Vec<ActorId> = game
        .entity_ids()
        .into_iter()
        .filter(|id| game.entity(id).is_some_and(|entity| entity.classname == "ltrail_start"))
        .collect();
    for trail in trails {
        game.remove(&trail)?;
    }
    let core = game.entity_ids().into_iter().find(|id| {
        game.entity(id)
            .is_some_and(|entity| entity.classname == "item_time_core")
    });
    let origin = core
        .as_ref()
        .and_then(|core| game.body(core).ok())
        .map(|body| body.origin)
        .unwrap_or(ZERO);
    game.host.emit(Q1Event::ColoredExplosion {
        origin,
        color_start: 230,
        color_length: 5,
    });
    if let Some(core) = core {
        return later(game, &core, 0.1, "SUB_Remove");
    }
    Ok(())
}

/// Pull the cinema actor out of lava (`escapeLava`).
fn escape_lava(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    if game.host.contents(game.body(actor)?.origin) != crate::q1::foundation::host::Q1Contents::Lava {
        return Ok(());
    }
    let point = game.find("point1").first().cloned();
    if let Some(point) = point {
        let origin = game.body(&point)?.origin;
        return game.set_origin(actor, origin);
    }
    Ok(())
}

/// Send the cinema actor to a path point (`goal`).
fn goal(game: &mut Q1EntityServices, actor: &ActorId, target: &str) -> Result<(), Q1Error> {
    let point = game
        .find(target)
        .first()
        .cloned()
        .ok_or_else(|| q1_error(format!("End sequence {target} placing screwed up!")))?;
    game.update_entity(actor, |actor| actor.target = target.to_string())?;
    let goal = point.clone();
    let follower = point;
    game.update_entity(actor, |actor| {
        actor.references.insert("goalentity".to_string(), Some(goal));
        actor.references.insert("movetarget".to_string(), Some(follower));
    })
}

/// Step the cinema state machine (`control`).
fn control(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let world = game
        .world
        .clone()
        .ok_or_else(|| q1_error("Ending requires worldspawn"))?;
    let stage = game
        .entity(&world)
        .map(|world| world.number("rogue:actorStage"))
        .unwrap_or(0.0);
    if stage == 0.0 {
        goal(game, actor, "point1")?;
        game.update_entity(actor, |actor| actor.frame = 6)?;
        game.update_entity(&world, |world| number(world, "rogue:actorStage", 1.0))?;
        return later(game, actor, 0.1, "rogue:actor_run");
    }
    if stage == 2.0 {
        goal(game, actor, "machine")?;
        game.update_entity(&world, |world| number(world, "rogue:actorStage", 5.0))?;
        return later(game, actor, 0.1, "rogue:actor_fire1");
    }
    if stage == 4.0 {
        game.update_entity(actor, |actor| actor.frame = 12)?;
        return later(game, actor, 2.0, "rogue:actor_teleport");
    }
    if stage == 3.0 {
        game.update_entity(actor, |actor| actor.target = "timepod".to_string())?;
        let activator = game.entity(actor).and_then(|entity| entity.activator.clone());
        game.use_targets(actor, activator.as_ref())?;
        goal(game, actor, "point2")?;
        game.update_entity(actor, |actor| actor.frame = 6)?;
        return later(game, actor, 0.1, "rogue:actor_run");
    }
    Ok(())
}

/// Start the Rogue ending cinema (`startRogueEnding`).
pub fn start_rogue_ending(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    let world = game.world.clone();
    let Some(world) = world else {
        return Ok(());
    };
    let (running, started) = game
        .entity(&world)
        .map(|world| {
            (
                world.number("rogue:cutscene_running"),
                world.number("rogue:ending_started"),
            )
        })
        .unwrap_or((0.0, 0.0));
    if running == 0.0 || started != 0.0 {
        return Ok(());
    }
    let body = game.host.bodies.read(player);
    let Some(body) = body else {
        return Ok(());
    };
    game.update_entity(&world, |world| number(world, "rogue:ending_started", 1.0))?;
    let camera = game.find("cameraview").first().cloned();
    let map = game.map_name.clone();
    let time = game.time;
    level_begin_cutscene(
        game,
        &map,
        Some(player),
        time + if game.options().coop || camera.is_none() {
            3.0
        } else {
            10_000_000.0
        },
    )?;
    if game.options().coop || camera.is_none() {
        game.control_player(
            player,
            &Q1CutsceneControl {
                origin: vadd(
                    body.origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 48.0,
                    },
                ),
                angles: body.angles,
                view_offset: ZERO,
            },
        )?;
        game.host.emit(Q1Event::Finale {
            text: mission_finale_text(game.options().edition, "$qc_finale_coop"),
            stage: 4,
        });
        remove_stuff(game)?;
        let target = machine(game)?;
        return later(game, &target, 0.1, "rogue:time_crash");
    }
    game.host.emit(Q1Event::Finale {
        text: String::new(),
        stage: 1,
    });
    let actor = game.create("actor", None, None)?;
    let player_id = player.clone();
    let frame = with_missionpack_hooks(game, |_, hooks| {
        Ok(hooks.player_frame.as_ref().map(|player_frame| player_frame(player)))
    })?;
    game.update_entity(&actor, |actor| {
        actor.owner = Some(player_id);
        actor.max_health = 100.0;
        actor.solid = Q1Solid::Slidebox;
        actor.movement = Q1MoveType::Step;
        actor.frame = frame.unwrap_or(0);
        actor.model = "progs/player.mdl".to_string();
    })?;
    let owned = game
        .entity(&actor)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    game.host.combat.set_health(&owned, 100.0)?;
    game.set_body(
        &actor,
        &BodyPatch {
            origin: Some(body.origin),
            angles: Some(body.angles),
            bounds: Some(PLAYER_BOUNDS),
            ..Default::default()
        },
    )?;
    game.update_entity(&actor, |actor| {
        vector(
            actor,
            "view_ofs",
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 25.0,
            },
        );
        actor.movement_flags |= 32;
        actor.ideal_yaw = f64::from(body.angles.y);
        actor.yaw_speed = 20.0;
    })?;
    let actor_id = actor.clone();
    game.update_entity(&world, |world| {
        world.references.insert("rogue:theActor".to_string(), Some(actor_id));
    })?;
    escape_lava(game, &actor)?;
    later(game, &actor, 0.1, "rogue:actor_control")?;
    game.link(&actor)?;
    let camera = camera.expect("camera");
    let origin = game.body(&camera)?.origin;
    let angles = velocity_angles(vsub(game.body(&actor)?.origin, origin));
    game.control_player(
        player,
        &Q1CutsceneControl {
            origin,
            angles,
            view_offset: ZERO,
        },
    )?;
    let tracker = game.create("rogue_camera_tracker", None, None)?;
    let player_id = player.clone();
    game.update_entity(&tracker, |tracker| tracker.owner = Some(player_id))?;
    later(game, &tracker, 0.05, "rogue:track_camera")
}

/// Advance cinema followers along their path.
fn world_followers(game: &mut Q1EntityServices, corner: &ActorId, mover: &ActorId) -> Result<bool, Q1Error> {
    let classname = game
        .entity(mover)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    if classname != "actor" && classname != "buzzsaw" {
        return Ok(false);
    }
    let current = game
        .entity(mover)
        .and_then(|entity| entity.references.get("movetarget").cloned().flatten());
    if current.as_ref().is_none_or(|current| !same_actor(current, corner)) {
        return Ok(false);
    }
    let target = game
        .entity(corner)
        .map(|entity| entity.target.clone())
        .unwrap_or_default();
    let next = game.find(&target).first().cloned();
    game.update_entity(mover, |mover| {
        mover.references.insert("goalentity".to_string(), next.clone());
        mover.references.insert("movetarget".to_string(), next.clone());
    })?;
    let destination = next
        .as_ref()
        .and_then(|next| game.body(next).ok())
        .map(|body| body.origin)
        .unwrap_or(ZERO);
    let origin = game.body(mover)?.origin;
    game.update_entity(mover, |mover| mover.ideal_yaw = yaw_for(vsub(destination, origin)))?;
    if next.is_none() {
        let time = game.time;
        game.update_entity(mover, |mover| number(mover, "pausetime", time + 999999.0))?;
        if classname == "buzzsaw" {
            later(game, mover, 0.1, "rogue:saw_stand")?;
        }
    }
    Ok(true)
}

/// Keep the camera on the cinema actor.
fn track_camera(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let actor = game
        .world
        .as_ref()
        .and_then(|world| game.entity(world))
        .and_then(|world| world.references.get("rogue:theActor").cloned().flatten())
        .filter(|actor| game.entity(actor).is_some());
    let player = game.entity(id).and_then(|entity| entity.owner.clone());
    let body = player.as_ref().and_then(|player| game.host.bodies.read(player));
    let (Some(actor), Some(player), Some(body)) = (actor, player, body) else {
        return game.remove(id);
    };
    let delta = vsub(game.body(&actor)?.origin, body.origin);
    game.control_player(
        &player,
        &Q1CutsceneControl {
            origin: body.origin,
            angles: velocity_angles(Vec3 {
                x: delta.x,
                y: delta.y,
                z: -delta.z,
            }),
            view_offset: ZERO,
        },
    )?;
    later(game, id, 0.1, "rogue:track_camera")
}

/// Step the cinema control state machine from a scheduled dispatch.
fn actor_control(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    control(game, id)
}

/// Teleport the cinema actor out.
fn actor_teleport(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let origin = game.body(id)?.origin;
    spawn_teleport_fog(game, origin)?;
    game.update_entity(id, |entity| entity.model.clear())?;
    later(game, id, 999999.0, "SUB_Null")
}

/// Run the cinema actor toward its goal.
fn actor_run(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    escape_lava(game, id)?;
    let point = game
        .entity(id)
        .and_then(|entity| entity.references.get("goalentity").cloned().flatten())
        .filter(|point| game.entity(point).is_some());
    if point.as_ref().is_some_and(|point| {
        game.entity(point)
            .is_some_and(|point| point.targetname == "endpoint1" || point.targetname == "endpoint2")
    }) {
        let point = point.expect("point");
        let endpoint = game
            .entity(&point)
            .map(|point| point.targetname.clone())
            .unwrap_or_default();
        if let Some(world) = game.world.clone() {
            let stage = if endpoint == "endpoint1" { 2.0 } else { 4.0 };
            game.update_entity(&world, |world| number(world, "rogue:actorStage", stage))?;
        }
        return later(game, id, 0.1, "rogue:actor_control");
    }
    game.update_entity(id, |entity| {
        entity.frame += 1;
        if entity.frame > 11 {
            entity.frame = 6;
        }
    })?;
    if let Some(point) = point {
        let owned = game
            .entity(id)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        game.host.move_to_goal(&owned, &point, 15.0, None);
    }
    later(game, id, 0.1, "rogue:actor_run")
}

/// Run one cinema fire stage.
fn actor_fire(game: &mut Q1EntityServices, id: &ActorId, stage: i32) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.frame = if stage <= 6 { 106 + stage } else { 12 + (stage - 7) % 5 };
    })?;
    if stage == 1 {
        let target = machine(game)?;
        game.update_entity(id, |entity| {
            entity.references.insert("goalentity".to_string(), Some(target.clone()));
        })?;
        let pain_name = game.named.pain("rogue:time_crash")?;
        let die_name = game.named.die("rogue:time_crash")?;
        game.update_entity(&target, |target| {
            target.pain = Some(pain_name);
            target.die = Some(die_name);
        })?;
        let owned = game
            .entity(&target)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        game.host.combat.set_health(&owned, 1.0)?;
        let angles = velocity_angles(vsub(game.body(&target)?.origin, game.body(id)?.origin));
        game.set_body(
            id,
            &BodyPatch {
                angles: Some(angles),
                ..Default::default()
            },
        )?;
        game.update_entity(id, |entity| entity.effects = 2)?;
        let flipped = Vec3 {
            x: -angles.x,
            y: angles.y,
            z: angles.z,
        };
        let basis = game.make_vectors(flipped);
        game.update_entity(id, |entity| {
            vector(entity, "v_angle", flipped);
            number(entity, "ammo_rockets1", entity.number("ammo_rockets1") - 1.0);
            number(entity, "currentammo", entity.number("ammo_rockets1"));
        })?;
        game.sound(
            id,
            "weapons/sgun1.wav",
            crate::q1::foundation::types::Q1SoundChannel::Weapon,
            1.0,
            1.0,
        )?;
        let owned = game
            .entity(id)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        let origin = game.body(id)?.origin;
        let aimed = aim(game, &owned, basis.forward);
        let missile = create_missile(
            game,
            Some(id),
            "missile",
            "missile",
            vadd(
                vadd(origin, vscale(basis.forward, 8.0)),
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 16.0,
                },
            ),
            vscale(aimed, 1000.0),
            5.0,
        )?;
        let touch_name = game.named.touch("projectile_touch")?;
        game.update_entity(&missile, |missile| {
            missile.projectile = Some(Q1ProjectileKind::Rocket);
            missile.touch = Some(touch_name);
        })?;
        game.host.emit(Q1Event::Finale {
            text: mission_finale_text(game.options().edition, "$qc_finale_rogue_end"),
            stage: 4,
        });
        if game.options().edition == Q1Edition::Rerelease && official_campaign_flag(game)? && game.map_name == "r2m8" {
            game.host.emit(Q1Event::Achievement {
                player: None,
                id: "ACH_COMPLETE_R2M8".to_string(),
            });
            if game.options().skill == 3 {
                game.host.emit(Q1Event::Achievement {
                    player: None,
                    id: "ACH_COMPLETE_R2M8_NIGHTMARE".to_string(),
                });
            }
        }
        start_finale_timer(game)?;
    } else if stage == 2 {
        let angles = game.body(id)?.angles;
        game.set_body(
            id,
            &BodyPatch {
                angles: Some(Vec3 {
                    x: 0.0,
                    y: angles.y,
                    z: angles.z,
                }),
                ..Default::default()
            },
        )?;
        let v_angle = game.entity(id).map(|entity| entity.vector("v_angle")).unwrap_or(ZERO);
        game.update_entity(id, |entity| {
            vector(
                entity,
                "v_angle",
                Vec3 {
                    x: 0.0,
                    y: v_angle.y,
                    z: v_angle.z,
                },
            );
        })?;
    } else if stage == 5 {
        remove_stuff(game)?;
        let target = machine(game)?;
        if game.health(&target) > 0.0 {
            later(game, &target, 0.1, "rogue:time_crash")?;
        }
    } else if stage == 6 {
        game.update_entity(id, |entity| entity.effects = 0)?;
    } else if stage == 21 {
        if let Some(world) = game.world.clone() {
            game.update_entity(&world, |world| number(world, "rogue:actorStage", 3.0))?;
        }
    }
    later(
        game,
        id,
        if stage == 1 { 0.1 } else { 0.15 },
        &if stage == 21 {
            "rogue:actor_control".to_string()
        } else {
            format!("rogue:actor_fire{}", stage + 1)
        },
    )
}

macro_rules! actor_fire {
    ($name:ident, $stage:expr) => {
        fn $name(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
            actor_fire(game, id, $stage)
        }
    };
}

actor_fire!(actor_fire_1, 1);
actor_fire!(actor_fire_2, 2);
actor_fire!(actor_fire_3, 3);
actor_fire!(actor_fire_4, 4);
actor_fire!(actor_fire_5, 5);
actor_fire!(actor_fire_6, 6);
actor_fire!(actor_fire_7, 7);
actor_fire!(actor_fire_8, 8);
actor_fire!(actor_fire_9, 9);
actor_fire!(actor_fire_10, 10);
actor_fire!(actor_fire_11, 11);
actor_fire!(actor_fire_12, 12);
actor_fire!(actor_fire_13, 13);
actor_fire!(actor_fire_14, 14);
actor_fire!(actor_fire_15, 15);
actor_fire!(actor_fire_16, 16);
actor_fire!(actor_fire_17, 17);
actor_fire!(actor_fire_18, 18);
actor_fire!(actor_fire_19, 19);
actor_fire!(actor_fire_20, 20);
actor_fire!(actor_fire_21, 21);

/// Register Rogue ending entities (`registerRogueEnding`).
pub fn register_rogue_ending(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.register_path_touch("rogue:world_followers", world_followers)?;
    game.named.register(
        "rogue:track_camera",
        Q1CallbackHandlers {
            action: Some(track_camera),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:actor_control",
        Q1CallbackHandlers {
            action: Some(actor_control),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:actor_teleport",
        Q1CallbackHandlers {
            action: Some(actor_teleport),
            ..Default::default()
        },
    )?;
    game.named.register(
        "rogue:actor_run",
        Q1CallbackHandlers {
            action: Some(actor_run),
            ..Default::default()
        },
    )?;
    for (name, action) in [
        (
            "rogue:actor_fire1",
            actor_fire_1 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire2",
            actor_fire_2 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire3",
            actor_fire_3 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire4",
            actor_fire_4 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire5",
            actor_fire_5 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire6",
            actor_fire_6 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire7",
            actor_fire_7 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire8",
            actor_fire_8 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire9",
            actor_fire_9 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire10",
            actor_fire_10 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire11",
            actor_fire_11 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire12",
            actor_fire_12 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire13",
            actor_fire_13 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire14",
            actor_fire_14 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire15",
            actor_fire_15 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire16",
            actor_fire_16 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire17",
            actor_fire_17 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire18",
            actor_fire_18 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire19",
            actor_fire_19 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire20",
            actor_fire_20 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
        (
            "rogue:actor_fire21",
            actor_fire_21 as crate::q1::foundation::callbacks::Q1ActionHandler,
        ),
    ] {
        game.named.register(
            name,
            Q1CallbackHandlers {
                action: Some(action),
                ..Default::default()
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn game_with_base() -> (Box<Q1EntityServices>, Q1BaseGuard) {
        let mut game = Box::new(test_game());
        let guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        register_rogue_ending(&mut game).expect("ending");
        crate::q1::missionpacks::world::rogue_time::register_rogue_time(&mut game).expect("time");
        game.host.control_player = Some(Box::new(|_, _| {}));
        (game, guard)
    }

    fn with_world(game: &mut Q1EntityServices) -> ActorId {
        let world = game.create("worldspawn", None, None).expect("world");
        game.world = Some(world.clone());
        world
    }

    #[test]
    fn ending_ignores_unready_worlds() {
        let (mut game, _guard) = game_with_base();
        let player = game.create("player", None, None).expect("player");
        start_rogue_ending(&mut game, &player).expect("no world");
        let world = with_world(&mut game);
        start_rogue_ending(&mut game, &player).expect("not running");
        assert_eq!(game.entity(&world).expect("world").number("rogue:ending_started"), 0.0);
    }

    #[test]
    fn ending_crashes_machine_without_camera() {
        let (mut game, _guard) = game_with_base();
        let world = with_world(&mut game);
        let machine = game.create("item_time_machine", None, None).expect("machine");
        game.spawn_entity(&machine, None).expect("spawn");
        game.update_entity(&world, |world| number(world, "rogue:cutscene_running", 1.0))
            .expect("running");
        let player = game.create("player", None, None).expect("player");
        start_rogue_ending(&mut game, &player).expect("start");
        assert_eq!(game.entity(&world).expect("world").number("rogue:ending_started"), 1.0);
        assert_eq!(
            game.entity(&machine).expect("machine").think.as_deref(),
            Some("rogue:time_crash")
        );
    }

    #[test]
    fn actor_control_walks_from_point_to_fire() {
        let (mut game, _guard) = game_with_base();
        let world = with_world(&mut game);
        let point = game.create("path_corner", None, None).expect("point");
        game.update_entity(&point, |entity| entity.targetname = "point1".to_string())
            .expect("targetname");
        let machine = game.create("path_corner", None, None).expect("machine point");
        game.update_entity(&machine, |entity| entity.targetname = "machine".to_string())
            .expect("targetname");
        let time_machine = game.create("item_time_machine", None, None).expect("time machine");
        game.spawn_entity(&time_machine, None).expect("spawn");
        let actor = game.create("actor", None, None).expect("actor");
        game.invoke_action(&actor, "rogue:actor_control").expect("control");
        assert_eq!(game.entity(&world).expect("world").number("rogue:actorStage"), 1.0);
        assert_eq!(
            game.entity(&actor).expect("actor").think.as_deref(),
            Some("rogue:actor_run")
        );
        game.update_entity(&world, |world| number(world, "rogue:actorStage", 2.0))
            .expect("stage");
        game.invoke_action(&actor, "rogue:actor_control").expect("control");
        assert_eq!(
            game.entity(&actor).expect("actor").think.as_deref(),
            Some("rogue:actor_fire1")
        );
        game.invoke_action(&actor, "rogue:actor_fire1").expect("fire");
        assert_eq!(game.entity(&actor).expect("actor").effects, 2);
        assert_eq!(
            game.entity(&actor).expect("actor").think.as_deref(),
            Some("rogue:actor_fire2")
        );
    }

    #[test]
    fn world_followers_advance_movers() {
        let (mut game, _guard) = game_with_base();
        let next = game.create("path_corner", None, None).expect("next");
        game.update_entity(&next, |entity| entity.targetname = "c2".to_string())
            .expect("targetname");
        let corner = game.create("path_corner", None, None).expect("corner");
        game.update_entity(&corner, |entity| {
            entity.targetname = "c1".to_string();
            entity.target = "c2".to_string();
        })
        .expect("target");
        let mover = game.create("actor", None, None).expect("mover");
        let corner_id = corner.clone();
        game.update_entity(&mover, |entity| {
            entity.references.insert("movetarget".to_string(), Some(corner_id));
        })
        .expect("movetarget");
        assert!(world_followers(&mut game, &corner, &mover).expect("follow"));
        assert_eq!(
            game.entity(&mover)
                .and_then(|entity| entity.references.get("goalentity").cloned().flatten())
                .as_ref(),
            Some(&next)
        );
    }
}
