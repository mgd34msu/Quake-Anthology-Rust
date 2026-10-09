use qa_core::{events::*, primitives::*};
fn sound(id: u32) -> FrameEvent {
    FrameEvent::Sound(SoundEvent {
        sound: SoundId(id),
        entity: None,
        channel: 1,
        position: Vec3::default(),
        volume: 1.0,
        attenuation: 1.0,
        action: SoundAction::Play,
    })
}
fn consume(ring: &mut EventRing, id: OutputConsumerId, submission: OutputSubmission) -> Vec<u64> {
    let mut sequences = Vec::new();
    if let Some(mut batch) = ring.batch(id) {
        while let Some(record) = ring.next(&mut batch) {
            sequences.push(record.sequence);
            assert!(ring.submit(id, record.sequence, submission));
        }
    }
    sequences
}
#[test]
fn submission_is_independent_of_reading_and_actual_reliable_ack() {
    let mut ring = EventRing::load(8, 3, 8, 32).unwrap();
    let reliable = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let datagram = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let unsent = ring.bind(OutputTarget::Module(ModuleId(2))).unwrap();
    ring.push(sound(1)).unwrap();
    assert_eq!(
        consume(
            &mut ring,
            reliable,
            OutputSubmission::Reliable(NativeReceipt(7))
        ),
        [0]
    );
    assert!(consume(&mut ring, reliable, OutputSubmission::Unsent).is_empty());
    assert_eq!(
        consume(&mut ring, datagram, OutputSubmission::BestEffort),
        [0]
    );
    assert_eq!(consume(&mut ring, unsent, OutputSubmission::Unsent), [0]);
    assert_eq!(ring.acknowledge(reliable, NativeReceipt(6)), 0);
    assert_eq!(ring.acknowledge(reliable, NativeReceipt(7)), 1);
    assert_eq!(ring.acknowledge(reliable, NativeReceipt(7)), 0);
    assert_eq!(ring.len(), 1); // Unsatisfied module still owns the slot.
    assert_eq!(
        consume(&mut ring, unsent, OutputSubmission::BestEffort),
        [0]
    );
    assert!(ring.is_empty());
    assert_eq!(ring.counters(datagram).unwrap().acknowledgements, 0);
    assert_eq!(ring.counters(reliable).unwrap().acknowledgements, 1);
}
#[test]
fn only_stalled_consumers_resync_and_old_epoch_receipts_cannot_retire_new_records() {
    assert!(EventRing::load(3, 2, 4, 32).is_err());
    let mut ring = EventRing::load(4, 2, 4, 32).unwrap();
    let stalled = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let healthy = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    for sequence in 0..4 {
        ring.push(sound(sequence)).unwrap();
        consume(&mut ring, healthy, OutputSubmission::BestEffort);
        consume(
            &mut ring,
            stalled,
            OutputSubmission::Reliable(NativeReceipt(9)),
        );
    }
    ring.push(sound(4)).unwrap();
    assert!(ring.needs_resync(stalled));
    assert!(!ring.needs_resync(healthy));
    assert_eq!(
        ring.counters(stalled).unwrap(),
        OutputCounters {
            submissions: 4,
            acknowledgements: 0,
            overflow: 1,
            resyncs: 1,
            retired_on_resync: 4,
            skipped_during_resync: 1,
            ..OutputCounters::default()
        }
    );
    assert_eq!(
        consume(&mut ring, healthy, OutputSubmission::BestEffort),
        [4]
    );
    assert!(ring.is_empty());
    let resumed = ring.resume(stalled).unwrap();
    ring.push(sound(5)).unwrap();
    consume(
        &mut ring,
        resumed,
        OutputSubmission::Reliable(NativeReceipt(9)),
    );
    consume(&mut ring, healthy, OutputSubmission::BestEffort);
    assert_eq!(ring.acknowledge(stalled, NativeReceipt(9)), 0);
    assert_eq!(ring.len(), 1);
    assert_eq!(ring.acknowledge(resumed, NativeReceipt(9)), 1);
}
#[test]
fn failed_record_does_not_block_later_records_in_the_same_pass() {
    let mut ring = EventRing::load(4, 1, 4, 32).unwrap();
    let id = ring.bind(OutputTarget::Presentation).unwrap();
    ring.push(sound(1)).unwrap();
    ring.push(sound(2)).unwrap();
    let mut batch = ring.batch(id).unwrap();
    let first = ring.next(&mut batch).unwrap();
    ring.submit(id, first.sequence, OutputSubmission::Unsent);
    let second = ring.next(&mut batch).unwrap();
    ring.submit(id, second.sequence, OutputSubmission::BestEffort);
    assert!(ring.next(&mut batch).is_none());
    assert_eq!(consume(&mut ring, id, OutputSubmission::BestEffort), [0]);
    assert!(ring.is_empty());
}
#[test]
fn pages_reuse_only_after_the_event_and_every_display_lease_release() {
    let mut ring = EventRing::load(2, 1, 3, 32).unwrap();
    let consumer = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    ring.print(None, PrintKind::Center, format_args!("first\n"))
        .unwrap();
    let record = ring.next(&mut ring.batch(consumer).unwrap()).unwrap();
    let FrameEvent::Print(print) = record.event else {
        panic!("print");
    };
    let display = ring.texts.lease(print.text).unwrap();
    ring.submit(
        consumer,
        record.sequence,
        OutputSubmission::Reliable(NativeReceipt(1)),
    );
    for i in 0..100 {
        ring.print(None, PrintKind::Notify, format_args!("{i}"))
            .unwrap();
        // Overflow can resync the native peer, but cannot release a HUD lease.
        if ring.needs_resync(consumer) {
            break;
        }
    }
    assert_eq!(ring.texts.get(display.id()), Some(b"first\n".as_slice()));
    ring.unbind(consumer);
    assert!(ring.is_empty());
    assert_eq!(ring.texts.get(display.id()), Some(b"first\n".as_slice()));
    ring.texts.release(display);
    assert!(ring.texts.get(print.text).is_none());
}
#[test]
fn text_pages_do_not_overwrite_leases_and_keep_utf8_and_newlines() {
    let mut text = TextStore::load(2, 5).unwrap();
    let first = text
        .insert_formatted(format_args!("{}{}{}", "ab", "é", "世"))
        .unwrap();
    let first_id = first.id();
    let display = text.lease(first_id).unwrap();
    let second = text.insert(b"two\n").unwrap();
    assert!(text.insert(b"blocked").is_none());
    text.release(first);
    assert!(text.insert(b"blocked").is_none());
    assert_eq!(text.get(display.id()), Some("abé".as_bytes()));
    assert_eq!(text.get(second.id()), Some(b"two\n".as_slice()));
    text.release(display);
    let third = text.insert(b"three").unwrap();
    assert!(text.get(first_id).is_none());
    assert_eq!(text.get(third.id()), Some(b"three".as_slice()));
    assert_eq!((text.reused(), text.truncated()), (1, 1));
}
#[test]
fn targeted_print_belongs_only_to_its_client_and_observing_module() {
    let mut ring = EventRing::load(4, 3, 4, 32).unwrap();
    let a = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let b = ring.bind(OutputTarget::Client(ClientId(2))).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(7))).unwrap();
    ring.print(Some(ClientId(2)), PrintKind::Layout, format_args!("layout"))
        .unwrap();
    assert!(consume(&mut ring, a, OutputSubmission::BestEffort).is_empty());
    assert_eq!(consume(&mut ring, b, OutputSubmission::BestEffort), [0]);
    assert_eq!(
        consume(&mut ring, module, OutputSubmission::BestEffort),
        [0]
    );
    assert!(ring.is_empty());
}

#[test]
fn late_binding_and_reused_consumer_slots_do_not_inherit_old_records_or_receipts() {
    let mut ring = EventRing::load(4, 2, 4, 32).unwrap();
    let old = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    ring.push(sound(0)).unwrap();
    consume(&mut ring, old, OutputSubmission::Reliable(NativeReceipt(1)));
    let module = ring.bind(OutputTarget::Module(ModuleId(2))).unwrap();
    assert!(consume(&mut ring, module, OutputSubmission::BestEffort).is_empty());
    ring.unbind(old);
    let replacement = ring.bind(OutputTarget::Client(ClientId(2))).unwrap();
    ring.push(sound(1)).unwrap();
    consume(
        &mut ring,
        replacement,
        OutputSubmission::Reliable(NativeReceipt(1)),
    );
    consume(&mut ring, module, OutputSubmission::BestEffort);
    assert_eq!(ring.acknowledge(old, NativeReceipt(1)), 0);
    assert_eq!(ring.len(), 1);
    assert_eq!(ring.acknowledge(replacement, NativeReceipt(1)), 1);
    assert!(ring.is_empty());
}
#[test]
fn one_native_receipt_can_ack_a_span_without_counting_each_record_as_an_ack() {
    let mut ring = EventRing::load(4, 1, 4, 32).unwrap();
    let id = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    for i in 0..4 {
        ring.push(sound(i)).unwrap();
    }
    consume(&mut ring, id, OutputSubmission::Reliable(NativeReceipt(42)));
    assert_eq!(ring.acknowledge(id, NativeReceipt(42)), 4);
    let counters = ring.counters(id).unwrap();
    assert_eq!(
        (counters.acknowledgements, counters.acknowledged_records),
        (1, 4)
    );
    assert!(ring.is_empty());
}
