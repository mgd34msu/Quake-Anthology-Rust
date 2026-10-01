//! Mission-pack trap (`src/content/q2/missionpacks/projectiles/trap.ts`).
//!
//! Xatrix g_weapon.c Trap_Think (GPL-2.0-or-later).

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, vec3, Vec3};

use crate::contract::{ArmorState, PoweredProtectionState, ProjectileRole, RegularArmorState};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2Die, Q2Edition, Q2GameServices, Q2Mode, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop,
    Q2SpawnFields, Q2Think, Q2TraceRequest,
};
use crate::q2::foundation::monsters::ai::change_yaw;
use crate::q2::foundation::weapons::ballistics::weapon_player_noise;
use crate::q2::foundation::weapons::player::weapon_can_target;
use crate::q2::foundation::weapons::vectors::{angle_vectors, vector_angles};
use crate::q2::support::contracts::CombatState;

use super::super::types::Q2_MISSION_PACK_DAMAGE;
use super::common::{explode, free_projectile, publish_projectile, sight, velocity};
use super::Q2MissionPackProjectiles;

/// Cross product.
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    vec3(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

/// Trap callbacks (merged over the nuke callbacks).
pub fn trap_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = super::nuke::nuke_callbacks();
    callbacks.think.insert("Trap_Think", trap_think as Q2Think);
    callbacks
        .think
        .insert("rerelease/Trap_Think", trap_rerelease as Q2Think);
    callbacks.think.insert("Trap_Gib_Think", trap_gib as Q2Think);
    callbacks.die.insert("trap_die", trap_die as Q2Die);
    callbacks
}

impl Q2MissionPackProjectiles {
    /// Fire a trap (`fireTrap`).
    #[allow(clippy::too_many_arguments)]
    pub fn fire_trap(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        timer: f64,
        radius: f64,
        held: bool,
    ) -> ActorId {
        let trap = self.throw_mine(&owner, game, "trap", start, direction, speed);
        if game.options.edition == Q2Edition::Rerelease {
            let axes = angle_vectors(vector_angles(direction));
            let body = game.body_of(trap.clone());
            let base_up = dot3(sub3(body.velocity, scale3(direction, speed as f32)), axes.up);
            let gravity = self
                .hooks
                .gravity
                .map(|gravity| gravity())
                .or_else(|| game.weapons.inputs.get(&owner).map(|input| input.gravity))
                .unwrap_or(800.0);
            let trap_player = game.host.is_player(&owner);
            let trap_collide = game
                .weapons
                .inputs
                .get(&owner)
                .is_some_and(|input| !input.players_collide);
            let trap_until = game.host.now() + 30.0;
            {
                let entity = game.require_entity_mut(&trap);
                entity.classname = "food_cube_trap".to_string();
                entity.model = "models/weapons/z_trap/tris.md2".to_string();
                entity.effects = 0;
                entity.render_flags = 0;
                entity.flags = 0x20000 | 0x2000;
                entity.flags += 2i64.pow(32);
                entity.die = Some(trap_die as Q2Die);
                entity.angular_velocity = vec3(0.0, 300.0, 0.0);
                entity.clip_mask = 0x42004003;
                if trap_player && trap_collide {
                    entity.clip_mask &= !0x40000000;
                }
                entity.timestamp = trap_until;
                entity.sound = "weapons/traploop.wav".to_string();
            }
            let owned = game.owned_of(trap.clone());
            game.host.combat().create(
                &owned,
                &CombatState {
                    health: 20.0,
                    armor: ArmorState {
                        regular: RegularArmorState::None,
                        powered: PoweredProtectionState::None,
                    },
                    mass: 0.0,
                    can_take_damage: true,
                    invulnerable: false,
                    no_knockback: false,
                    team: None,
                },
            );
            let mut moved = game.body_of(trap.clone());
            moved.angles = Vec3::default();
            moved.velocity = add3(
                body.velocity,
                scale3(axes.up, (f64::from(base_up) * (gravity / 800.0 - 1.0)) as f32),
            );
            moved.bounds.min = vec3(-4.0, -4.0, 0.0);
            moved.bounds.max = vec3(4.0, 4.0, 8.0);
            game.write_body(trap.clone(), &moved, false);
            game.schedule(trap.clone(), 1.0, trap_rerelease as Q2Think);
            let sound = game.require_entity(&trap).sound.clone();
            publish_projectile(
                trap.clone(),
                game,
                &sound,
                Some(("q2:ammo_trap", ProjectileRole::Grenade)),
            );
            return trap;
        }
        let classic_until = game.host.now() + 30.0;
        {
            let entity = game.require_entity_mut(&trap);
            entity.classname = "htrap".to_string();
            entity.model = "models/weapons/z_trap/tris.md2".to_string();
            entity.effects = 0;
            entity.render_flags = 0;
            entity.damageable_target = false;
            entity.team_master = None;
            entity.damage = damage;
            entity.damage_radius = radius;
            entity.spawnflags = if held { 3 } else { 1 };
            entity.angular_velocity = vec3(0.0, 300.0, 0.0);
            entity.timestamp = classic_until;
        }
        let mut moved = game.body_of(trap.clone());
        moved.angles = Vec3::default();
        moved.bounds.min = vec3(-4.0, -4.0, 0.0);
        moved.bounds.max = vec3(4.0, 4.0, 8.0);
        game.write_body(trap.clone(), &moved, false);
        let owned = game.owned_of(trap.clone());
        game.host.combat().create(
            &owned,
            &CombatState {
                health: 0.0,
                armor: ArmorState {
                    regular: RegularArmorState::None,
                    powered: PoweredProtectionState::None,
                },
                mass: 0.0,
                can_take_damage: false,
                invulnerable: false,
                no_knockback: false,
                team: None,
            },
        );
        if timer <= 0.0 {
            weapon_player_noise(
                game,
                &owner,
                start,
                crate::q2::foundation::weapons::ballistics::NoiseKind::Impact,
            );
            let (owner_id, damage, radius) = {
                let entity = game.require_entity(&trap);
                (entity.owner.clone(), entity.damage, entity.damage_radius)
            };
            game.radius_damage(
                trap.clone(),
                owner_id,
                damage,
                None,
                radius,
                if held { 24 } else { 16 },
                0,
                Some("q2:ammo_trap".to_string()),
            );
            self.grenade_effect(&trap, game);
            game.remove_actor(trap.clone());
        } else {
            game.schedule(trap.clone(), 1.0, trap_think as Q2Think);
            publish_projectile(
                trap.clone(),
                game,
                "weapons/traploop.wav",
                Some(("q2:ammo_trap", ProjectileRole::Grenade)),
            );
        }
        trap
    }
}

/// Trap die (`trapDie`).
fn trap_die(entity: ActorId, game: &mut Q2GameServices, _reaction: crate::q2::support::contracts::DeathReaction) {
    explode(&entity, game, "explosion1");
}

/// Trap gib think (`trapGib`).
fn trap_gib(entity: ActorId, game: &mut Q2GameServices) {
    let trap = game
        .require_entity(&entity)
        .owner
        .clone()
        .and_then(|owner| game.entity(&owner).map(|entity| entity.actor.id().clone()));
    let Some(trap) = trap else {
        game.remove_actor(entity);
        return;
    };
    if game.require_entity(&trap).frame != 5 {
        game.remove_actor(entity);
        return;
    }
    let body = game.body_of(entity.clone());
    let origin = game.body_of(trap.clone()).origin;
    let axes = angle_vectors(game.body_of(trap.clone()).angles);
    let frame_seconds = game.host.frame_seconds();
    let degrees = 150.0 * frame_seconds + game.require_entity(&trap).delay;
    let radians = degrees * std::f64::consts::PI / 180.0;
    let delta = sub3(origin, body.origin);
    let rotated = add3(
        add3(
            scale3(delta, radians.cos() as f32),
            scale3(cross(axes.up, delta), radians.sin() as f32),
        ),
        scale3(axes.up, dot3(axes.up, delta) * (1.0 - radians.cos() as f32)),
    );
    let trace = game.host.trace(&Q2TraceRequest {
        start: body.origin,
        end: sub3(origin, rotated),
        bounds: None,
        ignore: Some(entity.clone()),
        mask: 3,
        exclude: Vec::new(),
    });
    let mut moved = body.clone();
    moved.origin = add3(trace.end, scale3(normalize3(delta), (15.0 * frame_seconds) as f32));
    moved.angles = vec3(body.angles.x, body.angles.y + degrees as f32, body.angles.z);
    game.write_body(entity.clone(), &moved, true);
    game.schedule(entity, frame_seconds, trap_gib as Q2Think);
}

/// Trap rerelease think (`trapRerelease`).
fn trap_rerelease(entity: ActorId, game: &mut Q2GameServices) {
    let now = game.host.now();
    let body = game.body_of(entity.clone());
    if game.require_entity(&entity).timestamp < now {
        explode(&entity, game, "explosion1");
        return;
    }
    game.schedule(entity.clone(), 0.1, trap_rerelease as Q2Think);
    if body.ground.is_none() {
        return;
    }
    if game.require_entity(&entity).frame > 4 {
        if game.require_entity(&entity).frame == 5 {
            if game.require_entity(&entity).wait == 64.0 {
                game.sound(&entity, "weapons/trapdown.wav", 2, 1.0, 2.0);
            }
            game.require_entity_mut(&entity).wait -= 2.0;
            game.require_entity_mut(&entity).delay += 2.0;
            if game.require_entity(&entity).wait < 19.0 {
                game.require_entity_mut(&entity).frame += 1;
            }
        } else if {
            game.require_entity_mut(&entity).frame += 1;
            game.require_entity(&entity).frame == 8
        } {
            game.schedule(entity.clone(), 1.0, free_projectile);
            game.require_entity_mut(&entity).effects &= !0x2000000;
            let size = 1.0 + (game.require_entity(&entity).accel - 100.0) / 300.0;
            let mut values = BTreeMap::new();
            values.insert(
                "count".to_string(),
                format!(
                    "{}",
                    game.host
                        .combat()
                        .read(&entity)
                        .map(|combat| combat.mass)
                        .unwrap_or(0.0)
                ),
            );
            values.insert("spawnflags".to_string(), "65536".to_string());
            values.insert(
                "origin".to_string(),
                format!(
                    "{} {} {}",
                    body.origin.x,
                    body.origin.y,
                    body.origin.z + 24.0 * size as f32
                ),
            );
            let food = game.spawn(Q2SpawnFields {
                ordinal: -1,
                classname: "item_foodcube".to_string(),
                values,
            });
            game.require_entity_mut(&food).scale = size;
            let mut moved = game.body_of(food.clone());
            moved.angles = vec3(0.0, (game.host.random() * 360.0) as f32, 0.0);
            moved.velocity = vec3(0.0, 0.0, 400.0);
            game.write_body(food.clone(), &moved, true);
            if let Some(think) = game.require_entity(&food).think {
                think(food.clone(), game);
            }
            game.cancel_actor(food.clone());
            game.show(food.clone());
            game.sound(&food, "misc/fhit3.wav", 2, 1.0, 1.0);
        }
        game.show(entity);
        return;
    }
    game.require_entity_mut(&entity).effects &= !0x2000000;
    if game.require_entity(&entity).frame >= 4 {
        game.require_entity_mut(&entity).effects |= 0x2000000;
        if game.options.mode == Q2Mode::Deathmatch {
            game.require_entity_mut(&entity).owner = None;
            let motion = game.require_entity(&entity).motion;
            game.set_motion_kind(entity.clone(), motion);
        }
    } else {
        game.require_entity_mut(&entity).frame += 1;
        game.show(entity);
        return;
    }
    let mut best: Option<ActorId> = None;
    let mut nearest = 8000.0;
    for actor in game.host.nearby(body.origin, 256.0) {
        if actor == entity {
            continue;
        }
        let target = game.entity(&actor).map(|entity| entity.actor.id().clone());
        let target_body = game.host.bodies().read(&actor);
        if game.options.mode == Q2Mode::Deathmatch
            && target.as_ref().is_some_and(|target| {
                let classname = game.require_entity(target).classname.clone();
                classname.starts_with("info_player_")
                    || classname == "misc_teleporter_dest"
                    || classname.starts_with("item_flag_")
            })
            && target.as_ref().is_some_and(|target| sight(game, target, &entity))
        {
            explode(&entity, game, "explosion1");
            return;
        }
        let trap_master = game.require_entity(&entity).team_master.clone();
        if !game.host.is_player(&actor) && !game.host.is_monster(&actor)
            || game.options.mode != Q2Mode::Deathmatch && game.host.is_player(&actor)
            || actor != trap_master.clone().unwrap_or(entity.clone())
                && !weapon_can_target(game, trap_master.as_ref(), &actor)
            || game
                .host
                .combat()
                .read(&actor)
                .map(|combat| combat.health)
                .unwrap_or(0.0)
                <= 0.0
            || target_body.is_none()
            || !sight(game, &entity, &actor)
        {
            continue;
        }
        let distance = f64::from(length3(sub3(
            body.origin,
            target_body.expect("trap target body is missing").origin,
        )));
        if best.is_none() || distance < nearest {
            best = Some(actor);
            nearest = distance;
        }
    }
    let target = best.clone().and_then(|best| game.host.actors().resolve_owned(&best));
    let target_body = best.clone().and_then(|best| game.host.bodies().read(&best));
    if let (Some(target), Some(target_body)) = (target, target_body) {
        let origin = if target_body.ground.is_none() {
            target_body.origin
        } else {
            add3(target_body.origin, vec3(0.0, 0.0, 1.0))
        };
        let delta = sub3(body.origin, origin);
        let distance = f64::from(length3(delta));
        let mut next = target_body.clone();
        next.origin = origin;
        next.ground = None;
        game.host.bodies().write(&target, &next);
        let target_id = target.id().clone();
        let maximum: f64 = if game.host.is_player(&target_id) { 290.0 } else { 150.0 };
        velocity(
            game,
            &target_id,
            add3(
                target_body.velocity,
                scale3(normalize3(delta), (64.0f64.max(maximum.min(maximum - distance))) as f32),
            ),
            true,
        );
        if game.require_entity(&entity).sound != "weapons/trapsuck.wav" {
            game.require_entity_mut(&entity).sound = "weapons/trapsuck.wav".to_string();
            game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
                actor: Some(entity.clone()),
                origin: body.origin,
                path: "weapons/trapsuck.wav".to_string(),
                channel: 0,
                volume: 1.0,
                attenuation: 1.0,
                reliable: false,
                loop_: Q2SoundLoop::Start,
                loop_owner: None,
            }));
        }
        if distance < 48.0 {
            let mass = game
                .host
                .combat()
                .read(&target_id)
                .map(|combat| combat.mass)
                .unwrap_or(0.0);
            if mass >= 400.0 {
                explode(&entity, game, "explosion1");
                return;
            }
            let owned = game.owned_of(entity.clone());
            game.host.combat().set_traits(
                &owned,
                &crate::q2::support::contracts::CombatTraitChanges {
                    can_take_damage: Some(false),
                    mass: None,
                    invulnerable: None,
                    team: None,
                    no_knockback: None,
                },
            );
            game.set_solid(entity.clone(), Q2Solid::None);
            game.require_entity_mut(&entity).die = None;
            game.damage(
                target_id.clone(),
                entity.clone(),
                game.require_entity(&entity).team_master.clone(),
                100000.0,
                1.0,
                Vec3::default(),
                origin,
                Vec3::default(),
                Q2_MISSION_PACK_DAMAGE.trap,
                0,
                Some("q2:ammo_trap".to_string()),
            );
            {
                let record = game.require_entity_mut(&entity);
                record.enemy = Some(target_id);
                record.wait = 64.0;
                record.timestamp = now + 30.0;
                record.accel = mass;
                record.frame = 5;
            }
            let owned = game.owned_of(entity.clone());
            game.host.combat().set_traits(
                &owned,
                &crate::q2::support::contracts::CombatTraitChanges {
                    can_take_damage: None,
                    mass: Some(
                        (mass
                            / if game.options.mode == Q2Mode::Deathmatch {
                                4.0
                            } else {
                                10.0
                            })
                        .trunc(),
                    ),
                    invulnerable: None,
                    team: None,
                    no_knockback: None,
                },
            );
            let gibs: Vec<ActorId> = game.entities.values().map(|gib| gib.actor.id().clone()).collect();
            for gib in gibs {
                if game.require_entity(&gib).classname == "gib"
                    && f64::from(length3(sub3(game.body_of(gib.clone()).origin, body.origin))) <= 128.0
                {
                    game.set_motion_kind(gib.clone(), Q2MotionKind::Stationary);
                    game.require_entity_mut(&gib).owner = Some(entity.clone());
                    trap_gib(gib, game);
                }
            }
        }
    }
    game.show(entity);
}

/// Trap think (`trapThink`).
fn trap_think(entity: ActorId, game: &mut Q2GameServices) {
    let now = game.host.now();
    let body = game.body_of(entity.clone());
    if game.require_entity(&entity).timestamp < now {
        explode(&entity, game, "explosion1");
        return;
    }
    game.schedule(entity.clone(), 0.1, trap_think as Q2Think);
    if body.ground.is_none() {
        return;
    }
    if game.require_entity(&entity).frame > 4 {
        if game.require_entity(&entity).frame == 5 {
            if game.require_entity(&entity).wait == 64.0 {
                game.sound(&entity, "weapons/trapdown.wav", 2, 1.0, 2.0);
            }
            game.require_entity_mut(&entity).wait -= 2.0;
            game.require_entity_mut(&entity).delay += now;
            let axes = angle_vectors(body.angles);
            for index in 0..3 {
                let gib = game.create("trap_gib", BTreeMap::new());
                let radians =
                    (120.0 * f64::from(index) + game.require_entity(&entity).delay) * std::f64::consts::PI / 180.0;
                let rotated = add3(
                    add3(
                        scale3(axes.right, radians.cos() as f32),
                        scale3(cross(axes.up, axes.right), radians.sin() as f32),
                    ),
                    scale3(axes.up, dot3(axes.up, axes.right) * (1.0 - radians.cos() as f32)),
                );
                let wait = game.require_entity(&entity).wait;
                let point = add3(
                    add3(body.origin, scale3(rotated, 1.0 + (wait / 2.0) as f32)),
                    axes.forward,
                );
                let style = game.require_entity(&entity).style;
                let mass = game
                    .host
                    .combat()
                    .read(&entity)
                    .map(|combat| combat.mass)
                    .unwrap_or(0.0);
                {
                    let record = game.require_entity_mut(&gib);
                    record.model = if style == 1 {
                        "models/objects/gekkgib/torso/tris.md2".to_string()
                    } else if mass > 200.0 {
                        "models/objects/gibs/chest/tris.md2".to_string()
                    } else {
                        "models/objects/gibs/sm_meat/tris.md2".to_string()
                    };
                    record.effects = (if style == 1 { 26 } else { 1 }) | 2;
                    record.server_flags |= 4;
                }
                let mut moved = game.body_of(gib.clone());
                moved.origin = vec3(point.x, point.y, body.origin.z + wait as f32);
                moved.angles = body.angles;
                moved.bounds.min = Vec3::default();
                moved.bounds.max = Vec3::default();
                game.write_body(gib.clone(), &moved, true);
                let owned = game.owned_of(gib.clone());
                game.host.combat().create(
                    &owned,
                    &CombatState {
                        health: 0.0,
                        armor: ArmorState {
                            regular: RegularArmorState::None,
                            powered: PoweredProtectionState::None,
                        },
                        mass: 0.0,
                        can_take_damage: true,
                        invulnerable: false,
                        no_knockback: false,
                        team: None,
                    },
                );
                game.set_solid(gib.clone(), Q2Solid::None);
                game.set_motion_kind(gib.clone(), Q2MotionKind::Toss);
                game.show(gib.clone());
                game.schedule(gib, 0.1, free_projectile);
            }
            if game.require_entity(&entity).wait < 19.0 {
                game.require_entity_mut(&entity).frame += 1;
            }
        } else {
            game.require_entity_mut(&entity).frame += 1;
            if game.require_entity(&entity).frame == 8 {
                game.schedule(entity.clone(), 1.0, free_projectile);
                let mut values = BTreeMap::new();
                values.insert(
                    "count".to_string(),
                    format!(
                        "{}",
                        game.host
                            .combat()
                            .read(&entity)
                            .map(|combat| combat.mass)
                            .unwrap_or(0.0)
                    ),
                );
                values.insert("spawnflags".to_string(), "65536".to_string());
                values.insert(
                    "origin".to_string(),
                    format!("{} {} {}", body.origin.x, body.origin.y, body.origin.z + 16.0),
                );
                let food = game.spawn(Q2SpawnFields {
                    ordinal: -1,
                    classname: "item_foodcube".to_string(),
                    values,
                });
                game.require_entity_mut(&food).classname = "foodcube".to_string();
                let mut moved = game.body_of(food.clone());
                moved.velocity = vec3(0.0, 0.0, 400.0);
                game.write_body(food.clone(), &moved, true);
                game.set_motion_kind(food, Q2MotionKind::Toss);
            }
        }
        game.show(entity);
        return;
    }
    game.require_entity_mut(&entity).effects &= !0x2000000;
    if game.require_entity(&entity).frame >= 4 {
        game.require_entity_mut(&entity).effects |= 0x2000000;
        let mut moved = game.body_of(entity.clone());
        moved.bounds.min = Vec3::default();
        moved.bounds.max = Vec3::default();
        game.write_body(entity.clone(), &moved, true);
    }
    if game.require_entity(&entity).frame < 4 {
        game.require_entity_mut(&entity).frame += 1;
    }
    let mut best: Option<ActorId> = None;
    let mut old_length = 8000.0;
    for actor in game.host.nearby(body.origin, 256.0) {
        if actor == entity
            || !game.host.is_player(&actor) && !game.host.is_monster(&actor)
            || game
                .host
                .combat()
                .read(&actor)
                .map(|combat| combat.health)
                .unwrap_or(0.0)
                <= 0.0
            || !sight(game, &entity, &actor)
        {
            continue;
        }
        if best.is_none() {
            best = Some(actor);
            continue;
        }
        let Some(target) = game.host.bodies().read(&actor) else {
            continue;
        };
        let distance = f64::from(length3(sub3(body.origin, target.origin)).trunc());
        if distance < old_length {
            old_length = distance;
            best = Some(actor);
        }
    }
    if let Some(best) = best {
        let owned = game.host.actors().resolve_owned(&best);
        let target_body = game.host.bodies().read(&best);
        if let (Some(owned), Some(target_body)) = (owned, target_body) {
            let origin = if target_body.ground.is_none() {
                target_body.origin
            } else {
                add3(target_body.origin, vec3(0.0, 0.0, 1.0))
            };
            let mut next = target_body.clone();
            next.origin = origin;
            next.ground = None;
            game.host.bodies().write(&owned, &next);
            let delta = sub3(body.origin, origin);
            let distance = f64::from(length3(delta).trunc());
            if game.host.is_player(&best) {
                velocity(
                    game,
                    &best,
                    add3(target_body.velocity, scale3(normalize3(delta), 250.0)),
                    true,
                );
            } else {
                let mut monster = (super::mission_hooks(game).monster)(best.clone(), game);
                if let Some(monster) = monster.as_mut() {
                    monster.state_mut().ideal_yaw = f64::from(vector_angles(delta).y);
                    change_yaw(monster);
                }
                let angles = game
                    .host
                    .bodies()
                    .read(&best)
                    .map(|body| body.angles)
                    .unwrap_or(target_body.angles);
                velocity(game, &best, scale3(angle_vectors(angles).forward, 256.0), true);
            }
            game.sound(&entity, "weapons/trapsuck.wav", 2, 1.0, 2.0);
            if distance < 32.0 {
                let mass = game.host.combat().read(&best).map(|combat| combat.mass).unwrap_or(0.0);
                if mass >= 400.0 {
                    explode(&entity, game, "explosion1");
                    return;
                }
                let style =
                    if game.entity(&best).map(|entity| entity.classname.clone()).as_deref() == Some("monster_gekk") {
                        1
                    } else {
                        0
                    };
                game.require_entity_mut(&entity).style = style;
                game.damage(
                    best.clone(),
                    entity.clone(),
                    game.require_entity(&entity).owner.clone(),
                    100000.0,
                    1.0,
                    Vec3::default(),
                    origin,
                    Vec3::default(),
                    Q2_MISSION_PACK_DAMAGE.trap,
                    0,
                    Some("q2:ammo_trap".to_string()),
                );
                {
                    let record = game.require_entity_mut(&entity);
                    record.enemy = Some(best);
                    record.wait = 64.0;
                    record.timestamp = now + 30.0;
                }
                let owned = game.owned_of(entity.clone());
                game.host.combat().set_traits(
                    &owned,
                    &crate::q2::support::contracts::CombatTraitChanges {
                        can_take_damage: None,
                        mass: Some(
                            (mass
                                / if game.options.mode == Q2Mode::Deathmatch {
                                    4.0
                                } else {
                                    10.0
                                })
                            .trunc(),
                        ),
                        invulnerable: None,
                        team: None,
                        no_knockback: None,
                    },
                );
                game.require_entity_mut(&entity).frame = 5;
            }
        }
    }
    game.show(entity);
}
