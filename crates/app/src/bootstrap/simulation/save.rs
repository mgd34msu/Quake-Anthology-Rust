//! Simulation save-image readers and validators.
//!
//! Port of donor `src/app/bootstrap/simulation/save.ts`.

use std::rc::Rc;

use qa_content::contract::{
    ContentDigest, ModuleIdentity as ContentModuleIdentity, Q3ApiIdentity, QuakeCApiIdentity, QuakeCCheckpoint,
    QvmAbiProfile, QvmCheckpoint,
};
use qa_content::q3::base::shared::definitions::Product;
use qa_content::q3::foundation::arsenal::Q3ArsenalRuntimeState;
use qa_content::q3::team_arena::movement_host::ClientMovementOptions;
use qa_core::identity::{ProviderId, SavedActorId};
use qa_guest::checkpoint::{read_module, ModuleIdentity as GuestModuleIdentity};
use qa_world::movement::q3::weapon::Q3ExternalWeaponSlot;
use qa_world::registry::ActorSlotCheckpoint;
use qa_world::save::ownership::{save_provider_contract, validate_save_provider_owner, ProviderCheckpoint};
use qa_world::save::records::read_saved_actor;
use qa_world::save::value::{decode_checkpoint_value, SaveJson, SaveReader};

use super::native_q2_rerelease_save::{
    decode_q2_rerelease_native_save, Q2RereleaseNativeIdentity, Q2RereleaseNativeSave,
};
use crate::persistence::q2::classic_guest::{
    decode_q2_classic_original_save, Q2ClassicOriginalIdentity, Q2ClassicOriginalSave,
};
use crate::persistence::q2::foundation::{read_q2_attack_checkpoint, Q2AttackCheckpoint};

/// Simulation save error.
#[derive(Debug, thiserror::Error)]
pub enum SimulationSaveError {
    /// Invalid save data.
    #[error("invalid simulation save: {0}")]
    Invalid(String),
}

impl SimulationSaveError {
    /// Invalid-data error.
    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

impl From<qa_world::WorldError> for SimulationSaveError {
    fn from(error: qa_world::WorldError) -> Self {
        Self::invalid(error.to_string())
    }
}

fn mapped(error: impl ToString) -> SimulationSaveError {
    SimulationSaveError::invalid(error.to_string())
}

/// Mirror of `SaveImage` from donor `src/contracts/session.ts`
/// (canonical home: session lane; unify post-merge).
#[derive(Debug, Clone)]
pub struct SimulationSaveImage {
    /// Provider records.
    pub providers: Vec<ProviderCheckpoint>,
    /// Saved recipe.
    pub recipe: SimSavedRecipe,
    /// Guest checkpoints.
    pub guests: Vec<SimSavedGuest>,
    /// Saved clocks.
    pub clocks: Vec<SimSaveClock>,
    /// Saved random streams.
    pub random: Vec<SimSaveRandom>,
    /// Saved bodies.
    pub bodies: Vec<qa_world::save::records::UnifiedBody>,
    /// Saved combat states.
    pub combat: Vec<SimSavedCombat>,
    /// Saved inventories.
    pub inventories: Vec<SimSavedInventory>,
    /// Saved configurations.
    pub configurations: Vec<SimActorEntry>,
    /// Saved thinks.
    pub thinks: Vec<qa_world::save::records::UnifiedThink>,
    /// Frame time.
    pub frame_time: SaveJson,
    /// Saved mod session, if the image carries gameplay mods.
    pub mods: Option<crate::persistence::mods::ModSessionCheckpoint>,
    /// Save schema version (donor `schemaVersion`).
    pub schema_version: u32,
    /// Legacy armor layout flag (donor `legacyArmorLayout`).
    pub legacy_armor_layout: bool,
    /// Saved frame context (donor `frame`).
    pub frame: qa_core::time::FrameContext,
    /// Next event sequence (donor `nextEventSequence`).
    pub next_event_sequence: u64,
    /// Saved actor slots (donor `actors`).
    pub actors: Vec<ActorSlotCheckpoint>,
}

/// Mirror of the saved recipe subset `save.ts` consumes.
#[derive(Debug, Clone)]
pub struct SimSavedRecipe {
    /// Saved executions.
    pub execution: Vec<SimSavedExecution>,
    /// Map entities provider.
    pub map_entities_provider: String,
    /// Map geometry path.
    pub map_geometry_path: String,
}

/// Mirror of one saved execution.
#[derive(Debug, Clone)]
pub struct SimSavedExecution {
    /// Module role.
    pub role: String,
    /// Implementation kind.
    pub kind: SimExecutionKind,
    /// API kind.
    pub api_kind: String,
    /// API version.
    pub api_version: i64,
    /// ABI profile kind.
    pub profile_kind: String,
    /// Artifact digest.
    pub artifact_digest: String,
    /// Artifact path.
    pub artifact_path: String,
    /// Owner provider.
    pub owner_provider: String,
    /// Mount identity for QVM revisions.
    pub mount_id: String,
    /// Mount generation for QVM revisions.
    pub mount_generation: String,
}

/// Saved execution kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimExecutionKind {
    /// Native artifact.
    Native,
    /// QuakeC artifact.
    Quakec,
    /// QVM artifact.
    Qvm,
    /// Built-in source.
    Builtin,
}

/// Mirror of one saved guest checkpoint with its host module.
#[derive(Debug, Clone)]
pub struct SimSavedGuest {
    /// Guest checkpoint.
    pub checkpoint: SimGuestCheckpoint,
    /// Host module.
    pub host_module: ContentModuleIdentity,
}

/// Saved guest checkpoint union.
#[derive(Debug, Clone)]
pub enum SimGuestCheckpoint {
    /// QuakeC checkpoint.
    QuakeC(QuakeCCheckpoint),
    /// QVM checkpoint.
    Qvm(QvmCheckpoint),
}

impl SimGuestCheckpoint {
    /// Checkpoint kind tag.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::QuakeC(_) => "quakec",
            Self::Qvm(_) => "qvm",
        }
    }

    /// Owning module.
    #[must_use]
    pub fn module(&self) -> &ContentModuleIdentity {
        match self {
            Self::QuakeC(checkpoint) => &checkpoint.module,
            Self::Qvm(checkpoint) => &checkpoint.module,
        }
    }

    /// API kind and version projection for execution comparison.
    #[must_use]
    pub fn api_kind_version(&self) -> (String, i64) {
        match self {
            Self::QuakeC(checkpoint) => match checkpoint.api {
                QuakeCApiIdentity::Netquake => ("q1-netquake".to_string(), 6),
                QuakeCApiIdentity::Quakeworld => ("q1-quakeworld".to_string(), 6),
            },
            Self::Qvm(checkpoint) => match checkpoint.api {
                Q3ApiIdentity::Qagame(version) => ("q3-qagame".to_string(), i64::from(version)),
                Q3ApiIdentity::Cgame(version) => ("q3-cgame".to_string(), i64::from(version)),
                Q3ApiIdentity::Ui(version) => ("q3-ui".to_string(), i64::from(version)),
            },
        }
    }
}

/// Mirror of one saved clock.
#[derive(Debug, Clone)]
pub struct SimSaveClock {
    /// Clock provider.
    pub provider: String,
    /// Clock time.
    pub time: SaveJson,
}

/// Mirror of one saved random stream.
#[derive(Debug, Clone)]
pub struct SimSaveRandom {
    /// Stream provider.
    pub provider: String,
    /// Stream state (donor `RandomState`).
    pub state: qa_world::save::shared::SaveRandomState,
}

/// Mirror of one saved per-actor record.
#[derive(Debug, Clone)]
pub struct SimActorEntry {
    /// Saved actor.
    pub actor: SavedActorId,
}

/// Mirror of one saved inventory record (donor `InventoryCheckpoint`).
#[derive(Debug, Clone)]
pub struct SimSavedInventory {
    /// Saved actor.
    pub actor: SavedActorId,
    /// Saved entries.
    pub entries: Vec<qa_world::inventory::InventoryEntry>,
}

/// Mirror of one saved combat record (donor `CombatCheckpoint`).
#[derive(Debug, Clone)]
pub struct SimSavedCombat {
    /// Saved actor.
    pub actor: SavedActorId,
    /// Saved combat state.
    pub state: qa_world::combat::CombatState,
}

/// Mirror of `DecodedApplicationBotsCheckpoint` from donor
/// `src/app/bootstrap/simulation/bots.ts` (canonical home: bots lane,
/// wave 2; unify post-merge).
#[derive(Debug, Clone)]
pub struct DecodedApplicationBotsCheckpoint {
    /// Checkpoint version.
    pub version: u32,
    /// Transport checkpoint.
    pub transport: ApplicationBotTransportCheckpoint,
    /// Director state.
    pub director: SaveJson,
    /// Navigation state.
    pub navigation: SaveJson,
    /// Knowledge state.
    pub knowledge: SaveJson,
    /// Shared world state.
    pub shared_world: SaveJson,
    /// Observations.
    pub observations: Vec<BotObservation>,
}

/// Mirror of `ApplicationBotTransportCheckpoint` from donor
/// `src/app/bootstrap/simulation/bots.ts` (canonical home: bots lane,
/// wave 2; unify post-merge).
#[derive(Debug, Clone)]
pub struct ApplicationBotTransportCheckpoint {
    /// Checkpoint version.
    pub version: u32,
    /// Elapsed milliseconds.
    pub elapsed_milliseconds: f64,
    /// Connections.
    pub connections: Vec<BotConnection>,
    /// Snapshots.
    pub snapshots: Vec<BotSnapshot>,
}

/// Mirror of one saved bot connection.
#[derive(Debug, Clone)]
pub struct BotConnection {
    /// Client slot.
    pub client_slot: i64,
    /// Client generation.
    pub client_generation: i64,
    /// Bound actor.
    pub actor: SavedActorId,
    /// Reliable sequence.
    pub reliable_sequence: i64,
    /// Reliable acknowledge.
    pub reliable_acknowledge: i64,
    /// Reliable slots.
    pub reliable_slots: Vec<String>,
}

/// Mirror of one saved bot snapshot.
#[derive(Debug, Clone)]
pub struct BotSnapshot {
    /// Client slot.
    pub client: i64,
    /// Entities.
    pub entities: Vec<i64>,
}

/// Mirror of one saved bot observation.
#[derive(Debug, Clone)]
pub struct BotObservation {
    /// Observation number.
    pub number: i64,
    /// Observed actor.
    pub actor: SavedActorId,
}

/// Bots decode seam (donor `decodeApplicationBotsCheckpoint` from donor
/// `src/app/bootstrap/simulation/bots.ts`, canonical home: bots lane,
/// wave 2).
pub type DecodeApplicationBotsCheckpointFn =
    Rc<dyn Fn(&SaveJson) -> Result<DecodedApplicationBotsCheckpoint, SimulationSaveError>>;

/// Saved Q3 guest client slot (donor `savedQ3GuestClients` result subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedQ3GuestClientSlot {
    /// Client slot.
    pub slot: i64,
}

/// Q3 guest clients seam (donor `savedQ3GuestClients` from donor
/// `src/app/bootstrap/simulation/q3/guest-runtime.ts`, canonical home:
/// `q3::guest_runtime`).
pub type SavedQ3GuestClientsFn = Rc<dyn Fn(&QvmCheckpoint) -> Result<Vec<SavedQ3GuestClientSlot>, SimulationSaveError>>;

/// Owned simulation document: the decoded `world:simulation` payload
/// plus a borrowed reader.
#[derive(Debug, Clone)]
pub struct SimulationSaveDocument {
    value: SaveJson,
}

impl SimulationSaveDocument {
    /// Borrow a reader over the document.
    #[must_use]
    pub fn reader(&self) -> SaveReader<'_> {
        SaveReader::new(&self.value)
    }
}

/// Read the single provider record for a schema.
pub fn simulation_provider_checkpoint<'a>(
    image: &'a SimulationSaveImage,
    schema: &str,
) -> Result<&'a ProviderCheckpoint, SimulationSaveError> {
    let matches: Vec<&ProviderCheckpoint> = image.providers.iter().filter(|value| value.schema == schema).collect();
    let expected = save_provider_contract(schema, &image.recipe.map_entities_provider).map_err(mapped)?;
    match matches.as_slice() {
        [value] if value.provider == expected.provider && value.version == expected.version => Ok(value),
        _ => Err(SimulationSaveError::invalid(format!(
            "Missing or unsupported saved provider {schema}"
        ))),
    }
}

/// Decode the `world:simulation` document.
pub fn simulation_save_reader(image: &SimulationSaveImage) -> Result<SimulationSaveDocument, SimulationSaveError> {
    let record = simulation_provider_checkpoint(image, "world:simulation")?;
    let value = decode_checkpoint_value(&record.bytes).map_err(mapped)?;
    Ok(SimulationSaveDocument { value })
}

/// Read saved source cvars, if the image carries any.
pub fn saved_source_cvars(image: &SimulationSaveImage) -> Result<Option<SaveJson>, SimulationSaveError> {
    let cvars = match native_q2_original_save(image, None)? {
        Some(save) => Some(save.server.cvars),
        None => native_q2_rerelease_save(image, None)?.map(|save| save.server.cvars),
    };
    if let Some(cvars) = cvars {
        return decode_checkpoint_value(&cvars).map(Some).map_err(mapped);
    }
    if !image
        .providers
        .iter()
        .any(|record| record.schema == "world:source-cvars")
    {
        return Ok(None);
    }
    let record = simulation_provider_checkpoint(image, "world:source-cvars")?;
    decode_checkpoint_value(&record.bytes).map(Some).map_err(mapped)
}

/// Read and validate the saved bot checkpoint, if the image carries one.
pub fn saved_bot_checkpoint(
    image: &SimulationSaveImage,
    decode: &DecodeApplicationBotsCheckpointFn,
) -> Result<Option<DecodedApplicationBotsCheckpoint>, SimulationSaveError> {
    if !image.providers.iter().any(|record| record.schema == "world:bots") {
        return Ok(None);
    }
    let record = simulation_provider_checkpoint(image, "world:bots")?;
    let bots = decode(&decode_checkpoint_value(&record.bytes).map_err(mapped)?)?;
    let document = simulation_save_reader(image)?;
    let players: Vec<(i64, SavedActorId)> = document
        .reader()
        .field("players")
        .list(|value| {
            Ok::<_, SimulationSaveError>((
                value.field("clientSlot").integer(0).map_err(mapped)?,
                read_saved_actor(value.field("actor")).map_err(mapped)?,
            ))
        })
        .map_err(mapped)?;
    let mut slots = std::collections::HashSet::new();
    for connection in &bots.transport.connections {
        if !slots.insert(connection.client_slot) {
            return Err(SimulationSaveError::invalid("Duplicate saved bot client slot"));
        }
        let owned = players.iter().any(|(slot, actor)| {
            *slot == connection.client_slot
                && actor.slot == connection.actor.slot
                && actor.generation == connection.actor.generation
        });
        if !owned {
            return Err(SimulationSaveError::invalid(
                "Saved bot connection does not own its shared player",
            ));
        }
    }
    Ok(Some(bots))
}

pub(crate) fn provider_id(text: &str) -> ProviderId {
    match text.split_once(':') {
        Some((namespace, name)) => ProviderId {
            namespace: namespace.to_string(),
            name: name.to_string(),
        },
        None => ProviderId {
            namespace: String::new(),
            name: text.to_string(),
        },
    }
}

/// Read the selected server guest checkpoint, if the execution needs one.
pub fn simulation_guest_checkpoint(image: &SimulationSaveImage) -> Result<Option<SimSavedGuest>, SimulationSaveError> {
    let executions: Vec<&SimSavedExecution> = image
        .recipe
        .execution
        .iter()
        .filter(|module| module.role == "server-game")
        .collect();
    let execution = match executions.as_slice() {
        [execution] => execution,
        _ => {
            return Err(SimulationSaveError::invalid(
                "Save requires exactly one selected server execution",
            ));
        }
    };
    if !matches!(execution.kind, SimExecutionKind::Quakec | SimExecutionKind::Qvm) {
        if !image.guests.is_empty() {
            return Err(SimulationSaveError::invalid(
                "Selected source cannot restore guest memory providers",
            ));
        }
        return Ok(None);
    }
    let name = if execution.kind == SimExecutionKind::Quakec {
        "QuakeC"
    } else {
        "QVM"
    };
    let expected_kind = if execution.kind == SimExecutionKind::Quakec {
        "quakec"
    } else {
        "qvm"
    };
    let checkpoint = match image.guests.as_slice() {
        [guest] if guest.checkpoint.kind() == expected_kind => guest,
        _ => {
            return Err(SimulationSaveError::invalid(format!(
                "{name} save requires exactly one complete guest checkpoint"
            )));
        }
    };
    if let SimGuestCheckpoint::Qvm(qvm) = &checkpoint.checkpoint {
        let profile_version = match qvm.abi_profile {
            None | Some(QvmAbiProfile::Modern) => 8,
            Some(QvmAbiProfile::Legacy116n) => 7,
        };
        if execution.api_kind != "q3-qagame" || execution.api_version != profile_version {
            return Err(SimulationSaveError::invalid(
                "QVM save requires matching selected qagame API and ABI profile",
            ));
        }
    }
    let expected = ContentModuleIdentity {
        id: provider_id(&execution.owner_provider),
        artifact_path: execution.artifact_path.clone(),
        digest: ContentDigest(execution.artifact_digest.clone()),
        revision: if execution.kind == SimExecutionKind::Quakec {
            execution.artifact_digest.clone()
        } else {
            format!("{}:{}", execution.mount_id, execution.mount_generation)
        },
    };
    let (api_kind, api_version) = checkpoint.checkpoint.api_kind_version();
    if execution.owner_provider != image.recipe.map_entities_provider
        || checkpoint.checkpoint.module() != &expected
        || checkpoint.host_module != expected
        || api_kind != execution.api_kind
        || api_version != execution.api_version
    {
        return Err(SimulationSaveError::invalid(format!(
            "{name} checkpoint differs from the selected artifact and API"
        )));
    }
    Ok(Some(checkpoint.clone()))
}

/// Project a world provider record into the content checkpoint shape
/// the native save decoders consume.
fn content_checkpoint(record: &ProviderCheckpoint) -> qa_content::contract::ProviderCheckpoint {
    qa_content::contract::ProviderCheckpoint {
        provider: provider_id(&record.provider),
        schema: record.schema.clone(),
        version: record.version as f64,
        bytes: record.bytes.clone(),
    }
}

/// Read the saved native module identity, validating it against the
/// selected execution.
fn saved_native_module(image: &SimulationSaveImage, schema: &str) -> Result<GuestModuleIdentity, SimulationSaveError> {
    let execution = image
        .recipe
        .execution
        .iter()
        .find(|module| module.role == "server-game")
        .ok_or_else(|| SimulationSaveError::invalid("Native saved identity has no source execution"))?;
    if execution.kind != SimExecutionKind::Native {
        return Err(SimulationSaveError::invalid(
            "Native saved identity has no source execution",
        ));
    }
    let record = simulation_provider_checkpoint(image, schema)?;
    let payload = decode_checkpoint_value(&record.bytes).map_err(mapped)?;
    let reader = SaveReader::new(&payload);
    let module = read_module(reader.field("module")).map_err(mapped)?;
    let prefix = format!("{}/native-compatibility.json/", execution.artifact_digest);
    let declared = module.revision.starts_with(&prefix)
        && module.revision[prefix.len()..].len() == 71
        && module.revision[prefix.len()..].starts_with("sha256:")
        && module.revision[prefix.len() + 7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit());
    if module.id != execution.owner_provider
        || module.artifact_path != execution.artifact_path
        || module.digest != execution.artifact_digest
        || module.revision != execution.artifact_digest && !declared
    {
        return Err(SimulationSaveError::invalid(
            "Native saved identity differs from the selected artifact",
        ));
    }
    Ok(module)
}

/// Read the classic native save, if the execution selects one.
pub fn native_q2_original_save(
    image: &SimulationSaveImage,
    expected: Option<GuestModuleIdentity>,
) -> Result<Option<Q2ClassicOriginalSave>, SimulationSaveError> {
    let execution = image
        .recipe
        .execution
        .iter()
        .find(|module| module.role == "server-game");
    let native = matches!(execution, Some(execution) if execution.kind == SimExecutionKind::Native);
    if !native {
        if image
            .providers
            .iter()
            .any(|record| record.schema == "q2:classic-native-original")
        {
            return Err(SimulationSaveError::invalid(
                "Original API 3 save has no matching native execution",
            ));
        }
        return Ok(None);
    }
    let execution = execution.expect("native execution");
    if execution.api_kind == "q2-rerelease-game" {
        if image
            .providers
            .iter()
            .any(|record| record.schema == "q2:classic-native-original")
        {
            return Err(SimulationSaveError::invalid(
                "Classic native save has a rerelease execution",
            ));
        }
        return Ok(None);
    }
    if execution.api_kind != "q2-classic-game" || execution.api_version != 3 || execution.profile_kind != "windows-i386"
    {
        return Err(SimulationSaveError::invalid(
            "Unsupported native original-save execution",
        ));
    }
    if image
        .providers
        .iter()
        .any(|record| record.schema == "q2:rerelease-native-original")
    {
        return Err(SimulationSaveError::invalid(
            "Rerelease native save has a classic execution",
        ));
    }
    let record = simulation_provider_checkpoint(image, "q2:classic-native-original")?;
    let module = match expected {
        Some(module) => module,
        None => saved_native_module(image, "q2:classic-native-original")?,
    };
    decode_q2_classic_original_save(
        record,
        &Q2ClassicOriginalIdentity {
            module,
            map: image.recipe.map_geometry_path.clone(),
        },
    )
    .map(Some)
    .map_err(mapped)
}

/// Read the rerelease native save, if the execution selects one.
pub fn native_q2_rerelease_save(
    image: &SimulationSaveImage,
    expected: Option<GuestModuleIdentity>,
) -> Result<Option<Q2RereleaseNativeSave>, SimulationSaveError> {
    let execution = image
        .recipe
        .execution
        .iter()
        .find(|module| module.role == "server-game");
    let rerelease = matches!(execution, Some(execution)
        if execution.kind == SimExecutionKind::Native && execution.api_kind == "q2-rerelease-game");
    if !rerelease {
        if image
            .providers
            .iter()
            .any(|record| record.schema == "q2:rerelease-native-original")
        {
            return Err(SimulationSaveError::invalid(
                "Rerelease native save has no matching execution",
            ));
        }
        return Ok(None);
    }
    let execution = execution.expect("rerelease execution");
    if execution.api_version != 2023 || execution.profile_kind != "windows-x86-64" {
        return Err(SimulationSaveError::invalid(
            "Unsupported rerelease native save execution",
        ));
    }
    if image
        .providers
        .iter()
        .any(|record| record.schema == "q2:classic-native-original")
    {
        return Err(SimulationSaveError::invalid(
            "Classic native save has a rerelease execution",
        ));
    }
    let record = simulation_provider_checkpoint(image, "q2:rerelease-native-original")?;
    let module = match expected {
        Some(module) => module,
        None => saved_native_module(image, "q2:rerelease-native-original")?,
    };
    decode_q2_rerelease_native_save(
        &content_checkpoint(record),
        &Q2RereleaseNativeIdentity {
            module,
            map: image.recipe.map_geometry_path.clone(),
        },
    )
    .map(Some)
    .map_err(mapped)
}

/// Saved native client record.
#[derive(Debug, Clone)]
pub struct NativeQ2SavedClient {
    /// Client slot.
    pub client_slot: i64,
    /// Connection phase.
    pub phase: String,
    /// Userinfo.
    pub userinfo: String,
}

/// Read saved native clients, if the image carries a native save.
pub fn native_q2_saved_clients(image: &SimulationSaveImage) -> Result<Vec<NativeQ2SavedClient>, SimulationSaveError> {
    if native_q2_original_save(image, None)?.is_none() && native_q2_rerelease_save(image, None)?.is_none() {
        return Ok(Vec::new());
    }
    let document = simulation_save_reader(image)?;
    let clients: Vec<NativeQ2SavedClient> = document
        .reader()
        .field("nativeClients")
        .list(|value| {
            Ok::<_, SimulationSaveError>(NativeQ2SavedClient {
                client_slot: value.field("clientSlot").integer(0).map_err(mapped)?,
                phase: value
                    .field("phase")
                    .choice_str(&["connected", "active"])
                    .map_err(mapped)?,
                userinfo: value.field("userinfo").string().map_err(mapped)?,
            })
        })
        .map_err(mapped)?;
    let mut slots = std::collections::HashSet::new();
    if clients.iter().any(|client| !slots.insert(client.client_slot)) {
        return Err(SimulationSaveError::invalid("Duplicate native saved client slot"));
    }
    Ok(clients)
}

/// Read the QuakeC guest checkpoint, if the image carries one.
pub fn simulation_quakec_checkpoint(
    image: &SimulationSaveImage,
) -> Result<Option<QuakeCCheckpoint>, SimulationSaveError> {
    match simulation_guest_checkpoint(image)? {
        Some(guest) => match guest.checkpoint {
            SimGuestCheckpoint::QuakeC(checkpoint) => Ok(Some(checkpoint)),
            SimGuestCheckpoint::Qvm(_) => Ok(None),
        },
        None => Ok(None),
    }
}

/// Read the QVM guest checkpoint, if the image carries one.
pub fn simulation_qvm_checkpoint(image: &SimulationSaveImage) -> Result<Option<QvmCheckpoint>, SimulationSaveError> {
    match simulation_guest_checkpoint(image)? {
        Some(guest) => match guest.checkpoint {
            SimGuestCheckpoint::Qvm(checkpoint) => Ok(Some(checkpoint)),
            SimGuestCheckpoint::QuakeC(_) => Ok(None),
        },
        None => Ok(None),
    }
}

/// Validate a simulation save image.
pub fn validate_simulation_save(
    image: &SimulationSaveImage,
    decode_bots: &DecodeApplicationBotsCheckpointFn,
) -> Result<(), SimulationSaveError> {
    let mut records = std::collections::HashSet::new();
    for record in &image.providers {
        let key = format!("{}/{}", record.provider, record.schema);
        if !records.insert(key) {
            return Err(SimulationSaveError::invalid(format!(
                "Duplicate saved provider {}",
                record.schema
            )));
        }
        validate_save_provider_owner(record, &image.recipe.map_entities_provider).map_err(mapped)?;
    }
    simulation_provider_checkpoint(image, "world:simulation")?;
    simulation_provider_checkpoint(image, "world:source-slots")?;
    let guest = simulation_guest_checkpoint(image)?;
    let native = match native_q2_original_save(image, None)? {
        Some(save) => Some(save.server.cvars),
        None => native_q2_rerelease_save(image, None)?.map(|save| save.server.cvars),
    };
    let document = simulation_save_reader(image)?;
    let players = document
        .reader()
        .field("players")
        .list(|_| Ok::<_, SimulationSaveError>(()))
        .map_err(mapped)?;
    if native.is_some() && !players.is_empty() {
        return Err(SimulationSaveError::invalid(
            "Native Q2 players must remain source-owned",
        ));
    }
    if matches!(
        guest,
        Some(SimSavedGuest {
            checkpoint: SimGuestCheckpoint::Qvm(_),
            ..
        })
    ) && !players.is_empty()
    {
        return Err(SimulationSaveError::invalid(
            "QVM players must remain owned by the saved guest client records",
        ));
    }
    let execution = image
        .recipe
        .execution
        .iter()
        .find(|module| module.role == "server-game");
    let bots = saved_bot_checkpoint(image, decode_bots)?;
    if bots.is_some() && !matches!(execution, Some(execution) if execution.kind == SimExecutionKind::Builtin) {
        return Err(SimulationSaveError::invalid(
            "Saved bot services have no supported source owner",
        ));
    }
    if let Some(execution) = execution {
        if bots.is_some() && execution.kind == SimExecutionKind::Builtin && execution.api_kind != "q3-qagame" {
            simulation_provider_checkpoint(image, "world:source-cvars")?;
        }
    }
    if image
        .providers
        .iter()
        .any(|record| record.schema == "world:source-cvars")
    {
        let owned = matches!(execution, Some(execution)
        if execution.kind == SimExecutionKind::Builtin
            && matches!(
                execution.api_kind.as_str(),
                "q1-netquake" | "q1-quakeworld" | "q2-classic-game" | "q2-rerelease-game"
            ));
        if !owned {
            return Err(SimulationSaveError::invalid(
                "Saved common cvars have no matching source owner",
            ));
        }
        simulation_provider_checkpoint(image, "world:source-cvars")?;
    }
    if guest.is_none() {
        if let Some(execution) = execution {
            if execution.kind == SimExecutionKind::Builtin {
                match execution.api_kind.as_str() {
                    "q1-netquake" | "q1-quakeworld" => {
                        simulation_provider_checkpoint(image, "q1:foundation")?;
                    }
                    "q2-classic-game" | "q2-rerelease-game" => {
                        for schema in [
                            "q2:composition",
                            "q2:foundation",
                            "q2:items",
                            "q2:movers",
                            "q2:monsters",
                            "q2:weapons",
                            "q2:players",
                            "q2:base-entities",
                        ] {
                            simulation_provider_checkpoint(image, schema)?;
                        }
                    }
                    "q3-qagame" => {
                        simulation_provider_checkpoint(image, "q3:native")?;
                        simulation_provider_checkpoint(image, "world:q3-runtime")?;
                    }
                    _ => {}
                }
            }
        }
    }
    let clocks: Vec<&SimSaveClock> = image
        .clocks
        .iter()
        .filter(|clock| clock.provider == image.recipe.map_entities_provider)
        .collect();
    match clocks.as_slice() {
        [clock] if clock.time == image.frame_time => {}
        _ => {
            return Err(SimulationSaveError::invalid(
                "Saved source clock disagrees with its frame",
            ));
        }
    }
    let random = image
        .random
        .iter()
        .filter(|random| random.provider == image.recipe.map_entities_provider)
        .count();
    if random != 1 {
        return Err(SimulationSaveError::invalid(
            "Save requires one matching source random stream",
        ));
    }
    for keys in [
        image
            .bodies
            .iter()
            .map(|entry| (entry.actor.slot, entry.actor.generation))
            .collect::<Vec<_>>(),
        image
            .combat
            .iter()
            .map(|entry| (entry.actor.slot, entry.actor.generation))
            .collect::<Vec<_>>(),
        image
            .inventories
            .iter()
            .map(|entry| (entry.actor.slot, entry.actor.generation))
            .collect::<Vec<_>>(),
        image
            .configurations
            .iter()
            .map(|entry| (entry.actor.slot, entry.actor.generation))
            .collect::<Vec<_>>(),
        image
            .thinks
            .iter()
            .map(|entry| (entry.actor.slot, entry.actor.generation))
            .collect::<Vec<_>>(),
    ] {
        let mut actors = std::collections::HashSet::new();
        for key in &keys {
            if !actors.insert(*key) {
                return Err(SimulationSaveError::invalid("Duplicate saved actor state"));
            }
        }
    }
    Ok(())
}

/// Saved Q3 arsenal record.
#[derive(Debug, Clone)]
pub struct SavedQ3Arsenal {
    /// Owning actor.
    pub actor: SavedActorId,
    /// Runtime state.
    pub state: Q3ArsenalRuntimeState,
}

/// Saved Q3 movement record.
#[derive(Debug, Clone)]
pub struct SavedQ3Movement {
    /// Owning actor.
    pub actor: SavedActorId,
    /// Movement options.
    pub state: Option<ClientMovementOptions>,
}

/// Saved last-attack record.
#[derive(Debug, Clone)]
pub struct SavedLastAttack {
    /// Owning actor.
    pub actor: SavedActorId,
    /// Attack checkpoint.
    pub attack: Q2AttackCheckpoint,
}

/// Decoded Q3 runtime tables.
#[derive(Debug, Clone)]
pub struct NativeQ3RuntimeTables {
    /// Arsenal records.
    pub arsenals: Vec<SavedQ3Arsenal>,
    /// Movement records.
    pub movement: Vec<SavedQ3Movement>,
    /// Last attacks.
    pub last_attacks: Vec<SavedLastAttack>,
}

/// Read the Q3 runtime tables.
pub fn native_q3_runtime_reader(image: &SimulationSaveImage) -> Result<NativeQ3RuntimeTables, SimulationSaveError> {
    let record = simulation_provider_checkpoint(image, "world:q3-runtime")?;
    let payload = decode_checkpoint_value(&record.bytes).map_err(mapped)?;
    let reader = SaveReader::new(&payload);
    let arsenals = reader
        .field("arsenals")
        .list(|entry| {
            let state = entry.field("state");
            let product = match state
                .field("product")
                .choice_str(&["baseq3", "missionpack"])
                .map_err(mapped)?
                .as_str()
            {
                "baseq3" => Product::Baseq3,
                _ => Product::Missionpack,
            };
            let external = match state
                .field("externalSlot")
                .choice_str(&[
                    "active",
                    "holster-requested",
                    "dropping",
                    "holstered",
                    "resume-requested",
                ])
                .map_err(mapped)?
                .as_str()
            {
                "active" => Q3ExternalWeaponSlot::Active,
                "holster-requested" => Q3ExternalWeaponSlot::HolsterRequested,
                "dropping" => Q3ExternalWeaponSlot::Dropping,
                "holstered" => Q3ExternalWeaponSlot::Holstered,
                _ => Q3ExternalWeaponSlot::ResumeRequested,
            };
            Ok::<_, SimulationSaveError>(SavedQ3Arsenal {
                actor: read_saved_actor(entry.field("actor")).map_err(mapped)?,
                state: Q3ArsenalRuntimeState {
                    product,
                    max_health: state.field("maxHealth").number().map_err(mapped)?,
                    spectator: state.field("spectator").boolean().map_err(mapped)?,
                    persistent_powerup_tag: i32::try_from(
                        state.field("persistentPowerupTag").integer(i64::MIN).map_err(mapped)?,
                    )
                    .map_err(mapped)?,
                    holdable_item: i32::try_from(state.field("holdableItem").integer(i64::MIN).map_err(mapped)?)
                        .map_err(mapped)?,
                    holdable_tag: i32::try_from(state.field("holdableTag").integer(i64::MIN).map_err(mapped)?)
                        .map_err(mapped)?,
                    respawned: state.field("respawned").boolean().map_err(mapped)?,
                    use_item_held: state.field("useItemHeld").boolean().map_err(mapped)?,
                    event_sequence: i32::try_from(state.field("eventSequence").integer(i64::MIN).map_err(mapped)?)
                        .map_err(mapped)?,
                    fractional_milliseconds: state.field("fractionalMilliseconds").finite().map_err(mapped)?,
                    external_slot: external,
                    requested_weapon: state
                        .field("requestedWeapon")
                        .nullable(|value| i32::try_from(value.integer(i64::MIN).map_err(mapped)?).map_err(mapped))
                        .map_err(mapped)?,
                },
            })
        })
        .map_err(mapped)?;
    let movement = reader
        .field("movement")
        .list(|entry| {
            Ok::<_, SimulationSaveError>(SavedQ3Movement {
                actor: read_saved_actor(entry.field("actor")).map_err(mapped)?,
                state: entry
                    .field("state")
                    .nullable(|state| {
                        Ok::<_, SimulationSaveError>(ClientMovementOptions {
                            trace_mask: i32::try_from(state.field("traceMask").integer(i64::MIN).map_err(mapped)?)
                                .map_err(mapped)?,
                            fixed_msec: state
                                .field("fixedMsec")
                                .nullable(|value| {
                                    i32::try_from(value.integer(i64::MIN).map_err(mapped)?).map_err(mapped)
                                })
                                .map_err(mapped)?,
                            no_footsteps: state.field("noFootsteps").boolean().map_err(mapped)?,
                            gauntlet_hit: state.field("gauntletHit").boolean().map_err(mapped)?,
                            debug_level: i32::try_from(state.field("debugLevel").integer(i64::MIN).map_err(mapped)?)
                                .map_err(mapped)?,
                        })
                    })
                    .map_err(mapped)?,
            })
        })
        .map_err(mapped)?;
    let last_attacks = reader
        .field("lastAttacks")
        .list(|entry| {
            Ok::<_, SimulationSaveError>(SavedLastAttack {
                actor: read_saved_actor(entry.field("actor")).map_err(mapped)?,
                attack: read_q2_attack_checkpoint(entry.field("attack")).map_err(mapped)?,
            })
        })
        .map_err(mapped)?;
    Ok(NativeQ3RuntimeTables {
        arsenals,
        movement,
        last_attacks,
    })
}

/// Saved simulation settings.
#[derive(Debug, Clone)]
pub struct SavedSimulationSettings {
    /// Skill level.
    pub skill: i64,
    /// Session mode.
    pub mode: String,
    /// Maximum clients.
    pub max_clients: i64,
    /// Seed.
    pub seed: i64,
    /// Initial spawn point.
    pub initial_spawn_point: String,
    /// Start items.
    pub start_items: String,
    /// Host milliseconds.
    pub host_milliseconds: f64,
    /// Client slots.
    pub client_slots: Vec<i64>,
}

/// Read saved simulation settings.
pub fn saved_simulation_settings(
    image: &SimulationSaveImage,
    q3_clients: &SavedQ3GuestClientsFn,
) -> Result<SavedSimulationSettings, SimulationSaveError> {
    let document = simulation_save_reader(image)?;
    let reader = document.reader();
    let settings = reader.field("settings");
    let guest = simulation_qvm_checkpoint(image)?;
    let optional = |field: SaveReader<'_>| {
        if field.is_missing() {
            Ok(String::new())
        } else {
            field.string().map_err(mapped)
        }
    };
    let client_slots =
        if native_q2_original_save(image, None)?.is_some() || native_q2_rerelease_save(image, None)?.is_some() {
            native_q2_saved_clients(image)?
                .into_iter()
                .map(|client| client.client_slot)
                .collect()
        } else if let Some(qvm) = guest {
            q3_clients(&qvm)
                .map_err(mapped)?
                .into_iter()
                .map(|client| client.slot)
                .collect()
        } else {
            reader
                .field("players")
                .list(|value| value.field("clientSlot").integer(0).map_err(mapped))
                .map_err(mapped)?
        };
    Ok(SavedSimulationSettings {
        skill: settings.field("skill").choice_i64(&[0, 1, 2, 3]).map_err(mapped)?,
        mode: settings
            .field("mode")
            .choice_str(&["singleplayer", "coop", "deathmatch"])
            .map_err(mapped)?,
        max_clients: settings.field("maxClients").integer(1).map_err(mapped)?,
        seed: settings.field("seed").integer(0).map_err(mapped)?,
        initial_spawn_point: optional(settings.field("initialSpawnPoint"))?,
        start_items: optional(settings.field("startItems"))?,
        host_milliseconds: reader.field("hostMilliseconds").finite().map_err(mapped)?,
        client_slots,
    })
}

#[cfg(test)]
mod tests {
    use qa_world::save::records::write_saved_actor;
    use qa_world::save::value::{arr, encode_checkpoint_value, int, num, obj, str as json_str, SaveJson};

    use super::*;

    const ENTITIES: &str = "map:q2dm1";

    fn provider(schema: &str, value: &SaveJson) -> ProviderCheckpoint {
        let (owner, version) = match schema {
            "world:source-slots" => ("world:actors".to_string(), 1),
            "world:simulation" => (ENTITIES.to_string(), 11),
            _ => (ENTITIES.to_string(), 1),
        };
        ProviderCheckpoint {
            provider: owner,
            schema: schema.to_string(),
            version,
            bytes: encode_checkpoint_value(value),
        }
    }

    fn simulation_doc(players: SaveJson) -> SaveJson {
        obj(vec![
            (
                "settings",
                obj(vec![
                    ("skill", int(2)),
                    ("mode", json_str("deathmatch")),
                    ("maxClients", int(8)),
                    ("seed", int(42)),
                ]),
            ),
            ("hostMilliseconds", num(1000.0)),
            ("players", players),
            ("nativeClients", arr(vec![])),
        ])
    }

    fn execution(kind: SimExecutionKind, api_kind: &str, api_version: i64) -> SimSavedExecution {
        SimSavedExecution {
            role: "server-game".to_string(),
            kind,
            api_kind: api_kind.to_string(),
            api_version,
            profile_kind: String::new(),
            artifact_digest: "sha256:abc".to_string(),
            artifact_path: "progs.dat".to_string(),
            owner_provider: ENTITIES.to_string(),
            mount_id: String::new(),
            mount_generation: String::new(),
        }
    }

    fn image(execution: SimSavedExecution, providers: Vec<ProviderCheckpoint>) -> SimulationSaveImage {
        let frame_time = num(1.0);
        SimulationSaveImage {
            providers,
            recipe: SimSavedRecipe {
                execution: vec![execution],
                map_entities_provider: ENTITIES.to_string(),
                map_geometry_path: "maps/q2dm1.bsp".to_string(),
            },
            guests: Vec::new(),
            clocks: vec![SimSaveClock {
                provider: ENTITIES.to_string(),
                time: frame_time.clone(),
            }],
            random: vec![SimSaveRandom {
                provider: ENTITIES.to_string(),
                state: qa_world::save::shared::SaveRandomState::GlibcRandom {
                    words: Vec::new(),
                    front: 0,
                    rear: 0,
                    draws: 0,
                },
            }],
            bodies: Vec::new(),
            combat: Vec::new(),
            inventories: Vec::new(),
            configurations: Vec::new(),
            thinks: Vec::new(),
            frame_time,
            mods: None,
            schema_version: 3,
            legacy_armor_layout: false,
            frame: qa_core::time::FrameContext {
                frame: 0,
                time: qa_core::time::SourceTime::Seconds(0.0),
                elapsed: qa_core::time::SourceTime::Seconds(0.0),
                phase: qa_core::time::FramePhase::FrameEntry,
            },
            next_event_sequence: 0,
            actors: Vec::new(),
        }
    }

    fn quake_module() -> ContentModuleIdentity {
        ContentModuleIdentity {
            id: provider_id(ENTITIES),
            artifact_path: "progs.dat".to_string(),
            digest: ContentDigest("sha256:abc".to_string()),
            revision: "sha256:abc".to_string(),
        }
    }

    fn quake_checkpoint() -> QuakeCCheckpoint {
        QuakeCCheckpoint {
            module: quake_module(),
            random: Vec::new(),
            api: QuakeCApiIdentity::Netquake,
            globals: Vec::new(),
            entities: Vec::new(),
            entity_stride_bytes: 0,
            entity_count: 0,
            strings: Vec::new(),
            statement: 0,
            function_index: 0,
            argument_count: 0,
            call_stack: Vec::new(),
            locals: Vec::new(),
            host_state: qa_content::contract::GuestPrivateState {
                module: quake_module(),
                format: "q1:progs".to_string(),
                bytes: Vec::new(),
            },
        }
    }

    fn fail_decode(_: &SaveJson) -> Result<DecodedApplicationBotsCheckpoint, SimulationSaveError> {
        panic!("decode must not run")
    }

    #[test]
    fn provider_checkpoint_matches_contract() {
        let image = image(
            execution(SimExecutionKind::Builtin, "q1-netquake", 6),
            vec![provider("world:simulation", &simulation_doc(arr(vec![])))],
        );
        let record = simulation_provider_checkpoint(&image, "world:simulation").expect("record");
        assert_eq!(record.version, 11);
        assert!(simulation_provider_checkpoint(&image, "world:missing").is_err());
    }

    #[test]
    fn guest_checkpoint_reads_quakec() {
        let mut image = image(
            execution(SimExecutionKind::Quakec, "q1-netquake", 6),
            vec![
                provider("world:simulation", &simulation_doc(arr(vec![]))),
                provider("world:source-slots", &obj(vec![])),
            ],
        );
        image.guests.push(SimSavedGuest {
            checkpoint: SimGuestCheckpoint::QuakeC(quake_checkpoint()),
            host_module: quake_module(),
        });
        let guest = simulation_guest_checkpoint(&image).expect("guest").expect("some");
        assert_eq!(guest.checkpoint.kind(), "quakec");
        assert!(simulation_qvm_checkpoint(&image).expect("qvm").is_none());
        assert!(simulation_quakec_checkpoint(&image).expect("qc").is_some());
    }

    #[test]
    fn guest_checkpoint_rejects_strays() {
        let mut image = image(
            execution(SimExecutionKind::Native, "q2-classic-game", 3),
            vec![
                provider("world:simulation", &simulation_doc(arr(vec![]))),
                provider("world:source-slots", &obj(vec![])),
            ],
        );
        image.guests.push(SimSavedGuest {
            checkpoint: SimGuestCheckpoint::QuakeC(quake_checkpoint()),
            host_module: quake_module(),
        });
        assert!(simulation_guest_checkpoint(&image).is_err());
    }

    #[test]
    fn source_cvars_absent_without_providers() {
        let image = image(
            execution(SimExecutionKind::Builtin, "q1-netquake", 6),
            vec![
                provider("world:simulation", &simulation_doc(arr(vec![]))),
                provider("world:source-slots", &obj(vec![])),
            ],
        );
        assert!(saved_source_cvars(&image).expect("cvars").is_none());
    }

    #[test]
    fn bots_absent_without_provider() {
        let image = image(
            execution(SimExecutionKind::Builtin, "q3-qagame", 8),
            vec![
                provider("world:simulation", &simulation_doc(arr(vec![]))),
                provider("world:source-slots", &obj(vec![])),
            ],
        );
        let decode: DecodeApplicationBotsCheckpointFn = Rc::new(fail_decode);
        assert!(saved_bot_checkpoint(&image, &decode).expect("bots").is_none());
    }

    #[test]
    fn bots_validate_connections() {
        let actor = SavedActorId { slot: 2, generation: 1 };
        let players = arr(vec![obj(vec![
            ("clientSlot", int(3)),
            ("actor", write_saved_actor(actor)),
        ])]);
        let image = image(
            execution(SimExecutionKind::Builtin, "q3-qagame", 8),
            vec![
                provider("world:simulation", &simulation_doc(players)),
                provider("world:source-slots", &obj(vec![])),
                provider("world:bots", &obj(vec![])),
            ],
        );
        let decode: DecodeApplicationBotsCheckpointFn = Rc::new(move |_| {
            Ok(DecodedApplicationBotsCheckpoint {
                version: 1,
                transport: ApplicationBotTransportCheckpoint {
                    version: 1,
                    elapsed_milliseconds: 0.0,
                    connections: vec![BotConnection {
                        client_slot: 3,
                        client_generation: 1,
                        actor,
                        reliable_sequence: 0,
                        reliable_acknowledge: 0,
                        reliable_slots: Vec::new(),
                    }],
                    snapshots: Vec::new(),
                },
                director: SaveJson::Null,
                navigation: SaveJson::Null,
                knowledge: SaveJson::Null,
                shared_world: SaveJson::Null,
                observations: Vec::new(),
            })
        });
        let bots = saved_bot_checkpoint(&image, &decode).expect("bots").expect("some");
        assert_eq!(bots.transport.connections.len(), 1);
    }

    #[test]
    fn validate_accepts_typescript_q1() {
        let image = image(
            execution(SimExecutionKind::Builtin, "q1-netquake", 6),
            vec![
                provider("world:simulation", &simulation_doc(arr(vec![]))),
                provider("world:source-slots", &obj(vec![])),
                provider("q1:foundation", &obj(vec![])),
            ],
        );
        let decode: DecodeApplicationBotsCheckpointFn = Rc::new(fail_decode);
        validate_simulation_save(&image, &decode).expect("valid");
    }

    #[test]
    fn validate_rejects_duplicate_providers() {
        let image = image(
            execution(SimExecutionKind::Builtin, "q1-netquake", 6),
            vec![
                provider("world:simulation", &simulation_doc(arr(vec![]))),
                provider("world:simulation", &simulation_doc(arr(vec![]))),
                provider("world:source-slots", &obj(vec![])),
            ],
        );
        let decode: DecodeApplicationBotsCheckpointFn = Rc::new(fail_decode);
        assert!(validate_simulation_save(&image, &decode).is_err());
    }

    #[test]
    fn validate_rejects_clock_mismatch() {
        let mut image = image(
            execution(SimExecutionKind::Builtin, "q1-netquake", 6),
            vec![
                provider("world:simulation", &simulation_doc(arr(vec![]))),
                provider("world:source-slots", &obj(vec![])),
                provider("q1:foundation", &obj(vec![])),
            ],
        );
        image.frame_time = num(2.0);
        let decode: DecodeApplicationBotsCheckpointFn = Rc::new(fail_decode);
        assert!(validate_simulation_save(&image, &decode).is_err());
    }

    #[test]
    fn settings_read_players_and_defaults() {
        let players = arr(vec![
            obj(vec![("clientSlot", int(1))]),
            obj(vec![("clientSlot", int(2))]),
        ]);
        let image = image(
            execution(SimExecutionKind::Builtin, "q1-netquake", 6),
            vec![
                provider("world:simulation", &simulation_doc(players)),
                provider("world:source-slots", &obj(vec![])),
            ],
        );
        let q3: SavedQ3GuestClientsFn = Rc::new(|_| panic!("no qvm guest"));
        let settings = saved_simulation_settings(&image, &q3).expect("settings");
        assert_eq!(settings.skill, 2);
        assert_eq!(settings.mode, "deathmatch");
        assert_eq!(settings.max_clients, 8);
        assert_eq!(settings.seed, 42);
        assert_eq!(settings.initial_spawn_point, "");
        assert_eq!(settings.client_slots, vec![1, 2]);
    }
}
