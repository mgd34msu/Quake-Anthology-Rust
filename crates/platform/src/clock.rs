//! All event time and developer performance counters use SDL3 in platform.
use qa_core::sys_events::EventTime;
use std::{sync::OnceLock, time::Duration};
#[link(name = "SDL3")]
unsafe extern "C" {
    fn SDL_GetTicksNS() -> u64;
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
        let ticks = unsafe { SDL_GetPerformanceCounter() }.saturating_sub(self.start);
        Duration::from_nanos(
            (u128::from(ticks) * 1_000_000_000 / u128::from(self.frequency))
                .min(u128::from(u64::MAX)) as u64,
        )
    }
}
pub(crate) struct Clock(u64);
impl Clock {
    pub fn new() -> Self {
        Self(unsafe { SDL_GetTicksNS() })
    }
    pub fn now(&self) -> EventTime {
        EventTime(unsafe { SDL_GetTicksNS() }.saturating_sub(self.0))
    }
}
pub fn pause(duration: Duration) {
    unsafe {
        SDL_DelayNS(duration.as_nanos().min(u128::from(u64::MAX)) as u64);
    }
}
