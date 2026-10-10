//! Connected packet routing. Physical intake remains the platform event source.
use crate::channel::{Channel, Delivery};
use crate::commands::connection::Commands;
pub use qa_core::sys_events::Peer;
use qa_core::{
    events::{NativeReceipt, OutputConsumerId},
    loopback::Endpoint,
    primitives::ClientId,
    sys_events::EventTime,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub socket: u16,
    pub peer: Peer,
}
pub struct Connection {
    pub route: Route,
    pub channel: Channel,
    /// Old ACKs retain this consumer generation across client-slot reuse.
    pub output: Option<OutputConsumerId>,
    pub commands: Option<Commands>,
}
impl Connection {
    fn matches_socket(&self, socket: u16, from: std::net::SocketAddr, bytes: &[u8]) -> bool {
        let Peer::Socket(bound) = self.route.peer else {
            return false;
        };
        if self.route.socket != socket {
            return false;
        }
        if let Some(qport) = self.channel.routing_qport() {
            bound.ip() == from.ip()
                && self.channel.policy().format.server_qport(bytes) == Ok(Some(qport))
        } else {
            bound == from
        }
    }
    fn conflicts(&self, other: &Self) -> bool {
        if self.route.socket != other.route.socket {
            return false;
        }
        match (self.route.peer, other.route.peer) {
            (Peer::Socket(a), Peer::Socket(b)) => {
                match (self.channel.routing_qport(), other.channel.routing_qport()) {
                    (Some(a_port), Some(b_port)) => a.ip() == b.ip() && a_port == b_port,
                    _ => a == b,
                }
            }
            _ => self.route.peer == other.route.peer,
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
    Payload(&'a [u8]),
    ReliableCommand {
        sequence: u32,
        text: &'a [u8],
    },
    Snapshot(crate::snapshots::ReceivedFrame<'a>),
    PlayerInfo(crate::states::QwPlayerInfo),
    Print(crate::outputs::Print<'a>),
    Command {
        command: qa_core::primitives::UserCmd,
        output: Option<OutputConsumerId>,
    },
    Acknowledged {
        receipt: NativeReceipt,
        output: Option<OutputConsumerId>,
    },
}

pub struct Connections {
    clients: Box<[[Option<Connection>; 2]]>,
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
    pub frame_ns: u64,
}
fn index(endpoint: Endpoint) -> usize {
    match endpoint {
        Endpoint::Client => 0,
        Endpoint::Server => 1,
    }
}
impl Connections {
    pub fn load(clients: usize) -> Self {
        Self {
            clients: (0..clients).map(|_| [None, None]).collect(),
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
            frame_ns: 0,
        }
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
        if current[index(endpoint)].is_some() {
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
            .any(|c| c.conflicts(&connection))
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
        self.clients[slot][index(endpoint)] = Some(connection);
        Ok(())
    }
    pub fn get(&self, client: ClientId, endpoint: Endpoint) -> Option<&Connection> {
        self.clients.get(client.0 as usize)?[index(endpoint)].as_ref()
    }
    pub fn get_mut(&mut self, client: ClientId, endpoint: Endpoint) -> Option<&mut Connection> {
        self.clients.get_mut(client.0 as usize)?[index(endpoint)].as_mut()
    }
    pub fn unbind(&mut self, client: ClientId) {
        if let Some(endpoints) = self.clients.get_mut(client.0 as usize) {
            *endpoints = [None, None];
        }
    }
    pub fn receive(
        &mut self,
        socket: u16,
        from: impl Into<Peer>,
        bytes: &[u8],
        time: EventTime,
        mut consume: impl FnMut(ClientId, Endpoint, Incoming<'_>),
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
        let Some(connection) = &mut self.clients[client][endpoint] else {
            return;
        };
        if let (Peer::Socket(bound), Peer::Socket(received)) = (&mut connection.route.peer, from) {
            bound.set_port(received.port());
        }
        let endpoint = if endpoint == 0 {
            Endpoint::Client
        } else {
            Endpoint::Server
        };
        let client = ClientId(client as u32);
        match connection.channel.receive(bytes, time) {
            Ok(received) => {
                if let Delivery::Payload(payload) = received.delivery {
                    self.delivered += 1;
                    if !payload.is_empty()
                        && endpoint == Endpoint::Server
                        && let Some(commands) = &mut connection.commands
                    {
                        let sequence = received.header.sequence;
                        let length = commands.stage(payload);
                        let decoded = length.and_then(|length| {
                            commands.decode(
                                length,
                                sequence,
                                time.milliseconds() as i32,
                                self.frame_ns,
                                &mut connection.channel,
                            )
                        });
                        match decoded {
                            Ok(Some(command)) => {
                                self.commands += 1;
                                consume(
                                    client,
                                    endpoint,
                                    Incoming::Command {
                                        command,
                                        output: connection.output,
                                    },
                                );
                            }
                            Ok(None) => self.command_duplicates += 1,
                            Err(_) => self.command_errors += 1,
                        }
                    } else if !payload.is_empty()
                        && endpoint == Endpoint::Client
                        && let Some(commands) = &mut connection.commands
                        && matches!(
                            commands.protocol,
                            crate::commands::packet::Protocol::Quake3_68
                                | crate::commands::packet::Protocol::NetQuake15
                                | crate::commands::packet::Protocol::Quake2_34
                                | crate::commands::packet::Protocol::Quake2Repro1038
                                | crate::commands::packet::Protocol::QuakeWorld28
                        )
                    {
                        let sequence = received.header.sequence;
                        let length = commands.stage(payload);
                        match length.and_then(|length| {
                            commands.decode_output(
                                length,
                                sequence,
                                &mut connection.channel,
                                |incoming| consume(client, endpoint, incoming),
                            )
                        }) {
                            Err(_) => self.command_errors += 1,
                            Ok(Some(sequence)) => {
                                if let Some(frame) = connection.channel.snapshot(sequence) {
                                    consume(client, endpoint, Incoming::Snapshot(frame));
                                }
                            }
                            Ok(None) => {}
                        }
                    } else {
                        consume(client, endpoint, Incoming::Payload(payload));
                    }
                }
            }
            Err(_) => self.malformed += 1,
        }
        // A Q2 fragmented channel can accept an ACK before rejecting a fragment.
        // Consume receipts even on that native receive outcome.
        for &receipt in connection.channel.reliable_receipts() {
            self.acknowledged += 1;
            consume(
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
