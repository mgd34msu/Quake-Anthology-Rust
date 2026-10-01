//! Quake demo inputs (donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q1-demo.ts`).
//!
//! The demo readers live in the unported `network/q1/demos.ts` and the remote
//! presentations in the out-of-scope `remote-q1.ts`/`remote-qw.ts`, so this
//! port defines the structural traits both sides implement at merge time.

use std::collections::BTreeMap;

use qa_core::math::Vec3;
use qa_net::common::commands::UserCommand;
use qa_net::q1_net::{
    NetQuakeDecoder, NetQuakeMessage, NqUnit, Q1NetError, QuakeWorldDecoder, QuakeWorldMessage, QwUnit,
    RereleaseMessages,
};
use qa_net::q1_wide::{NqProfile, QwProfile};
use thiserror::Error;

use super::qw_types::{qw_server_data, QwServerData, QwTypesError};

/// One rendered-frame advance (`Q1DemoFrame`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1DemoFrame {
    /// Elapsed playback seconds for this frame.
    pub elapsed_seconds: f64,
    /// Rendered frame number.
    pub frame: u64,
    /// Timedemo (unthrottled) advance.
    pub timedemo: bool,
}

/// Demo terminal reason (`Q1DemoEnd`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1DemoEnd {
    /// Byte stream exhausted.
    Eof,
    /// The recording carries a disconnect.
    RecordedDisconnect,
    /// The input was closed.
    Closed,
}

/// Demo progress phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1DemoPhase {
    /// Signon/pre-cache in progress.
    Loading,
    /// Playback active.
    Active,
    /// Terminal.
    Ended(Q1DemoEnd),
}

/// Demo progress (`Q1DemoProgress`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1DemoProgress {
    /// Recorded playback seconds.
    pub recorded_seconds: f64,
    /// Records consumed by this advance.
    pub records_read: u64,
    /// Progress phase.
    pub phase: Q1DemoPhase,
}

/// Demo input failure.
#[derive(Debug, Error)]
pub enum Q1DemoError {
    /// Donor message.
    #[error("{0}")]
    Message(String),
    /// Wire decode failure.
    #[error(transparent)]
    Net(#[from] Q1NetError),
    /// Server data extraction failure.
    #[error(transparent)]
    Types(#[from] QwTypesError),
}

/// Single-flight demo advancement (`DemoOperation`).
#[derive(Debug)]
struct DemoOperation {
    /// Advance in progress.
    busy: bool,
    /// Terminal reason.
    ended: Option<Q1DemoEnd>,
    /// Last advanced frame number.
    frame: Option<u64>,
}

impl DemoOperation {
    /// Retire the operation (`close`).
    fn close(&mut self) {
        self.ended = Some(Q1DemoEnd::Closed);
    }

    /// Terminal reason (`reason`).
    fn reason(&self) -> Option<Q1DemoEnd> {
        self.ended
    }

    /// Record the first terminal reason (`finish`).
    fn finish(&mut self, reason: Q1DemoEnd) {
        if self.ended.is_none() {
            self.ended = Some(reason);
        }
    }

    /// Begin an advance; reports whether the work should run (`run` guard).
    fn begin(&mut self, frame: &Q1DemoFrame) -> Result<bool, Q1DemoError> {
        if self.busy {
            return Err(Q1DemoError::Message(
                "Demo advancement is already in progress".to_string(),
            ));
        }
        if !frame.elapsed_seconds.is_finite() || frame.elapsed_seconds < 0.0 {
            return Err(Q1DemoError::Message("Invalid demo frame clock".to_string()));
        }
        if self.ended.is_some() || self.frame == Some(frame.frame) {
            return Ok(false);
        }
        if self.frame.is_some_and(|seen| frame.frame < seen) {
            return Err(Q1DemoError::Message("Demo frame number moved backwards".to_string()));
        }
        self.busy = true;
        self.frame = Some(frame.frame);
        Ok(true)
    }

    /// End an advance, applying the work outcome (`run` finally).
    fn end(&mut self, outcome: Result<Option<Q1DemoEnd>, Q1DemoError>) -> Result<Option<Q1DemoEnd>, Q1DemoError> {
        self.busy = false;
        match outcome {
            Ok(reason) => {
                if let Some(reason) = reason {
                    self.finish(reason);
                }
                Ok(self.ended)
            }
            Err(error) => Err(error),
        }
    }
}

/// One NetQuake demo record (`NetQuakeDemoRecord`).
#[derive(Debug, Clone, PartialEq)]
pub struct NetQuakeDemoRecord {
    /// Recorded view angles.
    pub view_angles: Vec3,
    /// Recorded message bytes.
    pub message: Vec<u8>,
}

/// NetQuake demo reader (`NetQuakeDemoReader`).
pub trait NetQuakeDemoReader {
    /// Next record, or `None` at end of stream.
    fn next_record(&mut self) -> Option<NetQuakeDemoRecord>;
    /// Forced CD track from the demo header (-1 for none).
    fn forced_track(&self) -> i32;
}

/// NetQuake demo presentation (`Q1RemotePresentation` surface used here).
pub trait NetQuakeDemoRemote {
    /// Signon and output both ready.
    fn demo_ready(&self) -> bool;
    /// Recorded seconds rendered so far.
    fn recorded_seconds(&self) -> f64;
    /// Receive decoded messages.
    fn receive(&mut self, messages: &[NetQuakeMessage], milliseconds: f64, assert_current: &dyn Fn());
    /// Set the demo view angles.
    fn set_demo_view_angles(&mut self, angles: &Vec3, absolute: bool);
    /// Sample the demo clock (seconds).
    fn sample_demo(&mut self, seconds: f64);
}

/// NetQuake demo input (`NetQuakeDemoInput`).
pub struct NetQuakeDemoInput<R, H> {
    reader: R,
    remote: H,
    decoder: NetQuakeDecoder,
    operation: DemoOperation,
    signon: u8,
    clock: f64,
    primed: bool,
}

impl<R: NetQuakeDemoReader, H: NetQuakeDemoRemote> NetQuakeDemoInput<R, H> {
    /// Build a demo input.
    pub fn new(reader: R, remote: H) -> Self {
        Self {
            reader,
            remote,
            decoder: NetQuakeDecoder::new(NqProfile::Netquake, RereleaseMessages::KnownRetail, true),
            operation: DemoOperation {
                busy: false,
                ended: None,
                frame: None,
            },
            signon: 0,
            clock: 0.0,
            primed: false,
        }
    }

    /// Borrow the reader.
    pub fn reader(&self) -> &R {
        &self.reader
    }

    /// Borrow the remote.
    pub fn remote(&self) -> &H {
        &self.remote
    }

    /// Mutably borrow the remote.
    pub fn remote_mut(&mut self) -> &mut H {
        &mut self.remote
    }

    /// Retire the input (`close`).
    pub fn close(&mut self) {
        self.operation.close();
    }

    /// Whether signon completed with player and output (`ready`).
    fn ready(&self) -> bool {
        self.signon == 4 && self.remote.demo_ready()
    }

    /// Advance one rendered frame (`advance`).
    pub fn advance(&mut self, frame: &Q1DemoFrame) -> Result<Q1DemoProgress, Q1DemoError> {
        let mut records = 0u64;
        if !self.operation.begin(frame)? {
            return Ok(self.progress(records));
        }
        let outcome = self.work(frame, &mut records);
        let reason = self.operation.end(outcome)?;
        Ok(self.progress_with(records, reason))
    }

    /// Current progress with the live operation reason.
    fn progress(&self, records: u64) -> Q1DemoProgress {
        self.progress_with(records, self.operation.reason())
    }

    /// Current progress with an explicit reason.
    fn progress_with(&self, records: u64, reason: Option<Q1DemoEnd>) -> Q1DemoProgress {
        let phase = match reason {
            None => {
                if self.ready() {
                    Q1DemoPhase::Active
                } else {
                    Q1DemoPhase::Loading
                }
            }
            Some(reason) => Q1DemoPhase::Ended(reason),
        };
        Q1DemoProgress {
            recorded_seconds: self.clock,
            records_read: records,
            phase,
        }
    }

    /// Advance body; returns the first terminal reason when one lands.
    fn work(&mut self, frame: &Q1DemoFrame, records: &mut u64) -> Result<Option<Q1DemoEnd>, Q1DemoError> {
        let mut ended = None;
        if self.primed {
            self.clock += frame.elapsed_seconds;
        }
        loop {
            if self.ready() && !frame.timedemo && self.clock <= self.remote.recorded_seconds() {
                break;
            }
            let Some(record) = self.reader.next_record() else {
                ended = Some(Q1DemoEnd::Eof);
                break;
            };
            *records += 1;
            let mut messages = self.decoder.decode(&record.message)?;
            // Sync hosts resolve inline, so retirement cannot interleave; the
            // closure only carries the donor hook.
            let assert_current = || {};
            let forced = self.reader.forced_track();
            if forced != -1 {
                for message in &mut messages {
                    if let NetQuakeMessage::CdTrack { track, .. } = message {
                        *track = (forced & 255) as u8;
                    }
                }
            }
            if messages
                .iter()
                .any(|message| matches!(message, NetQuakeMessage::ServerInfo { .. }))
            {
                self.signon = 0;
                self.primed = false;
                self.clock = 0.0;
            }
            self.remote.receive(&messages, self.clock * 1000.0, &assert_current);
            self.remote.set_demo_view_angles(&record.view_angles, true);
            for message in &messages {
                match message {
                    NetQuakeMessage::Signon { stage } => {
                        if *stage <= self.signon || *stage > 4 {
                            return Err(Q1DemoError::Message(format!(
                                "Invalid demo signon stage {stage} after {}",
                                self.signon
                            )));
                        }
                        self.signon = *stage;
                    }
                    NetQuakeMessage::Entity { .. } if self.signon == 3 => {
                        self.signon = 4;
                    }
                    NetQuakeMessage::Unit(NqUnit::Disconnect) => {
                        ended.get_or_insert(Q1DemoEnd::RecordedDisconnect);
                    }
                    _ => {}
                }
            }
            if !self.primed && self.ready() {
                self.clock = self.remote.recorded_seconds();
                self.primed = true;
                break;
            }
            if self.ready() && frame.timedemo {
                self.clock = self.remote.recorded_seconds();
                break;
            }
            if ended.is_some() {
                break;
            }
        }
        self.remote.sample_demo(self.clock);
        Ok(ended)
    }
}

/// One QuakeWorld demo record (`QuakeWorldDemoRecord`).
#[derive(Debug, Clone, PartialEq)]
pub enum QuakeWorldDemoRecord {
    /// Predicted input command.
    Command {
        seconds: f64,
        command: UserCommand,
        view_angles: Vec3,
    },
    /// Recorded netchannel packet.
    Packet { seconds: f64, message: Vec<u8> },
    /// Sequence resynchronization.
    Sequences { seconds: f64, outgoing: i32, incoming: i32 },
}

/// QuakeWorld demo reader (`QuakeWorldDemoReader`).
pub trait QuakeWorldDemoReader {
    /// Next record, or `None` at end of stream.
    fn next_record(&mut self) -> Option<QuakeWorldDemoRecord>;
}

/// QuakeWorld demo prediction hooks.
pub trait QwDemoPrediction {
    /// A command was sent (or replayed).
    fn sent(&mut self, sequence: u32, command: &UserCommand, milliseconds: f64);
    /// The server acknowledged a command.
    fn acknowledged(&mut self, sequence: u32, milliseconds: f64);
}

/// QuakeWorld shared demo presentation.
pub trait QwDemoShared {
    /// Set the demo view angles.
    fn set_demo_view_angles(&mut self, angles: &Vec3, absolute: bool);
    /// Sample the demo clock (seconds).
    fn sample_demo(&mut self, seconds: f64);
}

/// QuakeWorld demo presentation (`QwRemotePresentation` surface used here).
pub trait QuakeWorldDemoRemote {
    /// World, output, and player all ready.
    fn demo_ready(&self) -> bool;
    /// Shared presentation surface.
    fn shared(&mut self) -> &mut dyn QwDemoShared;
    /// Prediction hooks.
    fn prediction(&mut self) -> &mut dyn QwDemoPrediction;
    /// A server-data message arrived.
    fn server_data(&mut self, message: &QuakeWorldMessage);
    /// Precache completed.
    fn game_state(&mut self, data: &QwServerData, models: &[String], sounds: &[String], assert_current: &dyn Fn());
    /// Receive decoded messages.
    fn receive(&mut self, messages: &[QuakeWorldMessage], milliseconds: f64, assert_current: &dyn Fn());
    /// Sample the presentation clock (milliseconds).
    fn sample_presentation(&mut self, milliseconds: f64);
}

/// Buffered demo command awaiting acknowledgement.
#[derive(Debug, Clone, PartialEq)]
struct QwDemoCommand {
    /// Input command.
    command: UserCommand,
    /// Recorded seconds.
    seconds: f64,
}

/// QuakeWorld demo input (`QuakeWorldDemoInput`).
pub struct QuakeWorldDemoInput<R, H> {
    reader: R,
    remote: H,
    decoder: QuakeWorldDecoder,
    operation: DemoOperation,
    pending: Option<QuakeWorldDemoRecord>,
    data: Option<QwServerData>,
    models: Vec<String>,
    sounds: Vec<String>,
    model_list_complete: bool,
    sound_list_complete: bool,
    world_ready: bool,
    incoming: i32,
    outgoing: i32,
    acknowledged: i32,
    clock: f64,
    commands: BTreeMap<i32, QwDemoCommand>,
}

impl<R: QuakeWorldDemoReader, H: QuakeWorldDemoRemote> QuakeWorldDemoInput<R, H> {
    /// Build a demo input.
    pub fn new(mut reader: R, remote: H) -> Self {
        let pending = reader.next_record();
        let clock = pending.as_ref().map_or(0.0, record_seconds);
        Self {
            reader,
            remote,
            decoder: QuakeWorldDecoder::new(QwProfile::Quakeworld),
            operation: DemoOperation {
                busy: false,
                ended: None,
                frame: None,
            },
            pending,
            data: None,
            models: Vec::new(),
            sounds: Vec::new(),
            model_list_complete: false,
            sound_list_complete: false,
            world_ready: false,
            incoming: -1,
            outgoing: 0,
            acknowledged: -1,
            clock,
            commands: BTreeMap::new(),
        }
    }

    /// Borrow the reader.
    pub fn reader(&self) -> &R {
        &self.reader
    }

    /// Borrow the remote.
    pub fn remote(&self) -> &H {
        &self.remote
    }

    /// Mutably borrow the remote.
    pub fn remote_mut(&mut self) -> &mut H {
        &mut self.remote
    }

    /// Retire the input (`close`).
    pub fn close(&mut self) {
        self.operation.close();
    }

    /// Whether the world, output, and player are ready (`ready`).
    fn ready(&self) -> bool {
        self.world_ready && self.remote.demo_ready()
    }

    /// Advance one rendered frame (`advance`).
    pub fn advance(&mut self, frame: &Q1DemoFrame) -> Result<Q1DemoProgress, Q1DemoError> {
        let mut records = 0u64;
        if !self.operation.begin(frame)? {
            return Ok(self.progress(records));
        }
        let outcome = self.work(frame, &mut records);
        let reason = self.operation.end(outcome)?;
        Ok(self.progress_with(records, reason))
    }

    /// Current progress with the live operation reason.
    fn progress(&self, records: u64) -> Q1DemoProgress {
        self.progress_with(records, self.operation.reason())
    }

    /// Current progress with an explicit reason.
    fn progress_with(&self, records: u64, reason: Option<Q1DemoEnd>) -> Q1DemoProgress {
        let phase = match reason {
            None => {
                if self.ready() {
                    Q1DemoPhase::Active
                } else {
                    Q1DemoPhase::Loading
                }
            }
            Some(reason) => Q1DemoPhase::Ended(reason),
        };
        Q1DemoProgress {
            recorded_seconds: self.clock,
            records_read: records,
            phase,
        }
    }

    /// Advance body; returns the first terminal reason when one lands.
    fn work(&mut self, frame: &Q1DemoFrame, records: &mut u64) -> Result<Option<Q1DemoEnd>, Q1DemoError> {
        let mut ended = None;
        if self.ready() && !frame.timedemo {
            self.clock += frame.elapsed_seconds;
        }
        let group = self.pending.as_ref().map_or(self.clock, record_seconds);
        while self.pending.is_some() && ended.is_none() {
            let Some(record) = self.pending.take() else {
                break;
            };
            let seconds = record_seconds(&record);
            if !frame.timedemo && self.ready() && self.clock + 1.0 < seconds {
                self.clock = seconds - 1.0;
            }
            if frame.timedemo {
                if seconds > group {
                    self.pending = Some(record);
                    break;
                }
            } else if self.ready() && seconds > self.clock {
                self.pending = Some(record);
                break;
            }
            if frame.timedemo || !self.ready() {
                self.clock = seconds;
            }
            *records += 1;
            match record {
                QuakeWorldDemoRecord::Sequences { outgoing, incoming, .. } => {
                    self.outgoing = outgoing;
                    self.incoming = incoming;
                }
                QuakeWorldDemoRecord::Command {
                    command, view_angles, ..
                } => {
                    self.remote.shared().set_demo_view_angles(&view_angles, false);
                    let sequence = self.outgoing;
                    self.outgoing += 1;
                    self.commands.insert(sequence, QwDemoCommand { command, seconds });
                    while self.commands.len() > 64 {
                        let Some(first) = self.commands.keys().next().copied() else {
                            break;
                        };
                        self.commands.remove(&first);
                    }
                    let ready = self.ready();
                    let acknowledged = self.acknowledged;
                    Self::replay_commands(&mut self.remote, &mut self.commands, ready, acknowledged);
                }
                QuakeWorldDemoRecord::Packet { message, .. } => {
                    if self.packet(&message, seconds)? && ended.is_none() {
                        ended = Some(Q1DemoEnd::RecordedDisconnect);
                    }
                }
            }
            if ended.is_none() {
                self.pending = self.reader.next_record();
            }
        }
        if self.pending.is_none() && ended.is_none() {
            ended = Some(Q1DemoEnd::Eof);
        }
        self.remote.shared().sample_demo(self.clock);
        self.remote.sample_presentation(self.clock * 1000.0);
        Ok(ended)
    }

    /// Replay unacknowledged commands (`replayCommands`).
    fn replay_commands(remote: &mut H, commands: &mut BTreeMap<i32, QwDemoCommand>, ready: bool, acknowledged: i32) {
        if !ready {
            return;
        }
        commands.retain(|sequence, _| *sequence > acknowledged);
        let prediction = remote.prediction();
        for (sequence, record) in commands.iter() {
            prediction.sent(*sequence as u32, &record.command, record.seconds * 1000.0);
        }
    }

    /// Handle a recorded packet (`packet`); reports a recorded disconnect.
    fn packet(&mut self, message: &[u8], seconds: f64) -> Result<bool, Q1DemoError> {
        if message.len() < 8 {
            return Err(Q1DemoError::Message("Truncated QWD netchannel header".to_string()));
        }
        let mut header = [0u8; 8];
        header.copy_from_slice(&message[..8]);
        if i32::from_le_bytes(header[0..4].try_into().expect("header")) == -1 {
            return Ok(false);
        }
        let sequence = (u32::from_le_bytes(header[0..4].try_into().expect("header")) & 0x7fff_ffff) as i32;
        let acknowledged = (u32::from_le_bytes(header[4..8].try_into().expect("header")) & 0x7fff_ffff) as i32;
        if sequence <= self.incoming {
            return Ok(false);
        }
        self.incoming = sequence;
        self.acknowledged = acknowledged;
        self.remote
            .prediction()
            .acknowledged(acknowledged as u32, seconds * 1000.0);
        // Sync hosts resolve inline, so retirement cannot interleave; the
        // closure only carries the donor hook.
        let assert_current = || {};
        let messages = self.decoder.decode(&message[8..], sequence as u32)?;
        for message in &messages {
            match message {
                QuakeWorldMessage::ServerData { player_slot, .. } => {
                    if *player_slot >= 32 {
                        return Err(Q1DemoError::Message(
                            "QWD requires a valid native player slot".to_string(),
                        ));
                    }
                    self.data = Some(qw_server_data(message)?);
                    self.models.clear();
                    self.sounds.clear();
                    self.model_list_complete = false;
                    self.sound_list_complete = false;
                    self.world_ready = false;
                    self.commands.clear();
                    self.remote.server_data(message);
                }
                QuakeWorldMessage::ModelList { first, names, next }
                | QuakeWorldMessage::SoundList { first, names, next } => {
                    let data = self
                        .data
                        .as_ref()
                        .ok_or_else(|| Q1DemoError::Message("QWD precache before serverdata".to_string()))?;
                    let sounds = matches!(message, QuakeWorldMessage::SoundList { .. });
                    let list = if sounds { &mut self.sounds } else { &mut self.models };
                    if u32::from(*first) != list.len() as u32 {
                        return Err(Q1DemoError::Message("Non-contiguous QWD precache list".to_string()));
                    }
                    list.extend(names.iter().cloned());
                    if *next == 0 {
                        if sounds {
                            self.sound_list_complete = true;
                        } else {
                            self.model_list_complete = true;
                        }
                    }
                    if !self.world_ready && self.model_list_complete && self.sound_list_complete {
                        let data = data.clone();
                        let models = self.models.clone();
                        let sounds = self.sounds.clone();
                        self.remote.game_state(&data, &models, &sounds, &assert_current);
                        self.world_ready = true;
                    }
                }
                _ => {}
            }
        }
        self.remote.receive(&messages, seconds * 1000.0, &assert_current);
        let ready = self.ready();
        let acknowledged = self.acknowledged;
        Self::replay_commands(&mut self.remote, &mut self.commands, ready, acknowledged);
        Ok(messages
            .iter()
            .any(|message| matches!(message, QuakeWorldMessage::Unit(QwUnit::Disconnect))))
    }
}

/// Recorded seconds of a QuakeWorld demo record.
fn record_seconds(record: &QuakeWorldDemoRecord) -> f64 {
    match *record {
        QuakeWorldDemoRecord::Command { seconds, .. }
        | QuakeWorldDemoRecord::Packet { seconds, .. }
        | QuakeWorldDemoRecord::Sequences { seconds, .. } => seconds,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_net::msg::MsgWriter;
    use qa_net::q1_net::{write_net_quake_message, write_quake_world_message, QwMoveVariables};

    /// Queued NetQuake reader.
    struct VecNQReader {
        records: Vec<NetQuakeDemoRecord>,
        forced: i32,
    }

    impl NetQuakeDemoReader for VecNQReader {
        fn next_record(&mut self) -> Option<NetQuakeDemoRecord> {
            if self.records.is_empty() {
                None
            } else {
                Some(self.records.remove(0))
            }
        }

        fn forced_track(&self) -> i32 {
            self.forced
        }
    }

    /// Recording NetQuake remote.
    struct MockNQRemote {
        ready: bool,
        recorded: f64,
        receives: Vec<(usize, f64)>,
        angles: Vec<(Vec3, bool)>,
        samples: Vec<f64>,
        tracks: Vec<u8>,
    }

    impl NetQuakeDemoRemote for MockNQRemote {
        fn demo_ready(&self) -> bool {
            self.ready
        }

        fn recorded_seconds(&self) -> f64 {
            self.recorded
        }

        fn receive(&mut self, messages: &[NetQuakeMessage], milliseconds: f64, _assert_current: &dyn Fn()) {
            self.receives.push((messages.len(), milliseconds));
            for message in messages {
                if let NetQuakeMessage::CdTrack { track, .. } = message {
                    self.tracks.push(*track);
                }
            }
        }

        fn set_demo_view_angles(&mut self, angles: &Vec3, absolute: bool) {
            self.angles.push((*angles, absolute));
        }

        fn sample_demo(&mut self, seconds: f64) {
            self.samples.push(seconds);
        }
    }

    /// Queued QuakeWorld reader.
    struct VecQWReader {
        records: Vec<QuakeWorldDemoRecord>,
    }

    impl QuakeWorldDemoReader for VecQWReader {
        fn next_record(&mut self) -> Option<QuakeWorldDemoRecord> {
            if self.records.is_empty() {
                None
            } else {
                Some(self.records.remove(0))
            }
        }
    }

    /// Recording prediction hooks.
    struct MockPred {
        sent: Vec<(u32, f64)>,
        acked: Vec<(u32, f64)>,
    }

    impl QwDemoPrediction for MockPred {
        fn sent(&mut self, sequence: u32, _command: &UserCommand, milliseconds: f64) {
            self.sent.push((sequence, milliseconds));
        }

        fn acknowledged(&mut self, sequence: u32, milliseconds: f64) {
            self.acked.push((sequence, milliseconds));
        }
    }

    /// Recording shared surface.
    struct MockShared {
        angles: Vec<(Vec3, bool)>,
        samples: Vec<f64>,
    }

    impl QwDemoShared for MockShared {
        fn set_demo_view_angles(&mut self, angles: &Vec3, absolute: bool) {
            self.angles.push((*angles, absolute));
        }

        fn sample_demo(&mut self, seconds: f64) {
            self.samples.push(seconds);
        }
    }

    /// Recording QuakeWorld remote.
    struct MockQWRemote {
        ready: bool,
        shared: MockShared,
        prediction: MockPred,
        server_datas: u32,
        game_states: Vec<(usize, usize)>,
        receives: Vec<(usize, f64)>,
        presentations: Vec<f64>,
    }

    impl QuakeWorldDemoRemote for MockQWRemote {
        fn demo_ready(&self) -> bool {
            self.ready
        }

        fn shared(&mut self) -> &mut dyn QwDemoShared {
            &mut self.shared
        }

        fn prediction(&mut self) -> &mut dyn QwDemoPrediction {
            &mut self.prediction
        }

        fn server_data(&mut self, _message: &QuakeWorldMessage) {
            self.server_datas += 1;
        }

        fn game_state(
            &mut self,
            _data: &QwServerData,
            models: &[String],
            sounds: &[String],
            _assert_current: &dyn Fn(),
        ) {
            self.game_states.push((models.len(), sounds.len()));
        }

        fn receive(&mut self, messages: &[QuakeWorldMessage], milliseconds: f64, _assert_current: &dyn Fn()) {
            self.receives.push((messages.len(), milliseconds));
        }

        fn sample_presentation(&mut self, milliseconds: f64) {
            self.presentations.push(milliseconds);
        }
    }

    fn nq_bytes(messages: &[NetQuakeMessage]) -> Vec<u8> {
        let mut writer = MsgWriter::new(65535, false);
        for message in messages {
            write_net_quake_message(
                &mut writer,
                NqProfile::Netquake,
                message,
                RereleaseMessages::KnownRetail,
                true,
            )
            .expect("write");
        }
        writer.bytes().to_vec()
    }

    fn qw_bytes(messages: &[QuakeWorldMessage]) -> Vec<u8> {
        let mut writer = MsgWriter::new(65535, false);
        for message in messages {
            write_quake_world_message(&mut writer, QwProfile::Quakeworld, message).expect("write");
        }
        writer.bytes().to_vec()
    }

    fn qw_packet(sequence: u32, acknowledged: u32, payload: &[u8]) -> Vec<u8> {
        let mut message = Vec::with_capacity(8 + payload.len());
        message.extend_from_slice(&sequence.to_le_bytes());
        message.extend_from_slice(&acknowledged.to_le_bytes());
        message.extend_from_slice(payload);
        message
    }

    fn test_angles() -> Vec3 {
        Vec3 { x: 1.0, y: 2.0, z: 3.0 }
    }

    fn nq_server_info() -> NetQuakeMessage {
        NetQuakeMessage::ServerInfo {
            protocol: NqProfile::Netquake,
            max_clients: 8,
            game_type: 0,
            level: "dm1".to_string(),
            models: vec!["progs/player.mdl".to_string()],
            sounds: vec!["weapons/r_exp3.wav".to_string()],
        }
    }

    fn qw_server_data(slot: u8) -> QuakeWorldMessage {
        QuakeWorldMessage::ServerData {
            protocol: QwProfile::Quakeworld,
            server_count: 1,
            game_directory: "qw".to_string(),
            player_slot: slot,
            spectator: false,
            level: "dm1".to_string(),
            move_variables: QwMoveVariables {
                gravity: 800.0,
                stop_speed: 100.0,
                max_speed: 320.0,
                spectator_max_speed: 500.0,
                accelerate: 10.0,
                air_accelerate: 1.0,
                water_accelerate: 1.0,
                friction: 4.0,
                water_friction: 4.0,
                entity_gravity: 1.0,
            },
        }
    }

    fn qw_command() -> UserCommand {
        UserCommand::Q1Quakeworld {
            milliseconds: 50.0,
            angles: [0.0; 3],
            forward_move: 100.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0.0,
            impulse: 0.0,
        }
    }

    #[test]
    fn demo_frame_guards() {
        let reader = VecNQReader {
            records: Vec::new(),
            forced: -1,
        };
        let remote = MockNQRemote {
            ready: true,
            recorded: 0.0,
            receives: Vec::new(),
            angles: Vec::new(),
            samples: Vec::new(),
            tracks: Vec::new(),
        };
        let mut input = NetQuakeDemoInput::new(reader, remote);
        for elapsed in [f64::NAN, f64::INFINITY, -1.0] {
            let error = input
                .advance(&Q1DemoFrame {
                    elapsed_seconds: elapsed,
                    frame: 0,
                    timedemo: false,
                })
                .expect_err("clock");
            assert_eq!(error.to_string(), "Invalid demo frame clock");
        }
        let progress = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 1,
                timedemo: false,
            })
            .expect("eof");
        assert_eq!(progress.phase, Q1DemoPhase::Ended(Q1DemoEnd::Eof));
        // Ended inputs short-circuit without consuming.
        let progress = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 2,
                timedemo: false,
            })
            .expect("ended");
        assert_eq!(progress.records_read, 0);
        assert_eq!(progress.phase, Q1DemoPhase::Ended(Q1DemoEnd::Eof));
    }

    #[test]
    fn demo_frame_ordering() {
        let reader = VecNQReader {
            records: Vec::new(),
            forced: -1,
        };
        let remote = MockNQRemote {
            ready: false,
            recorded: 0.0,
            receives: Vec::new(),
            angles: Vec::new(),
            samples: Vec::new(),
            tracks: Vec::new(),
        };
        let mut input = NetQuakeDemoInput::new(reader, remote);
        input.close();
        // Closed inputs report closed without work.
        let progress = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 0,
                timedemo: false,
            })
            .expect("closed");
        assert_eq!(progress.phase, Q1DemoPhase::Ended(Q1DemoEnd::Closed));

        let reader = VecNQReader {
            records: Vec::new(),
            forced: -1,
        };
        let remote = MockNQRemote {
            ready: false,
            recorded: 0.0,
            receives: Vec::new(),
            angles: Vec::new(),
            samples: Vec::new(),
            tracks: Vec::new(),
        };
        let mut input = NetQuakeDemoInput::new(reader, remote);
        input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 5,
                timedemo: false,
            })
            .expect("eof");
        // Duplicate frames are cheap no-ops; backwards is an error while live.
        // (After termination the donor short-circuits before the order check.)
        let signon = |stage: u8| NetQuakeDemoRecord {
            view_angles: test_angles(),
            message: nq_bytes(&[NetQuakeMessage::Signon { stage }]),
        };
        let reader = VecNQReader {
            records: vec![
                signon(1),
                signon(2),
                signon(3),
                signon(4),
                NetQuakeDemoRecord {
                    view_angles: test_angles(),
                    message: Vec::new(),
                },
            ],
            forced: -1,
        };
        let remote = MockNQRemote {
            ready: true,
            recorded: 100.0,
            receives: Vec::new(),
            angles: Vec::new(),
            samples: Vec::new(),
            tracks: Vec::new(),
        };
        let mut input = NetQuakeDemoInput::new(reader, remote);
        let first = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 5,
                timedemo: false,
            })
            .expect("run");
        assert_eq!(first.records_read, 4);
        assert_eq!(first.phase, Q1DemoPhase::Active);
        let duplicate = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 5,
                timedemo: false,
            })
            .expect("dup");
        assert_eq!(duplicate.records_read, 0);
        assert_eq!(duplicate.phase, Q1DemoPhase::Active);
        let error = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 4,
                timedemo: false,
            })
            .expect_err("backwards");
        assert_eq!(error.to_string(), "Demo frame number moved backwards");
    }

    #[test]
    fn netquake_signon_reaches_active() {
        let signon = |stage: u8| NetQuakeDemoRecord {
            view_angles: test_angles(),
            message: nq_bytes(&[NetQuakeMessage::Signon { stage }]),
        };
        let reader = VecNQReader {
            records: vec![signon(1), signon(2), signon(3), signon(4)],
            forced: -1,
        };
        let remote = MockNQRemote {
            ready: true,
            recorded: 12.0,
            receives: Vec::new(),
            angles: Vec::new(),
            samples: Vec::new(),
            tracks: Vec::new(),
        };
        let mut input = NetQuakeDemoInput::new(reader, remote);
        let progress = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.05,
                frame: 0,
                timedemo: false,
            })
            .expect("advance");
        assert_eq!(progress.records_read, 4);
        assert_eq!(progress.phase, Q1DemoPhase::Active);
        assert_eq!(progress.recorded_seconds, 12.0);
        assert_eq!(input.remote().receives.len(), 4);
        assert_eq!(input.remote().angles.len(), 4);
        assert!(input.remote().angles.iter().all(|(_, absolute)| *absolute));
        assert_eq!(input.remote().samples, vec![12.0]);
    }

    #[test]
    fn netquake_entity_completes_signon_three() {
        // The NQ writer refuses entity updates by design, so hand-encode the
        // minimal update: U_SIGNAL with zero bits plus entity number 1.
        let entity = NetQuakeDemoRecord {
            view_angles: test_angles(),
            message: vec![128u8, 1u8],
        };
        let signon = |stage: u8| NetQuakeDemoRecord {
            view_angles: test_angles(),
            message: nq_bytes(&[NetQuakeMessage::Signon { stage }]),
        };
        let reader = VecNQReader {
            records: vec![signon(1), signon(2), signon(3), entity],
            forced: -1,
        };
        let remote = MockNQRemote {
            ready: true,
            recorded: 3.0,
            receives: Vec::new(),
            angles: Vec::new(),
            samples: Vec::new(),
            tracks: Vec::new(),
        };
        let mut input = NetQuakeDemoInput::new(reader, remote);
        let progress = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.05,
                frame: 0,
                timedemo: false,
            })
            .expect("advance");
        assert_eq!(progress.phase, Q1DemoPhase::Active);
    }

    #[test]
    fn netquake_signon_validation() {
        for stages in [vec![1u8, 1u8], vec![1, 3, 2], vec![5]] {
            let reader = VecNQReader {
                records: stages
                    .into_iter()
                    .map(|stage| NetQuakeDemoRecord {
                        view_angles: test_angles(),
                        message: nq_bytes(&[NetQuakeMessage::Signon { stage }]),
                    })
                    .collect(),
                forced: -1,
            };
            let remote = MockNQRemote {
                ready: false,
                recorded: 0.0,
                receives: Vec::new(),
                angles: Vec::new(),
                samples: Vec::new(),
                tracks: Vec::new(),
            };
            let mut input = NetQuakeDemoInput::new(reader, remote);
            let error = input
                .advance(&Q1DemoFrame {
                    elapsed_seconds: 0.0,
                    frame: 0,
                    timedemo: false,
                })
                .expect_err("stage");
            assert!(error.to_string().starts_with("Invalid demo signon stage"));
        }
    }

    #[test]
    fn netquake_server_info_resets_signon() {
        let signon = |stage: u8| NetQuakeDemoRecord {
            view_angles: test_angles(),
            message: nq_bytes(&[NetQuakeMessage::Signon { stage }]),
        };
        let reader = VecNQReader {
            records: vec![
                signon(2),
                NetQuakeDemoRecord {
                    view_angles: test_angles(),
                    message: nq_bytes(&[nq_server_info()]),
                },
                signon(1),
            ],
            forced: -1,
        };
        let remote = MockNQRemote {
            ready: false,
            recorded: 0.0,
            receives: Vec::new(),
            angles: Vec::new(),
            samples: Vec::new(),
            tracks: Vec::new(),
        };
        let mut input = NetQuakeDemoInput::new(reader, remote);
        // Stage 1 after stage 2 only passes when the reset ran; the donor
        // then drains the exhausted reader to EOF in the same advance.
        let progress = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 0,
                timedemo: false,
            })
            .expect("advance");
        assert_eq!(progress.records_read, 3);
        assert_eq!(progress.phase, Q1DemoPhase::Ended(Q1DemoEnd::Eof));
    }

    #[test]
    fn netquake_disconnect_and_track_override() {
        let reader = VecNQReader {
            records: vec![NetQuakeDemoRecord {
                view_angles: test_angles(),
                message: nq_bytes(&[
                    NetQuakeMessage::CdTrack {
                        track: 5,
                        loop_track: 6,
                    },
                    NetQuakeMessage::Unit(NqUnit::Disconnect),
                ]),
            }],
            forced: 7,
        };
        let remote = MockNQRemote {
            ready: false,
            recorded: 0.0,
            receives: Vec::new(),
            angles: Vec::new(),
            samples: Vec::new(),
            tracks: Vec::new(),
        };
        let mut input = NetQuakeDemoInput::new(reader, remote);
        let progress = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 0,
                timedemo: false,
            })
            .expect("advance");
        assert_eq!(progress.phase, Q1DemoPhase::Ended(Q1DemoEnd::RecordedDisconnect));
        assert_eq!(input.remote().tracks, vec![7]);

        let reader = VecNQReader {
            records: vec![NetQuakeDemoRecord {
                view_angles: test_angles(),
                message: nq_bytes(&[NetQuakeMessage::CdTrack {
                    track: 5,
                    loop_track: 6,
                }]),
            }],
            forced: -1,
        };
        let remote = MockNQRemote {
            ready: false,
            recorded: 0.0,
            receives: Vec::new(),
            angles: Vec::new(),
            samples: Vec::new(),
            tracks: Vec::new(),
        };
        let mut input = NetQuakeDemoInput::new(reader, remote);
        input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 0,
                timedemo: false,
            })
            .expect("advance");
        assert_eq!(input.remote().tracks, vec![5]);
    }

    #[test]
    fn netquake_timedemo_ignores_clock_gate() {
        let signon = |stage: u8| NetQuakeDemoRecord {
            view_angles: test_angles(),
            message: nq_bytes(&[NetQuakeMessage::Signon { stage }]),
        };
        let extra = NetQuakeDemoRecord {
            view_angles: test_angles(),
            message: nq_bytes(&[NetQuakeMessage::Unit(NqUnit::Nop)]),
        };
        let reader = VecNQReader {
            records: vec![signon(1), signon(2), signon(3), signon(4), extra],
            forced: -1,
        };
        let remote = MockNQRemote {
            ready: true,
            recorded: 100.0,
            receives: Vec::new(),
            angles: Vec::new(),
            samples: Vec::new(),
            tracks: Vec::new(),
        };
        let mut input = NetQuakeDemoInput::new(reader, remote);
        // First advance primes and stops; the timedemo follow-up drains all.
        let first = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.05,
                frame: 0,
                timedemo: true,
            })
            .expect("prime");
        assert_eq!(first.records_read, 4);
        assert_eq!(first.phase, Q1DemoPhase::Active);
        let second = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.05,
                frame: 1,
                timedemo: true,
            })
            .expect("drain");
        assert_eq!(second.records_read, 1);
        assert_eq!(second.phase, Q1DemoPhase::Active);
        let third = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.05,
                frame: 2,
                timedemo: true,
            })
            .expect("eof");
        assert_eq!(third.records_read, 0);
        assert_eq!(third.phase, Q1DemoPhase::Ended(Q1DemoEnd::Eof));
    }

    #[test]
    fn quakeworld_precache_reaches_active() {
        let packet = |seconds: f64, sequence: u32, messages: &[QuakeWorldMessage]| QuakeWorldDemoRecord::Packet {
            seconds,
            message: qw_packet(sequence, 0, &qw_bytes(messages)),
        };
        let reader = VecQWReader {
            records: vec![
                packet(0.0, 1, &[qw_server_data(0)]),
                packet(
                    0.1,
                    2,
                    &[QuakeWorldMessage::ModelList {
                        first: 0,
                        names: vec!["progs/player.mdl".to_string()],
                        next: 0,
                    }],
                ),
                packet(
                    0.2,
                    3,
                    &[QuakeWorldMessage::SoundList {
                        first: 0,
                        names: vec!["weapons/r_exp3.wav".to_string()],
                        next: 0,
                    }],
                ),
                // Trailing future record: stays pending behind the clock gate so
                // the donor reports Active instead of draining to EOF.
                packet(999.0, 4, &[QuakeWorldMessage::Unit(QwUnit::Nop)]),
            ],
        };
        let remote = MockQWRemote {
            ready: true,
            shared: MockShared {
                angles: Vec::new(),
                samples: Vec::new(),
            },
            prediction: MockPred {
                sent: Vec::new(),
                acked: Vec::new(),
            },
            server_datas: 0,
            game_states: Vec::new(),
            receives: Vec::new(),
            presentations: Vec::new(),
        };
        let mut input = QuakeWorldDemoInput::new(reader, remote);
        let progress = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.05,
                frame: 0,
                timedemo: false,
            })
            .expect("advance");
        assert_eq!(progress.records_read, 3);
        assert_eq!(progress.phase, Q1DemoPhase::Active);
        assert_eq!(input.remote().server_datas, 1);
        assert_eq!(input.remote().game_states, vec![(1, 1)]);
        assert_eq!(input.remote().receives.len(), 3);
        assert_eq!(input.remote().presentations.len(), 1);
    }

    #[test]
    fn quakeworld_precache_guards() {
        // List before server data.
        let reader = VecQWReader {
            records: vec![QuakeWorldDemoRecord::Packet {
                seconds: 0.0,
                message: qw_packet(
                    1,
                    0,
                    &qw_bytes(&[QuakeWorldMessage::ModelList {
                        first: 0,
                        names: vec!["a".to_string()],
                        next: 0,
                    }]),
                ),
            }],
        };
        let mut input = QuakeWorldDemoInput::new(
            reader,
            MockQWRemote {
                ready: true,
                shared: MockShared {
                    angles: Vec::new(),
                    samples: Vec::new(),
                },
                prediction: MockPred {
                    sent: Vec::new(),
                    acked: Vec::new(),
                },
                server_datas: 0,
                game_states: Vec::new(),
                receives: Vec::new(),
                presentations: Vec::new(),
            },
        );
        let error = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 0,
                timedemo: false,
            })
            .expect_err("precache");
        assert_eq!(error.to_string(), "QWD precache before serverdata");

        // Non-contiguous list.
        let reader = VecQWReader {
            records: vec![QuakeWorldDemoRecord::Packet {
                seconds: 0.0,
                message: qw_packet(1, 0, &qw_bytes(&[qw_server_data(0)])),
            }],
        };
        let mut input = QuakeWorldDemoInput::new(
            reader,
            MockQWRemote {
                ready: true,
                shared: MockShared {
                    angles: Vec::new(),
                    samples: Vec::new(),
                },
                prediction: MockPred {
                    sent: Vec::new(),
                    acked: Vec::new(),
                },
                server_datas: 0,
                game_states: Vec::new(),
                receives: Vec::new(),
                presentations: Vec::new(),
            },
        );
        input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 0,
                timedemo: false,
            })
            .expect("data");
        // bad player slot tested through a second input below.
        let reader = VecQWReader {
            records: vec![QuakeWorldDemoRecord::Packet {
                seconds: 0.0,
                message: qw_packet(1, 0, &qw_bytes(&[qw_server_data(32)])),
            }],
        };
        let mut input = QuakeWorldDemoInput::new(
            reader,
            MockQWRemote {
                ready: true,
                shared: MockShared {
                    angles: Vec::new(),
                    samples: Vec::new(),
                },
                prediction: MockPred {
                    sent: Vec::new(),
                    acked: Vec::new(),
                },
                server_datas: 0,
                game_states: Vec::new(),
                receives: Vec::new(),
                presentations: Vec::new(),
            },
        );
        let error = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 0,
                timedemo: false,
            })
            .expect_err("slot");
        assert_eq!(error.to_string(), "QWD requires a valid native player slot");
    }

    #[test]
    fn quakeworld_packet_guards_and_stale_sequences() {
        // Truncated header.
        let reader = VecQWReader {
            records: vec![QuakeWorldDemoRecord::Packet {
                seconds: 0.0,
                message: vec![1, 2, 3],
            }],
        };
        let mut input = QuakeWorldDemoInput::new(
            reader,
            MockQWRemote {
                ready: true,
                shared: MockShared {
                    angles: Vec::new(),
                    samples: Vec::new(),
                },
                prediction: MockPred {
                    sent: Vec::new(),
                    acked: Vec::new(),
                },
                server_datas: 0,
                game_states: Vec::new(),
                receives: Vec::new(),
                presentations: Vec::new(),
            },
        );
        let error = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 0,
                timedemo: false,
            })
            .expect_err("truncated");
        assert_eq!(error.to_string(), "Truncated QWD netchannel header");

        // Stale and connectionless packets are ignored.
        let stale = qw_packet(1, 0, &qw_bytes(&[qw_server_data(0)]));
        let mut header = stale.clone();
        header[0..4].copy_from_slice(&(-1i32).to_le_bytes());
        let reader = VecQWReader {
            records: vec![
                QuakeWorldDemoRecord::Packet {
                    seconds: 0.0,
                    message: stale.clone(),
                },
                QuakeWorldDemoRecord::Packet {
                    seconds: 0.1,
                    message: stale,
                },
                QuakeWorldDemoRecord::Packet {
                    seconds: 0.2,
                    message: header,
                },
            ],
        };
        let mut input = QuakeWorldDemoInput::new(
            reader,
            MockQWRemote {
                ready: true,
                shared: MockShared {
                    angles: Vec::new(),
                    samples: Vec::new(),
                },
                prediction: MockPred {
                    sent: Vec::new(),
                    acked: Vec::new(),
                },
                server_datas: 0,
                game_states: Vec::new(),
                receives: Vec::new(),
                presentations: Vec::new(),
            },
        );
        let progress = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 0,
                timedemo: false,
            })
            .expect("advance");
        assert_eq!(progress.records_read, 3);
        assert_eq!(input.remote().server_datas, 1);
    }

    #[test]
    fn quakeworld_commands_replay_and_prune() {
        let command = |seconds: f64| QuakeWorldDemoRecord::Command {
            seconds,
            command: qw_command(),
            view_angles: test_angles(),
        };
        let packet = |seconds: f64, sequence: u32, acknowledged: u32| QuakeWorldDemoRecord::Packet {
            seconds,
            message: qw_packet(
                sequence,
                acknowledged,
                &qw_bytes(&[QuakeWorldMessage::Unit(QwUnit::Nop)]),
            ),
        };
        let reader = VecQWReader {
            records: vec![
                QuakeWorldDemoRecord::Sequences {
                    seconds: 0.0,
                    outgoing: 10,
                    incoming: 0,
                },
                command(0.0),
                command(0.0),
                packet(0.2, 1, 10),
            ],
        };
        let mut input = QuakeWorldDemoInput::new(
            reader,
            MockQWRemote {
                ready: true,
                shared: MockShared {
                    angles: Vec::new(),
                    samples: Vec::new(),
                },
                prediction: MockPred {
                    sent: Vec::new(),
                    acked: Vec::new(),
                },
                server_datas: 0,
                game_states: Vec::new(),
                receives: Vec::new(),
                presentations: Vec::new(),
            },
        );
        // Not world-ready yet: commands buffer without replay.
        let progress = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 0,
                timedemo: true,
            })
            .expect("advance");
        assert_eq!(progress.phase, Q1DemoPhase::Loading);
        assert!(input.remote().prediction.sent.is_empty());
        assert_eq!(input.remote().shared.angles.len(), 2);
    }

    #[test]
    fn quakeworld_disconnect_ends_and_eof_follows() {
        let reader = VecQWReader {
            records: vec![QuakeWorldDemoRecord::Packet {
                seconds: 0.0,
                message: qw_packet(1, 0, &qw_bytes(&[QuakeWorldMessage::Unit(QwUnit::Disconnect)])),
            }],
        };
        let mut input = QuakeWorldDemoInput::new(
            reader,
            MockQWRemote {
                ready: true,
                shared: MockShared {
                    angles: Vec::new(),
                    samples: Vec::new(),
                },
                prediction: MockPred {
                    sent: Vec::new(),
                    acked: Vec::new(),
                },
                server_datas: 0,
                game_states: Vec::new(),
                receives: Vec::new(),
                presentations: Vec::new(),
            },
        );
        let progress = input
            .advance(&Q1DemoFrame {
                elapsed_seconds: 0.0,
                frame: 0,
                timedemo: false,
            })
            .expect("advance");
        assert_eq!(progress.phase, Q1DemoPhase::Ended(Q1DemoEnd::RecordedDisconnect));
    }
}
