//! Classic Quake II guest projection through the native server channel.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/network-q2-guest.ts`.
//!
//! This file is self-contained like its donor: shared application shapes are
//! mirrored here with canonical-home notes instead of importing sibling
//! ports. The async content fetch and download construction
//! (`LoadedApplicationContent.forContent`, `createQ2ApplicationDownloads`
//! from donor `src/app/bootstrap/network/q2-downloads.ts`) live behind the
//! synchronous [`ClassicGuestHostContent`] seam and the injected `downloads`
//! handle; bootstrap ports are sync.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_bots::md4::block_checksum;
use qa_content::bsp::{Node, NodeChild};
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
    from_q2_entity, from_q2_player, to_q2_command, Q2AdapterError, Q2Command, Q2Entity, Q2EntityState, Q2Player,
    Q2PlayerState, Q2UserCommand, Q2Vec3,
};
use qa_net::q2_net::{Q2ConnectRequest, Q2ServerEvent, Q2ServerMessageOptions, Q2Status, Q2StatusPlayer, Q2WireFrame};
use qa_world::WorldError;
use thiserror::Error;

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
/// `ClassicGuestAudience` visibility (`"all" | "phs" | "pvs"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuestAudienceScope {
    /// Every client.
    All,
    /// Potentially hearable set.
    Phs,
    /// Potentially visible set.
    Pvs,
}

impl GuestAudienceScope {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            GuestAudienceScope::All => "all",
            GuestAudienceScope::Phs => "phs",
            GuestAudienceScope::Pvs => "pvs",
        }
    }
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

/// Mirror of `ClassicGuestAudience` from donor
/// `src/app/bootstrap/simulation/classic-guest-services.ts` (canonical home:
/// `crate::bootstrap::simulation::classic_guest_services`; kept local: the
/// canonical audience carries `Vec3`/`MulticastScope` while this wire-facing
/// mirror carries `Q2Vec3`/`GuestAudienceScope`).
#[derive(Debug, Clone, PartialEq)]
pub enum ClassicGuestAudience {
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

/// Mirror of `ClassicGuestMessage` from donor
/// `src/app/bootstrap/simulation/classic-guest-services.ts` (canonical home:
/// `crate::bootstrap::simulation::classic_guest_services`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicGuestMessage {
    /// Message audience.
    pub audience: ClassicGuestAudience,
    /// Reliable delivery.
    pub reliable: bool,
    /// Wire bytes.
    pub bytes: Vec<u8>,
}

/// Guest-client phase (canonical home:
/// [`super::classic_guest_world::ClassicGuestClientPhase`]).
pub use super::classic_guest_world::ClassicGuestClientPhase;

/// Connected guest client record (canonical home:
/// [`super::classic_guest_world::ClassicGuestClient`]).
pub use super::classic_guest_world::ClassicGuestClient;

/// Mirror of the donor `connect` outcome from
/// `src/app/bootstrap/simulation/classic-guest-world.ts` (canonical home:
/// `crate::bootstrap::simulation::classic_guest_world`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestConnectOutcome {
    /// Connection allowed.
    pub allowed: bool,
    /// Result userinfo.
    pub userinfo: String,
}

/// Mirror of the donor `entityInfo` result from
/// `src/app/bootstrap/simulation/classic-guest-world.ts` (canonical home:
/// `crate::bootstrap::simulation::classic_guest_world`); unify post-merge.
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
pub enum Q2GuestHostError {
    /// Non-classic protocol.
    #[error("Classic guest server requires protocol 34")]
    RequiresClassicProtocol,
    /// Retired network player.
    #[error("Q2 guest network player is retired")]
    PlayerRetired,
    /// Headnode outside the shared BSP.
    #[error("Guest visibility headnode is outside the shared BSP")]
    HeadnodeOutsideBsp,
    /// Carried client owned elsewhere.
    #[error("Q2 guest carried client is not owned by this world")]
    CarriedNotOwned,
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

/// Classic guest world surface used by this host.
///
/// Seam over `ClassicGuestWorld` from donor
/// `src/app/bootstrap/simulation/classic-guest-world.ts` (canonical home:
/// `crate::bootstrap::simulation::classic_guest_world`); the guest
/// partition implements it post-merge.
pub trait ClassicGuestWorld {
    /// Donor `services.options.maxClients`.
    fn max_clients(&self) -> u32;
    /// Donor `services.options.mapPath`.
    fn map_path(&self) -> &str;
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
    fn player_state(&self, slot: u32) -> Q2PlayerState;
    /// Donor `entityState`.
    fn entity_state(&self, slot: u32) -> Q2EntityState;
    /// Donor `entityStates`.
    fn entity_states(&self) -> Vec<Q2EntityState>;
    /// Donor `entityInfo`.
    fn entity_info(&self, slot: u32) -> GuestEntityInfo;
    /// Donor `clients`.
    fn clients(&self) -> Vec<ClassicGuestClient>;
    /// Donor `playerPing`.
    fn player_ping(&self, slot: u32) -> i32;
    /// Donor `connect`; the message propagates like the donor throw.
    fn connect(&mut self, slot: u32, userinfo: &str) -> Result<GuestConnectOutcome, String>;
    /// Donor `disconnect`; the message propagates like the donor throw.
    fn disconnect(&mut self, slot: u32) -> Result<(), String>;
    /// Donor `begin`.
    fn begin(&mut self, slot: u32);
    /// Donor `userinfo`.
    fn userinfo(&mut self, slot: u32, value: &str);
    /// Donor `command`.
    fn command(&mut self, slot: u32, argv: &[String], args: &str);
    /// Donor `think`.
    fn think(&mut self, slot: u32, command: &Q2UserCommand);
    /// Donor `rawMessages`.
    fn raw_messages(&mut self) -> Vec<ClassicGuestMessage>;
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
pub trait ClassicGuestHostContent {
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

/// Mirror of `Q2ApplicationServerBindingOptions` from donor
/// `src/app/bootstrap/simulation/network.ts` (canonical home:
/// `crate::bootstrap::simulation::network`); unify post-merge.
///
/// Only the surface `createClassicQ2ApplicationServerHost` uses is
/// mirrored; the downloads handle arrives preconstructed because
/// `createQ2ApplicationDownloads` belongs to the downloads partition.
pub struct Q2ClassicServerOptions<S, C, E, W, D, A> {
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
    /// Print sink.
    pub print: Q2PrintFn,
    /// Classic guest world.
    pub world: W,
    /// Application downloads handle.
    pub downloads: D,
    /// Userinfo parser.
    pub parse_userinfo: ParseQ2UserinfoFn,
}

/// Classic guest server host.
///
/// Concrete mirror of `Q2ApplicationServerHost` from donor
/// `src/app/bootstrap/network/types.ts` (canonical home:
/// `crate::bootstrap::network::types`); unify post-merge.
pub struct Q2ClassicGuestHost<S, C, E, W, D, A> {
    /// Engine session.
    session: E,
    /// Address rejection predicate.
    rejects: Option<Q2RejectsFn>,
    /// Shared simulation.
    simulation: S,
    /// Loaded content.
    content: C,
    /// Classic guest world.
    world: W,
    /// Application downloads handle.
    downloads: D,
    /// Rcon administration host.
    administration: Option<A>,
    /// Master server listing.
    masters: Option<Q2MastersFn>,
    /// Print sink.
    print: Q2PrintFn,
    /// Protocol identity.
    protocol: ProtocolIdentity,
    /// Maximum clients.
    max_clients: u32,
    /// Map name.
    map: String,
    /// Admitted players by client slot.
    players: HashMap<u32, Q2ApplicationPlayer>,
    /// Retained guest messages.
    messages: Vec<ClassicGuestMessage>,
    /// Fresh local guest messages.
    local_messages: Vec<ClassicGuestMessage>,
    /// Progressed guest messages awaiting publication.
    progressed_messages: Vec<ClassicGuestMessage>,
    /// Clients that already consumed local messages.
    local_recipients: HashSet<ClientId>,
    /// Userinfo parser.
    parse_userinfo: ParseQ2UserinfoFn,
}

impl<S, C, E, W, D, A> std::fmt::Debug for Q2ClassicGuestHost<S, C, E, W, D, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q2ClassicGuestHost")
            .field("protocol", &self.protocol)
            .field("max_clients", &self.max_clients)
            .field("map", &self.map)
            .field("players", &self.players)
            .field("messages", &self.messages)
            .finish_non_exhaustive()
    }
}

/// Create the classic guest host, mirroring donor
/// `createClassicQ2ApplicationServerHost`.
pub fn create_classic_q2_application_server_host<S, C, E, W, D, A>(
    options: Q2ClassicServerOptions<S, C, E, W, D, A>,
) -> Result<Q2ClassicGuestHost<S, C, E, W, D, A>, Q2GuestHostError>
where
    S: Q2GuestHostSimulation,
    C: ClassicGuestHostContent,
    E: Q2GuestHostSession,
    W: ClassicGuestWorld,
{
    if !matches!(options.protocol, ProtocolIdentity::Q2Classic) {
        return Err(Q2GuestHostError::RequiresClassicProtocol);
    }
    let mut world = options.world;
    let max_clients = world.max_clients();
    let stripped = world.map_path().strip_prefix("maps/").unwrap_or(world.map_path());
    let map = stripped.strip_suffix(".bsp").unwrap_or(stripped).to_string();
    world
        .cvars_mut()
        .register("hostname", "noname", q2_flags::SERVER_INFO | q2_flags::ARCHIVE)?;
    for (name, value) in [("protocol", "34"), ("mapname", map.as_str())] {
        world
            .cvars_mut()
            .register(name, value, q2_flags::SERVER_INFO | q2_flags::NO_SET)?;
        world.cvars_mut().set(name, value, true)?;
    }
    let checksum = block_checksum(&options.content.map_geometry_bytes()?)?;
    world.set_configstring(31, &checksum.to_string());
    world.set_configstring(30, &max_clients.to_string());
    let air = world.cvars().variable_string("sv_airaccelerate");
    world.set_configstring(29, &air);
    Ok(Q2ClassicGuestHost {
        session: options.session,
        rejects: options.rejects,
        simulation: options.simulation,
        content: options.content,
        world,
        downloads: options.downloads,
        administration: options.administration,
        masters: options.masters,
        print: options.print,
        protocol: options.protocol,
        max_clients,
        map,
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
fn wrap_game_callback<T>(result: Result<T, Q2GuestHostError>) -> Result<T, Q2GuestHostError> {
    result.map_err(|error| match error {
        Q2GuestHostError::GameCallback(_) => error,
        other => Q2GuestHostError::GameCallback(other.to_string()),
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

impl<S, C, E, W, D, A> Q2ClassicGuestHost<S, C, E, W, D, A>
where
    S: Q2GuestHostSimulation,
    C: ClassicGuestHostContent,
    E: Q2GuestHostSession,
    W: ClassicGuestWorld,
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
        Q2ServerMessageOptions::default()
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
    fn require_player(&self, player: &Q2ApplicationPlayer) -> Result<(), Q2GuestHostError> {
        match self.players.get(&player.client.slot()) {
            Some(current) if current.client == player.client && current.actor == player.actor => Ok(()),
            _ => Err(Q2GuestHostError::PlayerRetired),
        }
    }

    /// Donor `nativeState`.
    fn native_state(&self, player: &Q2ApplicationPlayer) -> Result<Q2PlayerState, Q2GuestHostError> {
        self.require_player(player)?;
        Ok(self.world.player_state(player.source_entity))
    }

    /// Donor `viewOrigin`.
    fn view_origin(&self, player: &Q2ApplicationPlayer) -> Result<Q2Vec3, Q2GuestHostError> {
        let state = self.native_state(player)?;
        let numeric = self.world.numeric();
        Ok(Q2Vec3 {
            x: numeric.add(
                numeric.divide(f64::from(state.movement.origin_eighths[0]), 8.0),
                state.view.view_offset.x,
            ),
            y: numeric.add(
                numeric.divide(f64::from(state.movement.origin_eighths[1]), 8.0),
                state.view.view_offset.y,
            ),
            z: numeric.add(
                numeric.divide(f64::from(state.movement.origin_eighths[2]), 8.0),
                state.view.view_offset.z,
            ),
        })
    }

    /// Donor `headnodeVisible`.
    fn headnode_visible(&self, headnode: i32, clusters: &[i32], nodes: &[Node]) -> Result<bool, Q2GuestHostError> {
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
                        .any(|cluster| scene.cluster_visible(*cluster, target, ClusterVisibility::Pvs))
                    {
                        return Ok(true);
                    }
                }
                NodeChild::Node(index) => {
                    let node = nodes.get(index as usize).ok_or(Q2GuestHostError::HeadnodeOutsideBsp)?;
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
        incoming: &[ClassicGuestMessage],
    ) -> Result<Vec<ClassicGuestMessage>, Q2GuestHostError> {
        self.require_player(player)?;
        let mut result = Vec::new();
        for message in incoming {
            let keep = match &message.audience {
                ClassicGuestAudience::Unicast { slot } => *slot == player.source_entity,
                ClassicGuestAudience::Multicast { origin, scope } => {
                    if *scope == GuestAudienceScope::All {
                        true
                    } else {
                        let entity = self.world.entity_state(player.source_entity);
                        let scene = self.simulation.scene();
                        let from = scene.point_leaf(origin);
                        let to = scene.point_leaf(&Q2Vec3 {
                            x: entity.origin.x,
                            y: entity.origin.y,
                            z: entity.origin.z,
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
    pub fn discovery_status(&mut self) -> Result<Q2Status, Q2GuestHostError> {
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

    /// Donor `admit`.
    pub fn admit(
        &mut self,
        from: &NetworkAddress,
        request: &Q2ConnectRequest,
    ) -> Result<Q2ApplicationAdmission, Q2GuestHostError> {
        wrap_game_callback(self.admit_inner(from, request))
    }

    /// Donor `admit` body.
    fn admit_inner(
        &mut self,
        from: &NetworkAddress,
        request: &Q2ConnectRequest,
    ) -> Result<Q2ApplicationAdmission, Q2GuestHostError> {
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
        self.session.connect_client(&client, NetClientOrigin::Remote);
        let mut info = (self.parse_userinfo)(&request.userinfo);
        set_userinfo(&mut info, "ip", address_key(from, true));
        let outcome = match self.world.connect(slot + 1, &join_userinfo(&info)) {
            Ok(outcome) => outcome,
            Err(message) => {
                self.session
                    .close_client(&client)
                    .map_err(Q2GuestHostError::SessionClose)?;
                return Err(Q2GuestHostError::GameCallback(message));
            }
        };
        if !outcome.allowed {
            self.session
                .close_client(&client)
                .map_err(Q2GuestHostError::SessionClose)?;
            return Ok(Q2ApplicationAdmission::Rejected {
                reason: self
                    .userinfo_value(&outcome.userinfo, "rejmsg")
                    .unwrap_or_else(|| "Connection refused".to_string()),
            });
        }
        let owned = match self.simulation.register_q2_native_client(&client) {
            Ok(owned) => owned,
            Err(message) => {
                let cleanup = self.world.disconnect(slot + 1);
                let close = self
                    .session
                    .close_client(&client)
                    .map_err(Q2GuestHostError::SessionClose);
                match (cleanup, close) {
                    (Ok(()), Ok(())) => return Err(Q2GuestHostError::GameCallback(message)),
                    (Err(cleanup), Ok(())) => {
                        return Err(Q2GuestHostError::GameCallback(
                            Q2GuestHostError::ConnectCleanup(vec![message, cleanup]).to_string(),
                        ));
                    }
                    (_, Err(close)) => return Err(close),
                }
            }
        };
        let player = Q2ApplicationPlayer {
            client: client.clone(),
            actor: owned.id().clone(),
            source_entity: slot + 1,
        };
        self.players.insert(slot, player.clone());
        Ok(Q2ApplicationAdmission::Accepted { player })
    }

    /// Donor `carriedPlayer`.
    pub fn carried_player(&mut self, client: &ClientId) -> Result<Q2ApplicationPlayer, Q2GuestHostError> {
        let actor = self
            .simulation
            .q2_native_players()
            .into_iter()
            .find(|player| player.client == *client)
            .map(|player| player.actor)
            .ok_or(Q2GuestHostError::CarriedNotOwned)?;
        let player = Q2ApplicationPlayer {
            client: client.clone(),
            actor,
            source_entity: client.slot() + 1,
        };
        self.players.insert(client.slot(), player.clone());
        Ok(player)
    }

    /// Donor `begin`.
    pub fn begin(&mut self, player: &Q2ApplicationPlayer) -> Result<(), Q2GuestHostError> {
        wrap_game_callback(self.begin_inner(player))
    }

    /// Donor `begin` body.
    fn begin_inner(&mut self, player: &Q2ApplicationPlayer) -> Result<(), Q2GuestHostError> {
        self.require_player(player)?;
        self.world.begin(player.source_entity);
        self.simulation
            .notify_client_event(ModClientEventKind::Admitted, &player.actor);
        Ok(())
    }

    /// Donor `disconnect`; the interface reason is unused by the donor body.
    pub fn disconnect(&mut self, player: &Q2ApplicationPlayer, _reason: &str) -> Result<(), Q2GuestHostError> {
        wrap_game_callback(self.disconnect_inner(player))
    }

    /// Donor `disconnect` body.
    fn disconnect_inner(&mut self, player: &Q2ApplicationPlayer) -> Result<(), Q2GuestHostError> {
        self.require_player(player)?;
        let mut failures = Vec::new();
        if !self.world.is_retired() {
            self.simulation.disconnect_player(&player.actor);
        }
        self.players.remove(&player.client.slot());
        if let Err(message) = self.session.close_client(&player.client) {
            failures.push(message);
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(Q2GuestHostError::ClientCleanup(failures))
        }
    }

    /// Donor `gameState`.
    pub fn game_state(&self, player: &Q2ApplicationPlayer) -> Result<Q2ApplicationGameState, Q2GuestHostError> {
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
                state.model_indexes.iter().any(|index| *index != 0) || state.sound != 0 || state.effects != 0
            })
            .map(|state| (state.number, from_q2_entity(&Q2Entity::Classic(state.clone()))))
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
                server_fps: Some(10.0),
            },
            config_strings: configs,
            baselines,
        })
    }

    /// Donor `frame`.
    pub fn frame(
        &self,
        player: &Q2ApplicationPlayer,
        output: &qa_world::session::SimulationOutput,
    ) -> Result<Q2WireFrame, Q2GuestHostError> {
        let origin = self.view_origin(player)?;
        let scene = self.simulation.scene();
        let numeric = self.world.numeric();
        let leaf = scene.point_leaf(&origin);
        let area = scene.leaf_area(leaf);
        let cluster = scene.leaf_cluster(leaf);
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
        let nodes = self.content.q2_nodes();
        let mut entities = Vec::new();
        for state in self.world.entity_states() {
            if !state.model_indexes.iter().any(|index| *index != 0)
                && state.effects == 0
                && state.sound == 0
                && state.event == 0
            {
                continue;
            }
            if u32::from(state.number) != player.source_entity {
                let info = self.world.entity_info(u32::from(state.number));
                if !scene.areas_connected(area, info.areas[0])
                    && (info.areas[1] == 0 || !scene.areas_connected(area, info.areas[1]))
                {
                    continue;
                }
                if state.render_effects & 128 != 0 {
                    if !scene.cluster_visible(cluster, info.first_cluster, ClusterVisibility::Phs) {
                        continue;
                    }
                } else {
                    let visible = match &info.clusters {
                        None => {
                            let nodes = nodes.ok_or(Q2GuestHostError::HeadnodeOutsideBsp)?;
                            self.headnode_visible(info.headnode, &clusters, nodes)?
                        }
                        Some(targets) => targets.iter().any(|target| {
                            clusters
                                .iter()
                                .any(|from| scene.cluster_visible(*from, *target, ClusterVisibility::Pvs))
                        }),
                    };
                    if !visible {
                        continue;
                    }
                    let dx = numeric.subtract(origin.x, state.origin.x);
                    let dy = numeric.subtract(origin.y, state.origin.y);
                    let dz = numeric.subtract(origin.z, state.origin.z);
                    let distance = numeric.square_root(numeric.add(
                        numeric.add(numeric.multiply(dx, dx), numeric.multiply(dy, dy)),
                        numeric.multiply(dz, dz),
                    ));
                    if state.model_indexes[0] == 0 && distance > 400.0 {
                        continue;
                    }
                }
            }
            let mut wire = from_q2_entity(&Q2Entity::Classic(state.clone()));
            if self.world.entity_info(u32::from(state.number)).owner_slot == Some(player.source_entity) {
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
            player: from_q2_player(&Q2Player::Classic(self.native_state(player)?))?,
            split_players: Vec::new(),
            entities,
        })
    }

    /// Donor `observe`.
    pub fn observe(&mut self) {
        let air = self.world.cvars().variable_string("sv_airaccelerate");
        if self.world.configstrings().get(&29).cloned().unwrap_or_default() != air {
            self.world.set_configstring(29, &air);
        }
        self.local_recipients.clear();
        self.local_messages = self.world.raw_messages();
        let mut messages = std::mem::take(&mut self.progressed_messages);
        messages.extend(self.local_messages.clone());
        self.messages = messages;
    }

    /// Donor `observeProgress`.
    pub fn observe_progress(&mut self) {
        self.local_recipients.clear();
        self.local_messages = self.world.raw_messages();
        self.progressed_messages.extend(self.local_messages.clone());
    }

    /// Donor `localMessages`.
    pub fn local_messages(
        &mut self,
        player: &Q2ApplicationPlayer,
    ) -> Result<Vec<Q2GuestByteMessage>, Q2GuestHostError> {
        self.require_player(player)?;
        if !self.local_recipients.insert(player.client.clone()) {
            return Ok(Vec::new());
        }
        let local = self.local_messages.clone();
        Ok(self
            .player_messages(player, &local)?
            .into_iter()
            .map(|message| Q2GuestByteMessage {
                bytes: message.bytes,
                reliable: message.reliable,
            })
            .collect())
    }

    /// Donor `rawMessages`.
    pub fn raw_messages(&self, player: &Q2ApplicationPlayer) -> Result<Vec<Q2GuestByteMessage>, Q2GuestHostError> {
        Ok(self
            .player_messages(player, &self.messages.clone())?
            .into_iter()
            .map(|message| Q2GuestByteMessage {
                bytes: message.bytes,
                reliable: message.reliable,
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
    ) -> Result<Option<ActorCommand>, Q2GuestHostError> {
        wrap_game_callback(self.input_inner(player, wire, sequence))
    }

    /// Donor `input` body.
    fn input_inner(
        &mut self,
        player: &Q2ApplicationPlayer,
        wire: &Usercmd,
        sequence: u32,
    ) -> Result<Option<ActorCommand>, Q2GuestHostError> {
        self.require_player(player)?;
        let command = to_q2_command(wire);
        self.simulation.observe_client_command(
            &player.actor,
            &player.client,
            sequence,
            &Q2Command::Classic(command.clone()),
        );
        self.world.think(player.source_entity, &command);
        Ok(None)
    }

    /// Donor `expandClientCommand`.
    pub fn expand_client_command(&self, text: &str) -> Result<Option<String>, Q2GuestHostError> {
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
    ) -> Result<(), Q2GuestHostError> {
        let mut parts = vec![name.to_string()];
        parts.extend(args.iter().cloned());
        self.command_text(player, &parts.join(" "))
    }

    /// Donor `commandText`.
    pub fn command_text(&mut self, player: &Q2ApplicationPlayer, text: &str) -> Result<(), Q2GuestHostError> {
        wrap_game_callback(self.command_text_inner(player, text))
    }

    /// Donor `commandText` body.
    fn command_text_inner(&mut self, player: &Q2ApplicationPlayer, text: &str) -> Result<(), Q2GuestHostError> {
        self.require_player(player)?;
        let tokens = tokenize_command(text, Dialect::Q2Classic, TextMode::Source)?;
        if !tokens.argv.is_empty() {
            self.world
                .command(player.source_entity, &tokens.argv, &tokens.args_text);
        }
        Ok(())
    }

    /// Donor `userinfo`.
    pub fn userinfo(&mut self, player: &Q2ApplicationPlayer, value: &str) -> Result<(), Q2GuestHostError> {
        wrap_game_callback(self.userinfo_inner(player, value))
    }

    /// Donor `userinfo` body.
    fn userinfo_inner(&mut self, player: &Q2ApplicationPlayer, value: &str) -> Result<(), Q2GuestHostError> {
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
    use qa_content::bsp::IndexRange;
    use qa_content::common::Bounds;
    use qa_core::cmd::Dialect;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::time::{FrameContext, FramePhase, SourceTime};
    use qa_net::q2_adapters::{Q2MovementState, Q2PlayerView};
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
        player_states: HashMap<u32, Q2PlayerState>,
        entity_states: HashMap<u32, Q2EntityState>,
        entity_order: Vec<u32>,
        entity_infos: HashMap<u32, GuestEntityInfo>,
        clients: Vec<ClassicGuestClient>,
        pings: HashMap<u32, i32>,
        connect_result: Result<GuestConnectOutcome, String>,
        connect_calls: Vec<(u32, String)>,
        disconnect_err: Option<String>,
        disconnect_calls: Vec<u32>,
        begun: Vec<u32>,
        userinfos: Vec<(u32, String)>,
        commands: Vec<(u32, Vec<String>, String)>,
        thinks: Vec<(u32, Q2UserCommand)>,
        raw: Vec<ClassicGuestMessage>,
        retired: bool,
    }

    impl StubWorld {
        fn new() -> Self {
            let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
            cvars.register("sv_airaccelerate", "150", 0).unwrap();
            Self {
                numeric: StubNumeric,
                cvars,
                configs: HashMap::new(),
                max_clients: 2,
                map_path: "maps/base1.bsp".to_string(),
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
                areas: [1, 0],
                clusters: Some(vec![5]),
                first_cluster: 5,
                headnode: 0,
                owner_slot: None,
            }
        }
    }

    impl ClassicGuestWorld for StubWorld {
        fn max_clients(&self) -> u32 {
            self.max_clients
        }

        fn map_path(&self) -> &str {
            &self.map_path
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

        fn player_state(&self, slot: u32) -> Q2PlayerState {
            self.player_states[&slot].clone()
        }

        fn entity_state(&self, slot: u32) -> Q2EntityState {
            self.entity_states[&slot].clone()
        }

        fn entity_states(&self) -> Vec<Q2EntityState> {
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

        fn connect(&mut self, slot: u32, userinfo: &str) -> Result<GuestConnectOutcome, String> {
            self.connect_calls.push((slot, userinfo.to_string()));
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
                None => Ok(()),
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

        fn think(&mut self, slot: u32, command: &Q2UserCommand) {
            self.thinks.push((slot, command.clone()));
        }

        fn raw_messages(&mut self) -> Vec<ClassicGuestMessage> {
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

    impl ClassicGuestHostContent for StubContent {
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

    type StubHost = Q2ClassicGuestHost<StubSim, StubContent, StubSession, StubWorld, (), ()>;

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
            let owner = IdentityOwner::create("q2-guest-test").unwrap();
            let actor1 = owner.actor(1, 1);
            let actor2 = owner.actor(2, 1);
            let provider = ProviderId::new("q2", "game");
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
        actor2: ActorId,
        client0: ClientId,
        client9: ClientId,
    }

    fn player_state() -> Q2PlayerState {
        let mut stats = vec![0_i16; 32];
        stats[14] = 15;
        Q2PlayerState {
            view: Q2PlayerView {
                view_offset: Q2Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                stats,
                ..Default::default()
            },
            movement: Q2MovementState {
                origin_eighths: [800, 1600, 2400],
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn standard_world() -> StubWorld {
        let mut world = StubWorld::new();
        world.player_states.insert(1, player_state());
        world.entity_states.insert(
            1,
            Q2EntityState {
                number: 1,
                origin: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                model_indexes: [1, 0, 0, 0],
                ..Default::default()
            },
        );
        world.entity_order.push(1);
        world.entity_infos.insert(1, StubWorld::info());
        world
    }

    fn standard_host() -> (StubHost, Ids) {
        standard_host_with(ProtocolIdentity::Q2Classic)
    }

    fn standard_host_with(protocol: ProtocolIdentity) -> (StubHost, Ids) {
        let fixture = Fixture::new();
        let ids = Ids {
            actor1: fixture.actor1.clone(),
            actor2: fixture.actor2.clone(),
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
        let options = Q2ClassicServerOptions {
            session,
            rejects: None,
            simulation: sim,
            content,
            protocol,
            administration: None::<()>,
            masters: None,
            print: Rc::new(|_| {}),
            world: standard_world(),
            downloads: (),
            parse_userinfo: Rc::new(parse_userinfo),
        };
        let host = create_classic_q2_application_server_host(options).unwrap();
        (host, ids)
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
            protocol: ProtocolIdentity::Q2Classic,
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
        let admission = host.admit(&loopback(), &request(userinfo)).unwrap();
        let Q2ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        player
    }

    #[test]
    fn rejects_non_classic_protocol() {
        let fixture = Fixture::new();
        let mut sim = StubSim::new();
        sim.register_ok.insert(fixture.client0.clone(), fixture.owned1.clone());
        let options = Q2ClassicServerOptions {
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
            protocol: ProtocolIdentity::Q2Rerelease,
            administration: None::<()>,
            masters: None,
            print: Rc::new(|_| {}),
            world: StubWorld::new(),
            downloads: (),
            parse_userinfo: Rc::new(parse_userinfo),
        };
        assert!(matches!(
            create_classic_q2_application_server_host(options).unwrap_err(),
            Q2GuestHostError::RequiresClassicProtocol
        ));
    }

    #[test]
    fn constructor_registers_cvars_and_configstrings() {
        let (host, _ids) = standard_host();
        assert_eq!(host.map(), "base1");
        assert_eq!(host.max_clients(), 2);
        assert_eq!(host.protocol(), ProtocolIdentity::Q2Classic);
        assert_eq!(host.world.cvars.variable_string("hostname"), "noname");
        assert_eq!(host.world.cvars.variable_string("protocol"), "34");
        assert_eq!(host.world.cvars.variable_string("mapname"), "base1");
        assert_eq!(
            host.world.configs.get(&31).cloned().unwrap(),
            block_checksum(b"fake-bsp").unwrap().to_string()
        );
        assert_eq!(host.world.configs.get(&30).cloned().unwrap(), "2");
        assert_eq!(host.world.configs.get(&29).cloned().unwrap(), "150");
        assert_eq!(host.message_options().max_config_strings, 2080);
        assert_eq!(host.message_options().inventory_slots, 256);
    }

    #[test]
    fn supports_source_wire_checks_world_and_recipe() {
        let (mut host, _ids) = standard_host();
        assert!(matches!(host.supports_source_wire(), WireAdmission::Supported));
        host.content.kind = "q1-bsp";
        host.simulation.recipe.movement = "q1:move".to_string();
        host.simulation.recipe.character_definition = "q3:char".to_string();
        let WireAdmission::Unsupported { reasons } = host.supports_source_wire() else {
            panic!("expected unsupported");
        };
        assert_eq!(reasons.len(), 3);
    }

    #[test]
    fn admit_roundtrip_rejections_and_cleanup() {
        let (mut host, ids) = standard_host();
        let player = admit(&mut host, "\\name\\solo");
        assert_eq!(player.client, ids.client0);
        assert_eq!(player.actor, ids.actor1);
        assert_eq!(player.source_entity, 1);
        assert_eq!(
            host.session.connected,
            vec![(ids.client0.clone(), NetClientOrigin::Remote)]
        );
        assert_eq!(
            host.world.connect_calls,
            vec![(1, "\\name\\solo\\ip\\loopback:test".to_string())]
        );

        host.world.max_clients = 1;
        host.max_clients = 1;
        let admission = host.admit(&loopback(), &request("\\name\\full")).unwrap();
        assert!(matches!(
            admission,
            Q2ApplicationAdmission::Rejected { reason } if reason == "Server is full"
        ));

        let (mut host, ids) = standard_host();
        host.world.connect_result = Ok(GuestConnectOutcome {
            allowed: false,
            userinfo: "\\rejmsg\\banned".to_string(),
        });
        let admission = host.admit(&loopback(), &request("\\name\\solo")).unwrap();
        assert!(matches!(
            admission,
            Q2ApplicationAdmission::Rejected { reason } if reason == "banned"
        ));
        assert_eq!(host.session.closed, vec![ids.client0.clone()]);
        host.world.connect_result = Ok(GuestConnectOutcome {
            allowed: false,
            userinfo: String::new(),
        });
        let admission = host.admit(&loopback(), &request("\\name\\solo")).unwrap();
        assert!(matches!(
            admission,
            Q2ApplicationAdmission::Rejected { reason } if reason == "Connection refused"
        ));

        host.world.connect_result = Err("denied".to_string());
        let error = host.admit(&loopback(), &request("\\name\\solo")).unwrap_err();
        assert_eq!(error.to_string(), "Q2 game callback failed: denied");
        assert!(host.players.is_empty());

        let (mut host, _ids) = standard_host();
        host.simulation.register_err = Some("bad actor".to_string());
        let error = host.admit(&loopback(), &request("\\name\\solo")).unwrap_err();
        assert_eq!(error.to_string(), "Q2 game callback failed: bad actor");
        assert_eq!(host.world.disconnect_calls, vec![1]);
        assert_eq!(host.session.closed.len(), 1);

        host.world.disconnect_err = Some("stuck".to_string());
        let error = host.admit(&loopback(), &request("\\name\\solo")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Q2 guest connect cleanup failed: bad actor; stuck"),
            "{error}"
        );
    }

    #[test]
    fn admit_skips_occupied_world_slots() {
        let fixture = Fixture::new();
        let mut world = standard_world();
        world.clients.push(ClassicGuestClient {
            slot: 1,
            phase: ClassicGuestClientPhase::Connected,
            userinfo: "\\name\\taken".to_string(),
        });
        let mut sim = StubSim::new();
        sim.register_ok.insert(fixture.client1.clone(), fixture.owned2.clone());
        let content = StubContent {
            geometry: b"fake-bsp".to_vec(),
            directory: "baseq2".to_string(),
            kind: "q2-bsp",
            nodes: None,
        };
        let session = StubSession {
            owner: fixture.owner,
            generations: HashMap::new(),
            connected: Vec::new(),
            closed: Vec::new(),
            close_err: None,
        };
        let client1 = sim.register_ok.keys().next().cloned().unwrap();
        let actor2 = sim.register_ok.values().next().cloned().unwrap();
        let mut host = create_classic_q2_application_server_host(Q2ClassicServerOptions {
            session,
            rejects: None,
            simulation: sim,
            content,
            protocol: ProtocolIdentity::Q2Classic,
            administration: None::<()>,
            masters: None,
            print: Rc::new(|_| {}),
            world,
            downloads: (),
            parse_userinfo: Rc::new(parse_userinfo),
        })
        .unwrap();
        let admission = host.admit(&loopback(), &request("\\name\\solo")).unwrap();
        let Q2ApplicationAdmission::Accepted { player } = admission else {
            panic!("expected admission");
        };
        assert_eq!(player.client, client1);
        assert_eq!(player.actor, actor2.id().clone());
        assert_eq!(player.source_entity, 2);
    }

    #[test]
    fn carried_begin_and_disconnect() {
        let (mut host, ids) = standard_host();
        assert!(matches!(
            host.carried_player(&ids.client9).unwrap_err(),
            Q2GuestHostError::CarriedNotOwned
        ));
        let player = host.carried_player(&ids.client0).unwrap();
        assert_eq!(player.source_entity, 1);
        host.begin(&player).unwrap();
        assert_eq!(host.world.begun, vec![1]);
        assert_eq!(
            host.simulation.notified,
            vec![(ModClientEventKind::Admitted, ids.actor1.clone())]
        );

        let forged = Q2ApplicationPlayer {
            client: ids.client0.clone(),
            actor: ids.actor2.clone(),
            source_entity: 1,
        };
        let error = host.begin(&forged).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Q2 game callback failed: Q2 guest network player is retired"
        );

        host.disconnect(&player, "bye").unwrap();
        assert_eq!(host.simulation.disconnects, vec![ids.actor1.clone()]);
        assert_eq!(host.session.closed, vec![ids.client0.clone()]);
        assert!(host.players.is_empty());

        let player = host.carried_player(&ids.client0).unwrap();
        host.world.retired = true;
        host.simulation.disconnects.clear();
        host.disconnect(&player, "bye").unwrap();
        assert!(host.simulation.disconnects.is_empty());

        let player = host.carried_player(&ids.client0).unwrap();
        host.world.retired = false;
        host.session.close_err = Some("nope".to_string());
        let error = host.disconnect(&player, "bye").unwrap_err();
        assert!(error.to_string().contains("Q2 guest client cleanup failed"), "{error}");
    }

    #[test]
    fn game_state_baselines_and_gamedir() {
        let (host, ids) = standard_host();
        let player = Q2ApplicationPlayer {
            client: ids.client0.clone(),
            actor: ids.actor1.clone(),
            source_entity: 1,
        };
        assert!(matches!(
            host.game_state(&player).unwrap_err(),
            Q2GuestHostError::PlayerRetired
        ));
        let (mut host, _ids) = standard_host();
        host.world.entity_states.insert(
            2,
            Q2EntityState {
                number: 2,
                ..Default::default()
            },
        );
        host.world.entity_order.push(2);
        host.world.entity_infos.insert(2, StubWorld::info());
        host.world.entity_states.insert(
            3,
            Q2EntityState {
                number: 3,
                sound: 4,
                ..Default::default()
            },
        );
        host.world.entity_order.push(3);
        host.world.entity_infos.insert(3, StubWorld::info());
        host.world.entity_states.insert(
            4,
            Q2EntityState {
                number: 4,
                effects: 8,
                ..Default::default()
            },
        );
        host.world.entity_order.push(4);
        host.world.entity_infos.insert(4, StubWorld::info());
        let player = admit(&mut host, "\\name\\solo");
        let state = host.game_state(&player).unwrap();
        assert_eq!(state.data.base.servercount, 1);
        assert!(!state.data.base.attractloop);
        assert_eq!(state.data.base.gamedir, "baseq2");
        assert_eq!(state.data.base.clientnum, 0);
        assert_eq!(state.data.base.levelname, "base1");
        assert_eq!(state.data.server_state, 2);
        assert_eq!(state.data.server_fps, Some(10.0));
        assert_eq!(state.config_strings.get(&30).cloned().unwrap(), "2");
        let mut keys: Vec<u16> = state.baselines.keys().copied().collect();
        keys.sort_unstable();
        assert_eq!(keys, vec![1, 3, 4]);

        host.world.configs.insert(0, "Level One".to_string());
        host.content.directory = "baseq2".to_string();
        let state = host.game_state(&player).unwrap();
        assert_eq!(state.data.base.levelname, "Level One");
        assert_eq!(state.data.base.gamedir, "baseq2");
    }

    #[test]
    fn frame_filters_visibility_and_owner() {
        let (mut host, _ids) = standard_host();
        host.world.entity_states.insert(
            5,
            Q2EntityState {
                number: 5,
                ..Default::default()
            },
        );
        host.world.entity_order.push(5);
        host.world.entity_infos.insert(5, StubWorld::info());
        host.world.entity_states.insert(
            6,
            Q2EntityState {
                number: 6,
                model_indexes: [2, 0, 0, 0],
                ..Default::default()
            },
        );
        host.world.entity_order.push(6);
        let mut areas = StubWorld::info();
        areas.areas = [2, 0];
        host.world.entity_infos.insert(6, areas);
        host.world.entity_states.insert(
            7,
            Q2EntityState {
                number: 7,
                origin: Q2Vec3 {
                    x: 1000.0,
                    y: 0.0,
                    z: 0.0,
                },
                model_indexes: [3, 0, 0, 0],
                render_effects: 128,
                ..Default::default()
            },
        );
        host.world.entity_order.push(7);
        host.world.entity_infos.insert(7, StubWorld::info());
        host.world.entity_states.insert(
            8,
            Q2EntityState {
                number: 8,
                origin: Q2Vec3 {
                    x: 1000.0,
                    y: 0.0,
                    z: 0.0,
                },
                model_indexes: [0, 1, 0, 0],
                ..Default::default()
            },
        );
        host.world.entity_order.push(8);
        host.world.entity_infos.insert(8, StubWorld::info());
        host.world.entity_states.insert(
            9,
            Q2EntityState {
                number: 9,
                model_indexes: [4, 0, 0, 0],
                solid: 5,
                ..Default::default()
            },
        );
        host.world.entity_order.push(9);
        let mut owner = StubWorld::info();
        owner.owner_slot = Some(1);
        host.world.entity_infos.insert(9, owner);
        host.world.entity_states.insert(
            10,
            Q2EntityState {
                number: 10,
                model_indexes: [5, 0, 0, 0],
                ..Default::default()
            },
        );
        host.world.entity_order.push(10);
        let mut headnode = StubWorld::info();
        headnode.clusters = None;
        host.world.entity_infos.insert(10, headnode);

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
        host.simulation.scene.leaf_clusters.insert(0, 4);
        host.simulation.scene.leaf_clusters.insert(1, 4);
        host.simulation.scene.disconnected.insert((1, 2));
        host.simulation.scene.area_bits = vec![9, 9];

        let player = admit(&mut host, "\\name\\solo");
        let frame = host.frame(&player, &output()).unwrap();
        assert!(frame.valid);
        assert_eq!(frame.server_frame, 7);
        assert_eq!(frame.delta_frame, -1);
        assert_eq!(frame.suppressed_count, 0);
        assert_eq!(frame.area_bits, vec![9, 9]);
        assert!(frame.split_players.is_empty());
        assert_eq!(frame.player.viewoffset, [1.0, 2.0, 3.0]);
        let numbers: Vec<u16> = frame.entities.iter().map(|entity| entity.number).collect();
        assert!(numbers.contains(&1));
        assert!(!numbers.contains(&5));
        assert!(!numbers.contains(&6));
        // Beam entity 7 is PHS-visible by default, bypassing the distance gate.
        assert!(numbers.contains(&7));
        // Entity 8 is distant with no primary model.
        assert!(!numbers.contains(&8));
        // Owned entity 9 keeps its number with cleared solidity.
        let owned = frame.entities.iter().find(|entity| entity.number == 9).unwrap();
        assert_eq!(owned.solid, 0);
        // Entity 10 resolves through the headnode walk.
        assert!(numbers.contains(&10));

        host.simulation.scene.default_visible = false;
        let frame = host.frame(&player, &output()).unwrap();
        let numbers: Vec<u16> = frame.entities.iter().map(|entity| entity.number).collect();
        assert_eq!(numbers, vec![1]);

        host.simulation.scene.default_visible = true;
        host.simulation
            .scene
            .visibility
            .insert((3, 5, ClusterVisibility::Phs), false);
        let frame = host.frame(&player, &output()).unwrap();
        let numbers: Vec<u16> = frame.entities.iter().map(|entity| entity.number).collect();
        assert!(!numbers.contains(&7));

        host.content.nodes = None;
        assert!(matches!(
            host.frame(&player, &output()).unwrap_err(),
            Q2GuestHostError::HeadnodeOutsideBsp
        ));

        let (mut host, _ids) = standard_host();
        let player = admit(&mut host, "\\name\\solo");
        host.world.player_states.get_mut(&1).unwrap().view.stats = vec![0; 65];
        assert!(matches!(
            host.frame(&player, &output()).unwrap_err(),
            Q2GuestHostError::Adapter(_)
        ));
    }

    #[test]
    fn observe_batches_and_filters_audience() {
        let (mut host, _ids) = standard_host();
        let player = admit(&mut host, "\\name\\solo");
        host.simulation.scene.point_leaves = vec![(
            Q2Vec3 {
                x: 50.0,
                y: 50.0,
                z: 50.0,
            },
            1,
        )];
        host.simulation.scene.leaf_areas.insert(0, 1);
        host.simulation.scene.leaf_areas.insert(1, 2);
        host.simulation.scene.disconnected.insert((1, 2));
        host.world.raw = vec![
            ClassicGuestMessage {
                audience: ClassicGuestAudience::Unicast { slot: 1 },
                reliable: true,
                bytes: vec![1],
            },
            ClassicGuestMessage {
                audience: ClassicGuestAudience::Unicast { slot: 2 },
                reliable: true,
                bytes: vec![2],
            },
            ClassicGuestMessage {
                audience: ClassicGuestAudience::Multicast {
                    origin: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    scope: GuestAudienceScope::All,
                },
                reliable: false,
                bytes: vec![3],
            },
            ClassicGuestMessage {
                audience: ClassicGuestAudience::Multicast {
                    origin: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    scope: GuestAudienceScope::Pvs,
                },
                reliable: false,
                bytes: vec![4],
            },
            ClassicGuestMessage {
                audience: ClassicGuestAudience::Multicast {
                    origin: Q2Vec3 {
                        x: 50.0,
                        y: 50.0,
                        z: 50.0,
                    },
                    scope: GuestAudienceScope::Pvs,
                },
                reliable: false,
                bytes: vec![5],
            },
        ];
        host.observe();
        let local = host.local_messages(&player).unwrap();
        assert_eq!(
            local.iter().map(|message| message.bytes.clone()).collect::<Vec<_>>(),
            vec![vec![1], vec![3], vec![4]]
        );
        assert!(local[0].reliable);
        assert!(!local[1].reliable);
        assert!(host.local_messages(&player).unwrap().is_empty());
        let raw = host.raw_messages(&player).unwrap();
        assert_eq!(raw.len(), 3);

        host.world.raw = vec![ClassicGuestMessage {
            audience: ClassicGuestAudience::Unicast { slot: 1 },
            reliable: false,
            bytes: vec![6],
        }];
        host.observe_progress();
        let local = host.local_messages(&player).unwrap();
        assert_eq!(local.len(), 1);
        host.world.raw = Vec::new();
        host.observe();
        let raw = host.raw_messages(&player).unwrap();
        assert_eq!(
            raw.iter().map(|message| message.bytes.clone()).collect::<Vec<_>>(),
            vec![vec![6]]
        );

        host.world.cvars.set("sv_airaccelerate", "200", true).unwrap();
        host.observe();
        assert_eq!(host.world.configs.get(&29).cloned().unwrap(), "200");
    }

    #[test]
    fn input_command_userinfo_and_expand() {
        let (mut host, ids) = standard_host();
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
            server_frame: 0,
        };
        assert_eq!(host.input(&player, &wire, 9).unwrap(), None);
        let expected = Q2UserCommand {
            milliseconds: 50,
            angle_shorts: [100, 200, 300],
            forward_move: 10,
            side_move: 20,
            up_move: 30,
            buttons: 1,
            impulse: 2,
            light_level: 3,
        };
        assert_eq!(host.world.thinks, vec![(1, expected.clone())]);
        assert_eq!(
            host.simulation.observed,
            vec![(ids.actor1.clone(), ids.client0.clone(), 9, Q2Command::Classic(expected))]
        );

        host.command(&player, "say", &["hi".to_string(), "there".to_string()])
            .unwrap();
        assert_eq!(
            host.world.commands,
            vec![(
                1,
                vec!["say".to_string(), "hi".to_string(), "there".to_string()],
                "hi there".to_string()
            )]
        );
        host.command_text(&player, "  ").unwrap();
        assert_eq!(host.world.commands.len(), 1);

        host.userinfo(&player, "\\name\\new").unwrap();
        assert_eq!(host.world.userinfos, vec![(1, "\\name\\new".to_string())]);
        assert_eq!(
            host.simulation.notified,
            vec![(ModClientEventKind::Userinfo, ids.actor1.clone())]
        );

        host.world.cvars.register("testvar", "v1", 0).unwrap();
        assert_eq!(
            host.expand_client_command("go $testvar").unwrap(),
            Some("go v1".to_string())
        );
        assert_eq!(host.expand_client_command(&"x".repeat(2048)).unwrap(), None);
    }

    #[test]
    fn discovery_reports_status_and_info() {
        let (mut host, _ids) = standard_host();
        host.world.clients.push(ClassicGuestClient {
            slot: 1,
            phase: ClassicGuestClientPhase::Active,
            userinfo: "\\name\\solo".to_string(),
        });
        host.world.pings.insert(1, 42);
        let status = host.discovery_status().unwrap();
        assert!(status.server_info.contains("noname"), "{}", status.server_info);
        assert_eq!(status.players.len(), 1);
        assert_eq!(status.players[0].score, 15);
        assert_eq!(status.players[0].ping, 42);
        assert_eq!(status.players[0].name, "solo");
        let info = host.discovery_info();
        assert_eq!(info.name, "noname");
        assert_eq!(info.map, "base1");
        assert_eq!(info.players, 1);
        assert_eq!(info.max_players, 2);
    }

    #[test]
    fn events_helpers_and_passthrough() {
        let (mut host, _ids) = standard_host();
        assert!(host.events().is_empty());
        assert!(!host.rejected(&loopback()));
        host.rejects = Some(Rc::new(|_| true));
        assert!(host.rejected(&loopback()));
        assert!(host.masters().is_none());
        host.masters = Some(Rc::new(|| vec![loopback()]));
        assert_eq!(host.masters(), Some(vec![loopback()]));
        assert_eq!(host.downloads(), &());
        assert_eq!(host.administration(), None);

        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let pushed = Rc::clone(&seen);
        host.print = Rc::new(move |text| pushed.borrow_mut().push(text.to_string()));
        host.print("hi");
        assert_eq!(*seen.borrow(), vec!["hi".to_string()]);
    }

    #[test]
    fn reexported_mirrors_share_canonical_identity() {
        use crate::bootstrap::network::types as canonical;
        let owner = IdentityOwner::create("q2-guest-unify-test").unwrap();
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
