use super::*;
use qa_core::loopback::Endpoint;
use qa_network::{
    channel::{self, Channel, Delivery, TransmitError},
    commands::packet::{self, Protocol},
    headers::{self, Direction, Header},
};

#[test]
fn full_qw_reliable_queue_retries_only_unsent_userinfo_fields() {
    let mut client = Channel::load(channel::QUAKEWORLD, Endpoint::Client, 64, 8).unwrap();
    let mut server = Channel::load(channel::QUAKEWORLD, Endpoint::Server, 64, 8).unwrap();
    // Native clc_nop bytes occupy the existing queue without changing state.
    client.queue_reliable(&[1; 48]).unwrap();
    client.queue_reliable(&[1; 48]).unwrap();
    let mut cache = ClientUserinfo::default();
    cache.queued.set_bytes(b"\\name\\old").unwrap();
    let desired = b"\\name\\new\\skin\\base";
    assert_eq!(
        cache.queue_qw(&mut client, desired),
        Err(TransmitError::Full)
    );
    assert_eq!(cache.queued.as_bytes(), b"\\name\\new");
    let mut commands = Vec::new();
    for time in 1..=8 {
        let now = EventTime(time);
        let Some(packet) = client.prepare(None, now).unwrap() else {
            break;
        };
        let packet = packet.bytes.to_vec();
        client.submitted(now).unwrap();
        let received = server.receive(&packet, now).unwrap();
        let acknowledgement = received.header.sequence;
        if let Delivery::Payload(body) = received.delivery {
            let mut body = body.to_vec();
            packet::read_with_commands(
                Protocol::QuakeWorld28,
                &mut body,
                received.header.sequence,
                Default::default(),
                |record| {
                    if let packet::Record::String { text, .. } = record {
                        commands.push(text.to_vec());
                    }
                    Ok(std::ops::ControlFlow::Continue(()))
                },
            )
            .unwrap();
        }
        let mut ack = [0; 16];
        let length = headers::encode(
            headers::QUAKEWORLD,
            Direction::ToClient,
            Header {
                sequence: time as u32,
                acknowledgement,
                reliable_ack: server.state().reliable_sequence,
                ..Default::default()
            },
            &[],
            &mut ack,
        )
        .unwrap();
        client.receive(&ack[..length], now).unwrap();
        cache.queue_qw(&mut client, desired).unwrap();
    }
    assert_eq!(cache.queued.as_bytes(), desired);
    assert_eq!(
        commands,
        [
            b"setinfo \"name\" \"new\"".to_vec(),
            b"setinfo \"skin\" \"base\"".to_vec()
        ]
    );
    assert_eq!(client.send_state().reliable_bytes, 0);
}

#[test]
fn qw_userinfo_shrinks_before_growing_within_the_native_total() {
    let mut channel = Channel::load(channel::QUAKEWORLD, Endpoint::Client, 2048, 8).unwrap();
    let mut cache = ClientUserinfo::default();
    let mut old = qa_core::text::FixedText::<196>::default();
    let mut desired = qa_core::text::FixedText::<196>::default();
    let rules = qa_compat::services::userinfo_rules(RuleSetId::QuakeWorld);
    for (key, before, after) in [
        (b"one".as_slice(), 1, 63),
        (b"two", 63, 1),
        (b"three", 63, 63),
        (b"four", 32, 32),
    ] {
        old.update_info(qa_core::text::InfoUpdate::new(key, &vec![b'a'; before], rules).unwrap())
            .unwrap();
        desired
            .update_info(qa_core::text::InfoUpdate::new(key, &vec![b'b'; after], rules).unwrap())
            .unwrap();
    }
    cache.queued.set_bytes(old.as_bytes()).unwrap();
    cache.queue_qw(&mut channel, desired.as_bytes()).unwrap();
    assert_eq!(cache.queued.as_bytes(), desired.as_bytes());
}
