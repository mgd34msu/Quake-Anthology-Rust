use super::*;
use qa_core::{primitives::ClientId, text::info_value};
use qa_network::commands::startup::write_qw_connect;

const QW: Protocol = Protocol::QuakeWorld28;

fn qw_context(seat: SeatId) -> Context {
    Context {
        source: RuleSetId::QuakeWorld,
        seat,
        ..Default::default()
    }
}
fn setting(host: &mut FrameHost, seat: SeatId, name: &str, value: &str) {
    let view = host.console.cvars.bind(name, qw_context(seat)).unwrap();
    host.console.cvars.force_write(view, value).unwrap();
}
fn connect(
    host: &mut FrameHost,
    source: &mut Source,
    seat: SeatId,
    address: SocketAddr,
    milliseconds: u64,
) -> ClientId {
    host.console
        .append(&format!("connect {address}\n"), qw_context(seat))
        .unwrap();
    frame(host, source, milliseconds);
    host.local_clients[seat.index()].unwrap()
}
fn userinfo(host: &FrameHost, seat: SeatId, server: SocketAddr) -> Vec<u8> {
    let mut bytes = [0; qa_network::commands::startup::QW_CONNECT_USERINFO_BYTES];
    let mut length = host
        .console
        .cvars
        .info_string(
            qw_context(seat),
            2,
            QW.userinfo_rules().unwrap(),
            &mut bytes,
        )
        .unwrap();
    qa_core::text::info_set(
        &mut bytes,
        &mut length,
        b"*ip",
        server.to_string().as_bytes(),
        QW.userinfo_rules().unwrap(),
    )
    .unwrap();
    bytes[..length].to_vec()
}
fn field<'a>(info: &'a [u8], key: &[u8]) -> &'a [u8] {
    &info[info_value(info, key, false).unwrap()]
}
fn expected_connect(qport: u16, challenge: i32, info: &[u8]) -> Vec<u8> {
    let mut bytes = [0; 1024];
    let length = write_qw_connect(qport, challenge, info, &mut bytes).unwrap();
    bytes[..length].to_vec()
}
fn text(writer: &mut qa_network::message::Writer<'_>, bytes: &[u8]) {
    writer.write_string(QW.string_policy(), bytes).unwrap();
}
fn stuff(bytes: &[u8]) -> Vec<u8> {
    wire(|writer| {
        writer.write_bits(9, 8).unwrap();
        text(writer, bytes);
    })
}
fn model_owner(
    host: &mut FrameHost,
    source: &mut Source,
) -> (support::Fixture, qa_app::model_assets::ModelProfile) {
    let mut fixture = support::Fixture::new(&[("maps/ready.bsp", support::world_map(16.0))]);
    host.runtime.vfs = std::mem::take(&mut fixture.vfs);
    let mut assets = qa_render::Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let profile =
        qa_app::model_assets::ModelRequest::native("", RuleSetId::QuakeWorld, Some(palette))
            .profile();
    let (scene, _) =
        qa_app::renderer::SceneComposer::load(Default::default(), &mut assets, vec![]).unwrap();
    let mut owner = qa_app::model_assets::ModelAssets::new(
        &host.runtime.vfs,
        assets,
        [profile],
        [],
        Default::default(),
    )
    .unwrap();
    owner
        .adopt_palettes(
            qa_app::render_settings::PaletteRegistrations::from_registered(
                &host.runtime.vfs,
                [Some(palette), Some(palette), None, None, None],
            ),
        )
        .unwrap();
    let backend = asset_frames::Source::load(owner.assets());
    host.runtime.model_assets = Some(owner);
    host.runtime.scene = Some(scene);
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

fn become_active(
    host: &mut FrameHost,
    source: &mut Source,
    server: &mut EventPump,
    socket: u16,
    local: SocketAddr,
    native: &mut NativePeer,
    client: ClientId,
    server_count: i32,
    first_ms: u64,
) {
    let history_before = history(host, client);
    let moves_before = native.moves;
    let header = wire(|writer| {
        writer.write_bits(11, 8).unwrap();
        writer.write_bits(28, 32).unwrap();
        writer.write_bits(server_count as u32, 32).unwrap();
        text(writer, b"id1");
        writer.write_bits(0, 8).unwrap();
        text(writer, b"Active UDP fixture");
        for value in [800.0, 100.0, 320.0, 500.0, 10.0, 0.7, 10.0, 4.0, 4.0, 1.0] {
            writer.write_float(value).unwrap();
        }
    });
    deliver(
        host, source, server, socket, local, native, &header, first_ms,
    );
    assert_eq!(
        native.count(format!("soundlist {server_count} 0").as_bytes()),
        1
    );
    let sounds = wire(|writer| {
        writer.write_bits(46, 8).unwrap();
        writer.write_bits(0, 8).unwrap();
        text(writer, b"");
        writer.write_bits(0, 8).unwrap();
    });
    deliver(
        host,
        source,
        server,
        socket,
        local,
        native,
        &sounds,
        first_ms + 10,
    );
    assert_eq!(
        native.count(format!("modellist {server_count} 0").as_bytes()),
        1
    );
    let models = wire(|writer| {
        writer.write_bits(45, 8).unwrap();
        writer.write_bits(0, 8).unwrap();
        text(writer, b"maps/ready.bsp");
        text(writer, b"");
        writer.write_bits(0, 8).unwrap();
    });
    deliver(
        host,
        source,
        server,
        socket,
        local,
        native,
        &models,
        first_ms + 20,
    );
    assert!(host.local_worlds[0].is_some());
    let checksum = host
        .runtime
        .model_assets
        .as_ref()
        .unwrap()
        .catalog()
        .world_native_checksums(b"maps/ready.bsp")
        .and_then(|checksums| checksums.quakeworld_lumps)
        .unwrap();
    assert_eq!(
        native.count(format!("prespawn {server_count} 0 {}", checksum as i32).as_bytes()),
        1
    );
    deliver(
        host,
        source,
        server,
        socket,
        local,
        native,
        &stuff(format!("cmd spawn {server_count} 0\n").as_bytes()),
        first_ms + 30,
    );
    assert_eq!(
        native.count(format!("spawn {server_count} 0").as_bytes()),
        1
    );
    deliver(
        host,
        source,
        server,
        socket,
        local,
        native,
        &stuff(b"skins\n"),
        first_ms + 40,
    );
    assert_eq!(native.count(format!("begin {server_count}").as_bytes()), 1);
    assert_eq!(history(host, client), history_before);
    let snapshot = qa_network::snapshots::ReceivedFrame::QuakeWorld(qa_network::snapshots::Frame {
        sequence: native.channel.send_state().sequence,
        time: qa_core::primitives::ThinkTime::Milliseconds((first_ms + 50) as i64),
        command: 0,
        flags: 0,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[],
        player: &[],
        entities: &[],
    });
    let bytes = wire(|writer| {
        native
            .channel
            .write_snapshot_candidate(writer, snapshot, None, 32)
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
        first_ms + 50,
    );
    assert!(
        history(host, client) > history_before,
        "an actual first snapshot must release the startup input gate"
    );
    assert!(native.moves > moves_before);
}

#[test]
fn active_qw_reconnect_drops_the_old_channel_then_repeats_oob_admission_with_the_same_roles() {
    for server_control in [false, true] {
        let mut server = EventPump::new();
        let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
        let mut source = source();
        let mut host = host(2);
        let (_fixture, profile) = model_owner(&mut host, &mut source);
        setting(&mut host, SeatId::FIRST, "net_qport", "0");
        let policy = ClientPolicy {
            client: RuleSetId::Quake3,
            input: RuleSetId::Quake2Rerelease,
            movement: RuleSetId::Quake3,
            trace: RuleSetId::Quake2,
        };
        host.console
            .append(
                &format!("connect {address}\n"),
                Context {
                    client_policy: Some(policy),
                    ..qw_context(SeatId::FIRST)
                },
            )
            .unwrap();
        frame(&mut host, &mut source, 0);
        let client = host.local_clients[0].unwrap();
        host.bind_client_model_assets([(client, profile, None)]);
        let requests = datagrams(&mut server);
        assert_eq!(requests.len(), 1);
        let local = peer(&requests[0]);
        response(&mut server, socket, local, b"c88");
        frame(&mut host, &mut source, 10);
        let requests = datagrams(&mut server);
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].1,
            expected_connect(local.port(), 88, &userinfo(&host, SeatId::FIRST, address))
        );
        response(&mut server, socket, local, b"j");
        frame(&mut host, &mut source, 20);
        let mut native = NativePeer::new(QW, local.port());
        for (_, bytes) in datagrams(&mut server) {
            native.receive(&bytes, 20);
        }
        assert_eq!(native.count(b"new"), 1);
        become_active(
            &mut host,
            &mut source,
            &mut server,
            socket,
            local,
            &mut native,
            client,
            77,
            30,
        );
        let mut restart_at = 100;
        if !server_control {
            let mount = host.runtime.vfs.mounts().next().unwrap().id;
            host.configure_client_downloads(client, mount, mount, 8192, 128)
                .unwrap();
            host.console
                .append("download sound/held.wav\n", qw_context(SeatId::FIRST))
                .unwrap();
            frame(&mut host, &mut source, 90);
            for (_, bytes) in datagrams(&mut server) {
                native.receive(&bytes, 90);
            }
            assert_eq!(native.count(b"download sound/held.wav"), 1);
            let before = history(&host, client);
            deliver(
                &mut host,
                &mut source,
                &mut server,
                socket,
                local,
                &mut native,
                &stuff(b"changing\nreconnect\n"),
                100,
            );
            assert!(
                history(&host, client) > before,
                "native controls are ignored while a download is outstanding"
            );
            assert_eq!(native.count(b"new"), 1);
            assert_eq!(native.count(b"drop"), 0);
            let mut bytes = [0; 64];
            let length = qa_network::downloads::write_chunk(
                QW,
                qa_network::downloads::Chunk::Sequential {
                    compressed: false,
                    percent: 100,
                    bytes: Some(b"held bytes"),
                },
                &mut bytes,
            )
            .unwrap();
            deliver(
                &mut host,
                &mut source,
                &mut server,
                socket,
                local,
                &mut native,
                &bytes[..length],
                110,
            );
            assert!(host.runtime.vfs.open(b"sound/held.wav").is_some());
            let before = history(&host, client);
            let token = host
                .runtime
                .network
                .get(client, Endpoint::Client)
                .unwrap()
                .route
                .socket;
            deliver(
                &mut host,
                &mut source,
                &mut server,
                socket,
                local,
                &mut native,
                &stuff(b"changing\n"),
                120,
            );
            assert_eq!(
                history(&host, client),
                before,
                "changing must close the active input gate before reconnect"
            );
            assert_eq!(native.count(b"new"), 1);
            deliver(
                &mut host,
                &mut source,
                &mut server,
                socket,
                local,
                &mut native,
                &stuff(b"reconnect\n"),
                130,
            );
            assert_eq!(native.count(b"new"), 2);
            assert_eq!(native.count(b"drop"), 0);
            assert_eq!(history(&host, client), before);
            assert_eq!(
                host.runtime
                    .network
                    .get(client, Endpoint::Client)
                    .unwrap()
                    .route
                    .socket,
                token
            );
            become_active(
                &mut host,
                &mut source,
                &mut server,
                socket,
                local,
                &mut native,
                client,
                78,
                140,
            );
            restart_at = 200;
        }
        let old_entity = host.runtime.server.clients[client.0 as usize].entity;
        let old_token = host
            .runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .route
            .socket;
        if server_control {
            native
                .channel
                .queue_reliable(&stuff(b"reconnect\n"))
                .unwrap();
            native.reply(&mut server, socket, local, restart_at);
        } else {
            host.console
                .append("reconnect\n", qw_context(SeatId::FIRST))
                .unwrap();
        }
        frame(&mut host, &mut source, restart_at);
        let packets = datagrams(&mut server);
        let mut requests = vec![];
        for packet in packets {
            if packet.1.starts_with(&[255; 4]) {
                requests.push(packet);
            } else {
                assert_eq!(peer(&packet), local);
                native.receive(&packet.1, restart_at);
            }
        }
        assert_eq!(native.count(b"drop"), 3);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].1, b"\xff\xff\xff\xffgetchallenge\n");
        let replacement = host.local_clients[0].unwrap();
        let connection = host
            .runtime
            .network
            .get(replacement, Endpoint::Client)
            .unwrap();
        assert_ne!(connection.route.socket, old_token);
        assert_eq!(connection.commands.as_ref().unwrap().protocol, QW);
        assert!(source.pump.send_udp(old_token, address, b"closed").is_err());
        assert_ne!(
            host.runtime.server.clients[replacement.0 as usize].entity,
            old_entity
        );
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
        let new_local = peer(&requests[0]);
        response(&mut server, socket, new_local, b"c89");
        frame(&mut host, &mut source, restart_at + 10);
        let packets = datagrams(&mut server);
        assert_eq!(packets.len(), 1);
        assert_eq!(
            packets[0].1,
            expected_connect(
                new_local.port(),
                89,
                &userinfo(&host, SeatId::FIRST, address)
            )
        );
        response(&mut server, socket, new_local, b"j");
        frame(&mut host, &mut source, restart_at + 20);
        let mut replacement_native = NativePeer::new(QW, new_local.port());
        for (_, bytes) in datagrams(&mut server) {
            replacement_native.receive(&bytes, restart_at + 20);
        }
        assert_eq!(replacement_native.count(b"new"), 1);
        assert_eq!(replacement_native.moves, 0);
        assert_eq!(history(&host, replacement), 0);
    }
}

#[test]
fn qw_retries_at_five_seconds_and_only_its_exact_server_can_advance_the_handshake() {
    let mut server = EventPump::new();
    let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let mut foreign = EventPump::new();
    let (foreign_socket, _) = foreign.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let mut source = source();
    let mut host = host(1);
    setting(&mut host, SeatId::FIRST, "net_qport", "0");
    let client = connect(&mut host, &mut source, SeatId::FIRST, address, 0);
    let packets = datagrams(&mut server);
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].1, b"\xff\xff\xff\xffgetchallenge\n");
    let local = peer(&packets[0]);
    frame(&mut host, &mut source, 4999);
    assert!(datagrams(&mut server).is_empty());
    frame(&mut host, &mut source, 5000);
    let packets = datagrams(&mut server);
    assert_eq!(packets.len(), 1);
    assert_eq!(peer(&packets[0]), local);
    assert_eq!(packets[0].1, b"\xff\xff\xff\xffgetchallenge\n");

    let unrouted = host.runtime.network.unrouted;
    response(&mut foreign, foreign_socket, local, b"c91");
    response(&mut foreign, foreign_socket, local, b"j");
    frame(&mut host, &mut source, 5001);
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

    setting(&mut host, SeatId::FIRST, "name", "Waiting");
    setting(&mut host, SeatId::FIRST, "skin", "blue");
    response(&mut server, socket, local, b"c-123\n");
    frame(&mut host, &mut source, 5002);
    let packets = datagrams(&mut server);
    assert_eq!(packets.len(), 1);
    let info = userinfo(&host, SeatId::FIRST, address);
    assert_eq!(
        (field(&info, b"name"), field(&info, b"skin")),
        (b"Waiting".as_slice(), b"blue".as_slice())
    );
    assert_eq!(packets[0].1, expected_connect(local.port(), -123, &info));
    frame(&mut host, &mut source, 10001);
    assert!(datagrams(&mut server).is_empty());
    frame(&mut host, &mut source, 10002);
    let packets = datagrams(&mut server);
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].1, b"\xff\xff\xff\xffgetchallenge\n");

    response(&mut server, socket, local, b"nServer is full\n");
    frame(&mut host, &mut source, 10003);
    assert_eq!(host.local_clients[0], Some(client));
    assert!(matches!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .phase,
        ConnectionPhase::Connecting(_)
    ));
    assert!(
        datagrams(&mut server).is_empty(),
        "native QW prints do not acknowledge a connection"
    );
    setting(&mut host, SeatId::FIRST, "name", "Retried");
    response(&mut server, socket, local, b"c456");
    frame(&mut host, &mut source, 10004);
    let packets = datagrams(&mut server);
    assert_eq!(packets.len(), 1);
    assert_eq!(
        packets[0].1,
        expected_connect(local.port(), 456, &userinfo(&host, SeatId::FIRST, address))
    );
}

#[test]
fn qw_acceptance_uses_the_last_admitted_userinfo_and_queues_new_before_serverdata() {
    let mut server = EventPump::new();
    let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let mut source = source();
    let mut host = host(1);
    setting(&mut host, SeatId::FIRST, "net_qport", "54321");
    setting(&mut host, SeatId::FIRST, "name", "Before");
    setting(&mut host, SeatId::FIRST, "skin", "red");
    let client = connect(&mut host, &mut source, SeatId::FIRST, address, 0);
    let packets = datagrams(&mut server);
    assert_eq!(packets.len(), 1);
    let local = peer(&packets[0]);
    let token = host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .route
        .socket;
    setting(&mut host, SeatId::FIRST, "name", "NotAdmitted");
    setting(&mut host, SeatId::FIRST, "skin", "purple");
    source.reject_connect = true;
    response(&mut server, socket, local, b"c93");
    frame(&mut host, &mut source, 10);
    assert!(source.rejected_connects > 0);
    assert!(datagrams(&mut server).is_empty());
    setting(&mut host, SeatId::FIRST, "name", "Admitted");
    setting(&mut host, SeatId::FIRST, "skin", "green");
    setting(&mut host, SeatId::FIRST, "net_qport", "0");
    source.reject_connect = false;
    frame(&mut host, &mut source, 20);
    let packets = datagrams(&mut server);
    assert_eq!(packets.len(), 1);
    let accepted_info = userinfo(&host, SeatId::FIRST, address);
    assert_eq!(packets[0].1, expected_connect(54321, 93, &accepted_info));
    assert_eq!(
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel
            .qport(),
        54321
    );

    setting(&mut host, SeatId::FIRST, "name", "AfterAccept");
    setting(&mut host, SeatId::FIRST, "skin", "post");
    response(&mut server, socket, local, b"j");
    frame(&mut host, &mut source, 30);
    let connection = host.runtime.network.get(client, Endpoint::Client).unwrap();
    assert!(connection.phase.is_connected());
    assert_eq!(connection.route.socket, token);
    let mut native = NativePeer::new(QW, 54321);
    for (_, bytes) in datagrams(&mut server) {
        native.receive(&bytes, 30);
    }
    assert_eq!(native.count(b"new"), 1);
    assert_eq!(native.strings.first().unwrap(), b"new");
    assert_eq!(native.count(b"setinfo \"name\" \"AfterAccept\""), 1);
    assert_eq!(native.count(b"setinfo \"skin\" \"post\""), 1);
    assert_eq!(
        native
            .strings
            .iter()
            .filter(|text| text.starts_with(b"setinfo "))
            .count(),
        2,
        "accepted connect fields already own the connected userinfo baseline"
    );
    assert_eq!(native.moves, 0);
    assert_eq!(history(&host, client), 0);
    native.reply(&mut server, socket, local, 35);
    frame(&mut host, &mut source, 40);
    for (_, bytes) in datagrams(&mut server) {
        native.receive(&bytes, 40);
    }
    assert_eq!(
        history(&host, client),
        0,
        "accepted transport still awaits native serverdata"
    );
    assert_eq!(native.count(b"new"), 1);
    assert_eq!(native.moves, 0);
    response(&mut server, socket, local, b"j");
    frame(&mut host, &mut source, 50);
    for (_, bytes) in datagrams(&mut server) {
        native.receive(&bytes, 50);
    }
    assert_eq!(
        native.count(b"new"),
        1,
        "duplicate native accepts cannot restart signon"
    );
    assert_eq!(history(&host, client), 0);
    host.console
        .append("reconnect\n", qw_context(SeatId::FIRST))
        .unwrap();
    frame(&mut host, &mut source, 60);
    for (_, bytes) in datagrams(&mut server) {
        native.receive(&bytes, 60);
    }
    assert_eq!(
        native.count(b"new"),
        2,
        "explicit reconnect uses the existing native channel"
    );
    assert_eq!(native.moves, 0);
    let connection = host.runtime.network.get(client, Endpoint::Client).unwrap();
    assert_eq!(connection.route.socket, token);
    assert_eq!(connection.channel.qport(), 54321);
    assert_eq!(history(&host, client), 0);
}

#[test]
fn two_seats_capture_distinct_default_qports_from_their_owned_udp_leases() {
    let mut server = EventPump::new();
    let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let mut source = source();
    let mut host = host(2);
    setting(&mut host, SeatId::FIRST, "net_qport", "0");
    for seat in [SeatId::FIRST, SeatId::ALL[1]] {
        host.console
            .append(&format!("connect {address}\n"), qw_context(seat))
            .unwrap();
    }
    frame(&mut host, &mut source, 0);
    let mut requests = datagrams(&mut server);
    assert_eq!(requests.len(), 2);
    requests.sort_by_key(|packet| peer(packet).port());
    assert_ne!(peer(&requests[0]).port(), peer(&requests[1]).port());
    let clients = [
        host.local_clients[0].unwrap(),
        host.local_clients[1].unwrap(),
    ];
    let tokens = clients.map(|client| {
        host.runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .route
            .socket
    });
    assert_ne!(tokens[0], tokens[1]);
    for (index, request) in requests.iter().enumerate() {
        assert_eq!(request.1, b"\xff\xff\xff\xffgetchallenge\n");
        response(
            &mut server,
            socket,
            peer(request),
            format!("c{}", 700 + index).as_bytes(),
        );
    }
    frame(&mut host, &mut source, 10);
    let connects = datagrams(&mut server);
    assert_eq!(connects.len(), 2);
    for (index, request) in requests.iter().enumerate() {
        let local = peer(request);
        let packet = connects
            .iter()
            .find(|packet| peer(packet) == local)
            .unwrap();
        assert_eq!(
            packet.1,
            expected_connect(
                local.port(),
                700 + index as i32,
                &userinfo(&host, SeatId::FIRST, address)
            )
        );
        let selected = clients
            .iter()
            .copied()
            .find(|client| {
                host.runtime
                    .network
                    .get(*client, Endpoint::Client)
                    .unwrap()
                    .channel
                    .qport()
                    == local.port()
            })
            .unwrap();
        assert!(
            host.runtime
                .network
                .get(selected, Endpoint::Server)
                .is_none()
        );
        response(&mut server, socket, local, b"j");
    }
    frame(&mut host, &mut source, 20);
    let connected = datagrams(&mut server);
    assert_eq!(connected.len(), 2);
    for request in &requests {
        let local = peer(request);
        let mut native = NativePeer::new(QW, local.port());
        for packet in connected.iter().filter(|packet| peer(packet) == local) {
            native.receive(&packet.1, 20);
        }
        assert_eq!(native.count(b"new"), 1);
        assert_eq!(native.moves, 0);
    }
    assert!(
        clients
            .into_iter()
            .all(|client| history(&host, client) == 0)
    );
}

#[test]
fn netquake_control_rejection_releases_only_its_client_lease() {
    let mut server = EventPump::new();
    let (socket, address) = server.bind_udp(([127, 0, 0, 1], 0).into()).unwrap();
    let mut source = source();
    let mut host = host(2);
    let healthy = connect(&mut host, &mut source, SeatId::ALL[1], address, 0);
    let healthy_packets = datagrams(&mut server);
    assert_eq!(healthy_packets.len(), 1);
    host.console
        .append(&format!("connect {address}\n"), context())
        .unwrap();
    frame(&mut host, &mut source, 10);
    let rejected = host.local_clients[0].unwrap();
    let token = host
        .runtime
        .network
        .get(rejected, Endpoint::Client)
        .unwrap()
        .route
        .socket;
    let packets = datagrams(&mut server);
    assert_eq!(packets.len(), 1);
    assert!(matches!(
        read_nq_control(&packets[0].1).unwrap(),
        NqControl::Connect
    ));
    let mut bytes = [0; 1024];
    let length = write_nq_control(
        NqControl::Reject {
            reason: b"fixture full",
        },
        &mut bytes,
    )
    .unwrap();
    server
        .send_udp(socket, peer(&packets[0]), &bytes[..length])
        .unwrap();
    frame(&mut host, &mut source, 20);
    assert!(host.local_clients[0].is_none());
    assert!(
        host.runtime
            .network
            .get(rejected, Endpoint::Client)
            .is_none()
    );
    assert!(source.pump.send_udp(token, address, b"closed").is_err());
    assert_eq!(host.local_clients[1], Some(healthy));
    assert!(matches!(
        host.runtime
            .network
            .get(healthy, Endpoint::Client)
            .unwrap()
            .phase,
        ConnectionPhase::Connecting(_)
    ));
}
