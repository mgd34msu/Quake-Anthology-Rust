//! OS clock reads are confined to this capability, including developer timers.
use qa_core::sys_events::EventTime;
use std::time::{Duration, Instant};

pub struct Stopwatch(Instant);
impl Stopwatch {
    pub fn start() -> Self {
        Self(Instant::now())
    }
    pub fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}
pub(crate) struct Clock(Stopwatch);
impl Clock {
    pub fn new() -> Self {
        Self(Stopwatch::start())
    }
    pub fn now(&self) -> EventTime {
        EventTime(self.0.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64)
    }
}
pub fn pause(duration: Duration) {
    std::thread::sleep(duration);
}
