//! Connected packet routing. Physical intake remains the platform event source.
use crate::channel::{Channel, Delivery};
use crate::commands::{
    connection::{Commands, OutputError},
    packet::Protocol,
    startup::{
        NqControl, Q2_CONNECT_PACKET_LENGTH, Q2Connectionless, Q3_CONNECT_USERINFO_BYTES,
        Q3Connectionless, QwConnectionless, read_nq_control, read_q2_connectionless,
        read_q3_connectionless, read_qw_connectionless, write_nq_control, write_q2_connect,
        write_q2_getchallenge, write_q3_connect, write_q3_getchallenge, write_qw_connect,
        write_qw_getchallenge,
    },
};
pub use qa_core::sys_events::Peer;
use qa_core::{
    events::{NativeReceipt, OutputConsumerId},
    loopback::Endpoint,
    primitives::ClientId,
    sys_events::EventTime,
    text::FixedText,
};
use std::ops::ControlFlow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub socket: u16,
    pub peer: Peer,
}

#[derive(Clone, Debug)]
pub struct NativeConnect {
    submitted: u8,
    last_submission: Option<EventTime>,
    pending_challenge: Option<i32>,
    admitted_userinfo: Option<FixedText<Q3_CONNECT_USERINFO_BYTES>>,
}

/// An actual connect attempt borrows current userinfo and native packet policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectParameters {
    pub userinfo_length: usize,
    pub packet_length: i32,
}
impl ConnectParameters {
    pub const fn new(userinfo_length: usize) -> Self {
        Self {
            userinfo_length,
            packet_length: Q2_CONNECT_PACKET_LENGTH,
        }
    }
}
impl NativeConnect {
    pub const fn new() -> Self {
        Self {
            submitted: 0,
            last_submission: None,
            pending_challenge: None,
            admitted_userinfo: None,
        }
    }
}
impl PartialEq for NativeConnect {
    fn eq(&self, other: &Self) -> bool {
        self.submitted == other.submitted
            && self.last_submission == other.last_submission
            && self.pending_challenge == other.pending_challenge
            && self.admitted_userinfo.as_ref().map(FixedText::as_bytes)
                == other.admitted_userinfo.as_ref().map(FixedText::as_bytes)
    }
}
impl Eq for NativeConnect {}
impl Default for NativeConnect {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionPhase {
    Connecting(NativeConnect),
    Connected,
    Rejected,
}
impl ConnectionPhase {
    pub const fn is_connected(&self) -> bool {
        matches!(self, Self::Connected)
    }
}

pub struct Connection {
    pub phase: ConnectionPhase,
    pub route: Route,
    pub channel: Channel,
    /// Old ACKs retain this consumer generation across client-slot reuse.
    pub output: Option<OutputConsumerId>,
    /// Ring sequence carried by the channel's one prepared unreliable payload.
    /// Other SERVER senders must leave it for its output consumer to complete.
    pub pending_output: Option<u64>,
    pub commands: Option<Commands>,
}

fn accept_native_connect(
    connection: &mut Connection,
    client: ClientId,
    endpoint: Endpoint,
    consume: &mut impl FnMut(ClientId, Endpoint, Incoming<'_>) -> ControlFlow<()>,
) {
    if !matches!(&connection.phase, ConnectionPhase::Connecting(connect) if connect.admitted_userinfo.is_some())
    {
        return;
    }
    let Peer::Socket(address) = connection.route.peer else {
        return;
    };
    let previous = std::mem::replace(&mut connection.phase, ConnectionPhase::Connected);
    if let ConnectionPhase::Connecting(connect) = previous {
        let _ = consume(
            client,
            endpoint,
            Incoming::ConnectAccepted {
                port: address.port(),
                userinfo: connect.admitted_userinfo.as_ref().map(FixedText::as_bytes),
            },
        );
    }
}
fn q3_print(
    text: &[u8],
    client: ClientId,
    endpoint: Endpoint,
    consume: &mut impl FnMut(ClientId, Endpoint, Incoming<'_>) -> ControlFlow<()>,
) {
    let mut scratch = [0; crate::message::COMMAND_STRING_BYTES];
    let mut reader = crate::message::Reader::new(text, crate::message::Encoding::Bytes);
    if let Ok(text) =
        reader.read_string_into(crate::message::StringPolicy::Q3.allow_eof(), &mut scratch)
    {
        let _ = consume(
            client,
            endpoint,
            Incoming::Print(crate::outputs::Print {
                kind: qa_core::primitives::PrintKind::Console,
                level: None,
                text,
            }),
        );
    }
}

impl Connection {
    fn matches_socket(&self, socket: u16, from: std::net::SocketAddr, bytes: &[u8]) -> bool {
        let Peer::Socket(bound) = self.route.peer else {
            return false;
        };
        if self.route.socket != socket {
            return false;
        }
        if let ConnectionPhase::Connecting(connect) = &self.phase
            && self
                .commands
                .as_ref()
                .is_some_and(|commands| commands.protocol == Protocol::Quake3_68)
            && bytes.starts_with(&[255; 4])
        {
            // Stock68 challenge responses may redirect to a proxy. The owned
            // socket identifies the pending request until the first challenge.
            return if connect.pending_challenge.is_none() {
                true
            } else {
                bound.ip() == from.ip()
            };
        }
        if let Some(qport) = self.channel.routing_qport() {
            bound.ip() == from.ip()
                && self.channel.policy().format.server_qport(bytes) == Ok(Some(qport))
        } else {
            bound == from
        }
    }
    fn conflicts(&self, route: Route, qport: Option<u16>) -> bool {
        if self.route.socket != route.socket {
            return false;
        }
        match (self.route.peer, route.peer) {
            (Peer::Socket(a), Peer::Socket(b)) => match (self.channel.routing_qport(), qport) {
                (Some(a_port), Some(b_port)) => a.ip() == b.ip() && a_port == b_port,
                _ => a == b,
            },
            _ => self.route.peer == route.peer,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindError {
    Client,
    Occupied,
    Route,
    Capacity,
}
/// Borrowed delivery callbacks never retain a packet or create another queue.
pub enum Incoming<'a> {
    ConnectAccepted {
        port: u16,
        /// Exact last transport-admitted native connect info; borrowed for this call.
        userinfo: Option<&'a [u8]>,
    },
    ConnectRejected {
        reason: &'a [u8],
    },
    CommandString {
        protocol: crate::commands::packet::Protocol,
        sequence: Option<u32>,
        text: &'a [u8],
        output: Option<OutputConsumerId>,
    },
    Userinfo {
        protocol: crate::commands::packet::Protocol,
        text: &'a [u8],
        output: Option<OutputConsumerId>,
    },
    Disconnect {
        output: Option<OutputConsumerId>,
    },
    Payload(&'a [u8]),
    ReliableCommand {
        sequence: u32,
        text: &'a [u8],
    },
    Gamestate(crate::channel::commands::Q3GameState<'a>),
    ServerInfo(crate::commands::startup::NqServerInfo<'a>),
    QwServerData(crate::commands::startup::QwServerData<'a>),
    Q2ServerData(crate::commands::startup::Q2ServerData<'a>),
    /// Borrowed server text; execution policy belongs to the CLIENT consumer.
    StuffText(&'a [u8]),
    Reconnect,
    ResourceList(crate::outputs::ResourceListProgress),
    ResourceStream(crate::commands::startup::ResourceStream),
    MaxSpeed(f32),
    EntityGravity(f32),
    Snapshot(crate::snapshots::ReceivedFrame<'a>),
    /// A native sequenced ACK maps to the newest submitted common command.
    /// It is not an output-record ACK or a server execution-time assertion.
    CommandProgress(Option<u64>),
    /// One accepted QW packet observes the existing acknowledged move timestamp.
    PacketTiming {
        acknowledged_sent: Option<EventTime>,
    },
    Pause(bool),
    PlayerInfo(crate::states::QwPlayerInfo),
    Peer(crate::outputs::PeerUpdate<'a>),
    PlayerValue {
        index: u8,
        value: i32,
    },
    SetView {
        entity: u16,
    },
    /// Checked NQ control; progression belongs to the connection's consumer.
    SignonStage {
        stage: u8,
    },
    /// One fully decoded NQ fast update, before any following service.
    NqEntityUpdate {
        number: u16,
        model: Option<qa_core::primitives::ModelRef>,
    },
    SetAngles(qa_core::primitives::Vec3),
    CdTrack(crate::outputs::CdTrack),
    Print(crate::outputs::Print<'a>),
    Download(Result<crate::downloads::Chunk<'a>, crate::downloads::Error>),
    ConfigString(crate::outputs::ConfigString<'a>),
    StaticEntity(qa_core::primitives::EntityRecord),
    Particle(crate::outputs::Particle),
    Cue {
        cue: crate::outputs::Cue,
        current: Option<crate::snapshots::ReceivedFrame<'a>>,
    },
    /// Q3 authorizes this decoded move before old-command filtering changes
    /// its retained command time. This synchronous boundary retains no bytes.
    MoveStart {
        output: Option<OutputConsumerId>,
        first: qa_core::primitives::UserCmd,
        native_weapon: u8,
        /// A synchronous EnterWorld seeds lastUsercmd before filtering.
        entered: &'a mut bool,
    },
    /// One accepted QW move record, including all admitted loss recovery.
    MoveBegin,
    /// Completion is omitted if the synchronous consumer stops the batch.
    MoveEnd,
    Command {
        command: qa_core::primitives::UserCmd,
        /// Q3's byte ordinal is mapped through the registry before execution.
        native_weapon: Option<u8>,
        observation: Option<qa_core::primitives::SnapshotObservation>,
        execution: crate::commands::packet::CommandExecution,
        output: Option<OutputConsumerId>,
    },
    Acknowledged {
        receipt: NativeReceipt,
        output: Option<OutputConsumerId>,
    },
}

pub struct Connections {
    clients: Box<[[Option<Connection>; 2]]>,
    history_capacity: usize,
    connect_codec: Option<Box<crate::message::AdaptiveHuffman>>,
    pub packets: u64,
    pub bytes: u64,
    pub last_time: EventTime,
    pub last_from: Option<Peer>,
    pub last_socket: Option<u16>,
    pub unrouted: u64,
    pub ambiguous_routes: u64,
    pub malformed: u64,
    pub delivered: u64,
    pub acknowledged: u64,
    pub commands: u64,
    pub command_errors: u64,
    pub command_duplicates: u64,
}
impl Connections {
    pub fn load(clients: usize) -> Self {
        Self {
            clients: (0..clients).map(|_| [None, None]).collect(),
            history_capacity: qa_core::command_history::DEFAULT_CAPACITY,
            connect_codec: (clients != 0).then(|| Box::new(crate::message::AdaptiveHuffman::new())),
            packets: 0,
            bytes: 0,
            last_time: EventTime::default(),
            last_from: None,
            last_socket: None,
            unrouted: 0,
            ambiguous_routes: 0,
            malformed: 0,
            delivered: 0,
            acknowledged: 0,
            commands: 0,
            command_errors: 0,
            command_duplicates: 0,
        }
    }

    /// The common CLIENT histories are sized before frames/at connection load.
    pub fn set_history_capacity(&mut self, capacity: usize) -> Result<(), BindError> {
        if capacity == 0 {
            return Err(BindError::Capacity);
        }
        for endpoints in &mut self.clients {
            if let Some(connection) = &mut endpoints[Endpoint::Client.index()]
                && let Some(commands) = &mut connection.commands
            {
                commands
                    .configure_history(capacity)
                    .map_err(|_| BindError::Capacity)?;
            }
        }
        self.history_capacity = capacity;
        Ok(())
    }

    pub fn history_capacity(&self) -> usize {
        self.history_capacity
    }
    /// Handshake/load supplies the endpoint and negotiated Channel explicitly.
    /// Native address/qport keys are unique; an ACK cannot reach another client.
    pub fn bind(
        &mut self,
        client: ClientId,
        endpoint: Endpoint,
        mut connection: Connection,
    ) -> Result<(), BindError> {
        let slot = client.0 as usize;
        let current = self.clients.get(slot).ok_or(BindError::Client)?;
        if connection.channel.endpoint() != endpoint {
            return Err(BindError::Route);
        }
        if !connection.phase.is_connected()
            && (endpoint != Endpoint::Client
                || !matches!(connection.route.peer, Peer::Socket(_))
                || !connection.commands.as_ref().is_some_and(|commands| {
                    matches!(
                        commands.protocol,
                        Protocol::NetQuake15
                            | Protocol::QuakeWorld28
                            | Protocol::Quake2_34
                            | Protocol::Quake2Repro1038
                            | Protocol::Quake3_68
                    ) && connection.channel.policy()
                        == commands.protocol.client_channel(connection.channel.qport())
                }))
        {
            return Err(BindError::Route);
        }
        if current[endpoint.index()].is_some() {
            return Err(BindError::Occupied);
        }
        if let Peer::Loopback(id) = connection.route.peer
            && (id != client || connection.route.socket != endpoint.socket())
        {
            return Err(BindError::Route);
        }
        if self
            .clients
            .iter()
            .flatten()
            .flatten()
            .any(|c| c.conflicts(connection.route, connection.channel.routing_qport()))
        {
            return Err(BindError::Route);
        }
        if let Some(commands) = &connection.commands {
            connection
                .channel
                .configure_snapshots(commands.protocol)
                .map_err(|error| match error {
                    crate::commands::packet::Error::Context => BindError::Route,
                    _ => BindError::Capacity,
                })?;
        }
        if endpoint == Endpoint::Client
            && let Some(commands) = &mut connection.commands
        {
            commands
                .configure_history(self.history_capacity)
                .map_err(|_| BindError::Capacity)?;
        }
        self.clients[slot][endpoint.index()] = Some(connection);
        Ok(())
    }
    pub fn get(&self, client: ClientId, endpoint: Endpoint) -> Option<&Connection> {
        self.clients.get(client.0 as usize)?[endpoint.index()].as_ref()
    }
    pub fn get_mut(&mut self, client: ClientId, endpoint: Endpoint) -> Option<&mut Connection> {
        self.clients.get_mut(client.0 as usize)?[endpoint.index()].as_mut()
    }
    pub fn unbind(&mut self, client: ClientId) {
        if let Some(endpoints) = self.clients.get_mut(client.0 as usize) {
            *endpoints = [None, None];
        }
    }
    /// Regenerate due native requests from the existing row. Only transport
    /// admission commits attempt time or the exact pending userinfo receipt.
    pub fn send_connect_requests(
        &mut self,
        time: EventTime,
        mut userinfo: impl FnMut(ClientId, Protocol, Route, &mut [u8]) -> Option<ConnectParameters>,
        mut submit: impl FnMut(Route, &[u8]) -> bool,
        mut consume: impl FnMut(ClientId, Endpoint, Incoming<'_>) -> ControlFlow<()>,
    ) {
        for (slot, endpoints) in self.clients.iter_mut().enumerate() {
            let Some(connection) = &mut endpoints[Endpoint::Client.index()] else {
                continue;
            };
            let ConnectionPhase::Connecting(connect) = &mut connection.phase else {
                continue;
            };
            let Some(protocol) = connection
                .commands
                .as_ref()
                .map(|commands| commands.protocol)
            else {
                continue;
            };
            let interval = match protocol {
                Protocol::NetQuake15 => 2500,
                Protocol::QuakeWorld28 => 5000,
                Protocol::Quake2_34 | Protocol::Quake2Repro1038 => 3000,
                Protocol::Quake3_68 => 3000,
            };
            if connect
                .last_submission
                .is_some_and(|last| time.since(last) < EventTime::from_milliseconds(interval).0)
            {
                continue;
            }
            if protocol == Protocol::NetQuake15 && connect.submitted == 3 {
                connection.phase = ConnectionPhase::Rejected;
                if consume(
                    ClientId(slot as u32),
                    Endpoint::Client,
                    Incoming::ConnectRejected {
                        reason: b"No Response",
                    },
                )
                .is_break()
                {
                    return;
                }
                continue;
            }
            let mut bytes = [0; crate::commands::packet::Q3_MESSAGE_BYTES];
            let mut info = [0; Q3_CONNECT_USERINFO_BYTES];
            let mut admitted = None;
            let request = if protocol != Protocol::NetQuake15 {
                if let Some(challenge) = connect.pending_challenge {
                    let Some(capacity) = protocol.userinfo_bytes() else {
                        continue;
                    };
                    let Some(info) = info.get_mut(..capacity) else {
                        continue;
                    };
                    let Some(parameters) =
                        userinfo(ClientId(slot as u32), protocol, connection.route, info)
                            .filter(|parameters| parameters.userinfo_length < info.len())
                    else {
                        continue;
                    };
                    let mut receipt = FixedText::default();
                    let info = &info[..parameters.userinfo_length];
                    if receipt.set_bytes(info).is_err() {
                        continue;
                    }
                    admitted = Some(receipt);
                    if protocol == Protocol::QuakeWorld28 {
                        write_qw_connect(connection.channel.qport(), challenge, info, &mut bytes)
                    } else if protocol == Protocol::Quake3_68 {
                        self.connect_codec
                            .as_deref_mut()
                            .ok_or(crate::commands::packet::Error::Context)
                            .and_then(|codec| {
                                write_q3_connect(
                                    codec,
                                    connection.channel.qport(),
                                    challenge,
                                    info,
                                    &mut bytes,
                                )
                            })
                    } else {
                        write_q2_connect(
                            protocol,
                            connection.channel.qport(),
                            challenge,
                            info,
                            parameters.packet_length,
                            &mut bytes,
                        )
                    }
                } else if protocol == Protocol::QuakeWorld28 {
                    write_qw_getchallenge(&mut bytes)
                } else if protocol == Protocol::Quake3_68 {
                    write_q3_getchallenge(&mut bytes)
                } else {
                    write_q2_getchallenge(&mut bytes)
                }
            } else {
                write_nq_control(NqControl::Connect, &mut bytes)
            };
            let Ok(length) = request else {
                continue;
            };
            if submit(connection.route, &bytes[..length]) {
                if let Some(receipt) = admitted {
                    connect.admitted_userinfo = Some(receipt);
                    if !matches!(protocol, Protocol::Quake2Repro1038 | Protocol::Quake3_68) {
                        connect.pending_challenge = None;
                    }
                }
                connect.submitted = connect.submitted.saturating_add(1);
                connect.last_submission = Some(time);
            }
        }
    }
    pub fn receive(
        &mut self,
        socket: u16,
        from: impl Into<Peer>,
        bytes: &[u8],
        time: EventTime,
        mut consume: impl FnMut(ClientId, Endpoint, Incoming<'_>) -> ControlFlow<()>,
    ) {
        let from = from.into();
        self.packets = self.packets.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes.len() as u64);
        self.last_time = time;
        self.last_from = Some(from);
        self.last_socket = Some(socket);
        let route = Route { socket, peer: from };
        let located = match from {
            Peer::Loopback(client) => self.clients.get(client.0 as usize).and_then(|c| {
                c.iter()
                    .position(|c| c.as_ref().is_some_and(|c| c.route == route))
                    .map(|endpoint| (client.0 as usize, endpoint))
            }),
            Peer::Socket(address) => {
                let mut exact = None;
                let mut translated = None;
                let mut exact_count = 0;
                let mut translated_count = 0;
                for (client, endpoints) in self.clients.iter().enumerate() {
                    for (endpoint, connection) in endpoints.iter().enumerate() {
                        let Some(connection) = connection else {
                            continue;
                        };
                        if !connection.matches_socket(socket, address, bytes) {
                            continue;
                        }
                        if connection.route.peer == from {
                            exact = Some((client, endpoint));
                            exact_count += 1;
                        } else {
                            translated = Some((client, endpoint));
                            translated_count += 1;
                        }
                    }
                }
                match (exact_count, translated_count) {
                    (1, _) => exact,
                    (0, 1) => translated,
                    (0, 0) => None,
                    _ => {
                        self.ambiguous_routes += 1;
                        None
                    }
                }
            }
        };
        let Some((client, endpoint)) = located else {
            self.unrouted += 1;
            return;
        };
        let Some(connection) = &self.clients[client][endpoint] else {
            return;
        };
        match &connection.phase {
            ConnectionPhase::Rejected => return,
            ConnectionPhase::Connecting(connect) => {
                // A reply is eligible only after a request actually reached
                // transport. Its socket and full peer already matched above.
                if connect.submitted == 0 {
                    return;
                }
                if connection
                    .commands
                    .as_ref()
                    .is_some_and(|commands| commands.protocol == Protocol::Quake3_68)
                {
                    let reply = match read_q3_connectionless(bytes) {
                        Ok(reply) => reply,
                        Err(_) => {
                            self.malformed += 1;
                            return;
                        }
                    };
                    let Some(connection) = &mut self.clients[client][endpoint] else {
                        return;
                    };
                    match reply {
                        Q3Connectionless::Challenge(challenge) => {
                            if let ConnectionPhase::Connecting(connect) = &mut connection.phase
                                && connect.pending_challenge.is_none()
                            {
                                connection.route.peer = from;
                                connect.pending_challenge = Some(challenge);
                                connect.last_submission = None;
                            }
                        }
                        Q3Connectionless::Accepted => {
                            let challenge = match &connection.phase {
                                ConnectionPhase::Connecting(connect)
                                    if connect.admitted_userinfo.is_some() =>
                                {
                                    connect.pending_challenge
                                }
                                _ => None,
                            };
                            if let Some(challenge) = challenge {
                                let Ok(mut context) = connection.channel.command_context() else {
                                    return;
                                };
                                context.challenge = challenge as u32;
                                if connection.channel.set_command_context(context).is_err() {
                                    return;
                                }
                                connection.route.peer = from;
                                accept_native_connect(
                                    connection,
                                    ClientId(client as u32),
                                    Endpoint::from_index(endpoint),
                                    &mut consume,
                                );
                            }
                        }
                        Q3Connectionless::Print(text) => q3_print(
                            text,
                            ClientId(client as u32),
                            Endpoint::from_index(endpoint),
                            &mut consume,
                        ),
                        Q3Connectionless::Disconnect => {}
                    }
                    return;
                }
                if connection
                    .commands
                    .as_ref()
                    .is_some_and(|commands| commands.protocol == Protocol::QuakeWorld28)
                {
                    let reply = match read_qw_connectionless(bytes) {
                        Ok(reply) => reply,
                        Err(_) => {
                            self.malformed += 1;
                            return;
                        }
                    };
                    let Some(connection) = &mut self.clients[client][endpoint] else {
                        return;
                    };
                    match reply {
                        QwConnectionless::Challenge(challenge) => {
                            if let ConnectionPhase::Connecting(connect) = &mut connection.phase {
                                connect.pending_challenge = Some(challenge);
                                connect.last_submission = None;
                            }
                        }
                        QwConnectionless::Accepted => {
                            accept_native_connect(
                                connection,
                                ClientId(client as u32),
                                Endpoint::from_index(endpoint),
                                &mut consume,
                            );
                        }
                        QwConnectionless::Print(text) => {
                            let _ = consume(
                                ClientId(client as u32),
                                Endpoint::from_index(endpoint),
                                Incoming::Print(crate::outputs::Print {
                                    kind: qa_core::primitives::PrintKind::Console,
                                    level: None,
                                    text,
                                }),
                            );
                        }
                    }
                    return;
                }
                if let Some(protocol @ (Protocol::Quake2_34 | Protocol::Quake2Repro1038)) =
                    connection
                        .commands
                        .as_ref()
                        .map(|commands| commands.protocol)
                {
                    let reply = match read_q2_connectionless(protocol, bytes) {
                        Ok(reply) => reply,
                        Err(_) => {
                            self.malformed += 1;
                            return;
                        }
                    };
                    let Some(connection) = &mut self.clients[client][endpoint] else {
                        return;
                    };
                    match reply {
                        Q2Connectionless::Challenge(challenge) => {
                            if let ConnectionPhase::Connecting(connect) = &mut connection.phase {
                                if protocol == Protocol::Quake2Repro1038
                                    && connect.pending_challenge.is_some()
                                {
                                    return;
                                }
                                connect.pending_challenge = Some(challenge);
                                connect.last_submission = None;
                            }
                        }
                        Q2Connectionless::Accepted(_) => {
                            accept_native_connect(
                                connection,
                                ClientId(client as u32),
                                Endpoint::from_index(endpoint),
                                &mut consume,
                            );
                        }
                        Q2Connectionless::Print(text) => {
                            if protocol == Protocol::Quake2Repro1038
                                && let ConnectionPhase::Connecting(connect) = &mut connection.phase
                            {
                                connect.pending_challenge = None;
                                connect.admitted_userinfo = None;
                            }
                            let _ = consume(
                                ClientId(client as u32),
                                Endpoint::from_index(endpoint),
                                Incoming::Print(crate::outputs::Print {
                                    kind: qa_core::primitives::PrintKind::Console,
                                    level: None,
                                    text,
                                }),
                            );
                        }
                    }
                    return;
                }
                let control = match read_nq_control(bytes) {
                    Ok(control) => control,
                    Err(_) => {
                        self.malformed += 1;
                        return;
                    }
                };
                let incoming = match control {
                    NqControl::Accept { port } => {
                        let mut route = connection.route;
                        let Peer::Socket(address) = &mut route.peer else {
                            return;
                        };
                        address.set_port(port);
                        if self
                            .clients
                            .iter()
                            .enumerate()
                            .any(|(other_client, endpoints)| {
                                endpoints.iter().enumerate().any(|(other_endpoint, other)| {
                                    (other_client, other_endpoint) != (client, endpoint)
                                        && other.as_ref().is_some_and(|other| {
                                            other.conflicts(
                                                route,
                                                connection.channel.routing_qport(),
                                            )
                                        })
                                })
                            })
                        {
                            self.malformed += 1;
                            return;
                        }
                        let Some(connection) = &mut self.clients[client][endpoint] else {
                            return;
                        };
                        connection.route = route;
                        connection.phase = ConnectionPhase::Connected;
                        Incoming::ConnectAccepted {
                            port,
                            userinfo: None,
                        }
                    }
                    NqControl::Reject { reason } => {
                        let Some(connection) = &mut self.clients[client][endpoint] else {
                            return;
                        };
                        connection.phase = ConnectionPhase::Rejected;
                        Incoming::ConnectRejected { reason }
                    }
                    NqControl::Connect => {
                        self.malformed += 1;
                        return;
                    }
                };
                let _ = consume(
                    ClientId(client as u32),
                    Endpoint::from_index(endpoint),
                    incoming,
                );
                return;
            }
            ConnectionPhase::Connected => {}
        }
        let Some(connection) = &mut self.clients[client][endpoint] else {
            return;
        };
        if connection.channel.endpoint() == Endpoint::Client
            && connection
                .commands
                .as_ref()
                .is_some_and(|commands| commands.protocol == Protocol::Quake3_68)
            && bytes.starts_with(&[255; 4])
        {
            match read_q3_connectionless(bytes) {
                Ok(Q3Connectionless::Print(text)) => q3_print(
                    text,
                    ClientId(client as u32),
                    Endpoint::from_index(endpoint),
                    &mut consume,
                ),
                Ok(Q3Connectionless::Disconnect)
                    if time.since(connection.channel.state().last_received)
                        >= EventTime::from_milliseconds(3000).0 =>
                {
                    let _ = consume(
                        ClientId(client as u32),
                        Endpoint::from_index(endpoint),
                        Incoming::Disconnect {
                            output: connection.output,
                        },
                    );
                }
                _ => {}
            }
            return;
        }
        // Native connectionless replies have no sequenced ACK ownership.
        // Late challenge/accept/print packets cannot touch the connected channel.
        if connection.channel.endpoint() == Endpoint::Client
            && connection.commands.as_ref().is_some_and(|commands| {
                matches!(
                    commands.protocol,
                    Protocol::QuakeWorld28 | Protocol::Quake2_34 | Protocol::Quake2Repro1038
                )
            })
            && bytes.starts_with(&[255; 4])
        {
            return;
        }
        if let (Peer::Socket(bound), Peer::Socket(received)) = (&mut connection.route.peer, from) {
            bound.set_port(received.port());
        }
        let endpoint = Endpoint::from_index(endpoint);
        let client = ClientId(client as u32);
        match connection.channel.receive(bytes, time) {
            Ok(received) => {
                if let Delivery::Payload(payload) = received.delivery {
                    self.delivered += 1;
                    if endpoint == Endpoint::Server
                        && let Some(commands) = &mut connection.commands
                        && (!payload.is_empty()
                            || commands.protocol == crate::commands::packet::Protocol::QuakeWorld28)
                    {
                        let sequence = received.header.sequence;
                        let length = commands.stage(payload);
                        let output = connection.output;
                        let mut accepted = 0;
                        let mut delivered = false;
                        let decoded = length.and_then(|length| {
                            commands.decode(
                                length,
                                sequence,
                                time.milliseconds() as i32,
                                &mut connection.channel,
                                output,
                                |incoming| {
                                    delivered = true;
                                    accepted +=
                                        u64::from(matches!(incoming, Incoming::Command { .. }));
                                    consume(client, endpoint, incoming)
                                },
                            )
                        });
                        // Native parsing retains earlier effects if a later
                        // command is malformed, so count callback deliveries.
                        self.commands += accepted;
                        match decoded {
                            Ok(0) if !delivered => self.command_duplicates += 1,
                            Ok(_) => {}
                            Err(_) => self.command_errors += 1,
                        }
                    } else if endpoint == Endpoint::Client
                        && let Some(commands) = &mut connection.commands
                    {
                        let sequence = received.header.sequence;
                        let acknowledgement = received.header.acknowledgement;
                        let empty = payload.is_empty();
                        let length = commands.stage(payload);
                        if commands.protocol == crate::commands::packet::Protocol::QuakeWorld28 {
                            let _ = consume(
                                client,
                                endpoint,
                                Incoming::PacketTiming {
                                    acknowledged_sent: connection
                                        .channel
                                        .submitted_time(acknowledgement),
                                },
                            );
                        }
                        if empty {
                            let _ = consume(client, endpoint, Incoming::Payload(&[]));
                        } else {
                            let acknowledged = connection
                                .channel
                                .submitted_command(connection.channel.state().acknowledged)
                                .map(|command| command.number);
                            let packet_ack = matches!(
                                commands.protocol,
                                crate::commands::packet::Protocol::QuakeWorld28
                                    | crate::commands::packet::Protocol::Quake2_34
                                    | crate::commands::packet::Protocol::Quake2Repro1038
                            );
                            let mut deliver = |incoming: Incoming<'_>| {
                                if packet_ack
                                    && matches!(
                                        &incoming,
                                        Incoming::PlayerInfo(_) | Incoming::Snapshot(_)
                                    )
                                {
                                    let _ = consume(
                                        client,
                                        endpoint,
                                        Incoming::CommandProgress(acknowledged),
                                    );
                                }
                                let _ = consume(client, endpoint, incoming);
                            };
                            match length.map_err(OutputError::from).and_then(|length| {
                                commands.decode_output(
                                    length,
                                    sequence,
                                    &mut connection.channel,
                                    qa_core::primitives::ResourceNamespace::Client(client),
                                    &mut deliver,
                                )
                            }) {
                                Err(OutputError::RejectedDownload(_)) => self.command_errors += 1,
                                Err(_) => {
                                    self.command_errors += 1;
                                    deliver(Incoming::Disconnect {
                                        output: connection.output,
                                    });
                                }
                                Ok(Some(sequence)) => {
                                    if let Some(frame) = connection.channel.snapshot(sequence) {
                                        deliver(Incoming::Snapshot(frame));
                                    }
                                }
                                Ok(None) => {}
                            }
                        }
                    } else {
                        let _ = consume(client, endpoint, Incoming::Payload(payload));
                    }
                }
            }
            Err(_) => self.malformed += 1,
        }
        // A Q2 fragmented channel can accept an ACK before rejecting a fragment.
        // Consume receipts even on that native receive outcome.
        for &receipt in connection.channel.reliable_receipts() {
            self.acknowledged += 1;
            let _ = consume(
                client,
                endpoint,
                Incoming::Acknowledged {
                    receipt,
                    output: connection.output,
                },
            );
        }
    }
}
