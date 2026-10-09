use qa_core::{events::NativeReceipt, loopback::Endpoint, sys_events::EventTime};
use qa_network::{
    channel::{self, Channel, Delivery, TransmitError},
    headers::{self, Direction, Fragment, Header, datagram},
};

fn send(channel: &mut Channel, body: Option<&[u8]>, time: u64) -> Vec<u8> {
    let packet = channel
        .prepare(body, EventTime(time))
        .expect("prepare")
        .expect("packet")
        .bytes
        .to_vec();
    assert!(packet.len() <= 1400);
    channel
        .submitted(EventTime(time))
        .expect("transport admitted");
    packet
}
fn packet(format: headers::Format, direction: Direction, header: Header, body: &[u8]) -> Vec<u8> {
    let mut packet = vec![0; body.len() + 16];
    let length =
        headers::encode(format, direction, header, body, &mut packet).expect("native packet");
    packet.truncate(length);
    packet
}

#[test]
fn rejected_transport_retains_packet_and_cannot_retire_a_flight() {
    let mut channel = Channel::load(channel::QUAKE2, Endpoint::Client, 1400, 8).expect("load");
    let receipt = channel.queue_reliable(b"command").expect("queue");
    let prepared = channel
        .prepare(Some(b"unreliable"), EventTime(0))
        .expect("prepare")
        .expect("packet")
        .bytes
        .to_vec();
    assert_eq!(channel.send_state().sequence, 1);
    assert_eq!(
        channel.pending_packet().expect("retry packet").bytes,
        prepared
    );
    assert!(matches!(
        channel.prepare(None, EventTime(2)),
        Err(TransmitError::PendingPacket)
    ));
    let ack = packet(
        headers::QUAKE2,
        Direction::ToClient,
        Header {
            sequence: 1,
            acknowledgement: 1,
            reliable_ack: true,
            ..Header::default()
        },
        b"",
    );
    channel.receive(&ack, EventTime(1)).expect("receive");
    assert!(channel.reliable_receipts().is_empty());
    assert_eq!(channel.send_state().reliable_bytes, 7);
    channel.submitted(EventTime(2)).expect("admitted");
    let ack = packet(
        headers::QUAKE2,
        Direction::ToClient,
        Header {
            sequence: 2,
            acknowledgement: 1,
            reliable_ack: true,
            ..Header::default()
        },
        b"",
    );
    channel.receive(&ack, EventTime(3)).expect("receive");
    assert_eq!(channel.reliable_receipts(), &[receipt]);
    channel.receive(&ack, EventTime(4)).expect("duplicate");
    assert!(channel.reliable_receipts().is_empty());
}

#[test]
fn netquake_timer_ignores_unreliable_sends_and_retry_keeps_sequence() {
    let mut channel = Channel::load(channel::NETQUAKE, Endpoint::Client, 8192, 8).expect("load");
    channel.queue_reliable(&[7; 2500]).expect("queue");
    let first = send(&mut channel, None, 0);
    assert_eq!(channel.send_state().sequence, 1);
    let datagram = send(&mut channel, Some(b"datagram"), 900_000_000);
    assert_ne!(first, datagram);
    assert!(
        channel
            .prepare(None, EventTime(1_000_000_000))
            .expect("not due")
            .is_none()
    );
    let retry = send(&mut channel, None, 1_000_000_001);
    assert_eq!(retry, first);
    assert_eq!(channel.send_state().sequence, 1);
    assert_eq!(channel.send_state().resends, 1);
    assert_eq!(channel.send_state().datagram_sequence, 1);
}

#[test]
fn netquake_retires_all_batched_records_only_after_final_fragment_ack() {
    let mut sender = Channel::load(channel::NETQUAKE, Endpoint::Client, 8192, 8).expect("load");
    let mut receiver = Channel::load(channel::NETQUAKE, Endpoint::Server, 8192, 8).expect("load");
    let receipts = [
        sender.queue_reliable(&[1; 1200]).expect("queue"),
        sender.queue_reliable(&[2; 1800]).expect("queue"),
    ];
    for part in 0..3 {
        let data = send(&mut sender, None, part);
        let received = receiver.receive(&data, EventTime(part)).expect("receive");
        if part < 2 {
            assert_eq!(received.delivery, Delivery::Pending);
        } else if let Delivery::Payload(body) = received.delivery {
            assert_eq!(&body[..1200], &[1; 1200]);
            assert_eq!(&body[1200..], &[2; 1800]);
        } else {
            panic!("final native payload absent");
        }
        let ack = send(&mut receiver, None, part);
        sender.receive(&ack, EventTime(part)).expect("native ack");
        if part < 2 {
            assert!(sender.reliable_receipts().is_empty());
        } else {
            assert_eq!(sender.reliable_receipts(), &receipts);
        }
        sender
            .receive(&ack, EventTime(part))
            .expect("duplicate ack");
        assert!(sender.reliable_receipts().is_empty());
    }
    assert_eq!(sender.send_state().ack_sequence, 3);
    assert_eq!(sender.send_state().reliable_bytes, 0);
}

#[test]
fn toggle_resend_threshold_preserves_classic_and_q2pro_bias() {
    for (policy, format, ack, should_resend) in [
        (channel::QUAKEWORLD, headers::QUAKEWORLD, 2, false),
        (channel::QUAKE2, headers::QUAKE2, 3, true),
        (
            channel::q2_old(headers::QPort::Short),
            headers::QUAKE2,
            2,
            true,
        ),
    ] {
        let mut channel = Channel::load(policy, Endpoint::Client, 1400, 8).expect("load");
        channel.queue_reliable(b"r").expect("queue");
        send(&mut channel, None, 0);
        let missed = packet(
            format,
            Direction::ToClient,
            Header {
                sequence: 1,
                acknowledgement: ack,
                ..Header::default()
            },
            b"",
        );
        channel.receive(&missed, EventTime(1)).expect("receive");
        let data = send(&mut channel, None, 1);
        let (header, body) = headers::decode(format, Direction::ToServer, &data).expect("decode");
        assert_eq!(header.reliable, should_resend);
        assert_eq!(body, if should_resend { &b"r"[..] } else { &b""[..] });
    }
}

#[test]
fn q2pro_pending_fragment_ack_can_retire_a_transmitted_flight() {
    let mut sender =
        Channel::load(channel::q2_new(false), Endpoint::Client, 32768, 8).expect("load");
    let receipt = sender.queue_reliable(b"r").expect("queue");
    send(&mut sender, None, 0);
    let ack = packet(
        headers::q2_new(false),
        Direction::ToClient,
        Header {
            sequence: 1,
            reliable_ack: true,
            fragment: Some(Fragment {
                offset: 0,
                more: true,
            }),
            ..Header::default()
        },
        b"prefix",
    );
    assert_eq!(
        sender
            .receive(&ack, EventTime(1))
            .expect("receive")
            .delivery,
        Delivery::Pending
    );
    assert_eq!(sender.reliable_receipts(), &[receipt]);
    assert_eq!(sender.send_state().reliable_bytes, 0);
}

#[test]
fn q2pro_fragment_flag_preserves_existing_flight_without_copying_it() {
    let mut channel =
        Channel::load(channel::q2_new(false), Endpoint::Server, 32768, 8).expect("load");
    channel.queue_reliable(b"r").expect("queue");
    send(&mut channel, None, 0);
    let previous = channel.send_state().last_reliable_sequence;
    let data = send(&mut channel, Some(&[5; 2600]), 1);
    let (header, body) =
        headers::decode(headers::q2_new(false), Direction::ToClient, &data).expect("decode");
    assert!(header.reliable);
    assert_eq!(
        header.fragment,
        Some(Fragment {
            offset: 0,
            more: true
        })
    );
    assert_eq!(body, &[5; 1300]);
    assert_eq!(channel.send_state().last_reliable_sequence, previous);
}

#[test]
fn q3_fragmentation_has_no_header_receipt_and_an_empty_final_packet() {
    let mut sender = Channel::load(channel::QUAKE3, Endpoint::Client, 16384, 8).expect("load");
    let mut receiver = Channel::load(channel::QUAKE3, Endpoint::Server, 16384, 8).expect("load");
    sender.set_qport(37);
    assert_eq!(sender.command_state().expect("command state").queued, 0);
    for part in 0..3 {
        let data = send(
            &mut sender,
            if part == 0 { Some(&[3; 2600]) } else { None },
            part,
        );
        let (header, body) =
            headers::decode(headers::QUAKE3, Direction::ToServer, &data).expect("decode");
        assert_eq!(header.sequence, 1);
        assert_eq!(body.len(), if part < 2 { 1300 } else { 0 });
        let received = receiver.receive(&data, EventTime(part)).expect("receive");
        if part < 2 {
            assert_eq!(received.delivery, Delivery::Pending);
        } else {
            assert_eq!(received.delivery, Delivery::Payload(&[3; 2600]));
        }
        assert!(sender.reliable_receipts().is_empty());
    }
    assert_eq!(sender.send_state().sequence, 2);
}

#[test]
fn reliable_queue_is_bounded_and_overflow_does_not_replace_records() {
    let mut channel = Channel::load(channel::QUAKE2, Endpoint::Server, 1400, 8).expect("load");
    for index in 1..=64 {
        assert_eq!(
            channel.queue_reliable(&[index]),
            Ok(NativeReceipt(u64::from(index)))
        );
    }
    assert_eq!(
        channel.queue_reliable(b"overflow"),
        Err(TransmitError::Full)
    );
    let data = send(&mut channel, Some(&[7; 1400]), 0);
    let (header, body) =
        headers::decode(headers::QUAKE2, Direction::ToClient, &data).expect("decode");
    assert!(header.reliable);
    assert_eq!(body, &(1..=64).collect::<Vec<_>>());
    assert_eq!(channel.send_state().unreliable_drops, 1);
    let ack = packet(
        headers::QUAKE2,
        Direction::ToServer,
        Header {
            sequence: 1,
            reliable_ack: true,
            ..Header::default()
        },
        b"",
    );
    channel.receive(&ack, EventTime(1)).expect("receive");
    assert_eq!(channel.reliable_receipts().len(), 64);
    assert_eq!(channel.reliable_receipts()[63], NativeReceipt(64));
}

#[test]
fn ack_during_a_rejected_resend_does_not_create_an_empty_flight() {
    let mut channel = Channel::load(channel::NETQUAKE, Endpoint::Client, 8192, 8).expect("load");
    let receipt = channel.queue_reliable(b"r").expect("queue");
    send(&mut channel, None, 0);
    let retry = channel
        .prepare(None, EventTime(1_100_000_000))
        .expect("prepare")
        .expect("resend")
        .bytes
        .to_vec();
    let ack = packet(
        headers::NETQUAKE,
        Direction::ToClient,
        Header {
            datagram_flags: datagram::ACK,
            ..Header::default()
        },
        b"",
    );
    channel
        .receive(&ack, EventTime(1_200_000_000))
        .expect("ack");
    assert_eq!(channel.reliable_receipts(), &[receipt]);
    assert_eq!(
        channel.pending_packet().expect("pending retry").bytes,
        retry
    );
    channel
        .submitted(EventTime(1_300_000_000))
        .expect("retry admitted");
    assert!(
        channel
            .prepare(None, EventTime(2_400_000_000))
            .expect("idle")
            .is_none()
    );
    assert_eq!(channel.send_state().reliable_bytes, 0);
}

#[test]
fn batched_fragment_message_respects_native_offset_width() {
    let mut channel =
        Channel::load(channel::q2_new(false), Endpoint::Server, 65536, 8).expect("load");
    channel.queue_reliable(&[1; 20000]).expect("queue first");
    channel.queue_reliable(&[2; 20000]).expect("queue second");
    let data = send(&mut channel, Some(&[3; 20000]), 0);
    let (header, body) =
        headers::decode(headers::q2_new(false), Direction::ToClient, &data).expect("decode");
    assert_eq!(
        header.fragment,
        Some(Fragment {
            offset: 0,
            more: true
        })
    );
    assert_eq!(body, &[1; 1300]);
    assert_eq!(channel.send_state().fragment_bytes, 20000);
    assert_eq!(channel.send_state().reliable_bytes, 20000);
    assert_eq!(channel.send_state().unreliable_drops, 1);
}

#[test]
fn empty_and_oversize_messages_fail_before_packet_preparation() {
    let mut channel = Channel::load(channel::QUAKE3, Endpoint::Server, 16384, 8).expect("load");
    assert!(matches!(
        channel.prepare(Some(&[0; 16385]), EventTime(0)),
        Err(TransmitError::MessageTooLarge)
    ));
    assert!(channel.pending_packet().is_none());
    assert_eq!(channel.send_state().sequence, 1);
    let mut nq = Channel::load(channel::NETQUAKE, Endpoint::Server, 8192, 8).expect("load");
    assert_eq!(nq.queue_reliable(b""), Err(TransmitError::MessageTooLarge));
    let data = send(&mut nq, Some(b""), 0);
    let (header, body) =
        headers::decode(headers::NETQUAKE, Direction::ToClient, &data).expect("decode");
    assert_eq!(header.datagram_flags, datagram::UNRELIABLE);
    assert!(body.is_empty());
    assert_eq!(nq.send_state().unreliable_drops, 0);
}
