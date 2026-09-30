//! Q2 CTF techs (`src/content/q2/multiplayer/ctf/techs.ts`).
//!
//! Zoid's original CTF 1.09b g_ctf.c techs, using shared items and combat.
//! GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3, add3, scale3, vec3};

use crate::contract::RegularArmorState;
use crate::q2::base::player::spawns::q2_entities_named;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::movedir;
use crate::q2::foundation::host::{Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop, Q2Think};
use crate::q2::foundation::items::{Q2ConsoleGive, Q2DropOptions, Q2ItemDefinition, Q2ItemKindData};
use crate::q2::foundation::weapons::ballistics::silencer_shots;

use super::types::{Q2CtfHooks, Q2CtfMatchPhase, ctf_player, item_id};

/// CTF tech info.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Q2CtfTechInfo {
    /// Classname.
    classname: &'static str,
    /// Item id.
    item: &'static str,
    /// Model name.
    model: &'static str,
    /// Icon.
    icon: &'static str,
    /// Display name.
    name: &'static str,
}

/// CTF techs by ordinal.
const TECHS: [Q2CtfTechInfo; 4] = [
    Q2CtfTechInfo { classname: "item_tech1", item: "q2:item_tech1", model: "resistance", icon: "tech1", name: "Disruptor Shield" },
    Q2CtfTechInfo { classname: "item_tech2", item: "q2:item_tech2", model: "strength", icon: "tech2", name: "Power Amplifier" },
    Q2CtfTechInfo { classname: "item_tech3", item: "q2:item_tech3", model: "haste", icon: "tech3", name: "Time Accel" },
    Q2CtfTechInfo { classname: "item_tech4", item: "q2:item_tech4", model: "regeneration", icon: "tech4", name: "AutoDoc" },
];

/// Tech lifetime seconds.
const TECH_TIMEOUT: f64 = 60.0;

/// CTF techs (`Q2CtfTechs`).
#[derive(Debug, Clone, Copy)]
pub struct Q2CtfTechs {
    /// Session hooks.
    pub hooks: Q2CtfHooks,
}

/// Tech callbacks (`Q2CtfTechs::callbacks`).
pub fn ctf_tech_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("TechThink", ctf_tech_think as Q2Think);
    callbacks.think.insert("SpawnTechs", ctf_spawn_techs as Q2Think);
    callbacks
}

/// Tech item pickup (`register` pickup).
fn ctf_tech_pickup(entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    let techs = Q2CtfTechs { hooks: super::ctf_hooks(game) };
    techs.pickup(entity, game, player)
}

/// Tech lifetime think (`think`).
fn ctf_tech_think(entity: ActorId, game: &mut Q2GameServices) {
    let techs = Q2CtfTechs { hooks: super::ctf_hooks(game) };
    let Some(spot) = techs.find_spawn(game) else {
        game.schedule(entity, TECH_TIMEOUT, ctf_tech_think as Q2Think);
        return;
    };
    let classname = game.require_entity(&entity).classname.clone();
    techs.create(&classname, spot, game);
    game.remove_actor(entity);
}

/// Tech spawner think (`spawnTechs`).
fn ctf_spawn_techs(entity: ActorId, game: &mut Q2GameServices) {
    let techs = Q2CtfTechs { hooks: super::ctf_hooks(game) };
    techs.spawn_all(game);
    game.remove_actor(entity);
}

impl Q2CtfTechs {
    /// Register tech items (`register`).
    pub fn register(&self, game: &mut Q2GameServices) {
        for tech in TECHS {
            self.hooks.items.register_item(
                game,
                Q2ItemDefinition {
                    classname: tech.classname.to_string(),
                    model: format!("models/ctf/{}/tris.md2", tech.model),
                    icon: tech.icon.to_string(),
                    name: tech.name.to_string(),
                    sound: "items/pkup.wav".to_string(),
                    rotate: true,
                    respawn: 0.0,
                    console_give: Some(Q2ConsoleGive::IndividualOnly),
                    kind: Q2ItemKindData::Custom {
                        capacity: 1.0,
                        quantity: 0.0,
                        coop_stay: false,
                        droppable: true,
                        pickup: ctf_tech_pickup,
                        use_item: None,
                    },
                },
            );
        }
    }

    /// Spawn a tech (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        let classname = game.require_entity(&entity).classname.clone();
        if !TECHS.iter().any(|tech| tech.classname == classname) {
            return false;
        }
        game.source_callbacks.register(&ctf_tech_callbacks());
        self.hooks.items.spawn(entity, game)
    }

    /// Whether the actor holds a tech (`has`).
    fn has(&self, actor: Option<ActorId>, game: &mut Q2GameServices, ordinal: usize) -> bool {
        let Some(actor) = actor else {
            return false;
        };
        if !game.ctf.states.contains_key(&actor) {
            return false;
        }
        game.host.inventory().count(&actor, &item_id(TECHS[ordinal - 1].item)) > 0.0
    }

    /// Pick up a tech (`pickup`).
    fn pickup(&self, entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
        let phase = game.ctf.match_state.phase;
        if phase == Q2CtfMatchPhase::Setup || phase == Q2CtfMatchPhase::Pregame {
            return false;
        }
        let id = player.id().clone();
        let now = game.now();
        if TECHS.iter().any(|tech| game.host.inventory().count(&id, &item_id(tech.item)) > 0.0) {
            if now - ctf_player(game, &id).last_tech_message > 2.0 {
                ctf_player(game, &id).last_tech_message = now;
                game.host_emit(Q2PresentationEvent::CenterPrint {
                    actor: id,
                    text: "You already have a TECH powerup.".to_string(),
                    instant: false,
                    duration_seconds: None,
                });
            }
            return false;
        }
        let classname = game.require_entity(&entity).classname.clone();
        game.host.inventory().give(&player, &item_id(&format!("q2:{classname}")), 1.0);
        ctf_player(game, &id).regen_time = now;
        true
    }

    /// Find a tech spawn spot (`findSpawn`).
    fn find_spawn(&self, game: &mut Q2GameServices) -> Option<ActorId> {
        let spots = q2_entities_named(game, "info_player_deathmatch");
        let mut index: i32 = -1;
        let mut remaining = 15.min((game.random() * 16.0).floor() as i32);
        // G_Find(NULL) restarts. Passing the final edict returns NULL for one iteration.
        while remaining > 0 {
            remaining -= 1;
            index = if index + 1 == spots.len() as i32 { -1 } else { index + 1 };
        }
        spots.get(if index < 0 { 0 } else { index as usize }).cloned()
    }

    /// Create a tech at a spot (`create`).
    fn create(&self, classname: &str, spot: ActorId, game: &mut Q2GameServices) -> ActorId {
        let entity = game.create(classname, BTreeMap::new());
        if !self.hooks.items.spawn(entity.clone(), game) {
            panic!("Unregistered CTF tech {classname}");
        }
        let touch = crate::q2::foundation::items::Q2ItemModule::touch_item_callback();
        {
            let record = game.require_entity_mut(&entity);
            record.spawnflags = 0x10000;
            record.effects = 1;
            record.render_flags = 512;
            record.owner = Some(entity.clone());
            record.touch = Some(touch);
        }
        let yaw = 359.min((game.random() * 360.0).floor() as i32);
        let velocity = scale3(movedir(vec3(0.0, yaw as f32, 0.0)), 100.0);
        let spot_origin = game.body_of(spot).origin;
        let mut body = game.body_of(entity.clone());
        body.origin = add3(spot_origin, vec3(0.0, 0.0, 16.0));
        body.velocity = Vec3 { x: velocity.x, y: velocity.y, z: 300.0 };
        body.bounds = Bounds { min: vec3(-15.0, -15.0, -15.0), max: vec3(15.0, 15.0, 15.0) };
        game.write_body(entity.clone(), &body, false);
        game.set_solid(entity.clone(), Q2Solid::Trigger);
        game.set_motion_kind(entity.clone(), Q2MotionKind::Toss);
        game.schedule(entity.clone(), TECH_TIMEOUT, ctf_tech_think as Q2Think);
        game.show(entity.clone());
        entity
    }

    /// Spawn all techs (`spawnAll`).
    fn spawn_all(&self, game: &mut Q2GameServices) {
        for tech in TECHS {
            if let Some(spot) = self.find_spawn(game) {
                self.create(tech.classname, spot, game);
            }
        }
    }

    /// Schedule tech setup (`setup`).
    pub fn setup(&self, game: &mut Q2GameServices) {
        if game.options.deathmatch_flags & 524288 != 0 {
            return;
        }
        let spawner = game.create("ctf_tech_spawn", BTreeMap::new());
        game.schedule(spawner, 2.0, ctf_spawn_techs as Q2Think);
    }

    /// Reset techs (`reset`).
    pub fn reset(&self, game: &mut Q2GameServices) {
        let victims: Vec<ActorId> = game
            .entities
            .values()
            .filter(|entity| {
                let id = entity.actor.id().clone();
                self.hooks.items.item_definition(game, &id).is_some_and(|definition| TECHS.iter().any(|tech| tech.classname == definition.classname))
            })
            .map(|entity| entity.actor.id().clone())
            .collect();
        for victim in victims {
            game.remove_actor(victim);
        }
        self.spawn_all(game);
    }

    /// Respawn a tech (`respawn`).
    ///
    /// `CTFRespawnTech` also removes the old actor when no spawn point is
    /// available.
    pub fn respawn(&self, entity: ActorId, game: &mut Q2GameServices) {
        if let Some(spot) = self.find_spawn(game) {
            let classname = game.require_entity(&entity).classname.clone();
            self.create(&classname, spot, game);
        }
        game.remove_actor(entity);
    }

    /// Drop held techs (`drop`).
    pub fn drop(&self, entity: ActorId, game: &mut Q2GameServices, dead: bool) {
        for tech in TECHS {
            let item = item_id(tech.item);
            let count = game.host.inventory().count(&entity, &item);
            if count == 0.0 {
                continue;
            }
            // Original CTFDeadDropTech still calls Drop_Item, not Drop_Player_Item.
            let dropped = self.hooks.items.drop(
                entity.clone(),
                game,
                &item,
                &Q2DropOptions { player_death: false, ..Q2DropOptions::default() },
            );
            let Some(dropped) = dropped else {
                continue;
            };
            if dead {
                game.require_entity_mut(&dropped).owner = None;
                let mut body = game.body_of(dropped.clone());
                body.velocity = vec3(
                    (599.min((game.random() * 600.0).floor() as i32) - 300) as f32,
                    (599.min((game.random() * 600.0).floor() as i32) - 300) as f32,
                    body.velocity.z,
                );
                game.write_body(dropped.clone(), &body, true);
            }
            // Replacing drop_make_touchable preserves the original manual owner's exclusion.
            game.schedule(dropped, TECH_TIMEOUT, ctf_tech_think as Q2Think);
            let owned = game.owned_of(entity.clone());
            game.host.inventory().consume(&owned, &item, count);
        }
    }

    /// Play a tech sound (`sound`).
    fn sound(&self, entity: ActorId, game: &mut Q2GameServices, name: &str) {
        let volume = if silencer_shots(game, &entity) > 0 { 0.2 } else { 1.0 };
        let origin = game.body_of(entity.clone()).origin;
        game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(entity),
            origin,
            path: format!("ctf/{name}.wav"),
            channel: 2,
            volume,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Once,
            loop_owner: None,
        }));
    }

    /// Apply tech strength (`strength`).
    pub fn strength(&self, actor: Option<ActorId>, game: &mut Q2GameServices, damage: f64) -> f64 {
        if damage != 0.0 && self.has(actor, game, 2) { damage * 2.0 } else { damage }
    }

    /// Apply tech resistance (`resistance`).
    pub fn resistance(&self, actor: ActorId, game: &mut Q2GameServices, take: f64) -> f64 {
        if take == 0.0 || !self.has(Some(actor.clone()), game, 1) {
            return take;
        }
        if game.entity(&actor).is_some() {
            self.sound(actor, game, "tech1");
        }
        (take / 2.0).trunc()
    }

    /// Whether the actor has haste (`haste`).
    pub fn haste(&self, actor: ActorId, game: &mut Q2GameServices) -> bool {
        self.has(Some(actor), game, 3)
    }

    /// Play the strength sound (`strengthSound`).
    pub fn strength_sound(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        if !self.has(Some(entity.clone()), game, 2) {
            return false;
        }
        let now = game.now();
        if ctf_player(game, &entity).tech_sound_time < now {
            ctf_player(game, &entity).tech_sound_time = now + 1.0;
            let quad = self.hooks.items.player_powerups(game, &entity).quad_until > now;
            self.sound(entity, game, if quad { "tech2x" } else { "tech2" });
        }
        true
    }

    /// Play the haste sound (`hasteSound`).
    pub fn haste_sound(&self, entity: ActorId, game: &mut Q2GameServices) {
        if !self.haste(entity.clone(), game) {
            return;
        }
        let now = game.now();
        if ctf_player(game, &entity).tech_sound_time < now {
            ctf_player(game, &entity).tech_sound_time = now + 1.0;
            self.sound(entity, game, "tech3");
        }
    }

    /// Whether the actor has regeneration (`hasRegeneration`).
    pub fn has_regeneration(&self, actor: ActorId, game: &mut Q2GameServices) -> bool {
        self.has(Some(actor), game, 4)
    }

    /// Regenerate the entity (`regenerate`).
    pub fn regenerate(&self, entity: ActorId, game: &mut Q2GameServices) {
        if !self.has_regeneration(entity.clone(), game) {
            return;
        }
        let now = game.now();
        let Some(current) = game.host.combat().read(&entity) else {
            return;
        };
        if ctf_player(game, &entity).regen_time >= now {
            return;
        }
        ctf_player(game, &entity).regen_time = now;
        let mut noise = false;
        if current.health < 150.0 {
            let owned = game.owned_of(entity.clone());
            game.host.combat().set_health(&owned, 150.0f64.min(current.health + 5.0));
            ctf_player(game, &entity).regen_time += 0.5;
            noise = true;
        }
        let points = match &current.armor.regular {
            RegularArmorState::None => None,
            RegularArmorState::Q1 { points, .. }
            | RegularArmorState::Q2 { points, .. }
            | RegularArmorState::Q3 { points, .. }
            | RegularArmorState::Source { points, .. } => Some(*points),
        };
        if let Some(points) = points {
            if points > 0.0 && points < 150.0 {
                let owned = game.owned_of(entity.clone());
                game.host.combat().set_regular_points(&owned, 150.0f64.min(points + 5.0), None);
                ctf_player(game, &entity).regen_time += 0.5;
                noise = true;
            }
        }
        if noise && ctf_player(game, &entity).tech_sound_time < now {
            ctf_player(game, &entity).tech_sound_time = now + 1.0;
            self.sound(entity, game, "tech4");
        }
    }
}
