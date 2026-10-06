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

use qa_bots::scene::{
    PointContentsQuery as ScenePointContentsQuery, PointContentsResult as ScenePointContentsResult,
    Q1MoveRule as SceneQ1MoveRule, QueryTarget as SceneQueryTarget, TracePolicy as SceneTracePolicy,
};
use qa_client::audio::SoundFamily;
use qa_content::bsp::{parse_q1_entities, read_q1_bsp, Q1BspOptions};
use qa_content::bsp2::read_q2_bsp;
use qa_content::bsp3::{parse_q3_bsp, parse_q3_entities};
use qa_content::catalog::InstalledCatalog;
use qa_content::mounts::MountedContent;
use qa_content::{classify_bsp, BspKind};
use qa_core::math::vec3;
use qa_core::numeric::Q1_DONOR_PROFILE;
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
use super::simulation::native_q1_travel::{q1_decode_level_parms, q1_set_change_parms, Q1TravelCarry};
use super::simulation::native_q1_triggers::{
    build_q1_button, build_q1_trigger, q1_is_brush_trigger, q1_is_use_point, q1_note_intermission, q1_note_light,
    q1_note_spawn_spot, q1_note_start_spot, q1_note_targetname, q1_note_teleport_destination, q1_note_use_point,
    q1_note_worldspawn, q1_registered_version, register_q1_trigger_spawns,
};
use super::simulation::native_q1_weapons::{
    q1_grant_spawn_loadout, q1_sample_water_level, q1_sample_water_type, q1_weapon_pass, Q1SpawnParms,
};
use super::windowed_scene::{build_presentation, open_product_mounts, select_q1_spawn, select_spawn, PlayPresentation};
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
    launch: PlayWorldLaunch,
}

/// Launch context retained for Q1 level travel (`Host_Changelevel_f`,
/// `host_cmd.c:311`): the transition reloads through [`load_play_world`]
/// with the destination map, so the world keeps what the load needs.
#[derive(Debug, Clone)]
pub struct PlayWorldLaunch {
    /// Resolved runtime configuration.
    pub config: StartupConfig,
    /// Installed content catalog.
    pub catalog: InstalledCatalog,
    /// Launch options (the transition rewrites `map` and `skill`).
    pub options: ApplicationOptions,
    /// Renderer resource owner for the fresh presentation.
    pub owner: qa_client::render::types::ResourceOwner,
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

    /// Raw Q1 contents at a point (the `V_SetContentsColor` input), or
    /// `None` without Q1 clip. Feeds the underwater view shift.
    #[must_use]
    pub fn q1_eye_contents(&self, eye: qa_core::math::Vec3) -> Option<i32> {
        let Some(PlayerClip::Q1(scene)) = self.clip.as_ref() else {
            return None;
        };
        match scene.point_contents(&ScenePointContentsQuery {
            point: eye,
            target: SceneQueryTarget::World,
            policy: SceneTracePolicy::Q1 {
                move_rule: SceneQ1MoveRule::Normal,
                hull: None,
            },
            numeric: Q1_DONOR_PROFILE,
            pass_actor: None,
        }) {
            Ok(ScenePointContentsResult::Q1 { contents }) => Some(contents),
            _ => None,
        }
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
        // the entry move unsolids the player and snaps the view, no step
        // runs until the exit travels (stock `MOVETYPE_NONE`), and the
        // live buttons latch every frame for the exit poll
        // (`IntermissionThink`, `client.qc:242`).
        if let Some(behaviors) = self.q1_behaviors.as_ref() {
            if behaviors.borrow().intermission.running != 0 {
                behaviors.borrow_mut().intermission.buttons = command.buttons() != 0;
                return Ok(());
            }
        }
        // Stock dying freeze (`PlayerPreThink`, `client.qc:921`): dead
        // players own no input (corpses toss, respawns snap).
        if self.q1_behaviors.as_ref().is_some_and(|behaviors| {
            behaviors.borrow().player_state.deadflag != super::simulation::native_q1_weapons::Q1_DEAD_NO
        }) {
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
    /// by design: failed traces block, and dead players run the
    /// death think instead of the weapon frame (`client.qc:921`).
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
        let water_type = q1_sample_water_type(scene, server.simulation(), &player);
        {
            let mut borrowed = behaviors.borrow_mut();
            borrowed.player_state.water_level = water;
            borrowed.player_state.water_type = water_type;
        }
        q1_weapon_pass(
            server,
            &mut behaviors.borrow_mut(),
            scene,
            &player,
            view_angles,
            buttons,
            impulse,
        );
        // Respawn view snap (`PutClientInServer` `fixangle`).
        let respawn_angles = behaviors.borrow_mut().player_state.respawn_angles.take();
        if let Some(angles) = respawn_angles {
            if let Some(PlayerBody::Q1(body)) = self.player.as_mut() {
                body.view_angles = angles;
            }
        }
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

    /// Run one pending Q1 level transition (`Host_Changelevel_f`,
    /// `host_cmd.c:311`): take the completed `GotoNextMap` destination,
    /// capture the leaver's spawn parms (`SV_SaveSpawnparms`,
    /// `sv_main.c:1015`), consume a pending `trigger_setskill` value,
    /// and load the destination map with the carried payload. Returns
    /// `None` without Q1 behaviors or without pending travel; the take
    /// is at-most-once, so a failed load logs once and the old world
    /// keeps running (stock `Host_Error`s the session instead).
    pub fn take_pending_travel(&mut self) -> Result<Option<PlayWorld>, PlayWorldError> {
        let Some(behaviors) = self.q1_behaviors.clone() else {
            return Ok(None);
        };
        let taken = {
            let mut borrowed = behaviors.borrow_mut();
            let destination = borrowed.pending_travel.take();
            let serverflags = borrowed.serverflags;
            let skill_override = borrowed.skill_override.take();
            (destination, serverflags, skill_override)
        };
        let (Some(destination), serverflags, skill_override) = taken else {
            return Ok(None);
        };
        let parms = match self.player_actor().cloned() {
            Some(player) => {
                let mut borrowed = behaviors.borrow_mut();
                q1_set_change_parms(&mut borrowed, self.server.simulation_mut(), &player)
            }
            None => Q1SpawnParms::default(),
        };
        // Flagged returns to `start` shed every carried thing
        // (`DecodeLevelParms`, `client.qc:79-83`).
        let parms = if destination == "start" && serverflags != 0 {
            Q1SpawnParms::default()
        } else {
            parms
        };
        let mut options = self.launch.options.clone();
        options.map = format!("maps/{destination}.bsp");
        if let Some(skill) = skill_override {
            options.skill = skill.parse::<f64>().unwrap_or(0.0).clamp(0.0, 3.0) as u8;
        }
        let next = load_play_world_inner(
            &self.launch.config,
            &self.launch.catalog,
            &options,
            self.launch.owner.clone(),
            Some(Q1TravelCarry { parms, serverflags }),
        )?;
        Ok(Some(next))
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
                    if classname == "info_player_start"
                        || classname == "info_player_start2"
                        || classname == "info_player_coop"
                        || classname == "info_player_deathmatch"
                        || classname == "testplayerstart"
                    {
                        q1_note_spawn_spot(&mut behaviors, &fields);
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
    load_play_world_inner(config, catalog, options, owner, None)
}

/// Load the selected map, optionally carrying a transition payload: a
/// carried run spawns flagged (`serverflags` set, `start2` preferred)
/// and decodes the carried inventory instead of granting the fresh
/// spawn loadout.
fn load_play_world_inner(
    config: &StartupConfig,
    catalog: &InstalledCatalog,
    options: &ApplicationOptions,
    owner: qa_client::render::types::ResourceOwner,
    carry: Option<Q1TravelCarry>,
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
    let spawn = if kind == BspKind::Q1 {
        select_q1_spawn(&entities, carry.as_ref().map_or(0, |carried| carried.serverflags))
    } else {
        select_spawn(&entities, kind)
    };
    let player = match spawn {
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
        if let Some(carried) = carry.as_ref() {
            // Carried arrival (`PutClientInServer` over `DecodeLevelParms`,
            // `client.qc:479`): the flags persist and the captured
            // inventory replaces the fresh grant.
            behaviors.serverflags = carried.serverflags;
            let actor = PlayerBody::actor(player).clone();
            q1_decode_level_parms(&mut behaviors, server.simulation_mut(), &actor, &carried.parms);
        } else {
            // Stock spawn loadout (`PutClientInServer` over fresh parms):
            // axe and shotgun with 25 shells, shotgun in hand.
            q1_grant_spawn_loadout(&mut behaviors);
        }
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
    let travel_owner = owner.clone();
    let (presentation, presentation_error) =
        match build_presentation(mounts, &content, &options.map, &bytes, &entities, owner) {
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
        launch: PlayWorldLaunch {
            config: config.clone(),
            catalog: catalog.clone(),
            options: options.clone(),
            owner: travel_owner,
        },
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

    /// Latch live button state through the frozen player step (the
    /// `PlayerPreThink` seam, `client.qc:901`): 0 releases, nonzero
    /// presses.
    fn live_press_buttons(world: &mut PlayWorld, buttons: i32) {
        use qa_world::movement::types::{Q1UserCommand, UserCommand};

        let (_, angles) = world.player_eye().expect("player eye");
        let command = UserCommand::Q1Netquake(Q1UserCommand {
            acknowledged_server_time_seconds: live_now(world),
            view_angles: angles,
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons,
            impulse: 0,
        });
        world.step_player(command).unwrap();
    }

    /// Touch a live exit and run the 0.1s execute think: returns with
    /// the world entered in the intermission and the buttons released.
    fn live_enter_intermission(world: &mut PlayWorld, exit: &qa_core::identity::ActorId) {
        let at_exit = live_volume_center(world, exit);
        live_place_player(world, at_exit);
        live_tick(world);
        live_advance(world, 0.2);
        live_press_buttons(world, 0);
        assert_eq!(
            world
                .q1_behaviors()
                .expect("Q1 behaviors")
                .borrow()
                .intermission
                .running,
            1,
            "exit entered the intermission"
        );
    }

    /// Advance past the live intermission exit gate with the buttons
    /// released (a held press would exit at the gate, like stock).
    fn live_pass_exit_gate(world: &mut PlayWorld) {
        let wait = {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            borrowed.intermission.exit_time_seconds - live_now(world) + 0.1
        };
        live_advance(world, wait.max(0.1));
    }

    /// Independent oracle for the stock finale scrolls (`client.qc:172-226`).
    fn live_expected_finale(which: &str) -> &'static str {
        match which {
            "e1-shareware" => "As the corpse of the monstrous entity\nChthon sinks back into the lava whence\nit rose, you grip the Rune of Earth\nMagic tightly. Now that you have\nconquered the Dimension of the Doomed,\nrealm of Earth Magic, you are ready to\ncomplete your task in the other three\nhaunted lands of Quake. Or are you? If\nyou don't register Quake, you'll never\nknow what awaits you in the Realm of\nBlack Magic, the Netherworld, and the\nElder World!",
            "e1" => "As the corpse of the monstrous entity\nChthon sinks back into the lava whence\nit rose, you grip the Rune of Earth\nMagic tightly. Now that you have\nconquered the Dimension of the Doomed,\nrealm of Earth Magic, you are ready to\ncomplete your task. A Rune of magic\npower lies at the end of each haunted\nland of Quake. Go forth, seek the\ntotality of the four Runes!",
            "e2" => "The Rune of Black Magic throbs evilly in\nyour hand and whispers dark thoughts\ninto your brain. You learn the inmost\nlore of the Hell-Mother; Shub-Niggurath!\nYou now know that she is behind all the\nterrible plotting which has led to so\nmuch death and horror. But she is not\ninviolate! Armed with this Rune, you\nrealize that once all four Runes are\ncombined, the gate to Shub-Niggurath's\nPit will open, and you can face the\nWitch-Goddess herself in her frightful\notherworld cathedral.",
            "e3" => "The charred viscera of diabolic horrors\nbubble viscously as you seize the Rune\nof Hell Magic. Its heat scorches your\nhand, and its terrible secrets blight\nyour mind. Gathering the shreds of your\ncourage, you shake the devil's shackles\nfrom your soul, and become ever more\nhard and determined to destroy the\nhideous creatures whose mere existence\nthreatens the souls and psyches of all\nthe population of Earth.",
            "e4" => "Despite the awful might of the Elder\nWorld, you have achieved the Rune of\nElder Magic, capstone of all types of\narcane wisdom. Beyond good and evil,\nbeyond life and death, the Rune\npulsates, heavy with import. Patient and\npotent, the Elder Being Shub-Niggurath\nweaves her dire plans to clear off all\nlife from the Earth, and bring her own\nfoul offspring to our world! For all the\ndwellers in these nightmare dimensions\nare her descendants! Once all Runes of\nmagic power are united, the energy\nbehind them will blast open the Gateway\nto Shub-Niggurath, and you can travel\nthere to foil the Hell-Mother's plots\nin person.",
            "runes" => "Now, you have all four Runes. You sense\ntremendous invisible forces moving to\nunseal ancient barriers. Shub-Niggurath\nhad hoped to use the Runes Herself to\nclear off the Earth, but now instead,\nyou will use them to enter her home and\nconfront her as an avatar of avenging\nEarth-life. If you defeat her, you will\nbe remembered forever as the savior of\nthe planet. If she conquers, it will be\nas if you had never been born.",
            _ => panic!("unknown finale {which}"),
        }
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

    /// Q1-0257: e1m1 intermission exit (`IntermissionThink`,
    /// `client.qc:242`): a gated press does nothing, a press past the
    /// 2s gate travels to e1m2, the tally mirrors the live counters,
    /// and deathmatch skips every text with its 5s gate.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0257_intermission_buttons_exit_to_travel() {
        use super::super::simulation::native_q1_triggers::q1_intermission_stats;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let exits = live_changelevel_exits(&world);
        let exit = exits[0].0.clone();
        live_enter_intermission(&mut world, &exit);
        // Gated press: nothing travels.
        live_press_buttons(&mut world, 1);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.pending_travel, None);
            assert_eq!(borrowed.intermission.running, 1);
        }
        live_press_buttons(&mut world, 0);
        live_pass_exit_gate(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(behaviors.borrow().pending_travel, None, "released buttons wait");
        }
        live_press_buttons(&mut world, 1);
        live_tick(&mut world);
        let now = live_now(&world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.intermission.running, 2);
            assert_eq!(borrowed.pending_travel.as_deref(), Some("e1m2"));
            assert_eq!(borrowed.finale_text, None, "plain level ends show no scroll");
            let stats = q1_intermission_stats(&borrowed, now);
            assert_eq!(stats.killed_monsters, borrowed.killed_monsters);
            assert_eq!(stats.total_monsters, borrowed.total_monsters);
            assert_eq!(stats.found_secrets, borrowed.found_secrets);
            assert_eq!(stats.total_secrets, borrowed.total_secrets);
            assert_eq!(stats.time_seconds, now);
            assert!(stats.total_monsters > 0, "e1m1 tallies real monsters");
            assert!(stats.total_secrets > 0, "e1m1 tallies real secrets");
        }
        // Deathmatch: the 5s gate, then travel with no text.
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Deathmatch, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let exits = live_changelevel_exits(&world);
        let exit = exits[0].0.clone();
        live_enter_intermission(&mut world, &exit);
        live_press_buttons(&mut world, 0);
        live_pass_exit_gate(&mut world);
        live_press_buttons(&mut world, 1);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.intermission.running, 1, "DM never counts texts");
            assert_eq!(borrowed.pending_travel.as_deref(), Some("e1m2"));
            assert_eq!(borrowed.finale_text, None);
        }
    }

    /// Q1-0256: e1m7 episode finale (`ExitIntermission`, `client.qc:162`):
    /// the first exit press queues the registered episode-1 scroll with
    /// CD track 2 and no travel; the second press travels to `start`.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0256_e1_finale_registered() {
        let Some(mut world) = live_q1_world("maps/e1m7.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert!(behaviors.borrow().registered, "steel corpus is registered");
        }
        let exits = live_changelevel_exits(&world);
        assert_eq!(exits.len(), 1, "e1m7 has one exit");
        assert_eq!(exits[0].1, "start");
        let exit = exits[0].0.clone();
        live_enter_intermission(&mut world, &exit);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(behaviors.borrow().nextmap.as_deref(), Some("start"));
        }
        live_press_buttons(&mut world, 0);
        live_pass_exit_gate(&mut world);
        live_press_buttons(&mut world, 1);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.intermission.running, 2);
            assert_eq!(borrowed.finale_text.as_deref(), Some(live_expected_finale("e1")));
            assert_eq!(borrowed.cd_tracks, vec![(3, 3), (2, 3)]);
            assert_eq!(borrowed.pending_travel, None, "scroll shows before travel");
        }
        live_press_buttons(&mut world, 0);
        live_pass_exit_gate(&mut world);
        live_press_buttons(&mut world, 1);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.intermission.running, 3);
            assert_eq!(borrowed.pending_travel.as_deref(), Some("start"));
        }
    }

    /// Q1-0256: episode 2-4 finales (`ExitIntermission`, `client.qc:182-212`):
    /// each end map scrolls its exact stock text with CD track 2.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0256_episode_finales_e2_e3_e4() {
        for (map, which) in [
            ("maps/e2m6.bsp", "e2"),
            ("maps/e3m6.bsp", "e3"),
            ("maps/e4m7.bsp", "e4"),
        ] {
            let Some(mut world) = live_q1_world(map, GameMode::Singleplayer, 1) else {
                return;
            };
            live_silence_door_fields(&mut world);
            let exits = live_changelevel_exits(&world);
            assert_eq!(exits.len(), 1, "{map} has one exit");
            let exit = exits[0].0.clone();
            live_enter_intermission(&mut world, &exit);
            live_press_buttons(&mut world, 0);
            live_pass_exit_gate(&mut world);
            live_press_buttons(&mut world, 1);
            live_tick(&mut world);
            {
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                let borrowed = behaviors.borrow();
                assert_eq!(borrowed.intermission.running, 2, "{map} counts the scroll");
                assert_eq!(
                    borrowed.finale_text.as_deref(),
                    Some(live_expected_finale(which)),
                    "{map} scroll"
                );
                assert_eq!(borrowed.cd_tracks, vec![(3, 3), (2, 3)], "{map} cues");
                assert_eq!(borrowed.pending_travel, None, "{map} scroll shows before travel");
            }
        }
    }

    /// Q1-0261: shareware end (`ExitIntermission`, `client.qc:215`): an
    /// unregistered e1m7 run shows the shareware scroll, then the sell
    /// screen instead of traveling.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0261_shareware_sell_screen() {
        let Some(mut world) = live_q1_world("maps/e1m7.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        world.q1_behaviors().expect("Q1 behaviors").borrow_mut().registered = false;
        let exits = live_changelevel_exits(&world);
        let exit = exits[0].0.clone();
        live_enter_intermission(&mut world, &exit);
        live_press_buttons(&mut world, 0);
        live_pass_exit_gate(&mut world);
        live_press_buttons(&mut world, 1);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.intermission.running, 2);
            assert_eq!(
                borrowed.finale_text.as_deref(),
                Some(live_expected_finale("e1-shareware"))
            );
            assert_eq!(borrowed.pending_travel, None);
        }
        live_press_buttons(&mut world, 0);
        live_pass_exit_gate(&mut world);
        live_press_buttons(&mut world, 1);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.intermission.running, 3);
            assert!(borrowed.sell_screen);
            assert_eq!(borrowed.pending_travel, None, "sell screen shows before travel");
        }
    }

    /// Q1-0255: all-runes finale (`ExitIntermission`, `client.qc:223`):
    /// with every episode bit set, the third press scrolls the runes
    /// text instead of traveling; the fourth press travels.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0255_all_runes_finale() {
        let Some(mut world) = live_q1_world("maps/e1m7.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        world.q1_behaviors().expect("Q1 behaviors").borrow_mut().serverflags = 15;
        let exits = live_changelevel_exits(&world);
        let exit = exits[0].0.clone();
        live_enter_intermission(&mut world, &exit);
        live_press_buttons(&mut world, 0);
        live_pass_exit_gate(&mut world);
        live_press_buttons(&mut world, 1);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(
                behaviors.borrow().finale_text.as_deref(),
                Some(live_expected_finale("e1"))
            );
        }
        live_press_buttons(&mut world, 0);
        live_pass_exit_gate(&mut world);
        live_press_buttons(&mut world, 1);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.intermission.running, 3);
            assert_eq!(borrowed.finale_text.as_deref(), Some(live_expected_finale("runes")));
            assert_eq!(borrowed.pending_travel, None, "runes scroll shows before travel");
        }
        live_press_buttons(&mut world, 0);
        live_pass_exit_gate(&mut world);
        live_press_buttons(&mut world, 1);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.intermission.running, 4);
            assert_eq!(borrowed.pending_travel.as_deref(), Some("start"));
        }
    }

    /// Q1-0258/CUS-0227: e1m1 -> e1m2 carry (`SetChangeParms`,
    /// `client.qc:32`): the exit captures the scripted loadout, the
    /// intermission exits to pending travel, and the arrival decodes
    /// stripped items, clamped health, floored shells, weapon, armor,
    /// and flags at the e1m2 start.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0258_travel_carries_parms_to_e1m2() {
        use qa_world::combat::{ArmorState, CombatState, RegularArmor};

        use super::super::simulation::native_q1_weapons::{
            Q1_IT_AMMO_BITS, Q1_IT_AXE, Q1_IT_KEY1, Q1_IT_NAILGUN, Q1_IT_NAILS, Q1_IT_QUAD, Q1_IT_SHOTGUN,
        };

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let player = world.player_actor().cloned().expect("player");
        // Scripted campaign loadout: nailgun plus a key and a quad the
        // capture must strip, superhealth to clamp, thin shells to
        // floor, yellow armor to carry.
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let mut borrowed = behaviors.borrow_mut();
            borrowed.player_items |= Q1_IT_NAILGUN | Q1_IT_KEY1 | Q1_IT_QUAD;
            borrowed.player_ammo.shells = 10.0;
            borrowed.player_ammo.nails = 40.0;
            borrowed.player_state.weapon = Q1_IT_NAILGUN;
            borrowed.serverflags = 1;
        }
        let worn = world.server().simulation().combat_state(&player).unwrap().armor.clone();
        world
            .server_mut()
            .simulation_mut()
            .set_combat(
                &player,
                CombatState {
                    health: 150.0,
                    armor: ArmorState {
                        regular: RegularArmor::Q1 {
                            points: 120.0,
                            absorption: 0.6,
                            item: "q1:item_armor2".to_string(),
                        },
                        ..worn
                    },
                    ..CombatState::default()
                },
            )
            .unwrap();
        let exits = live_changelevel_exits(&world);
        let exit = exits[0].0.clone();
        live_enter_intermission(&mut world, &exit);
        live_press_buttons(&mut world, 0);
        live_pass_exit_gate(&mut world);
        live_press_buttons(&mut world, 1);
        live_tick(&mut world);
        let next = world
            .take_pending_travel()
            .expect("travel loads")
            .expect("pending travel");
        assert_eq!(next.map(), "maps/e1m2.bsp");
        assert!(
            world.take_pending_travel().expect("second take").is_none(),
            "take is at-most-once"
        );
        let (eye, _) = next.player_eye().expect("arrival eye");
        assert_eq!(eye, vec3(1496.0, 1664.0, 318.0), "arrival at the e1m2 start");
        let arrival = next.player_actor().cloned().expect("arrival player");
        {
            let behaviors = next.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.serverflags, 1, "flags persist");
            let items = borrowed.player_items & !Q1_IT_AMMO_BITS;
            assert_eq!(
                items,
                Q1_IT_AXE | Q1_IT_SHOTGUN | Q1_IT_NAILGUN,
                "keys and quad stripped"
            );
            assert_ne!(borrowed.player_items & Q1_IT_NAILS, 0, "ammo indicator refreshed");
            assert_eq!(borrowed.player_ammo.shells, 25.0, "shells floored");
            assert_eq!(borrowed.player_ammo.nails, 40.0);
            assert_eq!(borrowed.player_state.weapon, Q1_IT_NAILGUN);
            assert_eq!(borrowed.player_state.parms.health, 100.0, "entry parms snapshotted");
            assert_eq!(borrowed.mapname, "e1m2");
        }
        let combat = next
            .server()
            .simulation()
            .combat_state(&arrival)
            .expect("arrival combat");
        assert_eq!(combat.health, 100.0, "superhealth clamped");
        match &combat.armor.regular {
            RegularArmor::Q1 { points, absorption, .. } => {
                assert_eq!((*points, *absorption), (120.0, 0.6), "armor carried");
            }
            _ => panic!("arrival wears the carried armor"),
        }
    }

    /// Q1-0260: flagged `start` return (`DecodeLevelParms`, `client.qc:79`):
    /// e1m7 -> start with flags keeps the flags but sheds the carried
    /// inventory for the fresh loadout.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0260_flagged_start_return_sheds_parms() {
        use super::super::simulation::native_q1_weapons::{Q1_IT_AMMO_BITS, Q1_IT_AXE, Q1_IT_NAILGUN, Q1_IT_SHOTGUN};

        let Some(mut world) = live_q1_world("maps/e1m7.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let mut borrowed = behaviors.borrow_mut();
            borrowed.player_items |= Q1_IT_NAILGUN;
            borrowed.player_ammo.shells = 50.0;
            borrowed.serverflags = 1;
        }
        let exits = live_changelevel_exits(&world);
        let exit = exits[0].0.clone();
        live_enter_intermission(&mut world, &exit);
        // Past the episode scroll, then past running 3 to travel.
        for _ in 0..2 {
            live_press_buttons(&mut world, 0);
            live_pass_exit_gate(&mut world);
            live_press_buttons(&mut world, 1);
            live_tick(&mut world);
        }
        let next = world
            .take_pending_travel()
            .expect("travel loads")
            .expect("pending travel");
        assert_eq!(next.map(), "maps/start.bsp");
        {
            let behaviors = next.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.serverflags, 1, "flags persist");
            assert_eq!(
                borrowed.player_items & !Q1_IT_AMMO_BITS,
                Q1_IT_AXE | Q1_IT_SHOTGUN,
                "carry shed for the fresh loadout"
            );
            assert_eq!(borrowed.player_ammo.shells, 25.0);
            assert_eq!(borrowed.player_state.weapon, Q1_IT_SHOTGUN);
        }
        let arrival = next.player_actor().cloned().expect("arrival player");
        assert_eq!(
            next.server()
                .simulation()
                .combat_state(&arrival)
                .map(|combat| combat.health),
            Some(100.0)
        );
    }

    /// Q1-0222: flagged `start` prefers `info_player_start2`
    /// (`SelectSpawnPoint`, `client.qc:454`): the same e1m7 -> start
    /// travel spawns at start1 unflagged and start2 flagged.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0222_flagged_start_prefers_start2() {
        for (serverflags, eye) in [(0, vec3(544.0, 288.0, 54.0)), (1, vec3(544.0, 1536.0, 54.0))] {
            let Some(mut world) = live_q1_world("maps/e1m7.bsp", GameMode::Singleplayer, 1) else {
                return;
            };
            live_silence_door_fields(&mut world);
            world.q1_behaviors().expect("Q1 behaviors").borrow_mut().serverflags = serverflags;
            let exits = live_changelevel_exits(&world);
            let exit = exits[0].0.clone();
            live_enter_intermission(&mut world, &exit);
            for _ in 0..2 {
                live_press_buttons(&mut world, 0);
                live_pass_exit_gate(&mut world);
                live_press_buttons(&mut world, 1);
                live_tick(&mut world);
            }
            let next = world
                .take_pending_travel()
                .expect("travel loads")
                .expect("pending travel");
            assert_eq!(
                next.player_eye().expect("arrival eye").0,
                eye,
                "flags {serverflags} selects the spawn"
            );
        }
    }

    /// Setskill triggers in a live world carrying `message`, in spawn order.
    fn live_setskill_by_message(world: &PlayWorld, message: &str) -> Vec<qa_core::identity::ActorId> {
        use super::super::simulation::native_q1_triggers::Q1TriggerKind;

        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        borrowed
            .triggers
            .iter()
            .filter_map(|(id, trigger)| match &trigger.kind {
                Q1TriggerKind::SetSkill if trigger.source.message.as_deref() == Some(message) => Some(id.clone()),
                _ => None,
            })
            .collect()
    }

    /// Q1-0251: `start` skill doors (`trigger_setskill`, `triggers.qc:475`):
    /// touching the "2" door records the override, and the episode exit
    /// consumes it: the e1m1 arrival runs skill 2.
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0251_setskill_selects_next_map_skill() {
        let Some(mut world) = live_q1_world("maps/start.bsp", GameMode::Singleplayer, 1) else {
            return;
        };
        live_silence_door_fields(&mut world);
        let doors = live_setskill_by_message(&world, "2");
        assert_eq!(doors.len(), 1, "start has one hard-skill door");
        let at_door = live_volume_center(&world, &doors[0]);
        live_place_player(&mut world, at_door);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(behaviors.borrow().skill_override.as_deref(), Some("2"));
        }
        let exits = live_changelevel_exits(&world);
        let exit = exits
            .iter()
            .find(|(_, map)| map == "e1m1")
            .expect("start e1m1 exit")
            .0
            .clone();
        let at_exit = live_volume_center(&world, &exit);
        live_place_player(&mut world, at_exit);
        live_tick(&mut world);
        let next = world
            .take_pending_travel()
            .expect("travel loads")
            .expect("pending travel");
        assert_eq!(next.map(), "maps/e1m1.bsp");
        assert_eq!(
            next.q1_behaviors().expect("Q1 behaviors").borrow().skill,
            2,
            "override consumed at travel"
        );
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

    /// Live monsters of one kind on the map, in record order.
    fn live_monsters(
        world: &PlayWorld,
        kind: super::super::simulation::native_q1_monsters::Q1MonsterKind,
    ) -> Vec<qa_core::identity::ActorId> {
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        borrowed
            .monsters
            .iter()
            .filter(|(_, monster)| monster.kind == kind)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Live dogs on the map, in record order.
    fn live_dogs(world: &PlayWorld) -> Vec<qa_core::identity::ActorId> {
        use super::super::simulation::native_q1_monsters::Q1MonsterKind;
        live_monsters(world, Q1MonsterKind::Dog)
    }

    /// Live grunts on the map, in record order.
    fn live_grunts(world: &PlayWorld) -> Vec<qa_core::identity::ActorId> {
        use super::super::simulation::native_q1_monsters::Q1MonsterKind;
        live_monsters(world, Q1MonsterKind::Grunt)
    }

    /// Live enforcers on the map, in record order.
    fn live_enforcers(world: &PlayWorld) -> Vec<qa_core::identity::ActorId> {
        use super::super::simulation::native_q1_monsters::Q1MonsterKind;
        live_monsters(world, Q1MonsterKind::Enforcer)
    }

    /// Live ogres on the map, in record order.
    fn live_ogres(world: &PlayWorld) -> Vec<qa_core::identity::ActorId> {
        use super::super::simulation::native_q1_monsters::Q1MonsterKind;
        live_monsters(world, Q1MonsterKind::Ogre)
    }

    /// Live zombies on the map, in record order.
    fn live_zombies(world: &PlayWorld) -> Vec<qa_core::identity::ActorId> {
        use super::super::simulation::native_q1_monsters::Q1MonsterKind;
        live_monsters(world, Q1MonsterKind::Zombie)
    }

    /// Live fish on the map, in record order.
    fn live_fish(world: &PlayWorld) -> Vec<qa_core::identity::ActorId> {
        use super::super::simulation::native_q1_monsters::Q1MonsterKind;
        live_monsters(world, Q1MonsterKind::Fish)
    }

    /// Live knights on the map, in record order.
    fn live_knights(world: &PlayWorld) -> Vec<qa_core::identity::ActorId> {
        use super::super::simulation::native_q1_monsters::Q1MonsterKind;
        live_monsters(world, Q1MonsterKind::Knight)
    }

    /// Live fiends on the map, in record order.
    fn live_fiends(world: &PlayWorld) -> Vec<qa_core::identity::ActorId> {
        use super::super::simulation::native_q1_monsters::Q1MonsterKind;
        live_monsters(world, Q1MonsterKind::Fiend)
    }

    fn live_shamblers(world: &PlayWorld) -> Vec<qa_core::identity::ActorId> {
        use super::super::simulation::native_q1_monsters::Q1MonsterKind;
        live_monsters(world, Q1MonsterKind::Shambler)
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

    /// Monster feet position (placement handle for facing offsets).
    fn live_monster_feet(world: &PlayWorld, monster: &qa_core::identity::ActorId) -> qa_core::math::Vec3 {
        let body = world.server().simulation().body_state(monster).expect("monster body");
        vec3(body.origin.x, body.origin.y, body.origin.z + body.bounds.min.z)
    }

    /// Dog feet position (placement handle for facing offsets).
    fn live_dog_feet(world: &PlayWorld, dog: &qa_core::identity::ActorId) -> qa_core::math::Vec3 {
        live_monster_feet(world, dog)
    }

    /// Monster facing yaw in degrees (live ideal yaw).
    fn live_monster_yaw(world: &PlayWorld, monster: &qa_core::identity::ActorId) -> f64 {
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        borrowed.monsters.get(monster).expect("monster record").ideal_yaw
    }

    /// Stand the player `dist` units along a monster's facing, feet to feet.
    fn live_place_player_before(world: &mut PlayWorld, monster: &qa_core::identity::ActorId, dist: f32) {
        let feet = live_monster_feet(world, monster);
        let yaw = live_monster_yaw(world, monster).to_radians();
        live_place_player(
            world,
            vec3(
                feet.x + yaw.cos() as f32 * dist,
                feet.y + yaw.sin() as f32 * dist,
                feet.z,
            ),
        );
    }

    /// Stand the player `dist` units along the dog's facing, feet to feet.
    fn live_place_player_before_dog(world: &mut PlayWorld, dog: &qa_core::identity::ActorId, dist: f32) {
        live_place_player_before(world, dog, dist)
    }

    /// Hold every monster's think an hour out (bolts and gibs step on
    /// their own records, so in-flight bolts still resolve).
    fn live_hold_monsters(world: &mut PlayWorld) {
        let now = live_now(world);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let mut borrowed = behaviors.borrow_mut();
        let ids: Vec<qa_core::identity::ActorId> = borrowed.monsters.keys().cloned().collect();
        for id in ids {
            if let Some(monster) = borrowed.monsters.get_mut(&id) {
                monster.nextthink = now + 3600.0;
            }
        }
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

    /// e1m1 monster census at skill 2: 8 dogs plus 34 grunts (no
    /// `monster_*` record carries the 1024 not-hard bit, so every
    /// authored monster spawns).
    const LIVE_E1M1_SKILL2_TOTAL: u32 = 42;

    /// e2m1 monster census at skill 2: 26 enforcers plus 13 grunts and
    /// 7 dogs (one grunt and one dog carry the 1024 not-hard bit, so
    /// skill inhibition drops them).
    const LIVE_E2M1_SKILL2_TOTAL: u32 = 46;

    /// e1m2 native census at skill 2: 12 ogres plus 16 grunts plus
    /// 5 knights plus 3 fiends plus 6 wizards (a sixth knight and a
    /// fourth fiend carry the 1024 not-hard bit). The scrags keep the
    /// generic path until their slice lands, so they stay out of the
    /// native count.
    const LIVE_E1M2_NATIVE_SKILL2_TOTAL: u32 = 42;

    /// e1m3 native census at skill 2: 13 ogres plus 35 zombies plus
    /// 7 fiends plus 3 shamblers plus 7 wizards (all walking; five
    /// more zombies and two more fiends carry the 1024 not-hard bit).
    /// Every e1m3 monster kind is native now.
    const LIVE_E1M3_NATIVE_SKILL2_TOTAL: u32 = 65;

    /// e2m3 native census at skill 2: 15 ogres plus 7 zombies plus
    /// 6 fish counting twice each (the classic swim double-count)
    /// plus 1 fiend plus 3 shamblers: 15 + 7 + 12 + 1 + 3 = 38.
    /// The hell knights keep the generic path until their slice lands.
    const LIVE_E2M3_NATIVE_SKILL2_TOTAL: u32 = 38;

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
        assert_eq!(borrowed.total_monsters, LIVE_E1M1_SKILL2_TOTAL);
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
        // Pass cost on a live map (42 standing monsters, relink included).
        drop(borrowed);
        let start = std::time::Instant::now();
        for _ in 0..120 {
            world.step_monsters();
        }
        let per_pass = start.elapsed().as_secs_f64() * 1000.0 / 120.0;
        eprintln!("live: step_monsters on e1m1 = {per_pass:.3} ms/pass (42 monsters)");
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
        // The shared sighting publishes (last sighter wins the slot, so
        // with a full map it may name a packmate instead).
        assert!(borrowed.sight_entity.is_some(), "the sighting publishes");
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
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink, Q1_FLAG_NOTARGET};
        use super::super::simulation::native_q1_weapons::Q1_IT_INVISIBILITY;

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
            assert_eq!(behaviors.borrow().total_monsters, LIVE_E1M1_SKILL2_TOTAL);
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
        // Hold every other monster's think: the shared sighting slot
        // is stock last-writer-wins, so a full map would steal the
        // denmate's sighting before the packmate's think runs.
        {
            let now = live_now(&world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let mut borrowed = behaviors.borrow_mut();
            let ids: Vec<qa_core::identity::ActorId> = borrowed.monsters.keys().cloned().collect();
            for id in ids {
                if id != first && id != second {
                    if let Some(monster) = borrowed.monsters.get_mut(&id) {
                        monster.nextthink = now + 3600.0;
                    }
                }
            }
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

    // --- Live Q1 weapons harness (slice: hitscan, impulse, loadout) ---

    /// Build a NetQuake weapon command: buttons plus impulse with
    /// zeroed movement (firing never moves the player).
    fn live_weapon_command(buttons: i32, impulse: i32) -> qa_world::movement::types::UserCommand {
        use qa_world::movement::types::{Q1UserCommand, UserCommand};
        UserCommand::Q1Netquake(Q1UserCommand {
            acknowledged_server_time_seconds: 0.0,
            view_angles: vec3(0.0, 0.0, 0.0),
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons,
            impulse,
        })
    }

    /// Run the weapon pass once with buttons plus impulse (the attack
    /// input seam; the clock does not advance).
    fn live_fire(world: &mut PlayWorld, buttons: i32, impulse: i32) {
        let command = live_weapon_command(buttons, impulse);
        world.step_weapons(Some(&command));
    }

    /// Live player eye from the sim body (placement truth; the movement
    /// state never sees scripted placements).
    fn live_eye(world: &PlayWorld) -> qa_core::math::Vec3 {
        let player = world.player_actor().cloned().expect("player");
        let body = world.server().simulation().body_state(&player).expect("player body");
        vec3(body.origin.x, body.origin.y, body.origin.z + 22.0)
    }

    /// Aim the player body at a map point (view angles only; no step
    /// runs, so scripted placements hold).
    fn live_aim_at(world: &mut PlayWorld, target: qa_core::math::Vec3) {
        use qa_core::math::vector_to_angles;
        let eye = live_eye(world);
        let angles = vector_to_angles(vec3(target.x - eye.x, target.y - eye.y, target.z - eye.z));
        match world.player.as_mut().expect("player body") {
            PlayerBody::Q1(body) => body.view_angles = angles,
            _ => panic!("live Q1 world admits a Q1 body"),
        }
    }

    /// Pitch the player view by degrees (negative looks up).
    fn live_pitch(world: &mut PlayWorld, degrees: f32) {
        match world.player.as_mut().expect("player body") {
            PlayerBody::Q1(body) => body.view_angles.x += degrees,
            _ => panic!("live Q1 world admits a Q1 body"),
        }
    }

    /// Yaw the player view by degrees.
    fn live_yaw(world: &mut PlayWorld, degrees: f32) {
        match world.player.as_mut().expect("player body") {
            PlayerBody::Q1(body) => body.view_angles.y += degrees,
            _ => panic!("live Q1 world admits a Q1 body"),
        }
    }

    /// Place a dog `dist` units ahead of the player, level, facing
    /// whatever it faced (the dog stays stood-down until wounded).
    fn live_place_dog_before_player(world: &mut PlayWorld, dog: &qa_core::identity::ActorId, dist: f32) {
        use qa_core::math::angle_vectors;
        let player = world.player_actor().cloned().expect("player");
        let body = world.server().simulation().body_state(&player).expect("player body");
        let angles = world.player_eye().expect("player eye").1;
        let forward = angle_vectors(angles).forward;
        world
            .server_mut()
            .simulation_mut()
            .set_body_origin(
                dog,
                vec3(
                    body.origin.x + forward.x * dist,
                    body.origin.y + forward.y * dist,
                    body.origin.z,
                ),
            )
            .unwrap();
    }

    /// Live dog health.
    fn live_dog_health(world: &PlayWorld, dog: &qa_core::identity::ActorId) -> f64 {
        world
            .server()
            .simulation()
            .combat_state(dog)
            .map_or(0.0, |combat| combat.health)
    }

    /// Grant a weapon bit plus an ammo pool (weapon pickups land with
    /// the items slice; selection proves against direct grants).
    fn live_grant_weapon(world: &mut PlayWorld, bit: u32, shells: f64, nails: f64, rockets: f64, cells: f64) {
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let mut borrowed = behaviors.borrow_mut();
        borrowed.player_items |= bit;
        borrowed.player_ammo.shells = shells;
        borrowed.player_ammo.nails = nails;
        borrowed.player_ammo.rockets = rockets;
        borrowed.player_ammo.cells = cells;
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0173_shotgun_wounds_dog() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        // Stock spawn loadout rides admission.
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.weapon, 1);
            assert_eq!(borrowed.player_state.currentammo, 25.0);
            assert_eq!(borrowed.player_state.weaponmodel, "progs/v_shot.mdl");
        }
        let dogs = live_dogs(&world);
        let player = world.player_actor().cloned().expect("player");
        live_place_dog_before_player(&mut world, &dogs[0], 56.0);
        let center = live_volume_center(&world, &dogs[0]);
        live_aim_at(&mut world, center);
        let fired_at = live_now(&world);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.shells, 24.0, "one shell per shot");
            assert_eq!(borrowed.player_state.currentammo, 24.0);
            assert_eq!(borrowed.player_state.attack_finished, fired_at + 0.5);
            assert_eq!(borrowed.player_state.show_hostile, fired_at + 1.0);
            assert_eq!(borrowed.player_state.punchangle, vec3(-2.0, 0.0, 0.0));
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample == "weapons/guncock.wav" && sound.channel == 1),
                "shotgun cocks on CHAN_WEAPON"
            );
        }
        // Point-blank: all 6 pellets strike for 4 each.
        assert_eq!(live_dog_health(&world, &dogs[0]), 1.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&dogs[0]).expect("dog record");
            assert_eq!(monster.enemy.as_ref(), Some(&player), "wounds turn the dog");
            assert!(
                matches!(
                    monster.think,
                    Q1MonsterThink::Frame(Q1MonsterSeq::DogPain, _) | Q1MonsterThink::Frame(Q1MonsterSeq::DogPainB, _)
                ),
                "survival runs th_pain, got {:?}",
                monster.think
            );
            assert_eq!(borrowed.killed_monsters, 0);
        }
        // The refire gate holds: a second trigger pull fizzles.
        live_fire(&mut world, 1, 0);
        assert_eq!(live_dog_health(&world, &dogs[0]), 1.0);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        assert_eq!(behaviors.borrow().player_ammo.shells, 24.0);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0175_supershotgun_kills_and_falls_back() {
        use super::super::simulation::native_q1_weapons::Q1_IT_SUPER_SHOTGUN;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_grant_weapon(&mut world, Q1_IT_SUPER_SHOTGUN, 3.0, 0.0, 0.0, 0.0);
        let dogs = live_dogs(&world);
        live_place_dog_before_player(&mut world, &dogs[0], 56.0);
        let center = live_volume_center(&world, &dogs[0]);
        live_aim_at(&mut world, center);
        live_fire(&mut world, 0, 3);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(behaviors.borrow().player_state.weapon, Q1_IT_SUPER_SHOTGUN);
        }
        // Full double at close range: 14 pellets for 56, a clean kill.
        let fired_at = live_now(&world);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.shells, 1.0, "two shells per double");
            assert_eq!(borrowed.player_state.attack_finished, fired_at + 0.7);
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample == "weapons/shotgn2.wav"),
                "double booms"
            );
            assert_eq!(borrowed.killed_monsters, 1);
        }
        assert_eq!(live_dog_health(&world, &dogs[0]), -31.0);
        // The last shell fires the plain 6-pellet shot, not the double
        // (second dog, opposite lane: the corpse stays solid).
        live_advance(&mut world, 0.8);
        live_yaw(&mut world, 180.0);
        live_place_dog_before_player(&mut world, &dogs[1], 56.0);
        let center = live_volume_center(&world, &dogs[1]);
        live_aim_at(&mut world, center);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.shells, 0.0);
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample == "weapons/guncock.wav"),
                "single shell cocks the plain shotgun"
            );
            assert_eq!(
                borrowed
                    .sounds
                    .iter()
                    .filter(|sound| sound.sample == "weapons/shotgn2.wav")
                    .count(),
                1,
                "single shell never booms again"
            );
        }
        assert_eq!(live_dog_health(&world, &dogs[1]), 1.0);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0168_axe_swing_lands_late() {
        use super::super::simulation::native_q1_weapons::{Q1PlayerAttack, Q1TempEnt, Q1_IT_AXE};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let dogs = live_dogs(&world);
        live_place_dog_before_player(&mut world, &dogs[0], 48.0);
        let center = live_volume_center(&world, &dogs[0]);
        live_aim_at(&mut world, center);
        live_fire(&mut world, 0, 1);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.weapon, Q1_IT_AXE);
            assert_eq!(borrowed.player_state.weaponmodel, "progs/v_axe.mdl");
            assert_eq!(borrowed.player_state.currentammo, 0.0);
        }
        let swung_at = live_now(&world);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(
                borrowed.sounds.iter().any(|sound| sound.sample == "weapons/ax1.wav"),
                "swing whooshes"
            );
            assert!(
                matches!(
                    borrowed.player_state.attack,
                    Q1PlayerAttack::AxeSwing { fire_at, .. } if fire_at == swung_at + 0.2
                ),
                "frame-3 fire 0.2 s out, got {:?}",
                borrowed.player_state.attack
            );
            assert_eq!(borrowed.player_state.attack_finished, swung_at + 0.5);
        }
        assert_eq!(live_dog_health(&world, &dogs[0]), 25.0, "no instant damage");
        live_advance(&mut world, 0.3);
        assert_eq!(live_dog_health(&world, &dogs[0]), 5.0, "frame 3 lands 20");
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(
                matches!(borrowed.player_state.attack, Q1PlayerAttack::AxeSwing { .. }),
                "axe4 still flying at +0.3"
            );
            assert_eq!(borrowed.player_state.weaponframe, 3);
        }
        live_advance(&mut world, 0.2);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.attack, Q1PlayerAttack::None);
            assert_eq!(borrowed.player_state.weaponframe, 0);
        }
        // A wall swing thunks and sparks (straight down at the floor).
        live_pitch(&mut world, 90.0);
        live_advance(&mut world, 0.6);
        live_fire(&mut world, 1, 0);
        live_advance(&mut world, 0.3);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(
                borrowed.sounds.iter().any(|sound| sound.sample == "player/axhit2.wav"),
                "wall thunk"
            );
            assert!(
                borrowed
                    .temp_ents
                    .iter()
                    .any(|ent| matches!(ent, Q1TempEnt::Gunshot { .. })),
                "wall spark"
            );
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0179_impulse_selects_slot() {
        use super::super::simulation::native_q1_weapons::{
            Q1_IT_AXE, Q1_IT_GRENADE_LAUNCHER, Q1_IT_LIGHTNING, Q1_IT_NAILGUN, Q1_IT_ROCKET_LAUNCHER, Q1_IT_SHOTGUN,
            Q1_IT_SUPER_NAILGUN, Q1_IT_SUPER_SHOTGUN,
        };

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_grant_weapon(&mut world, Q1_IT_AXE, 0.0, 0.0, 0.0, 0.0);
        for (impulse, bit, shells, nails, rockets, cells, model, current) in [
            (1, Q1_IT_AXE, 0.0, 0.0, 0.0, 0.0, "progs/v_axe.mdl", 0.0),
            (2, Q1_IT_SHOTGUN, 12.0, 0.0, 0.0, 0.0, "progs/v_shot.mdl", 12.0),
            (3, Q1_IT_SUPER_SHOTGUN, 12.0, 0.0, 0.0, 0.0, "progs/v_shot2.mdl", 12.0),
            (4, Q1_IT_NAILGUN, 0.0, 30.0, 0.0, 0.0, "progs/v_nail.mdl", 30.0),
            (5, Q1_IT_SUPER_NAILGUN, 0.0, 30.0, 0.0, 0.0, "progs/v_nail2.mdl", 30.0),
            (6, Q1_IT_GRENADE_LAUNCHER, 0.0, 0.0, 7.0, 0.0, "progs/v_rock.mdl", 7.0),
            (7, Q1_IT_ROCKET_LAUNCHER, 0.0, 0.0, 7.0, 0.0, "progs/v_rock2.mdl", 7.0),
            (8, Q1_IT_LIGHTNING, 0.0, 0.0, 0.0, 40.0, "progs/v_light.mdl", 40.0),
        ] {
            live_grant_weapon(&mut world, bit, shells, nails, rockets, cells);
            live_fire(&mut world, 0, impulse);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.weapon, bit, "impulse {impulse} selects");
            assert_eq!(borrowed.player_state.currentammo, current);
            assert_eq!(borrowed.player_state.weaponmodel, model);
        }
        // Unowned refuses; owned-but-dry refuses; both keep the weapon.
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            behaviors.borrow_mut().player_items &= !Q1_IT_ROCKET_LAUNCHER;
        }
        live_fire(&mut world, 0, 7);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.weapon, Q1_IT_LIGHTNING);
            assert!(borrowed.sprints.iter().any(|print| print.text == "no weapon.\n"));
        }
        live_grant_weapon(&mut world, Q1_IT_ROCKET_LAUNCHER, 0.0, 0.0, 0.0, 40.0);
        live_fire(&mut world, 0, 7);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.weapon, Q1_IT_LIGHTNING);
            assert!(borrowed.sprints.iter().any(|print| print.text == "not enough ammo.\n"));
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0178_cycle_walks_roster() {
        use super::super::simulation::native_q1_weapons::{
            Q1_IT_AXE, Q1_IT_NAILGUN, Q1_IT_SHOTGUN, Q1_IT_SUPER_SHOTGUN,
        };

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_grant_weapon(&mut world, Q1_IT_SUPER_SHOTGUN, 25.0, 0.0, 0.0, 0.0);
        live_grant_weapon(&mut world, Q1_IT_NAILGUN, 25.0, 9.0, 0.0, 0.0);
        for (impulse, expect) in [
            (10, Q1_IT_SUPER_SHOTGUN),
            (10, Q1_IT_NAILGUN),
            (10, Q1_IT_AXE),
            (10, Q1_IT_SHOTGUN),
            (12, Q1_IT_AXE),
            (12, Q1_IT_NAILGUN),
        ] {
            live_fire(&mut world, 0, impulse);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(borrowed_player_weapon(&behaviors), expect, "impulse {impulse}");
        }

        fn borrowed_player_weapon(behaviors: &std::rc::Rc<std::cell::RefCell<Q1NativeBehaviors>>) -> u32 {
            behaviors.borrow().player_state.weapon
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0169_best_fallback_on_empty() {
        use super::super::simulation::native_q1_weapons::{Q1_IT_AXE, Q1_IT_SUPER_SHOTGUN};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let dogs = live_dogs(&world);
        live_place_dog_before_player(&mut world, &dogs[0], 56.0);
        let center = live_volume_center(&world, &dogs[0]);
        live_aim_at(&mut world, center);
        // Fire the last shell, then the next pull drops to the axe.
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            behaviors.borrow_mut().player_ammo.shells = 1.0;
            behaviors.borrow_mut().player_state.currentammo = 1.0;
        }
        live_fire(&mut world, 1, 0);
        assert_eq!(live_dog_health(&world, &dogs[0]), 1.0);
        live_advance(&mut world, 0.6);
        let sounds_before = {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            borrowed.sounds.len()
        };
        live_fire(&mut world, 1, 0);
        {
            // The idle path drops to the axe first, so the held trigger
            // swings it (`W_CheckNoAmmo` always fires the axe).
            use super::super::simulation::native_q1_weapons::Q1PlayerAttack;
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.weapon, Q1_IT_AXE, "empty drops to best");
            assert_eq!(borrowed.player_state.currentammo, 0.0);
            assert_eq!(borrowed.sounds.len(), sounds_before + 1);
            assert_eq!(borrowed.sounds.last().expect("swing").sample, "weapons/ax1.wav");
            assert!(matches!(borrowed.player_state.attack, Q1PlayerAttack::AxeSwing { .. }));
        }
        // The idle path downgrades a drained held weapon without a pull.
        live_grant_weapon(&mut world, Q1_IT_SUPER_SHOTGUN, 0.0, 0.0, 0.0, 0.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let mut borrowed = behaviors.borrow_mut();
            borrowed.player_state.weapon = Q1_IT_SUPER_SHOTGUN;
            borrowed.player_state.currentammo = 0.0;
        }
        live_advance(&mut world, 0.6);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert_eq!(behaviors.borrow().player_state.weapon, Q1_IT_AXE, "idle downgrades");
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0167_aim_bends_within_cone() {
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let dogs = live_dogs(&world);
        live_place_dog_before_player(&mut world, &dogs[0], 200.0);
        let center = live_volume_center(&world, &dogs[0]);
        // Ten degrees high still wounds: the cone bends onto the dog.
        live_aim_at(&mut world, center);
        live_pitch(&mut world, -10.0);
        live_fire(&mut world, 1, 0);
        assert!(
            live_dog_health(&world, &dogs[0]) < 25.0,
            "aim bends 10 degrees onto the dog"
        );
        // Thirty degrees high is past the cone: a clean miss.
        live_advance(&mut world, 0.6);
        live_aim_at(&mut world, center);
        live_pitch(&mut world, -30.0);
        let before = live_dog_health(&world, &dogs[1]);
        live_place_dog_before_player(&mut world, &dogs[1], 200.0);
        let center = live_volume_center(&world, &dogs[1]);
        live_aim_at(&mut world, center);
        live_pitch(&mut world, -30.0);
        live_fire(&mut world, 1, 0);
        assert_eq!(live_dog_health(&world, &dogs[1]), before, "past-cone shots miss");
    }

    /// Count flashed `TE_EXPLOSION` temp entities.
    fn live_explosions(world: &PlayWorld) -> usize {
        use super::super::simulation::native_q1_weapons::Q1TempEnt;
        world
            .q1_behaviors()
            .expect("Q1 behaviors")
            .borrow()
            .temp_ents
            .iter()
            .filter(|ent| matches!(ent, Q1TempEnt::Explosion { .. }))
            .count()
    }

    /// Swim the player to the first waist-deep retail water found on a
    /// coarse e1m1 grid (the discharge proof needs real map water, not a
    /// scripted level). Leaves the player floating there; no movement
    /// step runs, so gravity never pulls them out.
    fn live_move_player_to_water(world: &mut PlayWorld) -> qa_core::math::Vec3 {
        use super::super::simulation::native_q1_weapons::q1_sample_water_level;
        use crate::bootstrap::play::PlayerClip;
        let player = world.player_actor().cloned().expect("player");
        let mut z = -320.0;
        while z <= 400.0 {
            let mut x = -1600.0;
            while x <= 1600.0 {
                let mut y = -1600.0;
                while y <= 1600.0 {
                    let point = vec3(x, y, z);
                    live_place_player(world, point);
                    let PlayerClip::Q1(scene) = world.clip.as_ref().expect("Q1 clip") else {
                        panic!("live Q1 world clips on a Q1 scene");
                    };
                    let level = q1_sample_water_level(scene, world.server().simulation(), &player);
                    if level > 1 {
                        return point;
                    }
                    y += 160.0;
                }
                x += 160.0;
            }
            z += 80.0;
        }
        panic!("retail e1m1 has no waist-deep water on the scan grid");
    }

    /// Swim the player to the first retail water at least `min_level`
    /// deep (drowning sounds need full submersion).
    fn live_move_player_to_level(world: &mut PlayWorld, min_level: i32) -> qa_core::math::Vec3 {
        use super::super::simulation::native_q1_weapons::q1_sample_water_level;
        use crate::bootstrap::play::PlayerClip;
        let player = world.player_actor().cloned().expect("player");
        let mut z = -320.0;
        while z <= 400.0 {
            let mut x = -1600.0;
            while x <= 1600.0 {
                let mut y = -1600.0;
                while y <= 1600.0 {
                    let point = vec3(x, y, z);
                    live_place_player(world, point);
                    let PlayerClip::Q1(scene) = world.clip.as_ref().expect("Q1 clip") else {
                        panic!("live Q1 world clips on a Q1 scene");
                    };
                    let level = q1_sample_water_level(scene, world.server().simulation(), &player);
                    if level >= min_level {
                        return point;
                    }
                    y += 160.0;
                }
                x += 160.0;
            }
            z += 80.0;
        }
        panic!("retail e1m1 has no level-{min_level} water on the scan grid");
    }

    /// Hold the attack for `ticks` 60 Hz frames (the nail burst only
    /// keeps firing while the trigger stays down).
    fn live_hold_attack(world: &mut PlayWorld, ticks: u32) {
        for _ in 0..ticks {
            world
                .server_mut()
                .tick(qa_core::time::SourceTime::Seconds(1.0 / 60.0))
                .unwrap();
            let command = live_weapon_command(1, 0);
            world.step_weapons(Some(&command));
            world.step_monsters();
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0171_nailgun_bursts_nails() {
        use super::super::simulation::native_q1_weapons::{Q1MissileKind, Q1PlayerAttack, Q1TempEnt, Q1_IT_NAILGUN};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_grant_weapon(&mut world, Q1_IT_NAILGUN, 25.0, 10.0, 0.0, 0.0);
        live_fire(&mut world, 0, 4);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.weapon, Q1_IT_NAILGUN);
            assert_eq!(borrowed.player_state.currentammo, 10.0);
            assert_eq!(borrowed.player_state.weaponmodel, "progs/v_nail.mdl");
        }
        let dogs = live_dogs(&world);
        live_place_dog_before_player(&mut world, &dogs[0], 56.0);
        let center = live_volume_center(&world, &dogs[0]);
        live_aim_at(&mut world, center);
        let fired_at = live_now(&world);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.nails, 9.0, "one nail per shot");
            assert_eq!(borrowed.player_state.currentammo, 9.0);
            assert_eq!(borrowed.player_state.attack_finished, fired_at + 0.2);
            assert!(
                matches!(borrowed.player_state.attack, Q1PlayerAttack::Nail { .. }),
                "first shot arms the burst"
            );
            assert_eq!(borrowed.player_state.nail_side, -4.0, "barrels alternate 4/-4");
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample == "weapons/rocket1i.wav" && sound.channel == 1),
                "nailgun rasps on CHAN_WEAPON"
            );
            assert_eq!(borrowed.missiles.values().count(), 1);
            assert!(
                borrowed
                    .missiles
                    .values()
                    .all(|missile| missile.kind == Q1MissileKind::Spike),
                "plain nailgun launches spikes"
            );
        }
        // Three nails in 17 held frames (t=0, 0.1, 0.2); each flies 56
        // units home well before the next think.
        live_hold_attack(&mut world, 17);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.nails, 7.0, "three nails spent");
            assert_eq!(borrowed.player_state.nail_side, -4.0, "third shot flips back");
            assert!(
                matches!(borrowed.player_state.attack, Q1PlayerAttack::Nail { .. }),
                "held trigger keeps the burst armed"
            );
            assert!(
                borrowed
                    .temp_ents
                    .iter()
                    .any(|ent| matches!(ent, Q1TempEnt::Blood { count: 9, .. })),
                "spike wounds spray 9-blood"
            );
        }
        assert_eq!(live_dog_health(&world, &dogs[0]), -2.0, "three 9-damage nails");
        // Release retires the burst (`player_run`).
        live_tick(&mut world);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        assert_eq!(behaviors.borrow().player_state.attack, Q1PlayerAttack::None);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0174_super_nailgun_spends_two_then_one() {
        use super::super::simulation::native_q1_weapons::{
            Q1PlayerAttack, Q1TempEnt, Q1_IT_SHOTGUN, Q1_IT_SUPER_NAILGUN,
        };

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_grant_weapon(&mut world, Q1_IT_SUPER_NAILGUN, 25.0, 3.0, 0.0, 0.0);
        live_fire(&mut world, 0, 5);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.weapon, Q1_IT_SUPER_NAILGUN);
            assert_eq!(borrowed.player_state.currentammo, 3.0);
            assert_eq!(borrowed.player_state.weaponmodel, "progs/v_nail2.mdl");
        }
        let dogs = live_dogs(&world);
        live_place_dog_before_player(&mut world, &dogs[0], 56.0);
        let center = live_volume_center(&world, &dogs[0]);
        live_aim_at(&mut world, center);
        live_fire(&mut world, 1, 0);
        // First shot spends two nails on an 18-damage super spike (5
        // frames: the spike lands, the 0.1 s think has not fired yet).
        live_hold_attack(&mut world, 5);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.nails, 1.0, "super spike spends two");
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample == "weapons/spike2.wav" && sound.channel == 1),
                "super nailgun hums on CHAN_WEAPON"
            );
        }
        assert_eq!(live_dog_health(&world, &dogs[0]), 7.0, "one 18-damage spike");
        // One nail left fires the plain nailgun's single spike instead;
        // the following dry think downgrades while still holding.
        live_hold_attack(&mut world, 8);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.nails, 0.0, "last nail fires plain");
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample == "weapons/rocket1i.wav" && sound.channel == 1),
                "fallback shot rasps like the nailgun"
            );
            assert!(
                borrowed
                    .temp_ents
                    .iter()
                    .any(|ent| matches!(ent, Q1TempEnt::Blood { count: 18, .. })),
                "super spike sprays 18-blood"
            );
            assert!(
                borrowed
                    .temp_ents
                    .iter()
                    .any(|ent| matches!(ent, Q1TempEnt::Blood { count: 9, .. })),
                "plain fallback sprays 9-blood"
            );
        }
        assert_eq!(live_dog_health(&world, &dogs[0]), -2.0, "18 + 9 kills the dog");
        // Still holding: the dry think downgrades to the shotgun (stock
        // keeps dry-clicking in the nail anim until release).
        for _ in 0..12 {
            let weapon = world.q1_behaviors().expect("Q1 behaviors").borrow().player_state.weapon;
            if weapon == Q1_IT_SHOTGUN {
                break;
            }
            live_hold_attack(&mut world, 1);
        }
        assert_eq!(
            world.q1_behaviors().expect("Q1 behaviors").borrow().player_state.weapon,
            Q1_IT_SHOTGUN,
            "dry burst falls back to best"
        );
        live_tick(&mut world);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        assert_eq!(behaviors.borrow().player_state.attack, Q1PlayerAttack::None);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0170_grenade_detonates_on_contact_and_fuse() {
        use super::super::simulation::native_q1_weapons::{Q1MissileKind, Q1TempEnt, Q1_IT_GRENADE_LAUNCHER};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_grant_weapon(&mut world, Q1_IT_GRENADE_LAUNCHER, 25.0, 0.0, 2.0, 0.0);
        live_fire(&mut world, 0, 6);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.weapon, Q1_IT_GRENADE_LAUNCHER);
            assert_eq!(borrowed.player_state.currentammo, 2.0);
            assert_eq!(borrowed.player_state.weaponmodel, "progs/v_rock.mdl");
        }
        let dogs = live_dogs(&world);
        live_place_dog_before_player(&mut world, &dogs[0], 56.0);
        let center = live_volume_center(&world, &dogs[0]);
        live_aim_at(&mut world, center);
        let fired_at = live_now(&world);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.rockets, 1.0, "one rocket per grenade");
            assert_eq!(borrowed.player_state.attack_finished, fired_at + 0.6);
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample == "weapons/grenade.wav" && sound.channel == 1),
                "launcher thumps on CHAN_WEAPON"
            );
            assert_eq!(borrowed.missiles.values().count(), 1);
            let missile = borrowed.missiles.values().next().expect("grenade record");
            assert_eq!(missile.kind, Q1MissileKind::Grenade);
            assert_eq!(missile.fuse_at, Some(fired_at + 2.5), "2.5 s fuse");
        }
        // Contact with the dog detonates long before the fuse: ~0.1 s
        // of flight, then the 120-radius blast.
        for _ in 0..12 {
            live_tick(&mut world);
        }
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(
                borrowed
                    .temp_ents
                    .iter()
                    .any(|ent| matches!(ent, Q1TempEnt::Explosion { .. })),
                "contact detonates the grenade"
            );
            assert_eq!(borrowed.missiles.values().count(), 0, "blast retires the grenade");
            assert_eq!(borrowed.killed_monsters, 1);
        }
        assert!(live_dog_health(&world, &dogs[0]) < 0.0, "radius blast kills the dog");
        let player_health = live_player_health(&world);
        assert!(
            player_health > 0.0 && player_health < 100.0,
            "the attacker takes half falloff, got {player_health}"
        );
        // A lobbed grenade that touches nothing still pops on its fuse.
        live_advance(&mut world, 0.7);
        live_pitch(&mut world, -60.0);
        let lobbed_at = live_now(&world);
        live_fire(&mut world, 1, 0);
        assert_eq!(
            world.q1_behaviors().expect("Q1 behaviors").borrow().player_ammo.rockets,
            0.0,
            "second grenade spends the last rocket"
        );
        let player = world.player_actor().cloned().expect("player");
        let body = world.server().simulation().body_state(&player).expect("player body");
        live_place_player(&mut world, vec3(body.origin.x + 300.0, body.origin.y, body.origin.z));
        assert_eq!(live_explosions(&world), 1);
        let to_go = lobbed_at + 2.4 - live_now(&world);
        live_advance(&mut world, to_go.max(0.0));
        assert_eq!(live_explosions(&world), 1, "no early detonation before the fuse");
        live_advance(&mut world, 0.3);
        assert_eq!(live_explosions(&world), 2, "the fuse pops at 2.5 s");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        assert_eq!(behaviors.borrow().missiles.values().count(), 0);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0172_rocket_kills_dog() {
        use super::super::simulation::native_q1_weapons::{Q1MissileKind, Q1TempEnt, Q1_IT_ROCKET_LAUNCHER};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_grant_weapon(&mut world, Q1_IT_ROCKET_LAUNCHER, 25.0, 0.0, 1.0, 0.0);
        live_fire(&mut world, 0, 7);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.weapon, Q1_IT_ROCKET_LAUNCHER);
            assert_eq!(borrowed.player_state.currentammo, 1.0);
            assert_eq!(borrowed.player_state.weaponmodel, "progs/v_rock2.mdl");
        }
        let dogs = live_dogs(&world);
        live_place_dog_before_player(&mut world, &dogs[0], 56.0);
        let center = live_volume_center(&world, &dogs[0]);
        live_aim_at(&mut world, center);
        let fired_at = live_now(&world);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.rockets, 0.0, "one rocket per shot");
            assert_eq!(borrowed.player_state.attack_finished, fired_at + 0.8);
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample == "weapons/sgun1.wav" && sound.channel == 1),
                "launcher whooshes on CHAN_WEAPON"
            );
            assert_eq!(borrowed.missiles.values().count(), 1);
            let missile = borrowed.missiles.values().next().expect("rocket record");
            assert_eq!(missile.kind, Q1MissileKind::Rocket);
            assert_eq!(missile.remove_at, fired_at + 5.0, "rockets live 5 s");
        }
        for _ in 0..12 {
            live_tick(&mut world);
        }
        // Direct 100-120 excludes the victim from the radius falloff, so
        // the dog takes exactly the direct roll: 25 - [100, 120].
        let health = live_dog_health(&world, &dogs[0]);
        assert!(
            (-95.0..=-75.0).contains(&health),
            "direct 100-120 kills the dog, got {health}"
        );
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.killed_monsters, 1);
            assert!(
                borrowed
                    .temp_ents
                    .iter()
                    .any(|ent| matches!(ent, Q1TempEnt::Explosion { .. })),
                "impact flashes TE_EXPLOSION"
            );
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample == "weapons/r_exp3.wav" && sound.channel == 0),
                "impact thumps on CHAN_AUTO"
            );
            assert_eq!(borrowed.missiles.values().count(), 0, "impact retires the rocket");
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0176_lightning_kills_dog() {
        use super::super::simulation::native_q1_weapons::{Q1PlayerAttack, Q1TempEnt, Q1_IT_LIGHTNING};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_grant_weapon(&mut world, Q1_IT_LIGHTNING, 25.0, 0.0, 0.0, 5.0);
        live_fire(&mut world, 0, 8);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.weapon, Q1_IT_LIGHTNING);
            assert_eq!(borrowed.player_state.currentammo, 5.0);
            assert_eq!(borrowed.player_state.weaponmodel, "progs/v_light.mdl");
        }
        let dogs = live_dogs(&world);
        live_place_dog_before_player(&mut world, &dogs[0], 56.0);
        let center = live_volume_center(&world, &dogs[0]);
        live_aim_at(&mut world, center);
        let fired_at = live_now(&world);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.cells, 4.0, "one cell per shot");
            assert_eq!(borrowed.player_state.attack_finished, fired_at + 0.1);
            assert_eq!(borrowed.player_state.t_width, fired_at + 0.6, "impact throttled");
            assert!(
                matches!(borrowed.player_state.attack, Q1PlayerAttack::Lightning { .. }),
                "first shot arms the burst"
            );
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample == "weapons/lstart.wav" && sound.channel == 0),
                "generator whines on CHAN_AUTO"
            );
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample == "weapons/lhit.wav" && sound.channel == 1),
                "impact cracks on CHAN_WEAPON"
            );
            let player = world.player_actor().cloned().expect("player");
            let body = world.server().simulation().body_state(&player).expect("player body");
            let org = vec3(body.origin.x, body.origin.y, body.origin.z + 16.0);
            assert!(
                borrowed.temp_ents.iter().any(|ent| matches!(
                    ent,
                    Q1TempEnt::Lightning { entity, start, .. }
                    if entity == &player && start == &org
                )),
                "TE_LIGHTNING2 leaves the muzzle"
            );
        }
        // Exactly one 30-damage wound: the three traces dedup on the dog.
        assert_eq!(live_dog_health(&world, &dogs[0]), -5.0);
        // A second shot inside the throttle burns a cell but stays quiet.
        live_advance(&mut world, 0.2);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_ammo.cells, 3.0);
            assert_eq!(
                borrowed
                    .sounds
                    .iter()
                    .filter(|sound| sound.sample == "weapons/lhit.wav")
                    .count(),
                1,
                "lhit throttles for 0.6 s"
            );
        }
        live_advance(&mut world, 0.5);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(
                borrowed
                    .sounds
                    .iter()
                    .filter(|sound| sound.sample == "weapons/lhit.wav")
                    .count(),
                2,
                "lhit returns after the throttle"
            );
        }
        live_tick(&mut world);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        assert_eq!(behaviors.borrow().player_state.attack, Q1PlayerAttack::None);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0177_thunderbolt_discharges_underwater() {
        use super::super::simulation::native_q1_weapons::{Q1TempEnt, Q1_IT_LIGHTNING};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_grant_weapon(&mut world, Q1_IT_LIGHTNING, 25.0, 0.0, 0.0, 2.0);
        live_move_player_to_water(&mut world);
        live_fire(&mut world, 0, 8);
        assert_eq!(
            world.q1_behaviors().expect("Q1 behaviors").borrow().player_state.weapon,
            Q1_IT_LIGHTNING
        );
        let dogs = live_dogs(&world);
        live_place_dog_before_player(&mut world, &dogs[0], 40.0);
        let center = live_volume_center(&world, &dogs[0]);
        live_aim_at(&mut world, center);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(
                borrowed.player_state.water_level > 1,
                "the proof fires from real map water"
            );
            assert_eq!(borrowed.player_ammo.cells, 0.0, "discharge burns every cell");
            assert!(
                !borrowed
                    .temp_ents
                    .iter()
                    .any(|ent| matches!(ent, Q1TempEnt::Lightning { .. })),
                "discharge shows no beam"
            );
            assert!(
                !borrowed
                    .temp_ents
                    .iter()
                    .any(|ent| matches!(ent, Q1TempEnt::Explosion { .. })),
                "discharge shows no explosion"
            );
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample == "weapons/lstart.wav" && sound.channel == 0),
                "entry still whines (stock plays it outside the fire)"
            );
            assert!(
                !borrowed.sounds.iter().any(|sound| sound.sample == "weapons/lhit.wav"),
                "discharge cracks nothing"
            );
            assert_eq!(borrowed.killed_monsters, 1, "70-radius blast kills the dog");
        }
        assert!(live_dog_health(&world, &dogs[0]) < 0.0);
        // The firer takes half falloff at the blast center and lives: 2
        // cells deal (70 - ~4) / 2.
        let player_health = live_player_health(&world);
        assert!(
            player_health > 0.0 && player_health < 100.0,
            "discharge splashes the firer, got {player_health}"
        );
    }
    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0162_pain_cries_dry_and_drowns() {
        use super::super::simulation::native_q1_weapons::{Q1PlayerAttack, Q1_IT_GRENADE_LAUNCHER};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_grant_weapon(&mut world, Q1_IT_GRENADE_LAUNCHER, 25.0, 0.0, 3.0, 0.0);
        live_fire(&mut world, 0, 6);
        // Lob one skyward: the gun anim retires long before the fuse
        // pops, so the splash lands on an idle frame and cries out.
        live_pitch(&mut world, -60.0);
        live_fire(&mut world, 1, 0);
        let mut guard = 0;
        while live_player_health(&world) >= 100.0 {
            live_tick(&mut world);
            guard += 1;
            assert!(guard < 600, "fuse splash wounds the thrower");
        }
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample.starts_with("player/pain") && sound.channel == 2),
                "wound cries pain1-6 on CHAN_VOICE"
            );
            assert!(
                matches!(borrowed.player_state.attack, Q1PlayerAttack::Pain { .. }),
                "wound runs the pain anim"
            );
            assert_eq!(borrowed.player_state.weaponframe, 0);
            assert!(
                borrowed.player_state.pain_finished > live_now(&world),
                "pain arms the 0.5 s gate"
            );
        }
        // Full submersion samples live from retail water (the drown
        // branch itself is unit-proven: nothing wounds a swimmer with
        // an idle frame on e1m1 — self-splash needs the gun anim).
        live_advance(&mut world, 0.7);
        live_move_player_to_level(&mut world, 3);
        live_tick(&mut world);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.water_level, 3);
            assert_eq!(borrowed.player_state.water_type, -3, "submersion reads retail water");
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0160_death_gib_and_obituaries() {
        use super::super::simulation::native_q1_weapons::{
            Q1_DEAD_DEAD, Q1_DEAD_RESPAWNABLE, Q1_IT_GRENADE_LAUNCHER, Q1_IT_LIGHTNING, Q1_IT_ROCKET_LAUNCHER,
        };

        // Ten cells discharge into a gib: past -40 the player becomes
        // the bouncing head with three flesh chunks.
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_grant_weapon(&mut world, Q1_IT_LIGHTNING, 25.0, 0.0, 0.0, 10.0);
        live_move_player_to_water(&mut world);
        live_fire(&mut world, 0, 8);
        live_fire(&mut world, 1, 0);
        assert!(live_player_health(&world) < -40.0, "ten cells gib");
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.deadflag, Q1_DEAD_DEAD, "gibs skip dying");
            assert!(borrowed.player_state.gibbed_head);
            let player = world.player_actor().cloned().expect("player");
            let corpse = world.server().simulation().body_state(&player).expect("corpse").origin;
            let chunks: Vec<_> = borrowed.pending_gibs.iter().filter(|gib| gib.at == corpse).collect();
            assert_eq!(chunks.len(), 3, "three flesh chunks at the corpse");
            assert_eq!(chunks[0].model, "progs/gib1.mdl");
            assert_eq!(borrowed.player_state.weaponmodel, "");
            assert!(
                borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.sample == "player/gib.wav" || sound.sample == "player/udeath.wav"),
                "gib cry on CHAN_VOICE"
            );
            assert!(
                borrowed
                    .sprints
                    .iter()
                    .any(|sprint| sprint.text == "Player discharges into the water.\n"),
                "discharge obituary"
            );
            assert_eq!(borrowed.player_state.frags, -1.0);
            assert!(!borrowed.solids.contains(&player), "corpses unsolid");
            assert!(
                !world
                    .server()
                    .simulation()
                    .combat_state(&player)
                    .expect("player combat")
                    .can_take_damage,
                "corpses stop taking damage"
            );
        }
        // Singleplayer restarts on the release-and-press.
        live_fire(&mut world, 1, 0);
        assert_eq!(
            world
                .q1_behaviors()
                .expect("Q1 behaviors")
                .borrow()
                .player_state
                .deadflag,
            Q1_DEAD_DEAD,
            "held trigger waits"
        );
        live_tick(&mut world);
        assert_eq!(
            world
                .q1_behaviors()
                .expect("Q1 behaviors")
                .borrow()
                .player_state
                .deadflag,
            Q1_DEAD_RESPAWNABLE,
            "release opens respawn"
        );
        live_fire(&mut world, 1, 0);
        assert!(
            world
                .q1_behaviors()
                .expect("Q1 behaviors")
                .borrow()
                .player_state
                .restart_requested,
            "singleplayer death restarts the level"
        );

        // Grenade suicide pins the pin-back-in line. Each grenade is
        // hugged before its fuse pops: 20 units overhead is a certain
        // ~55 splash, so two pops kill.
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_grant_weapon(&mut world, Q1_IT_GRENADE_LAUNCHER, 25.0, 0.0, 3.0, 0.0);
        live_fire(&mut world, 0, 6);
        live_pitch(&mut world, -60.0);
        for _ in 0..3 {
            if live_player_health(&world) <= 0.0 {
                break;
            }
            live_fire(&mut world, 1, 0);
            live_advance(&mut world, 2.0);
            let grenade = {
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                let borrowed = behaviors.borrow();
                let id = { borrowed.missiles.keys().next().cloned() };
                id
            };
            if let Some(grenade) = grenade {
                let at = world
                    .server()
                    .simulation()
                    .body_state(&grenade)
                    .expect("grenade body")
                    .origin;
                live_place_player(&mut world, vec3(at.x, at.y, at.z + 20.0));
            }
            live_advance(&mut world, 0.6);
        }
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(live_player_health(&world) <= 0.0, "three splashes kill");
            assert!(
                borrowed
                    .sprints
                    .iter()
                    .any(|sprint| sprint.text == "Player tries to put the pin back in\n"),
                "grenade suicide line"
            );
        }

        // Rocket suicide bores the victim to death.
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_grant_weapon(&mut world, Q1_IT_ROCKET_LAUNCHER, 25.0, 0.0, 3.0, 0.0);
        live_fire(&mut world, 0, 7);
        live_pitch(&mut world, 90.0);
        for _ in 0..3 {
            live_fire(&mut world, 1, 0);
            live_advance(&mut world, 0.9);
        }
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let health = live_player_health(&world);
            assert!(health <= 0.0, "three foot-rockets kill, got {health}");
            assert!(
                borrowed
                    .sprints
                    .iter()
                    .any(|sprint| sprint.text == "Player becomes bored with life\n"),
                "rocket suicide line"
            );
        }

        // A dog bite at 1 health mauls.
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        // One health: the wounding paths are live-proven elsewhere
        // (0162 pain, 0177 discharge); this world proves the maul.
        live_set_player_health(&mut world, 1.0);
        let dogs = live_dogs(&world);
        live_place_dog_before_player(&mut world, &dogs[0], 20.0);
        let mut guard = 0;
        while live_player_health(&world) > 0.0 {
            live_tick(&mut world);
            guard += 1;
            assert!(guard < 1200, "dog finishes the 1-health player");
        }
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(
                borrowed
                    .sprints
                    .iter()
                    .any(|sprint| sprint.text == "Player was mauled by a Rottweiler\n"),
                "dog-maul line"
            );
            assert_eq!(borrowed.player_state.frags, -1.0);
        }
    }

    /// Tick a dead player until the corpse turns respawnable (death
    /// anims run up to 15 frames at 0.1 s, then one released tick
    /// opens the wait).
    fn live_wait_respawnable(world: &mut PlayWorld) {
        use super::super::simulation::native_q1_weapons::Q1_DEAD_RESPAWNABLE;
        let mut guard = 0;
        while world
            .q1_behaviors()
            .expect("Q1 behaviors")
            .borrow()
            .player_state
            .deadflag
            != Q1_DEAD_RESPAWNABLE
        {
            live_tick(world);
            guard += 1;
            assert!(guard < 1200, "corpse turns respawnable");
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0163_respawn_after_death() {
        use super::super::simulation::native_q1_weapons::{Q1TempEnt, Q1_DEAD_NO, Q1_IT_SHOTGUN};

        // Singleplayer restarts the level on the post-death press.
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let player = world.player_actor().cloned().expect("player");
        live_set_player_health(&mut world, 100.0);
        live_damage(&mut world, &player, None, 100.0);
        assert!(live_player_health(&world) <= 0.0, "exact 100 kills clean");
        live_wait_respawnable(&mut world);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(borrowed.player_state.restart_requested, "SP press restarts");
        }

        // Deathmatch respawns on the press: full health, fresh parms,
        // a corpse copy, and fog on a deathmatch spot.
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Deathmatch, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &player, None, 500.0);
        live_wait_respawnable(&mut world);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.deadflag, Q1_DEAD_NO);
            assert!(!borrowed.player_state.restart_requested, "DM never restarts");
            assert_eq!(live_player_health(&world), 100.0);
            assert_eq!(borrowed.player_state.weapon, Q1_IT_SHOTGUN, "fresh parms arm");
            assert_eq!(borrowed.player_ammo.shells, 25.0);
            assert_eq!(borrowed.player_state.body_queue.len(), 1, "one corpse copy");
            assert!(
                borrowed
                    .temp_ents
                    .iter()
                    .any(|ent| matches!(ent, Q1TempEnt::Teleport { .. })),
                "respawn fog cracker"
            );
            let spots: Vec<_> = borrowed
                .spawn_spots
                .iter()
                .filter(|spot| spot.classname == "info_player_deathmatch")
                .collect();
            assert!(!spots.is_empty(), "retail e1m1 admits DM spots");
            let at = world.server().simulation().body_state(&player).expect("body").origin;
            assert!(
                spots
                    .iter()
                    .any(|spot| { spot.origin.x == at.x && spot.origin.y == at.y && spot.origin.z + 1.0 == at.z }),
                "respawn lands on a DM spot, got {at:?}"
            );
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0181_coop_respawn_restores_parms() {
        use super::super::simulation::native_q1_weapons::{
            Q1TempEnt, Q1_DEAD_NO, Q1_IT_AMMO_BITS, Q1_IT_AXE, Q1_IT_ROCKET_LAUNCHER, Q1_IT_SHELLS, Q1_IT_SHOTGUN,
        };

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Coop, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert!(borrowed.coop, "coop world flags coop");
            assert_eq!(borrowed.player_state.parms.items, Q1_IT_AXE | Q1_IT_SHOTGUN);
            assert_eq!(borrowed.player_state.parms.weapon, Q1_IT_SHOTGUN);
        }
        // A mid-level pickup changes the live loadout, never the
        // level-entry parms.
        live_grant_weapon(&mut world, Q1_IT_ROCKET_LAUNCHER, 25.0, 0.0, 3.0, 0.0);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &player, None, 500.0);
        live_wait_respawnable(&mut world);
        live_fire(&mut world, 1, 0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(borrowed.player_state.deadflag, Q1_DEAD_NO);
            assert_eq!(live_player_health(&world), 100.0);
            assert_eq!(
                borrowed.player_items & !Q1_IT_AMMO_BITS,
                Q1_IT_AXE | Q1_IT_SHOTGUN,
                "entry parms drop the mid-level launcher"
            );
            assert_eq!(
                borrowed.player_items & Q1_IT_AMMO_BITS,
                Q1_IT_SHELLS,
                "stock ammo indicator for the held shotgun"
            );
            assert_eq!(borrowed.player_state.weapon, Q1_IT_SHOTGUN);
            assert_eq!(borrowed.player_ammo.shells, 25.0);
            assert_eq!(borrowed.player_ammo.rockets, 0.0);
            assert_eq!(borrowed.player_state.body_queue.len(), 1, "coop keeps a corpse");
            assert!(
                borrowed
                    .temp_ents
                    .iter()
                    .any(|ent| matches!(ent, Q1TempEnt::Teleport { .. })),
                "respawn fog cracker"
            );
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0155_body_queue_ring() {
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Deathmatch, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let player = world.player_actor().cloned().expect("player");
        // Five deaths: gib, gib, clean, gib, gib. The clean one keeps
        // the player model; gibs leave the head behind.
        let mut graves: Vec<(qa_core::math::Vec3, String)> = Vec::new();
        for round in 0..5 {
            if round == 2 {
                live_set_player_health(&mut world, 100.0);
                live_damage(&mut world, &player, None, 100.0);
            } else {
                live_damage(&mut world, &player, None, 500.0);
            }
            live_wait_respawnable(&mut world);
            live_fire(&mut world, 1, 0);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let fresh = borrowed.player_state.body_queue.last().expect("fresh corpse");
            graves.push((fresh.origin, fresh.model.clone()));
        }
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let queue = &borrowed.player_state.body_queue;
            assert_eq!(queue.len(), 4, "four-slot ring");
            for (slot, grave) in queue.iter().zip(graves.iter().skip(1)) {
                assert_eq!(slot.origin, grave.0, "oldest dropped, order kept");
                assert_eq!(slot.model, grave.1);
            }
            assert_eq!(queue[1].model, "progs/player.mdl", "clean death keeps the body");
            assert!(
                queue.iter().filter(|slot| slot.model == "progs/h_player.mdl").count() == 3,
                "gibs leave heads"
            );
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0138_grunt_spawn_stands_armed() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let grunts = live_grunts(&world);
        assert_eq!(grunts.len(), 34, "e1m1 spawns thirty-four grunts");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(borrowed.total_monsters, LIVE_E1M1_SKILL2_TOTAL);
        let mut walkers = 0;
        for grunt in &grunts {
            let monster = borrowed.monsters.get(grunt).expect("grunt record");
            assert_eq!(monster.flags & 512, 512, "dropped grunts stand on ground");
            assert_eq!(monster.flags & 32, 32, "start_go flags the monster bit");
            assert_eq!(monster.takedamage, 2, "start_go arms DAMAGE_AIM");
            assert_eq!(monster.view_ofs, vec3(0.0, 0.0, 25.0));
            let combat = world.server().simulation().combat_state(grunt).expect("grunt combat");
            assert_eq!(combat.health, 30.0);
            if monster.movetarget.is_some() {
                walkers += 1;
                assert!(
                    matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::GruntWalk, _)),
                    "targeted grunts walk out, got {:?}",
                    monster.think
                );
            } else {
                assert!(
                    matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::GruntStand, _)),
                    "grunts stand, got {:?}",
                    monster.think
                );
                assert!(monster.pausetime > 9999999.0, "targetless grunts stand down");
            }
        }
        assert_eq!(walkers, 7, "seven e1m1 grunts patrol corners");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0138_grunt_shotgun_wounds() {
        use super::super::simulation::native_q1_monsters::Q1_EF_MUZZLEFLASH;
        use super::super::simulation::native_q1_weapons::Q1TempEnt;

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let grunts = live_grunts(&world);
        live_place_player_before(&mut world, &grunts[0], 200.0);
        let mut fired = false;
        for _ in 0..600 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&grunts[0]).expect("grunt record");
            if monster.effects & Q1_EF_MUZZLEFLASH != 0
                && borrowed
                    .sounds
                    .iter()
                    .any(|sound| sound.entity == grunts[0] && sound.sample == "soldier/sattck1.wav")
            {
                fired = true;
                break;
            }
        }
        assert!(fired, "the grunt sights, hunts, and fires its shotgun");
        assert!(live_player_health(&world) < 100.0, "the shotgun burst wounds");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert!(
            borrowed
                .temp_ents
                .iter()
                .any(|ent| matches!(ent, Q1TempEnt::Blood { .. })),
            "pellet strikes queue blood"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0138_grunt_pain_then_dies_drops_backpack() {
        use super::super::simulation::native_q1_items::Q1ItemKind;
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let grunts = live_grunts(&world);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &grunts[0], Some(&player), 5.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&grunts[0]).expect("grunt record");
            assert!(
                matches!(
                    monster.think,
                    Q1MonsterThink::Frame(Q1MonsterSeq::GruntPain, 0)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::GruntPainB, 0)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::GruntPainC, 0)
                ),
                "wounds run pain, got {:?}",
                monster.think
            );
        }
        live_damage(&mut world, &grunts[0], Some(&player), 30.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&grunts[0]).expect("grunt record");
            assert!(monster.dead);
            assert!(
                matches!(
                    monster.think,
                    Q1MonsterThink::Frame(Q1MonsterSeq::GruntDie, 0)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::GruntDieC, 0)
                ),
                "death runs die, got {:?}",
                monster.think
            );
            assert_eq!(borrowed.killed_monsters, 1);
            assert!(borrowed.sounds.iter().any(|sound| sound.sample == "soldier/death1.wav"));
        }
        // The third death frame drops the pack unsolid.
        let mut pack = None;
        for _ in 0..60 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            pack = borrowed.items.iter().find_map(|(id, item)| match &item.kind {
                Q1ItemKind::Backpack { shells, .. } => Some((id.clone(), *shells)),
                _ => None,
            });
            if pack.is_some() {
                break;
            }
        }
        let Some((pack, shells)) = pack else {
            panic!("the death drop leaves a backpack");
        };
        assert_eq!(shells, 5.0, "grunts drop 5 shells");
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert!(
                !behaviors.borrow().solids.contains(&grunts[0]),
                "the death drop goes unsolid"
            );
        }
        // The pack settles, then the player takes it for 5 shells.
        for _ in 0..120 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let settled = borrowed_settled(&behaviors.borrow(), &pack);
            if settled {
                break;
            }
        }
        let center = live_volume_center(&world, &pack);
        let before = world.q1_behaviors().expect("Q1 behaviors").borrow().player_ammo.shells;
        live_place_player(&mut world, center);
        live_tick(&mut world);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(
            borrowed.player_ammo.shells,
            before + 5.0,
            "the pack grants its 5 shells"
        );
        assert!(!borrowed.items.keys().any(|id| id == &pack), "taken packs remove");
        assert!(
            borrowed.sprints.iter().any(|sprint| sprint.text == "You get 5 shells"),
            "the pack prints its receipt"
        );
    }

    /// Whether a backpack item has settled (toss physics retired it).
    fn borrowed_settled(
        behaviors: &std::cell::Ref<'_, super::super::simulation::native_q1_spawns::Q1NativeBehaviors>,
        pack: &qa_core::identity::ActorId,
    ) -> bool {
        use super::super::simulation::native_q1_items::Q1ItemKind;
        behaviors.items.get(pack).is_some_and(|item| match &item.kind {
            Q1ItemKind::Backpack { settled, .. } => *settled,
            _ => false,
        })
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0138_grunt_patrol_walks_corners() {
        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let grunts = live_grunts(&world);
        let patrol = grunts
            .iter()
            .find(|grunt| {
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                let borrowed = behaviors.borrow();
                borrowed
                    .monsters
                    .get(grunt)
                    .is_some_and(|monster| monster.movetarget.is_some())
            })
            .cloned()
            .expect("e1m1 routes a grunt through corners");
        let start = live_monster_feet(&world, &patrol);
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
        for _ in 0..1200 {
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
        let end = live_monster_feet(&world, &patrol);
        let moved = ((end.x - start.x).powi(2) + (end.y - start.y).powi(2)).sqrt();
        assert!(moved > 10.0, "the patrol travels, moved {moved}");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0138_grunt_nightmare_refires() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m1.bsp", GameMode::Singleplayer, 3) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let grunts = live_grunts(&world);
        live_place_player_before(&mut world, &grunts[0], 200.0);
        // The attack starts (any attack frame), then the nightmare
        // refire rewinds it to the first attack frame for a second
        // volley.
        let mut started = false;
        let mut rewound = false;
        for _ in 0..900 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&grunts[0]).expect("grunt record");
            if matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::GruntAttack, 4..)) {
                started = true;
            }
            if started && matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::GruntAttack, 0)) {
                rewound = true;
                break;
            }
            if started && matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::GruntRun, _)) && !rewound {
                break;
            }
        }
        assert!(started, "the grunt starts its attack on nightmare");
        assert!(rewound, "nightmare rewinds the attack for a second volley");
        // The rewind lands back on the first attack frame; the second
        // volley fires five frames later.
        let mut shots = 0;
        for _ in 0..60 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            shots = borrowed_shots(&behaviors.borrow(), &grunts[0]);
            if shots >= 2 {
                break;
            }
        }
        assert!(shots >= 2, "the refire volleys twice, got {shots} shots");
    }

    /// Shots one grunt has fired (its shotgun barks).
    fn borrowed_shots(
        behaviors: &std::cell::Ref<'_, super::super::simulation::native_q1_spawns::Q1NativeBehaviors>,
        grunt: &qa_core::identity::ActorId,
    ) -> usize {
        behaviors
            .sounds
            .iter()
            .filter(|sound| sound.entity == *grunt && sound.sample == "soldier/sattck1.wav")
            .count()
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0142_enforcer_spawn_stands_armed() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e2m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let enforcers = live_enforcers(&world);
        assert_eq!(enforcers.len(), 26, "e2m1 spawns twenty-six enforcers");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(borrowed.total_monsters, LIVE_E2M1_SKILL2_TOTAL);
        let mut walkers = 0;
        for enforcer in &enforcers {
            let monster = borrowed.monsters.get(enforcer).expect("enforcer record");
            assert_eq!(monster.flags & 512, 512, "dropped enforcers stand on ground");
            assert_eq!(monster.flags & 32, 32, "start_go flags the monster bit");
            assert_eq!(monster.takedamage, 2, "start_go arms DAMAGE_AIM");
            assert_eq!(monster.view_ofs, vec3(0.0, 0.0, 25.0));
            let combat = world
                .server()
                .simulation()
                .combat_state(enforcer)
                .expect("enforcer combat");
            assert_eq!(combat.health, 80.0);
            if matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerWalk, _)) {
                walkers += 1;
                assert!(monster.movetarget.is_some(), "walkers route through corners");
            } else {
                assert!(
                    matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerStand, _)),
                    "enforcers stand, got {:?}",
                    monster.think
                );
                assert!(monster.pausetime > 9999999.0, "targetless enforcers stand down");
            }
        }
        assert_eq!(walkers, 6, "six e2m1 enforcers patrol corners");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0142_enforcer_laser_wounds() {
        use super::super::simulation::native_q1_monsters::Q1_EF_MUZZLEFLASH;
        use super::super::simulation::native_q1_weapons::Q1TempEnt;

        let Some(mut world) = live_q1_world("maps/e2m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_set_player_health(&mut world, 1000.0);
        let enforcers = live_enforcers(&world);
        // e2m1 dens its enforcers behind walls and doors; offer the
        // player to each in turn until one sights and volleys.
        let mut gunner = None;
        for candidate in enforcers.iter().take(8) {
            live_place_player_before(&mut world, candidate, 200.0);
            let mut fired = false;
            for _ in 0..150 {
                live_tick(&mut world);
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                let borrowed = behaviors.borrow();
                let monster = borrowed.monsters.get(candidate).expect("enforcer record");
                if monster.effects & Q1_EF_MUZZLEFLASH != 0
                    && borrowed
                        .sounds
                        .iter()
                        .any(|sound| sound.entity == *candidate && sound.sample == "enforcer/enfire.wav")
                {
                    fired = true;
                    break;
                }
            }
            if fired {
                gunner = Some(candidate.clone());
                break;
            }
        }
        let gunner = gunner.expect("an enforcer sights, hunts, and fires its laser");
        // Hold every think: no fresh volleys launch, while in-flight
        // bolts still resolve on their own records.
        live_hold_monsters(&mut world);
        // Bolts fly at 600 u/s, so the wounds land after the barks.
        for _ in 0..90 {
            live_tick(&mut world);
        }
        assert!(live_player_health(&world) < 1000.0, "the laser volley wounds");
        // Every bolt removes on strike or at its 5 s think; drain
        // past the lifetime so no stray bolt outlives the proof.
        for _ in 0..400 {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            if behaviors.borrow().missiles.is_empty() {
                break;
            }
            live_tick(&mut world);
        }
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert!(
            borrowed
                .temp_ents
                .iter()
                .any(|ent| matches!(ent, Q1TempEnt::Blood { .. })),
            "bolt strikes queue blood"
        );
        assert!(
            borrowed
                .sounds
                .iter()
                .any(|sound| sound.sample == "enforcer/enfstop.wav"),
            "struck bolts crack"
        );
        assert!(
            borrowed.missiles.is_empty(),
            "struck bolts remove, {} still flying",
            borrowed.missiles.len()
        );
        let _ = gunner;
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0142_enforcer_pain_then_dies_drops_cells() {
        use super::super::simulation::native_q1_items::Q1ItemKind;
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e2m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let enforcers = live_enforcers(&world);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &enforcers[0], Some(&player), 5.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&enforcers[0]).expect("enforcer record");
            assert!(
                matches!(
                    monster.think,
                    Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerPainA, 0)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerPainB, 0)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerPainC, 0)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerPainD, 0)
                ),
                "wounds run pain, got {:?}",
                monster.think
            );
        }
        live_damage(&mut world, &enforcers[0], Some(&player), 80.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&enforcers[0]).expect("enforcer record");
            assert!(monster.dead);
            assert!(
                matches!(
                    monster.think,
                    Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerDie, 0)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerFDie, 0)
                ),
                "death runs die, got {:?}",
                monster.think
            );
            assert_eq!(borrowed.killed_monsters, 1);
            assert!(borrowed
                .sounds
                .iter()
                .any(|sound| sound.sample == "enforcer/death1.wav"));
        }
        // The third death frame drops the pack unsolid.
        let mut pack = None;
        for _ in 0..60 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            pack = borrowed.items.iter().find_map(|(id, item)| match &item.kind {
                Q1ItemKind::Backpack { cells, .. } => Some((id.clone(), *cells)),
                _ => None,
            });
            if pack.is_some() {
                break;
            }
        }
        let Some((pack, cells)) = pack else {
            panic!("the death drop leaves a backpack");
        };
        assert_eq!(cells, 5.0, "enforcers drop 5 cells");
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert!(
                !behaviors.borrow().solids.contains(&enforcers[0]),
                "the death drop goes unsolid"
            );
        }
        // The pack settles, then the player takes it for 5 cells.
        for _ in 0..120 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let settled = borrowed_settled(&behaviors.borrow(), &pack);
            if settled {
                break;
            }
        }
        let center = live_volume_center(&world, &pack);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(borrowed.player_ammo.cells, 5.0, "the pack grants its cells");
        assert!(!borrowed.items.keys().any(|id| id == &pack), "taken packs remove");
        assert!(
            borrowed.sprints.iter().any(|sprint| sprint.text == "You get 5 cells"),
            "the pack prints its receipt"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0142_enforcer_patrol_walks_corners() {
        let Some(mut world) = live_q1_world("maps/e2m1.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let enforcers = live_enforcers(&world);
        let patrol = enforcers
            .iter()
            .find(|enforcer| {
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                let borrowed = behaviors.borrow();
                borrowed
                    .monsters
                    .get(enforcer)
                    .is_some_and(|monster| monster.movetarget.is_some())
            })
            .cloned()
            .expect("e2m1 routes an enforcer through corners");
        let start = live_monster_feet(&world, &patrol);
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
        for _ in 0..1200 {
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
        let end = live_monster_feet(&world, &patrol);
        let moved = ((end.x - start.x).powi(2) + (end.y - start.y).powi(2)).sqrt();
        assert!(moved > 10.0, "the patrol travels, moved {moved}");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0142_enforcer_nightmare_refires() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e2m1.bsp", GameMode::Singleplayer, 3) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_set_player_health(&mut world, 1000.0);
        let enforcers = live_enforcers(&world);
        // Offer the player to each enforcer in turn until one starts
        // its attack, then watch that attack for the refire rewind.
        let mut gunner = None;
        for candidate in enforcers.iter().take(8) {
            live_place_player_before(&mut world, candidate, 200.0);
            let mut started = false;
            for _ in 0..150 {
                live_tick(&mut world);
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                let borrowed = behaviors.borrow();
                let monster = borrowed.monsters.get(candidate).expect("enforcer record");
                if matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerAttack, 5..)) {
                    started = true;
                    break;
                }
            }
            if started {
                gunner = Some(candidate.clone());
                break;
            }
        }
        let gunner = gunner.expect("an enforcer starts its attack on nightmare");
        let mut rewound = false;
        for _ in 0..900 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&gunner).expect("enforcer record");
            if matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerAttack, 0)) {
                rewound = true;
                break;
            }
            if matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::EnforcerRun, _)) {
                break;
            }
        }
        assert!(rewound, "nightmare rewinds the attack for a second volley");
        // The rewind lands back on the first attack frame; the second
        // volley fires its first bolt five frames later.
        let mut shots = 0;
        for _ in 0..60 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            shots = borrowed_bolts(&behaviors.borrow(), &gunner);
            if shots >= 3 {
                break;
            }
        }
        assert!(shots >= 3, "the refire volleys again, got {shots} bolts");
    }

    /// Bolts one enforcer has fired (its laser barks).
    fn borrowed_bolts(
        behaviors: &std::cell::Ref<'_, super::super::simulation::native_q1_spawns::Q1NativeBehaviors>,
        enforcer: &qa_core::identity::ActorId,
    ) -> usize {
        behaviors
            .sounds
            .iter()
            .filter(|sound| sound.entity == *enforcer && sound.sample == "enforcer/enfire.wav")
            .count()
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0146_ogre_spawn_stands_armed() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let ogres = live_ogres(&world);
        assert_eq!(ogres.len(), 12, "e1m2 spawns twelve ogres");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(borrowed.total_monsters, LIVE_E1M2_NATIVE_SKILL2_TOTAL);
        let mut walkers = 0;
        for ogre in &ogres {
            let monster = borrowed.monsters.get(ogre).expect("ogre record");
            assert_eq!(monster.flags & 512, 512, "dropped ogres stand on ground");
            assert_eq!(monster.flags & 32, 32, "start_go flags the monster bit");
            assert_eq!(monster.takedamage, 2, "start_go arms DAMAGE_AIM");
            assert_eq!(monster.view_ofs, vec3(0.0, 0.0, 25.0));
            let combat = world.server().simulation().combat_state(ogre).expect("ogre combat");
            assert_eq!(combat.health, 200.0);
            if matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::OgreWalk, _)) {
                walkers += 1;
                assert!(monster.movetarget.is_some(), "walkers route through corners");
            } else {
                assert!(
                    matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::OgreStand, _)),
                    "ogres stand, got {:?}",
                    monster.think
                );
                assert!(monster.pausetime > 9999999.0, "targetless ogres stand down");
            }
        }
        // A seventh ogre routes t99 but carries the 1024 not-hard bit,
        // so skill inhibition drops it.
        assert_eq!(walkers, 6, "six e1m2 ogres patrol corners");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0146_ogre_grenade_wounds() {
        use super::super::simulation::native_q1_monsters::Q1_EF_MUZZLEFLASH;
        use super::super::simulation::native_q1_weapons::Q1TempEnt;

        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_set_player_health(&mut world, 1000.0);
        let ogres = live_ogres(&world);
        // Offer the player to each ogre in turn until one sights and
        // lobs. The 350-unit stand keeps mid range past the 1 s hunt
        // hold, so the ogre lobs instead of closing to the saw.
        let mut gunner = None;
        for candidate in ogres.iter().take(8) {
            live_place_player_before(&mut world, candidate, 350.0);
            let mut fired = false;
            for _ in 0..150 {
                live_tick(&mut world);
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                let borrowed = behaviors.borrow();
                let monster = borrowed.monsters.get(candidate).expect("ogre record");
                if monster.effects & Q1_EF_MUZZLEFLASH != 0
                    && borrowed
                        .sounds
                        .iter()
                        .any(|sound| sound.entity == *candidate && sound.sample == "weapons/grenade.wav")
                {
                    fired = true;
                    break;
                }
            }
            if fired {
                gunner = Some(candidate.clone());
                break;
            }
        }
        gunner.expect("an ogre sights, hunts, and lobs its grenade");
        // Hold every think: no fresh attacks launch, while in-flight
        // grenades still bounce and burst on their own records.
        live_hold_monsters(&mut world);
        // The lob arcs, bounces, and bursts on contact or fuse; only
        // grenade blasts can wound from here.
        let mut wounded = false;
        for _ in 0..240 {
            live_tick(&mut world);
            if live_player_health(&world) < 1000.0 {
                wounded = true;
                break;
            }
        }
        assert!(wounded, "the grenade blast wounds");
        // Every grenade bursts on contact or at its 2.5 s fuse; drain
        // past the fuse so no stray grenade outlives the proof.
        for _ in 0..400 {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            if behaviors.borrow().missiles.is_empty() {
                break;
            }
            live_tick(&mut world);
        }
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert!(
            borrowed
                .temp_ents
                .iter()
                .any(|ent| matches!(ent, Q1TempEnt::Explosion { .. })),
            "bursts queue the explosion flash"
        );
        assert!(
            borrowed.sounds.iter().any(|sound| sound.sample == "weapons/r_exp3.wav"),
            "bursts crack"
        );
        assert!(
            borrowed.missiles.is_empty(),
            "burst grenades remove, {} still bouncing",
            borrowed.missiles.len()
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0146_ogre_melee_wounds() {
        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_set_player_health(&mut world, 1000.0);
        let ogres = live_ogres(&world);
        // Stand inside chainsaw reach of each ogre in turn until one
        // starts its stroke and rips.
        let mut ripper = None;
        for candidate in ogres.iter().take(8) {
            live_place_player_before(&mut world, candidate, 80.0);
            let mut stroked = false;
            for _ in 0..150 {
                live_tick(&mut world);
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                if behaviors
                    .borrow()
                    .sounds
                    .iter()
                    .any(|sound| sound.entity == *candidate && sound.sample == "ogre/ogsawatk.wav")
                {
                    stroked = true;
                    break;
                }
            }
            if stroked {
                ripper = Some(candidate.clone());
                break;
            }
        }
        ripper.expect("an ogre starts its chainsaw stroke");
        let mut wounded = false;
        for _ in 0..120 {
            live_tick(&mut world);
            if live_player_health(&world) < 1000.0 {
                wounded = true;
                break;
            }
        }
        assert!(wounded, "the chainsaw stroke rips");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0146_ogre_pain_then_dies_drops_rockets() {
        use super::super::simulation::native_q1_items::Q1ItemKind;
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let ogres = live_ogres(&world);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &ogres[0], Some(&player), 5.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&ogres[0]).expect("ogre record");
            assert!(
                matches!(
                    monster.think,
                    Q1MonsterThink::Frame(Q1MonsterSeq::OgrePain, 0)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::OgrePainB, 0)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::OgrePainC, 0)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::OgrePainD, 0)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::OgrePainE, 0)
                ),
                "wounds run pain, got {:?}",
                monster.think
            );
        }
        live_damage(&mut world, &ogres[0], Some(&player), 200.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&ogres[0]).expect("ogre record");
            assert!(monster.dead);
            assert!(
                matches!(
                    monster.think,
                    Q1MonsterThink::Frame(Q1MonsterSeq::OgreDie, 0) | Q1MonsterThink::Frame(Q1MonsterSeq::OgreBDie, 0)
                ),
                "death runs die, got {:?}",
                monster.think
            );
            assert_eq!(borrowed.killed_monsters, 1);
            assert!(borrowed.sounds.iter().any(|sound| sound.sample == "ogre/ogdth.wav"));
        }
        // The third death frame drops the pack unsolid.
        let mut pack = None;
        for _ in 0..60 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            pack = borrowed.items.iter().find_map(|(id, item)| match &item.kind {
                Q1ItemKind::Backpack { rockets, .. } => Some((id.clone(), *rockets)),
                _ => None,
            });
            if pack.is_some() {
                break;
            }
        }
        let Some((pack, rockets)) = pack else {
            panic!("the death drop leaves a backpack");
        };
        assert_eq!(rockets, 2.0, "ogres drop 2 rockets");
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            assert!(
                !behaviors.borrow().solids.contains(&ogres[0]),
                "the death drop goes unsolid"
            );
        }
        // The pack settles, then the player takes it for 2 rockets.
        for _ in 0..120 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let settled = borrowed_settled(&behaviors.borrow(), &pack);
            if settled {
                break;
            }
        }
        let center = live_volume_center(&world, &pack);
        live_place_player(&mut world, center);
        live_tick(&mut world);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(borrowed.player_ammo.rockets, 2.0, "the pack grants its rockets");
        assert!(!borrowed.items.keys().any(|id| id == &pack), "taken packs remove");
        assert!(
            borrowed.sprints.iter().any(|sprint| sprint.text == "You get 2 rockets"),
            "the pack prints its receipt"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0146_ogre_patrol_walks_corners() {
        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let ogres = live_ogres(&world);
        let patrol = ogres
            .iter()
            .find(|ogre| {
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                let borrowed = behaviors.borrow();
                borrowed
                    .monsters
                    .get(ogre)
                    .is_some_and(|monster| monster.movetarget.is_some())
            })
            .cloned()
            .expect("e1m2 routes an ogre through corners");
        let start = live_monster_feet(&world, &patrol);
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
        for _ in 0..1200 {
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
        let end = live_monster_feet(&world, &patrol);
        let moved = ((end.x - start.x).powi(2) + (end.y - start.y).powi(2)).sqrt();
        assert!(moved > 10.0, "the patrol travels, moved {moved}");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0153_zombie_spawn_stands_armed() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let zombies = live_zombies(&world);
        assert_eq!(zombies.len(), 35, "e1m3 spawns thirty-five zombies");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(borrowed.total_monsters, LIVE_E1M3_NATIVE_SKILL2_TOTAL);
        let mut walkers = 0;
        for zombie in &zombies {
            let monster = borrowed.monsters.get(zombie).expect("zombie record");
            assert_eq!(monster.flags & 512, 512, "dropped zombies stand on ground");
            assert_eq!(monster.flags & 32, 32, "start_go flags the monster bit");
            assert_eq!(monster.takedamage, 2, "start_go arms DAMAGE_AIM");
            assert_eq!(monster.view_ofs, vec3(0.0, 0.0, 25.0));
            assert_eq!(monster.inpain, 0);
            let combat = world.server().simulation().combat_state(zombie).expect("zombie combat");
            assert_eq!(combat.health, 60.0);
            if matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::ZombieWalk, _)) {
                walkers += 1;
                assert!(monster.movetarget.is_some(), "walkers route through corners");
            } else {
                assert!(
                    matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::ZombieStand, _)),
                    "zombies stand, got {:?}",
                    monster.think
                );
                assert!(monster.pausetime > 9999999.0, "targetless zombies stand down");
            }
        }
        assert_eq!(walkers, 11, "eleven e1m3 zombies patrol corners");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0153_zombie_flesh_wounds() {
        use super::super::simulation::native_q1_weapons::Q1MissileKind;

        let Some(mut world) = live_q1_world("maps/e1m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_set_player_health(&mut world, 1000.0);
        let zombies = live_zombies(&world);
        // Offer the player to each zombie in turn until one sights and
        // throws. The stand keeps near range at a standing player
        // origin (feet + 24, the stock stance `live_place_player_before`
        // sinks to the floor, which the exact-aimed lob sails over);
        // the throw runs 1.2-1.4 s after the 1 s hunt hold, so each
        // sighted candidate gets 5 s, and the wound lands within 3 s.
        let mut hurler = None;
        for candidate in zombies.iter().take(12) {
            let feet = live_monster_feet(&world, candidate);
            let yaw = live_monster_yaw(&world, candidate).to_radians();
            live_place_player(
                &mut world,
                vec3(
                    feet.x + yaw.cos() as f32 * 200.0,
                    feet.y + yaw.sin() as f32 * 200.0,
                    feet.z + 24.0,
                ),
            );
            let mut sighted = false;
            for _ in 0..90 {
                live_tick(&mut world);
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                if behaviors
                    .borrow()
                    .monsters
                    .get(candidate)
                    .and_then(|monster| monster.enemy.clone())
                    .is_some()
                {
                    sighted = true;
                    break;
                }
            }
            if !sighted {
                continue;
            }
            let mut fired = false;
            for _ in 0..300 {
                live_tick(&mut world);
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                if behaviors
                    .borrow()
                    .sounds
                    .iter()
                    .any(|sound| sound.entity == *candidate && sound.sample == "zombie/z_shot1.wav")
                {
                    fired = true;
                    break;
                }
            }
            if !fired {
                continue;
            }
            let mut wounded = false;
            for _ in 0..180 {
                live_tick(&mut world);
                if live_player_health(&world) < 1000.0 {
                    wounded = true;
                    break;
                }
            }
            if wounded {
                hurler = Some(candidate.clone());
                break;
            }
        }
        hurler.expect("a zombie sights, hunts, and lands its flesh");
        // Hold every think: no fresh throws launch, while in-flight
        // chunks still resolve on their own records.
        live_hold_monsters(&mut world);
        for _ in 0..400 {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            if behaviors.borrow().missiles.is_empty() {
                break;
            }
            live_tick(&mut world);
        }
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert!(
            borrowed.sounds.iter().any(|sound| sound.sample == "zombie/z_hit.wav"),
            "struck chunks thump wet"
        );
        assert!(
            borrowed
                .missiles
                .values()
                .all(|missile| missile.kind != Q1MissileKind::ZombieFlesh),
            "spent chunks remove"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0153_zombie_pain_knockdown_revives() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let zombies = live_zombies(&world);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &zombies[0], Some(&player), 30.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&zombies[0]).expect("zombie record");
            assert_eq!(monster.inpain, 2);
            assert_eq!(
                monster.think,
                Q1MonsterThink::Frame(Q1MonsterSeq::ZombiePainE, 0),
                "a 30-point hit knocks the zombie down"
            );
        }
        // The fall runs 30 frames with a 5 s lie at paine11; 12 s runs
        // the full knockdown and the stand back up.
        live_advance(&mut world, 12.0);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        let monster = borrowed.monsters.get(&zombies[0]).expect("zombie record");
        assert_eq!(monster.inpain, 0, "the run clears the knockdown");
        assert!(
            matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::ZombieRun, _)),
            "the zombie hunts again, got {:?}",
            monster.think
        );
        assert!(borrowed.solids.contains(&zombies[0]), "revived zombies stand solid");
        assert!(
            borrowed.sounds.iter().any(|sound| sound.sample == "zombie/z_fall.wav"),
            "the fall thumps"
        );
        let combat = world
            .server()
            .simulation()
            .combat_state(&zombies[0])
            .expect("zombie combat");
        assert_eq!(combat.health, 60.0);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0153_zombie_gib_dies() {
        let Some(mut world) = live_q1_world("maps/e1m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let zombies = live_zombies(&world);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &zombies[0], Some(&player), 65.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&zombies[0]).expect("zombie record");
            assert!(monster.dead);
            assert_eq!(borrowed.killed_monsters, 1);
            assert_eq!(borrowed.pending_gibs.len(), 3);
            assert!(borrowed.gibs.contains_key(&zombies[0]), "the head keeps the actor");
            assert!(borrowed.sounds.iter().any(|sound| sound.sample == "zombie/z_gib.wav"));
        }
        live_tick(&mut world);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        assert_eq!(behaviors.borrow().gibs.len(), 4, "three chunks plus the head");
        assert!(
            behaviors.borrow().pending_gibs.is_empty(),
            "the pass spawns queued chunks"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0153_zombie_crucified_hangs() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/start.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let zombies = live_zombies(&world);
        assert_eq!(zombies.len(), 9, "start hangs nine zombies");
        let before: Vec<qa_core::math::Vec3> = zombies
            .iter()
            .map(|zombie| {
                world
                    .server()
                    .simulation()
                    .body_state(zombie)
                    .expect("zombie body")
                    .origin
            })
            .collect();
        live_advance(&mut world, 2.0);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(borrowed.total_monsters, 0, "crucified zombies skip the kill count");
        for zombie in &zombies {
            let monster = borrowed.monsters.get(zombie).expect("zombie record");
            assert!(
                matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::ZombieCruc, _)),
                "crucified zombies hang, got {:?}",
                monster.think
            );
            assert_eq!(monster.flags, 0, "no floor drop, no monster bit");
            assert_eq!(monster.takedamage, 0, "crucified zombies stay unarmed");
        }
        drop(borrowed);
        for (zombie, hung) in zombies.iter().zip(before.iter()) {
            let at = world
                .server()
                .simulation()
                .body_state(zombie)
                .expect("zombie body")
                .origin;
            assert_eq!(at, *hung, "nailed-up zombies never fall");
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0153_zombie_patrol_walks_corners() {
        let Some(mut world) = live_q1_world("maps/e1m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let zombies = live_zombies(&world);
        let patrol = zombies
            .iter()
            .find(|zombie| {
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                let borrowed = behaviors.borrow();
                borrowed
                    .monsters
                    .get(zombie)
                    .is_some_and(|monster| monster.movetarget.is_some())
            })
            .cloned()
            .expect("e1m3 routes a zombie through corners");
        let start = live_monster_feet(&world, &patrol);
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
        for _ in 0..1200 {
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
        let end = live_monster_feet(&world, &patrol);
        let moved = ((end.x - start.x).powi(2) + (end.y - start.y).powi(2)).sqrt();
        assert!(moved > 10.0, "the patrol travels, moved {moved}");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0143_fish_spawn_settles_armed() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e2m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        let fish = live_fish(&world);
        assert_eq!(fish.len(), 6, "e2m3 spawns six fish");
        let spawned: Vec<qa_core::math::Vec3> = fish
            .iter()
            .map(|fish| world.server().simulation().body_state(fish).expect("fish body").origin)
            .collect();
        live_advance(&mut world, 1.0);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(borrowed.total_monsters, LIVE_E2M3_NATIVE_SKILL2_TOTAL);
        for (one, at) in fish.iter().zip(spawned.iter()) {
            let monster = borrowed.monsters.get(one).expect("fish record");
            assert_eq!(monster.flags & 2, 2, "swim start flags FL_SWIM");
            assert_eq!(monster.flags & 32, 32, "swim start flags the monster bit");
            assert_eq!(monster.takedamage, 2, "swim start arms DAMAGE_AIM");
            assert_eq!(monster.view_ofs, vec3(0.0, 0.0, 10.0));
            assert_eq!(monster.yaw_speed, 10.0);
            assert!(
                matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::FishStand, _)),
                "fish stand, got {:?}",
                monster.think
            );
            assert!(monster.pausetime > 9999999.0, "targetless fish stand down");
            let combat = world.server().simulation().combat_state(one).expect("fish combat");
            assert_eq!(combat.health, 25.0);
            let settled = world.server().simulation().body_state(one).expect("fish body").origin;
            // Stock `SV_Physics_Step` freefalls step-movers with no
            // onground/fly/swim bits, and stock fish spawn flags 0 —
            // so they sink toward the pool floor until the start-go
            // arms SWIM. Settle, never rise.
            assert!(
                settled.z <= at.z + 0.01,
                "pre-start freefall settles, {settled:?} vs spawn {at:?}"
            );
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0143_fish_bite_wounds() {
        let Some(mut world) = live_q1_world("maps/e2m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_set_player_health(&mut world, 1000.0);
        let fish = live_fish(&world);
        // Offer the player to each fish in turn until one sights and
        // bites. The 50-unit stand sits inside bite range, so the
        // first stroke lands within 5 s of the sighting.
        let mut biter = None;
        for candidate in fish.iter() {
            live_place_player_before(&mut world, candidate, 50.0);
            let mut bit = false;
            for _ in 0..300 {
                live_tick(&mut world);
                if live_player_health(&world) < 1000.0 {
                    bit = true;
                    break;
                }
            }
            if bit {
                biter = Some(candidate.clone());
                break;
            }
        }
        biter.expect("a fish sights, hunts, and lands its bite");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        assert!(
            behaviors
                .borrow()
                .sounds
                .iter()
                .any(|sound| sound.sample == "fish/bite.wav"),
            "bites snap"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0143_fish_pain_then_dies_no_gib() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e2m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let fish = live_fish(&world);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &fish[0], Some(&player), 5.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            assert_eq!(
                borrowed.monsters.get(&fish[0]).expect("fish record").think,
                Q1MonsterThink::Frame(Q1MonsterSeq::FishPain, 0),
                "wounds always run pain"
            );
        }
        live_damage(&mut world, &fish[0], Some(&player), 30.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&fish[0]).expect("fish record");
            assert!(monster.dead);
            assert_eq!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::FishDie, 0));
            assert_eq!(borrowed.killed_monsters, 1);
            assert!(borrowed.pending_gibs.is_empty(), "fish never queue chunks");
            assert!(!borrowed.gibs.contains_key(&fish[0]), "fish never keep a head");
        }
        // The 21-frame death runs its cry, then drops unsolid at the
        // tail. Each tick runs one monster pass, so poll per-tick like
        // the walker death proofs (coarse advances starve the frames).
        let mut unsolid = false;
        for _ in 0..300 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            if !behaviors.borrow().solids.contains(&fish[0]) {
                unsolid = true;
                break;
            }
        }
        assert!(unsolid, "the death tail drops unsolid");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        assert!(
            behaviors
                .borrow()
                .sounds
                .iter()
                .any(|sound| sound.sample == "fish/death.wav"),
            "deaths cry"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0145_knight_spawn_stands_armed() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let knights = live_knights(&world);
        assert_eq!(knights.len(), 5, "e1m2 spawns five knights");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(borrowed.total_monsters, LIVE_E1M2_NATIVE_SKILL2_TOTAL);
        for (slot, knight) in knights.iter().enumerate() {
            let monster = borrowed.monsters.get(knight).expect("knight record");
            assert_eq!(monster.flags & 512, 512, "dropped knights stand on ground");
            assert_eq!(monster.flags & 32, 32, "start_go flags the monster bit");
            assert_eq!(monster.takedamage, 2, "start_go arms DAMAGE_AIM");
            assert_eq!(monster.view_ofs, vec3(0.0, 0.0, 25.0));
            if slot == 0 {
                assert!(
                    matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::KnightWalk, _)),
                    "the t41 knight patrols, got {:?}",
                    monster.think
                );
            } else {
                assert!(
                    matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::KnightStand, _)),
                    "knight stands, got {:?}",
                    monster.think
                );
                assert!(monster.pausetime > 9999999.0, "targetless knights stand down");
            }
            let combat = world.server().simulation().combat_state(knight).expect("knight combat");
            assert_eq!(combat.health, 75.0);
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0145_knight_sword_wounds() {
        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_set_player_health(&mut world, 1000.0);
        let knights = live_knights(&world);
        // Offer the player to each stander in turn until one sights
        // and lands its standing sword (50 units sits inside the
        // 80-unit standing choice).
        let mut swordsman = None;
        for candidate in knights.iter().skip(1) {
            live_place_player_before(&mut world, candidate, 50.0);
            let mut cut = false;
            for _ in 0..300 {
                live_tick(&mut world);
                if live_player_health(&world) < 1000.0 {
                    cut = true;
                    break;
                }
            }
            if cut {
                swordsman = Some(candidate.clone());
                break;
            }
        }
        swordsman.expect("a knight sights, hunts, and lands its sword");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert!(
            borrowed.sounds.iter().any(|sound| sound.sample == "knight/sword1.wav"),
            "swords swish"
        );
        assert!(
            borrowed.sounds.iter().any(|sound| sound.sample == "knight/ksight.wav"),
            "sightings bark"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0145_knight_runattack_at_range() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_set_player_health(&mut world, 1000.0);
        let knights = live_knights(&world);
        // At 100 units the eye distance sits past the 80-unit
        // standing choice but inside melee range, so the first run
        // frame opens the running sword (the check runs before the
        // move, so the knight cannot close out of the window first).
        let mut runner = false;
        for candidate in knights.iter().skip(1) {
            live_place_player_before(&mut world, candidate, 100.0);
            for _ in 0..120 {
                live_tick(&mut world);
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                let think = behaviors.borrow().monsters.get(candidate).expect("knight record").think;
                if matches!(think, Q1MonsterThink::Frame(Q1MonsterSeq::KnightRunAttack, _)) {
                    runner = true;
                    break;
                }
            }
            if runner {
                break;
            }
        }
        assert!(runner, "the mid-range knight opens its running sword");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0145_knight_pain_then_dies() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let knights = live_knights(&world);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &knights[1], Some(&player), 5.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&knights[1]).expect("knight record");
            assert!(
                matches!(
                    monster.think,
                    Q1MonsterThink::Frame(Q1MonsterSeq::KnightPain, 0)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::KnightPainB, 0)
                ),
                "wounds run pain, got {:?}",
                monster.think
            );
            assert!(borrowed.sounds.iter().any(|sound| sound.sample == "knight/khurt.wav"));
        }
        live_damage(&mut world, &knights[1], Some(&player), 80.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&knights[1]).expect("knight record");
            assert!(monster.dead);
            assert!(
                matches!(
                    monster.think,
                    Q1MonsterThink::Frame(Q1MonsterSeq::KnightDie, 0)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::KnightDieB, 0)
                ),
                "death runs die, got {:?}",
                monster.think
            );
            assert_eq!(borrowed.killed_monsters, 1);
            assert!(borrowed.sounds.iter().any(|sound| sound.sample == "knight/kdeath.wav"));
        }
        // The third death frame drops unsolid.
        let mut unsolid = false;
        for _ in 0..120 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            if !behaviors.borrow().solids.contains(&knights[1]) {
                unsolid = true;
                break;
            }
        }
        assert!(unsolid, "the death drop goes unsolid");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0145_knight_patrol_walks_corners() {
        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let knights = live_knights(&world);
        let patrol = knights
            .iter()
            .find(|knight| {
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                let borrowed = behaviors.borrow();
                borrowed
                    .monsters
                    .get(knight)
                    .is_some_and(|monster| monster.movetarget.is_some())
            })
            .cloned()
            .expect("e1m2 routes a knight through corners");
        let start = live_monster_feet(&world, &patrol);
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
        for _ in 0..1200 {
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
        let end = live_monster_feet(&world, &patrol);
        let moved = ((end.x - start.x).powi(2) + (end.y - start.y).powi(2)).sqrt();
        assert!(moved > 10.0, "the patrol travels, moved {moved}");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0140_fiend_spawn_stands_armed() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let fiends = live_fiends(&world);
        assert_eq!(fiends.len(), 3, "e1m2 spawns three fiends");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(borrowed.total_monsters, LIVE_E1M2_NATIVE_SKILL2_TOTAL);
        for fiend in &fiends {
            let monster = borrowed.monsters.get(fiend).expect("fiend record");
            assert_eq!(monster.flags & 512, 512, "dropped fiends stand on ground");
            assert_eq!(monster.flags & 32, 32, "start_go flags the monster bit");
            assert_eq!(monster.takedamage, 2, "start_go arms DAMAGE_AIM");
            assert_eq!(monster.view_ofs, vec3(0.0, 0.0, 25.0));
            assert!(
                matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::FiendStand, _)),
                "fiend stands, got {:?}",
                monster.think
            );
            assert!(monster.pausetime > 9999999.0, "targetless fiends stand down");
            let combat = world.server().simulation().combat_state(fiend).expect("fiend combat");
            assert_eq!(combat.health, 300.0);
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0140_fiend_claw_wounds() {
        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_set_player_health(&mut world, 1000.0);
        let fiends = live_fiends(&world);
        let mut raker = None;
        for candidate in fiends.iter() {
            live_place_player_before(&mut world, candidate, 50.0);
            let mut cut = false;
            for _ in 0..300 {
                live_tick(&mut world);
                if live_player_health(&world) < 1000.0 {
                    cut = true;
                    break;
                }
            }
            if cut {
                raker = Some(candidate.clone());
                break;
            }
        }
        raker.expect("a fiend sights, hunts, and lands its claw");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert!(
            borrowed.sounds.iter().any(|sound| sound.sample == "demon/dhit2.wav"),
            "claws thud"
        );
        assert!(
            borrowed.sounds.iter().any(|sound| sound.sample == "demon/sight2.wav"),
            "sightings bark"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0140_fiend_leaps_and_lands() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink, Q1MonsterTouch};

        let Some(mut world) = live_q1_world("maps/e1m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_set_player_health(&mut world, 1000.0);
        let fiends = live_fiends(&world);
        let mut leapt = false;
        let mut landed = false;
        // Offer the player at leap distance to each fiend until one
        // jumps (150 units sits inside the 100-200 leap window; the
        // e1m2 fiends face walls inside 100 units, so the leap runs
        // on e1m3's open hall).
        'offer: for candidate in fiends.iter() {
            live_place_player_before(&mut world, candidate, 150.0);
            for _ in 0..200 {
                live_tick(&mut world);
                let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                let borrowed = behaviors.borrow();
                let monster = borrowed.monsters.get(candidate).expect("fiend record");
                if matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::FiendJump, _))
                    || monster.touch == Q1MonsterTouch::FiendJumpTouch
                {
                    leapt = true;
                }
                // Landing runs the leap tail, then the hunt; a fresh
                // leap windup (leap frame, touch not yet set) does
                // not count.
                if leapt
                    && matches!(
                        monster.think,
                        Q1MonsterThink::Frame(Q1MonsterSeq::FiendRun, _)
                            | Q1MonsterThink::Frame(Q1MonsterSeq::FiendAttack, _)
                    )
                    && monster.flags & 512 == 512
                    && monster.touch == Q1MonsterTouch::None
                {
                    landed = true;
                    break 'offer;
                }
            }
        }
        assert!(leapt, "the fiend leaps at jump distance");
        assert!(landed, "the leap lands back into the hunt");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        assert!(
            behaviors
                .borrow()
                .sounds
                .iter()
                .any(|sound| sound.sample == "demon/djump.wav"),
            "leaps cry"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0140_fiend_pain_then_dies() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let fiends = live_fiends(&world);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &fiends[0], Some(&player), 5.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&fiends[0]).expect("fiend record");
            // Light hits bark and hold whether or not they flinch.
            assert!(borrowed.sounds.iter().any(|sound| sound.sample == "demon/dpain1.wav"));
            assert!(monster.pain_finished > 0.0, "pains hold a second");
            assert!(
                matches!(
                    monster.think,
                    Q1MonsterThink::Frame(Q1MonsterSeq::FiendStand, _)
                        | Q1MonsterThink::Frame(Q1MonsterSeq::FiendPain, 0)
                ),
                "wounds bark, flinch or not, got {:?}",
                monster.think
            );
        }
        live_damage(&mut world, &fiends[0], Some(&player), 310.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&fiends[0]).expect("fiend record");
            assert!(monster.dead);
            assert_eq!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::FiendDie, 0));
            assert_eq!(borrowed.killed_monsters, 1);
        }
        // The cry plays in the first death frame; the sixth drops unsolid.
        let mut unsolid = false;
        for _ in 0..120 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            if !behaviors.borrow().solids.contains(&fiends[0]) {
                unsolid = true;
                break;
            }
        }
        assert!(unsolid, "the death drop goes unsolid");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        assert!(
            behaviors
                .borrow()
                .sounds
                .iter()
                .any(|sound| sound.sample == "demon/ddeath.wav"),
            "deaths cry"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0140_fiend_gib_bursts() {
        let Some(mut world) = live_q1_world("maps/e1m2.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let fiends = live_fiends(&world);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &fiends[1], Some(&player), 400.0);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert!(borrowed.monsters.get(&fiends[1]).expect("fiend record").dead);
        assert_eq!(borrowed.killed_monsters, 1);
        assert!(borrowed.sounds.iter().any(|sound| sound.sample == "player/udeath.wav"));
        assert!(borrowed.gibs.contains_key(&fiends[1]), "the head keeps the actor");
        assert_eq!(borrowed.pending_gibs.len(), 3, "three chunks queue");
        assert!(!borrowed.solids.contains(&fiends[1]), "gibs go unsolid");
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0150_shambler_spawn_stands_armed() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let shamblers = live_shamblers(&world);
        assert_eq!(shamblers.len(), 3, "e1m3 spawns three shamblers");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert_eq!(borrowed.total_monsters, LIVE_E1M3_NATIVE_SKILL2_TOTAL);
        for shambler in &shamblers {
            let monster = borrowed.monsters.get(shambler).expect("shambler record");
            assert_eq!(monster.flags & 512, 512, "dropped shamblers stand on ground");
            assert_eq!(monster.flags & 32, 32, "start_go flags the monster bit");
            assert_eq!(monster.takedamage, 2, "start_go arms DAMAGE_AIM");
            assert_eq!(monster.view_ofs, vec3(0.0, 0.0, 25.0));
            assert!(
                matches!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::ShamStand, _)),
                "shambler stands, got {:?}",
                monster.think
            );
            assert!(monster.pausetime > 9999999.0, "targetless shamblers stand down");
            let combat = world
                .server()
                .simulation()
                .combat_state(shambler)
                .expect("shambler combat");
            assert_eq!(combat.health, 600.0);
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0150_shambler_smash_wounds() {
        let Some(mut world) = live_q1_world("maps/e1m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        live_set_player_health(&mut world, 1000.0);
        let shamblers = live_shamblers(&world);
        let mut smasher = None;
        for candidate in shamblers.iter() {
            live_place_player_before(&mut world, candidate, 50.0);
            let mut cut = false;
            for _ in 0..300 {
                live_tick(&mut world);
                if live_player_health(&world) < 1000.0 {
                    cut = true;
                    break;
                }
            }
            if cut {
                smasher = Some(candidate.clone());
                break;
            }
        }
        smasher.expect("a shambler sights, hunts, and lands its smash");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert!(
            borrowed.sounds.iter().any(|sound| sound.sample == "shambler/smack.wav"),
            "smashes smack"
        );
        assert!(
            borrowed
                .sounds
                .iter()
                .any(|sound| sound.sample == "shambler/melee1.wav" || sound.sample == "shambler/melee2.wav"),
            "strokes bark their wind-up"
        );
        assert!(
            borrowed
                .sounds
                .iter()
                .any(|sound| sound.sample == "shambler/ssight.wav"),
            "sightings bark"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0150_shambler_casts_lightning() {
        use super::super::simulation::native_q1_weapons::Q1TempEnt;

        let Some(mut world) = live_q1_world("maps/e1m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_silence_door_fields(&mut world);
        live_advance(&mut world, 1.0);
        let shamblers = live_shamblers(&world);
        let mut caster = None;
        let mut ball_seen = false;
        // Offer the player at cast distance to each shambler until one
        // fires its bolt: far enough to pick the missile stroke over
        // the melee, near enough to hold a clear shot.
        'offer: for candidate in shamblers.iter() {
            for dist in [500.0, 350.0, 250.0] {
                live_set_player_health(&mut world, 1000.0);
                live_place_player_before(&mut world, candidate, dist);
                for _ in 0..300 {
                    live_tick(&mut world);
                    let behaviors = world.q1_behaviors().expect("Q1 behaviors");
                    let borrowed = behaviors.borrow();
                    ball_seen |= borrowed.sham_balls.iter().any(|ball| ball.shambler == *candidate);
                    let fired = borrowed
                        .temp_ents
                        .iter()
                        .any(|ent| matches!(ent, Q1TempEnt::Lightning { entity, .. } if entity == candidate));
                    if fired {
                        caster = Some(candidate.clone());
                        break 'offer;
                    }
                }
            }
        }
        let caster = caster.expect("a shambler casts its lightning");
        assert!(ball_seen, "the cast charges its ball first");
        // `ShamCheckAttack` holds the next cast 2-4 s out (the generic
        // check never latches a hold).
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let hold = behaviors
            .borrow()
            .monsters
            .get(&caster)
            .expect("caster record")
            .attack_finished;
        assert!(hold > live_now(&world), "casts latch the refire hold");
        // The first bolt pops the ball in its own frame; read this
        // before the wound window in case the shambler re-casts.
        let ball_popped = {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            borrowed.sham_balls.iter().all(|ball| ball.shambler != caster)
        };
        assert!(ball_popped, "the first bolt pops the charge ball");
        // The bolt wounds within its own window.
        let mut wounded = live_player_health(&world) < 1000.0;
        for _ in 0..40 {
            if wounded {
                break;
            }
            live_tick(&mut world);
            wounded = live_player_health(&world) < 1000.0;
        }
        assert!(wounded, "the cast bolt wounds the player");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert!(
            borrowed
                .sounds
                .iter()
                .any(|sound| sound.sample == "shambler/sattck1.wav"),
            "casts charge"
        );
        assert!(
            borrowed.sounds.iter().any(|sound| sound.sample == "shambler/sboom.wav"),
            "bolts boom"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0150_shambler_pain_then_dies() {
        use super::super::simulation::native_q1_monsters::{Q1MonsterSeq, Q1MonsterThink};

        let Some(mut world) = live_q1_world("maps/e1m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let shamblers = live_shamblers(&world);
        let player = world.player_actor().cloned().expect("player");
        // A crushing hit always flinches (`random * 400 <= 400`) and
        // latches the 2 s hold.
        live_damage(&mut world, &shamblers[0], Some(&player), 400.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&shamblers[0]).expect("shambler record");
            assert!(borrowed
                .sounds
                .iter()
                .any(|sound| sound.sample == "shambler/shurt2.wav"));
            assert!(monster.pain_finished > 0.0, "pains hold two seconds");
            assert_eq!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::ShamPain, 0));
        }
        live_damage(&mut world, &shamblers[0], Some(&player), 200.0);
        {
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            let borrowed = behaviors.borrow();
            let monster = borrowed.monsters.get(&shamblers[0]).expect("shambler record");
            assert!(monster.dead);
            assert_eq!(monster.think, Q1MonsterThink::Frame(Q1MonsterSeq::ShamDie, 0));
            assert_eq!(borrowed.killed_monsters, 1);
        }
        // The third death frame drops unsolid.
        let mut unsolid = false;
        for _ in 0..120 {
            live_tick(&mut world);
            let behaviors = world.q1_behaviors().expect("Q1 behaviors");
            if !behaviors.borrow().solids.contains(&shamblers[0]) {
                unsolid = true;
                break;
            }
        }
        assert!(unsolid, "the death drop goes unsolid");
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        assert!(
            behaviors
                .borrow()
                .sounds
                .iter()
                .any(|sound| sound.sample == "shambler/sdeath.wav"),
            "deaths cry"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn live_q1_0150_shambler_gib_bursts() {
        let Some(mut world) = live_q1_world("maps/e1m3.bsp", GameMode::Singleplayer, 2) else {
            return;
        };
        live_advance(&mut world, 1.0);
        let shamblers = live_shamblers(&world);
        let player = world.player_actor().cloned().expect("player");
        live_damage(&mut world, &shamblers[1], Some(&player), 700.0);
        let behaviors = world.q1_behaviors().expect("Q1 behaviors");
        let borrowed = behaviors.borrow();
        assert!(borrowed.monsters.get(&shamblers[1]).expect("shambler record").dead);
        assert_eq!(borrowed.killed_monsters, 1);
        assert!(borrowed.sounds.iter().any(|sound| sound.sample == "player/udeath.wav"));
        assert!(borrowed.gibs.contains_key(&shamblers[1]), "the head keeps the actor");
        assert_eq!(borrowed.pending_gibs.len(), 3, "three chunks queue");
        assert!(!borrowed.solids.contains(&shamblers[1]), "gibs go unsolid");
    }
}
