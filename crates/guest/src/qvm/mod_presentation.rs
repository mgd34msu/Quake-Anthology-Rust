//! QVM mod presentation: artifact-qualified original cgame presentation.
//!
//! Provenance: `src/compat/qvm/mod-presentation.ts`.
//!
//! Absorbs the pure-Rust types of `src/contracts/qvm-mod-presentation.ts`.
//! State bytes reuse [`super::mod_presentation_checkpoint`]; images and ABI
//! reuse [`super::mod_provider`]; events reuse
//! [`super::mod_player_events::SourcePlayerEvent`]. Local mirrors:
//! [`qualify_qvm_body_calls`] (from `src/compat/qvm/body-scope.ts`),
//! [`vector_to_angles`]/[`qvm_angles_to_axis`] (f32-rounded mirrors of
//! `src/core/math.ts` and `src/core/qvm-math.ts`). The cgame module,
//! interpreter hooks, and engine traps integrate through [`PresentationHost`];
//! body-mesh and event-check hook points are explicit methods the host calls.

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::{vec3, Vec3};

use super::mod_player_events::SourcePlayerEvent;
use super::mod_presentation_checkpoint::{
    capture_presentation_game_state, capture_presentation_snapshot, read_presentation_game_state,
    read_presentation_snapshot, read_saved_actor_id, snapshot_to_bytes, QvmSourceSnapshot, SourceGameState,
    SourcePlayerState,
};
use super::mod_provider::{
    qvm_entity_state_bytes, qvm_player_state_bytes, qvm_snapshot_bytes, ModuleId, ProfileReader, ProfileValue, QvmAbi,
    QvmArtifact, QvmImage, QvmOpcode, QVM_GAME_STATE_BYTES, QVM_MAX_PRIVATE_ARGUMENT_WORDS,
};
use crate::error::GuestError;

// ---------------------------------------------------------------------------
// Presentation contract types (`src/contracts/qvm-mod-presentation.ts`).
// ---------------------------------------------------------------------------

/// Presented program identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmPresentationProgram {
    /// Artifact path.
    pub path: String,
    /// Artifact digest.
    pub digest: String,
    /// ABI profile.
    pub abi: QvmAbi,
}

/// Body part of one mesh call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QvmBodyPart {
    /// Whole body.
    Body,
    /// Lower body.
    Lower,
    /// Upper body.
    Upper,
    /// Head.
    Head,
}

/// Original presentation argument.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmPresentationArgument {
    /// Signed word.
    Int32(i32),
    /// Binary32 word.
    Float32(f64),
    /// Address word.
    Address(i32),
    /// Source record word.
    Source(PresentationSource),
}

/// Source record selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentationSource {
    /// Projected player state.
    PlayerState,
    /// Projected entity state.
    EntityState,
    /// Centity record.
    Centity,
    /// Event origin.
    Origin,
    /// Synthetic snapshot.
    Snapshot,
    /// Client number.
    ClientNumber,
    /// Caller time.
    Time,
    /// Event id.
    Event,
    /// Event parameter.
    Parameter,
    /// Snapshot number.
    SnapshotNumber,
    /// Server command sequence.
    ServerCommandSequence,
}

/// Original presentation call.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPresentationCall {
    /// Entry instruction.
    pub entry: usize,
    /// Weapon-presented gate, if any.
    pub when_weapon_presented: bool,
    /// Arguments.
    pub arguments: Vec<QvmPresentationArgument>,
}

/// Player-event presentation storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerEventStorage {
    /// Game-state address.
    pub game_state: usize,
    /// Player-state address.
    pub player_state: usize,
    /// Synthetic snapshot address.
    pub snapshot_address: usize,
    /// Snapshot pointer words.
    pub snapshot_pointers: Vec<usize>,
    /// Centity array.
    pub centities: PlayerEventCentities,
    /// Time words.
    pub time: Vec<usize>,
    /// Frame-time words.
    pub frame_time: Vec<usize>,
    /// View-origin words.
    pub view_origin: Vec<usize>,
    /// View-angles words, if any.
    pub view_angles: Vec<usize>,
    /// View-axis words, if any.
    pub view_axis: Vec<usize>,
}

/// Centity layout of player-event presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerEventCentities {
    /// Array address.
    pub address: usize,
    /// Row stride.
    pub stride: usize,
    /// Row capacity.
    pub capacity: usize,
    /// Entity-state offset.
    pub state: usize,
    /// Origin offset.
    pub origin: usize,
}

/// Scene presentation storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneStorage {
    /// Game-state address.
    pub game_state: usize,
    /// Server command sequence address.
    pub server_command_sequence: usize,
    /// Time words.
    pub time: Vec<usize>,
    /// Frame-time words.
    pub frame_time: Vec<usize>,
    /// View-origin words.
    pub view_origin: Vec<usize>,
    /// View-angles words, if any.
    pub view_angles: Vec<usize>,
    /// View-axis words, if any.
    pub view_axis: Vec<usize>,
    /// Centity array.
    pub centities: SceneCentities,
}

/// Centity layout of scene presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneCentities {
    /// Array address.
    pub address: usize,
    /// Row stride.
    pub stride: usize,
    /// Row capacity.
    pub capacity: usize,
    /// Entity-state offset.
    pub state: usize,
    /// Previous-event offset.
    pub previous_event: usize,
    /// Snapshot-time offset.
    pub snapshot_time: usize,
}

/// HUD mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HudMode {
    /// Overlay.
    Overlay,
    /// Replace status.
    ReplaceStatus,
}

/// HUD frame calls.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentationHud {
    /// Mode.
    pub mode: HudMode,
    /// Frame calls.
    pub frame: Vec<QvmPresentationCall>,
}

/// Player-event presentation declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmPlayerEventPresentation {
    /// Gameplay program.
    pub gameplay: QvmPresentationProgram,
    /// Cgame program.
    pub cgame: QvmPresentationProgram,
    /// Initialization calls.
    pub initialize: Vec<QvmPresentationCall>,
    /// Refresh calls.
    pub refresh: Vec<QvmPresentationCall>,
    /// Frame calls.
    pub frame: Vec<QvmPresentationCall>,
    /// HUD calls, if any.
    pub hud: Option<PresentationHud>,
    /// Storage.
    pub storage: PlayerEventStorage,
    /// Projection calls.
    pub project: Vec<QvmPresentationCall>,
    /// Event call.
    pub event: QvmPresentationCall,
}

/// Scene body mesh scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneBodyScope {
    /// Player entry and argument.
    pub player: SceneBodyEndpoint,
    /// Mesh entry, arguments, and parts.
    pub mesh: SceneMeshEndpoint,
}

/// Scene body endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneBodyEndpoint {
    /// Entry instruction.
    pub entry: usize,
    /// Centity argument.
    pub centity_argument: usize,
}

/// Scene mesh endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneMeshEndpoint {
    /// Entry instruction.
    pub entry: usize,
    /// Entity argument.
    pub entity_argument: usize,
    /// State argument.
    pub state_argument: usize,
    /// Shader field offset.
    pub shader_offset: usize,
    /// Call-site parts, if any.
    pub parts: Option<Vec<MeshPartSite>>,
}

/// Mesh call-site part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshPartSite {
    /// Call instruction.
    pub call: usize,
    /// Body part.
    pub part: QvmBodyPart,
}

/// Scene presentation declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmScenePresentation {
    /// Gameplay program.
    pub gameplay: QvmPresentationProgram,
    /// Cgame program.
    pub cgame: QvmPresentationProgram,
    /// Initialization calls.
    pub initialize: Vec<QvmPresentationCall>,
    /// Refresh calls.
    pub refresh: Vec<QvmPresentationCall>,
    /// Frame calls.
    pub frame: Vec<QvmPresentationCall>,
    /// HUD calls, if any.
    pub hud: Option<PresentationHud>,
    /// Cvars.
    pub cvars: Vec<(String, String)>,
    /// Storage.
    pub storage: SceneStorage,
    /// Snapshot calls.
    pub snapshots: Vec<QvmPresentationCall>,
    /// Event entity-type boundary.
    pub event_entity_type: i32,
    /// Event-check hook.
    pub event_check: SceneBodyEndpoint,
    /// Body scope.
    pub body: SceneBodyScope,
}

/// Mod presentation declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModPresentationDeclaration {
    /// Player-event runtime.
    PlayerEvents(QvmPlayerEventPresentation),
    /// Scene runtime.
    Scene(QvmScenePresentation),
}

impl QvmModPresentationDeclaration {
    /// Whether this is the scene runtime.
    #[must_use]
    pub const fn is_scene(&self) -> bool {
        matches!(self, Self::Scene(_))
    }

    /// Cgame program.
    #[must_use]
    pub fn cgame(&self) -> &QvmPresentationProgram {
        match self {
            Self::PlayerEvents(declaration) => &declaration.cgame,
            Self::Scene(declaration) => &declaration.cgame,
        }
    }

    /// Game-state base address.
    #[must_use]
    pub fn game_state_address(&self) -> usize {
        match self {
            Self::PlayerEvents(declaration) => declaration.storage.game_state,
            Self::Scene(declaration) => declaration.storage.game_state,
        }
    }

    /// Time words.
    #[must_use]
    pub fn time_words(&self) -> &[usize] {
        match self {
            Self::PlayerEvents(declaration) => &declaration.storage.time,
            Self::Scene(declaration) => &declaration.storage.time,
        }
    }

    /// Frame-time words.
    #[must_use]
    pub fn frame_time_words(&self) -> &[usize] {
        match self {
            Self::PlayerEvents(declaration) => &declaration.storage.frame_time,
            Self::Scene(declaration) => &declaration.storage.frame_time,
        }
    }

    /// View-origin words.
    #[must_use]
    pub fn view_origin_words(&self) -> &[usize] {
        match self {
            Self::PlayerEvents(declaration) => &declaration.storage.view_origin,
            Self::Scene(declaration) => &declaration.storage.view_origin,
        }
    }

    /// View-angles words.
    #[must_use]
    pub fn view_angles_words(&self) -> &[usize] {
        match self {
            Self::PlayerEvents(declaration) => &declaration.storage.view_angles,
            Self::Scene(declaration) => &declaration.storage.view_angles,
        }
    }

    /// View-axis words.
    #[must_use]
    pub fn view_axis_words(&self) -> &[usize] {
        match self {
            Self::PlayerEvents(declaration) => &declaration.storage.view_axis,
            Self::Scene(declaration) => &declaration.storage.view_axis,
        }
    }

    /// Initialization calls.
    #[must_use]
    pub fn initialize_calls(&self) -> &[QvmPresentationCall] {
        match self {
            Self::PlayerEvents(declaration) => &declaration.initialize,
            Self::Scene(declaration) => &declaration.initialize,
        }
    }

    /// Refresh calls.
    #[must_use]
    pub fn refresh_calls(&self) -> &[QvmPresentationCall] {
        match self {
            Self::PlayerEvents(declaration) => &declaration.refresh,
            Self::Scene(declaration) => &declaration.refresh,
        }
    }

    /// Frame calls.
    #[must_use]
    pub fn frame_calls(&self) -> &[QvmPresentationCall] {
        match self {
            Self::PlayerEvents(declaration) => &declaration.frame,
            Self::Scene(declaration) => &declaration.frame,
        }
    }

    /// HUD calls, if any.
    #[must_use]
    pub fn hud(&self) -> Option<&PresentationHud> {
        match self {
            Self::PlayerEvents(declaration) => declaration.hud.as_ref(),
            Self::Scene(declaration) => declaration.hud.as_ref(),
        }
    }

    /// Centity array layout.
    #[must_use]
    pub fn centities(&self) -> (usize, usize, usize) {
        match self {
            Self::PlayerEvents(declaration) => {
                let entities = &declaration.storage.centities;
                (entities.address, entities.stride, entities.capacity)
            }
            Self::Scene(declaration) => {
                let entities = &declaration.storage.centities;
                (entities.address, entities.stride, entities.capacity)
            }
        }
    }
}

/// Scene actor row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmSceneActor {
    /// Actor.
    pub actor: ActorId,
    /// Centity slot.
    pub slot: usize,
    /// Owned flag.
    pub owned: bool,
}

/// Scene server command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmSceneCommand {
    /// Sequence.
    pub sequence: u64,
    /// Arguments.
    pub arguments: Vec<String>,
}

/// Scene context published by the gameplay mod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmSceneContext {
    /// Scene revision.
    pub revision: u64,
    /// Game state.
    pub game_state: SourceGameState,
    /// Game-state revision.
    pub game_state_revision: u64,
    /// Snapshot (number is always zero here).
    pub snapshot: QvmSourceSnapshot,
    /// Scene actors.
    pub actors: Vec<QvmSceneActor>,
    /// Server commands.
    pub commands: Vec<QvmSceneCommand>,
    /// Baseline scene, if restoring.
    pub baseline: Option<Box<QvmSceneContext>>,
}

/// Snapshot half of a presentation context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationSnapshot {
    /// Server time.
    pub server_time: i32,
    /// Viewer player state.
    pub player_state: SourcePlayerState,
}

/// Presentation context of one call.
#[derive(Debug, Clone)]
pub struct QvmPresentationContext {
    /// Client number override, if any.
    pub client_number: Option<i32>,
    /// Game state.
    pub game_state: SourceGameState,
    /// Game-state revision.
    pub game_state_revision: u64,
    /// Frame time in milliseconds.
    pub frame_time_ms: i32,
    /// Caller time in milliseconds, if any.
    pub time_ms: Option<i32>,
    /// View origin.
    pub view_origin: Vec3,
    /// View axis, if any.
    pub view_axis: Option<[Vec3; 3]>,
    /// Weapon-presented flag, if any.
    pub weapon_presented: Option<bool>,
    /// Snapshot.
    pub snapshot: PresentationSnapshot,
    /// Scene, if any.
    pub scene: Option<QvmSceneContext>,
}

/// One captured body mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmBodyMesh {
    /// Body part.
    pub part: QvmBodyPart,
    /// Base pass submitted.
    pub base: bool,
    /// Extra model passes (raw ref-entity bytes each).
    pub passes: Vec<BodyMeshPass>,
}

/// One extra model pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyMeshPass {
    /// Custom shader.
    pub custom_shader: i32,
    /// Raw ref-entity bytes.
    pub bytes: Vec<u8>,
}

/// Body presentation of one actor.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmBodyPresentation {
    /// Actor.
    pub actor: ActorId,
    /// Mesh parts.
    pub parts: Vec<QvmBodyMesh>,
}

/// Decoded ref-entity view supplied by the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefEntityView {
    /// Model kind flag.
    pub is_model: bool,
    /// Custom shader.
    pub custom_shader: i32,
    /// Raw bytes.
    pub bytes: Vec<u8>,
}

// ---------------------------------------------------------------------------
// View math (f32-rounded mirrors of `core/math.ts` and `core/qvm-math.ts`).
// ---------------------------------------------------------------------------

const ANGLE_RADIANS: f32 = std::f32::consts::PI * 2.0 / 360.0;
const QVM_PI: f32 = std::f32::consts::PI;

/// Dot product with f32 rounding.
#[must_use]
pub fn dot3(left: Vec3, right: Vec3) -> f32 {
    left.x * right.x + (left.y * right.y + left.z * right.z)
}

/// Forward vector to angles (pitch, yaw, roll).
#[must_use]
pub fn vector_to_angles(value: Vec3) -> Vec3 {
    let (mut yaw, mut pitch);
    if value.y == 0.0 && value.x == 0.0 {
        yaw = 0.0;
        pitch = if value.z > 0.0 { 90.0 } else { 270.0 };
    } else {
        if value.x != 0.0 {
            yaw = (f32::atan2(value.y, value.x) * 180.0) / QVM_PI;
        } else {
            yaw = if value.y > 0.0 { 90.0 } else { 270.0 };
        }
        if yaw < 0.0 {
            yaw += 360.0;
        }
        let forward = f32::sqrt(value.x * value.x + value.y * value.y);
        pitch = (f32::atan2(value.z, forward) * 180.0) / QVM_PI;
        if pitch < 0.0 {
            pitch += 360.0;
        }
    }
    vec3(-pitch, yaw, 0.0)
}

/// Angles to axis triple.
#[must_use]
pub fn qvm_angles_to_axis(angles: Vec3) -> [Vec3; 3] {
    let yaw = angles.y * ANGLE_RADIANS;
    let pitch = angles.x * ANGLE_RADIANS;
    let roll = angles.z * ANGLE_RADIANS;
    let (sy, cy) = f32::sin_cos(yaw);
    let (sp, cp) = f32::sin_cos(pitch);
    let (sr, cr) = f32::sin_cos(roll);
    let forward = vec3(cp * cy, cp * sy, -sp);
    let right = vec3(-sr * sp * cy + -cr * -sy, -sr * sp * sy + -cr * cy, -sr * cp);
    let up = vec3(cr * sp * cy + -sr * -sy, cr * sp * sy + -sr * cy, cr * cp);
    [forward, vec3(-right.x, -right.y, -right.z), up]
}

// ---------------------------------------------------------------------------
// Declaration validation.
// ---------------------------------------------------------------------------

fn check_int32(value: i32) -> Result<i32, GuestError> {
    Ok(value)
}

/// Qualify body call sites (mirror of `qualifyQvmBodyCalls`).
pub fn qualify_qvm_body_calls(
    image: &QvmImage,
    body: &SceneBodyScope,
) -> Result<HashMap<usize, QvmBodyPart>, GuestError> {
    let player = &body.player;
    let mesh = &body.mesh;
    if [player.centity_argument, mesh.entity_argument, mesh.state_argument]
        .into_iter()
        .any(|argument| argument >= QVM_MAX_PRIVATE_ARGUMENT_WORDS)
        || mesh.shader_offset != 112
    {
        return Err(GuestError::invalid(
            "Source body arguments differ from the original refEntity ABI",
        ));
    }
    if image
        .instruction(player.entry)
        .is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
        || image
            .instruction(mesh.entry)
            .is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
    {
        return Err(GuestError::invalid(
            "Source body scope requires original function entries",
        ));
    }
    let end = image.function_end(player.entry);
    let valid = |index: usize| {
        index > player.entry
            && index < end
            && image
                .instruction(index)
                .is_some_and(|instruction| instruction.opcode == QvmOpcode::OpCall)
            && index
                .checked_sub(1)
                .and_then(|at| image.instruction(at))
                .is_some_and(|target| target.opcode == QvmOpcode::OpConst && target.operand == mesh.entry as i32)
    };
    let mut calls = HashMap::new();
    match mesh.parts.as_ref() {
        None => {
            let mut index = player.entry + 1;
            while index < end {
                if valid(index) {
                    calls.insert(index, QvmBodyPart::Body);
                }
                index += 1;
            }
        }
        Some(parts) => {
            for row in parts {
                if !valid(row.call) || calls.contains_key(&row.call) {
                    return Err(GuestError::invalid(
                        "Source body part does not name a distinct original player-to-mesh call",
                    ));
                }
                calls.insert(row.call, row.part);
            }
        }
    }
    if calls.is_empty() {
        return Err(GuestError::invalid(
            "Source body scope has no qualified original mesh calls",
        ));
    }
    Ok(calls)
}

/// Validate a presentation declaration against its artifacts.
pub fn validate_qvm_mod_presentation(
    artifact: &QvmArtifact,
    source: &ModuleId,
    declaration: &QvmModPresentationDeclaration,
) -> Result<(), GuestError> {
    let cgame = declaration.cgame();
    let (gameplay_path, gameplay_digest, gameplay_abi) = match declaration {
        QvmModPresentationDeclaration::PlayerEvents(declaration) => (
            declaration.gameplay.path.as_str(),
            declaration.gameplay.digest.as_str(),
            declaration.gameplay.abi,
        ),
        QvmModPresentationDeclaration::Scene(declaration) => (
            declaration.gameplay.path.as_str(),
            declaration.gameplay.digest.as_str(),
            declaration.gameplay.abi,
        ),
    };
    if artifact.role != super::mod_provider::QvmRole::Cgame
        || artifact.module.artifact_path != cgame.path
        || artifact.module.digest != cgame.digest
        || artifact.abi() != cgame.abi
        || source.artifact_path != gameplay_path
        || source.digest != gameplay_digest
    {
        return Err(GuestError::invalid(
            "Source presentation differs from its declared gameplay/cgame artifacts",
        ));
    }
    if gameplay_abi != cgame.abi {
        return Err(GuestError::invalid(
            "Source presentation requires matching player-state ABI profiles",
        ));
    }
    let end = artifact.image.data_end();
    let range = |address: usize, size: usize| -> Result<(), GuestError> {
        if size < 1 || address.saturating_add(size) > end {
            return Err(GuestError::invalid(
                "Source presentation storage exceeds the original data image",
            ));
        }
        Ok(())
    };
    range(declaration.game_state_address(), QVM_GAME_STATE_BYTES)?;
    for address in declaration.time_words().iter().chain(declaration.frame_time_words()) {
        range(*address, 4)?;
    }
    for address in declaration
        .view_origin_words()
        .iter()
        .chain(declaration.view_angles_words())
    {
        range(*address, 12)?;
    }
    for address in declaration.view_axis_words() {
        range(*address, 36)?;
    }
    let (address, stride, capacity) = declaration.centities();
    range(address, stride.saturating_mul(capacity))?;
    match declaration {
        QvmModPresentationDeclaration::PlayerEvents(declaration) => {
            let entities = &declaration.storage.centities;
            if capacity < 1 || entities.state + qvm_entity_state_bytes(cgame.abi) > stride {
                return Err(GuestError::invalid("Source presentation centity layout is invalid"));
            }
            let ps_bytes = qvm_player_state_bytes(cgame.abi);
            let snapshot_bytes = qvm_snapshot_bytes(cgame.abi);
            range(declaration.storage.player_state, ps_bytes)?;
            range(declaration.storage.snapshot_address, snapshot_bytes)?;
            for pointer in &declaration.storage.snapshot_pointers {
                range(*pointer, 4)?;
            }
            if entities.origin + 12 > stride {
                return Err(GuestError::invalid("Source centity origin exceeds its record"));
            }
            if declaration.storage.player_state < declaration.storage.snapshot_address + snapshot_bytes
                && declaration.storage.snapshot_address < declaration.storage.player_state + ps_bytes
            {
                return Err(GuestError::invalid(
                    "Event projection state must be separate from the viewing player's snapshot",
                ));
            }
        }
        QvmModPresentationDeclaration::Scene(declaration) => {
            let entities = &declaration.storage.centities;
            if capacity < 1 || entities.state + qvm_entity_state_bytes(cgame.abi) > stride {
                return Err(GuestError::invalid("Source presentation centity layout is invalid"));
            }
            range(declaration.storage.server_command_sequence, 4)?;
            for offset in [entities.previous_event, entities.snapshot_time] {
                if offset + 4 > stride {
                    return Err(GuestError::invalid("Source centity event cursor exceeds its record"));
                }
            }
            for entry in [
                declaration.body.player.entry,
                declaration.body.mesh.entry,
                declaration.event_check.entry,
            ] {
                if artifact
                    .image
                    .instruction(entry)
                    .is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
                {
                    return Err(GuestError::invalid("Source mesh scope has no original function entry"));
                }
            }
            for argument in [
                declaration.body.player.centity_argument,
                declaration.body.mesh.entity_argument,
                declaration.body.mesh.state_argument,
                declaration.event_check.centity_argument,
            ] {
                if argument >= QVM_MAX_PRIVATE_ARGUMENT_WORDS {
                    return Err(GuestError::invalid("Source mesh scope has an invalid argument"));
                }
            }
            if declaration.body.mesh.shader_offset != 112 {
                return Err(GuestError::invalid(
                    "Source mesh shader field differs from the declared refEntity ABI",
                ));
            }
            qualify_qvm_body_calls(&artifact.image, &declaration.body)?;
            if declaration.snapshots.is_empty() {
                return Err(GuestError::invalid(
                    "Source scene requires original snapshot processing",
                ));
            }
        }
    }
    let check_call = |call: &QvmPresentationCall, initializing: bool| -> Result<(), GuestError> {
        if artifact
            .image
            .instruction(call.entry)
            .is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
            || call.arguments.len() > QVM_MAX_PRIVATE_ARGUMENT_WORDS
        {
            return Err(GuestError::invalid(
                "Source presentation call has no original function entry",
            ));
        }
        for argument in &call.arguments {
            match argument {
                QvmPresentationArgument::Address(value) => {
                    if *value < 0 {
                        return Err(GuestError::invalid(
                            "Source presentation storage exceeds the original data image",
                        ));
                    }
                    range(*value as usize, 1)?;
                }
                QvmPresentationArgument::Int32(value) => {
                    check_int32(*value)?;
                }
                QvmPresentationArgument::Float32(value) => {
                    if !value.is_finite() || !(*value as f32).is_finite() {
                        return Err(GuestError::invalid("Source presentation float exceeds its ABI"));
                    }
                }
                QvmPresentationArgument::Source(PresentationSource::PlayerState | PresentationSource::Snapshot)
                    if declaration.is_scene() =>
                {
                    return Err(GuestError::invalid(
                        "Scene presentation receives its records through original snapshot traps",
                    ));
                }
                QvmPresentationArgument::Source(
                    PresentationSource::EntityState
                    | PresentationSource::Centity
                    | PresentationSource::Origin
                    | PresentationSource::Event
                    | PresentationSource::Parameter,
                ) if initializing => {
                    return Err(GuestError::invalid(
                        "Source presentation initialization requires an event-independent context",
                    ));
                }
                QvmPresentationArgument::Source(_) => {}
            }
        }
        Ok(())
    };
    let hud_frames: &[QvmPresentationCall] = declaration.hud().map_or(&[], |hud| &hud.frame);
    for call in declaration
        .initialize_calls()
        .iter()
        .chain(declaration.refresh_calls())
        .chain(declaration.frame_calls())
        .chain(hud_frames)
    {
        check_call(call, true)?;
    }
    match declaration {
        QvmModPresentationDeclaration::PlayerEvents(declaration) => {
            for call in declaration.project.iter().chain(std::iter::once(&declaration.event)) {
                check_call(call, false)?;
            }
        }
        QvmModPresentationDeclaration::Scene(declaration) => {
            for call in &declaration.snapshots {
                check_call(call, true)?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Presentation runtime.
// ---------------------------------------------------------------------------

/// Host services of the presentation module.
pub trait PresentationHost {
    /// Current presentation context.
    fn presentation_context(&self) -> Result<QvmPresentationContext, GuestError>;
    /// Actor owning a source slot, if any.
    fn actor_for_slot(&self, slot: usize) -> Option<ActorId>;
    /// Whether an actor is live.
    fn live(&self, actor: &ActorId) -> bool;
    /// Assert the caller runs on the owner thread.
    fn assert_current(&self) -> Result<(), GuestError>;
    /// Read one source word.
    fn read_i32(&self, address: usize) -> Result<i32, GuestError>;
    /// Write one source word.
    fn write_i32(&mut self, address: usize, value: i32) -> Result<(), GuestError>;
    /// Write one source float.
    fn write_f32(&mut self, address: usize, value: f32) -> Result<(), GuestError>;
    /// Read source bytes.
    fn read_bytes(&self, address: usize, len: usize) -> Result<Vec<u8>, GuestError>;
    /// Write source bytes.
    fn write_bytes(&mut self, address: usize, bytes: &[u8]) -> Result<(), GuestError>;
    /// Fill source bytes.
    fn fill_bytes(&mut self, address: usize, len: usize, value: u8) -> Result<(), GuestError>;
    /// Call a source function.
    fn call_module(&mut self, words: &[i32], entry: usize) -> Result<i32, GuestError>;
    /// Run a source console command.
    fn module_command(&mut self, words: &[i32], argv: &[String]) -> Result<i32, GuestError>;
    /// Read a ref-entity record.
    fn read_ref_entity(&self, pointer: usize) -> Result<Option<RefEntityView>, GuestError>;
    /// Retire the module.
    fn close_module(&mut self);
}

/// Presentation lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresentationPhase {
    Created,
    Initialized,
    Failed,
    Closed,
}

#[derive(Debug)]
enum FlowError {
    Retired,
    Failed(GuestError),
}

impl From<GuestError> for FlowError {
    fn from(error: GuestError) -> Self {
        Self::Failed(error)
    }
}

struct PlayerScope {
    actor: ActorId,
    state: usize,
    parts: Vec<QvmBodyMesh>,
    pending: HashMap<usize, usize>,
}

struct MeshScope {
    player: usize,
    pointer: usize,
    shader: i32,
    output: usize,
}

/// Executes artifact-qualified original presentation.
pub struct QvmModPresentation<H: PresentationHost> {
    host: H,
    // Retained qualification input; never read after construction.
    #[allow(dead_code)]
    artifact: QvmArtifact,
    source: ModuleId,
    declaration: QvmModPresentationDeclaration,
    body_calls: HashMap<usize, QvmBodyPart>,
    phase: PresentationPhase,
    busy: bool,
    sequence: i64,
    revision: i64,
    frame: i64,
    hud_frame: i64,
    command_arguments: Option<Vec<String>>,
    players: HashMap<usize, ActorId>,
    defaults: Option<Vec<u8>>,
    active_event: Option<SourcePlayerEvent>,
    snapshots: HashMap<i32, QvmSourceSnapshot>,
    snapshot_number: i32,
    scene_revision: i64,
    commands: HashMap<u64, Vec<String>>,
    arguments: Vec<String>,
    scene_actors: HashMap<usize, QvmSceneActor>,
    restoring_scene: bool,
    current_game_state: Option<SourceGameState>,
    body_frame: Vec<QvmBodyPresentation>,
    player_scopes: Vec<PlayerScope>,
    mesh_scopes: Vec<MeshScope>,
}

impl<H: PresentationHost> QvmModPresentation<H> {
    /// Create a presentation over a validated declaration.
    pub fn new(
        artifact: QvmArtifact,
        source: ModuleId,
        declaration: QvmModPresentationDeclaration,
        host: H,
    ) -> Result<Self, GuestError> {
        validate_qvm_mod_presentation(&artifact, &source, &declaration)?;
        let body_calls = match &declaration {
            QvmModPresentationDeclaration::Scene(declaration) => {
                qualify_qvm_body_calls(&artifact.image, &declaration.body)?
            }
            QvmModPresentationDeclaration::PlayerEvents(_) => HashMap::new(),
        };
        Ok(Self {
            host,
            artifact,
            source,
            declaration,
            body_calls,
            phase: PresentationPhase::Created,
            busy: false,
            sequence: -1,
            revision: -1,
            frame: -1,
            hud_frame: -1,
            command_arguments: None,
            players: HashMap::new(),
            defaults: None,
            active_event: None,
            snapshots: HashMap::new(),
            snapshot_number: 0,
            scene_revision: -1,
            commands: HashMap::new(),
            arguments: Vec::new(),
            scene_actors: HashMap::new(),
            restoring_scene: false,
            current_game_state: None,
            body_frame: Vec::new(),
            player_scopes: Vec::new(),
            mesh_scopes: Vec::new(),
        })
    }

    /// Borrow the host.
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Mutably borrow the host.
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    /// Last consumed event sequence.
    #[must_use]
    pub fn last_sequence(&self) -> i64 {
        self.sequence
    }

    /// Bodies captured this frame.
    pub fn bodies(&self) -> Result<&[QvmBodyPresentation], GuestError> {
        self.current(None).map_err(|error| match error {
            FlowError::Retired => GuestError::invalid("Source presentation is closed"),
            FlowError::Failed(error) => error,
        })?;
        Ok(&self.body_frame)
    }

    /// Current command arguments.
    #[must_use]
    pub fn arguments(&self) -> &[String] {
        self.command_arguments.as_ref().unwrap_or(&self.arguments)
    }

    fn current(&self, event: Option<&SourcePlayerEvent>) -> Result<(), FlowError> {
        self.host.assert_current()?;
        match self.phase {
            PresentationPhase::Closed => return Err(GuestError::invalid("Source presentation is closed").into()),
            PresentationPhase::Failed => return Err(GuestError::invalid("Source presentation is failed").into()),
            _ => {}
        }
        if let Some(event) = event {
            if !self.host.live(&event.actor)
                || self
                    .host
                    .actor_for_slot(event.player_state.client_number().max(0) as usize)
                    .as_ref()
                    != Some(&event.actor)
            {
                return Err(FlowError::Retired);
            }
        }
        Ok(())
    }

    fn fail(&mut self, error: GuestError) -> GuestError {
        if self.phase != PresentationPhase::Closed {
            self.phase = PresentationPhase::Failed;
        }
        error
    }

    /// Accept a scene publication, reporting whether it advanced.
    pub fn accept_scene(&mut self, scene: &QvmSceneContext, baseline: bool) -> Result<bool, GuestError> {
        if (scene.revision as i64) < self.scene_revision {
            return Err(GuestError::invalid("Component scene publication moved backward"));
        }
        if scene.revision as i64 == self.scene_revision {
            return Ok(false);
        }
        let QvmModPresentationDeclaration::Scene(declaration) = &self.declaration else {
            return Err(GuestError::invalid("Component scene has no initialized caller storage"));
        };
        let defaults = self
            .defaults
            .clone()
            .ok_or_else(|| GuestError::invalid("Component scene has no initialized caller storage"))?;
        let entities = &declaration.storage.centities;
        self.scene_actors.clear();
        for row in &scene.actors {
            if row.slot >= entities.capacity {
                return Err(GuestError::invalid("Component source actor exceeds cgame centities"));
            }
            if let Some(previous) = self.players.get(&row.slot) {
                if *previous != row.actor {
                    let start = row.slot * entities.stride;
                    self.host
                        .write_bytes(entities.address + start, &defaults[start..start + entities.stride])?;
                }
            }
            self.players.insert(row.slot, row.actor.clone());
            self.scene_actors.insert(row.slot, row.clone());
        }
        if baseline {
            for state in &scene.snapshot.entities {
                let view = entities.address + (state.number().max(0) as usize) * entities.stride;
                let previous = if state.etype() > declaration.event_entity_type {
                    1
                } else {
                    state.event()
                };
                self.host.write_i32(view + entities.previous_event, previous)?;
                self.host
                    .write_i32(view + entities.snapshot_time, scene.snapshot.server_time)?;
            }
        }
        self.scene_revision = scene.revision as i64;
        for command in &scene.commands {
            self.commands.insert(command.sequence, command.arguments.clone());
        }
        let cutoff = scene.snapshot.server_command_sequence as i64 - 64;
        self.commands.retain(|number, _| *number as i64 > cutoff);
        self.snapshot_number += 1;
        let number = self.snapshot_number;
        let mut snapshot = scene.snapshot.clone();
        snapshot.number = number;
        self.snapshots.insert(number, snapshot);
        self.snapshots.remove(&(number - 32));
        Ok(true)
    }

    fn write_game_state(&mut self, base: usize, state: &SourceGameState) -> Result<(), GuestError> {
        if state.offsets.len() != 1024 || state.data.len() != 16000 {
            return Err(GuestError::invalid("Invalid source gameState_t extent"));
        }
        for (index, offset) in state.offsets.iter().enumerate() {
            self.host.write_i32(base + index * 4, *offset)?;
        }
        self.host.write_bytes(base + 4096, &state.data)?;
        self.host.write_i32(base + 20096, state.count as i32)?;
        Ok(())
    }

    fn view(&mut self, context: &QvmPresentationContext) -> Result<(), GuestError> {
        for address in self.declaration.view_origin_words().to_vec() {
            self.write_vector(address, context.view_origin)?;
        }
        if self.declaration.view_angles_words().is_empty() && self.declaration.view_axis_words().is_empty() {
            return Ok(());
        }
        let Some(axis) = context.view_axis else {
            return Err(GuestError::invalid(
                "Declared original view storage requires the actual viewing camera axis",
            ));
        };
        for address in self.declaration.view_axis_words().to_vec() {
            for (index, value) in axis.iter().enumerate() {
                self.write_vector(address + index * 12, *value)?;
            }
        }
        let angles = vector_to_angles(axis[0]);
        let basis = qvm_angles_to_axis(angles);
        let roll = f32::atan2(dot3(axis[1], basis[2]), dot3(axis[1], basis[1])) * 180.0 / std::f32::consts::PI;
        for address in self.declaration.view_angles_words().to_vec() {
            self.write_vector(address, vec3(angles.x, angles.y, roll))?;
        }
        Ok(())
    }

    fn write_vector(&mut self, address: usize, value: Vec3) -> Result<(), GuestError> {
        self.host.write_f32(address, value.x)?;
        self.host.write_f32(address + 4, value.y)?;
        self.host.write_f32(address + 8, value.z)?;
        Ok(())
    }

    fn address_word(
        &mut self,
        argument: &QvmPresentationArgument,
        event: Option<&SourcePlayerEvent>,
        context: &QvmPresentationContext,
    ) -> Result<i32, FlowError> {
        match argument {
            QvmPresentationArgument::Int32(value) => Ok(*value),
            QvmPresentationArgument::Float32(value) => Ok((*value as f32).to_bits() as i32),
            QvmPresentationArgument::Address(value) => Ok(*value),
            QvmPresentationArgument::Source(source) => {
                let scene = self.declaration.is_scene();
                match source {
                    PresentationSource::PlayerState => {
                        let QvmModPresentationDeclaration::PlayerEvents(declaration) = &self.declaration else {
                            return Err(GuestError::invalid(
                                "Scene presentation receives its records through original snapshot traps",
                            )
                            .into());
                        };
                        Ok(declaration.storage.player_state as i32)
                    }
                    PresentationSource::EntityState => {
                        let event = event.ok_or_else(|| {
                            GuestError::invalid("Presentation event storage requires the projected event")
                        })?;
                        let (address, stride, _) = self.declaration.centities();
                        let state = match &self.declaration {
                            QvmModPresentationDeclaration::PlayerEvents(declaration) => {
                                declaration.storage.centities.state
                            }
                            QvmModPresentationDeclaration::Scene(declaration) => declaration.storage.centities.state,
                        };
                        Ok((address + (event.player_state.client_number().max(0) as usize) * stride + state) as i32)
                    }
                    PresentationSource::Centity => {
                        let event = event.ok_or_else(|| {
                            GuestError::invalid("Presentation event storage requires the projected event")
                        })?;
                        let (address, stride, _) = self.declaration.centities();
                        Ok((address + (event.player_state.client_number().max(0) as usize) * stride) as i32)
                    }
                    PresentationSource::Origin => {
                        let event = event.ok_or_else(|| {
                            GuestError::invalid("Presentation event storage requires the projected event")
                        })?;
                        let QvmModPresentationDeclaration::PlayerEvents(declaration) = &self.declaration else {
                            return Err(GuestError::invalid(
                                "Scene presentation receives its records through original snapshot traps",
                            )
                            .into());
                        };
                        let entities = &declaration.storage.centities;
                        Ok((entities.address
                            + (event.player_state.client_number().max(0) as usize) * entities.stride
                            + entities.origin) as i32)
                    }
                    PresentationSource::Snapshot => {
                        let QvmModPresentationDeclaration::PlayerEvents(declaration) = &self.declaration else {
                            return Err(GuestError::invalid(
                                "Scene presentation receives its records through original snapshot traps",
                            )
                            .into());
                        };
                        Ok(declaration.storage.snapshot_address as i32)
                    }
                    PresentationSource::ClientNumber => Ok(context
                        .client_number
                        .unwrap_or_else(|| context.snapshot.player_state.client_number())),
                    PresentationSource::Time => Ok(context.time_ms.unwrap_or(context.snapshot.server_time)),
                    PresentationSource::Event => Ok(event.map_or(0, |event| event.event)),
                    PresentationSource::Parameter => Ok(event.map_or(0, |event| event.parameter)),
                    PresentationSource::SnapshotNumber => Ok(if scene { 0.max(self.snapshot_number - 1) } else { 0 }),
                    PresentationSource::ServerCommandSequence => {
                        if scene {
                            let context = self.host.presentation_context()?;
                            Ok(context.scene.map_or(0, |scene| scene.snapshot.server_command_sequence))
                        } else {
                            Ok(0)
                        }
                    }
                }
            }
        }
    }

    fn call(&mut self, call: &QvmPresentationCall, event: Option<&SourcePlayerEvent>) -> Result<(), FlowError> {
        let context = self.host.presentation_context()?;
        if call.when_weapon_presented {
            match context.weapon_presented {
                None => {
                    return Err(
                        GuestError::invalid("Component presentation requires its declared weapon presentation").into(),
                    )
                }
                Some(false) => return Ok(()),
                Some(true) => {}
            }
        }
        let selected = event.cloned().or_else(|| self.active_event.clone());
        let words = call
            .arguments
            .iter()
            .map(|argument| self.address_word(argument, selected.as_ref(), &context))
            .collect::<Result<Vec<_>, _>>()?;
        self.view(&context)?;
        self.host.call_module(&words, call.entry)?;
        self.current(event.or(self.active_event.as_ref()))?;
        Ok(())
    }

    fn refresh(&mut self) -> Result<(), FlowError> {
        let context = self.host.presentation_context()?;
        if context.game_state_revision as i64 != self.revision {
            for call in self.declaration.refresh_calls().to_vec() {
                self.call(&call, None)?;
            }
            self.revision = context.game_state_revision as i64;
        }
        Ok(())
    }

    fn apply_context(&mut self, _event: Option<&SourcePlayerEvent>) -> Result<QvmPresentationContext, FlowError> {
        let context = self.host.presentation_context()?;
        if context.frame_time_ms < 0
            || ![context.view_origin.x, context.view_origin.y, context.view_origin.z]
                .into_iter()
                .all(f32::is_finite)
        {
            return Err(GuestError::invalid("Invalid source presentation context").into());
        }
        if context.game_state_revision as i64 != self.revision {
            self.write_game_state(self.declaration.game_state_address(), &context.game_state)?;
            self.current_game_state = Some(context.game_state.clone());
        }
        if !self.declaration.is_scene() {
            let QvmModPresentationDeclaration::PlayerEvents(declaration) = &self.declaration else {
                return Err(GuestError::invalid("Invalid source presentation context").into());
            };
            let storage = &declaration.storage;
            let abi = declaration.cgame.abi;
            let mut snapshot = super::mod_presentation_checkpoint::QvmSourceSnapshot {
                number: 0,
                server_time: context.snapshot.server_time,
                flags: 0,
                area_mask: vec![0u8; 32],
                player_state: context.snapshot.player_state.clone(),
                entities: Vec::new(),
                server_command_sequence: 0,
            };
            let _ = &mut snapshot;
            let bytes = snapshot_to_bytes(&snapshot, abi)?;
            self.host.write_bytes(storage.snapshot_address, &bytes)?;
            for pointer in &storage.snapshot_pointers {
                self.host.write_i32(*pointer, storage.snapshot_address as i32)?;
            }
            let time = context.time_ms.unwrap_or(context.snapshot.server_time);
            for address in self.declaration.time_words().to_vec() {
                self.host.write_i32(address, time)?;
            }
            for address in self.declaration.frame_time_words().to_vec() {
                self.host.write_i32(address, context.frame_time_ms)?;
            }
            self.view(&context)?;
        }
        Ok(context)
    }

    /// Run initialization calls and remember centity defaults.
    pub fn initialize(&mut self, baseline_sequence: i64) -> Result<(), GuestError> {
        if let Err(error) = self.current(None) {
            return Err(match error {
                FlowError::Retired => GuestError::invalid("Source presentation is closed"),
                FlowError::Failed(error) => error,
            });
        }
        if self.phase == PresentationPhase::Initialized || self.busy {
            return Err(self.fail(GuestError::invalid("Source presentation is already initialized")));
        }
        if baseline_sequence < -1 {
            return Err(self.fail(GuestError::invalid("Invalid source presentation checkpoint")));
        }
        self.busy = true;
        let outcome = self.initialize_inner(baseline_sequence);
        self.busy = false;
        outcome.map_err(|error| self.fail(error))
    }

    fn initialize_inner(&mut self, baseline_sequence: i64) -> Result<(), GuestError> {
        self.current(None).map_err(|error| match error {
            FlowError::Retired => GuestError::invalid("Source presentation is closed"),
            FlowError::Failed(error) => error,
        })?;
        let context = self.apply_context(None).map_err(|error| match error {
            FlowError::Retired => GuestError::invalid("Source presentation is closed"),
            FlowError::Failed(error) => error,
        })?;
        for call in self.declaration.initialize_calls().to_vec() {
            self.call(&call, None).map_err(|error| match error {
                FlowError::Retired => GuestError::invalid("Source presentation is closed"),
                FlowError::Failed(error) => error,
            })?;
        }
        self.sequence = baseline_sequence;
        self.revision = context.game_state_revision as i64;
        let (address, stride, capacity) = self.declaration.centities();
        self.defaults = Some(self.host.read_bytes(address, stride * capacity)?);
        self.phase = PresentationPhase::Initialized;
        if self.declaration.is_scene() {
            let Some(scene) = context.scene.clone() else {
                return Err(GuestError::invalid("Scene presentation requires its published scene"));
            };
            let QvmModPresentationDeclaration::Scene(declaration) = self.declaration.clone() else {
                return Err(GuestError::invalid("Scene presentation requires its published scene"));
            };
            self.host.write_i32(
                declaration.storage.server_command_sequence,
                scene.snapshot.server_command_sequence,
            )?;
            if let Some(baseline) = scene.baseline.as_ref() {
                self.restoring_scene = true;
                self.accept_scene(baseline, true)?;
                self.restoring_scene = false;
            }
            if self.accept_scene(&scene, false)? {
                let snapshots = declaration.snapshots.clone();
                for call in snapshots {
                    self.call(&call, None).map_err(|error| match error {
                        FlowError::Retired => GuestError::invalid("Source presentation is closed"),
                        FlowError::Failed(error) => error,
                    })?;
                }
            }
        }
        Ok(())
    }

    /// Consume one gameplay event.
    pub fn consume(&mut self, event: &SourcePlayerEvent, sequence: i64) -> Result<(), GuestError> {
        let outcome = self.consume_inner(event, sequence);
        match outcome {
            Ok(()) => Ok(()),
            Err(FlowError::Retired) => Ok(()),
            Err(FlowError::Failed(error)) => Err(self.fail(error)),
        }
    }

    fn consume_inner(&mut self, event: &SourcePlayerEvent, sequence: i64) -> Result<(), FlowError> {
        self.current(Some(event))?;
        if self.phase != PresentationPhase::Initialized || self.busy {
            return Err(GuestError::invalid("Source presentation cannot consume its retired caller").into());
        }
        if sequence < 0 || sequence <= self.sequence {
            return Err(GuestError::invalid("Source presentation event sequence moved backward").into());
        }
        if event.module.artifact_path != self.source.artifact_path || event.module.digest != self.source.digest {
            return Err(GuestError::invalid("Source presentation event differs from its gameplay module").into());
        }
        if self.declaration.is_scene() {
            return Err(
                GuestError::invalid("Scene presentation consumes authoritative snapshots through its context").into(),
            );
        }
        let QvmModPresentationDeclaration::PlayerEvents(declaration) = self.declaration.clone() else {
            return Err(
                GuestError::invalid("Scene presentation consumes authoritative snapshots through its context").into(),
            );
        };
        let slot = event.player_state.client_number();
        if slot < 0 || slot as usize >= declaration.storage.centities.capacity {
            return Err(GuestError::invalid("Source presentation event exceeds cgame centities").into());
        }
        for component in [event.origin.x, event.origin.y, event.origin.z] {
            if !component.is_finite() || !(component as f64).is_finite() {
                return Err(GuestError::invalid("Source presentation event has no finite origin").into());
            }
        }
        self.busy = true;
        self.active_event = Some(event.clone());
        let outcome = self.consume_event(event, sequence, &declaration);
        self.active_event = None;
        self.busy = false;
        outcome
    }

    fn consume_event(
        &mut self,
        event: &SourcePlayerEvent,
        sequence: i64,
        declaration: &QvmPlayerEventPresentation,
    ) -> Result<(), FlowError> {
        let slot = event.player_state.client_number().max(0) as usize;
        let entities = &declaration.storage.centities;
        let view = entities.address + slot * entities.stride;
        if let Some(previous) = self.players.get(&slot) {
            if *previous != event.actor {
                let defaults = self
                    .defaults
                    .clone()
                    .ok_or_else(|| GuestError::invalid("Source presentation has no initialized caller storage"))?;
                let start = slot * entities.stride;
                self.host.write_bytes(view, &defaults[start..start + entities.stride])?;
            }
        }
        self.players.insert(slot, event.actor.clone());
        self.apply_context(Some(event))?;
        self.refresh()?;
        self.host
            .write_bytes(declaration.storage.player_state, event.player_state.bytes())?;
        for call in &declaration.project {
            self.call(call, Some(event))?;
        }
        let mut state = super::mod_presentation_checkpoint::SourceEntityState::from_bytes(
            &self
                .host
                .read_bytes(view + entities.state, qvm_entity_state_bytes(declaration.cgame.abi))?,
            declaration.cgame.abi,
        )?;
        state.set_event(event.event, event.parameter);
        self.host.write_bytes(view + entities.state, state.bytes())?;
        self.write_vector(view + entities.origin, event.origin)?;
        let call = declaration.event.clone();
        self.call(&call, Some(event))?;
        self.sequence = sequence;
        Ok(())
    }

    /// Advance one frame.
    pub fn advance(&mut self, frame: i64) -> Result<(), GuestError> {
        if let Err(error) = self.current(None) {
            return Err(match error {
                FlowError::Retired => GuestError::invalid("Source presentation is closed"),
                FlowError::Failed(error) => error,
            });
        }
        if self.phase != PresentationPhase::Initialized || self.busy || frame < 0 || frame <= self.frame {
            return Err(self.fail(GuestError::invalid("Source presentation frame moved backward")));
        }
        self.busy = true;
        let outcome = (|| -> Result<(), FlowError> {
            self.frame = frame;
            self.body_frame.clear();
            let context = self.apply_context(None)?;
            self.refresh()?;
            if self.declaration.is_scene() {
                let Some(scene) = context.scene.clone() else {
                    return Err(GuestError::invalid("Scene presentation requires its published scene").into());
                };
                if self.accept_scene(&scene, false)? {
                    let QvmModPresentationDeclaration::Scene(declaration) = self.declaration.clone() else {
                        return Err(GuestError::invalid("Scene presentation requires its published scene").into());
                    };
                    let snapshots = declaration.snapshots.clone();
                    for call in snapshots {
                        self.call(&call, None)?;
                    }
                } else {
                    let QvmModPresentationDeclaration::Scene(declaration) = self.declaration.clone() else {
                        return Err(GuestError::invalid("Scene presentation requires its published scene").into());
                    };
                    self.host.write_i32(
                        declaration.storage.server_command_sequence,
                        scene.snapshot.server_command_sequence,
                    )?;
                    for row in &scene.actors {
                        if let Some(actor) = self.scene_actors.get(&row.slot) {
                            if actor.actor == row.actor {
                                continue;
                            }
                        }
                        return Err(
                            GuestError::invalid("Scene presentation callers changed without a scene revision").into(),
                        );
                    }
                }
            }
            for call in self.declaration.frame_calls().to_vec() {
                self.call(&call, None)?;
            }
            Ok(())
        })();
        self.busy = false;
        outcome.map_err(|error| match error {
            FlowError::Retired => self.fail(GuestError::invalid("Source presentation is closed")),
            FlowError::Failed(error) => self.fail(error),
        })
    }

    /// Run a console command through the source.
    pub fn console_command(&mut self, argv: &[String]) -> Result<bool, GuestError> {
        if let Err(error) = self.current(None) {
            return Err(match error {
                FlowError::Retired => GuestError::invalid("Source presentation is closed"),
                FlowError::Failed(error) => error,
            });
        }
        if self.phase != PresentationPhase::Initialized || self.busy {
            return Err(self.fail(GuestError::invalid("Source presentation is not initialized")));
        }
        self.busy = true;
        self.command_arguments = Some(argv.to_vec());
        let outcome = self.host.module_command(&[2], argv);
        self.command_arguments = None;
        self.busy = false;
        match outcome {
            Ok(result) => {
                if let Err(error) = self.current(None) {
                    return Err(match error {
                        FlowError::Retired => GuestError::invalid("Source presentation is closed"),
                        FlowError::Failed(error) => self.fail(error),
                    });
                }
                Ok(result != 0)
            }
            Err(error) => Err(self.fail(error)),
        }
    }

    /// Run HUD frame calls for the current frame.
    pub fn draw_hud(&mut self, frame: i64) -> Result<(), GuestError> {
        if let Err(error) = self.current(None) {
            return Err(match error {
                FlowError::Retired => GuestError::invalid("Source presentation is closed"),
                FlowError::Failed(error) => error,
            });
        }
        if self.phase != PresentationPhase::Initialized || self.busy || frame != self.frame {
            return Err(self.fail(GuestError::invalid("Source presentation HUD frame is not current")));
        }
        let Some(hud) = self.declaration.hud().cloned() else {
            return Ok(());
        };
        self.busy = true;
        let outcome = (|| -> Result<(), FlowError> {
            for call in &hud.frame {
                self.call(call, None)?;
            }
            Ok(())
        })();
        self.busy = false;
        outcome.map_err(|error| match error {
            FlowError::Retired => self.fail(GuestError::invalid("Source presentation is closed")),
            FlowError::Failed(error) => self.fail(error),
        })?;
        self.hud_frame = frame;
        Ok(())
    }

    /// Release one player's centity.
    pub fn release(&mut self, actor: &ActorId) {
        self.players.retain(|_, bound| bound != actor);
    }

    /// Close the presentation.
    pub fn close(&mut self) {
        self.host.close_module();
        self.phase = PresentationPhase::Closed;
        self.busy = false;
        self.players.clear();
        self.defaults = None;
        self.active_event = None;
        self.snapshots.clear();
        self.commands.clear();
        self.arguments.clear();
        self.scene_actors.clear();
        self.current_game_state = None;
        self.body_frame.clear();
        self.player_scopes.clear();
        self.mesh_scopes.clear();
    }

    /// Snapshot bytes for the original snapshot trap, if any.
    pub fn snapshot_for_trap(&self, number: i32) -> Result<Option<Vec<u8>>, GuestError> {
        let Some(snapshot) = self.snapshots.get(&number) else {
            return Ok(None);
        };
        let abi = self.declaration.cgame().abi;
        Ok(Some(snapshot_to_bytes(snapshot, abi)?))
    }

    /// Current snapshot trap state.
    #[must_use]
    pub fn snapshot_trap_state(&self) -> (i32, i32) {
        let time = self
            .snapshots
            .get(&self.snapshot_number)
            .map_or(0, |snapshot| snapshot.server_time);
        (self.snapshot_number, time)
    }

    /// Load server-command arguments for a sequence, reporting nonempty.
    pub fn server_command_args(&mut self, sequence: u64) -> Result<bool, GuestError> {
        let Some(command) = self.commands.get(&sequence).cloned() else {
            return Err(GuestError::invalid(
                "Original snapshot requested unknown server command arguments",
            ));
        };
        self.arguments = command;
        Ok(!self.arguments.is_empty())
    }

    /// Current game state for the trap.
    pub fn take_game_state(&self) -> Result<SourceGameState, GuestError> {
        self.current_game_state
            .clone()
            .ok_or_else(|| GuestError::invalid("Source presentation context has no game state"))
    }

    /// Enter a player-mesh hook scope; returns its token.
    pub fn enter_player_mesh(
        &mut self,
        centity: usize,
        state: usize,
        sites: &[(usize, QvmBodyPart)],
    ) -> Result<Option<usize>, GuestError> {
        let QvmModPresentationDeclaration::Scene(declaration) = &self.declaration else {
            return Ok(None);
        };
        let entities = &declaration.storage.centities;
        if centity < entities.address || !(centity - entities.address).is_multiple_of(entities.stride) {
            return Ok(None);
        }
        let slot = (centity - entities.address) / entities.stride;
        let Some(actor) = self.scene_actors.get(&slot) else {
            return Ok(None);
        };
        if state != centity + entities.state {
            return Ok(None);
        }
        let number = self.host.read_i32(state)?;
        if number < 0 || number as usize != slot {
            return Ok(None);
        }
        let mut pending = HashMap::new();
        for (index, (site, part)) in sites.iter().enumerate() {
            if self.body_calls.get(site) == Some(part) {
                pending.insert(*site, index);
            }
        }
        let token = self.player_scopes.len();
        self.player_scopes.push(PlayerScope {
            actor: actor.actor.clone(),
            state,
            parts: vec![QvmBodyMesh {
                part: QvmBodyPart::Body,
                base: false,
                passes: Vec::new(),
            }],
            pending,
        });
        Ok(Some(token))
    }

    /// Exit a player-mesh hook scope.
    pub fn exit_player_mesh(&mut self, token: usize) {
        if token >= self.player_scopes.len() {
            return;
        }
        let scope = self.player_scopes.remove(token);
        for mesh in self.mesh_scopes.iter().rev() {
            if mesh.player == token {
                return;
            }
        }
        if self.host.live(&scope.actor) && scope.parts.iter().any(|part| part.base) {
            match self.body_frame.iter_mut().find(|body| body.actor == scope.actor) {
                Some(body) => body.parts = scope.parts,
                None => self.body_frame.push(QvmBodyPresentation {
                    actor: scope.actor,
                    parts: scope.parts,
                }),
            }
        }
        for mesh in &mut self.mesh_scopes {
            if mesh.player > token {
                mesh.player -= 1;
            }
        }
    }

    /// Enter a mesh-call hook scope; returns its token.
    pub fn enter_mesh_call(
        &mut self,
        player: usize,
        entity: usize,
        state: usize,
        caller: Option<usize>,
    ) -> Option<usize> {
        let scope = self.player_scopes.get(player)?;
        if state != scope.state || entity < scope.state {
            return None;
        }
        let part = caller
            .and_then(|site| self.body_calls.get(&site).copied())
            .unwrap_or(QvmBodyPart::Body);
        let output = match scope.pending.get(&caller.unwrap_or(usize::MAX)) {
            Some(index) => *index,
            None if caller.is_none() => 0,
            None => return None,
        };
        let _ = part;
        let token = self.mesh_scopes.len();
        self.mesh_scopes.push(MeshScope {
            player,
            pointer: entity,
            shader: 0,
            output,
        });
        Some(token)
    }

    /// Note an add-ref-entity trap, capturing model passes; reports capture.
    pub fn note_add_ref_entity(&mut self, pointer: usize) -> Result<bool, GuestError> {
        let Some(token) = self.mesh_scopes.iter().rposition(|scope| scope.pointer == pointer) else {
            return Ok(false);
        };
        let view = self.host.read_ref_entity(pointer)?;
        let Some(view) = view else { return Ok(false) };
        let output = self.mesh_scopes[token].output;
        let player = self.mesh_scopes[token].player;
        if !view.is_model || view.custom_shader == 0 {
            return Ok(false);
        }
        self.mesh_scopes[token].shader = view.custom_shader;
        let Some(scope) = self.player_scopes.get_mut(player) else {
            return Ok(false);
        };
        while scope.parts.len() <= output {
            scope.parts.push(QvmBodyMesh {
                part: QvmBodyPart::Body,
                base: false,
                passes: Vec::new(),
            });
        }
        if !scope.parts[output].base {
            scope.parts[output].base = true;
        } else {
            scope.parts[output].passes.push(BodyMeshPass {
                custom_shader: view.custom_shader,
                bytes: view.bytes,
            });
        }
        Ok(true)
    }

    /// Exit a mesh-call hook scope.
    pub fn exit_mesh_call(&mut self, token: usize) {
        if token < self.mesh_scopes.len() {
            self.mesh_scopes.remove(token);
        }
    }

    /// Note an event-check hook during scene restore.
    pub fn note_event_check(&mut self, centity: usize) -> Result<(), GuestError> {
        if !self.restoring_scene {
            return Ok(());
        }
        let QvmModPresentationDeclaration::Scene(declaration) = &self.declaration else {
            return Ok(());
        };
        let entities = &declaration.storage.centities;
        if centity < entities.address || !(centity - entities.address).is_multiple_of(entities.stride) {
            return Ok(());
        }
        let entity = super::mod_presentation_checkpoint::SourceEntityState::from_bytes(
            &self
                .host
                .read_bytes(centity + entities.state, qvm_entity_state_bytes(declaration.cgame.abi))?,
            declaration.cgame.abi,
        )?;
        let previous = if entity.etype() > declaration.event_entity_type {
            1
        } else {
            entity.event()
        };
        self.host.write_i32(centity + entities.previous_event, previous)?;
        Ok(())
    }

    /// Capture the host half of a checkpoint.
    pub fn capture_host_state(&mut self) -> Result<ProfileValue, GuestError> {
        if let Err(error) = self.current(None) {
            return Err(match error {
                FlowError::Retired => GuestError::invalid("Source presentation is closed"),
                FlowError::Failed(error) => error,
            });
        }
        if self.phase != PresentationPhase::Initialized || self.busy {
            return Err(GuestError::invalid("Source presentation is not checkpointable"));
        }
        let abi = self.declaration.cgame().abi;
        let players = self
            .players
            .iter()
            .map(|(slot, actor)| {
                ProfileValue::record(vec![
                    ("slot", ProfileValue::Int(*slot as i64)),
                    (
                        "actor",
                        super::mod_presentation_checkpoint::capture_saved_actor_id(actor),
                    ),
                ])
            })
            .collect();
        let snapshots = self
            .snapshots
            .values()
            .map(|snapshot| capture_presentation_snapshot(snapshot, abi))
            .collect::<Result<Vec<_>, _>>()?;
        let commands = self
            .commands
            .iter()
            .map(|(sequence, arguments)| {
                ProfileValue::record(vec![
                    ("sequence", ProfileValue::Int(*sequence as i64)),
                    (
                        "arguments",
                        ProfileValue::Array(
                            arguments
                                .iter()
                                .map(|argument| ProfileValue::Str(argument.clone()))
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect();
        let mut actors: Vec<(&usize, &QvmSceneActor)> = self.scene_actors.iter().collect();
        actors.sort_by_key(|(slot, _)| **slot);
        let scene_actors = actors
            .into_iter()
            .map(|(slot, row)| {
                ProfileValue::record(vec![
                    ("slot", ProfileValue::Int(*slot as i64)),
                    ("owned", ProfileValue::Bool(row.owned)),
                    (
                        "actor",
                        super::mod_presentation_checkpoint::capture_saved_actor_id(&row.actor),
                    ),
                ])
            })
            .collect();
        Ok(ProfileValue::record(vec![
            ("version", ProfileValue::Int(1)),
            ("sequence", ProfileValue::Int(self.sequence)),
            ("revision", ProfileValue::Int(self.revision)),
            ("frame", ProfileValue::Int(self.frame)),
            ("hudFrame", ProfileValue::Int(self.hud_frame)),
            ("players", ProfileValue::Array(players)),
            (
                "defaults",
                self.defaults
                    .clone()
                    .map(ProfileValue::Bytes)
                    .unwrap_or(ProfileValue::Null),
            ),
            ("snapshots", ProfileValue::Array(snapshots)),
            ("snapshotNumber", ProfileValue::Int(i64::from(self.snapshot_number))),
            ("sceneRevision", ProfileValue::Int(self.scene_revision)),
            ("commands", ProfileValue::Array(commands)),
            (
                "arguments",
                ProfileValue::Array(
                    self.arguments
                        .iter()
                        .map(|argument| ProfileValue::Str(argument.clone()))
                        .collect(),
                ),
            ),
            ("sceneActors", ProfileValue::Array(scene_actors)),
            (
                "currentGameState",
                self.current_game_state
                    .as_ref()
                    .map(capture_presentation_game_state)
                    .unwrap_or(ProfileValue::Null),
            ),
        ]))
    }

    /// Restore host state captured by [`QvmModPresentation::capture_host_state`].
    pub fn restore_host_state(
        &mut self,
        host: &ProfileValue,
        resolve: &dyn Fn(qa_core::identity::SavedActorId) -> Result<ActorId, GuestError>,
    ) -> Result<(), GuestError> {
        if let Err(error) = self.current(None) {
            return Err(match error {
                FlowError::Retired => GuestError::invalid("Source presentation is closed"),
                FlowError::Failed(error) => error,
            });
        }
        if self.phase != PresentationPhase::Created || self.busy {
            return Err(self.fail(GuestError::invalid(
                "Source presentation restore requires a created module",
            )));
        }
        let reader = ProfileReader::new(host);
        reader.field("version")?.literal_int(1)?;
        let sequence = reader.field("sequence")?.integer(-1)?;
        let abi = self.declaration.cgame().abi;
        let mut snapshots = HashMap::new();
        for snapshot in reader
            .field("snapshots")?
            .list(|row| read_presentation_snapshot(row, abi))?
        {
            if snapshots.insert(snapshot.number, snapshot).is_some() {
                return Err(GuestError::invalid("Duplicate source snapshot"));
            }
        }
        let snapshot_number = reader.field("snapshotNumber")?.integer(0)?;
        if snapshot_number > i64::from(i32::MAX) {
            return Err(GuestError::invalid("Invalid source snapshot number"));
        }
        let snapshot_number = snapshot_number as i32;
        if !snapshots.contains_key(&snapshot_number) {
            return Err(GuestError::invalid(
                "Source presentation restore has no current snapshot",
            ));
        }
        let (_, _, capacity) = self.declaration.centities();
        let mut players = HashMap::new();
        for row in reader.field("players")?.list(|row| {
            let slot = row.field("slot")?.integer(0)? as usize;
            if slot >= capacity {
                return row.fail("Source presentation player exceeds cgame centities");
            }
            let actor = resolve(read_saved_actor_id(&row.field("actor")?)?)?;
            if !self.host.live(&actor) {
                return row.fail("Saved source presentation player is unavailable");
            }
            Ok((slot, actor))
        })? {
            if players.insert(row.0, row.1).is_some() {
                return Err(GuestError::invalid("Duplicate source presentation player"));
            }
        }
        let mut scene_actors = HashMap::new();
        for row in reader.field("sceneActors")?.list(|row| {
            let slot = row.field("slot")?.integer(0)? as usize;
            if slot >= capacity {
                return row.fail("Source presentation scene actor exceeds cgame centities");
            }
            let actor = resolve(read_saved_actor_id(&row.field("actor")?)?)?;
            if !self.host.live(&actor) {
                return row.fail("Saved source presentation scene actor is unavailable");
            }
            Ok((
                slot,
                QvmSceneActor {
                    actor,
                    slot,
                    owned: row.field("owned")?.boolean()?,
                },
            ))
        })? {
            if scene_actors.insert(row.0, row.1).is_some() {
                return Err(GuestError::invalid("Duplicate source presentation scene actor"));
            }
        }
        let mut commands = HashMap::new();
        for (sequence, arguments) in reader.field("commands")?.list(|row| {
            Ok((
                row.field("sequence")?.integer(0)? as u64,
                row.field("arguments")?.list(|value| value.string())?,
            ))
        })? {
            if commands.insert(sequence, arguments).is_some() {
                return Err(GuestError::invalid("Duplicate source presentation server command"));
            }
        }
        self.sequence = sequence;
        self.revision = reader.field("revision")?.integer(-1)?;
        self.frame = reader.field("frame")?.integer(-1)?;
        self.hud_frame = reader.field("hudFrame")?.integer(-1)?;
        self.players = players;
        self.defaults = reader.field("defaults")?.nullable(|value| value.bytes())?;
        self.snapshots = snapshots;
        self.snapshot_number = snapshot_number;
        self.scene_revision = reader.field("sceneRevision")?.integer(-1)?;
        self.commands = commands;
        self.arguments = reader.field("arguments")?.list(|value| value.string())?;
        self.scene_actors = scene_actors;
        self.current_game_state = reader
            .field("currentGameState")?
            .nullable(read_presentation_game_state)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use qa_core::identity::IdentityOwner;

    use super::super::mod_player_events::PlayerEventSequence;
    use super::super::mod_provider::{QvmInstruction, QvmRole};
    use super::*;

    struct FakeHost {
        memory: Vec<u8>,
        context: QvmPresentationContext,
        slots: HashMap<usize, ActorId>,
        live: HashSet<ActorId>,
        calls: Vec<(Vec<i32>, usize)>,
        ref_entities: HashMap<usize, RefEntityView>,
        closed: bool,
    }

    impl FakeHost {
        fn new(context: QvmPresentationContext) -> Self {
            Self {
                memory: vec![0; 200000],
                context,
                slots: HashMap::new(),
                live: HashSet::new(),
                calls: Vec::new(),
                ref_entities: HashMap::new(),
                closed: false,
            }
        }
    }

    impl PresentationHost for FakeHost {
        fn presentation_context(&self) -> Result<QvmPresentationContext, GuestError> {
            Ok(self.context.clone())
        }
        fn actor_for_slot(&self, slot: usize) -> Option<ActorId> {
            self.slots.get(&slot).cloned()
        }
        fn live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }
        fn assert_current(&self) -> Result<(), GuestError> {
            Ok(())
        }
        fn read_i32(&self, address: usize) -> Result<i32, GuestError> {
            Ok(i32::from_le_bytes(
                self.memory[address..address + 4]
                    .try_into()
                    .map_err(|_| GuestError::invalid("oob"))?,
            ))
        }
        fn write_i32(&mut self, address: usize, value: i32) -> Result<(), GuestError> {
            self.memory[address..address + 4].copy_from_slice(&value.to_le_bytes());
            Ok(())
        }
        fn write_f32(&mut self, address: usize, value: f32) -> Result<(), GuestError> {
            self.memory[address..address + 4].copy_from_slice(&value.to_le_bytes());
            Ok(())
        }
        fn read_bytes(&self, address: usize, len: usize) -> Result<Vec<u8>, GuestError> {
            Ok(self.memory[address..address + len].to_vec())
        }
        fn write_bytes(&mut self, address: usize, bytes: &[u8]) -> Result<(), GuestError> {
            self.memory[address..address + bytes.len()].copy_from_slice(bytes);
            Ok(())
        }
        fn fill_bytes(&mut self, address: usize, len: usize, value: u8) -> Result<(), GuestError> {
            self.memory[address..address + len].fill(value);
            Ok(())
        }
        fn call_module(&mut self, words: &[i32], entry: usize) -> Result<i32, GuestError> {
            self.calls.push((words.to_vec(), entry));
            Ok(0)
        }
        fn module_command(&mut self, words: &[i32], argv: &[String]) -> Result<i32, GuestError> {
            self.calls.push((words.to_vec(), argv.len()));
            Ok(1)
        }
        fn read_ref_entity(&self, pointer: usize) -> Result<Option<RefEntityView>, GuestError> {
            Ok(self.ref_entities.get(&pointer).cloned())
        }
        fn close_module(&mut self) {
            self.closed = true;
        }
    }

    fn gameplay_module() -> ModuleId {
        ModuleId {
            id: "test:game".to_string(),
            artifact_path: "vm/qagame.qvm".to_string(),
            digest: "sha256:game".to_string(),
            revision: "1".to_string(),
        }
    }

    fn cgame_module() -> ModuleId {
        ModuleId {
            id: "test:cgame".to_string(),
            artifact_path: "vm/cgame.qvm".to_string(),
            digest: "sha256:cgame".to_string(),
            revision: "1".to_string(),
        }
    }

    fn fixture_artifact() -> QvmArtifact {
        QvmArtifact {
            module: cgame_module(),
            role: QvmRole::Cgame,
            abi_profile: None,
            image: QvmImage {
                instructions: vec![
                    QvmInstruction::word(QvmOpcode::OpEnter, 32),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                ],
                data_length: 200000,
                literal_length: 0,
                bss_length: 0,
                initialized_length: 200000,
                allocated_data_length: 200000,
            },
        }
    }

    fn simple_call(entry: usize) -> QvmPresentationCall {
        QvmPresentationCall {
            entry,
            when_weapon_presented: false,
            arguments: Vec::new(),
        }
    }

    fn player_events_declaration() -> QvmModPresentationDeclaration {
        QvmModPresentationDeclaration::PlayerEvents(QvmPlayerEventPresentation {
            gameplay: QvmPresentationProgram {
                path: "vm/qagame.qvm".to_string(),
                digest: "sha256:game".to_string(),
                abi: QvmAbi::Modern,
            },
            cgame: QvmPresentationProgram {
                path: "vm/cgame.qvm".to_string(),
                digest: "sha256:cgame".to_string(),
                abi: QvmAbi::Modern,
            },
            initialize: vec![simple_call(0)],
            refresh: Vec::new(),
            frame: vec![simple_call(0)],
            hud: None,
            storage: PlayerEventStorage {
                game_state: 1000,
                player_state: 22000,
                snapshot_address: 23000,
                snapshot_pointers: vec![90024],
                centities: PlayerEventCentities {
                    address: 80000,
                    stride: 512,
                    capacity: 4,
                    state: 0,
                    origin: 208,
                },
                time: vec![90000],
                frame_time: vec![90004],
                view_origin: vec![90008],
                view_angles: Vec::new(),
                view_axis: Vec::new(),
            },
            project: Vec::new(),
            event: QvmPresentationCall {
                entry: 0,
                when_weapon_presented: false,
                arguments: vec![QvmPresentationArgument::Source(PresentationSource::Event)],
            },
        })
    }

    fn fixture_context() -> QvmPresentationContext {
        QvmPresentationContext {
            client_number: None,
            game_state: SourceGameState::empty(),
            game_state_revision: 0,
            frame_time_ms: 16,
            time_ms: None,
            view_origin: vec3(1.0, 2.0, 3.0),
            view_axis: None,
            weapon_presented: None,
            snapshot: PresentationSnapshot {
                server_time: 100,
                player_state: SourcePlayerState::zeroed(QvmAbi::Modern),
            },
            scene: None,
        }
    }

    #[test]
    fn player_events_initialize_consume_and_advance() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut host = FakeHost::new(fixture_context());
        host.live.insert(actor.clone());
        host.slots.insert(0, actor.clone());
        let mut presentation =
            QvmModPresentation::new(fixture_artifact(), gameplay_module(), player_events_declaration(), host).unwrap();
        presentation.initialize(-1).unwrap();
        assert_eq!(presentation.host().calls.len(), 1);
        let event = SourcePlayerEvent {
            actor: actor.clone(),
            module: gameplay_module(),
            abi: QvmAbi::Modern,
            player_state: SourcePlayerState::zeroed(QvmAbi::Modern),
            origin: vec3(4.0, 5.0, 6.0),
            time: 100,
            event: 7,
            parameter: 3,
            sequence: PlayerEventSequence::Predictable { sequence: 40 },
        };
        presentation.consume(&event, 1).unwrap();
        assert_eq!(presentation.last_sequence(), 1);
        assert_eq!(presentation.host().read_i32(80000 + 180).unwrap(), 7);
        assert_eq!(presentation.host().read_i32(80000 + 184).unwrap(), 3);
        presentation.advance(0).unwrap();
        assert!(presentation.console_command(&["status".to_string()]).unwrap());
        presentation.draw_hud(0).unwrap();
        assert!(presentation.consume(&event, 1).is_err());
        presentation.close();
        assert!(presentation.host().closed);
    }

    #[test]
    fn validation_rejects_mismatched_artifacts() {
        let mut artifact = fixture_artifact();
        artifact.role = QvmRole::Qagame;
        assert!(validate_qvm_mod_presentation(&artifact, &gameplay_module(), &player_events_declaration()).is_err());
        let mut declaration = player_events_declaration();
        if let QvmModPresentationDeclaration::PlayerEvents(declaration) = &mut declaration {
            declaration.storage.player_state = declaration.storage.snapshot_address + 8;
        }
        assert!(validate_qvm_mod_presentation(&fixture_artifact(), &gameplay_module(), &declaration).is_err());
    }

    fn scene_fixture() -> (QvmArtifact, QvmModPresentationDeclaration) {
        let artifact = QvmArtifact {
            module: cgame_module(),
            role: QvmRole::Cgame,
            abi_profile: None,
            image: QvmImage {
                instructions: vec![
                    QvmInstruction::word(QvmOpcode::OpEnter, 32),
                    QvmInstruction::word(QvmOpcode::OpConst, 5),
                    QvmInstruction::word(QvmOpcode::OpCall, 0),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 16),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 8),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                ],
                data_length: 200000,
                literal_length: 0,
                bss_length: 0,
                initialized_length: 200000,
                allocated_data_length: 200000,
            },
        };
        let declaration = QvmModPresentationDeclaration::Scene(QvmScenePresentation {
            gameplay: QvmPresentationProgram {
                path: "vm/qagame.qvm".to_string(),
                digest: "sha256:game".to_string(),
                abi: QvmAbi::Modern,
            },
            cgame: QvmPresentationProgram {
                path: "vm/cgame.qvm".to_string(),
                digest: "sha256:cgame".to_string(),
                abi: QvmAbi::Modern,
            },
            initialize: vec![simple_call(4)],
            refresh: Vec::new(),
            frame: vec![simple_call(4)],
            hud: None,
            cvars: Vec::new(),
            storage: SceneStorage {
                game_state: 1000,
                server_command_sequence: 90024,
                time: vec![90000],
                frame_time: vec![90004],
                view_origin: vec![90008],
                view_angles: Vec::new(),
                view_axis: Vec::new(),
                centities: SceneCentities {
                    address: 80000,
                    stride: 512,
                    capacity: 4,
                    state: 0,
                    previous_event: 256,
                    snapshot_time: 260,
                },
            },
            snapshots: vec![simple_call(4)],
            event_entity_type: 100,
            event_check: SceneBodyEndpoint {
                entry: 7,
                centity_argument: 0,
            },
            body: SceneBodyScope {
                player: SceneBodyEndpoint {
                    entry: 0,
                    centity_argument: 0,
                },
                mesh: SceneMeshEndpoint {
                    entry: 5,
                    entity_argument: 0,
                    state_argument: 1,
                    shader_offset: 112,
                    parts: None,
                },
            },
        });
        (artifact, declaration)
    }

    fn scene_context(actor: &ActorId) -> QvmSceneContext {
        QvmSceneContext {
            revision: 1,
            game_state: SourceGameState::empty(),
            game_state_revision: 0,
            snapshot: QvmSourceSnapshot {
                number: 0,
                server_time: 200,
                flags: 0,
                area_mask: vec![0u8; 32],
                player_state: SourcePlayerState::zeroed(QvmAbi::Modern),
                entities: Vec::new(),
                server_command_sequence: 5,
            },
            actors: vec![QvmSceneActor {
                actor: actor.clone(),
                slot: 0,
                owned: true,
            }],
            commands: Vec::new(),
            baseline: None,
        }
    }

    #[test]
    fn scene_accepts_publications_and_restores() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut context = fixture_context();
        context.scene = Some(scene_context(&actor));
        let mut host = FakeHost::new(context);
        host.live.insert(actor.clone());
        host.slots.insert(0, actor.clone());
        let (artifact, declaration) = scene_fixture();
        let mut presentation = QvmModPresentation::new(artifact, gameplay_module(), declaration, host).unwrap();
        presentation.initialize(-1).unwrap();
        assert_eq!(presentation.snapshot_trap_state().0, 1);
        presentation.advance(1).unwrap();
        assert_eq!(presentation.host().read_i32(90024).unwrap(), 5);
        let saved = presentation.capture_host_state().unwrap();
        let mut context = fixture_context();
        context.scene = Some(scene_context(&actor));
        let mut host = FakeHost::new(context);
        host.live.insert(actor.clone());
        let (artifact, declaration) = scene_fixture();
        let mut restored = QvmModPresentation::new(artifact, gameplay_module(), declaration, host).unwrap();
        restored.restore_host_state(&saved, &|_| Ok(actor.clone())).unwrap();
        assert_eq!(restored.snapshot_trap_state().0, 1);
        assert!(restored.server_command_args(9).is_err());
        restored.note_event_check(80000).unwrap();
    }

    #[test]
    fn body_hooks_capture_mesh_passes() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut context = fixture_context();
        context.scene = Some(scene_context(&actor));
        let mut host = FakeHost::new(context);
        host.live.insert(actor.clone());
        host.slots.insert(0, actor.clone());
        host.ref_entities.insert(
            81000,
            RefEntityView {
                is_model: true,
                custom_shader: 7,
                bytes: vec![1, 2],
            },
        );
        let (artifact, declaration) = scene_fixture();
        let mut presentation = QvmModPresentation::new(artifact, gameplay_module(), declaration, host).unwrap();
        presentation.initialize(-1).unwrap();
        let player = presentation
            .enter_player_mesh(80000, 80000, &[(2, QvmBodyPart::Body)])
            .unwrap();
        assert_eq!(player, Some(0));
        let mesh = presentation.enter_mesh_call(0, 81000, 80000, Some(2));
        assert_eq!(mesh, Some(0));
        assert!(presentation.note_add_ref_entity(81000).unwrap());
        assert!(presentation.note_add_ref_entity(81000).unwrap());
        assert!(!presentation.note_add_ref_entity(99999).unwrap());
        presentation.exit_mesh_call(0);
        presentation.exit_player_mesh(0);
        assert_eq!(presentation.bodies().unwrap().len(), 1);
        assert_eq!(presentation.bodies().unwrap()[0].parts[0].passes.len(), 1);
    }

    #[test]
    fn view_math_matches_forward_vectors() {
        let angles = vector_to_angles(vec3(1.0, 0.0, 0.0));
        assert_eq!((angles.x, angles.y, angles.z), (0.0, 0.0, 0.0));
        let axis = qvm_angles_to_axis(vec3(0.0, 0.0, 0.0));
        assert!((axis[0].x - 1.0).abs() < 1e-6);
    }
}
