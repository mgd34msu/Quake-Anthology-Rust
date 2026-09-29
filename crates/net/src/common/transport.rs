//! Datagram transports ported from `src/network/common/transport.ts`.
//!
//! [`PacketQueue`] is the shared bounded receive queue; [`UdpTransport`]
//! binds a nonblocking [`std::net::UdpSocket`] and pumps it on [`poll`]
//! instead of the donor's async callbacks, so polling drives both network
//! and loopback I/O with [`std::net`] only.
//!
//! [`poll`]: DatagramTransport::poll

use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use thiserror::Error;

use super::endpoint::{ip_address, ipv4_address, port_number, same_address, NetworkAddress};
use super::socks::{read_socks_datagram, socks_datagram, SocksAssociation, SocksOptions};

/// Receive queue limits (`DatagramLimits`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatagramLimits {
    /// Maximum accepted datagram bytes.
    pub max_bytes: usize,
    /// Maximum queued receive events.
    pub queue_packets: usize,
}

/// Quake III datagram limits.
pub const Q3_DATAGRAM_LIMITS: DatagramLimits = DatagramLimits {
    max_bytes: 16383,
    queue_packets: 256,
};
/// Quake II datagram limits.
pub const Q2_DATAGRAM_LIMITS: DatagramLimits = DatagramLimits {
    max_bytes: 4096,
    queue_packets: 256,
};
/// Unified datagram limits.
pub const UNIFIED_DATAGRAM_LIMITS: DatagramLimits = DatagramLimits {
    max_bytes: 65507,
    queue_packets: 256,
};

/// Queued receive event (`ReceiveEvent`).
#[derive(Debug, Clone)]
pub enum ReceiveEvent<A = NetworkAddress> {
    /// Inbound datagram.
    Packet {
        /// Sender.
        from: A,
        /// Payload bytes.
        payload: Vec<u8>,
        /// Monotonic receive time in milliseconds.
        received_at: f64,
    },
    /// Transport-level failure.
    Error {
        /// Failure description.
        error: String,
    },
    /// Dropped datagram.
    Dropped {
        /// Drop reason.
        reason: DropReason,
        /// Sender.
        from: A,
    },
}

/// Receive drop reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// Datagram exceeded the selected limit.
    Oversize,
    /// Queue overflow displaced an event.
    Overflow,
}

/// Error for transport misuse and bind failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransportError {
    /// Datagram limits are not positive.
    #[error("Invalid datagram limits")]
    BadLimits,
    /// Transport or queue is closed.
    #[error("{0}")]
    Closed(String),
    /// Datagram exceeds the selected transport limit.
    #[error("Datagram exceeds selected transport limit")]
    Oversize,
    /// SOCKS association is unavailable or the destination is unsupported.
    #[error("SOCKS association unavailable or destination unsupported")]
    SocksUnavailable,
    /// SOCKS UDP association requires IPv4.
    #[error("SOCKS UDP association requires IPv4")]
    SocksRequiresIpv4,
    /// UDP broadcast could not be enabled.
    #[error("Could not enable UDP broadcast")]
    BroadcastFailed,
    /// Bind or socket setup failed.
    #[error("UDP bind failed: {0}")]
    BindFailed(String),
    /// Invalid network port.
    #[error("Invalid network port")]
    BadPort,
}

/// Millisecond clock shared by transports and queues.
pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

fn epoch() -> &'static Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now)
}

/// Default monotonic millisecond clock.
#[must_use]
pub fn monotonic_clock() -> Clock {
    Arc::new(|| epoch().elapsed().as_secs_f64() * 1000.0)
}

struct QueueInner<A> {
    events: std::collections::VecDeque<ReceiveEvent<A>>,
    listeners: Vec<(u64, Arc<dyn Fn() + Send + Sync>)>,
    ended: bool,
    dropped: usize,
}

/// Bounded receive queue (`PacketQueue`).
pub struct PacketQueue<A> {
    inner: Mutex<QueueInner<A>>,
    next_listener: AtomicU64,
    /// Queue limits.
    pub limits: DatagramLimits,
    now: Clock,
}

impl<A> PacketQueue<A> {
    /// Create a queue with `limits` and a millisecond clock.
    pub fn new(limits: DatagramLimits, now: Clock) -> Result<Self, TransportError> {
        if limits.max_bytes < 1 || limits.queue_packets < 1 {
            return Err(TransportError::BadLimits);
        }
        Ok(Self {
            inner: Mutex::new(QueueInner {
                events: std::collections::VecDeque::new(),
                listeners: Vec::new(),
                ended: false,
                dropped: 0,
            }),
            next_listener: AtomicU64::new(1),
            limits,
            now,
        })
    }

    /// Events dropped so far.
    #[must_use]
    pub fn dropped(&self) -> usize {
        self.inner.lock().map(|inner| inner.dropped).unwrap_or(0)
    }

    /// Accept an inbound datagram, recording oversize as a drop event.
    pub fn accept(&self, from: A, payload: &[u8], truncated: bool) {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return,
        };
        if inner.ended {
            return;
        }
        if truncated || payload.len() > self.limits.max_bytes {
            inner.dropped += 1;
            let event = ReceiveEvent::Dropped {
                reason: DropReason::Oversize,
                from,
            };
            Self::push_locked(&mut inner, self.limits, event);
            let listeners = Self::listeners_locked(&inner);
            drop(inner);
            Self::notify(listeners);
            return;
        }
        let event = ReceiveEvent::Packet {
            from,
            payload: payload.to_vec(),
            received_at: (self.now)(),
        };
        Self::push_locked(&mut inner, self.limits, event);
        let listeners = Self::listeners_locked(&inner);
        drop(inner);
        Self::notify(listeners);
    }

    /// Push a synthesized event, displacing the oldest on overflow.
    pub fn push(&self, event: ReceiveEvent<A>) {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return,
        };
        if inner.ended {
            return;
        }
        Self::push_locked(&mut inner, self.limits, event);
        let listeners = Self::listeners_locked(&inner);
        drop(inner);
        Self::notify(listeners);
    }

    fn push_locked(inner: &mut QueueInner<A>, limits: DatagramLimits, event: ReceiveEvent<A>) {
        if inner.events.len() == limits.queue_packets {
            inner.events.pop_front();
            inner.dropped += 1;
        }
        inner.events.push_back(event);
    }

    fn listeners_locked(inner: &QueueInner<A>) -> Vec<Arc<dyn Fn() + Send + Sync>> {
        inner.listeners.iter().map(|(_, listener)| listener.clone()).collect()
    }

    fn notify(listeners: Vec<Arc<dyn Fn() + Send + Sync>>) {
        for listener in listeners {
            listener();
        }
    }

    /// Pop the oldest event.
    pub fn poll(&self) -> Option<ReceiveEvent<A>> {
        self.inner.lock().ok()?.events.pop_front()
    }

    /// Subscribe a readable listener; returns an unsubscribe token.
    pub fn subscribe(&self, listener: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| TransportError::Closed("Packet queue is closed".to_owned()))?;
        if inner.ended {
            return Err(TransportError::Closed("Packet queue is closed".to_owned()));
        }
        let token = self.next_listener.fetch_add(1, Ordering::Relaxed);
        inner.listeners.push((token, listener));
        Ok(token)
    }

    /// Remove a readable listener.
    pub fn unsubscribe(&self, token: u64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.listeners.retain(|(candidate, _)| *candidate != token);
        }
    }

    /// Close the queue and wake every listener once.
    pub fn close(&self) {
        let listeners = {
            let mut inner = match self.inner.lock() {
                Ok(inner) => inner,
                Err(_) => return,
            };
            if inner.ended {
                return;
            }
            inner.ended = true;
            inner.events.clear();
            std::mem::take(&mut inner.listeners)
        };
        for (_, listener) in listeners {
            listener();
        }
    }
}

/// Datagram endpoint (`DatagramTransport`).
pub trait DatagramTransport: Send + Sync {
    /// Endpoint address type.
    type Address: Clone + Send;

    /// Bound address.
    fn address(&self) -> Self::Address;
    /// True once closed.
    fn closed(&self) -> bool;
    /// Optional per-transport datagram ceiling.
    fn max_datagram_bytes(&self) -> Option<usize> {
        None
    }
    /// Send a datagram; `Ok(false)` reports a dropped send.
    fn send(&self, to: &Self::Address, payload: &[u8]) -> Result<bool, TransportError>;
    /// Pop the oldest receive event.
    fn poll(&self) -> Result<Option<ReceiveEvent<Self::Address>>, TransportError>;
    /// Subscribe a readable listener; returns an unsubscribe token.
    fn subscribe_readable(&self, listener: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError>;
    /// Remove a readable listener.
    fn unsubscribe(&self, token: u64);
    /// Close the transport.
    fn close(&self);
}

/// UDP bind options (`UdpBindOptions`).
pub struct UdpBindOptions {
    /// Bind host literal.
    pub host: String,
    /// Bind port (zero selects an ephemeral port).
    pub port: u32,
    /// Receive limits.
    pub limits: DatagramLimits,
    /// Millisecond clock.
    pub now: Clock,
    /// Enable broadcast sends.
    pub broadcast: bool,
}

impl UdpBindOptions {
    /// Bind options for `host:port`.
    #[must_use]
    pub fn new(host: &str, port: u32) -> Self {
        Self {
            host: host.to_owned(),
            port,
            limits: UNIFIED_DATAGRAM_LIMITS,
            now: monotonic_clock(),
            broadcast: false,
        }
    }
}

/// Nonblocking UDP transport (`UdpTransport`).
pub struct UdpTransport {
    socket: UdpSocket,
    queue: PacketQueue<NetworkAddress>,
    address: NetworkAddress,
    socks: Mutex<SocksAssociation>,
    ended: AtomicBool,
}

impl UdpTransport {
    /// Bind a UDP socket (`UdpTransport.bind`).
    pub fn bind(options: &UdpBindOptions) -> Result<Self, TransportError> {
        ip_address(&options.host, options.port, true).map_err(|_| TransportError::BadPort)?;
        let queue = PacketQueue::new(options.limits, options.now.clone())?;
        let socket = UdpSocket::bind(format!("{}:{}", options.host, options.port))
            .map_err(|error| TransportError::BindFailed(error.to_string()))?;
        socket
            .set_nonblocking(true)
            .map_err(|error| TransportError::BindFailed(error.to_string()))?;
        if options.broadcast && socket.set_broadcast(true).is_err() {
            return Err(TransportError::BroadcastFailed);
        }
        let local = socket
            .local_addr()
            .map_err(|error| TransportError::BindFailed(error.to_string()))?;
        let address = socket_address(&local);
        Ok(Self {
            socket,
            queue,
            address,
            socks: Mutex::new(SocksAssociation::new()),
            ended: AtomicBool::new(false),
        })
    }

    /// Dropped packet count.
    #[must_use]
    pub fn dropped_packets(&self) -> usize {
        self.queue.dropped()
    }

    fn opened(&self) -> Result<(), TransportError> {
        if self.ended.load(Ordering::Relaxed) {
            return Err(TransportError::Closed("UDP transport is closed".to_owned()));
        }
        Ok(())
    }

    /// Negotiate a SOCKS5 UDP association (`connectSocks`).
    pub fn connect_socks(&self, options: &SocksOptions) -> Result<(), super::socks::SocksError> {
        self.opened().map_err(|_| super::socks::SocksError::ControlFailed)?;
        if !matches!(self.address, NetworkAddress::Ipv4 { .. }) {
            return Err(super::socks::SocksError::Rejected(
                "SOCKS UDP association requires IPv4".to_owned(),
            ));
        }
        let mut socks = self.socks.lock().map_err(|_| super::socks::SocksError::ControlFailed)?;
        socks.close();
        *socks = SocksAssociation::new();
        let port = match &self.address {
            NetworkAddress::Ipv4 { port, .. } => *port,
            _ => 0,
        };
        let result = socks.open(options, port);
        if self.ended.load(Ordering::Relaxed) {
            socks.close();
            return Err(super::socks::SocksError::ControlFailed);
        }
        result
    }

    fn pump(&self) {
        let mut buffer = vec![0u8; 65535];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((length, from)) => {
                    self.queue.accept(socket_address(&from), &buffer[..length], false);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    self.queue.push(ReceiveEvent::Error {
                        error: error.to_string(),
                    });
                    break;
                }
            }
        }
    }
}

fn socket_address(addr: &std::net::SocketAddr) -> NetworkAddress {
    match addr {
        std::net::SocketAddr::V4(v4) => {
            let octets = v4.ip().octets();
            ipv4_address(octets, u32::from(v4.port()), true).unwrap_or(NetworkAddress::Ipv4 {
                host: octets,
                port: v4.port(),
            })
        }
        std::net::SocketAddr::V6(v6) => NetworkAddress::Ipv6 {
            host: v6.ip().to_string(),
            port: v6.port(),
        },
    }
}

impl DatagramTransport for UdpTransport {
    type Address = NetworkAddress;

    fn address(&self) -> NetworkAddress {
        self.address.clone()
    }

    fn closed(&self) -> bool {
        self.ended.load(Ordering::Relaxed)
    }

    fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
        self.opened()?;
        if matches!(to, NetworkAddress::Ipv4 { port: 0, .. }) {
            port_number(0, false).map_err(|_| TransportError::BadPort)?;
        }
        if payload.len() > self.queue.limits.max_bytes {
            return Err(TransportError::Oversize);
        }
        let socks = self
            .socks
            .lock()
            .map_err(|_| TransportError::Closed("UDP transport is closed".to_owned()))?;
        let relay = socks.relay().cloned();
        let proxied = match (&relay, to) {
            (
                Some(NetworkAddress::Ipv4 {
                    host: relay_host,
                    port: relay_port,
                }),
                NetworkAddress::Ipv4 { host, .. },
            ) if !host.iter().all(|byte| *byte == 255) => Some((relay_host, relay_port)),
            (Some(_), _) => return Err(TransportError::SocksUnavailable),
            _ => None,
        };
        drop(socks);
        let (destination, bytes) = match (proxied, to) {
            (Some((host, port)), NetworkAddress::Ipv4 { port: to_port, .. }) => {
                let host = *host;
                let bytes = socks_datagram(&host_of(to), *to_port, payload);
                (NetworkAddress::Ipv4 { host, port: *port }, bytes)
            }
            _ => (to.clone(), payload.to_vec()),
        };
        let target = match &destination {
            NetworkAddress::Ipv4 { host, port } => {
                format!("{}.{}.{}.{}:{}", host[0], host[1], host[2], host[3], port)
            }
            NetworkAddress::Ipv6 { host, port } => format!("[{host}]:{port}"),
            _ => return Ok(false),
        };
        match self.socket.send_to(&bytes, target) {
            Ok(length) => Ok(length == bytes.len()),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
            Err(error) => {
                self.queue.push(ReceiveEvent::Error {
                    error: error.to_string(),
                });
                Ok(false)
            }
        }
    }

    fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
        self.opened()?;
        self.pump();
        let event = self.queue.poll();
        let relay = self.socks.lock().ok().and_then(|socks| socks.relay().cloned());
        if let (
            Some(ReceiveEvent::Packet {
                from,
                payload,
                received_at,
            }),
            Some(relay),
        ) = (event.clone(), relay)
        {
            if same_address(&from, &relay, true) && from.kind() == "ipv4" {
                return Ok(match read_socks_datagram(&payload) {
                    Some((decoded_from, decoded)) => Some(ReceiveEvent::Packet {
                        from: decoded_from,
                        payload: decoded,
                        received_at,
                    }),
                    None => None,
                });
            }
            return Ok(Some(ReceiveEvent::Packet {
                from,
                payload,
                received_at,
            }));
        }
        Ok(event)
    }

    fn subscribe_readable(&self, listener: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
        self.opened()?;
        self.queue.subscribe(listener)
    }

    fn unsubscribe(&self, token: u64) {
        self.queue.unsubscribe(token);
    }

    fn close(&self) {
        if self.ended.swap(true, Ordering::Relaxed) {
            return;
        }
        if let Ok(mut socks) = self.socks.lock() {
            socks.close();
        }
        self.queue.close();
    }
}

fn host_of(address: &NetworkAddress) -> [u8; 4] {
    match address {
        NetworkAddress::Ipv4 { host, .. } => *host,
        _ => [0, 0, 0, 0],
    }
}

/// Retained-transport error for IPX-native and serial drivers
/// (`UnsupportedTransportError`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{transport}: {detail}")]
pub struct UnsupportedTransportError {
    /// Requested transport.
    pub transport: String,
    /// Failure detail.
    pub detail: String,
}

impl UnsupportedTransportError {
    /// Create an unsupported-transport error.
    #[must_use]
    pub fn new(transport: &str, detail: &str) -> Self {
        Self {
            transport: transport.to_owned(),
            detail: detail.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;

    #[test]
    fn queue_reports_oversize_and_overflow() {
        let queue = PacketQueue::new(
            DatagramLimits {
                max_bytes: 4,
                queue_packets: 1,
            },
            monotonic_clock(),
        )
        .unwrap();
        let from = ipv4_address([127, 0, 0, 1], 1, false).unwrap();
        queue.accept(from.clone(), &[1, 2, 3, 4, 5], false);
        assert!(matches!(
            queue.poll(),
            Some(ReceiveEvent::Dropped {
                reason: DropReason::Oversize,
                ..
            })
        ));
        queue.accept(from.clone(), &[1], false);
        queue.accept(from.clone(), &[2], false);
        assert_eq!(queue.dropped(), 2);
        assert!(matches!(queue.poll(), Some(ReceiveEvent::Packet { .. })));
    }

    #[test]
    fn queue_notifies_listeners() {
        let queue: PacketQueue<NetworkAddress> = PacketQueue::new(UNIFIED_DATAGRAM_LIMITS, monotonic_clock()).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let probe = hits.clone();
        queue
            .subscribe(Arc::new(move || {
                probe.fetch_add(1, Ordering::Relaxed);
            }))
            .unwrap();
        let from = ipv4_address([127, 0, 0, 1], 1, false).unwrap();
        queue.accept(from, &[1], false);
        assert_eq!(hits.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn udp_loopback_round_trip() {
        let first = UdpTransport::bind(&UdpBindOptions::new("127.0.0.1", 0)).unwrap();
        let second = UdpTransport::bind(&UdpBindOptions::new("127.0.0.1", 0)).unwrap();
        let target = second.address();
        assert!(first.send(&target, &[7, 7, 7]).unwrap());
        let mut received = None;
        for _ in 0..100 {
            if let Some(event) = second.poll().unwrap() {
                received = Some(event);
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(matches!(received, Some(ReceiveEvent::Packet { .. })));
    }
}
