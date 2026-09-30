//! Hipnotic trains, bobbing water, and pushables
//! (`src/content/q1/missionpacks/world/hipnotic-train.ts`).
//!
//! hiptrain.qc / hipwater.qc / hip_push.qc entity behavior.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{BodyPatch, DamageDelivery, TouchSurface};
use crate::q1::foundation::types::{vadd, vectors, vscale, vsub, Q1MoveType, Q1Solid, ZERO};
use crate::q1::missionpacks::types::fround;
use crate::q1::{q1_error, Q1Error};

use super::common::{brush, later, number, target_event, vector};

/// Advance a train to its next corner (`trainNext`).
fn train_next(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (current, target) = game
        .entity(id)
        .map(|entity| (entity.number("cnt"), entity.target.clone()))
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let corner = game
        .find(&target)
        .first()
        .cloned()
        .ok_or_else(|| q1_error(format!("hip_train_next: missing {target}")))?;
    let (speed, wait, next_target) = game
        .entity(&corner)
        .map(|corner| (corner.speed, corner.wait, corner.target.clone()))
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if next_target.is_empty() {
        return Err(q1_error("hip_train_next: no next target"));
    }
    let noise1 = game.entity(id).map(|entity| entity.text("noise1")).unwrap_or_default();
    game.update_entity(id, |entity| {
        number(entity, "cnt", speed);
        entity.target.clone_from(&next_target);
        entity.wait = wait;
    })?;
    game.sound_simple(id, &noise1)?;
    let prior = game
        .entity(id)
        .and_then(|entity| entity.references.get("goalentity").cloned().flatten());
    if let Some(prior) = prior {
        let (event, message) = game
            .entity(&prior)
            .map(|prior| (prior.text("event"), prior.message.clone()))
            .unwrap_or_default();
        if !event.is_empty() {
            target_event(game, id, &event, &message)?;
        }
    }
    let corner_id = corner.clone();
    game.update_entity(id, |entity| {
        entity.references.insert("goalentity".to_string(), Some(corner_id));
    })?;
    let next = if wait != 0.0 {
        "hip:train_wait"
    } else {
        "hip:train_next"
    };
    let destination = vsub(game.body(&corner)?.origin, game.body(id)?.bounds.min);
    if current == -1.0 {
        game.set_origin(id, destination)?;
        let ltime = game.entity(id).map(|entity| entity.number("ltime")).unwrap_or(0.0);
        let done = game.named.action(next)?;
        return game.schedule_at(id, fround(ltime + 0.01), &done);
    }
    if current > 0.0 {
        game.update_entity(id, |entity| entity.speed = current)?;
    }
    let speed = game.entity(id).map(|entity| entity.speed).unwrap_or(0.0);
    let done = game.named.action(next)?;
    game.calc_move(id, destination, speed, &done)
}

/// Place a train at its first corner (`hip:train_find`).
fn train_find(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let target = game.entity(id).map(|entity| entity.target.clone()).unwrap_or_default();
    let corner = game
        .find(&target)
        .first()
        .cloned()
        .ok_or_else(|| q1_error(format!("hip_func_train_find: missing {target}")))?;
    let speed = game.entity(&corner).map(|corner| corner.speed).unwrap_or(0.0);
    let next_target = game
        .entity(&corner)
        .map(|corner| corner.target.clone())
        .unwrap_or_default();
    let corner_id = corner.clone();
    game.update_entity(id, |entity| {
        entity.references.insert("goalentity".to_string(), Some(corner_id));
        number(entity, "cnt", speed);
        entity.target = next_target;
    })?;
    let destination = vsub(game.body(&corner)?.origin, game.body(id)?.bounds.min);
    game.set_origin(id, destination)?;
    let (targetname, ltime) = game
        .entity(id)
        .map(|entity| (entity.targetname.clone(), entity.number("ltime")))
        .unwrap_or_default();
    if targetname.is_empty() {
        let done = game.named.action("hip:train_next")?;
        return game.schedule_at(id, fround(ltime + 0.1), &done);
    }
    Ok(())
}

/// Wait at a corner, then continue (`hip:train_wait`).
fn train_wait(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (wait, ltime, noise) = game
        .entity(id)
        .map(|entity| (entity.wait, entity.number("ltime"), entity.text("noise")))
        .unwrap_or((0.0, 0.0, String::new()));
    if wait != 0.0 {
        game.sound_simple(id, &noise)?;
        if wait == -1.0 {
            return Ok(());
        }
        game.update_entity(id, |entity| entity.wait = 0.0)?;
        let done = game.named.action("hip:train_next")?;
        return game.schedule_at(id, fround(ltime + wait), &done);
    }
    let done = game.named.action("hip:train_next")?;
    game.schedule_at(id, fround(ltime + 0.1), &done)
}

/// Start a stopped train (`hip:train_use`).
fn train_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let velocity = game.body(id)?.velocity;
    if velocity.x != 0.0 || velocity.y != 0.0 || velocity.z != 0.0 {
        return Ok(());
    }
    let activator = activator.cloned();
    game.update_entity(id, |entity| entity.activator = activator)?;
    train_next(game, id)
}

/// Crush blocking entities (`hip:train_blocked`).
fn train_blocked(game: &mut Q1EntityServices, id: &ActorId, other: &ActorId) -> Result<(), Q1Error> {
    let (attack_finished, damage) = game
        .entity(id)
        .map(|entity| (entity.attack_finished, entity.damage))
        .unwrap_or((0.0, 0.0));
    if attack_finished > game.time {
        return Ok(());
    }
    let time = game.time;
    game.update_entity(id, |entity| entity.attack_finished = time + 0.5)?;
    let id_copy = id.clone();
    game.damage(
        other,
        Some(&id_copy),
        Some(&id_copy),
        damage,
        &Q1DamageParams {
            delivery: DamageDelivery::Direct,
            death_type: "crush".to_string(),
            ..Default::default()
        },
    );
    Ok(())
}

/// Spawn a `func_train2`.
fn spawn_train(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game
        .entity(id)
        .map(|entity| entity.target.clone())
        .unwrap_or_default()
        .is_empty()
    {
        return Err(q1_error("func_train2 without a target"));
    }
    brush(game, id)?;
    let use_name = game.named.use_callback("hip:train_use")?;
    let blocked_name = game.named.blocked("hip:train_blocked")?;
    let sounds = game.entity(id).map(|entity| entity.sounds).unwrap_or(0);
    game.update_entity(id, |entity| {
        if entity.speed == 0.0 {
            entity.speed = 100.0;
        }
        if entity.damage == 0.0 {
            entity.damage = 2.0;
        }
        number(entity, "cnt", 1.0);
        if entity.text("noise").is_empty() {
            entity.fields.insert(
                "noise".to_string(),
                if sounds == 1 {
                    "plats/train2.wav"
                } else {
                    "misc/null.wav"
                }
                .to_string(),
            );
        }
        if entity.text("noise1").is_empty() {
            entity.fields.insert(
                "noise1".to_string(),
                if sounds == 1 {
                    "plats/train1.wav"
                } else {
                    "misc/null.wav"
                }
                .to_string(),
            );
        }
        entity.use_callback = Some(use_name);
        entity.blocked = Some(blocked_name);
    })?;
    let ltime = game.entity(id).map(|entity| entity.number("ltime")).unwrap_or(0.0);
    let done = game.named.action("hip:train_find")?;
    game.schedule_at(id, fround(ltime + 0.1), &done)
}

/// Bob water up and down (`hip:bobbing_water`).
fn bobbing_water(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (count, speed, ltime, travel) = game
        .entity(id)
        .map(|entity| (entity.count, entity.speed, entity.number("ltime"), entity.number("cnt")))
        .unwrap_or((0.0, 0.0, 0.0, 0.0));
    let mut count = fround(count + speed * (game.time - ltime));
    if count > 360.0 {
        count -= 360.0;
    }
    game.update_entity(id, |entity| entity.count = count)?;
    let origin = game.body(id)?.origin;
    let bob = vectors(Vec3 {
        x: count as f32,
        y: 0.0,
        z: 0.0,
    })
    .forward
    .z;
    game.set_origin(
        id,
        Vec3 {
            x: origin.x,
            y: origin.y,
            z: (f64::from(bob) * travel) as f32,
        },
    )?;
    let time = game.time;
    game.update_entity(id, |entity| number(entity, "ltime", time))?;
    later(game, id, 0.02, "hip:bobbing_water")
}

/// Spawn a `func_bobbingwater`.
fn spawn_bobbing_water(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let (bounds, speed) = (
        game.body(id)?.bounds,
        game.entity(id).map(|entity| entity.speed).unwrap_or(0.0),
    );
    let time = game.time;
    game.update_entity(id, |entity| {
        entity.solid = Q1Solid::None;
        entity.movement = Q1MoveType::Step;
        entity.count = 0.0;
        entity.speed = 360.0 / if speed == 0.0 { 4.0 } else { speed };
        number(entity, "cnt", f64::from(bounds.max.z - bounds.min.z) / 2.0);
        number(entity, "ltime", time);
    })?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )?;
    later(game, id, 0.02, "hip:bobbing_water")
}

/// Shove a pushable away from a toucher (`hip:pushable_touch`).
fn pushable_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let body = game.host.bodies.read(other);
    let owner = game.entity(id).and_then(|entity| entity.owner.clone());
    let (Some(body), Some(owner)) = (body, owner) else {
        return Ok(());
    };
    let yaw = if body.velocity.x.abs() > body.velocity.y.abs() {
        if body.velocity.x > 0.0 {
            0.0
        } else {
            180.0
        }
    } else if body.velocity.y > 0.0 {
        90.0
    } else {
        270.0
    };
    let owned = game
        .entity(id)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let distance = 16.0 * game.frame_seconds;
    game.host.walk_move(&owned, yaw, distance);
    let (proxy_origin, proxy_old) = (
        game.body(id)?.origin,
        game.entity(id).map(|entity| entity.vector("oldorigin")).unwrap_or(ZERO),
    );
    let owner_old = game
        .entity(&owner)
        .map(|entity| entity.vector("oldorigin"))
        .unwrap_or(ZERO);
    game.set_origin(&owner, vadd(owner_old, vsub(proxy_origin, proxy_old)))
}

/// Spawn a `func_pushable` with its proxy wall.
fn spawn_pushable(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    brush(game, id)?;
    let body = game.body(id)?;
    game.update_entity(id, |entity| vector(entity, "oldorigin", body.origin))?;
    let proxy = game.create("pushablewallproxy", None, None)?;
    let touch_name = game.named.touch("hip:pushable_touch")?;
    let owner = id.clone();
    let size = vscale(vsub(body.bounds.max, body.bounds.min), 0.5);
    let origin = vadd(
        vscale(vadd(body.bounds.min, body.bounds.max), 0.5),
        Vec3 { x: 0.0, y: 0.0, z: 1.0 },
    );
    game.update_entity(&proxy, |proxy| {
        proxy.owner = Some(owner);
        proxy.solid = Q1Solid::Bbox;
        proxy.movement = Q1MoveType::Step;
        vector(proxy, "oldorigin", origin);
        proxy.touch = Some(touch_name);
    })?;
    game.set_body(
        &proxy,
        &BodyPatch {
            origin: Some(origin),
            bounds: Some(qa_core::math::Bounds {
                min: vsub(
                    Vec3 {
                        x: -1.0,
                        y: -1.0,
                        z: 0.0,
                    },
                    size,
                ),
                max: vadd(
                    Vec3 {
                        x: 1.0,
                        y: 1.0,
                        z: -2.0,
                    },
                    size,
                ),
            }),
            ..Default::default()
        },
    )?;
    game.link(&proxy)
}

/// Register Hipnotic train entities (`registerHipnoticTrain`).
pub fn register_hipnotic_train(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "hip:train_next",
        Q1CallbackHandlers {
            action: Some(train_next),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:train_find",
        Q1CallbackHandlers {
            action: Some(train_find),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:train_wait",
        Q1CallbackHandlers {
            action: Some(train_wait),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:train_use",
        Q1CallbackHandlers {
            use_callback: Some(train_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:train_blocked",
        Q1CallbackHandlers {
            blocked: Some(train_blocked),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_train2", spawn_train)?;
    game.named.register(
        "hip:bobbing_water",
        Q1CallbackHandlers {
            action: Some(bobbing_water),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_bobbingwater", spawn_bobbing_water)?;
    game.named.register(
        "hip:pushable_touch",
        Q1CallbackHandlers {
            touch: Some(pushable_touch),
            ..Default::default()
        },
    )?;
    game.register_spawn("func_pushable", spawn_pushable)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    #[test]
    fn train_find_places_train_at_corner() {
        let mut game = test_game();
        register_hipnotic_train(&mut game).expect("register");
        let corner = game.create("path_corner", None, None).expect("corner");
        game.update_entity(&corner, |entity| {
            entity.targetname = "c1".to_string();
            entity.target = "c2".to_string();
            entity.speed = 50.0;
        })
        .expect("corner");
        game.set_origin(
            &corner,
            Vec3 {
                x: 100.0,
                y: 0.0,
                z: 8.0,
            },
        )
        .expect("corner origin");
        let train = game.create("func_train2", None, None).expect("train");
        game.update_entity(&train, |entity| entity.target = "c1".to_string())
            .expect("target");
        game.spawn_entity(&train, None).expect("spawn");
        game.invoke_action(&train, "hip:train_find").expect("find");
        let train_entity = game.entity(&train).cloned().expect("train");
        assert_eq!(train_entity.target, "c2");
        assert_eq!(train_entity.number("cnt"), 50.0);
        assert_eq!(
            game.body(&train).expect("body").origin,
            Vec3 {
                x: 100.0,
                y: 0.0,
                z: 8.0
            }
        );
    }

    #[test]
    fn bobbing_water_reschedules_and_moves() {
        let mut game = test_game();
        register_hipnotic_train(&mut game).expect("register");
        let water = game.create("func_bobbingwater", None, None).expect("water");
        game.spawn_entity(&water, None).expect("spawn");
        game.time = 0.5;
        game.invoke_action(&water, "hip:bobbing_water").expect("bob");
        assert_eq!(
            game.entity(&water).expect("water").think.as_deref(),
            Some("hip:bobbing_water")
        );
        assert!(game.entity(&water).expect("water").count > 0.0);
    }

    #[test]
    fn pushable_spawns_a_touch_proxy() {
        let mut game = test_game();
        register_hipnotic_train(&mut game).expect("register");
        let pushable = game.create("func_pushable", None, None).expect("pushable");
        game.spawn_entity(&pushable, None).expect("spawn");
        let proxies: Vec<ActorId> = game
            .entity_ids()
            .into_iter()
            .filter(|id| {
                game.entity(id)
                    .is_some_and(|entity| entity.classname == "pushablewallproxy")
            })
            .collect();
        assert_eq!(proxies.len(), 1);
        let proxy = game.entity(&proxies[0]).cloned().expect("proxy");
        assert_eq!(proxy.owner.as_ref(), Some(&pushable));
        assert_eq!(proxy.touch.as_deref(), Some("hip:pushable_touch"));
    }
}
