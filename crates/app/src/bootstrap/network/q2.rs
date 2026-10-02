//! Quake II server and client networks.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q2.ts`
//! (`Q2ServerNetwork`, `Q2ClientNetwork`). The donor is asynchronous; this
//! port resolves every step inline: channels, handshakes, downloads, MVD
//! broadcast, and server demos all have synchronous `qa-net` counterparts,
//! and recording sinks append synchronously. Three sync adaptations apply:
//! receiver commands/reset notifications travel as drained
//! [`Q2ReceiverActions`](super::q2_client_receiver::Q2ReceiverActions); the
//! client shares its host through `Rc<RefCell<..>>` so the receiver, the
//! download source, and the handshake closures all reach it; and recording
//! state lives in `Rc<RefCell<..>>` shared structs because
//! [`ApplicationNetworkRecording::seed`](super::types::ApplicationNetworkRecording::seed)
//! borrows immutably while capture and seeds mutate.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use qa_core::cmd::{tokenize_command, Dialect, TextMode};
use qa_core::identity::ClientId;
use qa_core::time::SourceTime;
use qa_net::common::commands::{ActorCommand, CommandSource};
use qa_net::common::endpoint::{address_key, same_address, NetworkAddress};
use qa_net::common::session::{WireAdmission, WireSelection};
use qa_net::common::transport::{DatagramTransport, ReceiveEvent};
use qa_net::protocol::ProtocolIdentity;
use qa_net::q2::{EntityState, Usercmd};
use qa_net::q2_kex_lan::{KexLanOptions, KexLanTransport};
use qa_net::q2_net::{
    encode_q2_frame, handle_q2_rcon_host, q2_info_text, q2_kex_client_userinfo, q2_kex_seat_userinfo, q2_out_of_band,
    q2_status_text, read_q2_connect, read_q2_out_of_band, ChannelSide, Q2ChallengeTable, Q2Channel, Q2ChannelOptions,
    Q2ChannelReceive, Q2ClientHandshake, Q2ClientHandshakeState, Q2ConnectAdmission, Q2ConnectRequest,
    Q2ConnectionlessHost, Q2ConnectionlessMessage, Q2DiscoveryWire, Q2Info, Q2LimitedRcon, Q2ServerData, Q2ServerEvent,
    Q2ServerMessageOptions, Q2ServerProfile, Q2ServerRecord, Q2SplitPlayer, Q2Status, Q2Wire, Q2WireFrame,
};
use qa_net::q2_server_demo::{encode_q2_server_demo_frame, encode_q2_server_demo_signon, Q2ServerDemoState};
use qa_net::q2_svc::{
    encode_q2_client_control, encode_q2_move, encode_q2_server_event, read_q2_client_messages, MvdBroadcast,
    MvdBroadcastOptions, MvdCapture, MvdEmission, MvdEncoder, MvdRecipient, Q2ClientEvent, Q2CommandReplay,
};
use qa_net::services::discovery::DiscoveryWire;
use qa_world::session::SimulationOutput;
use thiserror::Error;

use super::q2_client_receiver::{
    q2_receiver_protocol, seed_protocol, signon_integer, Q2ClientReceiver, Q2ClientReceiverHost,
    Q2ClientReceiverSource, Q2ReceiverActions,
};
use super::q2_downloads::{Q2ApplicationClientDownloads, Q2PeerDownload};
use super::types::{
    ApplicationNetwork, ApplicationNetworkError, ApplicationNetworkPhase, ApplicationNetworkRecording,
    ApplicationNetworkRole, NetworkPresentationEvent, Q2ApplicationAdmission, Q2ApplicationClientHost,
    Q2ApplicationGameState, Q2ApplicationPlayer, Q2ApplicationRcon, Q2ApplicationServerHost, Q2ClientNetworkOptions,
    Q2ServerNetworkOptions,
};
use crate::bootstrap::demo_recording::{
    DemoRecordingIdentity, DemoRecordingPacket, DemoRecordingSeed, DemoRecordingSink, MvdRevision, Q2ProtocolIdentity,
};

/// Quake II network failure.
#[derive(Debug, Error)]
pub enum Q2NetworkError {
    /// Policy or protocol failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] qa_net::q2_net::Q2NetError),
    /// Receiver failure.
    #[error(transparent)]
    Receiver(#[from] super::q2_client_receiver::Q2ClientReceiverError),
    /// KEX LAN failure.
    #[error(transparent)]
    Kex(#[from] qa_net::q2_kex_packet::KexError),
    /// Server demo failure.
    #[error(transparent)]
    ServerDemo(#[from] qa_net::q2_server_demo::Q2ServerDemoError),
    /// Recording failure.
    #[error(transparent)]
    Recording(#[from] crate::bootstrap::demo_recording::DemoRecordingError),
    /// Command failure.
    #[error(transparent)]
    Cmd(#[from] qa_core::cmd::CmdError),
    /// Transport failure.
    #[error(transparent)]
    Transport(#[from] qa_net::common::transport::TransportError),
    /// Discovery failure.
    #[error(transparent)]
    Discovery(#[from] qa_net::services::discovery::DiscoveryError),
    /// Message failure.
    #[error(transparent)]
    Msg(#[from] qa_net::msg::MsgError),
}

impl From<Q2NetworkError> for ApplicationNetworkError {
    fn from(error: Q2NetworkError) -> Self {
        Self::Message(error.to_string())
    }
}

/// Protocol kind name (`Q2ProtocolIdentity['kind']`).
fn q2_identity_kind(protocol: &Q2ProtocolIdentity) -> &'static str {
    match protocol {
        Q2ProtocolIdentity::Classic => "q2-classic",
        Q2ProtocolIdentity::R1Q2 { .. } => "q2-r1q2",
        Q2ProtocolIdentity::Q2Pro { .. } => "q2-q2pro",
        Q2ProtocolIdentity::Rerelease => "q2-rerelease",
        Q2ProtocolIdentity::Kex => "q2-kex",
        Q2ProtocolIdentity::KexDemo => "q2-kex-demo",
    }
}

/// Whether two identities share kind and version (revisions may differ).
fn same_wire_family(left: &Q2ProtocolIdentity, right: &Q2ProtocolIdentity) -> bool {
    q2_identity_kind(left) == q2_identity_kind(right) && left.version() == right.version()
}

/// Whether an identity is exact KEX (never the KEX demo dialect).
fn is_exact_kex(protocol: &Q2ProtocolIdentity) -> bool {
    matches!(protocol, Q2ProtocolIdentity::Kex)
}

/// Negotiate an enhanced-protocol revision (`min(configured, offered)`).
fn negotiate_q2_protocol(configured: &Q2ProtocolIdentity, offered: &Q2ProtocolIdentity) -> Q2ProtocolIdentity {
    match (configured, offered) {
        (Q2ProtocolIdentity::R1Q2 { revision: a }, Q2ProtocolIdentity::R1Q2 { revision: b }) => {
            if a.revision() < b.revision() {
                *configured
            } else {
                *offered
            }
        }
        (Q2ProtocolIdentity::Q2Pro { revision: a }, Q2ProtocolIdentity::Q2Pro { revision: b }) => {
            if a.revision() < b.revision() {
                *configured
            } else {
                *offered
            }
        }
        _ => *offered,
    }
}

/// Set server data's server count across variants.
fn set_servercount(data: &mut Q2ServerData, servercount: i32) {
    match data {
        Q2ServerData::Vanilla(inner) => inner.servercount = servercount,
        Q2ServerData::R1Q2(inner) => inner.servercount = servercount,
        Q2ServerData::Q2Pro(inner) => inner.servercount = servercount,
        Q2ServerData::Rerelease(inner) => inner.servercount = servercount,
        Q2ServerData::Kex(inner) => inner.servercount = servercount,
    }
}

/// Selected transport: direct datagrams or the KEX lobby wrapper.
enum Q2Transport<T: DatagramTransport<Address = NetworkAddress> + 'static> {
    /// Direct transport.
    Direct(T),
    /// KEX lobby transport.
    Lan(Arc<KexLanTransport<T>>),
}

impl<T: DatagramTransport<Address = NetworkAddress> + 'static> DatagramTransport for Q2Transport<T> {
    type Address = NetworkAddress;

    fn address(&self) -> NetworkAddress {
        match self {
            Self::Direct(transport) => transport.address(),
            Self::Lan(lan) => lan.address(),
        }
    }

    fn closed(&self) -> bool {
        match self {
            Self::Direct(transport) => transport.closed(),
            Self::Lan(lan) => lan.closed(),
        }
    }

    fn max_datagram_bytes(&self) -> Option<usize> {
        match self {
            Self::Direct(transport) => transport.max_datagram_bytes(),
            Self::Lan(lan) => lan.max_datagram_bytes(),
        }
    }

    fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, qa_net::common::transport::TransportError> {
        match self {
            Self::Direct(transport) => transport.send(to, payload),
            Self::Lan(lan) => lan.send(to, payload),
        }
    }

    fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, qa_net::common::transport::TransportError> {
        match self {
            Self::Direct(transport) => transport.poll(),
            Self::Lan(lan) => lan.poll(),
        }
    }

    fn subscribe_readable(
        &self,
        listener: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<u64, qa_net::common::transport::TransportError> {
        match self {
            Self::Direct(transport) => transport.subscribe_readable(listener),
            Self::Lan(lan) => lan.subscribe_readable(listener),
        }
    }

    fn unsubscribe(&self, token: u64) {
        match self {
            Self::Direct(transport) => transport.unsubscribe(token),
            Self::Lan(lan) => lan.unsubscribe(token),
        }
    }

    fn close(&self) {
        match self {
            Self::Direct(transport) => transport.close(),
            Self::Lan(lan) => lan.close(),
        }
    }
}

/// Split-screen seat on a server peer (`ServerSplit`).
struct Q2ServerSplit {
    /// Seat player.
    player: Q2ApplicationPlayer,
    /// Move replay.
    replay: Q2CommandReplay,
    /// Input sequence.
    sequence: u32,
}

/// Admitted server peer (`ServerPeer`).
struct Q2ServerPeer {
    /// Remote address.
    remote: NetworkAddress,
    /// Negotiated application protocol (for game state and frames).
    protocol: Q2ProtocolIdentity,
    /// Wire version (port-roaming fallback; `Q2Channel` options stay private).
    protocol_version: u32,
    /// Qport (port-roaming fallback).
    qport: u32,
    /// Primary player.
    player: Q2ApplicationPlayer,
    /// Split-screen seats.
    splits: Vec<Q2ServerSplit>,
    /// Peer download.
    download: Q2PeerDownload,
    /// Deferred download failure (only `poll` drops players).
    download_failure: Option<String>,
    /// Reliable channel.
    channel: Q2Channel,
    /// Server wire codec.
    wire: Q2Wire,
    /// Move replay.
    replay: Q2CommandReplay,
    /// Recent frames by server frame (evicted oldest-first).
    frames: BTreeMap<i32, Q2WireFrame>,
    /// Admitted game state.
    game_state: Option<Q2ApplicationGameState>,
    /// Whether the client sent `begin`.
    active: bool,
    /// Input sequence.
    sequence: u32,
    /// Last receive time.
    last_received: u64,
    /// Pending unreliable bytes.
    datagram: Vec<Vec<u8>>,
    /// Last userinfo.
    userinfo: String,
}

/// Multiview recording owner.
struct Q2MvdOwner {
    /// Recording sink.
    sink: Box<dyn DemoRecordingSink>,
    /// Stream encoder.
    encoder: MvdEncoder,
    /// Detach identity.
    id: u64,
}

/// Server-demo recording owner.
struct Q2ServerDemoOwner {
    /// Recording sink.
    sink: Box<dyn DemoRecordingSink>,
    /// Last written configstrings.
    configs: BTreeMap<u16, String>,
    /// Detach identity.
    id: u64,
}

/// Server state shared with the recording taps.
struct Q2ServerShared<H> {
    /// Server host.
    host: H,
    /// Latest published frame.
    mvd_frame: Option<(SimulationOutput, Vec<NetworkPresentationEvent>)>,
    /// Server generation.
    server_generation: i32,
    /// Shutdown flag.
    ended: bool,
    /// Last published MVD tick.
    mvd_tick: Option<i64>,
    /// Buffered MVD messages.
    mvd_messages: Vec<MvdEmission>,
    /// Multiview recording owner.
    mvd_owner: Option<Q2MvdOwner>,
    /// Multiview broadcast.
    mvd_broadcast: Option<MvdBroadcast>,
    /// Live broadcast password.
    mvd_password: Rc<RefCell<String>>,
    /// Server-demo recording owner.
    server_demo_owner: Option<Q2ServerDemoOwner>,
    /// Server-demo configstring wire.
    server_demo_wire: Q2Wire,
    /// Detach identity counter.
    detach_counter: u64,
}

/// Server recording tap kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Q2ServerTapKind {
    /// Server-demo tap.
    ServerDemo,
    /// Multiview tap.
    Mvd,
}

/// Server recording tap (field-owned so the trait can lend it).
struct Q2ServerTap<H> {
    /// Tap kind.
    kind: Q2ServerTapKind,
    /// Shared server state.
    shared: Rc<RefCell<Q2ServerShared<H>>>,
    /// Pending seed.
    seed: RefCell<Option<MvdCapture>>,
}

/// Quake II server network (`Q2ServerNetwork`).
pub struct Q2ServerNetwork<T: DatagramTransport<Address = NetworkAddress> + 'static, H> {
    transport: Q2Transport<T>,
    lan: Option<Arc<KexLanTransport<T>>>,
    shared: Rc<RefCell<Q2ServerShared<H>>>,
    server_tap: Q2ServerTap<H>,
    mvd_tap: Q2ServerTap<H>,
    wire: WireSelection,
    challenges: Q2ChallengeTable,
    peers: HashMap<String, Q2ServerPeer>,
    pending: Vec<ActorCommand>,
    timeout_milliseconds: Option<u64>,
    last_now: u64,
    heartbeat_next: f64,
}

impl<T, H> Q2ServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q2ApplicationServerHost,
{
    /// Build a server network.
    pub fn new(options: Q2ServerNetworkOptions<T, H>) -> Result<Self, Q2NetworkError> {
        let Q2ServerNetworkOptions {
            transport,
            host,
            random,
            timeout_milliseconds,
        } = options;
        let protocol = host.protocol();
        if matches!(protocol, Q2ProtocolIdentity::KexDemo) {
            return Err(Q2NetworkError::Message(
                "KEX native live transport is unbound".to_string(),
            ));
        }
        if let WireAdmission::Unsupported { reasons } = host.supports_source_wire() {
            return Err(Q2NetworkError::Message(format!(
                "Native Q2 wire is unavailable: {}",
                reasons.join("; ")
            )));
        }
        if is_exact_kex(&protocol) && host.max_clients() > u32::from(u8::MAX) {
            return Err(Q2NetworkError::Message("Invalid KEX player capacity".to_string()));
        }
        let (transport, lan) = if is_exact_kex(&protocol) {
            let lan = KexLanTransport::open(
                transport,
                KexLanOptions::Host {
                    max_players: host.max_clients() as u8,
                    local_players: 0,
                    name: "Quake II".to_string(),
                },
                // mDNS advertisement needs host wiring; direct LAN play works
                // without it.
                None,
            )?;
            (Q2Transport::Lan(lan.clone()), Some(lan))
        } else {
            (Q2Transport::Direct(transport), None)
        };
        let mut random = random;
        let challenges = Q2ChallengeTable::new(move || (random() * 32768.0) as u32, 1024);
        let shared = Rc::new(RefCell::new(Q2ServerShared {
            host,
            mvd_frame: None,
            server_generation: 1,
            ended: false,
            mvd_tick: None,
            mvd_messages: Vec::new(),
            mvd_owner: None,
            mvd_broadcast: None,
            mvd_password: Rc::new(RefCell::new(String::new())),
            server_demo_owner: None,
            server_demo_wire: Q2Wire::new(ProtocolIdentity::Q2Classic)?,
            detach_counter: 0,
        }));
        Ok(Self {
            transport,
            lan,
            shared: shared.clone(),
            server_tap: Q2ServerTap {
                kind: Q2ServerTapKind::ServerDemo,
                shared: shared.clone(),
                seed: RefCell::new(None),
            },
            mvd_tap: Q2ServerTap {
                kind: Q2ServerTapKind::Mvd,
                shared,
                seed: RefCell::new(None),
            },
            wire: WireSelection::Source {
                protocol: q2_receiver_protocol(&protocol),
            },
            challenges,
            peers: HashMap::new(),
            pending: Vec::new(),
            timeout_milliseconds,
            last_now: 0,
            // The donor closure throws when masters publish without a status
            // surface; the send path below raises the same message.
            heartbeat_next: f64::NEG_INFINITY,
        })
    }
}

/// Rcon connectionless adapter over the server administration surface.
///
/// `status`, `info`, and `connect` only satisfy
/// [`Q2ConnectionlessHost`](qa_net::q2_net::Q2ConnectionlessHost); the rcon
/// handler never calls them.
struct ServerRcon<'a, T: DatagramTransport<Address = NetworkAddress> + 'static> {
    /// Administration surface.
    admin: &'a mut dyn Q2ApplicationRcon,
    /// Reply transport.
    transport: &'a Q2Transport<T>,
    /// Advertised protocols.
    protocols: Vec<ProtocolIdentity>,
    /// Packet sender; the donor reply closure answers it unconditionally.
    remote: NetworkAddress,
}

impl<'a, T: DatagramTransport<Address = NetworkAddress> + 'static> Q2ConnectionlessHost for ServerRcon<'a, T> {
    fn profile(&self) -> Q2ServerProfile {
        self.admin.profile()
    }

    fn protocols(&self) -> &[ProtocolIdentity] {
        &self.protocols
    }

    fn status(&self) -> Q2Status {
        Q2Status {
            server_info: String::new(),
            players: Vec::new(),
        }
    }

    fn info(&self) -> Q2Info {
        Q2Info {
            name: String::new(),
            map: String::new(),
            players: 0,
            max_players: 0,
        }
    }

    fn connect(&mut self, _from: &NetworkAddress, _request: &Q2ConnectRequest) -> Q2ConnectAdmission {
        Q2ConnectAdmission::Rejected { reason: String::new() }
    }

    fn reply(&mut self, _to: &NetworkAddress, bytes: Vec<u8>) {
        let _ = self.transport.send(&self.remote, &bytes);
    }

    fn rcon_password(&self) -> String {
        self.admin.rcon_password()
    }

    fn limited_rcon(&self) -> Option<Q2LimitedRcon> {
        self.admin.limited_rcon()
    }

    fn rcon_rate_allowed(&self, now: u64) -> bool {
        self.admin.rcon_rate_allowed(now)
    }

    fn recharge_rcon_rate(&mut self) {
        self.admin.recharge_rcon_rate();
    }

    fn execute_rcon(&mut self, command: &str, limited: bool, output: &mut dyn FnMut(&str)) {
        self.admin.execute_rcon(command, limited, output);
    }
}

impl<T, H> Q2ServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q2ApplicationServerHost,
{
    /// Connection phase.
    #[must_use]
    pub fn phase(&self) -> ApplicationNetworkPhase {
        if self.shared.borrow().ended {
            ApplicationNetworkPhase::Closed
        } else {
            ApplicationNetworkPhase::Active
        }
    }

    /// Admitted players across peers and splits.
    #[must_use]
    pub fn clients(&self) -> Vec<Q2ApplicationPlayer> {
        self.peers
            .values()
            .flat_map(|peer| {
                std::iter::once(peer.player.clone()).chain(peer.splits.iter().map(|split| split.player.clone()))
            })
            .collect()
    }

    /// Move peers to a new world host (`changeWorld`).
    pub fn change_world(&mut self, host: H) -> Result<(), Q2NetworkError> {
        if let WireAdmission::Unsupported { reasons } = host.supports_source_wire() {
            return Err(Q2NetworkError::Message(format!(
                "Native Q2 map transition is unavailable: {}",
                reasons.join("; ")
            )));
        }
        {
            let shared = self.shared.borrow();
            if !same_wire_family(&host.protocol(), &shared.host.protocol()) {
                return Err(Q2NetworkError::Message(
                    "Native Q2 map transition cannot change protocol".to_string(),
                ));
            }
            if shared.server_demo_owner.is_some() {
                return Err(Q2NetworkError::Message(
                    "Finish serverrecord with serverstop before changing maps".to_string(),
                ));
            }
        }
        let players: Vec<(String, Q2ApplicationPlayer)> = self
            .peers
            .iter()
            .map(|(key, peer)| (key.clone(), host.carried_player(&peer.player.client)))
            .collect();
        self.shared.borrow_mut().host = host;
        {
            let mut shared = self.shared.borrow_mut();
            shared.mvd_frame = None;
            shared.mvd_tick = None;
            shared.mvd_messages = Vec::new();
            shared.server_generation = shared.server_generation.wrapping_add(1);
        }
        self.server_tap.seed.borrow_mut().take();
        self.mvd_tap.seed.borrow_mut().take();
        self.pending = Vec::new();
        for (key, player) in players {
            let Some(peer) = self.peers.get_mut(&key) else {
                continue;
            };
            Self::close_download(peer);
            peer.player = player;
            for split in &mut peer.splits {
                let carried = self.shared.borrow_mut().host.carried_player(&split.player.client);
                split.player = carried;
                split.replay = Q2CommandReplay::new();
            }
            let userinfo = if is_exact_kex(&peer.protocol) {
                q2_kex_seat_userinfo(&peer.userinfo, 0)?
            } else {
                peer.userinfo.clone()
            };
            let player = peer.player.clone();
            self.shared.borrow_mut().host.userinfo(&player, &userinfo);
            for (seat, split) in peer.splits.iter().enumerate() {
                let split_userinfo = q2_kex_seat_userinfo(&peer.userinfo, seat as u32 + 1)?;
                let split_player = split.player.clone();
                self.shared.borrow_mut().host.userinfo(&split_player, &split_userinfo);
            }
            peer.active = false;
            peer.frames.clear();
            peer.game_state = None;
            peer.replay = Q2CommandReplay::new();
            peer.datagram = Vec::new();
            Self::stuff(peer, "changing\n")?;
            Self::reliable(peer, Q2ServerEvent::Reconnect)?;
        }
        Ok(())
    }

    /// Reply to an address (`reply`).
    fn reply(&mut self, to: &NetworkAddress, text: &str) {
        let kex = is_exact_kex(&self.shared.borrow().host.protocol());
        let _ = self.transport.send(to, &q2_out_of_band(text, kex));
    }

    /// Accept a connection (`acceptConnection`).
    fn accept_connection(&mut self, remote: &NetworkAddress) {
        let url = self.shared.borrow_mut().host.downloads().http_server();
        let kex = is_exact_kex(&self.shared.borrow().host.protocol());
        let mut text = format!("client_connect{}", if kex { " 2023" } else { "" });
        if let Some(url) = url {
            text.push_str(&format!(" dlserver={url}"));
        }
        self.reply(remote, &text);
    }

    /// Send a master heartbeat (`MasterHeartbeat.send` inline: the heartbeat
    /// borrows the transport, which the network owns, so the schedule and
    /// encoding live here).
    fn send_heartbeat(
        &mut self,
        masters: &[NetworkAddress],
        now_milliseconds: u64,
        active: bool,
        force: bool,
    ) -> Result<(), Q2NetworkError> {
        if !force && (now_milliseconds as f64) < self.heartbeat_next {
            return Ok(());
        }
        self.heartbeat_next = now_milliseconds as f64 + 300_000.0;
        let protocol = q2_receiver_protocol(&self.shared.borrow().host.protocol());
        for master in masters {
            let bytes = if !active {
                q2_out_of_band("shutdown", false)
            } else {
                let status = self
                    .shared
                    .borrow()
                    .host
                    .discovery()
                    .map(|discovery| discovery.status());
                let Some(status) = status else {
                    return Err(Q2NetworkError::Message(
                        "Q2 master publication requires source status".to_string(),
                    ));
                };
                Q2DiscoveryWire::new(protocol, move || status.clone()).heartbeat(true)?
            };
            let _ = self.transport.send(master, &bytes);
        }
        Ok(())
    }

    /// Periodic heartbeat (`heartbeat`).
    pub fn heartbeat(&mut self, now_milliseconds: u64) {
        if self.shared.borrow().ended {
            return;
        }
        let masters = self.shared.borrow().host.masters().unwrap_or_default();
        if let Err(error) = self.send_heartbeat(&masters, now_milliseconds, true, true) {
            panic!("{error}");
        }
    }

    /// Disconnect a client (`disconnectClient`).
    pub fn disconnect_client(&mut self, client: &ClientId, reason: &str) -> bool {
        let Some(key) = self.peers.iter().find_map(|(key, peer)| {
            if peer.player.client == *client || peer.splits.iter().any(|split| split.player.client == *client) {
                Some(key.clone())
            } else {
                None
            }
        }) else {
            return false;
        };
        // The donor propagates send failures and still drops; the trait is
        // infallible, so the drop (which always runs) is the contract.
        if let Some(peer) = self.peers.get_mut(&key) {
            let _ = Self::reliable(
                peer,
                Q2ServerEvent::Print {
                    level: 2,
                    text: format!("{reason}\n"),
                },
            );
            let _ = Self::reliable(peer, Q2ServerEvent::Disconnect);
            let remote = peer.remote.clone();
            let _ = peer.channel.send(&self.transport, &remote, &[], self.last_now);
        }
        self.drop(&key, reason);
        true
    }

    /// Drop a peer (`drop`).
    fn drop(&mut self, key: &str, reason: &str) {
        let Some(peer) = self.peers.get(key) else {
            return;
        };
        let players = players_of(peer);
        self.drop_resolved(key, &players, reason);
    }

    /// Drop a resolved peer, notifying captured players even when the packet
    /// already reaped the peer (the donor notifies through the held peer).
    fn drop_resolved(&mut self, key: &str, players: &[Q2ApplicationPlayer], reason: &str) {
        if let Some(mut peer) = self.peers.remove(key) {
            Self::close_download(&mut peer);
        }
        let clients: Vec<ClientId> = players.iter().map(|player| player.client.clone()).collect();
        self.pending.retain(|command| match &command.source {
            CommandSource::Remote { client } => !clients.contains(client),
            _ => true,
        });
        for player in players {
            self.shared.borrow_mut().host.disconnect(player, reason);
        }
    }

    /// Handle a connectionless packet (`connectionless`).
    fn connectionless(&mut self, from: &NetworkAddress, bytes: &[u8], now: u64) -> Result<bool, Q2NetworkError> {
        let protocol = self.shared.borrow().host.protocol();
        let Some(message) = read_q2_out_of_band(bytes, is_exact_kex(&protocol)) else {
            return Ok(false);
        };
        match message.command.as_str() {
            "rcon" => {
                // The administration surface borrows from the guard, which is
                // held across the handler; neither qa-net nor host rcon code
                // can reenter the network.
                let mut shared = self.shared.borrow_mut();
                let Some(admin) = shared.host.administration() else {
                    return Ok(true);
                };
                let mut adapter = ServerRcon {
                    admin,
                    transport: &self.transport,
                    protocols: vec![q2_receiver_protocol(&protocol)],
                    remote: from.clone(),
                };
                handle_q2_rcon_host(&mut adapter, from, &message, now)?;
                Ok(true)
            }
            "status" => {
                let status = self
                    .shared
                    .borrow()
                    .host
                    .discovery()
                    .map(|discovery| discovery.status());
                if let Some(status) = status {
                    let text = q2_status_text(&status, 1384)?;
                    self.reply(from, &format!("print\n{text}"));
                }
                Ok(true)
            }
            "info" => {
                let info = self.shared.borrow().host.discovery().map(|discovery| discovery.info());
                if let Some(info) = info {
                    let version = message
                        .arguments
                        .first()
                        .and_then(|text| text.parse::<i64>().ok())
                        .unwrap_or(0);
                    if let Some(text) = q2_info_text(&info, &[q2_receiver_protocol(&protocol)], version) {
                        self.reply(from, &text);
                    }
                }
                Ok(true)
            }
            "getchallenge" => {
                let reply = self.challenges.reply(from, now, &[q2_receiver_protocol(&protocol)]);
                let _ = self.transport.send(from, &reply);
                Ok(true)
            }
            "ping" => {
                self.reply(from, "ack");
                Ok(true)
            }
            "connect" => {
                self.connect(from, &message, now)?;
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    /// Admit a connect request (`connect` case).
    fn connect(
        &mut self,
        from: &NetworkAddress,
        message: &Q2ConnectionlessMessage,
        now: u64,
    ) -> Result<(), Q2NetworkError> {
        let request = read_q2_connect(message)?;
        let configured = self.shared.borrow().host.protocol();
        if request.protocol.version() != u32::from(configured.version()) {
            self.reply(from, "print\nUnsupported protocol.\n");
            return Ok(());
        }
        let admitted = if matches!(request.protocol, ProtocolIdentity::Q2Kex) {
            self.lan.as_ref().is_some_and(|lan| lan.admitted(from))
        } else {
            self.challenges.validate(from, request.challenge)
        };
        if !admitted {
            self.reply(from, "print\nBad challenge.\n");
            return Ok(());
        }
        if self.peers.contains_key(&address_key(from, true)) {
            self.accept_connection(from);
            return Ok(());
        }
        let seats = request.social_ids.as_ref().map_or(1, Vec::len);
        if self.clients().len() + seats > self.shared.borrow().host.max_clients() as usize {
            self.reply(from, "print\nServer is full.\n");
            return Ok(());
        }
        let mut primary = request.clone();
        primary.social_ids = request
            .social_ids
            .as_ref()
            .map(|ids| ids.iter().take(1).cloned().collect());
        // The Rust host admission drops the donor's `splitSeat`; the sliced
        // social identity still identifies the seat.
        let admission = self.shared.borrow_mut().host.admit(from, &primary);
        let player = match admission {
            Q2ApplicationAdmission::Accepted { player } => player,
            Q2ApplicationAdmission::Rejected { reason } => {
                self.reply(from, &format!("print\n{reason}\n"));
                return Ok(());
            }
        };
        let mut splits = Vec::new();
        let mut failure = None;
        if let Some(social_ids) = request.social_ids.as_ref() {
            for id in social_ids.iter().skip(1) {
                let mut additional = request.clone();
                additional.social_ids = Some(vec![id.clone()]);
                match self.shared.borrow_mut().host.admit(from, &additional) {
                    Q2ApplicationAdmission::Accepted { player } => splits.push(Q2ServerSplit {
                        player,
                        replay: Q2CommandReplay::new(),
                        sequence: 0,
                    }),
                    Q2ApplicationAdmission::Rejected { reason } => {
                        failure = Some(reason);
                        break;
                    }
                }
            }
        }
        if let Some(failure) = failure {
            let players: Vec<Q2ApplicationPlayer> = std::iter::once(player.clone())
                .chain(splits.iter().map(|split| split.player.clone()))
                .collect();
            for player in &players {
                self.shared.borrow_mut().host.disconnect(player, &failure);
            }
            self.reply(from, &format!("print\n{failure}\n"));
            return Ok(());
        }
        let offered = seed_protocol(&request.protocol)?;
        let protocol = negotiate_q2_protocol(&configured, &offered);
        let wire_protocol = q2_receiver_protocol(&protocol);
        let peer = Q2ServerPeer {
            remote: from.clone(),
            protocol,
            protocol_version: u32::from(protocol.version()),
            qport: request.qport,
            player,
            splits,
            download: Q2PeerDownload::new(),
            download_failure: None,
            channel: Q2Channel::new(Q2ChannelOptions {
                side: ChannelSide::Server,
                protocol: wire_protocol,
                channel: request.channel,
                qport: request.qport,
                payload_bytes: Some(request.payload_bytes),
                message_bytes: None,
                max_datagram_bytes: self.transport.max_datagram_bytes().or(Some(65507)),
                compress: request.compression,
                sequence_recording: None,
            })?,
            wire: Q2Wire::new(wire_protocol)?,
            replay: Q2CommandReplay::new(),
            frames: BTreeMap::new(),
            game_state: None,
            active: false,
            sequence: 0,
            last_received: now,
            datagram: Vec::new(),
            userinfo: request.userinfo.clone(),
        };
        self.peers.insert(address_key(from, true), peer);
        self.accept_connection(from);
        Ok(())
    }

    /// Queue a reliable event (`reliable`).
    fn reliable(peer: &mut Q2ServerPeer, event: Q2ServerEvent) -> Result<(), Q2NetworkError> {
        let bytes = encode_q2_server_event(&mut peer.wire, &event)?;
        peer.channel.queue_reliable(&bytes)?;
        Ok(())
    }

    /// Stuff a command (`stuff`).
    fn stuff(peer: &mut Q2ServerPeer, text: &str) -> Result<(), Q2NetworkError> {
        Self::reliable(peer, Q2ServerEvent::CommandText { text: text.to_string() })
    }

    /// Close a peer download (`closeDownload`).
    fn close_download(peer: &mut Q2ServerPeer) {
        peer.download.close();
        peer.download_failure = None;
    }

    /// Begin a peer download (`beginDownload`, resolved inline).
    fn begin_download(&mut self, key: &str, name: &str, offset: Option<&str>) {
        let generation = self.shared.borrow().server_generation;
        let Some(peer) = self.peers.get_mut(key) else {
            return;
        };
        let event = peer
            .download
            .begin(self.shared.borrow_mut().host.downloads(), name, offset);
        let revision = peer.download.revision();
        // The donor guards an async completion; the sync port resolves
        // inline, so the peer is still admitted exactly when the key holds.
        let current = !self.shared.borrow().ended
            && self.shared.borrow().server_generation == generation
            && self
                .peers
                .get(key)
                .is_some_and(|peer| peer.download.revision() == revision);
        if !current {
            return;
        }
        match event {
            Err(error) => {
                if let Some(peer) = self.peers.get_mut(key) {
                    peer.download_failure = Some(error.to_string());
                }
            }
            Ok(Some(event)) => {
                if let Some(peer) = self.peers.get_mut(key) {
                    if let Err(error) = Self::reliable(peer, event) {
                        peer.download_failure = Some(error.to_string());
                    }
                }
            }
            Ok(None) => {}
        }
    }

    /// Restart signon (`newClient`).
    fn new_client(&mut self, key: &str) -> Result<(), Q2NetworkError> {
        let Some(peer) = self.peers.get_mut(key) else {
            return Ok(());
        };
        Self::close_download(peer);
        peer.active = false;
        peer.frames.clear();
        peer.replay = Q2CommandReplay::new();
        for split in &mut peer.splits {
            split.replay = Q2CommandReplay::new();
        }
        peer.datagram = Vec::new();
        let mut state = self
            .shared
            .borrow_mut()
            .host
            .game_state(&peer.player.clone(), Some(peer.protocol));
        let generation = self.shared.borrow().server_generation;
        set_servercount(&mut state.data, generation);
        if !peer.splits.is_empty() {
            if let Q2ServerData::Kex(data) = &mut state.data {
                data.clientnums = std::iter::once(peer.player.source_entity)
                    .chain(peer.splits.iter().map(|split| split.player.source_entity))
                    .map(|entity| entity.wrapping_sub(1) as i16)
                    .collect();
            }
        }
        peer.game_state = Some(state.clone());
        Self::reliable(
            peer,
            Q2ServerEvent::ServerData {
                data: Box::new(state.data.clone()),
            },
        )?;
        Self::stuff(peer, &format!("cmd configstrings {} 0\n", state.data.servercount()))?;
        Ok(())
    }

    /// Send one signon page (`signonPage`).
    fn signon_page(&mut self, key: &str, kind: &str, start: i32) -> Result<(), Q2NetworkError> {
        let Some(peer) = self.peers.get_mut(key) else {
            return Ok(());
        };
        let Some(state) = peer.game_state.clone() else {
            return Err(Q2NetworkError::Message("Q2 signon has no game state".to_string()));
        };
        if start < 0 {
            return Err(Q2NetworkError::Message("Q2 signon index is negative".to_string()));
        }
        let mut entries: Vec<(u32, Vec<u8>)> = Vec::new();
        if kind == "configstrings" {
            let mut indexes: Vec<u32> = state
                .config_strings
                .keys()
                .copied()
                .filter(|index| *index >= start as u32)
                .collect();
            indexes.sort_unstable();
            for index in indexes {
                let value = state.config_strings.get(&index).cloned().unwrap_or_default();
                let wire_index = u16::try_from(index)
                    .map_err(|_| Q2NetworkError::Message("Q2 signon index exceeds configstring space".to_string()))?;
                let bytes = encode_q2_server_event(
                    &mut peer.wire,
                    &Q2ServerEvent::ConfigString {
                        index: wire_index,
                        value,
                    },
                )?;
                entries.push((index, bytes));
            }
        } else {
            let mut indexes: Vec<u32> = state
                .baselines
                .keys()
                .copied()
                .filter(|index| *index >= start as u32)
                .collect();
            indexes.sort_unstable();
            for index in indexes {
                let entity = state.baselines.get(&index).cloned().unwrap_or_default();
                let bytes = encode_q2_server_event(&mut peer.wire, &Q2ServerEvent::Baseline { entity })?;
                entries.push((index, bytes));
            }
        }
        let capacity = peer.channel.capacity().saturating_sub(96);
        let mut length = 0;
        let mut next = None;
        for (index, bytes) in &entries {
            if length + bytes.len() > capacity {
                next = Some(*index);
                break;
            }
            peer.channel.queue_reliable(bytes)?;
            length += bytes.len();
        }
        if next.is_some() && length == 0 {
            return Err(Q2NetworkError::Message(
                "Q2 signon record exceeds negotiated reliable message capacity".to_string(),
            ));
        }
        if let Some(next) = next {
            Self::stuff(peer, &format!("cmd {kind} {} {next}\n", state.data.servercount()))?;
        } else if kind == "configstrings" {
            Self::stuff(peer, &format!("cmd baselines {} 0\n", state.data.servercount()))?;
        } else {
            Self::stuff(peer, &format!("precache {}\n", state.data.servercount()))?;
        }
        Ok(())
    }

    /// Handle a client command (`clientCommand`).
    fn client_command(&mut self, key: &str, text: &str, player: &Q2ApplicationPlayer) -> Result<(), Q2NetworkError> {
        let mut text = text.to_string();
        if let Some(expanded) = self.shared.borrow_mut().host.expand_client_command(&text) {
            text = expanded;
        }
        let words = tokenize_command(&text, Dialect::Q2Classic, TextMode::Source)?.argv;
        let Some(name) = words.first().cloned() else {
            return Ok(());
        };
        if name == "disconnect" {
            self.drop(key, "Client disconnected");
            return Ok(());
        }
        if name == "new" {
            self.new_client(key)?;
            return Ok(());
        }
        if name == "download" {
            let name = words.get(1).cloned().unwrap_or_default();
            let offset = words.get(2).cloned();
            self.begin_download(key, &name, offset.as_deref());
            return Ok(());
        }
        if name == "nextdl" {
            let event = self
                .peers
                .get_mut(key)
                .map(|peer| peer.download.next())
                .transpose()?
                .flatten();
            if let (Some(peer), Some(event)) = (self.peers.get_mut(key), event) {
                Self::reliable(peer, event)?;
            }
            return Ok(());
        }
        if name == "configstrings" || name == "baselines" || name == "begin" {
            let state = self.peers.get(key).and_then(|peer| peer.game_state.clone());
            let Some(state) = state else {
                self.new_client(key)?;
                return Ok(());
            };
            if signon_integer(words.get(1).map(String::as_str))? != state.data.servercount() {
                self.new_client(key)?;
                return Ok(());
            }
            if name == "begin" {
                let players: Vec<Q2ApplicationPlayer> = self
                    .peers
                    .get(key)
                    .map(|peer| {
                        std::iter::once(peer.player.clone())
                            .chain(peer.splits.iter().map(|split| split.player.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(peer) = self.peers.get_mut(key) {
                    if !peer.active {
                        for player in &players {
                            self.shared.borrow_mut().host.begin(player);
                        }
                    }
                    peer.active = true;
                }
                return Ok(());
            }
            let start = signon_integer(words.get(2).map(String::as_str))?;
            self.signon_page(key, &name, start)?;
            return Ok(());
        }
        let active = self.peers.get(key).is_some_and(|peer| peer.active);
        if active {
            // The donor prefers raw `commandText` when the host defines it,
            // and every donor host does; the word entry serves callers that
            // only hold parsed words, so the network always sends raw text.
            self.shared.borrow_mut().host.command_text(player, &text);
        }
        Ok(())
    }
}

impl<T, H> Q2ServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q2ApplicationServerHost,
{
    /// Resolve a seat player (`seat === 0` is the primary).
    fn seat_player(&self, key: &str, seat: u8) -> Result<Option<Q2ApplicationPlayer>, Q2NetworkError> {
        let Some(peer) = self.peers.get(key) else {
            return Ok(None);
        };
        if seat == 0 {
            return Ok(Some(peer.player.clone()));
        }
        match peer.splits.get(seat as usize - 1) {
            Some(split) => Ok(Some(split.player.clone())),
            None => Err(Q2NetworkError::Message("Missing admitted Q2 split owner".to_string())),
        }
    }

    /// Handle a channel message (`process`).
    fn process(&mut self, key: &str, bytes: Vec<u8>, sequence: u32, dropped: u32) -> Result<(), Q2NetworkError> {
        let records = {
            let Some(peer) = self.peers.get_mut(key) else {
                return Ok(());
            };
            let seats = peer.splits.len() as u8 + 1;
            read_q2_client_messages(&mut peer.wire, &bytes, sequence, seats)?
        };
        for record in records {
            let seat = record.seat;
            match &record.event {
                Q2ClientEvent::Move { .. } | Q2ClientEvent::BatchMove { .. } => {
                    if !self.peers.get(key).is_some_and(|peer| peer.active) {
                        continue;
                    }
                    // Resolve the owner before borrowing replay state; the
                    // shared guard below never crosses a `self` call.
                    let split = if seat == 0 {
                        None
                    } else {
                        match self.peers.get(key).and_then(|peer| peer.splits.get(seat as usize - 1)) {
                            Some(_) => Some(seat as usize - 1),
                            None => {
                                return Err(Q2NetworkError::Message("Missing admitted Q2 split owner".to_string()));
                            }
                        }
                    };
                    let mut shared = self.shared.borrow_mut();
                    let pending = &mut self.pending;
                    let Some(peer) = self.peers.get_mut(key) else {
                        return Ok(());
                    };
                    if let Some(index) = split {
                        let split = &mut peer.splits[index];
                        let player = split.player.clone();
                        let mut sequence = split.sequence;
                        split.replay.execute(&record.event, dropped as usize, |command| {
                            let input = shared.host.input(&player, command, sequence);
                            sequence += 1;
                            if let Some(input) = input {
                                pending.push(input);
                            }
                        })?;
                        split.sequence = sequence;
                    } else {
                        let player = peer.player.clone();
                        let mut sequence = peer.sequence;
                        peer.replay.execute(&record.event, dropped as usize, |command| {
                            let input = shared.host.input(&player, command, sequence);
                            sequence += 1;
                            if let Some(input) = input {
                                pending.push(input);
                            }
                        })?;
                        peer.sequence = sequence;
                    }
                }
                Q2ClientEvent::Command(text) => {
                    let Some(player) = self.seat_player(key, seat)? else {
                        return Ok(());
                    };
                    self.client_command(key, text, &player)?;
                    if !self.peers.contains_key(key) {
                        return Ok(());
                    }
                }
                Q2ClientEvent::Userinfo(text) => {
                    let Some(peer) = self.peers.get(key) else {
                        return Ok(());
                    };
                    let kex = is_exact_kex(&peer.protocol);
                    let player = peer.player.clone();
                    let splits: Vec<Q2ApplicationPlayer> =
                        peer.splits.iter().map(|split| split.player.clone()).collect();
                    let main = if kex {
                        q2_kex_seat_userinfo(text, 0)?
                    } else {
                        text.clone()
                    };
                    self.shared.borrow_mut().host.userinfo(&player, &main);
                    for (seat, split) in splits.iter().enumerate() {
                        let value = q2_kex_seat_userinfo(text, seat as u32 + 1)?;
                        self.shared.borrow_mut().host.userinfo(split, &value);
                    }
                    if let Some(peer) = self.peers.get_mut(key) {
                        peer.userinfo.clone_from(text);
                    }
                }
                Q2ClientEvent::UserinfoDelta { name, value } => {
                    let Some(peer) = self.peers.get(key) else {
                        return Ok(());
                    };
                    let player = peer.player.clone();
                    self.shared
                        .borrow_mut()
                        .host
                        .command(&player, "userinfo_delta", &[name.clone(), value.clone()]);
                }
                Q2ClientEvent::Setting { index, value } => {
                    let Some(peer) = self.peers.get(key) else {
                        return Ok(());
                    };
                    let player = peer.player.clone();
                    self.shared.borrow_mut().host.command(
                        &player,
                        "set_setting",
                        &[index.to_string(), value.to_string()],
                    );
                }
                Q2ClientEvent::Nop => {}
            }
        }
        Ok(())
    }
}

/// Primary plus split players of a peer.
fn players_of(peer: &Q2ServerPeer) -> Vec<Q2ApplicationPlayer> {
    std::iter::once(peer.player.clone())
        .chain(peer.splits.iter().map(|split| split.player.clone()))
        .collect()
}

/// MVD clock in 10 Hz ticks (`mvdTime`).
fn mvd_time<H>(shared: &Q2ServerShared<H>) -> Result<i64, Q2NetworkError> {
    let Some((output, _)) = shared.mvd_frame.as_ref() else {
        return Err(Q2NetworkError::Message("MVD capture has no source clock".to_string()));
    };
    let ticks = match &output.snapshot.frame.time {
        SourceTime::Seconds(value) => f64::from(*value) * 10.0,
        SourceTime::Milliseconds(value) => f64::from(*value) / 100.0,
    };
    Ok((ticks + 1e-7).floor() as i64)
}

/// Authoritative MVD capture (`captureMvd`).
fn capture_mvd<H: Q2ApplicationServerHost>(shared: &mut Q2ServerShared<H>) -> Result<MvdCapture, Q2NetworkError> {
    let failed =
        || Q2NetworkError::Message("MVD recording requires an active authoritative Q2 capture source".to_string());
    if shared.ended {
        return Err(failed());
    }
    let Some((output, events)) = shared.mvd_frame.clone() else {
        return Err(failed());
    };
    let generation = shared.server_generation;
    shared.host.mvd_capture(&output, &events, generation).ok_or_else(failed)
}

impl<H: Q2ApplicationServerHost> ApplicationNetworkRecording for Q2ServerTap<H> {
    fn seed(&self) -> Result<DemoRecordingSeed, ApplicationNetworkError> {
        let failed = |error: &dyn ToString| ApplicationNetworkError::Message(error.to_string());
        let mut shared = self.shared.borrow_mut();
        let capture = capture_mvd(&mut shared).map_err(|error| failed(&error))?;
        match self.kind {
            Q2ServerTapKind::ServerDemo => {
                if capture.revision != 2010 {
                    return Err(ApplicationNetworkError::Message(
                        "serverrecord requires classic Quake II; use mvdrecord for other source revisions".to_string(),
                    ));
                }
                if shared.server_demo_owner.is_some() {
                    return Err(ApplicationNetworkError::Message(
                        "Already doing a serverrecord".to_string(),
                    ));
                }
                let state = Q2ServerDemoState {
                    servercount: capture.servercount,
                    gamedir: capture.gamedir.clone(),
                    config_strings: capture.config_strings.clone(),
                };
                let message = encode_q2_server_demo_signon(&state).map_err(|error| failed(&error))?;
                *self.seed.borrow_mut() = Some(capture);
                Ok(DemoRecordingSeed {
                    identity: DemoRecordingIdentity::Q2Server,
                    packets: vec![DemoRecordingPacket::Q2Server { message }],
                })
            }
            Q2ServerTapKind::Mvd => {
                let revision = match capture.revision {
                    2009 => MvdRevision::R2009,
                    2010 => MvdRevision::R2010,
                    2011 => MvdRevision::R2011,
                    2012 => MvdRevision::R2012,
                    2013 => MvdRevision::R2013,
                    3038 => MvdRevision::R3038,
                    _ => {
                        return Err(ApplicationNetworkError::Message(
                            "Unsupported MVD recording revision".to_string(),
                        ));
                    }
                };
                let mut encoder = MvdEncoder::new();
                let priming = MvdCapture {
                    messages: Vec::new(),
                    ..capture.clone()
                };
                let packets = encoder
                    .capture(&priming)
                    .map_err(|error| failed(&error))?
                    .into_iter()
                    .map(|message| DemoRecordingPacket::Mvd { message })
                    .collect();
                *self.seed.borrow_mut() = Some(capture);
                Ok(DemoRecordingSeed {
                    identity: DemoRecordingIdentity::Mvd { revision },
                    packets,
                })
            }
        }
    }

    fn attach(&mut self, sink: Box<dyn DemoRecordingSink>) -> Result<Box<dyn FnOnce() + '_>, ApplicationNetworkError> {
        let failed = |text: &str| ApplicationNetworkError::Message(text.to_string());
        match self.kind {
            Q2ServerTapKind::ServerDemo => {
                let seed = self.seed.borrow().clone();
                {
                    let shared = self.shared.borrow();
                    let stale = seed
                        .as_ref()
                        .is_none_or(|seed| seed.servercount != shared.server_generation);
                    if shared.ended || shared.server_demo_owner.is_some() || stale {
                        return Err(failed("Server recording requires a current unused seed"));
                    }
                }
                let seed = seed.expect("seed checked");
                *self.seed.borrow_mut() = None;
                let mut shared = self.shared.borrow_mut();
                shared.detach_counter += 1;
                let id = shared.detach_counter;
                shared.server_demo_owner = Some(Q2ServerDemoOwner {
                    sink,
                    configs: seed.config_strings.clone(),
                    id,
                });
                let cell = self.shared.clone();
                Ok(Box::new(move || {
                    let matched = cell
                        .borrow()
                        .server_demo_owner
                        .as_ref()
                        .is_some_and(|owner| owner.id == id);
                    if matched {
                        cell.borrow_mut().server_demo_owner = None;
                    }
                }))
            }
            Q2ServerTapKind::Mvd => {
                let seed = self.seed.borrow().clone();
                {
                    let shared = self.shared.borrow();
                    if shared.ended || shared.mvd_owner.is_some() || seed.is_none() {
                        return Err(failed("MVD recording requires an unused server seed"));
                    }
                }
                let seed = seed.expect("seed checked");
                let mut encoder = MvdEncoder::new();
                let priming = MvdCapture {
                    messages: Vec::new(),
                    ..seed
                };
                encoder.capture(&priming).map_err(|error| failed(&error.to_string()))?;
                *self.seed.borrow_mut() = None;
                let mut shared = self.shared.borrow_mut();
                shared.detach_counter += 1;
                let id = shared.detach_counter;
                shared.mvd_owner = Some(Q2MvdOwner { sink, encoder, id });
                if shared.mvd_broadcast.is_none() {
                    let tick = mvd_time(&shared).map_err(|error| failed(&error.to_string()))?;
                    shared.mvd_tick = Some(tick);
                    shared.mvd_messages = Vec::new();
                }
                let cell = self.shared.clone();
                Ok(Box::new(move || {
                    let matched = cell.borrow().mvd_owner.as_ref().is_some_and(|owner| owner.id == id);
                    if matched {
                        cell.borrow_mut().mvd_owner = None;
                    }
                }))
            }
        }
    }
}

impl<T, H> Q2ServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q2ApplicationServerHost,
{
    /// Start or stop the multiview broadcast (`configureMvd`).
    fn configure_mvd(&mut self) -> Result<(), Q2NetworkError> {
        let settings = self.shared.borrow().host.mvd_settings();
        if settings.as_ref().is_none_or(|settings| !settings.enabled) {
            self.shared.borrow_mut().mvd_broadcast = None;
            return Ok(());
        }
        let settings = settings.expect("settings checked");
        if self.shared.borrow().mvd_broadcast.is_some() {
            *self.shared.borrow().mvd_password.borrow_mut() = settings.password.clone();
            return Ok(());
        }
        let address = self.transport.address();
        let (host, port) = match &address {
            NetworkAddress::Ipv4 { host, port } => (format!("{}.{}.{}.{}", host[0], host[1], host[2], host[3]), *port),
            NetworkAddress::Ipv6 { host, port } => (host.clone(), *port),
            _ => {
                return Err(Q2NetworkError::Message(
                    "GTV broadcast requires an IP server transport".to_string(),
                ));
            }
        };
        // The donor checks the optional capture hook statically; the port
        // probes it once a published frame exists.
        let published = self.shared.borrow().mvd_frame.clone();
        if let Some((output, events)) = published {
            let generation = self.shared.borrow().server_generation;
            let capable = self
                .shared
                .borrow_mut()
                .host
                .mvd_capture(&output, &events, generation)
                .is_some();
            if !capable {
                return Err(Q2NetworkError::Message(
                    "GTV broadcast requires an authoritative Q2 capture source".to_string(),
                ));
            }
        }
        *self.shared.borrow().mvd_password.borrow_mut() = settings.password.clone();
        let password = self.shared.borrow().mvd_password.clone();
        let mut options = MvdBroadcastOptions::new(move |hello| hello.password == *password.borrow());
        options.max_viewers = settings.max_viewers as usize;
        let mut broadcast = MvdBroadcast::new(options)?;
        match broadcast.listen(&host, port) {
            Ok(_) => {
                if self.shared.borrow().ended {
                    return Ok(());
                }
                self.shared.borrow_mut().mvd_broadcast = Some(broadcast);
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Handle one packet for a resolved peer (`poll` body).
    fn handle_packet(
        &mut self,
        from: &NetworkAddress,
        payload: &[u8],
        now: u64,
        resolved: &mut Option<(String, Vec<Q2ApplicationPlayer>)>,
    ) -> Result<(), Q2NetworkError> {
        if self.connectionless(from, payload, now)? {
            return Ok(());
        }
        if resolved.is_none() {
            *resolved = self.peers.iter().find_map(|(key, peer)| {
                let roamed = same_address(&peer.remote, from, false)
                    && payload.len() >= 10
                    && if peer.protocol_version == 34 {
                        u32::from(u16::from_le_bytes([payload[8], payload[9]])) == peer.qport
                    } else {
                        payload[8] == (peer.qport & 255) as u8
                    };
                roamed.then(|| (key.clone(), players_of(peer)))
            });
        }
        let Some((key, _)) = resolved.clone() else {
            return Ok(());
        };
        let outcome = {
            let Some(peer) = self.peers.get_mut(&key) else {
                return Ok(());
            };
            peer.channel.receive(payload, now)?
        };
        if !matches!(outcome, Q2ChannelReceive::Rejected { .. }) {
            if let Some(peer) = self.peers.get_mut(&key) {
                peer.last_received = now;
            }
        }
        if let Q2ChannelReceive::Message {
            bytes,
            sequence,
            dropped,
            ..
        } = outcome
        {
            let migrated = self
                .peers
                .get(&key)
                .is_some_and(|peer| !same_address(&peer.remote, from, true));
            if migrated {
                if let Some(mut peer) = self.peers.remove(&key) {
                    peer.remote = from.clone();
                    let new_key = address_key(&peer.remote, true);
                    *resolved = Some((new_key.clone(), players_of(&peer)));
                    self.peers.insert(new_key.clone(), peer);
                    self.process(&new_key, bytes, sequence, dropped)?;
                }
            } else {
                self.process(&key, bytes, sequence, dropped)?;
            }
        }
        Ok(())
    }

    /// Poll the transport (`poll`).
    pub fn poll(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, Q2NetworkError> {
        if let Some(lan) = &self.lan {
            lan.tick(now_milliseconds as f64)?;
        }
        self.last_now = now_milliseconds;
        if self.shared.borrow().ended {
            return Ok(Vec::new());
        }
        self.configure_mvd()?;
        let masters = self.shared.borrow().host.masters().unwrap_or_default();
        if !masters.is_empty() {
            self.send_heartbeat(&masters, now_milliseconds, true, false)?;
        }
        let failures: Vec<(String, String)> = self
            .peers
            .iter()
            .filter_map(|(key, peer)| peer.download_failure.clone().map(|reason| (key.clone(), reason)))
            .collect();
        for (key, reason) in &failures {
            self.drop(key, reason);
        }
        if let Some(broadcast) = self.shared.borrow_mut().mvd_broadcast.as_mut() {
            // Sync adaptation: the donor broadcast drives its own sockets;
            // the port pumps it once per poll.
            broadcast.poll()?;
        }
        while let Some(packet) = self.transport.poll()? {
            let (from, payload) = match packet {
                ReceiveEvent::Packet { from, payload, .. } => (from, payload),
                ReceiveEvent::Error { error } => {
                    self.shared.borrow_mut().host.print(&format!("{error}\n"));
                    continue;
                }
                ReceiveEvent::Dropped { .. } => continue,
            };
            if self.shared.borrow().host.rejects(&from) {
                continue;
            }
            let direct = address_key(&from, true);
            let mut resolved = self.peers.get(&direct).map(|peer| (direct.clone(), players_of(peer)));
            if let Err(error) = self.handle_packet(&from, &payload, now_milliseconds, &mut resolved) {
                let reason = error.to_string();
                match resolved {
                    Some((key, players)) => self.drop_resolved(&key, &players, &reason),
                    None => self.reply(&from, &format!("print\n{reason}\n")),
                }
            }
        }
        let keys: Vec<String> = self.peers.keys().cloned().collect();
        for key in keys {
            let timed_out = self.peers.get(&key).is_some_and(|peer| {
                now_milliseconds.saturating_sub(peer.last_received) > self.timeout_milliseconds.unwrap_or(125_000)
            });
            if timed_out {
                self.drop(&key, "Connection timed out");
                continue;
            }
            let resend = self
                .peers
                .get(&key)
                .is_some_and(|peer| !peer.active && peer.channel.should_update(now_milliseconds));
            if resend {
                if let Some(peer) = self.peers.get_mut(&key) {
                    let remote = peer.remote.clone();
                    peer.channel.send(&self.transport, &remote, &[], now_milliseconds)?;
                }
            }
        }
        Ok(std::mem::take(&mut self.pending))
    }

    /// Reject submitted commands (`submit`).
    pub fn submit(&mut self, _commands: &[ActorCommand], _now_milliseconds: u64) -> Result<(), Q2NetworkError> {
        Err(Q2NetworkError::Message(
            "Q2 server commands must enter the authoritative application input batch".to_string(),
        ))
    }

    /// Publish a simulation step (`publish`).
    pub fn publish(
        &mut self,
        output: &SimulationOutput,
        events: &[NetworkPresentationEvent],
        now_milliseconds: u64,
    ) -> Result<(), Q2NetworkError> {
        if self.shared.borrow().ended {
            return Ok(());
        }
        self.shared.borrow_mut().host.observe(output, events);
        self.shared.borrow_mut().mvd_frame = Some((output.clone(), events.to_vec()));
        let recording = {
            let shared = self.shared.borrow();
            shared.mvd_owner.is_some() || shared.mvd_broadcast.is_some() || shared.server_demo_owner.is_some()
        };
        let current = if recording {
            Some(capture_mvd(&mut self.shared.borrow_mut())?)
        } else {
            None
        };
        if self.shared.borrow().server_demo_owner.is_some() {
            let current = current.as_ref().expect("capture taken with recording owners");
            let mut multicasts: Vec<Vec<u8>> = current
                .messages
                .iter()
                .filter(|message| {
                    !matches!(message.recipient, MvdRecipient::Player(_))
                        && matches!(message.bytes.first(), Some(1 | 2 | 3 | 9))
                })
                .map(|message| message.bytes.clone())
                .collect();
            {
                let mut shared = self.shared.borrow_mut();
                let owner = shared
                    .server_demo_owner
                    .as_mut()
                    .expect("server recording owner admitted");
                let deltas: Vec<(u16, String)> = current
                    .config_strings
                    .iter()
                    .filter(|(index, value)| owner.configs.get(index) != Some(value))
                    .map(|(index, value)| (*index, value.clone()))
                    .collect();
                owner.configs.clone_from(&current.config_strings);
                for (index, value) in deltas {
                    let bytes = encode_q2_server_event(
                        &mut shared.server_demo_wire,
                        &Q2ServerEvent::ConfigString { index, value },
                    )?;
                    multicasts.push(bytes);
                }
            }
            let message = encode_q2_server_demo_frame(output.snapshot.frame.frame, &current.entities, &multicasts)?;
            let failed = self
                .shared
                .borrow_mut()
                .server_demo_owner
                .as_mut()
                .expect("server recording owner admitted")
                .sink
                .append(&DemoRecordingPacket::Q2Server { message })
                .err();
            if let Some(error) = failed {
                let mut shared = self.shared.borrow_mut();
                shared.server_demo_owner = None;
                shared.host.print(&format!("Server recording failed: {error}\n"));
            }
        }
        let mvd_recording = {
            let shared = self.shared.borrow();
            shared.mvd_owner.is_some() || shared.mvd_broadcast.is_some()
        };
        if mvd_recording {
            let current = current.as_ref().expect("capture taken with recording owners");
            let tick = mvd_time(&self.shared.borrow())?;
            self.shared
                .borrow_mut()
                .mvd_messages
                .extend(current.messages.iter().cloned());
            if self.shared.borrow().mvd_tick != Some(tick) {
                self.shared.borrow_mut().mvd_tick = Some(tick);
                let mut capture = current.clone();
                capture.messages = std::mem::take(&mut self.shared.borrow_mut().mvd_messages);
                if let Some(broadcast) = self.shared.borrow_mut().mvd_broadcast.as_mut() {
                    broadcast.observe(&capture)?;
                }
                if self.shared.borrow().mvd_owner.is_some() {
                    let packets = self
                        .shared
                        .borrow_mut()
                        .mvd_owner
                        .as_mut()
                        .expect("multiview owner admitted")
                        .encoder
                        .capture(&capture)?;
                    let mut failure = None;
                    for message in &packets {
                        let packet = DemoRecordingPacket::Mvd {
                            message: message.clone(),
                        };
                        let result = self
                            .shared
                            .borrow_mut()
                            .mvd_owner
                            .as_mut()
                            .expect("multiview owner admitted")
                            .sink
                            .append(&packet);
                        if failure.is_none() {
                            failure = result.err();
                        }
                    }
                    if let Some(error) = failure {
                        let mut shared = self.shared.borrow_mut();
                        shared.mvd_owner = None;
                        shared.host.print(&format!("MVD recording failed: {error}\n"));
                    }
                }
            }
        } else {
            let mut shared = self.shared.borrow_mut();
            shared.mvd_tick = None;
            shared.mvd_messages = Vec::new();
        }
        let keys: Vec<String> = self.peers.keys().cloned().collect();
        for key in keys {
            let Some(player) = self.peers.get(&key).map(|peer| peer.player.clone()) else {
                continue;
            };
            let raw = self.shared.borrow().host.raw_messages(&player);
            for message in raw {
                let Some(peer) = self.peers.get_mut(&key) else {
                    break;
                };
                if message.reliable {
                    peer.channel.queue_reliable(&message.bytes)?;
                } else if peer.active {
                    peer.datagram.push(message.bytes);
                }
            }
            let splits: Vec<Q2ApplicationPlayer> = self
                .peers
                .get(&key)
                .map(|peer| peer.splits.iter().map(|split| split.player.clone()).collect())
                .unwrap_or_default();
            for (seat, split) in splits.iter().enumerate() {
                let prefix = [21u8, seat as u8 + 2];
                let raw = self.shared.borrow().host.raw_messages(split);
                for message in raw {
                    let mut bytes = Vec::with_capacity(4 + message.bytes.len());
                    bytes.extend_from_slice(&prefix);
                    bytes.extend_from_slice(&message.bytes);
                    bytes.extend_from_slice(&[21u8, 1u8]);
                    let Some(peer) = self.peers.get_mut(&key) else {
                        break;
                    };
                    if message.reliable {
                        peer.channel.queue_reliable(&bytes)?;
                    } else if peer.active {
                        peer.datagram.push(bytes);
                    }
                }
                if !self.peers.get(&key).is_some_and(|peer| peer.active) {
                    continue;
                }
                let events = self.shared.borrow_mut().host.events(split, output, events);
                for event in events {
                    let Some(peer) = self.peers.get_mut(&key) else {
                        break;
                    };
                    // The Rust event flag is total; hosts carry the donor
                    // default (unreliable only for sound, muzzle flash, and
                    // temporary entities).
                    let reliable = event.reliable;
                    let mut bytes = vec![21u8, seat as u8 + 2];
                    bytes.extend_from_slice(&encode_q2_server_event(&mut peer.wire, &event.event)?);
                    bytes.extend_from_slice(&[21u8, 1u8]);
                    if reliable {
                        peer.channel.queue_reliable(&bytes)?;
                    } else {
                        peer.datagram.push(bytes);
                    }
                }
            }
            if !self.peers.get(&key).is_some_and(|peer| peer.active) {
                continue;
            }
            let protocol = self.peers.get(&key).map(|peer| peer.protocol).expect("admitted peer");
            let primary = self.shared.borrow().host.frame(&player, output, Some(protocol));
            let mut split_frames = Vec::with_capacity(splits.len());
            for split in &splits {
                split_frames.push(self.shared.borrow().host.frame(split, output, Some(protocol)));
            }
            let mut frame = primary;
            if !split_frames.is_empty() {
                let mut entities: BTreeMap<u16, EntityState> = BTreeMap::new();
                for entity in frame
                    .entities
                    .drain(..)
                    .chain(split_frames.iter().flat_map(|split| split.entities.iter().cloned()))
                {
                    entities.insert(entity.number, entity);
                }
                frame.split_players = split_frames
                    .iter()
                    .map(|split| Q2SplitPlayer {
                        area_bits: split.area_bits.clone(),
                        player: split.player.clone(),
                    })
                    .collect();
                frame.entities = entities.into_values().collect();
            }
            let events = self.shared.borrow_mut().host.events(&player, output, events);
            for event in events {
                let Some(peer) = self.peers.get_mut(&key) else {
                    break;
                };
                let bytes = encode_q2_server_event(&mut peer.wire, &event.event)?;
                if event.reliable {
                    peer.channel.queue_reliable(&bytes)?;
                } else {
                    peer.datagram.push(bytes);
                }
            }
            if self.peers.get(&key).is_some_and(|peer| peer.channel.fragment_pending()) {
                if let Some(peer) = self.peers.get_mut(&key) {
                    let remote = peer.remote.clone();
                    peer.channel.send(&self.transport, &remote, &[], now_milliseconds)?;
                }
                continue;
            }
            if self
                .peers
                .get(&key)
                .is_some_and(|peer| peer.frames.contains_key(&frame.server_frame))
            {
                let update = self
                    .peers
                    .get(&key)
                    .is_some_and(|peer| peer.channel.should_update(now_milliseconds));
                if update {
                    if let Some(peer) = self.peers.get_mut(&key) {
                        let remote = peer.remote.clone();
                        peer.channel.send(&self.transport, &remote, &[], now_milliseconds)?;
                    }
                }
                continue;
            }
            let old = {
                let Some(peer) = self.peers.get(&key) else {
                    continue;
                };
                if peer.replay.last_frame < 0 {
                    None
                } else {
                    peer.frames.get(&peer.replay.last_frame).cloned()
                }
            };
            let baseline = {
                let Some(peer) = self.peers.get(&key) else {
                    continue;
                };
                let Some(state) = peer.game_state.as_ref() else {
                    return Err(Q2NetworkError::Message("Active Q2 client has no baselines".to_string()));
                };
                state.baselines.clone()
            };
            let mut baseline_wire = HashMap::with_capacity(baseline.len());
            for (index, entity) in &baseline {
                let index = u16::try_from(*index)
                    .map_err(|_| Q2NetworkError::Message("Q2 baseline index exceeds entity space".to_string()))?;
                baseline_wire.insert(index, entity.clone());
            }
            let max_clients = self.shared.borrow().host.max_clients();
            let frame_bytes = {
                let Some(peer) = self.peers.get(&key) else {
                    continue;
                };
                encode_q2_frame(&peer.wire, &frame, old.as_ref(), &baseline_wire, max_clients)?
            };
            let Some(peer) = self.peers.get_mut(&key) else {
                continue;
            };
            let mut datagram = frame_bytes;
            for chunk in peer.datagram.drain(..) {
                datagram.extend_from_slice(&chunk);
            }
            let remote = peer.remote.clone();
            peer.channel
                .send(&self.transport, &remote, &datagram, now_milliseconds)?;
            peer.frames.insert(frame.server_frame, frame);
            while peer.frames.len() > 16 {
                let Some(oldest) = peer.frames.keys().next().copied() else {
                    break;
                };
                peer.frames.remove(&oldest);
            }
        }
        Ok(())
    }

    /// Close the server (`close`).
    pub fn close(&mut self) {
        if self.shared.borrow().ended {
            return;
        }
        self.shared.borrow_mut().ended = true;
        self.shared.borrow_mut().mvd_owner = None;
        self.shared.borrow_mut().mvd_broadcast = None;
        self.shared.borrow_mut().server_demo_owner = None;
        self.server_tap.seed.borrow_mut().take();
        self.mvd_tap.seed.borrow_mut().take();
        self.shared.borrow_mut().mvd_frame = None;
        self.shared.borrow_mut().mvd_tick = None;
        self.shared.borrow_mut().mvd_messages = Vec::new();
        let mut failures: Vec<String> = Vec::new();
        let masters = self.shared.borrow().host.masters().unwrap_or_default();
        if let Err(error) = self.send_heartbeat(&masters, self.last_now, false, true) {
            failures.push(error.to_string());
        }
        let keys: Vec<String> = self.peers.keys().cloned().collect();
        for key in keys {
            if let Some(peer) = self.peers.get_mut(&key) {
                Self::close_download(peer);
                let remote = peer.remote.clone();
                let sent = match Self::reliable(peer, Q2ServerEvent::Disconnect) {
                    Err(error) => Err(error),
                    Ok(()) => peer
                        .channel
                        .send(&self.transport, &remote, &[], self.last_now)
                        .map(|_| ())
                        .map_err(Q2NetworkError::from),
                };
                if let Err(error) = sent {
                    failures.push(error.to_string());
                }
            }
            if let Some(peer) = self.peers.remove(&key) {
                for player in players_of(&peer) {
                    self.shared.borrow_mut().host.disconnect(&player, "Server shutdown");
                }
            }
        }
        self.peers.clear();
        self.pending.clear();
        self.transport.close();
        // Synchronous sinks already wrote inline and the broadcast closes by
        // drop; without a fallible close the shutdown report goes to the host.
        if !failures.is_empty() {
            self.shared
                .borrow_mut()
                .host
                .print(&format!("Q2 server shutdown failed: {}\n", failures.join("; ")));
        }
    }
}

impl<T, H> ApplicationNetwork for Q2ServerNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q2ApplicationServerHost,
{
    fn server_recording(&mut self) -> Option<&mut dyn ApplicationNetworkRecording> {
        Some(&mut self.server_tap)
    }

    fn mvd_recording(&mut self) -> Option<&mut dyn ApplicationNetworkRecording> {
        Some(&mut self.mvd_tap)
    }

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
        self.wire.clone()
    }

    fn poll(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, ApplicationNetworkError> {
        self.poll(now_milliseconds)
            .map_err(|error| ApplicationNetworkError::Message(error.to_string()))
    }

    fn submit(&mut self, commands: &[ActorCommand], now_milliseconds: u64) -> Result<(), ApplicationNetworkError> {
        self.submit(commands, now_milliseconds)
            .map_err(|error| ApplicationNetworkError::Message(error.to_string()))
    }

    fn publish(
        &mut self,
        output: &SimulationOutput,
        events: &[NetworkPresentationEvent],
        now_milliseconds: u64,
    ) -> Result<(), ApplicationNetworkError> {
        self.publish(output, events, now_milliseconds)
            .map_err(|error| ApplicationNetworkError::Message(error.to_string()))
    }

    fn close(&mut self) {
        self.close();
    }
}

/// Shared client host adapter for the receiver.
#[derive(Clone)]
struct SharedClientHost<H> {
    /// Client host.
    host: Rc<RefCell<H>>,
}

impl<H: Q2ApplicationClientHost> Q2ClientReceiverHost for SharedClientHost<H> {
    fn protocol(&self) -> Q2ProtocolIdentity {
        self.host.borrow().protocol()
    }

    fn message_options(&self) -> Q2ServerMessageOptions {
        self.host.borrow().message_options()
    }

    fn server_data(&mut self, data: &Q2ServerData, assert_current: &dyn Fn()) {
        self.host.borrow_mut().server_data(data, assert_current);
    }

    fn game_state(&mut self, state: &Q2ApplicationGameState) {
        self.host.borrow_mut().game_state(state);
    }

    fn frame(&mut self, frame: &Q2WireFrame, records: &[Q2ServerRecord], now_milliseconds: u64) {
        self.host.borrow_mut().frame(frame, records, now_milliseconds);
    }

    fn records(&mut self, records: &[Q2ServerRecord]) {
        self.host.borrow_mut().records(records);
    }

    fn disconnected(&mut self, reason: &str) {
        self.host.borrow_mut().disconnected(reason);
    }

    fn print(&mut self, text: &str) {
        self.host.borrow_mut().print(text);
    }
}

/// Network receiver source over the client host downloads.
struct Q2NetworkReceiverSource<H> {
    /// Client host.
    host: Rc<RefCell<H>>,
}

impl<H: Q2ApplicationClientHost> Q2ClientReceiverSource for Q2NetworkReceiverSource<H> {
    fn is_demo(&self) -> bool {
        false
    }

    fn with_downloads<R>(&mut self, f: impl FnOnce(Option<&mut dyn Q2ApplicationClientDownloads>) -> R) -> R {
        f(self.host.borrow_mut().downloads())
    }
}

/// Client recording owner.
struct Q2ClientRecordingOwner {
    /// Recording sink.
    sink: Box<dyn DemoRecordingSink>,
    /// Whether the next full frame still pends.
    waiting_full_frame: bool,
    /// Detach identity.
    id: u64,
}

/// Client state shared with the recording tap.
struct Q2ClientShared<H> {
    /// Record receiver.
    receiver: Q2ClientReceiver<SharedClientHost<H>, Q2NetworkReceiverSource<H>>,
    /// Reliable channel.
    channel: Option<Q2Channel>,
    /// Handshake state.
    state: ApplicationNetworkPhase,
    /// Recording owner.
    recording_owner: Option<Q2ClientRecordingOwner>,
    /// Detach identity counter.
    detach_counter: u64,
}

/// Client recording tap.
struct Q2ClientRecordingTap<H> {
    /// Shared client state.
    shared: Rc<RefCell<Q2ClientShared<H>>>,
}

/// Client connection phase (`phase`).
fn client_phase<H: Q2ClientReceiverHost, S: Q2ClientReceiverSource>(
    channel: &Option<Q2Channel>,
    state: &ApplicationNetworkPhase,
    receiver: &Q2ClientReceiver<H, S>,
) -> ApplicationNetworkPhase {
    if channel.is_none()
        || matches!(
            state,
            ApplicationNetworkPhase::Closed | ApplicationNetworkPhase::Rejected
        )
    {
        *state
    } else {
        receiver.phase()
    }
}

impl<H: Q2ApplicationClientHost> ApplicationNetworkRecording for Q2ClientRecordingTap<H> {
    fn seed(&self) -> Result<DemoRecordingSeed, ApplicationNetworkError> {
        self.shared
            .borrow_mut()
            .receiver
            .seed()
            .map_err(|error| ApplicationNetworkError::Message(error.to_string()))
    }

    fn attach(&mut self, sink: Box<dyn DemoRecordingSink>) -> Result<Box<dyn FnOnce() + '_>, ApplicationNetworkError> {
        let mut shared = self.shared.borrow_mut();
        if shared.recording_owner.is_some() {
            return Err(ApplicationNetworkError::Message(
                "Q2 connection is already recording".to_string(),
            ));
        }
        if client_phase(&shared.channel, &shared.state, &shared.receiver) != ApplicationNetworkPhase::Active {
            return Err(ApplicationNetworkError::Message(
                "Recording requires an active Q2 connection".to_string(),
            ));
        }
        shared.detach_counter += 1;
        let id = shared.detach_counter;
        shared.recording_owner = Some(Q2ClientRecordingOwner {
            sink,
            waiting_full_frame: true,
            id,
        });
        shared.receiver.request_full_frame();
        let cell = self.shared.clone();
        Ok(Box::new(move || {
            let matched = cell
                .borrow()
                .recording_owner
                .as_ref()
                .is_some_and(|owner| owner.id == id);
            if matched {
                cell.borrow_mut().recording_owner = None;
            }
        }))
    }
}

/// Quake II client network (`Q2ClientNetwork`).
pub struct Q2ClientNetwork<T: DatagramTransport<Address = NetworkAddress> + 'static, H> {
    transport: Q2Transport<T>,
    lan: Option<Arc<KexLanTransport<T>>>,
    host: Rc<RefCell<H>>,
    shared: Rc<RefCell<Q2ClientShared<H>>>,
    recording_tap: Q2ClientRecordingTap<H>,
    handshake: Q2ClientHandshake,
    remote: NetworkAddress,
    wire: WireSelection,
    timeout_milliseconds: Option<u64>,
    last_now: u64,
    last_received: Option<u64>,
    previous: Usercmd,
    oldest: Usercmd,
    pending_commands: Vec<Usercmd>,
}

impl<T, H> Q2ClientNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q2ApplicationClientHost + 'static,
{
    /// Build a client network.
    pub fn new(options: Q2ClientNetworkOptions<T, H>) -> Result<Self, Q2NetworkError> {
        let Q2ClientNetworkOptions {
            transport,
            remote,
            host,
            qport,
            timeout_milliseconds,
        } = options;
        let protocol = host.protocol();
        let payload_bytes = transport
            .max_datagram_bytes()
            .unwrap_or(65_507)
            .saturating_sub(12)
            .min(1390);
        let (transport, lan) = if is_exact_kex(&protocol) {
            let lan = KexLanTransport::open(
                transport,
                KexLanOptions::Client {
                    server: remote.clone(),
                    local_players: 1,
                },
                None,
            )?;
            (Q2Transport::Lan(lan.clone()), Some(lan))
        } else {
            (Q2Transport::Direct(transport), None)
        };
        let host = Rc::new(RefCell::new(host));
        let receiver = Q2ClientReceiver::new(
            SharedClientHost { host: host.clone() },
            Q2NetworkReceiverSource { host: host.clone() },
        )?;
        let userinfo_host = host.clone();
        let social_host = host.clone();
        let handshake = Q2ClientHandshake::new(
            remote.clone(),
            vec![q2_receiver_protocol(&protocol)],
            qport,
            move || userinfo_host.borrow().userinfo(),
            payload_bytes,
            3000,
            move || social_host.borrow().social_id().unwrap_or_default(),
        )?;
        let shared = Rc::new(RefCell::new(Q2ClientShared {
            receiver,
            channel: None,
            state: ApplicationNetworkPhase::Challenging,
            recording_owner: None,
            detach_counter: 0,
        }));
        Ok(Self {
            transport,
            lan,
            host,
            shared: shared.clone(),
            recording_tap: Q2ClientRecordingTap { shared },
            handshake,
            remote,
            wire: WireSelection::Source {
                protocol: q2_receiver_protocol(&protocol),
            },
            timeout_milliseconds,
            last_now: 0,
            last_received: None,
            previous: Usercmd::default(),
            oldest: Usercmd::default(),
            pending_commands: Vec::new(),
        })
    }

    /// Connection phase.
    #[must_use]
    pub fn phase(&self) -> ApplicationNetworkPhase {
        let shared = self.shared.borrow();
        client_phase(&shared.channel, &shared.state, &shared.receiver)
    }

    /// Acknowledged server frame.
    #[must_use]
    pub fn acknowledged_frame(&self) -> i32 {
        self.shared.borrow().receiver.acknowledged_frame()
    }

    /// Queue a reliable command (`command`).
    pub fn command(&mut self, text: &str) -> Result<(), Q2NetworkError> {
        let mut shared = self.shared.borrow_mut();
        let Some(channel) = shared.channel.as_mut() else {
            return Err(Q2NetworkError::Message("Q2 client is not connected".to_string()));
        };
        let kex = is_exact_kex(&self.host.borrow().protocol());
        let bytes = encode_q2_client_control(&Q2ClientEvent::Command(text.to_string()), kex)?;
        channel.queue_reliable(&bytes)?;
        Ok(())
    }

    /// Queue a userinfo update (`userinfo`).
    pub fn userinfo(&mut self, text: &str) -> Result<(), Q2NetworkError> {
        let mut shared = self.shared.borrow_mut();
        let Some(channel) = shared.channel.as_mut() else {
            return Ok(());
        };
        let kex = is_exact_kex(&self.host.borrow().protocol());
        let text = if kex {
            q2_kex_client_userinfo(text)?
        } else {
            text.to_string()
        };
        let bytes = encode_q2_client_control(&Q2ClientEvent::Userinfo(text), kex)?;
        channel.queue_reliable(&bytes)?;
        Ok(())
    }

    /// Drain receiver actions in order.
    fn drain_actions(&mut self, actions: Q2ReceiverActions) -> Result<(), Q2NetworkError> {
        for text in actions.commands {
            self.command(&text)?;
        }
        if actions.reset_commands {
            self.previous = Usercmd::default();
            self.oldest = Usercmd::default();
            self.pending_commands.clear();
        }
        Ok(())
    }

    /// Poll the transport (`poll`).
    pub fn poll(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, Q2NetworkError> {
        if let Some(lan) = &self.lan {
            lan.tick(now_milliseconds as f64)?;
        }
        self.last_now = now_milliseconds;
        if matches!(
            self.phase(),
            ApplicationNetworkPhase::Closed | ApplicationNetworkPhase::Rejected
        ) {
            return Ok(Vec::new());
        }
        let launch = self.lan.as_ref().is_none_or(|lan| lan.ready());
        if launch && self.shared.borrow().channel.is_none() {
            if let Some(packet) = self.handshake.poll(now_milliseconds)? {
                let _ = self.transport.send(&self.remote, &packet);
            }
        }
        while let Some(packet) = self.transport.poll()? {
            let (from, payload) = match packet {
                ReceiveEvent::Packet { from, payload, .. } => (from, payload),
                ReceiveEvent::Error { error } => {
                    self.host.borrow_mut().print(&format!("{error}\n"));
                    continue;
                }
                ReceiveEvent::Dropped { .. } => continue,
            };
            if !same_address(&from, &self.remote, true) {
                continue;
            }
            self.last_received = Some(now_milliseconds);
            let kex = is_exact_kex(&self.host.borrow().protocol());
            if let Some(oob) = read_q2_out_of_band(&payload, kex) {
                if oob.command == "print" {
                    self.host.borrow_mut().print(&oob.body);
                }
                let _ = self.handshake.receive(&from, &oob)?;
                let connected = match self.handshake.state() {
                    Q2ClientHandshakeState::Connecting { .. } => {
                        self.shared.borrow_mut().state = ApplicationNetworkPhase::Connecting;
                        None
                    }
                    Q2ClientHandshakeState::Rejected { reason } => {
                        self.shared.borrow_mut().state = ApplicationNetworkPhase::Rejected;
                        let reason = reason.clone();
                        self.host.borrow_mut().disconnected(&reason);
                        None
                    }
                    Q2ClientHandshakeState::Connected {
                        request,
                        download_server,
                    } => {
                        if self.shared.borrow().channel.is_some() {
                            None
                        } else {
                            Some((request.clone(), download_server.clone()))
                        }
                    }
                    _ => None,
                };
                if let Some((request, download_server)) = connected {
                    if let Some(downloads) = self.host.borrow_mut().downloads() {
                        downloads.set_http_server(download_server);
                    }
                    let channel = Q2Channel::new(Q2ChannelOptions {
                        side: ChannelSide::Client,
                        protocol: request.protocol,
                        channel: request.channel,
                        qport: request.qport,
                        payload_bytes: Some(request.payload_bytes),
                        message_bytes: None,
                        max_datagram_bytes: self.transport.max_datagram_bytes().or(Some(65_507)),
                        compress: false,
                        sequence_recording: None,
                    })?;
                    self.shared.borrow_mut().channel = Some(channel);
                    self.shared.borrow_mut().state = ApplicationNetworkPhase::Loading;
                    self.command("new")?;
                }
            } else if self.shared.borrow().channel.is_some() {
                let result = {
                    let mut shared = self.shared.borrow_mut();
                    let channel = shared.channel.as_mut().expect("channel checked");
                    channel.receive(&payload, now_milliseconds)?
                };
                if let Q2ChannelReceive::Message {
                    acknowledged, bytes, ..
                } = result
                {
                    if let Some(prediction) = self.host.borrow_mut().prediction() {
                        prediction.acknowledged(acknowledged, now_milliseconds);
                    }
                    let outcome =
                        self.shared
                            .borrow_mut()
                            .receiver
                            .receive(&bytes, now_milliseconds, self.transport.closed())?;
                    if self.shared.borrow().recording_owner.is_some() {
                        let mut messages = Vec::new();
                        {
                            let mut shared = self.shared.borrow_mut();
                            let owner = shared.recording_owner.as_mut().expect("recording checked");
                            for record in &outcome.records {
                                match &record.event {
                                    Q2ServerEvent::ServerData { .. } => {
                                        owner.waiting_full_frame = true;
                                    }
                                    Q2ServerEvent::Frame { frame } => {
                                        if !frame.valid {
                                            continue;
                                        }
                                        if owner.waiting_full_frame && frame.delta_frame > 0 {
                                            continue;
                                        }
                                        owner.waiting_full_frame = false;
                                    }
                                    _ => {}
                                }
                                messages.push(record.raw.clone());
                            }
                            if owner.waiting_full_frame {
                                shared.receiver.request_full_frame();
                            }
                        }
                        if !messages.is_empty() {
                            let mut joined = Vec::new();
                            for message in &messages {
                                joined.extend_from_slice(message);
                            }
                            let packet = DemoRecordingPacket::Q2 { message: joined };
                            self.shared
                                .borrow_mut()
                                .recording_owner
                                .as_mut()
                                .expect("recording checked")
                                .sink
                                .append(&packet)?;
                        }
                    }
                    self.drain_actions(outcome.actions)?;
                }
            }
        }
        if let Some(last) = self.last_received {
            if now_milliseconds.saturating_sub(last) > self.timeout_milliseconds.unwrap_or(120_000) {
                self.shared.borrow_mut().receiver.close();
                self.shared.borrow_mut().state = ApplicationNetworkPhase::Rejected;
                self.host.borrow_mut().disconnected("Connection timed out");
            }
        }
        let actions = self
            .shared
            .borrow_mut()
            .receiver
            .prepare_game_state(self.transport.closed())?;
        self.drain_actions(actions)?;
        self.send_pending(now_milliseconds)?;
        Ok(Vec::new())
    }

    /// Send queued moves and keepalives (`sendPending`).
    fn send_pending(&mut self, now_milliseconds: u64) -> Result<(), Q2NetworkError> {
        if self.shared.borrow().channel.is_none()
            || matches!(
                self.phase(),
                ApplicationNetworkPhase::Closed | ApplicationNetworkPhase::Rejected
            )
        {
            return Ok(());
        }
        for command in std::mem::take(&mut self.pending_commands) {
            let (sequence, acknowledged) = {
                let shared = self.shared.borrow();
                let channel = shared.channel.as_ref().expect("channel checked");
                (channel.outgoing_sequence(), shared.receiver.acknowledged_frame())
            };
            let bytes = {
                let mut shared = self.shared.borrow_mut();
                let wire = &mut shared.receiver.reader.wire;
                encode_q2_move(
                    wire,
                    sequence,
                    acknowledged,
                    &[self.oldest.clone(), self.previous.clone(), command.clone()],
                )?
            };
            {
                let mut shared = self.shared.borrow_mut();
                let channel = shared.channel.as_mut().expect("channel checked");
                channel.send(&self.transport, &self.remote, &bytes, now_milliseconds)?;
            }
            if let Some(prediction) = self.host.borrow_mut().prediction() {
                prediction.sent(sequence, &command, now_milliseconds);
            }
            self.oldest = self.previous.clone();
            self.previous = command;
        }
        let update = self
            .shared
            .borrow()
            .channel
            .as_ref()
            .is_some_and(|channel| channel.should_update(now_milliseconds));
        if update {
            let mut shared = self.shared.borrow_mut();
            let channel = shared.channel.as_mut().expect("channel checked");
            channel.send(&self.transport, &self.remote, &[], now_milliseconds)?;
        }
        Ok(())
    }

    /// Queue local input (`submit`).
    pub fn submit(&mut self, commands: &[ActorCommand], _now_milliseconds: u64) -> Result<(), Q2NetworkError> {
        if self.phase() != ApplicationNetworkPhase::Active {
            return Ok(());
        }
        if commands.len() > 1 {
            return Err(Q2NetworkError::Message(
                "A native Q2 connection carries one player; use independent connections for local seats".to_string(),
            ));
        }
        for command in commands {
            let usercmd = self.host.borrow().command(command);
            self.pending_commands.push(usercmd);
        }
        Ok(())
    }

    /// Reject authoritative publication (`publish`).
    pub fn publish(
        &mut self,
        _output: &SimulationOutput,
        _events: &[NetworkPresentationEvent],
        _now_milliseconds: u64,
    ) -> Result<(), Q2NetworkError> {
        Err(Q2NetworkError::Message(
            "Remote Q2 client cannot publish authoritative server state".to_string(),
        ))
    }

    /// Close the client (`close`).
    pub fn close(&mut self) {
        self.shared.borrow_mut().recording_owner = None;
        let phase = self.phase();
        self.shared.borrow_mut().receiver.close();
        if self.transport.closed() {
            return;
        }
        if self.shared.borrow().channel.is_some() && phase != ApplicationNetworkPhase::Closed {
            let _ = self.command("disconnect");
            let mut shared = self.shared.borrow_mut();
            if let Some(channel) = shared.channel.as_mut() {
                let _ = channel.send(&self.transport, &self.remote, &[], self.last_now);
            }
        }
        self.shared.borrow_mut().state = ApplicationNetworkPhase::Closed;
        self.pending_commands.clear();
        self.transport.close();
    }
}

impl<T, H> ApplicationNetwork for Q2ClientNetwork<T, H>
where
    T: DatagramTransport<Address = NetworkAddress> + 'static,
    H: Q2ApplicationClientHost + 'static,
{
    fn recording(&mut self) -> Option<&mut dyn ApplicationNetworkRecording> {
        Some(&mut self.recording_tap)
    }

    fn role(&self) -> ApplicationNetworkRole {
        ApplicationNetworkRole::Client
    }

    fn phase(&self) -> ApplicationNetworkPhase {
        self.phase()
    }

    fn wire(&self) -> WireSelection {
        self.wire.clone()
    }

    fn poll(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, ApplicationNetworkError> {
        self.poll(now_milliseconds)
            .map_err(|error| ApplicationNetworkError::Message(error.to_string()))
    }

    fn submit(&mut self, commands: &[ActorCommand], now_milliseconds: u64) -> Result<(), ApplicationNetworkError> {
        self.submit(commands, now_milliseconds)
            .map_err(|error| ApplicationNetworkError::Message(error.to_string()))
    }

    fn publish(
        &mut self,
        output: &SimulationOutput,
        events: &[NetworkPresentationEvent],
        now_milliseconds: u64,
    ) -> Result<(), ApplicationNetworkError> {
        self.publish(output, events, now_milliseconds)
            .map_err(|error| ApplicationNetworkError::Message(error.to_string()))
    }

    fn close(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::time::{FrameContext, FramePhase};
    use qa_net::common::commands::UserCommand;
    use qa_net::common::transport::TransportError;
    use qa_net::q2::{PlayerState, ServerData};
    use qa_net::q2_net::{write_q2_connect, ChannelKind, Q2StatusPlayer};
    use qa_net::q2_variants::Q2ProServerData;
    use qa_net::services::downloads::{DownloadError, DownloadSource};
    use qa_world::session::WorldSnapshot;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use crate::bootstrap::demo_recording::{DemoRecordingError, Q2ProRevision, R1Q2Revision};
    use crate::bootstrap::network::q2_downloads::{
        Q2ApplicationDownloads, Q2DownloadBlock, Q2DownloadError, Q2DownloadOutcome, Q2DownloadPreparation,
    };
    use crate::bootstrap::network::types::{
        Q2ApplicationDiscovery, Q2ApplicationServerEvent, Q2ClientPrediction, Q2MvdSettings,
    };

    /// Shared outbound queue.
    type OutboundQueue = Arc<Mutex<Vec<(NetworkAddress, Vec<u8>)>>>;

    /// In-memory datagram transport with shared queues.
    struct MockTransport {
        address: NetworkAddress,
        inbound: Arc<Mutex<VecDeque<ReceiveEvent<NetworkAddress>>>>,
        outbound: OutboundQueue,
        closed: Arc<Mutex<bool>>,
    }

    /// Test-side transport handles.
    struct MockTransportHandles {
        inbound: Arc<Mutex<VecDeque<ReceiveEvent<NetworkAddress>>>>,
        outbound: OutboundQueue,
        closed: Arc<Mutex<bool>>,
    }

    impl MockTransport {
        fn new(address: NetworkAddress) -> (Self, MockTransportHandles) {
            let inbound = Arc::new(Mutex::new(VecDeque::new()));
            let outbound = Arc::new(Mutex::new(Vec::new()));
            let closed = Arc::new(Mutex::new(false));
            let handles = MockTransportHandles {
                inbound: inbound.clone(),
                outbound: outbound.clone(),
                closed: closed.clone(),
            };
            (
                Self {
                    address,
                    inbound,
                    outbound,
                    closed,
                },
                handles,
            )
        }
    }

    impl MockTransportHandles {
        fn queue(&self, from: NetworkAddress, payload: Vec<u8>) {
            self.queue_event(ReceiveEvent::Packet {
                from,
                payload,
                received_at: 0.0,
            });
        }

        fn queue_event(&self, event: ReceiveEvent<NetworkAddress>) {
            self.inbound.lock().expect("inbound").push_back(event);
        }

        fn take_outbound(&self) -> Vec<(NetworkAddress, Vec<u8>)> {
            std::mem::take(&mut self.outbound.lock().expect("outbound"))
        }

        fn is_closed(&self) -> bool {
            *self.closed.lock().expect("closed")
        }
    }

    impl DatagramTransport for MockTransport {
        type Address = NetworkAddress;

        fn address(&self) -> NetworkAddress {
            self.address.clone()
        }

        fn closed(&self) -> bool {
            *self.closed.lock().expect("closed")
        }

        fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
            self.outbound
                .lock()
                .expect("outbound")
                .push((to.clone(), payload.to_vec()));
            Ok(true)
        }

        fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
            Ok(self.inbound.lock().expect("inbound").pop_front())
        }

        fn subscribe_readable(&self, _listener: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
            Ok(0)
        }

        fn unsubscribe(&self, _token: u64) {}

        fn close(&self) {
            *self.closed.lock().expect("closed") = true;
        }
    }

    /// In-memory download source.
    struct MockSource {
        data: Vec<u8>,
        fail: bool,
    }

    impl DownloadSource for MockSource {
        fn byte_length(&self) -> u64 {
            self.data.len() as u64
        }

        fn read(&mut self, offset: u64, max_bytes: usize) -> Result<Vec<u8>, DownloadError> {
            if self.fail {
                return Err(DownloadError::SourceClosed);
            }
            let start = offset as usize;
            if start >= self.data.len() {
                return Ok(Vec::new());
            }
            let end = (start + max_bytes).min(self.data.len());
            Ok(self.data[start..end].to_vec())
        }

        fn close(&mut self) {}
    }

    /// In-memory server downloads.
    struct MockDownloads {
        files: HashMap<String, Vec<u8>>,
        failing: Vec<String>,
        http_server: Option<String>,
    }

    impl Q2ApplicationDownloads for MockDownloads {
        fn http_server(&self) -> Option<String> {
            self.http_server.clone()
        }

        fn allowed(&self, _name: &str) -> bool {
            true
        }

        fn open(&mut self, name: &str) -> Option<Box<dyn DownloadSource>> {
            let fail = self.failing.iter().any(|failing| failing == name);
            self.files.get(name).map(|data| {
                Box::new(MockSource {
                    data: data.clone(),
                    fail,
                }) as Box<dyn DownloadSource>
            })
        }
    }

    /// Fixed discovery surface.
    #[derive(Clone)]
    struct MockDiscovery {
        status: Q2Status,
        info: Q2Info,
    }

    impl Q2ApplicationDiscovery for MockDiscovery {
        fn status(&self) -> Q2Status {
            self.status.clone()
        }

        fn info(&self) -> Q2Info {
            self.info.clone()
        }
    }

    /// Recording administration surface.
    struct MockRcon {
        password: String,
        log: Rc<Mutex<ServerLog>>,
    }

    impl Q2ApplicationRcon for MockRcon {
        fn profile(&self) -> Q2ServerProfile {
            Q2ServerProfile::Classic
        }

        fn rcon_password(&self) -> String {
            self.password.clone()
        }

        fn limited_rcon(&self) -> Option<Q2LimitedRcon> {
            None
        }

        fn rcon_rate_allowed(&self, _now: u64) -> bool {
            true
        }

        fn recharge_rcon_rate(&mut self) {}

        fn execute_rcon(&mut self, command: &str, limited: bool, output: &mut dyn FnMut(&str)) {
            self.log.lock().expect("log").rcon.push((command.to_string(), limited));
            output("done");
        }
    }

    /// Observed server host calls.
    #[derive(Default)]
    struct ServerLog {
        admissions: Vec<String>,
        inputs: Vec<(u32, u32)>,
        commands: Vec<(u32, String, Vec<String>)>,
        texts: Vec<(u32, String)>,
        userinfos: Vec<(u32, String)>,
        disconnects: Vec<(u32, String)>,
        prints: Vec<String>,
        begins: Vec<u32>,
        frames: Vec<u32>,
        events: Vec<u32>,
        observes: u32,
        rcon: Vec<(String, bool)>,
    }

    /// Configurable server host.
    struct MockServerHost {
        log: Rc<Mutex<ServerLog>>,
        owner: IdentityOwner,
        protocol: Q2ProtocolIdentity,
        max_clients: u32,
        players: Vec<Q2ApplicationPlayer>,
        next_slot: u32,
        game_state: Q2ApplicationGameState,
        downloads: MockDownloads,
        discovery: Option<MockDiscovery>,
        admin: Option<MockRcon>,
        masters: Option<Vec<NetworkAddress>>,
        capture_revision: Option<u16>,
        mvd_settings: Option<Q2MvdSettings>,
        unsupported: bool,
        reject_admit: Option<String>,
        reject_addr: Option<NetworkAddress>,
    }

    impl MockServerHost {
        fn new() -> (Self, Rc<Mutex<ServerLog>>) {
            let log = Rc::new(Mutex::new(ServerLog::default()));
            let owner = IdentityOwner::create("q2-server-test").expect("owner");
            let mut config_strings = HashMap::new();
            config_strings.insert(0u32, "greeting".to_string());
            config_strings.insert(1u32, "maxclients\\8".to_string());
            let mut baselines = HashMap::new();
            baselines.insert(
                1u32,
                EntityState {
                    number: 1,
                    ..EntityState::default()
                },
            );
            let host = Self {
                log: log.clone(),
                owner,
                protocol: Q2ProtocolIdentity::Classic,
                max_clients: 8,
                players: Vec::new(),
                next_slot: 0,
                game_state: Q2ApplicationGameState {
                    data: Q2ServerData::Vanilla(ServerData {
                        servercount: 0,
                        attractloop: false,
                        gamedir: "baseq2".to_string(),
                        clientnum: 0,
                        levelname: "test".to_string(),
                    }),
                    config_strings,
                    baselines,
                },
                downloads: MockDownloads {
                    files: HashMap::new(),
                    failing: Vec::new(),
                    http_server: None,
                },
                discovery: None,
                admin: None,
                masters: None,
                capture_revision: None,
                mvd_settings: None,
                unsupported: false,
                reject_admit: None,
                reject_addr: None,
            };
            (host, log)
        }

        fn with_discovery(mut self) -> Self {
            self.discovery = Some(MockDiscovery {
                status: Q2Status {
                    server_info: "test".to_string(),
                    players: vec![Q2StatusPlayer {
                        score: 1,
                        ping: 10,
                        name: "tester".to_string(),
                    }],
                },
                info: Q2Info {
                    name: "test".to_string(),
                    map: "base1".to_string(),
                    players: 1,
                    max_players: 8,
                },
            });
            self
        }

        fn with_admin(mut self, password: &str) -> Self {
            self.admin = Some(MockRcon {
                password: password.to_string(),
                log: self.log.clone(),
            });
            self
        }

        fn with_file(mut self, name: &str, bytes: Vec<u8>) -> Self {
            self.downloads.files.insert(name.to_string(), bytes);
            self
        }

        fn with_failing(mut self, name: &str, bytes: Vec<u8>) -> Self {
            self.downloads.files.insert(name.to_string(), bytes);
            self.downloads.failing.push(name.to_string());
            self
        }

        fn with_capture(mut self, revision: u16) -> Self {
            self.capture_revision = Some(revision);
            self
        }
    }

    impl Q2ApplicationServerHost for MockServerHost {
        fn rejects(&self, address: &NetworkAddress) -> bool {
            self.reject_addr.as_ref().is_some_and(|rejected| rejected == address)
        }

        fn administration(&mut self) -> Option<&mut dyn Q2ApplicationRcon> {
            self.admin.as_mut().map(|admin| admin as &mut dyn Q2ApplicationRcon)
        }

        fn masters(&self) -> Option<Vec<NetworkAddress>> {
            self.masters.clone()
        }

        fn discovery(&self) -> Option<&dyn Q2ApplicationDiscovery> {
            self.discovery
                .as_ref()
                .map(|discovery| discovery as &dyn Q2ApplicationDiscovery)
        }

        fn downloads(&mut self) -> &mut dyn Q2ApplicationDownloads {
            &mut self.downloads
        }

        fn protocol(&self) -> Q2ProtocolIdentity {
            self.protocol
        }

        fn message_options(&self) -> Q2ServerMessageOptions {
            Q2ServerMessageOptions::default()
        }

        fn max_clients(&self) -> u32 {
            self.max_clients
        }

        fn supports_source_wire(&self) -> WireAdmission {
            if self.unsupported {
                WireAdmission::Unsupported {
                    reasons: vec!["no test wire".to_string()],
                }
            } else {
                WireAdmission::Supported
            }
        }

        fn observe(&mut self, _output: &SimulationOutput, _events: &[NetworkPresentationEvent]) {
            self.log.lock().expect("log").observes += 1;
        }

        fn mvd_capture(
            &mut self,
            _output: &SimulationOutput,
            _events: &[NetworkPresentationEvent],
            servercount: i32,
        ) -> Option<MvdCapture> {
            self.capture_revision.map(|revision| {
                let mut config_strings = BTreeMap::new();
                config_strings.insert(30u16, "8".to_string());
                MvdCapture {
                    revision,
                    flags: 0,
                    servercount,
                    gamedir: "baseq2".to_string(),
                    dummy: 0,
                    config_strings,
                    portal_bits: Vec::new(),
                    players: BTreeMap::new(),
                    entities: Vec::new(),
                    messages: Vec::new(),
                }
            })
        }

        fn mvd_settings(&self) -> Option<Q2MvdSettings> {
            self.mvd_settings.clone()
        }

        fn admit(&mut self, _from: &NetworkAddress, request: &Q2ConnectRequest) -> Q2ApplicationAdmission {
            self.log.lock().expect("log").admissions.push(request.userinfo.clone());
            if let Some(reason) = &self.reject_admit {
                return Q2ApplicationAdmission::Rejected { reason: reason.clone() };
            }
            let slot = self.next_slot;
            self.next_slot += 1;
            let player = Q2ApplicationPlayer {
                client: self.owner.client(slot, 0),
                actor: self.owner.actor(slot, 0),
                source_entity: slot + 1,
            };
            self.players.push(player.clone());
            Q2ApplicationAdmission::Accepted { player }
        }

        fn disconnect(&mut self, player: &Q2ApplicationPlayer, reason: &str) {
            self.players.retain(|known| known.client != player.client);
            self.log
                .lock()
                .expect("log")
                .disconnects
                .push((player.client.slot(), reason.to_string()));
        }

        fn carried_player(&self, client: &ClientId) -> Q2ApplicationPlayer {
            if let Some(player) = self.players.iter().find(|player| player.client == *client) {
                return player.clone();
            }
            Q2ApplicationPlayer {
                client: client.clone(),
                actor: self.owner.actor(client.slot(), 0),
                source_entity: client.slot() + 1,
            }
        }

        fn begin(&mut self, player: &Q2ApplicationPlayer) {
            self.log.lock().expect("log").begins.push(player.client.slot());
        }

        fn game_state(
            &self,
            _player: &Q2ApplicationPlayer,
            protocol: Option<Q2ProtocolIdentity>,
        ) -> Q2ApplicationGameState {
            let mut state = self.game_state.clone();
            if matches!(protocol, Some(Q2ProtocolIdentity::Q2Pro { .. })) {
                state.data = Q2ServerData::Q2Pro(Q2ProServerData {
                    servercount: 0,
                    attractloop: false,
                    gamedir: "baseq2".to_string(),
                    clientnum: 0,
                    levelname: "test".to_string(),
                    version: 1021,
                    server_state: 0,
                    wire_flags: 0,
                });
            }
            state
        }

        fn frame(
            &self,
            player: &Q2ApplicationPlayer,
            output: &SimulationOutput,
            _protocol: Option<Q2ProtocolIdentity>,
        ) -> Q2WireFrame {
            self.log.lock().expect("log").frames.push(player.client.slot());
            Q2WireFrame {
                valid: true,
                server_frame: output.snapshot.frame.frame,
                delta_frame: 0,
                suppressed_count: 0,
                area_bits: Vec::new(),
                player: PlayerState::default(),
                split_players: Vec::new(),
                entities: Vec::new(),
            }
        }

        fn events(
            &mut self,
            player: &Q2ApplicationPlayer,
            _output: &SimulationOutput,
            _events: &[NetworkPresentationEvent],
        ) -> Vec<Q2ApplicationServerEvent> {
            self.log.lock().expect("log").events.push(player.client.slot());
            Vec::new()
        }

        fn input(&mut self, player: &Q2ApplicationPlayer, _command: &Usercmd, sequence: u32) -> Option<ActorCommand> {
            self.log
                .lock()
                .expect("log")
                .inputs
                .push((player.client.slot(), sequence));
            Some(ActorCommand {
                actor: player.actor.clone(),
                source: CommandSource::Remote {
                    client: player.client.clone(),
                },
                sequence: u64::from(sequence),
                command: UserCommand::Q2Classic {
                    milliseconds: 0.0,
                    angle_shorts: [0.0; 3],
                    forward_move: 0.0,
                    side_move: 0.0,
                    up_move: 0.0,
                    buttons: 0.0,
                    impulse: 0.0,
                    light_level: 0.0,
                },
                arsenal: None,
            })
        }

        fn command(&mut self, player: &Q2ApplicationPlayer, name: &str, args: &[String]) {
            self.log
                .lock()
                .expect("log")
                .commands
                .push((player.client.slot(), name.to_string(), args.to_vec()));
        }

        fn command_text(&mut self, player: &Q2ApplicationPlayer, text: &str) {
            self.log
                .lock()
                .expect("log")
                .texts
                .push((player.client.slot(), text.to_string()));
        }

        fn userinfo(&mut self, player: &Q2ApplicationPlayer, value: &str) {
            self.log
                .lock()
                .expect("log")
                .userinfos
                .push((player.client.slot(), value.to_string()));
        }

        fn print(&mut self, text: &str) {
            self.log.lock().expect("log").prints.push(text.to_string());
        }
    }

    /// Observed client host calls.
    #[derive(Default)]
    struct ClientLog {
        game_states: Vec<i32>,
        frames: Vec<(i32, u64)>,
        records: u32,
        disconnects: Vec<String>,
        prints: Vec<String>,
        sent: Vec<(u32, u64)>,
        acked: Vec<(u32, u64)>,
    }

    /// Recording prediction hooks.
    struct MockPrediction {
        log: Rc<Mutex<ClientLog>>,
    }

    impl Q2ClientPrediction for MockPrediction {
        fn sent(&mut self, sequence: u32, _command: &Usercmd, now_milliseconds: u64) {
            self.log.lock().expect("log").sent.push((sequence, now_milliseconds));
        }

        fn acknowledged(&mut self, sequence: u32, now_milliseconds: u64) {
            self.log.lock().expect("log").acked.push((sequence, now_milliseconds));
        }
    }

    /// Accepting client downloads.
    struct MockClientDownloads {
        http_server: Rc<Mutex<Option<String>>>,
    }

    impl Q2ApplicationClientDownloads for MockClientDownloads {
        fn set_http_server(&mut self, server: Option<String>) {
            *self.http_server.lock().expect("http") = server;
        }

        fn prepare(&mut self, _state: &Q2ApplicationGameState) -> Result<Q2DownloadPreparation, Q2DownloadError> {
            Ok(Q2DownloadPreparation::Ready)
        }

        fn receive(&mut self, _block: &Q2DownloadBlock) -> Result<Q2DownloadOutcome, Q2DownloadError> {
            Ok(Q2DownloadOutcome::Waiting)
        }

        fn close(&mut self) {}
    }

    /// Configurable client host.
    struct MockClientHost {
        log: Rc<Mutex<ClientLog>>,
        protocol: Q2ProtocolIdentity,
        userinfo: String,
        usercmd: Usercmd,
        prediction: Option<MockPrediction>,
        downloads: Option<MockClientDownloads>,
    }

    impl MockClientHost {
        fn new() -> (Self, Rc<Mutex<ClientLog>>) {
            let log = Rc::new(Mutex::new(ClientLog::default()));
            let host = Self {
                log: log.clone(),
                protocol: Q2ProtocolIdentity::Classic,
                userinfo: "\\name\\tester".to_string(),
                usercmd: Usercmd::default(),
                prediction: Some(MockPrediction { log: log.clone() }),
                downloads: None,
            };
            (host, log)
        }

        fn with_downloads(mut self) -> (Self, Rc<Mutex<Option<String>>>) {
            let http_server = Rc::new(Mutex::new(None));
            self.downloads = Some(MockClientDownloads {
                http_server: http_server.clone(),
            });
            (self, http_server)
        }
    }

    impl Q2ApplicationClientHost for MockClientHost {
        fn protocol(&self) -> Q2ProtocolIdentity {
            self.protocol
        }

        fn message_options(&self) -> Q2ServerMessageOptions {
            Q2ServerMessageOptions::default()
        }

        fn userinfo(&self) -> String {
            self.userinfo.clone()
        }

        fn prediction(&mut self) -> Option<&mut dyn Q2ClientPrediction> {
            self.prediction
                .as_mut()
                .map(|prediction| prediction as &mut dyn Q2ClientPrediction)
        }

        fn downloads(&mut self) -> Option<&mut dyn Q2ApplicationClientDownloads> {
            self.downloads
                .as_mut()
                .map(|downloads| downloads as &mut dyn Q2ApplicationClientDownloads)
        }

        fn game_state(&mut self, state: &Q2ApplicationGameState) {
            self.log.lock().expect("log").game_states.push(state.data.servercount());
        }

        fn frame(&mut self, frame: &Q2WireFrame, records: &[Q2ServerRecord], now_milliseconds: u64) {
            let mut log = self.log.lock().expect("log");
            log.frames.push((frame.server_frame, now_milliseconds));
            log.records += records.len() as u32;
        }

        fn records(&mut self, records: &[Q2ServerRecord]) {
            self.log.lock().expect("log").records += records.len() as u32;
        }

        fn command(&self, _command: &ActorCommand) -> Usercmd {
            self.usercmd.clone()
        }

        fn disconnected(&mut self, reason: &str) {
            self.log.lock().expect("log").disconnects.push(reason.to_string());
        }

        fn print(&mut self, text: &str) {
            self.log.lock().expect("log").prints.push(text.to_string());
        }
    }

    /// Collecting recording sink.
    struct MockSink {
        packets: Rc<Mutex<Vec<DemoRecordingPacket>>>,
    }

    impl MockSink {
        fn new() -> (Self, Rc<Mutex<Vec<DemoRecordingPacket>>>) {
            let packets = Rc::new(Mutex::new(Vec::new()));
            (
                Self {
                    packets: packets.clone(),
                },
                packets,
            )
        }
    }

    impl DemoRecordingSink for MockSink {
        fn append(&mut self, packet: &DemoRecordingPacket) -> Result<(), DemoRecordingError> {
            self.packets.lock().expect("packets").push(packet.clone());
            Ok(())
        }
    }

    /// Failing recording sink.
    struct FailingSink;

    impl DemoRecordingSink for FailingSink {
        fn append(&mut self, _packet: &DemoRecordingPacket) -> Result<(), DemoRecordingError> {
            Err(DemoRecordingError::Stopped)
        }
    }

    fn server_addr() -> NetworkAddress {
        NetworkAddress::Ipv4 {
            host: [127, 0, 0, 1],
            port: 27910,
        }
    }

    fn client_addr() -> NetworkAddress {
        NetworkAddress::Ipv4 {
            host: [127, 0, 0, 1],
            port: 27911,
        }
    }

    fn foreign_addr() -> NetworkAddress {
        NetworkAddress::Ipv4 {
            host: [127, 0, 0, 1],
            port: 27912,
        }
    }

    fn test_output(frame: i32, time_ms: i32) -> SimulationOutput {
        SimulationOutput {
            snapshot: WorldSnapshot {
                frame: FrameContext {
                    frame,
                    time: SourceTime::Milliseconds(time_ms),
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

    fn parse_oob(payload: &[u8]) -> Q2ConnectionlessMessage {
        read_q2_out_of_band(payload, false).expect("connectionless reply")
    }

    fn test_server(host: MockServerHost) -> (Q2ServerNetwork<MockTransport, MockServerHost>, MockTransportHandles) {
        let (transport, handles) = MockTransport::new(server_addr());
        let server = Q2ServerNetwork::new(Q2ServerNetworkOptions {
            transport,
            host,
            random: Box::new(|| 0.5),
            timeout_milliseconds: None,
        })
        .expect("server");
        (server, handles)
    }

    fn test_client(host: MockClientHost) -> (Q2ClientNetwork<MockTransport, MockClientHost>, MockTransportHandles) {
        let (transport, handles) = MockTransport::new(client_addr());
        let client = Q2ClientNetwork::new(Q2ClientNetworkOptions {
            transport,
            remote: server_addr(),
            host,
            qport: 1234,
            timeout_milliseconds: None,
        })
        .expect("client");
        (client, handles)
    }

    fn challenge_through(
        server: &mut Q2ServerNetwork<MockTransport, MockServerHost>,
        handles: &MockTransportHandles,
    ) -> i64 {
        handles.queue(client_addr(), q2_out_of_band("getchallenge", false));
        server.poll(1000).expect("poll");
        let outbound = handles.take_outbound();
        assert_eq!(outbound.len(), 1);
        let message = parse_oob(&outbound[0].1);
        assert_eq!(message.command, "challenge");
        message.arguments[0].parse::<i64>().expect("challenge")
    }

    fn connect_through(
        server: &mut Q2ServerNetwork<MockTransport, MockServerHost>,
        handles: &MockTransportHandles,
        challenge: i64,
    ) {
        let request = Q2ConnectRequest {
            protocol: ProtocolIdentity::Q2Classic,
            qport: 1234,
            challenge,
            userinfo: "\\name\\tester".to_string(),
            payload_bytes: 1390,
            channel: ChannelKind::New,
            compression: false,
            social_ids: None,
        };
        handles.queue(client_addr(), write_q2_connect(&request).expect("connect"));
        server.poll(2000).expect("poll");
        let outbound = handles.take_outbound();
        assert!(!outbound.is_empty());
        assert_eq!(parse_oob(&outbound[0].1).command, "client_connect");
    }

    fn test_client_channel() -> Q2Channel {
        Q2Channel::new(Q2ChannelOptions {
            side: ChannelSide::Client,
            protocol: ProtocolIdentity::Q2Classic,
            channel: ChannelKind::Old,
            qport: 1234,
            payload_bytes: Some(1390),
            message_bytes: None,
            max_datagram_bytes: Some(65_507),
            compress: false,
            sequence_recording: None,
        })
        .expect("channel")
    }

    fn send_control(channel: &mut Q2Channel, handles: &MockTransportHandles, now: u64, text: &str) {
        let bytes = encode_q2_client_control(&Q2ClientEvent::Command(text.to_string()), false).expect("control");
        channel.queue_reliable(&bytes).expect("queue");
        let packet = channel.transmit(&[], now).expect("transmit");
        handles.queue(client_addr(), packet);
    }

    fn signon_step(
        channel: &mut Q2Channel,
        server: &mut Q2ServerNetwork<MockTransport, MockServerHost>,
        handles: &MockTransportHandles,
        now: u64,
        text: &str,
    ) {
        send_control(channel, handles, now, text);
        server.poll(now).expect("poll");
        feed_acks(channel, handles, now);
    }

    /// Feed server channel packets back so reliable advances past the first.
    fn feed_acks(channel: &mut Q2Channel, handles: &MockTransportHandles, now: u64) {
        for (_, bytes) in handles.take_outbound() {
            if read_q2_out_of_band(&bytes, false).is_none() {
                let _ = channel.receive(&bytes, now);
            }
        }
    }

    #[test]
    fn server_rejects_unavailable_wire_and_kex_demo() {
        let (mut host, _log) = MockServerHost::new();
        host.unsupported = true;
        let (transport, _handles) = MockTransport::new(server_addr());
        let error = Q2ServerNetwork::new(Q2ServerNetworkOptions {
            transport,
            host,
            random: Box::new(|| 0.5),
            timeout_milliseconds: None,
        })
        .err()
        .expect("unsupported wire");
        assert_eq!(error.to_string(), "Native Q2 wire is unavailable: no test wire");

        let (mut host, _log) = MockServerHost::new();
        host.protocol = Q2ProtocolIdentity::KexDemo;
        let (transport, _handles) = MockTransport::new(server_addr());
        let error = Q2ServerNetwork::new(Q2ServerNetworkOptions {
            transport,
            host,
            random: Box::new(|| 0.5),
            timeout_milliseconds: None,
        })
        .err()
        .expect("kex demo");
        assert_eq!(error.to_string(), "KEX native live transport is unbound");
    }

    #[test]
    fn server_challenge_connect_and_reconnect() {
        let (host, log) = MockServerHost::new();
        let (mut server, handles) = test_server(host);
        assert_eq!(server.phase(), ApplicationNetworkPhase::Active);
        let challenge = challenge_through(&mut server, &handles);
        connect_through(&mut server, &handles, challenge);
        assert_eq!(server.clients().len(), 1);
        assert_eq!(log.lock().expect("log").admissions.len(), 1);

        // A second connect from the same address re-accepts without admitting.
        connect_through(&mut server, &handles, challenge);
        assert_eq!(server.clients().len(), 1);
        assert_eq!(log.lock().expect("log").admissions.len(), 1);
    }

    #[test]
    fn server_connect_guards() {
        let (host, _log) = MockServerHost::new();
        let (mut server, handles) = test_server(host);
        let challenge = challenge_through(&mut server, &handles);

        // Bad challenge.
        let request = Q2ConnectRequest {
            protocol: ProtocolIdentity::Q2Classic,
            qport: 4321,
            challenge: challenge + 1,
            userinfo: "\\name\\tester".to_string(),
            payload_bytes: 1390,
            channel: ChannelKind::New,
            compression: false,
            social_ids: None,
        };
        handles.queue(client_addr(), write_q2_connect(&request).expect("connect"));
        server.poll(2000).expect("poll");
        let outbound = handles.take_outbound();
        assert_eq!(parse_oob(&outbound[0].1).command, "print");
        assert!(parse_oob(&outbound[0].1).body.contains("Bad challenge"));

        // Wrong protocol version.
        handles.queue(client_addr(), q2_out_of_band("connect 36 4321 0 \"x\"\n", false));
        server.poll(2000).expect("poll");
        let outbound = handles.take_outbound();
        assert!(parse_oob(&outbound[0].1).body.contains("Unsupported protocol"));
        assert!(server.clients().is_empty());
    }

    #[test]
    fn server_full_and_rejected_admission() {
        let (mut host, _log) = MockServerHost::new();
        host.max_clients = 1;
        let (mut server, handles) = test_server(host);
        let challenge = challenge_through(&mut server, &handles);
        connect_through(&mut server, &handles, challenge);

        // Second client from another address: server is full.
        handles.queue(foreign_addr(), q2_out_of_band("getchallenge", false));
        server.poll(3000).expect("poll");
        let outbound = handles.take_outbound();
        let foreign_challenge = parse_oob(&outbound[0].1).arguments[0]
            .parse::<i64>()
            .expect("challenge");
        let request = Q2ConnectRequest {
            protocol: ProtocolIdentity::Q2Classic,
            qport: 9999,
            challenge: foreign_challenge,
            userinfo: "\\name\\other".to_string(),
            payload_bytes: 1390,
            channel: ChannelKind::New,
            compression: false,
            social_ids: None,
        };
        handles.queue(foreign_addr(), write_q2_connect(&request).expect("connect"));
        server.poll(3000).expect("poll");
        let outbound = handles.take_outbound();
        assert!(parse_oob(&outbound[0].1).body.contains("Server is full"));
        assert_eq!(server.clients().len(), 1);

        // Host rejection surfaces the reason.
        let (mut host, _log) = MockServerHost::new();
        host.reject_admit = Some("banned".to_string());
        let (mut server, handles) = test_server(host);
        let challenge = challenge_through(&mut server, &handles);
        let request = Q2ConnectRequest {
            protocol: ProtocolIdentity::Q2Classic,
            qport: 1234,
            challenge,
            userinfo: "\\name\\tester".to_string(),
            payload_bytes: 1390,
            channel: ChannelKind::New,
            compression: false,
            social_ids: None,
        };
        handles.queue(client_addr(), write_q2_connect(&request).expect("connect"));
        server.poll(2000).expect("poll");
        let outbound = handles.take_outbound();
        assert!(parse_oob(&outbound[0].1).body.contains("banned"));
        assert!(server.clients().is_empty());
    }

    #[test]
    fn server_answers_ping_status_and_info() {
        let (host, _log) = MockServerHost::new();
        let (mut server, handles) = test_server(host.with_discovery());
        handles.queue(client_addr(), q2_out_of_band("ping", false));
        server.poll(1000).expect("poll");
        let outbound = handles.take_outbound();
        assert_eq!(parse_oob(&outbound[0].1).command, "ack");

        handles.queue(client_addr(), q2_out_of_band("status", false));
        server.poll(1000).expect("poll");
        let outbound = handles.take_outbound();
        let status = parse_oob(&outbound[0].1);
        assert_eq!(status.command, "print");
        assert!(status.body.contains("tester"));

        handles.queue(client_addr(), q2_out_of_band("info 34", false));
        server.poll(1000).expect("poll");
        let outbound = handles.take_outbound();
        let info = parse_oob(&outbound[0].1);
        assert_eq!(info.command, "info");
        assert!(info.body.contains("base1"));

        handles.queue(client_addr(), q2_out_of_band("info 36", false));
        server.poll(1000).expect("poll");
        let outbound = handles.take_outbound();
        assert!(parse_oob(&outbound[0].1).body.contains("wrong version"));
    }

    #[test]
    fn server_rcon_executes_with_password() {
        let (host, log) = MockServerHost::new();
        let (mut server, handles) = test_server(host.with_admin("secret"));
        handles.queue(client_addr(), q2_out_of_band("rcon secret say hi", false));
        server.poll(1000).expect("poll");
        assert_eq!(log.lock().expect("log").rcon.len(), 1);
        assert!(log.lock().expect("log").rcon[0].0.contains("say hi"));
        assert!(!handles.take_outbound().is_empty());

        handles.queue(client_addr(), q2_out_of_band("rcon wrong say hi", false));
        server.poll(1000).expect("poll");
        assert_eq!(log.lock().expect("log").rcon.len(), 1);
        let outbound = handles.take_outbound();
        assert!(parse_oob(&outbound[0].1).body.contains("Bad rcon_password"));
    }

    #[test]
    fn server_signon_activates_and_publishes_frames() {
        let (host, log) = MockServerHost::new();
        let (mut server, handles) = test_server(host);
        let challenge = challenge_through(&mut server, &handles);
        connect_through(&mut server, &handles, challenge);
        let mut channel = test_client_channel();
        signon_step(&mut channel, &mut server, &handles, 3000, "new");
        signon_step(&mut channel, &mut server, &handles, 3100, "configstrings 1 0");
        signon_step(&mut channel, &mut server, &handles, 3200, "baselines 1 0");
        signon_step(&mut channel, &mut server, &handles, 3300, "begin 1");
        assert_eq!(log.lock().expect("log").begins, vec![0]);

        server.publish(&test_output(7, 700), &[], 3400).expect("publish");
        assert_eq!(log.lock().expect("log").frames, vec![0]);
        assert_eq!(log.lock().expect("log").observes, 1);
        assert!(!handles.take_outbound().is_empty());

        // Republishing the same frame takes the duplicate path without a new encode.
        server.publish(&test_output(7, 750), &[], 3500).expect("publish");
        assert_eq!(log.lock().expect("log").frames, vec![0, 0]);
    }

    #[test]
    fn server_move_queues_remote_input() {
        let (host, log) = MockServerHost::new();
        let (mut server, handles) = test_server(host);
        let challenge = challenge_through(&mut server, &handles);
        connect_through(&mut server, &handles, challenge);
        let mut channel = test_client_channel();

        // Moves before `begin` are ignored.
        let mut wire = Q2Wire::new(ProtocolIdentity::Q2Classic).expect("wire");
        let moves = encode_q2_move(
            &mut wire,
            channel.outgoing_sequence(),
            -1,
            &[Usercmd::default(), Usercmd::default(), Usercmd::default()],
        )
        .expect("move");
        let packet = channel.transmit(&moves, 3000).expect("transmit");
        handles.queue(client_addr(), packet);
        let commands = server.poll(3000).expect("poll");
        feed_acks(&mut channel, &handles, 3000);
        assert!(commands.is_empty());
        assert!(log.lock().expect("log").inputs.is_empty());

        for (step, text) in ["new", "configstrings 1 0", "baselines 1 0", "begin 1"]
            .into_iter()
            .enumerate()
        {
            signon_step(&mut channel, &mut server, &handles, 3100 + step as u64 * 100, text);
        }
        let moves = encode_q2_move(
            &mut wire,
            channel.outgoing_sequence(),
            -1,
            &[Usercmd::default(), Usercmd::default(), Usercmd::default()],
        )
        .expect("move");
        let packet = channel.transmit(&moves, 3600).expect("transmit");
        handles.queue(client_addr(), packet);
        let commands = server.poll(3600).expect("poll");
        feed_acks(&mut channel, &handles, 3600);
        assert_eq!(commands.len(), 1);
        assert!(matches!(commands[0].source, CommandSource::Remote { .. }));
        assert_eq!(commands[0].sequence, 0);
        assert_eq!(log.lock().expect("log").inputs, vec![(0, 0)]);
    }

    #[test]
    fn server_userinfo_settings_and_raw_commands() {
        let (host, log) = MockServerHost::new();
        let (mut server, handles) = test_server(host);
        let challenge = challenge_through(&mut server, &handles);
        connect_through(&mut server, &handles, challenge);
        let mut channel = test_client_channel();
        for (step, text) in ["new", "configstrings 1 0", "baselines 1 0", "begin 1"]
            .into_iter()
            .enumerate()
        {
            signon_step(&mut channel, &mut server, &handles, 3100 + step as u64 * 100, text);
        }
        server.publish(&test_output(7, 700), &[], 3500).expect("publish");
        feed_acks(&mut channel, &handles, 3500);

        let bytes =
            encode_q2_client_control(&Q2ClientEvent::Userinfo("\\name\\renamed".to_string()), false).expect("userinfo");
        channel.queue_reliable(&bytes).expect("queue");
        let packet = channel.transmit(&[], 3600).expect("transmit");
        handles.queue(client_addr(), packet);
        server.poll(3600).expect("poll");
        server.publish(&test_output(8, 800), &[], 3600).expect("publish");
        feed_acks(&mut channel, &handles, 3600);
        assert!(log
            .lock()
            .expect("log")
            .userinfos
            .iter()
            .any(|(slot, value)| *slot == 0 && value.contains("renamed")));

        send_control(&mut channel, &handles, 3700, "say   hello");
        server.poll(3700).expect("poll");
        assert!(log
            .lock()
            .expect("log")
            .texts
            .iter()
            .any(|(slot, text)| *slot == 0 && text == "say   hello"));
    }

    #[test]
    fn server_drops_unsupported_wire_opcode() {
        let (host, log) = MockServerHost::new();
        let (mut server, handles) = test_server(host);
        let challenge = challenge_through(&mut server, &handles);
        connect_through(&mut server, &handles, challenge);
        let mut channel = test_client_channel();
        for (step, text) in ["new", "configstrings 1 0", "baselines 1 0", "begin 1"]
            .into_iter()
            .enumerate()
        {
            signon_step(&mut channel, &mut server, &handles, 3100 + step as u64 * 100, text);
        }
        server.publish(&test_output(7, 700), &[], 3500).expect("publish");
        feed_acks(&mut channel, &handles, 3500);
        let bytes = encode_q2_client_control(
            &Q2ClientEvent::UserinfoDelta {
                name: "skin".to_string(),
                value: "male".to_string(),
            },
            false,
        )
        .expect("delta");
        channel.queue_reliable(&bytes).expect("queue");
        let packet = channel.transmit(&[], 3600).expect("transmit");
        handles.queue(client_addr(), packet);
        server.poll(3600).expect("poll");
        assert!(server.clients().is_empty());
        assert_eq!(
            log.lock().expect("log").disconnects,
            vec![(0, "Userinfo delta is not supported by selected Q2 wire".to_string())]
        );
    }

    #[test]
    fn server_disconnect_command_drops_peer() {
        let (host, log) = MockServerHost::new();
        let (mut server, handles) = test_server(host);
        let challenge = challenge_through(&mut server, &handles);
        connect_through(&mut server, &handles, challenge);
        let mut channel = test_client_channel();
        signon_step(&mut channel, &mut server, &handles, 3000, "new");
        signon_step(&mut channel, &mut server, &handles, 3100, "disconnect");
        assert!(server.clients().is_empty());
        assert_eq!(
            log.lock().expect("log").disconnects,
            vec![(0, "Client disconnected".to_string())]
        );
    }

    #[test]
    fn server_download_serves_and_missing_drops() {
        let (host, _log) = MockServerHost::new();
        let bytes: Vec<u8> = (0..64u8).collect();
        let (mut server, handles) = test_server(
            host.with_file("maps/test.bsp", bytes)
                .with_failing("maps/bad.bsp", vec![9u8; 32]),
        );
        let challenge = challenge_through(&mut server, &handles);
        connect_through(&mut server, &handles, challenge);
        let mut channel = test_client_channel();
        signon_step(&mut channel, &mut server, &handles, 3000, "new");
        signon_step(&mut channel, &mut server, &handles, 3100, "download maps/test.bsp");
        signon_step(&mut channel, &mut server, &handles, 3200, "nextdl");
        assert_eq!(server.clients().len(), 1);

        // A missing file refuses gracefully without dropping the peer.
        signon_step(&mut channel, &mut server, &handles, 3300, "download maps/missing.bsp");
        server.poll(3400).expect("poll");
        assert_eq!(server.clients().len(), 1);

        // A read failure drops the peer on the next poll.
        signon_step(&mut channel, &mut server, &handles, 3500, "download maps/bad.bsp");
        server.poll(3600).expect("poll");
        assert!(server.clients().is_empty());
    }

    #[test]
    fn server_recording_lifecycle() {
        let (host, _log) = MockServerHost::new();
        let (mut server, _handles) = test_server(host.with_capture(2010));
        server.publish(&test_output(7, 700), &[], 3400).expect("publish");

        let seed = server.server_recording().expect("tap").seed().expect("seed");
        assert!(matches!(seed.identity, DemoRecordingIdentity::Q2Server));
        assert_eq!(seed.packets.len(), 1);

        let (sink, packets) = MockSink::new();
        {
            let detach = server
                .server_recording()
                .expect("tap")
                .attach(Box::new(sink))
                .expect("attach");
            detach();
        }
        server.publish(&test_output(8, 800), &[], 3500).expect("publish");
        assert!(packets.lock().expect("packets").is_empty());

        // Dropping the detach handle keeps the recording attached.
        server.server_recording().expect("tap").seed().expect("seed");
        let (sink, packets) = MockSink::new();
        {
            let detach = server
                .server_recording()
                .expect("tap")
                .attach(Box::new(sink))
                .expect("attach");
            std::mem::drop(detach);
        }
        server.publish(&test_output(9, 900), &[], 3600).expect("publish");
        assert_eq!(packets.lock().expect("packets").len(), 1);

        // Attaching twice fails while the owner holds the sink.
        let (sink, _packets) = MockSink::new();
        let error = server
            .server_recording()
            .expect("tap")
            .attach(Box::new(sink))
            .err()
            .expect("busy");
        assert_eq!(error.to_string(), "Server recording requires a current unused seed");
    }

    #[test]
    fn server_recording_revision_guards() {
        let (host, _log) = MockServerHost::new();
        let (mut server, _handles) = test_server(host.with_capture(2011));
        server.publish(&test_output(7, 700), &[], 3400).expect("publish");
        let error = server.server_recording().expect("tap").seed().expect_err("revision");
        assert_eq!(
            error.to_string(),
            "serverrecord requires classic Quake II; use mvdrecord for other source revisions"
        );
        let seed = server.mvd_recording().expect("tap").seed().expect("mvd seed");
        assert!(matches!(seed.identity, DemoRecordingIdentity::Mvd { .. }));
        assert!(!seed.packets.is_empty());

        let (host, _log) = MockServerHost::new();
        let (mut server, _handles) = test_server(host.with_capture(2000));
        server.publish(&test_output(7, 700), &[], 3400).expect("publish");
        let error = server.mvd_recording().expect("tap").seed().expect_err("revision");
        assert_eq!(error.to_string(), "Unsupported MVD recording revision");

        // No published frame means no capture source.
        let (host, _log) = MockServerHost::new();
        let (mut server, _handles) = test_server(host.with_capture(2010));
        let error = server.server_recording().expect("tap").seed().expect_err("no frame");
        assert_eq!(
            error.to_string(),
            "MVD recording requires an active authoritative Q2 capture source"
        );
    }

    #[test]
    fn server_recording_failure_clears_owner() {
        let (host, log) = MockServerHost::new();
        let (mut server, _handles) = test_server(host.with_capture(2010));
        server.publish(&test_output(7, 700), &[], 3400).expect("publish");
        let seed = server.server_recording().expect("tap").seed().expect("seed");
        assert_eq!(seed.packets.len(), 1);
        {
            let detach = server
                .server_recording()
                .expect("tap")
                .attach(Box::new(FailingSink))
                .expect("attach");
            std::mem::drop(detach);
        }
        server.publish(&test_output(8, 800), &[], 3500).expect("publish");
        assert_eq!(log.lock().expect("log").prints.len(), 1);
        assert!(log.lock().expect("log").prints[0].contains("Server recording failed"));
        server.publish(&test_output(9, 900), &[], 3600).expect("publish");
        assert_eq!(log.lock().expect("log").prints.len(), 1);
    }

    #[test]
    fn server_mvd_publishes_on_tick_change() {
        let (host, _log) = MockServerHost::new();
        let (mut server, _handles) = test_server(host.with_capture(2010));
        server.publish(&test_output(7, 700), &[], 3400).expect("publish");
        server.mvd_recording().expect("tap").seed().expect("seed");
        let (sink, packets) = MockSink::new();
        {
            let detach = server
                .mvd_recording()
                .expect("tap")
                .attach(Box::new(sink))
                .expect("attach");
            std::mem::drop(detach);
        }
        // Same 10 Hz tick: buffered, not written.
        server.publish(&test_output(8, 710), &[], 3500).expect("publish");
        assert!(packets.lock().expect("packets").is_empty());
        // New tick: the buffered capture is written.
        server.publish(&test_output(9, 800), &[], 3600).expect("publish");
        assert!(!packets.lock().expect("packets").is_empty());
    }

    #[test]
    fn server_change_world_bumps_generation() {
        let (host, log) = MockServerHost::new();
        let (mut server, handles) = test_server(host);
        let challenge = challenge_through(&mut server, &handles);
        connect_through(&mut server, &handles, challenge);
        let mut channel = test_client_channel();
        for (step, text) in ["new", "configstrings 1 0", "baselines 1 0", "begin 1"]
            .into_iter()
            .enumerate()
        {
            signon_step(&mut channel, &mut server, &handles, 3100 + step as u64 * 100, text);
        }
        assert_eq!(log.lock().expect("log").begins, vec![0]);

        let (fresh, _fresh_log) = MockServerHost::new();
        server.change_world(fresh).expect("change world");
        assert_eq!(server.clients().len(), 1);

        // Stale generation numbers restart signon instead of activating.
        for (step, text) in ["new", "configstrings 1 0", "baselines 1 0", "begin 1"]
            .into_iter()
            .enumerate()
        {
            signon_step(&mut channel, &mut server, &handles, 4100 + step as u64 * 100, text);
        }
        assert_eq!(log.lock().expect("log").begins, vec![0]);
        server.publish(&test_output(7, 700), &[], 4500).expect("publish");
        assert!(handles.take_outbound().is_empty());

        // Current generation numbers activate.
        for (step, text) in ["configstrings 2 0", "baselines 2 0", "begin 2"]
            .into_iter()
            .enumerate()
        {
            signon_step(&mut channel, &mut server, &handles, 4600 + step as u64 * 100, text);
        }
        server.publish(&test_output(7, 700), &[], 4900).expect("publish");
        assert!(!handles.take_outbound().is_empty());

        // Protocol changes and active recordings block the transition.
        let (mut other, _log) = MockServerHost::new();
        other.protocol = Q2ProtocolIdentity::Rerelease;
        let error = server.change_world(other).expect_err("protocol");
        assert_eq!(error.to_string(), "Native Q2 map transition cannot change protocol");
    }

    #[test]
    fn server_disconnect_client_notifies() {
        let (host, log) = MockServerHost::new();
        let (mut server, handles) = test_server(host);
        let challenge = challenge_through(&mut server, &handles);
        connect_through(&mut server, &handles, challenge);
        let client = server.clients()[0].client.clone();
        assert!(server.disconnect_client(&client, "bye"));
        assert!(server.clients().is_empty());
        assert_eq!(log.lock().expect("log").disconnects, vec![(0, "bye".to_string())]);
        assert!(!handles.take_outbound().is_empty());

        let owner = IdentityOwner::create("q2-unknown-test").expect("owner");
        assert!(!server.disconnect_client(&owner.client(9, 0), "bye"));
    }

    #[test]
    fn server_poll_timeout_drops_and_reports_errors() {
        let (host, log) = MockServerHost::new();
        let (mut server, handles) = test_server(host);
        let challenge = challenge_through(&mut server, &handles);
        connect_through(&mut server, &handles, challenge);
        server.poll(2000 + 125_001).expect("poll");
        assert!(server.clients().is_empty());
        assert_eq!(
            log.lock().expect("log").disconnects,
            vec![(0, "Connection timed out".to_string())]
        );

        handles.queue_event(ReceiveEvent::Error {
            error: "boom".to_string(),
        });
        server.poll(3000).expect("poll");
        assert!(log.lock().expect("log").prints.iter().any(|text| text.contains("boom")));

        handles.queue_event(ReceiveEvent::Dropped {
            reason: qa_net::common::transport::DropReason::Oversize,
            from: client_addr(),
        });
        server.poll(3000).expect("poll");
    }

    #[test]
    fn server_rejects_skips_packets() {
        let (mut host, _log) = MockServerHost::new();
        host.reject_addr = Some(client_addr());
        let (mut server, handles) = test_server(host);
        handles.queue(client_addr(), q2_out_of_band("getchallenge", false));
        server.poll(1000).expect("poll");
        assert!(handles.take_outbound().is_empty());
    }

    #[test]
    fn server_submit_rejected_and_close_clears() {
        let (host, log) = MockServerHost::new();
        let (mut server, handles) = test_server(host);
        let challenge = challenge_through(&mut server, &handles);
        connect_through(&mut server, &handles, challenge);
        let error = server.submit(&[], 2000).expect_err("submit");
        assert_eq!(
            error.to_string(),
            "Q2 server commands must enter the authoritative application input batch"
        );
        server.close();
        assert_eq!(server.phase(), ApplicationNetworkPhase::Closed);
        assert!(server.clients().is_empty());
        assert_eq!(
            log.lock().expect("log").disconnects,
            vec![(0, "Server shutdown".to_string())]
        );
        assert!(handles.is_closed());
        assert!(server.poll(3000).expect("poll").is_empty());
        server.close();
    }

    #[test]
    fn server_heartbeat_publishes_and_throttles() {
        let master = NetworkAddress::Ipv4 {
            host: [203, 0, 113, 7],
            port: 27900,
        };
        let (mut host, _log) = MockServerHost::new();
        host.masters = Some(vec![master.clone()]);
        let (mut server, handles) = test_server(host.with_discovery());
        server.heartbeat(1000);
        let outbound = handles.take_outbound();
        assert_eq!(outbound.len(), 1);
        assert_eq!(outbound[0].0, master);

        // Poll heartbeats throttle within one interval.
        server.poll(301_000).expect("poll");
        server.poll(301_000).expect("poll");
        let outbound = handles.take_outbound();
        assert_eq!(outbound.len(), 1);
    }

    #[test]
    #[should_panic(expected = "Q2 master publication requires source status")]
    fn server_heartbeat_without_status_panics() {
        let master = NetworkAddress::Ipv4 {
            host: [203, 0, 113, 7],
            port: 27900,
        };
        let (mut host, _log) = MockServerHost::new();
        host.masters = Some(vec![master]);
        let (mut server, _handles) = test_server(host);
        server.heartbeat(1000);
    }

    #[test]
    fn server_broadcast_requires_ip_and_capture() {
        let (mut host, _log) = MockServerHost::new();
        host.mvd_settings = Some(Q2MvdSettings {
            enabled: true,
            max_viewers: 4,
            password: String::new(),
        });
        let (transport, _handles) = MockTransport::new(NetworkAddress::Loopback {
            id: "q2-test".to_string(),
        });
        let mut server = Q2ServerNetwork::new(Q2ServerNetworkOptions {
            transport,
            host,
            random: Box::new(|| 0.5),
            timeout_milliseconds: None,
        })
        .expect("server");
        let error = server.poll(1000).expect_err("ip required");
        assert_eq!(error.to_string(), "GTV broadcast requires an IP server transport");

        // Without a capture hook the broadcast cannot start once frames publish.
        let (mut host, _log) = MockServerHost::new();
        host.mvd_settings = Some(Q2MvdSettings {
            enabled: true,
            max_viewers: 4,
            password: String::new(),
        });
        let (mut server, _handles) = test_server(host);
        server.publish(&test_output(7, 700), &[], 1000).expect("publish");
        let error = server.poll(1000).expect_err("capture required");
        assert_eq!(
            error.to_string(),
            "GTV broadcast requires an authoritative Q2 capture source"
        );
    }

    fn shake_to_loading(client: &mut Q2ClientNetwork<MockTransport, MockClientHost>, handles: &MockTransportHandles) {
        client.poll(1000).expect("poll");
        let outbound = handles.take_outbound();
        assert_eq!(parse_oob(&outbound[0].1).command, "getchallenge");
        handles.queue(server_addr(), q2_out_of_band("challenge 777 p=34", false));
        client.poll(1100).expect("poll");
        client.poll(1200).expect("poll");
        let outbound = handles.take_outbound();
        assert_eq!(parse_oob(&outbound[0].1).command, "connect");
        handles.queue(server_addr(), q2_out_of_band("client_connect", false));
        client.poll(1300).expect("poll");
        assert_eq!(client.phase(), ApplicationNetworkPhase::Loading);
        assert!(!handles.take_outbound().is_empty());
    }

    #[test]
    fn client_handshake_reaches_loading() {
        let (host, _log) = MockClientHost::new();
        let (mut client, handles) = test_client(host);
        assert_eq!(client.phase(), ApplicationNetworkPhase::Challenging);
        shake_to_loading(&mut client, &handles);
        assert_eq!(client.acknowledged_frame(), -1);
    }

    #[test]
    fn client_connect_propagates_download_server() {
        let (host, _log) = MockClientHost::new();
        let (host, http_server) = host.with_downloads();
        let (mut client, handles) = test_client(host);
        client.poll(1000).expect("poll");
        handles.take_outbound();
        handles.queue(server_addr(), q2_out_of_band("challenge 777 p=34", false));
        client.poll(1100).expect("poll");
        client.poll(1200).expect("poll");
        handles.take_outbound();
        handles.queue(
            server_addr(),
            q2_out_of_band("client_connect dlserver=http://example.com", false),
        );
        client.poll(1300).expect("poll");
        assert_eq!(
            http_server.lock().expect("http").clone(),
            Some("http://example.com/".to_string())
        );
    }

    #[test]
    fn client_handshake_rejects_incompatible_challenge() {
        let (host, log) = MockClientHost::new();
        let (mut client, handles) = test_client(host);
        client.poll(1000).expect("poll");
        handles.take_outbound();
        handles.queue(server_addr(), q2_out_of_band("challenge 5 p=1038", false));
        client.poll(1100).expect("poll");
        assert_eq!(client.phase(), ApplicationNetworkPhase::Rejected);
        assert_eq!(
            log.lock().expect("log").disconnects,
            vec!["No compatible advertised Quake II protocol".to_string()]
        );
        assert!(client.poll(1200).expect("poll").is_empty());
    }

    #[test]
    fn client_print_displays_and_ignores_foreign_packets() {
        let (host, log) = MockClientHost::new();
        let (mut client, handles) = test_client(host);
        handles.queue(server_addr(), q2_out_of_band("print\nhello", false));
        client.poll(1000).expect("poll");
        assert_eq!(log.lock().expect("log").prints, vec!["hello".to_string()]);

        handles.queue(foreign_addr(), q2_out_of_band("client_connect", false));
        client.poll(1100).expect("poll");
        assert_eq!(client.phase(), ApplicationNetworkPhase::Challenging);
    }

    #[test]
    fn client_command_guards_pre_connect() {
        let (host, _log) = MockClientHost::new();
        let (mut client, _handles) = test_client(host);
        let error = client.command("say hi").expect_err("command");
        assert_eq!(error.to_string(), "Q2 client is not connected");
        client.userinfo("\\name\\x").expect("userinfo noop");
    }

    #[test]
    fn client_timeout_rejects() {
        let (host, log) = MockClientHost::new();
        let (mut client, handles) = test_client(host);
        handles.queue(server_addr(), q2_out_of_band("print\nhello", false));
        client.poll(1000).expect("poll");
        client.poll(1000 + 120_001).expect("poll");
        assert_eq!(client.phase(), ApplicationNetworkPhase::Rejected);
        assert_eq!(
            log.lock().expect("log").disconnects,
            vec!["Connection timed out".to_string()]
        );
    }

    #[test]
    fn client_submit_publish_and_recording_guards() {
        let (host, _log) = MockClientHost::new();
        let (mut client, _handles) = test_client(host);
        let owner = IdentityOwner::create("q2-submit-test").expect("owner");
        let command = ActorCommand {
            actor: owner.actor(0, 0),
            source: CommandSource::LocalSeat { seat: owner.seat(0) },
            sequence: 0,
            command: UserCommand::Q2Classic {
                milliseconds: 0.0,
                angle_shorts: [0.0; 3],
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0.0,
                impulse: 0.0,
                light_level: 0.0,
            },
            arsenal: None,
        };
        client
            .submit(&[command], 1000)
            .expect("submit ignored while challenging");
        let error = client.publish(&test_output(1, 100), &[], 1000).expect_err("publish");
        assert_eq!(
            error.to_string(),
            "Remote Q2 client cannot publish authoritative server state"
        );

        let (sink, _packets) = MockSink::new();
        let error = client
            .recording()
            .expect("tap")
            .attach(Box::new(sink))
            .err()
            .expect("attach");
        assert_eq!(error.to_string(), "Recording requires an active Q2 connection");
    }

    #[test]
    fn client_close_marks_closed() {
        let (host, _log) = MockClientHost::new();
        let (mut client, handles) = test_client(host);
        client.close();
        assert_eq!(client.phase(), ApplicationNetworkPhase::Closed);
        assert!(handles.is_closed());
        assert!(handles.take_outbound().is_empty());
        client.close();
    }

    #[test]
    fn client_close_connected_sends_disconnect() {
        let (host, _log) = MockClientHost::new();
        let (mut client, handles) = test_client(host);
        shake_to_loading(&mut client, &handles);
        client.close();
        assert_eq!(client.phase(), ApplicationNetworkPhase::Closed);
        assert!(handles.is_closed());
        assert!(!handles.take_outbound().is_empty());
    }

    /// Forward queued datagrams between the paired transports.
    fn forward(server_handles: &MockTransportHandles, client_handles: &MockTransportHandles) {
        for (to, bytes) in server_handles.take_outbound() {
            assert_eq!(to, client_addr());
            client_handles.queue(server_addr(), bytes);
        }
        for (to, bytes) in client_handles.take_outbound() {
            assert_eq!(to, server_addr());
            server_handles.queue(client_addr(), bytes);
        }
    }

    #[test]
    fn client_server_full_session() {
        let (server_host, server_log) = MockServerHost::new();
        let (mut server, server_handles) = test_server(server_host);
        let (client_host, client_log) = MockClientHost::new();
        let (mut client, client_handles) = test_client(client_host);

        // Drive the handshake and signon to active.
        let mut tick = 0i32;
        for _ in 0..300 {
            tick += 1;
            let now = tick as u64 * 50;
            forward(&server_handles, &client_handles);
            server.poll(now).expect("server poll");
            client.poll(now).expect("client poll");
            server
                .publish(&test_output(tick, tick * 50), &[], now)
                .expect("publish");
            if client.phase() == ApplicationNetworkPhase::Active {
                break;
            }
        }
        assert_eq!(client.phase(), ApplicationNetworkPhase::Active);
        tick += 1;
        let now = tick as u64 * 50;
        forward(&server_handles, &client_handles);
        server.poll(now).expect("server poll");
        client.poll(now).expect("client poll");
        assert_eq!(server.clients().len(), 1);
        assert_eq!(client_log.lock().expect("log").game_states, vec![1]);
        assert!(!server_log.lock().expect("log").begins.is_empty());

        // Client input reaches the server poll as remote commands.
        let owner = IdentityOwner::create("q2-session-test").expect("owner");
        let command = ActorCommand {
            actor: owner.actor(0, 0),
            source: CommandSource::LocalSeat { seat: owner.seat(0) },
            sequence: 7,
            command: UserCommand::Q2Classic {
                milliseconds: 50.0,
                angle_shorts: [0.0; 3],
                forward_move: 100.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0.0,
                impulse: 0.0,
                light_level: 0.0,
            },
            arsenal: None,
        };
        client
            .submit(&[command.clone(), command.clone()], 0)
            .expect_err("multi submit");
        client.submit(&[command], 0).expect("submit");
        let now = (tick + 1) as u64 * 50;
        client.poll(now).expect("client poll");
        forward(&server_handles, &client_handles);
        let inputs = server.poll(now).expect("server poll");
        assert!(inputs
            .iter()
            .any(|input| matches!(input.source, CommandSource::Remote { .. })));
        assert!(!client_log.lock().expect("log").sent.is_empty());

        // Server frames reach the client host.
        let frame = tick + 100;
        let now = (tick + 2) as u64 * 50;
        server
            .publish(&test_output(frame, frame * 50), &[], now)
            .expect("publish");
        forward(&server_handles, &client_handles);
        server.poll(now).expect("server poll");
        client.poll(now).expect("client poll");
        assert!(client_log
            .lock()
            .expect("log")
            .frames
            .iter()
            .any(|(number, _)| *number == frame));
        assert!(!client_log.lock().expect("log").acked.is_empty());

        // Client recording captures channel records.
        client.recording().expect("tap").seed().expect("seed");
        let (sink, packets) = MockSink::new();
        {
            let detach = client.recording().expect("tap").attach(Box::new(sink)).expect("attach");
            std::mem::drop(detach);
        }
        let now = (tick + 3) as u64 * 50;
        server
            .publish(&test_output(frame + 1, frame * 50), &[], now)
            .expect("publish");
        forward(&server_handles, &client_handles);
        server.poll(now).expect("server poll");
        client.poll(now).expect("client poll");
        assert!(!packets.lock().expect("packets").is_empty());

        // Client close drops the server peer.
        client.close();
        let now = (tick + 4) as u64 * 50;
        forward(&server_handles, &client_handles);
        server.poll(now).expect("server poll");
        assert!(server.clients().is_empty());
        assert!(!server_log.lock().expect("log").disconnects.is_empty());
    }

    #[test]
    fn client_server_survives_world_change() {
        let (server_host, _server_log) = MockServerHost::new();
        let (mut server, server_handles) = test_server(server_host);
        let (client_host, client_log) = MockClientHost::new();
        let (mut client, client_handles) = test_client(client_host);

        let mut tick = 0i32;
        for _ in 0..300 {
            tick += 1;
            let now = tick as u64 * 50;
            forward(&server_handles, &client_handles);
            server.poll(now).expect("server poll");
            client.poll(now).expect("client poll");
            server
                .publish(&test_output(tick, tick * 50), &[], now)
                .expect("publish");
            if client.phase() == ApplicationNetworkPhase::Active {
                break;
            }
        }
        assert_eq!(client.phase(), ApplicationNetworkPhase::Active);

        let (fresh, _fresh_log) = MockServerHost::new();
        server.change_world(fresh).expect("change world");
        for _ in 0..300 {
            tick += 1;
            let now = tick as u64 * 50;
            forward(&server_handles, &client_handles);
            server.poll(now).expect("server poll");
            client.poll(now).expect("client poll");
            server
                .publish(&test_output(tick, tick * 50), &[], now)
                .expect("publish");
            if client.phase() == ApplicationNetworkPhase::Active
                && client_log.lock().expect("log").game_states.len() == 2
            {
                break;
            }
        }
        assert_eq!(client.phase(), ApplicationNetworkPhase::Active);
        assert_eq!(client_log.lock().expect("log").game_states, vec![1, 2]);
    }

    #[test]
    fn negotiate_takes_minimum_variant_revision() {
        let configured = Q2ProtocolIdentity::R1Q2 {
            revision: R1Q2Revision::R1905,
        };
        let offered = Q2ProtocolIdentity::R1Q2 {
            revision: R1Q2Revision::R1903,
        };
        assert_eq!(
            negotiate_q2_protocol(&configured, &offered),
            Q2ProtocolIdentity::R1Q2 {
                revision: R1Q2Revision::R1903
            }
        );
        let configured = Q2ProtocolIdentity::Q2Pro {
            revision: Q2ProRevision::R1021,
        };
        let offered = Q2ProtocolIdentity::Q2Pro {
            revision: Q2ProRevision::R1026,
        };
        assert_eq!(
            negotiate_q2_protocol(&configured, &offered),
            Q2ProtocolIdentity::Q2Pro {
                revision: Q2ProRevision::R1021
            }
        );
        // Other identities pass the offer through.
        let configured = Q2ProtocolIdentity::Classic;
        let offered = Q2ProtocolIdentity::Classic;
        assert_eq!(negotiate_q2_protocol(&configured, &offered), offered);
        let offered = Q2ProtocolIdentity::Rerelease;
        assert_eq!(negotiate_q2_protocol(&configured, &offered), offered);
    }

    #[test]
    fn server_q2pro_forwards_settings_and_deltas_to_primary() {
        let (mut host, log) = MockServerHost::new();
        host.protocol = Q2ProtocolIdentity::Q2Pro {
            revision: Q2ProRevision::R1021,
        };
        let (mut server, handles) = test_server(host);
        handles.queue(client_addr(), q2_out_of_band("getchallenge", false));
        server.poll(1000).expect("poll");
        let outbound = handles.take_outbound();
        let challenge = parse_oob(&outbound[0].1).arguments[0]
            .parse::<i64>()
            .expect("challenge");

        let request = Q2ConnectRequest {
            protocol: ProtocolIdentity::Q2Q2pro { revision: 1021 },
            qport: 1234,
            challenge,
            userinfo: "\\name\\tester".to_string(),
            payload_bytes: 1390,
            channel: ChannelKind::Old,
            compression: false,
            social_ids: None,
        };
        handles.queue(client_addr(), write_q2_connect(&request).expect("connect"));
        server.poll(2000).expect("poll");
        let outbound = handles.take_outbound();
        assert_eq!(parse_oob(&outbound[0].1).command, "client_connect");
        assert_eq!(server.clients().len(), 1);

        // Enhanced wires mask the qport to one byte.
        let mut channel = Q2Channel::new(Q2ChannelOptions {
            side: ChannelSide::Client,
            protocol: ProtocolIdentity::Q2Q2pro { revision: 1021 },
            channel: ChannelKind::Old,
            qport: 1234 & 255,
            payload_bytes: Some(1390),
            message_bytes: None,
            max_datagram_bytes: Some(65_507),
            compress: false,
            sequence_recording: None,
        })
        .expect("channel");
        for (step, text) in ["new", "configstrings 1 0", "baselines 1 0", "begin 1"]
            .into_iter()
            .enumerate()
        {
            signon_step(&mut channel, &mut server, &handles, 3100 + step as u64 * 100, text);
        }
        assert_eq!(log.lock().expect("log").begins, vec![0]);
        server.publish(&test_output(7, 700), &[], 3500).expect("publish");
        feed_acks(&mut channel, &handles, 3500);

        let setting = encode_q2_client_control(&Q2ClientEvent::Setting { index: 1, value: 2 }, false).expect("setting");
        let delta = encode_q2_client_control(
            &Q2ClientEvent::UserinfoDelta {
                name: "skin".to_string(),
                value: "male".to_string(),
            },
            false,
        )
        .expect("delta");
        channel.queue_reliable(&setting).expect("queue");
        channel.queue_reliable(&delta).expect("queue");
        let packet = channel.transmit(&[], 3600).expect("transmit");
        handles.queue(client_addr(), packet);
        server.poll(3600).expect("poll");
        let commands = log.lock().expect("log").commands.clone();
        assert!(commands.iter().any(|(slot, name, args)| *slot == 0
            && name == "set_setting"
            && args == &["1".to_string(), "2".to_string()]));
        assert!(commands.iter().any(|(slot, name, args)| *slot == 0
            && name == "userinfo_delta"
            && args == &["skin".to_string(), "male".to_string()]));
    }
}
