use qa_core::{events::*, primitives::*};

fn binding(id: u32) -> ValueBinding {
    ValueBinding {
        id: ValueId(id),
        width: ValueWidth::Signed32,
        reset: ValueReset::Life,
    }
}

#[test]
fn producer_values_retry_rejection_and_do_not_repeat_for_a_stalled_consumer() {
    let mut baseline = ValueBank::load(2);
    let mut current = ValueBank::load(2);
    let bindings = [binding(0), binding(1)];
    let mut ring = EventRing::load(4, 1, 1, 8).unwrap();
    let consumer = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    assert!(current.set(ValueId(0), NumericValue::integer(300)));
    assert!(current.set(ValueId(1), NumericValue::integer(-17)));
    let mut attempts = 0;
    baseline.publish_changes(&bindings, &current, |id, value| {
        attempts += 1;
        if id == ValueId(1) {
            return false;
        }
        ring.push_to(
            OutputTarget::Client(ClientId(0)),
            FrameEvent::PlayerValue { id, value },
        )
        .is_ok()
    });
    assert_eq!(attempts, 2);
    assert_eq!(baseline.get(ValueId(0)), current.get(ValueId(0)));
    assert_eq!(baseline.get(ValueId(1)), Some(NumericValue::default()));
    assert_eq!(ring.len(), 1);

    baseline.publish_changes(&bindings, &current, |id, value| {
        attempts += 1;
        assert_eq!(id, ValueId(1));
        ring.push_to(
            OutputTarget::Client(ClientId(0)),
            FrameEvent::PlayerValue { id, value },
        )
        .is_ok()
    });
    assert_eq!(attempts, 3);
    assert_eq!(ring.len(), 2);
    baseline.publish_changes(&bindings, &current, |_, _| {
        panic!("admitted values must not follow the stalled consumer baseline")
    });
    let mut batch = ring.batch(consumer).unwrap();
    for (id, value) in [(0, 300), (1, -17)] {
        let record = ring.next(&mut batch).unwrap();
        let FrameEvent::PlayerValue {
            id: actual_id,
            value: actual_value,
        } = record.event
        else {
            panic!("numeric value")
        };
        assert_eq!(actual_id, ValueId(id));
        assert_eq!(actual_value, NumericValue::integer(value));
        assert!(ring.submit(consumer, record.sequence, OutputSubmission::BestEffort));
    }
    assert_eq!(ring.len(), 0);
}

#[test]
fn value_publication_uses_only_selected_numeric_bindings_and_raw_bits() {
    let mut baseline = ValueBank::load(3);
    let mut current = ValueBank::load(3);
    let changed = NumericValue(0x8000_0000);
    assert!(current.set(ValueId(0), NumericValue::integer(51)));
    assert!(current.set(ValueId(2), changed));
    let mut emitted = Vec::new();
    baseline.publish_changes(&[binding(2), binding(9)], &current, |id, value| {
        emitted.push((id, value));
        true
    });
    assert_eq!(emitted, [(ValueId(2), changed)]);
    assert_eq!(baseline.get(ValueId(0)), Some(NumericValue::default()));
    assert_eq!(baseline.get(ValueId(2)), Some(changed));
    baseline.clear();
    baseline.publish_changes(&[binding(2)], &current, |id, value| {
        emitted.push((id, value));
        true
    });
    assert_eq!(emitted, [(ValueId(2), changed); 2]);
}
