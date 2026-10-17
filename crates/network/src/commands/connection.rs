//! The connection owns its packet codec and fixed decode scratch.
mod compression;
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
    command_history::{CommandHistory, HistoryError},
    events::OutputConsumerId,
    primitives::{SnapshotObservation, UserCmd, WeaponId},
};
use std::ops::ControlFlow;

pub struct Commands {
    pub protocol: Protocol,
    scratch: Box<[u8]>,
    previous_time: i32,
    delta_request: Option<u32>,
    previous_command: Option<NativeBackup>,
    qw_players: Option<crate::projection::PlayerProjection>,
    history: Option<CommandHistory>,
    packet_dup: u8,
    q2_stream: Option<Q2Stream>,
    q2_config_end: u32,
    compression: Option<compression::Compression>,
}

#[derive(Clone, Copy)]
enum Q2Stream {
    Config { baselines_follow: bool },
    Baselines,
}

/// SERVER loss recovery retains its last decoded command.
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
fn reset_history_storage(history: &mut Option<CommandHistory>, keep_number: bool) {
    if let Some(history) = history {
        history.reset(if keep_number { history.number() } else { 0 });
    }
}

impl Commands {
    pub fn load(protocol: Protocol, maximum_message: usize) -> Self {
        Self {
            protocol,
            scratch: vec![0; maximum_message].into_boxed_slice(),
            previous_time: 0,
            history: None,
            packet_dup: 1,
            q2_stream: None,
            q2_config_end: protocol.resource_layout().count as u32,
            compression: (protocol == Protocol::Quake2Repro1038)
                .then(compression::Compression::load),
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

    pub fn configure_history(&mut self, capacity: usize) -> Result<(), HistoryError> {
        if let Some(history) = &mut self.history {
            history.resize(capacity)
        } else {
            self.history = Some(CommandHistory::load(capacity, 0)?);
            Ok(())
        }
    }

    pub fn history(&self) -> Option<&CommandHistory> {
        self.history.as_ref()
    }
    pub fn set_packet_dup(&mut self, value: i32) {
        self.packet_dup = value.clamp(0, 5) as u8;
    }
    pub fn record(&mut self, command: UserCmd) -> Result<u64, HistoryError> {
        Ok(self
            .history
            .as_mut()
            .ok_or(HistoryError::Capacity)?
            .append(command))
    }
    pub fn reset_history(&mut self, keep_number: bool) {
        reset_history_storage(&mut self.history, keep_number);
    }
    /// Retire the old compressed download before admitting a new request.
    /// Callers drain any outstanding old reply before starting its replacement.
    pub fn reset_download_stream(&mut self) {
        if let Some(compression) = &mut self.compression {
            compression.download.reset();
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
    /// The registry projects each retained weapon independently; common IDs
    /// never enter the wire. Command production precedes this encoding step.
    pub fn encode(
        &self,
        command: &UserCmd,
        mut weapon_number: impl FnMut(Option<WeaponId>) -> u8,
        channel: &Channel,
        out: &mut [u8],
    ) -> Result<usize, packet::Error> {
        let request = self.snapshot_request(channel);
        let older = |age| self.history.as_ref().and_then(|history| history.get(age));
        let backups = || [older(2), older(1), Some(command)];
        let movement = match self.protocol {
            Protocol::NetQuake15 => Move::NetQuake {
                timestamp: channel
                    .current_snapshot()
                    .map_or(0., |frame| frame.time().seconds() as f32),
                command: to_q1_move(command),
            },
            Protocol::QuakeWorld28 => Move::QuakeWorld {
                loss: 0,
                commands: backups().map(|command| command.map_or(ZERO_QW, to_qw_usercmd)),
                delta_request: request.map(|base| base as u8),
            },
            Protocol::Quake2_34 => Move::Quake2 {
                last_frame: request.map_or(-1, |sequence| sequence as i32),
                commands: backups().map(|command| command.map_or(ZERO_Q2, to_q2_usercmd)),
            },
            Protocol::Quake2Repro1038 => Move::Quake2Repro {
                last_frame: request.map_or(-1, |sequence| sequence as i32),
                commands: {
                    let frame = channel
                        .current_snapshot()
                        .map_or(0, |frame| frame.sequence());
                    backups().map(|command| {
                        command.map_or(ZERO_Q2_RR, |command| to_q2_rr_usercmd(command, frame))
                    })
                },
            },
            Protocol::Quake3_68 => {
                let mut commands = [ZERO_Q3; packet::Q3_COMMANDS];
                let count = self
                    .history
                    .as_ref()
                    .map_or(1, |history| {
                        let sequence = channel
                            .send_state()
                            .sequence
                            .wrapping_sub(1 + u32::from(self.packet_dup));
                        let sent = channel
                            .submitted_command(sequence)
                            .map_or(0, |command| command.number);
                        history
                            .number()
                            .wrapping_sub(sent)
                            .min(history.len() as u64)
                            .min(packet::Q3_COMMANDS as u64) as usize
                    })
                    .max(1);
                for (index, native) in commands[..count].iter_mut().enumerate() {
                    let age = count - index - 1;
                    let retained = if age == 0 {
                        command
                    } else {
                        older(age).ok_or(packet::Error::Context)?
                    };
                    *native = to_q3_usercmd(retained, weapon_number(retained.weapon));
                }
                Move::Quake3 {
                    commands,
                    count: count as u8,
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
    /// Send the existing native ACK/command envelope without producing user input.
    pub fn encode_control(
        &self,
        channel: &Channel,
        out: &mut [u8],
    ) -> Result<usize, packet::Error> {
        let key = if self.protocol == Protocol::Quake3_68 {
            channel.command_key(None)?
        } else {
            Key::default()
        };
        packet::write_control(out, self.protocol, key, |writer| {
            channel.write_command_records(writer)
        })
    }
    /// Copies the native payload into the connection's fixed decode scratch.
    pub fn stage(&mut self, bytes: &[u8]) -> Result<usize, packet::Error> {
        if self.protocol == Protocol::Quake2Repro1038
            && bytes.len() > qa_core::primitives::NATIVE_WIRE_BYTES
        {
            return Err(packet::Error::Count);
        }
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
        let scratch = self.scratch.get_mut(..length).ok_or(packet::Error::Count)?;
        if self.protocol == Protocol::Quake3_68 {
            channel.decode_server_output(scratch, sequence, |output| match output {
                crate::channel::commands::ServerOutput::ReliableCommand { sequence, text } => {
                    consume(Incoming::ReliableCommand { sequence, text });
                }
                crate::channel::commands::ServerOutput::Gamestate(gamestate) => {
                    if let Some(history) = &mut self.history {
                        history.reset(0);
                    }
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
        let raw = &scratch[..];
        let (mut packet_stream, mut download_stream) = match &mut self.compression {
            Some(compression) => (
                Some(&mut compression.packet),
                Some(&mut compression.download),
            ),
            None => (None, None),
        };
        let result = (|| {
            let mut at = 0;
            let mut raw_resume = None;
            let mut expanded_length = 0;
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
            loop {
                let scratch = if raw_resume.is_some() {
                    &packet_stream
                        .as_deref()
                        .ok_or(packet::Error::Context)?
                        .bytes[..expanded_length]
                } else {
                    raw
                };
                if at == scratch.len() {
                    if let Some(resume) = raw_resume.take() {
                        at = resume;
                        continue;
                    }
                    break;
                }
                if let Some(stream) = self.q2_stream {
                    use super::startup::ResourceStream;
                    let mut reader = Reader::new(&scratch[at..], Encoding::Bytes);
                    match stream {
                        Q2Stream::Config { baselines_follow } => {
                            if let Some(config) = crate::outputs::read_q2_configstring(
                                self.protocol,
                                &mut reader,
                                Some(self.q2_config_end),
                            )
                            .map_err(|_| packet::Error::Opcode)?
                            {
                                consume(Incoming::ConfigString(config));
                            } else {
                                self.q2_stream = baselines_follow.then_some(Q2Stream::Baselines);
                                consume(Incoming::ResourceStream(ResourceStream::ConfigStrings));
                            }
                        }
                        Q2Stream::Baselines => {
                            if !channel.read_q2_baseline(&mut reader, true)? {
                                self.q2_stream = None;
                                consume(Incoming::ResourceStream(ResourceStream::Baselines));
                            }
                        }
                    }
                    at += reader.byte_position();
                    continue;
                }
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
                    (Protocol::NetQuake15, 5) => {
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        let entity = reader.read_bits(16)? as u16;
                        at += 1 + reader.byte_position();
                        consume(Incoming::SetView { entity });
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
                    (Protocol::NetQuake15, 11) => {
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        let info = super::startup::NqServerInfo::read(&mut reader)?;
                        channel.reset_client_startup();
                        let storage = channel.nq_state_mut().ok_or(packet::Error::Context)?;
                        storage.ring.reset_gamestate();
                        nq = Some(storage.begin());
                        at += 1 + reader.byte_position();
                        consume(Incoming::ServerInfo(info));
                    }
                    (Protocol::QuakeWorld28, 11) => {
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        let data = super::startup::QwServerData::read(&mut reader)?;
                        at += 1 + reader.byte_position();
                        channel.reset_qw_startup()?;
                        reset_history_storage(&mut self.history, true);
                        self.delta_request = None;
                        snapshot = None;
                        players.fill(None);
                        consume(Incoming::QwServerData(data));
                    }
                    (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 12) => {
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        let data = super::startup::Q2ServerData::read(self.protocol, &mut reader)?;
                        at += 1 + reader.byte_position();
                        channel.reset_q2_startup(data)?;
                        reset_history_storage(&mut self.history, true);
                        self.delta_request = None;
                        self.q2_stream = None;
                        self.q2_config_end = data.resource_layout().count as u32;
                        if let Some(stream) = &mut download_stream {
                            stream.reset();
                        }
                        snapshot = None;
                        consume(Incoming::Q2ServerData(data));
                    }
                    (Protocol::QuakeWorld28, 9)
                    | (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 11) => {
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        let text = reader.read_string(self.protocol.string_policy())?;
                        at += 1 + reader.byte_position();
                        consume(Incoming::StuffText(text));
                    }
                    (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 8) => {
                        at += 1;
                        if let Some(stream) = &mut download_stream {
                            stream.reset();
                        }
                        consume(Incoming::Reconnect);
                    }
                    (Protocol::Quake2Repro1038, 34) => {
                        if raw_resume.is_some() {
                            return Err(packet::Error::Context);
                        }
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        let length = reader.read_bits(16)? as usize;
                        // Native q2repro ignores the announced expanded length.
                        reader.read_bits(16)?;
                        let first = at + 1 + reader.byte_position();
                        let end = first + length;
                        let input = raw.get(first..end).ok_or(packet::Error::Count)?;
                        expanded_length = packet_stream
                            .as_deref_mut()
                            .ok_or(packet::Error::Context)?
                            .packet(input)
                            .map_err(|_| packet::Error::Context)?;
                        raw_resume = Some(end);
                        at = 0;
                    }
                    (Protocol::Quake2Repro1038, 35) => {
                        let result = (|| {
                            // Native compression cannot recursively consume an
                            // already inflated service stream.
                            if raw_resume.is_some() {
                                return Err(crate::downloads::Error::Compression);
                            }
                            let (chunk, count) =
                                crate::downloads::read_chunk_service(35, &scratch[at..])?;
                            let input = chunk.bytes.ok_or(crate::downloads::Error::Size)?;
                            let bytes = download_stream
                                .as_deref_mut()
                                .ok_or(crate::downloads::Error::Compression)?
                                .download(input)
                                .map_err(|_| crate::downloads::Error::Compression)?;
                            consume(Incoming::Download(Ok(crate::downloads::Chunk {
                                percent: chunk.percent,
                                bytes: Some(bytes),
                                compressed: true,
                            })));
                            Ok(count)
                        })();
                        match result {
                            Ok(count) => at += count,
                            Err(error) => {
                                if let Some(stream) = &mut download_stream {
                                    stream.reset();
                                }
                                consume(Incoming::Download(Err(error)));
                                return Err(packet::Error::Context);
                            }
                        }
                    }
                    (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 14) => {
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        channel.read_q2_baseline(&mut reader, false)?;
                        at += 1 + reader.byte_position();
                    }
                    (Protocol::Quake2Repro1038, opcode @ (36 | 38 | 39)) => {
                        at += 1;
                        self.q2_stream = Some(if opcode == 39 {
                            Q2Stream::Baselines
                        } else {
                            Q2Stream::Config {
                                baselines_follow: opcode == 36,
                            }
                        });
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
                    (Protocol::NetQuake15 | Protocol::QuakeWorld28, 22) => {
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        channel.read_q1_baseline(&mut reader)?;
                        at += 1 + reader.byte_position();
                    }
                    (Protocol::NetQuake15, 25) => {
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        let stage = reader.read_bits(8)? as u8;
                        if !(1..=4).contains(&stage) {
                            return Err(packet::Error::Context);
                        }
                        at += 1 + reader.byte_position();
                        consume(Incoming::SignonStage { stage });
                    }
                    (Protocol::NetQuake15, opcode) if opcode & 128 != 0 => {
                        let packet = nq.as_mut().ok_or(packet::Error::Context)?;
                        let mut reader = Reader::new(&scratch[at..], Encoding::Bytes);
                        let update = channel
                            .nq_state_mut()
                            .ok_or(packet::Error::Context)?
                            .entity(&mut reader, packet)?;
                        at += reader.byte_position();
                        let mut record = qa_core::primitives::EntityRecord::default();
                        if !crate::projection::EntityProjection::load(Protocol::NetQuake15).apply(
                            &update.words.ok_or(packet::Error::Context)?,
                            &mut record,
                            namespace,
                        ) {
                            return Err(packet::Error::Context);
                        }
                        consume(Incoming::NqEntityUpdate {
                            number: update.number,
                            model: record.appearance.models[0],
                        });
                    }
                    (Protocol::QuakeWorld28, opcode @ (49 | 50)) => {
                        let mut reader = Reader::new(&scratch[at + 1..], Encoding::Bytes);
                        let value = f32::from_bits(reader.read_bits(32)?);
                        at += 1 + reader.byte_position();
                        consume(if opcode == 49 {
                            Incoming::MaxSpeed(value)
                        } else {
                            Incoming::EntityGravity(value)
                        });
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
                    (Protocol::QuakeWorld28, 41)
                    | (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 16) => {
                        let (chunk, count) =
                            match crate::downloads::read_chunk(self.protocol, &scratch[at..]) {
                                Ok(chunk) => chunk,
                                Err(error) => {
                                    consume(Incoming::Download(Err(error)));
                                    return Err(packet::Error::Context);
                                }
                            };
                        at += count;
                        consume(Incoming::Download(Ok(chunk)));
                    }
                    (Protocol::NetQuake15, 13 | 14 | 17 | 18)
                    | (Protocol::QuakeWorld28, 14 | 40 | 51)
                    | (
                        Protocol::NetQuake15 | Protocol::QuakeWorld28,
                        2 | 6 | 8 | 24 | 26 | 29 | 32,
                    )
                    | (
                        Protocol::Quake2_34 | Protocol::Quake2Repro1038,
                        1 | 2 | 4 | 7 | 9 | 10 | 15 | 32,
                    ) => {
                        let (service, count) =
                            crate::outputs::read_native_service(self.protocol, &scratch[at..])
                                .map_err(|_| packet::Error::Opcode)?;
                        at += count;
                        match service {
                            crate::outputs::NativeService::Peer(peer) => {
                                consume(Incoming::Peer(peer))
                            }
                            crate::outputs::NativeService::CdTrack(track) => {
                                consume(Incoming::CdTrack(track))
                            }
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
                            crate::outputs::NativeService::Pause(paused) => {
                                consume(Incoming::Pause(paused))
                            }
                            crate::outputs::NativeService::Disconnect => {
                                if let Some(stream) = &mut download_stream {
                                    stream.reset();
                                }
                                consume(Incoming::Disconnect { output: None });
                                return Ok(None);
                            }
                        }
                    }
                    (Protocol::QuakeWorld28, 12 | 45 | 46)
                    | (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 13) => {
                        let models = (self.protocol == Protocol::QuakeWorld28 && scratch[at] == 45)
                            .then(|| self.protocol.resource_layout().resources[0]);
                        let mut model_result = Ok(());
                        let read = crate::outputs::read_configstrings(
                            self.protocol,
                            &scratch[at..],
                            |config| {
                                if let Some(models) = models
                                    && config.text == b"progs/player.mdl"
                                {
                                    model_result =
                                        channel.set_qw_player_model(config.index - models.first);
                                }
                                consume(Incoming::ConfigString(config));
                            },
                        )
                        .map_err(|_| packet::Error::Opcode)?;
                        model_result?;
                        at += read.length;
                        if let Some(progress) = read.progress {
                            consume(Incoming::ResourceList(progress));
                        }
                    }
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
            // Failed raw records cannot carry framing into the next packet.
            // Already delivered rows and complete baselines remain accepted.
            self.q2_stream = None;
        }
        result
    }
}
