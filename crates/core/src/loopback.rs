//! Per-client local transport. Admission is bounded and never overwrites data.
use crate::{
    payloads::PayloadQueue,
    primitives::ClientId,
    sys_events::{EventKind, EventTime, Peer, QueueError, SysEvent, SysEventQueue},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Client,
    Server,
}
impl Endpoint {
    pub const fn socket(self) -> u16 {
        match self {
            Self::Client => u16::MAX,
            Self::Server => u16::MAX - 1,
        }
    }
    fn index(self) -> usize {
        match self {
            Self::Client => 0,
            Self::Server => 1,
        }
    }
}

/// Supplied by the connection boundary at load, from its negotiated limits.
#[derive(Clone, Copy, Debug)]
pub struct LoopbackLimits {
    pub maximum_message: usize,
    pub payload_bytes: usize,
    pub messages: usize,
}
struct Ring {
    payloads: PayloadQueue<u64>,
    full: u64,
}
struct Client {
    maximum_message: usize,
    rings: [Ring; 2],
}
pub struct Loopback {
    clients: Box<[Client]>,
    sequence: [u64; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    Client,
    PacketTooLarge,
    Full,
}

impl Loopback {
    pub fn load(limits: impl IntoIterator<Item = LoopbackLimits>) -> Result<Self, QueueError> {
        let clients = limits
            .into_iter()
            .map(|limits| {
                if limits.maximum_message == 0 || limits.maximum_message > limits.payload_bytes {
                    return Err(QueueError::Capacity);
                }
                Ok(Client {
                    maximum_message: limits.maximum_message,
                    rings: [
                        Ring {
                            payloads: PayloadQueue::load(limits.messages, limits.payload_bytes)?,
                            full: 0,
                        },
                        Ring {
                            payloads: PayloadQueue::load(limits.messages, limits.payload_bytes)?,
                            full: 0,
                        },
                    ],
                })
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice();
        if clients.len() > u32::MAX as usize {
            return Err(QueueError::Capacity);
        }
        Ok(Self {
            clients,
            sequence: [0; 2],
        })
    }

    /// Send to the opposite endpoint. A Full sender retains its own message
    /// for retry; loss simulation and native reliability belong to netchan.
    pub fn send(
        &mut self,
        from: Endpoint,
        client: ClientId,
        bytes: &[u8],
    ) -> Result<(), SendError> {
        let client = self
            .clients
            .get_mut(client.0 as usize)
            .ok_or(SendError::Client)?;
        if bytes.len() > client.maximum_message {
            return Err(SendError::PacketTooLarge);
        }
        let to = from.index() ^ 1;
        let ring = &mut client.rings[to];
        if ring.payloads.push(self.sequence[to], bytes, 0).is_err() {
            ring.full = ring.full.saturating_add(1);
            return Err(SendError::Full);
        }
        self.sequence[to] = self.sequence[to].wrapping_add(1);
        Ok(())
    }

    /// Transfer in endpoint/send order. A rejected event leaves its packet
    /// pending. Only SysEventQueue dispatch exposes packets to the network.
    /// This is queued memory consumption, never another physical intake.
    pub fn enqueue(&mut self, queue: &mut SysEventQueue, time: EventTime) -> usize {
        let mut admitted = 0;
        for to in [Endpoint::Client, Endpoint::Server] {
            let direction = to.index();
            loop {
                let next = self
                    .clients
                    .iter()
                    .enumerate()
                    .filter_map(|(index, client)| {
                        let (sequence, _) = client.rings[direction].payloads.front()?;
                        Some((index, sequence.wrapping_sub(self.sequence[direction])))
                    })
                    .min_by_key(|&(_, sequence)| sequence)
                    .map(|(index, _)| index);
                let Some(index) = next else { break };
                let ring = &mut self.clients[index].rings[direction];
                let Some((_, bytes)) = ring.payloads.front() else {
                    break;
                };
                if queue
                    .push(SysEvent {
                        time,
                        kind: EventKind::Packet {
                            socket: to.socket(),
                            from: Peer::Loopback(ClientId(index as u32)),
                            bytes,
                        },
                    })
                    .is_err()
                {
                    return admitted;
                }
                ring.payloads.pop();
                admitted += 1;
            }
        }
        admitted
    }

    pub fn pending(&self, to: Endpoint) -> usize {
        self.clients
            .iter()
            .map(|client| client.rings[to.index()].payloads.len())
            .sum()
    }
    pub fn pending_client(&self, to: Endpoint, client: ClientId) -> Option<usize> {
        Some(
            self.clients.get(client.0 as usize)?.rings[to.index()]
                .payloads
                .len(),
        )
    }
    pub fn full(&self, to: Endpoint, client: ClientId) -> Option<u64> {
        Some(self.clients.get(client.0 as usize)?.rings[to.index()].full)
    }
    pub fn clear_client(&mut self, client: ClientId) {
        if let Some(client) = self.clients.get_mut(client.0 as usize) {
            for ring in &mut client.rings {
                ring.payloads.clear();
                ring.full = 0;
            }
        }
    }
    pub fn clear(&mut self) {
        for client in &mut self.clients {
            for ring in &mut client.rings {
                ring.payloads.clear();
                ring.full = 0;
            }
        }
        self.sequence = [0; 2];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn internal_send_order_survives_sequence_wrap() -> Result<(), &'static str> {
        let mut local = Loopback::load(
            [LoopbackLimits {
                maximum_message: 8,
                payload_bytes: 32,
                messages: 4,
            }; 2],
        )
        .map_err(|_| "local capacity")?;
        local.sequence = [u64::MAX - 1; 2];
        for (id, bytes) in [
            (1, b"one".as_slice()),
            (0, b"two".as_slice()),
            (1, b"three".as_slice()),
        ] {
            local
                .send(Endpoint::Server, ClientId(id), bytes)
                .map_err(|_| "local send")?;
        }
        let mut queue = SysEventQueue::load(8, 32).map_err(|_| "queue capacity")?;
        assert_eq!(local.enqueue(&mut queue, EventTime(0)), 3);
        for expected in [b"one".as_slice(), b"two".as_slice(), b"three".as_slice()] {
            assert!(
                matches!(queue.pop().ok_or("event")?.kind, EventKind::Packet { bytes, .. } if bytes == expected)
            );
        }
        Ok(())
    }
}
