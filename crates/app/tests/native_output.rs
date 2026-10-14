use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{loopback::Endpoint, primitives::*, sys_events::*};
use qa_network::{
    channel::{Channel, Delivery},
    commands::{connection::Commands, packet::Protocol},
    ingress::{Connection as NativeConnection, Incoming, Route},
};
use qa_session::{clients::Connection, timing::TickRate};
use std::time::Duration;

#[derive(Default)]
struct Source {
    reject: Option<u16>,
    accepted_limit: Option<(u16, usize)>,
    packets: Vec<(u16, Vec<u8>)>,
    attempts: usize,
    now: EventTime,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(self.now.0)
    }
    fn poll_events(&mut self, q: &mut SysEventQueue) {
        q.push(SysEvent {
            time: self.now,
            kind: EventKind::Time,
        })
        .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::from_nanos(self.now.0)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn send_packet(&mut self, _: u16, to: std::net::SocketAddr, bytes: &[u8]) -> bool {
        self.attempts += 1;
        if self.reject == Some(to.port()) {
            return false;
        }
        if self.accepted_limit.is_some_and(|(port, limit)| {
            port == to.port() && self.packets.iter().filter(|(p, _)| *p == port).count() >= limit
        }) {
            return false;
        }
        self.packets.push((to.port(), bytes.to_vec()));
        true
    }
}
fn remote(runtime: &mut Runtime, protocol: Protocol, port: u16) -> ClientId {
    let id = runtime
        .server
        .connect(Connection::Remote, ModuleId(7), None)
        .unwrap();
    assert!(runtime.server.bind_client_native(
        id,
        NativeEntity {
            module: ModuleId(7),
            slot: id.0 as i32 + 1,
        },
    ));
    let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 8192, 16).unwrap();
    channel.set_qport(port);
    runtime
        .network
        .bind(
            id,
            Endpoint::Server,
            NativeConnection {
                route: Route {
                    socket: 0,
                    peer: Peer::Socket(([127, 0, 0, 1], port).into()),
                },
                channel,
                output: runtime.server.clients[id.0 as usize].output,
                pending_output: None,
                commands: Some(Commands::load(protocol, 8192)),
            },
        )
        .unwrap();
    id
}
fn publish(runtime: &mut Runtime, reliable: bool, target: NativeTarget, bytes: &[u8]) -> u64 {
    publish_from(runtime, RuleSetId::Quake2, reliable, target, bytes)
}
fn publish_from(
    runtime: &mut Runtime,
    source: RuleSetId,
    reliable: bool,
    target: NativeTarget,
    bytes: &[u8],
) -> u64 {
    runtime
        .server
        .events
        .native_message(
            NativeDelivery {
                module: ModuleId(7),
                source,
                target,
                origin: Vec3([1.0, 0.0, 0.0]),
                reliable,
                dupe_key: 0,
                component: 0,
                emission: None,
                check_portals: true,
            },
            bytes,
        )
        .unwrap()
}

#[test]
fn native_cue_entities_use_the_entity_namespace_independently_of_resources() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let client = remote(&mut runtime, Protocol::Quake2_34, 2000);
    assert!(runtime.server.bind_client_native(
        client,
        NativeEntity {
            module: ModuleId(8),
            slot: 1,
        },
    ));
    // The resource owner and native entity namespace are separate roles.
    runtime.server.clients[client.0 as usize].module = ModuleId(7);
    let emitter = runtime
        .server
        .entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(7),
            qa_world::entities::AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    runtime.server.entities.columns.native_entity[emitter.slot as usize] = Some(NativeEntity {
        module: ModuleId(7),
        slot: 5,
    });
    let mut stored = [0u8; 4 + NATIVE_CUE_CAPTURE_BYTES];
    stored[..4].copy_from_slice(&[1, 5, 0, 0]);
    NativeCueBinding {
        entity: Some(emitter),
    }
    .write_capture(0, &mut stored[4..])
    .unwrap();
    runtime
        .server
        .events
        .native_message_bound(
            NativeDelivery {
                module: ModuleId(7),
                source: RuleSetId::Quake2,
                target: NativeTarget::Client(client),
                origin: Vec3::default(),
                reliable: false,
                dupe_key: 0,
                component: 0,
                emission: None,
                check_portals: false,
            },
            4,
            &stored,
        )
        .unwrap();
    let mut source = Source {
        now: EventTime(10),
        ..Default::default()
    };
    let counts = dispatch(&mut runtime, &mut source);
    assert_eq!(counts.native_unsupported, 0);
    assert_eq!(source.packets.len(), 1);
    let projected = runtime.native_numbers[client.0 as usize]
        .number(emitter)
        .unwrap();
    assert_ne!(projected, 5);
    let mut peer =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
    let payload = receive(&mut peer, &source.packets[0].1);
    let (record, length) =
        qa_network::outputs::read_native_service(Protocol::Quake2_34, &payload).unwrap();
    assert_eq!(length, payload.len());
    assert!(
        matches!(record, qa_network::outputs::NativeService::Cue(qa_network::outputs::Cue::MuzzleFlash(flash)) if u32::from(flash.entity) == projected)
    );
}

#[test]
fn recognized_q2_records_reach_both_native_protocols_and_wait_for_each_real_ack() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let classic = remote(&mut runtime, Protocol::Quake2_34, 2000);
    let rerelease = remote(&mut runtime, Protocol::Quake2Repro1038, 2001);
    let mut peers = [
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap(),
        Channel::load(
            Protocol::Quake2Repro1038.channel(),
            Endpoint::Client,
            8192,
            16,
        )
        .unwrap(),
    ];
    for (peer, port) in peers.iter_mut().zip([2000, 2001]) {
        peer.set_qport(port);
    }
    let source_bytes = [10, 2, b'p', 0x80, 0x81, 0, 1, 5, 0, 128, 4, b'l', 0x81, 0];
    publish(&mut runtime, true, NativeTarget::All, &source_bytes);
    let mut source = Source {
        now: EventTime(10),
        ..Default::default()
    };
    let counts = dispatch(&mut runtime, &mut source);
    assert_eq!(counts.native_unsupported, 0);
    assert_eq!(source.packets.len(), 2);
    for (port, bytes) in &source.packets {
        let index = usize::from(*port - 2000);
        let payload = receive(&mut peers[index], bytes);
        if index == 0 {
            assert_eq!(payload, source_bytes);
        } else {
            let mut remaining = payload.as_slice();
            let (print, n) =
                qa_network::outputs::read_native_service(Protocol::Quake2Repro1038, remaining)
                    .unwrap();
            let qa_network::outputs::NativeService::Print(print) = print else {
                panic!("print");
            };
            assert_eq!(print.text, b"p\x80\x81");
            remaining = &remaining[n..];
            let (cue, n) =
                qa_network::outputs::read_native_service(Protocol::Quake2Repro1038, remaining)
                    .unwrap();
            assert!(
                matches!(cue, qa_network::outputs::NativeService::Cue(qa_network::outputs::Cue::MuzzleFlash(flash))
                if flash.entity == 5 && flash.flash == 0 && flash.silenced)
            );
            remaining = &remaining[n..];
            let (layout, n) =
                qa_network::outputs::read_native_service(Protocol::Quake2Repro1038, remaining)
                    .unwrap();
            assert!(
                matches!(layout, qa_network::outputs::NativeService::Print(print)
                if print.kind == PrintKind::Layout && print.text == b"l\x81")
            );
            assert_eq!(n, remaining.len());
        }
    }
    assert_eq!(runtime.server.events.len(), 1);
    for (index, client) in [classic, rerelease].into_iter().enumerate() {
        let packet = peers[index]
            .prepare(None, EventTime(40))
            .unwrap()
            .unwrap()
            .bytes
            .to_vec();
        peers[index].submitted(EventTime(40)).unwrap();
        runtime.network.receive(
            0,
            Peer::Socket(([127, 0, 0, 1], 2000 + index as u16).into()),
            &packet,
            EventTime(50),
            |id, endpoint, incoming| {
                assert_eq!((id, endpoint), (client, Endpoint::Server));
                if let Incoming::Acknowledged {
                    receipt,
                    output: Some(id),
                } = incoming
                {
                    runtime.server.events.acknowledge(id, receipt);
                }
                std::ops::ControlFlow::Continue(())
            },
        );
        assert_eq!(runtime.server.events.len(), usize::from(index == 0));
    }
}

#[test]
fn a_classic_peer_drops_only_unrepresentable_rerelease_cues_and_receives_later_prints() {
    let mut runtime = Runtime::load(1, []).unwrap();
    remote(&mut runtime, Protocol::Quake2_34, 2000);
    let bytes = [
        9, 32, 1, 4, 32, 7, 0, 2, 3, 10, 5, b'c', 0x80, 0, 4, b'l', 0x81, 0,
    ];
    publish_from(
        &mut runtime,
        RuleSetId::Quake2Rerelease,
        false,
        NativeTarget::All,
        &bytes,
    );
    let mut source = Source {
        now: EventTime(10),
        ..Default::default()
    };
    let counts = dispatch(&mut runtime, &mut source);
    assert_eq!(counts.native_unsupported, 2);
    assert_eq!(source.packets.len(), 1);
    let mut peer =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
    assert_eq!(
        receive(&mut peer, &source.packets[0].1),
        b"\x0fc\x80\0\x04l\x81\0"
    );
    assert!(runtime.server.events.is_empty());
}

#[test]
fn converted_unreliable_bytes_survive_a_blocked_peer_and_reused_output_scratch() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let blocked = remote(&mut runtime, Protocol::Quake2Repro1038, 2000);
    remote(&mut runtime, Protocol::Quake2_34, 2001);
    let mut peers = [
        Channel::load(
            Protocol::Quake2Repro1038.channel(),
            Endpoint::Client,
            8192,
            16,
        )
        .unwrap(),
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap(),
    ];
    let cue = [1, 5, 0, 128];
    let sequence = publish(&mut runtime, false, NativeTarget::All, &cue);
    let mut source = Source {
        reject: Some(2000),
        now: EventTime(10),
        ..Default::default()
    };
    assert_eq!(dispatch(&mut runtime, &mut source).native_blocked, 1);
    assert_eq!(source.packets.len(), 1);
    assert_eq!(receive(&mut peers[1], &source.packets[0].1), cue);
    assert_eq!(
        runtime
            .network
            .get(blocked, Endpoint::Server)
            .unwrap()
            .pending_output,
        Some(sequence)
    );
    publish(&mut runtime, false, NativeTarget::All, b"\x0fB\0");
    source.reject = None;
    source.packets.clear();
    dispatch(&mut runtime, &mut source);
    let mut delivered = [Vec::new(), Vec::new()];
    for (port, bytes) in &source.packets {
        let index = usize::from(*port - 2000);
        delivered[index].push(receive(&mut peers[index], bytes));
    }
    assert_eq!(delivered[0].len(), 2);
    let (converted, count) =
        qa_network::outputs::read_cue(Protocol::Quake2Repro1038, &delivered[0][0]).unwrap();
    assert_eq!(count, delivered[0][0].len());
    assert_eq!(
        converted,
        qa_network::outputs::read_cue(Protocol::Quake2_34, &cue)
            .unwrap()
            .0
    );
    assert_eq!(delivered[0][1], b"\x0fB\0");
    assert_eq!(delivered[1], [b"\x0fB\0".to_vec()]);
    assert!(runtime.server.events.is_empty());
}
fn dispatch(runtime: &mut Runtime, source: &mut Source) -> qa_app::output::OutputCounts {
    let mut scrollback = qa_console::presentation::Scrollback::default();
    let counts = qa_app::output::dispatch(
        runtime,
        source,
        &mut scrollback,
        &[None; SeatId::COUNT],
        source.now,
        3.0,
        2.0,
    );
    scrollback.clear(&mut runtime.server.events.texts);
    counts
}
fn receive(peer: &mut Channel, bytes: &[u8]) -> Vec<u8> {
    let received = peer.receive(bytes, EventTime(20)).unwrap();
    let Delivery::Payload(bytes) = received.delivery else {
        panic!()
    };
    bytes.to_vec()
}
#[test]
fn unreliable_transport_retry_keeps_its_record_and_does_not_hold_a_healthy_peer() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let first = remote(&mut runtime, Protocol::Quake2_34, 2000);
    let _second = remote(&mut runtime, Protocol::Quake2_34, 2001);
    let mut peers = std::array::from_fn::<_, 2, _>(|_| {
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap()
    });
    let mut source = Source {
        reject: Some(2000),
        now: EventTime(10),
        ..Default::default()
    };
    let sequence = publish(&mut runtime, false, NativeTarget::All, b"\x0fA\0");
    let counts = dispatch(&mut runtime, &mut source);
    assert_eq!(counts.native_blocked, 1);
    assert_eq!(source.attempts, 2);
    assert_eq!(source.packets.len(), 1);
    assert_eq!(receive(&mut peers[1], &source.packets[0].1), b"\x0fA\0");
    assert_eq!(
        runtime
            .network
            .get(first, Endpoint::Server)
            .unwrap()
            .pending_output,
        Some(sequence)
    );
    assert_eq!(runtime.server.events.len(), 1);
    publish(&mut runtime, false, NativeTarget::All, b"\x0fB\0");
    source.reject = None;
    source.now = EventTime(30);
    source.packets.clear();
    dispatch(&mut runtime, &mut source);
    assert_eq!(source.packets.len(), 3);
    let mut output = [Vec::new(), Vec::new()];
    for (port, bytes) in &source.packets {
        output[usize::from(port - 2000)].push(receive(&mut peers[usize::from(port - 2000)], bytes));
    }
    assert_eq!(output[0], [b"\x0fA\0".to_vec(), b"\x0fB\0".to_vec()]);
    assert_eq!(output[1], [b"\x0fB\0".to_vec()]);
    assert!(runtime.server.events.is_empty());
    assert!(
        runtime
            .network
            .get(first, Endpoint::Server)
            .unwrap()
            .pending_output
            .is_none()
    );
}

fn queue_move(host: &mut FrameHost, peer: &mut Channel, protocol: Protocol, time: EventTime) {
    let command = UserCmd {
        duration_ms: 16,
        server_time_ms: time.milliseconds() as i32,
        ..Default::default()
    };
    let mut bytes = [0; 1400];
    let length = Commands::load(protocol, 8192)
        .encode(&command, |_| 0, peer, &mut bytes)
        .unwrap();
    let packet = peer
        .prepare_move(&bytes[..length], time, None, None)
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    peer.submitted(time).unwrap();
    host.queue
        .push(SysEvent {
            time,
            kind: EventKind::Packet {
                socket: 0,
                from: Peer::Socket(([127, 0, 0, 1], 2000).into()),
                bytes: &packet,
            },
        })
        .unwrap();
}

#[test]
fn qw_cancelled_reply_rebuilds_its_retained_event_after_a_new_queued_move() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let blocked = remote(&mut runtime, Protocol::QuakeWorld28, 2000);
    let healthy = remote(&mut runtime, Protocol::Quake2_34, 2001);
    let mut peer =
        Channel::load(Protocol::QuakeWorld28.channel(), Endpoint::Client, 8192, 16).unwrap();
    peer.set_qport(2000);
    let mut healthy_peer =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(25).unwrap(),
        vec![],
    )
    .unwrap();
    let mut source = Source {
        reject: Some(2000),
        ..Default::default()
    };
    queue_move(&mut host, &mut peer, Protocol::QuakeWorld28, source.now);
    let sequence = publish_from(
        &mut host.runtime,
        RuleSetId::QuakeWorld,
        false,
        NativeTarget::All,
        b"\x1arebuilt\0",
    );
    let first = host.frame(&mut source, true);
    assert_eq!(first.output.native_blocked, 1);
    assert_eq!(host.runtime.network.commands, 1);
    let owner = host.runtime.server.clients[blocked.0 as usize]
        .output
        .unwrap();
    assert!(host.runtime.server.events.retained_for(owner, sequence));
    assert_eq!(source.packets.len(), 1);
    assert_eq!(
        receive(&mut healthy_peer, &source.packets[0].1),
        b"\x0frebuilt\0"
    );

    // This newer real clc_move cancels the rejected packet while the old ring
    // record remains retained. A replacement empty reply cannot retire it.
    source.now = EventTime::from_milliseconds(25);
    queue_move(&mut host, &mut peer, Protocol::QuakeWorld28, source.now);
    publish(
        &mut host.runtime,
        false,
        NativeTarget::Client(healthy),
        b"\x0fhealthy\0",
    );
    source.packets.clear();
    let second = host.frame(&mut source, true);
    assert_eq!(host.runtime.network.commands, 2);
    assert_eq!(host.runtime.network.command_errors, 0);
    assert_eq!(second.server_ticks, 1);
    assert_eq!(second.output.native_blocked, 1);
    assert!(host.runtime.server.events.retained_for(owner, sequence));
    let connection = host.runtime.network.get(blocked, Endpoint::Server).unwrap();
    assert_eq!(connection.pending_output, Some(sequence));
    let packet = connection.channel.pending_packet().unwrap();
    let (_, body) = qa_network::headers::decode(
        qa_network::headers::QUAKEWORLD,
        qa_network::headers::Direction::ToClient,
        packet.bytes,
    )
    .unwrap();
    assert_eq!(body, b"\x1arebuilt\0");
    assert_eq!(source.packets.len(), 1);
    assert_eq!(source.packets[0].0, 2001);
    assert_eq!(
        receive(&mut healthy_peer, &source.packets[0].1),
        b"\x0fhealthy\0"
    );

    source.reject = None;
    source.now = EventTime::from_milliseconds(50);
    source.packets.clear();
    let third = host.frame(&mut source, true);
    assert_eq!(third.server_ticks, 1);
    assert_eq!(source.packets.len(), 1);
    assert_eq!(source.packets[0].0, 2000);
    assert_eq!(receive(&mut peer, &source.packets[0].1), b"\x1arebuilt\0");
    assert!(host.runtime.server.events.is_empty());
    assert!(
        host.runtime
            .network
            .get(blocked, Endpoint::Server)
            .unwrap()
            .pending_output
            .is_none()
    );
    source.now = EventTime::from_milliseconds(75);
    source.packets.clear();
    assert_eq!(host.frame(&mut source, true).server_ticks, 1);
    assert!(source.packets.is_empty());
}

#[test]
fn overlarge_common_print_is_a_counted_terminal_drop_without_blocking_a_healthy_peer() {
    for protocol in [
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake3_68,
    ] {
        let mut runtime = Runtime::load(2, []).unwrap();
        let target = remote(&mut runtime, protocol, 2000);
        let healthy = remote(&mut runtime, Protocol::Quake2_34, 2001);
        let owner = runtime.server.clients[target.0 as usize].output.unwrap();
        // Q3's command body reaches 1023 bytes with the print prefix/quotes;
        // MSG_ReadString cannot consume its terminator at that width.
        let text = vec![
            b'x';
            if protocol == Protocol::Quake3_68 {
                1015
            } else {
                1500
            }
        ];
        let sequence = runtime
            .server
            .events
            .print_bytes(Some(target), PrintKind::Console, &text)
            .unwrap();
        publish(
            &mut runtime,
            false,
            NativeTarget::Client(healthy),
            b"\x0fhealthy\0",
        );
        let mut host = FrameHost::load(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::fixed(25).unwrap(),
            vec![],
        )
        .unwrap();
        let mut source = Source::default();
        let first = host.frame(&mut source, true);
        assert_eq!(first.output.native_unsupported, 1, "{protocol:?}");
        assert_eq!(first.output.native_disconnected, 0);
        assert!(!host.runtime.server.events.retained_for(owner, sequence));
        assert!(host.runtime.server.events.is_empty());
        assert_eq!(source.packets.len(), 1);
        assert_eq!(source.packets[0].0, 2001);
        let mut peer =
            Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
        assert_eq!(receive(&mut peer, &source.packets[0].1), b"\x0fhealthy\0");
        source.packets.clear();
        for frame in 1..=40 {
            source.now = EventTime::from_milliseconds(frame * 25);
            let result = host.frame(&mut source, true);
            assert_eq!(result.server_ticks, 1);
            assert_eq!(result.output.native_unsupported, 0);
            assert_eq!(result.output.native_disconnected, 0);
            assert!(!host.runtime.server.events.needs_resync(owner));
        }
        assert!(source.packets.is_empty());
        assert_eq!(
            host.runtime.server.clients[target.0 as usize].connection,
            Some(Connection::Remote)
        );
    }
}

#[test]
fn q3_common_prints_keep_the_native_width_limit_and_wait_for_a_real_queued_ack() {
    for length in [900, 1014] {
        let mut runtime = Runtime::load(2, []).unwrap();
        let target = remote(&mut runtime, Protocol::Quake3_68, 2000);
        let healthy = remote(&mut runtime, Protocol::Quake2_34, 2001);
        let owner = runtime.server.clients[target.0 as usize].output.unwrap();
        let text = vec![b'x'; length];
        let sequence = runtime
            .server
            .events
            .print_bytes(Some(target), PrintKind::Console, &text)
            .unwrap();
        publish(
            &mut runtime,
            false,
            NativeTarget::Client(healthy),
            b"\x0fhealthy\0",
        );
        let mut host = FrameHost::load(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::fixed(25).unwrap(),
            vec![],
        )
        .unwrap();
        let mut source = Source::default();
        let first = host.frame(&mut source, true);
        assert_eq!(first.output.native_unsupported, 0);
        assert!(host.runtime.server.events.retained_for(owner, sequence));
        let channel = &host
            .runtime
            .network
            .get(target, Endpoint::Server)
            .unwrap()
            .channel;
        assert_eq!(channel.command_state().unwrap().queued, 1);
        assert_eq!(channel.command_state().unwrap().submitted, 1);
        assert_eq!(channel.command_state().unwrap().acknowledged, 0);

        let mut peer =
            Channel::load(Protocol::Quake3_68.channel(), Endpoint::Client, 8192, 16).unwrap();
        peer.set_qport(2000);
        let mut commands = Vec::new();
        let mut healthy_peer =
            Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
        for (port, bytes) in &source.packets {
            if *port == 2000 {
                let received = peer.receive(bytes, EventTime(1)).unwrap();
                let sequence = received.header.sequence;
                if let Delivery::Payload(body) = received.delivery {
                    let mut body = body.to_vec();
                    peer.decode_server_output(&mut body, sequence, |message| {
                        if let qa_network::channel::commands::ServerOutput::ReliableCommand {
                            text,
                            ..
                        } = message
                        {
                            commands.push(text.to_vec());
                        }
                    })
                    .unwrap();
                }
            } else {
                assert_eq!(*port, 2001);
                assert_eq!(receive(&mut healthy_peer, bytes), b"\x0fhealthy\0");
            }
        }
        let mut native = vec![0; OUTPUT_PAGE_BYTES];
        let count = qa_network::outputs::print(
            Protocol::Quake3_68,
            PrintKind::Console,
            PrintKind::Console.default_level(),
            &text,
            None,
            &mut native,
        )
        .unwrap();
        assert_eq!(commands, [native[..count - 1].to_vec()]);
        assert!(host.runtime.server.events.retained_for(owner, sequence));

        source.now = EventTime::from_milliseconds(25);
        queue_move(&mut host, &mut peer, Protocol::Quake3_68, source.now);
        source.packets.clear();
        let next = host.frame(&mut source, true);
        assert_eq!(next.server_ticks, 1);
        assert_eq!(host.runtime.network.command_errors, 0);
        assert!(!host.runtime.server.events.retained_for(owner, sequence));
        assert!(host.runtime.server.events.is_empty());
        assert_eq!(
            host.runtime
                .network
                .get(target, Endpoint::Server)
                .unwrap()
                .channel
                .command_state()
                .unwrap()
                .acknowledged,
            1
        );
    }
}

#[test]
fn absent_presentation_backends_drop_once_and_leave_peer_delivery_and_ticks_running() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let client = remote(&mut runtime, Protocol::Quake2_34, 2000);
    runtime
        .server
        .events
        .push(qa_core::events::FrameEvent::Sound(SoundEvent::default()))
        .unwrap();
    runtime
        .server
        .events
        .push(qa_core::events::FrameEvent::Effect(EffectEvent {
            effect: EffectId(1),
            position: Vec3::default(),
            direction: Vec3::default(),
            count: 1,
            ..EffectEvent::default()
        }))
        .unwrap();
    publish(
        &mut runtime,
        false,
        NativeTarget::Client(client),
        b"\x0fhealthy\0",
    );
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(25).unwrap(),
        vec![],
    )
    .unwrap();
    // Source has no sound/effect backend, using the same false defaults as an
    // unloaded live capability. Counters report rejection, never playback.
    let mut source = Source::default();
    let first = host.frame(&mut source, true);
    assert_eq!((first.output.sounds, first.output.effects), (1, 1));
    assert_eq!(
        (
            first.output.unhandled_sounds,
            first.output.unhandled_effects
        ),
        (1, 1)
    );
    assert!(host.runtime.server.events.is_empty());
    assert_eq!(source.packets.len(), 1);
    let mut peer =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
    assert_eq!(receive(&mut peer, &source.packets[0].1), b"\x0fhealthy\0");
    source.now = EventTime::from_milliseconds(25);
    source.packets.clear();
    let second = host.frame(&mut source, true);
    assert_eq!(second.server_ticks, 1);
    assert_eq!((second.output.sounds, second.output.effects), (0, 0));
    assert_eq!(
        (
            second.output.unhandled_sounds,
            second.output.unhandled_effects
        ),
        (0, 0)
    );
    assert!(source.packets.is_empty());
    assert_eq!(second.output.native_disconnected, 0);
}

#[test]
fn reliable_native_pages_retire_only_on_the_selected_peers_channel_ack() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let first = remote(&mut runtime, Protocol::Quake2_34, 2000);
    let _foreign = remote(&mut runtime, Protocol::Quake3_68, 2001);
    let mut source = Source::default();
    source.now = EventTime(10);
    publish(&mut runtime, true, NativeTarget::All, b"\x0fraw\x80\x81\0");
    let counts = dispatch(&mut runtime, &mut source);
    assert_eq!(counts.native_unsupported, 1);
    assert_eq!(runtime.server.events.len(), 1);
    assert_eq!(source.packets.len(), 1);
    let mut peer =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
    peer.set_qport(2000);
    assert_eq!(
        receive(&mut peer, &source.packets[0].1),
        b"\x0fraw\x80\x81\0"
    );
    let ack = peer
        .prepare(None, EventTime(40))
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    peer.submitted(EventTime(40)).unwrap();
    runtime.network.receive(
        0,
        Peer::Socket(([127, 0, 0, 1], 2000).into()),
        &ack,
        EventTime(50),
        |client, endpoint, incoming| {
            assert_eq!(client, first);
            assert_eq!(endpoint, Endpoint::Server);
            if let Incoming::Acknowledged {
                receipt,
                output: Some(id),
            } = incoming
            {
                runtime.server.events.acknowledge(id, receipt);
            }
            std::ops::ControlFlow::Continue(())
        },
    );
    assert!(runtime.server.events.is_empty());
}
#[test]
fn native_layout_reaches_the_local_hud_through_channel_queue_and_client_ring() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let policy = ClientPolicy::select(Some(RuleSetId::Quake2), None, None, None).unwrap();
    let id = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Default::default(),
                angles: Default::default(),
                entity: 0,
                fixture_fallback: false,
            },
            policy,
            Protocol::Quake2_34,
            0,
        )
        .unwrap();
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[0] = Some(id);
    publish(
        &mut host.runtime,
        true,
        NativeTarget::Client(id),
        b"\x04raw\x80 layout\0",
    );
    let mut source = Source::default();
    host.frame(&mut source, true);
    assert!(host.runtime.server.clients[0].hud.layout_text.is_none());
    source.now = EventTime(20_000_000);
    host.frame(&mut source, true);
    let lease = host.runtime.server.clients[0]
        .hud
        .layout_text
        .as_ref()
        .unwrap();
    assert_eq!(
        host.runtime.server.events.texts.get(lease.id()).unwrap(),
        b"raw\x80 layout"
    );
    assert!(host.runtime.network.delivered > 0);
}

#[test]
fn separate_stats_and_set_angles_reach_client_state_only_through_queued_native_packets() {
    use qa_core::events::FrameEvent;
    for protocol in [Protocol::NetQuake15, Protocol::QuakeWorld28] {
        let mut runtime = Runtime::load(1, []).unwrap();
        let policy = ClientPolicy {
            client: RuleSetId::Quake2Rerelease,
            movement: RuleSetId::Quake3,
            trace: RuleSetId::Quake2,
        };
        let id = runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                policy,
                protocol,
                0,
            )
            .unwrap();
        let binding = runtime.catalog.hud.values.native(protocol.rule_set()).stats[4];
        let source_binding = runtime
            .catalog
            .hud
            .values
            .native(RuleSetId::QuakeWorld)
            .stats[4];
        let foreign = runtime.catalog.hud.values.native(policy.client).stats[4];
        let delivery = NativeDelivery {
            module: ModuleId(7),
            source: RuleSetId::QuakeWorld,
            target: NativeTarget::Client(id),
            origin: Vec3::default(),
            reliable: true,
            dupe_key: 0,
            component: 0,
            emission: None,
            check_portals: false,
        };
        runtime
            .server
            .events
            .push_delivered(
                delivery,
                FrameEvent::PlayerValue {
                    id: source_binding.id,
                    value: NumericValue::integer(333),
                },
            )
            .unwrap();
        let angles = Vec3([45.0, 90.0, -45.0]);
        runtime
            .server
            .events
            .push_delivered(
                NativeDelivery {
                    reliable: false,
                    ..delivery
                },
                FrameEvent::SetAngles(angles),
            )
            .unwrap();
        assert_eq!(
            runtime.prediction[0].player.values.get(binding.id),
            Some(NumericValue::default())
        );
        assert_eq!(runtime.prediction[0].player.view_angles, Vec3::default());
        let mut host = FrameHost::load(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![],
        )
        .unwrap();
        host.local_clients[0] = Some(id);
        let mut source = Source::default();
        // Receive the discrete services before any subsequent player snapshot.
        // The QW request also opens its native server reply window.
        for step in 1..=4 {
            source.now = EventTime::from_milliseconds(step * 20);
            assert!(
                host.runtime
                    .send_local_command(id, &UserCmd::default(), source.now,)
            );
            host.drain(&mut Default::default());
            dispatch(&mut host.runtime, &mut source);
            host.drain(&mut Default::default());
        }
        assert_eq!(host.runtime.prediction[0].player.armor, 333);
        assert_eq!(host.runtime.prediction[0].player.view_angles, angles);
        for frame in 0..5 {
            source.now = EventTime::from_milliseconds(100 + frame * 20);
            host.frame(&mut source, true);
        }
        let player = &host.runtime.prediction[0].player;
        // NQ's later clientdata carries authoritative armor separately; QW
        // playerinfo has no armor field and retains its stat service value.
        let armor = if protocol == Protocol::NetQuake15 {
            0
        } else {
            333
        };
        assert_eq!(
            player.values.get(binding.id),
            Some(NumericValue::integer(armor))
        );
        assert_eq!(player.values.get(foreign.id), Some(NumericValue::default()));
        assert_eq!(player.armor, armor);
        assert_eq!(player.view_angles, angles);
        assert_eq!(
            (player.movement_rules, player.trace_rules),
            (policy.movement, policy.trace)
        );
        assert_eq!(
            host.runtime.server.clients[0].player.values.get(binding.id),
            Some(NumericValue::default())
        );
        assert_eq!(
            host.runtime.server.clients[0].hud.values.get(binding.id),
            Some(NumericValue::integer(armor))
        );
        assert_eq!(host.runtime.server.clients[0].hud.armor, armor);
        assert!(host.runtime.network.delivered > 0);
    }
}
#[test]
fn native_multicast_uses_compiled_sight_hearing_and_portal_connectivity() {
    use qa_world::visibility::{PvsRows, SurfaceSpan, VisLeaf, VisNode, VisibilityWorld};
    let mut runtime = Runtime::load(2, []).unwrap();
    let first = remote(&mut runtime, Protocol::Quake2_34, 2000);
    let second = remote(&mut runtime, Protocol::Quake2_34, 2001);
    runtime.server.clients[first.0 as usize]
        .player
        .body
        .position = Vec3([1.0, 0.0, 0.0]);
    runtime.server.clients[second.0 as usize]
        .player
        .body
        .position = Vec3([-1.0, 0.0, 0.0]);
    let bounds = Bounds {
        mins: Vec3([-10.0; 3]),
        maxs: Vec3([10.0; 3]),
    };
    let world = VisibilityWorld::load(
        vec![Plane::oriented(Vec3([1.0, 0.0, 0.0]), 0.0)],
        vec![VisNode {
            plane: 0,
            children: [-1, -2],
            bounds,
            surfaces: SurfaceSpan::default(),
        }],
        (0..2)
            .map(|i| VisLeaf {
                selector: Some(i),
                area: Some(i),
                solid: false,
                bounds,
                surfaces: SurfaceSpan::default(),
            })
            .collect(),
        vec![],
        0,
        0,
        PvsRows::load(vec![Some(0), Some(1)], vec![1, 2, 3])
            .unwrap()
            .with_hearing(vec![Some(2); 2])
            .unwrap(),
    )
    .unwrap();
    let portals = qa_world::portals::AreaPortals::load(
        2,
        1,
        vec![qa_world::portals::Portal {
            number: 0,
            first: 0,
            second: 1,
        }],
    )
    .unwrap();
    runtime.collision = Some(
        qa_app::WorldCollision::new(
            &runtime.geometry,
            GeometryId {
                slot: 0,
                generation: 1,
            },
            0,
        )
        .with_visibility(std::sync::Arc::new(world))
        .with_portals(Some(portals)),
    );
    let mut source = Source::default();
    publish(&mut runtime, false, NativeTarget::Hearing, b"\x0fclosed\0");
    assert_eq!(dispatch(&mut runtime, &mut source).native_filtered, 1);
    assert_eq!(source.packets.len(), 1);
    runtime
        .collision
        .as_mut()
        .unwrap()
        .portals
        .as_mut()
        .unwrap()
        .set(0, true)
        .unwrap();
    source.packets.clear();
    publish(&mut runtime, false, NativeTarget::Sight, b"\x0fsight\0");
    assert_eq!(dispatch(&mut runtime, &mut source).native_filtered, 1);
    assert_eq!(source.packets.len(), 1);
    source.packets.clear();
    publish(&mut runtime, false, NativeTarget::Hearing, b"\x0fhearing\0");
    assert_eq!(dispatch(&mut runtime, &mut source).native_filtered, 0);
    assert_eq!(source.packets.len(), 2);
    assert!(runtime.server.events.is_empty());
}

#[test]
fn fragmented_rerelease_output_retries_exact_bytes_and_oversized_payloads_retire_as_unsupported() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let id = remote(&mut runtime, Protocol::Quake2Repro1038, 2000);
    let delivery = NativeDelivery {
        module: ModuleId(7),
        source: RuleSetId::Quake2Rerelease,
        target: NativeTarget::Client(id),
        origin: Default::default(),
        reliable: false,
        dupe_key: 0,
        component: 0,
        emission: None,
        check_portals: true,
    };
    let bytes: Vec<_> = (0..4000).map(|i| i as u8).collect();
    let sequence = runtime
        .server
        .events
        .native_message(delivery, &bytes)
        .unwrap();
    let mut source = Source {
        reject: Some(2000),
        ..Default::default()
    };
    assert_eq!(dispatch(&mut runtime, &mut source).native_blocked, 1);
    assert_eq!(
        runtime
            .network
            .get(id, Endpoint::Server)
            .unwrap()
            .pending_output,
        Some(sequence)
    );
    assert_eq!(runtime.server.events.len(), 1);
    source.reject = None;
    dispatch(&mut runtime, &mut source);
    assert_eq!(source.packets.len(), 4);
    let mut peer = Channel::load(
        Protocol::Quake2Repro1038.channel(),
        Endpoint::Client,
        8192,
        16,
    )
    .unwrap();
    let mut complete = 0;
    for (_, packet) in &source.packets {
        if let Delivery::Payload(body) = peer.receive(packet, EventTime(1)).unwrap().delivery {
            assert_eq!(body, bytes);
            complete += 1;
        }
    }
    assert_eq!(complete, 1);
    assert!(runtime.server.events.is_empty());
    runtime
        .server
        .events
        .native_message(delivery, &[0xff; 9000])
        .unwrap();
    assert_eq!(dispatch(&mut runtime, &mut source).native_unsupported, 1);
    assert!(runtime.server.events.is_empty());
    assert!(
        runtime
            .network
            .get(id, Endpoint::Server)
            .unwrap()
            .pending_output
            .is_none()
    );
}

#[test]
fn stalled_local_output_does_not_republish_unchanged_hud_records() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let policy = ClientPolicy::select(Some(RuleSetId::Quake2), None, None, None).unwrap();
    let ids = std::array::from_fn::<_, 2, _>(|seat| {
        runtime
            .connect_local(
                SeatId::new(seat as u8).unwrap(),
                SpawnAnchor {
                    position: Default::default(),
                    angles: Default::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                policy,
                Protocol::Quake2_34,
                seat as u32,
            )
            .unwrap()
    });
    runtime
        .network
        .get_mut(ids[0], Endpoint::Server)
        .unwrap()
        .route
        .peer = Peer::Socket(([127, 0, 0, 1], 2000).into());
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(50).unwrap(),
        vec![],
    )
    .unwrap();
    host.local_clients[..2].copy_from_slice(&ids.map(Some));
    let weapon = host.runtime.catalog.registry.weapons[0].id;
    for (seat, id) in ids.into_iter().enumerate() {
        for player in [
            &mut host.runtime.prediction[seat].player,
            &mut host.runtime.server.clients[id.0 as usize].player,
        ] {
            player.health = 75;
            player.inventory.item_counts[1] = 4;
            player.inventory.item_acquired_at[1] = 1.25;
            player.inventory.weapon = weapon;
            player.inventory.powerup_until[2] = 3.5;
            player.inventory.collectibles = 7;
        }
    }
    publish(
        &mut host.runtime,
        false,
        NativeTarget::Client(ids[0]),
        b"\x0fheld\0",
    );
    let mut source = Source {
        reject: Some(2000),
        ..Default::default()
    };
    qa_app::output::dispatch(
        &mut host.runtime,
        &mut source,
        &mut host.console.scrollback,
        &host.local_clients,
        EventTime(0),
        3.0,
        2.0,
    );
    assert!(
        host.runtime
            .network
            .get(ids[0], Endpoint::Server)
            .unwrap()
            .pending_output
            .is_some()
    );
    for frame in 1..=10 {
        source.now = EventTime::from_milliseconds(frame * 50);
        host.frame(&mut source, true);
        assert_eq!(host.runtime.server.events.len(), 9);
        let stalled = &host.runtime.server.clients[ids[0].0 as usize];
        assert_eq!(stalled.hud.health, 75);
        assert_eq!(stalled.hud.inventory.item_counts[1], 0);
        assert_eq!(host.runtime.server.events.rejected_publications(), 0);
        let healthy = &host.runtime.server.clients[ids[1].0 as usize].hud;
        assert_eq!(
            (
                healthy.inventory.item_counts[1],
                healthy.inventory.weapon,
                healthy.inventory.collectibles
            ),
            (4, weapon, 7)
        );
    }
    assert!(host.runtime.server.world_frame >= 9);
    host.runtime.prediction[0].player.inventory.item_counts[1] = 5;
    for frame in 11..=15 {
        source.now = EventTime::from_milliseconds(frame * 50);
        host.frame(&mut source, true);
        assert_eq!(host.runtime.server.events.len(), 10);
    }
    source.reject = None;
    let now = source.now;
    qa_app::output::dispatch(
        &mut host.runtime,
        &mut source,
        &mut host.console.scrollback,
        &host.local_clients,
        now,
        3.0,
        2.0,
    );
    assert!(host.runtime.server.events.is_empty());
    let hud = &host.runtime.server.clients[ids[0].0 as usize].hud;
    assert_eq!(
        (
            hud.inventory.item_counts[1],
            hud.inventory.weapon,
            hud.inventory.powerup_until[2],
            hud.inventory.collectibles
        ),
        (5, weapon, 3.5, 7)
    );
}

#[test]
fn snapshots_leave_a_rejected_native_datagram_for_its_ring_consumer() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let policy = ClientPolicy::select(Some(RuleSetId::Quake2), None, None, None).unwrap();
    let id = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Default::default(),
                angles: Default::default(),
                entity: 0,
                fixture_fallback: false,
            },
            policy,
            Protocol::Quake2_34,
            0,
        )
        .unwrap();
    while runtime.loopback.send(Endpoint::Server, id, &[0]).is_ok() {}
    publish(&mut runtime, false, NativeTarget::Client(id), b"\x0fheld\0");
    let mut source = Source::default();
    let mut scrollback = qa_console::presentation::Scrollback::default();
    qa_app::output::dispatch(
        &mut runtime,
        &mut source,
        &mut scrollback,
        &[Some(id), None, None, None],
        EventTime(1),
        3.0,
        2.0,
    );
    let channel = runtime.network.get(id, Endpoint::Server).unwrap();
    assert!(channel.pending_output.is_some());
    let prepared = channel.channel.pending_packet().unwrap().bytes.to_vec();
    assert!(!runtime.send_snapshot(id, EventTime(100_000_000), None, &mut source, |_, _| None));
    assert_eq!(
        runtime
            .network
            .get(id, Endpoint::Server)
            .unwrap()
            .channel
            .pending_packet()
            .unwrap()
            .bytes,
        prepared
    );
    runtime.loopback.clear_client(id);
    qa_app::output::dispatch(
        &mut runtime,
        &mut source,
        &mut scrollback,
        &[Some(id), None, None, None],
        EventTime(2),
        3.0,
        2.0,
    );
    assert!(runtime.server.events.is_empty());
    assert!(
        runtime
            .network
            .get(id, Endpoint::Server)
            .unwrap()
            .pending_output
            .is_none()
    );
}

#[test]
fn dropped_native_payload_releases_dupe_claim_for_healthy_negotiated_peer() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let a = remote(&mut runtime, Protocol::Quake2_34, 2000);
    let b = remote(&mut runtime, Protocol::Quake2_34, 2001);
    assert!(runtime.output_duplicates.negotiate(a, Some(1)));
    assert!(runtime.output_duplicates.negotiate(b, Some(1)));
    runtime
        .network
        .get_mut(a, Endpoint::Server)
        .unwrap()
        .channel
        .queue_reliable(&[0u8; 1388])
        .unwrap();
    runtime
        .server
        .events
        .native_message(
            NativeDelivery {
                module: ModuleId(7),
                source: RuleSetId::Quake2,
                target: NativeTarget::All,
                origin: Vec3::default(),
                reliable: false,
                dupe_key: 42,
                component: 0,
                emission: Some(NativeEmission {
                    module_lifetime: EntityId {
                        slot: 8,
                        generation: 1,
                    },
                    provider_frame: 17,
                }),
                check_portals: true,
            },
            b"\x0fhealthy\0",
        )
        .unwrap();
    let mut source = Source::default();
    let counts = dispatch(&mut runtime, &mut source);
    assert_eq!(counts.native_dropped, 1);
    assert_eq!(counts.native_dupe_suppressed, 0);
    assert!(runtime.server.events.is_empty());
    assert!(
        runtime
            .network
            .get(a, Endpoint::Server)
            .unwrap()
            .pending_output
            .is_none()
    );
    let mut peer =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
    peer.set_qport(2001);
    let bytes = &source
        .packets
        .iter()
        .find(|(port, _)| *port == 2001)
        .unwrap()
        .1;
    assert_eq!(receive(&mut peer, bytes), b"\x0fhealthy\0");
}
#[test]
fn reliable_dupe_waits_for_real_ack_without_stalling_an_independent_peer() {
    let mut runtime = Runtime::load(3, []).unwrap();
    let a = remote(&mut runtime, Protocol::Quake2_34, 2000);
    let b = remote(&mut runtime, Protocol::Quake2_34, 2001);
    let healthy = remote(&mut runtime, Protocol::Quake2_34, 2002);
    runtime.output_duplicates.negotiate(a, Some(1));
    runtime.output_duplicates.negotiate(b, Some(1));
    runtime
        .server
        .events
        .native_message(
            NativeDelivery {
                module: ModuleId(7),
                source: RuleSetId::Quake2,
                target: NativeTarget::All,
                origin: Vec3::default(),
                reliable: true,
                dupe_key: 42,
                component: 0,
                emission: Some(NativeEmission {
                    module_lifetime: EntityId {
                        slot: 8,
                        generation: 1,
                    },
                    provider_frame: 17,
                }),
                check_portals: true,
            },
            b"\x0fACK\0",
        )
        .unwrap();
    let mut source = Source::default();
    dispatch(&mut runtime, &mut source);
    assert!(source.packets.iter().any(|(port, _)| *port == 2000));
    assert!(!source.packets.iter().any(|(port, _)| *port == 2001));
    assert!(
        source.packets.iter().any(|(port, _)| *port == 2002),
        "separate physical peer progresses"
    );
    let mut peer =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
    peer.set_qport(2000);
    receive(
        &mut peer,
        &source
            .packets
            .iter()
            .find(|(port, _)| *port == 2000)
            .unwrap()
            .1,
    );
    let ack = peer
        .prepare(None, EventTime(40))
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    peer.submitted(EventTime(40)).unwrap();
    runtime.network.receive(
        0,
        Peer::Socket(([127, 0, 0, 1], 2000).into()),
        &ack,
        EventTime(50),
        |_, _, incoming| {
            if let Incoming::Acknowledged {
                receipt,
                output: Some(id),
            } = incoming
            {
                runtime.output_duplicates.acknowledged(id, receipt);
                runtime.server.events.acknowledge(id, receipt);
            }
            std::ops::ControlFlow::Continue(())
        },
    );
    let counts = dispatch(&mut runtime, &mut source);
    assert_eq!(counts.native_dupe_suppressed, 1);
    assert!(!source.packets.iter().any(|(port, _)| *port == 2001));
    assert!(
        runtime.server.events.len() > 0,
        "independent healthy peer still needs its own native ACK"
    );
    assert_ne!(a, healthy);
}

#[test]
fn same_frame_remote_slot_reconnect_forgets_sent_dupe_and_negotiation() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let old = remote(&mut runtime, Protocol::Quake2_34, 2000);
    let delivery = NativeDelivery {
        module: ModuleId(7),
        source: RuleSetId::Quake2,
        target: NativeTarget::All,
        origin: Vec3::default(),
        reliable: false,
        dupe_key: 42,
        component: 0,
        emission: Some(NativeEmission {
            module_lifetime: EntityId {
                slot: 8,
                generation: 1,
            },
            provider_frame: 17,
        }),
        check_portals: true,
    };
    runtime
        .server
        .events
        .native_message(delivery, b"\x0fold\0")
        .unwrap();
    let mut source = Source::default();
    assert_eq!(
        dispatch(&mut runtime, &mut source).native_dupe_suppressed,
        0
    );
    runtime.output_duplicates.negotiate(old, Some(7));
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        Vec::new(),
    )
    .unwrap();
    assert!(
        host.client_string_command(old, RuleSetId::Quake2, b"disconnect")
            .unwrap()
            .is_break()
    );
    let new = remote(&mut host.runtime, Protocol::Quake2_34, 2001);
    assert_eq!(old, new, "common client slot is reused");
    assert_eq!(
        host.runtime.output_duplicates.group(new, false),
        qa_app::output::duplicates::PhysicalGroup::Peer(new)
    );
    host.runtime
        .server
        .events
        .native_message(delivery, b"\x0fnew\0")
        .unwrap();
    assert_eq!(
        dispatch(&mut host.runtime, &mut source).native_dupe_suppressed,
        0
    );
    assert!(source.packets.iter().any(|(port, _)| *port == 2001));
}

fn resync_dupe_fixture(protocol: Protocol) -> (FrameHost, [ClientId; 2], ClientId) {
    use qa_core::events::{EventRing, OutputTarget};
    let mut runtime = Runtime::load(3, []).unwrap();
    runtime.server.events = EventRing::load(8, 4, 40, OUTPUT_PAGE_BYTES).unwrap();
    runtime.server.presentation = runtime
        .server
        .events
        .bind(OutputTarget::Presentation)
        .unwrap();
    let policy = ClientPolicy::select(Some(protocol.rule_set()), None, None, None).unwrap();
    let local = std::array::from_fn(|seat| {
        let id = runtime
            .connect_local(
                SeatId::new(seat as u8).unwrap(),
                SpawnAnchor {
                    position: Default::default(),
                    angles: Default::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                policy,
                protocol,
                seat as u32,
            )
            .unwrap();
        runtime.server.clients[id.0 as usize].module = ModuleId(7);
        assert!(runtime.server.bind_client_native(
            id,
            NativeEntity {
                module: ModuleId(7),
                slot: seat as i32 + 1,
            },
        ));
        // The connection remains an actual Local physical presentation. The
        // owned test transport exposes admission and reconstructed native bytes.
        let connection = runtime.network.get_mut(id, Endpoint::Server).unwrap();
        connection.route.socket = 0;
        connection.route.peer = Peer::Socket(([127, 0, 0, 1], 2000 + seat as u16).into());
        connection.channel.set_qport(2000 + seat as u16);
        id
    });
    let healthy = remote(&mut runtime, protocol, 2002);
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(50).unwrap(),
        vec![],
    )
    .unwrap();
    host.local_clients[..2].copy_from_slice(&local.map(Some));
    (host, local, healthy)
}

fn publish_keyed(runtime: &mut Runtime, target: ClientId, reliable: bool, bytes: &[u8]) -> u64 {
    runtime
        .server
        .events
        .native_message(
            NativeDelivery {
                module: ModuleId(7),
                source: RuleSetId::Quake2Rerelease,
                target: NativeTarget::Client(target),
                origin: Vec3::default(),
                reliable,
                dupe_key: 42,
                component: 0,
                emission: Some(NativeEmission {
                    module_lifetime: EntityId {
                        slot: 8,
                        generation: 1,
                    },
                    provider_frame: 17,
                }),
                check_portals: true,
            },
            bytes,
        )
        .unwrap()
}

fn overflow_only(runtime: &mut Runtime, stalled: ClientId, healthy: ClientId) {
    use qa_core::events::{FrameEvent, OutputTarget};
    let old = runtime.server.clients[stalled.0 as usize].output.unwrap();
    let other = runtime.server.clients[healthy.0 as usize].output.unwrap();
    let capacity = runtime.server.events.capacity();
    for value in 0..capacity {
        runtime
            .server
            .events
            .push_to(
                OutputTarget::Client(stalled),
                FrameEvent::Hud(HudEvent {
                    client: stalled,
                    change: HudChange::Collectibles(value as u64),
                }),
            )
            .unwrap();
        if runtime.server.events.needs_resync(old) {
            break;
        }
    }
    assert!(runtime.server.events.needs_resync(old));
    assert!(!runtime.server.events.needs_resync(other));
    assert_eq!(runtime.server.events.counters(old).unwrap().overflow, 1);
}

fn dispatch_local(host: &mut FrameHost, source: &mut Source) -> qa_app::output::OutputCounts {
    qa_app::output::dispatch(
        &mut host.runtime,
        source,
        &mut host.console.scrollback,
        &host.local_clients,
        source.now,
        3.0,
        2.0,
    )
}

#[test]
fn local_fragment_resync_keeps_the_physical_claim_until_the_old_flight_completes() {
    let (mut host, [a, b], healthy) = resync_dupe_fixture(Protocol::Quake2Repro1038);
    let mut bytes = vec![b'x'; 3002];
    bytes[0] = 15;
    *bytes.last_mut().unwrap() = 0;
    let sequence = publish_keyed(&mut host.runtime, a, false, &bytes);
    publish_keyed(&mut host.runtime, b, false, &bytes);
    publish(
        &mut host.runtime,
        false,
        NativeTarget::Client(healthy),
        b"\x0fhealthy\0",
    );
    let mut source = Source {
        accepted_limit: Some((2000, 1)),
        ..Default::default()
    };
    dispatch_local(&mut host, &mut source);
    let connection = host.runtime.network.get(a, Endpoint::Server).unwrap();
    assert_eq!(connection.pending_output, Some(sequence));
    assert!(connection.channel.pending_fragments());
    assert!(source.packets.iter().any(|(port, _)| *port == 2002));
    assert!(!source.packets.iter().any(|(port, _)| *port == 2001));

    let mut peer = Channel::load(
        Protocol::Quake2Repro1038.channel(),
        Endpoint::Client,
        8192,
        16,
    )
    .unwrap();
    let initial = source
        .packets
        .iter()
        .find(|(port, _)| *port == 2000)
        .unwrap();
    assert!(matches!(
        peer.receive(&initial.1, EventTime(1)).unwrap().delivery,
        Delivery::Pending
    ));
    source.now = EventTime::from_milliseconds(100);
    host.frame(&mut source, true);
    source.now = EventTime::from_milliseconds(150);
    assert!(host.frame(&mut source, true).server_ticks > 0);
    overflow_only(&mut host.runtime, a, b);
    source.accepted_limit = None;
    let before = source.packets.len();
    let counts = dispatch_local(&mut host, &mut source);
    let mut completed = 0;
    for (port, packet) in &source.packets[before..] {
        if *port == 2000
            && let Delivery::Payload(payload) = peer.receive(packet, source.now).unwrap().delivery
        {
            if payload == bytes {
                completed += 1;
            }
        }
    }
    assert_eq!(completed, 1, "the actual old native message completes");
    let mut sibling = Channel::load(
        Protocol::Quake2Repro1038.channel(),
        Endpoint::Client,
        8192,
        16,
    )
    .unwrap();
    let sibling_messages = source.packets.iter().filter(|(port, _)| *port == 2001)
        .filter(|(_, packet)| matches!(sibling.receive(packet, source.now).unwrap().delivery, Delivery::Payload(payload) if payload == bytes))
        .count();
    assert_eq!(
        sibling_messages, 0,
        "one physical group cannot reconstruct the same message twice"
    );
    assert_eq!(counts.native_dupe_suppressed, 1);
}

#[test]
fn local_reliable_resync_observes_the_real_ack_after_the_ring_claim_was_cancelled() {
    let (mut host, [a, b], healthy) = resync_dupe_fixture(Protocol::Quake2Repro1038);
    let bytes = b"\x0freliable once\0";
    publish_keyed(&mut host.runtime, a, true, bytes);
    publish_keyed(&mut host.runtime, b, true, bytes);
    publish(
        &mut host.runtime,
        false,
        NativeTarget::Client(healthy),
        b"\x0fhealthy\0",
    );
    let mut source = Source::default();
    dispatch_local(&mut host, &mut source);
    assert!(source.packets.iter().any(|(port, _)| *port == 2002));
    assert!(!source.packets.iter().any(|(port, _)| *port == 2001));
    assert!(
        host.runtime
            .network
            .get(a, Endpoint::Server)
            .unwrap()
            .pending_output
            .is_none()
    );
    let mut peer = Channel::load(
        Protocol::Quake2Repro1038.channel(),
        Endpoint::Client,
        8192,
        16,
    )
    .unwrap();
    peer.set_qport(2000);
    let initial = source
        .packets
        .iter()
        .find(|(port, _)| *port == 2000)
        .unwrap();
    assert_eq!(receive(&mut peer, &initial.1), bytes);
    source.now = EventTime::from_milliseconds(100);
    host.frame(&mut source, true);
    source.now = EventTime::from_milliseconds(150);
    assert!(host.frame(&mut source, true).server_ticks > 0);
    overflow_only(&mut host.runtime, a, b);
    dispatch_local(&mut host, &mut source);
    assert!(
        native_message_count(&source, 2001, bytes) == 0,
        "withheld real ACK must keep the other Local seat pending"
    );

    let old = host.runtime.server.clients[a.0 as usize].output.unwrap();
    assert!(host.runtime.server.events.needs_resync(old));
    let ack = peer
        .prepare(None, source.now)
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    peer.submitted(source.now).unwrap();
    host.queue
        .push(SysEvent {
            time: source.now,
            kind: EventKind::Packet {
                socket: 0,
                from: Peer::Socket(([127, 0, 0, 1], 2000).into()),
                bytes: &ack,
            },
        })
        .unwrap();
    host.drain(&mut qa_app::host::FrameResult::default());
    assert_eq!(
        host.runtime.network.acknowledged, 1,
        "an actual native ACK reaches the host"
    );
    assert_eq!(
        dispatch_local(&mut host, &mut source).native_dupe_suppressed,
        1
    );
    assert_eq!(native_message_count(&source, 2001, bytes), 0);
}

fn native_message_count(source: &Source, port: u16, bytes: &[u8]) -> usize {
    let mut channel = Channel::load(
        Protocol::Quake2Repro1038.channel(),
        Endpoint::Client,
        8192,
        16,
    )
    .unwrap();
    source
        .packets
        .iter()
        .filter(|(sent_port, _)| *sent_port == port)
        .filter(
            |(_, packet)| match channel.receive(packet, source.now).unwrap().delivery {
                Delivery::Payload(payload) => {
                    payload.windows(bytes.len()).any(|part| part == bytes)
                }
                _ => false,
            },
        )
        .count()
}

#[test]
fn local_owner_disconnect_releases_dead_flights_but_preserves_completed_physical_delivery() {
    for completed in [false, true] {
        let (mut host, [a, b], _) = resync_dupe_fixture(Protocol::Quake2Repro1038);
        let bytes = b"\x0fdisconnect once\0";
        publish_keyed(&mut host.runtime, a, !completed, bytes);
        let mut source = Source {
            reject: (!completed).then_some(2000),
            ..Default::default()
        };
        dispatch_local(&mut host, &mut source);
        let mut peer = Channel::load(
            Protocol::Quake2Repro1038.channel(),
            Endpoint::Client,
            8192,
            16,
        )
        .unwrap();
        if completed {
            let packet = source
                .packets
                .iter()
                .find(|(port, _)| *port == 2000)
                .unwrap();
            assert_eq!(receive(&mut peer, &packet.1), bytes);
        } else {
            assert!(!source.packets.iter().any(|(port, _)| *port == 2000));
            assert!(
                host.runtime
                    .network
                    .get(a, Endpoint::Server)
                    .unwrap()
                    .channel
                    .has_output()
            );
        }
        assert!(
            host.client_string_command(a, RuleSetId::Quake2Rerelease, b"disconnect")
                .unwrap()
                .is_break()
        );
        assert!(host.runtime.network.get(a, Endpoint::Server).is_none());
        publish_keyed(&mut host.runtime, b, !completed, bytes);
        let counts = dispatch_local(&mut host, &mut source);
        assert_eq!(counts.native_dupe_suppressed, u64::from(completed));
        let packet = source.packets.iter().find(|(port, _)| *port == 2001);
        if completed {
            assert!(
                packet.is_none(),
                "a finished Engine delivery survives its owner disconnect"
            );
        } else {
            assert_eq!(
                receive(&mut peer, &packet.unwrap().1),
                bytes,
                "the dead channel no longer owns a pending physical claim"
            );
        }
    }
}

fn publish_typed_prints(
    runtime: &mut Runtime,
    delivery: NativeDelivery,
    bytes: &[u8],
) -> std::ops::Range<u64> {
    let protocol = Protocol::native(delivery.source);
    let mut rest = bytes;
    let mut count = 0;
    while !rest.is_empty() {
        let (_, consumed) = qa_network::outputs::read_native_service(protocol, rest).unwrap();
        count += 1;
        rest = &rest[consumed..];
    }
    runtime
        .server
        .events
        .publish_group(count, |group| {
            let mut rest = bytes;
            while !rest.is_empty() {
                let component = bytes.len() - rest.len();
                let (service, consumed) =
                    qa_network::outputs::read_native_service(protocol, rest).unwrap();
                let delivery = NativeDelivery {
                    component: component as u32,
                    ..delivery
                };
                match service {
                    qa_network::outputs::NativeService::Print(print) => {
                        let first = print.text.as_ptr() as usize - rest.as_ptr() as usize;
                        group.print(
                            delivery,
                            print.kind,
                            print.level.unwrap_or(print.kind.default_level()),
                            &rest[..consumed],
                            IndexSpan {
                                first: first as u32,
                                count: print.text.len() as u32,
                            },
                            consumed as u32,
                        )?;
                    }
                    qa_network::outputs::NativeService::Cue(qa_network::outputs::Cue::Sound(
                        sound,
                    )) => {
                        group.push_delivered(
                            delivery,
                            qa_core::events::FrameEvent::Sound(sound.into_event(
                                SoundId(1),
                                qa_gameplay::rules::sound_policy(delivery.source),
                                None,
                                Vec3::default(),
                                None,
                                SoundAction::Play,
                            )),
                        )?;
                    }
                    _ => {
                        group.native_message_parts(delivery, &rest[..consumed], &[])?;
                    }
                }
                rest = &rest[consumed..];
            }
            Ok(())
        })
        .unwrap()
}

#[test]
fn typed_native_prints_queue_separate_q3_commands_and_retire_only_on_real_ack() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let target = remote(&mut runtime, Protocol::Quake3_68, 2000);
    let healthy = remote(&mut runtime, Protocol::Quake2_34, 2001);
    let owner = runtime.server.clients[target.0 as usize].output.unwrap();
    // The one known muzzle flash and unrepresentable layout are separate drops;
    // they cannot prevent the recognized following print from being queued.
    let range = publish_typed_prints(
        &mut runtime,
        NativeDelivery {
            module: ModuleId(7),
            source: RuleSetId::Quake2,
            target: NativeTarget::Client(target),
            origin: Vec3::default(),
            reliable: false,
            dupe_key: 0,
            component: 0,
            emission: None,
            check_portals: false,
        },
        b"\x0a\x00first\x80%\0\x09\0\x01\x01\x01\0\0\x04layout\0\x0fsecond\0\x06",
    );
    publish(
        &mut runtime,
        false,
        NativeTarget::Client(healthy),
        b"\x0fhealthy\0",
    );
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(25).unwrap(),
        vec![],
    )
    .unwrap();
    let mut source = Source::default();
    let first = host.frame(&mut source, true);
    assert_eq!(first.output.native_unsupported, 3);
    let state = host
        .runtime
        .network
        .get(target, Endpoint::Server)
        .unwrap()
        .channel
        .command_state()
        .unwrap();
    assert_eq!(
        (state.queued, state.submitted, state.acknowledged),
        (2, 2, 0)
    );
    assert!(host.runtime.server.events.retained_for(owner, range.start));
    assert!(
        host.runtime
            .server
            .events
            .retained_for(owner, range.end - 2)
    );
    let mut peer =
        Channel::load(Protocol::Quake3_68.channel(), Endpoint::Client, 8192, 16).unwrap();
    peer.set_qport(2000);
    let mut healthy_peer =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
    let mut commands = Vec::new();
    for (port, bytes) in &source.packets {
        if *port == 2000 {
            let received = peer.receive(bytes, EventTime(1)).unwrap();
            let sequence = received.header.sequence;
            if let Delivery::Payload(body) = received.delivery {
                let mut body = body.to_vec();
                peer.decode_server_output(&mut body, sequence, |message| {
                    if let qa_network::channel::commands::ServerOutput::ReliableCommand {
                        sequence,
                        text,
                    } = message
                    {
                        commands.push((sequence, text.to_vec()));
                    }
                })
                .unwrap();
            }
        } else {
            assert_eq!(*port, 2001);
            assert_eq!(receive(&mut healthy_peer, bytes), b"\x0fhealthy\0");
        }
    }
    assert_eq!(
        commands,
        [
            (1, b"print \"first..\"".to_vec()),
            (2, b"cp \"second\"".to_vec())
        ]
    );
    source.now = EventTime::from_milliseconds(25);
    queue_move(&mut host, &mut peer, Protocol::Quake3_68, source.now);
    source.packets.clear();
    let next = host.frame(&mut source, true);
    assert_eq!(next.server_ticks, 1);
    assert_eq!(next.output.native_unsupported, 0);
    assert!(!host.runtime.server.events.retained_for(owner, range.start));
    assert!(
        !host
            .runtime
            .server
            .events
            .retained_for(owner, range.end - 2)
    );
    assert_eq!(
        host.runtime
            .network
            .get(target, Endpoint::Server)
            .unwrap()
            .channel
            .command_state()
            .unwrap()
            .acknowledged,
        2
    );
}

#[test]
fn typed_native_oversized_print_drops_once_and_keeps_following_q3_command() {
    for length in [1015, 9000] {
        let mut runtime = Runtime::load(1, []).unwrap();
        let client = remote(&mut runtime, Protocol::Quake3_68, 2000);
        let owner = runtime.server.clients[client.0 as usize].output.unwrap();
        let mut bytes = vec![10, 2];
        bytes.extend(std::iter::repeat_n(b'x', length));
        bytes.extend_from_slice(b"\0\x0fafter\0");
        let range = publish_typed_prints(
            &mut runtime,
            NativeDelivery {
                module: ModuleId(7),
                source: RuleSetId::Quake2Rerelease,
                target: NativeTarget::Client(client),
                origin: Vec3::default(),
                reliable: false,
                dupe_key: 0,
                component: 0,
                emission: None,
                check_portals: false,
            },
            &bytes,
        );
        let mut source = Source::default();
        let counts = dispatch(&mut runtime, &mut source);
        assert_eq!(counts.native_unsupported, 1);
        assert!(!runtime.server.events.retained_for(owner, range.start));
        assert!(runtime.server.events.retained_for(owner, range.end - 1));
        assert_eq!(
            runtime
                .network
                .get(client, Endpoint::Server)
                .unwrap()
                .channel
                .command_state()
                .unwrap()
                .queued,
            1
        );
        let mut peer =
            Channel::load(Protocol::Quake3_68.channel(), Endpoint::Client, 8192, 16).unwrap();
        peer.set_qport(2000);
        let mut commands = Vec::new();
        for (_, bytes) in &source.packets {
            let received = peer.receive(bytes, EventTime(1)).unwrap();
            let sequence = received.header.sequence;
            if let Delivery::Payload(body) = received.delivery {
                let mut body = body.to_vec();
                peer.decode_server_output(&mut body, sequence, |message| {
                    if let qa_network::channel::commands::ServerOutput::ReliableCommand {
                        text,
                        ..
                    } = message
                    {
                        commands.push(text.to_vec());
                    }
                })
                .unwrap();
            }
        }
        assert_eq!(commands, [b"cp \"after\"".to_vec()]);
    }
}

#[test]
fn classic_full_byte_queue_cannot_reorder_a_large_print_behind_its_smaller_sibling() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let blocked = remote(&mut runtime, Protocol::Quake2_34, 2000);
    let healthy = remote(&mut runtime, Protocol::Quake2_34, 2001);
    let owner = runtime.server.clients[blocked.0 as usize].output.unwrap();
    let connection = runtime.network.get_mut(blocked, Endpoint::Server).unwrap();
    connection.channel =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Server, 100, 16).unwrap();
    connection.channel.set_qport(2000);
    let mut prefix = [b'p'; 90];
    prefix[0] = 10;
    prefix[1] = 2;
    prefix[89] = 0;
    connection.channel.queue_reliable(&prefix).unwrap();
    connection.channel.queue_reliable(&prefix).unwrap(); // 20 bytes remain; only the smaller sibling would fit.
    let mut bytes = vec![10, 2];
    bytes.extend(std::iter::repeat_n(b'l', 97));
    bytes.extend_from_slice(b"\0\x0a\x02small\0");
    let range = publish_typed_prints(
        &mut runtime,
        NativeDelivery {
            module: ModuleId(7),
            source: RuleSetId::Quake2,
            target: NativeTarget::Client(blocked),
            origin: Vec3::default(),
            reliable: true,
            dupe_key: 0,
            component: 0,
            emission: None,
            check_portals: false,
        },
        &bytes,
    );
    publish(
        &mut runtime,
        false,
        NativeTarget::Client(healthy),
        b"\x0fhealthy\0",
    );
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(25).unwrap(),
        vec![],
    )
    .unwrap();
    let mut peer =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
    peer.set_qport(2000);
    let mut healthy_peer =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
    let mut source = Source::default();
    let first = host.frame(&mut source, true);
    assert_eq!(first.output.native_unsupported, 0);
    assert_eq!(
        host.runtime
            .server
            .events
            .counters(owner)
            .unwrap()
            .submissions,
        0
    );
    assert!(host.runtime.server.events.retained_for(owner, range.start));
    assert!(
        host.runtime
            .server
            .events
            .retained_for(owner, range.end - 1)
    );
    let mut delivered = Vec::new();
    for (port, packet) in &source.packets {
        if *port == 2000 {
            delivered.push(receive(&mut peer, packet));
        } else {
            assert_eq!(receive(&mut healthy_peer, packet), b"\x0fhealthy\0");
        }
    }
    assert_eq!(delivered, [prefix.to_vec()]);
    for frame in 1..=3 {
        source.now = EventTime::from_milliseconds(frame * 25);
        queue_move(&mut host, &mut peer, Protocol::Quake2_34, source.now);
        source.packets.clear();
        let result = host.frame(&mut source, true);
        assert_eq!(result.server_ticks, 1);
        assert_eq!(result.output.native_unsupported, 0);
        for (port, packet) in &source.packets {
            if *port == 2000 {
                delivered.push(receive(&mut peer, packet));
            }
        }
    }
    assert_eq!(
        delivered,
        [
            prefix.to_vec(),
            prefix.to_vec(),
            bytes[..100].to_vec(),
            bytes[100..].to_vec()
        ]
    );
    assert_eq!(
        host.runtime
            .server
            .events
            .counters(owner)
            .unwrap()
            .submissions,
        2
    );
}

#[test]
fn typed_print_waits_for_q3_window_space_and_then_its_own_native_ack() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let target = remote(&mut runtime, Protocol::Quake3_68, 2000);
    let healthy = remote(&mut runtime, Protocol::Quake2_34, 2001);
    let owner = runtime.server.clients[target.0 as usize].output.unwrap();
    let channel = &mut runtime
        .network
        .get_mut(target, Endpoint::Server)
        .unwrap()
        .channel;
    for _ in 0..64 {
        channel.queue_reliable(b"print \"occupied\"\0").unwrap();
    }
    let range = publish_typed_prints(
        &mut runtime,
        NativeDelivery {
            module: ModuleId(7),
            source: RuleSetId::Quake2,
            target: NativeTarget::Client(target),
            origin: Vec3::default(),
            reliable: true,
            dupe_key: 0,
            component: 0,
            emission: None,
            check_portals: false,
        },
        b"\x0a\x02after\0",
    );
    publish(
        &mut runtime,
        false,
        NativeTarget::Client(healthy),
        b"\x0fhealthy\0",
    );
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::fixed(25).unwrap(),
        vec![],
    )
    .unwrap();
    let mut peer =
        Channel::load(Protocol::Quake3_68.channel(), Endpoint::Client, 8192, 16).unwrap();
    peer.set_qport(2000);
    let mut healthy_peer =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
    let mut source = Source::default();
    let first = host.frame(&mut source, true);
    assert_eq!(first.output.native_unsupported, 0);
    assert_eq!(
        host.runtime
            .server
            .events
            .counters(owner)
            .unwrap()
            .submissions,
        0
    );
    assert!(host.runtime.server.events.retained_for(owner, range.start));
    let mut commands = Vec::new();
    for (port, bytes) in &source.packets {
        if *port == 2000 {
            let received = peer.receive(bytes, EventTime(1)).unwrap();
            let sequence = received.header.sequence;
            if let Delivery::Payload(body) = received.delivery {
                let mut body = body.to_vec();
                peer.decode_server_output(&mut body, sequence, |message| {
                    if let qa_network::channel::commands::ServerOutput::ReliableCommand {
                        sequence,
                        text,
                    } = message
                    {
                        commands.push((sequence, text.to_vec()));
                    }
                })
                .unwrap();
            }
        } else {
            assert_eq!(*port, 2001);
            assert_eq!(receive(&mut healthy_peer, bytes), b"\x0fhealthy\0");
        }
    }
    assert_eq!(commands.len(), 64);
    assert!(
        commands
            .iter()
            .all(|(_, text)| text == b"print \"occupied\"")
    );
    let state = host
        .runtime
        .network
        .get(target, Endpoint::Server)
        .unwrap()
        .channel
        .command_state()
        .unwrap();
    assert_eq!(
        (state.queued, state.submitted, state.acknowledged),
        (64, 64, 0)
    );
    source.now = EventTime::from_milliseconds(25);
    queue_move(&mut host, &mut peer, Protocol::Quake3_68, source.now);
    source.packets.clear();
    let resumed = host.frame(&mut source, true);
    assert_eq!(resumed.server_ticks, 1);
    assert_eq!(resumed.output.native_unsupported, 0);
    assert_eq!(
        host.runtime
            .server
            .events
            .counters(owner)
            .unwrap()
            .submissions,
        1
    );
    assert!(host.runtime.server.events.retained_for(owner, range.start));
    commands.clear();
    for (port, bytes) in &source.packets {
        if *port == 2000 {
            let received = peer.receive(bytes, source.now).unwrap();
            let sequence = received.header.sequence;
            if let Delivery::Payload(body) = received.delivery {
                let mut body = body.to_vec();
                peer.decode_server_output(&mut body, sequence, |message| {
                    if let qa_network::channel::commands::ServerOutput::ReliableCommand {
                        sequence,
                        text,
                    } = message
                    {
                        commands.push((sequence, text.to_vec()));
                    }
                })
                .unwrap();
            }
        }
    }
    assert_eq!(commands, [(65, b"print \"after\"".to_vec())]);
    let state = host
        .runtime
        .network
        .get(target, Endpoint::Server)
        .unwrap()
        .channel
        .command_state()
        .unwrap();
    assert_eq!(
        (state.queued, state.submitted, state.acknowledged),
        (65, 65, 64)
    );
    source.now = EventTime::from_milliseconds(50);
    queue_move(&mut host, &mut peer, Protocol::Quake3_68, source.now);
    source.packets.clear();
    assert_eq!(host.frame(&mut source, true).server_ticks, 1);
    assert!(!host.runtime.server.events.retained_for(owner, range.start));
    assert_eq!(
        host.runtime
            .network
            .get(target, Endpoint::Server)
            .unwrap()
            .channel
            .command_state()
            .unwrap()
            .acknowledged,
        65
    );
}
