//! Quake III demo playback (donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q3-demo.ts`).
//!
//! The donor is `async`; this sync port resolves every host call inline.
//! Like [`Q3ClientNetwork`](super::q3_client::Q3ClientNetwork), the endpoint
//! owns its [`Q3ClientConnection`](qa_net::q3_net::Q3ClientConnection) through
//! [`Q3ConnectionCell`](super::types::Q3ConnectionCell) and drains queued
//! binding callbacks after each demo read, preserving donor order. The demo
//! bytes are owned by the playback: they are heap-leased to the framed
//! reader for the playback lifetime and reclaimed in [`Drop`]. The clock is
//! the canonical [`Q3ClientClock`](qa_net::q3_clock::Q3ClientClock), which
//! implements the structural [`Q3DemoClock`] trait; the caller owns the
//! clock and publishes snapshots into it, matching the donor.

use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use qa_net::q3_clock::{Q3ClientClock, Q3ClockOptions};
use qa_net::q3_net::{
    DemoEnd, DemoMessageReader, DemoReader, DownloadBlock, Gamestate, Q3ClientBindings, Q3ClientConnection,
    Q3ClientMode, Q3ConnectionIdentity, Q3DemoRead, Q3NetError, Q3Product, Snapshot,
};
use thiserror::Error;

use super::q3_client::Q3ApplicationClientHost;
use super::types::Q3ConnectionCell;

/// Quake III demo playback failure.
#[derive(Debug, Error)]
pub enum Q3DemoError {
    /// Donor failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] Q3NetError),
}

/// Quake III demo phase (donor `Q3DemoPlayback['state']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3DemoPhase {
    /// Awaiting the gamestate.
    Loading,
    /// Gamestate primed.
    Primed,
    /// Active snapshot.
    Active,
    /// Demo ended.
    Ended,
    /// Playback closed.
    Closed,
}

/// Demo timing report (donor `Q3ClientClock['demoTiming']` return).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3DemoTiming {
    /// Rendered demo frames.
    pub frames: i32,
    /// Elapsed milliseconds.
    pub elapsed_milliseconds: i32,
}

/// Demo clock options (donor `Q3ClockOptions`, with `demo` selected by the
/// playback when it advances).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3DemoClockOptions {
    /// Clock paused.
    pub paused: bool,
    /// Time nudge.
    pub time_nudge: f64,
    /// Timescale.
    pub timescale: f64,
    /// Demo mode (always true for playback).
    pub demo: bool,
    /// Frozen demo.
    pub freeze_demo: bool,
    /// Timedemo benchmark.
    pub timedemo: bool,
}

/// Demo frame advance options (donor `advanceFrame` options).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3DemoAdvanceOptions {
    /// Clock paused.
    pub paused: bool,
    /// Time nudge.
    pub time_nudge: f64,
    /// Timescale.
    pub timescale: f64,
    /// Frozen demo.
    pub freeze_demo: bool,
    /// Timedemo benchmark.
    pub timedemo: bool,
}

/// Demo clock surface (donor `Q3ClientClock`, projected to the demo calls).
pub trait Q3DemoClock {
    /// Benchmark timing, when the demo started (donor `demoTiming`).
    fn demo_timing(&self, milliseconds: f64) -> Option<Q3DemoTiming>;
    /// Advance the clock (donor `advance`).
    fn advance(&mut self, real_time: f64, options: &Q3DemoClockOptions) -> Option<i32>;
    /// Whether the clock needs another demo message (donor
    /// `needsDemoMessage`).
    fn needs_demo_message(&self) -> bool;
}

impl Q3DemoClock for Q3ClientClock {
    fn demo_timing(&self, milliseconds: f64) -> Option<Q3DemoTiming> {
        Q3ClientClock::demo_timing(self, milliseconds)
            .unwrap_or_else(|error| panic!("{error}"))
            .map(|timing| Q3DemoTiming {
                frames: timing.frames,
                elapsed_milliseconds: timing.elapsed_milliseconds,
            })
    }

    fn advance(&mut self, real_time: f64, options: &Q3DemoClockOptions) -> Option<i32> {
        let options = Q3ClockOptions {
            paused: options.paused,
            time_nudge: options.time_nudge,
            timescale: options.timescale,
            demo: options.demo,
            freeze_demo: options.freeze_demo,
            timedemo: options.timedemo,
        };
        Q3ClientClock::advance(self, real_time, &options).unwrap_or_else(|error| panic!("{error}"))
    }

    fn needs_demo_message(&self) -> bool {
        Q3ClientClock::needs_demo_message(self)
    }
}

/// Quake III demo frame (donor `Q3DemoFrame`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3DemoFrame {
    /// No frame yet.
    Pending,
    /// Playable frame.
    Frame {
        /// Server time.
        server_time: i32,
    },
    /// Demo ended.
    End {
        /// End record.
        end: DemoEnd,
        /// Benchmark timing.
        timing: Option<Q3DemoTiming>,
    },
}

/// Deferred demo binding callback (donor `Q3ClientBindings` methods; the
/// demo drops snapshots instead of queueing them).
#[derive(Debug, Clone, PartialEq)]
enum Q3DemoCallback {
    /// Print text.
    Print(String),
    /// Clear active state.
    ClearActive,
    /// Apply system info.
    SystemInfo(String),
    /// Apply a gamestate.
    Gamestate {
        /// Game state.
        state: Box<Gamestate>,
        /// Generation.
        generation: i32,
    },
    /// Receive a download block.
    Download(DownloadBlock),
    /// Restart the map.
    MapRestart,
    /// Remote levelshot request, always rejected.
    LevelShot,
}

/// Demo state shared with the bindings adapter.
struct Q3DemoShared<H> {
    /// Application host.
    host: H,
    /// Playback phase.
    state: Q3DemoPhase,
    /// Queued binding callbacks.
    events: Vec<Q3DemoCallback>,
    /// Host panic inside an inline query: truncate position plus reason.
    fatal_at: Option<(usize, String)>,
}

/// Bindings adapter leased to the demo connection.
struct Q3DemoAdapter<H> {
    /// Shared demo state.
    shared: Rc<RefCell<Q3DemoShared<H>>>,
}

impl<H: Q3ApplicationClientHost> Q3ClientBindings for Q3DemoAdapter<H> {
    fn assert_current(&mut self) {
        if self.shared.borrow().state == Q3DemoPhase::Closed {
            panic!("Q3 demo belongs to a retired source");
        }
    }

    fn print(&mut self, text: &str) {
        self.shared
            .borrow_mut()
            .events
            .push(Q3DemoCallback::Print(text.to_string()));
    }

    fn clear_active(&mut self) {
        self.shared.borrow_mut().events.push(Q3DemoCallback::ClearActive);
    }

    fn system_info(&mut self, info: &str) {
        self.shared
            .borrow_mut()
            .events
            .push(Q3DemoCallback::SystemInfo(info.to_string()));
    }

    fn gamestate(&mut self, state: &Gamestate, generation: i32) {
        self.shared.borrow_mut().events.push(Q3DemoCallback::Gamestate {
            state: Box::new(state.clone()),
            generation,
        });
    }

    fn snapshot(&mut self, _snapshot: &Snapshot, _ping: i32) {
        // The demo lifecycle ignores snapshots.
    }

    fn download_size(&mut self, size: i32) -> i32 {
        let outcome = catch_unwind(AssertUnwindSafe(|| self.shared.borrow_mut().host.download_size(size)));
        match outcome {
            Ok(value) => value,
            Err(payload) => {
                let mut shared = self.shared.borrow_mut();
                let reason = panic_text(&payload);
                shared.fatal_at = Some((shared.events.len(), reason));
                0
            }
        }
    }

    fn download(&mut self, block: &DownloadBlock) {
        self.shared
            .borrow_mut()
            .events
            .push(Q3DemoCallback::Download(block.clone()));
    }

    fn map_restart(&mut self) {
        self.shared.borrow_mut().events.push(Q3DemoCallback::MapRestart);
    }

    fn level_shot(&mut self) {
        self.shared.borrow_mut().events.push(Q3DemoCallback::LevelShot);
    }

    fn local_server_running(&self) -> bool {
        false
    }
}

/// Best-effort panic payload text (donor `error.message`).
fn panic_text(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        return text.to_string();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "Q3 host failure".to_string()
}

/// Run a host call, converting a panic into an error with no state change
/// (donor throws out of `prime`/`advanceFrame`).
fn guard_pass<H, T>(shared: &Rc<RefCell<Q3DemoShared<H>>>, call: impl FnOnce(&mut H) -> T) -> Result<T, Q3DemoError> {
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let mut shared = shared.borrow_mut();
        call(&mut shared.host)
    }));
    match outcome {
        Ok(value) => Ok(value),
        Err(payload) => Err(Q3DemoError::Message(panic_text(&payload))),
    }
}

/// Quake III demo playback options (donor `Q3DemoPlaybackOptions`, taking
/// the demo bytes the framed reader borrows).
pub struct Q3DemoPlaybackOptions<H, C> {
    /// Application host.
    pub host: H,
    /// Demo clock.
    pub clock: C,
    /// Framed demo bytes.
    pub bytes: Vec<u8>,
}

/// Quake III demo playback (`Q3DemoPlayback`).
pub struct Q3DemoPlayback<H, C> {
    shared: Rc<RefCell<Q3DemoShared<H>>>,
    cell: Q3ConnectionCell<Q3DemoAdapter<H>, Q3ClientConnection<'static>>,
    bytes: *mut [u8],
    clock: C,
    end: Option<DemoEnd>,
    first_frame_skipped: bool,
    busy: bool,
}

// The bytes outlive the connection: `Drop` clears the cell (dropping the
// reader) before reclaiming them.
impl<H, C> Drop for Q3DemoPlayback<H, C> {
    fn drop(&mut self) {
        self.cell.clear();
        drop(unsafe { Box::from_raw(self.bytes) });
    }
}

impl<H, C> Q3DemoPlayback<H, C>
where
    H: Q3ApplicationClientHost + 'static,
    C: Q3DemoClock,
{
    /// Build a demo playback.
    pub fn new(options: Q3DemoPlaybackOptions<H, C>) -> Result<Self, Q3DemoError> {
        let shared = Rc::new(RefCell::new(Q3DemoShared {
            host: options.host,
            state: Q3DemoPhase::Loading,
            events: Vec::new(),
            fatal_at: None,
        }));
        // SAFETY: the leaked bytes are reclaimed in `Drop` after the cell
        // (and its reader) is cleared, so every borrow below outlives its use.
        let bytes_ptr = Box::leak(options.bytes.into_boxed_slice()) as *mut [u8];
        let reader: Box<dyn DemoMessageReader> = Box::new(DemoReader::new(unsafe { &*bytes_ptr }));
        let identity: Q3ConnectionIdentity = guard_pass(&shared, |host| host.identity())?;
        let adapter_shared = shared.clone();
        let mut cell = Q3ConnectionCell::new(Q3DemoAdapter { shared: adapter_shared });
        cell.build(|adapter| {
            // SAFETY: the cell owns the adapter box exclusively, drops any
            // previous connection before this lease, and drops this
            // connection before reclaiming the box, so the lease outlives
            // the connection (see `Q3ConnectionCell`).
            let leased: &'static mut Q3DemoAdapter<H> = unsafe { &mut *(adapter as *mut Q3DemoAdapter<H>) };
            Q3ClientConnection::new(identity, Q3Product::Base, Q3ClientMode::Demo { reader }, leased)
        });
        let playback = Self {
            shared,
            cell,
            bytes: bytes_ptr,
            clock: options.clock,
            end: None,
            first_frame_skipped: false,
            busy: false,
        };
        let connection = playback.cell.connection().expect("connection just built");
        guard_pass(&playback.shared, |host| host.attach(connection))?;
        Ok(playback)
    }

    /// Borrow the native connection (donor `connection`).
    pub fn connection(&self) -> &Q3ClientConnection<'static> {
        self.cell.connection().expect("demo connection is always built")
    }

    /// Playback phase (donor `phase`).
    pub fn phase(&self) -> Q3DemoPhase {
        self.shared.borrow().state
    }

    /// Read until the gamestate primes, or the demo ends (donor `prime`).
    pub fn prime(&mut self, real_time: f64) -> Result<Option<DemoEnd>, Q3DemoError> {
        self.begin()?;
        let outcome = catch_unwind(AssertUnwindSafe(|| self.prime_inner(real_time)));
        self.busy = false;
        match outcome {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Advance one demo frame (donor `advanceFrame`).
    pub fn advance_frame(
        &mut self,
        real_time: f64,
        options: &Q3DemoAdvanceOptions,
    ) -> Result<Q3DemoFrame, Q3DemoError> {
        self.begin()?;
        let outcome = catch_unwind(AssertUnwindSafe(|| self.advance_frame_inner(real_time, options)));
        self.busy = false;
        match outcome {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Retire the playback (donor `close`).
    pub fn close(&mut self) {
        self.shared.borrow_mut().state = Q3DemoPhase::Closed;
    }

    /// Prime with the busy guard held.
    fn prime_inner(&mut self, real_time: f64) -> Result<Option<DemoEnd>, Q3DemoError> {
        while self.shared.borrow().state == Q3DemoPhase::Loading {
            if let Some(end) = self.read(real_time)? {
                return Ok(Some(end));
            }
        }
        Ok(None)
    }

    /// Advance with the busy guard held.
    fn advance_frame_inner(
        &mut self,
        real_time: f64,
        options: &Q3DemoAdvanceOptions,
    ) -> Result<Q3DemoFrame, Q3DemoError> {
        if let Some(end) = self.end {
            return Ok(self.completed(real_time, end));
        }
        while self.shared.borrow().state == Q3DemoPhase::Loading {
            if let Some(end) = self.read(real_time)? {
                return Ok(self.completed(real_time, end));
            }
        }
        if !self.first_frame_skipped {
            self.first_frame_skipped = true;
            return Ok(Q3DemoFrame::Pending);
        }
        if self.shared.borrow().state == Q3DemoPhase::Primed {
            if let Some(end) = self.read(real_time)? {
                return Ok(self.completed(real_time, end));
            }
        }
        let snapshot = self.cell.connection().expect("demo connection").history.latest();
        if snapshot.as_ref().is_some_and(|snapshot| snapshot.flags & 2 == 0) {
            self.shared.borrow_mut().state = Q3DemoPhase::Active;
        }
        if self.shared.borrow().state != Q3DemoPhase::Active {
            return Ok(Q3DemoFrame::Pending);
        }
        let clock_options = Q3DemoClockOptions {
            paused: options.paused,
            time_nudge: options.time_nudge,
            timescale: options.timescale,
            demo: true,
            freeze_demo: options.freeze_demo,
            timedemo: options.timedemo,
        };
        let Some(time) = self.clock.advance(real_time, &clock_options) else {
            return Ok(Q3DemoFrame::Pending);
        };
        while self.clock.needs_demo_message() {
            if let Some(end) = self.read(real_time)? {
                return Ok(self.completed(real_time, end));
            }
        }
        Ok(Q3DemoFrame::Frame { server_time: time })
    }

    /// Read one demo message, or the cached end (donor `read`).
    fn read(&mut self, real_time: f64) -> Result<Option<DemoEnd>, Q3DemoError> {
        self.assert_current()?;
        if let Some(end) = self.end {
            return Ok(Some(end));
        }
        let message = {
            let connection = self.cell.connection_mut().expect("demo connection");
            connection.read_demo(real_time as i32)?
        };
        self.drain()?;
        self.assert_current()?;
        match message {
            Q3DemoRead::End(end) => {
                self.end = Some(end);
                self.shared.borrow_mut().state = Q3DemoPhase::Ended;
                Ok(Some(end))
            }
            Q3DemoRead::Message(_) => Ok(None),
        }
    }

    /// Report a completed demo (donor `completed`).
    fn completed(&self, real_time: f64, end: DemoEnd) -> Q3DemoFrame {
        Q3DemoFrame::End {
            end,
            timing: self.clock.demo_timing(real_time),
        }
    }

    /// Take the busy guard (donor `begin`).
    fn begin(&mut self) -> Result<(), Q3DemoError> {
        self.assert_current()?;
        if self.busy {
            return Err(Q3DemoError::Message(
                "Q3 demo message processing is already in progress".to_string(),
            ));
        }
        self.busy = true;
        Ok(())
    }

    /// Reject retired use (donor `assertCurrent`).
    fn assert_current(&self) -> Result<(), Q3DemoError> {
        if self.shared.borrow().state == Q3DemoPhase::Closed {
            return Err(Q3DemoError::Message("Q3 demo belongs to a retired source".to_string()));
        }
        Ok(())
    }

    /// Drain queued binding callbacks in order (donor inline awaits).
    fn drain(&mut self) -> Result<(), Q3DemoError> {
        let events = std::mem::take(&mut self.shared.borrow_mut().events);
        let fatal = self.shared.borrow_mut().fatal_at.take();
        let mut events = events;
        if let Some((at, _)) = &fatal {
            events.truncate(*at);
        }
        for event in events {
            self.apply(event)?;
        }
        if let Some((_, reason)) = fatal {
            return Err(Q3DemoError::Message(reason));
        }
        Ok(())
    }

    /// Apply one deferred callback.
    fn apply(&mut self, event: Q3DemoCallback) -> Result<(), Q3DemoError> {
        match event {
            Q3DemoCallback::Print(text) => {
                guard_pass(&self.shared, |host| host.print(&text))?;
            }
            Q3DemoCallback::ClearActive => {
                guard_pass(&self.shared, |host| host.clear_active())?;
                self.shared.borrow_mut().state = Q3DemoPhase::Loading;
            }
            Q3DemoCallback::SystemInfo(info) => {
                guard_pass(&self.shared, |host| host.system_info(&info))?;
            }
            Q3DemoCallback::Gamestate { state, generation } => {
                guard_pass(&self.shared, |host| host.gamestate(&state, generation))?;
                if guard_pass(&self.shared, |host| host.downloading())? {
                    return Err(Q3DemoError::Message(
                        "Q3 demo cannot wait for package downloads".to_string(),
                    ));
                }
                self.shared.borrow_mut().state = Q3DemoPhase::Primed;
            }
            Q3DemoCallback::Download(block) => {
                guard_pass(&self.shared, |host| host.download(&block))?;
            }
            Q3DemoCallback::MapRestart => {
                guard_pass(&self.shared, |host| host.map_restart())?;
            }
            Q3DemoCallback::LevelShot => {
                return Err(Q3DemoError::Message(
                    "Remote server cannot request a local levelshot".to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_net::q3_net::{
        encode_server_message, DemoEndReason, GamestateEntry, Q3EntityState, Q3PlayerState, ServerMessageContext,
        ServerOperation, SnapshotValidity,
    };

    struct MockHost {
        identity: Q3ConnectionIdentity,
        downloading: bool,
        gamestates: u32,
        attached: bool,
        prints: Vec<String>,
    }

    impl MockHost {
        fn new() -> Self {
            let owner = IdentityOwner::create("q3-demo-test").expect("owner");
            Self {
                identity: Q3ConnectionIdentity {
                    client: owner.client(0, 0),
                    seat: None,
                },
                downloading: false,
                gamestates: 0,
                attached: false,
                prints: Vec::new(),
            }
        }
    }

    impl Q3ApplicationClientHost for MockHost {
        fn system_info(&mut self, _info: &str) {}
        fn snapshot(&mut self, _snapshot: &Snapshot, _ping: i32) {}
        fn map_restart(&mut self) {}
        fn print(&mut self, text: &str) {
            self.prints.push(text.to_string());
        }
        fn download_size(&mut self, size: i32) -> i32 {
            size
        }
        fn download(&mut self, _block: &DownloadBlock) {}
        fn clear_active(&mut self) {}
        fn gamestate(&mut self, _state: &Gamestate, _generation: i32) {
            self.gamestates += 1;
        }
        fn downloading(&self) -> bool {
            self.downloading
        }
        fn identity(&self) -> Q3ConnectionIdentity {
            self.identity.clone()
        }
        fn userinfo(&self) -> String {
            String::new()
        }
        fn attach(&mut self, _connection: &Q3ClientConnection<'_>) {
            self.attached = true;
        }
        fn command(&mut self, _command: &qa_net::common::commands::ActorCommand) -> qa_net::q3::WireUserCommand {
            qa_net::q3::WireUserCommand::default()
        }
        fn disconnected(&mut self, _reason: &str) {}
    }

    struct MockClock {
        times: Vec<Option<i32>>,
        needs: Vec<bool>,
        timing: Option<Q3DemoTiming>,
        advances: u32,
    }

    impl MockClock {
        fn new() -> Self {
            Self {
                times: Vec::new(),
                needs: Vec::new(),
                timing: None,
                advances: 0,
            }
        }
    }

    impl Q3DemoClock for MockClock {
        fn demo_timing(&self, _milliseconds: f64) -> Option<Q3DemoTiming> {
            self.timing
        }

        fn advance(&mut self, _real_time: f64, options: &Q3DemoClockOptions) -> Option<i32> {
            assert!(options.demo);
            self.advances += 1;
            self.times.pop().unwrap_or(Some(100))
        }

        fn needs_demo_message(&self) -> bool {
            self.needs.last().copied().unwrap_or(false)
        }
    }

    fn message_context<'a>(
        message_number: i32,
        baseline: &'a dyn Fn(i32) -> Option<Q3EntityState>,
        history: &'a dyn Fn(i32) -> Option<qa_net::q3_net::SnapshotHistoryEntry>,
    ) -> ServerMessageContext<'a> {
        ServerMessageContext {
            product: Q3Product::Base,
            message_number,
            reliable_sequence: 0,
            server_command_sequence: 0,
            parse_entities_number: 0,
            baseline,
            history,
        }
    }

    fn gamestate_bytes() -> Vec<u8> {
        let baseline = |_: i32| None;
        let history = |_: i32| None;
        let context = message_context(1, &baseline, &history);
        encode_server_message(
            0,
            &[ServerOperation::Gamestate(Box::new(Gamestate {
                command_sequence: 0,
                entries: vec![GamestateEntry::Configstring {
                    index: 1,
                    value: "\\sv_serverid\\7".to_string(),
                }],
                client_number: 0,
                checksum_feed: 0,
            }))],
            &context,
        )
        .expect("gamestate")
    }

    fn snapshot_bytes(server_time: i32) -> Vec<u8> {
        let baseline = |_: i32| None;
        let history = |_: i32| None;
        let context = message_context(2, &baseline, &history);
        encode_server_message(
            0,
            &[ServerOperation::Snapshot {
                validity: SnapshotValidity::Valid,
                snapshot: Box::new(Snapshot {
                    message_number: 2,
                    server_time,
                    delta_number: 0,
                    flags: 0,
                    server_command_number: 0,
                    parse_entities_number: 0,
                    area_mask: Vec::new(),
                    player_state: Q3PlayerState::new(Q3Product::Base),
                    entities: Vec::new(),
                }),
            }],
            &context,
        )
        .expect("snapshot")
    }

    fn frame(sequence: i32, payload: &[u8]) -> Vec<u8> {
        let mut bytes = sequence.to_le_bytes().to_vec();
        bytes.extend_from_slice(&(payload.len() as i32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn terminator(sequence: i32) -> Vec<u8> {
        let mut bytes = sequence.to_le_bytes().to_vec();
        bytes.extend_from_slice(&(-1i32).to_le_bytes());
        bytes
    }

    fn demo_bytes() -> Vec<u8> {
        let mut bytes = frame(1, &gamestate_bytes());
        bytes.extend_from_slice(&frame(2, &snapshot_bytes(50)));
        bytes.extend_from_slice(&terminator(3));
        bytes
    }

    fn advance_options() -> Q3DemoAdvanceOptions {
        Q3DemoAdvanceOptions {
            paused: false,
            time_nudge: 0.0,
            timescale: 1.0,
            freeze_demo: false,
            timedemo: false,
        }
    }

    fn playback(bytes: Vec<u8>, clock: MockClock, downloading: bool) -> Q3DemoPlayback<MockHost, MockClock> {
        let mut host = MockHost::new();
        host.downloading = downloading;
        Q3DemoPlayback::new(Q3DemoPlaybackOptions { host, clock, bytes }).expect("playback")
    }

    #[test]
    fn prime_reaches_primed() {
        let mut demo = playback(demo_bytes(), MockClock::new(), false);
        assert!(demo.shared.borrow().host.attached);
        assert_eq!(demo.prime(1000.0).expect("prime"), None);
        assert_eq!(demo.phase(), Q3DemoPhase::Primed);
        assert_eq!(demo.shared.borrow().host.gamestates, 1);
        assert_eq!(demo.connection().server_id, 7);
    }

    #[test]
    fn prime_rejects_package_downloads() {
        let mut demo = playback(demo_bytes(), MockClock::new(), true);
        let error = demo.prime(1000.0).expect_err("downloading");
        assert_eq!(error.to_string(), "Q3 demo cannot wait for package downloads");
    }

    #[test]
    fn advance_frame_skips_first_then_frames() {
        let mut demo = playback(demo_bytes(), MockClock::new(), false);
        assert_eq!(demo.prime(1000.0).expect("prime"), None);
        // First advance skips one frame without publishing.
        assert_eq!(
            demo.advance_frame(1000.0, &advance_options()).expect("skip"),
            Q3DemoFrame::Pending
        );
        // The primed read consumes the snapshot; the clock publishes time.
        let frame = demo.advance_frame(1050.0, &advance_options()).expect("frame");
        assert_eq!(frame, Q3DemoFrame::Frame { server_time: 100 });
        assert_eq!(demo.phase(), Q3DemoPhase::Active);
    }

    #[test]
    fn advance_frame_ends_with_timing() {
        let mut clock = MockClock::new();
        clock.timing = Some(Q3DemoTiming {
            frames: 12,
            elapsed_milliseconds: 240,
        });
        let mut demo = playback(demo_bytes(), clock, false);
        assert_eq!(demo.prime(1000.0).expect("prime"), None);
        assert_eq!(
            demo.advance_frame(1000.0, &advance_options()).expect("skip"),
            Q3DemoFrame::Pending
        );
        // Drain the snapshot, then the terminator ends the demo.
        let frame = demo.advance_frame(1050.0, &advance_options()).expect("frame");
        assert!(matches!(frame, Q3DemoFrame::Frame { .. }));
        // Force the clock to demand the trailing message.
        let frame = demo.advance_frame(1100.0, &advance_options()).expect("frame 2");
        assert!(matches!(frame, Q3DemoFrame::Frame { .. }));
        // Now the terminator is next; needs_demo_message drives the end.
        demo.clock.needs.push(true);
        let frame = demo.advance_frame(1150.0, &advance_options()).expect("end");
        match frame {
            Q3DemoFrame::End { end, timing } => {
                assert_eq!(end.reason, DemoEndReason::Terminator);
                assert_eq!(
                    timing,
                    Some(Q3DemoTiming {
                        frames: 12,
                        elapsed_milliseconds: 240
                    })
                );
            }
            other => panic!("expected end, got {other:?}"),
        }
        assert_eq!(demo.phase(), Q3DemoPhase::Ended);
        // Ends are sticky and keep reporting timing.
        let again = demo.advance_frame(1200.0, &advance_options()).expect("sticky");
        assert!(matches!(again, Q3DemoFrame::End { .. }));
    }

    #[test]
    fn empty_demo_ends_at_prime() {
        let mut demo = playback(Vec::new(), MockClock::new(), false);
        let end = demo.prime(1000.0).expect("prime").expect("end");
        assert_eq!(end.reason, DemoEndReason::Eof);
        assert_eq!(demo.phase(), Q3DemoPhase::Ended);
    }

    #[test]
    fn close_retires_and_busy_guards() {
        let mut demo = playback(demo_bytes(), MockClock::new(), false);
        demo.close();
        assert_eq!(demo.phase(), Q3DemoPhase::Closed);
        let error = demo.prime(1000.0).expect_err("retired");
        assert_eq!(error.to_string(), "Q3 demo belongs to a retired source");
        let error = demo.advance_frame(1000.0, &advance_options()).expect_err("retired");
        assert_eq!(error.to_string(), "Q3 demo belongs to a retired source");

        let mut demo = playback(demo_bytes(), MockClock::new(), false);
        demo.busy = true;
        let error = demo.prime(1000.0).expect_err("busy");
        assert_eq!(error.to_string(), "Q3 demo message processing is already in progress");
        demo.busy = false;
        assert_eq!(demo.prime(1000.0).expect("prime"), None);
    }

    #[test]
    fn real_clock_drives_playback_to_end() {
        let mut clock = Q3ClientClock::new();
        clock.publish(&Snapshot {
            message_number: 2,
            server_time: 50,
            delta_number: 0,
            flags: 0,
            server_command_number: 0,
            parse_entities_number: 0,
            area_mask: Vec::new(),
            player_state: Q3PlayerState::new(Q3Product::Base),
            entities: Vec::new(),
        });
        let mut demo = Q3DemoPlayback::new(Q3DemoPlaybackOptions {
            host: MockHost::new(),
            clock,
            bytes: demo_bytes(),
        })
        .expect("playback");
        assert_eq!(demo.prime(1000.0).expect("prime"), None);
        assert_eq!(
            demo.advance_frame(1000.0, &advance_options()).expect("skip"),
            Q3DemoFrame::Pending
        );
        // The published snapshot activates the clock at time 50, which is
        // already past, so the clock demands the trailing terminator.
        match demo.advance_frame(1050.0, &advance_options()).expect("end") {
            Q3DemoFrame::End { end, timing } => {
                assert_eq!(end.reason, DemoEndReason::Terminator);
                assert_eq!(
                    timing,
                    Some(Q3DemoTiming {
                        frames: 0,
                        elapsed_milliseconds: 1050
                    })
                );
            }
            other => panic!("expected end, got {other:?}"),
        }
        assert_eq!(demo.phase(), Q3DemoPhase::Ended);
    }

    #[test]
    fn message_reader_smoke() {
        // The framed bytes parse outside the playback too.
        let bytes = demo_bytes();
        let mut reader = DemoReader::new(&bytes);
        let mut sequences = Vec::new();
        let record = reader.next(&mut |sequence| sequences.push(sequence)).expect("record");
        assert!(matches!(record, qa_net::q3_net::DemoRecord::Message(_)));
        assert_eq!(sequences, vec![1]);
    }
}
