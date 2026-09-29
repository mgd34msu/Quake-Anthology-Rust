//! KEX LAN channel: compression, fragmentation, and reliable delivery.
//!
//! Donor provenance: `KexMessage` and `KexChannel` in
//! `src/network/q2/kex/channel.ts`. Compression uses `flate2` zlib (the
//! donor's `node:zlib` `deflateSync` / `inflateSync`); the emit callback
//! is a boxed `FnMut` sink; `tick(now)` stays explicit.

use std::collections::VecDeque;
use std::io::Read;

use flate2::read::{ZlibDecoder, ZlibEncoder};
use flate2::Compression;

use crate::q2_kex_packet::{
    read_kex_packet, write_kex_packet, KexError, KexPacket, KEX_DATAGRAM_BYTES, KEX_MESSAGE_BYTES,
};

/// First-fragment payload capacity of a sequenced datagram.
const FIRST_FRAGMENT_BYTES: usize = KEX_DATAGRAM_BYTES - 7;
/// Retransmit interval in milliseconds.
const RETRY_MILLISECONDS: f64 = 500.0;
/// Retransmit attempts before the peer times out.
const MAX_RETRIES: u32 = 40;
/// Keepalive interval in milliseconds.
const KEEPALIVE_MILLISECONDS: f64 = 5000.0;
/// Compressed game-message kind.
const COMPRESSED_KIND: u8 = 127;
/// Acknowledgement kind.
const ACK_KIND: u8 = 130;
/// Keepalive kind.
const KEEPALIVE_KIND: u8 = 129;

/// Send mode (`0 | 2 | 3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KexMode {
    /// Unsequenced.
    Unsequenced = 0,
    /// Sequenced.
    Sequential = 2,
    /// Sequenced and reliable.
    Reliable = 3,
}

impl KexMode {
    /// Wire flag bits.
    fn bits(self) -> u8 {
        self as u8
    }
}

/// Received message (`KexMessage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KexMessage {
    /// Message kind.
    pub kind: u8,
    /// Payload bytes.
    pub payload: Vec<u8>,
}

/// Unacknowledged reliable datagram.
struct Pending {
    reliable: u16,
    bytes: Vec<u8>,
}

/// Partial reassembly state.
struct Fragments {
    kind: u8,
    sequence: u16,
    bytes: Vec<u8>,
}

/// Compress a payload (donor `deflateSync` at the default level).
fn deflate(payload: &[u8]) -> Result<Vec<u8>, KexError> {
    let mut encoder = ZlibEncoder::new(payload, Compression::default());
    let mut out = Vec::new();
    encoder
        .read_to_end(&mut out)
        .map_err(|error| KexError::Message(error.to_string()))?;
    Ok(out)
}

/// Decompress a payload, enforcing the message cap (donor
/// `inflateSync` with `maxOutputLength`).
fn inflate(data: &[u8]) -> Result<Vec<u8>, KexError> {
    let decoder = ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .take(KEX_MESSAGE_BYTES as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|error| KexError::Message(error.to_string()))?;
    if out.len() > KEX_MESSAGE_BYTES {
        return Err(KexError::Protocol("KEX decompressed message exceeds limit"));
    }
    Ok(out)
}

/// KEX datagram emit sink.
pub type KexEmit = Box<dyn FnMut(&[u8]) -> bool + Send>;

/// KEX channel (`KexChannel`).
pub struct KexChannel {
    emit: KexEmit,
    sequence: u16,
    reliable: u16,
    incoming_sequence: u16,
    incoming_reliable: u16,
    pending: VecDeque<Pending>,
    pending_bytes: usize,
    retry_at: f64,
    retries: u32,
    received_at: f64,
    acknowledgment: u8,
    fragments: Option<Fragments>,
}

impl KexChannel {
    /// Build a channel over an emit sink.
    pub fn new(emit: impl FnMut(&[u8]) -> bool + Send + 'static) -> Self {
        Self {
            emit: Box::new(emit),
            sequence: 0,
            reliable: 0,
            incoming_sequence: 0,
            incoming_reliable: 0,
            pending: VecDeque::new(),
            pending_bytes: 0,
            retry_at: 0.0,
            retries: 0,
            received_at: 0.0,
            acknowledgment: 0,
            fragments: None,
        }
    }

    /// Send a message (`send`); returns the emit sink's acceptance.
    pub fn send(&mut self, kind: u8, payload: &[u8], mode: KexMode, now: f64) -> Result<bool, KexError> {
        if payload.len() > KEX_MESSAGE_BYTES {
            return Err(KexError::Protocol("Invalid KEX message"));
        }
        let (message_kind, data) = if kind < COMPRESSED_KIND && payload.len() >= 128 {
            let compressed = deflate(payload)?;
            if compressed.len() + 1 < payload.len() {
                let mut data = Vec::with_capacity(compressed.len() + 1);
                data.push(kind);
                data.extend_from_slice(&compressed);
                (COMPRESSED_KIND, data)
            } else {
                (kind, payload.to_vec())
            }
        } else {
            (kind, payload.to_vec())
        };
        let fragmented = data.len() + if mode == KexMode::Unsequenced { 3 } else { 7 } > KEX_DATAGRAM_BYTES;
        if fragmented && mode == KexMode::Unsequenced {
            return Err(KexError::Protocol("Unsequenced KEX message cannot be fragmented"));
        }
        if mode == KexMode::Reliable
            && self.pending_bytes + data.len() + data.len().div_ceil(FIRST_FRAGMENT_BYTES) * 7 > KEX_MESSAGE_BYTES * 2
        {
            return Err(KexError::Protocol("KEX reliable queue is full"));
        }
        let mut position = 0;
        let mut first = true;
        let mut accepted = true;
        loop {
            let capacity = KEX_DATAGRAM_BYTES - if mode == KexMode::Unsequenced { 2 } else { 6 } - usize::from(first);
            let end = (position + capacity).min(data.len());
            let final_fragment = end == data.len();
            let fragment_flags = if !fragmented {
                0
            } else if first {
                4
            } else if final_fragment {
                12
            } else {
                8
            };
            if mode != KexMode::Unsequenced {
                self.sequence = self.sequence.wrapping_add(1);
            }
            if mode == KexMode::Reliable {
                self.reliable = self.reliable.wrapping_add(1);
            }
            let bytes = write_kex_packet(&KexPacket {
                flags: mode.bits() | fragment_flags,
                sequence: self.sequence,
                reliable: self.reliable,
                kind: if first { Some(message_kind) } else { None },
                payload: data[position..end].to_vec(),
            })?;
            if mode == KexMode::Reliable {
                if self.pending.is_empty() {
                    self.retry_at = now;
                    self.retries = 0;
                }
                self.pending_bytes += bytes.len();
                self.pending.push_back(Pending {
                    reliable: self.reliable,
                    bytes: bytes.clone(),
                });
            }
            accepted = (self.emit)(&bytes) && accepted;
            position = end;
            first = false;
            if position >= data.len() {
                break;
            }
        }
        Ok(accepted)
    }

    /// Receive a datagram (`receive`); returns a completed message, if any.
    pub fn receive(&mut self, bytes: &[u8], now: f64) -> Result<Option<KexMessage>, KexError> {
        let packet = read_kex_packet(bytes)?;
        let reliable = packet.flags & 1 != 0;
        let sequential = packet.flags & 2 != 0;
        if packet.kind == Some(ACK_KIND) && packet.flags == 0 {
            if packet.payload.len() != 3 {
                return Err(KexError::Protocol("Invalid KEX acknowledgment"));
            }
            let acknowledgment = u16::from_be_bytes([packet.payload[1], packet.payload[2]]);
            // The native sender uses an unsigned ordinary comparison,
            // including at wrap.
            while self
                .pending
                .front()
                .is_some_and(|pending| pending.reliable <= acknowledgment)
            {
                if let Some(removed) = self.pending.pop_front() {
                    self.pending_bytes -= removed.bytes.len();
                }
                self.retry_at = now;
                self.retries = 0;
            }
            self.received_at = now;
            return Ok(None);
        }
        if reliable {
            self.acknowledgment |= 4;
            if packet.reliable != self.incoming_reliable.wrapping_add(1) {
                self.acknowledgment |= 2;
                return Ok(None);
            }
            self.incoming_reliable = packet.reliable;
            self.acknowledgment &= !2;
        } else if sequential {
            if packet.sequence == self.incoming_sequence {
                return Ok(None);
            }
            let mut comparison = u32::from(self.incoming_sequence);
            let sequence = u32::from(packet.sequence);
            if (sequence ^ comparison) & 32768 != 0 && sequence & 32768 != 0 {
                comparison <<= 16;
            } else if sequence >= comparison {
                comparison = sequence + 16384;
            }
            if comparison.wrapping_sub(sequence) < 16384 || packet.reliable != self.incoming_reliable {
                return Ok(None);
            }
        }
        if sequential {
            self.incoming_sequence = packet.sequence;
        }
        self.received_at = now;
        match packet.flags & 12 {
            0 => {
                let kind = packet.kind.ok_or(KexError::Protocol("Missing KEX kind"))?;
                Ok(Some(self.expand(kind, &packet.payload)?))
            }
            4 => {
                let kind = packet.kind.ok_or(KexError::Protocol("Missing KEX fragment kind"))?;
                self.fragments = Some(Fragments {
                    kind,
                    sequence: packet.sequence,
                    bytes: packet.payload,
                });
                Ok(None)
            }
            fragment => {
                let Some(previous) = self.fragments.take() else {
                    return Ok(None);
                };
                if packet.sequence != previous.sequence.wrapping_add(1) {
                    return Ok(None);
                }
                let length = previous.bytes.len() + packet.payload.len();
                if length > KEX_MESSAGE_BYTES {
                    return Err(KexError::Protocol("KEX fragmented message exceeds limit"));
                }
                let mut joined = Vec::with_capacity(length);
                joined.extend_from_slice(&previous.bytes);
                joined.extend_from_slice(&packet.payload);
                if fragment == 8 {
                    self.fragments = Some(Fragments {
                        kind: previous.kind,
                        sequence: packet.sequence,
                        bytes: joined,
                    });
                    Ok(None)
                } else {
                    Ok(Some(self.expand(previous.kind, &joined)?))
                }
            }
        }
    }

    /// Expand a possibly compressed message (`expand`).
    fn expand(&self, kind: u8, bytes: &[u8]) -> Result<KexMessage, KexError> {
        if kind != COMPRESSED_KIND {
            return Ok(KexMessage {
                kind,
                payload: bytes.to_vec(),
            });
        }
        let Some(original) = bytes.first().copied() else {
            return Err(KexError::Protocol("Invalid compressed KEX game kind"));
        };
        if original >= COMPRESSED_KIND {
            return Err(KexError::Protocol("Invalid compressed KEX game kind"));
        }
        Ok(KexMessage {
            kind: original,
            payload: inflate(&bytes[1..])?,
        })
    }

    /// Emit pending acknowledgements, retransmit, and keep alive (`tick`).
    pub fn tick(&mut self, now: f64) -> Result<(), KexError> {
        if self.acknowledgment != 0 {
            let mut payload = [0u8; 3];
            payload[0] = u8::from(self.acknowledgment & 2 != 0);
            payload[1..3].copy_from_slice(&self.incoming_reliable.to_be_bytes());
            let bytes = write_kex_packet(&KexPacket {
                flags: 0,
                sequence: 0,
                reliable: 0,
                kind: Some(ACK_KIND),
                payload: payload.to_vec(),
            })?;
            (self.emit)(&bytes);
            self.acknowledgment = 0;
        }
        if self.pending.front().is_some() && now - self.retry_at >= RETRY_MILLISECONDS {
            self.retries += 1;
            if self.retries >= MAX_RETRIES {
                return Err(KexError::Protocol("KEX LAN peer timed out"));
            }
            self.retry_at = now;
            if let Some(first) = self.pending.front() {
                let bytes = first.bytes.clone();
                (self.emit)(&bytes);
            }
        }
        if now - self.received_at >= KEEPALIVE_MILLISECONDS && self.pending.is_empty() {
            self.send(KEEPALIVE_KIND, &[], KexMode::Reliable, now)?;
            self.received_at = now;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Emit sink recording every datagram.
    #[allow(clippy::type_complexity)]
    fn recorder() -> (Arc<Mutex<Vec<Vec<u8>>>>, impl FnMut(&[u8]) -> bool + Send) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sink = sent.clone();
        let emit = move |bytes: &[u8]| {
            sink.lock().unwrap().push(bytes.to_vec());
            true
        };
        (sent, emit)
    }

    /// Feed recorded datagrams into a channel, collecting messages.
    fn pump(channel: &mut KexChannel, datagrams: &[Vec<u8>], now: f64) -> Vec<KexMessage> {
        let mut messages = Vec::new();
        for datagram in datagrams {
            if let Some(message) = channel.receive(datagram, now).unwrap() {
                messages.push(message);
            }
        }
        messages
    }

    #[test]
    fn unsequenced_round_trip() {
        let (sent, emit) = recorder();
        let mut sender = KexChannel::new(emit);
        assert!(sender.send(5, b"hello", KexMode::Unsequenced, 0.0).unwrap());
        let datagrams = sent.lock().unwrap().clone();
        assert_eq!(datagrams.len(), 1);
        let packet = read_kex_packet(&datagrams[0]).unwrap();
        assert_eq!(packet.flags, 0);
        assert_eq!(packet.kind, Some(5));
        let (_, emit) = recorder();
        let mut receiver = KexChannel::new(emit);
        let messages = pump(&mut receiver, &datagrams, 10.0);
        assert_eq!(
            messages,
            vec![KexMessage {
                kind: 5,
                payload: b"hello".to_vec()
            }]
        );
    }

    #[test]
    fn sequenced_duplicates_are_dropped() {
        let (sent, emit) = recorder();
        let mut sender = KexChannel::new(emit);
        sender.send(0, b"game", KexMode::Sequential, 0.0).unwrap();
        let datagrams = sent.lock().unwrap().clone();
        let (_, emit) = recorder();
        let mut receiver = KexChannel::new(emit);
        assert_eq!(pump(&mut receiver, &datagrams, 10.0).len(), 1);
        assert!(pump(&mut receiver, &datagrams, 11.0).is_empty());
    }

    #[test]
    fn reliable_acknowledgement_drains_pending() {
        let (sent, emit) = recorder();
        let mut sender = KexChannel::new(emit);
        sender.send(0, b"rel", KexMode::Reliable, 0.0).unwrap();
        let datagrams = sent.lock().unwrap().clone();
        let (received, emit) = recorder();
        let mut receiver = KexChannel::new(emit);
        let messages = pump(&mut receiver, &datagrams, 10.0);
        assert_eq!(messages.len(), 1);
        receiver.tick(11.0).unwrap();
        let acks = received.lock().unwrap().clone();
        assert_eq!(acks.len(), 1);
        let ack = read_kex_packet(&acks[0]).unwrap();
        assert_eq!(ack.kind, Some(130));
        assert_eq!(sender.receive(&acks[0], 12.0).unwrap(), None);
        // No retransmit once drained.
        let before = sent.lock().unwrap().len();
        sender.tick(600.0).unwrap();
        assert_eq!(sent.lock().unwrap().len(), before);
    }

    #[test]
    fn out_of_order_reliable_requests_resend() {
        let (sent, emit) = recorder();
        let mut sender = KexChannel::new(emit);
        sender.send(0, b"one", KexMode::Reliable, 0.0).unwrap();
        sender.send(0, b"two", KexMode::Reliable, 0.0).unwrap();
        let datagrams = sent.lock().unwrap().clone();
        let (received, emit) = recorder();
        let mut receiver = KexChannel::new(emit);
        // Skip the first datagram: the gap is dropped with a resend flag.
        assert!(pump(&mut receiver, &datagrams[1..], 10.0).is_empty());
        receiver.tick(11.0).unwrap();
        let acks = received.lock().unwrap().clone();
        assert_eq!(acks.len(), 1);
        assert_eq!(read_kex_packet(&acks[0]).unwrap().payload[0], 1);
    }

    #[test]
    fn fragmented_message_reassembles() {
        let payload = vec![0xABu8; 3000];
        let (sent, emit) = recorder();
        let mut sender = KexChannel::new(emit);
        // Kind 200 skips the compression path (kind >= 127).
        sender.send(200, &payload, KexMode::Sequential, 0.0).unwrap();
        let datagrams = sent.lock().unwrap().clone();
        assert!(datagrams.len() > 1);
        let (_, emit) = recorder();
        let mut receiver = KexChannel::new(emit);
        let messages = pump(&mut receiver, &datagrams, 10.0);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].kind, 200);
        assert_eq!(messages[0].payload, payload);
    }

    #[test]
    fn large_payload_compresses_on_the_wire() {
        let payload = vec![0u8; 512];
        let (sent, emit) = recorder();
        let mut sender = KexChannel::new(emit);
        sender.send(3, &payload, KexMode::Sequential, 0.0).unwrap();
        let datagrams = sent.lock().unwrap().clone();
        assert_eq!(datagrams.len(), 1);
        assert_eq!(read_kex_packet(&datagrams[0]).unwrap().kind, Some(127));
        let (_, emit) = recorder();
        let mut receiver = KexChannel::new(emit);
        let messages = pump(&mut receiver, &datagrams, 10.0);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].kind, 3);
        assert_eq!(messages[0].payload, payload);
    }

    #[test]
    fn reliable_overflow_fails() {
        let (_, emit) = recorder();
        let mut sender = KexChannel::new(emit);
        let payload = vec![0x55u8; 1000];
        let mut result = Ok(true);
        for _ in 0..3000 {
            result = sender.send(200, &payload, KexMode::Reliable, 0.0);
            if result.is_err() {
                break;
            }
        }
        assert_eq!(result, Err(KexError::Protocol("KEX reliable queue is full")));
    }

    #[test]
    fn retransmit_times_out() {
        let (_, emit) = recorder();
        let mut sender = KexChannel::new(emit);
        sender.send(0, b"rel", KexMode::Reliable, 0.0).unwrap();
        let mut result = Ok(());
        for step in 1..=40 {
            result = sender.tick(f64::from(step) * 500.0);
            if result.is_err() {
                break;
            }
        }
        assert_eq!(result, Err(KexError::Protocol("KEX LAN peer timed out")));
    }

    #[test]
    fn idle_channel_sends_keepalive() {
        let (sent, emit) = recorder();
        let mut channel = KexChannel::new(emit);
        channel.tick(6000.0).unwrap();
        let datagrams = sent.lock().unwrap().clone();
        assert_eq!(datagrams.len(), 1);
        let packet = read_kex_packet(&datagrams[0]).unwrap();
        assert_eq!(packet.kind, Some(129));
        assert_eq!(packet.flags & 1, 1);
    }

    #[test]
    fn bad_acknowledgement_fails() {
        let (_, emit) = recorder();
        let mut channel = KexChannel::new(emit);
        let bytes = write_kex_packet(&KexPacket {
            flags: 0,
            sequence: 0,
            reliable: 0,
            kind: Some(130),
            payload: vec![0, 1],
        })
        .unwrap();
        assert_eq!(
            channel.receive(&bytes, 0.0),
            Err(KexError::Protocol("Invalid KEX acknowledgment"))
        );
    }
}
