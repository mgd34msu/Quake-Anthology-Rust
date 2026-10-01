//! Rogue entities (`src/content/q2/missionpacks/entities/rogue.ts`).
//!
//! Rogue g_newtrig.c/g_newtarg.c and misc_nuke_core (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::{add3, normalize3, scale3, sub3, vec3, Vec3};

use crate::contract::{ArmorState, PoweredProtectionState, RegularArmorState};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::{integer_field, movedir};
use crate::q2::foundation::host::{
    Q2EffectEvent, Q2GameServices, Q2Mode, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2Think, Q2Touch, Q2Use,
};
use crate::q2::foundation::scenery::kill_q2_box;
use crate::q2::support::contracts::{CombatState, TouchContact};

use super::super::projectiles::common::sight;
use super::types::{mission_entity_hooks, Q2MissionPackEntityEvent, Q2MissionPackEntityHooks};

/// Rogue entities checkpoint (`Q2RogueEntitiesCheckpoint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2RogueEntitiesCheckpoint {
    /// Steam id.
    pub steam_id: i32,
}

/// Rogue entity callbacks (`Q2RogueEntities::callbacks`).
pub fn rogue_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks
        .think
        .insert("target_steam_start", target_steam_start as Q2Think);
    callbacks.think.insert("blacklight_think", blacklight_think as Q2Think);
    callbacks.think.insert("orb_think", orb_think as Q2Think);
    callbacks
        .touch
        .insert("trigger_teleport_touch", trigger_teleport_touch as Q2Touch);
    callbacks
        .touch
        .insert("trigger_disguise_touch", trigger_disguise_touch as Q2Touch);
    callbacks
        .use_
        .insert("trigger_teleport_use", trigger_teleport_use as Q2Use);
    callbacks
        .use_
        .insert("trigger_disguise_use", trigger_disguise_use as Q2Use);
    callbacks.use_.insert("use_target_steam", use_target_steam as Q2Use);
    callbacks.use_.insert("target_anger_use", target_anger_use as Q2Use);
    callbacks
        .use_
        .insert("target_killplayers_use", target_killplayers_use as Q2Use);
    callbacks.use_.insert("misc_nuke_core_use", misc_nuke_core_use as Q2Use);
    callbacks
}

/// Rogue entities (`Q2RogueEntities`).
#[derive(Debug, Clone, Copy)]
pub struct Q2RogueEntities {
    /// Entity hooks.
    pub hooks: Q2MissionPackEntityHooks,
}

impl Q2RogueEntities {
    /// Spawn a Rogue entity (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        let classname = game.require_entity(&entity).classname.clone();
        match classname.as_str() {
            "info_teleport_destination" => {}
            "trigger_teleport" => {
                if game.require_entity(&entity).wait == 0.0 {
                    game.require_entity_mut(&entity).wait = 0.2;
                }
                let delay = if !game.require_entity(&entity).targetname.is_empty()
                    && game.require_entity(&entity).spawnflags & 8 == 0
                {
                    1.0
                } else {
                    0.0
                };
                game.require_entity_mut(&entity).delay = delay;
                let use_ = if game.require_entity(&entity).targetname.is_empty() {
                    None
                } else {
                    Some(trigger_teleport_use as Q2Use)
                };
                game.require_entity_mut(&entity).use_ = use_;
                game.require_entity_mut(&entity).touch = Some(trigger_teleport_touch as Q2Touch);
                let angles = game.body_of(entity.clone()).angles;
                game.require_entity_mut(&entity).movedir = movedir(angles);
                let mut moved = game.body_of(entity.clone());
                moved.angles = Vec3::default();
                game.write_body(entity.clone(), &moved, true);
                game.set_solid(entity.clone(), Q2Solid::Trigger);
                game.set_motion_kind(entity.clone(), Q2MotionKind::Stationary);
            }
            "trigger_disguise" => {
                game.require_entity_mut(&entity).touch = Some(trigger_disguise_touch as Q2Touch);
                game.require_entity_mut(&entity).use_ = Some(trigger_disguise_use as Q2Use);
                game.require_entity_mut(&entity).visible = false;
                let solid = if game.require_entity(&entity).spawnflags & 2 != 0 {
                    Q2Solid::Trigger
                } else {
                    Q2Solid::None
                };
                game.set_solid(entity.clone(), solid);
                game.set_motion_kind(entity.clone(), Q2MotionKind::Stationary);
                game.show(entity);
            }
            "target_steam" => {
                if !game.require_entity(&entity).target.is_empty() {
                    game.schedule(entity, 1.0, target_steam_start as Q2Think);
                } else {
                    target_steam_start(entity, game);
                }
            }
            "target_anger" => {
                let (target, killtarget) = {
                    let record = game.require_entity(&entity);
                    (record.target.clone(), record.killtarget.clone())
                };
                if target.is_empty() || killtarget.is_empty() {
                    let missing = if target.is_empty() { "target" } else { "killtarget" };
                    game.host.diagnostic(&format!("target_anger without {missing}!"));
                    game.remove_actor(entity);
                } else {
                    game.require_entity_mut(&entity).use_ = Some(target_anger_use as Q2Use);
                    game.require_entity_mut(&entity).visible = false;
                    game.show(entity);
                }
            }
            "target_killplayers" => {
                game.require_entity_mut(&entity).use_ = Some(target_killplayers_use as Q2Use);
                game.require_entity_mut(&entity).visible = false;
                game.show(entity);
            }
            "target_blacklight" | "target_orb" => {
                if game.options.mode == Q2Mode::Deathmatch {
                    game.remove_actor(entity);
                    return true;
                }
                let orb = game.require_entity(&entity).classname == "target_orb";
                game.require_entity_mut(&entity).model = "models/items/spawngro2/tris.md2".to_string();
                game.require_entity_mut(&entity).effects |= if orb {
                    0x10000000
                } else {
                    0x80000000u32 as i64 | 0x4000000
                };
                game.require_entity_mut(&entity).frame = if orb { 2 } else { 1 };
                let mut moved = game.body_of(entity.clone());
                moved.bounds.min = Vec3::default();
                moved.bounds.max = Vec3::default();
                game.write_body(entity.clone(), &moved, true);
                game.schedule(
                    entity.clone(),
                    0.1,
                    if orb {
                        orb_think as Q2Think
                    } else {
                        blacklight_think as Q2Think
                    },
                );
                game.show(entity);
            }
            "misc_nuke_core" => {
                game.require_entity_mut(&entity).model = "models/objects/core/tris.md2".to_string();
                game.require_entity_mut(&entity).use_ = Some(misc_nuke_core_use as Q2Use);
                game.show(entity);
            }
            _ => return false,
        }
        game.source_callbacks.register(&rogue_callbacks());
        true
    }

    /// Capture the steam id (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> Q2RogueEntitiesCheckpoint {
        let _ = self;
        Q2RogueEntitiesCheckpoint {
            steam_id: game.mission_packs.steam_id,
        }
    }

    /// Restore the steam id (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, state: Q2RogueEntitiesCheckpoint) {
        let _ = self;
        game.mission_packs.steam_id = state.steam_id;
    }
}

/// Teleport use (`teleportUse`).
fn trigger_teleport_use(
    entity: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let delay = game.require_entity(&entity).delay;
    game.require_entity_mut(&entity).delay = if delay == 0.0 { 1.0 } else { 0.0 };
}

/// Teleport touch (`teleportTouch`).
fn trigger_teleport_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if !game.host.is_player(&contact.other) || game.require_entity(&entity).delay != 0.0 {
        return;
    }
    let target_name = game.require_entity(&entity).target.clone();
    let destination = game.targets(&target_name).into_iter().next();
    let player = game.entity(&contact.other).map(|entity| entity.actor.id().clone());
    let (Some(destination), Some(player)) = (destination, player) else {
        if game.targets(&target_name).is_empty() {
            game.host.diagnostic("Teleport Destination not found!");
        }
        return;
    };
    let player_origin = game.body_of(player.clone()).origin;
    game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: "q2:teleport_effect".to_string(),
        origin: player_origin,
        direction: Vec3::default(),
        count: 1,
        color: 0,
    }));
    let body = game.body_of(destination);
    let origin = add3(body.origin, vec3(0.0, 0.0, 10.0));
    let owned = game.owned_of(player.clone());
    game.host.bodies().unlink(&owned);
    let mut moved = game.body_of(player.clone());
    moved.origin = origin;
    moved.velocity = Vec3::default();
    moved.angles = Vec3::default();
    game.write_body(player.clone(), &moved, false);
    (mission_entity_hooks(game).teleport_player)(player.clone(), game, origin, body.angles);
    game.host.emit(Q2PresentationEvent::EntityEvent {
        actor: player.clone(),
        event: 6,
    });
    kill_q2_box(game, player.clone());
    game.link_actor(player);
}

/// Disguise touch (`disguiseTouch`).
fn trigger_disguise_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if !game.host.is_player(&contact.other) {
        return;
    }
    if game.entity(&contact.other).is_some() {
        let disguised = game.require_entity(&entity).spawnflags & 4 == 0;
        let flags = game.require_entity(&contact.other).flags;
        game.require_entity_mut(&contact.other).flags = if disguised { flags | 0x8000 } else { flags & !0x8000 };
    }
}

/// Disguise use (`disguiseUse`).
fn trigger_disguise_use(
    entity: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let solid = game.require_entity(&entity).solid;
    game.set_solid(
        entity,
        if solid == Q2Solid::None {
            Q2Solid::Trigger
        } else {
            Q2Solid::None
        },
    );
}

/// Nuke core use (`coreUse`).
fn misc_nuke_core_use(
    entity: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let visible = game.require_entity(&entity).visible;
    game.require_entity_mut(&entity).visible = !visible;
    game.show(entity);
}

/// Steam start (`steamStart`).
fn target_steam_start(entity: ActorId, game: &mut Q2GameServices) {
    game.require_entity_mut(&entity).use_ = Some(use_target_steam as Q2Use);
    if !game.require_entity(&entity).target.is_empty() {
        let target_name = game.require_entity(&entity).target.clone();
        let enemy = game.targets(&target_name).into_iter().next();
        if enemy.is_none() {
            let classname = game.require_entity(&entity).classname.clone();
            game.host
                .diagnostic(&format!("{classname}: {target_name} is a bad target"));
        }
        game.require_entity_mut(&entity).enemy = enemy;
    } else {
        let angles = game.body_of(entity.clone()).angles;
        game.require_entity_mut(&entity).movedir = movedir(angles);
        let mut moved = game.body_of(entity.clone());
        moved.angles = Vec3::default();
        game.write_body(entity.clone(), &moved, true);
    }
    let count = game.require_entity(&entity).count;
    game.require_entity_mut(&entity).count = (if count == 0 { 32 } else { count }) & 255;
    if game.require_entity(&entity).speed == 0.0 {
        game.require_entity_mut(&entity).speed = 75.0;
    }
    let spawn = game.require_entity(&entity).spawn.clone();
    let mut sounds = integer_field(&spawn, "sounds", 0);
    if sounds == 0 {
        sounds = 8;
    }
    game.require_entity_mut(&entity).style = sounds & 255;
    let wait = game.require_entity(&entity).wait;
    game.require_entity_mut(&entity).wait = wait * 1000.0;
    game.require_entity_mut(&entity).visible = false;
    game.show(entity);
}

/// Steam use (`steamUse`).
fn use_target_steam(entity: ActorId, game: &mut Q2GameServices, other: Option<ActorId>, _activator: Option<ActorId>) {
    if game.mission_packs.steam_id > 20000 {
        game.mission_packs.steam_id %= 20000;
    }
    game.mission_packs.steam_id += 1;
    if game.require_entity(&entity).wait == 0.0 {
        let wait = match other {
            None => 1000.0,
            Some(other) => game.entity(&other).map(|entity| entity.wait).unwrap_or(0.0) * 1000.0,
        };
        game.require_entity_mut(&entity).wait = wait;
    }
    let enemy = game.require_entity(&entity).enemy.clone();
    let target = enemy.as_ref().and_then(|enemy| game.host.bodies().read(enemy));
    let origin = game.body_of(entity.clone()).origin;
    if let Some(target) = target {
        let center = add3(target.origin, scale3(add3(target.bounds.min, target.bounds.max), 0.5));
        game.require_entity_mut(&entity).movedir = normalize3(sub3(center, origin));
    }
    let steam_id = game.mission_packs.steam_id;
    let record = game.require_entity(&entity);
    let wait = record.wait;
    let movedir = record.movedir;
    let count = record.count;
    let style = record.style;
    let speed = record.speed;
    (mission_entity_hooks(game).emit)(
        game,
        Q2MissionPackEntityEvent::Steam {
            id: if wait > 100.0 { steam_id } else { -1 },
            origin,
            direction: movedir,
            count,
            color: style,
            speed: speed.trunc() as i32,
            milliseconds: if wait > 100.0 { wait.trunc() as i32 } else { 0 },
        },
    );
}

/// Anger use (`angerUse`).
fn target_anger_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let killtarget = game.require_entity(&entity).killtarget.clone();
    let Some(target) = game.targets(&killtarget).into_iter().next() else {
        return;
    };
    if game.require_entity(&entity).target.is_empty() {
        return;
    }
    game.require_entity_mut(&target).server_flags |= 4;
    if game.host.combat().read(&target).is_none() {
        let owned = game.owned_of(target.clone());
        game.host.combat().create(
            &owned,
            &CombatState {
                health: 300.0,
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
    } else {
        let owned = game.owned_of(target.clone());
        game.host.combat().set_health(&owned, 300.0);
    }
    let target_name = game.require_entity(&entity).target.clone();
    for monster in game.targets(&target_name) {
        if monster == entity {
            game.host.diagnostic("WARNING: entity used itself.");
        } else if game.require_entity(&monster).use_.is_some() {
            if game
                .host
                .combat()
                .read(&monster)
                .map(|combat| combat.health)
                .unwrap_or(0.0)
                < 0.0
            {
                return;
            }
            (mission_entity_hooks(game).target_anger)(monster, target.clone(), game);
        }
        if !game.host.actors().is_live(&entity) {
            game.host.diagnostic("entity was removed while using targets");
            return;
        }
    }
}

/// Kill players use (`killPlayers`).
fn target_killplayers_use(
    entity: ActorId,
    game: &mut Q2GameServices,
    _other: Option<ActorId>,
    _activator: Option<ActorId>,
) {
    let origin = game.body_of(entity.clone()).origin;
    for player in game.host.players() {
        game.damage(
            player,
            entity.clone(),
            Some(entity.clone()),
            100000.0,
            0.0,
            Vec3::default(),
            origin,
            Vec3::default(),
            21,
            32,
            None,
        );
    }
    let targets: Vec<ActorId> = game.entities.values().map(|entity| entity.actor.id().clone()).collect();
    for target in targets {
        let Some(combat) = game.host.combat().read(&target) else {
            continue;
        };
        if combat.health < 1.0 || !combat.can_take_damage {
            continue;
        }
        for actor in game.host.players() {
            let player = game.entity(&actor).map(|entity| entity.actor.id().clone());
            if let Some(player) = player {
                if sight(game, &player, &target) {
                    let origin = game.body_of(target.clone()).origin;
                    game.damage(
                        target.clone(),
                        entity.clone(),
                        Some(entity.clone()),
                        combat.health,
                        0.0,
                        Vec3::default(),
                        origin,
                        Vec3::default(),
                        21,
                        32,
                        None,
                    );
                    break;
                }
            }
        }
    }
}

/// Randomize angles (`randomAngles`).
fn rogue_random_angles(entity: &ActorId, game: &mut Q2GameServices) {
    let mut moved = game.body_of(entity.clone());
    moved.angles = vec3(
        (game.host.random() * 360.0).floor() as f32,
        (game.host.random() * 360.0).floor() as f32,
        (game.host.random() * 360.0).floor() as f32,
    );
    game.write_body(entity.clone(), &moved, true);
}

/// Blacklight think (`blackLight`).
fn blacklight_think(entity: ActorId, game: &mut Q2GameServices) {
    rogue_random_angles(&entity, game);
    game.schedule(entity, 0.1, blacklight_think as Q2Think);
}

/// Orb think (`orb`).
fn orb_think(entity: ActorId, game: &mut Q2GameServices) {
    rogue_random_angles(&entity, game);
    game.schedule(entity, 0.1, orb_think as Q2Think);
}
