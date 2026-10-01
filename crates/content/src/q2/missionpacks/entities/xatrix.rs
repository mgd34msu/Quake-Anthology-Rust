//! Xatrix entities (`src/content/q2/missionpacks/entities/xatrix.ts`).
//!
//! Xatrix g_func.c/g_misc.c source-only scenery and campaign entities
//! (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::{normalize3, sub3, vec3, Vec3};

use crate::contract::{ArmorState, PoweredProtectionState, RegularArmorState};
use crate::q2::base::entities::targets::target_laser_think;
use crate::q2::foundation::callbacks::{free_q2_entity, Q2CallbackDefinitions};
use crate::q2::foundation::fields::{movedir, number_field};
use crate::q2::foundation::host::{
    Q2BeamEvent, Q2Die, Q2EffectEvent, Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2Think, Q2Use,
};
use crate::q2::foundation::weapons::ballistics::fire_rocket;
use crate::q2::support::contracts::{CombatState, DeathReaction};

use super::types::{mission_entity_hooks, Q2MissionPackEntityHooks};

/// Xatrix entity callbacks (`Q2XatrixEntities::callbacks`).
pub fn xatrix_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks
        .think
        .insert("rotating_light_alarm", rotating_light_alarm as Q2Think);
    callbacks
        .think
        .insert("object_repair_sparks", object_repair_sparks as Q2Think);
    callbacks
        .think
        .insert("object_repair_dead", object_repair_dead as Q2Think);
    callbacks.think.insert("object_repair_fx", object_repair_fx as Q2Think);
    callbacks.think.insert("amb4_think", amb4_think as Q2Think);
    callbacks.think.insert("mal_laser_think", mal_laser_think as Q2Think);
    callbacks
        .think
        .insert("target_laser_think", target_laser_think as Q2Think);
    callbacks
        .die
        .insert("rotating_light_killed", rotating_light_killed as Q2Die);
    callbacks.use_.insert("rotating_light_use", rotating_light_use as Q2Use);
    callbacks
        .use_
        .insert("misc_viper_missile_use", misc_viper_missile_use as Q2Use);
    callbacks.use_.insert("use_nuke", use_nuke as Q2Use);
    callbacks
        .use_
        .insert("target_mal_laser_use", target_mal_laser_use as Q2Use);
    callbacks
}

/// Xatrix entities (`Q2XatrixEntities`).
#[derive(Debug, Clone, Copy)]
pub struct Q2XatrixEntities {
    /// Entity hooks.
    pub hooks: Q2MissionPackEntityHooks,
}

impl Q2XatrixEntities {
    /// Spawn a Xatrix entity (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        let classname = game.require_entity(&entity).classname.clone();
        match classname.as_str() {
            "rotating_light" => {
                let spawn = game.require_entity(&entity).spawn.clone();
                let mut health = number_field(&spawn, "health", 0.0);
                if health == 0.0 {
                    health = 10.0;
                }
                {
                    let record = game.require_entity_mut(&entity);
                    record.model = "models/objects/light/tris.md2".to_string();
                    if record.speed == 0.0 {
                        record.speed = 32.0;
                    }
                    record.max_health = health;
                    record.frame = 0;
                    record.use_ = Some(rotating_light_use as Q2Use);
                    record.die = Some(rotating_light_killed as Q2Die);
                    if record.spawnflags & 1 != 0 {
                        record.effects &= !0x800000;
                    } else {
                        record.effects |= 0x800000;
                    }
                }
                let owned = game.owned_of(entity.clone());
                game.host.combat().create(
                    &owned,
                    &CombatState {
                        health,
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
                game.set_solid(entity.clone(), Q2Solid::Box);
                game.set_motion_kind(entity.clone(), Q2MotionKind::Stop);
                game.show(entity);
            }
            "func_object_repair" => {
                game.require_entity_mut(&entity).classname = "object_repair".to_string();
                if game.require_entity(&entity).delay == 0.0 {
                    game.require_entity_mut(&entity).delay = 1.0;
                }
                let owned = game.owned_of(entity.clone());
                game.host.combat().create(
                    &owned,
                    &CombatState {
                        health: 100.0,
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
                let mut moved = game.body_of(entity.clone());
                moved.bounds.min = vec3(-8.0, -8.0, 8.0);
                moved.bounds.max = vec3(8.0, 8.0, 8.0);
                game.write_body(entity.clone(), &moved, true);
                game.set_solid(entity.clone(), Q2Solid::Box);
                game.set_motion_kind(entity.clone(), Q2MotionKind::Stationary);
                game.schedule(entity, 1.0, object_repair_sparks as Q2Think);
            }
            "misc_viper_missile" => {
                {
                    let record = game.require_entity_mut(&entity);
                    if record.damage == 0.0 {
                        record.damage = 250.0;
                    }
                    record.model = "models/objects/bomb/tris.md2".to_string();
                    record.visible = false;
                    record.use_ = Some(misc_viper_missile_use as Q2Use);
                }
                let mut moved = game.body_of(entity.clone());
                moved.bounds.min = vec3(-8.0, -8.0, -8.0);
                moved.bounds.max = vec3(8.0, 8.0, 8.0);
                game.write_body(entity.clone(), &moved, true);
                game.set_solid(entity.clone(), Q2Solid::None);
                game.set_motion_kind(entity.clone(), Q2MotionKind::Stationary);
                game.show(entity);
            }
            "misc_amb4" => {
                game.schedule(entity, 1.0, amb4_think as Q2Think);
            }
            "misc_nuke" => {
                game.require_entity_mut(&entity).use_ = Some(use_nuke as Q2Use);
            }
            "misc_crashviper" | "misc_transport" => {
                if game.require_entity(&entity).target.is_empty() {
                    let classname = game.require_entity(&entity).classname.clone();
                    game.host.diagnostic(&format!("{classname} without a target"));
                    game.remove_actor(entity);
                    return true;
                }
                let transport = game.require_entity(&entity).classname == "misc_transport";
                let angles = game.body_of(entity.clone()).angles;
                mission_entity_hooks(game).movers.spawn_train(entity.clone(), game);
                game.require_entity_mut(&entity).blocked = None;
                game.require_entity_mut(&entity).model = if transport {
                    "models/objects/ship/tris.md2".to_string()
                } else {
                    "models/ships/bigviper/tris.md2".to_string()
                };
                let mut moved = game.body_of(entity.clone());
                moved.angles = angles;
                game.write_body(entity.clone(), &moved, true);
                if transport {
                    game.require_entity_mut(&entity).spawnflags |= 1;
                }
                game.show(entity);
            }
            "target_mal_laser" => {
                {
                    let record = game.require_entity_mut(&entity);
                    record.render_flags |= 0xa0;
                    record.frame = if record.spawnflags & 64 != 0 { 16 } else { 4 };
                    record.skin = if record.spawnflags & 2 != 0 {
                        0xf2f2f0f0u32 as i32
                    } else if record.spawnflags & 4 != 0 {
                        0xd0d1d2d3u32 as i32
                    } else if record.spawnflags & 8 != 0 {
                        0xf3f3f1f1u32 as i32
                    } else if record.spawnflags & 16 != 0 {
                        0xdcdddedfu32 as i32
                    } else if record.spawnflags & 32 != 0 {
                        0xe0e1e2e3u32 as i32
                    } else {
                        0
                    };
                }
                let angles = game.body_of(entity.clone()).angles;
                game.require_entity_mut(&entity).movedir = movedir(angles);
                if game.require_entity(&entity).delay == 0.0 {
                    game.require_entity_mut(&entity).delay = 0.1;
                }
                if game.require_entity(&entity).wait == 0.0 {
                    game.require_entity_mut(&entity).wait = 0.1;
                }
                if game.require_entity(&entity).damage == 0.0 {
                    game.require_entity_mut(&entity).damage = 5.0;
                }
                game.require_entity_mut(&entity).use_ = Some(target_mal_laser_use as Q2Use);
                let mut moved = game.body_of(entity.clone());
                moved.angles = Vec3::default();
                moved.bounds.min = vec3(-8.0, -8.0, -8.0);
                moved.bounds.max = vec3(8.0, 8.0, 8.0);
                game.write_body(entity.clone(), &moved, true);
                game.set_solid(entity.clone(), Q2Solid::None);
                game.set_motion_kind(entity.clone(), Q2MotionKind::Stationary);
                if game.require_entity(&entity).spawnflags & 1 != 0 {
                    mal_on(entity, game);
                } else {
                    mal_off(entity, game);
                }
            }
            _ => return false,
        }
        game.source_callbacks.register(&xatrix_callbacks());
        true
    }
}

/// Emit repair sparks (`sparks`).
fn xatrix_sparks(entity: &ActorId, game: &mut Q2GameServices, count: i32) {
    let origin = game.body_of(entity.clone()).origin;
    let color = 0xe0 + (game.host.random() * 8.0).floor() as i32;
    game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:welding_sparks".to_string(),
        origin,
        direction: Vec3::default(),
        count,
        color,
    }));
}

/// Rotating light alarm (`alarm`).
fn rotating_light_alarm(entity: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&entity).spawnflags & 1 != 0 {
        game.cancel_actor(entity);
        return;
    }
    game.sound(&entity, "misc/alarm.wav", 10, 1.0, 3.0);
    game.schedule(entity, 1.0, rotating_light_alarm as Q2Think);
}

/// Rotating light killed (`lightKilled`).
fn rotating_light_killed(entity: ActorId, game: &mut Q2GameServices, _reaction: DeathReaction) {
    xatrix_sparks(&entity, game, 30);
    game.require_entity_mut(&entity).effects &= !0x800000;
    game.require_entity_mut(&entity).use_ = None;
    game.show(entity.clone());
    game.schedule(entity, 0.1, free_q2_entity);
}

/// Rotating light use (`lightUse`).
fn rotating_light_use(
    entity: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    if game.require_entity(&entity).spawnflags & 1 != 0 {
        game.require_entity_mut(&entity).spawnflags &= !1;
        game.require_entity_mut(&entity).effects |= 0x800000;
        if game.require_entity(&entity).spawnflags & 2 != 0 {
            game.schedule(entity.clone(), 0.1, rotating_light_alarm as Q2Think);
        }
    } else {
        game.require_entity_mut(&entity).spawnflags |= 1;
        game.require_entity_mut(&entity).effects &= !0x800000;
    }
    game.show(entity);
}

/// Repair effect (`repairFx`).
fn object_repair_fx(entity: ActorId, game: &mut Q2GameServices) {
    let Some(health) = game.host.combat().read(&entity).map(|combat| combat.health) else {
        panic!("Source repair object lost health");
    };
    if health <= 100.0 {
        let owned = game.owned_of(entity.clone());
        game.host.combat().set_health(&owned, health + 1.0);
    } else {
        xatrix_sparks(&entity, game, 10);
    }
    let delay = game.require_entity(&entity).delay;
    game.schedule(entity, delay, object_repair_fx as Q2Think);
}

/// Repair dead (`repairDead`).
fn object_repair_dead(entity: ActorId, game: &mut Q2GameServices) {
    let authored = game.require_entity(&entity).authored_target();
    let id = entity.clone();
    game.use_targets(&authored, Some(&id), false);
    if game.host.actors().is_live(&entity) {
        game.schedule(entity, 0.1, object_repair_fx as Q2Think);
    }
}

/// Repair sparks (`repairSparks`).
fn object_repair_sparks(entity: ActorId, game: &mut Q2GameServices) {
    if game
        .host
        .combat()
        .read(&entity)
        .map(|combat| combat.health)
        .unwrap_or(0.0)
        < 0.0
    {
        game.schedule(entity, 0.1, object_repair_dead as Q2Think);
        return;
    }
    xatrix_sparks(&entity, game, 10);
    let delay = game.require_entity(&entity).delay;
    game.schedule(entity, delay, object_repair_sparks as Q2Think);
}

/// Ambience (`ambience`).
fn amb4_think(entity: ActorId, game: &mut Q2GameServices) {
    game.sound(&entity, "world/amb4.wav", 2, 1.0, 0.0);
    game.schedule(entity, 2.7, amb4_think as Q2Think);
}

/// Viper missile use (`missileUse`).
fn misc_viper_missile_use(
    entity: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let target_name = game.require_entity(&entity).target.clone();
    let Some(target) = game.targets(&target_name).into_iter().next() else {
        game.host
            .diagnostic(&format!("misc_viper_missile missing target {target_name}"));
        return;
    };
    game.require_entity_mut(&entity).enemy = Some(target.clone());
    let origin = game.body_of(entity.clone()).origin;
    let direction = normalize3(sub3(game.body_of(target).origin, origin));
    let damage = game.require_entity(&entity).damage;
    fire_rocket(
        entity.clone(),
        game,
        origin,
        direction,
        damage,
        500.0,
        damage + 20.0,
        damage,
    );
    game.host.emit(Q2PresentationEvent::MonsterMuzzleflash {
        actor: entity.clone(),
        flash: 57,
        origin,
        direction,
    });
    game.schedule(entity, 0.1, free_q2_entity);
}

/// Nuke use (`nukeUse`).
fn use_nuke(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let targets: Vec<ActorId> = game.entities.values().map(|entity| entity.actor.id().clone()).collect();
    for target in targets {
        if target == entity {
            continue;
        }
        if game.host.is_player(&target) {
            let origin = game.body_of(target.clone()).origin;
            game.damage(
                target,
                entity.clone(),
                Some(entity.clone()),
                100000.0,
                1.0,
                Vec3::default(),
                origin,
                Vec3::default(),
                39,
                0,
                None,
            );
        } else if game.host.is_monster(&target) {
            game.remove_actor(target);
        }
    }
    game.require_entity_mut(&entity).use_ = None;
}

/// Mal think (`malThink`).
fn mal_laser_think(entity: ActorId, game: &mut Q2GameServices) {
    target_laser_think(entity.clone(), game);
    game.require_entity_mut(&entity).spawnflags |= 0x80000000u32 as i32;
    let wait = game.require_entity(&entity).wait;
    game.schedule(entity, wait + 0.1, mal_laser_think as Q2Think);
}

/// Turn the mal laser on (`malOn`).
fn mal_on(entity: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&entity).activator.is_none() {
        game.require_entity_mut(&entity).activator = Some(entity.clone());
    }
    game.require_entity_mut(&entity).spawnflags |= 0x80000001u32 as i32;
    game.require_entity_mut(&entity).visible = true;
    game.show(entity.clone());
    let (wait, delay) = {
        let record = game.require_entity(&entity);
        (record.wait, record.delay)
    };
    game.schedule(entity, wait + delay, mal_laser_think as Q2Think);
}

/// Turn the mal laser off (`malOff`).
fn mal_off(entity: ActorId, game: &mut Q2GameServices) {
    game.require_entity_mut(&entity).spawnflags &= !1;
    game.require_entity_mut(&entity).visible = false;
    game.cancel_actor(entity.clone());
    game.show(entity.clone());
    let origin = game.body_of(entity.clone()).origin;
    let (frame, skin) = {
        let record = game.require_entity(&entity);
        (record.frame, record.skin)
    };
    game.host.emit(Q2PresentationEvent::Beam(Q2BeamEvent {
        actor: entity,
        start: origin,
        end: origin,
        width: f64::from(frame),
        color: skin,
        visible: false,
    }));
}

/// Mal laser use (`malUse`).
fn target_mal_laser_use(
    entity: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    activator: Option<ActorId>,
) {
    game.require_entity_mut(&entity).activator = activator;
    if game.require_entity(&entity).spawnflags & 1 != 0 {
        mal_off(entity, game);
    } else {
        mal_on(entity, game);
    }
}
