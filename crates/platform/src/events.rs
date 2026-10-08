use crate::{
    Window,
    clock::{Clock, Stopwatch},
};
use qa_core::sys_events::{EventKind, QueueError, SysEvent, SysEventQueue};
use std::{
    io,
    net::{SocketAddr, UdpSocket},
    time::Duration,
};

/// One nonblocking poll at the top of the host frame; no receive thread.
pub struct EventPump {
    clock: Clock,
    timer: Stopwatch,
    sockets: Vec<UdpSocket>,
    datagram: Box<[u8]>,
    dropped_packets: u64,
    socket_errors: u64,
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
            sockets: Vec::with_capacity(8),
            datagram: vec![0; 65536].into_boxed_slice(),
            dropped_packets: 0,
            socket_errors: 0,
        }
    }
    pub fn bind_udp(&mut self, address: SocketAddr) -> io::Result<(u16, SocketAddr)> {
        if self.sockets.len() == 8 {
            return Err(io::Error::from(io::ErrorKind::OutOfMemory));
        }
        let socket = UdpSocket::bind(address)?;
        socket.set_nonblocking(true)?;
        let local = socket.local_addr()?;
        let index = self.sockets.len() as u16;
        self.sockets.push(socket);
        Ok((index, local))
    }
    pub fn begin_frame(&mut self, window: &mut Window, queue: &mut SysEventQueue) {
        self.timer = Stopwatch::start();
        window.poll(queue, &self.clock);
        self.poll_network(queue);
        self.finish_events(queue);
    }
    /// Also used by headless socket qualification without opening SDL.
    pub fn poll_network(&mut self, queue: &mut SysEventQueue) {
        for (index, socket) in self.sockets.iter().enumerate() {
            for _ in 0..64 {
                match socket.recv_from(&mut self.datagram) {
                    Ok((length, from)) => {
                        if queue.len() + 1 >= queue.capacity()
                            || queue
                                .push(SysEvent {
                                    time: self.clock.now(),
                                    kind: EventKind::Packet {
                                        socket: index as u16,
                                        from,
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
    pub fn pace(&self, period: Duration) {
        crate::clock::pause(period.saturating_sub(self.elapsed()));
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
