//! The connection owns its packet codec and fixed decode scratch.
use super::{
    from_q1_move, from_q2_usercmd, from_q3_usercmd, from_qw_usercmd,
    packet::{self, Key, Move, Protocol, ZERO_Q2, ZERO_Q3, ZERO_QW},
    to_q1_move, to_q2_usercmd, to_q3_usercmd, to_qw_usercmd,
};
use crate::channel::Channel;
use crate::{
    ingress::Incoming,
    message::{Encoding, Reader},
    outputs::Prints,
};
use qa_core::primitives::UserCmd;

pub struct Commands {
    pub protocol: Protocol,
    scratch: Box<[u8]>,
    previous_time: i32,
    delta_request: Option<u32>,
}
impl Commands {
    pub fn load(protocol: Protocol) -> Self {
        Self {
            protocol,
            scratch: vec![0; 8192].into_boxed_slice(),
            previous_time: 0,
            delta_request: None,
        }
    }
    /// Native payload delta request. SERVER retains clc_move's request; Q2
    /// CLIENT clears it whenever the current svc_frame is invalid.
    pub fn delta_request(&self) -> Option<u32> {
        self.delta_request
    }
    /// Native packet redundancy is caller-supplied. The development host has
    /// one current command and no input history, so its older entries are zero.
    pub fn encode(
        &self,
        command: &UserCmd,
        channel: &Channel,
        out: &mut [u8],
    ) -> Result<usize, packet::Error> {
        let movement = match self.protocol {
            Protocol::NetQuake15 => Move::NetQuake {
                timestamp: 0.,
                command: to_q1_move(command),
            },
            Protocol::QuakeWorld28 => Move::QuakeWorld {
                loss: 0,
                commands: [ZERO_QW, ZERO_QW, to_qw_usercmd(command)],
            },
            Protocol::Quake2_34 => Move::Quake2 {
                last_frame: self
                    .delta_request
                    .filter(|&sequence| channel.snapshot(sequence).is_some())
                    .map_or(-1, |sequence| sequence as i32),
                commands: [ZERO_Q2, ZERO_Q2, to_q2_usercmd(command)],
            },
            Protocol::Quake3_68 => {
                let mut commands = [ZERO_Q3; 32];
                // No registry/module binding exists in the development host.
                // Weapon zero is native NONE, never an ItemId/WeaponId cast.
                commands[0] = to_q3_usercmd(command, 0);
                Move::Quake3 {
                    commands,
                    count: 1,
                    delta: channel.command_state().is_some_and(|state| {
                        channel.snapshot(state.message_acknowledged).is_some()
                    }),
                }
            }
        };
        let key = if self.protocol == Protocol::Quake3_68 {
            channel.command_key(None)?
        } else {
            Key::default()
        };
        packet::write_with_commands(
            out,
            &movement,
            channel.send_state().sequence,
            key,
            |writer| channel.write_command_records(writer),
        )
    }
    /// NQ duration is the native SERVER frame input, never its ping timestamp.
    pub fn stage(&mut self, bytes: &[u8]) -> Result<usize, packet::Error> {
        self.scratch
            .get_mut(..bytes.len())
            .ok_or(packet::Error::Count)?
            .copy_from_slice(bytes);
        Ok(bytes.len())
    }
    pub fn decode(
        &mut self,
        length: usize,
        sequence: u32,
        server_time: i32,
        frame_ns: u64,
        channel: &mut Channel,
    ) -> Result<Option<UserCmd>, packet::Error> {
        let scratch = &mut self.scratch[..length];
        // Keys borrow retained native strings. Scratch keeps that key stable
        // while optional incoming commands update the opposite direction.
        let mut command_text = [0; 1024];
        let mut message_acknowledged = 0;
        let key = if self.protocol == Protocol::Quake3_68 {
            let ack = packet::acknowledgements(scratch)?;
            message_acknowledged = ack.message as u32;
            let key = channel.command_key(Some(ack))?;
            let n = key.server_command.len();
            command_text[..n].copy_from_slice(key.server_command);
            let key = Key {
                server_command: &command_text[..n],
                ..key
            };
            channel.acknowledge_commands(ack.reliable as u32, ack.message as u32)?;
            key
        } else {
            Key::default()
        };
        let Some(movement) =
            packet::read_with_commands(self.protocol, scratch, sequence, key, |sequence, text| {
                channel.receive_command(sequence, text).map(|_| ())
            })?
        else {
            return Ok(None);
        };
        let command = match movement {
            Move::NetQuake { command, .. } => {
                let mut command = from_q1_move(
                    command,
                    (frame_ns / 1_000_000).min(u64::from(u16::MAX)) as u16,
                    server_time,
                );
                command.duration_ns = frame_ns;
                command
            }
            Move::QuakeWorld { commands, .. } => from_qw_usercmd(
                commands[2],
                self.previous_time.wrapping_add(i32::from(commands[2].msec)),
            ),
            Move::Quake2 {
                last_frame,
                commands,
            } => {
                self.delta_request = (last_frame >= 0).then_some(last_frame as u32);
                from_q2_usercmd(
                    commands[2],
                    self.previous_time.wrapping_add(i32::from(commands[2].msec)),
                )
            }
            Move::Quake3 {
                commands,
                count,
                delta,
            } => {
                // This precedes old-command filtering in native SV_UserMove.
                self.delta_request = delta.then_some(message_acknowledged);
                let native = commands[usize::from(count) - 1];
                if native.server_time <= self.previous_time {
                    return Ok(None);
                }
                from_q3_usercmd(
                    native,
                    self.previous_time,
                    qa_core::primitives::WeaponId::default(),
                )
            }
        };
        self.previous_time = command.server_time_ms;
        Ok(Some(command))
    }
    pub fn decode_output(
        &mut self,
        length: usize,
        sequence: u32,
        channel: &mut Channel,
        mut consume: impl FnMut(Incoming<'_>),
    ) -> Result<Option<u32>, packet::Error> {
        let scratch = &mut self.scratch[..length];
        if self.protocol == Protocol::Quake3_68 {
            channel.decode_server_output(scratch, sequence, |sequence, text| {
                consume(Incoming::ReliableCommand { sequence, text });
            })?;
            return Ok(channel.snapshot(sequence).map(|frame| frame.sequence()));
        }
        if self.protocol != Protocol::Quake2_34
            || channel.endpoint() != qa_core::loopback::Endpoint::Client
        {
            return Err(packet::Error::Context);
        }
        let result = (|| {
            let mut at = 0;
            let mut snapshot = None;
            // Native CL_ParseServerMessage ends at the byte boundary, with no EOF
            // opcode. Each accepted service consumes bytes from fixed scratch.
            while at < scratch.len() {
                match scratch[at] {
                    6 => at += 1, // svc_nop
                    20 => {
                        self.delta_request = None;
                        let ring = channel.q2_snapshots_mut().ok_or(packet::Error::Context)?;
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        let accepted = crate::snapshots::read_q2(&mut reader, ring)?;
                        at += 1 + reader.byte_position();
                        snapshot = if accepted {
                            ring.current().map(|frame| frame.sequence)
                        } else {
                            None
                        };
                        self.delta_request = snapshot;
                    }
                    4 | 10 | 15 => {
                        let mut prints = Prints::new(self.protocol, &scratch[at..]);
                        let print = prints
                            .next()
                            .ok_or(packet::Error::Opcode)?
                            .map_err(|_| packet::Error::Opcode)?;
                        at = scratch.len() - prints.remaining().len();
                        consume(Incoming::Print(print));
                    }
                    // Signon/configstrings, native sounds/effects, inventory and
                    // stuffed commands require their own service bindings later.
                    _ => return Err(packet::Error::Opcode),
                }
            }
            Ok(snapshot)
        })();
        if result.is_err() {
            self.delta_request = None;
        }
        result
    }
}
