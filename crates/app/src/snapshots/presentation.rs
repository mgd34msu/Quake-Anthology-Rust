//! CLIENT temporal presentation; native words remain in the connection ring.
use qa_core::{primitives::ThinkTime, sys_events::EventTime};
use qa_gameplay::rules::presentation::{ClockRules, ClockSynchronization};

/// Borrowed snapshot metadata, without a second native snapshot store.
#[derive(Clone, Copy, Debug)]
pub struct SnapshotClockSample {
    pub sequence: u32,
    pub time: ThinkTime,
    pub flags: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct ClockOptions {
    pub time_nudge_ms: i32,
    /// The native slow-drift guard, not a replacement for common frame scaling.
    /// This slice consumes queued realtime at the default frame scale.
    pub timescale: f32,
    pub paused: bool,
}
impl Default for ClockOptions {
    fn default() -> Self {
        Self {
            time_nudge_ms: 0,
            timescale: 1.0,
            paused: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockError {
    TimeDomain,
    TimeRange,
    TimeRegression,
}

#[derive(Default)]
struct Synchronization {
    delta_ms: i64,
    old_time_ms: i64,
    old_frame_ms: i64,
    sequence: Option<u32>,
    active: bool,
    extrapolated: bool,
}
impl Synchronization {
    fn advance(
        &mut self,
        queued: EventTime,
        snapshot: SnapshotClockSample,
        options: ClockOptions,
        rules: ClockSynchronization,
    ) -> Result<Option<ThinkTime>, ClockError> {
        let ThinkTime::Milliseconds(current) = snapshot.time else {
            return Err(ClockError::TimeDomain);
        };
        let realtime = i64::try_from(queued.milliseconds()).map_err(|_| ClockError::TimeRange)?;
        if !self.active {
            if snapshot.flags & rules.inactive_snapshot_mask != 0 {
                return Ok(None);
            }
            self.delta_ms = current.checked_sub(realtime).ok_or(ClockError::TimeRange)?;
            self.old_time_ms = current;
            self.sequence = Some(snapshot.sequence);
            self.active = true;
        }
        // As in CL_SetCGameTime, pausing neither consumes a pending snapshot
        // adjustment nor updates the previous accepted server-frame time.
        if options.paused {
            return Ok(None);
        }
        if current < self.old_frame_ms {
            return Err(ClockError::TimeRegression);
        }
        self.old_frame_ms = current;
        let unnudged = realtime
            .checked_add(self.delta_ms)
            .ok_or(ClockError::TimeRange)?;
        let nudge = options
            .time_nudge_ms
            .clamp(-rules.nudge_limit_ms, rules.nudge_limit_ms);
        let mut time = unnudged
            .checked_sub(i64::from(nudge))
            .ok_or(ClockError::TimeRange)?
            .max(self.old_time_ms);
        self.old_time_ms = time;
        if unnudged >= current.saturating_sub(rules.extrapolation_margin_ms) {
            self.extrapolated = true;
        }
        if self.sequence != Some(snapshot.sequence) {
            self.sequence = Some(snapshot.sequence);
            let delta = current.checked_sub(realtime).ok_or(ClockError::TimeRange)?;
            let difference = delta.abs_diff(self.delta_ms);
            if difference > rules.reset_difference_ms {
                self.delta_ms = delta;
                self.old_time_ms = current;
                time = current;
            } else if difference > rules.fast_difference_ms {
                self.delta_ms = self
                    .delta_ms
                    .checked_add(delta)
                    .ok_or(ClockError::TimeRange)?
                    >> 1;
            } else if options.timescale == 0.0 || options.timescale == 1.0 {
                let adjustment = if self.extrapolated {
                    self.extrapolated = false;
                    rules.extrapolated_adjustment_ms
                } else {
                    rules.interpolated_adjustment_ms
                };
                self.delta_ms = self
                    .delta_ms
                    .checked_add(adjustment)
                    .ok_or(ClockError::TimeRange)?;
            }
        }
        Ok(Some(ThinkTime::Milliseconds(time)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interpolation {
    pub time: ThinkTime,
    pub fraction: f32,
    pub frame_ms: u16,
}
impl Default for Interpolation {
    fn default() -> Self {
        Self {
            time: ThinkTime::Milliseconds(0),
            fraction: 1.0,
            frame_ms: 0,
        }
    }
}
impl Interpolation {
    pub fn gun_back_lerp(self, changed_at: ThinkTime, rate: i32) -> f32 {
        let rate = if rate == 0 { 10 } else { rate };
        if rate <= 0 {
            return 0.0;
        }
        let gun_ms = 1.0 / rate as f32 * 1000.0;
        (1.0 - (self.time.milliseconds() as f32
            - (changed_at.milliseconds() as f32 - f32::from(self.frame_ms)))
            / gun_ms)
            .clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GunAnimation {
    pub changed_at: ThinkTime,
    pub previous: Option<u32>,
}

#[derive(Default)]
pub struct PresentationClock {
    rules: ClockRules,
    queued_at: Option<EventTime>,
    time: Option<ThinkTime>,
    sample: Interpolation,
    synchronization: Synchronization,
}
impl PresentationClock {
    pub fn load(rules: ClockRules) -> Self {
        Self {
            rules,
            ..Self::default()
        }
    }
    pub fn sample(&self) -> Interpolation {
        self.sample
    }
    pub fn is_ready(&self) -> bool {
        self.rules.synchronization.is_none() || self.time.is_some()
    }
    pub fn reset(&mut self) {
        self.queued_at = None;
        self.time = None;
        self.sample = Interpolation::default();
        self.synchronization = Synchronization::default();
    }
    pub fn advance(
        &mut self,
        queued: EventTime,
        snapshot: SnapshotClockSample,
        previous: Option<ThinkTime>,
        native_period: Option<u16>,
        options: ClockOptions,
    ) -> Result<(), ClockError> {
        let current = snapshot.time;
        // QW snapshots have no native timestamp. Their zero-length interval
        // stays at fraction one rather than borrowing renderer uptime.
        let elapsed = self
            .queued_at
            .map_or(EventTime::default(), |old| EventTime(queued.since(old)));
        let elapsed_ms = self.queued_at.map_or(0, |old| {
            queued.milliseconds().saturating_sub(old.milliseconds())
        });
        self.queued_at = Some(queued);
        let mut time = if let Some(rules) = self.rules.synchronization {
            let Some(time) = self
                .synchronization
                .advance(queued, snapshot, options, rules)?
            else {
                return Ok(());
            };
            time
        } else {
            match (current, self.time) {
                (ThinkTime::Milliseconds(_), Some(ThinkTime::Milliseconds(time))) => {
                    ThinkTime::Milliseconds(time.saturating_add(elapsed_ms as i64))
                }
                (ThinkTime::Seconds(_), Some(ThinkTime::Seconds(time))) => {
                    ThinkTime::Seconds(time + elapsed.seconds())
                }
                (ThinkTime::Milliseconds(_), _) => ThinkTime::Milliseconds(0),
                (ThinkTime::Seconds(_), _) => ThinkTime::Seconds(0.0),
            }
        };
        // The interpolation fraction still selects the available pair. Native
        // synchronized trajectory/cgame time may extend beyond that pair.
        let synchronized_time = self.rules.synchronization.map(|_| time);
        let span = previous.map_or(0.0, |old| current.seconds() - old.seconds());
        let period = native_period.filter(|&ms| ms != 0);
        let lower = match (current, period) {
            (ThinkTime::Milliseconds(now), Some(period)) => {
                ThinkTime::Milliseconds(now.saturating_sub(i64::from(period)))
            }
            _ => previous
                .filter(|_| span > 0.0 && span.is_finite())
                .unwrap_or(current),
        };
        let fraction = match (time, lower, current) {
            (
                ThinkTime::Milliseconds(value),
                ThinkTime::Milliseconds(low),
                ThinkTime::Milliseconds(high),
            ) => {
                let value = value.clamp(low, high.max(low));
                time = ThinkTime::Milliseconds(value);
                if high == low {
                    1.0
                } else if let Some(scale) = self.rules.remaining_ms_scale {
                    1.0 - (high - value) as f32 * scale
                } else {
                    (value - low) as f32 * (1.0 / (high - low) as f32)
                }
            }
            _ => {
                let low = lower.seconds();
                let high = current.seconds();
                let value = if low.is_finite() && high.is_finite() {
                    time.seconds().clamp(low, high.max(low))
                } else {
                    high
                };
                time = ThinkTime::Seconds(value);
                if high > low && (high - low).is_finite() {
                    ((value - low) / (high - low)) as f32
                } else {
                    1.0
                }
            }
        };
        let time = synchronized_time.unwrap_or(time);
        self.time = Some(time);
        self.sample = Interpolation {
            time,
            fraction: if previous.is_none() { 1.0 } else { fraction },
            frame_ms: period
                .unwrap_or_else(|| (span.max(0.0) * 1000.0).min(u16::MAX as f64) as u16),
        };
        Ok(())
    }
}
