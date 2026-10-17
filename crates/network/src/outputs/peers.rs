//! Native scoreboard updates borrow the accepted message until intake returns.
use super::{Error, Protocol, read_text};
use crate::message::{Encoding, Reader, Writer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeerUpdate<'a> {
    Name {
        slot: u8,
        name: &'a [u8],
    },
    Frags {
        slot: u8,
        frags: i16,
    },
    Colors {
        slot: u8,
        colors: u8,
    },
    UserInfo {
        slot: u8,
        user_id: i32,
        info: &'a [u8],
    },
    SetInfo {
        slot: u8,
        key: &'a [u8],
        value: &'a [u8],
    },
}
impl PeerUpdate<'_> {
    pub fn slot(self) -> u8 {
        match self {
            Self::Name { slot, .. }
            | Self::Frags { slot, .. }
            | Self::Colors { slot, .. }
            | Self::UserInfo { slot, .. }
            | Self::SetInfo { slot, .. } => slot,
        }
    }
}

fn opcode(protocol: Protocol, update: PeerUpdate<'_>) -> Option<u32> {
    if !matches!(protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28) {
        return None;
    }
    if protocol == Protocol::QuakeWorld28
        && u32::from(update.slot()) >= protocol.resource_layout().players.count
    {
        return None;
    }
    match (protocol, update) {
        (Protocol::NetQuake15, PeerUpdate::Name { .. }) => Some(13),
        (_, PeerUpdate::Frags { .. }) => Some(14),
        (Protocol::NetQuake15, PeerUpdate::Colors { .. }) => Some(17),
        (Protocol::QuakeWorld28, PeerUpdate::UserInfo { .. }) => Some(40),
        (Protocol::QuakeWorld28, PeerUpdate::SetInfo { .. }) => Some(51),
        _ => None,
    }
}

pub(super) fn read(protocol: Protocol, bytes: &[u8]) -> Result<(PeerUpdate<'_>, usize), Error> {
    let mut reader = Reader::new(bytes, Encoding::Bytes);
    let service = reader.read_bits(8)?;
    let slot = reader.read_bits(8)? as u8;
    let update = match (protocol, service) {
        (Protocol::NetQuake15, 13) => PeerUpdate::Name {
            slot,
            name: read_text(protocol, &mut reader)?,
        },
        (Protocol::NetQuake15 | Protocol::QuakeWorld28, 14) => PeerUpdate::Frags {
            slot,
            frags: reader.read_signed(16)? as i16,
        },
        (Protocol::NetQuake15, 17) => PeerUpdate::Colors {
            slot,
            colors: reader.read_bits(8)? as u8,
        },
        (Protocol::QuakeWorld28, 40) => PeerUpdate::UserInfo {
            slot,
            user_id: reader.read_signed(32)?,
            info: read_text(protocol, &mut reader)?,
        },
        (Protocol::QuakeWorld28, 51) => PeerUpdate::SetInfo {
            slot,
            key: read_text(protocol, &mut reader)?,
            value: read_text(protocol, &mut reader)?,
        },
        _ => return Err(Error::Unsupported),
    };
    opcode(protocol, update).ok_or(Error::Opcode)?;
    Ok((update, reader.byte_position()))
}

/// Only identical peer representations project; unsupported kinds are dropped
/// by the existing native service publisher instead of becoming configstrings.
pub fn peer_update(
    protocol: Protocol,
    update: PeerUpdate<'_>,
    output: &mut [u8],
) -> Result<usize, Error> {
    let service = opcode(protocol, update).ok_or(Error::Unsupported)?;
    let mut writer = Writer::new(output, Encoding::Bytes);
    writer.write_bits(service, 8)?;
    writer.write_bits(u32::from(update.slot()), 8)?;
    match update {
        PeerUpdate::Name { name, .. } => writer.write_string(protocol.string_policy(), name)?,
        PeerUpdate::Frags { frags, .. } => writer.write_bits(frags as u16 as u32, 16)?,
        PeerUpdate::Colors { colors, .. } => writer.write_bits(u32::from(colors), 8)?,
        PeerUpdate::UserInfo { user_id, info, .. } => {
            writer.write_bits(user_id as u32, 32)?;
            writer.write_string(protocol.string_policy(), info)?;
        }
        PeerUpdate::SetInfo { key, value, .. } => {
            writer.write_string(protocol.string_policy(), key)?;
            writer.write_string(protocol.string_policy(), value)?;
        }
    }
    Ok(writer.size())
}
