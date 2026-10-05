//! Unified control channel codecs.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-control.ts`
//! (`encodeUnifiedControl`, `decodeUnifiedControl`, `encodeUnifiedInputs`,
//! `decodeUnifiedInputs`, `encodeUnifiedHandshake`, `decodeUnifiedHandshake`,
//! `UnifiedControl`, `UnifiedInput`, `UnifiedInputBatch`, `UnifiedHandshake`).
//!
//! The reliable control channel carries session offers, admission, resource
//! keys, event payloads, component updates, and console commands; the input
//! channel carries per-client movement commands; the handshake channel
//! carries connection setup. Composition identities reuse
//! [`super::unified_content`]; component owners and updates reuse
//! [`super::unified_components`]; movement commands reuse
//! [`UserCommand`](qa_net::common::commands::UserCommand).

use qa_content::contract::{ContentId, PresentationOwner, ResourceIdentity, MAX_SAFE_INTEGER};
use qa_net::common::commands::UserCommand;
use qa_world::save::shared::{read_content_id, read_digest};
use qa_world::save::value::{
    arr, decode_checkpoint_value, encode_checkpoint_value, int, namespaced, num, obj, str as json_str, SaveJson,
    SaveReader,
};
use qa_world::WorldError;

use super::unified_components::{
    read_component_owner, read_component_update, write_component_owner, write_component_update, UnifiedComponentError,
    UnifiedComponentUpdate,
};
use super::unified_content::{
    read_unified_composition, write_unified_composition, UnifiedCompositionIdentity, UnifiedContentError,
};
use super::unified_frame_codec::UnifiedResourceKey;

/// Maximum control payload in bytes (donor `qts-control` limit).
pub const MAX_CONTROL_BYTES: usize = 4 * 1024 * 1024;
/// Maximum input payload in bytes (donor `qts-input` limit).
pub const MAX_INPUT_BYTES: usize = 65536;
/// Maximum handshake payload in bytes (donor `qts-connect` limit).
pub const MAX_HANDSHAKE_BYTES: usize = 512;

/// Unified control codec failure.
#[derive(Debug, thiserror::Error)]
pub enum UnifiedControlError {
    /// Checkpoint value failure.
    #[error(transparent)]
    World(#[from] WorldError),
    /// Component failure.
    #[error(transparent)]
    Component(#[from] UnifiedComponentError),
    /// Content failure.
    #[error(transparent)]
    Content(#[from] UnifiedContentError),
    /// Control failure.
    #[error("{0}")]
    Control(String),
}

/// Wire actor reference (donor `UnifiedActorReference`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnifiedActorReference {
    /// Slot.
    pub slot: i64,
    /// Generation.
    pub generation: i64,
}

/// Server mode (donor control `mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedServerMode {
    /// Singleplayer.
    Singleplayer,
    /// Cooperative.
    Coop,
    /// Deathmatch.
    Deathmatch,
}

/// Control message (donor `UnifiedControl`).
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum UnifiedControl {
    /// Session offer.
    Offer {
        /// World epoch.
        epoch: u64,
        /// Composition identity.
        composition: UnifiedCompositionIdentity,
        /// Server mode.
        mode: UnifiedServerMode,
        /// Maximum clients.
        max_clients: i64,
    },
    /// Client ready.
    Ready {
        /// World epoch.
        epoch: u64,
        /// Composition digest.
        composition: String,
        /// Userinfo.
        userinfo: String,
    },
    /// Client admitted.
    Admitted {
        /// World epoch.
        epoch: u64,
        /// Client reference.
        client: UnifiedActorReference,
        /// Actor reference.
        actor: UnifiedActorReference,
        /// Source entity number.
        source_entity: i64,
    },
    /// Resource keys.
    Resources {
        /// World epoch.
        epoch: u64,
        /// Resource keys.
        resources: Vec<UnifiedResourceKey>,
    },
    /// Event payloads.
    Events {
        /// World epoch.
        epoch: u64,
        /// Frame number.
        frame: i64,
        /// Presentation payload.
        payload: Vec<u8>,
        /// Simulation payload.
        simulation: Vec<u8>,
    },
    /// Component update.
    Components {
        /// World epoch.
        epoch: u64,
        /// Update.
        update: UnifiedComponentUpdate,
    },
    /// Component command.
    ComponentCommand {
        /// World epoch.
        epoch: u64,
        /// Component owner.
        owner: PresentationOwner,
        /// Component generation.
        generation: i64,
        /// Command arguments.
        args: Vec<String>,
    },
    /// Console command.
    Command {
        /// World epoch.
        epoch: u64,
        /// Command name.
        name: String,
        /// Command arguments.
        args: Vec<String>,
    },
    /// Userinfo update.
    Userinfo {
        /// World epoch.
        epoch: u64,
        /// Userinfo value.
        value: String,
    },
    /// Disconnect.
    Disconnect {
        /// Reason.
        reason: String,
    },
}

/// Arsenal intent (donor `ArsenalIntent` with its optional impulse).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedArsenalIntent {
    /// Owning provider.
    pub provider: String,
    /// Requested weapon.
    pub weapon: Option<String>,
    /// Use-holdable flag.
    pub use_holdable: bool,
    /// Impulse.
    pub impulse: Option<i64>,
}

/// One client input (donor `UnifiedInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedInput {
    /// Input sequence.
    pub sequence: i64,
    /// Movement command.
    pub command: UserCommand,
    /// Arsenal intent.
    pub arsenal: Option<UnifiedArsenalIntent>,
}

/// Client input batch (donor `UnifiedInputBatch`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedInputBatch {
    /// World epoch.
    pub epoch: u64,
    /// Input commands.
    pub commands: Vec<UnifiedInput>,
}

/// Connection handshake (donor `UnifiedHandshake`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnifiedHandshake {
    /// Hello with a nonce.
    Hello {
        /// Nonce token.
        nonce: String,
    },
    /// Challenge with nonce and token.
    Challenge {
        /// Nonce token.
        nonce: String,
        /// Connection token.
        token: String,
    },
    /// Connect with nonce and token.
    Connect {
        /// Nonce token.
        nonce: String,
        /// Connection token.
        token: String,
    },
}

fn bounded(reader: SaveReader, minimum: i64, maximum: i64) -> Result<i64, WorldError> {
    let value = reader.integer(minimum)?;
    if value <= maximum {
        Ok(value)
    } else {
        Err(reader.fail("number exceeds protocol limit"))
    }
}

fn protocol_string(reader: SaveReader, maximum: usize) -> Result<String, WorldError> {
    let value = reader.string()?;
    if value.len() <= maximum && !value.contains('\0') {
        Ok(value)
    } else {
        Err(reader.fail("invalid protocol string"))
    }
}

fn read_actor(reader: SaveReader) -> Result<UnifiedActorReference, WorldError> {
    Ok(UnifiedActorReference {
        slot: bounded(reader.field("slot"), 0, 1_048_575)?,
        generation: reader.field("generation").integer(0)?,
    })
}

fn write_actor(value: UnifiedActorReference) -> SaveJson {
    obj(vec![("slot", int(value.slot)), ("generation", int(value.generation))])
}

fn read_token(reader: SaveReader) -> Result<String, WorldError> {
    let value = protocol_string(reader.clone(), 32)?;
    if value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(value)
    } else {
        Err(reader.fail("invalid connection token"))
    }
}

fn read_vector(reader: SaveReader) -> Result<[f64; 3], WorldError> {
    Ok([
        reader.field("x").finite()?,
        reader.field("y").finite()?,
        reader.field("z").finite()?,
    ])
}

fn write_vector(value: [f64; 3]) -> SaveJson {
    obj(vec![("x", num(value[0])), ("y", num(value[1])), ("z", num(value[2]))])
}

fn read_angles(reader: SaveReader) -> Result<[f64; 3], WorldError> {
    let values = reader.list(|value| bounded(value, i64::from(i32::MIN), i64::from(i32::MAX)))?;
    if values.len() != 3 {
        return Err(reader.fail("expected three command angles"));
    }
    Ok([values[0] as f64, values[1] as f64, values[2] as f64])
}

fn write_angles(value: [f64; 3]) -> SaveJson {
    arr(vec![num(value[0]), num(value[1]), num(value[2])])
}

fn read_command(reader: SaveReader) -> Result<UserCommand, WorldError> {
    let kind =
        reader
            .field("kind")
            .choice_str(&["q1-netquake", "q1-quakeworld", "q2-classic", "q2-rerelease", "q3"])?;
    let buttons = bounded(reader.field("buttons"), 0, i64::from(u32::MAX))? as f64;
    let movement = |name: &str| bounded(reader.field(name), -32768, 32767).map(|value| value as f64);
    match kind.as_str() {
        "q3" => Ok(UserCommand::Q3 {
            server_time_milliseconds: bounded(reader.field("serverTimeMilliseconds"), 0, i64::from(i32::MAX))? as f64,
            angle_words: read_angles(reader.field("angleWords"))?,
            buttons,
            weapon: bounded(reader.field("weapon"), 0, 255)? as f64,
            forward_move: movement("forwardMove")?,
            right_move: movement("rightMove")?,
            up_move: movement("upMove")?,
        }),
        "q2-rerelease" => Ok(UserCommand::Q2Rerelease {
            milliseconds: bounded(reader.field("milliseconds"), 0, 1000)? as f64,
            angles: read_vector(reader.field("angles"))?,
            forward_move: reader.field("forwardMove").finite()?,
            side_move: reader.field("sideMove").finite()?,
            buttons,
            server_frame: bounded(reader.field("serverFrame"), -1, i64::from(i32::MAX))? as f64,
        }),
        _ => {
            let forward_move = movement("forwardMove")?;
            let side_move = movement("sideMove")?;
            let up_move = movement("upMove")?;
            let impulse = bounded(reader.field("impulse"), 0, 255)? as f64;
            match kind.as_str() {
                "q1-netquake" => Ok(UserCommand::Q1Netquake {
                    acknowledged_server_time_seconds: reader.field("acknowledgedServerTimeSeconds").finite()?,
                    view_angles: read_vector(reader.field("viewAngles"))?,
                    forward_move,
                    side_move,
                    up_move,
                    buttons,
                    impulse,
                }),
                "q1-quakeworld" => Ok(UserCommand::Q1Quakeworld {
                    milliseconds: bounded(reader.field("milliseconds"), 0, 255)? as f64,
                    angles: read_vector(reader.field("angles"))?,
                    forward_move,
                    side_move,
                    up_move,
                    buttons,
                    impulse,
                }),
                _ => Ok(UserCommand::Q2Classic {
                    milliseconds: bounded(reader.field("milliseconds"), 0, 255)? as f64,
                    angle_shorts: read_angles(reader.field("angleShorts"))?,
                    forward_move,
                    side_move,
                    up_move,
                    buttons,
                    impulse,
                    light_level: bounded(reader.field("lightLevel"), 0, 255)? as f64,
                }),
            }
        }
    }
}

fn write_command(command: &UserCommand) -> SaveJson {
    match command {
        UserCommand::Q1Netquake {
            acknowledged_server_time_seconds,
            view_angles,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
        } => obj(vec![
            ("kind", json_str("q1-netquake")),
            ("buttons", num(*buttons)),
            ("forwardMove", num(*forward_move)),
            ("sideMove", num(*side_move)),
            ("upMove", num(*up_move)),
            ("impulse", num(*impulse)),
            ("acknowledgedServerTimeSeconds", num(*acknowledged_server_time_seconds)),
            ("viewAngles", write_vector(*view_angles)),
        ]),
        UserCommand::Q1Quakeworld {
            milliseconds,
            angles,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
        } => obj(vec![
            ("kind", json_str("q1-quakeworld")),
            ("buttons", num(*buttons)),
            ("forwardMove", num(*forward_move)),
            ("sideMove", num(*side_move)),
            ("upMove", num(*up_move)),
            ("impulse", num(*impulse)),
            ("milliseconds", num(*milliseconds)),
            ("angles", write_vector(*angles)),
        ]),
        UserCommand::Q2Classic {
            milliseconds,
            angle_shorts,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
            light_level,
        } => obj(vec![
            ("kind", json_str("q2-classic")),
            ("buttons", num(*buttons)),
            ("forwardMove", num(*forward_move)),
            ("sideMove", num(*side_move)),
            ("upMove", num(*up_move)),
            ("impulse", num(*impulse)),
            ("milliseconds", num(*milliseconds)),
            ("angleShorts", write_angles(*angle_shorts)),
            ("lightLevel", num(*light_level)),
        ]),
        UserCommand::Q2Rerelease {
            milliseconds,
            angles,
            forward_move,
            side_move,
            buttons,
            server_frame,
        } => obj(vec![
            ("kind", json_str("q2-rerelease")),
            ("buttons", num(*buttons)),
            ("milliseconds", num(*milliseconds)),
            ("angles", write_vector(*angles)),
            ("forwardMove", num(*forward_move)),
            ("sideMove", num(*side_move)),
            ("serverFrame", num(*server_frame)),
        ]),
        UserCommand::Q3 {
            server_time_milliseconds,
            angle_words,
            buttons,
            weapon,
            forward_move,
            right_move,
            up_move,
        } => obj(vec![
            ("kind", json_str("q3")),
            ("buttons", num(*buttons)),
            ("serverTimeMilliseconds", num(*server_time_milliseconds)),
            ("angleWords", write_angles(*angle_words)),
            ("weapon", num(*weapon)),
            ("forwardMove", num(*forward_move)),
            ("rightMove", num(*right_move)),
            ("upMove", num(*up_move)),
        ]),
    }
}

fn read_arsenal(reader: SaveReader) -> Result<UnifiedArsenalIntent, WorldError> {
    Ok(UnifiedArsenalIntent {
        provider: namespaced(reader.field("provider"))?,
        weapon: reader.field("weapon").nullable(namespaced)?,
        use_holdable: reader.field("useHoldable").boolean()?,
        impulse: if reader.field("impulse").value.is_none() {
            None
        } else {
            Some(bounded(reader.field("impulse"), 0, 255)?)
        },
    })
}

fn write_arsenal(intent: &UnifiedArsenalIntent) -> SaveJson {
    let mut members = vec![
        ("provider", json_str(&intent.provider)),
        (
            "weapon",
            intent.weapon.as_ref().map_or(SaveJson::Null, |weapon| json_str(weapon)),
        ),
        ("useHoldable", SaveJson::Bool(intent.use_holdable)),
    ];
    if let Some(impulse) = intent.impulse {
        members.push(("impulse", int(impulse)));
    }
    obj(members)
}

fn valid_resource_member(path: &str) -> bool {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return false;
    }
    let mut chars = path.chars();
    if matches!(chars.next(), Some(first) if first.is_ascii_alphabetic()) && chars.next() == Some(':') {
        return false;
    }
    !path
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
}

fn read_key(reader: SaveReader) -> Result<UnifiedResourceKey, WorldError> {
    let path = protocol_string(reader.field("path"), 1024)?;
    if !valid_resource_member(&path) {
        return Err(reader.fail("invalid resource member"));
    }
    let identity_field = reader.field("identity");
    let identity = ResourceIdentity::parse(&identity_field.string()?)
        .map(|parsed| parsed.canonical())
        .ok_or_else(|| identity_field.fail("expected a resource identity"))?;
    Ok(UnifiedResourceKey {
        content: ContentId(read_content_id(reader.field("content"))?),
        path,
        identity,
        byte_length: bounded(reader.field("byteLength"), 0, i64::from(i32::MAX)).and_then(|length| {
            u64::try_from(length).map_err(|_| reader.field("byteLength").fail("resource length exceeds its range"))
        })?,
    })
}

fn write_key(key: &UnifiedResourceKey) -> SaveJson {
    obj(vec![
        ("content", json_str(key.content.as_str())),
        ("path", json_str(&key.path)),
        ("identity", json_str(&key.identity)),
        ("byteLength", int(key.byte_length as i64)),
    ])
}

fn decode_envelope(bytes: &[u8], schema: &str, maximum: usize) -> Result<SaveJson, UnifiedControlError> {
    if bytes.len() > maximum {
        return Err(UnifiedControlError::Control(format!("{schema} exceeds protocol limit")));
    }
    let value = decode_checkpoint_value(bytes)?;
    let reader = SaveReader::new(&value);
    reader.field("schema").literal_str(schema)?;
    reader.field("version").literal_i64(1)?;
    Ok(value)
}

fn envelope_value(value: &SaveJson) -> SaveReader<'_> {
    SaveReader::new(value).field("value")
}

fn read_epoch(reader: &SaveReader) -> Result<u64, WorldError> {
    let epoch = bounded(reader.field("epoch"), 1, i64::from(u32::MAX))?;
    u64::try_from(epoch).map_err(|_| reader.field("epoch").fail("control epoch exceeds its range"))
}

fn valid_command_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' || first == '+' => {}
        _ => return false,
    }
    chars.all(|char| char.is_ascii_alphanumeric() || char == '_' || char == '+' || char == '-')
}

/// Read a control value (donor `decodeUnifiedControl` payload).
pub fn read_unified_control(reader: SaveReader) -> Result<UnifiedControl, UnifiedControlError> {
    let kind = reader.field("kind").choice_str(&[
        "offer",
        "ready",
        "admitted",
        "resources",
        "events",
        "components",
        "component-command",
        "command",
        "userinfo",
        "disconnect",
    ])?;
    if kind.as_str() == "disconnect" {
        return Ok(UnifiedControl::Disconnect {
            reason: protocol_string(reader.field("reason"), 1024)?,
        });
    }
    let epoch = read_epoch(&reader)?;
    match kind.as_str() {
        "offer" => {
            let mode = reader
                .field("mode")
                .choice_str(&["singleplayer", "coop", "deathmatch"])?;
            Ok(UnifiedControl::Offer {
                epoch,
                composition: read_unified_composition(reader.field("composition"))?,
                mode: match mode.as_str() {
                    "singleplayer" => UnifiedServerMode::Singleplayer,
                    "coop" => UnifiedServerMode::Coop,
                    _ => UnifiedServerMode::Deathmatch,
                },
                max_clients: bounded(reader.field("maxClients"), 1, 256)?,
            })
        }
        "ready" => Ok(UnifiedControl::Ready {
            epoch,
            composition: read_digest(reader.field("composition"))?,
            userinfo: protocol_string(reader.field("userinfo"), 8192)?,
        }),
        "admitted" => Ok(UnifiedControl::Admitted {
            epoch,
            client: read_actor(reader.field("client"))?,
            actor: read_actor(reader.field("actor"))?,
            source_entity: reader.field("sourceEntity").integer(0)?,
        }),
        "resources" => {
            let resources = reader.field("resources").list(read_key)?;
            if resources.len() > 32768 {
                return Err(reader.fail("too many resource declarations").into());
            }
            Ok(UnifiedControl::Resources { epoch, resources })
        }
        "events" => Ok(UnifiedControl::Events {
            epoch,
            frame: reader.field("frame").integer(0)?,
            payload: reader.field("payload").bytes()?,
            simulation: reader.field("simulation").bytes()?,
        }),
        "components" => Ok(UnifiedControl::Components {
            epoch,
            update: read_component_update(reader.field("update"))?,
        }),
        "component-command" => {
            let args = reader.field("args").list(|value| protocol_string(value, 8192))?;
            if args.is_empty() || args.len() > 128 {
                return Err(reader.fail("invalid component command").into());
            }
            let generation = reader.field("generation").integer(0)?;
            if u64::try_from(generation).is_ok_and(|value| value > MAX_SAFE_INTEGER) {
                return Err(reader.fail("number exceeds protocol limit").into());
            }
            Ok(UnifiedControl::ComponentCommand {
                epoch,
                owner: read_component_owner(reader.field("owner"))?,
                generation,
                args,
            })
        }
        "command" => {
            let name = protocol_string(reader.field("name"), 128)?;
            let args = reader.field("args").list(|value| protocol_string(value, 8192))?;
            if !valid_command_name(&name) || args.len() > 128 {
                return Err(reader.fail("invalid command").into());
            }
            Ok(UnifiedControl::Command { epoch, name, args })
        }
        _ => Ok(UnifiedControl::Userinfo {
            epoch,
            value: protocol_string(reader.field("value"), 8192)?,
        }),
    }
}

/// Write a control value (donor `encodeUnifiedControl` payload).
pub fn write_unified_control(control: &UnifiedControl) -> SaveJson {
    match control {
        UnifiedControl::Offer {
            epoch,
            composition,
            mode,
            max_clients,
        } => obj(vec![
            ("kind", json_str("offer")),
            ("epoch", int(*epoch as i64)),
            ("composition", write_unified_composition(composition)),
            (
                "mode",
                json_str(match mode {
                    UnifiedServerMode::Singleplayer => "singleplayer",
                    UnifiedServerMode::Coop => "coop",
                    UnifiedServerMode::Deathmatch => "deathmatch",
                }),
            ),
            ("maxClients", int(*max_clients)),
        ]),
        UnifiedControl::Ready {
            epoch,
            composition,
            userinfo,
        } => obj(vec![
            ("kind", json_str("ready")),
            ("epoch", int(*epoch as i64)),
            ("composition", json_str(composition)),
            ("userinfo", json_str(userinfo)),
        ]),
        UnifiedControl::Admitted {
            epoch,
            client,
            actor,
            source_entity,
        } => obj(vec![
            ("kind", json_str("admitted")),
            ("epoch", int(*epoch as i64)),
            ("client", write_actor(*client)),
            ("actor", write_actor(*actor)),
            ("sourceEntity", int(*source_entity)),
        ]),
        UnifiedControl::Resources { epoch, resources } => obj(vec![
            ("kind", json_str("resources")),
            ("epoch", int(*epoch as i64)),
            ("resources", arr(resources.iter().map(write_key).collect())),
        ]),
        UnifiedControl::Events {
            epoch,
            frame,
            payload,
            simulation,
        } => obj(vec![
            ("kind", json_str("events")),
            ("epoch", int(*epoch as i64)),
            ("frame", int(*frame)),
            ("payload", SaveJson::Bytes(payload.clone())),
            ("simulation", SaveJson::Bytes(simulation.clone())),
        ]),
        UnifiedControl::Components { epoch, update } => obj(vec![
            ("kind", json_str("components")),
            ("epoch", int(*epoch as i64)),
            ("update", write_component_update(update)),
        ]),
        UnifiedControl::ComponentCommand {
            epoch,
            owner,
            generation,
            args,
        } => obj(vec![
            ("kind", json_str("component-command")),
            ("epoch", int(*epoch as i64)),
            ("owner", write_component_owner(owner)),
            ("generation", int(*generation)),
            ("args", arr(args.iter().map(|arg| json_str(arg)).collect())),
        ]),
        UnifiedControl::Command { epoch, name, args } => obj(vec![
            ("kind", json_str("command")),
            ("epoch", int(*epoch as i64)),
            ("name", json_str(name)),
            ("args", arr(args.iter().map(|arg| json_str(arg)).collect())),
        ]),
        UnifiedControl::Userinfo { epoch, value } => obj(vec![
            ("kind", json_str("userinfo")),
            ("epoch", int(*epoch as i64)),
            ("value", json_str(value)),
        ]),
        UnifiedControl::Disconnect { reason } => {
            obj(vec![("kind", json_str("disconnect")), ("reason", json_str(reason))])
        }
    }
}

/// Encode a control message (donor `encodeUnifiedControl`).
#[must_use]
pub fn encode_unified_control(control: &UnifiedControl) -> Vec<u8> {
    encode_checkpoint_value(&obj(vec![
        ("schema", json_str("qts-control")),
        ("version", int(1)),
        ("value", write_unified_control(control)),
    ]))
}

/// Decode a control message (donor `decodeUnifiedControl`).
pub fn decode_unified_control(bytes: &[u8]) -> Result<UnifiedControl, UnifiedControlError> {
    let value = decode_envelope(bytes, "qts-control", MAX_CONTROL_BYTES)?;
    read_unified_control(envelope_value(&value))
}

/// Read an input batch (donor `decodeUnifiedInputs` payload).
pub fn read_unified_inputs(reader: SaveReader) -> Result<UnifiedInputBatch, UnifiedControlError> {
    let commands = reader.field("commands").list(|entry| {
        Ok::<_, UnifiedControlError>(UnifiedInput {
            sequence: entry.field("sequence").integer(0)?,
            command: read_command(entry.field("command"))?,
            arsenal: if entry.field("arsenal").value.is_none() {
                None
            } else {
                Some(read_arsenal(entry.field("arsenal"))?)
            },
        })
    })?;
    if commands.len() > 64 {
        return Err(reader.fail("too many input commands").into());
    }
    Ok(UnifiedInputBatch {
        epoch: read_epoch(&reader)?,
        commands,
    })
}

/// Write an input batch (donor `encodeUnifiedInputs` payload).
pub fn write_unified_inputs(batch: &UnifiedInputBatch) -> SaveJson {
    obj(vec![
        ("epoch", int(batch.epoch as i64)),
        (
            "commands",
            arr(batch
                .commands
                .iter()
                .map(|entry| {
                    let mut members = vec![
                        ("sequence", int(entry.sequence)),
                        ("command", write_command(&entry.command)),
                    ];
                    if let Some(arsenal) = &entry.arsenal {
                        members.push(("arsenal", write_arsenal(arsenal)));
                    }
                    obj(members)
                })
                .collect()),
        ),
    ])
}

/// Encode an input batch (donor `encodeUnifiedInputs`).
#[must_use]
pub fn encode_unified_inputs(batch: &UnifiedInputBatch) -> Vec<u8> {
    encode_checkpoint_value(&obj(vec![
        ("schema", json_str("qts-input")),
        ("version", int(1)),
        ("value", write_unified_inputs(batch)),
    ]))
}

/// Decode an input batch (donor `decodeUnifiedInputs`).
pub fn decode_unified_inputs(bytes: &[u8]) -> Result<UnifiedInputBatch, UnifiedControlError> {
    let value = decode_envelope(bytes, "qts-input", MAX_INPUT_BYTES)?;
    read_unified_inputs(envelope_value(&value))
}

/// Write a handshake (donor `encodeUnifiedHandshake` payload).
pub fn write_unified_handshake(handshake: &UnifiedHandshake) -> SaveJson {
    match handshake {
        UnifiedHandshake::Hello { nonce } => obj(vec![("kind", json_str("hello")), ("nonce", json_str(nonce))]),
        UnifiedHandshake::Challenge { nonce, token } => obj(vec![
            ("kind", json_str("challenge")),
            ("nonce", json_str(nonce)),
            ("token", json_str(token)),
        ]),
        UnifiedHandshake::Connect { nonce, token } => obj(vec![
            ("kind", json_str("connect")),
            ("nonce", json_str(nonce)),
            ("token", json_str(token)),
        ]),
    }
}

/// Encode a handshake (donor `encodeUnifiedHandshake`).
#[must_use]
pub fn encode_unified_handshake(handshake: &UnifiedHandshake) -> Vec<u8> {
    encode_checkpoint_value(&obj(vec![
        ("schema", json_str("qts-connect")),
        ("version", int(1)),
        ("value", write_unified_handshake(handshake)),
    ]))
}

/// Decode a handshake, returning `None` for any malformed offer (donor `decodeUnifiedHandshake`).
#[must_use]
pub fn decode_unified_handshake(bytes: &[u8]) -> Option<UnifiedHandshake> {
    if bytes.len() > MAX_HANDSHAKE_BYTES {
        return None;
    }
    let outcome: Result<UnifiedHandshake, UnifiedControlError> = (|| {
        let value = decode_envelope(bytes, "qts-connect", MAX_HANDSHAKE_BYTES)?;
        let reader = envelope_value(&value);
        let kind = reader.field("kind").choice_str(&["hello", "challenge", "connect"])?;
        let nonce = read_token(reader.field("nonce"))?;
        match kind.as_str() {
            "hello" => Ok(UnifiedHandshake::Hello { nonce }),
            "challenge" => Ok(UnifiedHandshake::Challenge {
                nonce,
                token: read_token(reader.field("token"))?,
            }),
            _ => Ok(UnifiedHandshake::Connect {
                nonce,
                token: read_token(reader.field("token"))?,
            }),
        }
    })();
    outcome.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::recipe::fixture_recipe;
    use qa_core::identity::ProviderId;

    use super::super::unified_content::create_unified_composition;

    fn offer() -> UnifiedControl {
        let identity = create_unified_composition(&fixture_recipe(), &[]).unwrap();
        UnifiedControl::Offer {
            epoch: 2,
            composition: identity,
            mode: UnifiedServerMode::Deathmatch,
            max_clients: 16,
        }
    }

    #[test]
    fn every_kind_round_trips() {
        let digest = "sha256:".to_string() + &"ab".repeat(32);
        let key = || UnifiedResourceKey {
            content: ContentId("q1:classic:base:1".to_string()),
            path: "maps/e1m1.bsp".to_string(),
            identity: "identity:0:0:8:0".to_string(),
            byte_length: 8,
        };
        let controls = vec![
            offer(),
            UnifiedControl::Ready {
                epoch: 1,
                composition: digest.clone(),
                userinfo: "name\\player".to_string(),
            },
            UnifiedControl::Admitted {
                epoch: 3,
                client: UnifiedActorReference { slot: 1, generation: 0 },
                actor: UnifiedActorReference { slot: 2, generation: 0 },
                source_entity: 7,
            },
            UnifiedControl::Resources {
                epoch: 4,
                resources: vec![key()],
            },
            UnifiedControl::Events {
                epoch: 5,
                frame: 9,
                payload: vec![1, 2, 3],
                simulation: vec![4, 5],
            },
            UnifiedControl::Userinfo {
                epoch: 6,
                value: "name\\other".to_string(),
            },
            UnifiedControl::Disconnect {
                reason: "bye".to_string(),
            },
        ];
        for control in controls {
            let encoded = encode_unified_control(&control);
            assert_eq!(decode_unified_control(&encoded).unwrap(), control);
        }
    }

    #[test]
    fn component_command_round_trips() {
        let owner = PresentationOwner {
            provider: ProviderId::new("test", "mod"),
            generation: 2,
        };
        let control = UnifiedControl::ComponentCommand {
            epoch: 7,
            owner,
            generation: 9,
            args: vec!["fire".to_string(), "now".to_string()],
        };
        let encoded = encode_unified_control(&control);
        assert_eq!(decode_unified_control(&encoded).unwrap(), control);
        let command = UnifiedControl::Command {
            epoch: 8,
            name: "say".to_string(),
            args: vec!["hello".to_string()],
        };
        let encoded = encode_unified_control(&command);
        assert_eq!(decode_unified_control(&encoded).unwrap(), command);
    }

    #[test]
    fn control_rejects_bad_bounds() {
        let admitted = |slot: i64| UnifiedControl::Admitted {
            epoch: 3,
            client: UnifiedActorReference { slot, generation: 0 },
            actor: UnifiedActorReference { slot: 2, generation: 0 },
            source_entity: 7,
        };
        assert!(decode_unified_control(&encode_unified_control(&admitted(1_048_576))).is_err());
        assert!(decode_unified_control(&vec![0u8; MAX_CONTROL_BYTES + 1]).is_err());
        let wrong_schema = encode_checkpoint_value(&obj(vec![
            ("schema", json_str("qts-input")),
            ("version", int(1)),
            ("value", write_unified_control(&admitted(1))),
        ]));
        assert!(decode_unified_control(&wrong_schema).is_err());
        let bad_name = UnifiedControl::Command {
            epoch: 1,
            name: "9bad".to_string(),
            args: Vec::new(),
        };
        assert!(decode_unified_control(&encode_unified_control(&bad_name)).is_err());
        let no_args = UnifiedControl::ComponentCommand {
            epoch: 1,
            owner: PresentationOwner {
                provider: ProviderId::new("test", "mod"),
                generation: 1,
            },
            generation: 0,
            args: Vec::new(),
        };
        assert!(decode_unified_control(&encode_unified_control(&no_args)).is_err());
    }

    fn q3_command() -> UserCommand {
        UserCommand::Q3 {
            server_time_milliseconds: 120.0,
            angle_words: [1.0, 2.0, 3.0],
            buttons: 5.0,
            weapon: 2.0,
            forward_move: 10.0,
            right_move: -10.0,
            up_move: 0.0,
        }
    }

    #[test]
    fn inputs_round_trip() {
        let batch = UnifiedInputBatch {
            epoch: 11,
            commands: vec![
                UnifiedInput {
                    sequence: 1,
                    command: UserCommand::Q1Netquake {
                        acknowledged_server_time_seconds: 1.5,
                        view_angles: [0.0, 90.0, 0.0],
                        forward_move: 4.0,
                        side_move: 0.0,
                        up_move: 0.0,
                        buttons: 1.0,
                        impulse: 0.0,
                    },
                    arsenal: Some(UnifiedArsenalIntent {
                        provider: "q1:weapons".to_string(),
                        weapon: Some("q1:ssg".to_string()),
                        use_holdable: true,
                        impulse: Some(3),
                    }),
                },
                UnifiedInput {
                    sequence: 2,
                    command: q3_command(),
                    arsenal: None,
                },
            ],
        };
        let encoded = encode_unified_inputs(&batch);
        assert_eq!(decode_unified_inputs(&encoded).unwrap(), batch);
    }

    #[test]
    fn inputs_reject_overflow() {
        let batch = UnifiedInputBatch {
            epoch: 1,
            commands: (0..65)
                .map(|sequence| UnifiedInput {
                    sequence,
                    command: q3_command(),
                    arsenal: None,
                })
                .collect(),
        };
        assert!(decode_unified_inputs(&encode_unified_inputs(&batch)).is_err());
    }

    #[test]
    fn handshake_round_trips_and_rejects() {
        let nonce = "ab".repeat(16);
        let token = "cd".repeat(16);
        for handshake in [
            UnifiedHandshake::Hello { nonce: nonce.clone() },
            UnifiedHandshake::Challenge {
                nonce: nonce.clone(),
                token: token.clone(),
            },
            UnifiedHandshake::Connect {
                nonce: nonce.clone(),
                token: token.clone(),
            },
        ] {
            let encoded = encode_unified_handshake(&handshake);
            assert_eq!(decode_unified_handshake(&encoded).unwrap(), handshake);
        }
        assert!(decode_unified_handshake(&vec![0u8; MAX_HANDSHAKE_BYTES + 1]).is_none());
        let bad_token = encode_checkpoint_value(&obj(vec![
            ("schema", json_str("qts-connect")),
            ("version", int(1)),
            (
                "value",
                obj(vec![("kind", json_str("hello")), ("nonce", json_str("ZZ"))]),
            ),
        ]));
        assert!(decode_unified_handshake(&bad_token).is_none());
        assert!(decode_unified_handshake(&[]).is_none());
    }
}
