//! QuakeWorld server network (donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/qw-server.ts`).
//!
//! The donor is `async`; this sync port resolves every host call inline.
//! The donor runs connection, signon, and command-phase callbacks
//! reentrantly on one `this`; the port shares the host and peer table
//! through `Rc<RefCell<..>>` so the connectionless and signon adapters can
//! run the same logic synchronously. The command phase cannot reenter the
//! host through a second borrow, so the phase action/emit closures record an
//! ordered op queue (prints, deliveries, game commands, disconnects) that is
//! replayed after the phase returns; relative order matches the donor. Client
//! info reads inside the phase come from a snapshot taken at phase entry
//! (the donor game never mutates infos between its flush and the action).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::ClientId;
use qa_core::math::Vec3;
use qa_core::numeric::{native_atoi, NumericError};
use qa_net::common::commands::ActorCommand;
use qa_net::common::endpoint::{same_address, NetworkAddress};
use qa_net::common::session::{WireAdmission, WireSelection};
use qa_net::common::transport::{DatagramTransport, ReceiveEvent, TransportError};
use qa_net::msg::{MsgError, MsgWriter};
use qa_net::protocol::ProtocolIdentity;
use qa_net::q1_net::{
    decode_quake_world_client, parse_q1_token, quake_world_command_arguments, quake_world_info, quake_world_shutdown,
    write_quake_world_entities, write_quake_world_message, Q1NetError, Q1TokenDialect, Q1Tokenizer, Q1WireEntity,
    QuakeWorldChallenges, QuakeWorldChannel, QuakeWorldClientMessage, QuakeWorldCommandReplay,
    QuakeWorldConnectRequest, QuakeWorldConnectVerdict, QuakeWorldConnectionHost, QuakeWorldConnectionlessServer,
    QuakeWorldMasterHeartbeat, QuakeWorldMessage, QuakeWorldMove, QuakeWorldSignonHost, QuakeWorldSignonServer,
    QwSlotStat, QwSlotValue, QwText, QwUnit, SignonCommand,
};
use qa_net::q1_wide::QwProfile;
use qa_net::qw::QwUsercmd;
use qa_net::services::admin::{AdminError, ChatVerdict, SourceChatFlood};
use qa_net::services::downloads::DownloadSource;
use qa_world::movement::types::QwUserCommand;
use qa_world::session::SimulationOutput;
use thiserror::Error;

use super::qw_server_types::{
    QwApplicationAdmission, QwApplicationFrame, QwApplicationPlayer, QwApplicationServerHost, QwServerMessage,
    QwServerNetworkOptions,
};
use super::types::{
    ApplicationNetwork, ApplicationNetworkError, ApplicationNetworkPhase, ApplicationNetworkRole,
    NetworkPresentationEvent, Q3ConnectionCell,
};

/// QuakeWorld server network failure.
#[derive(Debug, Error)]
pub enum QwServerError {
    /// Donor failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] Q1NetError),
    /// Message coding failure.
    #[error(transparent)]
    Msg(#[from] MsgError),
    /// Transport failure.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// Numeric parse failure.
    #[error(transparent)]
    Numeric(#[from] NumericError),
    /// Chat policy failure.
    #[error(transparent)]
    Admin(#[from] AdminError),
}

impl QwServerError {
    /// Map into the application network error.
    fn into_network(self) -> ApplicationNetworkError {
        ApplicationNetworkError::Message(self.to_string())
    }
}

/// Native message size (donor `1450`).
const MESSAGE_SIZE: usize = 1450;

/// Reliable back buffers (donor `5`).
const RELIABLE_BACK_BUFFERS: usize = 5;

/// Default idle timeout in milliseconds (donor `65000`).
const DEFAULT_TIMEOUT_MILLISECONDS: u64 = 65_000;

/// Challenge table capacity (donor `1024`).
const CHALLENGE_CAPACITY: usize = 1024;

/// Sent entity frame (donor `Frame`).
#[derive(Debug, Clone)]
struct QwFrame {
    /// Channel sequence.
    sequence: u32,
    /// Entity states.
    states: Vec<Q1WireEntity>,
}

/// Ping sample (donor `PingFrame`).
#[derive(Debug, Clone)]
struct QwPingFrame {
    /// Channel sequence.
    sequence: u32,
    /// Send time.
    sent: f64,
    /// Round trip, negative while pending.
    ping: f64,
}

/// Signon cell over a signon adapter.
type QwSignonCell<T, H> = Q3ConnectionCell<QwSignonAdapter<T, H>, QuakeWorldSignonServer<'static>>;

/// Connected peer (donor `Peer`).
struct QwPeer<T, H> {
    /// Table identity.
    id: u64,
    /// Remote endpoint.
    remote: NetworkAddress,
    /// Bound player.
    player: QwApplicationPlayer,
    /// Toggle channel.
    channel: QuakeWorldChannel,
    /// Signon server, taken out while a command runs so its callbacks can
    /// borrow the shared table.
    signon: Option<QwSignonCell<T, H>>,
    /// Loss-recovery replay.
    replay: QuakeWorldCommandReplay,
    /// Entity baselines by number.
    baselines: HashMap<u32, Q1WireEntity>,
    /// Queued reliable blocks.
    reliable: Vec<Vec<u8>>,
    /// In-flight reliable length.
    reliable_length: usize,
    /// Sent entity frames by sequence.
    frames: HashMap<u32, QwFrame>,
    /// Activated by signon begin.
    active: bool,
    /// A reply is due.
    reply: bool,
    /// Requested delta base.
    delta: Option<u8>,
    /// Choked packets.
    choked: u32,
    /// Last receive time.
    last_received: f64,
    /// Print floor.
    message_level: i32,
    /// Reported loss percent.
    loss_percent: u8,
    /// Ping samples by sequence slot.
    ping_frames: HashMap<u32, QwPingFrame>,
    /// Chat flood policy.
    chat_flood: SourceChatFlood,
}

/// Host cell (donor `this.host`).
struct QwHostState<H> {
    /// Current host.
    inner: H,
}

/// Peer table with an identity counter.
struct QwPeerTable<T, H> {
    /// Next peer identity.
    next_id: u64,
    /// Live peers.
    peers: Vec<QwPeer<T, H>>,
}

/// Shared server state (donor `this` reachable from callbacks).
struct QwShared<T, H> {
    /// Datagram transport.
    transport: Rc<T>,
    /// Host generation cell.
    host: Rc<RefCell<QwHostState<H>>>,
    /// Peer table cell.
    peers: Rc<RefCell<QwPeerTable<T, H>>>,
}

impl<T, H> Clone for QwShared<T, H> {
    fn clone(&self) -> Self {
        Self {
            transport: self.transport.clone(),
            host: self.host.clone(),
            peers: self.peers.clone(),
        }
    }
}

/// Signon binding (donor `bind` result).
struct QwSignonBinding<T, H> {
    /// Signon server.
    signon: QwSignonCell<T, H>,
    /// Entity baselines by number.
    baselines: HashMap<u32, Q1WireEntity>,
}

/// JavaScript `ToInt32` for challenge randoms.
pub(crate) fn js_int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    (value.trunc().rem_euclid(4_294_967_296.0) as u32) as i32
}

impl<T, H> QwShared<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: QwApplicationServerHost + 'static,
{
    /// Print server text.
    fn print(&self, text: &str) {
        self.host.borrow_mut().inner.print(text);
    }

    /// Whether a peer is still carried.
    fn includes(&self, id: u64) -> bool {
        self.peers.borrow().peers.iter().any(|peer| peer.id == id)
    }

    /// Live peer identities in table order.
    fn peer_ids(&self) -> Vec<u64> {
        self.peers.borrow().peers.iter().map(|peer| peer.id).collect()
    }

    /// Peer identity for a client.
    fn id_for_client(&self, client: &ClientId) -> Option<u64> {
        self.peers
            .borrow()
            .peers
            .iter()
            .find(|peer| peer.player.client == *client)
            .map(|peer| peer.id)
    }

    /// Active client pings (donor `clientPings`).
    fn client_pings(&self) -> HashMap<ClientId, i64> {
        self.peers
            .borrow()
            .peers
            .iter()
            .filter(|peer| peer.active)
            .map(|peer| {
                let samples: Vec<f64> = peer
                    .ping_frames
                    .values()
                    .filter(|frame| frame.ping > 0.0)
                    .map(|frame| frame.ping)
                    .collect();
                let ping = if samples.is_empty() {
                    9999
                } else {
                    (samples.iter().sum::<f64>() / samples.len() as f64).trunc() as i64
                };
                (peer.player.client.clone(), ping)
            })
            .collect()
    }

    /// Bind a signon server and baselines to a player (donor `bind`).
    fn bind(&self, player: &QwApplicationPlayer, host: &mut H) -> Result<QwSignonBinding<T, H>, QwServerError> {
        let source = host.signon(player);
        let data = source.server_data();
        let valid = matches!(
            &data,
            QuakeWorldMessage::ServerData { protocol: QwProfile::Quakeworld, player_slot, .. }
            if u32::from(*player_slot) == player.slot
        ) && player.slot < host.max_clients();
        if !valid {
            return Err(QwServerError::Message(
                "Source signon does not describe an admitted native QW player".to_string(),
            ));
        }
        let adapter = QwSignonAdapter {
            source,
            shared: self.clone(),
            player: player.clone(),
        };
        let mut signon = Q3ConnectionCell::new(adapter);
        signon.build(|adapter| {
            // SAFETY: `Q3ConnectionCell::build` hands out the only lease of
            // its uniquely owned adapter; the connection never outlives the
            // cell, and `Drop` reclaims the adapter after the connection.
            let leased: &'static mut QwSignonAdapter<T, H> = unsafe { &mut *(adapter as *mut QwSignonAdapter<T, H>) };
            QuakeWorldSignonServer::new(leased, false)
        });
        let baselines = host
            .baselines(player)
            .into_iter()
            .map(|state| (state.number, state))
            .collect();
        Ok(QwSignonBinding { signon, baselines })
    }

    /// Take a peer's signon cell out of the table.
    fn take_signon(&self, id: u64) -> Option<QwSignonCell<T, H>> {
        self.peers
            .borrow_mut()
            .peers
            .iter_mut()
            .find(|peer| peer.id == id)
            .and_then(|peer| peer.signon.take())
    }

    /// Restore a signon cell, dropping it when the peer is gone.
    fn restore_signon(&self, id: u64, signon: QwSignonCell<T, H>) {
        if let Some(peer) = self.peers.borrow_mut().peers.iter_mut().find(|peer| peer.id == id) {
            peer.signon = Some(signon);
        }
    }

    /// Retire a peer's signon server (donor `peer.signon.close()`).
    fn close_signon(&self, id: u64) {
        if let Some(mut signon) = self.take_signon(id) {
            if let Some(server) = signon.connection_mut() {
                server.close();
            }
            self.restore_signon(id, signon);
        }
    }

    /// Remove a peer (donor `remove`).
    fn remove_peer(&self, id: u64, reason: &str) {
        let player = {
            let mut table = self.peers.borrow_mut();
            let Some(index) = table.peers.iter().position(|peer| peer.id == id) else {
                return;
            };
            let mut peer = table.peers.remove(index);
            if let Some(signon) = peer.signon.as_mut().and_then(|cell| cell.connection_mut()) {
                signon.close();
            }
            peer.player
        };
        self.host.borrow_mut().inner.disconnect(&player, reason);
    }

    /// Disconnect a client (donor `disconnectClient`).
    fn disconnect_client(&self, client: &ClientId, reason: &str) -> Result<bool, QwServerError> {
        let id = self.id_for_client(client);
        let Some(id) = id else {
            return Ok(false);
        };
        let (remote, last_received, disc) = {
            let peers = self.peers.borrow();
            let peer = peers.peers.iter().find(|peer| peer.id == id).expect("peer present");
            (peer.remote.clone(), peer.last_received, self.disconnect_bytes()?)
        };
        {
            let mut peers = self.peers.borrow_mut();
            let peer = peers.peers.iter_mut().find(|peer| peer.id == id).expect("peer present");
            let packet = peer.channel.transmit(&disc, last_received, true)?;
            self.transport.send(&remote, &packet)?;
        }
        self.remove_peer(id, reason);
        Ok(true)
    }

    /// Encode the disconnect unit (donor `bytes([{ kind: 'disconnect' }])`).
    fn disconnect_bytes(&self) -> Result<Vec<u8>, QwServerError> {
        let message = QwServerMessage {
            message: QuakeWorldMessage::Unit(QwUnit::Disconnect),
        };
        self.bytes(std::slice::from_ref(&message))
    }

    /// Encode server messages (donor `bytes`).
    fn bytes(&self, messages: &[QwServerMessage]) -> Result<Vec<u8>, QwServerError> {
        let mut buffer = MsgWriter::new(MESSAGE_SIZE, false);
        for message in messages {
            write_quake_world_message(&mut buffer, QwProfile::Quakeworld, &message.message)?;
        }
        Ok(buffer.bytes().to_vec())
    }

    /// Queue a reliable block (donor `enqueue`).
    fn enqueue(&self, id: u64, bytes: &[u8]) -> Result<(), QwServerError> {
        if bytes.is_empty() {
            return Ok(());
        }
        if bytes.len() > MESSAGE_SIZE {
            return Err(QwServerError::Message(
                "QW reliable block exceeds native message size".to_string(),
            ));
        }
        let mut peers = self.peers.borrow_mut();
        let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) else {
            return Ok(());
        };
        if let Some(last) = peer.reliable.last_mut() {
            if last.len() + bytes.len() <= MESSAGE_SIZE {
                last.extend_from_slice(bytes);
                return Ok(());
            }
        }
        if peer.reliable.len() >= RELIABLE_BACK_BUFFERS {
            return Err(QwServerError::Message("QW reliable back buffers overflow".to_string()));
        }
        peer.reliable.push(bytes.to_vec());
        Ok(())
    }

    /// Deliver one message, dropping the client when it cannot queue (donor
    /// `deliver`).
    fn deliver(&self, id: u64, message: &QwServerMessage) -> Result<(), QwServerError> {
        let gate = {
            let peers = self.peers.borrow();
            peers
                .peers
                .iter()
                .find(|peer| peer.id == id)
                .map(|peer| (peer.player.client.clone(), peer.message_level))
        };
        let Some((client, floor)) = gate else {
            return Ok(());
        };
        if let QuakeWorldMessage::Print { level, .. } = &message.message {
            if i32::from(*level) < floor {
                return Ok(());
            }
        }
        let bytes = self.bytes(std::slice::from_ref(message));
        match bytes {
            Ok(bytes) => {
                if let Err(error) = self.enqueue(id, &bytes) {
                    self.disconnect_client(&client, &error.to_string())?;
                }
            }
            Err(error) => {
                self.disconnect_client(&client, &error.to_string())?;
            }
        }
        Ok(())
    }

    /// Print to one peer (donor `clientPrint`).
    fn client_print(&self, id: u64, level: u8, text: &str) -> Result<(), QwServerError> {
        let message = QwServerMessage {
            message: QuakeWorldMessage::Print {
                level,
                text: text.to_string(),
            },
        };
        self.deliver(id, &message)
    }
}

/// Clamp a rate string into bytes per second (donor `rate`).
fn qw_rate(value: &str) -> Result<f64, NumericError> {
    Ok(f64::from(native_atoi(value).clamp(500, 10_000)))
}

/// Signon host adapter (donor `bind` callbacks).
struct QwSignonAdapter<T, H> {
    /// Application signon source.
    source: Box<dyn QuakeWorldSignonHost>,
    /// Shared server state.
    shared: QwShared<T, H>,
    /// Bound player.
    player: QwApplicationPlayer,
}

impl<T, H> QuakeWorldSignonHost for QwSignonAdapter<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: QwApplicationServerHost + 'static,
{
    fn server_data(&self) -> QuakeWorldMessage {
        self.source.server_data()
    }

    fn models(&self) -> Vec<String> {
        self.source.models()
    }

    fn sounds(&self) -> Vec<String> {
        self.source.sounds()
    }

    fn signon_buffers(&self) -> Vec<Vec<u8>> {
        self.source.signon_buffers()
    }

    fn accepts_map_checksum(&self, checksum: u32) -> bool {
        self.source.accepts_map_checksum(checksum)
    }

    fn spawn(&mut self, start_client: u32) -> Vec<Vec<u8>> {
        self.source.spawn(start_client)
    }

    fn begin(&mut self) {
        self.source.begin();
        if let Some(id) = self.shared.id_for_client(&self.player.client) {
            let mut peers = self.shared.peers.borrow_mut();
            if let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) {
                peer.active = true;
            }
        }
    }

    fn disconnect(&mut self, reason: &str) {
        // The trait cannot fail; the donor throw lands in the packet catch,
        // which prints and drops an already-removed owner, so print here.
        if let Err(error) = self.shared.disconnect_client(&self.player.client, reason) {
            self.shared.print(&error.to_string());
        }
    }

    fn open_download(&mut self, path: &str) -> Option<Box<dyn DownloadSource>> {
        self.source.open_download(path)
    }
}

/// Connectionless host adapter (donor constructor callbacks).
struct QwConnlessAdapter<T, H> {
    /// Shared server state.
    shared: QwShared<T, H>,
}

impl<T, H> QwConnlessAdapter<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: QwApplicationServerHost + 'static,
{
    /// Admit a connect request (donor `connect` past the guards). The flag
    /// reports whether the player must be disconnected first (donor bind
    /// catch); later failures print like the packet catch.
    fn admit_peer(
        &self,
        request: &QuakeWorldConnectRequest,
        player: &QwApplicationPlayer,
        now: f64,
    ) -> Result<(), (bool, String)> {
        let binding = {
            let mut host = self.shared.host.borrow_mut();
            match self.shared.bind(player, &mut host.inner) {
                Ok(binding) => binding,
                Err(error) => return Err((true, error.to_string())),
            }
        };
        let info = quake_world_info(&request.userinfo);
        let rate = match info.get("rate") {
            Some(value) => qw_rate(value).map_err(|error| (false, error.to_string()))?,
            None => 2500.0,
        };
        let message_level = native_atoi(info.get("msg").map(String::as_str).unwrap_or("0"));
        let channel = QuakeWorldChannel::new(
            qa_net::q1_net::QuakeWorldSide::Server,
            u32::from(request.qport),
            MESSAGE_SIZE,
            rate,
        )
        .map_err(|error| (false, error.to_string()))?;
        let chat_flood = SourceChatFlood::new(4, 4.0, 10.0).map_err(|error| (false, error.to_string()))?;
        let mut table = self.shared.peers.borrow_mut();
        let id = table.next_id;
        table.next_id += 1;
        table.peers.push(QwPeer {
            id,
            remote: request.from.clone(),
            player: player.clone(),
            channel,
            signon: Some(binding.signon),
            replay: QuakeWorldCommandReplay::default(),
            baselines: binding.baselines,
            reliable: Vec::new(),
            reliable_length: 0,
            frames: HashMap::new(),
            active: false,
            reply: false,
            delta: None,
            choked: 0,
            last_received: now,
            message_level,
            loss_percent: 0,
            ping_frames: HashMap::new(),
            chat_flood,
        });
        Ok(())
    }
}

impl<T, H> QuakeWorldConnectionHost for QwConnlessAdapter<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: QwApplicationServerHost + 'static,
{
    fn password(&self) -> String {
        self.shared
            .host
            .borrow()
            .inner
            .authentication()
            .map(|authentication| authentication.password())
            .unwrap_or_default()
    }

    fn spectator_password(&self) -> String {
        self.shared
            .host
            .borrow()
            .inner
            .authentication()
            .map(|authentication| authentication.spectator_password())
            .unwrap_or_default()
    }

    fn rcon_password(&self) -> String {
        self.shared
            .host
            .borrow_mut()
            .inner
            .administration()
            .map(|administration| administration.rcon_password())
            .unwrap_or_default()
    }

    fn high_characters(&self) -> bool {
        self.shared
            .host
            .borrow()
            .inner
            .authentication()
            .is_some_and(|authentication| authentication.high_characters())
    }

    fn blocked(&self, from: &NetworkAddress) -> bool {
        self.shared
            .host
            .borrow_mut()
            .inner
            .administration()
            .is_some_and(|administration| administration.blocked(from))
    }

    fn status(&self) -> String {
        self.shared
            .host
            .borrow_mut()
            .inner
            .administration()
            .map(|administration| administration.status())
            .unwrap_or_else(|| "\\hostname\\QuakeWorld\n".to_string())
    }

    fn log(&self, sequence: i64) -> Option<String> {
        self.shared
            .host
            .borrow_mut()
            .inner
            .administration()
            .and_then(|administration| administration.log(sequence))
    }

    fn execute_admin(&mut self, command: &str, write: &mut dyn FnMut(&str)) {
        let mut host = self.shared.host.borrow_mut();
        match host.inner.administration() {
            Some(administration) => administration.execute_admin(command, write),
            // The trait cannot fail; the donor throw lands in the packet
            // catch, which prints, so print here and emit no reply.
            None => host
                .inner
                .print("QW administrator commands require a source command owner"),
        }
    }

    fn connect(&mut self, request: &QuakeWorldConnectRequest, now: f64) -> QuakeWorldConnectVerdict {
        if !matches!(request.from, NetworkAddress::Ipv4 { .. } | NetworkAddress::Ipv6 { .. }) {
            return QuakeWorldConnectVerdict::Rejected {
                reason: "QW requires an IP endpoint".to_string(),
            };
        }
        let existing = self
            .shared
            .peers
            .borrow()
            .peers
            .iter()
            .find(|peer| same_address(&peer.remote, &request.from, false) && peer.channel.qport == request.qport)
            .map(|peer| (peer.id, peer.active));
        if let Some((id, active)) = existing {
            if !active {
                return QuakeWorldConnectVerdict::Duplicate;
            }
            self.shared.remove_peer(id, "Reconnecting");
        }
        let full = self.shared.peers.borrow().peers.len() >= self.shared.host.borrow().inner.max_clients() as usize;
        if full {
            return QuakeWorldConnectVerdict::Rejected {
                reason: "Server is full".to_string(),
            };
        }
        let admitted = self.shared.host.borrow_mut().inner.admit(request);
        let player = match admitted {
            QwApplicationAdmission::Rejected { reason } => {
                return QuakeWorldConnectVerdict::Rejected { reason };
            }
            QwApplicationAdmission::Accepted { player } => player,
        };
        // The verdict cannot fail; the donor throw sends no reply and lands
        // in the packet catch, so `Duplicate` (silent) plus a manual print
        // and disconnect matches the observable behavior.
        match self.admit_peer(request, &player, now) {
            Ok(()) => QuakeWorldConnectVerdict::Accepted,
            Err((disconnect, reason)) => {
                if disconnect {
                    self.shared.host.borrow_mut().inner.disconnect(&player, "Signon failed");
                }
                self.shared.print(&reason);
                QuakeWorldConnectVerdict::Duplicate
            }
        }
    }
}

/// Validate a host (donor `validate`). The slot bounds are `u32`, so the
/// donor integer and negativity checks collapse into the range check.
fn validate_host<H: QwApplicationServerHost>(host: &H) -> Result<(), QwServerError> {
    if let WireAdmission::Unsupported { reasons } = host.supports_source_wire() {
        return Err(QwServerError::Message(reasons.join("; ")));
    }
    if host.max_clients() < 1 || host.max_clients() > 32 {
        return Err(QwServerError::Message(
            "Native QW admits at most 32 players".to_string(),
        ));
    }
    Ok(())
}

/// QuakeWorld server network (donor `QwServerNetwork`).
pub struct QwServerNetwork<T, H> {
    /// Shared server state.
    shared: QwShared<T, H>,
    /// Connectionless server over the leaked adapter (`None` only in `Drop`).
    connectionless: Option<QuakeWorldConnectionlessServer<'static>>,
    /// Leaked connectionless adapter, reclaimed by `Drop`.
    conn_adapter: *mut QwConnlessAdapter<T, H>,
    /// Master heartbeat scheduler.
    heartbeat: QuakeWorldMasterHeartbeat,
    /// Idle timeout override.
    timeout: Option<u64>,
    /// Closed flag.
    ended: bool,
}

impl<T, H> QwServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: QwApplicationServerHost + 'static,
{
    /// Create a server network (donor constructor).
    pub fn new(options: QwServerNetworkOptions<T, H>) -> Result<Self, QwServerError> {
        validate_host(&options.host)?;
        let QwServerNetworkOptions {
            transport,
            host,
            mut random,
            timeout_milliseconds,
        } = options;
        let shared = QwShared {
            transport: Rc::new(transport),
            host: Rc::new(RefCell::new(QwHostState { inner: host })),
            peers: Rc::new(RefCell::new(QwPeerTable {
                next_id: 0,
                peers: Vec::new(),
            })),
        };
        let challenges = QuakeWorldChallenges::new(Box::new(move || js_int32(random()) as u32), CHALLENGE_CAPACITY);
        let conn_adapter = Box::into_raw(Box::new(QwConnlessAdapter { shared: shared.clone() }));
        // SAFETY: the adapter is uniquely owned (reclaimed by `Drop` after
        // the connectionless server is dropped), so the lease is exclusive.
        let connectionless = unsafe { QuakeWorldConnectionlessServer::new(&mut *conn_adapter, challenges) };
        Ok(Self {
            shared,
            connectionless: Some(connectionless),
            conn_adapter,
            heartbeat: QuakeWorldMasterHeartbeat::new(),
            timeout: timeout_milliseconds,
            ended: false,
        })
    }

    /// Handle one datagram (donor poll packet `try`).
    fn packet(&mut self, from: &NetworkAddress, payload: &[u8], now: f64) -> Result<(), QwServerError> {
        if payload.len() >= 4 && u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) == 0xffff_ffff {
            if let Err(error) = self.receive_connectionless(payload, from, now) {
                self.shared.print(&error.to_string());
            }
            return Ok(());
        }
        if payload.len() < 10 {
            return Ok(());
        }
        let qport = u16::from_le_bytes([payload[8], payload[9]]);
        let owner = self
            .shared
            .peers
            .borrow()
            .peers
            .iter()
            .find(|peer| peer.channel.qport == qport && same_address(&peer.remote, from, false))
            .map(|peer| peer.id);
        let Some(id) = owner else {
            return Ok(());
        };
        if let Err(error) = self.sequenced(id, from, payload, now) {
            let reason = error.to_string();
            self.shared.print(&reason);
            let client = self
                .shared
                .peers
                .borrow()
                .peers
                .iter()
                .find(|peer| peer.id == id)
                .map(|peer| peer.player.client.clone());
            if let Some(client) = client {
                self.shared.disconnect_client(&client, &reason)?;
            }
        }
        Ok(())
    }

    /// Handle a connectionless datagram (donor `connectionless.receive` plus
    /// reply sends).
    fn receive_connectionless(&mut self, payload: &[u8], from: &NetworkAddress, now: f64) -> Result<(), QwServerError> {
        let Some(connectionless) = self.connectionless.as_mut() else {
            return Ok(());
        };
        let replies = connectionless.receive(payload, from, now)?;
        for reply in &replies {
            self.shared.transport.send(from, reply)?;
        }
        Ok(())
    }

    /// Handle a sequenced datagram (donor poll packet past the guards).
    fn sequenced(&mut self, id: u64, from: &NetworkAddress, payload: &[u8], now: f64) -> Result<(), QwServerError> {
        let word = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
        let outgoing = self
            .shared
            .peers
            .borrow()
            .peers
            .iter()
            .find(|peer| peer.id == id)
            .map(|peer| peer.channel.outgoing_sequence());
        let Some(outgoing) = outgoing else {
            return Ok(());
        };
        let can_reply = i64::from(word % 0x8000_0000) >= outgoing;
        let blocked = self
            .shared
            .host
            .borrow_mut()
            .inner
            .administration()
            .is_some_and(|administration| administration.blocked(from));
        if blocked {
            return Ok(());
        }
        let delivery = {
            let mut peers = self.shared.peers.borrow_mut();
            let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) else {
                return Ok(());
            };
            peer.channel.receive(payload, now)?
        };
        let Some(delivery) = delivery else {
            return Ok(());
        };
        {
            let mut peers = self.shared.peers.borrow_mut();
            let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) else {
                return Ok(());
            };
            if let Some(frame) = peer.ping_frames.get_mut(&(delivery.acknowledged & 63)) {
                if frame.sequence == delivery.acknowledged {
                    frame.ping = now - frame.sent;
                }
            }
            let next = peer.channel.outgoing_sequence() as u32;
            peer.ping_frames.insert(
                next & 63,
                QwPingFrame {
                    sequence: next,
                    sent: now,
                    ping: -1.0,
                },
            );
            peer.remote = from.clone();
            peer.last_received = now;
            peer.reply = can_reply;
            peer.delta = None;
        }
        let messages = decode_quake_world_client(&delivery.payload, QwProfile::Quakeworld, delivery.sequence)?;
        for message in &messages {
            if !self.shared.includes(id) {
                break;
            }
            match message {
                QuakeWorldClientMessage::Delta { sequence } => {
                    let mut peers = self.shared.peers.borrow_mut();
                    if let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) {
                        peer.delta = Some(*sequence);
                    }
                }
                QuakeWorldClientMessage::Move { bundle } => {
                    self.client_move(id, bundle, delivery.dropped, delivery.sequence);
                }
                QuakeWorldClientMessage::StringCommand { text } => {
                    self.client_string(id, text, now)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Handle a movement bundle (donor `move` branch).
    fn client_move(&mut self, id: u64, bundle: &QuakeWorldMove, dropped: u32, sequence: u32) {
        let gate = self
            .shared
            .peers
            .borrow()
            .peers
            .iter()
            .find(|peer| peer.id == id)
            .map(|peer| (peer.player.clone(), peer.active));
        let Some((player, active)) = gate else {
            return;
        };
        if !active {
            return;
        }
        let paused = self.shared.host.borrow().inner.paused();
        let mut commands = Vec::new();
        {
            let mut peers = self.shared.peers.borrow_mut();
            let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) else {
                return;
            };
            peer.loss_percent = bundle.loss_percent;
            peer.replay.run(bundle, dropped, paused, |command| {
                commands.push(convert_usercmd(command));
            });
        }
        if !commands.is_empty() {
            self.shared
                .host
                .borrow_mut()
                .inner
                .command_group(&player, &commands, sequence);
        }
    }

    /// Handle a string command (donor `string-command` branch).
    fn client_string(&mut self, id: u64, text: &str, now: f64) -> Result<(), QwServerError> {
        let parts = quake_world_command_arguments(text);
        let mut names = parts.iter();
        let name = names.next().cloned();
        let args: Vec<String> = names.cloned().collect();
        if matches!(name.as_deref(), Some("drop" | "disconnect")) {
            let client = self
                .shared
                .peers
                .borrow()
                .peers
                .iter()
                .find(|peer| peer.id == id)
                .map(|peer| peer.player.client.clone());
            if let Some(client) = client {
                self.shared.disconnect_client(&client, "Client disconnected")?;
            }
            return Ok(());
        }
        // The sync host call cannot interleave, so the donor await
        // revalidation always holds. `None` doubles as absent (donor
        // `undefined`, falling back to the signon source) because preparation
        // cannot fail without a `Result`.
        let download = name.as_deref() == Some("download");
        let prepared = if download {
            let player = self
                .shared
                .peers
                .borrow()
                .peers
                .iter()
                .find(|peer| peer.id == id)
                .map(|peer| peer.player.clone());
            player.and_then(|player| {
                self.shared
                    .host
                    .borrow_mut()
                    .inner
                    .prepare_download(&player, args.first().map(String::as_str).unwrap_or(""))
            })
        } else {
            None
        };
        let present = download && prepared.is_some();
        let Some(mut signon) = self.shared.take_signon(id) else {
            return Ok(());
        };
        let result = signon
            .connection_mut()
            .map(|server| server.command(text, prepared, present));
        self.shared.restore_signon(id, signon);
        let Some(result) = result else {
            return Err(QwServerError::Message("QW signon used before bind".to_string()));
        };
        match result? {
            SignonCommand::Handled { messages } => {
                for bytes in &messages {
                    self.shared.enqueue(id, bytes)?;
                }
            }
            SignonCommand::GameCommand { .. } => {
                if let Some(name) = name {
                    self.run_phase(id, &name, &args, text, now)?;
                }
            }
        }
        Ok(())
    }
}

impl<T, H> QwShared<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: QwApplicationServerHost + 'static,
{
    /// Send one datagram, optionally with a frame (donor `send`).
    fn send(&self, id: u64, frame: Option<&QwApplicationFrame>, now: f64) -> Result<(), QwServerError> {
        {
            let mut peers = self.peers.borrow_mut();
            let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) else {
                return Ok(());
            };
            if !peer.reply {
                return Ok(());
            }
            peer.reply = false;
        }
        let paused = self.host.borrow().inner.paused();
        {
            let mut peers = self.peers.borrow_mut();
            let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) else {
                return Ok(());
            };
            if !paused && !peer.channel.can_packet(now) {
                peer.choked += 1;
                return Ok(());
            }
            if !peer.channel.has_reliable() && !peer.reliable.is_empty() {
                let bytes = peer.reliable.remove(0);
                peer.reliable_length = bytes.len();
                peer.channel.queue_reliable(&bytes)?;
            }
        }
        let mut buffer = MsgWriter::new(MESSAGE_SIZE, true);
        let sequence = {
            let peers = self.peers.borrow();
            let Some(peer) = peers.peers.iter().find(|peer| peer.id == id) else {
                return Ok(());
            };
            peer.channel.outgoing_sequence() as u32
        };
        let mut sent: Option<QwFrame> = None;
        if let Some(frame) = frame {
            match self.render_frame(id, frame, sequence, &mut buffer) {
                Ok(rendered) => sent = rendered,
                Err(reason) => {
                    self.print(&reason);
                    buffer.clear();
                }
            }
        }
        let (remote, packet) = {
            let mut peers = self.peers.borrow_mut();
            let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) else {
                return Ok(());
            };
            let packet = peer.channel.transmit(buffer.bytes(), now, paused)?;
            (peer.remote.clone(), packet)
        };
        self.transport.send(&remote, &packet)?;
        if let Some(frame) = sent {
            let stored = {
                let peers = self.peers.borrow();
                let Some(peer) = peers.peers.iter().find(|peer| peer.id == id) else {
                    return Ok(());
                };
                let reliable_bytes = if packet.get(3).is_some_and(|byte| byte & 128 != 0) {
                    peer.reliable_length
                } else {
                    0
                };
                packet.len() == 8 + reliable_bytes + buffer.bytes().len()
            };
            if stored {
                let mut peers = self.peers.borrow_mut();
                if let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) {
                    peer.frames.insert(sequence, frame);
                    peer.frames.retain(|key, _| sequence.wrapping_sub(*key) < 64);
                }
            }
        }
        Ok(())
    }

    /// Render frame bytes, returning the stored frame (donor `send` `try`).
    /// The ported frame already carries wire entities, so the donor
    /// `qwWireEntity` conversion has no counterpart here.
    fn render_frame(
        &self,
        id: u64,
        frame: &QwApplicationFrame,
        sequence: u32,
        buffer: &mut MsgWriter,
    ) -> Result<Option<QwFrame>, String> {
        let mut peers = self.peers.borrow_mut();
        let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) else {
            return Ok(None);
        };
        if peer.choked != 0 {
            // The donor masks through `MSG_WriteByte`, so wrap like `as u8`.
            let choke = QuakeWorldMessage::ChokeCount {
                count: peer.choked as u8,
            };
            write_quake_world_message(buffer, QwProfile::Quakeworld, &choke).map_err(|error| error.to_string())?;
            peer.choked = 0;
        }
        for message in &frame.messages {
            if let QuakeWorldMessage::Print { level, .. } = &message.message {
                if i32::from(*level) < peer.message_level {
                    continue;
                }
            }
            write_quake_world_message(buffer, QwProfile::Quakeworld, &message.message)
                .map_err(|error| error.to_string())?;
        }
        let states = frame.entities.clone();
        let previous = peer.delta.and_then(|delta| {
            peer.frames
                .values()
                .find(|frame| (frame.sequence & 255) as u8 == delta && sequence.wrapping_sub(frame.sequence) < 64)
                .map(|frame| (frame.sequence, frame.states.as_slice()))
        });
        let baselines = peer.baselines.clone();
        write_quake_world_entities(buffer, QwProfile::Quakeworld, &states, &baselines, previous)
            .map_err(|error| error.to_string())?;
        if buffer.overflowed() {
            return Err("QW frame datagram overflow".to_string());
        }
        Ok(Some(QwFrame { sequence, states }))
    }
}

impl<T, H> Drop for QwServerNetwork<T, H> {
    fn drop(&mut self) {
        self.connectionless = None;
        // SAFETY: paired with `Box::into_raw` in `new`; the connectionless
        // server (the only lease) was just dropped.
        unsafe {
            drop(Box::from_raw(self.conn_adapter));
        }
    }
}

/// Queued command-phase effect (donor phase action/emit bodies, replayed in
/// order after the phase returns).
enum QwPhaseOp {
    /// Print to a peer.
    Print { id: u64, level: u8, text: String },
    /// Deliver a message to a peer.
    Message { id: u64, message: QwServerMessage },
    /// Print server text.
    HostPrint { text: String },
    /// Deliver a game message to a recipient player.
    Deliver {
        recipient: QwApplicationPlayer,
        message: QwServerMessage,
    },
    /// Run a game command for the accepted player.
    Command { name: String, args: Vec<String> },
    /// Set a peer channel rate.
    SetPeerRate { id: u64, bps: f64 },
    /// Set a peer message level.
    SetPeerMsg { id: u64, level: i32 },
    /// Refresh peer fields from client info (donor `setinfo` tail).
    SetinfoRefresh { id: u64 },
    /// Disconnect a client.
    Disconnect { client: ClientId, reason: String },
}

/// Peer client-info snapshot for one phase.
struct QwPhasePeer {
    /// Table identity.
    id: u64,
    /// Bound player.
    player: QwApplicationPlayer,
    /// Activation flag.
    active: bool,
    /// Client info name.
    name: Option<String>,
    /// Client info team.
    team: Option<String>,
    /// Reported loss percent.
    loss: u8,
}

/// Convert a wire user command into an application command. The donor hands
/// the replayed wire commands to `commandGroup` directly; the ported types
/// split the wire and application shapes, so map fieldwise.
fn convert_usercmd(command: &QwUsercmd) -> QwUserCommand {
    QwUserCommand {
        milliseconds: i32::from(command.msec),
        angles: Vec3 {
            x: command.angles[0] as f32,
            y: command.angles[1] as f32,
            z: command.angles[2] as f32,
        },
        forward_move: f64::from(command.forwardmove),
        side_move: f64::from(command.sidemove),
        up_move: f64::from(command.upmove),
        buttons: i32::from(command.buttons),
        impulse: i32::from(command.impulse),
    }
}

impl<T, H> QwServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: QwApplicationServerHost + 'static,
{
    /// Run one game command in the command phase (donor `commandPhase`
    /// block). The phase closures cannot reenter the host through a second
    /// borrow, so they record effects that [`QwServerNetwork::replay_phase`]
    /// applies in order; the game holds no table handle, so membership and
    /// host identity cannot change mid-phase.
    fn run_phase(&mut self, id: u64, name: &str, args: &[String], text: &str, now: f64) -> Result<(), QwServerError> {
        let accepted = self
            .shared
            .peers
            .borrow()
            .peers
            .iter()
            .find(|peer| peer.id == id)
            .map(|peer| peer.player.clone());
        let Some(accepted_player) = accepted else {
            return Ok(());
        };
        let live = !self.ended && self.shared.includes(id);
        let paused = self.shared.host.borrow().inner.paused();
        let admin_status = self
            .shared
            .host
            .borrow_mut()
            .inner
            .administration()
            .map(|admin| admin.status());
        let snapshots: Vec<QwPhasePeer> = {
            let peers = self.shared.peers.borrow();
            let host = self.shared.host.borrow();
            peers
                .peers
                .iter()
                .map(|peer| {
                    let info = host.inner.client_info(&peer.player);
                    QwPhasePeer {
                        id: peer.id,
                        player: peer.player.clone(),
                        active: peer.active,
                        name: info.get("name").cloned(),
                        team: info.get("team").cloned(),
                        loss: peer.loss_percent,
                    }
                })
                .collect()
        };
        let pings = self.shared.client_pings();
        let max_clients = self.shared.host.borrow().inner.max_clients();
        let ops: Rc<RefCell<Vec<QwPhaseOp>>> = Rc::new(RefCell::new(Vec::new()));
        let emit_ops = ops.clone();
        let mut emit = |recipient: &QwApplicationPlayer, message: &QwServerMessage| {
            emit_ops.borrow_mut().push(QwPhaseOp::Deliver {
                recipient: recipient.clone(),
                message: message.clone(),
            });
        };
        let action_ops = ops.clone();
        let action_peers = self.shared.peers.clone();
        let sender = accepted_player.clone();
        let name = name.to_string();
        let args = args.to_vec();
        let text = text.to_string();
        let action = || {
            if !live {
                return;
            }
            Self::phase_action(
                &action_ops,
                &action_peers,
                &sender,
                id,
                &name,
                &args,
                &text,
                &snapshots,
                admin_status.as_deref(),
                &pings,
                max_clients,
                paused,
                now,
            );
        };
        self.shared
            .host
            .borrow_mut()
            .inner
            .command_phase(&accepted_player, &action, &mut emit);
        let ops = std::mem::take(&mut *ops.borrow_mut());
        self.replay_phase(ops, &accepted_player)
    }

    /// Run the phase action (donor `commandPhase` action). Takes no host
    /// handle, so the phase borrow is never reentered.
    #[allow(clippy::too_many_arguments)]
    fn phase_action(
        ops: &Rc<RefCell<Vec<QwPhaseOp>>>,
        peers: &Rc<RefCell<QwPeerTable<T, H>>>,
        sender: &QwApplicationPlayer,
        id: u64,
        name: &str,
        args: &[String],
        text: &str,
        snapshots: &[QwPhasePeer],
        admin_status: Option<&str>,
        pings: &HashMap<ClientId, i64>,
        max_clients: u32,
        paused: bool,
        now: f64,
    ) {
        /// Queue a phase failure (donor `failed`).
        fn failed(ops: &Rc<RefCell<Vec<QwPhaseOp>>>, sender: &QwApplicationPlayer, reason: String) {
            ops.borrow_mut().push(QwPhaseOp::HostPrint { text: reason.clone() });
            ops.borrow_mut().push(QwPhaseOp::Disconnect {
                client: sender.client.clone(),
                reason,
            });
        }
        if name == "rate" {
            let set = args.len() == 1;
            let bps = if set {
                match qw_rate(&args[0]) {
                    Ok(bps) => {
                        ops.borrow_mut().push(QwPhaseOp::SetPeerRate { id, bps });
                        bps
                    }
                    Err(error) => {
                        failed(ops, sender, error.to_string());
                        return;
                    }
                }
            } else {
                peers
                    .borrow()
                    .peers
                    .iter()
                    .find(|peer| peer.id == id)
                    .map(|peer| peer.channel.bytes_per_second)
                    .unwrap_or(2500.0)
            };
            ops.borrow_mut().push(QwPhaseOp::Print {
                id,
                level: 2,
                text: format!("{} {bps}\n", if set { "Net rate set to" } else { "Current rate is" }),
            });
            return;
        }
        match Self::phase_client_command(
            ops,
            peers,
            id,
            name,
            args,
            text,
            snapshots,
            admin_status,
            pings,
            max_clients,
            paused,
            now,
        ) {
            Ok(true) => {}
            Ok(false) => {
                ops.borrow_mut().push(QwPhaseOp::Command {
                    name: name.to_string(),
                    args: args.to_vec(),
                });
                if name == "setinfo" {
                    ops.borrow_mut().push(QwPhaseOp::SetinfoRefresh { id });
                }
            }
            Err(reason) => failed(ops, sender, reason),
        }
    }

    /// Run a client command (donor `clientCommand`), reporting whether it was
    /// handled. Parse failures return the donor `failed` reason.
    #[allow(clippy::too_many_arguments)]
    fn phase_client_command(
        ops: &Rc<RefCell<Vec<QwPhaseOp>>>,
        peers: &Rc<RefCell<QwPeerTable<T, H>>>,
        id: u64,
        name: &str,
        args: &[String],
        text: &str,
        snapshots: &[QwPhasePeer],
        admin_status: Option<&str>,
        pings: &HashMap<ClientId, i64>,
        max_clients: u32,
        paused: bool,
        now: f64,
    ) -> Result<bool, String> {
        if name == "msg" {
            if args.len() == 1 {
                let level = native_atoi(&args[0]);
                ops.borrow_mut().push(QwPhaseOp::SetPeerMsg { id, level });
                ops.borrow_mut().push(QwPhaseOp::Print {
                    id,
                    level: 2,
                    text: format!("Msg level set to {level}\n"),
                });
            } else {
                let level = peers
                    .borrow()
                    .peers
                    .iter()
                    .find(|peer| peer.id == id)
                    .map(|peer| peer.message_level)
                    .unwrap_or(0);
                ops.borrow_mut().push(QwPhaseOp::Print {
                    id,
                    level: 2,
                    text: format!("Current msg level is {level}\n"),
                });
            }
            return Ok(true);
        }
        if name == "ping" || name == "status" {
            let body = if name == "ping" {
                let mut body = "Client ping times:\n".to_string();
                for peer in snapshots.iter().filter(|peer| peer.active) {
                    let ping = pings.get(&peer.player.client).copied().unwrap_or(9999);
                    body.push_str(&format!("{ping} {}\n", peer.name.as_deref().unwrap_or("unnamed")));
                }
                body
            } else if let Some(status) = admin_status {
                status.to_string()
            } else {
                let mut body = format!("players: {} active ({max_clients} max)\n", snapshots.len());
                for peer in snapshots {
                    body.push_str(&format!(
                        "#{} {}\n",
                        peer.player.slot + 1,
                        peer.name.as_deref().unwrap_or("unnamed")
                    ));
                }
                body
            };
            ops.borrow_mut().push(QwPhaseOp::Print {
                id,
                level: 2,
                text: body,
            });
            return Ok(true);
        }
        if name == "pings" {
            for peer in snapshots.iter().filter(|peer| peer.active) {
                let ping = pings.get(&peer.player.client).copied().unwrap_or(9999);
                let slot = peer.player.slot as u8;
                let ping_message = QwServerMessage {
                    message: QuakeWorldMessage::SlotStat {
                        kind: QwSlotStat::Ping,
                        slot,
                        value: QwSlotValue::Integer(ping as i32),
                    },
                };
                let loss_message = QwServerMessage {
                    message: QuakeWorldMessage::SlotStat {
                        kind: QwSlotStat::PacketLoss,
                        slot,
                        value: QwSlotValue::Integer(i32::from(peer.loss)),
                    },
                };
                ops.borrow_mut().push(QwPhaseOp::Message {
                    id,
                    message: ping_message,
                });
                ops.borrow_mut().push(QwPhaseOp::Message {
                    id,
                    message: loss_message,
                });
            }
            return Ok(true);
        }
        if name == "kill" {
            let active = snapshots
                .iter()
                .find(|peer| peer.id == id)
                .is_some_and(|peer| peer.active);
            if !active {
                ops.borrow_mut().push(QwPhaseOp::Print {
                    id,
                    level: 2,
                    text: "Can't suicide -- allready dead!\n".to_string(),
                });
                return Ok(true);
            }
            return Ok(false);
        }
        if name != "say" && name != "say_team" {
            return Ok(false);
        }
        if args.is_empty() {
            return Ok(true);
        }
        let verdict = peers
            .borrow_mut()
            .peers
            .iter_mut()
            .find(|peer| peer.id == id)
            .map(|peer| peer.chat_flood.check(now / 1000.0, paused));
        match verdict {
            Some(ChatVerdict::Locked { seconds }) => {
                ops.borrow_mut().push(QwPhaseOp::Print {
                    id,
                    level: 3,
                    text: format!("You can't talk for {seconds} more seconds\n"),
                });
                return Ok(true);
            }
            Some(ChatVerdict::Flood { seconds }) => {
                ops.borrow_mut().push(QwPhaseOp::Print {
                    id,
                    level: 3,
                    text: format!("FloodProt: You can't talk for {seconds} seconds.\n"),
                });
                return Ok(true);
            }
            _ => {}
        }
        let sender_info = snapshots.iter().find(|peer| peer.id == id);
        let team = name == "say_team";
        let sender_team: String = sender_info
            .and_then(|peer| peer.team.clone())
            .unwrap_or_default()
            .chars()
            .take(31)
            .collect();
        let mut tokenizer = Q1Tokenizer { index: 0 };
        parse_q1_token(text.as_bytes(), &mut tokenizer, Q1TokenDialect::Quakeworld);
        let mut words = text.get(tokenizer.index..).unwrap_or("").trim_start().to_string();
        if let Some(rest) = words.strip_prefix('"') {
            // The donor `slice(1, -1)` drops the final unit unconditionally.
            let end = rest.char_indices().last().map(|(index, _)| index).unwrap_or(0);
            words = rest[..end].to_string();
        }
        let sender_name: String = sender_info
            .and_then(|peer| peer.name.clone())
            .unwrap_or_default()
            .chars()
            .take(31)
            .collect();
        let message = if team {
            format!("({sender_name}): {words}\n")
        } else {
            format!("{sender_name}: {words}\n")
        };
        ops.borrow_mut().push(QwPhaseOp::HostPrint { text: message.clone() });
        for peer in snapshots.iter().filter(|peer| peer.active) {
            if !team || peer.team.clone().unwrap_or_default() == sender_team {
                ops.borrow_mut().push(QwPhaseOp::Print {
                    id: peer.id,
                    level: 3,
                    text: message.clone(),
                });
            }
        }
        Ok(true)
    }

    /// Replay queued phase effects in order (donor phase bodies).
    fn replay_phase(&mut self, ops: Vec<QwPhaseOp>, player: &QwApplicationPlayer) -> Result<(), QwServerError> {
        for op in ops {
            match op {
                QwPhaseOp::Print { id, level, text } => {
                    self.shared.client_print(id, level, &text)?;
                }
                QwPhaseOp::Message { id, message } => {
                    self.shared.deliver(id, &message)?;
                }
                QwPhaseOp::HostPrint { text } => self.shared.print(&text),
                QwPhaseOp::Deliver { recipient, message } => {
                    let id = self
                        .shared
                        .peers
                        .borrow()
                        .peers
                        .iter()
                        .find(|peer| peer.player.client == recipient.client && peer.player.actor == recipient.actor)
                        .map(|peer| peer.id);
                    if let Some(id) = id {
                        self.shared.deliver(id, &message)?;
                    }
                }
                QwPhaseOp::Command { name, args } => {
                    self.shared.host.borrow_mut().inner.command(player, &name, &args);
                }
                QwPhaseOp::SetPeerRate { id, bps } => {
                    let mut peers = self.shared.peers.borrow_mut();
                    if let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) {
                        peer.channel.bytes_per_second = bps;
                    }
                }
                QwPhaseOp::SetPeerMsg { id, level } => {
                    let mut peers = self.shared.peers.borrow_mut();
                    if let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) {
                        peer.message_level = level;
                    }
                }
                QwPhaseOp::SetinfoRefresh { id } => {
                    // The donor runs this inside the action `try`, so parse
                    // failures print and disconnect inline and the replay
                    // continues with the queued game emits.
                    let outcome = self.setinfo_refresh(id);
                    if let Err(error) = outcome {
                        let reason = error.to_string();
                        self.shared.print(&reason);
                        let client = self
                            .shared
                            .peers
                            .borrow()
                            .peers
                            .iter()
                            .find(|peer| peer.id == id)
                            .map(|peer| peer.player.client.clone());
                        if let Some(client) = client {
                            self.shared.disconnect_client(&client, &reason)?;
                        }
                    }
                }
                QwPhaseOp::Disconnect { client, reason } => {
                    self.shared.disconnect_client(&client, &reason)?;
                }
            }
        }
        Ok(())
    }

    /// Refresh peer fields from client info (donor `setinfo` tail).
    fn setinfo_refresh(&mut self, id: u64) -> Result<(), QwServerError> {
        let player = self
            .shared
            .peers
            .borrow()
            .peers
            .iter()
            .find(|peer| peer.id == id)
            .map(|peer| peer.player.clone());
        let Some(player) = player else {
            return Ok(());
        };
        let info = self.shared.host.borrow().inner.client_info(&player);
        let mut peers = self.shared.peers.borrow_mut();
        let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) else {
            return Ok(());
        };
        if let Some(level) = info.get("msg") {
            if !level.is_empty() {
                peer.message_level = native_atoi(level);
            }
        }
        if let Some(rate) = info.get("rate") {
            if !rate.is_empty() {
                peer.channel.bytes_per_second = qw_rate(rate)?;
            }
        }
        Ok(())
    }
}

impl<T, H> QwServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: QwApplicationServerHost + 'static,
{
    /// Bound address (donor `address`).
    #[must_use]
    pub fn address(&self) -> NetworkAddress {
        self.shared.transport.address()
    }

    /// Connected players (donor `clients`).
    #[must_use]
    pub fn clients(&self) -> Vec<QwApplicationPlayer> {
        self.shared
            .peers
            .borrow()
            .peers
            .iter()
            .map(|peer| peer.player.clone())
            .collect()
    }

    /// Active client pings (donor `clientPings`).
    #[must_use]
    pub fn client_pings(&self) -> HashMap<ClientId, i64> {
        self.shared.client_pings()
    }

    /// Disconnect a client (donor `disconnectClient`).
    pub fn disconnect_client(&mut self, client: &ClientId, reason: &str) -> Result<bool, QwServerError> {
        self.shared.disconnect_client(client, reason)
    }

    /// Carry peers across a world change (donor `changeWorld`).
    pub fn change_world(&mut self, host: H) -> Result<(), QwServerError> {
        validate_host(&host)?;
        let mut host = host;
        let carried: Vec<(u64, QwApplicationPlayer, QwSignonBinding<T, H>)> = {
            let peers = self.shared.peers.borrow();
            let mut carried = Vec::with_capacity(peers.peers.len());
            for peer in &peers.peers {
                let player = host.carried_player(&peer.player.client);
                let binding = self.shared.bind(&player, &mut host)?;
                carried.push((peer.id, player, binding));
            }
            carried
        };
        self.shared.host.borrow_mut().inner = host;
        for (id, player, binding) in carried {
            self.shared.close_signon(id);
            {
                let mut peers = self.shared.peers.borrow_mut();
                let Some(peer) = peers.peers.iter_mut().find(|peer| peer.id == id) else {
                    continue;
                };
                peer.player = player;
                peer.signon = Some(binding.signon);
                peer.baselines = binding.baselines;
                peer.active = false;
                peer.delta = None;
                peer.choked = 0;
                peer.frames.clear();
                peer.reliable.clear();
                peer.replay = QuakeWorldCommandReplay::default();
            }
            let message = QwServerMessage {
                message: QuakeWorldMessage::Text {
                    kind: QwText::Stufftext,
                    text: "changing\nreconnect\n".to_string(),
                },
            };
            let bytes = self.shared.bytes(std::slice::from_ref(&message))?;
            self.shared.enqueue(id, &bytes)?;
        }
        Ok(())
    }

    /// Poll the transport (donor `poll`).
    fn poll_inner(&mut self, now: f64) -> Result<Vec<ActorCommand>, QwServerError> {
        if !self.ended {
            let masters = self.shared.host.borrow().inner.masters().unwrap_or_default();
            if !masters.is_empty() {
                let count = self.shared.peers.borrow().peers.len() as u32;
                if let Some(packet) = self.heartbeat.next(now, count, false) {
                    for master in &masters {
                        self.shared.transport.send(master, &packet)?;
                    }
                }
            }
        }
        if self.ended {
            return Ok(Vec::new());
        }
        loop {
            let event = self.shared.transport.poll()?;
            let Some(event) = event else {
                break;
            };
            match event {
                ReceiveEvent::Packet { from, payload, .. } => self.packet(&from, &payload, now)?,
                ReceiveEvent::Error { error } => self.shared.print(&error),
                ReceiveEvent::Dropped { .. } => {}
            }
        }
        let timeout = self.timeout.unwrap_or(DEFAULT_TIMEOUT_MILLISECONDS) as f64;
        for id in self.shared.peer_ids() {
            let gate = self
                .shared
                .peers
                .borrow()
                .peers
                .iter()
                .find(|peer| peer.id == id)
                .map(|peer| (peer.last_received, peer.active, peer.player.client.clone()));
            let Some((last_received, active, client)) = gate else {
                continue;
            };
            if now - last_received > timeout {
                self.shared.disconnect_client(&client, "Client timed out")?;
            } else if !active {
                self.shared.send(id, None, now)?;
            }
        }
        Ok(Vec::new())
    }

    /// Publish a simulation step (donor `publish`).
    fn publish_inner(
        &mut self,
        output: &SimulationOutput,
        events: &[NetworkPresentationEvent],
        now: f64,
    ) -> Result<(), QwServerError> {
        if self.ended {
            return Ok(());
        }
        self.shared.host.borrow_mut().inner.observe(output, events);
        for id in self.shared.peer_ids() {
            let gate = self
                .shared
                .peers
                .borrow()
                .peers
                .iter()
                .find(|peer| peer.id == id)
                .map(|peer| (peer.player.clone(), peer.active));
            let Some((player, active)) = gate else {
                continue;
            };
            if !active {
                continue;
            }
            if let Err(error) = self.publish_peer(id, &player, output, now) {
                let reason = error.to_string();
                self.shared.disconnect_client(&player.client, &reason)?;
            }
        }
        Ok(())
    }

    /// Publish one peer frame (donor `publish` `try`).
    fn publish_peer(
        &mut self,
        id: u64,
        player: &QwApplicationPlayer,
        output: &SimulationOutput,
        now: f64,
    ) -> Result<(), QwServerError> {
        let frame = self.shared.host.borrow().inner.frame(player, output);
        for message in &frame.reliable {
            let gated = match &message.message {
                QuakeWorldMessage::Print { level, .. } => {
                    let floor = self
                        .shared
                        .peers
                        .borrow()
                        .peers
                        .iter()
                        .find(|peer| peer.id == id)
                        .map(|peer| peer.message_level)
                        .unwrap_or(0);
                    i32::from(*level) >= floor
                }
                _ => true,
            };
            if gated {
                let bytes = self.shared.bytes(std::slice::from_ref(message))?;
                self.shared.enqueue(id, &bytes)?;
            }
        }
        self.shared.send(id, Some(&frame), now)
    }

    /// Run the master heartbeat (donor `heartbeat`). Like the Q2 server,
    /// transport failures panic: the donor throw propagates to the caller and
    /// the trait cannot fail.
    fn heartbeat_inner(&mut self, now: f64) {
        if self.ended {
            return;
        }
        let count = self.shared.peers.borrow().peers.len() as u32;
        if let Some(packet) = self.heartbeat.next(now, count, true) {
            let masters = self.shared.host.borrow().inner.masters().unwrap_or_default();
            for master in &masters {
                if let Err(error) = self.shared.transport.send(master, &packet) {
                    panic!("{error}");
                }
            }
        }
    }

    /// Close the server (donor `close`). The trait cannot fail; the donor
    /// throw aborts the remaining shutdown, so print and return early with
    /// the server left open for a retry.
    fn close_inner(&mut self) {
        if self.ended {
            return;
        }
        let masters = self.shared.host.borrow().inner.masters().unwrap_or_default();
        for master in &masters {
            if let Err(error) = self.shared.transport.send(master, &quake_world_shutdown()) {
                self.shared.print(&error.to_string());
                return;
            }
        }
        for id in self.shared.peer_ids() {
            let client = self
                .shared
                .peers
                .borrow()
                .peers
                .iter()
                .find(|peer| peer.id == id)
                .map(|peer| peer.player.client.clone());
            if let Some(client) = client {
                if let Err(error) = self.shared.disconnect_client(&client, "Server shutdown") {
                    self.shared.print(&error.to_string());
                    return;
                }
            }
        }
        self.ended = true;
        self.shared.transport.close();
    }
}

impl<T, H> ApplicationNetwork for QwServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: QwApplicationServerHost + 'static,
{
    fn heartbeat(&mut self, now_milliseconds: u64) {
        self.heartbeat_inner(now_milliseconds as f64);
    }

    fn role(&self) -> ApplicationNetworkRole {
        ApplicationNetworkRole::Server
    }

    fn phase(&self) -> ApplicationNetworkPhase {
        if self.ended {
            ApplicationNetworkPhase::Closed
        } else {
            ApplicationNetworkPhase::Active
        }
    }

    fn wire(&self) -> WireSelection {
        WireSelection::Source {
            protocol: ProtocolIdentity::Q1Quakeworld,
        }
    }

    fn poll(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, ApplicationNetworkError> {
        self.poll_inner(now_milliseconds as f64)
            .map_err(QwServerError::into_network)
    }

    fn submit(&mut self, _commands: &[ActorCommand], _now_milliseconds: u64) -> Result<(), ApplicationNetworkError> {
        Ok(())
    }

    fn publish(
        &mut self,
        output: &SimulationOutput,
        events: &[NetworkPresentationEvent],
        now_milliseconds: u64,
    ) -> Result<(), ApplicationNetworkError> {
        self.publish_inner(output, events, now_milliseconds as f64)
            .map_err(QwServerError::into_network)
    }

    fn close(&mut self) {
        self.close_inner();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::time::{FrameContext, FramePhase, SourceTime};
    use qa_net::common::endpoint::ip_address;
    use qa_net::q1_net::{
        quake_world_out_of_band, write_client_string_command, write_quake_world_move, QwMoveVariables,
    };
    use qa_world::session::WorldSnapshot;
    use std::sync::{Arc, Mutex};

    use super::super::qw_server_types::QwServerAdministration;

    struct MockTransportInner {
        inbound: Vec<ReceiveEvent<NetworkAddress>>,
        sent: Vec<(NetworkAddress, Vec<u8>)>,
        closed: bool,
    }

    #[derive(Clone)]
    struct MockTransport {
        address: NetworkAddress,
        inner: Arc<Mutex<MockTransportInner>>,
    }

    impl MockTransport {
        fn new(address: NetworkAddress) -> Self {
            Self {
                address,
                inner: Arc::new(Mutex::new(MockTransportInner {
                    inbound: Vec::new(),
                    sent: Vec::new(),
                    closed: false,
                })),
            }
        }

        fn queue(&self, from: NetworkAddress, payload: Vec<u8>) {
            self.inner
                .lock()
                .expect("transport")
                .inbound
                .push(ReceiveEvent::Packet {
                    from,
                    payload,
                    received_at: 0.0,
                });
        }

        fn take_sent(&self) -> Vec<(NetworkAddress, Vec<u8>)> {
            std::mem::take(&mut self.inner.lock().expect("transport").sent)
        }

        fn resend(&self, packets: Vec<(NetworkAddress, Vec<u8>)>) {
            self.inner.lock().expect("transport").sent.extend(packets);
        }

        fn oob_text(payload: &[u8]) -> String {
            String::from_utf8_lossy(&payload[4..])
                .trim_end_matches('\0')
                .to_string()
        }
    }

    impl DatagramTransport for MockTransport {
        type Address = NetworkAddress;

        fn address(&self) -> NetworkAddress {
            self.address.clone()
        }

        fn closed(&self) -> bool {
            self.inner.lock().expect("transport").closed
        }

        fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
            self.inner
                .lock()
                .expect("transport")
                .sent
                .push((to.clone(), payload.to_vec()));
            Ok(true)
        }

        fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
            let mut inner = self.inner.lock().expect("transport");
            Ok(if inner.inbound.is_empty() {
                None
            } else {
                Some(inner.inbound.remove(0))
            })
        }

        fn subscribe_readable(&self, _listener: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
            Ok(0)
        }

        fn unsubscribe(&self, _token: u64) {}

        fn close(&self) {
            self.inner.lock().expect("transport").closed = true;
        }
    }

    #[derive(Default)]
    struct MockLogs {
        printed: Vec<String>,
        commands: Vec<(u32, String, Vec<String>)>,
        groups: Vec<(u32, usize, u32)>,
        disconnects: Vec<(u32, String)>,
        observed: usize,
    }

    struct MockSignon {
        slot: u8,
    }

    impl QuakeWorldSignonHost for MockSignon {
        fn server_data(&self) -> QuakeWorldMessage {
            QuakeWorldMessage::ServerData {
                protocol: QwProfile::Quakeworld,
                server_count: 1,
                game_directory: "qw".to_string(),
                player_slot: self.slot,
                spectator: false,
                level: "dm1".to_string(),
                move_variables: QwMoveVariables::default(),
            }
        }

        fn models(&self) -> Vec<String> {
            Vec::new()
        }

        fn sounds(&self) -> Vec<String> {
            Vec::new()
        }

        fn signon_buffers(&self) -> Vec<Vec<u8>> {
            Vec::new()
        }

        fn accepts_map_checksum(&self, _checksum: u32) -> bool {
            true
        }

        fn spawn(&mut self, _start_client: u32) -> Vec<Vec<u8>> {
            Vec::new()
        }

        fn begin(&mut self) {}

        fn disconnect(&mut self, _reason: &str) {}

        fn open_download(&mut self, _path: &str) -> Option<Box<dyn DownloadSource>> {
            None
        }
    }

    struct MockAdmin {
        status: String,
        blocked: Vec<NetworkAddress>,
    }

    impl QwServerAdministration for MockAdmin {
        fn rcon_password(&self) -> String {
            String::new()
        }

        fn blocked(&self, from: &NetworkAddress) -> bool {
            self.blocked.contains(from)
        }

        fn status(&self) -> String {
            self.status.clone()
        }

        fn log(&self, _sequence: i64) -> Option<String> {
            None
        }

        fn execute_admin(&mut self, _command: &str, _write: &mut dyn FnMut(&str)) {}
    }

    struct MockHost {
        owner: IdentityOwner,
        logs: Rc<RefCell<MockLogs>>,
        live: Vec<u32>,
        max: u32,
        infos: HashMap<u32, HashMap<String, String>>,
        admin: Option<MockAdmin>,
        paused: bool,
        bad_signon: bool,
        emit_probe: Option<QwServerMessage>,
        frame_messages: Vec<QwServerMessage>,
        frame_entities: Vec<Q1WireEntity>,
    }

    impl MockHost {
        fn new(logs: Rc<RefCell<MockLogs>>, max: u32) -> Self {
            Self {
                owner: IdentityOwner::create("test").expect("owner"),
                logs,
                live: Vec::new(),
                max,
                infos: HashMap::new(),
                admin: None,
                paused: false,
                bad_signon: false,
                emit_probe: None,
                frame_messages: Vec::new(),
                frame_entities: Vec::new(),
            }
        }

        fn with_info(mut self, slot: u32, name: &str, team: &str) -> Self {
            self.infos.insert(
                slot,
                HashMap::from([
                    ("name".to_string(), name.to_string()),
                    ("team".to_string(), team.to_string()),
                ]),
            );
            self
        }
    }

    impl QwApplicationServerHost for MockHost {
        fn administration(&mut self) -> Option<&mut dyn QwServerAdministration> {
            self.admin
                .as_mut()
                .map(|admin| admin as &mut dyn QwServerAdministration)
        }

        fn max_clients(&self) -> u32 {
            self.max
        }

        fn paused(&self) -> bool {
            self.paused
        }

        fn supports_source_wire(&self) -> WireAdmission {
            WireAdmission::Supported
        }

        fn admit(&mut self, _request: &QuakeWorldConnectRequest) -> QwApplicationAdmission {
            let mut slot = 0;
            while self.live.contains(&slot) {
                slot += 1;
            }
            self.live.push(slot);
            QwApplicationAdmission::Accepted {
                player: QwApplicationPlayer {
                    client: self.owner.client(slot, 0),
                    actor: self.owner.actor(slot, 0),
                    slot,
                },
            }
        }

        fn carried_player(&self, client: &ClientId) -> QwApplicationPlayer {
            QwApplicationPlayer {
                client: client.clone(),
                actor: self.owner.actor(client.slot(), 0),
                slot: client.slot(),
            }
        }

        fn client_info(&self, player: &QwApplicationPlayer) -> HashMap<String, String> {
            self.infos.get(&player.slot).cloned().unwrap_or_default()
        }

        fn command_phase(
            &mut self,
            player: &QwApplicationPlayer,
            action: &dyn Fn(),
            emit: &mut dyn FnMut(&QwApplicationPlayer, &QwServerMessage),
        ) {
            action();
            if let Some(probe) = self.emit_probe.clone() {
                emit(player, &probe);
            }
        }

        fn disconnect(&mut self, player: &QwApplicationPlayer, reason: &str) {
            self.live.retain(|slot| *slot != player.slot);
            self.logs
                .borrow_mut()
                .disconnects
                .push((player.slot, reason.to_string()));
        }

        fn signon(&mut self, player: &QwApplicationPlayer) -> Box<dyn QuakeWorldSignonHost> {
            let slot = if self.bad_signon {
                player.slot as u8 + 1
            } else {
                player.slot as u8
            };
            Box::new(MockSignon { slot })
        }

        fn baselines(&self, _player: &QwApplicationPlayer) -> Vec<Q1WireEntity> {
            Vec::new()
        }

        fn frame(&self, _player: &QwApplicationPlayer, _output: &SimulationOutput) -> QwApplicationFrame {
            QwApplicationFrame {
                entities: self.frame_entities.clone(),
                messages: Vec::new(),
                reliable: self.frame_messages.clone(),
            }
        }

        fn command_group(&mut self, player: &QwApplicationPlayer, commands: &[QwUserCommand], sequence: u32) {
            self.logs
                .borrow_mut()
                .groups
                .push((player.slot, commands.len(), sequence));
        }

        fn command(&mut self, player: &QwApplicationPlayer, name: &str, args: &[String]) {
            if name == "setinfo" && args.len() >= 2 {
                self.infos
                    .entry(player.slot)
                    .or_default()
                    .insert(args[0].clone(), args[1].clone());
            }
            self.logs
                .borrow_mut()
                .commands
                .push((player.slot, name.to_string(), args.to_vec()));
        }

        fn observe(&mut self, _output: &SimulationOutput, _events: &[NetworkPresentationEvent]) {
            self.logs.borrow_mut().observed += 1;
        }

        fn print(&mut self, text: &str) {
            self.logs.borrow_mut().printed.push(text.to_string());
        }
    }

    type TestNet = QwServerNetwork<MockTransport, MockHost>;

    struct Fixture {
        transport: MockTransport,
        network: TestNet,
        logs: Rc<RefCell<MockLogs>>,
    }

    impl Fixture {
        fn poll(&mut self, now: u64) {
            self.network.poll(now).expect("poll");
        }

        /// Publish a step (active peers only send on publish, like the donor
        /// application loop).
        fn publish(&mut self, now: u64) {
            self.network.publish(&test_output(), &[], now).expect("publish");
        }

        fn queue(&self, from: NetworkAddress, payload: Vec<u8>) {
            self.transport.queue(from, payload);
        }
    }

    fn build(host: MockHost) -> Fixture {
        let address = ip_address("127.0.0.1", 27500, false).expect("address");
        let transport = MockTransport::new(address);
        let logs = host.logs.clone();
        let network = QwServerNetwork::new(QwServerNetworkOptions {
            transport: transport.clone(),
            host,
            random: Box::new(|| 123.0),
            timeout_milliseconds: None,
        })
        .expect("server");
        Fixture {
            transport,
            network,
            logs,
        }
    }

    fn fixture(max: u32) -> Fixture {
        build(MockHost::new(Rc::new(RefCell::new(MockLogs::default())), max))
    }

    fn client_address(port: u32) -> NetworkAddress {
        ip_address("127.0.0.1", port, false).expect("client")
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

    /// Run the challenge handshake; returns the admitted player.
    fn connect(fx: &mut Fixture, port: u32, qport: u16, userinfo: &str, now: u64) -> QwApplicationPlayer {
        let from = client_address(port);
        fx.queue(from.clone(), quake_world_out_of_band("getchallenge\n", false));
        fx.poll(now);
        let sent = fx.transport.take_sent();
        assert_eq!(sent.len(), 1);
        let challenge = MockTransport::oob_text(&sent[0].1);
        assert!(challenge.starts_with('c'), "{challenge}");
        let text = format!("connect 28 {qport} {} \"{userinfo}\"\n", &challenge[1..]);
        fx.queue(from, quake_world_out_of_band(&text, false));
        fx.poll(now);
        let sent = fx.transport.take_sent();
        assert_eq!(sent.len(), 1, "expected one accept, got {}", sent.len());
        assert_eq!(MockTransport::oob_text(&sent[0].1), "j");
        fx.network.clients().last().cloned().expect("peer")
    }

    /// Manual toggle peer. A real channel would drop the throwaway
    /// sequence-zero packets on both ends (donor behavior); the manual peer
    /// numbers from 1000 so every test exchange lands, and tracks the
    /// reliable bit so the server clears on the next exchange.
    struct ManualLink {
        from: NetworkAddress,
        qport: u16,
        seq: u32,
        ack: u32,
        rel: u32,
    }

    impl ManualLink {
        fn new(port: u32, qport: u16) -> Self {
            Self {
                from: client_address(port),
                qport,
                seq: 1000,
                ack: 0,
                rel: 0,
            }
        }

        fn send_payload(&mut self, fx: &mut Fixture, payload: &[u8], now: f64) {
            let mut packet = Vec::with_capacity(10 + payload.len());
            packet.extend_from_slice(&self.seq.to_le_bytes());
            packet.extend_from_slice(&(self.ack | (self.rel << 31)).to_le_bytes());
            packet.extend_from_slice(&self.qport.to_le_bytes());
            packet.extend_from_slice(payload);
            self.seq += 1;
            fx.queue(self.from.clone(), packet);
            fx.poll(now as u64);
        }

        fn string_command(&mut self, fx: &mut Fixture, text: &str, now: f64) {
            let mut writer = MsgWriter::new(1450, false);
            write_client_string_command(&mut writer, text).expect("string");
            let payload = writer.bytes().to_vec();
            self.send_payload(fx, &payload, now);
        }

        fn mover(&mut self, fx: &mut Fixture, loss: u8, now: f64) {
            let mut writer = MsgWriter::new(1450, false);
            let bundle = QuakeWorldMove {
                oldest: QwUsercmd::default(),
                previous: QwUsercmd::default(),
                current: QwUsercmd::default(),
                loss_percent: loss,
            };
            write_quake_world_move(&mut writer, &bundle, self.seq).expect("move");
            let payload = writer.bytes().to_vec();
            self.send_payload(fx, &payload, now);
        }
    }

    /// Collect server payloads for one link, tracking acknowledgements and
    /// stashing other recipients' packets back in order.
    fn drain(link: &mut ManualLink, fx: &mut Fixture) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut others = Vec::new();
        for (to, payload) in fx.transport.take_sent() {
            if to != link.from {
                others.push((to, payload));
                continue;
            }
            if payload.len() >= 4 && payload[0] == 255 {
                continue;
            }
            if payload.len() < 8 {
                continue;
            }
            let word = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
            link.ack = link.ack.max(word & 0x7fff_ffff);
            if word >> 31 != 0 {
                link.rel ^= 1;
            }
            bytes.extend_from_slice(&payload[8..]);
        }
        fx.transport.resend(others);
        bytes
    }

    fn drain_text(link: &mut ManualLink, fx: &mut Fixture) -> String {
        String::from_utf8_lossy(&drain(link, fx)).into_owned()
    }

    #[test]
    fn rejects_non_ip_endpoints() {
        let mut fx = fixture(8);
        let from = NetworkAddress::Loopback { id: "loop".to_string() };
        fx.queue(from.clone(), quake_world_out_of_band("getchallenge\n", false));
        fx.poll(1000);
        let sent = fx.transport.take_sent();
        assert_eq!(sent.len(), 1);
        let challenge = MockTransport::oob_text(&sent[0].1);
        let text = format!("connect 28 1111 {} \"\name\\loop\"\n", &challenge[1..]);
        fx.queue(from, quake_world_out_of_band(&text, false));
        fx.poll(1000);
        let sent = fx.transport.take_sent();
        assert_eq!(sent.len(), 1);
        assert!(MockTransport::oob_text(&sent[0].1).contains("QW requires an IP endpoint"));
        assert!(fx.network.clients().is_empty());
    }

    #[test]
    fn admits_client_and_reports_endpoint() {
        let mut fx = fixture(8);
        let player = connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        assert_eq!(player.slot, 0);
        assert_eq!(fx.network.clients().len(), 1);
        assert_eq!(fx.network.role(), ApplicationNetworkRole::Server);
        assert_eq!(fx.network.phase(), ApplicationNetworkPhase::Active);
        assert_eq!(
            fx.network.wire(),
            WireSelection::Source {
                protocol: ProtocolIdentity::Q1Quakeworld
            }
        );
        assert_eq!(
            fx.network.address(),
            ip_address("127.0.0.1", 27500, false).expect("addr")
        );
    }

    #[test]
    fn duplicate_connect_is_silent() {
        let mut fx = fixture(8);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let from = client_address(27501);
        fx.queue(from.clone(), quake_world_out_of_band("getchallenge\n", false));
        fx.poll(2000);
        let sent = fx.transport.take_sent();
        let challenge = MockTransport::oob_text(&sent[0].1);
        let text = format!("connect 28 1111 {} \"\name\\alice\"\n", &challenge[1..]);
        fx.queue(from, quake_world_out_of_band(&text, false));
        fx.poll(2000);
        assert!(fx.transport.take_sent().is_empty());
        assert_eq!(fx.network.clients().len(), 1);
    }

    #[test]
    fn reconnect_drops_active_peer() {
        let mut fx = fixture(8);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let mut link = ManualLink::new(27501, 1111);
        link.string_command(&mut fx, "begin 1", 2000.0);
        let from = client_address(27501);
        fx.queue(from.clone(), quake_world_out_of_band("getchallenge\n", false));
        fx.poll(3000);
        let sent = fx.transport.take_sent();
        let challenge = MockTransport::oob_text(&sent[0].1);
        let text = format!("connect 28 1111 {} \"\name\\alice\"\n", &challenge[1..]);
        fx.queue(from, quake_world_out_of_band(&text, false));
        fx.poll(3000);
        let sent = fx.transport.take_sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(MockTransport::oob_text(&sent[0].1), "j");
        assert_eq!(fx.network.clients().len(), 1);
        let logs = fx.logs.borrow();
        assert!(logs.disconnects.iter().any(|(_, reason)| reason == "Reconnecting"));
    }

    #[test]
    fn server_full_rejects() {
        let mut fx = fixture(1);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let from = client_address(27502);
        fx.queue(from.clone(), quake_world_out_of_band("getchallenge\n", false));
        fx.poll(2000);
        let sent = fx.transport.take_sent();
        let challenge = MockTransport::oob_text(&sent[0].1);
        let text = format!("connect 28 2222 {} \"\name\\bob\"\n", &challenge[1..]);
        fx.queue(from, quake_world_out_of_band(&text, false));
        fx.poll(2000);
        let sent = fx.transport.take_sent();
        assert_eq!(sent.len(), 1);
        assert!(MockTransport::oob_text(&sent[0].1).contains("Server is full"));
        assert_eq!(fx.network.clients().len(), 1);
    }

    #[test]
    fn bad_signon_disconnects_silently() {
        let logs = Rc::new(RefCell::new(MockLogs::default()));
        let mut host = MockHost::new(logs, 8);
        host.bad_signon = true;
        let mut fx = build(host);
        let from = client_address(27501);
        fx.queue(from.clone(), quake_world_out_of_band("getchallenge\n", false));
        fx.poll(1000);
        let sent = fx.transport.take_sent();
        let challenge = MockTransport::oob_text(&sent[0].1);
        let text = format!("connect 28 1111 {} \"\name\\alice\"\n", &challenge[1..]);
        fx.queue(from, quake_world_out_of_band(&text, false));
        fx.poll(1000);
        assert!(fx.transport.take_sent().is_empty());
        assert!(fx.network.clients().is_empty());
        let logs = fx.logs.borrow();
        assert!(logs.disconnects.iter().any(|(_, reason)| reason == "Signon failed"));
        assert!(logs.printed.iter().any(|text| text.contains("Source signon")));
    }

    #[test]
    fn signon_replies_and_begin_activates() {
        let mut fx = fixture(8);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let mut link = ManualLink::new(27501, 1111);
        link.string_command(&mut fx, "new", 2000.0);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("dm1"), "{text}");
        link.string_command(&mut fx, "begin 1", 3000.0);
        let player = &fx.network.clients()[0];
        assert_eq!(fx.network.client_pings().get(&player.client), Some(&1000));
    }

    #[test]
    fn say_broadcasts_and_logs() {
        let logs = Rc::new(RefCell::new(MockLogs::default()));
        let host = MockHost::new(logs, 8)
            .with_info(0, "alice", "red")
            .with_info(1, "bob", "blue");
        let mut fx = build(host);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        connect(&mut fx, 27502, 2222, "\name\\bob", 1000);
        let mut alice = ManualLink::new(27501, 1111);
        let mut bob = ManualLink::new(27502, 2222);
        alice.string_command(&mut fx, "begin 1", 2000.0);
        bob.string_command(&mut fx, "begin 1", 2000.0);
        fx.transport.take_sent();
        alice.string_command(&mut fx, "say hello", 3000.0);
        fx.publish(3000);
        let alice_text = drain_text(&mut alice, &mut fx);
        assert!(alice_text.contains("alice: hello\n"), "{alice_text}");
        let bob_text = drain_text(&mut bob, &mut fx);
        assert!(bob_text.contains("alice: hello\n"), "{bob_text}");
        // Reliable retransmits may duplicate; the host log holds one copy.
        let logs = fx.logs.borrow();
        assert_eq!(
            logs.printed.iter().filter(|text| text.contains("alice: hello")).count(),
            1
        );
    }

    #[test]
    fn say_team_filters_recipients() {
        let logs = Rc::new(RefCell::new(MockLogs::default()));
        let host = MockHost::new(logs, 8)
            .with_info(0, "alice", "red")
            .with_info(1, "bob", "blue");
        let mut fx = build(host);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        connect(&mut fx, 27502, 2222, "\name\\bob", 1000);
        let mut alice = ManualLink::new(27501, 1111);
        let mut bob = ManualLink::new(27502, 2222);
        alice.string_command(&mut fx, "begin 1", 2000.0);
        bob.string_command(&mut fx, "begin 1", 2000.0);
        fx.transport.take_sent();
        alice.string_command(&mut fx, "say_team hi", 3000.0);
        fx.publish(3000);
        let alice_text = drain_text(&mut alice, &mut fx);
        assert!(alice_text.contains("(alice): hi\n"), "{alice_text}");
        let bob_text = drain_text(&mut bob, &mut fx);
        assert!(!bob_text.contains("(alice): hi"), "{bob_text}");
    }

    #[test]
    fn flood_blocks_chatter() {
        let logs = Rc::new(RefCell::new(MockLogs::default()));
        let host = MockHost::new(logs, 8).with_info(0, "alice", "red");
        let mut fx = build(host);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let mut link = ManualLink::new(27501, 1111);
        link.string_command(&mut fx, "begin 1", 2000.0);
        fx.transport.take_sent();
        for step in 0..4 {
            let now = 3000.0 + f64::from(step) * 100.0;
            link.string_command(&mut fx, "say hi", now);
            fx.publish(now as u64);
        }
        drain(&mut link, &mut fx);
        link.string_command(&mut fx, "say hi", 3400.0);
        fx.publish(3400);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("FloodProt: You can't talk for 10 seconds."), "{text}");
        drain(&mut link, &mut fx);
        link.string_command(&mut fx, "say hi", 3500.0);
        fx.publish(3500);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("You can't talk for"), "{text}");
    }

    #[test]
    fn msg_and_rate_commands() {
        let mut fx = fixture(8);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let mut link = ManualLink::new(27501, 1111);
        // Note: the donor gates the level-2 confirmation below a level-3+
        // floor, so confirm with a visible level (donor behavior).
        link.string_command(&mut fx, "msg 1", 2000.0);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("Msg level set to 1\n"), "{text}");
        link.string_command(&mut fx, "msg", 3000.0);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("Current msg level is 1\n"), "{text}");
        link.string_command(&mut fx, "rate 5000", 4000.0);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("Net rate set to 5000\n"), "{text}");
        link.string_command(&mut fx, "rate", 5000.0);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("Current rate is 5000\n"), "{text}");
    }

    #[test]
    fn ping_status_and_pings_commands() {
        let logs = Rc::new(RefCell::new(MockLogs::default()));
        let host = MockHost::new(logs, 2).with_info(0, "alice", "red");
        let mut fx = build(host);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let mut link = ManualLink::new(27501, 1111);
        link.string_command(&mut fx, "begin 1", 2000.0);
        link.string_command(&mut fx, "ping", 3000.0);
        fx.publish(3000);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("Client ping times:\n9999 alice\n"), "{text}");
        link.string_command(&mut fx, "status", 4000.0);
        fx.publish(4000);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("players: 1 active (2 max)\n#1 alice\n"), "{text}");
        link.mover(&mut fx, 7, 5000.0);
        link.string_command(&mut fx, "pings", 6000.0);
        let player = fx.network.clients()[0].clone();
        let ping = fx.network.client_pings().get(&player.client).copied().unwrap_or(9999);
        fx.publish(6000);
        let bytes = drain(&mut link, &mut fx);
        let expected = [36, 0, (ping & 255) as u8, (ping >> 8) as u8];
        assert!(bytes.windows(4).any(|window| window == expected));
        assert!(bytes.windows(3).any(|window| window == [53, 0, 7]));
    }

    #[test]
    fn move_commands_reach_active_hosts() {
        let mut fx = fixture(8);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        connect(&mut fx, 27502, 2222, "\name\\bob", 1000);
        let mut alice = ManualLink::new(27501, 1111);
        let mut bob = ManualLink::new(27502, 2222);
        alice.string_command(&mut fx, "begin 1", 2000.0);
        alice.mover(&mut fx, 0, 3000.0);
        bob.mover(&mut fx, 0, 3000.0);
        let logs = fx.logs.borrow();
        assert_eq!(logs.groups.len(), 1);
        assert_eq!(logs.groups[0].0, 0);
        assert_eq!(logs.groups[0].1, 1);
    }

    #[test]
    fn drop_disconnects() {
        let mut fx = fixture(8);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let mut link = ManualLink::new(27501, 1111);
        link.string_command(&mut fx, "drop", 2000.0);
        assert!(fx.network.clients().is_empty());
        let logs = fx.logs.borrow();
        assert!(logs
            .disconnects
            .iter()
            .any(|(_, reason)| reason == "Client disconnected"));
        assert!(!fx.transport.take_sent().is_empty());
    }

    #[test]
    fn timeout_disconnects_idle_peers() {
        let mut fx = fixture(8);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        fx.poll(1000 + 65_001);
        assert!(fx.network.clients().is_empty());
        let logs = fx.logs.borrow();
        assert!(logs.disconnects.iter().any(|(_, reason)| reason == "Client timed out"));
    }

    #[test]
    fn kill_before_begin_prints() {
        let mut fx = fixture(8);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let mut link = ManualLink::new(27501, 1111);
        link.string_command(&mut fx, "kill", 2000.0);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("Can't suicide -- allready dead!\n"), "{text}");
    }

    #[test]
    fn setinfo_refreshes_peer_fields() {
        let mut fx = fixture(8);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let mut link = ManualLink::new(27501, 1111);
        link.string_command(&mut fx, "setinfo msg 1", 2000.0);
        assert!(fx.logs.borrow().commands.iter().any(|(_, name, _)| name == "setinfo"));
        link.string_command(&mut fx, "msg", 3000.0);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("Current msg level is 1\n"), "{text}");
        // Donor quirk: a level-3+ floor gates the level-2 confirmation itself.
        link.string_command(&mut fx, "setinfo msg 4", 4000.0);
        link.string_command(&mut fx, "msg", 5000.0);
        let text = drain_text(&mut link, &mut fx);
        assert!(!text.contains("Current msg level"), "{text}");
    }

    #[test]
    fn game_emit_delivers_to_recipients() {
        let logs = Rc::new(RefCell::new(MockLogs::default()));
        let mut host = MockHost::new(logs, 8);
        host.emit_probe = Some(QwServerMessage {
            message: QuakeWorldMessage::Print {
                level: 1,
                text: "probe\n".to_string(),
            },
        });
        let mut fx = build(host);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let mut link = ManualLink::new(27501, 1111);
        link.string_command(&mut fx, "ping", 2000.0);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("probe\n"), "{text}");
    }

    #[test]
    fn publish_sends_frames_to_active_peers() {
        let logs = Rc::new(RefCell::new(MockLogs::default()));
        let mut host = MockHost::new(logs, 8);
        host.frame_messages = vec![QwServerMessage {
            message: QuakeWorldMessage::Print {
                level: 2,
                text: "frame\n".to_string(),
            },
        }];
        host.frame_entities = vec![Q1WireEntity {
            number: 5,
            ..Default::default()
        }];
        let mut fx = build(host);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let mut link = ManualLink::new(27501, 1111);
        link.string_command(&mut fx, "begin 1", 2000.0);
        link.mover(&mut fx, 0, 3000.0);
        fx.transport.take_sent();
        fx.network.publish(&test_output(), &[], 4000).expect("publish");
        let bytes = drain(&mut link, &mut fx);
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert!(text.contains("frame\n"), "{text}");
        assert!(bytes.contains(&47));
        assert_eq!(fx.logs.borrow().observed, 1);
    }

    #[test]
    fn change_world_carries_peers() {
        let mut fx = fixture(8);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let mut link = ManualLink::new(27501, 1111);
        link.string_command(&mut fx, "begin 1", 2000.0);
        let logs = Rc::new(RefCell::new(MockLogs::default()));
        fx.network.change_world(MockHost::new(logs, 8)).expect("change");
        assert_eq!(fx.network.clients().len(), 1);
        assert_eq!(fx.network.clients()[0].slot, 0);
        assert!(fx.logs.borrow().disconnects.is_empty());
        // The carried peer re-signons and receives the reconnect stufftext.
        link.string_command(&mut fx, "begin 1", 3000.0);
        fx.publish(3000);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("changing\nreconnect\n"), "{text}");
    }

    #[test]
    fn blocked_addresses_are_ignored() {
        let logs = Rc::new(RefCell::new(MockLogs::default()));
        let mut host = MockHost::new(logs, 8);
        host.admin = Some(MockAdmin {
            status: "admin\n".to_string(),
            blocked: vec![client_address(27501)],
        });
        let mut fx = build(host);
        connect(&mut fx, 27502, 2222, "\name\\bob", 1000);
        let mut link = ManualLink::new(27502, 2222);
        link.string_command(&mut fx, "begin 1", 2000.0);
        link.string_command(&mut fx, "status", 3000.0);
        fx.publish(3000);
        let text = drain_text(&mut link, &mut fx);
        assert!(text.contains("admin\n"), "{text}");
        // The blocked sender was never admitted: noalletraces.
        assert_eq!(fx.network.clients().len(), 1);
    }

    #[test]
    fn close_shuts_down() {
        let mut fx = fixture(8);
        connect(&mut fx, 27501, 1111, "\name\\alice", 1000);
        let mut link = ManualLink::new(27501, 1111);
        link.string_command(&mut fx, "begin 1", 2000.0);
        fx.network.close();
        assert_eq!(fx.network.phase(), ApplicationNetworkPhase::Closed);
        assert!(fx.transport.closed());
        assert!(fx.network.clients().is_empty());
        assert!(fx
            .logs
            .borrow()
            .disconnects
            .iter()
            .any(|(_, reason)| reason == "Server shutdown"));
        fx.poll(3000);
        assert!(fx.logs.borrow().printed.is_empty());
    }

    #[test]
    fn validate_rejects_bad_hosts() {
        let logs = Rc::new(RefCell::new(MockLogs::default()));
        let address = ip_address("127.0.0.1", 27500, false).expect("address");
        let options = |host: MockHost| QwServerNetworkOptions {
            transport: MockTransport::new(address.clone()),
            host,
            random: Box::new(|| 0.0),
            timeout_milliseconds: None,
        };
        let error = QwServerNetwork::new(options(MockHost::new(logs.clone(), 0)))
            .err()
            .expect("zero");
        assert!(error.to_string().contains("at most 32 players"), "{error}");
        let error = QwServerNetwork::new(options(MockHost::new(logs.clone(), 33)))
            .err()
            .expect("many");
        assert!(error.to_string().contains("at most 32 players"), "{error}");
    }
}
