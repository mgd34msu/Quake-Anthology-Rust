use super::{Channel, Error, Policy};
use crate::headers::{self, Direction, Fragment, Header, datagram};
use qa_core::{
    events::NativeReceipt, loopback::Endpoint, payloads::PayloadQueue, sys_events::EventTime,
};

const PACKET_BYTES: usize = 1400;
const RELIABLE_RECORDS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransmitError {
    Full,
    MessageTooLarge,
    PayloadReliability,
    PendingPacket,
    NoPacket,
    Header(headers::Error),
}
impl std::fmt::Display for TransmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "channel transmit {self:?}")
    }
}
impl std::error::Error for TransmitError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unreliable {
    Included,
    Dropped,
    Deferred,
}
pub struct Prepared<'a> {
    pub bytes: &'a [u8],
    pub unreliable: Unreliable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SendState {
    pub sequence: u32,
    pub datagram_sequence: u32,
    pub ack_sequence: u32,
    pub reliable_sequence: bool,
    pub last_reliable_sequence: u32,
    pub reliable_bytes: usize,
    pub fragment_bytes: usize,
    pub fragment_offset: usize,
    pub last_sent: EventTime,
    pub last_reliable_sent: EventTime,
    pub packets: u64,
    pub resends: u64,
    pub unreliable_drops: u64,
}

#[derive(Clone, Copy)]
enum Commit {
    Control,
    Datagram,
    ReliableDatagram {
        repeat: bool,
    },
    Sequenced {
        reliable: bool,
    },
    Fragment {
        length: usize,
        more: bool,
        reliable: bool,
    },
}
#[derive(Clone, Copy)]
struct Pending {
    length: usize,
    unreliable: Unreliable,
    commit: Commit,
}
pub(super) struct Transmit {
    direction: Direction,
    qport: u16,
    state: SendState,
    ring: PayloadQueue<NativeReceipt>,
    receipt: u64,
    pub(super) receipts: Box<[NativeReceipt]>,
    pub(super) receipt_count: usize,
    flight: Box<[u8]>,
    flight_records: usize,
    flight_length: usize,
    flight_offset: usize,
    started: bool,
    send_next: bool,
    message: Box<[u8]>,
    fragment_pending: bool,
    packet: [u8; PACKET_BYTES],
    pending: Option<Pending>,
}

impl Transmit {
    pub(super) fn load(policy: Policy, maximum: usize, endpoint: Endpoint) -> Result<Self, Error> {
        Ok(Self {
            direction: match endpoint {
                Endpoint::Client => Direction::ToServer,
                Endpoint::Server => Direction::ToClient,
            },
            qport: 0,
            state: SendState {
                sequence: u32::from(!policy.datagram),
                ..SendState::default()
            },
            ring: PayloadQueue::load(
                RELIABLE_RECORDS,
                maximum.saturating_mul(2).min(64 * 1024 * 1024),
            )
            .map_err(|_| Error::Capacity)?,
            receipt: 0,
            receipts: vec![NativeReceipt(0); RELIABLE_RECORDS].into_boxed_slice(),
            receipt_count: 0,
            flight: vec![0; maximum].into_boxed_slice(),
            flight_records: 0,
            flight_length: 0,
            flight_offset: 0,
            started: false,
            send_next: false,
            message: vec![0; maximum].into_boxed_slice(),
            fragment_pending: false,
            packet: [0; PACKET_BYTES],
            pending: None,
        })
    }
    fn retire(&mut self) {
        for _ in 0..self.flight_records {
            if let Some((receipt, _)) = self.ring.pop() {
                self.receipts[self.receipt_count] = receipt;
                self.receipt_count += 1;
            }
        }
        self.flight_records = 0;
        self.flight_length = 0;
        self.flight_offset = 0;
        self.state.reliable_bytes = 0;
        self.started = false;
        self.send_next = false;
    }
    pub(super) fn ack_toggle(&mut self, reliable_ack: bool) {
        if self.started && self.flight_records != 0 && reliable_ack == self.state.reliable_sequence
        {
            self.retire();
        }
    }
    pub(super) fn ack_datagram(&mut self, sequence: u32) {
        if !self.started
            || self.flight_records == 0
            || sequence != self.state.sequence.wrapping_sub(1)
            || sequence != self.state.ack_sequence
        {
            return;
        }
        self.state.ack_sequence = self.state.ack_sequence.wrapping_add(1);
        self.flight_offset = (self.flight_offset + 1024).min(self.flight_length);
        self.state.reliable_bytes = self.flight_length - self.flight_offset;
        if self.flight_offset == self.flight_length {
            self.retire();
        } else {
            self.send_next = true;
        }
    }
    fn select_flight(&mut self, capacity: usize) -> bool {
        if self.flight_records != 0 {
            return false;
        }
        let mut length = 0;
        while let Some((_, body)) = self.ring.get(self.flight_records) {
            if body.len() > capacity - length {
                break;
            }
            self.flight[length..length + body.len()].copy_from_slice(body);
            length += body.len();
            self.flight_records += 1;
        }
        if self.flight_records == 0 {
            return false;
        }
        self.flight_length = length;
        self.flight_offset = 0;
        self.state.reliable_bytes = length;
        self.started = false;
        true
    }
}

impl Channel {
    pub fn has_output(&self) -> bool {
        self.transmit.pending.is_some()
            || !self.transmit.ring.is_empty()
            || self.pending_controls() != 0
            || self.transmit.fragment_pending
    }
    /// A connection supplies its negotiated native qport; it is never ClientId.
    pub fn set_qport(&mut self, qport: u16) {
        self.transmit.qport = qport;
    }
    pub fn send_state(&self) -> SendState {
        self.transmit.state
    }
    /// Receipts from the most recent receive; consume before the next packet.
    pub fn reliable_receipts(&self) -> &[NativeReceipt] {
        &self.transmit.receipts[..self.transmit.receipt_count]
    }
    pub fn queue_reliable(&mut self, bytes: &[u8]) -> Result<NativeReceipt, TransmitError> {
        if !self.policy.datagram && !self.policy.toggle_ack {
            return Err(TransmitError::PayloadReliability);
        }
        let maximum = if !self.policy.datagram && self.policy.fragment_payload == 0 {
            PACKET_BYTES
                - self
                    .policy
                    .format
                    .size(self.transmit.direction, false)
                    .map_err(TransmitError::Header)?
        } else {
            self.transmit.flight.len().min(if self.policy.datagram {
                usize::MAX
            } else {
                32768
            })
        };
        if bytes.is_empty() || bytes.len() > maximum || bytes.len() > self.transmit.flight.len() {
            return Err(TransmitError::MessageTooLarge);
        }
        let receipt = NativeReceipt(self.transmit.receipt.wrapping_add(1));
        self.transmit
            .ring
            .push(receipt, bytes, 0)
            .map_err(|_| TransmitError::Full)?;
        self.transmit.receipt = receipt.0;
        Ok(receipt)
    }
    pub fn pending_packet(&self) -> Option<Prepared<'_>> {
        self.transmit.pending.map(|pending| Prepared {
            bytes: &self.transmit.packet[..pending.length],
            unreliable: pending.unreliable,
        })
    }
    /// Encode once. A failed transport admission leaves pending_packet intact.
    /// The caller commits only after the platform or loopback accepts its bytes.
    pub fn prepare(
        &mut self,
        unreliable: Option<&[u8]>,
        time: EventTime,
    ) -> Result<Option<Prepared<'_>>, TransmitError> {
        if self.transmit.pending.is_some() {
            return Err(TransmitError::PendingPacket);
        }
        let tx = &mut self.transmit;
        let header_size = self
            .policy
            .format
            .size(tx.direction, false)
            .map_err(TransmitError::Header)?;
        let mut header = Header {
            sequence: tx.state.sequence,
            acknowledgement: self.state.sequence,
            reliable_ack: self.state.reliable_sequence,
            qport: tx.qport,
            ..Header::default()
        };
        let body = unreliable.unwrap_or(&[]);
        if self.policy.fragment_payload != 0
            && (body.len() > tx.message.len() || body.len() > 32768)
        {
            return Err(TransmitError::MessageTooLarge);
        }
        let mut disposition = Unreliable::Included;
        let mut start = 0;
        let mut length;
        let commit;
        if let Some((control, _)) = self.controls.front() {
            header = control;
            length = 0;
            disposition = Unreliable::Deferred;
            commit = Commit::Control;
        } else if self.policy.datagram {
            let fresh = tx.select_flight(tx.flight.len());
            let repeat = tx.started
                && tx.flight_records != 0
                && !tx.send_next
                && time.since(tx.state.last_reliable_sent) > 1_000_000_000;
            if fresh || tx.send_next || repeat || (tx.flight_records != 0 && !tx.started) {
                header.sequence = if repeat {
                    tx.state.sequence.wrapping_sub(1)
                } else {
                    tx.state.sequence
                };
                length = (tx.flight_length - tx.flight_offset).min(1024);
                tx.message[..length]
                    .copy_from_slice(&tx.flight[tx.flight_offset..tx.flight_offset + length]);
                header.datagram_flags = datagram::DATA
                    | if tx.flight_offset + length == tx.flight_length {
                        datagram::EOM
                    } else {
                        0
                    };
                disposition = Unreliable::Deferred;
                commit = Commit::ReliableDatagram { repeat };
            } else if unreliable.is_some() {
                if body.len() > 1024 || body.len() > tx.message.len() {
                    return Err(TransmitError::MessageTooLarge);
                }
                header.sequence = tx.state.datagram_sequence;
                header.datagram_flags = datagram::UNRELIABLE;
                length = body.len();
                tx.message[..length].copy_from_slice(body);
                commit = Commit::Datagram;
            } else {
                return Ok(None);
            }
        } else if tx.fragment_pending {
            start = tx.state.fragment_offset;
            length = (tx.state.fragment_bytes - start).min(self.policy.fragment_payload);
            let more = if self.policy.fragment_inclusive {
                length == self.policy.fragment_payload
            } else {
                start + length < tx.state.fragment_bytes
            };
            header.fragment = Some(Fragment {
                offset: start as u16,
                more,
            });
            header.reliable = self.policy.toggle_ack && tx.state.reliable_bytes != 0;
            disposition = Unreliable::Deferred;
            commit = Commit::Fragment {
                length,
                more,
                reliable: false,
            };
        } else {
            if self.policy.fragment_inclusive {
                tx.state.fragment_offset = 0;
            }
            let capacity = if self.policy.fragment_payload == 0 {
                (PACKET_BYTES - header_size).min(tx.flight.len())
            } else {
                tx.flight.len().min(32768)
            };
            let fresh = tx.select_flight(capacity);
            if fresh {
                tx.state.reliable_sequence ^= true;
            }
            let reliable = fresh
                || (tx.flight_records != 0 && !tx.started)
                || (self.state.acknowledged > tx.state.last_reliable_sequence
                    && self.state.reliable_acknowledged != tx.state.reliable_sequence);
            let reliable_length = if reliable { tx.state.reliable_bytes } else { 0 };
            length = reliable_length;
            if length > 0 {
                tx.message[..length].copy_from_slice(&tx.flight[..length]);
            }
            let message_capacity = if self.policy.fragment_payload == 0 {
                tx.message.len()
            } else {
                tx.message.len().min(32768)
            };
            if body.len() <= message_capacity - length
                && (self.policy.fragment_payload != 0
                    || body.len() <= PACKET_BYTES - header_size - length)
            {
                tx.message[length..length + body.len()].copy_from_slice(body);
                length += body.len();
            } else {
                disposition = Unreliable::Dropped;
            }
            let fragmented = self.policy.fragment_payload != 0
                && (length > self.policy.fragment_payload
                    || (self.policy.fragment_inclusive && length == self.policy.fragment_payload));
            if fragmented {
                tx.state.fragment_bytes = length;
                tx.state.fragment_offset = 0;
                tx.fragment_pending = true;
                length = length.min(self.policy.fragment_payload);
                let more = if self.policy.fragment_inclusive {
                    length == self.policy.fragment_payload
                } else {
                    length < tx.state.fragment_bytes
                };
                header.fragment = Some(Fragment { offset: 0, more });
                header.reliable = self.policy.toggle_ack && tx.state.reliable_bytes != 0;
                commit = Commit::Fragment {
                    length,
                    more,
                    reliable,
                };
            } else {
                header.reliable = reliable;
                commit = Commit::Sequenced { reliable };
            }
        }
        let size = headers::encode(
            self.policy.format,
            tx.direction,
            header,
            &tx.message[start..start + length],
            &mut tx.packet,
        )
        .map_err(TransmitError::Header)?;
        tx.pending = Some(Pending {
            length: size,
            unreliable: disposition,
            commit,
        });
        Ok(self.pending_packet())
    }
    pub fn submitted(&mut self, time: EventTime) -> Result<(), TransmitError> {
        let tx = &mut self.transmit;
        let pending = tx.pending.take().ok_or(TransmitError::NoPacket)?;
        if pending.unreliable == Unreliable::Dropped {
            tx.state.unreliable_drops = tx.state.unreliable_drops.saturating_add(1);
        }
        match pending.commit {
            Commit::Control => {
                self.controls.pop();
            }
            Commit::Datagram => {
                tx.state.datagram_sequence = tx.state.datagram_sequence.wrapping_add(1);
            }
            Commit::ReliableDatagram { repeat } => {
                if repeat {
                    tx.state.resends = tx.state.resends.saturating_add(1);
                } else {
                    tx.state.sequence = tx.state.sequence.wrapping_add(1);
                }
                tx.started = tx.flight_records != 0;
                tx.send_next = false;
                tx.state.last_reliable_sent = time;
            }
            Commit::Sequenced { reliable } => {
                if reliable {
                    tx.state.last_reliable_sequence =
                        tx.state.sequence.wrapping_add(self.policy.reliable_bias);
                    tx.started = tx.flight_records != 0;
                }
                tx.state.sequence = tx.state.sequence.wrapping_add(1);
            }
            Commit::Fragment {
                length,
                more,
                reliable,
            } => {
                if reliable && tx.state.fragment_offset == 0 {
                    tx.state.last_reliable_sequence =
                        tx.state.sequence.wrapping_add(self.policy.reliable_bias);
                    tx.started = tx.flight_records != 0;
                }
                tx.state.fragment_offset += length;
                if !more {
                    tx.state.sequence = tx.state.sequence.wrapping_add(1);
                    tx.state.fragment_bytes = 0;
                    tx.fragment_pending = false;
                    if !self.policy.fragment_inclusive {
                        tx.state.fragment_offset = 0;
                    }
                }
            }
        }
        tx.state.last_sent = time;
        tx.state.packets = tx.state.packets.saturating_add(1);
        Ok(())
    }
}
