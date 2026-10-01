//! Unified control channel codecs.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-control.ts`
//! (`encodeUnifiedControl`, `decodeUnifiedControl`, `encodeUnifiedClientInput`,
//! `decodeUnifiedClientInput`, `UnifiedControl`, `UnifiedClientInput`).
//!
//! The reliable control channel carries session offers, admission, resource
//! keys, event payloads, and component updates; the input channel carries
//! per-client movement commands. Composition identities reuse
//! [`super::unified_content`]; component updates reuse
//! [`super::unified_components`]; command, module, and arsenal shapes are
//! local mirrors carrying exactly the fields the codec reads.

use qa_content::contract::PresentationOwner;
use qa_core::identity::ActorId;
use qa_world::save::value::{
    arr, decode_checkpoint_value, encode_checkpoint_value, int, namespaced, obj, str as json_str, SaveJson, SaveReader,
};
use qa_world::WorldError;

use super::unified_components::{
    read_component_owner, read_component_update, write_component_update, UnifiedComponentError, UnifiedComponentUpdate,
};
use super::unified_content::{
    read_unified_composition, write_unified_composition, UnifiedCompositionIdentity, UnifiedContentError,
};
use super::unified_event_codec::UnifiedModuleIdentity;
use super::unified_frame_codec::UnifiedResourceKey;
use super::unified_types::UnifiedIdentityDecoder;

/// Maximum control payload in bytes (donor `MAX_CONTROL_BYTES`).
pub const MAX_CONTROL_BYTES: usize = 16 * 1024 * 1024;
/// Maximum input payload in bytes (donor `MAX_INPUT_BYTES`).
pub const MAX_INPUT_BYTES: usize = 1024 * 1024;

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
}

/// Client module reference (donor `readModule` output).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedClientModule {
    /// Module id.
    pub id: String,
    /// Artifact path.
    pub artifact_path: String,
    /// Artifact digest.
    pub digest: String,
    /// Revision.
    pub revision: String,
}

impl From<&UnifiedModuleIdentity> for UnifiedClientModule {
    fn from(value: &UnifiedModuleIdentity) -> Self {
        Self {
            id: value.id.clone(),
            artifact_path: value.artifact_path.clone(),
            digest: value.digest.clone(),
            revision: value.revision.clone(),
        }
    }
}

/// Arsenal intent (donor `ArsenalIntent`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedArsenalIntent {
    /// Owning provider.
    pub provider: String,
    /// Requested weapon.
    pub weapon: Option<String>,
    /// Use-holdable flag.
    pub use_holdable: bool,
    /// Impulse.
    pub impulse: Option<f64>,
}

/// Client command (donor `UserCommand` variants admitted on the wire).
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedClientCommand {
    /// Module command.
    Module {
        /// Module.
        module: UnifiedClientModule,
        /// Command bytes.
        command: Vec<u8>,
    },
    /// Movement command.
    Movement {
        /// Forward milliseconds.
        forward_milliseconds: f64,
        /// Side milliseconds.
        side_milliseconds: f64,
        /// Up milliseconds.
        up_milliseconds: f64,
        /// Buttons.
        buttons: f64,
        /// Impulse.
        impulse: f64,
        /// Mouth (voice) value.
        mouth: f64,
        /// View angles.
        angles: (f64, f64, f64),
    },
    /// Arsenal intent.
    Arsenal {
        /// Intent.
        intent: UnifiedArsenalIntent,
    },
}

/// Client input (donor `UnifiedClientInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedClientInput {
    /// Client actor.
    pub client: ActorId,
    /// Acknowledged frame.
    pub acknowledged_frame: i64,
    /// Input sequence.
    pub sequence: i64,
    /// Command.
    pub command: UnifiedClientCommand,
    /// Component owners.
    pub components: Vec<PresentationOwner>,
}

fn read_actor_reference(reader: SaveReader) -> Result<UnifiedActorReference, WorldError> {
    Ok(UnifiedActorReference {
        slot: reader.field("slot").integer(0)?,
        generation: reader.field("generation").integer(0)?,
    })
}

fn write_actor_reference(value: UnifiedActorReference) -> SaveJson {
    obj(vec![("slot", int(value.slot)), ("generation", int(value.generation))])
}

fn read_owner_set(reader: SaveReader) -> Result<Vec<PresentationOwner>, WorldError> {
    let owners = reader.list(read_component_owner)?;
    if owners.len() > 256 {
        return Err(reader.fail("invalid component owner set"));
    }
    let distinct: std::collections::HashSet<(&str, &str)> = owners
        .iter()
        .map(|owner| (owner.provider.namespace.as_str(), owner.provider.name.as_str()))
        .collect();
    if distinct.len() != owners.len() {
        return Err(reader.fail("invalid component owner set"));
    }
    Ok(owners)
}

fn read_module(reader: SaveReader) -> Result<UnifiedClientModule, WorldError> {
    use qa_world::save::shared::read_digest;
    Ok(UnifiedClientModule {
        id: namespaced(reader.field("id"))?,
        artifact_path: reader.field("artifactPath").string()?,
        digest: read_digest(reader.field("digest"))?,
        revision: reader.field("revision").string()?,
    })
}

fn read_client_command(reader: SaveReader) -> Result<UnifiedClientCommand, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "module" => Ok(UnifiedClientCommand::Module {
            module: read_module(reader.field("module"))?,
            command: reader.field("command").bytes()?,
        }),
        "movement" => {
            let angles = reader.field("angles");
            let triple = angles.list(|value| value.finite())?;
            if triple.len() != 3 {
                return Err(angles.fail("movement command requires three angles"));
            }
            Ok(UnifiedClientCommand::Movement {
                forward_milliseconds: reader.field("forwardMilliseconds").finite()?,
                side_milliseconds: reader.field("sideMilliseconds").finite()?,
                up_milliseconds: reader.field("upMilliseconds").finite()?,
                buttons: reader.field("buttons").finite()?,
                impulse: reader.field("impulse").finite()?,
                mouth: reader.field("mouth").finite()?,
                angles: (triple[0], triple[1], triple[2]),
            })
        }
        _ => {
            let intent = reader.field("intent");
            let weapon = intent.field("weapon");
            let impulse = intent.field("impulse");
            Ok(UnifiedClientCommand::Arsenal {
                intent: UnifiedArsenalIntent {
                    provider: namespaced(intent.field("provider"))?,
                    weapon: if weapon.value == Some(&SaveJson::Null) {
                        None
                    } else {
                        Some(namespaced(weapon)?)
                    },
                    use_holdable: intent.field("useHoldable").boolean()?,
                    impulse: if impulse.value.is_none() {
                        None
                    } else {
                        Some(impulse.finite()?)
                    },
                },
            })
        }
    }
}

fn read_resource_key(reader: SaveReader) -> Result<UnifiedResourceKey, WorldError> {
    use qa_content::contract::ContentId;
    use qa_world::save::shared::{read_content_id, read_digest};
    let path = reader.field("path").string()?;
    if path.is_empty() || path.contains('\0') {
        return Err(reader.fail("invalid unified resource path"));
    }
    Ok(UnifiedResourceKey {
        content: ContentId(read_content_id(reader.field("content"))?),
        path,
        digest: read_digest(reader.field("digest"))?,
        byte_length: reader.field("byteLength").integer(0).and_then(|length| {
            u64::try_from(length).map_err(|_| reader.field("byteLength").fail("resource length exceeds its range"))
        })?,
    })
}

fn write_resource_key(key: &UnifiedResourceKey) -> SaveJson {
    obj(vec![
        ("content", json_str(key.content.as_str())),
        ("path", json_str(&key.path)),
        ("digest", json_str(&key.digest)),
        ("byteLength", int(key.byte_length as i64)),
    ])
}

fn read_epoch(reader: &SaveReader) -> Result<u64, WorldError> {
    let epoch = reader.field("epoch").integer(1)?;
    u64::try_from(epoch).map_err(|_| reader.field("epoch").fail("control epoch exceeds its range"))
}

/// Read a control value (donor `readUnifiedControl`, shared by decode paths).
pub fn read_unified_control(reader: SaveReader) -> Result<UnifiedControl, UnifiedControlError> {
    match reader.field("kind").string()?.as_str() {
        "offer" => {
            let mode = reader
                .field("mode")
                .choice_str(&["singleplayer", "coop", "deathmatch"])?;
            Ok(UnifiedControl::Offer {
                epoch: read_epoch(&reader)?,
                composition: read_unified_composition(reader.field("composition"))?,
                mode: match mode.as_str() {
                    "singleplayer" => UnifiedServerMode::Singleplayer,
                    "coop" => UnifiedServerMode::Coop,
                    _ => UnifiedServerMode::Deathmatch,
                },
                max_clients: reader.field("maxClients").integer(1)?,
            })
        }
        "ready" => Ok(UnifiedControl::Ready {
            epoch: read_epoch(&reader)?,
            composition: {
                use qa_world::save::shared::read_digest;
                read_digest(reader.field("composition"))?
            },
            userinfo: reader.field("userinfo").string()?,
        }),
        "admitted" => Ok(UnifiedControl::Admitted {
            epoch: read_epoch(&reader)?,
            client: read_actor_reference(reader.field("client"))?,
            actor: read_actor_reference(reader.field("actor"))?,
            source_entity: reader.field("sourceEntity").integer(0)?,
        }),
        "resources" => Ok(UnifiedControl::Resources {
            epoch: read_epoch(&reader)?,
            resources: reader.field("resources").list(read_resource_key)?,
        }),
        "events" => Ok(UnifiedControl::Events {
            epoch: read_epoch(&reader)?,
            frame: reader.field("frame").integer(0)?,
            payload: reader.field("payload").bytes()?,
            simulation: reader.field("simulation").bytes()?,
        }),
        "components" => Ok(UnifiedControl::Components {
            epoch: read_epoch(&reader)?,
            update: read_component_update(reader.field("update"))?,
        }),
        _ => Err(reader.fail("unknown control variant").into()),
    }
}

/// Write a control value (donor `writeUnifiedControl`, shared by encode paths).
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
            ("client", write_actor_reference(*client)),
            ("actor", write_actor_reference(*actor)),
            ("sourceEntity", int(*source_entity)),
        ]),
        UnifiedControl::Resources { epoch, resources } => obj(vec![
            ("kind", json_str("resources")),
            ("epoch", int(*epoch as i64)),
            ("resources", arr(resources.iter().map(write_resource_key).collect())),
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
    }
}

/// Encode a control message (donor `encodeUnifiedControl`).
#[must_use]
pub fn encode_unified_control(control: &UnifiedControl) -> Vec<u8> {
    encode_checkpoint_value(&write_unified_control(control))
}

/// Decode a control message (donor `decodeUnifiedControl`).
pub fn decode_unified_control(bytes: &[u8]) -> Result<UnifiedControl, UnifiedControlError> {
    if bytes.len() > MAX_CONTROL_BYTES {
        return Err(UnifiedControlError::Control(
            "Unified control exceeds byte limit".to_string(),
        ));
    }
    let value = decode_checkpoint_value(bytes)?;
    read_unified_control(SaveReader::new(&value))
}

/// Read client input (donor `readUnifiedClientInput`, shared by decode paths).
pub fn read_unified_client_input(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<UnifiedClientInput, UnifiedControlError> {
    use super::unified_frame_values::read_actor;
    let command = reader.field("command");
    let bytes = command.field("command");
    if !bytes.is_missing() {
        let raw = bytes.bytes()?;
        if raw.len() > MAX_INPUT_BYTES {
            return Err(UnifiedControlError::Control(
                "Unified input exceeds byte limit".to_string(),
            ));
        }
    }
    Ok(UnifiedClientInput {
        client: read_actor(reader.field("client"), identity)?,
        acknowledged_frame: reader.field("acknowledgedFrame").integer(0)?,
        sequence: reader.field("sequence").integer(0)?,
        command: read_client_command(command)?,
        components: read_owner_set(reader.field("components"))?,
    })
}

/// Decode client input (donor `decodeUnifiedClientInput`).
pub fn decode_unified_client_input(
    bytes: &[u8],
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<UnifiedClientInput, UnifiedControlError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(UnifiedControlError::Control(
            "Unified input exceeds byte limit".to_string(),
        ));
    }
    let value = decode_checkpoint_value(bytes)?;
    read_unified_client_input(SaveReader::new(&value), identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{ClientId, IdentityOwner, SeatId, SessionId};
    use qa_world::save::value::num;

    use super::super::unified_types::UnifiedIdentityDecoder as Decoder;

    struct Ledger {
        owner: IdentityOwner,
    }

    impl Decoder for Ledger {
        fn session(&self) -> SessionId {
            self.owner.session().clone()
        }
        fn actor(&self, slot: u32, generation: u32) -> ActorId {
            self.owner.actor(slot, generation)
        }
        fn client(&self, slot: u32, generation: u32) -> ClientId {
            self.owner.client(slot, generation)
        }
        fn seat(&self, index: u32) -> SeatId {
            self.owner.seat(index)
        }
        fn resource_id(&self, id: &str) -> String {
            id.to_string()
        }
    }

    #[test]
    fn admitted_round_trips() {
        let control = UnifiedControl::Admitted {
            epoch: 3,
            client: UnifiedActorReference { slot: 1, generation: 0 },
            actor: UnifiedActorReference { slot: 2, generation: 0 },
            source_entity: 7,
        };
        let encoded = encode_unified_control(&control);
        assert_eq!(decode_unified_control(&encoded).unwrap(), control);
    }

    #[test]
    fn control_rejects_oversize_payload() {
        assert!(decode_unified_control(&vec![0u8; MAX_CONTROL_BYTES + 1]).is_err());
    }

    #[test]
    fn movement_input_round_trips() {
        let ledger = Ledger {
            owner: IdentityOwner::create("control").unwrap(),
        };
        let client = ledger.actor(1, 0);
        let value = obj(vec![
            ("client", obj(vec![("slot", int(1)), ("generation", int(0))])),
            ("acknowledgedFrame", int(4)),
            ("sequence", int(9)),
            (
                "command",
                obj(vec![
                    ("kind", json_str("movement")),
                    ("forwardMilliseconds", num(16.0)),
                    ("sideMilliseconds", num(0.0)),
                    ("upMilliseconds", num(0.0)),
                    ("buttons", num(0.0)),
                    ("impulse", num(0.0)),
                    ("mouth", num(0.0)),
                    ("angles", arr(vec![num(0.0), num(90.0), num(0.0)])),
                ]),
            ),
            ("components", arr(Vec::new())),
        ]);
        let input = read_unified_client_input(SaveReader::new(&value), &ledger).unwrap();
        assert_eq!(input.client, client);
        assert_eq!(input.sequence, 9);
        assert!(matches!(input.command, UnifiedClientCommand::Movement { .. }));
    }

    #[test]
    fn owner_set_rejects_duplicates() {
        let value = arr(vec![
            obj(vec![("provider", json_str("test:mod")), ("generation", int(1))]),
            obj(vec![("provider", json_str("test:mod")), ("generation", int(2))]),
        ]);
        let reader = SaveReader::new(&value);
        assert!(read_owner_set(reader).is_err());
    }

    #[test]
    fn module_command_enforces_input_limit() {
        let ledger = Ledger {
            owner: IdentityOwner::create("control").unwrap(),
        };
        let value = obj(vec![
            ("client", obj(vec![("slot", int(1)), ("generation", int(0))])),
            ("acknowledgedFrame", int(0)),
            ("sequence", int(0)),
            (
                "command",
                obj(vec![
                    ("kind", json_str("module")),
                    (
                        "module",
                        obj(vec![
                            ("id", json_str("test:mod")),
                            ("artifactPath", json_str("vm.qvm")),
                            ("digest", json_str("sha256:00")),
                            ("revision", json_str("1")),
                        ]),
                    ),
                    ("command", SaveJson::Bytes(vec![0u8; MAX_INPUT_BYTES + 1])),
                ]),
            ),
            ("components", arr(Vec::new())),
        ]);
        assert!(read_unified_client_input(SaveReader::new(&value), &ledger).is_err());
    }
}
