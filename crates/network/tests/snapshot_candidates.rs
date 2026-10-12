use qa_core::{loopback::Endpoint, primitives::ThinkTime};
use qa_network::{
    channel::Channel,
    commands::packet::{Error, Protocol},
    message::{Encoding, Writer},
    snapshots::{Entity, Frame, Q2EntityPolicy, ReceivedFrame},
    states,
};
fn candidate(
    protocol: Protocol,
    frame: ReceivedFrame<'_>,
    encoding: Encoding,
) -> Result<(), Error> {
    let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16)
        .map_err(|_| Error::Context)?;
    channel.configure_snapshots(protocol)?;
    let mut small = [0; 1];
    assert!(
        channel
            .write_snapshot_candidate(&mut Writer::new(&mut small, encoding), frame, None, 32)
            .is_err()
    );
    assert!(channel.current_snapshot().is_none());
    assert!(channel.previous_snapshot().is_none());
    assert!(channel.snapshot(frame.sequence()).is_none());
    let mut before = [0; 1400];
    let mut writer = Writer::new(&mut before, encoding);
    channel.write_snapshot_candidate(&mut writer, frame, None, 32)?;
    let before = writer.bytes().to_vec();
    assert!(channel.current_snapshot().is_none());
    channel.publish_written_snapshot(frame, None)?;
    let mut after = [0; 1400];
    let mut writer = Writer::new(&mut after, encoding);
    channel.write_snapshot(&mut writer, frame.sequence(), None, 32)?;
    assert_eq!(writer.bytes(), before);
    assert_eq!(
        channel.current_snapshot().map(|frame| frame.sequence()),
        Some(frame.sequence())
    );
    Ok(())
}
#[test]
fn native_candidate_serialization_never_publishes_failed_trials() -> Result<(), Error> {
    let nq = [Entity {
        number: 40,
        words: [0; states::NQ_ENTITY_WORDS],
    }];
    candidate(
        Protocol::NetQuake15,
        ReceivedFrame::NetQuake(Frame {
            sequence: 1,
            time: ThinkTime::Seconds(1.25),
            command: 0,
            flags: 0,
            qw_peers: 0,
            areas: &[],
            player: &[0; states::NQ_PLAYER_WORDS],
            entities: &nq,
        }),
        Encoding::Bytes,
    )?;
    let qw = [Entity {
        number: 40,
        words: [0; states::QW_ENTITY_WORDS],
    }];
    candidate(
        Protocol::QuakeWorld28,
        ReceivedFrame::QuakeWorld(Frame {
            sequence: 1,
            time: ThinkTime::Seconds(1.25),
            command: 0,
            flags: 0,
            qw_peers: 0,
            areas: &[],
            player: &[],
            entities: &qw,
        }),
        Encoding::Bytes,
    )?;
    let q2 = [Entity {
        number: 40,
        words: [0; states::Q2_ENTITY_WORDS],
    }];
    candidate(
        Protocol::Quake2_34,
        ReceivedFrame::Quake2(Frame {
            sequence: 1,
            time: ThinkTime::Milliseconds(100),
            command: 0,
            flags: 0,
            qw_peers: 0,
            areas: &[],
            player: &[0; states::Q2_PLAYER_WORDS],
            entities: &q2,
        }),
        Encoding::Bytes,
    )?;
    let rr = [Entity {
        number: 40,
        words: [0; states::Q2_RERELEASE_ENTITY_WORDS],
    }];
    candidate(
        Protocol::Quake2Repro1038,
        ReceivedFrame::Quake2Repro(Frame {
            sequence: 1,
            time: ThinkTime::Milliseconds(25),
            command: 0,
            flags: 0,
            qw_peers: 0,
            areas: &[],
            player: &[0; states::Q2_REPRO_PLAYER_WORDS],
            entities: &rr,
        }),
        Encoding::Bytes,
    )?;
    let q3 = [Entity {
        number: 40,
        words: [0; states::ENTITY_WORDS],
    }];
    candidate(
        Protocol::Quake3_68,
        ReceivedFrame::Quake3(Frame {
            sequence: 1,
            time: ThinkTime::Milliseconds(50),
            command: 0,
            flags: 0,
            qw_peers: 0,
            areas: &[],
            player: &[0; states::PLAYER_WORDS],
            entities: &q3,
        }),
        Encoding::Q3,
    )?;
    Ok(())
}
#[test]
fn rerelease_written_candidate_keeps_the_sent_first_person_pose_in_history() -> Result<(), Error> {
    let mut channel = Channel::load(
        Protocol::Quake2Repro1038.channel(),
        Endpoint::Server,
        8192,
        16,
    )
    .map_err(|_| Error::Context)?;
    channel.configure_snapshots(Protocol::Quake2Repro1038)?;
    channel.configure_q2_repro(
        25,
        Q2EntityPolicy {
            first_person: Some(1),
            ..Default::default()
        },
    )?;
    let mut old = [Entity {
        number: 1,
        words: [0; states::Q2_RERELEASE_ENTITY_WORDS],
    }];
    old[0].words[0] = 5;
    old[0].words[8] = 100.0f32.to_bits();
    old[0].words[11] = 8192;
    let frame = |sequence, entities| {
        ReceivedFrame::Quake2Repro(Frame {
            sequence,
            time: ThinkTime::Milliseconds(i64::from(sequence) * 25),
            command: 0,
            flags: 0,
            qw_peers: 0,
            areas: &[],
            player: &[0; states::Q2_REPRO_PLAYER_WORDS],
            entities,
        })
    };
    channel.publish_snapshot(frame(1, &old))?;
    let mut new = old;
    new[0].words[8] = 200.0f32.to_bits();
    new[0].words[11] = 16384;
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    channel.write_snapshot_candidate(&mut writer, frame(2, &new), Some(1), 1)?;
    let sent = writer.bytes().to_vec();
    assert!(channel.snapshot(2).is_none());
    assert_eq!(new[0].words[8], 200.0f32.to_bits());
    channel.publish_written_snapshot(frame(2, &new), Some(1))?;
    let Some(ReceivedFrame::Quake2Repro(retained)) = channel.snapshot(2) else {
        panic!("retained frame")
    };
    assert_eq!(&retained.entities[0].words[8..14], &old[0].words[8..14]);
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    channel.write_snapshot(&mut writer, 2, Some(1), 1)?;
    assert_eq!(writer.bytes(), sent);
    Ok(())
}
#[test]
fn maximum_snapshot_body_uses_the_native_channel_limit() {
    for (protocol, expected) in [
        (Protocol::NetQuake15, 1024),
        (Protocol::QuakeWorld28, 1392),
        (Protocol::Quake2_34, 1392),
        (Protocol::Quake2Repro1038, 8192),
        (Protocol::Quake3_68, 8192),
    ] {
        let channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
        assert_eq!(channel.snapshot_payload_capacity().unwrap(), expected);
    }
}
