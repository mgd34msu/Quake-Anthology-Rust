use super::{Channel, Error, Policy};
use crate::headers::{self, Direction, Fragment, Header, datagram};
use qa_core::{
    events::NativeReceipt, loopback::Endpoint, payloads::PayloadQueue,
    primitives::NATIVE_WIRE_BYTES, sys_events::EventTime,
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
    SnapshotContext,
    ServerContext,
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

/// Metadata for the newest command in an encoded native message. This never
/// adds a field to the protocol and becomes visible only after submission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoveCommand {
    pub number: u64,
    pub server_time_ms: i32,
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
    commands: Option<u32>,
    snapshot_request: Option<Option<u32>>,
    snapshot: Option<SnapshotSubmission>,
    command: Option<MoveCommand>,
    server_state: Option<NativeReceipt>,
}
#[derive(Clone, Copy)]
struct SnapshotSubmission {
    sequence: u32,
    observation: Option<qa_core::primitives::SnapshotObservation>,
}
#[derive(Clone, Copy)]
struct MoveSubmission {
    time: EventTime,
    command: MoveCommand,
}
#[derive(Clone, Copy)]
struct ServerStateFlight {
    body_bits: usize,
    commands: u32,
    first_submitted: Option<u32>,
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
    command_output: Box<[u8]>,
    fragment_pending: bool,
    command_flight: Option<u32>,
    server_state: Option<ServerStateFlight>,
    server_state_flight: Option<NativeReceipt>,
    snapshot_flight: Option<SnapshotSubmission>,
    move_flight: Option<MoveSubmission>,
    last_command: MoveCommand,
    packet: [u8; PACKET_BYTES],
    pending: Option<Pending>,
    reply_allowed: bool,
}

impl Transmit {
    pub(super) fn load(policy: Policy, maximum: usize, endpoint: Endpoint) -> Result<Self, Error> {
        Ok(Self {
            direction: Direction::sent_from(endpoint),
            qport: 0,
            state: SendState {
                sequence: u32::from(!policy.datagram),
                ..SendState::default()
            },
            ring: PayloadQueue::load(
                if policy.command_ack {
                    1
                } else {
                    RELIABLE_RECORDS
                },
                if policy.command_ack {
                    if endpoint == Endpoint::Server {
                        maximum
                    } else {
                        1
                    }
                } else {
                    maximum.saturating_mul(2).min(64 * 1024 * 1024)
                },
            )
            .map_err(|_| Error::Capacity)?,
            receipt: 0,
            receipts: vec![
                NativeReceipt(0);
                RELIABLE_RECORDS
                    + usize::from(policy.command_ack && endpoint == Endpoint::Server)
            ]
            .into_boxed_slice(),
            receipt_count: 0,
            flight: vec![0; maximum].into_boxed_slice(),
            flight_records: 0,
            flight_length: 0,
            flight_offset: 0,
            started: false,
            send_next: false,
            message: vec![0; maximum].into_boxed_slice(),
            command_output: vec![
                0;
                if endpoint == Endpoint::Client {
                    crate::message::LEGACY_STRING_BYTES + 2
                } else if policy.command_ack {
                    NATIVE_WIRE_BYTES
                } else {
                    0
                }
            ]
            .into_boxed_slice(),
            fragment_pending: false,
            command_flight: None,
            server_state: None,
            server_state_flight: None,
            snapshot_flight: None,
            move_flight: None,
            last_command: MoveCommand::default(),
            packet: [0; PACKET_BYTES],
            pending: None,
            reply_allowed: !policy.reply_per_packet || endpoint == Endpoint::Client,
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
    /// Replace native startup's unsubmitted command text without rewinding the
    /// native reliable flight already accepted by its transport.
    pub(crate) fn reset_client_startup(&mut self) {
        if self.endpoint != Endpoint::Client
            || !self.policy.datagram && self.policy != super::QUAKEWORLD
        {
            return;
        }
        let tx = &mut self.transmit;
        tx.ring
            .truncate(if tx.started { tx.flight_records } else { 0 });
        tx.pending = None;
        if !tx.started {
            // Selecting an unsent sequenced flight already changed its local
            // reliable bit. Cancellation must restore the last submitted bit.
            if self.policy.toggle_ack && tx.flight_records != 0 {
                tx.state.reliable_sequence ^= true;
            }
            tx.flight_records = 0;
            tx.flight_length = 0;
            tx.flight_offset = 0;
            tx.state.reliable_bytes = 0;
            tx.send_next = false;
        }
    }

    /// Native callers that correlate replies with incoming packet sequences
    /// select this behavior explicitly. Header format never selects it.
    pub(crate) fn align_reply(&mut self, incoming: u32) {
        let tx = &mut self.transmit;
        tx.reply_allowed = incoming >= tx.state.sequence;
        // An unsent old payload belongs to its old request. Retain its selected
        // reliable flight/bit, but rebuild the reply against the new request.
        tx.pending = None;
        if tx.reply_allowed {
            tx.state.sequence = incoming;
        }
    }
    pub fn has_output(&self) -> bool {
        self.transmit.pending.is_some()
            || !self.transmit.ring.is_empty()
            || self.pending_controls() != 0
            || self.transmit.fragment_pending
            || self
                .commands
                .as_ref()
                .is_some_and(super::commands::CommandMessages::pending)
    }
    /// A connection supplies its negotiated native qport; it is never ClientId.
    pub fn set_qport(&mut self, qport: u16) {
        self.transmit.qport = qport;
    }
    pub(crate) fn routing_qport(&self) -> Option<u16> {
        match self
            .policy
            .format
            .port(Direction::received_at(self.endpoint))
        {
            crate::headers::QPort::None => None,
            crate::headers::QPort::Byte => Some(u16::from(self.transmit.qport as u8)),
            crate::headers::QPort::Short => Some(self.transmit.qport),
        }
    }
    pub fn send_state(&self) -> SendState {
        self.transmit.state
    }
    /// Receipts from the most recent receive; consume before the next packet.
    pub fn reliable_receipts(&self) -> &[NativeReceipt] {
        &self.transmit.receipts[..self.transmit.receipt_count]
    }
    /// Queue console text using this connection's native client framing. The
    /// ordinary transmit path owns submission, retransmission and retirement.
    pub fn queue_client_command(
        &mut self,
        protocol: crate::commands::packet::Protocol,
        text: &[u8],
    ) -> Result<NativeReceipt, TransmitError> {
        let result = if self.endpoint != Endpoint::Client || !self.supports_protocol(protocol) {
            Err(TransmitError::PayloadReliability)
        } else if protocol == crate::commands::packet::Protocol::Quake3_68 {
            self.queue_reliable(text)
        } else {
            self.queue_client_text(protocol, 4, text)
        };
        if result.is_err() {
            self.counts.client_command_rejected += 1;
        }
        result
    }
    /// Q2 full userinfo changes share the connection's ordinary reliable flight.
    pub fn queue_userinfo(
        &mut self,
        protocol: crate::commands::packet::Protocol,
        text: &[u8],
    ) -> Result<NativeReceipt, TransmitError> {
        use crate::commands::packet::Protocol;
        if self.endpoint != Endpoint::Client
            || !self.supports_protocol(protocol)
            || !matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038)
        {
            return Err(TransmitError::PayloadReliability);
        }
        if protocol.client_string_policy().truncates(text) {
            return Err(TransmitError::MessageTooLarge);
        }
        self.queue_client_text(protocol, 3, text)
    }
    fn queue_client_text(
        &mut self,
        protocol: crate::commands::packet::Protocol,
        opcode: u32,
        text: &[u8],
    ) -> Result<NativeReceipt, TransmitError> {
        let mut bytes = std::mem::take(&mut self.transmit.command_output);
        let result = (|| {
            let mut writer =
                crate::message::Writer::new(&mut bytes, crate::message::Encoding::Bytes);
            writer
                .write_bits(opcode, 8)
                .and_then(|_| writer.write_string(protocol.client_string_policy(), text))
                .map_err(|_| TransmitError::MessageTooLarge)?;
            let length = writer.bytes().len();
            self.queue_reliable(&bytes[..length])
        })();
        self.transmit.command_output = bytes;
        result
    }
    pub fn queue_reliable(&mut self, bytes: &[u8]) -> Result<NativeReceipt, TransmitError> {
        let endpoint = self.endpoint();
        if let Some(commands) = &mut self.commands {
            let receipt = NativeReceipt(self.transmit.receipt.wrapping_add(1));
            commands.queue(bytes, receipt, endpoint)?;
            self.transmit.receipt = receipt.0;
            return Ok(receipt);
        }
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
                NATIVE_WIRE_BYTES
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
    /// Retain a standalone Q3 svc_gamestate body until a native CLIENT message ACK
    /// covers a fully submitted copy. Each send frames this same body with current
    /// CLIENT ACK/XOR fields and only the captured SERVER command watermark.
    pub fn queue_server_state(
        &mut self,
        encoded_body: &[u8],
        body_bits: usize,
        encoded_command_sequence: u32,
    ) -> Result<NativeReceipt, TransmitError> {
        if self.endpoint != Endpoint::Server || !self.policy.command_ack {
            return Err(TransmitError::ServerContext);
        }
        let maximum = self.native_payload_capacity()?;
        if body_bits == 0
            || body_bits / 8 + 1 != encoded_body.len()
            || encoded_body.len() > maximum
            || self
                .server_output_bits(encoded_command_sequence, body_bits)
                .map_err(|_| TransmitError::MessageTooLarge)?
                > maximum * 8
        {
            return Err(TransmitError::MessageTooLarge);
        }
        let mut reader = crate::message::Reader::new(encoded_body, crate::message::Encoding::Q3);
        let valid = (|| {
            if reader.read_bits(8)? != 2 {
                return Err(crate::commands::packet::Error::Opcode);
            }
            let body = super::commands::Q3GameState::read(&mut reader, 0, 1024)?;
            if reader.bit_position() != body_bits
                || body.server_command_sequence != encoded_command_sequence
            {
                return Err(crate::commands::packet::Error::Context);
            }
            Ok(())
        })();
        valid.map_err(|_| TransmitError::MessageTooLarge)?;
        if !self.transmit.ring.is_empty() {
            return Err(TransmitError::Full);
        }
        if self.transmit.pending.is_some() || self.transmit.fragment_pending {
            return Err(TransmitError::PendingPacket);
        }
        let tx = &mut self.transmit;
        let receipt = NativeReceipt(tx.receipt.wrapping_add(1));
        tx.ring
            .push(receipt, encoded_body, 0)
            .map_err(|_| TransmitError::Full)?;
        tx.receipt = receipt.0;
        tx.state.reliable_bytes = encoded_body.len();
        tx.server_state = Some(ServerStateFlight {
            body_bits,
            commands: encoded_command_sequence,
            first_submitted: None,
        });
        Ok(receipt)
    }
    // Only the checked native CLIENT acknowledgement header invokes this path;
    // reliable-command ACK APIs deliberately cannot retire a server-state body.
    pub(crate) fn acknowledge_server_state(&mut self, message: u32) {
        let tx = &mut self.transmit;
        if tx
            .server_state
            .is_some_and(|state| state.first_submitted.is_some_and(|first| message >= first))
        {
            if let Some((receipt, _)) = tx.ring.pop() {
                tx.receipts[tx.receipt_count] = receipt;
                tx.receipt_count += 1;
            }
            tx.server_state = None;
            tx.state.reliable_bytes = 0;
        }
    }
    pub fn pending_packet(&self) -> Option<Prepared<'_>> {
        self.transmit.pending.map(|pending| Prepared {
            bytes: &self.transmit.packet[..pending.length],
            unreliable: pending.unreliable,
        })
    }
    pub fn pending_fragments(&self) -> bool {
        self.transmit.fragment_pending
    }
    /// Admission and sequence commitment are one boundary operation. A full
    /// transport retains the exact packet and all of its native metadata.
    pub fn submit_with(
        &mut self,
        time: EventTime,
        admit: impl FnOnce(&[u8]) -> bool,
    ) -> Result<Option<Unreliable>, TransmitError> {
        let packet = self.pending_packet().ok_or(TransmitError::NoPacket)?;
        let disposition = packet.unreliable;
        if !admit(packet.bytes) {
            return Ok(None);
        }
        self.submitted(time)?;
        Ok(Some(disposition))
    }
    /// Encode once. A failed transport admission leaves pending_packet intact.
    /// The caller commits only after the platform or loopback accepts its bytes.
    pub fn prepare(
        &mut self,
        unreliable: Option<&[u8]>,
        time: EventTime,
    ) -> Result<Option<Prepared<'_>>, TransmitError> {
        self.prepare_inner(unreliable, time, None, None, None)
    }
    /// Maximum native message body before reliable-flight pressure.
    /// Successful serialization does not claim submission or acknowledgement.
    pub fn native_payload_capacity(&self) -> Result<usize, TransmitError> {
        let header_size = self
            .policy
            .format
            .size(self.transmit.direction, false)
            .map_err(TransmitError::Header)?;
        Ok(self.payload_capacity(header_size))
    }
    fn payload_capacity(&self, header_size: usize) -> usize {
        self.transmit.message.len().min(if self.policy.datagram {
            1024
        } else if self.policy.fragment_payload == 0 {
            PACKET_BYTES - header_size
        } else {
            NATIVE_WIRE_BYTES
        })
    }
    fn prepare_inner(
        &mut self,
        unreliable: Option<&[u8]>,
        time: EventTime,
        commands: Option<u32>,
        snapshot_request: Option<Option<u32>>,
        snapshot: Option<SnapshotSubmission>,
    ) -> Result<Option<Prepared<'_>>, TransmitError> {
        if self.transmit.pending.is_some() {
            return Err(TransmitError::PendingPacket);
        }
        if !self.transmit.reply_allowed {
            return Ok(None);
        }
        let header_size = self
            .policy
            .format
            .size(self.transmit.direction, false)
            .map_err(TransmitError::Header)?;
        let body_capacity = self.payload_capacity(header_size);
        let tx = &mut self.transmit;
        let mut header = Header {
            sequence: tx.state.sequence,
            acknowledgement: self.state.sequence,
            reliable_ack: self.state.reliable_sequence,
            qport: tx.qport,
            ..Header::default()
        };
        let body = unreliable.unwrap_or(&[]);
        if self.policy.fragment_payload != 0 && body.len() > body_capacity {
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
                && time.since(tx.state.last_reliable_sent) > qa_core::sys_events::NANOS_PER_SECOND;
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
                if body.len() > body_capacity {
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
            let capacity = body_capacity.min(tx.flight.len());
            let fresh = !self.policy.command_ack && tx.select_flight(capacity);
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
            if body.len() <= body_capacity - length {
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
            commands: (disposition == Unreliable::Included)
                .then_some(commands)
                .flatten(),
            snapshot_request,
            command: None,
            server_state: None,
            snapshot: (disposition == Unreliable::Included)
                .then_some(snapshot)
                .flatten(),
        });
        Ok(self.pending_packet())
    }
    pub fn submitted(&mut self, time: EventTime) -> Result<(), TransmitError> {
        let tx = &mut self.transmit;
        let pending = tx.pending.take().ok_or(TransmitError::NoPacket)?;
        let request_sequence = tx.state.sequence;
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
                if self.policy.reply_per_packet && self.endpoint == Endpoint::Server {
                    tx.reply_allowed = false;
                }
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
                    if self.policy.reply_per_packet && self.endpoint == Endpoint::Server {
                        tx.reply_allowed = false;
                    }
                    if !self.policy.fragment_inclusive {
                        tx.state.fragment_offset = 0;
                    }
                }
            }
        }
        if !self.policy.datagram && self.endpoint == Endpoint::Client {
            if pending.unreliable == Unreliable::Included {
                tx.move_flight = Some(MoveSubmission {
                    time,
                    command: pending.command.unwrap_or(tx.last_command),
                });
            }
            if !tx.fragment_pending
                && let Some(observation) = tx.move_flight.take()
                && let Some(ring) = &mut self.snapshots
            {
                ring.record_move(request_sequence, observation.time, observation.command);
                tx.last_command = observation.command;
            }
        }
        tx.state.last_sent = time;
        tx.state.packets = tx.state.packets.saturating_add(1);
        if pending.server_state.is_some()
            && tx
                .server_state
                .is_some_and(|state| state.first_submitted.is_some())
        {
            tx.state.resends = tx.state.resends.saturating_add(1);
        }
        tx.server_state_flight = pending.server_state.or(tx.server_state_flight);
        if !tx.fragment_pending
            && let Some(receipt) = tx.server_state_flight.take()
            && tx
                .ring
                .front()
                .is_some_and(|(current, _)| current == receipt)
            && let Some(state) = &mut tx.server_state
        {
            state.first_submitted.get_or_insert(request_sequence);
        }
        tx.command_flight = pending.commands.or(tx.command_flight);
        tx.snapshot_flight = pending.snapshot.or(tx.snapshot_flight);
        if !tx.fragment_pending
            && let Some(snapshot) = tx.snapshot_flight.take()
            && let Some(storage) = &mut self.snapshots
        {
            storage.observe_submission(snapshot.sequence, snapshot.observation);
        }
        if !tx.fragment_pending
            && let Some(sequence) = tx.command_flight.take()
            && let Some(commands) = &mut self.commands
        {
            commands.submitted(sequence, tx.state.sequence.wrapping_sub(1));
        }
        if pending.unreliable != Unreliable::Deferred
            && let Some(base) = pending.snapshot_request
            && let Some(storage) = &mut self.snapshots
        {
            storage.record_request(request_sequence, base);
        }
        Ok(())
    }
    #[inline(never)]
    fn prepare_command_output(
        &mut self,
        time: EventTime,
    ) -> Result<Option<Prepared<'_>>, TransmitError> {
        let mut bytes = std::mem::take(&mut self.transmit.command_output);
        let result = (|| {
            let length = self
                .encode_server_output(&mut bytes, |_, _| Ok(()))
                .map_err(|_| TransmitError::MessageTooLarge)?;
            self.prepare_server_message(&bytes[..length], time)
                .map(|_| ())
        })();
        self.transmit.command_output = bytes;
        result?;
        Ok(self.pending_packet())
    }
    fn prepare_server_state(
        &mut self,
        time: EventTime,
    ) -> Result<Option<Prepared<'_>>, TransmitError> {
        let state = self.transmit.server_state.ok_or(TransmitError::NoPacket)?;
        let mut bytes = std::mem::take(&mut self.transmit.command_output);
        let ring = std::mem::take(&mut self.transmit.ring);
        let Some((receipt, body)) = ring.front() else {
            self.transmit.ring = ring;
            self.transmit.command_output = bytes;
            return Err(TransmitError::NoPacket);
        };
        let result = (|| {
            let length = self
                .encode_server_output_through(&mut bytes, state.commands, |writer, _| {
                    writer.append_encoded_q3(body, state.body_bits)?;
                    Ok(())
                })
                .map_err(|_| TransmitError::MessageTooLarge)?;
            self.prepare_inner(
                Some(&bytes[..length]),
                time,
                Some(state.commands),
                None,
                None,
            )
            .map(|_| ())
        })();
        self.transmit.ring = ring;
        self.transmit.command_output = bytes;
        result?;
        if let Some(pending) = &mut self.transmit.pending
            && pending.unreliable == Unreliable::Included
        {
            pending.server_state = Some(receipt);
        }
        Ok(self.pending_packet())
    }
    /// Pre-encoded native SERVER MSG bytes use the same submission/fragment
    /// ownership as reliable commands, without claiming snapshot provenance.
    pub fn prepare_server_message(
        &mut self,
        bytes: &[u8],
        time: EventTime,
    ) -> Result<Option<Prepared<'_>>, TransmitError> {
        if self.endpoint() != Endpoint::Server {
            return Err(TransmitError::ServerContext);
        }
        let queued = self.command_state().map(|state| state.queued);
        self.prepare_inner(Some(bytes), time, queued, None, None)
    }

    pub fn prepare_output(
        &mut self,
        time: EventTime,
    ) -> Result<Option<Prepared<'_>>, TransmitError> {
        if self.transmit.pending.is_some() {
            return Err(TransmitError::PendingPacket);
        }
        if self.transmit.server_state.is_some() && !self.transmit.fragment_pending {
            self.prepare_server_state(time)
        } else if self.commands.is_some() && !self.transmit.fragment_pending {
            self.prepare_command_output(time)
        } else {
            self.prepare(None, time)
        }
    }
    pub fn prepare_move(
        &mut self,
        bytes: &[u8],
        time: EventTime,
        snapshot_request: Option<u32>,
        command: Option<MoveCommand>,
    ) -> Result<Option<Prepared<'_>>, TransmitError> {
        let queued = self.command_state().map(|state| state.queued);
        self.prepare_inner(Some(bytes), time, queued, Some(snapshot_request), None)
            .map(|_| ())?;
        if let Some(pending) = &mut self.transmit.pending
            && pending.unreliable == Unreliable::Included
        {
            pending.command = command;
        }
        Ok(self.pending_packet())
    }
    /// Retain snapshot provenance with the same prepared bytes and native
    /// fragments; it becomes observable only when all those bytes are admitted.
    pub fn prepare_snapshot(
        &mut self,
        sequence: u32,
        observation: Option<qa_core::primitives::SnapshotObservation>,
        bytes: &[u8],
        time: EventTime,
    ) -> Result<Option<Prepared<'_>>, TransmitError> {
        if self.endpoint() != Endpoint::Server || self.snapshot(sequence).is_none() {
            return Err(TransmitError::SnapshotContext);
        }
        let queued = self.command_state().map(|state| state.queued);
        self.prepare_inner(
            Some(bytes),
            time,
            queued,
            None,
            Some(SnapshotSubmission {
                sequence,
                observation,
            }),
        )
    }
}
