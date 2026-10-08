//! System-event packet boundary. Protocol/channel consumers enter in R11.
use qa_core::sys_events::EventTime;
use std::net::SocketAddr;

#[derive(Default)]
pub struct PacketReceiver {
    pub packets: u64,
    pub bytes: u64,
    pub last_time: EventTime,
    pub last_from: Option<SocketAddr>,
}
impl PacketReceiver {
    pub fn receive(&mut self, _socket: u16, from: SocketAddr, bytes: &[u8], time: EventTime) {
        self.packets = self.packets.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes.len() as u64);
        self.last_time = time;
        self.last_from = Some(from);
    }
}
