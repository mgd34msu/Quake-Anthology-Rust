use crate::{
    Window,
    clock::{Clock, Stopwatch},
};
use qa_core::{
    loopback::Endpoint,
    sys_events::{EventKind, QueueError, SysEvent, SysEventQueue},
};
use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{Arc, Weak},
    time::Duration,
};

/// The CLIENT resource lifetime owns this socket. The event source retains
/// only a weak reference and never exposes its descriptor or shared bytes.
#[must_use]
pub struct UdpLease {
    token: u16,
    _socket: Arc<UdpSocket>,
}
impl UdpLease {
    pub fn socket(&self) -> u16 {
        self.token
    }
}

enum SocketOwner {
    Bound(Arc<UdpSocket>),
    Leased(Weak<UdpSocket>),
}
impl SocketOwner {
    fn borrow(&self) -> Option<Arc<UdpSocket>> {
        match self {
            Self::Bound(socket) => Some(Arc::clone(socket)),
            Self::Leased(socket) => socket.upgrade(),
        }
    }
    fn is_live(&self) -> bool {
        match self {
            Self::Bound(_) => true,
            Self::Leased(socket) => socket.strong_count() != 0,
        }
    }
}
struct SocketSlot {
    token: u16,
    owner: SocketOwner,
}

fn bind_socket(address: SocketAddr) -> io::Result<Arc<UdpSocket>> {
    let socket = UdpSocket::bind(address)?;
    socket.set_nonblocking(true)?;
    Ok(Arc::new(socket))
}

/// Collects physical input only at the two host event drains.
pub struct EventPump {
    clock: Clock,
    timer: Stopwatch,
    sockets: [Option<SocketSlot>; 8],
    next_socket: u32,
    datagram: Box<[u8]>,
    dropped_packets: u64,
    socket_errors: u64,
    socket_submission_enabled: bool,
    stdin: crate::stdin::ConsoleInput,
}
impl Default for EventPump {
    fn default() -> Self {
        Self::new()
    }
}
impl EventPump {
    pub fn new() -> Self {
        Self {
            clock: Clock::new(),
            timer: Stopwatch::start(),
            sockets: std::array::from_fn(|_| None),
            next_socket: 0,
            datagram: vec![0; 65536].into_boxed_slice(),
            dropped_packets: 0,
            socket_errors: 0,
            socket_submission_enabled: true,
            stdin: crate::stdin::ConsoleInput::open(),
        }
    }
    pub fn bind_udp(&mut self, address: SocketAddr) -> io::Result<(u16, SocketAddr)> {
        let socket = bind_socket(address)?;
        let local = socket.local_addr()?;
        let token = self.install_socket(SocketOwner::Bound(socket))?;
        Ok((token, local))
    }
    /// Cold address resolution and binding. Requests and connected datagrams
    /// continue through send_udp and the two existing physical event intakes.
    pub fn open_udp_client(
        &mut self,
        target: &str,
        default_port: u16,
    ) -> io::Result<(UdpLease, SocketAddr)> {
        if target.is_empty() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        if let Ok(address) = target.parse::<SocketAddr>() {
            return self.open_client_addresses([address]);
        }
        if let Ok(address) = target.parse::<IpAddr>() {
            return self.open_client_addresses([SocketAddr::new(address, default_port)]);
        }
        let addresses = target
            .to_socket_addrs()
            .or_else(|_| format!("{target}:{default_port}").to_socket_addrs())?;
        self.open_client_addresses(addresses)
    }
    fn open_client_addresses(
        &mut self,
        addresses: impl IntoIterator<Item = SocketAddr>,
    ) -> io::Result<(UdpLease, SocketAddr)> {
        let mut failure = io::Error::from(io::ErrorKind::AddrNotAvailable);
        for peer in addresses {
            if peer.port() == 0 {
                failure = io::ErrorKind::InvalidInput.into();
                continue;
            }
            let local = SocketAddr::new(
                match peer.ip() {
                    IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                    IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
                },
                0,
            );
            let socket = match bind_socket(local) {
                Ok(socket) => socket,
                Err(error) => {
                    failure = error;
                    continue;
                }
            };
            let token = self.install_socket(SocketOwner::Leased(Arc::downgrade(&socket)))?;
            return Ok((
                UdpLease {
                    token,
                    _socket: socket,
                },
                peer,
            ));
        }
        Err(failure)
    }
    fn install_socket(&mut self, owner: SocketOwner) -> io::Result<u16> {
        // Physical tokens never repeat, including after their storage slot is
        // reused. Queued packets therefore cannot acquire a new CLIENT lifetime.
        if self.next_socket >= u32::from(Endpoint::Server.socket()) {
            return Err(io::ErrorKind::OutOfMemory.into());
        }
        let slot = self
            .sockets
            .iter()
            .position(|entry| entry.as_ref().is_none_or(|entry| !entry.owner.is_live()))
            .ok_or(io::ErrorKind::OutOfMemory)?;
        let token = self.next_socket as u16;
        self.sockets[slot] = Some(SocketSlot { token, owner });
        self.next_socket += 1;
        Ok(token)
    }
    pub fn begin_frame(&mut self) -> Duration {
        self.timer = Stopwatch::start();
        Duration::from_nanos(self.clock.now().0)
    }
    /// Submission never polls a receive source or the clock.
    pub fn send_udp(&self, socket: u16, to: SocketAddr, bytes: &[u8]) -> io::Result<()> {
        let socket = self
            .sockets
            .iter()
            .flatten()
            .find(|entry| entry.token == socket)
            .and_then(|entry| entry.owner.borrow())
            .ok_or(io::ErrorKind::InvalidInput)?;
        if !self.socket_submission_enabled {
            return Ok(());
        }
        let sent = socket.send_to(bytes, to)?;
        if sent == bytes.len() {
            Ok(())
        } else {
            Err(io::ErrorKind::WriteZero.into())
        }
    }
    pub fn set_socket_submission_enabled(&mut self, enabled: bool) {
        self.socket_submission_enabled = enabled;
    }
    pub fn poll_events(&mut self, window: &mut Window, queue: &mut SysEventQueue) {
        window.poll(queue, &self.clock);
        self.poll_console(queue);
        self.poll_network(queue);
        self.finish_events(queue);
    }
    /// Also used by headless socket qualification without opening SDL.
    pub fn poll_console(&mut self, queue: &mut SysEventQueue) {
        self.stdin.poll(&self.clock, queue);
    }
    pub fn console_lines(&self) -> u64 {
        self.stdin.lines
    }
    pub fn discarded_console_lines(&self) -> u64 {
        self.stdin.discarded
    }
    pub fn console_errors(&self) -> u64 {
        self.stdin.errors
    }

    /// Also used by headless socket qualification without opening SDL.
    pub fn poll_network(&mut self, queue: &mut SysEventQueue) {
        for entry in self.sockets.iter().flatten() {
            let Some(socket) = entry.owner.borrow() else {
                continue;
            };
            for _ in 0..64 {
                match socket.recv_from(&mut self.datagram) {
                    Ok((length, from)) => {
                        if queue
                            .push(SysEvent {
                                time: self.clock.now(),
                                kind: EventKind::Packet {
                                    socket: entry.token,
                                    from: from.into(),
                                    bytes: &self.datagram[..length],
                                },
                            })
                            .is_err()
                        {
                            self.dropped_packets = self.dropped_packets.saturating_add(1);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        self.socket_errors = self.socket_errors.saturating_add(1);
                        break;
                    }
                }
            }
        }
    }
    fn finish_events(&self, queue: &mut SysEventQueue) {
        // SDL polling and UDP admission reserve this last slot.
        let _ = queue.push(SysEvent {
            time: self.clock.now(),
            kind: EventKind::Time,
        });
    }
    pub fn elapsed(&self) -> Duration {
        self.timer.elapsed()
    }
    /// Clock-only cap wait. Event admission belongs to poll_events, never here.
    pub fn wait_time(&self, remaining: Duration) -> Duration {
        crate::clock::pause(remaining);
        Duration::from_nanos(self.clock.now().0)
    }
    pub fn dropped_packets(&self) -> u64 {
        self.dropped_packets
    }
    pub fn socket_errors(&self) -> u64 {
        self.socket_errors
    }
    pub fn enqueue(
        &self,
        queue: &mut SysEventQueue,
        kind: EventKind<'_>,
    ) -> Result<(), QueueError> {
        queue.push(SysEvent {
            time: self.clock.now(),
            kind,
        })
    }
}
