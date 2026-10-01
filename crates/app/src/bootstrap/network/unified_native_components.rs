//! Unified native component projection and codecs.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-native-components.ts`
//! (`projectNativeComponents`, `writeNativeStates`, `readNativeStates`,
//! `writeNativeFrames`, `readNativeFrames`).
//!
//! Native client imports are authored by the server module; the projection
//! diffs consecutive publications so reliable state only resends changed
//! HUD/configstrings, while every frame carries the volatile stats, server
//! frame, and camera. Protocol kinds reuse
//! [`ProtocolIdentity`](qa_net::protocol::ProtocolIdentity) through
//! [`q2_application_layout`](super::q2_layout::q2_application_layout) for the
//! configstring bounds; HUD shapes are local mirrors carrying exactly the
//! fields the projection compares and the codec reads.

use std::collections::BTreeMap;

use qa_content::contract::PresentationOwner;
use qa_core::identity::ActorId;
use qa_net::protocol::ProtocolIdentity;
use qa_world::save::value::{arr, int, num, obj, str as json_str, SaveJson, SaveReader};
use qa_world::WorldError;

use super::q2_layout::q2_application_layout;
use super::types::NativeModCameraView;
use super::unified_components::{read_component_owner, same_owner};
use super::unified_frame_values::{read_actor, read_native_camera_view, wire_actor, write_native_camera_view};
use super::unified_types::UnifiedIdentityDecoder;
use crate::persistence::mods::{read_mod_identity, write_mod_identity, ModIdentity};

/// Native HUD mode (donor `NativeFrame['hud']['mode']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedNativeHudMode {
    /// Layout overlay (`layout-overlay`).
    LayoutOverlay,
    /// Replace status (`replace-status`).
    ReplaceStatus,
}

impl UnifiedNativeHudMode {
    fn text(self) -> &'static str {
        match self {
            Self::LayoutOverlay => "layout-overlay",
            Self::ReplaceStatus => "replace-status",
        }
    }
}

/// Native Q2 protocol (donor `Q2ProtocolIdentity` subset: kind + version).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedNativeProtocol {
    /// Classic protocol 34.
    Classic,
    /// Rerelease protocol 1038.
    Rerelease,
}

impl UnifiedNativeProtocol {
    fn identity(self) -> ProtocolIdentity {
        match self {
            Self::Classic => ProtocolIdentity::Q2Classic,
            Self::Rerelease => ProtocolIdentity::Q2Rerelease,
        }
    }
}

/// Reliable native HUD state (donor `HudState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedNativeHudState {
    /// Source protocol.
    pub protocol: UnifiedNativeProtocol,
    /// Configstrings, or [`None`] when unchanged from the previous revision.
    pub configstrings: Option<BTreeMap<i64, String>>,
    /// Layout program.
    pub layout: String,
    /// Inventory counts.
    pub inventory: Vec<i64>,
    /// Zero-based player slot.
    pub player_number: i64,
}

/// Reliable native HUD block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedNativeHudBlock {
    /// HUD mode.
    pub mode: UnifiedNativeHudMode,
    /// HUD state.
    pub frame: UnifiedNativeHudState,
}

/// Reliable native component state (donor `UnifiedNativeState`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedNativeState {
    /// Presenting owner.
    pub owner: PresentationOwner,
    /// Component identity.
    pub identity: ModIdentity,
    /// Activation generation.
    pub generation: i64,
    /// HUD block.
    pub hud: Option<UnifiedNativeHudBlock>,
}

/// Volatile native HUD frame (donor `UnifiedNativeFrame['hud']`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedNativeHudFrame {
    /// Playerstate stats (32 classic / 64 rerelease).
    pub stats: Vec<i64>,
    /// Server frame.
    pub server_frame: i64,
    /// Client time in milliseconds.
    pub time_milliseconds: f64,
    /// Last frame duration in milliseconds, if measured.
    pub frame_time_milliseconds: Option<f64>,
}

/// Native component frame (donor `UnifiedNativeFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedNativeFrame {
    /// Presenting owner.
    pub owner: PresentationOwner,
    /// Activation generation.
    pub generation: i64,
    /// Viewing actor.
    pub viewer: ActorId,
    /// Volatile HUD frame.
    pub hud: Option<UnifiedNativeHudFrame>,
    /// Camera view.
    pub view: Option<NativeModCameraView>,
}

/// Published native HUD (reliable + volatile halves together).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedNativePublicationHud {
    /// HUD mode.
    pub mode: UnifiedNativeHudMode,
    /// Reliable HUD state.
    pub state: UnifiedNativeHudState,
    /// Volatile HUD frame.
    pub frame: UnifiedNativeHudFrame,
}

/// Published native frame (donor `NativeFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedNativePublicationFrame {
    /// HUD publication.
    pub hud: Option<UnifiedNativePublicationHud>,
    /// Camera view.
    pub view: Option<NativeModCameraView>,
}

/// Published native component source (donor `UnifiedNativePublication`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedNativePublication {
    /// Presenting owner.
    pub owner: PresentationOwner,
    /// Component identity.
    pub identity: ModIdentity,
    /// Activation generation.
    pub generation: i64,
    /// Viewing actor.
    pub viewer: ActorId,
    /// Published frame.
    pub frame: UnifiedNativePublicationFrame,
}

/// Projection of native publications (donor `projectNativeComponents` output).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedNativeProjection {
    /// Whether reliable state changed.
    pub changed: bool,
    /// Reliable states.
    pub states: Vec<UnifiedNativeState>,
    /// Volatile frames.
    pub frames: Vec<UnifiedNativeFrame>,
}

/// Native projection failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnifiedNativeProjectionError {
    /// Activation changed identity or recipient.
    #[error("Native component activation changed identity or recipient")]
    IdentityChanged,
}

/// Project native publications onto reliable states and volatile frames.
///
/// The projection fails when an activation changed its identity or
/// recipient; unchanged configstrings collapse to [`None`] so reliable
/// state only resends what moved.
pub fn project_native_components(
    previous: &[UnifiedNativePublication],
    sources: &[UnifiedNativePublication],
) -> Result<UnifiedNativeProjection, UnifiedNativeProjectionError> {
    let mut changed = sources.len() != previous.len();
    let mut states = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        let old = previous.iter().find(|old| {
            old.owner.provider.namespace == source.owner.provider.namespace
                && old.owner.provider.name == source.owner.provider.name
        });
        let same = matches!(old, Some(old)
            if same_owner(&source.owner, &old.owner) && source.generation == old.generation);
        if same {
            let old = old.expect("same implies old");
            if source.identity != old.identity || source.viewer != old.viewer {
                return Err(UnifiedNativeProjectionError::IdentityChanged);
            }
        }
        let hud = source.frame.hud.as_ref();
        let old_hud = if same {
            old.expect("same implies old").frame.hud.as_ref()
        } else {
            None
        };
        let same_config = match (hud, old_hud) {
            (Some(hud), Some(old_hud)) => hud
                .state
                .configstrings
                .as_ref()
                .zip(old_hud.state.configstrings.as_ref())
                .is_some_and(|(left, right)| left == right),
            _ => false,
        };
        let same_hud = match (hud, old_hud) {
            (None, None) => true,
            (Some(hud), Some(old_hud)) => {
                same_config
                    && hud.mode == old_hud.mode
                    && hud.state.protocol == old_hud.state.protocol
                    && hud.state.layout == old_hud.state.layout
                    && hud.state.player_number == old_hud.state.player_number
                    && hud.state.inventory == old_hud.state.inventory
            }
            _ => false,
        };
        let order_same = previous
            .get(index)
            .is_some_and(|old| same_owner(&old.owner, &source.owner));
        changed = changed || !same || !same_hud || !order_same;
        states.push(UnifiedNativeState {
            owner: source.owner.clone(),
            identity: source.identity.clone(),
            generation: source.generation,
            hud: hud.map(|hud| UnifiedNativeHudBlock {
                mode: hud.mode,
                frame: UnifiedNativeHudState {
                    protocol: hud.state.protocol,
                    configstrings: if same_config {
                        None
                    } else {
                        hud.state.configstrings.clone()
                    },
                    layout: hud.state.layout.clone(),
                    inventory: hud.state.inventory.clone(),
                    player_number: hud.state.player_number,
                },
            }),
        });
    }
    let frames = sources
        .iter()
        .map(|source| UnifiedNativeFrame {
            owner: source.owner.clone(),
            generation: source.generation,
            viewer: source.viewer.clone(),
            view: source.frame.view.clone(),
            hud: source.frame.hud.as_ref().map(|hud| hud.frame.clone()),
        })
        .collect();
    Ok(UnifiedNativeProjection {
        changed,
        states,
        frames,
    })
}

fn integer(reader: SaveReader, minimum: i64, maximum: i64) -> Result<i64, WorldError> {
    let value = reader.integer(minimum)?;
    if value <= maximum {
        Ok(value)
    } else {
        Err(reader.fail("native component integer exceeds its range"))
    }
}

fn text(reader: SaveReader, maximum: usize) -> Result<String, WorldError> {
    let value = reader.string()?;
    if value.len() <= maximum && !value.contains('\0') {
        Ok(value)
    } else {
        Err(reader.fail("invalid native component string"))
    }
}

fn read_protocol(reader: SaveReader) -> Result<UnifiedNativeProtocol, WorldError> {
    let kind = reader.field("kind").choice_str(&["q2-classic", "q2-rerelease"])?;
    if kind == "q2-classic" {
        reader.field("version").literal_i64(34)?;
        Ok(UnifiedNativeProtocol::Classic)
    } else {
        reader.field("version").literal_i64(1038)?;
        Ok(UnifiedNativeProtocol::Rerelease)
    }
}

fn write_protocol(protocol: UnifiedNativeProtocol) -> SaveJson {
    match protocol {
        UnifiedNativeProtocol::Classic => obj(vec![("kind", json_str("q2-classic")), ("version", int(34))]),
        UnifiedNativeProtocol::Rerelease => obj(vec![("kind", json_str("q2-rerelease")), ("version", int(1038))]),
    }
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

/// Encode native states (donor `writeNativeStates`).
#[must_use]
pub fn write_native_states(sources: &[UnifiedNativeState]) -> SaveJson {
    arr(sources
        .iter()
        .map(|source| {
            let hud = source.hud.as_ref().map_or(SaveJson::Null, |hud| {
                let configstrings = hud.frame.configstrings.as_ref().map_or(SaveJson::Null, |configs| {
                    arr(configs
                        .iter()
                        .map(|(index, value)| obj(vec![("index", int(*index)), ("value", json_str(value))]))
                        .collect())
                });
                obj(vec![
                    ("mode", json_str(hud.mode.text())),
                    (
                        "frame",
                        obj(vec![
                            ("protocol", write_protocol(hud.frame.protocol)),
                            ("configstrings", configstrings),
                            ("layout", json_str(&hud.frame.layout)),
                            (
                                "inventory",
                                arr(hud.frame.inventory.iter().map(|slot| int(*slot)).collect()),
                            ),
                            ("playerNumber", int(hud.frame.player_number)),
                        ]),
                    ),
                ])
            });
            obj(vec![
                ("owner", write_owner(&source.owner)),
                ("identity", write_mod_identity(&source.identity)),
                ("generation", int(source.generation)),
                ("hud", hud),
            ])
        })
        .collect())
}

/// Decode native states (donor `readNativeStates`).
pub fn read_native_states(reader: SaveReader) -> Result<Vec<UnifiedNativeState>, WorldError> {
    if reader.value.is_none() {
        return Ok(Vec::new());
    }
    reader.list(|source| {
        let hud = source.field("hud").nullable(|hud| {
            let frame = hud.field("frame");
            let protocol = read_protocol(frame.field("protocol"))?;
            let layout = q2_application_layout(protocol.identity())
                .map_err(|_| frame.fail("native layout requires a Quake II protocol"))?;
            let configstrings = frame.field("configstrings").nullable(|configs| {
                let mut seen = std::collections::HashSet::new();
                let mut result = BTreeMap::new();
                for entry in configs.list(Ok)? {
                    let index = integer(entry.field("index"), 0, i64::from(layout.max_config_strings) - 1)?;
                    let value = text(entry.field("value"), 65535)?;
                    if !seen.insert(index) {
                        return Err(configs.fail("duplicate native configstrings"));
                    }
                    result.insert(index, value);
                }
                Ok(result)
            })?;
            let inventory = frame.field("inventory").list(|value| integer(value, -32768, 32767))?;
            if inventory.len() > 256 {
                return Err(frame.fail("native inventory exceeds source slots"));
            }
            let mode = hud.field("mode").choice_str(&["layout-overlay", "replace-status"])?;
            Ok(UnifiedNativeHudBlock {
                mode: if mode == "layout-overlay" {
                    UnifiedNativeHudMode::LayoutOverlay
                } else {
                    UnifiedNativeHudMode::ReplaceStatus
                },
                frame: UnifiedNativeHudState {
                    protocol,
                    configstrings,
                    layout: text(frame.field("layout"), 65535)?,
                    inventory,
                    player_number: integer(frame.field("playerNumber"), 0, 255)?,
                },
            })
        })?;
        Ok(UnifiedNativeState {
            owner: read_component_owner(source.field("owner"))?,
            identity: read_mod_identity(source.field("identity"))
                .map_err(|error| source.field("identity").fail(&error.to_string()))?,
            generation: integer(source.field("generation"), 0, i64::MAX)?,
            hud,
        })
    })
}

/// Encode native frames (donor `writeNativeFrames`).
#[must_use]
pub fn write_native_frames(sources: &[UnifiedNativeFrame]) -> SaveJson {
    arr(sources
        .iter()
        .map(|source| {
            obj(vec![
                ("owner", write_owner(&source.owner)),
                ("generation", int(source.generation)),
                ("viewer", wire_actor(&source.viewer)),
                (
                    "view",
                    source.view.as_ref().map_or(SaveJson::Null, write_native_camera_view),
                ),
                (
                    "hud",
                    source.hud.as_ref().map_or(SaveJson::Null, |hud| {
                        let mut hud_members = vec![
                            ("stats", arr(hud.stats.iter().map(|stat| int(*stat)).collect())),
                            ("serverFrame", int(hud.server_frame)),
                            ("timeMilliseconds", num(hud.time_milliseconds)),
                        ];
                        if let Some(frame_time) = hud.frame_time_milliseconds {
                            hud_members.push(("frameTimeMilliseconds", num(frame_time)));
                        }
                        obj(hud_members)
                    }),
                ),
            ])
        })
        .collect())
}

/// Decode native frames (donor `readNativeFrames`).
pub fn read_native_frames(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Vec<UnifiedNativeFrame>, WorldError> {
    if reader.value.is_none() {
        return Ok(Vec::new());
    }
    reader.list(|source| {
        Ok(UnifiedNativeFrame {
            owner: read_component_owner(source.field("owner"))?,
            generation: integer(source.field("generation"), 0, i64::MAX)?,
            viewer: read_actor(source.field("viewer"), identity)?,
            view: source.field("view").nullable(read_native_camera_view)?,
            hud: source.field("hud").nullable(|hud| {
                let stats = hud.field("stats").list(|value| integer(value, -32768, 32767))?;
                if stats.len() != 32 && stats.len() != 64 {
                    return Err(hud.fail("invalid source native stat count"));
                }
                let frame_time = hud.field("frameTimeMilliseconds");
                let frame_time_milliseconds = if frame_time.value.is_none() {
                    None
                } else {
                    Some(frame_time.finite()?)
                };
                if frame_time_milliseconds.is_some_and(|value| value <= 0.0) {
                    return Err(hud.fail("native source frame interval must be positive"));
                }
                Ok(UnifiedNativeHudFrame {
                    stats,
                    server_frame: integer(hud.field("serverFrame"), 0, i64::from(i32::MAX))?,
                    time_milliseconds: hud.field("timeMilliseconds").finite()?,
                    frame_time_milliseconds,
                })
            })?,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{ClientId, IdentityOwner, ProviderId, SeatId, SessionId};
    use qa_world::save::shared::ProviderRef;

    use super::super::unified_types::UnifiedIdentityDecoder as Decoder;
    use crate::persistence::mods::ModSelection;

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

    fn ledger() -> Ledger {
        Ledger {
            owner: IdentityOwner::create("native").unwrap(),
        }
    }

    fn identity() -> ModIdentity {
        ModIdentity {
            selection: ModSelection {
                product: "test".to_string(),
                id: "native".to_string(),
            },
            source: provider_ref(),
            declaration_digest: "sha256:0".to_string(),
            modules: Vec::new(),
            providers: Vec::new(),
        }
    }

    fn selection_check() -> ModSelection {
        identity().selection.clone()
    }

    fn provider_ref() -> ProviderRef {
        ProviderRef {
            provider: "test:mod".to_string(),
            content: "q2:classic:base:1".to_string(),
        }
    }

    #[test]
    fn projection_reports_first_publish_as_changed() {
        let viewer = ledger().actor(1, 0);
        let publication = UnifiedNativePublication {
            owner: PresentationOwner {
                provider: ProviderId::new("test", "native"),
                generation: 1,
            },
            identity: identity(),
            generation: 1,
            viewer,
            frame: UnifiedNativePublicationFrame { hud: None, view: None },
        };
        let projection = project_native_components(&[], &[publication]).unwrap();
        assert!(projection.changed);
        assert_eq!(projection.states.len(), 1);
        assert_eq!(projection.frames.len(), 1);
        assert_eq!(selection_check().product, identity().selection.product);
        assert_eq!(provider_ref().provider, "test:mod");
    }

    #[test]
    fn hud_stats_must_match_source_abi() {
        let ledger = ledger();
        let encoded = arr(vec![obj(vec![
            (
                "owner",
                obj(vec![("provider", json_str("test:native")), ("generation", int(1))]),
            ),
            ("generation", int(1)),
            ("viewer", obj(vec![("slot", int(1)), ("generation", int(0))])),
            ("view", SaveJson::Null),
            (
                "hud",
                obj(vec![
                    ("stats", arr(vec![int(0); 33])),
                    ("serverFrame", int(1)),
                    ("timeMilliseconds", num(16.0)),
                ]),
            ),
        ])]);
        let reader = SaveReader::new(&encoded);
        assert!(read_native_frames(reader, &ledger).is_err());
    }
}
