//! Presentation checkpoint capture: game state, snapshots, and scene publications.
//!
//! Provenance: `src/compat/qvm/mod-presentation-checkpoint.ts`.
//!
//! Local mirrors: [`SourceGameState`] (mirror of `SourceGameStateRecord` in
//! `src/network/q3/game-state.ts`), [`SourcePlayerState`] /
//! [`SourceEntityState`] (byte-oriented mirrors of the player/entity record
//! owners: exact byte round-trips with decoded event words only, which is all
//! this batch inspects), [`QvmSourceSnapshot`] (mirror of
//! `client-state-record.ts`), [`ModScenePublication`] (mirror of
//! `QvmModScenePublication` in `src/world/session/mod-presentations.ts`).
//! [`QvmSceneContext`] is owned by [`super::mod_presentation`]; record sizes
//! reuse [`super::mod_provider`]. [`read_saved_actor_id`] is shared by sibling
//! ports in this batch.

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{vec3, Vec3};

use super::mod_provider::{
    namespaced_id, qvm_entity_state_bytes, qvm_player_state_bytes, qvm_snapshot_bytes, ProfileReader, ProfileValue,
    QvmAbi,
};
use crate::error::GuestError;

/// Player-state record length.
#[must_use]
pub fn player_state_len(abi: QvmAbi) -> usize {
    qvm_player_state_bytes(abi)
}

/// Entity-state record length.
#[must_use]
pub fn entity_state_len(abi: QvmAbi) -> usize {
    qvm_entity_state_bytes(abi)
}

/// Snapshot record length.
#[must_use]
pub fn snapshot_len(abi: QvmAbi) -> usize {
    qvm_snapshot_bytes(abi)
}

/// Read a saved actor reference.
pub fn read_saved_actor_id(reader: &ProfileReader<'_>) -> Result<SavedActorId, GuestError> {
    Ok(SavedActorId {
        slot: reader.field("slot")?.integer(0)? as u32,
        generation: reader.field("generation")?.integer(0)? as u32,
    })
}

/// Capture a saved actor reference.
#[must_use]
pub fn capture_saved_actor_id(actor: &ActorId) -> ProfileValue {
    let saved = SavedActorId::from(actor);
    ProfileValue::record(vec![
        ("slot", ProfileValue::Int(i64::from(saved.slot))),
        ("generation", ProfileValue::Int(i64::from(saved.generation))),
    ])
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, GuestError> {
    bytes
        .get(offset..offset + 4)
        .ok_or_else(|| GuestError::invalid("source record exceeds its bytes"))
        .map(|word| i32::from_le_bytes([word[0], word[1], word[2], word[3]]))
}

fn write_i32(bytes: &mut [u8], offset: usize, value: i32) -> Result<(), GuestError> {
    let slot = bytes
        .get_mut(offset..offset + 4)
        .ok_or_else(|| GuestError::invalid("source record exceeds its bytes"))?;
    slot.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

/// Source game-state record: 1024 configstring offsets over 16000 data bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceGameState {
    /// String offsets.
    pub offsets: Vec<i32>,
    /// String data.
    pub data: Vec<u8>,
    /// Data count.
    pub count: usize,
}

impl SourceGameState {
    /// Empty record.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            offsets: vec![0; 1024],
            data: vec![0; 16000],
            count: 0,
        }
    }

    /// Build a record from configstring entries.
    pub fn from_entries(entries: impl Iterator<Item = (u32, String)>) -> Result<Self, GuestError> {
        let mut state = Self::empty();
        for (index, value) in entries {
            if index >= 1024 {
                return Err(GuestError::invalid("configstring index exceeds 1024"));
            }
            let bytes = value.as_bytes();
            if state.count + bytes.len() + 1 > 16000 {
                return Err(GuestError::invalid("configstring data exceeds 16000 bytes"));
            }
            state.offsets[index as usize] = state.count as i32;
            state.data[state.count..state.count + bytes.len()].copy_from_slice(bytes);
            state.data[state.count + bytes.len()] = 0;
            state.count += bytes.len() + 1;
        }
        Ok(state)
    }
}

/// Capture a game-state record.
#[must_use]
pub fn capture_presentation_game_state(state: &SourceGameState) -> ProfileValue {
    ProfileValue::record(vec![
        (
            "stringOffsets",
            ProfileValue::Array(
                state
                    .offsets
                    .iter()
                    .map(|offset| ProfileValue::Int(i64::from(*offset)))
                    .collect(),
            ),
        ),
        ("stringData", ProfileValue::Bytes(state.data.clone())),
        ("dataCount", ProfileValue::Int(state.count as i64)),
    ])
}

/// Read a game-state record.
pub fn read_presentation_game_state(reader: &ProfileReader<'_>) -> Result<SourceGameState, GuestError> {
    let offsets = reader
        .field("stringOffsets")?
        .list(|item| item.integer(0).map(|value| value as i32))?;
    let data = reader.field("stringData")?.bytes()?;
    let count = reader.field("dataCount")?.integer(0)? as usize;
    if offsets.len() != 1024
        || data.len() != 16000
        || count > 16000
        || offsets.iter().any(|offset| *offset >= count.max(1) as i32)
    {
        return reader.fail("invalid source gameState extent");
    }
    Ok(SourceGameState { offsets, data, count })
}

/// Source player-state record: exact bytes with decoded event words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePlayerState {
    bytes: Vec<u8>,
    abi: QvmAbi,
}

impl SourcePlayerState {
    /// Wrap exact record bytes.
    pub fn from_bytes(bytes: &[u8], abi: QvmAbi) -> Result<Self, GuestError> {
        if bytes.len() != qvm_player_state_bytes(abi) {
            return Err(GuestError::invalid("invalid source player bytes"));
        }
        Ok(Self {
            bytes: bytes.to_vec(),
            abi,
        })
    }

    /// Zeroed record.
    #[must_use]
    pub fn zeroed(abi: QvmAbi) -> Self {
        Self {
            bytes: vec![0; qvm_player_state_bytes(abi)],
            abi,
        }
    }

    /// Record bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Mutable record bytes.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    /// ABI profile.
    #[must_use]
    pub const fn abi(&self) -> QvmAbi {
        self.abi
    }

    /// Event sequence word.
    pub fn event_sequence(&self) -> i32 {
        read_i32(&self.bytes, 108).unwrap_or(0)
    }

    /// Predictable event slots.
    pub fn events(&self) -> [i32; 2] {
        [
            read_i32(&self.bytes, 112).unwrap_or(0),
            read_i32(&self.bytes, 116).unwrap_or(0),
        ]
    }

    /// Predictable event parameters.
    pub fn event_parameters(&self) -> [i32; 2] {
        [
            read_i32(&self.bytes, 120).unwrap_or(0),
            read_i32(&self.bytes, 124).unwrap_or(0),
        ]
    }

    /// External event.
    pub fn external_event(&self) -> i32 {
        read_i32(&self.bytes, 128).unwrap_or(0)
    }

    /// External event parameter.
    pub fn external_event_param(&self) -> i32 {
        read_i32(&self.bytes, 132).unwrap_or(0)
    }

    /// External event time.
    pub fn external_event_time(&self) -> i32 {
        read_i32(&self.bytes, 136).unwrap_or(0)
    }

    /// Client number.
    pub fn client_number(&self) -> i32 {
        read_i32(&self.bytes, 140).unwrap_or(0)
    }

    /// Ping in milliseconds.
    pub fn ping_ms(&self) -> i32 {
        read_i32(&self.bytes, if self.abi.is_modern() { 452 } else { 440 }).unwrap_or(0)
    }

    /// Write the event sequence word.
    pub fn set_event_sequence(&mut self, value: i32) {
        let _ = write_i32(&mut self.bytes, 108, value);
    }
}

/// Source entity-state record: exact bytes with decoded identity words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEntityState {
    bytes: Vec<u8>,
    abi: QvmAbi,
}

impl SourceEntityState {
    /// Wrap exact record bytes.
    pub fn from_bytes(bytes: &[u8], abi: QvmAbi) -> Result<Self, GuestError> {
        if bytes.len() != qvm_entity_state_bytes(abi) {
            return Err(GuestError::invalid("invalid source entity bytes"));
        }
        Ok(Self {
            bytes: bytes.to_vec(),
            abi,
        })
    }

    /// Zeroed record.
    #[must_use]
    pub fn zeroed(abi: QvmAbi) -> Self {
        Self {
            bytes: vec![0; qvm_entity_state_bytes(abi)],
            abi,
        }
    }

    /// Record bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// ABI profile.
    #[must_use]
    pub const fn abi(&self) -> QvmAbi {
        self.abi
    }

    /// Entity number.
    pub fn number(&self) -> i32 {
        read_i32(&self.bytes, 0).unwrap_or(0)
    }

    /// Entity type.
    pub fn etype(&self) -> i32 {
        read_i32(&self.bytes, 4).unwrap_or(0)
    }

    /// One-shot event.
    pub fn event(&self) -> i32 {
        read_i32(&self.bytes, 180).unwrap_or(0)
    }

    /// One-shot event parameter.
    pub fn event_param(&self) -> i32 {
        read_i32(&self.bytes, 184).unwrap_or(0)
    }

    /// Write the one-shot event words.
    pub fn set_event(&mut self, event: i32, param: i32) {
        let _ = write_i32(&mut self.bytes, 180, event);
        let _ = write_i32(&mut self.bytes, 184, param);
    }
}

/// Source snapshot record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmSourceSnapshot {
    /// Snapshot number.
    pub number: i32,
    /// Server time.
    pub server_time: i32,
    /// Snapshot flags.
    pub flags: i32,
    /// Area mask (32 bytes).
    pub area_mask: Vec<u8>,
    /// Viewer player state.
    pub player_state: SourcePlayerState,
    /// Entities.
    pub entities: Vec<SourceEntityState>,
    /// Server command sequence.
    pub server_command_sequence: i32,
}

/// Encode a snapshot to exact source bytes.
pub fn snapshot_to_bytes(snapshot: &QvmSourceSnapshot, abi: QvmAbi) -> Result<Vec<u8>, GuestError> {
    if snapshot.area_mask.len() != 32 || snapshot.entities.len() > 256 {
        return Err(GuestError::invalid("Invalid source snapshot_t extent"));
    }
    let player_len = qvm_player_state_bytes(abi);
    let entity_len = qvm_entity_state_bytes(abi);
    let mut bytes = vec![0u8; qvm_snapshot_bytes(abi)];
    write_i32(&mut bytes, 0, snapshot.flags)?;
    write_i32(&mut bytes, 4, snapshot.player_state.ping_ms())?;
    write_i32(&mut bytes, 8, snapshot.server_time)?;
    bytes[12..44].copy_from_slice(&snapshot.area_mask);
    bytes[44..44 + player_len].copy_from_slice(snapshot.player_state.bytes());
    write_i32(&mut bytes, 44 + player_len, snapshot.entities.len() as i32)?;
    for (index, entity) in snapshot.entities.iter().enumerate() {
        let at = 48 + player_len + index * entity_len;
        bytes[at..at + entity_len].copy_from_slice(entity.bytes());
    }
    let end = bytes.len();
    write_i32(&mut bytes, end - 4, snapshot.server_command_sequence)?;
    Ok(bytes)
}

/// Decode a snapshot from exact source bytes.
pub fn snapshot_from_bytes(bytes: &[u8], abi: QvmAbi, number: i32) -> Result<QvmSourceSnapshot, GuestError> {
    if bytes.len() != qvm_snapshot_bytes(abi) {
        return Err(GuestError::invalid("invalid source snapshot extent"));
    }
    let player_len = qvm_player_state_bytes(abi);
    let entity_len = qvm_entity_state_bytes(abi);
    let count = read_i32(bytes, 44 + player_len)?;
    if !(0..=256).contains(&count) {
        return Err(GuestError::invalid("invalid source snapshot entity count"));
    }
    let mut entities = Vec::with_capacity(count as usize);
    for index in 0..count as usize {
        let at = 48 + player_len + index * entity_len;
        entities.push(SourceEntityState::from_bytes(&bytes[at..at + entity_len], abi)?);
    }
    Ok(QvmSourceSnapshot {
        number,
        server_time: read_i32(bytes, 8)?,
        flags: read_i32(bytes, 0)?,
        area_mask: bytes[12..44].to_vec(),
        player_state: SourcePlayerState::from_bytes(&bytes[44..44 + player_len], abi)?,
        entities,
        server_command_sequence: read_i32(bytes, bytes.len() - 4)?,
    })
}

/// Capture a snapshot as `{ number, bytes }`.
pub fn capture_presentation_snapshot(snapshot: &QvmSourceSnapshot, abi: QvmAbi) -> Result<ProfileValue, GuestError> {
    Ok(ProfileValue::record(vec![
        ("number", ProfileValue::Int(i64::from(snapshot.number))),
        ("bytes", ProfileValue::Bytes(snapshot_to_bytes(snapshot, abi)?)),
    ]))
}

/// Read a captured snapshot.
pub fn read_presentation_snapshot(reader: &ProfileReader<'_>, abi: QvmAbi) -> Result<QvmSourceSnapshot, GuestError> {
    let number = reader.field("number")?.integer(0)?;
    if number > i64::from(i32::MAX) {
        return reader.fail("invalid source snapshot number");
    }
    let bytes = reader.field("bytes")?.bytes()?;
    snapshot_from_bytes(&bytes, abi, number as i32)
}

/// Capture one bounding box.
#[must_use]
pub fn capture_bounds(min: Vec3, max: Vec3) -> ProfileValue {
    let vector = |value: Vec3| {
        ProfileValue::record(vec![
            ("x", ProfileValue::Float(f64::from(value.x))),
            ("y", ProfileValue::Float(f64::from(value.y))),
            ("z", ProfileValue::Float(f64::from(value.z))),
        ])
    };
    ProfileValue::record(vec![("min", vector(min)), ("max", vector(max))])
}

/// Read one bounding box.
pub fn read_bounds(reader: &ProfileReader<'_>) -> Result<(Vec3, Vec3), GuestError> {
    let vector = |reader: &ProfileReader<'_>| -> Result<Vec3, GuestError> {
        Ok(vec3(
            reader.field("x")?.finite()? as f32,
            reader.field("y")?.finite()? as f32,
            reader.field("z")?.finite()? as f32,
        ))
    };
    Ok((vector(&reader.field("min")?)?, vector(&reader.field("max")?)?))
}

/// Scene entity row of a mod scene publication.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneEntity {
    /// Actor.
    pub actor: ActorId,
    /// Owned flag.
    pub owned: bool,
    /// Linked flag.
    pub linked: bool,
    /// Server flags.
    pub server_flags: i32,
    /// Single client.
    pub single_client: i32,
    /// Absolute minimums.
    pub bounds_min: Vec3,
    /// Absolute maximums.
    pub bounds_max: Vec3,
    /// Entity state.
    pub state: SourceEntityState,
}

/// Scene client row of a mod scene publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneClient {
    /// Actor.
    pub actor: ActorId,
    /// Slot.
    pub slot: usize,
    /// Player state.
    pub state: SourcePlayerState,
}

/// Scene server command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneCommand {
    /// Sequence.
    pub sequence: u64,
    /// Recipient, if targeted.
    pub recipient: Option<ActorId>,
    /// Text.
    pub text: String,
}

/// Mod scene publication (mirror of `QvmModScenePublication`).
#[derive(Debug, Clone, PartialEq)]
pub struct ModScenePublication {
    /// Publication revision.
    pub revision: u64,
    /// Server time.
    pub server_time: i32,
    /// Game-state revision.
    pub game_state_revision: u64,
    /// Game state.
    pub game_state: SourceGameState,
    /// Entities.
    pub entities: Vec<SceneEntity>,
    /// Clients.
    pub clients: Vec<SceneClient>,
    /// Commands.
    pub commands: Vec<SceneCommand>,
}

/// Capture a scene publication.
#[must_use]
pub fn capture_mod_scene_publication(scene: &ModScenePublication, abi: QvmAbi) -> ProfileValue {
    let _ = abi;
    ProfileValue::record(vec![
        ("revision", ProfileValue::Int(scene.revision as i64)),
        ("serverTime", ProfileValue::Int(i64::from(scene.server_time))),
        ("gameStateRevision", ProfileValue::Int(scene.game_state_revision as i64)),
        ("gameState", capture_presentation_game_state(&scene.game_state)),
        (
            "clients",
            ProfileValue::Array(
                scene
                    .clients
                    .iter()
                    .map(|row| {
                        ProfileValue::record(vec![
                            ("actor", capture_saved_actor_id(&row.actor)),
                            ("slot", ProfileValue::Int(row.slot as i64)),
                            ("state", ProfileValue::Bytes(row.state.bytes().to_vec())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "entities",
            ProfileValue::Array(
                scene
                    .entities
                    .iter()
                    .map(|row| {
                        ProfileValue::record(vec![
                            ("actor", capture_saved_actor_id(&row.actor)),
                            ("owned", ProfileValue::Bool(row.owned)),
                            ("linked", ProfileValue::Bool(row.linked)),
                            ("serverFlags", ProfileValue::Int(i64::from(row.server_flags))),
                            ("singleClient", ProfileValue::Int(i64::from(row.single_client))),
                            ("bounds", capture_bounds(row.bounds_min, row.bounds_max)),
                            ("state", ProfileValue::Bytes(row.state.bytes().to_vec())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "commands",
            ProfileValue::Array(
                scene
                    .commands
                    .iter()
                    .map(|command| {
                        ProfileValue::record(vec![
                            ("sequence", ProfileValue::Int(command.sequence as i64)),
                            ("text", ProfileValue::Str(command.text.clone())),
                            (
                                "recipient",
                                command
                                    .recipient
                                    .as_ref()
                                    .map_or(ProfileValue::Null, capture_saved_actor_id),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// Read a scene publication.
pub fn read_mod_scene_publication(
    reader: &ProfileReader<'_>,
    abi: QvmAbi,
    resolve: &dyn Fn(SavedActorId) -> Result<ActorId, GuestError>,
) -> Result<ModScenePublication, GuestError> {
    Ok(ModScenePublication {
        revision: reader.field("revision")?.integer(0)? as u64,
        server_time: reader.field("serverTime")?.integer(0)? as i32,
        game_state_revision: reader.field("gameStateRevision")?.integer(0)? as u64,
        game_state: read_presentation_game_state(&reader.field("gameState")?)?,
        clients: reader.field("clients")?.list(|row| {
            let state = row.field("state")?.bytes()?;
            if state.len() != qvm_player_state_bytes(abi) {
                return row.fail("invalid source player bytes");
            }
            Ok(SceneClient {
                actor: resolve(read_saved_actor_id(&row.field("actor")?)?)?,
                slot: row.field("slot")?.integer(0)? as usize,
                state: SourcePlayerState::from_bytes(&state, abi)?,
            })
        })?,
        entities: reader.field("entities")?.list(|row| {
            let state = row.field("state")?.bytes()?;
            if state.len() != qvm_entity_state_bytes(abi) {
                return row.fail("invalid source entity bytes");
            }
            let (bounds_min, bounds_max) = read_bounds(&row.field("bounds")?)?;
            Ok(SceneEntity {
                actor: resolve(read_saved_actor_id(&row.field("actor")?)?)?,
                owned: row.field("owned")?.boolean()?,
                linked: row.field("linked")?.boolean()?,
                server_flags: row.field("serverFlags")?.integer(0)? as i32,
                single_client: row.field("singleClient")?.integer(0)? as i32,
                bounds_min,
                bounds_max,
                state: SourceEntityState::from_bytes(&state, abi)?,
            })
        })?,
        commands: reader.field("commands")?.list(|row| {
            Ok(SceneCommand {
                sequence: row.field("sequence")?.integer(0)? as u64,
                text: row.field("text")?.string()?,
                recipient: row
                    .field("recipient")?
                    .nullable(|value| resolve(read_saved_actor_id(value)?))?,
            })
        })?,
    })
}

/// Capture a scene context.
pub fn capture_scene_context(
    scene: &super::mod_presentation::QvmSceneContext,
    abi: QvmAbi,
) -> Result<ProfileValue, GuestError> {
    let mut snapshot = scene.snapshot.clone();
    snapshot.number = 0;
    Ok(ProfileValue::record(vec![
        ("revision", ProfileValue::Int(scene.revision as i64)),
        ("gameState", capture_presentation_game_state(&scene.game_state)),
        ("gameStateRevision", ProfileValue::Int(scene.game_state_revision as i64)),
        ("snapshot", capture_presentation_snapshot(&snapshot, abi)?),
        (
            "actors",
            ProfileValue::Array(
                scene
                    .actors
                    .iter()
                    .map(|row| {
                        ProfileValue::record(vec![
                            ("slot", ProfileValue::Int(row.slot as i64)),
                            ("owned", ProfileValue::Bool(row.owned)),
                            ("actor", capture_saved_actor_id(&row.actor)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "commands",
            ProfileValue::Array(
                scene
                    .commands
                    .iter()
                    .map(|command| {
                        ProfileValue::record(vec![
                            ("sequence", ProfileValue::Int(command.sequence as i64)),
                            (
                                "arguments",
                                ProfileValue::Array(
                                    command
                                        .arguments
                                        .iter()
                                        .map(|argument| ProfileValue::Str(argument.clone()))
                                        .collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "baseline",
            match scene.baseline.as_ref() {
                Some(baseline) => capture_scene_context(baseline, abi)?,
                None => ProfileValue::Null,
            },
        ),
    ]))
}

/// Read a scene context.
pub fn read_scene_context(
    reader: &ProfileReader<'_>,
    abi: QvmAbi,
    resolve: &dyn Fn(SavedActorId) -> Result<ActorId, GuestError>,
    depth: usize,
) -> Result<super::mod_presentation::QvmSceneContext, GuestError> {
    use super::mod_presentation::{QvmSceneActor, QvmSceneCommand, QvmSceneContext};
    if depth > 1 {
        return reader.fail("nested scene baseline");
    }
    let baseline = reader
        .field("baseline")?
        .nullable(|value| read_scene_context(value, abi, resolve, depth + 1))?;
    Ok(QvmSceneContext {
        revision: reader.field("revision")?.integer(0)? as u64,
        game_state: read_presentation_game_state(&reader.field("gameState")?)?,
        game_state_revision: reader.field("gameStateRevision")?.integer(0)? as u64,
        snapshot: read_presentation_snapshot(&reader.field("snapshot")?, abi)?,
        actors: reader.field("actors")?.list(|row| {
            Ok(QvmSceneActor {
                slot: row.field("slot")?.integer(0)? as usize,
                owned: row.field("owned")?.boolean()?,
                actor: resolve(read_saved_actor_id(&row.field("actor")?)?)?,
            })
        })?,
        commands: reader.field("commands")?.list(|row| {
            Ok(QvmSceneCommand {
                sequence: row.field("sequence")?.integer(0)? as u64,
                arguments: row.field("arguments")?.list(|value| value.string())?,
            })
        })?,
        baseline: baseline.map(Box::new),
    })
}

/// Read a namespaced item identity (re-export for sibling profile readers).
pub fn read_namespaced(reader: &ProfileReader<'_>) -> Result<String, GuestError> {
    namespaced_id(reader)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    #[test]
    fn game_state_round_trips() {
        let state = SourceGameState::from_entries([(3u32, "hello".to_string())].into_iter()).unwrap();
        let captured = capture_presentation_game_state(&state);
        let restored = read_presentation_game_state(&ProfileReader::new(&captured)).unwrap();
        assert_eq!(restored, state);
        assert_eq!(restored.offsets[3], 0);
        assert_eq!(restored.count, 6);
    }

    #[test]
    fn game_state_rejects_bad_extents() {
        let mut state = SourceGameState::empty();
        state.data.push(0);
        let captured = capture_presentation_game_state(&state);
        assert!(read_presentation_game_state(&ProfileReader::new(&captured)).is_err());
    }

    #[test]
    fn player_state_decodes_event_words() {
        let mut state = SourcePlayerState::zeroed(QvmAbi::Modern);
        write_i32(state.bytes_mut(), 108, 41).unwrap();
        write_i32(state.bytes_mut(), 112, 7).unwrap();
        write_i32(state.bytes_mut(), 124, 9).unwrap();
        write_i32(state.bytes_mut(), 128, 3).unwrap();
        write_i32(state.bytes_mut(), 136, 500).unwrap();
        write_i32(state.bytes_mut(), 140, 2).unwrap();
        assert_eq!(state.event_sequence(), 41);
        assert_eq!(state.events(), [7, 0]);
        assert_eq!(state.event_parameters(), [0, 9]);
        assert_eq!(state.external_event(), 3);
        assert_eq!(state.external_event_time(), 500);
        assert_eq!(state.client_number(), 2);
        assert!(SourcePlayerState::from_bytes(&[0u8; 8], QvmAbi::Modern).is_err());
    }

    #[test]
    fn snapshot_bytes_round_trip() {
        let snapshot = QvmSourceSnapshot {
            number: 4,
            server_time: 900,
            flags: 1,
            area_mask: vec![7u8; 32],
            player_state: SourcePlayerState::zeroed(QvmAbi::Modern),
            entities: vec![SourceEntityState::zeroed(QvmAbi::Modern)],
            server_command_sequence: 12,
        };
        let bytes = snapshot_to_bytes(&snapshot, QvmAbi::Modern).unwrap();
        assert_eq!(bytes.len(), snapshot_len(QvmAbi::Modern));
        let restored = snapshot_from_bytes(&bytes, QvmAbi::Modern, 4).unwrap();
        assert_eq!(restored, snapshot);
        let captured = capture_presentation_snapshot(&snapshot, QvmAbi::Modern).unwrap();
        let reread = read_presentation_snapshot(&ProfileReader::new(&captured), QvmAbi::Modern).unwrap();
        assert_eq!(reread, snapshot);
    }

    #[test]
    fn scene_publication_round_trips() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 2);
        let scene = ModScenePublication {
            revision: 2,
            server_time: 100,
            game_state_revision: 3,
            game_state: SourceGameState::empty(),
            entities: vec![SceneEntity {
                actor: actor.clone(),
                owned: true,
                linked: true,
                server_flags: 1,
                single_client: 0,
                bounds_min: vec3(0.0, 0.0, 0.0),
                bounds_max: vec3(1.0, 1.0, 1.0),
                state: SourceEntityState::zeroed(QvmAbi::Modern),
            }],
            clients: vec![SceneClient {
                actor: actor.clone(),
                slot: 0,
                state: SourcePlayerState::zeroed(QvmAbi::Modern),
            }],
            commands: vec![SceneCommand {
                sequence: 1,
                recipient: None,
                text: "cs 0 x".to_string(),
            }],
        };
        let captured = capture_mod_scene_publication(&scene, QvmAbi::Modern);
        let restored = read_mod_scene_publication(&ProfileReader::new(&captured), QvmAbi::Modern, &|saved| {
            assert_eq!((saved.slot, saved.generation), (1, 2));
            Ok(actor.clone())
        })
        .unwrap();
        assert_eq!(restored, scene);
    }

    #[test]
    fn scene_context_nesting_is_bounded() {
        let context = super::super::mod_presentation::QvmSceneContext {
            revision: 1,
            game_state: SourceGameState::empty(),
            game_state_revision: 1,
            snapshot: QvmSourceSnapshot {
                number: 0,
                server_time: 0,
                flags: 0,
                area_mask: vec![0u8; 32],
                player_state: SourcePlayerState::zeroed(QvmAbi::Modern),
                entities: Vec::new(),
                server_command_sequence: 0,
            },
            actors: Vec::new(),
            commands: Vec::new(),
            baseline: None,
        };
        let captured = capture_scene_context(&context, QvmAbi::Modern).unwrap();
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(0, 0);
        let restored = read_scene_context(
            &ProfileReader::new(&captured),
            QvmAbi::Modern,
            &|_| Ok(actor.clone()),
            0,
        )
        .unwrap();
        assert_eq!(restored.revision, 1);
        assert!(read_scene_context(
            &ProfileReader::new(&captured),
            QvmAbi::Modern,
            &|_| Ok(actor.clone()),
            2
        )
        .is_err());
    }
}
