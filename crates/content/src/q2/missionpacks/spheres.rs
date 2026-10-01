//! Mission-pack spheres (`src/content/q2/missionpacks/spheres.ts`).
//!
//! Rogue g_sphere.c. Sphere reactions share the owner's combat and
//! movement authority.

use qa_core::identity::ActorId;
use qa_core::math::{add3, length3, normalize3, scale3, sub3, vec3, Vec3};

use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2Die, Q2Edition, Q2GameServices, Q2MotionKind, Q2Pain, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop,
    Q2Think, Q2Touch,
};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::weapons::vectors::vector_angles;
use crate::q2::support::contracts::{AttackProvenance, DeathReaction, PainReaction, TouchContact};

use super::projectiles::common::{explode, projectile_mask, publish_projectile, sight};
use super::projectiles::{mission_projectiles, Q2MissionPackProjectiles};
use super::types::{Q2MissionPackPlayerEffect, Q2_MISSION_PACK_DAMAGE};

/// Sphere kind (`Q2SphereKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2SphereKind {
    /// Defender.
    Defender,
    /// Hunter.
    Hunter,
    /// Vengeance.
    Vengeance,
}

/// Sphere hooks (`Q2SphereHooks`).
#[derive(Debug, Clone, Copy)]
pub struct Q2SphereHooks {
    /// Whether the hunter camera is enabled.
    pub hunter_camera: bool,
    /// Whether the match is in intermission.
    pub intermission: fn(&Q2GameServices) -> bool,
    /// Emit a player effect.
    pub player_effect: fn(&Q2GameServices, Q2MissionPackPlayerEffect),
}

/// Doppleganger sphere flag (`doppleFlag`).
fn dopple_flag(game: &Q2GameServices) -> i32 {
    if game.options.edition == Q2Edition::Rerelease {
        0x10000
    } else {
        0x100
    }
}

/// Sphere callbacks (`Q2MissionPackSpheres::callbacks`).
pub fn sphere_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("sphere_think_explode", sphere_expire as Q2Think);
    callbacks.think.insert("defender_think", defender_think as Q2Think);
    callbacks.think.insert("hunter_think", hunter_think as Q2Think);
    callbacks.think.insert("vengeance_think", vengeance_think as Q2Think);
    callbacks.pain.insert("defender_pain", defender_pain as Q2Pain);
    callbacks.pain.insert("hunter_pain", hunter_pain as Q2Pain);
    callbacks.pain.insert("vengeance_pain", vengeance_pain as Q2Pain);
    callbacks.die.insert("sphere_explode", sphere_die as Q2Die);
    callbacks.die.insert("sphere_if_idle_die", sphere_idle_die as Q2Die);
    callbacks.touch.insert("hunter_touch", hunter_touch as Q2Touch);
    callbacks.touch.insert("vengeance_touch", vengeance_touch as Q2Touch);
    callbacks
}

/// Disconnected intermission hook (never in intermission).
fn disconnected_intermission(_game: &Q2GameServices) -> bool {
    false
}

/// Disconnected player-effect hook (drops the effect).
fn disconnected_player_effect(_game: &Q2GameServices, _effect: Q2MissionPackPlayerEffect) {}

/// Sphere hooks used before the session installs its own.
pub fn disconnected_sphere_hooks() -> Q2SphereHooks {
    Q2SphereHooks {
        hunter_camera: false,
        intermission: disconnected_intermission,
        player_effect: disconnected_player_effect,
    }
}

/// Mission-pack spheres (`Q2MissionPackSpheres`).
#[derive(Debug, Clone, Copy)]
pub struct Q2MissionPackSpheres {
    /// Projectile driver.
    pub projectiles: Q2MissionPackProjectiles,
    /// Sphere hooks.
    pub hooks: Q2SphereHooks,
}

/// Spheres driver bound to the session hooks.
pub fn mission_spheres(game: &Q2GameServices) -> Q2MissionPackSpheres {
    Q2MissionPackSpheres {
        projectiles: mission_projectiles(game),
        hooks: game.mission_packs.sphere_hooks,
    }
}

impl Q2MissionPackSpheres {
    /// Find an actor's sphere (`ownedSphere`).
    pub fn owned_sphere(&self, actor: &ActorId, game: &Q2GameServices) -> Option<ActorId> {
        let flag = dopple_flag(game);
        game.entities.values().find_map(|entity| {
            let id = entity.actor.id().clone();
            let record = game.require_entity(&id);
            if record.classname == "sphere" && record.owner.as_ref() == Some(actor) && record.spawnflags & flag == 0 {
                Some(id)
            } else {
                None
            }
        })
    }

    /// Retaliate for owner damage (`ownerDamaged`).
    pub fn owner_damaged(&self, actor: &ActorId, attack: &AttackProvenance, game: &mut Q2GameServices) {
        let Some(sphere) = self.owned_sphere(actor, game) else {
            return;
        };
        let Some(pain) = game.require_entity(&sphere).pain else {
            return;
        };
        let owned = game.owned_of(sphere.clone());
        pain(
            sphere,
            game,
            PainReaction {
                attack: Some(attack.clone()),
                this: owned,
                attacker: attack.attacker.clone(),
                kick: 0.0,
                damage: 0.0,
            },
        );
    }

    /// Retaliate for owner death (`ownerDied`).
    pub fn owner_died(&self, actor: &ActorId, game: &mut Q2GameServices, attack: Option<&AttackProvenance>) {
        let Some(sphere) = self.owned_sphere(actor, game) else {
            return;
        };
        let Some(die) = game.require_entity(&sphere).die else {
            return;
        };
        let owned = game.owned_of(sphere.clone());
        die(
            sphere,
            game,
            DeathReaction {
                pain: PainReaction {
                    attack: attack.cloned(),
                    this: owned,
                    attacker: Some(actor.clone()),
                    kick: 0.0,
                    damage: 0.0,
                },
                inflictor: Some(actor.clone()),
                point: Vec3::default(),
            },
        );
    }

    /// Remove an actor's sphere on disconnect (`disconnect`).
    pub fn disconnect(&self, actor: &ActorId, game: &mut Q2GameServices) {
        if let Some(sphere) = self.owned_sphere(actor, game) {
            game.remove_actor(sphere);
        }
    }

    /// Launch a sphere (`launch`).
    pub fn launch(&self, owner: &ActorId, game: &mut Q2GameServices, kind: Q2SphereKind, dopple: bool) -> ActorId {
        game.source_callbacks.register(&sphere_callbacks());
        if !dopple {
            if let Some(old) = self.owned_sphere(owner, game) {
                game.remove_actor(old);
            }
        }
        let sphere = game.create("sphere", std::collections::BTreeMap::new());
        let body = game.body_of(owner.clone());
        let team_master = game.require_entity(owner).team_master.clone();
        let flag = dopple_flag(game);
        let wait = game.host.now() + 30.0;
        let mask = projectile_mask(game);
        {
            let record = game.require_entity_mut(&sphere);
            record.owner = if dopple { None } else { Some(owner.clone()) };
            record.team_master = if dopple { team_master } else { None };
            record.spawnflags = (match kind {
                Q2SphereKind::Defender => 1,
                Q2SphereKind::Hunter => 2,
                Q2SphereKind::Vengeance => 4,
            } | if dopple { flag } else { 0 });
            record.clip_mask = mask;
            record.render_flags = 8 | 0x8000;
            record.motion = Q2MotionKind::FlyMissile;
            record.wait = wait;
            record.model = format!(
                "models/items/{}/tris.md2",
                match kind {
                    Q2SphereKind::Defender => "defender",
                    Q2SphereKind::Hunter => "hunter",
                    Q2SphereKind::Vengeance => "vengnce",
                }
            );
            match kind {
                Q2SphereKind::Defender => {
                    record.model2 = "models/items/shell/tris.md2".to_string();
                    record.pain = Some(defender_pain as Q2Pain);
                    record.die = Some(sphere_die as Q2Die);
                }
                Q2SphereKind::Hunter => {
                    record.pain = Some(hunter_pain as Q2Pain);
                    record.die = Some(sphere_idle_die as Q2Die);
                }
                Q2SphereKind::Vengeance => {
                    record.pain = Some(vengeance_pain as Q2Pain);
                    record.die = Some(sphere_idle_die as Q2Die);
                    record.angular_velocity = vec3(30.0, 30.0, 0.0);
                }
            }
        }
        let mut moved = game.body_of(sphere.clone());
        moved.origin = vec3(body.origin.x, body.origin.y, body.origin.z + body.bounds.max.z);
        moved.angles = vec3(0.0, body.angles.y, 0.0);
        moved.bounds.min = Vec3::default();
        moved.bounds.max = Vec3::default();
        game.write_body(sphere.clone(), &moved, false);
        game.schedule(
            sphere.clone(),
            0.1,
            match kind {
                Q2SphereKind::Defender => defender_think as Q2Think,
                Q2SphereKind::Hunter => hunter_think as Q2Think,
                Q2SphereKind::Vengeance => vengeance_think as Q2Think,
            },
        );
        publish_projectile(sphere.clone(), game, "", None);
        let idle = match kind {
            Q2SphereKind::Defender => "spheres/d_idle.wav",
            Q2SphereKind::Hunter => "spheres/h_idle.wav",
            Q2SphereKind::Vengeance => "spheres/v_idle.wav",
        };
        sphere_loop(&sphere, game, idle);
        sphere
    }
}

/// Expire a sphere (`expire`).
fn sphere_expire(entity: ActorId, game: &mut Q2GameServices) {
    let owner = game.require_entity(&entity).owner.clone();
    if let Some(owner) = owner
        .as_ref()
        .and_then(|owner| game.entity(owner).map(|entity| entity.actor.id().clone()))
    {
        if game.require_entity(&owner).flags & 0x4000 != 0 {
            let mut moved = game.body_of(owner.clone());
            moved.velocity = Vec3::default();
            game.write_body(owner.clone(), &moved, true);
            game.set_motion_kind(owner.clone(), Q2MotionKind::Stationary);
            let body = game.body_of(owner.clone());
            (mission_spheres(game).hooks.player_effect)(
                game,
                Q2MissionPackPlayerEffect::SphereCamera {
                    actor: owner,
                    sphere: None,
                    origin: body.origin,
                    angles: body.angles,
                },
            );
        }
    }
    explode(&entity, game, "explosion1");
}

/// Sphere die (`die`).
fn sphere_die(entity: ActorId, game: &mut Q2GameServices, _reaction: DeathReaction) {
    sphere_expire(entity, game);
}

/// Idle sphere die (`idleDie`).
fn sphere_idle_die(entity: ActorId, game: &mut Q2GameServices, _reaction: DeathReaction) {
    if game.require_entity(&entity).enemy.is_none() {
        sphere_expire(entity, game);
    }
}

/// Switch a sphere loop sound (`loop`).
fn sphere_loop(entity: &ActorId, game: &mut Q2GameServices, sound: &str) {
    if game.require_entity(entity).sound == sound {
        return;
    }
    if !game.require_entity(entity).sound.is_empty() {
        let path = game.require_entity(entity).sound.clone();
        let origin = game.body_of(entity.clone()).origin;
        game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(entity.clone()),
            origin,
            path,
            channel: 0,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Stop,
            loop_owner: None,
        }));
    }
    game.require_entity_mut(entity).sound = sound.to_string();
    let origin = game.body_of(entity.clone()).origin;
    game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(entity.clone()),
        origin,
        path: sound.to_string(),
        channel: 0,
        volume: 1.0,
        attenuation: 1.0,
        reliable: false,
        loop_: Q2SoundLoop::Start,
        loop_owner: None,
    }));
}

/// Fly a sphere toward its owner (`fly`).
fn sphere_fly(entity: ActorId, game: &mut Q2GameServices) {
    if game.host.now() >= game.require_entity(&entity).wait {
        sphere_expire(entity, game);
        return;
    }
    let owner = game.require_entity(&entity).owner.clone();
    let owner_body = owner.as_ref().and_then(|owner| game.host.bodies().read(owner));
    let Some(owner_body) = owner_body else {
        game.remove_actor(entity);
        return;
    };
    let destination = vec3(
        owner_body.origin.x,
        owner_body.origin.y,
        owner_body.origin.z + owner_body.bounds.max.z + 4.0,
    );
    if game.host.now() == game.host.now().trunc()
        && owner.is_some()
        && !sight(game, &entity, owner.as_ref().expect("sphere owner is missing"))
    {
        let mut moved = game.body_of(entity.clone());
        moved.origin = destination;
        game.write_body(entity, &moved, true);
        return;
    }
    let mut moved = game.body_of(entity.clone());
    moved.velocity = scale3(sub3(destination, moved.origin), 5.0);
    game.write_body(entity.clone(), &moved, true);
    let motion = game.require_entity(&entity).motion;
    game.set_motion_kind(entity, motion);
}

/// Chase a sphere enemy (`chase`).
fn sphere_chase(entity: ActorId, game: &mut Q2GameServices, direct: bool) {
    let enemy = game.require_entity(&entity).enemy.clone();
    let body = enemy.as_ref().and_then(|enemy| game.host.bodies().read(enemy));
    if game.host.now() >= game.require_entity(&entity).wait
        || enemy.is_none()
        || body.is_none()
        || enemy.as_ref().is_some_and(|enemy| {
            game.host
                .combat()
                .read(enemy)
                .map(|combat| combat.health)
                .unwrap_or(0.0)
                < 1.0
        })
    {
        sphere_expire(entity, game);
        return;
    }
    let enemy = enemy.expect("sphere enemy is missing");
    let body = body.expect("sphere enemy body is missing");
    let destination = if game.host.is_player(&enemy) {
        add3(
            body.origin,
            vec3(
                0.0,
                0.0,
                game.entity(&enemy)
                    .map(|entity| entity.view_height as f32)
                    .unwrap_or(22.0),
            ),
        )
    } else {
        body.origin
    };
    let origin = game.body_of(entity.clone()).origin;
    let (direction, speed) = if direct || sight(game, &entity, &enemy) {
        if !direct {
            sphere_loop(&entity, game, "spheres/h_active.wav");
        }
        game.require_entity_mut(&entity).pos1 = destination;
        (normalize3(sub3(destination, origin)), 500.0)
    } else if f64::from(length3(game.require_entity(&entity).pos1)) == 0.0 {
        sphere_loop(&entity, game, "spheres/h_lurk.wav");
        (normalize3(sub3(body.origin, origin)), 0.0)
    } else {
        let direction = sub3(game.require_entity(&entity).pos1, origin);
        let distance = f64::from(length3(direction));
        let direction = normalize3(direction);
        if distance > 1.0 {
            let speed = if distance > 500.0 {
                500.0
            } else if distance < 20.0 {
                distance / game.host.frame_seconds()
            } else {
                distance
            };
            if !direct {
                sphere_loop(&entity, game, "spheres/h_active.wav");
            }
            (direction, speed)
        } else {
            if !direct {
                sphere_loop(&entity, game, "spheres/h_lurk.wav");
            }
            (normalize3(sub3(body.origin, origin)), 0.0)
        }
    };
    let mut moved = game.body_of(entity.clone());
    moved.angles = vector_angles(direction);
    moved.velocity = scale3(direction, speed as f32);
    game.write_body(entity.clone(), &moved, true);
    let motion = game.require_entity(&entity).motion;
    game.set_motion_kind(entity, motion);
}

/// Sphere touch (`touch`).
fn sphere_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact, means: i32) {
    let flag = dopple_flag(game);
    if game.require_entity(&entity).spawnflags & flag != 0 {
        if game.require_entity(&entity).team_master.as_ref() == Some(&contact.other) {
            return;
        }
        let master = game.require_entity(&entity).team_master.clone();
        game.require_entity_mut(&entity).owner = master;
        game.require_entity_mut(&entity).team_master = None;
    } else if game.require_entity(&entity).owner.as_ref() == Some(&contact.other)
        || game
            .entity(&contact.other)
            .map(|entity| entity.classname.clone())
            .as_deref()
            == Some("bodyque")
    {
        return;
    }
    if contact
        .surface
        .as_ref()
        .map(|surface| surface.native_flags)
        .unwrap_or(0)
        & 4
        != 0
    {
        game.remove_actor(entity);
        return;
    }
    let body = game.body_of(entity.clone());
    if game
        .host
        .combat()
        .read(&contact.other)
        .is_some_and(|combat| combat.can_take_damage)
    {
        let normal = contact.plane.map(|plane| plane.normal).unwrap_or_default();
        game.damage(
            contact.other.clone(),
            entity.clone(),
            game.require_entity(&entity).owner.clone(),
            10000.0,
            1.0,
            body.velocity,
            body.origin,
            normal,
            means,
            64,
            None,
        );
    } else {
        let owner = game.require_entity(&entity).owner.clone();
        game.radius_damage(entity.clone(), owner.clone(), 512.0, owner, 256.0, means, 0, None);
    }
    sphere_expire(entity, game);
}

/// Vengeance touch (`vengeanceTouch`).
fn vengeance_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let flag = dopple_flag(game);
    let means = if game.require_entity(&entity).spawnflags & flag != 0 {
        Q2_MISSION_PACK_DAMAGE.dopple_vengeance
    } else {
        Q2_MISSION_PACK_DAMAGE.vengeance_sphere
    };
    sphere_touch(entity, game, contact, means);
}

/// Hunter touch (`hunterTouch`).
fn hunter_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if contact.other == game.host.world_actor() {
        return;
    }
    let owner = game
        .require_entity(&entity)
        .owner
        .clone()
        .and_then(|owner| game.entity(&owner).map(|entity| entity.actor.id().clone()));
    if let Some(owner) = owner {
        if game.require_entity(&owner).flags & 0x4000 != 0 {
            let mut moved = game.body_of(owner.clone());
            moved.velocity = Vec3::default();
            game.write_body(owner.clone(), &moved, true);
            game.set_motion_kind(owner, Q2MotionKind::Stationary);
        }
    }
    let flag = dopple_flag(game);
    let means = if game.require_entity(&entity).spawnflags & flag != 0 {
        Q2_MISSION_PACK_DAMAGE.dopple_hunter
    } else {
        Q2_MISSION_PACK_DAMAGE.hunter_sphere
    };
    sphere_touch(entity, game, contact, means);
}

/// Defender pain (`defenderPain`).
fn defender_pain(entity: ActorId, game: &mut Q2GameServices, reaction: PainReaction) {
    if reaction.attacker != game.require_entity(&entity).owner {
        game.require_entity_mut(&entity).enemy = reaction.attacker;
    }
}

/// Vengeance pain (`vengeancePain`).
fn vengeance_pain(entity: ActorId, game: &mut Q2GameServices, reaction: PainReaction) {
    if game.require_entity(&entity).enemy.is_some() || reaction.attacker.is_none() {
        return;
    }
    let flag = dopple_flag(game);
    let owner_healthy = game.require_entity(&entity).owner.clone().is_some_and(|owner| {
        game.host
            .combat()
            .read(&owner)
            .map(|combat| combat.health)
            .unwrap_or(0.0)
            >= 25.0
    });
    if game.require_entity(&entity).spawnflags & flag == 0
        && (owner_healthy || reaction.attacker == game.require_entity(&entity).owner)
    {
        return;
    }
    let wait = game.require_entity(&entity).wait.max(game.host.now() + 15.0);
    game.require_entity_mut(&entity).wait = wait;
    game.require_entity_mut(&entity).effects |= 16;
    game.require_entity_mut(&entity).touch = Some(vengeance_touch as Q2Touch);
    game.require_entity_mut(&entity).enemy = reaction.attacker;
    game.show(entity);
}

/// Hunter pain (`hunterPain`).
fn hunter_pain(entity: ActorId, game: &mut Q2GameServices, reaction: PainReaction) {
    if game.require_entity(&entity).enemy.is_some() || reaction.attacker.is_none() {
        return;
    }
    let owner = game
        .require_entity(&entity)
        .owner
        .clone()
        .and_then(|owner| game.entity(&owner).map(|entity| entity.actor.id().clone()));
    let flag = dopple_flag(game);
    let dopple = game.require_entity(&entity).spawnflags & flag != 0;
    let owner_healthy = owner.as_ref().is_some_and(|owner| {
        game.host
            .combat()
            .read(owner)
            .map(|combat| combat.health)
            .unwrap_or(0.0)
            > 0.0
    });
    if !dopple && (owner_healthy || reaction.attacker == game.require_entity(&entity).owner) {
        return;
    }
    let wait = game.require_entity(&entity).wait.max(game.host.now() + 15.0);
    game.require_entity_mut(&entity).wait = wait;
    game.require_entity_mut(&entity).effects |= 8 | 0x4000000;
    game.require_entity_mut(&entity).touch = Some(hunter_touch as Q2Touch);
    game.require_entity_mut(&entity).enemy = reaction.attacker.clone();
    game.show(entity.clone());
    if dopple
        || owner.is_none()
        || owner.as_ref().is_some_and(|owner| !game.host.is_player(owner))
        || !mission_spheres(game).hooks.hunter_camera
        || game.options.deathmatch_flags & 1024 != 0
    {
        return;
    }
    let owner = owner.expect("sphere owner is missing");
    let attacker = reaction.attacker.expect("sphere attacker is missing");
    let target = game.host.bodies().read(&attacker);
    if target.is_none()
        || f64::from(length3(sub3(
            target.expect("sphere target is missing").origin,
            game.body_of(entity.clone()).origin,
        ))) < 192.0
    {
        return;
    }
    game.sound(&owner, "misc/udeath.wav", 4, 1.0, 1.0);
    for _ in 0..4 {
        throw_gib(
            owner.clone(),
            game,
            "models/objects/gibs/sm_meat/tris.md2",
            50.0,
            Q2GibOptions {
                head: false,
                metallic: false,
                skinned: false,
                upright: false,
            },
        );
    }
    throw_gib(
        owner.clone(),
        game,
        "models/objects/gibs/skull/tris.md2",
        50.0,
        Q2GibOptions {
            head: false,
            metallic: false,
            skinned: false,
            upright: false,
        },
    );
    let origin = add3(
        game.body_of(owner.clone()).origin,
        vec3(0.0, 0.0, game.require_entity(&owner).view_height as f32),
    );
    let mut moved = game.body_of(entity.clone());
    moved.origin = origin;
    game.write_body(entity.clone(), &moved, true);
    {
        let record = game.require_entity_mut(&owner);
        record.model = String::new();
        record.model2 = String::new();
        record.view_height = 8;
        record.flags |= 0x4000;
    }
    let angles = game.body_of(entity.clone()).angles;
    let mut moved = game.body_of(owner.clone());
    moved.origin = origin;
    moved.angles = angles;
    moved.bounds.min = vec3(-5.0, -5.0, -5.0);
    moved.bounds.max = vec3(5.0, 5.0, 5.0);
    game.write_body(owner.clone(), &moved, true);
    game.set_solid(owner.clone(), Q2Solid::None);
    game.set_motion_kind(owner.clone(), Q2MotionKind::FlyMissile);
    game.show(owner.clone());
    game.set_solid(entity.clone(), Q2Solid::Box);
    (mission_spheres(game).hooks.player_effect)(
        game,
        Q2MissionPackPlayerEffect::SphereCamera {
            actor: owner,
            sphere: Some(entity),
            origin,
            angles,
        },
    );
}

/// Defender think (`defenderThink`).
fn defender_think(entity: ActorId, game: &mut Q2GameServices) {
    let owner = game
        .require_entity(&entity)
        .owner
        .clone()
        .and_then(|owner| game.entity(&owner).map(|entity| entity.actor.id().clone()));
    let Some(owner) = owner else {
        game.remove_actor(entity);
        return;
    };
    if (mission_spheres(game).hooks.intermission)(game)
        || game
            .host
            .combat()
            .read(&owner)
            .map(|combat| combat.health)
            .unwrap_or(0.0)
            <= 0.0
    {
        sphere_expire(entity, game);
        return;
    }
    {
        let record = game.require_entity_mut(&entity);
        record.frame += 1;
        if record.frame > 19 {
            record.frame = 0;
        }
    }
    if let Some(enemy) = game.require_entity(&entity).enemy.clone() {
        let target = game.host.bodies().read(&enemy);
        if game
            .host
            .combat()
            .read(&enemy)
            .map(|combat| combat.health)
            .unwrap_or(0.0)
            <= 0.0
            || target.is_none()
        {
            game.require_entity_mut(&entity).enemy = None;
        } else if enemy != owner
            && game.require_entity(&entity).delay <= game.host.now()
            && sight(game, &entity, &enemy)
        {
            let origin = game.body_of(entity.clone()).origin;
            if let Some(target) = target {
                mission_projectiles(game).fire_blaster2(
                    owner,
                    game,
                    add3(origin, vec3(0.0, 0.0, 2.0)),
                    normalize3(sub3(target.origin, origin)),
                    10.0,
                    1000.0,
                    8,
                );
                game.require_entity_mut(&entity).delay = game.host.now() + 0.4;
            }
        }
    }
    sphere_fly(entity.clone(), game);
    if game.host.actors().is_live(&entity) {
        game.show(entity.clone());
        game.schedule(entity, 0.1, defender_think as Q2Think);
    }
}

/// Hunter think (`hunterThink`).
fn hunter_think(entity: ActorId, game: &mut Q2GameServices) {
    if (mission_spheres(game).hooks.intermission)(game) {
        sphere_expire(entity, game);
        return;
    }
    let owner = game
        .require_entity(&entity)
        .owner
        .clone()
        .and_then(|owner| game.entity(&owner).map(|entity| entity.actor.id().clone()));
    let enemy = game
        .require_entity(&entity)
        .enemy
        .clone()
        .and_then(|enemy| game.host.bodies().read(&enemy));
    let flag = dopple_flag(game);
    if owner.is_none() && game.require_entity(&entity).spawnflags & flag == 0 {
        game.remove_actor(entity);
        return;
    }
    let body = game.body_of(entity.clone());
    let ideal = if let Some(owner) = owner.clone() {
        game.body_of(owner).angles.y
    } else if let Some(enemy) = enemy.clone() {
        vector_angles(sub3(enemy.origin, body.origin)).y
    } else {
        body.angles.y
    };
    let mut delta = f64::from(ideal) - f64::from((body.angles.y % 360.0 + 360.0) % 360.0);
    if delta > 180.0 {
        delta -= 360.0;
    }
    if delta < -180.0 {
        delta += 360.0;
    }
    let mut moved = game.body_of(entity.clone());
    moved.angles = vec3(
        body.angles.x,
        ((f64::from(body.angles.y) + delta.clamp(-40.0, 40.0) + 360.0) % 360.0) as f32,
        body.angles.z,
    );
    game.write_body(entity.clone(), &moved, true);
    if game.require_entity(&entity).enemy.is_none() {
        sphere_fly(entity.clone(), game);
    } else {
        sphere_chase(entity.clone(), game, false);
    }
    if owner
        .as_ref()
        .is_some_and(|owner| game.require_entity(owner).flags & 0x4000 != 0)
        && game.host.actors().is_live(&entity)
    {
        let owner = owner.expect("sphere owner is missing");
        let current = game.body_of(entity.clone());
        let current_owner = game.body_of(owner.clone());
        game.require_entity_mut(&owner).view_height = (current.origin.z - current_owner.origin.z) as i32;
        let angles = match enemy {
            None => current.angles,
            Some(enemy) => vec3(0.0, vector_angles(sub3(enemy.origin, current.origin)).y, 0.0),
        };
        let mut moved = game.body_of(owner.clone());
        moved.origin = current.origin;
        moved.velocity = current.velocity;
        moved.bounds.min = Vec3::default();
        moved.bounds.max = Vec3::default();
        game.write_body(owner.clone(), &moved, true);
        game.set_motion_kind(owner.clone(), Q2MotionKind::FlyMissile);
        (mission_spheres(game).hooks.player_effect)(
            game,
            Q2MissionPackPlayerEffect::SphereCamera {
                actor: owner,
                sphere: Some(entity.clone()),
                origin: current.origin,
                angles,
            },
        );
    }
    if game.host.actors().is_live(&entity) {
        game.schedule(entity, 0.1, hunter_think as Q2Think);
    }
}

/// Vengeance think (`vengeanceThink`).
fn vengeance_think(entity: ActorId, game: &mut Q2GameServices) {
    if (mission_spheres(game).hooks.intermission)(game) {
        sphere_expire(entity, game);
        return;
    }
    let owner = game
        .require_entity(&entity)
        .owner
        .clone()
        .and_then(|owner| game.entity(&owner).map(|entity| entity.actor.id().clone()));
    let flag = dopple_flag(game);
    if owner.is_none() && game.require_entity(&entity).spawnflags & flag == 0 {
        game.remove_actor(entity);
        return;
    }
    if game.require_entity(&entity).enemy.is_none() {
        sphere_fly(entity.clone(), game);
    } else {
        sphere_chase(entity.clone(), game, true);
    }
    if game.host.actors().is_live(&entity) {
        game.schedule(entity, 0.1, vengeance_think as Q2Think);
    }
}
