//! Campaign unit world ownership.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/campaign-unit.ts`
//! (`CampaignUnit`).
//! A unit owns departed worlds; the active world remains owned by the
//! simulation. The donor operates on the full `SaveImage`; following the
//! [`ComponentClientImage`](super::component_client_save::ComponentClientImage)
//! precedent this module defines the minimal [`CampaignUnitImage`] view
//! (provider rows, map location, source provider, opaque world payload)
//! with a versioned roundtrip codec, preserving every staging, revision,
//! and validation rule.

use std::collections::HashMap;

use qa_content::contract::{is_content_id, ContentId, ResourceRequest};
use qa_world::save::ownership::{campaign_unit_contract, validate_save_provider_owner, ProviderCheckpoint};
use qa_world::save::value::{decode_checkpoint_value, encode_checkpoint_value, SaveJson, SaveReader};
use qa_world::WorldError;
use thiserror::Error;

/// Campaign unit failure.
#[derive(Debug, Error)]
pub enum CampaignUnitError {
    /// More than one campaign checkpoint row matched.
    #[error("Duplicate campaign unit checkpoint")]
    Duplicate,
    /// Departure map does not match the active map.
    #[error("Campaign departure does not match the active map")]
    DepartureMismatch,
    /// Visit was superseded by a newer staging.
    #[error("Campaign visit was superseded")]
    Superseded,
    /// Save map does not match the active map.
    #[error("Campaign save does not match the active map")]
    SaveMismatch,
    /// Checkpoint map does not match its world.
    #[error("Campaign checkpoint active map does not match its world")]
    CheckpointMismatch,
    /// Stored world checkpoint is invalid.
    #[error("Invalid campaign unit world checkpoint")]
    InvalidWorld,
    /// Campaign content identity is invalid.
    #[error("Invalid campaign content identity")]
    InvalidContent,
    /// Campaign image codec failure.
    #[error("Invalid campaign unit image: {0}")]
    InvalidImage(String),
    /// Save codec or ownership failure.
    #[error(transparent)]
    World(#[from] WorldError),
}

/// Minimal save-image view: provider rows, map location, source provider,
/// and the opaque world payload.
#[derive(Debug, Clone, PartialEq)]
pub struct CampaignUnitImage {
    /// Provider checkpoint rows.
    pub providers: Vec<ProviderCheckpoint>,
    /// Map geometry content (donor `recipe.map.geometryContent`).
    pub content: ContentId,
    /// Requested geometry path (donor `recipe.map.geometry.requestedPath`).
    pub path: String,
    /// Map entities provider (donor `recipe.map.entities.provider`).
    pub source_provider: String,
    /// Opaque world payload (the image minus campaign rows).
    pub world: Vec<u8>,
}

impl CampaignUnitImage {
    /// Map location (donor `location(image)`).
    #[must_use]
    pub fn location(&self) -> ResourceRequest {
        ResourceRequest {
            content: self.content.clone(),
            path: self.path.clone(),
        }
    }
}

fn strip_map_path(value: &str) -> &str {
    value
        .strip_prefix("maps/")
        .unwrap_or(value)
        .strip_suffix(".bsp")
        .unwrap_or_else(|| value.strip_prefix("maps/").unwrap_or(value))
}

fn location_key(map: &ResourceRequest) -> String {
    format!("{}/{}", map.content.as_str(), strip_map_path(&map.path))
}

fn campaign_entries(image: &CampaignUnitImage) -> Result<Vec<&ProviderCheckpoint>, CampaignUnitError> {
    let contract = campaign_unit_contract();
    let entries: Vec<&ProviderCheckpoint> = image
        .providers
        .iter()
        .filter(|entry| entry.provider == contract.provider || entry.schema == contract.schema)
        .collect();
    if entries.len() > 1 {
        return Err(CampaignUnitError::Duplicate);
    }
    for entry in entries.iter().copied() {
        validate_save_provider_owner(entry, &image.source_provider)?;
    }
    Ok(entries)
}

fn world_only(image: &CampaignUnitImage) -> Result<CampaignUnitImage, CampaignUnitError> {
    campaign_entries(image)?;
    let contract = campaign_unit_contract();
    Ok(CampaignUnitImage {
        providers: image
            .providers
            .iter()
            .filter(|entry| entry.schema != contract.schema)
            .cloned()
            .collect(),
        content: image.content.clone(),
        path: image.path.clone(),
        source_provider: image.source_provider.clone(),
        world: image.world.clone(),
    })
}

fn write_provider(row: &ProviderCheckpoint) -> SaveJson {
    SaveJson::Object(vec![
        ("provider".to_string(), SaveJson::String(row.provider.clone())),
        ("schema".to_string(), SaveJson::String(row.schema.clone())),
        ("version".to_string(), SaveJson::Number(row.version as f64)),
        ("bytes".to_string(), SaveJson::Bytes(row.bytes.clone())),
    ])
}

fn read_provider(reader: SaveReader) -> Result<ProviderCheckpoint, CampaignUnitError> {
    Ok(ProviderCheckpoint {
        provider: reader.field("provider").string().map_err(CampaignUnitError::World)?,
        schema: reader.field("schema").string().map_err(CampaignUnitError::World)?,
        version: reader.field("version").integer(0).map_err(CampaignUnitError::World)?,
        bytes: reader.field("bytes").bytes().map_err(CampaignUnitError::World)?,
    })
}

/// Encode a campaign image view with framing.
#[must_use]
pub fn encode_campaign_image(image: &CampaignUnitImage) -> Vec<u8> {
    encode_checkpoint_value(&SaveJson::Object(vec![
        ("version".to_string(), SaveJson::Number(1.0)),
        (
            "providers".to_string(),
            SaveJson::Array(image.providers.iter().map(write_provider).collect()),
        ),
        (
            "content".to_string(),
            SaveJson::String(image.content.as_str().to_string()),
        ),
        ("path".to_string(), SaveJson::String(image.path.clone())),
        (
            "sourceProvider".to_string(),
            SaveJson::String(image.source_provider.clone()),
        ),
        ("world".to_string(), SaveJson::Bytes(image.world.clone())),
    ]))
}

/// Decode a campaign image view.
pub fn decode_campaign_image(bytes: &[u8]) -> Result<CampaignUnitImage, CampaignUnitError> {
    let value = decode_checkpoint_value(bytes).map_err(CampaignUnitError::World)?;
    let reader = SaveReader::new(&value);
    reader
        .field("version")
        .literal_i64(1)
        .map_err(CampaignUnitError::World)?;
    let providers = reader
        .field("providers")
        .list::<ProviderCheckpoint, CampaignUnitError>(read_provider)?;
    let content = reader.field("content").string().map_err(CampaignUnitError::World)?;
    if !is_content_id(&content) {
        return Err(CampaignUnitError::InvalidContent);
    }
    Ok(CampaignUnitImage {
        providers,
        content: ContentId(content),
        path: reader.field("path").string().map_err(CampaignUnitError::World)?,
        source_provider: reader
            .field("sourceProvider")
            .string()
            .map_err(CampaignUnitError::World)?,
        world: reader.field("world").bytes().map_err(CampaignUnitError::World)?,
    })
}

/// One staged visit (`CampaignUnitVisit`): publish only after the
/// destination world has been admitted successfully.
pub struct StagedCampaignVisit {
    /// Saved destination world, if any.
    pub restore: Option<CampaignUnitImage>,
    revision: u64,
    current: ResourceRequest,
    worlds: HashMap<String, Vec<u8>>,
}

impl StagedCampaignVisit {
    /// Publish the staged visit (`commit`).
    pub fn commit(self, unit: &mut CampaignUnit) -> Result<(), CampaignUnitError> {
        if self.revision != unit.revision {
            return Err(CampaignUnitError::Superseded);
        }
        unit.current = Some(self.current);
        unit.worlds = self.worlds;
        unit.revision += 1;
        Ok(())
    }
}

/// Campaign unit world ownership (`CampaignUnit`).
#[derive(Debug, Default)]
pub struct CampaignUnit {
    current: Option<ResourceRequest>,
    worlds: HashMap<String, Vec<u8>>,
    revision: u64,
}

impl CampaignUnit {
    /// Empty unit.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage a visit to a destination (`stage`).
    pub fn stage(
        &self,
        destination: &ResourceRequest,
        new_unit: bool,
        departure: Option<&CampaignUnitImage>,
    ) -> Result<StagedCampaignVisit, CampaignUnitError> {
        if let (Some(departure), Some(current)) = (departure, &self.current) {
            if location_key(&departure.location()) != location_key(current) {
                return Err(CampaignUnitError::DepartureMismatch);
            }
        }
        let reset = new_unit
            || self
                .current
                .as_ref()
                .is_some_and(|current| current.content != destination.content);
        let mut worlds = if reset { HashMap::new() } else { self.worlds.clone() };
        if !reset {
            if let Some(departure) = departure {
                let stored = world_only(departure)?;
                worlds.insert(location_key(&departure.location()), encode_campaign_image(&stored));
            }
        }
        let saved = worlds.get(&location_key(destination));
        let restore = saved.map(|bytes| decode_campaign_image(bytes)).transpose()?;
        worlds.remove(&location_key(destination));
        Ok(StagedCampaignVisit {
            restore,
            revision: self.revision,
            current: destination.clone(),
            worlds,
        })
    }

    /// Checkpoint the unit (`checkpoint`).
    pub fn checkpoint(&self, current: Option<&ResourceRequest>) -> Result<ProviderCheckpoint, CampaignUnitError> {
        let contract = campaign_unit_contract();
        let current = current.map_or_else(
            || match &self.current {
                Some(current) => SaveJson::Object(vec![
                    (
                        "content".to_string(),
                        SaveJson::String(current.content.as_str().to_string()),
                    ),
                    ("path".to_string(), SaveJson::String(current.path.clone())),
                ]),
                None => SaveJson::Null,
            },
            |current| {
                SaveJson::Object(vec![
                    (
                        "content".to_string(),
                        SaveJson::String(current.content.as_str().to_string()),
                    ),
                    ("path".to_string(), SaveJson::String(current.path.clone())),
                ])
            },
        );
        let mut worlds: Vec<(&String, &Vec<u8>)> = self.worlds.iter().collect();
        worlds.sort_by(|a, b| a.0.cmp(b.0));
        Ok(ProviderCheckpoint {
            provider: contract.provider,
            schema: contract.schema,
            version: contract.version,
            bytes: encode_checkpoint_value(&SaveJson::Object(vec![
                ("current".to_string(), current),
                (
                    "worlds".to_string(),
                    SaveJson::Array(
                        worlds
                            .iter()
                            .map(|(map, bytes)| {
                                SaveJson::Object(vec![
                                    ("map".to_string(), SaveJson::String((*map).clone())),
                                    ("bytes".to_string(), SaveJson::Bytes((*bytes).clone())),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ])),
        })
    }

    /// Attach the unit checkpoint to a save (`attach`).
    pub fn attach(&self, image: &CampaignUnitImage) -> Result<CampaignUnitImage, CampaignUnitError> {
        let current = image.location();
        let mut world = world_only(image)?;
        if let Some(active) = &self.current {
            if location_key(active) != location_key(&current) {
                return Err(CampaignUnitError::SaveMismatch);
            }
        }
        world.providers.push(self.checkpoint(Some(&current))?);
        Ok(world)
    }

    /// Restore the unit from a save (`restore`).
    pub fn restore(&mut self, image: &CampaignUnitImage) -> Result<(), CampaignUnitError> {
        let entries = campaign_entries(image)?;
        let checkpoint = entries.first().copied().cloned();
        let mut current = Some(image.location());
        let mut worlds = HashMap::new();
        if let Some(checkpoint) = checkpoint {
            let value = decode_checkpoint_value(&checkpoint.bytes).map_err(CampaignUnitError::World)?;
            let reader = SaveReader::new(&value);
            let saved: Option<ResourceRequest> = reader
                .field("current")
                .nullable::<ResourceRequest, CampaignUnitError>(|entry| {
                    let content = entry.field("content").string().map_err(CampaignUnitError::World)?;
                    if !is_content_id(&content) {
                        return Err(CampaignUnitError::InvalidContent);
                    }
                    Ok(ResourceRequest {
                        content: ContentId(content),
                        path: entry.field("path").string().map_err(CampaignUnitError::World)?,
                    })
                })?;
            current = saved;
            let matches = current
                .as_ref()
                .is_some_and(|current| location_key(current) == location_key(&image.location()));
            if !matches {
                return Err(CampaignUnitError::CheckpointMismatch);
            }
            let active = current.clone().expect("checkpoint current checked");
            let stored = reader
                .field("worlds")
                .list::<(String, Vec<u8>), CampaignUnitError>(|value| {
                    Ok((
                        value.field("map").string().map_err(CampaignUnitError::World)?,
                        value.field("bytes").bytes().map_err(CampaignUnitError::World)?,
                    ))
                })?;
            for (map, bytes) in stored {
                let saved = decode_campaign_image(&bytes)?;
                let saved_map = saved.location();
                if map != location_key(&saved_map)
                    || saved_map.content != active.content
                    || map == location_key(&active)
                    || worlds.contains_key(&map)
                    || !campaign_entries(&saved)?.is_empty()
                {
                    return Err(CampaignUnitError::InvalidWorld);
                }
                worlds.insert(map, bytes);
            }
        }
        self.current = current;
        self.worlds = worlds;
        self.revision += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content() -> ContentId {
        ContentId("q2:rerelease:baseq2:1".to_string())
    }

    fn other_content() -> ContentId {
        ContentId("q2:rerelease:xatrix:1".to_string())
    }

    fn request(content: ContentId, path: &str) -> ResourceRequest {
        ResourceRequest {
            content,
            path: path.to_string(),
        }
    }

    fn image(path: &str, world: &[u8]) -> CampaignUnitImage {
        CampaignUnitImage {
            providers: Vec::new(),
            content: content(),
            path: path.to_string(),
            source_provider: "world:gameplay".to_string(),
            world: world.to_vec(),
        }
    }

    #[test]
    fn stage_commit_round_trip_preserves_departed_worlds() {
        let mut unit = CampaignUnit::new();
        let first = request(content(), "maps/base1.bsp");
        unit.stage(&first, true, None).unwrap().commit(&mut unit).unwrap();
        let departure = image("maps/base1.bsp", b"base1-world");
        let second = request(content(), "maps/base2.bsp");
        let visit = unit.stage(&second, false, Some(&departure)).unwrap();
        assert!(visit.restore.is_none());
        visit.commit(&mut unit).unwrap();
        // Returning restores the departed world.
        let current = image("maps/base2.bsp", b"base2-world");
        let back = unit.stage(&first, false, Some(&current)).unwrap();
        assert_eq!(back.restore.as_ref().unwrap().world, b"base1-world");
        assert_eq!(back.restore.as_ref().unwrap().path, "maps/base1.bsp");
        back.commit(&mut unit).unwrap();
    }

    #[test]
    fn new_unit_and_content_change_reset_worlds() {
        let mut unit = CampaignUnit::new();
        let first = request(content(), "maps/base1.bsp");
        unit.stage(&first, true, None).unwrap().commit(&mut unit).unwrap();
        let departure = image("maps/base1.bsp", b"base1-world");
        let fresh = request(content(), "maps/base2.bsp");
        unit.stage(&fresh, true, Some(&departure))
            .unwrap()
            .commit(&mut unit)
            .unwrap();
        let back = unit.stage(&first, false, None).unwrap();
        assert!(back.restore.is_none());
        back.commit(&mut unit).unwrap();
        let moved = request(other_content(), "maps/rmine1.bsp");
        let visit = unit
            .stage(&moved, false, Some(&image("maps/base1.bsp", b"stale")))
            .unwrap();
        assert!(visit.restore.is_none());
    }

    #[test]
    fn superseded_visits_and_mismatched_departures_fail() {
        let mut unit = CampaignUnit::new();
        let first = request(content(), "maps/base1.bsp");
        unit.stage(&first, true, None).unwrap().commit(&mut unit).unwrap();
        let stale = unit.stage(&request(content(), "maps/base2.bsp"), false, None).unwrap();
        unit.stage(&request(content(), "maps/base3.bsp"), false, None)
            .unwrap()
            .commit(&mut unit)
            .unwrap();
        assert!(matches!(stale.commit(&mut unit), Err(CampaignUnitError::Superseded)));
        let wrong = image("maps/other.bsp", b"world");
        assert!(matches!(
            unit.stage(&request(content(), "maps/base2.bsp"), false, Some(&wrong)),
            Err(CampaignUnitError::DepartureMismatch)
        ));
    }

    #[test]
    fn attach_and_restore_round_trip_through_checkpoint() {
        let mut unit = CampaignUnit::new();
        let first = request(content(), "maps/base1.bsp");
        unit.stage(&first, true, None).unwrap().commit(&mut unit).unwrap();
        let departure = image("maps/base1.bsp", b"base1-world");
        let second = request(content(), "maps/base2.bsp");
        unit.stage(&second, false, Some(&departure))
            .unwrap()
            .commit(&mut unit)
            .unwrap();
        let saved = unit.attach(&image("maps/base2.bsp", b"base2-world")).unwrap();
        let mut restored = CampaignUnit::new();
        restored.restore(&saved).unwrap();
        let back = restored
            .stage(&first, false, Some(&image("maps/base2.bsp", b"base2-again")))
            .unwrap();
        assert_eq!(back.restore.unwrap().world, b"base1-world");
    }

    #[test]
    fn restore_rejects_foreign_and_duplicate_checkpoints() {
        let mut unit = CampaignUnit::new();
        let mut foreign = image("maps/base1.bsp", b"world");
        foreign.providers.push(ProviderCheckpoint {
            provider: "other:owner".to_string(),
            schema: "session:campaign-unit".to_string(),
            version: 1,
            bytes: Vec::new(),
        });
        assert!(unit.restore(&foreign).is_err());
        let checkpoint = CampaignUnit::new()
            .checkpoint(Some(&request(content(), "maps/base1.bsp")))
            .unwrap();
        let mut duplicated = image("maps/base1.bsp", b"world");
        duplicated.providers.push(checkpoint.clone());
        duplicated.providers.push(checkpoint);
        assert!(matches!(unit.restore(&duplicated), Err(CampaignUnitError::Duplicate)));
    }
}
