//! Reliable sequencing ported from `src/network/common/reliability.ts`.
//!
//! [`ToggleReliableChannel`] is QW/Q2's reliable-bit protocol and
//! [`StopAndWaitChannel`] is NetQuake's acknowledged one-fragment-at-a-time
//! channel. Adapters write their own headers and qport around these packets.

use thiserror::Error;

/// Error for reliable-channel misuse.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ReliabilityError {
    /// Capacity or fragment limits are invalid.
    #[error("Invalid reliable capacity")]
    BadCapacity,
    /// Queued reliable bytes overflowed the channel.
    #[error("Reliable message overflow")]
    Overflow,
    /// Packet payload capacity is invalid.
    #[error("Invalid packet payload capacity")]
    BadPayloadCapacity,
    /// Reliable payload does not fit the packet.
    #[error("Reliable payload does not fit packet")]
    DoesNotFit,
    /// Channel sequence exhausted; reconnect required.
    #[error("Channel sequence exhausted; reconnect required")]
    Exhausted,
    /// Outgoing sequence advancement is invalid.
    #[error("Invalid outgoing sequence advancement")]
    BadAdvance,
    /// Reliable message exceeds capacity.
    #[error("Reliable message exceeds capacity")]
    TooLarge,
    /// Reliable message remains unacknowledged.
    #[error("Reliable message remains unacknowledged")]
    StillPending,
    /// NetQuake fragment limits are invalid.
    #[error("Invalid NetQuake fragment limits")]
    BadFragments,
    /// Reliable receive buffer overflowed.
    #[error("Reliable receive overflow")]
    ReceiveOverflow,
}

/// Reliable toggle bit (0 or 1).
pub type ReliableBit = u8;

fn flip(value: ReliableBit) -> ReliableBit {
    if value == 0 {
        1
    } else {
        0
    }
}

/// Outbound toggle packet (`TogglePacket`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TogglePacket {
    /// Outgoing sequence.
    pub sequence: i64,
    /// Acknowledged incoming sequence.
    pub acknowledged: i64,
    /// True when the reliable buffer is attached.
    pub reliable: bool,
    /// Incoming reliable bit being acknowledged.
    pub reliable_acknowledged: ReliableBit,
    /// Packet payload.
    pub payload: Vec<u8>,
}

/// Toggle receive result (`ToggleReceive`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToggleReceive {
    /// Packet accepted.
    Accepted {
        /// Dropped packets before this one.
        dropped: i64,
        /// Packet payload.
        payload: Vec<u8>,
    },
    /// Packet rejected as a duplicate or reordering.
    Rejected,
}

/// QW/Q2 reliable-bit channel (`ToggleReliableChannel`).
#[derive(Debug)]
pub struct ToggleReliableChannel {
    capacity: usize,
    pending: Vec<u8>,
    reliable: Vec<u8>,
    reliable_sequence: ReliableBit,
    incoming_reliable: ReliableBit,
    incoming_reliable_ack: ReliableBit,
    incoming_ack: i64,
    last_reliable: i64,
    incoming: i64,
    outgoing: i64,
}

impl ToggleReliableChannel {
    /// Create a channel with `capacity` bytes and first sequence 1.
    pub fn new(capacity: usize) -> Result<Self, ReliabilityError> {
        Self::with_first_sequence(capacity, 1)
    }

    /// Create a channel with `capacity` bytes and a first sequence.
    pub fn with_first_sequence(capacity: usize, first_sequence: i64) -> Result<Self, ReliabilityError> {
        if capacity == 0 {
            return Err(ReliabilityError::BadCapacity);
        }
        Ok(Self {
            capacity,
            pending: Vec::new(),
            reliable: Vec::new(),
            reliable_sequence: 0,
            incoming_reliable: 0,
            incoming_reliable_ack: 0,
            incoming_ack: 0,
            last_reliable: 0,
            incoming: 0,
            outgoing: first_sequence,
        })
    }

    /// Channel capacity in bytes.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Incoming sequence.
    #[must_use]
    pub fn incoming_sequence(&self) -> i64 {
        self.incoming
    }

    /// Outgoing sequence.
    #[must_use]
    pub fn outgoing_sequence(&self) -> i64 {
        self.outgoing
    }

    /// Align server replies with accepted command sequences
    /// (`advanceOutgoingSequence`).
    pub fn advance_outgoing_sequence(&mut self, sequence: i64) -> Result<(), ReliabilityError> {
        if sequence < self.outgoing || sequence > 0x7fff_ffff {
            return Err(ReliabilityError::BadAdvance);
        }
        self.outgoing = sequence;
        Ok(())
    }

    /// True while reliable bytes are queued or unacknowledged.
    #[must_use]
    pub fn has_pending_reliable(&self) -> bool {
        !self.pending.is_empty() || !self.reliable.is_empty()
    }

    /// Queue reliable bytes (`queue`).
    pub fn queue(&mut self, payload: &[u8]) -> Result<(), ReliabilityError> {
        if self.pending.len() + payload.len() > self.capacity {
            return Err(ReliabilityError::Overflow);
        }
        self.pending.extend_from_slice(payload);
        Ok(())
    }

    /// Build the next packet (`transmit`).
    pub fn transmit(
        &mut self,
        unreliable: &[u8],
        payload_capacity: Option<usize>,
    ) -> Result<TogglePacket, ReliabilityError> {
        let payload_capacity = payload_capacity.unwrap_or(self.capacity);
        if payload_capacity > self.capacity {
            return Err(ReliabilityError::BadPayloadCapacity);
        }
        if self.outgoing > 0x7fff_ffff {
            return Err(ReliabilityError::Exhausted);
        }
        let mut send_reliable =
            self.incoming_ack > self.last_reliable && self.incoming_reliable_ack != self.reliable_sequence;
        if self.reliable.is_empty() && !self.pending.is_empty() {
            self.reliable = std::mem::take(&mut self.pending);
            self.reliable_sequence = flip(self.reliable_sequence);
            send_reliable = true;
        }
        let reliable_bytes = if send_reliable {
            self.reliable.clone()
        } else {
            Vec::new()
        };
        if reliable_bytes.len() > payload_capacity {
            return Err(ReliabilityError::DoesNotFit);
        }
        let include_unreliable = reliable_bytes.len() + unreliable.len() <= payload_capacity;
        let mut payload =
            Vec::with_capacity(reliable_bytes.len() + if include_unreliable { unreliable.len() } else { 0 });
        payload.extend_from_slice(&reliable_bytes);
        if include_unreliable {
            payload.extend_from_slice(unreliable);
        }
        let sequence = self.outgoing;
        self.outgoing += 1;
        // C records the already-incremented outgoing_sequence for
        // retransmission detection.
        if send_reliable {
            self.last_reliable = self.outgoing;
        }
        Ok(TogglePacket {
            sequence,
            acknowledged: self.incoming,
            reliable: send_reliable,
            reliable_acknowledged: self.incoming_reliable,
            payload,
        })
    }

    /// Receive a packet (`receive`).
    pub fn receive(&mut self, packet: &TogglePacket) -> ToggleReceive {
        if packet.sequence <= self.incoming {
            return ToggleReceive::Rejected;
        }
        let dropped = packet.sequence - self.incoming - 1;
        if packet.reliable_acknowledged == self.reliable_sequence {
            self.reliable.clear();
        }
        self.incoming = packet.sequence;
        self.incoming_ack = packet.acknowledged;
        self.incoming_reliable_ack = packet.reliable_acknowledged;
        if packet.reliable {
            self.incoming_reliable = flip(self.incoming_reliable);
        }
        ToggleReceive::Accepted {
            dropped,
            payload: packet.payload.clone(),
        }
    }
}

/// NetQuake reliable fragment (`ReliableFragment`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReliableFragment {
    /// Stop-and-wait sequence.
    pub sequence: u32,
    /// True for the final fragment.
    pub final_fragment: bool,
    /// Fragment payload.
    pub payload: Vec<u8>,
}

/// Stop-and-wait receive result (`ReliableFragmentReceive`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReliableFragmentReceive {
    /// Duplicate fragment; acknowledge without consuming.
    Duplicate {
        /// Sequence to acknowledge.
        acknowledge: u32,
    },
    /// Non-final fragment accepted.
    Fragment {
        /// Sequence to acknowledge.
        acknowledge: u32,
    },
    /// Reassembled message.
    Message {
        /// Sequence to acknowledge.
        acknowledge: u32,
        /// Reassembled payload.
        payload: Vec<u8>,
    },
}

#[derive(Debug)]
struct Sending {
    bytes: Vec<u8>,
    offset: usize,
    sent_at: Option<f64>,
}

/// NetQuake stop-and-wait channel (`StopAndWaitChannel`).
#[derive(Debug)]
pub struct StopAndWaitChannel {
    max_message_bytes: usize,
    fragment_bytes: usize,
    retry_milliseconds: f64,
    outgoing: u32,
    incoming: u32,
    sending: Option<Sending>,
    received: Vec<u8>,
    received_length: usize,
}

impl StopAndWaitChannel {
    /// Create a channel with the donor default 1000 ms retry interval.
    pub fn new(max_message_bytes: usize, fragment_bytes: usize) -> Result<Self, ReliabilityError> {
        Self::with_retry(max_message_bytes, fragment_bytes, 1000.0)
    }

    /// Create a channel with message, fragment, and retry limits.
    pub fn with_retry(
        max_message_bytes: usize,
        fragment_bytes: usize,
        retry_milliseconds: f64,
    ) -> Result<Self, ReliabilityError> {
        if max_message_bytes == 0 || fragment_bytes == 0 || fragment_bytes > max_message_bytes {
            return Err(ReliabilityError::BadFragments);
        }
        Ok(Self {
            max_message_bytes,
            fragment_bytes,
            retry_milliseconds,
            outgoing: 0,
            incoming: 0,
            sending: None,
            received: vec![0; max_message_bytes],
            received_length: 0,
        })
    }

    /// True when no message awaits acknowledgment.
    #[must_use]
    pub fn can_send(&self) -> bool {
        self.sending.is_none()
    }

    /// Queue a reliable message (`begin`).
    pub fn begin(&mut self, bytes: &[u8]) -> Result<(), ReliabilityError> {
        if self.sending.is_some() {
            return Err(ReliabilityError::StillPending);
        }
        if bytes.len() > self.max_message_bytes {
            return Err(ReliabilityError::TooLarge);
        }
        self.sending = Some(Sending {
            bytes: bytes.to_vec(),
            offset: 0,
            sent_at: None,
        });
        Ok(())
    }

    /// Next fragment to transmit, honoring the retry interval (`next`).
    pub fn next(&mut self, now: f64) -> Option<ReliableFragment> {
        let pending = self.sending.as_mut()?;
        if pending
            .sent_at
            .is_some_and(|sent_at| now - sent_at <= self.retry_milliseconds)
        {
            return None;
        }
        let end = (pending.bytes.len()).min(pending.offset + self.fragment_bytes);
        pending.sent_at = Some(now);
        Some(ReliableFragment {
            sequence: self.outgoing,
            final_fragment: end == pending.bytes.len(),
            payload: pending.bytes[pending.offset..end].to_vec(),
        })
    }

    /// Acknowledge a transmitted fragment (`acknowledge`).
    pub fn acknowledge(&mut self, sequence: u32) -> bool {
        let Some(pending) = self.sending.as_mut() else {
            return false;
        };
        if pending.sent_at.is_none() || sequence != self.outgoing {
            return false;
        }
        self.outgoing = self.outgoing.wrapping_add(1);
        pending.offset += self.fragment_bytes;
        if pending.offset >= pending.bytes.len() {
            self.sending = None;
        } else {
            pending.sent_at = None;
        }
        true
    }

    /// Receive a fragment (`receive`).
    pub fn receive(&mut self, fragment: &ReliableFragment) -> Result<ReliableFragmentReceive, ReliabilityError> {
        if fragment.sequence != self.incoming {
            return Ok(ReliableFragmentReceive::Duplicate {
                acknowledge: fragment.sequence,
            });
        }
        if self.received_length + fragment.payload.len() > self.max_message_bytes {
            return Err(ReliabilityError::ReceiveOverflow);
        }
        self.received[self.received_length..self.received_length + fragment.payload.len()]
            .copy_from_slice(&fragment.payload);
        self.received_length += fragment.payload.len();
        self.incoming = self.incoming.wrapping_add(1);
        if !fragment.final_fragment {
            return Ok(ReliableFragmentReceive::Fragment {
                acknowledge: fragment.sequence,
            });
        }
        let payload = self.received[..self.received_length].to_vec();
        self.received_length = 0;
        Ok(ReliableFragmentReceive::Message {
            acknowledge: fragment.sequence,
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_channel_retransmits_until_acknowledged() {
        let mut sender = ToggleReliableChannel::with_first_sequence(64, 1).unwrap();
        let mut receiver = ToggleReliableChannel::with_first_sequence(64, 1).unwrap();
        sender.queue(&[1, 2, 3]).unwrap();
        let packet = sender.transmit(&[9], None).unwrap();
        assert!(packet.reliable);
        assert_eq!(packet.payload, vec![1, 2, 3, 9]);
        let ToggleReceive::Accepted { dropped, payload } = receiver.receive(&packet) else {
            panic!("expected acceptance");
        };
        assert_eq!(dropped, 0);
        assert_eq!(payload, vec![1, 2, 3, 9]);
        assert!(matches!(receiver.receive(&packet), ToggleReceive::Rejected));
        // Receiver's reply acknowledges the reliable bit.
        let reply = receiver.transmit(&[], None).unwrap();
        let ToggleReceive::Accepted { .. } = sender.receive(&reply) else {
            panic!("expected acceptance");
        };
        assert!(!sender.has_pending_reliable());
    }

    #[test]
    fn stop_and_wait_reassembles_fragments() {
        let mut sender = StopAndWaitChannel::new(8, 3).unwrap();
        let mut receiver = StopAndWaitChannel::new(8, 3).unwrap();
        sender.begin(&[1, 2, 3, 4, 5]).unwrap();
        let mut now = 0.0;
        let mut message = None;
        for _ in 0..8 {
            if let Some(fragment) = sender.next(now) {
                let final_fragment = fragment.final_fragment;
                let sequence = fragment.sequence;
                let result = receiver.receive(&fragment).unwrap();
                sender.acknowledge(sequence);
                now += 2000.0;
                if final_fragment {
                    assert!(matches!(result, ReliableFragmentReceive::Message { .. }));
                    if let ReliableFragmentReceive::Message { payload, .. } = result {
                        message = Some(payload);
                    }
                    break;
                }
            } else {
                now += 2000.0;
            }
        }
        assert_eq!(message, Some(vec![1, 2, 3, 4, 5]));
        assert!(sender.can_send());
    }
}
