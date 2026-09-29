//! Quake II client prediction history.
//!
//! Donor provenance: `Q2PredictionSample`, `Q2PredictionHost`,
//! `Q2PredictionResult`, `Q2PredictionHistory`, and
//! `Q2CommandPacketHistory` in `src/network/q2/prediction.ts` (Quake II
//! `cl_pred.c` and q2repro `client/predict.c` history and camera
//! correction). Generics range over host traits like the donor's host
//! interfaces; origins and errors are `[f64; 3]` triples at donor
//! precision (`qa-core` math vectors are single-precision).

use std::collections::BTreeMap;

/// Prediction triple at donor precision.
pub type Q2PredictionVec = [f64; 3];

/// Default history capacity (donor `capacity = 64`).
pub const Q2_PREDICTION_CAPACITY: usize = 64;

/// One prediction sample (`Q2PredictionSample`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PredictionSample<TState, TGround> {
    /// Simulated state.
    pub state: TState,
    /// Predicted origin.
    pub origin: Q2PredictionVec,
    /// Predicted view angles.
    pub view_angles: Q2PredictionVec,
    /// Whether the sample rests on the ground.
    pub on_ground: bool,
    /// Ground identity.
    pub ground: TGround,
    /// Whether the sample clipped a step.
    pub step_clip: bool,
    /// Whether the sample may step up.
    pub may_step: bool,
}

/// Host simulation callbacks (`Q2PredictionHost`).
pub trait Q2PredictionHost<TState, TCommand, TGround> {
    /// Copy a simulation state.
    fn copy(&self, state: &TState) -> TState;
    /// Advance a state by one command.
    fn predict(&mut self, state: &TState, command: &TCommand) -> Q2PredictionSample<TState, TGround>;
    /// Compare ground identities.
    fn same_ground(&self, left: &TGround, right: &TGround) -> bool;
}

/// Prediction profile (`'classic' | 'rerelease'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2PredictionProfile {
    /// Classic Quake II prediction.
    Classic,
    /// Rerelease prediction.
    Rerelease,
}

/// Replay outcome (`Q2PredictionResult`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2PredictionResult<TState, TGround> {
    /// A command in the replay window is missing or the window is full.
    HistoryExhausted,
    /// Replay completed.
    Predicted {
        /// Final sample.
        sample: Q2PredictionSample<TState, TGround>,
        /// Current prediction error.
        error: Q2PredictionVec,
        /// Camera step smoothing.
        step: f64,
        /// Step timestamp in milliseconds.
        step_time: f64,
    },
}

/// Command and origin history with camera correction (`Q2PredictionHistory`).
#[derive(Debug, Clone)]
pub struct Q2PredictionHistory<TState, TCommand, TGround, H> {
    host: H,
    profile: Q2PredictionProfile,
    capacity: usize,
    commands: BTreeMap<i32, TCommand>,
    origins: BTreeMap<i32, Q2PredictionVec>,
    last_ground: Option<TGround>,
    error: Q2PredictionVec,
    step: f64,
    step_time: f64,
    phantom: std::marker::PhantomData<TState>,
}

impl<TState, TCommand, TGround, H> Q2PredictionHistory<TState, TCommand, TGround, H>
where
    H: Q2PredictionHost<TState, TCommand, TGround>,
{
    /// Build a history over a host.
    pub fn new(host: H, profile: Q2PredictionProfile, capacity: usize) -> Self {
        Self {
            host,
            profile,
            capacity,
            commands: BTreeMap::new(),
            origins: BTreeMap::new(),
            last_ground: None,
            error: [0.0, 0.0, 0.0],
            step: 0.0,
            step_time: 0.0,
            phantom: std::marker::PhantomData,
        }
    }

    /// Host simulation callbacks.
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Prediction profile.
    pub fn profile(&self) -> Q2PredictionProfile {
        self.profile
    }

    /// History capacity in sequences.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Evict entries at or below `sequence - capacity`.
    fn evict(commands: &mut BTreeMap<i32, TCommand>, origins: &mut BTreeMap<i32, Q2PredictionVec>, sequence: i32, capacity: usize) {
        let threshold = i64::from(sequence) - capacity as i64;
        commands.retain(|key, _| i64::from(*key) > threshold);
        origins.retain(|key, _| i64::from(*key) > threshold);
    }

    /// Remember a command (`remember`).
    pub fn remember(&mut self, sequence: i32, command: TCommand) {
        self.commands.insert(sequence, command);
        Self::evict(&mut self.commands, &mut self.origins, sequence, self.capacity);
    }

    /// Acknowledge a predicted origin (`acknowledge`); returns the error.
    pub fn acknowledge(&mut self, sequence: i32, origin: Q2PredictionVec) -> Q2PredictionVec {
        let Some(predicted) = self.origins.get(&sequence).copied() else {
            self.error = [0.0, 0.0, 0.0];
            return self.error;
        };
        let delta = [
            origin[0] - predicted[0],
            origin[1] - predicted[1],
            origin[2] - predicted[2],
        ];
        if delta[0].abs() + delta[1].abs() + delta[2].abs() > 80.0 {
            self.error = [0.0, 0.0, 0.0];
            if self.profile == Q2PredictionProfile::Rerelease {
                return self.error;
            }
        } else {
            self.error = delta;
        }
        self.origins.insert(sequence, origin);
        self.error
    }

    /// Replay unacknowledged commands (`replay`).
    pub fn replay(
        &mut self,
        baseline: &Q2PredictionSample<TState, TGround>,
        acknowledged: i32,
        current_sequence: i32,
        now_milliseconds: f64,
        frame_seconds: f64,
        pending_command: Option<&TCommand>,
    ) -> Q2PredictionResult<TState, TGround>
    where
        TGround: Clone,
    {
        if i64::from(current_sequence) - i64::from(acknowledged) >= self.capacity as i64 {
            return Q2PredictionResult::HistoryExhausted;
        }
        let mut sample = Q2PredictionSample {
            state: self.host.copy(&baseline.state),
            origin: baseline.origin,
            view_angles: baseline.view_angles,
            on_ground: baseline.on_ground,
            ground: baseline.ground.clone(),
            step_clip: baseline.step_clip,
            may_step: baseline.may_step,
        };
        let last_command = if self.profile == Q2PredictionProfile::Classic {
            current_sequence - 1
        } else {
            current_sequence
        };
        for sequence in acknowledged + 1..=last_command {
            let Some(command) = self.commands.get(&sequence) else {
                return Q2PredictionResult::HistoryExhausted;
            };
            // The borrow of the stored command ends before the sample moves.
            let next = self.host.predict(&sample.state, command);
            let origin = next.origin;
            sample = next;
            self.origins.insert(sequence, origin);
        }
        if self.profile == Q2PredictionProfile::Rerelease {
            if let Some(command) = pending_command {
                let next = self.host.predict(&sample.state, command);
                let origin = next.origin;
                sample = next;
                self.origins.insert(current_sequence + 1, origin);
            }
        }
        let previous_key = if self.profile == Q2PredictionProfile::Classic {
            current_sequence - 2
        } else if pending_command.is_none() {
            current_sequence - 1
        } else {
            current_sequence
        };
        if let Some(previous) = self.origins.get(&previous_key).copied() {
            let delta = sample.origin[2] - previous[2];
            if self.profile == Q2PredictionProfile::Classic {
                if delta > 63.0 / 8.0 && delta < 20.0 && sample.on_ground {
                    self.step = delta;
                    self.step_time = now_milliseconds - frame_seconds * 500.0;
                }
            } else if delta.abs() > 1.0
                && delta.abs() < 20.0
                && (baseline.on_ground || sample.step_clip)
                && sample.on_ground
                && sample.may_step
                && (self.last_ground.is_none()
                    || !self
                        .host
                        .same_ground(self.last_ground.as_ref().unwrap_or(&sample.ground), &sample.ground))
            {
                let elapsed = now_milliseconds - self.step_time;
                let old = if elapsed < 100.0 {
                    self.step * (100.0 - elapsed) / 100.0
                } else {
                    0.0
                };
                let smoothed = old + delta;
                self.step = if smoothed.is_nan() {
                    f64::NAN
                } else {
                    smoothed.clamp(-32.0, 32.0)
                };
                self.step_time = now_milliseconds;
            }
        }
        self.last_ground = Some(sample.ground.clone());
        Q2PredictionResult::Predicted {
            sample,
            error: self.error,
            step: self.step,
            step_time: self.step_time,
        }
    }

    /// Clear the history (`clear`).
    pub fn clear(&mut self) {
        self.commands.clear();
        self.origins.clear();
        self.last_ground = None;
        self.error = [0.0, 0.0, 0.0];
        self.step = 0.0;
        self.step_time = 0.0;
    }
}

/// Rerelease packet-to-command acknowledgements (`Q2CommandPacketHistory`).
#[derive(Debug, Clone, Default)]
pub struct Q2CommandPacketHistory {
    entries: BTreeMap<i32, (i32, f64)>,
    capacity: usize,
}

impl Q2CommandPacketHistory {
    /// Build a history.
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            capacity,
        }
    }

    /// Record a sent packet (`sent`).
    pub fn sent(&mut self, packet_sequence: i32, command_number: i32, sent_at: f64) {
        self.entries.insert(packet_sequence, (command_number, sent_at));
        let threshold = i64::from(packet_sequence) - self.capacity as i64;
        self.entries.retain(|key, _| i64::from(*key) > threshold);
    }

    /// Look up an acknowledged packet (`acknowledged`).
    pub fn acknowledged(&self, packet_sequence: i32) -> Option<(i32, f64)> {
        self.entries.get(&packet_sequence).copied()
    }

    /// Clear the history (`clear`).
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Position state advanced by move commands over numbered ground.
    struct Fixture;

    impl Q2PredictionHost<[f64; 3], [f64; 3], u32> for Fixture {
        fn copy(&self, state: &[f64; 3]) -> [f64; 3] {
            *state
        }

        fn predict(&mut self, state: &[f64; 3], command: &[f64; 3]) -> Q2PredictionSample<[f64; 3], u32> {
            let origin = [state[0] + command[0], state[1] + command[1], state[2] + command[2]];
            Q2PredictionSample {
                state: origin,
                origin,
                view_angles: [0.0, 0.0, 0.0],
                on_ground: true,
                ground: 1,
                step_clip: false,
                may_step: true,
            }
        }

        fn same_ground(&self, left: &u32, right: &u32) -> bool {
            left == right
        }
    }

    fn baseline() -> Q2PredictionSample<[f64; 3], u32> {
        Q2PredictionSample {
            state: [0.0, 0.0, 0.0],
            origin: [0.0, 0.0, 0.0],
            view_angles: [0.0, 0.0, 0.0],
            on_ground: true,
            ground: 1,
            step_clip: false,
            may_step: true,
        }
    }

    #[test]
    fn classic_replay_stops_before_current() {
        let mut history = Q2PredictionHistory::new(Fixture, Q2PredictionProfile::Classic, Q2_PREDICTION_CAPACITY);
        history.remember(1, [1.0, 0.0, 0.0]);
        history.remember(2, [0.0, 2.0, 0.0]);
        history.remember(3, [0.0, 0.0, 4.0]);
        let result = history.replay(&baseline(), 0, 3, 1000.0, 0.016, None);
        let Q2PredictionResult::Predicted { sample, .. } = result else {
            panic!("expected a prediction");
        };
        // Classic replays acknowledged+1 through current-1.
        assert_eq!(sample.origin, [1.0, 2.0, 0.0]);
    }

    #[test]
    fn rerelease_replay_includes_pending() {
        let mut history = Q2PredictionHistory::new(Fixture, Q2PredictionProfile::Rerelease, Q2_PREDICTION_CAPACITY);
        history.remember(1, [1.0, 0.0, 0.0]);
        let result = history.replay(&baseline(), 0, 1, 1000.0, 0.016, Some(&[0.0, 5.0, 0.0]));
        let Q2PredictionResult::Predicted { sample, .. } = result else {
            panic!("expected a prediction");
        };
        assert_eq!(sample.origin, [1.0, 5.0, 0.0]);
    }

    #[test]
    fn missing_commands_exhaust_history() {
        let mut history = Q2PredictionHistory::new(Fixture, Q2PredictionProfile::Classic, Q2_PREDICTION_CAPACITY);
        history.remember(1, [1.0, 0.0, 0.0]);
        assert!(matches!(
            history.replay(&baseline(), 0, 3, 1000.0, 0.016, None),
            Q2PredictionResult::HistoryExhausted
        ));
        assert!(matches!(
            history.replay(&baseline(), 0, 64, 1000.0, 0.016, None),
            Q2PredictionResult::HistoryExhausted
        ));
    }

    #[test]
    fn acknowledge_tracks_small_errors() {
        let mut history = Q2PredictionHistory::new(Fixture, Q2PredictionProfile::Classic, Q2_PREDICTION_CAPACITY);
        history.remember(1, [10.0, 0.0, 0.0]);
        assert!(matches!(
            history.replay(&baseline(), 0, 2, 1000.0, 0.016, None),
            Q2PredictionResult::Predicted { .. }
        ));
        assert_eq!(history.acknowledge(9, [0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        assert_eq!(history.acknowledge(1, [12.0, 0.0, 0.0]), [2.0, 0.0, 0.0]);
    }

    #[test]
    fn large_errors_reset_and_update_classic_origins() {
        let mut history = Q2PredictionHistory::new(Fixture, Q2PredictionProfile::Classic, Q2_PREDICTION_CAPACITY);
        history.remember(1, [10.0, 0.0, 0.0]);
        let _ = history.replay(&baseline(), 0, 2, 1000.0, 0.016, None);
        assert_eq!(history.acknowledge(1, [200.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        // Classic records the authoritative origin even on reset.
        assert_eq!(history.acknowledge(1, [201.0, 0.0, 0.0]), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn rerelease_skips_origin_update_on_reset() {
        let mut history = Q2PredictionHistory::new(Fixture, Q2PredictionProfile::Rerelease, Q2_PREDICTION_CAPACITY);
        history.remember(1, [10.0, 0.0, 0.0]);
        let _ = history.replay(&baseline(), 0, 1, 1000.0, 0.016, None);
        assert_eq!(history.acknowledge(1, [200.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        // The stale predicted origin is still stored, so a near miss
        // measures against the original prediction.
        assert_eq!(history.acknowledge(1, [11.0, 0.0, 0.0]), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn classic_step_uses_height_delta() {
        let mut history = Q2PredictionHistory::new(Fixture, Q2PredictionProfile::Classic, Q2_PREDICTION_CAPACITY);
        history.remember(1, [0.0, 0.0, 0.0]);
        history.remember(2, [0.0, 0.0, 0.0]);
        history.remember(3, [0.0, 0.0, 10.0]);
        let result = history.replay(&baseline(), 0, 4, 1000.0, 0.016, None);
        let Q2PredictionResult::Predicted { step, step_time, .. } = result else {
            panic!("expected a prediction");
        };
        assert_eq!(step, 10.0);
        assert_eq!(step_time, 1000.0 - 0.016 * 500.0);
    }

    #[test]
    fn packet_history_evicts_old_packets() {
        let mut history = Q2CommandPacketHistory::new(2);
        history.sent(1, 10, 100.0);
        history.sent(2, 11, 101.0);
        history.sent(3, 12, 102.0);
        assert_eq!(history.acknowledged(1), None);
        assert_eq!(history.acknowledged(3), Some((12, 102.0)));
        history.clear();
        assert_eq!(history.acknowledged(3), None);
    }
}
