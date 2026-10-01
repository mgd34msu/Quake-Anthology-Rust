//! Q2 rerelease entities (`src/content/q2/rerelease/entities.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::{add3, angle_mod, dot3, length3, normalize3, scale3, sub3, vec3, Bounds, Vec3, Vec4};

use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::{movedir, number_field, vector_field};
use crate::q2::foundation::host::{
    Q2GameServices, Q2LandmarkCarry, Q2Mode, Q2PresentationEvent, Q2Solid, Q2Think, Q2Touch, Q2Use,
};
use crate::q2::foundation::targets::unrotate_q2_landmark;
use crate::q2::support::contracts::TouchContact;

use super::campaign::{q2_rerelease_unit_report, update_q2_rerelease_level};
use super::fog::{equal_q2_fog, interpolate_q2_fog, q2_rerelease_fog_fields};
use super::players::Q2RereleasePlayers;
use super::rerelease_hooks;
use super::types::{Q2FogState, Q2LocalizedPrintLevel, Q2RereleaseEvent, Q2RereleaseHooks, Q2RereleaseNavigation};
use super::world_text::{q2_world_text, Q2WorldTextRequest};

/// Rerelease point of interest (`Q2RereleaseEntities["poi"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleasePoi {
    /// Actor.
    pub actor: ActorId,
    /// Origin.
    pub origin: Vec3,
    /// Image.
    pub image: String,
    /// Dynamic actor.
    pub dynamic: Option<ActorId>,
}

/// Rerelease health bar (`HealthBar`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseHealthBar {
    /// Controller.
    pub controller: ActorId,
    /// Target.
    pub target: ActorId,
    /// Dead expiry.
    pub dead_until: Option<f64>,
}

/// Rerelease entities (`Q2RereleaseEntities`).
#[derive(Debug, Clone, Copy)]
pub struct Q2RereleaseEntities {
    /// Players module.
    pub players: Q2RereleasePlayers,
    /// Session hooks.
    pub hooks: Q2RereleaseHooks,
}

/// Absolute bounds of an entity.
fn absolute_bounds(entity: &ActorId, game: &mut Q2GameServices) -> Bounds {
    let body = game.body_of(entity.clone());
    Bounds {
        min: add3(body.origin, body.bounds.min),
        max: add3(body.origin, body.bounds.max),
    }
}

/// Whether two bounds intersect.
fn intersects(first: &Bounds, second: &Bounds) -> bool {
    first.min.x <= second.max.x
        && first.max.x >= second.min.x
        && first.min.y <= second.max.y
        && first.max.y >= second.min.y
        && first.min.z <= second.max.z
        && first.max.z >= second.min.z
}

/// Rerelease entity callbacks (`Q2RereleaseEntities[callbacks]`).
pub fn rerelease_entities_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks
        .think
        .insert("rr.info_world_text_think", world_text_think as Q2Think);
    callbacks.think.insert("rr.target_poi_setup", poi_setup as Q2Think);
    callbacks
        .think
        .insert("rr.trigger_coop_relay_think", coop_relay_think as Q2Think);
    callbacks
        .think
        .insert("rr.target_crossunit_target_think", cross_unit_think as Q2Think);
    callbacks
        .think
        .insert("rr.check_target_healthbar", healthbar_check as Q2Think);
    callbacks
        .touch
        .insert("rr.trigger_flashlight_touch", flashlight_touch as Q2Touch);
    callbacks.touch.insert("rr.trigger_fog_touch", fog_touch as Q2Touch);
    callbacks.use_.insert("rr.misc_flare_use", flare_use as Q2Use);
    callbacks.use_.insert("rr.info_world_text_use", world_text_use as Q2Use);
    callbacks
        .use_
        .insert("rr.trigger_coop_relay_use", coop_relay_use as Q2Use);
    callbacks.use_.insert("rr.target_poi_use", poi_use as Q2Use);
    callbacks.use_.insert("rr.use_target_music", music_use as Q2Use);
    callbacks.use_.insert("rr.use_target_sky", sky_use as Q2Use);
    callbacks
        .use_
        .insert("rr.trigger_crossunit_trigger_use", cross_unit_use as Q2Use);
    callbacks.use_.insert("rr.use_target_autosave", autosave_use as Q2Use);
    callbacks
        .use_
        .insert("rr.use_target_achievement", achievement_use as Q2Use);
    callbacks.use_.insert("rr.use_target_story", story_use as Q2Use);
    callbacks.use_.insert("rr.use_target_healthbar", healthbar_use as Q2Use);
    callbacks
        .use_
        .insert("rr.use_target_changelevel", change_level_use as Q2Use);
    callbacks
}

/// Spawn rerelease entities (`Q2RereleaseEntities[spawn]`).
pub fn rerelease_entities_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    Q2RereleaseEntities {
        players: Q2RereleasePlayers {
            players: super::players::base_module(game),
        },
        hooks: rerelease_hooks(game),
    }
    .spawn(entity, game)
}

impl Q2RereleaseEntities {
    /// Run end of unit (`endOfUnit`).
    pub fn end_of_unit(&self, game: &mut Q2GameServices) {
        update_q2_rerelease_level(game);
        let levels = q2_rerelease_unit_report(game);
        (self.hooks.emit)(
            game,
            Q2RereleaseEvent::EndOfUnit {
                levels,
                button_time: game.now() + 5.0,
            },
        );
    }

    /// Transfer a health bar target (`transferHealthbarTarget`).
    pub fn transfer_healthbar_target(&self, old_actor: ActorId, new_actor: ActorId, game: &mut Q2GameServices) {
        for slot in 0..game.rerelease.health_bars.len() {
            let matches = game.rerelease.health_bars[slot]
                .as_ref()
                .is_some_and(|bar| bar.target == old_actor);
            if !matches {
                continue;
            }
            if let Some(bar) = game.rerelease.health_bars[slot].as_mut() {
                bar.target = new_actor.clone();
            }
            let controller = game.rerelease.health_bars[slot]
                .as_ref()
                .expect("health bar checked")
                .controller
                .clone();
            if game.entity(&controller).is_some() {
                game.require_entity_mut(&controller).enemy = Some(new_actor.clone());
            }
        }
    }

    /// Initialize a trigger (`trigger`).
    fn trigger(&self, entity: &ActorId, game: &mut Q2GameServices) {
        let angles = game.body_of(entity.clone()).angles;
        {
            let record = game.require_entity_mut(entity);
            record.movedir = movedir(vec3(angles.x, if angles.y == 0.0 { 360.0 } else { angles.y }, angles.z));
            record.visible = false;
            record.server_flags |= 1;
        }
        let mut body = game.body_of(entity.clone());
        body.angles = vec3(0.0, 0.0, 0.0);
        game.write_body(entity.clone(), &body, false);
        game.set_solid(entity.clone(), Q2Solid::Trigger);
        game.link_actor(entity.clone());
    }

    /// Spawn rerelease entities (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        let classname = game.require_entity(&entity).classname.clone();
        if classname == "worldspawn" {
            let spawn = game.require_entity(&entity).spawn.clone();
            game.rerelease.world_fog = q2_rerelease_fog_fields(&spawn, false);
            game.rerelease.sky = super::checkpoint::Q2RereleaseSkyCheckpoint {
                name: spawn.values.get("sky").cloned().unwrap_or_else(|| "unit1_".to_string()),
                rotation: number_field(&spawn, "skyrotate", 0.0),
                auto_rotate: number_field(&spawn, "skyautorotate", 1.0) != 0.0,
                axis: if spawn.values.contains_key("skyaxis") {
                    vector_field(&spawn, "skyaxis")
                } else {
                    vec3(0.0, 0.0, 1.0)
                },
            };
            return false;
        }
        match classname.as_str() {
            "misc_flare" => {
                let (spawnflags, image, radius) = {
                    let record = game.require_entity(&entity);
                    (
                        record.spawnflags,
                        record.spawn.values.get("image").cloned().unwrap_or_default(),
                        number_field(&record.spawn, "radius", 0.0),
                    )
                };
                {
                    let record = game.require_entity_mut(&entity);
                    record.render_flags = 0x200000
                        | ((spawnflags & 7) << 10)
                        | (if spawnflags & 8 != 0 { 1 } else { 0 })
                        | (if image.is_empty() { 0 } else { 256 });
                    record.scale = radius;
                }
                game.set_solid(entity.clone(), Q2Solid::None);
                let mut body = game.body_of(entity.clone());
                body.bounds = Bounds {
                    min: vec3(-32.0, -32.0, -32.0),
                    max: vec3(32.0, 32.0, 32.0),
                };
                game.write_body(entity.clone(), &body, false);
                if !game.require_entity(&entity).targetname.is_empty() {
                    game.require_entity_mut(&entity).use_ = Some(flare_use);
                }
                game.link_actor(entity);
                true
            }
            "info_world_text" => {
                if !game.require_entity(&entity).spawn.values.contains_key("message") {
                    game.host.diagnostic("info_world_text: no message");
                    game.remove_actor(entity);
                    return true;
                }
                {
                    let record = game.require_entity_mut(&entity);
                    record.think = Some(world_text_think);
                    record.use_ = Some(world_text_use);
                }
                if game.require_entity(&entity).spawnflags & 1 == 0 {
                    game.require_entity_mut(&entity).activator = Some(entity.clone());
                    let frame = game.host.frame_seconds();
                    game.schedule(entity, frame, world_text_think);
                }
                true
            }
            "trigger_flashlight" => {
                self.trigger(&entity, game);
                let height = {
                    let record = game.require_entity(&entity);
                    number_field(&record.spawn, "height", 0.0)
                };
                {
                    let record = game.require_entity_mut(&entity);
                    record.movedir.z = height as f32;
                    record.touch = Some(flashlight_touch);
                }
                true
            }
            "trigger_fog" => {
                self.trigger(&entity, game);
                {
                    let record = game.require_entity_mut(&entity);
                    if record.delay == 0.0 {
                        record.delay = 0.5;
                    }
                }
                let target = game.require_entity(&entity).target.clone();
                let goal = game.pick_target(&target);
                game.require_entity_mut(&entity).goal = goal;
                game.require_entity_mut(&entity).touch = Some(fog_touch);
                if game.require_entity(&entity).spawnflags & 3 == 0 {
                    game.host.diagnostic("trigger_fog does not affect fog or height fog");
                }
                true
            }
            "trigger_coop_relay" => {
                self.trigger(&entity, game);
                {
                    let record = game.require_entity_mut(&entity);
                    if record.message.is_empty() {
                        record.message = "$g_coop_wait_for_players".to_string();
                    }
                    if record.map.is_empty() {
                        record.map = "$g_coop_players_waiting_for_you".to_string();
                    }
                    if record.wait == 0.0 {
                        record.wait = 1.0;
                    }
                }
                if game.require_entity(&entity).spawnflags & 1 != 0 {
                    let wait = game.require_entity(&entity).wait;
                    game.schedule(entity, wait, coop_relay_think);
                } else {
                    game.require_entity_mut(&entity).use_ = Some(coop_relay_use);
                }
                true
            }
            "target_poi" => {
                if game.options.mode == Q2Mode::Deathmatch {
                    game.remove_actor(entity);
                    return true;
                }
                {
                    let record = game.require_entity_mut(&entity);
                    record.visible = false;
                    record.server_flags |= 1;
                    record.use_ = Some(poi_use);
                }
                game.schedule(entity, 0.001, poi_setup);
                true
            }
            "target_music" => {
                game.require_entity_mut(&entity).use_ = Some(music_use);
                true
            }
            "target_changelevel" => {
                if game.require_entity(&entity).map.is_empty() {
                    game.host.diagnostic("target_changelevel has no map");
                    game.remove_actor(entity);
                    return true;
                }
                {
                    let record = game.require_entity_mut(&entity);
                    record.visible = false;
                    record.server_flags |= 1;
                    record.use_ = Some(change_level_use);
                }
                true
            }
            "target_sky" => {
                game.require_entity_mut(&entity).use_ = Some(sky_use);
                true
            }
            "target_crossunit_trigger"
            | "target_crossunit_target"
            | "target_autosave"
            | "target_achievement"
            | "target_story"
            | "target_healthbar" => {
                if game.options.mode == Q2Mode::Deathmatch {
                    game.remove_actor(entity);
                    return true;
                }
                {
                    let record = game.require_entity_mut(&entity);
                    record.visible = false;
                    record.server_flags |= 1;
                }
                if classname == "target_crossunit_trigger" {
                    game.require_entity_mut(&entity).use_ = Some(cross_unit_use);
                } else if classname == "target_crossunit_target" {
                    {
                        let record = game.require_entity_mut(&entity);
                        if record.delay == 0.0 {
                            record.delay = 1.0;
                        }
                    }
                    let delay = game.require_entity(&entity).delay;
                    game.schedule(entity, delay, cross_unit_think);
                } else if classname == "target_autosave" {
                    game.require_entity_mut(&entity).use_ = Some(autosave_use);
                } else if classname == "target_achievement" {
                    game.require_entity_mut(&entity).use_ = Some(achievement_use);
                } else if classname == "target_story" {
                    game.require_entity_mut(&entity).use_ = Some(story_use);
                } else if game.require_entity(&entity).target.is_empty()
                    || game.require_entity(&entity).message.is_empty()
                {
                    game.host.diagnostic("target_healthbar requires target and message");
                    game.remove_actor(entity);
                } else {
                    game.require_entity_mut(&entity).use_ = Some(healthbar_use);
                    game.schedule(entity, 0.025, healthbar_check);
                }
                true
            }
            _ => false,
        }
    }

    /// Toggle a flashlight (`toggleFlashlight`).
    pub fn toggle_flashlight(&self, actor: ActorId, game: &mut Q2GameServices, enabled: bool) {
        let extra = game
            .rerelease
            .states
            .get(&actor)
            .expect("Q2 rerelease player is not admitted")
            .clone();
        if extra.flashlight == enabled {
            return;
        }
        game.rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted")
            .flashlight = enabled;
        if game.entity(&actor).is_some() {
            let record = game.require_entity_mut(&actor);
            if enabled {
                record.flags |= 0x400000;
            } else {
                record.flags &= !0x400000;
            }
        }
        if game.entity(&actor).is_some() {
            game.sound(
                &actor,
                if enabled {
                    "items/flashlight_on.wav"
                } else {
                    "items/flashlight_off.wav"
                },
                0,
                1.0,
                3.0,
            );
        }
        super::players::rerelease_emit_flashlight(actor, game);
    }

    /// Force fog (`forceFog`).
    pub fn force_fog(&self, actor: ActorId, game: &mut Q2GameServices, instant: bool) {
        let extra = game
            .rerelease
            .states
            .get(&actor)
            .expect("Q2 rerelease player is not admitted")
            .clone();
        if equal_q2_fog(&extra.fog, &extra.wanted_fog) {
            return;
        }
        (self.hooks.emit)(
            game,
            Q2RereleaseEvent::Fog {
                actor: actor.clone(),
                value: extra.wanted_fog,
                transition_milliseconds: if instant {
                    0.0
                } else {
                    (extra.fog_transition * 1000.0).trunc().clamp(0.0, 65535.0)
                },
            },
        );
        game.rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted")
            .fog = extra.wanted_fog;
    }
}

/// Bind the entities module for callback dispatch.
fn bound_module(game: &Q2GameServices) -> Q2RereleaseEntities {
    Q2RereleaseEntities {
        players: Q2RereleasePlayers {
            players: super::players::base_module(game),
        },
        hooks: rerelease_hooks(game),
    }
}

/// Flashlight touch (`flashlightTouch`).
fn flashlight_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let module = bound_module(game);
    if !game.host.is_player(&contact.other) {
        return;
    }
    let record = game.require_entity(&entity).clone();
    if record.spawnflags & 1 != 0 && !(module.hooks.clip_trigger)(entity.clone(), contact.other.clone(), game) {
        return;
    }
    if record.style == 1 || record.style == 2 {
        return module.toggle_flashlight(contact.other, game, record.style == 1);
    }
    let Some(body) = game.host.bodies().read(&contact.other) else {
        return;
    };
    if dot3(body.velocity, body.velocity) > 32.0 {
        module.toggle_flashlight(
            contact.other,
            game,
            dot3(normalize3(body.velocity), record.movedir) > 0.0,
        );
    }
}

/// Fog touch (`fogTouch`).
fn fog_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if !game.host.is_player(&contact.other) {
        return;
    }
    let now = game.now();
    if game.require_entity(&entity).timestamp > now {
        return;
    }
    let wait = game.require_entity(&entity).wait;
    game.require_entity_mut(&entity).timestamp = now + wait;
    let record = game.require_entity(&entity).clone();
    let values = record
        .goal
        .as_ref()
        .and_then(|goal| game.entity(goal).cloned())
        .unwrap_or(record.clone());
    let Some(body) = game.host.bodies().read(&contact.other) else {
        return;
    };
    let transition = if record.spawnflags & 4 != 0 {
        0.0
    } else if values.delay == 0.0 {
        0.5
    } else {
        values.delay
    };
    game.rerelease
        .states
        .get_mut(&contact.other)
        .expect("Q2 rerelease player is not admitted")
        .fog_transition = transition;
    let value = if record.spawnflags & 16 != 0 {
        let bounds = absolute_bounds(&entity, game);
        let center = scale3(add3(bounds.min, bounds.max), 0.5);
        let size = scale3(
            add3(sub3(bounds.max, bounds.min), sub3(body.bounds.max, body.bounds.min)),
            0.5,
        );
        let dir = record.movedir;
        let start = vec3(-dir.x * size.x, -dir.y * size.y, -dir.z * size.z);
        let end = scale3(start, -1.0);
        let relative = sub3(body.origin, center);
        let distance = vec3(
            relative.x * dir.x.abs(),
            relative.y * dir.y.abs(),
            relative.z * dir.z.abs(),
        );
        let total = length3(sub3(start, end));
        let fraction = if total == 0.0 {
            0.0
        } else {
            f64::from(length3(sub3(distance, start)) / total).clamp(0.0, 1.0) as f64
        };
        interpolate_q2_fog(
            &q2_rerelease_fog_fields(&values.spawn, true),
            &q2_rerelease_fog_fields(&values.spawn, false),
            fraction,
        )
    } else {
        if record.spawnflags & 8 == 0 && length3(body.velocity) <= 0.0001 {
            return;
        }
        let on = record.spawnflags & 8 != 0 || dot3(normalize3(body.velocity), record.movedir) > 0.0;
        q2_rerelease_fog_fields(&values.spawn, !on)
    };
    let wanted = game
        .rerelease
        .states
        .get(&contact.other)
        .expect("Q2 rerelease player is not admitted")
        .wanted_fog;
    game.rerelease
        .states
        .get_mut(&contact.other)
        .expect("Q2 rerelease player is not admitted")
        .wanted_fog = Q2FogState {
        fog: if record.spawnflags & 1 != 0 {
            value.fog
        } else {
            wanted.fog
        },
        height_fog: if record.spawnflags & 2 != 0 {
            value.height_fog
        } else {
            wanted.height_fog
        },
    };
}

/// Eligible coop players (`eligiblePlayers`).
fn eligible_players(game: &mut Q2GameServices) -> Vec<ActorId> {
    let mut result = Vec::new();
    for actor in game.host.players() {
        let state = game.players.states.get(&actor).cloned();
        let Some(state) = state else {
            continue;
        };
        if game.entity(&actor).is_some()
            && !state.dead
            && !state.spectator
            && !state.noclip
            && game.host.combat().read(&actor).map_or(0.0, |combat| combat.health) > 0.0
        {
            result.push(actor);
        }
    }
    result
}

/// Fire a coop relay (`fireRelay`).
fn fire_relay(entity: ActorId, game: &mut Q2GameServices, actor: Option<ActorId>) {
    let authored = game.require_entity(&entity).authored_target();
    let message = game.require_entity(&entity).message.clone();
    game.require_entity_mut(&entity).message = String::new();
    game.use_targets(&authored, actor.as_ref(), false);
    game.require_entity_mut(&entity).message = message;
}

/// Coop relay use (`coopRelayUse`).
fn coop_relay_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    if game.options.mode != Q2Mode::Coop {
        return fire_relay(entity, game, activator);
    }
    let bounds = absolute_bounds(&entity, game);
    let outside: Vec<ActorId> = eligible_players(game)
        .into_iter()
        .filter(|player| !intersects(&bounds, &absolute_bounds(player, game)))
        .collect();
    if outside.is_empty() {
        return fire_relay(entity, game, activator);
    }
    if game.require_entity(&entity).timestamp < game.now() {
        let map = game.require_entity(&entity).map.clone();
        for player in &outside {
            game.host_emit(Q2PresentationEvent::CenterPrint {
                actor: player.clone(),
                text: map.clone(),
                instant: false,
                duration_seconds: None,
            });
        }
        if let Some(activator) = &activator {
            let message = game.require_entity(&entity).message.clone();
            game.host_emit(Q2PresentationEvent::CenterPrint {
                actor: activator.clone(),
                text: message,
                instant: false,
                duration_seconds: None,
            });
        }
    }
    let now = game.now();
    game.require_entity_mut(&entity).timestamp = now + 5.0;
}

/// Coop relay think (`coopRelayThink`).
fn coop_relay_think(entity: ActorId, game: &mut Q2GameServices) {
    let bounds = absolute_bounds(&entity, game);
    let active = eligible_players(game);
    let inside: Vec<ActorId> = active
        .iter()
        .filter(|player| intersects(&bounds, &absolute_bounds(player, game)))
        .cloned()
        .collect();
    if inside.len() == active.len() {
        let first = game.host.players().first().cloned();
        fire_relay(entity.clone(), game, first);
        return game.remove_actor(entity);
    }
    if !inside.is_empty() && game.require_entity(&entity).timestamp < game.now() {
        let message = game.require_entity(&entity).message.clone();
        let map = game.require_entity(&entity).map.clone();
        for actor in game.host.players() {
            game.host_emit(Q2PresentationEvent::CenterPrint {
                actor: actor.clone(),
                text: if inside.contains(&actor) {
                    message.clone()
                } else {
                    map.clone()
                },
                instant: false,
                duration_seconds: None,
            });
        }
        let now = game.now();
        game.require_entity_mut(&entity).timestamp = now + 5.0;
    }
    let wait = game.require_entity(&entity).wait;
    game.schedule(entity, wait, coop_relay_think);
}

/// Team members of an entity (`team`).
fn team(entity: &ActorId, game: &Q2GameServices) -> Vec<ActorId> {
    let name = game
        .require_entity(entity)
        .spawn
        .values
        .get("team")
        .cloned()
        .unwrap_or_default();
    if name.is_empty() {
        return vec![entity.clone()];
    }
    let mut members: Vec<ActorId> = game
        .entities
        .values()
        .filter(|member| member.spawn.values.get("team").is_some_and(|team| *team == name))
        .map(|member| member.actor.id().clone())
        .collect();
    super::sort_rerelease_actors(&mut members);
    members
}

/// POI setup (`poiSetup`).
fn poi_setup(entity: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&entity).spawnflags & 5 != 0 {
        let bits = game.require_entity(&entity).spawnflags & 5;
        for member in team(&entity, game) {
            game.require_entity_mut(&member).spawnflags |= bits;
        }
    }
}

/// POI use (`poiUse`).
fn poi_use(source: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    game.require_entity_mut(&source).spawnflags &= !8;
    let stage = game.rerelease.poi_stage;
    if game.require_entity(&source).count != 0 && stage > game.require_entity(&source).count {
        return;
    }
    let mut selected: Option<ActorId> = Some(source.clone());
    let members = team(&source, game);
    let master = members.first().cloned().unwrap_or(source.clone());
    if game.require_entity(&source).spawn.values.contains_key("team") {
        selected = None;
        let mut best_distance = f64::INFINITY;
        let mut best_style = i32::MAX;
        let mut fallback: Option<ActorId> = None;
        let origin = activator
            .as_ref()
            .and_then(|activator| game.host.bodies().read(activator))
            .map(|body| body.origin)
            .unwrap_or(vec3(0.0, 0.0, 0.0));
        let nearest = game.require_entity(&master).spawnflags & 1 != 0;
        for member in &members {
            let record = game.require_entity(member).clone();
            if record.spawnflags & 8 != 0 {
                continue;
            }
            if record.spawnflags & 2 != 0 {
                fallback = Some(member.clone());
                continue;
            }
            if record.count != 0 && game.rerelease.poi_stage > record.count || record.style > best_style {
                continue;
            }
            let destination = game.body_of(member.clone()).origin;
            let distance = match (rerelease_hooks(game).navigation)(game, origin, destination, activator.clone()) {
                Q2RereleaseNavigation::Path { distance_squared, .. } => distance_squared,
                Q2RereleaseNavigation::NoNavigation => {
                    let delta = sub3(destination, origin);
                    dot3(delta, delta) as f64
                }
                Q2RereleaseNavigation::Unreachable => f64::INFINITY,
            };
            if nearest && selected.is_some() && distance > best_distance {
                continue;
            }
            if record.style < best_style {
                if nearest && distance == f64::INFINITY {
                    continue;
                }
                best_style = record.style;
                if nearest {
                    best_distance = distance;
                }
                selected = Some(member.clone());
            } else if !nearest || distance < best_distance {
                best_distance = distance;
                selected = Some(member.clone());
            }
        }
        if selected.is_none() {
            if let Some(fallback) = fallback.clone() {
                if game.require_entity(&fallback).spawnflags & 4 != 0 {
                    selected = Some(fallback);
                }
            }
        }
    }
    let Some(selected) = selected else {
        return;
    };
    {
        let record = game.require_entity(&selected).clone();
        if record.classname == "target_poi" && record.spawnflags & 2 != 0 && record.spawnflags & 4 == 0 {
            return;
        }
        if record.count != 0 {
            game.rerelease.poi_stage = record.count;
        }
        let dynamic = if record.spawnflags & 4 != 0 {
            members
                .iter()
                .find(|member| game.require_entity(member).spawnflags & 2 != 0)
                .cloned()
        } else {
            None
        };
        let origin = game.body_of(selected.clone()).origin;
        let image = record
            .spawn
            .values
            .get("image")
            .cloned()
            .unwrap_or_else(|| "friend".to_string());
        game.rerelease.poi = Some(Q2RereleasePoi {
            actor: selected,
            origin,
            image,
            dynamic,
        });
    }
}

/// Flare use (`flareUse`).
fn flare_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    game.require_entity_mut(&entity).server_flags ^= 1;
    game.link_actor(entity);
}

/// Music use (`musicUse`).
fn music_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let track = game
        .require_entity(&entity)
        .spawn
        .values
        .get("sounds")
        .cloned()
        .unwrap_or_else(|| "0".to_string());
    game.host_emit(Q2PresentationEvent::Music { track });
}

/// Sky use (`skyUse`).
fn sky_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let record = game.require_entity(&entity).clone();
    let values = &record.spawn.values;
    let sky = super::checkpoint::Q2RereleaseSkyCheckpoint {
        name: values.get("sky").cloned().unwrap_or(game.rerelease.sky.name.clone()),
        rotation: if values.contains_key("skyrotate") {
            number_field(&record.spawn, "skyrotate", 0.0)
        } else {
            game.rerelease.sky.rotation
        },
        auto_rotate: if values.contains_key("skyautorotate") {
            number_field(&record.spawn, "skyautorotate", 0.0) != 0.0
        } else {
            game.rerelease.sky.auto_rotate
        },
        axis: if values.contains_key("skyaxis") {
            vector_field(&record.spawn, "skyaxis")
        } else {
            game.rerelease.sky.axis
        },
    };
    game.rerelease.sky = sky.clone();
    (rerelease_hooks(game).emit)(
        game,
        Q2RereleaseEvent::Sky {
            name: sky.name,
            rotation: sky.rotation,
            auto_rotate: sky.auto_rotate,
            axis: sky.axis,
        },
    );
}

/// Cross-unit use (`crossUnitUse`).
fn cross_unit_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let spawnflags = game.require_entity(&entity).spawnflags;
    game.rerelease.campaign.cross_unit_flags |= spawnflags;
    game.remove_actor(entity);
}

/// Cross-unit think (`crossUnitThink`).
fn cross_unit_think(entity: ActorId, game: &mut Q2GameServices) {
    let record = game.require_entity(&entity).clone();
    if record.spawnflags == game.rerelease.campaign.cross_unit_flags & 0xff00ff & record.spawnflags {
        let authored = record.authored_target();
        game.use_targets(&authored, Some(&entity), false);
        game.remove_actor(entity);
    }
}

/// Autosave use (`autosaveUse`).
fn autosave_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let _ = entity;
    if game.now() - game.rerelease.last_auto_save > game.rerelease.options.auto_save_minimum_time {
        (rerelease_hooks(game).emit)(game, Q2RereleaseEvent::Autosave);
        game.rerelease.last_auto_save = game.now();
    }
}

/// Achievement use (`achievementUse`).
fn achievement_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let id = game
        .require_entity(&entity)
        .spawn
        .values
        .get("achievement")
        .cloned()
        .unwrap_or_default();
    (rerelease_hooks(game).emit)(game, Q2RereleaseEvent::Achievement { id });
}

/// Story use (`storyUse`).
fn story_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let message = game.require_entity(&entity).message.clone();
    game.rerelease.story = message.clone();
    (rerelease_hooks(game).emit)(game, Q2RereleaseEvent::Story { text: message });
}

/// Change-level use (`changeLevelUse`).
fn change_level_use(entity: ActorId, game: &mut Q2GameServices, other: Option<ActorId>, activator: Option<ActorId>) {
    use crate::q2::base::player::index::Q2Intermission;
    if game.players.intermission != Q2Intermission::Playing {
        return;
    }
    if game.options.mode == Q2Mode::Singleplayer {
        let first = game.players.states.iter().find(|(_, state)| state.slot == 0);
        match first {
            None => return,
            Some((actor, _)) => {
                if game.host.combat().read(actor).map_or(0.0, |combat| combat.health) <= 0.0 {
                    return;
                }
            }
        }
    }
    if game.options.mode == Q2Mode::Deathmatch {
        if !game.rerelease.options.deathmatch_allow_exit && other.as_ref() != Some(&game.host.world_actor()) {
            if let Some(other) = other.clone() {
                if let Some(target) = game.entity(&other).cloned() {
                    let origin = game.body_of(other).origin;
                    game.damage(
                        target.actor.id().clone(),
                        entity.clone(),
                        Some(entity.clone()),
                        10.0 * target.max_health,
                        1000.0,
                        vec3(0.0, 0.0, 0.0),
                        origin,
                        vec3(0.0, 0.0, 0.0),
                        28,
                        0,
                        None,
                    );
                }
            }
            return;
        }
        if game.now() < 10.0 {
            return;
        }
        if let Some(activator) = activator.clone() {
            if let Some(player) = game.players.states.get(&activator).cloned() {
                (rerelease_hooks(game).emit)(
                    game,
                    Q2RereleaseEvent::LocalizedPrint {
                        actor: None,
                        level: Q2LocalizedPrintLevel::High,
                        text: "$g_exited_level".to_string(),
                        args: vec![player.name],
                    },
                );
            }
        }
    }
    let map = game.require_entity(&entity).map.clone();
    if map.contains('*') {
        game.counters.server_flags &= 0x0000ff00;
    }
    let mut landmark: Option<Q2LandmarkCarry> = None;
    if let Some(activator) = activator.clone() {
        if game.options.mode != Q2Mode::Deathmatch && game.players.states.contains_key(&activator) {
            let target_name = game.require_entity(&entity).target.clone();
            let target = game.pick_target(&target_name);
            let body = game.host.bodies().read(&activator);
            let view = game.host.player_view_state(&activator);
            game.require_entity_mut(&entity).goal = target.clone();
            if let (Some(target), Some(body), Some(view)) = (target, body, view) {
                let reference = game.body_of(target.clone());
                let targetname = game.require_entity(&target).targetname.clone();
                landmark = Some(Q2LandmarkCarry {
                    player: activator,
                    name: targetname,
                    relative_origin: unrotate_q2_landmark(sub3(body.origin, reference.origin), reference.angles),
                    relative_velocity: unrotate_q2_landmark(view.old_velocity, reference.angles),
                    relative_view_angles: sub3(view.view_angles, reference.angles),
                });
            }
        }
    }
    let spawnflags = game.require_entity(&entity).spawnflags;
    super::players::begin_rerelease_intermission(game, map, landmark, spawnflags);
}

/// Health bar check (`healthbarCheck`).
fn healthbar_check(entity: ActorId, game: &mut Q2GameServices) {
    let target_name = game.require_entity(&entity).target.clone();
    let target = game.pick_target(&target_name);
    match target {
        None => game.remove_actor(entity),
        Some(target) => {
            if !game.host.is_monster(&target) {
                return game.remove_actor(entity);
            }
            game.rerelease.health_targets.insert(entity, target);
        }
    }
}

/// Health bar use (`healthbarUse`).
fn healthbar_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let target_name = game.require_entity(&entity).target.clone();
    let target = game.pick_target(&target_name);
    match target {
        None => game.remove_actor(entity),
        Some(target) => {
            if game.rerelease.health_targets.get(&entity) != Some(&target) {
                return game.remove_actor(entity);
            }
            let slot = game.rerelease.health_bars.iter().position(|bar| bar.is_none());
            let Some(slot) = slot else {
                game.host.diagnostic("target_healthbar: too many health bars");
                return game.remove_actor(entity);
            };
            game.require_entity_mut(&entity).enemy = Some(target.clone());
            game.rerelease.health_bars[slot] = Some(Q2RereleaseHealthBar {
                controller: entity,
                target,
                dead_until: None,
            });
        }
    }
}

impl Q2RereleaseEntities {
    /// Run the end-player frame (`endPlayerFrame`).
    pub fn end_player_frame(&self, entity: ActorId, game: &mut Q2GameServices) {
        self.force_fog(entity.clone(), game, false);
        for slot in 0..game.rerelease.health_bars.len() {
            let Some(bar) = game.rerelease.health_bars[slot].clone() else {
                continue;
            };
            let now = game.now();
            let controller = game.entity(&bar.controller).cloned();
            let target = game.entity(&bar.target).cloned();
            let expired = bar.dead_until.is_some_and(|until| until < now);
            if controller.is_none() || expired {
                remove_health_bar(slot, &bar, game);
                continue;
            }
            let controller = controller.expect("controller checked");
            let health = target
                .as_ref()
                .map(|target| {
                    game.host
                        .combat()
                        .read(target.actor.id())
                        .map_or(0.0, |combat| combat.health)
                })
                .unwrap_or(0.0);
            let holds = match self.hooks.monster_holds_health_bar {
                Some(holds) => holds(game, bar.target.clone()),
                None => false,
            };
            if health <= 0.0 && bar.dead_until.is_none() && !holds {
                if controller.delay == 0.0 {
                    remove_health_bar(slot, &bar, game);
                    continue;
                }
                if let Some(entry) = game.rerelease.health_bars[slot].as_mut() {
                    entry.dead_until = Some(now + controller.delay);
                }
            }
            let bar = game.rerelease.health_bars[slot]
                .as_ref()
                .expect("health bar checked")
                .clone();
            let empty = bar.dead_until.is_some() || health <= 0.0;
            let origin = game.body_of(entity.clone()).origin;
            let target_origin = target
                .as_ref()
                .map(|target| game.body_of(target.actor.id().clone()).origin);
            let visible = empty
                || controller.spawnflags & 1 == 0
                || target_origin.is_some_and(|target_origin| game.host.in_pvs(origin, target_origin));
            let fraction = match (empty, target.as_ref()) {
                (true, _) | (false, None) => 0.0,
                (false, Some(target)) => (health / target.max_health).clamp(0.0, 1.0),
            };
            (self.hooks.emit)(
                game,
                Q2RereleaseEvent::Healthbar {
                    actor: entity.clone(),
                    slot: slot as i32,
                    target: bar.target,
                    name: controller.message,
                    fraction,
                    visible,
                },
            );
        }
    }
}

/// Remove a health bar slot and hide it for all players.
fn remove_health_bar(slot: usize, bar: &Q2RereleaseHealthBar, game: &mut Q2GameServices) {
    let name = game
        .entity(&bar.controller)
        .map(|controller| controller.message.clone())
        .unwrap_or_default();
    let target = bar.target.clone();
    game.rerelease.health_bars[slot] = None;
    for actor in game.host.players() {
        (rerelease_hooks(game).emit)(
            game,
            Q2RereleaseEvent::Healthbar {
                actor,
                slot: slot as i32,
                target: target.clone(),
                name: name.clone(),
                fraction: 0.0,
                visible: false,
            },
        );
    }
}

/// World text think (`worldTextThink`).
fn world_text_think(entity: ActorId, game: &mut Q2GameServices) {
    let colors = [
        Vec4 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
            w: 1.0,
        },
        Vec4 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        },
        Vec4 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
            w: 1.0,
        },
        Vec4 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
            w: 1.0,
        },
        Vec4 {
            x: 1.0,
            y: 1.0,
            z: 0.0,
            w: 1.0,
        },
        Vec4 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        },
        Vec4 {
            x: 0.0,
            y: 1.0,
            z: 1.0,
            w: 1.0,
        },
        Vec4 {
            x: 116.0 / 255.0,
            y: 61.0 / 255.0,
            z: 50.0 / 255.0,
            w: 1.0,
        },
    ];
    let record = game.require_entity(&entity).clone();
    let index = number_field(&record.spawn, "sounds", 0.0);
    let selected = if index >= 0.0 && index.fract() == 0.0 {
        colors.get(index as usize).copied()
    } else {
        None
    };
    if selected.is_none() {
        game.host.diagnostic("info_world_text: invalid color");
    }
    let body = game.body_of(entity.clone());
    let yaw = angle_mod(f64::from(body.angles.y)) + 180.0;
    let radius = number_field(&record.spawn, "radius", 0.0);
    if radius < 0.0 {
        let frame = game.host.frame_seconds();
        return game.schedule(entity, frame, world_text_think);
    }
    let frame = game.host.frame_seconds();
    let text = q2_world_text(&Q2WorldTextRequest {
        origin: body.origin,
        angles: if body.angles.y == -3.0 {
            None
        } else {
            Some(vec3(0.0, if yaw > 360.0 { yaw - 360.0 } else { yaw } as f32, 0.0))
        },
        text: record.message,
        color: selected.unwrap_or(Vec4 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
            w: 1.0,
        }),
        size: if radius == 0.0 { 0.2 } else { radius },
        depth_test: true,
    });
    (rerelease_hooks(game).emit)(game, Q2RereleaseEvent::WorldText { text, lifetime: frame });
    game.schedule(entity, frame, world_text_think);
}

/// World text use (`worldTextUse`).
fn world_text_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    if game.require_entity(&entity).activator.is_none() {
        game.require_entity_mut(&entity).activator = activator;
        world_text_think(entity.clone(), game);
    } else {
        game.cancel_actor(entity.clone());
        let record = game.require_entity_mut(&entity);
        record.think = Some(world_text_think);
        record.activator = None;
    }
    if game.require_entity(&entity).spawnflags & 2 != 0 {
        game.require_entity_mut(&entity).use_ = None;
    }
    let target_name = game.require_entity(&entity).target.clone();
    if let Some(target) = game.pick_target(&target_name) {
        if let Some(use_) = game.require_entity(&target).use_ {
            use_(target, game, Some(entity.clone()), Some(entity.clone()));
        }
    }
    if game.require_entity(&entity).spawnflags & 4 != 0 {
        game.remove_actor(entity);
    }
}
