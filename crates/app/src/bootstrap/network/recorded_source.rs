//! Recorded remote source.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/network/recorded-source.ts`
//! (`RecordedRemoteSource`). The donor `async` advances resolve inline. The
//! concrete remote presentations
//! ([`remote_q1`](super::remote_q1), [`remote_qw`](super::remote_qw),
//! [`remote`](super::remote), [`remote_q3`](super::remote_q3)) back the
//! remote union, which carries the exact surfaces this module uses: the
//! sibling demo-remote traits for Quake/QuakeWorld and capability bundles
//! for Quake II/III. Quake/QuakeWorld byte readers parse eagerly at
//! construction (the donor throws corrupt headers from its constructor
//! too); mid-stream corruption therefore fails construction instead of a
//! later advance. [`RecordedCompletion`] is the `demo-commands`
//! [`DemoCompletion`](crate::bootstrap::demo_commands::DemoCompletion) union.

use std::rc::Rc;

use qa_core::cvar::CvarRegistry;
use qa_core::math::Vec3;
use qa_net::common::commands::UserCommand;
use qa_net::demo::{DemoError, NqDemoReader, QwDemoReader, QwDemoRecord};
use qa_net::q3_net::DemoEndReason;
use thiserror::Error;

use crate::bootstrap::demo_playback::{DemoFamily, DemoResource};

use super::q1_demo::{
    NetQuakeDemoInput, NetQuakeDemoReader, NetQuakeDemoRecord, NetQuakeDemoRemote, Q1DemoEnd, Q1DemoError, Q1DemoFrame,
    Q1DemoPhase, QuakeWorldDemoInput, QuakeWorldDemoReader, QuakeWorldDemoRecord, QuakeWorldDemoRemote,
};
use super::q2_client_receiver::Q2ClientReceiverHost;
use super::q2_demo::{
    read_q2_playback_header, Q2DemoAdvance, Q2DemoError, Q2DemoPlayback, Q2MvdPresentation, Q2PlaybackHeader,
    Q2RecordedView,
};
use super::q3_client::Q3ApplicationClientHost;
use super::q3_demo::{
    Q3DemoAdvanceOptions, Q3DemoClock, Q3DemoError, Q3DemoFrame, Q3DemoPhase, Q3DemoPlayback, Q3DemoPlaybackOptions,
};

/// NetQuake demo message cap (donor default `64000`).
const NQ_MAX_MESSAGE_BYTES: usize = 64000;
/// QuakeWorld demo message cap (donor default `1450`).
const QW_MAX_MESSAGE_BYTES: usize = 1450;

/// Demo timing report (donor `DemoTiming`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DemoTiming {
    /// Rendered frames.
    pub frames: u64,
    /// Elapsed milliseconds.
    pub elapsed_milliseconds: f64,
}

/// Demo timing text (donor `demoTimingText`).
#[must_use]
pub fn demo_timing_text(timing: &DemoTiming) -> String {
    let seconds = timing.elapsed_milliseconds / 1000.0;
    let fps = if seconds > 0.0 {
        timing.frames as f64 / seconds
    } else {
        0.0
    };
    format!("{} frames, {seconds:.3} seconds: {fps:.1} fps\n", timing.frames)
}

/// Demo completion reason (donor `demo-commands` `DemoCompletion`).
pub use crate::bootstrap::demo_commands::DemoCompletion as RecordedCompletion;

/// Recorded source failure.
#[derive(Debug, Error)]
pub enum RecordedError {
    /// Donor failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Demo byte failure.
    #[error(transparent)]
    Demo(#[from] DemoError),
    /// Quake/QuakeWorld input failure.
    #[error(transparent)]
    Q1(#[from] Q1DemoError),
    /// Quake II playback failure.
    #[error(transparent)]
    Q2(#[from] Q2DemoError),
    /// Quake III playback failure.
    #[error(transparent)]
    Q3(#[from] Q3DemoError),
}

/// Source phase (donor `RecordedRemoteSource['state']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordedPhase {
    /// Signon/pre-cache in progress.
    Loading,
    /// Playback active.
    Active,
    /// Terminal.
    Closed,
}

/// Eager NetQuake byte reader (donor `NetQuakeDemoReader`).
struct BytesNqReader {
    /// Parsed records.
    records: Vec<NetQuakeDemoRecord>,
    /// Read cursor.
    cursor: usize,
    /// Forced CD track.
    track: i32,
}

impl BytesNqReader {
    /// Parse demo bytes.
    fn parse(bytes: &[u8]) -> Result<Self, DemoError> {
        let mut reader = NqDemoReader::new(bytes, NQ_MAX_MESSAGE_BYTES)?;
        let mut records = Vec::new();
        while let Some(record) = reader.next_record()? {
            let [x, y, z] = record.view_angles;
            records.push(NetQuakeDemoRecord {
                view_angles: Vec3 { x, y, z },
                message: record.message,
            });
        }
        Ok(Self {
            records,
            cursor: 0,
            track: reader.forced_track,
        })
    }
}

impl NetQuakeDemoReader for BytesNqReader {
    fn next_record(&mut self) -> Option<NetQuakeDemoRecord> {
        let record = self.records.get(self.cursor).cloned();
        if record.is_some() {
            self.cursor += 1;
        }
        record
    }

    fn forced_track(&self) -> i32 {
        self.track
    }
}

/// Eager QuakeWorld byte reader (donor `QuakeWorldDemoReader`).
struct BytesQwReader {
    /// Parsed records.
    records: Vec<QuakeWorldDemoRecord>,
    /// Read cursor.
    cursor: usize,
}

impl BytesQwReader {
    /// Parse demo bytes.
    fn parse(bytes: &[u8]) -> Result<Self, DemoError> {
        let mut reader = QwDemoReader::new(bytes, QW_MAX_MESSAGE_BYTES);
        let mut records = Vec::new();
        while let Some(record) = reader.next_record()? {
            records.push(match record {
                QwDemoRecord::Command {
                    seconds,
                    command,
                    view_angles,
                } => {
                    let [ax, ay, az] = view_angles;
                    QuakeWorldDemoRecord::Command {
                        seconds: f64::from(seconds),
                        command: UserCommand::Q1Quakeworld {
                            milliseconds: f64::from(command.milliseconds),
                            angles: [
                                f64::from(command.angles[0]),
                                f64::from(command.angles[1]),
                                f64::from(command.angles[2]),
                            ],
                            forward_move: f64::from(command.forward_move),
                            side_move: f64::from(command.side_move),
                            up_move: f64::from(command.up_move),
                            buttons: f64::from(command.buttons),
                            impulse: f64::from(command.impulse),
                        },
                        view_angles: Vec3 { x: ax, y: ay, z: az },
                    }
                }
                QwDemoRecord::Packet { seconds, message } => QuakeWorldDemoRecord::Packet {
                    seconds: f64::from(seconds),
                    message,
                },
                QwDemoRecord::Sequences {
                    seconds,
                    outgoing,
                    incoming,
                } => QuakeWorldDemoRecord::Sequences {
                    seconds: f64::from(seconds),
                    outgoing,
                    incoming,
                },
            });
        }
        Ok(Self { records, cursor: 0 })
    }
}

impl QuakeWorldDemoReader for BytesQwReader {
    fn next_record(&mut self) -> Option<QuakeWorldDemoRecord> {
        let record = self.records.get(self.cursor).cloned();
        if record.is_some() {
            self.cursor += 1;
        }
        record
    }
}

/// Leased demo bytes for the Quake II playback borrow, reclaimed on drop.
struct LeasedBytes {
    /// Boxed bytes pointer from `Box::into_raw`.
    raw: *mut [u8],
}

impl Drop for LeasedBytes {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `Box::into_raw` exactly once, and
        // the playback (the only borrower) is declared before this guard
        // so it always drops first.
        unsafe {
            drop(Box::from_raw(self.raw));
        }
    }
}

/// Quake II remote bundle (donor `Q2RemotePresentation` surface used here).
pub struct Q2RecordedRemote<H> {
    /// Receiver host.
    pub host: H,
    /// Multiview presentation, when the remote shows multiview.
    pub mvd_presentation: Option<Q2MvdPresentation>,
    /// Recorded-view selector.
    pub select_recorded_view: Option<Box<dyn Q2RecordedView>>,
    /// Demo clock sampler (donor `sampleDemo`).
    pub sample_demo: Box<dyn FnMut(f64)>,
}

/// Quake III remote bundle (donor `Q3RemotePresentation` surface used here).
pub struct Q3RecordedRemote<H, C> {
    /// Playback host.
    pub host: H,
    /// Playback clock.
    pub clock: C,
    /// Presentation sampler (donor `samplePresentation`).
    pub sample_presentation: Box<dyn FnMut(f64)>,
}

/// Remote union (donor `remote` parameter).
pub enum RecordedRemote<R1, R2, H2, H3, C3> {
    /// NetQuake presentation.
    Q1(R1),
    /// QuakeWorld presentation.
    Qw(R2),
    /// Quake II presentation bundle.
    Q2(Q2RecordedRemote<H2>),
    /// Quake III presentation bundle.
    Q3(Q3RecordedRemote<H3, C3>),
}

/// Family playback (donor `Playback`).
enum Playback<R1, R2, H2, H3, C3> {
    /// NetQuake input.
    Q1 {
        input: Box<NetQuakeDemoInput<BytesNqReader, R1>>,
    },
    /// QuakeWorld input.
    Qw {
        input: Box<QuakeWorldDemoInput<BytesQwReader, R2>>,
    },
    /// Quake II playback with its sampler and byte lease.
    Q2 {
        input: Box<Q2DemoPlayback<'static, H2>>,
        sample: Box<dyn FnMut(f64)>,
        /// Byte lease guard (drops after `input`).
        _bytes: LeasedBytes,
    },
    /// Quake III playback with its sampler.
    Q3 {
        input: Box<Q3DemoPlayback<H3, C3>>,
        sample: Box<dyn FnMut(f64)>,
    },
}

/// One rendered-frame advance over the protocol readers and presentation
/// clocks (donor `RecordedRemoteSource`).
pub struct RecordedRemoteSource<R1, R2, H2, H3, C3> {
    /// Opened demo resource (donor `resource`).
    pub resource: DemoResource,
    /// Family playback.
    playback: Playback<R1, R2, H2, H3, C3>,
    /// Quake II recorded clock.
    milliseconds: Option<f64>,
    /// Source phase.
    state: RecordedPhase,
    /// Terminal reason.
    terminal: Option<RecordedCompletion>,
    /// Completion already reported.
    reported: bool,
    /// Paused flag.
    paused: bool,
    /// Previous real time for pause accounting.
    previous_real_time: Option<f64>,
    /// Accumulated paused milliseconds.
    paused_milliseconds: f64,
    /// Benchmark start, once active.
    benchmark_start: Option<f64>,
    /// Benchmark frames after the start.
    benchmark_frames: u64,
    /// Reported timing.
    result_timing: Option<DemoTiming>,
    /// Forced timedemo.
    timedemo: bool,
    /// Live cvar registry.
    cvars: Rc<CvarRegistry>,
    /// Completion callback.
    complete: Box<dyn FnMut(RecordedCompletion, Option<DemoTiming>)>,
}

impl<R1, R2, H2, H3, C3> RecordedRemoteSource<R1, R2, H2, H3, C3>
where
    R1: NetQuakeDemoRemote,
    R2: QuakeWorldDemoRemote,
    H2: Q2ClientReceiverHost,
    H3: Q3ApplicationClientHost + 'static,
    C3: Q3DemoClock,
{
    /// Open a recorded source (donor constructor).
    pub fn new(
        resource: DemoResource,
        remote: RecordedRemote<R1, R2, H2, H3, C3>,
        timedemo: bool,
        cvars: Rc<CvarRegistry>,
        complete: impl FnMut(RecordedCompletion, Option<DemoTiming>) + 'static,
    ) -> Result<Self, RecordedError> {
        let playback = match (&resource, remote) {
            (
                DemoResource::Standard {
                    family: DemoFamily::Q1,
                    bytes,
                    ..
                },
                RecordedRemote::Q1(remote),
            ) => Playback::Q1 {
                input: Box::new(NetQuakeDemoInput::new(BytesNqReader::parse(bytes)?, remote)),
            },
            (
                DemoResource::Standard {
                    family: DemoFamily::Qw,
                    bytes,
                    ..
                },
                RecordedRemote::Qw(remote),
            ) => Playback::Qw {
                input: Box::new(QuakeWorldDemoInput::new(BytesQwReader::parse(bytes)?, remote)),
            },
            (
                DemoResource::Standard {
                    family: DemoFamily::Q2,
                    bytes,
                    ..
                },
                RecordedRemote::Q2(bundle),
            ) => Self::q2_playback(bytes, bundle)?,
            (
                DemoResource::Standard {
                    family: DemoFamily::Q3,
                    bytes,
                    ..
                },
                RecordedRemote::Q3(bundle),
            )
            | (DemoResource::Q3 { bytes, .. }, RecordedRemote::Q3(bundle)) => Self::q3_playback(bytes, bundle)?,
            _ => {
                return Err(RecordedError::Message(
                    "Recording and remote presentation families differ".to_string(),
                ));
            }
        };
        Ok(Self {
            resource,
            playback,
            milliseconds: None,
            state: RecordedPhase::Loading,
            terminal: None,
            reported: false,
            paused: false,
            previous_real_time: None,
            paused_milliseconds: 0.0,
            benchmark_start: None,
            benchmark_frames: 0,
            result_timing: None,
            timedemo,
            cvars,
            complete: Box::new(complete),
        })
    }

    /// Build the Quake II playback over leased bytes.
    fn q2_playback(bytes: &[u8], bundle: Q2RecordedRemote<H2>) -> Result<Playback<R1, R2, H2, H3, C3>, RecordedError> {
        let mvd = matches!(read_q2_playback_header(bytes)?, Q2PlaybackHeader::Mvd { .. });
        let raw = Box::into_raw(bytes.to_vec().into_boxed_slice());
        // SAFETY: reclaimed by the lease guard after the playback drops;
        // reclaimed inline when construction fails below.
        let leased: &'static [u8] = unsafe { &*raw };
        let presentation = if mvd { bundle.mvd_presentation } else { None };
        match Q2DemoPlayback::new(leased, bundle.host, presentation, bundle.select_recorded_view) {
            Ok(input) => Ok(Playback::Q2 {
                input: Box::new(input),
                sample: bundle.sample_demo,
                _bytes: LeasedBytes { raw },
            }),
            Err(error) => {
                unsafe {
                    drop(Box::from_raw(raw));
                }
                Err(error.into())
            }
        }
    }

    /// Build the Quake III playback.
    fn q3_playback(
        bytes: &[u8],
        bundle: Q3RecordedRemote<H3, C3>,
    ) -> Result<Playback<R1, R2, H2, H3, C3>, RecordedError> {
        let input = Q3DemoPlayback::new(Q3DemoPlaybackOptions {
            host: bundle.host,
            clock: bundle.clock,
            bytes: bytes.to_vec(),
        })?;
        Ok(Playback::Q3 {
            input: Box::new(input),
            sample: bundle.sample_presentation,
        })
    }

    /// Select the viewed player (donor `selectPlayer`).
    pub fn select_player(&mut self, clientnum: i32) -> Result<(), RecordedError> {
        match &mut self.playback {
            Playback::Q2 { input, .. } => Ok(input.select_player(clientnum)?),
            _ => Err(RecordedError::Message(
                "View selection requires a multiview Q2 recording".to_string(),
            )),
        }
    }

    /// Source phase (donor `phase`).
    #[must_use]
    pub fn phase(&self) -> RecordedPhase {
        self.state
    }

    /// Reported timing (donor `timing`).
    #[must_use]
    pub fn timing(&self) -> Option<DemoTiming> {
        self.result_timing
    }

    /// Paused flag (donor `isPaused`).
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Set the paused flag (donor `setPaused`).
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Advance one rendered frame (donor `advance`).
    pub fn advance(&mut self, milliseconds: f64, frame: u64, real_time: f64) -> Result<(), RecordedError> {
        if self.state == RecordedPhase::Closed {
            return Ok(());
        }
        if self.paused {
            if let Some(previous) = self.previous_real_time {
                self.paused_milliseconds += (real_time - previous).max(0.0);
            }
        }
        self.previous_real_time = Some(real_time);
        if self.paused {
            return Ok(());
        }
        let real_time = real_time - self.paused_milliseconds;
        let timedemo = self.timedemo || self.cvars.variable_value("timedemo") != 0.0;
        let q3 = matches!(self.playback, Playback::Q3 { .. });
        match &mut self.playback {
            Playback::Q1 { input } => {
                let result = input.advance(&Q1DemoFrame {
                    elapsed_seconds: milliseconds / 1000.0,
                    frame,
                    timedemo,
                })?;
                match result.phase {
                    Q1DemoPhase::Ended(reason) => self.terminal = Some(map_q1_end(reason)),
                    Q1DemoPhase::Loading => self.state = RecordedPhase::Loading,
                    Q1DemoPhase::Active => self.state = RecordedPhase::Active,
                }
            }
            Playback::Qw { input } => {
                let result = input.advance(&Q1DemoFrame {
                    elapsed_seconds: milliseconds / 1000.0,
                    frame,
                    timedemo,
                })?;
                match result.phase {
                    Q1DemoPhase::Ended(reason) => self.terminal = Some(map_q1_end(reason)),
                    Q1DemoPhase::Loading => self.state = RecordedPhase::Loading,
                    Q1DemoPhase::Active => self.state = RecordedPhase::Active,
                }
            }
            Playback::Q2 { input, sample, .. } => {
                let target = self.milliseconds.map_or(0.0, |at| at + milliseconds);
                let result = if timedemo {
                    input.next_frame()?
                } else {
                    input.advance(target)?
                };
                match result {
                    Q2DemoAdvance::Frame { time_milliseconds } => {
                        if self.milliseconds.is_none() || timedemo || time_milliseconds < target {
                            self.milliseconds = Some(time_milliseconds);
                        } else {
                            self.milliseconds = Some(target);
                        }
                        sample(self.milliseconds.expect("just set"));
                        self.state = RecordedPhase::Active;
                    }
                    Q2DemoAdvance::Eof => self.terminal = Some(RecordedCompletion::Eof),
                    Q2DemoAdvance::Disconnected => {
                        self.terminal = Some(RecordedCompletion::Disconnected);
                    }
                    Q2DemoAdvance::Closed => self.terminal = Some(RecordedCompletion::Closed),
                }
            }
            Playback::Q3 { input, sample } => {
                let result = input.advance_frame(
                    real_time,
                    &Q3DemoAdvanceOptions {
                        paused: false,
                        time_nudge: f64::from(self.cvars.variable_value("cl_timeNudge")),
                        timescale: f64::from(self.cvars.variable_value("timescale")),
                        freeze_demo: self.cvars.variable_value("cl_freezeDemo") != 0.0,
                        timedemo,
                    },
                )?;
                match result {
                    Q3DemoFrame::End { end, timing } => {
                        self.terminal = Some(match end.reason {
                            DemoEndReason::Terminator => RecordedCompletion::Terminator,
                            DemoEndReason::Eof => RecordedCompletion::Eof,
                            DemoEndReason::TruncatedHeader | DemoEndReason::TruncatedPayload => {
                                RecordedCompletion::Truncated
                            }
                        });
                        self.result_timing = timing.map(|report| DemoTiming {
                            frames: u64::try_from(report.frames).unwrap_or(0),
                            elapsed_milliseconds: f64::from(report.elapsed_milliseconds),
                        });
                    }
                    Q3DemoFrame::Frame { .. } => {
                        sample(real_time);
                        self.state = RecordedPhase::Active;
                    }
                    Q3DemoFrame::Pending => {
                        self.state = if input.phase() == Q3DemoPhase::Active {
                            RecordedPhase::Active
                        } else {
                            RecordedPhase::Loading
                        };
                    }
                }
            }
        }
        if timedemo && !q3 {
            if self.terminal.is_none() && self.state == RecordedPhase::Active {
                if self.benchmark_start.is_none() {
                    self.benchmark_start = Some(real_time);
                } else {
                    self.benchmark_frames += 1;
                }
            } else if let (Some(_), Some(start)) = (self.terminal, self.benchmark_start) {
                self.result_timing = Some(DemoTiming {
                    frames: self.benchmark_frames,
                    elapsed_milliseconds: (real_time - start).max(0.0),
                });
            }
        }
        if self.terminal.is_some() && !self.reported {
            if let Some(terminal) = self.terminal {
                self.reported = true;
                self.state = RecordedPhase::Closed;
                (self.complete)(terminal, self.result_timing);
            }
        }
        Ok(())
    }

    /// Retire the source (donor `close`).
    pub fn close(&mut self) {
        self.state = RecordedPhase::Closed;
        match &mut self.playback {
            Playback::Q1 { input } => input.close(),
            Playback::Qw { input } => input.close(),
            Playback::Q2 { input, .. } => input.close(),
            Playback::Q3 { input, .. } => input.close(),
        }
    }
}

/// Map a Quake terminal reason (donor `recorded-disconnect` mapping).
fn map_q1_end(reason: Q1DemoEnd) -> RecordedCompletion {
    match reason {
        Q1DemoEnd::Eof => RecordedCompletion::Eof,
        Q1DemoEnd::RecordedDisconnect => RecordedCompletion::Disconnected,
        Q1DemoEnd::Closed => RecordedCompletion::Closed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;
    use qa_core::identity::IdentityOwner;
    use qa_net::common::commands::ActorCommand;
    use qa_net::demo::write_nq_demo_header;
    use qa_net::q1_net::{NetQuakeMessage, QuakeWorldMessage};
    use qa_net::q2_net::{Q2ReadMode, Q2ServerData, Q2ServerMessageOptions, Q2ServerRecord, Q2WireFrame};
    use qa_net::q3::WireUserCommand;
    use qa_net::q3_net::{DownloadBlock, Gamestate, Q3ConnectionIdentity, Snapshot};
    use std::cell::RefCell;

    use super::super::q1_demo::{QwDemoPrediction, QwDemoShared};
    use super::super::q3_demo::{Q3DemoClockOptions, Q3DemoTiming};
    use super::super::qw_types::QwServerData;
    use super::super::types::Q2ApplicationGameState;
    use crate::bootstrap::demo_playback::Q3DemoProtocol;
    use crate::bootstrap::demo_recording::Q2ProtocolIdentity;

    /// Mock NetQuake remote.
    #[derive(Default)]
    struct MockNqRemote {
        samples: Vec<f64>,
    }

    impl NetQuakeDemoRemote for MockNqRemote {
        fn demo_ready(&self) -> bool {
            true
        }

        fn recorded_seconds(&self) -> f64 {
            0.0
        }

        fn receive(&mut self, _messages: &[NetQuakeMessage], _ms: f64, _assert: &dyn Fn()) {}

        fn set_demo_view_angles(&mut self, _angles: &Vec3, _interpolate: bool) {}

        fn sample_demo(&mut self, seconds: f64) {
            self.samples.push(seconds);
        }
    }

    /// Mock QuakeWorld prediction sink.
    #[derive(Default)]
    struct MockQwPrediction;

    impl QwDemoPrediction for MockQwPrediction {
        fn sent(&mut self, _sequence: u32, _command: &UserCommand, _milliseconds: f64) {}

        fn acknowledged(&mut self, _sequence: u32, _milliseconds: f64) {}
    }

    /// Mock QuakeWorld shared sink.
    #[derive(Default)]
    struct MockQwShared;

    impl QwDemoShared for MockQwShared {
        fn set_demo_view_angles(&mut self, _angles: &Vec3, _interpolate: bool) {}

        fn sample_demo(&mut self, _seconds: f64) {}
    }

    /// Mock QuakeWorld remote.
    #[derive(Default)]
    struct MockQwRemote {
        shared: MockQwShared,
        prediction: MockQwPrediction,
        samples: Vec<f64>,
    }

    impl QuakeWorldDemoRemote for MockQwRemote {
        fn demo_ready(&self) -> bool {
            true
        }

        fn shared(&mut self) -> &mut dyn QwDemoShared {
            &mut self.shared
        }

        fn prediction(&mut self) -> &mut dyn QwDemoPrediction {
            &mut self.prediction
        }

        fn server_data(&mut self, _message: &QuakeWorldMessage) {}

        fn game_state(&mut self, _data: &QwServerData, _models: &[String], _sounds: &[String], _assert: &dyn Fn()) {}

        fn receive(&mut self, _messages: &[QuakeWorldMessage], _ms: f64, _assert: &dyn Fn()) {}

        fn sample_presentation(&mut self, milliseconds: f64) {
            self.samples.push(milliseconds);
        }
    }

    /// Mock Quake II receiver host.
    #[derive(Default)]
    struct MockQ2Host;

    impl Q2ClientReceiverHost for MockQ2Host {
        fn protocol(&self) -> Q2ProtocolIdentity {
            Q2ProtocolIdentity::Classic
        }

        fn message_options(&self) -> Q2ServerMessageOptions {
            Q2ServerMessageOptions {
                read_mode: Q2ReadMode::Demo,
                max_config_strings: 1024,
                inventory_slots: 256,
                q2pro_extended_temp_entities: None,
            }
        }

        fn server_data(&mut self, _data: &Q2ServerData, _assert_current: &dyn Fn()) {}

        fn game_state(&mut self, _state: &Q2ApplicationGameState) {}

        fn frame(&mut self, _frame: &Q2WireFrame, _records: &[Q2ServerRecord], _now: u64) {}

        fn records(&mut self, _records: &[Q2ServerRecord]) {}

        fn disconnected(&mut self, _reason: &str) {}

        fn print(&mut self, _text: &str) {}
    }

    /// Mock Quake III client host.
    struct MockQ3Host {
        owner: IdentityOwner,
    }

    impl MockQ3Host {
        fn new() -> Self {
            Self {
                owner: IdentityOwner::create("recorded-test").expect("owner"),
            }
        }
    }

    impl Q3ApplicationClientHost for MockQ3Host {
        fn system_info(&mut self, _info: &str) {}

        fn snapshot(&mut self, _snapshot: &Snapshot, _ping: i32) {}

        fn map_restart(&mut self) {}

        fn print(&mut self, _text: &str) {}

        fn download_size(&mut self, size: i32) -> i32 {
            size
        }

        fn download(&mut self, _block: &DownloadBlock) {}

        fn clear_active(&mut self) {}

        fn gamestate(&mut self, _state: &Gamestate, _generation: i32) {}

        fn downloading(&self) -> bool {
            false
        }

        fn identity(&self) -> Q3ConnectionIdentity {
            Q3ConnectionIdentity {
                client: self.owner.client(0, 0),
                seat: None,
            }
        }

        fn userinfo(&self) -> String {
            String::new()
        }

        fn attach(&mut self, _connection: &qa_net::q3_net::Q3ClientConnection<'_>) {}

        fn command(&mut self, _command: &ActorCommand) -> WireUserCommand {
            WireUserCommand::default()
        }

        fn disconnected(&mut self, _reason: &str) {}
    }

    /// Mock Quake III demo clock.
    #[derive(Default)]
    struct MockQ3Clock;

    impl Q3DemoClock for MockQ3Clock {
        fn demo_timing(&self, _milliseconds: f64) -> Option<Q3DemoTiming> {
            None
        }

        fn advance(&mut self, _real_time: f64, _options: &Q3DemoClockOptions) -> Option<i32> {
            None
        }

        fn needs_demo_message(&self) -> bool {
            true
        }
    }

    type NqSource = RecordedRemoteSource<MockNqRemote, MockQwRemote, MockQ2Host, MockQ3Host, MockQ3Clock>;

    /// Shared completion log.
    type CompletedLog = Rc<RefCell<Vec<(RecordedCompletion, Option<DemoTiming>)>>>;

    fn nq_resource(bytes: Vec<u8>) -> DemoResource {
        DemoResource::Standard {
            family: DemoFamily::Q1,
            path: "demo.dem".to_string(),
            bytes,
        }
    }

    fn nq_source(bytes: Vec<u8>, timedemo: bool, completed: CompletedLog) -> NqSource {
        NqSource::new(
            nq_resource(bytes),
            RecordedRemote::Q1(MockNqRemote::default()),
            timedemo,
            Rc::new(CvarRegistry::new(Dialect::Q3)),
            move |reason, timing| completed.borrow_mut().push((reason, timing)),
        )
        .expect("source")
    }

    #[test]
    fn timing_text_formats() {
        assert_eq!(
            demo_timing_text(&DemoTiming {
                frames: 120,
                elapsed_milliseconds: 2000.0
            }),
            "120 frames, 2.000 seconds: 60.0 fps\n"
        );
        assert_eq!(
            demo_timing_text(&DemoTiming {
                frames: 0,
                elapsed_milliseconds: 0.0
            }),
            "0 frames, 0.000 seconds: 0.0 fps\n"
        );
    }

    #[test]
    fn mismatched_families_rejected() {
        let completed = Rc::new(RefCell::new(Vec::new()));
        let done = completed.clone();
        let error = match NqSource::new(
            nq_resource(write_nq_demo_header(-1)),
            RecordedRemote::Qw(MockQwRemote::default()),
            false,
            Rc::new(CvarRegistry::new(Dialect::Q3)),
            move |reason, timing| done.borrow_mut().push((reason, timing)),
        ) {
            Err(error) => error,
            Ok(_) => panic!("mismatched families admitted"),
        };
        assert_eq!(error.to_string(), "Recording and remote presentation families differ");
    }

    #[test]
    fn corrupt_nq_header_rejected() {
        let completed = Rc::new(RefCell::new(Vec::new()));
        let done = completed.clone();
        let error = match NqSource::new(
            nq_resource(b"not a demo".to_vec()),
            RecordedRemote::Q1(MockNqRemote::default()),
            false,
            Rc::new(CvarRegistry::new(Dialect::Q3)),
            move |reason, timing| done.borrow_mut().push((reason, timing)),
        ) {
            Err(error) => error,
            Ok(_) => panic!("corrupt header admitted"),
        };
        assert!(matches!(error, RecordedError::Demo(_)), "got {error:?}");
    }

    #[test]
    fn empty_nq_demo_completes_eof_once() {
        let completed = Rc::new(RefCell::new(Vec::new()));
        let mut source = nq_source(write_nq_demo_header(-1), false, completed.clone());
        assert_eq!(source.phase(), RecordedPhase::Loading);
        source.advance(16.0, 1, 100.0).expect("advance");
        assert_eq!(source.phase(), RecordedPhase::Closed);
        assert_eq!(*completed.borrow(), vec![(RecordedCompletion::Eof, None)]);
        source.advance(16.0, 2, 116.0).expect("again");
        assert_eq!(completed.borrow().len(), 1);
    }

    #[test]
    fn pause_freezes_completion() {
        let completed = Rc::new(RefCell::new(Vec::new()));
        let mut source = nq_source(write_nq_demo_header(-1), false, completed.clone());
        source.set_paused(true);
        assert!(source.is_paused());
        source.advance(16.0, 1, 100.0).expect("paused");
        assert_eq!(source.phase(), RecordedPhase::Loading);
        assert!(completed.borrow().is_empty());
        source.set_paused(false);
        source.advance(16.0, 2, 200.0).expect("resumed");
        assert_eq!(completed.borrow().len(), 1);
    }

    #[test]
    fn close_retires_source() {
        let completed = Rc::new(RefCell::new(Vec::new()));
        let mut source = nq_source(write_nq_demo_header(-1), false, completed.clone());
        source.close();
        assert_eq!(source.phase(), RecordedPhase::Closed);
        source.advance(16.0, 1, 100.0).expect("closed advance");
        assert!(completed.borrow().is_empty());
        assert!(source.timing().is_none());
    }

    #[test]
    fn select_player_rejects_non_q2() {
        let completed = Rc::new(RefCell::new(Vec::new()));
        let mut source = nq_source(write_nq_demo_header(-1), false, completed.clone());
        let error = source.select_player(3).expect_err("non-q2");
        assert_eq!(error.to_string(), "View selection requires a multiview Q2 recording");
    }

    #[test]
    fn empty_qw_demo_completes() {
        let completed = Rc::new(RefCell::new(Vec::new()));
        let done = completed.clone();
        let mut source = RecordedRemoteSource::<MockNqRemote, MockQwRemote, MockQ2Host, MockQ3Host, MockQ3Clock>::new(
            DemoResource::Standard {
                family: DemoFamily::Qw,
                path: "demo.qwd".to_string(),
                bytes: Vec::new(),
            },
            RecordedRemote::Qw(MockQwRemote::default()),
            false,
            Rc::new(CvarRegistry::new(Dialect::Q3)),
            move |reason, timing| done.borrow_mut().push((reason, timing)),
        )
        .expect("source");
        source.advance(16.0, 1, 100.0).expect("advance");
        assert_eq!(completed.borrow().len(), 1);
        assert_eq!(completed.borrow()[0].0, RecordedCompletion::Eof);
    }

    #[test]
    fn q2_garbage_rejected() {
        let completed = Rc::new(RefCell::new(Vec::new()));
        let done = completed.clone();
        let error = match RecordedRemoteSource::<MockNqRemote, MockQwRemote, MockQ2Host, MockQ3Host, MockQ3Clock>::new(
            DemoResource::Standard {
                family: DemoFamily::Q2,
                path: "demo.dm2".to_string(),
                bytes: b"garbage".to_vec(),
            },
            RecordedRemote::Q2(Q2RecordedRemote {
                host: MockQ2Host,
                mvd_presentation: None,
                select_recorded_view: None,
                sample_demo: Box::new(|_| {}),
            }),
            false,
            Rc::new(CvarRegistry::new(Dialect::Q3)),
            move |reason, timing| done.borrow_mut().push((reason, timing)),
        ) {
            Err(error) => error,
            Ok(_) => panic!("garbage admitted"),
        };
        assert!(matches!(error, RecordedError::Q2(_)), "got {error:?}");
    }

    #[test]
    fn q3_empty_demo_completes() {
        let completed = Rc::new(RefCell::new(Vec::new()));
        let done = completed.clone();
        let mut source = RecordedRemoteSource::<MockNqRemote, MockQwRemote, MockQ2Host, MockQ3Host, MockQ3Clock>::new(
            DemoResource::Q3 {
                path: "demo.dm3".to_string(),
                bytes: Vec::new(),
                protocol: Q3DemoProtocol::P68,
            },
            RecordedRemote::Q3(Q3RecordedRemote {
                host: MockQ3Host::new(),
                clock: MockQ3Clock,
                sample_presentation: Box::new(|_| {}),
            }),
            false,
            Rc::new(CvarRegistry::new(Dialect::Q3)),
            move |reason, timing| done.borrow_mut().push((reason, timing)),
        )
        .expect("source");
        source.advance(16.0, 1, 100.0).expect("advance");
        assert_eq!(completed.borrow().len(), 1);
    }
}
