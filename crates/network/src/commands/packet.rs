//! Connected native move payloads, independent of movement and map rules.
//! A packet is decoded into temporary native records before any SERVER action.
use super::{Q1Move, Q2Cmd, Q3Cmd, QwCmd, delta};
use crate::{
    channel,
    message::{Encoding, Error as MessageError, Reader, Writer},
};
use qa_core::{checksum, primitives::Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    NetQuake15,
    QuakeWorld28,
    Quake2_34,
    Quake3_68,
}
impl Protocol {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "15" => Some(Self::NetQuake15),
            "28" => Some(Self::QuakeWorld28),
            "34" => Some(Self::Quake2_34),
            "68" => Some(Self::Quake3_68),
            _ => None,
        }
    }
    pub const fn number(self) -> u32 {
        match self {
            Self::NetQuake15 => 15,
            Self::QuakeWorld28 => 28,
            Self::Quake2_34 => 34,
            Self::Quake3_68 => 68,
        }
    }
    pub const fn channel(self) -> channel::Policy {
        match self {
            Self::NetQuake15 => channel::NETQUAKE,
            Self::QuakeWorld28 => channel::QUAKEWORLD,
            Self::Quake2_34 => channel::QUAKE2,
            Self::Quake3_68 => channel::QUAKE3,
        }
    }
    fn encoding(self) -> Encoding {
        if self == Self::Quake3_68 {
            Encoding::Q3
        } else {
            Encoding::Bytes
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "A temporary native packet has at most 32 commands and must not allocate while parsing"
)]
pub enum Move {
    NetQuake {
        timestamp: f32,
        command: Q1Move,
    },
    QuakeWorld {
        loss: u8,
        commands: [QwCmd; 3],
    },
    Quake2 {
        last_frame: i32,
        commands: [Q2Cmd; 3],
    },
    Quake3 {
        commands: [Q3Cmd; 32],
        count: u8,
        delta: bool,
    },
}
impl Move {
    pub const fn protocol(&self) -> Protocol {
        match self {
            Self::NetQuake { .. } => Protocol::NetQuake15,
            Self::QuakeWorld { .. } => Protocol::QuakeWorld28,
            Self::Quake2 { .. } => Protocol::Quake2_34,
            Self::Quake3 { .. } => Protocol::Quake3_68,
        }
    }
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Acknowledgements {
    pub server_id: i32,
    pub message: i32,
    pub reliable: i32,
}
#[derive(Clone, Copy, Default)]
pub struct Key<'a> {
    pub challenge: u32,
    pub checksum_feed: u32,
    pub acknowledgements: Acknowledgements,
    /// Native command selected by the reliable ACK's 64-slot history entry.
    pub server_command: &'a [u8],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Message(MessageError),
    Opcode,
    Count,
    Checksum,
    Trailing,
    Context,
}
impl From<MessageError> for Error {
    fn from(e: MessageError) -> Self {
        Self::Message(e)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native move {self:?}")
    }
}
impl std::error::Error for Error {}

pub const ZERO_QW: QwCmd = QwCmd {
    msec: 0,
    view_angles: Vec3([0.; 3]),
    movement: [0; 3],
    buttons: 0,
    impulse: 0,
};
pub const ZERO_Q2: Q2Cmd = Q2Cmd {
    msec: 0,
    angles: [0; 3],
    movement: [0; 3],
    buttons: 0,
    impulse: 0,
    light_level: 0,
};
pub const ZERO_Q3: Q3Cmd = Q3Cmd {
    server_time: 0,
    angles: [0; 3],
    movement: [0; 3],
    buttons: 0,
    weapon: 0,
};

/// Com_HashKey uses signed native char arithmetic, including arithmetic shifts.
/// This is the original protocol key, never an engine content identity.
pub fn command_key(text: &[u8]) -> u32 {
    let hash = text
        .iter()
        .take(32)
        .take_while(|&&b| b != 0)
        .enumerate()
        .fold(0i32, |sum, (i, &b)| {
            sum.wrapping_add(i32::from(b as i8) * (119 + i as i32))
        });
    (hash ^ (hash >> 10) ^ (hash >> 20)) as u32
}
/// Same symmetric native byte transform for both channel directions.
pub fn xor(data: &mut [u8], start: usize, mut key: u8, text: &[u8]) {
    let length = text.iter().position(|&b| b == 0).unwrap_or(text.len());
    let mut at = 0;
    for (i, byte) in data.iter_mut().enumerate().skip(start) {
        let ch = if length == 0 { 0 } else { text[at] };
        let ch = if ch > 127 || ch == b'%' { b'.' } else { ch };
        key ^= ch.wrapping_shl((i & 1) as u32);
        *byte ^= key;
        if length != 0 {
            at = (at + 1) % length;
        }
    }
}

pub fn write(out: &mut [u8], movement: &Move, sequence: u32, key: Key<'_>) -> Result<usize, Error> {
    let mut w = Writer::new(out, movement.protocol().encoding());
    match movement {
        Move::NetQuake { timestamp, command } => {
            w.write_bits(3, 8)?;
            w.write_float(*timestamp)?;
            for angle in command.view_angles.0 {
                w.write_bits(((angle as i32).wrapping_mul(256) / 360) as u32, 8)?;
            }
            for value in command.movement {
                w.write_bits(value as u32, 16)?;
            }
            w.write_bits(command.buttons.into(), 8)?;
            w.write_bits(command.impulse.into(), 8)?;
        }
        Move::QuakeWorld { loss, commands } => {
            w.write_bits(3, 8)?;
            w.write_bits(0, 8)?;
            w.write_bits((*loss).into(), 8)?;
            let mut old = ZERO_QW;
            for &command in commands {
                delta::write_qw(&mut w, old, command)?;
                old = command;
            }
        }
        Move::Quake2 {
            last_frame,
            commands,
        } => {
            w.write_bits(2, 8)?;
            w.write_bits(0, 8)?;
            w.write_bits(*last_frame as u32, 32)?;
            let mut old = ZERO_Q2;
            for &command in commands {
                delta::write_q2(&mut w, old, command)?;
                old = command;
            }
        }
        Move::Quake3 {
            commands,
            count,
            delta: delta_snapshot,
        } => {
            if !(1..=32).contains(count) {
                return Err(Error::Count);
            }
            for word in [
                key.acknowledgements.server_id,
                key.acknowledgements.message,
                key.acknowledgements.reliable,
            ] {
                w.write_bits(word as u32, 32)?;
            }
            w.write_bits(if *delta_snapshot { 2 } else { 3 }, 8)?;
            w.write_bits((*count).into(), 8)?;
            let command_key = key.checksum_feed
                ^ key.acknowledgements.message as u32
                ^ command_key(key.server_command);
            let mut old = ZERO_Q3;
            for &command in &commands[..usize::from(*count)] {
                delta::write_q3(&mut w, old, command, command_key)?;
                old = command;
            }
            w.write_bits(5, 8)?;
        }
    }
    let n = w.size();
    match movement.protocol() {
        Protocol::QuakeWorld28 => out[1] = checksum::qw_sequence_crc(&out[2..n], sequence),
        Protocol::Quake2_34 => out[1] = checksum::q2_sequence_crc(&out[2..n], sequence),
        Protocol::Quake3_68 => xor(
            &mut out[..n],
            12,
            (key.challenge
                ^ key.acknowledgements.server_id as u32
                ^ key.acknowledgements.message as u32) as u8,
            key.server_command,
        ),
        _ => {}
    }
    Ok(n)
}

/// Q3 mutates only the caller's packet scratch to undo its native XOR.
/// The caller chooses the command history entry from the clear ACK prefix.
pub fn acknowledgements(bytes: &[u8]) -> Result<Acknowledgements, Error> {
    let mut r = Reader::new(bytes, Encoding::Q3);
    Ok(Acknowledgements {
        server_id: r.read_bits(32)? as i32,
        message: r.read_bits(32)? as i32,
        reliable: r.read_bits(32)? as i32,
    })
}
pub fn read(
    protocol: Protocol,
    bytes: &mut [u8],
    sequence: u32,
    key: Key<'_>,
) -> Result<Move, Error> {
    let acknowledgements = if protocol == Protocol::Quake3_68 {
        let ack = acknowledgements(bytes)?;
        if ack != key.acknowledgements {
            return Err(Error::Context);
        }
        xor(
            bytes,
            12,
            (key.challenge ^ ack.server_id as u32 ^ ack.message as u32) as u8,
            key.server_command,
        );
        ack
    } else {
        Acknowledgements::default()
    };
    let mut r = Reader::new(bytes, protocol.encoding());
    let movement = match protocol {
        Protocol::NetQuake15 => {
            if r.read_bits(8)? != 3 {
                return Err(Error::Opcode);
            }
            let timestamp = r.read_float()?;
            let mut angles = [0.; 3];
            for angle in &mut angles {
                *angle = (r.read_signed(8)? as f32) * (360.0 / 256.0);
            }
            let mut movement = [0; 3];
            for value in &mut movement {
                *value = r.read_signed(16)? as i16;
            }
            Move::NetQuake {
                timestamp,
                command: Q1Move {
                    view_angles: Vec3(angles),
                    movement,
                    buttons: r.read_bits(8)? as u8,
                    impulse: r.read_bits(8)? as u8,
                },
            }
        }
        Protocol::QuakeWorld28 => {
            if r.read_bits(8)? != 3 {
                return Err(Error::Opcode);
            }
            let checksum = r.read_bits(8)? as u8;
            let loss = r.read_bits(8)? as u8;
            let mut commands = [ZERO_QW; 3];
            let mut old = ZERO_QW;
            for command in &mut commands {
                *command = delta::read_qw(&mut r, old)?;
                old = *command;
            }
            if checksum != checksum::qw_sequence_crc(&bytes[2..r.byte_position()], sequence) {
                return Err(Error::Checksum);
            }
            Move::QuakeWorld { loss, commands }
        }
        Protocol::Quake2_34 => {
            if r.read_bits(8)? != 2 {
                return Err(Error::Opcode);
            }
            let checksum = r.read_bits(8)? as u8;
            let last_frame = r.read_bits(32)? as i32;
            let mut commands = [ZERO_Q2; 3];
            let mut old = ZERO_Q2;
            for command in &mut commands {
                *command = delta::read_q2(&mut r, old)?;
                old = *command;
            }
            if checksum != checksum::q2_sequence_crc(&bytes[2..r.byte_position()], sequence) {
                return Err(Error::Checksum);
            }
            Move::Quake2 {
                last_frame,
                commands,
            }
        }
        Protocol::Quake3_68 => {
            for _ in 0..3 {
                r.read_bits(32)?;
            }
            let opcode = r.read_bits(8)?;
            if !matches!(opcode, 2 | 3) {
                return Err(Error::Opcode);
            }
            let count = r.read_bits(8)? as u8;
            if !(1..=32).contains(&count) {
                return Err(Error::Count);
            }
            let mut commands = [ZERO_Q3; 32];
            let mut old = ZERO_Q3;
            let command_key = key.checksum_feed
                ^ acknowledgements.message as u32
                ^ command_key(key.server_command);
            for command in &mut commands[..usize::from(count)] {
                *command = delta::read_q3(&mut r, old, command_key)?;
                old = *command;
            }
            if r.read_bits(8)? != 5 {
                return Err(Error::Trailing);
            }
            Move::Quake3 {
                commands,
                count,
                delta: opcode == 2,
            }
        }
    };
    if protocol != Protocol::Quake3_68 && r.byte_position() != bytes.len() {
        return Err(Error::Trailing);
    }
    Ok(movement)
}
