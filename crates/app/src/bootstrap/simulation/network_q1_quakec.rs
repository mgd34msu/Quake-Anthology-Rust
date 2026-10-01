//! NetQuake server host over the NetQuake QuakeC source.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/network-q1-quakec.ts`.
//!
//! Borrow split: the donor reads the game through `simulation.quakecSource()`
//! and then calls mutating methods on both handles, which Rust cannot alias.
//! The options therefore carry the game handle alongside the simulation as
//! [`Q1QuakeCServerOptions::game`]; [`None`] mirrors a null source and the
//! [`Q1QuakeCNetGame::kind`] check preserves the donor's combined
//! null-or-wrong-ABI rejection. The async content fetch is injected behind
//! the synchronous [`Q1QuakeCHostContent`] seam; bootstrap ports are sync.
//! `codec.maxPrecache` (donor `src/network/q1/profile.ts`) arrives as
//! [`Q1QuakeCServerOptions::max_precache`] because the `qa-net` precache
//! limit stays private to its codec.

use std::collections::HashMap;
use std::rc::Rc;

use qa_content::contract::{ContentId, ResolvedResourceReference};
use qa_content::mounts::{MountError, MountedContent};
use qa_content::q1::foundation::types::Q1Event;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::{ActorId, ClientId, OwnedActor};
use qa_core::math::Vec3;
use qa_guest::qc::presentation_host::QcMessageDestination;
use qa_net::common::commands::{ActorCommand, CommandSource, UserCommand};
use qa_net::common::endpoint::NetworkAddress;
use qa_net::common::session::WireAdmission;
use qa_net::protocol::q1::ENTALPHA_ZERO;
use qa_net::q1_wide::{entalpha_encode, entscale_encode, NqProfile};
use qa_world::movement::types::Q1UserCommand;
use qa_world::session::SimulationOutput;
use qa_world::WorldError;
use thiserror::Error;

use super::types::{PlayerAdmission, PlayerView, SimulationPresentationEvent, SourcePresentationEvent};

/// Mirror of the donor `QuakeCSource["kind"]` from
/// `src/app/bootstrap/simulation/quakec-source.ts` (canonical home:
/// `crate::bootstrap::simulation::quakec_source`); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuakeCSourceKind {
    /// NetQuake ABI.
    Netquake,
    /// QuakeWorld ABI.
    Quakeworld,
}

impl QuakeCSourceKind {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            QuakeCSourceKind::Netquake => "netquake",
            QuakeCSourceKind::Quakeworld => "quakeworld",
        }
    }
}

/// Mirror of the donor `precacheNames` argument from
/// `src/app/bootstrap/simulation/quakec-source.ts` (canonical home:
/// `crate::bootstrap::simulation::quakec_source`); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuakeCPrecacheKind {
    /// Model precaches.
    Model,
    /// Sound precaches.
    Sound,
}

/// Mirror of `Q1ExtendedEntityState` from donor `src/contracts/protocol.ts`
/// (canonical home: `qa_net::q1` app surface); unify post-merge.
///
/// Alpha and scale are the encoded wire bytes the donor threads through as
/// numbers; the remaining fields keep donor precision at hub `f32` vectors.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ExtendedEntityState {
    /// Entity number.
    pub number: i32,
    /// Origin.
    pub origin: Vec3,
    /// Angles in degrees.
    pub angles: Vec3,
    /// Model index.
    pub model_index: i32,
    /// Frame.
    pub frame: i32,
    /// Color map.
    pub color_map: i32,
    /// Skin.
    pub skin: i32,
    /// Effects bitmask.
    pub effects: i32,
    /// Encoded alpha byte.
    pub alpha: u8,
    /// Encoded scale byte.
    pub scale: u8,
    /// Lerp finish, always zero from this host.
    pub lerp_finish_seconds: f64,
    /// Step animation flag.
    pub step: bool,
}

/// Mirror of `Q1ClientData` from donor `src/contracts/protocol.ts`
/// (canonical home: `qa_net::q1` app surface); unify post-merge.
///
/// This is the float application shape the host builds, not the decoded
/// integer [`qa_net::q1_net::NetQuakeClientData`] the wire codec owns.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ClientData {
    /// View height.
    pub view_height: f64,
    /// Ideal pitch.
    pub ideal_pitch: f64,
    /// Punch angles.
    pub punch_angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Item bits.
    pub items: i32,
    /// On ground.
    pub on_ground: bool,
    /// In water.
    pub in_water: bool,
    /// Weapon frame.
    pub weapon_frame: f64,
    /// Armor value.
    pub armor: f64,
    /// Weapon model index.
    pub weapon_model: i32,
    /// Health.
    pub health: f64,
    /// Current ammo.
    pub ammo: f64,
    /// Shells.
    pub shells: f64,
    /// Nails.
    pub nails: f64,
    /// Rockets.
    pub rockets: f64,
    /// Cells.
    pub cells: f64,
    /// Active weapon.
    pub active_weapon: i32,
}

/// Unit NetQuake message kinds, mirroring the donor `'nop' | ...` union.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1UnitMessageKind {
    /// No-op.
    Nop,
    /// Disconnect.
    Disconnect,
    /// Killed monster.
    KilledMonster,
    /// Found secret.
    FoundSecret,
    /// Intermission.
    Intermission,
    /// Sell screen.
    SellScreen,
    /// Bonus flash.
    BonusFlash,
    /// Level completed.
    LevelCompleted,
    /// Back to lobby.
    BackToLobby,
}

impl Q1UnitMessageKind {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1UnitMessageKind::Nop => "nop",
            Q1UnitMessageKind::Disconnect => "disconnect",
            Q1UnitMessageKind::KilledMonster => "killed-monster",
            Q1UnitMessageKind::FoundSecret => "found-secret",
            Q1UnitMessageKind::Intermission => "intermission",
            Q1UnitMessageKind::SellScreen => "sell-screen",
            Q1UnitMessageKind::BonusFlash => "bonus-flash",
            Q1UnitMessageKind::LevelCompleted => "level-completed",
            Q1UnitMessageKind::BackToLobby => "back-to-lobby",
        }
    }
}

/// Text NetQuake message kinds, mirroring the donor `'print' | ...` union.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1TextMessageKind {
    /// Print.
    Print,
    /// Center print.
    CenterPrint,
    /// Stuff text.
    Stufftext,
    /// Finale.
    Finale,
    /// Cutscene.
    Cutscene,
    /// Skybox.
    Skybox,
    /// Achievement.
    Achievement,
    /// Bot chat.
    Botchat,
    /// Raw print.
    RawPrint,
    /// Chat.
    Chat,
    /// Server vars.
    ServerVars,
}

impl Q1TextMessageKind {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1TextMessageKind::Print => "print",
            Q1TextMessageKind::CenterPrint => "center-print",
            Q1TextMessageKind::Stufftext => "stufftext",
            Q1TextMessageKind::Finale => "finale",
            Q1TextMessageKind::Cutscene => "cutscene",
            Q1TextMessageKind::Skybox => "skybox",
            Q1TextMessageKind::Achievement => "achievement",
            Q1TextMessageKind::Botchat => "botchat",
            Q1TextMessageKind::RawPrint => "raw-print",
            Q1TextMessageKind::Chat => "chat",
            Q1TextMessageKind::ServerVars => "server-vars",
        }
    }
}

/// Named-slot message kinds, mirroring `'name' | 'social' | 'player-info'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1NamedSlotKind {
    /// Player name.
    Name,
    /// Social identity.
    Social,
    /// Player info.
    PlayerInfo,
}

impl Q1NamedSlotKind {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1NamedSlotKind::Name => "name",
            Q1NamedSlotKind::Social => "social",
            Q1NamedSlotKind::PlayerInfo => "player-info",
        }
    }
}

/// Numbered-slot message kinds, mirroring `'frags' | 'colors' | 'ping'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1NumberedSlotKind {
    /// Frags.
    Frags,
    /// Colors.
    Colors,
    /// Ping.
    Ping,
}

impl Q1NumberedSlotKind {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1NumberedSlotKind::Frags => "frags",
            Q1NumberedSlotKind::Colors => "colors",
            Q1NumberedSlotKind::Ping => "ping",
        }
    }
}

/// Sound message kinds, mirroring `'sound' | 'static-sound'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1SoundKind {
    /// Positioned sound.
    Sound,
    /// Static sound.
    StaticSound,
}

impl Q1SoundKind {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1SoundKind::Sound => "sound",
            Q1SoundKind::StaticSound => "static-sound",
        }
    }
}

/// Valued private message kinds, mirroring
/// `'spawned-monster' | 'set-views' | 'sequence'`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1ValuedKind {
    /// Spawned monster.
    SpawnedMonster,
    /// Set views.
    SetViews,
    /// Sequence.
    Sequence,
}

impl Q1ValuedKind {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1ValuedKind::SpawnedMonster => "spawned-monster",
            Q1ValuedKind::SetViews => "set-views",
            Q1ValuedKind::Sequence => "sequence",
        }
    }
}

/// Mirror of `TemporaryEntity` from donor `src/network/q1/netquake.ts`
/// (canonical home: `qa_net::q1_net`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1TemporaryEntity {
    /// Beam between entities or points.
    Beam {
        /// Effect type.
        effect_type: i32,
        /// Beam entity.
        entity: i32,
        /// Beam start.
        start: Vec3,
        /// Beam end.
        end: Vec3,
    },
    /// Explosion with explicit colors.
    ExplosionColors {
        /// Effect type.
        effect_type: i32,
        /// Explosion origin.
        origin: Vec3,
        /// First particle color.
        color_start: i32,
        /// Particle color run length.
        color_length: i32,
    },
    /// Point effect.
    Point {
        /// Effect type.
        effect_type: i32,
        /// Effect origin.
        origin: Vec3,
        /// Particle count.
        count: i32,
    },
}

/// Mirror of the donor `server-info` message payload, shared by the message
/// union and [`Q1ApplicationGameState::info`].
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ServerInfo {
    /// Protocol identity.
    pub protocol: NqProfile,
    /// Maximum clients.
    pub max_clients: u32,
    /// Game type.
    pub game_type: i32,
    /// Level name.
    pub level: String,
    /// Model precaches in signon order.
    pub models: Vec<String>,
    /// Sound precaches in signon order.
    pub sounds: Vec<String>,
}

/// Mirror of `NetQuakeMessage` from donor `src/network/q1/netquake.ts`
/// (canonical home: `qa_net::q1_net` app surface); unify post-merge.
///
/// The application union keeps donor float shapes; the wire codec owns the
/// integer [`qa_net::q1_net::NetQuakeMessage`] and converts on encode.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1NetQuakeMessage {
    /// Unit message.
    Unit(Q1UnitMessageKind),
    /// Text message.
    Text {
        /// Text kind.
        kind: Q1TextMessageKind,
        /// Text.
        text: String,
    },
    /// Server time.
    Time {
        /// Seconds.
        seconds: f64,
    },
    /// Protocol version.
    Version {
        /// Version.
        version: i32,
    },
    /// Player stat.
    Stat {
        /// Stat index.
        index: i32,
        /// Stat value.
        value: f64,
    },
    /// Set view entity.
    SetView {
        /// Entity number.
        entity: i32,
    },
    /// Set view angles.
    SetAngle {
        /// Angles.
        angles: Vec3,
    },
    /// Server info.
    ServerInfo(Q1ServerInfo),
    /// Light style.
    LightStyle {
        /// Style slot.
        index: i32,
        /// Light pattern.
        value: String,
    },
    /// Named client slot.
    NamedSlot {
        /// Slot kind.
        kind: Q1NamedSlotKind,
        /// Client slot.
        slot: i32,
        /// Value.
        value: String,
    },
    /// Numbered client slot.
    NumberedSlot {
        /// Slot kind.
        kind: Q1NumberedSlotKind,
        /// Client slot.
        slot: i32,
        /// Value.
        value: f64,
    },
    /// Client data.
    ClientData {
        /// Client state.
        data: Q1ClientData,
        /// Encoded weapon alpha byte.
        weapon_alpha: u8,
    },
    /// Entity update; the host rejects these from QuakeC.
    Entity {
        /// Entity state.
        state: Q1ExtendedEntityState,
    },
    /// Entity baseline.
    Baseline {
        /// Entity state.
        state: Q1ExtendedEntityState,
    },
    /// Static entity.
    Static {
        /// Entity state.
        state: Q1ExtendedEntityState,
    },
    /// Sound.
    Sound {
        /// Sound kind.
        kind: Q1SoundKind,
        /// Entity number.
        entity: i32,
        /// Channel.
        channel: i32,
        /// Sound index.
        index: i32,
        /// Volume byte.
        volume: i32,
        /// Attenuation.
        attenuation: f64,
        /// Origin.
        origin: Vec3,
    },
    /// Stop sound.
    StopSound {
        /// Entity number.
        entity: i32,
        /// Channel.
        channel: i32,
    },
    /// Local sound.
    LocalSound {
        /// Sound index.
        index: i32,
    },
    /// Damage feedback.
    Damage {
        /// Armor damage.
        armor: f64,
        /// Blood damage.
        blood: f64,
        /// Damage source.
        source: Vec3,
    },
    /// Particle burst.
    Particle {
        /// Origin.
        origin: Vec3,
        /// Direction.
        direction: Vec3,
        /// Particle count.
        count: i32,
        /// Particle color.
        color: i32,
    },
    /// Temporary entity.
    TemporaryEntity {
        /// Effect.
        effect: Q1TemporaryEntity,
    },
    /// Pause flag.
    Pause {
        /// Paused.
        paused: bool,
    },
    /// Signon stage.
    Signon {
        /// Stage.
        stage: i32,
    },
    /// CD track.
    CdTrack {
        /// Track.
        track: i32,
        /// Loop track.
        loop_track: i32,
    },
    /// Fog parameters.
    Fog {
        /// Density.
        density: f64,
        /// Color.
        color: Vec3,
        /// Transition seconds.
        transition_seconds: f64,
    },
    /// Valued private message.
    Valued {
        /// Value kind.
        kind: Q1ValuedKind,
        /// Value.
        value: f64,
    },
    /// Prompt begin.
    PromptBegin {
        /// Text.
        text: String,
        /// Choice count.
        choices: i32,
    },
    /// Prompt choice.
    PromptChoice {
        /// Text.
        text: String,
        /// Impulse.
        impulse: i32,
    },
    /// Prompt clear.
    PromptClear,
}

impl Q1NetQuakeMessage {
    /// Donor `kind` discriminant.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Q1NetQuakeMessage::Unit(kind) => kind.as_str(),
            Q1NetQuakeMessage::Text { kind, .. } => kind.as_str(),
            Q1NetQuakeMessage::Time { .. } => "time",
            Q1NetQuakeMessage::Version { .. } => "version",
            Q1NetQuakeMessage::Stat { .. } => "stat",
            Q1NetQuakeMessage::SetView { .. } => "set-view",
            Q1NetQuakeMessage::SetAngle { .. } => "set-angle",
            Q1NetQuakeMessage::ServerInfo(_) => "server-info",
            Q1NetQuakeMessage::LightStyle { .. } => "light-style",
            Q1NetQuakeMessage::NamedSlot { kind, .. } => kind.as_str(),
            Q1NetQuakeMessage::NumberedSlot { kind, .. } => kind.as_str(),
            Q1NetQuakeMessage::ClientData { .. } => "client-data",
            Q1NetQuakeMessage::Entity { .. } => "entity",
            Q1NetQuakeMessage::Baseline { .. } => "baseline",
            Q1NetQuakeMessage::Static { .. } => "static",
            Q1NetQuakeMessage::Sound { kind, .. } => kind.as_str(),
            Q1NetQuakeMessage::StopSound { .. } => "stop-sound",
            Q1NetQuakeMessage::LocalSound { .. } => "local-sound",
            Q1NetQuakeMessage::Damage { .. } => "damage",
            Q1NetQuakeMessage::Particle { .. } => "particle",
            Q1NetQuakeMessage::TemporaryEntity { .. } => "temporary-entity",
            Q1NetQuakeMessage::Pause { .. } => "pause",
            Q1NetQuakeMessage::Signon { .. } => "signon",
            Q1NetQuakeMessage::CdTrack { .. } => "cd-track",
            Q1NetQuakeMessage::Fog { .. } => "fog",
            Q1NetQuakeMessage::Valued { kind, .. } => kind.as_str(),
            Q1NetQuakeMessage::PromptBegin { .. } => "prompt-begin",
            Q1NetQuakeMessage::PromptChoice { .. } => "prompt-choice",
            Q1NetQuakeMessage::PromptClear => "prompt-clear",
        }
    }
}

/// Mirror of `Q1ApplicationMessage` from donor
/// `src/app/bootstrap/network/q1-types.ts` (canonical home:
/// `crate::bootstrap::network::q1_types`); unify post-merge.
///
/// The donor spells this `Exclude<NetQuakeMessage, { kind: 'entity' }>`;
/// the host enforces the exclusion by construction: it never builds
/// [`Q1NetQuakeMessage::Entity`] and routing rejects entity messages from
/// QuakeC with the donor errors.
pub type Q1ApplicationMessage = Q1NetQuakeMessage;

/// Mirror of `Q1ApplicationPlayer` from donor
/// `src/app/bootstrap/network/q1-types.ts` (canonical home:
/// `crate::bootstrap::network::q1_types`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Q1ApplicationPlayer {
    /// Client handle.
    pub client: ClientId,
    /// Player actor.
    pub actor: ActorId,
    /// Source entity slot, supplied by the source registry.
    pub source_entity: i32,
}

/// Mirror of the donor `admit` admission union from
/// `src/app/bootstrap/network/q1-types.ts` (canonical home:
/// `crate::bootstrap::network::q1_types`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1ApplicationAdmission {
    /// Client accepted.
    Accepted {
        /// Admitted player.
        player: Q1ApplicationPlayer,
    },
    /// Client rejected.
    Rejected {
        /// Reason.
        reason: String,
    },
}

/// Mirror of `Q1ApplicationGameState` from donor
/// `src/app/bootstrap/network/q1-types.ts` (canonical home:
/// `crate::bootstrap::network::q1_types`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ApplicationGameState {
    /// Server info.
    pub info: Q1ServerInfo,
    /// Entity baselines by entity number.
    pub baselines: HashMap<i32, Q1ExtendedEntityState>,
    /// Signon messages.
    pub signon: Vec<Q1ApplicationMessage>,
}

/// Mirror of the donor `frame` result from
/// `src/app/bootstrap/network/q1-types.ts` (canonical home:
/// `crate::bootstrap::network::q1_types`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ApplicationFrame {
    /// Server time in seconds.
    pub seconds: f64,
    /// Unreliable messages.
    pub messages: Vec<Q1ApplicationMessage>,
    /// Reliable messages.
    pub reliable: Vec<Q1ApplicationMessage>,
    /// Datagram messages.
    pub datagram: Vec<Q1ApplicationMessage>,
    /// Visible entities.
    pub entities: Vec<Q1ExtendedEntityState>,
}

/// Mirror of the donor `consumeNetQuakeDamage` result from
/// `src/app/bootstrap/simulation/quakec-source.ts` (canonical home:
/// `crate::bootstrap::simulation::quakec_source`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1NetDamage {
    /// Armor damage.
    pub armor: f64,
    /// Blood damage.
    pub blood: f64,
    /// Damage source.
    pub source: Vec3,
}

/// Mirror of the donor `drainNetQuakeMessages` entries from
/// `src/app/bootstrap/simulation/quakec-source.ts` (canonical home:
/// `crate::bootstrap::simulation::quakec_source`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1RoutedBatch {
    /// Routed messages.
    pub messages: Vec<Q1NetQuakeMessage>,
    /// Batch destination.
    pub destination: QcMessageDestination,
}

/// Mirror of `ModClientEvent["kind"]` from donor
/// `src/world/session/mod-clients.ts` (canonical home:
/// `qa_guest::qc::mod_clients`); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModClientEventKind {
    /// Client admitted.
    Admitted,
    /// Client userinfo changed.
    Userinfo,
    /// Client disconnecting.
    Disconnecting,
}

impl ModClientEventKind {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ModClientEventKind::Admitted => "admitted",
            ModClientEventKind::Userinfo => "userinfo",
            ModClientEventKind::Disconnecting => "disconnecting",
        }
    }
}

/// Mirror of the donor `client.connect` origin argument
/// (`src/world/session/session.ts`, via `NetworkAddress["kind"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetClientOrigin {
    /// Loopback client.
    Loopback,
    /// Remote client.
    Remote,
}

impl NetClientOrigin {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            NetClientOrigin::Loopback => "loopback",
            NetClientOrigin::Remote => "remote",
        }
    }
}

/// Used-surface mirror of `ExecutableRecipe` from donor
/// `src/contracts/content.ts` (canonical home: the recipe partition);
/// unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1HostRecipe {
    /// Movement provider.
    pub movement: String,
    /// Character definition provider.
    pub character_definition: String,
    /// Inventory provider.
    pub inventory: String,
    /// Weapon providers.
    pub weapons: Vec<String>,
    /// Map entities content.
    pub map_entities_content: ContentId,
}

/// Host failures, preserving the donor throw sites verbatim.
#[derive(Debug, Error)]
pub enum Q1QuakeCHostError {
    /// No NetQuake QuakeC source.
    #[error("Native NetQuake requires a NetQuake QuakeC source")]
    NotNetquakeSource,
    /// Precaches exceed the selected protocol.
    #[error("Native QuakeC precaches exceed selected NetQuake protocol")]
    PrecachesExceedProtocol,
    /// A precached model has no resource.
    #[error("Missing QuakeC model {0}")]
    MissingModel(String),
    /// A precached sound has no resource.
    #[error("Missing QuakeC sound {0}")]
    MissingSound(String),
    /// A referenced resource was never precached.
    #[error("Unprecached QuakeC resource {0}")]
    UnprecachedResource(String),
    /// A NetQuake ABI field is absent.
    #[error("Missing NetQuake ABI field {0}")]
    MissingField(String),
    /// An actor has no source slot.
    #[error("NetQuake actor has no source slot")]
    NoSourceSlot,
    /// QuakeC routed to an unexpected destination.
    #[error("Unexpected NetQuake runtime destination")]
    UnexpectedDestination,
    /// QuakeC tried to replace the host entity snapshot.
    #[error("QuakeC cannot replace the host entity snapshot")]
    EntitySnapshot,
    /// QuakeC signon contained an entity update.
    #[error("Entity update in QuakeC signon")]
    EntitySignon,
    /// Spawn replaced the reserved client identity.
    #[error("QuakeC spawn replaced reserved client identity")]
    SpawnReplacedIdentity,
    /// A carried client was never reserved.
    #[error("Carried QuakeC client has not been reserved")]
    CarriedNotReserved,
    /// Client reservation failed; carries the simulation message.
    #[error("{0}")]
    ReserveClient(String),
    /// Session client allocation failed.
    #[error(transparent)]
    Session(#[from] WorldError),
    /// Session client close failed; carries the session message.
    #[error("{0}")]
    SessionClose(String),
    /// Content fetch failed.
    #[error(transparent)]
    Content(#[from] MountError),
}

/// NetQuake QuakeC game surface used by this host.
///
/// Seam over `QuakeCSource` from donor
/// `src/app/bootstrap/simulation/quakec-source.ts` (canonical home:
/// `crate::bootstrap::simulation::quakec_source`); the quakec-source
/// partition implements it post-merge. VM-partition invariant failures
/// surface through that partition's own channel; the host preserves every
/// failure the donor file handles or raises itself.
pub trait Q1QuakeCNetGame {
    /// Donor `kind` discriminant.
    fn kind(&self) -> QuakeCSourceKind;
    /// Donor `options.maxClients`.
    fn max_clients(&self) -> u32;
    /// Donor `options.recipe.map.entities.content`.
    fn map_entities_content(&self) -> &ContentId;
    /// Donor `prepared.execution.owner.content`.
    fn execution_owner_content(&self) -> &ContentId;
    /// Donor `precacheNames`.
    fn precache_names(&self, kind: QuakeCPrecacheKind) -> Vec<String>;
    /// Donor `prepared.program.fieldsByName` offset lookup.
    fn field_offset(&self, name: &str) -> Option<i32>;
    /// Donor `entities.count`.
    fn entity_count(&self) -> i32;
    /// Whether donor `slots.at` is non-null.
    fn slot_occupied(&self, slot: i32) -> bool;
    /// Donor `entities.at(slot).float(offset)`.
    fn entity_float(&self, slot: i32, offset: i32) -> f64;
    /// Donor `entities.at(slot).int(offset)`.
    fn entity_int(&self, slot: i32, offset: i32) -> i32;
    /// Donor `entities.at(slot).vector(offset)`.
    fn entity_vector(&self, slot: i32, offset: i32) -> Vec3;
    /// Donor `entities.at(slot).setFloat(offset, value)`.
    fn entity_set_float(&mut self, slot: i32, offset: i32, value: f64);
    /// Donor `machine.strings.get`.
    fn string(&self, index: i32) -> String;
    /// Donor `machine.globals.float`.
    fn global_float(&self, offset: i32) -> f64;
    /// Donor `machine.globals.int`.
    fn global_int(&self, offset: i32) -> i32;
    /// Donor `machine.globalOffset`.
    fn global_offset(&self, name: &str) -> i32;
    /// Donor `sourceSlot`.
    fn source_slot(&self, actor: &ActorId) -> Option<i32>;
    /// Donor `timeSeconds`.
    fn time_seconds(&self) -> f64;
    /// Donor `cvars` registry.
    fn cvars(&self) -> &CvarRegistry;
    /// Donor `attachNetQuakeWire`.
    fn attach_netquake_wire(&mut self);
    /// Donor `netQuakeSignonMessages`.
    fn signon_messages(&self) -> Vec<Q1NetQuakeMessage>;
    /// Donor `drainNetQuakeMessages`.
    fn drain_messages(&mut self) -> Vec<Q1RoutedBatch>;
    /// Donor `consumeNetQuakeDamage`.
    fn consume_damage(&mut self, actor: &ActorId) -> Option<Q1NetDamage>;
    /// Donor `clientInfo`.
    fn client_info(&self, client: &ClientId) -> HashMap<String, String>;
    /// Donor `setClientInfo`.
    fn set_client_info(&mut self, client: &ClientId, info: &HashMap<String, String>);
    /// Donor `reservedClient`.
    fn reserved_client(&mut self, client: &ClientId) -> OwnedActor;
    /// Donor `clientActor`.
    fn client_actor(&self, client: &ClientId) -> Option<ActorId>;
    /// Donor `disconnectClient`.
    fn disconnect_client(&mut self, actor: &OwnedActor);
    /// Donor `isActiveClient`.
    fn is_active_client(&self, actor: &ActorId) -> bool;
}

/// Scene queries used by this host.
///
/// Seam over `SharedSceneQueries` from donor
/// `src/world/collision/index.ts` (canonical home: the world partition);
/// the partition implements it post-merge.
pub trait Q1HostScene {
    /// Donor `pointLeaf`.
    fn point_leaf(&self, point: &Vec3) -> i32;
    /// Donor `leafCluster`.
    fn leaf_cluster(&self, leaf: i32) -> i32;
    /// Donor `clusterVisible` over PVS.
    fn cluster_visible_pvs(&self, from: i32, to: i32) -> bool;
}

/// Simulation surface used by this host.
///
/// Seam over `SharedSimulation` from donor
/// `src/app/bootstrap/simulation/runtime.ts` (canonical home: the runtime
/// partition); the partition implements it post-merge.
pub trait Q1QuakeCHostSimulation {
    /// Donor `recipe` (used surface).
    fn recipe(&self) -> Q1HostRecipe;
    /// Donor `scene`.
    fn scene(&self) -> &dyn Q1HostScene;
    /// Donor `registerResource`.
    fn register_resource(&mut self, content: &ContentId, path: &str, resource: &ResolvedResourceReference);
    /// Donor `q1Paused`.
    fn q1_paused(&self) -> bool;
    /// Donor `players`.
    fn players(&self) -> Vec<ActorId>;
    /// Donor `reserveNetQuakeClient`; the message propagates like the donor
    /// admit cleanup rethrow.
    fn reserve_netquake_client(&mut self, client: &ClientId) -> Result<ActorId, String>;
    /// Donor `disconnectPlayer`.
    fn disconnect_player(&mut self, actor: &ActorId);
    /// Donor `admitPlayer` with defaulted travel and userinfo.
    fn admit_player(&mut self, client: &ClientId) -> PlayerAdmission;
    /// Donor `events.capture().persistent`.
    fn capture_persistent(&self) -> Vec<SimulationPresentationEvent>;
    /// Donor `events.lightStyle`.
    fn light_style(&self, index: u32) -> String;
    /// Donor `toggleQ1Pause`.
    fn toggle_q1_pause(&mut self, actor: &ActorId) -> String;
    /// Donor `notifyClientEvent`.
    fn notify_client_event(&mut self, kind: ModClientEventKind, actor: &ActorId);
    /// Donor `playerView`.
    fn player_view(&self, actor: &ActorId) -> PlayerView;
    /// Donor `playerCommand`.
    fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]);
}

/// Content surface used by this host.
///
/// Seam over `LoadedApplicationContent` from donor
/// `src/app/bootstrap/content.ts` (canonical home: the content partition);
/// the partition implements it post-merge.
pub trait Q1QuakeCHostContent {
    /// Donor `forContent`, synchronously.
    fn for_content(&self, content: &ContentId) -> Result<MountedContent, MountError>;
}

/// Session surface used by this host.
///
/// Seam over `EngineSession`/`SessionClient` from donor
/// `src/world/session/session.ts` (canonical home: `qa_world::session`,
/// which has not ported client connect/close yet); unify post-merge.
pub trait Q1QuakeCHostSession {
    /// Donor `session.createClient`.
    fn create_client(&mut self, slot: u32) -> Result<ClientId, WorldError>;
    /// Donor `client.connect`.
    fn connect_client(&mut self, client: &ClientId, origin: NetClientOrigin);
    /// Donor `session.closeClient`.
    fn close_client(&mut self, client: &ClientId) -> Result<(), String>;
}

/// Address rejection predicate, mirroring donor `rejects`.
pub type Q1RejectsFn = Rc<dyn Fn(&NetworkAddress) -> bool>;
/// Print sink, mirroring donor `print`.
pub type Q1PrintFn = Rc<dyn Fn(&str)>;

/// Mirror of `Q1ApplicationServerBindingOptions` from donor
/// `src/app/bootstrap/simulation/network-q1.ts` (canonical home:
/// `crate::bootstrap::simulation::network_q1`); unify post-merge.
pub struct Q1QuakeCServerOptions<S, C, E, G> {
    /// Engine session.
    pub session: E,
    /// Address rejection predicate.
    pub rejects: Option<Q1RejectsFn>,
    /// Shared simulation.
    pub simulation: S,
    /// Loaded content.
    pub content: C,
    /// Protocol identity.
    pub protocol: NqProfile,
    /// Donor `createNetQuakeCodec(protocol, reader).maxPrecache`.
    pub max_precache: usize,
    /// Print sink.
    pub print: Q1PrintFn,
    /// Donor `simulation.quakecSource()` (see the module docs).
    pub game: Option<G>,
}

/// Routed message with its audience, mirroring the donor `Routed`.
#[derive(Debug, Clone, PartialEq)]
struct RoutedMessage {
    /// Recipient actor, or broadcast when [`None`].
    recipient: Option<ActorId>,
    /// Reliable delivery.
    reliable: bool,
    /// Message.
    message: Q1ApplicationMessage,
}

/// Scoreboard row, mirroring the donor `board` entries.
#[derive(Debug, Clone, PartialEq)]
struct BoardEntry {
    /// Player name.
    name: String,
    /// Packed colors.
    colors: i32,
    /// Frags.
    frags: f64,
}

/// NetQuake server host over the NetQuake QuakeC source.
///
/// Concrete mirror of `Q1ApplicationServerHost` from donor
/// `src/app/bootstrap/network/q1-types.ts` (canonical home:
/// `crate::bootstrap::network::q1_types`); unify post-merge.
pub struct Q1QuakeCNetHost<S, E, G> {
    /// Engine session.
    session: E,
    /// Address rejection predicate.
    rejects: Option<Q1RejectsFn>,
    /// Shared simulation.
    simulation: S,
    /// NetQuake game.
    game: G,
    /// Protocol identity.
    protocol: NqProfile,
    /// Wide protocol (anything but 15).
    wide: bool,
    /// Maximum clients.
    max_clients: u32,
    /// Map name.
    map_name: String,
    /// Model precache indexes by path.
    models: HashMap<String, i32>,
    /// Model precaches in signon order.
    model_order: Vec<String>,
    /// Sound precache indexes by path.
    sounds: HashMap<String, i32>,
    /// Sound precaches in signon order.
    sound_order: Vec<String>,
    /// Admitted players by client slot.
    clients: HashMap<u32, Q1ApplicationPlayer>,
    /// Routed messages awaiting frame publication.
    routed: Vec<RoutedMessage>,
    /// Command messages queued for the next observation.
    command_messages: Vec<RoutedMessage>,
    /// Pause flag at the previous observation.
    previous_pause: bool,
    /// Scoreboard rows by client slot.
    board: HashMap<u32, BoardEntry>,
    /// Print sink.
    print: Q1PrintFn,
}

impl<S, E, G> std::fmt::Debug for Q1QuakeCNetHost<S, E, G> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q1QuakeCNetHost")
            .field("protocol", &self.protocol)
            .field("wide", &self.wide)
            .field("max_clients", &self.max_clients)
            .field("map_name", &self.map_name)
            .field("clients", &self.clients)
            .field("routed", &self.routed)
            .field("board", &self.board)
            .finish_non_exhaustive()
    }
}

/// Create the NetQuake host, mirroring donor `createQuakeCNetQuakeHost`.
pub fn create_quakec_netquake_host<S, C, E, G>(
    options: Q1QuakeCServerOptions<S, C, E, G>,
) -> Result<Q1QuakeCNetHost<S, E, G>, Q1QuakeCHostError>
where
    S: Q1QuakeCHostSimulation,
    C: Q1QuakeCHostContent,
    E: Q1QuakeCHostSession,
    G: Q1QuakeCNetGame,
{
    let game = options.game;
    let mut game = match game {
        Some(game) if game.kind() == QuakeCSourceKind::Netquake => game,
        _ => return Err(Q1QuakeCHostError::NotNetquakeSource),
    };
    let max_clients = game.max_clients();
    let wide = !matches!(options.protocol, NqProfile::Netquake);
    let models: Vec<String> = game.precache_names(QuakeCPrecacheKind::Model);
    let sounds: Vec<String> = game.precache_names(QuakeCPrecacheKind::Sound);
    if models.len() + 1 > options.max_precache || sounds.len() + 1 > options.max_precache {
        return Err(Q1QuakeCHostError::PrecachesExceedProtocol);
    }
    let model_order = models.clone();
    let sound_order = sounds.clone();
    let models: HashMap<String, i32> = models
        .into_iter()
        .enumerate()
        .map(|(index, path)| (path, index as i32 + 1))
        .collect();
    let sounds: HashMap<String, i32> = sounds
        .into_iter()
        .enumerate()
        .map(|(index, path)| (path, index as i32 + 1))
        .collect();
    let mut simulation = options.simulation;
    let mounts = options.content.for_content(game.execution_owner_content())?;
    for path in models.keys() {
        if !path.starts_with('*') && mounts.resolve(path)?.is_none() {
            return Err(Q1QuakeCHostError::MissingModel(path.clone()));
        }
    }
    for path in sounds.keys() {
        let resolved = format!("sound/{path}");
        match mounts.resolve(&resolved)? {
            None => return Err(Q1QuakeCHostError::MissingSound(path.clone())),
            Some(resource) => simulation.register_resource(game.execution_owner_content(), &resolved, &resource),
        }
    }
    let map_name = {
        let offset = game.global_offset("mapname");
        game.string(game.global_int(offset))
    };
    let previous_pause = simulation.q1_paused();
    game.attach_netquake_wire();
    Ok(Q1QuakeCNetHost {
        session: options.session,
        rejects: options.rejects,
        simulation,
        game,
        protocol: options.protocol,
        wide,
        max_clients,
        map_name,
        models,
        model_order,
        sounds,
        sound_order,
        clients: HashMap::new(),
        routed: Vec::new(),
        command_messages: Vec::new(),
        previous_pause,
        board: HashMap::new(),
        print: options.print,
    })
}

/// Donor `Number` conversion for userinfo and console color text, following
/// the `color` precedent in `qa_content::q1::composition::clients`: trim,
/// decimal parse with radix-prefix fallback, unparseable or non-finite as
/// zero (matching `ToInt32` on those inputs).
fn js_number_lossy(text: &str) -> f64 {
    let trimmed = text.trim();
    let parsed = trimmed.parse::<f64>().ok().or_else(|| {
        for (prefix, radix) in [("0x", 16), ("0X", 16), ("0b", 2), ("0B", 2), ("0o", 8), ("0O", 8)] {
            if let Some(digits) = trimmed.strip_prefix(prefix) {
                if digits.is_empty() {
                    return None;
                }
                return i64::from_str_radix(digits, radix).ok().map(|parsed| parsed as f64);
            }
        }
        None
    });
    let parsed = parsed.unwrap_or(0.0);
    if parsed.is_finite() {
        parsed
    } else {
        0.0
    }
}

/// Truncate donor `String.slice` style at a character boundary.
fn truncate_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

impl<S, E, G> Q1QuakeCNetHost<S, E, G>
where
    S: Q1QuakeCHostSimulation,
    E: Q1QuakeCHostSession,
    G: Q1QuakeCNetGame,
{
    /// Donor `rejects` passthrough: false when the predicate is absent.
    #[must_use]
    pub fn rejected(&self, address: &NetworkAddress) -> bool {
        self.rejects.as_ref().is_some_and(|rejects| rejects(address))
    }

    /// Donor `protocol`.
    #[must_use]
    pub fn protocol(&self) -> NqProfile {
        self.protocol
    }

    /// Donor `maxClients`.
    #[must_use]
    pub fn max_clients(&self) -> u32 {
        self.max_clients
    }

    /// Donor `mapName`.
    #[must_use]
    pub fn map_name(&self) -> &str {
        &self.map_name
    }

    /// Donor `supportsSourceWire`.
    #[must_use]
    pub fn supports_source_wire(&self) -> WireAdmission {
        let recipe = self.simulation.recipe();
        if recipe.movement.starts_with("q1:")
            && recipe.character_definition.starts_with("q1:")
            && recipe.inventory.starts_with("q1:")
            && recipe.weapons.iter().all(|value| value.starts_with("q1:"))
        {
            WireAdmission::Supported
        } else {
            WireAdmission::Unsupported {
                reasons: vec!["Mixed composition requires unified serialization".to_string()],
            }
        }
    }

    /// Donor `print`.
    pub fn print(&self, text: &str) {
        (self.print)(text);
    }

    /// Donor `index` precache lookup.
    fn precache_index(&self, path: &str, table: &HashMap<String, i32>) -> Result<i32, Q1QuakeCHostError> {
        if path.is_empty() {
            return Ok(0);
        }
        table
            .get(path)
            .copied()
            .ok_or_else(|| Q1QuakeCHostError::UnprecachedResource(path.to_string()))
    }

    /// Donor `field` ABI lookup.
    fn field_offset(&self, name: &str) -> Result<i32, Q1QuakeCHostError> {
        self.game
            .field_offset(name)
            .ok_or_else(|| Q1QuakeCHostError::MissingField(name.to_string()))
    }

    /// Donor `scalar` entity float.
    fn scalar(&self, slot: i32, name: &str) -> Result<f64, Q1QuakeCHostError> {
        let offset = self.field_offset(name)?;
        Ok(self.game.entity_float(slot, offset))
    }

    /// Donor `optional` entity float.
    fn optional_scalar(&self, slot: i32, name: &str) -> f64 {
        self.game
            .field_offset(name)
            .map(|offset| self.game.entity_float(slot, offset))
            .unwrap_or(0.0)
    }

    /// Donor `string` entity string.
    fn entity_string(&self, slot: i32, name: &str) -> Result<String, Q1QuakeCHostError> {
        let offset = self.field_offset(name)?;
        Ok(self.game.string(self.game.entity_int(slot, offset)))
    }

    /// Donor `vector` entity vector.
    fn entity_vector(&self, slot: i32, name: &str) -> Result<Vec3, Q1QuakeCHostError> {
        let offset = self.field_offset(name)?;
        Ok(self.game.entity_vector(slot, offset))
    }

    /// Donor `global` machine float.
    fn global_scalar(&self, name: &str) -> f64 {
        let offset = self.game.global_offset(name);
        self.game.global_float(offset)
    }

    /// Donor `number` actor slot.
    fn source_number(&self, actor: &ActorId) -> Result<i32, Q1QuakeCHostError> {
        self.game.source_slot(actor).ok_or(Q1QuakeCHostError::NoSourceSlot)
    }

    /// Donor `entities` snapshot.
    fn entities(&self) -> Result<Vec<Q1ExtendedEntityState>, Q1QuakeCHostError> {
        let mut result = Vec::new();
        for slot in 1..self.game.entity_count() {
            if !self.game.slot_occupied(slot)
                || self.scalar(slot, "modelindex")? == 0.0
                || self.entity_string(slot, "model")?.is_empty()
            {
                continue;
            }
            result.push(Q1ExtendedEntityState {
                number: slot,
                origin: self.entity_vector(slot, "origin")?,
                angles: self.entity_vector(slot, "angles")?,
                model_index: self.scalar(slot, "modelindex")?.trunc() as i32,
                frame: self.scalar(slot, "frame")?.trunc() as i32,
                color_map: self.scalar(slot, "colormap")?.trunc() as i32,
                skin: self.scalar(slot, "skin")?.trunc() as i32,
                effects: self.scalar(slot, "effects")?.trunc() as i32,
                alpha: if self.wide {
                    entalpha_encode(self.optional_scalar(slot, "alpha"))
                } else {
                    0
                },
                scale: if self.wide {
                    entscale_encode(self.optional_scalar(slot, "scale"))
                } else {
                    16
                },
                lerp_finish_seconds: 0.0,
                step: self.scalar(slot, "movetype")? == 4.0,
            });
        }
        Ok(result)
    }

    /// Donor `baseline` derivation.
    fn baseline(&self, state: &Q1ExtendedEntityState) -> Result<Q1ExtendedEntityState, Q1QuakeCHostError> {
        let player = state.number >= 1 && (state.number as u32) <= self.max_clients;
        let model_index = if player {
            self.precache_index("progs/player.mdl", &self.models)?
        } else {
            state.model_index
        };
        let rmq = matches!(self.protocol, NqProfile::Rmq { .. });
        Ok(Q1ExtendedEntityState {
            number: state.number,
            origin: state.origin,
            angles: state.angles,
            model_index: if !self.wide && model_index > 255 {
                0
            } else {
                model_index
            },
            frame: if !self.wide && state.frame > 255 {
                0
            } else {
                state.frame
            },
            color_map: if player { state.number } else { 0 },
            skin: state.skin,
            effects: 0,
            alpha: if player { 0 } else { state.alpha },
            scale: if player || !rmq { 16 } else { state.scale },
            lerp_finish_seconds: state.lerp_finish_seconds,
            step: false,
        })
    }

    /// Donor `clientData` message.
    fn client_data(&self, player: &Q1ApplicationPlayer) -> Result<Q1ApplicationMessage, Q1QuakeCHostError> {
        let slot = player.source_entity;
        let items = (self.scalar(slot, "items")?.trunc() as i32)
            | match self.game.field_offset("items2") {
                None => (self.global_scalar("serverflags").trunc() as i32) << 28,
                Some(offset) => (self.game.entity_float(slot, offset).trunc() as i32) << 23,
            };
        let weapon = self.scalar(slot, "weapon")?.trunc() as i32;
        let content = self.game.map_entities_content();
        let expanded = content.as_str().contains(":rogue:") || content.as_str().contains(":hipnotic:");
        let active_weapon = if expanded {
            if weapon == 0 {
                0
            } else {
                31 - (weapon.isolate_lowest_one() as u32).leading_zeros() as i32
            }
        } else {
            weapon
        };
        Ok(Q1NetQuakeMessage::ClientData {
            weapon_alpha: if self.wide {
                entalpha_encode(self.optional_scalar(slot, "alpha"))
            } else {
                0
            },
            data: Q1ClientData {
                view_height: self.entity_vector(slot, "view_ofs")?.z as f64,
                ideal_pitch: self.scalar(slot, "idealpitch")?,
                punch_angles: self.entity_vector(slot, "punchangle")?,
                velocity: self.entity_vector(slot, "velocity")?,
                items,
                on_ground: (self.scalar(slot, "flags")?.trunc() as i32 & 512) != 0,
                in_water: self.scalar(slot, "waterlevel")? >= 2.0,
                weapon_frame: self.scalar(slot, "weaponframe")?,
                armor: self.scalar(slot, "armorvalue")?,
                weapon_model: self.precache_index(&self.entity_string(slot, "weaponmodel")?, &self.models)?,
                health: self.scalar(slot, "health")?,
                ammo: self.scalar(slot, "currentammo")?,
                shells: self.scalar(slot, "ammo_shells")?,
                nails: self.scalar(slot, "ammo_nails")?,
                rockets: self.scalar(slot, "ammo_rockets")?,
                cells: self.scalar(slot, "ammo_cells")?,
                active_weapon,
            },
        })
    }

    /// Donor `send` routing append.
    fn send(&mut self, message: Q1ApplicationMessage, reliable: bool, recipient: Option<ActorId>) {
        self.routed.push(RoutedMessage {
            recipient,
            reliable,
            message,
        });
    }

    /// Donor `sourceMessages` drain.
    fn source_messages(&mut self) -> Result<(), Q1QuakeCHostError> {
        for batch in self.game.drain_messages() {
            let (reliable, recipient) = match &batch.destination {
                QcMessageDestination::Multicast { .. } | QcMessageDestination::Signon => {
                    return Err(Q1QuakeCHostError::UnexpectedDestination);
                }
                QcMessageDestination::Broadcast { reliable } => (*reliable, None),
                QcMessageDestination::Client { actor } => (true, Some(actor.clone())),
            };
            for message in batch.messages {
                if matches!(message, Q1NetQuakeMessage::Entity { .. }) {
                    return Err(Q1QuakeCHostError::EntitySnapshot);
                }
                self.send(message, reliable, recipient.clone());
            }
        }
        Ok(())
    }

    /// Donor `persistentSignon`.
    fn persistent_signon(&self) -> Result<Vec<Q1ApplicationMessage>, Q1QuakeCHostError> {
        let mut messages = Vec::new();
        for message in self.game.signon_messages() {
            if matches!(message, Q1NetQuakeMessage::Entity { .. }) {
                return Err(Q1QuakeCHostError::EntitySignon);
            }
            messages.push(message);
        }
        for record in self.simulation.capture_persistent() {
            let SourcePresentationEvent::Q1(event) = record.event else {
                continue;
            };
            match event {
                Q1Event::Ambient {
                    origin,
                    path,
                    volume,
                    attenuation,
                } => messages.push(Q1NetQuakeMessage::Sound {
                    kind: Q1SoundKind::StaticSound,
                    entity: 0,
                    channel: 0,
                    index: self.precache_index(&path, &self.sounds)?,
                    volume: (volume * 255.0).trunc() as i32,
                    attenuation,
                    origin,
                }),
                Q1Event::StaticModel {
                    path,
                    frame,
                    color_map,
                    skin,
                    origin,
                    angles,
                } => messages.push(Q1NetQuakeMessage::Static {
                    state: Q1ExtendedEntityState {
                        number: 0,
                        origin,
                        angles,
                        model_index: self.precache_index(&path, &self.models)?,
                        frame,
                        color_map,
                        skin,
                        effects: 0,
                        alpha: 0,
                        scale: 16,
                        lerp_finish_seconds: 0.0,
                        step: false,
                    },
                }),
                _ => {}
            }
        }
        Ok(messages)
    }

    /// Donor `admit`.
    pub fn admit(&mut self, from: &NetworkAddress) -> Result<Q1ApplicationAdmission, Q1QuakeCHostError> {
        let mut slot = 0;
        while slot < self.max_clients {
            let taken = self.clients.contains_key(&slot)
                || self
                    .simulation
                    .players()
                    .iter()
                    .any(|actor| self.game.source_slot(actor) == Some(slot as i32 + 1));
            if !taken {
                break;
            }
            slot += 1;
        }
        if slot == self.max_clients {
            return Ok(Q1ApplicationAdmission::Rejected {
                reason: "Server is full".to_string(),
            });
        }
        let client = self.session.create_client(slot)?;
        let origin = if from.kind() == "loopback" {
            NetClientOrigin::Loopback
        } else {
            NetClientOrigin::Remote
        };
        self.session.connect_client(&client, origin);
        let actor = match self.simulation.reserve_netquake_client(&client) {
            Ok(actor) => actor,
            Err(message) => {
                self.session
                    .close_client(&client)
                    .map_err(Q1QuakeCHostError::SessionClose)?;
                return Err(Q1QuakeCHostError::ReserveClient(message));
            }
        };
        let source_entity = match self.source_number(&actor) {
            Ok(slot) => slot,
            Err(error) => {
                let reserved = self.game.reserved_client(&client);
                self.game.disconnect_client(&reserved);
                self.session
                    .close_client(&client)
                    .map_err(Q1QuakeCHostError::SessionClose)?;
                return Err(error);
            }
        };
        let player = Q1ApplicationPlayer {
            client: client.clone(),
            actor,
            source_entity,
        };
        self.clients.insert(slot, player.clone());
        Ok(Q1ApplicationAdmission::Accepted { player })
    }

    /// Donor `carriedPlayer`.
    pub fn carried_player(&mut self, client: &ClientId) -> Result<Q1ApplicationPlayer, Q1QuakeCHostError> {
        let actor = self
            .game
            .client_actor(client)
            .ok_or(Q1QuakeCHostError::CarriedNotReserved)?;
        let player = Q1ApplicationPlayer {
            client: client.clone(),
            actor: actor.clone(),
            source_entity: self.source_number(&actor)?,
        };
        self.clients.insert(client.slot(), player.clone());
        Ok(player)
    }

    /// Donor `disconnect`; the interface reason is unused by the donor body.
    pub fn disconnect(&mut self, player: &Q1ApplicationPlayer, _reason: &str) -> Result<(), Q1QuakeCHostError> {
        if self.game.is_active_client(&player.actor) {
            self.simulation.disconnect_player(&player.actor);
        } else {
            let reserved = self.game.reserved_client(&player.client);
            self.game.disconnect_client(&reserved);
        }
        self.session
            .close_client(&player.client)
            .map_err(Q1QuakeCHostError::SessionClose)?;
        self.clients.remove(&player.client.slot());
        Ok(())
    }

    /// Donor `gameState`.
    pub fn game_state(&self, _player: &Q1ApplicationPlayer) -> Result<Q1ApplicationGameState, Q1QuakeCHostError> {
        let mut baselines = HashMap::new();
        for state in self.entities()? {
            let baseline = self.baseline(&state)?;
            baselines.insert(state.number, baseline);
        }
        for slot in 1..=self.max_clients as i32 {
            if baselines.contains_key(&slot) {
                continue;
            }
            let shell = Q1ExtendedEntityState {
                number: slot,
                origin: self.entity_vector(slot, "origin")?,
                angles: self.entity_vector(slot, "angles")?,
                model_index: 0,
                frame: 0,
                color_map: slot,
                skin: 0,
                effects: 0,
                alpha: 0,
                scale: 16,
                lerp_finish_seconds: 0.0,
                step: false,
            };
            baselines.insert(slot, self.baseline(&shell)?);
        }
        let message = self.entity_string(0, "message")?;
        let level = if message.is_empty() {
            let offset = self.game.global_offset("mapname");
            self.game.string(self.game.global_int(offset))
        } else {
            message
        };
        Ok(Q1ApplicationGameState {
            info: Q1ServerInfo {
                protocol: self.protocol,
                max_clients: self.max_clients,
                game_type: if self.game.cvars().variable_value("deathmatch") == 0.0 {
                    0
                } else {
                    1
                },
                level,
                models: self.model_order.clone(),
                sounds: self.sound_order.clone(),
            },
            baselines,
            signon: self.persistent_signon()?,
        })
    }

    /// Donor `spawn`.
    pub fn spawn(&mut self, player: &Q1ApplicationPlayer) -> Result<Vec<Q1ApplicationMessage>, Q1QuakeCHostError> {
        if !self.game.is_active_client(&player.actor) {
            let admitted = self.simulation.admit_player(&player.client);
            if admitted.actor != player.actor {
                return Err(Q1QuakeCHostError::SpawnReplacedIdentity);
            }
        }
        let mut messages = vec![
            Q1NetQuakeMessage::Pause {
                paused: self.simulation.q1_paused(),
            },
            Q1NetQuakeMessage::Time {
                seconds: self.game.time_seconds(),
            },
        ];
        for index in 0..64u32 {
            messages.push(Q1NetQuakeMessage::LightStyle {
                index: index as i32,
                value: self.simulation.light_style(index),
            });
        }
        let mut slots: Vec<u32> = self.clients.keys().copied().collect();
        slots.sort_unstable();
        for slot in slots {
            let client = &self.clients[&slot];
            let source = client.source_entity;
            messages.push(Q1NetQuakeMessage::NamedSlot {
                kind: Q1NamedSlotKind::Name,
                slot: slot as i32,
                value: self.entity_string(source, "netname")?,
            });
            messages.push(Q1NetQuakeMessage::NumberedSlot {
                kind: Q1NumberedSlotKind::Frags,
                slot: slot as i32,
                value: self.scalar(source, "frags")?,
            });
            messages.push(Q1NetQuakeMessage::NumberedSlot {
                kind: Q1NumberedSlotKind::Colors,
                slot: slot as i32,
                value: self.board.get(&slot).map_or(0.0, |entry| entry.colors as f64),
            });
        }
        for (stat, name) in [
            (11, "total_secrets"),
            (12, "total_monsters"),
            (13, "found_secrets"),
            (14, "killed_monsters"),
        ] {
            messages.push(Q1NetQuakeMessage::Stat {
                index: stat,
                value: self.global_scalar(name),
            });
        }
        messages.push(Q1NetQuakeMessage::SetAngle {
            angles: self.entity_vector(player.source_entity, "angles")?,
        });
        messages.push(self.client_data(player)?);
        Ok(messages)
    }

    /// Donor `frame`; the interface output is unused by the donor body.
    pub fn frame(
        &mut self,
        player: &Q1ApplicationPlayer,
        _output: &SimulationOutput,
    ) -> Result<Q1ApplicationFrame, Q1QuakeCHostError> {
        let addressed =
            |event: &RoutedMessage| event.recipient.is_none() || event.recipient.as_ref() == Some(&player.actor);
        if !self.game.is_active_client(&player.actor) {
            return Ok(Q1ApplicationFrame {
                seconds: self.game.time_seconds(),
                messages: Vec::new(),
                reliable: self
                    .routed
                    .iter()
                    .filter(|event| event.reliable && addressed(event))
                    .map(|event| event.message.clone())
                    .collect(),
                datagram: Vec::new(),
                entities: Vec::new(),
            });
        }
        let view = self.simulation.player_view(&player.actor);
        let scene = self.simulation.scene();
        let cluster = scene.leaf_cluster(scene.point_leaf(&view.origin));
        let mut messages = vec![self.client_data(player)?];
        if let Some(damage) = self.game.consume_damage(&player.actor) {
            messages.insert(
                0,
                Q1NetQuakeMessage::Damage {
                    armor: damage.armor,
                    blood: damage.blood,
                    source: damage.source,
                },
            );
        }
        let mut entities = Vec::new();
        for state in self.entities()? {
            if self.wide && state.alpha == ENTALPHA_ZERO && state.effects == 0 {
                continue;
            }
            if state.number != player.source_entity {
                let scene = self.simulation.scene();
                let target = scene.leaf_cluster(scene.point_leaf(&state.origin));
                if !scene.cluster_visible_pvs(cluster, target) {
                    continue;
                }
            }
            entities.push(state);
        }
        Ok(Q1ApplicationFrame {
            seconds: self.game.time_seconds(),
            messages,
            reliable: self
                .routed
                .iter()
                .filter(|event| event.reliable && addressed(event))
                .map(|event| event.message.clone())
                .collect(),
            datagram: self
                .routed
                .iter()
                .filter(|event| !event.reliable && addressed(event))
                .map(|event| event.message.clone())
                .collect(),
            entities,
        })
    }

    /// Donor `input`.
    #[must_use]
    pub fn input(&self, player: &Q1ApplicationPlayer, command: &Q1UserCommand, sequence: u32) -> ActorCommand {
        ActorCommand {
            actor: player.actor.clone(),
            source: CommandSource::Remote {
                client: player.client.clone(),
            },
            sequence: u64::from(sequence),
            command: UserCommand::Q1Netquake {
                acknowledged_server_time_seconds: command.acknowledged_server_time_seconds,
                view_angles: [
                    f64::from(command.view_angles.x),
                    f64::from(command.view_angles.y),
                    f64::from(command.view_angles.z),
                ],
                forward_move: command.forward_move,
                side_move: command.side_move,
                up_move: command.up_move,
                buttons: f64::from(command.buttons),
                impulse: f64::from(command.impulse),
            },
            arsenal: None,
        }
    }

    /// Donor `command`.
    pub fn command(
        &mut self,
        player: &Q1ApplicationPlayer,
        name: &str,
        args: &[String],
    ) -> Result<(), Q1QuakeCHostError> {
        if name == "pause" {
            let before = self.simulation.q1_paused();
            let text = self.simulation.toggle_q1_pause(&player.actor);
            let recipient = if before == self.simulation.q1_paused() {
                Some(player.actor.clone())
            } else {
                None
            };
            self.command_messages.push(RoutedMessage {
                recipient,
                reliable: true,
                message: Q1NetQuakeMessage::Text {
                    kind: Q1TextMessageKind::Print,
                    text,
                },
            });
        } else if name == "name" || name == "color" {
            let mut info = self.game.client_info(&player.client);
            if name == "name" {
                let name = args.first().map_or("unconnected", String::as_str);
                info.insert("name".to_string(), truncate_chars(name, 15));
            } else {
                let first = args.first().map_or("0", String::as_str);
                let second = args.get(1).map_or(first, String::as_str);
                let top = (js_number_lossy(first).trunc() as i32 & 15).min(13);
                let bottom = (js_number_lossy(second).trunc() as i32 & 15).min(13);
                info.insert("topcolor".to_string(), top.to_string());
                info.insert("bottomcolor".to_string(), bottom.to_string());
                let offset = self.field_offset("team")?;
                self.game
                    .entity_set_float(player.source_entity, offset, f64::from(bottom + 1));
            }
            self.game.set_client_info(&player.client, &info);
            self.simulation
                .notify_client_event(ModClientEventKind::Userinfo, &player.actor);
        } else if name == "say" || name == "say_team" {
            let text = format!(
                "\u{1}{}: {}\n",
                self.entity_string(player.source_entity, "netname")?,
                truncate_chars(&args.join(" "), 126)
            );
            let teamplay = self.game.cvars().variable_value("teamplay");
            let mut recipients: Vec<u32> = self.clients.keys().copied().collect();
            recipients.sort_unstable();
            for slot in recipients {
                let recipient = self.clients[&slot].clone();
                if name == "say_team"
                    && teamplay != 0.0
                    && self.scalar(recipient.source_entity, "team")? != self.scalar(player.source_entity, "team")?
                {
                    continue;
                }
                self.command_messages.push(RoutedMessage {
                    recipient: Some(recipient.actor),
                    reliable: true,
                    message: Q1NetQuakeMessage::Text {
                        kind: Q1TextMessageKind::Print,
                        text: text.clone(),
                    },
                });
            }
        } else {
            self.simulation.player_command(&player.actor, name, args);
        }
        Ok(())
    }

    /// Donor `observe`.
    pub fn observe(
        &mut self,
        _output: &SimulationOutput,
        events: &[SimulationPresentationEvent],
    ) -> Result<(), Q1QuakeCHostError> {
        self.routed = std::mem::take(&mut self.command_messages);
        self.source_messages()?;
        if self.previous_pause != self.simulation.q1_paused() {
            self.previous_pause = self.simulation.q1_paused();
            let paused = self.previous_pause;
            self.send(Q1NetQuakeMessage::Pause { paused }, true, None);
        }
        let mut slots: Vec<u32> = self.clients.keys().copied().collect();
        slots.sort_unstable();
        for slot in slots {
            let player = self.clients[&slot].clone();
            let info = self.game.client_info(&player.client);
            let name = self.entity_string(player.source_entity, "netname")?;
            let top = js_number_lossy(info.get("topcolor").map_or("0", String::as_str)).trunc() as i32 & 15;
            let bottom = js_number_lossy(info.get("bottomcolor").map_or("0", String::as_str)).trunc() as i32 & 15;
            let colors = (top << 4) | bottom;
            let frags = self.scalar(player.source_entity, "frags")?;
            let (name_changed, colors_changed, frags_changed) = {
                let previous = self.board.get(&slot);
                (
                    previous.is_none_or(|entry| entry.name != name),
                    previous.is_none_or(|entry| entry.colors != colors),
                    previous.is_none_or(|entry| entry.frags != frags),
                )
            };
            if name_changed {
                self.send(
                    Q1NetQuakeMessage::NamedSlot {
                        kind: Q1NamedSlotKind::Name,
                        slot: slot as i32,
                        value: name.clone(),
                    },
                    true,
                    None,
                );
            }
            if colors_changed {
                self.send(
                    Q1NetQuakeMessage::NumberedSlot {
                        kind: Q1NumberedSlotKind::Colors,
                        slot: slot as i32,
                        value: f64::from(colors),
                    },
                    true,
                    None,
                );
            }
            if frags_changed {
                self.send(
                    Q1NetQuakeMessage::NumberedSlot {
                        kind: Q1NumberedSlotKind::Frags,
                        slot: slot as i32,
                        value: frags,
                    },
                    true,
                    None,
                );
            }
            self.board.insert(slot, BoardEntry { name, colors, frags });
        }
        let mut stale: Vec<u32> = self
            .board
            .keys()
            .copied()
            .filter(|slot| !self.clients.contains_key(slot))
            .collect();
        stale.sort_unstable();
        for slot in stale {
            self.send(
                Q1NetQuakeMessage::NamedSlot {
                    kind: Q1NamedSlotKind::Name,
                    slot: slot as i32,
                    value: String::new(),
                },
                true,
                None,
            );
            self.send(
                Q1NetQuakeMessage::NumberedSlot {
                    kind: Q1NumberedSlotKind::Colors,
                    slot: slot as i32,
                    value: 0.0,
                },
                true,
                None,
            );
            self.send(
                Q1NetQuakeMessage::NumberedSlot {
                    kind: Q1NumberedSlotKind::Frags,
                    slot: slot as i32,
                    value: 0.0,
                },
                true,
                None,
            );
            self.board.remove(&slot);
        }
        for record in events {
            match &record.event {
                SourcePresentationEvent::ViewReset { actor, angles, .. } => {
                    self.send(
                        Q1NetQuakeMessage::SetAngle { angles: *angles },
                        true,
                        Some(actor.clone()),
                    );
                }
                SourcePresentationEvent::Q1(event) => match event {
                    Q1Event::Particles {
                        origin,
                        direction,
                        color,
                        count,
                    } => {
                        let clamp = |value: f32| (f64::from(value) * 16.0).trunc().clamp(-128.0, 127.0) / 16.0;
                        self.send(
                            Q1NetQuakeMessage::Particle {
                                origin: *origin,
                                direction: Vec3 {
                                    x: clamp(direction.x) as f32,
                                    y: clamp(direction.y) as f32,
                                    z: clamp(direction.z) as f32,
                                },
                                count: *count,
                                color: *color,
                            },
                            false,
                            None,
                        );
                    }
                    Q1Event::Lightstyle { style, pattern } => {
                        self.send(
                            Q1NetQuakeMessage::LightStyle {
                                index: *style,
                                value: pattern.clone(),
                            },
                            true,
                            None,
                        );
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::{ContentMount, LooseMount, MountId, MountIdentity, MountPlanId, ResolvedMountPlan};
    use qa_content::mounts::{open_mount_plan, OpenMountOptions};
    use qa_core::cmd::Dialect;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::vec3;
    use qa_core::time::{FrameContext, FramePhase, SourceTime};
    use qa_guest::qc::presentation_host::VisibilityScope;
    use qa_world::session::{SimulationOutput, WorldSnapshot};

    use super::super::types::ViewResetReason;

    struct StubScene {
        visible: bool,
    }

    impl Q1HostScene for StubScene {
        fn point_leaf(&self, _point: &Vec3) -> i32 {
            7
        }

        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            3
        }

        fn cluster_visible_pvs(&self, _from: i32, _to: i32) -> bool {
            self.visible
        }
    }

    struct StubGame {
        kind: QuakeCSourceKind,
        max_clients: u32,
        map_content: ContentId,
        owner_content: ContentId,
        models: Vec<String>,
        sounds: Vec<String>,
        fields: HashMap<String, i32>,
        entity_count: i32,
        occupied: Vec<i32>,
        floats: HashMap<(i32, i32), f64>,
        ints: HashMap<(i32, i32), i32>,
        vectors: HashMap<(i32, i32), Vec3>,
        strings: Vec<String>,
        globals: HashMap<i32, f64>,
        global_ints: HashMap<i32, i32>,
        global_offsets: HashMap<String, i32>,
        source_slots: HashMap<ActorId, i32>,
        time_seconds: f64,
        cvars: CvarRegistry,
        attached: bool,
        signon: Vec<Q1NetQuakeMessage>,
        drain: Vec<Q1RoutedBatch>,
        damage: Option<Q1NetDamage>,
        infos: HashMap<ClientId, HashMap<String, String>>,
        reserved: HashMap<ClientId, OwnedActor>,
        actors: HashMap<ClientId, ActorId>,
        active: Vec<ActorId>,
        disconnects: Vec<ActorId>,
        set_floats: Vec<(i32, i32, f64)>,
    }

    impl StubGame {
        fn fields() -> HashMap<String, i32> {
            [
                "modelindex",
                "model",
                "origin",
                "angles",
                "frame",
                "colormap",
                "skin",
                "effects",
                "alpha",
                "scale",
                "movetype",
                "netname",
                "frags",
                "items",
                "weapon",
                "view_ofs",
                "idealpitch",
                "punchangle",
                "velocity",
                "flags",
                "waterlevel",
                "weaponframe",
                "armorvalue",
                "weaponmodel",
                "health",
                "currentammo",
                "ammo_shells",
                "ammo_nails",
                "ammo_rockets",
                "ammo_cells",
                "message",
                "team",
            ]
            .into_iter()
            .enumerate()
            .map(|(index, name)| (name.to_string(), index as i32))
            .collect()
        }

        fn offset(&self, name: &str) -> i32 {
            self.fields[name]
        }

        fn with_entities() -> Self {
            let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
            cvars.register("deathmatch", "1", 0).unwrap();
            cvars.register("teamplay", "0", 0).unwrap();
            let fields = Self::fields();
            let at = |name: &str| fields[name];
            let strings = vec![
                String::new(),
                "progs/player.mdl".to_string(),
                "progs/wizard.mdl".to_string(),
                "player".to_string(),
                "dm3".to_string(),
            ];
            let mut floats = HashMap::new();
            let mut ints = HashMap::new();
            let mut vectors = HashMap::new();
            floats.insert((1, at("modelindex")), 1.0);
            ints.insert((1, at("model")), 1);
            vectors.insert((1, at("origin")), vec3(10.0, 20.0, 30.0));
            vectors.insert((1, at("angles")), vec3(0.0, 90.0, 0.0));
            floats.insert((1, at("frame")), 5.0);
            floats.insert((1, at("colormap")), 1.0);
            floats.insert((1, at("effects")), 0.0);
            floats.insert((1, at("movetype")), 4.0);
            ints.insert((1, at("netname")), 3);
            floats.insert((1, at("frags")), 7.0);
            floats.insert((1, at("items")), 3.0);
            floats.insert((1, at("weapon")), 8.0);
            vectors.insert((1, at("view_ofs")), vec3(0.0, 0.0, 22.0));
            vectors.insert((1, at("punchangle")), vec3(1.0, 2.0, 3.0));
            vectors.insert((1, at("velocity")), vec3(4.0, 5.0, 6.0));
            floats.insert((1, at("flags")), 512.0);
            floats.insert((1, at("waterlevel")), 0.0);
            floats.insert((1, at("weaponframe")), 1.0);
            floats.insert((1, at("armorvalue")), 50.0);
            ints.insert((1, at("weaponmodel")), 1);
            floats.insert((1, at("health")), 100.0);
            floats.insert((1, at("currentammo")), 2.0);
            floats.insert((1, at("ammo_shells")), 10.0);
            floats.insert((1, at("ammo_nails")), 20.0);
            floats.insert((1, at("ammo_rockets")), 5.0);
            floats.insert((1, at("ammo_cells")), 30.0);
            floats.insert((1, at("team")), 1.0);
            floats.insert((2, at("modelindex")), 2.0);
            ints.insert((2, at("model")), 2);
            vectors.insert((2, at("origin")), vec3(40.0, 50.0, 60.0));
            vectors.insert((2, at("angles")), vec3(0.0, 0.0, 0.0));
            floats.insert((2, at("frame")), 300.0);
            floats.insert((2, at("effects")), 4.0);
            floats.insert((2, at("alpha")), 0.5);
            floats.insert((2, at("scale")), 2.0);
            floats.insert((2, at("movetype")), 3.0);
            ints.insert((0, at("message")), 0);
            Self {
                kind: QuakeCSourceKind::Netquake,
                max_clients: 2,
                map_content: ContentId("q1:id1:base:1".to_string()),
                owner_content: ContentId("q1:test:content:1".to_string()),
                models: vec!["progs/player.mdl".to_string(), "progs/wizard.mdl".to_string()],
                sounds: vec!["weapons/rocket1.wav".to_string()],
                fields,
                entity_count: 4,
                occupied: vec![1, 2],
                floats,
                ints,
                vectors,
                strings,
                globals: HashMap::from([(101, 2.0), (102, 5.0), (103, 20.0), (104, 1.0), (105, 7.0)]),
                global_ints: HashMap::from([(100, 4)]),
                global_offsets: HashMap::from([
                    ("mapname".to_string(), 100),
                    ("serverflags".to_string(), 101),
                    ("total_secrets".to_string(), 102),
                    ("total_monsters".to_string(), 103),
                    ("found_secrets".to_string(), 104),
                    ("killed_monsters".to_string(), 105),
                ]),
                source_slots: HashMap::new(),
                time_seconds: 12.5,
                cvars,
                attached: false,
                signon: Vec::new(),
                drain: Vec::new(),
                damage: None,
                infos: HashMap::new(),
                reserved: HashMap::new(),
                actors: HashMap::new(),
                active: Vec::new(),
                disconnects: Vec::new(),
                set_floats: Vec::new(),
            }
        }
    }

    impl Q1QuakeCNetGame for StubGame {
        fn kind(&self) -> QuakeCSourceKind {
            self.kind
        }

        fn max_clients(&self) -> u32 {
            self.max_clients
        }

        fn map_entities_content(&self) -> &ContentId {
            &self.map_content
        }

        fn execution_owner_content(&self) -> &ContentId {
            &self.owner_content
        }

        fn precache_names(&self, kind: QuakeCPrecacheKind) -> Vec<String> {
            match kind {
                QuakeCPrecacheKind::Model => self.models.clone(),
                QuakeCPrecacheKind::Sound => self.sounds.clone(),
            }
        }

        fn field_offset(&self, name: &str) -> Option<i32> {
            self.fields.get(name).copied()
        }

        fn entity_count(&self) -> i32 {
            self.entity_count
        }

        fn slot_occupied(&self, slot: i32) -> bool {
            self.occupied.contains(&slot)
        }

        fn entity_float(&self, slot: i32, offset: i32) -> f64 {
            self.floats.get(&(slot, offset)).copied().unwrap_or(0.0)
        }

        fn entity_int(&self, slot: i32, offset: i32) -> i32 {
            self.ints.get(&(slot, offset)).copied().unwrap_or(0)
        }

        fn entity_vector(&self, slot: i32, offset: i32) -> Vec3 {
            self.vectors
                .get(&(slot, offset))
                .copied()
                .unwrap_or(vec3(0.0, 0.0, 0.0))
        }

        fn entity_set_float(&mut self, slot: i32, offset: i32, value: f64) {
            self.set_floats.push((slot, offset, value));
            self.floats.insert((slot, offset), value);
        }

        fn string(&self, index: i32) -> String {
            self.strings.get(index as usize).cloned().unwrap_or_default()
        }

        fn global_float(&self, offset: i32) -> f64 {
            self.globals.get(&offset).copied().unwrap_or(0.0)
        }

        fn global_int(&self, offset: i32) -> i32 {
            self.global_ints.get(&offset).copied().unwrap_or(0)
        }

        fn global_offset(&self, name: &str) -> i32 {
            self.global_offsets.get(name).copied().unwrap_or(0)
        }

        fn source_slot(&self, actor: &ActorId) -> Option<i32> {
            self.source_slots.get(actor).copied()
        }

        fn time_seconds(&self) -> f64 {
            self.time_seconds
        }

        fn cvars(&self) -> &CvarRegistry {
            &self.cvars
        }

        fn attach_netquake_wire(&mut self) {
            self.attached = true;
        }

        fn signon_messages(&self) -> Vec<Q1NetQuakeMessage> {
            self.signon.clone()
        }

        fn drain_messages(&mut self) -> Vec<Q1RoutedBatch> {
            std::mem::take(&mut self.drain)
        }

        fn consume_damage(&mut self, _actor: &ActorId) -> Option<Q1NetDamage> {
            self.damage.take()
        }

        fn client_info(&self, client: &ClientId) -> HashMap<String, String> {
            self.infos.get(client).cloned().unwrap_or_default()
        }

        fn set_client_info(&mut self, client: &ClientId, info: &HashMap<String, String>) {
            self.infos.insert(client.clone(), info.clone());
        }

        fn reserved_client(&mut self, client: &ClientId) -> OwnedActor {
            self.reserved[client].clone()
        }

        fn client_actor(&self, client: &ClientId) -> Option<ActorId> {
            self.actors.get(client).cloned()
        }

        fn disconnect_client(&mut self, actor: &OwnedActor) {
            self.disconnects.push(actor.id().clone());
        }

        fn is_active_client(&self, actor: &ActorId) -> bool {
            self.active.contains(actor)
        }
    }

    struct StubSim {
        recipe: Q1HostRecipe,
        scene: StubScene,
        resources: Vec<(String, String)>,
        paused: bool,
        players: Vec<ActorId>,
        reserve_ok: HashMap<ClientId, ActorId>,
        reserve_err: Option<String>,
        disconnects: Vec<ActorId>,
        admit_actor: Option<ActorId>,
        persistent: Vec<SimulationPresentationEvent>,
        notified: Vec<(ModClientEventKind, ActorId)>,
        view_origin: Vec3,
        commands: Vec<(ActorId, String, Vec<String>)>,
    }

    impl StubSim {
        fn new() -> Self {
            Self {
                recipe: Q1HostRecipe {
                    movement: "q1:move".to_string(),
                    character_definition: "q1:char".to_string(),
                    inventory: "q1:inv".to_string(),
                    weapons: vec!["q1:shotgun".to_string()],
                    map_entities_content: ContentId("q1:id1:base:1".to_string()),
                },
                scene: StubScene { visible: true },
                resources: Vec::new(),
                paused: false,
                players: Vec::new(),
                reserve_ok: HashMap::new(),
                reserve_err: None,
                disconnects: Vec::new(),
                admit_actor: None,
                persistent: Vec::new(),
                notified: Vec::new(),
                view_origin: vec3(100.0, 200.0, 300.0),
                commands: Vec::new(),
            }
        }
    }

    impl Q1QuakeCHostSimulation for StubSim {
        fn recipe(&self) -> Q1HostRecipe {
            self.recipe.clone()
        }

        fn scene(&self) -> &dyn Q1HostScene {
            &self.scene
        }

        fn register_resource(&mut self, content: &ContentId, path: &str, _resource: &ResolvedResourceReference) {
            self.resources.push((content.as_str().to_string(), path.to_string()));
        }

        fn q1_paused(&self) -> bool {
            self.paused
        }

        fn players(&self) -> Vec<ActorId> {
            self.players.clone()
        }

        fn reserve_netquake_client(&mut self, client: &ClientId) -> Result<ActorId, String> {
            if let Some(message) = &self.reserve_err {
                return Err(message.clone());
            }
            self.reserve_ok
                .get(client)
                .cloned()
                .ok_or_else(|| "no reservation".to_string())
        }

        fn disconnect_player(&mut self, actor: &ActorId) {
            self.disconnects.push(actor.clone());
        }

        fn admit_player(&mut self, _client: &ClientId) -> PlayerAdmission {
            PlayerAdmission {
                actor: self.admit_actor.clone().expect("admit actor"),
                view_height: 22.0,
            }
        }

        fn capture_persistent(&self) -> Vec<SimulationPresentationEvent> {
            self.persistent.clone()
        }

        fn light_style(&self, index: u32) -> String {
            format!("style{index}")
        }

        fn toggle_q1_pause(&mut self, _actor: &ActorId) -> String {
            self.paused = !self.paused;
            if self.paused {
                "paused".to_string()
            } else {
                "unpaused".to_string()
            }
        }

        fn notify_client_event(&mut self, kind: ModClientEventKind, actor: &ActorId) {
            self.notified.push((kind, actor.clone()));
        }

        fn player_view(&self, _actor: &ActorId) -> PlayerView {
            PlayerView {
                client_view_offset_delta: None,
                blend: None,
                damage_blend: None,
                origin: self.view_origin,
                angles: vec3(0.0, 0.0, 0.0),
                view_height: 22.0,
                kick_angles: None,
                field_of_view: None,
                foreign_character_death: false,
                pitch_drift: None,
            }
        }

        fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]) {
            self.commands.push((actor.clone(), name.to_string(), args.to_vec()));
        }
    }

    struct StubContent {
        plan: ResolvedMountPlan,
    }

    impl Q1QuakeCHostContent for StubContent {
        fn for_content(&self, _content: &ContentId) -> Result<MountedContent, MountError> {
            open_mount_plan(&self.plan, OpenMountOptions::default())
        }
    }

    struct StubSession {
        owner: IdentityOwner,
        generations: HashMap<u32, u32>,
        connected: Vec<(ClientId, NetClientOrigin)>,
        closed: Vec<ClientId>,
    }

    impl Q1QuakeCHostSession for StubSession {
        fn create_client(&mut self, slot: u32) -> Result<ClientId, WorldError> {
            let generation = self.generations.get(&slot).copied().unwrap_or(0);
            self.generations.insert(slot, generation + 1);
            Ok(self.owner.client(slot, generation))
        }

        fn connect_client(&mut self, client: &ClientId, origin: NetClientOrigin) {
            self.connected.push((client.clone(), origin));
        }

        fn close_client(&mut self, client: &ClientId) -> Result<(), String> {
            self.closed.push(client.clone());
            Ok(())
        }
    }

    type StubHost = Q1QuakeCNetHost<StubSim, StubSession, StubGame>;

    struct Fixture {
        owner: IdentityOwner,
        actor1: ActorId,
        actor2: ActorId,
        owned1: OwnedActor,
        owned2: OwnedActor,
        client0: ClientId,
        client1: ClientId,
        client9: ClientId,
    }

    impl Fixture {
        fn new() -> Self {
            let owner = IdentityOwner::create("q1-host-test").unwrap();
            let actor1 = owner.actor(1, 1);
            let actor2 = owner.actor(2, 1);
            let provider = ProviderId::new("q1", "game");
            let owned1 = owner.owned_actor(&actor1, provider.clone()).unwrap();
            let owned2 = owner.owned_actor(&actor2, provider).unwrap();
            let client0 = owner.client(0, 0);
            let client1 = owner.client(1, 0);
            let client9 = owner.client(9, 0);
            Self {
                owner,
                actor1,
                actor2,
                owned1,
                owned2,
                client0,
                client1,
                client9,
            }
        }
    }

    fn fixture_dir(files: &[&str]) -> ResolvedMountPlan {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("qa-sim-net-q1-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for file in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"stub").unwrap();
        }
        let mount_id = MountId("mount:test:loose".to_string());
        let mount = ContentMount::Loose(LooseMount {
            identity: MountIdentity {
                id: mount_id.clone(),
                content: ContentId("test:content".to_string()),
                generation: 1,
            },
            root_path: dir.to_string_lossy().into_owned(),
        });
        ResolvedMountPlan {
            id: MountPlanId("mount-plan:test:1".to_string()),
            mounts: vec![mount],
            default_order: vec![mount_id],
            prefix_orders: Vec::new(),
        }
    }

    struct Ids {
        actor1: ActorId,
        actor2: ActorId,
        owned2: OwnedActor,
        client0: ClientId,
        client1: ClientId,
        client9: ClientId,
    }

    fn standard_game(fixture: &Fixture) -> StubGame {
        let mut game = StubGame::with_entities();
        game.source_slots.insert(fixture.actor1.clone(), 1);
        game.source_slots.insert(fixture.actor2.clone(), 2);
        game.active.push(fixture.actor1.clone());
        game.actors.insert(fixture.client0.clone(), fixture.actor1.clone());
        game.reserved.insert(fixture.client0.clone(), fixture.owned1.clone());
        game.infos.insert(
            fixture.client0.clone(),
            HashMap::from([
                ("name".to_string(), "player".to_string()),
                ("topcolor".to_string(), "2".to_string()),
                ("bottomcolor".to_string(), "5".to_string()),
            ]),
        );
        game
    }

    fn standard_sim(fixture: &Fixture) -> StubSim {
        let mut sim = StubSim::new();
        sim.reserve_ok.insert(fixture.client0.clone(), fixture.actor1.clone());
        sim.admit_actor = Some(fixture.actor1.clone());
        sim
    }

    fn host_with(
        fixture: Fixture,
        game: StubGame,
        sim: StubSim,
        files: &[&str],
        protocol: NqProfile,
    ) -> (StubHost, Ids) {
        let plan = fixture_dir(files);
        let ids = Ids {
            actor1: fixture.actor1.clone(),
            actor2: fixture.actor2.clone(),
            owned2: fixture.owned2.clone(),
            client0: fixture.client0.clone(),
            client1: fixture.client1.clone(),
            client9: fixture.client9.clone(),
        };
        let content = StubContent { plan };
        let session = StubSession {
            owner: fixture.owner,
            generations: HashMap::new(),
            connected: Vec::new(),
            closed: Vec::new(),
        };
        let options = Q1QuakeCServerOptions {
            session,
            rejects: None,
            simulation: sim,
            content,
            protocol,
            max_precache: 256,
            print: Rc::new(|_| {}),
            game: Some(game),
        };
        let host = create_quakec_netquake_host(options).unwrap();
        assert!(host.game.attached);
        (host, ids)
    }

    fn standard_host() -> (StubHost, Ids) {
        let fixture = Fixture::new();
        let game = standard_game(&fixture);
        let sim = standard_sim(&fixture);
        host_with(
            fixture,
            game,
            sim,
            &["progs/player.mdl", "progs/wizard.mdl", "sound/weapons/rocket1.wav"],
            NqProfile::Netquake,
        )
    }

    fn output() -> SimulationOutput {
        SimulationOutput {
            snapshot: WorldSnapshot {
                frame: FrameContext {
                    frame: 7,
                    time: SourceTime::Seconds(1.0),
                    elapsed: SourceTime::Seconds(0.1),
                    phase: FramePhase::FrameEntry,
                },
                actors: Vec::new(),
                bodies: Vec::new(),
                inventories: Vec::new(),
            },
            events: Vec::new(),
        }
    }

    fn presentation(event: SourcePresentationEvent) -> SimulationPresentationEvent {
        SimulationPresentationEvent {
            event,
            owner: None,
            recipient: None,
            sequence: 0,
            content: ContentId("q1:id1:base:1".to_string()),
            seconds: 0.0,
            source_entity: None,
        }
    }

    fn loopback() -> NetworkAddress {
        NetworkAddress::Loopback { id: "test".to_string() }
    }

    #[test]
    fn rejects_without_netquake_source() {
        let fixture = Fixture::new();
        let plan = fixture_dir(&[]);
        let options = Q1QuakeCServerOptions {
            session: StubSession {
                owner: fixture.owner,
                generations: HashMap::new(),
                connected: Vec::new(),
                closed: Vec::new(),
            },
            rejects: None,
            simulation: StubSim::new(),
            content: StubContent { plan },
            protocol: NqProfile::Netquake,
            max_precache: 256,
            print: Rc::new(|_| {}),
            game: None::<StubGame>,
        };
        assert!(matches!(
            create_quakec_netquake_host(options).unwrap_err(),
            Q1QuakeCHostError::NotNetquakeSource
        ));
        let fixture = Fixture::new();
        let mut game = standard_game(&fixture);
        game.kind = QuakeCSourceKind::Quakeworld;
        let sim = standard_sim(&fixture);
        let plan = fixture_dir(&[]);
        let options = Q1QuakeCServerOptions {
            session: StubSession {
                owner: fixture.owner,
                generations: HashMap::new(),
                connected: Vec::new(),
                closed: Vec::new(),
            },
            rejects: None,
            simulation: sim,
            content: StubContent { plan },
            protocol: NqProfile::Netquake,
            max_precache: 256,
            print: Rc::new(|_| {}),
            game: Some(game),
        };
        assert!(matches!(
            create_quakec_netquake_host(options).unwrap_err(),
            Q1QuakeCHostError::NotNetquakeSource
        ));
    }

    #[test]
    fn rejects_precache_overflow() {
        let fixture = Fixture::new();
        let mut game = standard_game(&fixture);
        game.models = (0..256).map(|index| format!("progs/{index}.mdl")).collect();
        let plan = fixture_dir(&[]);
        let options = Q1QuakeCServerOptions {
            session: StubSession {
                owner: fixture.owner,
                generations: HashMap::new(),
                connected: Vec::new(),
                closed: Vec::new(),
            },
            rejects: None,
            simulation: StubSim::new(),
            content: StubContent { plan },
            protocol: NqProfile::Netquake,
            max_precache: 256,
            print: Rc::new(|_| {}),
            game: Some(game),
        };
        assert!(matches!(
            create_quakec_netquake_host(options).unwrap_err(),
            Q1QuakeCHostError::PrecachesExceedProtocol
        ));
    }

    #[test]
    fn verifies_precache_resources() {
        let fixture = Fixture::new();
        let game = standard_game(&fixture);
        let sim = standard_sim(&fixture);
        let plan = fixture_dir(&["progs/player.mdl", "sound/weapons/rocket1.wav"]);
        let content = StubContent { plan };
        let session = StubSession {
            owner: fixture.owner,
            generations: HashMap::new(),
            connected: Vec::new(),
            closed: Vec::new(),
        };
        let options = Q1QuakeCServerOptions {
            session,
            rejects: None,
            simulation: sim,
            content,
            protocol: NqProfile::Netquake,
            max_precache: 256,
            print: Rc::new(|_| {}),
            game: Some(game),
        };
        let error = create_quakec_netquake_host(options).unwrap_err();
        assert!(matches!(error, Q1QuakeCHostError::MissingModel(_)));

        let fixture = Fixture::new();
        let mut game = standard_game(&fixture);
        game.models.push("*0".to_string());
        game.sounds = vec!["missing.wav".to_string()];
        let sim = standard_sim(&fixture);
        let plan = fixture_dir(&["progs/player.mdl", "progs/wizard.mdl"]);
        let content = StubContent { plan };
        let session = StubSession {
            owner: fixture.owner,
            generations: HashMap::new(),
            connected: Vec::new(),
            closed: Vec::new(),
        };
        let options = Q1QuakeCServerOptions {
            session,
            rejects: None,
            simulation: sim,
            content,
            protocol: NqProfile::Netquake,
            max_precache: 256,
            print: Rc::new(|_| {}),
            game: Some(game),
        };
        let error = create_quakec_netquake_host(options).unwrap_err();
        assert!(matches!(error, Q1QuakeCHostError::MissingSound(_)));

        let (host, _ids) = standard_host();
        assert_eq!(
            host.simulation.resources,
            vec![("q1:test:content:1".to_string(), "sound/weapons/rocket1.wav".to_string())]
        );
    }

    #[test]
    fn entities_baseline_narrow_and_wide() {
        let fixture = Fixture::new();
        let mut game = standard_game(&fixture);
        game.max_clients = 1;
        let sim = standard_sim(&fixture);
        let (host, ids) = host_with(
            fixture,
            game,
            sim,
            &["progs/player.mdl", "progs/wizard.mdl", "sound/weapons/rocket1.wav"],
            NqProfile::Netquake,
        );
        let player = Q1ApplicationPlayer {
            client: ids.client0.clone(),
            actor: ids.actor1.clone(),
            source_entity: 1,
        };
        let state = host.game_state(&player).unwrap();
        assert_eq!(state.info.protocol, NqProfile::Netquake);
        assert_eq!(state.info.max_clients, 1);
        assert_eq!(state.info.game_type, 1);
        assert_eq!(state.info.level, "dm3");
        assert_eq!(
            state.info.models,
            vec!["progs/player.mdl".to_string(), "progs/wizard.mdl".to_string()]
        );
        assert_eq!(state.info.sounds, vec!["weapons/rocket1.wav".to_string()]);
        let one = &state.baselines[&1];
        assert_eq!(one.model_index, 1);
        assert_eq!(one.frame, 5);
        assert_eq!(one.color_map, 1);
        assert_eq!(one.alpha, 0);
        assert_eq!(one.scale, 16);
        assert!(!one.step);
        assert_eq!(one.effects, 0);
        let two = &state.baselines[&2];
        assert_eq!(two.model_index, 2);
        assert_eq!(two.frame, 0);
        assert_eq!(two.color_map, 0);
        assert_eq!(host.map_name(), "dm3");
        assert_eq!(host.max_clients(), 1);
        assert_eq!(host.protocol(), NqProfile::Netquake);

        let fixture = Fixture::new();
        let mut game = standard_game(&fixture);
        game.max_clients = 1;
        let sim = standard_sim(&fixture);
        let (wide, ids) = host_with(
            fixture,
            game,
            sim,
            &["progs/player.mdl", "progs/wizard.mdl", "sound/weapons/rocket1.wav"],
            NqProfile::Fitzquake,
        );
        let player = Q1ApplicationPlayer {
            client: ids.client0.clone(),
            actor: ids.actor1.clone(),
            source_entity: 1,
        };
        let state = wide.game_state(&player).unwrap();
        let two = &state.baselines[&2];
        assert_eq!(two.frame, 300);
        assert_eq!(two.alpha, entalpha_encode(0.5));
        assert_eq!(two.scale, 16);
        let one = &state.baselines[&1];
        assert_eq!(one.alpha, 0);
        assert_eq!(one.scale, 16);
        assert!(!one.step);

        let fixture = Fixture::new();
        let mut game = standard_game(&fixture);
        game.max_clients = 1;
        let sim = standard_sim(&fixture);
        let (rmq, ids) = host_with(
            fixture,
            game,
            sim,
            &["progs/player.mdl", "progs/wizard.mdl", "sound/weapons/rocket1.wav"],
            NqProfile::Rmq { flags: 0 },
        );
        let player = Q1ApplicationPlayer {
            client: ids.client0.clone(),
            actor: ids.actor1.clone(),
            source_entity: 1,
        };
        let state = rmq.game_state(&player).unwrap();
        assert_eq!(state.baselines[&2].scale, entscale_encode(2.0));
    }

    #[test]
    fn client_data_items_and_rogue_bits() {
        let (mut host, ids) = standard_host();
        let admission = host.admit(&loopback()).unwrap();
        let Q1ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        host.observe(&output(), &[]).unwrap();
        let frame = host.frame(&player, &output()).unwrap();
        let Q1NetQuakeMessage::ClientData { data, weapon_alpha } = &frame.messages[0] else {
            panic!("expected client-data, got {:?}", frame.messages[0].kind());
        };
        assert_eq!(*weapon_alpha, 0);
        assert_eq!(data.items, 3 | (2 << 28));
        assert_eq!(data.active_weapon, 8);
        assert_eq!(data.view_height, 22.0);
        assert!(data.on_ground);
        assert!(!data.in_water);
        assert_eq!(data.weapon_model, 1);
        assert_eq!(data.health, 100.0);
        assert_eq!(data.shells, 10.0);
        assert_eq!(ids.actor1, player.actor);

        let fixture = Fixture::new();
        let mut game = standard_game(&fixture);
        game.map_content = ContentId("q1:rogue:pack:1".to_string());
        game.fields.insert("items2".to_string(), 32);
        game.floats.insert((1, 32), 5.0);
        let sim = standard_sim(&fixture);
        let (mut host, _ids) = host_with(
            fixture,
            game,
            sim,
            &["progs/player.mdl", "progs/wizard.mdl", "sound/weapons/rocket1.wav"],
            NqProfile::Netquake,
        );
        let admission = host.admit(&loopback()).unwrap();
        let Q1ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        host.observe(&output(), &[]).unwrap();
        let frame = host.frame(&player, &output()).unwrap();
        let Q1NetQuakeMessage::ClientData { data, .. } = &frame.messages[0] else {
            panic!("expected client-data");
        };
        assert_eq!(data.items, 3 | (5 << 23));
        assert_eq!(data.active_weapon, 3);
    }

    #[test]
    fn admit_spawn_frame_roundtrip() {
        let (mut host, ids) = standard_host();
        let admission = host.admit(&loopback()).unwrap();
        let Q1ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        assert_eq!(player.client, ids.client0);
        assert_eq!(player.actor, ids.actor1);
        assert_eq!(player.source_entity, 1);
        assert_eq!(
            host.session.connected,
            vec![(ids.client0.clone(), NetClientOrigin::Loopback)]
        );
        host.observe(&output(), &[]).unwrap();
        let spawn = host.spawn(&player).unwrap();
        assert!(matches!(spawn[0], Q1NetQuakeMessage::Pause { paused: false }));
        assert!(matches!(spawn[1], Q1NetQuakeMessage::Time { seconds: 12.5 }));
        assert!(matches!(
            &spawn[2],
            Q1NetQuakeMessage::LightStyle { index: 0, value } if value == "style0"
        ));
        assert!(matches!(
            &spawn[65],
            Q1NetQuakeMessage::LightStyle { index: 63, value } if value == "style63"
        ));
        assert!(matches!(
            &spawn[66],
            Q1NetQuakeMessage::NamedSlot { kind: Q1NamedSlotKind::Name, slot: 0, value }
                if value == "player"
        ));
        assert!(matches!(
            &spawn[67],
            Q1NetQuakeMessage::NumberedSlot {
                kind: Q1NumberedSlotKind::Frags,
                value: 7.0,
                ..
            }
        ));
        assert!(matches!(
            &spawn[68],
            Q1NetQuakeMessage::NumberedSlot {
                kind: Q1NumberedSlotKind::Colors,
                value: 37.0,
                ..
            }
        ));
        assert!(matches!(
            &spawn[69..73],
            [
                Q1NetQuakeMessage::Stat { index: 11, value: 5.0 },
                Q1NetQuakeMessage::Stat { index: 12, value: 20.0 },
                Q1NetQuakeMessage::Stat { index: 13, value: 1.0 },
                Q1NetQuakeMessage::Stat { index: 14, value: 7.0 },
            ]
        ));
        assert!(matches!(&spawn[73], Q1NetQuakeMessage::SetAngle { .. }));
        assert!(matches!(&spawn[74], Q1NetQuakeMessage::ClientData { .. }));
        assert_eq!(spawn.len(), 75);

        host.game.damage = Some(Q1NetDamage {
            armor: 4.0,
            blood: 6.0,
            source: vec3(1.0, 1.0, 1.0),
        });
        let frame = host.frame(&player, &output()).unwrap();
        assert_eq!(frame.seconds, 12.5);
        assert!(matches!(
            &frame.messages[0],
            Q1NetQuakeMessage::Damage {
                armor: 4.0,
                blood: 6.0,
                ..
            }
        ));
        assert!(matches!(&frame.messages[1], Q1NetQuakeMessage::ClientData { .. }));
        assert_eq!(frame.entities.len(), 2);
        assert_eq!(frame.entities[0].number, 1);
        assert!(frame.entities[0].step);
        assert_eq!(frame.entities[1].number, 2);
        assert!(!frame.entities[1].step);
        assert_eq!(frame.reliable.len(), 3);
        assert!(frame.datagram.is_empty());

        host.simulation.scene.visible = false;
        let frame = host.frame(&player, &output()).unwrap();
        assert_eq!(frame.entities.len(), 1);
        assert_eq!(frame.entities[0].number, 1);
    }

    #[test]
    fn admit_rejects_when_full_or_unreserved() {
        let fixture = Fixture::new();
        let mut game = standard_game(&fixture);
        game.max_clients = 1;
        let sim = standard_sim(&fixture);
        let (mut host, _ids) = host_with(
            fixture,
            game,
            sim,
            &["progs/player.mdl", "progs/wizard.mdl", "sound/weapons/rocket1.wav"],
            NqProfile::Netquake,
        );
        assert!(matches!(
            host.admit(&loopback()).unwrap(),
            Q1ApplicationAdmission::Accepted { .. }
        ));
        let admission = host.admit(&loopback()).unwrap();
        assert!(matches!(
            admission,
            Q1ApplicationAdmission::Rejected { reason } if reason == "Server is full"
        ));

        let (mut host, ids) = standard_host();
        host.simulation.reserve_err = Some("boom".to_string());
        let error = host.admit(&loopback()).unwrap_err();
        assert_eq!(error.to_string(), "boom");
        assert_eq!(host.session.closed, vec![ids.client0.clone()]);
    }

    #[test]
    fn admit_replaces_missing_source_slot() {
        let (mut host, ids) = standard_host();
        host.game.source_slots.remove(&ids.actor1);
        let error = host.admit(&loopback()).unwrap_err();
        assert!(matches!(error, Q1QuakeCHostError::NoSourceSlot));
        assert_eq!(host.game.disconnects, vec![ids.actor1.clone()]);
        assert_eq!(host.session.closed, vec![ids.client0.clone()]);
    }

    #[test]
    fn spawn_rejects_replaced_identity() {
        let (mut host, ids) = standard_host();
        host.game.active.clear();
        host.simulation.admit_actor = Some(ids.actor2.clone());
        let player = Q1ApplicationPlayer {
            client: ids.client0.clone(),
            actor: ids.actor1.clone(),
            source_entity: 1,
        };
        let error = host.spawn(&player).unwrap_err();
        assert!(matches!(error, Q1QuakeCHostError::SpawnReplacedIdentity));
    }

    #[test]
    fn observe_tracks_board_pause_and_events() {
        let (mut host, ids) = standard_host();
        let admission = host.admit(&loopback()).unwrap();
        let Q1ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        host.observe(&output(), &[]).unwrap();
        let frame = host.frame(&player, &output()).unwrap();
        assert_eq!(frame.reliable.len(), 3);
        host.observe(&output(), &[]).unwrap();
        let frame = host.frame(&player, &output()).unwrap();
        assert!(frame.reliable.is_empty());

        host.game.strings.push("renamed".to_string());
        let netname = host.game.offset("netname");
        host.game.ints.insert((1, netname), 5);
        host.observe(&output(), &[]).unwrap();
        let frame = host.frame(&player, &output()).unwrap();
        assert_eq!(frame.reliable.len(), 1);
        assert!(matches!(
            &frame.reliable[0],
            Q1NetQuakeMessage::NamedSlot { value, .. } if value == "renamed"
        ));

        host.simulation.paused = true;
        host.observe(&output(), &[]).unwrap();
        let frame = host.frame(&player, &output()).unwrap();
        assert!(matches!(&frame.reliable[0], Q1NetQuakeMessage::Pause { paused: true }));

        let events = vec![
            presentation(SourcePresentationEvent::ViewReset {
                reason: ViewResetReason::Spawn,
                actor: ids.actor1.clone(),
                angles: vec3(0.0, 180.0, 0.0),
            }),
            presentation(SourcePresentationEvent::Q1(Q1Event::Particles {
                origin: vec3(0.0, 0.0, 0.0),
                direction: vec3(10.0, -20.0, 0.5),
                color: 73,
                count: 9,
            })),
            presentation(SourcePresentationEvent::Q1(Q1Event::Lightstyle {
                style: 4,
                pattern: "mmnmmomm".to_string(),
            })),
        ];
        host.observe(&output(), &events).unwrap();
        let frame = host.frame(&player, &output()).unwrap();
        assert!(matches!(&frame.reliable[0], Q1NetQuakeMessage::SetAngle { .. }));
        assert!(matches!(
            &frame.reliable[1],
            Q1NetQuakeMessage::LightStyle { index: 4, .. }
        ));
        assert_eq!(frame.datagram.len(), 1);
        let Q1NetQuakeMessage::Particle {
            direction,
            count,
            color,
            ..
        } = &frame.datagram[0]
        else {
            panic!("expected particle");
        };
        assert_eq!(*direction, vec3(7.9375, -8.0, 0.5));
        assert_eq!(*count, 9);
        assert_eq!(*color, 73);

        host.disconnect(&player, "bye").unwrap();
        host.observe(&output(), &[]).unwrap();
        let frame = host.frame(&player, &output()).unwrap();
        assert_eq!(frame.reliable.len(), 3);
        assert!(matches!(
            &frame.reliable[0],
            Q1NetQuakeMessage::NamedSlot { value, .. } if value.is_empty()
        ));
    }

    #[test]
    fn source_messages_route_and_reject() {
        let (mut host, _ids) = standard_host();
        host.game.drain = vec![Q1RoutedBatch {
            messages: vec![Q1NetQuakeMessage::Time { seconds: 1.0 }],
            destination: QcMessageDestination::Multicast {
                origin: vec3(0.0, 0.0, 0.0),
                visibility: VisibilityScope::All,
                reliable: false,
            },
        }];
        assert!(matches!(
            host.observe(&output(), &[]).unwrap_err(),
            Q1QuakeCHostError::UnexpectedDestination
        ));

        let (mut host, _ids) = standard_host();
        host.game.drain = vec![Q1RoutedBatch {
            messages: vec![Q1NetQuakeMessage::Entity {
                state: Q1ExtendedEntityState {
                    number: 1,
                    origin: vec3(0.0, 0.0, 0.0),
                    angles: vec3(0.0, 0.0, 0.0),
                    model_index: 1,
                    frame: 0,
                    color_map: 0,
                    skin: 0,
                    effects: 0,
                    alpha: 0,
                    scale: 16,
                    lerp_finish_seconds: 0.0,
                    step: false,
                },
            }],
            destination: QcMessageDestination::Broadcast { reliable: false },
        }];
        assert!(matches!(
            host.observe(&output(), &[]).unwrap_err(),
            Q1QuakeCHostError::EntitySnapshot
        ));

        let (mut host, ids) = standard_host();
        let admission = host.admit(&loopback()).unwrap();
        let Q1ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        host.game.drain = vec![
            Q1RoutedBatch {
                messages: vec![Q1NetQuakeMessage::Time { seconds: 1.0 }],
                destination: QcMessageDestination::Broadcast { reliable: false },
            },
            Q1RoutedBatch {
                messages: vec![Q1NetQuakeMessage::Pause { paused: false }],
                destination: QcMessageDestination::Client {
                    actor: ids.actor1.clone(),
                },
            },
            Q1RoutedBatch {
                messages: vec![Q1NetQuakeMessage::Pause { paused: true }],
                destination: QcMessageDestination::Client {
                    actor: ids.actor2.clone(),
                },
            },
        ];
        host.observe(&output(), &[]).unwrap();
        let frame = host.frame(&player, &output()).unwrap();
        assert!(frame
            .datagram
            .iter()
            .any(|message| matches!(message, Q1NetQuakeMessage::Time { .. })));
        assert_eq!(
            frame
                .reliable
                .iter()
                .filter(|message| matches!(message, Q1NetQuakeMessage::Pause { .. }))
                .count(),
            1
        );
        assert!(frame
            .reliable
            .iter()
            .all(|message| !matches!(message, Q1NetQuakeMessage::Pause { paused: true })));
    }

    #[test]
    fn command_verbs_route() {
        let (mut host, ids) = standard_host();
        let admission = host.admit(&loopback()).unwrap();
        let Q1ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        host.command(&player, "pause", &[]).unwrap();
        host.observe(&output(), &[]).unwrap();
        let frame = host.frame(&player, &output()).unwrap();
        assert!(frame.reliable.iter().any(|message| matches!(
            message,
            Q1NetQuakeMessage::Text { kind: Q1TextMessageKind::Print, text }
                if text == "paused"
        )));

        host.command(&player, "name", &["averylongplayernameexceeding".to_string()])
            .unwrap();
        let info = host.game.client_info(&ids.client0);
        assert_eq!(info["name"], "averylongplayer");
        host.command(&player, "name", &[]).unwrap();
        assert_eq!(host.game.client_info(&ids.client0)["name"], "unconnected");

        host.command(&player, "color", &["3".to_string(), "7".to_string()])
            .unwrap();
        let info = host.game.client_info(&ids.client0);
        assert_eq!(info["topcolor"], "3");
        assert_eq!(info["bottomcolor"], "7");
        let team = host.game.offset("team");
        assert!(host.game.set_floats.contains(&(1, team, 8.0)));
        host.command(&player, "color", &["99".to_string()]).unwrap();
        let info = host.game.client_info(&ids.client0);
        assert_eq!(info["topcolor"], "3");
        assert_eq!(info["bottomcolor"], "3");
        assert_eq!(
            host.simulation
                .notified
                .iter()
                .filter(|(kind, _)| *kind == ModClientEventKind::Userinfo)
                .count(),
            4
        );

        host.command(&player, "say", &["hello".to_string(), "world".to_string()])
            .unwrap();
        host.observe(&output(), &[]).unwrap();
        let frame = host.frame(&player, &output()).unwrap();
        assert!(frame.reliable.iter().any(|message| matches!(
            message,
            Q1NetQuakeMessage::Text { text, .. }
                if text == "\u{1}player: hello world\n"
        )));

        host.command(&player, "god", &[]).unwrap();
        assert_eq!(
            host.simulation.commands,
            vec![(ids.actor1.clone(), "god".to_string(), Vec::new())]
        );
    }

    #[test]
    fn say_team_filters_by_teamplay() {
        let fixture = Fixture::new();
        let mut game = standard_game(&fixture);
        game.cvars.set("teamplay", "1", true).unwrap();
        let mut sim = standard_sim(&fixture);
        sim.reserve_ok.insert(fixture.client1.clone(), fixture.actor2.clone());
        let (mut host, ids) = host_with(
            fixture,
            game,
            sim,
            &["progs/player.mdl", "progs/wizard.mdl", "sound/weapons/rocket1.wav"],
            NqProfile::Netquake,
        );
        let Q1ApplicationAdmission::Accepted { player: one } = host.admit(&loopback()).unwrap() else {
            panic!("expected admission");
        };
        let remote = NetworkAddress::Ipv4 {
            host: [192, 168, 0, 2],
            port: 26000,
        };
        let Q1ApplicationAdmission::Accepted { player: two } = host.admit(&remote).unwrap() else {
            panic!("expected admission");
        };
        assert_eq!(two.client, ids.client1);
        assert_eq!(
            host.session.connected[1],
            (ids.client1.clone(), NetClientOrigin::Remote)
        );
        let team = host.game.offset("team");
        host.game.floats.insert((2, team), 2.0);
        host.command(&one, "say_team", &["go".to_string()]).unwrap();
        host.observe(&output(), &[]).unwrap();
        let first = host.frame(&one, &output()).unwrap();
        assert_eq!(
            first
                .reliable
                .iter()
                .filter(|message| matches!(message, Q1NetQuakeMessage::Text { .. }))
                .count(),
            1
        );
        host.game.active.push(ids.actor2.clone());
        let second = host.frame(&two, &output()).unwrap();
        assert!(!second
            .reliable
            .iter()
            .any(|message| matches!(message, Q1NetQuakeMessage::Text { .. })));

        host.command(&one, "say", &["go".to_string()]).unwrap();
        host.observe(&output(), &[]).unwrap();
        let second = host.frame(&two, &output()).unwrap();
        assert!(second
            .reliable
            .iter()
            .any(|message| matches!(message, Q1NetQuakeMessage::Text { .. })));
    }

    #[test]
    fn input_builds_actor_command() {
        let (host, ids) = standard_host();
        let player = Q1ApplicationPlayer {
            client: ids.client0.clone(),
            actor: ids.actor1.clone(),
            source_entity: 1,
        };
        let command = Q1UserCommand {
            acknowledged_server_time_seconds: 3.0,
            view_angles: vec3(0.0, 90.0, 0.0),
            forward_move: 100.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 3,
            impulse: 9,
        };
        let actor = host.input(&player, &command, 42);
        assert_eq!(actor.actor, ids.actor1);
        assert_eq!(
            actor.source,
            CommandSource::Remote {
                client: ids.client0.clone()
            }
        );
        assert_eq!(actor.sequence, 42);
        assert_eq!(
            actor.command,
            UserCommand::Q1Netquake {
                acknowledged_server_time_seconds: 3.0,
                view_angles: [0.0, 90.0, 0.0],
                forward_move: 100.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 3.0,
                impulse: 9.0,
            }
        );
        assert_eq!(actor.arsenal, None);
    }

    #[test]
    fn signon_carries_persistent_records() {
        let (mut host, ids) = standard_host();
        host.game.signon = vec![Q1NetQuakeMessage::Time { seconds: 1.0 }];
        host.simulation.persistent = vec![
            presentation(SourcePresentationEvent::Q1(Q1Event::Ambient {
                origin: vec3(1.0, 2.0, 3.0),
                path: "weapons/rocket1.wav".to_string(),
                volume: 0.5,
                attenuation: 1.0,
            })),
            presentation(SourcePresentationEvent::Q1(Q1Event::StaticModel {
                path: "progs/wizard.mdl".to_string(),
                frame: 2,
                color_map: 0,
                skin: 1,
                origin: vec3(4.0, 5.0, 6.0),
                angles: vec3(0.0, 0.0, 0.0),
            })),
            presentation(SourcePresentationEvent::Q1(Q1Event::Message {
                player: ids.actor1.clone(),
                text: "ignored".to_string(),
                center: false,
                args: None,
                parts: None,
            })),
        ];
        let player = Q1ApplicationPlayer {
            client: ids.client0.clone(),
            actor: ids.actor1.clone(),
            source_entity: 1,
        };
        let state = host.game_state(&player).unwrap();
        assert_eq!(state.signon.len(), 3);
        assert!(matches!(
            &state.signon[1],
            Q1NetQuakeMessage::Sound {
                kind: Q1SoundKind::StaticSound,
                entity: 0,
                channel: 0,
                index: 1,
                volume: 127,
                ..
            }
        ));
        assert!(matches!(
            &state.signon[2],
            Q1NetQuakeMessage::Static { state } if state.model_index == 2 && state.skin == 1
        ));

        host.game.signon = vec![Q1NetQuakeMessage::PromptClear];
        host.simulation.persistent = vec![presentation(SourcePresentationEvent::Q1(Q1Event::Ambient {
            origin: vec3(0.0, 0.0, 0.0),
            path: "missing.wav".to_string(),
            volume: 1.0,
            attenuation: 1.0,
        }))];
        assert!(matches!(
            host.game_state(&player).unwrap_err(),
            Q1QuakeCHostError::UnprecachedResource(_)
        ));
    }

    #[test]
    fn carried_and_disconnect_paths() {
        let (mut host, ids) = standard_host();
        assert!(matches!(
            host.carried_player(&ids.client9).unwrap_err(),
            Q1QuakeCHostError::CarriedNotReserved
        ));
        let player = host.carried_player(&ids.client0).unwrap();
        assert_eq!(player.actor, ids.actor1);
        assert_eq!(player.source_entity, 1);
        host.disconnect(&player, "bye").unwrap();
        assert_eq!(host.simulation.disconnects, vec![ids.actor1.clone()]);
        assert!(host.clients.is_empty());
        assert_eq!(host.session.closed, vec![ids.client0.clone()]);

        let (mut host, ids) = standard_host();
        host.game.reserved.insert(ids.client9.clone(), ids.owned2.clone());
        let player = Q1ApplicationPlayer {
            client: ids.client9.clone(),
            actor: ids.actor2.clone(),
            source_entity: 2,
        };
        host.disconnect(&player, "bye").unwrap();
        assert!(host.simulation.disconnects.is_empty());
        assert_eq!(host.game.disconnects, vec![ids.actor2.clone()]);
    }

    #[test]
    fn wire_support_helpers_and_gametype() {
        let (mut host, _ids) = standard_host();
        assert!(matches!(host.supports_source_wire(), WireAdmission::Supported));
        host.simulation.recipe.weapons = vec!["q2:railgun".to_string()];
        let admission = host.supports_source_wire();
        assert!(matches!(
            admission,
            WireAdmission::Unsupported { reasons }
                if reasons == vec!["Mixed composition requires unified serialization".to_string()]
        ));

        assert!(!host.rejected(&loopback()));
        host.rejects = Some(Rc::new(|_| true));
        assert!(host.rejected(&loopback()));

        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let pushed = Rc::clone(&seen);
        host.print = Rc::new(move |text| pushed.borrow_mut().push(text.to_string()));
        host.print("hi");
        assert_eq!(*seen.borrow(), vec!["hi".to_string()]);

        host.game.cvars.set("deathmatch", "0", true).unwrap();
        host.game.strings.push("start".to_string());
        let message = host.game.offset("message");
        host.game.ints.insert((0, message), 5);
        let player = Q1ApplicationPlayer {
            client: host.simulation.reserve_ok.keys().next().cloned().unwrap(),
            actor: host.simulation.reserve_ok.values().next().cloned().unwrap(),
            source_entity: 1,
        };
        let state = host.game_state(&player).unwrap();
        assert_eq!(state.info.game_type, 0);
        assert_eq!(state.info.level, "start");

        host.game.fields.remove("modelindex");
        assert!(matches!(
            host.game_state(&player).unwrap_err(),
            Q1QuakeCHostError::MissingField(_)
        ));
    }
}
