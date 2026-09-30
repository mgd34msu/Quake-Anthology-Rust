//! Project the shared-world interval into a source callback's clock.

use qa_core::time::{ClockProfile, FrameContext, SourceTime};

fn starts_interval(profile: &ClockProfile) -> bool {
    matches!(
        profile,
        ClockProfile::Q1Netquake { .. } | ClockProfile::Q1Quakeworld { .. }
    )
}

fn same_kind(world: &ClockProfile, provider: &ClockProfile) -> bool {
    matches!(
        (world, provider),
        (
            ClockProfile::Q1Netquake { .. },
            ClockProfile::Q1Netquake { .. }
        ) | (
            ClockProfile::Q1Quakeworld { .. },
            ClockProfile::Q1Quakeworld { .. }
        ) | (
            ClockProfile::Q2Classic { .. },
            ClockProfile::Q2Classic { .. }
        ) | (
            ClockProfile::Q2Rerelease { .. },
            ClockProfile::Q2Rerelease { .. }
        ) | (ClockProfile::Q3 { .. }, ClockProfile::Q3 { .. })
    )
}

fn uses_milliseconds(profile: &ClockProfile) -> bool {
    matches!(
        profile,
        ClockProfile::Q2Rerelease { .. } | ClockProfile::Q3 { .. }
    )
}

fn as_milliseconds(time: &SourceTime) -> f64 {
    match time {
        SourceTime::Seconds(value) => f64::from(*value) * 1000.0,
        SourceTime::Milliseconds(value) => f64::from(*value),
    }
}

/// Project the active shared-world interval into a source callback's clock.
#[must_use]
pub fn provider_frame(
    frame: FrameContext,
    world: &ClockProfile,
    provider: &ClockProfile,
) -> FrameContext {
    if same_kind(world, provider) {
        return frame;
    }
    let milliseconds = as_milliseconds(&frame.time);
    let elapsed_milliseconds = as_milliseconds(&frame.elapsed);
    let start = if starts_interval(world) {
        milliseconds
    } else {
        milliseconds - elapsed_milliseconds
    };
    let time = if starts_interval(provider) {
        start
    } else {
        start + elapsed_milliseconds
    };
    #[allow(clippy::cast_possible_truncation)]
    let (time, elapsed) = if uses_milliseconds(provider) {
        (
            SourceTime::Milliseconds(time.round() as i32),
            SourceTime::Milliseconds(elapsed_milliseconds.round() as i32),
        )
    } else {
        (
            SourceTime::Seconds((time / 1000.0) as f32),
            SourceTime::Seconds((elapsed_milliseconds / 1000.0) as f32),
        )
    };
    FrameContext { time, elapsed, ..frame }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::time::FramePhase;

    fn netquake() -> ClockProfile {
        ClockProfile::Q1Netquake {
            minimum_frame_seconds: 0.001,
            maximum_frame_seconds: 0.1,
            fixed_frame_seconds: None,
        }
    }

    fn quakeworld() -> ClockProfile {
        ClockProfile::Q1Quakeworld {
            maximum_command_milliseconds: 200.0,
        }
    }

    fn frame() -> FrameContext {
        FrameContext {
            frame: 7,
            time: SourceTime::Seconds(10.0),
            elapsed: SourceTime::Seconds(0.05),
            phase: FramePhase::FrameEntry,
        }
    }

    #[test]
    fn same_kind_returns_frame_unchanged() {
        let out = provider_frame(frame(), &netquake(), &netquake());
        assert_eq!(out.frame, 7);
        assert_eq!(out.time, SourceTime::Seconds(10.0));
        assert_eq!(out.elapsed, SourceTime::Seconds(0.05));
    }

    #[test]
    fn interval_start_clocks_preserve_time() {
        let out = provider_frame(frame(), &netquake(), &quakeworld());
        assert_eq!(out.time, SourceTime::Seconds(10.0));
        assert_eq!(out.elapsed, SourceTime::Seconds(0.05));
    }

    #[test]
    fn quakeworld_to_netquake_preserves_values() {
        let out = provider_frame(frame(), &quakeworld(), &netquake());
        assert_eq!(out.time, SourceTime::Seconds(10.0));
        assert_eq!(out.elapsed, SourceTime::Seconds(0.05));
    }
}
