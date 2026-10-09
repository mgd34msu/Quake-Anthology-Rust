use crate::{
    Window,
    clock::{Clock, Stopwatch},
};
use qa_core::sys_events::{EventKind, EventTime, QueueError, SysEvent, SysEventQueue};
use std::{
    io,
    net::{SocketAddr, UdpSocket},
    time::Duration,
};

/// Collects physical input only at the two host event drains.
pub struct EventPump {
    clock: Clock,
    timer: Stopwatch,
    sockets: Vec<UdpSocket>,
    datagram: Box<[u8]>,
    dropped_packets: u64,
    socket_errors: u64,
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
            sockets: Vec::with_capacity(8),
            datagram: vec![0; 65536].into_boxed_slice(),
            dropped_packets: 0,
            socket_errors: 0,
            stdin: crate::stdin::ConsoleInput::open(),
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
    pub fn begin_frame(&mut self) -> EventTime {
        self.timer = Stopwatch::start();
        self.clock.now()
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
    pub fn wait_time(&self, remaining: Duration) -> EventTime {
        crate::clock::pause(remaining);
        self.clock.now()
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
