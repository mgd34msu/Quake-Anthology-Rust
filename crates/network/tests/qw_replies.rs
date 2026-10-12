use qa_core::{loopback::Endpoint, primitives::UserCmd, sys_events::EventTime};
use qa_network::{
    channel::{self, Channel, Delivery, Unreliable},
    commands::{connection::Commands, packet::Protocol},
};

fn request(client: &mut Channel, server: &mut Channel, codec: &mut Commands) -> Vec<u8> {
    let mut body = [0; 1400];
    let writer = Commands::load(Protocol::QuakeWorld28, 8192);
    let length = writer
        .encode(&UserCmd::default(), 0, client, &mut body)
        .unwrap();
    let packet = client
        .prepare_move(&body[..length], EventTime(0), None)
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    client.submitted(EventTime(0)).unwrap();
    let received = server.receive(&packet, EventTime(0)).unwrap();
    let sequence = received.header.sequence;
    let Delivery::Payload(body) = received.delivery else {
        panic!("native request")
    };
    let length = codec.stage(body).unwrap();
    assert_eq!(
        codec
            .decode(length, sequence, 0, server, None, |_| {
                std::ops::ControlFlow::Continue(())
            })
            .unwrap(),
        1
    );
    packet
}

#[test]
fn qw_server_grants_one_reply_per_native_request_and_keeps_rejected_submission_pending() {
    let mut server = Channel::load(channel::QUAKEWORLD, Endpoint::Server, 8192, 16).unwrap();
    let mut client = Channel::load(channel::QUAKEWORLD, Endpoint::Client, 8192, 16).unwrap();
    let mut codec = Commands::load(Protocol::QuakeWorld28, 8192);
    let receipt = server.queue_reliable(b"retained").unwrap();
    assert!(server.prepare_output(EventTime(0)).unwrap().is_none());
    assert!(
        server
            .prepare(Some(b"unsolicited"), EventTime(0))
            .unwrap()
            .is_none()
    );
    assert_eq!(server.send_state().packets, 0);

    request(&mut client, &mut server, &mut codec);
    let reply = server
        .prepare_output(EventTime(1))
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    assert_eq!(server.submit_with(EventTime(2), |_| false).unwrap(), None);
    assert_eq!(server.pending_packet().unwrap().bytes, reply);
    assert!(server.reliable_receipts().is_empty());
    assert_eq!(server.send_state().packets, 0);
    assert_eq!(
        server.submit_with(EventTime(3), |_| true).unwrap(),
        Some(Unreliable::Included)
    );
    assert_eq!(server.send_state().packets, 1);
    assert!(server.prepare_output(EventTime(4)).unwrap().is_none());
    assert!(
        server
            .prepare(Some(b"extra snapshot"), EventTime(4))
            .unwrap()
            .is_none()
    );
    assert!(server.reliable_receipts().is_empty());

    assert!(matches!(
        client.receive(&reply, EventTime(5)).unwrap().delivery,
        Delivery::Payload(b"retained")
    ));
    let second = request(&mut client, &mut server, &mut codec);
    assert_eq!(server.reliable_receipts(), &[receipt]);
    assert_eq!(server.send_state().reliable_bytes, 0);
    assert!(
        server
            .prepare(Some(b"next reply"), EventTime(6))
            .unwrap()
            .is_some()
    );
    server.submitted(EventTime(6)).unwrap();
    assert!(server.prepare_output(EventTime(7)).unwrap().is_none());
    assert!(matches!(
        server.receive(&second, EventTime(8)).unwrap().delivery,
        Delivery::Ignored
    ));
    assert!(server.prepare_output(EventTime(8)).unwrap().is_none());
    assert!(server.reliable_receipts().is_empty());
}

#[test]
fn classic_q2_keeps_unrequested_transmit_and_its_reliable_bias() {
    let mut channel = Channel::load(channel::QUAKE2, Endpoint::Server, 8192, 16).unwrap();
    channel.queue_reliable(b"reliable").unwrap();
    assert!(channel.prepare_output(EventTime(0)).unwrap().is_some());
    channel.submitted(EventTime(0)).unwrap();
    assert_eq!(channel.send_state().last_reliable_sequence, 2);
    assert!(channel.prepare_output(EventTime(1)).unwrap().is_some());
}
