//! Quake II channels, handshakes, connectionless servers, frames, server
//! messages, downloads, MVD/GTV, and temp entities.
//!
//! Donor provenance: `src/network/q2/{channel,handshake,connectionless,
//! codec,frames,server-messages,server-write,temp-types,temp-entities,
//! mvd-recording,mvd-encoding,mvd-broadcast,mvd-playback,gtv,
//! gtv-transport}.ts` plus `parseQ2Token` in `src/core/common-parse.ts`.
//!
//! The donor's promise-based servers become synchronous poll pumps over
//! [`std::net`] sockets; every state transition keeps the donor's order and
//! limits. Protocol selection reuses [`ProtocolIdentity`]; per-protocol
//! framing lives in [`crate::q2`] and [`crate::q2_variants`].

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use flate2::Decompress;
use flate2::FlushDecompress;
use thiserror::Error;

use crate::common::endpoint::{address_key, ipv4_address, same_address, AddressError, NetworkAddress};
use crate::common::transport::{DatagramTransport, TransportError};
use crate::msg::{MsgError, MsgReader, MsgWriter};
use crate::protocol::q2 as protocol;
use crate::protocol::ProtocolIdentity;
use crate::q2::{read_dir, string_to_bytes, EntityState, FrameHeader, PlayerState, Q2CodecError, Usercmd};
use crate::q2_variants::{
    read_entity_bits_wide, read_q2pro_entity_bits, read_q2pro_int23, read_zpacket_payload, try_wrap_zpacket, BatchMove,
    BatchMoveFrame, FogData, KexCodec, KexConfigstringRecord, KexDamageIndicator, KexHelpPath, KexLocprint, KexPoi,
    Q2ProCodec, Q2ProFeatures, R1q2Codec, RereleaseCodec, VariantError, WideEntityBits, PROTOCOL_KEX_DEMOS,
};
use crate::services::discovery::{DiscoveryError, DiscoveryRequestKind, DiscoveryWire, PlayerDetail, ServerStatus};
use crate::services::downloads::DownloadError;

/// Error for Quake II netcode.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q2NetError {
    /// Underlying message failure.
    #[error("{0}")]
    Msg(#[from] MsgError),
    /// Underlying classic codec failure.
    #[error("{0}")]
    Codec(#[from] Q2CodecError),
    /// Underlying variant codec failure.
    #[error("{0}")]
    Variant(#[from] VariantError),
    /// Underlying transport failure.
    #[error("{0}")]
    Transport(#[from] TransportError),
    /// Underlying download failure.
    #[error("{0}")]
    Download(#[from] DownloadError),
    /// Underlying discovery failure.
    #[error("{0}")]
    Discovery(#[from] DiscoveryError),
    /// Underlying address failure.
    #[error("{0}")]
    Address(#[from] AddressError),
    /// Socket failure, with the display text of the I/O error.
    #[error("socket failure: {0}")]
    Io(String),
    /// Service opcode without a binding for the protocol.
    #[error("Q2 service opcode {opcode} has no binding")]
    Unbound {
        /// Opcode value.
        opcode: u8,
        /// Active protocol.
        protocol: ProtocolIdentity,
    },
    /// Serverdata protocol differs from the negotiated version.
    #[error("Q2 serverdata protocol {found} differs from negotiated {negotiated}")]
    ServerdataProtocol {
        /// Wire protocol.
        found: i64,
        /// Negotiated version.
        negotiated: u32,
    },
    /// Protocol violation, with the donor's message text.
    #[error("{0}")]
    Protocol(&'static str),
    /// Range violation, with the donor's message text.
    #[error("{0}")]
    Range(&'static str),
}

impl From<std::io::Error> for Q2NetError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// Parse cursor over UTF-16 units (`LegacyParseState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseState {
    /// Text units.
    pub data: Vec<u16>,
    /// Cursor position.
    pub index: usize,
}

impl ParseState {
    /// Build a cursor over text.
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            data: text.encode_utf16().collect(),
            index: 0,
        }
    }

    fn unit(&self, index: usize) -> u16 {
        self.data.get(index).copied().unwrap_or(0)
    }
}

/// Default token ceiling (`Q2_TOKEN_MAX`).
pub const Q2_TOKEN_MAX: usize = 128;

/// Parse a token (`parseQ2Token`, `COM_Parse`).
///
/// Operates on UTF-16 units exactly like the donor, including the quirk that
/// an over-long unquoted word parses to the empty string.
pub fn parse_q2_token(state: &mut ParseState, max_token_chars: usize) -> String {
    loop {
        let mut unit = state.unit(state.index);
        while unit <= 32 {
            if unit == 0 {
                return String::new();
            }
            state.index += 1;
            unit = state.unit(state.index);
        }
        if unit == 47 && state.unit(state.index + 1) == 47 {
            while state.unit(state.index) != 0 && state.unit(state.index) != 10 {
                state.index += 1;
            }
            continue;
        }
        break;
    }
    let mut unit = state.unit(state.index);
    let mut token = Vec::new();
    if unit == 34 {
        state.index += 1;
        loop {
            unit = state.unit(state.index);
            state.index += 1;
            if unit == 34 || unit == 0 {
                return String::from_utf16_lossy(&token);
            }
            if token.len() < max_token_chars {
                token.push(unit);
            }
        }
    }
    loop {
        if token.len() < max_token_chars {
            token.push(unit);
        }
        state.index += 1;
        unit = state.unit(state.index);
        if unit <= 32 {
            break;
        }
    }
    if token.len() == max_token_chars {
        return String::new();
    }
    String::from_utf16_lossy(&token)
}

/// Numeric wire version of a Q2 identity.
fn q2_version(protocol: ProtocolIdentity) -> u32 {
    protocol.version()
}

/// Whether an identity is a Q2 one.
fn is_q2(protocol: ProtocolIdentity) -> bool {
    matches!(
        protocol,
        ProtocolIdentity::Q2Classic
            | ProtocolIdentity::Q2R1q2 { .. }
            | ProtocolIdentity::Q2Q2pro { .. }
            | ProtocolIdentity::Q2Rerelease
            | ProtocolIdentity::Q2Kex
            | ProtocolIdentity::Q2KexDemo
            | ProtocolIdentity::Q2PrivateClassic
    )
}

// ---------------------------------------------------------------------------
// Channel
// ---------------------------------------------------------------------------

/// Channel side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelSide {
    /// Client side.
    Client,
    /// Server side.
    Server,
}

/// Channel generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    /// Original id channel.
    Old,
    /// Fragmenting channel.
    New,
}

/// Resend-sequence recording dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceRecording {
    /// Original id recording.
    Id,
    /// Q2Pro recording.
    Q2Pro,
}

/// Q2 channel options (`Q2ChannelOptions`).
#[derive(Debug, Clone)]
pub struct Q2ChannelOptions {
    /// Channel side.
    pub side: ChannelSide,
    /// Protocol identity.
    pub protocol: ProtocolIdentity,
    /// Channel generation.
    pub channel: ChannelKind,
    /// Qport.
    pub qport: u32,
    /// Writable payload limit.
    pub payload_bytes: Option<usize>,
    /// Reliable message capacity.
    pub message_bytes: Option<usize>,
    /// Transport datagram ceiling.
    pub max_datagram_bytes: Option<usize>,
    /// Server-side reliable compression.
    pub compress: bool,
    /// Resend-sequence recording dialect override.
    pub sequence_recording: Option<SequenceRecording>,
}

/// Q2 channel receive outcome (`Q2ChannelReceive`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2ChannelReceive {
    /// Complete message.
    Message {
        /// Sequence number.
        sequence: u32,
        /// Acknowledged sequence.
        acknowledged: u32,
        /// Dropped packets.
        dropped: u32,
        /// Payload bytes.
        bytes: Vec<u8>,
    },
    /// Fragment accepted, reassembly pending.
    Fragment {
        /// Sequence number.
        sequence: u32,
        /// Bytes received so far.
        received_bytes: usize,
    },
    /// Packet rejected.
    Rejected {
        /// Reason.
        reason: Q2RejectReason,
    },
}

/// Channel rejection reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2RejectReason {
    /// Packet too short.
    Short,
    /// Stale or duplicate sequence.
    Sequence,
    /// Fragment out of order.
    FragmentOrder,
    /// Fragment exceeds reassembly buffer.
    FragmentSize,
    /// Qport mismatch.
    Qport,
}

/// Quake II reliable channel (`Q2Channel`).
#[derive(Debug)]
pub struct Q2Channel {
    options: Q2ChannelOptions,
    incoming: u32,
    outgoing: u32,
    incoming_ack: u32,
    incoming_reliable: u32,
    incoming_reliable_ack: u32,
    ack_pending: bool,
    reliable_bit: u32,
    last_reliable: u32,
    queued: Vec<u8>,
    reliable: Vec<u8>,
    sending: Option<Q2Sending>,
    receiving: Vec<u8>,
    receive_sequence: u32,
    receive_length: usize,
    payload_bytes: usize,
    capacity: usize,
    last_sent_milliseconds: u64,
    last_received_milliseconds: u64,
}

#[derive(Debug)]
struct Q2Sending {
    bytes: Vec<u8>,
    reliable: bool,
    offset: usize,
}

impl Q2Channel {
    /// Build a channel, validating limits exactly like the donor.
    pub fn new(options: Q2ChannelOptions) -> Result<Self, Q2NetError> {
        if options.protocol == ProtocolIdentity::Q2Kex {
            return Ok(Self {
                options,
                incoming: 0,
                outgoing: 1,
                incoming_ack: 0,
                incoming_reliable: 0,
                incoming_reliable_ack: 0,
                ack_pending: false,
                reliable_bit: 0,
                last_reliable: 0,
                queued: Vec::new(),
                reliable: Vec::new(),
                sending: None,
                receiving: vec![0; 65527],
                receive_sequence: 0,
                receive_length: 0,
                payload_bytes: 65527,
                capacity: 65527,
                last_sent_milliseconds: 0,
                last_received_milliseconds: 0,
            });
        }
        let datagram_bytes = options.max_datagram_bytes.unwrap_or(65507);
        if !(524..=65507).contains(&datagram_bytes) {
            return Err(Q2NetError::Range("Invalid Q2 transport datagram limit"));
        }
        let payload_bytes = options.payload_bytes.unwrap_or(1390).min(datagram_bytes - 12);
        let recording = options.sequence_recording.unwrap_or_else(|| {
            if q2_version(options.protocol) == 34 {
                SequenceRecording::Id
            } else {
                SequenceRecording::Q2Pro
            }
        });
        let capacity = options.message_bytes.unwrap_or_else(|| {
            if options.channel == ChannelKind::New {
                32768
            } else if recording == SequenceRecording::Id {
                1384
            } else {
                payload_bytes
            }
        });
        let capacity = if options.channel == ChannelKind::New {
            capacity
        } else {
            capacity.min(datagram_bytes - 10)
        };
        if !(512..=4086).contains(&payload_bytes) || !(1..=32768).contains(&capacity) {
            return Err(Q2NetError::Range("Invalid Q2 channel limits"));
        }
        if options.qport > 65535 {
            return Err(Q2NetError::Range("Invalid Q2 qport"));
        }
        Ok(Self {
            options,
            incoming: 0,
            outgoing: 1,
            incoming_ack: 0,
            incoming_reliable: 0,
            incoming_reliable_ack: 0,
            ack_pending: false,
            reliable_bit: 0,
            last_reliable: 0,
            queued: Vec::new(),
            reliable: Vec::new(),
            sending: None,
            receiving: vec![0; 32768],
            receive_sequence: 0,
            receive_length: 0,
            payload_bytes,
            capacity,
            last_sent_milliseconds: 0,
            last_received_milliseconds: 0,
        })
    }

    /// Incoming sequence.
    #[must_use]
    pub fn incoming_sequence(&self) -> u32 {
        self.incoming
    }

    /// Incoming acknowledged sequence.
    #[must_use]
    pub fn incoming_acknowledged(&self) -> u32 {
        self.incoming_ack
    }

    /// Outgoing sequence.
    #[must_use]
    pub fn outgoing_sequence(&self) -> u32 {
        self.outgoing
    }

    /// Whether a fragmented send is in flight.
    #[must_use]
    pub fn fragment_pending(&self) -> bool {
        self.sending.is_some()
    }

    /// Whether reliable bytes await acknowledgement.
    #[must_use]
    pub fn reliable_pending(&self) -> bool {
        !self.queued.is_empty() || !self.reliable.is_empty()
    }

    /// Whether an acknowledgement is pending.
    #[must_use]
    pub fn acknowledgement_pending(&self) -> bool {
        self.ack_pending
    }

    /// Whether the channel wants a transmit tick.
    #[must_use]
    pub fn should_update(&self, now_milliseconds: u64) -> bool {
        !self.queued.is_empty()
            || self.ack_pending
            || self.sending.is_some()
            || now_milliseconds.wrapping_sub(self.last_sent_milliseconds) > 1000
    }

    /// Whether a new reliable message may start.
    #[must_use]
    pub fn can_reliable(&self) -> bool {
        self.reliable.is_empty()
    }

    /// Last transmit time.
    #[must_use]
    pub fn last_sent_milliseconds(&self) -> u64 {
        self.last_sent_milliseconds
    }

    /// Last receive time.
    #[must_use]
    pub fn last_received_milliseconds(&self) -> u64 {
        self.last_received_milliseconds
    }

    /// Writable payload limit.
    #[must_use]
    pub fn payload_bytes(&self) -> usize {
        self.payload_bytes
    }

    /// Reliable message capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Queue reliable bytes.
    pub fn queue_reliable(&mut self, bytes: &[u8]) -> Result<(), Q2NetError> {
        if self.queued.len() + bytes.len() > self.capacity {
            return Err(Q2NetError::Range("Q2 reliable message overflow"));
        }
        self.queued.extend_from_slice(bytes);
        Ok(())
    }

    fn recording(&self) -> SequenceRecording {
        self.options.sequence_recording.unwrap_or_else(|| {
            if q2_version(self.options.protocol) == 34 {
                SequenceRecording::Id
            } else {
                SequenceRecording::Q2Pro
            }
        })
    }

    fn qport_bytes(&self) -> usize {
        if self.options.channel == ChannelKind::Old && q2_version(self.options.protocol) < 35 {
            2
        } else if self.options.qport == 0 {
            0
        } else {
            1
        }
    }

    fn header(&self, reliable: bool, fragmented: bool) -> Result<MsgWriter, Q2NetError> {
        let classic_id = self.options.channel == ChannelKind::Old && self.recording() == SequenceRecording::Id;
        let size = (if classic_id { 1400 } else { 4096 }).min(self.options.max_datagram_bytes.unwrap_or(4096));
        let mut message = MsgWriter::new(size, false);
        let mask = if self.options.channel == ChannelKind::Old {
            0x7fff_ffff
        } else {
            0x3fff_ffff
        };
        let mut sequence = self.outgoing & mask;
        if reliable {
            sequence |= 0x8000_0000;
        }
        if fragmented {
            sequence |= 0x4000_0000;
        }
        message.write_long(sequence as i32)?;
        let mut ack = self.incoming & mask;
        if self.incoming_reliable != 0 {
            ack |= 0x8000_0000;
        }
        message.write_long(ack as i32)?;
        if self.options.side == ChannelSide::Client {
            if self.qport_bytes() == 2 {
                message.write_short(self.options.qport as i16)?;
            } else if self.qport_bytes() == 1 {
                message.write_byte(self.options.qport as u8)?;
            }
        }
        Ok(message)
    }

    /// Emit the next fragment of a fragmented send, if any.
    pub fn next_fragment(&mut self, now_milliseconds: u64) -> Result<Option<Vec<u8>>, Q2NetError> {
        let (reliable, offset, len) = match &self.sending {
            None => return Ok(None),
            Some(pending) => (pending.reliable, pending.offset, pending.bytes.len()),
        };
        let mut packet = self.header(reliable, true)?;
        let writable = self
            .payload_bytes
            .min(packet.maxsize().saturating_sub(packet.cursize() + 2));
        let end = (offset + writable).min(len);
        let more = end < len;
        packet.write_short((offset | if more { 0x8000 } else { 0 }) as i16)?;
        let bytes = self.sending.as_ref().map(|pending| pending.bytes[offset..end].to_vec());
        if let Some(chunk) = bytes {
            packet.write_bytes(&chunk)?;
        }
        if more {
            if let Some(pending) = self.sending.as_mut() {
                pending.offset = end;
            }
        } else {
            self.sending = None;
            self.outgoing = self.outgoing.wrapping_add(1);
            self.last_sent_milliseconds = now_milliseconds;
        }
        Ok(Some(packet.bytes().to_vec()))
    }

    /// Transmit unreliable bytes, flushing reliable and fragmented state first.
    pub fn transmit(&mut self, unreliable: &[u8], now_milliseconds: u64) -> Result<Vec<u8>, Q2NetError> {
        if self.options.protocol == ProtocolIdentity::Q2Kex {
            return self.kex_packet(unreliable, false, now_milliseconds);
        }
        if let Some(next) = self.next_fragment(now_milliseconds)? {
            return Ok(next);
        }
        let mut send_reliable =
            self.incoming_ack > self.last_reliable && self.incoming_reliable_ack != self.reliable_bit;
        if self.reliable.is_empty() && !self.queued.is_empty() {
            let wrapped = if self.options.side == ChannelSide::Server && self.options.compress {
                try_wrap_zpacket(&self.queued, self.capacity)
            } else {
                None
            };
            let compressed = wrapped.is_some();
            let mut reliable = wrapped.unwrap_or_else(|| self.queued.clone());
            if compressed
                && matches!(
                    self.options.protocol,
                    ProtocolIdentity::Q2Rerelease | ProtocolIdentity::Q2PrivateClassic
                )
            {
                reliable[0] = 34;
            }
            self.reliable = reliable;
            self.queued.clear();
            self.reliable_bit ^= 1;
            send_reliable = true;
        }
        let reliable = if send_reliable {
            self.reliable.clone()
        } else {
            Vec::new()
        };
        if self.options.channel == ChannelKind::New && reliable.len() + unreliable.len() > self.payload_bytes {
            let include = reliable.len() + unreliable.len() <= self.receiving.len();
            let mut bytes = Vec::with_capacity(reliable.len() + if include { unreliable.len() } else { 0 });
            bytes.extend_from_slice(&reliable);
            if include {
                bytes.extend_from_slice(unreliable);
            }
            self.sending = Some(Q2Sending {
                bytes,
                reliable: send_reliable,
                offset: 0,
            });
            if send_reliable {
                self.last_reliable = self.outgoing;
            }
            return self
                .next_fragment(now_milliseconds)?
                .ok_or(Q2NetError::Protocol("Q2 fragment disappeared"));
        }
        let mut packet = self.header(send_reliable, false)?;
        packet.write_bytes(&reliable)?;
        if packet.maxsize().saturating_sub(packet.cursize()) >= unreliable.len() {
            packet.write_bytes(unreliable)?;
        }
        if send_reliable {
            self.last_reliable = self
                .outgoing
                .wrapping_add(u32::from(self.recording() == SequenceRecording::Id));
        }
        self.outgoing = self.outgoing.wrapping_add(1);
        self.ack_pending = false;
        self.last_sent_milliseconds = now_milliseconds;
        Ok(packet.bytes().to_vec())
    }

    /// Receive a packet.
    pub fn receive(&mut self, bytes: &[u8], now_milliseconds: u64) -> Result<Q2ChannelReceive, Q2NetError> {
        if self.options.protocol == ProtocolIdentity::Q2Kex {
            if bytes.len() < 8 {
                return Ok(Q2ChannelReceive::Rejected {
                    reason: Q2RejectReason::Short,
                });
            }
            let sequence_word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            let ack_word = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
            if sequence_word == 0x8000_0000 && ack_word == 0x8000_0000 {
                self.last_received_milliseconds = now_milliseconds;
                return Ok(Q2ChannelReceive::Message {
                    sequence: self.incoming,
                    acknowledged: self.incoming_ack,
                    dropped: 0,
                    bytes: bytes[8..].to_vec(),
                });
            }
            let sequence = sequence_word & 0x7fff_ffff;
            let acknowledged = ack_word & 0x7fff_ffff;
            if sequence <= self.incoming {
                return Ok(Q2ChannelReceive::Rejected {
                    reason: Q2RejectReason::Sequence,
                });
            }
            let dropped = sequence - self.incoming - 1;
            self.incoming = sequence;
            self.incoming_ack = acknowledged;
            self.last_received_milliseconds = now_milliseconds;
            return Ok(Q2ChannelReceive::Message {
                sequence,
                acknowledged,
                dropped,
                bytes: bytes[8..].to_vec(),
            });
        }
        let mut packet = MsgReader::new(bytes);
        let sequence_word = packet.long()? as u32;
        let ack_word = packet.long()? as u32;
        if self.options.side == ChannelSide::Server {
            let width = self.qport_bytes();
            let qport = if width == 2 {
                u32::from(packet.word()?)
            } else if width == 1 {
                u32::from(packet.byte()?)
            } else {
                0
            };
            let expected = if width == 2 {
                self.options.qport
            } else {
                self.options.qport & 255
            };
            if packet.finish().is_err() {
                return Ok(Q2ChannelReceive::Rejected {
                    reason: Q2RejectReason::Short,
                });
            }
            if qport != expected {
                return Ok(Q2ChannelReceive::Rejected {
                    reason: Q2RejectReason::Qport,
                });
            }
        }
        let mask = if self.options.channel == ChannelKind::Old {
            0x7fff_ffff
        } else {
            0x3fff_ffff
        };
        let sequence = sequence_word & mask;
        let acknowledged = ack_word & mask;
        let reliable = sequence_word >> 31;
        let reliable_ack = ack_word >> 31;
        let fragmented = self.options.channel == ChannelKind::New && (sequence_word & 0x4000_0000) != 0;
        let fragment_word = if fragmented { u32::from(packet.word()?) } else { 0 };
        if packet.finish().is_err() {
            return Ok(Q2ChannelReceive::Rejected {
                reason: Q2RejectReason::Short,
            });
        }
        if sequence <= self.incoming {
            return Ok(Q2ChannelReceive::Rejected {
                reason: Q2RejectReason::Sequence,
            });
        }
        self.incoming_reliable_ack = reliable_ack;
        if reliable_ack == self.reliable_bit {
            self.reliable.clear();
        }
        let mut payload = bytes[packet.offset()..].to_vec();
        if fragmented {
            if sequence != self.receive_sequence {
                self.receive_sequence = sequence;
                self.receive_length = 0;
            }
            let offset = (fragment_word & 0x7fff) as usize;
            if offset != self.receive_length {
                return Ok(Q2ChannelReceive::Rejected {
                    reason: Q2RejectReason::FragmentOrder,
                });
            }
            if self.receive_length + payload.len() > self.receiving.len() {
                return Ok(Q2ChannelReceive::Rejected {
                    reason: Q2RejectReason::FragmentSize,
                });
            }
            self.receiving[self.receive_length..self.receive_length + payload.len()].copy_from_slice(&payload);
            self.receive_length += payload.len();
            if (fragment_word & 0x8000) != 0 {
                return Ok(Q2ChannelReceive::Fragment {
                    sequence,
                    received_bytes: self.receive_length,
                });
            }
            payload = self.receiving[..self.receive_length].to_vec();
            self.receive_length = 0;
        }
        let dropped = sequence - self.incoming - 1;
        self.incoming = sequence;
        self.incoming_ack = acknowledged;
        if reliable != 0 {
            self.ack_pending = true;
            self.incoming_reliable ^= 1;
        }
        self.last_received_milliseconds = now_milliseconds;
        Ok(Q2ChannelReceive::Message {
            sequence,
            acknowledged,
            dropped,
            bytes: payload,
        })
    }

    fn kex_packet(&mut self, payload: &[u8], reliable: bool, now: u64) -> Result<Vec<u8>, Q2NetError> {
        if payload.len() > self.capacity {
            return Err(Q2NetError::Range("KEX game message overflow"));
        }
        let mut bytes = vec![0u8; payload.len() + 8];
        let sequence = if reliable {
            0x8000_0000
        } else {
            let sequence = self.outgoing & 0x7fff_ffff;
            self.outgoing = self.outgoing.wrapping_add(1);
            sequence
        };
        let ack = if reliable {
            0x8000_0000
        } else {
            self.incoming & 0x7fff_ffff
        };
        bytes[0..4].copy_from_slice(&sequence.to_le_bytes());
        bytes[4..8].copy_from_slice(&ack.to_le_bytes());
        bytes[8..].copy_from_slice(payload);
        self.last_sent_milliseconds = now;
        Ok(bytes)
    }

    /// Transmit through a datagram transport.
    pub fn send<T: DatagramTransport<Address = NetworkAddress>>(
        &mut self,
        transport: &T,
        to: &NetworkAddress,
        unreliable: &[u8],
        now_milliseconds: u64,
    ) -> Result<bool, Q2NetError> {
        if self.options.protocol == ProtocolIdentity::Q2Kex {
            let mut sent = true;
            if !self.queued.is_empty() {
                let queued = self.queued.clone();
                let packet = self.kex_packet(&queued, true, now_milliseconds)?;
                sent = transport.send(to, &packet)?;
                if sent {
                    self.queued.clear();
                }
            }
            if !unreliable.is_empty() {
                let packet = self.kex_packet(unreliable, false, now_milliseconds)?;
                sent = transport.send(to, &packet)? && sent;
            }
            return Ok(sent);
        }
        let packet = self.transmit(unreliable, now_milliseconds)?;
        Ok(transport.send(to, &packet)?)
    }
}

// ---------------------------------------------------------------------------
// Handshake
// ---------------------------------------------------------------------------

/// Parsed connectionless message (`Q2ConnectionlessMessage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ConnectionlessMessage {
    /// Command word.
    pub command: String,
    /// Arguments.
    pub arguments: Vec<String>,
    /// Full text after the header.
    pub text: String,
    /// Body after the first newline.
    pub body: String,
}

/// Frame connectionless text (`q2OutOfBand`).
#[must_use]
pub fn q2_out_of_band(text: &str, utf8: bool) -> Vec<u8> {
    let data = if utf8 {
        text.as_bytes().to_vec()
    } else {
        string_to_bytes(text)
    };
    let mut packet = vec![255u8; data.len() + 4];
    packet[4..].copy_from_slice(&data);
    packet
}

/// Parse a connectionless packet (`readQ2OutOfBand`).
#[must_use]
pub fn read_q2_out_of_band(bytes: &[u8], utf8: bool) -> Option<Q2ConnectionlessMessage> {
    if bytes.len() < 4 || i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) != -1 {
        return None;
    }
    let mut text = String::new();
    for byte in &bytes[4..] {
        if *byte == 0 {
            break;
        }
        text.push(char::from(*byte));
    }
    if utf8 {
        let end = bytes[4..]
            .iter()
            .position(|byte| *byte == 0)
            .map_or(bytes.len(), |i| i + 4);
        text = String::from_utf8(bytes[4..end].to_vec()).ok()?;
    }
    let (line, body) = match text.find('\n') {
        Some(index) => (text[..index].to_string(), text[index + 1..].to_string()),
        None => (text.clone(), String::new()),
    };
    let mut cursor = ParseState::new(&line);
    let command = parse_q2_token(&mut cursor, Q2_TOKEN_MAX);
    let mut arguments = Vec::new();
    while cursor.index < cursor.data.len() {
        let before = cursor.index;
        let value = parse_q2_token(&mut cursor, Q2_TOKEN_MAX);
        if cursor.index == before {
            break;
        }
        arguments.push(value);
    }
    Some(Q2ConnectionlessMessage {
        command,
        arguments,
        text,
        body,
    })
}

/// Parse a connection integer, rejecting non-decimal text and overflow.
fn decimal(value: Option<&str>, fallback: Option<i64>) -> Result<i64, Q2NetError> {
    let Some(value) = value else {
        return fallback.ok_or(Q2NetError::Protocol("Invalid Q2 connection integer"));
    };
    if value.is_empty()
        || !value
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_digit() || (i == 0 && (b == b'-' || b == b'+')))
    {
        return Err(Q2NetError::Protocol("Invalid Q2 connection integer"));
    }
    value
        .parse::<i64>()
        .map_err(|_| Q2NetError::Protocol("Q2 connection integer outside range"))
}

/// Select a Q2 identity from a wire version and minor revision (`q2Protocol`).
pub fn q2_protocol(version: u32, minor: u32) -> Result<ProtocolIdentity, Q2NetError> {
    match version {
        34 => Ok(ProtocolIdentity::Q2Classic),
        35 => Ok(ProtocolIdentity::Q2R1q2 {
            revision: if minor <= 1903 {
                1903
            } else if minor == 1904 {
                1904
            } else {
                1905
            },
        }),
        36 => Ok(ProtocolIdentity::Q2Q2pro {
            revision: if minor <= 1016 {
                1015
            } else if minor <= 1025 {
                minor
            } else {
                1026
            },
        }),
        1038 => Ok(ProtocolIdentity::Q2Rerelease),
        4038 => Ok(ProtocolIdentity::Q2PrivateClassic),
        2022 => Ok(ProtocolIdentity::Q2KexDemo),
        2023 => Ok(ProtocolIdentity::Q2Kex),
        _ => Err(Q2NetError::Protocol("Unsupported Q2 wire version")),
    }
}

/// Whether a codec binding exists for a protocol (`q2CodecSupport`).
pub fn q2_codec_support(protocol: ProtocolIdentity) -> Result<(), Q2NetError> {
    if protocol == (ProtocolIdentity::Q2Q2pro { revision: 1016 }) {
        return Err(Q2NetError::Protocol(
            "Q2PRO revision 1016 is reserved by the native protocol",
        ));
    }
    Ok(())
}

/// Negotiate the R1Q2 revision actually spoken (`negotiatedR1Q2Protocol`).
pub fn negotiated_r1q2_protocol(offered_revision: u32, reported: Option<i64>) -> Result<u32, Q2NetError> {
    let reported = reported.unwrap_or(-1);
    if reported != 1903 && reported != 1904 && reported != 1905 {
        return Err(Q2NetError::Protocol("Unsupported R1Q2 server revision"));
    }
    Ok(reported.min(offered_revision as i64) as u32)
}

/// Q2 connect request (`Q2ConnectRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ConnectRequest {
    /// Protocol identity.
    pub protocol: ProtocolIdentity,
    /// Qport.
    pub qport: u32,
    /// Challenge.
    pub challenge: i64,
    /// Userinfo.
    pub userinfo: String,
    /// Payload bytes.
    pub payload_bytes: usize,
    /// Channel generation.
    pub channel: ChannelKind,
    /// Compression negotiated.
    pub compression: bool,
    /// KEX social identities.
    pub social_ids: Option<Vec<String>>,
}

/// Write a connect request (`writeQ2Connect`).
pub fn write_q2_connect(request: &Q2ConnectRequest) -> Result<Vec<u8>, Q2NetError> {
    if request.userinfo.contains(['"', '\r', '\n', '\0']) {
        return Err(Q2NetError::Protocol("Invalid Q2 connect userinfo"));
    }
    let tail = match request.protocol {
        ProtocolIdentity::Q2Classic => String::new(),
        ProtocolIdentity::Q2R1q2 { revision } => {
            format!(" {} {revision}", request.payload_bytes)
        }
        ProtocolIdentity::Q2Q2pro { revision } => format!(
            " {} {} {} {revision}",
            request.payload_bytes,
            u8::from(request.channel == ChannelKind::New),
            u8::from(request.compression),
        ),
        ProtocolIdentity::Q2Rerelease | ProtocolIdentity::Q2PrivateClassic => {
            format!(" {} {}", request.payload_bytes, u8::from(request.compression))
        }
        ProtocolIdentity::Q2Kex => {
            let identities = request.social_ids.clone().unwrap_or_else(|| vec![String::new()]);
            if identities.is_empty()
                || identities.len() > 8
                || identities
                    .iter()
                    .any(|value| value.contains(['"', '\\', '\r', '\n', '\0']))
            {
                return Err(Q2NetError::Protocol("Invalid KEX social identity"));
            }
            let userinfo = q2_kex_client_userinfo(&request.userinfo)?;
            let mut chunks = Vec::new();
            let mut chunk = String::new();
            for character in userinfo.chars() {
                if chunk.len() + character.len_utf8() > 510 {
                    chunks.push(std::mem::take(&mut chunk));
                }
                chunk.push(character);
            }
            if !chunk.is_empty() {
                chunks.push(chunk);
            }
            if chunks.is_empty() {
                chunks.push(String::new());
            }
            let quoted = |values: &[String]| {
                values
                    .iter()
                    .map(|value| format!("\"{value}\""))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            return Ok(q2_out_of_band(
                &format!(
                    "connect 2023 {} {} {}\n",
                    identities.len(),
                    quoted(&identities),
                    quoted(&chunks)
                ),
                true,
            ));
        }
        ProtocolIdentity::Q2KexDemo => {
            return Err(Q2NetError::Protocol(
                "KEX native transport connect dialect is not established by the available engine source",
            ));
        }
        _ => return Err(Q2NetError::Protocol("Not a Q2 connection request")),
    };
    let qport = if request.protocol == ProtocolIdentity::Q2Classic {
        request.qport & 65535
    } else {
        request.qport & 255
    };
    Ok(q2_out_of_band(
        &format!(
            "connect {} {qport} {} \"{}\"{tail}\n",
            q2_version(request.protocol),
            request.challenge,
            request.userinfo,
        ),
        false,
    ))
}

/// Read a connect request (`readQ2Connect`).
pub fn read_q2_connect(message: &Q2ConnectionlessMessage) -> Result<Q2ConnectRequest, Q2NetError> {
    if message.command != "connect" {
        return Err(Q2NetError::Protocol("Not a Q2 connection request"));
    }
    let args = &message.arguments;
    if args.first().is_some_and(|version| version == "2023") {
        let count = decimal(args.get(1).map(String::as_str), None)?;
        if !(1..=8).contains(&count) || args.len() < 3 + count as usize {
            return Err(Q2NetError::Protocol("Invalid KEX connection players"));
        }
        let count = count as usize;
        let social_ids = args[2..2 + count].to_vec();
        let userinfo = args[2 + count..].join("");
        if userinfo.len() > 8192 || userinfo.contains(['"', '\r', '\n', '\0']) {
            return Err(Q2NetError::Protocol("Invalid KEX userinfo"));
        }
        return Ok(Q2ConnectRequest {
            protocol: ProtocolIdentity::Q2Kex,
            qport: 0,
            challenge: 0,
            userinfo,
            social_ids: Some(social_ids),
            payload_bytes: 65527,
            channel: ChannelKind::Old,
            compression: false,
        });
    }
    let version = decimal(args.first().map(String::as_str), None)?;
    let qport = decimal(args.get(1).map(String::as_str), None)?;
    let challenge = decimal(args.get(2).map(String::as_str), None)?;
    let userinfo = args.get(3).cloned().unwrap_or_default();
    if args.get(3).is_none() || userinfo.contains(['"', '\r', '\n', '\0']) {
        return Err(Q2NetError::Protocol("Missing or invalid Q2 userinfo"));
    }
    let payload_bytes = if version == 34 {
        1390
    } else {
        decimal(args.get(4).map(String::as_str), Some(1390))?.clamp(512, 4086) as usize
    };
    let minor = if version == 35 {
        decimal(args.get(5).map(String::as_str), Some(1903))?
    } else if version == 36 {
        decimal(args.get(7).map(String::as_str), Some(1015))?
    } else {
        0
    };
    let protocol = q2_protocol(version as u32, minor as u32)?;
    let mut channel = ChannelKind::Old;
    let mut compression = false;
    if version == 36 {
        channel = if decimal(args.get(5).map(String::as_str), Some(1))? == 1 {
            ChannelKind::New
        } else {
            ChannelKind::Old
        };
        compression = decimal(args.get(6).map(String::as_str), Some(0))? != 0;
    } else if version == 1038 || version == 4038 {
        channel = ChannelKind::New;
        compression = decimal(args.get(5).map(String::as_str), Some(0))? != 0;
    } else if version == 35 {
        compression = true;
    } else if version != 34 {
        return Err(Q2NetError::Protocol("KEX native connect is unbound"));
    }
    Ok(Q2ConnectRequest {
        protocol,
        qport: (if version == 34 { qport & 65535 } else { qport & 255 }) as u32,
        challenge,
        userinfo,
        payload_bytes,
        channel,
        compression,
        social_ids: None,
    })
}

/// Parsed challenge (`Q2Challenge`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2Challenge {
    /// Challenge value.
    pub challenge: i64,
    /// Advertised versions.
    pub versions: Vec<i64>,
}

/// Read a challenge (`readQ2Challenge`).
pub fn read_q2_challenge(message: &Q2ConnectionlessMessage) -> Result<Q2Challenge, Q2NetError> {
    if message.command != "challenge" {
        return Err(Q2NetError::Protocol("Not a Q2 challenge"));
    }
    let challenge = decimal(message.arguments.first().map(String::as_str), None)?;
    let versions = match message.arguments.iter().find(|value| value.starts_with("p=")) {
        None => vec![34],
        Some(offer) => offer[2..]
            .split(',')
            .map(|value| decimal(Some(value), None))
            .collect::<Result<Vec<_>, _>>()?,
    };
    Ok(Q2Challenge { challenge, versions })
}

/// Read a Q2PRO download-server advertisement (`readQ2DownloadServer`).
///
/// Returns the normalized URL text: the donor's `URL` object serialized with
/// a guaranteed trailing slash, without depending on a URL crate.
#[must_use]
pub fn read_q2_download_server(arguments: &[String]) -> Option<String> {
    let advertised = arguments
        .iter()
        .find(|value| value.starts_with("dlserver="))
        .map(|value| value[9..].to_string())?;
    if advertised.is_empty() || advertised.len() >= 512 {
        return None;
    }
    let rest = if advertised
        .get(0..7)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("http://"))
    {
        advertised.get(7..)
    } else if advertised
        .get(0..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
    {
        advertised.get(8..)
    } else {
        None
    }?;
    if rest.is_empty() || rest.contains(['#', '?']) {
        return None;
    }
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    if rest.contains('/') {
        if advertised.ends_with('/') {
            Some(advertised)
        } else {
            Some(format!("{advertised}/"))
        }
    } else {
        Some(format!("{advertised}/"))
    }
}

/// Client handshake state (`Q2ClientHandshakeState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2ClientHandshakeState {
    /// Requesting a challenge.
    Challenging {
        /// Last transmit time.
        last_sent: Option<u64>,
    },
    /// Sending connect requests.
    Connecting {
        /// Connect request.
        request: Q2ConnectRequest,
        /// Last transmit time.
        last_sent: Option<u64>,
    },
    /// Connected.
    Connected {
        /// Connect request.
        request: Q2ConnectRequest,
        /// Download server.
        download_server: Option<String>,
    },
    /// Rejected.
    Rejected {
        /// Reason.
        reason: String,
    },
}

/// Q2 client handshake (`Q2ClientHandshake`).
pub struct Q2ClientHandshake {
    remote: NetworkAddress,
    preferences: Vec<ProtocolIdentity>,
    qport: u32,
    userinfo: Box<dyn FnMut() -> String>,
    payload_bytes: usize,
    retry_milliseconds: u64,
    social_id: Box<dyn FnMut() -> String>,
    state: Q2ClientHandshakeState,
}

impl Q2ClientHandshake {
    /// Build a handshake; KEX starts connecting immediately.
    pub fn new(
        remote: NetworkAddress,
        preferences: Vec<ProtocolIdentity>,
        qport: u32,
        userinfo: impl FnMut() -> String + 'static,
        payload_bytes: usize,
        retry_milliseconds: u64,
        social_id: impl FnMut() -> String + 'static,
    ) -> Result<Self, Q2NetError> {
        if preferences.is_empty() {
            return Err(Q2NetError::Protocol(
                "Q2 handshake needs an explicit protocol preference",
            ));
        }
        let mut handshake = Self {
            remote,
            preferences,
            qport,
            userinfo: Box::new(userinfo),
            payload_bytes,
            retry_milliseconds,
            social_id: Box::new(social_id),
            state: Q2ClientHandshakeState::Challenging { last_sent: None },
        };
        if handshake.preferences[0] == ProtocolIdentity::Q2Kex {
            let userinfo = (handshake.userinfo)();
            let social = (handshake.social_id)();
            handshake.state = Q2ClientHandshakeState::Connecting {
                request: Q2ConnectRequest {
                    protocol: ProtocolIdentity::Q2Kex,
                    qport: 0,
                    challenge: 0,
                    userinfo,
                    social_ids: Some(vec![social]),
                    payload_bytes: 65527,
                    channel: ChannelKind::Old,
                    compression: false,
                },
                last_sent: None,
            };
        }
        Ok(handshake)
    }

    /// Current state.
    #[must_use]
    pub fn state(&self) -> &Q2ClientHandshakeState {
        &self.state
    }

    /// Emit a retry packet when due.
    pub fn poll(&mut self, now: u64) -> Result<Option<Vec<u8>>, Q2NetError> {
        match &self.state {
            Q2ClientHandshakeState::Connected { .. } | Q2ClientHandshakeState::Rejected { .. } => {
                return Ok(None);
            }
            Q2ClientHandshakeState::Challenging { last_sent }
            | Q2ClientHandshakeState::Connecting { last_sent, .. } => {
                if last_sent.is_some_and(|sent| now.wrapping_sub(sent) < self.retry_milliseconds) {
                    return Ok(None);
                }
            }
        }
        if matches!(self.state, Q2ClientHandshakeState::Challenging { .. }) {
            self.state = Q2ClientHandshakeState::Challenging { last_sent: Some(now) };
            return Ok(Some(q2_out_of_band("getchallenge\n", false)));
        }
        let Q2ClientHandshakeState::Connecting { request, .. } = self.state.clone() else {
            return Ok(None);
        };
        let mut request = request.clone();
        request.userinfo = (self.userinfo)();
        if request.protocol == ProtocolIdentity::Q2Kex {
            request.social_ids = Some(vec![(self.social_id)()]);
        }
        let bytes = write_q2_connect(&request)?;
        self.state = Q2ClientHandshakeState::Connecting {
            request,
            last_sent: Some(now),
        };
        Ok(Some(bytes))
    }

    /// Feed a connectionless message; returns whether it was consumed.
    pub fn receive(&mut self, from: &NetworkAddress, message: &Q2ConnectionlessMessage) -> Result<bool, Q2NetError> {
        if !same_address(from, &self.remote, true) {
            return Ok(false);
        }
        if message.command == "challenge" && !matches!(self.state, Q2ClientHandshakeState::Connected { .. }) {
            let challenge = read_q2_challenge(message)?;
            let protocol = self
                .preferences
                .iter()
                .find(|identity| challenge.versions.contains(&(q2_version(**identity) as i64)))
                .copied();
            let Some(protocol) = protocol else {
                self.state = Q2ClientHandshakeState::Rejected {
                    reason: "No compatible advertised Quake II protocol".to_string(),
                };
                return Ok(true);
            };
            if matches!(protocol, ProtocolIdentity::Q2Kex | ProtocolIdentity::Q2KexDemo) {
                self.state = Q2ClientHandshakeState::Rejected {
                    reason: "KEX native live transport connect is unbound".to_string(),
                };
                return Ok(true);
            }
            if let Err(error) = q2_codec_support(protocol) {
                self.state = Q2ClientHandshakeState::Rejected {
                    reason: error.to_string(),
                };
                return Ok(true);
            }
            let version = q2_version(protocol);
            self.state = Q2ClientHandshakeState::Connecting {
                request: Q2ConnectRequest {
                    protocol,
                    qport: if version == 34 {
                        self.qport & 65535
                    } else {
                        self.qport & 255
                    },
                    challenge: challenge.challenge,
                    userinfo: (self.userinfo)(),
                    payload_bytes: self.payload_bytes,
                    channel: if version == 34 || version == 35 {
                        ChannelKind::Old
                    } else {
                        ChannelKind::New
                    },
                    compression: version != 34,
                    social_ids: None,
                },
                last_sent: None,
            };
            return Ok(true);
        }
        if message.command == "client_connect" {
            let connecting = matches!(self.state, Q2ClientHandshakeState::Connecting { .. });
            let kex_ok = !matches!(
                self.state,
                Q2ClientHandshakeState::Connecting {
                    request: Q2ConnectRequest {
                        protocol: ProtocolIdentity::Q2Kex,
                        ..
                    },
                    ..
                }
            ) || message.arguments.first().is_some_and(|arg| arg == "2023");
            if connecting && kex_ok {
                let Q2ClientHandshakeState::Connecting { request, .. } = self.state.clone() else {
                    return Ok(false);
                };
                let download_server = read_q2_download_server(&message.arguments);
                self.state = Q2ClientHandshakeState::Connected {
                    request,
                    download_server,
                };
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Server challenge table (`Q2ChallengeTable`).
pub struct Q2ChallengeTable {
    random: Box<dyn FnMut() -> u32>,
    capacity: usize,
    entries: HashMap<String, (u32, u64)>,
}

impl Q2ChallengeTable {
    /// Build a table with a challenge source.
    pub fn new(random: impl FnMut() -> u32 + 'static, capacity: usize) -> Self {
        Self {
            random: Box::new(random),
            capacity,
            entries: HashMap::new(),
        }
    }

    /// Issue (or recall) a challenge for an address.
    pub fn issue(&mut self, from: &NetworkAddress, now: u64) -> u32 {
        let key = address_key(from, false);
        if let Some((value, _)) = self.entries.get(&key) {
            return *value;
        }
        if self.entries.len() >= self.capacity {
            let mut oldest: Option<String> = None;
            let mut time = u64::MAX;
            for (name, (_, entry_time)) in &self.entries {
                if *entry_time < time {
                    oldest = Some(name.clone());
                    time = *entry_time;
                }
            }
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        let value = (self.random)() & 0x7fff;
        self.entries.insert(key, (value, now));
        value
    }

    /// Validate a challenge; loopback always passes.
    #[must_use]
    pub fn validate(&self, from: &NetworkAddress, challenge: i64) -> bool {
        from.kind() == "loopback"
            || self
                .entries
                .get(&address_key(from, false))
                .is_some_and(|(value, _)| i64::from(*value) == challenge)
    }

    /// Reply with a challenge advertisement.
    pub fn reply(&mut self, from: &NetworkAddress, now: u64, protocols: &[ProtocolIdentity]) -> Vec<u8> {
        let challenge = self.issue(from, now);
        let mut versions = Vec::new();
        for protocol in protocols {
            let version = q2_version(*protocol);
            if !versions.contains(&version) {
                versions.push(version);
            }
        }
        let list = versions.iter().map(ToString::to_string).collect::<Vec<_>>().join(",");
        q2_out_of_band(&format!("challenge {challenge} p={list}"), false)
    }
}

/// Strip unselected seat suffixes from KEX userinfo (`q2KexSeatUserinfo`).
pub fn q2_kex_seat_userinfo(text: &str, seat: u32) -> Result<String, Q2NetError> {
    if text.is_empty() {
        return Ok(String::new());
    }
    let parts: Vec<&str> = text.split('\\').collect();
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut selected: Vec<(String, String)> = Vec::new();
    let mut index = usize::from(text.starts_with('\\'));
    while index < parts.len() {
        let key = parts.get(index).copied().unwrap_or("");
        let value = parts.get(index + 1).copied();
        let Some(value) = value else {
            return Err(Q2NetError::Protocol("Invalid KEX userinfo pairs"));
        };
        match key.rsplit_once('_') {
            Some((base, suffix)) if !base.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) => {
                if suffix.parse::<u32>().is_ok_and(|number| number == seat) {
                    selected.push((base.to_string(), value.to_string()));
                }
            }
            _ => pairs.push((key.to_string(), value.to_string())),
        }
        index += 2;
    }
    for (key, value) in selected {
        if let Some(slot) = pairs.iter_mut().find(|(name, _)| *name == key) {
            slot.1 = value;
        } else {
            pairs.push((key, value));
        }
    }
    Ok(pairs
        .iter()
        .map(|(key, value)| format!("\\{key}\\{value}"))
        .collect::<String>())
}

/// Duplicate bare keys with a `_0` suffix (`q2KexClientUserinfo`).
pub fn q2_kex_client_userinfo(text: &str) -> Result<String, Q2NetError> {
    if text.is_empty() {
        return Ok(String::new());
    }
    let parts: Vec<&str> = text.split('\\').collect();
    let mut keys = HashSet::new();
    let mut index = 1;
    while index < parts.len() {
        if let Some(key) = parts.get(index) {
            keys.insert((*key).to_string());
        }
        index += 2;
    }
    let mut result = text.to_string();
    let mut index = 1;
    while index < parts.len() {
        let key = parts.get(index).copied().unwrap_or("");
        let value = parts.get(index + 1).copied();
        let Some(value) = value else {
            return Err(Q2NetError::Protocol("Invalid KEX client userinfo"));
        };
        let suffixed = key.rsplit_once('_').is_some_and(|(base, suffix)| {
            !base.is_empty() && !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit())
        });
        if !suffixed && !keys.contains(&format!("{key}_0")) {
            result.push_str(&format!("\\{key}_0\\{value}"));
        }
        index += 2;
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Connectionless
// ---------------------------------------------------------------------------

/// Player row for status replies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2StatusPlayer {
    /// Score.
    pub score: i64,
    /// Ping.
    pub ping: i64,
    /// Name.
    pub name: String,
}

/// Server status payload (`Q2Status`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2Status {
    /// Server info string.
    pub server_info: String,
    /// Players.
    pub players: Vec<Q2StatusPlayer>,
}

/// Render status text (`q2StatusText`).
pub fn q2_status_text(status: &Q2Status, maximum_bytes: usize) -> Result<String, Q2NetError> {
    let mut result = format!("{}\n", status.server_info);
    if result.len() >= maximum_bytes {
        return Err(Q2NetError::Range("Q2 serverinfo exceeds status packet capacity"));
    }
    for player in &status.players {
        let line = format!("{} {} \"{}\"\n", player.score, player.ping, player.name);
        if result.len() + line.len() >= maximum_bytes {
            break;
        }
        result.push_str(&line);
    }
    Ok(result)
}

/// Read a status reply (`readQ2Status`).
#[must_use]
pub fn read_q2_status(message: &Q2ConnectionlessMessage, protocol: ProtocolIdentity) -> Option<ServerStatus> {
    if message.command != "print" {
        return None;
    }
    let mut lines = message.body.split('\n');
    let info = lines.next()?;
    if !info.starts_with('\\') {
        return None;
    }
    let fields: Vec<&str> = info[1..].split('\\').collect();
    let mut rules = BTreeMap::new();
    let mut index = 0;
    while index + 1 < fields.len() {
        rules.insert(fields[index].to_string(), fields[index + 1].to_string());
        index += 2;
    }
    let mut players = Vec::new();
    for line in lines {
        let mut parts = line.splitn(3, ' ');
        let (Some(score), Some(ping), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        // Mirrors /^(-?\d+) (-?\d+) "(.*)"$/: strict decimal pair plus quotes.
        let decimal_int = |text: &str| {
            let digits = text.strip_prefix('-').unwrap_or(text);
            (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                .then(|| text.parse::<i64>().ok())
                .flatten()
        };
        let (Some(score), Some(ping)) = (decimal_int(score), decimal_int(ping)) else {
            continue;
        };
        if name.len() >= 2 && name.starts_with('"') && name.ends_with('"') {
            players.push(PlayerDetail {
                name: name[1..name.len() - 1].to_string(),
                score,
                ping,
            });
        }
    }
    let max_players = rules
        .get("maxclients")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
    Some(ServerStatus {
        name: rules.get("hostname").cloned().unwrap_or_default(),
        map: rules.get("mapname").cloned().unwrap_or_default(),
        players: players.len() as i64,
        max_players,
        rules,
        player_details: players,
        wire: crate::common::session::WireSelection::Source { protocol },
    })
}

/// Q2 discovery wire (`q2DiscoveryWire`).
pub struct Q2DiscoveryWire {
    protocol: ProtocolIdentity,
    status: Box<dyn Fn() -> Q2Status>,
}

impl Q2DiscoveryWire {
    /// Build a discovery wire for a protocol.
    pub fn new(protocol: ProtocolIdentity, status: impl Fn() -> Q2Status + 'static) -> Self {
        Self {
            protocol,
            status: Box::new(status),
        }
    }
}

impl DiscoveryWire for Q2DiscoveryWire {
    fn query(&self, kind: DiscoveryRequestKind, _challenge: &str) -> Result<Vec<u8>, DiscoveryError> {
        Ok(if kind == DiscoveryRequestKind::Info {
            q2_out_of_band(&format!("info {}", q2_version(self.protocol)), false)
        } else {
            q2_out_of_band("status", false)
        })
    }

    fn master_query(&self) -> Result<Vec<u8>, DiscoveryError> {
        Ok(string_to_bytes("query"))
    }

    fn heartbeat(&self, active: bool) -> Result<Vec<u8>, DiscoveryError> {
        if !active {
            return Ok(q2_out_of_band("shutdown", false));
        }
        let status = (self.status)();
        q2_status_text(&status, 1384)
            .map(|text| q2_out_of_band(&format!("heartbeat\n{text}"), false))
            .map_err(|error| DiscoveryError::Wire(error.to_string()))
    }
}

/// Parse a master-server reply (`readQ2MasterReply`).
pub fn read_q2_master_reply(bytes: &[u8]) -> Result<Option<Vec<NetworkAddress>>, Q2NetError> {
    let mut start = 0;
    if bytes.len() >= 4 && bytes[0..4].iter().all(|value| *value == 255) {
        start = 4;
    }
    let prefix = bytes.get(start..start + 8).unwrap_or_default();
    if prefix.len() != 8 || prefix[0..7] != *b"servers" || (prefix[7] != b' ' && prefix[7] != b'\n') {
        return Ok(None);
    }
    start += 8;
    if !(bytes.len() - start).is_multiple_of(6) {
        return Err(Q2NetError::Protocol("Partial Q2 master address"));
    }
    let mut found: HashMap<String, NetworkAddress> = HashMap::new();
    let mut offset = start;
    while offset < bytes.len() {
        let port = (u32::from(bytes[offset + 4]) << 8) | u32::from(bytes[offset + 5]);
        if port != 0 {
            let address = ipv4_address(
                [bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]],
                port,
                false,
            )?;
            found.insert(address_key(&address, true), address);
        }
        offset += 6;
    }
    Ok(Some(found.into_values().collect()))
}

/// Connect admission verdict (`Q2ConnectAdmission`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2ConnectAdmission {
    /// Accepted, with optional extra `client_connect` arguments.
    Accepted {
        /// Extra arguments.
        response_arguments: Option<String>,
    },
    /// Rejected with a reason.
    Rejected {
        /// Reason.
        reason: String,
    },
}

/// Server profile for rcon handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2ServerProfile {
    /// Classic behavior.
    Classic,
    /// Rerelease behavior.
    Rerelease,
}

/// Limited rcon credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2LimitedRcon {
    /// Password.
    pub password: String,
    /// Allowed command prefixes.
    pub prefixes: Vec<String>,
}

/// Connectionless host (`Q2ConnectionlessHost`).
///
/// The donor's asynchronous rcon execution becomes a synchronous callback;
/// callers needing background execution must drive it themselves.
pub trait Q2ConnectionlessHost {
    /// Server profile.
    fn profile(&self) -> Q2ServerProfile;
    /// Advertised protocols.
    fn protocols(&self) -> &[ProtocolIdentity];
    /// Current status.
    fn status(&self) -> Q2Status;
    /// Info row.
    fn info(&self) -> Q2Info;
    /// Admit a connect request.
    fn connect(&mut self, from: &NetworkAddress, request: &Q2ConnectRequest) -> Q2ConnectAdmission;
    /// Reply to an address.
    fn reply(&mut self, to: &NetworkAddress, bytes: Vec<u8>);
    /// Full rcon password.
    fn rcon_password(&self) -> String;
    /// Limited rcon credential.
    fn limited_rcon(&self) -> Option<Q2LimitedRcon>;
    /// Rerelease rate limiter check.
    fn rcon_rate_allowed(&self, now: u64) -> bool;
    /// Recharge the rerelease rate limiter.
    fn recharge_rcon_rate(&mut self);
    /// Execute an rcon command, streaming output through the sink.
    fn execute_rcon(&mut self, command: &str, limited: bool, output: &mut dyn FnMut(&str));
}

/// Info row for `info` replies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2Info {
    /// Server name.
    pub name: String,
    /// Map name.
    pub map: String,
    /// Player count.
    pub players: i64,
    /// Maximum players.
    pub max_players: i64,
}

/// Render an `info` reply (`q2InfoText`).
#[must_use]
pub fn q2_info_text(info: &Q2Info, protocols: &[ProtocolIdentity], version: i64) -> Option<String> {
    if info.max_players == 1 {
        return None;
    }
    if !protocols.iter().any(|protocol| q2_version(*protocol) as i64 == version) {
        return Some(format!("info\n{}: wrong version\n", info.name));
    }
    Some(format!(
        "info\n{:>16} {:>8} {:>2}/{:>2}\n",
        info.name, info.map, info.players, info.max_players
    ))
}

/// Connectionless server (`Q2ConnectionlessServer`).
pub struct Q2ConnectionlessServer<H> {
    host: H,
    challenges: Q2ChallengeTable,
}

impl<H: Q2ConnectionlessHost> Q2ConnectionlessServer<H> {
    /// Build a server over a host and challenge table.
    pub fn new(host: H, challenges: Q2ChallengeTable) -> Self {
        Self { host, challenges }
    }

    /// Borrow the host.
    #[must_use]
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Mutably borrow the host.
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    /// Handle one packet; returns whether it was consumed.
    pub fn receive(&mut self, from: &NetworkAddress, bytes: &[u8], now: u64) -> Result<bool, Q2NetError> {
        let Some(message) = read_q2_out_of_band(bytes, false) else {
            return Ok(false);
        };
        match message.command.as_str() {
            "ping" => {
                let reply = q2_out_of_band("ack", false);
                self.host.reply(from, reply);
                Ok(true)
            }
            "status" => {
                let status = self.host.status();
                let reply = q2_out_of_band(&format!("print\n{}", q2_status_text(&status, 1384)?), false);
                self.host.reply(from, reply);
                Ok(true)
            }
            "info" => {
                let info = self.host.info();
                let version = message
                    .arguments
                    .first()
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or(-1);
                if let Some(text) = q2_info_text(&info, self.host.protocols(), version) {
                    let reply = q2_out_of_band(&text, false);
                    self.host.reply(from, reply);
                }
                Ok(true)
            }
            "getchallenge" => {
                let protocols = self.host.protocols().to_vec();
                let reply = self.challenges.reply(from, now, &protocols);
                self.host.reply(from, reply);
                Ok(true)
            }
            "connect" => {
                let request = read_q2_connect(&message)?;
                if !self
                    .host
                    .protocols()
                    .iter()
                    .any(|protocol| q2_version(*protocol) == q2_version(request.protocol))
                {
                    let reply = q2_out_of_band("print\nUnsupported protocol.\n", false);
                    self.host.reply(from, reply);
                    return Ok(true);
                }
                if !self.challenges.validate(from, request.challenge) {
                    let reply = q2_out_of_band("print\nBad challenge.\n", false);
                    self.host.reply(from, reply);
                    return Ok(true);
                }
                let admission = self.host.connect(from, &request);
                let text = match admission {
                    Q2ConnectAdmission::Accepted { response_arguments } => match response_arguments {
                        None => "client_connect".to_string(),
                        Some(extra) => format!("client_connect {extra}"),
                    },
                    Q2ConnectAdmission::Rejected { reason } => format!("print\n{reason}\n"),
                };
                let reply = q2_out_of_band(&text, false);
                self.host.reply(from, reply);
                Ok(true)
            }
            "rcon" => {
                handle_q2_rcon_host(&mut self.host, from, &message, now)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

/// Handle an rcon request (`handleQ2Rcon`).
pub fn handle_q2_rcon_host<H: Q2ConnectionlessHost>(
    host: &mut H,
    from: &NetworkAddress,
    message: &Q2ConnectionlessMessage,
    now: u64,
) -> Result<(), Q2NetError> {
    if host.profile() == Q2ServerProfile::Rerelease && !host.rcon_rate_allowed(now) {
        return Ok(());
    }
    let password = message.arguments.first().cloned().unwrap_or_default();
    let full = host.rcon_password();
    let limit = host.limited_rcon();
    let full_match = !full.is_empty() && password == full;
    let limited = !full_match
        && host.profile() == Q2ServerProfile::Rerelease
        && limit
            .as_ref()
            .is_some_and(|limit| !limit.password.is_empty() && password == limit.password);
    if !full_match && !limited {
        let reply = q2_out_of_band("print\nBad rcon_password.\n", false);
        host.reply(from, reply);
        return Ok(());
    }
    if host.profile() == Q2ServerProfile::Rerelease {
        host.recharge_rcon_rate();
    }
    let mut cursor = ParseState::new(&message.text);
    parse_q2_token(&mut cursor, Q2_TOKEN_MAX);
    parse_q2_token(&mut cursor, Q2_TOKEN_MAX);
    let command = if host.profile() == Q2ServerProfile::Classic {
        format!("{} ", message.arguments[1..].join(" "))
    } else {
        String::from_utf16_lossy(&cursor.data[cursor.index.min(cursor.data.len())..])
            .trim_start()
            .to_string()
    };
    if limited {
        let limit = limit.unwrap_or(Q2LimitedRcon {
            password: String::new(),
            prefixes: Vec::new(),
        });
        if !limit.prefixes.iter().any(|prefix| command.starts_with(prefix)) {
            let reply = q2_out_of_band("print\nThis command is not permitted.\n", false);
            host.reply(from, reply);
            return Ok(());
        }
    }
    let mut chunks = Vec::new();
    host.execute_rcon(&command, limited, &mut |text| {
        chunks.push(text.to_string());
    });
    let mut buffered = String::new();
    for text in &chunks {
        for character in text.chars() {
            if character as u32 > 255 {
                // The donor throws mid-stream but still flushes buffered output.
                if !buffered.is_empty() {
                    let reply = q2_out_of_band(&format!("print\n{buffered}"), false);
                    host.reply(from, reply);
                }
                return Err(Q2NetError::Range("Q2 rcon output requires byte characters"));
            }
            if buffered.len() == 1383 {
                let reply = q2_out_of_band(&format!("print\n{buffered}"), false);
                host.reply(from, reply);
                buffered.clear();
            }
            buffered.push(character);
        }
    }
    if !buffered.is_empty() {
        let reply = q2_out_of_band(&format!("print\n{buffered}"), false);
        host.reply(from, reply);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Temp entities
// ---------------------------------------------------------------------------

/// Quake II temp entity type (`Q2TempType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Q2TempType {
    /// Gunshot.
    Gunshot = 0,
    /// Blood.
    Blood = 1,
    /// Blaster.
    Blaster = 2,
    /// Rail trail.
    Railtrail = 3,
    /// Shotgun.
    Shotgun = 4,
    /// Explosion 1.
    Explosion1 = 5,
    /// Explosion 2.
    Explosion2 = 6,
    /// Rocket explosion.
    RocketExplosion = 7,
    /// Grenade explosion.
    GrenadeExplosion = 8,
    /// Sparks.
    Sparks = 9,
    /// Splash.
    Splash = 10,
    /// Bubble trail.
    Bubbletrail = 11,
    /// Screen sparks.
    ScreenSparks = 12,
    /// Shield sparks.
    ShieldSparks = 13,
    /// Bullet sparks.
    BulletSparks = 14,
    /// Laser sparks.
    LaserSparks = 15,
    /// Parasite attack.
    ParasiteAttack = 16,
    /// Rocket explosion water.
    RocketExplosionWater = 17,
    /// Grenade explosion water.
    GrenadeExplosionWater = 18,
    /// Medic cable attack.
    MedicCableAttack = 19,
    /// BFG explosion.
    BfgExplosion = 20,
    /// BFG big explosion.
    BfgBigexplosion = 21,
    /// Boss teleport.
    Bosstport = 22,
    /// BFG laser.
    BfgLaser = 23,
    /// Grapple cable.
    GrappleCable = 24,
    /// Welding sparks.
    WeldingSparks = 25,
    /// Green blood.
    Greenblood = 26,
    /// Blue hyperblaster.
    Bluehyperblaster = 27,
    /// Plasma explosion.
    PlasmaExplosion = 28,
    /// Tunnel sparks.
    TunnelSparks = 29,
    /// Blaster 2.
    Blaster2 = 30,
    /// Rail trail 2.
    Railtrail2 = 31,
    /// Flame.
    Flame = 32,
    /// Lightning.
    Lightning = 33,
    /// Debug trail.
    Debugtrail = 34,
    /// Plain explosion.
    PlainExplosion = 35,
    /// Flashlight.
    Flashlight = 36,
    /// Force wall.
    Forcewall = 37,
    /// Heat beam.
    Heatbeam = 38,
    /// Monster heat beam.
    MonsterHeatbeam = 39,
    /// Steam.
    Steam = 40,
    /// Bubble trail 2.
    Bubbletrail2 = 41,
    /// More blood.
    Moreblood = 42,
    /// Heat beam sparks.
    HeatbeamSparks = 43,
    /// Heat beam steam.
    HeatbeamSteam = 44,
    /// Chainfist smoke.
    ChainfistSmoke = 45,
    /// Electric sparks.
    ElectricSparks = 46,
    /// Tracker explosion.
    TrackerExplosion = 47,
    /// Teleport effect.
    TeleportEffect = 48,
    /// Dball goal.
    DballGoal = 49,
    /// Widow beam out.
    Widowbeamout = 50,
    /// Nuke blast.
    Nukeblast = 51,
    /// Widow splash.
    Widowsplash = 52,
    /// Explosion 1 big.
    Explosion1Big = 53,
    /// Explosion 1 NP.
    Explosion1Np = 54,
    /// Flechette.
    Flechette = 55,
    /// Blue hyperblaster 2.
    Bluehyperblaster2 = 56,
    /// BFG zap.
    BfgZap = 57,
    /// Berserk slam.
    BerserkSlam = 58,
    /// Grapple cable 2.
    GrappleCable2 = 59,
    /// Power splash.
    PowerSplash = 60,
    /// Lightning beam.
    LightningBeam = 61,
    /// Explosion 1 NL.
    Explosion1Nl = 62,
    /// Explosion 2 NL.
    Explosion2Nl = 63,
    /// Q2Pro damage dealt.
    Q2proDamageDealt = 128,
    /// Entity count sentinel.
    NumEntities = 129,
}

impl Q2TempType {
    /// Decode a type value.
    #[must_use]
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Gunshot),
            1 => Some(Self::Blood),
            2 => Some(Self::Blaster),
            3 => Some(Self::Railtrail),
            4 => Some(Self::Shotgun),
            5 => Some(Self::Explosion1),
            6 => Some(Self::Explosion2),
            7 => Some(Self::RocketExplosion),
            8 => Some(Self::GrenadeExplosion),
            9 => Some(Self::Sparks),
            10 => Some(Self::Splash),
            11 => Some(Self::Bubbletrail),
            12 => Some(Self::ScreenSparks),
            13 => Some(Self::ShieldSparks),
            14 => Some(Self::BulletSparks),
            15 => Some(Self::LaserSparks),
            16 => Some(Self::ParasiteAttack),
            17 => Some(Self::RocketExplosionWater),
            18 => Some(Self::GrenadeExplosionWater),
            19 => Some(Self::MedicCableAttack),
            20 => Some(Self::BfgExplosion),
            21 => Some(Self::BfgBigexplosion),
            22 => Some(Self::Bosstport),
            23 => Some(Self::BfgLaser),
            24 => Some(Self::GrappleCable),
            25 => Some(Self::WeldingSparks),
            26 => Some(Self::Greenblood),
            27 => Some(Self::Bluehyperblaster),
            28 => Some(Self::PlasmaExplosion),
            29 => Some(Self::TunnelSparks),
            30 => Some(Self::Blaster2),
            31 => Some(Self::Railtrail2),
            32 => Some(Self::Flame),
            33 => Some(Self::Lightning),
            34 => Some(Self::Debugtrail),
            35 => Some(Self::PlainExplosion),
            36 => Some(Self::Flashlight),
            37 => Some(Self::Forcewall),
            38 => Some(Self::Heatbeam),
            39 => Some(Self::MonsterHeatbeam),
            40 => Some(Self::Steam),
            41 => Some(Self::Bubbletrail2),
            42 => Some(Self::Moreblood),
            43 => Some(Self::HeatbeamSparks),
            44 => Some(Self::HeatbeamSteam),
            45 => Some(Self::ChainfistSmoke),
            46 => Some(Self::ElectricSparks),
            47 => Some(Self::TrackerExplosion),
            48 => Some(Self::TeleportEffect),
            49 => Some(Self::DballGoal),
            50 => Some(Self::Widowbeamout),
            51 => Some(Self::Nukeblast),
            52 => Some(Self::Widowsplash),
            53 => Some(Self::Explosion1Big),
            54 => Some(Self::Explosion1Np),
            55 => Some(Self::Flechette),
            56 => Some(Self::Bluehyperblaster2),
            57 => Some(Self::BfgZap),
            58 => Some(Self::BerserkSlam),
            59 => Some(Self::GrappleCable2),
            60 => Some(Self::PowerSplash),
            61 => Some(Self::LightningBeam),
            62 => Some(Self::Explosion1Nl),
            63 => Some(Self::Explosion2Nl),
            128 => Some(Self::Q2proDamageDealt),
            129 => Some(Self::NumEntities),
            _ => None,
        }
    }
}

/// Temp entity integer field name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2TempInt {
    /// First entity.
    Entity1,
    /// Second entity.
    Entity2,
    /// Count.
    Count,
    /// Color.
    Color,
    /// Time.
    Time,
}

/// Temp entity vector field name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2TempVec {
    /// First position.
    Position1,
    /// Second position.
    Position2,
    /// Direction.
    Direction,
    /// Offset.
    Offset,
}

/// Temp entity field (`Q2TempField`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2TempField {
    /// Integer field.
    Integer {
        /// Name.
        name: Q2TempInt,
        /// Value.
        value: i32,
    },
    /// Vector field.
    Vector {
        /// Name.
        name: Q2TempVec,
        /// Value.
        value: [f64; 3],
    },
}

/// Temp entity (`Q2TempEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TempEntity {
    /// Type value.
    pub temp_type: u8,
    /// Fields.
    pub fields: Vec<Q2TempField>,
    /// Raw bytes.
    pub raw: Vec<u8>,
}

/// Read a game position (`readGamePosition`).
pub fn read_game_position(reader: &mut MsgReader<'_>, floating: bool, int23: bool) -> Result<[f64; 3], Q2NetError> {
    if int23 {
        Ok([
            f64::from(read_q2pro_int23(reader, 0)?) / 8.0,
            f64::from(read_q2pro_int23(reader, 0)?) / 8.0,
            f64::from(read_q2pro_int23(reader, 0)?) / 8.0,
        ])
    } else if floating {
        Ok([
            f64::from(reader.float()?),
            f64::from(reader.float()?),
            f64::from(reader.float()?),
        ])
    } else {
        Ok([reader.coord()?, reader.coord()?, reader.coord()?])
    }
}

/// Read a temp entity (`readTempEntity`).
pub fn read_temp_entity(
    reader: &mut MsgReader<'_>,
    floating: bool,
    q2pro_extended: bool,
    int23: bool,
) -> Result<Q2TempEntity, Q2NetError> {
    use Q2TempType as T;
    let start = reader.offset();
    let temp_type = reader.byte()?;
    let mut fields = Vec::new();
    macro_rules! integer {
        ($name:expr, $value:expr) => {{
            let value = $value;
            fields.push(Q2TempField::Integer { name: $name, value });
            value
        }};
    }
    macro_rules! position {
        ($name:expr) => {
            fields.push(Q2TempField::Vector {
                name: $name,
                value: read_game_position(reader, floating, int23)?,
            });
        };
    }
    match temp_type {
        x if matches!(
            T::from_u8(x),
            Some(
                T::Blood
                    | T::Gunshot
                    | T::Sparks
                    | T::BulletSparks
                    | T::ScreenSparks
                    | T::ShieldSparks
                    | T::Shotgun
                    | T::Blaster
                    | T::Greenblood
                    | T::Blaster2
                    | T::Flechette
                    | T::HeatbeamSparks
                    | T::HeatbeamSteam
                    | T::Moreblood
                    | T::ElectricSparks
                    | T::Bluehyperblaster2
                    | T::BerserkSlam
            )
        ) =>
        {
            position!(Q2TempVec::Position1);
            fields.push(Q2TempField::Vector {
                name: Q2TempVec::Direction,
                value: read_dir(reader)?,
            });
        }
        x if matches!(
            T::from_u8(x),
            Some(T::Splash | T::LaserSparks | T::WeldingSparks | T::TunnelSparks)
        ) =>
        {
            integer!(Q2TempInt::Count, i32::from(reader.byte()?));
            position!(Q2TempVec::Position1);
            fields.push(Q2TempField::Vector {
                name: Q2TempVec::Direction,
                value: read_dir(reader)?,
            });
            integer!(Q2TempInt::Color, i32::from(reader.byte()?));
        }
        x if matches!(
            T::from_u8(x),
            Some(
                T::Bluehyperblaster
                    | T::Railtrail
                    | T::Railtrail2
                    | T::Bubbletrail
                    | T::Debugtrail
                    | T::Bubbletrail2
                    | T::BfgLaser
                    | T::BfgZap
            )
        ) =>
        {
            position!(Q2TempVec::Position1);
            position!(Q2TempVec::Position2);
        }
        x if matches!(
            T::from_u8(x),
            Some(
                T::GrenadeExplosion
                    | T::GrenadeExplosionWater
                    | T::Explosion2
                    | T::PlasmaExplosion
                    | T::RocketExplosion
                    | T::RocketExplosionWater
                    | T::Explosion1
                    | T::Explosion1Np
                    | T::Explosion1Big
                    | T::BfgExplosion
                    | T::BfgBigexplosion
                    | T::Bosstport
                    | T::PlainExplosion
                    | T::ChainfistSmoke
                    | T::TrackerExplosion
                    | T::TeleportEffect
                    | T::DballGoal
                    | T::Widowsplash
                    | T::Nukeblast
                    | T::Explosion1Nl
                    | T::Explosion2Nl
            )
        ) =>
        {
            position!(Q2TempVec::Position1);
        }
        x if matches!(
            T::from_u8(x),
            Some(
                T::ParasiteAttack
                    | T::MedicCableAttack
                    | T::Heatbeam
                    | T::MonsterHeatbeam
                    | T::GrappleCable2
                    | T::LightningBeam
            )
        ) =>
        {
            integer!(Q2TempInt::Entity1, i32::from(reader.short()?));
            position!(Q2TempVec::Position1);
            position!(Q2TempVec::Position2);
        }
        x if T::from_u8(x) == Some(T::GrappleCable) => {
            integer!(Q2TempInt::Entity1, i32::from(reader.short()?));
            position!(Q2TempVec::Position1);
            position!(Q2TempVec::Position2);
            position!(Q2TempVec::Offset);
        }
        x if T::from_u8(x) == Some(T::Lightning) => {
            integer!(Q2TempInt::Entity1, i32::from(reader.short()?));
            integer!(Q2TempInt::Entity2, i32::from(reader.short()?));
            position!(Q2TempVec::Position1);
            position!(Q2TempVec::Position2);
        }
        x if T::from_u8(x) == Some(T::Flashlight) => {
            position!(Q2TempVec::Position1);
            integer!(Q2TempInt::Entity1, i32::from(reader.short()?));
        }
        x if T::from_u8(x) == Some(T::Forcewall) => {
            position!(Q2TempVec::Position1);
            position!(Q2TempVec::Position2);
            integer!(Q2TempInt::Color, i32::from(reader.byte()?));
        }
        x if T::from_u8(x) == Some(T::Steam) => {
            let entity = integer!(Q2TempInt::Entity1, i32::from(reader.short()?));
            integer!(Q2TempInt::Count, i32::from(reader.byte()?));
            position!(Q2TempVec::Position1);
            fields.push(Q2TempField::Vector {
                name: Q2TempVec::Direction,
                value: read_dir(reader)?,
            });
            integer!(Q2TempInt::Color, i32::from(reader.byte()?));
            integer!(Q2TempInt::Entity2, i32::from(reader.short()?));
            if entity != -1 {
                integer!(Q2TempInt::Time, reader.long()?);
            }
        }
        x if T::from_u8(x) == Some(T::Widowbeamout) => {
            integer!(Q2TempInt::Entity1, i32::from(reader.short()?));
            position!(Q2TempVec::Position1);
        }
        x if T::from_u8(x) == Some(T::PowerSplash) => {
            integer!(Q2TempInt::Entity1, i32::from(reader.short()?));
            integer!(Q2TempInt::Count, i32::from(reader.byte()?));
        }
        x if T::from_u8(x) == Some(T::Q2proDamageDealt) => {
            if !q2pro_extended {
                return Err(Q2NetError::Protocol(
                    "Q2PRO damage event needs its game message dialect",
                ));
            }
            integer!(Q2TempInt::Count, i32::from(reader.short()?));
        }
        _ => return Err(Q2NetError::Protocol("Unknown Q2 temporary entity")),
    }
    reader.finish()?;
    let raw = reader.data_slice(start, reader.offset()).to_vec();
    Ok(Q2TempEntity { temp_type, fields, raw })
}

// ---------------------------------------------------------------------------
// Wire codec
// ---------------------------------------------------------------------------

/// Per-protocol server data.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2ServerData {
    /// Classic server data.
    Vanilla(crate::q2::ServerData),
    /// R1Q2 server data.
    R1Q2(crate::q2_variants::R1q2ServerData),
    /// Q2Pro server data.
    Q2Pro(crate::q2_variants::Q2ProServerData),
    /// Rerelease server data.
    Rerelease(crate::q2_variants::RereleaseServerData),
    /// KEX server data.
    Kex(crate::q2_variants::KexServerData),
}

impl Q2ServerData {
    /// Server count.
    #[must_use]
    pub fn servercount(&self) -> i32 {
        match self {
            Self::Vanilla(data) => data.servercount,
            Self::R1Q2(data) => data.servercount,
            Self::Q2Pro(data) => data.servercount,
            Self::Rerelease(data) => data.servercount,
            Self::Kex(data) => data.servercount,
        }
    }

    /// Game directory.
    #[must_use]
    pub fn gamedir(&self) -> &str {
        match self {
            Self::Vanilla(data) => &data.gamedir,
            Self::R1Q2(data) => &data.gamedir,
            Self::Q2Pro(data) => &data.gamedir,
            Self::Rerelease(data) => &data.gamedir,
            Self::Kex(data) => &data.gamedir,
        }
    }

    /// Primary client number.
    #[must_use]
    pub fn clientnum(&self) -> i16 {
        match self {
            Self::Vanilla(data) => data.clientnum,
            Self::R1Q2(data) => data.clientnum,
            Self::Q2Pro(data) => data.clientnum,
            Self::Rerelease(data) => data.clientnum,
            Self::Kex(data) => data.clientnum(),
        }
    }

    /// Level name.
    #[must_use]
    pub fn levelname(&self) -> &str {
        match self {
            Self::Vanilla(data) => &data.levelname,
            Self::R1Q2(data) => &data.levelname,
            Self::Q2Pro(data) => &data.levelname,
            Self::Rerelease(data) => &data.levelname,
            Self::Kex(data) => &data.levelname,
        }
    }

    /// Reported R1Q2 minor version, if any.
    #[must_use]
    pub fn r1q2_version(&self) -> Option<u16> {
        match self {
            Self::R1Q2(data) => Some(data.version),
            _ => None,
        }
    }
}

/// Entity header bits in protocol-native shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2EntityBits {
    /// Classic 32-bit header.
    Classic(u32),
    /// Q2Pro 64-bit header.
    Q2Pro(u64),
    /// Wide header with high byte.
    Wide(WideEntityBits),
}

/// Entity header: number plus protocol-native bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2EntityHeader {
    /// Entity number.
    pub number: u16,
    /// Header bits.
    pub bits: Q2EntityBits,
}

/// Selected protocol codec state.
#[derive(Debug)]
enum Q2SelectedCodec {
    /// Classic protocol 34 (stateless).
    Vanilla,
    /// R1Q2 protocol 35.
    R1Q2(R1q2Codec),
    /// Q2Pro protocol 36.
    Q2Pro(Q2ProCodec),
    /// Rerelease protocol 1038.
    Rerelease(RereleaseCodec),
    /// Classic-compatible protocol 4038.
    PrivateClassic(RereleaseCodec),
    /// KEX protocols 2022/2023.
    Kex(KexCodec),
}

/// Per-connection protocol selection (`Q2WireCodec`).
#[derive(Debug)]
pub struct Q2Wire {
    offered: ProtocolIdentity,
    protocol: ProtocolIdentity,
    codec: Q2SelectedCodec,
    data: Vec<u8>,
    pos: usize,
}

impl Q2Wire {
    /// Select a codec for an offered protocol.
    pub fn new(offered: ProtocolIdentity) -> Result<Self, Q2NetError> {
        if !is_q2(offered) {
            return Err(Q2NetError::Protocol("Not a Q2 protocol identity"));
        }
        q2_codec_support(offered)?;
        let codec = match offered {
            ProtocolIdentity::Q2Classic => Q2SelectedCodec::Vanilla,
            ProtocolIdentity::Q2R1q2 { revision } => {
                if revision != 1903 && revision != 1904 && revision != 1905 {
                    return Err(Q2NetError::Range("Invalid Q2 protocol revision"));
                }
                Q2SelectedCodec::R1Q2(R1q2Codec::new(revision))
            }
            ProtocolIdentity::Q2Q2pro { revision } => {
                if !(1015..=1026).contains(&revision) {
                    return Err(Q2NetError::Range("Invalid Q2 protocol revision"));
                }
                Q2SelectedCodec::Q2Pro(Q2ProCodec::new(Q2ProFeatures {
                    revision: revision as u16,
                    flags: 0,
                }))
            }
            ProtocolIdentity::Q2Rerelease => Q2SelectedCodec::Rerelease(RereleaseCodec::new(false)),
            ProtocolIdentity::Q2PrivateClassic => Q2SelectedCodec::PrivateClassic(RereleaseCodec::new(true)),
            ProtocolIdentity::Q2Kex => Q2SelectedCodec::Kex(KexCodec::new(crate::q2_variants::PROTOCOL_KEX)),
            ProtocolIdentity::Q2KexDemo => Q2SelectedCodec::Kex(KexCodec::new(PROTOCOL_KEX_DEMOS)),
            _ => return Err(Q2NetError::Protocol("Not a Q2 protocol identity")),
        };
        Ok(Self {
            offered,
            protocol: offered,
            codec,
            data: Vec::new(),
            pos: 0,
        })
    }

    /// Active protocol.
    #[must_use]
    pub fn protocol(&self) -> ProtocolIdentity {
        self.protocol
    }

    /// Whether coordinates travel as floats.
    #[must_use]
    pub fn floating_coordinates(&self) -> bool {
        matches!(
            self.protocol,
            ProtocolIdentity::Q2Rerelease | ProtocolIdentity::Q2PrivateClassic | ProtocolIdentity::Q2Kex
        )
    }

    /// Whether wide indexes apply.
    #[must_use]
    pub fn wide_indexes(&self) -> bool {
        q2_version(self.protocol) >= 1038
    }

    /// Negotiated Q2Pro revision (0 when not Q2Pro).
    #[must_use]
    pub fn q2pro_revision(&self) -> u16 {
        match &self.codec {
            Q2SelectedCodec::Q2Pro(codec) => codec.features().revision,
            _ => 0,
        }
    }

    /// Whether Q2Pro game extensions are negotiated.
    #[must_use]
    pub fn q2pro_extended(&self) -> bool {
        matches!(&self.codec, Q2SelectedCodec::Q2Pro(codec) if codec.features().extensions())
    }

    /// Whether Q2Pro v2 game extensions are negotiated.
    #[must_use]
    pub fn q2pro_extended_v2(&self) -> bool {
        matches!(&self.codec, Q2SelectedCodec::Q2Pro(codec) if codec.features().extensions_v2())
    }

    /// Whether the KEX wire is active.
    #[must_use]
    pub fn is_kex(&self) -> bool {
        matches!(self.protocol, ProtocolIdentity::Q2Kex | ProtocolIdentity::Q2KexDemo)
    }

    /// Split-screen player count (KEX only).
    #[must_use]
    pub fn kex_split_player_count(&self) -> usize {
        match &self.codec {
            Q2SelectedCodec::Kex(codec) => codec.split_player_count(),
            _ => 1,
        }
    }

    /// Accept the server-reported R1Q2 revision.
    pub fn accept_server_revision(&mut self, reported: Option<i64>) -> Result<(), Q2NetError> {
        let ProtocolIdentity::Q2R1q2 { revision: offered } = self.offered else {
            return Ok(());
        };
        let revision = negotiated_r1q2_protocol(offered, reported)?;
        self.codec = Q2SelectedCodec::R1Q2(R1q2Codec::new(revision));
        self.protocol = ProtocolIdentity::Q2R1q2 { revision };
        Ok(())
    }

    /// Record negotiated Q2Pro features.
    pub fn accept_q2pro_features(&mut self, revision: u16, flags: u16) -> Result<(), Q2NetError> {
        let ProtocolIdentity::Q2Q2pro { revision: current } = self.protocol else {
            return Err(Q2NetError::Protocol(
                "Recorded Q2PRO features disagree with protocol identity",
            ));
        };
        if u32::from(revision) != current {
            return Err(Q2NetError::Protocol(
                "Recorded Q2PRO features disagree with protocol identity",
            ));
        }
        let Q2SelectedCodec::Q2Pro(codec) = &mut self.codec else {
            return Err(Q2NetError::Protocol(
                "Recorded Q2PRO features disagree with protocol identity",
            ));
        };
        // Features mutate through server data; mirror the shared donor record.
        let features = Q2ProFeatures { revision, flags };
        *codec = Q2ProCodec::new(features);
        Ok(())
    }

    /// Load bytes for reading (`begin`).
    pub fn begin(&mut self, bytes: &[u8]) {
        self.data = bytes.to_vec();
        self.pos = 0;
    }

    /// Check the read cursor (`finish`).
    pub fn finish(&self) -> Result<(), Q2NetError> {
        if self.pos > self.data.len() {
            return Err(Q2NetError::Msg(MsgError::Truncated(self.pos)));
        }
        Ok(())
    }

    /// Current read offset.
    #[must_use]
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Reposition the read cursor (MVD header re-reads).
    pub(crate) fn seek(&mut self, pos: usize) {
        self.pos = pos.min(self.data.len());
    }

    /// Bytes remaining.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    /// Raw slice from `start` to the cursor.
    #[must_use]
    pub fn raw_slice(&self, start: usize) -> &[u8] {
        &self.data[start.min(self.pos)..self.pos.min(self.data.len())]
    }

    /// Run a closure against the wire cursor, advancing past what it reads.
    pub fn with_reader<T>(
        &mut self,
        read: impl FnOnce(&mut MsgReader<'_>) -> Result<T, Q2NetError>,
    ) -> Result<T, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = read(&mut reader)?;
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read one raw byte from the wire cursor.
    pub fn read_raw_byte(&mut self) -> Result<u8, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = reader.byte()?;
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read raw bytes from the wire cursor.
    pub fn read_raw_data(&mut self, len: usize) -> Result<Vec<u8>, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = reader.bytes(len)?.to_vec();
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read a raw little-endian short.
    pub fn read_raw_short(&mut self) -> Result<i16, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = reader.short()?;
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read a raw little-endian long.
    pub fn read_raw_long(&mut self) -> Result<i32, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = reader.long()?;
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read a raw NUL-terminated string.
    pub fn read_raw_string(&mut self, limit: usize) -> Result<String, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = reader.string(limit);
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read a batched move for the active protocol.
    pub fn read_batch_move_wire(&mut self, nodelta: bool, opcode_extra: u8) -> Result<BatchMove, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let batch = match &mut self.codec {
            Q2SelectedCodec::Q2Pro(_) => Q2ProCodec::read_batch_move(&mut reader, nodelta, opcode_extra)?,
            Q2SelectedCodec::Rerelease(codec) | Q2SelectedCodec::PrivateClassic(codec) => {
                codec.read_batch_move(&mut reader, nodelta)?
            }
            _ => {
                return Err(Q2NetError::Protocol(
                    "Batched movement is not supported by selected Q2 wire",
                ))
            }
        };
        self.pos = reader.offset();
        Ok(batch)
    }

    /// Write a batched move for the active protocol.
    pub fn write_batch_move_wire(
        &mut self,
        writer: &mut MsgWriter,
        lastframe: Option<i32>,
        frames: &[BatchMoveFrame],
    ) -> Result<(), Q2NetError> {
        match &mut self.codec {
            Q2SelectedCodec::Q2Pro(_) => Ok(Q2ProCodec::write_batch_move(writer, lastframe, frames)?),
            Q2SelectedCodec::Rerelease(codec) | Q2SelectedCodec::PrivateClassic(codec) => {
                Ok(codec.write_batch_move(writer, lastframe, frames)?)
            }
            _ => Err(Q2NetError::Protocol("Selected Q2 codec cannot write batched moves")),
        }
    }

    /// Mask a frame opcode and note its extra bits (`opcode`).
    pub fn opcode(&mut self, raw: u8) -> u8 {
        match &mut self.codec {
            Q2SelectedCodec::R1Q2(codec) => {
                codec.set_frame_extrabits(raw & 0xe0);
                raw & 31
            }
            Q2SelectedCodec::Q2Pro(codec) => {
                codec.note_frame_opcode_extrabits(raw & 0xe0);
                raw & 31
            }
            _ => raw,
        }
    }

    /// Read a nested stream, resuming the outer packet after it (`nested`).
    pub fn nested<T>(
        &mut self,
        bytes: &[u8],
        read: impl FnOnce(&mut Self) -> Result<T, Q2NetError>,
    ) -> Result<T, Q2NetError> {
        let previous = std::mem::replace(&mut self.data, bytes.to_vec());
        let previous_pos = std::mem::replace(&mut self.pos, 0);
        let result = read(self);
        let finish = self.finish();
        self.data = previous;
        self.pos = previous_pos;
        finish?;
        result
    }

    /// Write server data for the active protocol.
    pub fn write_server_data(&mut self, writer: &mut MsgWriter, data: &Q2ServerData) -> Result<(), Q2NetError> {
        match (&mut self.codec, data) {
            (Q2SelectedCodec::Vanilla, Q2ServerData::Vanilla(data)) => Ok(crate::q2::write_server_data(writer, data)?),
            (Q2SelectedCodec::R1Q2(codec), Q2ServerData::R1Q2(data)) => Ok(codec.write_server_data(writer, data)?),
            (Q2SelectedCodec::Q2Pro(codec), Q2ServerData::Q2Pro(data)) => Ok(codec.write_server_data(writer, data)?),
            (Q2SelectedCodec::Rerelease(codec), Q2ServerData::Rerelease(data))
            | (Q2SelectedCodec::PrivateClassic(codec), Q2ServerData::Rerelease(data)) => {
                Ok(codec.write_server_data(writer, data)?)
            }
            (Q2SelectedCodec::Kex(codec), Q2ServerData::Kex(data)) => Ok(codec.write_server_data(writer, data)?),
            _ => Err(Q2NetError::Protocol("Q2 server data disagrees with protocol")),
        }
    }

    /// Read server data for the active protocol.
    pub fn read_server_data(&mut self) -> Result<Q2ServerData, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let data = match &mut self.codec {
            Q2SelectedCodec::Vanilla => Q2ServerData::Vanilla(crate::q2::read_server_data(&mut reader)?),
            Q2SelectedCodec::R1Q2(_) => Q2ServerData::R1Q2(R1q2Codec::read_server_data(&mut reader)?),
            Q2SelectedCodec::Q2Pro(codec) => {
                let minor = codec.features().revision;
                Q2ServerData::Q2Pro(codec.read_server_data(&mut reader, minor)?)
            }
            Q2SelectedCodec::Rerelease(_) | Q2SelectedCodec::PrivateClassic(_) => {
                Q2ServerData::Rerelease(RereleaseCodec::read_server_data(&mut reader)?)
            }
            Q2SelectedCodec::Kex(codec) => Q2ServerData::Kex(codec.read_server_data(&mut reader)?),
        };
        self.pos = reader.offset();
        Ok(data)
    }

    /// Write a spawn baseline.
    pub fn write_spawn_baseline(&mut self, writer: &mut MsgWriter, entity: &EntityState) -> Result<bool, Q2NetError> {
        match &mut self.codec {
            Q2SelectedCodec::Vanilla => Ok(crate::q2::write_spawn_baseline(writer, entity)?),
            Q2SelectedCodec::R1Q2(codec) => Ok(codec.write_spawn_baseline(writer, entity)?),
            Q2SelectedCodec::Q2Pro(codec) => Ok(codec.write_spawn_baseline(writer, entity)?),
            Q2SelectedCodec::Rerelease(_) | Q2SelectedCodec::PrivateClassic(_) => {
                Ok(RereleaseCodec::write_spawn_baseline(writer, entity)?)
            }
            Q2SelectedCodec::Kex(codec) => Ok(codec.write_spawn_baseline(writer, entity)?),
        }
    }

    /// Write an entity removal.
    pub fn write_entity_remove(&mut self, writer: &mut MsgWriter, number: u16) -> Result<(), Q2NetError> {
        match &mut self.codec {
            Q2SelectedCodec::Kex(_) => Ok(KexCodec::write_entity_remove(writer, number)?),
            _ => Ok(crate::q2::write_entity_remove(writer, number)?),
        }
    }

    /// Write the packet-entities terminator.
    pub fn write_packet_entities_end(&self, writer: &mut MsgWriter) -> Result<(), Q2NetError> {
        Ok(crate::q2::write_packet_entities_end(writer)?)
    }

    /// Consume the packet-entities opcode.
    pub fn read_packet_entities_begin(&mut self) -> Result<(), Q2NetError> {
        match &mut self.codec {
            Q2SelectedCodec::Kex(_) => {
                let mut reader = MsgReader::new(&self.data);
                reader.skip(self.pos)?;
                let result = KexCodec::read_packet_entities_begin(&mut reader);
                self.pos = reader.offset();
                Ok(result?)
            }
            _ => {
                let mut reader = MsgReader::new(&self.data);
                reader.skip(self.pos)?;
                let result = crate::q2::read_packet_entities_begin(&mut reader);
                self.pos = reader.offset();
                Ok(result?)
            }
        }
    }

    /// Write a delta entity.
    pub fn write_delta_entity(
        &mut self,
        writer: &mut MsgWriter,
        from: &EntityState,
        to: &EntityState,
        force: bool,
        newentity: bool,
    ) -> Result<bool, Q2NetError> {
        match &mut self.codec {
            Q2SelectedCodec::Vanilla => Ok(crate::q2::write_delta_entity(writer, from, to, force, newentity)?),
            Q2SelectedCodec::R1Q2(codec) => Ok(codec.write_delta_entity(writer, from, to, force, newentity)?),
            Q2SelectedCodec::Q2Pro(codec) => Ok(codec.write_delta_entity(writer, from, to, force, newentity)?),
            Q2SelectedCodec::Rerelease(_) | Q2SelectedCodec::PrivateClassic(_) => {
                Ok(RereleaseCodec::write_delta_entity(writer, from, to, force, newentity)?)
            }
            Q2SelectedCodec::Kex(codec) => Ok(codec.write_delta_entity(writer, from, to, force, newentity)?),
        }
    }

    /// Read entity header bits.
    pub fn read_entity_bits(&mut self) -> Result<Q2EntityHeader, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let header = match &mut self.codec {
            Q2SelectedCodec::Vanilla | Q2SelectedCodec::R1Q2(_) => {
                let (number, bits) = crate::q2::read_entity_bits(&mut reader)?;
                Q2EntityHeader {
                    number,
                    bits: Q2EntityBits::Classic(bits),
                }
            }
            Q2SelectedCodec::Q2Pro(_) => {
                let (number, bits) = read_q2pro_entity_bits(&mut reader)?;
                Q2EntityHeader {
                    number,
                    bits: Q2EntityBits::Q2Pro(bits),
                }
            }
            Q2SelectedCodec::Rerelease(_) | Q2SelectedCodec::PrivateClassic(_) | Q2SelectedCodec::Kex(_) => {
                let wide = read_entity_bits_wide(&mut reader)?;
                Q2EntityHeader {
                    number: wide.number,
                    bits: Q2EntityBits::Wide(wide),
                }
            }
        };
        self.pos = reader.offset();
        Ok(header)
    }

    /// Read a delta entity body.
    pub fn read_delta_entity(
        &mut self,
        from: &EntityState,
        number: u16,
        header: Q2EntityHeader,
    ) -> Result<EntityState, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let entity = match (&mut self.codec, header.bits) {
            (Q2SelectedCodec::Vanilla, Q2EntityBits::Classic(bits)) => {
                crate::q2::read_delta_entity(&mut reader, from, number, bits)?
            }
            (Q2SelectedCodec::R1Q2(codec), Q2EntityBits::Classic(bits)) => {
                codec.read_delta_entity(&mut reader, from, number, bits)?
            }
            (Q2SelectedCodec::Q2Pro(codec), Q2EntityBits::Q2Pro(bits)) => {
                codec.read_delta_entity(&mut reader, from, number, bits)?
            }
            (Q2SelectedCodec::Rerelease(_) | Q2SelectedCodec::PrivateClassic(_), Q2EntityBits::Wide(wide)) => {
                RereleaseCodec::read_delta_entity(&mut reader, from, number, wide)?
            }
            (Q2SelectedCodec::Kex(codec), Q2EntityBits::Wide(wide)) => {
                codec.read_delta_entity(&mut reader, from, number, wide)?
            }
            _ => return Err(Q2NetError::Protocol("Q2 entity bits disagree with protocol")),
        };
        self.pos = reader.offset();
        Ok(entity)
    }

    /// Read a frame header.
    pub fn read_frame_header(
        &mut self,
        areabits: &mut Vec<u8>,
        read_suppress_byte: bool,
    ) -> Result<FrameHeader, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let header = match &mut self.codec {
            Q2SelectedCodec::Vanilla => crate::q2::read_frame_header(&mut reader, areabits, read_suppress_byte)?,
            Q2SelectedCodec::R1Q2(codec) => codec.read_frame_header(&mut reader, areabits)?,
            Q2SelectedCodec::Q2Pro(codec) => codec.read_frame_header(&mut reader, areabits)?,
            Q2SelectedCodec::Rerelease(codec) | Q2SelectedCodec::PrivateClassic(codec) => {
                codec.read_frame_header(&mut reader, areabits)?
            }
            Q2SelectedCodec::Kex(_) => KexCodec::read_frame_header(&mut reader, areabits)?,
        };
        self.pos = reader.offset();
        Ok(header)
    }

    /// Read a frame player state.
    pub fn read_frame_playerstate(&mut self, from: &PlayerState) -> Result<PlayerState, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let player = match &mut self.codec {
            Q2SelectedCodec::Vanilla => crate::q2::read_frame_playerstate(&mut reader, from)?,
            Q2SelectedCodec::R1Q2(codec) => codec.read_frame_playerstate(&mut reader, from)?,
            Q2SelectedCodec::Q2Pro(codec) => codec.read_frame_playerstate(&mut reader, from)?,
            Q2SelectedCodec::Rerelease(codec) | Q2SelectedCodec::PrivateClassic(codec) => {
                codec.read_frame_playerstate(&mut reader, from)?
            }
            Q2SelectedCodec::Kex(_) => KexCodec::read_frame_playerstate(&mut reader, from)?,
        };
        self.pos = reader.offset();
        Ok(player)
    }

    /// Write a delta user command.
    pub fn write_delta_usercmd(
        &mut self,
        writer: &mut MsgWriter,
        from: &Usercmd,
        cmd: &Usercmd,
    ) -> Result<(), Q2NetError> {
        match &mut self.codec {
            Q2SelectedCodec::Vanilla => Ok(crate::q2::write_delta_usercmd(writer, from, cmd)?),
            Q2SelectedCodec::R1Q2(codec) => Ok(codec.write_delta_usercmd(writer, from, cmd)?),
            Q2SelectedCodec::Q2Pro(_) => Ok(crate::q2::write_delta_usercmd(writer, from, cmd)?),
            Q2SelectedCodec::Rerelease(codec) | Q2SelectedCodec::PrivateClassic(codec) => {
                Ok(codec.write_delta_usercmd(writer, from, cmd)?)
            }
            Q2SelectedCodec::Kex(_) => Ok(crate::q2_variants::write_kex_usercmd(writer, from, cmd)?),
        }
    }

    /// Read a delta user command.
    pub fn read_delta_usercmd(&mut self, from: &Usercmd) -> Result<Usercmd, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let cmd = match &mut self.codec {
            Q2SelectedCodec::Vanilla => crate::q2::read_delta_usercmd(&mut reader, from)?,
            Q2SelectedCodec::R1Q2(codec) => codec.read_delta_usercmd(&mut reader, from)?,
            Q2SelectedCodec::Q2Pro(_) => crate::q2::read_delta_usercmd(&mut reader, from)?,
            Q2SelectedCodec::Rerelease(codec) | Q2SelectedCodec::PrivateClassic(codec) => {
                codec.read_delta_usercmd(&mut reader, from)?
            }
            Q2SelectedCodec::Kex(_) => crate::q2_variants::read_kex_usercmd(&mut reader, from)?,
        };
        self.pos = reader.offset();
        Ok(cmd)
    }

    /// Read KEX damage indicators.
    pub fn read_kex_damage(&mut self) -> Result<Vec<KexDamageIndicator>, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = KexCodec::read_damage(&mut reader)?;
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read a KEX localized print.
    pub fn read_kex_locprint(&mut self) -> Result<KexLocprint, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = KexCodec::read_locprint(&mut reader)?;
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read a KEX point of interest.
    pub fn read_kex_poi(&mut self) -> Result<KexPoi, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = KexCodec::read_poi(&mut reader)?;
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read a KEX help path node.
    pub fn read_kex_help_path(&mut self) -> Result<KexHelpPath, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = KexCodec::read_help_path(&mut reader)?;
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read a KEX configstring blast.
    pub fn read_kex_configblast(&mut self) -> Result<Vec<KexConfigstringRecord>, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = KexCodec::read_configblast(&mut reader)?;
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read a KEX spawn baseline blast.
    pub fn read_kex_spawnbaselineblast(&mut self) -> Result<Vec<crate::q2_variants::KexBaseline>, Q2NetError> {
        let Q2SelectedCodec::Kex(codec) = &mut self.codec else {
            return Err(Q2NetError::Protocol("KEX blast on a non-KEX wire"));
        };
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = codec.read_spawnbaselineblast(&mut reader)?;
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read a KEX muzzle flash.
    pub fn read_kex_muzzleflash3(&mut self) -> Result<crate::q2_variants::KexMuzzleflash3, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = KexCodec::read_muzzleflash3(&mut reader)?;
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read a KEX achievement string.
    pub fn read_kex_achievement(&mut self) -> Result<String, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = KexCodec::read_achievement(&mut reader);
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read a KEX sound.
    pub fn read_kex_sound(&mut self) -> Result<crate::q2_variants::KexSound, Q2NetError> {
        let Q2SelectedCodec::Kex(codec) = &mut self.codec else {
            return Err(Q2NetError::Protocol("KEX sound on a non-KEX wire"));
        };
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = codec.read_sound(&mut reader)?;
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read fog data.
    pub fn read_fog(&mut self) -> Result<FogData, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = crate::q2_variants::read_fog(&mut reader)?;
        self.pos = reader.offset();
        Ok(value)
    }

    /// Read a zpacket payload.
    pub fn read_zpacket_payload(&mut self) -> Result<Vec<u8>, Q2NetError> {
        let mut reader = MsgReader::new(&self.data);
        reader.skip(self.pos)?;
        let value = read_zpacket_payload(&mut reader)?;
        self.pos = reader.offset();
        Ok(value)
    }
}

/// Decode bytes with a wire codec (`decodeWithQ2Codec`).
pub fn decode_with_q2_codec<T>(
    wire: &mut Q2Wire,
    bytes: &[u8],
    read: impl FnOnce(&mut Q2Wire) -> Result<T, Q2NetError>,
) -> Result<T, Q2NetError> {
    wire.begin(bytes);
    let result = read(wire)?;
    wire.finish()?;
    Ok(result)
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

/// KEX split-screen player (`splitPlayers` entry).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SplitPlayer {
    /// Area visibility bits.
    pub area_bits: Vec<u8>,
    /// Player state.
    pub player: PlayerState,
}

/// Decoded wire frame (`Q2WireFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WireFrame {
    /// Whether the delta base was usable.
    pub valid: bool,
    /// Server frame number.
    pub server_frame: i32,
    /// Delta base frame.
    pub delta_frame: i32,
    /// Suppressed-packet count.
    pub suppressed_count: i32,
    /// Area visibility bits.
    pub area_bits: Vec<u8>,
    /// Player state.
    pub player: PlayerState,
    /// Split-screen players (KEX).
    pub split_players: Vec<Q2SplitPlayer>,
    /// Entities.
    pub entities: Vec<EntityState>,
}

/// Encode packet entities with a selected codec.
fn encode_packet_entities(
    codec: &Q2SelectedCodec,
    from: &[EntityState],
    to: &[EntityState],
    baselines: &HashMap<u16, EntityState>,
    max_clients: u32,
) -> Result<Vec<u8>, Q2NetError> {
    ordered_entities(from)?;
    ordered_entities(to)?;
    let mut writer = MsgWriter::new(65536, false);
    writer.write_byte(protocol::Svc::Packetentities as u8)?;
    let mut before = 0usize;
    let mut after = 0usize;
    while before < from.len() || after < to.len() {
        let old_number = from.get(before).map_or(u32::MAX, |entity| u32::from(entity.number));
        let new_number = to.get(after).map_or(u32::MAX, |entity| u32::from(entity.number));
        if after < to.len() && before < from.len() && old_number == new_number {
            delta_entity(
                codec,
                &mut writer,
                &from[before],
                &to[after],
                false,
                u32::from(to[after].number) <= max_clients,
            )?;
            before += 1;
            after += 1;
        } else if after < to.len() && new_number < old_number {
            let base = baselines.get(&to[after].number).cloned().unwrap_or_default();
            delta_entity(codec, &mut writer, &base, &to[after], true, true)?;
            after += 1;
        } else if before < from.len() {
            remove_entity(codec, &mut writer, from[before].number)?;
            before += 1;
        } else {
            return Err(Q2NetError::Protocol("Invalid Q2 packet entity merge"));
        }
    }
    crate::q2::write_packet_entities_end(&mut writer)?;
    Ok(writer.bytes().to_vec())
}

/// Check entity ordering (`ordered`).
fn ordered_entities(entities: &[EntityState]) -> Result<(), Q2NetError> {
    let mut previous = 0u32;
    for entity in entities {
        let number = u32::from(entity.number);
        if number <= previous || number > 65535 {
            return Err(Q2NetError::Range("Q2 frame entity numbers must be unique and sorted"));
        }
        previous = number;
    }
    Ok(())
}

fn remove_entity(codec: &Q2SelectedCodec, writer: &mut MsgWriter, number: u16) -> Result<(), Q2NetError> {
    if matches!(codec, Q2SelectedCodec::Kex(_)) {
        Ok(KexCodec::write_entity_remove(writer, number)?)
    } else {
        Ok(crate::q2::write_entity_remove(writer, number)?)
    }
}

fn delta_entity(
    codec: &Q2SelectedCodec,
    writer: &mut MsgWriter,
    from: &EntityState,
    to: &EntityState,
    force: bool,
    newentity: bool,
) -> Result<(), Q2NetError> {
    match codec {
        Q2SelectedCodec::Vanilla => {
            crate::q2::write_delta_entity(writer, from, to, force, newentity)?;
        }
        Q2SelectedCodec::R1Q2(codec) => {
            codec.write_delta_entity(writer, from, to, force, newentity)?;
        }
        Q2SelectedCodec::Q2Pro(codec) => {
            codec.write_delta_entity(writer, from, to, force, newentity)?;
        }
        Q2SelectedCodec::Rerelease(_) | Q2SelectedCodec::PrivateClassic(_) => {
            RereleaseCodec::write_delta_entity(writer, from, to, force, newentity)?;
        }
        Q2SelectedCodec::Kex(codec) => {
            codec.write_delta_entity(writer, from, to, force, newentity)?;
        }
    }
    Ok(())
}

/// Write packet entities (`writePacketEntities`).
pub fn write_packet_entities(
    wire: &Q2Wire,
    writer: &mut MsgWriter,
    from: &[EntityState],
    to: &[EntityState],
    baselines: &HashMap<u16, EntityState>,
    max_clients: u32,
) -> Result<(), Q2NetError> {
    let bytes = encode_packet_entities(&wire.codec, from, to, baselines, max_clients)?;
    Ok(writer.write_bytes(&bytes)?)
}

/// Encode a full frame (`encodeQ2Frame`).
///
/// Entities encode first and splice into the frame closure, producing the
/// donor's exact byte order without aliasing the codec.
pub fn encode_q2_frame(
    wire: &Q2Wire,
    frame: &Q2WireFrame,
    old: Option<&Q2WireFrame>,
    baselines: &HashMap<u16, EntityState>,
    max_clients: u32,
) -> Result<Vec<u8>, Q2NetError> {
    if wire.is_kex() && !frame.split_players.is_empty() {
        let mut writer = MsgWriter::new(65536, false);
        writer.write_byte(protocol::Svc::Frame as u8)?;
        writer.write_long(frame.server_frame)?;
        writer.write_long(old.map_or(-1, |frame| frame.server_frame))?;
        writer.write_byte(frame.suppressed_count as u8)?;
        let mut states = Vec::with_capacity(frame.split_players.len() + 1);
        states.push((frame.area_bits.as_slice(), &frame.player));
        for split in &frame.split_players {
            states.push((split.area_bits.as_slice(), &split.player));
        }
        let base = PlayerState::default();
        for (index, (area_bits, player)) in states.iter().enumerate() {
            // The donor's missing-split error covers sparse arrays, which a
            // Vec cannot express; short histories fall back to a zero state.
            let previous = if index == 0 {
                old.map_or(&base, |frame| &frame.player)
            } else {
                old.and_then(|frame| frame.split_players.get(index - 1).map(|split| &split.player))
                    .unwrap_or(&base)
            };
            writer.write_byte(area_bits.len() as u8)?;
            writer.write_bytes(area_bits)?;
            writer.write_byte(protocol::Svc::Playerinfo as u8)?;
            KexCodec::write_player_state_delta(&mut writer, previous, player)?;
        }
        let entities = encode_packet_entities(
            &wire.codec,
            old.map_or(&[], |frame| frame.entities.as_slice()),
            &frame.entities,
            baselines,
            max_clients,
        )?;
        writer.write_bytes(&entities)?;
        return Ok(writer.bytes().to_vec());
    }
    let entities = encode_packet_entities(
        &wire.codec,
        old.map_or(&[], |frame| frame.entities.as_slice()),
        &frame.entities,
        baselines,
        max_clients,
    )?;
    let mut writer = MsgWriter::new(65536, false);
    let params = crate::q2::FrameWrite {
        framenum: frame.server_frame,
        lastframe: old.map_or(-1, |frame| frame.server_frame),
        surpress_count: frame.suppressed_count,
        areabits: &frame.area_bits,
        ps_from: old.map(|frame| &frame.player),
        ps_to: &frame.player,
    };
    match &wire.codec {
        Q2SelectedCodec::Vanilla => {
            crate::q2::write_frame(&mut writer, &params, |body| {
                body.write_bytes(&entities).map_err(Q2NetError::from)
            })?;
        }
        Q2SelectedCodec::R1Q2(_) => {
            R1q2Codec::write_frame(&mut writer, &params, |body| {
                body.write_bytes(&entities).map_err(Q2NetError::from)
            })?;
        }
        Q2SelectedCodec::Q2Pro(codec) => {
            codec.write_frame(&mut writer, &params, |body| {
                body.write_bytes(&entities).map_err(Q2NetError::from)
            })?;
        }
        Q2SelectedCodec::Rerelease(_) | Q2SelectedCodec::PrivateClassic(_) => {
            RereleaseCodec::write_frame(&mut writer, &params, |body| {
                body.write_bytes(&entities).map_err(Q2NetError::from)
            })?;
        }
        Q2SelectedCodec::Kex(_) => {
            KexCodec::write_frame(&mut writer, &params, |body| {
                body.write_bytes(&entities).map_err(Q2NetError::from)
            })?;
        }
    }
    Ok(writer.bytes().to_vec())
}

/// Delta-frame history for entity merges (`Q2FrameHistory`).
#[derive(Debug)]
pub struct Q2FrameHistory {
    capacity: usize,
    frames: HashMap<i32, Q2WireFrame>,
    order: VecDeque<i32>,
    /// Entity baselines.
    pub baselines: HashMap<u16, EntityState>,
}

impl Q2FrameHistory {
    /// Empty history with a frame capacity.
    pub fn new(capacity: usize) -> Result<Self, Q2NetError> {
        if capacity < 1 {
            return Err(Q2NetError::Range("Invalid Q2 frame history"));
        }
        Ok(Self {
            capacity,
            frames: HashMap::new(),
            order: VecDeque::new(),
            baselines: HashMap::new(),
        })
    }

    /// Seed baselines without overwriting entries already present.
    pub fn seed_baselines(&mut self, baselines: &HashMap<u16, EntityState>) {
        for (number, baseline) in baselines {
            self.baselines.entry(*number).or_insert_with(|| baseline.clone());
        }
    }

    /// Borrow a frame.
    #[must_use]
    pub fn get(&self, number: i32) -> Option<&Q2WireFrame> {
        self.frames.get(&number)
    }

    /// Latest valid frame, if any.
    #[must_use]
    pub fn latest(&self) -> Option<&Q2WireFrame> {
        let mut latest: Option<&Q2WireFrame> = None;
        for frame in self.frames.values() {
            if frame.valid && latest.is_none_or(|best: &Q2WireFrame| frame.server_frame > best.server_frame) {
                latest = Some(frame);
            }
        }
        latest
    }

    /// Clear frames and baselines.
    pub fn clear(&mut self) {
        self.frames.clear();
        self.order.clear();
        self.baselines.clear();
    }

    /// Record a frame, evicting the oldest past capacity.
    pub fn accept(&mut self, frame: Q2WireFrame) {
        if !self.frames.contains_key(&frame.server_frame) {
            self.order.push_back(frame.server_frame);
        }
        self.frames.insert(frame.server_frame, frame);
        while self.frames.len() > self.capacity {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.frames.remove(&oldest);
        }
    }

    /// Decode one frame from the wire cursor, recording it (`read`).
    ///
    /// The caller consumes and masks the frame opcode first.
    pub fn read(&mut self, wire: &mut Q2Wire, read_suppress_byte: bool) -> Result<Q2WireFrame, Q2NetError> {
        let mut area_bits = Vec::new();
        let header = wire.read_frame_header(&mut area_bits, read_suppress_byte)?;
        let old = if header.deltaframe > 0 {
            self.frames.get(&header.deltaframe)
        } else {
            None
        };
        let valid = header.deltaframe <= 0 || old.is_some_and(|frame| frame.valid);
        let base = PlayerState::default();
        let player = wire.read_frame_playerstate(old.map_or(&base, |frame| &frame.player))?;
        let mut split_players = Vec::new();
        if wire.is_kex() {
            for index in 1..wire.kex_split_player_count() {
                let len = usize::from(wire.read_raw_byte()?);
                let area = wire.read_raw_data(len)?;
                let next = wire.read_frame_playerstate(
                    old.and_then(|frame| frame.split_players.get(index - 1))
                        .map_or(&base, |split| &split.player),
                )?;
                split_players.push(Q2SplitPlayer {
                    area_bits: area,
                    player: next,
                });
            }
        }
        wire.read_packet_entities_begin()?;
        let previous: &[EntityState] = old.map_or(&[], |frame| frame.entities.as_slice());
        let mut entities = Vec::new();
        let mut cursor = 0usize;
        let mut last_number = 0u16;
        loop {
            let header_bits = wire.read_entity_bits()?;
            if header_bits.number == 0 {
                break;
            }
            if header_bits.number <= last_number {
                return Err(Q2NetError::Protocol("Unordered Q2 entity delta"));
            }
            last_number = header_bits.number;
            while let Some(from) = previous.get(cursor) {
                if from.number >= header_bits.number {
                    break;
                }
                let from = from.clone();
                let carried = Q2EntityHeader {
                    number: from.number,
                    bits: zero_entity_bits(wire.protocol()),
                };
                entities.push(wire.read_delta_entity(&from, from.number, carried)?);
                cursor += 1;
            }
            let from = previous.get(cursor).cloned();
            if (entity_remove_bit(header_bits) & protocol::U_REMOVE) != 0 {
                if from.as_ref().is_none_or(|from| from.number != header_bits.number) {
                    if valid {
                        return Err(Q2NetError::Protocol("Q2 entity removal has no delta source"));
                    }
                } else {
                    cursor += 1;
                }
                continue;
            }
            let matched = from.as_ref().is_some_and(|from| from.number == header_bits.number);
            // The donor's missing-baseline throw is dead: the lookup falls
            // back to a fresh state first.
            let base = if matched {
                from.clone().unwrap_or_default()
            } else {
                self.baselines.get(&header_bits.number).cloned().unwrap_or_default()
            };
            entities.push(wire.read_delta_entity(&base, header_bits.number, header_bits)?);
            if matched {
                cursor += 1;
            }
        }
        while cursor < previous.len() {
            let from = previous[cursor].clone();
            let carried = Q2EntityHeader {
                number: from.number,
                bits: zero_entity_bits(wire.protocol()),
            };
            entities.push(wire.read_delta_entity(&from, from.number, carried)?);
            cursor += 1;
        }
        let frame = Q2WireFrame {
            valid,
            server_frame: header.serverframe,
            delta_frame: header.deltaframe,
            suppressed_count: header.surpress_count,
            area_bits,
            player,
            split_players,
            entities,
        };
        self.accept(frame.clone());
        Ok(frame)
    }
}

/// Classic remove bit of an entity header (all protocols share bit 6).
fn entity_remove_bit(header: Q2EntityHeader) -> u32 {
    match header.bits {
        Q2EntityBits::Classic(bits) => bits,
        Q2EntityBits::Q2Pro(bits) => bits as u32,
        Q2EntityBits::Wide(wide) => wide.lo,
    }
}

/// Zero header bits in protocol-native shape.
fn zero_entity_bits(protocol: ProtocolIdentity) -> Q2EntityBits {
    match protocol {
        ProtocolIdentity::Q2Q2pro { .. } => Q2EntityBits::Q2Pro(0),
        ProtocolIdentity::Q2Rerelease
        | ProtocolIdentity::Q2PrivateClassic
        | ProtocolIdentity::Q2Kex
        | ProtocolIdentity::Q2KexDemo => Q2EntityBits::Wide(WideEntityBits {
            number: 0,
            lo: 0,
            hi: 0,
        }),
        _ => Q2EntityBits::Classic(0),
    }
}

// ---------------------------------------------------------------------------
// Server messages
// ---------------------------------------------------------------------------

/// Read mode for demo quirks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2ReadMode {
    /// Live network.
    Network,
    /// Demo playback.
    Demo,
}

/// Sound message (`Q2SoundMessage`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SoundMessage {
    /// Field flags.
    pub flags: u8,
    /// Sound index.
    pub index: u16,
    /// Entity.
    pub entity: u32,
    /// Channel.
    pub channel: u8,
    /// Position.
    pub position: Option<[f64; 3]>,
    /// Volume.
    pub volume: f64,
    /// Attenuation.
    pub attenuation: f64,
    /// Delay in seconds.
    pub delay_seconds: f64,
}

/// Server event (`Q2ServerEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2ServerEvent {
    /// No operation.
    Nop,
    /// Disconnect.
    Disconnect,
    /// Reconnect.
    Reconnect,
    /// Level restart.
    LevelRestart,
    /// Server data.
    ServerData {
        /// Parsed server data.
        data: Box<Q2ServerData>,
    },
    /// Print.
    Print {
        /// Level.
        level: u8,
        /// Text.
        text: String,
    },
    /// Center print.
    CenterPrint {
        /// Text.
        text: String,
    },
    /// Command text.
    CommandText {
        /// Text.
        text: String,
    },
    /// Layout.
    Layout {
        /// Text.
        text: String,
    },
    /// Achievement.
    Achievement {
        /// Text.
        text: String,
    },
    /// Configstring.
    ConfigString {
        /// Index.
        index: u16,
        /// Value.
        value: String,
    },
    /// Spawn baseline.
    Baseline {
        /// Entity.
        entity: EntityState,
    },
    /// Frame.
    Frame {
        /// Frame.
        frame: Box<Q2WireFrame>,
    },
    /// Sound.
    Sound {
        /// Sound.
        sound: Q2SoundMessage,
    },
    /// Temporary entity.
    TempEntity {
        /// Entity.
        value: Q2TempEntity,
    },
    /// Muzzle flash.
    MuzzleFlash {
        /// Entity.
        entity: i32,
        /// Flash.
        flash: i32,
        /// Monster flash.
        monster: bool,
        /// Silenced.
        silenced: bool,
    },
    /// Inventory.
    Inventory {
        /// Counts.
        counts: Vec<i16>,
    },
    /// Download chunk.
    Download {
        /// Percent complete.
        percent: u8,
        /// Bytes (absent for the terminal marker).
        bytes: Option<Vec<u8>>,
    },
    /// Server setting.
    Setting {
        /// Index.
        index: i32,
        /// Value.
        value: i32,
    },
    /// Split-screen seat.
    Seat {
        /// Seat.
        seat: u8,
    },
    /// KEX damage indicators.
    Damage {
        /// Indicators.
        indicators: Vec<KexDamageIndicator>,
    },
    /// Localized print.
    Locprint {
        /// Value.
        value: KexLocprint,
    },
    /// Fog.
    Fog {
        /// Value.
        value: FogData,
    },
    /// Point of interest.
    Poi {
        /// Value.
        value: KexPoi,
    },
    /// Help path.
    HelpPath {
        /// Value.
        value: KexHelpPath,
    },
    /// Mod-private message.
    Private {
        /// Name.
        name: String,
        /// Payload.
        payload: Vec<u8>,
    },
}

/// Decoded private message payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2PrivateMessage {
    /// Name.
    pub name: String,
    /// Payload.
    pub payload: Vec<u8>,
}

/// Server record (`Q2ServerRecord`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ServerRecord {
    /// Seat.
    pub seat: u8,
    /// Opcode.
    pub opcode: u8,
    /// Raw bytes.
    pub raw: Vec<u8>,
    /// Event.
    pub event: Q2ServerEvent,
}

/// Gamestate stream state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Q2Stream {
    None,
    Config,
    Baseline,
    GamestateConfig,
}

/// Download compression mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Q2DownloadMode {
    None,
    Block,
    Stream,
}

/// Server message reader options.
#[derive(Debug, Clone)]
pub struct Q2ServerMessageOptions {
    /// Read mode.
    pub read_mode: Q2ReadMode,
    /// Configstring space.
    pub max_config_strings: u16,
    /// Inventory slots.
    pub inventory_slots: usize,
    /// Temp-entity dialect override.
    pub q2pro_extended_temp_entities: Option<bool>,
}

impl Default for Q2ServerMessageOptions {
    fn default() -> Self {
        Self {
            read_mode: Q2ReadMode::Network,
            max_config_strings: 2080,
            inventory_slots: 256,
            q2pro_extended_temp_entities: None,
        }
    }
}

/// Server message reader (`Q2ServerMessageReader`).
pub struct Q2ServerMessageReader {
    /// Wire codec.
    pub wire: Q2Wire,
    /// Options.
    pub options: Q2ServerMessageOptions,
    /// Configstrings.
    pub config_strings: HashMap<u16, String>,
    histories: HashMap<u8, Q2FrameHistory>,
    baselines: HashMap<u16, EntityState>,
    selected_seat: u8,
    legacy_demo26: bool,
    stream: Q2Stream,
    compressed_download: Vec<u8>,
    inflated_download_bytes: usize,
    private_opcodes: HashSet<u8>,
    // Donor callback shape; an alias would force a lifetime parameter through
    // the public reader API.
    #[allow(clippy::type_complexity)]
    private_message:
        Option<Box<dyn FnMut(u8, &mut MsgReader<'_>, ProtocolIdentity) -> Result<Q2PrivateMessage, Q2NetError>>>,
}

impl Q2ServerMessageReader {
    /// Build a reader.
    #[allow(clippy::type_complexity)]
    pub fn new(
        protocol: ProtocolIdentity,
        options: Q2ServerMessageOptions,
        private_opcodes: HashSet<u8>,
        private_message: Option<
            Box<dyn FnMut(u8, &mut MsgReader<'_>, ProtocolIdentity) -> Result<Q2PrivateMessage, Q2NetError>>,
        >,
    ) -> Result<Self, Q2NetError> {
        if options.max_config_strings < 1 || options.inventory_slots < 1 || options.inventory_slots > 32768 {
            return Err(Q2NetError::Range("Invalid Q2 message layout limits"));
        }
        Ok(Self {
            wire: Q2Wire::new(protocol)?,
            options,
            config_strings: HashMap::new(),
            histories: HashMap::new(),
            baselines: HashMap::new(),
            selected_seat: 0,
            legacy_demo26: false,
            stream: Q2Stream::None,
            compressed_download: Vec::new(),
            inflated_download_bytes: 0,
            private_opcodes,
            private_message,
        })
    }

    /// Selected seat.
    #[must_use]
    pub fn seat(&self) -> u8 {
        self.selected_seat
    }

    /// Borrowed histories for baselines seeding.
    #[must_use]
    pub fn baselines(&self) -> &HashMap<u16, EntityState> {
        &self.baselines
    }

    /// Accept negotiated Q2Pro features on the embedded wire.
    pub fn accept_q2pro_features(&mut self, revision: u16, flags: u16) -> Result<(), Q2NetError> {
        self.wire.accept_q2pro_features(revision, flags)
    }

    /// Enter decoded records without fabricating datagrams (`acceptDecoded`).
    pub fn accept_decoded(&mut self, records: &[Q2ServerRecord]) -> Result<(), Q2NetError> {
        for record in records {
            if matches!(record.event, Q2ServerEvent::ServerData { .. }) {
                self.reset();
            }
            self.selected_seat = record.seat;
            match &record.event {
                Q2ServerEvent::ConfigString { index, value } => {
                    self.config_strings.insert(*index, value.clone());
                }
                Q2ServerEvent::Baseline { entity } => {
                    self.baselines.insert(entity.number, entity.clone());
                    for history in self.histories.values_mut() {
                        history.baselines.insert(entity.number, entity.clone());
                    }
                }
                Q2ServerEvent::Frame { frame } => {
                    self.history(record.seat).accept(frame.as_ref().clone());
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Latest frame per seat.
    pub fn latest_frames(&self) -> Vec<(u8, Q2WireFrame)> {
        let mut out = Vec::new();
        for (seat, history) in &self.histories {
            if let Some(frame) = history.latest() {
                out.push((*seat, frame.clone()));
            }
        }
        out
    }

    /// Borrow a seat history, seeding baselines on creation.
    pub fn history(&mut self, seat: u8) -> &mut Q2FrameHistory {
        let base = self.baselines.clone();
        let entry = self
            .histories
            .entry(seat)
            .or_insert_with(|| Q2FrameHistory::new(64).expect("nonzero capacity"));
        entry.seed_baselines(&base);
        entry
    }

    /// Reset retained state.
    pub fn reset(&mut self) {
        self.legacy_demo26 = false;
        self.histories.clear();
        self.baselines.clear();
        self.config_strings.clear();
        self.stream = Q2Stream::None;
        self.selected_seat = 0;
        self.compressed_download.clear();
        self.inflated_download_bytes = 0;
    }

    /// Decode one datagram.
    pub fn read(&mut self, bytes: &[u8]) -> Result<Vec<Q2ServerRecord>, Q2NetError> {
        self.wire.begin(bytes);
        let records = self.parse()?;
        self.wire.finish()?;
        Ok(records)
    }

    fn record(&self, opcode: u8, start: usize, event: Q2ServerEvent) -> Q2ServerRecord {
        let seat = if matches!(event, Q2ServerEvent::Frame { .. }) && self.wire.is_kex() {
            0
        } else {
            self.selected_seat
        };
        Q2ServerRecord {
            seat,
            opcode,
            raw: self.wire.raw_slice(start).to_vec(),
            event,
        }
    }

    fn config(&mut self) -> Result<Option<Q2ServerEvent>, Q2NetError> {
        let index = self.wire.with_reader(|reader| Ok(reader.word()?))?;
        if index == self.options.max_config_strings {
            return Ok(None);
        }
        if index > self.options.max_config_strings {
            return Err(Q2NetError::Range("Q2 configstring index exceeds selected layout"));
        }
        let value = self.wire.with_reader(|reader| Ok(reader.string(2047)))?;
        self.config_strings.insert(index, value.clone());
        Ok(Some(Q2ServerEvent::ConfigString { index, value }))
    }

    fn baseline(&mut self) -> Result<Option<Q2ServerEvent>, Q2NetError> {
        let header = self.wire.read_entity_bits()?;
        if header.number == 0 {
            return Ok(None);
        }
        let entity = self
            .wire
            .read_delta_entity(&EntityState::default(), header.number, header)?;
        self.baselines.insert(entity.number, entity.clone());
        for history in self.histories.values_mut() {
            history.baselines.insert(entity.number, entity.clone());
        }
        Ok(Some(Q2ServerEvent::Baseline { entity }))
    }

    fn sound(&mut self) -> Result<Q2SoundMessage, Q2NetError> {
        if self.wire.is_kex() {
            let sound = self.wire.read_kex_sound()?;
            return Ok(Q2SoundMessage {
                flags: sound.flags,
                index: sound.index,
                entity: sound.entity,
                channel: sound.channel,
                position: sound
                    .pos
                    .map(|pos| [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])]),
                volume: sound.volume,
                attenuation: sound.attenuation,
                delay_seconds: sound.timeofs,
            });
        }
        let floating = self.wire.floating_coordinates();
        let int23 = self.wire.q2pro_extended_v2();
        self.wire.with_reader(|reader| {
            let flags = reader.byte()?;
            let index = if (flags & 32) != 0 {
                reader.word()?
            } else {
                u16::from(reader.byte()?)
            };
            let volume = if (flags & 1) != 0 {
                f64::from(reader.byte()?) / 255.0
            } else {
                1.0
            };
            let attenuation = if (flags & 2) != 0 {
                f64::from(reader.byte()?) / 64.0
            } else {
                1.0
            };
            let delay_seconds = if (flags & 16) != 0 {
                f64::from(reader.byte()?) / 1000.0
            } else {
                0.0
            };
            let channel = if (flags & 8) != 0 { u32::from(reader.word()?) } else { 0 };
            let position = if (flags & 4) != 0 {
                Some(read_game_position(reader, floating, int23)?)
            } else {
                None
            };
            Ok(Q2SoundMessage {
                flags,
                index,
                entity: channel >> 3,
                channel: (channel & 7) as u8,
                position,
                volume,
                attenuation,
                delay_seconds,
            })
        })
    }

    fn download(&mut self, mode: Q2DownloadMode) -> Result<Q2ServerEvent, Q2NetError> {
        let (length, percent) = self.wire.with_reader(|reader| Ok((reader.short()?, reader.byte()?)))?;
        if length < 0 {
            self.compressed_download.clear();
            self.inflated_download_bytes = 0;
            return Ok(Q2ServerEvent::Download { percent, bytes: None });
        }
        let expected = if mode == Q2DownloadMode::Block {
            Some(self.wire.with_reader(|reader| Ok(reader.word()?))?)
        } else {
            None
        };
        let length = length as usize;
        if self.wire.remaining() < length {
            return Err(Q2NetError::Protocol("Truncated Q2 download block"));
        }
        let bytes = self.wire.read_raw_data(length)?;
        match mode {
            Q2DownloadMode::None => Ok(Q2ServerEvent::Download {
                percent,
                bytes: Some(bytes),
            }),
            Q2DownloadMode::Block => {
                let expected = expected.unwrap_or(0) as usize;
                let mut decoder = Decompress::new(false);
                let mut inflated = Vec::new();
                decoder
                    .decompress_vec(&bytes, &mut inflated, FlushDecompress::Finish)
                    .map_err(|error| Q2NetError::Variant(VariantError::Zlib(error.to_string())))?;
                // The donor caps output at the header length (or one byte).
                inflated.truncate(expected.max(1).saturating_add(1));
                if inflated.len() != expected {
                    return Err(Q2NetError::Protocol("Q2 download length mismatch"));
                }
                Ok(Q2ServerEvent::Download {
                    percent,
                    bytes: Some(inflated),
                })
            }
            Q2DownloadMode::Stream => {
                self.compressed_download.extend_from_slice(&bytes);
                let mut decoder = Decompress::new(false);
                let mut inflated = Vec::new();
                decoder
                    .decompress_vec(
                        &self.compressed_download,
                        &mut inflated,
                        if percent == 100 {
                            FlushDecompress::Finish
                        } else {
                            FlushDecompress::Sync
                        },
                    )
                    .map_err(|error| Q2NetError::Variant(VariantError::Zlib(error.to_string())))?;
                let delta = inflated[self.inflated_download_bytes.min(inflated.len())..].to_vec();
                self.inflated_download_bytes = inflated.len();
                if percent == 100 {
                    self.compressed_download.clear();
                    self.inflated_download_bytes = 0;
                }
                Ok(Q2ServerEvent::Download {
                    percent,
                    bytes: Some(delta),
                })
            }
        }
    }

    fn parse(&mut self) -> Result<Vec<Q2ServerRecord>, Q2NetError> {
        let mut records = Vec::new();
        let kex = self.wire.is_kex();
        let rerelease = matches!(
            self.wire.protocol(),
            ProtocolIdentity::Q2Rerelease | ProtocolIdentity::Q2PrivateClassic
        );
        while self.wire.remaining() > 0 {
            let start = self.wire.position();
            if self.stream != Q2Stream::None {
                let mode = self.stream;
                let event = if mode == Q2Stream::Baseline {
                    self.baseline()?
                } else {
                    self.config()?
                };
                if let Some(event) = event {
                    let opcode = if mode == Q2Stream::Baseline { 39 } else { 38 };
                    records.push(self.record(opcode, start, event));
                } else {
                    self.stream = if mode == Q2Stream::GamestateConfig {
                        Q2Stream::Baseline
                    } else {
                        Q2Stream::None
                    };
                }
                continue;
            }
            let raw = self.wire.read_raw_byte()?;
            let opcode = self.wire.opcode(raw);
            if self.private_opcodes.contains(&opcode) {
                let Some(mut decode) = self.private_message.take() else {
                    return Err(Q2NetError::Unbound {
                        opcode,
                        protocol: self.wire.protocol(),
                    });
                };
                let protocol = self.wire.protocol();
                let decoded = self.wire.with_reader(|reader| decode(opcode, reader, protocol))?;
                self.private_message = Some(decode);
                records.push(self.record(
                    opcode,
                    start,
                    Q2ServerEvent::Private {
                        name: decoded.name,
                        payload: decoded.payload,
                    },
                ));
                continue;
            }
            let is_r1q2 = matches!(self.wire.protocol(), ProtocolIdentity::Q2R1q2 { .. });
            let is_q2pro = matches!(self.wire.protocol(), ProtocolIdentity::Q2Q2pro { .. });
            if (opcode == 21 && (is_r1q2 || is_q2pro)) || (opcode == 34 && rerelease) {
                let payload = self.wire.read_zpacket_payload()?;
                let saved_data = std::mem::take(&mut self.wire.data);
                let saved_pos = std::mem::replace(&mut self.wire.pos, 0);
                self.wire.data = payload;
                let nested = self.parse();
                let finish = self.wire.finish();
                self.wire.data = saved_data;
                self.wire.pos = saved_pos;
                finish?;
                records.extend(nested?);
                continue;
            }
            match opcode {
                6 => records.push(self.record(opcode, start, Q2ServerEvent::Nop)),
                7 => records.push(self.record(opcode, start, Q2ServerEvent::Disconnect)),
                8 => records.push(self.record(opcode, start, Q2ServerEvent::Reconnect)),
                10 => {
                    let (level, text) = self
                        .wire
                        .with_reader(|reader| Ok((reader.byte()?, reader.string(2047))))?;
                    records.push(self.record(opcode, start, Q2ServerEvent::Print { level, text }));
                }
                11 => {
                    let text = self.wire.with_reader(|reader| Ok(reader.string(2047)))?;
                    records.push(self.record(opcode, start, Q2ServerEvent::CommandText { text }));
                }
                15 => {
                    let text = self.wire.with_reader(|reader| Ok(reader.string(2047)))?;
                    records.push(self.record(opcode, start, Q2ServerEvent::CenterPrint { text }));
                }
                4 => {
                    let text = self.wire.with_reader(|reader| Ok(reader.string(2047)))?;
                    records.push(self.record(opcode, start, Q2ServerEvent::Layout { text }));
                }
                12 => {
                    let version = self.wire.with_reader(|reader| Ok(reader.long()?))?;
                    let legacy_demo26 = version == 26
                        && self.options.read_mode == Q2ReadMode::Demo
                        && self.wire.protocol() == ProtocolIdentity::Q2Classic;
                    if version as u32 != q2_version(self.wire.protocol()) && !legacy_demo26 {
                        return Err(Q2NetError::ServerdataProtocol {
                            found: version as i64,
                            negotiated: q2_version(self.wire.protocol()),
                        });
                    }
                    let data = self.wire.read_server_data()?;
                    let reported = data.r1q2_version().map(i64::from);
                    self.wire.accept_server_revision(reported)?;
                    self.reset();
                    self.legacy_demo26 = legacy_demo26;
                    records.push(self.record(opcode, start, Q2ServerEvent::ServerData { data: Box::new(data) }));
                }
                13 => {
                    let Some(event) = self.config()? else {
                        return Err(Q2NetError::Protocol("Q2 configstring terminator outside stream"));
                    };
                    records.push(self.record(opcode, start, event));
                }
                14 => {
                    let Some(event) = self.baseline()? else {
                        return Err(Q2NetError::Protocol("Q2 zero spawn baseline"));
                    };
                    records.push(self.record(opcode, start, event));
                }
                20 => {
                    let seat = if kex { 0 } else { self.seat() };
                    let suppress = !self.legacy_demo26;
                    let base = self.baselines.clone();
                    let entry = self
                        .histories
                        .entry(seat)
                        .or_insert_with(|| Q2FrameHistory::new(64).expect("nonzero capacity"));
                    entry.seed_baselines(&base);
                    let frame = entry.read(&mut self.wire, suppress)?;
                    records.push(self.record(opcode, start, Q2ServerEvent::Frame { frame: Box::new(frame) }));
                }
                9 => {
                    let sound = self.sound()?;
                    records.push(self.record(opcode, start, Q2ServerEvent::Sound { sound }));
                }
                3 => {
                    let floating = self.wire.floating_coordinates();
                    let extended = self.options.q2pro_extended_temp_entities.unwrap_or_else(|| {
                        matches!(self.wire.protocol(), ProtocolIdentity::Q2Q2pro { .. }) && self.wire.q2pro_extended()
                    });
                    let int23 = self.wire.q2pro_extended_v2();
                    let value = self
                        .wire
                        .with_reader(|reader| read_temp_entity(reader, floating, extended, int23))?;
                    records.push(self.record(opcode, start, Q2ServerEvent::TempEntity { value }));
                }
                1 | 2 => {
                    let (mut entity, mut flash) = self
                        .wire
                        .with_reader(|reader| Ok((i32::from(reader.word()?), i32::from(reader.byte()?))))?;
                    let monster = opcode == 2;
                    let silenced = !monster && (flash & 128) != 0;
                    if !monster {
                        flash &= 127;
                    }
                    if monster && (rerelease || self.wire.q2pro_extended()) {
                        flash |= ((entity & 0xe000) >> 5) & 0x700;
                        entity &= 0x1fff;
                    }
                    records.push(self.record(
                        opcode,
                        start,
                        Q2ServerEvent::MuzzleFlash {
                            entity,
                            flash,
                            monster,
                            silenced,
                        },
                    ));
                }
                5 => {
                    let slots = self.options.inventory_slots;
                    let counts = self.wire.with_reader(|reader| {
                        let mut counts = Vec::with_capacity(slots);
                        for _ in 0..slots {
                            counts.push(reader.short()?);
                        }
                        Ok(counts)
                    })?;
                    records.push(self.record(opcode, start, Q2ServerEvent::Inventory { counts }));
                }
                16 => {
                    let event = self.download(Q2DownloadMode::None)?;
                    records.push(self.record(opcode, start, event));
                }
                21 => {
                    if !kex {
                        return Err(Q2NetError::Protocol("Unexpected Q2 splitclient"));
                    }
                    let seat = self.wire.with_reader(|reader| Ok(reader.byte()?))?;
                    self.selected_seat = seat;
                    records.push(self.record(opcode, start, Q2ServerEvent::Seat { seat }));
                }
                22 => {
                    if !kex {
                        let revision = self.wire.q2pro_revision();
                        let mode = if is_q2pro && revision >= 1021 {
                            Q2DownloadMode::Stream
                        } else {
                            Q2DownloadMode::Block
                        };
                        let event = self.download(mode)?;
                        records.push(self.record(opcode, start, event));
                        continue;
                    }
                    for item in self.wire.read_kex_configblast()? {
                        if item.index >= self.options.max_config_strings {
                            return Err(Q2NetError::Protocol("KEX configblast index exceeds selected layout"));
                        }
                        self.config_strings.insert(item.index, item.value.clone());
                        records.push(self.record(
                            opcode,
                            start,
                            Q2ServerEvent::ConfigString {
                                index: item.index,
                                value: item.value,
                            },
                        ));
                    }
                }
                23 => {
                    if !kex {
                        self.stream = Q2Stream::GamestateConfig;
                        continue;
                    }
                    for item in self.wire.read_kex_spawnbaselineblast()? {
                        self.baselines.insert(item.entnum, item.state.clone());
                        for history in self.histories.values_mut() {
                            history.baselines.insert(item.entnum, item.state.clone());
                        }
                        records.push(self.record(opcode, start, Q2ServerEvent::Baseline { entity: item.state }));
                    }
                }
                24 => {
                    if kex {
                        records.push(self.record(opcode, start, Q2ServerEvent::LevelRestart));
                    } else {
                        let (index, value) = self.wire.with_reader(|reader| Ok((reader.long()?, reader.long()?)))?;
                        records.push(self.record(opcode, start, Q2ServerEvent::Setting { index, value }));
                    }
                }
                25 => {
                    if !kex && !rerelease {
                        self.stream = Q2Stream::Config;
                        continue;
                    }
                    let indicators = self.wire.read_kex_damage()?;
                    records.push(self.record(opcode, start, Q2ServerEvent::Damage { indicators }));
                }
                26 => {
                    if !kex && !rerelease {
                        self.stream = Q2Stream::Baseline;
                        continue;
                    }
                    let value = self.wire.read_kex_locprint()?;
                    records.push(self.record(opcode, start, Q2ServerEvent::Locprint { value }));
                }
                27 => {
                    let value = self.wire.read_fog()?;
                    records.push(self.record(opcode, start, Q2ServerEvent::Fog { value }));
                }
                30 => {
                    let value = self.wire.read_kex_poi()?;
                    records.push(self.record(opcode, start, Q2ServerEvent::Poi { value }));
                }
                31 => {
                    let value = self.wire.read_kex_help_path()?;
                    records.push(self.record(opcode, start, Q2ServerEvent::HelpPath { value }));
                }
                32 => {
                    let flash = self.wire.read_kex_muzzleflash3()?;
                    records.push(self.record(
                        opcode,
                        start,
                        Q2ServerEvent::MuzzleFlash {
                            entity: i32::from(flash.entity),
                            flash: i32::from(flash.weapon),
                            monster: true,
                            silenced: false,
                        },
                    ));
                }
                33 => {
                    let text = self.wire.read_kex_achievement()?;
                    records.push(self.record(opcode, start, Q2ServerEvent::Achievement { text }));
                }
                35 => {
                    if !rerelease {
                        return Err(Q2NetError::Unbound {
                            opcode,
                            protocol: self.wire.protocol(),
                        });
                    }
                    let event = self.download(Q2DownloadMode::Stream)?;
                    records.push(self.record(opcode, start, event));
                }
                36 => {
                    if !rerelease {
                        return Err(Q2NetError::Unbound {
                            opcode,
                            protocol: self.wire.protocol(),
                        });
                    }
                    self.stream = Q2Stream::GamestateConfig;
                }
                37 => {
                    if !rerelease {
                        return Err(Q2NetError::Unbound {
                            opcode,
                            protocol: self.wire.protocol(),
                        });
                    }
                    let (index, value) = self.wire.with_reader(|reader| Ok((reader.long()?, reader.long()?)))?;
                    records.push(self.record(opcode, start, Q2ServerEvent::Setting { index, value }));
                }
                38 => {
                    if !rerelease {
                        return Err(Q2NetError::Unbound {
                            opcode,
                            protocol: self.wire.protocol(),
                        });
                    }
                    self.stream = Q2Stream::Config;
                }
                39 => {
                    if !rerelease {
                        return Err(Q2NetError::Unbound {
                            opcode,
                            protocol: self.wire.protocol(),
                        });
                    }
                    self.stream = Q2Stream::Baseline;
                }
                _ => {
                    let Some(mut decode) = self.private_message.take() else {
                        return Err(Q2NetError::Unbound {
                            opcode,
                            protocol: self.wire.protocol(),
                        });
                    };
                    let protocol = self.wire.protocol();
                    let decoded = self.wire.with_reader(|reader| decode(opcode, reader, protocol))?;
                    self.private_message = Some(decode);
                    records.push(self.record(
                        opcode,
                        start,
                        Q2ServerEvent::Private {
                            name: decoded.name,
                            payload: decoded.payload,
                        },
                    ));
                }
            }
        }
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel_options(side: ChannelSide) -> Q2ChannelOptions {
        Q2ChannelOptions {
            side,
            protocol: ProtocolIdentity::Q2Classic,
            channel: ChannelKind::Old,
            qport: 27960,
            payload_bytes: None,
            message_bytes: None,
            max_datagram_bytes: None,
            compress: false,
            sequence_recording: None,
        }
    }

    #[test]
    fn out_of_band_round_trip() {
        let bytes = q2_out_of_band("getchallenge", false);
        assert_eq!(&bytes[..4], &[255, 255, 255, 255]);
        let message = read_q2_out_of_band(&bytes, false).expect("message");
        assert_eq!(message.command, "getchallenge");
        assert!(read_q2_out_of_band(&[1, 2, 3], false).is_none());
    }

    #[test]
    fn connect_request_round_trip() {
        let request = Q2ConnectRequest {
            protocol: ProtocolIdentity::Q2Classic,
            qport: 27960,
            challenge: 1234,
            userinfo: "\\name\\t".to_string(),
            payload_bytes: 1390,
            channel: ChannelKind::Old,
            compression: false,
            social_ids: None,
        };
        let bytes = write_q2_connect(&request).unwrap();
        let message = read_q2_out_of_band(&bytes, false).expect("oob");
        let back = read_q2_connect(&message).unwrap();
        assert_eq!(back.protocol, ProtocolIdentity::Q2Classic);
        assert_eq!(back.challenge, 1234);
        assert_eq!(back.userinfo, "\\name\\t");
    }

    #[test]
    fn challenge_parses_versions() {
        let bytes = q2_out_of_band("challenge 777 p=34,35", false);
        let message = read_q2_out_of_band(&bytes, false).expect("oob");
        let challenge = read_q2_challenge(&message).unwrap();
        assert_eq!(challenge.challenge, 777);
        assert_eq!(challenge.versions, vec![34, 35]);
    }

    #[test]
    fn channel_loopback_unreliable() {
        let mut client = Q2Channel::new(channel_options(ChannelSide::Client)).unwrap();
        let mut server = Q2Channel::new(channel_options(ChannelSide::Server)).unwrap();
        let packet = client.transmit(&[9, 8, 7], 100).unwrap();
        let received = server.receive(&packet, 100).unwrap();
        let Q2ChannelReceive::Message { sequence, bytes, .. } = received else {
            panic!("expected message, got {received:?}");
        };
        assert_eq!(sequence, 1);
        assert_eq!(bytes, vec![9, 8, 7]);
        let reply = server.transmit(&[1], 101).unwrap();
        let back = client.receive(&reply, 101).unwrap();
        assert!(matches!(back, Q2ChannelReceive::Message { .. }));
    }

    #[test]
    fn channel_reliable_queue_and_ack() {
        let mut client = Q2Channel::new(channel_options(ChannelSide::Client)).unwrap();
        let mut server = Q2Channel::new(channel_options(ChannelSide::Server)).unwrap();
        client.queue_reliable(&[5, 6]).unwrap();
        assert!(client.reliable_pending());
        let packet = client.transmit(&[], 100).unwrap();
        let received = server.receive(&packet, 100).unwrap();
        assert!(matches!(received, Q2ChannelReceive::Message { .. }));
    }

    #[test]
    fn handshake_challenge_connect_flow() {
        let remote = ipv4_address([127, 0, 0, 1], 27910, false).unwrap();
        let mut handshake = Q2ClientHandshake::new(
            remote.clone(),
            vec![ProtocolIdentity::Q2Classic],
            27960,
            || "\\name\\t".to_string(),
            1390,
            1000,
            String::new,
        )
        .unwrap();
        assert!(matches!(handshake.state(), Q2ClientHandshakeState::Challenging { .. }));
        let poll = handshake.poll(0).unwrap().expect("challenge poll");
        let message = read_q2_out_of_band(&poll, false).expect("oob");
        assert_eq!(message.command, "getchallenge");
        let reply = q2_out_of_band("challenge 555 p=34", false);
        let message = read_q2_out_of_band(&reply, false).expect("oob");
        assert!(handshake.receive(&remote, &message).unwrap());
        assert!(matches!(handshake.state(), Q2ClientHandshakeState::Connecting { .. }));
        let poll = handshake.poll(100).unwrap().expect("connect poll");
        let message = read_q2_out_of_band(&poll, false).expect("oob");
        assert_eq!(message.command, "connect");
        let accept = q2_out_of_band("client_connect", false);
        let message = read_q2_out_of_band(&accept, false).expect("oob");
        assert!(handshake.receive(&remote, &message).unwrap());
        assert!(matches!(handshake.state(), Q2ClientHandshakeState::Connected { .. }));
    }

    #[test]
    fn protocol_negotiation() {
        assert_eq!(q2_protocol(34, 0).unwrap(), ProtocolIdentity::Q2Classic);
        q2_codec_support(ProtocolIdentity::Q2Classic).unwrap();
        assert_eq!(negotiated_r1q2_protocol(1905, Some(1904)).unwrap(), 1904);
        assert!(negotiated_r1q2_protocol(1905, Some(1800)).is_err());
    }
}
