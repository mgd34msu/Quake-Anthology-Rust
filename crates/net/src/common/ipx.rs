//! IPX packet framing and tunnel drivers ported from
//! `src/network/common/ipx.ts`, `ipx-host.ts`, and `ipx-dosbox.ts`.
//!
//! [`encode_ipx_packet`] emits Novell's checksum-disabled header form, as
//! used by Quake's IPX driver. Tunnels run over [`UdpTransport`](super::transport::UdpTransport);
//! DOSBox registration polls with sleeps instead of the donor's async waits.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use thiserror::Error;

use super::endpoint::{
    address_key, ipx_address, port_number, same_address, NetworkAddress,
};
use super::transport::{
    Clock, DatagramLimits, DatagramTransport, PacketQueue, ReceiveEvent, TransportError,
    UdpTransport, UNIFIED_DATAGRAM_LIMITS,
};

/// Error for IPX packet and tunnel failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IpxError {
    /// IPX packet fields are invalid.
    #[error("Invalid IPX packet")]
    BadPacket,
    /// IPX tunnel is closed.
    #[error("IPX tunnel is closed")]
    Closed,
    /// IPX payload exceeds tunnel capacity.
    #[error("IPX payload exceeds tunnel capacity")]
    Oversize,
    /// IPX socket is already bound or reserved.
    #[error("IPX socket is already bound or reserved")]
    SocketBusy,
    /// DOSBox IPX socket table is full.
    #[error("DOSBox IPX socket table is full")]
    TableFull,
    /// Invalid IPX packet type.
    #[error("Invalid IPX packet type")]
    BadPacketType,
    /// Invalid DOSBox registration timeout.
    #[error("Invalid DOSBox registration timeout")]
    BadTimeout,
    /// DOSBox IPX requires an IPv4 UDP socket.
    #[error("DOSBox IPX requires an IPv4 UDP socket")]
    RequiresIpv4,
    /// DOSBox IPX registration failed.
    #[error("DOSBox IPX registration {0}")]
    RegistrationFailed(String),
    /// DOSBox IPX network is closed.
    #[error("DOSBox IPX network is closed")]
    NetworkClosed,
    /// DOSBox IPX socket is closed.
    #[error("DOSBox IPX socket is closed")]
    SocketClosed,
    /// Packet exceeds DOSBox IPX capacity (1394 payload bytes).
    #[error("Packet exceeds DOSBox IPX capacity (1394 payload bytes)")]
    DosBoxOversize,
}

impl From<IpxError> for TransportError {
    fn from(error: IpxError) -> Self {
        TransportError::Closed(error.to_string())
    }
}

/// Decoded IPX packet (`IpxPacket`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpxPacket {
    /// Source address.
    pub from: NetworkAddress,
    /// Destination address.
    pub to: NetworkAddress,
    /// Packet type.
    pub packet_type: u8,
    /// Hop count.
    pub hops: u8,
    /// Payload.
    pub payload: Vec<u8>,
}

fn write_ipx_address(bytes: &mut [u8], offset: usize, address: &NetworkAddress) -> Result<(), IpxError> {
    let NetworkAddress::Ipx { network, node, port } = address else {
        return Err(IpxError::BadPacket);
    };
    bytes[offset..offset + 4].copy_from_slice(&network.to_be_bytes());
    bytes[offset + 4..offset + 10].copy_from_slice(node);
    bytes[offset + 10..offset + 12].copy_from_slice(&port.to_be_bytes());
    Ok(())
}

fn read_ipx_address(bytes: &[u8], offset: usize) -> Result<NetworkAddress, IpxError> {
    let network = u32::from_be_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]);
    let node = [
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
        bytes[offset + 8],
        bytes[offset + 9],
    ];
    let port = u16::from_be_bytes([bytes[offset + 10], bytes[offset + 11]]);
    ipx_address(network, node, u32::from(port)).map_err(|_| IpxError::BadPacket)
}

/// Encode an IPX packet (`encodeIpxPacket`).
pub fn encode_ipx_packet(packet: &IpxPacket) -> Result<Vec<u8>, IpxError> {
    if packet.payload.len() > 65505 {
        return Err(IpxError::BadPacket);
    }
    let mut bytes = vec![0u8; packet.payload.len() + 30];
    bytes[0..2].copy_from_slice(&65535u16.to_be_bytes());
    let length = bytes.len() as u16;
    bytes[2..4].copy_from_slice(&length.to_be_bytes());
    bytes[4] = packet.hops;
    bytes[5] = packet.packet_type;
    write_ipx_address(&mut bytes, 6, &packet.to)?;
    write_ipx_address(&mut bytes, 18, &packet.from)?;
    bytes[30..].copy_from_slice(&packet.payload);
    Ok(bytes)
}

/// Decode an IPX packet (`decodeIpxPacket`).
#[must_use]
pub fn decode_ipx_packet(bytes: &[u8]) -> Option<IpxPacket> {
    if bytes.len() < 30 || bytes.len() > 65535 {
        return None;
    }
    let checksum = u16::from_be_bytes([bytes[0], bytes[1]]);
    let length = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
    let to_port = u16::from_be_bytes([bytes[16], bytes[17]]);
    let from_port = u16::from_be_bytes([bytes[28], bytes[29]]);
    if checksum != 65535 || length != bytes.len() || to_port == 0 || from_port == 0 {
        return None;
    }
    Some(IpxPacket {
        from: read_ipx_address(bytes, 18).ok()?,
        to: read_ipx_address(bytes, 6).ok()?,
        packet_type: bytes[5],
        hops: bytes[4],
        payload: bytes[30..].to_vec(),
    })
}

/// IPX payload profile (`IpxPayloadProfile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpxPayloadProfile {
    /// Quake 1 sequencing (packet type 4, LE sequence prefix).
    Quake1,
    /// Raw datagrams with a fixed packet type.
    Datagram {
        /// Packet type.
        packet_type: u8,
    },
}

impl IpxPayloadProfile {
    fn packet_type(self) -> u8 {
        match self {
            Self::Quake1 => 4,
            Self::Datagram { packet_type } => packet_type,
        }
    }
}

struct IpxPeer {
    ipx: NetworkAddress,
    udp: NetworkAddress,
}

/// Explicit IPX-over-UDP tunnel (`IpxUdpTransport`).
pub struct IpxUdpTransport {
    address: NetworkAddress,
    transport: Arc<UdpTransport>,
    profile: IpxPayloadProfile,
    peers: Mutex<HashMap<String, IpxPeer>>,
    queue: PacketQueue<NetworkAddress>,
    subscription: Mutex<Option<u64>>,
    sequence: Mutex<u32>,
    ended: Mutex<bool>,
}

impl IpxUdpTransport {
    /// Wrap a UDP transport with an IPX address and payload profile.
    pub fn new(
        address: NetworkAddress,
        transport: Arc<UdpTransport>,
        profile: IpxPayloadProfile,
        now: Clock,
    ) -> Result<Arc<Self>, TransportError> {
        let queue = PacketQueue::new(
            DatagramLimits {
                max_bytes: UNIFIED_DATAGRAM_LIMITS.max_bytes - 34,
                queue_packets: 256,
            },
            now,
        )?;
        Ok(Arc::new(Self {
            address,
            transport,
            profile,
            peers: Mutex::new(HashMap::new()),
            queue,
            subscription: Mutex::new(None),
            sequence: Mutex::new(0),
            ended: Mutex::new(false),
        }))
    }

    fn is_ended(&self) -> bool {
        self.ended.lock().map(|ended| *ended).unwrap_or(true)
    }

    /// Register an IPX peer reachable at a UDP address (`addPeer`).
    pub fn add_peer(&self, ipx: NetworkAddress, udp: NetworkAddress) -> Result<(), IpxError> {
        if self.is_ended() {
            return Err(IpxError::Closed);
        }
        if let Ok(mut peers) = self.peers.lock() {
            peers.insert(address_key(&ipx, false), IpxPeer { ipx, udp });
        }
        Ok(())
    }

    /// Remove a peer (`removePeer`).
    pub fn remove_peer(&self, ipx: &NetworkAddress) {
        if let Ok(mut peers) = self.peers.lock() {
            peers.remove(&address_key(ipx, false));
        }
    }

    fn receive_packets(&self) {
        if self.is_ended() {
            return;
        }
        loop {
            let event = match self.transport.poll() {
                Ok(event) => event,
                Err(_) => return,
            };
            let Some(event) = event else { return };
            match event {
                ReceiveEvent::Error { error } => self.queue.push(ReceiveEvent::Error { error }),
                ReceiveEvent::Packet { from, payload, .. } => {
                    self.accept_udp(&from, &payload);
                }
                ReceiveEvent::Dropped { .. } => {}
            }
        }
    }

    fn accept_udp(&self, from: &NetworkAddress, payload: &[u8]) {
        let Some(packet) = decode_ipx_packet(payload) else {
            return;
        };
        let peers = match self.peers.lock() {
            Ok(peers) => peers,
            Err(_) => return,
        };
        let Some(peer) = peers.get(&address_key(&packet.from, false)) else {
            return;
        };
        if !same_address(&peer.udp, from, true) {
            return;
        }
        let is_broadcast = match (&packet.to, &self.address) {
            (
                NetworkAddress::Ipx {
                    network: to_network,
                    node: to_node,
                    ..
                },
                NetworkAddress::Ipx { network, .. },
            ) => to_node.iter().all(|byte| *byte == 255) && (*to_network == 0 || to_network == network),
            _ => false,
        };
        let port_matches = match (&packet.to, &self.address) {
            (
                NetworkAddress::Ipx { port: to_port, .. },
                NetworkAddress::Ipx { port, .. },
            ) => to_port == port,
            _ => false,
        };
        if !port_matches || (!is_broadcast && !same_address(&packet.to, &self.address, true)) {
            return;
        }
        if packet.packet_type != self.profile.packet_type() {
            return;
        }
        if self.profile == IpxPayloadProfile::Quake1 {
            if packet.payload.len() < 4 {
                return;
            }
            self.queue.accept(packet.from, &packet.payload[4..], false);
        } else {
            self.queue.accept(packet.from, &packet.payload, false);
        }
    }
}

impl DatagramTransport for IpxUdpTransport {
    type Address = NetworkAddress;

    fn address(&self) -> NetworkAddress {
        self.address.clone()
    }

    fn closed(&self) -> bool {
        self.is_ended()
    }

    fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
        if self.is_ended() {
            return Err(IpxError::Closed.into());
        }
        if payload.len() > self.queue.limits.max_bytes {
            return Err(IpxError::Oversize.into());
        }
        let mut bytes = payload.to_vec();
        if self.profile == IpxPayloadProfile::Quake1 {
            let mut prefixed = vec![0u8; payload.len() + 4];
            let sequence = self.sequence.lock().map(|mut sequence| {
                let current = *sequence;
                *sequence = current.wrapping_add(1);
                current
            });
            let Ok(sequence) = sequence else {
                return Err(IpxError::Closed.into());
            };
            prefixed[0..4].copy_from_slice(&sequence.to_le_bytes());
            prefixed[4..].copy_from_slice(payload);
            bytes = prefixed;
        }
        let packet = encode_ipx_packet(&IpxPacket {
            from: self.address.clone(),
            to: to.clone(),
            packet_type: self.profile.packet_type(),
            hops: 0,
            payload: bytes,
        })
        .map_err(|error| TransportError::Closed(error.to_string()))?;
        let is_broadcast = matches!(to, NetworkAddress::Ipx { node, .. } if node.iter().all(|byte| *byte == 255));
        if is_broadcast {
            let peers = self.peers.lock().map_err(|_| TransportError::Closed("IPX tunnel is closed".to_owned()))?;
            let mut sent = false;
            for peer in peers.values() {
                let same_network = match (to, &peer.ipx) {
                    (
                        NetworkAddress::Ipx { network: to_network, .. },
                        NetworkAddress::Ipx { network, .. },
                    ) => *to_network == 0 || to_network == network,
                    _ => false,
                };
                if same_network && self.transport.send(&peer.udp, &packet).unwrap_or(false) {
                    sent = true;
                }
            }
            return Ok(sent);
        }
        let peers = self.peers.lock().map_err(|_| TransportError::Closed("IPX tunnel is closed".to_owned()))?;
        let Some(peer) = peers.get(&address_key(to, false)) else {
            return Ok(false);
        };
        self.transport.send(&peer.udp, &packet)
    }

    fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
        if self.is_ended() {
            return Err(IpxError::Closed.into());
        }
        self.receive_packets();
        Ok(self.queue.poll())
    }

    fn subscribe_readable(
        &self,
        listener: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<u64, TransportError> {
        self.queue.subscribe(listener)
    }

    fn unsubscribe(&self, token: u64) {
        self.queue.unsubscribe(token);
    }

    fn close(&self) {
        let mut ended = match self.ended.lock() {
            Ok(ended) => ended,
            Err(_) => return,
        };
        if *ended {
            return;
        }
        *ended = true;
        drop(ended);
        if let Ok(mut subscription) = self.subscription.lock() {
            subscription.take();
        }
        if let Ok(mut peers) = self.peers.lock() {
            peers.clear();
        }
        self.queue.close();
        self.transport.close();
    }
}

/// Game payload selector (`IpxGame`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpxGame {
    /// Quake 1 sequencing.
    Quake1,
    /// Quake II datagrams.
    Quake2,
    /// Quake III datagrams.
    Quake3,
}

/// Raw AF_IPX socket capability (`NativeIpxCapability`).
pub enum NativeIpxCapability {
    /// Native IPX is unavailable.
    Unavailable {
        /// Reason.
        reason: String,
    },
    /// Native IPX bind callback owned by the host.
    Available(Box<dyn Fn(u16, u8) -> Result<IpxSocket, TransportError> + Send + Sync>),
}

/// Boxed IPX datagram socket.
pub type IpxSocket = Box<dyn DatagramTransport<Address = NetworkAddress>>;

/// IPX host selector (`IpxHost`).
pub enum IpxHost {
    /// Raw native socket capability.
    Native(NativeIpxCapability),
    /// DOSBox rendezvous network.
    DosBox(Arc<DosBoxIpxNetwork>),
}

/// Game payload contract over a raw IPX socket (`IpxGameTransport`).
pub struct IpxGameTransport {
    socket: IpxSocket,
    game: IpxGame,
    sequence: Mutex<u32>,
}

impl IpxGameTransport {
    /// Wrap a raw socket with a game payload contract.
    #[must_use]
    pub fn new(socket: IpxSocket, game: IpxGame) -> Self {
        Self {
            socket,
            game,
            sequence: Mutex::new(0),
        }
    }
}

impl DatagramTransport for IpxGameTransport {
    type Address = NetworkAddress;

    fn address(&self) -> NetworkAddress {
        self.socket.address()
    }

    fn closed(&self) -> bool {
        self.socket.closed()
    }

    fn max_datagram_bytes(&self) -> Option<usize> {
        Some(self.socket.max_datagram_bytes().unwrap_or(65507) - if self.game == IpxGame::Quake1 { 4 } else { 0 })
    }

    fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
        if self.game != IpxGame::Quake1 {
            return self.socket.send(to, payload);
        }
        let sequence = self
            .sequence
            .lock()
            .map(|mut sequence| {
                let current = *sequence;
                *sequence = current.wrapping_add(1);
                current
            })
            .map_err(|_| TransportError::Closed("IPX transport is closed".to_owned()))?;
        let mut packet = vec![0u8; payload.len() + 4];
        packet[0..4].copy_from_slice(&sequence.to_le_bytes());
        packet[4..].copy_from_slice(payload);
        self.socket.send(to, &packet)
    }

    fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
        loop {
            let event = self.socket.poll()?;
            if self.game != IpxGame::Quake1 {
                return Ok(event);
            }
            match event {
                Some(ReceiveEvent::Packet { from, payload, received_at }) if payload.len() >= 4 => {
                    return Ok(Some(ReceiveEvent::Packet {
                        from,
                        payload: payload[4..].to_vec(),
                        received_at,
                    }));
                }
                Some(ReceiveEvent::Packet { .. }) => {}
                other => return Ok(other),
            }
        }
    }

    fn subscribe_readable(
        &self,
        listener: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<u64, TransportError> {
        self.socket.subscribe_readable(listener)
    }

    fn unsubscribe(&self, token: u64) {
        self.socket.unsubscribe(token);
    }

    fn close(&self) {
        self.socket.close();
    }
}

/// Bind an IPX transport on a host (`bindIpxTransport`).
pub fn bind_ipx_transport(host: &IpxHost, game: IpxGame, port: u32) -> Result<IpxSocket, TransportError> {
    port_number(port, true).map_err(|_| TransportError::BadPort)?;
    let packet_type = if game == IpxGame::Quake1 { 4 } else { 0 };
    match host {
        IpxHost::DosBox(network) => {
            let socket = network
                .bind(port as u16, packet_type)
                .map_err(|error| TransportError::Closed(error.to_string()))?;
            Ok(Box::new(IpxGameTransport::new(Box::new(DosBoxSocketHandle { socket }), game)))
        }
        IpxHost::Native(NativeIpxCapability::Unavailable { reason }) => Err(TransportError::Closed(format!(
            "ipx-native: {reason}"
        ))),
        IpxHost::Native(NativeIpxCapability::Available(bind)) => {
            let socket = bind(port as u16, packet_type)?;
            Ok(Box::new(IpxGameTransport::new(socket, game)))
        }
    }
}

struct DosBoxSocketHandle {
    socket: Arc<DosBoxIpxSocket>,
}

impl DatagramTransport for DosBoxSocketHandle {
    type Address = NetworkAddress;

    fn address(&self) -> NetworkAddress {
        self.socket.address()
    }

    fn closed(&self) -> bool {
        self.socket.closed()
    }

    fn max_datagram_bytes(&self) -> Option<usize> {
        self.socket.max_datagram_bytes()
    }

    fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
        self.socket.send(to, payload)
    }

    fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
        self.socket.poll()
    }

    fn subscribe_readable(
        &self,
        listener: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<u64, TransportError> {
        self.socket.subscribe_readable(listener)
    }

    fn unsubscribe(&self, token: u64) {
        self.socket.unsubscribe(token);
    }

    fn close(&self) {
        self.socket.close();
    }
}

/// DOSBox IPX packet ceiling.
pub const DOSBOX_PACKET_BYTES: usize = 1424;
/// DOSBox IPX payload ceiling.
pub const DOSBOX_PAYLOAD_BYTES: usize = DOSBOX_PACKET_BYTES - 30;

fn dosbox_control() -> NetworkAddress {
    ipx_address(0, [0, 0, 0, 0, 0, 0], 2).unwrap_or(NetworkAddress::Ipx {
        network: 0,
        node: [0; 6],
        port: 2,
    })
}

fn is_broadcast(address: &NetworkAddress) -> bool {
    matches!(address, NetworkAddress::Ipx { node, .. } if node.iter().all(|byte| *byte == 255))
}

/// DOSBox IPX rendezvous network (`DosBoxIpxNetwork`).
pub struct DosBoxIpxNetwork {
    udp: Arc<UdpTransport>,
    server: NetworkAddress,
    address: NetworkAddress,
    now: Clock,
    sockets: Mutex<HashMap<u16, Weak<DosBoxIpxSocketInner>>>,
    ended: Mutex<bool>,
}

impl DosBoxIpxNetwork {
    /// Register with a DOSBox server, taking shared ownership of the UDP
    /// socket (`DosBoxIpxNetwork.connect`).
    pub fn connect(
        udp: Arc<UdpTransport>,
        server: NetworkAddress,
        timeout: Duration,
        now: Clock,
    ) -> Result<Arc<Self>, IpxError> {
        if !matches!(udp.address(), NetworkAddress::Ipv4 { .. }) {
            udp.close();
            return Err(IpxError::RequiresIpv4);
        }
        if timeout.is_zero() {
            udp.close();
            return Err(IpxError::BadTimeout);
        }
        let control = dosbox_control();
        let registration = encode_ipx_packet(&IpxPacket {
            from: control.clone(),
            to: control,
            packet_type: 0,
            hops: 0,
            payload: Vec::new(),
        })?;
        if !udp.send(&server, &registration).unwrap_or(false) {
            udp.close();
            return Err(IpxError::RegistrationFailed("send failed".to_owned()));
        }
        let deadline = Instant::now() + timeout;
        loop {
            if udp.closed() {
                udp.close();
                return Err(IpxError::RegistrationFailed("socket closed".to_owned()));
            }
            match udp.poll() {
                Ok(Some(ReceiveEvent::Packet { from, payload, .. })) => {
                    if !same_address(&from, &server, true) {
                        continue;
                    }
                    let Some(packet) = decode_ipx_packet(&payload) else {
                        continue;
                    };
                    let assigned = match (&packet.from, &packet.to) {
                        (
                            NetworkAddress::Ipx {
                                network: from_network,
                                port: from_port,
                                ..
                            },
                            NetworkAddress::Ipx { node: to_node, port: to_port, .. },
                        ) => {
                            packet.payload.is_empty()
                                && *from_port == 2
                                && *to_port == 2
                                && *from_network == 1
                                && !to_node.iter().all(|byte| *byte == 0)
                                && !is_broadcast(&packet.to)
                        }
                        _ => false,
                    };
                    if assigned {
                        return Ok(Arc::new(Self {
                            udp,
                            server,
                            address: packet.to,
                            now,
                            sockets: Mutex::new(HashMap::new()),
                            ended: Mutex::new(false),
                        }));
                    }
                }
                Ok(Some(ReceiveEvent::Error { error })) => {
                    udp.close();
                    return Err(IpxError::RegistrationFailed(error));
                }
                _ => {}
            }
            if Instant::now() >= deadline {
                udp.close();
                return Err(IpxError::RegistrationFailed("timed out".to_owned()));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Assigned IPX node address.
    #[must_use]
    pub fn address(&self) -> &NetworkAddress {
        &self.address
    }

    fn is_ended(&self) -> bool {
        self.ended.lock().map(|ended| *ended).unwrap_or(true)
    }

    /// Bind an application socket (`bind`).
    pub fn bind(self: &Arc<Self>, requested_port: u16, packet_type: u8) -> Result<Arc<DosBoxIpxSocket>, IpxError> {
        if self.is_ended() {
            return Err(IpxError::NetworkClosed);
        }
        let mut sockets = self.sockets.lock().map_err(|_| IpxError::NetworkClosed)?;
        if sockets.len() >= 150 {
            return Err(IpxError::TableFull);
        }
        let mut port = requested_port;
        if port == 0 {
            port = 0x4002;
            while sockets.contains_key(&port) && port < 0x7fff {
                port += 1;
            }
        }
        if port == 2 || sockets.contains_key(&port) {
            return Err(IpxError::SocketBusy);
        }
        let (network, node) = match &self.address {
            NetworkAddress::Ipx { network, node, .. } => (*network, *node),
            _ => return Err(IpxError::NetworkClosed),
        };
        let address = ipx_address(network, node, u32::from(port)).map_err(|_| IpxError::BadPacket)?;
        let queue = PacketQueue::new(
            DatagramLimits {
                max_bytes: DOSBOX_PAYLOAD_BYTES,
                queue_packets: 256,
            },
            self.now.clone(),
        )
        .map_err(|_| IpxError::NetworkClosed)?;
        let inner = Arc::new(DosBoxIpxSocketInner {
            address,
            network: Arc::downgrade(self),
            packet_type,
            queue,
            port,
            ended: std::sync::atomic::AtomicBool::new(false),
        });
        sockets.insert(port, Arc::downgrade(&inner));
        drop(sockets);
        let socket = Arc::new(DosBoxIpxSocket { inner });
        self.receive();
        Ok(socket)
    }

    fn send_packet(&self, packet: &IpxPacket) -> Result<bool, IpxError> {
        if self.is_ended() {
            return Err(IpxError::NetworkClosed);
        }
        if packet.payload.len() + 30 > DOSBOX_PACKET_BYTES {
            if let (Some(port), Ok(sockets)) = (from_port(&packet.from), self.sockets.lock()) {
                if let Some(socket) = sockets.get(&port).and_then(Weak::upgrade) {
                    socket.queue.push(ReceiveEvent::Error {
                        error: IpxError::DosBoxOversize.to_string(),
                    });
                }
            }
            return Ok(false);
        }
        let local = same_address(&packet.to, &self.address, false);
        let bytes = encode_ipx_packet(packet)?;
        let sent = local || self.udp.send(&self.server, &bytes).unwrap_or(false);
        if local || is_broadcast(&packet.to) {
            self.deliver(packet);
        }
        Ok(sent)
    }

    /// Pump the UDP socket into application sockets (`receive`).
    pub fn receive(&self) {
        if self.is_ended() {
            return;
        }
        if self.udp.closed() {
            self.close();
            return;
        }
        loop {
            if self.is_ended() {
                return;
            }
            let event = match self.udp.poll() {
                Ok(event) => event,
                Err(_) => return,
            };
            let Some(event) = event else { return };
            match event {
                ReceiveEvent::Error { error } => {
                    if let Ok(sockets) = self.sockets.lock() {
                        for socket in sockets.values().filter_map(Weak::upgrade) {
                            socket.queue.push(ReceiveEvent::Error { error: error.clone() });
                        }
                    }
                }
                ReceiveEvent::Packet { from, payload, .. } => {
                    if !same_address(&from, &self.server, true) || payload.len() > DOSBOX_PACKET_BYTES {
                        continue;
                    }
                    if let Some(packet) = decode_ipx_packet(&payload) {
                        self.deliver(&packet);
                    }
                }
                ReceiveEvent::Dropped { .. } => {}
            }
        }
    }

    fn deliver(&self, packet: &IpxPacket) {
        let local = same_address(&packet.to, &self.address, false);
        let network_broadcast = match (&packet.to, &self.address) {
            (
                NetworkAddress::Ipx {
                    network: to_network, ..
                },
                NetworkAddress::Ipx { network, .. },
            ) => is_broadcast(&packet.to) && (*to_network == 0 || to_network == network),
            _ => false,
        };
        if !local && !network_broadcast {
            return;
        }
        if to_port(&packet.to) == Some(2) {
            if is_broadcast(&packet.to)
                && from_port(&packet.from) == Some(2)
                && packet.payload.is_empty()
            {
                let reply = IpxPacket {
                    from: self.address.clone(),
                    to: packet.from.clone(),
                    packet_type: 0,
                    hops: 0,
                    payload: Vec::new(),
                };
                if let Ok(bytes) = encode_ipx_packet(&reply) {
                    let _ = self.udp.send(&self.server, &bytes);
                }
            }
            return;
        }
        if let (Some(port), Ok(sockets)) = (to_port(&packet.to), self.sockets.lock()) {
            if let Some(socket) = sockets.get(&port).and_then(Weak::upgrade) {
                socket.queue.accept(packet.from.clone(), &packet.payload, false);
            }
        }
    }

    /// Close the network and its sockets (`close`).
    pub fn close(&self) {
        let mut ended = match self.ended.lock() {
            Ok(ended) => ended,
            Err(_) => return,
        };
        if *ended {
            return;
        }
        *ended = true;
        drop(ended);
        let sockets: Vec<Weak<DosBoxIpxSocketInner>> = self
            .sockets
            .lock()
            .map(|sockets| sockets.values().cloned().collect())
            .unwrap_or_default();
        for socket in sockets {
            if let Some(socket) = socket.upgrade() {
                socket.queue.close();
            }
        }
        if let Ok(mut sockets) = self.sockets.lock() {
            sockets.clear();
        }
        self.udp.close();
    }
}

fn from_port(address: &NetworkAddress) -> Option<u16> {
    match address {
        NetworkAddress::Ipx { port, .. } => Some(*port),
        _ => None,
    }
}

fn to_port(address: &NetworkAddress) -> Option<u16> {
    from_port(address)
}

struct DosBoxIpxSocketInner {
    address: NetworkAddress,
    network: Weak<DosBoxIpxNetwork>,
    packet_type: u8,
    queue: PacketQueue<NetworkAddress>,
    port: u16,
    ended: std::sync::atomic::AtomicBool,
}

/// One DOSBox application socket.
pub struct DosBoxIpxSocket {
    inner: Arc<DosBoxIpxSocketInner>,
}

impl DosBoxIpxSocket {
    fn opened(&self) -> Result<Arc<DosBoxIpxNetwork>, TransportError> {
        if self.inner.ended.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(TransportError::Closed(IpxError::SocketClosed.to_string()));
        }
        let network = self
            .inner
            .network
            .upgrade()
            .ok_or_else(|| TransportError::Closed(IpxError::SocketClosed.to_string()))?;
        if network.is_ended() {
            return Err(TransportError::Closed(IpxError::SocketClosed.to_string()));
        }
        Ok(network)
    }
}

impl DatagramTransport for DosBoxIpxSocket {
    type Address = NetworkAddress;

    fn address(&self) -> NetworkAddress {
        self.inner.address.clone()
    }

    fn closed(&self) -> bool {
        self.inner.ended.load(std::sync::atomic::Ordering::Relaxed)
            || self.inner.network.upgrade().is_none()
    }

    fn max_datagram_bytes(&self) -> Option<usize> {
        Some(DOSBOX_PAYLOAD_BYTES)
    }

    fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
        let network = self.opened()?;
        network
            .send_packet(&IpxPacket {
                from: self.inner.address.clone(),
                to: to.clone(),
                packet_type: self.inner.packet_type,
                hops: 0,
                payload: payload.to_vec(),
            })
            .map_err(|error| TransportError::Closed(error.to_string()))
    }

    fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
        let network = self.opened()?;
        network.receive();
        Ok(self.inner.queue.poll())
    }

    fn subscribe_readable(
        &self,
        listener: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<u64, TransportError> {
        self.opened()?;
        self.inner.queue.subscribe(listener)
    }

    fn unsubscribe(&self, token: u64) {
        self.inner.queue.unsubscribe(token);
    }

    fn close(&self) {
        if self
            .inner
            .ended
            .swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        if let Some(network) = self.inner.network.upgrade() {
            if let Ok(mut sockets) = network.sockets.lock() {
                sockets.remove(&self.inner.port);
            }
        }
        self.inner.queue.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipx_packet_round_trips() {
        let from = ipx_address(1, [0, 0, 0, 0, 0, 1], 0x4002).unwrap();
        let to = ipx_address(1, [255, 255, 255, 255, 255, 255], 0x4002).unwrap();
        let packet = IpxPacket {
            from,
            to,
            packet_type: 4,
            hops: 0,
            payload: vec![1, 2, 3],
        };
        let bytes = encode_ipx_packet(&packet).unwrap();
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), 65535);
        let decoded = decode_ipx_packet(&bytes).unwrap();
        assert_eq!(decoded, packet);
        assert!(decode_ipx_packet(&bytes[..29]).is_none());
    }
}
