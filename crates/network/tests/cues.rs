use qa_core::{
    loopback::Endpoint,
    primitives::ClientId,
    sys_events::{EventTime, Peer},
};
use qa_network::{
    channel::Channel,
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection, Connections, Incoming, Route},
    outputs::{self, Cue, MuzzleFormat},
};

fn exact(protocol: Protocol, bytes: &[u8]) -> Cue {
    let (cue, consumed) = outputs::read_cue(protocol, bytes).unwrap();
    assert_eq!(consumed, bytes.len());
    let mut output = [0xcc; 64];
    let count = outputs::write_cue(protocol, cue, &mut output).unwrap();
    assert_eq!(&output[..count], bytes);
    assert_eq!(output[count], 0xcc);
    for cut in 0..bytes.len() {
        assert!(outputs::read_cue(protocol, &bytes[..cut]).is_err());
    }
    cue
}

#[test]
fn native_sound_defaults_and_present_parameters_keep_the_original_byte_order() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let Cue::Sound(sound) = exact(protocol, &[9, 0, 49]) else {
            panic!("sound");
        };
        assert_eq!(
            (sound.volume, sound.attenuation, sound.time_offset),
            (255, 64, 0)
        );
        assert_eq!((sound.entity, sound.channel), (0, 0));
    }
    // svc_sound flags/index/volume/attenuation/timeofs/entity-channel/position.
    let bytes = [9, 31, 49, 204, 128, 125, 41, 0, 247, 255, 162, 0, 1, 4];
    let Cue::Sound(sound) = exact(Protocol::Quake2_34, &bytes) else {
        panic!("sound");
    };
    assert_eq!(
        (sound.volume, sound.attenuation, sound.time_offset),
        (204, 128, 125)
    );
    assert_eq!((sound.entity, sound.channel), (5, 1));
    assert_eq!(sound.position.0, [-1.125, 20.25, 128.125]);
}

#[test]
fn rerelease_sound_uses_float_coordinates_and_the_negotiated_large_index() {
    let mut bytes = vec![9, 63, 1, 4, 51, 64, 200, 255, 255];
    let position = [-1.125f32, 20.25, 128.125];
    for value in position {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let Cue::Sound(sound) = exact(Protocol::Quake2Repro1038, &bytes) else {
        panic!("sound");
    };
    assert_eq!(sound.index, 1025);
    assert_eq!((sound.entity, sound.channel), (8191, 7));
    assert_eq!(
        sound.position.0.map(f32::to_bits),
        position.map(f32::to_bits)
    );
    assert!(outputs::read_cue(Protocol::Quake2_34, &bytes).is_err());
    assert!(outputs::write_cue(Protocol::Quake2_34, Cue::Sound(sound), &mut [0; 32]).is_err());
    assert!(outputs::read_cue(Protocol::Quake2Repro1038, &[9, 64, 1]).is_err());
}

#[test]
fn player_silencing_and_both_monster_index_encodings_are_preserved() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let Cue::MuzzleFlash(muzzle) = exact(protocol, &[1, 1, 0, 128]) else {
            panic!("muzzle");
        };
        assert_eq!(muzzle.format, MuzzleFormat::Player);
        assert_eq!((muzzle.entity, muzzle.flash, muzzle.silenced), (1, 0, true));
        let Cue::MuzzleFlash(muzzle) = exact(protocol, &[2, 7, 0, 255]) else {
            panic!("muzzle");
        };
        assert_eq!(
            (muzzle.entity, muzzle.flash, muzzle.silenced),
            (7, 255, false)
        );
    }
    let Cue::MuzzleFlash(packed) = exact(Protocol::Quake2Repro1038, &[2, 7, 96, 2]) else {
        panic!("muzzle");
    };
    assert_eq!((packed.entity, packed.flash), (7, 770));
    let Cue::MuzzleFlash(wide) = exact(Protocol::Quake2Repro1038, &[32, 7, 0, 2, 3]) else {
        panic!("muzzle");
    };
    assert_eq!(
        (wide.format, wide.entity, wide.flash),
        (MuzzleFormat::MonsterWide, 7, 770)
    );
    assert!(outputs::read_cue(Protocol::Quake2_34, &[32, 7, 0, 2, 3]).is_err());
    for entity in [0u16, 1024, u16::MAX] {
        let [a, b] = entity.to_le_bytes();
        assert!(outputs::read_cue(Protocol::Quake2_34, &[1, a, b, 0]).is_err());
    }
}

#[test]
fn each_native_flag_subset_and_entity_channel_round_trips_without_widening() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        for flags in 0..=if protocol == Protocol::Quake2_34 {
            31
        } else {
            63
        } {
            let mut bytes = vec![9, flags, 49];
            if flags & 32 != 0 {
                bytes.push(7);
            }
            for (mask, value) in [(1, 255), (2, 64), (16, 0)] {
                if flags & mask != 0 {
                    bytes.push(value);
                }
            }
            if flags & 8 != 0 {
                bytes.extend_from_slice(&[255, 255]);
            }
            if flags & 4 != 0 {
                let values = [0.125f32, -4096.0, 4095.875];
                for value in values {
                    if protocol == Protocol::Quake2_34 {
                        bytes.extend_from_slice(
                            &qa_core::math::eighth_to_short(value).to_le_bytes(),
                        );
                    } else {
                        bytes.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
            exact(protocol, &bytes);
        }
    }
}

#[test]
fn connected_channel_delivers_cues_in_order_and_continues_to_the_next_service() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let client = ClientId(0);
        let mut sender = Channel::load(protocol.channel(), Endpoint::Server, 8192, 4).unwrap();
        let mut connections = Connections::load(1);
        connections
            .bind(
                client,
                Endpoint::Client,
                Connection {
                    route: Route {
                        socket: Endpoint::Client.socket(),
                        peer: Peer::Loopback(client),
                    },
                    channel: Channel::load(protocol.channel(), Endpoint::Client, 8192, 4).unwrap(),
                    output: None,
                    pending_output: None,
                    commands: Some(Commands::load(protocol)),
                },
            )
            .unwrap();
        let payload = b"\x01\x01\x00\x80\x09\x00\x31\x0a\x02after\0";
        let packet = sender
            .prepare(Some(payload), EventTime(1))
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        let mut cues = Vec::new();
        let mut prints = Vec::new();
        connections.receive(
            Endpoint::Client.socket(),
            Peer::Loopback(client),
            &packet,
            EventTime(2),
            |_, _, incoming| {
                match incoming {
                    Incoming::Cue(cue) => cues.push(cue),
                    Incoming::Print(print) => prints.push(print.text.to_vec()),
                    _ => {}
                };
                std::ops::ControlFlow::Continue(())
            },
        );
        assert_eq!(cues.len(), 2);
        assert!(matches!(cues[0], Cue::MuzzleFlash(_)));
        assert!(matches!(cues[1], Cue::Sound(_)));
        assert_eq!(prints, [b"after".to_vec()]);
        assert_eq!((connections.command_errors, connections.malformed), (0, 0));
    }
}
