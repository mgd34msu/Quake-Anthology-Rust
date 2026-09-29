//! KEX LAN wire packets.
//!
//! Donor provenance: `KEX_DATAGRAM_BYTES`, `KEX_MESSAGE_BYTES`,
//! `KexReader`, `KexWriter`, `KexPacket`, `readKexPacket`,
//! `writeKexPacket`, `kexText`, and `readKexText` in
//! `src/network/q2/kex/packet.ts` (wire layout recovered from
//! `quake2ex_steam.exe`). Layouts are byte-exact; [`KexError`] is shared
//! by the KEX channel, discovery, and LAN modules.

use thiserror::Error;

use crate::common::endpoint::AddressError;
use crate::common::transport::TransportError;

/// Maximum KEX datagram bytes.
pub const KEX_DATAGRAM_BYTES: usize = 1400;
/// Maximum reassembled KEX message bytes.
pub const KEX_MESSAGE_BYTES: usize = 1024 * 1024;

/// KEX failure shared by the packet, channel, discovery, and LAN modules.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KexError {
    /// Wire or protocol violation, with the donor's message text.
    #[error("{0}")]
    Protocol(&'static str),
    /// Failure with dynamic text.
    #[error("{0}")]
    Message(String),
    /// Underlying transport failure.
    #[error("{0}")]
    Transport(#[from] TransportError),
    /// Underlying address failure.
    #[error("{0}")]
    Address(#[from] AddressError),
}

/// KEX message reader (`KexReader`).
#[derive(Debug)]
pub struct KexReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> KexReader<'a> {
    /// Read from borrowed bytes.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    /// Unread bytes.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// Read one byte (`byte`).
    pub fn byte(&mut self) -> Result<u8, KexError> {
        let Some(value) = self.bytes.get(self.position).copied() else {
            return Err(KexError::Protocol("Truncated KEX message"));
        };
        self.position += 1;
        Ok(value)
    }

    /// Read a little-endian base-128 integer (`integer`).
    pub fn integer(&mut self) -> Result<u64, KexError> {
        let mut value = 0u64;
        for shift in (0u32..70).step_by(7) {
            let byte = self.byte()?;
            if shift == 63 && byte > 1 {
                return Err(KexError::Protocol("Invalid KEX integer"));
            }
            value |= u64::from(byte & 127) << shift;
            if byte & 128 == 0 {
                return Ok(value);
            }
        }
        Err(KexError::Protocol("Invalid KEX integer"))
    }

    /// Read a length-prefixed UTF-8 string (`string`).
    pub fn string(&mut self) -> Result<String, KexError> {
        let length = self.integer()?;
        if length > self.remaining() as u64 {
            return Err(KexError::Protocol("Truncated KEX string"));
        }
        let start = self.position;
        self.position += length as usize;
        std::str::from_utf8(&self.bytes[start..self.position])
            .map(str::to_owned)
            .map_err(|_| KexError::Protocol("Invalid KEX string"))
    }

    /// Require end of input (`end`).
    pub fn end(&self) -> Result<(), KexError> {
        if self.remaining() != 0 {
            return Err(KexError::Protocol("Trailing KEX data"));
        }
        Ok(())
    }
}

/// KEX message writer (`KexWriter`).
#[derive(Debug, Clone, Default)]
pub struct KexWriter {
    bytes: Vec<u8>,
}

impl KexWriter {
    /// Fresh writer.
    pub fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// Append a byte (`byte`); the `u8` type enforces the donor's range.
    pub fn byte(&mut self, value: u8) -> &mut Self {
        self.bytes.push(value);
        self
    }

    /// Append a base-128 integer (`integer`); the `u64` type enforces the
    /// donor's `0..=0xffff_ffff_ffff_ffff` range.
    pub fn integer(&mut self, mut value: u64) -> &mut Self {
        loop {
            let byte = (value & 127) as u8;
            value >>= 7;
            self.bytes.push(byte | if value == 0 { 0 } else { 128 });
            if value == 0 {
                break;
            }
        }
        self
    }

    /// Append a length-prefixed UTF-8 string (`string`).
    pub fn string(&mut self, value: &str) -> &mut Self {
        self.integer(value.len() as u64);
        self.bytes.extend_from_slice(value.as_bytes());
        self
    }

    /// Written bytes (`finish`).
    #[must_use]
    pub fn finish(&self) -> Vec<u8> {
        self.bytes.clone()
    }
}

/// KEX datagram (`KexPacket`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KexPacket {
    /// Header flags.
    pub flags: u8,
    /// Sequence (sequenced packets only).
    pub sequence: u16,
    /// Reliable counter (sequenced packets only).
    pub reliable: u16,
    /// Message kind (`None` for continuation fragments).
    pub kind: Option<u8>,
    /// Payload bytes.
    pub payload: Vec<u8>,
}

/// Read a KEX datagram (`readKexPacket`).
pub fn read_kex_packet(bytes: &[u8]) -> Result<KexPacket, KexError> {
    if bytes.len() < 3 || bytes.len() > KEX_DATAGRAM_BYTES {
        return Err(KexError::Protocol("Invalid KEX datagram size"));
    }
    let header = u16::from_be_bytes([bytes[0], bytes[1]]);
    if (header >> 4) as usize != bytes.len() {
        return Err(KexError::Protocol("Invalid KEX datagram length"));
    }
    let flags = (header & 15) as u8;
    let sequential = flags & 2 != 0;
    let continued = flags & 8 != 0;
    let offset = if sequential { 6 } else { 2 };
    if (flags & 1 != 0 && !sequential) || bytes.len() < offset + usize::from(!continued as u8) {
        return Err(KexError::Protocol("Invalid KEX datagram flags"));
    }
    let (sequence, reliable) = if sequential {
        (
            u16::from_be_bytes([bytes[2], bytes[3]]),
            u16::from_be_bytes([bytes[4], bytes[5]]),
        )
    } else {
        (0, 0)
    };
    let (kind, body) = if continued {
        (None, offset)
    } else {
        (Some(bytes[offset]), offset + 1)
    };
    Ok(KexPacket {
        flags,
        sequence,
        reliable,
        kind,
        payload: bytes[body..].to_vec(),
    })
}

/// Write a KEX datagram (`writeKexPacket`).
pub fn write_kex_packet(packet: &KexPacket) -> Result<Vec<u8>, KexError> {
    let sequential = packet.flags & 2 != 0;
    let continued = packet.flags & 8 != 0;
    let offset = if sequential { 6usize } else { 2 };
    let length = offset + usize::from(!continued as u8) + packet.payload.len();
    if length > KEX_DATAGRAM_BYTES || packet.flags > 15 || continued != packet.kind.is_none() {
        return Err(KexError::Protocol("Invalid KEX packet"));
    }
    let mut bytes = vec![0u8; length];
    bytes[0..2].copy_from_slice(&((length as u16) << 4 | u16::from(packet.flags)).to_be_bytes());
    if sequential {
        bytes[2..4].copy_from_slice(&packet.sequence.to_be_bytes());
        bytes[4..6].copy_from_slice(&packet.reliable.to_be_bytes());
    }
    if let Some(kind) = packet.kind {
        bytes[offset] = kind;
    }
    let body = offset + usize::from(!continued as u8);
    bytes[body..].copy_from_slice(&packet.payload);
    Ok(bytes)
}

/// Encode lobby text (`kexText`).
#[must_use]
pub fn kex_text(value: &str) -> Vec<u8> {
    value.as_bytes().to_vec()
}

/// Decode lobby text (`readKexText`).
pub fn read_kex_text(value: &[u8]) -> Result<String, KexError> {
    std::str::from_utf8(value)
        .map(str::to_owned)
        .map_err(|_| KexError::Protocol("Invalid KEX text"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequenced_packet_is_byte_exact() {
        let bytes = write_kex_packet(&KexPacket {
            flags: 3,
            sequence: 0x1234,
            reliable: 0x5678,
            kind: Some(5),
            payload: vec![1, 2, 3],
        })
        .unwrap();
        assert_eq!(bytes, vec![0, (10 << 4) | 3, 0x12, 0x34, 0x56, 0x78, 5, 1, 2, 3]);
        assert_eq!(
            read_kex_packet(&bytes).unwrap(),
            KexPacket {
                flags: 3,
                sequence: 0x1234,
                reliable: 0x5678,
                kind: Some(5),
                payload: vec![1, 2, 3],
            }
        );
    }

    #[test]
    fn unsequenced_packet_is_byte_exact() {
        let bytes = write_kex_packet(&KexPacket {
            flags: 0,
            sequence: 0,
            reliable: 0,
            kind: Some(129),
            payload: Vec::new(),
        })
        .unwrap();
        assert_eq!(bytes, vec![0, 48, 129]);
        let packet = read_kex_packet(&bytes).unwrap();
        assert_eq!(packet.kind, Some(129));
        assert!(packet.payload.is_empty());
    }

    #[test]
    fn continuation_packet_has_no_kind() {
        let bytes = write_kex_packet(&KexPacket {
            flags: 0x0A,
            sequence: 7,
            reliable: 8,
            kind: None,
            payload: vec![9],
        })
        .unwrap();
        assert_eq!(bytes, vec![0, (7 << 4) | 10, 0, 7, 0, 8, 9]);
        let packet = read_kex_packet(&bytes).unwrap();
        assert_eq!(packet.kind, None);
        assert_eq!(packet.payload, vec![9]);
    }

    #[test]
    fn malformed_packets_fail() {
        assert_eq!(
            read_kex_packet(&[0, 48]),
            Err(KexError::Protocol("Invalid KEX datagram size"))
        );
        assert_eq!(
            read_kex_packet(&[0, 64, 129]),
            Err(KexError::Protocol("Invalid KEX datagram length"))
        );
        // Reliable without sequential.
        assert_eq!(
            read_kex_packet(&[0, (4 << 4) | 1, 5, 0]),
            Err(KexError::Protocol("Invalid KEX datagram flags"))
        );
        // Kind byte missing.
        assert_eq!(
            read_kex_packet(&[0, (6 << 4) | 2, 0, 1, 0, 2]),
            Err(KexError::Protocol("Invalid KEX datagram flags"))
        );
        assert_eq!(
            write_kex_packet(&KexPacket {
                flags: 16,
                sequence: 0,
                reliable: 0,
                kind: Some(1),
                payload: Vec::new()
            }),
            Err(KexError::Protocol("Invalid KEX packet"))
        );
        assert_eq!(
            write_kex_packet(&KexPacket {
                flags: 8,
                sequence: 0,
                reliable: 0,
                kind: Some(1),
                payload: Vec::new()
            }),
            Err(KexError::Protocol("Invalid KEX packet"))
        );
    }

    #[test]
    fn integers_use_base128() {
        for (value, bytes) in [
            (0u64, vec![0]),
            (127, vec![127]),
            (128, vec![0x80, 0x01]),
            (300, vec![0xAC, 0x02]),
            (u64::MAX, vec![0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01]),
        ] {
            let mut writer = KexWriter::new();
            writer.integer(value);
            assert_eq!(writer.finish(), bytes, "value {value}");
            let mut reader = KexReader::new(&bytes);
            assert_eq!(reader.integer().unwrap(), value);
            reader.end().unwrap();
        }
    }

    #[test]
    fn malformed_integers_fail() {
        // Tenth byte carries more than bit 63.
        let mut reader = KexReader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02]);
        assert_eq!(reader.integer(), Err(KexError::Protocol("Invalid KEX integer")));
        // Eleventh byte never terminates.
        let mut reader = KexReader::new(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00]);
        assert_eq!(reader.integer(), Err(KexError::Protocol("Invalid KEX integer")));
        let mut reader = KexReader::new(&[0x80]);
        assert_eq!(reader.integer(), Err(KexError::Protocol("Truncated KEX message")));
    }

    #[test]
    fn strings_round_trip() {
        let mut writer = KexWriter::new();
        writer.byte(7).string("héllo").integer(300);
        let bytes = writer.finish();
        let mut reader = KexReader::new(&bytes);
        assert_eq!(reader.byte().unwrap(), 7);
        assert_eq!(reader.string().unwrap(), "héllo");
        assert_eq!(reader.integer().unwrap(), 300);
        reader.end().unwrap();
    }

    #[test]
    fn truncated_string_fails() {
        let mut reader = KexReader::new(&[5, b'a', b'b']);
        assert_eq!(reader.string(), Err(KexError::Protocol("Truncated KEX string")));
        let mut reader = KexReader::new(&[1, 2]);
        reader.byte().unwrap();
        assert_eq!(reader.end(), Err(KexError::Protocol("Trailing KEX data")));
    }

    #[test]
    fn lobby_text_round_trip() {
        assert_eq!(kex_text("map\\q2dm1"), b"map\\q2dm1".to_vec());
        assert_eq!(read_kex_text(b"a\\b").unwrap(), "a\\b");
        assert_eq!(read_kex_text(&[0xFF]), Err(KexError::Protocol("Invalid KEX text")));
    }
}
