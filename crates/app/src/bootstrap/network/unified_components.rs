//! Unified component state and frame codecs.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-components.ts`
//! (`readComponentOwner`, `writeComponentUpdate`, `readComponentUpdate`,
//! `writeComponentFrames`, `readComponentFrames`).
//!
//! Reliable component state carries original configstrings and commands —
//! no locally generated side effects. Snapshots embed complete QVM
//! `playerState_t` / `entityState_t` records via [`qa_guest`]; game-state
//! records reuse [`GameStateRecord`](qa_guest::qvm::client_state::GameStateRecord),
//! scene rows reuse [`qa_guest::qvm::mod_presentation`], and component
//! identity reuses the app [`ModIdentity`](crate::persistence::mods) reader.
//! Presentation-context shapes are local mirrors carrying exactly the fields
//! the publisher and consumer exchange (client number, game state,
//! snapshot, weapon flag, scene data, bindings).

use std::collections::HashSet;

use qa_content::contract::{same_presentation_owner, PresentationOwner};
use qa_core::identity::{ActorId, ProviderId};
use qa_guest::error::GuestError;
use qa_guest::qvm::client_state::GameStateRecord;
use qa_guest::qvm::entity_record::{
    qvm_entity_state_bytes, read_source_qvm_entity_state, write_source_qvm_entity_state, QvmEntityState,
};
use qa_guest::qvm::game_data::AbiProfile;
use qa_guest::qvm::mod_presentation::{QvmSceneActor, QvmSceneCommand};
use qa_guest::qvm::player_record::{
    qvm_player_state_bytes, read_source_qvm_player_state, write_source_qvm_player_state, QvmPlayerState,
};
use qa_world::save::value::{arr, boolean, int, namespaced, obj, str as json_str, SaveJson, SaveReader};
use qa_world::WorldError;
use thiserror::Error;

use super::unified_frame_values::{read_actor, wire_actor};
use super::unified_native_components::{
    read_native_frames, read_native_states, write_native_frames, write_native_states, UnifiedNativeFrame,
    UnifiedNativeState,
};
use super::unified_types::UnifiedIdentityDecoder;
use crate::persistence::mods::{read_mod_identity, ModIdentity};
use crate::persistence::PersistenceError;

/// Unified component codec failure.
#[derive(Debug, Error)]
pub enum UnifiedComponentError {
    /// Checkpoint value failure.
    #[error(transparent)]
    World(#[from] WorldError),
    /// QVM record failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
    /// Mod identity failure.
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
}

/// Component ABI profile (donor `QvmAbiProfile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedComponentAbi {
    /// Modern Quake III ABI (`q3-modern`).
    Modern,
    /// Legacy 1.16n ABI (`q3-1.16n-base`).
    Legacy116n,
}

impl UnifiedComponentAbi {
    fn profile(self) -> AbiProfile {
        match self {
            Self::Modern => AbiProfile::Modern,
            Self::Legacy116n => AbiProfile::Legacy,
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::Modern => "q3-modern",
            Self::Legacy116n => "q3-1.16n-base",
        }
    }
}

/// Component runtime (donor `UnifiedComponentIdentity['runtime']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedComponentRuntime {
    /// Scene runtime (`qvm-scene`).
    Scene,
    /// Player-events runtime (`qvm-player-events`).
    PlayerEvents,
}

impl UnifiedComponentRuntime {
    fn text(self) -> &'static str {
        match self {
            Self::Scene => "qvm-scene",
            Self::PlayerEvents => "qvm-player-events",
        }
    }
}

/// Component identity (donor `UnifiedComponentIdentity`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentIdentity {
    /// Presenting owner.
    pub owner: PresentationOwner,
    /// Component identity.
    pub identity: ModIdentity,
    /// Activation generation.
    pub generation: i64,
    /// ABI profile.
    pub abi: UnifiedComponentAbi,
    /// Runtime.
    pub runtime: UnifiedComponentRuntime,
}

/// Published component context (donor `UnifiedComponentPublication['context']`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentContext {
    /// Client number override.
    pub client_number: Option<i64>,
    /// Game-state revision.
    pub game_state_revision: i64,
    /// Game-state record.
    pub game_state: GameStateRecord,
    /// Viewer snapshot.
    pub snapshot: UnifiedComponentSnapshot,
    /// Weapon-presented flag.
    pub weapon_presented: bool,
    /// Scene data, for scene runtimes.
    pub scene: Option<UnifiedComponentSceneData>,
}

/// Viewer snapshot (donor `QvmPresentationContext['snapshot']`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentSnapshot {
    /// Server time.
    pub server_time: i64,
    /// Viewer player state.
    pub player_state: QvmPlayerState,
}

/// Scene snapshot data (donor `QvmSceneContext['snapshot']`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSceneSnapshotData {
    /// Server time.
    pub server_time: i64,
    /// Snapshot flags.
    pub flags: i64,
    /// Area mask (32 bytes).
    pub area_mask: Vec<u8>,
    /// Viewer player state.
    pub player_state: QvmPlayerState,
    /// Visible entities.
    pub entities: Vec<QvmEntityState>,
    /// Server command sequence.
    pub server_command_sequence: i64,
}

/// Published scene data (donor `QvmSceneContext` subset).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentSceneData {
    /// Scene revision.
    pub revision: i64,
    /// Scene snapshot.
    pub snapshot: UnifiedSceneSnapshotData,
    /// Retained server commands.
    pub commands: Vec<QvmSceneCommand>,
}

/// Published component source (donor `UnifiedComponentPublication`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentPublication {
    /// Component identity.
    pub identity_block: UnifiedComponentIdentity,
    /// Viewing actor.
    pub viewer: ActorId,
    /// Published context.
    pub context: UnifiedComponentContext,
    /// Scene bindings.
    pub bindings: Vec<QvmSceneActor>,
}

/// Reliable component state (donor `UnifiedComponentState`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentState {
    /// Component identity.
    pub identity_block: UnifiedComponentIdentity,
    /// Game-state revision.
    pub game_state_revision: i64,
    /// Full game state on first publish or revision change.
    pub game_state: Option<GameStateRecord>,
    /// Command window base.
    pub command_base: i64,
    /// Commands after the base.
    pub commands: Vec<QvmSceneCommand>,
}

/// Component frame (donor `UnifiedComponentFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentFrame {
    /// Presenting owner.
    pub owner: PresentationOwner,
    /// Activation generation.
    pub generation: i64,
    /// ABI profile.
    pub abi: UnifiedComponentAbi,
    /// Viewing actor.
    pub viewer: ActorId,
    /// Client number.
    pub client_number: i64,
    /// Game-state revision.
    pub game_state_revision: i64,
    /// Viewer snapshot.
    pub snapshot: UnifiedComponentSnapshot,
    /// Weapon-presented flag.
    pub weapon_presented: bool,
    /// Scene bindings.
    pub bindings: Vec<QvmSceneActor>,
    /// Frame scene, for scene runtimes.
    pub scene: Option<UnifiedComponentFrameScene>,
}

/// Frame scene (donor `UnifiedComponentFrame['scene']`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentFrameScene {
    /// Scene revision.
    pub revision: i64,
    /// Scene snapshot.
    pub snapshot: UnifiedSceneSnapshotData,
}

/// Reliable component update (donor `UnifiedComponentUpdate`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentUpdate {
    /// Reliable revision.
    pub revision: i64,
    /// Component states.
    pub sources: Vec<UnifiedComponentState>,
    /// Native states.
    pub native: Vec<UnifiedNativeState>,
}

/// Component frames (donor `UnifiedComponentFrames`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentFrames {
    /// Reliable revision.
    pub revision: i64,
    /// Component frames.
    pub sources: Vec<UnifiedComponentFrame>,
    /// Native frames.
    pub native: Option<Vec<UnifiedNativeFrame>>,
}

fn integer(reader: SaveReader, minimum: i64, maximum: i64) -> Result<i64, WorldError> {
    let value = reader.integer(minimum)?;
    if value <= maximum {
        Ok(value)
    } else {
        Err(reader.fail("component integer exceeds its range"))
    }
}

fn read_provider(reader: SaveReader) -> Result<ProviderId, WorldError> {
    let provider = namespaced(reader.clone())?;
    let (namespace, name) = provider.split_once(':').unwrap_or(("", ""));
    if namespace.is_empty() || name.is_empty() {
        return Err(reader.fail("invalid provider reference"));
    }
    Ok(ProviderId::new(namespace, name))
}

/// Read a component owner (donor `readComponentOwner`).
pub fn read_component_owner(reader: SaveReader) -> Result<PresentationOwner, WorldError> {
    let generation = integer(reader.field("generation"), 1, i64::MAX)?;
    Ok(PresentationOwner {
        provider: read_provider(reader.field("provider"))?,
        generation: generation as u64,
    })
}

fn write_owner(owner: &PresentationOwner) -> SaveJson {
    obj(vec![
        (
            "provider",
            json_str(&format!("{}:{}", owner.provider.namespace, owner.provider.name)),
        ),
        ("generation", int(owner.generation as i64)),
    ])
}

fn read_abi(reader: SaveReader) -> Result<UnifiedComponentAbi, WorldError> {
    Ok(match reader.choice_str(&["q3-modern", "q3-1.16n-base"])?.as_str() {
        "q3-modern" => UnifiedComponentAbi::Modern,
        _ => UnifiedComponentAbi::Legacy116n,
    })
}

fn read_runtime(reader: SaveReader) -> Result<UnifiedComponentRuntime, WorldError> {
    Ok(match reader.choice_str(&["qvm-scene", "qvm-player-events"])?.as_str() {
        "qvm-scene" => UnifiedComponentRuntime::Scene,
        _ => UnifiedComponentRuntime::PlayerEvents,
    })
}

fn exact_bytes(reader: SaveReader, length: usize) -> Result<Vec<u8>, WorldError> {
    let result = reader.bytes()?;
    if result.len() == length {
        Ok(result)
    } else {
        Err(reader.fail("invalid original source record extent"))
    }
}

fn write_game_state(value: &GameStateRecord) -> SaveJson {
    obj(vec![
        (
            "stringOffsets",
            arr(value
                .string_offsets
                .iter()
                .map(|offset| int(i64::from(*offset)))
                .collect()),
        ),
        ("stringData", SaveJson::Bytes(value.string_data.clone())),
        ("dataCount", int(i64::from(value.data_count))),
    ])
}

fn read_game_state(reader: SaveReader) -> Result<GameStateRecord, WorldError> {
    let count = integer(reader.field("dataCount"), 0, 16000)?;
    let data = exact_bytes(reader.field("stringData"), 16000)?;
    let offsets = reader
        .field("stringOffsets")
        .list(|offset| integer(offset, 0, count.saturating_sub(1).max(0)))?;
    if offsets.len() != 1024 || data.first() != Some(&0) {
        return Err(reader.fail("invalid original gameState_t"));
    }
    for offset in &offsets {
        if *offset != 0 {
            let start = *offset as usize;
            let end = data
                .iter()
                .skip(start)
                .position(|byte| *byte == 0)
                .map(|index| start + index);
            match end {
                Some(terminator) if (terminator as i64) < count => {}
                _ => return Err(reader.fail("unterminated original configstring")),
            }
        }
    }
    Ok(GameStateRecord {
        string_offsets: offsets.into_iter().map(|offset| offset as i32).collect(),
        string_data: data,
        data_count: count as i32,
    })
}

fn read_commands(reader: SaveReader) -> Result<Vec<QvmSceneCommand>, WorldError> {
    let commands = reader.list(|value| {
        Ok(QvmSceneCommand {
            sequence: integer(value.field("sequence"), 1, i64::from(i32::MAX))? as u64,
            arguments: value.field("arguments").list(|argument| {
                let text = argument.string()?;
                if text.len() <= 8192 && !text.contains('\0') {
                    Ok(text)
                } else {
                    Err(argument.fail("invalid component command argument"))
                }
            })?,
        })
    })?;
    if commands.len() > 64
        || commands.iter().any(|command| command.arguments.len() > 128)
        || commands.windows(2).any(|pair| pair[1].sequence != pair[0].sequence + 1)
    {
        return Err(reader.fail("invalid original reliable command window"));
    }
    Ok(commands)
}

fn write_commands(commands: &[QvmSceneCommand]) -> SaveJson {
    arr(commands
        .iter()
        .map(|command| {
            obj(vec![
                ("sequence", int(command.sequence as i64)),
                (
                    "arguments",
                    arr(command.arguments.iter().map(|argument| json_str(argument)).collect()),
                ),
            ])
        })
        .collect())
}

fn write_identity(identity: &UnifiedComponentIdentity) -> Vec<(&str, SaveJson)> {
    vec![
        ("owner", write_owner(&identity.owner)),
        (
            "identity",
            crate::persistence::mods::write_mod_identity(&identity.identity),
        ),
        ("generation", int(identity.generation)),
        ("abi", json_str(identity.abi.text())),
        ("runtime", json_str(identity.runtime.text())),
    ]
}

fn read_identity(reader: SaveReader) -> Result<UnifiedComponentIdentity, WorldError> {
    Ok(UnifiedComponentIdentity {
        owner: read_component_owner(reader.field("owner"))?,
        identity: read_mod_identity(reader.field("identity"))
            .map_err(|error| reader.field("identity").fail(&error.to_string()))?,
        generation: integer(reader.field("generation"), 0, i64::MAX)?,
        abi: read_abi(reader.field("abi"))?,
        runtime: read_runtime(reader.field("runtime"))?,
    })
}

/// Encode a reliable component update (donor `writeComponentUpdate`).
#[must_use]
pub fn write_component_update(update: &UnifiedComponentUpdate) -> SaveJson {
    obj(vec![
        ("revision", int(update.revision)),
        ("native", write_native_states(&update.native)),
        (
            "sources",
            arr(update
                .sources
                .iter()
                .map(|source| {
                    let mut members = write_identity(&source.identity_block);
                    members.push(("gameStateRevision", int(source.game_state_revision)));
                    members.push((
                        "gameState",
                        source.game_state.as_ref().map_or(SaveJson::Null, write_game_state),
                    ));
                    members.push(("commandBase", int(source.command_base)));
                    members.push(("commands", write_commands(&source.commands)));
                    obj(members)
                })
                .collect()),
        ),
    ])
}

fn check_owners(reader: SaveReader, providers: &[&PresentationOwner]) -> Result<(), WorldError> {
    let distinct: HashSet<(&str, &str)> = providers
        .iter()
        .map(|owner| (owner.provider.namespace.as_str(), owner.provider.name.as_str()))
        .collect();
    if providers.len() > 256 || distinct.len() != providers.len() {
        return Err(reader.fail("invalid component owner set"));
    }
    Ok(())
}

/// Decode a reliable component update (donor `readComponentUpdate`).
pub fn read_component_update(reader: SaveReader) -> Result<UnifiedComponentUpdate, UnifiedComponentError> {
    let sources = reader.field("sources").list(|source| {
        Ok::<_, UnifiedComponentError>(UnifiedComponentState {
            identity_block: read_identity(source.clone())?,
            game_state_revision: integer(source.field("gameStateRevision"), 0, i64::MAX)?,
            game_state: source.field("gameState").nullable(read_game_state)?,
            command_base: integer(source.field("commandBase"), 0, i64::from(i32::MAX))?,
            commands: read_commands(source.field("commands"))?,
        })
    })?;
    if sources.len() > 256 {
        return Err(reader.fail("invalid component owner set").into());
    }
    {
        let providers: Vec<&PresentationOwner> = sources.iter().map(|source| &source.identity_block.owner).collect();
        let distinct: HashSet<(&str, &str)> = providers
            .iter()
            .map(|owner| (owner.provider.namespace.as_str(), owner.provider.name.as_str()))
            .collect();
        if distinct.len() != providers.len() {
            return Err(reader.fail("invalid component owner set").into());
        }
    }
    let native = read_native_states(reader.field("native"))?;
    let mut providers: Vec<&PresentationOwner> = sources.iter().map(|source| &source.identity_block.owner).collect();
    providers.extend(native.iter().map(|state| &state.owner));
    check_owners(reader.clone(), &providers)?;
    Ok(UnifiedComponentUpdate {
        revision: integer(reader.field("revision"), 1, i64::MAX)?,
        sources,
        native,
    })
}

fn encode_player_state(state: &QvmPlayerState, abi: UnifiedComponentAbi) -> Result<Vec<u8>, GuestError> {
    let mut bytes = vec![0u8; qvm_player_state_bytes(abi.profile())];
    write_source_qvm_player_state(&mut bytes, state, abi.profile())?;
    Ok(bytes)
}

fn encode_entity_state(state: &QvmEntityState, abi: UnifiedComponentAbi) -> Result<Vec<u8>, GuestError> {
    let mut bytes = vec![0u8; qvm_entity_state_bytes(abi.profile())];
    write_source_qvm_entity_state(&mut bytes, state, abi.profile())?;
    Ok(bytes)
}

fn write_bindings(bindings: &[QvmSceneActor]) -> SaveJson {
    arr(bindings
        .iter()
        .map(|binding| {
            obj(vec![
                ("actor", wire_actor(&binding.actor)),
                ("slot", int(binding.slot as i64)),
                ("owned", boolean(binding.owned)),
            ])
        })
        .collect())
}

/// Encode component frames (donor `writeComponentFrames`).
pub fn write_component_frames(frames: &UnifiedComponentFrames) -> Result<SaveJson, UnifiedComponentError> {
    let mut sources = Vec::with_capacity(frames.sources.len());
    for source in &frames.sources {
        let player_state = SaveJson::Bytes(encode_player_state(&source.snapshot.player_state, source.abi)?);
        let scene = match &source.scene {
            None => SaveJson::Null,
            Some(scene) => {
                let mut entities = Vec::with_capacity(scene.snapshot.entities.len());
                for entity in &scene.snapshot.entities {
                    entities.push(SaveJson::Bytes(encode_entity_state(entity, source.abi)?));
                }
                obj(vec![
                    ("revision", int(scene.revision)),
                    (
                        "snapshot",
                        obj(vec![
                            ("serverTime", int(scene.snapshot.server_time)),
                            ("flags", int(scene.snapshot.flags)),
                            ("areaMask", SaveJson::Bytes(scene.snapshot.area_mask.clone())),
                            ("playerState", player_state.clone()),
                            ("entities", arr(entities)),
                            ("serverCommandSequence", int(scene.snapshot.server_command_sequence)),
                        ]),
                    ),
                ])
            }
        };
        sources.push(obj(vec![
            ("owner", write_owner(&source.owner)),
            ("generation", int(source.generation)),
            ("abi", json_str(source.abi.text())),
            ("viewer", wire_actor(&source.viewer)),
            ("clientNumber", int(source.client_number)),
            ("gameStateRevision", int(source.game_state_revision)),
            (
                "snapshot",
                obj(vec![
                    ("serverTime", int(source.snapshot.server_time)),
                    ("playerState", player_state),
                ]),
            ),
            ("weaponPresented", boolean(source.weapon_presented)),
            ("bindings", write_bindings(&source.bindings)),
            ("scene", scene),
        ]));
    }
    Ok(obj(vec![
        ("revision", int(frames.revision)),
        ("native", write_native_frames(frames.native.as_deref().unwrap_or(&[]))),
        ("sources", arr(sources)),
    ]))
}

fn read_bindings(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<Vec<QvmSceneActor>, WorldError> {
    let bindings = reader.list(|binding| {
        Ok(QvmSceneActor {
            actor: read_actor(binding.field("actor"), identity)?,
            slot: integer(binding.field("slot"), 0, 1023)? as usize,
            owned: binding.field("owned").boolean()?,
        })
    })?;
    let distinct: HashSet<usize> = bindings.iter().map(|binding| binding.slot).collect();
    if bindings.len() > 1024 || distinct.len() != bindings.len() {
        return Err(reader.fail("duplicate source actor slots"));
    }
    Ok(bindings)
}

fn read_scene_snapshot(
    reader: SaveReader,
    abi: UnifiedComponentAbi,
    player_state: &QvmPlayerState,
) -> Result<UnifiedSceneSnapshotData, UnifiedComponentError> {
    let state = reader.field("snapshot");
    let size = qvm_entity_state_bytes(abi.profile());
    let entities = state.field("entities").list(|entity| {
        let bytes = exact_bytes(entity, size)?;
        Ok::<_, UnifiedComponentError>(read_source_qvm_entity_state(&bytes, abi.profile())?)
    })?;
    if entities.len() > 256 {
        return Err(reader.fail("invalid visible source entity set").into());
    }
    let distinct: HashSet<i32> = entities.iter().map(|entity| entity.number).collect();
    if distinct.len() != entities.len() {
        return Err(reader.fail("invalid visible source entity set").into());
    }
    Ok(UnifiedSceneSnapshotData {
        server_time: integer(state.field("serverTime"), 0, i64::from(i32::MAX))?,
        flags: integer(state.field("flags"), 0, 255)?,
        area_mask: exact_bytes(state.field("areaMask"), 32)?,
        player_state: player_state.clone(),
        entities,
        server_command_sequence: integer(state.field("serverCommandSequence"), 0, i64::from(i32::MAX))?,
    })
}

/// Decode component frames (donor `readComponentFrames`).
pub fn read_component_frames(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<UnifiedComponentFrames, UnifiedComponentError> {
    let mut sources = Vec::new();
    for source in reader.field("sources").list(Ok::<_, UnifiedComponentError>)? {
        let abi = read_abi(source.field("abi"))?;
        let snapshot_field = source.field("snapshot");
        let player_bytes = exact_bytes(
            snapshot_field.field("playerState"),
            qvm_player_state_bytes(abi.profile()),
        )?;
        let player_state = read_source_qvm_player_state(&player_bytes, abi.profile())?;
        let bindings = read_bindings(source.field("bindings"), identity)?;
        let scene = source.field("scene").nullable(|scene| {
            Ok::<_, UnifiedComponentError>(UnifiedComponentFrameScene {
                revision: integer(scene.field("revision"), 0, i64::MAX)?,
                snapshot: read_scene_snapshot(scene, abi, &player_state)?,
            })
        })?;
        sources.push(UnifiedComponentFrame {
            owner: read_component_owner(source.field("owner"))?,
            generation: integer(source.field("generation"), 0, i64::MAX)?,
            abi,
            viewer: read_actor(source.field("viewer"), identity)?,
            client_number: integer(source.field("clientNumber"), 0, 1023)?,
            game_state_revision: integer(source.field("gameStateRevision"), 0, i64::MAX)?,
            snapshot: UnifiedComponentSnapshot {
                server_time: integer(snapshot_field.field("serverTime"), 0, i64::from(i32::MAX))?,
                player_state,
            },
            weapon_presented: source.field("weaponPresented").boolean()?,
            bindings,
            scene,
        });
    }
    if sources.len() > 256 {
        return Err(reader.fail("invalid component frame owner set").into());
    }
    {
        let providers: Vec<&PresentationOwner> = sources.iter().map(|source| &source.owner).collect();
        let distinct: HashSet<(&str, &str)> = providers
            .iter()
            .map(|owner| (owner.provider.namespace.as_str(), owner.provider.name.as_str()))
            .collect();
        if distinct.len() != providers.len() {
            return Err(reader.fail("invalid component frame owner set").into());
        }
    }
    let native_field = reader.field("native");
    let native = read_native_frames(native_field.clone(), identity)?;
    let mut providers: Vec<&PresentationOwner> = sources.iter().map(|source| &source.owner).collect();
    providers.extend(native.iter().map(|frame| &frame.owner));
    check_owners(reader.clone(), &providers)?;
    Ok(UnifiedComponentFrames {
        revision: integer(reader.field("revision"), 0, i64::MAX)?,
        sources,
        native: if native_field.is_missing() { None } else { Some(native) },
    })
}

/// Whether two owners match exactly (both provider and generation).
#[must_use]
pub fn same_owner(left: &PresentationOwner, right: &PresentationOwner) -> bool {
    same_presentation_owner(Some(left), right)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;

    fn owner() -> PresentationOwner {
        PresentationOwner {
            provider: ProviderId::new("test", "mod"),
            generation: 3,
        }
    }

    #[test]
    fn owner_round_trips() {
        let encoded = write_owner(&owner());
        let reader = SaveReader::new(&encoded);
        assert_eq!(read_component_owner(reader).unwrap(), owner());
    }

    #[test]
    fn owner_rejects_zero_generation() {
        let encoded = obj(vec![("provider", json_str("test:mod")), ("generation", int(0))]);
        let reader = SaveReader::new(&encoded);
        assert!(read_component_owner(reader).is_err());
    }

    #[test]
    fn same_owner_compares_provider_and_generation() {
        let other = PresentationOwner {
            provider: ProviderId::new("test", "mod"),
            generation: 4,
        };
        assert!(same_owner(&owner(), &owner()));
        assert!(!same_owner(&owner(), &other));
    }

    #[test]
    fn commands_reject_sequence_gap() {
        let encoded = arr(vec![
            obj(vec![("sequence", int(1)), ("arguments", arr(vec![]))]),
            obj(vec![("sequence", int(3)), ("arguments", arr(vec![]))]),
        ]);
        let reader = SaveReader::new(&encoded);
        assert!(read_commands(reader).is_err());
    }

    #[test]
    fn game_state_rejects_short_offsets() {
        let encoded = obj(vec![
            ("stringOffsets", arr(vec![int(0)])),
            ("stringData", SaveJson::Bytes(vec![0u8; 16000])),
            ("dataCount", int(1)),
        ]);
        let reader = SaveReader::new(&encoded);
        assert!(read_game_state(reader).is_err());
    }
}
