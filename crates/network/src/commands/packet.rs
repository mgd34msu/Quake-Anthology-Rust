//! Connected native move payloads, independent of movement and map rules.
//! A packet is decoded into temporary native records before any SERVER action.
use super::{Q1Move, Q2Cmd, Q2RrCmd, Q3Cmd, QwCmd, delta};
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
        delta_request: Option<u8>,
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
pub const ZERO_Q2_RR: Q2RrCmd = Q2RrCmd {
    msec: 0,
    buttons: 0,
    angles: Vec3([0.; 3]),
    movement: [0.; 2],
    server_frame: 0,
};

fn write_series<C: Copy>(
    writer: &mut Writer<'_>,
    commands: &[C],
    mut old: C,
    mut encode: impl FnMut(&mut Writer<'_>, C, C) -> Result<(), MessageError>,
) -> Result<(), MessageError> {
    for &command in commands {
        encode(writer, old, command)?;
        old = command;
    }
    Ok(())
}
fn read_series<C: Copy>(
    reader: &mut Reader<'_>,
    commands: &mut [C],
    mut old: C,
    mut decode: impl FnMut(&mut Reader<'_>, C) -> Result<C, MessageError>,
) -> Result<(), MessageError> {
    for command in commands {
        *command = decode(reader, old)?;
        old = *command;
    }
    Ok(())
}

/// Q2repro 1038 nonbatched clc_move payload. Its last-frame prefix has no CRC;
/// per-command server_frame is supplied later by the native module boundary.
/// Connection selection remains separate from this native payload entry.
pub fn write_q2_repro_move(
    out: &mut [u8],
    last_frame: i32,
    commands: &[Q2RrCmd; 3],
) -> Result<usize, Error> {
    let mut writer = Writer::new(out, Encoding::Bytes);
    writer.write_bits(2, 8)?;
    writer.write_bits(last_frame as u32, 32)?;
    write_series(&mut writer, commands, ZERO_Q2_RR, delta::write_q2_repro)?;
    Ok(writer.size())
}
pub fn read_q2_repro_move(bytes: &[u8]) -> Result<(i32, [Q2RrCmd; 3]), Error> {
    let mut reader = Reader::new(bytes, Encoding::Bytes);
    if reader.read_bits(8)? != 2 {
        return Err(Error::Opcode);
    }
    let last_frame = reader.read_bits(32)? as i32;
    let mut commands = [ZERO_Q2_RR; 3];
    read_series(&mut reader, &mut commands, ZERO_Q2_RR, delta::read_q2_repro)?;
    if reader.byte_position() != bytes.len() {
        return Err(Error::Trailing);
    }
    Ok((last_frame, commands))
}

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
    write_with_commands(out, movement, sequence, key, |_| Ok(()))
}
pub fn write_with_commands(
    out: &mut [u8],
    movement: &Move,
    sequence: u32,
    key: Key<'_>,
    reliable: impl FnOnce(&mut Writer<'_>) -> Result<(), Error>,
) -> Result<usize, Error> {
    let mut w = Writer::new(out, movement.protocol().encoding());
    let mut qw_move_end = 0;
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
        Move::QuakeWorld {
            loss,
            commands,
            delta_request,
        } => {
            w.write_bits(3, 8)?;
            w.write_bits(0, 8)?;
            w.write_bits((*loss).into(), 8)?;
            write_series(&mut w, commands, ZERO_QW, delta::write_qw)?;
            // CL_SendCmd checksums only the move, before clc_delta.
            qw_move_end = w.size();
            if let Some(request) = delta_request {
                w.write_bits(5, 8)?;
                w.write_bits((*request).into(), 8)?;
            }
        }
        Move::Quake2 {
            last_frame,
            commands,
        } => {
            w.write_bits(2, 8)?;
            w.write_bits(0, 8)?;
            w.write_bits(*last_frame as u32, 32)?;
            write_series(&mut w, commands, ZERO_Q2, delta::write_q2)?;
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
            reliable(&mut w)?;
            w.write_bits(if *delta_snapshot { 2 } else { 3 }, 8)?;
            w.write_bits((*count).into(), 8)?;
            let command_key = key.checksum_feed
                ^ key.acknowledgements.message as u32
                ^ command_key(key.server_command);
            write_series(
                &mut w,
                &commands[..usize::from(*count)],
                ZERO_Q3,
                |writer, old, command| delta::write_q3(writer, old, command, command_key),
            )?;
            w.write_bits(5, 8)?;
        }
    }
    let n = w.size();
    match movement.protocol() {
        Protocol::QuakeWorld28 => {
            out[1] = checksum::qw_sequence_crc(&out[2..qw_move_end], sequence);
        }
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
    read_with_commands(protocol, bytes, sequence, key, |_, _| Ok(()))?.ok_or(Error::Opcode)
}

/// SV_ExecuteClientMessage permits nop and repeated clc_delta around one move.
/// Requests replace earlier requests; their bytes are outside the move CRC.
fn qw_opcode(
    reader: &mut Reader<'_>,
    length: usize,
    request: &mut Option<u8>,
) -> Result<Option<u32>, Error> {
    while reader.byte_position() < length {
        match reader.read_bits(8)? {
            1 => {} // clc_nop
            5 => *request = Some(reader.read_bits(8)? as u8),
            opcode => return Ok(Some(opcode)),
        }
    }
    Ok(None)
}

pub fn read_with_commands(
    protocol: Protocol,
    bytes: &mut [u8],
    sequence: u32,
    key: Key<'_>,
    mut consume: impl FnMut(u32, &[u8]) -> Result<(), Error>,
) -> Result<Option<Move>, Error> {
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
            let mut delta_request = None;
            if qw_opcode(&mut r, bytes.len(), &mut delta_request)? != Some(3) {
                return Err(Error::Opcode);
            }
            let checksum_index = r.byte_position();
            let checksum = r.read_bits(8)? as u8;
            let loss = r.read_bits(8)? as u8;
            let mut commands = [ZERO_QW; 3];
            read_series(&mut r, &mut commands, ZERO_QW, delta::read_qw)?;
            if checksum
                != checksum::qw_sequence_crc(
                    &bytes[checksum_index + 1..r.byte_position()],
                    sequence,
                )
            {
                return Err(Error::Checksum);
            }
            if qw_opcode(&mut r, bytes.len(), &mut delta_request)?.is_some() {
                return Err(Error::Opcode);
            }
            Move::QuakeWorld {
                loss,
                commands,
                delta_request,
            }
        }
        Protocol::Quake2_34 => {
            if r.read_bits(8)? != 2 {
                return Err(Error::Opcode);
            }
            let checksum = r.read_bits(8)? as u8;
            let last_frame = r.read_bits(32)? as i32;
            let mut commands = [ZERO_Q2; 3];
            read_series(&mut r, &mut commands, ZERO_Q2, delta::read_q2)?;
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
            let mut text = [0; 1024];
            let mut opcode = r.read_bits(8)?;
            let mut command_count = 0;
            while opcode == 4 {
                if command_count >= 64 {
                    return Err(Error::Count);
                }
                let seq = r.read_bits(32)?;
                let text = crate::channel::commands::read_string(&mut r, &mut text)?;
                consume(seq, text)?;
                command_count += 1;
                opcode = r.read_bits(8)?;
            }
            if opcode == 5 {
                return Ok(None);
            }
            if !matches!(opcode, 2 | 3) {
                return Err(Error::Opcode);
            }
            let count = r.read_bits(8)? as u8;
            if !(1..=32).contains(&count) {
                return Err(Error::Count);
            }
            let mut commands = [ZERO_Q3; 32];
            let command_key = key.checksum_feed
                ^ acknowledgements.message as u32
                ^ command_key(key.server_command);
            read_series(
                &mut r,
                &mut commands[..usize::from(count)],
                ZERO_Q3,
                |reader, old| delta::read_q3(reader, old, command_key),
            )?;
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
    Ok(Some(movement))
}
