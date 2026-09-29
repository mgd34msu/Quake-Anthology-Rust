//! Client-side movement prediction and reconciliation.
//!
//! Donor provenance:
//! `src/network/q1/prediction.ts` (`QuakeWorldPredictionHistory`,
//! `splitQuakeWorldCommand`), `src/network/q1/commands.ts`
//! (`splitQuakeWorldCommand`), `src/network/q2/prediction.ts`
//! (`Q2PredictionHistory`, `Q2CommandPacketHistory`),
//! `src/movement/q3/prediction.ts` (`Q3CommandHistory`) and
//! `src/content/q3/presentation/prediction.ts` (`ClientCommandHistory`,
//! `PredictionRuntime::predictPlayerState`, `buildSolidList`).
//!
//! The movement step itself stays with its owner (world/guest); this module
//! owns command history windows, replay loops, error decay, and snapshot
//! interpolation. Bookkeeping positions use workspace [`Vec3`] (`f32`).

use std::collections::BTreeMap;

use qa_core::math::{add3, scale3, vec3, Vec3};
use qa_core::numeric::qvm_float_to_int;

use crate::ClientError;

/// Command-history window: `CMD_BACKUP` / `PACKET_BACKUP` (Q3) and the
/// 64-slot Q1/Q2 histories.
pub const COMMAND_HISTORY: usize = 64;
/// QuakeWorld replay window: `outgoing - acknowledged >= 63` overruns.
pub const QW_REPLAY_WINDOW: i32 = 63;
/// QuakeWorld teleport snap: per-axis units that skip interpolation.
pub const QW_TELEPORT_SNAP: f32 = 128.0;
/// Q2 teleport snap: Manhattan units that clear prediction error.
pub const Q2_TELEPORT_SNAP: f32 = 80.0;

/// Fixed 64-slot command ring (`CL_GetUserCmd` semantics).
///
/// Slots start zeroed and survive map restarts; reads past the newest
/// command fail, reads older than the window return [`None`].
#[derive(Debug, Clone)]
pub struct CommandRing<C: Clone> {
    slots: Vec<C>,
    number: i32,
}

impl<C: Clone> CommandRing<C> {
    /// Zero-filled ring with no recorded commands.
    pub fn new(zero: C) -> Self {
        Self {
            slots: vec![zero; COMMAND_HISTORY],
            number: 0,
        }
    }

    /// Newest recorded command number.
    #[must_use]
    pub fn current_number(&self) -> i32 {
        self.number
    }

    /// Record a command, returning its number.
    pub fn append(&mut self, command: C) -> i32 {
        self.number = self.number.wrapping_add(1);
        let slot = (self.number & 63) as usize;
        self.slots[slot] = command;
        self.number
    }

    /// Read a recorded command, or [`None`] past the backup window.
    pub fn read(&self, number: i32) -> Result<Option<C>, ClientError> {
        if number > self.number {
            return Err(ClientError::FutureCommand {
                requested: number,
                current: self.number,
            });
        }
        if number <= self.number.wrapping_sub(COMMAND_HISTORY as i32) {
            return Ok(None);
        }
        Ok(Some(self.slots[(number & 63) as usize].clone()))
    }
}

/// Split a QuakeWorld command into `<= 50` ms slices.
///
/// The donor halves once and replays the truncated half twice, so odd
/// millisecond counts lose a millisecond; this preserves that behavior.
#[must_use]
pub fn qw_split_msec(milliseconds: i32) -> Vec<i32> {
    fn split(milliseconds: i32, out: &mut Vec<i32>) {
        if milliseconds > 50 {
            let half = milliseconds / 2;
            split(half, out);
            split(half, out);
        } else {
            out.push(milliseconds);
        }
    }
    let mut out = Vec::new();
    split(milliseconds, &mut out);
    out
}

/// QuakeWorld prediction target time (`cl_predict.c`).
///
/// `min(realtime, realtime - latency - min(0, push_latency_ms) / 1000)`.
#[must_use]
pub fn qw_predict_target(realtime_seconds: f64, latency_seconds: f64, push_latency_ms: f64) -> f64 {
    realtime_seconds.min(realtime_seconds - latency_seconds - push_latency_ms.min(0.0) / 1000.0)
}

/// QuakeWorld latency tracker (`acknowledged` in `cl_pred.c`).
///
/// Samples outside `[0, 1]` seconds are dropped; lower samples win
/// outright, higher samples creep up by a millisecond.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QwLatency {
    /// Smoothed latency in seconds.
    pub latency_seconds: f64,
}

impl QwLatency {
    /// Zero latency.
    #[must_use]
    pub const fn new() -> Self {
        Self { latency_seconds: 0.0 }
    }

    /// Fold an acknowledgement into the estimate.
    pub fn acknowledged(&mut self, sent_at_seconds: f64, received_at_seconds: f64) {
        let latency = received_at_seconds - sent_at_seconds;
        if !(0.0..=1.0).contains(&latency) {
            return;
        }
        self.latency_seconds = if latency < self.latency_seconds {
            latency
        } else {
            self.latency_seconds + 0.001
        };
    }
}

impl Default for QwLatency {
    fn default() -> Self {
        Self::new()
    }
}

/// Recorded QuakeWorld command frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QwFrame<C: Clone> {
    /// Command sequence.
    pub sequence: i32,
    /// Send time in seconds.
    pub sent_at_seconds: f64,
    /// Command payload.
    pub command: C,
}

/// QuakeWorld command history: record, acknowledge, bundle.
#[derive(Debug, Clone)]
pub struct QwHistory<C: Clone> {
    frames: BTreeMap<i32, QwFrame<C>>,
    /// Smoothed latency estimate.
    pub latency: QwLatency,
}

impl<C: Clone> QwHistory<C> {
    /// Empty history.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            frames: BTreeMap::new(),
            latency: QwLatency::new(),
        }
    }

    /// Record a sent command, pruning frames older than the window.
    pub fn record(&mut self, sequence: i32, sent_at_seconds: f64, command: C) {
        self.frames.insert(
            sequence,
            QwFrame {
                sequence,
                sent_at_seconds,
                command,
            },
        );
        let horizon = sequence.wrapping_sub(COMMAND_HISTORY as i32);
        self.frames.retain(|sequence, _| *sequence > horizon);
    }

    /// Fold an acknowledgement into the latency estimate.
    pub fn acknowledged(&mut self, sequence: i32, received_at_seconds: f64) {
        if let Some(frame) = self.frames.get(&sequence) {
            self.latency.acknowledged(frame.sent_at_seconds, received_at_seconds);
        }
    }

    /// Outgoing bundle: oldest, previous, and current commands.
    pub fn bundle(&self, sequence: i32) -> Result<[C; 3], ClientError> {
        let at = |sequence: i32| {
            self.frames
                .get(&sequence)
                .map(|frame| frame.command.clone())
                .ok_or(ClientError::HistoryExhausted)
        };
        Ok([at(sequence - 2)?, at(sequence - 1)?, at(sequence)?])
    }

    /// Whether the unacknowledged span overruns the replay window.
    #[must_use]
    pub fn overrun(acknowledged: i32, outgoing: i32) -> bool {
        i64::from(outgoing) - i64::from(acknowledged) >= i64::from(QW_REPLAY_WINDOW)
    }
}

impl<C: Clone> Default for QwHistory<C> {
    fn default() -> Self {
        Self::new()
    }
}

/// Prediction sample produced by one Q2 movement step.
#[derive(Debug, Clone)]
pub struct Q2Sample<State, Ground> {
    /// Stepped state.
    pub state: State,
    /// Predicted origin.
    pub origin: Vec3,
    /// Predicted view angles.
    pub view_angles: Vec3,
    /// Whether the step ended grounded.
    pub on_ground: bool,
    /// Ground identity for step-change detection.
    pub ground: Ground,
    /// Whether the step clipped (rerelease step smoothing).
    pub step_clip: bool,
    /// Whether the step may smooth (rerelease step smoothing).
    pub may_step: bool,
}

/// Q2 prediction profile: classic `cl_pred.c` vs rerelease smoothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2Profile {
    /// Classic Quake II.
    Classic,
    /// Rerelease / q2repro smoothing.
    Rerelease,
}

/// Movement host for Q2 prediction replay.
pub trait Q2PredictHost<State, Command, Ground> {
    /// Copy a state for replay.
    fn copy_state(&self, state: &State) -> State;
    /// Step `state` with `command`.
    fn predict(&mut self, state: State, command: &Command) -> Q2Sample<State, Ground>;
    /// Compare ground identities.
    fn same_ground(&self, left: &Ground, right: &Ground) -> bool;
}

/// Replayed Q2 prediction.
#[derive(Debug, Clone)]
pub struct Q2Predicted<State, Ground> {
    /// Final replayed sample.
    pub sample: Q2Sample<State, Ground>,
    /// Smoothed prediction error.
    pub error: Vec3,
    /// Pending step height.
    pub step: f32,
    /// Step timestamp in milliseconds.
    pub step_time_ms: i64,
}

/// Q2 command history with replay and camera correction.
#[derive(Debug)]
pub struct Q2Predictor<Host, State, Command: Clone, Ground: Clone> {
    host: Host,
    /// Phantom state type (states live in samples).
    state: std::marker::PhantomData<State>,
    profile: Q2Profile,
    capacity: i32,
    commands: BTreeMap<i32, Command>,
    origins: BTreeMap<i32, Vec3>,
    last_ground: Option<Ground>,
    error: Vec3,
    step: f32,
    step_time_ms: i64,
}

impl<Host, State, Command: Clone, Ground: Clone> Q2Predictor<Host, State, Command, Ground> {
    /// History with the default 64-command window.
    #[must_use]
    pub const fn new(host: Host, profile: Q2Profile) -> Self {
        Self {
            host,
            state: std::marker::PhantomData,
            profile,
            capacity: COMMAND_HISTORY as i32,
            commands: BTreeMap::new(),
            origins: BTreeMap::new(),
            last_ground: None,
            error: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            step: 0.0,
            step_time_ms: 0,
        }
    }

    /// Current smoothed prediction error.
    #[must_use]
    pub fn error(&self) -> Vec3 {
        self.error
    }

    /// Remember a sent command, pruning older than the window.
    pub fn remember(&mut self, sequence: i32, command: Command) {
        self.commands.insert(sequence, command);
        let horizon = sequence - self.capacity;
        self.commands.retain(|key, _| *key > horizon);
        self.origins.retain(|key, _| *key > horizon);
    }

    /// Reconcile a server origin against the predicted one.
    ///
    /// Unknown sequences and teleports clear the error; classic stores
    /// the server origin even on teleports, rerelease does not.
    pub fn acknowledge(&mut self, sequence: i32, origin: Vec3) -> Vec3 {
        let Some(predicted) = self.origins.get(&sequence).copied() else {
            self.error = vec3(0.0, 0.0, 0.0);
            return self.error;
        };
        let delta = Vec3 {
            x: origin.x - predicted.x,
            y: origin.y - predicted.y,
            z: origin.z - predicted.z,
        };
        if delta.x.abs() + delta.y.abs() + delta.z.abs() > Q2_TELEPORT_SNAP {
            self.error = vec3(0.0, 0.0, 0.0);
            if self.profile == Q2Profile::Rerelease {
                return self.error;
            }
        } else {
            self.error = delta;
        }
        self.origins.insert(sequence, origin);
        self.error
    }

    /// Replay unacknowledged commands over `baseline`.
    pub fn replay(
        &mut self,
        baseline: Q2Sample<State, Ground>,
        acknowledged: i32,
        current: i32,
        now_ms: i64,
        frame_seconds: f64,
        pending: Option<&Command>,
    ) -> Result<Q2Predicted<State, Ground>, ClientError>
    where
        Host: Q2PredictHost<State, Command, Ground>,
    {
        if i64::from(current) - i64::from(acknowledged) >= i64::from(self.capacity) {
            return Err(ClientError::HistoryExhausted);
        }
        let mut sample = Q2Sample {
            state: self.host.copy_state(&baseline.state),
            origin: baseline.origin,
            view_angles: baseline.view_angles,
            on_ground: baseline.on_ground,
            ground: baseline.ground,
            step_clip: baseline.step_clip,
            may_step: baseline.may_step,
        };
        let last = match self.profile {
            Q2Profile::Classic => current - 1,
            Q2Profile::Rerelease => current,
        };
        for sequence in (acknowledged + 1)..=last {
            let Some(command) = self.commands.get(&sequence) else {
                return Err(ClientError::HistoryExhausted);
            };
            sample = self.host.predict(sample.state, command);
            self.origins.insert(sequence, sample.origin);
        }
        if self.profile == Q2Profile::Rerelease {
            if let Some(command) = pending {
                sample = self.host.predict(sample.state, command);
                self.origins.insert(current + 1, sample.origin);
            }
        }
        let anchor = match self.profile {
            Q2Profile::Classic => current - 2,
            Q2Profile::Rerelease => {
                if pending.is_some() {
                    current
                } else {
                    current - 1
                }
            }
        };
        if let Some(previous) = self.origins.get(&anchor) {
            let delta = sample.origin.z - previous.z;
            match self.profile {
                Q2Profile::Classic => {
                    if delta > 63.0 / 8.0 && delta < 20.0 && sample.on_ground {
                        self.step = delta;
                        self.step_time_ms = now_ms - (f64::from(frame_seconds as f32) * 500.0) as i64;
                    }
                }
                Q2Profile::Rerelease => {
                    let ground_changed = self
                        .last_ground
                        .as_ref()
                        .is_none_or(|ground| !self.host.same_ground(ground, &sample.ground));
                    if delta.abs() > 1.0
                        && delta.abs() < 20.0
                        && (baseline.on_ground || sample.step_clip)
                        && sample.on_ground
                        && sample.may_step
                        && ground_changed
                    {
                        let elapsed = now_ms - self.step_time_ms;
                        let old = if elapsed < 100 {
                            self.step * (100 - elapsed.min(100)) as f32 / 100.0
                        } else {
                            0.0
                        };
                        self.step = (old + delta).clamp(-32.0, 32.0);
                        self.step_time_ms = now_ms;
                    }
                }
            }
        }
        self.last_ground = Some(sample.ground.clone());
        Ok(Q2Predicted {
            sample,
            error: self.error,
            step: self.step,
            step_time_ms: self.step_time_ms,
        })
    }

    /// Clear history, error, and step state.
    pub fn clear(&mut self) {
        self.commands.clear();
        self.origins.clear();
        self.last_ground = None;
        self.error = vec3(0.0, 0.0, 0.0);
        self.step = 0.0;
        self.step_time_ms = 0;
    }
}

/// Rerelease packet acknowledgements resolve to command numbers here.
#[derive(Debug, Clone, Default)]
pub struct Q2CommandPacketHistory {
    entries: BTreeMap<i32, (i32, i64)>,
    capacity: i32,
}

impl Q2CommandPacketHistory {
    /// History with the default 64-packet window.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            capacity: COMMAND_HISTORY as i32,
        }
    }

    /// Record a sent packet's command number.
    pub fn sent(&mut self, packet: i32, command: i32, sent_at_ms: i64) {
        self.entries.insert(packet, (command, sent_at_ms));
        let horizon = packet - self.capacity;
        self.entries.retain(|key, _| *key > horizon);
    }

    /// Resolve an acknowledged packet to `(command, sent_at_ms)`.
    #[must_use]
    pub fn acknowledged(&self, packet: i32) -> Option<(i32, i64)> {
        self.entries.get(&packet).copied()
    }

    /// Clear the history.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Q3 prediction-error decay (`CG_PredictPlayerState`).
///
/// Integer decay scales the carried error by the remaining fraction;
/// otherwise the error restarts from zero. `decay_ms` is `cg_errorDecay`.
#[must_use]
pub fn q3_decay_error(error: Vec3, elapsed_ms: i32, decay_ms: f32, decay_integer: bool) -> Vec3 {
    if !decay_integer {
        return vec3(0.0, 0.0, 0.0);
    }
    let mut fraction = (decay_ms - elapsed_ms as f32) / decay_ms;
    if fraction < 0.0 {
        fraction = 0.0;
    }
    scale3(error, fraction)
}

/// Q3 `PACKET_BACKUP` overrun check: the oldest readable command is
/// newer than the snapshot but older than the current time.
#[must_use]
pub const fn q3_backup_exceeded(oldest_server_time_ms: i32, snapshot_command_time_ms: i32, now_ms: i32) -> bool {
    oldest_server_time_ms > snapshot_command_time_ms && oldest_server_time_ms < now_ms
}

/// Clamp `pmove_msec` into the source `[8, 33]` range.
#[must_use]
pub const fn clamp_pmove_msec(milliseconds: i32) -> i32 {
    if milliseconds < 8 {
        8
    } else if milliseconds > 33 {
        33
    } else {
        milliseconds
    }
}

/// Add a fresh prediction delta onto the decayed carried error.
#[must_use]
pub fn q3_accumulate_error(carried: Vec3, delta: Vec3) -> Vec3 {
    add3(delta, carried)
}

/// Exact `float32` vector interpolation shared by Q3 prediction and view.
#[must_use]
pub fn interpolate_vector(a: Vec3, b: Vec3, fraction: f32) -> Vec3 {
    vec3(
        a.x + fraction * (b.x - a.x),
        a.y + fraction * (b.y - a.y),
        a.z + fraction * (b.z - a.z),
    )
}

/// Exact `float32` angle interpolation with `±180` wrap.
#[must_use]
pub fn lerp_angle(from: f32, mut to: f32, fraction: f32) -> f32 {
    if to - from > 180.0 {
        to -= 360.0;
    }
    if to - from < -180.0 {
        to += 360.0;
    }
    from + fraction * (to - from)
}

/// Q3 `bobCycle` interpolation with 256-cycle wrap.
#[must_use]
pub fn interpolate_bob_cycle(previous: i32, next: i32, fraction: f32) -> i32 {
    let cycle = if next < previous { next + 256 } else { next };
    qvm_float_to_int(previous as f32 + fraction * (cycle - previous) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_round_trips_and_expires() {
        let mut ring = CommandRing::new(0_i32);
        for command in 1..=70 {
            assert_eq!(ring.append(command), command);
        }
        assert_eq!(ring.current_number(), 70);
        assert_eq!(ring.read(70).unwrap(), Some(70));
        assert_eq!(ring.read(7).unwrap(), Some(7));
        assert_eq!(ring.read(6).unwrap(), None);
        assert!(matches!(ring.read(71), Err(ClientError::FutureCommand { .. })));
    }

    #[test]
    fn qw_split_preserves_donor_truncation() {
        assert_eq!(qw_split_msec(30), vec![30]);
        assert_eq!(qw_split_msec(50), vec![50]);
        assert_eq!(qw_split_msec(100), vec![50, 50]);
        assert_eq!(qw_split_msec(101), vec![50, 50]);
        assert_eq!(qw_split_msec(120), vec![30, 30, 30, 30]);
    }

    #[test]
    fn qw_history_bundles_and_tracks_latency() {
        let mut history = QwHistory::new();
        for sequence in 1..=4 {
            history.record(sequence, f64::from(sequence) * 0.05, sequence * 10);
        }
        assert_eq!(history.bundle(3).unwrap(), [10, 20, 30]);
        assert!(history.bundle(9).is_err());
        history.acknowledged(2, 0.15);
        assert!((history.latency.latency_seconds - 0.001).abs() < f64::EPSILON);
        history.acknowledged(3, 10.0);
        assert!((history.latency.latency_seconds - 0.001).abs() < f64::EPSILON);
        assert!(QwHistory::<i32>::overrun(0, 63));
        assert!(!QwHistory::<i32>::overrun(0, 62));
        assert!((qw_predict_target(10.0, 0.05, -20.0) - 9.97).abs() < 1e-9);
        assert!((qw_predict_target(10.0, 0.05, 20.0) - 9.95).abs() < 1e-9);
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Lift(i32);

    struct LiftHost;
    impl Q2PredictHost<f32, i32, Lift> for LiftHost {
        fn copy_state(&self, state: &f32) -> f32 {
            *state
        }
        fn predict(&mut self, state: f32, command: &i32) -> Q2Sample<f32, Lift> {
            let height = state + *command as f32;
            Q2Sample {
                state: height,
                origin: vec3(0.0, 0.0, height),
                view_angles: vec3(0.0, 0.0, 0.0),
                on_ground: true,
                ground: Lift(*command),
                step_clip: false,
                may_step: true,
            }
        }
        fn same_ground(&self, left: &Lift, right: &Lift) -> bool {
            left == right
        }
    }

    fn baseline(height: f32) -> Q2Sample<f32, Lift> {
        Q2Sample {
            state: height,
            origin: vec3(0.0, 0.0, height),
            view_angles: vec3(0.0, 0.0, 0.0),
            on_ground: true,
            ground: Lift(0),
            step_clip: false,
            may_step: true,
        }
    }

    #[test]
    fn q2_classic_replay_detects_steps() {
        let mut predictor = Q2Predictor::new(LiftHost, Q2Profile::Classic);
        for sequence in 1..=4 {
            predictor.remember(sequence, 8);
        }
        let predicted = predictor.replay(baseline(0.0), 0, 4, 1000, 0.016, None).unwrap();
        assert_eq!(predicted.sample.origin.z, 24.0);
        assert_eq!(predicted.step, 8.0);
        assert_eq!(predicted.step_time_ms, 1000 - 8);
        let error = predictor.acknowledge(3, vec3(0.0, 0.0, 25.0));
        assert_eq!(error, vec3(0.0, 0.0, 1.0));
        let cleared = predictor.acknowledge(3, vec3(100.0, 0.0, 25.0));
        assert_eq!(cleared, vec3(0.0, 0.0, 0.0));
        assert!(predictor.replay(baseline(0.0), 0, 99, 1000, 0.016, None).is_err());
    }

    #[test]
    fn q2_rerelease_replay_uses_pending_command() {
        let mut predictor = Q2Predictor::new(LiftHost, Q2Profile::Rerelease);
        for sequence in 1..=3 {
            predictor.remember(sequence, 2);
        }
        let predicted = predictor.replay(baseline(0.0), 0, 3, 500, 0.016, Some(&6)).unwrap();
        assert_eq!(predicted.sample.origin.z, 12.0);
        assert_eq!(predicted.step, 6.0);
        assert_eq!(predicted.step_time_ms, 500);
    }

    #[test]
    fn q2_packet_history_resolves_commands() {
        let mut history = Q2CommandPacketHistory::new();
        history.sent(10, 40, 900);
        assert_eq!(history.acknowledged(10), Some((40, 900)));
        assert_eq!(history.acknowledged(11), None);
    }

    #[test]
    fn q3_prediction_math_matches_donor() {
        let decayed = q3_decay_error(vec3(8.0, 0.0, 0.0), 25, 100.0, true);
        assert_eq!(decayed, vec3(6.0, 0.0, 0.0));
        assert_eq!(
            q3_decay_error(vec3(8.0, 0.0, 0.0), 25, 100.0, false),
            vec3(0.0, 0.0, 0.0)
        );
        assert!(q3_backup_exceeded(120, 100, 200));
        assert!(!q3_backup_exceeded(90, 100, 200));
        assert_eq!(clamp_pmove_msec(4), 8);
        assert_eq!(clamp_pmove_msec(40), 33);
        assert_eq!(clamp_pmove_msec(16), 16);
        assert_eq!(
            q3_accumulate_error(vec3(1.0, 0.0, 0.0), vec3(0.0, 2.0, 0.0)),
            vec3(1.0, 2.0, 0.0)
        );
        assert_eq!(
            interpolate_vector(vec3(0.0, 0.0, 0.0), vec3(10.0, 0.0, 0.0), 0.25),
            vec3(2.5, 0.0, 0.0)
        );
        assert!((lerp_angle(170.0, -170.0, 0.5) - 180.0).abs() < 1e-5);
        assert_eq!(interpolate_bob_cycle(250, 4, 0.5), 255);
    }
}
