//! Display-rate frame pacer for the windowed drive loop.
//!
//! Donor provenance: `src/app/bootstrap/renderer.ts`
//! (`NativeRenderer.open` sets swap interval 1 so the real swap path
//! blocks on vsync where the driver honors it) and `src/app/bootstrap/startup.ts`
//! (`StartupApplication.run` sleeps the remainder of its frame budget
//! after each step). The swap path already paces where honored; this
//! pacer adds the sleep-based minimum frame time fallback so drivers
//! that ignore swap control (Mesa swrast under Xvfb) still pace at the
//! display rate instead of running flat-out. The sleep-after-step shape
//! adapts automatically: when the swap blocked for a full frame the
//! remainder is zero and no sleep happens.
//!
//! Dedicated and headless benchmark paths never construct this pacer;
//! they stay flat-out by design.

use std::thread;
use std::time::{Duration, Instant};

/// Refresh rate assumed when SDL reports none (0 or negative).
pub const DEFAULT_REFRESH_HZ: i32 = 60;

/// Slowest display rate the pacer tracks; slower reports clamp here.
pub const MIN_REFRESH_HZ: i32 = 30;

/// Fastest display rate the pacer tracks; faster reports clamp here.
pub const MAX_REFRESH_HZ: i32 = 240;

/// Minimum frame duration for a display refresh rate: one frame per
/// refresh, clamped to [`MIN_REFRESH_HZ`]..=[`MAX_REFRESH_HZ`], with
/// unknown rates (0 or negative) defaulting to [`DEFAULT_REFRESH_HZ`].
#[must_use]
pub fn min_frame_for_refresh(refresh_hz: i32) -> Duration {
    let hz = if refresh_hz <= 0 {
        DEFAULT_REFRESH_HZ
    } else {
        refresh_hz.clamp(MIN_REFRESH_HZ, MAX_REFRESH_HZ)
    };
    Duration::from_nanos(1_000_000_000 / u64::from(hz as u32))
}

/// Sleep-based minimum-frame-time pacer over [`Instant::now`].
pub struct WindowedPacer {
    min_frame: Duration,
    last: Instant,
}

impl WindowedPacer {
    /// Pacer for an explicit minimum frame duration.
    #[must_use]
    pub fn new(min_frame: Duration) -> Self {
        Self {
            min_frame,
            last: Instant::now(),
        }
    }

    /// Pacer for a display refresh rate via [`min_frame_for_refresh`].
    #[must_use]
    pub fn for_refresh_hz(refresh_hz: i32) -> Self {
        Self::new(min_frame_for_refresh(refresh_hz))
    }

    /// Minimum frame duration this pacer enforces.
    #[must_use]
    pub fn min_frame(&self) -> Duration {
        self.min_frame
    }

    /// Sleep so the frame ending now lasts at least `min_frame`.
    ///
    /// `elapsed` is the frame's measured duration; the return is the
    /// remainder slept (zero when the frame already ran long).
    #[must_use]
    pub fn sleep_for(min_frame: Duration, elapsed: Duration) -> Duration {
        min_frame.saturating_sub(elapsed)
    }

    /// End the current frame: sleep its remaining budget, if any, and
    /// re-anchor the clock for the next frame.
    pub fn end_frame(&mut self) {
        let rest = Self::sleep_for(self.min_frame, self.last.elapsed());
        if !rest.is_zero() {
            thread::sleep(rest);
        }
        self.last = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_maps_to_one_frame() {
        assert_eq!(min_frame_for_refresh(60), Duration::from_nanos(16_666_666));
        assert_eq!(min_frame_for_refresh(30), Duration::from_nanos(33_333_333));
        assert_eq!(min_frame_for_refresh(120), Duration::from_nanos(8_333_333));
    }

    #[test]
    fn unknown_refresh_defaults_to_sixty_hertz() {
        assert_eq!(min_frame_for_refresh(0), min_frame_for_refresh(60));
        assert_eq!(min_frame_for_refresh(-1), min_frame_for_refresh(60));
    }

    #[test]
    fn refresh_clamps_to_supported_range() {
        assert_eq!(min_frame_for_refresh(1000), min_frame_for_refresh(240));
        assert_eq!(min_frame_for_refresh(29), min_frame_for_refresh(30));
        assert_eq!(min_frame_for_refresh(240), Duration::from_nanos(4_166_666));
    }

    #[test]
    fn sleep_covers_fast_frame_remainder() {
        let min_frame = min_frame_for_refresh(60);
        assert_eq!(
            WindowedPacer::sleep_for(min_frame, Duration::from_millis(2)),
            min_frame - Duration::from_millis(2)
        );
    }

    #[test]
    fn slow_frames_sleep_nothing() {
        let min_frame = min_frame_for_refresh(60);
        assert_eq!(WindowedPacer::sleep_for(min_frame, min_frame), Duration::ZERO);
        assert_eq!(
            WindowedPacer::sleep_for(min_frame, min_frame + Duration::from_millis(5)),
            Duration::ZERO
        );
    }

    #[test]
    fn simulated_fast_frames_pace_to_display_rate() {
        let frames = 5u32;
        let mut pacer = WindowedPacer::for_refresh_hz(60);
        let started = Instant::now();
        for _ in 0..frames {
            thread::sleep(Duration::from_millis(1));
            pacer.end_frame();
        }
        let total = started.elapsed();
        let budget = pacer.min_frame() * frames;
        assert!(total >= budget, "paced {total:?} for a {budget:?} budget");
        assert!(
            total < budget + Duration::from_secs(2),
            "paced {total:?} for a {budget:?} budget"
        );
    }
}
