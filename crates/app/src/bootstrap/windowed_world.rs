//! Windowed map world: the selected map's real entities in a live server.
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
//! are recorded in [`WindowedWorld::skipped`] instead of aborting the load.

use std::collections::BTreeSet;

use qa_content::bsp::{parse_q1_entities, read_q1_bsp, Q1BspOptions};
use qa_content::bsp2::read_q2_bsp;
use qa_content::bsp3::{parse_q3_bsp, parse_q3_entities};
use qa_content::catalog::InstalledCatalog;
use qa_content::{classify_bsp, BspKind};
use qa_guest::server::GuestServerLogic;
use qa_world::server::Server;
use qa_world::spawn::{SpawnFields, SpawnRequest};
use thiserror::Error;

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
pub enum WindowedWorldError {
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
}

/// A live server holding one map's real entities.
pub struct WindowedWorld {
    server: Server<GuestServerLogic>,
    content: String,
    map: String,
    entity_records: usize,
    spawned: usize,
    skipped: Vec<SkippedEntity>,
}

impl std::fmt::Debug for WindowedWorld {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowedWorld")
            .field("content", &self.content)
            .field("map", &self.map)
            .field("entity_records", &self.entity_records)
            .field("spawned", &self.spawned)
            .field("skipped", &self.skipped)
            .field("entity_count", &self.entity_count())
            .finish()
    }
}

impl WindowedWorld {
    /// Borrow the live server.
    #[must_use]
    pub fn server(&self) -> &Server<GuestServerLogic> {
        &self.server
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
fn decode_map_entities(bytes: &[u8], map: &str) -> Result<Vec<Vec<(String, String)>>, WindowedWorldError> {
    let kind = classify_bsp(bytes, map).map_err(|error| WindowedWorldError::MapDecode {
        map: map.to_string(),
        reason: error.to_string(),
    })?;
    match kind {
        BspKind::Q1 => {
            let parsed =
                read_q1_bsp(bytes, map, Q1BspOptions::default()).map_err(|error| WindowedWorldError::MapDecode {
                    map: map.to_string(),
                    reason: error.to_string(),
                })?;
            Ok(parsed.entity_list.into_iter().map(|entity| entity.properties).collect())
        }
        BspKind::Q2 => {
            let parsed = read_q2_bsp(bytes, map).map_err(|error| WindowedWorldError::MapDecode {
                map: map.to_string(),
                reason: error.to_string(),
            })?;
            parse_q1_entities(&parsed.entities, &format!("{map}:entities"))
                .map(|entities| entities.into_iter().map(|entity| entity.properties).collect())
                .map_err(|error| WindowedWorldError::EntityParse {
                    map: map.to_string(),
                    reason: error.to_string(),
                })
        }
        BspKind::Q3 => {
            let parsed = parse_q3_bsp(bytes, map).map_err(|error| WindowedWorldError::MapDecode {
                map: map.to_string(),
                reason: error.to_string(),
            })?;
            parse_q3_entities(&parsed.entities, &format!("{map}:entities"))
                .map(|entities| entities.into_iter().map(|entity| entity.properties).collect())
                .map_err(|error| WindowedWorldError::EntityParse {
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
/// spawns every record. Fails honestly when the product is unknown, the
/// map is unreadable or undecodable, or no record spawns.
pub fn load_windowed_world(
    config: &StartupConfig,
    catalog: &InstalledCatalog,
    options: &ApplicationOptions,
) -> Result<WindowedWorld, WindowedWorldError> {
    let content = map_content_id(options).to_string();
    if catalog.require(&content).is_err() {
        return Err(WindowedWorldError::UnknownProduct(content));
    }
    let bytes = catalog
        .read(&content, &options.map)
        .map_err(|error| WindowedWorldError::MapUnread {
            content: content.clone(),
            map: options.map.clone(),
            reason: error.to_string(),
        })?;
    let entities = decode_map_entities(&bytes, &options.map)?;
    let mut server = open_server(config).map_err(|error| WindowedWorldError::Server(error.to_string()))?;
    let summary = spawn_map_entities(&mut server, &entities, &options.map);
    if summary.spawned == 0 {
        let reason = summary.skipped.first().map_or_else(
            || "map has no entity records".to_string(),
            |skipped| skipped.reason.clone(),
        );
        return Err(WindowedWorldError::NothingSpawned {
            map: options.map.clone(),
            records: entities.len(),
            reason,
        });
    }
    Ok(WindowedWorld {
        server,
        content,
        map: options.map.clone(),
        entity_records: entities.len(),
        spawned: summary.spawned,
        skipped: summary.skipped,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use qa_content::catalog::DiscoverContentOptions;

    use super::*;

    fn steel_corpus_root() -> PathBuf {
        PathBuf::from("/home/buzzkill/Projects/qa-muse/target")
    }

    fn steel_catalog() -> Option<InstalledCatalog> {
        let root = steel_corpus_root();
        if !root.join("q1").is_dir() && !root.join("q2").is_dir() && !root.join("q3a").is_dir() {
            return None;
        }
        qa_content::catalog::discover_installed_content(&DiscoverContentOptions::new(root)).ok()
    }

    fn test_config(options: &ApplicationOptions) -> StartupConfig {
        StartupConfig::from_options(options).unwrap()
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
        let error = load_windowed_world(&config, &catalog, &options).unwrap_err();
        let message = error.to_string();
        assert!(
            matches!(
                error,
                WindowedWorldError::UnknownProduct(_) | WindowedWorldError::MapUnread { .. }
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
            let world = match load_windowed_world(&config, &catalog, &options) {
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
            assert_eq!(world.entity_count(), world.spawned());
            loaded += 1;
        }
        assert!(loaded > 0, "expected at least one Steel map to load");
    }
}
