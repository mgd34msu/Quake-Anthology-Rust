//! Native IPX datagram transport.
//!
//! Port of donor `src/platform/ipx.ts`: a fault-latching [`DatagramTransport`]
//! over a bound [`NativeSocket`](crate::ipx_native::NativeSocket), plus the
//! host capability probe. Loading this module loads no libraries and acquires
//! no sockets.

use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::ipx_native::{bind_native_ipx_socket, NativeSocket};

/// An IPX internetwork address: 32-bit network, 48-bit node, 16-bit port.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IpxAddress {
    /// Network number in host order.
    pub network: u32,
    /// Node address (usually the MAC).
    pub node: [u8; 6],
    /// Socket number; zero is never a valid endpoint.
    pub port: u16,
}

/// Validate a network port: 1..=65535, or 0..=65535 when `allow_zero`.
pub fn port_number(port: i64, allow_zero: bool) -> Result<u16> {
    let minimum = i64::from(!allow_zero);
    if port < minimum || port > 65535 {
        return Err(Error::OutOfRange("invalid network port".to_string()));
    }
    Ok(port as u16)
}

/// Build a validated IPX endpoint address.
pub fn ipx_address(network: u32, node: [u8; 6], port: u16) -> Result<IpxAddress> {
    if port == 0 {
        return Err(Error::OutOfRange("invalid IPX address".to_string()));
    }
    Ok(IpxAddress { network, node, port })
}

/// Canonical key: `ipx:<network hex>:<node hex>:<port>`.
pub fn address_key(address: &IpxAddress, include_port: bool) -> String {
    let node: String = address.node.iter().map(|b| format!("{b:02x}")).collect();
    if include_port {
        format!("ipx:{:08x}:{node}:{}", address.network, address.port)
    } else {
        format!("ipx:{:08x}:{node}", address.network)
    }
}

/// Bind options for a native IPX socket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeIpxBindOptions {
    /// Local socket number; 0 selects an ephemeral port.
    pub port: u16,
    /// IPX packet type placed on the wire.
    pub packet_type: u8,
    /// Broadcast must be enabled, like the donor.
    pub broadcast: bool,
}

/// A datagram receive outcome.
#[derive(Clone, Debug)]
pub enum ReceiveEvent {
    /// One datagram arrived.
    Packet {
        /// Sender address.
        from: IpxAddress,
        /// Payload bytes.
        payload: Vec<u8>,
        /// Monotonic receive timestamp in milliseconds.
        received_at_ms: f64,
    },
    /// A datagram was dropped before delivery.
    Dropped {
        /// Machine-readable cause, e.g. `"oversize"`.
        reason: String,
        /// Purported sender.
        from: IpxAddress,
    },
    /// The transport faulted; reported once, then the transport stays silent.
    Error {
        /// Latched fault.
        error: String,
    },
}

/// Minimal datagram transport surface used by the IPX host.
pub trait DatagramTransport: Send {
    /// Local bound address.
    fn address(&self) -> IpxAddress;
    /// Maximum payload bytes accepted by [`DatagramTransport::send`].
    fn max_datagram_bytes(&self) -> usize;
    /// Whether the transport is closed.
    fn is_closed(&self) -> bool;
    /// Queue one datagram; `Ok(false)` means transiently busy.
    fn send(&mut self, to: &IpxAddress, payload: &[u8]) -> Result<bool>;
    /// Take one pending receive outcome.
    fn poll(&mut self) -> Result<Option<ReceiveEvent>>;
    /// Close the transport; idempotent.
    fn close(&mut self);
}

/// Shared poll state behind the readability watcher thread.
struct Shared {
    ended: bool,
    fault: Option<String>,
    fault_reported: bool,
}

type Listener = Box<dyn Fn() + Send>;

struct WatcherHandle {
    stop: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
    listeners: Arc<Mutex<Vec<(u64, Listener)>>>,
    next_id: u64,
}

/// Native IPX transport with donor-equivalent fault latching.
pub struct NativeIpxTransport {
    socket: Option<Arc<dyn NativeSocket>>,
    address: IpxAddress,
    limit: usize,
    shared: Arc<Mutex<Shared>>,
    watcher: Option<WatcherHandle>,
}

impl NativeIpxTransport {
    /// Wrap a bound socket.
    pub fn new(socket: Arc<dyn NativeSocket>) -> Self {
        let address = socket.address();
        let limit = socket.max_datagram_bytes();
        Self {
            socket: Some(socket),
            address,
            limit,
            shared: Arc::new(Mutex::new(Shared {
                ended: false,
                fault: None,
                fault_reported: false,
            })),
            watcher: None,
        }
    }

    fn open(&self) -> Result<Arc<dyn NativeSocket>> {
        self.socket
            .clone()
            .ok_or_else(|| Error::Closed("native IPX socket".to_string()))
    }

    fn fail(&self, error: Error) {
        if let Ok(mut shared) = self.shared.lock() {
            if shared.fault.is_none() {
                shared.fault = Some(error.to_string());
            }
        }
    }

    fn take_fault(&self) -> Option<String> {
        let mut shared = self.shared.lock().ok()?;
        if shared.fault.is_some() && !shared.fault_reported {
            shared.fault_reported = true;
            return shared.fault.clone();
        }
        None
    }

    fn has_fault(&self) -> bool {
        self.shared.lock().is_ok_and(|shared| shared.fault.is_some())
    }

    fn notify_all(listeners: &Arc<Mutex<Vec<(u64, Listener)>>>) {
        if let Ok(list) = listeners.lock() {
            for (_, listener) in list.iter() {
                listener();
            }
        }
    }

    /// Subscribe a readability listener. One watcher thread polls the socket
    /// every 4 ms (like the donor's interval) and calls every listener
    /// whenever input is pending, the transport faults, or it closes.
    /// Dropping the last subscription stops the thread.
    pub fn subscribe_readable(&mut self, listener: impl Fn() + Send + 'static) -> Result<ReadableSubscription> {
        let socket = self.open()?;
        if self.watcher.is_none() {
            let (stop_tx, stop_rx) = mpsc::channel::<()>();
            let listeners: Arc<Mutex<Vec<(u64, Listener)>>> = Arc::new(Mutex::new(Vec::new()));
            let thread_listeners = Arc::clone(&listeners);
            let shared = Arc::clone(&self.shared);
            let thread = thread::Builder::new()
                .name("qa-ipx-readable".to_string())
                .spawn(move || loop {
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }
                    let done = shared.lock().is_ok_and(|shared| shared.ended || shared.fault.is_some());
                    if done {
                        Self::notify_all(&thread_listeners);
                        break;
                    }
                    let ready = match socket.readable() {
                        Ok(readable) => readable,
                        Err(error) => {
                            if let Ok(mut shared) = shared.lock() {
                                if shared.fault.is_none() {
                                    shared.fault = Some(error.to_string());
                                }
                            }
                            true
                        }
                    };
                    if ready {
                        Self::notify_all(&thread_listeners);
                        if shared.lock().is_ok_and(|shared| shared.fault.is_some()) {
                            break;
                        }
                    }
                    thread::sleep(Duration::from_millis(4));
                })
                .map_err(Error::Io)?;
            self.watcher = Some(WatcherHandle {
                stop: stop_tx,
                thread: Some(thread),
                listeners,
                next_id: 0,
            });
        }
        let watcher = self.watcher.as_mut().expect("watcher just created");
        let id = watcher.next_id;
        watcher.next_id += 1;
        watcher
            .listeners
            .lock()
            .map_err(|_| Error::native("subscribe", "listener list is poisoned".to_string()))?
            .push((id, Box::new(listener)));
        if self.has_fault() {
            Self::notify_all(&self.watcher.as_ref().expect("watcher just created").listeners);
        }
        Ok(ReadableSubscription {
            listeners: Arc::clone(&self.watcher.as_ref().expect("watcher just created").listeners),
            stop: self.watcher.as_ref().expect("watcher just created").stop.clone(),
            id,
        })
    }

    fn stop_watcher(&mut self) {
        if let Some(mut watcher) = self.watcher.take() {
            watcher.stop.send(()).ok();
            if let Some(thread) = watcher.thread.take() {
                thread.join().ok();
            }
        }
    }
}

/// Active readability subscription; dropping the last one stops the watcher.
pub struct ReadableSubscription {
    listeners: Arc<Mutex<Vec<(u64, Listener)>>>,
    stop: mpsc::Sender<()>,
    id: u64,
}

impl Drop for ReadableSubscription {
    fn drop(&mut self) {
        if let Ok(mut list) = self.listeners.lock() {
            list.retain(|(id, _)| *id != self.id);
            if list.is_empty() {
                self.stop.send(()).ok();
            }
        }
    }
}

impl DatagramTransport for NativeIpxTransport {
    fn address(&self) -> IpxAddress {
        self.address.clone()
    }

    fn max_datagram_bytes(&self) -> usize {
        self.limit
    }

    fn is_closed(&self) -> bool {
        self.socket.is_none()
    }

    fn send(&mut self, to: &IpxAddress, payload: &[u8]) -> Result<bool> {
        let socket = self.open()?;
        ipx_address(to.network, to.node, to.port)?;
        if payload.len() > self.limit {
            return Err(Error::OutOfRange(
                "datagram exceeds native IPX payload limit".to_string(),
            ));
        }
        if self.has_fault() {
            return Ok(false);
        }
        match socket.send(to, payload) {
            Ok(accepted) => Ok(accepted),
            Err(error) => {
                self.fail(error);
                Ok(false)
            }
        }
    }

    fn poll(&mut self) -> Result<Option<ReceiveEvent>> {
        let socket = self.open()?;
        if !self.has_fault() {
            match socket.receive() {
                Ok(event) => return Ok(event),
                Err(error) => self.fail(error),
            }
        }
        if let Some(error) = self.take_fault() {
            return Ok(Some(ReceiveEvent::Error { error }));
        }
        Ok(None)
    }

    fn close(&mut self) {
        if let Some(socket) = self.socket.take() {
            socket.close();
        }
        if let Ok(mut shared) = self.shared.lock() {
            shared.ended = true;
        }
        self.stop_watcher();
    }
}

impl Drop for NativeIpxTransport {
    fn drop(&mut self) {
        self.close();
    }
}

/// Host capability for native AF_IPX sockets.
pub enum NativeIpxCapability {
    /// No AF_IPX ABI on this host.
    Unavailable {
        /// Human-readable cause.
        reason: String,
    },
    /// Sockets can be bound.
    Available,
}

impl NativeIpxCapability {
    /// Bind a transport. Only [`NativeIpxCapability::Available`] binds.
    pub fn bind(&self, options: NativeIpxBindOptions) -> Result<NativeIpxTransport> {
        match self {
            Self::Unavailable { reason } => Err(Error::Unsupported(format!("ipx-native: {reason}"))),
            Self::Available => {
                let socket = bind_native_ipx_socket(options).map_err(|error| {
                    if matches!(error, Error::Unsupported(_)) {
                        return error;
                    }
                    Error::native("bind", format!("native IPX bind failed: {error}"))
                })?;
                Ok(NativeIpxTransport::new(socket))
            }
        }
    }
}

/// Probe the host for a native AF_IPX ABI without loading anything.
pub fn native_ipx_capability() -> NativeIpxCapability {
    #[cfg(target_os = "linux")]
    {
        if cfg!(target_arch = "x86_64") || cfg!(target_arch = "aarch64") {
            NativeIpxCapability::Available
        } else {
            NativeIpxCapability::Unavailable {
                reason: "native IPX requires Linux x64/arm64 glibc".to_string(),
            }
        }
    }
    #[cfg(windows)]
    {
        if cfg!(target_arch = "x86_64") {
            return NativeIpxCapability::Available;
        }
        return NativeIpxCapability::Unavailable {
            reason: "native IPX requires Windows x64 Winsock".to_string(),
        };
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        NativeIpxCapability::Unavailable {
            reason: "no AF_IPX socket ABI is implemented for this platform".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;

    struct FakeSocket {
        address: IpxAddress,
        limit: usize,
        sent: StdMutex<Vec<(IpxAddress, Vec<u8>)>>,
        inbound: StdMutex<VecDeque<ReceiveEvent>>,
        fail_send: bool,
    }

    impl FakeSocket {
        fn with_events(events: Vec<ReceiveEvent>) -> Self {
            Self {
                address: ipx_address(1, [2; 6], 0x4711).unwrap(),
                limit: 546,
                sent: StdMutex::new(Vec::new()),
                inbound: StdMutex::new(events.into()),
                fail_send: false,
            }
        }
    }

    impl NativeSocket for FakeSocket {
        fn address(&self) -> IpxAddress {
            self.address.clone()
        }
        fn max_datagram_bytes(&self) -> usize {
            self.limit
        }
        fn send(&self, to: &IpxAddress, payload: &[u8]) -> Result<bool> {
            if self.fail_send {
                return Err(Error::native("sendto", "boom"));
            }
            self.sent.lock().unwrap().push((to.clone(), payload.to_vec()));
            Ok(true)
        }
        fn receive(&self) -> Result<Option<ReceiveEvent>> {
            Ok(self.inbound.lock().unwrap().pop_front())
        }
        fn readable(&self) -> Result<bool> {
            Ok(!self.inbound.lock().unwrap().is_empty())
        }
        fn close(&self) {}
    }

    #[test]
    fn address_validation_and_keys() {
        assert!(port_number(0, false).is_err());
        assert_eq!(port_number(0, true).unwrap(), 0);
        assert!(port_number(65536, true).is_err());
        assert!(ipx_address(0, [0; 6], 0).is_err());
        let address = ipx_address(0x0a0b_0c0d, [1, 2, 3, 4, 5, 6], 213).unwrap();
        assert_eq!(address_key(&address, true), "ipx:0a0b0c0d:010203040506:213");
        assert_eq!(address_key(&address, false), "ipx:0a0b0c0d:010203040506");
    }

    #[test]
    fn transport_sends_receives_and_latches_faults() {
        let peer = ipx_address(9, [9; 6], 100).unwrap();
        let mut transport = NativeIpxTransport::new(Arc::new(FakeSocket::with_events(vec![ReceiveEvent::Packet {
            from: peer.clone(),
            payload: vec![1, 2, 3],
            received_at_ms: 4.0,
        }])));
        assert!(transport.send(&peer, &[7]).unwrap());
        assert!(transport.send(&peer, &vec![0u8; 547]).is_err());
        assert!(!transport.is_closed());
        let event = transport.poll().unwrap().unwrap();
        assert!(matches!(event, ReceiveEvent::Packet { .. }));
        assert!(transport.poll().unwrap().is_none());
        transport.close();
        assert!(transport.is_closed());
        assert!(transport.send(&peer, &[1]).is_err());
        assert!(transport.poll().is_err());
    }

    #[test]
    fn fault_reports_once_then_silent() {
        struct Failing;
        impl NativeSocket for Failing {
            fn address(&self) -> IpxAddress {
                ipx_address(1, [0; 6], 1).unwrap()
            }
            fn max_datagram_bytes(&self) -> usize {
                546
            }
            fn send(&self, _: &IpxAddress, _: &[u8]) -> Result<bool> {
                Err(Error::native("sendto", "dead"))
            }
            fn receive(&self) -> Result<Option<ReceiveEvent>> {
                Err(Error::native("recvfrom", "dead"))
            }
            fn readable(&self) -> Result<bool> {
                Ok(true)
            }
            fn close(&self) {}
        }
        let peer = ipx_address(1, [1; 6], 2).unwrap();
        let mut transport = NativeIpxTransport::new(Arc::new(Failing));
        assert!(!transport.send(&peer, &[1]).unwrap());
        let first = transport.poll().unwrap().unwrap();
        assert!(matches!(first, ReceiveEvent::Error { .. }));
        assert!(transport.poll().unwrap().is_none());
    }

    #[test]
    fn capability_binds_or_reports_honestly() {
        match native_ipx_capability() {
            NativeIpxCapability::Unavailable { reason } => {
                assert!(!reason.is_empty());
            }
            NativeIpxCapability::Available => {
                // Binding may fail on hosts without IPX (modern kernels
                // removed AF_IPX); any failure must be a well-formed error,
                // and a success must round-trip close.
                let options = NativeIpxBindOptions {
                    port: 0,
                    packet_type: 4,
                    broadcast: true,
                };
                match NativeIpxCapability::Available.bind(options) {
                    Ok(mut transport) => {
                        assert_eq!(transport.max_datagram_bytes(), 546);
                        transport.close();
                    }
                    Err(error) => assert!(!error.to_string().is_empty(), "{error}"),
                }
            }
        }
        let bad = NativeIpxBindOptions {
            port: 0,
            packet_type: 4,
            broadcast: false,
        };
        assert!(NativeIpxCapability::Available.bind(bad).is_err());
    }

    #[test]
    fn sockaddr_layouts_round_trip() {
        use crate::ipx_native::{address_from_bytes, sockaddr_bytes};
        let address = ipx_address(0x1122_3344, [6, 5, 4, 3, 2, 1], 0x2bee).unwrap();
        for windows in [false, true] {
            let bytes = sockaddr_bytes(windows, 0x2bee, 7, Some(&address));
            assert_eq!(bytes.len(), if windows { 14 } else { 16 });
            let decoded = address_from_bytes(&bytes, bytes.len(), windows).unwrap();
            assert_eq!(decoded, address);
            if !windows {
                assert_eq!(bytes[14], 7);
            }
        }
        assert!(address_from_bytes(&[0u8; 4], 4, false).is_err());
        let mut foreign = sockaddr_bytes(false, 1, 0, None);
        foreign[0] = 2;
        assert!(address_from_bytes(&foreign, foreign.len(), false).is_err());
    }

    #[test]
    fn failure_classification_matches_donor() {
        use crate::ipx_native::{ipx_failure, is_family_missing, is_no_interface};
        assert!(is_family_missing(97, false));
        assert!(!is_family_missing(11, false));
        assert!(is_family_missing(10047, true));
        assert!(is_no_interface(101, false));
        assert!(is_no_interface(10051, true));
        let missing = ipx_failure("socket", 97, false);
        assert!(matches!(missing, Error::Unsupported(_)), "{missing}");
        assert!(missing.to_string().contains("no installed IPX"), "{missing}");
        let route = ipx_failure("bind", 101, false);
        assert!(route.to_string().contains("no usable configured IPX"), "{route}");
    }
}
