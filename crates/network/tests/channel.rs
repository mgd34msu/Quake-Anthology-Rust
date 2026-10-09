use qa_core::{loopback::Endpoint, sys_events::EventTime};
use qa_network::{
    channel::{self, Channel, Delivery, Error},
    headers::{self, Direction, Fragment, Header, datagram},
};

fn packet(format: headers::Format, header: Header, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; body.len() + 16];
    let length = headers::encode(format, Direction::ToClient, header, body, &mut bytes)
        .expect("native test packet");
    bytes.truncate(length);
    bytes
}
fn receive<'a>(channel: &'a mut Channel, packet: &'a [u8]) -> Delivery<'a> {
    channel
        .receive(packet, EventTime(1))
        .expect("receive")
        .delivery
}

#[test]
fn netquake_sequences_are_independent_and_duplicates_still_get_acks() {
    let mut channel = Channel::load(channel::NETQUAKE, Endpoint::Client, 8192, 8).expect("load");
    let reliable = packet(
        headers::NETQUAKE,
        Header {
            sequence: 0,
            datagram_flags: datagram::DATA,
            ..Header::default()
        },
        b"first",
    );
    assert_eq!(receive(&mut channel, &reliable), Delivery::Pending);
    assert_eq!(receive(&mut channel, &reliable), Delivery::Ignored);
    let datagram = packet(
        headers::NETQUAKE,
        Header {
            sequence: 7,
            datagram_flags: datagram::UNRELIABLE,
            ..Header::default()
        },
        b"unreliable",
    );
    assert_eq!(
        receive(&mut channel, &datagram),
        Delivery::Payload(b"unreliable")
    );
    let final_packet = packet(
        headers::NETQUAKE,
        Header {
            sequence: 1,
            datagram_flags: datagram::DATA | datagram::EOM,
            ..Header::default()
        },
        b"last",
    );
    assert_eq!(
        receive(&mut channel, &final_packet),
        Delivery::Payload(b"firstlast")
    );
    assert_eq!(channel.state().sequence, 2);
    assert_eq!(channel.state().next_datagram, 8);
    for sequence in [0, 0, 1] {
        assert_eq!(
            channel.next_control(),
            Some(Header {
                sequence,
                datagram_flags: datagram::ACK,
                ..Header::default()
            })
        );
    }
    assert_eq!(channel.next_control(), None);
}

#[test]
fn full_ack_queue_retains_receiver_position_for_retry() {
    let mut channel = Channel::load(channel::NETQUAKE, Endpoint::Client, 64, 1).expect("load");
    let first = packet(
        headers::NETQUAKE,
        Header {
            datagram_flags: datagram::DATA | datagram::EOM,
            ..Header::default()
        },
        b"a",
    );
    let second = packet(
        headers::NETQUAKE,
        Header {
            sequence: 1,
            datagram_flags: datagram::DATA | datagram::EOM,
            ..Header::default()
        },
        b"b",
    );
    assert_eq!(receive(&mut channel, &first), Delivery::Payload(b"a"));
    assert_eq!(
        channel.receive(&second, EventTime(2)),
        Err(Error::ControlFull)
    );
    assert_eq!(channel.state().sequence, 1);
    assert_eq!(channel.counts().control_full, 1);
    channel.next_control();
    assert_eq!(receive(&mut channel, &second), Delivery::Payload(b"b"));
}

#[test]
fn toggle_ack_is_observed_only_on_a_new_sequence() {
    let mut channel = Channel::load(channel::QUAKEWORLD, Endpoint::Client, 1450, 1).expect("load");
    let accepted = packet(
        headers::QUAKEWORLD,
        Header {
            sequence: 3,
            acknowledgement: 8,
            reliable: true,
            reliable_ack: true,
            ..Header::default()
        },
        b"x",
    );
    assert_eq!(receive(&mut channel, &accepted), Delivery::Payload(b"x"));
    let state = channel.state();
    let duplicate = packet(
        headers::QUAKEWORLD,
        Header {
            sequence: 3,
            acknowledgement: 99,
            ..Header::default()
        },
        b"y",
    );
    assert_eq!(receive(&mut channel, &duplicate), Delivery::Ignored);
    assert_eq!(channel.state(), state);
    assert!(state.reliable_sequence && state.reliable_acknowledged);
    assert_eq!(state.acknowledged, 8);
    assert_eq!(state.dropped, 2);
}

#[test]
fn q2pro_pending_fragments_observe_native_ack_before_completion() {
    let mut channel =
        Channel::load(channel::q2_new(false), Endpoint::Client, 8192, 1).expect("load");
    let first = packet(
        headers::q2_new(false),
        Header {
            sequence: 2,
            acknowledgement: 7,
            reliable: true,
            reliable_ack: true,
            fragment: Some(Fragment {
                offset: 0,
                more: true,
            }),
            ..Header::default()
        },
        b"prefix",
    );
    assert_eq!(receive(&mut channel, &first), Delivery::Pending);
    assert!(channel.state().reliable_acknowledged);
    assert_eq!(channel.state().acknowledged, 0);
    assert!(!channel.state().reliable_sequence);
    let wrong = packet(
        headers::q2_new(false),
        Header {
            sequence: 2,
            fragment: Some(Fragment {
                offset: 7,
                more: false,
            }),
            ..Header::default()
        },
        b"suffix",
    );
    assert_eq!(receive(&mut channel, &wrong), Delivery::Ignored);
    assert!(!channel.state().reliable_acknowledged);
    assert_eq!(channel.state().fragment_bytes, 6);
    let last = packet(
        headers::q2_new(false),
        Header {
            sequence: 2,
            acknowledgement: 9,
            reliable: true,
            fragment: Some(Fragment {
                offset: 6,
                more: false,
            }),
            ..Header::default()
        },
        b"suffix",
    );
    assert_eq!(
        receive(&mut channel, &last),
        Delivery::Payload(b"prefixsuffix")
    );
    assert_eq!(channel.state().sequence, 2);
    assert_eq!(channel.state().acknowledged, 9);
    assert!(channel.state().reliable_sequence);
}

#[test]
fn q3_exact_fragment_multiple_needs_empty_final_fragment() {
    let mut channel = Channel::load(channel::QUAKE3, Endpoint::Client, 16384, 1).expect("load");
    let first = packet(
        headers::QUAKE3,
        Header {
            sequence: 1,
            fragment: Some(Fragment {
                offset: 0,
                more: true,
            }),
            ..Header::default()
        },
        &[3; 1300],
    );
    assert_eq!(receive(&mut channel, &first), Delivery::Pending);
    assert_eq!(receive(&mut channel, &first), Delivery::Ignored);
    let last = packet(
        headers::QUAKE3,
        Header {
            sequence: 1,
            fragment: Some(Fragment {
                offset: 1300,
                more: false,
            }),
            ..Header::default()
        },
        b"",
    );
    assert_eq!(receive(&mut channel, &last), Delivery::Payload(&[3; 1300]));
    assert_eq!(channel.state().sequence, 1);
    assert_eq!(channel.state().acknowledged, 0);
    assert_eq!(channel.counts().fragment_order, 1);
}

#[test]
fn oversize_fragment_is_scoped_and_preserves_prefix() {
    let mut channel = Channel::load(channel::q2_new(false), Endpoint::Client, 8, 1).expect("load");
    let first = packet(
        headers::q2_new(false),
        Header {
            sequence: 1,
            fragment: Some(Fragment {
                offset: 0,
                more: true,
            }),
            ..Header::default()
        },
        b"prefix",
    );
    assert_eq!(receive(&mut channel, &first), Delivery::Pending);
    let oversized = packet(
        headers::q2_new(false),
        Header {
            sequence: 1,
            fragment: Some(Fragment {
                offset: 6,
                more: false,
            }),
            ..Header::default()
        },
        b"suffix",
    );
    assert_eq!(
        channel.receive(&oversized, EventTime(1)),
        Err(Error::MessageTooLarge)
    );
    assert_eq!(channel.state().fragment_bytes, 6);
    let last = packet(
        headers::q2_new(false),
        Header {
            sequence: 1,
            fragment: Some(Fragment {
                offset: 6,
                more: false,
            }),
            ..Header::default()
        },
        b"ok",
    );
    assert_eq!(receive(&mut channel, &last), Delivery::Payload(b"prefixok"));
    assert_eq!(channel.counts().malformed, 1);
}
