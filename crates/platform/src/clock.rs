//! All event time and developer performance counters use SDL3 in platform.
use qa_core::sys_events::EventTime;
use std::{
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// One cold wall-time origin for native calendar APIs. Hot calls advance it
/// only with the existing queued event time, without another clock intake.
pub(crate) fn event_epoch_millis() -> i64 {
    static EPOCH: OnceLock<i64> = OnceLock::new();
    *EPOCH.get_or_init(|| {
        let millis = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(delta) => delta.as_millis() as i128,
            Err(error) => -(error.duration().as_nanos().div_ceil(1_000_000) as i128),
        };
        millis.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    })
}
#[link(name = "SDL3")]
unsafe extern "C" {
    fn SDL_GetPerformanceCounter() -> u64;
    fn SDL_GetPerformanceFrequency() -> u64;
    fn SDL_DelayNS(duration: u64);
}
pub struct Stopwatch {
    start: u64,
    frequency: u64,
}
impl Stopwatch {
    pub fn start() -> Self {
        static FREQUENCY: OnceLock<u64> = OnceLock::new();
        Self {
            start: unsafe { SDL_GetPerformanceCounter() },
            frequency: *FREQUENCY.get_or_init(|| unsafe { SDL_GetPerformanceFrequency() }),
        }
    }
    pub fn elapsed(&self) -> Duration {
        Duration::from_nanos(self.elapsed_time().0)
    }
    fn elapsed_time(&self) -> EventTime {
        let ticks = unsafe { SDL_GetPerformanceCounter() }.saturating_sub(self.start);
        EventTime(
            (u128::from(ticks) * 1_000_000_000 / u128::from(self.frequency))
                .min(u128::from(u64::MAX)) as u64,
        )
    }
}
pub(crate) struct Clock(Stopwatch);
impl Clock {
    pub fn new() -> Self {
        let start = Stopwatch::start();
        event_epoch_millis();
        Self(start)
    }
    pub fn now(&self) -> EventTime {
        self.0.elapsed_time()
    }
}
pub fn pause(duration: Duration) {
    unsafe {
        SDL_DelayNS(duration.as_nanos().min(u128::from(u64::MAX)) as u64);
    }
}

#[cfg(test)]
#[path = "../tests/clock/monotonic.rs"]
mod tests;
