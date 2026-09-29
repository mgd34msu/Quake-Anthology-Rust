//! KEX retail LAN lobby transport.
//!
//! Donor provenance: `KEX_LAN_PORT`, `KexLanOptions`, `KexLobbyPlayer`,
//! and `KexLanTransport` in `src/network/q2/kex/lan.ts`. The lobby owns
//! retail LAN sessions and presents only admitted game datagrams to Q2.
//! It reuses [`KexChannel`](crate::q2_kex_channel::KexChannel) peers,
//! [`PacketQueue`](crate::common::transport::PacketQueue) for receives,
//! and [`KexMdns`](crate::q2_kex_discovery::KexMdns) for host
//! advertisement; DNS resolution and the mDNS socket stay host-owned
//! (see [`KexMdnsConfig`]).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::common::endpoint::{address_key, same_address, NetworkAddress};
use crate::common::transport::{Clock, DatagramLimits, DatagramTransport, PacketQueue, ReceiveEvent, TransportError};
use crate::q2_kex_channel::{KexChannel, KexMode};
use crate::q2_kex_discovery::{KexMdns, KexMdnsSend};
use crate::q2_kex_packet::{kex_text, read_kex_text, write_kex_packet, KexError, KexPacket, KexReader, KexWriter};

/// KEX LAN port.
pub const KEX_LAN_PORT: u16 = 5069;
/// Maximum local players per endpoint.
pub const KEX_MAX_LOCAL_PLAYERS: u8 = 8;
/// Maximum lobby players.
pub const KEX_MAX_PLAYERS: usize = 255;
/// Maximum lobby peers.
const KEX_MAX_PEERS: usize = 256;
/// Join-request kind.
const JOIN_KIND: u8 = 128;
/// Lobby-query kind.
const QUERY_KIND: u8 = 129;
/// Player-roster kind.
const PLAYER_KIND: u8 = 253;
/// Disconnect kind.
const DISCONNECT_KIND: u8 = 254;
/// Lobby-attribute kind.
const ATTRIBUTE_KIND: u8 = 255;
/// Game-datagram kind.
const GAME_KIND: u8 = 0;
/// Compressed game-message kind (channel-level; game kinds are below it).
const COMPRESSED_KIND: u8 = 127;
/// Join-retry interval in milliseconds.
const JOIN_RETRY_MILLISECONDS: f64 = 500.0;

/// LAN role options (`KexLanOptions`).
#[derive(Debug, Clone)]
pub enum KexLanOptions {
    /// Host a lobby.
    Host {
        /// Player capacity (`localPlayers..=255`).
        max_players: u8,
        /// Local player count (`0..=8`).
        local_players: u8,
        /// Lobby name.
        name: String,
    },
    /// Join a lobby.
    Client {
        /// Server endpoint.
        server: NetworkAddress,
        /// Local player count (`1..=8`).
        local_players: u8,
    },
}

impl KexLanOptions {
    /// Local player count.
    fn local_players(&self) -> u8 {
        match self {
            Self::Host { local_players, .. } | Self::Client { local_players, .. } => *local_players,
        }
    }
}

/// Host-owned mDNS wiring for lobby advertisement.
///
/// The donor opens its own multicast socket; here the host provides the
/// socket's target inputs and send closure, and feeds inbound mDNS
/// datagrams to [`KexLanTransport::mdns_receive`].
pub struct KexMdnsConfig {
    /// Local hostname for the advertised target.
    pub hostname: String,
    /// Advertised IPv4 hosts.
    pub ipv4_hosts: Vec<[u8; 4]>,
    /// Multicast send closure.
    pub send: KexMdnsSend,
}

impl KexMdnsConfig {
    /// Build mDNS wiring.
    pub fn new(
        hostname: impl Into<String>,
        ipv4_hosts: Vec<[u8; 4]>,
        send: impl FnMut(&[u8]) -> Result<(), String> + Send + 'static,
    ) -> Self {
        Self {
            hostname: hostname.into(),
            ipv4_hosts,
            send: Box::new(send),
        }
    }
}

/// Lobby player (`KexLobbyPlayer`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KexLobbyPlayer {
    /// Player id.
    pub id: u64,
    /// Player attributes.
    pub attributes: HashMap<String, String>,
}

/// Lobby peer.
struct KexLanPeer {
    address: NetworkAddress,
    channel: KexChannel,
    players: Vec<u64>,
}

/// Lobby roster entry.
struct KexLanPlayer {
    id: u64,
    attributes: HashMap<String, String>,
}

/// Queued receive event: accepted datagrams enforce queue limits at flush.
enum OutboxEvent {
    Accept { from: NetworkAddress, payload: Vec<u8> },
    Push(ReceiveEvent<NetworkAddress>),
}

/// Mutable lobby state.
struct KexLanInner {
    peers: HashMap<String, KexLanPeer>,
    players: Vec<KexLanPlayer>,
    attributes: HashMap<String, String>,
    next_id: u64,
    retry_at: f64,
    joined: bool,
    ended: bool,
    discovery: Option<KexMdns>,
}

/// KEX LAN lobby transport (`KexLanTransport`).
///
/// Construct with [`open`](Self::open), which returns an `Arc` so the
/// inner transport's readable subscription can drain through a weak
/// reference.
pub struct KexLanTransport<T: DatagramTransport<Address = NetworkAddress>> {
    transport: Arc<T>,
    options: KexLanOptions,
    inner: Mutex<KexLanInner>,
    queue: PacketQueue<NetworkAddress>,
    clock: Arc<Mutex<f64>>,
    token: AtomicU64,
}

impl<T: DatagramTransport<Address = NetworkAddress> + 'static> KexLanTransport<T> {
    /// Open a lobby over an IP transport (`constructor`).
    pub fn open(transport: T, options: KexLanOptions, mdns: Option<KexMdnsConfig>) -> Result<Arc<Self>, KexError> {
        let transport = Arc::new(transport);
        if transport.address().kind() == "ipx" {
            return Err(KexError::Protocol("KEX LAN requires an IP transport"));
        }
        if options.local_players() > KEX_MAX_LOCAL_PLAYERS
            || (matches!(options, KexLanOptions::Client { .. }) && options.local_players() == 0)
        {
            return Err(KexError::Protocol("Invalid KEX local player count"));
        }
        if let KexLanOptions::Host {
            max_players,
            local_players,
            ..
        } = &options
        {
            if max_players < local_players {
                return Err(KexError::Protocol("Invalid KEX player capacity"));
            }
        }
        let clock = Arc::new(Mutex::new(0.0f64));
        let tick_clock = clock.clone();
        let tick_clock_fn: Clock = Arc::new(move || tick_clock.lock().map(|guard| *guard).unwrap_or(0.0));
        let queue = PacketQueue::new(
            DatagramLimits {
                max_bytes: 65535,
                queue_packets: 256,
            },
            tick_clock_fn,
        )?;
        let mut inner = KexLanInner {
            peers: HashMap::new(),
            players: Vec::new(),
            attributes: HashMap::new(),
            next_id: 1,
            retry_at: f64::NEG_INFINITY,
            joined: false,
            ended: false,
            discovery: None,
        };
        if let KexLanOptions::Host { local_players, .. } = &options {
            for _ in 0..*local_players {
                inner.players.push(KexLanPlayer {
                    id: inner.next_id,
                    attributes: HashMap::new(),
                });
                inner.next_id += 1;
            }
            inner.attributes.insert("ingame".to_owned(), "1".to_owned());
            inner.joined = true;
        }
        if let KexLanOptions::Client { server, .. } = &options {
            Self::peer_transport(&transport, &mut inner, server);
        }
        let this = Arc::new(Self {
            transport,
            options,
            inner: Mutex::new(inner),
            queue,
            clock,
            token: AtomicU64::new(0),
        });
        let weak = Arc::downgrade(&this);
        let token = this.transport.subscribe_readable(Arc::new(move || {
            if let Some(owner) = weak.upgrade() {
                owner.drain();
            }
        }))?;
        this.token.store(token, Ordering::Relaxed);
        if let (KexLanOptions::Host { .. }, Some(config)) = (&this.options, mdns) {
            if this.transport.address().is_ip() {
                let weak = Arc::downgrade(&this);
                let port = this.transport.address().port().unwrap_or(KEX_LAN_PORT);
                let discovery = KexMdns::new(
                    Some(port),
                    &config.hostname,
                    config.ipv4_hosts,
                    config.send,
                    |_| {},
                    // The queue drops pushes after close, so no ended check
                    // (and no inner lock, which would reenter) is needed.
                    move |error| {
                        if let Some(owner) = weak.upgrade() {
                            owner.queue.push(ReceiveEvent::Error { error });
                        }
                    },
                );
                if let Ok(mut inner) = this.inner.lock() {
                    inner.discovery = Some(discovery);
                }
            }
        }
        Ok(this)
    }

    /// Look up or create a peer (`peer`).
    fn peer_transport<'i>(
        transport: &Arc<T>,
        inner: &'i mut KexLanInner,
        address: &NetworkAddress,
    ) -> &'i mut KexLanPeer {
        inner.peers.entry(address_key(address, true)).or_insert_with(|| {
            let sender = transport.clone();
            let target = address.clone();
            KexLanPeer {
                address: address.clone(),
                channel: KexChannel::new(move |bytes| sender.send(&target, bytes).unwrap_or(false)),
                players: Vec::new(),
            }
        })
    }

    /// Current lobby clock in milliseconds.
    fn clock(&self) -> f64 {
        self.clock.lock().map(|guard| *guard).unwrap_or(0.0)
    }

    /// Whether the lobby is closed.
    fn is_ended(&self) -> bool {
        self.inner.lock().map(|inner| inner.ended).unwrap_or(true)
    }

    /// Whether an endpoint is admitted (`admitted`).
    pub fn admitted(&self, address: &NetworkAddress) -> bool {
        match self.inner.lock() {
            Ok(inner) => Self::is_admitted(&self.options, &inner, address),
            Err(_) => false,
        }
    }

    /// Admission over locked state.
    fn is_admitted(options: &KexLanOptions, inner: &KexLanInner, address: &NetworkAddress) -> bool {
        match options {
            KexLanOptions::Host { .. } => inner
                .peers
                .get(&address_key(address, true))
                .is_some_and(|peer| !peer.players.is_empty()),
            KexLanOptions::Client { server, .. } => inner.joined && same_address(address, server, true),
        }
    }

    /// Whether the lobby is joined and in game (`ready`).
    pub fn ready(&self) -> bool {
        self.inner
            .lock()
            .map(|inner| inner.joined && inner.attributes.get("ingame").is_some_and(|value| value == "1"))
            .unwrap_or(false)
    }

    /// Current lobby roster (`lobbyPlayers`).
    pub fn lobby_players(&self) -> Vec<KexLobbyPlayer> {
        self.inner
            .lock()
            .map(|inner| {
                inner
                    .players
                    .iter()
                    .map(|player| KexLobbyPlayer {
                        id: player.id,
                        attributes: player.attributes.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Advance timers, retries, and peer channels (`tick`).
    pub fn tick(&self, now: f64) -> Result<(), KexError> {
        if let Ok(mut clock) = self.clock.lock() {
            *clock = now;
        }
        self.drain();
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return Ok(()),
        };
        if let KexLanOptions::Client { server, local_players } = &self.options {
            if !inner.joined && now - inner.retry_at >= JOIN_RETRY_MILLISECONDS {
                inner.retry_at = now;
                let mut writer = KexWriter::new();
                writer.string("CRANTIME").string("QuakeII").byte(*local_players);
                let join = write_kex_packet(&KexPacket {
                    flags: 0,
                    sequence: 0,
                    reliable: 0,
                    kind: Some(JOIN_KIND),
                    payload: writer.finish(),
                })?;
                let _ = self.transport.send(server, &join);
            }
        }
        let keys: Vec<String> = inner.peers.keys().cloned().collect();
        let mut errors = Vec::new();
        for key in keys {
            let ticked = match inner.peers.get_mut(&key) {
                Some(peer) => peer.channel.tick(now),
                None => continue,
            };
            if let Err(error) = ticked {
                let message = error.to_string();
                if let Err(remove) = Self::remove(&self.options, &mut inner, &key, now) {
                    drop(inner);
                    for error in errors {
                        self.queue.push(ReceiveEvent::Error { error });
                    }
                    return Err(remove);
                }
                errors.push(message);
            }
        }
        drop(inner);
        for error in errors {
            self.queue.push(ReceiveEvent::Error { error });
        }
        Ok(())
    }

    /// Set a host lobby attribute (`setAttribute`).
    pub fn set_attribute(&self, key: &str, value: &str) -> Result<(), KexError> {
        if !matches!(self.options, KexLanOptions::Host { .. })
            || key.is_empty()
            || key.contains('\\')
            || key.contains('\0')
            || value.contains('\\')
            || value.contains('\0')
        {
            return Err(KexError::Protocol("Invalid KEX host attribute"));
        }
        let clock = self.clock();
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return Err(KexError::Protocol("KEX transport is closed")),
        };
        if value.is_empty() {
            inner.attributes.remove(key);
        } else {
            inner.attributes.insert(key.to_owned(), value.to_owned());
        }
        let text = kex_text(&format!("{key}\\{value}"));
        for peer in inner.peers.values_mut().filter(|peer| !peer.players.is_empty()) {
            peer.channel.send(ATTRIBUTE_KIND, &text, KexMode::Reliable, clock)?;
        }
        Ok(())
    }

    /// Feed an inbound mDNS datagram to host advertisement.
    pub fn mdns_receive(&self, bytes: &[u8]) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(discovery) = inner.discovery.as_mut() {
                discovery.receive(bytes);
            }
        }
    }

    /// Disconnect every peer and close (`close`).
    pub fn close(&self) {
        {
            let mut inner = match self.inner.lock() {
                Ok(inner) => inner,
                Err(_) => return,
            };
            if inner.ended {
                return;
            }
            let clock = self.clock();
            for peer in inner.peers.values_mut() {
                let mut writer = KexWriter::new();
                writer.string("Disconnected");
                let _ = peer
                    .channel
                    .send(DISCONNECT_KIND, &writer.finish(), KexMode::Unsequenced, clock);
            }
            inner.ended = true;
            if let Some(discovery) = inner.discovery.as_mut() {
                discovery.close();
            }
            inner.discovery = None;
            inner.peers.clear();
        }
        // Listener callbacks run without the inner lock so reentrant
        // lobby calls cannot deadlock.
        self.transport.unsubscribe(self.token.load(Ordering::Relaxed));
        self.queue.close();
        self.transport.close();
    }

    /// Drain the inner transport (`drain`).
    ///
    /// Malformed network input never enters the Q2 message stream: every
    /// per-event failure is swallowed like the donor's `catch`. The lock
    /// is only tried so a reentrant drain (a stacked transport's
    /// synchronous listener) skips instead of deadlocking; queue events
    /// flush after unlock so listener callbacks can reenter the lobby.
    fn drain(&self) {
        let mut inner = match self.inner.try_lock() {
            Ok(inner) => inner,
            Err(_) => return,
        };
        if inner.ended {
            return;
        }
        let clock = self.clock();
        let mut outbox = Vec::new();
        loop {
            let event = match self.transport.poll() {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(error) => {
                    outbox.push(OutboxEvent::Push(ReceiveEvent::Error {
                        error: error.to_string(),
                    }));
                    break;
                }
            };
            let ReceiveEvent::Packet { from, payload, .. } = event else {
                outbox.push(OutboxEvent::Push(event));
                continue;
            };
            if let KexLanOptions::Client { server, .. } = &self.options {
                if !same_address(&from, server, true) {
                    continue;
                }
            }
            // Unknown endpoints may only enter through an unsequenced lobby query/join.
            let key = address_key(&from, true);
            if !inner.peers.contains_key(&key) {
                if payload.len() < 3 || payload[1] % 16 != 0 || (payload[2] != JOIN_KIND && payload[2] != QUERY_KIND) {
                    continue;
                }
                if inner.peers.len() >= KEX_MAX_PEERS {
                    continue;
                }
                Self::peer_transport(&self.transport, &mut inner, &from);
            }
            let received = match inner.peers.get_mut(&key) {
                Some(peer) => peer.channel.receive(&payload, clock),
                None => continue,
            };
            let Ok(message) = received else {
                continue;
            };
            if let Some(message) = message {
                let _ = self.message(&mut inner, &key, message.kind, &message.payload, clock, &mut outbox);
            }
        }
        drop(inner);
        for event in outbox {
            match event {
                OutboxEvent::Accept { from, payload } => self.queue.accept(from, &payload, false),
                OutboxEvent::Push(event) => self.queue.push(event),
            }
        }
    }

    /// Handle a completed lobby message (`message`).
    fn message(
        &self,
        inner: &mut KexLanInner,
        key: &str,
        kind: u8,
        bytes: &[u8],
        clock: f64,
        outbox: &mut Vec<OutboxEvent>,
    ) -> Result<(), KexError> {
        if kind < COMPRESSED_KIND {
            let from = inner.peers.get(key).map(|peer| peer.address.clone());
            if let Some(from) = from {
                if Self::is_admitted(&self.options, inner, &from) {
                    outbox.push(OutboxEvent::Accept {
                        from,
                        payload: bytes.to_vec(),
                    });
                }
            }
            return Ok(());
        }
        if kind == JOIN_KIND {
            return Self::join(&self.options, inner, key, bytes, clock);
        }
        if kind == QUERY_KIND {
            if bytes.is_empty() {
                return Ok(());
            }
            let KexLanOptions::Host { name, max_players, .. } = &self.options else {
                return Ok(());
            };
            let mut reader = KexReader::new(bytes);
            if reader.string()? != "CRANTIME" || reader.string()? != "QuakeII" {
                return Ok(());
            }
            reader.end()?;
            let mut response = KexWriter::new();
            response
                .string(name)
                .integer(inner.players.len() as u64)
                .integer(u64::from(*max_players));
            for (attr, value) in &inner.attributes {
                if !value.is_empty() && !attr.starts_with('_') {
                    response.string(attr).string(value);
                }
            }
            if let Some(peer) = inner.peers.get(key) {
                let _ = self.transport.send(&peer.address, &response.finish());
            }
            return Ok(());
        }
        if kind == DISCONNECT_KIND {
            let mut reader = KexReader::new(bytes);
            let reason = reader.string()?;
            reader.end()?;
            Self::remove(&self.options, inner, key, clock)?;
            outbox.push(OutboxEvent::Push(ReceiveEvent::Error {
                error: format!("KEX LAN disconnected: {reason}"),
            }));
            return Ok(());
        }
        let from = inner.peers.get(key).map(|peer| peer.address.clone());
        let Some(from) = from else {
            return Ok(());
        };
        if !Self::is_admitted(&self.options, inner, &from) {
            return Ok(());
        }
        if kind == ATTRIBUTE_KIND && matches!(self.options, KexLanOptions::Client { .. }) {
            let text = read_kex_text(bytes)?;
            let Some(separator) = text.find('\\').filter(|separator| *separator > 0) else {
                return Err(KexError::Protocol("Invalid KEX lobby attribute"));
            };
            let (attr, value) = (&text[..separator], &text[separator + 1..]);
            if value.is_empty() {
                inner.attributes.remove(attr);
            } else {
                inner.attributes.insert(attr.to_owned(), value.to_owned());
            }
            return Ok(());
        }
        if kind == PLAYER_KIND {
            return Self::player_message(&self.options, inner, key, bytes, clock);
        }
        Ok(())
    }

    /// Handle a join request or response (`join`).
    fn join(
        options: &KexLanOptions,
        inner: &mut KexLanInner,
        key: &str,
        bytes: &[u8],
        clock: f64,
    ) -> Result<(), KexError> {
        let mut reader = KexReader::new(bytes);
        if reader.string()? != "CRANTIME" {
            return Ok(());
        }
        if let KexLanOptions::Host { max_players, .. } = options {
            if reader.string()? != "QuakeII" {
                return Ok(());
            }
            let count = reader.byte()?;
            reader.end()?;
            if !(1..=KEX_MAX_LOCAL_PLAYERS).contains(&count) {
                return Ok(());
            }
            let admitted = inner.peers.get(key).is_some_and(|peer| !peer.players.is_empty());
            if !admitted && inner.players.len() + usize::from(count) > usize::from(*max_players) {
                return Ok(());
            }
            if !admitted {
                let mut added = KexWriter::new();
                added.byte(1).integer(u64::from(count));
                for _ in 0..count {
                    let id = inner.next_id;
                    inner.next_id += 1;
                    if let Some(peer) = inner.peers.get_mut(key) {
                        peer.players.push(id);
                    }
                    inner.players.push(KexLanPlayer {
                        id,
                        attributes: HashMap::new(),
                    });
                    added.integer(id);
                }
                let added = added.finish();
                let targets: Vec<String> = inner
                    .peers
                    .iter()
                    .filter(|(candidate, peer)| *candidate != key && !peer.players.is_empty())
                    .map(|(candidate, _)| candidate.clone())
                    .collect();
                for target in targets {
                    if let Some(peer) = inner.peers.get_mut(&target) {
                        peer.channel.send(PLAYER_KIND, &added, KexMode::Reliable, clock)?;
                    }
                }
            }
            let first = inner
                .peers
                .get(key)
                .and_then(|peer| peer.players.first().copied())
                .and_then(|id| inner.players.iter().position(|player| player.id == id));
            let Some(first) = first else {
                return Err(KexError::Protocol("Invalid KEX integer"));
            };
            let mut response = KexWriter::new();
            response.string("CRANTIME").integer(first as u64);
            for player in &inner.players {
                response.integer(player.id);
            }
            let response = response.finish();
            let attributes: Vec<(String, String)> = inner
                .attributes
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            if let Some(peer) = inner.peers.get_mut(key) {
                peer.channel.send(JOIN_KIND, &response, KexMode::Unsequenced, clock)?;
                for (attr, value) in &attributes {
                    peer.channel.send(
                        ATTRIBUTE_KIND,
                        &kex_text(&format!("{attr}\\{value}")),
                        KexMode::Reliable,
                        clock,
                    )?;
                }
            }
            return Ok(());
        }
        if inner.joined {
            return Ok(());
        }
        let first = reader.integer()?;
        if first > 255 {
            return Err(KexError::Protocol("Invalid KEX local player index"));
        }
        let KexLanOptions::Client { local_players, .. } = options else {
            return Ok(());
        };
        let count = first as usize + usize::from(*local_players);
        let mut players = Vec::with_capacity(count);
        for _ in 0..count {
            players.push(KexLanPlayer {
                id: reader.integer()?,
                attributes: HashMap::new(),
            });
        }
        reader.end()?;
        inner.players = players;
        inner.joined = true;
        Ok(())
    }

    /// Handle a player roster message (`playerMessage`).
    fn player_message(
        options: &KexLanOptions,
        inner: &mut KexLanInner,
        key: &str,
        bytes: &[u8],
        clock: f64,
    ) -> Result<(), KexError> {
        let mut reader = KexReader::new(bytes);
        let operation = reader.byte()?;
        if matches!(options, KexLanOptions::Host { .. }) {
            let slot = inner
                .peers
                .get(key)
                .and_then(|peer| peer.players.get(usize::from(operation)).copied())
                .and_then(|id| {
                    inner
                        .players
                        .iter()
                        .position(|player| player.id == id)
                        .map(|index| (id, index))
                });
            let Some((_, index)) = slot else {
                return Ok(());
            };
            let attribute = reader.string()?;
            reader.end()?;
            Self::player_attribute(&mut inner.players, index, &attribute)?;
            let mut output = KexWriter::new();
            output.byte(0).integer(index as u64).string(&attribute);
            let output = output.finish();
            let targets: Vec<String> = inner
                .peers
                .iter()
                .filter(|(_, peer)| !peer.players.is_empty())
                .map(|(candidate, _)| candidate.clone())
                .collect();
            for target in targets {
                if let Some(peer) = inner.peers.get_mut(&target) {
                    peer.channel.send(PLAYER_KIND, &output, KexMode::Reliable, clock)?;
                }
            }
            return Ok(());
        }
        let index = reader.integer()?;
        if index > 255 {
            return Err(KexError::Protocol("Invalid KEX player index"));
        }
        match operation {
            0 => {
                let attribute = reader.string()?;
                Self::player_attribute(&mut inner.players, index as usize, &attribute)?;
            }
            1 => {
                let count = index as usize;
                if inner.players.len() + count > KEX_MAX_PLAYERS {
                    return Err(KexError::Protocol("KEX player roster exceeds capacity"));
                }
                for _ in 0..count {
                    inner.players.push(KexLanPlayer {
                        id: reader.integer()?,
                        attributes: HashMap::new(),
                    });
                }
            }
            2 => {
                // `splice` with an out-of-range index removes nothing.
                if (index as usize) < inner.players.len() {
                    inner.players.remove(index as usize);
                }
            }
            _ => return Err(KexError::Protocol("Invalid KEX player operation")),
        }
        reader.end()?;
        Ok(())
    }

    /// Apply a `key\value` player attribute (`playerAttribute`).
    fn player_attribute(players: &mut [KexLanPlayer], index: usize, text: &str) -> Result<(), KexError> {
        let Some(player) = players.get_mut(index) else {
            return Err(KexError::Protocol("Invalid KEX player attribute"));
        };
        let Some(separator) = text.find('\\').filter(|separator| *separator > 0) else {
            return Err(KexError::Protocol("Invalid KEX player attribute"));
        };
        if kex_text(text).len() >= 128 {
            return Err(KexError::Protocol("Invalid KEX player attribute"));
        }
        let (attr, value) = (&text[..separator], &text[separator + 1..]);
        if !value.is_empty() {
            player.attributes.insert(attr.to_owned(), value.to_owned());
        } else if attr != "name" {
            player.attributes.remove(attr);
        }
        Ok(())
    }

    /// Drop a peer (`remove`).
    fn remove(options: &KexLanOptions, inner: &mut KexLanInner, key: &str, clock: f64) -> Result<(), KexError> {
        let Some(peer) = inner.peers.remove(key) else {
            return Ok(());
        };
        if matches!(options, KexLanOptions::Client { .. }) {
            inner.joined = false;
            inner.attributes.clear();
            inner.players.clear();
            return Ok(());
        }
        for id in peer.players {
            let Some(index) = inner.players.iter().position(|player| player.id == id) else {
                continue;
            };
            inner.players.remove(index);
            let mut writer = KexWriter::new();
            writer.byte(2).integer(index as u64);
            let bytes = writer.finish();
            let targets: Vec<String> = inner
                .peers
                .iter()
                .filter(|(_, peer)| !peer.players.is_empty())
                .map(|(candidate, _)| candidate.clone())
                .collect();
            for target in targets {
                if let Some(peer) = inner.peers.get_mut(&target) {
                    peer.channel.send(PLAYER_KIND, &bytes, KexMode::Reliable, clock)?;
                }
            }
        }
        Ok(())
    }
}

impl<T: DatagramTransport<Address = NetworkAddress> + 'static> DatagramTransport for KexLanTransport<T> {
    type Address = NetworkAddress;

    fn address(&self) -> NetworkAddress {
        self.transport.address()
    }

    fn closed(&self) -> bool {
        self.is_ended()
    }

    fn max_datagram_bytes(&self) -> Option<usize> {
        Some(65535)
    }

    fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
        if self.is_ended() {
            return Err(TransportError::Closed("KEX transport is closed".to_owned()));
        }
        if !self.admitted(to) {
            return Ok(false);
        }
        let reliable = payload.len() >= 8 && payload[0..4] == [0, 0, 0, 0x80] && payload[4..8] == [0, 0, 0, 0x80];
        let clock = self.clock();
        let result = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| TransportError::Closed("KEX transport is closed".to_owned()))?;
            let peer = Self::peer_transport(&self.transport, &mut inner, to);
            peer.channel.send(
                GAME_KIND,
                payload,
                if reliable {
                    KexMode::Reliable
                } else {
                    KexMode::Sequential
                },
                clock,
            )
        };
        match result {
            Ok(accepted) => Ok(accepted),
            Err(error) => {
                self.queue.push(ReceiveEvent::Error {
                    error: error.to_string(),
                });
                Ok(false)
            }
        }
    }

    fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
        self.drain();
        Ok(self.queue.poll())
    }

    fn subscribe_readable(&self, listener: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
        self.queue.subscribe(listener)
    }

    fn unsubscribe(&self, token: u64) {
        self.queue.unsubscribe(token);
    }

    fn close(&self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::loopback::LoopbackHub;
    use crate::common::transport::{monotonic_clock, UNIFIED_DATAGRAM_LIMITS};

    /// Open a bound host/client pair over loopback.
    fn lobby_pair(
        host_players: u8,
        client_players: u8,
    ) -> (
        Arc<KexLanTransport<crate::common::loopback::LoopbackTransport>>,
        Arc<KexLanTransport<crate::common::loopback::LoopbackTransport>>,
    ) {
        let hub = Arc::new(LoopbackHub::new(UNIFIED_DATAGRAM_LIMITS, monotonic_clock()));
        let host_link = Arc::try_unwrap(hub.bind("host").unwrap()).unwrap_or_else(|_| panic!("hub holds no clone"));
        let host_address = host_link.address();
        let host = KexLanTransport::open(
            host_link,
            KexLanOptions::Host {
                max_players: 8,
                local_players: host_players,
                name: "test".to_owned(),
            },
            None,
        )
        .unwrap();
        let client_link = Arc::try_unwrap(hub.bind("client").unwrap()).unwrap_or_else(|_| panic!("hub holds no clone"));
        let client = KexLanTransport::open(
            client_link,
            KexLanOptions::Client {
                server: host_address,
                local_players: client_players,
            },
            None,
        )
        .unwrap();
        // The hub must outlive the test transports; leak it like the donor's process sockets.
        std::mem::forget(hub);
        (host, client)
    }

    /// Pump both lobbies until the client joins or attempts run out.
    fn join(
        host: &Arc<KexLanTransport<crate::common::loopback::LoopbackTransport>>,
        client: &Arc<KexLanTransport<crate::common::loopback::LoopbackTransport>>,
    ) {
        for step in 0..10 {
            let now = f64::from(step) * 600.0;
            client.tick(now).unwrap();
            host.tick(now).unwrap();
            client.tick(now).unwrap();
            if client.ready() {
                return;
            }
        }
        panic!("client never joined");
    }

    #[test]
    fn client_joins_and_exchanges_game_datagrams() {
        let (host, client) = lobby_pair(1, 2);
        assert!(host.ready());
        assert!(!client.ready());
        join(&host, &client);
        assert!(client.ready());
        assert_eq!(host.lobby_players().len(), 3);
        assert_eq!(client.lobby_players().len(), 3);
        assert!(host.admitted(&client.address()));
        assert!(client.admitted(&host.address()));
        assert!(client.send(&host.address(), b"move").unwrap());
        host.tick(9999.0).unwrap();
        let event = host.poll().unwrap().unwrap();
        let ReceiveEvent::Packet { from, payload, .. } = event else {
            panic!("expected a game packet, got {event:?}");
        };
        assert_eq!(from, client.address());
        assert_eq!(payload, b"move");
    }

    #[test]
    fn host_attributes_reach_clients() {
        let (host, client) = lobby_pair(1, 1);
        join(&host, &client);
        host.set_attribute("map", "q2dm1").unwrap();
        for step in 0..10 {
            let now = 20000.0 + f64::from(step) * 600.0;
            host.tick(now).unwrap();
            client.tick(now).unwrap();
        }
        assert!(client.ready());
        assert_eq!(
            client.inner.lock().unwrap().attributes.get("map").map(String::as_str),
            Some("q2dm1")
        );
        assert!(host.set_attribute("", "x").is_err());
        assert!(host.set_attribute("bad\\key", "x").is_err());
        assert!(client.set_attribute("map", "x").is_err());
    }

    #[test]
    fn unadmitted_sends_drop() {
        let (host, client) = lobby_pair(1, 1);
        assert!(!client.send(&host.address(), b"early").unwrap());
        assert!(!host.send(&client.address(), b"early").unwrap());
        assert!(host.poll().unwrap().is_none());
    }

    #[test]
    fn options_are_validated() {
        let hub = Arc::new(LoopbackHub::new(UNIFIED_DATAGRAM_LIMITS, monotonic_clock()));
        let first = Arc::try_unwrap(hub.bind("v1").unwrap()).unwrap_or_else(|_| panic!("hub holds no clone"));
        assert!(matches!(
            KexLanTransport::open(
                first,
                KexLanOptions::Client {
                    server: NetworkAddress::Loopback { id: "v2".to_owned() },
                    local_players: 0,
                },
                None,
            ),
            Err(KexError::Protocol("Invalid KEX local player count"))
        ));
        let second = Arc::try_unwrap(hub.bind("v2").unwrap()).unwrap_or_else(|_| panic!("hub holds no clone"));
        assert!(matches!(
            KexLanTransport::open(
                second,
                KexLanOptions::Host {
                    max_players: 1,
                    local_players: 2,
                    name: "x".to_owned(),
                },
                None,
            ),
            Err(KexError::Protocol("Invalid KEX player capacity"))
        ));
    }

    #[test]
    fn close_disconnects() {
        let (host, client) = lobby_pair(1, 1);
        join(&host, &client);
        client.close();
        assert!(client.closed());
        assert!(matches!(
            client.send(&host.address(), b"late"),
            Err(TransportError::Closed(_))
        ));
        host.tick(99999.0).unwrap();
        // The host drops the departed peer.
        assert!(!host.admitted(&client.address()));
    }
}
