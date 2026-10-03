//! Native QuakeWorld server network host over the admitted QuakeC source.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/network-qw.ts`.
//!
//! This file is self-contained like its donor and the sibling host ports:
//! shared application shapes are mirrored here with canonical-home notes
//! instead of importing sibling ports. The async content reads
//! (`LoadedApplicationContent.mounts`) live behind the synchronous
//! [`QwHostContent`] seam; bootstrap ports are sync.
//!
//! Sibling homes (narrow host seams over live siblings):
//! - [`SharedSimulation`](super::runtime::SharedSimulation)
//!   (`simulation/runtime.ts` port): [`QwHostSimulation`].
//! - QuakeWorld QuakeC source (`simulation.quakecSource`,
//!   [`quakec_source`](super::quakec_source) port): [`QwHostGame`].
//! - [`LoadedApplicationContent`](crate::bootstrap::content::LoadedApplicationContent)
//!   (`bootstrap/content.ts` port): [`QwHostContent`].
//! - [`EngineSession`](qa_world::session::EngineSession)
//!   (`world/session/session.ts` port): [`QwHostSession`].
//! - `bootstrap/network/qw-server-types.ts`
//!   ([`network::qw_server_types`](crate::bootstrap::network::qw_server_types)
//!   port, `QwApplicationServerHost` and friends): mirrored here as
//!   [`QwApplicationServerHost`] and the `Qw*` shapes, which keep
//!   host-local shapes.
//!
//! The donor `signon` returns a per-player closure object; the closures are
//! flattened here into `signon_*`/`spawn`/`begin` host methods taking the
//! player each time. `QwServerMessage` is the donor
//! `Exclude<QuakeWorldMessage, 'packet-entities' | 'invalid-delta'>`; the
//! ported [`QuakeWorldMessage`] is used directly and the two excluded
//! variants are filtered at the donor filter sites.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use qa_content::q1::foundation::types::Q1Event;
use qa_core::cvar::{CvarError, CvarRegistry};
use qa_core::identity::{ActorId, ClientId};
use qa_core::math::{Bounds, Vec3};
use qa_guest::qc::message_routing::{receives_quake_world_message, QwVisibilityScene};
use qa_guest::qc::presentation_host::{QcMessageDestination, VisibilityScope};
use qa_net::msg::{MsgError, MsgWriter};
use qa_net::protocol::qw::{
    PF_COMMAND, PF_DEAD, PF_EFFECTS, PF_GIB, PF_MODEL, PF_MSEC, PF_SKINNUM, PF_VELOCITY1, PF_VELOCITY2, PF_VELOCITY3,
    PF_WEAPONFRAME,
};
use qa_net::q1_net::{
    quake_world_info, quake_world_map_checksum2, write_quake_world_message, Q1NetError, Q1WireEntity,
    QuakeWorldMessage, QwMoveVariables, QwPlayerState, QwProjectile, QwSlotStat, QwSlotValue,
};
use qa_net::q1_wide::{QwProfile, WideEntityState};
use qa_net::qw::QwEntityState;
use qa_net::services::downloads::download_path;
use qa_world::session::SimulationOutput;
use qa_world::WorldError;
use thiserror::Error;

use super::types::{SimulationPresentationEvent, SourcePresentationEvent};

/// Mirror of `QwApplicationPlayer` from donor
/// `src/app/bootstrap/network/qw-server-types.ts` (canonical home:
/// `crate::bootstrap::network::qw_server_types`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QwApplicationPlayer {
    /// Client handle.
    pub client: ClientId,
    /// Player actor.
    pub actor: ActorId,
    /// Client slot.
    pub slot: u32,
}

/// Mirror of the donor `admit` request (canonical home:
/// `crate::bootstrap::network::qw_server_types`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwAdmitRequest {
    /// Whether the client connects as a spectator.
    pub spectator: bool,
    /// Connect userinfo.
    pub userinfo: String,
}

/// Mirror of the donor `admit` result (canonical home:
/// `crate::bootstrap::network::qw_server_types`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QwAdmission {
    /// Client accepted.
    Accepted {
        /// Admitted player.
        player: QwApplicationPlayer,
    },
    /// Client rejected.
    Rejected {
        /// Reason.
        reason: String,
    },
}

/// Mirror of `QwUserCommand` from donor `src/contracts/protocol.ts`
/// (canonical home: `qa_net::qw`); unify post-merge.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct QwUserCommand {
    /// Milliseconds.
    pub milliseconds: u8,
    /// View angles in degrees.
    pub angles: Vec3,
    /// Forward move.
    pub forward_move: i16,
    /// Side move.
    pub side_move: i16,
    /// Up move.
    pub up_move: i16,
    /// Buttons.
    pub buttons: u8,
    /// Impulse.
    pub impulse: u8,
}

/// QuakeWorld client role (donor `'player' | 'spectator'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QwRole {
    /// Player.
    Player,
    /// Spectator.
    Spectator,
}

/// Connected client state (donor `ClientState`).
#[derive(Debug, Clone, PartialEq)]
struct QwClientState {
    /// Player binding.
    player: QwApplicationPlayer,
    /// Client role.
    role: QwRole,
    /// Client info.
    info: BTreeMap<String, String>,
    /// Whether the client began.
    begun: bool,
    /// Latest command.
    command: QwUserCommand,
    /// Latest command time in seconds.
    command_time: f64,
    /// Last sent stats by index.
    stats: HashMap<u32, i32>,
    /// Last sent frags.
    frags: f64,
}

/// Mirror of the donor `frame` result (canonical home:
/// `crate::bootstrap::network::qw_server_types`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct QwFrame {
    /// Unreliable messages.
    pub messages: Vec<QuakeWorldMessage>,
    /// Reliable messages.
    pub reliable: Vec<QuakeWorldMessage>,
    /// Visible entities.
    pub entities: Vec<QwEntityState>,
}

/// Mirror of the donor `authentication` getters (canonical home:
/// `crate::bootstrap::network::qw_server_types`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwAuthentication {
    /// Server password.
    pub password: String,
    /// Spectator password.
    pub spectator_password: String,
    /// High characters.
    pub high_characters: bool,
}

/// Mirror of the donor `supportsSourceWire` result (canonical home:
/// `crate::bootstrap::network::qw_server_types`); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QwWireSupport {
    /// Native wire supported.
    Supported,
    /// Native wire unsupported.
    Unsupported {
        /// Reasons.
        reasons: Vec<String>,
    },
}

/// QuakeWorld entity read (donor `QuakeWorldEntity` used surface).
#[derive(Debug, Clone, PartialEq)]
pub struct QwHostEntity {
    /// Entity number.
    pub number: u16,
    /// Model index.
    pub model_index: u8,
    /// Frame.
    pub frame: u8,
    /// Colormap.
    pub color_map: u8,
    /// Skin.
    pub skin: u8,
    /// Effects.
    pub effects: u8,
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
}

impl QwHostEntity {
    /// Baseline/static wire encoding.
    fn to_wire(&self) -> Q1WireEntity {
        Q1WireEntity {
            number: u32::from(self.number),
            state: WideEntityState {
                modelindex: u16::from(self.model_index),
                frame: u16::from(self.frame),
                colormap: self.color_map,
                skin: self.skin,
                effects: self.effects,
                origin: [
                    f64::from(self.origin.x),
                    f64::from(self.origin.y),
                    f64::from(self.origin.z),
                ],
                angles: [
                    f64::from(self.angles.x),
                    f64::from(self.angles.y),
                    f64::from(self.angles.z),
                ],
                alpha: 0,
                scale: 16,
            },
            lerp_finish_seconds: 0.0,
            step: false,
            quakeworld_flags: 0,
        }
    }

    /// Frame entity encoding.
    fn to_qw(&self) -> QwEntityState {
        QwEntityState {
            number: self.number,
            origin: [
                f64::from(self.origin.x),
                f64::from(self.origin.y),
                f64::from(self.origin.z),
            ],
            angles: [
                f64::from(self.angles.x),
                f64::from(self.angles.y),
                f64::from(self.angles.z),
            ],
            modelindex: self.model_index,
            frame: self.frame,
            colormap: self.color_map,
            skinnum: self.skin,
            effects: self.effects,
            flags: 0,
            alpha: 0,
            scale: 16,
            solid: false,
        }
    }
}

/// Drained QuakeC message batch (donor `drainMessages` batch surface).
#[derive(Debug, Clone, PartialEq)]
pub struct QwMessageBatch {
    /// Batch entries.
    pub entries: Vec<QuakeWorldMessage>,
    /// Batch destination.
    pub destination: QcMessageDestination,
}

/// Routed message with its destination.
#[derive(Debug, Clone, PartialEq)]
struct QwRoutedMessage {
    /// Message.
    message: QuakeWorldMessage,
    /// Destination.
    destination: QcMessageDestination,
}

/// Carried travel client (donor `options.travel.source.clients` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwTravelClient {
    /// Client handle.
    pub client: ClientId,
    /// Carried userinfo.
    pub user_info: Vec<(String, String)>,
}

/// Mounted download asset (donor `mounts.open` result surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwDownloadAsset {
    /// Asset bytes.
    pub bytes: Vec<u8>,
    /// Whether the reference is downloadable.
    pub downloadable: bool,
    /// Whether the provenance is an archive.
    pub archive_provenance: bool,
}

/// Prepared download handle (donor `prepareDownload` result).
#[derive(Debug, Clone)]
pub struct QwPreparedDownload {
    /// Byte length.
    pub byte_length: usize,
    /// Retained bytes; cleared by close.
    data: Rc<RefCell<Option<Vec<u8>>>>,
}

impl QwPreparedDownload {
    /// Read a byte range (donor `read`).
    pub fn read(&self, offset: usize, maximum: usize) -> Result<Vec<u8>, QwHostError> {
        let data = self.data.borrow();
        let bytes = data.as_ref().ok_or(QwHostError::DownloadClosed)?;
        let end = offset.saturating_add(maximum).min(bytes.len());
        let start = offset.min(bytes.len());
        Ok(bytes[start..end].to_vec())
    }

    /// Close the download (donor `close`).
    pub fn close(&self) {
        self.data.borrow_mut().take();
    }
}

/// Precache table kind (donor `precacheNames` argument).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QwPrecacheKind {
    /// Model precaches.
    Model,
    /// Sound precaches.
    Sound,
}

/// Visibility scope for scene queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QwVisibilityScope {
    /// Potentially visible set.
    Pvs,
    /// Potentially hearable set.
    Phs,
}

/// Scene surface used by this host.
///
/// Seam over `SharedSimulation["scene"]` from donor
/// `src/app/bootstrap/simulation/runtime.ts` (canonical home: the runtime
/// partition); the partition implements it post-merge.
pub trait QwHostScene {
    /// Donor `pointLeaf`.
    fn point_leaf(&self, point: &Vec3) -> i32;
    /// Donor `leafCluster`.
    fn leaf_cluster(&self, leaf: i32) -> i32;
    /// Donor `boxLeaves` leaves.
    fn box_leaves(&self, min: &Vec3, max: &Vec3, limit: usize) -> Vec<i32>;
    /// Donor `clusterVisible`.
    fn cluster_visible(&self, from: i32, to: i32, scope: QwVisibilityScope) -> bool;
}

/// Adapter from the host scene seam to the ported visibility scene.
struct QwSceneAdapter<'s>(&'s dyn QwHostScene);

impl QwVisibilityScene for QwSceneAdapter<'_> {
    fn point_leaf(&self, point: Vec3) -> u32 {
        self.0.point_leaf(&point) as u32
    }

    fn leaf_cluster(&self, leaf: u32) -> i32 {
        self.0.leaf_cluster(leaf as i32)
    }

    fn cluster_visible(&self, from: i32, to: i32, scope: VisibilityScope) -> bool {
        // The ported router short-circuits `All` before querying.
        let scope = match scope {
            VisibilityScope::All | VisibilityScope::Pvs => QwVisibilityScope::Pvs,
            VisibilityScope::Phs => QwVisibilityScope::Phs,
        };
        self.0.cluster_visible(from, to, scope)
    }
}

/// QuakeWorld QuakeC source surface used by this host.
///
/// Seam over `simulation.quakecSource()` from donor
/// `src/app/bootstrap/simulation/runtime.ts` (canonical home: the runtime
/// partition); the partition implements it post-merge.
pub trait QwHostGame {
    /// Whether this is a QuakeWorld source (`game.kind`).
    fn is_quakeworld(&self) -> bool;
    /// Donor `game.precacheNames`.
    fn precache_names(&self, kind: QwPrecacheKind) -> Vec<String>;
    /// Donor `game.cvars`.
    fn cvars(&self) -> &CvarRegistry;
    /// Donor `game.cvars`, mutably.
    fn cvars_mut(&mut self) -> &mut CvarRegistry;
    /// Donor `prepared.program.fieldsByName` offset.
    fn field_offset(&self, name: &str) -> Option<i32>;
    /// Donor `game.sourceSlot`.
    fn source_slot(&self, actor: &ActorId) -> Option<u32>;
    /// Donor `game.entities.at(slot).float(offset)`.
    fn entity_float(&self, slot: u32, offset: i32) -> f64;
    /// Donor `game.entities.at(slot).vector(offset)`.
    fn entity_vector(&self, slot: u32, offset: i32) -> Vec3;
    /// Donor `game.entities.at(slot).int(offset)`.
    fn entity_int(&self, slot: u32, offset: i32) -> i32;
    /// Donor `game.machine.strings.get`.
    fn lookup_string(&self, index: i32) -> String;
    /// Donor `game.machine.globals.float(game.machine.globalOffset(name))`.
    fn global_float(&self, name: &str) -> f64;
    /// Donor `game.timeSeconds`.
    fn time_seconds(&self) -> f64;
    /// Donor `game.signonMessages`.
    fn signon_messages(&self) -> Vec<QuakeWorldMessage>;
    /// Donor `game.drainMessages`.
    fn drain_messages(&mut self) -> Vec<QwMessageBatch>;
    /// Donor `game.messages.flush`.
    fn flush_messages(&mut self);
    /// Donor `game.setClientRole`.
    fn set_client_role(&mut self, client: &ClientId, role: QwRole);
    /// Donor `game.setClientInfo`.
    fn set_client_info(&mut self, client: &ClientId, info: &BTreeMap<String, String>);
    /// Donor `game.reservedClient`; the message propagates like the donor
    /// throw.
    fn reserved_client(&mut self, client: &ClientId) -> Result<ActorId, String>;
    /// Donor `game.isActiveClient`.
    fn is_active_client(&self, actor: &ActorId) -> bool;
    /// Donor `game.isSpectatorClient`.
    fn is_spectator_client(&self, actor: &ActorId) -> bool;
    /// Donor `game.clientInfo`.
    fn client_info(&self, client: &ClientId) -> BTreeMap<String, String>;
    /// Donor `game.disconnectClient`.
    fn disconnect_client(&mut self, actor: &ActorId);
    /// Donor `game.prepareClientSpawn`.
    fn prepare_client_spawn(&mut self, client: &ClientId);
    /// Donor `game.clientKill`.
    fn client_kill(&mut self, actor: &ActorId) -> bool;
}

/// Simulation surface used by this host.
///
/// Seam over `SharedSimulation` from donor
/// `src/app/bootstrap/simulation/runtime.ts` (canonical home: the runtime
/// partition); the partition implements it post-merge.
pub trait QwHostSimulation {
    /// Map entities provider (donor `simulation.recipe.map.entities.provider`).
    fn recipe_entities_provider(&self) -> String;
    /// Donor `simulation.actors.ownedBy`.
    fn owned_actors(&self, provider: &str) -> Vec<ActorId>;
    /// Donor `simulation.actors.resolveOwned`.
    fn resolve_owned_actor(&self, actor: &ActorId) -> Option<ActorId>;
    /// Donor `simulation.bodies.linked` absolute bounds.
    fn linked_bounds(&self, actor: &ActorId) -> Option<Bounds>;
    /// Donor `simulation.scene`.
    fn scene(&self) -> &dyn QwHostScene;
    /// Donor `simulation.players`.
    fn players(&self) -> Vec<ActorId>;
    /// Donor `simulation.movementPlayer` client.
    fn movement_client(&self, actor: &ActorId) -> Option<ClientId>;
    /// Donor `simulation.q1Paused`.
    fn q1_paused(&self) -> bool;
    /// Donor `simulation.toggleQ1Pause`.
    fn toggle_q1_pause(&mut self, actor: &ActorId) -> String;
    /// Donor `simulation.options.maxClients`.
    fn max_clients(&self) -> u32;
    /// QuakeWorld travel clients (donor `options.travel.source`), if any.
    fn travel_clients(&self) -> Option<Vec<QwTravelClient>>;
    /// Donor `simulation.queueQuakeWorldAction`; runs `action` before
    /// returning.
    fn queue_quakeworld_action(&mut self, client: &ClientId, action: &mut dyn FnMut());
    /// Donor `simulation.queueQuakeWorldCommands`.
    fn queue_quakeworld_commands(&mut self, client: &ClientId, commands: &[QwUserCommand], sequence: u64);
    /// Donor `simulation.admitPlayer`; the message propagates like the donor
    /// throw.
    fn admit_player(&mut self, client: &ClientId) -> Result<ActorId, String>;
    /// Donor `simulation.disconnectPlayer`.
    fn disconnect_player(&mut self, actor: &ActorId);
    /// Donor `simulation.notifyClientEvent`.
    fn notify_client_event(&mut self, kind: &str, actor: &ActorId);
    /// Donor `simulation.events.capture().persistent` Q1 events.
    fn q1_persistent_events(&self) -> Vec<Q1Event>;
    /// Donor `simulation.events.lightStyle`.
    fn light_style(&self, index: u32) -> String;
}

/// Content surface used by this host.
///
/// Seam over `LoadedApplicationContent` from donor
/// `src/app/bootstrap/content.ts` (canonical home: the content partition);
/// the partition implements it post-merge.
pub trait QwHostContent {
    /// Whether `world.kind` is `q1-bsp`.
    fn is_q1_bsp_world(&self) -> bool;
    /// Donor `world.leaves.length`.
    fn world_leaf_count(&self) -> usize;
    /// Donor `simulation.recipe.map.geometry.requestedPath`.
    fn recipe_geometry_path(&self) -> String;
    /// Donor `mounts.read` bytes.
    fn map_geometry_bytes(&self, path: &str) -> Result<Vec<u8>, String>;
    /// Whether `preparedQuakeC` is present.
    fn has_prepared_quakec(&self) -> bool;
    /// Basename of the entities content directory (donor `gameDirectory`).
    fn game_directory(&self) -> String;
    /// Donor `mounts.open` result.
    fn open_download(&self, path: &str) -> Option<QwDownloadAsset>;
}

/// Session surface used by this host.
///
/// Seam over `EngineSession` from donor `src/world/session/session.ts`
/// (canonical home: `qa_world::session`); the partition implements it
/// post-merge.
pub trait QwHostSession {
    /// Donor `session.createClient`.
    fn create_client(&mut self, slot: u32) -> Result<ClientId, WorldError>;
    /// Donor `client.connect('remote')`.
    fn connect_remote_client(&mut self, client: &ClientId);
    /// Donor `session.closeClient`.
    fn close_client(&mut self, client: &ClientId);
}

/// Print sink, mirroring donor `print`.
pub type QwPrintFn = Rc<dyn Fn(&str)>;
/// Master server listing, mirroring donor `masters`.
pub type QwMastersFn = Rc<dyn Fn() -> Vec<QwMasterServer>>;
/// Master server address (donor `masters` entry surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwMasterServer {
    /// Address text.
    pub address: String,
}

/// Mirror of `QwApplicationServerBindingOptions` from donor
/// `src/app/bootstrap/simulation/network-qw.ts` (canonical home: this file).
pub struct QwApplicationServerOptions<S, G, C, E, A> {
    /// Engine session.
    pub session: E,
    /// Shared simulation.
    pub simulation: S,
    /// Admitted QuakeC source, if any.
    pub game: Option<G>,
    /// Loaded content.
    pub content: C,
    /// Server count.
    pub server_count: i64,
    /// Rcon administration host.
    pub administration: Option<A>,
    /// Master server listing.
    pub masters: Option<QwMastersFn>,
    /// Print sink.
    pub print: QwPrintFn,
}

/// Native QuakeWorld server host error.
#[derive(Debug, Error)]
pub enum QwHostError {
    /// No admitted QuakeWorld source or Q1 BSP world is bound.
    #[error("Native QW wire requires its admitted QuakeWorld source and 32 reserved players")]
    MissingGame,
    /// Precache tables exceed 8-bit indices.
    #[error("Native QW precaches exceed 8-bit indices")]
    PrecacheOverflow,
    /// QuakeC source field is missing.
    #[error("Missing QW source field {0}")]
    MissingField(String),
    /// Actor has no live source slot.
    #[error("QW actor has no live source slot")]
    MissingSlot,
    /// Client handle is stale.
    #[error("Stale QW client")]
    StaleClient,
    /// Recording requires an active source player.
    #[error("QW recording requires an active source player")]
    RecordingPlayer,
    /// Recording slot belongs to another source player.
    #[error("QW recording client slot belongs to another source player")]
    RecordingSlot,
    /// Begin changed the reserved actor.
    #[error("QW begin changed the reserved actor")]
    BeginChangedActor,
    /// Wire value out of range.
    #[error("QW wire value out of range: {0}")]
    WireRange(&'static str),
    /// Mounted download is closed.
    #[error("QW mounted download is closed")]
    DownloadClosed,
    /// Game callback failure; the message propagates like the donor throw.
    #[error("{0}")]
    GameCallback(String),
    /// Content read failure.
    #[error("QW host content is unavailable: {0}")]
    Content(String),
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] Q1NetError),
    /// Message writer failure.
    #[error(transparent)]
    Message(#[from] MsgError),
    /// Session failure.
    #[error(transparent)]
    Session(#[from] WorldError),
}

/// Native QuakeWorld application server host.
///
/// Concrete mirror of `QwApplicationServerHost` from donor
/// `src/app/bootstrap/network/qw-server-types.ts` (canonical home:
/// `crate::bootstrap::network::qw_server_types`); unify post-merge.
pub struct QwApplicationServerHost<S, G, C, E, A> {
    /// Engine session.
    session: E,
    /// Shared simulation.
    simulation: S,
    /// Admitted QuakeC source.
    game: G,
    /// Loaded content.
    content: C,
    /// Server count.
    server_count: i64,
    /// Rcon administration host.
    administration: Option<A>,
    /// Master server listing.
    masters: Option<QwMastersFn>,
    /// Print sink.
    print: QwPrintFn,
    /// Model precache names.
    models: Vec<String>,
    /// Sound precache names.
    sounds: Vec<String>,
    /// Model name to 1-based index.
    model_index: HashMap<String, u32>,
    /// Map checksum.
    checksum: u32,
    /// Connected clients by slot.
    clients: HashMap<u32, QwClientState>,
    /// Entity baselines.
    baselines: Vec<Q1WireEntity>,
    /// Signon buffers.
    signon: Vec<Vec<u8>>,
    /// Queued routed messages.
    queued: Vec<QwRoutedMessage>,
    /// Messages routed this frame.
    routed: Vec<QwRoutedMessage>,
    /// Light styles by index.
    styles: [String; 64],
    /// Pause state at the last observation.
    previous_pause: bool,
    /// Player model index.
    player_model: u32,
}

impl<S, G, C, E, A> std::fmt::Debug for QwApplicationServerHost<S, G, C, E, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QwApplicationServerHost")
            .field("server_count", &self.server_count)
            .field("clients", &self.clients)
            .finish_non_exhaustive()
    }
}

/// Create the native QuakeWorld application server host (donor
/// `createQwApplicationServerHost`).
pub fn create_qw_application_server_host<S, G, C, E, A>(
    options: QwApplicationServerOptions<S, G, C, E, A>,
) -> Result<QwApplicationServerHost<S, G, C, E, A>, QwHostError>
where
    S: QwHostSimulation,
    G: QwHostGame,
    C: QwHostContent,
    E: QwHostSession,
{
    let mut game = options
        .game
        .filter(|game| game.is_quakeworld())
        .ok_or(QwHostError::MissingGame)?;
    if !options.content.is_q1_bsp_world() {
        return Err(QwHostError::MissingGame);
    }
    let models = game.precache_names(QwPrecacheKind::Model);
    let sounds = game.precache_names(QwPrecacheKind::Sound);
    if models.len() >= 256 || sounds.len() >= 256 {
        return Err(QwHostError::PrecacheOverflow);
    }
    let geometry_path = options.content.recipe_geometry_path();
    let geometry = options
        .content
        .map_geometry_bytes(&geometry_path)
        .map_err(QwHostError::Content)?;
    let checksum = quake_world_map_checksum2(&geometry)?;
    for name in [
        "allow_download",
        "allow_download_skins",
        "allow_download_models",
        "allow_download_sounds",
        "allow_download_maps",
    ] {
        if game.cvars().get(name).is_none() {
            game.cvars_mut().register(name, "1", 0)?;
        }
    }
    let model_index: HashMap<String, u32> = models
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index as u32 + 1))
        .collect();
    let styles: [String; 64] = std::array::from_fn(|index| options.simulation.light_style(index as u32));
    let mut host = QwApplicationServerHost {
        session: options.session,
        simulation: options.simulation,
        game,
        content: options.content,
        server_count: options.server_count,
        administration: options.administration,
        masters: options.masters,
        print: options.print,
        models,
        sounds,
        model_index,
        checksum,
        clients: HashMap::new(),
        baselines: Vec::new(),
        signon: Vec::new(),
        queued: Vec::new(),
        routed: Vec::new(),
        styles,
        previous_pause: false,
        player_model: 0,
    };
    host.previous_pause = host.simulation.q1_paused();
    host.player_model = host.model_index.get("progs/player.mdl").copied().unwrap_or(0);
    let mut baselines: Vec<Q1WireEntity> = Vec::new();
    for actor in host.source_actors()? {
        let mut entity = host.state(&actor)?;
        entity.effects = 0;
        baselines.push(entity.to_wire());
    }
    for index in 1..=32u16 {
        baselines.push(
            QwHostEntity {
                number: index,
                model_index: u8::try_from(host.player_model).map_err(|_| QwHostError::WireRange("player model"))?,
                frame: 0,
                color_map: index as u8,
                skin: 0,
                effects: 0,
                origin: Vec3::default(),
                angles: Vec3::default(),
            }
            .to_wire(),
        );
    }
    host.baselines = baselines;
    let mut persistent: Vec<QuakeWorldMessage> = if host.content.has_prepared_quakec() {
        host.game
            .signon_messages()
            .into_iter()
            .filter(|message| {
                !matches!(
                    message,
                    QuakeWorldMessage::PacketEntities { .. } | QuakeWorldMessage::InvalidDelta { .. }
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    for event in host.simulation.q1_persistent_events() {
        match event {
            Q1Event::Ambient {
                origin,
                path,
                volume,
                attenuation,
            } => {
                let index = host
                    .sounds
                    .iter()
                    .position(|sound| sound == &path)
                    .map(|slot| slot + 1)
                    .unwrap_or(0);
                persistent.push(QuakeWorldMessage::StaticSound {
                    index: u16::try_from(index).map_err(|_| QwHostError::WireRange("static sound"))?,
                    origin: [f64::from(origin.x), f64::from(origin.y), f64::from(origin.z)],
                    volume: (volume * 255.0).trunc() as u8,
                    attenuation,
                });
            }
            Q1Event::StaticModel {
                path,
                frame,
                color_map,
                skin,
                origin,
                angles,
                ..
            } => {
                persistent.push(QuakeWorldMessage::Static {
                    state: QwHostEntity {
                        number: 0,
                        model_index: u8::try_from(host.model_index.get(&path).copied().unwrap_or(0))
                            .map_err(|_| QwHostError::WireRange("static model"))?,
                        frame: frame as u8,
                        color_map: color_map as u8,
                        skin: skin as u8,
                        effects: 0,
                        origin,
                        angles,
                    }
                    .to_wire(),
                });
            }
            _ => {}
        }
    }
    let mut records: Vec<QuakeWorldMessage> = host
        .baselines
        .iter()
        .map(|state| QuakeWorldMessage::Baseline { state: state.clone() })
        .collect();
    records.extend(persistent);
    host.signon = host.encode(&records)?;
    Ok(host)
}

/// Serialize client info (donor `infoText`).
fn info_text(info: &BTreeMap<String, String>) -> String {
    info.iter()
        .map(|(key, value)| format!("\\{key}\\{value}"))
        .collect::<String>()
}

/// Whether a destination carries reliable delivery.
fn destination_reliable(destination: &QcMessageDestination) -> bool {
    match destination {
        QcMessageDestination::Broadcast { reliable } | QcMessageDestination::Multicast { reliable, .. } => *reliable,
        QcMessageDestination::Client { .. } => true,
        QcMessageDestination::Signon => false,
    }
}

impl<S, G, C, E, A> QwApplicationServerHost<S, G, C, E, A>
where
    S: QwHostSimulation,
    G: QwHostGame,
    C: QwHostContent,
    E: QwHostSession,
{
    /// Resolve a source field offset (donor `field`).
    fn field(&self, name: &str) -> Result<i32, QwHostError> {
        self.game
            .field_offset(name)
            .ok_or_else(|| QwHostError::MissingField(name.to_string()))
    }

    /// Resolve an actor to its live source slot (donor `slot`).
    fn slot(&self, actor: &ActorId) -> Result<u32, QwHostError> {
        self.game.source_slot(actor).ok_or(QwHostError::MissingSlot)
    }

    /// Read a source scalar field (donor `scalar`).
    fn scalar(&self, actor: &ActorId, name: &str) -> Result<f64, QwHostError> {
        Ok(self.game.entity_float(self.slot(actor)?, self.field(name)?))
    }

    /// Read a source vector field (donor `vector`).
    fn vector(&self, actor: &ActorId, name: &str) -> Result<Vec3, QwHostError> {
        Ok(self.game.entity_vector(self.slot(actor)?, self.field(name)?))
    }

    /// Read a source string field (donor `text`).
    fn text(&self, actor: &ActorId, name: &str) -> Result<String, QwHostError> {
        let slot = self.slot(actor)?;
        let field = self.field(name)?;
        Ok(self.game.lookup_string(self.game.entity_int(slot, field)))
    }

    /// Read a source global (donor `global`).
    fn global(&self, name: &str) -> f64 {
        self.game.global_float(name)
    }

    /// Read an entity snapshot (donor `state`).
    fn state(&self, actor: &ActorId) -> Result<QwHostEntity, QwHostError> {
        Ok(QwHostEntity {
            number: u16::try_from(self.slot(actor)?).map_err(|_| QwHostError::WireRange("entity number"))?,
            model_index: self.scalar(actor, "modelindex")?.trunc() as u8,
            frame: self.scalar(actor, "frame")?.trunc() as u8,
            color_map: self.scalar(actor, "colormap")?.trunc() as u8,
            skin: self.scalar(actor, "skin")?.trunc() as u8,
            effects: self.scalar(actor, "effects")?.trunc() as u8,
            origin: self.vector(actor, "origin")?,
            angles: self.vector(actor, "angles")?,
        })
    }

    /// Entities owned by the map provider with a visible model (donor
    /// `sourceActors`).
    fn source_actors(&self) -> Result<Vec<ActorId>, QwHostError> {
        let provider = self.simulation.recipe_entities_provider();
        let mut actors = Vec::new();
        for actor in self.simulation.owned_actors(&provider) {
            if self.slot(&actor)? > 32
                && self.scalar(&actor, "modelindex")? != 0.0
                && !self.text(&actor, "model")?.is_empty()
            {
                actors.push(actor);
            }
        }
        Ok(actors)
    }

    /// Encode messages into 1400-byte buffers (donor `encode`).
    fn encode(&self, records: &[QuakeWorldMessage]) -> Result<Vec<Vec<u8>>, QwHostError> {
        let mut result = Vec::new();
        let mut buffer = MsgWriter::new(1400, false);
        for message in records {
            let mut one = MsgWriter::new(1400, false);
            write_quake_world_message(&mut one, QwProfile::Quakeworld, message)?;
            if buffer.cursize() + one.cursize() > 1400 {
                result.push(buffer.bytes().to_vec());
                buffer = MsgWriter::new(1400, false);
            }
            buffer.write_bytes(one.bytes())?;
        }
        if buffer.cursize() != 0 {
            result.push(buffer.bytes().to_vec());
        }
        Ok(result)
    }

    /// Require a live client handle (donor `requireClient`).
    fn require_client(&self, player: &QwApplicationPlayer) -> Result<&QwClientState, QwHostError> {
        match self.clients.get(&player.slot) {
            Some(client) if client.player.client == player.client => Ok(client),
            _ => Err(QwHostError::StaleClient),
        }
    }

    /// Eye position for an actor (donor `eye`).
    fn eye(&self, actor: &ActorId) -> Result<Vec3, QwHostError> {
        let origin = self.vector(actor, "origin")?;
        let offset = self.vector(actor, "view_ofs")?;
        Ok(Vec3 {
            x: origin.x + offset.x,
            y: origin.y + offset.y,
            z: origin.z + offset.z,
        })
    }

    /// Whether a viewer sees a target (donor `visible`).
    fn visible(&self, viewer: &ActorId, target: &ActorId) -> Result<bool, QwHostError> {
        if viewer == target {
            return Ok(true);
        }
        let origin = self.eye(viewer)?;
        let linked = match self.simulation.linked_bounds(target) {
            Some(linked) => linked,
            None => return Ok(false),
        };
        let scene = self.simulation.scene();
        let from = scene.box_leaves(
            &Vec3 {
                x: origin.x - 8.0,
                y: origin.y - 8.0,
                z: origin.z - 8.0,
            },
            &Vec3 {
                x: origin.x + 8.0,
                y: origin.y + 8.0,
                z: origin.z + 8.0,
            },
            self.content.world_leaf_count(),
        );
        // SV_FindTouchedLeafs uses the source link envelope and retains at most MAX_ENT_LEAFS (16).
        let to = scene.box_leaves(&linked.min, &linked.max, 16);
        Ok(from.iter().any(|first| {
            to.iter().any(|second| {
                scene.cluster_visible(
                    scene.leaf_cluster(*first),
                    scene.leaf_cluster(*second),
                    QwVisibilityScope::Pvs,
                )
            })
        }))
    }

    /// Whether a player receives a routed message (donor `receives`).
    fn receives(&self, player: &QwApplicationPlayer, destination: &QcMessageDestination) -> Result<bool, QwHostError> {
        match destination {
            QcMessageDestination::Client { actor } => Ok(&player.actor == actor),
            QcMessageDestination::Signon => Ok(false),
            QcMessageDestination::Broadcast { .. } => Ok(true),
            QcMessageDestination::Multicast { .. } => {
                let origin = self.vector(&player.actor, "origin")?;
                let adapter = QwSceneAdapter(self.simulation.scene());
                Ok(receives_quake_world_message(
                    &player.actor,
                    destination,
                    origin,
                    &adapter,
                ))
            }
        }
    }
}

impl<S, G, C, E, A> QwApplicationServerHost<S, G, C, E, A>
where
    S: QwHostSimulation,
    G: QwHostGame,
    C: QwHostContent,
    E: QwHostSession,
{
    /// Build a player state for a frame (donor `playerState`).
    fn player_state(&self, client: &QwClientState, owner: &QwApplicationPlayer) -> Result<QwPlayerState, QwHostError> {
        let actor = &client.player.actor;
        let entity = self.state(actor)?;
        let velocity = self.vector(actor, "velocity")?;
        let mut flags = PF_MSEC | PF_COMMAND;
        if u32::from(entity.model_index) != self.player_model {
            flags |= PF_MODEL;
        }
        if velocity.x != 0.0 {
            flags |= PF_VELOCITY1;
        }
        if velocity.y != 0.0 {
            flags |= PF_VELOCITY2;
        }
        if velocity.z != 0.0 {
            flags |= PF_VELOCITY3;
        }
        if entity.effects != 0 {
            flags |= PF_EFFECTS;
        }
        if entity.skin != 0 {
            flags |= PF_SKINNUM;
        }
        let dead = self.scalar(actor, "health")? <= 0.0;
        if dead {
            flags |= PF_DEAD;
        }
        if self.vector(actor, "mins")?.z != -24.0 {
            flags |= PF_GIB;
        }
        let weapon_frame = self.scalar(actor, "weaponframe")?.trunc() as u8;
        if owner.client == client.player.client {
            flags &= !(PF_MSEC | PF_COMMAND);
            if weapon_frame != 0 {
                flags |= PF_WEAPONFRAME;
            }
        }
        let angles = if dead {
            Vec3 {
                x: 0.0,
                y: entity.angles.y,
                z: client.command.angles.z,
            }
        } else {
            client.command.angles
        };
        Ok(QwPlayerState {
            number: u8::try_from(client.player.slot).map_err(|_| QwHostError::WireRange("player slot"))?,
            flags,
            origin: [
                f64::from(entity.origin.x),
                f64::from(entity.origin.y),
                f64::from(entity.origin.z),
            ],
            velocity: [velocity.x as i16, velocity.y as i16, velocity.z as i16],
            model_index: u16::from(entity.model_index),
            frame: entity.frame,
            skin: entity.skin,
            effects: entity.effects,
            weapon_frame,
            milliseconds: (1000.0 * (self.game.time_seconds() - client.command_time))
                .trunc()
                .clamp(0.0, 255.0) as u8,
            command: qa_net::qw::QwUsercmd {
                msec: client.command.milliseconds,
                angles: [f64::from(angles.x), f64::from(angles.y), f64::from(angles.z)],
                forwardmove: client.command.forward_move,
                sidemove: client.command.side_move,
                upmove: client.command.up_move,
                buttons: 0,
                impulse: 0,
            },
        })
    }

    /// Player stat table (donor `stats`).
    fn stats(&self, actor: &ActorId) -> Result<Vec<(u32, f64)>, QwHostError> {
        let weapon_model = self
            .model_index
            .get(&self.text(actor, "weaponmodel")?)
            .copied()
            .unwrap_or(0);
        Ok(vec![
            (0, self.scalar(actor, "health")?),
            (2, f64::from(weapon_model)),
            (3, self.scalar(actor, "currentammo")?),
            (4, self.scalar(actor, "armorvalue")?),
            (5, self.scalar(actor, "weaponframe")?),
            (6, self.scalar(actor, "ammo_shells")?),
            (7, self.scalar(actor, "ammo_nails")?),
            (8, self.scalar(actor, "ammo_rockets")?),
            (9, self.scalar(actor, "ammo_cells")?),
            (10, self.scalar(actor, "weapon")?),
            (11, self.global("total_secrets")),
            (12, self.global("total_monsters")),
            (13, self.global("found_secrets")),
            (14, self.global("killed_monsters")),
            (
                15,
                f64::from(
                    (self.scalar(actor, "items")?.trunc() as i32) | ((self.global("serverflags").trunc() as i32) << 28),
                ),
            ),
        ])
    }

    /// Spawn-time messages for a player (donor `spawnMessages`).
    fn spawn_messages(&self, player: &QwApplicationPlayer, start: u32) -> Result<Vec<Vec<u8>>, QwHostError> {
        let mut records = vec![QuakeWorldMessage::Pause {
            paused: self.simulation.q1_paused(),
        }];
        let mut slots: Vec<u32> = self
            .clients
            .values()
            .filter(|client| client.player.slot >= start)
            .map(|client| client.player.slot)
            .collect();
        slots.sort_unstable();
        for slot in slots {
            let client = &self.clients[&slot];
            records.push(QuakeWorldMessage::Userinfo {
                slot: u8::try_from(slot).map_err(|_| QwHostError::WireRange("client slot"))?,
                user_id: client.player.client.generation() as i32 * 32 + slot as i32 + 1,
                value: info_text(&client.info),
            });
        }
        for (index, value) in self.styles.iter().enumerate() {
            records.push(QuakeWorldMessage::LightStyle {
                index: index as u8,
                value: value.clone(),
            });
        }
        for (index, raw) in self.stats(&player.actor)? {
            records.push(QuakeWorldMessage::Stat {
                index: u8::try_from(index).map_err(|_| QwHostError::WireRange("stat index"))?,
                value: raw.trunc() as i32,
            });
        }
        self.encode(&records)
    }

    /// Flush drained messages into the queue or the emitter (donor `flush`
    /// inside `commandPhase`).
    fn flush_messages(
        &mut self,
        emit: &mut dyn FnMut(QwApplicationPlayer, QuakeWorldMessage),
    ) -> Result<(), QwHostError> {
        self.game.flush_messages();
        let mut pending = std::mem::take(&mut self.queued);
        for batch in self.game.drain_messages() {
            for entry in batch.entries {
                if matches!(
                    entry,
                    QuakeWorldMessage::PacketEntities { .. } | QuakeWorldMessage::InvalidDelta { .. }
                ) {
                    continue;
                }
                pending.push(QwRoutedMessage {
                    message: entry,
                    destination: batch.destination.clone(),
                });
            }
        }
        for entry in pending {
            if matches!(entry.destination, QcMessageDestination::Signon) || !destination_reliable(&entry.destination) {
                self.queued.push(entry);
                continue;
            }
            let recipients: Vec<QwApplicationPlayer> = self
                .clients
                .values()
                .filter(|client| client.begun || matches!(entry.destination, QcMessageDestination::Client { .. }))
                .map(|client| client.player.clone())
                .collect();
            for player in recipients {
                if self.receives(&player, &entry.destination)? {
                    emit(player, entry.message.clone());
                }
            }
        }
        Ok(())
    }
}

impl<S, G, C, E, A> QwApplicationServerHost<S, G, C, E, A>
where
    S: QwHostSimulation,
    G: QwHostGame,
    C: QwHostContent,
    E: QwHostSession,
{
    /// Donor `administration`.
    #[must_use]
    pub fn administration(&self) -> Option<&A> {
        self.administration.as_ref()
    }

    /// Donor `masters`.
    #[must_use]
    pub fn masters(&self) -> Option<&QwMastersFn> {
        self.masters.as_ref()
    }

    /// Donor `print`.
    pub fn print(&self, text: &str) {
        (self.print)(text);
    }

    /// Donor `maxClients`.
    #[must_use]
    pub fn max_clients(&self) -> u32 {
        32
    }

    /// Donor `paused`.
    #[must_use]
    pub fn paused(&self) -> bool {
        self.simulation.q1_paused()
    }

    /// Donor `supportsSourceWire`.
    #[must_use]
    pub fn supports_source_wire(&self) -> QwWireSupport {
        QwWireSupport::Supported
    }

    /// Donor `authentication`.
    #[must_use]
    pub fn authentication(&self) -> QwAuthentication {
        QwAuthentication {
            password: self.game.cvars().variable_string("password"),
            spectator_password: self.game.cvars().variable_string("spectator_password"),
            high_characters: self.game.cvars().variable_value("sv_highchars") != 0.0,
        }
    }

    /// Donor `clientInfo`.
    pub fn client_info(&self, player: &QwApplicationPlayer) -> Result<&BTreeMap<String, String>, QwHostError> {
        Ok(&self.require_client(player)?.info)
    }

    /// Run an action bracketed by message flushes (donor `commandPhase`).
    pub fn command_phase(
        &mut self,
        player: &QwApplicationPlayer,
        action: &mut dyn FnMut(),
        emit: &mut dyn FnMut(QwApplicationPlayer, QuakeWorldMessage),
    ) -> Result<(), QwHostError> {
        if self.require_client(player).is_err() {
            return Ok(());
        }
        self.flush_messages(emit)?;
        self.simulation.queue_quakeworld_action(&player.client, action);
        self.flush_messages(emit)?;
        Ok(())
    }

    /// Admit a connecting client (donor `admit`).
    pub fn admit(&mut self, request: &QwAdmitRequest) -> Result<QwAdmission, QwHostError> {
        let role = if request.spectator {
            QwRole::Spectator
        } else {
            QwRole::Player
        };
        let limit = if request.spectator {
            self.game
                .cvars()
                .variable_value("maxspectators")
                .trunc()
                .clamp(0.0, 32.0) as u32
        } else {
            self.simulation.max_clients()
        };
        if self.clients.values().filter(|client| client.role == role).count() as u32 >= limit {
            return Ok(QwAdmission::Rejected {
                reason: "Server is full".to_string(),
            });
        }
        let mut index = 0;
        while self.clients.contains_key(&index) && index < 32 {
            index += 1;
        }
        if index == 32 {
            return Ok(QwAdmission::Rejected {
                reason: "Server is full".to_string(),
            });
        }
        let client = self.session.create_client(index)?;
        self.session.connect_remote_client(&client);
        let admitted = self.admit_inner(&client, index, role, request);
        if admitted.is_err() {
            self.session.close_client(&client);
        }
        admitted
    }

    /// Admit body cleaned up by `admit` on failure (donor `admit` try block).
    fn admit_inner(
        &mut self,
        client: &ClientId,
        index: u32,
        role: QwRole,
        request: &QwAdmitRequest,
    ) -> Result<QwAdmission, QwHostError> {
        let mut info = quake_world_info(&request.userinfo);
        info.remove("*spectator");
        if role == QwRole::Spectator {
            info.insert("*spectator".to_string(), "1".to_string());
        }
        self.game.set_client_role(client, role);
        self.game.set_client_info(client, &info);
        let actor = self.game.reserved_client(client).map_err(QwHostError::GameCallback)?;
        let player = QwApplicationPlayer {
            client: client.clone(),
            actor,
            slot: index,
        };
        let user_id = client.generation() as i32 * 32 + index as i32 + 1;
        self.clients.insert(
            index,
            QwClientState {
                player: player.clone(),
                role,
                info: info.clone(),
                begun: false,
                command: QwUserCommand::default(),
                command_time: self.game.time_seconds(),
                stats: HashMap::new(),
                frags: 0.0,
            },
        );
        self.queued.push(QwRoutedMessage {
            message: QuakeWorldMessage::Userinfo {
                slot: u8::try_from(index).map_err(|_| QwHostError::WireRange("client slot"))?,
                user_id,
                value: info_text(&info),
            },
            destination: QcMessageDestination::Broadcast { reliable: true },
        });
        Ok(QwAdmission::Accepted { player })
    }

    /// Bind a recording client (donor `recordingPlayer`).
    pub fn recording_player(&mut self, client: &ClientId) -> Result<QwApplicationPlayer, QwHostError> {
        let actor = self
            .simulation
            .players()
            .into_iter()
            .find(|actor| self.simulation.movement_client(actor).as_ref() == Some(client))
            .ok_or(QwHostError::RecordingPlayer)?;
        if !self.game.is_active_client(&actor) {
            return Err(QwHostError::RecordingPlayer);
        }
        if let Some(existing) = self.clients.get(&client.slot()) {
            if existing.player.client != *client || existing.player.actor != actor {
                return Err(QwHostError::RecordingSlot);
            }
            return Ok(existing.player.clone());
        }
        let player = QwApplicationPlayer {
            client: client.clone(),
            actor: actor.clone(),
            slot: client.slot(),
        };
        let role = if self.game.is_spectator_client(&actor) {
            QwRole::Spectator
        } else {
            QwRole::Player
        };
        let info = self.game.client_info(client);
        let command_time = self.game.time_seconds();
        let frags = self.scalar(&actor, "frags")?;
        self.clients.insert(
            client.slot(),
            QwClientState {
                player: player.clone(),
                role,
                info,
                begun: true,
                command: QwUserCommand::default(),
                command_time,
                stats: HashMap::new(),
                frags,
            },
        );
        Ok(player)
    }

    /// Recording signon buffers (donor `recordingSignon`).
    pub fn recording_signon(&self, player: &QwApplicationPlayer) -> Result<Vec<Vec<u8>>, QwHostError> {
        self.require_client(player)?;
        self.spawn_messages(player, 0)
    }

    /// Rebind a carried client (donor `carriedPlayer`).
    pub fn carried_player(&mut self, client: &ClientId) -> Result<QwApplicationPlayer, QwHostError> {
        let actor = self.game.reserved_client(client).map_err(QwHostError::GameCallback)?;
        let player = QwApplicationPlayer {
            client: client.clone(),
            actor: actor.clone(),
            slot: client.slot(),
        };
        let info: BTreeMap<String, String> = self
            .simulation
            .travel_clients()
            .and_then(|carry| {
                carry
                    .into_iter()
                    .find(|entry| entry.client == *client)
                    .map(|entry| entry.user_info)
            })
            .unwrap_or_default()
            .into_iter()
            .collect();
        let role = if self.game.is_spectator_client(&actor) {
            QwRole::Spectator
        } else {
            QwRole::Player
        };
        let command_time = self.game.time_seconds();
        self.clients.insert(
            client.slot(),
            QwClientState {
                player: player.clone(),
                role,
                info,
                begun: false,
                command: QwUserCommand::default(),
                command_time,
                stats: HashMap::new(),
                frags: 0.0,
            },
        );
        Ok(player)
    }

    /// Disconnect a player (donor `disconnect`).
    pub fn disconnect(&mut self, player: &QwApplicationPlayer, reason: &str) {
        match self.simulation.resolve_owned_actor(&player.actor) {
            Some(actor) if !self.game.is_active_client(&actor) => {
                self.game.disconnect_client(&actor);
            }
            _ => self.simulation.disconnect_player(&player.actor),
        }
        self.session.close_client(&player.client);
        self.clients.remove(&player.slot);
        self.queued.push(QwRoutedMessage {
            message: QuakeWorldMessage::Userinfo {
                slot: player.slot.min(255) as u8,
                user_id: 0,
                value: String::new(),
            },
            destination: QcMessageDestination::Broadcast { reliable: true },
        });
        self.print(reason);
    }

    /// Donor `baselines`.
    #[must_use]
    pub fn baselines(&self) -> Vec<Q1WireEntity> {
        self.baselines.clone()
    }
}

impl<S, G, C, E, A> QwApplicationServerHost<S, G, C, E, A>
where
    S: QwHostSimulation,
    G: QwHostGame,
    C: QwHostContent,
    E: QwHostSession,
{
    /// Prepare a client download (donor `prepareDownload`).
    pub fn prepare_download(&self, _player: &QwApplicationPlayer, requested_path: &str) -> Option<QwPreparedDownload> {
        let path = requested_path.to_lowercase();
        if path.contains("..")
            || path.starts_with('.')
            || !path.contains('/')
            || self.game.cvars().variable_value("allow_download") == 0.0
        {
            return None;
        }
        if download_path(&path).is_err() {
            return None;
        }
        for (prefix, setting) in [
            ("skins/", "skins"),
            ("progs/", "models"),
            ("sound/", "sounds"),
            ("maps/", "maps"),
        ] {
            if path.starts_with(prefix) && self.game.cvars().variable_value(&format!("allow_download_{setting}")) == 0.0
            {
                return None;
            }
        }
        let asset = self.content.open_download(&path)?;
        if asset.bytes.len() > 0x7fff_ffff
            || !asset.downloadable
            || (path.starts_with("maps/") && asset.archive_provenance)
        {
            return None;
        }
        Some(QwPreparedDownload {
            byte_length: asset.bytes.len(),
            data: Rc::new(RefCell::new(Some(asset.bytes))),
        })
    }

    /// Signon server data (donor `signon.serverData`).
    pub fn signon_server_data(&self, player: &QwApplicationPlayer) -> Result<QuakeWorldMessage, QwHostError> {
        let spectator = self.require_client(player)?.role == QwRole::Spectator;
        let level = self.game.lookup_string(self.game.entity_int(0, self.field("message")?));
        let cvars = self.game.cvars();
        Ok(QuakeWorldMessage::ServerData {
            protocol: QwProfile::Quakeworld,
            server_count: self.server_count,
            game_directory: self.content.game_directory(),
            player_slot: u8::try_from(player.slot).map_err(|_| QwHostError::WireRange("player slot"))?,
            spectator,
            level,
            move_variables: QwMoveVariables {
                gravity: cvars.variable_value("sv_gravity"),
                stop_speed: cvars.variable_value("sv_stopspeed"),
                max_speed: cvars.variable_value("sv_maxspeed"),
                spectator_max_speed: cvars.variable_value("sv_spectatormaxspeed"),
                accelerate: cvars.variable_value("sv_accelerate"),
                air_accelerate: cvars.variable_value("sv_airaccelerate"),
                water_accelerate: cvars.variable_value("sv_wateraccelerate"),
                friction: cvars.variable_value("sv_friction"),
                water_friction: cvars.variable_value("sv_waterfriction"),
                entity_gravity: 1.0,
            },
        })
    }

    /// Signon model precaches (donor `signon.models`).
    #[must_use]
    pub fn signon_models(&self) -> Vec<String> {
        self.models.clone()
    }

    /// Signon sound precaches (donor `signon.sounds`).
    #[must_use]
    pub fn signon_sounds(&self) -> Vec<String> {
        self.sounds.clone()
    }

    /// Signon buffers (donor `signon.signonBuffers`).
    #[must_use]
    pub fn signon_buffers(&self) -> Vec<Vec<u8>> {
        self.signon.clone()
    }

    /// Whether a map checksum matches (donor `signon.acceptsMapChecksum`).
    #[must_use]
    pub fn accepts_map_checksum(&self, value: u32) -> bool {
        value == self.checksum
    }

    /// Spawn a player (donor `signon.spawn`).
    pub fn spawn(&mut self, player: &QwApplicationPlayer, start: u32) -> Result<Vec<Vec<u8>>, QwHostError> {
        self.game.prepare_client_spawn(&player.client);
        self.spawn_messages(player, start)
    }

    /// Begin a player (donor `signon.begin`).
    pub fn begin(&mut self, player: &QwApplicationPlayer) -> Result<(), QwHostError> {
        self.require_client(player)?;
        let admitted = self
            .simulation
            .admit_player(&player.client)
            .map_err(QwHostError::GameCallback)?;
        if admitted != player.actor {
            return Err(QwHostError::BeginChangedActor);
        }
        if let Some(client) = self.clients.get_mut(&player.slot) {
            client.begun = true;
        }
        Ok(())
    }

    /// Signon disconnect sink (donor `signon.disconnect`).
    pub fn signon_disconnect(&self, reason: &str) {
        self.print(reason);
    }

    /// Signon download opener, always empty (donor `signon.openDownload`).
    #[must_use]
    pub fn open_download(&self) -> Option<QwPreparedDownload> {
        None
    }

    /// Queue a client command group (donor `commandGroup`).
    pub fn command_group(
        &mut self,
        player: &QwApplicationPlayer,
        commands: &[QwUserCommand],
        sequence: u64,
    ) -> Result<(), QwHostError> {
        self.require_client(player)?;
        self.simulation
            .queue_quakeworld_commands(&player.client, commands, sequence);
        if let Some(client) = self.clients.get_mut(&player.slot) {
            client.command = commands.last().copied().unwrap_or_default();
            client.command_time = self.game.time_seconds();
        }
        Ok(())
    }

    /// Run a client command (donor `command`).
    pub fn command(&mut self, player: &QwApplicationPlayer, name: &str, args: &[String]) -> Result<(), QwHostError> {
        if name == "pause" {
            let previous = self.simulation.q1_paused();
            let text = self.simulation.toggle_q1_pause(&player.actor);
            let destination = if previous == self.simulation.q1_paused() {
                QcMessageDestination::Client {
                    actor: player.actor.clone(),
                }
            } else {
                QcMessageDestination::Broadcast { reliable: true }
            };
            self.queued.push(QwRoutedMessage {
                message: QuakeWorldMessage::Print { level: 2, text },
                destination,
            });
        } else if name == "kill" {
            if !self.game.client_kill(&player.actor) {
                self.queued.push(QwRoutedMessage {
                    message: QuakeWorldMessage::Print {
                        level: 2,
                        text: "Can't suicide -- allready dead!\n".to_string(),
                    },
                    destination: QcMessageDestination::Client {
                        actor: player.actor.clone(),
                    },
                });
            }
        } else if name == "setinfo" && args.len() == 2 {
            let (Some(key), Some(value)) = (args.first(), args.get(1)) else {
                return Ok(());
            };
            if key.starts_with('*')
                || key
                    .chars()
                    .chain(value.chars())
                    .any(|char| char == '\\' || char == '"' || char == '\n' || char == '\r')
            {
                return Ok(());
            }
            let client = self
                .clients
                .get_mut(&player.slot)
                .filter(|client| client.player.client == player.client)
                .ok_or(QwHostError::StaleClient)?;
            if value.is_empty() {
                client.info.remove(key);
            } else {
                client.info.insert(key.clone(), value.clone());
            }
            let info = client.info.clone();
            self.game.set_client_info(&player.client, &info);
            self.simulation.notify_client_event("userinfo", &player.actor);
            self.queued.push(QwRoutedMessage {
                message: QuakeWorldMessage::SetInfo {
                    slot: u8::try_from(player.slot).map_err(|_| QwHostError::WireRange("client slot"))?,
                    key: key.clone(),
                    value: value.clone(),
                },
                destination: QcMessageDestination::Broadcast { reliable: true },
            });
        } else {
            self.print(&format!("Unhandled QW client command: {name}"));
        }
        Ok(())
    }

    /// Route drained and derived messages (donor `observe`).
    pub fn observe(
        &mut self,
        _output: &SimulationOutput,
        events: &[SimulationPresentationEvent],
    ) -> Result<(), QwHostError> {
        self.routed = std::mem::take(&mut self.queued);
        if self.previous_pause != self.simulation.q1_paused() {
            self.previous_pause = self.simulation.q1_paused();
            self.routed.push(QwRoutedMessage {
                message: QuakeWorldMessage::Pause {
                    paused: self.previous_pause,
                },
                destination: QcMessageDestination::Broadcast { reliable: true },
            });
        }
        for batch in self.game.drain_messages() {
            for entry in batch.entries {
                if matches!(
                    entry,
                    QuakeWorldMessage::PacketEntities { .. } | QuakeWorldMessage::InvalidDelta { .. }
                ) {
                    continue;
                }
                self.routed.push(QwRoutedMessage {
                    message: entry,
                    destination: batch.destination.clone(),
                });
            }
        }
        for item in events {
            if let SourcePresentationEvent::ViewReset { actor, angles, .. } = &item.event {
                self.routed.push(QwRoutedMessage {
                    message: QuakeWorldMessage::SetAngle {
                        angles: [f64::from(angles.x), f64::from(angles.y), f64::from(angles.z)],
                    },
                    destination: QcMessageDestination::Client { actor: actor.clone() },
                });
            }
        }
        for index in 0..64u32 {
            let value = self.simulation.light_style(index);
            if value != self.styles[index as usize] {
                self.styles[index as usize] = value.clone();
                self.routed.push(QwRoutedMessage {
                    message: QuakeWorldMessage::LightStyle {
                        index: index as u8,
                        value,
                    },
                    destination: QcMessageDestination::Broadcast { reliable: true },
                });
            }
        }
        let begun: Vec<ActorId> = self
            .clients
            .values()
            .filter(|client| client.begun)
            .map(|client| client.player.actor.clone())
            .collect();
        for actor in begun {
            let frags = self.scalar(&actor, "frags")?;
            let slot = self
                .clients
                .values()
                .find(|client| client.player.actor == actor)
                .map(|client| client.player.slot);
            if let Some(slot) = slot {
                let changed = self.clients.get(&slot).is_some_and(|client| frags != client.frags);
                if changed {
                    if let Some(client) = self.clients.get_mut(&slot) {
                        client.frags = frags;
                    }
                    self.routed.push(QwRoutedMessage {
                        message: QuakeWorldMessage::SlotStat {
                            kind: QwSlotStat::Frags,
                            slot: u8::try_from(slot).map_err(|_| QwHostError::WireRange("client slot"))?,
                            value: QwSlotValue::Integer(frags.trunc() as i32),
                        },
                        destination: QcMessageDestination::Broadcast { reliable: true },
                    });
                }
            }
        }
        Ok(())
    }

    /// Build a frame for a player (donor `frame`).
    pub fn frame(&mut self, player: &QwApplicationPlayer) -> Result<QwFrame, QwHostError> {
        self.require_client(player)?;
        let mut messages = Vec::new();
        let mut reliable = Vec::new();
        for entry in self.routed.clone() {
            if self.receives(player, &entry.destination)? {
                if !matches!(entry.destination, QcMessageDestination::Signon)
                    && destination_reliable(&entry.destination)
                {
                    reliable.push(entry.message);
                } else {
                    messages.push(entry.message);
                }
            }
        }
        for (index, raw) in self.stats(&player.actor)? {
            let value = raw.trunc() as i32;
            let changed = self
                .clients
                .get(&player.slot)
                .and_then(|live| live.stats.get(&index).copied())
                != Some(value);
            if changed {
                if let Some(live) = self.clients.get_mut(&player.slot) {
                    live.stats.insert(index, value);
                }
                reliable.push(QuakeWorldMessage::Stat {
                    index: index as u8,
                    value,
                });
            }
        }
        let viewers: Vec<QwClientState> = self.clients.values().cloned().collect();
        for other in &viewers {
            if other.begun
                && (other.role == QwRole::Player || other.player.actor == player.actor)
                && self.visible(&player.actor, &other.player.actor)?
            {
                messages.push(QuakeWorldMessage::Player {
                    state: self.player_state(other, player)?,
                });
            }
        }
        let mut entities = Vec::new();
        let mut nails = Vec::new();
        let spike = self.model_index.get("progs/spike.mdl").copied().unwrap_or(u32::MAX);
        let super_spike = self.model_index.get("progs/s_spike.mdl").copied().unwrap_or(u32::MAX);
        for actor in self.source_actors()? {
            if !self.visible(&player.actor, &actor)? {
                continue;
            }
            let entity = self.state(&actor)?;
            if u32::from(entity.model_index) == spike || u32::from(entity.model_index) == super_spike {
                if nails.len() < 32 {
                    nails.push(QwProjectile {
                        origin: [entity.origin.x as i32, entity.origin.y as i32, entity.origin.z as i32],
                        pitch: entity.angles.x as i32,
                        yaw: entity.angles.y as i32,
                    });
                }
            } else if entities.len() < 64 {
                entities.push(entity.to_qw());
            }
        }
        if !nails.is_empty() {
            messages.push(QuakeWorldMessage::Nails { projectiles: nails });
        }
        Ok(QwFrame {
            messages,
            reliable,
            entities,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_text_serializes_pairs() {
        let info = BTreeMap::from([
            ("name".to_string(), "a".to_string()),
            ("ip".to_string(), "loopback:0".to_string()),
        ]);
        assert_eq!(info_text(&info), "\\ip\\loopback:0\\name\\a");
    }

    #[test]
    fn destinations_report_reliability() {
        assert!(destination_reliable(&QcMessageDestination::Broadcast {
            reliable: true
        }));
        assert!(!destination_reliable(&QcMessageDestination::Broadcast {
            reliable: false
        }));
        assert!(!destination_reliable(&QcMessageDestination::Signon));
    }

    #[test]
    fn host_entity_encodes_baseline() {
        let entity = QwHostEntity {
            number: 7,
            model_index: 3,
            frame: 1,
            color_map: 2,
            skin: 0,
            effects: 0,
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            angles: Vec3::default(),
        };
        let wire = entity.to_wire();
        assert_eq!(wire.number, 7);
        assert_eq!(wire.state.modelindex, 3);
        assert_eq!(wire.state.origin, [1.0, 2.0, 3.0]);
        assert!(!wire.step);
        let qw = entity.to_qw();
        assert_eq!(qw.number, 7);
        assert_eq!(qw.modelindex, 3);
    }

    #[test]
    fn download_read_clamps_and_close_fails() {
        let prepared = QwPreparedDownload {
            byte_length: 4,
            data: Rc::new(RefCell::new(Some(vec![1, 2, 3, 4]))),
        };
        assert_eq!(prepared.read(1, 2).expect("range reads"), vec![2, 3]);
        assert_eq!(prepared.read(2, 99).expect("clamped reads"), vec![3, 4]);
        prepared.close();
        assert!(matches!(prepared.read(0, 1), Err(QwHostError::DownloadClosed)));
    }

    #[test]
    fn idle_command_is_zero() {
        let idle = QwUserCommand::default();
        assert_eq!(idle.milliseconds, 0);
        assert_eq!(idle.buttons, 0);
        assert_eq!(idle.impulse, 0);
    }
}
