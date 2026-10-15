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
mod support;

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
fn nq_qw_dynamic_sounds_keep_native_field_order_presence_and_coordinate_widths() {
    use qa_core::primitives::{SoundAction, SoundId, SpatialPolicy};
    for (protocol, bytes) in [
        (
            Protocol::NetQuake15,
            &[6, 0, 41, 0, 49, 247, 255, 162, 0, 1, 4][..],
        ),
        (
            Protocol::NetQuake15,
            &[6, 3, 255, 64, 41, 0, 49, 247, 255, 162, 0, 1, 4][..],
        ),
        (
            Protocol::NetQuake15,
            &[6, 3, 204, 128, 41, 0, 49, 247, 255, 162, 0, 1, 4][..],
        ),
        (
            Protocol::QuakeWorld28,
            &[6, 41, 0, 49, 247, 255, 162, 0, 1, 4][..],
        ),
        (
            Protocol::QuakeWorld28,
            &[6, 41, 192, 255, 64, 49, 247, 255, 162, 0, 1, 4][..],
        ),
        (
            Protocol::QuakeWorld28,
            &[6, 41, 192, 204, 128, 49, 247, 255, 162, 0, 1, 4][..],
        ),
    ] {
        let Cue::Sound(wire) = exact(protocol, bytes) else {
            panic!("dynamic sound");
        };
        assert_eq!((wire.entity(), wire.channel, wire.index), (Some(5), 1, 49));
        assert_eq!(wire.position().unwrap().0, [-1.125, 20.25, 128.125]);
        let event = wire.into_event(
            SoundId(7),
            SpatialPolicy::default(),
            None,
            wire.position,
            None,
            SoundAction::Play,
        );
        let mut encoded = [0xa5; 32];
        let length = outputs::sound(protocol, event, 49, Some(5), false, &mut encoded).unwrap();
        assert_eq!(&encoded[..length], bytes);
        assert_eq!(encoded[length], 0xa5);
        for cut in 0..length {
            assert!(outputs::write_cue(protocol, Cue::Sound(wire), &mut [0; 32][..cut]).is_err());
        }
    }
    // POSITION/ENTITY are implicit in stock NQ, not NQ extension-mask bits.
    for unsupported_mask in [4, 8, 16, 32, 64, 128] {
        assert!(outputs::read_cue(Protocol::NetQuake15, &[6, unsupported_mask]).is_err());
    }
}

#[test]
fn dynamic_sound_field_limits_keep_world_zero_and_native_optional_byte_values() {
    for protocol in [Protocol::NetQuake15, Protocol::QuakeWorld28] {
        for presence in 0..4 {
            for entity in [
                0,
                if protocol == Protocol::NetQuake15 {
                    8191
                } else {
                    1023
                },
            ] {
                let wire = outputs::Sound {
                    flags: 12 | presence,
                    index: 255,
                    volume: 0,
                    attenuation: 255,
                    time_offset: 0,
                    entity,
                    channel: 7,
                    position: qa_core::primitives::Vec3([-4096.0, 4095.875, 0.125]),
                };
                let mut bytes = [0xa5; 32];
                let length = outputs::write_cue(protocol, Cue::Sound(wire), &mut bytes).unwrap();
                let Cue::Sound(decoded) = exact(protocol, &bytes[..length]) else {
                    panic!("sound");
                };
                assert_eq!(decoded.entity(), Some(entity));
                assert_eq!(
                    (decoded.channel, decoded.index, decoded.position),
                    (7, 255, wire.position)
                );
                assert_eq!(decoded.volume, if presence & 1 != 0 { 0 } else { 255 });
                assert_eq!(
                    decoded.attenuation,
                    if presence & 2 != 0 { 255 } else { 64 }
                );
                assert_eq!(decoded.flags, wire.flags);
            }
        }
    }
}

#[test]
fn common_and_cross_protocol_sounds_translate_implicit_fields_and_drop_unrepresentable_values() {
    use qa_core::primitives::{SoundEvent, Vec3};
    let event = SoundEvent {
        channel: 2,
        volume: 0.8,
        attenuation: 2.0,
        position: Vec3([-1.125, 20.25, 128.125]),
        ..Default::default()
    };
    for (protocol, expected) in [
        (
            Protocol::NetQuake15,
            &[6, 3, 204, 128, 2, 0, 49, 247, 255, 162, 0, 1, 4][..],
        ),
        (
            Protocol::QuakeWorld28,
            &[6, 2, 192, 204, 128, 49, 247, 255, 162, 0, 1, 4][..],
        ),
    ] {
        let mut bytes = [0; 64];
        let length = outputs::sound(protocol, event, 49, Some(0), false, &mut bytes).unwrap();
        assert_eq!(&bytes[..length], expected);
        assert_eq!(
            outputs::sound(
                protocol,
                SoundEvent {
                    time_offset: 0.125,
                    ..event
                },
                49,
                Some(0),
                false,
                &mut bytes
            ),
            Err(outputs::Error::Unsupported)
        );
        assert!(outputs::sound(protocol, event, 256, Some(0), false, &mut bytes).is_err());
    }
    // Q2 uses flags/index before optional values; its4/8 bits must become
    // implicit NQ/QW fields rather than being emitted in NQ's mask byte.
    let source = [9, 15, 49, 255, 64, 41, 0, 247, 255, 162, 0, 1, 4];
    for target in [Protocol::NetQuake15, Protocol::QuakeWorld28] {
        let mut bytes = [0; 64];
        let encoded = outputs::native(
            target,
            qa_core::primitives::RuleSetId::Quake2,
            &source,
            &mut bytes,
            false,
            |cue, _| Ok(cue),
        )
        .unwrap();
        let Cue::Sound(wire) = exact(target, &bytes[..encoded.length]) else {
            panic!("sound");
        };
        assert_eq!((wire.flags, wire.entity(), wire.channel), (15, Some(5), 1));
        assert_eq!(encoded.unsupported, 0);
        let mut restored = [0; 64];
        let reencoded = outputs::native(
            Protocol::Quake2_34,
            target.rule_set(),
            &bytes[..encoded.length],
            &mut restored,
            false,
            |cue, _| Ok(cue),
        )
        .unwrap();
        assert_eq!(&restored[..reencoded.length], &source);
        for source in [&[9, 16, 49, 125][..], &[9, 32, 1, 4][..]] {
            let mut bytes = [0; 64];
            let encoded = outputs::native(
                target,
                qa_core::primitives::RuleSetId::Quake2Rerelease,
                source,
                &mut bytes,
                false,
                |cue, _| Ok(cue),
            )
            .unwrap();
            assert_eq!((encoded.length, encoded.unsupported), (0, 1));
        }
    }
}
#[test]
fn shared_sound_events_preserve_explicit_defaults_and_index_width_flags() {
    use qa_core::primitives::{SoundAction, SoundId, SpatialPolicy, Vec3};
    for (protocol, bytes) in [
        (Protocol::Quake2_34, &[9, 27, 1, 255, 64, 0, 10, 0][..]),
        (
            Protocol::Quake2Repro1038,
            &[9, 59, 1, 0, 255, 64, 0, 10, 0][..],
        ),
        (Protocol::Quake2_34, &[9, 0, 1][..]),
        (Protocol::Quake2Repro1038, &[9, 32, 1, 0][..]),
    ] {
        let Cue::Sound(wire) = exact(protocol, bytes) else {
            panic!("sound")
        };
        let event = wire.into_event(
            SoundId(3),
            SpatialPolicy::default(),
            None,
            Vec3::default(),
            None,
            SoundAction::Play,
        );
        let mut output = [0xa5; 64];
        // Reliable RR normally requests a position. Captured presence wins.
        let length = outputs::sound(
            protocol,
            event,
            u32::from(wire.index),
            wire.entity().map(u32::from),
            true,
            &mut output,
        )
        .unwrap();
        assert_eq!(&output[..length], bytes);
        assert_eq!(output[length], 0xa5);
        assert_eq!(event.native_fields.unwrap().flags, wire.flags);
    }
}
#[test]
fn native_quantized_parameters_and_float_position_bits_survive_shared_sound() {
    use qa_core::primitives::{SoundAction, SoundId, SpatialPolicy, Vec3};
    for value in 0..=u8::MAX {
        let mut bytes = vec![9, 63, 1, 0, value, value, value, 10, 0];
        for bits in [(-0.0f32).to_bits(), 0x7fc1_2345, 0x3f80_0001] {
            bytes.extend(bits.to_le_bytes());
        }
        let (Cue::Sound(wire), length) =
            outputs::read_cue(Protocol::Quake2Repro1038, &bytes).unwrap()
        else {
            panic!("sound")
        };
        assert_eq!(length, bytes.len());
        let event = wire.into_event(
            SoundId(2),
            SpatialPolicy::default(),
            None,
            wire.position,
            None,
            SoundAction::Play,
        );
        let mut output = [0; 64];
        let length = outputs::sound(
            Protocol::Quake2Repro1038,
            event,
            1,
            Some(1),
            false,
            &mut output,
        )
        .unwrap();
        assert_eq!(&output[..length], bytes);
        assert_eq!(event.position.0[0].to_bits(), (-0.0f32).to_bits());
        let no_position = outputs::Sound {
            flags: 0,
            position: Vec3::default(),
            ..wire
        };
        let event = no_position.into_event(
            SoundId(2),
            SpatialPolicy::default(),
            None,
            Vec3::default(),
            None,
            SoundAction::Play,
        );
        let length =
            outputs::sound(Protocol::Quake2_34, event, 1, None, false, &mut output).unwrap();
        assert_eq!(&output[..length], &[9, 0, 1]);
    }
}

#[test]
fn native_static_sound_keeps_q1_qw_wire_fields_and_persistent_kind() {
    let bytes = [29, 247, 255, 162, 0, 1, 4, 255, 204, 128];
    for protocol in [Protocol::NetQuake15, Protocol::QuakeWorld28] {
        let Cue::StaticSound(sound) = exact(protocol, &bytes) else {
            panic!("static sound")
        };
        assert_eq!(sound.index, 255);
        assert_eq!(
            (sound.volume, sound.attenuation, sound.time_offset),
            (204, 128, 0)
        );
        assert_eq!((sound.entity(), sound.channel), (None, 0));
        assert_eq!(
            sound.position(),
            Some(qa_core::primitives::Vec3([-1.125, 20.25, 128.125]))
        );
        for target in [
            Protocol::Quake2_34,
            Protocol::Quake2Repro1038,
            Protocol::Quake3_68,
        ] {
            assert_eq!(
                outputs::write_cue(target, Cue::StaticSound(sound), &mut [0; 32]),
                Err(outputs::Error::Unsupported)
            );
        }
        let mut wide = sound.with_index(256).unwrap();
        assert_eq!(
            outputs::write_cue(protocol, Cue::StaticSound(wide), &mut [0; 32]),
            Err(outputs::Error::Unsupported)
        );
        wide.index = 1;
        wide.flags = 7;
        wide.channel = 1;
        assert_eq!(
            outputs::write_cue(protocol, Cue::StaticSound(wide), &mut [0; 32]),
            Err(outputs::Error::Unsupported)
        );
    }
}

#[test]
fn common_ambient_qw_sound_encodes_native_loop_and_never_a_one_shot() {
    use qa_core::primitives::{SoundAction, SoundEvent, Vec3};
    let event = SoundEvent {
        action: SoundAction::StartLoop,
        position: Vec3([-1.125, 20.25, 128.125]),
        volume: 0.8,
        attenuation: 2.0,
        ..SoundEvent::default()
    };
    let mut bytes = [0xcc; 32];
    let count = outputs::sound(Protocol::QuakeWorld28, event, 49, None, true, &mut bytes).unwrap();
    assert_eq!(&bytes[..count], &[29, 247, 255, 162, 0, 1, 4, 49, 204, 128]);
    assert_eq!(bytes[count], 0xcc);
    assert!(matches!(
        exact(Protocol::QuakeWorld28, &bytes[..count]),
        Cue::StaticSound(_)
    ));
    for protocol in [
        Protocol::NetQuake15,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        assert_eq!(
            outputs::sound(protocol, event, 49, None, true, &mut bytes),
            Err(outputs::Error::Unsupported)
        );
    }
    assert_eq!(
        outputs::sound(Protocol::QuakeWorld28, event, 256, None, true, &mut bytes),
        Err(outputs::Error::Unsupported)
    );
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
fn common_sound_projection_uses_native_presence_quantization_and_limits() {
    use qa_core::primitives::{SoundAction, SoundEvent, Vec3};
    let mut output = [0xcc; 64];
    let event = SoundEvent {
        channel: 9,
        position: Vec3([-1.125, 20.25, 128.125]),
        volume: 0.8,
        attenuation: 2.0,
        time_offset: 0.125,
        ..SoundEvent::default()
    };
    let length =
        outputs::sound(Protocol::Quake2_34, event, 49, Some(5), true, &mut output).unwrap();
    assert_eq!(
        &output[..length],
        &[9, 31, 49, 204, 128, 125, 41, 0, 247, 255, 162, 0, 1, 4]
    );
    assert_eq!(output[length], 0xcc);
    let defaults = SoundEvent {
        volume: 1.0,
        attenuation: 1.0,
        ..SoundEvent::default()
    };
    let length = outputs::sound(
        Protocol::Quake2_34,
        defaults,
        255,
        Some(1023),
        false,
        &mut output,
    )
    .unwrap();
    assert_eq!(&output[..length], &[9, 8, 255, 248, 31]);
    assert!(
        outputs::sound(
            Protocol::Quake2_34,
            defaults,
            256,
            Some(1),
            false,
            &mut output
        )
        .is_err()
    );
    assert!(
        outputs::sound(
            Protocol::Quake2_34,
            defaults,
            1,
            Some(1024),
            false,
            &mut output
        )
        .is_err()
    );
    let mut wide = defaults;
    wide.channel = 7;
    wide.attenuation = 4.0;
    let length = outputs::sound(
        Protocol::Quake2Repro1038,
        wide,
        2047,
        Some(8191),
        true,
        &mut output,
    )
    .unwrap();
    let Cue::Sound(projected) = exact(Protocol::Quake2Repro1038, &output[..length]) else {
        panic!("sound")
    };
    assert_eq!(
        (
            projected.index,
            projected.entity,
            projected.channel,
            projected.attenuation
        ),
        (2047, 8191, 7, 0)
    );
    assert!(projected.position().is_some());
    assert!(
        outputs::sound(
            Protocol::Quake2Repro1038,
            wide,
            2048,
            Some(1),
            false,
            &mut output
        )
        .is_err()
    );
    assert!(
        outputs::sound(
            Protocol::Quake2Repro1038,
            wide,
            1,
            Some(8192),
            false,
            &mut output
        )
        .is_err()
    );
    wide.action = SoundAction::Stop;
    assert!(outputs::sound(Protocol::Quake2Repro1038, wide, 1, None, false, &mut output).is_err());
    assert!(outputs::sound(Protocol::Quake3_68, defaults, 1, None, false, &mut output).is_err());
    assert!(outputs::sound(Protocol::Quake2_34, defaults, 0, None, false, &mut output).is_err());
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
                    commands: Some(Commands::load(protocol, 8192)),
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
                    Incoming::Cue { cue, .. } => cues.push(cue),
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

#[test]
fn framed_nq_qw_dynamic_sound_intake_continues_to_the_following_print() {
    for (protocol, body) in [
        (
            Protocol::NetQuake15,
            &[6, 0, 2, 0, 49, 8, 0, 16, 0, 24, 0, 8, b'o', b'k', 0][..],
        ),
        (
            Protocol::QuakeWorld28,
            &[6, 2, 0, 49, 8, 0, 16, 0, 24, 0, 8, 2, b'o', b'k', 0][..],
        ),
    ] {
        let client = ClientId(0);
        let mut sender = Channel::load(protocol.channel(), Endpoint::Server, 8192, 4).unwrap();
        if protocol == Protocol::QuakeWorld28 {
            support::admit_qw_reply(&mut sender, 1);
        }
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
                    commands: Some(Commands::load(protocol, 8192)),
                },
            )
            .unwrap();
        let packet = sender
            .prepare(Some(body), EventTime(1))
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
                    Incoming::Cue { cue, .. } => cues.push(cue),
                    Incoming::Print(print) => prints.push(print.text.to_vec()),
                    _ => {}
                }
                std::ops::ControlFlow::Continue(())
            },
        );
        assert_eq!(cues.len(), 1);
        let Cue::Sound(sound) = cues[0] else {
            panic!("dynamic sound");
        };
        assert_eq!(
            (sound.entity(), sound.channel, sound.position),
            (Some(0), 2, qa_core::primitives::Vec3([1.0, 2.0, 3.0]))
        );
        assert_eq!(prints, [b"ok".to_vec()]);
        assert_eq!((connections.command_errors, connections.malformed), (0, 0));
    }
}

#[test]
fn client_loop_controls_are_never_reinterpreted_as_native_static_sound() {
    use qa_core::primitives::{
        LoopGain, LoopLifetime, LoopOrigin, LoopPhase, LoopPlayback, SoundAction, SoundEvent,
    };
    let playback = LoopPlayback {
        master: LoopGain::Integer {
            master: 255,
            volume_scale: 256,
        },
        phase: LoopPhase::Painted,
        lifetime: LoopLifetime::Temporary,
        panning: true,
        clamp_pan: false,
        origin: LoopOrigin::Accepted,
        latched: false,
        cue_extent: true,
    };
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        for action in [
            SoundAction::StartLoop,
            SoundAction::StopLoop,
            SoundAction::ResetClientLoops,
            SoundAction::ClearTemporaryLoops,
            SoundAction::ClearClientLoops,
            SoundAction::UpdateEmitterPosition,
            SoundAction::Respatialize,
            SoundAction::ReleaseListener,
        ] {
            let event = SoundEvent {
                action,
                looping: (action == SoundAction::StartLoop).then_some(playback),
                ..Default::default()
            };
            let mut bytes = [0xcc; 128];
            assert_eq!(
                outputs::sound(protocol, event, 1, Some(1), true, &mut bytes),
                Err(outputs::Error::Unsupported)
            );
            assert_eq!(bytes, [0xcc; 128]);
        }
    }
}
