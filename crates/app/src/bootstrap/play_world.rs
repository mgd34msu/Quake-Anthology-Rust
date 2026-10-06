//! Play map world: the selected map's real entities in a live server.
//!
//! Donor provenance: `src/app/bootstrap/content.ts`
//! (`loadApplicationContent`: catalog discovery, mount read of
//! `recipe.map.geometry`, `classifyBsp`, then `readQ1Bsp` / `readQ2Bsp` /
//! `decodeQ3World`) plus the entity-string parses in
//! `src/formats/q1-map/entities.ts` (also used for Quake II entity strings
//! in `src/app/bootstrap/simulation/native-q2-map.ts`) and
//! `src/formats/q3-map/entities.ts`. Spawning follows the engine side of
//! the donor gamecode bind: [`SpawnFields`] text, [`SpawnRegistry`]
//! dispatch, [`Server::spawn_entity`](qa_world::server::Server::spawn_entity).
//!
//! The windowed composition discovers the catalog but never resolves a
//! launch, so this module takes the short faithful path: read the selected
//! map's BSP bytes through the installed product's mounts
//! ([`InstalledCatalog::read`]), classify and decode with the format
//! readers, parse the entity string for the map's family, and spawn every
//! record into a server opened by [`open_server`]. Classnames outside the
//! stub table get a generic `map:{classname}` spawn function registered in
//! deterministic order; per-game spawn behaviors still live in the guest
//! game modules, which the windowed run does not bind (the same no-op
//! logic hooks as the stub path). Records that fail field parse or spawn
//! are recorded in [`PlayWorld::skipped`] instead of aborting the load.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use qa_client::audio::SoundFamily;
use qa_content::bsp::{parse_q1_entities, read_q1_bsp, Q1BspOptions};
use qa_content::bsp2::read_q2_bsp;
use qa_content::bsp3::{parse_q3_bsp, parse_q3_entities};
use qa_content::catalog::InstalledCatalog;
use qa_content::mounts::MountedContent;
use qa_content::{classify_bsp, BspKind};
use qa_core::math::vec3;
use qa_guest::server::GuestServerLogic;
use qa_world::server::Server;
use qa_world::spawn::{SpawnFields, SpawnRequest};
use thiserror::Error;

use super::audio_bridge::{map_speakers, MapSpeaker};
use super::play::{
    admit_player, build_clip, eye_height_for_family, link_q1_scene, movement_content_edition,
    movement_dialect_for_selection, provider_for_product, PlayerBody, PlayerClip, Q1SceneLinks,
};
use super::simulation::native_q1_items::{build_q1_item, q1_is_item, register_q1_item_spawns};
use super::simulation::native_q1_monsters::{
    build_q1_monster, build_q1_movetarget, q1_is_monster, q1_is_movetarget, q1_monster_pass, register_q1_monster_spawns,
};
use super::simulation::native_q1_spawns::{
    build_q1_door, install_q1_native, link_q1_doors, q1_note_solid, q1_pre_spawn, register_q1_spawns,
    Q1NativeBehaviors, Q1PendingDoor, Q1PreSpawn,
};
use super::simulation::native_q1_triggers::{
    build_q1_button, build_q1_trigger, q1_is_brush_trigger, q1_is_use_point, q1_note_intermission, q1_note_light,
    q1_note_start_spot, q1_note_targetname, q1_note_teleport_destination, q1_note_use_point, q1_note_worldspawn,
    q1_registered_version, register_q1_trigger_spawns,
};
use super::simulation::native_q1_weapons::{q1_grant_spawn_loadout, q1_sample_water_level, q1_weapon_pass};
use super::windowed_scene::{build_presentation, open_product_mounts, select_spawn, PlayPresentation};
use crate::options::{ApplicationOptions, GameMode};
use crate::startup::{open_server, StartupConfig};

/// One map entity record that did not spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedEntity {
    /// Zero-based record index in the parsed entity string.
    pub index: usize,
    /// Record classname (`""` when the record has none).
    pub classname: String,
    /// Why field parse or spawn failed.
    pub reason: String,
}

/// Outcome of spawning one map's parsed entity records.
#[derive(Debug, Default)]
pub struct MapSpawnSummary {
    /// Records that became live actors.
    pub spawned: usize,
    /// Records that failed field parse or spawn, in record order.
    pub skipped: Vec<SkippedEntity>,
}

/// Windowed map-world load failure.
#[derive(Debug, Error)]
pub enum PlayWorldError {
    /// The selected content product is not in the catalog.
    #[error("unknown content product {0}")]
    UnknownProduct(String),
    /// The map bytes could not be read through the product mounts.
    #[error("cannot read map {map} from {content}: {reason}")]
    MapUnread {
        /// Content product id.
        content: String,
        /// Map resource path.
        map: String,
        /// Catalog read failure.
        reason: String,
    },
    /// The map bytes did not classify or decode.
    #[error("cannot decode {map}: {reason}")]
    MapDecode {
        /// Map resource path.
        map: String,
        /// Classifier/decoder failure.
        reason: String,
    },
    /// The decoded entity string did not parse.
    #[error("cannot parse entities from {map}: {reason}")]
    EntityParse {
        /// Map resource path.
        map: String,
        /// Entity parser failure.
        reason: String,
    },
    /// The map server could not open.
    #[error("cannot open map server: {0}")]
    Server(String),
    /// The decoded world did not become a scene presentation.
    #[error("cannot present {map}: {reason}")]
    Presentation {
        /// Map resource path.
        map: String,
        /// Presentation failure.
        reason: String,
    },
    /// No entity record spawned.
    #[error("spawned 0 of {records} map entities from {map} ({reason})")]
    NothingSpawned {
        /// Map resource path.
        map: String,
        /// Parsed entity records.
        records: usize,
        /// First skip reason, or `map has no entity records`.
        reason: String,
    },
    /// The map loaded but its interactive player could not start.
    #[error("cannot start play on {map}: {reason}")]
    Play {
        /// Map resource path.
        map: String,
        /// Clip-build or admission failure.
        reason: String,
    },
}

/// A live server holding one map's real entities.
pub struct PlayWorld {
    server: Server<GuestServerLogic>,
    content: String,
    map: String,
    entity_records: usize,
    spawned: usize,
    skipped: Vec<SkippedEntity>,
    presentation: Option<PlayPresentation>,
    presentation_error: Option<String>,
    clip: Option<PlayerClip>,
    player: Option<PlayerBody>,
    dialect: qa_core::cmd::Dialect,
    audio_mounts: Option<Rc<MountedContent>>,
    speakers: Vec<MapSpeaker>,
    sound_family: SoundFamily,
    q1_behaviors: Option<Rc<RefCell<Q1NativeBehaviors>>>,
}

impl std::fmt::Debug for PlayWorld {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlayWorld")
            .field("content", &self.content)
            .field("map", &self.map)
            .field("entity_records", &self.entity_records)
            .field("spawned", &self.spawned)
            .field("skipped", &self.skipped)
            .field("entity_count", &self.entity_count())
            .field("presentation", &self.presentation)
            .field("presentation_error", &self.presentation_error)
            .field("audio_mounts", &self.audio_mounts.is_some())
            .field("speakers", &self.speakers.len())
            .field("sound_family", &self.sound_family)
            .field(
                "q1_doors",
                &self
                    .q1_behaviors
                    .as_ref()
                    .map(|behaviors| behaviors.borrow().doors.len()),
            )
            .finish()
    }
}

impl PlayWorld {
    /// Borrow the live server.
    #[must_use]
    pub fn server(&self) -> &Server<GuestServerLogic> {
        &self.server
    }

    /// Borrow the live server mutably (per-frame tick and player steps).
    pub fn server_mut(&mut self) -> &mut Server<GuestServerLogic> {
        &mut self.server
    }

    /// Player eye origin plus view angles for the follow camera, or `None`
    /// when the family has no admitted player (the view falls back to the
    /// static spawn).
    #[must_use]
    pub fn player_eye(&self) -> Option<(qa_core::math::Vec3, qa_core::math::Vec3)> {
        self.player.as_ref().map(PlayerBody::eye)
    }

    /// Whether an interactive player is admitted.
    #[must_use]
    pub fn has_player(&self) -> bool {
        self.player.is_some()
    }

    /// The admitted body's simulation actor, or `None` without a player.
    #[must_use]
    pub fn player_actor(&self) -> Option<&qa_core::identity::ActorId> {
        self.player.as_ref().map(PlayerBody::actor)
    }

    /// Live native Q1 gamecode state (doors, fields, opener), or `None`
    /// for non-Q1 maps.
    #[must_use]
    pub fn q1_behaviors(&self) -> Option<Rc<RefCell<Q1NativeBehaviors>>> {
        self.q1_behaviors.clone()
    }

    /// Retained product mounts for game audio (see
    /// [`MountsSoundContent`](super::audio_bridge::MountsSoundContent)), or
    /// `None` when the audio open failed (the run stays silent).
    #[must_use]
    pub fn audio_mounts(&self) -> Option<Rc<MountedContent>> {
        self.audio_mounts.clone()
    }

    /// Map `target_speaker` loops stashed at load for the audio bridge to
    /// start once after the world installs.
    #[must_use]
    pub fn map_speakers(&self) -> &[MapSpeaker] {
        &self.speakers
    }

    /// Sound family of the loaded map, from its decoded BSP kind.
    #[must_use]
    pub fn sound_family(&self) -> SoundFamily {
        self.sound_family
    }

    /// Run one player movement step for a world user command. No admitted
    /// player (or no clip) keeps the static-spawn behavior: the world still
    /// ticks, the camera just does not follow.
    pub fn step_player(&mut self, command: qa_world::movement::types::UserCommand) -> Result<(), PlayWorldError> {
        // Stock intermission freeze (`PlayerPreThink`, `client.qc:901`):
        // the entry move unsolids the player and snaps the view, and no
        // step runs until the exit travels (stock `MOVETYPE_NONE`).
        if self
            .q1_behaviors
            .as_ref()
            .is_some_and(|behaviors| behaviors.borrow().intermission.running != 0)
        {
            return Ok(());
        }
        let (Some(player), Some(clip)) = (self.player.as_mut(), self.clip.as_mut()) else {
            return Ok(());
        };
        // Mirror the view angles into gamecode before stepping so the
        // angle-gated trigger facing check reads live facing.
        if let Some(behaviors) = self.q1_behaviors.as_ref() {
            behaviors.borrow_mut().player_angles = player.eye().1;
        }
        let behaviors = self.q1_behaviors.clone();
        let borrowed = behaviors.as_ref().map(|behaviors| behaviors.borrow());
        let links = borrowed.as_ref().map(|behaviors| Q1SceneLinks {
            door_models: &behaviors.brush_models,
            solids: &behaviors.solids,
        });
        let (simulation, triggers) = self.server.simulation_and_triggers();
        player
            .step(simulation, triggers, clip, links.as_ref(), command)
            .map_err(|reason| PlayWorldError::Play {
                map: self.map.clone(),
                reason,
            })
    }

    /// Run one weapon pass for native Q1 maps: relink the collision
    /// scene, sample the player's water level, then run the stock
    /// weapon frame (impulse selection plus the held trigger) and any
    /// due attack-anim think. `None` advances anims without new input
    /// (the trigger reads released). Non-Q1 maps (or missing scenes
    /// or players) keep the static behavior. The pass is infallible
    /// by design: failed traces block, and dead players skip (death
    /// thinks own them once the player slice lands).
    pub fn step_weapons(&mut self, command: Option<&qa_world::movement::types::UserCommand>) {
        use qa_world::movement::types::UserCommand;
        let Some(behaviors) = self.q1_behaviors.clone() else {
            return;
        };
        let player = behaviors.borrow().player.clone();
        let Some(player) = player else {
            return;
        };
        let view_angles = self.player.as_ref().map(|body| body.eye().1);
        let Some(view_angles) = view_angles else {
            return;
        };
        let Self { server, clip, .. } = self;
        let Some(PlayerClip::Q1(scene)) = clip.as_mut() else {
            return;
        };
        {
            let borrowed = behaviors.borrow();
            let links = Q1SceneLinks {
                door_models: &borrowed.brush_models,
                solids: &borrowed.solids,
            };
            let (simulation, triggers) = server.simulation_and_triggers();
            link_q1_scene(scene, simulation, triggers, Some(&links));
        }
        let (buttons, impulse) = match command {
            Some(UserCommand::Q1Netquake(command)) => (command.buttons, command.impulse),
            Some(UserCommand::Q1Quakeworld(command)) => (command.buttons, command.impulse),
            _ => (0, 0),
        };
        let water = q1_sample_water_level(scene, server.simulation(), &player);
        behaviors.borrow_mut().player_state.water_level = water;
        q1_weapon_pass(
            server,
            &mut behaviors.borrow_mut(),
            scene,
            &player,
            view_angles,
            buttons,
            impulse,
        );
    }

    /// Run one monster think pass for native Q1 maps: relink the
    /// collision scene, then think, move, and toss every monster and
    /// gib. Non-Q1 maps (or missing scenes) keep the static behavior.
    /// The pass is infallible by design: failed traces block, failed
    /// spawns drop, and moves touch on the next tick's sweep.
    pub fn step_monsters(&mut self) {
        let Some(behaviors) = self.q1_behaviors.clone() else {
            return;
        };
        let Self { server, clip, .. } = self;
        let Some(PlayerClip::Q1(scene)) = clip.as_mut() else {
            return;
        };
        {
            let borrowed = behaviors.borrow();
            let links = Q1SceneLinks {
                door_models: &borrowed.brush_models,
                solids: &borrowed.solids,
            };
            let (simulation, triggers) = server.simulation_and_triggers();
            link_q1_scene(scene, simulation, triggers, Some(&links));
        }
        q1_monster_pass(server, &mut behaviors.borrow_mut(), scene);
    }

    /// Content product the map bytes came from.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Map resource path that was loaded.
    #[must_use]
    pub fn map(&self) -> &str {
        &self.map
    }

    /// Input dialect for the map's catalog family and edition.
    #[must_use]
    pub fn dialect(&self) -> qa_core::cmd::Dialect {
        self.dialect
    }

    /// Parsed entity records in the map's entity string.
    #[must_use]
    pub fn entity_records(&self) -> usize {
        self.entity_records
    }

    /// Records that became live actors.
    #[must_use]
    pub fn spawned(&self) -> usize {
        self.spawned
    }

    /// Records that failed field parse or spawn, in record order.
    #[must_use]
    pub fn skipped(&self) -> &[SkippedEntity] {
        &self.skipped
    }

    /// Live actors in the map server.
    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.server.simulation().actor_count()
    }

    /// Scene presentation (world geometry plus model-bearing entities), or
    /// `None` when presentation failed (see [`Self::presentation_error`]).
    #[must_use]
    pub fn presentation(&self) -> Option<&PlayPresentation> {
        self.presentation.as_ref()
    }

    /// Why presentation failed, when [`Self::presentation`] is `None`.
    #[must_use]
    pub fn presentation_error(&self) -> Option<&str> {
        self.presentation_error.as_deref()
    }

    /// Move the scene presentation out for the windowed scene view.
    pub fn take_presentation(&mut self) -> Option<PlayPresentation> {
        self.presentation.take()
    }
}

/// Content product holding the selected map (`--map-game` override or `--game`).
fn map_content_id(options: &ApplicationOptions) -> &str {
    options.map_product.as_deref().unwrap_or(&options.product)
}

/// One map's decoded entity records plus the Q1 brush-model bounds the
/// native door spawns size from.
pub(crate) struct DecodedMapEntities {
    /// Entity records as ordered key/value property lists.
    pub records: Vec<Vec<(String, String)>>,
    /// Q1 brush-model bounds by model index (`*N`); empty for other kinds.
    pub q1_models: Vec<qa_core::math::Bounds>,
}

/// Convert content brush-model bounds to engine bounds.
fn core_bounds(bounds: qa_content::common::Bounds) -> qa_core::math::Bounds {
    qa_core::math::Bounds {
        min: vec3(bounds.min[0], bounds.min[1], bounds.min[2]),
        max: vec3(bounds.max[0], bounds.max[1], bounds.max[2]),
    }
}

/// Decode one map's entity records as ordered key/value property lists.
///
/// Quake II entity strings share the Quake brace syntax, so they parse
/// with the Quake reader, matching the donor (`native-q2-map.ts` parses
/// `world.entities` with `parseQ1Entities`).
pub(crate) fn decode_map_entities(
    bytes: &[u8],
    map: &str,
    kind: BspKind,
) -> Result<DecodedMapEntities, PlayWorldError> {
    match kind {
        BspKind::Q1 => {
            let parsed =
                read_q1_bsp(bytes, map, Q1BspOptions::default()).map_err(|error| PlayWorldError::MapDecode {
                    map: map.to_string(),
                    reason: error.to_string(),
                })?;
            Ok(DecodedMapEntities {
                records: parsed.entity_list.into_iter().map(|entity| entity.properties).collect(),
                q1_models: parsed
                    .models
                    .into_iter()
                    .map(|model| core_bounds(model.bounds))
                    .collect(),
            })
        }
        BspKind::Q2 => {
            let parsed = read_q2_bsp(bytes, map).map_err(|error| PlayWorldError::MapDecode {
                map: map.to_string(),
                reason: error.to_string(),
            })?;
            parse_q1_entities(&parsed.entities, &format!("{map}:entities"))
                .map(|entities| DecodedMapEntities {
                    records: entities.into_iter().map(|entity| entity.properties).collect(),
                    q1_models: Vec::new(),
                })
                .map_err(|error| PlayWorldError::EntityParse {
                    map: map.to_string(),
                    reason: error.to_string(),
                })
        }
        BspKind::Q3 => {
            let parsed = parse_q3_bsp(bytes, map).map_err(|error| PlayWorldError::MapDecode {
                map: map.to_string(),
                reason: error.to_string(),
            })?;
            parse_q3_entities(&parsed.entities, &format!("{map}:entities"))
                .map(|entities| DecodedMapEntities {
                    records: entities.into_iter().map(|entity| entity.properties).collect(),
                    q1_models: Vec::new(),
                })
                .map_err(|error| PlayWorldError::EntityParse {
                    map: map.to_string(),
                    reason: error.to_string(),
                })
        }
    }
}

/// Register a generic spawn function for one map classname.
///
/// The engine-side request keeps the record origin so the actor gets a
/// body at the map position; per-game behavior stays with the guest game
/// modules, which the windowed run does not bind.
fn register_map_classname(server: &mut Server<GuestServerLogic>, classname: &str) {
    let definition = format!("map:{classname}");
    server.spawns_mut().register(
        classname,
        Box::new(move |fields| {
            Ok(SpawnRequest {
                definition: definition.clone(),
                origin: Some(fields.origin),
                combat: None,
                grants: Vec::new(),
            })
        }),
    );
}

/// Native Q1 spawn context: brush-model bounds for door/trigger
/// sizing plus the live behavior set the native spawns populate.
pub struct Q1SpawnContext {
    /// Brush-model bounds by model index (`*N`).
    pub models: Vec<qa_core::math::Bounds>,
    /// Live native behaviors, shared with the server hooks.
    pub behaviors: Rc<RefCell<Q1NativeBehaviors>>,
    /// Whether the mounts hold the registered version (shareware gates).
    pub registered: bool,
}

/// Context for spawning one map's records.
pub struct MapSpawnContext {
    /// Stock skill level for Q1 spawnflags inhibition.
    pub skill: u8,
    /// Deathmatch mode for Q1 spawnflags inhibition.
    pub deathmatch: bool,
    /// Cooperative rules for Q1 monster target re-selection.
    pub coop: bool,
    /// Native Q1 spawn path (`None` for other families).
    pub q1: Option<Q1SpawnContext>,
}

/// Spawn parsed entity records into a server.
///
/// Classnames outside the registry gain a generic `map:{classname}`
/// function first (deterministic order); records that fail field parse or
/// spawn are collected as skips instead of aborting the load. Q1 maps
/// additionally run the native spawn path: the stock pre-spawn filter
/// (inhibition, light/static removal), native spawn functions, door
/// sizing/linking, trigger/button sizing, the targetname index, and the
/// native hook install.
///
/// Door/trigger/button-build failures release the spawned actor and
/// record a skip; door-link failures record a skip; none aborts the load.
pub fn spawn_map_entities(
    server: &mut Server<GuestServerLogic>,
    entities: &[Vec<(String, String)>],
    source: &str,
    context: &MapSpawnContext,
) -> MapSpawnSummary {
    let mut summary = MapSpawnSummary::default();
    if let Some(q1) = context.q1.as_ref() {
        register_q1_spawns(server.spawns_mut());
        register_q1_trigger_spawns(server.spawns_mut());
        register_q1_item_spawns(server.spawns_mut());
        register_q1_monster_spawns(server.spawns_mut());
        q1.behaviors.borrow_mut().registered = q1.registered;
        q1.behaviors.borrow_mut().deathmatch = context.deathmatch;
        q1.behaviors.borrow_mut().skill = context.skill;
        q1.behaviors.borrow_mut().coop = context.coop;
        q1.behaviors.borrow_mut().mapname = q1_mapname(source);
    }
    let mut classnames = BTreeSet::new();
    for properties in entities {
        if let Some((_, classname)) = properties.iter().find(|(key, _)| key == "classname") {
            classnames.insert(classname.clone());
        }
    }
    for classname in &classnames {
        let known = server.spawns_mut().classnames().iter().any(|known| known == classname);
        if !known {
            register_map_classname(server, classname);
        }
    }
    let mut pending_doors: Vec<Q1PendingDoor> = Vec::new();
    for (index, properties) in entities.iter().enumerate() {
        let pairs: Vec<(&str, &str)> = properties
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        let classname = properties
            .iter()
            .find(|(key, _)| key == "classname")
            .map_or("", |(_, value)| value.as_str())
            .to_string();
        let fields = match SpawnFields::parse(&pairs) {
            Ok(fields) => fields,
            Err(error) => {
                summary.skipped.push(SkippedEntity {
                    index,
                    classname,
                    reason: format!("{source}: invalid spawn fields: {error}"),
                });
                continue;
            }
        };
        if let Some(q1) = context.q1.as_ref() {
            match q1_pre_spawn(&classname, &fields, context.skill, context.deathmatch) {
                Q1PreSpawn::Skip(reason) => {
                    summary.skipped.push(SkippedEntity {
                        index,
                        classname,
                        reason: format!("{source}: {reason}"),
                    });
                    continue;
                }
                Q1PreSpawn::Spawn => {}
            }
            if classname == "func_door" {
                match server.spawn_entity(&fields) {
                    Ok(actor) => {
                        let built = {
                            let models = &q1.models;
                            let mut behaviors = q1.behaviors.borrow_mut();
                            build_q1_door(server, &mut behaviors, &actor, &fields, models)
                        };
                        match built {
                            Ok(pending) => {
                                pending_doors.push(pending);
                                q1_note_targetname(&mut q1.behaviors.borrow_mut(), &fields, actor.id());
                                summary.spawned += 1;
                            }
                            Err(error) => {
                                let _ignored = server.simulation_mut().release(&actor);
                                summary.skipped.push(SkippedEntity {
                                    index,
                                    classname,
                                    reason: format!("{source}: door build failed: {error}"),
                                });
                            }
                        }
                    }
                    Err(error) => summary.skipped.push(SkippedEntity {
                        index,
                        classname,
                        reason: format!("{source}: spawn failed: {error}"),
                    }),
                }
                continue;
            }
            if q1_is_brush_trigger(&classname) || classname == "func_button" {
                let build = if classname == "func_button" {
                    "button"
                } else {
                    "trigger"
                };
                match server.spawn_entity(&fields) {
                    Ok(actor) => {
                        let built = {
                            let models = &q1.models;
                            let mut behaviors = q1.behaviors.borrow_mut();
                            if classname == "func_button" {
                                build_q1_button(server, &mut behaviors, &actor, &fields, models)
                            } else {
                                build_q1_trigger(server, &mut behaviors, &actor, &fields, models)
                            }
                        };
                        match built {
                            Ok(()) => {
                                q1_note_targetname(&mut q1.behaviors.borrow_mut(), &fields, actor.id());
                                summary.spawned += 1;
                            }
                            Err(error) => {
                                let _ignored = server.simulation_mut().release(&actor);
                                summary.skipped.push(SkippedEntity {
                                    index,
                                    classname,
                                    reason: format!("{source}: {build} build failed: {error}"),
                                });
                            }
                        }
                    }
                    Err(error) => summary.skipped.push(SkippedEntity {
                        index,
                        classname,
                        reason: format!("{source}: spawn failed: {error}"),
                    }),
                }
                continue;
            }
            if q1_is_use_point(&classname) {
                match server.spawn_entity(&fields) {
                    Ok(actor) => {
                        let mut behaviors = q1.behaviors.borrow_mut();
                        q1_note_use_point(&mut behaviors, actor.id(), &fields);
                        q1_note_targetname(&mut behaviors, &fields, actor.id());
                        summary.spawned += 1;
                    }
                    Err(error) => summary.skipped.push(SkippedEntity {
                        index,
                        classname,
                        reason: format!("{source}: spawn failed: {error}"),
                    }),
                }
                continue;
            }
            if q1_is_item(&classname) {
                match server.spawn_entity(&fields) {
                    Ok(actor) => {
                        let built = build_q1_item(server, &mut q1.behaviors.borrow_mut(), &actor, &fields);
                        match built {
                            Ok(()) => {
                                q1_note_targetname(&mut q1.behaviors.borrow_mut(), &fields, actor.id());
                                summary.spawned += 1;
                            }
                            Err(error) => {
                                let _ignored = server.simulation_mut().release(&actor);
                                summary.skipped.push(SkippedEntity {
                                    index,
                                    classname,
                                    reason: format!("{source}: item build failed: {error}"),
                                });
                            }
                        }
                    }
                    Err(error) => summary.skipped.push(SkippedEntity {
                        index,
                        classname,
                        reason: format!("{source}: spawn failed: {error}"),
                    }),
                }
                continue;
            }
            if q1_is_monster(&classname) || q1_is_movetarget(&classname) {
                let build = if q1_is_monster(&classname) {
                    "monster"
                } else {
                    "movetarget"
                };
                match server.spawn_entity(&fields) {
                    Ok(actor) => {
                        let built = if q1_is_monster(&classname) {
                            build_q1_monster(server, &mut q1.behaviors.borrow_mut(), &actor, &fields)
                        } else {
                            build_q1_movetarget(server, &mut q1.behaviors.borrow_mut(), &actor, &fields)
                        };
                        match built {
                            Ok(()) => {
                                q1_note_targetname(&mut q1.behaviors.borrow_mut(), &fields, actor.id());
                                summary.spawned += 1;
                            }
                            Err(error) => {
                                let _ignored = server.simulation_mut().release(&actor);
                                summary.skipped.push(SkippedEntity {
                                    index,
                                    classname,
                                    reason: format!("{source}: {build} build failed: {error}"),
                                });
                            }
                        }
                    }
                    Err(error) => summary.skipped.push(SkippedEntity {
                        index,
                        classname,
                        reason: format!("{source}: spawn failed: {error}"),
                    }),
                }
                continue;
            }
        }
        match server.spawn_entity(&fields) {
            Ok(actor) => {
                if let Some(q1) = context.q1.as_ref() {
                    let mut behaviors = q1.behaviors.borrow_mut();
                    q1_note_solid(&classname, actor.id(), &mut behaviors);
                    q1_note_targetname(&mut behaviors, &fields, actor.id());
                    if classname == "light" {
                        q1_note_light(&mut behaviors, actor.id(), &fields);
                    }
                    if classname == "worldspawn" {
                        q1_note_worldspawn(&mut behaviors, &fields);
                    }
                    if classname == "info_intermission" {
                        q1_note_intermission(&mut behaviors, &fields);
                    }
                    if classname == "info_player_start" || classname == "testplayerstart" {
                        q1_note_start_spot(&mut behaviors, &fields);
                    }
                    if classname == "info_teleport_destination" {
                        if let Err(error) = q1_note_teleport_destination(&mut behaviors, actor.id(), &fields) {
                            let _ignored = server.simulation_mut().release(&actor);
                            summary.skipped.push(SkippedEntity {
                                index,
                                classname,
                                reason: format!("{source}: destination record failed: {error}"),
                            });
                            continue;
                        }
                    }
                }
                summary.spawned += 1;
            }
            Err(error) => summary.skipped.push(SkippedEntity {
                index,
                classname,
                reason: format!("{source}: spawn failed: {error}"),
            }),
        }
    }
    if let Some(q1) = context.q1.as_ref() {
        if let Err(error) = link_q1_doors(server, &mut q1.behaviors.borrow_mut(), pending_doors) {
            summary.skipped.push(SkippedEntity {
                index: entities.len(),
                classname: "func_door".to_string(),
                reason: format!("{source}: door link failed: {error}"),
            });
        }
        install_q1_native(server, Rc::clone(&q1.behaviors));
    }
    summary
}

/// Stock `mapname` stem for a map resource path (`maps/e1m1.bsp` →
/// `e1m1`): the leaf name without its extension (`SV_SpawnServer`
/// names the level for `client.qc`).
fn q1_mapname(source: &str) -> String {
    let leaf = source.rsplit('/').next().unwrap_or(source);
    leaf.rsplit_once('.').map_or(leaf, |(stem, _)| stem).to_string()
}

/// Selected map read through product mounts and decoded (shared by the
/// windowed and dedicated loaders).
pub struct SelectedMap {
    /// Raw map bytes (clip and presentation builds read these).
    pub bytes: Vec<u8>,
    /// Classified BSP kind.
    pub kind: BspKind,
    /// Entity records as ordered key/value property lists.
    pub entities: Vec<Vec<(String, String)>>,
    /// Q1 brush-model bounds by model index (`*N`); empty for other kinds.
    pub q1_models: Vec<qa_core::math::Bounds>,
}

/// Read `map` through opened product mounts, classify, and decode its
/// entity records (plus Q1 brush-model bounds for door sizing).
pub fn load_selected_map(mounts: &MountedContent, content: &str, map: &str) -> Result<SelectedMap, PlayWorldError> {
    let bytes = mounts
        .read(qa_content::mounts::ResourceRef::Path(map))
        .map_err(|error| PlayWorldError::MapUnread {
            content: content.to_string(),
            map: map.to_string(),
            reason: error.to_string(),
        })?;
    let kind = classify_bsp(&bytes, map).map_err(|error| PlayWorldError::MapDecode {
        map: map.to_string(),
        reason: error.to_string(),
    })?;
    let decoded = decode_map_entities(&bytes, map, kind)?;
    Ok(SelectedMap {
        bytes,
        kind,
        entities: decoded.records,
        q1_models: decoded.q1_models,
    })
}

/// Load the selected map's real entities into a live server.
///
/// Reads `options.map` through the `--map-game`/`--game` product mounts,
/// decodes the BSP for the map's family, parses the entity string, and
/// spawns every record, then builds the scene presentation over `owner`
/// (the windowed renderer's resource owner, so the presentation's image
/// uploads apply to the live backend). Fails honestly when the product is
/// unknown, the map is unreadable or undecodable, or no record spawns.
pub fn load_play_world(
    config: &StartupConfig,
    catalog: &InstalledCatalog,
    options: &ApplicationOptions,
    owner: qa_client::render::types::ResourceOwner,
) -> Result<PlayWorld, PlayWorldError> {
    let content = map_content_id(options).to_string();
    let product = catalog
        .require(&content)
        .map_err(|_| PlayWorldError::UnknownProduct(content.clone()))?;
    let family = product.expectation.family;
    let campaign = product.expectation.campaign.clone();
    let movement_edition = movement_content_edition(catalog, options.movement_product.as_deref())
        .map_err(|_| PlayWorldError::UnknownProduct(options.movement_product.clone().unwrap_or_default()))?;
    let dialect = movement_dialect_for_selection(
        super::content::content_family(options.movement),
        options.movement_product.as_deref(),
        &movement_edition,
    );
    let mounts = open_product_mounts(catalog, &content, &options.map)?;
    let selected = load_selected_map(&mounts, &content, &options.map)?;
    let bytes = selected.bytes;
    let kind = selected.kind;
    let entities = selected.entities;
    let q1_models = selected.q1_models;
    let sound_family = match kind {
        BspKind::Q1 => SoundFamily::Q1,
        BspKind::Q2 => SoundFamily::Q2,
        BspKind::Q3 => SoundFamily::Q3,
    };
    let speakers = map_speakers(&entities, sound_family);
    let mut server = open_server(config).map_err(|error| PlayWorldError::Server(error.to_string()))?;
    let context = MapSpawnContext {
        skill: options.skill,
        deathmatch: options.mode == GameMode::Deathmatch,
        coop: options.mode == GameMode::Coop,
        q1: (kind == BspKind::Q1).then(|| Q1SpawnContext {
            models: q1_models,
            behaviors: Rc::new(RefCell::new(Q1NativeBehaviors::new())),
            registered: q1_registered_version(&mounts),
        }),
    };
    let summary = spawn_map_entities(&mut server, &entities, &options.map, &context);
    if summary.spawned == 0 {
        let reason = summary.skipped.first().map_or_else(
            || "map has no entity records".to_string(),
            |skipped| skipped.reason.clone(),
        );
        return Err(PlayWorldError::NothingSpawned {
            map: options.map.clone(),
            records: entities.len(),
            reason,
        });
    }
    let clip = build_clip(&bytes, &options.map, family).map_err(|reason| PlayWorldError::Play {
        map: options.map.clone(),
        reason,
    })?;
    let player = match select_spawn(&entities, kind) {
        Some(spawn) => {
            let feet = vec3(
                spawn.origin.x,
                spawn.origin.y,
                spawn.origin.z - eye_height_for_family(family),
            );
            admit_player(
                server.simulation_mut(),
                family,
                provider_for_product(family, &campaign),
                feet,
                spawn.angles,
                dialect,
                &movement_edition,
            )
            .map_err(|reason| PlayWorldError::Play {
                map: options.map.clone(),
                reason,
            })?
        }
        None => None,
    };
    if let (Some(player), Some(q1)) = (player.as_ref(), context.q1.as_ref()) {
        let mut behaviors = q1.behaviors.borrow_mut();
        behaviors.set_player(Some(PlayerBody::actor(player).clone()));
        // Stock players spawn `SOLID_SLIDEBOX`; the scene links the
        // mover like every other solid and skips it via passentity.
        behaviors.solids.insert(PlayerBody::actor(player));
        // Stock players always carry health (`PutClientInServer`); the
        // touch gates (door fields, hurt, push) read it.
        let _ignored = server
            .simulation_mut()
            .set_combat(PlayerBody::actor(player), qa_world::combat::CombatState::default());
        // Stock spawn loadout (`PutClientInServer` over fresh parms):
        // axe and shotgun with 25 shells, shotgun in hand.
        q1_grant_spawn_loadout(&mut behaviors);
        behaviors.player_angles = player.eye().1;
    }
    // The presentation consumes its mounts, so audio keeps a second open over
    // the same product: without retained mounts no bank can open `sound/*`
    // bytes. Best-effort only; a failed audio open keeps the run silent.
    let audio_mounts = match open_product_mounts(catalog, &content, &options.map) {
        Ok(mounts) => Some(Rc::new(mounts)),
        Err(error) => {
            eprintln!("windowed: game audio unavailable ({error})");
            None
        }
    };
    let (presentation, presentation_error) = match build_presentation(mounts, &options.map, &bytes, &entities, owner) {
        Ok(presentation) => (Some(presentation), None),
        Err(error) => (None, Some(error.to_string())),
    };
    Ok(PlayWorld {
        server,
        content,
        map: options.map.clone(),
        entity_records: entities.len(),
        spawned: summary.spawned,
        skipped: summary.skipped,
        presentation,
        presentation_error,
        clip,
        player,
        dialect,
        audio_mounts,
        speakers,
        sound_family,
        q1_behaviors: context.q1.map(|q1| q1.behaviors),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use qa_content::catalog::DiscoverContentOptions;

    use super::*;
    use crate::bootstrap::live_proof::{require_live_corpus_any, require_live_data};

    fn steel_catalog() -> Option<InstalledCatalog> {
        let root = require_live_corpus_any("Steel game data", &["q1", "q2", "q3a"])?;
        require_live_data(
            "Steel installed-content catalog",
            qa_content::catalog::discover_installed_content(&DiscoverContentOptions::new(root)).ok(),
        )
    }

    fn test_config(options: &ApplicationOptions) -> StartupConfig {
        StartupConfig::from_options(options).unwrap()
    }

    fn test_owner() -> qa_client::render::types::ResourceOwner {
        let session = qa_core::identity::IdentityOwner::create("windowed-world-test")
            .unwrap()
            .session()
            .clone();
        qa_client::render::types::ResourceOwner::new(7, session, 0)
    }

    fn generic_context() -> MapSpawnContext {
        MapSpawnContext {
            skill: 1,
            deathmatch: false,
            coop: false,
            q1: None,
        }
    }

    fn q1_context() -> (MapSpawnContext, Rc<RefCell<Q1NativeBehaviors>>) {
        let behaviors = Rc::new(RefCell::new(Q1NativeBehaviors::new()));
        let context = MapSpawnContext {
            skill: 1,
            deathmatch: false,
            coop: false,
            q1: Some(Q1SpawnContext {
                models: vec![qa_core::math::Bounds {
                    min: vec3(0.0, 0.0, 0.0),
                    max: vec3(64.0, 64.0, 128.0),
                }],
                behaviors: Rc::clone(&behaviors),
                registered: true,
            }),
        };
        (context, behaviors)
    }

    fn record(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn q1_context_runs_native_spawns_with_generic_fallback() {
        let options = ApplicationOptions::default();
        let config = test_config(&options);
        let mut server = open_server(&config).unwrap();
        let (context, behaviors) = q1_context();
        let entities = vec![
            record(&[("classname", "worldspawn")]),
            record(&[("classname", "info_player_start"), ("origin", "0 0 32")]),
            record(&[("classname", "light"), ("origin", "0 0 64")]),
            record(&[
                ("classname", "light"),
                ("origin", "0 0 72"),
                ("targetname", "t1"),
                ("style", "32"),
            ]),
            record(&[("classname", "light_torch_small_walltorch"), ("origin", "0 0 80")]),
            record(&[("classname", "func_door"), ("origin", "0 0 0"), ("model", "*0")]),
            record(&[("classname", "func_door"), ("origin", "512 0 0")]),
            record(&[
                ("classname", "monster_ogre"),
                ("origin", "256 0 32"),
                ("spawnflags", "512"),
            ]),
            record(&[("classname", "monster_ogre"), ("origin", "288 0 32")]),
        ];
        let summary = spawn_map_entities(&mut server, &entities, "maps/test.bsp", &context);
        // Native: worldspawn, start, targeted light, modelled door;
        // generic: the uninhibited ogre.
        assert_eq!(summary.spawned, 5);
        // Inert light, torch static, unmodelled door, inhibited ogre.
        assert_eq!(summary.skipped.len(), 4);
        assert!(summary
            .skipped
            .iter()
            .any(|skipped| skipped.reason.contains("door build failed")));
        // The failed door actor was released: 5 spawns plus 1 field.
        assert_eq!(server.simulation().actor_count(), 6);
        assert_eq!(behaviors.borrow().doors.len(), 1);
        assert_eq!(behaviors.borrow().fields.len(), 1);
    }

    #[test]
    fn map_content_prefers_map_game() {
        let options = ApplicationOptions::default();
        assert_eq!(map_content_id(&options), "q2-classic-baseq2");
        let options = ApplicationOptions {
            map_product: Some("q3-baseq3".to_string()),
            ..ApplicationOptions::default()
        };
        assert_eq!(map_content_id(&options), "q3-baseq3");
    }

    #[test]
    fn generic_spawns_cover_every_map_classname() {
        let options = ApplicationOptions::default();
        let config = test_config(&options);
        let mut server = open_server(&config).unwrap();
        let entities: Vec<Vec<(String, String)>> = [
            ("worldspawn", "0 0 0"),
            ("info_player_start", "0 0 32"),
            ("weapon_rocketlauncher", "128 0 32"),
            ("ammo_rockets", "128 64 32"),
            ("light", "0 128 64"),
            ("monster_ogre", "256 0 32"),
            ("trigger_once", "0 256 32"),
            ("func_door", "512 0 32"),
        ]
        .into_iter()
        .map(|(classname, origin)| {
            vec![
                ("classname".to_string(), classname.to_string()),
                ("origin".to_string(), origin.to_string()),
            ]
        })
        .collect();
        let summary = spawn_map_entities(&mut server, &entities, "maps/test.bsp", &generic_context());
        assert!(summary.skipped.is_empty(), "skips: {:?}", summary.skipped);
        assert_eq!(summary.spawned, entities.len());
        assert_eq!(server.simulation().actor_count(), entities.len());
        assert!(server.simulation().actor_count() > 5);
    }

    #[test]
    fn bad_records_skip_without_aborting() {
        let options = ApplicationOptions::default();
        let config = test_config(&options);
        let mut server = open_server(&config).unwrap();
        let entities = vec![
            vec![
                ("classname".to_string(), "light".to_string()),
                ("origin".to_string(), "not a vector".to_string()),
            ],
            vec![("targetname".to_string(), "nameless".to_string())],
            vec![
                ("classname".to_string(), "info_player_start".to_string()),
                ("origin".to_string(), "0 0 32".to_string()),
            ],
        ];
        let summary = spawn_map_entities(&mut server, &entities, "maps/test.bsp", &generic_context());
        assert_eq!(summary.spawned, 1);
        assert_eq!(summary.skipped.len(), 2);
        assert_eq!(summary.skipped[0].index, 0);
        assert_eq!(summary.skipped[0].classname, "light");
        assert_eq!(summary.skipped[1].index, 1);
        assert_eq!(summary.skipped[1].classname, "");
        assert_eq!(server.simulation().actor_count(), 1);
    }

    #[test]
    fn q1_brace_syntax_decodes_q2_entity_strings() {
        let entities = parse_q1_entities(
            "{\n\"classname\" \"worldspawn\"\n\"message\" \"base1\"\n}\n{\n\"classname\" \"info_player_start\"\n\"origin\" \"0 0 32\"\n}\n",
            "maps/base1.bsp:entities",
        )
        .unwrap();
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[1].properties.len(), 2);
    }

    #[test]
    fn missing_content_reports_an_honest_error() {
        let options = ApplicationOptions {
            corpus_root: "/tmp/qa-windowed-world-missing".to_string(),
            product: "q3-baseq3".to_string(),
            map: "maps/q3dm1.bsp".to_string(),
            ..ApplicationOptions::default()
        };
        let catalog = qa_content::catalog::discover_installed_content(&DiscoverContentOptions::new(PathBuf::from(
            &options.corpus_root,
        )))
        .unwrap();
        let config = test_config(&options);
        let error = load_play_world(&config, &catalog, &options, test_owner()).unwrap_err();
        let message = error.to_string();
        assert!(
            matches!(
                error,
                PlayWorldError::UnknownProduct(_) | PlayWorldError::MapUnread { .. }
            ),
            "{message}"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_steel_maps_spawn_more_than_stub() {
        let Some(catalog) = steel_catalog() else {
            return;
        };
        let mut loaded = 0;
        for (product, map) in [
            ("q3-baseq3", "maps/q3dm1.bsp"),
            ("q1-classic-id1", "maps/e1m1.bsp"),
            ("q2-classic-baseq2", "maps/base1.bsp"),
        ] {
            let options = ApplicationOptions {
                product: product.to_string(),
                map: map.to_string(),
                ..ApplicationOptions::default()
            };
            let config = test_config(&options);
            let world = match load_play_world(&config, &catalog, &options, test_owner()) {
                Ok(world) => world,
                Err(error) => {
                    require_live_data::<()>(&format!("{product} {map} load ({error})"), None);
                    continue;
                }
            };
            assert_eq!(world.content(), product);
            assert_eq!(world.map(), map);
            assert_eq!(world.entity_records(), world.spawned() + world.skipped().len());
            assert!(
                world.entity_count() > 5,
                "{product} {map}: expected more than the stub 5, got {}",
                world.entity_count()
            );
            let players = usize::from(world.has_player());
            // Native Q1 doors add trigger-field actors beyond the spawned
            // records (one per LinkDoors chain); other families add none.
            let fields = world
                .q1_behaviors()
                .map(|behaviors| behaviors.borrow().fields.len())
                .unwrap_or(0);
            assert_eq!(world.entity_count(), world.spawned() + players + fields);
            loaded += 1;
        }
        assert!(loaded > 0, "expected at least one Steel map to load");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_e1m1_descent_stalls_at_hall_wall() {
        use qa_world::movement::types::{Q1UserCommand, UserCommand};

        let Some(catalog) = steel_catalog() else {
            return;
        };
        let options = ApplicationOptions {
            product: "q1-classic-id1".to_string(),
            map: "maps/e1m1.bsp".to_string(),
            ..ApplicationOptions::default()
        };
        let config = test_config(&options);
        let mut world = match load_play_world(&config, &catalog, &options, test_owner()) {
            Ok(world) => world,
            Err(error) => {
                require_live_data::<()>(&format!("q1-classic-id1 maps/e1m1.bsp load ({error})"), None);
                return;
            }
        };
        assert!(world.has_player(), "Q1 e1m1 world admits no player");
        let (start_eye, angles) = world.player_eye().expect("player eye");
        let mut lowest = start_eye.z;
        let mut at_600 = start_eye;
        for step in 0..660 {
            let command = UserCommand::Q1Netquake(Q1UserCommand {
                acknowledged_server_time_seconds: f64::from(step) / 60.0,
                view_angles: angles,
                forward_move: 200.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0,
                impulse: 0,
            });
            world
                .server_mut()
                .tick(qa_core::time::SourceTime::Seconds(1.0 / 60.0))
                .unwrap();
            world.step_player(command).unwrap();
            let (eye, _) = world.player_eye().expect("player eye");
            lowest = lowest.min(eye.z);
            if step == 599 {
                at_600 = eye;
            }
        }
        let (eye, _) = world.player_eye().expect("player eye");
        assert!(
            lowest < start_eye.z - 40.0,
            "player never descended the entry ramp: lowest {lowest} from {start_eye:?}"
        );
        assert!(at_600.y > 600.0, "player never reached the hall: {at_600:?}");
        let pinned = ((eye.x - at_600.x) as f64).hypot((eye.y - at_600.y) as f64);
        assert!(pinned < 1.0, "player never stalled at the wall: {eye:?} vs {at_600:?}");
        assert!((eye.z - 46.0).abs() < 4.0, "player left the hall floor: {eye:?}");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_world_admits_player_and_eye_follows_steps() {
        use qa_world::movement::types::{Q1UserCommand, UserCommand};

        let Some(catalog) = steel_catalog() else {
            return;
        };
        let options = ApplicationOptions {
            product: "q1-classic-id1".to_string(),
            map: "maps/start.bsp".to_string(),
            ..ApplicationOptions::default()
        };
        let config = test_config(&options);
        let mut world = match load_play_world(&config, &catalog, &options, test_owner()) {
            Ok(world) => world,
            Err(error) => {
                require_live_data::<()>(&format!("q1-classic-id1 maps/start.bsp load ({error})"), None);
                return;
            }
        };
        assert!(world.has_player(), "Q1 world admits no player");
        let (start_eye, angles) = world.player_eye().expect("player eye");
        for step in 0..30 {
            let command = UserCommand::Q1Netquake(Q1UserCommand {
                acknowledged_server_time_seconds: f64::from(step) / 60.0,
                view_angles: angles,
                forward_move: 200.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0,
                impulse: 0,
            });
            world
                .server_mut()
                .tick(qa_core::time::SourceTime::Seconds(1.0 / 60.0))
                .unwrap();
            world.step_player(command).unwrap();
        }
        let (eye, _) = world.player_eye().expect("player eye");
        let moved = ((eye.x - start_eye.x) as f64).hypot((eye.y - start_eye.y) as f64);
        assert!(moved > 5.0, "eye did not follow the player: {eye:?} from {start_eye:?}");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_start_doors_travel_along_map_angles() {
        use qa_core::math::{normalize3, sub3};

        use super::super::simulation::native_q1_spawns::q1_movedir;

        let Some(catalog) = steel_catalog() else {
            return;
        };
        let options = ApplicationOptions {
            product: "q1-classic-id1".to_string(),
            map: "maps/start.bsp".to_string(),
            ..ApplicationOptions::default()
        };
        let config = test_config(&options);
        let mut world = match load_play_world(&config, &catalog, &options, test_owner()) {
            Ok(world) => world,
            Err(error) => {
                require_live_data::<()>(&format!("q1-classic-id1 maps/start.bsp load ({error})"), None);
                return;
            }
        };
        // Expected travel directions, read straight from the raw map
        // records (independent of `SpawnFields::parse`, so the pre-fix
        // +X collapse fails this proof instead of passing vacuously).
        let Some(mounts) = require_live_data(
            "q1-classic-id1 mounts for maps/start.bsp",
            open_product_mounts(&catalog, "q1-classic-id1", "maps/start.bsp").ok(),
        ) else {
            return;
        };
        let Some(bytes) = require_live_data(
            "maps/start.bsp bytes",
            mounts
                .read(qa_content::mounts::ResourceRef::Path("maps/start.bsp"))
                .ok(),
        ) else {
            return;
        };
        let records = decode_map_entities(&bytes, "maps/start.bsp", BspKind::Q1)
            .unwrap()
            .records;
        let mut expected = Vec::new();
        for record in &records {
            let get = |key: &str| {
                record
                    .iter()
                    .find(|(name, _)| name == key)
                    .map(|(_, value)| value.as_str())
            };
            if get("classname") != Some("func_door") {
                continue;
            }
            let angles = match (get("angles"), get("angle")) {
                (Some(triple), _) => {
                    let mut parts = triple.split_whitespace();
                    let (x, y, z) = (parts.next(), parts.next(), parts.next());
                    match (x, y, z, parts.next()) {
                        (Some(x), Some(y), Some(z), None) => vec3(
                            x.parse::<f32>().unwrap(),
                            y.parse::<f32>().unwrap(),
                            z.parse::<f32>().unwrap(),
                        ),
                        _ => panic!("start.bsp door has a malformed angles triple"),
                    }
                }
                (None, Some(yaw)) => vec3(0.0, yaw.parse::<f32>().unwrap(), 0.0),
                (None, None) => vec3(0.0, 0.0, 0.0),
            };
            expected.push(q1_movedir(angles));
        }
        assert!(!expected.is_empty(), "start.bsp has func_door records");
        assert!(
            expected.iter().any(|dir| (dir.x - 1.0).abs() > 0.5),
            "start.bsp pins doors that do not travel +X"
        );
        // Actual travel directions from the live door movers.
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let door_ids: Vec<_> = behaviors.borrow().doors.keys().cloned().collect();
        let mut actual = Vec::new();
        for id in &door_ids {
            let mover = world.server_mut().movers_mut().get(id).expect("door mover");
            let travel = sub3(mover.pos2, mover.pos1);
            let length = f64::from(travel.x)
                .hypot(f64::from(travel.y))
                .hypot(f64::from(travel.z));
            assert!(length > 0.0, "door {id:?} travels somewhere");
            actual.push(normalize3(travel));
        }
        assert_eq!(actual.len(), expected.len(), "every map door spawned a mover");
        let mut expected_sorted = expected.clone();
        let mut actual_sorted = actual.clone();
        for sorted in [&mut expected_sorted, &mut actual_sorted] {
            sorted.sort_by(|left, right| {
                left.x
                    .total_cmp(&right.x)
                    .then(left.y.total_cmp(&right.y))
                    .then(left.z.total_cmp(&right.z))
            });
        }
        for (index, (got, want)) in actual_sorted.iter().zip(expected_sorted.iter()).enumerate() {
            let delta = sub3(*got, *want);
            let distance = f64::from(delta.x).hypot(f64::from(delta.y)).hypot(f64::from(delta.z));
            assert!(distance < 1e-3, "door {index} travels {got:?}, map angle says {want:?}");
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_e1m1_closing_door_crushes_and_reverses() {
        use qa_core::identity::ActorId;
        use qa_core::math::sub3;
        use qa_world::body::translated_body_bounds;
        use qa_world::movers::{use_mover, MoverPhase};

        let Some(catalog) = steel_catalog() else {
            return;
        };
        let options = ApplicationOptions {
            product: "q1-classic-id1".to_string(),
            map: "maps/e1m1.bsp".to_string(),
            ..ApplicationOptions::default()
        };
        let config = test_config(&options);
        let mut world = match load_play_world(&config, &catalog, &options, test_owner()) {
            Ok(world) => world,
            Err(error) => {
                require_live_data::<()>(&format!("q1-classic-id1 maps/e1m1.bsp load ({error})"), None);
                return;
            }
        };
        let player = world.player_actor().cloned().expect("e1m1 admits a player");
        // Pick a reversing door whose swept volume holds no other bodies,
        // so the opening below is deterministic. Brush doors sit at origin
        // zero with offset bounds, so sweep the absolute bounds.
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let doors = behaviors.borrow().doors.clone();
        let server = world.server_mut();
        let mut bodies = Vec::new();
        for id in server.simulation().body_actors() {
            let Some(body) = server.simulation().body_state(&id) else {
                continue;
            };
            let skipped = server.movers_mut().get(&id).is_some() || server.triggers_mut().is_trigger(&id);
            bodies.push((translated_body_bounds(&body), skipped));
        }
        let mut picked: Option<(ActorId, f32)> = None;
        let mut door_ids: Vec<_> = doors.keys().cloned().collect();
        door_ids.sort_by(|left, right| {
            left.slot()
                .cmp(&right.slot())
                .then(left.generation().cmp(&right.generation()))
        });
        for id in &door_ids {
            let door = &doors[id];
            if door.wait < 0.0 {
                continue;
            }
            let mover = world.server_mut().movers_mut().get(id).cloned().expect("door mover");
            if mover.phase != MoverPhase::AtPos1 {
                continue;
            }
            let travel = sub3(mover.pos2, mover.pos1);
            let length = f64::from(travel.x)
                .hypot(f64::from(travel.y))
                .hypot(f64::from(travel.z));
            if length < 1.0 {
                continue;
            }
            let closed = translated_body_bounds(&world.server().simulation().body_state(id).expect("door body"));
            let (lo, hi) = (
                vec3(
                    closed.min.x.min(closed.min.x + travel.x) - 40.0,
                    closed.min.y.min(closed.min.y + travel.y) - 40.0,
                    closed.min.z.min(closed.min.z + travel.z) - 40.0,
                ),
                vec3(
                    closed.max.x.max(closed.max.x + travel.x) + 40.0,
                    closed.max.y.max(closed.max.y + travel.y) + 40.0,
                    closed.max.z.max(closed.max.z + travel.z) + 40.0,
                ),
            );
            let overlaps = |bounds: &qa_core::math::Bounds| {
                bounds.min.x < hi.x
                    && bounds.min.y < hi.y
                    && bounds.min.z < hi.z
                    && bounds.max.x > lo.x
                    && bounds.max.y > lo.y
                    && bounds.max.z > lo.z
            };
            let clear = bodies.iter().all(|(bounds, skipped)| *skipped || !overlaps(bounds));
            if clear {
                picked = Some((id.clone(), door.dmg));
                break;
            }
        }
        let (door_id, crush_dmg) = picked.expect("e1m1 has a crush-testable door");
        let crush_dmg = f64::from(crush_dmg);
        // Throttle every trigger field past the proof so touches never
        // re-fire the door while it closes onto the player; the blocked
        // path below is the only gamecode in play.
        {
            let now = world.server().simulation().frame().time.as_seconds_f64();
            for field in behaviors.borrow_mut().fields.values_mut() {
                field.throttle_until = now + 3600.0;
            }
        }
        // Open the door, then stand the player in the closed volume.
        {
            let server = world.server_mut();
            let origin = server.simulation().body_state(&door_id).unwrap().origin;
            use_mover(server.movers_mut().get_mut(&door_id).unwrap(), origin);
        }
        let mut opened = false;
        for _ in 0..600 {
            let phase = world.server_mut().movers_mut().get(&door_id).unwrap().phase;
            if phase == MoverPhase::AtPos2 {
                opened = true;
                break;
            }
            world
                .server_mut()
                .tick(qa_core::time::SourceTime::Seconds(1.0 / 60.0))
                .unwrap();
        }
        assert!(opened, "test door opens without obstruction");
        // Stand the player inside the door's open volume: the first
        // closing tick overlaps old and new bounds at once, so the
        // transaction blocks instead of shoving the player ahead.
        let open = translated_body_bounds(&world.server().simulation().body_state(&door_id).unwrap());
        let crush_at = vec3(
            (open.min.x + open.max.x) / 2.0,
            (open.min.y + open.max.y) / 2.0,
            (open.min.z + open.max.z) / 2.0,
        );
        world
            .server_mut()
            .simulation_mut()
            .set_body_origin(&player, crush_at)
            .unwrap();
        {
            let server = world.server_mut();
            let origin = server.simulation().body_state(&door_id).unwrap().origin;
            use_mover(server.movers_mut().get_mut(&door_id).unwrap(), origin);
            assert_eq!(server.movers_mut().get(&door_id).unwrap().phase, MoverPhase::ToPos1);
        }
        // Close until the door leaves its closing phase: one crush's
        // worth of damage, reversal toward open, and rollback to the
        // pre-tick origin. The reversal snaps to AtPos2: rollback already
        // restored the open origin, so the return trip has zero distance.
        let loop_health = world
            .server()
            .simulation()
            .combat_state(&player)
            .map_or(100.0, |combat| combat.health);
        let mut crushed = false;
        for _ in 0..600 {
            let before = world.server().simulation().body_state(&door_id).unwrap().origin;
            world
                .server_mut()
                .tick(qa_core::time::SourceTime::Seconds(1.0 / 60.0))
                .unwrap();
            match world.server_mut().movers_mut().get(&door_id).unwrap().phase {
                MoverPhase::ToPos1 => {}
                MoverPhase::ToPos2 | MoverPhase::AtPos2 => {
                    assert_eq!(
                        world
                            .server()
                            .simulation()
                            .combat_state(&player)
                            .map(|combat| combat.health),
                        Some(loop_health - crush_dmg)
                    );
                    assert_eq!(world.server().simulation().body_state(&door_id).unwrap().origin, before);
                    crushed = true;
                    break;
                }
                MoverPhase::AtPos1 => panic!("door closed through the player"),
            }
        }
        assert!(crushed, "closing door crushes the player and reverses");
    }

    // --- Live Q1 scenario harness (slice 1: triggers, buttons,
    // --- teleport, items) ---
    //
    // One harness drives every Q1 live proof: load a retail map through
    // the real binary path (`load_play_world`), script the player with
    // direct body placements, tick the live server, and assert
    // simulation state against qsrc values (`progs106/triggers.qc`,
    // `items.qc`, `combat.qc`) every step. The input seam is exactly
    // two helpers — `live_q1_world` (map + mode + skill selection) and
    // `live_place_player` (scripted movement) — so console driving
    // (`map`, `skill`, `setpos`-style teleports, `give`/`god`) can
    // replace the scripted inputs once the console lane merges, without
    // touching the scenarios. Scenarios are named after their checklist
    // row IDs: a passing `live_q1_NNNN_*` scenario upgrades `Q1-NNNN`
    // to done.

    /// Load a retail Q1 map for a live scenario: the input seam's
    /// selection half (future `map`/`skill`/mode console driving lands
    /// here). Returns `None` (skip, or fail under `QA_REQUIRE_LIVE=1`)
    /// when the corpus or map is unavailable.
    fn live_q1_world(map: &str, mode: GameMode, skill: u8) -> Option<PlayWorld> {
        let catalog = steel_catalog()?;
        let options = ApplicationOptions {
            product: "q1-classic-id1".to_string(),
            map: map.to_string(),
            mode,
            skill,
            ..ApplicationOptions::default()
        };
        let config = test_config(&options);
        match load_play_world(&config, &catalog, &options, test_owner()) {
            Ok(world) => Some(world),
            Err(error) => {
                require_live_data::<()>(&format!("q1-classic-id1 {map} load ({error})"), None);
                None
            }
        }
    }

    /// One live server tick at the 1/60 s host step, plus the weapon
    /// and monster passes (the production order: tick, then steps).
    /// The weapon pass runs without new input (trigger released), so
    /// it only advances anims; scenarios drive fire through
    /// `live_fire`.
    fn live_tick(world: &mut PlayWorld) {
        world
            .server_mut()
            .tick(qa_core::time::SourceTime::Seconds(1.0 / 60.0))
            .unwrap();
        world.step_weapons(None);
        world.step_monsters();
    }

    /// Master-clock seconds of the live simulation.
    fn live_now(world: &PlayWorld) -> f64 {
        world.server().simulation().frame().time.as_seconds_f64()
    }

    /// Advance the live server at least `seconds` of sim time. The Q1
    /// clamped plan caps host steps at 0.1 s, so long soaks (respawns,
    /// megahealth rot) loop 1 s host steps until sim time arrives.
    fn live_advance(world: &mut PlayWorld, seconds: f64) {
        let target = live_now(world) + seconds;
        let mut guard = 0;
        while live_now(world) < target {
            world
                .server_mut()
                .tick(qa_core::time::SourceTime::Seconds(1.0))
                .unwrap();
            world.step_weapons(None);
            world.step_monsters();
            guard += 1;
            assert!(guard < 100_000, "live_advance stalled before {target}s");
        }
    }

    /// Script the player to a map point: the input seam's movement half
    /// (future console teleports land here). No movement step runs, so
    /// the body floats exactly where placed until gamecode moves it.
    fn live_place_player(world: &mut PlayWorld, point: qa_core::math::Vec3) {
        let player = world.player_actor().cloned().expect("scenario world admits a player");
        world
            .server_mut()
            .simulation_mut()
            .set_body_origin(&player, point)
            .unwrap();
    }

    /// Player eye position (placement handle for "move away" steps).
    fn live_player_spawn_eye(world: &PlayWorld) -> qa_core::math::Vec3 {
        world.player_eye().expect("player eye").0
    }

    /// Center of an actor's absolute touch volume.
    fn live_volume_center(world: &PlayWorld, actor: &qa_core::identity::ActorId) -> qa_core::math::Vec3 {
        use qa_world::body::translated_body_bounds;
        let body = world
            .server()
            .simulation()
            .body_state(actor)
            .expect("scenario actor has a body");
        let bounds = translated_body_bounds(&body);
        vec3(
            (bounds.min.x + bounds.max.x) / 2.0,
            (bounds.min.y + bounds.max.y) / 2.0,
            (bounds.min.z + bounds.max.z) / 2.0,
        )
    }

    /// Silence door trigger fields for scenarios that do not test
    /// doors (the crush proof uses the same isolation): teleported
    /// players must not re-fire doors while the scenario asserts.
    fn live_silence_door_fields(world: &mut PlayWorld) {
        let now = live_now(world);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        for field in behaviors.borrow_mut().fields.values_mut() {
            field.throttle_until = now + 3600.0;
        }
    }

    /// Live player health (stock `PutClientInServer` grants 100).
    fn live_player_health(world: &PlayWorld) -> f64 {
        let player = world.player_actor().cloned().expect("player");
        world
            .server()
            .simulation()
            .combat_state(&player)
            .map_or(0.0, |combat| combat.health)
    }

    /// Set live player health (wounds the player for pickup proofs).
    fn live_set_player_health(world: &mut PlayWorld, health: f64) {
        use qa_world::combat::CombatState;
        let player = world.player_actor().cloned().expect("player");
        let combat = world
            .server()
            .simulation()
            .combat_state(&player)
            .cloned()
            .unwrap_or_default();
        world
            .server_mut()
            .simulation_mut()
            .set_combat(&player, CombatState { health, ..combat })
            .unwrap();
    }

    /// Trigger actors whose `SUB_UseTargets` source fires `target`.
    fn live_triggers_by_target(world: &PlayWorld, target: &str) -> Vec<qa_core::identity::ActorId> {
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        let found: Vec<qa_core::identity::ActorId> = borrowed
            .triggers
            .iter()
            .filter(|(_, trigger)| trigger.source.target.as_deref() == Some(target))
            .map(|(id, _)| id.clone())
            .collect();
        found
    }

    /// Button actors whose firing source targets `target`, in slot
    /// order for deterministic multi-button scenarios.
    fn live_buttons_by_target(world: &PlayWorld, target: &str) -> Vec<qa_core::identity::ActorId> {
        let mut found: Vec<qa_core::identity::ActorId> = {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            borrowed
                .buttons
                .iter()
                .filter(|(_, button)| button.source.target.as_deref() == Some(target))
                .map(|(id, _)| id.clone())
                .collect()
        };
        found.sort_by(|left, right| {
            left.slot()
                .cmp(&right.slot())
                .then(left.generation().cmp(&right.generation()))
        });
        found
    }

    /// Door actors indexed under `targetname`.
    fn live_doors_by_targetname(world: &PlayWorld, targetname: &str) -> Vec<qa_core::identity::ActorId> {
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        let found: Vec<qa_core::identity::ActorId> = borrowed
            .by_targetname
            .get(targetname)
            .map(|matches| {
                matches
                    .iter()
                    .filter(|id| borrowed.doors.contains_key(id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        found
    }

    /// Item actor whose body sits at a map origin (exact `f32`: spawn
    /// origins parse from the same entity text).
    fn live_item_by_origin(world: &PlayWorld, origin: qa_core::math::Vec3) -> qa_core::identity::ActorId {
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        let found = borrowed
            .items
            .keys()
            .find(|id| {
                world
                    .server()
                    .simulation()
                    .body_state(id)
                    .is_some_and(|body| body.origin == origin)
            })
            .cloned()
            .unwrap_or_else(|| panic!("no live item at {origin:?}"));
        found
    }

    /// Q1-0246: e1m1 `trigger_once` fires its target through a live
    /// touch, then removes itself. The `*30 -> t11` chain flips the
    /// `START_OFF` style-33 toggle light on (`misc.qc:41-57`) and the
    /// `*18 -> t5` chain opens the closed `*17` door; each once
    /// schedules `SUB_Remove` 0.1 s out (`triggers.qc:168-171`).
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0246_trigger_once_fires_lights_then_removes() {
        use qa_world::movers::MoverPhase;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let light_feeders = live_triggers_by_target(&world, "t11");
        assert_eq!(light_feeders.len(), 1, "e1m1 fires t11 from one trigger_once");
        let light_once = light_feeders[0].clone();
        let door_feeders = live_triggers_by_target(&world, "t5");
        assert_eq!(door_feeders.len(), 1, "e1m1 fires t5 from one trigger_once");
        let door_once = door_feeders[0].clone();
        let doors = live_doors_by_targetname(&world, "t5");
        assert_eq!(doors.len(), 1, "t5 names the *17 door");
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(behaviors.borrow().light_styles.get(&33), Some(&'a'));
        }
        assert_eq!(
            world.server_mut().movers_mut().get(&doors[0]).unwrap().phase,
            MoverPhase::AtPos1,
            "t5 door starts closed"
        );
        let center = live_volume_center(&world, &light_once);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(
                behaviors.borrow().light_styles.get(&33),
                Some(&'m'),
                "live touch toggled the t11 light on"
            );
        }
        let center = live_volume_center(&world, &door_once);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        assert_eq!(
            world.server_mut().movers_mut().get(&doors[0]).unwrap().phase,
            MoverPhase::ToPos2,
            "live touch fired the t5 door open"
        );
        live_advance(&mut world, 0.5);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(
                !borrowed.triggers.contains_key(&light_once) && !borrowed.triggers.contains_key(&door_once),
                "fired trigger_onces removed themselves"
            );
        }
    }

    /// Q1-0245: e1m1 `trigger_multiple` fires through a live touch
    /// with the stock wait re-arm (`triggers.qc:30-67`). Touching `*51`
    /// prints its message and arms 5 s out; a touch while armed stays
    /// silent; after the wait it fires again. A second world pins the
    /// stock `SUB_UseTargets` killtarget order through `*54`
    /// (`subs.qc:210`: victims removed, targets never fired).
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0245_trigger_multiple_fires_and_rearms() {
        use super::super::simulation::native_q1_triggers::Q1TriggerKind;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let multiple = {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let matches = borrowed.by_targetname.get("t31").cloned().unwrap_or_default();
            assert_eq!(matches.len(), 1, "t31 names one trigger_multiple");
            matches[0].clone()
        };
        // Fire by touch: the multiple prints and arms.
        let center = live_volume_center(&world, &multiple);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        let armed_until = {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(
                borrowed
                    .centerprints
                    .iter()
                    .any(|print| print.text == "You can jump up here..."),
                "touched multiple printed its map message"
            );
            let armed = match borrowed.triggers.get(&multiple).map(|trigger| &trigger.kind) {
                Some(Q1TriggerKind::Multiple { armed_until, .. }) => *armed_until,
                other => panic!("t31 is a touch multiple, found {other:?}"),
            };
            assert!(armed > live_now(&world), "touched multiple armed its wait");
            armed
        };
        // A touch while armed stays silent (the player never left).
        let prints = world.q1_behaviors().expect("Q1 behaviors").borrow().centerprints.len();
        assert_eq!(prints, 1, "first touch printed once");
        live_tick(&mut world);
        assert_eq!(
            world.q1_behaviors().expect("Q1 behaviors").borrow().centerprints.len(),
            prints,
            "armed multiple ignores the touch"
        );
        // Past the wait, the standing player fires it again.
        let soak = armed_until - live_now(&world) + 0.5;
        live_advance(&mut world, soak);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(
                borrowed.centerprints.len(),
                prints + 1,
                "multiple re-fired after its wait"
            );
            assert_eq!(borrowed.centerprints[prints].text, "You can jump up here...");
        }
        // Stock killtarget order in a fresh world: `*54` carries both
        // `target` and `killtarget` t31, so touching it removes `*51`
        // and fires nothing — no message, no re-arm.
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let feeders = live_triggers_by_target(&world, "t31");
        assert_eq!(feeders.len(), 1, "e1m1 fires t31 from one trigger_once");
        let center = live_volume_center(&world, &feeders[0]);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(borrowed.centerprints.is_empty(), "killtarget stops the firing");
            assert!(!borrowed.by_targetname.contains_key("t31"), "killtarget removed t31");
        }
    }

    /// Q1-0250: e1m1 `trigger_secret` counts through a live touch
    /// (`triggers.qc:196-218`): `found_secrets` rises, the default
    /// message prints, and the secret removes itself.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0250_trigger_secret_counts() {
        use super::super::simulation::native_q1_triggers::Q1TriggerKind;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let secret = {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.total_secrets, 6, "e1m1 holds six secrets");
            assert_eq!(borrowed.found_secrets, 0);
            let found = borrowed
                .triggers
                .iter()
                .find(|(_, trigger)| matches!(trigger.kind, Q1TriggerKind::Multiple { secret: true, .. }))
                .map(|(id, _)| id.clone())
                .expect("e1m1 spawns secret triggers");
            found
        };
        let center = live_volume_center(&world, &secret);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.found_secrets, 1, "live touch counted the secret");
            assert!(
                borrowed
                    .centerprints
                    .iter()
                    .any(|print| print.text == "You found a secret area!"),
                "secret printed the stock default message"
            );
        }
        live_advance(&mut world, 0.5);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert!(!behaviors.borrow().triggers.contains_key(&secret));
        }
    }

    /// Q1-0242: e1m1 `trigger_counter` counts live button presses
    /// (`triggers.qc:222-270`): the three `t9` buttons count 3-2-1,
    /// printing the stock countdown, and the last press fires the
    /// `t10` door with "Sequence completed!". Each press taps the
    /// button and steps back: stock pushers stall while a body stands
    /// inside the swept volume, so the travel needs the player out.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0242_counter_counts_buttons_then_fires_door() {
        use super::super::simulation::native_q1_triggers::Q1TriggerKind;
        use qa_world::movers::MoverPhase;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let counter = {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let found: Vec<_> = behaviors
                .borrow()
                .triggers
                .iter()
                .filter(|(_, trigger)| matches!(trigger.kind, Q1TriggerKind::Counter { .. }))
                .map(|(id, _)| id.clone())
                .collect();
            assert_eq!(found.len(), 1, "e1m1 holds one counter");
            found[0].clone()
        };
        let buttons = live_buttons_by_target(&world, "t9");
        assert_eq!(buttons.len(), 3, "three e1m1 buttons feed the counter");
        let doors = live_doors_by_targetname(&world, "t10");
        assert_eq!(doors.len(), 1, "t10 names one door");
        let count = |world: &PlayWorld| {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let count = match borrowed.triggers.get(&counter).map(|trigger| &trigger.kind) {
                Some(Q1TriggerKind::Counter { count, .. }) => *count,
                other => panic!("t9 stays a counter, found {other:?}"),
            };
            count
        };
        assert_eq!(count(&world), 3, "t9 counter needs three presses");
        let away = live_player_spawn_eye(&world);
        // Each press taps its button (touch fires on the first sweep)
        // and steps back so the pusher travel completes.
        for (press, button) in buttons.iter().enumerate() {
            let center = live_volume_center(&world, button);
            live_place_player(&mut world, center);
            live_tick(&mut world);
            assert_eq!(
                world.server_mut().movers_mut().get(button).unwrap().phase,
                MoverPhase::ToPos2,
                "touch fired button {}",
                press + 1
            );
            live_place_player(&mut world, away);
            let mut pressed = false;
            for _ in 0..600 {
                live_tick(&mut world);
                if world.server_mut().movers_mut().get(button).unwrap().phase == MoverPhase::AtPos2 {
                    pressed = true;
                    break;
                }
            }
            assert!(pressed, "counter button {} pressed", press + 1);
            assert_eq!(count(&world), 2 - press as i32);
        }
        for text in ["Only 2 more to go...", "Only 1 more to go..."] {
            assert!(
                world
                    .q1_behaviors()
                    .expect("Q1 behaviors")
                    .borrow()
                    .centerprints
                    .iter()
                    .any(|print| print.text == text),
                "counter printed {text:?}"
            );
        }
        assert!(
            world
                .q1_behaviors()
                .expect("Q1 behaviors")
                .borrow()
                .centerprints
                .iter()
                .any(|print| print.text == "Sequence completed!"),
            "counter printed the stock completion"
        );
        assert_ne!(
            world.server_mut().movers_mut().get(&doors[0]).unwrap().phase,
            MoverPhase::AtPos1,
            "counter fired the t10 door"
        );
    }

    /// Q1-0208: e1m1 `func_button` runs the full stock press cycle
    /// through live ticks (`buttons.qc:36-60`): touch travels to
    /// pressed, arrival fires the `t1` door, and after the player steps
    /// away the button returns and rests.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0208_button_presses_fires_door_and_returns() {
        use qa_world::movers::MoverPhase;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        // Two e1m1 buttons fire t1; the `*4` one keeps the stock 1 s
        // wait, so it returns after firing.
        let buttons = live_buttons_by_target(&world, "t1");
        assert_eq!(buttons.len(), 2, "two e1m1 buttons fire t1");
        let button = buttons
            .iter()
            .find(|id| {
                let mover = world.server_mut().movers_mut().get(id).cloned().expect("button mover");
                mover.pos1 != mover.pos2
            })
            .cloned()
            .expect("a travelling t1 button");
        let doors = live_doors_by_targetname(&world, "t1");
        assert_eq!(doors.len(), 1, "t1 names one door");
        assert_eq!(
            world.server_mut().movers_mut().get(&button).unwrap().phase,
            MoverPhase::AtPos1
        );
        // Tap: one sweep fires the touch, then the player steps back
        // so the pusher travel completes (a body inside the swept
        // volume stalls stock pushers) and the button can return
        // (stock re-fires a button its toucher still holds).
        let center = live_volume_center(&world, &button);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        assert_eq!(
            world.server_mut().movers_mut().get(&button).unwrap().phase,
            MoverPhase::ToPos2,
            "touch fired the button toward pressed"
        );
        let center = live_player_spawn_eye(&world);
        live_place_player(&mut world, center);
        let mut pressed = false;
        for _ in 0..600 {
            live_tick(&mut world);
            if world.server_mut().movers_mut().get(&button).unwrap().phase == MoverPhase::AtPos2 {
                pressed = true;
                break;
            }
        }
        assert!(pressed, "tapped button arrived at pressed");
        assert_ne!(
            world.server_mut().movers_mut().get(&doors[0]).unwrap().phase,
            MoverPhase::AtPos1,
            "button arrival fired the t1 door"
        );
        let mut returned = false;
        for _ in 0..600 {
            live_tick(&mut world);
            if world.server_mut().movers_mut().get(&button).unwrap().phase == MoverPhase::AtPos1 {
                returned = true;
                break;
            }
        }
        assert!(returned, "button returned and rests after its wait");
    }

    /// Q1-0249: e3m7 `trigger_relay` forwards `use` down a live chain:
    /// the `*37` button fires relay `t30`, which fires relay `t29`,
    /// which opens the `t18` doors (`triggers.qc:179-194`).
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0249_relay_chain_opens_door() {
        use qa_world::movers::MoverPhase;

        let Some(mut world) = live_q1_world("maps/e3m7.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let buttons = live_buttons_by_target(&world, "t30");
        assert_eq!(buttons.len(), 1, "one e3m7 button feeds relay t30");
        let doors = live_doors_by_targetname(&world, "t18");
        assert_eq!(doors.len(), 2, "t18 names two doors");
        // Tap and step back: the touch fires on the first sweep, and
        // the travel needs the player out of the swept volume.
        let center = live_volume_center(&world, &buttons[0]);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        assert_eq!(
            world.server_mut().movers_mut().get(&buttons[0]).unwrap().phase,
            MoverPhase::ToPos2,
            "touch fired the relay feeder button"
        );
        let center = live_player_spawn_eye(&world);
        live_place_player(&mut world, center);
        let mut pressed = false;
        for _ in 0..600 {
            live_tick(&mut world);
            if world.server_mut().movers_mut().get(&buttons[0]).unwrap().phase == MoverPhase::AtPos2 {
                pressed = true;
                break;
            }
        }
        assert!(pressed, "relay feeder button pressed");
        for door in &doors {
            assert_ne!(
                world.server_mut().movers_mut().get(door).unwrap().phase,
                MoverPhase::AtPos1,
                "relay chain fired the t18 door"
            );
        }
    }

    /// Q1-0243: the `end` map `trigger_hurt` damages a live taker 10
    /// per touch, rests unmarked 1 s (`triggers.qc:538-570`), then
    /// re-arms and damages again while the player stands inside.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0243_hurt_damages_and_rearms() {
        use super::super::simulation::native_q1_triggers::Q1TriggerKind;

        let Some(mut world) = live_q1_world("maps/end.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let hurt = {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let found: Vec<_> = behaviors
                .borrow()
                .triggers
                .iter()
                .filter(|(_, trigger)| matches!(trigger.kind, Q1TriggerKind::Hurt { .. }))
                .map(|(id, _)| id.clone())
                .collect();
            assert_eq!(found.len(), 1, "end holds one hurt trigger");
            found[0].clone()
        };
        assert_eq!(live_player_health(&world), 100.0);
        let center = live_volume_center(&world, &hurt);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        assert_eq!(live_player_health(&world), 90.0, "hurt dealt its map 10 damage");
        assert!(
            !world.server_mut().triggers_mut().is_trigger(&hurt),
            "hurt rests unmarked after firing"
        );
        let mut rearmed = false;
        for _ in 0..600 {
            live_tick(&mut world);
            if live_player_health(&world) <= 80.0 {
                rearmed = true;
                break;
            }
        }
        assert!(rearmed, "hurt re-armed and damaged again within 10 s");
        assert_eq!(live_player_health(&world), 80.0);
    }

    /// Q1-0248: an e3m5 `trigger_push` sets the live player velocity
    /// along its movedir at ten times its speed (`triggers.qc:572-610`)
    /// and queues the movement force for the player step to mirror.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0248_push_sets_velocity() {
        use super::super::simulation::native_q1_triggers::Q1TriggerKind;

        let Some(mut world) = live_q1_world("maps/e3m5.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let (push, movedir, speed) = {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let found = borrowed
                .triggers
                .iter()
                .find_map(|(id, trigger)| match &trigger.kind {
                    Q1TriggerKind::Push { movedir, speed, .. } if *speed == 100.0 => {
                        Some((id.clone(), *movedir, *speed))
                    }
                    _ => None,
                })
                .expect("e3m5 spawns a speed-100 push trigger");
            found
        };
        assert_ne!(movedir, vec3(0.0, 0.0, 0.0), "scenario push parsed its angle");
        let player = world.player_actor().cloned().expect("player");
        let before = world
            .server()
            .simulation()
            .body_state(&player)
            .expect("player body")
            .origin;
        let center = live_volume_center(&world, &push);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        let body = world.server().simulation().body_state(&player).expect("player body");
        let shove = speed as f32 * 10.0;
        assert_eq!(
            body.velocity,
            vec3(movedir.x * shove, movedir.y * shove, movedir.z * shove)
        );
        assert_ne!(body.origin, before, "placement reached the push volume");
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let force = borrowed.player_forces.last().expect("push queued a force");
            assert_eq!(force.actor, player);
            assert_eq!(force.velocity, Some(body.velocity));
        }
    }

    /// Q1-0252 (+ Q1-0223, Q1-0165, Q1-0166): e1m1 `trigger_teleport`
    /// moves the live player to its `info_teleport_destination`
    /// (`triggers.qc:368-423`): the arrival origin lifts 27 units, the
    /// mangle snaps facing, velocity runs 300 along it, both fog
    /// flashes queue, the `teledeath` volume spawns — and a second body
    /// standing at the arrival takes the stock 50000 telefrag.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0252_teleport_moves_player_fogs_and_telefrags() {
        use qa_core::identity::ProviderId;
        use qa_core::math::angle_vectors;
        use qa_world::body::BodyState;
        use qa_world::combat::CombatState;

        use super::super::simulation::native_q1_triggers::Q1TriggerKind;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let feeders = live_triggers_by_target(&world, "t6");
        assert_eq!(feeders.len(), 1, "e1m1 fires t6 from one teleporter");
        let teleporter = feeders[0].clone();
        let (destination, mangle) = {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let matches = borrowed.by_targetname.get("t6").cloned().unwrap_or_default();
            assert_eq!(matches.len(), 1, "t6 names one destination");
            let record = borrowed
                .teleport_destinations
                .get(&matches[0])
                .cloned()
                .expect("t6 record");
            (record.origin, record.mangle)
        };
        // Map truth plus the stock 27-unit lift (`triggers.qc:425`).
        assert_eq!(destination, vec3(-32.0, 1800.0, -56.0 + 27.0));
        assert_eq!(mangle, vec3(0.0, 0.0, 0.0), "t6 has no angle key");
        // A second living body stands at the arrival: the telefrag
        // victim. Plain sim spawn plus stock 100 health.
        let player = world.player_actor().cloned().expect("player");
        let victim_bounds = world
            .server()
            .simulation()
            .body_state(&player)
            .expect("player body")
            .bounds;
        let victim = world
            .server_mut()
            .simulation_mut()
            .spawn(
                ProviderId::new("game", "q1"),
                "q1:proof-victim",
                Some(BodyState {
                    origin: destination,
                    angles: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    bounds: victim_bounds,
                    ground: None,
                }),
                None,
                Vec::new(),
            )
            .expect("victim spawns")
            .id()
            .clone();
        world
            .server_mut()
            .simulation_mut()
            .set_combat(&victim, CombatState::default())
            .unwrap();
        // Touch: the player departs for the arrival in one live tick.
        let departure = live_volume_center(&world, &teleporter);
        live_place_player(&mut world, departure);
        live_tick(&mut world);
        let tick_now = live_now(&world);
        let forward = angle_vectors(mangle).forward;
        let arrived = vec3(
            destination.x + forward.x * 32.0,
            destination.y + forward.y * 32.0,
            destination.z + forward.z * 32.0,
        );
        let body = world.server().simulation().body_state(&player).expect("player body");
        assert_eq!(body.origin, destination, "teleport moved the player to the arrival");
        assert_eq!(
            body.velocity,
            vec3(forward.x * 300.0, forward.y * 300.0, forward.z * 300.0),
            "teleport snapped velocity to 300 along the mangle"
        );
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(
                borrowed.teleport_fogs,
                vec![departure, arrived],
                "both fog flashes queued"
            );
            let force = borrowed.player_forces.last().expect("teleport queued a force");
            assert_eq!(force.actor, player);
            assert_eq!(force.origin, Some(destination));
            assert_eq!(force.angles, Some(mangle));
            assert_eq!(force.velocity, Some(body.velocity));
            assert_eq!(force.teleport_time_seconds, Some(tick_now + 0.7));
        }
        // The victim dies on the next sweep through the live teledeath
        // volume (the owner stays immune); the volume removes itself
        // after its stock 0.2 s.
        let center = live_player_spawn_eye(&world);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        assert_eq!(
            world
                .server()
                .simulation()
                .combat_state(&victim)
                .map(|combat| combat.health),
            Some(-99.0),
            "teledeath dealt its stock 50000, floored at the Q1 -99"
        );
        assert_eq!(
            live_player_health(&world),
            100.0,
            "teleport owner immune to its teledeath"
        );
        live_advance(&mut world, 0.5);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert!(
                behaviors
                    .borrow()
                    .triggers
                    .iter()
                    .all(|(_, trigger)| !matches!(trigger.kind, Q1TriggerKind::Teledeath { .. })),
                "teledeath removed itself after 0.2 s"
            );
        }
    }

    /// Changelevel exits in a live world: actor plus destination map
    /// stem, in spawn order.
    fn live_changelevel_exits(world: &PlayWorld) -> Vec<(qa_core::identity::ActorId, String)> {
        use super::super::simulation::native_q1_triggers::Q1TriggerKind;

        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        borrowed
            .triggers
            .iter()
            .filter_map(|(id, trigger)| match &trigger.kind {
                Q1TriggerKind::Changelevel { map, .. } => Some((id.clone(), map.clone())),
                _ => None,
            })
            .collect()
    }

    /// Q1-0241: e1m1 slipgate exit (`trigger_changelevel`, `client.qc:290`):
    /// the touch sets `nextmap`, nulls the touch, thinks
    /// `execute_changelevel` 0.1s later, and the entry freezes the live
    /// player at one of the four map cameras with the single-player 2s
    /// exit gate, CD track 3, and no travel yet.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0241_changelevel_touch_enters_intermission() {
        use qa_world::movement::types::{Q1UserCommand, UserCommand};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(behaviors.borrow().mapname, "e1m1");
        }
        let exits = live_changelevel_exits(&world);
        assert_eq!(exits.len(), 1, "e1m1 has one exit");
        assert_eq!(exits[0].1, "e1m2");
        let exit = exits[0].0.clone();
        let player = world.player_actor().cloned().expect("player");
        let at_exit = live_volume_center(&world, &exit);
        live_place_player(&mut world, at_exit);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.nextmap.as_deref(), Some("e1m2"));
            assert_eq!(borrowed.intermission.running, 0, "touch alone enters nothing");
            assert_eq!(borrowed.pending_travel, None);
        }
        assert!(!world.server_mut().triggers_mut().is_trigger(&exit));
        live_advance(&mut world, 0.2);
        let now = live_now(&world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.intermission.running, 1);
            assert!(
                (borrowed.intermission.exit_time_seconds - (now + 2.0)).abs() < 0.25,
                "single-player 2s gate: {} vs {now}",
                borrowed.intermission.exit_time_seconds
            );
            assert_eq!(borrowed.cd_tracks, vec![(3, 3)]);
            let spot = borrowed.intermission.spot.clone().expect("camera spot");
            assert!(borrowed.intermission_spots.contains(&spot));
            let body = world.server().simulation().body_state(&player).expect("player body");
            assert_eq!(body.origin, spot.origin, "entry moved the player to the camera");
            assert_eq!(body.angles, spot.mangle);
            assert!(!borrowed.solids.contains(&player), "entry unsolids the player");
        }
        assert!(
            !world
                .server()
                .simulation()
                .combat_state(&player)
                .unwrap()
                .can_take_damage,
            "entry drops takedamage"
        );
        // Frozen: a full forward step moves nothing (`client.qc:901`).
        let before = world
            .server()
            .simulation()
            .body_state(&player)
            .expect("player body")
            .origin;
        let (_, angles) = world.player_eye().expect("player eye");
        let command = UserCommand::Q1Netquake(Q1UserCommand {
            acknowledged_server_time_seconds: now,
            view_angles: angles,
            forward_move: 400.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        });
        world.step_player(command).unwrap();
        let after = world
            .server()
            .simulation()
            .body_state(&player)
            .expect("player body")
            .origin;
        assert_eq!(before, after, "intermission player step is frozen");
    }

    /// Q1-0241: `start` NO_INTERMISSION exits (`client.qc:315`): the
    /// e1m1 slipgate travels at once in single player — pending travel
    /// set, no intermission entered, no execute think armed.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0241_start_exit_skips_intermission() {
        let Some(mut world) = live_q1_world("maps/start.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let exits = live_changelevel_exits(&world);
        assert_eq!(exits.len(), 5, "start has five episode exits");
        let exit = exits
            .iter()
            .find(|(_, map)| map == "e1m1")
            .expect("start e1m1 exit")
            .0
            .clone();
        let at_exit = live_volume_center(&world, &exit);
        live_place_player(&mut world, at_exit);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.mapname, "start");
            assert_eq!(borrowed.nextmap.as_deref(), Some("e1m1"));
            assert_eq!(borrowed.pending_travel.as_deref(), Some("e1m1"));
            assert!(borrowed.changelevel_issued);
            assert_eq!(borrowed.intermission.running, 0);
            assert!(borrowed.thinks.is_empty());
        }
        assert!(world.server_mut().triggers_mut().is_trigger(&exit));
    }

    /// Q1-0187: `noexit` at a live exit (`client.qc:295`): 1 kills on
    /// e1m1, 2 kills outside `start` but spares the `start` exits.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0187_noexit_kills_at_the_exit() {
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let exits = live_changelevel_exits(&world);
        let exit = exits[0].0.clone();
        world.q1_behaviors().expect("Q1 behaviors").borrow_mut().noexit = 1;
        let at_exit = live_volume_center(&world, &exit);
        live_place_player(&mut world, at_exit);
        live_tick(&mut world);
        assert_eq!(live_player_health(&world), -99.0, "noexit 1 kills on e1m1");
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(behaviors.borrow().nextmap, None);
        }
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let exits = live_changelevel_exits(&world);
        let exit = exits[0].0.clone();
        world.q1_behaviors().expect("Q1 behaviors").borrow_mut().noexit = 2;
        let at_exit = live_volume_center(&world, &exit);
        live_place_player(&mut world, at_exit);
        live_tick(&mut world);
        assert_eq!(live_player_health(&world), -99.0, "noexit 2 kills outside start");
        let Some(mut world) = live_q1_world("maps/start.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let exits = live_changelevel_exits(&world);
        let exit = exits
            .iter()
            .find(|(_, map)| map == "e1m1")
            .expect("start e1m1 exit")
            .0
            .clone();
        world.q1_behaviors().expect("Q1 behaviors").borrow_mut().noexit = 2;
        let at_exit = live_volume_center(&world, &exit);
        live_place_player(&mut world, at_exit);
        live_tick(&mut world);
        assert_eq!(live_player_health(&world), 100.0, "noexit 2 spares start");
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(behaviors.borrow().pending_travel.as_deref(), Some("e1m1"));
        }
    }

    /// Q1-0216: e1m1 `info_intermission` cameras (`client.qc:23`): all
    /// four record in spawn order with map-exact origins and mangles,
    /// plus the `FindIntermission` start fallback.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0216_intermission_spots_recorded() {
        let Some(world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        let spots: Vec<(qa_core::math::Vec3, qa_core::math::Vec3)> = borrowed
            .intermission_spots
            .iter()
            .map(|spot| (spot.origin, spot.mangle))
            .collect();
        assert_eq!(
            spots,
            vec![
                (vec3(-112.0, 704.0, 56.0), vec3(20.0, 45.0, 0.0)),
                (vec3(-208.0, 2736.0, 192.0), vec3(20.0, 225.0, 0.0)),
                (vec3(240.0, 2664.0, 104.0), vec3(20.0, 120.0, 0.0)),
                (vec3(1376.0, 1936.0, 64.0), vec3(20.0, 135.0, 0.0)),
            ]
        );
        assert_eq!(borrowed.start_spots.len(), 1, "e1m1 has one player start");
        assert_eq!(borrowed.start_spots[0].origin, vec3(480.0, -352.0, 88.0));
    }

    /// Q1-0110: e1m1 `item_health` heals a live wounded player
    /// (`items.qc:150-204`): +25 to a 50-health player, the receipt
    /// prints, the box hides and unmarks, and single player arms no
    /// respawn.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0110_health_pickup_heals() {
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let health = live_item_by_origin(&world, vec3(1376.0, 808.0, -432.0));
        live_set_player_health(&mut world, 50.0);
        let center = live_volume_center(&world, &health);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        assert_eq!(live_player_health(&world), 75.0, "normal health heals 25");
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(borrowed.items.get(&health).is_some_and(|item| item.taken));
            assert!(
                borrowed
                    .sprints
                    .iter()
                    .any(|print| print.text == "You receive 25 health"),
                "pickup printed the stock receipt"
            );
            assert!(borrowed.thinks.is_empty(), "single player arms no respawn");
        }
        assert!(
            !world.server_mut().triggers_mut().is_trigger(&health),
            "taken box unmarked"
        );
    }

    /// Q1-0111: e1m1 megahealth pickup rots (`items.qc:206-233`): +100
    /// over the cap to 200, one point per second back to 100, the
    /// superhealth bit clears, and the taken box stays hidden in
    /// single player (respawn is deathmatch-only).
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0111_megahealth_pickup_rots() {
        use super::super::simulation::native_q1_spawns::IT_SUPERHEALTH;
        use super::super::simulation::native_q1_triggers::Q1ThinkKind;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let mega = live_item_by_origin(&world, vec3(944.0, 1008.0, -272.0));
        let center = live_volume_center(&world, &mega);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        let tick_now = live_now(&world);
        assert_eq!(live_player_health(&world), 200.0, "megahealth adds 100 over the cap");
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_ne!(borrowed.player_items & IT_SUPERHEALTH, 0);
            let rot = borrowed
                .thinks
                .iter()
                .find(|think| matches!(think.kind, Q1ThinkKind::MegaRot { .. }))
                .expect("megahealth armed its rot");
            assert_eq!(rot.due_seconds, tick_now + 5.0, "first rot 5 s out");
        }
        live_advance(&mut world, 6.0);
        assert_eq!(live_player_health(&world), 199.0, "rot ticks one point per second");
        // Rot re-arms one second out per point; frame granularity lags
        // each re-arm, so soak on state, not on the nominal 100 s.
        for _ in 0..40 {
            if live_player_health(&world) == 100.0 {
                break;
            }
            live_advance(&mut world, 5.0);
        }
        assert_eq!(live_player_health(&world), 100.0, "rot stops at the health cap");
        live_advance(&mut world, 2.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_items & IT_SUPERHEALTH, 0, "superhealth bit cleared");
            assert!(borrowed.items.get(&mega).is_some_and(|item| item.taken));
            assert!(
                borrowed
                    .thinks
                    .iter()
                    .all(|think| !matches!(think.kind, Q1ThinkKind::Regen)),
                "single player never respawns the box"
            );
        }
    }

    /// Q1-0101: e1m1 `item_armor1` pickup (`items.qc:238-285`): green
    /// armor grants 100 points at 0.3 absorption, sets `IT_ARMOR1`,
    /// prints, hides, and arms no single-player respawn.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0101_armor1_pickup() {
        use qa_world::combat::RegularArmor;

        use super::super::simulation::native_q1_spawns::IT_ARMOR1;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let armor = live_item_by_origin(&world, vec3(688.0, 480.0, 80.0));
        let center = live_volume_center(&world, &armor);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        let player = world.player_actor().cloned().expect("player");
        match &world
            .server()
            .simulation()
            .combat_state(&player)
            .expect("player combat")
            .armor
            .regular
        {
            RegularArmor::Q1 { points, absorption, .. } => {
                assert_eq!((*points, *absorption), (100.0, 0.3));
            }
            other => panic!("green armor grants Q1 armor, found {other:?}"),
        }
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_ne!(borrowed.player_items & IT_ARMOR1, 0);
            assert!(borrowed.sprints.iter().any(|print| print.text == "You got armor"));
            assert!(borrowed.items.get(&armor).is_some_and(|item| item.taken));
        }
    }

    /// Q1-0102: e1m1 `item_armor2` pickup: yellow armor grants 150
    /// points at 0.6 absorption and sets `IT_ARMOR2`.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0102_armor2_pickup() {
        use qa_world::combat::RegularArmor;

        use super::super::simulation::native_q1_spawns::IT_ARMOR2;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let armor = live_item_by_origin(&world, vec3(1312.0, 1048.0, -432.0));
        let center = live_volume_center(&world, &armor);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        let player = world.player_actor().cloned().expect("player");
        match &world
            .server()
            .simulation()
            .combat_state(&player)
            .expect("player combat")
            .armor
            .regular
        {
            RegularArmor::Q1 { points, absorption, .. } => {
                assert_eq!((*points, *absorption), (150.0, 0.6));
            }
            other => panic!("yellow armor grants Q1 armor, found {other:?}"),
        }
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_ne!(behaviors.borrow().player_items & IT_ARMOR2, 0);
        }
    }

    /// Q1-0103: e2m1 `item_armorInv` pickup: red armor grants 200
    /// points at 0.8 absorption and sets `IT_ARMOR3`.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0103_armorinv_pickup() {
        use qa_world::combat::RegularArmor;

        use super::super::simulation::native_q1_spawns::IT_ARMOR3;

        let Some(mut world) = live_q1_world("maps/e2m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let armor = live_item_by_origin(&world, vec3(1256.0, 664.0, -32.0));
        let center = live_volume_center(&world, &armor);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        let player = world.player_actor().cloned().expect("player");
        match &world
            .server()
            .simulation()
            .combat_state(&player)
            .expect("player combat")
            .armor
            .regular
        {
            RegularArmor::Q1 { points, absorption, .. } => {
                assert_eq!((*points, *absorption), (200.0, 0.8));
            }
            other => panic!("red armor grants Q1 armor, found {other:?}"),
        }
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_ne!(behaviors.borrow().player_items & IT_ARMOR3, 0);
        }
    }

    /// Q1-0115: e1m1 `item_shells` pickup (`items.qc:597-681`): a small
    /// box adds 20 shells, prints the receipt, and hides.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0115_shells_pickup() {
        use super::super::simulation::native_q1_items::Q1AmmoKind;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let shells = live_item_by_origin(&world, vec3(296.0, 2136.0, -192.0));
        let center = live_volume_center(&world, &shells);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            // 25-shell spawn loadout (`SetNewParms`) plus the 20-shell box.
            assert_eq!(borrowed.player_ammo.get(Q1AmmoKind::Shells), 45.0);
            assert!(borrowed.sprints.iter().any(|print| print.text == "You got the shells"));
            assert!(borrowed.items.get(&shells).is_some_and(|item| item.taken));
        }
    }

    /// Q1-0117: e1m1 `item_spikes` pickup: a small box adds 25 nails.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0117_spikes_pickup() {
        use super::super::simulation::native_q1_items::Q1AmmoKind;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let spikes = live_item_by_origin(&world, vec3(272.0, 2352.0, 64.0));
        let center = live_volume_center(&world, &spikes);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.get(Q1AmmoKind::Nails), 25.0);
            assert!(borrowed.sprints.iter().any(|print| print.text == "You got the nails"));
        }
    }

    /// Q1-0114: e1m3 `item_rockets` pickup: a small box adds 5 rockets
    /// (every e1m1 rockets box is deathmatch-only).
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0114_rockets_pickup() {
        use super::super::simulation::native_q1_items::Q1AmmoKind;

        let Some(mut world) = live_q1_world("maps/e1m3.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let rockets = live_item_by_origin(&world, vec3(-352.0, -1128.0, 48.0));
        let center = live_volume_center(&world, &rockets);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.get(Q1AmmoKind::Rockets), 5.0);
            assert!(borrowed.sprints.iter().any(|print| print.text == "You got the rockets"));
        }
    }

    /// Q1-0109: e2m6 `item_cells` pickup: a small box adds 6 cells.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0109_cells_pickup() {
        use super::super::simulation::native_q1_items::Q1AmmoKind;

        let Some(mut world) = live_q1_world("maps/e2m6.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let cells = live_item_by_origin(&world, vec3(1832.0, 608.0, -616.0));
        let center = live_volume_center(&world, &cells);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.get(Q1AmmoKind::Cells), 6.0);
            assert!(borrowed.sprints.iter().any(|print| print.text == "You got the cells"));
        }
    }

    /// Q1-0100: deathmatch item respawn (`items.qc:6-12`): a taken e1m1
    /// health box regenerates 20 s out and picks up again, an ammo box
    /// regenerates 30 s out, and a megahealth box rots to the cap,
    /// clears its bit, then regenerates — all through live ticks.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0100_deathmatch_respawn() {
        use super::super::simulation::native_q1_items::Q1AmmoKind;
        use super::super::simulation::native_q1_spawns::IT_SUPERHEALTH;
        use super::super::simulation::native_q1_triggers::Q1ThinkKind;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Deathmatch, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let away = live_player_spawn_eye(&world);
        // Health: take, regen 20 s out, take again.
        let health = live_item_by_origin(&world, vec3(1376.0, 808.0, -432.0));
        live_set_player_health(&mut world, 50.0);
        let center = live_volume_center(&world, &health);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        let taken_now = live_now(&world);
        assert_eq!(live_player_health(&world), 75.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(borrowed.items.get(&health).is_some_and(|item| item.taken));
            let regen = borrowed
                .thinks
                .iter()
                .find(|think| think.actor == health && matches!(think.kind, Q1ThinkKind::Regen))
                .expect("deathmatch armed the health regen");
            assert_eq!(regen.due_seconds, taken_now + 20.0, "health respawns 20 s out");
        }
        live_place_player(&mut world, away);
        live_advance(&mut world, 21.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert!(
                behaviors.borrow().items.get(&health).is_some_and(|item| !item.taken),
                "health box regenerated"
            );
        }
        assert!(
            world.server_mut().triggers_mut().is_trigger(&health),
            "regen re-marked the touch"
        );
        live_set_player_health(&mut world, 50.0);
        let center = live_volume_center(&world, &health);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        assert_eq!(live_player_health(&world), 75.0, "regenerated box picks up again");
        // Ammo: take, regen 30 s out.
        let shells = live_item_by_origin(&world, vec3(296.0, 2136.0, -192.0));
        let center = live_volume_center(&world, &shells);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        let taken_now = live_now(&world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            // 25-shell spawn loadout (`SetNewParms`) plus the 20-shell box.
            assert_eq!(borrowed.player_ammo.get(Q1AmmoKind::Shells), 45.0);
            let regen = borrowed
                .thinks
                .iter()
                .find(|think| think.actor == shells && matches!(think.kind, Q1ThinkKind::Regen))
                .expect("deathmatch armed the ammo regen");
            assert_eq!(regen.due_seconds, taken_now + 30.0, "ammo respawns 30 s out");
        }
        live_place_player(&mut world, away);
        live_advance(&mut world, 31.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert!(
                behaviors.borrow().items.get(&shells).is_some_and(|item| !item.taken),
                "ammo box regenerated"
            );
        }
        // Megahealth: take, rot to the cap, clear the bit, regen.
        let mega = live_item_by_origin(&world, vec3(944.0, 1008.0, -272.0));
        live_set_player_health(&mut world, 100.0);
        let center = live_volume_center(&world, &mega);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        assert_eq!(live_player_health(&world), 200.0);
        live_place_player(&mut world, away);
        for _ in 0..40 {
            if live_player_health(&world) == 100.0 {
                break;
            }
            live_advance(&mut world, 5.0);
        }
        assert_eq!(live_player_health(&world), 100.0, "megahealth rotted to the cap");
        live_advance(&mut world, 2.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(behaviors.borrow().player_items & IT_SUPERHEALTH, 0);
        }
        live_advance(&mut world, 21.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert!(
                behaviors.borrow().items.get(&mega).is_some_and(|item| !item.taken),
                "megahealth box regenerated after the rot"
            );
        }
    }

    /// Assert one Steel map presents draw batches at its spawn camera.
    /// Returns `None` when the corpus or map is unavailable (skip).
    fn steel_presentation_batches(product: &str, map: &str) -> Option<usize> {
        use qa_client::render::types::{RenderOperation, SourceTime, ViewTarget};
        use qa_client::view::perspective_projection;
        use qa_core::math::{angles_to_axis, vec3};

        let catalog = steel_catalog()?;
        let options = ApplicationOptions {
            product: product.to_string(),
            map: map.to_string(),
            ..ApplicationOptions::default()
        };
        let config = test_config(&options);
        let mut world = match load_play_world(&config, &catalog, &options, test_owner()) {
            Ok(world) => world,
            Err(error) => {
                require_live_data::<()>(&format!("{product} {map} load ({error})"), None);
                return None;
            }
        };
        let presentation = world.presentation().unwrap_or_else(|| {
            panic!(
                "{product} {map}: expected a scene presentation ({:?})",
                world.presentation_error()
            )
        });
        assert!(
            presentation.surface_count() > 0,
            "{product} {map}: expected world surfaces"
        );
        let (origin, angles) = presentation
            .spawn()
            .map_or((vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)), |spawn| {
                (spawn.origin, spawn.angles)
            });
        let mut presentation = world.take_presentation().unwrap();
        let camera = qa_client::view::SceneCamera {
            origin,
            axis: angles_to_axis(angles),
            viewport: qa_client::view::Rect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
            projection: perspective_projection(90.0, 90.0, 16384.0, 4.0).unwrap(),
            clip: qa_client::view::CameraClip::None,
        };
        let (view, _) = presentation
            .prepare_frame_view(
                camera,
                ViewTarget::Preview("steel".to_string()),
                SourceTime::Milliseconds(33.0),
            )
            .unwrap_or_else(|error| panic!("{product} {map}: frame view failed: {error}"));
        let batches: usize = view
            .operations
            .iter()
            .map(|operation| match operation {
                RenderOperation::Draw(batches) => batches.len(),
                // Static world geometry stays arena-resident under the
                // retained/VBO design; each retained batch is one draw.
                RenderOperation::RetainedDraw(draw) => draw.batches.len(),
                _ => 0,
            })
            .sum();
        assert!(batches > 0, "{product} {map}: expected draw batches, got none");
        eprintln!(
            "{product} {map}: {} draw batches ({} model entities, {} inline models, {} skipped models)",
            batches,
            presentation.entities().len(),
            presentation.inline_models().len(),
            presentation.skipped_models().len()
        );
        for skipped in presentation.skipped_models() {
            eprintln!(
                "{product} {map}: skipped model #{} {} {}: {}",
                skipped.index, skipped.classname, skipped.model, skipped.reason
            );
        }
        Some(batches)
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_steel_q3_presentation_prepares_draw_batches() {
        // Asserts internally; `None` means the helper already skipped.
        steel_presentation_batches("q3-baseq3", "maps/q3dm1.bsp");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_steel_q3_entities_submit_model_batches() {
        use qa_client::render::types::SourceTime;
        use qa_client::view::perspective_projection;
        use qa_core::math::{angles_to_axis, normalize3, sub3, vector_to_angles};

        let Some(catalog) = steel_catalog() else {
            return;
        };
        let options = ApplicationOptions {
            product: "q3-baseq3".to_string(),
            map: "maps/q3dm1.bsp".to_string(),
            ..ApplicationOptions::default()
        };
        let config = test_config(&options);
        let mut world = match load_play_world(&config, &catalog, &options, test_owner()) {
            Ok(world) => world,
            Err(error) => {
                require_live_data::<()>(&format!("q3-baseq3 maps/q3dm1.bsp load ({error})"), None);
                return;
            }
        };
        let presentation = world
            .take_presentation()
            .unwrap_or_else(|| panic!("expected a scene presentation ({:?})", world.presentation_error()));
        assert!(!presentation.entities().is_empty(), "q3dm1 has item entities");
        let target = presentation.entities()[0].transform.origin;
        let origin = qa_core::math::add3(target, qa_core::math::vec3(48.0, 0.0, 24.0));
        let camera = qa_client::view::SceneCamera {
            origin,
            axis: angles_to_axis(vector_to_angles(normalize3(sub3(target, origin)))),
            viewport: qa_client::view::Rect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
            projection: perspective_projection(90.0, 90.0, 16384.0, 4.0).unwrap(),
            clip: qa_client::view::CameraClip::None,
        };
        let batches = presentation
            .prepare_entity_batches(camera, SourceTime::Milliseconds(33.0))
            .expect("entity batches");
        assert!(!batches.is_empty(), "close-up entity submits model batches");
        eprintln!("q3dm1 close-up: {} entity batches", batches.len());
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_steel_q3_entity_batches_bind_loaded_skins() {
        use qa_client::render::types::{ImageSource, SourceTime, TextureBinding};
        use qa_client::view::perspective_projection;
        use qa_core::math::{add3, angles_to_axis, normalize3, sub3, vec3, vector_to_angles};

        let Some(catalog) = steel_catalog() else {
            return;
        };
        let options = ApplicationOptions {
            product: "q3-baseq3".to_string(),
            map: "maps/q3dm1.bsp".to_string(),
            ..ApplicationOptions::default()
        };
        let config = test_config(&options);
        let mut world = match load_play_world(&config, &catalog, &options, test_owner()) {
            Ok(world) => world,
            Err(error) => {
                require_live_data::<()>(&format!("q3-baseq3 maps/q3dm1.bsp load ({error})"), None);
                return;
            }
        };
        let presentation = world
            .take_presentation()
            .unwrap_or_else(|| panic!("expected a scene presentation ({:?})", world.presentation_error()));
        assert!(!presentation.entities().is_empty(), "q3dm1 has item entities");
        // Close-up cameras over the first entities; every submitted batch
        // counts by image source. White/missing fallbacks are generated;
        // resolved skins are mount-loaded resources.
        let mut resource = 0;
        let mut generated = 0;
        let mut other = 0;
        for entity in presentation.entities().iter().take(8) {
            let target = entity.transform.origin;
            let origin = add3(target, vec3(48.0, 0.0, 24.0));
            let camera = qa_client::view::SceneCamera {
                origin,
                axis: angles_to_axis(vector_to_angles(normalize3(sub3(target, origin)))),
                viewport: qa_client::view::Rect {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 64,
                },
                projection: perspective_projection(90.0, 90.0, 16384.0, 4.0).unwrap(),
                clip: qa_client::view::CameraClip::None,
            };
            let batches = presentation
                .prepare_entity_batches(camera, SourceTime::Milliseconds(33.0))
                .expect("entity batches");
            for batch in &batches {
                match &batch.texture {
                    TextureBinding::BindImage(image) => match &image.source {
                        ImageSource::Resource { requested_path } => {
                            resource += 1;
                            eprintln!("q3dm1 textured batch: {requested_path}");
                        }
                        ImageSource::Generated { name } => {
                            generated += 1;
                            eprintln!("q3dm1 fallback batch: {name}");
                        }
                    },
                    _ => other += 1,
                }
            }
        }
        eprintln!("q3dm1 skin bindings: {resource} resource, {generated} generated, {other} other");
        assert!(
            resource > 0,
            "expected entity batches bound to mount-loaded skins, got {resource} resource vs {generated} generated"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_steel_q1_presentation_prepares_draw_batches() {
        // Asserts internally; `None` means the helper already skipped.
        steel_presentation_batches("q1-classic-id1", "maps/e1m1.bsp");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_steel_q2_presentation_prepares_draw_batches() {
        // Asserts internally; `None` means the helper already skipped.
        steel_presentation_batches("q2-classic-baseq2", "maps/base1.bsp");
    }

    /// Live dogs on the map, in record order.
    fn live_dogs(world: &PlayWorld) -> Vec<qa_core::identity::ActorId> {
        use super::super::simulation::native_q1_monsters::Q1MonsterKind;
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        borrowed
            .monsters
            .iter()
            .filter(|(_, monster)| monster.kind == Q1MonsterKind::Dog)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Two dogs denning within earshot (< 500 units), if the map dens
    /// any together.
    fn live_den_pair(world: &PlayWorld) -> Option<(qa_core::identity::ActorId, qa_core::identity::ActorId)> {
        let dogs = live_dogs(world);
        dogs.iter().find_map(|first| {
            dogs.iter()
                .find(|second| {
                    *second != first && {
                        let a = live_dog_feet(world, first);
                        let b = live_dog_feet(world, second);
                        ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt() < 500.0
                    }
                })
                .map(|second| (first.clone(), second.clone()))
        })
    }

    /// Dog feet position (placement handle for facing offsets).
    fn live_dog_feet(world: &PlayWorld, dog: &qa_core::identity::ActorId) -> qa_core::math::Vec3 {
        let body = world.server().simulation().body_state(dog).expect("dog body");
        vec3(body.origin.x, body.origin.y, body.origin.z + body.bounds.min.z)
    }

    /// Dog facing yaw in degrees (live ideal yaw).
    fn live_dog_yaw(world: &PlayWorld, dog: &qa_core::identity::ActorId) -> f64 {
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        borrowed.monsters.get(dog).expect("dog record").ideal_yaw
    }

    /// Stand the player `dist` units along the dog's facing, feet to feet.
    fn live_place_player_before_dog(world: &mut PlayWorld, dog: &qa_core::identity::ActorId, dist: f32) {
        let feet = live_dog_feet(world, dog);
        let yaw = live_dog_yaw(world, dog).to_radians();
        live_place_player(
            world,
            vec3(
                feet.x + yaw.cos() as f32 * dist,
                feet.y + yaw.sin() as f32 * dist,
                feet.z,
            ),
        );
    }

    /// Wound something through the real `T_Damage` (weapons stand-in
    /// until the weapons slice fires it).
    fn live_damage(
        world: &mut PlayWorld,
        targ: &qa_core::identity::ActorId,
        attacker: Option<&qa_core::identity::ActorId>,
        damage: f64,
    ) {
        use super::super::simulation::native_q1_monsters::q1_t_damage;
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let (simulation, movers, triggers) = world.server_mut().simulation_movers_and_triggers_mut();
        q1_t_damage(
            &mut behaviors.borrow_mut(),
            simulation,
            movers,
            triggers,
            targ,
            attacker,
            attacker,
            damage,
        );
    }

    /// Fire one `use` at an actor (trigger stand-in).
    fn live_fire_use(
        world: &mut PlayWorld,
        target: &qa_core::identity::ActorId,
        activator: &qa_core::identity::ActorId,
    ) {
        use super::super::simulation::native_q1_triggers::q1_fire_use;
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let (simulation, movers, triggers) = world.server_mut().simulation_movers_and_triggers_mut();
        q1_fire_use(
            &mut behaviors.borrow_mut(),
            simulation,
            movers,
            triggers,
            target,
            Some(activator),
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0141_dog_spawn_stands_armed() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let dogs = live_dogs(&world);
        assert_eq!(dogs.len(), 8, "e1m1 spawns eight dogs");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(borrowed.total_monsters, 8);
        for dog in &dogs {
            let monster = borrowed.monsters.get(dog).expect("dog record");
            assert!(
                matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::DogStand, _)),
                "dog stands, got {:?}",
                monster.think
            );
            assert_eq!(monster.flags & 512, 512, "dropped dog stands on ground");
            assert_eq!(monster.flags & 32, 32, "start_go flags the monster bit");
            assert_eq!(monster.takedamage, 2, "start_go arms DAMAGE_AIM");
            assert_eq!(monster.view_ofs, vec3(0.0, 0.0, 25.0));
            assert!(monster.pausetime > 9999999.0, "targetless dogs stand down");
        }
        // Pass cost on a live map (8 standing dogs, relink included).
        drop(borrowed);
        let start = std::time::Instant::now();
        for _ in 0..120 {
            world.step_monsters();
        }
        let per_pass = start.elapsed().as_secs_f64() * 1000.0 / 120.0;
        eprintln!("live: step_monsters on e1m1 = {per_pass:.3} ms/pass (8 dogs)");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0134_dog_sight_hunts() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let dogs = live_dogs(&world);
        let player = world.player_actor().cloned().expect("player");
        live_place_player_before_dog(&mut world, &dogs[0], 200.0);
        let mut woke = false;
        for _ in 0..180 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            if behaviors
                .borrow()
                .monsters
                .get(&dogs[0])
                .and_then(|monster| monster.enemy.clone())
                == Some(player.clone())
            {
                woke = true;
                break;
            }
        }
        assert!(woke, "the dog sights the player at 200 units");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        let monster = borrowed.monsters.get(&dogs[0]).expect("dog record");
        assert!(
            matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::DogRun, _)),
            "sighted dogs hunt, got {:?}",
            monster.think
        );
        assert!(monster.attack_finished > live_now(&world), "HuntTarget holds missiles");
        assert_eq!(borrowed.sight_entity.as_ref(), Some(&dogs[0]));
        assert!(
            borrowed.sounds.iter().any(|sound| sound.sample == "dog/dsight.wav"),
            "sight barks"
        );
        assert_eq!(
            borrowed
                .monsters
                .get(&dogs[1])
                .and_then(|monster| monster.enemy.clone()),
            None,
            "far dogs stay asleep"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0141_dog_bite_wounds() {
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let dogs = live_dogs(&world);
        live_place_player_before_dog(&mut world, &dogs[0], 50.0);
        let mut bit = false;
        for _ in 0..300 {
            live_tick(&mut world);
            if live_player_health(&world) < 100.0 {
                bit = true;
                break;
            }
        }
        assert!(bit, "the dog closes 50 units and bites");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        assert!(
            behaviors
                .borrow()
                .sounds
                .iter()
                .any(|sound| sound.sample == "dog/dattack1.wav"),
            "the bite stroke sounds"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0141_dog_leaps_and_lands() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink, Q1MonsterTouch};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let dogs = live_dogs(&world);
        live_place_player_before_dog(&mut world, &dogs[0], 120.0);
        let mut leapt = false;
        let mut landed = false;
        for _ in 0..600 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&dogs[0]).expect("dog record");
            if matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::DogLeap, _))
                || monster.touch == Q1MonsterTouch::JumpTouch
            {
                leapt = true;
            }
            // Landing runs the dog on; a second leap windup (grounded
            // leap frame, touch not yet set) does not count.
            if leapt
                && matches!(
                    monster.think,
                    Q1MonsterThink::Frame(Q1MonsterSeq::DogRun, _) | Q1MonsterThink::Frame(Q1MonsterSeq::DogAttack, _)
                )
                && monster.flags & 512 == 512
                && monster.touch == Q1MonsterTouch::None
            {
                landed = true;
                break;
            }
        }
        assert!(leapt, "the dog leaps at jump distance");
        assert!(landed, "the leap lands back into the hunt");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        let monster = borrowed.monsters.get(&dogs[0]).expect("dog record");
        assert_eq!(monster.flags & 512, 512, "the leap lands");
        assert_eq!(monster.touch, Q1MonsterTouch::None, "landing clears the touch");
        assert!(
            matches!(
                monster.think,
                Q1MonsterThink::Frame(Q1MonsterSeq::DogRun, _) | Q1MonsterThink::Frame(Q1MonsterSeq::DogAttack, _)
            ),
            "landed dogs run on, got {:?}",
            monster.think
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0141_dog_pain_then_dies() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let dogs = live_dogs(&world);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &dogs[0], Some(&player), 5.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&dogs[0]).expect("dog record");
            assert!(
                matches!(
                    monster.think,
                    Q1MonsterThink::Frame(Q1MonsterSeq::DogPain, 0) | Q1MonsterThink::Frame(Q1MonsterSeq::DogPainB, 0)
                ),
                "wounds run pain, got {:?}",
                monster.think
            );
            assert!(borrowed.sounds.iter().any(|sound| sound.sample == "dog/dpain1.wav"));
        }
        live_damage(&mut world, &dogs[0], Some(&player), 30.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&dogs[0]).expect("dog record");
            assert!(monster.dead);
            assert!(
                matches!(
                    monster.think,
                    Q1MonsterThink::Frame(Q1MonsterSeq::DogDie, 0) | Q1MonsterThink::Frame(Q1MonsterSeq::DogDieB, 0)
                ),
                "death runs die, got {:?}",
                monster.think
            );
            assert_eq!(borrowed.killed_monsters, 1);
            assert!(borrowed.sounds.iter().any(|sound| sound.sample == "dog/ddeath.wav"));
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0131_dog_gib_bursts_and_settles() {
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let dogs = live_dogs(&world);
        let player = world.player_actor().cloned().expect("player");
        let before = world
            .server()
            .simulation()
            .body_state(&dogs[0])
            .expect("dog body")
            .origin;
        live_damage(&mut world, &dogs[0], Some(&player), 1000.0);
        // The head drops 24 units the instant it bursts.
        let burst = world
            .server()
            .simulation()
            .body_state(&dogs[0])
            .expect("dog body")
            .origin;
        assert_eq!(burst.z, before.z - 24.0);
        live_tick(&mut world);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        assert_eq!(behaviors.borrow().gibs.len(), 4, "three chunks plus the head");
        assert!(
            behaviors.borrow().pending_gibs.is_empty(),
            "the pass spawns queued chunks"
        );
        drop(behaviors);
        // Chunks toss (origins move under gravity), then remove on
        // schedule; the head stays down.
        live_advance(&mut world, 25.0);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(borrowed.gibs.len(), 1, "chunks removed, head kept");
        assert!(borrowed.gibs.contains_key(&dogs[0]));
        assert_eq!(borrowed.gibs.get(&dogs[0]).expect("head").remove_at, None);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0136_dog_infighting() {
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let dogs = live_dogs(&world);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &dogs[1], Some(&dogs[0]), 5.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(
                behaviors
                    .borrow()
                    .monsters
                    .get(&dogs[1])
                    .and_then(|monster| monster.enemy.clone()),
                None,
                "same-class dogs stay friendly"
            );
        }
        live_damage(&mut world, &dogs[1], Some(&player), 5.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(
                behaviors
                    .borrow()
                    .monsters
                    .get(&dogs[1])
                    .and_then(|monster| monster.enemy.clone()),
                Some(player),
                "wounds turn dogs on outsiders"
            );
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0137_dog_ignores_hidden_player() {
        use super::super::simulation::native_q1_monsters::{
            Q1MonsterSeq, Q1MonsterThink, Q1_FLAG_NOTARGET, Q1_IT_INVISIBILITY,
        };

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let dogs = live_dogs(&world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            behaviors.borrow_mut().player_items |= Q1_IT_INVISIBILITY;
        }
        live_place_player_before_dog(&mut world, &dogs[0], 200.0);
        live_advance(&mut world, 1.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&dogs[0]).expect("dog record");
            assert_eq!(monster.enemy, None, "invisible players never wake dogs");
            assert!(matches!(
                monster.think,
                Q1MonsterThink::Frame(Q1MonsterSeq::DogStand, _)
            ));
        }
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let mut borrowed = behaviors.borrow_mut();
            borrowed.player_items &= !Q1_IT_INVISIBILITY;
            borrowed.player_flags |= Q1_FLAG_NOTARGET;
        }
        live_advance(&mut world, 1.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(
                borrowed
                    .monsters
                    .get(&dogs[0])
                    .and_then(|monster| monster.enemy.clone()),
                None,
                "notarget players never wake dogs"
            );
        }
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            behaviors.borrow_mut().player_flags &= !Q1_FLAG_NOTARGET;
        }
        live_advance(&mut world, 1.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let player = world.player_actor().cloned().expect("player");
            assert_eq!(
                behaviors
                    .borrow()
                    .monsters
                    .get(&dogs[0])
                    .and_then(|monster| monster.enemy.clone()),
                Some(player),
                "visible players wake dogs"
            );
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0133_dog_use_wakes() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let dogs = live_dogs(&world);
        let player = world.player_actor().cloned().expect("player");
        live_fire_use(&mut world, &dogs[0], &player);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&dogs[0]).expect("dog record");
            assert_eq!(monster.enemy.as_ref(), Some(&player));
            assert_eq!(monster.think, Q1MonsterThink::FoundTarget);
        }
        live_advance(&mut world, 0.5);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&dogs[0]).expect("dog record");
            assert!(
                matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::DogRun, _)),
                "used dogs hunt, got {:?}",
                monster.think
            );
            assert!(borrowed.sounds.iter().any(|sound| sound.sample == "dog/dsight.wav"));
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0132_kill_counting() {
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(behaviors.borrow().total_monsters, 8);
            assert_eq!(behaviors.borrow().killed_monsters, 0);
        }
        let dogs = live_dogs(&world);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &dogs[0], Some(&player), 30.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(behaviors.borrow().killed_monsters, 1);
        }
        live_damage(&mut world, &dogs[1], Some(&player), 30.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(behaviors.borrow().killed_monsters, 2);
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0134_dog_patrol_walks_corners() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e2m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let dogs = live_dogs(&world);
        let patrol = dogs.iter().find(|dog| {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            borrowed
                .monsters
                .get(dog)
                .is_some_and(|monster| monster.movetarget.is_some())
        });
        let Some(patrol) = patrol.cloned() else {
            panic!("e2m1 should route a dog through corners");
        };
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&patrol).expect("dog record");
            assert!(
                matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::DogWalk, _)),
                "targeted dogs walk out, got {:?}",
                monster.think
            );
        }
        let start = live_dog_feet(&world, &patrol);
        let first = {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            borrowed
                .monsters
                .get(&patrol)
                .and_then(|monster| monster.movetarget.clone())
                .expect("first corner")
        };
        let mut advanced = false;
        for _ in 0..900 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            if behaviors
                .borrow()
                .monsters
                .get(&patrol)
                .and_then(|monster| monster.movetarget.clone())
                != Some(first.clone())
            {
                advanced = true;
                break;
            }
        }
        assert!(advanced, "the patrol reaches its corner and turns onward");
        let end = live_dog_feet(&world, &patrol);
        let moved = ((end.x - start.x).powi(2) + (end.y - start.y).powi(2)).sqrt();
        assert!(moved > 10.0, "the patrol travels, moved {moved}");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0134_dog_sight_wakes_pack() {
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        // The two mid-map dogs den together (~210 units apart).
        let Some((first, second)) = live_den_pair(&world) else {
            panic!("e1m1 should den two dogs together");
        };
        let player = world.player_actor().cloned().expect("player");
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_ne!(
                behaviors.borrow().monsters.get(&second).expect("packmate").spawnflags & 3,
                0,
                "the den pair starts ambush-flagged"
            );
        }
        // Ambush dogs ignore the shared sighting and wait for a real
        // look: wound the denmate twice, the packmate sleeps through.
        for _ in 0..2 {
            live_damage(&mut world, &first, Some(&player), 1.0);
            for _ in 0..30 {
                live_tick(&mut world);
            }
        }
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(
                behaviors
                    .borrow()
                    .monsters
                    .get(&second)
                    .and_then(|monster| monster.enemy.clone()),
                None,
                "ambush dogs ignore the shared sighting"
            );
        }
        // The wake phase needs the denmate back at the den: the ignore
        // phase above let it run off hunting, out of the packmate's
        // sight, so a fresh world dens the pair together again. The
        // ambush bits clear before the first wound, arming the
        // shortcut branch below.
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let Some((first, second)) = live_den_pair(&world) else {
            panic!("e1m1 should den two dogs together");
        };
        let player = world.player_actor().cloned().expect("player");
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            behaviors
                .borrow_mut()
                .monsters
                .get_mut(&second)
                .expect("packmate")
                .spawnflags &= !3;
        }
        // Stock publishes the 0.1 s sighting once, for a new attacker
        // only, so the single wound must land inside the packmate's
        // next think: wait until its 0.1 s grid comes due, then wound
        // (the denmate never leaves the den).
        for _ in 0..30 {
            let due = {
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                let nextthink = behaviors.borrow().monsters.get(&second).expect("packmate").nextthink;
                nextthink - live_now(&world) <= 0.09
            };
            if due {
                break;
            }
            live_tick(&mut world);
        }
        live_damage(&mut world, &first, Some(&player), 1.0);
        let mut woke = false;
        for _ in 0..12 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            if behaviors
                .borrow()
                .monsters
                .get(&second)
                .and_then(|monster| monster.enemy.clone())
                == Some(player.clone())
            {
                woke = true;
                break;
            }
        }
        assert!(woke, "a sighted packmate wakes its den");
    }
}
