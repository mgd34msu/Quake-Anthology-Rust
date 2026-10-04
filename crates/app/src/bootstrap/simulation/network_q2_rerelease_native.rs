//! Rerelease Quake II guest projection through the native server channel.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/network-q2-rerelease-native.ts`.
//!
//! This file is self-contained like its donor: shared application shapes are
//! mirrored here with canonical-home notes instead of importing sibling
//! ports. The async content fetch and download construction
//! (`LoadedApplicationContent.forContent`, `createQ2ApplicationDownloads`
//! from donor `src/app/bootstrap/network/q2-downloads.ts`) live behind the
//! synchronous [`RereleaseGuestHostContent`] seam and the injected
//! `downloads` handle; bootstrap ports are sync.
//!
//! The donor `admit` reads the split seat from `request.splitSeat` (donor
//! Port of Quake-Anthology-TS `src/network/q2/handshake.ts`); the worktree [`Q2ConnectRequest`] has not
//! ported that field yet, so the seat travels as an explicit [`admit`]
//! parameter until the port lands.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_bots::md4::block_checksum;
use qa_content::bsp::Node;
use qa_content::catalog::CatalogError;
use qa_content::mounts::MountError;
use qa_core::cmd::{expand_command_macros, tokenize_command, CmdError, Dialect, TextMode};
use qa_core::cvar::{q2_flags, CvarError, CvarRegistry};
use qa_core::identity::{ActorId, ClientId, OwnedActor};
use qa_net::common::commands::ActorCommand;
use qa_net::common::endpoint::{address_key, NetworkAddress};
use qa_net::common::session::WireAdmission;
use qa_net::protocol::ProtocolIdentity;
use qa_net::q2::{EntityState, ServerData, Usercmd};
use qa_net::q2_adapters::{
    from_q2_entity, from_q2_player, to_q2_rerelease_command, Q2AdapterError, Q2Command, Q2Entity, Q2Player,
    Q2RereleaseEntityState, Q2RereleasePlayerState, Q2RereleaseUserCommand, Q2Vec3,
};
use qa_net::q2_net::{
    Q2ConnectRequest, Q2NetError, Q2ServerEvent, Q2ServerMessageOptions, Q2ServerMessageReader, Q2Status,
    Q2StatusPlayer, Q2Wire, Q2WireFrame,
};
use qa_net::q2_svc::encode_q2_server_event;
use qa_world::WorldError;
use thiserror::Error;

use crate::bootstrap::network::q2_layout::{q2_application_layout, Q2ApplicationLayout, Q2LayoutError};

/// Quake II application player (canonical home:
/// [`crate::bootstrap::network::types::Q2ApplicationPlayer`]).
pub use crate::bootstrap::network::types::Q2ApplicationPlayer;

/// Admission verdict (canonical home:
/// [`crate::bootstrap::network::types::Q2ApplicationAdmission`]).
pub use crate::bootstrap::network::types::Q2ApplicationAdmission;

/// Mirror of the donor `ServerDataParamsT` game-state data beyond the ported
/// `qa_net::q2::ServerData` (canonical home: `qa_net::q2`, extending
/// `ServerData` post-merge).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ServerDataParams {
    /// Ported server-data base.
    pub base: ServerData,
    /// Server state.
    pub server_state: i32,
    /// Server frames per second.
    pub server_fps: Option<f64>,
}

/// Mirror of `Q2ApplicationGameState` from donor
/// `src/app/bootstrap/network/types.ts` (canonical home:
/// `crate::bootstrap::network::types`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ApplicationGameState {
    /// Server data.
    pub data: Q2ServerDataParams,
    /// Configstrings by index.
    pub config_strings: HashMap<u32, String>,
    /// Entity baselines by entity number.
    pub baselines: HashMap<u16, EntityState>,
}

/// Mirror of `Q2ApplicationServerEvent` from donor
/// `src/app/bootstrap/network/types.ts` (canonical home:
/// `crate::bootstrap::network::types`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ApplicationServerEvent {
    /// Server event.
    pub event: Q2ServerEvent,
    /// Reliable delivery override.
    pub reliable: Option<bool>,
}

/// Guest message audience scope, mirroring donor
/// `RereleaseGuestAudience` visibility (`"all" | "phs" | "pvs"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestAudienceScope {
    /// Every client.
    All,
    /// Potentially hearable set.
    Phs,
    /// Potentially visible set.
    Pvs,
}

/// Cluster visibility scope for scene queries, mirroring the donor
/// `clusterVisible` `"pvs" | "phs"` argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClusterVisibility {
    /// Potentially visible set.
    Pvs,
    /// Potentially hearable set.
    Phs,
}

/// Mirror of `RereleaseGuestAudience` from donor
/// `src/app/bootstrap/simulation/rerelease-guest-services-contract.ts`
/// (canonical home:
/// `crate::bootstrap::simulation::rerelease_guest_services_contract`);
/// unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum RereleaseGuestAudience {
    /// One client slot.
    Unicast {
        /// Recipient slot.
        slot: u32,
    },
    /// Area multicast.
    Multicast {
        /// Multicast origin.
        origin: Q2Vec3,
        /// Multicast scope.
        scope: GuestAudienceScope,
    },
}

/// Mirror of the donor `sourceDialect` literal (`"q2-multicast-float"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RereleaseSourceDialect {
    /// Float multicast records.
    Q2MulticastFloat,
}

impl RereleaseSourceDialect {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            RereleaseSourceDialect::Q2MulticastFloat => "q2-multicast-float",
        }
    }
}

/// Mirror of `RereleaseGuestMessage` from donor
/// `src/app/bootstrap/simulation/rerelease-guest-services-contract.ts`
/// (canonical home:
/// `crate::bootstrap::simulation::rerelease_guest_services_contract`);
/// unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseGuestMessage {
    /// Message audience.
    pub audience: RereleaseGuestAudience,
    /// Source dialect.
    pub source_dialect: RereleaseSourceDialect,
    /// Reliable delivery.
    pub reliable: bool,
    /// Deduplication key.
    pub dupe_key: u32,
    /// Wire bytes.
    pub bytes: Vec<u8>,
}

/// Collected guest message with its transcoded wire form, mirroring the
/// donor `{ source, wire }` pairs.
#[derive(Debug, Clone, PartialEq)]
pub struct CollectedGuestMessage {
    /// Guest message as emitted.
    pub source: RereleaseGuestMessage,
    /// Guest message with transcoded bytes.
    pub wire: RereleaseGuestMessage,
}

/// Guest-client phase (canonical home:
/// [`super::classic_guest_world::ClassicGuestClientPhase`]).
pub use super::classic_guest_world::ClassicGuestClientPhase;

/// Connected guest client record (canonical home:
/// [`super::classic_guest_world::ClassicGuestClient`]).
pub use super::classic_guest_world::ClassicGuestClient;

/// Mirror of the donor `connect` outcome from
/// `src/app/bootstrap/simulation/rerelease-guest-world.ts` (canonical home:
/// `crate::bootstrap::simulation::rerelease_guest_world`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestConnectOutcome {
    /// Connection allowed.
    pub allowed: bool,
    /// Result userinfo.
    pub userinfo: String,
}

/// Mirror of the donor `entityInfo` result from
/// `src/app/bootstrap/simulation/rerelease-guest-services.ts` (canonical
/// home: `crate::bootstrap::simulation::rerelease_guest_services`); unify
/// post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestEntityInfo {
    /// Owning actor.
    pub actor: Option<ActorId>,
    /// Entity active.
    pub active: bool,
    /// Server flags.
    pub server_flags: i32,
    /// Entity areas.
    pub areas: [i32; 2],
    /// Entity clusters, or headnode visibility when [`None`].
    pub clusters: Option<Vec<i32>>,
    /// First cluster.
    pub first_cluster: i32,
    /// Visibility headnode.
    pub headnode: i32,
    /// Owning client slot.
    pub owner_slot: Option<u32>,
}

/// Mirror of the donor seat identity passed to `playerIdentity`, from
/// `src/app/bootstrap/simulation/network.ts` (canonical home:
/// `crate::bootstrap::simulation::network`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseSeatIdentity {
    /// Split-screen seat.
    pub seat: u32,
    /// Social identity, empty when absent.
    pub social_id: String,
}

/// Raw wire bytes with a reliability flag (canonical home:
/// [`crate::bootstrap::network::types::Q2RawServerMessage`]).
pub use crate::bootstrap::network::types::Q2RawServerMessage as Q2GuestByteMessage;

/// Mirror of the donor discovery `info` result from
/// `src/app/bootstrap/network/types.ts` (canonical home:
/// `crate::bootstrap::network::types`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2DiscoveryInfo {
    /// Server name.
    pub name: String,
    /// Map name.
    pub map: String,
    /// Player count.
    pub players: u32,
    /// Maximum players.
    pub max_players: u32,
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

/// Mirror of the donor `client.connect` origin argument
/// (`src/world/session/session.ts`, via `NetworkAddress["kind"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetClientOrigin {
    /// Loopback client.
    Loopback,
    /// Remote client.
    Remote,
}

/// Used-surface mirror of `ExecutableRecipe` from donor
/// `src/contracts/content.ts` (canonical home: the recipe partition);
/// unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2GuestHostRecipe {
    /// Movement provider.
    pub movement: String,
    /// Character definition provider.
    pub character_definition: String,
}

/// Mirror of the donor `q2NativePlayers` entries from
/// `src/app/bootstrap/simulation/runtime.ts` (canonical home: the runtime
/// partition); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Q2NativePlayer {
    /// Client handle.
    pub client: ClientId,
    /// Player actor.
    pub actor: ActorId,
}

/// Host failures, preserving the donor throw sites verbatim.
#[derive(Debug, Error)]
pub enum Q2RereleaseHostError {
    /// Non-rerelease protocol.
    #[error("Rerelease guest server requires protocol1038 or KEX2023")]
    RequiresRereleaseProtocol,
    /// Transcoder protocol rejection.
    #[error("Native rerelease messages require protocol1038 or KEX2023")]
    TranscoderRequiresRerelease,
    /// Retired network player.
    #[error("Q2 guest network player is retired")]
    PlayerRetired,
    /// Headnode outside the shared BSP.
    #[error("Guest visibility headnode is outside the shared BSP")]
    HeadnodeOutsideBsp,
    /// Carried client owned elsewhere.
    #[error("Q2 guest carried client is not owned by this world")]
    CarriedNotOwned,
    /// Engine-owned message from the game.
    #[error("Native game emitted an unsupported engine-owned {0} message")]
    EngineOwnedMessage(&'static str),
    /// Connect cleanup failed; carries the ordered donor messages.
    #[error("Q2 guest connect cleanup failed: {}", .0.join("; "))]
    ConnectCleanup(Vec<String>),
    /// Client cleanup failed; carries the ordered donor messages.
    #[error("Q2 guest client cleanup failed: {}", .0.join("; "))]
    ClientCleanup(Vec<String>),
    /// Game callback failed; carries the cause message.
    #[error("Q2 game callback failed: {0}")]
    GameCallback(String),
    /// Session client allocation failed.
    #[error(transparent)]
    Session(#[from] WorldError),
    /// Session client close failed; carries the session message.
    #[error("{0}")]
    SessionClose(String),
    /// Map geometry read failed.
    #[error(transparent)]
    Content(#[from] MountError),
    /// Product catalog lookup failed.
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    /// Cvar operation failed.
    #[error(transparent)]
    Cvar(#[from] CvarError),
    /// Contract/wire adaptation failed.
    #[error(transparent)]
    Adapter(#[from] Q2AdapterError),
    /// Command parsing failed.
    #[error(transparent)]
    Command(#[from] CmdError),
    /// Map checksum failed.
    #[error(transparent)]
    Checksum(#[from] qa_bots::BotsError),
    /// Wire encode/decode failed.
    #[error(transparent)]
    Wire(#[from] Q2NetError),
    /// Layout selection failed.
    #[error(transparent)]
    Layout(#[from] Q2LayoutError),
}

/// Guest numeric operations used by this host.
///
/// Used-subset seam over `NumericOperations` from donor
/// `src/contracts/numeric.ts` (canonical home: the guest partition); the
/// partition implements it post-merge.
pub trait Q2GuestNumericOps {
    /// Donor `add`.
    fn add(&self, left: f64, right: f64) -> f64;
    /// Donor `subtract`.
    fn subtract(&self, left: f64, right: f64) -> f64;
    /// Donor `multiply`.
    fn multiply(&self, left: f64, right: f64) -> f64;
    /// Donor `divide`.
    fn divide(&self, left: f64, right: f64) -> f64;
    /// Donor `squareRoot`.
    fn square_root(&self, value: f64) -> f64;
}

/// Scene queries used by this host.
///
/// Seam over `SharedSceneQueries` from donor
/// `src/world/collision/index.ts` (canonical home: the world partition);
/// the partition implements it post-merge.
pub trait Q2GuestScene {
    /// Donor `pointLeaf`.
    fn point_leaf(&self, point: &Q2Vec3) -> i32;
    /// Donor `leafCluster`.
    fn leaf_cluster(&self, leaf: i32) -> i32;
    /// Donor `leafArea`.
    fn leaf_area(&self, leaf: i32) -> i32;
    /// Donor `boxLeaves` leaves.
    fn box_leaves(&self, min: &Q2Vec3, max: &Q2Vec3, limit: usize) -> Vec<i32>;
    /// Donor `areasConnected`.
    fn areas_connected(&self, first: i32, second: i32) -> bool;
    /// Donor `areaBits`.
    fn area_bits(&self, area: i32) -> Vec<u8>;
    /// Donor `clusterVisible`.
    fn cluster_visible(&self, from: i32, to: i32, scope: ClusterVisibility) -> bool;
}

/// Rerelease guest world surface used by this host.
///
/// Seam over `RereleaseGuestWorld` from donor
/// `src/app/bootstrap/simulation/rerelease-guest-world.ts` (canonical home:
/// `crate::bootstrap::simulation::rerelease_guest_world`); the guest
/// partition implements it post-merge.
pub trait RereleaseGuestWorld {
    /// Donor `services.options.maxClients`.
    fn max_clients(&self) -> u32;
    /// Donor `services.options.mapPath`.
    fn map_path(&self) -> &str;
    /// Donor `services.options.frameMilliseconds`.
    fn frame_milliseconds(&self) -> u32;
    /// Donor `services.options.numeric`.
    fn numeric(&self) -> &dyn Q2GuestNumericOps;
    /// Donor `services.options.cvars`.
    fn cvars(&self) -> &CvarRegistry;
    /// Donor `services.options.cvars`, mutably.
    fn cvars_mut(&mut self) -> &mut CvarRegistry;
    /// Donor `services.setConfigstring`.
    fn set_configstring(&mut self, index: u32, value: &str);
    /// Donor `configstrings`.
    fn configstrings(&self) -> HashMap<u32, String>;
    /// Donor `playerState`.
    fn player_state(&self, slot: u32) -> Q2RereleasePlayerState;
    /// Donor `entityState`.
    fn entity_state(&self, slot: u32) -> Q2RereleaseEntityState;
    /// Donor `entityStates`.
    fn entity_states(&self) -> Vec<Q2RereleaseEntityState>;
    /// Donor `entityInfo`.
    fn entity_info(&self, slot: u32) -> GuestEntityInfo;
    /// Donor `clients`.
    fn clients(&self) -> Vec<ClassicGuestClient>;
    /// Donor `playerPing`.
    fn player_ping(&self, slot: u32) -> i32;
    /// Donor `connect`; the message propagates like the donor throw.
    fn connect(
        &mut self,
        slot: u32,
        userinfo: &str,
        social_id: &str,
        is_bot: bool,
    ) -> Result<GuestConnectOutcome, String>;
    /// Donor `disconnect`; the message propagates like the donor throw.
    fn disconnect(&mut self, slot: u32) -> Result<(), String>;
    /// Donor `begin`.
    fn begin(&mut self, slot: u32);
    /// Donor `userinfo`.
    fn userinfo(&mut self, slot: u32, value: &str);
    /// Donor `command`.
    fn command(&mut self, slot: u32, argv: &[String], args: &str);
    /// Donor `think`.
    fn think(&mut self, slot: u32, command: &Q2RereleaseUserCommand);
    /// Donor `rawMessages`.
    fn raw_messages(&mut self) -> Vec<RereleaseGuestMessage>;
    /// Donor `isRetired`.
    fn is_retired(&self) -> bool;
}

/// Simulation surface used by this host.
///
/// Seam over `SharedSimulation` from donor
/// `src/app/bootstrap/simulation/runtime.ts` (canonical home: the runtime
/// partition); the partition implements it post-merge.
pub trait Q2GuestHostSimulation {
    /// Donor `recipe` (used surface).
    fn recipe(&self) -> Q2GuestHostRecipe;
    /// Donor `scene`.
    fn scene(&self) -> &dyn Q2GuestScene;
    /// Donor `registerQ2NativeClient`; the message propagates like the
    /// donor admit cleanup rethrow.
    fn register_q2_native_client(&mut self, client: &ClientId) -> Result<OwnedActor, String>;
    /// Donor `q2NativePlayers`.
    fn q2_native_players(&self) -> Vec<Q2NativePlayer>;
    /// Donor `disconnectPlayer`.
    fn disconnect_player(&mut self, actor: &ActorId);
    /// Donor `notifyClientEvent`.
    fn notify_client_event(&mut self, kind: ModClientEventKind, actor: &ActorId);
    /// Donor `observeClientCommand` with the contract command.
    fn observe_client_command(&mut self, actor: &ActorId, client: &ClientId, sequence: u32, command: &Q2Command);
}

/// Content surface used by this host.
///
/// Seam over `LoadedApplicationContent` from donor
/// `src/app/bootstrap/content.ts` (canonical home: the content partition);
/// the partition implements it post-merge.
pub trait RereleaseGuestHostContent {
    /// Donor `mounts.read(recipe.map.geometry)`.
    fn map_geometry_bytes(&self) -> Result<Vec<u8>, MountError>;
    /// Donor `catalog.product(entities.content).expectation.contentDirectory`.
    fn map_content_directory(&self) -> Result<String, CatalogError>;
    /// Donor `world.kind`.
    fn world_kind(&self) -> &'static str;
    /// Donor `world` Q2 BSP nodes for headnode visibility.
    fn q2_nodes(&self) -> Option<&[Node]>;
}

/// Session surface used by this host.
///
/// Seam over `EngineSession`/`SessionClient` from donor
/// `src/world/session/session.ts` (canonical home: `qa_world::session`,
/// which has not ported client connect/close yet); unify post-merge.
pub trait Q2GuestHostSession {
    /// Donor `session.createClient`.
    fn create_client(&mut self, slot: u32) -> Result<ClientId, WorldError>;
    /// Donor `client.connect`.
    fn connect_client(&mut self, client: &ClientId, origin: NetClientOrigin);
    /// Donor `session.closeClient`.
    fn close_client(&mut self, client: &ClientId) -> Result<(), String>;
}

/// Address rejection predicate, mirroring donor `rejects`.
pub type Q2RejectsFn = Rc<dyn Fn(&NetworkAddress) -> bool>;
/// Master server listing, mirroring donor `masters`.
pub type Q2MastersFn = Rc<dyn Fn() -> Vec<NetworkAddress>>;
/// Print sink, mirroring donor `print`.
pub type Q2PrintFn = Rc<dyn Fn(&str)>;
/// Userinfo parser, mirroring donor `q2Userinfo` from
/// `src/content/q2/base/player/index.ts` (canonical home: the content
/// partition); the partition implements it post-merge.
pub type ParseQ2UserinfoFn = Rc<dyn Fn(&str) -> Vec<(String, String)>>;
/// Social identity owner, mirroring donor `playerIdentity`; the message
/// propagates like the donor throw.
pub type Q2PlayerIdentityFn = Rc<dyn Fn(&ClientId, Option<RereleaseSeatIdentity>) -> Result<(), String>>;

/// Mirror of `Q2ApplicationServerBindingOptions` from donor
/// `src/app/bootstrap/simulation/network.ts` (canonical home:
/// `crate::bootstrap::simulation::network`); unify post-merge.
///
/// Only the surface `createRereleaseNativeQ2ApplicationServerHost` uses is
/// mirrored; the downloads handle arrives preconstructed because
/// `createQ2ApplicationDownloads` belongs to the downloads partition.
pub struct Q2RereleaseServerOptions<S, C, E, W, D, A> {
    /// Engine session.
    pub session: E,
    /// Address rejection predicate.
    pub rejects: Option<Q2RejectsFn>,
    /// Shared simulation.
    pub simulation: S,
    /// Loaded content.
    pub content: C,
    /// Protocol identity.
    pub protocol: ProtocolIdentity,
    /// Rcon administration host.
    pub administration: Option<A>,
    /// Master server listing.
    pub masters: Option<Q2MastersFn>,
    /// Social identity owner.
    pub player_identity: Option<Q2PlayerIdentityFn>,
    /// Print sink.
    pub print: Q2PrintFn,
    /// Rerelease guest world.
    pub world: W,
    /// Application downloads handle.
    pub downloads: D,
    /// Userinfo parser.
    pub parse_userinfo: ParseQ2UserinfoFn,
}

/// Rerelease native message transcoder, mirroring donor
/// `createRereleaseNativeMessageTranscoder`.
pub struct RereleaseNativeMessageTranscoder {
    /// Rerelease record reader.
    reader: Q2ServerMessageReader,
    /// Target wire codec.
    wire: Q2Wire,
}

impl std::fmt::Debug for RereleaseNativeMessageTranscoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RereleaseNativeMessageTranscoder")
            .finish_non_exhaustive()
    }
}

/// Create the transcoder, mirroring donor
/// `createRereleaseNativeMessageTranscoder`.
pub fn create_rerelease_native_message_transcoder(
    protocol: ProtocolIdentity,
) -> Result<RereleaseNativeMessageTranscoder, Q2RereleaseHostError> {
    if !matches!(protocol, ProtocolIdentity::Q2Rerelease | ProtocolIdentity::Q2Kex) {
        return Err(Q2RereleaseHostError::TranscoderRequiresRerelease);
    }
    let reader = Q2ServerMessageReader::new(
        ProtocolIdentity::Q2Rerelease,
        Q2ServerMessageOptions {
            max_config_strings: 12448,
            inventory_slots: 256,
            ..Default::default()
        },
        HashSet::new(),
        None,
    )?;
    let wire = Q2Wire::new(protocol)?;
    Ok(RereleaseNativeMessageTranscoder { reader, wire })
}

impl RereleaseNativeMessageTranscoder {
    /// Transcode guest bytes into target wire records.
    pub fn transcode(&mut self, bytes: &[u8]) -> Result<Vec<u8>, Q2RereleaseHostError> {
        let mut out = Vec::new();
        for record in self.reader.read(bytes)? {
            let kind = match &record.event {
                Q2ServerEvent::Frame { .. } => Some("frame"),
                Q2ServerEvent::Private { .. } => Some("private"),
                _ => None,
            };
            if let Some(kind) = kind {
                return Err(Q2RereleaseHostError::EngineOwnedMessage(kind));
            }
            out.extend(encode_q2_server_event(&mut self.wire, &record.event)?);
        }
        Ok(out)
    }
}

/// Rerelease guest server host.
///
/// Concrete mirror of `Q2ApplicationServerHost` from donor
/// `src/app/bootstrap/network/types.ts` (canonical home:
/// `crate::bootstrap::network::types`); unify post-merge.
pub struct Q2RereleaseGuestHost<S, C, E, W, D, A> {
    /// Engine session.
    session: E,
    /// Address rejection predicate.
    rejects: Option<Q2RejectsFn>,
    /// Shared simulation.
    simulation: S,
    /// Loaded content.
    content: C,
    /// Rerelease guest world.
    world: W,
    /// Application downloads handle.
    downloads: D,
    /// Rcon administration host.
    administration: Option<A>,
    /// Master server listing.
    masters: Option<Q2MastersFn>,
    /// Social identity owner.
    player_identity: Option<Q2PlayerIdentityFn>,
    /// Print sink.
    print: Q2PrintFn,
    /// Protocol identity.
    protocol: ProtocolIdentity,
    /// Application layout.
    layout: Q2ApplicationLayout,
    /// Maximum clients.
    max_clients: u32,
    /// Map name.
    map: String,
    /// Message transcoder.
    transcoder: RereleaseNativeMessageTranscoder,
    /// Admitted players by client slot.
    players: HashMap<u32, Q2ApplicationPlayer>,
    /// Retained guest messages.
    messages: Vec<CollectedGuestMessage>,
    /// Fresh local guest messages.
    local_messages: Vec<CollectedGuestMessage>,
    /// Progressed guest messages awaiting publication.
    progressed_messages: Vec<CollectedGuestMessage>,
    /// Clients that already consumed local messages.
    local_recipients: HashSet<ClientId>,
    /// Userinfo parser.
    parse_userinfo: ParseQ2UserinfoFn,
}

impl<S, C, E, W, D, A> std::fmt::Debug for Q2RereleaseGuestHost<S, C, E, W, D, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q2RereleaseGuestHost")
            .field("protocol", &self.protocol)
            .field("max_clients", &self.max_clients)
            .field("map", &self.map)
            .field("players", &self.players)
            .field("messages", &self.messages)
            .finish_non_exhaustive()
    }
}

/// Create the rerelease guest host, mirroring donor
/// `createRereleaseNativeQ2ApplicationServerHost`.
pub fn create_rerelease_native_q2_application_server_host<S, C, E, W, D, A>(
    options: Q2RereleaseServerOptions<S, C, E, W, D, A>,
) -> Result<Q2RereleaseGuestHost<S, C, E, W, D, A>, Q2RereleaseHostError>
where
    S: Q2GuestHostSimulation,
    C: RereleaseGuestHostContent,
    E: Q2GuestHostSession,
    W: RereleaseGuestWorld,
{
    if !matches!(
        options.protocol,
        ProtocolIdentity::Q2Rerelease | ProtocolIdentity::Q2Kex
    ) {
        return Err(Q2RereleaseHostError::RequiresRereleaseProtocol);
    }
    let layout = q2_application_layout(options.protocol)?;
    let mut world = options.world;
    let max_clients = world.max_clients();
    let stripped = world.map_path().strip_prefix("maps/").unwrap_or(world.map_path());
    let map = stripped.strip_suffix(".bsp").unwrap_or(stripped).to_string();
    world
        .cvars_mut()
        .register("hostname", "noname", q2_flags::SERVER_INFO | q2_flags::ARCHIVE)?;
    let version = options.protocol.version().to_string();
    for (name, value) in [("protocol", version.as_str()), ("mapname", map.as_str())] {
        world
            .cvars_mut()
            .register(name, value, q2_flags::SERVER_INFO | q2_flags::NO_SET)?;
        world.cvars_mut().set(name, value, true)?;
    }
    let checksum = block_checksum(&options.content.map_geometry_bytes()?)?;
    world.set_configstring(layout.map_checksum, &checksum.to_string());
    world.set_configstring(layout.max_clients, &max_clients.to_string());
    let air = world.cvars().variable_string("sv_airaccelerate");
    world.set_configstring(layout.air_accelerate, &air);
    let transcoder = create_rerelease_native_message_transcoder(options.protocol)?;
    Ok(Q2RereleaseGuestHost {
        session: options.session,
        rejects: options.rejects,
        simulation: options.simulation,
        content: options.content,
        world,
        downloads: options.downloads,
        administration: options.administration,
        masters: options.masters,
        player_identity: options.player_identity,
        print: options.print,
        protocol: options.protocol,
        layout,
        max_clients,
        map,
        transcoder,
        players: HashMap::new(),
        messages: Vec::new(),
        local_messages: Vec::new(),
        progressed_messages: Vec::new(),
        local_recipients: HashSet::new(),
        parse_userinfo: options.parse_userinfo,
    })
}

/// Donor `q2GameCallback` from `src/app/bootstrap/network/types.ts`
/// (canonical home: `crate::bootstrap::network::types`); unify post-merge.
fn wrap_game_callback<T>(result: Result<T, Q2RereleaseHostError>) -> Result<T, Q2RereleaseHostError> {
    result.map_err(|error| match error {
        Q2RereleaseHostError::GameCallback(_) => error,
        other => Q2RereleaseHostError::GameCallback(other.to_string()),
    })
}

/// Replace-or-append `key` in donor `Map` insertion order.
fn set_userinfo(info: &mut Vec<(String, String)>, key: &str, value: String) {
    match info.iter_mut().find(|(name, _)| name == key) {
        Some(entry) => entry.1 = value,
        None => info.push((key.to_string(), value)),
    }
}

/// Serialize donor `[[key, value]]` userinfo pairs.
fn join_userinfo(info: &[(String, String)]) -> String {
    info.iter()
        .map(|(key, value)| format!("\\{key}\\{value}"))
        .collect::<String>()
}

impl<S, C, E, W, D, A> Q2RereleaseGuestHost<S, C, E, W, D, A>
where
    S: Q2GuestHostSimulation,
    C: RereleaseGuestHostContent,
    E: Q2GuestHostSession,
    W: RereleaseGuestWorld,
{
    /// Donor `rejects` passthrough: false when the predicate is absent.
    #[must_use]
    pub fn rejected(&self, address: &NetworkAddress) -> bool {
        self.rejects.as_ref().is_some_and(|rejects| rejects(address))
    }

    /// Donor `protocol`.
    #[must_use]
    pub fn protocol(&self) -> ProtocolIdentity {
        self.protocol
    }

    /// Donor `messageOptions`.
    #[must_use]
    pub fn message_options(&self) -> Q2ServerMessageOptions {
        Q2ServerMessageOptions {
            max_config_strings: u16::try_from(self.layout.max_config_strings).unwrap_or(u16::MAX),
            ..Default::default()
        }
    }

    /// Donor `maxClients`.
    #[must_use]
    pub fn max_clients(&self) -> u32 {
        self.max_clients
    }

    /// Donor `map`.
    #[must_use]
    pub fn map(&self) -> &str {
        &self.map
    }

    /// Donor `downloads`.
    #[must_use]
    pub fn downloads(&self) -> &D {
        &self.downloads
    }

    /// Donor `administration`.
    #[must_use]
    pub fn administration(&self) -> Option<&A> {
        self.administration.as_ref()
    }

    /// Donor `masters`.
    #[must_use]
    pub fn masters(&self) -> Option<Vec<NetworkAddress>> {
        self.masters.as_ref().map(|masters| masters())
    }

    /// Donor `print`.
    pub fn print(&self, text: &str) {
        (self.print)(text);
    }

    /// Donor `supportsSourceWire`.
    #[must_use]
    pub fn supports_source_wire(&self) -> WireAdmission {
        let mut reasons = Vec::new();
        if self.content.world_kind() != "q2-bsp" {
            reasons.push(
                "Original Quake II peers require Quake II BSP geometry; use local or unified presentation for foreign maps"
                    .to_string(),
            );
        }
        let recipe = self.simulation.recipe();
        if !recipe.movement.starts_with("q2:") {
            reasons.push("Guest native wire requires Q2 movement".to_string());
        }
        if !recipe.character_definition.starts_with("q2:") {
            reasons.push("Guest native wire requires Q2 character state".to_string());
        }
        if reasons.is_empty() {
            WireAdmission::Supported
        } else {
            WireAdmission::Unsupported { reasons }
        }
    }

    /// Donor `requirePlayer`.
    fn require_player(&self, player: &Q2ApplicationPlayer) -> Result<(), Q2RereleaseHostError> {
        match self.players.get(&player.client.slot()) {
            Some(current) if current.client == player.client && current.actor == player.actor => Ok(()),
            _ => Err(Q2RereleaseHostError::PlayerRetired),
        }
    }

    /// Donor `nativeState`.
    fn native_state(&self, player: &Q2ApplicationPlayer) -> Result<Q2RereleasePlayerState, Q2RereleaseHostError> {
        self.require_player(player)?;
        Ok(self.world.player_state(player.source_entity))
    }

    /// Donor `viewOrigin`.
    fn view_origin(&self, player: &Q2ApplicationPlayer) -> Result<Q2Vec3, Q2RereleaseHostError> {
        let state = self.native_state(player)?;
        let numeric = self.world.numeric();
        Ok(Q2Vec3 {
            x: numeric.add(state.movement.origin.x, state.view.view_offset.x),
            y: numeric.add(state.movement.origin.y, state.view.view_offset.y),
            z: numeric.add(state.movement.origin.z, state.view.view_offset.z),
        })
    }

    /// Donor `headnodeVisible`, replacing `BspChild` records with the ported
    /// `qa_content::bsp` children.
    fn headnode_visible(
        &self,
        headnode: i32,
        clusters: &[i32],
        scope: ClusterVisibility,
        nodes: &[Node],
    ) -> Result<bool, Q2RereleaseHostError> {
        use qa_content::bsp::NodeChild;
        let scene = self.simulation.scene();
        let mut pending = vec![if headnode < 0 {
            NodeChild::Leaf((-1 - headnode) as u32)
        } else {
            NodeChild::Node(headnode as u32)
        }];
        while let Some(child) = pending.pop() {
            match child {
                NodeChild::Leaf(index) => {
                    let target = scene.leaf_cluster(index as i32);
                    if clusters
                        .iter()
                        .any(|cluster| scene.cluster_visible(*cluster, target, scope))
                    {
                        return Ok(true);
                    }
                }
                NodeChild::Node(index) => {
                    let node = nodes
                        .get(index as usize)
                        .ok_or(Q2RereleaseHostError::HeadnodeOutsideBsp)?;
                    pending.extend(node.children);
                }
            }
        }
        Ok(false)
    }

    /// Donor `playerMessages`.
    fn player_messages(
        &self,
        player: &Q2ApplicationPlayer,
        incoming: &[CollectedGuestMessage],
    ) -> Result<Vec<CollectedGuestMessage>, Q2RereleaseHostError> {
        self.require_player(player)?;
        let mut result = Vec::new();
        for message in incoming {
            let keep = match &message.source.audience {
                RereleaseGuestAudience::Unicast { slot } => *slot == player.source_entity,
                RereleaseGuestAudience::Multicast { origin, scope } => {
                    if *scope == GuestAudienceScope::All {
                        true
                    } else {
                        let entity = self.world.entity_state(player.source_entity);
                        let scene = self.simulation.scene();
                        let from = scene.point_leaf(origin);
                        let to = scene.point_leaf(&Q2Vec3 {
                            x: entity.base.origin.x,
                            y: entity.base.origin.y,
                            z: entity.base.origin.z,
                        });
                        let kind = match scope {
                            GuestAudienceScope::Phs => ClusterVisibility::Phs,
                            _ => ClusterVisibility::Pvs,
                        };
                        scene.areas_connected(scene.leaf_area(from), scene.leaf_area(to))
                            && scene.cluster_visible(scene.leaf_cluster(from), scene.leaf_cluster(to), kind)
                    }
                }
            };
            if keep {
                result.push(message.clone());
            }
        }
        Ok(result)
    }

    /// Look up a parsed userinfo value.
    fn userinfo_value(&self, userinfo: &str, key: &str) -> Option<String> {
        (self.parse_userinfo)(userinfo)
            .into_iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    /// Donor `discovery.status`.
    pub fn discovery_status(&mut self) -> Result<Q2Status, Q2RereleaseHostError> {
        let server_info = self.world.cvars_mut().info_string(q2_flags::SERVER_INFO, None)?;
        let mut players = Vec::new();
        for client in self.world.clients() {
            let score = i64::from(
                self.world
                    .player_state(client.slot)
                    .view
                    .stats
                    .get(14)
                    .copied()
                    .unwrap_or(0),
            );
            players.push(Q2StatusPlayer {
                score,
                ping: i64::from(self.world.player_ping(client.slot)),
                name: self.userinfo_value(&client.userinfo, "name").unwrap_or_default(),
            });
        }
        Ok(Q2Status { server_info, players })
    }

    /// Donor `discovery.info`.
    #[must_use]
    pub fn discovery_info(&self) -> Q2DiscoveryInfo {
        Q2DiscoveryInfo {
            name: self.world.cvars().variable_string("hostname"),
            map: self.map.clone(),
            players: self.world.clients().len() as u32,
            max_players: self.max_clients,
        }
    }

    /// Donor `admit`. The split seat travels explicitly because the worktree
    /// [`Q2ConnectRequest`] has not ported donor `splitSeat` yet.
    pub fn admit(
        &mut self,
        from: &NetworkAddress,
        request: &Q2ConnectRequest,
        split_seat: Option<u32>,
    ) -> Result<Q2ApplicationAdmission, Q2RereleaseHostError> {
        wrap_game_callback(self.admit_inner(from, request, split_seat))
    }

    /// Donor `admit` body.
    fn admit_inner(
        &mut self,
        from: &NetworkAddress,
        request: &Q2ConnectRequest,
        split_seat: Option<u32>,
    ) -> Result<Q2ApplicationAdmission, Q2RereleaseHostError> {
        if request.protocol != self.protocol {
            return Ok(Q2ApplicationAdmission::Rejected {
                reason: "Native game and requested wire editions differ".to_string(),
            });
        }
        let occupied: HashSet<u32> = self
            .world
            .clients()
            .iter()
            .filter_map(|client| client.slot.checked_sub(1))
            .collect();
        let mut slot = 0;
        while slot < self.max_clients && occupied.contains(&slot) {
            slot += 1;
        }
        if slot == self.max_clients {
            return Ok(Q2ApplicationAdmission::Rejected {
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
        let mut info = (self.parse_userinfo)(&request.userinfo);
        if matches!(request.protocol, ProtocolIdentity::Q2Kex) {
            let suffix = format!("_{}", split_seat.unwrap_or(0));
            for (key, value) in info.clone() {
                if let Some(base) = key.strip_suffix(suffix.as_str()) {
                    set_userinfo(&mut info, base, value);
                }
            }
        }
        set_userinfo(&mut info, "ip", address_key(from, true));
        let social_id = request
            .social_ids
            .as_ref()
            .and_then(|social| social.first().cloned())
            .unwrap_or_default();
        if !social_id.is_empty() && self.player_identity.is_none() {
            self.session
                .close_client(&client)
                .map_err(Q2RereleaseHostError::SessionClose)?;
            return Ok(Q2ApplicationAdmission::Rejected {
                reason: "Server has no rerelease social identity owner".to_string(),
            });
        }
        let seat = RereleaseSeatIdentity {
            seat: split_seat.unwrap_or(0),
            social_id: social_id.clone(),
        };
        if let Some(identify) = &self.player_identity {
            if let Err(message) = identify(&client, Some(seat)) {
                return self.admit_cleanup(&client, slot, false, message);
            }
        }
        let outcome = match self.world.connect(slot + 1, &join_userinfo(&info), &social_id, false) {
            Ok(outcome) => outcome,
            Err(message) => return self.admit_cleanup(&client, slot, false, message),
        };
        if !outcome.allowed {
            if let Some(identify) = &self.player_identity {
                identify(&client, None).map_err(Q2RereleaseHostError::GameCallback)?;
            }
            self.session
                .close_client(&client)
                .map_err(Q2RereleaseHostError::SessionClose)?;
            return Ok(Q2ApplicationAdmission::Rejected {
                reason: self
                    .userinfo_value(&outcome.userinfo, "rejmsg")
                    .unwrap_or_else(|| "Connection refused".to_string()),
            });
        }
        let owned = match self.simulation.register_q2_native_client(&client) {
            Ok(owned) => owned,
            Err(message) => return self.admit_cleanup(&client, slot, true, message),
        };
        let player = Q2ApplicationPlayer {
            client: client.clone(),
            actor: owned.id().clone(),
            source_entity: slot + 1,
        };
        self.players.insert(slot, player.clone());
        Ok(Q2ApplicationAdmission::Accepted { player })
    }

    /// Donor admit `catch`/`finally` cleanup, preserving the masking order:
    /// the `finally` error (close masking identity) replaces the
    /// in-flight aggregate or original error.
    fn admit_cleanup(
        &mut self,
        client: &ClientId,
        slot: u32,
        connected: bool,
        original: String,
    ) -> Result<Q2ApplicationAdmission, Q2RereleaseHostError> {
        let disconnect = if connected {
            self.world.disconnect(slot + 1)
        } else {
            Ok(())
        };
        let identity = self.player_identity.as_ref().map(|identify| identify(client, None));
        let close = self
            .session
            .close_client(client)
            .map_err(Q2RereleaseHostError::SessionClose);
        let finally = match (identity, close) {
            (_, Err(close)) => Some(close),
            (Some(Err(identity)), Ok(())) => Some(Q2RereleaseHostError::GameCallback(identity)),
            _ => None,
        };
        match (disconnect, finally) {
            (Err(_), Some(finally)) => Err(finally),
            (Err(cleanup), None) => Err(Q2RereleaseHostError::GameCallback(
                Q2RereleaseHostError::ConnectCleanup(vec![original, cleanup]).to_string(),
            )),
            (Ok(()), Some(finally)) => Err(finally),
            (Ok(()), None) => Err(Q2RereleaseHostError::GameCallback(original)),
        }
    }

    /// Donor `carriedPlayer`.
    pub fn carried_player(&mut self, client: &ClientId) -> Result<Q2ApplicationPlayer, Q2RereleaseHostError> {
        let actor = self
            .simulation
            .q2_native_players()
            .into_iter()
            .find(|player| player.client == *client)
            .map(|player| player.actor)
            .ok_or(Q2RereleaseHostError::CarriedNotOwned)?;
        let player = Q2ApplicationPlayer {
            client: client.clone(),
            actor,
            source_entity: client.slot() + 1,
        };
        self.players.insert(client.slot(), player.clone());
        Ok(player)
    }

    /// Donor `begin`.
    pub fn begin(&mut self, player: &Q2ApplicationPlayer) -> Result<(), Q2RereleaseHostError> {
        wrap_game_callback(self.begin_inner(player))
    }

    /// Donor `begin` body.
    fn begin_inner(&mut self, player: &Q2ApplicationPlayer) -> Result<(), Q2RereleaseHostError> {
        self.require_player(player)?;
        self.world.begin(player.source_entity);
        self.simulation
            .notify_client_event(ModClientEventKind::Admitted, &player.actor);
        Ok(())
    }

    /// Donor `disconnect`; the interface reason is unused by the donor body.
    pub fn disconnect(&mut self, player: &Q2ApplicationPlayer, _reason: &str) -> Result<(), Q2RereleaseHostError> {
        wrap_game_callback(self.disconnect_inner(player))
    }

    /// Donor `disconnect` body.
    fn disconnect_inner(&mut self, player: &Q2ApplicationPlayer) -> Result<(), Q2RereleaseHostError> {
        self.require_player(player)?;
        let mut failures = Vec::new();
        if !self.world.is_retired() {
            self.simulation.disconnect_player(&player.actor);
        }
        if let Some(identify) = &self.player_identity {
            if let Err(message) = identify(&player.client, None) {
                failures.push(message);
            }
        }
        self.players.remove(&player.client.slot());
        if let Err(message) = self.session.close_client(&player.client) {
            failures.push(message);
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(Q2RereleaseHostError::ClientCleanup(failures))
        }
    }
}

impl<S, C, E, W, D, A> Q2RereleaseGuestHost<S, C, E, W, D, A>
where
    S: Q2GuestHostSimulation,
    C: RereleaseGuestHostContent,
    E: Q2GuestHostSession,
    W: RereleaseGuestWorld,
{
    /// Donor `gameState`.
    pub fn game_state(&self, player: &Q2ApplicationPlayer) -> Result<Q2ApplicationGameState, Q2RereleaseHostError> {
        self.require_player(player)?;
        let configs = self.world.configstrings();
        let gamedir = self
            .content
            .map_content_directory()?
            .split('/')
            .next_back()
            .unwrap_or("baseq2")
            .to_string();
        let baselines = self
            .world
            .entity_states()
            .iter()
            .filter(|state| {
                state.base.model_indexes.iter().any(|index| *index != 0) || state.base.sound != 0 || state.effects != 0
            })
            .map(|state| (state.base.number, from_q2_entity(&Q2Entity::Rerelease(state.clone()))))
            .collect();
        Ok(Q2ApplicationGameState {
            data: Q2ServerDataParams {
                base: ServerData {
                    servercount: 1,
                    attractloop: false,
                    gamedir,
                    clientnum: player.source_entity as i16 - 1,
                    levelname: configs.get(&0).cloned().unwrap_or_else(|| self.map.clone()),
                },
                server_state: 2,
                server_fps: Some(1000.0 / f64::from(self.world.frame_milliseconds())),
            },
            config_strings: configs,
            baselines,
        })
    }

    /// Donor frame `visible` helper.
    fn entity_visible(
        &self,
        info: &GuestEntityInfo,
        clusters: &[i32],
        scope: ClusterVisibility,
    ) -> Result<bool, Q2RereleaseHostError> {
        match &info.clusters {
            None => {
                let nodes = self
                    .content
                    .q2_nodes()
                    .ok_or(Q2RereleaseHostError::HeadnodeOutsideBsp)?;
                self.headnode_visible(info.headnode, clusters, scope, nodes)
            }
            Some(targets) => {
                let scene = self.simulation.scene();
                Ok(targets
                    .iter()
                    .any(|target| clusters.iter().any(|from| scene.cluster_visible(*from, *target, scope))))
            }
        }
    }

    /// Donor frame entity filter.
    fn entity_kept(
        &self,
        player: &Q2ApplicationPlayer,
        state: &Q2RereleaseEntityState,
        origin: &Q2Vec3,
        area: i32,
        clusters: &[i32],
    ) -> Result<bool, Q2RereleaseHostError> {
        if u32::from(state.base.number) == player.source_entity {
            return Ok(true);
        }
        let info = self.world.entity_info(u32::from(state.base.number));
        if info.server_flags & 1024 != 0 || self.world.cvars().variable_value("sv_novis") != 0.0 {
            return Ok(true);
        }
        let scene = self.simulation.scene();
        if !scene.areas_connected(area, info.areas[0]) && !scene.areas_connected(area, info.areas[1]) {
            return Ok(false);
        }
        let beam = state.base.render_effects & 128 != 0;
        let shadow = state.base.render_effects & 16384 != 0;
        if !self.entity_visible(
            &info,
            clusters,
            if beam || shadow || state.base.sound != 0 {
                ClusterVisibility::Phs
            } else {
                ClusterVisibility::Pvs
            },
        )? {
            return Ok(false);
        }
        let numeric = self.world.numeric();
        let dx = numeric.subtract(origin.x, state.base.origin.x);
        let dy = numeric.subtract(origin.y, state.base.origin.y);
        let dz = numeric.subtract(origin.z, state.base.origin.z);
        let distance = numeric.square_root(numeric.add(
            numeric.add(numeric.multiply(dx, dx), numeric.multiply(dy, dy)),
            numeric.multiply(dz, dz),
        ));
        if state.base.sound != 0 {
            let attenuation = if state.loop_attenuation == -1.0 {
                0.0
            } else if state.loop_attenuation > 0.0 && state.loop_attenuation != 3.0 {
                numeric.multiply(state.loop_attenuation, 0.0006)
            } else {
                0.003
            };
            if numeric.multiply(numeric.subtract(distance, 80.0), attenuation) > 1.0 {
                return Ok(state.base.model_indexes[0] != 0
                    && (beam || self.entity_visible(&info, clusters, ClusterVisibility::Pvs)?));
            }
            return Ok(true);
        }
        Ok(state.base.model_indexes[0] != 0 || shadow || distance <= 400.0)
    }

    /// Donor `frame`.
    pub fn frame(
        &self,
        player: &Q2ApplicationPlayer,
        output: &qa_world::session::SimulationOutput,
    ) -> Result<Q2WireFrame, Q2RereleaseHostError> {
        let origin = self.view_origin(player)?;
        let scene = self.simulation.scene();
        let numeric = self.world.numeric();
        let leaf = scene.point_leaf(&origin);
        let area = scene.leaf_area(leaf);
        let mut clusters: Vec<i32> = scene
            .box_leaves(
                &Q2Vec3 {
                    x: numeric.subtract(origin.x, 8.0),
                    y: numeric.subtract(origin.y, 8.0),
                    z: numeric.subtract(origin.z, 8.0),
                },
                &Q2Vec3 {
                    x: numeric.add(origin.x, 8.0),
                    y: numeric.add(origin.y, 8.0),
                    z: numeric.add(origin.z, 8.0),
                },
                64,
            )
            .into_iter()
            .map(|leaf| scene.leaf_cluster(leaf))
            .collect();
        clusters.sort_unstable();
        clusters.dedup();
        let mut entities = Vec::new();
        for state in self.world.entity_states() {
            if !state.base.model_indexes.iter().any(|index| *index != 0)
                && state.effects == 0
                && state.base.sound == 0
                && state.base.event == 0
            {
                continue;
            }
            if !self.entity_kept(player, &state, &origin, area, &clusters)? {
                continue;
            }
            let mut wire = from_q2_entity(&Q2Entity::Rerelease(state.clone()));
            if self.world.entity_info(u32::from(state.base.number)).owner_slot == Some(player.source_entity) {
                wire.solid = 0;
            }
            entities.push(wire);
        }
        Ok(Q2WireFrame {
            valid: true,
            server_frame: output.snapshot.frame.frame,
            delta_frame: -1,
            suppressed_count: 0,
            area_bits: scene.area_bits(area),
            player: from_q2_player(&Q2Player::Rerelease(self.native_state(player)?))?,
            split_players: Vec::new(),
            entities,
        })
    }

    /// Donor `collectMessages`.
    fn collect_messages(&mut self) -> Result<Vec<CollectedGuestMessage>, Q2RereleaseHostError> {
        let mut collected = Vec::new();
        for source in self.world.raw_messages() {
            let bytes = self.transcoder.transcode(&source.bytes)?;
            let mut wire = source.clone();
            wire.bytes = bytes;
            collected.push(CollectedGuestMessage { source, wire });
        }
        Ok(collected)
    }

    /// Donor `observe`.
    pub fn observe(&mut self) -> Result<(), Q2RereleaseHostError> {
        let air = self.world.cvars().variable_string("sv_airaccelerate");
        let index = self.layout.air_accelerate;
        if self.world.configstrings().get(&index).cloned().unwrap_or_default() != air {
            self.world.set_configstring(index, &air);
        }
        self.local_recipients.clear();
        self.local_messages = self.collect_messages()?;
        let mut messages = std::mem::take(&mut self.progressed_messages);
        messages.extend(self.local_messages.clone());
        self.messages = messages;
        Ok(())
    }

    /// Donor `observeProgress`.
    pub fn observe_progress(&mut self) -> Result<(), Q2RereleaseHostError> {
        self.local_recipients.clear();
        self.local_messages = self.collect_messages()?;
        self.progressed_messages.extend(self.local_messages.clone());
        Ok(())
    }

    /// Donor `localMessages`.
    pub fn local_messages(
        &mut self,
        player: &Q2ApplicationPlayer,
    ) -> Result<Vec<Q2GuestByteMessage>, Q2RereleaseHostError> {
        self.require_player(player)?;
        if !self.local_recipients.insert(player.client.clone()) {
            return Ok(Vec::new());
        }
        let local = self.local_messages.clone();
        Ok(self
            .player_messages(player, &local)?
            .into_iter()
            .map(|message| Q2GuestByteMessage {
                bytes: message.source.bytes,
                reliable: message.source.reliable,
            })
            .collect())
    }

    /// Donor `rawMessages`.
    pub fn raw_messages(&self, player: &Q2ApplicationPlayer) -> Result<Vec<Q2GuestByteMessage>, Q2RereleaseHostError> {
        Ok(self
            .player_messages(player, &self.messages.clone())?
            .into_iter()
            .map(|message| Q2GuestByteMessage {
                bytes: message.wire.bytes,
                reliable: message.wire.reliable,
            })
            .collect())
    }

    /// Donor `sourceMessages`.
    pub fn source_messages(
        &self,
        player: &Q2ApplicationPlayer,
    ) -> Result<Vec<Q2GuestByteMessage>, Q2RereleaseHostError> {
        Ok(self
            .player_messages(player, &self.messages.clone())?
            .into_iter()
            .map(|message| Q2GuestByteMessage {
                bytes: message.source.bytes,
                reliable: message.source.reliable,
            })
            .collect())
    }

    /// Donor `events`, always empty.
    #[must_use]
    pub fn events(&self) -> Vec<Q2ApplicationServerEvent> {
        Vec::new()
    }

    /// Donor `input`, always consumed locally like the donor null.
    pub fn input(
        &mut self,
        player: &Q2ApplicationPlayer,
        wire: &Usercmd,
        sequence: u32,
    ) -> Result<Option<ActorCommand>, Q2RereleaseHostError> {
        wrap_game_callback(self.input_inner(player, wire, sequence))
    }

    /// Donor `input` body.
    fn input_inner(
        &mut self,
        player: &Q2ApplicationPlayer,
        wire: &Usercmd,
        sequence: u32,
    ) -> Result<Option<ActorCommand>, Q2RereleaseHostError> {
        self.require_player(player)?;
        let server_frame = if matches!(self.protocol, ProtocolIdentity::Q2Kex) {
            wire.server_frame
        } else {
            i32::try_from(sequence).unwrap_or(i32::MAX)
        };
        let command = to_q2_rerelease_command(wire, server_frame);
        self.simulation.observe_client_command(
            &player.actor,
            &player.client,
            sequence,
            &Q2Command::Rerelease(command.clone()),
        );
        self.world.think(player.source_entity, &command);
        Ok(None)
    }

    /// Donor `expandClientCommand`.
    pub fn expand_client_command(&self, text: &str) -> Result<Option<String>, Q2RereleaseHostError> {
        let print = Rc::clone(&self.print);
        let mut print = |line: &str| print(line);
        Ok(expand_command_macros(
            text,
            &|name| self.world.cvars().variable_string(name),
            &mut print,
            TextMode::Source,
        )?)
    }

    /// Donor `command`.
    pub fn command(
        &mut self,
        player: &Q2ApplicationPlayer,
        name: &str,
        args: &[String],
    ) -> Result<(), Q2RereleaseHostError> {
        let mut parts = vec![name.to_string()];
        parts.extend(args.iter().cloned());
        self.command_text(player, &parts.join(" "))
    }

    /// Donor `commandText`.
    pub fn command_text(&mut self, player: &Q2ApplicationPlayer, text: &str) -> Result<(), Q2RereleaseHostError> {
        wrap_game_callback(self.command_text_inner(player, text))
    }

    /// Donor `commandText` body.
    fn command_text_inner(&mut self, player: &Q2ApplicationPlayer, text: &str) -> Result<(), Q2RereleaseHostError> {
        self.require_player(player)?;
        let tokens = tokenize_command(text, Dialect::Q2Rerelease, TextMode::Source)?;
        if !tokens.argv.is_empty() {
            self.world
                .command(player.source_entity, &tokens.argv, &tokens.args_text);
        }
        Ok(())
    }

    /// Donor `userinfo`.
    pub fn userinfo(&mut self, player: &Q2ApplicationPlayer, value: &str) -> Result<(), Q2RereleaseHostError> {
        wrap_game_callback(self.userinfo_inner(player, value))
    }

    /// Donor `userinfo` body.
    fn userinfo_inner(&mut self, player: &Q2ApplicationPlayer, value: &str) -> Result<(), Q2RereleaseHostError> {
        self.require_player(player)?;
        self.world.userinfo(player.source_entity, value);
        self.simulation
            .notify_client_event(ModClientEventKind::Userinfo, &player.actor);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::bsp::{IndexRange, NodeChild};
    use qa_content::common::Bounds;
    use qa_core::cmd::Dialect;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::time::{FrameContext, FramePhase, SourceTime};
    use qa_net::q2::short_to_angle;
    use qa_net::q2_adapters::{Q2EntityState, Q2PlayerView, Q2RereleaseMovementState};
    use qa_net::q2_net::ChannelKind;
    use qa_world::session::{SimulationOutput, WorldSnapshot};

    struct StubNumeric;

    impl Q2GuestNumericOps for StubNumeric {
        fn add(&self, left: f64, right: f64) -> f64 {
            left + right
        }

        fn subtract(&self, left: f64, right: f64) -> f64 {
            left - right
        }

        fn multiply(&self, left: f64, right: f64) -> f64 {
            left * right
        }

        fn divide(&self, left: f64, right: f64) -> f64 {
            left / right
        }

        fn square_root(&self, value: f64) -> f64 {
            value.sqrt()
        }
    }

    struct StubScene {
        point_leaves: Vec<(Q2Vec3, i32)>,
        leaf_clusters: HashMap<i32, i32>,
        leaf_areas: HashMap<i32, i32>,
        box_leaves: Vec<i32>,
        disconnected: HashSet<(i32, i32)>,
        area_bits: Vec<u8>,
        visibility: HashMap<(i32, i32, ClusterVisibility), bool>,
        default_visible: bool,
    }

    impl StubScene {
        fn new() -> Self {
            Self {
                point_leaves: Vec::new(),
                leaf_clusters: HashMap::new(),
                leaf_areas: HashMap::new(),
                box_leaves: vec![0],
                disconnected: HashSet::new(),
                area_bits: vec![1],
                visibility: HashMap::new(),
                default_visible: true,
            }
        }
    }

    impl Q2GuestScene for StubScene {
        fn point_leaf(&self, point: &Q2Vec3) -> i32 {
            self.point_leaves
                .iter()
                .find(|(known, _)| known == point)
                .map_or(0, |(_, leaf)| *leaf)
        }

        fn leaf_cluster(&self, leaf: i32) -> i32 {
            self.leaf_clusters.get(&leaf).copied().unwrap_or(leaf)
        }

        fn leaf_area(&self, leaf: i32) -> i32 {
            self.leaf_areas.get(&leaf).copied().unwrap_or(1)
        }

        fn box_leaves(&self, _min: &Q2Vec3, _max: &Q2Vec3, _limit: usize) -> Vec<i32> {
            self.box_leaves.clone()
        }

        fn areas_connected(&self, first: i32, second: i32) -> bool {
            !self.disconnected.contains(&(first, second)) && !self.disconnected.contains(&(second, first))
        }

        fn area_bits(&self, _area: i32) -> Vec<u8> {
            self.area_bits.clone()
        }

        fn cluster_visible(&self, from: i32, to: i32, scope: ClusterVisibility) -> bool {
            self.visibility
                .get(&(from, to, scope))
                .copied()
                .unwrap_or(self.default_visible)
        }
    }

    struct StubWorld {
        numeric: StubNumeric,
        cvars: CvarRegistry,
        configs: HashMap<u32, String>,
        max_clients: u32,
        map_path: String,
        frame_milliseconds: u32,
        player_states: HashMap<u32, Q2RereleasePlayerState>,
        entity_states: HashMap<u32, Q2RereleaseEntityState>,
        entity_order: Vec<u32>,
        entity_infos: HashMap<u32, GuestEntityInfo>,
        clients: Vec<ClassicGuestClient>,
        pings: HashMap<u32, i32>,
        connect_result: Result<GuestConnectOutcome, String>,
        connect_calls: Vec<(u32, String, String, bool)>,
        disconnect_err: Option<String>,
        disconnect_calls: Vec<u32>,
        begun: Vec<u32>,
        userinfos: Vec<(u32, String)>,
        commands: Vec<(u32, Vec<String>, String)>,
        thinks: Vec<(u32, Q2RereleaseUserCommand)>,
        raw: Vec<RereleaseGuestMessage>,
        retired: bool,
    }

    impl StubWorld {
        fn new() -> Self {
            let mut cvars = CvarRegistry::new(Dialect::Q2Rerelease);
            cvars.register("sv_airaccelerate", "150", 0).unwrap();
            cvars.register("sv_novis", "0", 0).unwrap();
            Self {
                numeric: StubNumeric,
                cvars,
                configs: HashMap::new(),
                max_clients: 2,
                map_path: "maps/base1.bsp".to_string(),
                frame_milliseconds: 50,
                player_states: HashMap::new(),
                entity_states: HashMap::new(),
                entity_order: Vec::new(),
                entity_infos: HashMap::new(),
                clients: Vec::new(),
                pings: HashMap::new(),
                connect_result: Ok(GuestConnectOutcome {
                    allowed: true,
                    userinfo: String::new(),
                }),
                connect_calls: Vec::new(),
                disconnect_err: None,
                disconnect_calls: Vec::new(),
                begun: Vec::new(),
                userinfos: Vec::new(),
                commands: Vec::new(),
                thinks: Vec::new(),
                raw: Vec::new(),
                retired: false,
            }
        }

        fn info() -> GuestEntityInfo {
            GuestEntityInfo {
                actor: None,
                active: true,
                server_flags: 0,
                areas: [1, 1],
                clusters: Some(vec![5]),
                first_cluster: 5,
                headnode: 0,
                owner_slot: None,
            }
        }
    }

    impl RereleaseGuestWorld for StubWorld {
        fn max_clients(&self) -> u32 {
            self.max_clients
        }

        fn map_path(&self) -> &str {
            &self.map_path
        }

        fn frame_milliseconds(&self) -> u32 {
            self.frame_milliseconds
        }

        fn numeric(&self) -> &dyn Q2GuestNumericOps {
            &self.numeric
        }

        fn cvars(&self) -> &CvarRegistry {
            &self.cvars
        }

        fn cvars_mut(&mut self) -> &mut CvarRegistry {
            &mut self.cvars
        }

        fn set_configstring(&mut self, index: u32, value: &str) {
            self.configs.insert(index, value.to_string());
        }

        fn configstrings(&self) -> HashMap<u32, String> {
            self.configs.clone()
        }

        fn player_state(&self, slot: u32) -> Q2RereleasePlayerState {
            self.player_states[&slot].clone()
        }

        fn entity_state(&self, slot: u32) -> Q2RereleaseEntityState {
            self.entity_states[&slot].clone()
        }

        fn entity_states(&self) -> Vec<Q2RereleaseEntityState> {
            self.entity_order
                .iter()
                .map(|slot| self.entity_states[slot].clone())
                .collect()
        }

        fn entity_info(&self, slot: u32) -> GuestEntityInfo {
            self.entity_infos[&slot].clone()
        }

        fn clients(&self) -> Vec<ClassicGuestClient> {
            self.clients.clone()
        }

        fn player_ping(&self, slot: u32) -> i32 {
            self.pings.get(&slot).copied().unwrap_or(0)
        }

        fn connect(
            &mut self,
            slot: u32,
            userinfo: &str,
            social_id: &str,
            is_bot: bool,
        ) -> Result<GuestConnectOutcome, String> {
            self.connect_calls
                .push((slot, userinfo.to_string(), social_id.to_string(), is_bot));
            let outcome = self.connect_result.clone();
            if let Ok(allowed) = &outcome {
                if allowed.allowed {
                    self.clients.push(ClassicGuestClient {
                        slot,
                        phase: ClassicGuestClientPhase::Connected,
                        userinfo: userinfo.to_string(),
                    });
                }
            }
            outcome
        }

        fn disconnect(&mut self, slot: u32) -> Result<(), String> {
            self.disconnect_calls.push(slot);
            match &self.disconnect_err {
                None => {
                    self.clients.retain(|client| client.slot != slot);
                    Ok(())
                }
                Some(message) => Err(message.clone()),
            }
        }

        fn begin(&mut self, slot: u32) {
            self.begun.push(slot);
        }

        fn userinfo(&mut self, slot: u32, value: &str) {
            self.userinfos.push((slot, value.to_string()));
        }

        fn command(&mut self, slot: u32, argv: &[String], args: &str) {
            self.commands.push((slot, argv.to_vec(), args.to_string()));
        }

        fn think(&mut self, slot: u32, command: &Q2RereleaseUserCommand) {
            self.thinks.push((slot, command.clone()));
        }

        fn raw_messages(&mut self) -> Vec<RereleaseGuestMessage> {
            std::mem::take(&mut self.raw)
        }

        fn is_retired(&self) -> bool {
            self.retired
        }
    }

    struct StubSim {
        recipe: Q2GuestHostRecipe,
        scene: StubScene,
        register_ok: HashMap<ClientId, OwnedActor>,
        register_err: Option<String>,
        native_players: Vec<Q2NativePlayer>,
        disconnects: Vec<ActorId>,
        notified: Vec<(ModClientEventKind, ActorId)>,
        observed: Vec<(ActorId, ClientId, u32, Q2Command)>,
    }

    impl StubSim {
        fn new() -> Self {
            Self {
                recipe: Q2GuestHostRecipe {
                    movement: "q2:move".to_string(),
                    character_definition: "q2:char".to_string(),
                },
                scene: StubScene::new(),
                register_ok: HashMap::new(),
                register_err: None,
                native_players: Vec::new(),
                disconnects: Vec::new(),
                notified: Vec::new(),
                observed: Vec::new(),
            }
        }
    }

    impl Q2GuestHostSimulation for StubSim {
        fn recipe(&self) -> Q2GuestHostRecipe {
            self.recipe.clone()
        }

        fn scene(&self) -> &dyn Q2GuestScene {
            &self.scene
        }

        fn register_q2_native_client(&mut self, client: &ClientId) -> Result<OwnedActor, String> {
            if let Some(message) = &self.register_err {
                return Err(message.clone());
            }
            self.register_ok
                .get(client)
                .cloned()
                .ok_or_else(|| "no registration".to_string())
        }

        fn q2_native_players(&self) -> Vec<Q2NativePlayer> {
            self.native_players.clone()
        }

        fn disconnect_player(&mut self, actor: &ActorId) {
            self.disconnects.push(actor.clone());
        }

        fn notify_client_event(&mut self, kind: ModClientEventKind, actor: &ActorId) {
            self.notified.push((kind, actor.clone()));
        }

        fn observe_client_command(&mut self, actor: &ActorId, client: &ClientId, sequence: u32, command: &Q2Command) {
            self.observed
                .push((actor.clone(), client.clone(), sequence, command.clone()));
        }
    }

    struct StubContent {
        geometry: Vec<u8>,
        directory: String,
        kind: &'static str,
        nodes: Option<Vec<Node>>,
    }

    impl RereleaseGuestHostContent for StubContent {
        fn map_geometry_bytes(&self) -> Result<Vec<u8>, MountError> {
            Ok(self.geometry.clone())
        }

        fn map_content_directory(&self) -> Result<String, CatalogError> {
            Ok(self.directory.clone())
        }

        fn world_kind(&self) -> &'static str {
            self.kind
        }

        fn q2_nodes(&self) -> Option<&[Node]> {
            self.nodes.as_deref()
        }
    }

    struct StubSession {
        owner: IdentityOwner,
        generations: HashMap<u32, u32>,
        connected: Vec<(ClientId, NetClientOrigin)>,
        closed: Vec<ClientId>,
        close_err: Option<String>,
    }

    impl Q2GuestHostSession for StubSession {
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
            match &self.close_err {
                None => Ok(()),
                Some(message) => Err(message.clone()),
            }
        }
    }

    type StubHost = Q2RereleaseGuestHost<StubSim, StubContent, StubSession, StubWorld, (), ()>;

    struct Fixture {
        owner: IdentityOwner,
        actor1: ActorId,
        owned1: OwnedActor,
        client0: ClientId,
        client9: ClientId,
    }

    impl Fixture {
        fn new() -> Self {
            let owner = IdentityOwner::create("q2-rerelease-test").unwrap();
            let actor1 = owner.actor(1, 1);
            let owned1 = owner.owned_actor(&actor1, ProviderId::new("q2", "game")).unwrap();
            let client0 = owner.client(0, 0);
            let client9 = owner.client(9, 0);
            Self {
                owner,
                actor1,
                owned1,
                client0,
                client9,
            }
        }
    }

    /// Donor `q2Userinfo` first-wins parse, standing in for the content
    /// partition implementation behind the seam.
    fn parse_userinfo(source: &str) -> Vec<(String, String)> {
        let trimmed = source.strip_prefix('\\').unwrap_or(source);
        let tokens: Vec<&str> = trimmed.split('\\').collect();
        let mut result = Vec::new();
        let mut index = 0;
        while index + 1 < tokens.len() {
            let (key, value) = (tokens[index], tokens[index + 1]);
            if !result.iter().any(|(known, _)| known == key) {
                result.push((key.to_string(), value.to_string()));
            }
            index += 2;
        }
        result
    }

    struct Ids {
        actor1: ActorId,
        client0: ClientId,
        client9: ClientId,
    }

    struct IdentityLog {
        calls: Vec<(ClientId, Option<RereleaseSeatIdentity>)>,
        error: Option<String>,
    }

    fn player_state() -> Q2RereleasePlayerState {
        let mut stats = vec![0_i16; 32];
        stats[14] = 15;
        Q2RereleasePlayerState {
            view: Q2PlayerView {
                view_offset: Q2Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                stats,
                ..Default::default()
            },
            movement: Q2RereleaseMovementState {
                origin: Q2Vec3 {
                    x: 100.0,
                    y: 200.0,
                    z: 300.0,
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn entity(number: u16) -> Q2RereleaseEntityState {
        Q2RereleaseEntityState {
            base: Q2EntityState {
                number,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn standard_world() -> StubWorld {
        let mut world = StubWorld::new();
        world.player_states.insert(1, player_state());
        let mut own = entity(1);
        own.base.model_indexes = [1, 0, 0, 0];
        world.entity_states.insert(1, own);
        world.entity_order.push(1);
        world.entity_infos.insert(1, StubWorld::info());
        world
    }

    fn standard_host() -> (StubHost, Ids, std::rc::Rc<std::cell::RefCell<IdentityLog>>) {
        standard_host_with(ProtocolIdentity::Q2Rerelease)
    }

    fn standard_host_with(protocol: ProtocolIdentity) -> (StubHost, Ids, std::rc::Rc<std::cell::RefCell<IdentityLog>>) {
        let fixture = Fixture::new();
        let ids = Ids {
            actor1: fixture.actor1.clone(),
            client0: fixture.client0.clone(),
            client9: fixture.client9.clone(),
        };
        let mut sim = StubSim::new();
        sim.register_ok.insert(fixture.client0.clone(), fixture.owned1.clone());
        sim.native_players.push(Q2NativePlayer {
            client: fixture.client0.clone(),
            actor: fixture.actor1.clone(),
        });
        let content = StubContent {
            geometry: b"fake-bsp".to_vec(),
            directory: "q2/quake2/baseq2".to_string(),
            kind: "q2-bsp",
            nodes: Some(vec![Node {
                plane: 0,
                children: [NodeChild::Leaf(0), NodeChild::Leaf(1)],
                bounds: Bounds {
                    min: [0.0, 0.0, 0.0],
                    max: [1.0, 1.0, 1.0],
                },
                faces: IndexRange { first: 0, count: 0 },
            }]),
        };
        let session = StubSession {
            owner: fixture.owner,
            generations: HashMap::new(),
            connected: Vec::new(),
            closed: Vec::new(),
            close_err: None,
        };
        let log = std::rc::Rc::new(std::cell::RefCell::new(IdentityLog {
            calls: Vec::new(),
            error: None,
        }));
        let identify = Rc::clone(&log);
        let options = Q2RereleaseServerOptions {
            session,
            rejects: None,
            simulation: sim,
            content,
            protocol,
            administration: None::<()>,
            masters: None,
            player_identity: Some(Rc::new(move |client, seat| {
                let mut log = identify.borrow_mut();
                log.calls.push((client.clone(), seat));
                match &log.error {
                    None => Ok(()),
                    Some(message) => Err(message.clone()),
                }
            })),
            print: Rc::new(|_| {}),
            world: standard_world(),
            downloads: (),
            parse_userinfo: Rc::new(parse_userinfo),
        };
        let host = create_rerelease_native_q2_application_server_host(options).unwrap();
        (host, ids, log)
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

    fn request(userinfo: &str) -> Q2ConnectRequest {
        Q2ConnectRequest {
            protocol: ProtocolIdentity::Q2Rerelease,
            qport: 0,
            challenge: 0,
            userinfo: userinfo.to_string(),
            payload_bytes: 0,
            channel: ChannelKind::New,
            compression: false,
            social_ids: None,
        }
    }

    fn loopback() -> NetworkAddress {
        NetworkAddress::Loopback { id: "test".to_string() }
    }

    fn admit(host: &mut StubHost, userinfo: &str) -> Q2ApplicationPlayer {
        let admission = host.admit(&loopback(), &request(userinfo), None).unwrap();
        let Q2ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        player
    }

    fn guest_message(bytes: Vec<u8>) -> RereleaseGuestMessage {
        RereleaseGuestMessage {
            audience: RereleaseGuestAudience::Unicast { slot: 1 },
            source_dialect: RereleaseSourceDialect::Q2MulticastFloat,
            reliable: false,
            dupe_key: 7,
            bytes,
        }
    }

    /// Minimal rerelease `svc_frame` datagram: frame 1, full update
    /// (`offset 31` marker), empty flags, no areas, default player delta,
    /// empty packet entities.
    fn frame_bytes() -> Vec<u8> {
        vec![20, 1, 0, 0, 248, 0, 0, 0, 0, 0, 18, 0, 0]
    }

    #[test]
    fn rejects_non_rerelease_protocol() {
        let fixture = Fixture::new();
        let mut sim = StubSim::new();
        sim.register_ok.insert(fixture.client0.clone(), fixture.owned1.clone());
        let options = Q2RereleaseServerOptions {
            session: StubSession {
                owner: fixture.owner,
                generations: HashMap::new(),
                connected: Vec::new(),
                closed: Vec::new(),
                close_err: None,
            },
            rejects: None,
            simulation: sim,
            content: StubContent {
                geometry: Vec::new(),
                directory: String::new(),
                kind: "q2-bsp",
                nodes: None,
            },
            protocol: ProtocolIdentity::Q2Classic,
            administration: None::<()>,
            masters: None,
            player_identity: None,
            print: Rc::new(|_| {}),
            world: StubWorld::new(),
            downloads: (),
            parse_userinfo: Rc::new(parse_userinfo),
        };
        assert!(matches!(
            create_rerelease_native_q2_application_server_host(options).unwrap_err(),
            Q2RereleaseHostError::RequiresRereleaseProtocol
        ));
        assert!(matches!(
            create_rerelease_native_message_transcoder(ProtocolIdentity::Q2Classic).unwrap_err(),
            Q2RereleaseHostError::TranscoderRequiresRerelease
        ));
    }

    #[test]
    fn constructor_applies_layout() {
        let (host, _ids, _log) = standard_host();
        let layout = q2_application_layout(ProtocolIdentity::Q2Rerelease).unwrap();
        assert_eq!(host.map(), "base1");
        assert_eq!(host.protocol(), ProtocolIdentity::Q2Rerelease);
        assert_eq!(host.world.cvars.variable_string("protocol"), "1038");
        assert_eq!(host.world.cvars.variable_string("mapname"), "base1");
        assert_eq!(
            host.world.configs.get(&layout.map_checksum).cloned().unwrap(),
            block_checksum(b"fake-bsp").unwrap().to_string()
        );
        assert_eq!(host.world.configs.get(&layout.max_clients).cloned().unwrap(), "2");
        assert_eq!(host.world.configs.get(&layout.air_accelerate).cloned().unwrap(), "150");
        assert_eq!(
            host.message_options().max_config_strings,
            u16::try_from(layout.max_config_strings).unwrap_or(u16::MAX)
        );
        assert_eq!(host.message_options().inventory_slots, 256);
    }

    #[test]
    fn transcoder_roundtrips_and_rejects_owned() {
        let mut wire = Q2Wire::new(ProtocolIdentity::Q2Rerelease).unwrap();
        let nop = encode_q2_server_event(&mut wire, &Q2ServerEvent::Nop).unwrap();
        let mut transcoder = create_rerelease_native_message_transcoder(ProtocolIdentity::Q2Rerelease).unwrap();
        assert_eq!(transcoder.transcode(&nop).unwrap(), nop);
        assert!(transcoder.transcode(&[]).unwrap().is_empty());

        // Minimal rerelease `svc_frame` datagram; `encode_q2_server_event`
        // refuses frames by design, so the bytes are spelled out.
        let error = transcoder.transcode(&frame_bytes()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Native game emitted an unsupported engine-owned frame message"
        );
    }

    #[test]
    fn admit_checks_edition_identity_and_cleanup() {
        let (mut host, ids, log) = standard_host();
        let mut mismatched = request("\\name\\solo");
        mismatched.protocol = ProtocolIdentity::Q2Classic;
        let admission = host.admit(&loopback(), &mismatched, None).unwrap();
        assert!(matches!(
            admission,
            Q2ApplicationAdmission::Rejected { reason }
                if reason == "Native game and requested wire editions differ"
        ));

        let player = admit(&mut host, "\\name\\solo");
        assert_eq!(player.source_entity, 1);
        assert_eq!(
            host.session.connected,
            vec![(ids.client0.clone(), NetClientOrigin::Loopback)]
        );
        assert_eq!(
            host.world.connect_calls,
            vec![(1, "\\name\\solo\\ip\\loopback:test".to_string(), String::new(), false)]
        );
        assert_eq!(
            log.borrow().calls,
            vec![(
                ids.client0.clone(),
                Some(RereleaseSeatIdentity {
                    seat: 0,
                    social_id: String::new(),
                })
            )]
        );

        let (mut host, _ids, _log) = standard_host();
        host.player_identity = None;
        let mut social = request("\\name\\solo");
        social.social_ids = Some(vec!["s1".to_string()]);
        let admission = host.admit(&loopback(), &social, None).unwrap();
        assert!(matches!(
            admission,
            Q2ApplicationAdmission::Rejected { reason }
                if reason == "Server has no rerelease social identity owner"
        ));

        let (mut host, ids, log) = standard_host();
        let mut social = request("\\name\\solo");
        social.social_ids = Some(vec!["s1".to_string()]);
        let admission = host.admit(&loopback(), &social, Some(2)).unwrap();
        assert!(matches!(admission, Q2ApplicationAdmission::Accepted { .. }));
        assert_eq!(host.world.connect_calls[0].2, "s1");
        assert_eq!(
            log.borrow().calls[0],
            (
                ids.client0.clone(),
                Some(RereleaseSeatIdentity {
                    seat: 2,
                    social_id: "s1".to_string(),
                })
            )
        );

        let (mut host, ids, log) = standard_host();
        host.world.connect_result = Ok(GuestConnectOutcome {
            allowed: false,
            userinfo: "\\rejmsg\\banned".to_string(),
        });
        let admission = host.admit(&loopback(), &request("\\name\\solo"), None).unwrap();
        assert!(matches!(
            admission,
            Q2ApplicationAdmission::Rejected { reason } if reason == "banned"
        ));
        assert_eq!(
            log.borrow().calls,
            vec![
                (
                    ids.client0.clone(),
                    Some(RereleaseSeatIdentity {
                        seat: 0,
                        social_id: String::new(),
                    })
                ),
                (ids.client0.clone(), None),
            ]
        );
        assert_eq!(host.session.closed, vec![ids.client0.clone()]);

        host.world.connect_result = Err("denied".to_string());
        let error = host.admit(&loopback(), &request("\\name\\solo"), None).unwrap_err();
        assert_eq!(error.to_string(), "Q2 game callback failed: denied");

        let (mut host, _ids, log) = standard_host();
        host.simulation.register_err = Some("bad actor".to_string());
        let error = host.admit(&loopback(), &request("\\name\\solo"), None).unwrap_err();
        assert_eq!(error.to_string(), "Q2 game callback failed: bad actor");
        assert_eq!(host.world.disconnect_calls, vec![1]);

        host.world.disconnect_err = Some("stuck".to_string());
        let error = host.admit(&loopback(), &request("\\name\\solo"), None).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Q2 guest connect cleanup failed: bad actor; stuck"),
            "{error}"
        );

        host.world.disconnect_err = None;
        host.session.close_err = Some("close failed".to_string());
        let error = host.admit(&loopback(), &request("\\name\\solo"), None).unwrap_err();
        assert_eq!(error.to_string(), "Q2 game callback failed: close failed");

        log.borrow_mut().error = Some("identity down".to_string());
        host.session.close_err = None;
        let error = host.admit(&loopback(), &request("\\name\\solo"), None).unwrap_err();
        assert_eq!(error.to_string(), "Q2 game callback failed: identity down");
    }

    #[test]
    fn kex_admit_strips_seat_userinfo() {
        let (mut host, ids, log) = standard_host_with(ProtocolIdentity::Q2Kex);
        let mut kex = request("\\name_1\\split\\name\\base");
        kex.protocol = ProtocolIdentity::Q2Kex;
        let admission = host.admit(&loopback(), &kex, Some(1)).unwrap();
        let Q2ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        assert_eq!(player.client, ids.client0);
        assert_eq!(
            host.world.connect_calls[0].1,
            "\\name_1\\split\\name\\split\\ip\\loopback:test"
        );
        assert_eq!(
            log.borrow().calls[0].1,
            Some(RereleaseSeatIdentity {
                seat: 1,
                social_id: String::new(),
            })
        );
    }

    #[test]
    fn carried_begin_and_disconnect() {
        let (mut host, ids, log) = standard_host();
        assert!(matches!(
            host.carried_player(&ids.client9).unwrap_err(),
            Q2RereleaseHostError::CarriedNotOwned
        ));
        let player = host.carried_player(&ids.client0).unwrap();
        host.begin(&player).unwrap();
        assert_eq!(host.world.begun, vec![1]);
        assert_eq!(
            host.simulation.notified,
            vec![(ModClientEventKind::Admitted, ids.actor1.clone())]
        );

        host.disconnect(&player, "bye").unwrap();
        assert_eq!(host.simulation.disconnects, vec![ids.actor1.clone()]);
        assert_eq!(log.borrow().calls, vec![(ids.client0.clone(), None)]);
        assert_eq!(host.session.closed, vec![ids.client0.clone()]);

        let player = host.carried_player(&ids.client0).unwrap();
        log.borrow_mut().error = Some("identity down".to_string());
        host.session.close_err = Some("close failed".to_string());
        let error = host.disconnect(&player, "bye").unwrap_err();
        assert_eq!(
            error.to_string(),
            "Q2 game callback failed: Q2 guest client cleanup failed: identity down; close failed"
        );
    }

    #[test]
    fn game_state_reports_rerelease_fps() {
        let (mut host, _ids, _log) = standard_host();
        let mut effects = entity(2);
        effects.effects = 8;
        host.world.entity_states.insert(2, effects);
        host.world.entity_order.push(2);
        host.world.entity_infos.insert(2, StubWorld::info());
        host.world.entity_states.insert(3, entity(3));
        host.world.entity_order.push(3);
        host.world.entity_infos.insert(3, StubWorld::info());
        let player = admit(&mut host, "\\name\\solo");
        let state = host.game_state(&player).unwrap();
        assert_eq!(state.data.base.clientnum, 0);
        assert_eq!(state.data.server_state, 2);
        assert_eq!(state.data.server_fps, Some(20.0));
        assert_eq!(state.data.base.gamedir, "baseq2");
        let mut keys: Vec<u16> = state.baselines.keys().copied().collect();
        keys.sort_unstable();
        assert_eq!(keys, vec![1, 2]);
    }

    #[test]
    fn frame_applies_rerelease_rules() {
        let (mut host, _ids, _log) = standard_host();
        // Server-flag bypass with fully disconnected areas.
        let mut flagged = entity(5);
        flagged.base.model_indexes = [1, 0, 0, 0];
        host.world.entity_states.insert(5, flagged);
        host.world.entity_order.push(5);
        let mut info = StubWorld::info();
        info.server_flags = 1024;
        info.areas = [2, 3];
        host.world.entity_infos.insert(5, info);
        // Sound entity with default attenuation, far away.
        let mut sound = entity(6);
        sound.base.model_indexes = [0, 0, 0, 0];
        sound.base.sound = 1;
        sound.base.origin = Q2Vec3 {
            x: 1000.0,
            y: 0.0,
            z: 0.0,
        };
        sound.loop_attenuation = 0.0;
        host.world.entity_states.insert(6, sound);
        host.world.entity_order.push(6);
        host.world.entity_infos.insert(6, StubWorld::info());
        // Beam entity, far away.
        let mut beam = entity(7);
        beam.base.model_indexes = [3, 0, 0, 0];
        beam.base.origin = Q2Vec3 {
            x: 1000.0,
            y: 0.0,
            z: 0.0,
        };
        beam.base.render_effects = 128;
        host.world.entity_states.insert(7, beam);
        host.world.entity_order.push(7);
        host.world.entity_infos.insert(7, StubWorld::info());
        // Shadow entity without a primary model, nearby.
        let mut shadow = entity(8);
        shadow.base.model_indexes = [0, 1, 0, 0];
        shadow.base.render_effects = 16384;
        host.world.entity_states.insert(8, shadow);
        host.world.entity_order.push(8);
        host.world.entity_infos.insert(8, StubWorld::info());
        // Owned entity.
        let mut owned = entity(9);
        owned.base.model_indexes = [4, 0, 0, 0];
        owned.base.solid = 5;
        host.world.entity_states.insert(9, owned);
        host.world.entity_order.push(9);
        let mut owner = StubWorld::info();
        owner.owner_slot = Some(1);
        host.world.entity_infos.insert(9, owner);
        // Beam resolved through the headnode walk.
        let mut headnode = entity(10);
        headnode.base.model_indexes = [5, 0, 0, 0];
        headnode.base.render_effects = 128;
        host.world.entity_states.insert(10, headnode);
        host.world.entity_order.push(10);
        let mut head = StubWorld::info();
        head.clusters = None;
        host.world.entity_infos.insert(10, head);

        host.simulation.scene.point_leaves = vec![(
            Q2Vec3 {
                x: 101.0,
                y: 202.0,
                z: 303.0,
            },
            10,
        )];
        host.simulation.scene.leaf_clusters.insert(10, 3);
        host.simulation.scene.leaf_areas.insert(10, 1);
        host.simulation.scene.box_leaves = vec![11];
        host.simulation.scene.leaf_clusters.insert(11, 4);
        host.simulation.scene.leaf_clusters.insert(0, 5);
        host.simulation.scene.leaf_clusters.insert(1, 5);
        host.simulation.scene.disconnected.insert((1, 2));
        host.simulation.scene.disconnected.insert((1, 3));

        let player = admit(&mut host, "\\name\\solo");
        let frame = host.frame(&player, &output()).unwrap();
        assert_eq!(frame.server_frame, 7);
        assert_eq!(frame.player.viewoffset, [1.0, 2.0, 3.0]);
        let numbers: Vec<u16> = frame.entities.iter().map(|entity| entity.number).collect();
        assert!(numbers.contains(&1));
        assert!(numbers.contains(&5));
        // Default attenuation drops the distant model-less sound:
        // (970 - 80) * 0.003 > 1 with no primary model and no beam.
        assert!(!numbers.contains(&6));
        assert!(numbers.contains(&7));
        assert!(numbers.contains(&8));
        let owned = frame.entities.iter().find(|entity| entity.number == 9).unwrap();
        assert_eq!(owned.solid, 0);
        assert!(numbers.contains(&10));

        // Loop attenuation -1 disables the distance gate for sounds.
        host.world.entity_states.get_mut(&6).unwrap().loop_attenuation = -1.0;
        let frame = host.frame(&player, &output()).unwrap();
        let numbers: Vec<u16> = frame.entities.iter().map(|entity| entity.number).collect();
        assert!(numbers.contains(&6));

        // PHS-gated beam drops when the beam scope goes dark.
        host.simulation
            .scene
            .visibility
            .insert((4, 5, ClusterVisibility::Phs), false);
        let frame = host.frame(&player, &output()).unwrap();
        let numbers: Vec<u16> = frame.entities.iter().map(|entity| entity.number).collect();
        assert!(!numbers.contains(&7));
        assert!(!numbers.contains(&10));

        // novis bypasses visibility for area-culled entities.
        let mut culled = entity(11);
        culled.base.model_indexes = [6, 0, 0, 0];
        host.world.entity_states.insert(11, culled);
        host.world.entity_order.push(11);
        let mut areas = StubWorld::info();
        areas.areas = [2, 3];
        host.world.entity_infos.insert(11, areas);
        let frame = host.frame(&player, &output()).unwrap();
        let numbers: Vec<u16> = frame.entities.iter().map(|entity| entity.number).collect();
        assert!(!numbers.contains(&11));
        host.world.cvars.set("sv_novis", "1", true).unwrap();
        let frame = host.frame(&player, &output()).unwrap();
        let numbers: Vec<u16> = frame.entities.iter().map(|entity| entity.number).collect();
        assert!(numbers.contains(&11));
    }

    #[test]
    fn observe_transcodes_and_syncs_air() {
        let (mut host, _ids, _log) = standard_host();
        let player = admit(&mut host, "\\name\\solo");
        let mut wire = Q2Wire::new(ProtocolIdentity::Q2Rerelease).unwrap();
        let nop = encode_q2_server_event(&mut wire, &Q2ServerEvent::Nop).unwrap();
        host.world.raw = vec![guest_message(nop.clone())];
        host.observe().unwrap();
        let local = host.local_messages(&player).unwrap();
        assert_eq!(
            local,
            vec![Q2GuestByteMessage {
                bytes: nop.clone(),
                reliable: false,
            }]
        );
        let raw = host.raw_messages(&player).unwrap();
        assert_eq!(raw, local);
        let source = host.source_messages(&player).unwrap();
        assert_eq!(source, local);
        assert!(host.local_messages(&player).unwrap().is_empty());

        let layout = q2_application_layout(ProtocolIdentity::Q2Rerelease).unwrap();
        host.world.cvars.set("sv_airaccelerate", "200", true).unwrap();
        host.observe().unwrap();
        assert_eq!(host.world.configs.get(&layout.air_accelerate).cloned().unwrap(), "200");

        host.world.raw = vec![guest_message(frame_bytes())];
        assert!(matches!(
            host.observe().unwrap_err(),
            Q2RereleaseHostError::EngineOwnedMessage("frame")
        ));
    }

    #[test]
    fn input_uses_kex_server_frame() {
        let (mut host, ids, _log) = standard_host();
        let player = admit(&mut host, "\\name\\solo");
        let wire = Usercmd {
            msec: 50,
            buttons: 1,
            angles: [100, 200, 300],
            forwardmove: 10,
            sidemove: 20,
            upmove: 30,
            impulse: 2,
            lightlevel: 3,
            server_frame: 44,
        };
        assert_eq!(host.input(&player, &wire, 9).unwrap(), None);
        let expected = Q2RereleaseUserCommand {
            milliseconds: 50,
            angles: Q2Vec3 {
                x: short_to_angle(100),
                y: short_to_angle(200),
                z: short_to_angle(300),
            },
            forward_move: 10,
            side_move: 20,
            buttons: 1,
            server_frame: 9,
        };
        assert_eq!(host.world.thinks, vec![(1, expected.clone())]);
        assert_eq!(
            host.simulation.observed,
            vec![(
                ids.actor1.clone(),
                ids.client0.clone(),
                9,
                Q2Command::Rerelease(expected)
            )]
        );

        let (mut host, _ids, _log) = standard_host_with(ProtocolIdentity::Q2Kex);
        let mut kex = request("\\name\\solo");
        kex.protocol = ProtocolIdentity::Q2Kex;
        let admission = host.admit(&loopback(), &kex, None).unwrap();
        let Q2ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        host.input(&player, &wire, 9).unwrap();
        assert_eq!(host.world.thinks[0].1.server_frame, 44);
    }

    #[test]
    fn command_userinfo_expand_and_discovery() {
        let (mut host, ids, _log) = standard_host();
        let player = admit(&mut host, "\\name\\solo");
        host.command(&player, "say", &["hi".to_string()]).unwrap();
        assert_eq!(
            host.world.commands,
            vec![(1, vec!["say".to_string(), "hi".to_string()], "hi".to_string())]
        );
        host.command_text(&player, "").unwrap();
        assert_eq!(host.world.commands.len(), 1);
        host.userinfo(&player, "\\name\\new").unwrap();
        assert_eq!(
            host.simulation.notified,
            vec![(ModClientEventKind::Userinfo, ids.actor1.clone())]
        );
        host.world.cvars.register("testvar", "v1", 0).unwrap();
        assert_eq!(
            host.expand_client_command("go $testvar").unwrap(),
            Some("go v1".to_string())
        );

        host.world.clients.push(ClassicGuestClient {
            slot: 1,
            phase: ClassicGuestClientPhase::Active,
            userinfo: "\\name\\solo".to_string(),
        });
        host.world.pings.insert(1, 42);
        let status = host.discovery_status().unwrap();
        // admit() already registered the world client; both rows report.
        assert_eq!(status.players.len(), 2);
        assert!(status.players.iter().all(|row| row.score == 15 && row.name == "solo"));
        let info = host.discovery_info();
        assert_eq!(info.name, "noname");
        assert_eq!(info.map, "base1");
        assert_eq!(info.players, 2);
        assert_eq!(info.max_players, 2);

        assert!(host.events().is_empty());
        assert!(!host.rejected(&loopback()));
        assert!(host.masters().is_none());
        assert_eq!(host.downloads(), &());
        assert_eq!(host.administration(), None);
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let pushed = Rc::clone(&seen);
        host.print = Rc::new(move |text| pushed.borrow_mut().push(text.to_string()));
        host.print("hi");
        assert_eq!(*seen.borrow(), vec!["hi".to_string()]);
    }

    #[test]
    fn supports_source_wire_matches_classic() {
        let (mut host, _ids, _log) = standard_host();
        assert!(matches!(host.supports_source_wire(), WireAdmission::Supported));
        host.content.kind = "foreign";
        let WireAdmission::Unsupported { reasons } = host.supports_source_wire() else {
            panic!("expected unsupported");
        };
        assert_eq!(reasons.len(), 1);
    }

    #[test]
    fn reexported_mirrors_share_canonical_identity() {
        use crate::bootstrap::network::types as canonical;
        let owner = IdentityOwner::create("q2-rerelease-unify-test").unwrap();
        let player: canonical::Q2ApplicationPlayer = Q2ApplicationPlayer {
            client: owner.client(1, 1),
            actor: owner.actor(1, 1),
            source_entity: 5,
        };
        let admitted = Q2ApplicationAdmission::Accepted { player };
        let canonical::Q2ApplicationAdmission::Accepted { player } = admitted else {
            panic!("expected accepted");
        };
        assert_eq!(player.source_entity, 5);
        let message: canonical::Q2RawServerMessage = Q2GuestByteMessage {
            bytes: vec![1, 2],
            reliable: true,
        };
        assert_eq!(message.bytes, vec![1, 2]);
        let client: super::super::classic_guest_world::ClassicGuestClient = ClassicGuestClient {
            slot: 1,
            phase: ClassicGuestClientPhase::Active,
            userinfo: String::new(),
        };
        assert_eq!(client.phase.as_str(), "active");
    }
}
