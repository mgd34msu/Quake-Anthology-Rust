use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, EntityRecord, ModelRef, ResourceNamespace, Vec3},
    sys_events::{EventTime, Peer},
};
use qa_network::{
    channel::Channel,
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection, Connections, Incoming, Route},
    outputs::{self, STATIC_ENTITY_BYTES},
};
use std::ops::ControlFlow;
mod support;

// Original CL_ParseBaseline: four bytes then short-coordinate/byte-angle pairs.
const STATIC: [u8; STATIC_ENTITY_BYTES] = [20, 7, 3, 2, 4, 247, 255, 255, 162, 0, 64, 1, 4, 128];
const CLIENT: ResourceNamespace = ResourceNamespace::Client(ClientId(3));

#[test]
fn original_static_entity_literal_imports_native_fields_and_client_namespace() {
    for protocol in [Protocol::NetQuake15, Protocol::QuakeWorld28] {
        let (record, consumed) = outputs::read_static_entity(protocol, &STATIC, CLIENT).unwrap();
        assert_eq!(consumed, STATIC_ENTITY_BYTES);
        assert_eq!(record.position, Vec3([-1.125, 20.25, 128.125]));
        assert_eq!(record.angles, Vec3([-1.40625, 90., -180.]));
        assert_eq!(record.velocity, Vec3::default());
        assert_eq!(record.appearance.source, protocol.rule_set());
        assert_eq!(record.appearance.models[0], ModelRef::native(CLIENT, 7));
        assert_eq!(record.appearance.models[1..], [None; 3]);
        assert_eq!((record.appearance.frame, record.appearance.skin), (3, 4));
        assert_eq!(record.appearance.native.colormap, 2);
        assert_eq!(record.appearance.native.effects, 0);
        let mut out = [0xcc; STATIC_ENTITY_BYTES + 1];
        let length = outputs::static_entity(
            protocol,
            &record,
            |model| {
                assert_eq!(model.namespace, CLIENT);
                Some(model.ordinal)
            },
            &mut out,
        )
        .unwrap();
        // NQ integral-angle export cannot reproduce arbitrary signed angle8
        // values. QW's original floating export preserves this literal.
        let mut expected = STATIC;
        if protocol == Protocol::NetQuake15 {
            expected[7] = 0;
        }
        assert_eq!(&out[..length], expected);
        assert_eq!(out[length], 0xcc);
        for cut in 0..STATIC_ENTITY_BYTES {
            assert!(outputs::read_static_entity(protocol, &STATIC[..cut], CLIENT).is_err());
        }
        let mut zero = STATIC;
        zero[1] = 0;
        let (hidden, _) = outputs::read_static_entity(protocol, &zero, CLIENT).unwrap();
        assert_eq!(hidden.appearance.models[0], None);
        assert_eq!(hidden.position, record.position);
    }
}

#[test]
fn static_entity_native_angle_arithmetic_and_model_width_are_not_interchanged() {
    let (mut record, _) =
        outputs::read_static_entity(Protocol::QuakeWorld28, &STATIC, CLIENT).unwrap();
    record.angles.0[0] = -1.9;
    for (protocol, expected) in [(Protocol::NetQuake15, 0), (Protocol::QuakeWorld28, 255)] {
        let mut out = [0; STATIC_ENTITY_BYTES];
        let mut lookups = 0;
        outputs::static_entity(
            protocol,
            &record,
            |_| {
                lookups += 1;
                Some(7)
            },
            &mut out,
        )
        .unwrap();
        assert_eq!(lookups, 1);
        assert_eq!(out[7], expected);
        assert_eq!(
            outputs::static_entity(protocol, &record, |_| Some(256), &mut out),
            Err(outputs::Error::Unsupported)
        );
        assert_eq!(
            outputs::static_entity(protocol, &record, |_| None, &mut out),
            Err(outputs::Error::Unsupported)
        );
        assert!(
            outputs::static_entity(
                protocol,
                &record,
                |_| Some(7),
                &mut out[..STATIC_ENTITY_BYTES - 1]
            )
            .is_err()
        );
    }
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        assert_eq!(
            outputs::read_static_entity(protocol, &STATIC, CLIENT),
            Err(outputs::Error::Unsupported)
        );
        assert_eq!(
            outputs::static_entity(
                protocol,
                &record,
                |_| Some(7),
                &mut [0; STATIC_ENTITY_BYTES]
            ),
            Err(outputs::Error::Unsupported)
        );
    }
    let mut malformed = STATIC;
    malformed[0] = 22;
    assert_eq!(
        outputs::read_static_entity(Protocol::NetQuake15, &malformed, CLIENT),
        Err(outputs::Error::Opcode)
    );
}

#[test]
fn literal_static_entities_traverse_native_channel_in_service_order_without_snapshot_publication() {
    for protocol in [Protocol::NetQuake15, Protocol::QuakeWorld28] {
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
        let mut server = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
        if protocol == Protocol::QuakeWorld28 {
            support::admit_qw_reply(&mut server, 1);
        }
        let mut payload = STATIC.to_vec();
        payload.push(8);
        if protocol == Protocol::QuakeWorld28 {
            payload.push(2);
        }
        payload.extend_from_slice(b"after\0");
        payload.extend_from_slice(&STATIC);
        let packet = server
            .prepare_move(&payload, EventTime(2), None, None)
            .unwrap()
            .unwrap();
        let mut bytes = [0; 1400];
        let length = packet.bytes.len();
        bytes[..length].copy_from_slice(packet.bytes);
        server.submitted(EventTime(2)).unwrap();
        let mut order = Vec::new();
        let mut timing = 0;
        connections.receive(
            Endpoint::Client.socket(),
            Peer::Loopback(ClientId(0)),
            &bytes[..length],
            EventTime(3),
            |_, _, incoming| {
                match incoming {
                    Incoming::StaticEntity(record) => {
                        assert_eq!(record.position, Vec3([-1.125, 20.25, 128.125]));
                        assert_eq!(
                            record.appearance.models[0],
                            ModelRef::native(ResourceNamespace::Client(ClientId(0)), 7)
                        );
                        order.push("static");
                    }
                    Incoming::Print(print) => {
                        assert_eq!(print.text, b"after");
                        order.push("print");
                    }
                    Incoming::PacketTiming { acknowledged_sent } => {
                        assert_eq!(protocol, Protocol::QuakeWorld28);
                        assert_eq!(acknowledged_sent, None);
                        timing += 1;
                    }
                    _ => panic!("static/print service stream"),
                }
                ControlFlow::Continue(())
            },
        );
        assert_eq!(order, ["static", "print", "static"]);
        assert_eq!(timing, usize::from(protocol == Protocol::QuakeWorld28));
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
}

#[test]
fn empty_model_static_entity_never_calls_resource_resolver() {
    let record = EntityRecord::default();
    let mut out = [0xcc; STATIC_ENTITY_BYTES];
    for protocol in [Protocol::NetQuake15, Protocol::QuakeWorld28] {
        outputs::static_entity(
            protocol,
            &record,
            |_| panic!("model zero has no resource identity"),
            &mut out,
        )
        .unwrap();
        assert_eq!(out, [20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    }
}
