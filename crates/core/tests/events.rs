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
        ..qa_core::primitives::SoundEvent::default()
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
fn game_state_pages_follow_the_client_receipt_and_keep_encoded_watermarks() {
    let mut ring = EventRing::load(4, 3, 4, 8).unwrap();
    let client = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let other = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let delivery = native_delivery(NativeTarget::Client(ClientId(0)));
    let sequence = ring
        .game_state(delivery, 37, 129, b"full native state")
        .unwrap();
    assert_eq!(consume(&mut ring, other, OutputSubmission::BestEffort), []);
    assert_eq!(
        consume(&mut ring, presentation, OutputSubmission::BestEffort),
        []
    );
    let mut batch = ring.batch(client).unwrap();
    let record = ring.next(&mut batch).unwrap();
    let FrameEvent::GameState {
        payload,
        command_sequence,
        body_bits,
    } = record.event
    else {
        panic!("native game state");
    };
    assert_eq!((command_sequence, body_bits), (37, 129));
    let mut bytes = [0; 32];
    assert_eq!(
        ring.texts.copy_payload(payload, &mut bytes).unwrap(),
        b"full native state"
    );
    assert!(ring.submit(
        client,
        sequence,
        OutputSubmission::Reliable(NativeReceipt(9))
    ));
    assert_eq!(ring.acknowledge(client, NativeReceipt(8)), 0);
    assert!(ring.retained_for(client, sequence));
    assert_eq!(
        ring.texts.copy_payload(payload, &mut bytes).unwrap(),
        b"full native state"
    );
    assert_eq!(ring.acknowledge(client, NativeReceipt(9)), 1);
    assert!(ring.is_empty());
    assert!(ring.texts.copy_payload(payload, &mut bytes).is_none());
}
#[test]
fn game_state_bit_boundary_rejection_preserves_existing_payload() {
    let mut ring = EventRing::load(2, 1, 2, 8).unwrap();
    let client = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let delivery = native_delivery(NativeTarget::Client(ClientId(0)));
    let sequence = ring.game_state(delivery, 2, 7, b"x").unwrap();
    for bits in [0, 9, usize::MAX] {
        assert_eq!(
            ring.game_state(delivery, 2, bits, b"x"),
            Err(EventStorageError::TextCapacity)
        );
    }
    assert!(ring.retained_for(client, sequence));
    assert_eq!(ring.rejected_publications(), 3);
    assert!(!ring.needs_resync(client));
    let mut batch = ring.batch(client).unwrap();
    let FrameEvent::GameState { payload, .. } = ring.next(&mut batch).unwrap().event else {
        panic!("retained game state");
    };
    assert_eq!(ring.texts.copy_payload(payload, &mut [0; 8]).unwrap(), b"x");
}
#[test]
fn received_local_text_shares_one_record_and_independent_hud_lease() {
    let mut ring = EventRing::load(4, 4, 4, 32).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let selected = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let other = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(7))).unwrap();
    let sequence = ring
        .print_level_bytes(
            Some(OutputTarget::Local(ClientId(0))),
            Some(ClientId(0)),
            PrintKind::Center,
            2,
            b"received text",
        )
        .unwrap();
    assert_eq!(consume(&mut ring, other, OutputSubmission::BestEffort), []);
    assert_eq!(consume(&mut ring, module, OutputSubmission::BestEffort), []);
    assert_eq!(
        consume(&mut ring, presentation, OutputSubmission::BestEffort),
        [sequence]
    );
    assert_eq!(ring.len(), 1);
    let mut batch = ring.batch(selected).unwrap();
    let record = ring.next(&mut batch).unwrap();
    let FrameEvent::Print(print) = record.event else {
        panic!("local print")
    };
    let display = ring.texts.lease(print.text).unwrap();
    assert!(ring.submit(selected, record.sequence, OutputSubmission::BestEffort));
    assert_eq!(ring.len(), 0);
    assert_eq!(ring.texts.get(display.id()), Some(&b"received text"[..]));
    ring.texts.release(display);
    assert_eq!(ring.texts.get(print.text), None);
}

#[test]
fn local_client_cursor_accepts_only_its_explicit_received_scope() {
    let mut ring = EventRing::load(8, 4, 8, 64).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let server_client = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let local = ring.bind(OutputTarget::Local(ClientId(1))).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(7))).unwrap();
    let broadcast = ring
        .print_level_bytes(None, None, PrintKind::Notify, 2, b"SERVER broadcast")
        .unwrap();
    ring.print_level_bytes(
        Some(OutputTarget::Client(ClientId(1))),
        Some(ClientId(1)),
        PrintKind::Center,
        2,
        b"SERVER unicast",
    )
    .unwrap();
    let native = ring
        .push_delivered(native_delivery(NativeTarget::All), sound(3))
        .unwrap();
    let received = ring
        .print_level_bytes(
            Some(OutputTarget::Local(ClientId(1))),
            Some(ClientId(1)),
            PrintKind::Center,
            2,
            b"received CLIENT text",
        )
        .unwrap();
    assert_eq!(
        consume(&mut ring, local, OutputSubmission::BestEffort),
        [received]
    );
    assert_eq!(
        consume(&mut ring, server_client, OutputSubmission::BestEffort),
        [broadcast, native]
    );
    assert_eq!(
        consume(&mut ring, module, OutputSubmission::BestEffort),
        [broadcast, native]
    );
    assert_eq!(
        consume(&mut ring, presentation, OutputSubmission::BestEffort),
        [broadcast, received]
    );
    assert!(ring.is_empty());
}
#[test]
fn oversized_group_rejection_does_not_cancel_a_stalled_peer() {
    let mut ring = EventRing::load(2, 2, 2, 32).unwrap();
    let stalled = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let healthy = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    ring.push_delivered(native_delivery(NativeTarget::All), sound(1))
        .unwrap();
    let before = ring.sequence_range();
    let counters = ring.counters(stalled).unwrap();
    assert_eq!(
        ring.publish_group(3, |_| panic!("capacity failure must not stage")),
        Err(EventStorageError::RingCapacity)
    );
    assert_eq!(ring.sequence_range(), before);
    assert_eq!(ring.counters(stalled), Some(counters));
    assert!(!ring.needs_resync(stalled));
    assert_eq!(ring.rejected_publications(), 1);
    assert_eq!(
        consume(&mut ring, healthy, OutputSubmission::BestEffort),
        [0]
    );
    assert_eq!(ring.len(), 1);
}
#[test]
fn grouped_publication_resyncs_only_the_stalled_peer_and_keeps_healthy_delivery() {
    let mut ring = EventRing::load(4, 2, 4, 32).unwrap();
    let stalled = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let healthy = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let delivery = native_delivery(NativeTarget::All);
    for i in 0..3 {
        ring.publish_group(1, |group| {
            group.push_delivered(delivery, sound(i))?;
            Ok(())
        })
        .unwrap();
    }
    assert_eq!(
        consume(&mut ring, healthy, OutputSubmission::BestEffort),
        [0, 1, 2]
    );
    assert_eq!(
        consume(
            &mut ring,
            stalled,
            OutputSubmission::Reliable(NativeReceipt(7))
        ),
        [0, 1, 2]
    );
    // One unused slot cannot fit the next group. The stalled receipts alone
    // are retired so that this whole publication reaches the healthy peer.
    assert_eq!(
        ring.publish_group(2, |group| {
            group.push_delivered(delivery, sound(3))?;
            group.push_delivered(delivery, sound(4))?;
            Ok(())
        })
        .unwrap(),
        3..5
    );
    assert!(ring.needs_resync(stalled));
    assert!(!ring.needs_resync(healthy));
    assert_eq!(ring.counters(stalled).unwrap().retired_on_resync, 3);
    assert_eq!(ring.acknowledge(stalled, NativeReceipt(7)), 0);
    assert_eq!(
        consume(&mut ring, healthy, OutputSubmission::BestEffort),
        [3, 4]
    );
    assert!(ring.is_empty());
    assert_eq!(ring.rejected_publications(), 0);
}
#[test]
fn grouped_page_failure_rolls_back_text_and_resync_skip_counters() {
    let mut ring = EventRing::load(4, 2, 1, 32).unwrap();
    let stalled = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let healthy = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    for i in 0..5 {
        ring.push(sound(i)).unwrap();
    }
    assert!(ring.needs_resync(stalled));
    let before = ring.sequence_range();
    let counters = ring.counters(stalled).unwrap();
    let reused = ring.texts.reused();
    let delivery = native_delivery(NativeTarget::All);
    assert_eq!(
        ring.publish_group(3, |group| {
            group.push_delivered(delivery, sound(9))?;
            group.native_message_parts(delivery, b"\x0aprint\0", &[])?;
            group.native_message_parts(delivery, b"\x0alater\0", &[])?;
            Ok(())
        }),
        Err(EventStorageError::TextCapacity)
    );
    assert_eq!(ring.sequence_range(), before);
    assert_eq!(ring.counters(stalled), Some(counters));
    assert_eq!(ring.texts.reused(), reused);
    assert_eq!(ring.rejected_publications(), 1);
    let lease = ring.texts.insert(b"reusable").unwrap();
    assert_eq!(lease.id().generation, 1);
    ring.texts.release(lease);
    assert!(ring.batch(healthy).is_none());
}
#[test]
fn grouped_siblings_keep_order_and_wait_for_each_native_receipt() {
    let mut ring = EventRing::load(4, 2, 2, 32).unwrap();
    let client = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let delivery = native_delivery(NativeTarget::All);
    let range = ring
        .publish_group(3, |group| {
            group.native_message_parts(delivery, b"\x0a\x02print\0", &[])?;
            group.push_delivered(
                NativeDelivery {
                    component: 8,
                    ..delivery
                },
                sound(11),
            )?;
            group.push_delivered(
                NativeDelivery {
                    component: 11,
                    ..delivery
                },
                sound(12),
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(range, 0..3);
    assert!(ring.next(&mut ring.batch(presentation).unwrap()).is_none());
    let mut batch = ring.batch(client).unwrap();
    let first = ring.next(&mut batch).unwrap();
    let FrameEvent::Native(payload) = first.event else {
        panic!("print")
    };
    for (i, expected) in [(1, 11), (2, 12)] {
        let record = ring.next(&mut batch).unwrap();
        assert_eq!(record.sequence, i);
        assert!(matches!(record.event, FrameEvent::Sound(e) if e.sound == SoundId(expected)));
        assert!(ring.submit(client, i, OutputSubmission::Reliable(NativeReceipt(i + 10))));
    }
    assert!(ring.submit(client, 0, OutputSubmission::Reliable(NativeReceipt(10))));
    assert_eq!(ring.acknowledge(client, NativeReceipt(11)), 1);
    assert_eq!(ring.acknowledge(client, NativeReceipt(12)), 1);
    assert!(ring.texts.copy_payload(payload, &mut [0; 32]).is_some());
    assert_eq!(ring.acknowledge(client, NativeReceipt(10)), 1);
    assert!(ring.is_empty());
}
#[test]
fn configstring_text_stays_exact_and_leased_until_the_native_ack() {
    let mut ring = EventRing::load(4, 2, 4, 8).unwrap();
    let client = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let other = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let sequence = ring
        .publish_configstring(
            Some(OutputTarget::Client(ClientId(0))),
            ModuleId(2),
            RuleSetId::Quake2Rerelease,
            62,
            62,
            b"m\x80\xff",
        )
        .unwrap();
    let mut batch = ring.batch(client).unwrap();
    let event = ring.next(&mut batch).unwrap();
    let FrameEvent::ConfigString {
        module,
        source,
        index,
        destination_index,
        text,
    } = event.event
    else {
        panic!("wrong event")
    };
    assert_eq!(
        (module, source, index, destination_index),
        (ModuleId(2), RuleSetId::Quake2Rerelease, 62, 62)
    );
    assert_eq!(ring.texts.get(text), Some(&b"m\x80\xff"[..]));
    assert!(consume(&mut ring, other, OutputSubmission::BestEffort).is_empty());
    assert!(ring.submit(
        client,
        sequence,
        OutputSubmission::Reliable(NativeReceipt(7))
    ));
    assert_eq!(ring.acknowledge(client, NativeReceipt(6)), 0);
    assert_eq!(ring.texts.get(text), Some(&b"m\x80\xff"[..]));
    assert_eq!(ring.acknowledge(client, NativeReceipt(7)), 1);
    assert!(ring.texts.get(text).is_none());
    assert_eq!(
        ring.publish_configstring(
            Some(OutputTarget::Client(ClientId(0))),
            ModuleId(2),
            RuleSetId::Quake2,
            32,
            32,
            b"123456789"
        ),
        Err(EventStorageError::TextCapacity)
    );
    assert_eq!(ring.texts.truncated(), 0);
    assert_eq!(ring.rejected_publications(), 1);
}
#[test]
fn targeted_publications_belong_only_to_the_selected_consumer() {
    let mut ring = EventRing::load(4, 3, 4, 32).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let remote = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(0))).unwrap();
    for sequence in 0..16 {
        ring.push_to(OutputTarget::Presentation, sound(sequence))
            .unwrap();
        assert_eq!(
            consume(&mut ring, presentation, OutputSubmission::BestEffort),
            [u64::from(sequence)]
        );
        assert!(consume(&mut ring, remote, OutputSubmission::Unsent).is_empty());
        assert!(consume(&mut ring, module, OutputSubmission::Unsent).is_empty());
        assert!(ring.is_empty());
    }
    assert!(!ring.needs_resync(remote));
    assert_eq!(ring.counters(remote).unwrap().submissions, 0);
}

#[test]
fn private_sound_only_requires_its_client_and_presentation() {
    let mut ring = EventRing::load(4, 3, 4, 32).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let selected = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let other = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let FrameEvent::Sound(mut event) = sound(1) else {
        unreachable!()
    };
    event.client = Some(ClientId(0));
    let sequence = ring.push(FrameEvent::Sound(event)).unwrap();
    assert_eq!(
        consume(&mut ring, presentation, OutputSubmission::BestEffort),
        [sequence]
    );
    assert!(consume(&mut ring, other, OutputSubmission::Unsent).is_empty());
    assert!(!ring.is_empty());
    assert_eq!(
        consume(&mut ring, selected, OutputSubmission::BestEffort),
        [sequence]
    );
    assert!(ring.is_empty());
}

#[test]
fn delivered_server_sound_uses_only_delivery_audience_and_never_direct_presentation() {
    let mut ring = EventRing::load(4, 4, 4, 32).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let selected = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let other = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(7))).unwrap();
    let FrameEvent::Sound(mut event) = sound(1) else {
        unreachable!()
    };
    // Presentation listener state cannot override SERVER recipient authority.
    event.client = Some(ClientId(1));
    let delivery = NativeDelivery {
        module: ModuleId(7),
        source: RuleSetId::Quake2,
        target: NativeTarget::Client(ClientId(0)),
        origin: Vec3::default(),
        reliable: true,
        dupe_key: 42,
        component: 0,
        emission: None,
        check_portals: true,
    };
    let sequence = ring
        .push_delivered(delivery, FrameEvent::Sound(event))
        .unwrap();
    assert!(consume(&mut ring, presentation, OutputSubmission::Unsent).is_empty());
    assert!(consume(&mut ring, other, OutputSubmission::Unsent).is_empty());
    let mut batch = ring.batch(selected).unwrap();
    assert_eq!(ring.next(&mut batch).unwrap().delivery, Some(delivery));
    assert!(ring.submit(
        selected,
        sequence,
        OutputSubmission::Reliable(NativeReceipt(8))
    ));
    assert_eq!(
        consume(&mut ring, module, OutputSubmission::BestEffort),
        [sequence]
    );
    assert_eq!(ring.acknowledge(selected, NativeReceipt(7)), 0);
    assert!(!ring.is_empty());
    assert_eq!(ring.acknowledge(selected, NativeReceipt(8)), 1);
    assert!(ring.is_empty());
    // Received/local presentation has no native delivery and cannot retransmit.
    let presented = ring.push_to(OutputTarget::Presentation, sound(2)).unwrap();
    assert_eq!(
        consume(&mut ring, presentation, OutputSubmission::BestEffort),
        [presented]
    );
    assert!(consume(&mut ring, selected, OutputSubmission::Unsent).is_empty());
}

#[test]
fn native_ack_observer_sees_the_owned_record_once_before_text_retirement() {
    let mut ring = EventRing::load(4, 1, 4, 32).unwrap();
    let client = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let sequence = ring
        .publish_configstring(
            Some(OutputTarget::Client(ClientId(0))),
            ModuleId(7),
            RuleSetId::Quake2,
            33,
            33,
            b"a.wav",
        )
        .unwrap();
    assert_eq!(
        consume(
            &mut ring,
            client,
            OutputSubmission::Reliable(NativeReceipt(8))
        ),
        [sequence]
    );
    let mut observed = Vec::new();
    assert_eq!(
        ring.acknowledge_with(client, NativeReceipt(7), |record| observed.push(record)),
        0
    );
    assert!(observed.is_empty());
    assert_eq!(
        ring.acknowledge_with(client, NativeReceipt(8), |record| observed.push(record)),
        1
    );
    assert_eq!(observed.len(), 1);
    let FrameEvent::ConfigString {
        module,
        index,
        text,
        ..
    } = observed[0].event
    else {
        panic!("config")
    };
    assert_eq!(
        (module, index, observed[0].sequence),
        (ModuleId(7), 33, sequence)
    );
    assert!(ring.texts.get(text).is_none());
    assert_eq!(
        ring.acknowledge_with(client, NativeReceipt(8), |_| panic!("duplicate ACK")),
        0
    );
}

#[test]
fn server_commands_target_only_clients_and_hold_text_through_native_ack() {
    let mut ring = EventRing::load(4, 4, 4, 32).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let first = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let second = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(7))).unwrap();
    let sequence = ring
        .server_command(None, ModuleId(7), RuleSetId::Quake3, b"cp \"ready\"")
        .unwrap();

    assert!(consume(&mut ring, presentation, OutputSubmission::Unsent).is_empty());
    assert!(consume(&mut ring, module, OutputSubmission::Unsent).is_empty());
    let mut batch = ring.batch(first).unwrap();
    let record = ring.next(&mut batch).unwrap();
    let FrameEvent::ServerCommand {
        module: owner,
        source,
        text,
    } = record.event
    else {
        panic!("server command");
    };
    assert_eq!((owner, source), (ModuleId(7), RuleSetId::Quake3));
    assert_eq!(ring.texts.get(text), Some(b"cp \"ready\"".as_slice()));
    assert!(ring.submit(
        first,
        sequence,
        OutputSubmission::Reliable(NativeReceipt(3))
    ));
    assert_eq!(
        consume(
            &mut ring,
            second,
            OutputSubmission::Reliable(NativeReceipt(9))
        ),
        [sequence]
    );
    assert_eq!(ring.acknowledge(first, NativeReceipt(3)), 1);
    assert_eq!(ring.texts.get(text), Some(b"cp \"ready\"".as_slice()));
    assert_eq!(ring.acknowledge(second, NativeReceipt(9)), 1);
    assert!(ring.is_empty());
    assert!(ring.texts.get(text).is_none());

    let targeted = ring
        .server_command(
            Some(ClientId(1)),
            ModuleId(7),
            RuleSetId::Quake3,
            b"print \"only second\"",
        )
        .unwrap();
    assert!(consume(&mut ring, first, OutputSubmission::BestEffort).is_empty());
    assert_eq!(
        consume(&mut ring, second, OutputSubmission::BestEffort),
        [targeted]
    );
    assert!(ring.is_empty());
}
#[test]
fn console_parts_preserve_raw_bytes_in_one_bounded_ring_lease() {
    let mut ring = EventRing::load(4, 2, 1, 8).unwrap();
    let local = ring.bind(OutputTarget::Presentation).unwrap();
    let remote = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    ring.console_parts(&[b"raw ", b"\xff\xfe", b" tail"])
        .unwrap();
    assert_eq!(ring.len(), 1);
    assert_eq!(ring.texts.truncated(), 1);
    let mut batch = ring.batch(local).unwrap();
    let record = ring.next(&mut batch).unwrap();
    let FrameEvent::Print(print) = record.event else {
        panic!("print");
    };
    assert_eq!(
        (print.kind, print.client, print.level),
        (PrintKind::Console, None, 2)
    );
    assert_eq!(ring.texts.get(print.text).unwrap(), b"raw \xff\xfe t");
    assert_eq!(
        ring.console_parts(&[b"blocked"]),
        Err(EventStorageError::TextCapacity)
    );
    assert_eq!(ring.rejected_publications(), 1);
    assert!(consume(&mut ring, remote, OutputSubmission::Unsent).is_empty());
    assert!(ring.submit(local, record.sequence, OutputSubmission::BestEffort));
    assert!(ring.is_empty());
    ring.console_parts(&[b"", b"next", b""]).unwrap();
    let mut batch = ring.batch(local).unwrap();
    let FrameEvent::Print(print) = ring.next(&mut batch).unwrap().event else {
        panic!("print");
    };
    assert_eq!(ring.texts.get(print.text).unwrap(), b"next");
    assert_eq!(ring.texts.truncated(), 1);
}

#[test]
fn formatted_console_uses_local_delivery_and_the_ring_text_boundary() {
    let mut ring = EventRing::load(4, 3, 1, 32).unwrap();
    let local = ring.bind(OutputTarget::Presentation).unwrap();
    let remote = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(7))).unwrap();
    ring.console_formatted(format_args!("{} {}", "é".repeat(20), 42))
        .unwrap();
    let mut batch = ring.batch(local).unwrap();
    let record = ring.next(&mut batch).unwrap();
    let FrameEvent::Print(print) = record.event else {
        panic!("print");
    };
    assert_eq!(
        (print.client, print.kind, print.level),
        (None, PrintKind::Console, 2)
    );
    assert_eq!(
        ring.texts.get(print.text).unwrap(),
        "é".repeat(16).as_bytes()
    );
    assert_eq!(ring.texts.truncated(), 1);
    assert_eq!(
        ring.console_formatted(format_args!("blocked {}", 7)),
        Err(EventStorageError::TextCapacity)
    );
    assert!(consume(&mut ring, remote, OutputSubmission::Unsent).is_empty());
    assert!(consume(&mut ring, module, OutputSubmission::Unsent).is_empty());
    assert!(ring.submit(local, record.sequence, OutputSubmission::BestEffort));
    assert!(ring.is_empty());
    ring.console_formatted(format_args!("ready {}\n", 7))
        .unwrap();
    let mut batch = ring.batch(local).unwrap();
    let FrameEvent::Print(print) = ring.next(&mut batch).unwrap().event else {
        panic!("print");
    };
    assert_eq!(ring.texts.get(print.text).unwrap(), b"ready 7\n");
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
    let mut resumed = stalled;
    assert!(ring.resume_if_needed(&mut resumed));
    assert_ne!(resumed, stalled);
    let unchanged = resumed;
    assert!(ring.resume_if_needed(&mut resumed));
    assert_eq!(resumed, unchanged);
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

fn native_delivery(target: NativeTarget) -> NativeDelivery {
    NativeDelivery {
        module: ModuleId(7),
        source: RuleSetId::Quake2Rerelease,
        target,
        origin: Vec3([1.0, -2.0, 3.0]),
        reliable: true,
        dupe_key: u32::MAX,
        component: 0,
        emission: None,
        check_portals: true,
    }
}
#[test]
fn native_pages_follow_each_consumer_receipt_without_cloning_or_truncating() {
    let mut ring = EventRing::load(4, 3, 4, 32).unwrap();
    let first = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let second = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(7))).unwrap();
    let bytes: Vec<_> = (0..128).map(|i| i as u8).collect();
    let delivery = native_delivery(NativeTarget::All);
    let sequence = ring.native_message(delivery, &bytes).unwrap();
    let mut batch = ring.batch(first).unwrap();
    let record = ring.next(&mut batch).unwrap();
    let FrameEvent::Native(message) = record.event else {
        panic!()
    };
    assert_eq!(record.delivery, Some(delivery));
    let mut output = [0xcc; 128];
    assert!(
        ring.texts
            .copy_payload(message, &mut output[..127])
            .is_none()
    );
    assert_eq!(
        ring.texts.copy_payload(message, &mut output).unwrap(),
        bytes
    );
    assert_eq!(ring.texts.truncated(), 0);
    consume(&mut ring, second, OutputSubmission::BestEffort);
    consume(&mut ring, module, OutputSubmission::BestEffort);
    assert!(ring.submit(
        first,
        sequence,
        OutputSubmission::Reliable(NativeReceipt(9))
    ));
    assert_eq!(ring.acknowledge(first, NativeReceipt(8)), 0);
    assert!(ring.texts.copy_payload(message, &mut output).is_some());
    assert_eq!(ring.acknowledge(first, NativeReceipt(9)), 1);
    assert!(ring.texts.copy_payload(message, &mut output).is_none());
    assert!(ring.is_empty());
}
#[test]
fn native_failed_page_admission_releases_partial_pages_and_unicast_excludes_other_clients() {
    let mut ring = EventRing::load(4, 2, 3, 32).unwrap();
    let first = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let second = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let delivery = native_delivery(NativeTarget::Client(ClientId(1)));
    assert_eq!(
        ring.native_message(delivery, &[1; 129]),
        Err(EventStorageError::TextCapacity)
    );
    assert_eq!(
        ring.native_message(delivery, &[2; 128]),
        Err(EventStorageError::TextCapacity)
    );
    assert!(ring.is_empty());
    ring.native_message(delivery, &[3; 96]).unwrap();
    assert!(consume(&mut ring, first, OutputSubmission::Unsent).is_empty());
    assert_eq!(
        consume(&mut ring, second, OutputSubmission::BestEffort),
        [0]
    );
    assert!(ring.is_empty());
    assert_eq!(ring.rejected_publications(), 2);
    ring.native_message(delivery, &[4; 96]).unwrap();
    consume(&mut ring, second, OutputSubmission::BestEffort);
    assert!(ring.is_empty());
}

#[test]
fn full_native_wire_and_dense_capture_pages_wait_for_every_real_ack() {
    let mut ring = EventRing::load(4, 2, NATIVE_PAYLOAD_PAGES, OUTPUT_PAGE_BYTES).unwrap();
    let first = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let second = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let mut stored = vec![6; NATIVE_PAYLOAD_BYTES];
    let binding = NativeCueBinding {
        entity: Some(EntityId {
            slot: 2,
            generation: 19,
        }),
    };
    for index in 0..NATIVE_CUE_CAPACITY {
        let offset = index * NATIVE_CUE_MIN_WIRE_BYTES;
        stored[offset..offset + 4].copy_from_slice(&[1, 2, 0, 0]);
        binding
            .write_capture(
                offset as u32,
                &mut stored[NATIVE_WIRE_BYTES + index * NATIVE_CUE_CAPTURE_BYTES..],
            )
            .unwrap();
    }
    let sequence = ring
        .native_message_bound(
            native_delivery(NativeTarget::All),
            NATIVE_WIRE_BYTES,
            &stored,
        )
        .unwrap();
    let mut batch = ring.batch(first).unwrap();
    let FrameEvent::Native(payload) = ring.next(&mut batch).unwrap().event else {
        panic!("native payload")
    };
    let mut copied = vec![0; NATIVE_PAYLOAD_BYTES];
    assert_eq!(
        ring.texts.copy_payload(payload, &mut copied).unwrap(),
        stored
    );
    for offset in [0, (NATIVE_CUE_CAPACITY - 1) * NATIVE_CUE_MIN_WIRE_BYTES] {
        let captured = payload.cue(&copied, offset).unwrap();
        assert_eq!(captured.entity, binding.entity);
    }
    assert_eq!(payload.len(), NATIVE_WIRE_BYTES);
    assert_eq!(ring.rejected_publications(), 0);
    assert!(ring.submit(
        first,
        sequence,
        OutputSubmission::Reliable(NativeReceipt(7))
    ));
    assert!(ring.submit(
        second,
        sequence,
        OutputSubmission::Reliable(NativeReceipt(8))
    ));
    assert_eq!(ring.acknowledge(first, NativeReceipt(6)), 0);
    assert_eq!(ring.acknowledge(first, NativeReceipt(7)), 1);
    assert!(ring.texts.copy_payload(payload, &mut copied).is_some());
    assert_eq!(ring.acknowledge(second, NativeReceipt(8)), 1);
    assert!(ring.texts.copy_payload(payload, &mut copied).is_none());
    assert!(ring.is_empty());
    // All leased pages are reusable after the last real receipt, not before.
    ring.native_message_bound(
        native_delivery(NativeTarget::All),
        NATIVE_WIRE_BYTES,
        &stored,
    )
    .unwrap();
}

#[test]
fn dispatch_stop_preserves_pending_record_and_next_cursor() {
    let mut ring = EventRing::load(8, 1, 8, 32).unwrap();
    let id = ring.bind(OutputTarget::Presentation).unwrap();
    ring.push(sound(1)).unwrap();
    ring.push(sound(2)).unwrap();
    let stopped = std::cell::Cell::new(false);
    let mut seen = Vec::new();
    dispatch_consumer_until(
        &mut ring,
        id,
        &mut |record: OutputRecord, _: &mut TextStore| {
            seen.push(record.sequence);
            stopped.set(true);
            OutputSubmission::Unsent
        },
        || stopped.get(),
    );
    assert_eq!(seen, [0]);
    assert_eq!(ring.len(), 2);
    seen.clear();
    dispatch_consumer(
        &mut ring,
        id,
        &mut |record: OutputRecord, _: &mut TextStore| {
            seen.push(record.sequence);
            OutputSubmission::BestEffort
        },
    );
    assert_eq!(seen, [0, 1]);
    assert!(ring.is_empty());
}

#[test]
fn client_audio_retirement_is_a_presentation_only_lifetime_control() {
    use qa_core::primitives::{ClientId, EntityId, SoundAction};
    let mut ring = EventRing::load(4, 3, 4, 64).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let client = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let lifetime = EntityId {
        slot: 7,
        generation: 3,
    };
    let event = SoundEvent::retire_client(ClientId(0), lifetime);
    assert_eq!(event.action, SoundAction::RetireClient);
    assert_eq!(event.client_lifetime, Some(lifetime));
    assert!(event.native_fields.is_none());
    let sequence = ring
        .push_to(OutputTarget::Presentation, FrameEvent::Sound(event))
        .unwrap();
    assert!(ring.retained_for(presentation, sequence));
    assert!(!ring.retained_for(client, sequence));
    dispatch_consumer(
        &mut ring,
        presentation,
        &mut |record: OutputRecord, _: &mut TextStore| {
            assert!(
                matches!(record.event, FrameEvent::Sound(sound) if sound.client_lifetime == Some(lifetime))
            );
            OutputSubmission::BestEffort
        },
    );
    assert!(ring.is_empty());
}

#[test]
fn grouped_native_prints_retain_multi_page_raw_text_until_each_real_receipt() {
    let mut ring = EventRing::load(8, 2, 8, OUTPUT_PAGE_BYTES).unwrap();
    let first = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let second = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let mut delivery = native_delivery(NativeTarget::All);
    delivery.source = RuleSetId::Quake2Rerelease;
    delivery.reliable = true;
    let mut bytes = vec![b'x'; OUTPUT_PAGE_BYTES + 100];
    bytes[0] = 10;
    bytes[1] = 0;
    let last = bytes.len() - 1;
    bytes[last] = 0;
    let span = IndexSpan {
        first: 2,
        count: (bytes.len() - 3) as u32,
    };
    let range = ring
        .publish_group(2, |group| {
            group.print(
                delivery,
                PrintKind::Notify,
                0,
                &bytes,
                span,
                bytes.len() as u32,
            )?;
            group.print(
                NativeDelivery {
                    component: bytes.len() as u32,
                    ..delivery
                },
                PrintKind::Center,
                2,
                b"\x0fafter\0",
                IndexSpan { first: 1, count: 5 },
                7,
            )?;
            Ok(())
        })
        .unwrap();
    let mut batch = ring.batch(first).unwrap();
    let record = ring.next(&mut batch).unwrap();
    let FrameEvent::Print(print) = record.event else {
        panic!("typed print")
    };
    let PrintText::Native { wire, text, .. } = print.text else {
        panic!("raw page owner")
    };
    let mut scratch = vec![0; bytes.len()];
    let stored = ring.texts.copy_payload(wire, &mut scratch).unwrap();
    assert_eq!(stored, bytes);
    assert_eq!(text, span);
    assert_eq!(&stored[text.range()], &bytes[2..last]);
    assert_eq!(ring.texts.truncated(), 0);
    assert!(
        ring.texts.get(print.text).is_none(),
        "SERVER wire is not a CLIENT display lease"
    );
    for sequence in range.clone() {
        assert!(ring.submit(
            first,
            sequence,
            OutputSubmission::Reliable(NativeReceipt(sequence + 1))
        ));
        assert!(ring.submit(second, sequence, OutputSubmission::BestEffort));
    }
    assert_eq!(ring.len(), 2);
    assert!(ring.texts.copy_payload(wire, &mut scratch).is_some());
    assert_eq!(ring.acknowledge(first, NativeReceipt(1)), 1);
    assert!(ring.texts.copy_payload(wire, &mut scratch).is_none());
    assert_eq!(ring.len(), 1);
    assert_eq!(ring.acknowledge(first, NativeReceipt(2)), 1);
    assert!(ring.is_empty());
}

#[test]
fn grouped_native_print_failure_has_no_visible_prefix_or_leaked_pages() {
    let mut ring = EventRing::load(4, 1, 1, 8).unwrap();
    let consumer = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let delivery = native_delivery(NativeTarget::All);
    let checkpoint = (
        ring.sequence_range(),
        ring.texts.reused(),
        ring.texts.truncated(),
    );
    assert_eq!(
        ring.publish_group(2, |group| {
            group.print(
                delivery,
                PrintKind::Console,
                2,
                b"\x0a\x02p\0",
                IndexSpan { first: 2, count: 1 },
                4,
            )?;
            group.print(
                delivery,
                PrintKind::Console,
                2,
                b"\x0a\x02q\0",
                IndexSpan { first: 2, count: 1 },
                4,
            )?;
            Ok(())
        }),
        Err(EventStorageError::TextCapacity)
    );
    assert_eq!(
        (
            ring.sequence_range(),
            ring.texts.reused(),
            ring.texts.truncated()
        ),
        checkpoint
    );
    assert_eq!(ring.rejected_publications(), 1);
    assert!(ring.next(&mut ring.batch(consumer).unwrap()).is_none());
    assert!(ring.texts.insert(b"reusable").is_some());
    assert_eq!(
        ring.publish_group(1, |group| {
            group.print(
                delivery,
                PrintKind::Console,
                2,
                b"\x0a\x02p\0",
                IndexSpan { first: 3, count: 2 },
                4,
            )?;
            Ok(())
        }),
        Err(EventStorageError::TextCapacity)
    );
}

#[test]
fn music_paths_retire_with_presentation_and_an_independent_text_lease() {
    let mut ring = EventRing::load(8, 4, 4, 64).unwrap();
    let healthy = ring.bind(OutputTarget::Presentation).unwrap();
    let remote = ring.bind(OutputTarget::Client(ClientId(2))).unwrap();
    let owner = MusicOwner {
        module: Some(ModuleId(7)),
        client: Some((
            ClientId(2),
            EntityId {
                slot: 9,
                generation: 3,
            },
        )),
    };
    let playback = MusicPlayback {
        wave_first: true,
        smooth_volume: true,
        preserve_queued: true,
        signed_stereo_bytes: true,
    };
    let sequence = ring
        .music(owner, playback, b"music/intro.wav", b"music/loop.wav", None)
        .unwrap();
    let mut batch = ring.batch(healthy).unwrap();
    let record = ring.next(&mut batch).unwrap();
    let FrameEvent::Music(MusicControl::Start {
        owner: actual,
        intro,
        loop_track,
        ..
    }) = record.event
    else {
        panic!("music");
    };
    assert_eq!(actual, owner);
    let retained = ring.texts.lease(intro).unwrap();
    let mut remote_batch = ring.batch(remote).unwrap();
    assert!(ring.next(&mut remote_batch).is_none());
    assert!(ring.submit(healthy, sequence, OutputSubmission::BestEffort));
    assert_eq!(ring.texts.get(intro), Some(b"music/intro.wav".as_slice()));
    assert_eq!(ring.texts.get(loop_track), None);
    ring.texts.release(retained);
    assert_eq!(ring.texts.get(intro), None);
    assert_eq!(ring.texts.get(loop_track), None);
}

#[test]
fn music_second_path_failure_rolls_back_first_path_and_counts_once() {
    let mut ring = EventRing::load(4, 2, 2, 64).unwrap();
    let _consumer = ring.bind(OutputTarget::Presentation).unwrap();
    let retained = ring.texts.insert(b"held display text").unwrap();
    let before = ring.sequence_range();
    let owner = MusicOwner {
        module: Some(ModuleId(1)),
        client: None,
    };
    let playback = MusicPlayback {
        wave_first: false,
        smooth_volume: false,
        preserve_queued: false,
        signed_stereo_bytes: false,
    };
    assert_eq!(
        ring.music(owner, playback, b"intro", b"loop", None),
        Err(EventStorageError::TextCapacity)
    );
    assert_eq!(ring.rejected_publications(), 1);
    assert_eq!(ring.sequence_range(), before);
    let new = ring.texts.insert(b"still free").unwrap();
    assert_eq!(ring.texts.get(new.id()), Some(b"still free".as_slice()));
    assert_eq!(
        ring.texts.get(retained.id()),
        Some(b"held display text".as_slice())
    );
    ring.texts.release(new);
    ring.texts.release(retained);
}

#[test]
fn music_remap_owns_binary_text_and_rejects_malformed_updates_atomically() {
    let mut ring = EventRing::load(8, 2, 4, 1024).unwrap();
    let consumer = ring.bind(OutputTarget::Presentation).unwrap();
    let bytes: Vec<_> = [0_u32, u32::MAX]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    let sequence = ring.music_remap(&bytes).unwrap();
    let before = ring.sequence_range();
    for bad in [&b""[..], &b"abc"[..], &[0; 1024][..]] {
        assert!(ring.music_remap(bad).is_err());
        assert_eq!(ring.sequence_range(), before);
    }
    assert_eq!(ring.rejected_publications(), 3);
    let mut batch = ring.batch(consumer).unwrap();
    let record = ring.next(&mut batch).unwrap();
    let FrameEvent::Music(MusicControl::Remap { entries }) = record.event else {
        panic!("remap");
    };
    assert_eq!(record.sequence, sequence);
    assert_eq!(ring.texts.get(entries).unwrap(), bytes);
    let lease = ring.texts.lease(entries).unwrap();
    assert!(ring.submit(consumer, sequence, OutputSubmission::BestEffort));
    assert!(ring.is_empty());
    assert_eq!(
        MusicRemap::parse(ring.texts.get(entries).unwrap())
            .unwrap()
            .tracks()
            .collect::<Vec<_>>(),
        [0, u32::MAX]
    );
    ring.texts.release(lease);
    assert_eq!(ring.texts.get(entries), None);
}

#[test]
fn console_response_transfers_its_existing_text_and_preserves_display_leases() {
    let mut ring = EventRing::load(8, 2, 4, 64).unwrap();
    let consumer = ring.bind(OutputTarget::Presentation).unwrap();
    let sequence = ring
        .music_control(MusicControl::Report(MusicQuery::Remaps))
        .unwrap();
    let lease = ring
        .texts
        .insert_formatted(format_args!("response {}", 17))
        .unwrap();
    let id = lease.id();
    let address = ring.texts.get(id).unwrap().as_ptr();
    let before = ring.sequence_range();
    assert!(ring.console_response(consumer, sequence, lease));
    assert_eq!(ring.sequence_range(), before);
    assert_eq!(ring.texts.get(id).unwrap().as_ptr(), address);
    let display = ring.texts.lease(id).unwrap();
    assert!(ring.submit(consumer, sequence, OutputSubmission::BestEffort));
    assert_eq!(ring.texts.get(id), Some(&b"response 17"[..]));
    ring.texts.release(display);
    assert_eq!(ring.texts.get(id), None);
}

#[test]
fn console_responses_reject_wrong_records_and_stale_consumers_without_lending_text() {
    let mut ring = EventRing::load(8, 2, 4, 64).unwrap();
    let consumer = ring.bind(OutputTarget::Presentation).unwrap();
    let wrong = ring.music_control(MusicControl::Reset).unwrap();
    let lease = ring.texts.insert(b"wrong").unwrap();
    let id = lease.id();
    assert!(!ring.console_response(consumer, wrong, lease));
    assert_eq!(ring.texts.get(id), None);
    assert!(ring.retained_for(consumer, wrong));
    assert!(ring.submit(consumer, wrong, OutputSubmission::BestEffort));
    ring.unbind(consumer);
    let current = ring.bind(OutputTarget::Presentation).unwrap();
    let sequence = ring
        .music_control(MusicControl::Report(MusicQuery::Remaps))
        .unwrap();
    let lease = ring.texts.insert(b"stale").unwrap();
    let id = lease.id();
    assert!(!ring.console_response(consumer, sequence, lease));
    assert_eq!(ring.texts.get(id), None);
    assert!(ring.retained_for(current, sequence));
    assert_eq!(ring.rejected_publications(), 2);
}

#[test]
fn peer_info_routes_keep_receiver_generations_and_pages_until_every_real_ack() {
    let routes = [
        NativePeerRoute {
            receiver: EntityId {
                slot: 2,
                generation: 7,
            },
            ordinal: 15,
            reset_count: 16,
        },
        NativePeerRoute {
            receiver: EntityId {
                slot: 2,
                generation: 9,
            },
            ordinal: 0,
            reset_count: 0,
        },
        NativePeerRoute {
            receiver: EntityId {
                slot: 20,
                generation: 3,
            },
            ordinal: 31,
            reset_count: 32,
        },
    ];
    let mut captures = [0; 3 * NATIVE_PEER_ROUTE_BYTES];
    for (route, row) in routes
        .into_iter()
        .zip(captures.chunks_exact_mut(NATIVE_PEER_ROUTE_BYTES))
    {
        route.write_capture(row).unwrap();
    }
    assert_eq!(
        &captures[..16],
        &[2, 0, 0, 0, 7, 0, 0, 0, 15, 0, 0, 0, 16, 0, 0, 0]
    );
    assert_eq!(NativePeerRoute::read_capture(&captures), Some(routes[0]));
    assert_eq!(NativePeerRoute::read_capture(&captures[..15]), None);
    let mut short = [0xcc; NATIVE_PEER_ROUTE_BYTES - 1];
    assert_eq!(routes[0].write_capture(&mut short), None);
    assert_eq!(short, [0xcc; NATIVE_PEER_ROUTE_BYTES - 1]);
    let mut ring = EventRing::load(1, 5, 8, 8).unwrap();
    let first = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let second = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(7))).unwrap();
    let local = ring.bind(OutputTarget::Local(ClientId(0))).unwrap();
    let info = b"\\name\\peer\x80";
    let sequence = ring.peer_info(i32::MIN, info, &captures).unwrap();
    assert!(consume(&mut ring, presentation, OutputSubmission::BestEffort).is_empty());
    assert!(consume(&mut ring, module, OutputSubmission::BestEffort).is_empty());
    assert!(consume(&mut ring, local, OutputSubmission::BestEffort).is_empty());
    let mut batch = ring.batch(first).unwrap();
    let record = ring.next(&mut batch).unwrap();
    assert_eq!(record.delivery, None);
    let FrameEvent::PeerInfo { user_id, payload } = record.event else {
        panic!("engine peer info")
    };
    assert_eq!(user_id, i32::MIN);
    assert_eq!(payload.len(), info.len());
    let mut stored = [0; 64];
    let bytes = ring.texts.copy_payload(payload, &mut stored).unwrap();
    assert_eq!(bytes, [info.as_slice(), &captures].concat());
    for route in routes {
        assert_eq!(payload.peer(bytes, route.receiver), Some(route));
    }
    assert_eq!(
        payload.peer(
            bytes,
            EntityId {
                slot: 2,
                generation: 8
            }
        ),
        None
    );
    assert_eq!(
        payload.peer(
            bytes,
            EntityId {
                slot: 20,
                generation: 4
            }
        ),
        None
    );
    assert_eq!(
        payload.peer(&bytes[..bytes.len() - 1], routes[0].receiver),
        None
    );
    assert!(ring.submit(
        first,
        sequence,
        OutputSubmission::Reliable(NativeReceipt(7))
    ));
    assert!(ring.submit(
        second,
        sequence,
        OutputSubmission::Reliable(NativeReceipt(9))
    ));
    assert_eq!(ring.acknowledge(first, NativeReceipt(6)), 0);
    assert_eq!(ring.acknowledge(second, NativeReceipt(8)), 0);
    assert!(ring.texts.copy_payload(payload, &mut stored).is_some());
    assert_eq!(ring.acknowledge(first, NativeReceipt(7)), 1);
    assert!(ring.retained_for(second, sequence));
    assert_eq!(
        ring.texts.copy_payload(payload, &mut stored).unwrap(),
        [info.as_slice(), &captures].concat()
    );
    assert_eq!(ring.acknowledge(second, NativeReceipt(9)), 1);
    assert!(ring.is_empty());
    assert!(ring.texts.copy_payload(payload, &mut stored).is_none());
    ring.peer_info(0, b"", &captures).unwrap();
    let mut batch = ring.batch(first).unwrap();
    let FrameEvent::PeerInfo {
        payload,
        user_id: 0,
    } = ring.next(&mut batch).unwrap().event
    else {
        panic!("empty peer info")
    };
    assert!(payload.is_empty());
    let bytes = ring.texts.copy_payload(payload, &mut stored).unwrap();
    assert_eq!(payload.peer(bytes, routes[2].receiver), Some(routes[2]));
}

#[test]
fn republished_peer_info_leases_the_existing_payload_pages() {
    let mut ring = EventRing::load(2, 4, 3, 8).unwrap();
    let client = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(7))).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let local = ring.bind(OutputTarget::Local(ClientId(0))).unwrap();
    let receiver = EntityId {
        slot: 3,
        generation: 17,
    };
    let mut captures = [0; NATIVE_PEER_ROUTE_BYTES];
    NativePeerRoute {
        receiver,
        ordinal: u32::MAX,
        reset_count: 0,
    }
    .write_capture(&mut captures)
    .unwrap();
    let first = ring.peer_info(i32::MAX, b"info", &captures).unwrap();
    let mut batch = ring.batch(client).unwrap();
    let record = ring.next(&mut batch).unwrap();
    let FrameEvent::PeerInfo { payload, .. } = record.event else {
        panic!("peer info")
    };
    assert!(ring.submit(client, first, OutputSubmission::Reliable(NativeReceipt(1))));
    let second = ring
        .push_delivered(native_delivery(NativeTarget::All), record.event)
        .unwrap();
    for other in [module, presentation, local] {
        assert!(consume(&mut ring, other, OutputSubmission::BestEffort).is_empty());
    }
    assert_eq!(ring.acknowledge(client, NativeReceipt(1)), 1);
    let mut bytes = [0; 24];
    assert_eq!(
        payload.peer(
            ring.texts.copy_payload(payload, &mut bytes).unwrap(),
            receiver
        ),
        Some(NativePeerRoute {
            receiver,
            ordinal: u32::MAX,
            reset_count: 0
        })
    );
    assert!(ring.submit(client, second, OutputSubmission::Reliable(NativeReceipt(2))));
    assert_eq!(ring.acknowledge(client, NativeReceipt(2)), 1);
    assert!(ring.texts.copy_payload(payload, &mut bytes).is_none());
}

#[test]
fn peer_info_text_pressure_and_malformed_captures_publish_no_partial_record() {
    let mut ring = EventRing::load(1, 1, 3, 8).unwrap();
    let client = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let held = ring.texts.insert(b"display").unwrap();
    let mut captures = [0; NATIVE_PEER_ROUTE_BYTES];
    NativePeerRoute {
        receiver: EntityId {
            slot: 1,
            generation: 2,
        },
        ordinal: 3,
        reset_count: 16,
    }
    .write_capture(&mut captures)
    .unwrap();
    let before = ring.sequence_range();
    assert_eq!(
        ring.peer_info(9, b"12345678", &captures),
        Err(EventStorageError::TextCapacity)
    );
    assert_eq!(ring.sequence_range(), before);
    assert!(ring.is_empty());
    assert!(!ring.needs_resync(client));
    assert_eq!(ring.texts.get(held.id()), Some(b"display".as_slice()));
    let reclaimed = ring.texts.insert(b"free").unwrap();
    assert_eq!(ring.texts.get(reclaimed.id()), Some(b"free".as_slice()));
    ring.texts.release(reclaimed);
    assert_eq!(
        ring.peer_info(9, b"x", &captures[..12]),
        Err(EventStorageError::TextCapacity)
    );
    assert_eq!(ring.sequence_range(), before);
    assert_eq!(ring.rejected_publications(), 2);
    ring.texts.release(held);
    assert_eq!(ring.peer_info(9, b"12345678", &captures), Ok(before.end));
}

#[test]
fn shared_capture_search_keeps_native_cue_offsets_and_absent_entity_sentinel() {
    let mut ring = EventRing::load(1, 1, 4, 8).unwrap();
    let client = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let mut stored = [0; 8 + 2 * NATIVE_CUE_CAPTURE_BYTES];
    let entity = EntityId {
        slot: 99,
        generation: 33,
    };
    NativeCueBinding { entity: None }
        .write_capture(0, &mut stored[8..])
        .unwrap();
    NativeCueBinding {
        entity: Some(entity),
    }
    .write_capture(4, &mut stored[20..])
    .unwrap();
    ring.native_message_bound(native_delivery(NativeTarget::All), 8, &stored)
        .unwrap();
    let mut batch = ring.batch(client).unwrap();
    let FrameEvent::Native(payload) = ring.next(&mut batch).unwrap().event else {
        panic!("native cue payload")
    };
    let mut copy = [0; 32];
    let bytes = ring.texts.copy_payload(payload, &mut copy).unwrap();
    assert_eq!(payload.cue(bytes, 0).unwrap().entity, None);
    assert_eq!(payload.cue(bytes, 4).unwrap().entity, Some(entity));
    assert!(payload.cue(bytes, 1).is_none());
    assert!(payload.cue(bytes, usize::MAX).is_none());
    assert!(payload.cue(&bytes[..31], 4).is_none());
}

#[test]
fn peer_and_cue_publishers_keep_their_own_capture_alignment_and_single_rejection_count() {
    let mut ring = EventRing::load(1, 1, 4, 16).unwrap();
    let _client = ring.bind(OutputTarget::Client(ClientId(0))).unwrap();
    let before = ring.sequence_range();
    assert_eq!(
        ring.native_message_bound(native_delivery(NativeTarget::All), 1, &[0; 17]),
        Err(EventStorageError::TextCapacity)
    );
    assert_eq!(ring.rejected_publications(), 1);
    assert_eq!(
        ring.publish_group(1, |group| {
            group.native_message_parts(native_delivery(NativeTarget::All), b"x", &[0; 16])?;
            Ok(())
        }),
        Err(EventStorageError::TextCapacity)
    );
    assert_eq!(ring.rejected_publications(), 2);
    assert_eq!(
        ring.peer_info(1, b"x", &[0; 12]),
        Err(EventStorageError::TextCapacity)
    );
    assert_eq!(ring.rejected_publications(), 3);
    assert_eq!(ring.sequence_range(), before);
    assert!(ring.is_empty());
}

#[test]
fn file_chunks_target_only_the_selected_native_client_and_preserve_empty_vs_unavailable() {
    let mut ring = EventRing::load(4, 5, 4, 8).unwrap();
    let client = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let peer = ring.bind(OutputTarget::Client(ClientId(2))).unwrap();
    let presentation = ring.bind(OutputTarget::Presentation).unwrap();
    let module = ring.bind(OutputTarget::Module(ModuleId(7))).unwrap();
    let local = ring.bind(OutputTarget::Local(ClientId(1))).unwrap();
    let receiver = EntityId {
        slot: 8,
        generation: 19,
    };
    ring.file_chunk(
        ClientId(1),
        receiver,
        qa_core::primitives::FileChunk::Sequential {
            percent: 23,
            bytes: Some(&[0, 128, 255]),
            compressed: false,
        },
    )
    .unwrap();
    ring.file_chunk(
        ClientId(1),
        receiver,
        qa_core::primitives::FileChunk::Sequential {
            percent: 0,
            bytes: Some(&[]),
            compressed: false,
        },
    )
    .unwrap();
    ring.file_chunk(
        ClientId(1),
        receiver,
        qa_core::primitives::FileChunk::Sequential {
            percent: 0,
            bytes: None,
            compressed: false,
        },
    )
    .unwrap();
    for excluded in [peer, presentation, module, local] {
        assert!(consume(&mut ring, excluded, OutputSubmission::BestEffort).is_empty());
    }
    let mut batch = ring.batch(client).unwrap();
    for (percent, expected) in [
        (23, Some(&[0, 128, 255][..])),
        (0, Some(&[][..])),
        (0, None),
    ] {
        let record = ring.next(&mut batch).unwrap();
        assert_eq!(record.delivery, None);
        let FrameEvent::FileChunk {
            receiver: captured,
            chunk:
                qa_core::primitives::FileChunk::Sequential {
                    percent: actual,
                    bytes,
                    ..
                },
        } = record.event
        else {
            panic!("file chunk");
        };
        assert_eq!(captured, receiver);
        assert_eq!((actual, bytes.is_some()), (percent, expected.is_some()));
        let mut stored = [0; 8];
        assert_eq!(
            bytes.and_then(|payload| ring.texts.copy_payload(payload, &mut stored)),
            expected
        );
        assert!(ring.submit(client, record.sequence, OutputSubmission::BestEffort));
    }
    assert!(ring.is_empty());
    let before = ring.sequence_range();
    assert_eq!(
        ring.file_chunk(
            ClientId(1),
            receiver,
            qa_core::primitives::FileChunk::Sequential {
                percent: 101,
                bytes: Some(b"x"),
                compressed: false
            }
        ),
        Err(EventStorageError::TextCapacity)
    );
    assert_eq!(ring.sequence_range(), before);
}

#[test]
fn numbered_file_payloads_hold_leases_until_the_client_submits() {
    use qa_core::primitives::FileChunk;
    let mut ring = EventRing::load(4, 2, 8, 8).unwrap();
    let client = ring.bind(OutputTarget::Client(ClientId(1))).unwrap();
    let receiver = EntityId {
        slot: 8,
        generation: 19,
    };
    for chunk in [
        FileChunk::Block {
            number: 0,
            total: Some(3),
            bytes: &[0, 128, 255][..],
        },
        FileChunk::Block {
            number: 1,
            total: None,
            bytes: &[],
        },
        FileChunk::Rejected {
            size: -1,
            message: b"no file",
        },
    ] {
        let sequence = ring.file_chunk(ClientId(1), receiver, chunk).unwrap();
        let mut batch = ring.batch(client).unwrap();
        let record = ring.next(&mut batch).unwrap();
        assert_eq!(record.sequence, sequence);
        let FrameEvent::FileChunk {
            receiver: actual,
            chunk: retained,
        } = record.event
        else {
            panic!("file chunk");
        };
        assert_eq!(actual, receiver);
        let mut buffer = [0; 32];
        let payload = *retained.payload().unwrap();
        assert_eq!(
            retained.map(|p| ring.texts.copy_payload(p, &mut buffer).unwrap().to_vec()),
            chunk.map(<[u8]>::to_vec)
        );
        assert!(ring.submit(client, sequence, OutputSubmission::Unsent));
        assert_eq!(
            ring.texts.copy_payload(payload, &mut buffer),
            chunk.payload().copied()
        );
        assert!(ring.submit(client, sequence, OutputSubmission::BestEffort));
        assert!(ring.is_empty());
        if !payload.is_empty() {
            assert!(ring.texts.copy_payload(payload, &mut buffer).is_none());
        }
    }
}
