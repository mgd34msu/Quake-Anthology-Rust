//! Q2 rerelease module (`src/content/q2/rerelease/index.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, vec3, Vec3, Vec4};

use crate::contract::{InventoryEntry, ItemId};
use crate::q2::base::player::index::{player_hooks, player_items, Q2Intermission};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::checkpoint::{restore_q2_actor, save_q2_actor};
use crate::q2::foundation::fields::movedir;
use crate::q2::foundation::host::{
    Q2GameServices, Q2Mode, Q2PresentationEvent, Q2PrintLevel, Q2SoundEvent, Q2SoundLoop, Q2TraceRequest,
};
use crate::q2::foundation::items::{Q2ConsoleGive, Q2ItemDefinition, Q2ItemKindData, Q2PickupPolicy};
use crate::q2::support::contracts::TraceContact;
use crate::q2::support::misc::DebugShape;

use super::campaign::{q2_rerelease_unit_report, Q2RereleaseCampaignState};
use super::checkpoint::{
    Q2RereleaseHealthTarget, Q2RereleaseLightCheckpoint, Q2RereleaseModuleCheckpoint, Q2RereleasePickupRecord,
    Q2RereleasePoiCheckpoint, Q2RereleaseTriggerSoundTime,
};
use super::debug_shapes::q2_debug_shape;
use super::entities::{rerelease_entities_callbacks, rerelease_entities_spawn, Q2RereleaseEntities};
use super::goals::{rerelease_goal_callbacks, rerelease_goals_spawn, Q2RereleaseGoals};
use super::lights::{q2_rerelease_color, rerelease_light_callbacks, rerelease_light_spawn};
use super::players::{
    rerelease_emit_flashlight, rerelease_player_overrides, Q2RereleasePlayerExtension, Q2RereleasePlayers,
};
use super::q64::{rerelease_q64_callbacks, rerelease_q64_spawn};
use super::triggers::{rerelease_trigger_callbacks, rerelease_trigger_spawn};
use super::types::{q2_uses_instanced_items, Q2RereleaseEvent, Q2RereleaseHooks, Q2RereleaseNavigation};
use super::{rerelease_hooks, sort_rerelease_actors};

/// Rerelease module (`Q2RereleaseModule`).
#[derive(Debug, Clone, Copy)]
pub struct Q2RereleaseModule {
    /// Entities module.
    pub entities: Q2RereleaseEntities,
}

/// Create the rerelease module (`createQ2RereleaseModule`), installing hooks,
/// player overrides, the player extension, the pickup policy and items.
pub fn create_q2_rerelease_module(
    game: &mut Q2GameServices,
    players: Q2RereleasePlayers,
    hooks: Q2RereleaseHooks,
) -> Q2RereleaseModule {
    game.rerelease.hooks = Some(hooks);
    game.players.overrides = rerelease_player_overrides();
    game.rerelease.extension = Some(rerelease_player_extension());
    player_items(game).set_pickup_policy(game, rerelease_pickup_policy());
    player_items(game).register_item(
        game,
        Q2ItemDefinition {
            classname: "item_invisibility".to_string(),
            model: "models/items/cloaker/tris.md2".to_string(),
            icon: "p_cloaker".to_string(),
            name: "Invisibility".to_string(),
            sound: "items/pkup.wav".to_string(),
            rotate: true,
            respawn: 300.0,
            console_give: None,
            kind: Q2ItemKindData::Power { coop_stay: false },
        },
    );
    player_items(game).register_item(
        game,
        Q2ItemDefinition {
            classname: "item_flashlight".to_string(),
            model: "models/items/flashlight/tris.md2".to_string(),
            icon: "p_torch".to_string(),
            name: "Flashlight".to_string(),
            sound: "items/pkup.wav".to_string(),
            rotate: true,
            respawn: 0.0,
            console_give: None,
            kind: Q2ItemKindData::Custom {
                capacity: 1.0,
                quantity: 1.0,
                coop_stay: true,
                droppable: false,
                pickup: flashlight_pickup,
                use_item: Some(flashlight_use),
            },
        },
    );
    player_items(game).register_item(
        game,
        Q2ItemDefinition {
            classname: "item_compass".to_string(),
            model: String::new(),
            icon: "p_compass".to_string(),
            name: "Compass".to_string(),
            sound: String::new(),
            rotate: false,
            respawn: 0.0,
            console_give: Some(Q2ConsoleGive::InventoryOnly),
            kind: Q2ItemKindData::Custom {
                capacity: 1.0,
                quantity: 0.0,
                coop_stay: true,
                droppable: false,
                pickup: compass_pickup,
                use_item: Some(compass_use),
            },
        },
    );
    Q2RereleaseModule {
        entities: Q2RereleaseEntities { players, hooks },
    }
}

/// Flashlight pickup.
fn flashlight_pickup(_entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    let item = "q2:item_flashlight".to_string();
    game.host.inventory().count(player.id(), &item) == 0.0 && game.host.inventory().give(&player, &item, 1.0) > 0.0
}

/// Flashlight use.
fn flashlight_use(player: OwnedActor, game: &mut Q2GameServices) -> bool {
    let enabled = !game
        .rerelease
        .states
        .get(player.id())
        .expect("Q2 rerelease player is not admitted")
        .flashlight;
    bound_module(game)
        .entities
        .toggle_flashlight(player.id().clone(), game, enabled);
    true
}

/// Compass pickup (never taken).
fn compass_pickup(_entity: ActorId, _game: &mut Q2GameServices, _player: OwnedActor) -> bool {
    false
}

/// Compass use.
fn compass_use(player: OwnedActor, game: &mut Q2GameServices) -> bool {
    bound_module(game).use_compass(player.id().clone(), game);
    true
}

/// Bind the module for callback dispatch.
fn bound_module(game: &Q2GameServices) -> Q2RereleaseModule {
    let hooks = rerelease_hooks(game);
    Q2RereleaseModule {
        entities: Q2RereleaseEntities {
            players: Q2RereleasePlayers {
                players: super::players::base_module(game),
            },
            hooks,
        },
    }
}

/// Rerelease pickup policy.
fn rerelease_pickup_policy() -> Q2PickupPolicy {
    Q2PickupPolicy {
        instanced_coop: Some(rerelease_instanced_coop),
        can_pickup: rerelease_can_pickup,
        before_pickup: rerelease_before_pickup,
        before_targets: Some(rerelease_before_targets),
        after_pickup: rerelease_after_pickup,
        keep_after_pickup: rerelease_keep_after_pickup,
    }
}

/// Build the rerelease player extension (`Q2RereleaseModule` as extension).
pub fn rerelease_player_extension() -> Q2RereleasePlayerExtension {
    Q2RereleasePlayerExtension {
        admitted: extension_admitted,
        end_player_frame: extension_end_player_frame,
        spawned: extension_spawned,
        before_level_change: extension_before_level_change,
        end_of_unit: extension_end_of_unit,
        leave_unit: extension_leave_unit,
        begin_player_frame: extension_begin_player_frame,
        help: extension_help,
    }
}

/// Extension admitted handler.
fn extension_admitted(actor: ActorId, game: &mut Q2GameServices) {
    bound_module(game).admitted(actor, game);
}

/// Extension end-player-frame handler.
fn extension_end_player_frame(actor: ActorId, game: &mut Q2GameServices) {
    bound_module(game).end_player_frame(actor, game);
}

/// Extension spawned handler.
fn extension_spawned(actor: ActorId, game: &mut Q2GameServices) {
    bound_module(game).spawned(actor, game);
}

/// Extension before-level-change handler.
fn extension_before_level_change(game: &mut Q2GameServices) {
    bound_module(game).before_level_change(game);
}

/// Extension end-of-unit handler.
fn extension_end_of_unit(game: &mut Q2GameServices) {
    bound_module(game).entities.end_of_unit(game);
}

/// Extension leave-unit handler.
fn extension_leave_unit(game: &mut Q2GameServices) {
    bound_module(game).leave_unit(game);
}

/// Extension begin-player-frame handler.
fn extension_begin_player_frame(actor: ActorId, game: &mut Q2GameServices) {
    bound_module(game).begin_player_frame(actor, game);
}

/// Extension help handler.
fn extension_help(actor: ActorId, game: &mut Q2GameServices) {
    bound_module(game).help(actor, game);
}

/// Rerelease module callbacks (`Q2RereleaseModule[callbacks]`).
pub fn rerelease_module_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = rerelease_entities_callbacks();
    for extra in [
        rerelease_trigger_callbacks(),
        rerelease_q64_callbacks(),
        rerelease_light_callbacks(),
        rerelease_goal_callbacks(),
    ] {
        callbacks.think.extend(extra.think);
        callbacks.use_.extend(extra.use_);
        callbacks.touch.extend(extra.touch);
        callbacks.pain.extend(extra.pain);
        callbacks.die.extend(extra.die);
        callbacks.blocked.extend(extra.blocked);
        callbacks.trajectory.extend(extra.trajectory);
    }
    callbacks
}

/// Spawn the rerelease module (`Q2RereleaseModule[spawn]`).
pub fn rerelease_module_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    let record = game.require_entity(&entity).clone();
    let color = record
        .spawn
        .values
        .get("rgba")
        .or_else(|| record.spawn.values.get("_color"))
        .cloned();
    if let Some(color) = color {
        game.require_entity_mut(&entity).skin = q2_rerelease_color(&color);
    }
    rerelease_q64_spawn(entity.clone(), game)
        || rerelease_light_spawn(entity.clone(), game)
        || rerelease_goals_spawn(entity.clone(), game)
        || rerelease_trigger_spawn(entity.clone(), game)
        || rerelease_entities_spawn(entity, game)
}

/// Release rerelease state for an actor (`onRelease` cleanup).
pub fn rerelease_actor_released(game: &mut Q2GameServices, actor: &ActorId) {
    game.rerelease.picked_up_by.remove(actor);
    game.rerelease.pickup_messages.remove(actor);
    game.rerelease.health_targets.remove(actor);
    game.rerelease.trigger_sound_times.remove(actor);
    game.rerelease.q64_eyes.remove(actor);
    game.rerelease.q64_cameras.remove(actor);
    game.rerelease.q64_dummies.remove(actor);
    game.rerelease.light_active.remove(actor);
    game.rerelease.states.remove(actor);
    game.rerelease.squad_spawns.remove(actor);
    game.rerelease.selected_spawns.remove(actor);
    if game
        .rerelease
        .poi
        .as_ref()
        .is_some_and(|poi| poi.dynamic.as_ref() == Some(actor))
    {
        if let Some(poi) = game.rerelease.poi.as_mut() {
            poi.dynamic = None;
        }
    }
}

impl Q2RereleaseModule {
    /// Draw a debug shape (`drawDebugShape`).
    pub fn draw_debug_shape(
        &self,
        game: &mut Q2GameServices,
        shape: &DebugShape,
        color: Vec4,
        lifetime_seconds: f64,
        depth_test: bool,
    ) {
        let event = q2_debug_shape(shape, color, lifetime_seconds, depth_test);
        (self.entities.hooks.emit)(game, event);
    }

    /// Use a powerup (`usePowerup`).
    pub fn use_powerup(&self, player: OwnedActor, item: ItemId, game: &mut Q2GameServices) -> bool {
        if item == "q2:item_invisibility" {
            let now = game.now();
            let extra = game
                .rerelease
                .states
                .get_mut(player.id())
                .expect("Q2 rerelease player is not admitted");
            extra.invisibility_until = extra.invisibility_until.max(now) + 30.0;
            if game.entity(player.id()).is_some() {
                game.sound(player.id(), "items/protect.wav", 3, 1.0, 1.0);
            }
            return true;
        }
        if item == "q2:item_adrenaline" {
            if game.entity(player.id()).is_none() {
                panic!("Rerelease adrenaline requires a source player");
            }
            if game.options.mode != Q2Mode::Deathmatch {
                game.require_entity_mut(player.id()).max_health += 1.0;
            }
            let max_health = game.require_entity(player.id()).max_health;
            if game.host.combat().read(player.id()).map_or(0.0, |combat| combat.health) < max_health {
                game.host.combat().set_health(&player, max_health);
            }
            game.sound(player.id(), "items/n_health.wav", 3, 1.0, 1.0);
            return true;
        }
        false
    }

    /// Capture the module (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> Q2RereleaseModuleCheckpoint {
        let mut picked: Vec<ActorId> = game.rerelease.picked_up_by.keys().cloned().collect();
        sort_rerelease_actors(&mut picked);
        let picked_up_by = picked
            .into_iter()
            .map(|actor| {
                let mut slots: Vec<i32> = game
                    .rerelease
                    .picked_up_by
                    .get(&actor)
                    .expect("pickup checked")
                    .iter()
                    .copied()
                    .collect();
                slots.sort();
                Q2RereleasePickupRecord {
                    actor: SavedActorId::from(&actor),
                    slots,
                }
            })
            .collect();
        let mut sounds: Vec<ActorId> = game.rerelease.trigger_sound_times.keys().cloned().collect();
        sort_rerelease_actors(&mut sounds);
        let trigger_sound_times = sounds
            .into_iter()
            .map(|actor| Q2RereleaseTriggerSoundTime {
                actor: SavedActorId::from(&actor),
                time: game.rerelease.trigger_sound_times.get(&actor).copied().unwrap_or(0.0),
            })
            .collect();
        let mut targets: Vec<ActorId> = game.rerelease.health_targets.keys().cloned().collect();
        sort_rerelease_actors(&mut targets);
        let health_targets = targets
            .into_iter()
            .map(|controller| Q2RereleaseHealthTarget {
                controller: SavedActorId::from(&controller),
                target: SavedActorId::from(
                    game.rerelease
                        .health_targets
                        .get(&controller)
                        .expect("health target checked"),
                ),
            })
            .collect();
        let mut lights: Vec<ActorId> = game.rerelease.light_active.keys().cloned().collect();
        sort_rerelease_actors(&mut lights);
        let lights = lights
            .into_iter()
            .map(|actor| Q2RereleaseLightCheckpoint {
                actor: SavedActorId::from(&actor),
                active: game.rerelease.light_active.get(&actor).copied().unwrap_or(false),
            })
            .collect();
        Q2RereleaseModuleCheckpoint {
            version: 1,
            world_fog: game.rerelease.world_fog.clone(),
            story: game.rerelease.story.clone(),
            sky: game.rerelease.sky.clone(),
            poi: game.rerelease.poi.clone().map(|poi| Q2RereleasePoiCheckpoint {
                actor: SavedActorId::from(&poi.actor),
                origin: poi.origin,
                image: poi.image,
                dynamic: save_q2_actor(poi.dynamic.as_ref()),
            }),
            poi_stage: game.rerelease.poi_stage,
            last_auto_save: game.rerelease.last_auto_save,
            campaign: super::checkpoint::Q2RereleaseCampaignCheckpoint {
                cross_unit_flags: game.rerelease.campaign.cross_unit_flags,
                visited_maps: {
                    let mut maps: Vec<String> = game.rerelease.campaign.visited_maps.iter().cloned().collect();
                    maps.sort();
                    maps
                },
                levels: {
                    let mut levels: Vec<_> = game.rerelease.campaign.levels.values().cloned().collect();
                    levels.sort_by(|first, second| first.map.cmp(&second.map));
                    levels
                },
                mission: game.rerelease.campaign.mission.clone(),
            },
            health_bars: game
                .rerelease
                .health_bars
                .iter()
                .map(|bar| {
                    bar.as_ref()
                        .map(|bar| super::checkpoint::Q2RereleaseHealthBarCheckpoint {
                            controller: SavedActorId::from(&bar.controller),
                            target: SavedActorId::from(&bar.target),
                            dead_until: bar.dead_until,
                        })
                })
                .collect(),
            health_targets,
            picked_up_by,
            trigger_sound_times,
            q64: super::q64::q64_capture(game),
            lights,
            goals: Q2RereleaseGoals {
                hooks: game.rerelease.hooks.expect("Q2 rerelease is not registered"),
            }
            .capture(game),
        }
    }
}

impl Q2RereleaseModule {
    /// Restore the module (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, checkpoint: &Q2RereleaseModuleCheckpoint) {
        self.restore_campaign(game, &checkpoint.campaign);
        game.rerelease.world_fog = checkpoint.world_fog.clone();
        game.rerelease.story = checkpoint.story.clone();
        game.rerelease.sky = checkpoint.sky.clone();
        let poi = checkpoint.poi.clone().map(|poi| {
            let actor = game.host.actors().reference_saved(poi.actor);
            let dynamic = poi.dynamic.map(|dynamic| game.host.actors().reference_saved(dynamic));
            super::entities::Q2RereleasePoi {
                actor,
                origin: poi.origin,
                image: poi.image,
                dynamic,
            }
        });
        game.rerelease.poi = poi;
        game.rerelease.poi_stage = checkpoint.poi_stage;
        game.rerelease.last_auto_save = checkpoint.last_auto_save;
        let mut health_bars: [Option<super::entities::Q2RereleaseHealthBar>; 2] = [None, None];
        for (slot, bar) in checkpoint.health_bars.iter().enumerate().take(2) {
            health_bars[slot] = bar.map(|bar| {
                let controller = game.host.actors().reference_saved(bar.controller);
                let target = game.host.actors().reference_saved(bar.target);
                super::entities::Q2RereleaseHealthBar {
                    controller,
                    target,
                    dead_until: bar.dead_until,
                }
            });
        }
        game.rerelease.health_bars = health_bars;
        game.rerelease.health_targets.clear();
        for entry in &checkpoint.health_targets {
            let controller = restore_q2_actor(game, entry.controller).id().clone();
            let target = game.host.actors().reference_saved(entry.target);
            game.rerelease.health_targets.insert(controller, target);
        }
        game.rerelease.picked_up_by.clear();
        for entry in &checkpoint.picked_up_by {
            let actor = restore_q2_actor(game, entry.actor).id().clone();
            game.rerelease
                .picked_up_by
                .insert(actor, entry.slots.iter().copied().collect());
        }
        game.rerelease.trigger_sound_times.clear();
        for entry in &checkpoint.trigger_sound_times {
            let actor = restore_q2_actor(game, entry.actor).id().clone();
            game.rerelease.trigger_sound_times.insert(actor, entry.time);
        }
        game.rerelease.light_active.clear();
        for entry in &checkpoint.lights {
            let actor = restore_q2_actor(game, entry.actor).id().clone();
            game.rerelease.light_active.insert(actor, entry.active);
        }
        super::q64::q64_restore(game, &checkpoint.q64);
        Q2RereleaseGoals {
            hooks: rerelease_hooks(game),
        }
        .restore(game, &checkpoint.goals);
    }

    /// Restore the campaign (`restoreCampaign`).
    pub fn restore_campaign(
        &self,
        game: &mut Q2GameServices,
        campaign: &super::checkpoint::Q2RereleaseCampaignCheckpoint,
    ) {
        game.rerelease.campaign = Q2RereleaseCampaignState {
            cross_unit_flags: campaign.cross_unit_flags,
            visited_maps: campaign.visited_maps.iter().cloned().collect(),
            levels: campaign
                .levels
                .iter()
                .map(|level| (level.map.clone(), level.clone()))
                .collect(),
            mission: campaign.mission.clone(),
        };
    }

    /// Admit a player (`admitted`).
    pub fn admitted(&self, actor: ActorId, game: &mut Q2GameServices) {
        super::campaign::enter_q2_rerelease_level(game);
        let state = game
            .players
            .states
            .get(&actor)
            .expect("Q2 player has not been admitted")
            .clone();
        let awaiting = game
            .rerelease
            .states
            .get(&actor)
            .expect("Q2 rerelease player is not admitted")
            .awaiting_respawn;
        self.publish_item_visibility(game, actor.clone(), state.slot);
        let world_fog = game.rerelease.world_fog.clone();
        {
            let extra = game
                .rerelease
                .states
                .get_mut(&actor)
                .expect("Q2 rerelease player is not admitted");
            extra.spawned = !awaiting;
            extra.wanted_fog = world_fog;
        }
        self.entities.force_fog(actor.clone(), game, true);
        if state.use_q2_inventory && game.options.mode != Q2Mode::Deathmatch {
            let owned = game.owned_of(actor.clone());
            let item = "q2:item_compass".to_string();
            if game.host.inventory().count(&actor, &item) == 0.0 {
                game.host.inventory().configure(
                    &owned,
                    &InventoryEntry {
                        item,
                        count: 1.0,
                        capacity: 1.0,
                        count_policy: None,
                    },
                );
            }
        }
        if game.options.mode == Q2Mode::Coop {
            if let Some(other) = game.host.players().into_iter().find(|other| {
                *other != actor
                    && game.entity(other).is_some()
                    && game.players.states.get(other).is_some_and(|player| player.connected)
                    && game.host.combat().read(other).map_or(0.0, |combat| combat.health) > 0.0
            }) {
                let owned = game.owned_of(actor.clone());
                for entry in game.host.inventory().entries(&other) {
                    game.host.inventory().configure(&owned, &entry);
                }
                let coop = game
                    .players
                    .states
                    .get(&other)
                    .expect("Q2 player has not been admitted")
                    .coop_respawn
                    .clone();
                game.players
                    .states
                    .get_mut(&actor)
                    .expect("Q2 player has not been admitted")
                    .coop_respawn = coop;
            }
        }
    }

    /// Run after the player frames (`afterPlayerFrames`).
    pub fn after_player_frames(&self, game: &mut Q2GameServices) {
        let _ = self;
        let frame = game.host.frame_seconds();
        let map = game.options.map_name.clone();
        let host_connected = game
            .players
            .states
            .values()
            .any(|player| player.slot == 0 && player.connected);
        if game.players.intermission == Q2Intermission::Playing && host_connected {
            if let Some(entry) = game.rerelease.campaign.levels.get_mut(&map) {
                entry.time += frame;
            }
        }
    }

    /// Run before a level change (`beforeLevelChange`).
    pub fn before_level_change(&self, game: &mut Q2GameServices) {
        let _ = self;
        super::campaign::update_q2_rerelease_level(game);
    }

    /// Leave a unit (`leaveUnit`).
    pub fn leave_unit(&self, game: &mut Q2GameServices) {
        let _ = self;
        game.rerelease.campaign.levels.clear();
        if game.rerelease.options.coop_lives {
            let lives = game.rerelease.options.coop_num_lives + 1;
            for extra in game.rerelease.states.values_mut() {
                extra.lives = lives;
            }
        }
    }

    /// Spawn a player (`spawned`).
    pub fn spawned(&self, actor: ActorId, game: &mut Q2GameServices) {
        let awaiting = game
            .rerelease
            .states
            .get(&actor)
            .expect("Q2 rerelease player is not admitted")
            .awaiting_respawn;
        let world_fog = game.rerelease.world_fog.clone();
        {
            let extra = game
                .rerelease
                .states
                .get_mut(&actor)
                .expect("Q2 rerelease player is not admitted");
            extra.wanted_fog = world_fog;
            extra.spawned = !awaiting;
        }
        self.entities.force_fog(actor.clone(), game, true);
        rerelease_emit_flashlight(actor, game);
    }

    /// Whether items are instanced (`instanced`).
    pub fn instanced(&self, game: &Q2GameServices) -> bool {
        let _ = self;
        game.options.mode == Q2Mode::Coop && q2_uses_instanced_items(&game.rerelease.options)
    }

    /// Publish item visibility (`publishItemVisibility`).
    pub fn publish_item_visibility(&self, game: &mut Q2GameServices, actor: ActorId, slot: i32) {
        if !self.instanced(game) {
            return;
        }
        let mut items: Vec<ActorId> = game.rerelease.picked_up_by.keys().cloned().collect();
        sort_rerelease_actors(&mut items);
        for item in items {
            let taken = game
                .rerelease
                .picked_up_by
                .get(&item)
                .is_some_and(|slots| slots.contains(&slot));
            if taken {
                (rerelease_hooks(game).emit)(
                    game,
                    Q2RereleaseEvent::ItemVisibility {
                        actor: actor.clone(),
                        item,
                        visible: false,
                    },
                );
            }
        }
    }

    /// Run the begin-player frame (`beginPlayerFrame`).
    pub fn begin_player_frame(&self, entity: ActorId, game: &mut Q2GameServices) {
        self.entities.force_fog(entity.clone(), game, false);
        Q2RereleaseGoals {
            hooks: rerelease_hooks(game),
        }
        .notify(entity, game);
    }

    /// Run the end-player frame (`endPlayerFrame`).
    pub fn end_player_frame(&self, entity: ActorId, game: &mut Q2GameServices) {
        self.entities.end_player_frame(entity.clone(), game);
        if game.players.intermission == Q2Intermission::Playing {
            Q2RereleaseGoals {
                hooks: rerelease_hooks(game),
            }
            .end_player_frame(entity.clone(), game);
        }
        self.compass_update(entity, game, false);
    }

    /// Show help (`help`).
    pub fn help(&self, entity: ActorId, game: &mut Q2GameServices) {
        Q2RereleaseGoals {
            hooks: rerelease_hooks(game),
        }
        .help(entity, game);
    }
}

/// Instanced-coop policy.
fn rerelease_instanced_coop(game: &mut Q2GameServices) -> bool {
    game.options.mode == Q2Mode::Coop && q2_uses_instanced_items(&game.rerelease.options)
}

/// Pickup policy: whether an item may be picked up (`canPickup`).
fn rerelease_can_pickup(entity: ActorId, game: &mut Q2GameServices, player: ActorId) -> bool {
    let Some(state) = game.players.states.get(&player).cloned() else {
        return false;
    };
    !rerelease_instanced_coop(game)
        || !game
            .rerelease
            .picked_up_by
            .get(&entity)
            .is_some_and(|slots| slots.contains(&state.slot))
}

/// Pickup policy: before pickup (`beforePickup`).
fn rerelease_before_pickup(entity: ActorId, game: &mut Q2GameServices, player: ActorId) -> bool {
    if !rerelease_can_pickup(entity.clone(), game, player) {
        return false;
    }
    if rerelease_instanced_coop(game) || game.options.mode == Q2Mode::Deathmatch {
        let message = game.require_entity(&entity).message.clone();
        game.rerelease.pickup_messages.insert(entity.clone(), message);
        game.require_entity_mut(&entity).message = String::new();
    }
    true
}

/// Pickup policy: before targets (`beforeTargets`).
fn rerelease_before_targets(entity: ActorId, game: &mut Q2GameServices, player: ActorId, taken: bool) {
    let Some(state) = game.players.states.get(&player).cloned() else {
        return;
    };
    if !taken {
        return;
    }
    game.players
        .states
        .get_mut(&player)
        .expect("Q2 player has not been admitted")
        .bonus_alpha = 0.25;
    let classname = game.require_entity(&entity).classname.clone();
    let descriptor = player_items(game).lookup(game, &classname);
    if let Some(descriptor) = descriptor {
        if descriptor.usable && game.host.inventory().count(&player, &descriptor.id) != 0.0 {
            game.players
                .states
                .get_mut(&player)
                .expect("Q2 player has not been admitted")
                .selected_item = Some(descriptor.id);
        }
    }
    if rerelease_instanced_coop(game) {
        game.rerelease
            .picked_up_by
            .entry(entity.clone())
            .or_default()
            .insert(state.slot);
        (rerelease_hooks(game).emit)(
            game,
            Q2RereleaseEvent::ItemVisibility {
                actor: player.clone(),
                item: entity.clone(),
                visible: false,
            },
        );
        let message = game
            .rerelease
            .pickup_messages
            .get(&entity)
            .cloned()
            .unwrap_or_else(|| game.require_entity(&entity).message.clone());
        if !message.is_empty() {
            game.host_emit(Q2PresentationEvent::CenterPrint {
                actor: player,
                text: message,
                instant: false,
                duration_seconds: None,
            });
        }
    }
}

/// Pickup policy: after pickup (`afterPickup`).
fn rerelease_after_pickup(entity: ActorId, game: &mut Q2GameServices, _player: ActorId, _taken: bool) {
    if let Some(message) = game.rerelease.pickup_messages.remove(&entity) {
        game.require_entity_mut(&entity).message = message;
    }
}

/// Pickup policy: keep after pickup (`keepAfterPickup`).
fn rerelease_keep_after_pickup(entity: ActorId, game: &mut Q2GameServices, _player: ActorId) -> bool {
    rerelease_instanced_coop(game) && game.require_entity(&entity).spawnflags & 0x20000 == 0
}

impl Q2RereleaseModule {
    /// Resume presentation (`resumePresentation`).
    pub fn resume_presentation(&self, game: &mut Q2GameServices, only: Option<ActorId>) {
        let now = game.now();
        let primary = game.rerelease.campaign.mission.primary.clone();
        let secondary = game.rerelease.campaign.mission.secondary.clone();
        let mut actors: Vec<ActorId> = game.rerelease.states.keys().cloned().collect();
        sort_rerelease_actors(&mut actors);
        for actor in actors {
            if only.as_ref().is_some_and(|only| *only != actor) {
                continue;
            }
            let state = game.players.states.get(&actor).cloned();
            let extra = game
                .rerelease
                .states
                .get(&actor)
                .expect("Q2 rerelease player is not admitted")
                .clone();
            let Some(state) = state else {
                continue;
            };
            if !state.connected {
                continue;
            }
            (rerelease_hooks(game).emit)(
                game,
                Q2RereleaseEvent::HelpComputer {
                    actor: actor.clone(),
                    visible: state.show_help,
                    primary: primary.clone(),
                    secondary: secondary.clone(),
                    slow_time: state.show_help,
                },
            );
            if extra.help_marker_until > now {
                (rerelease_hooks(game).emit)(
                    game,
                    Q2RereleaseEvent::Poi {
                        actor: actor.clone(),
                        position: extra.help_location,
                        image: extra.help_image.clone(),
                        duration: (extra.help_marker_until - now) * 1000.0,
                        color: 208,
                    },
                );
            }
            let point = if extra.help_index > 0 {
                extra.help_points.get((extra.help_index - 1) as usize).copied()
            } else {
                None
            };
            if let Some(point) = point {
                let next = extra
                    .help_points
                    .get(extra.help_index as usize)
                    .copied()
                    .unwrap_or(extra.help_location);
                (rerelease_hooks(game).emit)(
                    game,
                    Q2RereleaseEvent::HelpPath {
                        actor,
                        first: true,
                        position: point,
                        direction: normalize3(sub3(next, point)),
                    },
                );
            }
        }
        if let Q2Intermission::Intermission { map, started, .. } = game.players.intermission.clone() {
            if map.contains('*') && game.rerelease.intermission_flags & 16 == 0 {
                let levels = q2_rerelease_unit_report(game);
                (rerelease_hooks(game).emit)(
                    game,
                    Q2RereleaseEvent::EndOfUnit {
                        levels,
                        button_time: started + 5.0,
                    },
                );
            }
        }
    }

    /// Send a POI marker (`sendPoi`).
    pub fn send_poi(&self, actor: ActorId, game: &mut Q2GameServices) {
        let _ = self;
        let now = game.now();
        let extra = game
            .rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted");
        extra.help_marker_until = now + 10.0;
        let position = extra.help_location;
        let image = extra.help_image.clone();
        (rerelease_hooks(game).emit)(
            game,
            Q2RereleaseEvent::Poi {
                actor,
                position,
                image,
                duration: 10000.0,
                color: 208,
            },
        );
    }

    /// Use the compass (`useCompass`).
    pub fn use_compass(&self, actor: ActorId, game: &mut Q2GameServices) {
        let Some(poi) = game.rerelease.poi.clone() else {
            game.host_emit(Q2PresentationEvent::Print {
                actor: Some(actor),
                level: Q2PrintLevel::High,
                text: "$no_valid_poi".to_string(),
            });
            return;
        };
        if let Some(dynamic) = poi.dynamic.clone() {
            if let Some(use_) = game.entity(&dynamic).and_then(|entity| entity.use_) {
                use_(dynamic, game, Some(actor.clone()), Some(actor.clone()));
            }
        }
        let Some(body) = game.host.bodies().read(&actor) else {
            return;
        };
        {
            let extra = game
                .rerelease
                .states
                .get_mut(&actor)
                .expect("Q2 rerelease player is not admitted");
            extra.help_location = poi.origin;
            extra.help_image = poi.image;
        }
        let navigation = (rerelease_hooks(game).navigation)(game, body.origin, poi.origin, Some(actor.clone()));
        let Q2RereleaseNavigation::Path { points, .. } = navigation else {
            self.send_poi(actor.clone(), game);
            let origin = game.body_of(actor.clone()).origin;
            game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
                actor: Some(actor),
                origin,
                path: "misc/help_marker.wav".to_string(),
                channel: 0,
                volume: 1.0,
                attenuation: 1.0,
                reliable: false,
                loop_: Q2SoundLoop::Once,
                loop_owner: None,
            }));
            return;
        };
        let mut points: Vec<Vec3> = points.into_iter().take(129).collect();
        let index = 0;
        while index + 1 < points.len() && length3(sub3(points[index + 1], body.origin)) < 192.0 {
            points.remove(index + 1);
        }
        let movement = (player_hooks(game).movement)(actor.clone());
        if !points.is_empty()
            && dot3(
                normalize3(sub3(points[index], body.origin)),
                movedir(movement.view_angles),
            ) < 0.3
        {
            let height = f64::from(game.require_entity(&actor).view_height.max(0)) + 22.0;
            let forward = movedir(movement.view_angles);
            let start = add3(body.origin, vec3(0.0, 0.0, height as f32));
            let trace = game.host.trace(&Q2TraceRequest {
                start,
                end: add3(start, scale3(forward, 64.0)),
                bounds: None,
                ignore: None,
                mask: 1,
                exclude: Vec::new(),
            });
            let mut point = trace.end;
            if trace.fraction < 1.0 {
                if let TraceContact::Plane { plane } = trace.contact {
                    point = add3(trace.end, scale3(plane.normal, 8.0));
                }
            }
            points.insert(index, point);
        }
        {
            let extra = game
                .rerelease
                .states
                .get_mut(&actor)
                .expect("Q2 rerelease player is not admitted");
            extra.help_points = points;
            extra.help_index = index as i32;
            extra.help_draw_time = 0.0;
        }
        self.compass_update(actor, game, true);
    }

    /// Update the compass (`compassUpdate`).
    pub fn compass_update(&self, actor: ActorId, game: &mut Q2GameServices, first: bool) {
        let _ = self;
        let extra = game
            .rerelease
            .states
            .get(&actor)
            .expect("Q2 rerelease player is not admitted")
            .clone();
        let Some(point) = usize::try_from(extra.help_index)
            .ok()
            .and_then(|index| extra.help_points.get(index).copied())
        else {
            return;
        };
        let Some(body) = game.host.bodies().read(&actor) else {
            return;
        };
        if extra.help_draw_time >= game.now() {
            return;
        }
        if length3(sub3(point, body.origin)) > 4096.0 || !game.host.in_phs(body.origin, point) {
            game.rerelease
                .states
                .get_mut(&actor)
                .expect("Q2 rerelease player is not admitted")
                .help_points = Vec::new();
            return;
        }
        let next = usize::try_from(extra.help_index + 1)
            .ok()
            .and_then(|index| extra.help_points.get(index).copied())
            .unwrap_or(extra.help_location);
        (rerelease_hooks(game).emit)(
            game,
            Q2RereleaseEvent::HelpPath {
                actor: actor.clone(),
                first,
                position: point,
                direction: normalize3(sub3(next, point)),
            },
        );
        bound_module(game).send_poi(actor.clone(), game);
        game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(actor.clone()),
            origin: point,
            path: "misc/help_marker.wav".to_string(),
            channel: 0,
            volume: 1.0,
            attenuation: 1.0,
            reliable: false,
            loop_: Q2SoundLoop::Once,
            loop_owner: None,
        }));
        let now = game.now();
        let extra = game
            .rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted");
        extra.help_index += 1;
        extra.help_draw_time = now + 0.2;
    }
}
