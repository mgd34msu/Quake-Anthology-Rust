//! Q1 reborn Old One projectiles (`src/content/q1/addons/monsters/bosses/oldnew-projectiles.ts`).
//!
//! `mg3_oldone_new.qc` projectiles and sphere managers.
//! GPL-2.0-or-later.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::{set_addon_number, Q1AddonContext};
use crate::q1::base::projectiles::{launch_spike, SpikeKind};
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::host::Q1Contents;
use crate::q1::foundation::types::{
    dot, normalize, vadd, vscale, vsub, Q1MoveType, Q1Powerup, Q1Solid, Q1SoundChannel, POINT, ZERO,
};
use crate::q1::missionpacks::types::velocity_angles;
use crate::q1::{q1_error, Q1Error};

use super::sphere_points::sphere_point;

/// Old One boss callback prefix (`oldnewPrefix`).
pub const OLDNEW_PREFIX: &str = "mg3:bosses:";

/// Schedule a boss callback (`bossLater`).
pub fn boss_later(
    context: &Q1AddonContext,
    game: &mut Q1EntityServices,
    id: &ActorId,
    name: &str,
    delay: f64,
) -> Result<(), Q1Error> {
    let _ = context;
    let action = game.named.action(&format!("{OLDNEW_PREFIX}{name}"))?;
    game.schedule(id, delay, &action)
}

/// Read the boss enemy (`bossEnemy`).
pub fn boss_enemy(_context: &Q1AddonContext, game: &Q1EntityServices, id: &ActorId) -> Option<ActorId> {
    game.entity_ref(id).and_then(|entity| {
        entity
            .monster
            .as_ref()
            .and_then(|monster| monster.enemy.clone())
            .or_else(|| entity.references.get("enemy").and_then(|enemy| enemy.clone()))
    })
}

/// Resolve worldspawn for boss damage (`bossWorld`).
pub fn boss_world(game: &Q1EntityServices) -> Result<ActorId, Q1Error> {
    game.world
        .clone()
        .ok_or_else(|| q1_error("MG3 boss damage requires worldspawn"))
}

/// Read the boss target origin (`bossTarget`).
pub fn boss_target(context: &Q1AddonContext, game: &Q1EntityServices, id: &ActorId) -> Vec3 {
    let _ = context;
    boss_enemy(context, game, id)
        .and_then(|enemy| game.host.bodies.read(&enemy).map(|body| body.origin))
        .unwrap_or(ZERO)
}

/// Cross product (`cross`).
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.y * b.z - a.z * b.y,
        y: a.z * b.x - a.x * b.z,
        z: a.x * b.y - a.y * b.x,
    }
}

fn flat_dot(a: Vec3, b: Vec3) -> f64 {
    f64::from(dot(
        normalize(Vec3 { x: a.x, y: a.y, z: 0.0 }),
        normalize(Vec3 { x: b.x, y: b.y, z: 0.0 }),
    ))
}

fn missile(
    game: &mut Q1EntityServices,
    manager: &ActorId,
    origin: Vec3,
    direction: Vec3,
    model: &str,
    velocity: Vec3,
) -> Result<ActorId, Q1Error> {
    let owner = game.entity_ref(manager).and_then(|entity| entity.owner.clone());
    let shot = launch_spike(game, owner.as_ref(), origin, direction, SpikeKind::Spike)?;
    let touch = game.named.touch(&format!("{OLDNEW_PREFIX}sphere_mis_touch"))?;
    game.update_entity(&shot, |entity| {
        entity.model = model.to_string();
        entity.touch = Some(touch);
    })?;
    game.set_body(
        &shot,
        &BodyPatch {
            velocity: Some(velocity),
            bounds: Some(POINT),
            ..Default::default()
        },
    )?;
    Ok(shot)
}

/// Spawn a sphere manager (`spawnSphereManager`).
pub fn spawn_sphere_manager(
    context: &Q1AddonContext,
    game: &mut Q1EntityServices,
    source: &ActorId,
    maximum: f64,
) -> Result<ActorId, Q1Error> {
    let manager = game.create("sphere_manager", None, None)?;
    game.update_entity(&manager, |entity| {
        entity.classname = String::new();
        entity.owner = Some(source.clone());
    })?;
    let origin = game.body(source).map(|body| body.origin)?;
    game.set_origin(&manager, origin)?;
    if boss_enemy(context, game, source).is_none() {
        let player = (game.host.players)().first().cloned();
        game.update_entity(source, |entity| {
            if let Some(monster) = entity.monster.as_mut() {
                monster.enemy = player.clone();
            } else {
                entity.references.insert(String::from("enemy"), player);
            }
        })?;
    }
    let target = boss_target(context, game, source);
    let source_origin = game.body(source).map(|body| body.origin)?;
    let direction = normalize(Vec3 {
        x: target.x - source_origin.x,
        y: target.y - source_origin.y,
        z: 0.0,
    });
    let angles = game.body(source).map(|body| body.angles)?;
    let basis = game.make_vectors(angles);
    let side = flat_dot(basis.right, cross(direction, Vec3 { x: 0.0, y: 0.0, z: 1.0 })) > 1.0;
    set_addon_number(game, &manager, "aflag", if side { 1.0 } else { 0.0 })?;
    game.set_body(
        &manager,
        &BodyPatch {
            angles: Some(velocity_angles(direction)),
            ..Default::default()
        },
    )?;
    game.update_entity(&manager, |entity| {
        entity.count = maximum;
    })?;
    boss_later(context, game, &manager, "actual_sphere", 0.1)?;
    Ok(manager)
}

/// Spawn a sphere chunk manager (`spawnSphereChunkManager`).
pub fn spawn_sphere_chunk_manager(
    context: &Q1AddonContext,
    game: &mut Q1EntityServices,
    source: &ActorId,
    maximum: f64,
) -> Result<ActorId, Q1Error> {
    let manager = game.create("sphere_chunk_manager", None, None)?;
    game.update_entity(&manager, |entity| {
        entity.classname = String::new();
        entity.owner = Some(source.clone());
        entity.wait = 22.5;
        entity.delay = 0.8;
        entity.count = 0.0;
    })?;
    set_addon_number(game, &manager, "aflag", 1.0)?;
    set_addon_number(game, &manager, "cnt", maximum)?;
    let enemy = boss_enemy(context, game, source).or_else(|| (game.host.players)().first().cloned());
    game.update_entity(&manager, |entity| {
        entity.references.insert(String::from("enemy"), enemy);
    })?;
    let (origin, angles) = game.body(source).map(|body| (body.origin, body.angles))?;
    game.set_body(
        &manager,
        &BodyPatch {
            origin: Some(origin),
            angles: Some(angles),
            ..Default::default()
        },
    )?;
    boss_later(context, game, &manager, "sphere_chunk", 0.1)?;
    Ok(manager)
}

/// Fire an auto gun spike (`autoGun`).
pub fn auto_gun(
    context: &Q1AddonContext,
    game: &mut Q1EntityServices,
    source: &ActorId,
    origin: Vec3,
    offset: f64,
) -> Result<(), Q1Error> {
    let _ = context;
    let player = (game.host.players)().first().cloned();
    let target = player
        .as_ref()
        .and_then(|player| game.host.bodies.read(player).map(|body| body.origin))
        .unwrap_or(ZERO);
    let mut direction = normalize(vsub(target, origin));
    if offset != 0.0 {
        direction = vadd(
            vscale(cross(direction, Vec3 { x: 0.0, y: 0.0, z: 1.0 }), offset),
            vscale(direction, 1.0 - offset.abs()),
        );
    }
    let shot = launch_spike(game, Some(source), origin, direction, SpikeKind::Spike)?;
    let touch = game.named.touch(&format!("{OLDNEW_PREFIX}sphere_mis_touch"))?;
    game.update_entity(&shot, |entity| {
        entity.touch = Some(touch);
        entity.model = String::from("progs/rogue/sphere.mdl");
        entity.effects = 64;
    })?;
    game.set_body(
        &shot,
        &BodyPatch {
            velocity: Some(vscale(direction, if offset == 0.0 { 600.0 } else { 800.0 })),
            bounds: Some(POINT),
            ..Default::default()
        },
    )
}

/// Spawn a boss teledeath (`spawnBossTeledeath`).
pub fn spawn_boss_teledeath(
    context: &Q1AddonContext,
    game: &mut Q1EntityServices,
    origin: Vec3,
    owner: &ActorId,
) -> Result<ActorId, Q1Error> {
    let _ = context;
    let death = game.create("teledeath", None, None)?;
    let bounds = game.body(owner).map(|body| body.bounds)?;
    let touch = game.named.touch(&format!("{OLDNEW_PREFIX}tdeath_boss_touch"))?;
    game.update_entity(&death, |entity| {
        entity.owner = Some(owner.clone());
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::None;
        entity.touch = Some(touch);
    })?;
    game.set_body(
        &death,
        &BodyPatch {
            origin: Some(origin),
            angles: Some(ZERO),
            bounds: Some(Bounds {
                min: vsub(bounds.min, Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
                max: vadd(bounds.max, Vec3 { x: 1.0, y: 1.0, z: 1.0 }),
            }),
            ..Default::default()
        },
    )?;
    let remove = game.named.action("SUB_Remove")?;
    game.schedule(&death, 0.2, &remove)?;
    game.link(&death)?;
    game.force_retouch = 2;
    Ok(death)
}

fn sphere_mis_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let entity = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if entity.owner.as_ref().is_some_and(|owner| same_actor(owner, other)) {
        return Ok(());
    }
    let target = game.entity_ref(other).cloned();
    if target.as_ref().is_some_and(|target| {
        target.classname == "oldnew_child" || target.classname == "oldnew_eye" || target.classname == "monster_szombie"
    }) {
        return game.remove(id);
    }
    if target.as_ref().is_some_and(|target| target.solid == Q1Solid::Trigger) {
        return Ok(());
    }
    if game.host.contents(game.body(id).map(|body| body.origin)?) == Q1Contents::Sky {
        return game.remove(id);
    }
    if game
        .host
        .combat
        .read(other)
        .is_some_and(|combat| combat.can_take_damage)
    {
        game.damage(other, Some(id), entity.owner.as_ref(), 18.0, &Q1DamageParams::default());
    }
    game.remove(id)
}

fn actual_sphere(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game
        .entity_ref(id)
        .and_then(|entity| {
            entity
                .monster
                .as_ref()
                .and_then(|monster| monster.enemy.clone())
                .or_else(|| entity.references.get("enemy").and_then(|enemy| enemy.clone()))
        })
        .is_none()
    {
        let player = (game.host.players)().first().cloned();
        game.update_entity(id, |entity| {
            entity.references.insert(String::from("enemy"), player);
        })?;
    }
    let angles = game.body(id).map(|body| body.angles)?;
    let direction = game.make_vectors(angles).forward;
    let origin = game.body(id).map(|body| body.origin)?;
    for i in 0..100 {
        let point = sphere_point(i)?;
        let aim = vadd(vscale(direction, 0.65), vscale(normalize(point), 0.35));
        let shot = missile(
            game,
            id,
            vadd(
                vadd(
                    origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 32.0,
                    },
                ),
                vscale(point, 32.0),
            ),
            aim,
            "progs/rogue/sphere.mdl",
            vscale(aim, 800.0),
        )?;
        game.update_entity(&shot, |entity| {
            entity.damage = 18.0;
            if i % 5 == 0 {
                entity.effects = 64;
            }
        })?;
    }
    game.update_entity(id, |entity| {
        entity.count -= 1.0;
    })?;
    if game.entity_ref(id).map(|entity| entity.count).unwrap_or(0.0) <= 0.0 {
        return game.remove(id);
    }
    let action = game.named.action(&format!("{OLDNEW_PREFIX}actual_sphere"))?;
    game.schedule(id, 0.8, &action)?;
    let step = if game.entity_ref(id).map(|entity| entity.number("aflag")).unwrap_or(0.0) != 0.0 {
        20.0
    } else {
        -20.0
    };
    let angles = game.body(id).map(|body| body.angles)?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(vadd(
                angles,
                Vec3 {
                    x: 0.0,
                    y: step,
                    z: 0.0,
                },
            )),
            ..Default::default()
        },
    )
}

fn spawn_sphere(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let angles = game.body(id).map(|body| body.angles)?;
    let initial = game.make_vectors(angles);
    let (excluded, excluded2) = (initial.forward, initial.right);
    game.sound(id, "weapons/spike2.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    if game
        .entity_ref(id)
        .and_then(|entity| {
            entity
                .monster
                .as_ref()
                .and_then(|monster| monster.enemy.clone())
                .or_else(|| entity.references.get("enemy").and_then(|enemy| enemy.clone()))
        })
        .is_none()
    {
        let player = (game.host.players)().first().cloned();
        game.update_entity(id, |entity| {
            entity.references.insert(String::from("enemy"), player);
        })?;
    }
    for y in -1..2 {
        for x in 0..(if y == 0 { 44 } else { 45 }) {
            let angles = game.body(id).map(|body| body.angles)?;
            let basis = game.make_vectors(vadd(
                angles,
                Vec3 {
                    x: 0.0,
                    y: (f64::from(x) * 8.0) as f32,
                    z: 0.0,
                },
            ));
            if flat_dot(basis.forward, excluded).abs() >= 0.9 || flat_dot(basis.forward, excluded2).abs() >= 0.9 {
                continue;
            }
            let origin = game.body(id).map(|body| body.origin)?;
            let shot = missile(
                game,
                id,
                vadd(
                    vadd(
                        vadd(origin, vscale(basis.forward, 64.0)),
                        vscale(basis.up, f64::from(y) * 16.0),
                    ),
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 24.0,
                    },
                ),
                vscale(basis.forward, 200.0),
                "progs/diamond.mdl",
                vscale(normalize(basis.forward), 400.0),
            )?;
            game.update_entity(&shot, |entity| {
                if y == -1 && x % 2 == 0 {
                    entity.effects = 64;
                }
                entity.angular_velocity = Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: -100.0,
                };
            })?;
        }
        let angles = game.body(id).map(|body| body.angles)?;
        game.set_body(
            id,
            &BodyPatch {
                angles: Some(vadd(angles, Vec3 { x: 0.0, y: 4.0, z: 0.0 })),
                ..Default::default()
            },
        )?;
    }
    let (aflag, wait, delay) = game
        .entity_ref(id)
        .map(|entity| (entity.number("aflag"), entity.wait, entity.delay))
        .unwrap_or((0.0, 0.0, 0.0));
    let angles = game.body(id).map(|body| body.angles)?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(vadd(
                angles,
                Vec3 {
                    x: 0.0,
                    y: (aflag * wait) as f32,
                    z: 0.0,
                },
            )),
            ..Default::default()
        },
    )?;
    let action = game.named.action(&format!("{OLDNEW_PREFIX}spawn_sphere"))?;
    game.schedule(id, delay, &action)?;
    game.update_entity(id, |entity| {
        entity.count += 1.0;
    })?;
    let (count, cnt) = game
        .entity_ref(id)
        .map(|entity| (entity.count, entity.number("cnt")))
        .unwrap_or((0.0, 0.0));
    if count > cnt {
        let owner = game.entity_ref(id).and_then(|entity| entity.owner.clone());
        if let Some(owner) = owner
            .as_ref()
            .and_then(|owner| game.entity_ref(owner).map(|_| owner.clone()))
        {
            if game
                .entity_ref(&owner)
                .map(|entity| entity.number("boss_immune"))
                .unwrap_or(0.0)
                != 0.0
            {
                set_addon_number(game, &owner, "boss_immune", 0.0)?;
            }
        }
        return game.remove(id);
    }
    Ok(())
}

fn sphere_chunk(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let target = game
        .entity_ref(id)
        .and_then(|entity| {
            entity
                .monster
                .as_ref()
                .and_then(|monster| monster.enemy.clone())
                .or_else(|| entity.references.get("enemy").and_then(|enemy| enemy.clone()))
        })
        .and_then(|enemy| game.host.bodies.read(&enemy).map(|body| body.origin))
        .unwrap_or(ZERO);
    let origin = game.body(id).map(|body| body.origin)?;
    let attack = normalize(Vec3 {
        x: target.x - origin.x,
        y: target.y - origin.y,
        z: 0.0,
    });
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(velocity_angles(attack)),
            ..Default::default()
        },
    )?;
    let angles = game.body(id).map(|body| body.angles)?;
    let excluded = game
        .make_vectors(vadd(
            angles,
            Vec3 {
                x: 0.0,
                y: 22.5,
                z: 0.0,
            },
        ))
        .forward;
    let excluded2 = game
        .make_vectors(vadd(
            angles,
            Vec3 {
                x: 0.0,
                y: -22.5,
                z: 0.0,
            },
        ))
        .forward;
    game.sound(id, "weapons/spike2.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    for y in -1..1 {
        for x in 0..45 {
            let angles = game.body(id).map(|body| body.angles)?;
            let basis = game.make_vectors(vadd(
                angles,
                Vec3 {
                    x: 0.0,
                    y: (f64::from(x) * 8.0) as f32,
                    z: 0.0,
                },
            ));
            if flat_dot(basis.forward, attack) <= 0.5
                || flat_dot(basis.forward, excluded) > 0.985
                || flat_dot(basis.forward, excluded2) > 0.985
            {
                continue;
            }
            let origin = game.body(id).map(|body| body.origin)?;
            let shot = missile(
                game,
                id,
                vadd(
                    vadd(
                        vadd(origin, vscale(basis.forward, 64.0)),
                        vscale(basis.up, f64::from(y) * 16.0),
                    ),
                    Vec3 { x: 0.0, y: 0.0, z: 8.0 },
                ),
                vscale(basis.forward, 200.0),
                "progs/diamond.mdl",
                vscale(normalize(basis.forward), 400.0),
            )?;
            game.update_entity(&shot, |entity| {
                if y == 0 && x % 2 == 0 {
                    entity.effects = 64;
                }
                entity.angular_velocity = Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: -100.0,
                };
            })?;
        }
        let angles = game.body(id).map(|body| body.angles)?;
        game.set_body(
            id,
            &BodyPatch {
                angles: Some(vadd(angles, Vec3 { x: 0.0, y: 4.0, z: 0.0 })),
                ..Default::default()
            },
        )?;
    }
    game.update_entity(id, |entity| {
        entity.count += 1.0;
    })?;
    let (count, cnt) = game
        .entity_ref(id)
        .map(|entity| (entity.count, entity.number("cnt")))
        .unwrap_or((0.0, 0.0));
    if count > cnt {
        game.remove(id)
    } else {
        Ok(())
    }
}

fn tdeath_boss_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    let entity = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if entity.owner.as_ref().is_some_and(|owner| same_actor(owner, other)) {
        return Ok(());
    }
    if game.is_player(other) || game.host.classname(other) == "monster_oldone_new" {
        if game
            .player_ref(other)
            .and_then(|player| player.powerups.get(&Q1Powerup::Invulnerability).copied())
            .unwrap_or(0.0)
            > game.time
        {
            game.update_entity(id, |entity| {
                entity.classname = String::from("teledeath2");
            })?;
        }
        if !entity.owner.as_ref().is_some_and(|owner| game.is_player(owner)) {
            if let Some(owner) = entity.owner.clone() {
                game.damage(&owner, Some(id), Some(id), 50000.0, &Q1DamageParams::default());
            }
            return Ok(());
        }
    }
    if game.health(other) != 0.0 {
        game.damage(other, Some(id), Some(id), 50000.0, &Q1DamageParams::default());
    }
    Ok(())
}

/// Register Old One projectile touches and managers
/// (`registerOldnewProjectiles`).
pub fn register_oldnew_projectiles(_context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        &format!("{OLDNEW_PREFIX}sphere_mis_touch"),
        Q1CallbackHandlers {
            touch: Some(sphere_mis_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}actual_sphere"),
        Q1CallbackHandlers {
            action: Some(actual_sphere as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}spawn_sphere"),
        Q1CallbackHandlers {
            action: Some(spawn_sphere as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}sphere_chunk"),
        Q1CallbackHandlers {
            action: Some(sphere_chunk as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}tdeath_boss_touch"),
        Q1CallbackHandlers {
            touch: Some(tdeath_boss_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    Ok(())
}
