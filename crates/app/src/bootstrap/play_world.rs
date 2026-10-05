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
    admit_player, build_clip, eye_height_for_family, movement_content_edition, movement_dialect_for_selection,
    provider_for_product, PlayerBody, PlayerClip,
};
use super::windowed_scene::{build_presentation, open_product_mounts, select_spawn, PlayPresentation};
use crate::options::ApplicationOptions;
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
        let (Some(player), Some(clip)) = (self.player.as_mut(), self.clip.as_ref()) else {
            return Ok(());
        };
        let (simulation, triggers) = self.server.simulation_and_triggers();
        player
            .step(simulation, triggers, clip, command)
            .map_err(|reason| PlayWorldError::Play {
                map: self.map.clone(),
                reason,
            })
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

/// Decode one map's entity records as ordered key/value property lists.
///
/// Quake II entity strings share the Quake brace syntax, so they parse
/// with the Quake reader, matching the donor (`native-q2-map.ts` parses
/// `world.entities` with `parseQ1Entities`).
pub(crate) fn decode_map_entities(
    bytes: &[u8],
    map: &str,
    kind: BspKind,
) -> Result<Vec<Vec<(String, String)>>, PlayWorldError> {
    match kind {
        BspKind::Q1 => {
            let parsed =
                read_q1_bsp(bytes, map, Q1BspOptions::default()).map_err(|error| PlayWorldError::MapDecode {
                    map: map.to_string(),
                    reason: error.to_string(),
                })?;
            Ok(parsed.entity_list.into_iter().map(|entity| entity.properties).collect())
        }
        BspKind::Q2 => {
            let parsed = read_q2_bsp(bytes, map).map_err(|error| PlayWorldError::MapDecode {
                map: map.to_string(),
                reason: error.to_string(),
            })?;
            parse_q1_entities(&parsed.entities, &format!("{map}:entities"))
                .map(|entities| entities.into_iter().map(|entity| entity.properties).collect())
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
                .map(|entities| entities.into_iter().map(|entity| entity.properties).collect())
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

/// Spawn parsed entity records into a server.
///
/// Classnames outside the registry gain a generic `map:{classname}`
/// function first (deterministic order); records that fail field parse or
/// spawn are collected as skips instead of aborting the load.
pub fn spawn_map_entities(
    server: &mut Server<GuestServerLogic>,
    entities: &[Vec<(String, String)>],
    source: &str,
) -> MapSpawnSummary {
    let mut summary = MapSpawnSummary::default();
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
        match server.spawn_entity(&fields) {
            Ok(_) => summary.spawned += 1,
            Err(error) => summary.skipped.push(SkippedEntity {
                index,
                classname,
                reason: format!("{source}: spawn failed: {error}"),
            }),
        }
    }
    summary
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
    let bytes = mounts
        .read(qa_content::mounts::ResourceRef::Path(&options.map))
        .map_err(|error| PlayWorldError::MapUnread {
            content: content.clone(),
            map: options.map.clone(),
            reason: error.to_string(),
        })?;
    let kind = classify_bsp(&bytes, &options.map).map_err(|error| PlayWorldError::MapDecode {
        map: options.map.clone(),
        reason: error.to_string(),
    })?;
    let entities = decode_map_entities(&bytes, &options.map, kind)?;
    let sound_family = match kind {
        BspKind::Q1 => SoundFamily::Q1,
        BspKind::Q2 => SoundFamily::Q2,
        BspKind::Q3 => SoundFamily::Q3,
    };
    let speakers = map_speakers(&entities, sound_family);
    let mut server = open_server(config).map_err(|error| PlayWorldError::Server(error.to_string()))?;
    let summary = spawn_map_entities(&mut server, &entities, &options.map);
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
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use qa_content::catalog::DiscoverContentOptions;

    use super::*;

    fn steel_corpus_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target")
    }

    fn steel_catalog() -> Option<InstalledCatalog> {
        let root = steel_corpus_root();
        if !root.join("q1").is_dir() && !root.join("q2").is_dir() && !root.join("q3a").is_dir() {
            eprintln!("skipped: Steel corpus root {} has no game data", root.display());
            return None;
        }
        match qa_content::catalog::discover_installed_content(&DiscoverContentOptions::new(root.clone())) {
            Ok(catalog) => Some(catalog),
            Err(error) => {
                eprintln!("skipped: Steel catalog discovery failed at {}: {error}", root.display());
                None
            }
        }
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
        let summary = spawn_map_entities(&mut server, &entities, "maps/test.bsp");
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
        let summary = spawn_map_entities(&mut server, &entities, "maps/test.bsp");
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
    fn live_steel_maps_spawn_more_than_stub() {
        let Some(catalog) = steel_catalog() else {
            eprintln!("skipped: Steel corpus root has no game data");
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
                    eprintln!("skipped: {product} {map}: {error}");
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
            assert_eq!(world.entity_count(), world.spawned() + players);
            loaded += 1;
        }
        assert!(loaded > 0, "expected at least one Steel map to load");
    }

    #[test]
    fn live_q1_world_admits_player_and_eye_follows_steps() {
        use qa_world::movement::types::{Q1UserCommand, UserCommand};

        let Some(catalog) = steel_catalog() else {
            eprintln!("skipped: Steel corpus root has no game data");
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
                eprintln!("skipped: q1-classic-id1 maps/start.bsp: {error}");
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
                eprintln!("skipped: {product} {map}: {error}");
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
    fn live_steel_q3_presentation_prepares_draw_batches() {
        if steel_presentation_batches("q3-baseq3", "maps/q3dm1.bsp").is_none() {
            eprintln!("skipped: Steel corpus root has no game data");
        }
    }

    #[test]
    fn live_steel_q3_entities_submit_model_batches() {
        use qa_client::render::types::SourceTime;
        use qa_client::view::perspective_projection;
        use qa_core::math::{angles_to_axis, normalize3, sub3, vector_to_angles};

        let Some(catalog) = steel_catalog() else {
            eprintln!("skipped: Steel corpus root has no game data");
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
                eprintln!("skipped: q3-baseq3 maps/q3dm1.bsp: {error}");
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
    fn live_steel_q3_entity_batches_bind_loaded_skins() {
        use qa_client::render::types::{ImageSource, SourceTime, TextureBinding};
        use qa_client::view::perspective_projection;
        use qa_core::math::{add3, angles_to_axis, normalize3, sub3, vec3, vector_to_angles};

        let Some(catalog) = steel_catalog() else {
            eprintln!("skipped: Steel corpus root has no game data");
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
                eprintln!("skipped: q3-baseq3 maps/q3dm1.bsp: {error}");
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
    fn live_steel_q1_presentation_prepares_draw_batches() {
        if steel_presentation_batches("q1-classic-id1", "maps/e1m1.bsp").is_none() {
            eprintln!("skipped: Steel corpus root has no game data");
        }
    }

    #[test]
    fn live_steel_q2_presentation_prepares_draw_batches() {
        if steel_presentation_batches("q2-classic-baseq2", "maps/base1.bsp").is_none() {
            eprintln!("skipped: Steel corpus root has no game data");
        }
    }
}
