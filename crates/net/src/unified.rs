//! Unified reliable/frame channel ported from `src/network/unified/packet.ts`
//! and `src/network/unified/channel.ts`.
//!
//! One established peer: control uses a bounded ordered window while
//! snapshots never block control. Packet framing is byte-exact with the
//! donor (`QTUC` magic, version 1).

use std::collections::{BTreeMap, HashMap, HashSet};

use thiserror::Error;

/// Unified packet header bytes.
pub const UNIFIED_PACKET_HEADER_BYTES: usize = 48;
/// Maximum unified sequence.
pub const UNIFIED_SEQUENCE_MAX: u32 = 0xffff_ffff;
/// Maximum datagram bytes.
pub const UNIFIED_DATAGRAM_MAX: usize = 65507;

/// Error for unified packet and channel failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UnifiedError {
    /// Unified channel token must contain 128 bits of hexadecimal.
    #[error("Unified channel token must contain 128 bits of hexadecimal")]
    BadToken,
    /// Invalid unified packet sequence.
    #[error("Invalid unified packet sequence")]
    BadSequence,
    /// Invalid unified packet fragment.
    #[error("Invalid unified packet fragment")]
    BadFragment,
    /// Invalid unified channel limit.
    #[error("Invalid unified channel limit")]
    BadLimit,
    /// Invalid unified channel capacity.
    #[error("Invalid unified channel capacity")]
    BadCapacity,
    /// Unified message exceeds channel capacity.
    #[error("Unified message exceeds channel capacity")]
    TooLarge,
    /// Unified sequence exhausted; reconnect required.
    #[error("Unified sequence exhausted; reconnect required")]
    Exhausted,
    /// Unified reliable queue overflow.
    #[error("Unified reliable queue overflow")]
    QueueOverflow,
    /// Frame depends on an unqueued reliable message.
    #[error("Frame depends on an unqueued reliable message")]
    BadFrameDependency,
    /// Invalid unified channel time.
    #[error("Invalid unified channel time")]
    BadTime,
    /// Unified channel is closed.
    #[error("Unified channel is closed")]
    Closed,
    /// Unified reliable assembly timed out.
    #[error("Unified reliable assembly timed out")]
    AssemblyTimeout,
    /// Unified reliable retry limit exceeded.
    #[error("Unified reliable retry limit exceeded")]
    RetryLimit,
}

/// Unified packet (`UnifiedPacket`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnifiedPacket {
    /// Acknowledgment.
    Ack {
        /// Channel token.
        token: String,
        /// Acknowledged message sequence.
        sequence: u32,
        /// Cumulatively acknowledged reliable sequence.
        acknowledged_reliable: u32,
        /// Acknowledged fragment.
        fragment: u16,
    },
    /// Reliable message fragment.
    Reliable {
        /// Channel token.
        token: String,
        /// Message sequence.
        sequence: u32,
        /// Cumulatively acknowledged reliable sequence.
        acknowledged_reliable: u32,
        /// Required reliable sequence (always 0).
        required_reliable_sequence: u32,
        /// Total message bytes.
        total_bytes: u32,
        /// Fragment bytes.
        fragment_bytes: u32,
        /// Fragment index.
        fragment: u16,
        /// Fragment count.
        fragments: u16,
        /// Fragment payload.
        payload: Vec<u8>,
    },
    /// Frame fragment.
    Frame {
        /// Channel token.
        token: String,
        /// Message sequence.
        sequence: u32,
        /// Cumulatively acknowledged reliable sequence.
        acknowledged_reliable: u32,
        /// Required reliable sequence.
        required_reliable_sequence: u32,
        /// Total message bytes.
        total_bytes: u32,
        /// Fragment bytes.
        fragment_bytes: u32,
        /// Fragment index.
        fragment: u16,
        /// Fragment count.
        fragments: u16,
        /// Fragment payload.
        payload: Vec<u8>,
    },
}

impl UnifiedPacket {
    fn token(&self) -> &str {
        match self {
            Self::Ack { token, .. } | Self::Reliable { token, .. } | Self::Frame { token, .. } => token,
        }
    }

    fn sequence(&self) -> u32 {
        match self {
            Self::Ack { sequence, .. } | Self::Reliable { sequence, .. } | Self::Frame { sequence, .. } => *sequence,
        }
    }

    fn acknowledged_reliable(&self) -> u32 {
        match self {
            Self::Ack {
                acknowledged_reliable, ..
            }
            | Self::Reliable {
                acknowledged_reliable, ..
            }
            | Self::Frame {
                acknowledged_reliable, ..
            } => *acknowledged_reliable,
        }
    }

    fn fragment(&self) -> u16 {
        match self {
            Self::Ack { fragment, .. } | Self::Reliable { fragment, .. } | Self::Frame { fragment, .. } => *fragment,
        }
    }
}

/// Validate and normalize a channel token (`unifiedToken`).
pub fn unified_token(token: &str) -> Result<String, UnifiedError> {
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(UnifiedError::BadToken);
    }
    Ok(token.to_ascii_lowercase())
}

fn fragment_count(total_bytes: u32, fragment_bytes: u32) -> u32 {
    if fragment_bytes == 0 {
        return 0;
    }
    total_bytes.div_ceil(fragment_bytes).max(1)
}

fn fragment_length(total_bytes: u32, fragment_bytes: u32, fragment: u16) -> usize {
    let start = u32::from(fragment) * fragment_bytes;
    fragment_bytes.min(total_bytes.saturating_sub(start)) as usize
}

/// Encode a unified packet (`encodeUnifiedPacket`).
pub fn encode_unified_packet(packet: &UnifiedPacket) -> Result<Vec<u8>, UnifiedError> {
    let token = unified_token(packet.token())?;
    if packet.sequence() == 0 {
        return Err(UnifiedError::BadSequence);
    }
    let payload_length = match packet {
        UnifiedPacket::Ack { .. } => 0,
        UnifiedPacket::Reliable {
            required_reliable_sequence,
            total_bytes,
            fragment_bytes,
            fragment,
            fragments,
            payload,
            ..
        }
        | UnifiedPacket::Frame {
            required_reliable_sequence,
            total_bytes,
            fragment_bytes,
            fragment,
            fragments,
            payload,
            ..
        } => {
            if *fragment_bytes == 0
                || *fragment_bytes > (UNIFIED_DATAGRAM_MAX - UNIFIED_PACKET_HEADER_BYTES) as u32
                || *fragments as u32 != fragment_count(*total_bytes, *fragment_bytes)
                || *fragments == 0
                || u32::from(*fragment) >= u32::from(*fragments)
                || payload.len() != fragment_length(*total_bytes, *fragment_bytes, *fragment)
                || (matches!(packet, UnifiedPacket::Reliable { .. }) && *required_reliable_sequence != 0)
            {
                return Err(UnifiedError::BadFragment);
            }
            payload.len()
        }
    };
    let mut bytes = vec![0u8; UNIFIED_PACKET_HEADER_BYTES + payload_length];
    bytes[0..4].copy_from_slice(b"QTUC");
    bytes[4] = 1;
    bytes[5] = match packet {
        UnifiedPacket::Ack { .. } => 0,
        UnifiedPacket::Reliable { .. } => 1,
        UnifiedPacket::Frame { .. } => 2,
    };
    for index in 0..16 {
        bytes[8 + index] =
            u8::from_str_radix(&token[index * 2..index * 2 + 2], 16).map_err(|_| UnifiedError::BadToken)?;
    }
    bytes[24..28].copy_from_slice(&packet.sequence().to_le_bytes());
    bytes[28..32].copy_from_slice(&packet.acknowledged_reliable().to_le_bytes());
    bytes[44..46].copy_from_slice(&packet.fragment().to_le_bytes());
    match packet {
        UnifiedPacket::Ack { .. } => {}
        UnifiedPacket::Reliable {
            required_reliable_sequence,
            total_bytes,
            fragment_bytes,
            fragments,
            payload,
            ..
        }
        | UnifiedPacket::Frame {
            required_reliable_sequence,
            total_bytes,
            fragment_bytes,
            fragments,
            payload,
            ..
        } => {
            bytes[32..36].copy_from_slice(&required_reliable_sequence.to_le_bytes());
            bytes[36..40].copy_from_slice(&total_bytes.to_le_bytes());
            bytes[40..44].copy_from_slice(&fragment_bytes.to_le_bytes());
            bytes[46..48].copy_from_slice(&fragments.to_le_bytes());
            bytes[UNIFIED_PACKET_HEADER_BYTES..].copy_from_slice(payload);
        }
    }
    Ok(bytes)
}

/// Decode a unified packet (`decodeUnifiedPacket`).
#[must_use]
pub fn decode_unified_packet(bytes: &[u8]) -> Option<UnifiedPacket> {
    if bytes.len() < UNIFIED_PACKET_HEADER_BYTES || bytes.len() > UNIFIED_DATAGRAM_MAX {
        return None;
    }
    if &bytes[0..4] != b"QTUC" || bytes[4] != 1 || u16::from_le_bytes([bytes[6], bytes[7]]) != 0 {
        return None;
    }
    let kind = bytes[5];
    let sequence = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    let acknowledged_reliable = u32::from_le_bytes([bytes[28], bytes[29], bytes[30], bytes[31]]);
    let fragment = u16::from_le_bytes([bytes[44], bytes[45]]);
    if kind > 2 || sequence == 0 {
        return None;
    }
    let mut token = String::with_capacity(32);
    for index in 0..16 {
        token.push_str(&format!("{:02x}", bytes[8 + index]));
    }
    let required_reliable_sequence = u32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]);
    let total_bytes = u32::from_le_bytes([bytes[36], bytes[37], bytes[38], bytes[39]]);
    let fragment_bytes = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
    let fragments = u16::from_le_bytes([bytes[46], bytes[47]]);
    if kind == 0 {
        if bytes.len() == UNIFIED_PACKET_HEADER_BYTES
            && required_reliable_sequence == 0
            && total_bytes == 0
            && fragment_bytes == 0
            && fragments == 0
        {
            return Some(UnifiedPacket::Ack {
                token,
                sequence,
                acknowledged_reliable,
                fragment,
            });
        }
        return None;
    }
    if fragment_bytes == 0
        || fragment_bytes > (UNIFIED_DATAGRAM_MAX - UNIFIED_PACKET_HEADER_BYTES) as u32
        || u32::from(fragments) != fragment_count(total_bytes, fragment_bytes)
        || u32::from(fragment) >= u32::from(fragments)
        || bytes.len() - UNIFIED_PACKET_HEADER_BYTES != fragment_length(total_bytes, fragment_bytes, fragment)
        || (kind == 1 && required_reliable_sequence != 0)
    {
        return None;
    }
    let payload = bytes[UNIFIED_PACKET_HEADER_BYTES..].to_vec();
    if kind == 1 {
        Some(UnifiedPacket::Reliable {
            token,
            sequence,
            acknowledged_reliable,
            required_reliable_sequence,
            total_bytes,
            fragment_bytes,
            fragment,
            fragments,
            payload,
        })
    } else {
        Some(UnifiedPacket::Frame {
            token,
            sequence,
            acknowledged_reliable,
            required_reliable_sequence,
            total_bytes,
            fragment_bytes,
            fragment,
            fragments,
            payload,
        })
    }
}

/// Unified channel limits (`UnifiedChannelLimits`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnifiedChannelLimits {
    /// Datagram bytes.
    pub datagram_bytes: usize,
    /// Maximum message bytes.
    pub message_bytes: usize,
    /// Maximum queued reliable bytes.
    pub queued_reliable_bytes: usize,
    /// Maximum queued reliable messages.
    pub queued_reliable_messages: usize,
    /// Reliable window messages.
    pub reliable_window_messages: usize,
    /// Maximum fragments per message.
    pub fragments: usize,
    /// Packets per flush.
    pub packets_per_flush: usize,
    /// Retry interval in milliseconds.
    pub retry_milliseconds: u64,
    /// Maximum transmissions per fragment.
    pub maximum_transmissions: u32,
    /// Assembly timeout in milliseconds.
    pub assembly_milliseconds: u64,
}

impl Default for UnifiedChannelLimits {
    fn default() -> Self {
        Self {
            datagram_bytes: 1200,
            message_bytes: 4 * 1024 * 1024,
            queued_reliable_bytes: 8 * 1024 * 1024,
            queued_reliable_messages: 256,
            reliable_window_messages: 8,
            fragments: 8192,
            packets_per_flush: 32,
            retry_milliseconds: 250,
            maximum_transmissions: 40,
            assembly_milliseconds: 30000,
        }
    }
}

/// Unified delivery (`UnifiedDelivery`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnifiedDelivery {
    /// Ordered reliable message.
    Reliable {
        /// Message sequence.
        sequence: u32,
        /// Payload.
        payload: Vec<u8>,
    },
    /// Newest complete frame whose dependencies are met.
    Frame {
        /// Message sequence.
        sequence: u32,
        /// Required reliable sequence.
        required_reliable_sequence: u32,
        /// Payload.
        payload: Vec<u8>,
    },
}

struct Outgoing {
    sequence: u32,
    payload: Vec<u8>,
    required: u32,
    fragments: usize,
    sent: BTreeMap<u16, (f64, u32)>,
    acknowledged: HashSet<u16>,
    next: usize,
}

struct Assembly {
    sequence: u32,
    required: u32,
    fragment_bytes: u32,
    fragments: u16,
    payload: Vec<u8>,
    received: HashSet<u16>,
    started: f64,
}

/// One established unified peer (`UnifiedChannel`).
pub struct UnifiedChannel {
    token: String,
    limits: UnifiedChannelLimits,
    ended: bool,
    lane: usize,
    next_reliable: u32,
    next_frame: u32,
    reliable_received: u32,
    reliable_acknowledged: u32,
    frame_received: u32,
    newest_frame: u32,
    queued_bytes: usize,
    reliable: Vec<Outgoing>,
    reliable_cursor: usize,
    received_bytes: usize,
    frame: Option<Outgoing>,
    pending_frame: Option<Outgoing>,
    reliable_assemblies: HashMap<u32, Assembly>,
    frame_assembly: Option<Assembly>,
    waiting_frame: Option<UnifiedDelivery>,
    acknowledgments: BTreeMap<String, (u32, u16)>,
}

impl UnifiedChannel {
    /// Create a channel with a token and limits.
    pub fn new(token: &str, limits: UnifiedChannelLimits) -> Result<Self, UnifiedError> {
        let token = unified_token(token)?;
        let values = [
            limits.datagram_bytes,
            limits.message_bytes,
            limits.queued_reliable_bytes,
            limits.queued_reliable_messages,
            limits.reliable_window_messages,
            limits.fragments,
            limits.packets_per_flush,
            limits.retry_milliseconds as usize,
            limits.maximum_transmissions as usize,
            limits.assembly_milliseconds as usize,
        ];
        if values.iter().any(|value| *value < 1) {
            return Err(UnifiedError::BadLimit);
        }
        if limits.datagram_bytes <= UNIFIED_PACKET_HEADER_BYTES
            || limits.datagram_bytes > UNIFIED_DATAGRAM_MAX
            || limits.message_bytes > UNIFIED_SEQUENCE_MAX as usize
            || limits.fragments > 65535
            || limits.reliable_window_messages > 64
            || limits.queued_reliable_bytes < limits.message_bytes
        {
            return Err(UnifiedError::BadCapacity);
        }
        Ok(Self {
            token,
            limits,
            ended: false,
            lane: 0,
            next_reliable: 1,
            next_frame: 1,
            reliable_received: 0,
            reliable_acknowledged: 0,
            frame_received: 0,
            newest_frame: 0,
            queued_bytes: 0,
            reliable: Vec::new(),
            reliable_cursor: 0,
            received_bytes: 0,
            frame: None,
            pending_frame: None,
            reliable_assemblies: HashMap::new(),
            frame_assembly: None,
            waiting_frame: None,
            acknowledgments: BTreeMap::new(),
        })
    }

    /// Channel token.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Received reliable sequence.
    #[must_use]
    pub fn received_reliable_sequence(&self) -> u32 {
        self.reliable_received
    }

    /// Acknowledged reliable sequence.
    #[must_use]
    pub fn acknowledged_reliable_sequence(&self) -> u32 {
        self.reliable_acknowledged
    }

    /// True once closed.
    #[must_use]
    pub fn closed(&self) -> bool {
        self.ended
    }

    fn window_messages(&self) -> usize {
        self.limits
            .reliable_window_messages
            .min(self.limits.queued_reliable_messages)
    }

    fn open(&self) -> Result<(), UnifiedError> {
        if self.ended {
            return Err(UnifiedError::Closed);
        }
        Ok(())
    }

    fn check_time(now: f64) -> Result<(), UnifiedError> {
        if !now.is_finite() || now < 0.0 {
            return Err(UnifiedError::BadTime);
        }
        Ok(())
    }

    fn outgoing(&self, payload: &[u8], sequence: u32, required: u32) -> Result<Outgoing, UnifiedError> {
        let fragment_bytes = self.limits.datagram_bytes - UNIFIED_PACKET_HEADER_BYTES;
        let fragments = payload.len().div_ceil(fragment_bytes).max(1);
        if payload.len() > self.limits.message_bytes || fragments > self.limits.fragments {
            return Err(UnifiedError::TooLarge);
        }
        if sequence == UNIFIED_SEQUENCE_MAX {
            return Err(UnifiedError::Exhausted);
        }
        Ok(Outgoing {
            sequence,
            payload: payload.to_vec(),
            required,
            fragments,
            sent: BTreeMap::new(),
            acknowledged: HashSet::new(),
            next: 0,
        })
    }

    /// Queue a reliable message (`queueReliable`).
    pub fn queue_reliable(&mut self, payload: &[u8]) -> Result<u32, UnifiedError> {
        self.open()?;
        if self.reliable.len() >= self.limits.queued_reliable_messages
            || self.queued_bytes + payload.len() > self.limits.queued_reliable_bytes
        {
            return Err(UnifiedError::QueueOverflow);
        }
        let message = self.outgoing(payload, self.next_reliable, 0)?;
        self.next_reliable = self.next_reliable.wrapping_add(1);
        self.queued_bytes += payload.len();
        let sequence = message.sequence;
        self.reliable.push(message);
        Ok(sequence)
    }

    /// Queue a frame (`queueFrame`).
    pub fn queue_frame(&mut self, payload: &[u8], required_reliable_sequence: u32) -> Result<(), UnifiedError> {
        self.open()?;
        if required_reliable_sequence >= self.next_reliable {
            return Err(UnifiedError::BadFrameDependency);
        }
        let frame = self.outgoing(payload, self.next_frame, required_reliable_sequence)?;
        self.next_frame = self.next_frame.wrapping_add(1);
        if self.frame.is_none() {
            self.frame = Some(frame);
        } else {
            self.pending_frame = Some(frame);
        }
        Ok(())
    }

    fn expire(&mut self, now: f64) -> Result<(), UnifiedError> {
        if self
            .reliable_assemblies
            .values()
            .any(|assembly| now - assembly.started >= self.limits.assembly_milliseconds as f64)
        {
            self.close();
            return Err(UnifiedError::AssemblyTimeout);
        }
        if self
            .frame_assembly
            .as_ref()
            .is_some_and(|assembly| now - assembly.started >= self.limits.assembly_milliseconds as f64)
        {
            self.frame_assembly = None;
        }
        Ok(())
    }

    fn acknowledge(&mut self, sequence: u32, fragment: u16) {
        // A single cumulative ACK suffices for every already delivered duplicate.
        let key = if sequence <= self.reliable_received {
            "delivered".to_owned()
        } else {
            format!("{sequence}:{fragment}")
        };
        self.acknowledgments.insert(key, (sequence, fragment));
    }

    fn accept_acknowledgment(&mut self, packet: &UnifiedPacket) {
        let acknowledged = packet.acknowledged_reliable();
        if let Some(end) = self
            .reliable
            .iter()
            .position(|message| message.sequence == acknowledged)
        {
            let window = self.window_messages();
            let complete = end < window
                && self.reliable[..=end]
                    .iter()
                    .all(|message| message.next == message.fragments);
            if complete {
                let removed: Vec<Outgoing> = self.reliable.drain(..=end).collect();
                for message in removed {
                    self.queued_bytes -= message.payload.len();
                }
                self.reliable_acknowledged = acknowledged;
            }
        }
        if matches!(packet, UnifiedPacket::Ack { .. }) {
            let window = self.window_messages();
            let extent = window.min(self.reliable.len());
            let sequence = packet.sequence();
            let fragment = packet.fragment();
            if let Some(message) = self.reliable[..extent]
                .iter_mut()
                .find(|message| message.sequence == sequence)
            {
                if message.sent.contains_key(&fragment) {
                    message.acknowledged.insert(fragment);
                }
            }
        }
    }

    fn make_assembly(
        sequence: u32,
        required: u32,
        fragment_bytes: u32,
        fragments: u16,
        total_bytes: u32,
        now: f64,
    ) -> Assembly {
        Assembly {
            sequence,
            required,
            fragment_bytes,
            fragments,
            payload: vec![0; total_bytes as usize],
            received: HashSet::new(),
            started: now,
        }
    }

    fn append(
        assembly: &mut Assembly,
        required: u32,
        total_bytes: u32,
        fragment_bytes: u32,
        fragments: u16,
        fragment: u16,
        payload: &[u8],
    ) -> bool {
        if assembly.required != required
            || assembly.payload.len() != total_bytes as usize
            || assembly.fragment_bytes != fragment_bytes
            || assembly.fragments != fragments
        {
            return false;
        }
        if !assembly.received.contains(&fragment) {
            let start = u32::from(fragment) * fragment_bytes;
            assembly.payload[start as usize..start as usize + payload.len()].copy_from_slice(payload);
            assembly.received.insert(fragment);
        }
        true
    }

    fn release_frame(&mut self, delivered: &mut Vec<UnifiedDelivery>) {
        let Some(frame) = self.waiting_frame.clone() else {
            return;
        };
        let (required, sequence) = match &frame {
            UnifiedDelivery::Frame {
                required_reliable_sequence,
                sequence,
                ..
            } => (*required_reliable_sequence, *sequence),
            UnifiedDelivery::Reliable { .. } => return,
        };
        if required > self.reliable_received {
            return;
        }
        self.waiting_frame = None;
        if sequence <= self.frame_received {
            return;
        }
        self.frame_received = sequence;
        delivered.push(frame);
    }

    /// Receive a datagram (`receive`).
    pub fn receive(&mut self, datagram: &[u8], now: f64) -> Result<Vec<UnifiedDelivery>, UnifiedError> {
        self.open()?;
        Self::check_time(now)?;
        self.expire(now)?;
        if datagram.len() > self.limits.datagram_bytes {
            return Ok(Vec::new());
        }
        let Some(packet) = decode_unified_packet(datagram) else {
            return Ok(Vec::new());
        };
        if packet.token() != self.token {
            return Ok(Vec::new());
        }
        match &packet {
            UnifiedPacket::Reliable {
                total_bytes, fragments, ..
            }
            | UnifiedPacket::Frame {
                total_bytes, fragments, ..
            } if *total_bytes as usize > self.limits.message_bytes || *fragments as usize > self.limits.fragments => {
                return Ok(Vec::new());
            }
            _ => {}
        }
        self.accept_acknowledgment(&packet);
        let mut delivered = Vec::new();
        match packet {
            UnifiedPacket::Ack { .. } => {}
            UnifiedPacket::Reliable {
                sequence,
                total_bytes,
                fragment_bytes,
                fragment,
                fragments,
                payload,
                ..
            } => {
                if sequence <= self.reliable_received {
                    self.acknowledge(sequence, fragment);
                    return Ok(delivered);
                }
                if sequence > self.reliable_received + self.window_messages() as u32 {
                    return Ok(delivered);
                }
                if !self.reliable_assemblies.contains_key(&sequence) {
                    // Future assemblies cannot occupy the space needed to close the first gap.
                    let mut reserve = 0;
                    for missing in self.reliable_received + 1..sequence {
                        if !self.reliable_assemblies.contains_key(&missing) {
                            reserve = self.limits.message_bytes;
                            break;
                        }
                    }
                    if self.received_bytes + total_bytes as usize + reserve > self.limits.queued_reliable_bytes {
                        return Ok(delivered);
                    }
                    let assembly = Self::make_assembly(sequence, 0, fragment_bytes, fragments, total_bytes, now);
                    self.received_bytes += total_bytes as usize;
                    self.reliable_assemblies.insert(sequence, assembly);
                }
                let assembly = self
                    .reliable_assemblies
                    .get_mut(&sequence)
                    .unwrap_or_else(|| unreachable!("assembly was just inserted"));
                if !Self::append(assembly, 0, total_bytes, fragment_bytes, fragments, fragment, &payload) {
                    return Ok(delivered);
                }
                self.acknowledge(sequence, fragment);
                loop {
                    let next = self.reliable_received + 1;
                    let ready = self
                        .reliable_assemblies
                        .get(&next)
                        .is_some_and(|assembly| assembly.received.len() == assembly.fragments as usize);
                    if !ready {
                        break;
                    }
                    let ready = self
                        .reliable_assemblies
                        .remove(&next)
                        .unwrap_or_else(|| unreachable!("assembly readiness was just checked"));
                    self.reliable_received = ready.sequence;
                    self.received_bytes -= ready.payload.len();
                    delivered.push(UnifiedDelivery::Reliable {
                        sequence: ready.sequence,
                        payload: ready.payload,
                    });
                }
                if !delivered.is_empty() {
                    self.acknowledgments
                        .retain(|_, (ack_sequence, _)| *ack_sequence > self.reliable_received);
                    self.acknowledge(sequence, fragment);
                    self.release_frame(&mut delivered);
                }
            }
            UnifiedPacket::Frame {
                sequence,
                required_reliable_sequence,
                total_bytes,
                fragment_bytes,
                fragment,
                fragments,
                payload,
                ..
            } => {
                if sequence <= self.frame_received || sequence < self.newest_frame {
                    return Ok(delivered);
                }
                if sequence > self.newest_frame {
                    self.newest_frame = sequence;
                    self.frame_assembly = None;
                }
                if matches!(&self.waiting_frame, Some(UnifiedDelivery::Frame { sequence: waiting, .. }) if *waiting == sequence)
                {
                    return Ok(delivered);
                }
                if self.frame_assembly.is_none() {
                    self.frame_assembly = Some(Self::make_assembly(
                        sequence,
                        required_reliable_sequence,
                        fragment_bytes,
                        fragments,
                        total_bytes,
                        now,
                    ));
                }
                let complete = {
                    let assembly = self
                        .frame_assembly
                        .as_mut()
                        .unwrap_or_else(|| unreachable!("frame assembly was just created"));
                    if !Self::append(
                        assembly,
                        required_reliable_sequence,
                        total_bytes,
                        fragment_bytes,
                        fragments,
                        fragment,
                        &payload,
                    ) {
                        return Ok(delivered);
                    }
                    assembly.received.len() == assembly.fragments as usize
                };
                if complete {
                    let assembly = self
                        .frame_assembly
                        .take()
                        .unwrap_or_else(|| unreachable!("frame assembly was just appended"));
                    self.waiting_frame = Some(UnifiedDelivery::Frame {
                        sequence,
                        required_reliable_sequence: assembly.required,
                        payload: assembly.payload,
                    });
                    self.release_frame(&mut delivered);
                }
            }
        }
        Ok(delivered)
    }

    fn encode_data(&self, message: &Outgoing, reliable: bool, fragment: u16) -> Result<Vec<u8>, UnifiedError> {
        let fragment_bytes = (self.limits.datagram_bytes - UNIFIED_PACKET_HEADER_BYTES) as u32;
        let start = u32::from(fragment) * fragment_bytes;
        let end = (start + fragment_bytes).min(message.payload.len() as u32);
        let packet = if reliable {
            UnifiedPacket::Reliable {
                token: self.token.clone(),
                sequence: message.sequence,
                acknowledged_reliable: self.reliable_received,
                required_reliable_sequence: message.required,
                total_bytes: message.payload.len() as u32,
                fragment_bytes,
                fragment,
                fragments: message.fragments as u16,
                payload: message.payload[start as usize..end as usize].to_vec(),
            }
        } else {
            UnifiedPacket::Frame {
                token: self.token.clone(),
                sequence: message.sequence,
                acknowledged_reliable: self.reliable_received,
                required_reliable_sequence: message.required,
                total_bytes: message.payload.len() as u32,
                fragment_bytes,
                fragment,
                fragments: message.fragments as u16,
                payload: message.payload[start as usize..end as usize].to_vec(),
            }
        };
        encode_unified_packet(&packet)
    }

    fn send_reliable(&mut self, now: f64) -> Result<Option<Vec<u8>>, UnifiedError> {
        let count = self.reliable.len().min(self.window_messages());
        for _ in 0..count {
            let index = self.reliable_cursor % count;
            self.reliable_cursor = (index + 1) % count;
            let (fragment, attempts) = {
                let message = &mut self.reliable[index];
                let mut fragment = message.next as u16;
                if message.next == message.fragments {
                    let mut retry = None;
                    for (candidate, (at, _)) in &message.sent {
                        if !message.acknowledged.contains(candidate)
                            && now - *at >= self.limits.retry_milliseconds as f64
                        {
                            retry = Some(*candidate);
                            break;
                        }
                    }
                    if let Some(retry) = retry {
                        fragment = retry;
                    } else if index == 0 && message.acknowledged.len() == message.fragments {
                        // Probe only the head for a lost cumulative ACK; buffered successors wait.
                        fragment = (message.fragments - 1) as u16;
                        let due = message
                            .sent
                            .get(&fragment)
                            .is_some_and(|(at, _)| now - *at >= self.limits.retry_milliseconds as f64);
                        if !due {
                            continue;
                        }
                    } else {
                        continue;
                    }
                } else {
                    message.next += 1;
                }
                let attempts = message.sent.get(&fragment).map_or(0, |(_, attempts)| *attempts) + 1;
                if attempts > self.limits.maximum_transmissions {
                    self.close();
                    return Err(UnifiedError::RetryLimit);
                }
                message.sent.insert(fragment, (now, attempts));
                (fragment, attempts)
            };
            let _ = attempts;
            let packet = self.encode_data(&self.reliable[index], true, fragment)?;
            return Ok(Some(packet));
        }
        Ok(None)
    }

    fn send_frame(&mut self) -> Result<Option<Vec<u8>>, UnifiedError> {
        let Some(message) = self.frame.as_mut() else {
            return Ok(None);
        };
        let fragment = message.next as u16;
        message.next += 1;
        let packet = self.encode_data(
            self.frame
                .as_ref()
                .unwrap_or_else(|| unreachable!("frame was just checked")),
            false,
            fragment,
        )?;
        if self
            .frame
            .as_ref()
            .is_some_and(|message| message.next == message.fragments)
        {
            self.frame = self.pending_frame.take();
        }
        Ok(Some(packet))
    }

    fn send_ack(&mut self) -> Result<Option<Vec<u8>>, UnifiedError> {
        let Some(key) = self.acknowledgments.keys().next().cloned() else {
            return Ok(None);
        };
        let (sequence, fragment) = self.acknowledgments.remove(&key).unwrap_or((0, 0));
        Ok(Some(encode_unified_packet(&UnifiedPacket::Ack {
            token: self.token.clone(),
            sequence,
            acknowledged_reliable: self.reliable_received,
            fragment,
        })?))
    }

    /// Flush queued packets (`flush`).
    pub fn flush(&mut self, now: f64) -> Result<Vec<Vec<u8>>, UnifiedError> {
        self.open()?;
        Self::check_time(now)?;
        self.expire(now)?;
        let mut packets = Vec::new();
        while packets.len() < self.limits.packets_per_flush {
            let mut progress = false;
            for _ in 0..3 {
                if packets.len() == self.limits.packets_per_flush {
                    break;
                }
                let lane = self.lane;
                self.lane = (self.lane + 1) % 3;
                let packet = if lane == 0 {
                    self.send_ack()?
                } else if lane == 1 {
                    self.send_reliable(now)?
                } else {
                    self.send_frame()?
                };
                if let Some(packet) = packet {
                    packets.push(packet);
                    progress = true;
                }
            }
            if !progress {
                break;
            }
        }
        Ok(packets)
    }

    /// Close the channel (`close`).
    pub fn close(&mut self) {
        self.ended = true;
        self.reliable.clear();
        self.queued_bytes = 0;
        self.frame = None;
        self.pending_frame = None;
        self.reliable_assemblies.clear();
        self.received_bytes = 0;
        self.frame_assembly = None;
        self.waiting_frame = None;
        self.acknowledgments.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn packet_framing_round_trips() {
        let packet = UnifiedPacket::Reliable {
            token: TOKEN.to_owned(),
            sequence: 3,
            acknowledged_reliable: 1,
            required_reliable_sequence: 0,
            total_bytes: 3,
            fragment_bytes: 1152,
            fragment: 0,
            fragments: 1,
            payload: vec![9, 8, 7],
        };
        let bytes = encode_unified_packet(&packet).unwrap();
        assert_eq!(bytes.len(), UNIFIED_PACKET_HEADER_BYTES + 3);
        assert_eq!(&bytes[0..6], &[b'Q', b'T', b'U', b'C', 1, 1]);
        assert_eq!(decode_unified_packet(&bytes), Some(packet));
        assert!(decode_unified_packet(&bytes[..UNIFIED_PACKET_HEADER_BYTES - 1]).is_none());
    }

    #[test]
    fn channel_delivers_reliable_and_frame() {
        let limits = UnifiedChannelLimits::default();
        let mut sender = UnifiedChannel::new(TOKEN, limits).unwrap();
        let mut receiver = UnifiedChannel::new(TOKEN, limits).unwrap();
        sender.queue_reliable(&[1, 2, 3]).unwrap();
        sender.queue_frame(&[9], 0).unwrap();
        let packets = sender.flush(0.0).unwrap();
        assert!(!packets.is_empty());
        let mut delivered = Vec::new();
        for packet in &packets {
            delivered.extend(receiver.receive(packet, 1.0).unwrap());
        }
        assert!(delivered
            .iter()
            .any(|delivery| matches!(delivery, UnifiedDelivery::Reliable { sequence: 1, .. })));
        assert!(delivered
            .iter()
            .any(|delivery| matches!(delivery, UnifiedDelivery::Frame { sequence: 1, .. })));
    }
}
