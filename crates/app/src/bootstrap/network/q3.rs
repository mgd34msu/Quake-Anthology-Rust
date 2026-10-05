//! Quake III server network.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q3.ts`
//! (`Q3ServerNetwork`). The donor `async` flow collapses to synchronous
//! calls: the operation guard is vacuous without interleaving, and the
//! world/cross-check asserts hold by construction. Game callbacks wrapped by
//! the donor `q3GameCallback` (admit, disconnect, userinfo, command, input,
//! begin, round reconnect) abort the poll like the donor rethrow; every other
//! failure prints and continues like the donor catch.
//!
//! Mid-datagram client drops cannot remove the peer while the library frame
//! runs, so the drop bindings mark the connection zombie (exactly like the
//! donor `queueDisconnect`) and queue the removal for a post-datagram drain;
//! the poll aborts before the next packet when a queued drop carries a game
//! failure, matching the donor. Presentation payloads (`drop-client`,
//! `server-command`, `configstring`) ride [`Q3SourcePresentationEvent`]:
//! [`NetworkPresentationEvent`] has no payload carrier, so the publish path
//! filters `q3-source` carriers until the simulation producer port grows one.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use qa_core::cmd::{tokenize_command, Dialect, TextMode};
use qa_core::identity::{ActorId, ClientId};
use qa_core::numeric::native_atoi;
use qa_net::common::commands::ActorCommand;
use qa_net::common::endpoint::NetworkAddress;
use qa_net::common::session::{WireAdmission, WireSelection};
use qa_net::common::transport::{DatagramTransport, ReceiveEvent, TransportError};
use qa_net::protocol::ProtocolIdentity;
use qa_net::q3::{Q3MsgWriter, WireUserCommand};
use qa_net::q3_authorization::{resolve_q3_authority, Q3ServerAuthorization, Q3ServerAuthorizationBindings};
use qa_net::q3_net::{
    check_q3_download_name, decode_connectionless, encode_connectionless_text, q3_configstring_commands,
    q3_is_lan_address, route_q3_sequenced_packet, ChannelDelivery, ChannelResult, ConnectionlessPacket,
    ConnectionlessReceiver, GamestateEntry, Q3AcceptedConnect, Q3AdmissionSlot, Q3Challenge, Q3ConnectionIdentity,
    Q3DownloadRate, Q3DownloadServerBindings, Q3EntityState, Q3NetError, Q3ServerAdmission, Q3ServerAdmissionBindings,
    Q3ServerBindings, Q3ServerConnection, Q3ServerDownload, Q3ServerPhase, Q3ServerSnapshotHistory, Q3SlotPhase,
    Q3SnapshotEntities, ReliableCommand, ServerCommandAppend,
};
use qa_net::services::admin::{q3_rcon_command, AdminError, RconHost, RconProfile, RconService};
use qa_net::services::discovery::{DiscoveryError, MasterHeartbeat, PacketSender, Q3DiscoveryWire};
use qa_world::session::SimulationOutput;
use thiserror::Error;

use super::q3_types::{
    q3_game_callback, Q3ApplicationAdmission, Q3ApplicationAdmissionSurface, Q3ApplicationPlayer,
    Q3ApplicationServerHost, Q3GameCallbackError, Q3NetworkRoundRestart, Q3SnapshotServerBit,
};
use super::qw_server::js_int32;
use super::types::{
    ApplicationNetwork, ApplicationNetworkError, ApplicationNetworkPhase, ApplicationNetworkRole,
    NetworkPresentationEvent, Q3ConnectionCell,
};

/// Rcon output budget (donor literal `1008`).
const RCON_OUTPUT_BYTES: usize = 1008;
/// Master heartbeat interval (donor literal `300000`).
const HEARTBEAT_INTERVAL_MILLISECONDS: f64 = 300_000.0;
/// Default idle timeout (donor `timeoutMilliseconds ?? 30000`).
const DEFAULT_TIMEOUT_MILLISECONDS: u64 = 30_000;
/// Entity arena capacity (donor `new Q3SnapshotEntities(32768)`).
const SNAPSHOT_ENTITIES: usize = 32768;
/// `download` name limit (donor `slice(0, 63)`).
const DOWNLOAD_NAME_CHARS: usize = 63;

/// Server failure (`Q3ServerNetwork` rejections).
#[derive(Debug, Error)]
pub enum Q3ServerError {
    /// Endpoint failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Wrapped game-callback failure (donor `Q3GameCallbackError` rethrow).
    #[error(transparent)]
    GameCallback(#[from] Q3GameCallbackError),
    /// Quake III wire failure.
    #[error(transparent)]
    Net(#[from] Q3NetError),
    /// Transport failure.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// Rcon failure.
    #[error(transparent)]
    Admin(#[from] AdminError),
    /// Master heartbeat failure.
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
}

impl Q3ServerError {
    /// Convert into the endpoint error (donor rejection text intact).
    #[must_use]
    pub fn into_network(self) -> ApplicationNetworkError {
        ApplicationNetworkError::Message(self.to_string())
    }
}

/// Quake III server network options (donor `Q3ServerNetworkOptions`).
pub struct Q3ServerNetworkOptions<T, H> {
    /// Shared datagram transport.
    pub transport: T,
    /// Application host.
    pub host: H,
    /// Scripted random source (donor `() => number`).
    pub random: Box<dyn FnMut() -> f64>,
    /// Idle timeout override (donor `timeoutMilliseconds`).
    pub timeout_milliseconds: Option<u64>,
    /// Authorization resolver override (donor `resolveAuthorization`).
    pub resolve_authorization: Option<Box<dyn FnMut() -> Result<NetworkAddress, String>>>,
}

/// Next scripted random as the donor `<<` operand.
fn next_random(random: &Rc<RefCell<Box<dyn FnMut() -> f64>>>) -> i32 {
    js_int32((random.borrow_mut())())
}

/// Panic payload text.
fn panic_message(payload: Box<dyn std::any::Any + Send + 'static>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(ToString::to_string))
        .unwrap_or_else(|| "unknown panic".to_string())
}

/// Server generation (donor `serverId`, `restartedServerId`, `serverFlags`,
/// `checksumFeed`).
#[derive(Debug, Clone, Copy)]
struct Q3ServerEpoch {
    /// Current server id.
    server_id: i32,
    /// Restarted server id.
    restarted_server_id: i32,
    /// Snapshot server bit.
    server_flags: Q3SnapshotServerBit,
    /// Checksum feed.
    checksum_feed: i32,
}

/// Queued mid-datagram client removal (donor inline `disconnectClient`).
struct Q3DeferredDrop {
    /// Peer slot.
    slot: i32,
    /// Drop reason.
    reason: String,
}

/// Shared server state behind every binding (donor `this`).
struct Q3Shared<T, H> {
    /// Shared datagram transport.
    transport: Rc<T>,
    /// Current host (replaced in place by `changeWorld`).
    host: Rc<RefCell<H>>,
    /// Live peers by slot.
    peers: Rc<RefCell<Vec<Q3Peer<T, H>>>>,
    /// Server generation.
    epoch: Rc<RefCell<Q3ServerEpoch>>,
    /// Scripted random source.
    random: Rc<RefCell<Box<dyn FnMut() -> f64>>>,
    /// Retired flag (donor `ended`).
    ended: Rc<Cell<bool>>,
    /// Truncated poll time (donor `now`).
    now: Rc<Cell<i32>>,
    /// Queued actor commands (donor `pending`).
    pending: Rc<RefCell<Vec<ActorCommand>>>,
    /// Queued mid-datagram drops.
    deferred: Rc<RefCell<Vec<Q3DeferredDrop>>>,
    /// Game-callback failure awaiting the poll abort (donor rethrow).
    fatal: Rc<RefCell<Option<String>>>,
    /// Queued admission disconnects (donor inline `admission.disconnect`).
    pending_admission_drops: Rc<RefCell<Vec<NetworkAddress>>>,
}

impl<T, H> Clone for Q3Shared<T, H> {
    fn clone(&self) -> Self {
        Self {
            transport: self.transport.clone(),
            host: self.host.clone(),
            peers: self.peers.clone(),
            epoch: self.epoch.clone(),
            random: self.random.clone(),
            ended: self.ended.clone(),
            now: self.now.clone(),
            pending: self.pending.clone(),
            deferred: self.deferred.clone(),
            fatal: self.fatal.clone(),
            pending_admission_drops: self.pending_admission_drops.clone(),
        }
    }
}

/// Per-peer connection cell: baseline closure, snapshot history, and the
/// connection over its bindings. The history borrows the leaked baseline
/// closure and the connection borrows both, so `Drop` tears down in reverse
/// dependency order.
struct Q3PeerConn<T, H> {
    /// Leaked baseline lookup, reclaimed by `Drop`.
    baseline_raw: *mut dyn Fn(i32) -> Q3EntityState,
    /// Leaked snapshot history, reclaimed by `Drop`.
    history_raw: *mut Q3ServerSnapshotHistory<'static>,
    /// Connection over its bindings.
    cell: Q3ConnectionCell<Q3ConnBindings<T, H>, Q3ServerConnection<'static>>,
}

impl<T, H> Drop for Q3PeerConn<T, H> {
    fn drop(&mut self) {
        self.cell.clear();
        // SAFETY: the connection (the only history lease) just dropped, and
        // each pointer came from `Box::into_raw` exactly once.
        unsafe {
            drop(Box::from_raw(self.history_raw));
            drop(Box::from_raw(self.baseline_raw));
        }
    }
}

/// Per-peer download with owned bindings.
struct Q3PeerDownload<T, H> {
    /// Live download (`None` only in `Drop`).
    download: Option<Q3ServerDownload<'static>>,
    /// Leaked bindings, reclaimed by `Drop`.
    raw: *mut Q3DownloadBindings<T, H>,
}

impl<T, H> Drop for Q3PeerDownload<T, H> {
    fn drop(&mut self) {
        self.download = None;
        // SAFETY: the download (the only lease) just dropped and the pointer
        // came from `Box::into_raw` exactly once.
        unsafe {
            drop(Box::from_raw(self.raw));
        }
    }
}

/// Admitted peer (donor `Peer`).
struct Q3Peer<T, H> {
    /// Slot.
    slot: i32,
    /// Current remote address.
    remote: NetworkAddress,
    /// Bound player.
    player: Q3ApplicationPlayer,
    /// Connection cell.
    conn: Q3PeerConn<T, H>,
    /// Download state.
    download: Q3PeerDownload<T, H>,
    /// Gamestate baselines by entity number.
    baselines: Rc<RefCell<HashMap<i32, Q3EntityState>>>,
    /// Last accepted datagram time.
    last_received: i32,
    /// Admission time.
    connected_at: i32,
    /// Input sequence number.
    sequence: u32,
    /// Current userinfo.
    userinfo: String,
}

/// Channel delivery over the shared transport (donor `q3ChannelDelivery`).
struct Q3Delivery<'t, T: DatagramTransport<Address = NetworkAddress>> {
    /// Shared transport.
    transport: &'t T,
    /// Current remote.
    remote: NetworkAddress,
    /// First send failure, reported after the transmit.
    error: Option<TransportError>,
    /// Trace sink.
    trace: Box<dyn FnMut(&str) + 't>,
}

impl<T: DatagramTransport<Address = NetworkAddress>> ChannelDelivery for Q3Delivery<'_, T> {
    fn send(&mut self, datagram: &[u8]) {
        if self.error.is_some() {
            return;
        }
        if let Err(error) = self.transport.send(&self.remote, datagram) {
            self.error = Some(error);
        }
    }

    fn trace(&mut self, message: &str) {
        (self.trace)(message);
    }
}

/// Quake III source presentation payload (donor `Q3SourceEvent` network
/// subset: `drop-client`, `server-command`, `configstring`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3SourceEvent {
    /// Drop a client by source entity number.
    DropClient {
        /// Source entity number.
        client: i32,
        /// Drop reason.
        reason: String,
    },
    /// Queue a server command (`-1` broadcasts).
    ServerCommand {
        /// Source entity number or `-1`.
        client: i32,
        /// Command text.
        text: String,
    },
    /// Broadcast a configstring.
    Configstring {
        /// Configstring index.
        index: u32,
        /// Configstring value.
        value: String,
    },
}

/// Source presentation event with its recipient (donor
/// `SimulationPresentationEvent` with `kind: 'q3-source'`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3SourcePresentationEvent {
    /// Optional per-actor recipient.
    pub recipient: Option<ActorId>,
    /// Payload.
    pub event: Q3SourceEvent,
}

/// Map a shared error into the library error (donor throws surface through
/// the poll catch, which prints them).
fn into_lib(error: Q3ServerError) -> Q3NetError {
    match error {
        Q3ServerError::Net(error) => error,
        error => panic!("{error}"),
    }
}

impl<T, H> Q3Shared<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    /// Run a wrapped game callback (donor `q3GameCallback`): record the
    /// fatal flag and panic so the poll aborts like the donor rethrow.
    fn guard_game<R>(&self, run: impl FnOnce() -> R) -> R {
        match q3_game_callback(run) {
            Ok(value) => value,
            Err(error) => {
                self.fatal.borrow_mut().replace(error.to_string());
                panic!("{error}");
            }
        }
    }

    /// Print server text (donor `host.print`, raw like the donor).
    fn print(&self, text: &str) {
        self.host.borrow_mut().print(text);
    }

    /// Whether a peer holds the slot.
    fn has_peer(&self, slot: i32) -> bool {
        self.peers.borrow().iter().any(|peer| peer.slot == slot)
    }

    /// Queue a mid-datagram drop and mark the connection zombie like the
    /// donor `queueDisconnect`, so phase checks read the donor values.
    fn defer_drop(&self, slot: i32, reason: &str) {
        if let Some(peer) = self.peers.borrow_mut().iter_mut().find(|peer| peer.slot == slot) {
            let connection = peer.conn.cell.adapter_mut().connection;
            if !connection.is_null() {
                // SAFETY: the pointer is armed only during this peer's
                // datagram and the library never touches the connection
                // while a callback runs.
                unsafe {
                    (*connection).phase = Q3ServerPhase::Zombie;
                }
            }
        }
        self.deferred.borrow_mut().push(Q3DeferredDrop {
            slot,
            reason: reason.to_string(),
        });
    }
}

/// Authorization bindings (donor `Q3ServerAuthorization` options).
struct Q3AuthBindings<T, H> {
    /// Shared server state.
    shared: Q3Shared<T, H>,
    /// Resolver override.
    resolve: Option<Box<dyn FnMut() -> Result<NetworkAddress, String>>>,
}

impl<T, H> Q3ServerAuthorizationBindings for Q3AuthBindings<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    fn enabled(&mut self) -> bool {
        !self.shared.ended.get()
    }

    fn game_directory(&mut self) -> String {
        self.shared
            .host
            .borrow()
            .admission()
            .map(|admission| admission.game_directory())
            .unwrap_or_default()
    }

    fn strict_auth(&mut self) -> String {
        self.shared
            .host
            .borrow()
            .admission()
            .map(|admission| admission.strict_auth())
            .unwrap_or_else(|| "1".to_string())
    }

    fn resolve_authority(&mut self) -> Result<NetworkAddress, String> {
        if let Some(resolve) = self.resolve.as_mut() {
            resolve()
        } else {
            resolve_q3_authority().map_err(|error| error.to_string())
        }
    }

    fn send(&mut self, to: &NetworkAddress, packet: &[u8]) {
        let _ = self.shared.transport.send(to, packet);
    }

    fn print(&mut self, text: &str) {
        self.shared.print(text);
    }
}

/// Admission surface value (donor `host.admission?.<field> ?? <default>`).
fn admission_value<T, H, R>(
    shared: &Q3Shared<T, H>,
    default: R,
    select: impl FnOnce(&dyn Q3ApplicationAdmissionSurface) -> R,
) -> R
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    let host = shared.host.borrow();
    host.admission().map(select).unwrap_or(default)
}

/// Admission bindings (donor `Q3ServerAdmission` options).
struct Q3AdmissionBindings<T, H> {
    /// Shared server state.
    shared: Q3Shared<T, H>,
    /// Authorization server.
    authorization: Q3ServerAuthorization<'static>,
}

impl<T, H> Q3ServerAdmissionBindings for Q3AdmissionBindings<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    fn enabled(&self) -> bool {
        !self.shared.ended.get() && admission_value(&self.shared, true, |admission| admission.enabled())
    }

    fn slots(&self) -> Vec<Q3AdmissionSlot> {
        self.shared.slots()
    }

    fn private_clients(&self) -> i32 {
        admission_value(&self.shared, 0, |admission| admission.private_clients())
    }

    fn private_password(&self) -> String {
        admission_value(&self.shared, String::new(), |admission| admission.private_password())
    }

    fn reconnect_limit_seconds(&self) -> i32 {
        admission_value(&self.shared, 3, |admission| admission.reconnect_limit_seconds())
    }

    fn minimum_ping(&self) -> f32 {
        admission_value(&self.shared, 0.0, |admission| admission.minimum_ping())
    }

    fn maximum_ping(&self) -> f32 {
        admission_value(&self.shared, 0.0, |admission| admission.maximum_ping())
    }

    fn authorize_address(&self) -> Option<NetworkAddress> {
        self.authorization.address().cloned()
    }

    fn demo_restricted(&self) -> bool {
        admission_value(&self.shared, false, |admission| admission.demo_restricted())
    }

    fn is_lan(&self, address: &NetworkAddress) -> bool {
        q3_is_lan_address(address)
    }

    fn random(&mut self) -> i32 {
        next_random(&self.shared.random)
    }

    fn authorize(&mut self, challenge: &Q3Challenge) {
        let _ = self.authorization.request(challenge);
    }

    fn send(&mut self, address: &NetworkAddress, packet: &[u8]) {
        let _ = self.shared.transport.send(address, packet);
    }

    fn admit(&mut self, request: &Q3AcceptedConnect) -> Option<String> {
        if self.shared.has_peer(request.slot) {
            let client = self
                .shared
                .peers
                .borrow()
                .iter()
                .find(|peer| peer.slot == request.slot)
                .map(|peer| peer.player.client.clone());
            if let Some(client) = client {
                match self.shared.disconnect_client(&client, "reconnected") {
                    Ok(_) => {}
                    Err(Q3ServerError::GameCallback(error)) => {
                        self.shared.fatal.borrow_mut().replace(error.to_string());
                        panic!("{error}");
                    }
                    Err(error) => {
                        // The donor rejection prints and the attempt is lost;
                        // finish the eviction so a retry finds a free slot.
                        self.shared.remove_peer(request.slot);
                        self.shared.print(&error.to_string());
                    }
                }
            }
        }
        let admitted = self.shared.guard_game(|| self.shared.host.borrow_mut().admit(request));
        let player = match admitted {
            Q3ApplicationAdmission::Accepted { player } => player,
            Q3ApplicationAdmission::Rejected { reason } => return Some(reason),
        };
        let epoch = *self.shared.epoch.borrow();
        let baselines: HashMap<i32, Q3EntityState> = self
            .shared
            .host
            .borrow()
            .game_state(&player, epoch.server_id)
            .entries
            .iter()
            .filter_map(|entry| match entry {
                GamestateEntry::Baseline { number, entity } => Some((*number, entity.clone())),
                GamestateEntry::Configstring { .. } => None,
            })
            .collect();
        let entities = Q3SnapshotEntities::new(SNAPSHOT_ENTITIES).expect("32768 is a valid entity capacity");
        let product = self.shared.host.borrow().product();
        let baselines = Rc::new(RefCell::new(baselines));
        let lookup = baselines.clone();
        let baseline_raw =
            Box::into_raw(
                Box::new(move |number: i32| lookup.borrow().get(&number).cloned().unwrap_or_default())
                    as Box<dyn Fn(i32) -> Q3EntityState>,
            );
        // SAFETY: the baseline outlives the history (see `Q3PeerConn`).
        let history_raw = Box::into_raw(Box::new(Q3ServerSnapshotHistory::new(entities, product, unsafe {
            &*baseline_raw
        })));
        let download_raw = Box::into_raw(Box::new(Q3DownloadBindings {
            shared: self.shared.clone(),
            slot: request.slot,
        }));
        // SAFETY: the bindings are uniquely owned (reclaimed by the peer).
        let download = Q3ServerDownload::new(unsafe { &mut *download_raw });
        let mut cell = Q3ConnectionCell::new(Q3ConnBindings {
            shared: self.shared.clone(),
            slot: request.slot,
            connection: std::ptr::null_mut(),
        });
        let identity = Q3ConnectionIdentity {
            client: player.client.clone(),
            seat: None,
        };
        cell.build(|bindings| {
            // SAFETY: the cell owns the adapter box exclusively, drops any
            // previous connection before this lease, and drops this
            // connection before reclaiming the box, so the lease outlives
            // the connection (see `Q3ConnectionCell`); same for the
            // history (see `Q3PeerConn`).
            let leased: &'static mut Q3ConnBindings<T, H> = unsafe { &mut *(bindings as *mut Q3ConnBindings<T, H>) };
            Q3ServerConnection::new(
                identity,
                request.challenge,
                request.qport,
                unsafe { &mut *history_raw },
                leased,
            )
        });
        let now = self.shared.now.get();
        self.shared.peers.borrow_mut().push(Q3Peer {
            slot: request.slot,
            remote: request.address.clone(),
            player,
            conn: Q3PeerConn {
                baseline_raw,
                history_raw,
                cell,
            },
            download: Q3PeerDownload {
                download: Some(download),
                raw: download_raw,
            },
            baselines,
            last_received: now,
            connected_at: now,
            sequence: 0,
            userinfo: request.userinfo.clone(),
        });
        None
    }

    fn drop_bot(&mut self, _slot: i32) {
        panic!("Native Q3 admission cannot evict an application bot");
    }

    fn print(&mut self, text: &str) {
        self.shared.print(text);
    }

    fn query(&mut self, from: &NetworkAddress, packet: &ConnectionlessPacket) {
        if packet.command == "getinfo" || packet.command == "getstatus" {
            let challenge = packet.arguments.first().map(String::as_str).unwrap_or("");
            let detailed = packet.command == "getstatus";
            if let Some(response) = self.shared.host.borrow().status(challenge, detailed) {
                match encode_connectionless_text(&response) {
                    Ok(encoded) => {
                        let _ = self.shared.transport.send(from, &encoded);
                    }
                    Err(error) => self.shared.print(&error.to_string()),
                }
            }
        }
    }
}

/// Rcon host adapter (donor `RconService` options).
struct Q3RconAdapter<T, H> {
    /// Shared server state.
    shared: Q3Shared<T, H>,
}

impl<T, H> RconHost for Q3RconAdapter<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    fn password(&self) -> String {
        self.shared
            .host
            .borrow_mut()
            .administration()
            .map(|administration| administration.rcon_password())
            .unwrap_or_default()
    }

    fn execute(&mut self, command: &str, output: &mut dyn FnMut(&str)) -> Result<(), AdminError> {
        let mut host = self.shared.host.borrow_mut();
        let Some(administration) = host.administration() else {
            panic!("Server administration is unavailable");
        };
        administration.execute(command, output);
        Ok(())
    }

    fn reply(&mut self, to: &NetworkAddress, text: &str) {
        match encode_connectionless_text(&format!("print\n{text}")) {
            Ok(packet) => {
                let _ = self.shared.transport.send(to, &packet);
            }
            Err(error) => self.shared.print(&error.to_string()),
        }
    }
}

/// Heartbeat packet sender over the shared transport.
struct Q3PacketSender<T> {
    /// Shared transport.
    transport: Rc<T>,
}

impl<T: DatagramTransport<Address = NetworkAddress>> PacketSender for Q3PacketSender<T> {
    fn send(&mut self, to: &NetworkAddress, bytes: &[u8]) -> bool {
        self.transport.send(to, bytes).unwrap_or(false)
    }
}

/// Leaked heartbeat bundle (wire plus sender).
struct Q3HeartbeatBundle<T> {
    /// Discovery wire.
    wire: Q3DiscoveryWire,
    /// Packet sender.
    sender: Q3PacketSender<T>,
}

/// Download bindings (donor `Q3ServerDownload` options).
struct Q3DownloadBindings<T, H> {
    /// Shared server state.
    shared: Q3Shared<T, H>,
    /// Peer slot.
    slot: i32,
}

impl<T, H> Q3DownloadServerBindings for Q3DownloadBindings<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    fn open(&mut self, name: &str) -> Option<Box<dyn qa_net::q3_net::Q3DownloadReadFile>> {
        self.shared.host.borrow_mut().open_download(name)
    }

    fn enabled(&self) -> bool {
        self.shared.host.borrow().downloads_enabled()
    }

    fn pure(&self) -> bool {
        let epoch = *self.shared.epoch.borrow();
        self.shared
            .host
            .borrow()
            .pure(epoch.server_id, Some(epoch.restarted_server_id))
            .enabled
    }

    fn drop_client(&mut self, reason: &str) {
        self.shared.defer_drop(self.slot, reason);
    }

    fn print(&mut self, text: &str) {
        self.shared.print(text);
    }
}

/// Connection bindings (donor `Q3ServerConnection` options). The connection
/// pointer is armed only during the peer's datagram so callbacks can run the
/// reentrant commands (`cp`, `vdr`) exactly like the donor.
struct Q3ConnBindings<T, H> {
    /// Shared server state.
    shared: Q3Shared<T, H>,
    /// Peer slot.
    slot: i32,
    /// Armed connection.
    connection: *mut Q3ServerConnection<'static>,
}

impl<T, H> Q3ConnBindings<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    /// Borrow the armed connection, if any.
    fn armed(&mut self) -> Option<&mut Q3ServerConnection<'static>> {
        // SAFETY: the pointer is armed only during the peer's datagram and
        // the library never touches the connection while a callback runs.
        unsafe { self.connection.as_mut() }
    }

    /// Whether the peer is live for message purposes (donor command tail).
    fn message_live(&mut self) -> bool {
        self.shared.has_peer(self.slot)
            && self
                .armed()
                .is_some_and(|connection| connection.phase != Q3ServerPhase::Zombie)
    }

    /// Run one user command (donor `input`).
    fn input_by_slot(&mut self, slot: i32, command: &WireUserCommand) {
        let gate = self
            .shared
            .peers
            .borrow_mut()
            .iter_mut()
            .find(|peer| peer.slot == slot)
            .map(|peer| {
                let player = peer.player.clone();
                let sequence = peer.sequence;
                peer.sequence = peer.sequence.wrapping_add(1);
                (player, sequence)
            });
        let Some((player, sequence)) = gate else {
            return;
        };
        let accepted = self
            .shared
            .guard_game(|| self.shared.host.borrow_mut().input(&player, command, sequence));
        if self.message_live() {
            if let Some(accepted) = accepted {
                self.shared.pending.borrow_mut().push(accepted);
            }
        }
    }
}

impl<T, H> Q3ServerBindings for Q3ConnBindings<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    fn assert_current(&mut self) {
        if !self.message_live() {
            panic!("Q3 server callback belongs to a dropped client");
        }
    }

    fn server_id(&self) -> i32 {
        self.shared.epoch.borrow().server_id
    }

    fn restarted_server_id(&self) -> i32 {
        self.shared.epoch.borrow().restarted_server_id
    }

    fn checksum_feed(&self) -> i32 {
        self.shared.epoch.borrow().checksum_feed
    }

    fn pure(&self) -> bool {
        let epoch = *self.shared.epoch.borrow();
        self.shared
            .host
            .borrow()
            .pure(epoch.server_id, Some(epoch.restarted_server_id))
            .enabled
    }

    fn debug_build(&self) -> bool {
        false
    }

    fn time(&self) -> i32 {
        self.shared.host.borrow().time()
    }

    fn client_running(&self) -> bool {
        false
    }

    fn flood_protect(&self) -> bool {
        admission_value(&self.shared, true, |admission| admission.flood_protect())
    }

    fn download_name(&self) -> String {
        self.shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == self.slot)
            .and_then(|peer| peer.download.download.as_ref())
            .map(|download| download.name.clone())
            .unwrap_or_default()
    }

    fn command(&mut self, command: &ReliableCommand, client_ok: bool) -> Result<bool, Q3NetError> {
        let argv = tokenize_command(&command.text, Dialect::Q3, TextMode::Source)?.argv;
        let name = argv.first().map(String::as_str).unwrap_or("");
        let slot = self.slot;
        if name == "disconnect" {
            self.shared.defer_drop(slot, "disconnected");
            return Ok(false);
        }
        if name == "cp" {
            let epoch = *self.shared.epoch.borrow();
            let server = self
                .shared
                .host
                .borrow()
                .pure(epoch.server_id, Some(epoch.restarted_server_id));
            let shared = self.shared.clone();
            let mut resnapshot = || {
                if let Err(error) = shared.snapshot(slot) {
                    panic!("{error}");
                }
            };
            let Some(connection) = self.armed() else {
                return Ok(false);
            };
            connection.verify_pure(&server, &argv, &mut resnapshot)?;
            return Ok(connection.phase != Q3ServerPhase::Zombie);
        }
        if name == "vdr" {
            if let Some(connection) = self.armed() {
                connection.pure_authentic = false;
                connection.got_pure_command = false;
            }
            return Ok(true);
        }
        if name == "download" {
            let requested: String = argv
                .get(1)
                .map(|argument| argument.chars().take(DOWNLOAD_NAME_CHARS).collect())
                .unwrap_or_default();
            if !requested.is_empty() {
                if let Err(_invalid) = check_q3_download_name(&requested) {
                    self.shared.defer_drop(slot, "Invalid download path");
                    return Ok(false);
                }
            }
            if let Some(peer) = self.shared.peers.borrow_mut().iter_mut().find(|peer| peer.slot == slot) {
                if let Some(download) = peer.download.download.as_mut() {
                    download.begin(&requested);
                }
            }
            return Ok(true);
        }
        if name == "nextdl" {
            let block = native_atoi(argv.get(1).map(String::as_str).unwrap_or(""));
            let time = self.shared.host.borrow().time();
            // Acknowledge runs drop callbacks (peer borrows), so grab the
            // download pointer and release the borrow first. The peer
            // vector is stable across the call (drops only queue).
            let download = self
                .shared
                .peers
                .borrow_mut()
                .iter_mut()
                .find(|peer| peer.slot == slot)
                .and_then(|peer| peer.download.download.as_mut())
                .map(|live| live as *mut Q3ServerDownload<'static>);
            if let Some(download) = download {
                // SAFETY: see above.
                unsafe {
                    (&mut *download).acknowledge(block, time);
                }
            }
            return Ok(self
                .armed()
                .is_some_and(|connection| connection.phase != Q3ServerPhase::Zombie));
        }
        if name == "stopdl" {
            if let Some(peer) = self.shared.peers.borrow_mut().iter_mut().find(|peer| peer.slot == slot) {
                if let Some(download) = peer.download.download.as_mut() {
                    download.close();
                }
            }
            return Ok(true);
        }
        if name == "donedl" {
            let primed = self
                .armed()
                .is_some_and(|connection| connection.phase != Q3ServerPhase::Active);
            if primed {
                self.shared.gamestate(slot).map_err(into_lib)?;
            }
            return Ok(true);
        }
        if name == "userinfo" {
            let value = argv.get(1).cloned().unwrap_or_default();
            let player = self
                .shared
                .peers
                .borrow_mut()
                .iter_mut()
                .find(|peer| peer.slot == slot)
                .map(|peer| {
                    peer.userinfo = value.clone();
                    peer.player.clone()
                });
            if let Some(player) = player {
                self.shared
                    .guard_game(|| self.shared.host.borrow_mut().userinfo(&player, &value));
            }
        } else if client_ok {
            let gate = self
                .shared
                .peers
                .borrow()
                .iter()
                .find(|peer| peer.slot == slot)
                .map(|peer| peer.player.clone());
            if let Some(player) = gate {
                let args = argv.get(1..).unwrap_or(&[]).to_vec();
                self.shared
                    .guard_game(|| self.shared.host.borrow_mut().command(&player, name, &args));
            }
        }
        Ok(self.message_live())
    }

    fn enter_world(&mut self, command: &WireUserCommand) -> Result<(), Q3NetError> {
        let player = self
            .shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == self.slot)
            .map(|peer| peer.player.clone());
        let Some(player) = player else {
            return Ok(());
        };
        if self
            .shared
            .guard_game(|| self.shared.host.borrow_mut().begin(&player, command))
        {
            self.input_by_slot(self.slot, command);
        }
        Ok(())
    }

    fn think(&mut self, command: &WireUserCommand) -> Result<(), Q3NetError> {
        self.input_by_slot(self.slot, command);
        Ok(())
    }

    fn resend_gamestate(&mut self) -> Result<(), Q3NetError> {
        self.shared.gamestate(self.slot).map_err(into_lib)
    }

    fn drop_client(&mut self, reason: &str) -> Result<(), Q3NetError> {
        self.shared.defer_drop(self.slot, reason);
        Ok(())
    }

    fn print(&mut self, text: &str) {
        self.shared.print(text);
    }
}

impl<T, H> Q3Shared<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    /// Live peer slots in order.
    fn peer_slots(&self) -> Vec<i32> {
        self.peers.borrow().iter().map(|peer| peer.slot).collect()
    }

    /// Admission slot views (donor `slots`).
    fn slots(&self) -> Vec<Q3AdmissionSlot> {
        let host = self.host.borrow();
        let max_clients = host.max_clients();
        let occupied = host.occupied_slots();
        let peers = self.peers.borrow();
        (0..max_clients)
            .map(|slot| {
                let slot = slot as i32;
                match peers.iter().find(|peer| peer.slot == slot) {
                    None => Q3AdmissionSlot {
                        slot,
                        phase: if occupied.contains(&slot) {
                            Q3SlotPhase::Active
                        } else {
                            Q3SlotPhase::Free
                        },
                        address: None,
                        bot: false,
                        qport: 0,
                        last_connect_time: 0,
                    },
                    Some(peer) => {
                        let Some(connection) = peer.conn.cell.connection() else {
                            return Q3AdmissionSlot {
                                slot,
                                phase: Q3SlotPhase::Free,
                                address: None,
                                bot: false,
                                qport: 0,
                                last_connect_time: 0,
                            };
                        };
                        Q3AdmissionSlot {
                            slot,
                            phase: match connection.phase {
                                Q3ServerPhase::Connected => Q3SlotPhase::Connected,
                                Q3ServerPhase::Primed => Q3SlotPhase::Primed,
                                Q3ServerPhase::Active => Q3SlotPhase::Active,
                                Q3ServerPhase::Zombie => Q3SlotPhase::Zombie,
                            },
                            address: Some(peer.remote.clone()),
                            bot: false,
                            qport: connection.channel.qport(),
                            last_connect_time: peer.connected_at,
                        }
                    }
                }
            })
            .collect()
    }

    /// Remove a peer without notifying (eviction cleanup).
    fn remove_peer(&self, slot: i32) {
        self.peers.borrow_mut().retain(|peer| peer.slot != slot);
    }

    /// Build a delivery for a remote address (no peer borrow, so send
    /// paths can hold the peer while transmitting).
    fn delivery_for(&self, remote: &NetworkAddress) -> Q3Delivery<'_, T> {
        let shared = self.clone();
        Q3Delivery {
            transport: &self.transport,
            remote: remote.clone(),
            error: None,
            trace: Box::new(move |message| shared.print(message)),
        }
    }

    /// Send the gamestate (donor `gamestate`).
    fn gamestate(&self, slot: i32) -> Result<(), Q3ServerError> {
        let gate = self
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == slot)
            .map(|peer| (peer.player.clone(), peer.baselines.clone(), peer.remote.clone()));
        let Some((player, baselines, remote)) = gate else {
            return Ok(());
        };
        let epoch = *self.epoch.borrow();
        let state = self.host.borrow().game_state(&player, epoch.server_id);
        let rate = self.host.borrow().rate(&player);
        baselines.borrow_mut().clear();
        for entry in &state.entries {
            if let GamestateEntry::Baseline { number, entity } = entry {
                baselines.borrow_mut().insert(*number, entity.clone());
            }
        }
        // Capture the connection pointer, then release the borrow:
        // transmit reenters through `download_name` (peer borrows). The
        // peer vector is stable across the send (transmit callbacks never
        // mutate peers).
        let connection = {
            let mut peers = self.peers.borrow_mut();
            peers
                .iter_mut()
                .find(|peer| peer.slot == slot)
                .and_then(|peer| peer.conn.cell.connection_mut())
                .map(|live| live as *mut Q3ServerConnection<'static>)
        };
        let Some(connection) = connection else {
            return Ok(());
        };
        let mut delivery = self.delivery_for(&remote);
        // SAFETY: see above.
        unsafe {
            (&mut *connection).send_gamestate(&state, rate, &mut delivery)?;
        }
        if let Some(error) = delivery.error {
            return Err(Q3ServerError::Transport(error));
        }
        Ok(())
    }

    /// Send a snapshot (donor `snapshot`). Download-write failures reject
    /// like the donor throw.
    fn snapshot(&self, slot: i32) -> Result<(), Q3ServerError> {
        let gate = self
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == slot)
            .map(|peer| (peer.player.clone(), peer.remote.clone()));
        let Some((player, remote)) = gate else {
            return Ok(());
        };
        let server_flags = match self.epoch.borrow().server_flags {
            Q3SnapshotServerBit::Zero => 0,
            Q3SnapshotServerBit::Four => 4,
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            // Capture pointers, then release the borrow: transmit reenters
            // through `download_name` (peer borrows). The peer vector is
            // stable across the send (transmit callbacks never mutate
            // peers).
            let gate = {
                let mut peers = self.peers.borrow_mut();
                let Some(peer) = peers.iter_mut().find(|peer| peer.slot == slot) else {
                    return Ok(());
                };
                let download = peer
                    .download
                    .download
                    .as_mut()
                    .map(|live| live as *mut Q3ServerDownload<'static>);
                let connection = peer
                    .conn
                    .cell
                    .connection_mut()
                    .map(|live| live as *mut Q3ServerConnection<'static>);
                (download, connection)
            };
            let (download, Some(connection)) = gate else {
                return Ok(());
            };
            // SAFETY: see above.
            let connection = unsafe { &mut *connection };
            if !connection.channel.has_unsent_fragments() {
                let frame = self.host.borrow().snapshot(&player);
                connection.snapshots.capture(
                    connection.channel.outgoing_sequence(),
                    &frame.player,
                    &frame.area_mask,
                    &frame.entities,
                )?;
            }
            let rate = self.host.borrow().rate(&player);
            let time = self.host.borrow().time();
            let settings = Q3DownloadRate {
                rate: rate.rate,
                max_rate: rate.max_rate,
                snapshot_msec: rate.snapshot_msec,
            };
            let mut delivery = self.delivery_for(&remote);
            let outcome =
                connection.send_snapshot(server_flags, rate, &mut delivery, &mut |writer: &mut Q3MsgWriter| {
                    if let Some(download) = download {
                        // SAFETY: disjoint from the connection borrow (see
                        // above); the closure runs before any other
                        // download access.
                        if let Err(error) = unsafe { &mut *download }.write(writer, time, settings) {
                            panic!("{error}");
                        }
                    }
                });
            if let Some(error) = delivery.error {
                return Err(Q3ServerError::Transport(error));
            }
            outcome.map_err(Q3ServerError::Net)
        }));
        match outcome {
            Ok(result) => result,
            Err(payload) => Err(Q3ServerError::Message(panic_message(payload))),
        }
    }

    /// Queue a reliable server command (donor `queue`).
    fn queue(&self, slot: i32, text: &str) -> Result<(), Q3ServerError> {
        let gate = self
            .peers
            .borrow_mut()
            .iter_mut()
            .find(|peer| peer.slot == slot)
            .map(|peer| peer.player.client.clone());
        let Some(client) = gate else {
            return Ok(());
        };
        let overflow = {
            let mut peers = self.peers.borrow_mut();
            let Some(peer) = peers.iter_mut().find(|peer| peer.slot == slot) else {
                return Ok(());
            };
            let Some(connection) = peer.conn.cell.connection_mut() else {
                return Ok(());
            };
            matches!(connection.reliable.add(text)?, ServerCommandAppend::Overflow { .. })
        };
        if overflow {
            self.disconnect_client(&client, "Server command overflow")?;
        }
        Ok(())
    }

    /// Send one configstring (donor `configstring`).
    fn configstring(&self, slot: i32, index: u32, value: &str) -> Result<(), Q3ServerError> {
        let index = i32::try_from(index).unwrap_or(i32::MAX);
        for command in q3_configstring_commands(index, value) {
            if self.ended.get() || !self.has_peer(slot) {
                break;
            }
            self.queue(slot, &command)?;
        }
        Ok(())
    }

    /// Broadcast one configstring (donor `broadcastConfigstring`).
    fn broadcast_configstring(&self, index: u32, value: &str) -> Result<(), Q3ServerError> {
        for slot in self.peer_slots() {
            let connected = self
                .peers
                .borrow()
                .iter()
                .find(|peer| peer.slot == slot)
                .is_some_and(|peer| {
                    peer.conn
                        .cell
                        .connection()
                        .is_some_and(|connection| connection.phase == Q3ServerPhase::Connected)
                });
            if connected {
                continue;
            }
            self.configstring(slot, index, value)?;
        }
        Ok(())
    }

    /// Disconnect a client (donor `disconnectClient`).
    fn disconnect_client(&self, client: &ClientId, reason: &str) -> Result<bool, Q3ServerError> {
        let gate = self
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.player.client == *client)
            .map(|peer| (peer.slot, peer.player.clone(), peer.remote.clone()));
        let Some((slot, player, remote)) = gate else {
            return Ok(false);
        };
        if let Some(peer) = self.peers.borrow_mut().iter_mut().find(|peer| peer.slot == slot) {
            if let Some(download) = peer.download.download.as_mut() {
                download.close();
            }
        }
        self.queue_disconnect(slot, reason)?;
        self.remove_peer(slot);
        self.pending_admission_drops.borrow_mut().push(remote);
        self.pending
            .borrow_mut()
            .retain(|command| command.actor != player.actor);
        match q3_game_callback(|| self.host.borrow_mut().disconnect(&player, reason)) {
            Ok(()) => Ok(true),
            Err(error) => Err(Q3ServerError::GameCallback(error)),
        }
    }

    /// Queue the disconnect packets (donor `queueDisconnect`).
    fn queue_disconnect(&self, slot: i32, reason: &str) -> Result<(), Q3ServerError> {
        let gate = self
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == slot)
            .map(|peer| (peer.player.clone(), peer.remote.clone()));
        let Some((player, remote)) = gate else {
            return Ok(());
        };
        let sanitized: String = reason
            .chars()
            .filter(|cell| *cell != '"' && *cell != '\n' && *cell != '\r')
            .collect();
        let server_flags = match self.epoch.borrow().server_flags {
            Q3SnapshotServerBit::Zero => 0,
            Q3SnapshotServerBit::Four => 4,
        };
        let rate = self.host.borrow().rate(&player);
        // Capture the connection pointer, then release the borrow:
        // transmit reenters through `download_name` (peer borrows). The
        // peer vector is stable across the send (transmit callbacks never
        // mutate peers).
        let connection = {
            let mut peers = self.peers.borrow_mut();
            peers
                .iter_mut()
                .find(|peer| peer.slot == slot)
                .and_then(|peer| peer.conn.cell.connection_mut())
                .map(|live| live as *mut Q3ServerConnection<'static>)
        };
        let Some(connection) = connection else {
            return Ok(());
        };
        // SAFETY: see above.
        let connection = unsafe { &mut *connection };
        let _ = connection.reliable.add(&format!("disconnect \"{sanitized}\""));
        let mut delivery = self.delivery_for(&remote);
        while connection.channel.has_unsent_fragments() {
            connection.transmit_next_fragment(&mut delivery)?;
        }
        connection.send_snapshot(server_flags, rate, &mut delivery, &mut |_| {})?;
        while connection.channel.has_unsent_fragments() {
            connection.transmit_next_fragment(&mut delivery)?;
        }
        if let Some(error) = delivery.error {
            return Err(Q3ServerError::Transport(error));
        }
        Ok(())
    }
}

/// Quake III server network (donor `Q3ServerNetwork`).
pub struct Q3ServerNetwork<T, H> {
    /// Shared server state.
    shared: Q3Shared<T, H>,
    /// Admission server over leaked bindings (`None` only in `Drop`).
    admission: Option<Q3ServerAdmission<'static>>,
    /// Leaked admission bindings, reclaimed by `Drop`.
    admission_raw: *mut Q3AdmissionBindings<T, H>,
    /// Leaked authorization bindings, reclaimed by `Drop`.
    auth_raw: *mut Q3AuthBindings<T, H>,
    /// Rcon service over the leaked adapter (`None` only in `Drop`).
    rcon: Option<RconService<'static>>,
    /// Leaked rcon adapter, reclaimed by `Drop`.
    rcon_raw: *mut Q3RconAdapter<T, H>,
    /// Master heartbeat over the leaked bundle (`None` only in `Drop`).
    heartbeat: Option<MasterHeartbeat<'static>>,
    /// Leaked heartbeat bundle, reclaimed by `Drop`.
    heartbeat_raw: *mut Q3HeartbeatBundle<T>,
    /// Idle timeout override.
    timeout: Option<u64>,
    /// Truncated poll time.
    now: i32,
    /// World-changed flag (donor `changedWorld`).
    changed_world: bool,
}

impl<T, H> Q3ServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    /// Create a server network (donor constructor).
    pub fn new(options: Q3ServerNetworkOptions<T, H>) -> Result<Self, Q3ServerError> {
        if let WireAdmission::Unsupported { reasons } = options.host.supports_source_wire() {
            return Err(Q3ServerError::Message(reasons.join("; ")));
        }
        let Q3ServerNetworkOptions {
            transport,
            host,
            random,
            timeout_milliseconds,
            resolve_authorization,
        } = options;
        let random: Rc<RefCell<Box<dyn FnMut() -> f64>>> = Rc::new(RefCell::new(random));
        let checksum_feed = next_random(&random).wrapping_shl(16) ^ next_random(&random);
        let shared = Q3Shared {
            transport: Rc::new(transport),
            host: Rc::new(RefCell::new(host)),
            peers: Rc::new(RefCell::new(Vec::new())),
            epoch: Rc::new(RefCell::new(Q3ServerEpoch {
                server_id: 1,
                restarted_server_id: 1,
                server_flags: Q3SnapshotServerBit::Zero,
                checksum_feed,
            })),
            random,
            ended: Rc::new(Cell::new(false)),
            now: Rc::new(Cell::new(0)),
            pending: Rc::new(RefCell::new(Vec::new())),
            deferred: Rc::new(RefCell::new(Vec::new())),
            fatal: Rc::new(RefCell::new(None)),
            pending_admission_drops: Rc::new(RefCell::new(Vec::new())),
        };
        let rcon_raw = Box::into_raw(Box::new(Q3RconAdapter { shared: shared.clone() }));
        // SAFETY: the adapter is uniquely owned (reclaimed by `Drop` after
        // the service drops), so the lease is exclusive.
        let rcon = RconService::new(unsafe { &mut *rcon_raw }, RconProfile::Q3, RCON_OUTPUT_BYTES)?;
        let heartbeat_raw = Box::into_raw(Box::new(Q3HeartbeatBundle {
            wire: Q3DiscoveryWire::new(&[], 68)?,
            sender: Q3PacketSender {
                transport: shared.transport.clone(),
            },
        }));
        // SAFETY: same unique-ownership protocol; the wire and sender
        // leases borrow disjoint fields.
        let heartbeat = unsafe {
            let bundle: &'static mut Q3HeartbeatBundle<T> = &mut *heartbeat_raw;
            MasterHeartbeat::new(&bundle.wire, &mut bundle.sender, HEARTBEAT_INTERVAL_MILLISECONDS)
        };
        if matches!(shared.transport.address(), NetworkAddress::Ipx { .. }) {
            shared.print("Q3 IPX transport supports LAN play; IPv4 master advertisement is unavailable.\n");
        }
        let auth_raw = Box::into_raw(Box::new(Q3AuthBindings {
            shared: shared.clone(),
            resolve: resolve_authorization,
        }));
        // SAFETY: same unique-ownership protocol.
        let authorization = unsafe { Q3ServerAuthorization::new(&mut *auth_raw) };
        let admission_raw = Box::into_raw(Box::new(Q3AdmissionBindings {
            shared: shared.clone(),
            authorization,
        }));
        // SAFETY: same unique-ownership protocol.
        let admission = unsafe { Q3ServerAdmission::new(&mut *admission_raw) };
        Ok(Self {
            shared,
            admission: Some(admission),
            admission_raw,
            auth_raw,
            rcon: Some(rcon),
            rcon_raw,
            heartbeat: Some(heartbeat),
            heartbeat_raw,
            timeout: timeout_milliseconds,
            now: 0,
            changed_world: false,
        })
    }

    /// Poll the transport (donor `pollPackets`). Game-callback failures
    /// abort with an error like the donor rethrow; everything else prints
    /// and continues like the donor catch.
    fn poll_packets(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, Q3ServerError> {
        if self.shared.ended.get() {
            return Ok(Vec::new());
        }
        self.now = i32::try_from(now_milliseconds).unwrap_or(i32::MAX);
        self.shared.now.set(self.now);
        let epoch = *self.shared.epoch.borrow();
        let prepare = catch_unwind(AssertUnwindSafe(|| {
            let shared = self.shared.clone();
            self.shared.host.borrow_mut().prepare(
                epoch.checksum_feed,
                epoch.server_id,
                Some(&mut |index, value| {
                    // The donor callback floats its promise (a rejection
                    // escapes); fail the poll instead.
                    if let Err(error) = shared.broadcast_configstring(index, value) {
                        panic!("{error}");
                    }
                }),
            );
        }));
        match prepare {
            Ok(()) => {}
            Err(payload) => return Err(Q3ServerError::Message(panic_message(payload))),
        }
        if self.shared.ended.get() {
            return Ok(Vec::new());
        }
        if self.changed_world {
            self.changed_world = false;
            for slot in self.shared.peer_slots() {
                self.shared.gamestate(slot)?;
            }
        }
        loop {
            let event = self.shared.transport.poll()?;
            let Some(event) = event else {
                break;
            };
            if self.shared.ended.get() {
                break;
            }
            match event {
                ReceiveEvent::Packet { from, payload, .. } => {
                    if !matches!(
                        from,
                        NetworkAddress::Ipv4 { .. } | NetworkAddress::Loopback { .. } | NetworkAddress::Ipx { .. }
                    ) {
                        continue;
                    }
                    // Rejects run outside the catch like the donor.
                    let rejected = self
                        .shared
                        .host
                        .borrow_mut()
                        .administration()
                        .is_some_and(|administration| administration.rejects(&from));
                    if rejected {
                        continue;
                    }
                    let outcome = catch_unwind(AssertUnwindSafe(|| self.event(&from, &payload)));
                    if let Some(message) = self.shared.fatal.borrow_mut().take() {
                        return Err(Q3ServerError::GameCallback(Q3GameCallbackError::Failed(message)));
                    }
                    match outcome {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => {
                            if self.shared.ended.get() {
                                break;
                            }
                            self.guard_print(&error.to_string())?;
                        }
                        Err(payload) => {
                            if self.shared.ended.get() {
                                break;
                            }
                            self.guard_print(&panic_message(payload))?;
                        }
                    }
                }
                ReceiveEvent::Error { error } => self.shared.print(&error),
                ReceiveEvent::Dropped { .. } => {}
            }
        }
        if self.shared.ended.get() {
            self.shared.pending.borrow_mut().clear();
            return Ok(Vec::new());
        }
        Ok(std::mem::take(&mut *self.shared.pending.borrow_mut()))
    }

    /// Print from the poll catch (donor catch print; aborts when printing
    /// itself fails, like the donor throw out of poll).
    fn guard_print(&mut self, text: &str) -> Result<(), Q3ServerError> {
        match catch_unwind(AssertUnwindSafe(|| self.shared.print(text))) {
            Ok(()) => Ok(()),
            Err(payload) => Err(Q3ServerError::Message(panic_message(payload))),
        }
    }

    /// Handle one packet (donor poll packet `try`).
    fn event(&mut self, from: &NetworkAddress, payload: &[u8]) -> Result<(), Q3ServerError> {
        if payload.len() >= 4 && payload[0] == 255 && payload[1] == 255 && payload[2] == 255 && payload[3] == 255 {
            let packet = decode_connectionless(payload, ConnectionlessReceiver::Server)?;
            if packet.command == "rcon" {
                let argument = packet.arguments.first().map(String::as_str).unwrap_or("");
                let command = q3_rcon_command(&packet.line);
                let Some(rcon) = self.rcon.as_mut() else {
                    return Ok(());
                };
                let result = rcon.handle(from, argument, &command, f64::from(self.now))?;
                if let Some(administration) = self.shared.host.borrow_mut().administration() {
                    administration.record(from, result);
                }
            } else {
                let taken = self.admission.take();
                if let Some(mut admission) = taken {
                    let outcome = admission.receive(from, payload, self.now);
                    self.admission = Some(admission);
                    outcome?;
                }
            }
            self.drain_admission();
            return Ok(());
        }
        let slots = self.shared.slots();
        let route = route_q3_sequenced_packet(from, payload, &slots);
        let Some(route) = route else {
            return Ok(());
        };
        let slot = route.slot;
        if !self.shared.has_peer(slot) {
            return Ok(());
        }
        if let Some(peer) = self.shared.peers.borrow_mut().iter_mut().find(|peer| peer.slot == slot) {
            peer.remote.clone_from(from);
        }
        // Arm the callback connection, then release every borrow before
        // the library frame runs: callbacks re-borrow peers freely, and the
        // peer vector is stable across the frame (drops only queue).
        let armed = {
            let mut peers = self.shared.peers.borrow_mut();
            let Some(peer) = peers.iter_mut().find(|peer| peer.slot == slot) else {
                return Ok(());
            };
            let previous = peer.conn.cell.adapter_mut().connection;
            let live = peer
                .conn
                .cell
                .connection_mut()
                .map(|live| live as *mut Q3ServerConnection<'static>);
            if let Some(live) = live {
                peer.conn.cell.adapter_mut().connection = live;
            }
            (live, previous)
        };
        let (live, previous) = armed;
        let Some(live) = live else {
            return Ok(());
        };
        // SAFETY: the peer vector is stable across the frame (see above),
        // so the connection pointer stays valid; disarmed below on every
        // path, including unwinds.
        let outcome = catch_unwind(AssertUnwindSafe(|| unsafe { (*live).receive_datagram(payload) }));
        if let Some(peer) = self.shared.peers.borrow_mut().iter_mut().find(|peer| peer.slot == slot) {
            peer.conn.cell.adapter_mut().connection = previous;
        }
        let result = match outcome {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        };
        let accepted = result?;
        self.drain_deferred()?;
        self.drain_admission();
        if !self.shared.has_peer(slot) {
            return Ok(());
        }
        // Peers dropped mid-message read zombie (donor `peers.get` miss).
        let zombie = self
            .shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == slot)
            .is_some_and(|peer| {
                peer.conn
                    .cell
                    .connection()
                    .is_some_and(|connection| connection.phase == Q3ServerPhase::Zombie)
            });
        if zombie {
            return Ok(());
        }
        if matches!(accepted, ChannelResult::Accepted { .. }) {
            if let Some(peer) = self.shared.peers.borrow_mut().iter_mut().find(|peer| peer.slot == slot) {
                peer.last_received = self.now;
            }
            let connected = self
                .shared
                .peers
                .borrow()
                .iter()
                .find(|peer| peer.slot == slot)
                .is_some_and(|peer| {
                    peer.conn
                        .cell
                        .connection()
                        .is_some_and(|connection| connection.phase == Q3ServerPhase::Connected)
                });
            if connected {
                self.shared.gamestate(slot)?;
            }
        }
        Ok(())
    }

    /// Drain deferred mid-datagram drops in order. Game failures record the
    /// fatal flag (the poll aborts like the donor rethrow); transport
    /// failures return for the donor catch.
    fn drain_deferred(&mut self) -> Result<(), Q3ServerError> {
        for deferred in std::mem::take(&mut *self.shared.deferred.borrow_mut()) {
            let client = self
                .shared
                .peers
                .borrow()
                .iter()
                .find(|peer| peer.slot == deferred.slot)
                .map(|peer| peer.player.client.clone());
            let Some(client) = client else {
                continue;
            };
            match self.shared.disconnect_client(&client, &deferred.reason) {
                Ok(_) => {}
                Err(Q3ServerError::GameCallback(message)) => {
                    self.shared.fatal.borrow_mut().replace(message.to_string());
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Drain queued admission disconnects.
    fn drain_admission(&mut self) {
        let drops = std::mem::take(&mut *self.shared.pending_admission_drops.borrow_mut());
        if drops.is_empty() {
            return;
        }
        if let Some(admission) = self.admission.as_mut() {
            for address in &drops {
                admission.disconnect(address);
            }
        } else {
            self.shared.pending_admission_drops.borrow_mut().extend(drops);
        }
    }
}

impl<T, H> Drop for Q3ServerNetwork<T, H> {
    fn drop(&mut self) {
        self.admission = None;
        self.rcon = None;
        self.heartbeat = None;
        // SAFETY: each box was uniquely leased above and every lease
        // dropped with the fields just cleared.
        unsafe {
            drop(Box::from_raw(self.admission_raw));
            drop(Box::from_raw(self.auth_raw));
            drop(Box::from_raw(self.rcon_raw));
            drop(Box::from_raw(self.heartbeat_raw));
        }
    }
}

/// Server recording source (donor `recordingSource`).
pub struct Q3ServerRecordingSource<H> {
    /// Current host cell.
    pub host: Rc<RefCell<H>>,
    /// Current server id.
    pub server_id: i32,
    /// Snapshot server bit.
    pub snapshot_server_bit: Q3SnapshotServerBit,
}

/// Game output handle (donor `gameOutput`). The shared state travels
/// through `Rc` handles so the game can hold the output across polls.
pub struct Q3ServerGameOutput<T, H> {
    /// Shared server state.
    shared: Q3Shared<T, H>,
}

impl<T, H> Clone for Q3ServerGameOutput<T, H> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl<T, H> Q3ServerGameOutput<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    /// Drop a client by source entity number (donor `dropClient`).
    pub fn drop_client(&self, slot: u32, reason: &str) -> Result<(), Q3ServerError> {
        let client = self
            .shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.player.source_entity == slot)
            .map(|peer| peer.player.client.clone());
        if let Some(client) = client {
            self.shared.disconnect_client(&client, reason)?;
        }
        Ok(())
    }

    /// Queue a server command (`-1` broadcasts, donor `sendServerCommand`).
    pub fn send_server_command(&self, slot: i32, text: &str) -> Result<(), Q3ServerError> {
        for peer_slot in self.shared.peer_slots() {
            if self.shared.ended.get() || !self.shared.has_peer(peer_slot) {
                continue;
            }
            let addressed = self
                .shared
                .peers
                .borrow()
                .iter()
                .find(|peer| peer.slot == peer_slot)
                .is_some_and(|peer| slot == -1 || i64::from(peer.player.source_entity) == i64::from(slot));
            if addressed {
                self.shared.queue(peer_slot, text)?;
            }
        }
        Ok(())
    }

    /// Broadcast a configstring (donor `configstring`).
    pub fn configstring(&self, index: u32, value: &str) -> Result<(), Q3ServerError> {
        self.shared.broadcast_configstring(index, value)
    }
}

/// Round-restart scope (donor `Q3NetworkRoundRestart`).
pub struct Q3RoundRestartScope<'n, T, H> {
    /// Borrowing network.
    network: &'n mut Q3ServerNetwork<T, H>,
    /// Peers captured at restart start (slot plus client).
    snapshot: Vec<(i32, ClientId)>,
    /// Slots awaiting reconnect.
    pending: HashSet<i32>,
    /// Lifecycle phase (donor `lifecycle.phase`).
    bound: bool,
    /// Stored step failure, returned as-is by the restart.
    failed: Option<Q3ServerError>,
    /// Scope closed flag.
    closed: bool,
}

impl<T, H> Q3RoundRestartScope<'_, T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    /// Guard the scope (donor `current`; the world check is vacuous
    /// synchronously).
    fn current(&self) {
        if self.closed {
            panic!("Q3 source restart scope has ended");
        }
    }

    /// Store a step failure and panic for the restart catch.
    fn fail(&mut self, error: Q3ServerError) -> ! {
        let message = error.to_string();
        self.failed = Some(error);
        panic!("{message}");
    }
}

impl<T, H> Q3NetworkRoundRestart for Q3RoundRestartScope<'_, T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    fn clients(&self) -> Vec<ClientId> {
        self.snapshot.iter().map(|(_, client)| client.clone()).collect()
    }

    fn snapshot_server_bit(&self) -> Q3SnapshotServerBit {
        self.network.shared.epoch.borrow().server_flags
    }

    fn bind_source(&mut self) {
        self.current();
        if self.bound {
            panic!("Q3 network source round was already rebound");
        }
        if let Some(binding) = self.network.shared.host.borrow_mut().source_round() {
            binding.rebind();
        }
        let epoch = *self.network.shared.epoch.borrow();
        let shared = self.network.shared.clone();
        self.network.shared.host.borrow_mut().prepare(
            epoch.checksum_feed,
            epoch.server_id,
            Some(&mut |index, value| {
                if let Err(error) = shared.broadcast_configstring(index, value) {
                    panic!("{error}");
                }
            }),
        );
        self.current();
        self.bound = true;
    }

    fn receive_events(&mut self, events: &[NetworkPresentationEvent]) {
        self.current();
        if !self.bound {
            panic!("Bind the Q3 network source before delivering restart events");
        }
        // The carrier has no Q3 payload (see `publish_events`); filter the
        // `q3-source` events so the lifecycle still observes them.
        for event in events {
            if event.family == "q3" && event.kind == "q3-source" {
                self.current();
            }
        }
        self.current();
    }

    fn reconnect_client(&mut self, client: &ClientId) -> bool {
        self.current();
        if !self.bound {
            panic!("Bind the Q3 network source before reconnecting");
        }
        let slot = self
            .snapshot
            .iter()
            .find(|(_, known)| known == client)
            .map(|(slot, _)| *slot);
        let Some(slot) = slot else {
            return false;
        };
        if !self.pending.contains(&slot) {
            panic!("Q3 network client was already reconnected");
        }
        self.pending.remove(&slot);
        if self.network.shared.has_peer(slot) {
            if let Err(error) = self.network.shared.queue(slot, "map_restart\n") {
                self.fail(error);
            }
            self.current();
            if self.network.shared.has_peer(slot) {
                let gate = self
                    .network
                    .shared
                    .peers
                    .borrow()
                    .iter()
                    .find(|peer| peer.slot == slot)
                    .map(|peer| {
                        (
                            peer.userinfo.clone(),
                            peer.conn
                                .cell
                                .connection()
                                .map(|connection| connection.last_user_command.clone()),
                        )
                    });
                let Some((userinfo, Some(last_command))) = gate else {
                    return true;
                };
                let admitted = match q3_game_callback(|| {
                    self.network
                        .shared
                        .host
                        .borrow_mut()
                        .source_round()
                        .map(|binding| binding.reconnect(client, &userinfo, &last_command))
                }) {
                    Ok(Some(admitted)) => admitted,
                    Ok(None) => panic!("Q3 network host has no native source round restart"),
                    Err(error) => self.fail(Q3ServerError::GameCallback(error)),
                };
                self.current();
                match admitted {
                    Q3ApplicationAdmission::Rejected { reason } => {
                        if let Err(error) = self.network.shared.disconnect_client(client, &reason) {
                            self.fail(error);
                        }
                    }
                    Q3ApplicationAdmission::Accepted { player } => {
                        if player.client != *client {
                            panic!("Q3 round admission changed client identity");
                        }
                        let time = self.network.shared.host.borrow().time();
                        if let Some(peer) = self
                            .network
                            .shared
                            .peers
                            .borrow_mut()
                            .iter_mut()
                            .find(|peer| peer.slot == slot)
                        {
                            peer.player = player;
                            if let Some(connection) = peer.conn.cell.connection_mut() {
                                connection.phase = Q3ServerPhase::Active;
                                connection.delta_message = -1;
                                connection.next_snapshot_time = time;
                            }
                        }
                    }
                }
            }
        }
        true
    }
}

impl<T, H> Q3RoundRestartScope<'_, T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    /// Deliver native source events (donor `receiveEvents` with payloads).
    pub fn receive_source(&mut self, events: &[Q3SourcePresentationEvent]) -> Result<(), Q3ServerError> {
        self.current();
        if !self.bound {
            panic!("Bind the Q3 network source before delivering restart events");
        }
        self.network.receive_source_events(events)?;
        self.current();
        Ok(())
    }
}

impl<T, H> Q3ServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    /// Poll before the simulation step (donor `poll`).
    pub fn poll(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, Q3ServerError> {
        self.poll_packets(now_milliseconds)
    }

    /// Submit local input (donor `submit`: remote servers ignore it).
    pub fn submit(&mut self, _commands: &[ActorCommand], _now_milliseconds: u64) -> Result<(), Q3ServerError> {
        Ok(())
    }

    /// Publish after the simulation step (donor `publishEvents`).
    pub fn publish(
        &mut self,
        _output: &SimulationOutput,
        events: &[NetworkPresentationEvent],
        now_milliseconds: u64,
    ) -> Result<(), Q3ServerError> {
        if self.shared.ended.get() {
            return Ok(());
        }
        self.now = i32::try_from(now_milliseconds).unwrap_or(i32::MAX);
        self.shared.now.set(self.now);
        // `NetworkPresentationEvent` carries no Q3 source payload; native
        // payloads arrive through `receive_source_events` until the
        // simulation producer port grows a carrier.
        let _ = events;
        if self.shared.ended.get() {
            return Ok(());
        }
        let masters = self
            .shared
            .host
            .borrow_mut()
            .administration()
            .map(|administration| administration.masters())
            .unwrap_or_default();
        if !masters.is_empty() && !matches!(self.shared.transport.address(), NetworkAddress::Ipx { .. }) {
            if let Some(heartbeat) = self.heartbeat.as_mut() {
                heartbeat.send(&masters, now_milliseconds as f64, true, false)?;
            }
        }
        let timeout = self.timeout.unwrap_or(DEFAULT_TIMEOUT_MILLISECONDS);
        for slot in self.shared.peer_slots() {
            let gate = self
                .shared
                .peers
                .borrow()
                .iter()
                .find(|peer| peer.slot == slot)
                .map(|peer| (peer.player.client.clone(), peer.last_received));
            let Some((client, last_received)) = gate else {
                continue;
            };
            let age = u64::try_from(self.now.saturating_sub(last_received)).unwrap_or(u64::MAX);
            if age > timeout {
                self.shared.disconnect_client(&client, "timed out")?;
            }
        }
        for slot in self.shared.peer_slots() {
            let gate = self
                .shared
                .peers
                .borrow()
                .iter()
                .find(|peer| peer.slot == slot)
                .is_some_and(|peer| {
                    peer.conn.cell.connection().is_some_and(|connection| {
                        connection.phase != Q3ServerPhase::Connected
                            && self.shared.host.borrow().time() >= connection.next_snapshot_time
                    })
                });
            if gate {
                self.shared.snapshot(slot)?;
            }
        }
        Ok(())
    }

    /// Deliver native source presentation events (donor `receiveEvents`).
    pub fn receive_source_events(&mut self, events: &[Q3SourcePresentationEvent]) -> Result<(), Q3ServerError> {
        for item in events {
            for slot in self.shared.peer_slots() {
                if self.shared.ended.get() {
                    return Ok(());
                }
                let gate = self
                    .shared
                    .peers
                    .borrow()
                    .iter()
                    .find(|peer| peer.slot == slot)
                    .map(|peer| peer.player.clone());
                let Some(player) = gate else {
                    continue;
                };
                if let Some(recipient) = &item.recipient {
                    if *recipient != player.actor {
                        continue;
                    }
                }
                match &item.event {
                    Q3SourceEvent::DropClient { client, reason } => {
                        if i64::from(*client) == i64::from(player.source_entity) {
                            self.shared.disconnect_client(&player.client, reason)?;
                        }
                    }
                    Q3SourceEvent::ServerCommand { client, text } => {
                        if *client == -1 || i64::from(*client) == i64::from(player.source_entity) {
                            self.shared.queue(slot, text)?;
                        }
                    }
                    Q3SourceEvent::Configstring { index, value } => {
                        let connected = self
                            .shared
                            .peers
                            .borrow()
                            .iter()
                            .find(|peer| peer.slot == slot)
                            .is_some_and(|peer| {
                                peer.conn
                                    .cell
                                    .connection()
                                    .is_some_and(|connection| connection.phase == Q3ServerPhase::Connected)
                            });
                        if !connected {
                            self.shared.configstring(slot, *index, value)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Run the master heartbeat (donor `heartbeat`). Like the Q2 server,
    /// transport failures panic: the donor throw propagates to the caller
    /// and the trait cannot fail.
    pub fn heartbeat(&mut self, now_milliseconds: u64) {
        if self.shared.ended.get() || matches!(self.shared.transport.address(), NetworkAddress::Ipx { .. }) {
            return;
        }
        let masters = self
            .shared
            .host
            .borrow_mut()
            .administration()
            .map(|administration| administration.masters())
            .unwrap_or_default();
        if let Some(heartbeat) = self.heartbeat.as_mut() {
            if let Err(error) = heartbeat.send(&masters, now_milliseconds as f64, true, true) {
                panic!("{error}");
            }
        }
    }

    /// Close the endpoint (donor `closeOwned`). Without a fallible close
    /// the shutdown report goes to the host like the Q2 server.
    pub fn close(&mut self) {
        if self.shared.ended.get() {
            return;
        }
        self.shared.ended.set(true);
        let mut failures: Vec<String> = Vec::new();
        for slot in self.shared.peer_slots() {
            let client = self
                .shared
                .peers
                .borrow()
                .iter()
                .find(|peer| peer.slot == slot)
                .map(|peer| peer.player.client.clone());
            if let Some(client) = client {
                if let Err(error) = self.shared.disconnect_client(&client, "Server shutdown") {
                    failures.push(error.to_string());
                }
            }
        }
        self.shared.peers.borrow_mut().clear();
        self.shared.transport.close();
        if !failures.is_empty() {
            self.shared
                .print(&format!("Q3 network shutdown failed: {}\n", failures.join("; ")));
        }
    }

    /// Bound address (donor `address`).
    #[must_use]
    pub fn address(&self) -> NetworkAddress {
        self.shared.transport.address()
    }

    /// Endpoint phase (donor `phase`).
    #[must_use]
    pub fn phase(&self) -> ApplicationNetworkPhase {
        if self.shared.ended.get() {
            ApplicationNetworkPhase::Closed
        } else {
            ApplicationNetworkPhase::Active
        }
    }

    /// Admitted players (donor `clients`).
    #[must_use]
    pub fn clients(&self) -> Vec<Q3ApplicationPlayer> {
        self.shared
            .peers
            .borrow()
            .iter()
            .map(|peer| peer.player.clone())
            .collect()
    }

    /// Recording source (donor `recordingSource`).
    pub fn recording_source(&self) -> Result<Q3ServerRecordingSource<H>, Q3ServerError> {
        if self.shared.ended.get() {
            return Err(Q3ServerError::Message("Q3 recording source is retired".to_string()));
        }
        let epoch = *self.shared.epoch.borrow();
        Ok(Q3ServerRecordingSource {
            host: self.shared.host.clone(),
            server_id: epoch.server_id,
            snapshot_server_bit: epoch.server_flags,
        })
    }

    /// Game output handle (donor `gameOutput`).
    #[must_use]
    pub fn game_output(&self) -> Q3ServerGameOutput<T, H> {
        Q3ServerGameOutput {
            shared: self.shared.clone(),
        }
    }

    /// Replace the world (donor `replaceWorld`).
    pub fn change_world(&mut self, host: H, rejected: &[(ClientId, String)]) -> Result<(), Q3ServerError> {
        if let WireAdmission::Unsupported { reasons } = host.supports_source_wire() {
            return Err(Q3ServerError::Message(reasons.join("; ")));
        }
        let mut refused: HashMap<i32, String> = HashMap::new();
        for (client, reason) in rejected {
            let slot = self
                .shared
                .peers
                .borrow()
                .iter()
                .find(|peer| i64::from(peer.slot) == i64::from(client.slot()) && peer.player.client == *client)
                .map(|peer| peer.slot);
            let Some(slot) = slot else {
                return Err(Q3ServerError::Message(
                    "Q3 map rejection does not identify one carried peer".to_string(),
                ));
            };
            if refused.contains_key(&slot) {
                return Err(Q3ServerError::Message(
                    "Q3 map rejection does not identify one carried peer".to_string(),
                ));
            }
            refused.insert(slot, reason.clone());
        }
        let carried: Vec<(i32, Q3ApplicationPlayer)> = self
            .shared
            .peer_slots()
            .iter()
            .filter(|slot| !refused.contains_key(slot))
            .filter_map(|slot| {
                self.shared
                    .peers
                    .borrow()
                    .iter()
                    .find(|peer| peer.slot == *slot)
                    .map(|peer| (*slot, host.carried_player(&peer.player.client)))
            })
            .collect();
        *self.shared.host.borrow_mut() = host;
        {
            let mut epoch = self.shared.epoch.borrow_mut();
            epoch.server_id += 1;
            epoch.restarted_server_id = epoch.server_id;
            epoch.server_flags = match epoch.server_flags {
                Q3SnapshotServerBit::Zero => Q3SnapshotServerBit::Four,
                Q3SnapshotServerBit::Four => Q3SnapshotServerBit::Zero,
            };
            epoch.checksum_feed = next_random(&self.shared.random).wrapping_shl(16) ^ next_random(&self.shared.random);
        }
        self.shared.pending.borrow_mut().clear();
        self.changed_world = true;
        for (client, reason) in rejected {
            self.shared.disconnect_client(client, reason)?;
        }
        for (slot, player) in carried {
            let userinfo = self
                .shared
                .peers
                .borrow()
                .iter()
                .find(|peer| peer.slot == slot)
                .map(|peer| peer.userinfo.clone());
            let Some(userinfo) = userinfo else {
                continue;
            };
            match q3_game_callback(|| self.shared.host.borrow_mut().userinfo(&player, &userinfo)) {
                Ok(()) => {}
                Err(error) => return Err(Q3ServerError::GameCallback(error)),
            }
            if let Some(peer) = self.shared.peers.borrow_mut().iter_mut().find(|peer| peer.slot == slot) {
                peer.player = player;
                if let Some(download) = peer.download.download.as_mut() {
                    download.close();
                }
                if let Some(connection) = peer.conn.cell.connection_mut() {
                    connection.delta_message = -1;
                    connection.phase = Q3ServerPhase::Connected;
                }
            }
        }
        Ok(())
    }

    /// Restart the source round (donor `restartSourceRound`). The donor
    /// `AggregateError` is unreachable: every retirement step is infallible
    /// in Rust, so the original failure returns as-is.
    pub fn restart_source_round(
        &mut self,
        run: impl FnOnce(&mut Q3RoundRestartScope<'_, T, H>),
        on_mutation: Option<impl FnOnce()>,
    ) -> Result<(), Q3ServerError> {
        if self.shared.ended.get() || self.shared.host.borrow_mut().source_round().is_none() {
            return Err(Q3ServerError::Message(
                "Q3 network host has no native source round restart".to_string(),
            ));
        }
        if let Some(binding) = self.shared.host.borrow_mut().source_round() {
            binding.preflight();
        }
        if self.shared.epoch.borrow().server_id == i32::MAX {
            return Err(Q3ServerError::Message("Q3 server id exhausted".to_string()));
        }
        let snapshot: Vec<(i32, ClientId)> = self
            .shared
            .peers
            .borrow()
            .iter()
            .map(|peer| (peer.slot, peer.player.client.clone()))
            .collect();
        let pending: HashSet<i32> = snapshot.iter().map(|(slot, _)| *slot).collect();
        {
            let mut epoch = self.shared.epoch.borrow_mut();
            epoch.server_id += 1;
            epoch.server_flags = match epoch.server_flags {
                Q3SnapshotServerBit::Zero => Q3SnapshotServerBit::Four,
                Q3SnapshotServerBit::Four => Q3SnapshotServerBit::Zero,
            };
        }
        self.shared.pending.borrow_mut().clear();
        let mut scope = Q3RoundRestartScope {
            network: self,
            snapshot,
            pending,
            bound: false,
            failed: None,
            closed: false,
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            if let Some(on_mutation) = on_mutation {
                on_mutation();
            }
            let epoch = *scope.network.shared.epoch.borrow();
            scope
                .network
                .shared
                .host
                .borrow_mut()
                .prepare(epoch.checksum_feed, epoch.server_id, None);
            scope.current();
            run(&mut scope);
            scope.current();
            if !scope.bound || !scope.pending.is_empty() {
                panic!("Q3 source restart did not bind and reconnect every network client");
            }
        }));
        scope.closed = true;
        match outcome {
            Ok(()) => Ok(()),
            Err(payload) => {
                if let Some(failed) = scope.failed.take() {
                    scope.network.shared.ended.set(true);
                    scope.network.shared.pending.borrow_mut().clear();
                    scope.network.shared.peers.borrow_mut().clear();
                    scope.network.shared.transport.close();
                    return Err(failed);
                }
                scope.network.shared.ended.set(true);
                scope.network.shared.pending.borrow_mut().clear();
                scope.network.shared.peers.borrow_mut().clear();
                scope.network.shared.transport.close();
                Err(Q3ServerError::Message(panic_message(payload)))
            }
        }
    }
}

impl<T, H> ApplicationNetwork for Q3ServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q3ApplicationServerHost + 'static,
{
    fn heartbeat(&mut self, now_milliseconds: u64) {
        self.heartbeat(now_milliseconds);
    }

    fn role(&self) -> ApplicationNetworkRole {
        ApplicationNetworkRole::Server
    }

    fn phase(&self) -> ApplicationNetworkPhase {
        self.phase()
    }

    fn wire(&self) -> WireSelection {
        WireSelection::Source {
            protocol: ProtocolIdentity::Q3,
        }
    }

    fn poll(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, ApplicationNetworkError> {
        self.poll(now_milliseconds).map_err(Q3ServerError::into_network)
    }

    fn submit(&mut self, commands: &[ActorCommand], now_milliseconds: u64) -> Result<(), ApplicationNetworkError> {
        self.submit(commands, now_milliseconds)
            .map_err(Q3ServerError::into_network)
    }

    fn publish(
        &mut self,
        output: &SimulationOutput,
        events: &[NetworkPresentationEvent],
        now_milliseconds: u64,
    ) -> Result<(), ApplicationNetworkError> {
        self.publish(output, events, now_milliseconds)
            .map_err(Q3ServerError::into_network)
    }

    fn close(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::time::{FrameContext, FramePhase, SourceTime};
    use qa_net::common::commands::{CommandSource, UserCommand};
    use qa_net::common::endpoint::ipv4_address;
    use qa_net::q3_net::{encode_connect, Gamestate, Q3PlayerState, Q3Product, Q3PureServer, Q3ServerRate};
    use qa_net::services::admin::{RconResult, ServerAdministration};
    use qa_world::session::WorldSnapshot;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::super::q3_types::{
        Q3ApplicationAdmissionSurface, Q3ApplicationSnapshot, Q3ConfigstringFn, Q3SourceRoundBinding,
    };

    /// Mock datagram transport with scripted inbound and captured outbound.
    #[derive(Clone)]
    struct MockTransport {
        inner: Arc<Mutex<MockTransportInner>>,
    }

    struct MockTransportInner {
        address: NetworkAddress,
        inbound: VecDeque<ReceiveEvent<NetworkAddress>>,
        outbound: Vec<(NetworkAddress, Vec<u8>)>,
        closed: bool,
    }

    impl MockTransport {
        fn new(address: NetworkAddress) -> Self {
            Self {
                inner: Arc::new(Mutex::new(MockTransportInner {
                    address,
                    inbound: VecDeque::new(),
                    outbound: Vec::new(),
                    closed: false,
                })),
            }
        }

        fn queue(&self, from: NetworkAddress, payload: Vec<u8>) {
            self.inner
                .lock()
                .expect("inbound")
                .inbound
                .push_back(ReceiveEvent::Packet {
                    from,
                    payload,
                    received_at: 0.0,
                });
        }

        fn sent(&self) -> Vec<(NetworkAddress, Vec<u8>)> {
            self.inner.lock().expect("outbound").outbound.clone()
        }

        fn sent_texts(&self) -> Vec<(NetworkAddress, String)> {
            self.sent()
                .iter()
                .map(|(to, bytes)| (to.clone(), String::from_utf8_lossy(bytes).into_owned()))
                .collect()
        }

        fn is_closed(&self) -> bool {
            self.inner.lock().expect("closed").closed
        }
    }

    impl DatagramTransport for MockTransport {
        type Address = NetworkAddress;

        fn address(&self) -> NetworkAddress {
            self.inner.lock().expect("address").address.clone()
        }

        fn closed(&self) -> bool {
            self.is_closed()
        }

        fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
            let mut inner = self.inner.lock().expect("outbound");
            if inner.closed {
                return Ok(false);
            }
            inner.outbound.push((to.clone(), payload.to_vec()));
            Ok(true)
        }

        fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
            Ok(self.inner.lock().expect("inbound").inbound.pop_front())
        }

        fn subscribe_readable(&self, _listener: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
            Ok(0)
        }

        fn unsubscribe(&self, _token: u64) {}

        fn close(&self) {
            self.inner.lock().expect("close").closed = true;
        }
    }

    /// Mock admission surface.
    #[derive(Debug, Clone)]
    struct MockAdmissionSurface {
        private_clients: i32,
        private_password: String,
        reconnect_limit_seconds: i32,
        minimum_ping: f32,
        maximum_ping: f32,
        demo_restricted: bool,
        enabled: bool,
        game_directory: String,
        strict_auth: String,
        flood_protect: bool,
    }

    impl Default for MockAdmissionSurface {
        fn default() -> Self {
            Self {
                private_clients: 0,
                private_password: String::new(),
                reconnect_limit_seconds: 3,
                minimum_ping: 0.0,
                maximum_ping: 0.0,
                demo_restricted: false,
                enabled: true,
                game_directory: String::new(),
                strict_auth: "1".to_string(),
                flood_protect: true,
            }
        }
    }

    impl Q3ApplicationAdmissionSurface for MockAdmissionSurface {
        fn private_clients(&self) -> i32 {
            self.private_clients
        }

        fn private_password(&self) -> String {
            self.private_password.clone()
        }

        fn reconnect_limit_seconds(&self) -> i32 {
            self.reconnect_limit_seconds
        }

        fn minimum_ping(&self) -> f32 {
            self.minimum_ping
        }

        fn maximum_ping(&self) -> f32 {
            self.maximum_ping
        }

        fn demo_restricted(&self) -> bool {
            self.demo_restricted
        }

        fn enabled(&self) -> bool {
            self.enabled
        }

        fn game_directory(&self) -> String {
            self.game_directory.clone()
        }

        fn strict_auth(&self) -> String {
            self.strict_auth.clone()
        }

        fn flood_protect(&self) -> bool {
            self.flood_protect
        }
    }

    /// Mock administration surface.
    #[derive(Debug, Clone, Default)]
    struct MockAdministration {
        password: String,
        rejects_all: bool,
        masters: Vec<NetworkAddress>,
        executed: Vec<String>,
        records: Vec<RconResult>,
    }

    impl ServerAdministration for MockAdministration {
        fn rcon_password(&self) -> String {
            self.password.clone()
        }

        fn execute(&mut self, command: &str, output: &mut dyn FnMut(&str)) {
            self.executed.push(command.to_string());
            output("ok\n");
        }

        fn rejects(&self, _address: &NetworkAddress) -> bool {
            self.rejects_all
        }

        fn masters(&self) -> Vec<NetworkAddress> {
            self.masters.clone()
        }

        fn record(&mut self, _address: &NetworkAddress, result: RconResult) {
            self.records.push(result);
        }
    }

    /// Mock source round binding.
    #[derive(Debug, Clone)]
    struct MockRound {
        preflights: usize,
        rebinds: usize,
        reconnects: Vec<ClientId>,
        reject_reason: Option<String>,
        change_identity: bool,
    }

    impl MockRound {
        fn new() -> Self {
            Self {
                preflights: 0,
                rebinds: 0,
                reconnects: Vec::new(),
                reject_reason: None,
                change_identity: false,
            }
        }
    }

    impl Q3SourceRoundBinding for MockRound {
        fn preflight(&mut self) {
            self.preflights += 1;
        }

        fn rebind(&mut self) {
            self.rebinds += 1;
        }

        fn reconnect(
            &mut self,
            client: &ClientId,
            _userinfo: &str,
            _last_command: &WireUserCommand,
        ) -> Q3ApplicationAdmission {
            self.reconnects.push(client.clone());
            if let Some(reason) = &self.reject_reason {
                return Q3ApplicationAdmission::Rejected { reason: reason.clone() };
            }
            let owner = IdentityOwner::create("q3-round-test").expect("owner");
            let mut player = Q3ApplicationPlayer {
                client: client.clone(),
                actor: owner.actor(client.slot(), 0),
                source_entity: client.slot(),
            };
            if self.change_identity {
                player.client = owner.client(client.slot() + 100, 0);
            }
            Q3ApplicationAdmission::Accepted { player }
        }
    }

    /// Mock server host.
    struct MockHost {
        owner: IdentityOwner,
        max_clients: u32,
        supported: Vec<String>,
        prepares: Vec<(i32, i32)>,
        prepare_broadcasts: Vec<(u32, String)>,
        pure_server: Q3PureServer,
        rate: Q3ServerRate,
        time: i32,
        occupied: Vec<i32>,
        admit_reject: Option<String>,
        admitted: Vec<(i32, u16, String)>,
        disconnects: Vec<(ClientId, String)>,
        disconnect_panics: bool,
        game_state: Gamestate,
        snapshot: Q3ApplicationSnapshot,
        begin_returns: bool,
        inputs: Vec<(ClientId, u32)>,
        input_returns: Option<ActorCommand>,
        input_panics: bool,
        commands: Vec<(ClientId, String, Vec<String>)>,
        command_panics: bool,
        userinfos: Vec<(ClientId, String)>,
        status_response: Option<String>,
        prints: Vec<String>,
        admission_surface: Option<MockAdmissionSurface>,
        administration: Option<MockAdministration>,
        round: Option<MockRound>,
    }

    impl MockHost {
        fn new() -> Self {
            let owner = IdentityOwner::create("q3-server-test").expect("owner");
            Self {
                owner,
                max_clients: 8,
                supported: Vec::new(),
                prepares: Vec::new(),
                prepare_broadcasts: Vec::new(),
                pure_server: Q3PureServer {
                    enabled: false,
                    checksum_feed: 0,
                    checksum_feed_server_id: 0,
                    cgame_checksum: None,
                    ui_checksum: None,
                    loaded_pure_checksums: Vec::new(),
                },
                rate: Q3ServerRate {
                    rate: 25000,
                    max_rate: 0,
                    snapshot_msec: 50,
                    local: false,
                    force_lan: false,
                    lan: true,
                },
                time: 1000,
                occupied: Vec::new(),
                admit_reject: None,
                admitted: Vec::new(),
                disconnects: Vec::new(),
                disconnect_panics: false,
                game_state: Gamestate {
                    command_sequence: 0,
                    entries: Vec::new(),
                    client_number: 0,
                    checksum_feed: 0,
                },
                snapshot: Q3ApplicationSnapshot {
                    player: Q3PlayerState::new(Q3Product::Base),
                    area_mask: Vec::new(),
                    entities: Vec::new(),
                },
                begin_returns: true,
                inputs: Vec::new(),
                input_returns: None,
                input_panics: false,
                commands: Vec::new(),
                command_panics: false,
                userinfos: Vec::new(),
                status_response: None,
                prints: Vec::new(),
                admission_surface: Some(MockAdmissionSurface::default()),
                administration: None,
                round: None,
            }
        }

        fn player_for(&self, client: &ClientId) -> Q3ApplicationPlayer {
            Q3ApplicationPlayer {
                client: client.clone(),
                actor: self.owner.actor(client.slot(), 0),
                source_entity: client.slot(),
            }
        }
    }

    impl Q3ApplicationServerHost for MockHost {
        fn admission(&self) -> Option<&dyn Q3ApplicationAdmissionSurface> {
            self.admission_surface
                .as_ref()
                .map(|surface| surface as &dyn Q3ApplicationAdmissionSurface)
        }

        fn administration(&mut self) -> Option<&mut dyn ServerAdministration> {
            self.administration
                .as_mut()
                .map(|surface| surface as &mut dyn ServerAdministration)
        }

        fn source_round(&mut self) -> Option<&mut dyn Q3SourceRoundBinding> {
            self.round.as_mut().map(|round| round as &mut dyn Q3SourceRoundBinding)
        }

        fn product(&self) -> Q3Product {
            Q3Product::Base
        }

        fn max_clients(&self) -> u32 {
            self.max_clients
        }

        fn prepare(&mut self, checksum_feed: i32, server_id: i32, configstring: Option<Q3ConfigstringFn<'_>>) {
            self.prepares.push((checksum_feed, server_id));
            if let Some(emit) = configstring {
                for (index, value) in self.prepare_broadcasts.clone() {
                    emit(index, &value);
                }
            }
        }

        fn pure(&self, _server_id: i32, _checksum_feed_server_id: Option<i32>) -> Q3PureServer {
            self.pure_server.clone()
        }

        fn downloads_enabled(&self) -> bool {
            true
        }

        fn open_download(&mut self, _name: &str) -> Option<Box<dyn qa_net::q3_net::Q3DownloadReadFile>> {
            None
        }

        fn rate(&self, _player: &Q3ApplicationPlayer) -> Q3ServerRate {
            self.rate
        }

        fn supports_source_wire(&self) -> WireAdmission {
            if self.supported.is_empty() {
                WireAdmission::Supported
            } else {
                WireAdmission::Unsupported {
                    reasons: self.supported.clone(),
                }
            }
        }

        fn time(&self) -> i32 {
            self.time
        }

        fn occupied_slots(&self) -> Vec<i32> {
            self.occupied.clone()
        }

        fn admit(&mut self, request: &Q3AcceptedConnect) -> Q3ApplicationAdmission {
            self.admitted
                .push((request.challenge, request.qport, request.userinfo.clone()));
            if let Some(reason) = &self.admit_reject {
                return Q3ApplicationAdmission::Rejected { reason: reason.clone() };
            }
            let client = self.owner.client(request.slot as u32, 0);
            Q3ApplicationAdmission::Accepted {
                player: self.player_for(&client),
            }
        }

        fn carried_player(&self, client: &ClientId) -> Q3ApplicationPlayer {
            Q3ApplicationPlayer {
                client: client.clone(),
                actor: self.owner.actor(client.slot(), 1),
                source_entity: client.slot(),
            }
        }

        fn disconnect(&mut self, player: &Q3ApplicationPlayer, reason: &str) {
            if self.disconnect_panics {
                panic!("disconnect boom");
            }
            self.disconnects.push((player.client.clone(), reason.to_string()));
        }

        fn game_state(&self, _player: &Q3ApplicationPlayer, _server_id: i32) -> Gamestate {
            self.game_state.clone()
        }

        fn snapshot(&self, _player: &Q3ApplicationPlayer) -> Q3ApplicationSnapshot {
            self.snapshot.clone()
        }

        fn begin(&mut self, _player: &Q3ApplicationPlayer, _command: &WireUserCommand) -> bool {
            self.begin_returns
        }

        fn input(
            &mut self,
            player: &Q3ApplicationPlayer,
            _command: &WireUserCommand,
            sequence: u32,
        ) -> Option<ActorCommand> {
            if self.input_panics {
                panic!("input boom");
            }
            self.inputs.push((player.client.clone(), sequence));
            self.input_returns.clone()
        }

        fn command(&mut self, player: &Q3ApplicationPlayer, name: &str, args: &[String]) {
            if self.command_panics {
                panic!("command boom");
            }
            self.commands
                .push((player.client.clone(), name.to_string(), args.to_vec()));
        }

        fn userinfo(&mut self, player: &Q3ApplicationPlayer, value: &str) {
            self.userinfos.push((player.client.clone(), value.to_string()));
        }

        fn status(&self, _challenge: &str, _detailed: bool) -> Option<String> {
            self.status_response.clone()
        }

        fn print(&mut self, text: &str) {
            self.prints.push(text.to_string());
        }
    }

    /// Scripted random source shared with the test.
    #[derive(Clone)]
    struct ScriptRandom {
        values: Rc<RefCell<VecDeque<f64>>>,
    }

    impl ScriptRandom {
        fn new(values: Vec<f64>) -> Self {
            Self {
                values: Rc::new(RefCell::new(values.into())),
            }
        }

        fn consumer(&self) -> Box<dyn FnMut() -> f64> {
            let values = self.values.clone();
            Box::new(move || values.borrow_mut().pop_front().unwrap_or(0.0))
        }
    }

    fn server_address() -> NetworkAddress {
        ipv4_address([127, 0, 0, 1], 27960, false).expect("server address")
    }

    fn client_address() -> NetworkAddress {
        NetworkAddress::Loopback {
            id: "client".to_string(),
        }
    }

    fn oob(text: &str) -> Vec<u8> {
        let mut packet = vec![255, 255, 255, 255];
        packet.extend_from_slice(text.as_bytes());
        packet
    }

    fn connect_userinfo(challenge: i32, qport: u16) -> String {
        format!("\\protocol\\68\\challenge\\{challenge}\\qport\\{qport}\\name\\t")
    }

    fn test_output() -> SimulationOutput {
        SimulationOutput {
            snapshot: WorldSnapshot {
                frame: FrameContext {
                    frame: 1,
                    time: SourceTime::Milliseconds(100),
                    elapsed: SourceTime::Milliseconds(50),
                    phase: FramePhase::FrameEntry,
                },
                actors: Vec::new(),
                bodies: Vec::new(),
                inventories: Vec::new(),
            },
            events: Vec::new(),
        }
    }

    fn test_actor_command(owner: &IdentityOwner, actor_slot: u32) -> ActorCommand {
        ActorCommand {
            actor: owner.actor(actor_slot, 0),
            source: CommandSource::LocalSeat { seat: owner.seat(0) },
            sequence: 7,
            command: UserCommand::Q3 {
                server_time_milliseconds: 100.0,
                angle_words: [0.0; 3],
                buttons: 0.0,
                weapon: 0.0,
                forward_move: 0.0,
                right_move: 0.0,
                up_move: 0.0,
            },
            arsenal: None,
        }
    }

    fn test_network(
        host: MockHost,
        transport: &MockTransport,
        random: &ScriptRandom,
    ) -> Q3ServerNetwork<MockTransport, MockHost> {
        Q3ServerNetwork::new(Q3ServerNetworkOptions {
            transport: transport.clone(),
            host,
            random: random.consumer(),
            timeout_milliseconds: None,
            resolve_authorization: None,
        })
        .expect("network")
    }

    /// Admit one loopback peer and return its client.
    fn admit_peer(
        network: &mut Q3ServerNetwork<MockTransport, MockHost>,
        transport: &MockTransport,
        now: u64,
    ) -> ClientId {
        let from = client_address();
        transport.queue(from.clone(), encode_connect(&connect_userinfo(0, 1)).expect("connect"));
        network.poll(now).expect("poll");
        assert!(
            transport
                .sent_texts()
                .iter()
                .any(|(_, text)| text.contains("connectResponse")),
            "expected connectResponse, got {:?}",
            transport.sent_texts()
        );
        assert_eq!(network.clients().len(), 1);
        network.clients()[0].client.clone()
    }

    #[test]
    fn js_int32_folds_like_javascript() {
        assert_eq!(js_int32(0.5), 0);
        assert_eq!(js_int32(-0.5), 0);
        assert_eq!(js_int32(f64::NAN), 0);
        assert_eq!(js_int32(f64::INFINITY), 0);
        assert_eq!(js_int32(4_294_967_297.0), 1);
        assert_eq!(js_int32(-1.0), -1);
        assert_eq!(js_int32(2_147_483_648.0), i32::MIN);
    }

    #[test]
    fn requires_supported_wire() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut host = MockHost::new();
        host.supported = vec!["no q3".to_string(), "no sauce".to_string()];
        let error = match Q3ServerNetwork::new(Q3ServerNetworkOptions {
            transport: transport.clone(),
            host,
            random: random.consumer(),
            timeout_milliseconds: None,
            resolve_authorization: None,
        }) {
            Err(error) => error,
            Ok(_) => panic!("unsupported wire admitted"),
        };
        assert_eq!(error.to_string(), "no q3; no sauce");
    }

    #[test]
    fn checksum_feed_folds_two_randoms() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![1.0, 2.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        network.poll(1000).expect("poll");
        let prepares = network.shared.host.borrow().prepares.clone();
        assert_eq!(prepares, vec![((1 << 16) ^ 2, 1)]);
        assert!(random.values.borrow().is_empty());
    }

    #[test]
    fn loopback_connect_admits_peer() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let client = admit_peer(&mut network, &transport, 1000);
        assert_eq!(client.slot(), 0);
        let host = network.shared.host.borrow();
        assert_eq!(host.admitted.len(), 1);
        assert_eq!(host.admitted[0].1, 1);
        assert!(host.admitted[0].2.contains("\\protocol\\68"));
        assert_eq!(network.clients()[0].source_entity, 0);
    }

    #[test]
    fn ipv4_handshake_challenges_then_connects() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0, 0.0, 0.0]);
        let from = ipv4_address([192, 168, 1, 7], 27960, false).expect("client");
        let authority = from.clone();
        let mut network = Q3ServerNetwork::new(Q3ServerNetworkOptions {
            transport: transport.clone(),
            host: MockHost::new(),
            random: random.consumer(),
            timeout_milliseconds: None,
            resolve_authorization: Some(Box::new(move || Ok(authority.clone()))),
        })
        .expect("network");
        transport.queue(from.clone(), oob("getchallenge"));
        network.poll(1000).expect("challenge");
        let texts = transport.sent_texts();
        assert_eq!(texts.len(), 1, "one challenge reply, got {texts:?}");
        let challenge: i32 = texts[0]
            .1
            .split_whitespace()
            .nth(1)
            .expect("challenge")
            .parse()
            .expect("number");
        transport.queue(
            from.clone(),
            encode_connect(&connect_userinfo(challenge, 9)).expect("connect"),
        );
        network.poll(1000).expect("connect");
        assert_eq!(network.clients().len(), 1);
        assert!(transport
            .sent_texts()
            .iter()
            .any(|(_, text)| text.contains("connectResponse")));
    }

    #[test]
    fn admission_rejection_replies_with_reason() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut host = MockHost::new();
        host.admit_reject = Some("banned".to_string());
        let mut network = test_network(host, &transport, &random);
        transport.queue(
            client_address(),
            encode_connect(&connect_userinfo(0, 1)).expect("connect"),
        );
        network.poll(1000).expect("poll");
        assert!(network.clients().is_empty());
        assert!(transport.sent_texts().iter().any(|(_, text)| text.contains("banned")));
    }

    #[test]
    fn reconnect_evicts_previous_peer() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let first = admit_peer(&mut network, &transport, 1000);
        transport.queue(
            client_address(),
            encode_connect(&connect_userinfo(0, 1)).expect("connect"),
        );
        network.poll(5000).expect("reconnect");
        assert_eq!(network.clients().len(), 1);
        let host = network.shared.host.borrow();
        assert_eq!(host.disconnects, vec![(first, "reconnected".to_string())]);
        assert_eq!(host.admitted.len(), 2);
    }

    #[test]
    fn status_queries_reply() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut host = MockHost::new();
        host.status_response = Some("statusline".to_string());
        let mut network = test_network(host, &transport, &random);
        transport.queue(client_address(), oob("getstatus 17"));
        transport.queue(client_address(), oob("getinfo 18"));
        network.poll(1000).expect("poll");
        let texts = transport.sent_texts();
        assert_eq!(texts.len(), 2);
        assert!(texts.iter().all(|(_, text)| text.contains("statusline")));
    }

    #[test]
    fn status_queries_stay_silent_without_host() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        transport.queue(client_address(), oob("getstatus 17"));
        network.poll(1000).expect("poll");
        assert!(transport.sent().is_empty());
    }

    #[test]
    fn rcon_denied_records() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut host = MockHost::new();
        host.administration = Some(MockAdministration {
            password: "secret".to_string(),
            ..Default::default()
        });
        let mut network = test_network(host, &transport, &random);
        transport.queue(client_address(), oob("rcon wrong status"));
        network.poll(1000).expect("poll");
        let texts = transport.sent_texts();
        assert!(texts.iter().any(|(_, text)| text.contains("Bad rconpassword.")));
        let host = network.shared.host.borrow();
        let administration = host.administration.as_ref().expect("administration");
        assert_eq!(administration.records, vec![RconResult::Denied]);
        assert!(administration.executed.is_empty());
    }

    #[test]
    fn rcon_disabled_without_password() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut host = MockHost::new();
        host.administration = Some(MockAdministration::default());
        let mut network = test_network(host, &transport, &random);
        transport.queue(client_address(), oob("rcon anything status"));
        network.poll(1000).expect("poll");
        assert!(transport
            .sent_texts()
            .iter()
            .any(|(_, text)| text.contains("No rconpassword set")));
    }

    #[test]
    fn rcon_executes_with_password() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut host = MockHost::new();
        host.administration = Some(MockAdministration {
            password: "secret".to_string(),
            ..Default::default()
        });
        let mut network = test_network(host, &transport, &random);
        transport.queue(client_address(), oob("rcon secret status"));
        network.poll(1000).expect("poll");
        assert!(transport.sent_texts().iter().any(|(_, text)| text.contains("ok")));
        let host = network.shared.host.borrow();
        let administration = host.administration.as_ref().expect("administration");
        assert_eq!(administration.executed, vec!["status".to_string()]);
        assert_eq!(administration.records, vec![RconResult::Executed]);
    }

    #[test]
    fn rejected_addresses_skip_packets() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut host = MockHost::new();
        host.administration = Some(MockAdministration {
            rejects_all: true,
            ..Default::default()
        });
        let mut network = test_network(host, &transport, &random);
        transport.queue(client_address(), oob("getchallenge"));
        network.poll(1000).expect("poll");
        assert!(transport.sent().is_empty());
    }

    #[test]
    fn error_events_print() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        transport
            .inner
            .lock()
            .expect("inbound")
            .inbound
            .push_back(ReceiveEvent::Error {
                error: "cable cut".to_string(),
            });
        network.poll(1000).expect("poll");
        assert_eq!(network.shared.host.borrow().prints, vec!["cable cut".to_string()]);
    }

    /// Set the admitted peer's connection phase.
    fn set_phase(network: &mut Q3ServerNetwork<MockTransport, MockHost>, phase: Q3ServerPhase) {
        if let Some(connection) = network
            .shared
            .peers
            .borrow_mut()
            .iter_mut()
            .find(|peer| peer.slot == 0)
            .and_then(|peer| peer.conn.cell.connection_mut())
        {
            connection.phase = phase;
        }
    }

    /// Borrow the admitted peer's bindings without holding the peer
    /// borrow: commands re-borrow peers through drop callbacks.
    fn with_bindings<R>(
        network: &mut Q3ServerNetwork<MockTransport, MockHost>,
        run: impl FnOnce(&mut Q3ConnBindings<MockTransport, MockHost>) -> R,
    ) -> R {
        let bindings = {
            let mut peers = network.shared.peers.borrow_mut();
            let peer = peers.iter_mut().find(|peer| peer.slot == 0).expect("peer");
            let armed = peer
                .conn
                .cell
                .connection_mut()
                .map(|live| live as *mut Q3ServerConnection<'static>);
            let bindings = peer.conn.cell.adapter_mut() as *mut Q3ConnBindings<MockTransport, MockHost>;
            if let Some(armed) = armed {
                // SAFETY: test-only arming; single-threaded, no frame runs.
                unsafe {
                    (*bindings).connection = armed;
                }
            }
            bindings
        };
        // SAFETY: the peer vector is stable across the call (drops only
        // queue) and the adapter box never moves.
        run(unsafe { &mut *bindings })
    }

    /// Run one reliable command through the admitted peer's bindings.
    fn run_command(
        network: &mut Q3ServerNetwork<MockTransport, MockHost>,
        text: &str,
        client_ok: bool,
    ) -> Result<bool, Q3NetError> {
        with_bindings(network, |bindings| {
            let command = ReliableCommand {
                sequence: 1,
                text: text.to_string(),
            };
            bindings.command(&command, client_ok)
        })
    }

    #[test]
    fn command_dispatches_to_host() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let client = admit_peer(&mut network, &transport, 1000);
        assert!(run_command(&mut network, "say hello", true).expect("command"));
        let host = network.shared.host.borrow();
        assert_eq!(
            host.commands,
            vec![(client, "say".to_string(), vec!["hello".to_string()])]
        );
    }

    #[test]
    fn unknown_command_without_client_ok_is_ignored() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        assert!(run_command(&mut network, "say hello", false).expect("command"));
        assert!(network.shared.host.borrow().commands.is_empty());
    }

    #[test]
    fn userinfo_updates_peer_and_host() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let client = admit_peer(&mut network, &transport, 1000);
        assert!(run_command(&mut network, "userinfo \\name\\new", true).expect("command"));
        let userinfo = network
            .shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == 0)
            .map(|peer| peer.userinfo.clone());
        assert_eq!(userinfo.as_deref(), Some("\\name\\new"));
        let host = network.shared.host.borrow();
        assert_eq!(host.userinfos, vec![(client, "\\name\\new".to_string())]);
        assert!(host.commands.is_empty());
    }

    #[test]
    fn disconnect_command_defers_drop() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let client = admit_peer(&mut network, &transport, 1000);
        assert!(!run_command(&mut network, "disconnect", true).expect("command"));
        assert_eq!(network.clients().len(), 1);
        network.drain_deferred().expect("drain");
        assert!(network.clients().is_empty());
        assert_eq!(
            network.shared.host.borrow().disconnects,
            vec![(client, "disconnected".to_string())]
        );
    }

    #[test]
    fn download_begins_valid_names() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        assert!(run_command(&mut network, "download baseq3/pak0.pk3", true).expect("command"));
        let name = network
            .shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == 0)
            .and_then(|peer| peer.download.download.as_ref())
            .map(|download| download.name.clone());
        assert_eq!(name.as_deref(), Some("baseq3/pak0.pk3"));
    }

    #[test]
    fn download_rejects_unsafe_names() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let client = admit_peer(&mut network, &transport, 1000);
        assert!(!run_command(&mut network, "download ../evil.pk3", true).expect("command"));
        network.drain_deferred().expect("drain");
        assert!(network.clients().is_empty());
        assert_eq!(
            network.shared.host.borrow().disconnects,
            vec![(client, "Invalid download path".to_string())]
        );
    }

    #[test]
    fn download_truncates_long_names() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        let long = format!("baseq3/{}.pk3EXTRA", "p".repeat(52));
        assert!(run_command(&mut network, &format!("download {long}"), true).expect("command"));
        let name = network
            .shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == 0)
            .and_then(|peer| peer.download.download.as_ref())
            .map(|download| download.name.clone())
            .expect("name");
        assert_eq!(name.chars().count(), DOWNLOAD_NAME_CHARS);
    }

    #[test]
    fn nextdl_stopdl_and_donedl_round_trip() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        run_command(&mut network, "download baseq3/pak0.pk3", true).expect("download");
        assert!(run_command(&mut network, "nextdl 0", true).expect("nextdl"));
        assert!(run_command(&mut network, "stopdl", true).expect("stopdl"));
        let sent_before = transport.sent().len();
        assert!(run_command(&mut network, "donedl", true).expect("donedl"));
        assert!(transport.sent().len() > sent_before, "donedl sends the gamestate");
    }

    #[test]
    fn donedl_skips_gamestate_when_active() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        set_phase(&mut network, Q3ServerPhase::Active);
        let sent_before = transport.sent().len();
        assert!(run_command(&mut network, "donedl", true).expect("donedl"));
        assert_eq!(transport.sent().len(), sent_before);
    }

    #[test]
    fn cp_ignored_when_pure_disabled() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        assert!(run_command(&mut network, "cp 1 @ 2", true).expect("cp"));
        assert_eq!(network.clients().len(), 1);
    }

    #[test]
    fn vdr_clears_pure_state() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        if let Some(connection) = network
            .shared
            .peers
            .borrow_mut()
            .iter_mut()
            .find(|peer| peer.slot == 0)
            .and_then(|peer| peer.conn.cell.connection_mut())
        {
            connection.pure_authentic = true;
            connection.got_pure_command = true;
        }
        assert!(run_command(&mut network, "vdr", true).expect("vdr"));
        let pure = network
            .shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == 0)
            .and_then(|peer| peer.conn.cell.connection())
            .map(|connection| (connection.pure_authentic, connection.got_pure_command));
        assert_eq!(pure, Some((false, false)));
    }

    #[test]
    fn think_queues_input_for_poll() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let owner = IdentityOwner::create("q3-input-test").expect("owner");
        let mut host = MockHost::new();
        host.input_returns = Some(test_actor_command(&owner, 3));
        let mut network = test_network(host, &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        with_bindings(&mut network, |bindings| bindings.think(&WireUserCommand::default())).expect("think");
        let commands = network.poll(1000).expect("poll");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].sequence, 7);
        assert_eq!(network.shared.host.borrow().inputs.len(), 1);
        assert_eq!(network.shared.host.borrow().inputs[0].1, 0);
    }

    #[test]
    fn enter_world_falls_back_to_input() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        with_bindings(&mut network, |bindings| {
            bindings.enter_world(&WireUserCommand::default())
        })
        .expect("enter");
        assert_eq!(network.shared.host.borrow().inputs.len(), 1);
        network.shared.host.borrow_mut().begin_returns = false;
        with_bindings(&mut network, |bindings| {
            bindings.enter_world(&WireUserCommand::default())
        })
        .expect("enter");
        assert_eq!(network.shared.host.borrow().inputs.len(), 1);
    }

    #[test]
    fn game_callback_failure_aborts_poll() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut host = MockHost::new();
        host.command_panics = true;
        let mut network = test_network(host, &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        let failed = std::panic::catch_unwind(AssertUnwindSafe(|| run_command(&mut network, "say boom", true)));
        assert!(failed.is_err(), "game panic escapes the binding");
        transport.queue(client_address(), oob("getchallenge"));
        let error = network.poll(2000).expect_err("poll aborts");
        assert!(matches!(error, Q3ServerError::GameCallback(_)), "got {error:?}");
    }

    #[test]
    fn timeout_sweep_disconnects_idle_peers() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let client = admit_peer(&mut network, &transport, 1000);
        let output = test_output();
        network.publish(&output, &[], 2000).expect("fresh");
        assert_eq!(network.clients().len(), 1);
        network
            .publish(&output, &[], 1000 + DEFAULT_TIMEOUT_MILLISECONDS + 1)
            .expect("stale");
        assert!(network.clients().is_empty());
        assert_eq!(
            network.shared.host.borrow().disconnects,
            vec![(client, "timed out".to_string())]
        );
    }

    #[test]
    fn publish_sends_active_snapshots() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        set_phase(&mut network, Q3ServerPhase::Active);
        let sent_before = transport.sent().len();
        network.publish(&test_output(), &[], 1000).expect("publish");
        assert!(transport.sent().len() > sent_before, "snapshot datagram sent");
    }

    #[test]
    fn publish_skips_connected_snapshots() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        let sent_before = transport.sent().len();
        network.publish(&test_output(), &[], 1000).expect("publish");
        assert_eq!(transport.sent().len(), sent_before);
    }

    #[test]
    fn heartbeat_sends_to_masters() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let master = ipv4_address([203, 0, 113, 9], 27950, false).expect("master");
        let mut host = MockHost::new();
        host.administration = Some(MockAdministration {
            masters: vec![master.clone()],
            ..Default::default()
        });
        let mut network = test_network(host, &transport, &random);
        network.heartbeat(1000);
        network.heartbeat(1000);
        let to_master: Vec<_> = transport
            .sent()
            .iter()
            .filter(|(to, _)| *to == master)
            .cloned()
            .collect();
        assert_eq!(to_master.len(), 2, "forced beats send every time");
    }

    #[test]
    fn publish_heartbeat_is_interval_gated() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let master = ipv4_address([203, 0, 113, 9], 27950, false).expect("master");
        let mut host = MockHost::new();
        host.administration = Some(MockAdministration {
            masters: vec![master.clone()],
            ..Default::default()
        });
        let mut network = test_network(host, &transport, &random);
        network.publish(&test_output(), &[], 1000).expect("first");
        network.publish(&test_output(), &[], 2000).expect("second");
        let to_master = transport.sent().iter().filter(|(to, _)| *to == master).count();
        assert_eq!(to_master, 1);
    }

    #[test]
    fn ipx_skips_heartbeat_and_announces() {
        let ipx = NetworkAddress::Ipx {
            network: 1,
            node: [2, 3, 4, 5, 6, 7],
            port: 1000,
        };
        let transport = MockTransport::new(ipx);
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut host = MockHost::new();
        host.administration = Some(MockAdministration {
            masters: vec![server_address()],
            ..Default::default()
        });
        let mut network = test_network(host, &transport, &random);
        assert!(network
            .shared
            .host
            .borrow()
            .prints
            .iter()
            .any(|text| text.contains("IPX")));
        network.heartbeat(1000);
        network.publish(&test_output(), &[], 1000).expect("publish");
        assert!(transport.sent().is_empty());
    }

    #[test]
    fn close_reports_shutdown_failures() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut host = MockHost::new();
        host.disconnect_panics = true;
        let mut network = test_network(host, &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        network.close();
        assert_eq!(network.phase(), ApplicationNetworkPhase::Closed);
        assert!(transport.is_closed());
        assert!(network.poll(2000).expect("poll").is_empty());
        assert!(network
            .shared
            .host
            .borrow()
            .prints
            .iter()
            .any(|text| text.contains("Q3 network shutdown failed")));
    }

    #[test]
    fn recording_source_retires_on_close() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let source = network.recording_source().expect("source");
        assert_eq!(source.server_id, 1);
        assert_eq!(source.snapshot_server_bit, Q3SnapshotServerBit::Zero);
        network.close();
        let error = match network.recording_source() {
            Err(error) => error,
            Ok(_) => panic!("retired source returned"),
        };
        assert_eq!(error.to_string(), "Q3 recording source is retired");
    }

    #[test]
    fn change_world_carries_peers() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0, 3.0, 4.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let client = admit_peer(&mut network, &transport, 1000);
        network.change_world(MockHost::new(), &[]).expect("change");
        assert_eq!(network.clients().len(), 1);
        assert_eq!(network.clients()[0].client, client);
        assert_eq!(
            network.clients()[0].actor,
            network.shared.host.borrow().owner.actor(0, 1)
        );
        assert_eq!(network.shared.host.borrow().userinfos.len(), 1);
        network.poll(2000).expect("poll");
        let prepares = network.shared.host.borrow().prepares.clone();
        assert_eq!(prepares.last(), Some(&((3 << 16) ^ 4, 2)));
        let source = network.recording_source().expect("source");
        assert_eq!(source.server_id, 2);
        assert_eq!(source.snapshot_server_bit, Q3SnapshotServerBit::Four);
    }

    #[test]
    fn change_world_validates_rejections() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let client = admit_peer(&mut network, &transport, 1000);
        let owner = IdentityOwner::create("q3-bogus-test").expect("owner");
        let bogus = owner.client(9, 0);
        let error = network
            .change_world(MockHost::new(), &[(bogus, "nope".to_string())])
            .expect_err("bogus");
        assert_eq!(error.to_string(), "Q3 map rejection does not identify one carried peer");
        let error = network
            .change_world(
                MockHost::new(),
                &[(client.clone(), "a".to_string()), (client.clone(), "b".to_string())],
            )
            .expect_err("duplicate");
        assert_eq!(error.to_string(), "Q3 map rejection does not identify one carried peer");
        network
            .change_world(MockHost::new(), &[(client.clone(), "kicked".to_string())])
            .expect("kick");
        assert!(network.clients().is_empty());
        assert_eq!(
            network.shared.host.borrow().disconnects,
            vec![(client, "kicked".to_string())]
        );
    }

    #[test]
    fn restart_round_reconnects_every_client() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut host = MockHost::new();
        host.round = Some(MockRound::new());
        let mut network = test_network(host, &transport, &random);
        let client = admit_peer(&mut network, &transport, 1000);
        let mutated = Rc::new(Cell::new(false));
        let seen = mutated.clone();
        network
            .restart_source_round(
                |scope| {
                    assert_eq!(scope.clients(), vec![client.clone()]);
                    assert_eq!(scope.snapshot_server_bit(), Q3SnapshotServerBit::Four);
                    scope.bind_source();
                    assert!(scope.reconnect_client(&client));
                },
                Some(move || seen.set(true)),
            )
            .expect("restart");
        assert!(mutated.get());
        let host = network.shared.host.borrow();
        let round = host.round.as_ref().expect("round");
        assert_eq!((round.preflights, round.rebinds), (1, 1));
        assert_eq!(round.reconnects, vec![client]);
        let phase = network
            .shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == 0)
            .and_then(|peer| peer.conn.cell.connection())
            .map(|connection| connection.phase);
        assert_eq!(phase, Some(Q3ServerPhase::Active));
        let queued = network
            .shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == 0)
            .and_then(|peer| peer.conn.cell.connection())
            .map(|connection| connection.reliable.pending())
            .expect("pending");
        assert!(queued.iter().any(|command| command.text == "map_restart\n"));
    }

    #[test]
    fn restart_requires_binding_and_clients() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let error = network
            .restart_source_round(|_| {}, None::<fn()>)
            .expect_err("no binding");
        assert_eq!(error.to_string(), "Q3 network host has no native source round restart");
    }

    #[test]
    fn restart_failure_retires_network() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut host = MockHost::new();
        host.round = Some(MockRound::new());
        let mut network = test_network(host, &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        let error = network
            .restart_source_round(
                |scope| {
                    scope.bind_source();
                },
                None::<fn()>,
            )
            .expect_err("missing reconnect");
        assert_eq!(
            error.to_string(),
            "Q3 source restart did not bind and reconnect every network client"
        );
        assert_eq!(network.phase(), ApplicationNetworkPhase::Closed);
        assert!(transport.is_closed());
    }

    #[test]
    fn game_output_drops_and_broadcasts() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let client = admit_peer(&mut network, &transport, 1000);
        // Fresh peers read `connected`, so configstrings only queue after
        // the phase advances; advance it like the first snapshot would.
        set_phase(&mut network, Q3ServerPhase::Active);
        let output = network.game_output();
        output.send_server_command(-1, "say hi").expect("send");
        output.configstring(0, "value").expect("configstring");
        let queued = network
            .shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == 0)
            .and_then(|peer| peer.conn.cell.connection())
            .map(|connection| connection.reliable.pending())
            .expect("pending");
        assert!(queued.iter().any(|command| command.text == "say hi"));
        assert!(queued.iter().any(|command| command.text == "cs 0 \"value\""));
        output.drop_client(0, "bye").expect("drop");
        assert!(network.clients().is_empty());
        assert_eq!(
            network.shared.host.borrow().disconnects,
            vec![(client, "bye".to_string())]
        );
    }

    #[test]
    fn receive_source_events_route_by_client() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let client = admit_peer(&mut network, &transport, 1000);
        let actor = network.clients()[0].actor.clone();
        network
            .receive_source_events(&[Q3SourcePresentationEvent {
                recipient: None,
                event: Q3SourceEvent::ServerCommand {
                    client: -1,
                    text: "say all".to_string(),
                },
            }])
            .expect("broadcast");
        let queued = network
            .shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == 0)
            .and_then(|peer| peer.conn.cell.connection())
            .map(|connection| connection.reliable.pending())
            .expect("pending");
        assert!(queued.iter().any(|command| command.text == "say all"));
        let owner = IdentityOwner::create("q3-foreign-test").expect("owner");
        network
            .receive_source_events(&[Q3SourcePresentationEvent {
                recipient: Some(owner.actor(5, 0)),
                event: Q3SourceEvent::DropClient {
                    client: 0,
                    reason: "nope".to_string(),
                },
            }])
            .expect("foreign recipient ignored");
        assert_eq!(network.clients().len(), 1);
        network
            .receive_source_events(&[Q3SourcePresentationEvent {
                recipient: Some(actor),
                event: Q3SourceEvent::DropClient {
                    client: 0,
                    reason: "gone".to_string(),
                },
            }])
            .expect("drop");
        assert!(network.clients().is_empty());
        assert_eq!(
            network.shared.host.borrow().disconnects,
            vec![(client, "gone".to_string())]
        );
    }

    #[test]
    fn configstring_skips_connected_peers() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        network
            .receive_source_events(&[Q3SourcePresentationEvent {
                recipient: None,
                event: Q3SourceEvent::Configstring {
                    index: 1,
                    value: "x".to_string(),
                },
            }])
            .expect("configstring");
        let queued = network
            .shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == 0)
            .and_then(|peer| peer.conn.cell.connection())
            .map(|connection| connection.reliable.pending())
            .expect("pending");
        assert!(queued.is_empty());
    }

    #[test]
    fn configstring_chunks_long_values() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        admit_peer(&mut network, &transport, 1000);
        set_phase(&mut network, Q3ServerPhase::Active);
        let value = "v".repeat(2500);
        network
            .receive_source_events(&[Q3SourcePresentationEvent {
                recipient: None,
                event: Q3SourceEvent::Configstring { index: 2, value },
            }])
            .expect("configstring");
        let queued = network
            .shared
            .peers
            .borrow()
            .iter()
            .find(|peer| peer.slot == 0)
            .and_then(|peer| peer.conn.cell.connection())
            .map(|connection| connection.reliable.pending())
            .expect("pending");
        let texts: Vec<_> = queued.iter().map(|command| command.text.as_str()).collect();
        assert!(texts.iter().any(|text| text.starts_with("bcs0 2 ")));
        assert!(texts.iter().any(|text| text.starts_with("bcs1 2 ")));
        assert!(texts.iter().any(|text| text.starts_with("bcs2 2 ")));
    }

    #[test]
    fn queue_overflow_disconnects() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        let client = admit_peer(&mut network, &transport, 1000);
        for index in 0..70 {
            network.shared.queue(0, &format!("say {index}")).expect("queue");
        }
        assert!(network.clients().is_empty());
        assert_eq!(
            network.shared.host.borrow().disconnects,
            vec![(client, "Server command overflow".to_string())]
        );
    }

    #[test]
    fn endpoint_surface_reports() {
        let transport = MockTransport::new(server_address());
        let random = ScriptRandom::new(vec![0.0, 0.0]);
        let mut network = test_network(MockHost::new(), &transport, &random);
        assert_eq!(network.address(), server_address());
        assert_eq!(network.phase(), ApplicationNetworkPhase::Active);
        assert_eq!(network.role(), ApplicationNetworkRole::Server);
        assert!(matches!(network.wire(), WireSelection::Source { .. }));
        assert!(network.server_recording().is_none());
        network.submit(&[], 1000).expect("submit");
        network.close();
        network.close();
        assert_eq!(network.phase(), ApplicationNetworkPhase::Closed);
    }
}
