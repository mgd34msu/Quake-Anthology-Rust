//! Borrowed native startup records enter the existing CLIENT resource rows.
mod q2;
use super::{packet, packet::Protocol};
use crate::{
    headers::datagram,
    message::{Encoding, Reader, StringPolicy, Writer},
    outputs::ConfigString,
};
pub use q2::{Q2ReproServerData, Q2ServerData, ResourceStream};

/// NetQuake's control protocol version is independent of game protocol 15.
#[derive(Clone, Copy)]
pub enum NqControl<'a> {
    Connect,
    Accept { port: u16 },
    Reject { reason: &'a [u8] },
}

pub fn read_nq_control(bytes: &[u8]) -> Result<NqControl<'_>, packet::Error> {
    if !(5..=u16::MAX as usize).contains(&bytes.len()) {
        return Err(packet::Error::Count);
    }
    let mut reader = Reader::new(bytes, Encoding::Bytes);
    let header = reader.read_bits(32)?.swap_bytes();
    if header & !0xffff != datagram::CONTROL || header & 0xffff != bytes.len() as u32 {
        return Err(packet::Error::Context);
    }
    let control = match reader.read_bits(8)? {
        1 => {
            if reader.read_string(StringPolicy::UNSIGNED)? != b"QUAKE" || reader.read_bits(8)? != 3
            {
                return Err(packet::Error::Context);
            }
            NqControl::Connect
        }
        0x81 => {
            let port = reader.read_signed(32)?;
            if !(1..=i32::from(u16::MAX)).contains(&port) {
                return Err(packet::Error::Count);
            }
            NqControl::Accept { port: port as u16 }
        }
        0x82 => NqControl::Reject {
            reason: reader.read_string(StringPolicy::UNSIGNED)?,
        },
        _ => return Err(packet::Error::Context),
    };
    if reader.byte_position() != bytes.len() {
        return Err(packet::Error::Count);
    }
    Ok(control)
}

pub fn write_nq_control(control: NqControl<'_>, output: &mut [u8]) -> Result<usize, packet::Error> {
    let length = match control {
        NqControl::Connect => 12,
        NqControl::Accept { port: 0 } => return Err(packet::Error::Count),
        NqControl::Accept { .. } => 9,
        NqControl::Reject { reason } => {
            if reason.contains(&0) {
                return Err(packet::Error::Context);
            }
            reason.len().checked_add(6).ok_or(packet::Error::Count)?
        }
    };
    if length > u16::MAX as usize || length > output.len() {
        return Err(packet::Error::Count);
    }
    let mut writer = Writer::new(output, Encoding::Bytes);
    writer.write_bits((datagram::CONTROL | length as u32).swap_bytes(), 32)?;
    match control {
        NqControl::Connect => {
            writer.write_bits(1, 8)?;
            writer.write_string(StringPolicy::UNSIGNED, b"QUAKE")?;
            writer.write_bits(3, 8)?;
        }
        NqControl::Accept { port } => {
            writer.write_bits(0x81, 8)?;
            writer.write_bits(u32::from(port), 32)?;
        }
        NqControl::Reject { reason } => {
            writer.write_bits(0x82, 8)?;
            writer.write_string(StringPolicy::UNSIGNED, reason)?;
        }
    }
    Ok(writer.size())
}

/// MAX_INFO_STRING includes its native NUL terminator.
pub const QW_CONNECT_USERINFO_BYTES: usize = 196;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QwConnectionless<'a> {
    Challenge(i32),
    Accepted,
    Print(&'a [u8]),
}

pub fn read_qw_connectionless(bytes: &[u8]) -> Result<QwConnectionless<'_>, packet::Error> {
    let mut reader = Reader::out_of_band(bytes)?;
    let code = reader.read_bits(8)?;
    let text = Protocol::QuakeWorld28
        .string_policy()
        .tail(&bytes[reader.byte_position()..]);
    match code as u8 {
        b'c' => Ok(QwConnectionless::Challenge(qa_core::text::atoi_prefix(
            text,
        ))),
        b'j' => Ok(QwConnectionless::Accepted),
        b'n' => Ok(QwConnectionless::Print(text)),
        _ => Err(packet::Error::Context),
    }
}

pub fn write_qw_getchallenge(output: &mut [u8]) -> Result<usize, packet::Error> {
    let mut writer = Writer::out_of_band(output)?;
    writer.write_data(b"getchallenge\n")?;
    Ok(writer.size())
}

pub fn write_qw_connect(
    qport: u16,
    challenge: i32,
    userinfo: &[u8],
    output: &mut [u8],
) -> Result<usize, packet::Error> {
    use std::fmt::Write;
    if userinfo.len() >= QW_CONNECT_USERINFO_BYTES
        || userinfo
            .iter()
            .any(|byte| matches!(byte, 0 | 255 | b'"' | b'\r' | b'\n'))
    {
        return Err(packet::Error::Context);
    }
    let mut prefix = qa_core::text::FixedText::<64>::default();
    write!(prefix, "connect 28 {qport} {challenge} \"").map_err(|_| packet::Error::Count)?;
    let length = 4 + prefix.as_bytes().len() + userinfo.len() + 2;
    if length > output.len() {
        return Err(packet::Error::Count);
    }
    let mut writer = Writer::out_of_band(output)?;
    writer.write_data(prefix.as_bytes())?;
    writer.write_data(userinfo)?;
    writer.write_data(b"\"\n")?;
    Ok(writer.size())
}

#[derive(Clone)]
pub struct NqServerInfo<'a> {
    reader: Reader<'a>,
    pub max_clients: u8,
    pub game_type: u8,
    pub level_name: &'a [u8],
}

struct Header<'a> {
    max_clients: u8,
    game_type: u8,
    level_name: &'a [u8],
}

fn walk<'a>(
    reader: &mut Reader<'a>,
    mut consume: impl FnMut(ConfigString<'a>),
) -> Result<Header<'a>, packet::Error> {
    let protocol = Protocol::NetQuake15;
    if reader.read_bits(32)? != protocol.number() {
        return Err(packet::Error::Context);
    }
    let max_clients = reader.read_bits(8)? as u8;
    if max_clients == 0
        || u32::from(max_clients) > crate::snapshots::snapshot_limits(protocol).client_capacity
    {
        return Err(packet::Error::Count);
    }
    let game_type = reader.read_bits(8)? as u8;
    let level_name = reader.read_string(protocol.string_policy())?;
    for range in &protocol.resource_layout().resources[..2] {
        let mut ordinal = 1;
        loop {
            let text = reader.read_string(protocol.string_policy())?;
            if text.is_empty() {
                break;
            }
            let index = range.get(ordinal).ok_or(packet::Error::Count)?;
            if text.len() >= 64 {
                return Err(packet::Error::Count);
            }
            consume(ConfigString {
                index: index as u32,
                text,
            });
            ordinal += 1;
        }
    }
    Ok(Header {
        max_clients,
        game_type,
        level_name,
    })
}

impl<'a> NqServerInfo<'a> {
    pub(crate) fn read(reader: &mut Reader<'a>) -> Result<Self, packet::Error> {
        let start = reader.clone();
        let header = walk(reader, |_| {})?;
        Ok(Self {
            reader: start,
            max_clients: header.max_clients,
            game_type: header.game_type,
            level_name: header.level_name,
        })
    }

    pub fn for_each_config(
        &self,
        consume: impl FnMut(ConfigString<'a>),
    ) -> Result<(), packet::Error> {
        walk(&mut self.reader.clone(), consume)?;
        Ok(())
    }
}

/// Protocol 28 startup fields borrow this packet; role selection stays with app.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QwServerData<'a> {
    pub server_count: i32,
    pub game_directory: &'a [u8],
    pub player_slot: u8,
    pub spectator: bool,
    pub level_name: &'a [u8],
    pub tuning: qa_core::primitives::MovementTuning,
}
impl<'a> QwServerData<'a> {
    pub(crate) fn read(reader: &mut Reader<'a>) -> Result<Self, packet::Error> {
        use qa_core::primitives::{MovementMagnitude, MovementTuning};
        let protocol = Protocol::QuakeWorld28;
        if reader.read_bits(32)? != protocol.number() {
            return Err(packet::Error::Context);
        }
        let server_count = reader.read_signed(32)?;
        let game_directory = reader.read_string(protocol.string_policy())?;
        if game_directory.len() >= 128 {
            return Err(packet::Error::Count);
        }
        let raw_slot = reader.read_bits(8)? as u8;
        let player_slot = raw_slot & 127;
        if u32::from(player_slot) >= protocol.resource_layout().players.count {
            return Err(packet::Error::Count);
        }
        let level_name = reader.read_string(protocol.string_policy())?;
        let mut number = || reader.read_bits(32).map(f32::from_bits);
        // CL_ParseServerData reads these ten single-precision values in order.
        let tuning = MovementTuning {
            gravity: Some(MovementMagnitude::Base(number()?)),
            stop_speed: Some(number()?),
            max_speed: Some(MovementMagnitude::Base(number()?)),
            spectator_max_speed: Some(number()?),
            accelerate: Some(number()?),
            air_accelerate: Some(number()?),
            water_accelerate: Some(number()?),
            friction: Some(number()?),
            water_friction: Some(number()?),
            gravity_multiplier: number()?,
            ..Default::default()
        };
        Ok(Self {
            server_count,
            game_directory,
            player_slot,
            spectator: raw_slot & 128 != 0,
            level_name,
            tuning,
        })
    }
}
