//! Quake 1 channels, sessions, handshakes, and message codecs.
//!
//! Donor provenance: `channels.ts`, `handshake.ts`, `session.ts`,
//! `discovery.ts`, `prediction.ts`, `recording.ts`, `qw-recording.ts`,
//! `checksum.ts`, `commands.ts`, `netquake.ts`, and `quakeworld.ts` under
//! `src/network/q1/`, with token parsing from `core/common-parse.ts`
//! (`parseQ1Token`). Wide/NQ15/QW28 field codecs live in [`crate::q1`],
//! [`crate::qw`], and [`crate::q1_wide`]; this module owns packet framing,
//! session state machines, discovery, prediction history, demo capture,
//! and the full NetQuake/QuakeWorld message enums.

use std::collections::{BTreeMap, HashMap};

use thiserror::Error;

use crate::common::endpoint::{address_key, same_address, NetworkAddress};
use crate::common::hash::md4_block_checksum;
use crate::common::reliability::{StopAndWaitChannel, ToggleReceive, ToggleReliableChannel};
use crate::common::scheduling::PacketRate;
use crate::common::transport::DatagramTransport;
use crate::msg::{MsgError, MsgReader, MsgWriter};
use crate::protocol::q1 as protocol;
use crate::protocol::qw as qw_protocol;
use crate::q1::ClientData;
use crate::q1_chktbl::QW_CHKTBL;
use crate::q1_wide::{NqProfile, QwProfile, WideEntityState};
use crate::qw::{read_delta_usercmd, write_delta_usercmd, QwUsercmd};
use crate::services::discovery::{DiscoveryError, DiscoveryRequestKind, ServerStatus};

/// Error for Quake 1 session and message failures.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum Q1NetError {
    /// Message read overflow.
    #[error("Quake 1 message overflow")]
    Overflow,
    /// Unknown NetQuake service.
    #[error("Unknown NetQuake service {0}")]
    UnknownService(u8),
    /// Unknown QuakeWorld service.
    #[error("Unknown QuakeWorld service {0}")]
    UnknownQwService(u8),
    /// Unknown NetQuake client service.
    #[error("Unknown NetQuake client service {0}")]
    UnknownClientService(u8),
    /// Unknown temporary entity.
    #[error("Unknown temporary entity {0}")]
    UnknownTempEntity(u8),
    /// Short NetQuake header.
    #[error("Short NetQuake header")]
    ShortHeader,
    /// NetQuake length mismatch.
    #[error("NetQuake length mismatch")]
    LengthMismatch,
    /// Unknown NetQuake packet flags.
    #[error("Unknown NetQuake packet flags")]
    UnknownFlags,
    /// NetQuake datagram exceeds 16-bit header.
    #[error("NetQuake datagram exceeds 16-bit header")]
    DatagramTooLarge,
    /// NetQuake unreliable message exceeds profile.
    #[error("NetQuake unreliable message exceeds profile")]
    UnreliableTooLarge,
    /// Short QuakeWorld header.
    #[error("Short QuakeWorld header")]
    ShortQwHeader,
    /// Invalid qport.
    #[error("Invalid qport")]
    BadQport,
    /// Invalid challenge.
    #[error("Invalid challenge")]
    BadChallenge,
    /// Invalid QuakeWorld profile.
    #[error("Invalid QuakeWorld profile")]
    BadProfile,
    /// Invalid signon stage.
    #[error("Invalid signon stage {0} after {1}")]
    BadSignonStage(u8, u8),
    /// Unsorted packet entities.
    #[error("Unsorted packet entities")]
    UnsortedEntities,
    /// Removal in full packet entities.
    #[error("Removal in full packet entities")]
    RemovalInFull,
    /// Packet entities overflow.
    #[error("Packet entities overflow")]
    EntitiesOverflow,
    /// QW precache overflow.
    #[error("QW precache overflow")]
    PrecacheOverflow,
    /// Player slot out of range.
    #[error("Player slot out of range")]
    BadPlayerSlot,
    /// Too many nail projectiles.
    #[error("Too many nail projectiles")]
    TooManyNails,
    /// QuakeWorld kick must be -2 or -4.
    #[error("QuakeWorld kick must be -2 or -4")]
    BadKick,
    /// NetQuake color explosion has no QuakeWorld opcode.
    #[error("NetQuake color explosion has no QuakeWorld opcode")]
    ColorExplosion,
    /// Sound exceeds selected wire.
    #[error("Sound exceeds selected wire")]
    SoundExceedsWire,
    /// Static exceeds selected wire.
    #[error("Static exceeds selected wire")]
    StaticExceedsWire,
    /// Entity exceeds QuakeWorld profile.
    #[error("Entity {0} exceeds QuakeWorld profile")]
    EntityExceedsProfile(u32),
    /// Too many QuakeWorld packet entities.
    #[error("Too many QuakeWorld packet entities")]
    TooManyEntities,
    /// QuakeWorld download block exceeds source block size.
    #[error("QuakeWorld download block exceeds source block size")]
    DownloadTooLarge,
    /// Invalid rerelease private message.
    #[error("Invalid rerelease private message")]
    BadPrivate,
    /// Unknown rerelease message.
    #[error("Unknown rerelease message {0}")]
    UnknownPrivate(u8),
    /// Short control header.
    #[error("Short control header")]
    ShortControl,
    /// Invalid control header.
    #[error("Invalid control header")]
    BadControl,
    /// Unknown control command.
    #[error("Unknown control command {0}")]
    UnknownControl(u8),
    /// Not an out-of-band packet.
    #[error("Not an out-of-band packet")]
    NotOutOfBand,
    /// Invalid accepted port.
    #[error("Invalid accepted port")]
    BadAcceptedPort,
    /// Selected QW profile cannot represent precache.
    #[error("Selected QW profile cannot represent precache")]
    PrecacheUnrepresentable,
    /// Precache list skipped entries.
    #[error("Precache list skipped entries")]
    PrecacheSkipped,
    /// Multiple moves in one packet.
    #[error("Multiple moves in one packet")]
    MultipleMoves,
    /// Invalid movement checksum.
    #[error("Invalid movement checksum")]
    BadMoveChecksum,
    /// Unknown QuakeWorld client message.
    #[error("Unknown QuakeWorld client message")]
    UnknownClientMessage,
    /// QuakeWorld upload block too large.
    #[error("QuakeWorld upload block too large")]
    UploadTooLarge,
    /// Three recorded commands are required.
    #[error("Three recorded commands are required")]
    MissingCommands,
    /// NetQuake recording has no server info.
    #[error("NetQuake recording has no server info")]
    NoServerInfo,
    /// Short Quake BSP header.
    #[error("Short Quake BSP header")]
    ShortBsp,
    /// QW requires BSP version 29.
    #[error("QW requires BSP version 29")]
    BadBspVersion,
    /// Invalid Quake BSP lump.
    #[error("Invalid Quake BSP lump")]
    BadBspLump,
    /// Invalid sequence.
    #[error("Invalid sequence")]
    BadSequence,
    /// Not a QuakeWorld status reply.
    #[error("Not a QuakeWorld status reply")]
    NotStatusReply,
    /// Not a compatible NetQuake discovery response.
    #[error("Not a compatible NetQuake discovery response")]
    NotDiscoveryResponse,
    /// NetQuake master discovery is not configured.
    #[error("NetQuake master discovery is not configured")]
    NoMasterDiscovery,
    /// NetQuake master registration is not configured.
    #[error("NetQuake master registration is not configured")]
    NoMasterHeartbeat,
    /// Use an HTTP QuakeWorld master list.
    #[error("Use an HTTP QuakeWorld master list")]
    HttpMasterOnly,
    /// QuakeWorld heartbeat requires its source sequence and active client count.
    #[error("QuakeWorld heartbeat requires its source sequence and active client count")]
    HeartbeatNeedsSource,
    /// Download source failure.
    #[error("Download source failure: {0}")]
    DownloadSource(String),
}

impl From<MsgError> for Q1NetError {
    fn from(_: MsgError) -> Self {
        Q1NetError::Overflow
    }
}

// ---------------------------------------------------------------------------
// Token parsing (parseQ1Token)
// ---------------------------------------------------------------------------

/// Tokenizer dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1TokenDialect {
    /// NetQuake with single-character punctuation tokens.
    Netquake,
    /// QuakeWorld words.
    Quakeworld,
}

/// Tokenizer state over a Latin-1 string (`LegacyParseState`).
#[derive(Debug, Clone, Default)]
pub struct Q1Tokenizer {
    /// Character index.
    pub index: usize,
}

fn byte_at(data: &[u8], index: usize) -> u8 {
    data.get(index).copied().unwrap_or(0)
}

fn signed_byte_at(data: &[u8], index: usize) -> i16 {
    let byte = byte_at(data, index);
    if byte >= 128 {
        i16::from(byte) - 256
    } else {
        i16::from(byte)
    }
}

fn is_single_char_token(byte: u8) -> bool {
    matches!(byte, b'{' | b'}' | b')' | b'(' | b'\'' | b':')
}

/// Parse one token (`parseQ1Token`).
///
/// Operates on Latin-1 bytes like the donor's `charCodeAt` string. Returns
/// `None` at end-of-input for both dialects; an unterminated quote returns
/// the accumulated token.
pub fn parse_q1_token(data: &[u8], state: &mut Q1Tokenizer, dialect: Q1TokenDialect) -> Option<String> {
    let mut index = state.index;
    loop {
        let mut current = signed_byte_at(data, index);
        while current <= 32 {
            if current == 0 {
                state.index = index;
                return None;
            }
            index += 1;
            current = signed_byte_at(data, index);
        }
        if byte_at(data, index) == b'/' && byte_at(data, index + 1) == b'/' {
            while byte_at(data, index) != 0 && byte_at(data, index) != b'\n' {
                index += 1;
            }
            continue;
        }
        break;
    }
    let first = byte_at(data, index);
    if first == b'"' {
        index += 1;
        let mut token = Vec::new();
        loop {
            let byte = byte_at(data, index);
            index += 1;
            if byte == b'"' || byte == 0 {
                state.index = index;
                return Some(token.iter().map(|byte| *byte as char).collect());
            }
            token.push(byte);
        }
    }
    if dialect != Q1TokenDialect::Quakeworld && is_single_char_token(first) {
        state.index = index + 1;
        return Some(String::from(first as char));
    }
    let mut token = Vec::new();
    let mut byte = first;
    loop {
        token.push(byte);
        index += 1;
        byte = byte_at(data, index);
        if dialect != Q1TokenDialect::Quakeworld && is_single_char_token(byte) {
            break;
        }
        if signed_byte_at(data, index) <= 32 {
            break;
        }
    }
    state.index = index;
    Some(token.iter().map(|byte| *byte as char).collect())
}

/// Tokenize QuakeWorld command text (`quakeWorldCommandArguments`).
#[must_use]
pub fn quake_world_command_arguments(text: &str) -> Vec<String> {
    let mut state = Q1Tokenizer::default();
    let data = text.as_bytes();
    let mut args = Vec::new();
    while let Some(token) = parse_q1_token(data, &mut state, Q1TokenDialect::Quakeworld) {
        args.push(token);
    }
    args
}

/// Parse a backslash info string (`quakeWorldInfo`).
#[must_use]
pub fn quake_world_info(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let parts: Vec<&str> = text.split('\\').collect();
    let offset = usize::from(parts.first() == Some(&""));
    let mut index = offset;
    while index + 1 < parts.len() {
        out.insert(parts[index].to_owned(), parts[index + 1].to_owned());
        index += 2;
    }
    out
}

fn info_string(values: &BTreeMap<String, String>) -> String {
    let mut text = String::new();
    for (key, value) in values {
        text.push('\\');
        text.push_str(key);
        text.push('\\');
        text.push_str(value);
    }
    text
}

// ---------------------------------------------------------------------------
// Channels (q1/channels.ts)
// ---------------------------------------------------------------------------

/// NetQuake data flag.
pub const NETFLAG_DATA: u32 = 0x10000;
/// NetQuake acknowledgment flag.
pub const NETFLAG_ACK: u32 = 0x20000;
/// NetQuake end-of-message flag.
pub const NETFLAG_EOM: u32 = 0x80000;
/// NetQuake unreliable flag.
pub const NETFLAG_UNRELIABLE: u32 = 0x100000;
/// NetQuake control flag.
pub const NETFLAG_CTL: u32 = 0x80000000;

/// Channel delivery (`ChannelDelivery`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelDelivery {
    /// Reliable or unreliable.
    pub reliable: bool,
    /// Payload.
    pub payload: Vec<u8>,
    /// Sequence.
    pub sequence: u32,
    /// Dropped packets before this one.
    pub dropped: u32,
}

/// Channel receive result (`ChannelReceive`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelReceive {
    /// Delivery, if any.
    pub delivery: Option<ChannelDelivery>,
    /// Replies to transmit.
    pub replies: Vec<Vec<u8>>,
}

fn nq_packet(flags: u32, sequence: u32, payload: &[u8]) -> Result<Vec<u8>, Q1NetError> {
    if payload.len() > 65527 {
        return Err(Q1NetError::DatagramTooLarge);
    }
    let mut bytes = vec![0u8; payload.len() + 8];
    let word = flags | bytes.len() as u32;
    bytes[0..4].copy_from_slice(&word.to_be_bytes());
    bytes[4..8].copy_from_slice(&sequence.to_be_bytes());
    bytes[8..].copy_from_slice(payload);
    Ok(bytes)
}

/// NetQuake stop-and-wait channel (`NetQuakeChannel`).
#[derive(Debug)]
pub struct NetQuakeChannel {
    reliable: StopAndWaitChannel,
    unreliable_send: u32,
    unreliable_receive: u32,
    /// Maximum message bytes.
    pub max_message_bytes: usize,
    /// Fragment bytes.
    pub fragment_bytes: usize,
}

impl NetQuakeChannel {
    /// Create a channel with message and fragment limits.
    pub fn new(max_message_bytes: usize, fragment_bytes: usize) -> Result<Self, Q1NetError> {
        Ok(Self {
            reliable: StopAndWaitChannel::new(max_message_bytes, fragment_bytes).map_err(|_| Q1NetError::Overflow)?,
            unreliable_send: 0,
            unreliable_receive: 0,
            max_message_bytes,
            fragment_bytes,
        })
    }

    /// True when no reliable message awaits acknowledgment.
    #[must_use]
    pub fn can_send_reliable(&self) -> bool {
        self.reliable.can_send()
    }

    /// Queue a reliable message (`queueReliable`).
    pub fn queue_reliable(&mut self, payload: &[u8]) -> Result<(), Q1NetError> {
        self.reliable.begin(payload).map_err(|_| Q1NetError::Overflow)
    }

    /// Next reliable fragment packet (`next`).
    pub fn next(&mut self, now: f64) -> Result<Option<Vec<u8>>, Q1NetError> {
        let Some(fragment) = self.reliable.next(now) else {
            return Ok(None);
        };
        Ok(Some(nq_packet(
            NETFLAG_DATA | if fragment.final_fragment { NETFLAG_EOM } else { 0 },
            fragment.sequence,
            &fragment.payload,
        )?))
    }

    /// Frame an unreliable message (`unreliable`).
    pub fn unreliable(&mut self, payload: &[u8]) -> Result<Vec<u8>, Q1NetError> {
        if payload.len() > self.max_message_bytes {
            return Err(Q1NetError::UnreliableTooLarge);
        }
        let sequence = self.unreliable_send;
        self.unreliable_send = self.unreliable_send.wrapping_add(1);
        nq_packet(NETFLAG_UNRELIABLE, sequence, payload)
    }

    /// Receive a datagram (`receive`).
    pub fn receive(&mut self, bytes: &[u8], now: f64) -> Result<ChannelReceive, Q1NetError> {
        if bytes.len() < 8 {
            return Err(Q1NetError::ShortHeader);
        }
        let word = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let flags = word & 0xffff_0000;
        let sequence = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if word & 65535 != bytes.len() as u32 {
            return Err(Q1NetError::LengthMismatch);
        }
        if flags & NETFLAG_CTL != 0 {
            return Ok(ChannelReceive {
                delivery: None,
                replies: Vec::new(),
            });
        }
        let payload = bytes[8..].to_vec();
        if flags & NETFLAG_UNRELIABLE != 0 {
            if sequence < self.unreliable_receive {
                return Ok(ChannelReceive {
                    delivery: None,
                    replies: Vec::new(),
                });
            }
            let dropped = sequence - self.unreliable_receive;
            self.unreliable_receive = sequence + 1;
            return Ok(ChannelReceive {
                delivery: Some(ChannelDelivery {
                    reliable: false,
                    payload,
                    sequence,
                    dropped,
                }),
                replies: Vec::new(),
            });
        }
        if flags & NETFLAG_ACK != 0 {
            self.reliable.acknowledge(sequence);
            let next = self.next(now)?;
            return Ok(ChannelReceive {
                delivery: None,
                replies: next.into_iter().collect(),
            });
        }
        if flags & NETFLAG_DATA != 0 {
            let result = self
                .reliable
                .receive(&crate::common::reliability::ReliableFragment {
                    sequence,
                    final_fragment: flags & NETFLAG_EOM != 0,
                    payload,
                })
                .map_err(|_| Q1NetError::Overflow)?;
            let (delivery, acknowledge) = match result {
                crate::common::reliability::ReliableFragmentReceive::Duplicate { acknowledge }
                | crate::common::reliability::ReliableFragmentReceive::Fragment { acknowledge } => (None, acknowledge),
                crate::common::reliability::ReliableFragmentReceive::Message { acknowledge, payload } => (
                    Some(ChannelDelivery {
                        reliable: true,
                        payload,
                        sequence,
                        dropped: 0,
                    }),
                    acknowledge,
                ),
            };
            return Ok(ChannelReceive {
                delivery,
                replies: vec![nq_packet(NETFLAG_ACK, acknowledge, &[])?],
            });
        }
        Err(Q1NetError::UnknownFlags)
    }
}

/// QuakeWorld delivery (`QuakeWorldDelivery`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuakeWorldDelivery {
    /// Payload.
    pub payload: Vec<u8>,
    /// Sequence.
    pub sequence: u32,
    /// Acknowledged sequence.
    pub acknowledged: u32,
    /// Dropped packets before this one.
    pub dropped: u32,
}

/// QuakeWorld channel side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuakeWorldSide {
    /// Client (10-byte header with qport).
    Client,
    /// Server (8-byte header).
    Server,
}

/// QuakeWorld toggle channel (`QuakeWorldChannel`).
#[derive(Debug)]
pub struct QuakeWorldChannel {
    reliable: ToggleReliableChannel,
    packet_rate: PacketRate,
    /// Last receive time in milliseconds.
    pub last_received: f64,
    /// Smoothed frame latency.
    pub frame_latency: f64,
    /// Smoothed frame rate.
    pub frame_rate: f64,
    /// Channel side.
    pub side: QuakeWorldSide,
    /// QPort.
    pub qport: u16,
    /// Maximum message bytes.
    pub max_message_bytes: usize,
    /// Bytes per second.
    pub bytes_per_second: f64,
}

impl QuakeWorldChannel {
    /// Create a channel with side, qport, message, and rate limits.
    pub fn new(
        side: QuakeWorldSide,
        qport: u32,
        max_message_bytes: usize,
        bytes_per_second: f64,
    ) -> Result<Self, Q1NetError> {
        if qport > 65535 {
            return Err(Q1NetError::BadQport);
        }
        Ok(Self {
            reliable: ToggleReliableChannel::with_first_sequence(max_message_bytes, 0)
                .map_err(|_| Q1NetError::Overflow)?,
            packet_rate: PacketRate::new(bytes_per_second).map_err(|_| Q1NetError::Overflow)?,
            last_received: 0.0,
            frame_latency: 0.0,
            frame_rate: 0.0,
            side,
            qport: qport as u16,
            max_message_bytes,
            bytes_per_second,
        })
    }

    /// Incoming sequence.
    #[must_use]
    pub fn incoming_sequence(&self) -> i64 {
        self.reliable.incoming_sequence()
    }

    /// Outgoing sequence.
    #[must_use]
    pub fn outgoing_sequence(&self) -> i64 {
        self.reliable.outgoing_sequence()
    }

    /// True while reliable bytes are queued or unacknowledged.
    #[must_use]
    pub fn has_reliable(&self) -> bool {
        self.reliable.has_pending_reliable()
    }

    /// Queue reliable bytes (`queueReliable`).
    pub fn queue_reliable(&mut self, bytes: &[u8]) -> Result<(), Q1NetError> {
        self.reliable.queue(bytes).map_err(|_| Q1NetError::Overflow)
    }

    /// Clear-time send check (`canPacket`).
    pub fn can_packet(&mut self, now: f64) -> bool {
        self.packet_rate.bytes_per_second = self.bytes_per_second;
        self.packet_rate.can_send(now, false)
    }

    /// Transmit a packet (`transmit`).
    pub fn transmit(&mut self, unreliable: &[u8], now: f64, server_paused: bool) -> Result<Vec<u8>, Q1NetError> {
        let packet = self
            .reliable
            .transmit(unreliable, None)
            .map_err(|_| Q1NetError::Overflow)?;
        let header = if self.side == QuakeWorldSide::Client { 10 } else { 8 };
        let mut bytes = vec![0u8; header + packet.payload.len()];
        let sequence = packet.sequence as u32 | if packet.reliable { 0x8000_0000 } else { 0 };
        let acknowledged = packet.acknowledged as u32 | u32::from(packet.reliable_acknowledged) << 31;
        bytes[0..4].copy_from_slice(&sequence.to_le_bytes());
        bytes[4..8].copy_from_slice(&acknowledged.to_le_bytes());
        if self.side == QuakeWorldSide::Client {
            bytes[8..10].copy_from_slice(&self.qport.to_le_bytes());
        }
        bytes[header..].copy_from_slice(&packet.payload);
        self.packet_rate.bytes_per_second = self.bytes_per_second;
        self.packet_rate
            .sent(bytes.len(), now, self.side == QuakeWorldSide::Server && server_paused);
        Ok(bytes)
    }

    /// Receive a packet (`receive`).
    pub fn receive(&mut self, bytes: &[u8], now: f64) -> Result<Option<QuakeWorldDelivery>, Q1NetError> {
        let header = if self.side == QuakeWorldSide::Server { 10 } else { 8 };
        if bytes.len() < header {
            return Err(Q1NetError::ShortQwHeader);
        }
        if self.side == QuakeWorldSide::Server && u16::from_le_bytes([bytes[8], bytes[9]]) != self.qport {
            return Ok(None);
        }
        let word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let ack = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        if word == 0xffff_ffff {
            return Ok(None);
        }
        let sequence = word & 0x7fff_ffff;
        let acknowledged = ack & 0x7fff_ffff;
        let result = self.reliable.receive(&crate::common::reliability::TogglePacket {
            sequence: i64::from(sequence),
            acknowledged: i64::from(acknowledged),
            reliable: word >> 31 != 0,
            reliable_acknowledged: u8::from(ack >> 31 != 0),
            payload: bytes[header..].to_vec(),
        });
        let ToggleReceive::Accepted { dropped, payload } = result else {
            return Ok(None);
        };
        self.frame_latency =
            self.frame_latency * 0.99 + (self.outgoing_sequence() - i64::from(acknowledged)) as f64 * 0.01;
        self.frame_rate = self.frame_rate * 0.99 + (now - self.last_received) * 0.01;
        self.last_received = now;
        if self.side == QuakeWorldSide::Server && i64::from(sequence) >= self.outgoing_sequence() {
            let _ = self.reliable.advance_outgoing_sequence(i64::from(sequence));
        }
        Ok(Some(QuakeWorldDelivery {
            payload,
            sequence,
            acknowledged,
            dropped: dropped.max(0) as u32,
        }))
    }
}

/// Channel peer over a datagram transport (`QuakePeer`).
pub struct QuakePeer {
    transport: Box<dyn DatagramTransport<Address = NetworkAddress>>,
    /// Remote endpoint (rebindable port on QW servers).
    pub remote: NetworkAddress,
    channel: QuakePeerChannel,
}

/// Peer channel variant.
pub enum QuakePeerChannel {
    /// NetQuake channel.
    NetQuake(NetQuakeChannel),
    /// QuakeWorld channel.
    QuakeWorld(QuakeWorldChannel),
}

/// Peer delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuakePeerDelivery {
    /// NetQuake delivery.
    NetQuake(ChannelDelivery),
    /// QuakeWorld delivery.
    QuakeWorld(QuakeWorldDelivery),
}

impl QuakePeer {
    /// Create a peer over a transport.
    pub fn new(
        transport: Box<dyn DatagramTransport<Address = NetworkAddress>>,
        remote: NetworkAddress,
        channel: QuakePeerChannel,
    ) -> Self {
        Self {
            transport,
            remote,
            channel,
        }
    }

    /// Send bytes to the remote endpoint.
    pub fn send(&self, bytes: &[u8]) -> bool {
        self.transport.send(&self.remote, bytes).unwrap_or(false)
    }

    /// Receive a datagram, answering NQ reliable fragments inline.
    pub fn receive(
        &mut self,
        from: &NetworkAddress,
        bytes: &[u8],
        now: f64,
    ) -> Result<Option<QuakePeerDelivery>, Q1NetError> {
        match &mut self.channel {
            QuakePeerChannel::NetQuake(channel) => {
                if !same_address(from, &self.remote, true) {
                    return Ok(None);
                }
                let result = channel.receive(bytes, now)?;
                for reply in &result.replies {
                    self.send(reply);
                }
                Ok(result.delivery.map(QuakePeerDelivery::NetQuake))
            }
            QuakePeerChannel::QuakeWorld(channel) => {
                if !same_address(from, &self.remote, channel.side == QuakeWorldSide::Client) {
                    return Ok(None);
                }
                let result = channel.receive(bytes, now)?;
                if result.is_some() {
                    self.remote = from.clone();
                }
                Ok(result.map(QuakePeerDelivery::QuakeWorld))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Handshake (q1/handshake.ts)
// ---------------------------------------------------------------------------

/// NetQuake control message (`NetQuakeControl`).
#[derive(Debug, Clone, PartialEq)]
pub enum NetQuakeControl {
    /// Connection request.
    ConnectRequest {
        /// Game name.
        game: String,
        /// Net protocol version.
        version: u8,
    },
    /// Server info request.
    ServerInfoRequest {
        /// Game name.
        game: String,
        /// Net protocol version.
        version: u8,
    },
    /// Player info request.
    PlayerInfoRequest {
        /// Player slot.
        player: u8,
    },
    /// Rule info request.
    RuleInfoRequest {
        /// Previous rule name.
        previous: String,
    },
    /// Connection accepted.
    Accept {
        /// Server port.
        port: i32,
    },
    /// Connection rejected.
    Reject {
        /// Reason.
        reason: String,
    },
    /// Server info.
    ServerInfo {
        /// Address text.
        address: String,
        /// Server name.
        name: String,
        /// Map name.
        map: String,
        /// Player count.
        players: u8,
        /// Maximum players.
        max_players: u8,
        /// Net protocol version.
        version: u8,
    },
    /// Player info.
    PlayerInfo {
        /// Player slot.
        player: u8,
        /// Name.
        name: String,
        /// Colors.
        colors: i32,
        /// Frags.
        frags: i32,
        /// Connected seconds.
        seconds: i32,
        /// Address text.
        address: String,
    },
    /// Rule info.
    RuleInfo {
        /// Rule name/value or end of rules.
        rule: Option<(String, String)>,
    },
}

/// Encode a control message (`encodeNetQuakeControl`).
pub fn encode_net_quake_control(message: &NetQuakeControl) -> Result<Vec<u8>, Q1NetError> {
    let mut writer = MsgWriter::new(65535, false);
    writer.write_long(0)?;
    match message {
        NetQuakeControl::ConnectRequest { game, version } | NetQuakeControl::ServerInfoRequest { game, version } => {
            writer.write_byte(if matches!(message, NetQuakeControl::ConnectRequest { .. }) {
                1
            } else {
                2
            })?;
            writer.write_string(game)?;
            writer.write_byte(*version)?;
        }
        NetQuakeControl::PlayerInfoRequest { player } => {
            writer.write_byte(3)?;
            writer.write_byte(*player)?;
        }
        NetQuakeControl::RuleInfoRequest { previous } => {
            writer.write_byte(4)?;
            writer.write_string(previous)?;
        }
        NetQuakeControl::Accept { port } => {
            writer.write_byte(0x81)?;
            writer.write_long(*port)?;
        }
        NetQuakeControl::Reject { reason } => {
            writer.write_byte(0x82)?;
            writer.write_string(reason)?;
        }
        NetQuakeControl::ServerInfo {
            address,
            name,
            map,
            players,
            max_players,
            version,
        } => {
            writer.write_byte(0x83)?;
            writer.write_string(address)?;
            writer.write_string(name)?;
            writer.write_string(map)?;
            writer.write_byte(*players)?;
            writer.write_byte(*max_players)?;
            writer.write_byte(*version)?;
        }
        NetQuakeControl::PlayerInfo {
            player,
            name,
            colors,
            frags,
            seconds,
            address,
        } => {
            writer.write_byte(0x84)?;
            writer.write_byte(*player)?;
            writer.write_string(name)?;
            writer.write_long(*colors)?;
            writer.write_long(*frags)?;
            writer.write_long(*seconds)?;
            writer.write_string(address)?;
        }
        NetQuakeControl::RuleInfo { rule: None } => {
            writer.write_byte(0x85)?;
        }
        NetQuakeControl::RuleInfo {
            rule: Some((name, value)),
        } => {
            writer.write_byte(0x85)?;
            writer.write_string(name)?;
            writer.write_string(value)?;
        }
    }
    let mut bytes = writer.bytes().to_vec();
    let header = NETFLAG_CTL | bytes.len() as u32;
    bytes[0..4].copy_from_slice(&header.to_be_bytes());
    Ok(bytes)
}

/// Decode a control message (`decodeNetQuakeControl`).
pub fn decode_net_quake_control(bytes: &[u8]) -> Result<NetQuakeControl, Q1NetError> {
    if bytes.len() < 5 {
        return Err(Q1NetError::ShortControl);
    }
    let header = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if header & 65535 != bytes.len() as u32 || header >> 16 != 0x8000 {
        return Err(Q1NetError::BadControl);
    }
    let mut reader = MsgReader::new(&bytes[4..]);
    let op = reader.byte()?;
    let message = match op {
        1 | 2 => {
            let game = reader.string(512);
            let version = reader.byte()?;
            if op == 1 {
                NetQuakeControl::ConnectRequest { game, version }
            } else {
                NetQuakeControl::ServerInfoRequest { game, version }
            }
        }
        3 => NetQuakeControl::PlayerInfoRequest { player: reader.byte()? },
        4 => NetQuakeControl::RuleInfoRequest {
            previous: reader.string(512),
        },
        0x81 => NetQuakeControl::Accept { port: reader.long()? },
        0x82 => NetQuakeControl::Reject {
            reason: reader.string(512),
        },
        0x83 => NetQuakeControl::ServerInfo {
            address: reader.string(512),
            name: reader.string(512),
            map: reader.string(512),
            players: reader.byte()?,
            max_players: reader.byte()?,
            version: reader.byte()?,
        },
        0x84 => NetQuakeControl::PlayerInfo {
            player: reader.byte()?,
            name: reader.string(512),
            colors: reader.long()?,
            frags: reader.long()?,
            seconds: reader.long()?,
            address: reader.string(512),
        },
        0x85 => NetQuakeControl::RuleInfo {
            rule: if reader.remaining() > 0 {
                Some((reader.string(512), reader.string(512)))
            } else {
                None
            },
        },
        _ => return Err(Q1NetError::UnknownControl(op)),
    };
    reader.finish()?;
    Ok(message)
}

/// NetQuake connection host (`NetQuakeConnectionHost`).
pub trait NetQuakeConnectionHost {
    /// Server info reply.
    fn server_info(&self) -> NetQuakeControl;
    /// Player info reply or silent drop.
    fn player_info(&self, index: u8) -> Option<NetQuakeControl>;
    /// Next rule after `previous`.
    fn next_rule(&self, previous: &str) -> Option<(String, String)>;
    /// Connection verdict.
    fn connect(&mut self, from: &NetworkAddress, now: f64) -> NetQuakeConnectVerdict;
}

/// Connection verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetQuakeConnectVerdict {
    /// Accepted with a server port.
    Accepted {
        /// Server port.
        port: i32,
    },
    /// Rejected with a reason.
    Rejected {
        /// Reason.
        reason: String,
    },
    /// Silent retry.
    Retry,
}

/// Answer a control datagram (`answerNetQuakeControl`).
pub fn answer_net_quake_control(
    bytes: &[u8],
    from: &NetworkAddress,
    now: f64,
    host: &mut dyn NetQuakeConnectionHost,
) -> Result<Option<Vec<u8>>, Q1NetError> {
    let message = decode_net_quake_control(bytes)?;
    match message {
        NetQuakeControl::ServerInfoRequest { game, .. } => {
            if game != "QUAKE" {
                return Ok(None);
            }
            Ok(Some(encode_net_quake_control(&host.server_info())?))
        }
        NetQuakeControl::PlayerInfoRequest { player } => match host.player_info(player) {
            None => Ok(None),
            Some(info) => Ok(Some(encode_net_quake_control(&info)?)),
        },
        NetQuakeControl::RuleInfoRequest { previous } => {
            Ok(Some(encode_net_quake_control(&NetQuakeControl::RuleInfo {
                rule: host.next_rule(&previous),
            })?))
        }
        NetQuakeControl::ConnectRequest { game, version } => {
            if game != "QUAKE" {
                return Ok(None);
            }
            if version != 3 {
                return Ok(Some(encode_net_quake_control(&NetQuakeControl::Reject {
                    reason: "Incompatible version.\n".to_owned(),
                })?));
            }
            match host.connect(from, now) {
                NetQuakeConnectVerdict::Retry => Ok(None),
                NetQuakeConnectVerdict::Accepted { port } => {
                    Ok(Some(encode_net_quake_control(&NetQuakeControl::Accept { port })?))
                }
                NetQuakeConnectVerdict::Rejected { reason } => {
                    Ok(Some(encode_net_quake_control(&NetQuakeControl::Reject { reason })?))
                }
            }
        }
        _ => Ok(None),
    }
}

/// Format a QuakeWorld out-of-band packet (`quakeWorldOutOfBand`).
#[must_use]
pub fn quake_world_out_of_band(text: &str, nul: bool) -> Vec<u8> {
    let mut bytes = vec![255u8, 255, 255, 255];
    for ch in text.chars() {
        bytes.push((ch as u32 & 255) as u8);
    }
    if nul {
        bytes.push(0);
    }
    bytes
}

/// Read a QuakeWorld out-of-band packet (`readQuakeWorldOutOfBand`).
pub fn read_quake_world_out_of_band(bytes: &[u8]) -> Result<String, Q1NetError> {
    if bytes.len() < 4 || i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) != -1 {
        return Err(Q1NetError::NotOutOfBand);
    }
    Ok(bytes[4..]
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| *byte as char)
        .collect())
}

/// Parse a leading decimal integer like `Number.parseInt(text, 10)`.
pub fn parse_int_prefix(text: &str) -> Option<i64> {
    let text = text.trim_start_matches(|ch: char| ch.is_whitespace());
    let (sign, digits) = match text.strip_prefix(['+', '-']) {
        Some(rest) => (text.starts_with('-'), rest),
        None => (false, text),
    };
    let mut value: i64 = 0;
    let mut count = 0;
    for ch in digits.chars() {
        let Some(digit) = ch.to_digit(10) else {
            break;
        };
        value = value.checked_mul(10)?.checked_add(i64::from(digit))?;
        count += 1;
    }
    if count == 0 {
        return None;
    }
    Some(if sign { -value } else { value })
}

/// QuakeWorld challenge table (`QuakeWorldChallenges`).
pub struct QuakeWorldChallenges {
    records: HashMap<String, (u32, u64)>,
    random: Box<dyn FnMut() -> u32>,
    /// Table capacity.
    pub capacity: usize,
}

impl QuakeWorldChallenges {
    /// Create a table with a random source and capacity.
    pub fn new(random: Box<dyn FnMut() -> u32>, capacity: usize) -> Self {
        Self {
            records: HashMap::new(),
            random,
            capacity,
        }
    }

    /// Issue a challenge (`issue`).
    pub fn issue(&mut self, address: &NetworkAddress, now: f64) -> u32 {
        let key = address_key(address, false);
        if let Some((challenge, _)) = self.records.get(&key) {
            return *challenge;
        }
        if self.records.len() >= self.capacity {
            let mut oldest = (String::new(), u64::MAX);
            for (entry, (_, issued)) in &self.records {
                if *issued < oldest.1 {
                    oldest = (entry.clone(), *issued);
                }
            }
            self.records.remove(&oldest.0);
        }
        let challenge = ((self.random)() & 32767) << 16 ^ ((self.random)() & 32767);
        self.records.insert(key, (challenge, (now / 1000.0).trunc() as u64));
        challenge
    }

    /// Validate a challenge (`validate`).
    #[must_use]
    pub fn validate(&self, address: &NetworkAddress, challenge: u32) -> bool {
        self.records
            .get(&address_key(address, false))
            .is_some_and(|(stored, _)| *stored == challenge)
    }
}

/// QuakeWorld connect request (`QuakeWorldConnectRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuakeWorldConnectRequest {
    /// Sender.
    pub from: NetworkAddress,
    /// QPort.
    pub qport: u16,
    /// Challenge.
    pub challenge: u32,
    /// User info.
    pub userinfo: String,
    /// Spectator flag.
    pub spectator: bool,
    /// Donor wide-protocol flag.
    pub donor_wide: bool,
}

/// QuakeWorld connection verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuakeWorldConnectVerdict {
    /// Accepted.
    Accepted,
    /// Rejected.
    Rejected {
        /// Reason.
        reason: String,
    },
    /// Duplicate (silent).
    Duplicate,
}

/// QuakeWorld connection host (`QuakeWorldConnectionHost`).
pub trait QuakeWorldConnectionHost {
    /// Client password.
    fn password(&self) -> String;
    /// Spectator password.
    fn spectator_password(&self) -> String;
    /// Rcon password.
    fn rcon_password(&self) -> String;
    /// High characters allowed.
    fn high_characters(&self) -> bool;
    /// True when the address is blocked.
    fn blocked(&self, from: &NetworkAddress) -> bool;
    /// Connection verdict.
    fn connect(&mut self, request: &QuakeWorldConnectRequest, now: f64) -> QuakeWorldConnectVerdict;
    /// Status text.
    fn status(&self) -> String;
    /// Log text for a sequence.
    fn log(&self, sequence: i64) -> Option<String>;
    /// Execute an admin command, streaming output.
    fn execute_admin(&mut self, command: &str, write: &mut dyn FnMut(&str));
}

/// QuakeWorld connectionless server (`QuakeWorldConnectionlessServer`).
pub struct QuakeWorldConnectionlessServer<'a> {
    host: &'a mut dyn QuakeWorldConnectionHost,
    challenges: QuakeWorldChallenges,
}

impl<'a> QuakeWorldConnectionlessServer<'a> {
    /// Create a server over a host and challenge table.
    pub fn new(host: &'a mut dyn QuakeWorldConnectionHost, challenges: QuakeWorldChallenges) -> Self {
        Self { host, challenges }
    }

    /// Handle a connectionless datagram (`receive`).
    pub fn receive(&mut self, bytes: &[u8], from: &NetworkAddress, now: f64) -> Result<Vec<Vec<u8>>, Q1NetError> {
        if self.host.blocked(from) {
            return Ok(vec![quake_world_out_of_band("n\nbanned.\n", false)]);
        }
        let text = read_quake_world_out_of_band(bytes)?;
        let first = text.split('\n').next().unwrap_or("");
        let args = quake_world_command_arguments(first);
        let command = args.first().map(String::as_str).unwrap_or("");
        if command == "ping" || text == "k" {
            return Ok(vec![quake_world_out_of_band("l", false)]);
        }
        if command == "getchallenge" {
            let challenge = self.challenges.issue(from, now);
            return Ok(vec![quake_world_out_of_band(&format!("c{challenge}"), false)]);
        }
        if command == "status" {
            return Ok(vec![quake_world_out_of_band(
                &format!("n{}", self.host.status()),
                false,
            )]);
        }
        if command == "log" {
            let sequence = args.get(1).and_then(|arg| parse_int_prefix(arg)).unwrap_or(-1);
            let reply = self.host.log(sequence);
            return Ok(vec![quake_world_out_of_band(reply.as_deref().unwrap_or("m"), false)]);
        }
        if command == "rcon" {
            if self.host.rcon_password().is_empty()
                || args.get(1).map(String::as_str) != Some(self.host.rcon_password().as_str())
            {
                return Ok(vec![quake_world_out_of_band("nBad rcon_password.\n", false)]);
            }
            let mut replies = Vec::new();
            let mut output = String::new();
            let command_text: String = args.iter().skip(2).map(|arg| format!("{arg} ")).collect();
            let flush = |output: &mut String, replies: &mut Vec<Vec<u8>>| {
                if !output.is_empty() {
                    replies.push(quake_world_out_of_band(&format!("n{output}"), false));
                    output.clear();
                }
            };
            self.host.execute_admin(&command_text, &mut |part| {
                for ch in part.chars() {
                    output.push(ch);
                    if output.len() >= 7995 {
                        flush(&mut output, &mut replies);
                    }
                }
            });
            flush(&mut output, &mut replies);
            return Ok(replies);
        }
        if command != "connect" {
            return Ok(Vec::new());
        }
        if args.get(1).map(String::as_str) != Some("28") {
            return Ok(vec![quake_world_out_of_band(
                "n\nServer uses QuakeWorld protocol 28.\n",
                false,
            )]);
        }
        let qport = args.get(2).and_then(|arg| parse_int_prefix(arg)).unwrap_or(-1);
        let challenge = args.get(3).and_then(|arg| parse_int_prefix(arg)).unwrap_or(-1);
        if !(0..=65535).contains(&qport) || challenge < 0 || !self.challenges.validate(from, challenge as u32) {
            return Ok(vec![quake_world_out_of_band("n\nBad challenge.\n", false)]);
        }
        let raw = args.get(4).map(String::as_str).unwrap_or("");
        let truncated: String = raw.chars().take(1022).collect();
        let mut info = quake_world_info(&truncated)
            .into_iter()
            .collect::<BTreeMap<String, String>>();
        let spectator_key = info.get("spectator").cloned().unwrap_or_default();
        let spectator = !spectator_key.is_empty() && spectator_key != "0";
        let password = if spectator {
            self.host.spectator_password()
        } else {
            self.host.password()
        };
        let supplied = if spectator {
            spectator_key
        } else {
            info.get("password").cloned().unwrap_or_default()
        };
        if !password.is_empty() && password.to_lowercase() != "none" && password != supplied {
            return Ok(vec![quake_world_out_of_band(
                if spectator {
                    "n\nrequires a spectator password\n\n"
                } else {
                    "n\nserver requires a password\n\n"
                },
                false,
            )]);
        }
        info.remove(if spectator { "spectator" } else { "password" });
        if spectator {
            info.insert("*spectator".to_owned(), "1".to_owned());
        }
        let donor_wide = info.get("*wide").map(String::as_str) == Some("1");
        let mut userinfo = info_string(&info);
        if !self.host.high_characters() {
            userinfo = userinfo.chars().filter(|ch| *ch > '\x1f' && *ch <= '\x7f').collect();
        }
        userinfo = userinfo.chars().take(195).collect();
        let result = self.host.connect(
            &QuakeWorldConnectRequest {
                from: from.clone(),
                qport: qport as u16,
                challenge: challenge as u32,
                userinfo,
                spectator,
                donor_wide,
            },
            now,
        );
        Ok(match result {
            QuakeWorldConnectVerdict::Duplicate => Vec::new(),
            QuakeWorldConnectVerdict::Accepted => vec![quake_world_out_of_band("j", false)],
            QuakeWorldConnectVerdict::Rejected { reason } => {
                vec![quake_world_out_of_band(&format!("n{reason}"), false)]
            }
        })
    }
}

/// QuakeWorld connect state (`QuakeWorldConnectState`).
#[derive(Debug, Clone, PartialEq)]
pub enum QuakeWorldConnectState {
    /// Requesting a challenge.
    Challenge {
        /// Last send time.
        sent_at: f64,
    },
    /// Connecting with a challenge.
    Connect {
        /// Challenge.
        challenge: i64,
        /// Last send time.
        sent_at: f64,
    },
    /// Connected.
    Connected,
    /// Rejected.
    Rejected {
        /// Reason.
        reason: String,
    },
}

/// QuakeWorld connect client (`QuakeWorldConnectClient`).
#[derive(Debug, Clone)]
pub struct QuakeWorldConnectClient {
    /// QPort.
    pub qport: u16,
    /// User info.
    pub userinfo: String,
    /// State.
    pub state: QuakeWorldConnectState,
}

impl QuakeWorldConnectClient {
    /// Create a client.
    #[must_use]
    pub fn new(qport: u16, userinfo: &str) -> Self {
        Self {
            qport,
            userinfo: userinfo.to_owned(),
            state: QuakeWorldConnectState::Challenge {
                sent_at: f64::NEG_INFINITY,
            },
        }
    }

    /// Next handshake packet (`next`).
    pub fn next(&mut self, now: f64) -> Option<Vec<u8>> {
        match &self.state {
            QuakeWorldConnectState::Connected | QuakeWorldConnectState::Rejected { .. } => None,
            QuakeWorldConnectState::Challenge { sent_at } | QuakeWorldConnectState::Connect { sent_at, .. }
                if now - sent_at < 5000.0 =>
            {
                None
            }
            QuakeWorldConnectState::Challenge { .. } => {
                self.state = QuakeWorldConnectState::Challenge { sent_at: now };
                Some(quake_world_out_of_band("getchallenge\n", false))
            }
            QuakeWorldConnectState::Connect { challenge, .. } => {
                let challenge = *challenge;
                self.state = QuakeWorldConnectState::Connect {
                    challenge,
                    sent_at: now,
                };
                Some(quake_world_out_of_band(
                    &format!("connect 28 {} {challenge} \"{}\"\n", self.qport, self.userinfo),
                    false,
                ))
            }
        }
    }

    /// Handle a reply (`receive`).
    pub fn receive(&mut self, bytes: &[u8]) -> Result<(), Q1NetError> {
        let text = read_quake_world_out_of_band(bytes)?;
        if let Some(challenge) = text.strip_prefix('c') {
            if let Some(challenge) = parse_int_prefix(challenge) {
                self.state = QuakeWorldConnectState::Connect {
                    challenge,
                    sent_at: f64::NEG_INFINITY,
                };
            }
        } else if text == "j" {
            self.state = QuakeWorldConnectState::Connected;
        } else if let Some(reason) = text.strip_prefix('n') {
            self.state = QuakeWorldConnectState::Rejected {
                reason: reason.to_owned(),
            };
        }
        Ok(())
    }
}

/// QuakeWorld heartbeat (`quakeWorldHeartbeat`).
#[must_use]
pub fn quake_world_heartbeat(sequence: u32, active_clients: u32) -> Vec<u8> {
    quake_world_out_of_band(&format!("a\n{sequence}\n{active_clients}\n"), false)
}

/// QuakeWorld shutdown (`quakeWorldShutdown`).
#[must_use]
pub fn quake_world_shutdown() -> Vec<u8> {
    quake_world_out_of_band("C\n", false)
}

/// NetQuake connect state (`NetQuakeConnectState`).
#[derive(Debug, Clone, PartialEq)]
pub enum NetQuakeConnectState {
    /// Waiting for a reply.
    Waiting {
        /// Attempts made.
        attempts: u32,
        /// Last send time.
        sent_at: f64,
    },
    /// Connected.
    Connected {
        /// Server port.
        port: i32,
    },
    /// Rejected.
    Rejected {
        /// Reason.
        reason: String,
    },
}

/// NetQuake connect client (`NetQuakeConnectClient`).
#[derive(Debug, Clone)]
pub struct NetQuakeConnectClient {
    /// State.
    pub state: NetQuakeConnectState,
}

impl NetQuakeConnectClient {
    /// Create a client.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: NetQuakeConnectState::Waiting {
                attempts: 0,
                sent_at: f64::NEG_INFINITY,
            },
        }
    }

    /// Next handshake packet (`next`).
    pub fn next(&mut self, now: f64) -> Result<Option<Vec<u8>>, Q1NetError> {
        let NetQuakeConnectState::Waiting { attempts, sent_at } = &self.state else {
            return Ok(None);
        };
        let (attempts, sent_at) = (*attempts, *sent_at);
        if now - sent_at < 2500.0 {
            return Ok(None);
        }
        if attempts == 3 {
            self.state = NetQuakeConnectState::Rejected {
                reason: "No response".to_owned(),
            };
            return Ok(None);
        }
        self.state = NetQuakeConnectState::Waiting {
            attempts: attempts + 1,
            sent_at: now,
        };
        Ok(Some(encode_net_quake_control(&NetQuakeControl::ConnectRequest {
            game: "QUAKE".to_owned(),
            version: 3,
        })?))
    }

    /// Handle a reply (`receive`).
    pub fn receive(&mut self, bytes: &[u8]) -> Result<(), Q1NetError> {
        let message = decode_net_quake_control(bytes)?;
        if let NetQuakeControl::Reject { reason } = message {
            self.state = NetQuakeConnectState::Rejected { reason };
        } else if let NetQuakeControl::Accept { port } = message {
            if !(1..=65535).contains(&port) {
                return Err(Q1NetError::BadAcceptedPort);
            }
            self.state = NetQuakeConnectState::Connected { port };
        }
        Ok(())
    }
}

impl Default for NetQuakeConnectClient {
    fn default() -> Self {
        Self::new()
    }
}

/// QuakeWorld master heartbeat (`QuakeWorldMasterHeartbeat`).
#[derive(Debug, Clone)]
pub struct QuakeWorldMasterHeartbeat {
    previous: f64,
    sequence: u32,
}

impl QuakeWorldMasterHeartbeat {
    /// Create a heartbeat scheduler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            previous: f64::NEG_INFINITY,
            sequence: 0,
        }
    }

    /// Next heartbeat packet (`next`).
    pub fn next(&mut self, now: f64, active_clients: u32, force: bool) -> Option<Vec<u8>> {
        if !force && now - self.previous < 300000.0 {
            return None;
        }
        self.previous = now;
        self.sequence += 1;
        Some(quake_world_heartbeat(self.sequence, active_clients))
    }
}

impl Default for QuakeWorldMasterHeartbeat {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Signon (q1/session.ts)
// ---------------------------------------------------------------------------

/// Write a client string command (`writeClientStringCommand`).
pub fn write_client_string_command(writer: &mut MsgWriter, text: &str) -> Result<(), Q1NetError> {
    writer.write_byte(4)?;
    writer.write_string(text)?;
    Ok(())
}

fn write_stuff(writer: &mut MsgWriter, text: &str) -> Result<(), Q1NetError> {
    writer.write_byte(9)?;
    writer.write_string(text)?;
    Ok(())
}

/// NetQuake seat identity (`NetQuakeSeatIdentity`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetQuakeSeatIdentity {
    /// Player name.
    pub name: String,
    /// Shirt/pants color.
    pub color: u8,
    /// Spawn parameters.
    pub spawn_parameters: String,
    /// Extension flags, if any.
    pub extension_flags: Option<u32>,
}

/// NetQuake signon progression (`NetQuakeSignon`).
#[derive(Debug, Clone)]
pub struct NetQuakeSignon {
    /// Current stage.
    pub stage: u8,
    /// Seat identity.
    pub seat: NetQuakeSeatIdentity,
}

impl NetQuakeSignon {
    /// Create a signon for a seat.
    #[must_use]
    pub fn new(seat: NetQuakeSeatIdentity) -> Self {
        Self { stage: 0, seat }
    }

    /// True once the first entity arrived after stage 3.
    #[must_use]
    pub fn active(&self) -> bool {
        self.stage == 4
    }

    /// Receive a signon stage, returning the reply (`receive`).
    pub fn receive(&mut self, stage: u8) -> Result<Vec<u8>, Q1NetError> {
        if stage <= self.stage || stage > 4 {
            return Err(Q1NetError::BadSignonStage(stage, self.stage));
        }
        self.stage = stage;
        let mut writer = MsgWriter::new(8000, false);
        match stage {
            1 => write_client_string_command(&mut writer, "prespawn")?,
            2 => {
                write_client_string_command(&mut writer, &format!("name \"{}\"\n", self.seat.name))?;
                write_client_string_command(
                    &mut writer,
                    &format!("color {} {}\n", self.seat.color >> 4, self.seat.color & 15),
                )?;
                if let Some(flags) = self.seat.extension_flags {
                    write_client_string_command(&mut writer, &format!("ex_flags {flags}\n"))?;
                }
                write_client_string_command(&mut writer, &format!("spawn {}", self.seat.spawn_parameters))?;
            }
            3 => write_client_string_command(&mut writer, "begin")?,
            _ => {}
        }
        Ok(writer.bytes().to_vec())
    }

    /// Mark the first entity (`firstEntity`).
    pub fn first_entity(&mut self) {
        if self.stage == 3 {
            self.stage = 4;
        }
    }
}

/// QuakeWorld signon host (`QuakeWorldSignonHost`).
pub trait QuakeWorldSignonHost {
    /// Server data message.
    fn server_data(&self) -> QuakeWorldMessage;
    /// Model precache names.
    fn models(&self) -> Vec<String>;
    /// Sound precache names.
    fn sounds(&self) -> Vec<String>;
    /// Signon buffers.
    fn signon_buffers(&self) -> Vec<Vec<u8>>;
    /// True when the map checksum is accepted.
    fn accepts_map_checksum(&self, checksum: u32) -> bool;
    /// Spawn a client, returning engine messages.
    fn spawn(&mut self, start_client: u32) -> Vec<Vec<u8>>;
    /// Begin the match.
    fn begin(&mut self);
    /// Disconnect with a reason.
    fn disconnect(&mut self, reason: &str);
    /// Open a download source.
    fn open_download(&mut self, path: &str) -> Option<Box<dyn crate::services::downloads::DownloadSource>>;
}

/// Signon command result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignonCommand {
    /// Handled with reply messages.
    Handled {
        /// Reply messages.
        messages: Vec<Vec<u8>>,
    },
    /// Game command text.
    GameCommand {
        /// Command text.
        text: String,
    },
}

/// QuakeWorld signon server (`QuakeWorldSignonServer`).
pub struct QuakeWorldSignonServer<'a> {
    host: &'a mut dyn QuakeWorldSignonHost,
    donor_wide: bool,
    spawned: bool,
    download: Option<Box<dyn crate::services::downloads::DownloadSource>>,
    download_offset: u64,
}

impl<'a> QuakeWorldSignonServer<'a> {
    /// Create a server over a host.
    pub fn new(host: &'a mut dyn QuakeWorldSignonHost, donor_wide: bool) -> Self {
        Self {
            host,
            donor_wide,
            spawned: false,
            download: None,
            download_offset: 0,
        }
    }

    fn fresh(&mut self) -> Result<Vec<Vec<u8>>, Q1NetError> {
        let data = self.host.server_data();
        let version = match &data {
            QuakeWorldMessage::ServerData { protocol, .. } => match protocol {
                QwProfile::Quakeworld => 28,
                QwProfile::Wide { .. } => 29,
            },
            _ => return Err(Q1NetError::NoServerInfo),
        };
        if version == 29 && !self.donor_wide {
            self.host
                .disconnect("Client does not support donor QuakeWorld protocol 29");
            return Ok(Vec::new());
        }
        self.spawned = false;
        let mut writer = MsgWriter::new(1450, false);
        write_quake_world_server_data(&mut writer, &data)?;
        Ok(vec![writer.bytes().to_vec()])
    }

    /// Handle a string command (`command`).
    pub fn command(
        &mut self,
        text: &str,
        prepared_download: Option<Box<dyn crate::services::downloads::DownloadSource>>,
        prepared_present: bool,
    ) -> Result<SignonCommand, Q1NetError> {
        let args = quake_world_command_arguments(text);
        let op = args.first().map(String::as_str).unwrap_or("");
        if op == "new" {
            if self.spawned {
                return Ok(SignonCommand::Handled { messages: Vec::new() });
            }
            return Ok(SignonCommand::Handled {
                messages: self.fresh()?,
            });
        }
        if op == "download" {
            let previous = self.download.take();
            if !prepared_present {
                self.download = None;
            } else {
                self.download = prepared_download;
            }
            if let Some(mut source) = previous {
                source.close();
            }
            let path = args.get(1).map(String::as_str).unwrap_or("");
            if crate::services::downloads::download_path(path).is_ok() {
                if !prepared_present {
                    self.download = self.host.open_download(path);
                }
            } else {
                self.close();
            }
            self.download_offset = 0;
            let message = self.next_download()?;
            return Ok(SignonCommand::Handled {
                messages: vec![message],
            });
        }
        if op == "nextdl" {
            if self.download.is_none() {
                return Ok(SignonCommand::Handled { messages: Vec::new() });
            }
            let message = self.next_download()?;
            return Ok(SignonCommand::Handled {
                messages: vec![message],
            });
        }
        if op != "soundlist" && op != "modellist" && op != "prespawn" && op != "spawn" && op != "begin" {
            return Ok(SignonCommand::GameCommand { text: text.to_owned() });
        }
        let data = self.host.server_data();
        if self.spawned {
            return Ok(SignonCommand::Handled { messages: Vec::new() });
        }
        let (server_count, protocol) = match &data {
            QuakeWorldMessage::ServerData {
                server_count, protocol, ..
            } => (*server_count, *protocol),
            _ => return Err(Q1NetError::NoServerInfo),
        };
        if args.get(1).and_then(|arg| parse_int_prefix(arg)) != Some(server_count) {
            return Ok(SignonCommand::Handled {
                messages: self.fresh()?,
            });
        }
        let mut writer = MsgWriter::new(1450, false);
        let max_precache = match protocol {
            QwProfile::Quakeworld => 512,
            QwProfile::Wide { .. } => crate::q1_wide::QW29_MAX_PRECACHE,
        };
        let start = args.get(2).and_then(|arg| parse_int_prefix(arg)).unwrap_or(0);
        if op == "soundlist" || op == "modellist" {
            let names = if op == "soundlist" {
                self.host.sounds()
            } else {
                self.host.models()
            };
            if start < 0 || start >= max_precache as i64 || start > names.len() as i64 {
                return Ok(SignonCommand::Handled {
                    messages: self.fresh()?,
                });
            }
            writer.write_byte(if op == "soundlist" { 46 } else { 45 })?;
            match protocol {
                QwProfile::Quakeworld => writer.write_byte(start as u8)?,
                QwProfile::Wide { .. } => writer.write_short(start as i16)?,
            }
            let mut next = start as usize;
            while next < names.len() && writer.cursize() < 725 {
                if next + 1 >= max_precache {
                    return Err(Q1NetError::PrecacheUnrepresentable);
                }
                writer.write_string(&names[next])?;
                next += 1;
            }
            writer.write_byte(0)?;
            let tail = if next < names.len() { next as i64 } else { 0 };
            match protocol {
                QwProfile::Quakeworld => writer.write_byte(tail as u8)?,
                QwProfile::Wide { .. } => writer.write_short(tail as i16)?,
            }
            return Ok(SignonCommand::Handled {
                messages: vec![writer.bytes().to_vec()],
            });
        }
        if op == "prespawn" {
            let buffers = self.host.signon_buffers();
            let index = if start >= 0 && (start as usize) < buffers.len() {
                start as usize
            } else {
                0
            };
            if index == 0 {
                let checksum = args.get(3).and_then(|arg| parse_int_prefix(arg)).unwrap_or(0) as u32;
                if !self.host.accepts_map_checksum(checksum) {
                    self.host.disconnect("Map model file does not match");
                    return Ok(SignonCommand::Handled { messages: Vec::new() });
                }
            }
            if let Some(bytes) = buffers.get(index) {
                writer.write_bytes(bytes)?;
            }
            let follow = if index + 1 >= buffers.len() {
                format!("cmd spawn {server_count} 0\n")
            } else {
                format!("cmd prespawn {server_count} {}\n", index + 1)
            };
            write_stuff(&mut writer, &follow)?;
            return Ok(SignonCommand::Handled {
                messages: vec![writer.bytes().to_vec()],
            });
        }
        if op == "spawn" {
            if !(0..=32).contains(&start) {
                return Ok(SignonCommand::Handled {
                    messages: self.fresh()?,
                });
            }
            let mut messages = self.host.spawn(start as u32);
            write_stuff(&mut writer, "skins\n")?;
            messages.push(writer.bytes().to_vec());
            return Ok(SignonCommand::Handled { messages });
        }
        self.host.begin();
        self.spawned = true;
        Ok(SignonCommand::Handled { messages: Vec::new() })
    }

    fn next_download(&mut self) -> Result<Vec<u8>, Q1NetError> {
        let mut writer = MsgWriter::new(1450, false);
        match self.download.as_mut() {
            None => write_quake_world_download(&mut writer, &QwDownload::Missing)?,
            Some(source) => {
                let bytes = match source.read(self.download_offset, 768) {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        self.close();
                        return Err(Q1NetError::Overflow);
                    }
                };
                self.download_offset += bytes.len() as u64;
                let length = source.byte_length();
                let percent = (self.download_offset * 100 / length.max(1)) as u8;
                write_quake_world_download(
                    &mut writer,
                    &QwDownload::Data {
                        percent,
                        bytes: bytes.clone(),
                    },
                )?;
                if self.download_offset == length {
                    if let Some(mut source) = self.download.take() {
                        source.close();
                    }
                }
            }
        }
        Ok(writer.bytes().to_vec())
    }

    /// Close the download source (`close`).
    pub fn close(&mut self) {
        if let Some(mut source) = self.download.take() {
            source.close();
        }
    }
}

/// QuakeWorld precache client (`QuakeWorldPrecacheClient`).
#[derive(Debug, Default)]
pub struct QuakeWorldPrecacheClient {
    /// Server count.
    pub server_count: i64,
    /// Model names.
    pub models: Vec<String>,
    /// Sound names.
    pub sounds: Vec<String>,
}

impl QuakeWorldPrecacheClient {
    /// Receive a message, returning follow-up commands (`receive`).
    pub fn receive(&mut self, message: &QuakeWorldMessage) -> Result<Vec<String>, Q1NetError> {
        match message {
            QuakeWorldMessage::ServerData { server_count, .. } => {
                self.server_count = *server_count;
                self.models.clear();
                self.sounds.clear();
                Ok(vec![format!("soundlist {} 0", self.server_count)])
            }
            QuakeWorldMessage::ModelList { first, names, next }
            | QuakeWorldMessage::SoundList { first, names, next } => {
                let sounds = matches!(message, QuakeWorldMessage::SoundList { .. });
                let target = if sounds { &mut self.sounds } else { &mut self.models };
                if *first as usize > target.len() {
                    return Err(Q1NetError::PrecacheSkipped);
                }
                target.truncate(*first as usize);
                target.extend(names.iter().cloned());
                if *next != 0 {
                    return Ok(vec![format!(
                        "{} {} {next}",
                        if sounds { "soundlist" } else { "modellist" },
                        self.server_count
                    )]);
                }
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    /// Model list command once sounds are ready (`soundsReady`).
    #[must_use]
    pub fn sounds_ready(&self) -> String {
        format!("modellist {} 0", self.server_count)
    }

    /// Prespawn command once models are ready (`modelsReady`).
    #[must_use]
    pub fn models_ready(&self, checksum: u32) -> String {
        format!("prespawn {} 0 {checksum}", self.server_count)
    }

    /// Begin command once skins are ready (`skinsReady`).
    #[must_use]
    pub fn skins_ready(&self) -> String {
        format!("begin {}", self.server_count)
    }
}

/// Choose a QuakeWorld profile (`chooseQuakeWorldProtocol`).
#[must_use]
pub fn choose_quake_world_protocol(mode: QwConnectMode, needs_wide: bool) -> QwProfile {
    if mode == QwConnectMode::Wide || (mode == QwConnectMode::Auto && needs_wide) {
        QwProfile::Wide { flags: 130 }
    } else {
        QwProfile::Quakeworld
    }
}

/// QuakeWorld connect mode: 28, 29, or auto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QwConnectMode {
    /// Protocol 28.
    Classic,
    /// Protocol 29.
    Wide,
    /// Automatic.
    Auto,
}

// ---------------------------------------------------------------------------
// Checksums (q1/checksum.ts)
// ---------------------------------------------------------------------------

/// QuakeWorld sequence checksum (`quakeWorldChecksum`).
pub fn quake_world_checksum(bytes: &[u8], sequence: u32) -> Result<u8, Q1NetError> {
    let count = bytes.len().min(60);
    let offset = (sequence as usize) % (QW_CHKTBL.len() - 8);
    let mut data = vec![0u8; count + 4];
    data[..count].copy_from_slice(&bytes[..count]);
    data[count] = (sequence as u8) ^ QW_CHKTBL[offset];
    data[count + 1] = QW_CHKTBL[offset + 1];
    data[count + 2] = ((sequence >> 8) as u8) ^ QW_CHKTBL[offset + 2];
    data[count + 3] = QW_CHKTBL[offset + 3];
    let mut crc: u32 = 65535;
    for value in &data {
        crc ^= u32::from(*value) << 8;
        for _ in 0..8 {
            crc = ((crc << 1) ^ if crc & 32768 != 0 { 0x1021 } else { 0 }) & 65535;
        }
    }
    Ok((crc & 255) as u8)
}

/// QuakeWorld BSP checksum (`quakeWorldMapChecksum2`).
pub fn quake_world_map_checksum2(bytes: &[u8]) -> Result<u32, Q1NetError> {
    if bytes.len() < 124 {
        return Err(Q1NetError::ShortBsp);
    }
    if i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) != 29 {
        return Err(Q1NetError::BadBspVersion);
    }
    let mut checksum = 0;
    for lump in 0..15 {
        let start = i32::from_le_bytes([
            bytes[4 + lump * 8],
            bytes[5 + lump * 8],
            bytes[6 + lump * 8],
            bytes[7 + lump * 8],
        ]);
        let length = i32::from_le_bytes([
            bytes[8 + lump * 8],
            bytes[9 + lump * 8],
            bytes[10 + lump * 8],
            bytes[11 + lump * 8],
        ]);
        if start < 0 || length < 0 || start as usize + length as usize > bytes.len() {
            return Err(Q1NetError::BadBspLump);
        }
        if lump == 0 || lump == 4 || lump == 5 || lump == 10 {
            continue;
        }
        checksum ^= md4_block_checksum(&bytes[start as usize..start as usize + length as usize]);
    }
    Ok(checksum)
}

// ---------------------------------------------------------------------------
// Discovery (q1/discovery.ts)
// ---------------------------------------------------------------------------

/// NetQuake discovery wire (`netQuakeDiscoveryWire`).
#[derive(Debug, Default)]
pub struct NetQuakeDiscoveryWire;

impl crate::services::discovery::DiscoveryWire for NetQuakeDiscoveryWire {
    fn query(&self, _kind: DiscoveryRequestKind, _challenge: &str) -> Result<Vec<u8>, DiscoveryError> {
        encode_net_quake_control(&NetQuakeControl::ServerInfoRequest {
            game: "QUAKE".to_owned(),
            version: 3,
        })
        .map_err(|error| DiscoveryError::Wire(error.to_string()))
    }

    fn master_query(&self) -> Result<Vec<u8>, DiscoveryError> {
        Err(DiscoveryError::Wire(
            "NetQuake master discovery is not configured".to_owned(),
        ))
    }

    fn heartbeat(&self, _active: bool) -> Result<Vec<u8>, DiscoveryError> {
        Err(DiscoveryError::Wire(
            "NetQuake master registration is not configured".to_owned(),
        ))
    }
}

/// Read a NetQuake discovery response (`readNetQuakeDiscovery`).
pub fn read_net_quake_discovery(bytes: &[u8]) -> Result<ServerStatus, Q1NetError> {
    let message = decode_net_quake_control(bytes)?;
    let NetQuakeControl::ServerInfo {
        name,
        map,
        players,
        max_players,
        version,
        ..
    } = message
    else {
        return Err(Q1NetError::NotDiscoveryResponse);
    };
    if version != 3 {
        return Err(Q1NetError::NotDiscoveryResponse);
    }
    Ok(ServerStatus {
        name,
        map,
        players: i64::from(players),
        max_players: i64::from(max_players),
        rules: BTreeMap::new(),
        player_details: Vec::new(),
        wire: crate::common::session::WireSelection::Source {
            protocol: crate::protocol::ProtocolIdentity::Q1Netquake,
        },
    })
}

/// QuakeWorld discovery wire (`quakeWorldDiscoveryWire`).
#[derive(Debug, Default)]
pub struct QuakeWorldDiscoveryWire;

impl crate::services::discovery::DiscoveryWire for QuakeWorldDiscoveryWire {
    fn query(&self, _kind: DiscoveryRequestKind, _challenge: &str) -> Result<Vec<u8>, DiscoveryError> {
        Ok(quake_world_out_of_band("status\n", false))
    }

    fn master_query(&self) -> Result<Vec<u8>, DiscoveryError> {
        Err(DiscoveryError::Wire("Use an HTTP QuakeWorld master list".to_owned()))
    }

    fn heartbeat(&self, _active: bool) -> Result<Vec<u8>, DiscoveryError> {
        Err(DiscoveryError::Wire(
            "QuakeWorld heartbeat requires its source sequence and active client count".to_owned(),
        ))
    }
}

fn parse_status_player(line: &str) -> Option<(i64, i64, String)> {
    // `userid frags minutes ping "name" "skin" top bottom`, trailing spaces allowed.
    let mut cursor = line;
    let mut numbers = Vec::new();
    for _ in 0..4 {
        cursor = cursor.trim_start_matches(' ');
        let end = cursor.find(' ').unwrap_or(cursor.len());
        if end == 0 {
            return None;
        }
        numbers.push(cursor[..end].parse::<i64>().ok()?);
        cursor = &cursor[end..];
    }
    cursor = cursor.trim_start_matches(' ');
    let name = quoted(cursor)?;
    cursor = cursor.trim_start_matches(' ');
    let _skin = quoted(cursor.strip_prefix(&format!("\"{name}\""))?.trim_start_matches(' '))?;
    let rest = cursor.strip_prefix(&format!("\"{name}\""))?.trim_start_matches(' ');
    let rest = rest.strip_prefix(&format!("\"{_skin}\""))?.trim_start_matches(' ');
    let mut tail = rest.split_whitespace();
    tail.next()?.parse::<i64>().ok()?;
    tail.next()?.parse::<i64>().ok()?;
    if tail.next().is_some() {
        return None;
    }
    Some((numbers[1], numbers[3], name))
}

fn quoted(text: &str) -> Option<String> {
    let rest = text.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

/// Read a QuakeWorld discovery response (`readQuakeWorldDiscovery`).
pub fn read_quake_world_discovery(bytes: &[u8]) -> Result<ServerStatus, Q1NetError> {
    let text = read_quake_world_out_of_band(bytes)?;
    if !text.starts_with("n\\") {
        return Err(Q1NetError::NotStatusReply);
    }
    let body = text[1..].trim_end_matches('\0');
    let mut lines = body.split('\n');
    let info = lines.next().unwrap_or("");
    let fields: Vec<&str> = info[1..].split('\\').collect();
    let mut rules = BTreeMap::new();
    let mut index = 0;
    while index + 1 < fields.len() {
        rules.insert(fields[index].to_owned(), fields[index + 1].to_owned());
        index += 2;
    }
    let mut player_details = Vec::new();
    for line in lines {
        if let Some((score, ping, name)) = parse_status_player(line) {
            player_details.push(crate::services::discovery::PlayerDetail { name, score, ping });
        }
    }
    let max_players = rules
        .get("maxclients")
        .map(|value| value.parse::<f64>().unwrap_or(32.0))
        .unwrap_or(32.0) as i64;
    Ok(ServerStatus {
        name: rules.get("hostname").cloned().unwrap_or_default(),
        map: rules.get("map").cloned().unwrap_or_default(),
        players: player_details.len() as i64,
        max_players,
        rules,
        player_details,
        wire: crate::common::session::WireSelection::Source {
            protocol: crate::protocol::ProtocolIdentity::Q1Quakeworld,
        },
    })
}

// ---------------------------------------------------------------------------
// Prediction (q1/prediction.ts)
// ---------------------------------------------------------------------------

/// QuakeWorld prediction frame (`QuakeWorldPredictionFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeWorldPredictionFrame {
    /// Command sequence.
    pub sequence: u32,
    /// Send time in seconds.
    pub sent_at_seconds: f64,
    /// Command.
    pub command: QwUsercmd,
}

/// Prediction state for host movement steps.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QwPredictionState {
    /// Origin.
    pub origin: [f64; 3],
    /// Velocity.
    pub velocity: [f64; 3],
}

/// Host movement step for prediction (`moveQuakeWorld`).
pub trait QwPredictionStep {
    /// Run one command; `None` reports actor removal.
    fn step(&mut self, state: &QwPredictionState, command: &QwUsercmd, sequence: u32) -> Option<QwPredictionState>;
}

/// QuakeWorld prediction result (`QuakeWorldPrediction`).
#[derive(Debug, Clone, PartialEq)]
pub enum QuakeWorldPrediction {
    /// Prediction unavailable.
    Unavailable {
        /// Reason.
        reason: QwPredictionReason,
    },
    /// Predicted state.
    Predicted {
        /// Origin.
        origin: [f64; 3],
        /// Velocity.
        velocity: [f64; 3],
        /// Sequence.
        sequence: u32,
        /// State.
        state: QwPredictionState,
    },
}

/// Prediction unavailability reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QwPredictionReason {
    /// History overrun.
    HistoryOverrun,
    /// Missing command.
    MissingCommand,
    /// Actor removed.
    ActorRemoved,
    /// No pending command.
    NoPendingCommand,
}

/// QuakeWorld prediction history (`QuakeWorldPredictionHistory`).
#[derive(Debug, Default)]
pub struct QuakeWorldPredictionHistory {
    frames: HashMap<u32, QuakeWorldPredictionFrame>,
    /// Smoothed latency in seconds.
    pub latency_seconds: f64,
}

impl QuakeWorldPredictionHistory {
    /// Record a frame (`record`).
    pub fn record(&mut self, frame: QuakeWorldPredictionFrame) {
        let sequence = frame.sequence;
        self.frames.insert(sequence, frame);
        let stale: Vec<u32> = self
            .frames
            .keys()
            .copied()
            .filter(|candidate| *candidate + 64 <= sequence)
            .collect();
        for candidate in stale {
            self.frames.remove(&candidate);
        }
    }

    /// Record an acknowledgment (`acknowledged`).
    pub fn acknowledged(&mut self, sequence: u32, received_at: f64) {
        let Some(frame) = self.frames.get(&sequence) else {
            return;
        };
        let latency = received_at - frame.sent_at_seconds;
        if latency < 0.0 || latency > 1.0 {
            return;
        }
        self.latency_seconds = if latency < self.latency_seconds {
            latency
        } else {
            self.latency_seconds + 0.001
        };
    }

    /// Command bundle for a move (`bundle`).
    pub fn bundle(&self, sequence: u32, loss_percent: u8) -> Result<QuakeWorldMove, Q1NetError> {
        let (Some(current), Some(previous), Some(oldest)) = (
            self.frames.get(&sequence),
            self.frames.get(&sequence.wrapping_sub(1)),
            self.frames.get(&sequence.wrapping_sub(2)),
        ) else {
            return Err(Q1NetError::MissingCommands);
        };
        Ok(QuakeWorldMove {
            oldest: oldest.command.clone(),
            previous: previous.command.clone(),
            current: current.command.clone(),
            loss_percent,
        })
    }

    /// Predict presentation state (`predict`).
    pub fn predict(
        &self,
        initial: &QwPredictionState,
        acknowledged: u32,
        outgoing: u32,
        realtime: f64,
        push_latency_ms: f64,
        step: &mut dyn QwPredictionStep,
    ) -> QuakeWorldPrediction {
        if outgoing.wrapping_sub(acknowledged) >= 63 {
            return QuakeWorldPrediction::Unavailable {
                reason: QwPredictionReason::HistoryOverrun,
            };
        }
        let target = realtime.min(realtime - self.latency_seconds - push_latency_ms.min(0.0) / 1000.0);
        let mut previous_time = self
            .frames
            .get(&acknowledged)
            .map(|frame| frame.sent_at_seconds)
            .unwrap_or(target);
        let mut previous_origin = initial.origin;
        let mut previous_velocity = initial.velocity;
        let mut current = initial.clone();
        let mut last: Option<QwPredictionState> = None;
        let mut last_sequence = acknowledged;
        let mut sequence = acknowledged.wrapping_add(1);
        while sequence < outgoing {
            let Some(frame) = self.frames.get(&sequence) else {
                return QuakeWorldPrediction::Unavailable {
                    reason: QwPredictionReason::MissingCommand,
                };
            };
            let mut commands = Vec::new();
            split_quake_world_command(&frame.command, &mut |command| commands.push(command));
            for command in &commands {
                match step.step(&current, command, sequence) {
                    None => {
                        return QuakeWorldPrediction::Unavailable {
                            reason: QwPredictionReason::ActorRemoved,
                        }
                    }
                    Some(state) => {
                        last = Some(state.clone());
                        current = state;
                    }
                }
            }
            last_sequence = sequence;
            if frame.sent_at_seconds >= target {
                let Some(result) = last.clone() else {
                    return QuakeWorldPrediction::Unavailable {
                        reason: QwPredictionReason::NoPendingCommand,
                    };
                };
                let factor = if frame.sent_at_seconds == previous_time {
                    0.0
                } else {
                    ((target - previous_time) / (frame.sent_at_seconds - previous_time)).clamp(0.0, 1.0)
                };
                let teleport = (previous_origin[0] - result.origin[0]).abs() > 128.0
                    || (previous_origin[1] - result.origin[1]).abs() > 128.0
                    || (previous_origin[2] - result.origin[2]).abs() > 128.0;
                return QuakeWorldPrediction::Predicted {
                    origin: if teleport {
                        result.origin
                    } else {
                        lerp_f32(previous_origin, result.origin, factor)
                    },
                    velocity: if teleport {
                        result.velocity
                    } else {
                        lerp_f32(previous_velocity, result.velocity, factor)
                    },
                    sequence,
                    state: result,
                };
            }
            previous_time = frame.sent_at_seconds;
            previous_origin = current.origin;
            previous_velocity = current.velocity;
            sequence = sequence.wrapping_add(1);
        }
        match last {
            None => QuakeWorldPrediction::Unavailable {
                reason: QwPredictionReason::NoPendingCommand,
            },
            Some(state) => QuakeWorldPrediction::Predicted {
                origin: current.origin,
                velocity: current.velocity,
                sequence: last_sequence,
                state,
            },
        }
    }
}

fn lerp_f32(a: [f64; 3], b: [f64; 3], factor: f64) -> [f64; 3] {
    [
        f64::from((a[0] + factor * (b[0] - a[0])) as f32),
        f64::from((a[1] + factor * (b[1] - a[1])) as f32),
        f64::from((a[2] + factor * (b[2] - a[2])) as f32),
    ]
}

// ---------------------------------------------------------------------------
// QuakeWorld commands (q1/commands.ts)
// ---------------------------------------------------------------------------

/// QuakeWorld movement bundle (`QuakeWorldMove`).
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeWorldMove {
    /// Oldest command.
    pub oldest: QwUsercmd,
    /// Previous command.
    pub previous: QwUsercmd,
    /// Current command.
    pub current: QwUsercmd,
    /// Loss percent.
    pub loss_percent: u8,
}

/// QuakeWorld client message (`QuakeWorldClientMessage`).
#[derive(Debug, Clone, PartialEq)]
pub enum QuakeWorldClientMessage {
    /// No-op.
    Nop,
    /// String command.
    StringCommand {
        /// Command text.
        text: String,
    },
    /// Delta request.
    Delta {
        /// Sequence.
        sequence: u8,
    },
    /// Spectator teleport.
    SpectatorTeleport {
        /// Origin.
        origin: [f64; 3],
    },
    /// Upload chunk.
    Upload {
        /// Percent.
        percent: u8,
        /// Bytes.
        bytes: Vec<u8>,
    },
    /// Movement bundle.
    Move {
        /// Bundle.
        bundle: QuakeWorldMove,
    },
}

/// Write a movement bundle (`writeQuakeWorldMove`).
pub fn write_quake_world_move(
    writer: &mut MsgWriter,
    bundle: &QuakeWorldMove,
    sequence: u32,
) -> Result<(), Q1NetError> {
    let mut body = MsgWriter::new(writer.maxsize(), false);
    body.write_byte(bundle.loss_percent)?;
    let empty = QwUsercmd::default();
    write_delta_usercmd(&mut body, &empty, &bundle.oldest)?;
    write_delta_usercmd(&mut body, &bundle.oldest, &bundle.previous)?;
    write_delta_usercmd(&mut body, &bundle.previous, &bundle.current)?;
    let checksum = quake_world_checksum(body.bytes(), sequence)?;
    writer.write_byte(3)?;
    writer.write_byte(checksum)?;
    writer.write_bytes(body.bytes())?;
    Ok(())
}

/// Decode client messages (`decodeQuakeWorldClient`).
pub fn decode_quake_world_client(
    bytes: &[u8],
    profile: QwProfile,
    sequence: u32,
) -> Result<Vec<QuakeWorldClientMessage>, Q1NetError> {
    let flags = crate::q1_wide::qw_protocol_flags(profile);
    let mut reader = MsgReader::new(bytes);
    let mut out = Vec::new();
    let mut moved = false;
    while reader.remaining() > 0 {
        match reader.byte()? {
            1 => out.push(QuakeWorldClientMessage::Nop),
            4 => out.push(QuakeWorldClientMessage::StringCommand {
                text: reader.string(512),
            }),
            5 => out.push(QuakeWorldClientMessage::Delta {
                sequence: reader.byte()?,
            }),
            6 => {
                let origin = if matches!(profile, QwProfile::Quakeworld) {
                    [reader.float()? as f64, reader.float()? as f64, reader.float()? as f64]
                } else {
                    [
                        reader.coord_flags(flags)?,
                        reader.coord_flags(flags)?,
                        reader.coord_flags(flags)?,
                    ]
                };
                out.push(QuakeWorldClientMessage::SpectatorTeleport { origin });
            }
            7 => {
                let size = reader.short()?;
                let percent = reader.byte()?;
                if size < 0 {
                    return Err(Q1NetError::Overflow);
                }
                out.push(QuakeWorldClientMessage::Upload {
                    percent,
                    bytes: reader.bytes(size as usize)?.to_vec(),
                });
            }
            3 => {
                if moved {
                    return Err(Q1NetError::MultipleMoves);
                }
                moved = true;
                let check = reader.byte()?;
                let start = reader.offset();
                let loss_percent = reader.byte()?;
                let empty = QwUsercmd::default();
                let oldest = read_delta_usercmd(&mut reader, &empty)?;
                let previous = read_delta_usercmd(&mut reader, &oldest)?;
                let current = read_delta_usercmd(&mut reader, &previous)?;
                reader.finish()?;
                if quake_world_checksum(&bytes[start..reader.offset()], sequence)? != check {
                    return Err(Q1NetError::BadMoveChecksum);
                }
                out.push(QuakeWorldClientMessage::Move {
                    bundle: QuakeWorldMove {
                        oldest,
                        previous,
                        current,
                        loss_percent,
                    },
                });
            }
            _ => return Err(Q1NetError::UnknownClientMessage),
        }
        reader.finish()?;
    }
    Ok(out)
}

/// Write an upload chunk (`writeQuakeWorldUpload`).
pub fn write_quake_world_upload(writer: &mut MsgWriter, bytes: &[u8], percent: u8) -> Result<(), Q1NetError> {
    if bytes.len() > 768 {
        return Err(Q1NetError::UploadTooLarge);
    }
    writer.write_byte(7)?;
    writer.write_short(bytes.len() as i16)?;
    writer.write_byte(percent)?;
    writer.write_bytes(bytes)?;
    Ok(())
}

/// Server loss-recovery replay (`QuakeWorldCommandReplay`).
#[derive(Debug, Default)]
pub struct QuakeWorldCommandReplay {
    last: QwUsercmd,
}

impl QuakeWorldCommandReplay {
    /// Replay dropped commands, then the current one (`run`).
    pub fn run(&mut self, bundle: &QuakeWorldMove, dropped: u32, paused: bool, mut apply: impl FnMut(&QwUsercmd)) {
        if !paused {
            if dropped < 20 {
                let mut pending = dropped;
                while pending > 2 {
                    apply(&self.last);
                    pending -= 1;
                }
                if dropped > 1 {
                    apply(&bundle.oldest);
                }
                if dropped > 0 {
                    apply(&bundle.previous);
                }
            }
            apply(&bundle.current);
        }
        self.last = bundle.current.clone();
        self.last.buttons = 0;
    }
}

/// Split commands over 50 ms (`splitQuakeWorldCommand`).
pub fn split_quake_world_command(command: &QwUsercmd, apply: &mut dyn FnMut(QwUsercmd)) {
    if command.msec > 50 {
        let half = QwUsercmd {
            msec: command.msec / 2,
            ..command.clone()
        };
        split_quake_world_command(&half, apply);
        split_quake_world_command(&half, apply);
    } else {
        apply(command.clone());
    }
}

// ---------------------------------------------------------------------------
// Shared wire entities
// ---------------------------------------------------------------------------

/// Extended wire entity (`Q1ExtendedEntityState`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q1WireEntity {
    /// Entity number.
    pub number: u32,
    /// Visible state.
    pub state: WideEntityState,
    /// Lerp finish in seconds.
    pub lerp_finish_seconds: f64,
    /// Step animation.
    pub step: bool,
    /// QuakeWorld solid flags.
    pub quakeworld_flags: u32,
}

/// Temporary entity effect (`TemporaryEntity`).
#[derive(Debug, Clone, PartialEq)]
pub enum TemporaryEntity {
    /// Point effect.
    Point {
        /// Effect type.
        effect_type: u8,
        /// Origin.
        origin: [f64; 3],
        /// Count (QW types 2/12 carry a count byte).
        count: u8,
    },
    /// Beam effect.
    Beam {
        /// Effect type.
        effect_type: u8,
        /// Entity.
        entity: u16,
        /// Start.
        start: [f64; 3],
        /// End.
        end: [f64; 3],
    },
    /// NetQuake color explosion.
    ExplosionColors {
        /// Origin.
        origin: [f64; 3],
        /// Color start.
        color_start: u8,
        /// Color length.
        color_length: u8,
    },
}

/// Read a temporary entity (`readTemporaryEntity`).
pub fn read_temporary_entity(
    reader: &mut MsgReader<'_>,
    coord: &mut dyn FnMut(&mut MsgReader<'_>) -> Result<f64, Q1NetError>,
    quakeworld: bool,
) -> Result<TemporaryEntity, Q1NetError> {
    let effect_type = reader.byte()?;
    if effect_type == 5 || effect_type == 6 || effect_type == 9 || (!quakeworld && effect_type == 13) {
        let entity = reader.short()? as u16;
        let start = [coord(reader)?, coord(reader)?, coord(reader)?];
        let end = [coord(reader)?, coord(reader)?, coord(reader)?];
        return Ok(TemporaryEntity::Beam {
            effect_type,
            entity,
            start,
            end,
        });
    }
    if effect_type > 13 {
        return Err(Q1NetError::UnknownTempEntity(effect_type));
    }
    let count = if quakeworld && (effect_type == 2 || effect_type == 12) {
        reader.byte()?
    } else {
        1
    };
    let origin = [coord(reader)?, coord(reader)?, coord(reader)?];
    if !quakeworld && effect_type == 12 {
        return Ok(TemporaryEntity::ExplosionColors {
            origin,
            color_start: reader.byte()?,
            color_length: reader.byte()?,
        });
    }
    Ok(TemporaryEntity::Point {
        effect_type,
        origin,
        count,
    })
}

// ---------------------------------------------------------------------------
// NetQuake messages (q1/netquake.ts)
// ---------------------------------------------------------------------------

/// Rerelease message policy (`RereleaseMessages`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RereleaseMessages {
    /// Only verified retail services.
    #[default]
    KnownRetail,
    /// Rerelease private services.
    Quake1ReTsPrivate,
}

/// NetQuake client data (`Q1ClientData` as decoded).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NetQuakeClientData {
    /// View height.
    pub view_height: i8,
    /// Ideal pitch.
    pub ideal_pitch: i8,
    /// Punch angles.
    pub punch_angles: [i8; 3],
    /// Velocity.
    pub velocity: [i16; 3],
    /// Item bits.
    pub items: i32,
    /// On ground.
    pub on_ground: bool,
    /// In water.
    pub in_water: bool,
    /// Weapon frame.
    pub weapon_frame: u16,
    /// Armor.
    pub armor: u16,
    /// Weapon model.
    pub weapon_model: u16,
    /// Health.
    pub health: i16,
    /// Ammo.
    pub ammo: u16,
    /// Shells.
    pub shells: u16,
    /// Nails.
    pub nails: u16,
    /// Rockets.
    pub rockets: u16,
    /// Cells.
    pub cells: u16,
    /// Active weapon.
    pub active_weapon: u32,
}

/// NetQuake message (`NetQuakeMessage`).
#[derive(Debug, Clone, PartialEq)]
pub enum NetQuakeMessage {
    /// Unit messages.
    Unit(NqUnit),
    /// Text messages.
    Text {
        /// Kind.
        kind: NqText,
        /// Text.
        text: String,
    },
    /// Time.
    Time {
        /// Seconds.
        seconds: f32,
    },
    /// Version.
    Version {
        /// Version.
        version: i32,
    },
    /// Stat.
    Stat {
        /// Index.
        index: u8,
        /// Value.
        value: i32,
    },
    /// Set view.
    SetView {
        /// Entity.
        entity: u16,
    },
    /// Set angle.
    SetAngle {
        /// Angles.
        angles: [f64; 3],
    },
    /// Server info.
    ServerInfo {
        /// Protocol.
        protocol: NqProfile,
        /// Maximum clients.
        max_clients: u8,
        /// Game type.
        game_type: u8,
        /// Level.
        level: String,
        /// Models.
        models: Vec<String>,
        /// Sounds.
        sounds: Vec<String>,
    },
    /// Light style.
    LightStyle {
        /// Index.
        index: u8,
        /// Value.
        value: String,
    },
    /// Named slot.
    NamedSlot {
        /// Kind.
        kind: NqNamedSlot,
        /// Slot.
        slot: u8,
        /// Value.
        value: String,
    },
    /// Numbered slot.
    NumberedSlot {
        /// Kind.
        kind: NqNumberedSlot,
        /// Slot.
        slot: u8,
        /// Value.
        value: i16,
    },
    /// Client data.
    ClientData {
        /// Data.
        data: NetQuakeClientData,
        /// Weapon alpha.
        weapon_alpha: u8,
    },
    /// Entity update.
    Entity {
        /// State.
        state: Q1WireEntity,
    },
    /// Baseline.
    Baseline {
        /// State.
        state: Q1WireEntity,
    },
    /// Static entity.
    Static {
        /// State.
        state: Q1WireEntity,
    },
    /// Sound.
    Sound {
        /// Entity.
        entity: u16,
        /// Channel.
        channel: u8,
        /// Sound index.
        index: u16,
        /// Volume byte.
        volume: u8,
        /// Attenuation.
        attenuation: f64,
        /// Origin.
        origin: [f64; 3],
    },
    /// Static sound.
    StaticSound {
        /// Sound index.
        index: u16,
        /// Volume byte.
        volume: u8,
        /// Attenuation.
        attenuation: f64,
        /// Origin.
        origin: [f64; 3],
    },
    /// Stop sound.
    StopSound {
        /// Entity.
        entity: u16,
        /// Channel.
        channel: u8,
    },
    /// Local sound.
    LocalSound {
        /// Index.
        index: u16,
    },
    /// Damage.
    Damage {
        /// Armor.
        armor: u8,
        /// Blood.
        blood: u8,
        /// Source.
        source: [f64; 3],
    },
    /// Particle.
    Particle {
        /// Origin.
        origin: [f64; 3],
        /// Direction.
        direction: [f64; 3],
        /// Count.
        count: u8,
        /// Color.
        color: u8,
    },
    /// Temporary entity.
    TemporaryEntity {
        /// Effect.
        effect: TemporaryEntity,
    },
    /// Pause.
    Pause {
        /// Paused.
        paused: bool,
    },
    /// Signon.
    Signon {
        /// Stage.
        stage: u8,
    },
    /// CD track.
    CdTrack {
        /// Track.
        track: u8,
        /// Loop track.
        loop_track: u8,
    },
    /// Fog.
    Fog {
        /// Density.
        density: f64,
        /// Color.
        color: [f64; 3],
        /// Transition seconds.
        transition_seconds: f64,
    },
    /// Valued private message.
    Valued {
        /// Kind.
        kind: NqValued,
        /// Value.
        value: i32,
    },
    /// Prompt begin.
    PromptBegin {
        /// Text.
        text: String,
        /// Choices.
        choices: u8,
    },
    /// Prompt choice.
    PromptChoice {
        /// Text.
        text: String,
        /// Impulse.
        impulse: u8,
    },
    /// Prompt clear.
    PromptClear,
}

/// NetQuake unit message kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NqUnit {
    /// No-op.
    Nop,
    /// Disconnect.
    Disconnect,
    /// Killed monster.
    KilledMonster,
    /// Found secret.
    FoundSecret,
    /// Intermission.
    Intermission,
    /// Sell screen.
    SellScreen,
    /// Bonus flash.
    BonusFlash,
    /// Level completed.
    LevelCompleted,
    /// Back to lobby.
    BackToLobby,
}

/// NetQuake text message kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NqText {
    /// Print.
    Print,
    /// Center print.
    CenterPrint,
    /// Stuff text.
    Stufftext,
    /// Finale.
    Finale,
    /// Cutscene.
    Cutscene,
    /// Skybox.
    Skybox,
    /// Achievement.
    Achievement,
    /// Bot chat.
    Botchat,
    /// Raw print.
    RawPrint,
    /// Chat.
    Chat,
    /// Server vars.
    ServerVars,
}

/// NetQuake named slot kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NqNamedSlot {
    /// Name.
    Name,
    /// Social.
    Social,
    /// Player info.
    PlayerInfo,
}

/// NetQuake numbered slot kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NqNumberedSlot {
    /// Frags.
    Frags,
    /// Colors.
    Colors,
    /// Ping.
    Ping,
}

/// NetQuake valued private kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NqValued {
    /// Spawned monster.
    SpawnedMonster,
    /// Set views.
    SetViews,
    /// Sequence.
    Sequence,
}

fn nq_flags(profile: NqProfile) -> u32 {
    crate::q1_wide::nq_protocol_flags(profile)
}

fn nq_max_precache(profile: NqProfile) -> usize {
    match profile {
        NqProfile::Netquake => crate::q1::MAX_PRECACHE,
        _ => crate::q1_wide::WIDE_MAX_PRECACHE,
    }
}

/// NetQuake decoder (`NetQuakeDecoder`).
#[derive(Debug)]
pub struct NetQuakeDecoder {
    /// Protocol.
    pub protocol: NqProfile,
    /// Rerelease message policy.
    pub rerelease_messages: RereleaseMessages,
    /// Standard Quake weapon rule.
    pub standard_quake: bool,
    /// Baselines by entity number.
    pub baselines: HashMap<u32, WideEntityState>,
    /// Server time in seconds.
    pub time_seconds: f32,
    flags: u32,
}

impl NetQuakeDecoder {
    /// Create a decoder.
    #[must_use]
    pub fn new(protocol: NqProfile, rerelease_messages: RereleaseMessages, standard_quake: bool) -> Self {
        let flags = nq_flags(protocol);
        Self {
            protocol,
            rerelease_messages,
            standard_quake,
            baselines: HashMap::new(),
            time_seconds: 0.0,
            flags,
        }
    }

    /// Protocol flags.
    #[must_use]
    pub fn flags(&self) -> u32 {
        self.flags
    }

    fn coord(&self, reader: &mut MsgReader<'_>) -> Result<f64, Q1NetError> {
        Ok(match self.protocol {
            NqProfile::Netquake => reader.float()? as f64,
            _ => reader.coord_flags(self.flags)?,
        })
    }

    fn angle(&self, reader: &mut MsgReader<'_>) -> Result<f64, Q1NetError> {
        Ok(match self.protocol {
            NqProfile::Netquake => f64::from(reader.char()?) * 360.0 / 256.0,
            _ => reader.angle_flags(self.flags)?,
        })
    }

    fn read_list(&self, reader: &mut MsgReader<'_>, max_precache: usize) -> Result<Vec<String>, Q1NetError> {
        let mut names = Vec::new();
        loop {
            let name = reader.string(512);
            reader.finish()?;
            if name.is_empty() {
                break;
            }
            if names.len() + 1 >= max_precache {
                return Err(Q1NetError::PrecacheOverflow);
            }
            names.push(name);
        }
        Ok(names)
    }

    fn read_entity(&mut self, reader: &mut MsgReader<'_>, initial: u32) -> Result<Q1WireEntity, Q1NetError> {
        let mut bits = initial;
        if bits & protocol::U_MOREBITS != 0 {
            bits |= u32::from(reader.byte()?) << 8;
        }
        bits = crate::q1_wide::read_wide_entity_bits(reader, bits)?;
        let number = if bits & protocol::U_LONGENTITY != 0 {
            u32::from(reader.short()? as u16)
        } else {
            u32::from(reader.byte()?)
        };
        let baseline = self.baselines.get(&number).cloned().unwrap_or_default();
        let mut state = baseline.clone();
        if bits & protocol::U_MODEL != 0 {
            state.modelindex = u16::from(reader.byte()?);
        }
        if bits & protocol::U_FRAME != 0 {
            state.frame = u16::from(reader.byte()?);
        }
        if bits & protocol::U_COLORMAP != 0 {
            state.colormap = reader.byte()?;
        }
        if bits & protocol::U_SKIN != 0 {
            state.skin = reader.byte()?;
        }
        if bits & protocol::U_EFFECTS != 0 {
            state.effects = reader.byte()?;
        }
        let angle_bits = [protocol::U_ANGLE1, protocol::U_ANGLE2, protocol::U_ANGLE3];
        let origin_bits = [protocol::U_ORIGIN1, protocol::U_ORIGIN2, protocol::U_ORIGIN3];
        for (((origin, angles), angle_bit), origin_bit) in state
            .origin
            .iter_mut()
            .zip(state.angles.iter_mut())
            .zip(angle_bits.iter())
            .zip(origin_bits.iter())
        {
            if bits & origin_bit != 0 {
                *origin = self.coord(reader)?;
            }
            if bits & angle_bit != 0 {
                *angles = self.angle(reader)?;
            }
        }
        let tail = crate::q1_wide::read_wide_entity_tail(reader, bits)?;
        if let Some(alpha) = tail.alpha {
            state.alpha = alpha;
        }
        if let Some(scale) = tail.scale {
            state.scale = scale;
        }
        if let Some(frame_high) = tail.frame_high {
            state.frame |= u16::from(frame_high) << 8;
        }
        if let Some(model_high) = tail.model_high {
            state.modelindex |= u16::from(model_high) << 8;
        }
        Ok(Q1WireEntity {
            number,
            state,
            lerp_finish_seconds: tail
                .lerpfinish
                .map(|finish| f64::from(self.time_seconds) + finish)
                .unwrap_or(0.0),
            step: bits & protocol::U_STEP != 0,
            quakeworld_flags: 0,
        })
    }

    fn read_client_data(&self, reader: &mut MsgReader<'_>) -> Result<NetQuakeMessage, Q1NetError> {
        let bits = match self.protocol {
            NqProfile::Netquake => u32::from(reader.short()? as u16),
            _ => crate::q1_wide::read_wide_clientdata_bits(reader)?,
        };
        let view_height = if bits & protocol::SU_VIEWHEIGHT != 0 {
            reader.char()?
        } else {
            22
        };
        let ideal_pitch = if bits & protocol::SU_IDEALPITCH != 0 {
            reader.char()?
        } else {
            0
        };
        let mut punch = [0i8; 3];
        let mut velocity = [0i16; 3];
        for axis in 0..3 {
            punch[axis] = if bits & (protocol::SU_PUNCH1 << axis) != 0 {
                reader.char()?
            } else {
                0
            };
            velocity[axis] = if bits & (protocol::SU_VELOCITY1 << axis) != 0 {
                i16::from(reader.char()?) * 16
            } else {
                0
            };
        }
        let items = reader.long()?;
        let weapon_frame = if bits & protocol::SU_WEAPONFRAME != 0 {
            u16::from(reader.byte()?)
        } else {
            0
        };
        let armor = if bits & protocol::SU_ARMOR != 0 {
            u16::from(reader.byte()?)
        } else {
            0
        };
        let weapon_model = if bits & protocol::SU_WEAPON != 0 {
            u16::from(reader.byte()?)
        } else {
            0
        };
        let health = reader.short()?;
        let ammo = u16::from(reader.byte()?);
        let shells = u16::from(reader.byte()?);
        let nails = u16::from(reader.byte()?);
        let rockets = u16::from(reader.byte()?);
        let cells = u16::from(reader.byte()?);
        let weapon = reader.byte()?;
        let tail = crate::q1_wide::read_wide_clientdata_tail(reader, bits)?;
        Ok(NetQuakeMessage::ClientData {
            weapon_alpha: tail.weaponalpha,
            data: NetQuakeClientData {
                view_height,
                ideal_pitch,
                punch_angles: punch,
                velocity,
                items,
                on_ground: bits & protocol::SU_ONGROUND != 0,
                in_water: bits & protocol::SU_INWATER != 0,
                weapon_frame: weapon_frame | u16::from(tail.weaponframe_high) << 8,
                armor: armor | u16::from(tail.armor_high) << 8,
                weapon_model: weapon_model | u16::from(tail.weapon_high) << 8,
                health,
                ammo: ammo | u16::from(tail.ammo_high) << 8,
                shells: shells | u16::from(tail.shells_high) << 8,
                nails: nails | u16::from(tail.nails_high) << 8,
                rockets: rockets | u16::from(tail.rockets_high) << 8,
                cells: cells | u16::from(tail.cells_high) << 8,
                active_weapon: if self.standard_quake {
                    u32::from(weapon)
                } else {
                    1 << u32::from(weapon)
                },
            },
        })
    }

    fn read_private(&self, reader: &mut MsgReader<'_>, op: u8) -> Result<NetQuakeMessage, Q1NetError> {
        if self.rerelease_messages != RereleaseMessages::Quake1ReTsPrivate {
            return Err(Q1NetError::BadPrivate);
        }
        match op {
            38 => Ok(NetQuakeMessage::Text {
                kind: NqText::Botchat,
                text: reader.string(512),
            }),
            39 => Ok(NetQuakeMessage::Valued {
                kind: NqValued::SpawnedMonster,
                value: i32::from(reader.byte()?),
            }),
            45 => Ok(NetQuakeMessage::Valued {
                kind: NqValued::SetViews,
                value: i32::from(reader.byte()?),
            }),
            46 => Ok(NetQuakeMessage::NumberedSlot {
                kind: NqNumberedSlot::Ping,
                slot: reader.byte()?,
                value: reader.short()?,
            }),
            47 => Ok(NetQuakeMessage::NamedSlot {
                kind: NqNamedSlot::Social,
                slot: reader.byte()?,
                value: reader.string(512),
            }),
            48 => Ok(NetQuakeMessage::NamedSlot {
                kind: NqNamedSlot::PlayerInfo,
                slot: reader.byte()?,
                value: reader.string(512),
            }),
            49 => Ok(NetQuakeMessage::Text {
                kind: NqText::RawPrint,
                text: reader.string(512),
            }),
            50 => Ok(NetQuakeMessage::Text {
                kind: NqText::ServerVars,
                text: reader.string(512),
            }),
            51 => Ok(NetQuakeMessage::Valued {
                kind: NqValued::Sequence,
                value: reader.long()?,
            }),
            53 => Ok(NetQuakeMessage::Text {
                kind: NqText::Chat,
                text: reader.string(512),
            }),
            54 => Ok(NetQuakeMessage::Unit(NqUnit::LevelCompleted)),
            55 => Ok(NetQuakeMessage::Unit(NqUnit::BackToLobby)),
            57 => match reader.byte()? {
                0 => Ok(NetQuakeMessage::PromptBegin {
                    text: reader.string(512),
                    choices: reader.byte()?,
                }),
                1 => Ok(NetQuakeMessage::PromptChoice {
                    text: reader.string(512),
                    impulse: reader.byte()?,
                }),
                2 => Ok(NetQuakeMessage::PromptClear),
                sub => Err(Q1NetError::UnknownPrivate(sub)),
            },
            _ => Err(Q1NetError::UnknownService(op)),
        }
    }

    /// Decode a message buffer (`decode`).
    pub fn decode(&mut self, bytes: &[u8]) -> Result<Vec<NetQuakeMessage>, Q1NetError> {
        let mut reader = MsgReader::new(bytes);
        let mut messages = Vec::new();
        while reader.remaining() > 0 {
            let op = reader.byte()?;
            let message = if op & 128 != 0 {
                NetQuakeMessage::Entity {
                    state: self.read_entity(&mut reader, u32::from(op & 127))?,
                }
            } else {
                match op {
                    1 => NetQuakeMessage::Unit(NqUnit::Nop),
                    2 => NetQuakeMessage::Unit(NqUnit::Disconnect),
                    3 => NetQuakeMessage::Stat {
                        index: reader.byte()?,
                        value: reader.long()?,
                    },
                    4 => {
                        let version = reader.long()?;
                        self.protocol = crate::q1_wide::net_quake_profile(version as u16, self.flags)
                            .map_err(|_| Q1NetError::BadProfile)?;
                        NetQuakeMessage::Version { version }
                    }
                    5 => NetQuakeMessage::SetView {
                        entity: reader.short()? as u16,
                    },
                    6 => {
                        let mask = u32::from(reader.byte()?);
                        let volume = if mask & protocol::SND_VOLUME != 0 {
                            reader.byte()?
                        } else {
                            255
                        };
                        let attenuation = if mask & protocol::SND_ATTENUATION != 0 {
                            f64::from(reader.byte()?) / 64.0
                        } else {
                            1.0
                        };
                        let header = match self.protocol {
                            NqProfile::Netquake => {
                                let channel = reader.short()? as u16;
                                crate::q1_wide::WideSoundHeader {
                                    ent: channel >> 3,
                                    channel: (channel & 7) as u8,
                                    sound_num: u16::from(reader.byte()?),
                                }
                            }
                            _ => crate::q1_wide::read_wide_sound_header(&mut reader, mask)?,
                        };
                        let origin = [
                            self.coord(&mut reader)?,
                            self.coord(&mut reader)?,
                            self.coord(&mut reader)?,
                        ];
                        NetQuakeMessage::Sound {
                            entity: header.ent,
                            channel: header.channel,
                            index: header.sound_num,
                            volume,
                            attenuation,
                            origin,
                        }
                    }
                    7 => {
                        self.time_seconds = reader.float()?;
                        NetQuakeMessage::Time {
                            seconds: self.time_seconds,
                        }
                    }
                    8 => NetQuakeMessage::Text {
                        kind: NqText::Print,
                        text: reader.string(512),
                    },
                    9 => NetQuakeMessage::Text {
                        kind: NqText::Stufftext,
                        text: reader.string(512),
                    },
                    10 => {
                        let angles = [
                            self.angle(&mut reader)?,
                            self.angle(&mut reader)?,
                            self.angle(&mut reader)?,
                        ];
                        NetQuakeMessage::SetAngle { angles }
                    }
                    11 => {
                        let version = reader.long()?;
                        let protocol = crate::q1_wide::net_quake_profile(
                            version as u16,
                            if version == 999 { reader.long()? as u32 } else { 0 },
                        )
                        .map_err(|_| Q1NetError::BadProfile)?;
                        self.protocol = protocol;
                        self.flags = nq_flags(protocol);
                        self.baselines.clear();
                        let max_clients = reader.byte()?;
                        let game_type = reader.byte()?;
                        let level = reader.string(512);
                        let max_precache = nq_max_precache(protocol);
                        let models = self.read_list(&mut reader, max_precache)?;
                        let sounds = self.read_list(&mut reader, max_precache)?;
                        NetQuakeMessage::ServerInfo {
                            protocol,
                            max_clients,
                            game_type,
                            level,
                            models,
                            sounds,
                        }
                    }
                    12 => NetQuakeMessage::LightStyle {
                        index: reader.byte()?,
                        value: reader.string(512),
                    },
                    13 => NetQuakeMessage::NamedSlot {
                        kind: NqNamedSlot::Name,
                        slot: reader.byte()?,
                        value: reader.string(512),
                    },
                    14 => NetQuakeMessage::NumberedSlot {
                        kind: NqNumberedSlot::Frags,
                        slot: reader.byte()?,
                        value: reader.short()?,
                    },
                    15 => self.read_client_data(&mut reader)?,
                    16 => {
                        let channel = reader.short()? as u16;
                        NetQuakeMessage::StopSound {
                            entity: channel >> 3,
                            channel: (channel & 7) as u8,
                        }
                    }
                    17 => NetQuakeMessage::NumberedSlot {
                        kind: NqNumberedSlot::Colors,
                        slot: reader.byte()?,
                        value: i16::from(reader.byte()?),
                    },
                    18 => {
                        let origin = [
                            self.coord(&mut reader)?,
                            self.coord(&mut reader)?,
                            self.coord(&mut reader)?,
                        ];
                        let direction = [
                            f64::from(reader.char()?) / 16.0,
                            f64::from(reader.char()?) / 16.0,
                            f64::from(reader.char()?) / 16.0,
                        ];
                        NetQuakeMessage::Particle {
                            origin,
                            direction,
                            count: reader.byte()?,
                            color: reader.byte()?,
                        }
                    }
                    19 => NetQuakeMessage::Damage {
                        armor: reader.byte()?,
                        blood: reader.byte()?,
                        source: [
                            self.coord(&mut reader)?,
                            self.coord(&mut reader)?,
                            self.coord(&mut reader)?,
                        ],
                    },
                    20 | 22 | 42 | 43 => {
                        let baseline = op == 22 || op == 42;
                        let number = if baseline { u32::from(reader.short()? as u16) } else { 0 };
                        let state = match self.protocol {
                            NqProfile::Netquake => {
                                let mut state = WideEntityState {
                                    modelindex: u16::from(reader.byte()?),
                                    frame: u16::from(reader.byte()?),
                                    colormap: reader.byte()?,
                                    skin: reader.byte()?,
                                    alpha: protocol::ENTALPHA_DEFAULT,
                                    scale: protocol::ENTSCALE_DEFAULT,
                                    ..Default::default()
                                };
                                for (origin, angles) in state.origin.iter_mut().zip(state.angles.iter_mut()) {
                                    *origin = reader.float()? as f64;
                                    *angles = f64::from(reader.char()?) * 360.0 / 256.0;
                                }
                                state
                            }
                            _ => crate::q1_wide::read_wide_baseline(
                                &mut reader,
                                if op >= 42 { 2 } else { 1 },
                                self.flags,
                            )?,
                        };
                        if baseline {
                            self.baselines.insert(number, state.clone());
                        }
                        let state = Q1WireEntity {
                            number,
                            state,
                            lerp_finish_seconds: 0.0,
                            step: false,
                            quakeworld_flags: 0,
                        };
                        if baseline {
                            NetQuakeMessage::Baseline { state }
                        } else {
                            NetQuakeMessage::Static { state }
                        }
                    }
                    23 => {
                        let flags = self.flags;
                        let protocol = self.protocol;
                        let mut coord = |reader: &mut MsgReader<'_>| -> Result<f64, Q1NetError> {
                            Ok(match protocol {
                                NqProfile::Netquake => reader.float()? as f64,
                                _ => reader.coord_flags(flags)?,
                            })
                        };
                        NetQuakeMessage::TemporaryEntity {
                            effect: read_temporary_entity(&mut reader, &mut coord, false)?,
                        }
                    }
                    24 => NetQuakeMessage::Pause {
                        paused: reader.byte()? != 0,
                    },
                    25 => NetQuakeMessage::Signon { stage: reader.byte()? },
                    26 => NetQuakeMessage::Text {
                        kind: NqText::CenterPrint,
                        text: reader.string(512),
                    },
                    27 => NetQuakeMessage::Unit(NqUnit::KilledMonster),
                    28 => NetQuakeMessage::Unit(NqUnit::FoundSecret),
                    29 | 44 => {
                        let origin = [
                            self.coord(&mut reader)?,
                            self.coord(&mut reader)?,
                            self.coord(&mut reader)?,
                        ];
                        let index = match self.protocol {
                            NqProfile::Netquake => u16::from(reader.byte()?),
                            _ => {
                                crate::q1_wide::read_wide_static_sound_index(&mut reader, if op == 44 { 2 } else { 1 })?
                            }
                        };
                        NetQuakeMessage::StaticSound {
                            index,
                            volume: reader.byte()?,
                            attenuation: f64::from(reader.byte()?) / 64.0,
                            origin,
                        }
                    }
                    30 => NetQuakeMessage::Unit(NqUnit::Intermission),
                    31 => NetQuakeMessage::Text {
                        kind: NqText::Finale,
                        text: reader.string(512),
                    },
                    32 => NetQuakeMessage::CdTrack {
                        track: reader.byte()?,
                        loop_track: reader.byte()?,
                    },
                    33 => NetQuakeMessage::Unit(NqUnit::SellScreen),
                    34 => NetQuakeMessage::Text {
                        kind: NqText::Cutscene,
                        text: reader.string(512),
                    },
                    37 => NetQuakeMessage::Text {
                        kind: NqText::Skybox,
                        text: reader.string(512),
                    },
                    40 => NetQuakeMessage::Unit(NqUnit::BonusFlash),
                    41 => NetQuakeMessage::Fog {
                        density: f64::from(reader.byte()?) / 255.0,
                        color: [
                            f64::from(reader.byte()?) / 255.0,
                            f64::from(reader.byte()?) / 255.0,
                            f64::from(reader.byte()?) / 255.0,
                        ],
                        transition_seconds: f64::from(reader.short()?) / 100.0,
                    },
                    52 => NetQuakeMessage::Text {
                        kind: NqText::Achievement,
                        text: reader.string(512),
                    },
                    56 => {
                        let mask = u32::from(reader.byte()?);
                        NetQuakeMessage::LocalSound {
                            index: if mask & protocol::SND_LARGESOUND != 0 {
                                reader.short()? as u16
                            } else {
                                u16::from(reader.byte()?)
                            },
                        }
                    }
                    _ => self.read_private(&mut reader, op)?,
                }
            };
            reader.finish()?;
            messages.push(message);
        }
        Ok(messages)
    }
}

/// Write a NetQuake stat (`writeNetQuakeStat`).
pub fn write_net_quake_stat(writer: &mut MsgWriter, index: u8, value: i32) -> Result<(), Q1NetError> {
    writer.write_byte(3)?;
    writer.write_byte(index)?;
    writer.write_long(value)?;
    Ok(())
}

/// Write NetQuake time (`writeNetQuakeTime`).
pub fn write_net_quake_time(writer: &mut MsgWriter, seconds: f32) -> Result<(), Q1NetError> {
    writer.write_byte(7)?;
    writer.write_float(seconds)?;
    Ok(())
}

fn to_nq15_entity(state: &WideEntityState) -> crate::q1::EntityState {
    crate::q1::EntityState {
        modelindex: state.modelindex,
        frame: state.frame,
        colormap: state.colormap,
        skin: state.skin,
        effects: state.effects,
        origin: state.origin,
        angles: state.angles,
    }
}

/// Write a NetQuake entity update (`writeNetQuakeEntity`).
pub fn write_net_quake_entity(
    writer: &mut MsgWriter,
    profile: NqProfile,
    state: &Q1WireEntity,
    baseline: &Q1WireEntity,
    server_time: f64,
) -> Result<(), Q1NetError> {
    match profile {
        NqProfile::Netquake => crate::q1::write_entity_update(
            writer,
            state.number as u16,
            &crate::q1::EntityUpdate {
                state: to_nq15_entity(&state.state),
                baseline: to_nq15_entity(&baseline.state),
                step: state.step,
            },
        )
        .map_err(Q1NetError::from),
        _ => crate::q1_wide::write_wide_entity_update(
            writer,
            state.number as u16,
            &crate::q1_wide::WideEntityUpdate {
                state: state.state.clone(),
                baseline: baseline.state.clone(),
                step: state.step,
                sendinterval: state.lerp_finish_seconds != 0.0,
                lerpfinish: state.lerp_finish_seconds - server_time,
            },
            nq_flags(profile),
        )
        .map_err(Q1NetError::from),
    }
}

/// Write NetQuake client data (`writeNetQuakeClientData`).
pub fn write_net_quake_client_data(
    writer: &mut MsgWriter,
    profile: NqProfile,
    data: &NetQuakeClientData,
    weapon_alpha: u8,
    standard_quake: bool,
) -> Result<(), Q1NetError> {
    match profile {
        NqProfile::Netquake => {
            let data = ClientData {
                viewheight: data.view_height,
                idealpitch: data.ideal_pitch,
                punchangle: data.punch_angles,
                velocity: data.velocity,
                items: data.items,
                onground: data.on_ground,
                inwater: data.in_water,
                weaponframe: data.weapon_frame as u8,
                armorvalue: data.armor as u8,
                weaponmodelindex: data.weapon_model as u8,
                health: data.health,
                currentammo: data.ammo as u8,
                ammo_shells: data.shells as u8,
                ammo_nails: data.nails as u8,
                ammo_rockets: data.rockets as u8,
                ammo_cells: data.cells as u8,
                weapon: data.active_weapon as u8,
            };
            write_nq15_client_data(writer, &data, standard_quake)
        }
        _ => crate::q1_wide::write_wide_clientdata(
            writer,
            &crate::q1_wide::WideClientData {
                viewheight: data.view_height,
                idealpitch: data.ideal_pitch,
                punchangle: data.punch_angles,
                velocity: data.velocity,
                items: data.items,
                onground: data.on_ground,
                inwater: data.in_water,
                weaponframe: data.weapon_frame,
                armorvalue: data.armor,
                weaponmodelindex: data.weapon_model,
                health: data.health,
                currentammo: data.ammo,
                ammo_shells: data.shells,
                ammo_nails: data.nails,
                ammo_rockets: data.rockets,
                ammo_cells: data.cells,
                weapon: data.active_weapon as u8,
                alpha: weapon_alpha,
                standard_quake,
            },
        )
        .map_err(Q1NetError::from),
    }
}

fn write_nq15_client_data(writer: &mut MsgWriter, data: &ClientData, standard_quake: bool) -> Result<(), Q1NetError> {
    let mut bits = 0;
    if data.viewheight != protocol::DEFAULT_VIEWHEIGHT as i8 {
        bits |= protocol::SU_VIEWHEIGHT;
    }
    if data.idealpitch != 0 {
        bits |= protocol::SU_IDEALPITCH;
    }
    bits |= protocol::SU_ITEMS;
    if data.onground {
        bits |= protocol::SU_ONGROUND;
    }
    if data.inwater {
        bits |= protocol::SU_INWATER;
    }
    for axis in 0..3 {
        if data.punchangle[axis] != 0 {
            bits |= protocol::SU_PUNCH1 << axis;
        }
        if data.velocity[axis] != 0 {
            bits |= protocol::SU_VELOCITY1 << axis;
        }
    }
    if data.weaponframe != 0 {
        bits |= protocol::SU_WEAPONFRAME;
    }
    if data.armorvalue != 0 {
        bits |= protocol::SU_ARMOR;
    }
    bits |= protocol::SU_WEAPON;
    writer.write_byte(protocol::Svc::Clientdata as u8)?;
    writer.write_short(bits as i16)?;
    if bits & protocol::SU_VIEWHEIGHT != 0 {
        writer.write_char(data.viewheight)?;
    }
    if bits & protocol::SU_IDEALPITCH != 0 {
        writer.write_char(data.idealpitch)?;
    }
    for axis in 0..3 {
        if bits & (protocol::SU_PUNCH1 << axis) != 0 {
            writer.write_char(data.punchangle[axis])?;
        }
        if bits & (protocol::SU_VELOCITY1 << axis) != 0 {
            writer.write_char((data.velocity[axis] / 16) as i8)?;
        }
    }
    writer.write_long(data.items)?;
    if bits & protocol::SU_WEAPONFRAME != 0 {
        writer.write_byte(data.weaponframe)?;
    }
    if bits & protocol::SU_ARMOR != 0 {
        writer.write_byte(data.armorvalue)?;
    }
    writer.write_byte(data.weaponmodelindex)?;
    writer.write_short(data.health)?;
    writer.write_byte(data.currentammo)?;
    writer.write_byte(data.ammo_shells)?;
    writer.write_byte(data.ammo_nails)?;
    writer.write_byte(data.ammo_rockets)?;
    writer.write_byte(data.ammo_cells)?;
    if standard_quake {
        writer.write_byte(data.weapon)?;
    } else {
        let mut weapon = 0;
        for index in 0..32 {
            if u32::from(data.weapon) & (1 << index) != 0 {
                weapon = index;
                break;
            }
        }
        writer.write_byte(weapon as u8)?;
    }
    Ok(())
}

/// Write a NetQuake sound or static sound (`writeNetQuakeSound`).
pub fn write_net_quake_sound(
    writer: &mut MsgWriter,
    profile: NqProfile,
    message: &NetQuakeMessage,
) -> Result<bool, Q1NetError> {
    let (entity, channel, index, volume, attenuation, origin, statik) = match message {
        NetQuakeMessage::Sound {
            entity,
            channel,
            index,
            volume,
            attenuation,
            origin,
        } => (*entity, *channel, *index, *volume, *attenuation, *origin, false),
        NetQuakeMessage::StaticSound {
            index,
            volume,
            attenuation,
            origin,
        } => (0, 0, *index, *volume, *attenuation, *origin, true),
        _ => return Ok(false),
    };
    match profile {
        NqProfile::Netquake => {
            if statik {
                crate::q1::write_static_sound(writer, origin, index, f64::from(volume) / 255.0, attenuation)
                    .map_err(Q1NetError::from)
            } else {
                crate::q1::write_sound(
                    writer,
                    &crate::q1::SoundMessage {
                        ent: entity,
                        channel,
                        sound_num: index,
                        volume,
                        attenuation,
                        origin,
                    },
                )
                .map_err(Q1NetError::from)
            }
        }
        _ => {
            if statik {
                crate::q1_wide::write_wide_static_sound(
                    writer,
                    origin,
                    index,
                    f64::from(volume) / 255.0,
                    attenuation,
                    nq_flags(profile),
                )
                .map_err(Q1NetError::from)
            } else {
                crate::q1_wide::write_wide_sound(
                    writer,
                    &crate::q1_wide::WideSoundMessage {
                        ent: entity,
                        channel,
                        sound_num: index,
                        volume,
                        attenuation,
                        origin,
                    },
                    nq_flags(profile),
                )
                .map_err(Q1NetError::from)
            }
        }
    }
}

/// Write NetQuake server info (`writeNetQuakeServerInfo`).
pub fn write_net_quake_server_info(writer: &mut MsgWriter, message: &NetQuakeMessage) -> Result<(), Q1NetError> {
    let NetQuakeMessage::ServerInfo {
        protocol,
        max_clients,
        game_type,
        level,
        models,
        sounds,
    } = message
    else {
        return Err(Q1NetError::NoServerInfo);
    };
    writer.write_byte(11)?;
    let version = match protocol {
        NqProfile::Netquake => 15,
        NqProfile::Fitzquake => 666,
        NqProfile::Rmq { .. } => 999,
    };
    crate::q1_wide::write_wide_protocol(
        writer,
        version,
        nq_flags(*protocol),
        matches!(protocol, NqProfile::Rmq { .. }),
    )?;
    writer.write_byte(*max_clients)?;
    writer.write_byte(*game_type)?;
    writer.write_string(level)?;
    for list in [models, sounds] {
        for name in list.iter() {
            writer.write_string(name)?;
        }
        writer.write_byte(0)?;
    }
    Ok(())
}

fn nq_write_text(writer: &mut MsgWriter, op: u8, value: &str) -> Result<(), Q1NetError> {
    writer.write_byte(op)?;
    writer.write_string(value)?;
    Ok(())
}

fn nq_write_vec(writer: &mut MsgWriter, profile: NqProfile, flags: u32, origin: [f64; 3]) -> Result<(), Q1NetError> {
    for value in origin {
        match profile {
            NqProfile::Netquake => writer.write_float(value as f32)?,
            _ => writer.write_coord_flags(value, flags)?,
        }
    }
    Ok(())
}

/// Write a NetQuake message (`writeNetQuakeMessage`, excluding entities).
pub fn write_net_quake_message(
    writer: &mut MsgWriter,
    profile: NqProfile,
    message: &NetQuakeMessage,
    rerelease_messages: RereleaseMessages,
    standard_quake: bool,
) -> Result<(), Q1NetError> {
    let flags = nq_flags(profile);
    match message {
        NetQuakeMessage::Unit(unit) => {
            let op = match unit {
                NqUnit::Nop => 1,
                NqUnit::Disconnect => 2,
                NqUnit::KilledMonster => 27,
                NqUnit::FoundSecret => 28,
                NqUnit::Intermission => 30,
                NqUnit::SellScreen => 33,
                NqUnit::BonusFlash => 40,
                NqUnit::LevelCompleted => {
                    if rerelease_messages != RereleaseMessages::Quake1ReTsPrivate {
                        return Err(Q1NetError::BadPrivate);
                    }
                    54
                }
                NqUnit::BackToLobby => {
                    if rerelease_messages != RereleaseMessages::Quake1ReTsPrivate {
                        return Err(Q1NetError::BadPrivate);
                    }
                    55
                }
            };
            writer.write_byte(op)?;
        }
        NetQuakeMessage::Text { kind, text } => {
            let op = match kind {
                NqText::Print => 8,
                NqText::Stufftext => 9,
                NqText::CenterPrint => 26,
                NqText::Finale => 31,
                NqText::Cutscene => 34,
                NqText::Skybox => 37,
                NqText::Achievement => 52,
                NqText::Botchat => 38,
                NqText::RawPrint => 49,
                NqText::Chat => 53,
                NqText::ServerVars => 50,
            };
            if matches!(
                kind,
                NqText::Botchat | NqText::RawPrint | NqText::Chat | NqText::ServerVars
            ) && rerelease_messages != RereleaseMessages::Quake1ReTsPrivate
            {
                return Err(Q1NetError::BadPrivate);
            }
            nq_write_text(writer, op, text)?;
        }
        NetQuakeMessage::Time { seconds } => write_net_quake_time(writer, *seconds)?,
        NetQuakeMessage::Version { version } => {
            writer.write_byte(4)?;
            writer.write_long(*version)?;
        }
        NetQuakeMessage::Stat { index, value } => write_net_quake_stat(writer, *index, *value)?,
        NetQuakeMessage::SetView { entity } => {
            writer.write_byte(5)?;
            writer.write_short(*entity as i16)?;
        }
        NetQuakeMessage::SetAngle { angles } => {
            writer.write_byte(10)?;
            for &value in angles {
                match profile {
                    NqProfile::Netquake => writer.write_angle(value)?,
                    _ => writer.write_angle_flags(value, flags)?,
                }
            }
        }
        NetQuakeMessage::ServerInfo { .. } => write_net_quake_server_info(writer, message)?,
        NetQuakeMessage::LightStyle { index, value } => {
            writer.write_byte(12)?;
            writer.write_byte(*index)?;
            writer.write_string(value)?;
        }
        NetQuakeMessage::NamedSlot { kind, slot, value } => {
            if !matches!(kind, NqNamedSlot::Name) && rerelease_messages != RereleaseMessages::Quake1ReTsPrivate {
                return Err(Q1NetError::BadPrivate);
            }
            writer.write_byte(match kind {
                NqNamedSlot::Name => 13,
                NqNamedSlot::Social => 47,
                NqNamedSlot::PlayerInfo => 48,
            })?;
            writer.write_byte(*slot)?;
            writer.write_string(value)?;
        }
        NetQuakeMessage::NumberedSlot { kind, slot, value } => {
            if matches!(kind, NqNumberedSlot::Ping) && rerelease_messages != RereleaseMessages::Quake1ReTsPrivate {
                return Err(Q1NetError::BadPrivate);
            }
            match kind {
                NqNumberedSlot::Frags => {
                    writer.write_byte(14)?;
                    writer.write_byte(*slot)?;
                    writer.write_short(*value)?;
                }
                NqNumberedSlot::Colors => {
                    writer.write_byte(17)?;
                    writer.write_byte(*slot)?;
                    writer.write_byte(*value as u8)?;
                }
                NqNumberedSlot::Ping => {
                    writer.write_byte(46)?;
                    writer.write_byte(*slot)?;
                    writer.write_short(*value)?;
                }
            }
        }
        NetQuakeMessage::ClientData { data, weapon_alpha } => {
            write_net_quake_client_data(writer, profile, data, *weapon_alpha, standard_quake)?;
        }
        NetQuakeMessage::Entity { .. } => return Err(Q1NetError::Overflow),
        NetQuakeMessage::Baseline { state } => match profile {
            NqProfile::Netquake => {
                crate::q1::write_baseline(writer, state.number as u16, &to_nq15_entity(&state.state))?
            }
            _ => crate::q1_wide::write_wide_baseline(writer, state.number as u16, &state.state, flags)?,
        },
        NetQuakeMessage::Static { state } => {
            let written = match profile {
                NqProfile::Netquake => crate::q1::write_static(writer, &to_nq15_entity(&state.state))?,
                _ => crate::q1_wide::write_wide_static(
                    writer,
                    &state.state,
                    flags,
                    matches!(profile, NqProfile::Rmq { .. }),
                )?,
            };
            if !written {
                return Err(Q1NetError::StaticExceedsWire);
            }
        }
        NetQuakeMessage::Sound { .. } | NetQuakeMessage::StaticSound { .. } => {
            if !write_net_quake_sound(writer, profile, message)? {
                return Err(Q1NetError::SoundExceedsWire);
            }
        }
        NetQuakeMessage::StopSound { entity, channel } => {
            writer.write_byte(16)?;
            writer.write_short((entity * 8 + u16::from(*channel)) as i16)?;
        }
        NetQuakeMessage::LocalSound { index } => {
            writer.write_byte(56)?;
            writer.write_byte(if *index > 255 {
                protocol::SND_LARGESOUND as u8
            } else {
                0
            })?;
            if *index > 255 {
                writer.write_short(*index as i16)?;
            } else {
                writer.write_byte(*index as u8)?;
            }
        }
        NetQuakeMessage::Damage { armor, blood, source } => {
            writer.write_byte(19)?;
            writer.write_byte(*armor)?;
            writer.write_byte(*blood)?;
            nq_write_vec(writer, profile, flags, *source)?;
        }
        NetQuakeMessage::Particle {
            origin,
            direction,
            count,
            color,
        } => {
            writer.write_byte(18)?;
            nq_write_vec(writer, profile, flags, *origin)?;
            for &value in direction {
                writer.write_byte((value * 16.0).trunc() as u8)?;
            }
            writer.write_byte(*count)?;
            writer.write_byte(*color)?;
        }
        NetQuakeMessage::TemporaryEntity { effect } => {
            writer.write_byte(23)?;
            match effect {
                TemporaryEntity::Beam {
                    effect_type,
                    entity,
                    start,
                    end,
                } => {
                    writer.write_byte(*effect_type)?;
                    writer.write_short(*entity as i16)?;
                    nq_write_vec(writer, profile, flags, *start)?;
                    nq_write_vec(writer, profile, flags, *end)?;
                }
                TemporaryEntity::ExplosionColors {
                    origin,
                    color_start,
                    color_length,
                } => {
                    writer.write_byte(12)?;
                    nq_write_vec(writer, profile, flags, *origin)?;
                    writer.write_byte(*color_start)?;
                    writer.write_byte(*color_length)?;
                }
                TemporaryEntity::Point {
                    effect_type, origin, ..
                } => {
                    writer.write_byte(*effect_type)?;
                    nq_write_vec(writer, profile, flags, *origin)?;
                }
            }
        }
        NetQuakeMessage::Pause { paused } => {
            writer.write_byte(24)?;
            writer.write_byte(u8::from(*paused))?;
        }
        NetQuakeMessage::Signon { stage } => {
            writer.write_byte(25)?;
            writer.write_byte(*stage)?;
        }
        NetQuakeMessage::CdTrack { track, loop_track } => {
            writer.write_byte(32)?;
            writer.write_byte(*track)?;
            writer.write_byte(*loop_track)?;
        }
        NetQuakeMessage::Fog {
            density,
            color,
            transition_seconds,
        } => {
            writer.write_byte(41)?;
            writer.write_byte((density * 255.0) as u8)?;
            for &value in color {
                writer.write_byte((value * 255.0) as u8)?;
            }
            writer.write_short((transition_seconds * 100.0) as i16)?;
        }
        NetQuakeMessage::Valued { kind, value } => {
            if rerelease_messages != RereleaseMessages::Quake1ReTsPrivate {
                return Err(Q1NetError::BadPrivate);
            }
            match kind {
                NqValued::SpawnedMonster => {
                    writer.write_byte(39)?;
                    writer.write_byte(*value as u8)?;
                }
                NqValued::SetViews => {
                    writer.write_byte(45)?;
                    writer.write_byte(*value as u8)?;
                }
                NqValued::Sequence => {
                    writer.write_byte(51)?;
                    writer.write_long(*value)?;
                }
            }
        }
        NetQuakeMessage::PromptBegin { text, choices } => {
            if rerelease_messages != RereleaseMessages::Quake1ReTsPrivate {
                return Err(Q1NetError::BadPrivate);
            }
            writer.write_byte(57)?;
            writer.write_byte(0)?;
            writer.write_string(text)?;
            writer.write_byte(*choices)?;
        }
        NetQuakeMessage::PromptChoice { text, impulse } => {
            if rerelease_messages != RereleaseMessages::Quake1ReTsPrivate {
                return Err(Q1NetError::BadPrivate);
            }
            writer.write_byte(57)?;
            writer.write_byte(1)?;
            writer.write_string(text)?;
            writer.write_byte(*impulse)?;
        }
        NetQuakeMessage::PromptClear => {
            if rerelease_messages != RereleaseMessages::Quake1ReTsPrivate {
                return Err(Q1NetError::BadPrivate);
            }
            writer.write_byte(57)?;
            writer.write_byte(2)?;
        }
    }
    Ok(())
}

/// NetQuake user command (`Q1UserCommand`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NqUserCommand {
    /// Acknowledged server time in seconds.
    pub acknowledged_server_time_seconds: f32,
    /// View angles in degrees.
    pub view_angles: [f64; 3],
    /// Forward move.
    pub forward_move: i16,
    /// Side move.
    pub side_move: i16,
    /// Up move.
    pub up_move: i16,
    /// Buttons.
    pub buttons: u8,
    /// Impulse.
    pub impulse: u8,
}

/// Write a NetQuake move (`writeNetQuakeMove`).
pub fn write_net_quake_move(
    writer: &mut MsgWriter,
    command: &NqUserCommand,
    profile: NqProfile,
) -> Result<(), Q1NetError> {
    let flags = nq_flags(profile);
    writer.write_byte(3)?;
    writer.write_float(command.acknowledged_server_time_seconds)?;
    for axis in 0..3 {
        if matches!(profile, NqProfile::Netquake) {
            writer.write_angle(command.view_angles[axis])?;
        } else {
            writer.write_move_angle16(command.view_angles[axis], flags)?;
        }
    }
    writer.write_short(command.forward_move)?;
    writer.write_short(command.side_move)?;
    writer.write_short(command.up_move)?;
    writer.write_byte(command.buttons)?;
    writer.write_byte(command.impulse)?;
    Ok(())
}

/// NetQuake client message (`NetQuakeClientMessage`).
#[derive(Debug, Clone, PartialEq)]
pub enum NetQuakeClientMessage {
    /// No-op.
    Nop,
    /// Disconnect.
    Disconnect,
    /// String command.
    StringCommand {
        /// Command text.
        text: String,
    },
    /// Move.
    Move {
        /// Command.
        command: NqUserCommand,
    },
}

/// Decode NetQuake client messages (`decodeNetQuakeClient`).
pub fn decode_net_quake_client(bytes: &[u8], profile: NqProfile) -> Result<Vec<NetQuakeClientMessage>, Q1NetError> {
    let flags = nq_flags(profile);
    let mut reader = MsgReader::new(bytes);
    let mut out = Vec::new();
    while reader.remaining() > 0 {
        match reader.byte()? {
            1 => out.push(NetQuakeClientMessage::Nop),
            2 => out.push(NetQuakeClientMessage::Disconnect),
            4 => out.push(NetQuakeClientMessage::StringCommand {
                text: reader.string(512),
            }),
            3 => {
                let acknowledged = reader.float()?;
                let mut view_angles = [0.0; 3];
                for slot in view_angles.iter_mut() {
                    *slot = if matches!(profile, NqProfile::Netquake) {
                        f64::from(reader.char()?) * 360.0 / 256.0
                    } else {
                        reader.move_angle16(flags)?
                    };
                }
                out.push(NetQuakeClientMessage::Move {
                    command: NqUserCommand {
                        acknowledged_server_time_seconds: acknowledged,
                        view_angles,
                        forward_move: reader.short()?,
                        side_move: reader.short()?,
                        up_move: reader.short()?,
                        buttons: reader.byte()?,
                        impulse: reader.byte()?,
                    },
                });
            }
            op => return Err(Q1NetError::UnknownClientService(op)),
        }
        reader.finish()?;
    }
    Ok(out)
}

/// NetQuake move sender (`NetQuakeMoveSender`).
#[derive(Debug, Default)]
pub struct NetQuakeMoveSender {
    messages: u32,
}

impl NetQuakeMoveSender {
    /// Reset at a map change.
    pub fn reset(&mut self) {
        self.messages = 0;
    }

    /// Next move message (`next`).
    pub fn next(
        &mut self,
        command: &NqUserCommand,
        profile: NqProfile,
        demo_playback: bool,
    ) -> Result<Option<Vec<u8>>, Q1NetError> {
        if demo_playback {
            return Ok(None);
        }
        self.messages += 1;
        if self.messages <= 2 {
            return Ok(None);
        }
        let mut writer = MsgWriter::new(128, false);
        write_net_quake_move(&mut writer, command, profile)?;
        Ok(Some(writer.bytes().to_vec()))
    }
}

// ---------------------------------------------------------------------------
// QuakeWorld messages (q1/quakeworld.ts)
// ---------------------------------------------------------------------------

/// QuakeWorld movement variables (`QwMoveVariables`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QwMoveVariables {
    /// Gravity.
    pub gravity: f32,
    /// Stop speed.
    pub stop_speed: f32,
    /// Maximum speed.
    pub max_speed: f32,
    /// Spectator maximum speed.
    pub spectator_max_speed: f32,
    /// Accelerate.
    pub accelerate: f32,
    /// Air accelerate.
    pub air_accelerate: f32,
    /// Water accelerate.
    pub water_accelerate: f32,
    /// Friction.
    pub friction: f32,
    /// Water friction.
    pub water_friction: f32,
    /// Entity gravity.
    pub entity_gravity: f32,
}

/// QuakeWorld player state (`QwPlayerState`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QwPlayerState {
    /// Player number.
    pub number: u8,
    /// Update flags.
    pub flags: u32,
    /// Origin.
    pub origin: [f64; 3],
    /// Velocity.
    pub velocity: [i16; 3],
    /// Model index.
    pub model_index: u16,
    /// Frame.
    pub frame: u8,
    /// Skin.
    pub skin: u8,
    /// Effects.
    pub effects: u8,
    /// Weapon frame.
    pub weapon_frame: u8,
    /// Milliseconds.
    pub milliseconds: u8,
    /// Command.
    pub command: QwUsercmd,
}

/// QuakeWorld download result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QwDownload {
    /// Missing file.
    Missing,
    /// Data chunk.
    Data {
        /// Percent.
        percent: u8,
        /// Bytes.
        bytes: Vec<u8>,
    },
}

/// Nail projectile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QwProjectile {
    /// Origin.
    pub origin: [i32; 3],
    /// Pitch.
    pub pitch: i32,
    /// Yaw.
    pub yaw: i32,
}

/// QuakeWorld message (`QuakeWorldMessage`).
#[derive(Debug, Clone, PartialEq)]
pub enum QuakeWorldMessage {
    /// Unit messages.
    Unit(QwUnit),
    /// Stat.
    Stat {
        /// Index.
        index: u8,
        /// Value.
        value: i32,
    },
    /// Print.
    Print {
        /// Level.
        level: u8,
        /// Text.
        text: String,
    },
    /// Text messages.
    Text {
        /// Kind.
        kind: QwText,
        /// Text.
        text: String,
    },
    /// Set angle.
    SetAngle {
        /// Angles.
        angles: [f64; 3],
    },
    /// Set view / muzzle flash.
    ViewEntity {
        /// Muzzle flash flag.
        muzzle_flash: bool,
        /// Entity.
        entity: u16,
    },
    /// Server data.
    ServerData {
        /// Protocol.
        protocol: QwProfile,
        /// Server count.
        server_count: i64,
        /// Game directory.
        game_directory: String,
        /// Player slot.
        player_slot: u8,
        /// Spectator flag.
        spectator: bool,
        /// Level.
        level: String,
        /// Movement variables.
        move_variables: QwMoveVariables,
    },
    /// Light style.
    LightStyle {
        /// Index.
        index: u8,
        /// Value.
        value: String,
    },
    /// Slot stat.
    SlotStat {
        /// Kind.
        kind: QwSlotStat,
        /// Slot.
        slot: u8,
        /// Value.
        value: QwSlotValue,
    },
    /// User info.
    Userinfo {
        /// Slot.
        slot: u8,
        /// User id.
        user_id: i32,
        /// Value.
        value: String,
    },
    /// Set info.
    SetInfo {
        /// Slot.
        slot: u8,
        /// Key.
        key: String,
        /// Value.
        value: String,
    },
    /// Server info.
    ServerInfo {
        /// Key.
        key: String,
        /// Value.
        value: String,
    },
    /// Baseline.
    Baseline {
        /// State.
        state: Q1WireEntity,
    },
    /// Static entity.
    Static {
        /// State.
        state: Q1WireEntity,
    },
    /// Sound.
    Sound {
        /// Entity.
        entity: u16,
        /// Channel.
        channel: u8,
        /// Sound index.
        index: u16,
        /// Origin.
        origin: [f64; 3],
        /// Volume byte.
        volume: u8,
        /// Attenuation.
        attenuation: f64,
    },
    /// Static sound.
    StaticSound {
        /// Sound index.
        index: u16,
        /// Origin.
        origin: [f64; 3],
        /// Volume byte.
        volume: u8,
        /// Attenuation.
        attenuation: f64,
    },
    /// Stop sound.
    StopSound {
        /// Entity.
        entity: u16,
        /// Channel.
        channel: u8,
    },
    /// Damage.
    Damage {
        /// Armor.
        armor: u8,
        /// Blood.
        blood: u8,
        /// Source.
        source: [f64; 3],
    },
    /// Temporary entity.
    TemporaryEntity {
        /// Effect.
        effect: TemporaryEntity,
    },
    /// Pause.
    Pause {
        /// Paused.
        paused: bool,
    },
    /// Intermission.
    Intermission {
        /// Origin.
        origin: [f64; 3],
        /// Angles.
        angles: [f64; 3],
    },
    /// CD track.
    CdTrack {
        /// Track.
        track: u8,
    },
    /// Kick.
    Kick {
        /// Degrees (-2 or -4).
        degrees: i8,
    },
    /// Download.
    Download {
        /// Result.
        result: QwDownload,
    },
    /// Player.
    Player {
        /// State.
        state: QwPlayerState,
    },
    /// Nails.
    Nails {
        /// Projectiles.
        projectiles: Vec<QwProjectile>,
    },
    /// Choke count.
    ChokeCount {
        /// Count.
        count: u8,
    },
    /// Model list.
    ModelList {
        /// First index.
        first: u16,
        /// Names.
        names: Vec<String>,
        /// Next index.
        next: u16,
    },
    /// Sound list.
    SoundList {
        /// First index.
        first: u16,
        /// Names.
        names: Vec<String>,
        /// Next index.
        next: u16,
    },
    /// Packet entities.
    PacketEntities {
        /// Sequence.
        sequence: u32,
        /// Delta base sequence.
        delta_sequence: Option<u32>,
        /// Entities.
        entities: Vec<Q1WireEntity>,
    },
    /// Invalid delta.
    InvalidDelta {
        /// Sequence.
        sequence: u32,
        /// Requested base.
        requested: u8,
    },
    /// Speed update.
    Speed {
        /// Entity gravity flag.
        entity_gravity: bool,
        /// Value.
        value: f32,
    },
}

/// QuakeWorld unit kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QwUnit {
    /// No-op.
    Nop,
    /// Disconnect.
    Disconnect,
    /// Killed monster.
    KilledMonster,
    /// Found secret.
    FoundSecret,
    /// Sell screen.
    SellScreen,
}

/// QuakeWorld text kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QwText {
    /// Stuff text.
    Stufftext,
    /// Center print.
    CenterPrint,
    /// Finale.
    Finale,
}

/// QuakeWorld slot stat kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QwSlotStat {
    /// Frags.
    Frags,
    /// Ping.
    Ping,
    /// Enter time.
    EnterTime,
    /// Packet loss.
    PacketLoss,
}

/// QuakeWorld slot stat values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QwSlotValue {
    /// Integer.
    Integer(i32),
    /// Float.
    Float(f32),
}

fn qw_flags(profile: QwProfile) -> u32 {
    crate::q1_wide::qw_protocol_flags(profile)
}

fn qw_max_precache(profile: QwProfile) -> usize {
    match profile {
        QwProfile::Quakeworld => 512,
        QwProfile::Wide { .. } => crate::q1_wide::QW29_MAX_PRECACHE,
    }
}

fn qw_max_packet_entities(profile: QwProfile) -> usize {
    match profile {
        QwProfile::Quakeworld => qw_protocol::MAX_PACKET_ENTITIES,
        QwProfile::Wide { .. } => 256,
    }
}

fn qw_wire_to_entity(state: &crate::q1_wide::QwWideEntityState, flags: u32) -> Q1WireEntity {
    Q1WireEntity {
        number: state.number,
        state: WideEntityState {
            modelindex: state.modelindex,
            frame: state.frame,
            colormap: state.colormap,
            skin: state.skinnum,
            effects: state.effects,
            origin: state.origin,
            angles: state.angles,
            alpha: state.alpha,
            scale: state.scale,
        },
        lerp_finish_seconds: 0.0,
        step: false,
        quakeworld_flags: flags,
    }
}

fn qw_entity_to_wire(entity: &Q1WireEntity, solid: bool) -> crate::q1_wide::QwWideEntityState {
    crate::q1_wide::QwWideEntityState {
        number: entity.number,
        origin: entity.state.origin,
        angles: entity.state.angles,
        modelindex: entity.state.modelindex,
        frame: entity.state.frame,
        colormap: entity.state.colormap,
        skinnum: entity.state.skin,
        effects: entity.state.effects,
        alpha: entity.state.alpha,
        scale: entity.state.scale,
        solid,
    }
}

/// QuakeWorld decoder (`QuakeWorldDecoder`).
#[derive(Debug)]
pub struct QuakeWorldDecoder {
    /// Protocol.
    pub protocol: QwProfile,
    /// Baselines by entity number.
    pub baselines: HashMap<u32, crate::q1_wide::QwWideEntityState>,
    frames: HashMap<u32, Vec<crate::q1_wide::QwWideEntityState>>,
    delta_requests: HashMap<u32, Option<u32>>,
    /// Player model index.
    pub player_model_index: u16,
}

impl QuakeWorldDecoder {
    /// Create a decoder.
    #[must_use]
    pub fn new(protocol: QwProfile) -> Self {
        Self {
            protocol,
            baselines: HashMap::new(),
            frames: HashMap::new(),
            delta_requests: HashMap::new(),
            player_model_index: 0,
        }
    }

    /// Record a delta request (`recordDeltaRequest`).
    pub fn record_delta_request(&mut self, command_sequence: u32, base: Option<u32>) {
        self.delta_requests.insert(command_sequence, base);
        let stale: Vec<u32> = self
            .delta_requests
            .keys()
            .copied()
            .filter(|sequence| *sequence + 64 <= command_sequence)
            .collect();
        for sequence in stale {
            self.delta_requests.remove(&sequence);
        }
    }

    fn coord(&self, reader: &mut MsgReader<'_>) -> Result<f64, Q1NetError> {
        Ok(match self.protocol {
            QwProfile::Quakeworld => reader.float()? as f64,
            QwProfile::Wide { .. } => reader.coord_flags(qw_flags(self.protocol))?,
        })
    }

    fn angle(&self, reader: &mut MsgReader<'_>) -> Result<f64, Q1NetError> {
        Ok(match self.protocol {
            QwProfile::Quakeworld => reader.float()? as f64,
            QwProfile::Wide { .. } => reader.angle_flags(qw_flags(self.protocol))?,
        })
    }

    fn sound_index(&self, reader: &mut MsgReader<'_>) -> Result<u16, Q1NetError> {
        Ok(match self.protocol {
            QwProfile::Quakeworld => u16::from(crate::qw::read_sound_index(reader)?),
            QwProfile::Wide { .. } => crate::q1_wide::read_qw29_sound_index(reader)?,
        })
    }

    fn model_index(&self, reader: &mut MsgReader<'_>) -> Result<u16, Q1NetError> {
        Ok(match self.protocol {
            QwProfile::Quakeworld => u16::from(crate::qw::read_model_index(reader)?),
            QwProfile::Wide { .. } => crate::q1_wide::read_qw29_model_index(reader)?,
        })
    }

    fn precache_count(&self, reader: &mut MsgReader<'_>) -> Result<u16, Q1NetError> {
        Ok(match self.protocol {
            QwProfile::Quakeworld => u16::from(reader.byte()?),
            QwProfile::Wide { .. } => crate::q1_wide::read_qw29_precache_count(reader)?,
        })
    }

    fn read_baseline_state(&self, reader: &mut MsgReader<'_>) -> Result<crate::q1_wide::QwWideEntityState, Q1NetError> {
        match self.protocol {
            QwProfile::Quakeworld => {
                let state = crate::qw::read_baseline(reader)?;
                Ok(crate::q1_wide::QwWideEntityState {
                    number: u32::from(state.number),
                    origin: state.origin,
                    angles: state.angles,
                    modelindex: u16::from(state.modelindex),
                    frame: u16::from(state.frame),
                    colormap: state.colormap,
                    skinnum: state.skinnum,
                    effects: state.effects,
                    alpha: protocol::ENTALPHA_DEFAULT,
                    scale: protocol::ENTSCALE_DEFAULT,
                    solid: false,
                })
            }
            QwProfile::Wide { .. } => {
                crate::q1_wide::read_qw29_baseline(reader, qw_flags(self.protocol)).map_err(Q1NetError::from)
            }
        }
    }

    fn read_player(&mut self, reader: &mut MsgReader<'_>) -> Result<QwPlayerState, Q1NetError> {
        let number = reader.byte()?;
        if number >= 32 {
            return Err(Q1NetError::BadPlayerSlot);
        }
        let bits = u32::from(reader.short()? as u16);
        let origin = [self.coord(reader)?, self.coord(reader)?, self.coord(reader)?];
        let frame = reader.byte()?;
        let milliseconds = if bits & qw_protocol::PF_MSEC != 0 {
            reader.byte()?
        } else {
            0
        };
        let command = if bits & qw_protocol::PF_COMMAND != 0 {
            read_delta_usercmd(reader, &QwUsercmd::default())?
        } else {
            QwUsercmd::default()
        };
        let velocity = [
            if bits & qw_protocol::PF_VELOCITY1 != 0 {
                reader.short()?
            } else {
                0
            },
            if bits & qw_protocol::PF_VELOCITY2 != 0 {
                reader.short()?
            } else {
                0
            },
            if bits & qw_protocol::PF_VELOCITY3 != 0 {
                reader.short()?
            } else {
                0
            },
        ];
        Ok(QwPlayerState {
            number,
            flags: bits,
            origin,
            velocity,
            model_index: if bits & qw_protocol::PF_MODEL != 0 {
                self.model_index(reader)?
            } else {
                self.player_model_index
            },
            frame,
            skin: if bits & qw_protocol::PF_SKINNUM != 0 {
                reader.byte()?
            } else {
                0
            },
            effects: if bits & qw_protocol::PF_EFFECTS != 0 {
                reader.byte()?
            } else {
                0
            },
            weapon_frame: if bits & qw_protocol::PF_WEAPONFRAME != 0 {
                reader.byte()?
            } else {
                0
            },
            milliseconds,
            command,
        })
    }

    fn read_entities(
        &mut self,
        reader: &mut MsgReader<'_>,
        sequence: u32,
        delta: bool,
    ) -> Result<QuakeWorldMessage, Q1NetError> {
        let requested = if delta { Some(reader.byte()?) } else { None };
        let mut base_sequence = None;
        if let Some(requested) = requested {
            if self.delta_requests.contains_key(&sequence) {
                if let Some(Some(selected)) = self.delta_requests.get(&sequence) {
                    if self.frames.contains_key(selected) && sequence.wrapping_sub(*selected) < 63 {
                        base_sequence = Some(*selected);
                    }
                }
            } else {
                for previous in self.frames.keys() {
                    if previous & 255 == u32::from(requested)
                        && *previous < sequence
                        && base_sequence.is_none_or(|base| *previous > base)
                    {
                        base_sequence = Some(*previous);
                    }
                }
            }
        }
        let mut states: BTreeMap<u32, crate::q1_wide::QwWideEntityState> = base_sequence
            .and_then(|base| self.frames.get(&base))
            .map(|states| states.iter().map(|state| (state.number, state.clone())).collect())
            .unwrap_or_default();
        let invalid = delta && base_sequence.is_none();
        let mut previous_number = 0;
        let flags = qw_flags(self.protocol);
        let max_entities = qw_max_packet_entities(self.protocol);
        loop {
            let word = reader.short()? as u16;
            reader.finish()?;
            if word == 0 {
                break;
            }
            let (number, remove, state) = match self.protocol {
                QwProfile::Quakeworld => {
                    let number = u32::from(word & 511);
                    let mut bits = u32::from(word) & !511;
                    if bits & qw_protocol::U_MOREBITS != 0 {
                        bits |= u32::from(reader.byte()?);
                    }
                    let remove = bits & qw_protocol::U_REMOVE != 0;
                    let from = states
                        .get(&number)
                        .cloned()
                        .or_else(|| self.baselines.get(&number).cloned())
                        .unwrap_or_default();
                    let state = if remove {
                        from
                    } else {
                        let narrow_from = crate::qw::QwEntityState {
                            number: number as u16,
                            origin: from.origin,
                            angles: from.angles,
                            modelindex: from.modelindex as u8,
                            frame: from.frame as u8,
                            colormap: from.colormap,
                            skinnum: from.skinnum,
                            effects: from.effects,
                            ..crate::qw::QwEntityState::default()
                        };
                        let narrow = crate::qw::read_delta_entity(
                            reader,
                            &narrow_from,
                            crate::qw::EntityHeader {
                                number: number as u16,
                                bits,
                                remove,
                            },
                        )?;
                        crate::q1_wide::QwWideEntityState {
                            number,
                            origin: narrow.origin,
                            angles: narrow.angles,
                            modelindex: u16::from(narrow.modelindex),
                            frame: u16::from(narrow.frame),
                            colormap: narrow.colormap,
                            skinnum: narrow.skinnum,
                            effects: narrow.effects,
                            alpha: from.alpha,
                            scale: from.scale,
                            solid: narrow.solid,
                        }
                    };
                    (number, remove, state)
                }
                QwProfile::Wide { .. } => {
                    let header = crate::q1_wide::read_qw29_entity_header(reader, word)?;
                    let from = states
                        .get(&header.number)
                        .cloned()
                        .or_else(|| self.baselines.get(&header.number).cloned())
                        .unwrap_or_default();
                    let state = if header.remove {
                        from
                    } else {
                        crate::q1_wide::read_qw29_delta_entity(reader, &from, &header, flags)?
                    };
                    (header.number, header.remove, state)
                }
            };
            if number <= previous_number {
                return Err(Q1NetError::UnsortedEntities);
            }
            previous_number = number;
            if remove {
                if !delta {
                    return Err(Q1NetError::RemovalInFull);
                }
                states.remove(&number);
                continue;
            }
            states.insert(number, state);
            reader.finish()?;
            if states.len() > max_entities {
                return Err(Q1NetError::EntitiesOverflow);
            }
        }
        if invalid {
            return Ok(QuakeWorldMessage::InvalidDelta {
                sequence,
                requested: requested.unwrap_or(0),
            });
        }
        let mut values: Vec<crate::q1_wide::QwWideEntityState> = states.into_values().collect();
        values.sort_by_key(|state| state.number);
        self.frames.insert(sequence, values.clone());
        let stale: Vec<u32> = self
            .frames
            .keys()
            .copied()
            .filter(|old| *old + 64 <= sequence)
            .collect();
        for old in stale {
            self.frames.remove(&old);
        }
        Ok(QuakeWorldMessage::PacketEntities {
            sequence,
            delta_sequence: base_sequence,
            entities: values
                .iter()
                .map(|state| qw_wire_to_entity(state, u32::from(state.solid) * qw_protocol::U_SOLID))
                .collect(),
        })
    }

    /// Decode a message buffer (`decode`).
    pub fn decode(&mut self, bytes: &[u8], sequence: u32) -> Result<Vec<QuakeWorldMessage>, Q1NetError> {
        let mut reader = MsgReader::new(bytes);
        let mut messages = Vec::new();
        while reader.remaining() > 0 {
            let op = reader.byte()?;
            let message = match op {
                1 => QuakeWorldMessage::Unit(QwUnit::Nop),
                2 => QuakeWorldMessage::Unit(QwUnit::Disconnect),
                3 | 38 => QuakeWorldMessage::Stat {
                    index: reader.byte()?,
                    value: if op == 3 {
                        i32::from(reader.byte()?)
                    } else {
                        reader.long()?
                    },
                },
                5 => QuakeWorldMessage::ViewEntity {
                    muzzle_flash: false,
                    entity: reader.short()? as u16,
                },
                6 => {
                    let word = reader.short()? as u16;
                    let volume = if u32::from(word) & qw_protocol::SND_VOLUME != 0 {
                        reader.byte()?
                    } else {
                        255
                    };
                    let attenuation = if u32::from(word) & qw_protocol::SND_ATTENUATION != 0 {
                        f64::from(reader.byte()?) / 64.0
                    } else {
                        1.0
                    };
                    QuakeWorldMessage::Sound {
                        entity: (word >> 3) & 1023,
                        channel: (word & 7) as u8,
                        index: self.sound_index(&mut reader)?,
                        origin: [
                            self.coord(&mut reader)?,
                            self.coord(&mut reader)?,
                            self.coord(&mut reader)?,
                        ],
                        volume,
                        attenuation,
                    }
                }
                8 => QuakeWorldMessage::Print {
                    level: reader.byte()?,
                    text: reader.string(512),
                },
                9 => QuakeWorldMessage::Text {
                    kind: QwText::Stufftext,
                    text: reader.string(512),
                },
                10 => QuakeWorldMessage::SetAngle {
                    angles: [
                        self.angle(&mut reader)?,
                        self.angle(&mut reader)?,
                        self.angle(&mut reader)?,
                    ],
                },
                11 => {
                    let version = reader.long()?;
                    let profile = crate::q1_wide::quake_world_profile(
                        version as u16,
                        if version == 29 { reader.long()? as u32 } else { 0 },
                    )
                    .map_err(|_| Q1NetError::BadProfile)?;
                    self.protocol = profile;
                    self.baselines.clear();
                    self.frames.clear();
                    self.delta_requests.clear();
                    let server_count = reader.long()? as i64;
                    let game_directory = reader.string(512);
                    let slot = reader.byte()?;
                    let level = reader.string(512);
                    QuakeWorldMessage::ServerData {
                        protocol: profile,
                        server_count,
                        game_directory,
                        player_slot: slot & 127,
                        spectator: slot & 128 != 0,
                        level,
                        move_variables: QwMoveVariables {
                            gravity: reader.float()?,
                            stop_speed: reader.float()?,
                            max_speed: reader.float()?,
                            spectator_max_speed: reader.float()?,
                            accelerate: reader.float()?,
                            air_accelerate: reader.float()?,
                            water_accelerate: reader.float()?,
                            friction: reader.float()?,
                            water_friction: reader.float()?,
                            entity_gravity: reader.float()?,
                        },
                    }
                }
                12 => QuakeWorldMessage::LightStyle {
                    index: reader.byte()?,
                    value: reader.string(512),
                },
                14 | 36 | 37 | 53 => {
                    let slot = reader.byte()?;
                    let (kind, value) = if op == 14 {
                        (QwSlotStat::Frags, QwSlotValue::Integer(i32::from(reader.short()?)))
                    } else if op == 36 {
                        (QwSlotStat::Ping, QwSlotValue::Integer(i32::from(reader.short()?)))
                    } else if op == 37 {
                        (QwSlotStat::EnterTime, QwSlotValue::Float(reader.float()?))
                    } else {
                        (QwSlotStat::PacketLoss, QwSlotValue::Integer(i32::from(reader.byte()?)))
                    };
                    QuakeWorldMessage::SlotStat { kind, slot, value }
                }
                16 => {
                    let word = reader.short()? as u16;
                    QuakeWorldMessage::StopSound {
                        entity: word >> 3,
                        channel: (word & 7) as u8,
                    }
                }
                19 => QuakeWorldMessage::Damage {
                    armor: reader.byte()?,
                    blood: reader.byte()?,
                    source: [
                        self.coord(&mut reader)?,
                        self.coord(&mut reader)?,
                        self.coord(&mut reader)?,
                    ],
                },
                20 | 22 => {
                    let number = if op == 22 { u32::from(reader.short()? as u16) } else { 0 };
                    let mut state = self.read_baseline_state(&mut reader)?;
                    state.number = number;
                    if op == 22 {
                        self.baselines.insert(number, state.clone());
                    }
                    let entity = qw_wire_to_entity(&state, 0);
                    if op == 22 {
                        QuakeWorldMessage::Baseline { state: entity }
                    } else {
                        QuakeWorldMessage::Static { state: entity }
                    }
                }
                23 => {
                    let flags = qw_flags(self.protocol);
                    let protocol = self.protocol;
                    let mut coord = |reader: &mut MsgReader<'_>| -> Result<f64, Q1NetError> {
                        Ok(match protocol {
                            QwProfile::Quakeworld => reader.float()? as f64,
                            QwProfile::Wide { .. } => reader.coord_flags(flags)?,
                        })
                    };
                    QuakeWorldMessage::TemporaryEntity {
                        effect: read_temporary_entity(&mut reader, &mut coord, true)?,
                    }
                }
                24 => QuakeWorldMessage::Pause {
                    paused: reader.byte()? != 0,
                },
                26 => QuakeWorldMessage::Text {
                    kind: QwText::CenterPrint,
                    text: reader.string(512),
                },
                27 => QuakeWorldMessage::Unit(QwUnit::KilledMonster),
                28 => QuakeWorldMessage::Unit(QwUnit::FoundSecret),
                29 => {
                    let origin = [
                        self.coord(&mut reader)?,
                        self.coord(&mut reader)?,
                        self.coord(&mut reader)?,
                    ];
                    QuakeWorldMessage::StaticSound {
                        index: self.sound_index(&mut reader)?,
                        origin,
                        volume: reader.byte()?,
                        attenuation: f64::from(reader.byte()?) / 64.0,
                    }
                }
                30 => QuakeWorldMessage::Intermission {
                    origin: [
                        self.coord(&mut reader)?,
                        self.coord(&mut reader)?,
                        self.coord(&mut reader)?,
                    ],
                    angles: [
                        self.angle(&mut reader)?,
                        self.angle(&mut reader)?,
                        self.angle(&mut reader)?,
                    ],
                },
                31 => QuakeWorldMessage::Text {
                    kind: QwText::Finale,
                    text: reader.string(512),
                },
                32 => QuakeWorldMessage::CdTrack { track: reader.byte()? },
                33 => QuakeWorldMessage::Unit(QwUnit::SellScreen),
                34 | 35 => QuakeWorldMessage::Kick {
                    degrees: if op == 34 { -2 } else { -4 },
                },
                39 => QuakeWorldMessage::ViewEntity {
                    muzzle_flash: true,
                    entity: reader.short()? as u16,
                },
                40 => QuakeWorldMessage::Userinfo {
                    slot: reader.byte()?,
                    user_id: reader.long()?,
                    value: reader.string(512),
                },
                41 => {
                    let size = reader.short()?;
                    let percent = reader.byte()?;
                    QuakeWorldMessage::Download {
                        result: if size == -1 {
                            QwDownload::Missing
                        } else {
                            if size < 0 {
                                return Err(Q1NetError::Overflow);
                            }
                            QwDownload::Data {
                                percent,
                                bytes: reader.bytes(size as usize)?.to_vec(),
                            }
                        },
                    }
                }
                42 => QuakeWorldMessage::Player {
                    state: self.read_player(&mut reader)?,
                },
                43 => {
                    let count = reader.byte()?;
                    let mut projectiles = Vec::new();
                    for _ in 0..count {
                        let b = [
                            reader.byte()?,
                            reader.byte()?,
                            reader.byte()?,
                            reader.byte()?,
                            reader.byte()?,
                            reader.byte()?,
                        ];
                        projectiles.push(QwProjectile {
                            origin: [
                                (i32::from(b[0] | ((b[1] & 15) << 4)) << 1) - 4096,
                                (i32::from((b[1] >> 4) | (b[2] << 4)) << 1) - 4096,
                                (i32::from(b[3] | ((b[4] & 15) << 4)) << 1) - 4096,
                            ],
                            pitch: 360 * i32::from(b[4] >> 4) / 16,
                            yaw: 360 * i32::from(b[5]) / 256,
                        });
                    }
                    QuakeWorldMessage::Nails { projectiles }
                }
                44 => QuakeWorldMessage::ChokeCount { count: reader.byte()? },
                45 | 46 => {
                    let first = self.precache_count(&mut reader)?;
                    let mut names = Vec::new();
                    loop {
                        let name = reader.string(512);
                        reader.finish()?;
                        if name.is_empty() {
                            break;
                        }
                        if first as usize + names.len() + 1 >= qw_max_precache(self.protocol) {
                            return Err(Q1NetError::PrecacheOverflow);
                        }
                        names.push(name.clone());
                        if op == 45 && name == "progs/player.mdl" {
                            self.player_model_index = first + names.len() as u16;
                        }
                    }
                    let next = self.precache_count(&mut reader)?;
                    if op == 45 {
                        QuakeWorldMessage::ModelList { first, names, next }
                    } else {
                        QuakeWorldMessage::SoundList { first, names, next }
                    }
                }
                47 | 48 => self.read_entities(&mut reader, sequence, op == 48)?,
                49 | 50 => QuakeWorldMessage::Speed {
                    entity_gravity: op == 50,
                    value: reader.float()?,
                },
                51 => QuakeWorldMessage::SetInfo {
                    slot: reader.byte()?,
                    key: reader.string(512),
                    value: reader.string(512),
                },
                52 => QuakeWorldMessage::ServerInfo {
                    key: reader.string(512),
                    value: reader.string(512),
                },
                _ => return Err(Q1NetError::UnknownQwService(op)),
            };
            reader.finish()?;
            messages.push(message);
        }
        Ok(messages)
    }
}

/// Write QuakeWorld packet entities (`writeQuakeWorldEntities`).
pub fn write_quake_world_entities(
    writer: &mut MsgWriter,
    profile: QwProfile,
    states: &[Q1WireEntity],
    baseline: &HashMap<u32, Q1WireEntity>,
    previous: Option<(u32, &[Q1WireEntity])>,
) -> Result<(), Q1NetError> {
    let max_entities = qw_max_packet_entities(profile);
    if states.len() > max_entities {
        return Err(Q1NetError::TooManyEntities);
    }
    writer.write_byte(if previous.is_none() { 47 } else { 48 })?;
    if let Some((sequence, _)) = previous {
        writer.write_byte((sequence & 255) as u8)?;
    }
    let old: HashMap<u32, &Q1WireEntity> = previous
        .map(|(_, states)| states.iter().map(|state| (state.number, state)).collect())
        .unwrap_or_default();
    let current: HashMap<u32, &Q1WireEntity> = states.iter().map(|state| (state.number, state)).collect();
    let mut numbers: Vec<u32> = old.keys().chain(current.keys()).copied().collect();
    numbers.sort_unstable();
    numbers.dedup();
    let flags = qw_flags(profile);
    for number in numbers {
        let state = current.get(&number);
        let prior = old.get(&number);
        match state {
            None => match profile {
                QwProfile::Quakeworld => crate::qw::write_remove_entity(writer, number as u16)?,
                QwProfile::Wide { .. } => crate::q1_wide::write_qw29_remove_entity(writer, number)?,
            },
            Some(state) => {
                let base = prior
                    .map(|prior| qw_entity_to_wire(prior, false))
                    .or_else(|| baseline.get(&number).map(|base| qw_entity_to_wire(base, false)))
                    .unwrap_or_default();
                let force = prior.is_none();
                let written = match profile {
                    QwProfile::Quakeworld => {
                        let from = crate::qw::QwEntityState {
                            number: base.number as u16,
                            origin: base.origin,
                            angles: base.angles,
                            modelindex: base.modelindex as u8,
                            frame: base.frame as u8,
                            colormap: base.colormap,
                            skinnum: base.skinnum,
                            effects: base.effects,
                            ..crate::qw::QwEntityState::default()
                        };
                        let to = crate::qw::QwEntityState {
                            number: number as u16,
                            origin: state.state.origin,
                            angles: state.state.angles,
                            modelindex: state.state.modelindex as u8,
                            frame: state.state.frame as u8,
                            colormap: state.state.colormap,
                            skinnum: state.state.skin,
                            effects: state.state.effects,
                            ..crate::qw::QwEntityState::default()
                        };
                        crate::qw::write_delta_entity(writer, &from, &to, force)?;
                        true
                    }
                    QwProfile::Wide { .. } => crate::q1_wide::write_qw29_delta_entity(
                        writer,
                        &base,
                        &qw_entity_to_wire(state, false),
                        force,
                        flags,
                    )?,
                };
                if !written {
                    return Err(Q1NetError::EntityExceedsProfile(number));
                }
            }
        }
    }
    match profile {
        QwProfile::Quakeworld => crate::qw::write_packet_entities_end(writer)?,
        QwProfile::Wide { .. } => crate::q1_wide::write_qw29_packet_entities_end(writer)?,
    }
    Ok(())
}

/// Write a QuakeWorld player (`writeQuakeWorldPlayer`).
pub fn write_quake_world_player(
    writer: &mut MsgWriter,
    profile: QwProfile,
    state: &QwPlayerState,
) -> Result<(), Q1NetError> {
    let flags = qw_flags(profile);
    let bits = state.flags;
    writer.write_byte(42)?;
    writer.write_byte(state.number)?;
    writer.write_short(bits as i16)?;
    for axis in 0..3 {
        match profile {
            QwProfile::Quakeworld => writer.write_float(state.origin[axis] as f32)?,
            QwProfile::Wide { .. } => writer.write_coord_flags(state.origin[axis], flags)?,
        }
    }
    writer.write_byte(state.frame)?;
    if bits & qw_protocol::PF_MSEC != 0 {
        writer.write_byte(state.milliseconds)?;
    }
    if bits & qw_protocol::PF_COMMAND != 0 {
        write_delta_usercmd(writer, &QwUsercmd::default(), &state.command)?;
    }
    if bits & qw_protocol::PF_VELOCITY1 != 0 {
        writer.write_short(state.velocity[0])?;
    }
    if bits & qw_protocol::PF_VELOCITY2 != 0 {
        writer.write_short(state.velocity[1])?;
    }
    if bits & qw_protocol::PF_VELOCITY3 != 0 {
        writer.write_short(state.velocity[2])?;
    }
    if bits & qw_protocol::PF_MODEL != 0 {
        match profile {
            QwProfile::Quakeworld => crate::qw::write_model_index(writer, state.model_index as u8)?,
            QwProfile::Wide { .. } => crate::q1_wide::write_qw29_model_index(writer, state.model_index)?,
        }
    }
    if bits & qw_protocol::PF_SKINNUM != 0 {
        writer.write_byte(state.skin)?;
    }
    if bits & qw_protocol::PF_EFFECTS != 0 {
        writer.write_byte(state.effects)?;
    }
    if bits & qw_protocol::PF_WEAPONFRAME != 0 {
        writer.write_byte(state.weapon_frame)?;
    }
    Ok(())
}

/// Write QuakeWorld server data (`writeQuakeWorldServerData`).
pub fn write_quake_world_server_data(writer: &mut MsgWriter, message: &QuakeWorldMessage) -> Result<(), Q1NetError> {
    let QuakeWorldMessage::ServerData {
        protocol,
        server_count,
        game_directory,
        player_slot,
        spectator,
        level,
        move_variables,
    } = message
    else {
        return Err(Q1NetError::NoServerInfo);
    };
    writer.write_byte(11)?;
    match protocol {
        QwProfile::Quakeworld => crate::qw::write_protocol(writer)?,
        QwProfile::Wide { .. } => crate::q1_wide::write_qw29_protocol(writer, qw_flags(*protocol))?,
    }
    writer.write_long(*server_count as i32)?;
    writer.write_string(game_directory)?;
    writer.write_byte(player_slot | (u8::from(*spectator) * 128))?;
    writer.write_string(level)?;
    for value in [
        move_variables.gravity,
        move_variables.stop_speed,
        move_variables.max_speed,
        move_variables.spectator_max_speed,
        move_variables.accelerate,
        move_variables.air_accelerate,
        move_variables.water_accelerate,
        move_variables.friction,
        move_variables.water_friction,
        move_variables.entity_gravity,
    ] {
        writer.write_float(value)?;
    }
    Ok(())
}

/// Write a QuakeWorld download (`writeQuakeWorldDownload`).
pub fn write_quake_world_download(writer: &mut MsgWriter, result: &QwDownload) -> Result<(), Q1NetError> {
    writer.write_byte(41)?;
    match result {
        QwDownload::Missing => {
            writer.write_short(-1)?;
            writer.write_byte(0)?;
        }
        QwDownload::Data { percent, bytes } => {
            if bytes.len() > 768 {
                return Err(Q1NetError::DownloadTooLarge);
            }
            writer.write_short(bytes.len() as i16)?;
            writer.write_byte(*percent)?;
            writer.write_bytes(bytes)?;
        }
    }
    Ok(())
}

fn qw_write_text(writer: &mut MsgWriter, op: u8, value: &str) -> Result<(), Q1NetError> {
    writer.write_byte(op)?;
    writer.write_string(value)?;
    Ok(())
}

fn qw_write_coord(writer: &mut MsgWriter, profile: QwProfile, flags: u32, origin: [f64; 3]) -> Result<(), Q1NetError> {
    for value in origin {
        match profile {
            QwProfile::Quakeworld => writer.write_float(value as f32)?,
            QwProfile::Wide { .. } => writer.write_coord_flags(value, flags)?,
        }
    }
    Ok(())
}

fn qw_write_angle(writer: &mut MsgWriter, profile: QwProfile, flags: u32, angles: [f64; 3]) -> Result<(), Q1NetError> {
    for value in angles {
        match profile {
            QwProfile::Quakeworld => writer.write_float(value as f32)?,
            QwProfile::Wide { .. } => writer.write_angle_flags(value, flags)?,
        }
    }
    Ok(())
}

/// Write a QuakeWorld message (`writeQuakeWorldMessage`, excluding entities).
pub fn write_quake_world_message(
    writer: &mut MsgWriter,
    profile: QwProfile,
    message: &QuakeWorldMessage,
) -> Result<(), Q1NetError> {
    let flags = qw_flags(profile);
    let max_precache = qw_max_precache(profile);
    match message {
        QuakeWorldMessage::Unit(unit) => {
            writer.write_byte(match unit {
                QwUnit::Nop => 1,
                QwUnit::Disconnect => 2,
                QwUnit::KilledMonster => 27,
                QwUnit::FoundSecret => 28,
                QwUnit::SellScreen => 33,
            })?;
        }
        QuakeWorldMessage::Stat { index, value } => {
            if (0..=255).contains(value) {
                writer.write_byte(3)?;
                writer.write_byte(*index)?;
                writer.write_byte(*value as u8)?;
            } else {
                writer.write_byte(38)?;
                writer.write_byte(*index)?;
                writer.write_long(*value)?;
            }
        }
        QuakeWorldMessage::ViewEntity { muzzle_flash, entity } => {
            writer.write_byte(if *muzzle_flash { 39 } else { 5 })?;
            writer.write_short(*entity as i16)?;
        }
        QuakeWorldMessage::Sound {
            entity,
            channel,
            index,
            origin,
            volume,
            attenuation,
        } => {
            if *entity >= 1024 || *channel >= 8 || *index as usize >= max_precache {
                return Err(Q1NetError::SoundExceedsWire);
            }
            let mut mask = (u32::from(*entity) << 3) | u32::from(*channel);
            if *volume != 255 {
                mask |= qw_protocol::SND_VOLUME;
            }
            if *attenuation != 1.0 {
                mask |= qw_protocol::SND_ATTENUATION;
            }
            writer.write_byte(6)?;
            writer.write_short(mask as i16)?;
            if mask & qw_protocol::SND_VOLUME != 0 {
                writer.write_byte(*volume)?;
            }
            if mask & qw_protocol::SND_ATTENUATION != 0 {
                writer.write_byte((attenuation * 64.0) as u8)?;
            }
            match profile {
                QwProfile::Quakeworld => crate::qw::write_sound_index(writer, *index as u8)?,
                QwProfile::Wide { .. } => crate::q1_wide::write_qw29_sound_index(writer, *index)?,
            }
            qw_write_coord(writer, profile, flags, *origin)?;
        }
        QuakeWorldMessage::Print { level, text: value } => {
            writer.write_byte(8)?;
            writer.write_byte(*level)?;
            writer.write_string(value)?;
        }
        QuakeWorldMessage::Text { kind, text: value } => {
            qw_write_text(
                writer,
                match kind {
                    QwText::Stufftext => 9,
                    QwText::CenterPrint => 26,
                    QwText::Finale => 31,
                },
                value,
            )?;
        }
        QuakeWorldMessage::StaticSound {
            index,
            origin,
            volume,
            attenuation,
        } => {
            writer.write_byte(29)?;
            qw_write_coord(writer, profile, flags, *origin)?;
            match profile {
                QwProfile::Quakeworld => crate::qw::write_sound_index(writer, *index as u8)?,
                QwProfile::Wide { .. } => crate::q1_wide::write_qw29_sound_index(writer, *index)?,
            }
            writer.write_byte(*volume)?;
            writer.write_byte((attenuation * 64.0) as u8)?;
        }
        QuakeWorldMessage::SetAngle { angles } => {
            writer.write_byte(10)?;
            qw_write_angle(writer, profile, flags, *angles)?;
        }
        QuakeWorldMessage::ServerData { .. } => write_quake_world_server_data(writer, message)?,
        QuakeWorldMessage::LightStyle { index, value } => {
            writer.write_byte(12)?;
            writer.write_byte(*index)?;
            writer.write_string(value)?;
        }
        QuakeWorldMessage::SlotStat { kind, slot, value } => match kind {
            QwSlotStat::Frags => {
                writer.write_byte(14)?;
                writer.write_byte(*slot)?;
                writer.write_short(match value {
                    QwSlotValue::Integer(value) => *value as i16,
                    QwSlotValue::Float(value) => *value as i16,
                })?;
            }
            QwSlotStat::Ping => {
                writer.write_byte(36)?;
                writer.write_byte(*slot)?;
                writer.write_short(match value {
                    QwSlotValue::Integer(value) => *value as i16,
                    QwSlotValue::Float(value) => *value as i16,
                })?;
            }
            QwSlotStat::EnterTime => {
                writer.write_byte(37)?;
                writer.write_byte(*slot)?;
                writer.write_float(match value {
                    QwSlotValue::Integer(value) => *value as f32,
                    QwSlotValue::Float(value) => *value,
                })?;
            }
            QwSlotStat::PacketLoss => {
                writer.write_byte(53)?;
                writer.write_byte(*slot)?;
                writer.write_byte(match value {
                    QwSlotValue::Integer(value) => *value as u8,
                    QwSlotValue::Float(value) => *value as u8,
                })?;
            }
        },
        QuakeWorldMessage::StopSound { entity, channel } => {
            writer.write_byte(16)?;
            writer.write_short((entity * 8 + u16::from(*channel)) as i16)?;
        }
        QuakeWorldMessage::Damage { armor, blood, source } => {
            writer.write_byte(19)?;
            writer.write_byte(*armor)?;
            writer.write_byte(*blood)?;
            qw_write_coord(writer, profile, flags, *source)?;
        }
        QuakeWorldMessage::Static { state } | QuakeWorldMessage::Baseline { state } => {
            let baseline = matches!(message, QuakeWorldMessage::Baseline { .. });
            writer.write_byte(if baseline { 22 } else { 20 })?;
            if baseline {
                writer.write_short(state.number as i16)?;
            }
            match profile {
                QwProfile::Quakeworld => {
                    let wire = qw_entity_to_wire(state, false);
                    crate::qw::write_baseline(
                        writer,
                        &crate::qw::QwEntityState {
                            number: wire.number as u16,
                            origin: wire.origin,
                            angles: wire.angles,
                            modelindex: wire.modelindex as u8,
                            frame: wire.frame as u8,
                            colormap: wire.colormap,
                            skinnum: wire.skinnum,
                            effects: wire.effects,
                            ..crate::qw::QwEntityState::default()
                        },
                    )?;
                }
                QwProfile::Wide { .. } => {
                    crate::q1_wide::write_qw29_baseline(writer, &qw_entity_to_wire(state, false), flags)?;
                }
            }
        }
        QuakeWorldMessage::TemporaryEntity { effect } => {
            writer.write_byte(23)?;
            match effect {
                TemporaryEntity::Beam {
                    effect_type,
                    entity,
                    start,
                    end,
                } => {
                    writer.write_byte(*effect_type)?;
                    writer.write_short(*entity as i16)?;
                    qw_write_coord(writer, profile, flags, *start)?;
                    qw_write_coord(writer, profile, flags, *end)?;
                }
                TemporaryEntity::ExplosionColors { .. } => return Err(Q1NetError::ColorExplosion),
                TemporaryEntity::Point {
                    effect_type,
                    origin,
                    count,
                } => {
                    writer.write_byte(*effect_type)?;
                    if *effect_type == 2 || *effect_type == 12 {
                        writer.write_byte(*count)?;
                    }
                    qw_write_coord(writer, profile, flags, *origin)?;
                }
            }
        }
        QuakeWorldMessage::Pause { paused } => {
            writer.write_byte(24)?;
            writer.write_byte(u8::from(*paused))?;
        }
        QuakeWorldMessage::Intermission { origin, angles } => {
            writer.write_byte(30)?;
            qw_write_coord(writer, profile, flags, *origin)?;
            qw_write_angle(writer, profile, flags, *angles)?;
        }
        QuakeWorldMessage::CdTrack { track } => {
            writer.write_byte(32)?;
            writer.write_byte(*track)?;
        }
        QuakeWorldMessage::Kick { degrees } => {
            if *degrees != -2 && *degrees != -4 {
                return Err(Q1NetError::BadKick);
            }
            writer.write_byte(if *degrees == -2 { 34 } else { 35 })?;
        }
        QuakeWorldMessage::Userinfo { slot, user_id, value } => {
            writer.write_byte(40)?;
            writer.write_byte(*slot)?;
            writer.write_long(*user_id)?;
            writer.write_string(value)?;
        }
        QuakeWorldMessage::Download { result } => write_quake_world_download(writer, result)?,
        QuakeWorldMessage::Player { state } => write_quake_world_player(writer, profile, state)?,
        QuakeWorldMessage::Nails { projectiles } => {
            if projectiles.len() > 255 {
                return Err(Q1NetError::TooManyNails);
            }
            writer.write_byte(43)?;
            writer.write_byte(projectiles.len() as u8)?;
            for projectile in projectiles {
                let x = ((projectile.origin[0] + 4096) / 2) as u32;
                let y = ((projectile.origin[1] + 4096) / 2) as u32;
                let z = ((projectile.origin[2] + 4096) / 2) as u32;
                let pitch = ((projectile.pitch * 16 / 360) & 15) as u32;
                let yaw = ((projectile.yaw * 256 / 360) & 255) as u32;
                for byte in [
                    x & 255,
                    ((x >> 8) & 15) | ((y & 15) << 4),
                    (y >> 4) & 255,
                    z & 255,
                    ((z >> 8) & 15) | (pitch << 4),
                    yaw,
                ] {
                    writer.write_byte(byte as u8)?;
                }
            }
        }
        QuakeWorldMessage::ChokeCount { count } => {
            writer.write_byte(44)?;
            writer.write_byte(*count)?;
        }
        QuakeWorldMessage::ModelList { first, names, next } | QuakeWorldMessage::SoundList { first, names, next } => {
            writer.write_byte(if matches!(message, QuakeWorldMessage::ModelList { .. }) {
                45
            } else {
                46
            })?;
            match profile {
                QwProfile::Quakeworld => writer.write_byte(*first as u8)?,
                QwProfile::Wide { .. } => crate::q1_wide::write_qw29_precache_count(writer, *first)?,
            }
            for name in names {
                writer.write_string(name)?;
            }
            writer.write_byte(0)?;
            match profile {
                QwProfile::Quakeworld => writer.write_byte(*next as u8)?,
                QwProfile::Wide { .. } => crate::q1_wide::write_qw29_precache_count(writer, *next)?,
            }
        }
        QuakeWorldMessage::Speed { entity_gravity, value } => {
            writer.write_byte(if *entity_gravity { 50 } else { 49 })?;
            writer.write_float(*value)?;
        }
        QuakeWorldMessage::SetInfo { slot, key, value } => {
            writer.write_byte(51)?;
            writer.write_byte(*slot)?;
            writer.write_string(key)?;
            writer.write_string(value)?;
        }
        QuakeWorldMessage::ServerInfo { key, value } => {
            writer.write_byte(52)?;
            writer.write_string(key)?;
            writer.write_string(value)?;
        }
        QuakeWorldMessage::PacketEntities { .. } | QuakeWorldMessage::InvalidDelta { .. } => {
            return Err(Q1NetError::Overflow)
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Recording (q1/recording.ts, q1/qw-recording.ts)
// ---------------------------------------------------------------------------

/// NetQuake recording state (`NetQuakeRecordingState`).
#[derive(Debug, Default)]
pub struct NetQuakeRecordingState {
    info: Option<NetQuakeMessage>,
    statics: Vec<NetQuakeMessage>,
    fields: Vec<(String, NetQuakeMessage)>,
}

impl NetQuakeRecordingState {
    /// Observe messages (`observe`).
    pub fn observe(&mut self, messages: &[NetQuakeMessage]) {
        for message in messages {
            match message {
                NetQuakeMessage::ServerInfo { .. } => {
                    self.info = Some(message.clone());
                    self.statics.clear();
                    self.fields.clear();
                }
                NetQuakeMessage::Static { .. } | NetQuakeMessage::StaticSound { .. } => {
                    self.statics.push(message.clone());
                }
                NetQuakeMessage::Baseline { state } => {
                    self.set_field(format!("baseline:{}", state.number), message.clone());
                }
                NetQuakeMessage::NamedSlot { kind, slot, .. } => {
                    self.set_field(format!("{kind:?}:{slot}"), message.clone());
                }
                NetQuakeMessage::NumberedSlot { kind, slot, .. } => {
                    self.set_field(format!("{kind:?}:{slot}"), message.clone());
                }
                NetQuakeMessage::Stat { index, .. } => {
                    self.set_field(format!("stat:{index}"), message.clone());
                }
                NetQuakeMessage::LightStyle { index, .. } => {
                    self.set_field(format!("light-style:{index}"), message.clone());
                }
                NetQuakeMessage::SetView { .. }
                | NetQuakeMessage::SetAngle { .. }
                | NetQuakeMessage::ClientData { .. }
                | NetQuakeMessage::CdTrack { .. }
                | NetQuakeMessage::Time { .. }
                | NetQuakeMessage::Text { .. }
                | NetQuakeMessage::Unit(NqUnit::Intermission) => {
                    self.set_field(format!("{:?}", message_kind_name(message)), message.clone());
                }
                _ => {}
            }
        }
    }

    fn set_field(&mut self, key: String, message: NetQuakeMessage) {
        if let Some(slot) = self.fields.iter_mut().find(|(existing, _)| *existing == key) {
            slot.1 = message;
        } else {
            self.fields.push((key, message));
        }
    }

    /// Seed messages for a recording (`seed`).
    pub fn seed(
        &self,
        profile: NqProfile,
        rerelease_messages: RereleaseMessages,
        standard_quake: bool,
    ) -> Result<Vec<Vec<u8>>, Q1NetError> {
        let Some(info) = &self.info else {
            return Err(Q1NetError::NoServerInfo);
        };
        let mut messages = vec![info.clone(), NetQuakeMessage::Signon { stage: 1 }];
        messages.extend(self.statics.iter().cloned());
        messages.extend(self.fields.iter().map(|(_, message)| message.clone()));
        messages.push(NetQuakeMessage::Signon { stage: 2 });
        messages.push(NetQuakeMessage::Signon { stage: 3 });
        messages
            .iter()
            .map(|message| {
                let mut writer = MsgWriter::new(65536, false);
                write_net_quake_message(&mut writer, profile, message, rerelease_messages, standard_quake)?;
                Ok(writer.bytes().to_vec())
            })
            .collect()
    }
}

fn message_kind_name(message: &NetQuakeMessage) -> &'static str {
    match message {
        NetQuakeMessage::SetView { .. } => "set-view",
        NetQuakeMessage::SetAngle { .. } => "set-angle",
        NetQuakeMessage::ClientData { .. } => "client-data",
        NetQuakeMessage::CdTrack { .. } => "cd-track",
        NetQuakeMessage::Time { .. } => "time",
        NetQuakeMessage::Text { kind, .. } => match kind {
            NqText::Skybox => "skybox",
            NqText::Finale => "finale",
            NqText::Cutscene => "cutscene",
            _ => "text",
        },
        NetQuakeMessage::Unit(NqUnit::Intermission) => "intermission",
        _ => "other",
    }
}

/// QuakeWorld recording state (`QuakeWorldRecordingState`).
#[derive(Debug, Default)]
pub struct QuakeWorldRecordingState {
    fields: Vec<(String, QuakeWorldMessage)>,
    statics: Vec<QuakeWorldMessage>,
}

impl QuakeWorldRecordingState {
    /// Observe messages (`observe`).
    pub fn observe(&mut self, messages: &[QuakeWorldMessage]) {
        for message in messages {
            match message {
                QuakeWorldMessage::ServerData { .. } => {
                    self.fields.clear();
                    self.statics.clear();
                }
                QuakeWorldMessage::Static { .. } | QuakeWorldMessage::StaticSound { .. } => {
                    self.statics.push(message.clone());
                }
                QuakeWorldMessage::Baseline { state } => {
                    self.set_field(format!("baseline:{}", state.number), message.clone());
                }
                QuakeWorldMessage::LightStyle { index, .. } => {
                    self.set_field(format!("light-style:{index}"), message.clone());
                }
                QuakeWorldMessage::Stat { index, .. } => {
                    self.set_field(format!("stat:{index}"), message.clone());
                }
                QuakeWorldMessage::Userinfo { slot, .. } => {
                    let prefix = format!("set-info:{slot}:");
                    self.fields.retain(|(key, _)| !key.starts_with(&prefix));
                    self.set_field(format!("userinfo:{slot}"), message.clone());
                }
                QuakeWorldMessage::SetInfo { slot, key, .. } => {
                    self.set_field(format!("set-info:{slot}:{key}"), message.clone());
                }
                QuakeWorldMessage::ServerInfo { key, .. } => {
                    self.set_field(format!("server-info:{key}"), message.clone());
                }
                QuakeWorldMessage::SlotStat { kind, slot, .. } => {
                    self.set_field(format!("{kind:?}:{slot}"), message.clone());
                }
                QuakeWorldMessage::SetAngle { .. }
                | QuakeWorldMessage::ViewEntity { .. }
                | QuakeWorldMessage::Pause { .. }
                | QuakeWorldMessage::Intermission { .. }
                | QuakeWorldMessage::Text { .. }
                | QuakeWorldMessage::CdTrack { .. }
                | QuakeWorldMessage::Speed { .. } => {
                    self.set_field(qw_message_kind_name(message).to_owned(), message.clone());
                }
                _ => {}
            }
        }
    }

    fn set_field(&mut self, key: String, message: QuakeWorldMessage) {
        if let Some(slot) = self.fields.iter_mut().find(|(existing, _)| *existing == key) {
            slot.1 = message;
        } else {
            self.fields.push((key, message));
        }
    }

    /// Seed demo records for a recording (`seed`).
    pub fn seed(
        &self,
        data: &QuakeWorldMessage,
        models: &[String],
        sounds: &[String],
        seconds: f32,
        outgoing: i32,
        incoming: i32,
    ) -> Result<Vec<crate::demo::QwDemoRecord>, Q1NetError> {
        let QuakeWorldMessage::ServerData { protocol, .. } = data else {
            return Err(Q1NetError::NoServerInfo);
        };
        let mut messages = vec![data.clone()];
        for (model_list, names) in [(true, models), (false, sounds)] {
            if names.is_empty() {
                messages.push(if model_list {
                    QuakeWorldMessage::ModelList {
                        first: 0,
                        names: Vec::new(),
                        next: 0,
                    }
                } else {
                    QuakeWorldMessage::SoundList {
                        first: 0,
                        names: Vec::new(),
                        next: 0,
                    }
                });
            }
            for (first, name) in names.iter().enumerate() {
                let next = if first + 1 == names.len() { 0 } else { first + 1 };
                let list = if model_list {
                    QuakeWorldMessage::ModelList {
                        first: first as u16,
                        names: vec![name.clone()],
                        next: next as u16,
                    }
                } else {
                    QuakeWorldMessage::SoundList {
                        first: first as u16,
                        names: vec![name.clone()],
                        next: next as u16,
                    }
                };
                messages.push(list);
            }
        }
        messages.extend(self.statics.iter().cloned());
        messages.extend(self.fields.iter().map(|(_, message)| message.clone()));
        let mut records = Vec::new();
        for (index, message) in messages.iter().enumerate() {
            let mut payload = MsgWriter::new(1442, false);
            write_quake_world_message(&mut payload, *protocol, message)?;
            let mut bytes = vec![0u8; payload.cursize() + 8];
            bytes[0..4].copy_from_slice(&(index as i32 + 1).to_le_bytes());
            bytes[4..8].copy_from_slice(&(outgoing - 1).to_le_bytes());
            bytes[8..].copy_from_slice(payload.bytes());
            records.push(crate::demo::QwDemoRecord::Packet {
                seconds,
                message: bytes,
            });
        }
        records.push(crate::demo::QwDemoRecord::Sequences {
            seconds,
            outgoing,
            incoming,
        });
        Ok(records)
    }
}

fn qw_message_kind_name(message: &QuakeWorldMessage) -> String {
    match message {
        QuakeWorldMessage::SetAngle { .. } => "set-angle".to_owned(),
        QuakeWorldMessage::ViewEntity { .. } => "set-view".to_owned(),
        QuakeWorldMessage::Pause { .. } => "pause".to_owned(),
        QuakeWorldMessage::Intermission { .. } => "intermission".to_owned(),
        QuakeWorldMessage::Text { .. } => "finale".to_owned(),
        QuakeWorldMessage::CdTrack { .. } => "cd-track".to_owned(),
        QuakeWorldMessage::Speed { entity_gravity, .. } => {
            if *entity_gravity {
                "entity-gravity".to_owned()
            } else {
                "max-speed".to_owned()
            }
        }
        _ => "other".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn tokens_match_donor() {
        let mut state = Q1Tokenizer::default();
        assert_eq!(
            parse_q1_token(b"connect 28", &mut state, Q1TokenDialect::Quakeworld),
            Some("connect".to_owned())
        );
        assert_eq!(
            parse_q1_token(b"connect 28", &mut state, Q1TokenDialect::Quakeworld),
            Some("28".to_owned())
        );
        assert_eq!(
            parse_q1_token(b"connect 28", &mut state, Q1TokenDialect::Quakeworld),
            None
        );
        let mut state = Q1Tokenizer::default();
        assert_eq!(
            parse_q1_token(b"a // skip\nb", &mut state, Q1TokenDialect::Netquake),
            Some("a".to_owned())
        );
        assert_eq!(
            parse_q1_token(b"a // skip\nb", &mut state, Q1TokenDialect::Netquake),
            Some("b".to_owned())
        );
        let mut state = Q1Tokenizer::default();
        assert_eq!(
            parse_q1_token(br#"say "hi there""#, &mut state, Q1TokenDialect::Quakeworld),
            Some("say".to_owned())
        );
        assert_eq!(
            parse_q1_token(br#"say "hi there""#, &mut state, Q1TokenDialect::Quakeworld),
            Some("hi there".to_owned())
        );
        let mut state = Q1Tokenizer::default();
        assert_eq!(
            parse_q1_token(b"{", &mut state, Q1TokenDialect::Netquake),
            Some("{".to_owned())
        );
        // Unterminated quotes return the accumulated token.
        let mut state = Q1Tokenizer::default();
        assert_eq!(
            parse_q1_token(b"\"abc", &mut state, Q1TokenDialect::Quakeworld),
            Some("abc".to_owned())
        );
        assert_eq!(
            quake_world_info("\\a\\1\\b\\2"),
            BTreeMap::from([("a".to_owned(), "1".to_owned()), ("b".to_owned(), "2".to_owned())])
        );
    }

    #[test]
    fn netquake_channel_matches_donor_bytes() {
        let mut channel = NetQuakeChannel::new(8000, 1024).unwrap();
        channel.queue_reliable(&[9, 8, 7]).unwrap();
        assert_eq!(hex(&channel.next(0.0).unwrap().unwrap()), "0009000b00000000090807");
        assert_eq!(hex(&channel.unreliable(&[1, 2]).unwrap()), "0010000a000000000102");
        // Loopback: unreliable delivery with sequence tracking.
        let mut peer = NetQuakeChannel::new(8000, 1024).unwrap();
        let packet = channel.unreliable(&[5]).unwrap();
        let received = peer.receive(&packet, 1.0).unwrap();
        let delivery = received.delivery.unwrap();
        assert!(!delivery.reliable);
        assert_eq!(delivery.payload, vec![5]);
        assert_eq!(delivery.sequence, 1);
        // The peer never saw sequence 0, so one packet counts as dropped.
        assert_eq!(delivery.dropped, 1);
        // Reliable round trip with acknowledgment.
        let fragment = channel.next(2000.0).unwrap().unwrap();
        let received = peer.receive(&fragment, 2.0).unwrap();
        assert_eq!(received.replies.len(), 1);
        let ack = channel.receive(&received.replies[0], 3.0).unwrap();
        assert!(ack.delivery.is_none());
        assert!(channel.can_send_reliable());
        let delivery = received.delivery.unwrap();
        assert!(delivery.reliable);
        assert_eq!(delivery.payload, vec![9, 8, 7]);
    }

    #[test]
    fn quakeworld_channel_matches_donor_bytes() {
        let mut channel = QuakeWorldChannel::new(QuakeWorldSide::Client, 27001, 1450, 9999.0).unwrap();
        channel.queue_reliable(&[5, 6]).unwrap();
        assert_eq!(
            hex(&channel.transmit(&[7], 1000.0, false).unwrap()),
            "00000080000000007969050607"
        );
        let mut server = QuakeWorldChannel::new(QuakeWorldSide::Server, 27001, 1450, 9999.0).unwrap();
        let packet = channel.transmit(&[], 1100.0, false).unwrap();
        let delivery = server.receive(&packet, 1100.0).unwrap().unwrap();
        assert_eq!(delivery.payload, Vec::<u8>::new());
        assert_eq!(delivery.sequence, 1);
        assert_eq!(server.incoming_sequence(), 1);
    }

    #[test]
    fn control_packets_match_donor_bytes() {
        assert_eq!(
            hex(&encode_net_quake_control(&NetQuakeControl::ConnectRequest {
                game: "QUAKE".to_owned(),
                version: 3,
            })
            .unwrap()),
            "8000000c015155414b450003"
        );
        assert_eq!(
            hex(&encode_net_quake_control(&NetQuakeControl::Reject {
                reason: "nope".to_owned(),
            })
            .unwrap()),
            "8000000a826e6f706500"
        );
        assert_eq!(
            hex(&quake_world_out_of_band("getchallenge\n", false)),
            "ffffffff6765746368616c6c656e67650a"
        );
        let decoded = decode_net_quake_control(
            &encode_net_quake_control(&NetQuakeControl::RuleInfo {
                rule: Some(("a".to_owned(), "b".to_owned())),
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            decoded,
            NetQuakeControl::RuleInfo {
                rule: Some(("a".to_owned(), "b".to_owned())),
            }
        );
    }

    #[test]
    fn quakeworld_handshake_accepts() {
        struct Host;
        impl QuakeWorldConnectionHost for Host {
            fn password(&self) -> String {
                String::new()
            }
            fn spectator_password(&self) -> String {
                String::new()
            }
            fn rcon_password(&self) -> String {
                "secret".to_owned()
            }
            fn high_characters(&self) -> bool {
                true
            }
            fn blocked(&self, _from: &NetworkAddress) -> bool {
                false
            }
            fn connect(&mut self, _request: &QuakeWorldConnectRequest, _now: f64) -> QuakeWorldConnectVerdict {
                QuakeWorldConnectVerdict::Accepted
            }
            fn status(&self) -> String {
                "status".to_owned()
            }
            fn log(&self, _sequence: i64) -> Option<String> {
                None
            }
            fn execute_admin(&mut self, command: &str, write: &mut dyn FnMut(&str)) {
                write(&format!("ran {command}"));
            }
        }
        let from = crate::common::endpoint::ipv4_address([127, 0, 0, 1], 27001, false).unwrap();
        let mut host = Host;
        let mut server = QuakeWorldConnectionlessServer::new(&mut host, QuakeWorldChallenges::new(Box::new(|| 7), 8));
        let replies = server
            .receive(&quake_world_out_of_band("getchallenge\n", false), &from, 0.0)
            .unwrap();
        let challenge = read_quake_world_out_of_band(&replies[0]).unwrap();
        assert!(challenge.starts_with('c'));
        let replies = server
            .receive(
                &quake_world_out_of_band(
                    &format!("connect 28 27001 {} \"\\\\name\\\\p\"\n", &challenge[1..]),
                    false,
                ),
                &from,
                1.0,
            )
            .unwrap();
        assert_eq!(read_quake_world_out_of_band(&replies[0]).unwrap(), "j");
        let replies = server
            .receive(&quake_world_out_of_band("rcon secret status\n", false), &from, 2.0)
            .unwrap();
        assert!(read_quake_world_out_of_band(&replies[0])
            .unwrap()
            .contains("ran status"));
        let replies = server
            .receive(&quake_world_out_of_band("rcon wrong status\n", false), &from, 3.0)
            .unwrap();
        assert!(read_quake_world_out_of_band(&replies[0]).unwrap().contains("Bad rcon"));
    }

    #[test]
    fn netquake_signon_progresses() {
        let mut signon = NetQuakeSignon::new(NetQuakeSeatIdentity {
            name: "player".to_owned(),
            color: 0x13,
            spawn_parameters: "+map dm1".to_owned(),
            extension_flags: Some(1),
        });
        let reply = signon.receive(2).unwrap();
        let messages = decode_net_quake_client(&reply, NqProfile::Netquake).unwrap();
        assert_eq!(messages.len(), 4);
        assert!(signon.receive(2).is_err());
        signon.receive(3).unwrap();
        assert!(!signon.active());
        signon.first_entity();
        assert!(signon.active());
    }

    #[test]
    fn checksums_match_donor() {
        assert_eq!(quake_world_checksum(&[1, 2, 3, 4], 7).unwrap(), 63);
        assert_eq!(quake_world_checksum(&[65u8; 70], 12345).unwrap(), 8);
    }

    #[test]
    fn move_bundle_round_trips() {
        let bundle = QuakeWorldMove {
            oldest: QwUsercmd {
                msec: 10,
                angles: [1.0, 2.0, 3.0],
                forwardmove: 100,
                ..QwUsercmd::default()
            },
            previous: QwUsercmd {
                msec: 11,
                ..QwUsercmd::default()
            },
            current: QwUsercmd {
                msec: 12,
                buttons: 3,
                ..QwUsercmd::default()
            },
            loss_percent: 5,
        };
        let mut writer = MsgWriter::new(1450, false);
        write_quake_world_move(&mut writer, &bundle, 9).unwrap();
        let bytes = writer.bytes().to_vec();
        let messages = decode_quake_world_client(&bytes, QwProfile::Quakeworld, 9).unwrap();
        assert_eq!(messages.len(), 1);
        let QuakeWorldClientMessage::Move { bundle: decoded } = &messages[0] else {
            panic!("expected move");
        };
        assert_eq!(decoded.loss_percent, 5);
        assert_eq!(decoded.current.msec, 12);
        assert_eq!(decoded.oldest.forwardmove, 100);
        assert!(decode_quake_world_client(&bytes, QwProfile::Quakeworld, 10).is_err());
    }

    #[test]
    fn netquake_messages_match_donor_bytes() {
        let mut writer = MsgWriter::new(8000, false);
        write_net_quake_stat(&mut writer, 3, 42).unwrap();
        assert_eq!(hex(writer.bytes()), "03032a000000");
        let mut writer = MsgWriter::new(8000, false);
        write_net_quake_time(&mut writer, 12.5).unwrap();
        assert_eq!(hex(writer.bytes()), "0700004841");
        // Server info round trip.
        let mut writer = MsgWriter::new(8000, false);
        write_net_quake_server_info(
            &mut writer,
            &NetQuakeMessage::ServerInfo {
                protocol: NqProfile::Netquake,
                max_clients: 8,
                game_type: 1,
                level: "dm1".to_owned(),
                models: vec!["progs/player.mdl".to_owned()],
                sounds: Vec::new(),
            },
        )
        .unwrap();
        let bytes = writer.bytes().to_vec();
        let mut decoder = NetQuakeDecoder::new(NqProfile::Netquake, RereleaseMessages::KnownRetail, true);
        let messages = decoder.decode(&bytes).unwrap();
        assert_eq!(messages.len(), 1);
        let NetQuakeMessage::ServerInfo { level, models, .. } = &messages[0] else {
            panic!("expected server info");
        };
        assert_eq!(level, "dm1");
        assert_eq!(models.len(), 1);
    }

    #[test]
    fn quakeworld_server_data_round_trips() {
        let message = QuakeWorldMessage::ServerData {
            protocol: QwProfile::Wide { flags: 130 },
            server_count: 7,
            game_directory: "qw".to_owned(),
            player_slot: 3,
            spectator: false,
            level: "dm2".to_owned(),
            move_variables: QwMoveVariables {
                gravity: 800.0,
                ..QwMoveVariables::default()
            },
        };
        let mut writer = MsgWriter::new(1450, false);
        write_quake_world_server_data(&mut writer, &message).unwrap();
        let bytes = writer.bytes().to_vec();
        let mut decoder = QuakeWorldDecoder::new(QwProfile::Quakeworld);
        let messages = decoder.decode(&bytes, 1).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], message);
    }

    #[test]
    fn prediction_history_bundles_and_predicts() {
        struct Step;
        impl QwPredictionStep for Step {
            fn step(
                &mut self,
                state: &QwPredictionState,
                _command: &QwUsercmd,
                _sequence: u32,
            ) -> Option<QwPredictionState> {
                Some(QwPredictionState {
                    origin: [state.origin[0] + 1.0, 0.0, 0.0],
                    velocity: state.velocity,
                })
            }
        }
        let mut history = QuakeWorldPredictionHistory::default();
        for sequence in 100..104 {
            history.record(QuakeWorldPredictionFrame {
                sequence,
                sent_at_seconds: f64::from(sequence - 100) * 0.01,
                command: QwUsercmd {
                    msec: 10,
                    ..QwUsercmd::default()
                },
            });
        }
        let bundle = history.bundle(103, 0).unwrap();
        assert_eq!(bundle.current.msec, 10);
        let prediction = history.predict(&QwPredictionState::default(), 100, 104, 10.0, 0.0, &mut Step);
        assert!(matches!(prediction, QuakeWorldPrediction::Predicted { .. }));
    }

    #[test]
    fn discovery_reads_status() {
        let bytes = encode_net_quake_control(&NetQuakeControl::ServerInfo {
            address: "addr".to_owned(),
            name: "srv".to_owned(),
            map: "dm1".to_owned(),
            players: 2,
            max_players: 8,
            version: 3,
        })
        .unwrap();
        let status = read_net_quake_discovery(&bytes).unwrap();
        assert_eq!(status.name, "srv");
        assert_eq!(status.players, 2);
        let reply = quake_world_out_of_band(
            "n\\hostname\\srv\\map\\dm2\\maxclients\\8\n1 10 5 20 \"p\" \"s\" 0 0 \n",
            false,
        );
        let status = read_quake_world_discovery(&reply).unwrap();
        assert_eq!(status.map, "dm2");
        assert_eq!(status.max_players, 8);
        assert_eq!(status.player_details.len(), 1);
    }
}
