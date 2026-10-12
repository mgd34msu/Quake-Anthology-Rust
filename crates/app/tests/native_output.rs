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
        self.packets.push((to.port(), bytes.to_vec()));
        true
    }
}
fn remote(runtime: &mut Runtime, protocol: Protocol, port: u16) -> ClientId {
    let id = runtime
        .server
        .connect(Connection::Remote, ModuleId(0), PlayerTail::None, None)
        .unwrap();
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
                commands: Some(Commands::load(protocol)),
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
                check_portals: true,
            },
            bytes,
        )
        .unwrap()
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
    let source_bytes = [
        10, 2, b'p', 0x80, 0xff, 0, 9, 31, 49, 204, 128, 125, 41, 0, 247, 255, 162, 0, 1, 4, 1, 5,
        0, 128, 4, b'l', 0xff, 0,
    ];
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
            assert_eq!(print.text, b"p\x80\xff");
            remaining = &remaining[n..];
            let (cue, n) =
                qa_network::outputs::read_native_service(Protocol::Quake2Repro1038, remaining)
                    .unwrap();
            let qa_network::outputs::NativeService::Cue(qa_network::outputs::Cue::Sound(sound)) =
                cue
            else {
                panic!("sound");
            };
            assert_eq!(sound.position, Vec3([-1.125, 20.25, 128.125]));
            assert_eq!((sound.entity, sound.channel, sound.index), (5, 1, 49));
            remaining = &remaining[n..];
            let (_, n) =
                qa_network::outputs::read_native_service(Protocol::Quake2Repro1038, remaining)
                    .unwrap();
            remaining = &remaining[n..];
            let (layout, n) =
                qa_network::outputs::read_native_service(Protocol::Quake2Repro1038, remaining)
                    .unwrap();
            assert!(
                matches!(layout, qa_network::outputs::NativeService::Print(print)
                if print.kind == PrintKind::Layout && print.text == b"l\xff")
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
        9, 32, 1, 4, 32, 7, 0, 2, 3, 10, 5, b'c', 0x80, 0, 4, b'l', 0xff, 0,
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
        b"\x0fc\x80\0\x04l\xff\0"
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
    let cue = [9, 4, 49, 247, 255, 162, 0, 1, 4];
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
#[test]
fn reliable_native_pages_retire_only_on_the_selected_peers_channel_ack() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let first = remote(&mut runtime, Protocol::Quake2_34, 2000);
    let _foreign = remote(&mut runtime, Protocol::Quake3_68, 2001);
    let mut source = Source::default();
    source.now = EventTime(10);
    publish(&mut runtime, true, NativeTarget::All, b"\x0fraw\x80\xff\0");
    let counts = dispatch(&mut runtime, &mut source);
    assert_eq!(counts.native_unsupported, 1);
    assert_eq!(runtime.server.events.len(), 1);
    assert_eq!(source.packets.len(), 1);
    let mut peer =
        Channel::load(Protocol::Quake2_34.channel(), Endpoint::Client, 8192, 16).unwrap();
    peer.set_qport(2000);
    assert_eq!(
        receive(&mut peer, &source.packets[0].1),
        b"\x0fraw\x80\xff\0"
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
fn local_snapshots_leave_a_rejected_native_datagram_for_its_ring_consumer() {
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
    assert!(!runtime.send_local_snapshot(id, EventTime(100_000_000), None));
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
