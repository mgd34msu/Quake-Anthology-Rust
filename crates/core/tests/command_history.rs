use qa_core::{
    command_history::{CommandHistory, DEFAULT_CAPACITY, HistoryError},
    primitives::{UserCmd, Vec3, WeaponId},
};

fn command(index: i32) -> UserCmd {
    UserCmd {
        server_time_ms: index,
        view_angles: Vec3([index as f32, 20.0, -30.0]),
        movement: [123.0, -44.0, 17.0],
        weapon: Some(WeaponId(index as u32)),
        duration_ms: 8,
        duration_ns: 8_000_000,
        buttons: 0x1234,
        impulse: 7,
        light_level: 99,
    }
}

#[test]
fn default_retains_thousands_and_arbitrary_capacities_wrap_without_aliasing() {
    assert!(DEFAULT_CAPACITY > 64);
    for capacity in [1, 3, 64, 4096, 8193] {
        let mut history = CommandHistory::load(capacity, 0).unwrap();
        for number in 1..capacity + 129 {
            history.append(command(number as i32));
        }
        assert_eq!(history.len(), capacity);
        for age in 0..capacity {
            let number = history.number() - age as u64;
            let saved = history.read(number).unwrap().unwrap();
            assert_eq!(saved.server_time_ms, number as i32);
            assert_eq!(saved.view_angles.0[0], number as f32);
            assert_eq!(saved.movement, [123.0, -44.0, 17.0]);
            assert_eq!(saved.weapon, Some(WeaponId(number as u32)));
            assert_eq!(saved.duration_ns, 8_000_000);
        }
        assert!(
            history
                .read(history.number() - capacity as u64)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            history.read(history.number() + 1).unwrap_err(),
            HistoryError::Future
        );
    }
}

#[test]
fn reset_clears_retained_commands_and_can_preserve_or_replace_the_number() {
    let mut history = CommandHistory::load(7, 42).unwrap();
    history.append(command(43));
    history.reset(history.number());
    assert_eq!(history.number(), 43);
    assert!(history.is_empty() && history.get(0).is_none());
    assert_eq!(history.append(command(44)), 44);
    history.reset(0);
    assert_eq!(history.number(), 0);
    assert_eq!(history.append(command(1)), 1);
}

#[test]
fn full_number_wrap_keeps_non_power_of_two_slots_and_future_ordering_correct() {
    let mut history = CommandHistory::load(3, u64::MAX - 1).unwrap();
    assert_eq!(history.append(command(1)), u64::MAX);
    assert_eq!(history.append(command(2)), 0);
    assert_eq!(history.append(command(3)), 1);
    for (age, time) in [(0, 3), (1, 2), (2, 1)] {
        assert_eq!(history.get(age).unwrap().server_time_ms, time);
    }
    assert_eq!(history.read(u64::MAX).unwrap().unwrap().server_time_ms, 1);
    assert_eq!(history.read(2).unwrap_err(), HistoryError::Future);
}

#[test]
fn impossible_reservations_are_rejected_at_load() {
    assert!(matches!(
        CommandHistory::load(0, 0),
        Err(HistoryError::Capacity)
    ));
    assert!(matches!(
        CommandHistory::load(usize::MAX, 0),
        Err(HistoryError::Capacity)
    ));
}

#[test]
fn resizing_preserves_numbers_and_the_newest_records_without_restarting() {
    let mut history = CommandHistory::load(7, 100).unwrap();
    for time in 101..111 {
        history.append(command(time));
    }
    history.resize(3).unwrap();
    assert_eq!(
        (history.number(), history.capacity(), history.len()),
        (110, 3, 3)
    );
    assert_eq!(history.get(2).unwrap().server_time_ms, 108);
    history.resize(8193).unwrap();
    assert_eq!(
        (history.number(), history.capacity(), history.len()),
        (110, 8193, 3)
    );
    assert_eq!(history.get(0).unwrap().server_time_ms, 110);
    assert_eq!(history.append(command(111)), 111);
    assert!(history.resize(0).is_err());
    assert_eq!(history.get(0).unwrap().server_time_ms, 111);
}
