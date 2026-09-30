//! Q1 reborn Old One children (`src/content/q1/addons/monsters/bosses/oldnew-children.ts`).
//!
//! `mg3_oldone_new.qc` summoned attack actors. GPL-2.0-or-later.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::context::{addon_emit, set_addon_number, Q1AddonContext, Q1AddonEvent, Q1AddonLightningStyle};
use crate::q1::base::projectiles::{launch_spike, SpikeKind};
use crate::q1::foundation::callbacks::{
    Q1ActionHandler, Q1CallbackHandlers, Q1DieHandler, Q1PainHandler, Q1TouchHandler,
};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::spawns::spawn_teleport_fog;
use crate::q1::foundation::types::{
    length, normalize, vadd, vscale, vsub, Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, Q1TraceRequest, POINT, ZERO,
};
use crate::q1::missionpacks::types::velocity_angles;
use crate::q1::{q1_error, Q1Error};

use super::oldnew_projectiles::{boss_enemy, boss_later, boss_world, cross, spawn_boss_teledeath, OLDNEW_PREFIX};

const LARGE_HULL: Bounds = Bounds {
    min: Vec3 {
        x: -32.0,
        y: -32.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 32.0,
        y: 32.0,
        z: 64.0,
    },
};

fn child(game: &mut Q1EntityServices, owner: &ActorId) -> Result<ActorId, Q1Error> {
    let entity = game.create("oldnew_child", None, None)?;
    let origin = game.body(owner).map(|body| body.origin)?;
    game.update_entity(&entity, |entity| {
        entity.owner = Some(owner.clone());
    })?;
    game.set_origin(&entity, origin)?;
    Ok(entity)
}

/// Spawn a spammer (`spawnSpammer`).
pub fn spawn_spammer(
    context: &Q1AddonContext,
    game: &mut Q1EntityServices,
    owner: &ActorId,
) -> Result<ActorId, Q1Error> {
    let entity = child(game, owner)?;
    let enemy = boss_enemy(context, game, owner).or_else(|| (game.host.players)().first().cloned());
    game.update_entity(&entity, |entity| {
        entity.references.insert(String::from("enemy"), enemy);
    })?;
    boss_later(context, game, &entity, "spammer_think", 0.1)?;
    Ok(entity)
}

/// Spawn a swiper (`spawnSwiper`).
pub fn spawn_swiper(
    context: &Q1AddonContext,
    game: &mut Q1EntityServices,
    owner: &ActorId,
) -> Result<ActorId, Q1Error> {
    let entity = child(game, owner)?;
    let angles = game.body(owner).map(|body| body.angles)?;
    game.update_entity(&entity, |entity| {
        entity.count = 0.0;
    })?;
    game.set_body(
        &entity,
        &BodyPatch {
            angles: Some(angles),
            ..Default::default()
        },
    )?;
    game.sound(&entity, "weapons/lstart.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    let aflag = game
        .entity_ref(owner)
        .map(|entity| entity.number("aflag"))
        .unwrap_or(0.0);
    set_addon_number(game, &entity, "aflag", aflag)?;
    set_addon_number(game, owner, "aflag", 1.0 - aflag)?;
    boss_later(context, game, &entity, "oldnew_swipe", 0.025)?;
    Ok(entity)
}

/// Spawn an eye (`spawnEye`).
pub fn spawn_eye(context: &Q1AddonContext, game: &mut Q1EntityServices, owner: &ActorId) -> Result<ActorId, Q1Error> {
    let entity = game.create("oldnew_eye", None, None)?;
    let origin = vadd(
        game.body(owner).map(|body| body.origin)?,
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 300.0,
        },
    );
    spawn_teleport_fog(game, origin)?;
    game.set_body(
        &entity,
        &BodyPatch {
            origin: Some(origin),
            bounds: Some(LARGE_HULL),
            ..Default::default()
        },
    )?;
    let pain = game.named.pain(&format!("{OLDNEW_PREFIX}eye_pain"))?;
    let die = game.named.die(&format!("{OLDNEW_PREFIX}eye_die"))?;
    let touch = game.named.touch(&format!("{OLDNEW_PREFIX}eye_touch"))?;
    game.update_entity(&entity, |entity| {
        entity.model = String::from("progs/teleporter_eye.mdl");
        entity.solid = Q1Solid::Slidebox;
        entity.movement = Q1MoveType::Fly;
        entity.aimed_damage = true;
        entity.pain = Some(pain);
        entity.die = Some(die);
        entity.touch = Some(touch);
        entity.owner = Some(owner.clone());
        entity.speed = 100.0;
        entity.max_health = 300.0;
    })?;
    game.set_damageable(&entity, true)?;
    let owned = game
        .entity_ref(&entity)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    game.host.combat.set_health(&owned, 300.0)?;
    let player = (game.host.players)().first().cloned();
    game.update_entity(&entity, |entity| {
        entity.references.insert(String::from("enemy"), player);
    })?;
    boss_later(context, game, &entity, "eye_chase", 0.1)?;
    game.link(&entity)?;
    Ok(entity)
}

fn armed_eye(
    context: &Q1AddonContext,
    game: &mut Q1EntityServices,
    owner: &ActorId,
    offset: f64,
) -> Result<ActorId, Q1Error> {
    let entity = child(game, owner)?;
    let angles = game.body(owner).map(|body| body.angles)?;
    let basis = game.make_vectors(angles);
    let origin = vadd(
        vadd(
            vadd(
                game.body(owner).map(|body| body.origin)?,
                vscale(basis.right, offset * 300.0),
            ),
            vscale(basis.up, 80.0),
        ),
        vscale(basis.forward, 200.0),
    );
    game.set_body(
        &entity,
        &BodyPatch {
            origin: Some(origin),
            bounds: Some(LARGE_HULL),
            ..Default::default()
        },
    )?;
    let die = game.named.die(&format!("{OLDNEW_PREFIX}eye_die"))?;
    game.update_entity(&entity, |entity| {
        entity.model = String::from("progs/teleporter_eye.mdl");
        entity.solid = Q1Solid::Slidebox;
        entity.movement = Q1MoveType::Step;
        entity.movement_flags = 33;
        entity.die = Some(die);
    })?;
    spawn_boss_teledeath(context, game, origin, &entity)?;
    let owned = game
        .entity_ref(&entity)
        .map(|entity| entity.actor.clone())
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    game.host.combat.set_health(&owned, 120.0)?;
    game.update_entity(&entity, |entity| {
        entity.max_health = 120.0;
        entity.aimed_damage = true;
    })?;
    game.set_damageable(&entity, true)?;
    game.total_monsters += 1;
    let total = game.total_monsters;
    game.host.emit(Q1Event::MonsterTotal { total });
    let angles = game.body(owner).map(|body| body.angles)?;
    game.set_body(
        &entity,
        &BodyPatch {
            angles: Some(angles),
            ..Default::default()
        },
    )?;
    spawn_teleport_fog(game, origin)?;
    game.update_entity(&entity, |entity| {
        entity.effects = 64;
    })?;
    game.link(&entity)?;
    Ok(entity)
}

/// Spawn blasters (`spawnBlaster`).
pub fn spawn_blaster(context: &Q1AddonContext, game: &mut Q1EntityServices, owner: &ActorId) -> Result<(), Q1Error> {
    for offset in [1.0, -1.0] {
        let entity = armed_eye(context, game, owner, offset)?;
        set_addon_number(game, &entity, "aflag", offset)?;
        game.update_entity(&entity, |entity| {
            entity.count = 0.0;
            entity.damage = -offset;
        })?;
        boss_later(context, game, &entity, "blast", 2.0)?;
    }
    Ok(())
}

/// Spawn a vortex (`spawnVortex`).
pub fn spawn_vortex(
    context: &Q1AddonContext,
    game: &mut Q1EntityServices,
    owner: &ActorId,
) -> Result<ActorId, Q1Error> {
    let side = if game
        .entity_ref(owner)
        .map(|entity| entity.number("ammo_cells"))
        .unwrap_or(0.0)
        != 0.0
    {
        1.0
    } else {
        -1.0
    };
    let entity = armed_eye(context, game, owner, side)?;
    set_addon_number(game, &entity, "aflag", side)?;
    set_addon_number(
        game,
        owner,
        "ammo_cells",
        1.0 - game
            .entity_ref(owner)
            .map(|entity| entity.number("ammo_cells"))
            .unwrap_or(0.0),
    )?;
    let enemy = boss_enemy(context, game, owner);
    game.update_entity(&entity, |entity| {
        entity.count = 0.0;
        entity.references.insert(String::from("enemy"), enemy);
    })?;
    boss_later(context, game, &entity, "vortex_think_alt", 2.0)?;
    Ok(entity)
}

/// Clean up Old One children (`cleanupOldnew`).
pub fn cleanup_oldnew(context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    for id in game.entity_ids() {
        let classname = game
            .entity_ref(&id)
            .map(|entity| entity.classname.clone())
            .unwrap_or_default();
        if classname == "oldnew_child" || classname == "oldnew_eye" {
            boss_later(context, game, &id, "oldnew_cleanup_think", 0.1)?;
        }
    }
    let timer = game.create("oldnew_cleanup_zombies", None, None)?;
    game.update_entity(&timer, |entity| {
        entity.classname = String::new();
    })?;
    boss_later(context, game, &timer, "oldnew_cleanup_zombies", 2.0)
}

fn eye_pain(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _attacker: Option<&ActorId>,
    _damage: f64,
) -> Result<(), Q1Error> {
    game.sound(id, "misc/power.wav", Q1SoundChannel::Voice, 1.0, 1.0)?;
    let origin = game.body(id).map(|body| body.origin)?;
    addon_emit(
        game,
        Q1AddonEvent::Lightning {
            actor: id.clone(),
            style: Q1AddonLightningStyle::Style1,
            start: origin,
            end: vadd(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 200.0,
                },
            ),
        },
    )
}

fn eye_die(game: &mut Q1EntityServices, id: &ActorId, _attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    let origin = game.body(id).map(|body| body.origin)?;
    addon_emit(
        game,
        Q1AddonEvent::ColoredExplosion {
            origin,
            color_start: 244,
            color_length: 3,
        },
    )?;
    let ignore = game.world.clone();
    game.radius_damage(id, Some(id), 100.0, ignore.as_ref(), None, "");
    game.remove(id)
}

fn eye_touch(
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
    if game.entity_ref(other).map(|_| other.clone()) == game.world
        || entity.owner.as_ref().is_some_and(|owner| same_actor(owner, other))
    {
        return Ok(());
    }
    if game.health(other) != 0.0 {
        game.damage(
            other,
            Some(id),
            entity.owner.as_ref(),
            500.0,
            &Q1DamageParams::default(),
        );
    }
    Ok(())
}

fn eye_chase(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let body = game.body(id)?;
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
    let speed = f64::from(length(body.velocity));
    let speed = if speed < 300.0 { speed + 10.0 } else { speed };
    game.update_entity(id, |entity| {
        entity.speed = speed;
    })?;
    let mut velocity = vadd(
        vscale(normalize(vsub(target, body.origin)), 0.3),
        vscale(normalize(body.velocity), 0.7),
    );
    if f64::from(body.origin.z) < f64::from(target.z) + 32.0 {
        velocity = normalize(Vec3 {
            x: velocity.x,
            y: velocity.y,
            z: 0.0,
        });
    }
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(velocity_angles(velocity)),
            velocity: Some(vscale(velocity, speed)),
            ..Default::default()
        },
    )?;
    let action = game.named.action(&format!("{OLDNEW_PREFIX}eye_chase"))?;
    game.schedule(id, 0.1, &action)?;
    if game.entity_ref(id).map(|entity| entity.count).unwrap_or(0.0) % 2.0 == 0.0 {
        game.update_entity(id, |entity| {
            entity.effects |= 2;
        })?;
        game.sound_simple(id, "misc/power.wav")?;
    }
    game.update_entity(id, |entity| {
        entity.count += 1.0;
    })
}

fn blast(game: &mut Q1EntityServices, id: &ActorId, name: &str) -> Result<(), Q1Error> {
    let enemy = game.entity_ref(id).and_then(|entity| {
        entity
            .monster
            .as_ref()
            .and_then(|monster| monster.enemy.clone())
            .or_else(|| entity.references.get("enemy").and_then(|enemy| enemy.clone()))
    });
    if enemy.as_ref().is_none_or(|enemy| !game.is_player(enemy)) {
        let player = (game.host.players)().first().cloned();
        game.update_entity(id, |entity| {
            entity.references.insert(String::from("enemy"), player);
        })?;
    }
    let body = game.body(id)?;
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
    let mut direction = normalize(vsub(target, body.origin));
    let (count, aflag) = game
        .entity_ref(id)
        .map(|entity| (entity.count, entity.number("aflag")))
        .unwrap_or((0.0, 0.0));
    direction = normalize(vadd(
        direction,
        vscale(
            cross(direction, Vec3 { x: 0.0, y: 0.0, z: 1.0 }),
            (count * 15.0).cos() * 0.5 * aflag,
        ),
    ));
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(velocity_angles(direction)),
            ..Default::default()
        },
    )?;
    let shot = launch_spike(
        game,
        Some(id),
        vadd(body.origin, vscale(direction, 8.0)),
        direction,
        SpikeKind::Spike,
    )?;
    let touch = game.named.touch(&format!("{OLDNEW_PREFIX}blast_touch"))?;
    game.update_entity(&shot, |entity| {
        entity.touch = Some(touch);
        entity.model = String::from("progs/rogue/sphere.mdl");
        entity.effects = 64;
    })?;
    game.sound(id, "weapons/spike2.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    game.set_body(
        &shot,
        &BodyPatch {
            velocity: Some(vscale(direction, 500.0)),
            bounds: Some(POINT),
            ..Default::default()
        },
    )?;
    let spin = game.host.random() * 2.0 - 1.0;
    game.update_entity(&shot, |entity| {
        entity.angular_velocity = vscale(
            Vec3 {
                x: 300.0,
                y: 300.0,
                z: 300.0,
            },
            spin,
        );
    })?;
    let action = game.named.action(&format!("{OLDNEW_PREFIX}{name}"))?;
    game.schedule(id, 0.2, &action)?;
    game.update_entity(id, |entity| {
        entity.count += 1.0;
    })?;
    if game.entity_ref(id).map(|entity| entity.count).unwrap_or(0.0) > 72.0 {
        let world = boss_world(game)?;
        game.damage(id, Some(&world), Some(&world), 500.0, &Q1DamageParams::default());
    }
    Ok(())
}

fn blast_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    blast(game, id, "blast")
}

fn vortex_think_alt_handler(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    blast(game, id, "vortex_think_alt")
}

fn blast_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&crate::q1::foundation::gameplay::TouchSurface>,
) -> Result<(), Q1Error> {
    if game.entity_ref(other).map(|_| other.clone()) == game.world {
        return game.remove(id);
    }
    let entity = game
        .entity_ref(id)
        .cloned()
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if game.host.classname(other) == "sphere" || entity.owner.as_ref().is_some_and(|owner| same_actor(owner, other)) {
        return Ok(());
    }
    if game.host.classname(other) == "monster_oldone_new" {
        return game.remove(id);
    }
    if game
        .entity_ref(other)
        .is_some_and(|target| target.solid == Q1Solid::Trigger)
    {
        return Ok(());
    }
    if game.health(other) != 0.0 {
        game.damage(other, Some(id), entity.owner.as_ref(), 15.0, &Q1DamageParams::default());
    }
    game.remove(id)
}

fn spammer_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let owner = game.entity_ref(id).and_then(|entity| entity.owner.clone());
    let angles = owner
        .as_ref()
        .and_then(|owner| game.body(owner).map(|body| body.angles).ok())
        .unwrap_or(ZERO);
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(angles),
            ..Default::default()
        },
    )?;
    const OFFSETS: [i32; 13] = [0, 1, -3, 2, -2, 3, -1, 4, 2, -1, -3, 2, -4];
    let count = game.entity_ref(id).map(|entity| entity.count).unwrap_or(0.0) as usize;
    let Some(offset) = OFFSETS.get(count).copied() else {
        return game.remove(id);
    };
    let basis = game.make_vectors(vadd(
        angles,
        Vec3 {
            x: 0.0,
            y: (f64::from(offset) * 6.0) as f32,
            z: 0.0,
        },
    ));
    let shot = game.create("spam", None, None)?;
    let origin = vadd(
        vadd(
            game.body(id).map(|body| body.origin)?,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 50.0,
            },
        ),
        vscale(basis.forward, 48.0),
    );
    let touch = game.named.touch(&format!("{OLDNEW_PREFIX}spam_touch"))?;
    let owner = game.entity_ref(id).and_then(|entity| entity.owner.clone());
    game.update_entity(&shot, |entity| {
        entity.owner = owner;
        entity.model = String::from("progs/rogue/plasma.mdl");
        entity.solid = Q1Solid::Bbox;
        entity.movement = Q1MoveType::Toss;
        entity.effects = 64;
        entity.touch = Some(touch);
    })?;
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
    let origin_here = game.body(id).map(|body| body.origin)?;
    let oomph = (f64::from(length(vsub(target, origin_here))) * 0.8).max(200.0);
    let velocity = vscale(
        basis.forward,
        oomph + 25.0 * game.entity_ref(id).map(|entity| entity.count).unwrap_or(0.0),
    );
    game.set_body(
        &shot,
        &BodyPatch {
            origin: Some(origin),
            bounds: Some(POINT),
            velocity: Some(Vec3 {
                x: velocity.x,
                y: velocity.y,
                z: 200.0,
            }),
            ..Default::default()
        },
    )?;
    game.sound(id, "weapons/grenade.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
    game.update_entity(id, |entity| {
        entity.count += 1.0;
    })?;
    game.link(&shot)?;
    let action = game.named.action(&format!("{OLDNEW_PREFIX}spammer_think"))?;
    game.schedule(id, 0.1, &action)
}

fn spam_touch(
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
    if game.entity_ref(other).map(|_| other.clone()) == game.world {
        let spin = (game.host.random() * 360.0) as f32;
        game.set_body(
            id,
            &BodyPatch {
                velocity: Some(ZERO),
                angles: Some(Vec3 {
                    x: 0.0,
                    y: spin,
                    z: 0.0,
                }),
                ..Default::default()
            },
        )?;
        game.update_entity(id, |entity| {
            entity.movement = Q1MoveType::None;
            entity.solid = Q1Solid::None;
            entity.model = String::from("maps/bmodel/b_splash.bsp");
            entity.touch = None;
        })?;
        game.link(id)?;
        let action = game.named.action(&format!("{OLDNEW_PREFIX}spam1"))?;
        return game.schedule(id, 2.0, &action);
    }
    if game.health(other) != 0.0 {
        let world = boss_world(game)?;
        game.damage(
            other,
            entity.owner.as_ref().or(Some(&world)),
            Some(id),
            10.0,
            &Q1DamageParams::default(),
        );
    }
    game.remove(id)
}

fn spam1(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let origin = game.body(id).map(|body| body.origin)?;
    addon_emit(
        game,
        Q1AddonEvent::Lightning {
            actor: id.clone(),
            style: Q1AddonLightningStyle::Style1,
            start: origin,
            end: vadd(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 500.0,
                },
            ),
        },
    )?;
    let action = game.named.action(&format!("{OLDNEW_PREFIX}spam2"))?;
    game.schedule(id, 0.1, &action)
}

fn spam2(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let origin = game.body(id).map(|body| body.origin)?;
    addon_emit(
        game,
        Q1AddonEvent::ColoredExplosion {
            origin,
            color_start: 244,
            color_length: 3,
        },
    )?;
    let ignore = game.world.clone();
    game.radius_damage(id, Some(id), 100.0, ignore.as_ref(), None, "");
    game.remove(id)
}

fn oldnew_swipe(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let count = game.entity_ref(id).map(|entity| entity.count).unwrap_or(0.0);
    let a = (count / 50.0).powi(2);
    let angles = game.body(id).map(|body| body.angles)?;
    let sweep = if game.entity_ref(id).map(|entity| entity.number("aflag")).unwrap_or(0.0) == 0.0 {
        -60.0 + a * 180.0
    } else {
        60.0 - a * 180.0
    };
    let basis = game.make_vectors(vadd(
        angles,
        Vec3 {
            x: 0.0,
            y: sweep as f32,
            z: 0.0,
        },
    ));
    let origin = game.body(id).map(|body| body.origin)?;
    let start = vadd(origin, vscale(basis.forward, 130.0));
    let trace = game.host.trace(&Q1TraceRequest {
        start,
        end: vadd(origin, vscale(basis.forward, 1000.0)),
        bounds: POINT,
        ignore: Some(id.clone()),
        monsters: true,
        missile: false,
    });
    if let Some(actor) = trace.actor.clone() {
        if game.health(&actor) != 0.0 {
            let world = boss_world(game)?;
            let owner = game.entity_ref(id).and_then(|entity| entity.owner.clone());
            let amount = if game.host.classname(&actor) == "monster_szombie" {
                100.0
            } else {
                25.0
            };
            game.damage(
                &actor,
                owner.as_ref().or(Some(&world)),
                owner.as_ref(),
                amount,
                &Q1DamageParams::default(),
            );
        }
    }
    if game.entity_ref(id).map(|entity| entity.count).unwrap_or(0.0) % 2.0 == 0.0 {
        addon_emit(
            game,
            Q1AddonEvent::Lightning {
                actor: id.clone(),
                style: Q1AddonLightningStyle::Style3,
                start,
                end: trace.end,
            },
        )?;
    }
    if game
        .entity_ref(id)
        .map(|entity| entity.number("t_width"))
        .unwrap_or(0.0)
        < game.time
    {
        game.sound(id, "weapons/lhit.wav", Q1SoundChannel::Weapon, 1.0, 1.0)?;
        let until = game.time + 0.6;
        set_addon_number(game, id, "t_width", until)?;
    }
    game.update_entity(id, |entity| {
        entity.count += 1.0;
    })?;
    if game.entity_ref(id).map(|entity| entity.count).unwrap_or(0.0) > 50.0 {
        let remove = game.named.action("SUB_Remove")?;
        game.schedule(id, 0.025, &remove)
    } else {
        let action = game.named.action(&format!("{OLDNEW_PREFIX}oldnew_swipe"))?;
        game.schedule(id, 0.025, &action)
    }
}

fn oldnew_cleanup_think(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.damage(id, Some(id), Some(id), 5000.0, &Q1DamageParams::default());
    Ok(())
}

fn oldnew_cleanup_zombies(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let zombie = game.entity_ids().into_iter().find(|candidate| {
        game.entity_ref(candidate)
            .is_some_and(|entity| entity.classname == "monster_szombie")
    });
    let action = game.named.action(&format!("{OLDNEW_PREFIX}oldnew_cleanup_zombies"))?;
    if let Some(zombie) = zombie {
        let world = boss_world(game)?;
        game.damage(&zombie, Some(&world), Some(&world), 500.0, &Q1DamageParams::default());
        let delay = 0.2 + game.host.random() * 0.5;
        game.schedule(id, delay, &action)
    } else {
        game.schedule(id, 1.5, &action)
    }
}

/// Register Old One child callbacks (`registerOldnewChildren`).
pub fn register_oldnew_children(_context: &Q1AddonContext, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        &format!("{OLDNEW_PREFIX}eye_pain"),
        Q1CallbackHandlers {
            pain: Some(eye_pain as Q1PainHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}eye_die"),
        Q1CallbackHandlers {
            die: Some(eye_die as Q1DieHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}eye_touch"),
        Q1CallbackHandlers {
            touch: Some(eye_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}eye_chase"),
        Q1CallbackHandlers {
            action: Some(eye_chase as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}blast"),
        Q1CallbackHandlers {
            action: Some(blast_handler as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}vortex_think_alt"),
        Q1CallbackHandlers {
            action: Some(vortex_think_alt_handler as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}blast_touch"),
        Q1CallbackHandlers {
            touch: Some(blast_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}spammer_think"),
        Q1CallbackHandlers {
            action: Some(spammer_think as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}spam_touch"),
        Q1CallbackHandlers {
            touch: Some(spam_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}spam1"),
        Q1CallbackHandlers {
            action: Some(spam1 as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}spam2"),
        Q1CallbackHandlers {
            action: Some(spam2 as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}oldnew_swipe"),
        Q1CallbackHandlers {
            action: Some(oldnew_swipe as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}oldnew_cleanup_think"),
        Q1CallbackHandlers {
            action: Some(oldnew_cleanup_think as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        &format!("{OLDNEW_PREFIX}oldnew_cleanup_zombies"),
        Q1CallbackHandlers {
            action: Some(oldnew_cleanup_zombies as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    Ok(())
}
