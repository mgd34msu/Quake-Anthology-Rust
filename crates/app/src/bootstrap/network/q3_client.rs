//! Quake III client network (donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q3-client.ts`).
//!
//! The donor is `async`; this sync port resolves every host call inline.
//! `qa-net`'s [`Q3ClientConnection`](qa_net::q3_net::Q3ClientConnection)
//! borrows its bindings `&mut`, so the endpoint owns both through
//! [`Q3ConnectionCell`](super::types::Q3ConnectionCell): the bindings adapter
//! lives on the heap and shared state travels through `Rc` handles, never
//! borrows. Binding callbacks never touch the connection: value-returning
//! queries (`download_size`) run inline, and every other callback is queued
//! and drained in order after each connection call, preserving donor order.
//! A host panic during the drain rejects the connection exactly like the
//! donor's `catch` in `poll`.

use std::any::Any;
use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use qa_core::cvar::CvarRegistry;
use qa_net::common::commands::ActorCommand;
use qa_net::common::endpoint::{same_address, NetworkAddress};
use qa_net::common::session::WireSelection;
use qa_net::common::transport::{DatagramTransport, ReceiveEvent, TransportError};
use qa_net::protocol::ProtocolIdentity;
use qa_net::q3::{MessageMode, Q3MsgError, Q3MsgReader, WireUserCommand};
use qa_net::q3_client_authorization::Q3ClientAuthorization;
use qa_net::q3_net::{
    q3_is_lan_address, ChannelDelivery, ConnectionlessPacket, DownloadBlock, Gamestate, Q3ClientAdmission,
    Q3ClientAdmissionPhase, Q3ClientAdmissionResult, Q3ClientBindings, Q3ClientConnection, Q3ClientMode,
    Q3ClientPacketResult, Q3ClientSendOptions, Q3ClientSendReadiness, Q3ConnectionIdentity, Q3NetError,
    Q3OutgoingDatagram, Q3Product, Snapshot,
};
use qa_net::q3_recording::q3_demo_gamestate;
use qa_world::session::SimulationOutput;
use thiserror::Error;

use super::types::{
    ApplicationNetwork, ApplicationNetworkError, ApplicationNetworkPhase, ApplicationNetworkRecording,
    ApplicationNetworkRole, NetworkPresentationEvent, Q3ConnectionCell,
};
use crate::bootstrap::demo_recording::{
    DemoRecordingError, DemoRecordingIdentity, DemoRecordingPacket, DemoRecordingSeed, DemoRecordingSink,
};

/// Default connection timeout in milliseconds (donor `120000`).
const DEFAULT_TIMEOUT_MILLISECONDS: u64 = 120_000;

/// Minimum milliseconds since the last packet before a server `disconnect`
/// takes effect (donor `3000`).
const DISCONNECT_QUIET_MILLISECONDS: u64 = 3_000;

/// Quake III client network failure.
#[derive(Debug, Error)]
pub enum Q3ClientError {
    /// Donor failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] Q3NetError),
    /// Message coding failure.
    #[error(transparent)]
    Msg(#[from] Q3MsgError),
    /// Transport failure.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// Recording failure.
    #[error(transparent)]
    Record(#[from] DemoRecordingError),
}

impl Q3ClientError {
    /// Map into the application network error.
    fn into_network(self) -> ApplicationNetworkError {
        ApplicationNetworkError::Message(self.to_string())
    }
}

/// Quake III application client host (donor `Q3ApplicationClientHost`).
pub trait Q3ApplicationClientHost {
    /// Apply system info (donor `systemInfo`).
    fn system_info(&mut self, info: &str);
    /// Apply a snapshot (donor `snapshot`).
    fn snapshot(&mut self, snapshot: &Snapshot, ping: i32);
    /// Restart the map (donor `mapRestart`).
    fn map_restart(&mut self);
    /// Print text (donor `print`).
    fn print(&mut self, text: &str);
    /// Publish a download size (donor `downloadSize`).
    fn download_size(&mut self, size: i32) -> i32;
    /// Receive a download block (donor `download`).
    fn download(&mut self, block: &DownloadBlock);
    /// Clear active state (donor `clearActive`).
    fn clear_active(&mut self);
    /// Apply a gamestate (donor `gamestate`).
    fn gamestate(&mut self, state: &Gamestate, generation: i32);
    /// Whether package downloads are in flight (donor `downloading`).
    fn downloading(&self) -> bool;
    /// Connection identity (donor `identity`).
    fn identity(&self) -> Q3ConnectionIdentity;
    /// Current userinfo (donor `userinfo`).
    fn userinfo(&self) -> String;
    /// Observe the admitted connection (donor `attach`).
    fn attach(&mut self, connection: &Q3ClientConnection<'_>);
    /// Convert an actor command (donor `command`).
    fn command(&mut self, command: &ActorCommand) -> WireUserCommand;
    /// Handle a disconnect (donor `disconnected`).
    fn disconnected(&mut self, reason: &str);
}

/// Client key authorization owner (donor
/// `Pick<Q3ClientAuthorization, 'request'>`).
pub trait Q3ClientAuthorizationOwner {
    /// Request WAN authorization (donor `request`).
    fn request(
        &mut self,
        assert_current: &mut dyn FnMut(),
        send: &mut dyn FnMut(&NetworkAddress, &[u8]),
    ) -> Result<(), Q3NetError>;
}

impl Q3ClientAuthorizationOwner for Q3ClientAuthorization<'_> {
    fn request(
        &mut self,
        assert_current: &mut dyn FnMut(),
        send: &mut dyn FnMut(&NetworkAddress, &[u8]),
    ) -> Result<(), Q3NetError> {
        Q3ClientAuthorization::request(self, assert_current, send)
    }
}

/// Deferred client binding callback (donor `Q3ClientBindings` methods).
#[derive(Debug, Clone, PartialEq)]
enum Q3ClientCallback {
    /// Print text.
    Print(String),
    /// Clear active state.
    ClearActive,
    /// Apply system info.
    SystemInfo(String),
    /// Apply a gamestate.
    Gamestate {
        /// Game state.
        state: Box<Gamestate>,
        /// Generation.
        generation: i32,
    },
    /// Apply a snapshot.
    Snapshot {
        /// Snapshot.
        snapshot: Box<Snapshot>,
        /// Ping.
        ping: i32,
    },
    /// Receive a download block.
    Download(DownloadBlock),
    /// Restart the map.
    MapRestart,
    /// Remote levelshot request, always rejected.
    LevelShot,
}

/// Attached recording sink with its detach identity.
struct Q3ClientRecordingOwner {
    /// Recording sink.
    sink: Box<dyn DemoRecordingSink>,
    /// Detach identity.
    id: u64,
}

/// Client state shared with the bindings adapter.
struct Q3ClientShared<H> {
    /// Application host.
    host: H,
    /// Connection phase.
    state: ApplicationNetworkPhase,
    /// Gamestate primed.
    primed: bool,
    /// Enter-world command sent.
    entered: bool,
    /// Live connection identity.
    connection_id: u64,
    /// Queued binding callbacks.
    events: Vec<Q3ClientCallback>,
    /// Host panic inside an inline query: truncate position plus reason.
    fatal_at: Option<(usize, String)>,
    /// Attached recording sink.
    recording: Option<Q3ClientRecordingOwner>,
    /// Detach identity counter.
    detach_counter: u64,
}

/// Bindings adapter leased to the connection (donor
/// `q3ApplicationClientBindings`).
struct Q3ClientAdapter<H> {
    /// Shared client state.
    shared: Rc<RefCell<Q3ClientShared<H>>>,
    /// Admitted connection identity.
    connection_id: u64,
}

impl<H: Q3ApplicationClientHost> Q3ClientBindings for Q3ClientAdapter<H> {
    fn assert_current(&mut self) {
        let shared = self.shared.borrow();
        if shared.connection_id != self.connection_id
            || matches!(
                shared.state,
                ApplicationNetworkPhase::Closed | ApplicationNetworkPhase::Rejected
            )
        {
            panic!("Q3 callback belongs to a retired connection");
        }
    }

    fn print(&mut self, text: &str) {
        self.shared
            .borrow_mut()
            .events
            .push(Q3ClientCallback::Print(text.to_string()));
    }

    fn clear_active(&mut self) {
        self.shared.borrow_mut().events.push(Q3ClientCallback::ClearActive);
    }

    fn system_info(&mut self, info: &str) {
        self.shared
            .borrow_mut()
            .events
            .push(Q3ClientCallback::SystemInfo(info.to_string()));
    }

    fn gamestate(&mut self, state: &Gamestate, generation: i32) {
        self.shared.borrow_mut().events.push(Q3ClientCallback::Gamestate {
            state: Box::new(state.clone()),
            generation,
        });
    }

    fn snapshot(&mut self, snapshot: &Snapshot, ping: i32) {
        self.shared.borrow_mut().events.push(Q3ClientCallback::Snapshot {
            snapshot: Box::new(snapshot.clone()),
            ping,
        });
    }

    fn download_size(&mut self, size: i32) -> i32 {
        // The only value-returning query: the connection needs the answer
        // inline, so the host runs now. A panic truncates the drain and
        // rejects, matching the donor's poll catch.
        let outcome = catch_unwind(AssertUnwindSafe(|| self.shared.borrow_mut().host.download_size(size)));
        match outcome {
            Ok(value) => value,
            Err(payload) => {
                let mut shared = self.shared.borrow_mut();
                let reason = panic_message(&payload);
                shared.fatal_at = Some((shared.events.len(), reason));
                0
            }
        }
    }

    fn download(&mut self, block: &DownloadBlock) {
        self.shared
            .borrow_mut()
            .events
            .push(Q3ClientCallback::Download(block.clone()));
    }

    fn map_restart(&mut self) {
        self.shared.borrow_mut().events.push(Q3ClientCallback::MapRestart);
    }

    fn level_shot(&mut self) {
        self.shared.borrow_mut().events.push(Q3ClientCallback::LevelShot);
    }

    fn local_server_running(&self) -> bool {
        false
    }
}

/// Best-effort panic payload text (donor `error.message`).
fn panic_message(payload: &Box<dyn Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        return text.to_string();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "Q3 host failure".to_string()
}

/// Channel delivery over the shared datagram transport (donor
/// `q3ChannelDelivery`).
struct Q3ClientDelivery<'t, T: DatagramTransport<Address = NetworkAddress>> {
    /// Shared transport.
    transport: &'t T,
    /// Current peer.
    remote: NetworkAddress,
    /// First send failure, reported after the transmit.
    error: Option<TransportError>,
    /// Trace sink.
    trace: Box<dyn FnMut(&str) + 't>,
}

impl<T: DatagramTransport<Address = NetworkAddress>> ChannelDelivery for Q3ClientDelivery<'_, T> {
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

/// Quake III client network options (donor `Q3ClientNetworkOptions`).
pub struct Q3ClientNetworkOptions<'c, T, H> {
    /// Shared datagram transport.
    pub transport: T,
    /// Remote server address.
    pub remote: NetworkAddress,
    /// Application host.
    pub host: H,
    /// Client qport.
    pub qport: u16,
    /// Shared cvar registry for packet tuning.
    pub cvars: Option<&'c CvarRegistry>,
    /// Connection timeout in milliseconds.
    pub timeout_milliseconds: Option<u64>,
    /// WAN authorization owner.
    pub authorization: Option<Box<dyn Q3ClientAuthorizationOwner>>,
}

/// Quake III client network (`Q3ClientNetwork`).
pub struct Q3ClientNetwork<'c, T: DatagramTransport<Address = NetworkAddress>, H> {
    transport: T,
    shared: Rc<RefCell<Q3ClientShared<H>>>,
    cell: Q3ConnectionCell<Q3ClientAdapter<H>, Q3ClientConnection<'static>>,
    admission: Q3ClientAdmission<'static>,
    peer: NetworkAddress,
    cvars: Option<&'c CvarRegistry>,
    authorization: Option<Box<dyn Q3ClientAuthorizationOwner>>,
    timeout_milliseconds: Option<u64>,
    last_received: u64,
    last_userinfo: String,
    last_now: u64,
}

impl<'c, T, H> Q3ClientNetwork<'c, T, H>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: Q3ApplicationClientHost + 'static,
{
    /// Build a client network.
    pub fn new(options: Q3ClientNetworkOptions<'c, T, H>) -> Self {
        let shared = Rc::new(RefCell::new(Q3ClientShared {
            host: options.host,
            state: ApplicationNetworkPhase::Connecting,
            primed: false,
            entered: false,
            connection_id: 0,
            events: Vec::new(),
            fatal_at: None,
            recording: None,
            detach_counter: 0,
        }));
        let print_shared = shared.clone();
        let mut admission = Q3ClientAdmission::new(options.qport, move |text| {
            print_shared.borrow_mut().host.print(text);
        });
        admission.begin(options.remote.clone());
        Self {
            transport: options.transport,
            shared: shared.clone(),
            cell: Q3ConnectionCell::new(Q3ClientAdapter {
                shared,
                connection_id: 0,
            }),
            admission,
            peer: options.remote,
            cvars: options.cvars,
            authorization: options.authorization,
            timeout_milliseconds: options.timeout_milliseconds,
            last_received: 0,
            last_userinfo: String::new(),
            last_now: 0,
        }
    }

    /// Borrow the native connection, when admitted (donor `native`).
    pub fn native(&self) -> Option<&Q3ClientConnection<'static>> {
        self.cell.connection()
    }

    /// Admission connect packet count (donor `connectPacketCount`).
    pub fn connect_packet_count(&self) -> i32 {
        self.admission.connect_packet_count
    }

    /// Queue a reliable command (donor `command`).
    pub fn command(&mut self, text: &str) -> Result<(), Q3ClientError> {
        let Some(connection) = self.cell.connection_mut() else {
            return Err(Q3ClientError::Message("Q3 client has no connection".to_string()));
        };
        connection.reliable.add(text)?;
        Ok(())
    }

    /// Transmit one packet immediately (donor `sendPacket`, with explicit time).
    pub fn send_packet(&mut self, now_milliseconds: u64) -> Result<(), Q3ClientError> {
        self.send(now_milliseconds.min(i32::MAX as u64) as i32)
    }

    /// Poll the transport and admission (donor `poll`).
    fn poll_inner(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, Q3ClientError> {
        self.last_now = now_milliseconds;
        {
            let state = self.shared.borrow().state;
            if matches!(
                state,
                ApplicationNetworkPhase::Closed | ApplicationNetworkPhase::Rejected
            ) {
                return Ok(Vec::new());
            }
        }
        let now = now_milliseconds.min(i32::MAX as u64) as i32;
        let userinfo = guard_pass(&self.shared, |host| host.userinfo())?;
        let request = self.admission.resend(now, &userinfo)?;
        if let Some(request) = request {
            if request.to.kind() != "loopback" {
                let peer = request.to.clone();
                self.send_resend(request, &peer)?;
            }
        }
        loop {
            let event = self.transport.poll()?;
            let Some(event) = event else {
                break;
            };
            let ReceiveEvent::Packet { from, payload, .. } = event else {
                continue;
            };
            if from.kind() != "ipv4" && from.kind() != "ipx" {
                continue;
            }
            let result = self.admission.receive(from.clone(), &payload, now)?;
            match result {
                Q3ClientAdmissionResult::Admitted {
                    address,
                    challenge,
                    qport,
                } => {
                    if address.kind() == "loopback" {
                        return Err(Q3ClientError::Message(
                            "Q3 remote application requires IPv4 or IPX".to_string(),
                        ));
                    }
                    self.admit(address, challenge, qport, now_milliseconds)?;
                }
                Q3ClientAdmissionResult::Sequenced { bytes } => {
                    if self.cell.connection().is_some() {
                        if let Err(error) = self.receive_sequenced(&bytes, now, now_milliseconds) {
                            let reason = error.to_string();
                            self.reject(&reason);
                            return Err(error);
                        }
                    }
                }
                Q3ClientAdmissionResult::Connectionless { packet } => {
                    if self.receive_connectionless(&packet, &from, now_milliseconds)? {
                        return Ok(Vec::new());
                    }
                }
                Q3ClientAdmissionResult::Handled | Q3ClientAdmissionResult::Ignored => {}
            }
        }
        if self.cell.connection().is_some() {
            let userinfo = guard_pass(&self.shared, |host| host.userinfo())?;
            if userinfo != self.last_userinfo {
                self.cell
                    .connection_mut()
                    .expect("connection checked above")
                    .reliable
                    .add(&format!("userinfo \"{userinfo}\""))?;
                self.last_userinfo = userinfo;
            }
            if now_milliseconds.saturating_sub(self.last_received)
                > self.timeout_milliseconds.unwrap_or(DEFAULT_TIMEOUT_MILLISECONDS)
            {
                self.reject("Q3 connection timed out");
                return Ok(Vec::new());
            }
            let primed = self.shared.borrow().primed;
            let entered = self.shared.borrow().entered;
            if primed && !entered {
                self.cell
                    .connection_mut()
                    .expect("connection checked above")
                    .commands
                    .append(&WireUserCommand::default());
                self.shared.borrow_mut().entered = true;
            }
            let downloading = guard_pass(&self.shared, |host| host.downloading())?;
            let state = self.shared.borrow().state;
            let primed = self.shared.borrow().primed;
            let readiness = Q3ClientSendReadiness {
                real_time: now,
                active: state == ApplicationNetworkPhase::Active,
                primed,
                cinematic: false,
                downloading,
                local: false,
                lan: q3_is_lan_address(&self.peer),
                maximum_packets: self.cvar("cl_maxpackets", 30),
            };
            let ready = self
                .cell
                .connection()
                .expect("connection checked above")
                .ready_to_send(&readiness)?;
            if ready {
                self.send(now)?;
            }
        }
        Ok(Vec::new())
    }

    /// Send an admission resend, authorizing WAN challenges first.
    fn send_resend(&mut self, request: Q3OutgoingDatagram, peer: &NetworkAddress) -> Result<(), Q3ClientError> {
        let lan = q3_is_lan_address(peer);
        if self.admission.phase == Q3ClientAdmissionPhase::Connecting && !lan {
            let Some(authorization) = self.authorization.as_mut() else {
                return Err(Q3ClientError::Message(
                    "Q3 WAN admission requires the client key authorization owner".to_string(),
                ));
            };
            // The guard runs synchronously with no interleaving state
            // change, so one evaluation covers every call.
            let current = {
                let state = self.shared.borrow().state;
                !matches!(
                    state,
                    ApplicationNetworkPhase::Closed | ApplicationNetworkPhase::Rejected
                ) && self.admission.phase == Q3ClientAdmissionPhase::Connecting
                    && self
                        .admission
                        .address
                        .as_ref()
                        .is_some_and(|address| same_address(address, peer, true))
            };
            let mut assert_current = move || {
                if !current {
                    panic!("Q3 authorization belongs to a retired connection");
                }
            };
            let transport = &self.transport;
            let mut send_error = None;
            {
                let mut send = |address: &NetworkAddress, payload: &[u8]| {
                    if send_error.is_none() {
                        if let Err(error) = transport.send(address, payload) {
                            send_error = Some(error);
                        }
                    }
                };
                authorization.request(&mut assert_current, &mut send)?;
            }
            assert_current();
            if let Some(error) = send_error {
                return Err(Q3ClientError::Transport(error));
            }
        }
        self.transport.send(peer, &request.payload)?;
        Ok(())
    }

    /// Build the admitted connection (donor `admitted` branch).
    fn admit(
        &mut self,
        address: NetworkAddress,
        challenge: i32,
        qport: u16,
        now_milliseconds: u64,
    ) -> Result<(), Q3ClientError> {
        self.peer = address;
        self.shared.borrow_mut().state = ApplicationNetworkPhase::Loading;
        self.last_received = now_milliseconds;
        let identity = guard_pass(&self.shared, |host| host.identity())?;
        self.shared.borrow_mut().connection_id += 1;
        let connection_id = self.shared.borrow().connection_id;
        self.cell.build(|adapter| {
            adapter.connection_id = connection_id;
            // SAFETY: the cell owns the adapter box exclusively, drops any
            // previous connection before this lease, and drops this
            // connection before reclaiming the box, so the lease outlives
            // the connection (see `Q3ConnectionCell`).
            let leased: &'static mut Q3ClientAdapter<H> = unsafe { &mut *(adapter as *mut Q3ClientAdapter<H>) };
            Q3ClientConnection::new(
                identity,
                Q3Product::Base,
                Q3ClientMode::Network { challenge, qport },
                leased,
            )
        });
        let connection = self.cell.connection().expect("connection just built");
        guard_pass(&self.shared, |host| host.attach(connection))?;
        Ok(())
    }

    /// Receive one sequenced datagram (donor `sequenced` branch).
    fn receive_sequenced(&mut self, bytes: &[u8], now: i32, now_milliseconds: u64) -> Result<(), Q3ClientError> {
        let result = {
            let connection = self.cell.connection_mut().expect("connection checked by caller");
            connection.receive_datagram(bytes, now)?
        };
        self.drain()?;
        if let Q3ClientPacketResult::Accepted {
            sequence, plaintext, ..
        } = &result
        {
            let waiting = self.cell.connection().is_some_and(|connection| connection.demo_waiting);
            if !waiting {
                let mut shared = self.shared.borrow_mut();
                if let Some(owner) = shared.recording.as_mut() {
                    owner.sink.append(&DemoRecordingPacket::Q3 {
                        sequence: *sequence,
                        message: plaintext.clone(),
                    })?;
                }
            }
        }
        self.last_received = now_milliseconds;
        Ok(())
    }

    /// Receive one connectionless packet; reports a closing disconnect
    /// (donor `connectionless` branch).
    fn receive_connectionless(
        &mut self,
        packet: &ConnectionlessPacket,
        from: &NetworkAddress,
        now_milliseconds: u64,
    ) -> Result<bool, Q3ClientError> {
        if packet.command == "print" {
            let mut text = packet.payload.clone();
            text.push(0);
            let mut reader = Q3MsgReader::new(&text, MessageMode::Oob)?;
            let line = reader.read_string()?;
            guard_pass(&self.shared, |host| host.print(&line))?;
        } else if packet.command == "disconnect"
            && self.cell.connection().is_some()
            && same_address(from, &self.peer, true)
            && now_milliseconds.saturating_sub(self.last_received) >= DISCONNECT_QUIET_MILLISECONDS
        {
            self.shared.borrow_mut().state = ApplicationNetworkPhase::Closed;
            self.shared.borrow_mut().host.disconnected("Server disconnected");
            return Ok(true);
        }
        Ok(false)
    }

    /// Transmit one packet (donor `send`).
    fn send(&mut self, now: i32) -> Result<(), Q3ClientError> {
        if self.cell.connection().is_none() {
            return Ok(());
        }
        let packet_dup = self.cvar("cl_packetdup", 1);
        let delivery_error = {
            let trace_shared = self.shared.clone();
            let mut delivery = Q3ClientDelivery {
                transport: &self.transport,
                remote: self.peer.clone(),
                error: None,
                trace: Box::new(move |text: &str| {
                    trace_shared.borrow_mut().host.print(text);
                }),
            };
            {
                let connection = self.cell.connection_mut().expect("connection checked above");
                connection.transmit(
                    Q3ClientSendOptions {
                        real_time: now,
                        packet_dup,
                        no_delta: false,
                    },
                    &mut delivery,
                )?;
            }
            delivery.error
        };
        if let Some(error) = delivery_error {
            return Err(Q3ClientError::Transport(error));
        }
        self.drain()?;
        Ok(())
    }

    /// Reject the connection with a host disconnect (donor poll `catch`).
    fn reject(&mut self, reason: &str) {
        let mut shared = self.shared.borrow_mut();
        shared.state = ApplicationNetworkPhase::Rejected;
        shared.host.disconnected(reason);
    }

    /// Read an integer cvar with a fallback.
    fn cvar(&self, name: &str, fallback: i32) -> i32 {
        self.cvars
            .and_then(|cvars| cvars.get(name))
            .map_or(fallback, |snapshot| snapshot.integer_value)
    }

    /// Drain queued binding callbacks in order (donor inline awaits).
    fn drain(&mut self) -> Result<(), Q3ClientError> {
        let events = std::mem::take(&mut self.shared.borrow_mut().events);
        let fatal = self.shared.borrow_mut().fatal_at.take();
        let mut events = events;
        if let Some((at, _)) = &fatal {
            events.truncate(*at);
        }
        for event in events {
            self.apply(event)?;
        }
        if let Some((_, reason)) = fatal {
            return Err(Q3ClientError::Message(reason));
        }
        Ok(())
    }

    /// Apply one deferred callback.
    fn apply(&mut self, event: Q3ClientCallback) -> Result<(), Q3ClientError> {
        match event {
            Q3ClientCallback::Print(text) => {
                self.guard_reject(|host| host.print(&text))?;
            }
            Q3ClientCallback::ClearActive => {
                self.guard_reject(|host| host.clear_active())?;
                let mut shared = self.shared.borrow_mut();
                shared.state = ApplicationNetworkPhase::Loading;
                shared.primed = false;
                shared.entered = false;
            }
            Q3ClientCallback::SystemInfo(info) => {
                self.guard_reject(|host| host.system_info(&info))?;
            }
            Q3ClientCallback::Gamestate { state, generation } => {
                self.guard_reject(|host| host.gamestate(&state, generation))?;
                let downloading = self.guard_reject(|host| host.downloading())?;
                self.shared.borrow_mut().primed = !downloading;
            }
            Q3ClientCallback::Snapshot { snapshot, ping } => {
                self.guard_reject(|host| host.snapshot(&snapshot, ping))?;
                let primed = self.shared.borrow().primed;
                if snapshot.flags & 2 == 0 && primed {
                    self.shared.borrow_mut().state = ApplicationNetworkPhase::Active;
                }
            }
            Q3ClientCallback::Download(block) => {
                self.guard_reject(|host| host.download(&block))?;
            }
            Q3ClientCallback::MapRestart => {
                self.guard_reject(|host| host.map_restart())?;
            }
            Q3ClientCallback::LevelShot => {
                return Err(Q3ClientError::Message(
                    "Remote server cannot request a local levelshot".to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Run a host call inside the drain, converting a panic into an error.
    /// Rejection stays with the poll `catch` (donor `catch` around
    /// `receiveDatagram`); drains after plain transmits propagate untouched,
    /// exactly like the donor's `send` outside `try`.
    fn guard_reject<V>(&mut self, call: impl FnOnce(&mut H) -> V) -> Result<V, Q3ClientError> {
        let outcome = catch_unwind(AssertUnwindSafe(|| call(&mut self.shared.borrow_mut().host)));
        match outcome {
            Ok(value) => Ok(value),
            Err(payload) => Err(Q3ClientError::Message(panic_message(&payload))),
        }
    }
}

/// Run a host call outside the reject domain: a panic becomes an error
/// with no state change (donor throws outside `try`).
fn guard_pass<H, T>(
    shared: &Rc<RefCell<Q3ClientShared<H>>>,
    call: impl FnOnce(&mut H) -> T,
) -> Result<T, Q3ClientError> {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let mut shared = shared.borrow_mut();
        call(&mut shared.host)
    }));
    match outcome {
        Ok(value) => Ok(value),
        Err(payload) => Err(Q3ClientError::Message(panic_message(&payload))),
    }
}

impl<'c, T, H> ApplicationNetworkRecording for Q3ClientNetwork<'c, T, H>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: Q3ApplicationClientHost + 'static,
{
    fn seed(&self) -> Result<DemoRecordingSeed, ApplicationNetworkError> {
        let failed = |message: &str| ApplicationNetworkError::Message(message.to_string());
        let shared = self.shared.borrow();
        let Some(connection) = self.cell.connection() else {
            return Err(failed("Recording requires an active Q3 connection"));
        };
        if shared.state != ApplicationNetworkPhase::Active {
            return Err(failed("Recording requires an active Q3 connection"));
        }
        let seed = q3_demo_gamestate(connection).map_err(|error| failed(&error.to_string()))?;
        Ok(DemoRecordingSeed {
            identity: DemoRecordingIdentity::Q3,
            packets: vec![DemoRecordingPacket::Q3 {
                sequence: seed.sequence,
                message: seed.message,
            }],
        })
    }

    fn attach(&mut self, sink: Box<dyn DemoRecordingSink>) -> Result<Box<dyn FnOnce() + '_>, ApplicationNetworkError> {
        let failed = |message: &str| ApplicationNetworkError::Message(message.to_string());
        if self.shared.borrow().recording.is_some() {
            return Err(failed("Q3 recording is already attached"));
        }
        let live = self.cell.connection().is_some() && self.shared.borrow().state == ApplicationNetworkPhase::Active;
        if !live {
            return Err(failed("Recording requires an active Q3 connection"));
        }
        self.cell
            .connection_mut()
            .expect("connection checked above")
            .demo_waiting = true;
        let mut shared = self.shared.borrow_mut();
        shared.detach_counter += 1;
        let id = shared.detach_counter;
        shared.recording = Some(Q3ClientRecordingOwner { sink, id });
        let cell = self.shared.clone();
        Ok(Box::new(move || {
            let matched = cell.borrow().recording.as_ref().is_some_and(|owner| owner.id == id);
            if matched {
                cell.borrow_mut().recording = None;
            }
        }))
    }
}

impl<'c, T, H> ApplicationNetwork for Q3ClientNetwork<'c, T, H>
where
    T: DatagramTransport<Address = NetworkAddress>,
    H: Q3ApplicationClientHost + 'static,
{
    fn recording(&mut self) -> Option<&mut dyn ApplicationNetworkRecording> {
        Some(self)
    }

    fn role(&self) -> ApplicationNetworkRole {
        ApplicationNetworkRole::Client
    }

    fn phase(&self) -> ApplicationNetworkPhase {
        self.shared.borrow().state
    }

    fn wire(&self) -> WireSelection {
        WireSelection::Source {
            protocol: ProtocolIdentity::Q3,
        }
    }

    fn poll(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, ApplicationNetworkError> {
        self.poll_inner(now_milliseconds).map_err(Q3ClientError::into_network)
    }

    fn submit(&mut self, commands: &[ActorCommand], _now_milliseconds: u64) -> Result<(), ApplicationNetworkError> {
        if self.shared.borrow().state != ApplicationNetworkPhase::Active || self.cell.connection().is_none() {
            return Ok(());
        }
        if commands.len() > 1 {
            return Err(ApplicationNetworkError::Message(
                "A native Q3 connection carries one player".to_string(),
            ));
        }
        for command in commands {
            let wire = guard_pass(&self.shared, |host| host.command(command)).map_err(Q3ClientError::into_network)?;
            self.cell
                .connection_mut()
                .expect("connection checked above")
                .commands
                .append(&wire);
        }
        Ok(())
    }

    fn publish(
        &mut self,
        _output: &SimulationOutput,
        _events: &[NetworkPresentationEvent],
        _now_milliseconds: u64,
    ) -> Result<(), ApplicationNetworkError> {
        Err(ApplicationNetworkError::Message(
            "Remote client cannot publish authoritative state".to_string(),
        ))
    }

    fn close(&mut self) {
        if self.transport.closed() {
            return;
        }
        let live = self.cell.connection().is_some()
            && !matches!(
                self.shared.borrow().state,
                ApplicationNetworkPhase::Closed | ApplicationNetworkPhase::Rejected
            );
        if live {
            let now = self.last_now.min(i32::MAX as u64) as i32;
            let packet_dup = self.cvar("cl_packetdup", 1);
            let trace_shared = self.shared.clone();
            let mut delivery = Q3ClientDelivery {
                transport: &self.transport,
                remote: self.peer.clone(),
                error: None,
                trace: Box::new(move |text: &str| {
                    trace_shared.borrow_mut().host.print(text);
                }),
            };
            let outcome = {
                let connection = self.cell.connection_mut().expect("connection checked above");
                connection.disconnect_packets(
                    Q3ClientSendOptions {
                        real_time: now,
                        packet_dup,
                        no_delta: false,
                    },
                    &mut delivery,
                )
            };
            if let Err(error) = outcome {
                self.shared.borrow_mut().host.print(&error.to_string());
            } else if let Some(error) = delivery.error {
                self.shared.borrow_mut().host.print(&error.to_string());
            }
        }
        self.shared.borrow_mut().state = ApplicationNetworkPhase::Closed;
        self.admission.disconnect();
        self.transport.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;
    use qa_core::identity::IdentityOwner;
    use qa_core::time::{FrameContext, FramePhase, SourceTime};
    use qa_net::common::commands::{CommandSource, UserCommand};
    use qa_net::q3_net::{
        encode_connectionless_text, GamestateEntry, Q3ServerBindings, Q3ServerConnection, Q3ServerRate,
        Q3ServerSnapshotHistory, Q3SnapshotEntities, ReliableCommand,
    };
    use qa_world::session::WorldSnapshot;
    use std::sync::Mutex;

    struct MockTransportInner {
        inbound: Vec<ReceiveEvent<NetworkAddress>>,
        sent: Vec<(NetworkAddress, Vec<u8>)>,
        closed: bool,
    }

    struct MockTransport {
        address: NetworkAddress,
        inner: Mutex<MockTransportInner>,
    }

    impl MockTransport {
        fn new(address: NetworkAddress) -> Self {
            Self {
                address,
                inner: Mutex::new(MockTransportInner {
                    inbound: Vec::new(),
                    sent: Vec::new(),
                    closed: false,
                }),
            }
        }

        fn feed(&self, from: NetworkAddress, payload: Vec<u8>) {
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

        fn sent(&self) -> Vec<(NetworkAddress, Vec<u8>)> {
            self.inner.lock().expect("transport").sent.clone()
        }

        fn take_sent(&self) -> Vec<(NetworkAddress, Vec<u8>)> {
            std::mem::take(&mut self.inner.lock().expect("transport").sent)
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
            if inner.inbound.is_empty() {
                return Ok(None);
            }
            Ok(Some(inner.inbound.remove(0)))
        }

        fn subscribe_readable(&self, _listener: std::sync::Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
            Ok(0)
        }

        fn unsubscribe(&self, _token: u64) {}

        fn close(&self) {
            self.inner.lock().expect("transport").closed = true;
        }
    }

    struct MockHost {
        identity: Q3ConnectionIdentity,
        userinfo: String,
        downloading: bool,
        prints: Vec<String>,
        system_infos: Vec<String>,
        gamestates: Vec<(usize, i32)>,
        snapshots: Vec<(i32, i32)>,
        downloads: Vec<String>,
        map_restarts: u32,
        clears: u32,
        attached: bool,
        disconnects: Vec<String>,
        panic_on_snapshot: bool,
    }

    impl MockHost {
        fn new() -> Self {
            let owner = IdentityOwner::create("q3-client-test").expect("owner");
            Self {
                identity: Q3ConnectionIdentity {
                    client: owner.client(0, 0),
                    seat: None,
                },
                userinfo: String::new(),
                downloading: false,
                prints: Vec::new(),
                system_infos: Vec::new(),
                gamestates: Vec::new(),
                snapshots: Vec::new(),
                downloads: Vec::new(),
                map_restarts: 0,
                clears: 0,
                attached: false,
                disconnects: Vec::new(),
                panic_on_snapshot: false,
            }
        }
    }

    impl Q3ApplicationClientHost for MockHost {
        fn system_info(&mut self, info: &str) {
            self.system_infos.push(info.to_string());
        }

        fn snapshot(&mut self, snapshot: &Snapshot, ping: i32) {
            if self.panic_on_snapshot {
                panic!("boom");
            }
            self.snapshots.push((snapshot.message_number, ping));
        }

        fn map_restart(&mut self) {
            self.map_restarts += 1;
        }

        fn print(&mut self, text: &str) {
            self.prints.push(text.to_string());
        }

        fn download_size(&mut self, size: i32) -> i32 {
            size
        }

        fn download(&mut self, block: &DownloadBlock) {
            self.downloads.push(format!("{block:?}"));
        }

        fn clear_active(&mut self) {
            self.clears += 1;
        }

        fn gamestate(&mut self, state: &Gamestate, generation: i32) {
            self.gamestates.push((state.entries.len(), generation));
        }

        fn downloading(&self) -> bool {
            self.downloading
        }

        fn identity(&self) -> Q3ConnectionIdentity {
            self.identity.clone()
        }

        fn userinfo(&self) -> String {
            self.userinfo.clone()
        }

        fn attach(&mut self, _connection: &Q3ClientConnection<'_>) {
            self.attached = true;
        }

        fn command(&mut self, command: &ActorCommand) -> WireUserCommand {
            WireUserCommand {
                server_time: command.sequence as i32,
                angles: [0; 3],
                moves: [0; 3],
                buttons: 0,
                weapon: 0,
            }
        }

        fn disconnected(&mut self, reason: &str) {
            self.disconnects.push(reason.to_string());
        }
    }

    struct MockAuth {
        calls: u32,
        authority: NetworkAddress,
    }

    impl Q3ClientAuthorizationOwner for MockAuth {
        fn request(
            &mut self,
            assert_current: &mut dyn FnMut(),
            send: &mut dyn FnMut(&NetworkAddress, &[u8]),
        ) -> Result<(), Q3NetError> {
            assert_current();
            self.calls += 1;
            let authority = self.authority.clone();
            send(&authority, b"key");
            assert_current();
            Ok(())
        }
    }

    struct MockSink {
        packets: Vec<DemoRecordingPacket>,
    }

    impl DemoRecordingSink for MockSink {
        fn append(&mut self, packet: &DemoRecordingPacket) -> Result<(), DemoRecordingError> {
            self.packets.push(packet.clone());
            Ok(())
        }
    }

    fn lan() -> NetworkAddress {
        NetworkAddress::Ipv4 {
            host: [127, 0, 0, 1],
            port: 27960,
        }
    }

    fn wan() -> NetworkAddress {
        NetworkAddress::Ipv4 {
            host: [8, 8, 8, 8],
            port: 27960,
        }
    }

    fn local() -> NetworkAddress {
        NetworkAddress::Ipv4 {
            host: [127, 0, 0, 1],
            port: 27961,
        }
    }

    /// Build a network whose transport stays reachable through `Arc`.
    fn client_pair(
        remote: NetworkAddress,
        host: MockHost,
        authorization: Option<MockAuth>,
    ) -> (
        std::sync::Arc<MockTransport>,
        Q3ClientNetwork<'static, SharedTransport, MockHost>,
    ) {
        let transport = std::sync::Arc::new(MockTransport::new(local()));
        let network = Q3ClientNetwork::new(Q3ClientNetworkOptions {
            transport: SharedTransport {
                inner: transport.clone(),
            },
            remote,
            host,
            qport: 27960,
            cvars: None,
            timeout_milliseconds: None,
            authorization: authorization.map(|auth| {
                let boxed: Box<dyn Q3ClientAuthorizationOwner> = Box::new(auth);
                boxed
            }),
        });
        (transport, network)
    }

    struct SharedTransport {
        inner: std::sync::Arc<MockTransport>,
    }

    impl DatagramTransport for SharedTransport {
        type Address = NetworkAddress;

        fn address(&self) -> NetworkAddress {
            self.inner.address()
        }

        fn closed(&self) -> bool {
            self.inner.closed()
        }

        fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
            self.inner.send(to, payload)
        }

        fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
            self.inner.poll()
        }

        fn subscribe_readable(&self, listener: std::sync::Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
            self.inner.subscribe_readable(listener)
        }

        fn unsubscribe(&self, token: u64) {
            self.inner.unsubscribe(token);
        }

        fn close(&self) {
            self.inner.close();
        }
    }

    fn oob(text: &str) -> Vec<u8> {
        encode_connectionless_text(text).expect("oob")
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

    #[test]
    fn wan_admission_requires_authorization_owner() {
        let (_transport, mut network) = client_pair(wan(), MockHost::new(), None);
        let error = network.poll_inner(0).expect_err("wan auth");
        assert_eq!(
            error.to_string(),
            "Q3 WAN admission requires the client key authorization owner"
        );
    }

    #[test]
    fn wan_admission_authorizes_then_challenges() {
        let authority = NetworkAddress::Ipv4 {
            host: [9, 9, 9, 9],
            port: 27950,
        };
        let (transport, mut network) = client_pair(
            wan(),
            MockHost::new(),
            Some(MockAuth {
                calls: 0,
                authority: authority.clone(),
            }),
        );
        network.poll_inner(0).expect("poll");
        let sent = transport.sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].0, authority);
        assert_eq!(sent[0].1, b"key");
        assert_eq!(sent[1].0, wan());
        assert!(sent[1].1.ends_with(b"getchallenge"));
    }

    #[test]
    fn lan_admission_skips_authorization() {
        let (transport, mut network) = client_pair(
            lan(),
            MockHost::new(),
            Some(MockAuth {
                calls: 0,
                authority: wan(),
            }),
        );
        network.poll_inner(0).expect("poll");
        let sent = transport.sent();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].1.ends_with(b"getchallenge"));
        assert_eq!(network.connect_packet_count(), 1);
    }

    #[test]
    fn loopback_remote_sends_nothing() {
        let remote = NetworkAddress::Loopback { id: "loop".to_string() };
        let (transport, mut network) = client_pair(remote, MockHost::new(), None);
        network.poll_inner(0).expect("poll");
        assert!(transport.sent().is_empty());
    }

    #[test]
    fn command_requires_connection() {
        let (_transport, mut network) = client_pair(lan(), MockHost::new(), None);
        let error = network.command("say hi").expect_err("no connection");
        assert_eq!(error.to_string(), "Q3 client has no connection");
        assert!(network.native().is_none());
    }

    #[test]
    fn recording_gates_require_active_connection() {
        let (_transport, mut network) = client_pair(lan(), MockHost::new(), None);
        let error = ApplicationNetworkRecording::seed(&network).expect_err("seed");
        assert_eq!(error.to_string(), "Recording requires an active Q3 connection");
        let Err(error) = ApplicationNetworkRecording::attach(&mut network, Box::new(MockSink { packets: Vec::new() }))
        else {
            panic!("attach gate");
        };
        assert_eq!(error.to_string(), "Recording requires an active Q3 connection");
    }

    #[test]
    fn submit_before_active_is_silent() {
        let (_transport, mut network) = client_pair(lan(), MockHost::new(), None);
        let owner = IdentityOwner::create("q3-submit").expect("owner");
        let command = ActorCommand {
            actor: owner.actor(0, 0),
            source: CommandSource::LocalSeat { seat: owner.seat(0) },
            sequence: 1,
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
        ApplicationNetwork::submit(&mut network, &[command], 0).expect("silent");
    }

    #[test]
    fn drain_applies_lifecycle_in_order() {
        let (_transport, mut network) = client_pair(lan(), MockHost::new(), None);
        {
            let mut shared = network.shared.borrow_mut();
            shared.events.push(Q3ClientCallback::ClearActive);
            shared.events.push(Q3ClientCallback::SystemInfo("\\a\\b".to_string()));
        }
        network.drain().expect("drain");
        // White-box access to the mock host through the shared handle.
        let shared = network.shared.borrow();
        assert_eq!(shared.host.clears, 1);
        assert_eq!(shared.host.system_infos, vec!["\\a\\b".to_string()]);
        assert_eq!(shared.state, ApplicationNetworkPhase::Loading);
        assert!(!shared.primed);
    }

    #[test]
    fn drain_rejects_levelshot() {
        let (_transport, mut network) = client_pair(lan(), MockHost::new(), None);
        network.shared.borrow_mut().events.push(Q3ClientCallback::LevelShot);
        let error = network.drain().expect_err("levelshot");
        assert_eq!(error.to_string(), "Remote server cannot request a local levelshot");
    }

    // A live qa-net server driving the client through admit, gamestate,
    // and snapshots.
    #[derive(Default)]
    struct ServerLog {
        commands: Vec<(ReliableCommand, bool)>,
        entered: Vec<WireUserCommand>,
    }

    struct TestServerBindings {
        server_id: i32,
        feed: i32,
        time: i32,
        log: Rc<RefCell<ServerLog>>,
    }

    impl Q3ServerBindings for TestServerBindings {
        fn assert_current(&mut self) {}
        fn server_id(&self) -> i32 {
            self.server_id
        }
        fn restarted_server_id(&self) -> i32 {
            self.server_id
        }
        fn checksum_feed(&self) -> i32 {
            self.feed
        }
        fn pure(&self) -> bool {
            false
        }
        fn debug_build(&self) -> bool {
            false
        }
        fn time(&self) -> i32 {
            self.time
        }
        fn client_running(&self) -> bool {
            false
        }
        fn flood_protect(&self) -> bool {
            false
        }
        fn download_name(&self) -> String {
            String::new()
        }
        fn command(&mut self, command: &ReliableCommand, client_ok: bool) -> Result<bool, Q3NetError> {
            self.log.borrow_mut().commands.push((command.clone(), client_ok));
            Ok(true)
        }
        fn enter_world(&mut self, command: &WireUserCommand) -> Result<(), Q3NetError> {
            self.log.borrow_mut().entered.push(command.clone());
            Ok(())
        }
        fn think(&mut self, _command: &WireUserCommand) -> Result<(), Q3NetError> {
            Ok(())
        }
        fn resend_gamestate(&mut self) -> Result<(), Q3NetError> {
            Ok(())
        }
        fn drop_client(&mut self, _reason: &str) -> Result<(), Q3NetError> {
            Ok(())
        }
        fn print(&mut self, _text: &str) {}
    }

    struct CaptureDelivery {
        packets: Vec<Vec<u8>>,
    }

    impl ChannelDelivery for CaptureDelivery {
        fn send(&mut self, datagram: &[u8]) {
            self.packets.push(datagram.to_vec());
        }

        fn trace(&mut self, _message: &str) {}
    }

    fn server_rate() -> Q3ServerRate {
        Q3ServerRate {
            rate: 25000,
            max_rate: 0,
            snapshot_msec: 50,
            local: true,
            force_lan: false,
            lan: false,
        }
    }

    fn gamestate(server_id: i32) -> Gamestate {
        Gamestate {
            command_sequence: 0,
            entries: vec![GamestateEntry::Configstring {
                index: 1,
                value: format!("\\sv_serverid\\{server_id}"),
            }],
            client_number: 0,
            checksum_feed: 0,
        }
    }

    /// Drive the client through challenge admission.
    fn admit_client(
        transport: &std::sync::Arc<MockTransport>,
        network: &mut Q3ClientNetwork<'static, SharedTransport, MockHost>,
        remote: &NetworkAddress,
        challenge: i32,
    ) {
        network.poll_inner(0).expect("challenge");
        transport.feed(remote.clone(), oob(&format!("challengeresponse {challenge}")));
        network.poll_inner(100).expect("challenged");
        network.poll_inner(3100).expect("connect");
        let sent = transport.take_sent();
        assert!(sent
            .iter()
            .any(|(_, payload)| payload.windows(7).any(|w| w == b"connect")));
        transport.feed(remote.clone(), oob("connectresponse"));
        network.poll_inner(3200).expect("admitted");
        assert!(network.native().is_some());
    }

    #[test]
    fn full_session_reaches_active_and_records() {
        let remote = lan();
        let mut host = MockHost::new();
        host.userinfo = "\\name\\t".to_string();
        let (transport, mut network) = client_pair(remote.clone(), host, None);
        let owner = IdentityOwner::create("q3-session").expect("owner");
        let identity = Q3ConnectionIdentity {
            client: owner.client(0, 0),
            seat: None,
        };
        let server_log = Rc::new(RefCell::new(ServerLog::default()));
        let mut bindings = TestServerBindings {
            server_id: 7,
            feed: 99,
            time: 4000,
            log: server_log.clone(),
        };
        let baseline = |_: i32| qa_net::q3_net::Q3EntityState::default();
        let mut snapshots = Q3ServerSnapshotHistory::new(
            Q3SnapshotEntities::new(1024).expect("entities"),
            Q3Product::Base,
            &baseline,
        );
        let mut server = Q3ServerConnection::new(identity, 1234, 27960, &mut snapshots, &mut bindings);
        let mut delivery = CaptureDelivery { packets: Vec::new() };
        admit_client(&transport, &mut network, &remote, 1234);
        // The admit poll already transmitted (3200ms elapsed); the server
        // observes it before the gamestate, as on the wire.
        for (_, payload) in transport.take_sent() {
            server.receive_datagram(&payload).expect("admit packet");
        }
        assert_eq!(network.shared.borrow().state, ApplicationNetworkPhase::Loading);
        assert!(network.shared.borrow().host.attached);
        // Userinfo synced into the reliable queue on admission.
        assert_eq!(network.native().expect("native").reliable.sequence(), 1);

        // The admit transmit carries server id 0, so the server drops it and
        // resends gamestate; the userinfo arrives with the next transmit.
        assert_eq!(server_log.borrow().commands.len(), 0);

        server
            .send_gamestate(&gamestate(7), server_rate(), &mut delivery)
            .expect("gamestate");
        assert_eq!(delivery.packets.len(), 1);
        transport.feed(remote.clone(), delivery.packets.pop().expect("packet"));
        network.poll_inner(3300).expect("gamestate poll");
        {
            let shared = network.shared.borrow();
            assert_eq!(shared.host.gamestates.len(), 1);
            assert!(shared.primed);
            assert_eq!(shared.state, ApplicationNetworkPhase::Loading);
        }
        // The primed client entered and transmitted to the server.
        let sent = transport.take_sent();
        assert_eq!(sent.len(), 1);
        server.receive_datagram(&sent[0].1).expect("client packet");
        assert_eq!(server.phase, qa_net::q3_net::Q3ServerPhase::Active);
        assert_eq!(server_log.borrow().commands.len(), 1);
        assert_eq!(server_log.borrow().entered.len(), 1);

        server
            .send_snapshot(0, server_rate(), &mut delivery, &mut |_| {})
            .expect("snapshot");
        transport.feed(remote.clone(), delivery.packets.pop().expect("snapshot packet"));
        network.poll_inner(3400).expect("snapshot poll");
        assert_eq!(network.shared.borrow().state, ApplicationNetworkPhase::Active);
        assert_eq!(network.shared.borrow().host.snapshots.len(), 1);
        assert!(!network.native().expect("native").demo_waiting);

        // Submit carries one player command.
        let command = ActorCommand {
            actor: owner.actor(0, 0),
            source: CommandSource::LocalSeat { seat: owner.seat(0) },
            sequence: 9,
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
        let before = network.native().expect("native").commands.current_number();
        ApplicationNetwork::submit(&mut network, &[command], 3400).expect("submit");
        assert_eq!(network.native().expect("native").commands.current_number(), before + 1);
        let owner2 = IdentityOwner::create("q3-submit-2").expect("owner");
        let second = ActorCommand {
            actor: owner2.actor(0, 0),
            source: CommandSource::LocalSeat { seat: owner2.seat(0) },
            sequence: 10,
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
        let first = ActorCommand {
            actor: owner.actor(0, 0),
            source: CommandSource::LocalSeat { seat: owner.seat(0) },
            sequence: 11,
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
        let error = ApplicationNetwork::submit(&mut network, &[first, second], 3400).expect_err("two players");
        assert_eq!(error.to_string(), "A native Q3 connection carries one player");

        // Recording seed, attach, detach, and live append.
        let seed = ApplicationNetworkRecording::seed(&network).expect("seed");
        assert_eq!(seed.identity, DemoRecordingIdentity::Q3);
        assert_eq!(seed.packets.len(), 1);
        // Attach then detach: the detach call ends its borrow and the
        // demo-waiting flag persists, as in the donor.
        ApplicationNetworkRecording::attach(&mut network, Box::new(MockSink { packets: Vec::new() })).expect("attach")(
        );
        assert!(network.native().expect("native").demo_waiting);
        let sink_packets = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        struct SharedSink {
            packets: std::rc::Rc<std::cell::RefCell<Vec<DemoRecordingPacket>>>,
        }
        impl DemoRecordingSink for SharedSink {
            fn append(&mut self, packet: &DemoRecordingPacket) -> Result<(), DemoRecordingError> {
                self.packets.borrow_mut().push(packet.clone());
                Ok(())
            }
        }
        // Dropping the detach closure without calling it leaves the sink
        // attached; a second attach then fails the donor gate.
        drop(
            ApplicationNetworkRecording::attach(
                &mut network,
                Box::new(SharedSink {
                    packets: sink_packets.clone(),
                }),
            )
            .expect("reattach"),
        );
        server
            .send_snapshot(0, server_rate(), &mut delivery, &mut |_| {})
            .expect("snapshot 2");
        transport.feed(remote.clone(), delivery.packets.pop().expect("packet 2"));
        network.poll_inner(3500).expect("append poll");
        let packets = sink_packets.borrow();
        assert_eq!(packets.len(), 1);
        assert!(matches!(packets[0], DemoRecordingPacket::Q3 { sequence: 3, .. }));
        drop(packets);

        // Remote clients cannot publish; close sends disconnects once.
        let output = test_output();
        let error = ApplicationNetwork::publish(&mut network, &output, &[], 3500).expect_err("publish");
        assert_eq!(error.to_string(), "Remote client cannot publish authoritative state");
        network.command("say hi").expect("command");
        let sent_before = transport.sent().len();
        ApplicationNetwork::close(&mut network);
        assert_eq!(network.shared.borrow().state, ApplicationNetworkPhase::Closed);
        assert!(transport.sent().len() > sent_before);
        assert!(transport.closed());
        let sent_after_close = transport.sent().len();
        ApplicationNetwork::close(&mut network);
        assert_eq!(transport.sent().len(), sent_after_close);
        assert!(network.poll_inner(999_999).expect("closed poll").is_empty());
    }

    #[test]
    fn connectionless_print_and_quiet_disconnect() {
        let remote = lan();
        let (transport, mut network) = client_pair(remote.clone(), MockHost::new(), None);
        admit_client(&transport, &mut network, &remote, 77);
        transport.feed(remote.clone(), oob("print\nHello there"));
        network.poll_inner(3300).expect("print poll");
        assert!(network
            .shared
            .borrow()
            .host
            .prints
            .iter()
            .any(|line| line.contains("Hello there")));
        // Early disconnects are ignored inside the quiet window.
        transport.feed(remote.clone(), oob("disconnect"));
        network.poll_inner(3400).expect("early disconnect");
        assert_eq!(network.shared.borrow().state, ApplicationNetworkPhase::Loading);
        // After 3000 quiet milliseconds the server disconnect closes.
        transport.feed(remote.clone(), oob("disconnect"));
        let commands = network.poll_inner(6400).expect("late disconnect");
        assert!(commands.is_empty());
        assert_eq!(network.shared.borrow().state, ApplicationNetworkPhase::Closed);
        assert_eq!(
            network.shared.borrow().host.disconnects,
            vec!["Server disconnected".to_string()]
        );
    }

    #[test]
    fn sequenced_garbage_is_rejected_without_error() {
        let remote = lan();
        let (transport, mut network) = client_pair(remote.clone(), MockHost::new(), None);
        admit_client(&transport, &mut network, &remote, 55);
        // Under four bytes the channel rejects without parsing.
        transport.feed(remote.clone(), vec![1, 2, 3]);
        network.poll_inner(3300).expect("garbage poll");
        assert_eq!(network.shared.borrow().state, ApplicationNetworkPhase::Loading);
    }

    #[test]
    fn host_panic_rejects_with_disconnect() {
        let remote = lan();
        let mut host = MockHost::new();
        host.panic_on_snapshot = true;
        let (transport, mut network) = client_pair(remote.clone(), host, None);
        admit_client(&transport, &mut network, &remote, 1234);
        let owner = IdentityOwner::create("q3-panic").expect("owner");
        let identity = Q3ConnectionIdentity {
            client: owner.client(0, 0),
            seat: None,
        };
        let mut bindings = TestServerBindings {
            server_id: 7,
            feed: 99,
            time: 4000,
            log: Rc::new(RefCell::new(ServerLog::default())),
        };
        let baseline = |_: i32| qa_net::q3_net::Q3EntityState::default();
        let mut snapshots = Q3ServerSnapshotHistory::new(
            Q3SnapshotEntities::new(1024).expect("entities"),
            Q3Product::Base,
            &baseline,
        );
        let mut server = Q3ServerConnection::new(identity, 1234, 27960, &mut snapshots, &mut bindings);
        let mut delivery = CaptureDelivery { packets: Vec::new() };
        server
            .send_gamestate(&gamestate(7), server_rate(), &mut delivery)
            .expect("gamestate");
        transport.feed(remote.clone(), delivery.packets.pop().expect("packet"));
        network.poll_inner(3300).expect("primed");
        server
            .send_snapshot(0, server_rate(), &mut delivery, &mut |_| {})
            .expect("snapshot");
        transport.feed(remote.clone(), delivery.packets.pop().expect("snapshot packet"));
        let error = network.poll_inner(3400).expect_err("panic");
        assert_eq!(error.to_string(), "boom");
        assert_eq!(network.shared.borrow().state, ApplicationNetworkPhase::Rejected);
        assert_eq!(network.shared.borrow().host.disconnects, vec!["boom".to_string()]);
    }

    #[test]
    fn idle_connection_times_out() {
        let remote = lan();
        let (transport, mut network) = client_pair(remote.clone(), MockHost::new(), None);
        admit_client(&transport, &mut network, &remote, 9);
        let commands = network.poll_inner(3200 + 120_001).expect("timeout poll");
        assert!(commands.is_empty());
        assert_eq!(network.shared.borrow().state, ApplicationNetworkPhase::Rejected);
        assert_eq!(
            network.shared.borrow().host.disconnects,
            vec!["Q3 connection timed out".to_string()]
        );
    }

    #[test]
    fn cvar_tuning_is_honored() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        cvars.register("cl_packetdup", "0", 0).expect("dup");
        cvars.register("cl_maxpackets", "15", 0).expect("max");
        let remote = lan();
        let transport = std::sync::Arc::new(MockTransport::new(local()));
        let mut network = Q3ClientNetwork::new(Q3ClientNetworkOptions {
            transport: SharedTransport {
                inner: transport.clone(),
            },
            remote: remote.clone(),
            host: MockHost::new(),
            qport: 27960,
            cvars: Some(&cvars),
            timeout_milliseconds: None,
            authorization: None,
        });
        network.poll_inner(0).expect("poll");
        assert_eq!(network.cvar("cl_packetdup", 1), 0);
        assert_eq!(network.cvar("cl_maxpackets", 30), 15);
        assert_eq!(network.cvar("missing", 7), 7);
    }
}
