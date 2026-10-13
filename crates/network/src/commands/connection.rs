//! The connection owns its packet codec and fixed decode scratch.
use super::{
    Q2Cmd, Q2RrCmd, QwCmd, from_q1_move, from_q2_rr_usercmd, from_q2_usercmd, from_q3_usercmd,
    from_qw_usercmd,
    packet::{self, Key, Move, Protocol, ZERO_Q2, ZERO_Q2_RR, ZERO_Q3, ZERO_QW},
    to_q1_move, to_q2_rr_usercmd, to_q2_usercmd, to_q3_usercmd, to_qw_usercmd,
};
use crate::channel::Channel;
use crate::{
    ingress::Incoming,
    message::{Encoding, Reader},
};
use qa_core::{
    events::OutputConsumerId,
    primitives::{SnapshotObservation, UserCmd},
};
use std::ops::ControlFlow;

pub struct Commands {
    pub protocol: Protocol,
    scratch: Box<[u8]>,
    previous_time: i32,
    delta_request: Option<u32>,
    previous_command: Option<NativeBackup>,
    qw_players: Option<crate::projection::PlayerProjection>,
}

/// Native loss recovery retains one command, not an input history.
#[derive(Clone, Copy)]
enum NativeBackup {
    QuakeWorld(QwCmd, Option<SnapshotObservation>),
    Quake2(Q2Cmd, Option<SnapshotObservation>),
    Quake2Repro(Q2RrCmd, Option<SnapshotObservation>),
}

/// QW/Q2 repeat the last command only for gaps smaller than twenty packets.
fn recover<C: Copy>(
    mut dropped: u32,
    previous: (C, Option<SnapshotObservation>),
    commands: [C; 3],
    observation: Option<SnapshotObservation>,
    mut consume: impl FnMut(C, Option<SnapshotObservation>) -> ControlFlow<()>,
) -> ControlFlow<usize, usize> {
    let mut accepted = 0;
    if dropped < 20 {
        while dropped > 2 {
            accepted += 1;
            if consume(previous.0, previous.1).is_break() {
                return ControlFlow::Break(accepted);
            }
            dropped -= 1;
        }
        if dropped > 1 {
            accepted += 1;
            if consume(commands[0], observation).is_break() {
                return ControlFlow::Break(accepted);
            }
        }
        if dropped > 0 {
            accepted += 1;
            if consume(commands[1], observation).is_break() {
                return ControlFlow::Break(accepted);
            }
        }
    }
    accepted += 1;
    if consume(commands[2], observation).is_break() {
        ControlFlow::Break(accepted)
    } else {
        ControlFlow::Continue(accepted)
    }
}
impl Commands {
    pub fn load(protocol: Protocol, maximum_message: usize) -> Self {
        Self {
            protocol,
            scratch: vec![0; maximum_message].into_boxed_slice(),
            previous_time: 0,
            qw_players: (protocol == Protocol::QuakeWorld28)
                .then(|| crate::projection::PlayerProjection::load(protocol, &[])),
            delta_request: None,
            previous_command: match protocol {
                Protocol::QuakeWorld28 => Some(NativeBackup::QuakeWorld(ZERO_QW, None)),
                Protocol::Quake2_34 => Some(NativeBackup::Quake2(ZERO_Q2, None)),
                Protocol::Quake2Repro1038 => Some(NativeBackup::Quake2Repro(ZERO_Q2_RR, None)),
                _ => None,
            },
        }
    }
    /// Native payload delta request. QW SERVER retains clc_delta's low byte;
    /// Q2 SERVER retains clc_move's frame, and CLIENT clears invalid frames.
    pub fn delta_request(&self) -> Option<u32> {
        self.delta_request
    }
    /// Select the native base for the current move and its retained reply
    /// association. Selection is shared by encoding and transmit metadata.
    pub fn snapshot_request(&self, channel: &Channel) -> Option<u32> {
        let request = match self.protocol {
            Protocol::NetQuake15 => None,
            Protocol::QuakeWorld28 => self
                .delta_request
                .filter(|&base| base != 0 && channel.send_state().sequence.wrapping_sub(base) < 63),
            Protocol::Quake2_34 | Protocol::Quake2Repro1038 => self.delta_request,
            Protocol::Quake3_68 => channel
                .command_state()
                .map(|state| state.message_acknowledged),
        };
        request.filter(|&base| channel.snapshot(base).is_some())
    }
    /// Native packet redundancy is caller-supplied. The development host has
    /// one current command and no input history, so its older entries are zero.
    /// The registry supplies `native_weapon`; common IDs never enter the wire.
    pub fn encode(
        &self,
        command: &UserCmd,
        native_weapon: u8,
        channel: &Channel,
        out: &mut [u8],
    ) -> Result<usize, packet::Error> {
        let request = self.snapshot_request(channel);
        let movement = match self.protocol {
            Protocol::NetQuake15 => Move::NetQuake {
                timestamp: channel
                    .current_snapshot()
                    .map_or(0., |frame| frame.time().seconds() as f32),
                command: to_q1_move(command),
            },
            Protocol::QuakeWorld28 => Move::QuakeWorld {
                loss: 0,
                commands: [ZERO_QW, ZERO_QW, to_qw_usercmd(command)],
                delta_request: request.map(|base| base as u8),
            },
            Protocol::Quake2_34 => Move::Quake2 {
                last_frame: request.map_or(-1, |sequence| sequence as i32),
                commands: [ZERO_Q2, ZERO_Q2, to_q2_usercmd(command)],
            },
            Protocol::Quake2Repro1038 => Move::Quake2Repro {
                last_frame: request.map_or(-1, |sequence| sequence as i32),
                commands: [
                    ZERO_Q2_RR,
                    ZERO_Q2_RR,
                    to_q2_rr_usercmd(
                        command,
                        channel
                            .current_snapshot()
                            .map_or(0, |frame| frame.sequence()),
                    ),
                ],
            },
            Protocol::Quake3_68 => {
                let mut commands = [ZERO_Q3; packet::Q3_COMMANDS];
                commands[0] = to_q3_usercmd(command, native_weapon);
                Move::Quake3 {
                    commands,
                    count: 1,
                    delta: request.is_some(),
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
    /// Copies the native payload into the connection's fixed decode scratch.
    pub fn stage(&mut self, bytes: &[u8]) -> Result<usize, packet::Error> {
        self.scratch
            .get_mut(..bytes.len())
            .ok_or(packet::Error::Count)?
            .copy_from_slice(bytes);
        Ok(bytes.len())
    }
    /// Delivers each accepted record synchronously in native packet order.
    /// Return value counts delivered usercmds; text does not enter an input queue.
    pub fn decode(
        &mut self,
        length: usize,
        sequence: u32,
        server_time: i32,
        channel: &mut Channel,
        output: Option<OutputConsumerId>,
        mut consume: impl FnMut(Incoming<'_>) -> ControlFlow<()>,
    ) -> Result<usize, packet::Error> {
        if self.protocol == Protocol::QuakeWorld28 {
            self.delta_request = None;
            channel.align_reply(sequence);
        }
        // Move the sole owned scratch pointer so callback delivery can update
        // connection state while text still borrows this same fixed storage.
        let mut storage = std::mem::take(&mut self.scratch);
        let result = (|| {
            let scratch = storage.get_mut(..length).ok_or(packet::Error::Count)?;
            let mut command_text = [0; crate::channel::commands::STRING_BYTES];
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
                channel.acknowledge_server_state(ack.message as u32);
                key
            } else {
                Key::default()
            };
            let mut packet_request = None;
            if self.protocol == Protocol::QuakeWorld28 {
                // CL_SendCmd places its packet-level ACK after MOVE. Walk the
                // same parser inertly to associate only deltas before a native
                // packet stop. A later error must not suppress live prefix effects.
                let _ =
                    packet::read_with_commands(self.protocol, scratch, sequence, key, |record| {
                        if let packet::Record::Delta(value) = record {
                            packet_request = Some(u32::from(value));
                        }
                        Ok(ControlFlow::Continue(()))
                    });
            }
            let packet_observation =
                packet_request.and_then(|request| channel.snapshot_observation(request));
            let dropped = channel.state().dropped;
            let execution = self.protocol.command_execution();
            let mut accepted = 0;
            packet::read_with_commands(
                self.protocol,
                scratch,
                sequence,
                key,
                |record| match record {
                    packet::Record::Move(movement) => {
                        let batch = matches!(movement, Move::QuakeWorld { .. });
                        if batch && consume(Incoming::MoveBegin).is_break() {
                            return Ok(ControlFlow::Break(()));
                        }
                        let flow = self.decode_move(
                            movement,
                            server_time,
                            dropped,
                            message_acknowledged,
                            packet_observation,
                            channel,
                            |command, native_weapon, observation| {
                                consume(Incoming::Command {
                                    command,
                                    native_weapon,
                                    observation,
                                    execution,
                                    output,
                                })
                            },
                        )?;
                        match flow {
                            ControlFlow::Continue(count) => accepted += count,
                            ControlFlow::Break(count) => {
                                accepted += count;
                                return Ok(ControlFlow::Break(()));
                            }
                        }
                        if batch {
                            return Ok(consume(Incoming::MoveEnd));
                        }
                        Ok(ControlFlow::Continue(()))
                    }
                    packet::Record::Delta(value) => {
                        self.delta_request = Some(u32::from(value));
                        Ok(ControlFlow::Continue(()))
                    }
                    packet::Record::String { sequence, text } => {
                        if let Some(sequence) = sequence
                            && !channel.receive_command(sequence, text)?
                        {
                            return Ok(ControlFlow::Continue(()));
                        }
                        Ok(consume(Incoming::CommandString {
                            protocol: self.protocol,
                            sequence,
                            text,
                            output,
                        }))
                    }
                    packet::Record::Userinfo(text) => Ok(consume(Incoming::Userinfo {
                        protocol: self.protocol,
                        text,
                        output,
                    })),
                    packet::Record::Disconnect => Ok(consume(Incoming::Disconnect { output })),
                },
            )?;
            Ok(accepted)
        })();
        self.scratch = storage;
        result
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "The bounded native admission shares packet provenance and connection state without another queue"
    )]
    fn decode_move(
        &mut self,
        movement: Move,
        server_time: i32,
        dropped: u32,
        message_acknowledged: u32,
        packet_observation: Option<SnapshotObservation>,
        channel: &Channel,
        mut consume: impl FnMut(UserCmd, Option<u8>, Option<SnapshotObservation>) -> ControlFlow<()>,
    ) -> Result<ControlFlow<usize, usize>, packet::Error> {
        let accepted = match movement {
            Move::NetQuake { command, .. } => {
                // NetQuake's wire has no duration. SERVER resolves it once
                // when consuming the latest command for its queued frame span.
                let command = from_q1_move(command, 0, server_time);
                self.previous_time = command.server_time_ms;
                if consume(command, None, None).is_break() {
                    ControlFlow::Break(1)
                } else {
                    ControlFlow::Continue(1)
                }
            }
            Move::QuakeWorld { commands, .. } => {
                let Some(NativeBackup::QuakeWorld(previous, previous_observation)) =
                    self.previous_command
                else {
                    return Err(packet::Error::Context);
                };
                let observation = packet_observation;
                let accepted = recover(
                    dropped,
                    (previous, previous_observation),
                    commands,
                    observation,
                    |native, observation| {
                        self.previous_time =
                            self.previous_time.wrapping_add(i32::from(native.msec));
                        consume(
                            from_qw_usercmd(native, self.previous_time),
                            None,
                            observation,
                        )
                    },
                );
                let mut previous = commands[2];
                // SV_ExecuteClientMessage avoids repeated fire during recovery.
                previous.buttons = 0;
                if accepted.is_continue() {
                    self.previous_command = Some(NativeBackup::QuakeWorld(previous, observation));
                }
                accepted
            }
            Move::Quake2 {
                last_frame,
                commands,
            } => {
                self.delta_request = (last_frame >= 0).then_some(last_frame as u32);
                let Some(NativeBackup::Quake2(previous, previous_observation)) =
                    self.previous_command
                else {
                    return Err(packet::Error::Context);
                };
                let observation = self
                    .delta_request
                    .and_then(|sequence| channel.snapshot_observation(sequence));
                let accepted = recover(
                    dropped,
                    (previous, previous_observation),
                    commands,
                    observation,
                    |native, observation| {
                        self.previous_time =
                            self.previous_time.wrapping_add(i32::from(native.msec));
                        consume(
                            from_q2_usercmd(native, self.previous_time),
                            None,
                            observation,
                        )
                    },
                );
                if accepted.is_continue() {
                    self.previous_command = Some(NativeBackup::Quake2(commands[2], observation));
                }
                accepted
            }
            Move::Quake2Repro {
                last_frame,
                commands,
            } => {
                self.delta_request = (last_frame >= 0).then_some(last_frame as u32);
                let Some(NativeBackup::Quake2Repro(previous, previous_observation)) =
                    self.previous_command
                else {
                    return Err(packet::Error::Context);
                };
                // Protocol 1038 carries one packet-level lastframe, not a
                // sampling frame for each older command. A retained repeat
                // keeps the observation from the packet which admitted it.
                let observation = self
                    .delta_request
                    .and_then(|sequence| channel.snapshot_observation(sequence));
                let accepted = recover(
                    dropped,
                    (previous, previous_observation),
                    commands,
                    observation,
                    |native, observation| {
                        self.previous_time =
                            self.previous_time.wrapping_add(i32::from(native.msec));
                        consume(
                            from_q2_rr_usercmd(native, self.previous_time),
                            None,
                            observation,
                        )
                    },
                );
                if accepted.is_continue() {
                    self.previous_command =
                        Some(NativeBackup::Quake2Repro(commands[2], observation));
                }
                accepted
            }
            Move::Quake3 {
                commands,
                count,
                delta,
            } => {
                // This precedes old-command filtering in native SV_UserMove.
                self.delta_request = delta.then_some(message_acknowledged);
                let observation = channel.snapshot_observation(message_acknowledged);
                let final_time = commands[usize::from(count) - 1].server_time;
                let mut accepted = 0;
                for &native in &commands[..usize::from(count)] {
                    if native.server_time > final_time || native.server_time <= self.previous_time {
                        continue;
                    }
                    let command = from_q3_usercmd(native, self.previous_time, None);
                    self.previous_time = native.server_time;
                    accepted += 1;
                    if consume(command, Some(native.weapon), observation).is_break() {
                        return Ok(ControlFlow::Break(accepted));
                    }
                }
                ControlFlow::Continue(accepted)
            }
        };
        Ok(accepted)
    }
    pub fn decode_output(
        &mut self,
        length: usize,
        sequence: u32,
        channel: &mut Channel,
        namespace: qa_core::primitives::ResourceNamespace,
        mut consume: impl FnMut(Incoming<'_>),
    ) -> Result<Option<u32>, packet::Error> {
        let scratch = &mut self.scratch[..length];
        if self.protocol == Protocol::Quake3_68 {
            channel.decode_server_output(scratch, sequence, |output| match output {
                crate::channel::commands::ServerOutput::ReliableCommand { sequence, text } => {
                    consume(Incoming::ReliableCommand { sequence, text });
                }
                crate::channel::commands::ServerOutput::Gamestate(gamestate) => {
                    consume(Incoming::Gamestate(gamestate));
                }
            })?;
            return Ok(channel.snapshot(sequence).map(|frame| frame.sequence()));
        }
        if !matches!(
            self.protocol,
            Protocol::NetQuake15
                | Protocol::QuakeWorld28
                | Protocol::Quake2_34
                | Protocol::Quake2Repro1038
        ) || channel.endpoint() != qa_core::loopback::Endpoint::Client
        {
            return Err(packet::Error::Context);
        }
        let result = (|| {
            let mut at = 0;
            let mut players = [None; crate::states::QW_PLAYER_SLOTS];
            let mut snapshot = None;
            let mut nq = if self.protocol == Protocol::NetQuake15 {
                Some(
                    channel
                        .nq_state_mut()
                        .ok_or(packet::Error::Context)?
                        .begin(),
                )
            } else {
                None
            };
            // Native CL_ParseServerMessage ends at the byte boundary, with no EOF
            // opcode. Each accepted service consumes bytes from fixed scratch.
            while at < scratch.len() {
                match (self.protocol, scratch[at]) {
                    (Protocol::NetQuake15 | Protocol::QuakeWorld28, 1)
                    | (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 6) => at += 1,
                    (Protocol::NetQuake15 | Protocol::QuakeWorld28, 3)
                    | (Protocol::QuakeWorld28, 38) => {
                        let ((index, value), count) =
                            crate::outputs::read_player_value(self.protocol, &scratch[at..])
                                .map_err(|_| packet::Error::Context)?;
                        at += count;
                        consume(Incoming::PlayerValue { index, value });
                    }
                    (Protocol::NetQuake15 | Protocol::QuakeWorld28, 10) => {
                        let (angles, count) =
                            crate::outputs::read_set_angles(self.protocol, &scratch[at..])
                                .map_err(|_| packet::Error::Context)?;
                        at += count;
                        consume(Incoming::SetAngles(angles));
                    }
                    (Protocol::NetQuake15, 4) => {
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        if reader.read_bits(32)? != self.protocol.number() {
                            return Err(packet::Error::Context);
                        }
                        at += 1 + reader.byte_position();
                    }
                    (Protocol::NetQuake15, 7) => {
                        let packet = nq.as_mut().ok_or(packet::Error::Context)?;
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        packet.time = qa_core::primitives::ThinkTime::Seconds(f64::from(
                            f32::from_bits(reader.read_bits(32)?),
                        ));
                        packet.changed = true;
                        at += 1 + reader.byte_position();
                    }
                    (Protocol::NetQuake15, 15) => {
                        let packet = nq.as_mut().ok_or(packet::Error::Context)?;
                        let storage = channel.nq_state_mut().ok_or(packet::Error::Context)?;
                        let mut reader = Reader::new(&scratch[at..], Encoding::Bytes);
                        packet.player =
                            crate::states::read_nq_player(&mut reader, storage.weapon_is_mask)?;
                        packet.changed = true;
                        at += reader.byte_position();
                    }
                    (Protocol::NetQuake15 | Protocol::QuakeWorld28, 20) => {
                        let (record, length) = crate::outputs::read_static_entity(
                            self.protocol,
                            &scratch[at..],
                            namespace,
                        )
                        .map_err(|_| packet::Error::Context)?;
                        consume(Incoming::StaticEntity(record));
                        at += length;
                    }
                    (Protocol::NetQuake15, opcode) if opcode & 128 != 0 => {
                        let packet = nq.as_mut().ok_or(packet::Error::Context)?;
                        let mut reader = Reader::new(&scratch[at..], Encoding::Bytes);
                        channel
                            .nq_state_mut()
                            .ok_or(packet::Error::Context)?
                            .entity(&mut reader, packet)?;
                        at += reader.byte_position();
                    }
                    (Protocol::QuakeWorld28, 42) => {
                        let (prior, model) = channel
                            .qw_player_context(sequence)
                            .ok_or(packet::Error::Context)?;
                        let mut reader = Reader::new(&scratch[at..], Encoding::Bytes);
                        let player = crate::states::read_qw_player(&mut reader, model, |number| {
                            prior[usize::from(number)]
                        })?;
                        prior[usize::from(player.number)] = player.command;
                        players[usize::from(player.number)] = Some(player);
                        at += reader.byte_position();
                        consume(Incoming::PlayerInfo(player));
                    }
                    (Protocol::QuakeWorld28, opcode @ (47 | 48)) => {
                        self.delta_request = None;
                        let outgoing = channel.send_state().sequence;
                        let ring = channel.qw_snapshots_mut().ok_or(packet::Error::Context)?;
                        let request = ring.requested_base(sequence).flatten();
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        let accepted = crate::snapshots::read_qw_players(
                            &mut reader,
                            ring,
                            sequence,
                            opcode == 48,
                            request,
                            outgoing,
                            &players,
                            self.qw_players.as_ref().ok_or(packet::Error::Context)?,
                            namespace,
                        )?;
                        at += 1 + reader.byte_position();
                        snapshot = if accepted {
                            ring.current().map(|frame| frame.sequence)
                        } else {
                            None
                        };
                        self.delta_request = snapshot;
                    }
                    (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 20) => {
                        self.delta_request = None;
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        let accepted = channel.read_q2_snapshot(&mut reader)?;
                        at += 1 + reader.byte_position();
                        snapshot = if accepted {
                            channel.current_snapshot().map(|frame| frame.sequence())
                        } else {
                            None
                        };
                        self.delta_request = snapshot;
                    }
                    (Protocol::NetQuake15, 18)
                    | (Protocol::NetQuake15 | Protocol::QuakeWorld28, 8 | 26 | 29)
                    | (
                        Protocol::Quake2_34 | Protocol::Quake2Repro1038,
                        1 | 2 | 4 | 9 | 10 | 15 | 32,
                    ) => {
                        let (service, count) =
                            crate::outputs::read_native_service(self.protocol, &scratch[at..])
                                .map_err(|_| packet::Error::Opcode)?;
                        at += count;
                        match service {
                            crate::outputs::NativeService::Cue(cue) => consume(Incoming::Cue {
                                cue,
                                current: channel.current_snapshot(),
                            }),
                            crate::outputs::NativeService::Print(print) => {
                                consume(Incoming::Print(print))
                            }
                            crate::outputs::NativeService::Particle(particle) => {
                                consume(Incoming::Particle(particle))
                            }
                            crate::outputs::NativeService::Nop => {}
                        }
                    }
                    (Protocol::QuakeWorld28, 12 | 45 | 46)
                    | (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 13) => {
                        let count = crate::outputs::read_configstrings(
                            self.protocol,
                            &scratch[at..],
                            |config| consume(Incoming::ConfigString(config)),
                        )
                        .map_err(|_| packet::Error::Opcode)?;
                        at += count;
                    }
                    // Signon, temp entities, inventory and
                    // stuffed commands require their own service bindings later.
                    _ => return Err(packet::Error::Opcode),
                }
            }
            if let Some(packet) = nq {
                snapshot = channel
                    .nq_state_mut()
                    .ok_or(packet::Error::Context)?
                    .finish(sequence, packet);
            }
            Ok(snapshot)
        })();
        if result.is_err() {
            self.delta_request = None;
        }
        result
    }
}
