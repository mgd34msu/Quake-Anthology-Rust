//! Q3 net_chan.c loopback_t: two fixed 16-message in-process directions.
use crate::primitives::ClientId;
pub const PACKET_BYTES: usize = 1400;
pub const MESSAGES: usize = 16;

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

#[derive(Clone, Copy)]
struct Message {
    bytes: [u8; PACKET_BYTES],
    length: u16,
    client: ClientId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet<'a> {
    pub client: ClientId,
    pub bytes: &'a [u8],
}
struct Ring {
    messages: Box<[Message; MESSAGES]>,
    head: usize,
    len: usize,
    overwritten: u64,
}
pub struct Loopback {
    rings: [Ring; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    PacketTooLarge,
}

impl Default for Loopback {
    fn default() -> Self {
        Self::load()
    }
}
impl Loopback {
    pub fn load() -> Self {
        Self {
            rings: std::array::from_fn(|_| Ring {
                messages: Box::new(
                    [Message {
                        bytes: [0; PACKET_BYTES],
                        length: 0,
                        client: ClientId(0),
                    }; MESSAGES],
                ),
                head: 0,
                len: 0,
                overwritten: 0,
            }),
        }
    }

    /// Send to the opposite endpoint, just as loopbacks[sock ^ 1] does.
    /// Oversize packets leave the ring untouched; netchan fragments upstream.
    pub fn send(
        &mut self,
        from: Endpoint,
        client: ClientId,
        bytes: &[u8],
    ) -> Result<(), SendError> {
        if bytes.len() > PACKET_BYTES {
            return Err(SendError::PacketTooLarge);
        }
        let ring = &mut self.rings[from.index() ^ 1];
        if ring.len == MESSAGES {
            ring.head = (ring.head + 1) & (MESSAGES - 1);
            ring.len -= 1;
            ring.overwritten = ring.overwritten.saturating_add(1);
        }
        let slot = (ring.head + ring.len) & (MESSAGES - 1);
        let message = &mut ring.messages[slot];
        message.bytes[..bytes.len()].copy_from_slice(bytes);
        message.length = bytes.len() as u16;
        message.client = client;
        ring.len += 1;
        Ok(())
    }

    /// Borrow lasts until the next mutable operation, so slot reuse cannot
    /// invalidate a consumer's packet view.
    pub fn receive(&mut self, to: Endpoint) -> Option<Packet<'_>> {
        let ring = &mut self.rings[to.index()];
        if ring.len == 0 {
            return None;
        }
        let slot = ring.head;
        ring.head = (ring.head + 1) & (MESSAGES - 1);
        ring.len -= 1;
        let message = &ring.messages[slot];
        Some(Packet {
            client: message.client,
            bytes: &message.bytes[..usize::from(message.length)],
        })
    }

    pub fn pending(&self, to: Endpoint) -> usize {
        self.rings[to.index()].len
    }
    pub fn overwritten(&self, to: Endpoint) -> u64 {
        self.rings[to.index()].overwritten
    }
    pub fn clear(&mut self) {
        for ring in &mut self.rings {
            ring.head = 0;
            ring.len = 0;
            ring.overwritten = 0;
        }
    }
}
