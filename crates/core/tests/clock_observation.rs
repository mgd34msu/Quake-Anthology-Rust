use qa_core::{
    primitives::{ClockObservation, PlayerState, ThinkTime},
    sys_events::EventTime,
};

#[test]
fn native_seconds_map_to_the_supplied_queued_epoch_and_keep_past_deadlines() {
    let clock = ClockObservation {
        native: ThinkTime::Seconds(100.0),
        queued: EventTime::from_milliseconds(1250),
    };
    assert!(clock.is_valid());
    for (deadline, queued) in [(100.5, 1.75), (100.0, 1.25), (98.0, -0.75)] {
        assert_eq!(clock.deadline(ThinkTime::Seconds(deadline)), Some(queued));
    }
}

#[test]
fn integer_deadlines_subtract_before_float_conversion_at_both_native_limits() {
    for (observed, deadline, seconds) in [
        (i64::MAX, i64::MAX - 1, -0.001f64),
        (i64::MIN, i64::MIN + 2, 0.002),
        (-1, -2, -0.001),
        (1, -1, -0.002),
    ] {
        let clock = ClockObservation {
            native: ThinkTime::Milliseconds(observed),
            queued: EventTime::default(),
        };
        assert_eq!(
            clock
                .deadline(ThinkTime::Milliseconds(deadline))
                .unwrap()
                .to_bits(),
            seconds.to_bits()
        );
    }
    let clock = ClockObservation {
        native: ThinkTime::Milliseconds(i64::MIN),
        queued: EventTime::from_milliseconds(5000),
    };
    let far_future = clock.deadline(ThinkTime::Milliseconds(i64::MAX)).unwrap();
    assert!(far_future.is_finite() && far_future > 1.8e16);
    let reverse = ClockObservation {
        native: ThinkTime::Milliseconds(i64::MAX),
        ..clock
    };
    let far_past = reverse.deadline(ThinkTime::Milliseconds(i64::MIN)).unwrap();
    assert!(far_past.is_finite() && far_past < -1.8e16);
}

#[test]
fn clock_tags_and_nonfinite_seconds_reject_without_coercion() {
    let seconds = ClockObservation {
        native: ThinkTime::Seconds(1.0),
        queued: EventTime::default(),
    };
    let milliseconds = ClockObservation {
        native: ThinkTime::Milliseconds(1000),
        ..seconds
    };
    assert_eq!(seconds.deadline(ThinkTime::Milliseconds(1000)), None);
    assert_eq!(milliseconds.deadline(ThinkTime::Seconds(1.0)), None);
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let clock = ClockObservation {
            native: ThinkTime::Seconds(invalid),
            ..seconds
        };
        assert!(!clock.is_valid());
        assert_eq!(clock.deadline(ThinkTime::Seconds(1.0)), None);
        assert_eq!(seconds.deadline(ThinkTime::Seconds(invalid)), None);
    }
    let overflow = ClockObservation {
        native: ThinkTime::Seconds(-f64::MAX),
        ..seconds
    };
    assert_eq!(overflow.deadline(ThinkTime::Seconds(f64::MAX)), None);
}

#[test]
fn advancing_both_clocks_preserves_identical_absolute_deadline_bits() {
    for elapsed in [0, 1, 2, 7, 18, 19, 2000, 1_000_000] {
        let clock = ClockObservation {
            native: ThinkTime::Milliseconds(1800 + elapsed),
            queued: EventTime::from_milliseconds((500 + elapsed) as u64),
        };
        assert_eq!(
            clock
                .deadline(ThinkTime::Milliseconds(1819))
                .unwrap()
                .to_bits(),
            0.519f64.to_bits()
        );
        assert_eq!(
            clock
                .deadline(ThinkTime::Milliseconds(1000))
                .unwrap()
                .to_bits(),
            (-0.3f64).to_bits()
        );
    }
}

#[test]
fn player_reset_discards_the_native_sample_and_preserves_inventory_storage() {
    let mut player = PlayerState::with_capacity(2, 3, 4);
    assert_eq!(player.native_clock, None);
    let item_storage = player.inventory.item_counts.as_ptr();
    let timer_storage = player.inventory.powerup_until.as_ptr();
    player.native_clock = Some(ClockObservation {
        native: ThinkTime::Milliseconds(5000),
        queued: EventTime::from_milliseconds(137),
    });
    player.movement.command_time_ms = 300;
    player.inventory.powerup_until[1] = 7.137;
    player.reset();
    assert_eq!(player.native_clock, None);
    assert_eq!(player.inventory.item_counts.as_ptr(), item_storage);
    assert_eq!(player.inventory.powerup_until.as_ptr(), timer_storage);
    assert!(
        player
            .inventory
            .powerup_until
            .iter()
            .all(|timer| *timer == 0.0)
    );
}
