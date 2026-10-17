//! Connected native move payloads, independent of movement and map rules.
//! Borrowed records are delivered in native order after each record validates.
use super::{Q1Move, Q2Cmd, Q2RrCmd, Q3Cmd, QwCmd, delta};
use crate::{
    channel,
    message::{Encoding, Error as MessageError, Reader, StringPolicy, Writer},
};
pub use qa_core::primitives::CommandExecution;
use qa_core::{checksum, primitives::Vec3};
use std::ops::ControlFlow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    NetQuake15,
    QuakeWorld28,
    Quake2_34,
    Quake2Repro1038,
    Quake3_68,
}
impl Protocol {
    /// Connection storage follows its native message limit, not map geometry.
    pub const fn message_bytes(self) -> usize {
        match self {
            Self::Quake3_68 => Q3_MESSAGE_BYTES,
            Self::Quake2Repro1038 => qa_core::primitives::NATIVE_WIRE_BYTES,
            _ => 8192,
        }
    }
    /// Native connection controls with a CLIENT handshake owner.
    pub const fn default_connect_port(self) -> Option<u16> {
        match self {
            Self::NetQuake15 => Some(26000),
            Self::QuakeWorld28 => Some(27500),
            Self::Quake2_34 | Self::Quake2Repro1038 => Some(27910),
            _ => None,
        }
    }
    /// Native wire userinfo is independent of a module ABI's input buffer.
    pub const fn userinfo_rules(self) -> Option<qa_core::text::InfoRules> {
        use qa_core::text::{InfoBytes, InfoRules};
        let (bytes, fields, filter) = match self {
            Self::NetQuake15 => return None,
            Self::QuakeWorld28 => (
                super::startup::QW_CONNECT_USERINFO_BYTES,
                64,
                InfoBytes::Exact,
            ),
            Self::Quake2_34 | Self::Quake2Repro1038 => (
                super::startup::Q2_CONNECT_USERINFO_BYTES,
                64,
                InfoBytes::Ascii7Printable,
            ),
            Self::Quake3_68 => (
                crate::message::COMMAND_STRING_BYTES,
                crate::message::COMMAND_STRING_BYTES,
                InfoBytes::Exact,
            ),
        };
        Some(InfoRules {
            bytes,
            key_bytes: fields,
            value_bytes: fields,
            filter,
        })
    }
    pub const fn userinfo_bytes(self) -> Option<usize> {
        match self.userinfo_rules() {
            Some(rules) => Some(rules.bytes),
            None => None,
        }
    }
    /// Protocol1038 omits its byte qport entirely when that byte is zero.
    pub const fn client_channel(self, qport: u16) -> channel::Policy {
        match self {
            Self::Quake2Repro1038 => channel::q2_new(qport as u8 != 0),
            _ => self.channel(),
        }
    }
    /// Native final CLIENT payload; Q3 keeps its reliable command path.
    pub fn write_disconnect(self, out: &mut [u8]) -> Result<usize, Error> {
        match self {
            Self::NetQuake15 => {
                let mut writer = Writer::new(out, Encoding::Bytes);
                writer.write_bits(2, 8)?;
                Ok(writer.size())
            }
            Self::QuakeWorld28 => self.write_client_text(4, b"drop", out),
            Self::Quake2_34 | Self::Quake2Repro1038 => {
                let length = self.write_client_text(4, b"disconnect", out)?;
                Ok(length - usize::from(self == Self::Quake2_34))
            }
            _ => Err(Error::Context),
        }
    }
    pub(crate) fn write_client_text(
        self,
        opcode: u32,
        text: &[u8],
        out: &mut [u8],
    ) -> Result<usize, Error> {
        let mut writer = Writer::new(out, Encoding::Bytes);
        writer.write_bits(opcode, 8)?;
        writer.write_string(self.client_string_policy(), text)?;
        Ok(writer.size())
    }
    pub const fn native(rules: qa_core::primitives::RuleSetId) -> Self {
        use qa_core::primitives::RuleSetId;
        match rules {
            RuleSetId::Quake => Self::NetQuake15,
            RuleSetId::QuakeWorld => Self::QuakeWorld28,
            RuleSetId::Quake2 => Self::Quake2_34,
            RuleSetId::Quake2Rerelease => Self::Quake2Repro1038,
            RuleSetId::Quake3 => Self::Quake3_68,
        }
    }
    pub const fn string_policy(self) -> StringPolicy {
        match self {
            Self::Quake2Repro1038 => StringPolicy::UNSIGNED,
            Self::Quake3_68 => StringPolicy::Q3,
            _ => StringPolicy::LEGACY,
        }
    }
    pub const fn client_string_policy(self) -> StringPolicy {
        if matches!(self, Self::Quake2Repro1038) {
            self.string_policy()
                .limited(crate::message::COMMAND_STRING_BYTES - 1)
        } else {
            self.string_policy()
        }
    }
    /// Native protocol identity at the wire boundary, independent of the
    /// connection's selected movement, presentation and module capabilities.
    pub const fn rule_set(self) -> qa_core::primitives::RuleSetId {
        use qa_core::primitives::RuleSetId;
        match self {
            Self::NetQuake15 => RuleSetId::Quake,
            Self::QuakeWorld28 => RuleSetId::QuakeWorld,
            Self::Quake2_34 => RuleSetId::Quake2,
            Self::Quake2Repro1038 => RuleSetId::Quake2Rerelease,
            Self::Quake3_68 => RuleSetId::Quake3,
        }
    }
    /// Native resource limits exist even when signon uses resource lists rather
    /// than configstrings.
    pub const fn resource_layout(self) -> qa_core::configstrings::ConfigLayout {
        qa_core::configstrings::ConfigLayout::for_rules(self.rule_set())
    }
    /// Storage availability does not imply a native configstring wire adapter.
    /// NQ serverinfo resource lists still require their native signon adapter.
    pub const fn config_layout(self) -> Option<qa_core::configstrings::ConfigLayout> {
        match self {
            Self::NetQuake15 => None,
            _ => Some(self.resource_layout()),
        }
    }
    /// Native wire limits are independent of a module's resource storage.
    pub fn config_capacity(self, index: usize) -> Option<usize> {
        let layout = self.config_layout()?;
        if self != Self::QuakeWorld28 {
            return layout.capacity(index);
        }
        if layout.lights.contains(index)
            || layout.resources[..2]
                .iter()
                .any(|range| range.contains(index) && index != range.first as usize)
        {
            // MAX_QPATH includes NUL, as does the native lightstyle buffer.
            Some(63)
        } else {
            None
        }
    }
    pub const fn command_execution(self) -> CommandExecution {
        match self {
            Self::NetQuake15 => CommandExecution::FrameLatest,
            _ => CommandExecution::PerCommand,
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "15" => Some(Self::NetQuake15),
            "28" => Some(Self::QuakeWorld28),
            "34" => Some(Self::Quake2_34),
            "1038" => Some(Self::Quake2Repro1038),
            "68" => Some(Self::Quake3_68),
            _ => None,
        }
    }
    pub const fn number(self) -> u32 {
        match self {
            Self::NetQuake15 => 15,
            Self::QuakeWorld28 => 28,
            Self::Quake2_34 => 34,
            Self::Quake2Repro1038 => 1038,
            Self::Quake3_68 => 68,
        }
    }
    pub const fn channel(self) -> channel::Policy {
        match self {
            Self::NetQuake15 => channel::NETQUAKE,
            Self::QuakeWorld28 => channel::QUAKEWORLD,
            Self::Quake2_34 => channel::QUAKE2,
            Self::Quake2Repro1038 => channel::q2_new(false),
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
    Quake2Repro {
        last_frame: i32,
        commands: [Q2RrCmd; 3],
    },
    Quake3 {
        commands: [Q3Cmd; Q3_COMMANDS],
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
            Self::Quake2Repro { .. } => Protocol::Quake2Repro1038,
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

pub const Q3_MESSAGE_BYTES: usize = 16384;
pub const Q3_COMMANDS: usize = 32;

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
    write(
        out,
        &Move::Quake2Repro {
            last_frame,
            commands: *commands,
        },
        0,
        Key::default(),
    )
}
pub fn read_q2_repro_move(bytes: &[u8]) -> Result<(i32, [Q2RrCmd; 3]), Error> {
    let mut reader = Reader::new(bytes, Encoding::Bytes);
    let decoded = read_repro_move(&mut reader)?;
    if reader.byte_position() != bytes.len() {
        return Err(Error::Trailing);
    }
    Ok(decoded)
}
fn read_repro_move(reader: &mut Reader<'_>) -> Result<(i32, [Q2RrCmd; 3]), Error> {
    if reader.read_bits(8)? != 2 {
        return Err(Error::Opcode);
    }
    read_repro_body(reader)
}
fn read_repro_body(reader: &mut Reader<'_>) -> Result<(i32, [Q2RrCmd; 3]), Error> {
    let last_frame = reader.read_bits(32)? as i32;
    let mut commands = [ZERO_Q2_RR; 3];
    read_series(reader, &mut commands, ZERO_Q2_RR, delta::read_q2_repro)?;
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
    let length = qa_core::text::until_nul(text).len();
    let mut at = 0;
    for (i, byte) in data.iter_mut().enumerate().skip(start) {
        let ch = if length == 0 { 0 } else { text[at] };
        let ch = StringPolicy::Q3.read_byte(ch);
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
    write_client_packet(
        out,
        movement.protocol(),
        Some(movement),
        sequence,
        key,
        reliable,
    )
}

/// Native client ACK/reliable-command envelope without another movement command.
pub fn write_control(
    out: &mut [u8],
    protocol: Protocol,
    key: Key<'_>,
    reliable: impl FnOnce(&mut Writer<'_>) -> Result<(), Error>,
) -> Result<usize, Error> {
    write_client_packet(out, protocol, None, 0, key, reliable)
}

fn write_client_packet(
    out: &mut [u8],
    protocol: Protocol,
    movement: Option<&Move>,
    sequence: u32,
    key: Key<'_>,
    reliable: impl FnOnce(&mut Writer<'_>) -> Result<(), Error>,
) -> Result<usize, Error> {
    let mut w = Writer::new(out, protocol.encoding());
    if protocol == Protocol::Quake3_68 {
        for word in [
            key.acknowledgements.server_id,
            key.acknowledgements.message,
            key.acknowledgements.reliable,
        ] {
            w.write_bits(word as u32, 32)?;
        }
        reliable(&mut w)?;
    }
    let mut qw_move_end = 0;
    match movement {
        Some(Move::NetQuake { timestamp, command }) => {
            w.write_bits(3, 8)?;
            w.write_float(*timestamp)?;
            for angle in command.view_angles.0 {
                w.write_bits(
                    qa_core::math::angle_to_byte(angle, qa_core::math::AngleByteForm::Integral)
                        as u32,
                    8,
                )?;
            }
            for value in command.movement {
                w.write_bits(value as u32, 16)?;
            }
            w.write_bits(command.buttons.into(), 8)?;
            w.write_bits(command.impulse.into(), 8)?;
        }
        Some(Move::QuakeWorld {
            loss,
            commands,
            delta_request,
        }) => {
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
        Some(Move::Quake2 {
            last_frame,
            commands,
        }) => {
            w.write_bits(2, 8)?;
            w.write_bits(0, 8)?;
            w.write_bits(*last_frame as u32, 32)?;
            write_series(&mut w, commands, ZERO_Q2, delta::write_q2)?;
        }
        Some(Move::Quake2Repro {
            last_frame,
            commands,
        }) => {
            w.write_bits(2, 8)?;
            w.write_bits(*last_frame as u32, 32)?;
            write_series(&mut w, commands, ZERO_Q2_RR, delta::write_q2_repro)?;
        }
        Some(Move::Quake3 {
            commands,
            count,
            delta: delta_snapshot,
        }) => {
            if !(1..=Q3_COMMANDS as u8).contains(count) {
                return Err(Error::Count);
            }
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
        }
        None => {}
    }
    if protocol == Protocol::Quake3_68 {
        w.write_bits(5, 8)?;
    }
    let n = w.size();
    match protocol {
        Protocol::QuakeWorld28 if movement.is_some() => {
            out[1] = checksum::qw_sequence_crc(&out[2..qw_move_end], sequence);
        }
        Protocol::Quake2_34 if movement.is_some() => {
            out[1] = checksum::q2_sequence_crc(&out[2..n], sequence)
        }
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
/// A synchronous view of one native packet record. No record survives its callback.
#[derive(Clone, Copy, Debug, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "The native move owns bounded temporary command scratch, never a heap record"
)]
pub enum Record<'a> {
    Move(Move),
    String {
        sequence: Option<u32>,
        text: &'a [u8],
    },
    Delta(u8),
    Userinfo(&'a [u8]),
    Disconnect,
}

pub fn read(
    protocol: Protocol,
    bytes: &mut [u8],
    sequence: u32,
    key: Key<'_>,
) -> Result<Move, Error> {
    let mut movement = None;
    let mut delta_request = None;
    read_with_commands(protocol, bytes, sequence, key, |record| {
        match record {
            Record::Move(value) => movement = Some(value),
            Record::Delta(value) => delta_request = Some(value),
            _ => {}
        }
        Ok(ControlFlow::Continue(()))
    })?;
    let mut movement = movement.ok_or(Error::Opcode)?;
    if let Move::QuakeWorld {
        delta_request: request,
        ..
    } = &mut movement
    {
        *request = delta_request;
    }
    Ok(movement)
}

/// Natively accepted prefix effects remain visible when a later record fails.
/// A second QW/Q2 move stops the packet without retracting its first move.
pub fn read_with_commands(
    protocol: Protocol,
    bytes: &mut [u8],
    sequence: u32,
    key: Key<'_>,
    mut consume: impl FnMut(Record<'_>) -> Result<ControlFlow<()>, Error>,
) -> Result<(), Error> {
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
    if protocol == Protocol::Quake3_68 {
        for _ in 0..3 {
            r.read_bits(32)?;
        }
    }
    let mut move_issued = false;
    let mut string_count = 0;
    let mut delta_request = None;
    let mut q3_text = [0; crate::channel::commands::STRING_BYTES];
    loop {
        if protocol != Protocol::Quake3_68 && r.byte_position() == bytes.len() {
            return Ok(());
        }
        let opcode = r.read_bits(8)?;
        match (protocol, opcode) {
            (
                Protocol::NetQuake15
                | Protocol::QuakeWorld28
                | Protocol::Quake2_34
                | Protocol::Quake2Repro1038,
                1,
            ) => {}
            (Protocol::NetQuake15, 2) => {
                let _ = consume(Record::Disconnect)?;
                return Ok(());
            }
            (Protocol::QuakeWorld28, 5) => {
                let request = r.read_bits(8)? as u8;
                delta_request = Some(request);
                if consume(Record::Delta(request))?.is_break() {
                    return Ok(());
                }
            }
            (
                Protocol::NetQuake15
                | Protocol::QuakeWorld28
                | Protocol::Quake2_34
                | Protocol::Quake2Repro1038,
                4,
            )
            | (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 3) => {
                let policy = if protocol == Protocol::Quake2_34 && opcode == 4 {
                    protocol.client_string_policy().allow_eof()
                } else {
                    protocol.client_string_policy()
                };
                let text = r.read_string(policy).map_err(|error| {
                    if error.kind == crate::message::ErrorKind::Capacity {
                        Error::Count
                    } else {
                        Error::Message(error)
                    }
                })?;
                let record = if opcode == 3 {
                    Record::Userinfo(text)
                } else {
                    string_count += 1;
                    let maximum = match protocol {
                        Protocol::Quake2_34 => 7,
                        Protocol::Quake2Repro1038 => 8,
                        _ => usize::MAX,
                    };
                    if string_count > maximum {
                        continue;
                    }
                    Record::String {
                        sequence: None,
                        text,
                    }
                };
                if consume(record)?.is_break() {
                    return Ok(());
                }
            }
            (Protocol::Quake3_68, 4) => {
                let sequence = r.read_bits(32)?;
                let text = r.read_string_into(protocol.string_policy(), &mut q3_text)?;
                if consume(Record::String {
                    sequence: Some(sequence),
                    text,
                })?
                .is_break()
                {
                    return Ok(());
                }
            }
            (Protocol::Quake3_68, 5) => return Ok(()),
            (Protocol::NetQuake15, 3)
            | (Protocol::QuakeWorld28, 3)
            | (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 2)
            | (Protocol::Quake3_68, 2 | 3) => {
                if move_issued && protocol != Protocol::NetQuake15 {
                    return Ok(());
                }
                move_issued = true;
                let movement = match protocol {
                    Protocol::NetQuake15 => {
                        let timestamp = r.read_float()?;
                        let mut angles = [0.; 3];
                        for angle in &mut angles {
                            *angle = qa_core::math::byte_to_angle(r.read_signed(8)?);
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
                        Move::QuakeWorld {
                            loss,
                            commands,
                            delta_request,
                        }
                    }
                    Protocol::Quake2_34 => {
                        let checksum_index = r.byte_position();
                        let checksum = r.read_bits(8)? as u8;
                        let last_frame = r.read_bits(32)? as i32;
                        let mut commands = [ZERO_Q2; 3];
                        read_series(&mut r, &mut commands, ZERO_Q2, delta::read_q2)?;
                        if checksum
                            != checksum::q2_sequence_crc(
                                &bytes[checksum_index + 1..r.byte_position()],
                                sequence,
                            )
                        {
                            return Err(Error::Checksum);
                        }
                        Move::Quake2 {
                            last_frame,
                            commands,
                        }
                    }
                    Protocol::Quake2Repro1038 => {
                        let (last_frame, commands) = read_repro_body(&mut r)?;
                        Move::Quake2Repro {
                            last_frame,
                            commands,
                        }
                    }
                    Protocol::Quake3_68 => {
                        let count = r.read_bits(8)? as u8;
                        if !(1..=Q3_COMMANDS as u8).contains(&count) {
                            return Err(Error::Count);
                        }
                        let mut commands = [ZERO_Q3; Q3_COMMANDS];
                        let command_key = key.checksum_feed
                            ^ acknowledgements.message as u32
                            ^ command_key(key.server_command);
                        read_series(
                            &mut r,
                            &mut commands[..usize::from(count)],
                            ZERO_Q3,
                            |reader, old| delta::read_q3(reader, old, command_key),
                        )?;
                        Move::Quake3 {
                            commands,
                            count,
                            delta: opcode == 2,
                        }
                    }
                };
                if protocol == Protocol::Quake3_68 && r.read_bits(8)? != 5 {
                    return Err(Error::Trailing);
                }
                if consume(Record::Move(movement))?.is_break() || protocol == Protocol::Quake3_68 {
                    return Ok(());
                }
            }
            _ => return Err(Error::Opcode),
        }
    }
}
