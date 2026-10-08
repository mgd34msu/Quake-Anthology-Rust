//! System-event packet boundary. Protocol/channel consumers enter in R11.
use qa_core::primitives::ClientId;
use qa_core::sys_events::EventTime;
use std::net::SocketAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Peer {
    Socket(SocketAddr),
    Loopback(ClientId),
}
impl From<SocketAddr> for Peer {
    fn from(address: SocketAddr) -> Self {
        Self::Socket(address)
    }
}

#[derive(Default)]
pub struct PacketReceiver {
    pub packets: u64,
    pub bytes: u64,
    pub last_time: EventTime,
    pub last_from: Option<Peer>,
    pub last_socket: Option<u16>,
}
impl PacketReceiver {
    pub fn receive(&mut self, socket: u16, from: impl Into<Peer>, bytes: &[u8], time: EventTime) {
        self.packets = self.packets.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes.len() as u64);
        self.last_time = time;
        self.last_from = Some(from.into());
        self.last_socket = Some(socket);
    }
}
