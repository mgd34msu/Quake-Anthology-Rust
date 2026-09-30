//! Application network transports (UDP, DOSBox IPX, native IPX).
//!
//! Port of `src/app/bootstrap/network/transport.ts`. The donor is async with
//! an `AbortSignal`; this sync port drops cancellation (callers run each open
//! to completion) and resolves every bind inline. Endpoint, transport, IPX,
//! and SOCKS behavior reuse `qa_net::common`; the native IPX probe comes from
//! `qa_platform::ipx` (see gap note below) and is adapted onto the shared
//! [`DatagramTransport`] by [`NativeIpxAdapter`].
//!
//! GAP: `crates/app` does not currently depend on `qa-platform`; add
//! `qa-platform = { workspace = true }` to `crates/app/Cargo.toml` before
//! compiling this module.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use qa_net::common::endpoint::{ipx_address, resolve_address, AddressError, NetworkAddress, ResolveFamily};
use qa_net::common::ipx::{
    bind_ipx_transport, DosBoxIpxNetwork, IpxError, IpxGame, IpxHost, IpxSocket, NativeIpxCapability,
};
use qa_net::common::socks::{SocksError, SocksOptions};
use qa_net::common::transport::{
    monotonic_clock, DatagramLimits, DatagramTransport, DropReason, ReceiveEvent, TransportError, UdpBindOptions,
    UdpTransport,
};
use qa_platform::ipx as platform_ipx;
use qa_platform::ipx::DatagramTransport as PlatformDatagramTransport;
use qa_platform::ipx::IpxAddress as PlatformIpxAddress;
use qa_platform::ipx::ReadableSubscription;
use thiserror::Error;

/// Application network transport selection
/// (`ApplicationNetworkTransport`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationNetworkTransport {
    /// UDP datagrams.
    Udp,
    /// DOSBox IPX rendezvous relay.
    IpxDosBox {
        /// Relay host.
        relay: String,
    },
    /// Native IPX sockets.
    IpxNative,
}

/// Application network address (`ApplicationNetworkAddress`).
pub type ApplicationNetworkAddress = NetworkAddress;

/// Application network family (`ApplicationNetworkFamily`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationNetworkFamily {
    /// Quake.
    Q1,
    /// QuakeWorld.
    Qw,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// Application UDP socket (`ApplicationUdpSocket`).
pub type ApplicationUdpSocket = Arc<UdpTransport>;

/// UDP bind callback.
pub type UdpBinder = Box<dyn Fn(&UdpBindOptions) -> Result<UdpTransport, TransportError> + Send + Sync>;

/// Application transport capabilities
/// (`ApplicationTransportCapabilities`).
pub struct ApplicationTransportCapabilities {
    /// Native IPX capability.
    pub native_ipx: NativeIpxCapability,
    /// UDP bind callback.
    pub bind_udp: UdpBinder,
}

/// Error for application network transports.
#[derive(Debug, Error)]
pub enum ApplicationTransportError {
    /// IPX was selected for QuakeWorld.
    #[error("QuakeWorld uses UDP; IPX is not a QuakeWorld transport")]
    QuakeWorldRequiresUdp,
    /// IPX remote text is malformed.
    #[error("IPX address must be NETWORK.NODE[:SOCKET] with 8 and 12 hexadecimal digits")]
    BadIpxText,
    /// A UDP socket was requested from an IPX transport.
    #[error("The selected source requires a UDP transport")]
    RequiresUdp,
    /// An IP destination was used with an IPX transport.
    #[error("An IPX transport requires an IPX destination")]
    RequiresIpxDestination,
    /// An IPX destination was used with a UDP transport.
    #[error("An IPX destination requires an explicitly selected IPX transport")]
    RequiresIpxTransport,
    /// SOCKS was requested on an IPX transport.
    #[error("SOCKS proxying is not available for the selected IPX transport")]
    SocksRequiresUdp,
    /// A DOSBox relay did not resolve to IPv4.
    #[error("DOSBox IPX relay must resolve to IPv4")]
    DosBoxRelayRequiresIpv4,
    /// Address failure.
    #[error(transparent)]
    Address(#[from] AddressError),
    /// Transport failure.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// SOCKS failure.
    #[error(transparent)]
    Socks(#[from] SocksError),
    /// IPX failure.
    #[error(transparent)]
    Ipx(#[from] IpxError),
}

/// Host transport capabilities (donor `bunApplicationTransports`).
pub fn host_application_transports() -> ApplicationTransportCapabilities {
    ApplicationTransportCapabilities {
        native_ipx: host_native_ipx_capability(),
        bind_udp: Box::new(UdpTransport::bind),
    }
}

/// Native IPX capability from the host probe (donor
/// `bunNativeIpxCapability`), adapted onto the shared IPX capability.
pub fn host_native_ipx_capability() -> NativeIpxCapability {
    match platform_ipx::native_ipx_capability() {
        platform_ipx::NativeIpxCapability::Unavailable { reason } => NativeIpxCapability::Unavailable { reason },
        platform_ipx::NativeIpxCapability::Available => {
            NativeIpxCapability::Available(Box::new(|port, packet_type| {
                let capability = platform_ipx::native_ipx_capability();
                let transport = capability
                    .bind(platform_ipx::NativeIpxBindOptions {
                        port,
                        packet_type,
                        broadcast: true,
                    })
                    .map_err(|error| TransportError::Closed(error.to_string()))?;
                NativeIpxAdapter::wrap(transport)
            }))
        }
    }
}

/// Reject IPX for QuakeWorld (donor `checkFamily`).
fn check_family(
    selection: &ApplicationNetworkTransport,
    family: ApplicationNetworkFamily,
) -> Result<(), ApplicationTransportError> {
    if *selection != ApplicationNetworkTransport::Udp && family == ApplicationNetworkFamily::Qw {
        return Err(ApplicationTransportError::QuakeWorldRequiresUdp);
    }
    Ok(())
}

/// IPX game payload for a family (donor `openApplicationTransport` mapping).
fn ipx_game(family: ApplicationNetworkFamily) -> IpxGame {
    match family {
        ApplicationNetworkFamily::Q1 => IpxGame::Quake1,
        ApplicationNetworkFamily::Q2 => IpxGame::Quake2,
        ApplicationNetworkFamily::Qw | ApplicationNetworkFamily::Q3 => IpxGame::Quake3,
    }
}

/// Parse `NETWORK.NODE[:SOCKET]` IPX text (donor `parseIpxRemote`).
pub fn parse_ipx_remote(text: &str, default_port: u32) -> Result<NetworkAddress, ApplicationTransportError> {
    let body = text.strip_prefix("ipx:").unwrap_or(text);
    if body.len() < 8 + 1 + 12 {
        return Err(ApplicationTransportError::BadIpxText);
    }
    let (network_text, rest) = body.split_at(8);
    if !network_text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApplicationTransportError::BadIpxText);
    }
    let rest = rest
        .strip_prefix('.')
        .or_else(|| rest.strip_prefix(':'))
        .ok_or(ApplicationTransportError::BadIpxText)?;
    if rest.len() < 12 {
        return Err(ApplicationTransportError::BadIpxText);
    }
    let (node_text, port_text) = rest.split_at(12);
    if !node_text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApplicationTransportError::BadIpxText);
    }
    let port = if port_text.is_empty() {
        default_port
    } else {
        let digits = port_text
            .strip_prefix(':')
            .ok_or(ApplicationTransportError::BadIpxText)?;
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ApplicationTransportError::BadIpxText);
        }
        digits.parse::<u32>().map_err(|_| AddressError::BadPort)?
    };
    let network = u32::from_str_radix(network_text, 16).map_err(|_| ApplicationTransportError::BadIpxText)?;
    let mut node = [0u8; 6];
    for (index, slot) in node.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&node_text[index * 2..index * 2 + 2], 16)
            .map_err(|_| ApplicationTransportError::BadIpxText)?;
    }
    Ok(ipx_address(network, node, port)?)
}

/// Resolve an application address (donor `resolveApplicationAddress`).
pub fn resolve_application_address(
    text: &str,
    default_port: u32,
    selection: &ApplicationNetworkTransport,
    family: ApplicationNetworkFamily,
) -> Result<NetworkAddress, ApplicationTransportError> {
    check_family(selection, family)?;
    if *selection == ApplicationNetworkTransport::Udp {
        let resolved = if family == ApplicationNetworkFamily::Q3 {
            ResolveFamily::V4
        } else {
            ResolveFamily::Any
        };
        Ok(resolve_address(text, default_port, resolved)?)
    } else {
        parse_ipx_remote(text, default_port)
    }
}

/// Socket owner (donor `SocketOwner`).
enum SocketOwner {
    /// UDP socket.
    Udp(ApplicationUdpSocket),
    /// IPX socket with its optional DOSBox network registration.
    Ipx {
        /// Game socket.
        socket: IpxSocket,
        /// DOSBox network; `None` for native IPX.
        network: Option<Arc<DosBoxIpxNetwork>>,
    },
}

/// Application transport (donor `ApplicationTransport`).
pub struct ApplicationTransport {
    owner: SocketOwner,
}

impl ApplicationTransport {
    /// Borrow the UDP socket (donor `udpSocket`).
    pub fn udp_socket(&self) -> Result<ApplicationUdpSocket, ApplicationTransportError> {
        match &self.owner {
            SocketOwner::Udp(socket) => Ok(Arc::clone(socket)),
            SocketOwner::Ipx { .. } => Err(ApplicationTransportError::RequiresUdp),
        }
    }

    /// Open a SOCKS association on a UDP transport (donor `connectSocks`).
    pub fn connect_socks(&self, options: &SocksOptions) -> Result<(), ApplicationTransportError> {
        match &self.owner {
            SocketOwner::Udp(socket) => Ok(socket.connect_socks(options)?),
            SocketOwner::Ipx { .. } => Err(ApplicationTransportError::SocksRequiresUdp),
        }
    }
}

impl DatagramTransport for ApplicationTransport {
    type Address = ApplicationNetworkAddress;

    fn address(&self) -> ApplicationNetworkAddress {
        match &self.owner {
            SocketOwner::Udp(socket) => socket.address(),
            SocketOwner::Ipx { socket, .. } => socket.address(),
        }
    }

    fn closed(&self) -> bool {
        match &self.owner {
            SocketOwner::Udp(socket) => socket.closed(),
            SocketOwner::Ipx { socket, .. } => socket.closed(),
        }
    }

    fn max_datagram_bytes(&self) -> Option<usize> {
        let ceiling = match &self.owner {
            SocketOwner::Udp(socket) => socket.max_datagram_bytes(),
            SocketOwner::Ipx { socket, .. } => socket.max_datagram_bytes(),
        };
        Some(ceiling.unwrap_or(65507))
    }

    fn send(&self, to: &ApplicationNetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
        match &self.owner {
            SocketOwner::Ipx { socket, .. } => {
                if !matches!(to, NetworkAddress::Ipx { .. }) {
                    return Err(TransportError::Closed(
                        "An IPX transport requires an IPX destination".to_owned(),
                    ));
                }
                socket.send(to, payload)
            }
            SocketOwner::Udp(socket) => {
                if matches!(to, NetworkAddress::Ipx { .. }) {
                    return Err(TransportError::Closed(
                        "An IPX destination requires an explicitly selected IPX transport".to_owned(),
                    ));
                }
                socket.send(to, payload)
            }
        }
    }

    fn poll(&self) -> Result<Option<ReceiveEvent<ApplicationNetworkAddress>>, TransportError> {
        match &self.owner {
            SocketOwner::Udp(socket) => socket.poll(),
            SocketOwner::Ipx { socket, .. } => socket.poll(),
        }
    }

    fn subscribe_readable(&self, listener: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
        match &self.owner {
            SocketOwner::Udp(socket) => socket.subscribe_readable(listener),
            SocketOwner::Ipx { socket, .. } => socket.subscribe_readable(listener),
        }
    }

    fn unsubscribe(&self, token: u64) {
        match &self.owner {
            SocketOwner::Udp(socket) => socket.unsubscribe(token),
            SocketOwner::Ipx { socket, .. } => socket.unsubscribe(token),
        }
    }

    fn close(&self) {
        match &self.owner {
            SocketOwner::Udp(socket) => socket.close(),
            SocketOwner::Ipx { socket, network } => {
                socket.close();
                if let Some(network) = network {
                    network.close();
                }
            }
        }
    }
}

/// Options for [`open_application_transport`].
pub struct OpenApplicationTransportOptions {
    /// Transport selection.
    pub selection: ApplicationNetworkTransport,
    /// Network family.
    pub family: ApplicationNetworkFamily,
    /// Bind host.
    pub host: String,
    /// Bind port.
    pub port: u32,
    /// Receive limits.
    pub limits: DatagramLimits,
}

/// Open an application transport (donor `openApplicationTransport`).
pub fn open_application_transport(
    options: &OpenApplicationTransportOptions,
    capabilities: ApplicationTransportCapabilities,
) -> Result<ApplicationTransport, ApplicationTransportError> {
    check_family(&options.selection, options.family)?;
    match &options.selection {
        ApplicationNetworkTransport::Udp => {
            let mut bind = UdpBindOptions::new(&options.host, options.port);
            bind.limits = options.limits;
            let socket = (capabilities.bind_udp)(&bind)?;
            Ok(ApplicationTransport {
                owner: SocketOwner::Udp(Arc::new(socket)),
            })
        }
        ApplicationNetworkTransport::IpxNative => {
            let socket = bind_ipx_transport(
                &IpxHost::Native(capabilities.native_ipx),
                ipx_game(options.family),
                options.port,
            )?;
            Ok(ApplicationTransport {
                owner: SocketOwner::Ipx { socket, network: None },
            })
        }
        ApplicationNetworkTransport::IpxDosBox { relay } => {
            let server = resolve_address(relay, 213, ResolveFamily::V4)?;
            if !matches!(server, NetworkAddress::Ipv4 { .. }) {
                return Err(ApplicationTransportError::DosBoxRelayRequiresIpv4);
            }
            let mut bind = UdpBindOptions::new(&options.host, 0);
            bind.limits = DatagramLimits {
                max_bytes: 1424,
                queue_packets: options.limits.queue_packets,
            };
            let udp = Arc::new((capabilities.bind_udp)(&bind)?);
            let network = DosBoxIpxNetwork::connect(udp, server, Duration::from_millis(5000), monotonic_clock())?;
            let socket = match bind_ipx_transport(
                &IpxHost::DosBox(Arc::clone(&network)),
                ipx_game(options.family),
                options.port,
            ) {
                Ok(socket) => socket,
                Err(error) => {
                    network.close();
                    return Err(error.into());
                }
            };
            Ok(ApplicationTransport {
                owner: SocketOwner::Ipx {
                    socket,
                    network: Some(network),
                },
            })
        }
    }
}

/// Convert a platform IPX address to a network address.
fn network_address(address: &PlatformIpxAddress) -> NetworkAddress {
    NetworkAddress::Ipx {
        network: address.network,
        node: address.node,
        port: address.port,
    }
}

/// Native IPX adapter from `qa_platform::ipx` onto the shared
/// [`DatagramTransport`].
struct NativeIpxAdapter {
    address: NetworkAddress,
    limit: usize,
    inner: Mutex<platform_ipx::NativeIpxTransport>,
    subscriptions: Mutex<HashMap<u64, ReadableSubscription>>,
    next_subscription: AtomicU64,
}

impl NativeIpxAdapter {
    /// Wrap a bound native transport.
    fn wrap(transport: platform_ipx::NativeIpxTransport) -> Result<IpxSocket, TransportError> {
        let address = network_address(&transport.address());
        let limit = transport.max_datagram_bytes();
        Ok(Box::new(Self {
            address,
            limit,
            inner: Mutex::new(transport),
            subscriptions: Mutex::new(HashMap::new()),
            next_subscription: AtomicU64::new(0),
        }))
    }

    /// Lock failure as a transport error.
    fn locked(&self) -> Result<std::sync::MutexGuard<'_, platform_ipx::NativeIpxTransport>, TransportError> {
        self.inner
            .lock()
            .map_err(|_| TransportError::Closed("native IPX socket is unavailable".to_owned()))
    }
}

impl DatagramTransport for NativeIpxAdapter {
    type Address = NetworkAddress;

    fn address(&self) -> NetworkAddress {
        self.address.clone()
    }

    fn closed(&self) -> bool {
        self.locked().is_ok_and(|inner| inner.is_closed())
    }

    fn max_datagram_bytes(&self) -> Option<usize> {
        Some(self.limit)
    }

    fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
        let NetworkAddress::Ipx { network, node, port } = to else {
            return Err(TransportError::Closed(
                "An IPX transport requires an IPX destination".to_owned(),
            ));
        };
        let dest = PlatformIpxAddress {
            network: *network,
            node: *node,
            port: *port,
        };
        self.locked()?
            .send(&dest, payload)
            .map_err(|error| TransportError::Closed(error.to_string()))
    }

    fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
        let event = self
            .locked()?
            .poll()
            .map_err(|error| TransportError::Closed(error.to_string()))?;
        Ok(event.map(|event| match event {
            platform_ipx::ReceiveEvent::Packet {
                from,
                payload,
                received_at_ms,
            } => ReceiveEvent::Packet {
                from: network_address(&from),
                payload,
                received_at: received_at_ms,
            },
            platform_ipx::ReceiveEvent::Dropped { reason, from } => ReceiveEvent::Dropped {
                reason: if reason == "overflow" {
                    DropReason::Overflow
                } else {
                    DropReason::Oversize
                },
                from: network_address(&from),
            },
            platform_ipx::ReceiveEvent::Error { error } => ReceiveEvent::Error { error },
        }))
    }

    fn subscribe_readable(&self, listener: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
        let subscription = self
            .locked()?
            .subscribe_readable(move || listener())
            .map_err(|error| TransportError::Closed(error.to_string()))?;
        let token = self.next_subscription.fetch_add(1, Ordering::Relaxed);
        self.subscriptions
            .lock()
            .map_err(|_| TransportError::Closed("native IPX socket is unavailable".to_owned()))?
            .insert(token, subscription);
        Ok(token)
    }

    fn unsubscribe(&self, token: u64) {
        if let Ok(mut subscriptions) = self.subscriptions.lock() {
            subscriptions.remove(&token);
        }
    }

    fn close(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubTransport;

    impl DatagramTransport for StubTransport {
        type Address = NetworkAddress;

        fn address(&self) -> NetworkAddress {
            NetworkAddress::Ipx {
                network: 1,
                node: [2; 6],
                port: 3,
            }
        }

        fn closed(&self) -> bool {
            false
        }

        fn send(&self, _: &NetworkAddress, _: &[u8]) -> Result<bool, TransportError> {
            Ok(true)
        }

        fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
            Ok(None)
        }

        fn subscribe_readable(&self, _: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
            Ok(7)
        }

        fn unsubscribe(&self, _: u64) {}

        fn close(&self) {}
    }

    fn ipx_transport() -> ApplicationTransport {
        ApplicationTransport {
            owner: SocketOwner::Ipx {
                socket: Box::new(StubTransport),
                network: None,
            },
        }
    }

    fn udp_options() -> OpenApplicationTransportOptions {
        OpenApplicationTransportOptions {
            selection: ApplicationNetworkTransport::Udp,
            family: ApplicationNetworkFamily::Q3,
            host: "127.0.0.1".to_string(),
            port: 0,
            limits: DatagramLimits {
                max_bytes: 4096,
                queue_packets: 8,
            },
        }
    }

    #[test]
    fn parse_ipx_remote_accepts_donor_shapes() {
        let parsed = parse_ipx_remote("ipx:00000001.112233445566", 26000).unwrap();
        assert_eq!(
            parsed,
            NetworkAddress::Ipx {
                network: 1,
                node: [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
                port: 26000,
            }
        );
        let parsed = parse_ipx_remote("A1B2C3D4:e5f60718293a:1234", 26000).unwrap();
        assert_eq!(
            parsed,
            NetworkAddress::Ipx {
                network: 0xa1b2c3d4,
                node: [0xe5, 0xf6, 0x07, 0x18, 0x29, 0x3a],
                port: 1234,
            }
        );
        let parsed = parse_ipx_remote("00000001.112233445566", 26000).unwrap();
        assert!(matches!(parsed, NetworkAddress::Ipx { port: 26000, .. }));
    }

    #[test]
    fn parse_ipx_remote_rejects_malformed_text() {
        for text in [
            "",
            "xyz",
            "ipx:0000001.112233445566",
            "00000001.11223344556",
            "0000000G.112233445566",
            "00000001-112233445566",
            "00000001.1122334455667",
            "00000001.112233445566:abc",
            "00000001.112233445566:",
            "00000001.112233445566:12x4",
        ] {
            assert!(
                matches!(
                    parse_ipx_remote(text, 26000),
                    Err(ApplicationTransportError::BadIpxText)
                ),
                "unexpected accept: {text}"
            );
        }
        assert_eq!(
            ApplicationTransportError::BadIpxText.to_string(),
            "IPX address must be NETWORK.NODE[:SOCKET] with 8 and 12 hexadecimal digits"
        );
    }

    #[test]
    fn parse_ipx_remote_rejects_out_of_range_ports() {
        assert!(matches!(
            parse_ipx_remote("00000001.112233445566:99999999999", 26000),
            Err(ApplicationTransportError::Address(AddressError::BadPort))
        ));
    }

    #[test]
    fn quakeworld_rejects_ipx() {
        for selection in [
            ApplicationNetworkTransport::IpxNative,
            ApplicationNetworkTransport::IpxDosBox {
                relay: "relay".to_string(),
            },
        ] {
            assert!(matches!(
                resolve_application_address("00000001.112233445566", 26000, &selection, ApplicationNetworkFamily::Qw),
                Err(ApplicationTransportError::QuakeWorldRequiresUdp)
            ));
            assert!(matches!(
                open_application_transport(
                    &OpenApplicationTransportOptions {
                        selection,
                        family: ApplicationNetworkFamily::Qw,
                        host: "127.0.0.1".to_string(),
                        port: 0,
                        limits: DatagramLimits {
                            max_bytes: 1424,
                            queue_packets: 8,
                        },
                    },
                    host_application_transports(),
                ),
                Err(ApplicationTransportError::QuakeWorldRequiresUdp)
            ));
        }
        assert_eq!(
            ApplicationTransportError::QuakeWorldRequiresUdp.to_string(),
            "QuakeWorld uses UDP; IPX is not a QuakeWorld transport"
        );
    }

    #[test]
    fn resolve_application_address_routes_by_selection() {
        let udp = resolve_application_address(
            "127.0.0.1",
            27960,
            &ApplicationNetworkTransport::Udp,
            ApplicationNetworkFamily::Q3,
        )
        .unwrap();
        assert!(matches!(udp, NetworkAddress::Ipv4 { .. }));
        let ipx = resolve_application_address(
            "00000001.112233445566",
            26000,
            &ApplicationNetworkTransport::IpxNative,
            ApplicationNetworkFamily::Q2,
        )
        .unwrap();
        assert!(matches!(ipx, NetworkAddress::Ipx { .. }));
    }

    #[test]
    fn open_udp_binds_and_closes() {
        let transport = open_application_transport(&udp_options(), host_application_transports()).unwrap();
        assert!(matches!(transport.address(), NetworkAddress::Ipv4 { .. }));
        assert!(!transport.closed());
        assert!(transport.max_datagram_bytes().is_some());
        let socket = transport.udp_socket().unwrap();
        assert_eq!(socket.address(), transport.address());
        let local = transport.address();
        assert!(transport.send(&local, &[1, 2, 3]).is_ok());
        assert!(transport.poll().is_ok());
        assert!(matches!(
            transport.send(
                &NetworkAddress::Ipx {
                    network: 0,
                    node: [0; 6],
                    port: 1,
                },
                &[]
            ),
            Err(TransportError::Closed(message))
                if message == "An IPX destination requires an explicitly selected IPX transport"
        ));
        transport.close();
        assert!(transport.closed());
    }

    #[test]
    fn open_udp_propagates_bind_failures() {
        let capabilities = ApplicationTransportCapabilities {
            native_ipx: NativeIpxCapability::Unavailable {
                reason: "test".to_string(),
            },
            bind_udp: Box::new(|_| Err(TransportError::BadPort)),
        };
        assert!(matches!(
            open_application_transport(&udp_options(), capabilities),
            Err(ApplicationTransportError::Transport(TransportError::BadPort))
        ));
    }

    #[test]
    fn ipx_owner_rejects_udp_operations() {
        let transport = ipx_transport();
        assert!(matches!(
            transport.udp_socket(),
            Err(ApplicationTransportError::RequiresUdp)
        ));
        assert_eq!(
            ApplicationTransportError::RequiresUdp.to_string(),
            "The selected source requires a UDP transport"
        );
        assert!(matches!(
            transport.connect_socks(&SocksOptions {
                server: "proxy".to_string(),
                port: 1080,
                username: String::new(),
                password: String::new(),
            }),
            Err(ApplicationTransportError::SocksRequiresUdp)
        ));
        assert_eq!(
            ApplicationTransportError::SocksRequiresUdp.to_string(),
            "SOCKS proxying is not available for the selected IPX transport"
        );
        assert!(matches!(
            transport.send(
                &NetworkAddress::Ipv4 {
                    host: [127, 0, 0, 1],
                    port: 27960,
                },
                &[]
            ),
            Err(TransportError::Closed(message))
                if message == "An IPX transport requires an IPX destination"
        ));
        assert_eq!(transport.max_datagram_bytes(), Some(65507));
        assert!(transport.poll().unwrap().is_none());
        transport.close();
    }

    #[test]
    fn host_capabilities_probe_without_binding() {
        let capabilities = host_application_transports();
        match &capabilities.native_ipx {
            NativeIpxCapability::Unavailable { reason } => assert!(!reason.is_empty()),
            NativeIpxCapability::Available(_) => {}
        }
    }
}
