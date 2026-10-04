//! Native Quake II server network host over the shared simulation.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/network.ts`.
//!
//! This file is self-contained like its donor and the sibling Q2 host
//! ports: shared application shapes are mirrored here with canonical-home
//! notes instead of importing sibling ports. The async content fetch and
//! download construction (`LoadedApplicationContent.forContent`,
//! `createQ2ApplicationDownloads` from donor
//! Port of Quake-Anthology-TS `src/app/bootstrap/network/q2-downloads.ts`) live behind the synchronous
//! [`Q2HostContent`] seam and the injected `downloads` handle; bootstrap
//! ports are sync.
//!
//! Sibling homes (narrow host seams over live siblings):
//! - [`SharedSimulation`](super::runtime::SharedSimulation)
//!   (`simulation/runtime.ts` port): [`Q2HostSimulation`].
//! - [`LoadedApplicationContent`](crate::bootstrap::content::LoadedApplicationContent)
//!   (`bootstrap/content.ts` port): [`Q2HostContent`].
//! - [`EngineSession`](qa_world::session::EngineSession)
//!   (`world/session/session.ts` port): [`Q2HostSession`].
//! - [`network::types`](crate::bootstrap::network::types) port
//!   (`Q2ApplicationServerHost` and friends): mirrored here as
//!   [`Q2ApplicationServerHost`] and the `Q2Application*` shapes, which
//!   keep host-local shapes.
//! - [`create_q2_application_downloads`](crate::bootstrap::network::q2_downloads::create_q2_application_downloads)
//!   (`q2-downloads.ts` port): the injected `downloads` handle.
//! - [`q2_effect_to_wire`] is re-exported from the
//!   [`q2-effects.ts`](crate::bootstrap::network::q2_effects) port.
//!
//! The donor `admit` reads the split seat from `request.splitSeat` (donor
//! Port of Quake-Anthology-TS `src/network/q2/handshake.ts`); the worktree [`Q2ConnectRequest`] has not
//! ported that field yet, so the seat travels as an explicit `admit`
//! parameter until the port lands.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use qa_bots::md4::block_checksum;
use qa_bots::BotsError;
use qa_content::contract::ItemId;
use qa_content::q2::base::player::types::{Q2PlayerEvent, Q2PlayerView, Q2PrintLevel as Q2PlayerPrintLevel};

use qa_content::q2::foundation::host::{Q2PresentationEvent, Q2PrintLevel as Q2HostPrintLevel, Q2SoundLoop};
use qa_content::q2::foundation::weapons::definitions::base_weapons;
use qa_content::q2::foundation::weapons::types::Q2WeaponEvent;
use qa_core::cmd::{expand_command_macros, CmdError, TextMode};
use qa_core::cvar::{q2_flags, CvarError, CvarRegistry};
use qa_core::identity::{ActorId, ClientId};
use qa_core::math::{Bounds, Vec3};
use qa_net::common::commands::{ActorCommand, CommandSource, UserCommand as NetUserCommand};
use qa_net::common::endpoint::{address_key, NetworkAddress};
use qa_net::protocol::ProtocolIdentity;
use qa_net::q2::{EntityState, PlayerState, ServerData, Usercmd};
use qa_net::q2_adapters::{to_q2_command, to_q2_rerelease_command};
use qa_net::q2_net::{
    Q2ConnectRequest, Q2ServerEvent, Q2ServerMessageOptions, Q2SoundMessage, Q2Status, Q2StatusPlayer, Q2Wire,
    Q2WireFrame,
};
use qa_net::q2_solid::{pack_q2_solid, q2_solid_encoding};
use qa_net::q2_svc::{encode_q2_server_event, MvdCapture, MvdEmission, MvdRecipient};
use qa_world::movement::q2::types::Q2State;
use qa_world::session::SimulationOutput;
use qa_world::WorldError;
use thiserror::Error;

use crate::bootstrap::network::q2_layout::{q2_application_layout, Q2ApplicationLayout, Q2LayoutError};

use super::types::{SimulationPresentationEvent, SourcePresentationEvent};

/// Quake II application player (canonical home:
/// [`crate::bootstrap::network::types::Q2ApplicationPlayer`]).
pub use crate::bootstrap::network::types::Q2ApplicationPlayer;

/// Admission verdict (canonical home:
/// [`crate::bootstrap::network::types::Q2ApplicationAdmission`]).
pub use crate::bootstrap::network::types::Q2ApplicationAdmission;

/// Mirror of the donor `gameState` data beyond the ported
/// `qa_net::q2::ServerData` (canonical home: `qa_net::q2`, extending
/// `ServerData` post-merge).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ServerDataParams {
    /// Ported server-data base.
    pub base: ServerData,
    /// Server state.
    pub server_state: i32,
    /// Server frames per second.
    pub server_fps: f64,
    /// R1Q2 revision, when the protocol carries one.
    pub r1q2_version: Option<u32>,
    /// R1Q2 strafe-jump landing-timer hack.
    pub r1q2_strafejump_hack: bool,
    /// Q2Pro revision, when the protocol carries one.
    pub q2pro_version: Option<u32>,
    /// Q2Pro wire flags.
    pub wire_flags: u32,
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

/// Mirror of the donor `discovery.info` payload (canonical home:
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

/// Mirror of the donor `mvdSettings` payload (canonical home:
/// `crate::bootstrap::network::types`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2MvdSettings {
    /// MVD capture enabled.
    pub enabled: bool,
    /// Maximum viewers.
    pub max_viewers: u32,
    /// Viewer password.
    pub password: String,
}

/// Mirror of the donor `supportsSourceWire` result (canonical home:
/// `crate::bootstrap::network::types`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2WireSupport {
    /// Native wire supported.
    Supported,
    /// Native wire unsupported.
    Unsupported {
        /// Reasons.
        reasons: Vec<String>,
    },
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

/// Q2 game edition (`source.game.options.edition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2Edition {
    /// Classic game API.
    Classic,
    /// Rerelease (KEX) game API.
    Rerelease,
}

/// Q2 entity solidity (`entity.solid`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2EntitySolid {
    /// Brush model solidity.
    Brush,
    /// Bounding-box solidity.
    Box,
    /// Any other solidity; wires a zero solid.
    Other,
}

/// Client origin for `session.createClient` follow-up (`client.connect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2ClientOrigin {
    /// Loopback client.
    Loopback,
    /// Remote client.
    Remote,
}

/// Translate a presentation effect into a wire entity (`q2EffectToWire`,
/// re-exported from the `q2-effects.ts` port with its donor `effects`
/// table).
pub use crate::bootstrap::network::q2_effects::q2_effect_to_wire;

/// Copy a vector into a wire triple (donor `vector`).
fn wire_vec(value: &Vec3) -> [f64; 3] {
    [f64::from(value.x), f64::from(value.y), f64::from(value.z)]
}

/// Copy a vector into a float wire triple.
fn wire_vec_f32(value: &Vec3) -> [f32; 3] {
    [value.x, value.y, value.z]
}

/// Print level value for host presentation events.
fn host_print_level(level: Q2HostPrintLevel) -> u8 {
    match level {
        Q2HostPrintLevel::Chat => 3,
        Q2HostPrintLevel::High => 2,
        Q2HostPrintLevel::Medium => 1,
        Q2HostPrintLevel::Low => 0,
    }
}

/// Print level value for player presentation events.
fn player_print_level(level: Q2PlayerPrintLevel) -> u8 {
    match level {
        Q2PlayerPrintLevel::Chat => 3,
        Q2PlayerPrintLevel::High => 2,
        Q2PlayerPrintLevel::Medium => 1,
        Q2PlayerPrintLevel::Low => 0,
    }
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

/// Recipe providers used by this host (donor `simulation.recipe` surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2HostRecipe {
    /// Movement provider.
    pub movement_provider: String,
    /// Character definition provider.
    pub character_provider: String,
    /// Map entities provider.
    pub entities_provider: String,
}

/// Q2 source game options (`source.game.options`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2SourceGameOptions {
    /// Game edition.
    pub edition: Q2Edition,
    /// Map name.
    pub map_name: String,
    /// Maximum clients.
    pub max_clients: u32,
}

/// Q2 source entity flare (`entity.flare`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2FlareView {
    /// Fade start.
    pub fade_start: i32,
    /// Fade end.
    pub fade_end: i32,
    /// Flare image.
    pub image: String,
}

/// Q2 source game entity (donor `source.game.entities` value surface).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SourceEntity {
    /// Owning actor.
    pub actor: ActorId,
    /// Classname.
    pub classname: String,
    /// Model paths.
    pub model: String,
    /// Second model path.
    pub model2: String,
    /// Third model path.
    pub model3: String,
    /// Fourth model path.
    pub model4: String,
    /// Sound path.
    pub sound: String,
    /// Frame.
    pub frame: i32,
    /// Skin number.
    pub skin: i32,
    /// Effects.
    pub effects: i32,
    /// Render flags.
    pub render_flags: i32,
    /// Whether visible.
    pub visible: bool,
    /// Server flags.
    pub server_flags: i32,
    /// Spawn flags.
    pub spawnflags: i32,
    /// Looping-sound volume.
    pub volume: f64,
    /// Sound attenuation.
    pub attenuation: f64,
    /// Entity scale.
    pub scale: f64,
    /// Solidity.
    pub solid: Q2EntitySolid,
    /// Flare override.
    pub flare: Option<Q2FlareView>,
    /// Worldspawn message.
    pub message: String,
    /// Worldspawn sky.
    pub sky: Option<String>,
    /// Worldspawn sky axis.
    pub skyaxis: Option<String>,
    /// Worldspawn sky rotation.
    pub skyrotate: Option<String>,
    /// Owning actor, when attached.
    pub owner: Option<ActorId>,
}

/// Q2 source player state (`source.players.states` value surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2SourcePlayerState {
    /// Owning actor.
    pub actor: ActorId,
    /// Client slot.
    pub slot: u32,
    /// Player name.
    pub name: String,
    /// Player skin.
    pub skin: String,
    /// Score.
    pub score: i64,
    /// Ping.
    pub ping: i64,
    /// Whether connected.
    pub connected: bool,
}

/// Q2 player connect outcome (`source.players.connect` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ConnectOutcome {
    /// Whether the connection is allowed.
    pub allowed: bool,
    /// Rejection reason.
    pub reason: String,
    /// Normalized userinfo.
    pub userinfo: String,
}

/// Q2 weapon state (`source.weapons.states` value surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2WeaponStateView {
    /// Selected weapon item.
    pub weapon: Option<ItemId>,
    /// Weapon frame.
    pub frame: i32,
    /// Gun frame rate.
    pub gun_rate: u8,
}

/// Q2 weapon definition (`source.weapons.definition` result surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2WeaponDefView {
    /// View model path.
    pub view_model: String,
    /// Player model number.
    pub player_model: i32,
}

/// Q2 item list entry (`source.items.list` value surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ItemView {
    /// Item id.
    pub id: ItemId,
    /// Item name.
    pub name: String,
}

/// Q2 source movement configuration (`simulation.q2MovementConfig`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2MovementConfig {
    /// Air acceleration.
    pub air_accelerate: f64,
    /// N64 physics.
    pub n64_physics: bool,
}

/// Source entity address (`simulation.actors.sourceOf` result surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2SourceAddress {
    /// Address provider.
    pub provider: String,
    /// Entity slot.
    pub slot: u32,
}

/// Shared presentation read (`simulation.presentations` value surface).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PresentationView {
    /// Owning actor.
    pub actor: ActorId,
    /// Whether this is a view weapon.
    pub view_weapon: bool,
    /// Presentation model path.
    pub path: Option<String>,
    /// Previous origin.
    pub previous_origin: Option<Vec3>,
    /// Presentation frame.
    pub frame: Option<i32>,
    /// Presentation skin.
    pub skin: Option<i32>,
    /// Presentation effects.
    pub effects: Option<i32>,
    /// Presentation render flags.
    pub render_flags: Option<i32>,
}

/// Shared body read (`simulation.bodies.read` result surface).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2BodyView {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Local bounds.
    pub bounds: Bounds,
}

/// Movement player read (`simulation.movementPlayer` result surface).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2MovementPlayerView {
    /// Native movement state.
    pub state: Q2State,
    /// View angles.
    pub view_angles: Vec3,
    /// View height.
    pub view_height: f64,
    /// Owning client slot, when bound.
    pub client_slot: Option<u32>,
}

/// Player UI read (`simulation.playerUi` result surface).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2PlayerUiView {
    /// Health.
    pub health: f64,
    /// Ammo count.
    pub ammo_count: f64,
    /// Regular armor points.
    pub armor_points: f64,
}

/// Scene surface used by this host.
///
/// Seam over `SharedSimulation["scene"]` from donor
/// `src/app/bootstrap/simulation/runtime.ts` (canonical home: the runtime
/// partition); the partition implements it post-merge.
pub trait Q2HostScene {
    /// Donor `pointLeaf`.
    fn point_leaf(&self, point: &Vec3) -> i32;
    /// Donor `leafCluster`.
    fn leaf_cluster(&self, leaf: i32) -> i32;
    /// Donor `leafArea`.
    fn leaf_area(&self, leaf: i32) -> i32;
    /// Donor `boxLeaves` leaves.
    fn box_leaves(&self, min: &Vec3, max: &Vec3, limit: usize) -> Vec<i32>;
    /// Donor `areasConnected`.
    fn areas_connected(&self, first: i32, second: i32) -> bool;
    /// Donor `areaBits`.
    fn area_bits(&self, area: i32) -> Vec<u8>;
    /// Donor `clusterVisible`.
    fn cluster_visible(&self, from: i32, to: i32, scope: ClusterVisibility) -> bool;
    /// Whether the scene geometry is Q2 BSP portal topology.
    fn is_q2_bsp(&self) -> bool;
    /// Maximum Q2 area portal number.
    fn q2_max_portal(&self) -> i32;
    /// Open Q2 area portals.
    fn q2_portal_state(&self) -> Vec<u32>;
}

/// Simulation surface used by this host.
///
/// Seam over `SharedSimulation` from donor
/// `src/app/bootstrap/simulation/runtime.ts` (canonical home: the runtime
/// partition); the partition implements it post-merge.
pub trait Q2HostSimulation {
    /// Donor `recipe` (used surface).
    fn q2_recipe(&self) -> Q2HostRecipe;
    /// Donor `source.game.options`; `None` when no Q2 source is bound.
    fn q2_source_options(&self) -> Option<Q2SourceGameOptions>;
    /// Donor `source.game.entities` values.
    fn q2_source_entities(&self) -> Vec<Q2SourceEntity>;
    /// Donor `source.game.entity`.
    fn q2_source_entity(&self, actor: &ActorId) -> Option<Q2SourceEntity>;
    /// Donor `simulation.q2ServerCvars`.
    fn q2_server_cvars(&mut self) -> Option<&mut CvarRegistry>;
    /// Donor `simulation.q2MovementConfig`.
    fn q2_movement_config(&self) -> Option<Q2MovementConfig>;
    /// Donor `simulation.actors.sourceOf`.
    fn actors_source(&self, actor: &ActorId) -> Option<Q2SourceAddress>;
    /// Donor `simulation.presentations`.
    fn presentations(&self) -> Vec<Q2PresentationView>;
    /// Donor `simulation.bodies.read`.
    fn body(&self, actor: &ActorId) -> Option<Q2BodyView>;
    /// Donor `simulation.bodies.linked` absolute bounds.
    fn linked_bounds(&self, actor: &ActorId) -> Option<Bounds>;
    /// Donor `simulation.movementPlayer`.
    fn movement_player(&self, actor: &ActorId) -> Option<Q2MovementPlayerView>;
    /// Donor `simulation.q2PlayerView`.
    fn q2_player_view(&self, actor: &ActorId) -> Option<Q2PlayerView>;
    /// Donor `simulation.playerUi`.
    fn player_ui(&self, actor: &ActorId) -> Q2PlayerUiView;
    /// Donor `simulation.players`.
    fn players(&self) -> Vec<ActorId>;
    /// Donor `simulation.scene`.
    fn scene(&self) -> &dyn Q2HostScene;
    /// Donor `simulation.admitPlayer`; the message propagates like the donor
    /// throw.
    fn admit_player(&mut self, client: &ClientId) -> Result<ActorId, String>;
    /// Donor `simulation.disconnectPlayer`.
    fn disconnect_player(&mut self, actor: &ActorId);
    /// Donor `simulation.notifyClientEvent`.
    fn notify_client_event(&mut self, kind: &str, actor: &ActorId);
    /// Donor `source.players.connect`.
    fn q2_players_connect(&mut self, userinfo: &str) -> Q2ConnectOutcome;
    /// Donor `source.players.userinfoChanged`.
    fn q2_players_userinfo_changed(&mut self, actor: &ActorId, userinfo: &str);
    /// Donor `source.players.clientCommand`.
    fn q2_players_client_command(&mut self, actor: &ActorId, name: &str, args: &[String]);
    /// Donor `source.players.states` values.
    fn q2_player_states(&self) -> Vec<Q2SourcePlayerState>;
    /// Donor `source.weapons.states.get`.
    fn q2_weapon_state(&self, actor: &ActorId) -> Option<Q2WeaponStateView>;
    /// Donor `source.weapons.definition`.
    fn q2_weapon_definition(&self, weapon: &ItemId) -> Q2WeaponDefView;
    /// Donor `source.items.list`.
    fn q2_items(&self) -> Vec<Q2ItemView>;
}

/// Content surface used by this host.
///
/// Seam over `LoadedApplicationContent` from donor
/// `src/app/bootstrap/content.ts` (canonical home: the content partition);
/// the partition implements it post-merge.
pub trait Q2HostContent {
    /// Donor `mounts.read(recipe.map.geometry)` bytes.
    fn map_geometry_bytes(&self) -> Result<Vec<u8>, String>;
    /// Donor `recipe.map.geometry.requestedPath`.
    fn map_geometry_path(&self) -> String;
    /// Last segment of the entities content directory (donor `gamedir`).
    fn content_directory(&self) -> String;
    /// Q2 world leaf count, when `world.kind` is `q2-bsp`.
    fn world_q2_leaf_count(&self) -> Option<usize>;
}

/// Session surface used by this host.
///
/// Seam over `EngineSession` from donor `src/world/session/session.ts`
/// (canonical home: `qa_world::session`); the partition implements it
/// post-merge.
pub trait Q2HostSession {
    /// Donor `session.createClient`.
    fn create_client(&mut self, slot: u32) -> Result<ClientId, WorldError>;
    /// Donor `client.connect`.
    fn connect_client(&mut self, client: &ClientId, origin: Q2ClientOrigin);
    /// Donor `session.closeClient`.
    fn close_client(&mut self, client: &ClientId);
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

/// Rerelease seat identity (donor `playerIdentity` seat payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2SeatIdentity {
    /// Split-screen seat.
    pub seat: u32,
    /// Social id.
    pub social_id: String,
}

/// Social identity owner, mirroring donor `playerIdentity`.
pub type Q2PlayerIdentityFn = Rc<dyn Fn(&ClientId, Option<Q2SeatIdentity>)>;

/// Mirror of `Q2ApplicationServerBindingOptions` from donor
/// `src/app/bootstrap/simulation/network.ts` (canonical home: this file).
pub struct Q2ApplicationServerOptions<S, C, E, D, A> {
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
    /// Application downloads handle.
    pub downloads: D,
    /// Userinfo parser.
    pub parse_userinfo: ParseQ2UserinfoFn,
}

/// Native Q2 server host error.
#[derive(Debug, Error)]
pub enum Q2HostError {
    /// No Q2 source game provider is bound.
    #[error("Q2 server network host requires the Q2 source game provider")]
    MissingSource,
    /// No source cvar registry is bound.
    #[error("Q2 server network host requires the source cvar registry")]
    MissingCvars,
    /// Source movement configuration disappeared.
    #[error("Q2 host lost source movement configuration")]
    MissingMovementConfig,
    /// Native resource table is full.
    #[error("Native Q2 resource table is full")]
    ResourceTableFull,
    /// Actor has no Q2 source entity address.
    #[error("Actor has no Q2 source entity address")]
    ActorWithoutAddress,
    /// Network player has no movement state.
    #[error("Network player has no movement state")]
    MissingMovement,
    /// Native playerstate requires a Q2 movement provider.
    #[error("Native Q2 playerstate requires a Q2 movement provider")]
    MovementProvider,
    /// MVD capture requires Quake II portal topology.
    #[error("MVD source capture requires Quake II portal topology")]
    MvdTopology,
    /// Client has no configstring history.
    #[error("Q2 client has no configstring history")]
    MissingConfigHistory,
    /// Network player body disappeared.
    #[error("Network player body disappeared")]
    MissingBody,
    /// Admitted player has no source entity.
    #[error("Admitted Q2 player has no source entity")]
    MissingEntity,
    /// Carried client was never admitted.
    #[error("Application has not admitted carried Q2 network client")]
    CarriedClient,
    /// Command has no source player.
    #[error("Q2 command has no source player")]
    CommandEntity,
    /// Userinfo has no source entity.
    #[error("Q2 userinfo has no source entity")]
    UserinfoEntity,
    /// Wire value out of range.
    #[error("Q2 wire value out of range: {0}")]
    WireRange(&'static str),
    /// Game callback failure; the message propagates like the donor throw.
    #[error("{0}")]
    GameCallback(String),
    /// Content read failure.
    #[error("Q2 host content is unavailable: {0}")]
    Content(String),
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
    /// Layout failure.
    #[error(transparent)]
    Layout(#[from] Q2LayoutError),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] qa_net::q2_net::Q2NetError),
    /// Checksum failure.
    #[error(transparent)]
    Checksum(#[from] BotsError),
    /// Command macro failure.
    #[error(transparent)]
    Command(#[from] CmdError),
    /// Session failure.
    #[error(transparent)]
    Session(#[from] WorldError),
}

/// Native Q2 application server host.
///
/// Concrete mirror of `Q2ApplicationServerHost` from donor
/// `src/app/bootstrap/network/types.ts` (canonical home:
/// `crate::bootstrap::network::types`); unify post-merge.
pub struct Q2ApplicationServerHost<S, C, E, D, A> {
    /// Engine session.
    session: E,
    /// Address rejection predicate.
    rejects: Option<Q2RejectsFn>,
    /// Shared simulation.
    simulation: S,
    /// Loaded content.
    content: C,
    /// Protocol identity.
    protocol: ProtocolIdentity,
    /// Rcon administration host.
    administration: Option<A>,
    /// Master server listing.
    masters: Option<Q2MastersFn>,
    /// Social identity owner.
    player_identity: Option<Q2PlayerIdentityFn>,
    /// Print sink.
    print: Q2PrintFn,
    /// Application downloads handle.
    downloads: D,
    /// Userinfo parser.
    parse_userinfo: ParseQ2UserinfoFn,
    /// Configstring layout.
    layout: Q2ApplicationLayout,
    /// Game edition.
    edition: Q2Edition,
    /// Map name.
    map_name: String,
    /// Entities content directory.
    content_directory: String,
    /// Maximum clients.
    max_clients: u32,
    /// Model path table.
    models: HashMap<String, u32>,
    /// Sound path table.
    sounds: HashMap<String, u32>,
    /// Image path table.
    images: HashMap<String, u32>,
    /// Configstrings by index.
    configs: HashMap<u32, String>,
    /// Admitted players by client slot.
    players: HashMap<u32, Q2ApplicationPlayer>,
    /// Entity events by actor for the current frame.
    entity_events: HashMap<ActorId, i32>,
    /// Frame number of the retained entity events.
    event_frame: i32,
    /// Configstrings last sent to each client slot.
    known_configs: HashMap<u32, HashMap<u32, String>>,
}

impl<S, C, E, D, A> std::fmt::Debug for Q2ApplicationServerHost<S, C, E, D, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q2ApplicationServerHost")
            .field("protocol", &self.protocol)
            .field("edition", &self.edition)
            .field("map_name", &self.map_name)
            .field("max_clients", &self.max_clients)
            .field("players", &self.players)
            .finish_non_exhaustive()
    }
}

/// Create the native Q2 application server host (donor
/// `createQ2ApplicationServerHost`).
///
/// This adapter reads shared actor/body/combat state and source player
/// fields on every frame.
pub fn create_q2_application_server_host<S, C, E, D, A>(
    options: Q2ApplicationServerOptions<S, C, E, D, A>,
) -> Result<Q2ApplicationServerHost<S, C, E, D, A>, Q2HostError>
where
    S: Q2HostSimulation,
    C: Q2HostContent,
    E: Q2HostSession,
{
    let mut simulation = options.simulation;
    let source = simulation.q2_source_options().ok_or(Q2HostError::MissingSource)?;
    {
        let cvars = simulation.q2_server_cvars().ok_or(Q2HostError::MissingCvars)?;
        cvars.register("sv_mvd_enable", "0", q2_flags::LATCH)?;
        cvars.register("sv_mvd_maxclients", "8", q2_flags::LATCH)?;
        cvars.register("sv_mvd_password", "", q2_flags::PRIVATE)?;
        cvars.register("hostname", "noname", q2_flags::SERVER_INFO | q2_flags::ARCHIVE)?;
        for (name, value) in [
            ("protocol", options.protocol.version().to_string()),
            ("mapname", source.map_name.clone()),
            ("maxclients", source.max_clients.to_string()),
        ] {
            let flags = q2_flags::SERVER_INFO
                | if name == "maxclients" {
                    q2_flags::LATCH
                } else {
                    q2_flags::NO_SET
                };
            cvars.register(name, &value, flags)?;
            cvars.set(name, &value, true)?;
        }
    }
    let layout = q2_application_layout(options.protocol)?;
    let mut host = Q2ApplicationServerHost {
        session: options.session,
        rejects: options.rejects,
        simulation,
        content: options.content,
        protocol: options.protocol,
        administration: options.administration,
        masters: options.masters,
        player_identity: options.player_identity,
        print: options.print,
        downloads: options.downloads,
        parse_userinfo: options.parse_userinfo,
        layout,
        edition: source.edition,
        map_name: source.map_name.clone(),
        content_directory: String::new(),
        max_clients: source.max_clients,
        models: HashMap::new(),
        sounds: HashMap::new(),
        images: HashMap::new(),
        configs: HashMap::new(),
        players: HashMap::new(),
        entity_events: HashMap::new(),
        event_frame: -1,
        known_configs: HashMap::new(),
    };
    host.content_directory = host.content.content_directory();
    let geometry = host.content.map_geometry_bytes().map_err(Q2HostError::Content)?;
    let checksum = block_checksum(&geometry)?;
    let entities = host.simulation.q2_source_entities();
    let world = entities.iter().find(|entity| entity.classname == "worldspawn");
    let level_name = match world {
        Some(world) if !world.message.is_empty() => world.message.clone(),
        _ => source.map_name.clone(),
    };
    host.configs.insert(0, level_name);
    host.configs.insert(
        2,
        world
            .and_then(|world| world.sky.clone())
            .unwrap_or_else(|| "unit1_".to_string()),
    );
    host.configs.insert(
        3,
        world
            .and_then(|world| world.skyaxis.clone())
            .unwrap_or_else(|| "0 0 0".to_string()),
    );
    host.configs.insert(
        4,
        world
            .and_then(|world| world.skyrotate.clone())
            .unwrap_or_else(|| "0".to_string()),
    );
    host.configs.insert(layout.map_checksum, checksum.to_string());
    host.configs.insert(layout.max_clients, source.max_clients.to_string());
    host.update_movement_configs()?;
    let geometry_path = host.content.map_geometry_path();
    host.model(&geometry_path)?;
    for entity in &entities {
        for path in [&entity.model, &entity.model2, &entity.model3, &entity.model4] {
            host.model(path)?;
        }
        if !entity.sound.is_empty() {
            host.sound(&entity.sound)?;
        }
    }
    for weapon in base_weapons() {
        host.model(&weapon.definition.view_model)?;
        host.model(&weapon.definition.world_model)?;
    }
    host.image("i_health")?;
    for (ordinal, item) in host.simulation.q2_items().iter().enumerate() {
        host.configs
            .insert(layout.items + ordinal as u32 + 1, item.name.clone());
    }
    Ok(host)
}

impl<S, C, E, D, A> Q2ApplicationServerHost<S, C, E, D, A>
where
    S: Q2HostSimulation,
    C: Q2HostContent,
    E: Q2HostSession,
{
    /// Resolve an actor to its Q2 source entity number (donor `sourceNumber`).
    fn source_number(&self, actor: &ActorId) -> Result<u32, Q2HostError> {
        match self.simulation.actors_source(actor) {
            Some(address) if address.provider == self.simulation.q2_recipe().entities_provider => Ok(address.slot),
            _ => Err(Q2HostError::ActorWithoutAddress),
        }
    }

    /// Intern a resource path (donor `index`).
    fn index_resource(
        &mut self,
        path: &str,
        table: &mut HashMap<String, u32>,
        base: u32,
        maximum: u32,
    ) -> Result<u32, Q2HostError> {
        if path.is_empty() {
            return Ok(0);
        }
        if let Some(existing) = table.get(path) {
            return Ok(*existing);
        }
        let value = table.len() as u32 + 1;
        if value >= maximum {
            return Err(Q2HostError::ResourceTableFull);
        }
        table.insert(path.to_string(), value);
        self.configs.insert(base + value, path.to_string());
        Ok(value)
    }

    /// Intern a model path (donor `model`).
    fn model(&mut self, path: &str) -> Result<u32, Q2HostError> {
        let layout = self.layout;
        let mut table = std::mem::take(&mut self.models);
        let value = self.index_resource(path, &mut table, layout.models, layout.max_models);
        self.models = table;
        value
    }

    /// Intern a sound path (donor `sound`).
    fn sound(&mut self, path: &str) -> Result<u32, Q2HostError> {
        let layout = self.layout;
        let mut table = std::mem::take(&mut self.sounds);
        let value = self.index_resource(path, &mut table, layout.sounds, layout.max_sounds);
        self.sounds = table;
        value
    }

    /// Intern an image path (donor `image`).
    fn image(&mut self, path: &str) -> Result<u32, Q2HostError> {
        let layout = self.layout;
        let mut table = std::mem::take(&mut self.images);
        let value = self.index_resource(path, &mut table, layout.images, layout.max_images);
        self.images = table;
        value
    }

    /// Refresh the movement configstrings (donor `updateMovementConfigs`).
    fn update_movement_configs(&mut self) -> Result<(), Q2HostError> {
        let movement = self
            .simulation
            .q2_movement_config()
            .ok_or(Q2HostError::MissingMovementConfig)?;
        self.configs
            .insert(self.layout.air_accelerate, movement.air_accelerate.to_string());
        if let Some(n64) = self.layout.n64_physics {
            self.configs.insert(
                n64,
                if movement.n64_physics {
                    "1".to_string()
                } else {
                    "0".to_string()
                },
            );
        }
        Ok(())
    }

    /// Inventory ordinal for an item id (donor `inventoryOrdinal`).
    fn inventory_ordinal(&self, item: &ItemId) -> i32 {
        self.simulation
            .q2_items()
            .iter()
            .position(|definition| &definition.id == item)
            .map_or(0, |ordinal| ordinal as i32 + 1)
    }
}

impl<S, C, E, D, A> Q2ApplicationServerHost<S, C, E, D, A>
where
    S: Q2HostSimulation,
    C: Q2HostContent,
    E: Q2HostSession,
{
    /// Build sorted wire entity states (donor `entityStates`).
    #[allow(clippy::field_reassign_with_default)]
    fn entity_states(&mut self, protocol: ProtocolIdentity) -> Result<Vec<EntityState>, Q2HostError> {
        let presentations: HashMap<ActorId, Q2PresentationView> = self
            .simulation
            .presentations()
            .into_iter()
            .filter(|presentation| !presentation.view_weapon)
            .map(|presentation| (presentation.actor.clone(), presentation))
            .collect();
        let mut result = Vec::new();
        for entity in self.simulation.q2_source_entities() {
            let number = self.source_number(&entity.actor)?;
            let body = match self.simulation.body(&entity.actor) {
                Some(body) => body,
                None => continue,
            };
            if number == 0 || !entity.visible || (entity.server_flags & 1) != 0 {
                continue;
            }
            let presentation = presentations.get(&entity.actor);
            let mut wire = EntityState::default();
            wire.number = u16::try_from(number).map_err(|_| Q2HostError::WireRange("entity number"))?;
            wire.origin = wire_vec(&body.origin);
            wire.old_origin = wire_vec(
                &presentation
                    .and_then(|presentation| presentation.previous_origin)
                    .unwrap_or(body.origin),
            );
            wire.angles = wire_vec(&body.angles);
            let model_path = presentation
                .and_then(|presentation| presentation.path.clone())
                .unwrap_or_else(|| entity.model.clone());
            wire.modelindex =
                u16::try_from(self.model(&model_path)?).map_err(|_| Q2HostError::WireRange("model index"))?;
            wire.modelindex2 =
                u16::try_from(self.model(&entity.model2)?).map_err(|_| Q2HostError::WireRange("model index"))?;
            wire.modelindex3 =
                u16::try_from(self.model(&entity.model3)?).map_err(|_| Q2HostError::WireRange("model index"))?;
            wire.modelindex4 =
                u16::try_from(self.model(&entity.model4)?).map_err(|_| Q2HostError::WireRange("model index"))?;
            wire.frame = presentation
                .and_then(|presentation| presentation.frame)
                .unwrap_or(entity.frame);
            if let Some(flare) = &entity.flare {
                wire.modelindex = 1;
                wire.modelindex2 = u16::try_from(flare.fade_start).map_err(|_| Q2HostError::WireRange("flare fade"))?;
                wire.modelindex3 = u16::try_from(flare.fade_end).map_err(|_| Q2HostError::WireRange("flare fade"))?;
                wire.frame = if (entity.render_flags & 256) != 0 {
                    self.image(&flare.image)? as i32
                } else {
                    0
                };
            }
            wire.skinnum = presentation
                .and_then(|presentation| presentation.skin)
                .unwrap_or(entity.skin);
            wire.effects = presentation
                .and_then(|presentation| presentation.effects)
                .unwrap_or(entity.effects);
            wire.renderfx = presentation
                .and_then(|presentation| presentation.render_flags)
                .unwrap_or(entity.render_flags);
            wire.event = u8::try_from(self.entity_events.get(&entity.actor).copied().unwrap_or(0))
                .map_err(|_| Q2HostError::WireRange("entity event"))?;
            if let Some(native) = self
                .simulation
                .q2_player_states()
                .into_iter()
                .find(|player| player.actor == entity.actor)
            {
                wire.modelindex = 255;
                wire.skinnum = i32::try_from(native.slot).map_err(|_| Q2HostError::WireRange("player slot"))?;
                if let Some(weapon) = self
                    .simulation
                    .q2_weapon_state(&entity.actor)
                    .and_then(|state| state.weapon)
                {
                    wire.modelindex2 = 255;
                    wire.skinnum |= self.simulation.q2_weapon_definition(&weapon).player_model << 8;
                }
            }
            wire.sound =
                u16::try_from(self.sound(&entity.sound)?).map_err(|_| Q2HostError::WireRange("sound index"))?;
            wire.loop_volume = if entity.classname == "target_speaker" && (entity.spawnflags & 3) != 0 {
                1.0
            } else {
                entity.volume
            };
            wire.loop_attenuation = entity.attenuation;
            wire.scale = if entity.scale == 1.0 { 0.0 } else { entity.scale };
            match entity.solid {
                Q2EntitySolid::Brush => wire.solid = 31,
                Q2EntitySolid::Box => {
                    let encoding =
                        q2_solid_encoding(protocol, false).ok_or(Q2HostError::WireRange("solid encoding"))?;
                    wire.solid = pack_q2_solid(&body.bounds, encoding);
                }
                Q2EntitySolid::Other => {}
            }
            if wire.modelindex != 0 || wire.sound != 0 || wire.effects != 0 || wire.event != 0 {
                result.push(wire);
            }
        }
        result.sort_by_key(|state| state.number);
        Ok(result)
    }

    /// Filter entity states to those visible to a player (donor
    /// `visibleEntities`).
    fn visible_entities(
        &self,
        player: &Q2ApplicationPlayer,
        mut states: Vec<EntityState>,
        origin: &Vec3,
    ) -> Result<Vec<EntityState>, Q2HostError> {
        let scene = self.simulation.scene();
        let leaf = scene.point_leaf(origin);
        let area = scene.leaf_area(leaf);
        let cluster = scene.leaf_cluster(leaf);
        let fat_min = Vec3 {
            x: origin.x - 8.0,
            y: origin.y - 8.0,
            z: origin.z - 8.0,
        };
        let fat_max = Vec3 {
            x: origin.x + 8.0,
            y: origin.y + 8.0,
            z: origin.z + 8.0,
        };
        let clusters: HashSet<i32> = scene
            .box_leaves(&fat_min, &fat_max, 64)
            .into_iter()
            .map(|index| scene.leaf_cluster(index))
            .collect();
        let mut by_number = HashMap::new();
        for entity in self.simulation.q2_source_entities() {
            by_number.insert(self.source_number(&entity.actor)?, entity);
        }
        let is_rerelease = self.edition == Q2Edition::Rerelease;
        let leaf_limit = self.content.world_q2_leaf_count().unwrap_or(65536);
        let mut visible = Vec::new();
        for mut state in states.drain(..) {
            let Some(entity) = by_number.get(&u32::from(state.number)) else {
                continue;
            };
            if entity.owner.as_ref() == Some(&player.actor) {
                state.solid = 0;
            }
            if u32::from(state.number) == player.source_entity {
                visible.push(state);
                continue;
            }
            // Rerelease speaker ATTN_LOOP_NONE carries source SVF_NOCULL semantics.
            if is_rerelease
                && entity.classname == "target_speaker"
                && (entity.spawnflags & 3) != 0
                && entity.attenuation == -1.0
            {
                visible.push(state);
                continue;
            }
            let Some(body) = self.simulation.body(&entity.actor) else {
                continue;
            };
            let bounds = self.simulation.linked_bounds(&entity.actor).unwrap_or(Bounds {
                min: Vec3 {
                    x: body.origin.x + body.bounds.min.x - 1.0,
                    y: body.origin.y + body.bounds.min.y - 1.0,
                    z: body.origin.z + body.bounds.min.z - 1.0,
                },
                max: Vec3 {
                    x: body.origin.x + body.bounds.max.x + 1.0,
                    y: body.origin.y + body.bounds.max.y + 1.0,
                    z: body.origin.z + body.bounds.max.z + 1.0,
                },
            });
            let leaves = scene.box_leaves(&bounds.min, &bounds.max, leaf_limit);
            if !leaves
                .iter()
                .any(|index| scene.areas_connected(area, scene.leaf_area(*index)))
            {
                continue;
            }
            if (state.renderfx & 128) != 0 {
                let first = leaves.first().copied();
                let Some(first) = first else {
                    continue;
                };
                if scene.cluster_visible(cluster, scene.leaf_cluster(first), ClusterVisibility::Phs) {
                    visible.push(state);
                }
                continue;
            }
            if !leaves.iter().any(|index| {
                let target = scene.leaf_cluster(*index);
                clusters
                    .iter()
                    .any(|from| scene.cluster_visible(*from, target, ClusterVisibility::Pvs))
            }) {
                continue;
            }
            let distance = ((f64::from(origin.x) - f64::from(body.origin.x)).powi(2)
                + (f64::from(origin.y) - f64::from(body.origin.y)).powi(2)
                + (f64::from(origin.z) - f64::from(body.origin.z)).powi(2))
            .sqrt();
            if state.modelindex != 0 || distance <= 400.0 {
                visible.push(state);
            }
        }
        Ok(visible)
    }

    /// Build a wire player state (donor `playerState`).
    fn player_state(&mut self, actor: &ActorId) -> Result<PlayerState, Q2HostError> {
        let movement = self
            .simulation
            .movement_player(actor)
            .ok_or(Q2HostError::MissingMovement)?;
        let view = self.simulation.q2_player_view(actor);
        let mut state = PlayerState::default();
        match movement.state {
            Q2State::Classic(native) => {
                state.pmove.pm_type =
                    u8::try_from(native.move_type).map_err(|_| Q2HostError::WireRange("pmove type"))?;
                state.pmove.origin = native.origin_eighths;
                state.pmove.velocity = native.velocity_eighths;
                state.pmove.origin_f = native.origin_eighths.map(|value| value as f32 / 8.0);
                state.pmove.velocity_f = native.velocity_eighths.map(|value| value as f32 / 8.0);
                state.pmove.delta_angles = [
                    i16::try_from(native.delta_angle_shorts[0]).map_err(|_| Q2HostError::WireRange("delta angles"))?,
                    i16::try_from(native.delta_angle_shorts[1]).map_err(|_| Q2HostError::WireRange("delta angles"))?,
                    i16::try_from(native.delta_angle_shorts[2]).map_err(|_| Q2HostError::WireRange("delta angles"))?,
                ];
                state.pmove.pm_flags = native.flags;
                state.pmove.pm_time = native.time_eight_milliseconds;
                state.pmove.gravity =
                    i16::try_from(native.gravity as i32).map_err(|_| Q2HostError::WireRange("gravity"))?;
            }
            Q2State::Rerelease(native) => {
                state.pmove.pm_type =
                    u8::try_from(native.move_type).map_err(|_| Q2HostError::WireRange("pmove type"))?;
                state.pmove.origin_f = wire_vec_f32(&native.origin);
                state.pmove.velocity_f = wire_vec_f32(&native.velocity);
                state.pmove.delta_angles_f = wire_vec_f32(&native.delta_angles);
                state.pmove.delta_angle_float = true;
                state.pmove.delta_angles = [
                    (f64::from(native.delta_angles.x) * 65536.0 / 360.0).trunc() as i32 as u16 as i16,
                    (f64::from(native.delta_angles.y) * 65536.0 / 360.0).trunc() as i32 as u16 as i16,
                    (f64::from(native.delta_angles.z) * 65536.0 / 360.0).trunc() as i32 as u16 as i16,
                ];
                state.pmove.pm_flags = native.flags;
                state.pmove.pm_time = native.time_milliseconds;
                state.pmove.gravity =
                    i16::try_from(native.gravity as i32).map_err(|_| Q2HostError::WireRange("gravity"))?;
                state.pmove.viewheight = native.view_height as i32;
            }
        }
        state.viewangles = wire_vec(&view.as_ref().map(|view| view.angles).unwrap_or(movement.view_angles));
        state.viewoffset = wire_vec(&view.as_ref().map(|view| view.offset).unwrap_or(Vec3 {
            x: 0.0,
            y: 0.0,
            z: movement.view_height as f32,
        }));
        state.kick_angles =
            wire_vec(
                &view
                    .as_ref()
                    .map(|view| view.kick_angles)
                    .unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 }),
            );
        state.gunangles =
            wire_vec(
                &view
                    .as_ref()
                    .map(|view| view.gun_angles)
                    .unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 }),
            );
        state.gunoffset =
            wire_vec(
                &view
                    .as_ref()
                    .map(|view| view.gun_offset)
                    .unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 }),
            );
        if let Some(view) = &view {
            state.blend = [
                f64::from(view.blend.x),
                f64::from(view.blend.y),
                f64::from(view.blend.z),
                f64::from(view.blend.w),
            ];
        }
        state.fov = view.as_ref().map(|view| view.fov).unwrap_or(90) as u8;
        state.rdflags = if view.as_ref().is_some_and(|view| view.underwater) {
            1
        } else {
            0
        };
        if let Some(weapon) = self.simulation.q2_weapon_state(actor) {
            state.gunindex = match &weapon.weapon {
                None => 0,
                Some(item) => {
                    let definition = self.simulation.q2_weapon_definition(item);
                    self.model(&definition.view_model)? as i32
                }
            };
            state.gunframe = weapon.frame;
            state.gunrate = weapon.gun_rate;
        }
        let ui = self.simulation.player_ui(actor);
        state.stats[0] = self.image("i_health")? as i16;
        state.stats[1] = view.as_ref().map(|view| view.health).unwrap_or(ui.health) as i16;
        state.stats[3] = view.as_ref().map(|view| view.ammo).unwrap_or(ui.ammo_count) as i16;
        state.stats[5] = view.as_ref().map(|view| view.armor).unwrap_or(ui.armor_points) as i16;
        state.stats[9] = view
            .as_ref()
            .and_then(|view| view.timer.as_ref())
            .map(|timer| timer.seconds)
            .unwrap_or(0) as i16;
        state.stats[13] = view.as_ref().map(|view| view.layouts).unwrap_or(0) as i16;
        state.stats[14] = view.as_ref().map(|view| view.score).unwrap_or(0) as i16;
        state.stats[16] = match view.as_ref().and_then(|view| view.selected_item.clone()) {
            None => 0,
            Some(item) => self.inventory_ordinal(&item) as i16,
        };
        state.stats[17] = if view.as_ref().is_some_and(|view| view.spectator) {
            1
        } else {
            0
        };
        Ok(state)
    }
}

impl<S, C, E, D, A> Q2ApplicationServerHost<S, C, E, D, A>
where
    S: Q2HostSimulation,
    C: Q2HostContent,
    E: Q2HostSession,
{
    /// Donor `rejects` passthrough: false when the predicate is absent.
    #[must_use]
    pub fn rejected(&self, address: &NetworkAddress) -> bool {
        self.rejects.as_ref().is_some_and(|rejects| rejects(address))
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
    pub fn masters(&self) -> Option<&Q2MastersFn> {
        self.masters.as_ref()
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
            max_config_strings: self.layout.max_config_strings as u16,
            inventory_slots: 256,
            ..Q2ServerMessageOptions::default()
        }
    }

    /// Donor `maxClients`.
    #[must_use]
    pub fn max_clients(&self) -> u32 {
        self.max_clients
    }

    /// Donor `print`.
    pub fn print(&self, text: &str) {
        (self.print)(text);
    }

    /// Capture an authoritative snapshot for MVD (donor `captureMvd`).
    pub fn mvd_capture(
        &mut self,
        _output: &SimulationOutput,
        events: &[SimulationPresentationEvent],
        servercount: i32,
    ) -> Result<MvdCapture, Q2HostError> {
        self.update_movement_configs()?;
        let rerelease = self.edition == Q2Edition::Rerelease;
        let protocol = if rerelease {
            ProtocolIdentity::Q2Rerelease
        } else {
            ProtocolIdentity::Q2Classic
        };
        let mut wire = Q2Wire::new(protocol)?;
        let mut players = BTreeMap::new();
        for player in self.simulation.q2_player_states() {
            if !player.connected || self.simulation.movement_player(&player.actor).is_none() {
                continue;
            }
            let number = self.source_number(&player.actor)? - 1;
            let slot = u8::try_from(number).map_err(|_| Q2HostError::WireRange("MVD player slot"))?;
            players.insert(slot, self.player_state(&player.actor)?);
        }
        let entities = self.entity_states(protocol)?;
        if !self.simulation.scene().is_q2_bsp() {
            return Err(Q2HostError::MvdTopology);
        }
        let max_portal = self.simulation.scene().q2_max_portal();
        let mut portal_bits = vec![0u8; ((max_portal + 1).max(0) as usize).div_ceil(8)];
        for portal in self.simulation.scene().q2_portal_state() {
            let index = portal as usize / 8;
            if let Some(byte) = portal_bits.get_mut(index) {
                *byte |= 1 << (portal & 7);
            }
        }
        let mut messages = Vec::new();
        for item in events {
            match &item.event {
                SourcePresentationEvent::Q2Player(event) => match event {
                    Q2PlayerEvent::Print { target, level, text } => {
                        self.mvd_emit(
                            &mut wire,
                            &mut messages,
                            Q2ServerEvent::Print {
                                level: player_print_level(*level),
                                text: text.clone(),
                            },
                            self.mvd_target(target.as_ref())?,
                            true,
                        )?;
                    }
                    Q2PlayerEvent::StuffText { actor, text } => {
                        self.mvd_emit(
                            &mut wire,
                            &mut messages,
                            Q2ServerEvent::CommandText { text: text.clone() },
                            self.mvd_target(Some(actor))?,
                            true,
                        )?;
                    }
                    Q2PlayerEvent::Inventory { actor, entries, .. } => {
                        let mut counts = vec![0i16; 256];
                        for entry in entries {
                            let ordinal = self.inventory_ordinal(&entry.item);
                            if ordinal > 0 && ordinal < 256 {
                                counts[ordinal as usize] = entry.count as i16;
                            }
                        }
                        self.mvd_emit(
                            &mut wire,
                            &mut messages,
                            Q2ServerEvent::Inventory { counts },
                            self.mvd_target(Some(actor))?,
                            true,
                        )?;
                    }
                    _ => {}
                },
                SourcePresentationEvent::Q2(event) => match event {
                    Q2PresentationEvent::Print { actor, level, text } => {
                        self.mvd_emit(
                            &mut wire,
                            &mut messages,
                            Q2ServerEvent::Print {
                                level: host_print_level(*level),
                                text: text.clone(),
                            },
                            self.mvd_target(actor.as_ref())?,
                            true,
                        )?;
                    }
                    Q2PresentationEvent::CenterPrint { actor, text, .. } => {
                        self.mvd_emit(
                            &mut wire,
                            &mut messages,
                            Q2ServerEvent::CenterPrint { text: text.clone() },
                            self.mvd_target(Some(actor))?,
                            true,
                        )?;
                    }
                    Q2PresentationEvent::Effect(event) => {
                        // Presentation effects currently retain no native multicast scope;
                        // preserve existing server routing.
                        if let Some(value) = q2_effect_to_wire(event) {
                            self.mvd_emit(
                                &mut wire,
                                &mut messages,
                                Q2ServerEvent::TempEntity { value },
                                MvdRecipient::All,
                                false,
                            )?;
                        }
                    }
                    Q2PresentationEvent::Sound(event) => {
                        if event.loop_ == Q2SoundLoop::Once {
                            let recipient = if event.attenuation == 0.0 || (event.channel & 8) != 0 {
                                MvdRecipient::All
                            } else {
                                let leaf = self.simulation.scene().point_leaf(&event.origin);
                                MvdRecipient::Phs(
                                    u16::try_from(leaf).map_err(|_| Q2HostError::WireRange("MVD sound leaf"))?,
                                )
                            };
                            let entity = match &event.actor {
                                None => 0,
                                Some(actor) => match item.source_entity {
                                    Some(number) => number as u32,
                                    None => self.source_number(actor)?,
                                },
                            };
                            let index = u16::try_from(self.sound(&event.path)?)
                                .map_err(|_| Q2HostError::WireRange("sound index"))?;
                            self.mvd_emit(
                                &mut wire,
                                &mut messages,
                                Q2ServerEvent::Sound {
                                    sound: Q2SoundMessage {
                                        flags: 0,
                                        index,
                                        entity,
                                        channel: (event.channel & 7) as u8,
                                        position: Some(wire_vec(&event.origin)),
                                        volume: event.volume,
                                        attenuation: event.attenuation,
                                        delay_seconds: 0.0,
                                    },
                                },
                                recipient,
                                event.reliable || (event.channel & 16) != 0,
                            )?;
                        }
                    }
                    Q2PresentationEvent::MonsterMuzzleflash { actor, flash, .. } => {
                        let entity = match item.source_entity {
                            Some(number) => number,
                            None => self.source_number(actor)? as i32,
                        };
                        if let Some(body) = self.simulation.body(actor) {
                            let leaf = self.simulation.scene().point_leaf(&body.origin);
                            self.mvd_emit(
                                &mut wire,
                                &mut messages,
                                Q2ServerEvent::MuzzleFlash {
                                    entity,
                                    flash: *flash,
                                    monster: true,
                                    silenced: false,
                                },
                                MvdRecipient::Pvs(
                                    u16::try_from(leaf).map_err(|_| Q2HostError::WireRange("MVD flash leaf"))?,
                                ),
                                false,
                            )?;
                        }
                    }
                    _ => {}
                },
                SourcePresentationEvent::Q2Weapon(Q2WeaponEvent::Muzzleflash {
                    actor, flash, silenced, ..
                }) => {
                    if let Some(body) = self.simulation.body(actor) {
                        let entity = match item.source_entity {
                            Some(number) => number,
                            None => self.source_number(actor)? as i32,
                        };
                        let leaf = self.simulation.scene().point_leaf(&body.origin);
                        self.mvd_emit(
                            &mut wire,
                            &mut messages,
                            Q2ServerEvent::MuzzleFlash {
                                entity,
                                flash: *flash,
                                monster: false,
                                silenced: *silenced,
                            },
                            MvdRecipient::Pvs(
                                u16::try_from(leaf).map_err(|_| Q2HostError::WireRange("MVD flash leaf"))?,
                            ),
                            false,
                        )?;
                    }
                }
                _ => {}
            }
        }
        let mut config_strings = BTreeMap::new();
        for (index, value) in &self.configs {
            config_strings.insert(
                u16::try_from(*index).map_err(|_| Q2HostError::WireRange("configstring index"))?,
                value.clone(),
            );
        }
        Ok(MvdCapture {
            revision: if rerelease { 3038 } else { 2010 },
            flags: 0,
            servercount,
            gamedir: self.content_directory.clone(),
            dummy: -1,
            config_strings,
            portal_bits,
            players,
            entities,
            messages,
        })
    }

    /// MVD recipient for an optional target actor (donor `target`).
    fn mvd_target(&self, actor: Option<&ActorId>) -> Result<MvdRecipient, Q2HostError> {
        match actor {
            None => Ok(MvdRecipient::All),
            Some(actor) => {
                let number = self.source_number(actor)? - 1;
                Ok(MvdRecipient::Player(
                    u8::try_from(number).map_err(|_| Q2HostError::WireRange("MVD player slot"))?,
                ))
            }
        }
    }

    /// Encode and route one MVD message (donor `emit`).
    fn mvd_emit(
        &self,
        wire: &mut Q2Wire,
        messages: &mut Vec<MvdEmission>,
        event: Q2ServerEvent,
        recipient: MvdRecipient,
        reliable: bool,
    ) -> Result<(), Q2HostError> {
        messages.push(MvdEmission {
            bytes: encode_q2_server_event(wire, &event)?,
            recipient,
            reliable,
        });
        Ok(())
    }
}

impl<S, C, E, D, A> Q2ApplicationServerHost<S, C, E, D, A>
where
    S: Q2HostSimulation,
    C: Q2HostContent,
    E: Q2HostSession,
{
    /// Donor `mvdSettings`.
    pub fn mvd_settings(&mut self) -> Result<Q2MvdSettings, Q2HostError> {
        let cvars = self.simulation.q2_server_cvars().ok_or(Q2HostError::MissingCvars)?;
        Ok(Q2MvdSettings {
            enabled: cvars.variable_value("sv_mvd_enable") != 0.0,
            max_viewers: cvars.variable_value("sv_mvd_maxclients").trunc().clamp(1.0, 256.0) as u32,
            password: cvars.variable_string("sv_mvd_password"),
        })
    }

    /// Donor `discovery.status`.
    pub fn discovery_status(&mut self) -> Result<Q2Status, Q2HostError> {
        let server_info = self
            .simulation
            .q2_server_cvars()
            .ok_or(Q2HostError::MissingCvars)?
            .info_string(q2_flags::SERVER_INFO, None)?;
        Ok(Q2Status {
            server_info,
            players: self
                .simulation
                .q2_player_states()
                .into_iter()
                .filter(|player| player.connected)
                .map(|player| Q2StatusPlayer {
                    score: player.score,
                    ping: player.ping,
                    name: player.name,
                })
                .collect(),
        })
    }

    /// Donor `discovery.info`.
    pub fn discovery_info(&mut self) -> Result<Q2DiscoveryInfo, Q2HostError> {
        let cvars = self.simulation.q2_server_cvars().ok_or(Q2HostError::MissingCvars)?;
        let name = cvars.variable_string("hostname");
        Ok(Q2DiscoveryInfo {
            name,
            map: self.map_name.clone(),
            players: self
                .simulation
                .q2_player_states()
                .iter()
                .filter(|player| player.connected)
                .count() as u32,
            max_players: self.max_clients,
        })
    }

    /// Observe shared events into configstrings and entity events (donor
    /// `observe`).
    pub fn observe(
        &mut self,
        output: &SimulationOutput,
        events: &[SimulationPresentationEvent],
    ) -> Result<(), Q2HostError> {
        if self.event_frame != output.snapshot.frame.frame {
            self.entity_events.clear();
            self.event_frame = output.snapshot.frame.frame;
        }
        for item in events {
            match &item.event {
                SourcePresentationEvent::Q2(event) => match event {
                    Q2PresentationEvent::LightStyle { style, pattern } => {
                        self.configs.insert(
                            self.layout.lights
                                + u32::try_from(*style).map_err(|_| Q2HostError::WireRange("light style"))?,
                            pattern.clone(),
                        );
                    }
                    Q2PresentationEvent::EntityEvent { actor, event } => {
                        self.entity_events.insert(actor.clone(), *event);
                    }
                    Q2PresentationEvent::Sound(event) => {
                        self.sound(&event.path)?;
                    }
                    Q2PresentationEvent::Model(event) => {
                        self.model(&event.path)?;
                        for path in &event.attached_models {
                            self.model(path)?;
                        }
                    }
                    _ => {}
                },
                SourcePresentationEvent::Q2Player(Q2PlayerEvent::Userinfo { slot, name, skin, .. }) => {
                    self.configs.insert(
                        self.layout.player_skins
                            + u32::try_from(*slot).map_err(|_| Q2HostError::WireRange("player skin slot"))?,
                        format!("{name}\\{skin}"),
                    );
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Donor `supportsSourceWire`.
    #[must_use]
    pub fn supports_source_wire(&self) -> Q2WireSupport {
        let mut reasons = Vec::new();
        let recipe = self.simulation.q2_recipe();
        if !recipe.movement_provider.starts_with("q2:") {
            reasons.push("Selected movement requires unified peer serialization".to_string());
        }
        if !recipe.character_provider.starts_with("q2:") {
            reasons.push("Selected character requires unified peer serialization".to_string());
        }
        if self.protocol == ProtocolIdentity::Q2KexDemo {
            reasons.push("KEX native live transport is not bound".to_string());
        }
        let classic_mismatch = self.edition == Q2Edition::Classic
            && matches!(
                self.protocol,
                ProtocolIdentity::Q2Rerelease | ProtocolIdentity::Q2Kex | ProtocolIdentity::Q2KexDemo
            );
        let rerelease_mismatch = self.edition == Q2Edition::Rerelease
            && !matches!(self.protocol, ProtocolIdentity::Q2Rerelease | ProtocolIdentity::Q2Kex);
        if classic_mismatch || rerelease_mismatch {
            reasons.push("Selected application game API and native message layout differ".to_string());
        }
        if reasons.is_empty() {
            Q2WireSupport::Supported
        } else {
            Q2WireSupport::Unsupported { reasons }
        }
    }

    /// Admit a connecting client (donor `admit`).
    pub fn admit(
        &mut self,
        from: &NetworkAddress,
        request: &Q2ConnectRequest,
        split_seat: u32,
    ) -> Result<Q2ApplicationAdmission, Q2HostError> {
        let mut pairs = (self.parse_userinfo)(&request.userinfo);
        if request.protocol == ProtocolIdentity::Q2Kex {
            let suffix = format!("_{split_seat}");
            let stripped: Vec<(String, String)> = pairs
                .iter()
                .filter(|(key, _)| key.ends_with(&suffix))
                .map(|(key, value)| (key[..key.len() - suffix.len()].to_string(), value.clone()))
                .collect();
            for (key, value) in stripped {
                set_userinfo(&mut pairs, &key, value);
            }
        }
        set_userinfo(&mut pairs, "ip", address_key(from, true));
        let userinfo = join_userinfo(&pairs);
        let social_id = request
            .social_ids
            .as_ref()
            .and_then(|ids| ids.first().cloned())
            .unwrap_or_default();
        if !social_id.is_empty() && self.player_identity.is_none() {
            return Ok(Q2ApplicationAdmission::Rejected {
                reason: "Server has no rerelease social identity owner".to_string(),
            });
        }
        let allowed = self.simulation.q2_players_connect(&userinfo);
        if !allowed.allowed {
            return Ok(Q2ApplicationAdmission::Rejected { reason: allowed.reason });
        }
        let mut slot = 0;
        while slot < self.max_clients {
            let occupied = self.players.contains_key(&slot)
                || self.simulation.players().iter().any(|actor| {
                    self.simulation
                        .movement_player(actor)
                        .and_then(|movement| movement.client_slot)
                        == Some(slot)
                });
            if !occupied {
                break;
            }
            slot += 1;
        }
        if slot == self.max_clients {
            return Ok(Q2ApplicationAdmission::Rejected {
                reason: "Server is full".to_string(),
            });
        }
        let client = self.session.create_client(slot)?;
        self.session.connect_client(
            &client,
            if matches!(from, NetworkAddress::Loopback { .. }) {
                Q2ClientOrigin::Loopback
            } else {
                Q2ClientOrigin::Remote
            },
        );
        if let Some(identify) = &self.player_identity {
            identify(
                &client,
                Some(Q2SeatIdentity {
                    seat: split_seat,
                    social_id: social_id.clone(),
                }),
            );
        }
        let admitted = match self.simulation.admit_player(&client) {
            Ok(actor) => actor,
            Err(message) => {
                if let Some(identify) = &self.player_identity {
                    identify(&client, None);
                }
                self.session.close_client(&client);
                return Err(Q2HostError::GameCallback(message));
            }
        };
        if self.simulation.q2_source_entity(&admitted).is_none() {
            self.simulation.disconnect_player(&admitted);
            if let Some(identify) = &self.player_identity {
                identify(&client, None);
            }
            self.session.close_client(&client);
            return Err(Q2HostError::MissingEntity);
        }
        self.simulation
            .q2_players_userinfo_changed(&admitted, &allowed.userinfo);
        let source_entity = self.source_number(&admitted)?;
        let player = Q2ApplicationPlayer {
            client: client.clone(),
            actor: admitted,
            source_entity,
        };
        self.players.insert(slot, player.clone());
        Ok(Q2ApplicationAdmission::Accepted { player })
    }

    /// Disconnect a player (donor `disconnect`).
    pub fn disconnect(&mut self, player: &Q2ApplicationPlayer) {
        self.simulation.disconnect_player(&player.actor);
        if let Some(identify) = &self.player_identity {
            identify(&player.client, None);
        }
        self.session.close_client(&player.client);
        self.players.remove(&player.client.slot());
        self.known_configs.remove(&player.client.slot());
    }

    /// Rebind a carried client (donor `carriedPlayer`).
    pub fn carried_player(&mut self, client: &ClientId) -> Result<Q2ApplicationPlayer, Q2HostError> {
        let actor = self
            .simulation
            .players()
            .into_iter()
            .find(|actor| {
                self.simulation
                    .movement_player(actor)
                    .and_then(|movement| movement.client_slot)
                    .is_some_and(|slot| slot == client.slot())
            })
            .ok_or(Q2HostError::CarriedClient)?;
        let player = Q2ApplicationPlayer {
            client: client.clone(),
            actor: actor.clone(),
            source_entity: self.source_number(&actor)?,
        };
        self.players.insert(client.slot(), player.clone());
        Ok(player)
    }
}

impl<S, C, E, D, A> Q2ApplicationServerHost<S, C, E, D, A>
where
    S: Q2HostSimulation,
    C: Q2HostContent,
    E: Q2HostSession,
{
    /// Build the game state for a player (donor `gameState`).
    pub fn game_state(
        &mut self,
        player: &Q2ApplicationPlayer,
        protocol: ProtocolIdentity,
    ) -> Result<Q2ApplicationGameState, Q2HostError> {
        self.update_movement_configs()?;
        let entities = self.entity_states(protocol)?;
        for state in self.simulation.q2_player_states() {
            self.configs.insert(
                self.layout.player_skins + state.slot,
                format!("{}\\{}", state.name, state.skin),
            );
        }
        self.known_configs.insert(player.client.slot(), self.configs.clone());
        let (r1q2_version, q2pro_version) = match protocol {
            ProtocolIdentity::Q2R1q2 { revision } => (Some(revision), None),
            ProtocolIdentity::Q2Q2pro { revision } => (None, Some(revision)),
            _ => (None, None),
        };
        Ok(Q2ApplicationGameState {
            data: Q2ServerDataParams {
                base: ServerData {
                    servercount: 1,
                    attractloop: false,
                    gamedir: self.content_directory.clone(),
                    clientnum: player.source_entity as i16 - 1,
                    levelname: self.configs.get(&0).cloned().unwrap_or_default(),
                },
                server_state: 2,
                server_fps: if self.edition == Q2Edition::Rerelease {
                    40.0
                } else {
                    10.0
                },
                r1q2_version,
                r1q2_strafejump_hack: false,
                q2pro_version,
                wire_flags: 0,
            },
            config_strings: self.configs.clone(),
            baselines: entities.into_iter().map(|entity| (entity.number, entity)).collect(),
        })
    }

    /// Build a wire frame for a player (donor `frame`).
    pub fn frame(
        &mut self,
        player: &Q2ApplicationPlayer,
        output: &SimulationOutput,
        protocol: ProtocolIdentity,
    ) -> Result<Q2WireFrame, Q2HostError> {
        let body = self.simulation.body(&player.actor).ok_or(Q2HostError::MissingBody)?;
        let state = self.player_state(&player.actor)?;
        let origin = Vec3 {
            x: body.origin.x + state.viewoffset[0] as f32,
            y: body.origin.y + state.viewoffset[1] as f32,
            z: body.origin.z + state.viewoffset[2] as f32,
        };
        let area_bits = {
            let scene = self.simulation.scene();
            scene.area_bits(scene.leaf_area(scene.point_leaf(&origin)))
        };
        let states = self.entity_states(protocol)?;
        let entities = self.visible_entities(player, states, &origin)?;
        Ok(Q2WireFrame {
            valid: true,
            server_frame: output.snapshot.frame.frame,
            delta_frame: -1,
            suppressed_count: 0,
            area_bits,
            player: state,
            split_players: Vec::new(),
            entities,
        })
    }

    /// Collect per-frame events for a player (donor `events`).
    pub fn events(
        &mut self,
        player: &Q2ApplicationPlayer,
        _output: &SimulationOutput,
        events: &[SimulationPresentationEvent],
    ) -> Result<Vec<Q2ApplicationServerEvent>, Q2HostError> {
        self.update_movement_configs()?;
        let mut messages = Vec::new();
        for item in events {
            match &item.event {
                SourcePresentationEvent::Q2Player(event) => match event {
                    Q2PlayerEvent::Print { target, level, text } => {
                        if target.as_ref().is_none_or(|target| target == &player.actor) {
                            messages.push(Q2ApplicationServerEvent {
                                event: Q2ServerEvent::Print {
                                    level: player_print_level(*level),
                                    text: text.clone(),
                                },
                                reliable: None,
                            });
                        }
                    }
                    Q2PlayerEvent::StuffText { actor, text } => {
                        if actor == &player.actor {
                            messages.push(Q2ApplicationServerEvent {
                                event: Q2ServerEvent::CommandText { text: text.clone() },
                                reliable: None,
                            });
                        }
                    }
                    Q2PlayerEvent::Userinfo { slot, name, skin, .. } => {
                        self.configs.insert(
                            self.layout.player_skins
                                + u32::try_from(*slot).map_err(|_| Q2HostError::WireRange("player skin slot"))?,
                            format!("{name}\\{skin}"),
                        );
                    }
                    Q2PlayerEvent::Inventory { actor, entries, .. } if actor == &player.actor => {
                        let mut counts = vec![0i16; 256];
                        for entry in entries {
                            let ordinal = self.inventory_ordinal(&entry.item);
                            if ordinal > 0 && ordinal < 256 {
                                counts[ordinal as usize] = entry.count as i16;
                            }
                        }
                        messages.push(Q2ApplicationServerEvent {
                            event: Q2ServerEvent::Inventory { counts },
                            reliable: None,
                        });
                    }
                    _ => {}
                },
                SourcePresentationEvent::Q2(event) => match event {
                    Q2PresentationEvent::Print { actor, level, text } => {
                        if actor.as_ref().is_none_or(|actor| actor == &player.actor) {
                            messages.push(Q2ApplicationServerEvent {
                                event: Q2ServerEvent::Print {
                                    level: host_print_level(*level),
                                    text: text.clone(),
                                },
                                reliable: None,
                            });
                        }
                    }
                    Q2PresentationEvent::CenterPrint { actor, text, .. } => {
                        if actor == &player.actor {
                            messages.push(Q2ApplicationServerEvent {
                                event: Q2ServerEvent::CenterPrint { text: text.clone() },
                                reliable: None,
                            });
                        }
                    }
                    Q2PresentationEvent::LightStyle { style, pattern } => {
                        self.configs.insert(
                            self.layout.lights
                                + u32::try_from(*style).map_err(|_| Q2HostError::WireRange("light style"))?,
                            pattern.clone(),
                        );
                    }
                    Q2PresentationEvent::Effect(event) => {
                        if let Some(value) = q2_effect_to_wire(event) {
                            messages.push(Q2ApplicationServerEvent {
                                event: Q2ServerEvent::TempEntity { value },
                                reliable: None,
                            });
                        }
                    }
                    Q2PresentationEvent::Sound(event) => {
                        if event.loop_ == Q2SoundLoop::Once {
                            let entity = match &event.actor {
                                None => 0,
                                Some(actor) => match item.source_entity {
                                    Some(number) => number as u32,
                                    None => self.source_number(actor)?,
                                },
                            };
                            let index = u16::try_from(self.sound(&event.path)?)
                                .map_err(|_| Q2HostError::WireRange("sound index"))?;
                            messages.push(Q2ApplicationServerEvent {
                                event: Q2ServerEvent::Sound {
                                    sound: Q2SoundMessage {
                                        flags: 0,
                                        index,
                                        entity,
                                        channel: event.channel as u8,
                                        position: Some(wire_vec(&event.origin)),
                                        volume: event.volume,
                                        attenuation: event.attenuation,
                                        delay_seconds: 0.0,
                                    },
                                },
                                reliable: Some(event.reliable),
                            });
                        }
                    }
                    Q2PresentationEvent::MonsterMuzzleflash { actor, flash, .. } => {
                        let entity = match item.source_entity {
                            Some(number) => number,
                            None => self.source_number(actor)? as i32,
                        };
                        messages.push(Q2ApplicationServerEvent {
                            event: Q2ServerEvent::MuzzleFlash {
                                entity,
                                flash: *flash,
                                monster: true,
                                silenced: false,
                            },
                            reliable: None,
                        });
                    }
                    _ => {}
                },
                SourcePresentationEvent::Q2Weapon(Q2WeaponEvent::Muzzleflash {
                    actor, flash, silenced, ..
                }) => {
                    messages.push(Q2ApplicationServerEvent {
                        event: Q2ServerEvent::MuzzleFlash {
                            entity: self.source_number(actor)? as i32,
                            flash: *flash,
                            monster: false,
                            silenced: *silenced,
                        },
                        reliable: None,
                    });
                }
                _ => {}
            }
        }
        let known = self
            .known_configs
            .get_mut(&player.client.slot())
            .ok_or(Q2HostError::MissingConfigHistory)?;
        let mut updates = Vec::new();
        for (index, value) in &self.configs {
            if known.get(index) != Some(value) {
                updates.push(Q2ApplicationServerEvent {
                    event: Q2ServerEvent::ConfigString {
                        index: u16::try_from(*index).map_err(|_| Q2HostError::WireRange("configstring index"))?,
                        value: value.clone(),
                    },
                    reliable: None,
                });
                known.insert(*index, value.clone());
            }
        }
        updates.extend(messages);
        Ok(updates)
    }

    /// Translate a wire command into an actor command (donor `input`).
    #[must_use]
    pub fn input(&self, player: &Q2ApplicationPlayer, command: &Usercmd, sequence: u64) -> ActorCommand {
        let contract = if self.edition == Q2Edition::Classic {
            let native = to_q2_command(command);
            NetUserCommand::Q2Classic {
                milliseconds: f64::from(native.milliseconds),
                angle_shorts: [
                    f64::from(native.angle_shorts[0]),
                    f64::from(native.angle_shorts[1]),
                    f64::from(native.angle_shorts[2]),
                ],
                forward_move: f64::from(native.forward_move),
                side_move: f64::from(native.side_move),
                up_move: f64::from(native.up_move),
                buttons: f64::from(native.buttons),
                impulse: f64::from(native.impulse),
                light_level: f64::from(native.light_level),
            }
        } else {
            let server_frame = if self.protocol == ProtocolIdentity::Q2Kex {
                command.server_frame
            } else {
                sequence.min(i32::MAX as u64) as i32
            };
            let native = to_q2_rerelease_command(command, server_frame);
            NetUserCommand::Q2Rerelease {
                milliseconds: f64::from(native.milliseconds),
                angles: [native.angles.x, native.angles.y, native.angles.z],
                forward_move: f64::from(native.forward_move),
                side_move: f64::from(native.side_move),
                buttons: f64::from(native.buttons),
                server_frame: f64::from(native.server_frame),
            }
        };
        ActorCommand {
            actor: player.actor.clone(),
            source: CommandSource::Remote {
                client: player.client.clone(),
            },
            sequence,
            command: contract,
            arsenal: None,
        }
    }

    /// Donor `expandClientCommand`.
    pub fn expand_client_command(&mut self, text: &str) -> Result<Option<String>, Q2HostError> {
        let print = Rc::clone(&self.print);
        let mut print = |line: &str| print(line);
        let cvars = self.simulation.q2_server_cvars().ok_or(Q2HostError::MissingCvars)?;
        Ok(expand_command_macros(
            text,
            &|name| cvars.variable_string(name),
            &mut print,
            TextMode::Source,
        )?)
    }

    /// Run a client command (donor `command`).
    pub fn command(&mut self, player: &Q2ApplicationPlayer, name: &str, args: &[String]) -> Result<(), Q2HostError> {
        if self.simulation.q2_source_entity(&player.actor).is_none() {
            return Err(Q2HostError::CommandEntity);
        }
        self.simulation.q2_players_client_command(&player.actor, name, args);
        Ok(())
    }

    /// Apply client userinfo (donor `userinfo`).
    pub fn userinfo(&mut self, player: &Q2ApplicationPlayer, value: &str) -> Result<(), Q2HostError> {
        if self.simulation.q2_source_entity(&player.actor).is_none() {
            return Err(Q2HostError::UserinfoEntity);
        }
        self.simulation.q2_players_userinfo_changed(&player.actor, value);
        self.simulation.notify_client_event("userinfo", &player.actor);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::q2::foundation::host::Q2EffectEvent;
    use qa_net::q2_net::{Q2TempField, Q2TempInt, Q2TempType, Q2TempVec};

    fn effect(name: &str) -> Q2EffectEvent {
        Q2EffectEvent {
            effect: name.to_string(),
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            direction: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            count: 9,
            color: 0xd0,
        }
    }

    #[test]
    fn direction_effect_wires_position_and_direction() {
        let wired = q2_effect_to_wire(&effect("q2:gunshot")).expect("gunshot wires");
        assert_eq!(wired.temp_type, Q2TempType::Gunshot as u8);
        assert_eq!(
            wired.fields,
            vec![
                Q2TempField::Vector {
                    name: Q2TempVec::Position1,
                    value: [1.0, 2.0, 3.0]
                },
                Q2TempField::Vector {
                    name: Q2TempVec::Direction,
                    value: [0.0, 0.0, 1.0]
                },
            ]
        );
    }

    #[test]
    fn splash_effect_wires_count_and_color() {
        let wired = q2_effect_to_wire(&effect("splash")).expect("splash wires");
        assert_eq!(wired.temp_type, Q2TempType::Splash as u8);
        assert_eq!(
            wired.fields,
            vec![
                Q2TempField::Integer {
                    name: Q2TempInt::Count,
                    value: 9
                },
                Q2TempField::Vector {
                    name: Q2TempVec::Position1,
                    value: [1.0, 2.0, 3.0]
                },
                Q2TempField::Vector {
                    name: Q2TempVec::Direction,
                    value: [0.0, 0.0, 1.0]
                },
                Q2TempField::Integer {
                    name: Q2TempInt::Color,
                    value: 0xd0
                },
            ]
        );
    }

    #[test]
    fn position_effect_wires_position_only() {
        let wired = q2_effect_to_wire(&effect("q2:explosion1")).expect("explosion wires");
        assert_eq!(wired.temp_type, Q2TempType::Explosion1 as u8);
        assert_eq!(
            wired.fields,
            vec![Q2TempField::Vector {
                name: Q2TempVec::Position1,
                value: [1.0, 2.0, 3.0]
            }]
        );
    }

    #[test]
    fn unknown_effect_stays_unwired() {
        assert!(q2_effect_to_wire(&effect("q2:glitter")).is_none());
    }

    #[test]
    fn userinfo_round_trip_replaces_keys() {
        let mut info = vec![("name".to_string(), "a".to_string())];
        set_userinfo(&mut info, "name", "b".to_string());
        set_userinfo(&mut info, "ip", "loopback:0".to_string());
        assert_eq!(join_userinfo(&info), "\\name\\b\\ip\\loopback:0");
    }

    #[test]
    fn print_levels_match_wire_values() {
        assert_eq!(host_print_level(Q2HostPrintLevel::Chat), 3);
        assert_eq!(host_print_level(Q2HostPrintLevel::High), 2);
        assert_eq!(host_print_level(Q2HostPrintLevel::Medium), 1);
        assert_eq!(host_print_level(Q2HostPrintLevel::Low), 0);
        assert_eq!(player_print_level(Q2PlayerPrintLevel::Chat), 3);
        assert_eq!(player_print_level(Q2PlayerPrintLevel::Low), 0);
    }

    #[test]
    fn reexported_player_shares_canonical_identity() {
        use crate::bootstrap::network::types as canonical;
        let owner = qa_core::identity::IdentityOwner::create("network-unify-test").unwrap();
        let player: canonical::Q2ApplicationPlayer = Q2ApplicationPlayer {
            client: owner.client(1, 1),
            actor: owner.actor(1, 1),
            source_entity: 9,
        };
        let admitted = Q2ApplicationAdmission::Accepted { player };
        let canonical::Q2ApplicationAdmission::Accepted { player } = admitted else {
            panic!("expected accepted");
        };
        assert_eq!(player.source_entity, 9);
    }
}
