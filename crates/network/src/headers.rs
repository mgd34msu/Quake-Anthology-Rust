//! Native connected-packet layouts. Sequencing, ACK validation and fragment
//! assembly belong to the channel; this codec returns a borrowed payload.
use crate::message::{self, Encoding, Reader, Writer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    ToServer,
    ToClient,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QPort {
    None,
    Byte,
    Short,
}
impl QPort {
    const fn bytes(self) -> usize {
        match self {
            Self::None => 0,
            Self::Byte => 1,
            Self::Short => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FragmentLayout {
    None,
    Length16,
    MoreBit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    datagram: bool,
    acknowledgement: bool,
    qport: QPort,
    sequence_mask: u32,
    reliable_bit: u32,
    fragment_bit: u32,
    fragments: FragmentLayout,
}

/// WinQuake net.h NETFLAG values, also used by negotiated 666/999 datagrams.
pub mod datagram {
    pub const DATA: u32 = 0x0001_0000;
    pub const ACK: u32 = 0x0002_0000;
    pub const NAK: u32 = 0x0004_0000;
    pub const EOM: u32 = 0x0008_0000;
    pub const UNRELIABLE: u32 = 0x0010_0000;
    pub const CONTROL: u32 = 0x8000_0000;
}

pub const NETQUAKE: Format = Format {
    datagram: true,
    acknowledgement: false,
    qport: QPort::None,
    sequence_mask: u32::MAX,
    reliable_bit: 0,
    fragment_bit: 0,
    fragments: FragmentLayout::None,
};
pub const QUAKEWORLD: Format = q2_old(QPort::Short);
pub const QUAKE2: Format = QUAKEWORLD;
pub const QUAKE3: Format = Format {
    datagram: false,
    acknowledgement: false,
    qport: QPort::Short,
    sequence_mask: 0x7fff_ffff,
    reliable_bit: 0,
    fragment_bit: 1 << 31,
    fragments: FragmentLayout::Length16,
};

/// q2repro old-style negotiation: native 34 uses a short; R1Q2 and later
/// select one byte when qport is nonzero, or omit it when zero.
pub const fn q2_old(qport: QPort) -> Format {
    Format {
        datagram: false,
        acknowledgement: true,
        qport,
        sequence_mask: 0x7fff_ffff,
        reliable_bit: 1 << 31,
        fragment_bit: 0,
        fragments: FragmentLayout::None,
    }
}
/// q2repro new-style negotiation adds bit30 fragmentation and the offset's
/// bit15 continuation marker. It does not use Q3's fragment-length word.
pub const fn q2_new(qport_present: bool) -> Format {
    Format {
        datagram: false,
        acknowledgement: true,
        qport: if qport_present {
            QPort::Byte
        } else {
            QPort::None
        },
        sequence_mask: 0x3fff_ffff,
        reliable_bit: 1 << 31,
        fragment_bit: 1 << 30,
        fragments: FragmentLayout::MoreBit,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fragment {
    pub offset: u16,
    /// Q3 derives this from a 1300-byte payload; Q2pro carries it explicitly.
    pub more: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub sequence: u32,
    pub acknowledgement: u32,
    pub reliable: bool,
    pub reliable_ack: bool,
    pub qport: u16,
    pub datagram_flags: u32,
    pub fragment: Option<Fragment>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Message(message::Error),
    Connectionless,
    Length,
    Fragment,
}
impl From<message::Error> for Error {
    fn from(error: message::Error) -> Self {
        Self::Message(error)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native header {self:?}")
    }
}
impl std::error::Error for Error {}

impl Format {
    fn port(self, direction: Direction) -> QPort {
        if direction == Direction::ToServer {
            self.qport
        } else {
            QPort::None
        }
    }
    pub fn size(self, direction: Direction, fragmented: bool) -> Result<usize, Error> {
        let extra = if fragmented {
            match self.fragments {
                FragmentLayout::None => return Err(Error::Fragment),
                FragmentLayout::Length16 => 4,
                FragmentLayout::MoreBit => 2,
            }
        } else {
            0
        };
        Ok(4 + 4 * usize::from(self.datagram || self.acknowledgement)
            + self.port(direction).bytes()
            + extra)
    }
}

/// The caller supplies its fixed transport buffer and outgoing size limit.
/// Receive admission may be larger for a legacy peer (QW can emit 1460 bytes).
pub fn encode(
    format: Format,
    direction: Direction,
    header: Header,
    payload: &[u8],
    output: &mut [u8],
) -> Result<usize, Error> {
    let size = format.size(direction, header.fragment.is_some())?;
    let total = size.checked_add(payload.len()).ok_or(Error::Length)?;
    if total > output.len() || (format.datagram && total > 0xffff) {
        return Err(Error::Length);
    }
    if format.datagram && header.datagram_flags & datagram::CONTROL != 0 {
        return Err(Error::Connectionless);
    }
    if header.fragment.is_some_and(|f| f.offset > 0x7fff)
        || (format.fragments == FragmentLayout::Length16
            && header.fragment.is_some()
            && payload.len() > 0x7fff)
    {
        return Err(Error::Fragment);
    }
    let mut writer = Writer::new(&mut output[..size], Encoding::Bytes);
    if format.datagram {
        writer.write_bits(
            ((header.datagram_flags & 0xffff_0000) | total as u32).swap_bytes(),
            32,
        )?;
    }
    let sequence = (header.sequence & format.sequence_mask)
        | if header.reliable {
            format.reliable_bit
        } else {
            0
        }
        | if header.fragment.is_some() {
            format.fragment_bit
        } else {
            0
        };
    writer.write_bits(
        if format.datagram {
            sequence.swap_bytes()
        } else {
            sequence
        },
        32,
    )?;
    if format.acknowledgement {
        writer.write_bits(
            (header.acknowledgement & format.sequence_mask)
                | if header.reliable_ack {
                    format.reliable_bit
                } else {
                    0
                },
            32,
        )?;
    }
    let port = format.port(direction);
    if port != QPort::None {
        writer.write_bits(u32::from(header.qport), (port.bytes() * 8) as u8)?;
    }
    if let Some(fragment) = header.fragment {
        let offset = fragment.offset
            | if format.fragments == FragmentLayout::MoreBit && fragment.more {
                0x8000
            } else {
                0
            };
        writer.write_bits(u32::from(offset), 16)?;
        if format.fragments == FragmentLayout::Length16 {
            writer.write_bits(payload.len() as u32, 16)?;
        }
    }
    output[size..total].copy_from_slice(payload);
    Ok(total)
}

pub fn decode(
    format: Format,
    direction: Direction,
    packet: &[u8],
) -> Result<(Header, &[u8]), Error> {
    let mut reader = Reader::new(packet, Encoding::Bytes);
    let first = reader.read_bits(32)?;
    let mut end = packet.len();
    let mut header = Header::default();
    let sequence = if format.datagram {
        let first = first.swap_bytes();
        if first & datagram::CONTROL != 0 {
            return Err(Error::Connectionless);
        }
        end = (first & 0xffff) as usize;
        if !(8..=packet.len()).contains(&end) {
            return Err(Error::Length);
        }
        header.datagram_flags = first & 0xffff_0000;
        reader.read_bits(32)?.swap_bytes()
    } else {
        if first == u32::MAX {
            return Err(Error::Connectionless);
        }
        first
    };
    header.sequence = sequence & format.sequence_mask;
    header.reliable = sequence & format.reliable_bit != 0;
    if format.acknowledgement {
        let acknowledgement = reader.read_bits(32)?;
        header.acknowledgement = acknowledgement & format.sequence_mask;
        header.reliable_ack = acknowledgement & format.reliable_bit != 0;
    }
    let port = format.port(direction);
    if port != QPort::None {
        header.qport = reader.read_bits((port.bytes() * 8) as u8)? as u16;
    }
    if sequence & format.fragment_bit != 0 {
        let offset = reader.read_bits(16)? as u16;
        let fragment = match format.fragments {
            FragmentLayout::Length16 => {
                let length = reader.read_bits(16)? as usize;
                if offset > 0x7fff || length > 0x7fff {
                    return Err(Error::Fragment);
                }
                end = reader
                    .byte_position()
                    .checked_add(length)
                    .ok_or(Error::Length)?;
                Fragment {
                    offset,
                    more: length == 1300,
                }
            }
            FragmentLayout::MoreBit => Fragment {
                offset: offset & 0x7fff,
                more: offset & 0x8000 != 0,
            },
            FragmentLayout::None => return Err(Error::Fragment),
        };
        header.fragment = Some(fragment);
    }
    let payload = packet
        .get(reader.byte_position()..end)
        .ok_or(Error::Length)?;
    Ok((header, payload))
}
