//! Native Q2 rerelease source-save checkpoint. Port of
//! `src/app/bootstrap/simulation/native-q2-rerelease-save.ts`.

use std::collections::HashSet;

use qa_content::contract::ProviderCheckpoint;
use qa_core::identity::{ProviderId, SavedActorId};
use qa_guest::checkpoint::{ModuleIdentity, read_module, write_module};
use qa_world::registry::provider_key;
use qa_world::save::json::{SourceJson, parse_source_json};
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::read_vector;
use qa_world::save::value::{
    SaveJson, SaveReader, arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int,
    num, obj, str as json_str,
};

use crate::persistence::q2::foundation::{
    Q2AttackCheckpoint, read_q2_attack_checkpoint, write_q2_attack_checkpoint,
};

pub const NATIVE_SCHEMA: &str = "q2:rerelease-native-original";
pub const NATIVE_API_KIND: &str = "q2-rerelease-game";
pub const NATIVE_API_VERSION: i64 = 2023;
pub const NATIVE_ABI: &str = "windows-x86-64";
const MAX_CONFIGSTRING: i64 = 12447;

/// Expected native save identity.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseNativeIdentity {
    pub module: ModuleIdentity,
    pub map: String,
}

/// Saved actor reference: native slot or shared live actor.
#[derive(Debug, Clone, PartialEq)]
pub enum RereleaseSavedActor {
    Native { slot: u32, generation: u32 },
    Shared { actor: SavedActorId },
}

/// Deferred damage actor reference fixup.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseDamageReference {
    pub actor: SavedActorId,
    pub reference: RereleaseSavedActor,
}

/// Damage delivery mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageDelivery {
    Direct,
    Radius,
}

impl DamageDelivery {
    fn parse(value: &str) -> Self {
        if value == "direct" { Self::Direct } else { Self::Radius }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Radius => "radius",
        }
    }
}

/// Deferred damage request.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseDamageRequest {
    pub amount: f64,
    pub knockback: f64,
    pub direction: [f64; 3],
    pub point: [f64; 3],
    pub normal: [f64; 3],
    pub delivery: DamageDelivery,
}

/// One deferred damage record.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseDeferredDamageSave {
    pub target: RereleaseSavedActor,
    pub attack: Q2AttackCheckpoint,
    pub references: Vec<RereleaseDamageReference>,
    pub request: RereleaseDamageRequest,
    pub blood: f64,
    pub knockback: f64,
    pub point: [f64; 3],
    pub modifiers: Vec<u8>,
    pub attacker_slot: u32,
    pub inflictor_slot: u32,
}

/// Native slot to live actor projection.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseProjection {
    pub slot: u32,
    pub actor: SavedActorId,
}

/// One native source save blob.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseSourceSave {
    pub native: Vec<u8>,
    pub deferred_damage: Vec<RereleaseDeferredDamageSave>,
    pub projections: Vec<RereleaseProjection>,
}

/// Configstring entry.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigStringEntry {
    pub index: u32,
    pub value: String,
}

/// Portal state entry.
#[derive(Debug, Clone, PartialEq)]
pub struct PortalEntry {
    pub portal: u32,
    pub open: bool,
}

/// Per-level configstrings plus portals.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q2RereleaseLevelState {
    pub configstrings: Vec<ConfigStringEntry>,
    pub portals: Vec<PortalEntry>,
}

/// Visited level (always checkpoint version 1).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseVisitedLevel {
    pub state: Q2RereleaseLevelState,
    pub map: String,
    pub level: RereleaseSourceSave,
}

/// Server state: level metadata plus raw cvar checkpoint bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseServerState {
    pub state: Q2RereleaseLevelState,
    pub cvars: Vec<u8>,
}

/// Native API identity block.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseNativeApi {
    pub kind: String,
    pub version: i64,
}

/// Full native rerelease save.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseNativeSave {
    pub module: ModuleIdentity,
    pub map: String,
    pub api: RereleaseNativeApi,
    pub abi: String,
    pub autosave: bool,
    pub server: Q2RereleaseServerState,
    pub game: RereleaseSourceSave,
    pub level: RereleaseSourceSave,
    pub visited_levels: Vec<Q2RereleaseVisitedLevel>,
}

/// Native rerelease save failure.
#[derive(Debug, thiserror::Error)]
pub enum NativeQ2RereleaseError {
    #[error(transparent)]
    World(#[from] qa_world::WorldError),
    #[error(transparent)]
    Guest(#[from] qa_guest::GuestError),
    #[error(transparent)]
    Persistence(#[from] crate::persistence::PersistenceError),
    #[error("Native API 2023 value exceeds its public range")]
    ValueOutOfRange,
    #[error("Native API 2023 save requires unterminated JSON bytes")]
    UnterminatedBytes,
    #[error("Native API 2023 save must contain a JSON object")]
    NotObject,
    #[error("Duplicate native API 2023 projection")]
    DuplicateProjection,
    #[error("Invalid native API 2023 deferred damage")]
    InvalidDamage,
    #[error("Duplicate native API 2023 level metadata")]
    DuplicateLevelMetadata,
    #[error("Invalid native API 2023 map path")]
    InvalidMapPath,
    #[error("Unsupported rerelease native source save provider")]
    UnsupportedProvider,
    #[error("Rerelease native save differs from the selected module, ABI or map")]
    SaveMismatch,
    #[error("Duplicate native API 2023 visited map")]
    DuplicateVisitedMap,
    #[error("Native game save cannot own level projections")]
    GameOwnsProjections,
    #[error("{0}")]
    Logic(String),
}

fn bounded_integer(reader: SaveReader, maximum: i64) -> Result<i64, NativeQ2RereleaseError> {
    let value = reader.integer(0)?;
    if value > maximum {
        return Err(NativeQ2RereleaseError::ValueOutOfRange);
    }
    Ok(value)
}

fn read_u32(reader: SaveReader) -> Result<u32, NativeQ2RereleaseError> {
    let value = reader.integer(0)?;
    u32::try_from(value).map_err(|_| NativeQ2RereleaseError::ValueOutOfRange)
}

fn read_angles(reader: SaveReader) -> Result<[f64; 3], NativeQ2RereleaseError> {
    let vector = read_vector(reader)?;
    Ok([f64::from(vector.x), f64::from(vector.y), f64::from(vector.z)])
}

fn write_vector(value: [f64; 3]) -> SaveJson {
    obj(vec![("x", num(value[0])), ("y", num(value[1])), ("z", num(value[2]))])
}

fn read_reference(reader: SaveReader) -> Result<RereleaseSavedActor, NativeQ2RereleaseError> {
    let kind = reader.field("kind").choice_str(&["native", "shared"])?;
    if kind == "shared" {
        return Ok(RereleaseSavedActor::Shared { actor: read_saved_actor(reader.field("actor"))? });
    }
    Ok(RereleaseSavedActor::Native {
        slot: read_u32(reader.field("slot"))?,
        generation: read_u32(reader.field("generation"))?,
    })
}

fn write_reference(value: &RereleaseSavedActor) -> SaveJson {
    match value {
        RereleaseSavedActor::Native { slot, generation } => obj(vec![
            ("kind", json_str("native")),
            ("slot", int(i64::from(*slot))),
            ("generation", int(i64::from(*generation))),
        ]),
        RereleaseSavedActor::Shared { actor } => obj(vec![
            ("kind", json_str("shared")),
            ("actor", write_saved_actor(*actor)),
        ]),
    }
}

fn read_damage(reader: SaveReader) -> Result<RereleaseDeferredDamageSave, NativeQ2RereleaseError> {
    let request = reader.field("request");
    let delivery = request.field("delivery").choice_str(&["direct", "radius"])?;
    Ok(RereleaseDeferredDamageSave {
        target: read_reference(reader.field("target"))?,
        attack: read_q2_attack_checkpoint(reader.field("attack"))?,
        references: reader.field("references").list(|value| -> Result<RereleaseDamageReference, NativeQ2RereleaseError> {
            Ok(RereleaseDamageReference {
                actor: read_saved_actor(value.field("actor"))?,
                reference: read_reference(value.field("reference"))?,
            })
        })?,
        request: RereleaseDamageRequest {
            amount: request.field("amount").finite()?,
            knockback: request.field("knockback").finite()?,
            direction: read_angles(request.field("direction"))?,
            point: read_angles(request.field("point"))?,
            normal: read_angles(request.field("normal"))?,
            delivery: DamageDelivery::parse(&delivery),
        },
        blood: reader.field("blood").finite()?,
        knockback: reader.field("knockback").finite()?,
        point: read_angles(reader.field("point"))?,
        modifiers: reader
            .field("mod")
            .list(|value| bounded_integer(value, 255))?
            .into_iter()
            .map(|byte| byte as u8)
            .collect(),
        attacker_slot: read_u32(reader.field("attackerSlot"))?,
        inflictor_slot: read_u32(reader.field("inflictorSlot"))?,
    })
}

fn write_damage(value: &RereleaseDeferredDamageSave) -> SaveJson {
    obj(vec![
        ("target", write_reference(&value.target)),
        ("attack", write_q2_attack_checkpoint(&value.attack)),
        (
            "references",
            arr(value
                .references
                .iter()
                .map(|entry| {
                    obj(vec![
                        ("actor", write_saved_actor(entry.actor)),
                        ("reference", write_reference(&entry.reference)),
                    ])
                })
                .collect()),
        ),
        (
            "request",
            obj(vec![
                ("amount", num(value.request.amount)),
                ("knockback", num(value.request.knockback)),
                ("direction", write_vector(value.request.direction)),
                ("point", write_vector(value.request.point)),
                ("normal", write_vector(value.request.normal)),
                ("delivery", json_str(value.request.delivery.as_str())),
            ]),
        ),
        ("blood", num(value.blood)),
        ("knockback", num(value.knockback)),
        ("point", write_vector(value.point)),
        ("mod", arr(value.modifiers.iter().map(|byte| int(i64::from(*byte))).collect())),
        ("attackerSlot", int(i64::from(value.attacker_slot))),
        ("inflictorSlot", int(i64::from(value.inflictor_slot))),
    ])
}

/// Read one native source save blob.
pub fn read_rerelease_source_save(reader: SaveReader) -> Result<RereleaseSourceSave, NativeQ2RereleaseError> {
    let native = reader.field("native").bytes()?;
    let deferred_damage = reader.field("deferredDamage").list(read_damage)?;
    let projections = reader.field("projections").list(|value| -> Result<RereleaseProjection, NativeQ2RereleaseError> {
        Ok(RereleaseProjection {
            slot: read_u32(value.field("slot"))?,
            actor: read_saved_actor(value.field("actor"))?,
        })
    })?;
    if native.is_empty() || native.contains(&0) {
        return Err(NativeQ2RereleaseError::UnterminatedBytes);
    }
    let text = std::str::from_utf8(&native).map_err(|_| NativeQ2RereleaseError::NotObject)?;
    let parsed = parse_source_json(text).map_err(|_| NativeQ2RereleaseError::NotObject)?;
    if !matches!(parsed, SourceJson::Object(_)) {
        return Err(NativeQ2RereleaseError::NotObject);
    }
    let mut slots = HashSet::new();
    let mut actors = HashSet::new();
    if projections.iter().any(|value| !slots.insert(value.slot))
        || projections.iter().any(|value| !actors.insert((value.actor.slot, value.actor.generation)))
    {
        return Err(NativeQ2RereleaseError::DuplicateProjection);
    }
    if deferred_damage.iter().any(|damage| damage.modifiers.len() != 3) {
        return Err(NativeQ2RereleaseError::InvalidDamage);
    }
    Ok(RereleaseSourceSave { native, deferred_damage, projections })
}

fn write_rerelease_source_save(value: &RereleaseSourceSave) -> SaveJson {
    obj(vec![
        ("native", SaveJson::Bytes(value.native.clone())),
        ("deferredDamage", arr(value.deferred_damage.iter().map(write_damage).collect())),
        (
            "projections",
            arr(value
                .projections
                .iter()
                .map(|entry| {
                    obj(vec![
                        ("slot", int(i64::from(entry.slot))),
                        ("actor", write_saved_actor(entry.actor)),
                    ])
                })
                .collect()),
        ),
    ])
}

fn read_level(reader: &SaveReader) -> Result<Q2RereleaseLevelState, NativeQ2RereleaseError> {
    let state = Q2RereleaseLevelState {
        configstrings: reader.field("configstrings").list(|value| -> Result<ConfigStringEntry, NativeQ2RereleaseError> {
            Ok(ConfigStringEntry {
                index: u32::try_from(bounded_integer(value.field("index"), MAX_CONFIGSTRING)?)
                    .map_err(|_| NativeQ2RereleaseError::ValueOutOfRange)?,
                value: value.field("value").string()?,
            })
        })?,
        portals: reader.field("portals").list(|value| -> Result<PortalEntry, NativeQ2RereleaseError> {
            Ok(PortalEntry {
                portal: read_u32(value.field("portal"))?,
                open: value.field("open").boolean()?,
            })
        })?,
    };
    let mut indexes = HashSet::new();
    let mut portals = HashSet::new();
    if state.configstrings.iter().any(|value| !indexes.insert(value.index))
        || state.portals.iter().any(|value| !portals.insert(value.portal))
    {
        return Err(NativeQ2RereleaseError::DuplicateLevelMetadata);
    }
    Ok(state)
}

fn write_level_members(state: &Q2RereleaseLevelState) -> Vec<(&'static str, SaveJson)> {
    vec![
        (
            "configstrings",
            arr(state
                .configstrings
                .iter()
                .map(|entry| {
                    obj(vec![
                        ("index", int(i64::from(entry.index))),
                        ("value", json_str(&entry.value)),
                    ])
                })
                .collect()),
        ),
        (
            "portals",
            arr(state
                .portals
                .iter()
                .map(|entry| obj(vec![("portal", int(i64::from(entry.portal))), ("open", boolean(entry.open))]))
                .collect()),
        ),
    ]
}

fn map_path(reader: SaveReader) -> Result<String, NativeQ2RereleaseError> {
    let path = reader.string()?;
    let valid = path.starts_with("maps/")
        && path.ends_with(".bsp")
        && path.len() > 9
        && !path.bytes().any(|byte| byte < 0x20 || byte == 0x7f || byte == b'\\' || byte == b':')
        && !path.split('/').any(|part| part.is_empty() || part == "." || part == "..");
    if !valid {
        return Err(NativeQ2RereleaseError::InvalidMapPath);
    }
    Ok(path)
}

fn same_module(left: &ModuleIdentity, right: &ModuleIdentity) -> bool {
    left.id == right.id
        && left.artifact_path == right.artifact_path
        && left.digest == right.digest
        && left.revision == right.revision
}

fn provider_from_module(id: &str) -> Result<ProviderId, NativeQ2RereleaseError> {
    let Some((namespace, name)) = id.split_once(':') else {
        return Err(NativeQ2RereleaseError::UnsupportedProvider);
    };
    Ok(ProviderId::new(namespace, name))
}

/// Decode a native rerelease provider checkpoint.
pub fn decode_q2_rerelease_native_save(
    record: &ProviderCheckpoint,
    expected: &Q2RereleaseNativeIdentity,
) -> Result<Q2RereleaseNativeSave, NativeQ2RereleaseError> {
    if provider_key(&record.provider) != expected.module.id
        || record.schema != NATIVE_SCHEMA
        || record.version != 1.0
    {
        return Err(NativeQ2RereleaseError::UnsupportedProvider);
    }
    let value = decode_checkpoint_value(&record.bytes)?;
    let reader = SaveReader::at(&value, "q2.rerelease-native-original");
    let api = reader.field("api");
    api.field("kind").literal_str(NATIVE_API_KIND)?;
    api.field("version").literal_i64(NATIVE_API_VERSION)?;
    reader.field("abi").literal_str(NATIVE_ABI)?;
    let module = read_module(reader.field("module"))?;
    let map = map_path(reader.field("map"))?;
    if !same_module(&module, &expected.module) || map != expected.map {
        return Err(NativeQ2RereleaseError::SaveMismatch);
    }
    let state = reader.field("server");
    let cvars = state.field("cvars").bytes()?;
    decode_checkpoint_value(&cvars)?;
    let visited_levels = reader.field("visitedLevels").list(|value| -> Result<Q2RereleaseVisitedLevel, NativeQ2RereleaseError> {
        value.field("version").literal_i64(1)?;
        Ok(Q2RereleaseVisitedLevel {
            state: read_level(&value)?,
            map: map_path(value.field("map"))?,
            level: read_rerelease_source_save(value.field("level"))?,
        })
    })?;
    let mut maps = HashSet::new();
    if visited_levels.iter().any(|value| !maps.insert(value.map.clone()))
        || visited_levels.iter().any(|value| value.map == map)
    {
        return Err(NativeQ2RereleaseError::DuplicateVisitedMap);
    }
    let game = read_rerelease_source_save(reader.field("game"))?;
    if !game.deferred_damage.is_empty() || !game.projections.is_empty() {
        return Err(NativeQ2RereleaseError::GameOwnsProjections);
    }
    let level = read_rerelease_source_save(reader.field("level"))?;
    Ok(Q2RereleaseNativeSave {
        module,
        map,
        api: RereleaseNativeApi { kind: NATIVE_API_KIND.to_string(), version: NATIVE_API_VERSION },
        abi: NATIVE_ABI.to_string(),
        autosave: reader.field("autosave").boolean()?,
        server: Q2RereleaseServerState { state: read_level(&state)?, cvars },
        game,
        level,
        visited_levels,
    })
}

/// Encode a native rerelease save, validating with a decode round-trip.
pub fn encode_q2_rerelease_native_save(
    save: &Q2RereleaseNativeSave,
) -> Result<ProviderCheckpoint, NativeQ2RereleaseError> {
    let mut server = write_level_members(&save.server.state);
    server.push(("cvars", SaveJson::Bytes(save.server.cvars.clone())));
    let value = obj(vec![
        (
            "api",
            obj(vec![
                ("kind", json_str(&save.api.kind)),
                ("version", int(save.api.version)),
            ]),
        ),
        ("abi", json_str(&save.abi)),
        ("module", write_module(&save.module)),
        ("map", json_str(&save.map)),
        ("autosave", boolean(save.autosave)),
        ("server", obj(server)),
        ("game", write_rerelease_source_save(&save.game)),
        ("level", write_rerelease_source_save(&save.level)),
        (
            "visitedLevels",
            arr(save
                .visited_levels
                .iter()
                .map(|visited| {
                    let mut members = vec![("version", int(1))];
                    members.extend(write_level_members(&visited.state));
                    members.push(("map", json_str(&visited.map)));
                    members.push(("level", write_rerelease_source_save(&visited.level)));
                    obj(members)
                })
                .collect()),
        ),
    ]);
    let record = ProviderCheckpoint {
        provider: provider_from_module(&save.module.id)?,
        schema: NATIVE_SCHEMA.to_string(),
        version: 1.0,
        bytes: encode_checkpoint_value(&value),
    };
    decode_q2_rerelease_native_save(
        &record,
        &Q2RereleaseNativeIdentity { module: save.module.clone(), map: save.map.clone() },
    )?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(value: &SaveJson) -> SaveReader<'_> {
        SaveReader::at(value, "test")
    }

    #[test]
    fn map_path_accepts_and_rejects() {
        let good = json_str("maps/base1.bsp");
        assert_eq!(map_path(reader(&good)).unwrap(), "maps/base1.bsp");
        for bad in ["base1.bsp", "maps/../x.bsp", "maps/a.txt", "maps//a.bsp", "maps/a:.bsp"] {
            let value = json_str(bad);
            let error = map_path(reader(&value)).unwrap_err().to_string();
            assert_eq!(error, "Invalid native API 2023 map path");
        }
    }

    #[test]
    fn bounded_integer_rejects_overflow() {
        let value = int(12448);
        let error = bounded_integer(reader(&value), MAX_CONFIGSTRING).unwrap_err().to_string();
        assert_eq!(error, "Native API 2023 value exceeds its public range");
        assert_eq!(bounded_integer(reader(&int(12447)), MAX_CONFIGSTRING).unwrap(), 12447);
    }

    #[test]
    fn references_parse_both_kinds() {
        let native = obj(vec![("kind", json_str("native")), ("slot", int(2)), ("generation", int(5))]);
        assert_eq!(
            read_reference(reader(&native)).unwrap(),
            RereleaseSavedActor::Native { slot: 2, generation: 5 }
        );
        let shared = obj(vec![
            ("kind", json_str("shared")),
            ("actor", obj(vec![("slot", int(1)), ("generation", int(1))])),
        ]);
        assert_eq!(
            read_reference(reader(&shared)).unwrap(),
            RereleaseSavedActor::Shared { actor: SavedActorId { slot: 1, generation: 1 } }
        );
    }

    #[test]
    fn duplicate_level_metadata_is_rejected() {
        let entry = obj(vec![("index", int(3)), ("value", json_str("x"))]);
        let value = obj(vec![
            ("configstrings", arr(vec![entry.clone(), entry])),
            ("portals", arr(vec![])),
        ]);
        let error = read_level(&reader(&value)).unwrap_err().to_string();
        assert_eq!(error, "Duplicate native API 2023 level metadata");
    }

    #[test]
    fn empty_native_bytes_are_rejected() {
        let value = obj(vec![
            ("native", SaveJson::Bytes(Vec::new())),
            ("deferredDamage", arr(vec![])),
            ("projections", arr(vec![])),
        ]);
        let error = read_rerelease_source_save(reader(&value)).unwrap_err().to_string();
        assert_eq!(error, "Native API 2023 save requires unterminated JSON bytes");
    }
}
