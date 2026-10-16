use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, RuleSetId, Vec3},
    sys_events::{EventTime, Peer},
};
use qa_network::{
    channel::Channel,
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection, Connections, Incoming, Route},
    outputs::{self, Error, NativeService, Particle},
};
use std::ops::ControlFlow;

// WinQuake SV_StartParticle: coord16 x3, signed direction8 x3, count, color.
const LITERAL: [u8; 12] = [18, 247, 255, 162, 0, 0, 128, 128, 225, 127, 255, 231];
const DECODED: Particle = Particle {
    position: Vec3([-1.125, 20.25, -4096.0]),
    direction: Vec3([-8.0, -1.9375, 7.9375]),
    count: 255,
    color: 231,
};

#[test]
fn original_particle_literal_retains_signed_fields_and_count_255() {
    assert_eq!(
        outputs::read_particle(Protocol::NetQuake15, &LITERAL),
        Ok((DECODED, LITERAL.len()))
    );
    let mut output = [0xcc; 13];
    let length = outputs::particle(Protocol::NetQuake15, DECODED, &mut output).unwrap();
    assert_eq!(&output[..length], LITERAL);
    assert_eq!(output[length], 0xcc);

    let mut with_suffix = LITERAL.to_vec();
    with_suffix.extend_from_slice(&[8, b'a', 0, 1]);
    let (NativeService::Particle(particle), consumed) =
        outputs::read_native_service(Protocol::NetQuake15, &with_suffix).unwrap()
    else {
        panic!("native particle service");
    };
    assert_eq!((particle, consumed), (DECODED, LITERAL.len()));
    let (NativeService::Print(print), consumed) =
        outputs::read_native_service(Protocol::NetQuake15, &with_suffix[consumed..]).unwrap()
    else {
        panic!("following print service");
    };
    assert_eq!((print.text, consumed), (&b"a"[..], 3));
}

#[test]
fn original_particle_direction_truncates_before_signed_byte_clamp() {
    for (direction, expected) in [
        ([-100.0, -1.999, 100.0], [128, 225, 127]),
        ([-0.06249, 0.06249, -0.0625], [0, 0, 255]),
        ([0.0625, 7.999, -7.999], [1, 127, 129]),
        ([8.0, -8.0, -8.0625], [127, 128, 128]),
    ] {
        let particle = Particle {
            position: Vec3([-0.1249, -0.125, 4096.0]),
            direction: Vec3(direction),
            count: 0,
            color: 255,
        };
        let mut output = [0xcc; 12];
        outputs::particle(Protocol::NetQuake15, particle, &mut output).unwrap();
        assert_eq!(&output[..7], &[18, 0, 0, 255, 255, 0, 128]);
        assert_eq!(&output[7..10], &expected);
        assert_eq!(&output[10..], &[0, 255]);
    }
}

#[test]
fn particle_admission_rejects_truncation_and_absent_native_services() {
    for cut in 0..LITERAL.len() {
        assert_eq!(
            outputs::read_particle(Protocol::NetQuake15, &LITERAL[..cut]),
            Err(Error::Truncated)
        );
        assert!(outputs::read_native_service(Protocol::NetQuake15, &LITERAL[..cut]).is_err());
        let mut output = [0xcc; 12];
        assert!(outputs::particle(Protocol::NetQuake15, DECODED, &mut output[..cut]).is_err());
        assert_eq!(output, [0xcc; 12]);
    }
    let mut wrong_opcode = LITERAL;
    wrong_opcode[0] = 23;
    assert_eq!(
        outputs::read_particle(Protocol::NetQuake15, &wrong_opcode),
        Err(Error::Opcode)
    );
    for protocol in [
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let mut output = [0xcc; 12];
        assert_eq!(
            outputs::particle(protocol, DECODED, &mut output),
            Err(Error::Unsupported)
        );
        assert_eq!(output, [0xcc; 12]);
        assert_eq!(
            outputs::read_particle(protocol, &LITERAL),
            Err(Error::Unsupported)
        );
        assert!(matches!(
            outputs::read_native_service(protocol, &LITERAL),
            Err(Error::Unsupported)
        ));
    }
}

#[test]
fn native_particle_transcode_drops_only_unrepresented_record_and_continues() {
    let mut source = LITERAL.to_vec();
    source.extend_from_slice(b"\x08after\0\x01");
    let mut output = [0xcc; 64];
    let encoded = outputs::native(
        Protocol::NetQuake15,
        RuleSetId::Quake,
        &source,
        &mut output,
        true,
        |_, _| panic!("particle is not a sound cue"),
    )
    .unwrap();
    assert_eq!(&output[..encoded.length], source);
    assert_eq!(encoded.unsupported, 0);
    for (protocol, expected) in [
        (Protocol::QuakeWorld28, &b"\x08\x02after\0\x01"[..]),
        (Protocol::Quake2_34, &b"\x0a\x02after\0\x06"[..]),
        (Protocol::Quake2Repro1038, &b"\x0a\x02after\0\x06"[..]),
    ] {
        let encoded = outputs::native(
            protocol,
            RuleSetId::Quake,
            &source,
            &mut output,
            false,
            |_, _| panic!("particle is not a sound cue"),
        )
        .unwrap();
        assert_eq!(encoded.unsupported, 1);
        assert_eq!(&output[..encoded.length], expected);
    }
    assert_eq!(
        outputs::native(
            Protocol::Quake3_68,
            RuleSetId::Quake,
            &source,
            &mut output,
            false,
            |_, _| panic!("Q3 has no native particle service"),
        ),
        Err(Error::Unsupported)
    );
}

fn connected_client() -> Connections {
    let protocol = Protocol::NetQuake15;
    let mut connections = Connections::load(1);
    connections
        .bind(
            ClientId(0),
            Endpoint::Client,
            Connection {
                phase: qa_network::ingress::ConnectionPhase::Connected,
                route: Route {
                    socket: Endpoint::Client.socket(),
                    peer: Peer::Loopback(ClientId(0)),
                },
                channel: Channel::load(protocol.channel(), Endpoint::Client, 8192, 16).unwrap(),
                output: None,
                pending_output: None,
                commands: Some(Commands::load(protocol, 8192)),
            },
        )
        .unwrap();
    connections
}

fn native_packet(payload: &[u8]) -> Vec<u8> {
    let mut server =
        Channel::load(Protocol::NetQuake15.channel(), Endpoint::Server, 8192, 16).unwrap();
    let packet = server
        .prepare_move(payload, EventTime(2), None, None)
        .unwrap()
        .unwrap();
    let bytes = packet.bytes.to_vec();
    server.submitted(EventTime(2)).unwrap();
    bytes
}

#[test]
fn connected_particle_ingress_preserves_service_order_without_snapshot() {
    let mut connections = connected_client();
    let mut payload = LITERAL.to_vec();
    payload.extend_from_slice(b"\x08after\0");
    payload.extend_from_slice(&LITERAL);
    let bytes = native_packet(&payload);
    let mut order = Vec::new();
    connections.receive(
        Endpoint::Client.socket(),
        Peer::Loopback(ClientId(0)),
        &bytes,
        EventTime(3),
        |_, _, incoming| {
            match incoming {
                Incoming::Particle(particle) => {
                    assert_eq!(particle, DECODED);
                    order.push("particle");
                }
                Incoming::Print(print) => {
                    assert_eq!(print.text, b"after");
                    order.push("print");
                }
                _ => panic!("particle/print stream"),
            }
            ControlFlow::Continue(())
        },
    );
    assert_eq!(order, ["particle", "print", "particle"]);
    assert_eq!(connections.command_errors, 0);
    assert!(
        connections
            .get(ClientId(0), Endpoint::Client)
            .unwrap()
            .channel
            .current_snapshot()
            .is_none()
    );
}

#[test]
fn incomplete_particle_never_delivers_a_partial_record_or_suppresses_prior_print() {
    for cut in 1..LITERAL.len() {
        let mut connections = connected_client();
        let mut payload = b"\x08before\0".to_vec();
        payload.extend_from_slice(&LITERAL[..cut]);
        let bytes = native_packet(&payload);
        let mut prints = 0;
        connections.receive(
            Endpoint::Client.socket(),
            Peer::Loopback(ClientId(0)),
            &bytes,
            EventTime(3),
            |_, _, incoming| {
                match incoming {
                    Incoming::Print(print) => {
                        assert_eq!(print.text, b"before");
                        prints += 1;
                    }
                    Incoming::Particle(_) => panic!("incomplete native record"),
                    _ => panic!("print/particle stream"),
                }
                ControlFlow::Continue(())
            },
        );
        assert_eq!(prints, 1);
        assert_eq!(connections.command_errors, 1);
    }
}
