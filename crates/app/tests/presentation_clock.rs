use qa_app::snapshots::{ClockError, ClockOptions, PresentationClock, SnapshotClockSample};
use qa_core::{
    primitives::{RuleSetId, ThinkTime},
    sys_events::EventTime,
};

fn clock() -> PresentationClock {
    PresentationClock::load(qa_gameplay::rules::presentation::clock_rules(
        RuleSetId::Quake3,
    ))
}
fn snapshot(sequence: u32, ms: i64) -> SnapshotClockSample {
    SnapshotClockSample {
        sequence,
        time: ThinkTime::Milliseconds(ms),
        flags: 0,
    }
}
fn advance(
    clock: &mut PresentationClock,
    queued_ms: u64,
    current: SnapshotClockSample,
    options: ClockOptions,
) -> Result<i64, ClockError> {
    clock.advance(
        EventTime::from_milliseconds(queued_ms),
        current,
        None,
        None,
        options,
    )?;
    Ok(clock.sample().time.milliseconds())
}

#[test]
fn first_active_snapshot_starts_at_its_native_time() {
    let mut clock = clock();
    let mut inactive = snapshot(1, 1000);
    inactive.flags = 2;
    advance(&mut clock, 10_000, inactive, ClockOptions::default()).unwrap();
    advance(&mut clock, 10_025, inactive, ClockOptions::default()).unwrap();
    assert!(!clock.is_ready());
    assert_eq!(
        advance(
            &mut clock,
            20_000,
            snapshot(2, 1100),
            ClockOptions::default()
        ),
        Ok(1100)
    );
    assert!(clock.is_ready());
}

#[test]
fn packetless_queued_time_advances_beyond_the_latest_snapshot() {
    let mut clock = clock();
    let current = snapshot(1, 1000);
    for (queued, time) in [
        (10_000, 1000),
        (10_025, 1025),
        (10_200, 1200),
        (10_600, 1600),
    ] {
        assert_eq!(
            advance(&mut clock, queued, current, ClockOptions::default()),
            Ok(time)
        );
        assert_eq!(clock.sample().fraction, 1.0);
    }
}

#[test]
fn time_nudge_matches_the_native_typescript_fixture_without_mutating_settings() {
    let mut clock = clock();
    for (queued, nudge, time) in [
        (0, 0, 1000),
        (50, -15, 1065),
        (60, 30, 1065),
        (100, 999, 1070),
        (150, -999, 1180),
    ] {
        let options = ClockOptions {
            time_nudge_ms: nudge,
            ..Default::default()
        };
        assert_eq!(
            advance(&mut clock, queued, snapshot(1, 1000), options),
            Ok(time)
        );
        assert_eq!(options.time_nudge_ms, nudge);
    }
    let options = ClockOptions {
        time_nudge_ms: -999,
        ..Default::default()
    };
    assert_eq!(
        advance(&mut clock, 200, snapshot(2, 1200), options),
        Ok(1230)
    );
    // CL_AdjustTimeDelta runs after this frame's output. Its -2-ms drift
    // appears only on the following frame and never repeats without a packet.
    assert_eq!(
        advance(&mut clock, 250, snapshot(2, 1200), options),
        Ok(1278)
    );
    assert_eq!(
        advance(&mut clock, 300, snapshot(2, 1200), options),
        Ok(1328)
    );
}

#[test]
fn snapshot_difference_thresholds_use_native_strict_comparisons() {
    for (difference, adjusted_frame, next_frame) in [
        (100, 1100, 1198),
        (101, 1100, 1250),
        (500, 1100, 1450),
        (501, 1601, 1701),
    ] {
        let mut clock = clock();
        advance(&mut clock, 0, snapshot(1, 1000), ClockOptions::default()).unwrap();
        let current = snapshot(2, 1100 + difference);
        assert_eq!(
            advance(&mut clock, 100, current, ClockOptions::default()),
            Ok(adjusted_frame),
            "difference {difference}"
        );
        assert_eq!(
            advance(&mut clock, 200, current, ClockOptions::default()),
            Ok(next_frame),
            "difference {difference}"
        );
    }
}

#[test]
fn fast_correction_uses_signed_shift_and_applies_after_output() {
    let mut clock = clock();
    advance(
        &mut clock,
        10_000,
        snapshot(1, 1000),
        ClockOptions::default(),
    )
    .unwrap();
    assert_eq!(
        advance(
            &mut clock,
            10_099,
            snapshot(2, 1200),
            ClockOptions::default()
        ),
        Ok(1099)
    );
    // (-9000 + -8899) >> 1 = -8950, not truncation toward zero.
    assert_eq!(
        advance(
            &mut clock,
            10_200,
            snapshot(2, 1200),
            ClockOptions::default()
        ),
        Ok(1250)
    );
}

#[test]
fn slow_drift_happens_once_per_sequence_and_uses_the_extrapolation_marker() {
    let mut clock = clock();
    advance(&mut clock, 0, snapshot(1, 1000), ClockOptions::default()).unwrap();
    assert_eq!(
        advance(&mut clock, 100, snapshot(2, 1100), ClockOptions::default()),
        Ok(1100)
    );
    // The prior slow adjustment cleared extrapolation. This frame stays well
    // behind its new snapshot, so the next adjustment is +1 rather than -2.
    assert_eq!(
        advance(&mut clock, 101, snapshot(3, 1150), ClockOptions::default()),
        Ok(1100)
    );
    assert_eq!(
        advance(&mut clock, 102, snapshot(3, 1150), ClockOptions::default()),
        Ok(1101)
    );
    assert_eq!(
        advance(&mut clock, 105, snapshot(3, 1150), ClockOptions::default()),
        Ok(1104)
    );
}

#[test]
fn extrapolation_uses_unnudged_time() {
    let mut clock = clock();
    advance(&mut clock, 0, snapshot(1, 1000), ClockOptions::default()).unwrap();
    // Consume the initial extrapolation marker first.
    advance(&mut clock, 1, snapshot(2, 1100), ClockOptions::default()).unwrap();
    let options = ClockOptions {
        time_nudge_ms: -30,
        ..Default::default()
    };
    // Unnudged time 1098 is below 1110-5 even though displayed time is 1128.
    assert_eq!(
        advance(&mut clock, 100, snapshot(3, 1110), options),
        Ok(1128)
    );
    assert_eq!(
        advance(&mut clock, 120, snapshot(3, 1110), options),
        Ok(1149)
    );
}

#[test]
fn nonunit_timescale_only_disables_slow_adjustments_in_this_default_scale_clock() {
    for timescale in [0.0, 1.0, 0.5, 2.0] {
        let mut clock = clock();
        let options = ClockOptions {
            timescale,
            ..Default::default()
        };
        advance(&mut clock, 0, snapshot(1, 1000), options).unwrap();
        advance(&mut clock, 100, snapshot(2, 1100), options).unwrap();
        assert_eq!(
            advance(&mut clock, 200, snapshot(2, 1100), options),
            Ok(if timescale == 0.0 || timescale == 1.0 {
                1198
            } else {
                1200
            })
        );
    }
    for (server_time, expected) in [(1300, 1300), (1700, 1800)] {
        let mut clock = clock();
        let options = ClockOptions {
            timescale: 2.0,
            ..Default::default()
        };
        advance(&mut clock, 0, snapshot(1, 1000), options).unwrap();
        advance(&mut clock, 100, snapshot(2, server_time), options).unwrap();
        assert_eq!(
            advance(&mut clock, 200, snapshot(2, server_time), options),
            Ok(expected)
        );
    }
}

#[test]
fn pause_preserves_sample_and_leaves_new_snapshot_adjustment_pending() {
    let mut clock = clock();
    advance(&mut clock, 0, snapshot(1, 1000), ClockOptions::default()).unwrap();
    let options = ClockOptions {
        paused: true,
        ..Default::default()
    };
    assert_eq!(
        advance(&mut clock, 100, snapshot(2, 1200), options),
        Ok(1000)
    );
    assert_eq!(
        advance(&mut clock, 200, snapshot(2, 1200), ClockOptions::default()),
        Ok(1200)
    );
    assert_eq!(
        advance(&mut clock, 300, snapshot(2, 1200), ClockOptions::default()),
        Ok(1298)
    );
}

#[test]
fn hard_reset_can_rebase_after_loss_and_gamestate_reset_clears_activation() {
    let mut clock = clock();
    advance(&mut clock, 0, snapshot(1, 1000), ClockOptions::default()).unwrap();
    advance(
        &mut clock,
        10_000,
        snapshot(1, 1000),
        ClockOptions::default(),
    )
    .unwrap();
    assert_eq!(
        advance(
            &mut clock,
            10_001,
            snapshot(2, 1100),
            ClockOptions::default()
        ),
        Ok(1100)
    );
    assert_eq!(
        advance(
            &mut clock,
            10_011,
            snapshot(2, 1100),
            ClockOptions::default()
        ),
        Ok(1110)
    );
    clock.reset();
    assert!(!clock.is_ready());
    assert_eq!(
        advance(
            &mut clock,
            20_000,
            snapshot(1, 500),
            ClockOptions::default()
        ),
        Ok(500)
    );
}

#[test]
fn reverse_server_time_is_scoped_and_does_not_rebase_silently() {
    let mut affected = clock();
    let mut healthy = clock();
    advance(&mut affected, 0, snapshot(1, 1000), ClockOptions::default()).unwrap();
    advance(&mut healthy, 0, snapshot(1, 2000), ClockOptions::default()).unwrap();
    assert_eq!(
        advance(&mut affected, 10, snapshot(2, 999), ClockOptions::default()),
        Err(ClockError::TimeRegression)
    );
    assert_eq!(
        advance(&mut healthy, 10, snapshot(1, 2000), ClockOptions::default()),
        Ok(2010)
    );
}

#[test]
fn synchronized_clock_rejects_a_seconds_domain_without_becoming_ready() {
    let mut clock = clock();
    assert_eq!(
        advance(
            &mut clock,
            0,
            SnapshotClockSample {
                time: ThinkTime::Seconds(1.0),
                ..snapshot(1, 1000)
            },
            ClockOptions::default()
        ),
        Err(ClockError::TimeDomain)
    );
    assert!(!clock.is_ready());
}

#[test]
fn interpolation_fraction_remains_bounded_while_native_time_extrapolates() {
    let mut clock = clock();
    for queued in [1000, 1025, 1100] {
        clock
            .advance(
                EventTime::from_milliseconds(queued),
                snapshot(1, 1000),
                Some(ThinkTime::Milliseconds(950)),
                None,
                ClockOptions::default(),
            )
            .unwrap();
        assert_eq!(clock.sample().time, ThinkTime::Milliseconds(queued as i64));
        assert_eq!(clock.sample().fraction, 1.0);
        assert_eq!(clock.sample().frame_ms, 50);
    }
}

#[test]
fn quakeworld_animation_clock_uses_queued_realtime_and_native_packet_latency() {
    let mut clock = PresentationClock::load(qa_gameplay::rules::presentation::clock_rules(
        RuleSetId::QuakeWorld,
    ));
    let zero_wire_time = SnapshotClockSample {
        sequence: 1,
        time: ThinkTime::Milliseconds(0),
        flags: 0,
    };
    let queued = EventTime(12_345_678_901);
    clock
        .advance(queued, zero_wire_time, None, None, ClockOptions::default())
        .unwrap();
    assert_eq!(clock.sample().time, ThinkTime::Seconds(queued.seconds()));
    assert_eq!(clock.sample().fraction, 1.0);
    // Native cls.latency starts at zero and drifts up by a double .001,
    // stored back in float after each accepted packet, even without a snapshot.
    clock.observe_packet_timing(EventTime(10_125_000_000), Some(EventTime(10_000_000_000)));
    clock
        .advance(queued, zero_wire_time, None, None, ClockOptions::default())
        .unwrap();
    assert_eq!(
        clock.sample().time,
        ThinkTime::Seconds(queued.seconds() - f64::from(0.001f32))
    );
    clock.observe_packet_timing(EventTime(10_250_000_000), Some(EventTime(10_000_000_000)));
    clock
        .advance(queued, zero_wire_time, None, None, ClockOptions::default())
        .unwrap();
    let drifted = (f64::from(0.001f32) + 0.001) as f32;
    assert_eq!(
        clock.sample().time,
        ThinkTime::Seconds(queued.seconds() - f64::from(drifted))
    );
    clock.observe_packet_timing(EventTime(10_000_500_000), Some(EventTime(10_000_000_000)));
    clock
        .advance(queued, zero_wire_time, None, None, ClockOptions::default())
        .unwrap();
    let native_sample = (10.0005f64 - 10.0) as f32;
    let expected = ThinkTime::Seconds(queued.seconds() - f64::from(native_sample));
    assert_eq!(clock.sample().time, expected);
    // Missing/negative/over-one-second observations do not change the estimate.
    for (received, sent) in [
        (20_000_000_000, None),
        (9_000_000_000, Some(EventTime(10_000_000_000))),
        (12_000_000_000, Some(EventTime(10_000_000_000))),
    ] {
        clock.observe_packet_timing(EventTime(received), sent);
    }
    clock
        .advance(queued, zero_wire_time, None, None, ClockOptions::default())
        .unwrap();
    assert_eq!(clock.sample().time, expected);
    // Native negative push latency moves toward realtime, never beyond it.
    clock
        .advance(
            queued,
            zero_wire_time,
            None,
            None,
            ClockOptions {
                push_latency_ms: -500.0,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(clock.sample().time, ThinkTime::Seconds(queued.seconds()));
    let paused = clock.sample();
    clock
        .advance(
            EventTime(30_000_000_000),
            zero_wire_time,
            None,
            None,
            ClockOptions {
                paused: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(clock.sample(), paused);
    clock
        .advance(
            EventTime(30_000_000_000),
            zero_wire_time,
            None,
            None,
            ClockOptions::default(),
        )
        .unwrap();
    assert_eq!(
        clock.sample().time,
        ThinkTime::Seconds(30.0 - f64::from(native_sample))
    );
    // A new server header clears cl presentation state, not cls.latency.
    clock.reset();
    clock
        .advance(queued, zero_wire_time, None, None, ClockOptions::default())
        .unwrap();
    assert_eq!(clock.sample().time, expected);
}
