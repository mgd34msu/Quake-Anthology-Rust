use super::*;
use qa_core::{
    primitives::{ResourceNamespace, ThinkTime},
    text::info_value,
};
use qa_network::{
    commands::startup::write_q2_connect,
    message::Writer,
    snapshots::{Frame, ReceivedFrame},
    states,
};

#[path = "../support/q2_startup.rs"]
mod q2_files;
#[path = "../support/sound_registration.rs"]
#[allow(dead_code)]
mod sound_support;
const CLASSIC: Protocol = Protocol::Quake2_34;
const RR: Protocol = Protocol::Quake2Repro1038;

fn context(protocol: Protocol, seat: SeatId) -> Context {
    Context {
        source: protocol.rule_set(),
        seat,
        ..Default::default()
    }
}
fn setting(host: &mut FrameHost, protocol: Protocol, name: &str, value: &str) {
    let view = host
        .console
        .cvars
        .bind(name, context(protocol, SeatId::FIRST))
        .unwrap();
    host.console.cvars.force_write(view, value).unwrap();
}
fn connect(
    host: &mut FrameHost,
    source: &mut Source,
    protocol: Protocol,
    address: SocketAddr,
    policy: Option<ClientPolicy>,
    time: u64,
) -> ClientId {
    host.console
        .append(
            &format!("connect {address}\n"),
            Context {
                client_policy: policy,
                ..context(protocol, SeatId::FIRST)
            },
        )
        .unwrap();
    frame(host, source, time);
    host.local_clients[0].unwrap()
}
fn userinfo(host: &FrameHost, protocol: Protocol) -> Vec<u8> {
    let rules = protocol.userinfo_rules().unwrap();
    let mut bytes = vec![0; rules.bytes];
    let length = host
        .console
        .cvars
        .info_string(context(protocol, SeatId::FIRST), 2, rules, &mut bytes)
        .unwrap();
    bytes.truncate(length);
    bytes
}
fn field<'a>(info: &'a [u8], key: &[u8]) -> &'a [u8] {
    &info[info_value(info, key, false).unwrap()]
}
fn expected_connect(
    protocol: Protocol,
    qport: u16,
    challenge: i32,
    info: &[u8],
    packet_length: i32,
) -> Vec<u8> {
    let mut bytes = [0; 1024];
    let length =
        write_q2_connect(protocol, qport, challenge, info, packet_length, &mut bytes).unwrap();
    bytes[..length].to_vec()
}
fn collect(server: &mut EventPump, native: &mut NativePeer, time: u64) {
    for (_, bytes) in datagrams(server) {
        native.receive(&bytes, time);
    }
}
fn reconnect_requests(
    server: &mut EventPump,
    native: &mut NativePeer,
    local: SocketAddr,
    time: u64,
) -> Vec<(Peer, Vec<u8>)> {
    let before = native.count(b"disconnect");
    let protocol = native.commands.protocol;
    let qport = native.channel.qport();
    let mut requests = vec![];
    let mut disconnect_packets = vec![];
    for packet in datagrams(server) {
        if packet.1.starts_with(&[255; 4]) {
            requests.push(packet);
        } else {
            assert_eq!(peer(&packet), local);
            // Decode each physical packet independently, then feed the live
            // peer which suppresses repeated reliable commands.
            let mut physical = NativePeer::new(protocol, qport);
            physical.receive(&packet.1, time);
            if physical.count(b"disconnect") != 0 {
                assert_eq!(physical.count(b"disconnect"), 1);
                let format = if protocol == RR {
                    headers::q2_new(qport != 0)
                } else {
                    headers::QUAKE2
                };
                let (header, payload) =
                    headers::decode(format, Direction::ToServer, &packet.1).unwrap();
                let expected = if protocol == RR {
                    b"\x04disconnect\0".as_slice()
                } else {
                    b"\x04disconnect".as_slice()
                };
                assert_eq!(payload, expected, "native disconnect bytes and length");
                disconnect_packets.push((header.sequence, packet.1.clone()));
            }
            native.receive(&packet.1, time);
        }
    }
    assert_eq!(disconnect_packets.len(), 3);
    let logical = if protocol == RR {
        assert_eq!(disconnect_packets[0], disconnect_packets[1]);
        assert_eq!(disconnect_packets[0], disconnect_packets[2]);
        1
    } else {
        assert_eq!(disconnect_packets[1].0, disconnect_packets[0].0 + 1);
        assert_eq!(disconnect_packets[2].0, disconnect_packets[1].0 + 1);
        3
    };
    assert_eq!(native.count(b"disconnect"), before + logical);
    requests
}
fn text(writer: &mut Writer<'_>, protocol: Protocol, bytes: &[u8]) {
    writer
        .write_string(protocol.string_policy(), bytes)
        .unwrap();
}
fn stuff(protocol: Protocol, bytes: &[u8]) -> Vec<u8> {
    wire(|writer| {
        writer.write_bits(11, 8).unwrap();
        text(writer, protocol, bytes);
    })
}
fn model_owner(
    host: &mut FrameHost,
    source: &mut Source,
    protocol: Protocol,
) -> (support::Fixture, qa_app::model_assets::ModelProfile) {
    let mut fixture = support::Fixture::new(&[
        ("maps/ready.bsp", q2_files::world(b"wall")),
        ("textures/wall.wal", q2_files::wal(11)),
        ("models/test/tris.md2", support::md2()),
        ("models/test/skin0.pcx", support::pcx(11)),
        ("models/test/skin1.pcx", support::pcx(22)),
        ("sound/step.wav", sound_support::wav()),
    ]);
    host.runtime.vfs = std::mem::take(&mut fixture.vfs);
    let mut assets = qa_render::Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile =
        qa_app::model_assets::ModelRequest::native("", protocol.rule_set(), Some(palette))
            .profile();
    let (scene, _) =
        qa_app::renderer::SceneComposer::load(Default::default(), &mut assets, vec![]).unwrap();
    let mut owner =
        qa_app::model_assets::ModelAssets::new(assets, [profile], [], Default::default()).unwrap();
    owner
        .adopt_palettes(
            qa_app::render_settings::PaletteRegistrations::from_registered([
                None,
                None,
                Some(palette),
                Some(palette),
                None,
            ]),
        )
        .unwrap();
    let backend = asset_frames::Source::load(owner.assets());
    host.runtime.model_assets = Some(owner);
    host.runtime.scene = Some(scene);
    host.runtime.sound_assets = Some(Box::new(
        qa_app::audio::SoundAssets::load(32, 4096).unwrap(),
    ));
    host.publish_model_admission(backend.cpu.admitted_assets().unwrap())
        .unwrap();
    host.runtime
        .model_assets
        .as_mut()
        .unwrap()
        .end_initial_registration();
    source.backend = Some(backend);
    (fixture, profile)
}

fn server_data(protocol: Protocol, count: i32) -> Vec<u8> {
    let mut header = wire(|writer| {
        writer.write_bits(12, 8).unwrap();
        writer.write_bits(protocol.number(), 32).unwrap();
        writer.write_bits(count as u32, 32).unwrap();
        writer.write_bits(0, 8).unwrap();
        text(writer, protocol, b"baseq2");
        writer.write_bits(3, 16).unwrap();
        text(writer, protocol, b"Owned UDP Q2 startup");
        if protocol == RR {
            writer.write_bits(1024, 16).unwrap();
            writer.write_bits(2, 8).unwrap();
            writer.write_bits(0, 16).unwrap();
            writer.write_bits(40, 8).unwrap();
        }
    });
    if protocol == CLASSIC {
        header.extend(stuff(
            protocol,
            format!("cmd configstrings {count} 0\n").as_bytes(),
        ));
    }
    header
}

fn become_active(
    host: &mut FrameHost,
    source: &mut Source,
    server: &mut EventPump,
    socket: u16,
    local: SocketAddr,
    native: &mut NativePeer,
    client: ClientId,
    count: i32,
    first_ms: u64,
) {
    let protocol = native.commands.protocol;
    let before = history(host, client);
    let moves = native.moves;
    deliver(
        host,
        source,
        server,
        socket,
        local,
        native,
        &server_data(protocol, count),
        first_ms,
    );
    assert_eq!(
        host.runtime.snapshots[client.0 as usize]
            .as_ref()
            .unwrap()
            .native_client(),
        Some(3)
    );
    assert_eq!(history(host, client), before);
    assert_eq!(native.moves, moves);
    let layout = host.runtime.client_resources[client.0 as usize]
        .as_ref()
        .unwrap()
        .state
        .layout;
    let checksum = format!(
        "{}",
        qa_core::checksum::block_checksum(&q2_files::world(b"wall")) as i32
    );
    let rows = [
        (
            layout.resources[0].get(1).unwrap(),
            b"maps/ready.bsp".as_slice(),
        ),
        (layout.resources[0].get(2).unwrap(), b"models/test/tris.md2"),
        (layout.resources[1].get(1).unwrap(), b"step.wav"),
        (layout.map_checksum().unwrap(), checksum.as_bytes()),
    ];
    let mut bytes = vec![];
    if protocol == CLASSIC {
        assert_eq!(
            native.count(format!("configstrings {count} 0").as_bytes()),
            1
        );
        for (index, value) in rows {
            bytes.extend(asset_frames::config(protocol, index, value));
        }
        bytes.extend(stuff(
            protocol,
            format!("cmd baselines {count} 0\n").as_bytes(),
        ));
    } else {
        bytes = wire(|writer| {
            writer.write_bits(38, 8).unwrap();
            for (index, value) in rows {
                writer.write_bits(index as u32, 16).unwrap();
                text(writer, protocol, value);
            }
            writer.write_bits(layout.count as u32, 16).unwrap();
            writer.write_bits(39, 8).unwrap();
            writer.write_bits(0, 16).unwrap();
        });
    }
    let attempts = source.backend.as_ref().unwrap().attempts;
    deliver(
        host,
        source,
        server,
        socket,
        local,
        native,
        &bytes,
        first_ms + 10,
    );
    assert_eq!(source.backend.as_ref().unwrap().attempts, attempts);
    assert_eq!(history(host, client), before);
    assert_eq!(native.count(format!("begin {count}").as_bytes()), 0);
    if protocol == CLASSIC {
        assert_eq!(native.count(format!("baselines {count} 0").as_bytes()), 1);
    }
    deliver(
        host,
        source,
        server,
        socket,
        local,
        native,
        &stuff(protocol, format!("precache {count}\n").as_bytes()),
        first_ms + 20,
    );
    assert!(source.backend.as_ref().unwrap().attempts > attempts);
    assert!(host.local_worlds[0].is_some());
    assert_eq!(native.count(format!("begin {count}").as_bytes()), 1);
    assert_eq!(history(host, client), before);
    assert_eq!(native.moves, moves);
    let owner = host.runtime.model_assets.as_ref().unwrap();
    assert_eq!(
        source.backend.as_ref().unwrap().cpu.admitted_assets(),
        Some(owner.assets().counts())
    );
    let resources = &host.runtime.client_resources[client.0 as usize]
        .as_ref()
        .unwrap()
        .state;
    assert!(resources.models.collision_world().is_some());
    assert!(
        resources
            .models
            .resolve(
                qa_core::primitives::ModelRef::native(ResourceNamespace::Client(client), 2)
                    .unwrap(),
                0,
                owner.catalog(),
                Default::default()
            )
            .is_some()
    );
    let bank = host.runtime.sound_bank().unwrap();
    assert!(bank.sample(bank.find("sound/step.wav").unwrap()).is_some());
    let classic_player = [0; states::Q2_PLAYER_WORDS];
    let rr_player = [0; states::Q2_REPRO_PLAYER_WORDS];
    let frame_number = native.channel.send_state().sequence;
    let snapshot = if protocol == CLASSIC {
        ReceivedFrame::Quake2(Frame {
            sequence: frame_number,
            time: ThinkTime::Milliseconds((first_ms + 30) as i64),
            command: 0,
            flags: 0,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[],
            player: &classic_player,
            entities: &[],
        })
    } else {
        ReceivedFrame::Quake2Repro(Frame {
            sequence: frame_number,
            time: ThinkTime::Milliseconds((first_ms + 30) as i64),
            command: 0,
            flags: 0,
            qw_peers: 0,
            qw_dead: 0,
            areas: &[],
            player: &rr_player,
            entities: &[],
        })
    };
    let bytes = wire(|writer| {
        native
            .channel
            .write_snapshot_candidate(writer, snapshot, None, 256)
            .unwrap()
    });
    native
        .channel
        .publish_written_snapshot(snapshot, None)
        .unwrap();
    deliver(
        host,
        source,
        server,
        socket,
        local,
        native,
        &bytes,
        first_ms + 30,
    );
    assert!(history(host, client) > before);
    assert!(native.moves > moves);
}

#[test]
fn q2_and_rerelease_owned_udp_admit_real_media_and_reconnect_from_active_via_svc8() {
    for (protocol, configured, qport) in [(CLASSIC, "65535", 65535), (RR, "256", 0)] {
        let mut server = EventPump::new();
        let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
        let mut source = source();
        let mut host = host(2);
        let (_fixture, profile) = model_owner(&mut host, &mut source, protocol);
        setting(&mut host, protocol, "net_qport", configured);
        setting(&mut host, protocol, "net_maxmsglen", "777");
        setting(&mut host, protocol, "name", "Native Player");
        let ctx = context(protocol, SeatId::FIRST);
        host.console
            .cvars
            .register("_qa_oversized", Some(&"x".repeat(600)), 2, ctx)
            .unwrap();
        host.console
            .cvars
            .register("_qa_long_field", Some(&"l".repeat(80)), 2, ctx)
            .unwrap();
        for index in 0..8 {
            host.console
                .cvars
                .register(&format!("_qa_extra{index}"), Some(&"v".repeat(60)), 2, ctx)
                .unwrap();
        }
        let policy = ClientPolicy {
            client: RuleSetId::Quake3,
            input: RuleSetId::QuakeWorld,
            movement: RuleSetId::Quake3,
            trace: RuleSetId::Quake,
        };
        let client = connect(&mut host, &mut source, protocol, address, Some(policy), 0);
        host.bind_client_model_assets([(client, profile, None)]);
        let packets = datagrams(&mut server);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].1, b"\xff\xff\xff\xffgetchallenge\n");
        let local = peer(&packets[0]);
        response(
            &mut server,
            socket,
            local,
            format!("challenge -88 p={}\n", protocol.number()).as_bytes(),
        );
        frame(&mut host, &mut source, 10);
        let packets = datagrams(&mut server);
        let info = userinfo(&host, protocol);
        assert!(info.len() < 512);
        assert_eq!(field(&info, b"name"), b"Native Player");
        assert!(info_value(&info, b"_qa_oversized", false).is_none());
        assert!(
            info_value(&info, b"_qa_long_field", false).is_none(),
            "wire fields are shorter than 64 bytes even when the module ABI permits 256"
        );
        assert!(
            (0..8).any(
                |index| info_value(&info, format!("_qa_extra{index}").as_bytes(), false).is_none()
            ),
            "native512 budget must omit fields which cannot fit"
        );
        assert_eq!(packets.len(), 1);
        assert_eq!(
            packets[0].1,
            expected_connect(protocol, qport, -88, &info, 777)
        );
        assert_eq!(
            host.runtime
                .network
                .get(client, Endpoint::Client)
                .unwrap()
                .channel
                .qport(),
            qport
        );
        response(
            &mut server,
            socket,
            local,
            format!("client_connect nc={} map=ready\n", u8::from(protocol == RR)).as_bytes(),
        );
        frame(&mut host, &mut source, 20);
        let mut native = NativePeer::new(protocol, qport);
        collect(&mut server, &mut native, 20);
        assert_eq!(native.count(b"new"), 1);
        assert_eq!(native.moves, 0);
        assert!(
            native.userinfos.is_empty(),
            "accepted connect is the connected userinfo baseline"
        );
        assert_eq!(history(&host, client), 0);
        assert_eq!(
            host.runtime.snapshots[client.0 as usize]
                .as_ref()
                .unwrap()
                .native_client(),
            None
        );
        become_active(
            &mut host,
            &mut source,
            &mut server,
            socket,
            local,
            &mut native,
            client,
            -501,
            30,
        );
        setting(&mut host, protocol, "name", "After Active");
        frame(&mut host, &mut source, 70);
        collect(&mut server, &mut native, 70);
        assert_eq!(native.userinfos, [userinfo(&host, protocol)]);
        assert_eq!(field(&native.userinfos[0], b"name"), b"After Active");
        native.reply(&mut server, socket, local, 71);
        frame(&mut host, &mut source, 71);
        collect(&mut server, &mut native, 71);
        host.runtime
            .network
            .get_mut(client, Endpoint::Client)
            .unwrap()
            .channel
            .queue_client_command(protocol, b"older reliable")
            .unwrap();
        assert!(host.runtime.send_client_command(
            client,
            &qa_core::primitives::UserCmd::default(),
            EventTime::from_milliseconds(75),
            &mut source,
        ));
        let channel = &mut host
            .runtime
            .network
            .get_mut(client, Endpoint::Client)
            .unwrap()
            .channel;
        assert!(channel.send_state().reliable_bytes > 0);
        channel
            .queue_client_command(protocol, b"queued behind older flight")
            .unwrap();
        assert_eq!(native.count(b"older reliable"), 0);
        let connection = host.runtime.network.get(client, Endpoint::Client).unwrap();
        let old_token = connection.route.socket;
        let old_entity = host.runtime.server.clients[client.0 as usize].entity;
        native.channel.queue_reliable(&[8]).unwrap();
        native.reply(&mut server, socket, local, 80);
        frame(&mut host, &mut source, 80);
        let requests = reconnect_requests(&mut server, &mut native, local, 80);
        assert_eq!(native.count(b"older reliable"), 1);
        assert_eq!(native.count(b"queued behind older flight"), 0);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, b"\xff\xff\xff\xffgetchallenge\n");
        let replacement = host.local_clients[0].unwrap();
        let connection = host
            .runtime
            .network
            .get(replacement, Endpoint::Client)
            .unwrap();
        assert_ne!(connection.route.socket, old_token);
        assert!(source.pump.send_udp(old_token, address, b"closed").is_err());
        assert_ne!(
            host.runtime.server.clients[replacement.0 as usize].entity,
            old_entity
        );
        assert_eq!(connection.commands.as_ref().unwrap().protocol, protocol);
        assert_eq!(
            host.runtime.server.clients[replacement.0 as usize].client_rules,
            policy.client
        );
        assert_eq!(
            host.runtime.server.clients[replacement.0 as usize].input_rules,
            policy.input
        );
        assert_eq!(
            host.runtime.prediction[0].player.movement_rules,
            policy.movement
        );
        assert_eq!(host.runtime.prediction[0].player.trace_rules, policy.trace);
        assert!(host.local_worlds[0].is_none());
        assert_eq!(history(&host, replacement), 0);
        let local = peer(&requests[0]);
        response(
            &mut server,
            socket,
            local,
            format!("challenge 89 p={}\n", protocol.number()).as_bytes(),
        );
        frame(&mut host, &mut source, 90);
        let packets = datagrams(&mut server);
        assert_eq!(packets.len(), 1);
        assert_eq!(
            packets[0].1,
            expected_connect(protocol, qport, 89, &userinfo(&host, protocol), 777)
        );
        response(
            &mut server,
            socket,
            local,
            format!("client_connect nc={}\n", u8::from(protocol == RR)).as_bytes(),
        );
        frame(&mut host, &mut source, 100);
        let mut replacement_native = NativePeer::new(protocol, qport);
        collect(&mut server, &mut replacement_native, 100);
        assert_eq!(replacement_native.count(b"new"), 1);
        assert_eq!(replacement_native.moves, 0);
        assert_eq!(history(&host, replacement), 0);
    }
}

#[test]
fn q2_udp_peer_checks_transport_retry_and_last_admitted_userinfo_are_protocol_selected() {
    for protocol in [CLASSIC, RR] {
        let mut server = EventPump::new();
        let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
        let mut foreign = EventPump::new();
        let (foreign_socket, _) = foreign.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
        let mut source = source();
        let mut host = host(1);
        setting(&mut host, protocol, "net_qport", "0");
        let client = connect(&mut host, &mut source, protocol, address, None, 0);
        let packets = datagrams(&mut server);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].1, b"\xff\xff\xff\xffgetchallenge\n");
        let local = peer(&packets[0]);
        let qport = if protocol == RR {
            u16::from(local.port() as u8)
        } else {
            local.port()
        };
        assert_eq!(
            host.runtime
                .network
                .get(client, Endpoint::Client)
                .unwrap()
                .channel
                .qport(),
            qport
        );
        let unrouted = host.runtime.network.unrouted;
        response(
            &mut foreign,
            foreign_socket,
            local,
            format!("challenge 700 p={}\n", protocol.number()).as_bytes(),
        );
        response(&mut foreign, foreign_socket, local, b"client_connect\n");
        frame(&mut host, &mut source, 1);
        assert!(host.runtime.network.unrouted >= unrouted + 2);
        assert!(datagrams(&mut server).is_empty());
        assert!(matches!(
            host.runtime
                .network
                .get(client, Endpoint::Client)
                .unwrap()
                .phase,
            ConnectionPhase::Connecting(_)
        ));
        source.reject_connect = true;
        setting(&mut host, protocol, "name", "Failed transport");
        response(
            &mut server,
            socket,
            local,
            format!("challenge 700 p={}\n", protocol.number()).as_bytes(),
        );
        frame(&mut host, &mut source, 10);
        assert_eq!(source.rejected_connects, 1);
        assert!(datagrams(&mut server).is_empty());
        setting(&mut host, protocol, "name", "Actually sent");
        source.reject_connect = false;
        frame(&mut host, &mut source, 11);
        let packets = datagrams(&mut server);
        assert_eq!(packets.len(), 1);
        assert_eq!(
            packets[0].1,
            expected_connect(protocol, qport, 700, &userinfo(&host, protocol), 1390)
        );
        frame(&mut host, &mut source, 3010);
        assert!(datagrams(&mut server).is_empty());
        setting(&mut host, protocol, "name", "Retried transport");
        frame(&mut host, &mut source, 3011);
        let packets = datagrams(&mut server);
        assert_eq!(packets.len(), 1);
        if protocol == CLASSIC {
            assert_eq!(packets[0].1, b"\xff\xff\xff\xffgetchallenge\n");
            response(&mut server, socket, local, b"challenge 701 p=34\n");
            frame(&mut host, &mut source, 3012);
            let packets = datagrams(&mut server);
            assert_eq!(packets.len(), 1);
            assert_eq!(
                packets[0].1,
                expected_connect(protocol, qport, 701, &userinfo(&host, protocol), 1390)
            );
        } else {
            assert_eq!(
                packets[0].1,
                expected_connect(protocol, qport, 700, &userinfo(&host, protocol), 1390)
            );
        }
        response(
            &mut server,
            socket,
            local,
            format!("client_connect nc={}\n", u8::from(protocol != RR)).as_bytes(),
        );
        frame(&mut host, &mut source, 3013);
        assert!(matches!(
            host.runtime
                .network
                .get(client, Endpoint::Client)
                .unwrap()
                .phase,
            ConnectionPhase::Connecting(_)
        ));
        assert!(datagrams(&mut server).is_empty());
        setting(&mut host, protocol, "name", "Changed after send");
        response(
            &mut server,
            socket,
            local,
            format!("client_connect nc={}\n", u8::from(protocol == RR)).as_bytes(),
        );
        frame(&mut host, &mut source, 3020);
        let mut native = NativePeer::new(protocol, qport);
        collect(&mut server, &mut native, 3020);
        assert_eq!(native.count(b"new"), 1);
        assert_eq!(native.moves, 0);
        assert_eq!(history(&host, client), 0);
        assert_eq!(
            native.userinfos,
            [userinfo(&host, protocol)],
            "accepted receipt must compare against the last admitted connect, then publish the new value"
        );
        assert_eq!(field(&native.userinfos[0], b"name"), b"Changed after send");
        native.reply(&mut server, socket, local, 3030);
        frame(&mut host, &mut source, 3030);
        collect(&mut server, &mut native, 3030);
        assert_eq!(native.count(b"new"), 1);
        assert_eq!(native.userinfos.len(), 1);
    }
}

#[test]
fn rerelease_download_reconnect_checks_origin_and_loading_state_before_its_veto() {
    for control in ["manual", "active", "loading"] {
        let mut server = EventPump::new();
        let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
        let mut source = source();
        let mut host = host(2);
        let (_fixture, profile) = model_owner(&mut host, &mut source, RR);
        setting(&mut host, RR, "net_qport", "256");
        setting(&mut host, RR, "cl_allowDownload", "1");
        setting(&mut host, RR, "allow_download_sounds", "1");
        let client = connect(&mut host, &mut source, RR, address, None, 0);
        host.bind_client_model_assets([(client, profile, None)]);
        let packets = datagrams(&mut server);
        assert_eq!(packets.len(), 1);
        let local = peer(&packets[0]);
        response(&mut server, socket, local, b"challenge 44 p=1038\n");
        frame(&mut host, &mut source, 10);
        let packets = datagrams(&mut server);
        assert_eq!(packets.len(), 1);
        assert_eq!(
            packets[0].1,
            expected_connect(RR, 0, 44, &userinfo(&host, RR), 1390)
        );
        response(&mut server, socket, local, b"client_connect nc=1\n");
        frame(&mut host, &mut source, 20);
        let mut native = NativePeer::new(RR, 0);
        collect(&mut server, &mut native, 20);
        assert_eq!(native.count(b"new"), 1);
        if control == "loading" {
            deliver(
                &mut host,
                &mut source,
                &mut server,
                socket,
                local,
                &mut native,
                &server_data(RR, 812),
                30,
            );
            assert!(
                host.runtime.client_resources[client.0 as usize]
                    .as_ref()
                    .unwrap()
                    .received_q2_server_data()
                    .is_some()
            );
        } else {
            become_active(
                &mut host,
                &mut source,
                &mut server,
                socket,
                local,
                &mut native,
                client,
                812,
                30,
            );
        }
        let mount = host.runtime.vfs.mounts().next().unwrap().id;
        let root = host.runtime.vfs.directory_path(mount).unwrap().to_owned();
        host.configure_client_downloads(client, mount, mount, 8192, 128)
            .unwrap();
        host.console
            .append("download sound/held.wav\n", context(RR, SeatId::FIRST))
            .unwrap();
        frame(&mut host, &mut source, 70);
        collect(&mut server, &mut native, 70);
        assert_eq!(native.count(b"download sound/held.wav"), 1);
        let mut block = [0; 64];
        let length = qa_network::downloads::write_chunk(
            RR,
            qa_network::downloads::Chunk {
                compressed: false,
                percent: 10,
                bytes: Some(b"partial"),
            },
            &mut block,
        )
        .unwrap();
        deliver(
            &mut host,
            &mut source,
            &mut server,
            socket,
            local,
            &mut native,
            &block[..length],
            80,
        );
        assert_eq!(native.count(b"nextdl"), 1);
        assert!(qa_platform::files::read(root.join("sound/held.wav")).is_err());
        let old_token = host
            .runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .route
            .socket;
        let old_entity = host.runtime.server.clients[client.0 as usize].entity;
        if control == "loading" {
            let before = history(&host, client);
            deliver(
                &mut host,
                &mut source,
                &mut server,
                socket,
                local,
                &mut native,
                &stuff(RR, b"reconnect\nchanging\n"),
                90,
            );
            assert_eq!(native.count(b"new"), 1, "loading reconnect is vetoed");
            assert_eq!(native.count(b"disconnect"), 0);
            assert_eq!(history(&host, client), before);
            assert!(
                host.runtime.client_resources[client.0 as usize]
                    .as_ref()
                    .unwrap()
                    .received_q2_server_data()
                    .is_none(),
                "changing is accepted while the partial download remains open"
            );
            let length = qa_network::downloads::write_chunk(
                RR,
                qa_network::downloads::Chunk {
                    compressed: false,
                    percent: 100,
                    bytes: Some(b"done"),
                },
                &mut block,
            )
            .unwrap();
            deliver(
                &mut host,
                &mut source,
                &mut server,
                socket,
                local,
                &mut native,
                &block[..length],
                100,
            );
            assert_eq!(
                qa_platform::files::read(root.join("sound/held.wav")).unwrap(),
                b"partialdone"
            );
            deliver(
                &mut host,
                &mut source,
                &mut server,
                socket,
                local,
                &mut native,
                &stuff(RR, b"reconnect\n"),
                110,
            );
            assert_eq!(native.count(b"new"), 2);
            assert_eq!(native.count(b"disconnect"), 0);
            assert_eq!(
                host.runtime
                    .network
                    .get(client, Endpoint::Client)
                    .unwrap()
                    .route
                    .socket,
                old_token
            );
            assert_eq!(
                host.runtime.server.clients[client.0 as usize].entity,
                old_entity
            );
            assert_eq!(history(&host, client), before);
        } else {
            if control == "manual" {
                host.console
                    .append("reconnect\n", context(RR, SeatId::FIRST))
                    .unwrap();
                frame(&mut host, &mut source, 90);
            } else {
                native
                    .channel
                    .queue_reliable(&stuff(RR, b"reconnect\n"))
                    .unwrap();
                native.reply(&mut server, socket, local, 90);
                frame(&mut host, &mut source, 90);
            }
            let requests = reconnect_requests(&mut server, &mut native, local, 90);
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].1, b"\xff\xff\xff\xffgetchallenge\n");
            let replacement = host.local_clients[0].unwrap();
            assert_ne!(
                host.runtime.server.clients[replacement.0 as usize].entity,
                old_entity
            );
            assert_ne!(
                host.runtime
                    .network
                    .get(replacement, Endpoint::Client)
                    .unwrap()
                    .route
                    .socket,
                old_token
            );
            assert!(qa_platform::files::read(root.join("sound/held.wav")).is_err());
            assert!(source.pump.send_udp(old_token, address, b"closed").is_err());
            assert_eq!(history(&host, replacement), 0);
        }
    }
}
