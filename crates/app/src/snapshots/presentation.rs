//! CLIENT temporal presentation; native words remain in the connection ring.
use qa_core::{primitives::ThinkTime, sys_events::EventTime};
use qa_gameplay::rules::presentation::ClockRules;

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
    pub(crate) fn reset(&mut self) {
        self.queued_at = None;
        self.time = None;
        self.sample = Interpolation::default();
    }
    pub fn advance(
        &mut self,
        queued: EventTime,
        current: ThinkTime,
        previous: Option<ThinkTime>,
        native_period: Option<u16>,
    ) {
        // QW snapshots have no native timestamp. Their zero-length interval
        // stays at fraction one rather than borrowing renderer uptime.
        let elapsed = self
            .queued_at
            .map_or(EventTime::default(), |old| EventTime(queued.since(old)));
        let elapsed_ms = self.queued_at.map_or(0, |old| {
            queued.milliseconds().saturating_sub(old.milliseconds())
        });
        self.queued_at = Some(queued);
        let mut time = match (current, self.time) {
            (ThinkTime::Milliseconds(_), Some(ThinkTime::Milliseconds(time))) => {
                ThinkTime::Milliseconds(time.saturating_add(elapsed_ms as i64))
            }
            (ThinkTime::Seconds(_), Some(ThinkTime::Seconds(time))) => {
                ThinkTime::Seconds(time + elapsed.seconds())
            }
            (ThinkTime::Milliseconds(_), _) => ThinkTime::Milliseconds(0),
            (ThinkTime::Seconds(_), _) => ThinkTime::Seconds(0.0),
        };
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
        self.time = Some(time);
        self.sample = Interpolation {
            time,
            fraction: if previous.is_none() { 1.0 } else { fraction },
            frame_ms: period
                .unwrap_or_else(|| (span.max(0.0) * 1000.0).min(u16::MAX as f64) as u16),
        };
    }
}
